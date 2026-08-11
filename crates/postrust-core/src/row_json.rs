//! Conversion of PostgreSQL rows to JSON.
//!
//! Shared by every adapter that returns query results (the HTTP server, the
//! Lambda adapter). Keeping one implementation matters: a per-adapter copy
//! that reads every column as `serde_json::Value` silently yields `null` for
//! anything that is not `json`/`jsonb`, because sqlx cannot decode an `int4`
//! into a JSON value.

/// Convert a sqlx row to a JSON object, decoding each column by its
/// PostgreSQL type.
///
/// A column whose value cannot be decoded becomes `null` rather than failing
/// the whole row.
pub fn row_to_json(row: &sqlx::postgres::PgRow) -> serde_json::Value {
    use sqlx::{Column, Row, TypeInfo};

    let mut map = serde_json::Map::new();

    for column in row.columns() {
        let name = column.name();
        let type_name = column.type_info().name();

        let value = match type_name {
            "INT2" | "SMALLINT" => row
                .try_get::<i16, _>(name)
                .ok()
                .map(|v| serde_json::Value::Number(v.into())),
            "INT4" | "INT" | "INTEGER" => row
                .try_get::<i32, _>(name)
                .ok()
                .map(|v| serde_json::Value::Number(v.into())),
            "INT8" | "BIGINT" => row
                .try_get::<i64, _>(name)
                .ok()
                .map(|v| serde_json::Value::Number(v.into())),
            "FLOAT4" | "REAL" => row
                .try_get::<f32, _>(name)
                .ok()
                .and_then(|v| serde_json::Number::from_f64(v as f64))
                .map(serde_json::Value::Number),
            "FLOAT8" | "DOUBLE PRECISION" => row
                .try_get::<f64, _>(name)
                .ok()
                .and_then(serde_json::Number::from_f64)
                .map(serde_json::Value::Number),
            // PostgreSQL's own row_to_json emits NUMERIC as a JSON number, and so
            // does PostgREST, so returning a string here made every numeric column
            // arrive at the client quoted.
            //
            // The value is parsed from its exact decimal text rather than through
            // f64, so a NUMERIC(20, 6) keeps the digits it was stored with. NUMERIC
            // also permits NaN, which JSON has no number for; that falls back to a
            // string, which is what row_to_json does as well.
            "NUMERIC" | "DECIMAL" => {
                row.try_get::<sqlx::types::BigDecimal, _>(name)
                    .ok()
                    .map(|v| {
                        let text = v.to_string();
                        serde_json::from_str::<serde_json::Value>(&text)
                            .ok()
                            .filter(serde_json::Value::is_number)
                            .unwrap_or(serde_json::Value::String(text))
                    })
            }
            "BOOL" | "BOOLEAN" => row
                .try_get::<bool, _>(name)
                .ok()
                .map(serde_json::Value::Bool),
            "JSON" | "JSONB" => row.try_get::<serde_json::Value, _>(name).ok(),
            "UUID" => row
                .try_get::<sqlx::types::Uuid, _>(name)
                .ok()
                .map(|v| serde_json::Value::String(v.to_string())),
            "TIMESTAMPTZ" | "TIMESTAMP WITH TIME ZONE" => row
                .try_get::<chrono::DateTime<chrono::Utc>, _>(name)
                .ok()
                .map(|v| serde_json::Value::String(v.to_rfc3339())),
            "TIMESTAMP" | "TIMESTAMP WITHOUT TIME ZONE" => row
                .try_get::<chrono::NaiveDateTime, _>(name)
                .ok()
                .map(|v| serde_json::Value::String(v.to_string())),
            "DATE" => row
                .try_get::<chrono::NaiveDate, _>(name)
                .ok()
                .map(|v| serde_json::Value::String(v.to_string())),
            "TIME" | "TIME WITHOUT TIME ZONE" => row
                .try_get::<chrono::NaiveTime, _>(name)
                .ok()
                .map(|v| serde_json::Value::String(v.to_string())),
            _ => row
                .try_get::<String, _>(name)
                .ok()
                .map(serde_json::Value::String),
        };

        map.insert(name.to_string(), value.unwrap_or(serde_json::Value::Null));
    }

    serde_json::Value::Object(map)
}
