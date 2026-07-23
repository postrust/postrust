//! Domain management service.
//!
//! High-level service for managing custom domains, routes, and upstreams.

use crate::error::{ProxyError, ProxyResult};
use crate::saas::db;
use crate::saas::types::*;
use crate::saas::verification::DomainVerificationService;
use rand::Rng;
use sqlx::PgPool;
use std::sync::Arc;
use uuid::Uuid;

/// Domain management service.
pub struct DomainManager {
    pool: PgPool,
    verification_service: Arc<DomainVerificationService>,
}

impl DomainManager {
    /// Create a new domain manager.
    pub fn new(pool: PgPool, verification_service: Arc<DomainVerificationService>) -> Self {
        Self {
            pool,
            verification_service,
        }
    }

    // =========================================================================
    // Domain Management
    // =========================================================================

    /// Create a new domain for a tenant.
    pub async fn create_domain(
        &self,
        tenant_id: Uuid,
        req: CreateDomainRequest,
    ) -> ProxyResult<DomainResponse> {
        // Validate domain format
        self.validate_domain_format(&req.domain)?;

        // Check tenant quota
        let (current, max) = db::check_domain_quota(&self.pool, tenant_id).await?;
        if current >= max as i64 {
            return Err(ProxyError::QuotaExceeded(format!(
                "Domain limit reached ({}/{})",
                current, max
            )));
        }

        // Check if domain already exists
        if db::domain_exists(&self.pool, &req.domain).await? {
            return Err(ProxyError::Conflict("Domain already registered".into()));
        }

        // Generate verification token
        let verification_token = generate_verification_token();

        // Create domain
        let domain =
            db::create_domain(&self.pool, tenant_id, req.clone(), &verification_token).await?;

        // Create verification challenge
        let expected_value = format!("postrust-verify={}", verification_token);
        let challenge_type = match req.verification_method {
            VerificationMethod::Dns => "dns",
            VerificationMethod::Http => "http",
        };
        db::create_verification_challenge(
            &self.pool,
            domain.id,
            challenge_type,
            &verification_token,
            &expected_value,
        )
        .await?;

        // Generate verification instructions
        let instructions = match domain.verification_method {
            VerificationMethod::Dns => {
                VerificationInstructions::dns(&domain.domain, &verification_token)
            }
            VerificationMethod::Http => {
                VerificationInstructions::http(&domain.domain, &verification_token)
            }
        };

        Ok(DomainResponse {
            domain,
            verification_instructions: instructions,
        })
    }

    /// Get a domain by ID for a tenant.
    pub async fn get_domain(
        &self,
        id: Uuid,
        tenant_id: Uuid,
    ) -> ProxyResult<Option<DomainResponse>> {
        let domain = db::get_domain_for_tenant(&self.pool, id, tenant_id).await?;

        Ok(domain.map(|d| {
            let instructions = match d.verification_method {
                VerificationMethod::Dns => {
                    VerificationInstructions::dns(&d.domain, &d.verification_token)
                }
                VerificationMethod::Http => {
                    VerificationInstructions::http(&d.domain, &d.verification_token)
                }
            };
            DomainResponse {
                domain: d,
                verification_instructions: instructions,
            }
        }))
    }

    /// List all domains for a tenant.
    pub async fn list_domains(&self, tenant_id: Uuid) -> ProxyResult<Vec<Domain>> {
        db::list_domains(&self.pool, tenant_id).await
    }

    /// Delete a domain.
    pub async fn delete_domain(&self, id: Uuid, tenant_id: Uuid) -> ProxyResult<bool> {
        db::delete_domain(&self.pool, id, tenant_id).await
    }

    /// Verify a domain.
    pub async fn verify_domain(
        &self,
        id: Uuid,
        tenant_id: Uuid,
    ) -> ProxyResult<VerificationResult> {
        let domain = db::get_domain_for_tenant(&self.pool, id, tenant_id)
            .await?
            .ok_or_else(|| ProxyError::NotFound("Domain not found".into()))?;

        // Record verification attempt
        db::record_verification_attempt(&self.pool, id).await?;

        // Perform verification based on method
        let result = match domain.verification_method {
            VerificationMethod::Dns => {
                self.verification_service
                    .verify_dns(&domain.domain, &domain.verification_token)
                    .await
            }
            VerificationMethod::Http => {
                self.verification_service
                    .verify_http(&domain.domain, &domain.verification_token)
                    .await
            }
        };

        match &result {
            VerificationResult::Verified => {
                // Update domain status
                db::update_verification_status(&self.pool, id, VerificationStatus::Verified)
                    .await?;

                // If ACME is enabled, trigger SSL provisioning
                if domain.ssl_provider == SslProvider::Acme {
                    // TODO: Trigger ACME certificate provisioning
                    db::update_ssl_status(&self.pool, id, SslStatus::Provisioning, None).await?;
                }

                tracing::info!(domain = %domain.domain, "Domain verified successfully");
            }
            VerificationResult::Failed { reason } => {
                tracing::warn!(domain = %domain.domain, reason = %reason, "Domain verification failed");
            }
            VerificationResult::Pending => {}
        }

        Ok(result)
    }

    /// Enable a verified domain.
    pub async fn enable_domain(&self, id: Uuid, tenant_id: Uuid) -> ProxyResult<bool> {
        // First check if domain is verified
        let domain = db::get_domain_for_tenant(&self.pool, id, tenant_id)
            .await?
            .ok_or_else(|| ProxyError::NotFound("Domain not found".into()))?;

        if domain.verification_status != VerificationStatus::Verified {
            return Err(ProxyError::Validation(
                "Domain must be verified before enabling".into(),
            ));
        }

        db::enable_domain(&self.pool, id).await
    }

    /// Disable a domain.
    pub async fn disable_domain(&self, id: Uuid, tenant_id: Uuid) -> ProxyResult<bool> {
        // Verify ownership
        db::get_domain_for_tenant(&self.pool, id, tenant_id)
            .await?
            .ok_or_else(|| ProxyError::NotFound("Domain not found".into()))?;

        db::disable_domain(&self.pool, id).await
    }

    // =========================================================================
    // Route Management
    // =========================================================================

    /// Create a route for a domain.
    pub async fn create_route(
        &self,
        domain_id: Uuid,
        tenant_id: Uuid,
        req: CreateDomainRouteRequest,
    ) -> ProxyResult<DomainRoute> {
        // Verify domain belongs to tenant
        let _domain = db::get_domain_for_tenant(&self.pool, domain_id, tenant_id)
            .await?
            .ok_or_else(|| ProxyError::NotFound("Domain not found".into()))?;

        // Verify upstream belongs to tenant
        db::get_upstream_for_tenant(&self.pool, req.upstream_id, tenant_id)
            .await?
            .ok_or_else(|| ProxyError::NotFound("Upstream not found".into()))?;

        db::create_route(&self.pool, domain_id, tenant_id, req).await
    }

    /// Get a route by ID.
    pub async fn get_route(&self, id: Uuid, tenant_id: Uuid) -> ProxyResult<Option<DomainRoute>> {
        db::get_route_for_tenant(&self.pool, id, tenant_id).await
    }

    /// List routes for a domain.
    pub async fn list_routes_for_domain(
        &self,
        domain_id: Uuid,
        tenant_id: Uuid,
    ) -> ProxyResult<Vec<DomainRoute>> {
        // Verify domain belongs to tenant
        db::get_domain_for_tenant(&self.pool, domain_id, tenant_id)
            .await?
            .ok_or_else(|| ProxyError::NotFound("Domain not found".into()))?;

        db::list_routes_for_domain(&self.pool, domain_id, tenant_id).await
    }

    /// Update a route.
    pub async fn update_route(
        &self,
        id: Uuid,
        tenant_id: Uuid,
        req: UpdateDomainRouteRequest,
    ) -> ProxyResult<Option<DomainRoute>> {
        // If upstream_id is being updated, verify it belongs to tenant
        if let Some(upstream_id) = req.upstream_id {
            db::get_upstream_for_tenant(&self.pool, upstream_id, tenant_id)
                .await?
                .ok_or_else(|| ProxyError::NotFound("Upstream not found".into()))?;
        }

        db::update_route(&self.pool, id, tenant_id, req).await
    }

    /// Delete a route.
    pub async fn delete_route(&self, id: Uuid, tenant_id: Uuid) -> ProxyResult<bool> {
        db::delete_route(&self.pool, id, tenant_id).await
    }

    // =========================================================================
    // Upstream Management
    // =========================================================================

    /// Create an upstream for a tenant.
    pub async fn create_upstream(
        &self,
        tenant_id: Uuid,
        req: CreateUpstreamRequest,
    ) -> ProxyResult<DomainUpstream> {
        db::create_upstream(&self.pool, tenant_id, req).await
    }

    /// Get an upstream by ID.
    pub async fn get_upstream(
        &self,
        id: Uuid,
        tenant_id: Uuid,
    ) -> ProxyResult<Option<DomainUpstream>> {
        db::get_upstream_for_tenant(&self.pool, id, tenant_id).await
    }

    /// List upstreams for a tenant.
    pub async fn list_upstreams(&self, tenant_id: Uuid) -> ProxyResult<Vec<DomainUpstream>> {
        db::list_upstreams(&self.pool, tenant_id).await
    }

    /// Update an upstream.
    pub async fn update_upstream(
        &self,
        id: Uuid,
        tenant_id: Uuid,
        req: UpdateUpstreamRequest,
    ) -> ProxyResult<Option<DomainUpstream>> {
        db::update_upstream(&self.pool, id, tenant_id, req).await
    }

    /// Delete an upstream.
    pub async fn delete_upstream(&self, id: Uuid, tenant_id: Uuid) -> ProxyResult<bool> {
        db::delete_upstream(&self.pool, id, tenant_id).await
    }

    /// Add a backend to an upstream.
    pub async fn add_backend(
        &self,
        upstream_id: Uuid,
        tenant_id: Uuid,
        req: CreateBackendRequest,
    ) -> ProxyResult<DomainBackend> {
        // Verify upstream belongs to tenant
        db::get_upstream_for_tenant(&self.pool, upstream_id, tenant_id)
            .await?
            .ok_or_else(|| ProxyError::NotFound("Upstream not found".into()))?;

        db::create_backend(&self.pool, upstream_id, req).await
    }

    /// Remove a backend from an upstream.
    pub async fn remove_backend(
        &self,
        backend_id: Uuid,
        upstream_id: Uuid,
        tenant_id: Uuid,
    ) -> ProxyResult<bool> {
        db::delete_backend(&self.pool, backend_id, upstream_id, tenant_id).await
    }

    // =========================================================================
    // Tenant Management
    // =========================================================================

    /// Get tenant usage statistics.
    pub async fn get_tenant_usage(&self, tenant_id: Uuid) -> ProxyResult<TenantUsage> {
        db::get_tenant_usage(&self.pool, tenant_id).await
    }

    // =========================================================================
    // Validation Helpers
    // =========================================================================

    /// Validate domain format.
    fn validate_domain_format(&self, domain: &str) -> ProxyResult<()> {
        // Check length
        if domain.is_empty() || domain.len() > 253 {
            return Err(ProxyError::Validation("Invalid domain length".into()));
        }

        // Must have at least one dot
        if !domain.contains('.') {
            return Err(ProxyError::Validation(
                "Domain must have at least one dot".into(),
            ));
        }

        // Check each label
        for label in domain.split('.') {
            if label.is_empty() || label.len() > 63 {
                return Err(ProxyError::Validation("Invalid domain label length".into()));
            }

            // Check first and last characters
            let chars: Vec<char> = label.chars().collect();
            if chars.first().is_none_or(|c| !c.is_alphanumeric())
                || chars.last().is_none_or(|c| !c.is_alphanumeric())
            {
                return Err(ProxyError::Validation(
                    "Domain labels must start and end with alphanumeric characters".into(),
                ));
            }

            // Check all characters
            for c in label.chars() {
                if !c.is_alphanumeric() && c != '-' {
                    return Err(ProxyError::Validation(
                        "Domain labels can only contain alphanumeric characters and hyphens".into(),
                    ));
                }
            }
        }

        Ok(())
    }
}

/// Generate a secure verification token.
fn generate_verification_token() -> String {
    const CHARSET: &[u8] = b"abcdefghijklmnopqrstuvwxyz0123456789";
    let mut rng = rand::rng();

    (0..32)
        .map(|_| {
            let idx = rng.random_range(0..CHARSET.len());
            CHARSET[idx] as char
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    // Note: `validate_domain_format` is exercised via integration tests that
    // build a real `DomainManager` (it needs a live pool). A previous unit test
    // here constructed the manager from `std::mem::zeroed()`, which is undefined
    // behavior and crashed on drop, so it was removed.

    #[test]
    fn test_generate_verification_token() {
        let token = generate_verification_token();
        assert_eq!(token.len(), 32);
        assert!(token.chars().all(|c| c.is_alphanumeric()));
    }
}
