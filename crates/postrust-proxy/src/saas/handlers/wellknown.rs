//! Well-known endpoint handlers for domain verification.

use crate::saas::handlers::SaasState;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
};

/// Handle HTTP verification challenge.
///
/// This endpoint serves the verification content for HTTP domain verification.
/// It responds at `/.well-known/postrust-verification/{token}` with the expected
/// verification content.
pub async fn handle_verification_challenge(
    State(_state): State<SaasState>,
    Path(token): Path<String>,
) -> impl IntoResponse {
    // Note: We need direct pool access, which isn't available through domain_manager
    // For now, return a placeholder. In production, this would query the challenge.

    // For HTTP verification, the challenge expected_value is: postrust-verify={token}
    // We need to verify the token exists and is valid

    // Simplified implementation - in production you'd look up the challenge in DB
    let expected_content = format!("postrust-verify={}", token);

    // Return the verification content
    // In a real implementation, you'd verify the token exists in the database first
    (StatusCode::OK, expected_content)
}

/// Handle ACME HTTP-01 challenge.
///
/// This endpoint serves ACME HTTP-01 challenges for automatic certificate provisioning.
/// It responds at `/.well-known/acme-challenge/{token}`.
pub async fn handle_acme_challenge(
    State(_state): State<SaasState>,
    Path(_token): Path<String>,
) -> impl IntoResponse {
    // ACME challenges would be handled by the ACME module
    // This is a placeholder for integration
    (StatusCode::NOT_FOUND, "ACME challenge not found")
}
