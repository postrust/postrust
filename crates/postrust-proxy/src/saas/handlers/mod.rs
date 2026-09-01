//! API handlers for SaaS domain management.

pub mod auth;
pub mod domains;
pub mod routes;
pub mod upstreams;
pub mod wellknown;

use crate::admin::api::ApiResponse;
use crate::error::ProxyError;
use crate::saas::api_keys::ApiKeyService;
use crate::saas::auth::SaasAuthLayer;
use crate::saas::manager::DomainManager;
use crate::saas::verification::DomainVerificationService;
use axum::{
    middleware,
    routing::{delete, get, post, put},
    Router,
};
use sqlx::PgPool;
use std::sync::Arc;

/// Shared state for SaaS API handlers.
#[derive(Clone)]
pub struct SaasState {
    pub domain_manager: Arc<DomainManager>,
    pub api_key_service: Arc<ApiKeyService>,
    pub auth_layer: Arc<SaasAuthLayer>,
    /// The pool, for handlers whose concern is not domain management.
    ///
    /// `wellknown` serves ACME challenges out of `proxy_acme_challenges`, which
    /// is a TLS concern that has no business being a method on
    /// `DomainManager`. The original code carried a comment saying it needed
    /// this and did not have it, and answered with a placeholder instead.
    pub pool: PgPool,
}

impl SaasState {
    /// Create new SaaS state.
    pub fn new(pool: PgPool, jwt_secret: Option<String>) -> Self {
        let verification_service = Arc::new(DomainVerificationService::new());
        let domain_manager = Arc::new(DomainManager::new(pool.clone(), verification_service));
        let api_key_service = Arc::new(ApiKeyService::new(pool.clone()));
        let pool_for_handlers = pool.clone();
        let auth_layer = Arc::new(SaasAuthLayer::new(pool, jwt_secret));

        Self {
            domain_manager,
            api_key_service,
            auth_layer,
            pool: pool_for_handlers,
        }
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

    // Requeue a domain whose issuance failed. Gated because the worker that
    // would act on it only exists with the `acme` feature -- an endpoint that
    // sets a status nothing drains would be worse than no endpoint.
    #[cfg(feature = "acme")]
    let protected_routes =
        protected_routes.route("/domains/{id}/ssl/retry", post(domains::retry_ssl));

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
        .route("/domains/{id}/disable", post(domains::disable_domain))
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

/// Helper to convert ProxyError to API response.
pub fn error_response(error: ProxyError) -> (axum::http::StatusCode, axum::Json<ApiResponse<()>>) {
    let status = error.status_code();
    let response = ApiResponse::<()>::error(error.to_string());
    (status, axum::Json(response))
}
