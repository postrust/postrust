//! Relationship types for resource embedding.

use crate::api_request::QualifiedIdentifier;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// A relationship between tables.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Relationship {
    /// Foreign key relationship
    ForeignKey {
        /// Source table
        table: QualifiedIdentifier,
        /// Target table
        foreign_table: QualifiedIdentifier,
        /// Whether this is a self-referential relationship
        is_self: bool,
        /// Relationship cardinality
        cardinality: Cardinality,
        /// Whether the source is a view
        table_is_view: bool,
        /// Whether the target is a view
        foreign_table_is_view: bool,
        /// FK constraint name
        constraint_name: String,
    },
    /// Computed relationship (from a function)
    Computed {
        /// Function that computes the relationship
        function: QualifiedIdentifier,
        /// Source table
        table: QualifiedIdentifier,
        /// Target table
        foreign_table: QualifiedIdentifier,
        /// Alias for the relationship
        table_alias: QualifiedIdentifier,
        /// Whether this returns a single row
        to_one: bool,
        /// Whether this is self-referential
        is_self: bool,
        /// The name of the parameter that takes the parent's row.
        ///
        /// Usually the only one, and then the call is positional. A function
        /// that also takes a search term or the caller's session has the row
        /// somewhere among its parameters and is called by name, which is why
        /// the name is kept rather than the position.
        #[serde(default)]
        row_argument: Option<String>,
    },
}

impl Relationship {
    /// Get the foreign table for this relationship.
    pub fn foreign_table(&self) -> &QualifiedIdentifier {
        match self {
            Self::ForeignKey { foreign_table, .. } => foreign_table,
            Self::Computed { foreign_table, .. } => foreign_table,
        }
    }

    /// Check if this relationship returns a single row.
    pub fn is_to_one(&self) -> bool {
        match self {
            Self::ForeignKey { cardinality, .. } => matches!(
                cardinality,
                Cardinality::M2O { .. } | Cardinality::O2O { .. }
            ),
            Self::Computed { to_one, .. } => *to_one,
        }
    }

    /// The name of the foreign key constraint behind this relationship.
    ///
    /// Empty for a many-to-many, which is two constraints rather than one, and
    /// for a computed relationship, which has none.
    pub fn constraint_name(&self) -> &str {
        match self {
            Self::ForeignKey { cardinality, .. } => match cardinality {
                Cardinality::O2M { constraint, .. }
                | Cardinality::M2O { constraint, .. }
                | Cardinality::O2O { constraint, .. } => constraint,
                Cardinality::M2M(_) => "",
            },
            Self::Computed { .. } => "",
        }
    }

    /// How PostgREST names this relationship's cardinality.
    pub fn cardinality_name(&self) -> &'static str {
        match self {
            Self::ForeignKey { cardinality, .. } => match cardinality {
                Cardinality::O2M { .. } => "one-to-many",
                Cardinality::M2O { .. } => "many-to-one",
                Cardinality::O2O { .. } => "one-to-one",
                Cardinality::M2M(_) => "many-to-many",
            },
            Self::Computed { to_one, .. } => match to_one {
                true => "many-to-one",
                false => "one-to-many",
            },
        }
    }

    /// How this relationship reads when several of them cannot be told apart.
    ///
    /// `message_sender_fkey using message(sender) and person_detail(id)` --
    /// the constraint, then each side with the columns it joins on, which is
    /// what a client needs to write the hint that disambiguates it.
    pub fn describe(&self) -> String {
        match self {
            // A junction is described by the table it joins through and the
            // two constraints that make it one -- there is no single
            // constraint to name, and the empty string that came out instead
            // told a client nothing about which relationship it had found.
            Self::ForeignKey {
                cardinality: Cardinality::M2M(junction),
                ..
            } => {
                // Both constraints belong to the junction table, so both are
                // named with the junction's own columns. The two lists are
                // oriented in opposite directions -- `source_columns` runs
                // (source, through) and `target_columns` runs (through,
                // target) -- so the junction's side is the second element of
                // one and the first of the other. Taking the second of both
                // described the far constraint with the column it points at
                // rather than the column it is on:
                // `whatev_jobs_project_id_1_fkey(id)` for a constraint that is
                // on `project_id_1`.
                let joined = |columns: Vec<String>| columns.join(", ");
                format!(
                    "{} using {}({}) and {}({})",
                    junction.table.name,
                    junction.constraint1,
                    joined(
                        junction
                            .source_columns
                            .iter()
                            .map(|(_, through)| through.clone())
                            .collect()
                    ),
                    junction.constraint2,
                    joined(
                        junction
                            .target_columns
                            .iter()
                            .map(|(through, _)| through.clone())
                            .collect()
                    ),
                )
            }
            Self::ForeignKey {
                table,
                foreign_table,
                cardinality,
                ..
            } => {
                let columns = cardinality.columns();
                let local = columns
                    .iter()
                    .map(|(local, _)| local.clone())
                    .collect::<Vec<_>>()
                    .join(", ");
                let foreign = columns
                    .iter()
                    .map(|(_, foreign)| foreign.clone())
                    .collect::<Vec<_>>()
                    .join(", ");
                format!(
                    "{} using {}({}) and {}({})",
                    self.constraint_name(),
                    table.name,
                    local,
                    foreign_table.name,
                    foreign
                )
            }
            Self::Computed { function, .. } => function.name.clone(),
        }
    }

    /// The `!hint` that picks this relationship out from its rivals.
    ///
    /// The constraint for a foreign key, the junction table for a
    /// many-to-many: whichever name is the one a client would have to write to
    /// mean this relationship and no other.
    pub fn disambiguator(&self) -> String {
        match self {
            Self::ForeignKey {
                cardinality: Cardinality::M2M(junction),
                ..
            } => junction.table.name.clone(),
            Self::ForeignKey {
                constraint_name, ..
            } => constraint_name.clone(),
            Self::Computed { function, .. } => function.name.clone(),
        }
    }

    /// Whether a hint names this relationship.
    ///
    /// A hint may be the constraint, the column that joins them, or the table
    /// on the far side -- whichever the client found unambiguous.
    pub fn matches_hint(&self, hint: &str) -> bool {
        // Both columns of the join count, because which side carries the
        // telling name depends on the direction: `whatev_jobs!site_id_1` names
        // a column on the jobs, which is the far side when the request starts
        // from the sites.
        //
        // A self-referential relationship appears twice, once each way, and
        // both directions join on the same pair of columns -- so matching
        // either side would match both and make every hint ambiguous. The far
        // side is the telling one: `family_tree!parent` asks for the rows
        // whose `parent` points here.
        let self_referential = matches!(self, Self::ForeignKey { is_self, .. } if *is_self);
        let columns_match = self
            .join_columns()
            .iter()
            .any(|(local, foreign)| foreign == hint || (!self_referential && local == hint));

        self.constraint_name() == hint
            || self.foreign_table().name == hint
            || columns_match
            || match self {
                Self::Computed { function, .. } => function.name == hint,
                // A junction is named by the table it joins through, or by
                // either of the constraints that make it one.
                Self::ForeignKey {
                    cardinality: Cardinality::M2M(junction),
                    ..
                } => {
                    junction.table.name == hint
                        || junction.constraint1 == hint
                        || junction.constraint2 == hint
                        || junction
                            .source_columns
                            .iter()
                            .chain(junction.target_columns.iter())
                            .any(|(a, b)| a == hint || b == hint)
                }
                Self::ForeignKey { .. } => false,
            }
    }

    /// Whether this is a self-referential foreign key.
    pub fn is_self_referential(&self) -> bool {
        matches!(self, Self::ForeignKey { is_self, .. } if *is_self)
    }

    /// Whether this relationship yields many rows per row of its own table.
    pub fn is_one_to_many(&self) -> bool {
        matches!(
            self,
            Self::ForeignKey {
                cardinality: Cardinality::O2M { .. },
                ..
            }
        )
    }

    /// The single column this side joins on, where it joins on exactly one.
    pub fn single_local_column(&self) -> Option<&str> {
        match self.join_columns().len() {
            1 => match self {
                Self::ForeignKey { cardinality, .. } => match cardinality {
                    Cardinality::O2M { columns, .. }
                    | Cardinality::M2O { columns, .. }
                    | Cardinality::O2O { columns, .. } => Some(columns[0].0.as_str()),
                    Cardinality::M2M(_) => None,
                },
                Self::Computed { .. } => None,
            },
            _ => None,
        }
    }

    /// The single column the other side joins on, where there is exactly one.
    pub fn single_foreign_column(&self) -> Option<&str> {
        match self {
            Self::ForeignKey { cardinality, .. } => match cardinality {
                Cardinality::O2M { columns, .. }
                | Cardinality::M2O { columns, .. }
                | Cardinality::O2O { columns, .. }
                    if columns.len() == 1 =>
                {
                    Some(columns[0].1.as_str())
                }
                _ => None,
            },
            Self::Computed { .. } => None,
        }
    }

    /// Get the join columns for this relationship.
    pub fn join_columns(&self) -> Vec<(String, String)> {
        match self {
            Self::ForeignKey { cardinality, .. } => cardinality.columns(),
            Self::Computed { .. } => vec![],
        }
    }
}

/// Relationship cardinality.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Cardinality {
    /// One-to-Many: parent has many children
    O2M {
        constraint: String,
        columns: Vec<(String, String)>,
    },
    /// Many-to-One: child has one parent
    M2O {
        constraint: String,
        columns: Vec<(String, String)>,
    },
    /// One-to-One
    O2O {
        constraint: String,
        columns: Vec<(String, String)>,
        /// Whether this table is the parent in the relationship
        is_parent: bool,
    },
    /// Many-to-Many (via junction table)
    M2M(Junction),
}

impl Cardinality {
    /// Get the join columns as `(local column, foreign column)` pairs.
    ///
    /// "Local" is the table the relationship is stored under, so a pair can be
    /// used directly to join from the local table to the foreign one without
    /// having to know the cardinality.
    pub fn columns(&self) -> Vec<(String, String)> {
        match self {
            Self::O2M { columns, .. } => columns.clone(),
            Self::M2O { columns, .. } => columns.clone(),
            Self::O2O { columns, .. } => columns.clone(),
            Self::M2M(junction) => junction.source_columns(),
        }
    }

    /// Get the constraint name.
    pub fn constraint_name(&self) -> &str {
        match self {
            Self::O2M { constraint, .. } => constraint,
            Self::M2O { constraint, .. } => constraint,
            Self::O2O { constraint, .. } => constraint,
            Self::M2M(junction) => &junction.constraint1,
        }
    }
}

/// Junction table for M2M relationships.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Junction {
    /// The junction table
    pub table: QualifiedIdentifier,
    /// FK constraint from junction to source
    pub constraint1: String,
    /// FK constraint from junction to target
    pub constraint2: String,
    /// Columns linking source to junction
    pub source_columns: Vec<(String, String)>,
    /// Columns linking junction to target
    pub target_columns: Vec<(String, String)>,
}

impl Junction {
    /// Get the source-side join columns.
    pub fn source_columns(&self) -> Vec<(String, String)> {
        self.source_columns.clone()
    }

    /// Get the target-side join columns.
    pub fn target_columns(&self) -> Vec<(String, String)> {
        self.target_columns.clone()
    }
}

/// Map of (table, schema) to relationships.
pub type RelationshipsMap = HashMap<(QualifiedIdentifier, String), Vec<Relationship>>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_relationship_foreign_table() {
        let rel = Relationship::ForeignKey {
            table: QualifiedIdentifier::new("public", "orders"),
            foreign_table: QualifiedIdentifier::new("public", "users"),
            is_self: false,
            cardinality: Cardinality::M2O {
                constraint: "orders_user_id_fkey".into(),
                columns: vec![("user_id".into(), "id".into())],
            },
            table_is_view: false,
            foreign_table_is_view: false,
            constraint_name: "orders_user_id_fkey".into(),
        };

        assert_eq!(rel.foreign_table().name, "users");
        assert!(rel.is_to_one());
    }

    #[test]
    fn test_cardinality_columns() {
        let card = Cardinality::O2M {
            constraint: "users_id_fkey".into(),
            columns: vec![("id".into(), "user_id".into())],
        };

        let cols = card.columns();
        assert_eq!(cols.len(), 1);
        assert_eq!(cols[0], ("id".into(), "user_id".into()));
    }
}

/// A user-defined renderer for a media type.
///
/// PostgREST lets a schema override how a media type is produced by declaring
/// an aggregate whose state type is a domain named after that media type --
/// `create domain "application/geo+json" as jsonb` and an aggregate over the
/// table's rows returning it. The aggregate is then applied to the rows the
/// request selected and its result is the whole response body.
#[derive(Clone, Debug)]
pub struct MediaHandler {
    /// The aggregate to apply.
    pub aggregate: QualifiedIdentifier,
    /// The table it renders, or `None` when it takes `anyelement` and so
    /// renders any of them.
    pub table: Option<QualifiedIdentifier>,
    /// The type the handler's media-type domain is a domain over, which is
    /// what says whether its output is bytes or text.
    pub base_type: String,
}

/// Media handlers by (schema, media type).
pub type MediaHandlerMap = std::collections::HashMap<(String, String), Vec<MediaHandler>>;
