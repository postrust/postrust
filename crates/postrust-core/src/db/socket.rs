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

use super::{BoxFuture, Db, DbError, MaybeSend, MaybeSync, RawRows, Tx};
use crate::error::DatabaseError;
use postrust_sql::SqlParam;
use std::future::Future;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio_postgres::tls::TlsConnect;
use tokio_postgres::types::{FromSql, ToSql, Type};
use tokio_postgres::{Client, Row};

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
}

/// A database reached over a socket per transaction. See the module docs.
pub struct PgSocketDb<C> {
    connector: C,
    config: tokio_postgres::Config,
}

impl<C: Connector> PgSocketDb<C> {
    /// A database at `config`, reached through `connector`. The host, port
    /// and TLS settings in `config` are not used to connect -- the connector
    /// decides those -- only the user, password, database and options.
    pub fn new(connector: C, config: tokio_postgres::Config) -> Self {
        Self { connector, config }
    }
}

/// A transaction on its own connection.
pub struct SocketTx {
    client: Client,
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
        let stream = self
            .connector
            .connect()
            .await
            .map_err(|e| DbError::other(e.to_string()))?;
        let (client, connection) = self
            .config
            .connect_raw(stream, self.connector.tls())
            .await?;
        self.connector.spawn(Box::pin(async move {
            // An error here is the connection dying, which the client sees as
            // the failure of whatever it was waiting for.
            let _ = connection.await;
        }));
        client.batch_execute("BEGIN").await?;
        Ok(SocketTx { client })
    }
}

impl Tx for SocketTx {
    async fn execute(&mut self, sql: &str, params: &[SqlParam]) -> Result<u64, DbError> {
        let bound = Bound::new(params);
        let stream = self.client.query_typed_raw(sql, bound.iter()).await?;
        futures::pin_mut!(stream);
        while futures::StreamExt::next(&mut stream)
            .await
            .transpose()?
            .is_some()
        {}
        Ok(stream.rows_affected().unwrap_or_default())
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
        let count = rows.first().and_then(|row| {
            let index = row
                .columns()
                .iter()
                .position(|c| c.name() == count_column)?;
            match row.columns()[index].type_().oid() {
                oid::INT8 => be_bytes(raw(row, index)?).map(i64::from_be_bytes),
                _ => None,
            }
        });
        Ok(RawRows::new(rows.len(), first, count))
    }

    async fn commit(self) -> Result<(), DbError> {
        Ok(self.client.batch_execute("COMMIT").await?)
    }
}

impl SocketTx {
    async fn query(&self, sql: &str, params: &[SqlParam]) -> Result<Vec<Row>, DbError> {
        let bound = Bound::new(params);
        let stream = self.client.query_typed_raw(sql, bound.iter()).await?;
        Ok(futures::TryStreamExt::try_collect(stream).await?)
    }
}

/// Parameters as sqlx binds them: each with the type of its Rust value.
struct Bound(Vec<(Box<dyn ToSql + Sync + Send>, Type)>);

impl Bound {
    fn new(params: &[SqlParam]) -> Self {
        Self(params.iter().map(bind).collect())
    }

    fn iter(&self) -> impl Iterator<Item = (&(dyn ToSql + Sync), Type)> {
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
        let message = e.to_string();
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
