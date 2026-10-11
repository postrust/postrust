//! The two database drivers answer alike.
//!
//! The REST pipeline runs on sqlx in the server and on `tokio-postgres` over a
//! socket in a Cloudflare Worker (`postrust_core::db`, issue #50). Whatever
//! one decodes, binds or reports, the other has to as well, or a client sees
//! a different API depending on where it is deployed. Each statement here
//! runs through both and the answers are compared whole.
//!
//! The database must be reachable at `DATABASE_URL`; nothing in it is used
//! except what these tests create in their own transactions, which are rolled
//! back. Run with:
//!
//! ```text
//! cargo test --package postrust-server --test drivers_agree -- --ignored
//! ```

use postrust_core::db::{BoxFuture, Connector, Db, DbError, PgSocketDb, RawRows, Tx};
use postrust_sql::SqlParam;
use serde_json::{json, Value};

fn database_url() -> String {
    std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://postgres:postgres@localhost:5432/postrust_test".to_string())
}

/// A plain TCP socket, as the server would open one, standing in for the
/// Worker's socket to Hyperdrive.
struct Tcp {
    host: String,
    port: u16,
}

impl Connector for Tcp {
    type Stream = tokio::net::TcpStream;
    type Tls = tokio_postgres::NoTls;

    async fn connect(&self) -> std::io::Result<tokio::net::TcpStream> {
        tokio::net::TcpStream::connect((self.host.as_str(), self.port)).await
    }

    fn tls(&self) -> tokio_postgres::NoTls {
        tokio_postgres::NoTls
    }

    fn spawn(&self, connection: BoxFuture<'static, ()>) {
        tokio::spawn(connection);
    }
}

async fn drivers() -> (sqlx::PgPool, PgSocketDb<Tcp>) {
    let url = database_url();
    let pool = sqlx::PgPool::connect(&url).await.expect("connect (sqlx)");
    let config: tokio_postgres::Config = url.parse().expect("a postgres:// URL");
    let (host, port) = match (config.get_hosts().first(), config.get_ports().first()) {
        (Some(tokio_postgres::config::Host::Tcp(host)), Some(port)) => (host.clone(), *port),
        _ => panic!("DATABASE_URL needs a TCP host and port"),
    };
    (pool, PgSocketDb::new(Tcp { host, port }, config))
}

/// What a statement produced, through every way the pipeline reads one.
#[derive(Debug, PartialEq)]
enum Answer {
    Rows {
        json: Vec<Value>,
        values: Vec<Vec<Value>>,
        raw: RawRows,
    },
    Failed {
        code: Option<String>,
        message: Option<String>,
        details: Option<String>,
        hint: Option<String>,
        constraint: Option<String>,
        table: Option<String>,
    },
}

fn failed(e: DbError) -> Answer {
    let db = e.as_database();
    Answer::Failed {
        code: db.map(|d| d.code.clone()),
        message: db.map(|d| d.message.clone()),
        details: db.and_then(|d| d.details.clone()),
        hint: db.and_then(|d| d.hint.clone()),
        constraint: db.and_then(|d| d.constraint.clone()),
        table: db.and_then(|d| d.table.clone()),
    }
}

/// Run `setup`, then `sql` three ways -- as JSON objects, as positional
/// values, and raw -- on one transaction that is rolled back.
///
/// One transaction rather than three: setup that creates types (`ZOO`) would
/// otherwise make them anew each time, and sqlx's statement cache, keyed by
/// the SQL text on a pooled connection, would then hold a plan for types that
/// no longer exist.
async fn answer<D: Db>(db: &D, setup: &str, sql: &str, params: &[SqlParam]) -> Answer {
    async fn read<D: Db>(
        db: &D,
        setup: &str,
        sql: &str,
        params: &[SqlParam],
    ) -> Result<(Vec<Value>, Vec<Vec<Value>>, RawRows), DbError> {
        let mut tx = db.begin().await?;
        for statement in setup.split(";\n").filter(|s| !s.trim().is_empty()) {
            tx.execute(statement, &[]).await?;
        }
        let json = tx.fetch_json(sql, params).await?;
        let values = tx.fetch_values(sql, params).await?;
        let raw = tx.fetch_raw(sql, params, "count").await?;
        Ok((json, values, raw))
    }

    match read(db, setup, sql, params).await {
        Ok((json, values, raw)) => Answer::Rows { json, values, raw },
        Err(e) => failed(e),
    }
}

async fn agree(setup: &str, sql: &str, params: &[SqlParam]) -> Answer {
    let (pool, socket) = drivers().await;
    let sqlx = answer(&pool, setup, sql, params).await;
    let tokio = answer(&socket, setup, sql, params).await;
    assert_eq!(sqlx, tokio, "the drivers disagree on {sql}");
    sqlx
}

fn rows(answer: &Answer) -> &[Value] {
    match answer {
        Answer::Rows { json, .. } => json,
        failed => panic!("expected rows, got {failed:?}"),
    }
}

const ZOO: &str = "
CREATE TYPE pg_temp.mood AS ENUM ('sad', 'happy');
CREATE DOMAIN pg_temp.label AS text;
CREATE DOMAIN pg_temp.amount AS numeric(10, 2)";

#[tokio::test]
#[ignore = "needs a PostgreSQL database"]
async fn every_type_decodes_alike() {
    let answer = agree(
        ZOO,
        "SELECT 1::int2 AS i2, -2::int4 AS i4, 9007199254740993::int8 AS i8,
                0.1::float4 AS f4, 0.1::float8 AS f8, 'NaN'::float8 AS f8_nan,
                0.1::numeric AS n1, 0.1000::numeric AS n4, 12345678901234567890.123456789::numeric AS nbig,
                0::numeric AS nzero, -1.5::numeric AS nneg, 1e-20::numeric AS ntiny, 1e30::numeric AS nhuge,
                'NaN'::numeric AS nnan, 'Infinity'::numeric AS ninf,
                true AS yes, false AS no,
                '{\"b\": 1, \"a\": [1.10, \"x\"]}'::json AS j, '{\"b\": 1, \"a\": [1.10]}'::jsonb AS jb,
                'a0eebc99-9c0b-4ef8-bb6d-6bb9bd380a11'::uuid AS u,
                '2015-12-08 04:22:57.472738+00'::timestamptz AS tz,
                '2015-12-08 04:22:57'::timestamp AS ts, '1999-12-31 23:59:59.5'::timestamp AS ts_frac,
                '2020-02-29'::date AS d, '0044-03-15 BC'::date AS d_bc,
                '13:45:01.25'::time AS t, '24:00:00'::time AS t_midnight,
                'text'::text AS tx, 'name'::name AS nm, 'ch'::char(4) AS bp, 'vc'::varchar(9) AS vc,
                'happy'::pg_temp.mood AS enum_value, 'lbl'::pg_temp.label AS domain_text,
                12.5::pg_temp.amount AS domain_numeric,
                ARRAY[1, 2] AS int_array, ARRAY['a'] AS text_array, '1 day'::interval AS iv,
                'x'::\"char\" AS one_char, '<a/>'::xml AS x, '\\xdeadbeef'::bytea AS bin,
                '192.168.0.1'::inet AS addr, 42::oid AS an_oid, NULL::int AS nothing,
                ROW(1, 'a') AS composite, 7 AS count",
        &[],
    )
    .await;
    // Spot checks on the rules themselves, so agreeing on a wrong answer is
    // not enough.
    let row = &rows(&answer)[0];
    assert_eq!(row["n1"].to_string(), "0.1000");
    assert_eq!(row["f4"].to_string(), "0.10000000149011612");
    assert_eq!(row["i8"].to_string(), "9007199254740993");
    assert_eq!(row["tz"], "2015-12-08T04:22:57.472738+00:00");
    assert_eq!(row["ts"], "2015-12-08T04:22:57");
    assert_eq!(row["t_midnight"], "00:00:00");
    assert_eq!(row["nnan"], Value::Null);
    assert_eq!(row["enum_value"], Value::Null);
    assert_eq!(row["domain_text"], "lbl");
    assert_eq!(row["nothing"], Value::Null);
}

#[tokio::test]
#[ignore = "needs a PostgreSQL database"]
async fn a_shared_column_name_reads_the_first_column() {
    agree("", "SELECT 1 AS a, 2 AS a, 'x' AS b", &[]).await;
}

#[tokio::test]
#[ignore = "needs a PostgreSQL database"]
async fn many_rows_keep_their_order() {
    agree(
        "",
        "SELECT g AS n, md5(g::text) AS h FROM generate_series(1, 500) g ORDER BY g DESC",
        &[],
    )
    .await;
}

#[tokio::test]
#[ignore = "needs a PostgreSQL database"]
async fn parameters_are_typed_alike() {
    // Each of these is a statement where the type a parameter is sent with
    // decides whether PostgreSQL accepts it -- an `int8` against an `int4`
    // column, a `jsonb` where `json` is wanted, a `text[]` for `ANY`.
    let cases: Vec<(&str, Vec<SqlParam>)> = vec![
        ("SELECT $1 AS v", vec![SqlParam::Int(5)]),
        ("SELECT $1::int4 + 1 AS v", vec![SqlParam::Int(5)]),
        ("SELECT 2::int4 = $1 AS v", vec![SqlParam::Int(2)]),
        ("SELECT $1 AS v", vec![SqlParam::Float(1.25)]),
        ("SELECT $1 AS v", vec![SqlParam::Bool(true)]),
        (
            "SELECT $1 AS v, $2 AS w",
            vec![SqlParam::Text("é".into()), SqlParam::Null],
        ),
        ("SELECT $1::int AS v", vec![SqlParam::Text("12".into())]),
        (
            "SELECT $1 AS v",
            vec![SqlParam::Json(json!({"a": [1.50, null]}))],
        ),
        (
            "SELECT $1::json AS v",
            vec![SqlParam::Json(json!({"a": 1}))],
        ),
        ("SELECT $1 AS v", vec![SqlParam::Bytes(vec![0, 255])]),
        (
            "SELECT $1 AS v",
            vec![SqlParam::Uuid(uuid::Uuid::from_u128(0xa0ee_bc99))],
        ),
        (
            "SELECT $1 AS v",
            vec![SqlParam::Timestamp(
                chrono::DateTime::from_timestamp(1_449_548_577, 5_000).unwrap(),
            )],
        ),
        (
            "SELECT 'b' = ANY($1) AS v, $1 AS w",
            vec![SqlParam::Array(vec![
                SqlParam::Text("a".into()),
                SqlParam::Text("b".into()),
            ])],
        ),
        (
            "SELECT 2 = ANY($1::int[]) AS v",
            vec![SqlParam::Array(vec![SqlParam::Int(1), SqlParam::Int(2)])],
        ),
        (
            "SELECT set_config($1, $2, true) AS v, current_setting('request.x') AS w",
            vec![
                SqlParam::Text("request.x".into()),
                SqlParam::Text("1".into()),
            ],
        ),
    ];
    for (sql, params) in cases {
        let answer = agree("", sql, &params).await;
        rows(&answer);
    }
}

#[tokio::test]
#[ignore = "needs a PostgreSQL database"]
async fn failures_are_reported_alike() {
    let table = "
CREATE TEMP TABLE agree_t (id int PRIMARY KEY CONSTRAINT agree_positive CHECK (id > 0), name text NOT NULL);
INSERT INTO agree_t VALUES (1, 'a')";
    let cases = [
        "SELECT 1 / 0 AS v",
        "SELECT nope FROM agree_t",
        "INSERT INTO agree_t VALUES (1, 'b') RETURNING id",
        "INSERT INTO agree_t VALUES (-1, 'b') RETURNING id",
        "INSERT INTO agree_t VALUES (2, NULL) RETURNING id",
        "SELECT 'x'::int AS v",
        "DO $$ BEGIN RAISE EXCEPTION 'custom' USING DETAIL = 'the detail', HINT = 'the hint', ERRCODE = 'P0001'; END $$",
        "SELECT * FROM",
    ];
    for sql in cases {
        let answer = agree(table, sql, &[]).await;
        assert!(
            matches!(answer, Answer::Failed { code: Some(_), .. }),
            "{sql}: {answer:?}"
        );
    }
}

#[tokio::test]
#[ignore = "needs a PostgreSQL database"]
async fn a_rendered_body_is_read_raw_alike() {
    agree(
        "",
        "SELECT string_agg(g::text, ',')::text AS pgrst_body, count(*) AS count FROM generate_series(1, 3) g",
        &[],
    )
    .await;
    agree(
        "",
        "SELECT '\\xdeadbeef'::bytea AS body, 1::int8 AS count",
        &[],
    )
    .await;
    agree("", "SELECT NULL::text AS body, 0::int8 AS count", &[]).await;
    agree("", "SELECT 'x' AS body WHERE false", &[]).await;
}

#[tokio::test]
#[ignore = "needs a PostgreSQL database"]
async fn a_transaction_that_is_not_committed_is_rolled_back() {
    let (pool, socket) = drivers().await;
    async fn check<D: Db>(db: &D) {
        let mut tx = db.begin().await.unwrap();
        tx.execute("CREATE TABLE agree_rollback (id int)", &[])
            .await
            .unwrap();
        drop(tx);
        let mut tx = db.begin().await.unwrap();
        let exists = tx
            .fetch_values("SELECT to_regclass('agree_rollback') IS NOT NULL", &[])
            .await
            .unwrap();
        assert_eq!(exists, vec![vec![Value::Bool(false)]]);
        tx.commit().await.unwrap();
    }
    check(&pool).await;
    check(&socket).await;
}
