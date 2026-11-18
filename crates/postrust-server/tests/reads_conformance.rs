//! Reads, as PostgREST answers them: a filter on an embed's own embed, data
//! representations inside embeds, `like` operands, array casts, a function's
//! own columns shaped by `select`/filters/`order`, and a JSON path in an
//! embed's `order`.
//!
//! `DATABASE_URL` must name a superuser on a database with
//! `scripts/init-db.sql` loaded (for `web_anon`). Run with:
//!
//! ```text
//! cargo test --package postrust-server --test reads_conformance -- --ignored
//! cargo test --package postrust-server --features pg-socket --test reads_conformance -- --ignored
//! ```

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use postrust_core::AppConfig;
use postrust_server::{Builder, Host};
use serde_json::{json, Value};
use sqlx::postgres::PgPoolOptions;
use sqlx::AssertSqlSafe;
use tower::ServiceExt;

const SCHEMA: &str = "reads_conformance";

fn database_url() -> String {
    std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://postgres:postgres@localhost:5432/postrust_test".to_string())
}

async fn schema() {
    let s = SCHEMA;
    let pool = PgPoolOptions::new()
        .max_connections(1)
        .connect(&database_url())
        .await
        .unwrap();
    let statements = [
        format!("DROP SCHEMA IF EXISTS {s} CASCADE"),
        format!("CREATE SCHEMA {s}"),
        format!("GRANT USAGE ON SCHEMA {s} TO web_anon"),
        // Embeds, three deep.
        format!("CREATE TABLE {s}.entities (id int PRIMARY KEY, name text)"),
        format!(
            "CREATE TABLE {s}.child_entities (id int PRIMARY KEY, name text, \
             parent_id int REFERENCES {s}.entities(id))"
        ),
        format!(
            "CREATE TABLE {s}.grandchild_entities (id int PRIMARY KEY, name text, \
             parent_id int REFERENCES {s}.child_entities(id))"
        ),
        format!("INSERT INTO {s}.entities VALUES (1, 'entity 1'), (2, 'entity 2')"),
        format!(
            "INSERT INTO {s}.child_entities VALUES (1, 'child 1', 1), (2, 'child 2', 1), \
             (3, 'child 3', 2)"
        ),
        // Only child 1 has a grandchild.
        format!("INSERT INTO {s}.grandchild_entities VALUES (1, 'grandchild 1', 1)"),
        // A data representation, as PostgREST's fixtures declare one. It is
        // in `public`: a cast's type is named in the catalog as the search
        // path would spell it, which is how the schema cache finds it.
        "DROP DOMAIN IF EXISTS public.rc_color CASCADE".to_string(),
        "DROP FUNCTION IF EXISTS public.rc_color(text)".to_string(),
        "DROP FUNCTION IF EXISTS public.rc_color(json)".to_string(),
        "DROP FUNCTION IF EXISTS public.rc_color_json(public.rc_color)".to_string(),
        "CREATE DOMAIN public.rc_color AS integer CHECK (VALUE >= 0 AND VALUE <= 16777215)"
            .to_string(),
        "CREATE FUNCTION public.rc_color(text) RETURNS public.rc_color LANGUAGE sql IMMUTABLE AS $$ \
         SELECT (('x' || lpad((CASE WHEN substring($1, 1, 1) = '#' THEN substring($1, 2) \
         ELSE $1 END), 8, '0'))::bit(32)::int)::public.rc_color $$"
            .to_string(),
        "CREATE FUNCTION public.rc_color(json) RETURNS public.rc_color LANGUAGE sql IMMUTABLE AS $$ \
         SELECT public.rc_color($1 #>> '{}') $$"
            .to_string(),
        "CREATE FUNCTION public.rc_color_json(public.rc_color) RETURNS json LANGUAGE sql IMMUTABLE \
         AS $$ SELECT to_json('#' || lpad(upper(to_hex($1)), 6, '0')) $$"
            .to_string(),
        "CREATE CAST (public.rc_color AS json) WITH FUNCTION public.rc_color_json(public.rc_color) \
         AS IMPLICIT"
            .to_string(),
        "CREATE CAST (json AS public.rc_color) WITH FUNCTION public.rc_color(json) AS IMPLICIT"
            .to_string(),
        "CREATE CAST (text AS public.rc_color) WITH FUNCTION public.rc_color(text) AS IMPLICIT"
            .to_string(),
        "GRANT EXECUTE ON FUNCTION public.rc_color(text), public.rc_color(json), \
         public.rc_color_json(public.rc_color) TO PUBLIC"
            .to_string(),
        format!(
            "CREATE TABLE {s}.todos (id int PRIMARY KEY, name text, label_color public.rc_color)"
        ),
        format!(
            "INSERT INTO {s}.todos VALUES (1, 'a', 0), (2, 'b', 255), (3, 'c', 1193046)"
        ),
        format!(
            "CREATE TABLE {s}.lists (id int PRIMARY KEY, name text, \
             first_id int REFERENCES {s}.todos(id))"
        ),
        format!("INSERT INTO {s}.lists VALUES (1, 'one', 1), (2, 'two', 3)"),
        // Arrays and JSON.
        format!("CREATE TABLE {s}.arrays (id int PRIMARY KEY, nums int[], data jsonb)"),
        format!(
            "INSERT INTO {s}.arrays VALUES (1, '{{1,2,3}}', '{{\"k\": 3}}'), \
             (2, '{{4,5}}', '{{\"k\": 1}}'), (3, '{{}}', '{{\"k\": 2}}')"
        ),
        format!(
            "CREATE TABLE {s}.owners (id int PRIMARY KEY, name text)"
        ),
        format!("INSERT INTO {s}.owners VALUES (1, 'o')"),
        format!(
            "CREATE TABLE {s}.pets (id int PRIMARY KEY, owner_id int REFERENCES {s}.owners(id), \
             data jsonb)"
        ),
        format!(
            "INSERT INTO {s}.pets VALUES (1, 1, '{{\"rank\": 3}}'), (2, 1, '{{\"rank\": 1}}'), \
             (3, 1, '{{\"rank\": 2}}')"
        ),
        // Functions with their own columns, and the kinds that have none.
        format!("CREATE TYPE {s}.complex AS (r double precision, i double precision)"),
        format!(
            "CREATE FUNCTION {s}.returns_complex() RETURNS TABLE(id int, val {s}.complex) \
             LANGUAGE sql AS $$ SELECT 1, row(0.1, 0.5)::{s}.complex UNION \
             SELECT 2, row(0.2, 0.6)::{s}.complex UNION SELECT 3, row(0.3, 0.7)::{s}.complex $$"
        ),
        format!(
            "CREATE FUNCTION {s}.out_params(OUT a int, OUT b text) RETURNS record \
             LANGUAGE sql AS $$ SELECT 7, 'seven' $$"
        ),
        format!(
            "CREATE FUNCTION {s}.three_rows() RETURNS TABLE(n int, label text) LANGUAGE sql AS \
             $$ SELECT g, 'n' || g FROM generate_series(1, 3) g $$"
        ),
        format!("CREATE FUNCTION {s}.answer() RETURNS int LANGUAGE sql AS $$ SELECT 42 $$"),
        format!(
            "CREATE FUNCTION {s}.all_todos() RETURNS SETOF {s}.todos LANGUAGE sql AS \
             $$ SELECT * FROM {s}.todos $$"
        ),
        format!("GRANT SELECT ON ALL TABLES IN SCHEMA {s} TO web_anon"),
        format!("GRANT EXECUTE ON ALL FUNCTIONS IN SCHEMA {s} TO web_anon"),
    ];
    for statement in statements {
        sqlx::query(AssertSqlSafe(statement))
            .execute(&pool)
            .await
            .unwrap_or_else(|e| panic!("{e}"));
    }
}

async fn router(compat_mode: bool) -> Router {
    let mut config = AppConfig::default();
    config.db_uri = database_url();
    config.db_schemas = vec![SCHEMA.to_string()];
    config.db_anon_role = Some("web_anon".to_string());
    config.db_pool_size = 2;
    config.compat_mode = compat_mode;
    Builder::new(config, Host::SERVER)
        .build()
        .await
        .unwrap()
        .router
}

/// GET `path`, which is relative to the API root in either mode.
async fn get(router: &Router, compat_mode: bool, path: &str) -> (StatusCode, Value) {
    let prefix = if compat_mode { "" } else { "/api" };
    // The paths are written as a client would type them; the wire wants the
    // characters a URI does not allow escaped.
    let escaped: String = path
        .chars()
        .map(|c| match c {
            ' ' | '{' | '}' | '>' | '<' | '"' | '|' | '\\' | '^' | '`' => {
                format!("%{:02X}", c as u8)
            }
            other => other.to_string(),
        })
        .collect();
    let request = Request::get(format!("{prefix}{escaped}"))
        .body(Body::empty())
        .unwrap();
    let response = router.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let body = serde_json::from_slice(&body).unwrap_or(Value::Null);
    (status, body)
}

#[tokio::test]
#[ignore = "requires DATABASE_URL"]
async fn reads_match_postgrest() {
    schema().await;

    for compat in [true, false] {
        let router = router(compat).await;
        let ask = |path: &'static str| {
            let router = router.clone();
            async move { get(&router, compat, path).await }
        };

        // C2: whether an embed's own embed matched, filtered at depth.
        let (status, body) = ask(
            "/entities?select=name,child_entities(name,grandchild_entities())\
             &child_entities.grandchild_entities=not.is.null&child_entities=not.is.null&order=id",
        )
        .await;
        assert_eq!(status, StatusCode::OK, "compat={compat} {body}");
        assert_eq!(
            body,
            json!([{"name": "entity 1", "child_entities": [{"name": "child 1"}]}]),
            "compat={compat}"
        );
        let (status, body) = ask(
            "/entities?select=name,child_entities(name,grandchild_entities())\
             &child_entities.grandchild_entities=is.null&order=id",
        )
        .await;
        assert_eq!(status, StatusCode::OK, "compat={compat} {body}");
        assert_eq!(
            body,
            json!([
                {"name": "entity 1", "child_entities": [{"name": "child 2"}]},
                {"name": "entity 2", "child_entities": [{"name": "child 3"}]},
            ]),
            "compat={compat}"
        );
        // And as a logic tree at the nested path.
        let (status, body) = ask(
            "/entities?select=name,child_entities(name,grandchild_entities())\
             &child_entities.or=(grandchild_entities.not.is.null,grandchild_entities.is.null)&order=id",
        )
        .await;
        assert_eq!(status, StatusCode::OK, "compat={compat} {body}");
        assert_eq!(
            body[0]["child_entities"],
            json!([{"name": "child 1"}, {"name": "child 2"}]),
            "compat={compat}"
        );
        let (_, body) = ask(
            "/entities?select=name,child_entities(name,grandchild_entities())\
             &child_entities.and=(grandchild_entities.not.is.null)&order=id",
        )
        .await;
        assert_eq!(
            body[0]["child_entities"],
            json!([{"name": "child 1"}]),
            "compat={compat}"
        );

        // C4: a filter in an embed reads its value in the column's spelling,
        // so `#000100` is 256 and no row has it.
        let (status, body) =
            ask("/lists?select=id,todos(label_color)&todos.label_color=neq.%23000000&order=id")
                .await;
        assert_eq!(status, StatusCode::OK, "compat={compat} {body}");
        assert_eq!(
            body,
            json!([
                {"id": 1, "todos": null},
                {"id": 2, "todos": {"label_color": "#123456"}},
            ]),
            "compat={compat}"
        );
        let (_, body) =
            ask("/lists?select=id,todos(label_color)&todos.or=(label_color.eq.%23123456)&order=id")
                .await;
        assert_eq!(body[0]["todos"], Value::Null, "compat={compat} {body}");
        assert_eq!(
            body[1]["todos"],
            json!({"label_color": "#123456"}),
            "compat={compat}"
        );

        // C5: `*` in an embed renders a represented column through its
        // function, as an explicit field does.
        let (_, body) = ask("/lists?select=id,todos(*)&order=id").await;
        assert_eq!(
            body[1]["todos"],
            json!({"id": 3, "name": "c", "label_color": "#123456"}),
            "compat={compat}"
        );

        // A `$1` in a like pattern inside an embed is text, not a placeholder.
        let (status, body) =
            ask("/lists?select=id,todos(name)&todos.name=like.*%241*&todos.id=eq.1&order=id").await;
        assert_eq!(status, StatusCode::OK, "compat={compat} {body}");
        assert_eq!(body[0]["todos"], Value::Null, "compat={compat} {body}");

        // C3: like and ilike take their operand untyped, in every spelling.
        let (status, body) = ask("/todos?select=name&name=like.*&order=id").await;
        assert_eq!(status, StatusCode::OK, "compat={compat} {body}");
        assert_eq!(body.as_array().unwrap().len(), 3);
        let (_, body) = ask("/todos?select=id&name=ilike.A&order=id").await;
        assert_eq!(body, json!([{"id": 1}]), "compat={compat}");
        let (_, body) = ask("/todos?select=id&name=like(any).{a,b}&order=id").await;
        assert_eq!(body, json!([{"id": 1}, {"id": 2}]), "compat={compat}");
        let (_, body) = ask("/todos?select=id&name=not.like(all).{*,a}&order=id").await;
        assert_eq!(body, json!([{"id": 2}, {"id": 3}]), "compat={compat}");
        let (status, _) = ask("/todos?select=id&name=like.it%27s").await;
        assert_eq!(status, StatusCode::OK, "compat={compat}");
        if compat {
            let (status, body) = ask("/todos?select=id,name&label_color=ilike.%23*100").await;
            assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
            assert_eq!(body["code"], "42883", "{body}");
            let message = body["message"].as_str().unwrap();
            assert!(
                message.starts_with("operator does not exist: ")
                    && message.ends_with("~~* unknown"),
                "{body}"
            );
        }

        // C6: a cast to an array type comes back as an array.
        let (_, body) = ask("/arrays?select=id,nums::_int4&order=id").await;
        assert_eq!(
            body,
            json!([
                {"id": 1, "nums": [1, 2, 3]},
                {"id": 2, "nums": [4, 5]},
                {"id": 3, "nums": []},
            ]),
            "compat={compat}"
        );
        let (_, body) = ask("/arrays?select=id::int,total:id::text&id=eq.1").await;
        assert_eq!(body, json!([{"id": 1, "total": "1"}]), "compat={compat}");

        // C8: an embed's order carries the JSON path.
        let (status, body) = ask("/owners?select=id,pets(id)&pets.order=data->>rank.desc").await;
        assert_eq!(status, StatusCode::OK, "compat={compat} {body}");
        assert_eq!(
            body[0]["pets"],
            json!([{"id": 1}, {"id": 3}, {"id": 2}]),
            "compat={compat}"
        );
        let (_, body) = ask("/owners?select=id,pets(id)&pets.order=data->>rank.asc").await;
        assert_eq!(
            body[0]["pets"],
            json!([{"id": 2}, {"id": 3}, {"id": 1}]),
            "compat={compat}"
        );

        // C1: a malformed JSON operand says where and what.
        let (status, body) = ask("/arrays?select=data->>--34").await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "compat={compat}");
        assert_eq!(
            body,
            json!({
                "code": "PGRST100",
                "message": "\"failed to parse select parameter (data->>--34)\" (line 1, column 9)",
                "details": "unexpected \"-\" expecting digit",
                "hint": null,
            }),
            "compat={compat}"
        );
        let (status, body) = ask("/arrays?select=data->(x").await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "compat={compat}");
        assert_eq!(
            body["message"], "\"failed to parse select parameter (data->(x)\" (line 1, column 7)",
            "compat={compat}"
        );

        // C9: text left over after the last select item.
        let (status, body) = ask("/entities?select=name,child_entities(id))&order=id").await;
        match compat {
            true => {
                assert_eq!(status, StatusCode::OK, "{body}");
                assert_eq!(body.as_array().unwrap().len(), 2);
            }
            false => {
                assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
                assert_eq!(body["code"], "PGRST100", "{body}");
            }
        }
        let (status, _) = ask("/entities?select=name,").await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "compat={compat}");

        // C10: a select column the table does not have.
        let (status, body) = ask("/todos?select=id,banana").await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "compat={compat}");
        match compat {
            true => {
                assert_eq!(body["code"], "42703", "{body}");
                assert_eq!(
                    body["message"], "column todos.banana does not exist",
                    "{body}"
                );
                assert_eq!(body["details"], Value::Null);
                assert_eq!(body["hint"], Value::Null);
            }
            false => assert_eq!(body["code"], "PGRST204", "{body}"),
        }

        // C7: a function's own columns are shaped like a table's.
        let (status, body) =
            ask("/rpc/returns_complex?select=val->r&val->i=gt.0.5&order=val->>i.desc").await;
        assert_eq!(status, StatusCode::OK, "compat={compat} {body}");
        assert_eq!(body, json!([{"r": 0.3}, {"r": 0.2}]), "compat={compat}");

        let (_, body) = ask("/rpc/three_rows").await;
        assert_eq!(body.as_array().unwrap().len(), 3, "compat={compat} {body}");
        assert_eq!(body[0], json!({"n": 1, "label": "n1"}), "compat={compat}");
        let (_, body) = ask("/rpc/three_rows?select=label&n=gt.1&order=n.desc").await;
        assert_eq!(
            body,
            json!([{"label": "n3"}, {"label": "n2"}]),
            "compat={compat}"
        );
        let (_, body) = ask("/rpc/three_rows?limit=1&order=n.desc").await;
        assert_eq!(body, json!([{"n": 3, "label": "n3"}]), "compat={compat}");
        let (_, body) = ask("/rpc/three_rows?select=n::text&n=eq.2").await;
        assert_eq!(body, json!([{"n": "2"}]), "compat={compat}");

        // OUT parameters: one row.
        let (_, plain) = ask("/rpc/out_params").await;
        let (_, shaped) = ask("/rpc/out_params?select=b").await;
        match compat {
            true => {
                assert_eq!(plain, json!({"a": 7, "b": "seven"}));
                assert_eq!(shaped, json!({"b": "seven"}));
            }
            false => {
                assert_eq!(plain, json!([{"a": 7, "b": "seven"}]));
                assert_eq!(shaped, json!([{"b": "seven"}]));
            }
        }
        let (_, body) = ask("/rpc/out_params?select=a&b=eq.nope").await;
        assert!(
            body == json!([]) || body == Value::Null,
            "compat={compat} {body}"
        );

        // A scalar and a table function are unchanged.
        let (_, body) = ask("/rpc/answer").await;
        match compat {
            true => assert_eq!(body, json!(42)),
            false => assert_eq!(body, json!([{"answer": 42}])),
        }
        let (_, body) = ask("/rpc/all_todos?select=id,name&id=gt.1&order=id.desc").await;
        assert_eq!(
            body,
            json!([{"id": 3, "name": "c"}, {"id": 2, "name": "b"}]),
            "compat={compat}"
        );
        let (_, body) = ask("/rpc/all_todos?select=label_color&id=eq.3").await;
        assert_eq!(body, json!([{"label_color": "#123456"}]), "compat={compat}");
    }
}
