//! JWT token validation.
//!
//! The signature is checked by the library; the claims are checked here. That
//! split is deliberate: a claim can be wrong in two different ways -- absent,
//! or present and not the kind of thing that claim is -- and the client needs
//! to be told which. A library that folds both into "invalid token" answers a
//! question nobody asked.

use super::{AuthResult, ClaimFault, JwtConfig, JwtError};
use jsonwebtoken::{decode, Algorithm, DecodingKey, Validation};
use std::collections::HashMap;

/// How far ahead of the server a token's clock may be.
///
/// A token minted a moment ago on a machine whose clock runs a little fast is
/// not a forgery, and rejecting it would make a working deployment fail
/// intermittently and unreproducibly.
///
/// Only in that direction. `exp` is checked to the second, because leniency
/// there is leniency about a token that has been *withdrawn* -- it keeps a
/// revoked session alive past the moment its issuer said it ended, and it is a
/// window an attacker holding a stale token gets for free. `nbf` and `iat`
/// describe a token that is not valid *yet*, where the same slack costs
/// nothing and is the only remedy for a clock nobody controls.
const ALLOWED_SKEW: i64 = 30;

/// Validate a JWT token and extract claims.
pub fn validate_token(token: &str, config: &JwtConfig) -> Result<AuthResult, JwtError> {
    let secret = config.secret.as_ref().ok_or(JwtError::SecretMissing)?;

    // Decode secret
    let key_bytes = if config.secret_is_base64 {
        base64_decode(secret)?
    } else {
        secret.as_bytes().to_vec()
    };

    let key = DecodingKey::from_secret(&key_bytes);

    // Signature only. Every claim the library could check here, it checks
    // differently from the way this API's contract says to -- a missing `exp`
    // is not an error, an `aud` that is not a string has a code of its own --
    // so all of it is done below, where the answers can be the right ones.
    let mut validation = Validation::new(Algorithm::HS256);
    validation.validate_exp = false;
    validation.validate_nbf = false;
    validation.validate_aud = false;
    validation.required_spec_claims.clear();

    let token_data = decode::<HashMap<String, serde_json::Value>>(token, &key, &validation)
        .map_err(map_jwt_error)?;
    let claims = token_data.claims;

    check_claims(&claims, config, now())?;

    // The role decides what the request may do, so a token naming none, on a
    // server with no anonymous role, is a request with no identity at all.
    let role = role_of(&claims, config).ok_or(JwtError::NoIdentity)?;

    Ok(AuthResult { role, claims })
}

/// The role a token names, or the anonymous one where it names none.
///
/// A role is a name, and only a string is one. Rendering anything else --
/// `"role": 42`, `"role": {"a": 1}` -- would make a database role nobody
/// created out of a claim that does not name one, so a claim of the wrong
/// shape is read as a token that named no role at all and falls back the same
/// way an absent claim does.
fn role_of(claims: &HashMap<String, serde_json::Value>, config: &JwtConfig) -> Option<String> {
    claims
        .get(&config.role_claim_key)
        .and_then(serde_json::Value::as_str)
        .map(str::to_string)
        .or_else(|| config.anon_role.clone())
}

/// The registered claims, checked in the order the contract checks them.
///
/// Order is part of the answer: a token that is both expired and out of
/// audience is reported as expired, because that is the first thing wrong with
/// it and the one the client can act on.
fn check_claims(
    claims: &HashMap<String, serde_json::Value>,
    config: &JwtConfig,
    now: i64,
) -> Result<(), JwtError> {
    // A claim that is absent is not a claim that is wrong. Only one that is
    // present in the wrong shape is.
    let number = |name: &'static str| -> Result<Option<i64>, JwtError> {
        match claims.get(name) {
            None | Some(serde_json::Value::Null) => Ok(None),
            Some(value) => value
                .as_f64()
                .map(|seconds| seconds as i64)
                .map(Some)
                .ok_or(JwtError::Claim(ClaimFault::NotANumber(name))),
        }
    };

    if let Some(exp) = number("exp")? {
        if now > exp {
            return Err(JwtError::Claim(ClaimFault::Expired));
        }
    }
    if let Some(nbf) = number("nbf")? {
        if now + ALLOWED_SKEW < nbf {
            return Err(JwtError::Claim(ClaimFault::NotYetValid));
        }
    }
    if let Some(iat) = number("iat")? {
        if now + ALLOWED_SKEW < iat {
            return Err(JwtError::Claim(ClaimFault::IssuedInFuture));
        }
    }

    // With no audience configured the server accepts any, but the claim still
    // has to *be* an audience: a list with an object in it is a malformed
    // token, not a token meant for somebody else. That is the difference
    // between a client fixing how it mints tokens and hunting for a setting
    // this server does not have.
    let matches = |aud: &str| match &config.audience {
        Some(wanted) => aud == wanted,
        None => true,
    };
    match claims.get("aud") {
        None | Some(serde_json::Value::Null) => {}
        Some(serde_json::Value::String(aud)) => {
            if !matches(aud) {
                return Err(JwtError::Claim(ClaimFault::NotInAudience));
            }
        }
        Some(serde_json::Value::Array(auds)) => {
            if !auds.iter().all(serde_json::Value::is_string) {
                return Err(JwtError::Claim(ClaimFault::AudienceNotStrings));
            }
            // A list naming no audience excludes nobody.
            let admitted = auds
                .iter()
                .filter_map(serde_json::Value::as_str)
                .any(matches);
            if !auds.is_empty() && !admitted {
                return Err(JwtError::Claim(ClaimFault::NotInAudience));
            }
        }
        Some(_) => return Err(JwtError::Claim(ClaimFault::AudienceNotStrings)),
    }

    Ok(())
}

/// Seconds since the epoch.
fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs() as i64)
        .unwrap_or_default()
}

/// Decode base64 secret.
fn base64_decode(s: &str) -> Result<Vec<u8>, JwtError> {
    use base64::{engine::general_purpose::STANDARD, Engine};
    STANDARD.decode(s).map_err(|_| JwtError::NoSuitableKey)
}

/// Map jsonwebtoken error to JwtError.
fn map_jwt_error(e: jsonwebtoken::errors::Error) -> JwtError {
    use jsonwebtoken::errors::ErrorKind;

    match e.kind() {
        ErrorKind::InvalidSignature => JwtError::InvalidSignature,
        // The token was read and its claims were not a JSON object, which is a
        // different failure from a key that could not decode it at all.
        ErrorKind::Json(_) => JwtError::Claim(ClaimFault::Unparsable),
        _ => JwtError::NoSuitableKey,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> JwtConfig {
        JwtConfig {
            secret: Some("reallyreallyreallyreallyverysafe".into()),
            ..JwtConfig::default()
        }
    }

    fn claims(json: serde_json::Value) -> HashMap<String, serde_json::Value> {
        match json {
            serde_json::Value::Object(map) => map.into_iter().collect(),
            _ => panic!("claims must be an object"),
        }
    }

    /// The token in PostgREST's own suite carries `"aud": [{...}, "test"]`,
    /// which is neither an audience nor a list of them.
    #[test]
    fn an_audience_that_is_not_a_string_is_a_malformed_token() {
        assert!(matches!(
            check_claims(
                &claims(serde_json::json!({"aud": [{"invalid": "value"}, "test"]})),
                &config(),
                0
            ),
            Err(JwtError::Claim(ClaimFault::AudienceNotStrings))
        ));
    }

    /// With no audience configured, any audience is this server's.
    #[test]
    fn any_audience_is_accepted_when_none_is_configured() {
        assert!(check_claims(
            &claims(serde_json::json!({"aud": ["someone", "else"]})),
            &config(),
            0
        )
        .is_ok());
    }

    /// A list naming no audience excludes nobody.
    #[test]
    fn an_empty_audience_list_excludes_no_one() {
        let configured = JwtConfig {
            audience: Some("us".into()),
            ..config()
        };
        assert!(check_claims(&claims(serde_json::json!({"aud": []})), &configured, 0).is_ok());
        assert!(matches!(
            check_claims(
                &claims(serde_json::json!({"aud": ["them"]})),
                &configured,
                0
            ),
            Err(JwtError::Claim(ClaimFault::NotInAudience))
        ));
    }

    /// A token that does not say when it expires does not expire.
    #[test]
    fn a_token_without_exp_is_not_rejected_for_having_none() {
        assert!(check_claims(&claims(serde_json::json!({"role": "x"})), &config(), 0).is_ok());
    }

    /// Present, but not the kind of thing that claim is.
    #[test]
    fn a_timestamp_claim_that_is_not_a_number_is_reported_as_such() {
        assert!(matches!(
            check_claims(&claims(serde_json::json!({"exp": "soon"})), &config(), 0),
            Err(JwtError::Claim(ClaimFault::NotANumber("exp")))
        ));
    }

    /// A clock a few seconds ahead is not a forgery.
    #[test]
    fn a_token_that_is_not_valid_yet_is_given_the_benefit_of_the_clock() {
        let issued_moments_hence = claims(serde_json::json!({"iat": 1_000}));
        assert!(check_claims(&issued_moments_hence, &config(), 990).is_ok());
        assert!(matches!(
            check_claims(&issued_moments_hence, &config(), 900),
            Err(JwtError::Claim(ClaimFault::IssuedInFuture))
        ));

        let valid_moments_hence = claims(serde_json::json!({"nbf": 1_000}));
        assert!(check_claims(&valid_moments_hence, &config(), 990).is_ok());
        assert!(matches!(
            check_claims(&valid_moments_hence, &config(), 900),
            Err(JwtError::Claim(ClaimFault::NotYetValid))
        ));
    }

    /// The same slack the other way would keep a withdrawn token alive past
    /// the second its issuer said it ended.
    #[test]
    fn an_expiry_is_honoured_to_the_second() {
        let expires_at = claims(serde_json::json!({"exp": 1_000}));
        assert!(check_claims(&expires_at, &config(), 1_000).is_ok());
        assert!(matches!(
            check_claims(&expires_at, &config(), 1_001),
            Err(JwtError::Claim(ClaimFault::Expired))
        ));
    }

    /// A role is a name, and a claim that is not a string does not carry one.
    #[test]
    fn a_role_claim_that_is_not_a_string_names_no_role() {
        let configured = JwtConfig {
            anon_role: Some("anon".into()),
            ..config()
        };
        for value in [
            serde_json::json!(42),
            serde_json::json!({"a": 1}),
            serde_json::json!(["admin"]),
        ] {
            let mut claims = HashMap::new();
            claims.insert("role".to_string(), value.clone());
            assert_eq!(
                role_of(&claims, &configured),
                Some("anon".to_string()),
                "{value}"
            );
        }

        let mut named = HashMap::new();
        named.insert("role".to_string(), serde_json::json!("web_user"));
        assert_eq!(role_of(&named, &configured), Some("web_user".to_string()));
    }

    /// The first thing wrong with the token is what the client is told.
    #[test]
    fn an_expired_token_is_reported_as_expired_whatever_else_is_wrong() {
        assert!(matches!(
            check_claims(
                &claims(serde_json::json!({"exp": 1_000, "aud": [{"bad": 1}]})),
                &config(),
                9_000
            ),
            Err(JwtError::Claim(ClaimFault::Expired))
        ));
    }
}
