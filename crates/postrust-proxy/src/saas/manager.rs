//! Domain management service.
//!
//! High-level service for managing custom domains, routes, and upstreams.

use crate::error::{ProxyError, ProxyResult};
use crate::saas::db;
use crate::saas::options::{DomainPolicy, UploadTrust};
use crate::saas::types::*;
use crate::saas::validation;
use crate::saas::verification::DomainVerificationService;
use rand::Rng;
use sqlx::PgPool;
use std::sync::Arc;
use uuid::Uuid;

/// Domain management service.
pub struct DomainManager {
    pool: PgPool,
    verification_service: Arc<DomainVerificationService>,
    policy: DomainPolicy,
}

impl DomainManager {
    /// Create a new domain manager with the default [`DomainPolicy`].
    pub fn new(pool: PgPool, verification_service: Arc<DomainVerificationService>) -> Self {
        Self::with_policy(pool, verification_service, DomainPolicy::default())
    }

    /// Create a domain manager with an explicit policy.
    pub fn with_policy(
        pool: PgPool,
        verification_service: Arc<DomainVerificationService>,
        policy: DomainPolicy,
    ) -> Self {
        Self {
            pool,
            verification_service,
            policy,
        }
    }

    // =========================================================================
    // Domain Management
    // =========================================================================

    /// Create a new domain for a tenant.
    ///
    /// The name is normalised first and stored in that form, so that the
    /// uniqueness check, the Host-header match and the certificate all see one
    /// spelling. A reserved name is refused outright.
    pub async fn create_domain(
        &self,
        tenant_id: Uuid,
        mut req: CreateDomainRequest,
    ) -> ProxyResult<DomainResponse> {
        req.domain = validation::normalize_domain(&req.domain)?;
        if self.policy.reserved.is_reserved(&req.domain) {
            return Err(ProxyError::Validation(format!(
                "{} is reserved and cannot be registered",
                req.domain
            )));
        }

        let verification_token = generate_verification_token();
        let expected_value = format!("postrust-verify={}", verification_token);

        let domain = db::register_domain(
            &self.pool,
            tenant_id,
            &req,
            &verification_token,
            &expected_value,
            self.policy.pending_ttl,
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

    /// Delete a domain, and every certificate linked to it.
    ///
    /// The certificate rows go with the domain row; `cert_store` is told as
    /// well, so its files and its memory cache do not go on holding a private
    /// key for a domain that no longer exists. A certificate is only removed
    /// if it is *linked* to this domain: a row for the same name with no
    /// domain is the operator's, and is left alone.
    pub async fn delete_domain(
        &self,
        id: Uuid,
        tenant_id: Uuid,
        cert_store: &crate::tls::CertificateStore,
    ) -> ProxyResult<bool> {
        let Some(certificates) = db::delete_domain(&self.pool, id, tenant_id).await? else {
            return Ok(false);
        };
        for name in certificates {
            if let Err(error) = cert_store.remove(&name).await {
                // The row is already gone with the domain; what is left is a
                // cached file. Worth a warning, not worth failing a delete
                // that has happened.
                tracing::warn!(domain = %name, %error, "could not clear a cached certificate");
            }
        }
        Ok(true)
    }

    /// Verify a domain.
    ///
    /// Throttled per domain: one attempt per
    /// [`verification_min_interval`](DomainPolicy::verification_min_interval),
    /// and at most
    /// [`verification_max_attempts`](DomainPolicy::verification_max_attempts)
    /// in all. Refused once the challenge or the registration has expired. An
    /// already-verified domain answers `verified` without checking again.
    pub async fn verify_domain(
        &self,
        id: Uuid,
        tenant_id: Uuid,
    ) -> ProxyResult<VerificationResult> {
        let domain = match db::start_verification_attempt(
            &self.pool,
            id,
            tenant_id,
            self.policy.verification_min_interval,
            self.policy.verification_max_attempts,
            self.policy.pending_ttl,
        )
        .await?
        {
            db::AttemptStart::Started(domain) => domain,
            db::AttemptStart::AlreadyVerified => return Ok(VerificationResult::Verified),
            db::AttemptStart::NotFound => {
                return Err(ProxyError::NotFound("Domain not found".into()))
            }
            db::AttemptStart::TooSoon => return Err(ProxyError::RateLimitExceeded),
            db::AttemptStart::Exhausted => {
                return Ok(VerificationResult::Failed {
                    reason: "This domain has used all its verification attempts; \
                             delete it and add it again"
                        .into(),
                })
            }
            db::AttemptStart::Expired => {
                return Ok(VerificationResult::Failed {
                    reason: "The verification challenge has expired; \
                             delete the domain and add it again"
                        .into(),
                })
            }
        };

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
                // A domain asking for ACME is queued, not marked
                // `provisioning`.
                //
                // `pending` is the worker's inbox: `saas::ssl::run` claims
                // these rows, flips them to `provisioning` itself, and records
                // `active` or `failed` with a reason. Setting `provisioning`
                // here would claim work was under way before anything had
                // started, and would take the row out of the worker's own
                // claim query -- so it would sit mid-issuance for ever.
                db::mark_verified(&self.pool, id).await?;
                tracing::info!(domain = %domain.domain, "Domain verified successfully");
            }
            VerificationResult::Failed { reason } => {
                tracing::warn!(domain = %domain.domain, reason = %reason, "Domain verification failed");
            }
            VerificationResult::Pending => {}
        }

        Ok(result)
    }

    /// Look up a live HTTP verification challenge by its token.
    ///
    /// Exposed so the well-known endpoint can answer challenges without
    /// reaching past this service for the pool. The caller must still check
    /// that the request arrived for the returned domain.
    pub async fn find_live_http_challenge(
        &self,
        token: &str,
    ) -> ProxyResult<Option<db::HttpChallenge>> {
        db::find_live_http_challenge(&self.pool, token).await
    }

    /// Apply a partial update to a domain.
    ///
    /// The domain name cannot be changed -- see [`UpdateDomainRequest`] for
    /// why -- so this only moves the verification method and the SSL provider.
    pub async fn update_domain(
        &self,
        id: Uuid,
        tenant_id: Uuid,
        req: UpdateDomainRequest,
    ) -> ProxyResult<Domain> {
        db::update_domain(
            &self.pool,
            id,
            tenant_id,
            req.verification_method.as_ref(),
            req.ssl_provider.as_ref(),
        )
        .await?
        .ok_or_else(|| ProxyError::NotFound("Domain not found".into()))
    }

    /// Store a certificate an operator supplied, after checking it.
    ///
    /// The checks are the point. An unchecked upload gives a listener that
    /// accepts the certificate now and fails every handshake later: the key may
    /// not match the chain, the certificate may be expired, or it may be for
    /// another domain entirely. All three are refused here, with a message
    /// saying which.
    ///
    /// And the domain must be verified. What is stored here is served by the
    /// TLS listener for that name, to everyone; before this check, a tenant
    /// could register any name -- the platform's own included -- and have its
    /// certificate served for it without proving anything. The chain must also
    /// be trusted according to [`DomainPolicy::upload_trust`].
    pub async fn upload_certificate(
        &self,
        id: Uuid,
        tenant_id: Uuid,
        cert_store: &crate::tls::CertificateStore,
        req: UploadCertificateRequest,
    ) -> ProxyResult<Domain> {
        let domain = db::get_domain_for_tenant(&self.pool, id, tenant_id)
            .await?
            .ok_or_else(|| ProxyError::NotFound("Domain not found".into()))?;

        if domain.verification_status != VerificationStatus::Verified {
            return Err(ProxyError::Validation(
                "Domain must be verified before a certificate can be uploaded".into(),
            ));
        }

        let facts = crate::tls::validate_for_domain(
            &domain.domain,
            req.cert_pem.as_bytes(),
            req.key_pem.as_bytes(),
        )?;
        verify_chain(
            &self.policy.upload_trust,
            &domain.domain,
            req.cert_pem.as_bytes(),
        )?;

        cert_store
            .save(crate::tls::Certificate {
                domain: domain.domain.clone(),
                cert_pem: req.cert_pem.into_bytes(),
                key_pem: req.key_pem.into_bytes(),
                expires_at: Some(facts.expires_at),
            })
            .await?;

        // The verified condition is in this statement too: the domain may have
        // been deleted since it was read. If it has, the certificate just
        // saved belongs to nobody, and is unlinked -- nothing would ever
        // remove it, and the TLS listener would serve it. Take it back out.
        let Some(updated) =
            db::record_uploaded_certificate(&self.pool, id, tenant_id, facts.expires_at).await?
        else {
            if let Err(error) = cert_store.remove(&domain.domain).await {
                tracing::error!(
                    domain = %domain.domain,
                    %error,
                    "could not remove a certificate uploaded for a domain deleted mid-upload"
                );
            }
            return Err(ProxyError::NotFound("Domain not found".into()));
        };

        tracing::info!(
            domain = %updated.domain,
            expires_at = %facts.expires_at,
            "stored an uploaded certificate"
        );
        Ok(updated)
    }

    /// Queue a verified ACME domain for certificate issuance.
    ///
    /// Sets state and returns; the work is done by the issuance worker. That is
    /// the whole endpoint, and deliberately so: an ACME order is several round
    /// trips to the CA plus a challenge the CA has to fetch back from us, and
    /// retrying under a rate limit is the normal failure mode. Doing it inline
    /// would hold the request open and let a client timeout orphan an order the
    /// CA had already started.
    ///
    /// Idempotent, and also the way to retry after a failure -- it clears the
    /// attempt count and the backoff.
    pub async fn queue_ssl_issuance(&self, id: Uuid, tenant_id: Uuid) -> ProxyResult<Domain> {
        db::queue_for_issuance(&self.pool, id, tenant_id)
            .await?
            .ok_or_else(|| {
                ProxyError::NotFound(
                    "No verified domain with ssl_provider = acme for this tenant".into(),
                )
            })
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

        db::enable_domain(&self.pool, id, tenant_id).await
    }

    /// Disable a domain.
    pub async fn disable_domain(&self, id: Uuid, tenant_id: Uuid) -> ProxyResult<bool> {
        db::disable_domain(&self.pool, id, tenant_id).await
    }

    // =========================================================================
    // Route Management
    // =========================================================================

    /// Create a route for a verified domain, within the per-domain quota.
    pub async fn create_route(
        &self,
        domain_id: Uuid,
        tenant_id: Uuid,
        req: CreateDomainRouteRequest,
    ) -> ProxyResult<DomainRoute> {
        validation::validate_resource_name("Route", &req.name)?;
        validation::validate_route_match(
            &req.path_pattern,
            &req.path_type,
            req.methods.as_deref(),
        )?;
        validation::validate_add_headers(&req.add_headers)?;
        validation::validate_remove_headers(&req.remove_headers)?;

        // Ownership of the domain and the upstream, verification and the quota
        // are all checked inside the insert's transaction.
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
        if let Some(name) = &req.name {
            validation::validate_resource_name("Route", name)?;
        }
        if req.path_pattern.is_some() || req.path_type.is_some() || req.methods.is_some() {
            // The pattern is checked against the type it will have after the
            // update, which may be the stored one.
            let current = db::get_route_for_tenant(&self.pool, id, tenant_id)
                .await?
                .ok_or_else(|| ProxyError::NotFound("Route not found".into()))?;
            validation::validate_route_match(
                req.path_pattern.as_deref().unwrap_or(&current.path_pattern),
                req.path_type.as_ref().unwrap_or(&current.path_type),
                req.methods.as_deref(),
            )?;
        }
        if let Some(headers) = &req.add_headers {
            validation::validate_add_headers(headers)?;
        }
        if let Some(headers) = &req.remove_headers {
            validation::validate_remove_headers(headers)?;
        }

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

    /// Create an upstream for a tenant, with any backends given inline.
    ///
    /// All or nothing: either the upstream and every backend are stored, or
    /// none of them is.
    pub async fn create_upstream(
        &self,
        tenant_id: Uuid,
        mut req: CreateUpstreamRequest,
    ) -> ProxyResult<DomainUpstream> {
        validation::validate_resource_name("Upstream", &req.name)?;
        validation::validate_health_check_path(&req.health_check_path)?;
        for backend in &mut req.backends {
            check_backend(backend)?;
        }
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
        if let Some(name) = &req.name {
            validation::validate_resource_name("Upstream", name)?;
        }
        if let Some(path) = &req.health_check_path {
            validation::validate_health_check_path(path)?;
        }
        db::update_upstream(&self.pool, id, tenant_id, req).await
    }

    /// Delete an upstream.
    pub async fn delete_upstream(&self, id: Uuid, tenant_id: Uuid) -> ProxyResult<bool> {
        db::delete_upstream(&self.pool, id, tenant_id).await
    }

    /// Add a backend to an upstream, within the per-upstream quota.
    pub async fn add_backend(
        &self,
        upstream_id: Uuid,
        tenant_id: Uuid,
        mut req: CreateBackendRequest,
    ) -> ProxyResult<DomainBackend> {
        check_backend(&mut req)?;
        db::create_backend(&self.pool, upstream_id, tenant_id, req).await
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
}

/// Check a backend and put its address in canonical form.
///
/// Nothing reads `proxy_domain_backends` yet, but something will, and an
/// address stored now is one connected to then. See
/// [`validate_backend_address`](validation::validate_backend_address) for what
/// is refused, and for the check the eventual consumer still has to make.
fn check_backend(backend: &mut CreateBackendRequest) -> ProxyResult<()> {
    validation::validate_scheme(&backend.scheme)?;
    backend.address = validation::validate_backend_address(&backend.address)?;
    if !(1..=10_000).contains(&backend.weight) {
        return Err(ProxyError::Validation(
            "backend weight must be from 1 to 10000".into(),
        ));
    }
    Ok(())
}

/// Check that an uploaded chain leads to a trusted root, for `domain`.
///
/// The leaf's name, expiry and key have already been checked by
/// `tls::validate_for_domain`; this adds the one thing it does not -- whether
/// anyone but the uploader vouches for the certificate. The verifier is
/// rustls's own, the one a client would run, so a chain passes here exactly
/// when a client with these roots would accept it.
fn verify_chain(trust: &UploadTrust, domain: &str, cert_pem: &[u8]) -> ProxyResult<()> {
    use rustls::client::danger::ServerCertVerifier;
    use rustls::pki_types::{ServerName, UnixTime};

    let roots = match trust {
        UploadTrust::Unchecked => return Ok(()),
        UploadTrust::PublicRoots => Arc::new(rustls::RootCertStore {
            roots: webpki_roots::TLS_SERVER_ROOTS.to_vec(),
        }),
        UploadTrust::Roots(roots) => roots.clone(),
    };

    let chain: Vec<_> = rustls_pemfile::certs(&mut &cert_pem[..])
        .collect::<Result<_, _>>()
        .map_err(|_| ProxyError::Validation("could not parse the certificate chain".into()))?;
    let (leaf, intermediates) = chain
        .split_first()
        .ok_or_else(|| ProxyError::Validation("the certificate chain is empty".into()))?;
    let name = ServerName::try_from(domain.to_string())
        .map_err(|_| ProxyError::Validation("the domain is not a valid server name".into()))?;

    let verifier = rustls::client::WebPkiServerVerifier::builder_with_provider(
        roots,
        Arc::new(rustls::crypto::aws_lc_rs::default_provider()),
    )
    .build()
    .map_err(|e| ProxyError::Tls(format!("could not build a certificate verifier: {e}")))?;

    verifier
        .verify_server_cert(leaf, intermediates, &name, &[], UnixTime::now())
        .map_err(|error| {
            tracing::info!(%domain, %error, "refused an uploaded certificate chain");
            ProxyError::Validation(
                "the certificate chain is not trusted: it must be issued by a trusted \
                 certificate authority and include any intermediate certificates"
                    .into(),
            )
        })?;
    Ok(())
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

    // Domain-name rules are tested in `validation`, and the manager's use of
    // them through the router in tests/control_plane.rs.

    #[test]
    fn test_generate_verification_token() {
        let token = generate_verification_token();
        assert_eq!(token.len(), 32);
        assert!(token.chars().all(|c| c.is_alphanumeric()));
    }
}
