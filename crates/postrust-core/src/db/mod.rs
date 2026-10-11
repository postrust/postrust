//! The database, as the request pipeline and the schema-cache loader use it.
//!
//! Two drivers answer to this interface. sqlx is the one the server and the
//! Lambda adapter run on. A Cloudflare Worker has no sqlx, and speaks the
//! PostgreSQL wire protocol over a socket of its own (issue #50).
//!
//! The interface is small, and it is shaped by what the callers need rather
//! than by what a driver offers:
//!
//! * Rows are decoded to JSON by the driver itself, with no row type in
//!   between. An intermediate value per column would be an allocation and a
//!   copy on every value of every paged or embedded read.
//! * It is used generically, never as `dyn`, so the native build calls sqlx
//!   exactly as it did before the interface existed.
//! * Every statement runs inside a transaction ([`Db::begin`]). That is where
//!   a request's role and claims are set, and nothing runs outside it.
//!
//! On wasm32 the futures are not `Send`, because nothing in a Worker is; see
//! [`MaybeSend`].

use crate::error::{DatabaseError, Error};
use postrust_sql::SqlParam;
use std::future::Future;

#[cfg(feature = "pg-socket")]
mod socket;
#[cfg(feature = "pg-socket")]
pub use socket::{Connector, PgSocketDb, SocketTx};
/// The client library [`PgSocketDb`] speaks through, for its `Config` and TLS
/// types.
#[cfg(feature = "pg-socket")]
pub use tokio_postgres;
#[cfg(feature = "sqlx")]
mod sqlx_impl;
#[cfg(feature = "sqlx")]
pub use sqlx_impl::SqlxTx;

/// `Send` natively, nothing at all on wasm32.
///
/// A native host runs a request's future on a multi-threaded runtime, which
/// needs it to be `Send`; a Worker runs it on one thread, and its socket and
/// bindings are JavaScript objects that cannot be. One bound spelled this way
/// lets the same generic code serve both.
#[cfg(not(target_arch = "wasm32"))]
pub trait MaybeSend: Send {}
#[cfg(not(target_arch = "wasm32"))]
impl<T: Send + ?Sized> MaybeSend for T {}

/// `Send` natively, nothing at all on wasm32. See the native definition.
#[cfg(target_arch = "wasm32")]
pub trait MaybeSend {}
#[cfg(target_arch = "wasm32")]
impl<T: ?Sized> MaybeSend for T {}

/// `Sync` natively, nothing at all on wasm32. See [`MaybeSend`].
#[cfg(not(target_arch = "wasm32"))]
pub trait MaybeSync: Sync {}
#[cfg(not(target_arch = "wasm32"))]
impl<T: Sync + ?Sized> MaybeSync for T {}

/// `Sync` natively, nothing at all on wasm32. See [`MaybeSend`].
#[cfg(target_arch = "wasm32")]
pub trait MaybeSync {}
#[cfg(target_arch = "wasm32")]
impl<T: ?Sized> MaybeSync for T {}

/// A boxed future, `Send` where [`MaybeSend`] is. For recursion, which an
/// `async fn` cannot do without a box.
#[cfg(not(target_arch = "wasm32"))]
pub type BoxFuture<'a, T> = std::pin::Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// A boxed future, `Send` where [`MaybeSend`] is. For recursion, which an
/// `async fn` cannot do without a box.
#[cfg(target_arch = "wasm32")]
pub type BoxFuture<'a, T> = std::pin::Pin<Box<dyn Future<Output = T> + 'a>>;

/// A database a request can open a transaction on.
pub trait Db: MaybeSend + MaybeSync {
    /// A transaction on this database.
    type Tx<'a>: Tx
    where
        Self: 'a;

    /// Open a transaction. Nothing is sent before the caller's first
    /// statement, so `SET TRANSACTION` may still follow.
    fn begin(&self) -> impl Future<Output = Result<Self::Tx<'_>, DbError>> + MaybeSend;
}

/// An open transaction. Dropped without [`Tx::commit`], it rolls back.
pub trait Tx: MaybeSend {
    /// Run a statement for its effect. Returns the rows it affected.
    fn execute(
        &mut self,
        sql: &str,
        params: &[SqlParam],
    ) -> impl Future<Output = Result<u64, DbError>> + MaybeSend;

    /// Every row as a JSON object keyed by column name, each column decoded by
    /// its PostgreSQL type -- see `row_json::row_to_json` for the rules, which
    /// every driver has to follow exactly.
    fn fetch_json(
        &mut self,
        sql: &str,
        params: &[SqlParam],
    ) -> impl Future<Output = Result<Vec<serde_json::Value>, DbError>> + MaybeSend;

    /// Every row as its columns in order, decoded by the same rules as
    /// [`Tx::fetch_json`]. For statements whose columns are read by position: a
    /// count, a plan, a pair of JSON values.
    fn fetch_values(
        &mut self,
        sql: &str,
        params: &[SqlParam],
    ) -> impl Future<Output = Result<Vec<Vec<serde_json::Value>>, DbError>> + MaybeSend;

    /// The result of a statement whose first column is a body to send as it
    /// is: a media type the schema renders itself. See [`RawRows`].
    fn fetch_raw(
        &mut self,
        sql: &str,
        params: &[SqlParam],
        count_column: &str,
    ) -> impl Future<Output = Result<RawRows, DbError>> + MaybeSend;

    /// Commit the transaction.
    fn commit(self) -> impl Future<Output = Result<(), DbError>> + MaybeSend;

    /// Run a statement for its effect, but not yet: it is sent with the next
    /// statement, ahead of it, and a failure is reported as that statement's.
    ///
    /// For the statements that set a transaction up -- `SET TRANSACTION`, the
    /// request's settings, a pre-request function -- whose only answer worth
    /// having is whether they failed. A driver that can pipeline sends them
    /// together with the query in one flight rather than waiting on each in
    /// turn; one that cannot runs them first, one by one, which is what the
    /// caller would have done anyway. Either way they run in order, before the
    /// next statement, and if one fails nothing after it does.
    fn queue(&mut self, sql: &str, params: &[SqlParam]);

    /// [`Tx::fetch_json`], and then `then` -- a statement with no parameters,
    /// whose columns are read by position, as by [`Tx::fetch_values`].
    ///
    /// Two statements whose order matters but whose second does not depend on
    /// what the first returned, so a driver that can pipeline sends both at
    /// once. For a function's response settings, read back after it ran.
    fn fetch_json_then(
        &mut self,
        sql: &str,
        params: &[SqlParam],
        then: &str,
    ) -> impl Future<Output = FetchedThen> + MaybeSend;

    /// [`Tx::fetch_json`] as the transaction's last statement, and
    /// [`Tx::commit`].
    ///
    /// For a read, where nothing that follows can change whether it should
    /// commit, so a driver that can pipeline sends the `COMMIT` with the
    /// query. A failed query is reported as it would be by
    /// [`Tx::fetch_json`], and the transaction rolls back.
    fn fetch_json_and_commit(
        self,
        sql: &str,
        params: &[SqlParam],
    ) -> impl Future<Output = Result<Vec<serde_json::Value>, DbError>> + MaybeSend;
}

/// What [`Tx::fetch_json_then`] returns: the rows, and the second statement's
/// rows, by position.
pub type FetchedThen = Result<(Vec<serde_json::Value>, Vec<Vec<serde_json::Value>>), DbError>;

/// What [`Tx::fetch_raw`] reads from a result.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct RawRows {
    /// How many rows the statement returned.
    pub rows: usize,
    /// The first row's first column, as the bytes PostgreSQL sent for it.
    /// `None` where there was no row or the value was null. A domain the
    /// driver has no decoder for arrives in text format, so a `bytea` is `\x`
    /// and hex here, and the caller decodes it.
    pub first: Option<Vec<u8>>,
    /// The first row's `count_column`, read as a `bigint`, where it has one.
    pub count: Option<i64>,
}

impl RawRows {
    /// A result read by a driver.
    pub fn new(rows: usize, first: Option<Vec<u8>>, count: Option<i64>) -> Self {
        Self { rows, first, count }
    }
}

/// A failure reported by a driver.
///
/// Its `Display` is the driver's own account, which is what gets logged.
/// Converted to [`Error`] it is the database's error where PostgreSQL sent
/// one -- SQLSTATE, message, detail, hint, constraint -- and an internal error
/// otherwise.
#[derive(Debug)]
pub struct DbError {
    message: String,
    // Boxed: a failure is rare, and every `Result` a driver returns would
    // otherwise be as large as PostgreSQL's whole error report.
    database: Option<Box<DatabaseError>>,
}

impl DbError {
    /// A failure that is not PostgreSQL's: a broken connection, a protocol or
    /// decoding fault.
    pub fn other(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            database: None,
        }
    }

    /// An error PostgreSQL reported, with `message` as the driver phrased it.
    pub fn database(message: impl Into<String>, error: DatabaseError) -> Self {
        Self {
            message: message.into(),
            database: Some(Box::new(error)),
        }
    }

    /// The error PostgreSQL reported, if it was one.
    pub fn as_database(&self) -> Option<&DatabaseError> {
        self.database.as_deref()
    }
}

impl std::fmt::Display for DbError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for DbError {}

impl From<DbError> for Error {
    fn from(error: DbError) -> Self {
        match error.database {
            Some(database) => Error::Database(*database),
            None => Error::Internal(error.message),
        }
    }
}
