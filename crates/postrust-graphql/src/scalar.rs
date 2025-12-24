//! Custom GraphQL scalar definitions.

use async_graphql::{InputValueError, InputValueResult, Scalar, ScalarType, Value};

/// BigInt scalar for 64-bit integers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BigInt(pub i64);

#[Scalar]
impl ScalarType for BigInt {
    fn parse(value: Value) -> InputValueResult<Self> {
        match &value {
            Value::Number(n) => {
                if let Some(i) = n.as_i64() {
                    Ok(BigInt(i))
                } else {
                    Err(InputValueError::expected_type(value))
                }
            }
            Value::String(s) => s
                .parse::<i64>()
                .map(BigInt)
                .map_err(|_| InputValueError::expected_type(value)),
            _ => Err(InputValueError::expected_type(value)),
        }
    }

    fn to_value(&self) -> Value {
        Value::Number(self.0.into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn test_bigint_parse_number() {
        let value = Value::Number(42.into());
        let result = BigInt::parse(value).unwrap();
        assert_eq!(result.0, 42);
    }

    #[test]
    fn test_bigint_parse_string() {
        let value = Value::String("9223372036854775807".into());
        let result = BigInt::parse(value).unwrap();
        assert_eq!(result.0, i64::MAX);
    }

    #[test]
    fn test_bigint_to_value() {
        let bigint = BigInt(42);
        let value = bigint.to_value();
        assert_eq!(value, Value::Number(42.into()));
    }

    #[test]
    fn test_bigint_parse_invalid() {
        let value = Value::Boolean(true);
        let result = BigInt::parse(value);
        assert!(result.is_err());
    }
}
