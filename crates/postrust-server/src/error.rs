//! The error a route of your own answers with, in PostgREST's shape.
//!
//! A client of the API branches on PostgREST's error body -- `code`,
//! `message`, `details`, `hint` -- and on the status that comes with it. A
//! route nested beside the API is answered by the same client, so when it
//! fails it should fail in the same words: a token that does not verify is
//! the same 401, a unique violation from its own `INSERT` is the same 409.
//! [`Error`] is that. It is what [`User`](crate::User) rejects a request
//! with, what [`AppState::begin_as`](crate::AppState::begin_as) returns, and
//! what `?` makes of a `sqlx::Error` in a handler that returns it.

use crate::state::AppState;
use crate::user::User;
use axum::extract::State;
use axum::response::{IntoResponse, Response};
use http::StatusCode;
use std::sync::{Arc, Mutex};

/// A failure answered as the REST API would answer it.
///
/// Wraps the [`postrust_core::Error`] the REST pipeline reports with, so its
/// status and body come from the same table: a database error by its
/// SQLSTATE (`23505` is 409, `42501` a missing grant is 401 or 403, `P0001` a
/// bare `RAISE` is 400), a refusal raised with `sqlstate 'PGRST'` with the status,
/// headers and body the function wrote, a broken connection as 503.
///
/// The database's own account of an error -- its message, detail and hint,
/// which name constraints and columns -- is passed through only where the
/// REST API would pass it through: in compatibility mode (`PGRST_COMPAT_MODE`),
/// for a SQLSTATE `PGRST_DB_ERRORS_VERBATIM` lists, or with `PGRST_DEBUG`. The
/// first two are the server's configuration, which an error converted by `?`
/// has no way to reach, so they are applied on the way
/// out by the routers given to [`Builder::nest`](crate::Builder::nest). A
/// router served some other way gets the sanitised body.
///
/// A missing grant is 401 to an anonymous request and 403 to an authenticated
/// one, as in PostgREST, and an error cannot tell which it is for unless it is
/// told. [`AppState::begin_as`](crate::AppState::begin_as) tells it, and
/// [`Error::for_user`] does for a failure of your own queries; one converted
/// by `?` from somewhere that has not seen the user is answered as for an
/// anonymous request.
pub struct Error(Box<postrust_core::Error>, bool);

impl Error {
    /// The status the built-in table gives this error.
    ///
    /// An error cannot see the configuration, so this is without
    /// `PGRST_DB_ERROR_STATUS` and without compatibility mode: `P0002` is a
    /// 404 here whichever mode the server is in. The response a route sends
    /// is rendered again where the configuration is known (see
    /// [`Builder::nest`](crate::Builder::nest)) and carries the status the
    /// configuration gives, which is what REST answers.
    pub fn status(&self) -> StatusCode {
        self.0.status_code_as(self.1)
    }

    /// This error, answered as it is for `user`: a missing grant is a 403
    /// rather than a 401 where `user` runs as a role other than the
    /// anonymous one.
    ///
    /// For a failure of a query run on a transaction from
    /// [`AppState::begin_as`](crate::AppState::begin_as), which does this for
    /// its own errors: `.map_err(|e| Error::from(e).for_user(&user, &state))`.
    pub fn for_user(mut self, user: &User, state: &AppState) -> Self {
        self.1 = state.config.is_authenticated_role(user.role());
        self
    }

    /// The error as the REST pipeline reports it.
    pub fn as_core(&self) -> &postrust_core::Error {
        &self.0
    }

    /// The error as the REST pipeline reports it, by value.
    pub fn into_core(self) -> postrust_core::Error {
        *self.0
    }
}

impl std::fmt::Debug for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&*self.0)
    }
}

impl From<postrust_core::Error> for Error {
    fn from(error: postrust_core::Error) -> Self {
        Self(Box::new(error), false)
    }
}

impl From<postrust_core::db::DbError> for Error {
    fn from(error: postrust_core::db::DbError) -> Self {
        postrust_core::Error::from(error).into()
    }
}

/// Through the driver-neutral [`DbError`](postrust_core::db::DbError), as the
/// REST pipeline's own sqlx errors go: a statement PostgreSQL refused keeps
/// its SQLSTATE, a connection that broke or could not be had from the pool is
/// the database being unavailable, anything else is internal.
impl From<sqlx::Error> for Error {
    fn from(error: sqlx::Error) -> Self {
        postrust_core::db::DbError::from(error).into()
    }
}

impl IntoResponse for Error {
    fn into_response(self) -> Response {
        // Logged as the REST pipeline logs a query: a fault -- a 5xx, which
        // is the operator's to look at -- at error, and a refusal the client
        // brought on itself at debug, where it does not bury the faults.
        if self.status().is_server_error() {
            tracing::error!("Custom route error: {}", self.0);
        } else {
            tracing::debug!("Custom route refused with {}: {}", self.status(), self.0);
        }

        // Rendered sanitised, which is right unless the server passes
        // database errors through (compatibility mode, or
        // `PGRST_DB_ERRORS_VERBATIM`), and with the status the default
        // configuration gives, which is right unless compatibility mode or
        // `PGRST_DB_ERROR_STATUS` says otherwise. A database error -- the
        // only kind either renders differently -- rides along in the
        // extensions as well, so that `render_verbatim`, which can see the
        // configuration, can render it again where it is. Never sent:
        // extensions are not serialised.
        let authenticated = self.1;
        let kept = match &*self.0 {
            postrust_core::Error::Database(db) => Some(postrust_core::Error::Database(copy_of(db))),
            _ => None,
        };
        // Rendered without a configuration, as the comment above says; the
        // default one is that. `authenticated` is the one thing this takes
        // from the error itself.
        let mut response = postrust_engine::error_response_as(
            &postrust_core::AppConfig::default(),
            *self.0,
            authenticated,
        );
        if let Some(error) = kept {
            // With the status it left with, so that `render_verbatim` can tell
            // it from one a layer of the route's own has changed since.
            let rendered = response.status();
            response
                .extensions_mut()
                .insert(Unrendered(Arc::new(Mutex::new(Some((
                    error,
                    authenticated,
                    rendered,
                ))))));
        }
        response.map(axum::body::Body::from)
    }
}

/// `DatabaseError` is not `Clone`; every field is a string.
fn copy_of(db: &postrust_core::error::DatabaseError) -> postrust_core::error::DatabaseError {
    postrust_core::error::DatabaseError {
        code: db.code.clone(),
        message: db.message.clone(),
        details: db.details.clone(),
        hint: db.hint.clone(),
        constraint: db.constraint.clone(),
        table: db.table.clone(),
        column: db.column.clone(),
    }
}

/// A database error a response was rendered from, sanitised, kept for
/// [`render_verbatim`] with whether the request was authenticated and the
/// status it was rendered with. Only database errors are kept: configuration
/// changes how nothing else is rendered.
#[derive(Clone)]
struct Unrendered(Arc<Mutex<Option<(postrust_core::Error, bool, StatusCode)>>>);

/// Render a route's [`Error`] again as the configuration answers it: with the
/// database's own words where it passes the SQLSTATE through, and with the
/// status it gives that SQLSTATE.
///
/// A layer on each router given to [`Builder::nest`](crate::Builder::nest)
/// when compatibility mode is on, `PGRST_DB_ERRORS_VERBATIM` lists any
/// SQLSTATE or `PGRST_DB_ERROR_STATUS` maps any, and only then. Without
/// either the route's own rendering, which is as the default configuration
/// answers, is already right.
pub(crate) async fn render_verbatim(
    State(state): State<Arc<AppState>>,
    mut response: Response,
) -> Response {
    let Some(Unrendered(error)) = response.extensions_mut().remove::<Unrendered>() else {
        return response;
    };
    let error = error.lock().ok().and_then(|mut error| error.take());
    match error {
        Some((error, authenticated, rendered)) => {
            let (fresh, body) =
                postrust_engine::error_response_as(&state.config, error, authenticated)
                    .into_parts();
            let (mut parts, _) = response.into_parts();
            // The status and headers are kept as they left the route --
            // including any a layer of the route's own added -- unless the
            // configuration gives the error another status than the route
            // rendered it with. Then the status is the configuration's, and
            // the challenge only a 401 carries goes with it.
            if parts.status == rendered && fresh.status != rendered {
                parts.status = fresh.status;
                parts.headers.remove(http::header::WWW_AUTHENTICATE);
                if let Some(challenge) = fresh.headers.get(http::header::WWW_AUTHENTICATE) {
                    parts
                        .headers
                        .insert(http::header::WWW_AUTHENTICATE, challenge.clone());
                }
            }
            Response::from_parts(parts, axum::body::Body::from(body))
        }
        None => response,
    }
}
