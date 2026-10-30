//! A cache of validated tokens.
//!
//! Verifying a signature is the expensive part of authenticating a request,
//! and a client typically sends the same token on every request until it
//! expires. Caching the result turns that into a map lookup.
//!
//! The cache is keyed by the token itself rather than by a digest of it. A
//! non-cryptographic hash would let two different tokens collide onto one
//! entry, which is an authentication bypass, and a cryptographic one costs
//! about what it saves. The token is already in memory for the length of the
//! request either way.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
// `std::time` natively; on wasm32, where `Instant::now()` and
// `SystemTime::now()` panic, the JavaScript host's clock.
use web_time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::{extract_bearer_token, validate_token, AuthResult, JwtConfig, JwtError};

/// How many validated tokens to hold before making room.
///
/// A bound is the point: without one, anything that can present tokens can
/// grow this map until the process dies.
const CAPACITY: usize = 10_000;

struct Entry {
    result: AuthResult,
    expires: Instant,
}

/// A bounded cache of validated tokens.
pub struct JwtCache {
    entries: Mutex<HashMap<String, Entry>>,
    max_lifetime: Duration,
    /// The [`crate::JwtKeys::generation`] every entry was verified under.
    ///
    /// Read and written only while `entries` is locked, so the two change
    /// together; atomic only so that `JwtCache` stays `Sync` without a second
    /// lock. When the keys change, every entry was verified against a set
    /// that is no longer in force -- possibly by a key that has just been
    /// withdrawn -- and they are all forgotten.
    keys_generation: AtomicU64,
}

impl JwtCache {
    /// Build a cache, or `None` if the configuration turns it off.
    ///
    /// A zero lifetime disables it, as in PostgREST -- an entry that expires
    /// the moment it is written is only overhead.
    pub fn new(enabled: bool, max_lifetime_secs: u64) -> Option<Self> {
        if !enabled || max_lifetime_secs == 0 {
            return None;
        }
        Some(Self {
            entries: Mutex::new(HashMap::new()),
            max_lifetime: Duration::from_secs(max_lifetime_secs),
            keys_generation: AtomicU64::new(0),
        })
    }

    /// Forget every token held.
    ///
    /// Done by the cache itself whenever the key set it is used with changes;
    /// for a host that wants to revoke for some other reason.
    pub fn clear(&self) {
        self.entries
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clear();
    }

    /// Authenticate, consulting the cache.
    ///
    /// Falls through to [`crate::authenticate`] for anything not cacheable: a
    /// request with no token at all, and a token already at or past its own
    /// expiry.
    pub fn authenticate(
        &self,
        auth_header: Option<&str>,
        config: &JwtConfig,
    ) -> Result<AuthResult, JwtError> {
        let Some(header) = auth_header else {
            return crate::authenticate(None, config);
        };
        let token = extract_bearer_token(header)?;

        // Read before the token is verified, and checked again when the entry
        // is written: a set replaced in between was not the one it was
        // verified against.
        let generation = config.keys.as_ref().map_or(0, crate::JwtKeys::generation);

        if let Some(hit) = self.get_under(token, generation) {
            return Ok(hit);
        }

        let result = validate_token(token, config)?;

        // Never hold an entry past the token's own expiry: doing so would keep
        // answering with a token the issuer has already ended. `exp` bounds the
        // lifetime, it does not extend it.
        let ttl = match remaining_lifetime(&result) {
            // No `exp` at all. Such a token does not expire, so the configured
            // maximum is the only bound there is.
            Expiry::Absent => self.max_lifetime,
            Expiry::In(remaining) if remaining.is_zero() => return Ok(result),
            Expiry::In(remaining) => self.max_lifetime.min(remaining),
            // An `exp` we cannot read is not the same as no `exp`, and must not
            // fall through to the configured maximum: that would keep serving
            // a token for up to an hour after it expired. Refuse to cache and
            // let every request verify it.
            Expiry::Unreadable => return Ok(result),
        };

        self.insert_under(token.to_string(), result.clone(), ttl, generation);
        Ok(result)
    }

    /// [`Self::get`], first forgetting everything if the keys are no longer
    /// those of `generation`.
    fn get_under(&self, token: &str, generation: u64) -> Option<AuthResult> {
        let mut entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        if self.keys_generation.load(Ordering::Relaxed) != generation {
            entries.clear();
            self.keys_generation.store(generation, Ordering::Relaxed);
        }
        Self::lookup(&mut entries, token)
    }

    /// [`Self::insert`], unless the entries are by now those of another
    /// generation of keys -- the token was verified against a set that has
    /// since been replaced, and is verified again next time.
    ///
    /// Checked under the same lock the entry is written under, so a request
    /// that saw the new keys -- and cleared the entries -- cannot have an
    /// entry from the old ones written after it.
    fn insert_under(&self, token: String, result: AuthResult, ttl: Duration, generation: u64) {
        let mut entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        if self.keys_generation.load(Ordering::Relaxed) != generation {
            return;
        }
        Self::store(&mut entries, token, result, ttl);
    }

    #[cfg(test)]
    fn get(&self, token: &str) -> Option<AuthResult> {
        let mut entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        Self::lookup(&mut entries, token)
    }

    fn lookup(entries: &mut HashMap<String, Entry>, token: &str) -> Option<AuthResult> {
        let entry = entries.get(token)?;
        if entry.expires > Instant::now() {
            return Some(entry.result.clone());
        }
        entries.remove(token);
        None
    }

    #[cfg(test)]
    fn insert(&self, token: String, result: AuthResult, ttl: Duration) {
        let mut entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        Self::store(&mut entries, token, result, ttl);
    }

    fn store(
        entries: &mut HashMap<String, Entry>,
        token: String,
        result: AuthResult,
        ttl: Duration,
    ) {
        let now = Instant::now();

        if entries.len() >= CAPACITY {
            entries.retain(|_, e| e.expires > now);
            // Everything still live and still at capacity. Dropping the lot is
            // crude but bounded, and the cost of a miss is one verification.
            if entries.len() >= CAPACITY {
                entries.clear();
            }
        }

        entries.insert(
            token,
            Entry {
                result,
                expires: now + ttl,
            },
        );
    }
}

/// What the token's own `exp` claim says about how long it has left.
enum Expiry {
    /// No `exp` claim. The token does not expire.
    Absent,
    /// It expires this long from now; zero if it already has.
    In(Duration),
    /// There is an `exp`, and it is not a number this can read. Distinct from
    /// `Absent` on purpose -- see the caller.
    Unreadable,
}

/// Read `exp`, which RFC 7519 defines as a NumericDate and explicitly allows
/// to carry a fraction. `as_i64` alone returns `None` for `1735689600.5`, and
/// treating that as "no expiry" is how an expired token stays cached.
fn remaining_lifetime(result: &AuthResult) -> Expiry {
    let Some(exp) = result.claims.get("exp") else {
        return Expiry::Absent;
    };

    let exp_secs = match exp.as_i64() {
        Some(n) => n,
        // Truncating towards zero is the conservative direction: it can only
        // make the cached lifetime shorter.
        None => match exp.as_f64() {
            Some(f) if f.is_finite() => f.trunc() as i64,
            _ => return Expiry::Unreadable,
        },
    };

    let Ok(now) = SystemTime::now().duration_since(UNIX_EPOCH) else {
        return Expiry::Unreadable;
    };

    Expiry::In(Duration::from_secs(
        exp_secs.saturating_sub(now.as_secs() as i64).max(0) as u64,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn result_with(claims: serde_json::Value) -> AuthResult {
        AuthResult {
            role: "web_user".to_string(),
            claims: claims
                .as_object()
                .unwrap()
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect(),
        }
    }

    #[test]
    fn disabled_by_flag_or_by_zero_lifetime() {
        assert!(JwtCache::new(false, 3600).is_none());
        assert!(JwtCache::new(true, 0).is_none());
        assert!(JwtCache::new(true, 1).is_some());
    }

    #[test]
    fn a_hit_is_returned_and_an_expired_entry_is_not() {
        let cache = JwtCache::new(true, 3600).unwrap();
        let result = result_with(json!({"sub": "alice"}));

        cache.insert("tok".to_string(), result.clone(), Duration::from_secs(60));
        assert_eq!(cache.get("tok").unwrap().role, "web_user");

        // Already expired when written.
        cache.insert("stale".to_string(), result, Duration::from_secs(0));
        assert!(cache.get("stale").is_none());
        // ...and reading it removed it.
        assert!(!cache.entries.lock().unwrap().contains_key("stale"));
    }

    fn now_secs() -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64
    }

    #[test]
    fn exp_bounds_the_lifetime_but_does_not_extend_it() {
        let soon = now_secs() + 30;
        let Expiry::In(remaining) = remaining_lifetime(&result_with(json!({"exp": soon}))) else {
            panic!("an integer exp should be readable");
        };
        assert!(remaining <= Duration::from_secs(30));
        assert!(remaining > Duration::from_secs(25));

        // A token already expired has nothing left, so it is not cached.
        let Expiry::In(none_left) = remaining_lifetime(&result_with(json!({"exp": soon - 600})))
        else {
            panic!("an integer exp should be readable");
        };
        assert!(none_left.is_zero());

        // No `exp` at all is not "forever" -- the caller applies its maximum.
        assert!(matches!(
            remaining_lifetime(&result_with(json!({"sub": "alice"}))),
            Expiry::Absent
        ));
    }

    #[test]
    fn a_fractional_exp_is_read_rather_than_treated_as_absent() {
        // RFC 7519 allows a NumericDate to carry a fraction, and `as_i64`
        // returns None for one. Reading it as `Absent` would give the entry
        // the full configured lifetime and keep serving the token for up to an
        // hour after it expired.
        let soon = now_secs() as f64 + 30.5;
        let Expiry::In(remaining) = remaining_lifetime(&result_with(json!({"exp": soon}))) else {
            panic!("a fractional exp must not read as absent or unreadable");
        };
        assert!(remaining <= Duration::from_secs(31));
        assert!(remaining > Duration::from_secs(25));

        // And one already past still reads as nothing left.
        let past = now_secs() as f64 - 600.5;
        let Expiry::In(none_left) = remaining_lifetime(&result_with(json!({"exp": past}))) else {
            panic!("a fractional exp must not read as absent or unreadable");
        };
        assert!(none_left.is_zero());
    }

    #[test]
    fn an_exp_that_is_not_a_number_is_unreadable_not_absent() {
        // The distinction is the whole point: `Absent` means "does not
        // expire", and applying that to a token that does expire is the bug.
        for value in [json!({"exp": "soon"}), json!({"exp": null})] {
            assert!(
                matches!(
                    remaining_lifetime(&result_with(value.clone())),
                    Expiry::Unreadable
                ),
                "{value:?}"
            );
        }
    }

    #[test]
    fn a_token_with_an_unreadable_exp_is_not_cached() {
        let cache = JwtCache::new(true, 3600).unwrap();
        // Nothing was inserted for it, so a later lookup misses and the token
        // is verified again rather than served from a stale entry.
        assert!(cache.get("tok").is_none());
        assert!(cache.entries.lock().unwrap().is_empty());
    }

    #[test]
    fn the_cache_is_bounded() {
        let cache = JwtCache::new(true, 3600).unwrap();
        let result = result_with(json!({"sub": "alice"}));
        for i in 0..CAPACITY + 50 {
            cache.insert(format!("tok{i}"), result.clone(), Duration::from_secs(600));
        }
        assert!(cache.entries.lock().unwrap().len() <= CAPACITY);
    }
}
