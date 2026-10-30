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
use axum::extract::State;
use axum::response::{IntoResponse, Response};
use http::StatusCode;
use std::sync::{Arc, Mutex};

/// A failure answered as the REST API would answer it.
///
/// Wraps the [`postrust_core::Error`] the REST pipeline reports with, so its
/// status and body come from the same table: a database error by its
/// SQLSTATE (`23505` is 409, `42501` a missing grant is 401, `P0001` a bare
/// `RAISE` is 400), a refusal raised with `sqlstate 'PGRST'` with the status,
/// headers and body the function wrote, a broken connection as 503.
///
/// The database's own account of an error -- its message, detail and hint,
/// which name constraints and columns -- is passed through only where the
/// REST API would pass it through: in compatibility mode (`PGRST_COMPAT_MODE`)
/// or with `PGRST_DEBUG`. Compatibility mode is the server's configuration, which
/// an error converted by `?` has no way to reach, so it is applied on the way
/// out by the routers given to [`Builder::nest`](crate::Builder::nest). A
/// router served some other way gets the sanitised body.
pub struct Error(Box<postrust_core::Error>);

impl Error {
    /// The status this is answered with.
    pub fn status(&self) -> StatusCode {
        self.0.status_code()
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
        Self(Box::new(error))
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

        // Rendered sanitised, which is right unless the server is in
        // compatibility mode. A database error -- the only kind compatibility
        // mode renders differently -- rides along in the extensions as well,
        // so that `render_verbatim`, which can see the configuration, can
        // render it again where it is. Never sent: extensions are not
        // serialised.
        let kept = match &*self.0 {
            postrust_core::Error::Database(db) => Some(Unrendered(Arc::new(Mutex::new(Some(
                postrust_core::Error::Database(copy_of(db)),
            ))))),
            _ => None,
        };
        let mut response = postrust_engine::error_response(*self.0, false);
        if let Some(kept) = kept {
            response.extensions_mut().insert(kept);
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
/// [`render_verbatim`]. Only database errors are kept: compatibility mode
/// changes how nothing else is rendered.
#[derive(Clone)]
struct Unrendered(Arc<Mutex<Option<postrust_core::Error>>>);

/// Render a route's [`Error`] again with the database's own words, in
/// compatibility mode.
///
/// A layer on each router given to [`Builder::nest`](crate::Builder::nest)
/// when compatibility mode is on, and only then.
pub(crate) async fn render_verbatim(
    State(state): State<Arc<AppState>>,
    mut response: Response,
) -> Response {
    let Some(Unrendered(error)) = response.extensions_mut().remove::<Unrendered>() else {
        return response;
    };
    let error = error.lock().ok().and_then(|mut error| error.take());
    match error {
        // Only the body: for a database error that is all compatibility mode
        // changes, and the status and headers are kept as they left the
        // route -- including any a layer of the route's own added.
        Some(error) => {
            let (_, body) =
                postrust_engine::error_response(error, state.config.compat_mode).into_parts();
            let (parts, _) = response.into_parts();
            Response::from_parts(parts, axum::body::Body::from(body))
        }
        None => response,
    }
}
