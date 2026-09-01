//! Database round-trips for the global proxy configuration tables.
//!
//! These need a live PostgreSQL, so they are `#[ignore]`d and run by the CI
//! job that has one:
//!
//!     DATABASE_URL=postgres://postgres:postgres@localhost:5432/postrust_test \
//!         cargo test -p postrust-proxy --test config_persistence -- --ignored
//!
//! Each test applies the migration itself rather than relying on the schema
//! having been loaded. It is `CREATE TABLE IF NOT EXISTS` throughout, so that
//! is idempotent, and it means these run against any empty database without a
//! setup step to forget.
//!
//! Every test namespaces its rows by a fresh UUID so the file can run in
//! parallel with itself and with anything else using the same database.
//!
//! What these are for: the unit tests in `config::database` cover the enum
//! mappings, but a mapping can be perfect while the SQL is wrong. Only a real
//! round-trip shows that what was written is what comes back.

use std::collections::HashMap;

use postrust_proxy::config::{
    delete_route, delete_upstream, load_routes, load_upstreams, save_route, save_upstream, Backend,
    HealthCheckConfig, LoadBalanceStrategy, PathMatchType, RateLimitKey, Route, RouteMatch,
    RouteRateLimit, Upstream, UpstreamHttpVersion,
};
use sqlx::{Executor, PgPool};
use uuid::Uuid;

const MIGRATION: &str = include_str!("../migrations/20260901000001_proxy_config.sql");

async fn pool() -> PgPool {
    let url = std::env::var("DATABASE_URL")
        .expect("DATABASE_URL must be set for the database-backed tests");
    let pool = PgPool::connect(&url)
        .await
        .expect("could not connect to DATABASE_URL");
    pool.execute(MIGRATION)
        .await
        .expect("could not apply the proxy config migration");
    pool
}

/// A name no other test run will collide with.
fn unique(prefix: &str) -> String {
    format!("{prefix}-{}", Uuid::new_v4())
}

fn route(name: &str, upstream: &str) -> Route {
    Route {
        id: Some(Uuid::new_v4()),
        name: name.to_owned(),
        description: None,
        match_: RouteMatch::default(),
        priority: 100,
        upstream: upstream.to_owned(),
        strip_path: false,
        add_headers: HashMap::new(),
        remove_headers: Vec::new(),
        rate_limit: None,
        timeout_secs: 30,
        retry_count: 0,
        enabled: true,
    }
}

fn upstream(name: &str) -> Upstream {
    Upstream {
        id: Some(Uuid::new_v4()),
        name: name.to_owned(),
        description: None,
        lb_strategy: LoadBalanceStrategy::RoundRobin,
        backends: Vec::new(),
        health_check: HealthCheckConfig::default(),
        enabled: true,
    }
}

async fn find_route(pool: &PgPool, name: &str) -> Option<Route> {
    load_routes(pool)
        .await
        .expect("load_routes failed")
        .into_iter()
        .find(|r| r.name == name)
}

async fn find_upstream(pool: &PgPool, name: &str) -> Option<Upstream> {
    load_upstreams(pool)
        .await
        .expect("load_upstreams failed")
        .into_iter()
        .find(|u| u.name == name)
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn a_saved_route_comes_back_field_for_field() {
    let pool = pool().await;
    let name = unique("full");

    let mut saved = route(&name, "api");
    saved.description = Some("everything set".into());
    saved.match_ = RouteMatch {
        host: Some("api.example.com".into()),
        path: Some("/v1".into()),
        path_type: PathMatchType::Exact,
        headers: HashMap::from([("x-tenant".to_string(), "acme".to_string())]),
        methods: Some(vec!["GET".into(), "POST".into()]),
    };
    saved.priority = 250;
    saved.strip_path = true;
    saved.add_headers = HashMap::from([("x-added".to_string(), "1".to_string())]);
    saved.remove_headers = vec!["x-internal".into()];
    saved.timeout_secs = 45;
    saved.retry_count = 3;
    saved.enabled = false;

    save_route(&pool, &saved).await.expect("save failed");
    let loaded = find_route(&pool, &name).await.expect("route not loaded");

    assert_eq!(loaded.id, saved.id);
    assert_eq!(loaded.description, saved.description);
    assert_eq!(loaded.match_.host, saved.match_.host);
    assert_eq!(loaded.match_.path, saved.match_.path);
    assert_eq!(loaded.match_.path_type, saved.match_.path_type);
    assert_eq!(loaded.match_.headers, saved.match_.headers);
    assert_eq!(loaded.match_.methods, saved.match_.methods);
    assert_eq!(loaded.priority, saved.priority);
    assert_eq!(loaded.upstream, saved.upstream);
    assert_eq!(loaded.strip_path, saved.strip_path);
    assert_eq!(loaded.add_headers, saved.add_headers);
    assert_eq!(loaded.remove_headers, saved.remove_headers);
    assert_eq!(loaded.timeout_secs, saved.timeout_secs);
    assert_eq!(loaded.retry_count, saved.retry_count);
    assert_eq!(loaded.enabled, saved.enabled);

    delete_route(&pool, saved.id.unwrap()).await.unwrap();
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn no_rate_limit_stays_absent() {
    let pool = pool().await;
    let name = unique("no-limit");
    let saved = route(&name, "api");

    save_route(&pool, &saved).await.expect("save failed");
    let loaded = find_route(&pool, &name).await.expect("route not loaded");

    assert!(
        loaded.rate_limit.is_none(),
        "a route saved without a rate limit came back with one"
    );

    delete_route(&pool, saved.id.unwrap()).await.unwrap();
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn every_rate_limit_key_survives_the_round_trip() {
    let pool = pool().await;

    for key in [
        RateLimitKey::ClientIp,
        RateLimitKey::Route,
        RateLimitKey::Header("x-api-key".into()),
    ] {
        let name = unique("limit");
        let mut saved = route(&name, "api");
        saved.rate_limit = Some(RouteRateLimit {
            requests: 120,
            window_secs: 60,
            key: key.clone(),
        });

        save_route(&pool, &saved).await.expect("save failed");
        let loaded = find_route(&pool, &name).await.expect("route not loaded");
        let limit = loaded.rate_limit.expect("rate limit was dropped");

        assert_eq!(limit.requests, 120);
        assert_eq!(limit.window_secs, 60);
        assert_eq!(
            format!("{:?}", limit.key),
            format!("{key:?}"),
            "rate limit key changed in the database"
        );

        delete_route(&pool, saved.id.unwrap()).await.unwrap();
    }
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn saving_the_same_id_twice_updates_rather_than_duplicating() {
    let pool = pool().await;
    let name = unique("upsert");
    let mut saved = route(&name, "api");

    save_route(&pool, &saved).await.expect("first save failed");
    saved.priority = 900;
    saved.upstream = "other".into();
    save_route(&pool, &saved).await.expect("second save failed");

    let matching: Vec<_> = load_routes(&pool)
        .await
        .unwrap()
        .into_iter()
        .filter(|r| r.name == name)
        .collect();

    assert_eq!(matching.len(), 1, "the second save inserted a second row");
    assert_eq!(matching[0].priority, 900);
    assert_eq!(matching[0].upstream, "other");

    delete_route(&pool, saved.id.unwrap()).await.unwrap();
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn routes_load_highest_priority_first() {
    let pool = pool().await;
    let tag = Uuid::new_v4();

    let mut ids = Vec::new();
    for priority in [10, 500, 250] {
        let mut r = route(&format!("prio-{tag}-{priority}"), "api");
        r.priority = priority;
        save_route(&pool, &r).await.expect("save failed");
        ids.push(r.id.unwrap());
    }

    let ours: Vec<i32> = load_routes(&pool)
        .await
        .unwrap()
        .into_iter()
        .filter(|r| r.name.contains(&tag.to_string()))
        .map(|r| r.priority)
        .collect();

    assert_eq!(
        ours,
        vec![500, 250, 10],
        "routes are matched in priority order, so they must load in it"
    );

    for id in ids {
        delete_route(&pool, id).await.unwrap();
    }
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn an_upstream_and_its_backends_come_back_together() {
    let pool = pool().await;
    let name = unique("up");

    let mut saved = upstream(&name);
    saved.description = Some("two backends".into());
    saved.lb_strategy = LoadBalanceStrategy::LeastConnections;
    saved.health_check = HealthCheckConfig {
        enabled: false,
        path: "/healthz".into(),
        interval_secs: 15,
        timeout_secs: 3,
        healthy_threshold: 4,
        unhealthy_threshold: 5,
    };
    saved.backends = vec![
        Backend {
            id: Some(Uuid::new_v4()),
            address: "10.0.0.1:8080".into(),
            scheme: "http".into(),
            weight: 7,
            enabled: true,
            http_version: UpstreamHttpVersion::Http11,
        },
        Backend {
            id: Some(Uuid::new_v4()),
            address: "10.0.0.2:8080".into(),
            scheme: "https".into(),
            weight: 3,
            enabled: false,
            // The reason this column exists: h2c has no ALPN, so it has to be
            // declared, and a round-trip that lost it would silently downgrade
            // the backend to HTTP/1.1.
            http_version: UpstreamHttpVersion::H2c,
        },
    ];

    save_upstream(&pool, &saved).await.expect("save failed");
    let loaded = find_upstream(&pool, &name)
        .await
        .expect("upstream not loaded");

    assert_eq!(loaded.lb_strategy, saved.lb_strategy);
    assert!(!loaded.health_check.enabled);
    assert_eq!(loaded.health_check.path, "/healthz");
    assert_eq!(loaded.health_check.interval_secs, 15);
    assert_eq!(loaded.health_check.timeout_secs, 3);
    assert_eq!(loaded.health_check.healthy_threshold, 4);
    assert_eq!(loaded.health_check.unhealthy_threshold, 5);
    assert_eq!(loaded.backends.len(), 2);

    let h2c = loaded
        .backends
        .iter()
        .find(|b| b.address == "10.0.0.2:8080")
        .expect("second backend missing");
    assert_eq!(h2c.http_version, UpstreamHttpVersion::H2c);
    assert_eq!(h2c.scheme, "https");
    assert_eq!(h2c.weight, 3);
    assert!(!h2c.enabled);

    delete_upstream(&pool, saved.id.unwrap()).await.unwrap();
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn saving_an_upstream_replaces_its_backend_set() {
    let pool = pool().await;
    let name = unique("replace");

    let mut saved = upstream(&name);
    saved.backends = vec![
        Backend {
            id: Some(Uuid::new_v4()),
            address: "10.0.0.1:8080".into(),
            scheme: "http".into(),
            weight: 1,
            enabled: true,
            http_version: UpstreamHttpVersion::Http11,
        },
        Backend {
            id: Some(Uuid::new_v4()),
            address: "10.0.0.2:8080".into(),
            scheme: "http".into(),
            weight: 1,
            enabled: true,
            http_version: UpstreamHttpVersion::Http11,
        },
    ];
    save_upstream(&pool, &saved).await.expect("save failed");

    // Drop one and save again: `backends` is the whole set, so the removed one
    // has to disappear rather than linger and keep taking traffic.
    saved.backends.remove(1);
    save_upstream(&pool, &saved).await.expect("resave failed");

    let loaded = find_upstream(&pool, &name)
        .await
        .expect("upstream not loaded");
    assert_eq!(
        loaded.backends.len(),
        1,
        "a backend removed from the value stayed in the database"
    );
    assert_eq!(loaded.backends[0].address, "10.0.0.1:8080");

    delete_upstream(&pool, saved.id.unwrap()).await.unwrap();
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn deleting_an_upstream_takes_its_backends_with_it() {
    let pool = pool().await;
    let name = unique("cascade");

    let mut saved = upstream(&name);
    saved.backends = vec![Backend {
        id: Some(Uuid::new_v4()),
        address: "10.0.0.9:8080".into(),
        scheme: "http".into(),
        weight: 1,
        enabled: true,
        http_version: UpstreamHttpVersion::Http11,
    }];
    save_upstream(&pool, &saved).await.expect("save failed");
    let id = saved.id.unwrap();

    assert!(delete_upstream(&pool, id).await.unwrap());

    let orphans: i64 =
        sqlx::query_scalar("SELECT count(*) FROM proxy_upstream_backends WHERE upstream_id = $1")
            .bind(id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(orphans, 0, "backends outlived their upstream");
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn deleting_reports_whether_anything_was_there() {
    let pool = pool().await;
    let saved = route(&unique("gone"), "api");
    let id = saved.id.unwrap();

    save_route(&pool, &saved).await.expect("save failed");
    assert!(delete_route(&pool, id).await.unwrap(), "first delete");
    assert!(
        !delete_route(&pool, id).await.unwrap(),
        "deleting a route that is already gone reported success"
    );
    assert!(
        !delete_upstream(&pool, Uuid::new_v4()).await.unwrap(),
        "deleting an upstream that never existed reported success"
    );
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn a_route_saved_without_an_id_gets_one() {
    let pool = pool().await;
    let name = unique("no-id");
    let mut saved = route(&name, "api");
    saved.id = None;

    let stored = save_route(&pool, &saved).await.expect("save failed");
    let id = stored.id.expect("save_route returned a route with no id");

    let loaded = find_route(&pool, &name).await.expect("route not loaded");
    assert_eq!(loaded.id, Some(id));

    delete_route(&pool, id).await.unwrap();
}
