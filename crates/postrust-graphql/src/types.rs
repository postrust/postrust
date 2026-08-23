//! PostgreSQL to GraphQL type mapping.

use std::fmt;

/// Represents a GraphQL type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GraphQLType {
    /// GraphQL Int (32-bit signed integer)
    Int,
    /// GraphQL Float (double-precision floating point)
    Float,
    /// GraphQL String
    String,
    /// GraphQL Boolean
    Boolean,
    /// GraphQL ID
    Id,
    /// Custom BigInt scalar (64-bit integer)
    BigInt,
    /// Custom BigDecimal scalar (arbitrary precision)
    BigDecimal,
    /// Custom JSON scalar
    Json,
    /// Custom UUID scalar
    Uuid,
    /// Custom Date scalar
    Date,
    /// Custom DateTime scalar
    DateTime,
    /// Custom Time scalar
    Time,
    /// List type wrapping another type
    List(Box<GraphQLType>),
    /// Custom/unknown type (falls back to String)
    Custom(std::string::String),
}

impl fmt::Display for GraphQLType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GraphQLType::Int => write!(f, "Int"),
            GraphQLType::Float => write!(f, "Float"),
            GraphQLType::String => write!(f, "String"),
            GraphQLType::Boolean => write!(f, "Boolean"),
            GraphQLType::Id => write!(f, "ID"),
            // Named as PostgreSQL names them, which is how Hasura names them
            // and therefore how a client's own queries declare their
            // variables: `query ($x: jsonb!)` names a type that has to exist
            // under exactly that spelling.
            GraphQLType::BigInt => write!(f, "bigint"),
            GraphQLType::BigDecimal => write!(f, "numeric"),
            GraphQLType::Json => write!(f, "jsonb"),
            GraphQLType::Uuid => write!(f, "uuid"),
            GraphQLType::Date => write!(f, "date"),
            GraphQLType::DateTime => write!(f, "timestamptz"),
            GraphQLType::Time => write!(f, "time"),
            GraphQLType::List(inner) => write!(f, "[{}]", inner),
            GraphQLType::Custom(name) => write!(f, "{}", name),
        }
    }
}

/// Maps a PostgreSQL type name to a GraphQL type.
pub fn pg_type_to_graphql(pg_type: &str) -> GraphQLType {
    // Normalize the type name
    let normalized = pg_type.to_lowercase().trim().to_string();

    // Check for array types first
    if let Some(inner_type) = normalized.strip_prefix('_') {
        // PostgreSQL array types start with underscore (e.g., _int4)
        return GraphQLType::List(Box::new(pg_type_to_graphql(inner_type)));
    }

    if normalized.ends_with("[]") {
        // Alternative array syntax (e.g., integer[])
        let inner_type = normalized.trim_end_matches("[]");
        return GraphQLType::List(Box::new(pg_type_to_graphql(inner_type)));
    }

    match normalized.as_str() {
        // Integer types
        "integer" | "int" | "int4" | "smallint" | "int2" => GraphQLType::Int,

        // BigInt types
        "bigint" | "int8" => GraphQLType::BigInt,

        // Float types
        "real" | "float4" | "double precision" | "float8" => GraphQLType::Float,

        // Numeric/Decimal types
        "numeric" | "decimal" => GraphQLType::BigDecimal,

        // Boolean
        "boolean" | "bool" => GraphQLType::Boolean,

        // String types
        "text" | "varchar" | "character varying" | "char" | "character" | "bpchar" => {
            GraphQLType::String
        }

        // `json` and `jsonb` are different types to PostgreSQL and to a
        // client declaring a variable, however alike they look here.
        "jsonb" => GraphQLType::Json,
        "json" => GraphQLType::Custom("json".to_string()),

        // UUID
        "uuid" => GraphQLType::Uuid,

        // Date/Time types
        "timestamptz" | "timestamp with time zone" => GraphQLType::DateTime,
        "timestamp" | "timestamp without time zone" => {
            GraphQLType::Custom("timestamp".to_string())
        }
        "date" => GraphQLType::Date,
        "time" | "time without time zone" => GraphQLType::Time,
        "timetz" | "time with time zone" => GraphQLType::Custom("timetz".to_string()),

        // Types with no GraphQL equivalent keep their own name. A client that
        // was generated against Hasura declares `$point: geometry!` and
        // `$tags: _text`, and a scalar named anything else is a type that does
        // not exist as far as that query is concerned. This also covers user
        // types the database defines -- an enum arrives here as its own type
        // name and becomes a scalar under it.
        other => GraphQLType::Custom(other.to_string()),
    }
}

/// Check if a PostgreSQL type is nullable in GraphQL context.
pub fn is_nullable_type(nullable: bool, is_pk: bool) -> bool {
    // Primary keys are never null in GraphQL
    if is_pk {
        return false;
    }
    nullable
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn test_pg_to_graphql_integer_types() {
        assert_eq!(pg_type_to_graphql("integer"), GraphQLType::Int);
        assert_eq!(pg_type_to_graphql("int4"), GraphQLType::Int);
        assert_eq!(pg_type_to_graphql("int"), GraphQLType::Int);
        assert_eq!(pg_type_to_graphql("smallint"), GraphQLType::Int);
        assert_eq!(pg_type_to_graphql("int2"), GraphQLType::Int);
    }

    #[test]
    fn test_pg_to_graphql_bigint() {
        assert_eq!(pg_type_to_graphql("bigint"), GraphQLType::BigInt);
        assert_eq!(pg_type_to_graphql("int8"), GraphQLType::BigInt);
    }

    #[test]
    fn test_pg_to_graphql_float_types() {
        assert_eq!(pg_type_to_graphql("real"), GraphQLType::Float);
        assert_eq!(pg_type_to_graphql("float4"), GraphQLType::Float);
        assert_eq!(pg_type_to_graphql("double precision"), GraphQLType::Float);
        assert_eq!(pg_type_to_graphql("float8"), GraphQLType::Float);
    }

    #[test]
    fn test_pg_to_graphql_numeric_types() {
        assert_eq!(pg_type_to_graphql("numeric"), GraphQLType::BigDecimal);
        assert_eq!(pg_type_to_graphql("decimal"), GraphQLType::BigDecimal);
    }

    #[test]
    fn test_pg_to_graphql_string_types() {
        assert_eq!(pg_type_to_graphql("text"), GraphQLType::String);
        assert_eq!(pg_type_to_graphql("varchar"), GraphQLType::String);
        assert_eq!(pg_type_to_graphql("character varying"), GraphQLType::String);
        assert_eq!(pg_type_to_graphql("char"), GraphQLType::String);
        assert_eq!(pg_type_to_graphql("bpchar"), GraphQLType::String);
    }

    #[test]
    fn test_pg_to_graphql_boolean() {
        assert_eq!(pg_type_to_graphql("boolean"), GraphQLType::Boolean);
        assert_eq!(pg_type_to_graphql("bool"), GraphQLType::Boolean);
    }

    #[test]
    fn test_pg_to_graphql_json() {
        // Different types to PostgreSQL, and to a client declaring a variable.
        assert_eq!(pg_type_to_graphql("jsonb"), GraphQLType::Json);
        assert_eq!(
            pg_type_to_graphql("json"),
            GraphQLType::Custom("json".to_string())
        );
    }

    #[test]
    fn test_pg_to_graphql_uuid() {
        assert_eq!(pg_type_to_graphql("uuid"), GraphQLType::Uuid);
    }

    #[test]
    fn test_pg_to_graphql_datetime_types() {
        assert_eq!(
            pg_type_to_graphql("timestamp"),
            GraphQLType::Custom("timestamp".to_string())
        );
        assert_eq!(pg_type_to_graphql("timestamptz"), GraphQLType::DateTime);
        assert_eq!(
            pg_type_to_graphql("timestamp with time zone"),
            GraphQLType::DateTime
        );
        assert_eq!(
            pg_type_to_graphql("timestamp without time zone"),
            GraphQLType::Custom("timestamp".to_string())
        );
    }

    #[test]
    fn test_pg_to_graphql_date() {
        assert_eq!(pg_type_to_graphql("date"), GraphQLType::Date);
    }

    #[test]
    fn test_pg_to_graphql_time() {
        assert_eq!(pg_type_to_graphql("time"), GraphQLType::Time);
        assert_eq!(
            pg_type_to_graphql("timetz"),
            GraphQLType::Custom("timetz".to_string())
        );
        assert_eq!(
            pg_type_to_graphql("time with time zone"),
            GraphQLType::Custom("timetz".to_string())
        );
    }

    #[test]
    fn test_pg_to_graphql_array_types_underscore() {
        assert_eq!(
            pg_type_to_graphql("_int4"),
            GraphQLType::List(Box::new(GraphQLType::Int))
        );
        assert_eq!(
            pg_type_to_graphql("_text"),
            GraphQLType::List(Box::new(GraphQLType::String))
        );
        assert_eq!(
            pg_type_to_graphql("_uuid"),
            GraphQLType::List(Box::new(GraphQLType::Uuid))
        );
    }

    #[test]
    fn test_pg_to_graphql_array_types_bracket() {
        assert_eq!(
            pg_type_to_graphql("integer[]"),
            GraphQLType::List(Box::new(GraphQLType::Int))
        );
        assert_eq!(
            pg_type_to_graphql("text[]"),
            GraphQLType::List(Box::new(GraphQLType::String))
        );
    }

    #[test]
    fn test_pg_to_graphql_unknown_keeps_its_own_name() {
        // A client generated against Hasura declares `$point: geometry!`, and
        // a scalar named anything else is a type its query cannot name.
        assert_eq!(
            pg_type_to_graphql("customtype"),
            GraphQLType::Custom("customtype".to_string())
        );
        assert_eq!(
            pg_type_to_graphql("my_domain"),
            GraphQLType::Custom("my_domain".to_string())
        );
    }

    #[test]
    fn test_pg_to_graphql_case_insensitive() {
        assert_eq!(pg_type_to_graphql("INTEGER"), GraphQLType::Int);
        assert_eq!(pg_type_to_graphql("Text"), GraphQLType::String);
        assert_eq!(pg_type_to_graphql("BOOLEAN"), GraphQLType::Boolean);
    }

    #[test]
    fn test_graphql_type_display() {
        assert_eq!(format!("{}", GraphQLType::Int), "Int");
        assert_eq!(format!("{}", GraphQLType::String), "String");
        assert_eq!(
            format!("{}", GraphQLType::List(Box::new(GraphQLType::Int))),
            "[Int]"
        );
    }

    #[test]
    fn test_is_nullable_type() {
        // PK is never nullable
        assert!(!is_nullable_type(true, true));
        assert!(!is_nullable_type(false, true));

        // Non-PK follows the nullable flag
        assert!(is_nullable_type(true, false));
        assert!(!is_nullable_type(false, false));
    }
}
