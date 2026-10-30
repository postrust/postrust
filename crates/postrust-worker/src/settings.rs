//! The Worker's decisions that are plain logic: reading its bindings into a
//! configuration, naming and expiring the KV copy of the schema cache, and
//! which response headers the runtime can be handed. None of it touches a
//! JavaScript object, so it is built for every target and tested natively.

use http::HeaderMap;
use postrust_core::AppConfig;

/// The row ceiling when neither max-rows variable is set. An isolate has
/// 128 MB, and an unbounded read of a large table is the quickest way to run
/// out.
pub(crate) const DEFAULT_MAX_ROWS: i64 = 1000;
/// How long an isolate trusts its copy of the schema cache before looking in
/// KV for a newer one, unless `PGRST_WORKER_SCHEMA_TTL` says otherwise.
pub(crate) const DEFAULT_SCHEMA_TTL_SECS: f64 = 60.0;
/// The shortest expiration Workers KV accepts on a `put`.
pub(crate) const KV_MIN_EXPIRATION_SECS: u64 = 60;

/// Both spellings core accepts for the row ceiling (`db-max-rows`, as
/// PostgREST names it, and the older `max-rows`).
const MAX_ROWS_VARS: [&str; 2] = ["PGRST_DB_MAX_ROWS", "PGRST_MAX_ROWS"];
const SCHEMA_TTL_VAR: &str = "PGRST_WORKER_SCHEMA_TTL";
const CORS_ORIGINS_VAR: &str = "PGRST_SERVER_CORS_ORIGINS";

/// A Worker binding's value, as far as configuration cares.
pub(crate) enum Binding {
    Text(String),
    Number(f64),
    Bool(bool),
    /// An object or an array: a JSON variable, or a binding such as
    /// Hyperdrive or a KV namespace.
    Other,
}

/// A binding's value as a process environment would have it, or `None` when
/// it is not one a variable could be.
///
/// Wrangler passes an unquoted TOML value through as what it is: a
/// `PGRST_DB_MAX_ROWS = 5000` arrives as a number and a
/// `PGRST_JWT_SECRET_IS_BASE64 = true` as a boolean. Keeping only strings
/// dropped them without a word -- the second made every token a 401. They are
/// spelled here as a shell would spell them.
pub(crate) fn binding_text(value: Binding) -> Option<String> {
    match value {
        Binding::Text(text) => Some(text),
        // `f64`'s `Display` writes integers without a fraction, as
        // JavaScript's `String(5000)` does.
        Binding::Number(n) => Some(n.to_string()),
        Binding::Bool(b) => Some(b.to_string()),
        Binding::Other => None,
    }
}

/// Whether a binding of this name is meant as configuration, so that one
/// which cannot be read as a variable deserves a warning. Every setting the
/// server reads is a `PGRST_` variable; the Worker's other bindings
/// (Hyperdrive, KV, the reload secret) are not.
pub(crate) fn is_setting(name: &str) -> bool {
    name.starts_with("PGRST_")
}

/// What the Worker runs with.
pub(crate) struct Settings {
    pub(crate) config: AppConfig,
    /// `PGRST_WORKER_SCHEMA_TTL`, in seconds.
    pub(crate) schema_ttl_secs: f64,
}

/// Why the Worker cannot run, and the CORS origins to answer with all the
/// same, so a browser can read the reason.
#[derive(Debug)]
pub(crate) struct Unconfigured {
    pub(crate) reason: String,
    /// `None` when even the origins could not be read; the answer then
    /// carries no CORS headers rather than guessing at a policy.
    pub(crate) cors_origins: Option<Vec<String>>,
}

/// The configuration, from the Worker's variables and secrets -- the server's
/// own names, read from bindings rather than a process environment.
pub(crate) fn settings(pairs: Vec<(String, String)>) -> Result<Settings, Unconfigured> {
    let unconfigured = |reason: String, pairs: &[(String, String)]| Unconfigured {
        reason,
        cors_origins: cors_origins(pairs),
    };
    let schema_ttl_secs = match schema_ttl(
        pairs
            .iter()
            .rev()
            .find(|(k, _)| k == SCHEMA_TTL_VAR)
            .map(|(_, v)| v.as_str()),
    ) {
        Ok(ttl) => ttl,
        Err(e) => return Err(unconfigured(e, &pairs)),
    };
    // `unlimited` is how a deployment that has thought about it says so; the
    // server would warn about the word and ignore it, which here would leave
    // the default ceiling in place. Either spelling of the variable may say
    // it.
    let unlimited = |(k, v): &(String, String)| {
        MAX_ROWS_VARS.contains(&k.as_str()) && v.trim().eq_ignore_ascii_case("unlimited")
    };
    let uncapped = pairs.iter().any(unlimited);
    let config = AppConfig::try_from_vars(pairs.iter().filter(|p| !unlimited(p)).cloned());
    let mut config = match config {
        Ok(config) => config,
        Err(e) => return Err(unconfigured(e.to_string(), &pairs)),
    };
    if config.db_max_rows.is_none() && !uncapped {
        config.db_max_rows = Some(DEFAULT_MAX_ROWS);
    }
    Ok(Settings {
        config,
        schema_ttl_secs,
    })
}

/// `PGRST_SERVER_CORS_ORIGINS` alone, read as the server reads it, for an
/// answer that has no configuration to take it from.
fn cors_origins(pairs: &[(String, String)]) -> Option<Vec<String>> {
    let origins = pairs.iter().filter(|(k, _)| k == CORS_ORIGINS_VAR).cloned();
    AppConfig::try_from_vars(origins)
        .ok()
        .map(|config| config.server_cors_origins)
}

/// `PGRST_WORKER_SCHEMA_TTL`, in seconds. A value that is not a finite,
/// non-negative number is a configuration error: a negative or `NaN` TTL would
/// make every comparison against it false, which is a cache never refreshed.
pub(crate) fn schema_ttl(value: Option<&str>) -> Result<f64, String> {
    let Some(value) = value.map(str::trim).filter(|v| !v.is_empty()) else {
        return Ok(DEFAULT_SCHEMA_TTL_SECS);
    };
    match value.parse::<f64>() {
        Ok(secs) if secs.is_finite() && secs >= 0.0 => Ok(secs),
        _ => Err(format!(
            "Invalid value for {SCHEMA_TTL_VAR}: {value:?} (expected a non-negative number of seconds)"
        )),
    }
}

/// How long the KV copy of the schema cache lives. Without an expiration an
/// isolate would read the catalog only when KV is empty, which with the copy
/// written once is never -- a migration would go unseen until the cron or a
/// reload. So the copy expires with the TTL, or after KV's minimum of 60
/// seconds, and the next isolate to miss loads the catalog again.
pub(crate) fn kv_expiration_secs(schema_ttl_secs: f64) -> u64 {
    (schema_ttl_secs.ceil() as u64).max(KV_MIN_EXPIRATION_SECS)
}

/// The KV key for a configuration's schema cache: which schemas it describes,
/// and which build wrote it. `postrust-core` defines the cache's shape, so its
/// version is in the key as well as this crate's; a record written by another
/// build is never read as this one's.
pub(crate) fn schema_key(config: &AppConfig) -> String {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    config.db_schemas.hash(&mut hasher);
    config.db_extra_search_path.hash(&mut hasher);
    format!(
        "postrust/{}+core.{}/schema-cache/{:016x}",
        env!("CARGO_PKG_VERSION"),
        postrust_core::VERSION,
        hasher.finish()
    )
}

/// The KV key for the copy of the JWKS at `url`, beside the schema cache's
/// in the same namespace and never mistaken for it. Hashed because a URL can
/// be longer than the 512 bytes KV allows a key, and versioned like the schema
/// cache's so that another build's record is not read as this one's.
pub(crate) fn jwks_key(url: &str) -> String {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    url.hash(&mut hasher);
    format!(
        "postrust/{}/jwks/{:016x}",
        env!("CARGO_PKG_VERSION"),
        hasher.finish()
    )
}

/// Whether a schema cache for `key`, loaded from the catalog at `loaded_at`,
/// should replace the one an isolate holds. One for another key always does;
/// one for the same key only when it is strictly newer, so a slower request
/// cannot put back a copy older than what a reload has installed meanwhile.
pub(crate) fn supersedes(held: Option<(&str, f64)>, key: &str, loaded_at: f64) -> bool {
    match held {
        Some((held_key, held_at)) if held_key == key => loaded_at > held_at,
        _ => true,
    }
}

/// Drop the response headers the Workers runtime cannot be handed, and say
/// which. The `worker` crate converts every header to a JavaScript string and
/// fails the whole response on one that is not UTF-8; a JavaScript `Headers`
/// in turn refuses a character past U+00FF. A header set from SQL through
/// `response.headers` can be either, and costing the client that one header
/// is better than costing it the response.
pub(crate) fn drop_unsendable_headers(headers: &mut HeaderMap) -> Vec<String> {
    let sendable = |value: &http::HeaderValue| {
        std::str::from_utf8(value.as_bytes()).is_ok_and(|text| text.chars().all(|c| c <= '\u{ff}'))
    };
    let unsendable: Vec<_> = headers
        .iter()
        .filter(|(_, value)| !sendable(value))
        .map(|(name, _)| name.clone())
        .collect();
    let mut dropped = Vec::new();
    for name in unsendable {
        if dropped.iter().any(|d| d == name.as_str()) {
            continue;
        }
        // A name can carry several values; keep the ones that can be sent.
        let kept: Vec<_> = headers
            .get_all(&name)
            .iter()
            .filter(|v| sendable(v))
            .cloned()
            .collect();
        headers.remove(&name);
        for value in kept {
            headers.append(name.clone(), value);
        }
        dropped.push(name.as_str().to_string());
    }
    dropped
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vars(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn unquoted_numbers_and_booleans_are_read_as_a_shell_would_spell_them() {
        assert_eq!(
            binding_text(Binding::Number(5000.0)).as_deref(),
            Some("5000")
        );
        assert_eq!(binding_text(Binding::Number(1.5)).as_deref(), Some("1.5"));
        assert_eq!(binding_text(Binding::Bool(true)).as_deref(), Some("true"));
        assert_eq!(binding_text(Binding::Bool(false)).as_deref(), Some("false"));
        assert_eq!(
            binding_text(Binding::Text("x".into())).as_deref(),
            Some("x")
        );
        assert_eq!(binding_text(Binding::Other), None);

        // And they take effect: the two the review found dropped.
        let pairs = vec![
            (
                "PGRST_DB_MAX_ROWS".to_string(),
                binding_text(Binding::Number(5000.0)).unwrap(),
            ),
            (
                "PGRST_JWT_SECRET_IS_BASE64".to_string(),
                binding_text(Binding::Bool(true)).unwrap(),
            ),
        ];
        let parsed = settings(pairs).unwrap();
        assert_eq!(parsed.config.db_max_rows, Some(5000));
        assert!(parsed.config.jwt_secret_is_base64);
    }

    #[test]
    fn only_pgrst_names_are_settings() {
        assert!(is_setting("PGRST_DB_MAX_ROWS"));
        assert!(!is_setting("HYPERDRIVE"));
        assert!(!is_setting("POSTRUST_SCHEMA_CACHE"));
    }

    #[test]
    fn rows_are_capped_by_default() {
        let parsed = settings(vars(&[])).unwrap();
        assert_eq!(parsed.config.db_max_rows, Some(DEFAULT_MAX_ROWS));
        assert_eq!(parsed.schema_ttl_secs, DEFAULT_SCHEMA_TTL_SECS);
    }

    #[test]
    fn unlimited_lifts_the_cap_under_either_name() {
        for var in MAX_ROWS_VARS {
            let parsed = settings(vars(&[(var, " Unlimited ")])).unwrap();
            assert_eq!(parsed.config.db_max_rows, None, "{var}");
        }
        // A number under the other name still applies.
        let parsed = settings(vars(&[
            ("PGRST_DB_MAX_ROWS", "unlimited"),
            ("PGRST_MAX_ROWS", "250"),
        ]))
        .unwrap();
        assert_eq!(parsed.config.db_max_rows, Some(250));
        let parsed = settings(vars(&[("PGRST_MAX_ROWS", "250")])).unwrap();
        assert_eq!(parsed.config.db_max_rows, Some(250));
    }

    #[test]
    fn the_schema_ttl_must_be_a_finite_non_negative_number() {
        assert_eq!(schema_ttl(None), Ok(DEFAULT_SCHEMA_TTL_SECS));
        assert_eq!(schema_ttl(Some(" ")), Ok(DEFAULT_SCHEMA_TTL_SECS));
        assert_eq!(schema_ttl(Some("0")), Ok(0.0));
        assert_eq!(schema_ttl(Some(" 2.5 ")), Ok(2.5));
        for bad in ["NaN", "-1", "inf", "-inf", "infinity", "soon"] {
            let e = schema_ttl(Some(bad)).unwrap_err();
            assert!(e.contains("PGRST_WORKER_SCHEMA_TTL"), "{bad}: {e}");
        }
        // As a configuration error, with the CORS origins still known.
        let e = settings(vars(&[
            ("PGRST_WORKER_SCHEMA_TTL", "NaN"),
            ("PGRST_SERVER_CORS_ORIGINS", "https://app.example"),
        ]))
        .err()
        .unwrap();
        assert!(e.reason.contains("PGRST_WORKER_SCHEMA_TTL"));
        assert_eq!(
            e.cors_origins,
            Some(vec!["https://app.example".to_string()])
        );
    }

    #[test]
    fn a_configuration_error_keeps_the_cors_origins() {
        let e = settings(vars(&[
            ("PGRST_JWT_SECRET_IS_BASE64", "perhaps"),
            (
                "PGRST_SERVER_CORS_ORIGINS",
                "https://a.example, https://b.example",
            ),
        ]))
        .err()
        .unwrap();
        assert!(
            e.reason.contains("PGRST_JWT_SECRET_IS_BASE64"),
            "{}",
            e.reason
        );
        assert_eq!(
            e.cors_origins,
            Some(vec![
                "https://a.example".to_string(),
                "https://b.example".to_string()
            ])
        );
        let e = settings(vars(&[("PGRST_JWT_SECRET_IS_BASE64", "perhaps")]))
            .err()
            .unwrap();
        assert_eq!(e.cors_origins, Some(vec![]), "unset is any origin");
    }

    #[test]
    fn the_kv_copy_expires_with_the_ttl_but_not_before_kvs_minimum() {
        assert_eq!(kv_expiration_secs(0.0), 60);
        assert_eq!(kv_expiration_secs(60.0), 60);
        assert_eq!(kv_expiration_secs(90.5), 91);
        assert_eq!(kv_expiration_secs(3600.0), 3600);
    }

    #[test]
    fn the_jwks_key_is_its_own_and_one_per_url() {
        let a = jwks_key("https://a.example/.well-known/jwks.json");
        assert!(a.starts_with(&format!("postrust/{}/jwks/", env!("CARGO_PKG_VERSION"))));
        assert_eq!(a, jwks_key("https://a.example/.well-known/jwks.json"));
        assert_ne!(a, jwks_key("https://b.example/.well-known/jwks.json"));
        let config = settings(vars(&[])).unwrap().config;
        assert_ne!(a, schema_key(&config));
        assert!(jwks_key(&"x".repeat(4096)).len() < 512);
    }

    #[test]
    fn the_kv_key_names_both_versions_and_the_schemas() {
        let config = settings(vars(&[("PGRST_DB_SCHEMAS", "public")]))
            .unwrap()
            .config;
        let key = schema_key(&config);
        assert!(key.contains(env!("CARGO_PKG_VERSION")), "{key}");
        assert!(
            key.contains(&format!("core.{}", postrust_core::VERSION)),
            "{key}"
        );
        let other = settings(vars(&[("PGRST_DB_SCHEMAS", "api")]))
            .unwrap()
            .config;
        assert_ne!(key, schema_key(&other));
    }

    #[test]
    fn an_older_schema_cache_never_replaces_a_newer_one() {
        assert!(supersedes(None, "k", 1.0));
        assert!(supersedes(Some(("k", 1.0)), "k", 2.0));
        assert!(!supersedes(Some(("k", 2.0)), "k", 1.0));
        assert!(!supersedes(Some(("k", 2.0)), "k", 2.0));
        assert!(supersedes(Some(("other", 2.0)), "k", 1.0));
    }

    #[test]
    fn headers_the_runtime_cannot_take_are_dropped_not_the_response() {
        let mut headers = HeaderMap::new();
        headers.insert("content-type", "application/json".parse().unwrap());
        // `HeaderValue` admits obs-text: any byte from 0x80 up.
        headers.insert(
            "x-latin1",
            http::HeaderValue::from_bytes(b"caf\xe9").unwrap(),
        );
        headers.insert(
            "x-euro",
            http::HeaderValue::from_bytes("\u{20ac}".as_bytes()).unwrap(),
        );
        headers.insert(
            "x-accent",
            http::HeaderValue::from_bytes("caf\u{e9}".as_bytes()).unwrap(),
        );
        headers.append("x-multi", "fine".parse().unwrap());
        headers.append("x-multi", http::HeaderValue::from_bytes(b"\xff").unwrap());

        let mut dropped = drop_unsendable_headers(&mut headers);
        dropped.sort();
        assert_eq!(dropped, ["x-euro", "x-latin1", "x-multi"]);
        assert_eq!(headers["content-type"], "application/json");
        assert!(headers.get("x-accent").is_some());
        assert_eq!(
            headers.get_all("x-multi").iter().collect::<Vec<_>>(),
            ["fine"]
        );
        assert!(drop_unsendable_headers(&mut headers).is_empty());
    }
}
