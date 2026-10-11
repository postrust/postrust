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

/// Statements sent together are still run as they would be one at a time:
/// in order, each seeing what the last did, and the first failure is the one
/// reported -- by either driver, whether it can pipeline or not.
#[tokio::test]
#[ignore = "needs a PostgreSQL database"]
async fn queued_statements_run_first_in_order() {
    let (pool, socket) = drivers().await;
    async fn check<D: Db>(db: &D) -> Vec<Answer> {
        let mut answers = Vec::new();
        let settings = [SqlParam::Text("agree.a".into()), SqlParam::Text("1".into())];

        // In order, and before the statement that follows.
        let mut tx = db.begin().await.unwrap();
        tx.queue("SELECT set_config($1, $2, true)", &settings);
        tx.queue(
            "SELECT set_config('agree.a', current_setting('agree.a') || '2', true)",
            &[],
        );
        let rows = tx
            .fetch_json("SELECT current_setting('agree.a') AS a", &[])
            .await;
        answers.push(rows.map_or_else(failed, |json| Answer::Rows {
            json,
            values: vec![],
            raw: RawRows::default(),
        }));

        // A queued failure is reported in place of the statement after it.
        let mut tx = db.begin().await.unwrap();
        tx.queue("SELECT set_config($1, $2, true)", &settings);
        tx.queue("SELECT 1 / 0", &[]);
        tx.queue("SELECT 'x'::int", &[]);
        answers.push(match tx.fetch_json("SELECT 1 AS v", &[]).await {
            Ok(rows) => panic!("expected a failure, got {rows:?}"),
            Err(e) => failed(e),
        });

        // The statement after: run after the first, and not at all when the
        // first failed.
        let mut tx = db.begin().await.unwrap();
        let (rows, then) = tx
            .fetch_json_then(
                "SELECT set_config('response.status', '201', true) AS s",
                &[],
                "SELECT current_setting('response.status', true)",
            )
            .await
            .unwrap();
        answers.push(Answer::Rows {
            json: rows,
            values: then,
            raw: RawRows::default(),
        });
        let mut tx = db.begin().await.unwrap();
        answers.push(
            match tx.fetch_json_then("SELECT nope", &[], "SELECT 1 / 0").await {
                Ok(rows) => panic!("expected a failure, got {rows:?}"),
                Err(e) => failed(e),
            },
        );
        answers
    }
    let sqlx = check(&pool).await;
    assert_eq!(sqlx, check(&socket).await);
    assert_eq!(rows(&sqlx[0]), [json!({"a": "12"})]);
    assert!(matches!(&sqlx[1], Answer::Failed { code: Some(c), .. } if c == "22012"));
    assert!(matches!(&sqlx[2], Answer::Rows { values, .. } if values == &[vec![json!("201")]]));
    assert!(matches!(&sqlx[3], Answer::Failed { code: Some(c), .. } if c == "42703"));
}

/// A query sent with its `COMMIT` is committed when it succeeds, and rolled
/// back, with its own error, when it fails.
#[tokio::test]
#[ignore = "needs a PostgreSQL database"]
async fn a_query_committed_with_its_commit() {
    let (pool, socket) = drivers().await;
    sqlx::query("DROP TABLE IF EXISTS agree_commit")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("CREATE TABLE agree_commit (n int PRIMARY KEY)")
        .execute(&pool)
        .await
        .unwrap();
    async fn check<D: Db>(db: &D, n: i64) -> (Answer, Answer) {
        let insert = "INSERT INTO agree_commit VALUES ($1) RETURNING n";
        let tx = db.begin().await.unwrap();
        let ok = tx
            .fetch_json_and_commit(insert, &[SqlParam::Int(n)])
            .await
            .map_or_else(failed, |json| Answer::Rows {
                json,
                values: vec![],
                raw: RawRows::default(),
            });
        // A duplicate key: nothing new is committed.
        let mut tx = db.begin().await.unwrap();
        tx.queue(insert, &[SqlParam::Int(n + 100)]);
        let duplicate = tx
            .fetch_json_and_commit(insert, &[SqlParam::Int(n)])
            .await
            .map_or_else(failed, |rows| panic!("expected a failure, got {rows:?}"));
        (ok, duplicate)
    }
    let (sqlx_ok, sqlx_failed) = check(&pool, 1).await;
    let (socket_ok, socket_failed) = check(&socket, 2).await;
    assert_eq!(rows(&sqlx_ok), [json!({"n": 1})]);
    assert_eq!(rows(&socket_ok), [json!({"n": 2})]);
    // Each driver inserted its own key, so the details name different rows.
    for failed in [&sqlx_failed, &socket_failed] {
        assert!(matches!(failed, Answer::Failed { code: Some(c), .. } if c == "23505"));
    }
    let committed: Vec<i32> = sqlx::query_scalar("SELECT n FROM agree_commit ORDER BY n")
        .fetch_all(&pool)
        .await
        .unwrap();
    assert_eq!(committed, [1, 2]);
    sqlx::query("DROP TABLE agree_commit")
        .execute(&pool)
        .await
        .unwrap();
}

/// The point of pipelining: how many times the socket driver waits on the
/// database. A flight is a burst of writes followed by a wait for the answer,
/// and on a real network each one is a round trip.
#[tokio::test]
#[ignore = "needs a PostgreSQL database"]
async fn a_read_is_one_flight_and_a_write_two() {
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::task::{Context, Poll};
    use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

    #[derive(Default)]
    struct Flights {
        count: AtomicUsize,
        writing: AtomicBool,
    }

    /// A socket that counts its flights: a write after a read starts one.
    struct Counted(tokio::net::TcpStream, Arc<Flights>);

    impl AsyncRead for Counted {
        fn poll_read(
            mut self: std::pin::Pin<&mut Self>,
            cx: &mut Context<'_>,
            buf: &mut ReadBuf<'_>,
        ) -> Poll<std::io::Result<()>> {
            let before = buf.filled().len();
            let poll = std::pin::Pin::new(&mut self.0).poll_read(cx, buf);
            if buf.filled().len() > before {
                self.1.writing.store(false, Ordering::SeqCst);
            }
            poll
        }
    }

    impl AsyncWrite for Counted {
        fn poll_write(
            mut self: std::pin::Pin<&mut Self>,
            cx: &mut Context<'_>,
            buf: &[u8],
        ) -> Poll<std::io::Result<usize>> {
            if !self.1.writing.swap(true, Ordering::SeqCst) {
                self.1.count.fetch_add(1, Ordering::SeqCst);
            }
            std::pin::Pin::new(&mut self.0).poll_write(cx, buf)
        }

        fn poll_flush(
            mut self: std::pin::Pin<&mut Self>,
            cx: &mut Context<'_>,
        ) -> Poll<std::io::Result<()>> {
            std::pin::Pin::new(&mut self.0).poll_flush(cx)
        }

        fn poll_shutdown(
            mut self: std::pin::Pin<&mut Self>,
            cx: &mut Context<'_>,
        ) -> Poll<std::io::Result<()>> {
            std::pin::Pin::new(&mut self.0).poll_shutdown(cx)
        }
    }

    struct CountingTcp(Tcp, Arc<Flights>);

    impl Connector for CountingTcp {
        type Stream = Counted;
        type Tls = tokio_postgres::NoTls;

        async fn connect(&self) -> std::io::Result<Counted> {
            Ok(Counted(self.0.connect().await?, self.1.clone()))
        }

        fn tls(&self) -> tokio_postgres::NoTls {
            tokio_postgres::NoTls
        }

        fn spawn(&self, connection: BoxFuture<'static, ()>) {
            tokio::spawn(connection);
        }
    }

    let (pool, _) = drivers().await;
    sqlx::query("CREATE TABLE IF NOT EXISTS agree_flights (n int)")
        .execute(&pool)
        .await
        .unwrap();

    let config: tokio_postgres::Config = database_url().parse().unwrap();
    let (host, port) = match (config.get_hosts().first(), config.get_ports().first()) {
        (Some(tokio_postgres::config::Host::Tcp(host)), Some(port)) => (host.clone(), *port),
        _ => panic!("DATABASE_URL needs a TCP host and port"),
    };
    let flights = Arc::new(Flights::default());
    let db = PgSocketDb::new(CountingTcp(Tcp { host, port }, flights.clone()), config);
    // Flights from opening a transaction until it is answered.
    let after_connecting = || {
        let flights = flights.clone();
        let start = flights.count.load(Ordering::SeqCst);
        move || flights.count.load(Ordering::SeqCst) - start
    };
    let settings = [SqlParam::Text("agree.a".into()), SqlParam::Text("1".into())];

    // A read, as the engine sends one: the transaction set up, the query, and
    // the commit.
    let mut tx = db.begin().await.unwrap();
    let read = after_connecting();
    tx.queue("SET TRANSACTION READ ONLY", &[]);
    tx.queue("SELECT set_config($1, $2, true)", &settings);
    tx.fetch_json_and_commit("SELECT n FROM agree_flights", &[])
        .await
        .unwrap();
    assert_eq!(read(), 1, "a read");

    // A write: set up, the statement and the function's settings read back,
    // and then the commit, which waits on the response being sendable.
    let mut tx = db.begin().await.unwrap();
    let write = after_connecting();
    tx.queue("SELECT set_config($1, $2, true)", &settings);
    tx.fetch_json_then(
        "INSERT INTO agree_flights VALUES (1) RETURNING n",
        &[],
        "SELECT current_setting('response.status', true)",
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();
    assert_eq!(write(), 2, "a write");

    sqlx::query("DROP TABLE agree_flights")
        .execute(&pool)
        .await
        .unwrap();
}
