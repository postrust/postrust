//! The keys tokens are verified with.
//!
//! `PGRST_JWT_SECRET` is read the way PostgREST reads `jwt-secret`: a plain
//! string is an HMAC secret, and a JSON Web Key or a JSON Web Key Set is a set
//! of keys -- symmetric (`oct`) or asymmetric (`RSA`, `EC`, `OKP`). Either way
//! the result is a [`KeySet`], parsed once when the configuration is read and
//! never per request.
//!
//! The rule everything here serves: **the algorithm comes from the key, never
//! from the token**. A token's header says how it claims to be signed, and the
//! party that wrote that header is the one being authenticated. Each key
//! carries the algorithms it admits -- its JWK `alg` if it names one, the
//! algorithms its type can do if it does not -- and a token whose `alg` the
//! selected key does not admit is refused before any cryptography runs. That
//! is what closes the classic confusions: `alg: none` names nothing any key
//! admits, and an `HS256` token "signed" with an RSA public key's bytes finds
//! no HMAC key to be checked against, because the RSA key admits only RSA.
//!
//! Selection follows PostgREST (which follows `jose-jwt`'s `findMatchingJwsKeys`):
//! a token that names a `kid` is checked against the keys carrying that `kid`
//! and no others; a token that names none is checked against every key that
//! admits its algorithm, and passes if any of them verifies it.

use std::sync::{Arc, RwLock};

use base64::Engine;

use jsonwebtoken::{Algorithm, DecodingKey};

/// The algorithms a plain-string HMAC secret admits.
///
/// Only HS256, as in PostgREST, which builds its key for a string secret as
/// `SymmetricJwk bytes Nothing (Just Sig) (Just (Signed HS256))`. A
/// deployment that wants HS384 or HS512 says so with an `oct` JWK.
const PLAIN_SECRET_ALGORITHMS: &[Algorithm] = &[Algorithm::HS256];

const HMAC: &[Algorithm] = &[Algorithm::HS256, Algorithm::HS384, Algorithm::HS512];
const RSA: &[Algorithm] = &[
    Algorithm::RS256,
    Algorithm::RS384,
    Algorithm::RS512,
    Algorithm::PS256,
    Algorithm::PS384,
    Algorithm::PS512,
];

/// The smallest RSA modulus accepted, in bits.
///
/// ring, which verifies natively, refuses anything smaller at verification
/// time, and the RustCrypto backend a Worker uses does not. Refusing it here,
/// once, makes both builds agree -- and says so at startup rather than as a
/// 401 on every request.
const MIN_RSA_BITS: usize = 2048;

/// A JWT secret that cannot be turned into keys.
///
/// Always a configuration fault, reported when the configuration is read: a
/// server that started with a key it cannot use would answer every token with
/// a 401 and leave the operator to work out why.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum JwtKeyError {
    /// The secret is said to be base64 and is not.
    #[error("the JWT secret is configured as base64 and is not valid base64")]
    NotBase64,

    /// The secret starts like a JSON object and does not parse as one.
    ///
    /// Not quietly read as an HMAC secret instead, which is what PostgREST
    /// does: a public key with a typo in it would then become an HMAC secret
    /// that anyone holding the public key can sign with.
    #[error("the JWT secret looks like a JSON Web Key but is not valid JSON: {0}")]
    NotJson(String),

    /// A JSON object that is neither a JWK nor a JWK Set.
    #[error("the JWT secret is a JSON object but neither a JWK (no \"kty\") nor a JWK Set (no \"keys\" array)")]
    NotAKey,

    /// One key is malformed.
    #[error("{at}: {reason}")]
    BadKey {
        /// Which key: `JWK`, or `key 2 of the JWK Set`.
        at: String,
        /// What is wrong with it.
        reason: String,
    },

    /// Every key was set aside as unable to verify a signature.
    #[error("no key in the JWT secret can verify a JWT signature ({0})")]
    NoUsableKeys(String),
}

/// One key, and what it may verify.
struct Key {
    /// The JWK `kid`, if it has one.
    kid: Option<String>,
    /// Whether a token's `kid` is ignored when selecting this key.
    ///
    /// Only for the plain-string secret, which is a single key nobody can
    /// name. PostgREST gives it no `kid`, so a token carrying one is refused
    /// there; tokens minted by identity providers routinely carry one even
    /// when signed with a shared secret, and this server has always accepted
    /// them, so it still does.
    any_kid: bool,
    /// The algorithms this key admits: never empty.
    algorithms: &'static [Algorithm],
    /// Kept for the `Debug` output; the key material never is.
    kty: &'static str,
    key: DecodingKey,
}

impl Key {
    fn admits(&self, alg: Algorithm) -> bool {
        self.algorithms.contains(&alg)
    }

    fn named(&self, kid: Option<&str>) -> bool {
        match kid {
            None => true,
            Some(_) if self.any_kid => true,
            // A key with no `kid` cannot be the one a `kid` names.
            Some(kid) => self.kid.as_deref() == Some(kid),
        }
    }
}

/// A parsed set of verification keys.
///
/// Immutable once built. A set that has to change -- a JWKS fetched again
/// after its issuer rotated a key -- is replaced whole through [`JwtKeys`], so
/// a request never sees half of one set and half of another.
pub struct KeySet {
    keys: Vec<Key>,
}

impl std::fmt::Debug for KeySet {
    /// The shape of the set, never the key material: a `Debug` of the
    /// configuration ends up in logs.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_list()
            .entries(self.keys.iter().map(|k| {
                format!(
                    "{}{} {:?}",
                    k.kty,
                    k.kid
                        .as_deref()
                        .map(|kid| format!(" kid={kid}"))
                        .unwrap_or_default(),
                    k.algorithms
                )
            }))
            .finish()
    }
}

impl KeySet {
    /// Read `PGRST_JWT_SECRET`.
    ///
    /// Base64 is undone first when the configuration says the secret is
    /// base64, and what it decodes to is then read like any other secret --
    /// so a base64-encoded JWK is a JWK, as in PostgREST.
    pub fn from_secret(secret: &str, secret_is_base64: bool) -> Result<Self, JwtKeyError> {
        let bytes = if secret_is_base64 {
            base64::engine::general_purpose::STANDARD
                .decode(secret)
                .map_err(|_| JwtKeyError::NotBase64)?
        } else {
            secret.as_bytes().to_vec()
        };

        // A JSON object is a key or a set of them; anything else is the bytes
        // of an HMAC secret. Only text can be JSON: random bytes from a base64
        // secret that happen to start with `{` are almost never UTF-8, and
        // stay a secret.
        if let Ok(text) = std::str::from_utf8(&bytes) {
            if text.trim_start().starts_with('{') {
                return Self::from_jwk_json(text);
            }
        }

        Ok(Self {
            keys: vec![Key {
                kid: None,
                any_kid: true,
                algorithms: PLAIN_SECRET_ALGORITHMS,
                kty: "secret",
                key: DecodingKey::from_secret(&bytes),
            }],
        })
    }

    /// Read a JWK, or a JWK Set (`{"keys": [...]}`), from its JSON.
    ///
    /// Keys that cannot verify a signature -- `"use": "enc"`, `key_ops`
    /// without `verify`, an encryption `alg`, a key type or curve this server
    /// does not implement -- are set aside with a warning rather than failing
    /// the whole set, because an identity provider's JWKS routinely mixes them
    /// in with its signing keys. A set left with no key at all is an error.
    ///
    /// A key that claims to be usable and is malformed fails the set: that is
    /// a mistake in the configuration, not a key meant for something else.
    pub fn from_jwk_json(json: &str) -> Result<Self, JwtKeyError> {
        let value: serde_json::Value =
            serde_json::from_str(json).map_err(|e| JwtKeyError::NotJson(e.to_string()))?;
        let object = value.as_object().ok_or(JwtKeyError::NotAKey)?;

        let entries: Vec<(String, &serde_json::Value)> = match object.get("keys") {
            Some(serde_json::Value::Array(keys)) => keys
                .iter()
                .enumerate()
                .map(|(i, k)| (format!("key {} of the JWK Set", i + 1), k))
                .collect(),
            Some(_) => {
                return Err(JwtKeyError::BadKey {
                    at: "JWK Set".into(),
                    reason: "\"keys\" must be an array".into(),
                })
            }
            None if object.contains_key("kty") => vec![("JWK".to_string(), &value)],
            None => return Err(JwtKeyError::NotAKey),
        };

        let mut keys = Vec::with_capacity(entries.len());
        let mut set_aside = Vec::new();
        for (at, entry) in entries {
            match parse_jwk(entry) {
                Ok(Parsed::Key(key)) => keys.push(key),
                Ok(Parsed::SetAside(why)) => {
                    tracing::warn!("JWT secret: {at} is not used: {why}");
                    set_aside.push(format!("{at}: {why}"));
                }
                Err(reason) => return Err(JwtKeyError::BadKey { at, reason }),
            }
        }

        if keys.is_empty() {
            return Err(JwtKeyError::NoUsableKeys(if set_aside.is_empty() {
                "the JWK Set is empty".into()
            } else {
                set_aside.join("; ")
            }));
        }
        Ok(Self { keys })
    }

    /// How many keys the set holds.
    pub fn len(&self) -> usize {
        self.keys.len()
    }

    /// Whether the set holds no keys. Never true of a set this module built.
    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }

    /// The keys a token with this header may be checked against, in the order
    /// the configuration lists them.
    ///
    /// The seam key selection lives behind: a `kid` narrows the set to the
    /// keys carrying it, and the header's algorithm to the keys admitting it.
    pub(crate) fn candidates<'a>(
        &'a self,
        kid: Option<&'a str>,
        alg: Algorithm,
    ) -> impl Iterator<Item = &'a DecodingKey> + 'a {
        self.keys
            .iter()
            .filter(move |k| k.named(kid) && k.admits(alg))
            .map(|k| &k.key)
    }
}

/// The verification keys a server holds, shared and replaceable.
///
/// Cloning shares the set. [`JwtKeys::replace`] swaps the whole set at once
/// for every holder, which is what a JWKS refresher needs: a request takes
/// [`JwtKeys::current`] once and verifies against that snapshot, so a swap
/// mid-request changes nothing about the request already running.
///
/// A plain `RwLock` around an `Arc`: the lock is held only to clone the `Arc`
/// out, never across a verification, so readers do not wait on each other in
/// any way that matters and a writer waits for nothing but those clones.
#[derive(Clone)]
pub struct JwtKeys {
    current: Arc<RwLock<Arc<KeySet>>>,
}

impl std::fmt::Debug for JwtKeys {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.current().fmt(f)
    }
}

impl JwtKeys {
    /// Hold `set`.
    pub fn new(set: KeySet) -> Self {
        Self {
            current: Arc::new(RwLock::new(Arc::new(set))),
        }
    }

    /// The keys for `PGRST_JWT_SECRET`, or `None` when no secret is set.
    ///
    /// Call it where the configuration is read, and treat an error as a
    /// reason not to start.
    pub fn from_config(
        secret: Option<&str>,
        secret_is_base64: bool,
    ) -> Result<Option<Self>, JwtKeyError> {
        secret
            .map(|secret| KeySet::from_secret(secret, secret_is_base64).map(Self::new))
            .transpose()
    }

    /// The set as it is now.
    pub fn current(&self) -> Arc<KeySet> {
        // A poisoned lock still holds a whole set: the only write is a single
        // assignment of a finished one.
        self.current
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// Replace the set, for every holder of this handle.
    ///
    /// Tokens already in a [`crate::JwtCache`] are not re-checked: one
    /// verified by a key that has since been withdrawn is answered from the
    /// cache until its entry expires.
    pub fn replace(&self, set: KeySet) {
        *self.current.write().unwrap_or_else(|e| e.into_inner()) = Arc::new(set);
    }
}

/// What one JWK turned out to be.
enum Parsed {
    Key(Key),
    /// A well-formed key that cannot verify a signature here, and why.
    SetAside(String),
}

/// Read one JWK. `Err` is a malformed key, with the reason.
fn parse_jwk(value: &serde_json::Value) -> Result<Parsed, String> {
    let jwk = value.as_object().ok_or("a JWK must be a JSON object")?;

    let string = |name: &str| -> Result<Option<&str>, String> {
        match jwk.get(name) {
            None | Some(serde_json::Value::Null) => Ok(None),
            Some(serde_json::Value::String(s)) => Ok(Some(s)),
            Some(_) => Err(format!("\"{name}\" must be a string")),
        }
    };
    let required = |name: &str| -> Result<&str, String> {
        string(name)?.ok_or_else(|| format!("\"{name}\" is missing"))
    };

    let kty = required("kty")?;
    let kid = string("kid")?.map(str::to_string);

    // RFC 7517 4.2 and 4.3: a key declared for encryption, or for operations
    // that do not include verifying, is not a signature key whatever its
    // type would allow.
    match string("use")? {
        None | Some("sig") => {}
        Some(other) => return Ok(Parsed::SetAside(format!("\"use\" is \"{other}\""))),
    }
    if let Some(ops) = jwk.get("key_ops") {
        let ops = ops
            .as_array()
            .ok_or("\"key_ops\" must be an array of strings")?;
        if !ops.iter().any(|op| op.as_str() == Some("verify")) {
            return Ok(Parsed::SetAside(
                "\"key_ops\" does not include \"verify\"".into(),
            ));
        }
    }

    let alg = match string("alg")? {
        None => None,
        Some(name) => match signing_algorithm(name) {
            Some(alg) => Some(alg),
            // `RSA-OAEP`, `ECDH-ES`, `ES512`, ...: an encryption algorithm or
            // one this server does not verify.
            None => {
                return Ok(Parsed::SetAside(format!(
                    "\"alg\" is \"{name}\", which this server does not verify"
                )))
            }
        },
    };

    let (kty, family, key): (&'static str, &'static [Algorithm], DecodingKey) = match kty {
        "oct" => {
            let k = base64url(required("k")?, "k")?;
            if k.is_empty() {
                return Err("\"k\" is empty".into());
            }
            ("oct", HMAC, DecodingKey::from_secret(&k))
        }
        "RSA" => {
            let (n, e) = (required("n")?, required("e")?);
            let modulus = base64url(n, "n")?;
            let bits = significant_bits(&modulus);
            if bits < MIN_RSA_BITS {
                return Err(format!(
                    "the RSA modulus is {bits} bits; at least {MIN_RSA_BITS} are required"
                ));
            }
            base64url(e, "e")?;
            let key = DecodingKey::from_rsa_components(n, e).map_err(|e| e.to_string())?;
            ("RSA", RSA, key)
        }
        "EC" => {
            let (family, size): (&'static [Algorithm], usize) = match required("crv")? {
                "P-256" => (&[Algorithm::ES256], 32),
                "P-384" => (&[Algorithm::ES384], 48),
                other => {
                    return Ok(Parsed::SetAside(format!(
                        "the curve \"{other}\" is not one this server verifies"
                    )))
                }
            };
            let (x, y) = (required("x")?, required("y")?);
            for (name, coordinate) in [("x", x), ("y", y)] {
                if base64url(coordinate, name)?.len() != size {
                    return Err(format!("\"{name}\" must be {size} bytes for this curve"));
                }
            }
            let key = DecodingKey::from_ec_components(x, y).map_err(|e| e.to_string())?;
            ("EC", family, key)
        }
        "OKP" => {
            match required("crv")? {
                "Ed25519" => {}
                other => {
                    return Ok(Parsed::SetAside(format!(
                        "the curve \"{other}\" is not one this server verifies"
                    )))
                }
            }
            let x = required("x")?;
            if base64url(x, "x")?.len() != 32 {
                return Err("\"x\" must be 32 bytes for Ed25519".into());
            }
            let key = DecodingKey::from_ed_components(x).map_err(|e| e.to_string())?;
            ("OKP", &[Algorithm::EdDSA], key)
        }
        other => {
            return Ok(Parsed::SetAside(format!(
                "the key type \"{other}\" is not one this server verifies"
            )))
        }
    };

    // An `alg` naming a signing algorithm of another family is a key that
    // contradicts itself -- `"kty": "RSA", "alg": "HS256"` is exactly the
    // confusion this module exists to prevent -- so it is refused outright
    // rather than set aside.
    let algorithms: &'static [Algorithm] = match alg {
        None => family,
        Some(alg) => match family.iter().position(|a| *a == alg) {
            Some(i) => &family[i..=i],
            None => return Err(format!("\"alg\" is {alg:?}, which a {kty} key cannot do")),
        },
    };

    Ok(Parsed::Key(Key {
        kid,
        any_kid: false,
        algorithms,
        kty,
        key,
    }))
}

/// The JWS algorithms this server verifies, by their JOSE names.
///
/// Spelled out rather than parsed through the library's `Algorithm`, so that
/// `none` -- and anything else the library might one day learn -- can never be
/// admitted by a key without this list changing.
fn signing_algorithm(name: &str) -> Option<Algorithm> {
    Some(match name {
        "HS256" => Algorithm::HS256,
        "HS384" => Algorithm::HS384,
        "HS512" => Algorithm::HS512,
        "RS256" => Algorithm::RS256,
        "RS384" => Algorithm::RS384,
        "RS512" => Algorithm::RS512,
        "PS256" => Algorithm::PS256,
        "PS384" => Algorithm::PS384,
        "PS512" => Algorithm::PS512,
        "ES256" => Algorithm::ES256,
        "ES384" => Algorithm::ES384,
        "EdDSA" => Algorithm::EdDSA,
        _ => return None,
    })
}

/// Decode a JWK member: base64url, unpadded per RFC 7515, though a padded one
/// is accepted rather than refused over an `=`.
fn base64url(value: &str, name: &str) -> Result<Vec<u8>, String> {
    base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(value.trim_end_matches('='))
        .map_err(|_| format!("\"{name}\" is not base64url"))
}

/// The bit length of a big-endian unsigned integer.
fn significant_bits(bytes: &[u8]) -> usize {
    match bytes.iter().position(|b| *b != 0) {
        None => 0,
        Some(i) => (bytes.len() - i) * 8 - bytes[i].leading_zeros() as usize,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{validate_token, ClaimFault, JwtConfig, JwtError};
    use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
    use jsonwebtoken::{encode, EncodingKey, Header};
    use serde_json::{json, Value};

    /// Keys generated once for these tests (two RSA 2048, P-256, P-384,
    /// Ed25519): each a private key in DER -- PKCS#1 for RSA, PKCS#8
    /// otherwise -- beside its public JWK.
    fn fixture(name: &str) -> (EncodingKey, Value) {
        let all: Value = serde_json::from_str(include_str!("../tests/fixtures/keys.json")).unwrap();
        let entry = &all[name];
        let der = STANDARD
            .decode(entry["private_der"].as_str().unwrap())
            .unwrap();
        let key = match entry["jwk"]["kty"].as_str().unwrap() {
            "RSA" => EncodingKey::from_rsa_der(&der),
            "EC" => EncodingKey::from_ec_der(&der),
            _ => EncodingKey::from_ed_der(&der),
        };
        (key, entry["jwk"].clone())
    }

    /// A fixture's public JWK, with members added or overridden.
    fn jwk(name: &str, extra: Value) -> Value {
        let mut jwk = fixture(name).1;
        jwk.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        jwk
    }

    fn config_for(secret: &str) -> JwtConfig {
        JwtConfig {
            keys: JwtKeys::from_config(Some(secret), false).unwrap(),
            ..JwtConfig::default()
        }
    }

    fn jwks(keys: &[Value]) -> JwtConfig {
        config_for(&json!({ "keys": keys }).to_string())
    }

    fn token(alg: Algorithm, kid: Option<&str>, key: &EncodingKey) -> String {
        let mut header = Header::new(alg);
        header.kid = kid.map(str::to_string);
        encode(&header, &json!({"role": "web_user"}), key).unwrap()
    }

    fn signed_by(name: &str, alg: Algorithm, kid: Option<&str>) -> String {
        token(alg, kid, &fixture(name).0)
    }

    fn accepted(token: &str, config: &JwtConfig) -> bool {
        match validate_token(token, config) {
            Ok(result) => {
                assert_eq!(result.role, "web_user");
                true
            }
            Err(_) => false,
        }
    }

    fn refusal(token: &str, config: &JwtConfig) -> JwtError {
        validate_token(token, config).expect_err("the token should have been refused")
    }

    #[test]
    fn an_rsa_key_without_alg_admits_both_rsa_families() {
        let config = config_for(&fixture("rsa_a").1.to_string());
        for alg in RSA {
            assert!(
                accepted(&signed_by("rsa_a", *alg, None), &config),
                "{alg:?}"
            );
        }
    }

    #[test]
    fn each_curve_verifies_its_own_algorithm() {
        for (name, alg) in [
            ("ec_p256", Algorithm::ES256),
            ("ec_p384", Algorithm::ES384),
            ("ed25519", Algorithm::EdDSA),
        ] {
            let config = config_for(&fixture(name).1.to_string());
            assert!(accepted(&signed_by(name, alg, None), &config), "{alg:?}");
        }
    }

    #[test]
    fn a_curve_admits_no_other_algorithm() {
        // A P-384 key is not a P-256 key: ES256 names no key in this set.
        let config = config_for(&fixture("ec_p384").1.to_string());
        assert!(matches!(
            refusal(&signed_by("ec_p256", Algorithm::ES256, None), &config),
            JwtError::NoSuitableKey
        ));
    }

    #[test]
    fn a_token_signed_by_another_key_does_not_verify() {
        let config = config_for(&fixture("rsa_a").1.to_string());
        assert!(matches!(
            refusal(&signed_by("rsa_b", Algorithm::RS256, None), &config),
            JwtError::InvalidSignature
        ));
    }

    #[test]
    fn a_jwk_alg_narrows_what_the_key_admits() {
        let config = config_for(&jwk("rsa_a", json!({"alg": "RS256"})).to_string());
        assert!(accepted(
            &signed_by("rsa_a", Algorithm::RS256, None),
            &config
        ));
        assert!(matches!(
            refusal(&signed_by("rsa_a", Algorithm::PS256, None), &config),
            JwtError::NoSuitableKey
        ));
    }

    #[test]
    fn a_kid_selects_that_key_and_no_other() {
        let config = jwks(&[
            jwk("rsa_a", json!({"kid": "a"})),
            jwk("rsa_b", json!({"kid": "b"})),
        ]);
        assert!(accepted(
            &signed_by("rsa_b", Algorithm::RS256, Some("b")),
            &config
        ));
        // Named the wrong key: that key is the only one tried.
        assert!(matches!(
            refusal(&signed_by("rsa_b", Algorithm::RS256, Some("a")), &config),
            JwtError::InvalidSignature
        ));
        // Named a key the server does not hold.
        assert!(matches!(
            refusal(&signed_by("rsa_b", Algorithm::RS256, Some("c")), &config),
            JwtError::NoSuitableKey
        ));
    }

    #[test]
    fn a_kid_does_not_select_a_key_without_one() {
        // As in jose-jwt, which PostgREST uses: a key with no `kid` cannot be
        // the one a `kid` names.
        let config = config_for(&fixture("rsa_a").1.to_string());
        assert!(matches!(
            refusal(&signed_by("rsa_a", Algorithm::RS256, Some("a")), &config),
            JwtError::NoSuitableKey
        ));
    }

    #[test]
    fn without_a_kid_every_key_that_admits_the_algorithm_is_tried() {
        let config = jwks(&[
            jwk("rsa_a", json!({"kid": "a"})),
            jwk("ec_p256", json!({})),
            jwk("rsa_b", json!({"kid": "b"})),
        ]);
        assert!(accepted(
            &signed_by("rsa_b", Algorithm::RS512, None),
            &config
        ));
        assert!(accepted(
            &signed_by("ec_p256", Algorithm::ES256, None),
            &config
        ));
        assert!(matches!(
            refusal(&signed_by("ed25519", Algorithm::EdDSA, None), &config),
            JwtError::NoSuitableKey
        ));
    }

    #[test]
    fn an_hmac_token_made_from_a_public_key_finds_no_key() {
        // The classic confusion: the public key is public, so a server that
        // let the header choose HMAC would verify a token "signed" with it.
        let public = fixture("rsa_a").1;
        let config = config_for(&public.to_string());
        let n = URL_SAFE_NO_PAD
            .decode(public["n"].as_str().unwrap())
            .unwrap();
        for secret in [public.to_string().into_bytes(), n] {
            let forged = token(Algorithm::HS256, None, &EncodingKey::from_secret(&secret));
            assert!(matches!(refusal(&forged, &config), JwtError::NoSuitableKey));
        }
    }

    #[test]
    fn a_key_whose_alg_contradicts_its_type_is_a_configuration_error() {
        let error = KeySet::from_secret(&jwk("rsa_a", json!({"alg": "HS256"})).to_string(), false)
            .unwrap_err();
        assert!(matches!(error, JwtKeyError::BadKey { .. }), "{error}");
    }

    #[test]
    fn alg_none_is_refused_whatever_the_key() {
        let unsigned = format!(
            "{}.{}.",
            URL_SAFE_NO_PAD.encode(json!({"alg": "none"}).to_string()),
            URL_SAFE_NO_PAD.encode(json!({"role": "web_user"}).to_string())
        );
        for config in [
            config_for("reallyreallyreallyreallyverysafe"),
            config_for(&fixture("rsa_a").1.to_string()),
        ] {
            assert!(matches!(
                refusal(&unsigned, &config),
                JwtError::NoSuitableKey
            ));
        }
        // And no key can be configured to admit it.
        let error = KeySet::from_secret(&jwk("rsa_a", json!({"alg": "none"})).to_string(), false)
            .unwrap_err();
        assert!(matches!(error, JwtKeyError::NoUsableKeys(_)), "{error}");
    }

    #[test]
    fn an_encryption_key_is_never_used_to_verify() {
        let config = jwks(&[
            jwk("rsa_a", json!({"kid": "a", "use": "enc"})),
            jwk("rsa_b", json!({"kid": "b", "use": "sig"})),
        ]);
        assert!(matches!(
            refusal(&signed_by("rsa_a", Algorithm::RS256, Some("a")), &config),
            JwtError::NoSuitableKey
        ));
        // Without a kid only the signing key is tried, and it is the wrong one.
        assert!(matches!(
            refusal(&signed_by("rsa_a", Algorithm::RS256, None), &config),
            JwtError::InvalidSignature
        ));
        assert!(accepted(
            &signed_by("rsa_b", Algorithm::RS256, None),
            &config
        ));

        // `key_ops` without `verify` says the same thing another way.
        let config = jwks(&[
            jwk("rsa_a", json!({"key_ops": ["encrypt"]})),
            jwk("rsa_b", json!({})),
        ]);
        assert!(matches!(
            refusal(&signed_by("rsa_a", Algorithm::RS256, None), &config),
            JwtError::InvalidSignature
        ));
    }

    #[test]
    fn a_secret_of_nothing_but_encryption_keys_is_refused_at_startup() {
        let only_enc = jwk("rsa_a", json!({"use": "enc"})).to_string();
        assert!(matches!(
            KeySet::from_secret(&only_enc, false),
            Err(JwtKeyError::NoUsableKeys(_))
        ));
        assert!(matches!(
            KeySet::from_secret(r#"{"keys": []}"#, false),
            Err(JwtKeyError::NoUsableKeys(_))
        ));
    }

    #[test]
    fn keys_this_server_cannot_use_are_set_aside_not_fatal() {
        // What an identity provider's JWKS looks like: a P-521 key, an X25519
        // key and an encryption algorithm beside the one signing key.
        let config = jwks(&[
            json!({"kty": "EC", "crv": "P-521", "x": "AA", "y": "AA"}),
            json!({"kty": "OKP", "crv": "X25519", "x": "AA"}),
            jwk("rsa_b", json!({"alg": "RSA-OAEP"})),
            json!({"kty": "unheard-of"}),
            jwk("rsa_a", json!({})),
        ]);
        assert_eq!(config.keys.as_ref().unwrap().current().len(), 1);
        assert!(accepted(
            &signed_by("rsa_a", Algorithm::RS256, None),
            &config
        ));
    }

    #[test]
    fn an_oct_jwk_is_an_hmac_key() {
        let secret = b"reallyreallyreallyreallyverysafe";
        let oct = json!({"kty": "oct", "k": URL_SAFE_NO_PAD.encode(secret)});
        let config = config_for(&oct.to_string());
        for alg in HMAC {
            let signed = token(*alg, None, &EncodingKey::from_secret(secret));
            assert!(accepted(&signed, &config), "{alg:?}");
        }

        let mut only_512 = oct.clone();
        only_512["alg"] = json!("HS512");
        let config = config_for(&only_512.to_string());
        let hs256 = token(Algorithm::HS256, None, &EncodingKey::from_secret(secret));
        assert!(matches!(refusal(&hs256, &config), JwtError::NoSuitableKey));
    }

    #[test]
    fn a_plain_secret_is_hs256_and_ignores_kid() {
        let secret = "reallyreallyreallyreallyverysafe";
        let key = EncodingKey::from_secret(secret.as_bytes());
        let config = config_for(secret);
        assert!(accepted(&token(Algorithm::HS256, None, &key), &config));
        assert!(accepted(
            &token(Algorithm::HS256, Some("whatever"), &key),
            &config
        ));
        // PostgREST builds a string secret's key for HS256 alone.
        assert!(matches!(
            refusal(&token(Algorithm::HS512, None, &key), &config),
            JwtError::NoSuitableKey
        ));
        let other = EncodingKey::from_secret(b"other");
        assert!(matches!(
            refusal(&token(Algorithm::HS256, None, &other), &config),
            JwtError::InvalidSignature
        ));
    }

    #[test]
    fn a_base64_secret_may_itself_be_a_jwk() {
        let public = STANDARD.encode(fixture("ed25519").1.to_string());
        let config = JwtConfig {
            keys: JwtKeys::from_config(Some(&public), true).unwrap(),
            ..JwtConfig::default()
        };
        assert!(accepted(
            &signed_by("ed25519", Algorithm::EdDSA, None),
            &config
        ));

        let secret = STANDARD.encode("reallyreallyreallyreallyverysafe");
        let config = JwtConfig {
            keys: JwtKeys::from_config(Some(&secret), true).unwrap(),
            ..JwtConfig::default()
        };
        let key = EncodingKey::from_secret(b"reallyreallyreallyreallyverysafe");
        assert!(accepted(&token(Algorithm::HS256, None, &key), &config));

        assert_eq!(
            KeySet::from_secret("not base64!", true).unwrap_err(),
            JwtKeyError::NotBase64
        );
    }

    #[test]
    fn a_malformed_key_is_a_configuration_error() {
        let rsa = fixture("rsa_a").1;
        let short_modulus =
            json!({"kty": "RSA", "n": URL_SAFE_NO_PAD.encode([0xffu8; 128]), "e": "AQAB"});
        let mut no_n = rsa.clone();
        no_n.as_object_mut().unwrap().remove("n");
        let mut bad_x = fixture("ec_p256").1;
        bad_x["x"] = json!("AAAA");

        assert!(matches!(
            KeySet::from_secret(r#"{"kty": "RSA", "n": "#, false),
            Err(JwtKeyError::NotJson(_))
        ));
        assert_eq!(
            KeySet::from_secret(r#"{"hello": "world"}"#, false).unwrap_err(),
            JwtKeyError::NotAKey
        );
        for malformed in [short_modulus, no_n, bad_x, json!({"keys": "nope"})] {
            let error = KeySet::from_secret(&malformed.to_string(), false).unwrap_err();
            assert!(matches!(error, JwtKeyError::BadKey { .. }), "{error}");
        }
        // The message says which key, and what is wrong with it.
        let error = KeySet::from_secret(
            &json!({"keys": [rsa, {"kty": "RSA", "e": "AQAB"}]}).to_string(),
            false,
        )
        .unwrap_err();
        assert_eq!(error.to_string(), "key 2 of the JWK Set: \"n\" is missing");
    }

    #[test]
    fn replacing_the_set_is_seen_by_every_holder() {
        let set = |name: &str| KeySet::from_secret(&fixture(name).1.to_string(), false).unwrap();
        let keys = JwtKeys::new(set("rsa_a"));
        let config = JwtConfig {
            keys: Some(keys.clone()),
            ..JwtConfig::default()
        };
        let by_b = signed_by("rsa_b", Algorithm::RS256, None);
        assert!(!accepted(&by_b, &config));

        keys.replace(set("rsa_b"));
        assert!(accepted(&by_b, &config));
    }

    #[test]
    fn debug_output_shows_no_key_material() {
        let secret = "reallyreallyreallyreallyverysafe";
        let k = URL_SAFE_NO_PAD.encode(secret);
        let oct = json!({"kty": "oct", "kid": "k1", "k": k});
        for keys in [
            KeySet::from_secret(secret, false).unwrap(),
            KeySet::from_secret(&oct.to_string(), false).unwrap(),
        ] {
            let shown = format!("{keys:?}");
            assert!(!shown.contains(secret) && !shown.contains(&k), "{shown}");
        }
    }

    #[test]
    fn the_issuer_is_checked_once_the_signature_verifies() {
        let config = JwtConfig {
            issuer: Some("https://issuer.example".into()),
            ..config_for(&fixture("rsa_a").1.to_string())
        };
        let (key, _) = fixture("rsa_a");
        let with = |claims: Value| encode(&Header::new(Algorithm::RS256), &claims, &key).unwrap();
        assert!(validate_token(
            &with(json!({"role": "r", "iss": "https://issuer.example"})),
            &config
        )
        .is_ok());
        for claims in [
            json!({"role": "r", "iss": "https://elsewhere.example"}),
            json!({"role": "r", "iss": ["https://issuer.example"]}),
            json!({"role": "r"}),
        ] {
            assert!(matches!(
                validate_token(&with(claims), &config),
                Err(JwtError::Claim(ClaimFault::UnexpectedIssuer))
            ));
        }
    }
}
