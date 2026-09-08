//! TLS and ACME certificate management.

mod cert_store;
pub(crate) mod server;
mod sni;
mod validate;

#[cfg(feature = "acme")]
mod acme;

pub use cert_store::{Certificate, CertificateStore};
pub use server::{
    build_server_config, build_server_config_with_resolver, load_server_config, ALPN_PROTOCOLS,
};
pub use sni::SniCertResolver;
pub use validate::{expiry_of, facts, validate_for_domain, CertificateFacts};

#[cfg(feature = "acme")]
pub use acme::{
    find_challenge, AcmeChallengeResponse, AcmeIssuer, LETS_ENCRYPT_PRODUCTION,
    LETS_ENCRYPT_STAGING,
};
