//! Database-backed configuration loading and persistence.
//!
//! The database equivalent of `config::file`: the same [`Route`] and
//! [`Upstream`] values a TOML config parses into, read from and written to the
//! tables in `migrations/20260901000001_proxy_config.sql`.
//!
//! These are the *global* proxy configuration tables, distinct from the
//! `proxy_domain_*` tables the multi-tenant SaaS module owns, which are scoped
//! to a tenant and a domain.
//!
//! Queries use runtime sqlx (`query`) rather than the compile-time-checked
//! macros, matching `saas::db`, so the crate builds without a live database.
//!
//! # Decoding is strict
//!
//! Every enum column has a `CHECK` constraint, so an unrecognised value should
//! be impossible. When one turns up anyway -- a hand-edited row, a database
//! restored from a newer schema -- these functions return an error rather than
//! substituting a default. A silent default here means a route quietly matching
//! differently, or a rate limit quietly not applying, which is worse than
//! refusing to start.

use std::collections::HashMap;

use sqlx::{PgPool, Row};
use uuid::Uuid;

use crate::config::{
    Backend, HealthCheckConfig, LoadBalanceStrategy, PathMatchType, RateLimitKey, Route,
    RouteMatch, RouteRateLimit, Upstream, UpstreamHttpVersion,
};
use crate::error::{ProxyError, ProxyResult};

// ============================================================================
// Enum <-> string
//
// The stored spelling is the serde-canonical form of each variant, so what is
// in the database matches what a TOML config would say.
// ============================================================================

fn lb_strategy_str(strategy: &LoadBalanceStrategy) -> &'static str {
    match strategy {
        LoadBalanceStrategy::RoundRobin => "round_robin",
        LoadBalanceStrategy::LeastConnections => "least_connections",
        LoadBalanceStrategy::Weighted => "weighted",
        LoadBalanceStrategy::Random => "random",
        LoadBalanceStrategy::Sticky => "sticky",
    }
}

fn lb_strategy_from(value: &str) -> ProxyResult<LoadBalanceStrategy> {
    Ok(match value {
        "round_robin" => LoadBalanceStrategy::RoundRobin,
        "least_connections" => LoadBalanceStrategy::LeastConnections,
        "weighted" => LoadBalanceStrategy::Weighted,
        "random" => LoadBalanceStrategy::Random,
        "sticky" => LoadBalanceStrategy::Sticky,
        other => return Err(unknown("lb_strategy", other)),
    })
}

fn path_type_str(path_type: &PathMatchType) -> &'static str {
    match path_type {
        PathMatchType::Prefix => "prefix",
        PathMatchType::Exact => "exact",
        PathMatchType::Regex => "regex",
    }
}

fn path_type_from(value: &str) -> ProxyResult<PathMatchType> {
    Ok(match value {
        "prefix" => PathMatchType::Prefix,
        "exact" => PathMatchType::Exact,
        "regex" => PathMatchType::Regex,
        other => return Err(unknown("match_path_type", other)),
    })
}

fn http_version_str(version: &UpstreamHttpVersion) -> &'static str {
    match version {
        UpstreamHttpVersion::Http11 => "http11",
        UpstreamHttpVersion::H2c => "h2c",
    }
}

fn http_version_from(value: &str) -> ProxyResult<UpstreamHttpVersion> {
    Ok(match value {
        "http11" => UpstreamHttpVersion::Http11,
        "h2c" => UpstreamHttpVersion::H2c,
        other => return Err(unknown("http_version", other)),
    })
}

/// `RateLimitKey` split into the two columns that hold it: a discriminant, and
/// the header name that only the `Header` variant carries.
fn rate_limit_key_columns(key: &RateLimitKey) -> (&'static str, Option<&str>) {
    match key {
        RateLimitKey::ClientIp => ("client_ip", None),
        RateLimitKey::Header(name) => ("header", Some(name.as_str())),
        RateLimitKey::Route => ("route", None),
    }
}

fn rate_limit_key_from(kind: &str, header: Option<String>) -> ProxyResult<RateLimitKey> {
    Ok(match (kind, header) {
        ("client_ip", _) => RateLimitKey::ClientIp,
        ("route", _) => RateLimitKey::Route,
        ("header", Some(name)) => RateLimitKey::Header(name),
        // The schema's `proxy_routes_rate_limit_header` constraint makes this
        // unreachable through normal writes; say so rather than inventing a
        // header name to limit on.
        ("header", None) => {
            return Err(ProxyError::Config(
                "rate_limit_key is 'header' but rate_limit_header is NULL".into(),
            ))
        }
        (other, _) => return Err(unknown("rate_limit_key", other)),
    })
}

fn unknown(column: &str, value: &str) -> ProxyError {
    ProxyError::Config(format!(
        "unrecognised {column} value {value:?} in the proxy configuration tables"
    ))
}

/// Widen a stored `INTEGER` into the `u32` the config types use.
///
/// The schema forbids negatives on every column this is applied to, so a
/// failure here means the row was written around the constraints.
fn positive(column: &str, value: i32) -> ProxyResult<u32> {
    u32::try_from(value)
        .map_err(|_| ProxyError::Config(format!("{column} is negative ({value}) in the database")))
}

/// Narrow a `u32` from the config types into a storable `INTEGER`.
fn storable(column: &str, value: u32) -> ProxyResult<i32> {
    i32::try_from(value).map_err(|_| {
        ProxyError::Validation(format!(
            "{column} is too large to store ({value} > 2147483647)"
        ))
    })
}

// ============================================================================
// Loading
// ============================================================================

/// Load the whole proxy configuration: every route and every upstream.
///
/// Rows are returned whether or not they are `enabled`; that flag is part of
/// the configuration, and the caller decides what to do with it, exactly as it
/// would for a TOML config.
pub async fn load_from_database(pool: &PgPool) -> ProxyResult<(Vec<Route>, Vec<Upstream>)> {
    let routes = load_routes(pool).await?;
    let upstreams = load_upstreams(pool).await?;
    Ok((routes, upstreams))
}

/// Load every route, highest priority first.
///
/// The ordering is not cosmetic: routes are matched in priority order, so
/// loading them in that order means the in-memory table is already in matching
/// order. `name` breaks ties so that two runs against an unchanged database
/// produce the same table.
pub async fn load_routes(pool: &PgPool) -> ProxyResult<Vec<Route>> {
    let rows = sqlx::query(
        "SELECT id, name, description, match_host, match_path, match_path_type, \
                match_headers, match_methods, priority, upstream, strip_path, \
                add_headers, remove_headers, rate_limit_requests, \
                rate_limit_window_secs, rate_limit_key, rate_limit_header, \
                timeout_secs, retry_count, enabled \
         FROM proxy_routes \
         ORDER BY priority DESC, name ASC",
    )
    .fetch_all(pool)
    .await?;

    rows.into_iter().map(route_from_row).collect()
}

/// Load every upstream, each with its backends.
///
/// Two queries rather than one per upstream: the backends come back in a single
/// pass and are grouped in memory. A proxy with a hundred upstreams should not
/// make a hundred and one round trips to start.
pub async fn load_upstreams(pool: &PgPool) -> ProxyResult<Vec<Upstream>> {
    let rows = sqlx::query(
        "SELECT id, name, description, lb_strategy, health_check_enabled, \
                health_check_path, health_check_interval_secs, \
                health_check_timeout_secs, healthy_threshold, \
                unhealthy_threshold, enabled \
         FROM proxy_upstreams \
         ORDER BY name ASC",
    )
    .fetch_all(pool)
    .await?;

    let mut backends = load_all_backends(pool).await?;

    rows.into_iter()
        .map(|row| {
            let id: Uuid = row.try_get("id")?;
            Ok(Upstream {
                id: Some(id),
                name: row.try_get("name")?,
                description: row.try_get("description")?,
                lb_strategy: lb_strategy_from(row.try_get("lb_strategy")?)?,
                backends: backends.remove(&id).unwrap_or_default(),
                health_check: HealthCheckConfig {
                    enabled: row.try_get("health_check_enabled")?,
                    path: row.try_get("health_check_path")?,
                    interval_secs: positive(
                        "health_check_interval_secs",
                        row.try_get("health_check_interval_secs")?,
                    )?,
                    timeout_secs: positive(
                        "health_check_timeout_secs",
                        row.try_get("health_check_timeout_secs")?,
                    )?,
                    healthy_threshold: positive(
                        "healthy_threshold",
                        row.try_get("healthy_threshold")?,
                    )?,
                    unhealthy_threshold: positive(
                        "unhealthy_threshold",
                        row.try_get("unhealthy_threshold")?,
                    )?,
                },
                enabled: row.try_get("enabled")?,
            })
        })
        .collect()
}

/// Every backend in the database, grouped by the upstream it belongs to.
async fn load_all_backends(pool: &PgPool) -> ProxyResult<HashMap<Uuid, Vec<Backend>>> {
    let rows = sqlx::query(
        "SELECT id, upstream_id, address, scheme, weight, http_version, enabled \
         FROM proxy_upstream_backends \
         ORDER BY address ASC",
    )
    .fetch_all(pool)
    .await?;

    let mut grouped: HashMap<Uuid, Vec<Backend>> = HashMap::new();
    for row in rows {
        let upstream_id: Uuid = row.try_get("upstream_id")?;
        grouped.entry(upstream_id).or_default().push(Backend {
            id: Some(row.try_get("id")?),
            address: row.try_get("address")?,
            scheme: row.try_get("scheme")?,
            weight: positive("weight", row.try_get("weight")?)?,
            enabled: row.try_get("enabled")?,
            http_version: http_version_from(row.try_get("http_version")?)?,
        });
    }
    Ok(grouped)
}

fn route_from_row(row: sqlx::postgres::PgRow) -> ProxyResult<Route> {
    // All three columns are present or none are, enforced by
    // `proxy_routes_rate_limit_whole`; read the discriminant and let the other
    // two follow it rather than assembling a partial limit.
    let rate_limit = match row.try_get::<Option<String>, _>("rate_limit_key")? {
        Some(kind) => Some(RouteRateLimit {
            requests: positive(
                "rate_limit_requests",
                row.try_get::<Option<i32>, _>("rate_limit_requests")?
                    .ok_or_else(|| {
                        ProxyError::Config("rate_limit_requests is NULL with a key set".into())
                    })?,
            )?,
            window_secs: positive(
                "rate_limit_window_secs",
                row.try_get::<Option<i32>, _>("rate_limit_window_secs")?
                    .ok_or_else(|| {
                        ProxyError::Config("rate_limit_window_secs is NULL with a key set".into())
                    })?,
            )?,
            key: rate_limit_key_from(&kind, row.try_get("rate_limit_header")?)?,
        }),
        None => None,
    };

    let match_headers: sqlx::types::Json<HashMap<String, String>> = row.try_get("match_headers")?;
    let add_headers: sqlx::types::Json<HashMap<String, String>> = row.try_get("add_headers")?;

    Ok(Route {
        id: Some(row.try_get("id")?),
        name: row.try_get("name")?,
        description: row.try_get("description")?,
        match_: RouteMatch {
            host: row.try_get("match_host")?,
            path: row.try_get("match_path")?,
            path_type: path_type_from(row.try_get("match_path_type")?)?,
            headers: match_headers.0,
            methods: row.try_get("match_methods")?,
        },
        priority: row.try_get("priority")?,
        upstream: row.try_get("upstream")?,
        strip_path: row.try_get("strip_path")?,
        add_headers: add_headers.0,
        remove_headers: row.try_get("remove_headers")?,
        rate_limit,
        timeout_secs: positive("timeout_secs", row.try_get("timeout_secs")?)?,
        retry_count: positive("retry_count", row.try_get("retry_count")?)?,
        enabled: row.try_get("enabled")?,
    })
}

// ============================================================================
// Persistence
// ============================================================================

/// Insert a route, or replace it if one with the same id is already stored.
///
/// Returns the route as stored, with `id` filled in. A route arriving without
/// one gets a fresh id rather than being rejected: the admin API generates ids
/// itself, but a caller building a `Route` by hand should not have to.
pub async fn save_route(pool: &PgPool, route: &Route) -> ProxyResult<Route> {
    let id = route.id.unwrap_or_else(Uuid::new_v4);
    let (rate_limit_key, rate_limit_header) = match &route.rate_limit {
        Some(limit) => {
            let (kind, header) = rate_limit_key_columns(&limit.key);
            (Some(kind), header.map(str::to_owned))
        }
        None => (None, None),
    };
    let (rate_limit_requests, rate_limit_window_secs) = match &route.rate_limit {
        Some(limit) => (
            Some(storable("rate_limit_requests", limit.requests)?),
            Some(storable("rate_limit_window_secs", limit.window_secs)?),
        ),
        None => (None, None),
    };

    sqlx::query(
        "INSERT INTO proxy_routes \
            (id, name, description, match_host, match_path, match_path_type, \
             match_headers, match_methods, priority, upstream, strip_path, \
             add_headers, remove_headers, rate_limit_requests, \
             rate_limit_window_secs, rate_limit_key, rate_limit_header, \
             timeout_secs, retry_count, enabled) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, \
                 $15, $16, $17, $18, $19, $20) \
         ON CONFLICT (id) DO UPDATE SET \
             name = EXCLUDED.name, \
             description = EXCLUDED.description, \
             match_host = EXCLUDED.match_host, \
             match_path = EXCLUDED.match_path, \
             match_path_type = EXCLUDED.match_path_type, \
             match_headers = EXCLUDED.match_headers, \
             match_methods = EXCLUDED.match_methods, \
             priority = EXCLUDED.priority, \
             upstream = EXCLUDED.upstream, \
             strip_path = EXCLUDED.strip_path, \
             add_headers = EXCLUDED.add_headers, \
             remove_headers = EXCLUDED.remove_headers, \
             rate_limit_requests = EXCLUDED.rate_limit_requests, \
             rate_limit_window_secs = EXCLUDED.rate_limit_window_secs, \
             rate_limit_key = EXCLUDED.rate_limit_key, \
             rate_limit_header = EXCLUDED.rate_limit_header, \
             timeout_secs = EXCLUDED.timeout_secs, \
             retry_count = EXCLUDED.retry_count, \
             enabled = EXCLUDED.enabled, \
             updated_at = NOW()",
    )
    .bind(id)
    .bind(&route.name)
    .bind(&route.description)
    .bind(&route.match_.host)
    .bind(&route.match_.path)
    .bind(path_type_str(&route.match_.path_type))
    .bind(sqlx::types::Json(&route.match_.headers))
    .bind(&route.match_.methods)
    .bind(route.priority)
    .bind(&route.upstream)
    .bind(route.strip_path)
    .bind(sqlx::types::Json(&route.add_headers))
    .bind(&route.remove_headers)
    .bind(rate_limit_requests)
    .bind(rate_limit_window_secs)
    .bind(rate_limit_key)
    .bind(rate_limit_header)
    .bind(storable("timeout_secs", route.timeout_secs)?)
    .bind(storable("retry_count", route.retry_count)?)
    .bind(route.enabled)
    .execute(pool)
    .await?;

    Ok(Route {
        id: Some(id),
        ..route.clone()
    })
}

/// Delete a route. Returns whether a row was removed.
pub async fn delete_route(pool: &PgPool, id: Uuid) -> ProxyResult<bool> {
    let result = sqlx::query("DELETE FROM proxy_routes WHERE id = $1")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(result.rows_affected() > 0)
}

/// Insert an upstream and its backends, or replace them if the id is stored.
///
/// The upstream and its backend set are written in one transaction, and the
/// backends are replaced wholesale rather than merged: `Upstream::backends` is
/// the complete set, so a backend dropped from the value has to disappear from
/// the database. Doing that outside a transaction would leave a window where
/// the upstream has no backends at all and every request through it fails.
pub async fn save_upstream(pool: &PgPool, upstream: &Upstream) -> ProxyResult<Upstream> {
    let id = upstream.id.unwrap_or_else(|| upstream.resolved_id());
    let health = &upstream.health_check;

    let mut tx = pool.begin().await?;

    sqlx::query(
        "INSERT INTO proxy_upstreams \
            (id, name, description, lb_strategy, health_check_enabled, \
             health_check_path, health_check_interval_secs, \
             health_check_timeout_secs, healthy_threshold, unhealthy_threshold, \
             enabled) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11) \
         ON CONFLICT (id) DO UPDATE SET \
             name = EXCLUDED.name, \
             description = EXCLUDED.description, \
             lb_strategy = EXCLUDED.lb_strategy, \
             health_check_enabled = EXCLUDED.health_check_enabled, \
             health_check_path = EXCLUDED.health_check_path, \
             health_check_interval_secs = EXCLUDED.health_check_interval_secs, \
             health_check_timeout_secs = EXCLUDED.health_check_timeout_secs, \
             healthy_threshold = EXCLUDED.healthy_threshold, \
             unhealthy_threshold = EXCLUDED.unhealthy_threshold, \
             enabled = EXCLUDED.enabled, \
             updated_at = NOW()",
    )
    .bind(id)
    .bind(&upstream.name)
    .bind(&upstream.description)
    .bind(lb_strategy_str(&upstream.lb_strategy))
    .bind(health.enabled)
    .bind(&health.path)
    .bind(storable(
        "health_check_interval_secs",
        health.interval_secs,
    )?)
    .bind(storable("health_check_timeout_secs", health.timeout_secs)?)
    .bind(storable("healthy_threshold", health.healthy_threshold)?)
    .bind(storable("unhealthy_threshold", health.unhealthy_threshold)?)
    .bind(upstream.enabled)
    .execute(&mut *tx)
    .await?;

    sqlx::query("DELETE FROM proxy_upstream_backends WHERE upstream_id = $1")
        .bind(id)
        .execute(&mut *tx)
        .await?;

    let mut stored = Vec::with_capacity(upstream.backends.len());
    for backend in &upstream.backends {
        let backend_id = backend.id.unwrap_or_else(Uuid::new_v4);
        sqlx::query(
            "INSERT INTO proxy_upstream_backends \
                (id, upstream_id, address, scheme, weight, http_version, enabled) \
             VALUES ($1, $2, $3, $4, $5, $6, $7)",
        )
        .bind(backend_id)
        .bind(id)
        .bind(&backend.address)
        .bind(&backend.scheme)
        .bind(storable("weight", backend.weight)?)
        .bind(http_version_str(&backend.http_version))
        .bind(backend.enabled)
        .execute(&mut *tx)
        .await?;

        stored.push(Backend {
            id: Some(backend_id),
            ..backend.clone()
        });
    }

    tx.commit().await?;

    Ok(Upstream {
        id: Some(id),
        backends: stored,
        ..upstream.clone()
    })
}

/// Delete an upstream and, by cascade, its backends. Returns whether a row was
/// removed.
pub async fn delete_upstream(pool: &PgPool, id: Uuid) -> ProxyResult<bool> {
    let result = sqlx::query("DELETE FROM proxy_upstreams WHERE id = $1")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(result.rows_affected() > 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    // The enum mappings are the part that can silently corrupt a config: a
    // wrong string round-trips as a *different valid* value rather than an
    // error. Every variant, both directions.

    #[test]
    fn lb_strategy_round_trips_every_variant() {
        for strategy in [
            LoadBalanceStrategy::RoundRobin,
            LoadBalanceStrategy::LeastConnections,
            LoadBalanceStrategy::Weighted,
            LoadBalanceStrategy::Random,
            LoadBalanceStrategy::Sticky,
        ] {
            let stored = lb_strategy_str(&strategy);
            assert_eq!(
                lb_strategy_from(stored).unwrap(),
                strategy,
                "via {stored:?}"
            );
        }
    }

    #[test]
    fn path_type_round_trips_every_variant() {
        for path_type in [
            PathMatchType::Prefix,
            PathMatchType::Exact,
            PathMatchType::Regex,
        ] {
            let stored = path_type_str(&path_type);
            assert_eq!(path_type_from(stored).unwrap(), path_type, "via {stored:?}");
        }
    }

    #[test]
    fn http_version_round_trips_every_variant() {
        for version in [UpstreamHttpVersion::Http11, UpstreamHttpVersion::H2c] {
            let stored = http_version_str(&version);
            assert_eq!(
                http_version_from(stored).unwrap(),
                version,
                "via {stored:?}"
            );
        }
    }

    #[test]
    fn rate_limit_key_round_trips_every_variant() {
        for key in [
            RateLimitKey::ClientIp,
            RateLimitKey::Route,
            RateLimitKey::Header("x-api-key".into()),
        ] {
            let (kind, header) = rate_limit_key_columns(&key);
            let back = rate_limit_key_from(kind, header.map(str::to_owned)).unwrap();
            assert_eq!(
                format!("{back:?}"),
                format!("{key:?}"),
                "via {kind:?}/{header:?}"
            );
        }
    }

    #[test]
    fn only_the_header_variant_stores_a_header() {
        assert_eq!(rate_limit_key_columns(&RateLimitKey::ClientIp).1, None);
        assert_eq!(rate_limit_key_columns(&RateLimitKey::Route).1, None);
        assert_eq!(
            rate_limit_key_columns(&RateLimitKey::Header("x-tenant".into())).1,
            Some("x-tenant")
        );
    }

    #[test]
    fn unknown_enum_values_are_refused_not_defaulted() {
        // The bug this guards against is `unwrap_or_default()`, which would
        // turn an unreadable row into a plausible, wrong configuration.
        assert!(lb_strategy_from("round-robin").is_err());
        assert!(lb_strategy_from("").is_err());
        assert!(path_type_from("glob").is_err());
        assert!(
            http_version_from("h2").is_err(),
            "alias is not the stored form"
        );
        assert!(rate_limit_key_from("ip", None).is_err());
    }

    #[test]
    fn a_header_key_without_a_header_is_an_error() {
        assert!(rate_limit_key_from("header", None).is_err());
    }

    #[test]
    fn negative_integers_from_the_database_are_refused() {
        assert!(positive("weight", -1).is_err());
        assert_eq!(positive("weight", 0).unwrap(), 0);
        assert_eq!(positive("weight", 7).unwrap(), 7);
    }

    #[test]
    fn oversized_integers_are_refused_before_they_wrap() {
        assert!(storable("timeout_secs", u32::MAX).is_err());
        assert_eq!(storable("timeout_secs", 30).unwrap(), 30);
        assert_eq!(storable("timeout_secs", i32::MAX as u32).unwrap(), i32::MAX);
    }
}
