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

    async fn sleep(&self, duration: std::time::Duration) {
        tokio::time::sleep(duration).await
    }
}

fn socket_host() -> String {
    match database_url()
        .parse::<tokio_postgres::Config>()
        .unwrap()
        .get_hosts()
        .first()
    {
        Some(tokio_postgres::config::Host::Tcp(host)) => host.clone(),
        _ => panic!("DATABASE_URL needs a TCP host"),
    }
}

fn socket_port() -> u16 {
    database_url()
        .parse::<tokio_postgres::Config>()
        .unwrap()
        .get_ports()
        .first()
        .copied()
        .unwrap_or(5432)
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
async fn a_shared_column_name_reads_the_last_column() {
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
    // Two count columns: the last is the count, as for every other read.
    let answer = agree(
        "",
        "SELECT 'x' AS body, 1::int8 AS count, 2::int8 AS count",
        &[],
    )
    .await;
    match answer {
        Answer::Rows { raw, .. } => assert_eq!(raw.count, Some(2)),
        failed => panic!("expected rows, got {failed:?}"),
    }
}

#[tokio::test]
#[ignore = "needs a PostgreSQL database"]
async fn a_failure_is_its_own_whatever_types_the_result_has() {
    // A statement that fails after PostgreSQL has described its result, with
    // a column of a type that is not built in. `tokio-postgres` looks such a
    // type up when the description arrives -- and if the statement has
    // already failed, that lookup is refused (`25P02`) and its error was the
    // one reported.
    let setup = format!(
        "{ZOO};
CREATE TEMP TABLE agree_typed (id int PRIMARY KEY, mood pg_temp.mood, label pg_temp.label);
INSERT INTO agree_typed VALUES (1, 'sad', 'a');
CREATE FUNCTION pg_temp.agree_raise() RETURNS pg_temp.mood LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'raised' USING ERRCODE = 'P0001'; END $$"
    );
    let cases = [
        ("INSERT INTO agree_typed VALUES (1, 'happy', 'b') RETURNING id, mood", "23505"),
        ("INSERT INTO agree_typed VALUES (2, 'happy', 'b') RETURNING id, label, 1 / (id - 2) AS z", "22012"),
        ("SELECT mood, 1 / (id - 1) AS z FROM agree_typed", "22012"),
        ("SELECT pg_temp.agree_raise() AS mood", "P0001"),
        ("SELECT ROW(id, mood) AS r, 1 / (id - 1) AS z FROM agree_typed", "22012"),
    ];
    for (sql, code) in cases {
        let answer = agree(&setup, sql, &[]).await;
        assert!(
            matches!(&answer, Answer::Failed { code: Some(c), .. } if c == code),
            "{sql}: {answer:?}"
        );
    }
    // And the same columns read alike where nothing fails.
    let answer = agree(&setup, "SELECT id, mood, label FROM agree_typed", &[]).await;
    assert_eq!(rows(&answer)[0]["label"], "a");
}

#[tokio::test]
#[ignore = "needs a PostgreSQL database"]
async fn a_refused_connection_says_why_as_sqlx_does() {
    let mut config: tokio_postgres::Config = database_url().parse().unwrap();
    config.password("not the password");
    let wrong = PgSocketDb::new(
        Tcp {
            host: socket_host(),
            port: socket_port(),
        },
        config,
    );
    let tokio = match wrong.begin().await {
        Err(e) => e.to_string(),
        Ok(_) => panic!("a wrong password was accepted"),
    };
    let options: sqlx::postgres::PgConnectOptions = database_url().parse().unwrap();
    let pool = sqlx::postgres::PgPoolOptions::new()
        .acquire_timeout(std::time::Duration::from_secs(5))
        .connect_lazy_with(options.password("not the password"));
    let sqlx = match Db::begin(&pool).await {
        Err(e) => e.to_string(),
        Ok(_) => panic!("a wrong password was accepted"),
    };
    assert!(tokio.contains("password authentication failed"), "{tokio}");
    assert_eq!(tokio, sqlx);
}

#[tokio::test]
#[ignore = "needs a PostgreSQL database"]
async fn a_connection_ended_by_the_server_says_why() {
    let (pool, socket) = drivers().await;
    let mut tx = socket.begin().await.unwrap();
    let pid = tx
        .fetch_values("SELECT pg_backend_pid()", &[])
        .await
        .unwrap();
    let pid = pid[0][0].as_i64().unwrap();
    sqlx::query("SELECT pg_terminate_backend($1::int)")
        .bind(pid)
        .execute(&pool)
        .await
        .unwrap();
    // The FATAL arrives while the connection is idle, and ends it.
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    let e = tx.fetch_values("SELECT 1", &[]).await.unwrap_err();
    assert_eq!(
        e.as_database().map(|d| d.code.as_str()),
        Some("57P01"),
        "{e}"
    );
}

#[tokio::test]
#[ignore = "needs a PostgreSQL database"]
async fn a_server_that_never_answers_times_out() {
    // Accepts the connection and then says nothing at all.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let silent = tokio::spawn(async move {
        let mut held = Vec::new();
        while let Ok((stream, _)) = listener.accept().await {
            held.push(stream);
        }
    });
    let mut config: tokio_postgres::Config = database_url().parse().unwrap();
    config.connect_timeout(std::time::Duration::from_secs(1));
    let db = PgSocketDb::new(
        Tcp {
            host: "127.0.0.1".into(),
            port,
        },
        config,
    );
    let started = std::time::Instant::now();
    let e = match db.begin().await {
        Err(e) => e,
        Ok(_) => panic!("a silent server opened a transaction"),
    };
    assert!(started.elapsed() < std::time::Duration::from_secs(5));
    assert!(e.to_string().contains("timed out"), "{e}");
    silent.abort();
}

#[tokio::test]
#[ignore = "needs a PostgreSQL database"]
async fn a_transaction_that_is_not_committed_is_rolled_back() {
    let (pool, socket) = drivers().await;
    // A table of each driver's own, committed, with one row in it.
    async fn check<D: Db>(db: &D, table: &str) {
        let mut tx = db.begin().await.unwrap();
        tx.execute(&format!("DROP TABLE IF EXISTS {table}"), &[])
            .await
            .unwrap();
        tx.execute(&format!("CREATE TABLE {table} (id int PRIMARY KEY)"), &[])
            .await
            .unwrap();
        tx.execute(&format!("INSERT INTO {table} VALUES (1)"), &[])
            .await
            .unwrap();
        tx.commit().await.unwrap();

        // A write and a row lock, then dropped without a commit.
        let mut tx = db.begin().await.unwrap();
        tx.execute(&format!("INSERT INTO {table} VALUES (2)"), &[])
            .await
            .unwrap();
        tx.fetch_values(
            &format!("SELECT id FROM {table} WHERE id = 1 FOR UPDATE"),
            &[],
        )
        .await
        .unwrap();
        drop(tx);

        // Neither outlives it. The lock is waited for a while rather than
        // refused at once: the rollback happens on the server, a moment
        // after the drop.
        let mut tx = db.begin().await.unwrap();
        tx.execute("SET LOCAL lock_timeout = '5s'", &[])
            .await
            .unwrap();
        let locked = tx
            .fetch_values(
                &format!("SELECT id FROM {table} WHERE id = 1 FOR UPDATE"),
                &[],
            )
            .await
            .unwrap_or_else(|e| panic!("{table}: the dropped transaction kept its lock: {e}"));
        assert_eq!(locked, vec![vec![json!(1)]]);
        let written = tx
            .fetch_values(&format!("SELECT count(*) FROM {table} WHERE id = 2"), &[])
            .await
            .unwrap();
        assert_eq!(written, vec![vec![json!(0)]], "{table}");
        tx.execute(&format!("DROP TABLE {table}"), &[])
            .await
            .unwrap();
        tx.commit().await.unwrap();
    }
    check(&pool, "agree_rollback_sqlx").await;
    check(&socket, "agree_rollback_socket").await;
}
