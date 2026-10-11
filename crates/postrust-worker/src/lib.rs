//! Postrust's REST API in a Cloudflare Worker, reaching PostgreSQL through
//! Hyperdrive.
//!
//! The request pipeline is `postrust-engine`, the same one the server runs, and
//! the database driver is `postrust_core::db::PgSocketDb`: the PostgreSQL wire
//! protocol over a `worker::Socket` to Hyperdrive. That driver is held to
//! answering exactly as the server's sqlx driver does. Every request runs in a
//! transaction that sets the token's role and claims, as on the server, so
//! row-level security applies the same way.
//!
//! What is different here, and why:
//!
//! * **One connection per request.** A Worker may not share an I/O object
//!   between requests. Hyperdrive pools connections on its side, so opening
//!   one costs little.
//! * **The schema cache is kept in Workers KV** (`POSTRUST_SCHEMA_CACHE`), so
//!   a cold isolate reads it rather than querying the catalog. It is loaded
//!   live on a miss, refreshed by a Cron Trigger or by an authenticated
//!   `POST /_/reload`, and re-read from KV once `PGRST_WORKER_SCHEMA_TTL`
//!   seconds (default 60) have passed.
//! * **Rows are capped** at `PGRST_DB_MAX_ROWS`, 1000 unless set, because an
//!   isolate has 128 MB. `PGRST_DB_MAX_ROWS=unlimited` lifts the cap.
//! * **REST only.** No GraphQL, no subscriptions, no `LISTEN`.
//!
//! The REST API answers at the root, as PostgREST does: `/todos`, `/rpc/f`.
//! Configuration is the server's own variables (`PGRST_DB_SCHEMAS`,
//! `PGRST_DB_ANON_ROLE`, `PGRST_JWT_SECRET`, ...), set as Worker variables or
//! secrets. See docs/workers.md.
//!
//! The crate is on its own `0.x` line. See docs/stability.md.

#![cfg(target_arch = "wasm32")]

use bytes::Bytes;
use http::{HeaderValue, Method, Response, StatusCode};
use http_body_util::{BodyExt, Full};
use postrust_auth::{JwtCache, JwtConfig};
use postrust_core::db::tokio_postgres::Config;
use postrust_core::db::{BoxFuture, Connector, PgSocketDb};
use postrust_core::{AppConfig, SchemaCache};
use std::cell::RefCell;
use std::rc::Rc;
use worker::postgres_tls::PassthroughTls;
use worker::wasm_bindgen::{JsCast, JsValue};
use worker::{
    event, Context, Env, HttpRequest, ScheduleContext, ScheduledEvent, SecureTransport, Socket,
};

/// The Hyperdrive binding requests reach PostgreSQL through.
const HYPERDRIVE: &str = "HYPERDRIVE";
/// The KV namespace the schema cache is kept in. Optional.
const SCHEMA_KV: &str = "POSTRUST_SCHEMA_CACHE";
/// The secret a `POST /_/reload` must carry as a bearer token. Without it the
/// endpoint does not exist.
const RELOAD_SECRET: &str = "POSTRUST_RELOAD_SECRET";
/// The row ceiling when `PGRST_DB_MAX_ROWS` is not set. An isolate has 128 MB,
/// and an unbounded read of a large table is the quickest way to run out.
const DEFAULT_MAX_ROWS: i64 = 1000;
/// How long an isolate trusts its copy of the schema cache before looking in
/// KV for a newer one, unless `PGRST_WORKER_SCHEMA_TTL` says otherwise.
const DEFAULT_SCHEMA_TTL_SECS: f64 = 60.0;

type Body = Full<Bytes>;

/// A request body already read: its bytes, or the error reading it met.
type ReadBody = http_body_util::Either<
    Full<Bytes>,
    http_body_util::StreamBody<
        futures::stream::Once<futures::future::Ready<Result<http_body::Frame<Bytes>, String>>>,
    >,
>;

/// A panic aborts the isolate, and every request in it. The hook at least
/// leaves the reason in the Workers log.
#[event(start)]
fn start() {
    console_error_panic_hook::set_once();
}

#[event(fetch)]
async fn fetch(req: HttpRequest, env: Env, _ctx: Context) -> worker::Result<Response<Body>> {
    let config = match config(&env) {
        Ok(config) => config,
        Err(e) => return Ok(plain(StatusCode::INTERNAL_SERVER_ERROR, &e)),
    };
    let path = req.uri().path().to_string();
    let response = match (req.method(), path.as_str()) {
        (&Method::GET, "/_/health") => plain(StatusCode::OK, "OK"),
        (&Method::POST, "/_/reload") => reload(&req, &env, &config).await,
        _ => rest(req, &env, config).await,
    };
    Ok(response)
}

#[event(scheduled)]
async fn scheduled(_event: ScheduledEvent, env: Env, _ctx: ScheduleContext) {
    if let Ok(config) = config(&env) {
        if let Err(e) = refresh(&env, &config).await {
            worker::console_error!("postrust: scheduled schema reload failed: {e}");
        }
    }
}

/// A request to the REST API.
async fn rest(req: HttpRequest, env: &Env, config: AppConfig) -> Response<Body> {
    let verbatim = config.compat_mode;
    let state = match State::new(env, config).await {
        Ok(state) => state,
        Err(e) => return postrust_engine::error_response(e, verbatim).map(Body::new),
    };
    let origin = req.headers().get(http::header::ORIGIN).cloned();

    if req.method() == Method::OPTIONS {
        return preflight(&state, req, origin).await;
    }

    let (parts, body) = req.into_parts();
    let body = read_body(body, state.config.max_body_size).await;
    let req = http::Request::from_parts(parts, body);
    let (method, path) = (req.method().clone(), req.uri().path().to_string());
    let response = postrust_engine::handle(&state, req).await;
    state.db.connector().closed().await;
    // The engine logs through `tracing`, which nothing collects in a Worker.
    // A failure on this side is worth a line in the Workers console; the body
    // has the detail the configuration allows a client to see.
    if response.status().is_server_error() {
        worker::console_error!(
            "postrust: {method} {path} answered {}: {}",
            response.status(),
            String::from_utf8_lossy(response.body())
        );
    }
    cors(
        &state.config,
        origin.as_ref(),
        response.map(Body::new),
        false,
    )
}

/// The request body, read to its end before the engine sees it.
///
/// The engine reads a body only once the token has been checked, and not at
/// all when it is refused. On a server that leaves nothing behind. In a Worker
/// an unread request stream outlives the response, is touched afterwards
/// ("Can't read from request stream after response has been sent"), and the
/// exception takes the isolate down -- with whichever request it is serving
/// next. So the stream is always drained here, and the engine is handed what
/// it would have read: the bytes, or past `limit`, the error it would have
/// met, which it still reports only after the token check.
async fn read_body(mut body: worker::Body, limit: usize) -> ReadBody {
    use http_body_util::Either;

    let mut collected = bytes::BytesMut::new();
    let mut failure = None;
    while let Some(frame) = body.frame().await {
        match frame {
            Ok(frame) => {
                if let Ok(data) = frame.into_data() {
                    if failure.is_none() {
                        if collected.len() + data.len() > limit {
                            // What `http_body_util::Limited` says, so the
                            // client is told what the server tells it.
                            failure = Some("length limit exceeded".to_string());
                        } else {
                            collected.extend_from_slice(&data);
                        }
                    }
                }
            }
            Err(e) => {
                failure.get_or_insert_with(|| e.to_string());
                break;
            }
        }
    }
    match failure {
        None => Either::Left(Full::new(collected.freeze())),
        Some(e) => Either::Right(http_body_util::StreamBody::new(futures::stream::once(
            futures::future::ready(Err(e)),
        ))),
    }
}

/// An `OPTIONS`: what the resource allows, with the CORS answer the server's
/// layer would give.
async fn preflight(state: &State, req: HttpRequest, origin: Option<HeaderValue>) -> Response<Body> {
    let (parts, _) = req.into_parts();
    let probe = http::Request::from_parts(parts, Bytes::new());
    let allowed = postrust_engine::allow(state, probe).await;
    state.db.connector().closed().await;
    let response = match allowed {
        Ok(allow) => {
            let mut response = Response::new(Body::default());
            if let Ok(value) = HeaderValue::from_str(&allow) {
                response.headers_mut().insert(http::header::ALLOW, value);
            }
            response
        }
        Err(e) => postrust_engine::error_response(e, state.config.compat_mode).map(Body::new),
    };
    cors(&state.config, origin.as_ref(), response, true)
}

/// The headers tower-http's `CorsLayer` adds on the server, configured as the
/// server configures it: any or the listed origins, the seven methods, any
/// request header, every response header exposed.
fn cors(
    config: &AppConfig,
    origin: Option<&HeaderValue>,
    mut response: Response<Body>,
    preflight: bool,
) -> Response<Body> {
    let headers = response.headers_mut();
    let allowed = match &config.server_cors_origins[..] {
        [] => Some(HeaderValue::from_static("*")),
        list => origin
            .filter(|o| {
                list.iter()
                    .any(|allowed| allowed.as_bytes() == o.as_bytes())
            })
            .cloned(),
    };
    if let Some(allowed) = allowed {
        headers.insert(http::header::ACCESS_CONTROL_ALLOW_ORIGIN, allowed);
    }
    if preflight {
        headers.insert(
            http::header::ACCESS_CONTROL_ALLOW_METHODS,
            HeaderValue::from_static("GET,POST,PUT,PATCH,DELETE,OPTIONS,HEAD"),
        );
        headers.insert(
            http::header::ACCESS_CONTROL_ALLOW_HEADERS,
            HeaderValue::from_static("*"),
        );
    } else {
        headers.insert(
            http::header::ACCESS_CONTROL_EXPOSE_HEADERS,
            HeaderValue::from_static("*"),
        );
    }
    headers.append(
        http::header::VARY,
        HeaderValue::from_static(
            "origin, access-control-request-method, access-control-request-headers",
        ),
    );
    response
}

/// `POST /_/reload`: load the schema cache from the catalog now, for every
/// isolate, rather than waiting for the cron or the TTL.
async fn reload(req: &HttpRequest, env: &Env, config: &AppConfig) -> Response<Body> {
    let Ok(secret) = env.secret(RELOAD_SECRET).map(|s| s.to_string()) else {
        return plain(StatusCode::NOT_FOUND, "Not Found");
    };
    let presented = req
        .headers()
        .get(http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .unwrap_or_default();
    if !constant_time_eq(presented.as_bytes(), secret.as_bytes()) {
        return plain(StatusCode::UNAUTHORIZED, "Unauthorized");
    }
    match refresh(env, config).await {
        Ok(()) => Response::builder()
            .status(StatusCode::NO_CONTENT)
            .body(Body::default())
            .unwrap_or_default(),
        Err(e) => plain(StatusCode::SERVICE_UNAVAILABLE, &e.to_string()),
    }
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

fn plain(status: StatusCode, text: &str) -> Response<Body> {
    Response::builder()
        .status(status)
        .header(http::header::CONTENT_TYPE, "text/plain; charset=utf-8")
        .body(Body::new(Bytes::copy_from_slice(text.as_bytes())))
        .unwrap_or_default()
}

/// The configuration, from the Worker's variables and secrets -- the server's
/// own names, read from bindings rather than a process environment.
fn config(env: &Env) -> Result<AppConfig, String> {
    let env: &JsValue = env.as_ref();
    let pairs = js_sys::Object::entries(env.unchecked_ref())
        .iter()
        .filter_map(|entry| {
            let entry: js_sys::Array = entry.into();
            Some((entry.get(0).as_string()?, entry.get(1).as_string()?))
        })
        .collect::<Vec<_>>();
    // `unlimited` is how a deployment that has thought about it says so; the
    // server would warn about the word and ignore it, which here would leave
    // the default ceiling in place.
    let uncapped = pairs
        .iter()
        .any(|(k, v)| k == "PGRST_DB_MAX_ROWS" && v.trim().eq_ignore_ascii_case("unlimited"));
    let pairs = pairs
        .into_iter()
        .filter(|(k, _)| !(uncapped && k == "PGRST_DB_MAX_ROWS"));
    let mut config = AppConfig::try_from_vars(pairs).map_err(|e| e.to_string())?;
    if config.db_max_rows.is_none() && !uncapped {
        config.db_max_rows = Some(DEFAULT_MAX_ROWS);
    }
    Ok(config)
}

/// Hyperdrive, as a socket per transaction.
///
/// It keeps track of the connections it spawns, because a Worker may do I/O
/// only on behalf of the request in progress. A connection left running when
/// its request has answered -- one whose transaction was dropped after a
/// database error, say -- is polled later, during some other request, and
/// workerd kills the isolate for it, taking that request down with it.
/// [`Hyperdrive::closed`] waits for every connection to close, and the
/// request awaits it before it answers.
struct Hyperdrive {
    host: String,
    port: u16,
    open: RefCell<Vec<futures::channel::oneshot::Receiver<()>>>,
}

impl Hyperdrive {
    /// Every connection this request opened, closed. A connection closes once
    /// its transaction is committed or dropped, which by the time a response
    /// exists they all are.
    async fn closed(&self) {
        let open = std::mem::take(&mut *self.open.borrow_mut());
        for connection in open {
            let _ = connection.await;
        }
    }
}

impl Connector for Hyperdrive {
    type Stream = Socket;
    type Tls = PassthroughTls;

    async fn connect(&self) -> std::io::Result<Socket> {
        Socket::builder()
            .secure_transport(SecureTransport::StartTls)
            .connect(self.host.clone(), self.port)
            .map_err(|e| std::io::Error::other(e.to_string()))
    }

    fn tls(&self) -> PassthroughTls {
        PassthroughTls
    }

    fn spawn(&self, connection: BoxFuture<'static, ()>) {
        let (done, closed) = futures::channel::oneshot::channel();
        self.open.borrow_mut().push(closed);
        wasm_bindgen_futures::spawn_local(async move {
            connection.await;
            let _ = done.send(());
        });
    }
}

fn database(env: &Env) -> Result<PgSocketDb<Hyperdrive>, postrust_core::Error> {
    let unavailable = |e: String| postrust_core::Error::ConnectionPool(e);
    let hyperdrive = env
        .hyperdrive(HYPERDRIVE)
        .map_err(|e| unavailable(format!("no Hyperdrive binding named {HYPERDRIVE}: {e}")))?;
    let config: Config = hyperdrive
        .connection_string()
        .parse()
        .map_err(|e| unavailable(format!("Hyperdrive's connection string: {e}")))?;
    let connector = Hyperdrive {
        host: hyperdrive.host(),
        port: hyperdrive.port(),
        open: RefCell::default(),
    };
    Ok(PgSocketDb::new(connector, config))
}

/// What an isolate keeps between requests.
#[derive(Default)]
struct Isolate {
    schema: Option<Schema>,
    jwt_cache: Option<Rc<JwtCache>>,
}

/// A schema cache, and when this isolate last made sure it was current.
#[derive(Clone)]
struct Schema {
    cache: Rc<SchemaCache>,
    key: String,
    loaded_at: f64,
    checked_at: f64,
}

thread_local! {
    static ISOLATE: RefCell<Isolate> = RefCell::default();
}

/// The KV record a schema cache is kept as.
#[derive(serde::Serialize, serde::Deserialize)]
struct Stored {
    /// When it was loaded from the catalog, in milliseconds since the epoch.
    loaded_at: f64,
    cache: SchemaCache,
}

/// The KV key for a configuration's schema cache: which schemas it describes
/// and which build wrote it, since another version may shape it differently.
fn schema_key(config: &AppConfig) -> String {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    config.db_schemas.hash(&mut hasher);
    config.db_extra_search_path.hash(&mut hasher);
    format!(
        "postrust/{}/schema-cache/{:016x}",
        env!("CARGO_PKG_VERSION"),
        hasher.finish()
    )
}

fn ttl_ms(env: &Env) -> f64 {
    env.var("PGRST_WORKER_SCHEMA_TTL")
        .ok()
        .and_then(|v| v.to_string().trim().parse::<f64>().ok())
        .unwrap_or(DEFAULT_SCHEMA_TTL_SECS)
        * 1000.0
}

/// The schema cache this request runs against: the isolate's own while it is
/// fresh, else KV's if KV has a newer one, else the catalog's.
async fn schema(env: &Env, config: &AppConfig) -> Result<Rc<SchemaCache>, postrust_core::Error> {
    let key = schema_key(config);
    let now = js_sys::Date::now();
    let held = ISOLATE
        .with(|i| i.borrow().schema.clone())
        .filter(|s| s.key == key);
    if let Some(held) = &held {
        if now - held.checked_at < ttl_ms(env) {
            return Ok(held.cache.clone());
        }
    }

    if let Ok(kv) = env.kv(SCHEMA_KV) {
        if let Ok(Some(stored)) = kv.get(&key).json::<Stored>().await {
            let schema = match held {
                Some(held) if held.loaded_at >= stored.loaded_at => Schema {
                    checked_at: now,
                    ..held
                },
                _ => Schema {
                    cache: Rc::new(stored.cache),
                    key,
                    loaded_at: stored.loaded_at,
                    checked_at: now,
                },
            };
            let cache = schema.cache.clone();
            ISOLATE.with(|i| i.borrow_mut().schema = Some(schema));
            return Ok(cache);
        }
    }

    load(env, config).await
}

/// Load the schema cache from the catalog, keep it in this isolate, and write
/// it to KV for the rest.
async fn load(env: &Env, config: &AppConfig) -> Result<Rc<SchemaCache>, postrust_core::Error> {
    let db = database(env)?;
    let cache = SchemaCache::load_with_search_path_from(
        &db,
        &config.db_schemas,
        &config.db_extra_search_path,
    )
    .await;
    db.connector().closed().await;
    let cache = cache?;
    let now = js_sys::Date::now();
    let key = schema_key(config);
    if let Ok(kv) = env.kv(SCHEMA_KV) {
        let stored = Stored {
            loaded_at: now,
            cache,
        };
        // As a JSON string, not as the value: `put` of a `Serialize` value
        // goes through `serde_wasm_bindgen`, which makes every map a
        // JavaScript `Map` -- and KV stores what `JSON.stringify` makes of a
        // `Map`, which is `{}`. Every table read back had no columns.
        let written = match serde_json::to_string(&stored) {
            Ok(json) => match kv.put(&key, json) {
                Ok(put) => put.execute().await.map_err(|e| e.to_string()),
                Err(e) => Err(e.to_string()),
            },
            Err(e) => Err(e.to_string()),
        };
        if let Err(e) = written {
            worker::console_error!("postrust: could not write the schema cache to KV: {e}");
        }
        let cache = Rc::new(stored.cache);
        keep(cache.clone(), key, now);
        return Ok(cache);
    }
    let cache = Rc::new(cache);
    keep(cache.clone(), key, now);
    Ok(cache)
}

fn keep(cache: Rc<SchemaCache>, key: String, loaded_at: f64) {
    ISOLATE.with(|i| {
        i.borrow_mut().schema = Some(Schema {
            cache,
            key,
            loaded_at,
            checked_at: loaded_at,
        })
    });
}

async fn refresh(env: &Env, config: &AppConfig) -> Result<(), postrust_core::Error> {
    load(env, config).await.map(|_| ())
}

/// What a request runs against.
struct State {
    db: PgSocketDb<Hyperdrive>,
    schema: Rc<SchemaCache>,
    config: AppConfig,
    jwt_config: JwtConfig,
    jwt_cache: Option<Rc<JwtCache>>,
}

impl State {
    async fn new(env: &Env, config: AppConfig) -> Result<Self, postrust_core::Error> {
        let db = database(env)?;
        let schema = schema(env, &config).await?;
        let jwt_config = JwtConfig {
            secret: config.jwt_secret.clone(),
            secret_is_base64: config.jwt_secret_is_base64,
            audience: config.jwt_aud.clone(),
            role_claim_key: config.jwt_role_claim_key.clone(),
            anon_role: config.db_anon_role.clone(),
        };
        let jwt_cache = ISOLATE.with(|i| {
            let mut isolate = i.borrow_mut();
            if isolate.jwt_cache.is_none() {
                isolate.jwt_cache =
                    JwtCache::new(config.jwt_cache_enabled, config.jwt_cache_max_lifetime)
                        .map(Rc::new);
            }
            isolate.jwt_cache.clone()
        });
        Ok(Self {
            db,
            schema,
            config,
            jwt_config,
            jwt_cache,
        })
    }
}

impl postrust_engine::State for State {
    type Db = PgSocketDb<Hyperdrive>;
    type SchemaCache<'a> = &'a SchemaCache;

    fn db(&self) -> &PgSocketDb<Hyperdrive> {
        &self.db
    }

    async fn schema_cache(&self) -> &SchemaCache {
        &self.schema
    }

    fn config(&self) -> &AppConfig {
        &self.config
    }

    fn jwt_config(&self) -> &JwtConfig {
        &self.jwt_config
    }

    fn jwt_cache(&self) -> Option<&JwtCache> {
        self.jwt_cache.as_deref()
    }
}
