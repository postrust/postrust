//! Mutation (INSERT/UPDATE/DELETE) query planning.

use super::types::*;
use crate::api_request::{
    ApiRequest, Mutation, Payload, PreferResolution, QualifiedIdentifier,
};
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
    ) -> Result<Self> {
        let qi = table.qualified_identifier();

        match mutation {
            Mutation::Create => Self::create_insert(request, table, qi),
            Mutation::Update => Self::create_update(request, table, qi),
            Mutation::Delete => Self::create_delete(request, table, qi),
            Mutation::SingleUpsert => Self::create_upsert(request, table, qi),
        }
    }

    /// Create an INSERT plan.
    fn create_insert(
        request: &ApiRequest,
        table: &Table,
        qi: QualifiedIdentifier,
    ) -> Result<Self> {
        let columns = get_payload_columns(request, table)?;
        let body = get_body_bytes(request)?;
        let returning = get_returning_columns(request, table);
        let apply_defaults = request.preferences.missing == crate::api_request::PreferMissing::ApplyDefaults;

        let on_conflict = request.query_params.on_conflict.as_ref().map(|cols| {
            let resolution = request
                .preferences
                .resolution
                .clone()
                .unwrap_or(PreferResolution::MergeDuplicates);
            (resolution, cols.clone())
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
    ) -> Result<Self> {
        let columns = get_payload_columns(request, table)?;
        let body = get_body_bytes(request)?;
        let where_clauses = build_mutation_where(request, table)?;
        let returning = get_returning_columns(request, table);
        let apply_defaults = request.preferences.missing == crate::api_request::PreferMissing::ApplyDefaults;

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
    fn create_delete(
        request: &ApiRequest,
        table: &Table,
        qi: QualifiedIdentifier,
    ) -> Result<Self> {
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
    ) -> Result<Self> {
        let columns = get_payload_columns(request, table)?;
        let body = get_body_bytes(request)?;
        let returning = get_returning_columns(request, table);

        // Upsert uses PK for conflict
        let on_conflict = Some((
            PreferResolution::MergeDuplicates,
            table.pk_cols.clone(),
        ));

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
) -> Result<Vec<CoercibleField>> {
    let keys = match &request.payload {
        Some(Payload::ProcessedJson { keys, .. }) => keys,
        Some(Payload::ProcessedUrlEncoded { keys, .. }) => keys,
        _ => return Ok(vec![]),
    };

    let mut columns = Vec::new();

    for key in keys {
        let column = table
            .get_column(key)
            .ok_or_else(|| Error::UnknownColumn(key.clone()))?;

        columns.push(CoercibleField::simple(key, &column.data_type));
    }

    Ok(columns)
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
                &data.iter().cloned().collect::<std::collections::HashMap<_, _>>()
            ).map_err(|e| Error::InvalidBody(e.to_string()))?;
            Ok(Some(bytes::Bytes::from(json)))
        }
        None => Ok(None),
    }
}

/// Get returning columns.
fn get_returning_columns(request: &ApiRequest, table: &Table) -> Vec<String> {
    if request.preferences.representation.needs_body() {
        table.column_names().map(|s| s.to_string()).collect()
    } else {
        // Always return PK for Location header
        table.pk_cols.clone()
    }
}

/// Build WHERE clauses for mutations.
fn build_mutation_where(
    request: &ApiRequest,
    table: &Table,
) -> Result<Vec<CoercibleLogicTree>> {
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
