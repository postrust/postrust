//! RPC (stored function) call planning.

use crate::api_request::{ApiRequest, Payload, QualifiedIdentifier};
use crate::error::{Error, Result};
use crate::schema_cache::Routine;
use serde::{Deserialize, Serialize};

/// A plan for calling a stored function.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CallPlan {
    /// Function identifier
    pub function: QualifiedIdentifier,
    /// Call parameters
    pub params: CallParams,
    /// Whether to return a scalar result
    pub returns_scalar: bool,
    /// The declared return type, as PostgreSQL spells it.
    ///
    /// Used to decide whether this process can decode the result or whether
    /// the database should render it, exactly as for a table's columns.
    #[serde(default)]
    pub return_type: Option<String>,
    /// Whether the function is set-returning
    pub returns_set: bool,
    /// Whether the function returns nothing at all.
    ///
    /// A `void` function has no result to report, which is 204 rather than a
    /// body of `null` -- and `RetType::Void` has no type name, so asking for
    /// one would never find it.
    #[serde(default)]
    pub returns_void: bool,
    /// Whether the return type is composite (row type or `record`): its
    /// columns are real output columns, never the function-name wrapper.
    #[serde(default)]
    pub returns_composite: bool,
    /// Function volatility (for transaction handling)
    pub volatility: String,
    /// Declared parameter types, in the routine's own order, as
    /// `(name, pg_type)`.
    ///
    /// Arguments arrive from the URL or a JSON body as strings. Binding them
    /// as `text` leaves PostgreSQL unable to resolve any signature that isn't
    /// text -- `add_them(a => text, b => text) does not exist` -- so each one
    /// is cast to its declared type at call time.
    #[serde(default)]
    pub param_types: Vec<(String, String)>,
}

/// How parameters are passed to the function.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum CallParams {
    /// Named parameters from URL query or JSON body
    Named(Vec<(String, String)>),
    /// Positional parameters (from JSON array)
    Positional(Vec<String>),
    /// Single JSON object passed as first argument
    SingleObject(bytes::Bytes),
    /// No parameters
    None,
}

impl CallPlan {
    /// Create a call plan from an API request.
    pub fn from_request(request: &ApiRequest, routine: &Routine) -> Result<Self> {
        let qi = routine.qualified_identifier();

        let params = extract_call_params(request, routine)?;

        let returns_scalar = !routine.return_type.is_set_returning()
            && routine
                .return_type
                .type_name()
                .map(|t| !t.contains("record"))
                .unwrap_or(true);

        Ok(Self {
            function: qi,
            params,
            returns_scalar,
            return_type: routine.return_type.type_name().map(str::to_string),
            returns_set: routine.return_type.is_set_returning(),
            returns_void: matches!(routine.return_type, crate::schema_cache::RetType::Void),
            returns_composite: routine.returns_composite,
            volatility: format!("{:?}", routine.volatility),
            param_types: routine
                .params
                .iter()
                .map(|p| (p.name.clone(), p.param_type.clone()))
                .collect(),
        })
    }

    /// Check if this call has parameters.
    pub fn has_params(&self) -> bool {
        !matches!(self.params, CallParams::None)
    }
}

/// Extract call parameters from request.
fn extract_call_params(request: &ApiRequest, routine: &Routine) -> Result<CallParams> {
    // Check for JSON body first
    if let Some(payload) = &request.payload {
        match payload {
            Payload::ProcessedJson { raw, .. } => {
                // Check if it's an object or array
                let value: serde_json::Value =
                    serde_json::from_slice(raw).map_err(|e| Error::InvalidBody(e.to_string()))?;

                match value {
                    serde_json::Value::Object(map) => {
                        // Named parameters from JSON object
                        let params: Vec<(String, String)> = map
                            .into_iter()
                            .map(|(k, v)| {
                                // Extract string values without JSON quotes
                                let value = match v {
                                    serde_json::Value::String(s) => s,
                                    serde_json::Value::Null => String::new(),
                                    // A JSON array for a parameter the function
                                    // declares as an array is that array, not a
                                    // string spelled in JSON: `{"v":["a","b"]}`
                                    // passes two values to a variadic. PostgreSQL
                                    // reads `{...}`, not `[...]`.
                                    serde_json::Value::Array(items)
                                        if routine.params.iter().any(|p| {
                                            p.name == k && p.param_type.ends_with("[]")
                                        }) =>
                                    {
                                        array_literal(
                                            &items
                                                .iter()
                                                .map(|item| match item {
                                                    serde_json::Value::String(s) => s.clone(),
                                                    other => other.to_string(),
                                                })
                                                .collect::<Vec<_>>(),
                                        )
                                    }
                                    other => other.to_string(),
                                };
                                (k, value)
                            })
                            .collect();
                        return Ok(CallParams::Named(params));
                    }
                    serde_json::Value::Array(_) => {
                        // Pass entire JSON as single argument
                        return Ok(CallParams::SingleObject(raw.clone()));
                    }
                    _ => {
                        // Scalar value - pass as single argument
                        return Ok(CallParams::SingleObject(raw.clone()));
                    }
                }
            }
            Payload::ProcessedUrlEncoded { data, .. } => {
                // Named parameters from form data
                return Ok(CallParams::Named(data.clone()));
            }
            Payload::RawJson(raw) | Payload::RawPayload(raw) => {
                return Ok(CallParams::SingleObject(raw.clone()));
            }
        }
    }

    // Fall back to query parameters. Only those naming a declared parameter
    // are arguments -- the rest are filters on the function's result, which
    // is how `/rpc/getallprojects?id=eq.1` filters rather than failing.
    if !request.query_params.params.is_empty() {
        let mut args: Vec<(String, Vec<String>)> = Vec::new();
        for (name, value) in &request.query_params.params {
            // A value carrying an operator filters the result; only a bare
            // one is an argument. `?id=5&id=gt.2` is both at once.
            if crate::api_request::value_is_filter(value) {
                continue;
            }
            if !routine.params.iter().any(|p| &p.name == name) {
                continue;
            }
            match args.iter_mut().find(|(existing, _)| existing == name) {
                Some((_, values)) => values.push(value.clone()),
                None => args.push((name.clone(), vec![value.clone()])),
            }
        }

        // A name given more than once is one argument, not two: for a
        // variadic parameter every value goes into the array it takes, and
        // for any other the last one wins -- repeating it cannot mean asking
        // for two values where the signature has room for one.
        let args: Vec<(String, String)> = args
            .into_iter()
            .map(|(name, values)| {
                let variadic = routine.params.iter().any(|p| p.name == name && p.variadic);
                let value = match variadic {
                    true => array_literal(&values),
                    false => values.last().cloned().unwrap_or_default(),
                };
                (name, value)
            })
            .collect();

        if !args.is_empty() {
            return Ok(CallParams::Named(args));
        }
    }

    // No parameters
    Ok(CallParams::None)
}

/// Render values as a PostgreSQL array literal.
///
/// Every element is quoted, which is always valid and saves deciding which
/// ones would otherwise need it -- an element containing a comma, a brace or a
/// quote of its own would silently change the array's shape.
fn array_literal(values: &[String]) -> String {
    let elements: Vec<String> = values
        .iter()
        .map(|value| format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\"")))
        .collect();
    format!("{{{}}}", elements.join(","))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema_cache::{FuncVolatility, RetType};

    fn make_routine() -> Routine {
        Routine {
            schema: "public".into(),
            name: "get_users".into(),
            description: None,
            params: vec![],
            return_type: RetType::SetOf("users".into()),
            returns_composite: true,
            volatility: FuncVolatility::Stable,
            has_variadic: false,
            isolation_level: None,
            settings: vec![],
            is_procedure: false,
        }
    }

    #[test]
    fn test_call_plan_basic() {
        let request = ApiRequest::default();
        let routine = make_routine();

        let plan = CallPlan::from_request(&request, &routine).unwrap();

        assert_eq!(plan.function.name, "get_users");
        assert!(plan.returns_set);
        assert!(!plan.returns_scalar);
        assert!(plan.returns_composite);
    }

    #[test]
    fn test_call_plan_scalar_is_not_composite() {
        let request = ApiRequest::default();
        let routine = Routine {
            return_type: RetType::Single("integer".into()),
            returns_composite: false,
            ..make_routine()
        };

        let plan = CallPlan::from_request(&request, &routine).unwrap();

        assert!(plan.returns_scalar);
        assert!(!plan.returns_set);
        assert!(!plan.returns_composite);
    }

    #[test]
    fn test_call_params_none() {
        let request = ApiRequest::default();
        let routine = make_routine();

        let plan = CallPlan::from_request(&request, &routine).unwrap();
        assert!(!plan.has_params());
    }
}
