//! Keys fetched from a JWKS URL (`PGRST_JWT_JWKS_URL`), and kept current.
//!
//! An identity provider -- Supabase Auth (GoTrue), Auth0, Clerk, Cognito,
//! Keycloak -- publishes the public keys its tokens are signed with as a JSON
//! Web Key Set at a well-known URL, and rotates them: a new key is published,
//! tokens start being signed with it, and the old one is withdrawn once no
//! token signed by it can still be valid. A server that read the set once, at
//! start-up, would refuse every token signed after the first rotation. So the
//! set is fetched again, on a schedule, and whenever a token names a `kid`
//! this server has never seen.
//!
//! # Where the waiting happens
//!
//! Verifying a token is synchronous ([`crate::validate_token`]), and stays
//! so: the keys a request verifies against are a snapshot taken from
//! [`JwtKeys`], swapped whole when a fetch brings a new set. The only
//! asynchronous step is [`Jwks::ensure_for_token`] (reached through
//! [`crate::ensure_keys`]), which a host awaits *before* the synchronous
//! check. It reads the token's header, and if the snapshot already holds a key
//! the header selects it returns at once without awaiting anything -- the
//! common case, every request but the few after a rotation. Only an unknown
//! `kid`, or keys never loaded, or (on a host with no background task) keys
//! past their refresh time, make it fetch.
//!
//! # Fetching
//!
//! The HTTP request itself is the host's: [`Fetch`] is implemented with
//! reqwest by the server and with the Workers Fetch API by the Worker, so
//! this crate stays free of an HTTP client and builds for wasm32. What is
//! fetched, and whether the answer is trusted, is decided here, once for every
//! host:
//!
//! - **HTTPS only**, unless the host is loopback (`localhost`, `127.0.0.0/8`,
//!   `::1`) -- a development IdP, or a sidecar. A key set fetched over plain
//!   HTTP is a key set anyone on the path can replace with their own, and then
//!   sign any token they like. Checked when the URL is configured, and again
//!   for every redirect.
//! - **No redirect to another host.** The URL the operator configured names
//!   the party trusted to say which keys are valid; a redirect elsewhere is
//!   that party delegating trust in a way nobody reviewed. A redirect on the
//!   same host (`/jwks` to `/jwks/`, http to https on loopback) is followed,
//!   at most [`MAX_REDIRECTS`] times. The [`Fetch`] implementation must not
//!   follow redirects itself, so that this rule is the only one.
//! - **[`FETCH_TIMEOUT`] and [`MAX_JWKS_BYTES`]**, enforced by the [`Fetch`]
//!   implementation, which is the only party that can stop reading: a JWKS
//!   endpoint that hangs or streams forever must not hold a request, or the
//!   refresh task, hostage. Real key sets are a few kilobytes.
//! - The body is parsed with [`KeySet::from_jwk_json`], exactly as a JWKS in
//!   `PGRST_JWT_SECRET` is. A response that is not a 200, or not a set this
//!   server can use, is a failed refresh.
//!
//! # Failing
//!
//! A failed refresh keeps the last good set and logs a warning: an identity
//! provider having a bad minute must not log every user out. If no set has
//! ever loaded, every token the JWKS would have verified is refused -- fail
//! closed -- and [`Jwks::status`] says so, which the server's readiness
//! endpoints report as not ready. Start-up does not fail: an IdP outage that
//! crash-looped every replica would turn a partial outage into a total one.
//!
//! # Refreshing
//!
//! A successful fetch is trusted for its `Cache-Control: max-age`, clamped to
//! [`MIN_MAX_AGE`]..=[`MAX_MAX_AGE`], or for the configured refresh interval
//! (`PGRST_JWT_JWKS_REFRESH`, default [`DEFAULT_REFRESH`]) when the response
//! says nothing. The provider knows how often it rotates; the clamp keeps a
//! `max-age=0` from turning into a fetch per request and a `max-age` of a year
//! from making a withdrawn key valid for a year. A failed fetch is retried
//! after 1 s, 2 s, 4 s... up to [`MAX_RETRY`] (and never later than the
//! refresh interval).
//!
//! A token whose `kid` is unknown triggers one fetch, at most once per
//! [`UNKNOWN_KID_INTERVAL`] -- anyone can mint a token with a random `kid`,
//! and without the limit every such request would be a request to the
//! identity provider. Requests that arrive while that fetch is in flight wait
//! for it rather than each starting their own (natively; a Worker's requests
//! never wait on one another, see `Jwks::exclusive`); if the `kid` is still
//! unknown afterwards the token is refused with PGRST301, as any token is
//! whose key this server does not hold.
//!
//! # With `PGRST_JWT_SECRET`
//!
//! Both may be set, and a token is accepted if either verifies it: the keys
//! in force are the union of the secret's and the fetched set's. That is the
//! deployment where a backend mints HS256 service tokens with a shared secret
//! while users arrive with tokens from the identity provider -- Supabase's own
//! legacy `service_role` key is exactly that. Neither source can widen the
//! other: a key still admits only its own algorithms, and a token naming a
//! `kid` still selects only keys carrying that `kid` (the plain-string secret,
//! which has no `kid`, has always accepted any, and still does).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use jsonwebtoken::Algorithm;
use web_time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::{JwtKeys, KeySet};
use address::Address;

mod address;

/// How long one fetch may take, connecting and reading included.
pub const FETCH_TIMEOUT: Duration = Duration::from_secs(5);

/// The largest JWKS response read, in bytes: 1 MiB.
pub const MAX_JWKS_BYTES: usize = 1 << 20;

/// How many same-host redirects are followed before the fetch is a failure.
pub const MAX_REDIRECTS: usize = 3;

/// How long a fetched set is used without being fetched again, when neither
/// the response nor `PGRST_JWT_JWKS_REFRESH` says otherwise: five minutes.
pub const DEFAULT_REFRESH: Duration = Duration::from_secs(300);

/// The shortest `Cache-Control: max-age` honoured. Shorter, `no-cache` and
/// `no-store` all mean this.
pub const MIN_MAX_AGE: Duration = Duration::from_secs(60);

/// The longest `Cache-Control: max-age` honoured: a day.
pub const MAX_MAX_AGE: Duration = Duration::from_secs(86_400);

/// How often a token with an unknown `kid` may cause a fetch.
pub const UNKNOWN_KID_INTERVAL: Duration = Duration::from_secs(30);

/// The longest wait before retrying a failed fetch.
pub const MAX_RETRY: Duration = Duration::from_secs(30);

/// `Send + Sync` natively, nothing at all on wasm32, where a Worker's
/// JavaScript objects are neither and nothing runs on another thread anyway.
#[cfg(not(target_arch = "wasm32"))]
pub trait MaybeSendSync: Send + Sync {}
#[cfg(not(target_arch = "wasm32"))]
impl<T: Send + Sync + ?Sized> MaybeSendSync for T {}

/// `Send + Sync` natively, nothing at all on wasm32. See the native definition.
#[cfg(target_arch = "wasm32")]
pub trait MaybeSendSync {}
#[cfg(target_arch = "wasm32")]
impl<T: ?Sized> MaybeSendSync for T {}

/// What one `GET` answered.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct FetchResponse {
    /// The HTTP status.
    pub status: u16,
    /// The `Location` header, for a redirect.
    pub location: Option<String>,
    /// The `Cache-Control` header.
    pub cache_control: Option<String>,
    /// The body, no longer than [`MAX_JWKS_BYTES`].
    pub body: Vec<u8>,
}

impl FetchResponse {
    /// A response with this status and body, and no headers.
    pub fn new(status: u16, body: Vec<u8>) -> Self {
        Self {
            status,
            location: None,
            cache_control: None,
            body,
        }
    }

    /// With a `Location` header.
    pub fn with_location(mut self, location: Option<String>) -> Self {
        self.location = location;
        self
    }

    /// With a `Cache-Control` header.
    pub fn with_cache_control(mut self, cache_control: Option<String>) -> Self {
        self.cache_control = cache_control;
        self
    }
}

/// A fetch or a refresh that failed, and why, for the log.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct FetchError(String);

impl FetchError {
    /// A failure described by `reason`.
    pub fn new(reason: impl Into<String>) -> Self {
        Self(reason.into())
    }
}

/// An HTTP client, as much of one as fetching a JWKS needs.
///
/// Implementations must, so that the rules in the [module documentation](self)
/// hold on every host:
///
/// - **not follow redirects**, but answer a 3xx with its `Location`;
/// - give up after [`FETCH_TIMEOUT`];
/// - stop reading, and fail, once a body exceeds [`MAX_JWKS_BYTES`].
#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
pub trait Fetch: MaybeSendSync {
    /// `GET` an absolute `http` or `https` URL this module has already
    /// checked.
    async fn get(&self, url: &str) -> Result<FetchResponse, FetchError>;
}

/// A copy of the fetched set somewhere other processes can read it.
///
/// For a host whose processes come and go faster than the set changes: a
/// Cloudflare Worker keeps it in KV, so that an isolate starting cold reads
/// what another fetched a minute ago instead of asking the identity provider
/// again. A stored copy is used only while it is fresh, and never to answer
/// an unknown `kid` -- that question is only answered by the provider.
#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
pub trait JwksStore: MaybeSendSync {
    /// The stored copy for `url`, if there is one.
    async fn load(&self, url: &str) -> Option<StoredJwks>;
    /// Store `jwks` for `url`. It may be dropped once `ttl` has passed.
    /// Failing is not an error worth more than a log line.
    async fn save(&self, url: &str, jwks: &StoredJwks, ttl: Duration);
}

/// A fetched set, as a [`JwksStore`] keeps it.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct StoredJwks {
    /// The JWKS document, as fetched.
    pub body: String,
    /// Until when it may be used without fetching again, in seconds since the
    /// Unix epoch.
    pub fresh_until: u64,
}

/// What a [`Jwks`] has done so far: the numbers a metric or a readiness check
/// reads. A snapshot, taken by [`Jwks::status`].
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct JwksStatus {
    /// Whether a set has ever been fetched. Until it has, every token the
    /// JWKS would have verified is refused.
    pub loaded: bool,
    /// How many keys the fetched set holds.
    pub keys: usize,
    /// Successful fetches, a fresh copy read from a [`JwksStore`] included.
    pub refreshes: u64,
    /// Failed fetches.
    pub failures: u64,
    /// Failed fetches since the last successful one.
    pub consecutive_failures: u32,
    /// How many times a fetch brought a set different from the one held.
    pub changes: u64,
    /// When the last successful fetch was.
    pub last_success: Option<SystemTime>,
    /// How long ago the set held was last confirmed by a successful fetch.
    pub age: Option<Duration>,
    /// How long a fetched set is used for: the last response's `max-age`,
    /// clamped, or the configured interval.
    pub refresh_interval: Duration,
    /// How long until the next fetch is due.
    pub next_refresh_in: Duration,
    /// Why the last fetch failed, while it is the most recent outcome.
    pub last_error: Option<String>,
}

/// A JWKS URL that cannot be fetched from.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct JwksUrlError(String);

/// Check a JWKS URL against the rules in the [module documentation](self):
/// absolute, `https`, or `http` to a loopback host.
pub fn check_url(url: &str) -> Result<(), JwksUrlError> {
    parse_url(url).map(drop)
}

fn parse_url(url: &str) -> Result<Address, JwksUrlError> {
    let parsed = Address::parse(url).map_err(|e| JwksUrlError(format!("{url:?}: {e}")))?;
    check_parsed(&parsed)?;
    Ok(parsed)
}

fn check_parsed(url: &Address) -> Result<(), JwksUrlError> {
    if url.is_https() || url.is_loopback() {
        return Ok(());
    }
    Err(JwksUrlError(format!(
        "{url}: a key set fetched over plain HTTP can be replaced by anyone on the path; \
         use https (http is accepted only for localhost, 127.0.0.0/8 and [::1])"
    )))
}

/// How long a response says it may be used for: its `max-age`, clamped, or
/// [`MIN_MAX_AGE`] for `no-cache` and `no-store`; `None` when it says nothing.
fn max_age(cache_control: Option<&str>) -> Option<Duration> {
    let mut found = None;
    for directive in cache_control?.split(',') {
        let directive = directive.trim();
        let (name, value) = directive.split_once('=').unwrap_or((directive, ""));
        match name.trim().to_ascii_lowercase().as_str() {
            "no-cache" | "no-store" => return Some(MIN_MAX_AGE),
            "max-age" => {
                if let Ok(secs) = value.trim().trim_matches('"').parse::<u64>() {
                    found = Some(Duration::from_secs(secs));
                }
            }
            _ => {}
        }
    }
    found.map(|age| age.clamp(MIN_MAX_AGE, MAX_MAX_AGE))
}

/// How long to wait after the `failures`th failure in a row.
fn retry_after(failures: u32, interval: Duration) -> Duration {
    let backoff = Duration::from_secs(1u64 << failures.saturating_sub(1).min(5));
    backoff.min(MAX_RETRY).min(interval)
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Builds a [`Jwks`]; see [`Jwks::builder`].
pub struct JwksBuilder {
    url: String,
    fetch: Box<dyn Fetch>,
    fixed: Option<Arc<KeySet>>,
    refresh: Duration,
    unknown_kid_interval: Duration,
    store: Option<Box<dyn JwksStore>>,
}

impl JwksBuilder {
    /// Accept the keys of `PGRST_JWT_SECRET` as well as the fetched ones.
    /// See "With `PGRST_JWT_SECRET`" in the [module documentation](self).
    pub fn fixed(mut self, keys: Option<&JwtKeys>) -> Self {
        self.fixed = keys.map(JwtKeys::current);
        self
    }

    /// How long a set is used when the response carries no `max-age`
    /// (`PGRST_JWT_JWKS_REFRESH`). Default [`DEFAULT_REFRESH`]; zero is
    /// read as one second.
    pub fn refresh(mut self, interval: Duration) -> Self {
        self.refresh = interval.max(Duration::from_secs(1));
        self
    }

    /// How often an unknown `kid` may cause a fetch. Default
    /// [`UNKNOWN_KID_INTERVAL`]; for tests.
    pub fn unknown_kid_interval(mut self, interval: Duration) -> Self {
        self.unknown_kid_interval = interval;
        self
    }

    /// Keep a copy of every fetched set in `store`, and read a fresh one from
    /// there before fetching.
    pub fn store(mut self, store: impl JwksStore + 'static) -> Self {
        self.store = Some(Box::new(store));
        self
    }

    /// The [`Jwks`], or the reason its URL cannot be fetched from.
    pub fn build(self) -> Result<Jwks, JwksUrlError> {
        let url = parse_url(&self.url)?;
        let keys = JwtKeys::new(match &self.fixed {
            Some(fixed) => KeySet::union(fixed, &KeySet::empty()),
            None => KeySet::empty(),
        });
        // On wasm32 the fetcher is a JavaScript object and `Inner` is not
        // `Send`; nothing there runs on another thread, and one type for both
        // targets is worth more than the `Rc` the lint suggests.
        #[cfg_attr(target_arch = "wasm32", allow(clippy::arc_with_non_send_sync))]
        Ok(Jwks {
            inner: Arc::new(Inner {
                url_text: url.to_string(),
                url,
                fetch: self.fetch,
                store: self.store,
                keys,
                fixed: self.fixed,
                unknown_kid_interval: self.unknown_kid_interval,
                background: AtomicBool::new(false),
                state: Mutex::new(State {
                    body: None,
                    keys: 0,
                    last_success: None,
                    interval: self.refresh,
                    configured: self.refresh,
                    next_due: Instant::now(),
                    consecutive_failures: 0,
                    refreshes: 0,
                    failures: 0,
                    changes: 0,
                    last_error: None,
                    last_unknown_kid: None,
                }),
                refreshing: futures::lock::Mutex::new(()),
            }),
        })
    }
}

/// The keys at a JWKS URL, fetched and kept current. See the
/// [module documentation](self).
///
/// Cloning shares it. [`Jwks::keys`] is the handle a [`crate::JwtConfig`]
/// verifies with; [`crate::JwtConfig::with_jwks`] sets both.
#[derive(Clone)]
pub struct Jwks {
    inner: Arc<Inner>,
}

struct Inner {
    url: Address,
    /// `url`, as text, for [`Jwks::url`].
    url_text: String,
    fetch: Box<dyn Fetch>,
    store: Option<Box<dyn JwksStore>>,
    /// The keys in force: the fixed ones and the last good fetched set.
    keys: JwtKeys,
    fixed: Option<Arc<KeySet>>,
    unknown_kid_interval: Duration,
    /// Whether a background task keeps the set fresh. When none does, a
    /// request finding the set due for a refresh does it.
    background: AtomicBool,
    state: Mutex<State>,
    /// Held for the length of a fetch, so that only one runs at a time and a
    /// request needing its outcome can wait for it (natively; see
    /// [`Jwks::exclusive`]). An async lock with no runtime behind it: the
    /// futures crate's.
    refreshing: futures::lock::Mutex<()>,
}

struct State {
    /// The document the set in force was parsed from. A fetch bringing the
    /// same bytes changes nothing -- in particular, it does not empty the
    /// token cache.
    body: Option<String>,
    keys: usize,
    last_success: Option<(Instant, SystemTime)>,
    interval: Duration,
    configured: Duration,
    next_due: Instant,
    consecutive_failures: u32,
    refreshes: u64,
    failures: u64,
    changes: u64,
    last_error: Option<String>,
    last_unknown_kid: Option<Instant>,
}

impl std::fmt::Debug for Jwks {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Jwks")
            .field("url", &self.inner.url_text)
            .field("keys", &self.inner.keys)
            .finish_non_exhaustive()
    }
}

/// Why a fetch is being made, which decides whether a stored copy will do.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Reason {
    /// Due, or never loaded: a fresh stored copy is as good as a fetch.
    AnySource,
    /// A `kid` nobody here has seen, or a schedule writing the store for
    /// everyone else. Only the provider can answer.
    ProviderOnly,
}

impl Jwks {
    /// Start building one for `url`, fetched through `fetch`.
    pub fn builder(url: impl Into<String>, fetch: impl Fetch + 'static) -> JwksBuilder {
        JwksBuilder {
            url: url.into(),
            fetch: Box::new(fetch),
            fixed: None,
            refresh: DEFAULT_REFRESH,
            unknown_kid_interval: UNKNOWN_KID_INTERVAL,
            store: None,
        }
    }

    /// The keys in force, for [`crate::JwtConfig::keys`].
    pub fn keys(&self) -> &JwtKeys {
        &self.inner.keys
    }

    /// The URL the keys are fetched from.
    pub fn url(&self) -> &str {
        &self.inner.url_text
    }

    /// Whether a set has ever been fetched.
    pub fn is_loaded(&self) -> bool {
        self.state().last_success.is_some()
    }

    /// What has happened so far.
    pub fn status(&self) -> JwksStatus {
        let state = self.state();
        let now = Instant::now();
        JwksStatus {
            loaded: state.last_success.is_some(),
            keys: state.keys,
            refreshes: state.refreshes,
            failures: state.failures,
            consecutive_failures: state.consecutive_failures,
            changes: state.changes,
            last_success: state.last_success.map(|(_, at)| at),
            age: state
                .last_success
                .map(|(at, _)| now.saturating_duration_since(at)),
            refresh_interval: state.interval,
            next_refresh_in: state.next_due.saturating_duration_since(now),
            last_error: state.last_error.clone(),
        }
    }

    /// How long until the next fetch is due.
    pub fn next_refresh_in(&self) -> Duration {
        self.state()
            .next_due
            .saturating_duration_since(Instant::now())
    }

    /// Fetch the set now, whenever it was last fetched, waiting for any fetch
    /// already in flight first. The outcome is also in [`Jwks::status`].
    ///
    /// Always from the provider, never from a [`JwksStore`]: this is what a
    /// schedule calls, and what writes the store for everyone else.
    pub async fn refresh(&self) -> Result<(), FetchError> {
        let _fetching = self.exclusive().await;
        self.fetch_and_install(Reason::ProviderOnly).await
    }

    /// Keep the set fresh until the end of time: fetch whenever the next
    /// fetch is due, sleeping with `sleep` in between.
    ///
    /// For a host that runs background tasks -- the server spawns this on its
    /// runtime with `tokio::time::sleep`. Once it runs, requests no longer
    /// refresh a set that is merely due; they still fetch for an unknown
    /// `kid`.
    pub async fn refresh_forever<S, F>(&self, mut sleep: S)
    where
        S: FnMut(Duration) -> F,
        F: std::future::Future<Output = ()>,
    {
        self.inner.background.store(true, Ordering::Relaxed);
        loop {
            sleep(self.next_refresh_in()).await;
            // The outcome is logged and kept in the status; the schedule
            // follows from it either way.
            let _ = self.refresh().await;
        }
    }

    /// Make sure the keys a token needs are here, fetching if they may not
    /// be. Never fails: a token still without a key is refused by the
    /// synchronous check that follows, as PGRST301.
    ///
    /// Returns without awaiting anything when the set in force holds a key
    /// for the token's header -- unless no background task keeps the set
    /// fresh and it is due, when the request that notices fetches it (and
    /// one that finds a fetch already running goes on with the set it has).
    pub async fn ensure_for_token(&self, token: &str) {
        // A header that does not parse is refused by the check that follows,
        // and no fetch could change that.
        let Ok(header) = jsonwebtoken::decode_header(token) else {
            return;
        };
        let kid = header.kid.as_deref();
        if self.inner.keys.current().selects(kid, header.alg) {
            if !self.inner.background.load(Ordering::Relaxed) && self.due() {
                self.refresh_if_due().await;
            }
            return;
        }
        self.refresh_for(kid, header.alg).await;
    }

    /// The right to fetch, waiting for a fetch already in flight to finish.
    ///
    /// Natively, a request that needs a fetch's outcome waits for the one in
    /// flight rather than starting its own. On wasm32 it never waits: in a
    /// Worker every request is its own I/O context, and a request woken by
    /// another's fetch completing would go on running -- and doing its own
    /// I/O -- inside the other's context, which the Workers runtime refuses
    /// and can kill the isolate for. So there a request that finds a fetch in
    /// flight makes its own (the unknown-`kid` limit still applies, and a
    /// cold isolate's fetches are usually reads of the [`JwksStore`]).
    async fn exclusive(&self) -> Option<futures::lock::MutexGuard<'_, ()>> {
        #[cfg(not(target_arch = "wasm32"))]
        return Some(self.inner.refreshing.lock().await);
        #[cfg(target_arch = "wasm32")]
        return self.inner.refreshing.try_lock();
    }

    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        self.inner.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn due(&self) -> bool {
        Instant::now() >= self.state().next_due
    }

    /// A lazy host's scheduled refresh, done by the request that noticed it
    /// was due. One request does it; the rest go on with the set they have.
    async fn refresh_if_due(&self) {
        let Some(_fetching) = self.inner.refreshing.try_lock() else {
            return;
        };
        if self.due() {
            let _ = self.fetch_and_install(Reason::AnySource).await;
        }
    }

    /// A token selects no key: fetch, unless one was fetched for the same
    /// reason too recently, and wait for the outcome.
    async fn refresh_for(&self, kid: Option<&str>, alg: Algorithm) {
        let _fetching = self.exclusive().await;
        // Whoever held the lock may have brought the key.
        if self.inner.keys.current().selects(kid, alg) {
            return;
        }
        let reason = {
            let mut state = self.state();
            if state.last_success.is_none() && Instant::now() >= state.next_due {
                // Never loaded, and due to be tried again: the schedule
                // already rate-limits this, so it does not spend the
                // unknown-kid allowance.
                Reason::AnySource
            } else {
                let now = Instant::now();
                if let Some(last) = state.last_unknown_kid {
                    if now.saturating_duration_since(last) < self.inner.unknown_kid_interval {
                        return;
                    }
                }
                state.last_unknown_kid = Some(now);
                Reason::ProviderOnly
            }
        };
        let _ = self.fetch_and_install(reason).await;
    }

    /// Fetch -- or read a fresh stored copy, where `reason` allows -- and
    /// install what comes back, keeping the books either way. The caller
    /// holds `refreshing`.
    async fn fetch_and_install(&self, reason: Reason) -> Result<(), FetchError> {
        if reason == Reason::AnySource {
            if let Some(store) = &self.inner.store {
                if let Some(stored) = store.load(self.url()).await {
                    let now = unix_now();
                    if stored.fresh_until > now && self.install(&stored.body).is_ok() {
                        self.succeeded(Duration::from_secs(stored.fresh_until - now));
                        return Ok(());
                    }
                }
            }
        }

        let fetched = self.download().await.and_then(|(body, max_age)| {
            self.install(&body)?;
            Ok((body, max_age))
        });
        match fetched {
            Ok((body, max_age)) => {
                let interval = max_age.unwrap_or(self.state().configured);
                self.succeeded(interval);
                if let Some(store) = &self.inner.store {
                    let stored = StoredJwks {
                        body,
                        fresh_until: unix_now() + interval.as_secs(),
                    };
                    store.save(self.url(), &stored, interval).await;
                }
                Ok(())
            }
            Err(e) => {
                let loaded = {
                    let mut state = self.state();
                    state.failures += 1;
                    state.consecutive_failures += 1;
                    state.last_error = Some(e.to_string());
                    state.next_due =
                        Instant::now() + retry_after(state.consecutive_failures, state.interval);
                    state.last_success.is_some()
                };
                if loaded {
                    tracing::warn!(
                        "PGRST_JWT_JWKS_URL: refreshing the keys from {} failed, keeping the \
                         ones fetched before: {e}",
                        self.url()
                    );
                } else {
                    tracing::warn!(
                        "PGRST_JWT_JWKS_URL: fetching the keys from {} failed, and no keys \
                         from it have loaded yet, so tokens signed by them are refused: {e}",
                        self.url()
                    );
                }
                Err(e)
            }
        }
    }

    fn succeeded(&self, interval: Duration) {
        let mut state = self.state();
        let now = Instant::now();
        state.last_success = Some((now, SystemTime::now()));
        state.refreshes += 1;
        state.consecutive_failures = 0;
        state.last_error = None;
        state.interval = interval;
        state.next_due = now + interval;
    }

    /// Parse `body` and put it in force, unless it is what is in force
    /// already. Parsing only when it changed also keeps the warnings about
    /// keys set aside to one per change rather than one per refresh.
    fn install(&self, body: &str) -> Result<(), FetchError> {
        if self.state().body.as_deref() == Some(body) {
            return Ok(());
        }
        let fetched = KeySet::from_jwk_json(body)
            .map_err(|e| FetchError::new(format!("the response is not a usable JWKS: {e}")))?;
        let count = fetched.len();
        let set = match &self.inner.fixed {
            Some(fixed) => KeySet::union(fixed, &fetched),
            None => fetched,
        };
        let mut state = self.state();
        self.inner.keys.replace(set);
        state.body = Some(body.to_string());
        state.keys = count;
        state.changes += 1;
        tracing::info!(
            "PGRST_JWT_JWKS_URL: {count} key(s) in force from {}",
            self.inner.url
        );
        Ok(())
    }

    /// `GET` the URL, following same-host redirects, and return the body and
    /// how long it says it may be used for.
    async fn download(&self) -> Result<(String, Option<Duration>), FetchError> {
        let mut url = self.inner.url.clone();
        for _ in 0..=MAX_REDIRECTS {
            let response = self.inner.fetch.get(&url.to_string()).await?;
            match response.status {
                200 => {
                    if response.body.len() > MAX_JWKS_BYTES {
                        return Err(FetchError::new(format!(
                            "the response is larger than {MAX_JWKS_BYTES} bytes"
                        )));
                    }
                    let body = String::from_utf8(response.body)
                        .map_err(|_| FetchError::new("the response is not UTF-8"))?;
                    return Ok((body, max_age(response.cache_control.as_deref())));
                }
                301 | 302 | 303 | 307 | 308 => {
                    let location = response.location.ok_or_else(|| {
                        FetchError::new(format!("HTTP {} with no Location", response.status))
                    })?;
                    let next = url.join(&location).map_err(|e| {
                        FetchError::new(format!("a redirect to {location:?} is not followed: {e}"))
                    })?;
                    if next.host() != url.host() {
                        return Err(FetchError::new(format!(
                            "a redirect to another host ({next}) is not followed"
                        )));
                    }
                    check_parsed(&next).map_err(|e| FetchError::new(e.to_string()))?;
                    url = next;
                }
                status => return Err(FetchError::new(format!("HTTP {status} from {url}"))),
            }
        }
        Err(FetchError::new(format!(
            "more than {MAX_REDIRECTS} redirects"
        )))
    }
}

#[cfg(test)]
mod tests;
