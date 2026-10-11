//! [`Db`] over a socket the host opens: the PostgreSQL wire protocol through
//! `tokio-postgres`, with no runtime, pool or TLS of its own.
//!
//! This is the driver a Cloudflare Worker uses (issue #50). There, the socket is
//! a `worker::Socket` to Hyperdrive, and the connection is driven by
//! `spawn_local`. Natively it runs over a TCP stream, which is how it is proved
//! against [the sqlx driver](super::SqlxTx) -- the two must answer every request
//! alike.
//!
//! Every [`Db::begin`] opens a connection of its own, and the transaction owns
//! it. A Worker may not share an I/O object between requests, and Hyperdrive
//! pools on its side, so there is nothing to keep. A transaction dropped
//! without [`Tx::commit`] closes its connection, and PostgreSQL rolls it back.
//!
//! Statements are pipelined where the caller allows it. The `BEGIN`, every
//! [`Tx::queue`]d statement and the next real one go out together, and the
//! answers are read in order afterwards; a read's `COMMIT` can go with its
//! query ([`Tx::fetch_json_and_commit`]). Over a network every statement waited
//! on is a round trip, so a plain read is one flight after the connection is
//! open where it was five, and a write two where it was five or six. Each
//! statement has its own `Sync`, so they are still separate statements: when
//! one fails, PostgreSQL refuses the rest of the transaction, and the error
//! reported is the first one, as if they had been sent one at a time.
//!
//! A statement whose result has a type PostgreSQL does not build in -- an
//! enum, a domain, a composite, an extension's type -- is prepared before it
//! is run. `tokio-postgres` looks such a type up in the catalog when the
//! result's description arrives, and for an unprepared statement that is
//! after the statement has run: if it failed, the transaction is aborted, the
//! lookup fails with `25P02`, and the statement's own error is lost. Prepared
//! first, the lookups run before anything can have failed. A flight stops
//! short of such a statement, which is prepared once those before it have
//! answered and starts the next flight. That costs round trips, so which
//! statements need it is remembered (`ResultTypes`): one whose columns are
//! all built in is pipelined like any other, and one not seen before is
//! prepared, the first time, to find out.
//!
//! Two things have to match sqlx exactly, and both are done by hand rather
//! than left to `tokio-postgres`:
//!
//! * **Parameters are typed as sqlx types them.** sqlx sends each parameter
//!   with the OID of its Rust type and lets the SQL's casts decide.
//!   `query_typed` sends the same OIDs, so PostgreSQL resolves the statement
//!   exactly as it does for sqlx. A plain `query` would prepare first and then
//!   refuse, say, an `i64` for an `int4` slot.
//! * **Rows are decoded by `row_json`'s rules**, from the bytes PostgreSQL
//!   sent, with the conversions sqlx applies. One example: a `numeric` keeps
//!   the scale of its base-10000 digits, so `0.1` is `0.1000`.

use super::{BoxFuture, Db, DbError, FetchedThen, MaybeSend, MaybeSync, RawRows, Tx};
use crate::error::DatabaseError;
use postrust_sql::SqlParam;
use std::collections::HashMap;
use std::future::Future;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio_postgres::tls::TlsConnect;
use tokio_postgres::types::{FromSql, ToSql, Type};
use tokio_postgres::{Client, Row, Statement};

/// How a [`PgSocketDb`] reaches PostgreSQL: the host's socket, the host's TLS,
/// and the host's way of running a background task.
pub trait Connector: MaybeSend + MaybeSync {
    /// The socket.
    type Stream: AsyncRead + AsyncWrite + Unpin + MaybeSend + 'static;
    /// TLS over it, or `tokio_postgres::NoTls`.
    type Tls: TlsConnect<Self::Stream> + MaybeSend;

    /// Open a socket to the server.
    fn connect(&self) -> impl Future<Output = std::io::Result<Self::Stream>> + MaybeSend;

    /// The TLS to negotiate on a new socket.
    fn tls(&self) -> Self::Tls;

    /// Run a connection's I/O until it closes: `tokio::spawn` natively,
    /// `wasm_bindgen_futures::spawn_local` in a Worker.
    fn spawn(&self, connection: BoxFuture<'static, ()>);

    /// Wait for `duration`: `tokio::time::sleep` natively. It bounds how long
    /// opening a connection may take (see [`PgSocketDb::new`]); there is no
    /// timer that works on every host, so each brings its own. One that does
    /// not -- this default never wakes -- has no connect timeout.
    fn sleep(&self, duration: Duration) -> impl Future<Output = ()> + MaybeSend {
        let _ = duration;
        std::future::pending()
    }
}

/// How long opening a connection may take when the configuration does not
/// say (`connect_timeout`): the socket, the startup and the authentication.
pub const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// A database reached over a socket per transaction. See the module docs.
pub struct PgSocketDb<C> {
    connector: C,
    config: tokio_postgres::Config,
}

impl<C: Connector> PgSocketDb<C> {
    /// A database at `config`, reached through `connector`. The host and port
    /// in `config` are not used to connect -- the connector decides those --
    /// only the user, password, database and options, `sslmode` (with the
    /// connector's TLS), and `connect_timeout`, which bounds opening a
    /// connection when the connector has a timer ([`Connector::sleep`]),
    /// [`DEFAULT_CONNECT_TIMEOUT`] where it is not set.
    pub fn new(connector: C, config: tokio_postgres::Config) -> Self {
        Self { connector, config }
    }

    /// The connector, for a host that has to know when the connections it
    /// spawned have closed -- a Worker must not leave one running past the
    /// request that opened it.
    pub fn connector(&self) -> &C {
        &self.connector
    }
}

/// A transaction on its own connection.
pub struct SocketTx {
    client: Client,
    /// Why the connection ended, if it ended with an error of its own.
    closed: Arc<Mutex<Option<DbError>>>,
    /// Statements to send ahead of the next one: the `BEGIN`, and whatever
    /// was [`Tx::queue`]d.
    queued: Vec<(String, Vec<SqlParam>)>,
}

impl<C> Db for PgSocketDb<C>
where
    C: Connector,
    <C::Tls as TlsConnect<C::Stream>>::Stream: MaybeSend + 'static,
    <C::Tls as TlsConnect<C::Stream>>::Future: MaybeSend,
{
    type Tx<'a>
        = SocketTx
    where
        Self: 'a;

    async fn begin(&self) -> Result<SocketTx, DbError> {
        let timeout = self
            .config
            .get_connect_timeout()
            .copied()
            .unwrap_or(DEFAULT_CONNECT_TIMEOUT);
        let connect = std::pin::pin!(async {
            let stream =
                self.connector.connect().await.map_err(|e| {
                    DbError::other(format!("error connecting to the database: {e}"))
                })?;
            Ok::<_, DbError>(
                self.config
                    .connect_raw(stream, self.connector.tls())
                    .await?,
            )
        });
        let expired = std::pin::pin!(self.connector.sleep(timeout));
        let (client, connection) = match futures::future::select(connect, expired).await {
            futures::future::Either::Left((connected, _)) => connected?,
            futures::future::Either::Right(_) => {
                return Err(DbError::other(format!(
                    "timed out after {timeout:?} connecting to the database"
                )))
            }
        };
        let closed = Arc::new(Mutex::new(None));
        let died = closed.clone();
        self.connector.spawn(Box::pin(async move {
            // A connection ends with an error of its own when PostgreSQL ends
            // it between statements -- an administrator's shutdown, an idle
            // transaction's timeout. The client sees only that it is closed,
            // so the reason is kept for it to report. Kept before the
            // connection is dropped, which is what closes the client.
            let mut connection = std::pin::pin!(connection);
            if let Err(e) = connection.as_mut().await {
                if let Ok(mut died) = died.lock() {
                    *died = Some(DbError::from(e));
                }
            }
        }));
        // Sent with the first statement, not on its own.
        Ok(SocketTx {
            client,
            closed,
            queued: vec![("BEGIN".to_string(), Vec::new())],
        })
    }
}

impl Tx for SocketTx {
    async fn execute(&mut self, sql: &str, params: &[SqlParam]) -> Result<u64, DbError> {
        let [(_, affected)] = self.send([(sql, params)]).await?;
        Ok(affected)
    }

    async fn fetch_json(
        &mut self,
        sql: &str,
        params: &[SqlParam],
    ) -> Result<Vec<serde_json::Value>, DbError> {
        Ok(self
            .query(sql, params)
            .await?
            .iter()
            .map(row_to_json)
            .collect())
    }

    async fn fetch_values(
        &mut self,
        sql: &str,
        params: &[SqlParam],
    ) -> Result<Vec<Vec<serde_json::Value>>, DbError> {
        Ok(self
            .query(sql, params)
            .await?
            .iter()
            .map(row_to_values)
            .collect())
    }

    async fn fetch_raw(
        &mut self,
        sql: &str,
        params: &[SqlParam],
        count_column: &str,
    ) -> Result<RawRows, DbError> {
        let rows = self.query(sql, params).await?;
        let first = rows
            .first()
            .and_then(|row| row.try_get::<_, Raw>(0).ok()?.0.map(<[u8]>::to_vec));
        // The last column of the name, as sqlx reads one (see `row_to_json`).
        let count = rows.first().and_then(|row| {
            let index = row
                .columns()
                .iter()
                .rposition(|c| c.name() == count_column)?;
            match row.columns()[index].type_().oid() {
                oid::INT8 => be_bytes(raw(row, index)?).map(i64::from_be_bytes),
                _ => None,
            }
        });
        Ok(RawRows::new(rows.len(), first, count))
    }

    async fn commit(mut self) -> Result<(), DbError> {
        self.send([("COMMIT", &[][..])]).await?;
        Ok(())
    }

    fn queue(&mut self, sql: &str, params: &[SqlParam]) {
        self.queued.push((sql.to_string(), params.to_vec()));
    }

    async fn fetch_json_then(&mut self, sql: &str, params: &[SqlParam], then: &str) -> FetchedThen {
        let [(rows, _), (then, _)] = self.send([(sql, params), (then, &[][..])]).await?;
        Ok((
            rows.iter().map(row_to_json).collect(),
            then.iter().map(row_to_values).collect(),
        ))
    }

    async fn fetch_json_and_commit(
        mut self,
        sql: &str,
        params: &[SqlParam],
    ) -> Result<Vec<serde_json::Value>, DbError> {
        let [(rows, _), _] = self.send([(sql, params), ("COMMIT", &[][..])]).await?;
        Ok(rows.iter().map(row_to_json).collect())
    }
}

impl SocketTx {
    async fn query(&mut self, sql: &str, params: &[SqlParam]) -> Result<Vec<Row>, DbError> {
        let [(rows, _)] = self.send([(sql, params)]).await?;
        Ok(rows)
    }

    /// Send what was queued and then `statements`, and read every answer.
    /// Each statement's rows and the rows it affected, for `statements` only;
    /// the first failure, in the order they were sent, if any failed.
    ///
    /// They go out in as few flights as they can: all at once, unless one of
    /// `statements` has to be prepared first (see the module docs). That one
    /// is prepared once everything before it has answered, and starts the
    /// next flight -- a statement sent after it would otherwise run before
    /// it. What was queued is never prepared -- its rows are never read --
    /// and nor is a `COMMIT`.
    async fn send<const N: usize>(
        &mut self,
        statements: [(&str, &[SqlParam]); N],
    ) -> Result<[(Vec<Row>, u64); N], DbError> {
        let queued = std::mem::take(&mut self.queued);
        let all: Vec<(&str, &[SqlParam])> = queued
            .iter()
            .map(|(sql, params)| (sql.as_str(), params.as_slice()))
            .chain(statements)
            .collect();
        let prepare_first = |index: usize| {
            let sql = all[index].0;
            index >= queued.len() && sql != "COMMIT" && !RESULT_TYPES.built_in(sql)
        };
        let mut answers = Vec::with_capacity(all.len());
        let mut start = 0;
        while start < all.len() {
            let end = (start + 1..all.len())
                .find(|&index| prepare_first(index))
                .unwrap_or(all.len());
            let mut prepared = match prepare_first(start) {
                true => Some(self.prepare(all[start].0, all[start].1).await?),
                false => None,
            };
            let client = &self.client;
            let futures: Vec<_> = all[start..end]
                .iter()
                .map(|(sql, params)| Box::pin(run(client, sql, params, prepared.take())))
                .collect();
            for answer in in_order(futures).await {
                answers.push(answer.map_err(|e| self.error(e))?);
            }
            start = end;
        }
        // The queued statements' answers mattered only if they failed.
        answers
            .split_off(queued.len())
            .try_into()
            .map_err(|_| DbError::other("a pipelined statement went unanswered"))
    }

    /// `sql` prepared with its parameters' types, and its result's types
    /// looked up and remembered.
    async fn prepare(&self, sql: &str, params: &[SqlParam]) -> Result<Statement, DbError> {
        let types: Vec<Type> = Bound::new(params).iter().map(|(_, ty)| ty).collect();
        let statement = self
            .client
            .prepare_typed(sql, &types)
            .await
            .map_err(|e| self.error(e))?;
        RESULT_TYPES.learn(sql, statement.columns());
        Ok(statement)
    }

    /// A failure, as the caller should hear of it: where the connection was
    /// closed under it, why it was.
    fn error(&self, e: tokio_postgres::Error) -> DbError {
        if e.is_closed() {
            if let Some(why) = self.closed.lock().ok().and_then(|mut why| why.take()) {
                return why;
            }
        }
        e.into()
    }
}

/// Whether a statement's columns are all of types PostgreSQL builds in, by
/// its SQL, for every statement this process has run on a socket.
///
/// Process-wide rather than per [`PgSocketDb`], because a Worker makes a
/// `PgSocketDb` per request. Nothing goes wrong when it is out of date: a
/// statement wrongly thought built-in runs as before this existed, with its
/// errors liable to the `25P02` described in the module docs, and one wrongly
/// thought otherwise costs a round trip. A statement's types are learned
/// again every time it runs.
struct ResultTypes(Mutex<HashMap<Box<str>, bool>>);

/// How many statements [`ResultTypes`] remembers before it starts over. The
/// SQL a request builds depends on its columns and filters, so there is no
/// fixed set of them.
const RESULT_TYPES_KEPT: usize = 1024;

static RESULT_TYPES: LazyLock<ResultTypes> =
    LazyLock::new(|| ResultTypes(Mutex::new(HashMap::new())));

impl ResultTypes {
    /// Whether `sql` is known to return only built-in types.
    fn built_in(&self, sql: &str) -> bool {
        self.0
            .lock()
            .is_ok_and(|known| known.get(sql).copied().unwrap_or_default())
    }

    /// What `sql` returned, as its result's `columns` describe it.
    fn learn(&self, sql: &str, columns: &[tokio_postgres::Column]) {
        let built_in = columns
            .iter()
            .all(|c| Type::from_oid(c.type_().oid()).is_some());
        let Ok(mut known) = self.0.lock() else {
            return;
        };
        if known.get(sql) == Some(&built_in) {
            return;
        }
        if known.len() >= RESULT_TYPES_KEPT {
            known.clear();
        }
        known.insert(sql.into(), built_in);
    }
}

/// One statement: its rows, and the rows it affected. `prepared` is the
/// statement, where it had to be prepared first.
async fn run(
    client: &Client,
    sql: &str,
    params: &[SqlParam],
    prepared: Option<Statement>,
) -> Result<(Vec<Row>, u64), tokio_postgres::Error> {
    let bound = Bound::new(params);
    let stream = match &prepared {
        Some(statement) => {
            client
                .query_raw(statement, bound.iter().map(|(value, _)| value))
                .await?
        }
        None => client.query_typed_raw(sql, bound.iter()).await?,
    };
    futures::pin_mut!(stream);
    let mut rows = Vec::new();
    while let Some(row) = futures::StreamExt::next(&mut stream).await.transpose()? {
        rows.push(row);
    }
    // A schema changed since: the column has a type of its own now.
    if let (None, Some(row)) = (&prepared, rows.first()) {
        RESULT_TYPES.learn(sql, row.columns());
    }
    Ok((rows, stream.rows_affected().unwrap_or_default()))
}

/// Every future to completion, each polled for the first time in the order
/// given, and their outputs in that order.
///
/// The order is the point. A statement's messages are written to the
/// connection when its future is first polled, so polling them in turn sends
/// them in turn, all before any answer is waited on -- and PostgreSQL runs
/// them in the order it received them. `join_all` makes no promise about the
/// order of its first polls once there are more than a few futures.
async fn in_order<F: Future + Unpin>(futures: Vec<F>) -> Vec<F::Output> {
    let mut slots: Vec<(F, Option<F::Output>)> = futures.into_iter().map(|f| (f, None)).collect();
    std::future::poll_fn(|cx| {
        let mut pending = false;
        for (future, output) in slots.iter_mut().filter(|(_, output)| output.is_none()) {
            match std::pin::Pin::new(future).poll(cx) {
                std::task::Poll::Ready(value) => *output = Some(value),
                std::task::Poll::Pending => pending = true,
            }
        }
        match pending {
            true => std::task::Poll::Pending,
            false => std::task::Poll::Ready(()),
        }
    })
    .await;
    slots.into_iter().filter_map(|(_, output)| output).collect()
}

/// Parameters as sqlx binds them: each with the type of its Rust value.
struct Bound(Vec<(Box<dyn ToSql + Sync + Send>, Type)>);

impl Bound {
    fn new(params: &[SqlParam]) -> Self {
        Self(params.iter().map(bind).collect())
    }

    fn iter(&self) -> impl ExactSizeIterator<Item = (&(dyn ToSql + Sync), Type)> {
        self.0
            .iter()
            .map(|(value, ty)| (&**value as &(dyn ToSql + Sync), ty.clone()))
    }
}

/// One parameter, typed as sqlx's `bind` types it (see `sqlx_impl::bind`).
fn bind(param: &SqlParam) -> (Box<dyn ToSql + Sync + Send>, Type) {
    match param {
        SqlParam::Null => (Box::new(None::<String>), Type::TEXT),
        SqlParam::Bool(b) => (Box::new(*b), Type::BOOL),
        SqlParam::Int(n) => (Box::new(*n), Type::INT8),
        SqlParam::Float(f) => (Box::new(*f), Type::FLOAT8),
        SqlParam::Text(s) => (Box::new(s.clone()), Type::TEXT),
        SqlParam::Bytes(b) => (Box::new(b.clone()), Type::BYTEA),
        SqlParam::Json(j) => (Box::new(j.clone()), Type::JSONB),
        SqlParam::Uuid(u) => (Box::new(*u), Type::UUID),
        SqlParam::Timestamp(t) => (Box::new(*t), Type::TIMESTAMPTZ),
        SqlParam::Array(arr) => {
            let strings: Vec<String> = arr
                .iter()
                .map(|p| match p {
                    SqlParam::Text(s) => s.clone(),
                    SqlParam::Int(n) => n.to_string(),
                    SqlParam::Bool(b) => b.to_string(),
                    other => format!("{:?}", other),
                })
                .collect();
            (Box::new(strings), Type::TEXT_ARRAY)
        }
    }
}

/// A column's bytes as PostgreSQL sent them, whatever its type.
struct Raw<'a>(Option<&'a [u8]>);

impl<'a> FromSql<'a> for Raw<'a> {
    fn from_sql(_: &Type, raw: &'a [u8]) -> Result<Self, Box<dyn std::error::Error + Sync + Send>> {
        Ok(Raw(Some(raw)))
    }

    fn from_sql_null(_: &Type) -> Result<Self, Box<dyn std::error::Error + Sync + Send>> {
        Ok(Raw(None))
    }

    fn accepts(_: &Type) -> bool {
        true
    }
}

fn raw(row: &Row, index: usize) -> Option<&[u8]> {
    row.try_get::<_, Raw>(index).ok()?.0
}

fn be_bytes<const N: usize>(bytes: &[u8]) -> Option<[u8; N]> {
    bytes.try_into().ok()
}

/// `row_json::row_to_json`, for a `tokio-postgres` row. By name, so that two
/// columns sharing a name both read the last of them, as sqlx's
/// `try_get(name)` does: its index of names is a map later columns overwrite.
fn row_to_json(row: &Row) -> serde_json::Value {
    let columns = row.columns();
    let mut map = serde_json::Map::new();
    for column in columns {
        let index = columns
            .iter()
            .rposition(|c| c.name() == column.name())
            .unwrap_or_default();
        let value = decode(columns[index].type_(), raw(row, index));
        map.insert(column.name().to_string(), value);
    }
    serde_json::Value::Object(map)
}

/// `row_json::row_to_values`, for a `tokio-postgres` row.
fn row_to_values(row: &Row) -> Vec<serde_json::Value> {
    row.columns()
        .iter()
        .enumerate()
        .map(|(index, column)| decode(column.type_(), raw(row, index)))
        .collect()
}

/// One value, decoded by `row_json`'s rules from its binary form.
///
/// A value that would fail to decode under sqlx is null here too: sqlx's
/// `try_get(..).ok()` turns the failure into `None`. That covers a type sqlx
/// will not read as a string (an enum, an array, an interval), a `numeric`
/// NaN, and a timestamp out of chrono's range, which sqlx itself would panic
/// on rather than decode.
fn decode(ty: &Type, raw: Option<&[u8]>) -> serde_json::Value {
    use serde_json::Value;

    let Some(raw) = raw else {
        return Value::Null;
    };
    let number = |n: serde_json::Number| Some(Value::Number(n));
    let value = match ty.oid() {
        oid::INT2 => be_bytes(raw)
            .map(i16::from_be_bytes)
            .and_then(|v| number(v.into())),
        oid::INT4 => be_bytes(raw)
            .map(i32::from_be_bytes)
            .and_then(|v| number(v.into())),
        oid::INT8 => be_bytes(raw)
            .map(i64::from_be_bytes)
            .and_then(|v| number(v.into())),
        oid::FLOAT4 => be_bytes(raw)
            .map(f32::from_be_bytes)
            .and_then(|v| serde_json::Number::from_f64(v as f64))
            .map(Value::Number),
        oid::FLOAT8 => be_bytes(raw)
            .map(f64::from_be_bytes)
            .and_then(serde_json::Number::from_f64)
            .map(Value::Number),
        oid::NUMERIC => numeric(raw).map(|text| {
            serde_json::from_str::<Value>(&text)
                .ok()
                .filter(Value::is_number)
                .unwrap_or(Value::String(text))
        }),
        oid::BOOL => match raw {
            [b] => Some(Value::Bool(*b != 0)),
            _ => None,
        },
        oid::JSON => serde_json::from_slice(raw).ok(),
        // Binary `jsonb` is a version byte, 1, and then the text.
        oid::JSONB => match raw.split_first() {
            Some((1, text)) => serde_json::from_slice(text).ok(),
            _ => None,
        },
        oid::UUID => uuid::Uuid::from_slice(raw)
            .ok()
            .map(|v| Value::String(v.to_string())),
        oid::TIMESTAMPTZ => timestamp(raw).map(|v| Value::String(v.and_utc().to_rfc3339())),
        oid::TIMESTAMP => {
            timestamp(raw).map(|v| Value::String(v.format("%Y-%m-%dT%H:%M:%S%.f").to_string()))
        }
        oid::DATE => be_bytes(raw)
            .map(i32::from_be_bytes)
            .and_then(|days| {
                epoch()
                    .date()
                    .checked_add_signed(chrono::TimeDelta::try_days(days.into())?)
            })
            .map(|v| Value::String(v.to_string())),
        oid::TIME => be_bytes(raw)
            .map(i64::from_be_bytes)
            // Wrapping past midnight, as sqlx's `NaiveTime + Duration` does:
            // `24:00:00` reads as `00:00:00`.
            .map(|us| {
                chrono::NaiveTime::default()
                    .overflowing_add_signed(chrono::TimeDelta::microseconds(us))
                    .0
            })
            .map(|v| Value::String(v.to_string())),
        // The types sqlx reads as a string. Anything else -- an enum, an
        // array, an interval -- it refuses, and the column is null.
        oid::TEXT | oid::NAME | oid::BPCHAR | oid::VARCHAR | oid::UNKNOWN => {
            std::str::from_utf8(raw)
                .ok()
                .map(|s| Value::String(s.to_string()))
        }
        _ if ty.name() == "citext" => std::str::from_utf8(raw)
            .ok()
            .map(|s| Value::String(s.to_string())),
        _ => None,
    };
    value.unwrap_or(Value::Null)
}

/// The built-in types `row_json` decodes, by OID. Fixed by PostgreSQL
/// (`pg_type.dat`), so they are spelled out rather than looked up.
mod oid {
    pub const BOOL: u32 = 16;
    pub const NAME: u32 = 19;
    pub const INT8: u32 = 20;
    pub const INT2: u32 = 21;
    pub const INT4: u32 = 23;
    pub const TEXT: u32 = 25;
    pub const JSON: u32 = 114;
    pub const FLOAT4: u32 = 700;
    pub const FLOAT8: u32 = 701;
    pub const UNKNOWN: u32 = 705;
    pub const BPCHAR: u32 = 1042;
    pub const VARCHAR: u32 = 1043;
    pub const DATE: u32 = 1082;
    pub const TIME: u32 = 1083;
    pub const TIMESTAMP: u32 = 1114;
    pub const TIMESTAMPTZ: u32 = 1184;
    pub const NUMERIC: u32 = 1700;
    pub const UUID: u32 = 2950;
    pub const JSONB: u32 = 3802;
}

/// PostgreSQL's epoch, which its binary dates and times count from.
fn epoch() -> chrono::NaiveDateTime {
    chrono::NaiveDate::from_ymd_opt(2000, 1, 1)
        .and_then(|d| d.and_hms_opt(0, 0, 0))
        .unwrap_or_default()
}

/// A binary `timestamp`/`timestamptz`: microseconds since the epoch.
fn timestamp(raw: &[u8]) -> Option<chrono::NaiveDateTime> {
    let us = i64::from_be_bytes(be_bytes(raw)?);
    epoch().checked_add_signed(chrono::TimeDelta::microseconds(us))
}

/// A binary `numeric`, as the text sqlx's `BigDecimal` would print.
///
/// The conversion is sqlx's own (`TryFrom<&PgNumeric> for BigDecimal`): the
/// digits in base 10000, a scale of four decimal places for every digit after
/// the point, and no regard for the value's declared scale. That is why `0.1`
/// prints as `0.1000`, and the point of doing it by hand is that it must.
fn numeric(raw: &[u8]) -> Option<String> {
    use bigdecimal::num_bigint::{BigInt, Sign};
    use bigdecimal::BigDecimal;

    let word =
        |i: usize| -> Option<u16> { Some(u16::from_be_bytes(be_bytes(raw.get(i..i + 2)?)?)) };
    let ndigits = word(0)? as usize;
    let weight = word(2)? as i16;
    let sign = word(4)?;
    // NaN and the infinities, which BigDecimal cannot hold.
    if sign != 0x0000 && sign != 0x4000 {
        return None;
    }
    let digits: Vec<u16> = (0..ndigits)
        .map(|i| word(8 + i * 2))
        .collect::<Option<_>>()?;
    if digits.is_empty() {
        return Some(BigDecimal::from(0u64).to_string());
    }
    let mut cents = Vec::with_capacity(digits.len() * 2);
    for digit in &digits {
        if *digit >= 10000 {
            return None;
        }
        cents.push((digit / 100) as u8);
        cents.push((digit % 100) as u8);
    }
    let sign = if sign == 0x4000 {
        Sign::Minus
    } else {
        Sign::Plus
    };
    let scale = (digits.len() as i64 - weight as i64 - 1) * 4;
    let bigint = BigInt::from_radix_be(sign, &cents, 100)?;
    Some(BigDecimal::new(bigint, scale).to_string())
}

impl From<tokio_postgres::Error> for DbError {
    fn from(e: tokio_postgres::Error) -> Self {
        // `tokio-postgres` says only "db error" or "error communicating with
        // the server"; what went wrong is in the error it wraps. Phrased as
        // sqlx phrases it, so a log reads the same whichever driver wrote it.
        let message = match e.as_db_error() {
            Some(db) => format!("error returned from database: {}", db.message()),
            None => {
                let mut message = e.to_string();
                let mut source = std::error::Error::source(&e);
                while let Some(cause) = source {
                    message.push_str(": ");
                    message.push_str(&cause.to_string());
                    source = cause.source();
                }
                message
            }
        };
        match e.as_db_error() {
            Some(db) => DbError::database(
                message,
                DatabaseError {
                    code: db.code().code().to_string(),
                    message: db.message().to_string(),
                    details: db.detail().map(String::from),
                    hint: db.hint().map(String::from),
                    constraint: db.constraint().map(String::from),
                    table: db.table().map(String::from),
                    // sqlx does not report it, so neither does this.
                    column: None,
                },
            ),
            None => DbError::other(message),
        }
    }
}
