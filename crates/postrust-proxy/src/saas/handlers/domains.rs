//! Domain API handlers.

use crate::admin::api::ApiResponse;
use crate::saas::auth::Auth;
use crate::saas::handlers::{error_response, SaasState};
use crate::saas::types::{CreateDomainRequest, UpdateDomainRequest, UploadCertificateRequest};
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use uuid::Uuid;

/// List all domains for the authenticated tenant.
pub async fn list_domains(State(state): State<SaasState>, Auth(auth): Auth) -> impl IntoResponse {
    match state.domain_manager.list_domains(auth.tenant_id).await {
        Ok(domains) => Json(ApiResponse::success(domains)).into_response(),
        Err(e) => error_response(e).into_response(),
    }
}

/// Create a new domain.
pub async fn create_domain(
    State(state): State<SaasState>,
    Auth(auth): Auth,
    Json(req): Json<CreateDomainRequest>,
) -> impl IntoResponse {
    if !auth.can_write("domains") {
        return (
            StatusCode::FORBIDDEN,
            Json(ApiResponse::<()>::error("Insufficient permissions")),
        )
            .into_response();
    }

    match state
        .domain_manager
        .create_domain(auth.tenant_id, req)
        .await
    {
        Ok(domain_response) => (
            StatusCode::CREATED,
            Json(ApiResponse::success(domain_response)),
        )
            .into_response(),
        Err(e) => error_response(e).into_response(),
    }
}

/// Get a domain by ID.
pub async fn get_domain(
    State(state): State<SaasState>,
    Auth(auth): Auth,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    match state.domain_manager.get_domain(id, auth.tenant_id).await {
        Ok(Some(domain)) => Json(ApiResponse::success(domain)).into_response(),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(ApiResponse::<()>::error("Domain not found")),
        )
            .into_response(),
        Err(e) => error_response(e).into_response(),
    }
}

/// Delete a domain.
pub async fn delete_domain(
    State(state): State<SaasState>,
    Auth(auth): Auth,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    if !auth.can_write("domains") {
        return (
            StatusCode::FORBIDDEN,
            Json(ApiResponse::<()>::error("Insufficient permissions")),
        )
            .into_response();
    }

    match state.domain_manager.delete_domain(id, auth.tenant_id).await {
        Ok(true) => Json(ApiResponse::success(())).into_response(),
        Ok(false) => (
            StatusCode::NOT_FOUND,
            Json(ApiResponse::<()>::error("Domain not found")),
        )
            .into_response(),
        Err(e) => error_response(e).into_response(),
    }
}

/// Trigger domain verification.
pub async fn verify_domain(
    State(state): State<SaasState>,
    Auth(auth): Auth,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    if !auth.can_write("domains") {
        return (
            StatusCode::FORBIDDEN,
            Json(ApiResponse::<()>::error("Insufficient permissions")),
        )
            .into_response();
    }

    match state.domain_manager.verify_domain(id, auth.tenant_id).await {
        Ok(result) => Json(ApiResponse::success(result)).into_response(),
        Err(e) => error_response(e).into_response(),
    }
}

/// Enable a verified domain.
pub async fn enable_domain(
    State(state): State<SaasState>,
    Auth(auth): Auth,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    if !auth.can_write("domains") {
        return (
            StatusCode::FORBIDDEN,
            Json(ApiResponse::<()>::error("Insufficient permissions")),
        )
            .into_response();
    }

    match state.domain_manager.enable_domain(id, auth.tenant_id).await {
        Ok(true) => Json(ApiResponse::success(())).into_response(),
        Ok(false) => (
            StatusCode::BAD_REQUEST,
            Json(ApiResponse::<()>::error(
                "Could not enable domain. Is it verified?",
            )),
        )
            .into_response(),
        Err(e) => error_response(e).into_response(),
    }
}

/// Disable a domain.
pub async fn disable_domain(
    State(state): State<SaasState>,
    Auth(auth): Auth,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    if !auth.can_write("domains") {
        return (
            StatusCode::FORBIDDEN,
            Json(ApiResponse::<()>::error("Insufficient permissions")),
        )
            .into_response();
    }

    match state
        .domain_manager
        .disable_domain(id, auth.tenant_id)
        .await
    {
        Ok(true) => Json(ApiResponse::success(())).into_response(),
        Ok(false) => (
            StatusCode::NOT_FOUND,
            Json(ApiResponse::<()>::error("Domain not found")),
        )
            .into_response(),
        Err(e) => error_response(e).into_response(),
    }
}

/// Change a domain's verification method or SSL provider.
///
/// `PUT /domains/{id}`. Partial: absent fields are left alone.
///
/// The domain name is not updatable. It is the identity of the record and what
/// the verification token proves control of, so a rename would carry a proof of
/// ownership over to a name nobody has proved anything about. Delete and re-add.
pub async fn update_domain(
    State(state): State<SaasState>,
    Auth(auth): Auth,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateDomainRequest>,
) -> impl IntoResponse {
    if !auth.can_write("domains") {
        return (
            StatusCode::FORBIDDEN,
            Json(ApiResponse::<()>::error("Insufficient permissions")),
        )
            .into_response();
    }

    match state
        .domain_manager
        .update_domain(id, auth.tenant_id, req)
        .await
    {
        Ok(domain) => Json(ApiResponse::success(domain)).into_response(),
        Err(e) => error_response(e).into_response(),
    }
}

/// Store a certificate the tenant supplies.
///
/// `POST /domains/{id}/ssl/upload`. For a domain whose certificate comes from
/// somewhere other than ACME -- an internal CA, or one issued by hand.
///
/// The certificate is checked before it is stored: the key must match the
/// chain, it must not be expired, and it must cover this domain. Skipping any of
/// those produces a listener that accepts the upload and then fails every
/// handshake, long after whoever could fix it has stopped watching. A rejection
/// says which check failed.
///
/// Sets the provider to `manual`, and turns `auto_renew` off on the stored
/// certificate: nothing here can renew a certificate it did not obtain, so the
/// renewal scan must not queue it for an ACME worker that has no authorization
/// for the domain.
pub async fn upload_certificate(
    State(state): State<SaasState>,
    Auth(auth): Auth,
    Path(id): Path<Uuid>,
    Json(req): Json<UploadCertificateRequest>,
) -> impl IntoResponse {
    if !auth.can_write("domains") {
        return (
            StatusCode::FORBIDDEN,
            Json(ApiResponse::<()>::error("Insufficient permissions")),
        )
            .into_response();
    }

    match state
        .domain_manager
        .upload_certificate(id, auth.tenant_id, &state.cert_store, req)
        .await
    {
        Ok(domain) => Json(ApiResponse::success(domain)).into_response(),
        Err(e) => error_response(e).into_response(),
    }
}

/// Queue a verified ACME domain for certificate issuance.
///
/// `POST /domains/{id}/ssl/provision`. Returns **202 Accepted**: it sets state
/// and returns, and the issuance worker does the work. Poll `GET /domains/{id}`
/// and watch `ssl_status` go `pending` -> `provisioning` -> `active`, or
/// `failed` with `ssl_error` saying why.
///
/// It does not issue inline, and that is the design rather than a shortcut. An
/// ACME order is several round trips to the CA plus a challenge the CA has to
/// fetch back from us; held open in a request, a slow order blocks the caller
/// and a client timeout orphans an order the CA has already started. Retrying
/// under a rate limit is the normal failure mode, which wants a worker with
/// backoff, not a caller with a retry button.
///
/// Idempotent, and this is also how to retry after a failure: it clears the
/// attempt count, the recorded error and the backoff, so the next pass picks the
/// domain up immediately rather than waiting out a wait computed for a cause
/// that has just been fixed.
#[cfg(feature = "acme")]
pub async fn provision_ssl(
    State(state): State<SaasState>,
    Auth(auth): Auth,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    if !auth.can_write("domains") {
        return (
            StatusCode::FORBIDDEN,
            Json(ApiResponse::<()>::error("Insufficient permissions")),
        )
            .into_response();
    }

    match state
        .domain_manager
        .queue_ssl_issuance(id, auth.tenant_id)
        .await
    {
        Ok(domain) => (StatusCode::ACCEPTED, Json(ApiResponse::success(domain))).into_response(),
        Err(e) => error_response(e).into_response(),
    }
}
