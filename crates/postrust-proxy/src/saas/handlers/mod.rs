//! API handlers for SaaS domain management.

pub mod auth;
pub mod domains;
pub mod routes;
pub mod upstreams;
pub mod wellknown;

use crate::admin::api::ApiResponse;
use crate::error::{ProxyError, ProxyResult};
use crate::saas::api_keys::ApiKeyService;
use crate::saas::auth::{AuthContext, SaasAuthLayer};
use crate::saas::manager::DomainManager;
use crate::saas::options::SaasOptions;
use crate::saas::verification::DomainVerificationService;
use crate::tls::CertificateStore;
use axum::{
    http::StatusCode,
    middleware,
    routing::{delete, get, post, put},
    Router,
};
use sqlx::PgPool;
use std::sync::Arc;

/// Shared state for SaaS API handlers.
#[derive(Clone)]
pub struct SaasState {
    /// Domains, their routes and their upstreams.
    pub domain_manager: Arc<DomainManager>,
    /// Creating, listing and revoking API keys.
    pub api_key_service: Arc<ApiKeyService>,
    /// The middleware that turns a credential into an [`AuthContext`].
    ///
    /// [`AuthContext`]: crate::saas::AuthContext
    pub auth_layer: Arc<SaasAuthLayer>,
    /// The pool, for handlers whose concern is not domain management.
    ///
    /// `wellknown` serves ACME challenges out of `proxy_acme_challenges`, which
    /// is a TLS concern that has no business being a method on
    /// `DomainManager`. The original code carried a comment saying it needed
    /// this and did not have it, and answered with a placeholder instead.
    pub pool: PgPool,

    /// Where uploaded and issued certificates are kept.
    pub cert_store: Arc<CertificateStore>,
}

impl SaasState {
    /// Build the state with default [`SaasOptions`] and this JWT secret.
    ///
    /// `cert_dir` is where certificates are cached on disk. A parameter rather
    /// than a default, because a proxy that silently writes private keys to a
    /// directory nobody chose is worse than one that makes you say where.
    ///
    /// The defaults reserve no platform hostnames. A deployment that serves
    /// its own names from the same proxy should use
    /// [`with_options`](Self::with_options) and list them in
    /// [`DomainPolicy::reserved`](crate::saas::DomainPolicy::reserved).
    pub async fn new(
        pool: PgPool,
        jwt_secret: Option<String>,
        cert_dir: impl AsRef<std::path::Path>,
    ) -> ProxyResult<Self> {
        let options = SaasOptions {
            jwt_secret,
            ..SaasOptions::default()
        };
        Self::with_options(pool, options, cert_dir).await
    }

    /// Build the state from explicit options.
    pub async fn with_options(
        pool: PgPool,
        options: SaasOptions,
        cert_dir: impl AsRef<std::path::Path>,
    ) -> ProxyResult<Self> {
        let verification_service = Arc::new(DomainVerificationService::new());
        let domain_manager = Arc::new(DomainManager::with_policy(
            pool.clone(),
            verification_service,
            options.domains,
        ));
        let api_key_service = Arc::new(ApiKeyService::new(pool.clone()));
        let pool_for_handlers = pool.clone();
        let auth_layer = Arc::new(
            SaasAuthLayer::new(pool, options.jwt_secret)
                .with_jwt_audience(options.jwt_audience)
                .with_jwt_issuer(options.jwt_issuer),
        );

        let cert_store =
            Arc::new(CertificateStore::new(pool_for_handlers.clone(), cert_dir).await?);

        Ok(Self {
            domain_manager,
            api_key_service,
            auth_layer,
            pool: pool_for_handlers,
            cert_store,
        })
    }
}

/// Create the SaaS API router.
pub fn saas_router(state: SaasState) -> Router {
    let auth_layer = state.auth_layer.clone();

    // Public routes (no auth required)
    let public_routes = Router::new().route(
        "/.well-known/postrust-verification/{token}",
        get(wellknown::handle_verification_challenge),
    );

    // Public because the CA fetching it has no credentials and cannot be given
    // any. It only ever returns a value we put there ourselves. Gated with the
    // issuer that writes those values.
    #[cfg(feature = "acme")]
    let public_routes = public_routes.route(
        "/.well-known/acme-challenge/{token}",
        get(wellknown::handle_acme_challenge),
    );

    // Protected routes (require authentication)
    let protected_routes = Router::new();

    // Queue certificate issuance -- and requeue it after a failure, which is
    // the same operation. Gated because the worker that acts on it only exists
    // with the `acme` feature: an endpoint that sets a status nothing drains
    // would be worse than no endpoint.
    #[cfg(feature = "acme")]
    let protected_routes =
        protected_routes.route("/domains/{id}/ssl/provision", post(domains::provision_ssl));

    let protected_routes = protected_routes
        // Auth / API Keys
        .route("/auth/api-keys", post(auth::create_api_key))
        .route("/auth/api-keys", get(auth::list_api_keys))
        .route("/auth/api-keys/{id}", delete(auth::revoke_api_key))
        // Tenant
        .route("/tenant/me", get(auth::get_current_tenant))
        .route("/tenant/usage", get(auth::get_tenant_usage))
        // Domains
        .route("/domains", get(domains::list_domains))
        .route("/domains", post(domains::create_domain))
        .route("/domains/{id}", get(domains::get_domain))
        .route("/domains/{id}", delete(domains::delete_domain))
        .route("/domains/{id}/verify", post(domains::verify_domain))
        .route("/domains/{id}/enable", post(domains::enable_domain))
        .route("/domains/{id}", put(domains::update_domain))
        .route("/domains/{id}/disable", post(domains::disable_domain))
        .route(
            "/domains/{id}/ssl/upload",
            post(domains::upload_certificate),
        )
        // Domain Routes
        .route("/domains/{domain_id}/routes", get(routes::list_routes))
        .route("/domains/{domain_id}/routes", post(routes::create_route))
        .route("/domains/{domain_id}/routes/{id}", get(routes::get_route))
        .route(
            "/domains/{domain_id}/routes/{id}",
            put(routes::update_route),
        )
        .route(
            "/domains/{domain_id}/routes/{id}",
            delete(routes::delete_route),
        )
        // Upstreams
        .route("/upstreams", get(upstreams::list_upstreams))
        .route("/upstreams", post(upstreams::create_upstream))
        .route("/upstreams/{id}", get(upstreams::get_upstream))
        .route("/upstreams/{id}", put(upstreams::update_upstream))
        .route("/upstreams/{id}", delete(upstreams::delete_upstream))
        .route("/upstreams/{id}/backends", post(upstreams::add_backend))
        .route(
            "/upstreams/{id}/backends/{backend_id}",
            delete(upstreams::remove_backend),
        )
        .layer(middleware::from_fn_with_state(
            auth_layer,
            crate::saas::auth::auth_middleware,
        ))
        .with_state(state.clone());

    // Combine public and protected routes
    Router::new()
        .merge(public_routes.with_state(state.clone()))
        .nest("/api/v1", protected_routes)
}

/// Turn an error into the response a tenant sees.
///
/// Errors that are about the request -- validation, not found, conflict, quota
/// -- carry a message written for the caller, and it is returned. Everything
/// else is ours: a database error, an I/O error, a TLS library's complaint.
/// Those are logged in full and answered with a fixed message, because their
/// text is written for an operator and says things about the deployment --
/// table and constraint names, file paths, sometimes the offending value --
/// that a tenant has no business reading.
///
/// Every response carries a stable `code` to match on.
pub fn error_response(error: ProxyError) -> (StatusCode, axum::Json<ApiResponse<()>>) {
    let (status, code, message) = match error {
        ProxyError::Validation(message) => (StatusCode::BAD_REQUEST, "validation_failed", message),
        ProxyError::NotFound(message) => (StatusCode::NOT_FOUND, "not_found", message),
        ProxyError::Conflict(message) => (StatusCode::CONFLICT, "conflict", message),
        ProxyError::QuotaExceeded(message) => {
            (StatusCode::PAYMENT_REQUIRED, "quota_exceeded", message)
        }
        ProxyError::Forbidden(message) => (StatusCode::FORBIDDEN, "forbidden", message),
        ProxyError::Auth(message) => (StatusCode::UNAUTHORIZED, "unauthorized", message),
        ProxyError::Verification(message) => {
            (StatusCode::BAD_REQUEST, "verification_failed", message)
        }
        ProxyError::TenantSuspended => (
            StatusCode::FORBIDDEN,
            "tenant_suspended",
            "Tenant account is suspended".to_string(),
        ),
        ProxyError::RateLimitExceeded => (
            StatusCode::TOO_MANY_REQUESTS,
            "rate_limited",
            "Too many attempts; try again later".to_string(),
        ),
        ProxyError::Database(error) => return database_error(error),
        other => {
            tracing::error!(error = %other, "SaaS API request failed");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal_error",
                "Internal server error".to_string(),
            )
        }
    };
    (
        status,
        axum::Json(ApiResponse::<()>::error_with_code(code, message)),
    )
}

/// A database error, as a tenant sees it.
///
/// A constraint violation is the caller's -- two routes on one path, two
/// upstreams of one name, a value out of range -- and gets a 4xx, but still a
/// fixed message: the constraint's name and the duplicated value are in the
/// error text, and the duplicated value may be someone else's. Anything else is
/// a 500.
fn database_error(error: sqlx::Error) -> (StatusCode, axum::Json<ApiResponse<()>>) {
    let kind = error.as_database_error().map(|e| e.kind());
    let (status, code, message) = match kind {
        Some(sqlx::error::ErrorKind::UniqueViolation) => (
            StatusCode::CONFLICT,
            "conflict",
            "A resource with these values already exists",
        ),
        Some(sqlx::error::ErrorKind::ForeignKeyViolation) => (
            StatusCode::CONFLICT,
            "conflict",
            "The request refers to a resource that does not exist or is still in use",
        ),
        Some(sqlx::error::ErrorKind::CheckViolation | sqlx::error::ErrorKind::NotNullViolation) => {
            (
                StatusCode::BAD_REQUEST,
                "validation_failed",
                "A value in the request is not allowed",
            )
        }
        _ => {
            tracing::error!(%error, "SaaS API database error");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal_error",
                "Internal server error",
            )
        }
    };
    if status.is_client_error() {
        tracing::debug!(%error, "SaaS API request refused by a constraint");
    }
    (
        status,
        axum::Json(ApiResponse::<()>::error_with_code(code, message)),
    )
}

/// Refuse a request whose credential lacks `scope`.
///
/// Every handler calls this first, reads included. Reads were unchecked for a
/// long time, and a key created with no scopes at all could list every domain
/// with its verification token.
pub(crate) fn require_scope(
    auth: &AuthContext,
    scope: &str,
) -> Result<(), axum::response::Response> {
    use axum::response::IntoResponse;

    if auth.grants(scope) {
        return Ok(());
    }
    Err((
        StatusCode::FORBIDDEN,
        axum::Json(ApiResponse::<()>::error_with_code(
            "insufficient_scope",
            format!("Insufficient permissions: this requires the {scope} scope"),
        )),
    )
        .into_response())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn render(error: ProxyError) -> (StatusCode, serde_json::Value) {
        let (status, axum::Json(body)) = error_response(error);
        (status, serde_json::to_value(body).unwrap())
    }

    #[test]
    fn errors_of_ours_are_not_described() {
        for error in [
            ProxyError::Database(sqlx::Error::PoolTimedOut),
            ProxyError::Database(sqlx::Error::Protocol(
                "relation \"proxy_domains\" does not exist".into(),
            )),
            ProxyError::Io(std::io::Error::other(
                "/var/lib/postrust/certs: permission denied",
            )),
            ProxyError::Tls("bad key".into()),
            ProxyError::Internal("secret detail".into()),
        ] {
            let (status, body) = render(error);
            assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
            assert_eq!(body["code"], "internal_error");
            assert_eq!(body["error"], "Internal server error");
        }
    }

    #[test]
    fn errors_about_the_request_keep_their_message() {
        let (status, body) = render(ProxyError::Validation("the domain is empty".into()));
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["code"], "validation_failed");
        assert_eq!(body["error"], "the domain is empty");

        let (status, body) = render(ProxyError::RateLimitExceeded);
        assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(body["code"], "rate_limited");
    }
}
