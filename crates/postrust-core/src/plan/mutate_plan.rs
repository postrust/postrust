//! Mutation (INSERT/UPDATE/DELETE) query planning.

use super::types::*;
use crate::api_request::{ApiRequest, Mutation, Payload, PreferResolution, QualifiedIdentifier};
use crate::error::{Error, Result};
use crate::schema_cache::Table;
use serde::{Deserialize, Serialize};

/// A mutation plan.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum MutatePlan {
    /// INSERT operation
    Insert {
        /// Target table
        target: QualifiedIdentifier,
        /// Columns to insert
        columns: Vec<CoercibleField>,
        /// Request body (JSON)
        body: Option<bytes::Bytes>,
        /// ON CONFLICT handling
        on_conflict: Option<(PreferResolution, Vec<String>)>,
        /// WHERE clause (for filtered inserts)
        where_clauses: Vec<CoercibleLogicTree>,
        /// RETURNING columns
        returning: Vec<String>,
        /// Primary key columns
        pk_cols: Vec<String>,
        /// Apply defaults for missing columns
        apply_defaults: bool,
    },
    /// UPDATE operation
    Update {
        /// Target table
        target: QualifiedIdentifier,
        /// Columns to update
        columns: Vec<CoercibleField>,
        /// Request body (JSON)
        body: Option<bytes::Bytes>,
        /// WHERE clauses
        where_clauses: Vec<CoercibleLogicTree>,
        /// RETURNING columns
        returning: Vec<String>,
        /// Apply defaults for NULL columns
        apply_defaults: bool,
    },
    /// DELETE operation
    Delete {
        /// Target table
        target: QualifiedIdentifier,
        /// WHERE clauses
        where_clauses: Vec<CoercibleLogicTree>,
        /// RETURNING columns
        returning: Vec<String>,
    },
}

impl MutatePlan {
    /// Create a mutation plan from an API request.
    pub fn from_request(
        request: &ApiRequest,
        table: &Table,
        mutation: &Mutation,
        schema_cache: &crate::schema_cache::SchemaCache,
    ) -> Result<Self> {
        let qi = table.qualified_identifier();

        match mutation {
            Mutation::Create => Self::create_insert(request, table, qi, schema_cache),
            Mutation::Update => Self::create_update(request, table, qi, schema_cache),
            Mutation::Delete => Self::create_delete(request, table, qi),
            Mutation::SingleUpsert => Self::create_upsert(request, table, qi, schema_cache),
        }
    }

    /// Create an INSERT plan.
    fn create_insert(
        request: &ApiRequest,
        table: &Table,
        qi: QualifiedIdentifier,
        schema_cache: &crate::schema_cache::SchemaCache,
    ) -> Result<Self> {
        let columns = get_payload_columns(request, table, schema_cache)?;
        let body = get_body_bytes(request)?;
        let returning = get_returning_columns(request, table);
        let apply_defaults =
            request.preferences.missing == crate::api_request::PreferMissing::ApplyDefaults;

        // `Prefer: resolution=` says what to do about a duplicate; `on_conflict=`
        // says which columns make one. Without the latter the answer is the
        // primary key, which is what "duplicate" means when nothing else is
        // said -- and without a resolution there is nothing to do about one.
        let on_conflict = request
            .preferences
            .resolution
            .clone()
            .and_then(|resolution| {
                let columns = request
                    .query_params
                    .on_conflict
                    .clone()
                    .unwrap_or_else(|| table.pk_cols.clone());
                match columns.is_empty() {
                    true => None,
                    false => Some((resolution, columns)),
                }
            });

        Ok(Self::Insert {
            target: qi,
            columns,
            body,
            on_conflict,
            where_clauses: vec![],
            returning,
            pk_cols: table.pk_cols.clone(),
            apply_defaults,
        })
    }

    /// Create an UPDATE plan.
    fn create_update(
        request: &ApiRequest,
        table: &Table,
        qi: QualifiedIdentifier,
        schema_cache: &crate::schema_cache::SchemaCache,
    ) -> Result<Self> {
        let columns = get_payload_columns(request, table, schema_cache)?;
        let body = get_body_bytes(request)?;
        let where_clauses = build_mutation_where(request, table)?;
        let returning = get_returning_columns(request, table);
        let apply_defaults =
            request.preferences.missing == crate::api_request::PreferMissing::ApplyDefaults;

        Ok(Self::Update {
            target: qi,
            columns,
            body,
            where_clauses,
            returning,
            apply_defaults,
        })
    }

    /// Create a DELETE plan.
    fn create_delete(request: &ApiRequest, table: &Table, qi: QualifiedIdentifier) -> Result<Self> {
        let where_clauses = build_mutation_where(request, table)?;
        let returning = get_returning_columns(request, table);

        Ok(Self::Delete {
            target: qi,
            where_clauses,
            returning,
        })
    }

    /// Create a PUT (upsert) plan.
    fn create_upsert(
        request: &ApiRequest,
        table: &Table,
        qi: QualifiedIdentifier,
        schema_cache: &crate::schema_cache::SchemaCache,
    ) -> Result<Self> {
        validate_put(request, table)?;

        let columns = get_payload_columns(request, table, schema_cache)?;
        let body = get_body_bytes(request)?;
        let returning = get_returning_columns(request, table);

        // Upsert uses PK for conflict
        let on_conflict = Some((PreferResolution::MergeDuplicates, table.pk_cols.clone()));

        Ok(Self::Insert {
            target: qi,
            columns,
            body,
            on_conflict,
            where_clauses: vec![],
            returning,
            pk_cols: table.pk_cols.clone(),
            apply_defaults: true,
        })
    }

    /// Get the target table.
    pub fn target(&self) -> &QualifiedIdentifier {
        match self {
            Self::Insert { target, .. } => target,
            Self::Update { target, .. } => target,
            Self::Delete { target, .. } => target,
        }
    }

    /// Check if this mutation has a body.
    pub fn has_body(&self) -> bool {
        match self {
            Self::Insert { body, .. } => body.is_some(),
            Self::Update { body, .. } => body.is_some(),
            Self::Delete { .. } => false,
        }
    }
}

/// Get columns from payload.
fn get_payload_columns(
    request: &ApiRequest,
    table: &Table,
    schema_cache: &crate::schema_cache::SchemaCache,
) -> Result<Vec<CoercibleField>> {
    let keys = match &request.payload {
        Some(Payload::ProcessedJson { keys, .. }) => keys,
        Some(Payload::ProcessedUrlEncoded { keys, .. }) => keys,
        _ => return Ok(vec![]),
    };

    // `?columns=` names the columns to write and fixes their order, which is
    // what makes a bulk insert of ragged rows well-defined. Without it the
    // body's own keys are the columns.
    let names: Vec<&String> = match &request.query_params.columns {
        Some(columns) => columns.iter().collect(),
        None => keys.iter().collect(),
    };

    let mut columns = Vec::new();

    for key in names {
        let column = table.get_column(key).ok_or_else(|| Error::UnknownColumn {
            column: key.clone(),
            relation: table.name.clone(),
        })?;

        let mut field = CoercibleField::simple(key, &column.nominal_type);
        // A schema that declared how one of its domains is written in JSON
        // also declared how one arrives: the value is read out of the body as
        // JSON and handed to that cast, rather than to PostgreSQL's own input
        // function for the type underneath.
        if let Some(function) = schema_cache.representation("json", &column.nominal_type) {
            field.ir_type = "json".to_string();
            field.transform = Some(function.to_string());
        }
        columns.push(field);
    }

    Ok(columns)
}

/// Check that a `PUT` names exactly one row, and that its body agrees.
///
/// `PUT` replaces the row the URL identifies, so the URL has to identify one:
/// every primary key column, compared with `eq`, and nothing else. A page
/// makes no sense on top of that, and a body naming a different row would
/// write somewhere the client did not ask for.
fn validate_put(request: &ApiRequest, table: &Table) -> Result<()> {
    use crate::api_request::{Operation, QuantOperator};

    if request
        .query_params
        .ranges
        .values()
        .any(|range| range.limit.is_some() || range.offset != 0)
    {
        return Err(Error::PutLimitNotAllowed);
    }

    let mut keyed: std::collections::HashMap<&str, &str> = std::collections::HashMap::new();
    for filter in &request.query_params.filters_root {
        let Operation::Quant {
            op: QuantOperator::Equal,
            quantifier: None,
            value,
        } = &filter.op_expr.operation
        else {
            return Err(Error::InvalidPutFilters);
        };
        if filter.op_expr.negated {
            return Err(Error::InvalidPutFilters);
        }
        keyed.insert(&filter.field.name, value);
    }

    // Anything else in the query string that filters -- a logic group, a
    // filter on an embedded resource -- also fails to name exactly one row.
    if !request.query_params.logic.is_empty() || !request.query_params.filters.is_empty() {
        return Err(Error::InvalidPutFilters);
    }

    if table.pk_cols.is_empty()
        || keyed.len() != table.pk_cols.len()
        || !table
            .pk_cols
            .iter()
            .all(|key| keyed.contains_key(key.as_str()))
    {
        return Err(Error::InvalidPutFilters);
    }

    // The body may leave a key out -- the URL already said what it is -- but
    // where it names one it has to be the same one. A body written as an
    // array is checked element by element: `PUT` writes the row the URL names
    // and no other, however many the body offers.
    if let Some(Payload::ProcessedJson { raw, .. }) = &request.payload {
        let rows = match serde_json::from_slice(raw) {
            Ok(serde_json::Value::Object(row)) => vec![row],
            Ok(serde_json::Value::Array(rows)) => rows
                .into_iter()
                .filter_map(|row| match row {
                    serde_json::Value::Object(row) => Some(row),
                    _ => None,
                })
                .collect(),
            _ => vec![],
        };

        for row in rows {
            for key in &table.pk_cols {
                let Some(value) = row.get(key) else { continue };
                let value = match value {
                    serde_json::Value::String(text) => text.clone(),
                    other => other.to_string(),
                };
                if keyed.get(key.as_str()) != Some(&value.as_str()) {
                    return Err(Error::PutMatchingPk);
                }
            }
        }
    }

    Ok(())
}

/// Get body as bytes.
fn get_body_bytes(request: &ApiRequest) -> Result<Option<bytes::Bytes>> {
    match &request.payload {
        Some(Payload::ProcessedJson { raw, .. }) => Ok(Some(raw.clone())),
        Some(Payload::RawJson(raw)) => Ok(Some(raw.clone())),
        Some(Payload::RawPayload(raw)) => Ok(Some(raw.clone())),
        Some(Payload::ProcessedUrlEncoded { data, .. }) => {
            // Convert to JSON
            let json = serde_json::to_vec(
                &data
                    .iter()
                    .cloned()
                    .collect::<std::collections::HashMap<_, _>>(),
            )
            .map_err(|e| Error::InvalidBody(e.to_string()))?;
            Ok(Some(bytes::Bytes::from(json)))
        }
        None => Ok(None),
    }
}

/// Get returning columns.
fn get_returning_columns(_request: &ApiRequest, table: &Table) -> Vec<String> {
    // Every column, always. What the client asked for is decided by the read
    // that runs over the result -- `?select=` may name computed fields and
    // embedded resources, neither of which a `RETURNING` list can express --
    // and the primary key is needed for the `Location` header whether or not
    // the client asked to see it.
    table.column_names().map(|s| s.to_string()).collect()
}

/// Build WHERE clauses for mutations.
fn build_mutation_where(request: &ApiRequest, table: &Table) -> Result<Vec<CoercibleLogicTree>> {
    let type_resolver = |name: &str| -> String {
        table
            .get_column(name)
            .map(|c| c.data_type.clone())
            .unwrap_or_else(|| "text".to_string())
    };

    let mut clauses = Vec::new();

    for filter in &request.query_params.filters_root {
        let pg_type = type_resolver(&filter.field.name);
        clauses.push(CoercibleLogicTree::Stmt(CoercibleFilter::from_filter(
            filter, &pg_type,
        )));
    }

    Ok(clauses)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mutate_plan_target() {
        let qi = QualifiedIdentifier::new("public", "users");
        let plan = MutatePlan::Delete {
            target: qi.clone(),
            where_clauses: vec![],
            returning: vec!["id".into()],
        };

        assert_eq!(plan.target().name, "users");
    }

    #[test]
    fn test_mutate_plan_has_body() {
        let qi = QualifiedIdentifier::new("public", "users");

        let insert = MutatePlan::Insert {
            target: qi.clone(),
            columns: vec![],
            body: Some(bytes::Bytes::from("{}".as_bytes())),
            on_conflict: None,
            where_clauses: vec![],
            returning: vec![],
            pk_cols: vec![],
            apply_defaults: true,
        };
        assert!(insert.has_body());

        let delete = MutatePlan::Delete {
            target: qi,
            where_clauses: vec![],
            returning: vec![],
        };
        assert!(!delete.has_body());
    }
}
