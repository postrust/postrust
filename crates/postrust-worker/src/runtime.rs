use crate::settings::{self, Binding, Settings, Unconfigured};
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
    let origin = req.headers().get(http::header::ORIGIN).cloned();
    let health = req.method() == Method::GET && req.uri().path() == "/_/health";
    let mut response = match config(&env) {
        Ok(settings) => {
            // Every answer carries the CORS headers, the ones produced before
            // the engine runs as well: a browser cannot read an error
            // without them.
            let origins = settings.config.server_cors_origins.clone();
            let preflight = req.method() == Method::OPTIONS;
            let response = route(req, &env, settings).await;
            cors(&origins, origin.as_ref(), response, preflight)
        }
        Err(unconfigured) => {
            drain(req.into_body()).await;
            unconfigured_response(unconfigured, health, origin.as_ref())
        }
    };
    for name in settings::drop_unsendable_headers(response.headers_mut()) {
        worker::console_error!(
            "postrust: dropped the response header {name}: a Worker cannot send its value"
        );
    }
    Ok(response)
}

async fn route(req: HttpRequest, env: &Env, settings: Settings) -> Response<Body> {
    let path = req.uri().path().to_string();
    match (req.method(), path.as_str()) {
        (&Method::GET, "/_/health") => {
            drain(req.into_body()).await;
            plain(StatusCode::OK, "OK")
        }
        (&Method::POST, "/_/reload") => reload(req, env, &settings).await,
        _ => rest(req, env, settings).await,
    }
}

/// The answer when the configuration cannot be read: the reason, with the
/// CORS origins if those at least could be. `/_/health` reports the Worker
/// unhealthy (503) -- the server would not have started at all with this
/// configuration -- and anything else is a 500.
fn unconfigured_response(
    unconfigured: Unconfigured,
    health: bool,
    origin: Option<&HeaderValue>,
) -> Response<Body> {
    worker::console_error!("postrust: {}", unconfigured.reason);
    let status = if health {
        StatusCode::SERVICE_UNAVAILABLE
    } else {
        StatusCode::INTERNAL_SERVER_ERROR
    };
    let response = plain(status, &unconfigured.reason);
    match &unconfigured.cors_origins {
        Some(origins) => cors(origins, origin, response, false),
        None => response,
    }
}

#[event(scheduled)]
async fn scheduled(_event: ScheduledEvent, env: Env, _ctx: ScheduleContext) {
    match config(&env) {
        Ok(settings) => {
            if let Err(e) = refresh(&env, &settings).await {
                worker::console_error!("postrust: scheduled schema reload failed: {e}");
            }
        }
        Err(unconfigured) => worker::console_error!(
            "postrust: scheduled schema reload skipped: {}",
            unconfigured.reason
        ),
    }
}

/// A request to the REST API.
async fn rest(req: HttpRequest, env: &Env, settings: Settings) -> Response<Body> {
    let verbatim = settings.config.compat_mode;
    // Read before anything can answer: see `read_body`.
    let (parts, body) = req.into_parts();
    let body = read_body(body, settings.config.max_body_size).await;
    let (method, path) = (parts.method.clone(), parts.uri.path().to_string());

    let response = match State::new(env, settings).await {
        Err(e) => postrust_engine::error_response(e, verbatim),
        Ok(state) if method == Method::OPTIONS => preflight(&state, parts).await,
        Ok(state) => {
            let req = http::Request::from_parts(parts, body);
            let response = postrust_engine::handle(&state, req).await;
            state.db.connector().closed().await;
            response
        }
    };
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
    response.map(Body::new)
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

/// A request body nobody wants, read to its end and thrown away, for the
/// reason `read_body` gives: an unread one takes the isolate down after the
/// response.
async fn drain(mut body: worker::Body) {
    while let Some(frame) = body.frame().await {
        if frame.is_err() {
            break;
        }
    }
}

/// An `OPTIONS`: what the resource allows. `fetch` adds the CORS answer the
/// server's layer would give.
async fn preflight(state: &State, parts: http::request::Parts) -> Response<Bytes> {
    let probe = http::Request::from_parts(parts, Bytes::new());
    let allowed = postrust_engine::allow(state, probe).await;
    state.db.connector().closed().await;
    match allowed {
        Ok(allow) => {
            let mut response = Response::new(Bytes::new());
            if let Ok(value) = HeaderValue::from_str(&allow) {
                response.headers_mut().insert(http::header::ALLOW, value);
            }
            response
        }
        Err(e) => postrust_engine::error_response(e, state.config.compat_mode),
    }
}

/// The headers tower-http's `CorsLayer` adds on the server, configured as the
/// server configures it: any or the listed origins, the seven methods, any
/// request header, every response header exposed.
fn cors(
    origins: &[String],
    origin: Option<&HeaderValue>,
    mut response: Response<Body>,
    preflight: bool,
) -> Response<Body> {
    let headers = response.headers_mut();
    let allowed = match origins {
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
async fn reload(req: HttpRequest, env: &Env, settings: &Settings) -> Response<Body> {
    // Nothing here wants the body, but it is read all the same: see
    // `read_body`.
    let (parts, body) = req.into_parts();
    drain(body).await;
    let Ok(secret) = env.secret(RELOAD_SECRET).map(|s| s.to_string()) else {
        return plain(StatusCode::NOT_FOUND, "Not Found");
    };
    let presented = parts
        .headers
        .get(http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .unwrap_or_default();
    if !constant_time_eq(presented.as_bytes(), secret.as_bytes()) {
        return plain(StatusCode::UNAUTHORIZED, "Unauthorized");
    }
    match refresh(env, settings).await {
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

/// The configuration, from the Worker's variables and secrets (see
/// `settings::settings`).
fn config(env: &Env) -> Result<Settings, Unconfigured> {
    let env: &JsValue = env.as_ref();
    let pairs = js_sys::Object::entries(env.unchecked_ref())
        .iter()
        .filter_map(|entry| {
            let entry: js_sys::Array = entry.into();
            let name = entry.get(0).as_string()?;
            let value = entry.get(1);
            let binding = if let Some(text) = value.as_string() {
                Binding::Text(text)
            } else if let Some(b) = value.as_bool() {
                Binding::Bool(b)
            } else if let Some(n) = value.as_f64() {
                Binding::Number(n)
            } else if value.is_null() || value.is_undefined() {
                return None;
            } else {
                Binding::Other
            };
            let text = settings::binding_text(binding);
            if text.is_none() && settings::is_setting(&name) {
                worker::console_warn!(
                    "postrust: ignoring {name}: it is a JSON object or array, not a value a \
                     setting can take; give it as a string"
                );
            }
            Some((name, text?))
        })
        .collect::<Vec<_>>();
    settings::settings(pairs)
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
struct Schema {
    cache: Rc<SchemaCache>,
    key: String,
    loaded_at: f64,
    checked_at: f64,
}

thread_local! {
    static ISOLATE: RefCell<Isolate> = RefCell::default();
}

/// The KV record a schema cache is kept as: written from a borrowed cache,
/// read back as an owned one.
#[derive(serde::Serialize, serde::Deserialize)]
struct Stored<C> {
    /// When it was loaded from the catalog, in milliseconds since the epoch.
    loaded_at: f64,
    cache: C,
}

/// The schema cache this request runs against: the isolate's own while it is
/// fresh, else KV's if KV has one, else the catalog's.
async fn schema(env: &Env, settings: &Settings) -> Result<Rc<SchemaCache>, postrust_core::Error> {
    let key = settings::schema_key(&settings.config);
    let now = js_sys::Date::now();
    let fresh = ISOLATE.with(|i| {
        i.borrow()
            .schema
            .as_ref()
            .filter(|s| s.key == key && now - s.checked_at < settings.schema_ttl_secs * 1000.0)
            .map(|s| s.cache.clone())
    });
    if let Some(cache) = fresh {
        return Ok(cache);
    }

    if let Ok(kv) = env.kv(SCHEMA_KV) {
        match kv.get(&key).json::<Stored<SchemaCache>>().await {
            Ok(Some(stored)) => {
                return Ok(keep(Schema {
                    cache: Rc::new(stored.cache),
                    key,
                    loaded_at: stored.loaded_at,
                    checked_at: now,
                }))
            }
            Ok(None) => {}
            // A record that does not read back is a miss: the catalog's copy
            // replaces it below.
            Err(e) => worker::console_warn!(
                "postrust: the schema cache in KV could not be read, loading it from the \
                 catalog: {e}"
            ),
        }
    }

    load(env, settings).await
}

/// Load the schema cache from the catalog, keep it in this isolate, and write
/// it to KV for the rest.
async fn load(env: &Env, settings: &Settings) -> Result<Rc<SchemaCache>, postrust_core::Error> {
    let config = &settings.config;
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
    let key = settings::schema_key(config);
    let cache = Rc::new(cache);
    if let Ok(kv) = env.kv(SCHEMA_KV) {
        let stored = Stored {
            loaded_at: now,
            cache: &*cache,
        };
        // As a JSON string, not as the value: `put` of a `Serialize` value
        // goes through `serde_wasm_bindgen`, which makes every map a
        // JavaScript `Map` -- and KV stores what `JSON.stringify` makes of a
        // `Map`, which is `{}`. Every table read back had no columns.
        //
        // It expires, so that a migration is seen without the cron or a
        // reload: the next isolate to miss reads the catalog again.
        let expiration = settings::kv_expiration_secs(settings.schema_ttl_secs);
        let written = match serde_json::to_string(&stored) {
            Ok(json) => match kv.put(&key, json) {
                Ok(put) => put
                    .expiration_ttl(expiration)
                    .execute()
                    .await
                    .map_err(|e| e.to_string()),
                Err(e) => Err(e.to_string()),
            },
            Err(e) => Err(e.to_string()),
        };
        if let Err(e) = written {
            worker::console_error!("postrust: could not write the schema cache to KV: {e}");
        }
    }
    Ok(keep(Schema {
        cache,
        key,
        loaded_at: now,
        checked_at: now,
    }))
}

/// Keep `candidate` as this isolate's schema cache, unless what the isolate
/// holds by now is newer, and return whichever it keeps.
///
/// The decision is made here, after every `await`, against what is held at
/// this moment: a request that read KV or the catalog while a reload or the
/// cron installed a newer copy must not put its older one back.
fn keep(candidate: Schema) -> Rc<SchemaCache> {
    ISOLATE.with(|i| {
        let mut isolate = i.borrow_mut();
        if let Some(held) = isolate.schema.as_mut() {
            if !settings::supersedes(
                Some((&held.key, held.loaded_at)),
                &candidate.key,
                candidate.loaded_at,
            ) {
                // Nothing newer than the copy held: it is current as of now.
                held.checked_at = held.checked_at.max(candidate.checked_at);
                return held.cache.clone();
            }
        }
        let cache = candidate.cache.clone();
        isolate.schema = Some(candidate);
        cache
    })
}

async fn refresh(env: &Env, settings: &Settings) -> Result<(), postrust_core::Error> {
    load(env, settings).await.map(|_| ())
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
    async fn new(env: &Env, settings: Settings) -> Result<Self, postrust_core::Error> {
        let db = database(env)?;
        let schema = schema(env, &settings).await?;
        let config = settings.config;
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
