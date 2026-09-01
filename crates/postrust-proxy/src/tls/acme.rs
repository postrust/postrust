//! ACME certificate issuance over HTTP-01.
//!
//! One certificate at a time, for one domain, on demand. That shape is what the
//! multi-tenant case needs: domains arrive and leave while the proxy runs, so
//! the set cannot be known at startup.
//!
//! This replaced a `rustls-acme` wrapper that took a fixed domain list and
//! handed back a `ServerConfig` whose resolver answered challenges inside the
//! TLS handshake. That model cannot serve a domain it was not started with, and
//! the wrapper was never constructed anywhere — so rather than keep two ACME
//! clients in one crate (which also resolved two copies of `rcgen`), it is
//! gone.
//!
//! # The flow
//!
//! 1. Load the account from `proxy_acme_accounts`, or register one and store it.
//! 2. Place an order for the domain.
//! 3. For each authorization, take the `http-01` challenge and write its token
//!    and key authorization to `proxy_acme_challenges`, where
//!    [`crate::saas::handlers::wellknown::handle_acme_challenge`] can find them.
//! 4. Tell the CA the challenge is ready, and poll until the order is.
//! 5. Finalize — which builds the CSR and returns a fresh private key — then
//!    poll for the certificate chain.
//! 6. Store both in the certificate store and delete the challenge rows.
//!
//! # Why HTTP-01 and not DNS-01
//!
//! HTTP-01 needs only that the domain resolves to this proxy, which is already
//! true for any domain it is serving. DNS-01 would need write access to each
//! tenant's zone, which a proxy has no way to obtain. The trade is that HTTP-01
//! cannot issue wildcards.

use std::sync::Arc;
use std::time::Duration;

use instant_acme::{
    Account, AccountCredentials, AuthorizationStatus, ChallengeType, Identifier, NewAccount,
    NewOrder, RetryPolicy,
};
use sqlx::{PgPool, Row};

use crate::error::{ProxyError, ProxyResult};
use crate::tls::cert_store::{Certificate, CertificateStore};

/// Let's Encrypt's production directory.
pub const LETS_ENCRYPT_PRODUCTION: &str = "https://acme-v02.api.letsencrypt.org/directory";
/// Let's Encrypt's staging directory, which issues untrusted certificates
/// against far looser rate limits.
pub const LETS_ENCRYPT_STAGING: &str = "https://acme-staging-v02.api.letsencrypt.org/directory";

/// How long to keep polling one order before giving up.
///
/// Pebble, and Let's Encrypt under load, can leave an order pending for tens of
/// seconds. The worker retries with backoff, so a timeout here is a delay
/// rather than a failure.
const ORDER_TIMEOUT: Duration = Duration::from_secs(60);

/// Issues certificates for individual domains over ACME HTTP-01.
pub struct AcmeIssuer {
    directory_url: String,
    contact: Option<String>,
    /// A PEM root to trust for the directory itself.
    ///
    /// Only for testing against a CA with a private root -- Pebble serves its
    /// directory over a self-signed chain. `None` uses the platform roots.
    root_pem_path: Option<std::path::PathBuf>,
    pool: PgPool,
    cert_store: Arc<CertificateStore>,
}

impl AcmeIssuer {
    /// Create an issuer against a directory URL.
    pub fn new(
        directory_url: impl Into<String>,
        contact: Option<String>,
        pool: PgPool,
        cert_store: Arc<CertificateStore>,
    ) -> Self {
        Self {
            directory_url: directory_url.into(),
            contact,
            root_pem_path: None,
            pool,
            cert_store,
        }
    }

    /// Trust a private root for the ACME directory's own TLS.
    ///
    /// For testing against Pebble, which serves its directory with a
    /// self-signed chain. Never needed against a public CA.
    pub fn with_root_certificate(mut self, pem_path: impl Into<std::path::PathBuf>) -> Self {
        self.root_pem_path = Some(pem_path.into());
        self
    }

    /// The directory this issuer talks to.
    pub fn directory_url(&self) -> &str {
        &self.directory_url
    }

    /// Obtain a certificate for `domain` and store it.
    ///
    /// Returns the stored certificate. Any challenge rows this created are
    /// removed before returning, on the failure path too -- a token left
    /// answerable after its order is gone is a response the proxy will hand out
    /// to anyone who asks for it.
    pub async fn issue(&self, domain: &str) -> ProxyResult<Certificate> {
        let account = self.account().await?;

        let identifiers = [Identifier::Dns(domain.to_owned())];
        let mut order = account
            .new_order(&NewOrder::new(&identifiers))
            .await
            .map_err(|e| ProxyError::Acme(format!("could not place an order for {domain}: {e}")))?;

        let result = self.satisfy_and_finalize(&mut order, domain).await;

        // Whatever happened, the tokens must stop being answerable.
        if let Err(cleanup) = self.clear_challenges(domain).await {
            tracing::warn!(%domain, error = %cleanup, "could not clear ACME challenge rows");
        }

        let (cert_pem, key_pem) = result?;

        let certificate = Certificate {
            domain: domain.to_owned(),
            expires_at: crate::tls::expiry_of(cert_pem.as_bytes()),
            cert_pem: cert_pem.into_bytes(),
            key_pem: key_pem.into_bytes(),
        };
        self.cert_store.save(certificate.clone()).await?;

        tracing::info!(
            %domain,
            expires_at = ?certificate.expires_at,
            "issued an ACME certificate"
        );
        Ok(certificate)
    }

    /// Answer every authorization, finalize, and fetch the chain.
    ///
    /// Returns `(certificate_chain_pem, private_key_pem)`.
    async fn satisfy_and_finalize(
        &self,
        order: &mut instant_acme::Order,
        domain: &str,
    ) -> ProxyResult<(String, String)> {
        let mut authorizations = order.authorizations();
        while let Some(result) = authorizations.next().await {
            let mut authz = result
                .map_err(|e| ProxyError::Acme(format!("could not read an authorization: {e}")))?;

            match authz.status {
                // Already satisfied, from an earlier order for the same domain.
                AuthorizationStatus::Valid => continue,
                AuthorizationStatus::Pending => {}
                other => {
                    return Err(ProxyError::Acme(format!(
                        "authorization for {domain} is {other:?}, which cannot be answered"
                    )))
                }
            }

            let mut challenge = authz.challenge(ChallengeType::Http01).ok_or_else(|| {
                ProxyError::Acme(format!(
                    "the CA offered no http-01 challenge for {domain}; \
                     only http-01 is implemented"
                ))
            })?;

            // Store the response *before* telling the CA to look for it. The
            // other order loses the race: validation can arrive immediately.
            let token = challenge.token.clone();
            let key_authorization = challenge.key_authorization().as_str().to_owned();
            self.store_challenge(&token, domain, &key_authorization)
                .await?;

            challenge.set_ready().await.map_err(|e| {
                ProxyError::Acme(format!("the CA rejected our challenge for {domain}: {e}"))
            })?;
        }

        let retry = RetryPolicy::default().timeout(ORDER_TIMEOUT);

        order.poll_ready(&retry).await.map_err(|e| {
            ProxyError::Acme(format!(
                "the order for {domain} did not become ready: {e}. \
                 The usual cause is that {domain} does not resolve to this proxy, \
                 so the CA could not fetch the challenge."
            ))
        })?;

        let key_pem = order
            .finalize()
            .await
            .map_err(|e| ProxyError::Acme(format!("could not finalize {domain}: {e}")))?;

        let cert_pem = order
            .poll_certificate(&retry)
            .await
            .map_err(|e| ProxyError::Acme(format!("no certificate came back for {domain}: {e}")))?;

        Ok((cert_pem, key_pem))
    }

    /// Load the stored account, or register one and store it.
    ///
    /// Registering is rate-limited -- Let's Encrypt allows 10 per IP per three
    /// hours -- so this must not happen per issuance. Keyed by directory URL so
    /// moving between staging and production does not reuse an account the
    /// other CA has never heard of.
    async fn account(&self) -> ProxyResult<Account> {
        if let Some(credentials) = self.load_credentials().await? {
            return self
                .builder()?
                .from_credentials(credentials)
                .await
                .map_err(|e| {
                    ProxyError::Acme(format!("stored ACME credentials are unusable: {e}"))
                });
        }

        let contact = self.contact.as_ref().map(|email| format!("mailto:{email}"));
        let contact_refs: Vec<&str> = contact.iter().map(String::as_str).collect();

        let (account, credentials) = self
            .builder()?
            .create(
                &NewAccount {
                    contact: &contact_refs,
                    terms_of_service_agreed: true,
                    only_return_existing: false,
                },
                self.directory_url.clone(),
                None,
            )
            .await
            .map_err(|e| {
                ProxyError::Acme(format!(
                    "could not register an ACME account at {}: {e}",
                    self.directory_url
                ))
            })?;

        self.save_credentials(&credentials).await?;
        tracing::info!(directory = %self.directory_url, "registered a new ACME account");
        Ok(account)
    }

    fn builder(&self) -> ProxyResult<instant_acme::AccountBuilder> {
        // instant-acme talks to the directory over rustls, and rustls refuses
        // to guess a provider when both aws-lc-rs and ring are reachable in the
        // tree -- it panics on first use instead. The TLS listener installs one
        // for the same reason; an ACME order can happen without any listener
        // having been built, so it has to be done here as well. Idempotent.
        crate::tls::server::install_crypto_provider();

        match &self.root_pem_path {
            Some(path) => Account::builder_with_root(path).map_err(|e| {
                ProxyError::Acme(format!(
                    "could not use {} as an ACME root: {e}",
                    path.display()
                ))
            }),
            None => Account::builder()
                .map_err(|e| ProxyError::Acme(format!("could not build an ACME client: {e}"))),
        }
    }

    async fn load_credentials(&self) -> ProxyResult<Option<AccountCredentials>> {
        let row =
            sqlx::query("SELECT credentials FROM proxy_acme_accounts WHERE directory_url = $1")
                .bind(&self.directory_url)
                .fetch_optional(&self.pool)
                .await?;

        let Some(row) = row else { return Ok(None) };
        let value: serde_json::Value = row.try_get("credentials")?;
        serde_json::from_value(value)
            .map(Some)
            .map_err(|e| ProxyError::Acme(format!("stored ACME credentials do not parse: {e}")))
    }

    async fn save_credentials(&self, credentials: &AccountCredentials) -> ProxyResult<()> {
        let value = serde_json::to_value(credentials)
            .map_err(|e| ProxyError::Acme(format!("could not serialize ACME credentials: {e}")))?;

        // ON CONFLICT DO NOTHING, not DO UPDATE: if two workers registered at
        // once, the first account stored is the one whose authorizations the CA
        // already knows about. Overwriting it would discard them.
        sqlx::query(
            "INSERT INTO proxy_acme_accounts (directory_url, credentials) \
             VALUES ($1, $2) ON CONFLICT (directory_url) DO NOTHING",
        )
        .bind(&self.directory_url)
        .bind(value)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn store_challenge(
        &self,
        token: &str,
        domain: &str,
        key_authorization: &str,
    ) -> ProxyResult<()> {
        sqlx::query(
            "INSERT INTO proxy_acme_challenges (token, domain, key_authorization) \
             VALUES ($1, $2, $3) \
             ON CONFLICT (token) DO UPDATE SET \
                 domain = EXCLUDED.domain, \
                 key_authorization = EXCLUDED.key_authorization, \
                 expires_at = NOW() + INTERVAL '1 hour'",
        )
        .bind(token)
        .bind(domain)
        .bind(key_authorization)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn clear_challenges(&self, domain: &str) -> ProxyResult<()> {
        sqlx::query("DELETE FROM proxy_acme_challenges WHERE domain = $1 OR expires_at < NOW()")
            .bind(domain)
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}

/// A live HTTP-01 challenge response.
pub struct AcmeChallengeResponse {
    /// The domain the challenge was issued for.
    pub domain: String,
    /// The body to return, verbatim.
    pub key_authorization: String,
}

/// Look up a pending HTTP-01 challenge response by token.
///
/// Free function rather than a method because the endpoint that serves
/// challenges has a pool but no reason to hold an issuer.
pub async fn find_challenge(
    pool: &PgPool,
    token: &str,
) -> ProxyResult<Option<AcmeChallengeResponse>> {
    let row = sqlx::query(
        "SELECT domain, key_authorization FROM proxy_acme_challenges \
         WHERE token = $1 AND expires_at > NOW()",
    )
    .bind(token)
    .fetch_optional(pool)
    .await?;

    Ok(row.map(|row| AcmeChallengeResponse {
        domain: row.get("domain"),
        key_authorization: row.get("key_authorization"),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_directories_are_https_urls() {
        for url in [LETS_ENCRYPT_PRODUCTION, LETS_ENCRYPT_STAGING] {
            assert!(url.starts_with("https://"), "{url}");
            assert!(url.ends_with("/directory"), "{url}");
        }
        assert_ne!(LETS_ENCRYPT_PRODUCTION, LETS_ENCRYPT_STAGING);
    }
}
