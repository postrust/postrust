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
    postrust_engine::handle(&state, request)
        .await
        .map(Body::from)
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
        Ok(probe) => postrust_engine::allow(&state, probe).await,
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
}
