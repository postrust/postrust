//! [`Db`] over a sqlx pool: the driver the server and the Lambda adapter use.

use super::{Db, DbError, FetchedThen, RawRows, Tx};
use crate::error::DatabaseError;
use crate::row_json::{row_to_json, row_to_values};
use postrust_sql::SqlParam;
use sqlx::postgres::{PgArguments, PgPool, PgRow};
use sqlx::query::Query;
use sqlx::{Postgres, Row, ValueRef};

/// A transaction on a pooled connection, which goes back to the pool when this
/// is committed or dropped.
///
/// sqlx cannot pipeline, so [`Tx::queue`]d statements run one after another
/// ahead of the next statement -- the round trips they always took. On a pool
/// next to the database each is a fraction of a millisecond.
pub struct SqlxTx {
    tx: sqlx::Transaction<'static, Postgres>,
    queued: Vec<(String, Vec<SqlParam>)>,
}

impl Db for PgPool {
    type Tx<'a> = SqlxTx;

    async fn begin(&self) -> Result<SqlxTx, DbError> {
        Ok(SqlxTx {
            tx: PgPool::begin(self).await?,
            queued: Vec::new(),
        })
    }
}

impl SqlxTx {
    /// Run what was queued, in order.
    async fn flush(&mut self) -> Result<(), DbError> {
        for (sql, params) in std::mem::take(&mut self.queued) {
            bind(sqlx::query(&sql), &params)
                .execute(&mut *self.tx)
                .await
                .map_err(|e| DbError::from(e).queued())?;
        }
        Ok(())
    }
}

impl Tx for SqlxTx {
    async fn execute(&mut self, sql: &str, params: &[SqlParam]) -> Result<u64, DbError> {
        self.flush().await?;
        Ok(bind(sqlx::query(sql), params)
            .execute(&mut *self.tx)
            .await?
            .rows_affected())
    }

    async fn fetch_json(
        &mut self,
        sql: &str,
        params: &[SqlParam],
    ) -> Result<Vec<serde_json::Value>, DbError> {
        // `into_iter` matters for large result sets: each row's buffers are
        // freed as soon as it has been converted, rather than the whole
        // `Vec<PgRow>` staying alive alongside the whole `Vec<Value>`.
        self.flush().await?;
        Ok(fetch_all(&mut self.tx, sql, params)
            .await?
            .into_iter()
            .map(|row| row_to_json(&row))
            .collect())
    }

    async fn fetch_values(
        &mut self,
        sql: &str,
        params: &[SqlParam],
    ) -> Result<Vec<Vec<serde_json::Value>>, DbError> {
        self.flush().await?;
        Ok(fetch_all(&mut self.tx, sql, params)
            .await?
            .into_iter()
            .map(|row| row_to_values(&row))
            .collect())
    }

    async fn fetch_raw(
        &mut self,
        sql: &str,
        params: &[SqlParam],
        count_column: &str,
    ) -> Result<RawRows, DbError> {
        self.flush().await?;
        let rows = fetch_all(&mut self.tx, sql, params).await?;
        let first = rows.first().and_then(|row| {
            let value = row.try_get_raw(0).ok()?;
            if value.is_null() {
                return None;
            }
            Some(value.as_bytes().ok()?.to_vec())
        });
        let count = rows
            .first()
            .and_then(|row| row.try_get::<i64, _>(count_column).ok());
        Ok(RawRows::new(rows.len(), first, count))
    }

    async fn commit(mut self) -> Result<(), DbError> {
        self.flush().await?;
        Ok(self.tx.commit().await?)
    }

    fn queue(&mut self, sql: &str, params: &[SqlParam]) {
        self.queued.push((sql.to_string(), params.to_vec()));
    }

    async fn fetch_json_then(&mut self, sql: &str, params: &[SqlParam], then: &str) -> FetchedThen {
        let rows = self.fetch_json(sql, params).await?;
        let then = self.fetch_values(then, &[]).await?;
        Ok((rows, then))
    }

    async fn fetch_json_and_commit(
        mut self,
        sql: &str,
        params: &[SqlParam],
    ) -> Result<Vec<serde_json::Value>, DbError> {
        let rows = self.fetch_json(sql, params).await?;
        self.commit().await?;
        Ok(rows)
    }
}

async fn fetch_all(
    tx: &mut sqlx::Transaction<'static, Postgres>,
    sql: &str,
    params: &[SqlParam],
) -> Result<Vec<PgRow>, sqlx::Error> {
    bind(sqlx::query(sql), params).fetch_all(&mut **tx).await
}

/// Bind each parameter with the type sqlx gives its Rust value, and let the
/// SQL's own casts decide the rest.
fn bind<'q>(
    mut query: Query<'q, Postgres, PgArguments>,
    params: &'q [SqlParam],
) -> Query<'q, Postgres, PgArguments> {
    for param in params {
        query = match param {
            SqlParam::Null => query.bind(None::<String>),
            SqlParam::Bool(b) => query.bind(b),
            SqlParam::Int(n) => query.bind(n),
            SqlParam::Float(f) => query.bind(f),
            SqlParam::Text(s) => query.bind(s),
            SqlParam::Bytes(b) => query.bind(b),
            SqlParam::Json(j) => query.bind(j),
            SqlParam::Uuid(u) => query.bind(u),
            SqlParam::Timestamp(t) => query.bind(t),
            SqlParam::Array(arr) => {
                // Convert array to Vec<String> for text arrays
                let strings: Vec<String> = arr
                    .iter()
                    .map(|p| match p {
                        SqlParam::Text(s) => s.clone(),
                        SqlParam::Int(n) => n.to_string(),
                        SqlParam::Bool(b) => b.to_string(),
                        other => format!("{:?}", other),
                    })
                    .collect();
                query.bind(strings)
            }
        };
    }

    query
}

impl From<sqlx::Error> for DbError {
    fn from(e: sqlx::Error) -> Self {
        let message = e.to_string();
        match e {
            sqlx::Error::Database(db_err) => {
                // Try to downcast to Postgres-specific error for additional details
                let (details, hint) = db_err
                    .try_downcast_ref::<sqlx::postgres::PgDatabaseError>()
                    .map(|pg_err| {
                        (
                            pg_err.detail().map(String::from),
                            pg_err.hint().map(String::from),
                        )
                    })
                    .unwrap_or((None, None));

                DbError::database(
                    message,
                    DatabaseError {
                        code: db_err.code().map(|c| c.to_string()).unwrap_or_default(),
                        message: db_err.message().to_string(),
                        details,
                        hint,
                        constraint: db_err.constraint().map(|s| s.to_string()),
                        table: db_err.table().map(|s| s.to_string()),
                        column: None,
                    },
                )
            }
            // The connection, not a statement on it: it broke, it could not be
            // had from the pool in time, or the pool is gone.
            sqlx::Error::Io(_)
            | sqlx::Error::Tls(_)
            | sqlx::Error::PoolTimedOut
            | sqlx::Error::PoolClosed
            | sqlx::Error::WorkerCrashed => DbError::connection(message),
            _ => DbError::other(message),
        }
    }
}
