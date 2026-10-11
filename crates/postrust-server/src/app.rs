//! Request handling: the axum side of the REST API.
//!
//! The pipeline itself -- authentication, parsing, planning, execution -- is
//! `postrust-engine`, so that a host without axum can run it too. What stays
//! here is what only a router has: the handler signature, and the `OPTIONS`
//! layer that has to see paths before the mount is stripped off.

use crate::state::AppState;
use axum::{
    body::Body,
    extract::{Request, State},
    response::Response,
};
use postrust_engine::error_response;
use std::sync::Arc;

/// Main request handler.
pub async fn handle_request(State(state): State<Arc<AppState>>, request: Request) -> Response {
    #[cfg(not(feature = "pg-socket"))]
    let response = postrust_engine::handle(state.as_ref(), request).await;
    #[cfg(feature = "pg-socket")]
    let response = postrust_engine::handle(&via_socket::ViaSocket(state), request).await;
    response.map(Body::from)
}

/// The test-only `pg-socket` feature: REST through the Worker's driver.
#[cfg(feature = "pg-socket")]
mod via_socket {
    use crate::state::AppState;
    use postrust_auth::{JwtCache, JwtConfig};
    use postrust_core::db::tokio_postgres::config::{Host, SslMode};
    use postrust_core::db::tokio_postgres::{Config, NoTls};
    use postrust_core::db::{BoxFuture, Connector, PgSocketDb};
    use postrust_core::{AppConfig, SchemaCache};
    use std::sync::{Arc, Mutex};
    use tokio::sync::RwLockReadGuard;

    /// A TCP socket per request, where a Worker opens one to Hyperdrive.
    ///
    /// Or why there can be none: a database URI this build cannot use fails
    /// each request with the reason, rather than panicking inside one.
    pub struct Tcp(Result<(String, u16), String>);

    impl Connector for Tcp {
        type Stream = tokio::net::TcpStream;
        type Tls = NoTls;

        async fn connect(&self) -> std::io::Result<tokio::net::TcpStream> {
            let (host, port) = self
                .0
                .as_ref()
                .map_err(|e| std::io::Error::other(e.clone()))?;
            let stream = tokio::net::TcpStream::connect((host.as_str(), *port)).await?;
            stream.set_nodelay(true)?;
            Ok(stream)
        }

        fn tls(&self) -> NoTls {
            NoTls
        }

        fn spawn(&self, connection: BoxFuture<'static, ()>) {
            tokio::spawn(connection);
        }

        async fn sleep(&self, duration: std::time::Duration) {
            tokio::time::sleep(duration).await
        }
    }

    /// The database a URI names, as this build can reach it: plain TCP, no
    /// TLS. `sslmode=require` is refused here, saying so, rather than at
    /// every connection as a failed TLS handshake.
    pub(super) fn database(uri: &str) -> PgSocketDb<Tcp> {
        let parsed = uri
            .parse::<Config>()
            .map_err(|e| format!("pg-socket: the database URI does not parse: {e}"));
        let (config, target) = match parsed {
            Ok(config) => {
                let target = match (config.get_hosts().first(), config.get_ports().first()) {
                    _ if config.get_ssl_mode() == SslMode::Require => Err(
                        "pg-socket: this build connects without TLS, and the database URI \
                         says sslmode=require"
                            .to_string(),
                    ),
                    (Some(Host::Tcp(host)), port) => {
                        Ok((host.clone(), port.copied().unwrap_or(5432)))
                    }
                    _ => Err("pg-socket: the database URI needs a TCP host".to_string()),
                };
                (config, target)
            }
            Err(e) => (Config::new(), Err(e)),
        };
        match &target {
            Ok((host, port)) => tracing::warn!(
                "pg-socket: REST requests run through the socket driver, one connection \
                 per request to {host}:{port}. This build is for testing."
            ),
            Err(e) => tracing::error!("{e}"),
        }
        PgSocketDb::new(Tcp(target), config)
    }

    /// One database per URI, made the first time a request names it.
    ///
    /// Per URI rather than one for the process: the first request's state used
    /// to decide it for every state after, so a second `AppState` -- a test's,
    /// logging in as another role -- was served through the first one's
    /// login. Leaked, since a database is needed for as long as the process
    /// runs, and a process has a URI or two.
    static DBS: Mutex<Vec<(String, &'static PgSocketDb<Tcp>)>> = Mutex::new(Vec::new());

    fn database_for(uri: &str) -> &'static PgSocketDb<Tcp> {
        let mut known = DBS.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some((_, db)) = known.iter().find(|(known, _)| known == uri) {
            return db;
        }
        let db: &'static PgSocketDb<Tcp> = Box::leak(Box::new(database(uri)));
        known.push((uri.to_string(), db));
        db
    }

    /// The server's state, with the socket driver in place of the pool.
    ///
    /// Owns its `Arc` rather than borrowing: a borrowed state gives the
    /// request future a lifetime the router cannot prove `Send` for every
    /// choice of.
    pub struct ViaSocket(pub Arc<AppState>);

    impl postrust_engine::State for ViaSocket {
        type Db = PgSocketDb<Tcp>;
        type SchemaCache<'a>
            = RwLockReadGuard<'a, SchemaCache>
        where
            Self: 'a;

        fn db(&self) -> &PgSocketDb<Tcp> {
            database_for(&self.0.config.db_uri)
        }

        async fn schema_cache(&self) -> RwLockReadGuard<'_, SchemaCache> {
            self.0.schema_cache().await
        }

        fn config(&self) -> &AppConfig {
            &self.0.config
        }

        fn jwt_config(&self) -> &JwtConfig {
            &self.0.jwt_config
        }

        fn jwt_cache(&self) -> Option<&JwtCache> {
            self.0.jwt_cache.as_ref()
        }
    }
}

/// Answer an `OPTIONS` with the methods its resource actually has.
///
/// This runs outside the CORS layer because that layer answers every `OPTIONS`
/// itself and never calls what it wraps -- `if parts.method == Method::OPTIONS`
/// returns immediately, preflight or not. So an `OPTIONS` never reached the
/// request handler at all: the body was empty because nothing built one, and
/// `Allow` was missing because the code that knows the answer was never asked.
///
/// The CORS layer still writes the response, and this adds to it, so a
/// preflight keeps every header it had.
pub async fn options_allow(
    State(state): State<Arc<AppState>>,
    request: Request,
    next: axum::middleware::Next,
) -> Response {
    if request.method() != http::Method::OPTIONS {
        return next.run(request).await;
    }

    // Not every route is the API's. A layer added to the router wraps every
    // one of them, so `/admin` and `/_` arrive here too, and answering those
    // with what a table allows -- or with "no such table" -- takes a working
    // preflight and turns it into a 404.
    let Some(path) = api_path(request.uri().path(), state.config.compat_mode) else {
        return next.run(request).await;
    };

    let verbatim_db_errors = state.config.compat_mode;

    // Built here, from borrows, before anything is awaited: `axum::body::Body`
    // is `Send` but not `Sync`, and `&T` is `Send` only where `T` is `Sync`, so
    // a `&Request` held across an await makes the whole future not `Send` --
    // which a middleware has to be.
    let probe = probe_request(&request, path);
    let allow = match probe {
        Ok(probe) => postrust_engine::allow(state.as_ref(), probe).await,
        Err(error) => Err(error),
    };

    let mut response = next.run(request).await;

    match allow {
        Ok(allow) => {
            if let Ok(value) = http::HeaderValue::from_str(&allow) {
                response.headers_mut().insert(http::header::ALLOW, value);
            }
            response
        }
        // Asking what may be done with a table that is not there is answered
        // the same way as asking for the table.
        Err(error) => {
            let mut refusal = error_response(error, verbatim_db_errors).map(Body::from);
            // The CORS layer wrote its headers on the response this replaces.
            // Without them a browser reports the request as blocked by CORS
            // rather than as the 404 it earned, which is a different thing to
            // go and fix.
            for (name, value) in response.headers() {
                if name.as_str().starts_with("access-control-") || name == http::header::VARY {
                    refusal.headers_mut().append(name, value.clone());
                }
            }
            refusal
        }
    }
}

/// The path the API parser should read, or `None` where the request names
/// something that is not the REST surface.
///
/// A layer added with `Router::layer` wraps each endpoint, and `nest` puts its
/// prefix-stripping *inside* that endpoint -- so the path seen here is the one
/// the client wrote and still carries the mount it arrived under, unlike the
/// one the handler reads.
fn api_path(path: &str, compat_mode: bool) -> Option<&str> {
    /// Whether a path names a mount point or something under it.
    fn under(path: &str, mount: &str) -> bool {
        path == mount || path.strip_prefix(mount).is_some_and(|r| r.starts_with('/'))
    }

    // Every mount this server answers itself. In compatibility mode the REST
    // surface sits at the root, so a path not named here is read as a table:
    // `/healthz` becomes a question about a table called `healthz`, and the
    // two-segment ones are not a valid resource at all. Either way the
    // refusal replaces the preflight the CORS layer wrote, which is how a
    // browser comes to report a working endpoint as blocked.
    for mount in [
        "/admin",
        "/_",
        "/api/graphql",
        "/healthz",
        "/v1/version",
        "/v1/graphql",
        "/v1alpha1/graphql",
    ] {
        if under(path, mount) {
            return None;
        }
    }
    if under(path, "/api") {
        return Some(match path.strip_prefix("/api") {
            None | Some("") => "/",
            Some(rest) => rest,
        });
    }
    // Everywhere else the REST surface exists only in compatibility mode,
    // where it answers at the root. Without it `/` is this server's own
    // directory of endpoints, which allows what its handler allows.
    compat_mode.then_some(path)
}

/// The request again, with an empty body and the API's own path, for the
/// parser to read.
fn probe_request(
    request: &Request,
    path: &str,
) -> Result<http::Request<bytes::Bytes>, postrust_core::Error> {
    let target = match request.uri().query() {
        Some(query) => format!("{path}?{query}"),
        None => path.to_string(),
    };
    let mut builder = http::Request::builder()
        .method(request.method().clone())
        .uri(target);
    for (name, value) in request.headers() {
        builder = builder.header(name, value);
    }
    builder
        .body(bytes::Bytes::new())
        .map_err(|e| postrust_core::Error::Internal(e.to_string()))
}

#[cfg(test)]
mod tests {
    /// The REST surface is mounted at `/api`, and an `OPTIONS` reaches the
    /// layer before the mount is stripped off.
    #[test]
    fn the_api_path_is_the_one_below_the_mount() {
        assert_eq!(super::api_path("/api/items", false), Some("/items"));
        assert_eq!(super::api_path("/api/rpc/f", false), Some("/rpc/f"));
        assert_eq!(super::api_path("/api", false), Some("/"));
        assert_eq!(super::api_path("/api/", false), Some("/"));
    }

    /// Answering for a route that is not the API's turns a working preflight
    /// into a 404 for a table nobody asked about.
    #[test]
    fn a_route_that_is_not_the_api_is_left_alone() {
        for path in [
            "/admin",
            "/admin/swagger",
            "/_",
            "/_/health",
            "/api/graphql",
            // The endpoints a Hasura deployment has. `/v1/graphql` is the one
            // every generated client is pointed at, so answering its preflight
            // with "no such table" breaks the surface this branch exists to
            // offer -- and only in compatibility mode, where nothing else
            // looks wrong.
            "/healthz",
            "/v1/version",
            "/v1/graphql",
            "/v1/graphql/ws",
            "/v1alpha1/graphql",
        ] {
            assert_eq!(super::api_path(path, false), None, "{path}");
            assert_eq!(super::api_path(path, true), None, "{path}");
        }
    }

    /// The mounts are whole segments here too: a table may be called `v1`.
    #[test]
    fn a_hasura_mount_does_not_swallow_a_table_beside_it() {
        assert_eq!(super::api_path("/v1", true), Some("/v1"));
        assert_eq!(super::api_path("/healthzz", true), Some("/healthzz"));
        assert_eq!(super::api_path("/v1alpha1", true), Some("/v1alpha1"));
    }

    /// Outside the mounts the surface exists only in compatibility mode,
    /// where PostgREST's own paths answer at the root.
    #[test]
    fn the_root_is_the_api_only_in_compatibility_mode() {
        assert_eq!(super::api_path("/items", true), Some("/items"));
        assert_eq!(super::api_path("/", true), Some("/"));
        assert_eq!(super::api_path("/items", false), None);
        assert_eq!(super::api_path("/", false), None);
    }

    /// A mount is a path segment: `/apiary` is not under `/api`.
    #[test]
    fn a_mount_matches_whole_segments_only() {
        assert_eq!(super::api_path("/apiary", true), Some("/apiary"));
        assert_eq!(
            super::api_path("/administrators", true),
            Some("/administrators")
        );
    }

    /// A database URI the socket build cannot use fails the request that
    /// needs it, saying why. It used to panic inside the request handler.
    #[cfg(feature = "pg-socket")]
    #[tokio::test]
    async fn an_unusable_database_uri_fails_requests_rather_than_the_process() {
        use postrust_core::db::Db;

        for (uri, says) in [
            ("postgres://u@:what/db", "does not parse"),
            (
                "postgres://u@localhost/db?sslmode=require",
                "sslmode=require",
            ),
            ("host=/var/run/postgresql dbname=db", "TCP host"),
        ] {
            let db = super::via_socket::database(uri);
            match db.begin().await {
                Err(e) => assert!(e.to_string().contains(says), "{uri}: {e}"),
                Ok(_) => panic!("{uri}: connected"),
            }
        }
    }
}
