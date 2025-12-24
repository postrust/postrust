//! INSERT statement builder.

use crate::{
    builder::SqlFragment,
    identifier::{escape_ident, from_qi, QualifiedIdentifier},
    param::SqlParam,
};

/// Builder for INSERT statements.
#[derive(Clone, Debug, Default)]
pub struct InsertBuilder {
    table: Option<SqlFragment>,
    columns: Vec<String>,
    values: Vec<Vec<SqlFragment>>,
    on_conflict: Option<OnConflict>,
    returning: Vec<SqlFragment>,
}

#[derive(Clone, Debug)]
pub enum OnConflict {
    DoNothing,
    DoUpdate {
        columns: Vec<String>,
        set: Vec<(String, SqlFragment)>,
        where_clause: Option<SqlFragment>,
    },
}

impl InsertBuilder {
    /// Create a new INSERT builder.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the target table.
    pub fn into_table(mut self, qi: &QualifiedIdentifier) -> Self {
        self.table = Some(SqlFragment::raw(from_qi(qi)));
        self
    }

    /// Set the columns to insert.
    pub fn columns(mut self, cols: Vec<String>) -> Self {
        self.columns = cols;
        self
    }

    /// Add a row of values.
    pub fn values(mut self, vals: Vec<SqlParam>) -> Self {
        let row: Vec<SqlFragment> = vals
            .into_iter()
            .map(|v| {
                let mut frag = SqlFragment::new();
                frag.push_param(v);
                frag
            })
            .collect();
        self.values.push(row);
        self
    }

    /// Add a row of raw SQL values.
    pub fn values_raw(mut self, vals: Vec<SqlFragment>) -> Self {
        self.values.push(vals);
        self
    }

    /// Set ON CONFLICT DO NOTHING.
    pub fn on_conflict_do_nothing(mut self) -> Self {
        self.on_conflict = Some(OnConflict::DoNothing);
        self
    }

    /// Set ON CONFLICT DO UPDATE.
    pub fn on_conflict_do_update(
        mut self,
        conflict_columns: Vec<String>,
        set: Vec<(String, SqlFragment)>,
    ) -> Self {
        self.on_conflict = Some(OnConflict::DoUpdate {
            columns: conflict_columns,
            set,
            where_clause: None,
        });
        self
    }

    /// Add RETURNING clause.
    pub fn returning(mut self, column: &str) -> Self {
        self.returning
            .push(SqlFragment::raw(escape_ident(column)));
        self
    }

    /// Add RETURNING * clause.
    pub fn returning_all(mut self) -> Self {
        self.returning.push(SqlFragment::raw("*"));
        self
    }

    /// Build the INSERT statement.
    pub fn build(self) -> SqlFragment {
        let mut result = SqlFragment::new();

        result.push("INSERT INTO ");

        if let Some(table) = self.table {
            result.append(table);
        }

        // Columns
        if !self.columns.is_empty() {
            result.push(" (");
            for (i, col) in self.columns.iter().enumerate() {
                if i > 0 {
                    result.push(", ");
                }
                result.push(&escape_ident(col));
            }
            result.push(")");
        }

        // VALUES
        if !self.values.is_empty() {
            result.push(" VALUES ");
            for (i, row) in self.values.into_iter().enumerate() {
                if i > 0 {
                    result.push(", ");
                }
                result.push("(");
                for (j, val) in row.into_iter().enumerate() {
                    if j > 0 {
                        result.push(", ");
                    }
                    result.append(val);
                }
                result.push(")");
            }
        } else {
            result.push(" DEFAULT VALUES");
        }

        // ON CONFLICT
        if let Some(conflict) = self.on_conflict {
            match conflict {
                OnConflict::DoNothing => {
                    result.push(" ON CONFLICT DO NOTHING");
                }
                OnConflict::DoUpdate {
                    columns,
                    set,
                    where_clause,
                } => {
                    result.push(" ON CONFLICT (");
                    for (i, col) in columns.iter().enumerate() {
                        if i > 0 {
                            result.push(", ");
                        }
                        result.push(&escape_ident(col));
                    }
                    result.push(") DO UPDATE SET ");
                    for (i, (col, val)) in set.into_iter().enumerate() {
                        if i > 0 {
                            result.push(", ");
                        }
                        result.push(&escape_ident(&col));
                        result.push(" = ");
                        result.append(val);
                    }
                    if let Some(where_sql) = where_clause {
                        result.push(" WHERE ");
                        result.append(where_sql);
                    }
                }
            }
        }

        // RETURNING
        if !self.returning.is_empty() {
            result.push(" RETURNING ");
            for (i, ret) in self.returning.into_iter().enumerate() {
                if i > 0 {
                    result.push(", ");
                }
                result.append(ret);
            }
        }

        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simple_insert() {
        let qi = QualifiedIdentifier::new("public", "users");
        let sql = InsertBuilder::new()
            .into_table(&qi)
            .columns(vec!["name".into(), "email".into()])
            .values(vec![SqlParam::text("John"), SqlParam::text("john@example.com")])
            .build();

        assert!(sql.sql().contains("INSERT INTO"));
        assert!(sql.sql().contains("VALUES"));
        assert_eq!(sql.params().len(), 2);
    }

    #[test]
    fn test_insert_returning() {
        let qi = QualifiedIdentifier::unqualified("users");
        let sql = InsertBuilder::new()
            .into_table(&qi)
            .columns(vec!["name".into()])
            .values(vec![SqlParam::text("John")])
            .returning("id")
            .build();

        assert!(sql.sql().contains("RETURNING"));
    }

    #[test]
    fn test_insert_on_conflict_nothing() {
        let qi = QualifiedIdentifier::unqualified("users");
        let sql = InsertBuilder::new()
            .into_table(&qi)
            .columns(vec!["email".into()])
            .values(vec![SqlParam::text("john@example.com")])
            .on_conflict_do_nothing()
            .build();

        assert!(sql.sql().contains("ON CONFLICT DO NOTHING"));
    }

    #[test]
    fn test_insert_upsert() {
        let qi = QualifiedIdentifier::unqualified("users");
        let mut name_val = SqlFragment::new();
        name_val.push("EXCLUDED.\"name\"");

        let sql = InsertBuilder::new()
            .into_table(&qi)
            .columns(vec!["id".into(), "name".into()])
            .values(vec![SqlParam::Int(1), SqlParam::text("John")])
            .on_conflict_do_update(vec!["id".into()], vec![("name".into(), name_val)])
            .build();

        assert!(sql.sql().contains("ON CONFLICT"));
        assert!(sql.sql().contains("DO UPDATE SET"));
    }
}
