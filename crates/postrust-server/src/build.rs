//! Building the application a host serves.
//!
//! Everything between a loaded configuration and a router answering requests:
//! the pool, the schema cache, the REST surface, GraphQL and the layers around
//! them. The `postrust` binary serves the result on a socket; the Lambda
//! adapter hands the same router to the Lambda runtime. One path, so a request
//! is answered the same way wherever it arrives -- the Lambda adapter used to
//! be a second copy of the request path, and that copy bound no parameters and
//! switched to no role.

use crate::app::{self, handle_request};
use crate::custom;
use crate::state::AppState;
use anyhow::Result;
#[cfg(feature = "graphql")]
use axum::routing::{get, post};
use axum::{
    extract::{DefaultBodyLimit, Request},
    http::{HeaderValue, Method},
    response::{IntoResponse, Json},
    routing::{any, Route},
    Router,
};
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use std::convert::Infallible;
use std::sync::Arc;
use tower::{Layer, Service};
use tower_http::cors::{AllowOrigin, Any as CorsAny, CorsLayer};
use tracing::{error, info};

/// What the host running the application can do beyond answering requests.
///
/// A server process lives as long as it likes and owns its sockets. A Lambda
/// sandbox is frozen between invocations, so a task listening for `NOTIFY` is
/// asleep exactly when the notification arrives, and API Gateway does not
/// upgrade a connection to a WebSocket at all.
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub struct Host {
    /// Long-lived background work: the `NOTIFY` schema reloader
    /// (`db_channel_enabled`) and the subscription broker's listener.
    pub background_tasks: bool,
    /// Upgrading a connection to a WebSocket, which GraphQL subscriptions are
    /// served over.
    pub websockets: bool,
}

impl Host {
    /// A long-running server process.
    pub const SERVER: Host = Host {
        background_tasks: true,
        websockets: true,
    };

    /// An AWS Lambda function: no background work, no WebSockets.
    pub const LAMBDA: Host = Host {
        background_tasks: false,
        websockets: false,
    };
}

/// The application, ready to serve.
#[non_exhaustive]
pub struct App {
    /// Every route the configuration enables, with its layers and state.
    pub router: Router,
    /// The state the routes share, for anything served beside them -- the
    /// binary's admin port answers health checks from it.
    pub state: Arc<AppState>,
}

/// Connect, load the schema cache and build the router.
///
/// Fails rather than serving something partial when the database cannot be
/// reached or a setting cannot be honoured. GraphQL is the exception: a schema
/// that cannot be built is logged and the rest is served without it.
///
/// The application with nothing of the caller's added. [`Builder`] builds the
/// same with routes of the caller's own beside it.
pub async fn build(config: postrust_core::AppConfig, host: Host) -> Result<App> {
    Builder::new(config, host).build().await
}

/// Builds the application with routes of the caller's own beside the API.
///
/// For a binary of your own that depends on this crate rather than a fork of
/// it: webhooks, an endpoint that does several things in one transaction,
/// anything that is not a table or a function.
///
/// ```no_run
/// # async fn run() -> anyhow::Result<()> {
/// use axum::{routing::post, Router};
/// use postrust_server::{AppState, Builder, Host};
/// use std::sync::Arc;
///
/// let hooks: Router<Arc<AppState>> = Router::new().route("/stripe", post(|| async { "ok" }));
/// let config = postrust_core::AppConfig::try_from_env()?;
/// let app = Builder::new(config, Host::SERVER)
///     .nest("/hooks", hooks)
///     .build()
///     .await?;
/// # let _ = app;
/// # Ok(())
/// # }
/// ```
///
/// A nested router is served with everything the API is served with: the
/// CORS policy, the body limit, the `OPTIONS` handling and the shared
/// [`AppState`]. Its routes are explicit routes, so in compatibility mode they
/// are matched before the REST surface served at the root -- a table that
/// shares a prefix's name is shadowed there, and stays reachable under `/api`.
///
/// [`Builder::layer`] is for what has to see every request rather than only
/// the caller's own: an authentication gate, a rate limit, a header every
/// response carries.
#[must_use = "a Builder does nothing until `build` is called"]
pub struct Builder {
    config: postrust_core::AppConfig,
    host: Host,
    nests: Vec<(String, Router<Arc<AppState>>)>,
    layers: Vec<Wrap>,
}

/// A layer given to [`Builder::layer`], held until there is a router for it.
///
/// Boxed as the step that applies it rather than as the layer, since each
/// layer is its own type and only `Router::layer` knows what it needs of one.
type Wrap = Box<dyn FnOnce(Router) -> Router + Send>;

impl Builder {
    /// The application `config` asks for, on `host`, with nothing added yet.
    pub fn new(config: postrust_core::AppConfig, host: Host) -> Self {
        Builder {
            config,
            host,
            nests: Vec::new(),
            layers: Vec::new(),
        }
    }

    /// Serve `router` under `prefix`. May be called more than once.
    ///
    /// The prefix is taken as a path: the leading slash is added if missing
    /// and a trailing one dropped, so `"hooks"`, `"/hooks"` and `"/hooks/"`
    /// are one prefix. Nothing is checked until [`Builder::build`], which
    /// refuses a prefix that is the root, that overlaps a path the server
    /// serves itself ([`RESERVED_PREFIXES`], and `/_` itself), or that
    /// overlaps another prefix given here. In compatibility mode, where REST
    /// is served at `/` as well, a prefix whose first segment is `rpc` or the
    /// name of a table or view in an exposed schema takes those paths from
    /// REST, and a warning says so at start-up. A prefix beneath `/_`, such as
    /// `/_/hooks`, is served beside the built-in `/_/health` and `/_/ready`.
    pub fn nest(mut self, prefix: impl Into<String>, router: Router<Arc<AppState>>) -> Self {
        self.nests.push((prefix.into(), router));
        self
    }

    /// Wrap the whole application in `layer`. May be called more than once.
    ///
    /// Outermost, around every route the server serves and around its own
    /// layers too, so it sees each request first and each response last --
    /// including a preflight the CORS policy answers itself, and a refusal
    /// that never reaches a handler. Of several, the one given last is the
    /// outermost, as with `Router::layer`.
    ///
    /// For something only the caller's own routes need, put the layer on the
    /// router given to [`Builder::nest`] instead.
    pub fn layer<L>(mut self, layer: L) -> Self
    where
        L: Layer<Route> + Clone + Send + Sync + 'static,
        L::Service: Service<Request> + Clone + Send + Sync + 'static,
        <L::Service as Service<Request>>::Response: IntoResponse + 'static,
        <L::Service as Service<Request>>::Error: Into<Infallible> + 'static,
        <L::Service as Service<Request>>::Future: Send + 'static,
    {
        self.layers
            .push(Box::new(move |router: Router| router.layer(layer)));
        self
    }

    /// Connect, load the schema cache and build the router.
    ///
    /// The prefixes given to [`Builder::nest`] are checked first, before
    /// anything is connected to, so a clash is reported as an error rather
    /// than as a panic from the router -- and without a database to find it.
    /// Otherwise as [`build`].
    pub async fn build(self) -> Result<App> {
        let nests = mount_points(self.nests)?;
        // The JWT keys too, for the same reason: a key set that does not
        // parse, or a JWKS URL that would not be fetched, is a configuration
        // error, and is better reported before the database is connected to
        // than after. Made once, here, and handed to the state.
        let jwt = jwt_config(&self.config)?;
        let mut app = assemble(self.config, self.host, nests, jwt).await?;
        for wrap in self.layers {
            app.router = wrap(app.router);
        }
        Ok(app)
    }
}

/// The paths the server serves itself, which a prefix given to
/// [`Builder::nest`] may neither equal nor sit above or beneath.
///
/// Reserved whether or not the feature that serves them is compiled in, so
/// that a binary which builds today does not start failing when GraphQL or the
/// admin UI is turned on. `/_` is not in the list: the built-in routes under
/// it are, and the rest of it is the caller's.
pub const RESERVED_PREFIXES: &[&str] = &[
    "/api",
    "/healthz",
    "/v1/version",
    "/v1/graphql",
    "/v1alpha1/graphql",
    "/admin",
    "/_/health",
    "/_/ready",
];

/// Where the built-in custom router is mounted.
const CUSTOM_PREFIX: &str = "/_";

/// The caller's routers, each under its prefix once the prefix is checked.
type Nests = Vec<(String, Router<Arc<AppState>>)>;

/// Check the prefixes given to [`Builder::nest`], normalizing each.
fn mount_points(nests: Nests) -> Result<Nests> {
    let mut checked: Nests = Vec::with_capacity(nests.len());
    for (given, router) in nests {
        let taken: Vec<&str> = checked.iter().map(|(prefix, _)| prefix.as_str()).collect();
        let prefix = check_prefix(&given, &taken)?;
        checked.push((prefix, router));
    }
    Ok(checked)
}

/// `given` as a prefix to nest at -- one leading slash, no trailing one -- or
/// why it cannot be nested at beside the server's own paths and the prefixes
/// already `taken`.
fn check_prefix(given: &str, taken: &[&str]) -> Result<String> {
    let trimmed = given.trim().trim_matches('/');
    if trimmed.is_empty() {
        // The root is the API's in compatibility mode and the server's own
        // directory otherwise; a router merged there would take one or the
        // other.
        anyhow::bail!("cannot nest routes at {given:?}: a prefix must name a path below /");
    }
    // An empty segment matches nothing a client sends, and a parameter or a
    // wildcard in a nest path is something the router refuses by panicking.
    // Both are refused here instead, where it can be said why.
    if trimmed
        .split('/')
        .any(|segment| segment.is_empty() || segment.contains(['{', '}', '*']))
    {
        anyhow::bail!(
            "cannot nest routes at {given:?}: a prefix is literal path segments, \
             without parameters, wildcards or empty segments"
        );
    }
    let prefix = format!("/{trimmed}");

    // Overlap rather than equality. `/v1` is not `/v1/version`, but nesting
    // at `/v1` claims everything beneath it, and which of the two then
    // answers `/v1/version` would be settled by the router's matching
    // priority rather than by anything the caller wrote. A nest at exactly
    // the same path as another is one the router refuses by panicking.
    if prefix == CUSTOM_PREFIX {
        anyhow::bail!(
            "cannot nest routes at {given:?}: the server serves /_ itself; nest beneath it, \
             at /_/<name>, instead"
        );
    }
    if let Some(reserved) = RESERVED_PREFIXES
        .iter()
        .find(|reserved| overlaps(&prefix, reserved))
    {
        anyhow::bail!(
            "cannot nest routes at {given:?}: it overlaps {reserved}, which the server serves itself"
        );
    }
    if let Some(other) = taken.iter().find(|other| overlaps(&prefix, other)) {
        anyhow::bail!("cannot nest routes at {given:?}: it overlaps {other}, nested earlier");
    }
    Ok(prefix)
}

/// Whether `path` is `base` or beneath it, segment by segment: `/apis` is not
/// beneath `/api`.
/// In compatibility mode REST is served at `/` too, beneath every path no
/// route claims, so a prefix naming a table, a view or `/rpc` takes those
/// requests from REST. That is allowed -- the route was asked for, and the
/// table is still served under `/api` -- but said, so it is not found out
/// from a response. Checked against the schema cache at start-up.
fn warn_of_shadowing(nests: &Nests, schema_cache: &postrust_core::SchemaCache, schemas: &[String]) {
    for (prefix, _) in nests {
        let first = prefix
            .trim_start_matches('/')
            .split('/')
            .next()
            .unwrap_or_default();
        if first == "rpc" {
            tracing::warn!(
                "compatibility mode: routes nested at {prefix} answer in place of the \
                 database functions REST serves at /rpc; they are still served at /api/rpc"
            );
        }
        if let Some(table) = schema_cache
            .tables
            .keys()
            .find(|qi| qi.name == first && schemas.contains(&qi.schema))
        {
            tracing::warn!(
                "compatibility mode: routes nested at {prefix} answer in place of {}.{first}, \
                 which REST would serve at /{first}; it is still served at /api/{first}",
                table.schema
            );
        }
    }
}

fn beneath(path: &str, base: &str) -> bool {
    path.strip_prefix(base)
        .is_some_and(|rest| rest.is_empty() || rest.starts_with('/'))
}

/// Whether nesting at both `a` and `b` would claim a path twice.
fn overlaps(a: &str, b: &str) -> bool {
    beneath(a, b) || beneath(b, a)
}

/// The keys and claims tokens are verified with: `PGRST_JWT_SECRET`, and the
/// key set at `PGRST_JWT_JWKS_URL` when there is one (not yet fetched).
fn jwt_config(config: &postrust_core::AppConfig) -> Result<postrust_auth::JwtConfig> {
    let jwt = postrust_engine::jwt_config(config)
        .map_err(|e| anyhow::anyhow!("PGRST_JWT_SECRET: {e}"))?;
    // Its errors name `PGRST_JWT_JWKS_URL` themselves.
    crate::jwks::attach(jwt, config).map_err(|e| anyhow::anyhow!("{e}"))
}

/// Everything [`Builder::build`] does once the caller's prefixes and the JWT
/// configuration are checked.
async fn assemble(
    config: postrust_core::AppConfig,
    host: Host,
    nests: Nests,
    jwt: postrust_auth::JwtConfig,
) -> Result<App> {
    info!("Database: {}", mask_db_uri(&config.db_uri));

    // Create database pool
    let connect_options: PgConnectOptions = config
        .db_uri
        .parse()
        .map_err(|e| anyhow::anyhow!("invalid database URI: {e}"))?;

    // Prepared statements are sqlx's default and stay on unless asked. Turning
    // them off is for a connection pooler in transaction mode -- PgBouncer and
    // the like -- where the server connection a statement was prepared on is
    // not the one the next query lands on, and every reuse fails with
    // "prepared statement does not exist".
    let connect_options = if config.db_prepared_statements {
        connect_options
    } else {
        info!("Prepared statements disabled");
        connect_options.statement_cache_capacity(0)
    };

    let pool = PgPoolOptions::new()
        .max_connections(config.db_pool_size)
        .acquire_timeout(std::time::Duration::from_secs(config.db_pool_timeout))
        .connect_with(connect_options)
        .await?;

    info!("Connected to database");

    // Load schema cache
    let schema_cache = postrust_core::SchemaCache::load_with_search_path(
        &pool,
        &config.db_schemas,
        &config.db_extra_search_path,
    )
    .await?;
    info!("{}", schema_cache.summary());
    if config.compat_mode {
        warn_of_shadowing(&nests, &schema_cache, &config.db_schemas);
    }

    // Create app state
    let state = Arc::new(AppState::with_jwt(pool, schema_cache, config.clone(), jwt));

    // The pool and the token cache, for the metrics read when collected.
    #[cfg(feature = "otel")]
    crate::otel::observe(&state);

    // `db_channel_enabled`: reload the schema cache when PostgreSQL says the
    // schema changed, instead of on a restart. A migration ends with
    // `NOTIFY pgrst, 'reload schema'` and the API picks the new shape up.
    //
    // The REST cache is swapped here; each reload then bumps `reloaded`, and
    // the GraphQL schema, built from its own snapshot of the cache, is
    // rebuilt from the new one when it sees the bump.
    // Keys from `PGRST_JWT_JWKS_URL`: fetched once before serving, then kept
    // fresh in the background where the host runs background work. A fetch
    // that fails does not stop the start; see `crate::jwks`.
    crate::jwks::start(&state, host).await;

    let (reloaded, schema_reloads) = tokio::sync::watch::channel(0u64);
    if config.db_channel_enabled && host.background_tasks {
        spawn_schema_reloader(state.clone(), config.db_channel.clone(), reloaded);
    }
    #[cfg(not(feature = "graphql"))]
    drop(schema_reloads);

    // Build REST API router (under /api prefix)
    let api_router: Router<Arc<AppState>> = Router::new()
        .route("/", any(handle_request))
        .route("/{*path}", any(handle_request));

    // Build main router
    let mut app: Router<Arc<AppState>> = Router::new().nest("/api", api_router);

    // Two endpoints every Hasura deployment has, and that the things around a
    // deployment reach for without being asked to: `/healthz` is what a load
    // balancer, a Kubernetes probe and `docker-compose` healthchecks are
    // already configured to poll, and `/v1/version` is what a client library
    // calls to decide which features to use. Serving them costs nothing and
    // not serving them makes a drop-in replacement fail its first health
    // check.
    app = app
        .route("/healthz", axum::routing::get(|| async { "OK" }))
        .route(
            "/v1/version",
            axum::routing::get(|| async {
                Json(serde_json::json!({ "version": env!("CARGO_PKG_VERSION") }))
            }),
        );

    // Add custom routes (health checks, webhooks, etc.)
    //
    // The caller's own routes go in here too, before any layer is applied, so
    // they are wrapped in the same CORS policy, body limit and `OPTIONS`
    // handling as the API. One beneath `/_`, such as `/_/hooks`, sits beside
    // the nest at `/_`: the more specific path is matched first.
    app = app.nest(CUSTOM_PREFIX, custom::custom_router());
    info!("Custom routes enabled at /_");
    for (prefix, router) in nests {
        info!("Custom routes enabled at {}", prefix);
        // In compatibility mode a database error is answered in the
        // database's own words, by REST and so by a route's
        // `postrust_server::Error` too. The error cannot see the
        // configuration when `?` makes it, so this renders it again on the
        // way out; see `crate::error`.
        let router = match config.compat_mode {
            true => router.layer(axum::middleware::map_response_with_state(
                state.clone(),
                crate::error::render_verbatim,
            )),
            false => router,
        };
        app = app.nest(&prefix, router);
    }

    // Add admin routes and GraphQL endpoint if feature is enabled
    #[cfg(feature = "graphql")]
    {
        use async_graphql_axum::GraphQLBatchRequest as GqlRequest;
        use axum::extract::State as AxumState;
        use axum::http::HeaderMap;
        use postrust_graphql::handler::GraphQLState;
        use postrust_graphql::schema::SchemaConfig;

        #[cfg(feature = "admin-ui")]
        {
            info!("Admin UI enabled at /admin");
            app = app.nest("/admin", crate::admin::admin_router());
        }

        // Create GraphQL state with subscriptions enabled
        let schema_cache_snapshot = state.schema_cache.read().await.clone();
        let schema_cache_arc = Arc::new(schema_cache_snapshot);
        // What Hasura keeps in metadata and a schema cannot carry: the names,
        // and what each role may do. Absent, every name is derived exactly as
        // before and there is no permission layer at all.
        //
        // Two spellings because the document outgrew the first one. `_NAMES`
        // is what it was called when names were all it held, and a deployment
        // already setting it should not have to change anything.
        let names_var = ["PGRST_GRAPHQL_METADATA", "PGRST_GRAPHQL_NAMES"]
            .into_iter()
            .find_map(|name| std::env::var(name).ok().map(|value| (name, value)));

        let graphql_names = match names_var {
            Some((var, value)) => match postrust_graphql::names::NameOverrides::parse(&value) {
                Ok(names) => {
                    if !names.is_empty() {
                        info!("GraphQL names given for {} tables", names.len());
                    }
                    if names.placed_functions() > 0 {
                        info!(
                            "GraphQL roots given for {} functions",
                            names.placed_functions()
                        );
                    }
                    if names.has_permissions() {
                        info!(
                            "GraphQL permissions given for {} roles, {} grants",
                            names.roles().len(),
                            names.granted()
                        );
                    }
                    names
                }
                Err(e) => {
                    // Serving the derived names instead would answer every
                    // request under a name the client does not send, which
                    // reads as a broken server rather than a bad setting. A
                    // permission read wrong is worse still: it would serve
                    // rows a rule was written to withhold.
                    tracing::error!("{}: {}", var, e);
                    return Err(anyhow::anyhow!("{}: {}", var, e));
                }
            },
            None => postrust_graphql::names::NameOverrides::default(),
        };

        // An enum table's members are rows, not schema, so they are read here
        // rather than reflected, and again each time the schema is rebuilt:
        // a migration that adds a value notifies like one that adds a column.
        let graphql_enum_values = read_enum_values(
            &state.pool,
            &*state.schema_cache.read().await,
            &graphql_names,
        )
        .await;

        // How often a live query re-reads itself with nothing having
        // notified it. The notifications do the work; this is the floor
        // under what a trigger cannot see -- a view, an embedded row on a
        // table with no trigger, a predicate written against the clock.
        let subscription_refresh_seconds = std::env::var("PGRST_SUBSCRIPTION_REFRESH")
            .ok()
            .and_then(|value| value.trim().parse::<u64>().ok())
            .unwrap_or(30);

        let mut graphql_config = SchemaConfig::default();
        // A subscription is a WebSocket, so a host that cannot hold one
        // does not advertise a `Subscription` root it cannot serve.
        graphql_config.enable_subscriptions = host.websockets;
        graphql_config.subscription_refresh_seconds = subscription_refresh_seconds;
        graphql_config.max_rows = config.db_max_rows;
        graphql_config.enable_federation = config.graphql_federation;
        graphql_config.type_prefix = config.graphql_type_prefix.clone();
        // The GraphQL schema was built for `public` whatever the server
        // was told to expose, so a table in any other schema of
        // `PGRST_DB_SCHEMAS` was reachable over REST and invisible over
        // GraphQL.
        graphql_config.exposed_schemas = config.db_schemas.clone();
        graphql_config.enum_values = graphql_enum_values;
        graphql_config.names = graphql_names;
        // A schema that cannot be built is a reason to serve without GraphQL,
        // not a reason to exit. The REST surface, the admin UI and the health
        // endpoint do not depend on it, and taking the process down with it
        // turns one unrepresentable table into a server that will not start.
        // The endpoint is mounted all the same, answering that there is no
        // schema, so the reload after a migration that fixes the table brings
        // GraphQL up without a restart.
        let graphql_state = match GraphQLState::new(
            state.pool.clone(),
            schema_cache_arc.clone(),
            graphql_config.clone(),
        ) {
            Ok(graphql_state) => {
                // Initialize subscription broker
                if !host.websockets {
                    info!("GraphQL subscriptions disabled: this host cannot hold a WebSocket");
                } else if let Err(e) = graphql_state.init_subscriptions().await {
                    tracing::warn!("Failed to initialize subscription broker: {}. Subscriptions may not work until triggers are created.", e);
                } else {
                    info!("GraphQL subscriptions enabled");
                }
                info!("GraphQL endpoint enabled at /api/graphql");
                Some(Arc::new(graphql_state))
            }
            Err(e) => {
                tracing::error!(
                    "GraphQL schema could not be built, serving without it: {}",
                    e
                );
                None
            }
        };
        {
            // The schema in use, replaced whole when the database's changes.
            // A request takes the current one once and keeps it, so an
            // operation is never answered half from one schema and half from
            // the next.
            let current = Arc::new(std::sync::RwLock::new(graphql_state));
            spawn_graphql_rebuilder(
                state.clone(),
                current.clone(),
                graphql_config,
                schema_reloads,
            );

            // Combined state for GraphQL routes (includes JWT config for auth)
            #[derive(Clone)]
            struct GraphQLAppState {
                gql_state: Arc<std::sync::RwLock<Option<Arc<GraphQLState>>>>,
                /// REST's state, for the JWT configuration and the cache of
                /// verified tokens: one token is verified once, whichever
                /// surface it is presented to.
                rest: Arc<AppState>,
                hasura_auth: postrust_auth::HasuraAuthConfig,
                /// REST's configuration, shared by every operation's context:
                /// a GraphQL transaction is set up from it as a REST one is.
                /// One copy for the server's life, since the configuration is
                /// read once at start-up and never changes under it.
                config: Arc<postrust_core::AppConfig>,
            }

            impl GraphQLAppState {
                /// The GraphQL state to answer this operation from, if there
                /// is one yet.
                fn gql(&self) -> Option<Arc<GraphQLState>> {
                    self.gql_state
                        .read()
                        .unwrap_or_else(|e| e.into_inner())
                        .clone()
                }
            }

            let graphql_app_state = GraphQLAppState {
                gql_state: current,
                rest: state.clone(),
                hasura_auth: postrust_auth::HasuraAuthConfig {
                    admin_secret: config.hasura_admin_secret.clone(),
                    unauthorized_role: config.hasura_unauthorized_role.clone(),
                },
                config: Arc::new(state.config.clone()),
            };

            // Wrapper handler that creates context from request with proper auth
            /// A body that is a JSON *array* is a batch: several operations
            /// in one request, answered with an array of responses in the
            /// order they were sent. Each is its own operation with its own
            /// transaction -- batching is a way to save round trips, not a
            /// way to make several mutations atomic, which is what naming
            /// several root fields in one mutation is for.
            async fn handle_graphql(
                AxumState(app_state): AxumState<GraphQLAppState>,
                // Which of the three addresses this arrived at. The routes are
                // nested, so the request's own path is `/` by the time it gets
                // here and the original is the only thing that still says.
                axum::extract::OriginalUri(uri): axum::extract::OriginalUri,
                headers: HeaderMap,
                req: GqlRequest,
            ) -> (axum::http::StatusCode, Json<serde_json::Value>) {
                // The status a refused write is answered with belongs to the
                // endpoint rather than to the refusal: `/v1alpha1/graphql`
                // reports it in the transport and `/v1/graphql` does not.
                let endpoint = match uri.path().starts_with("/v1alpha1/") {
                    true => postrust_graphql::hasura::Endpoint::Legacy,
                    false => postrust_graphql::hasura::Endpoint::Current,
                };
                match req.0 {
                    async_graphql::BatchRequest::Single(request) => {
                        let body = one_graphql(app_state, headers, request).await;
                        let status = axum::http::StatusCode::from_u16(
                            postrust_graphql::hasura::status_for(&body, endpoint),
                        )
                        .unwrap_or(axum::http::StatusCode::OK);
                        (status, Json(body))
                    }
                    async_graphql::BatchRequest::Batch(requests) => {
                        let mut answers = Vec::with_capacity(requests.len());
                        for request in requests {
                            answers.push(
                                one_graphql(app_state.clone(), headers.clone(), request).await,
                            );
                        }
                        // A batch is answered 200 whatever its parts say: the
                        // transport carried every one of them, and which of
                        // them failed is in the array.
                        (
                            axum::http::StatusCode::OK,
                            Json(serde_json::Value::Array(answers)),
                        )
                    }
                }
            }

            /// Who is asking, settled once: per request over HTTP, and per
            /// connection over a WebSocket, where the token arrives in the
            /// `connection_init` payload rather than on every operation.
            #[derive(Clone)]
            struct GraphQLIdentity {
                auth_result: postrust_auth::AuthResult,
                hasura_role: Option<String>,
                session: std::collections::HashMap<String, String>,
                elevated: bool,
                headers: HeaderMap,
            }

            /// An operation ready to run against the schema its role answers
            /// from, and the transaction its writes will share.
            struct PreparedOperation {
                schema: async_graphql::dynamic::Schema,
                request: async_graphql::Request,
                write: postrust_graphql::context::SharedWrite,
            }

            /// What [`postrust_auth::ensure_keys`] is to REST: keys from a
            /// JWKS URL fetched, if the token needs them, before the
            /// synchronous [`graphql_identity`] verifies it.
            async fn ensure_keys(app_state: &GraphQLAppState, headers: &HeaderMap) {
                let auth_header = headers.get("authorization").and_then(|v| v.to_str().ok());
                postrust_auth::ensure_keys(auth_header, &app_state.rest.jwt_config).await;
            }

            /// The caller's identity, or the Hasura-shaped body refusing it.
            fn graphql_identity(
                app_state: &GraphQLAppState,
                headers: HeaderMap,
            ) -> Result<GraphQLIdentity, serde_json::Value> {
                // Extract auth header and authenticate
                let auth_header = headers.get("authorization").and_then(|v| v.to_str().ok());

                // Through the token cache where one is configured, as REST
                // authenticates.
                let jwt_config = &app_state.rest.jwt_config;
                let token = match &app_state.rest.jwt_cache {
                    Some(cache) => cache.authenticate(auth_header, jwt_config),
                    None => postrust_auth::authenticate(auth_header, jwt_config),
                };
                // A token that was offered and verified. Not the same as
                // `token.is_ok()`, which is also true of the anonymous
                // fallback -- and an anonymous request is precisely the one
                // that has not been authenticated.
                let token_verified = auth_header.is_some() && token.is_ok();
                // A token that was offered and is refused: REST answers each of
                // these 401. Without a secret there is nothing to verify
                // against, and the header is ignored as Hasura ignores it when
                // JWT mode is off -- a router forwarding `Authorization` to
                // every subgraph relies on that. With no header at all, and no
                // anonymous role, the request goes on as it always has.
                let token_refusal = match (&token, auth_header) {
                    (Ok(_) | Err(postrust_auth::JwtError::SecretMissing), _) | (Err(_), None) => {
                        None
                    }
                    // Verified, and naming no role, with no anonymous role to
                    // fall back to. Served as anyone, it would be served as a
                    // role nobody chose.
                    (Err(postrust_auth::JwtError::NoIdentity), Some(_)) => Some(
                        postrust_graphql::hasura::missing_role_claim(&jwt_config.role_claim_key),
                    ),
                    (Err(e), Some(_)) => {
                        Some(postrust_graphql::hasura::invalid_jwt(&e.to_string()))
                    }
                };

                let auth_result = match token {
                    Ok(auth) => auth,
                    Err(e) => {
                        tracing::debug!("GraphQL auth failed: {}, using anon role", e);
                        postrust_auth::AuthResult {
                            role: jwt_config
                                .anon_role
                                .clone()
                                .unwrap_or_else(|| "anon".to_string()),
                            claims: std::collections::HashMap::new(),
                        }
                    }
                };

                tracing::debug!(
                    "GraphQL request authenticated as role: {}",
                    auth_result.role
                );

                // Who the caller is in Hasura's sense, which is a different
                // question from which database user the transaction runs as.
                // The secret goes first: a caller that tried to be an
                // administrator and got the secret wrong is refused rather
                // than asked for a token.
                let pairs: Vec<(&str, &str)> = headers
                    .iter()
                    .filter_map(|(name, value)| {
                        value.to_str().ok().map(|value| (name.as_str(), value))
                    })
                    .collect();

                let (hasura_role, session, elevated) = match (
                    postrust_auth::hasura::from_admin_secret(&app_state.hasura_auth, &pairs[..]),
                    token_refusal,
                ) {
                    (postrust_auth::SecretOutcome::Accepted(identity), _) => {
                        (Some(identity.role), identity.session, identity.elevated)
                    }
                    (postrust_auth::SecretOutcome::Rejected, _) => {
                        return Err(postrust_graphql::hasura::denied(
                            postrust_auth::hasura::SECRET_INVALID,
                        ));
                    }
                    // The secret settled nothing, so the token is refused
                    // -- and an administrator's secret beside it wins, as
                    // it does in Hasura.
                    (_, Some(refusal)) => return Err(refusal),
                    // Nothing settled by the secret. A verified token speaks
                    // next, and session variables come from its claims.
                    //
                    // One header is read here and only here: `X-Hasura-Role`,
                    // against the `x-hasura-allowed-roles` the token carries.
                    // That list is inside the signature, so it lets a caller
                    // choose among the identities it was issued and not add
                    // one. Every other `x-hasura-*` header is still ignored --
                    // on a server with no secret to gate them they would let
                    // any caller name its own identity.
                    _ if token_verified => {
                        match postrust_auth::hasura::role_for_token(&auth_result.claims, &pairs[..])
                        {
                            postrust_auth::hasura::TokenRole::Is(role) => (
                                role,
                                postrust_auth::hasura::session_from_claims(&auth_result.claims),
                                false,
                            ),
                            postrust_auth::hasura::TokenRole::NotAllowed => {
                                return Err(postrust_graphql::hasura::denied(
                                    postrust_auth::hasura::ROLE_NOT_ALLOWED,
                                ));
                            }
                        }
                    }
                    // A secret is configured and nothing authenticated this
                    // request. Answering it as a stranger is a choice the
                    // deployment makes; refusing is the default.
                    _ if app_state.hasura_auth.is_configured() => {
                        match postrust_auth::hasura::unauthenticated(&app_state.hasura_auth) {
                            Some(identity) => (Some(identity.role), identity.session, false),
                            None => {
                                return Err(postrust_graphql::hasura::denied(
                                    postrust_auth::hasura::SECRET_MISSING,
                                ));
                            }
                        }
                    }
                    // No Hasura auth configured at all: unchanged from before
                    // any of this existed.
                    _ => (
                        postrust_auth::hasura::role_from_claims(&auth_result.claims),
                        postrust_auth::hasura::session_from_claims(&auth_result.claims),
                        false,
                    ),
                };

                Ok(GraphQLIdentity {
                    auth_result,
                    hasura_role,
                    session,
                    elevated,
                    headers,
                })
            }

            /// Everything between knowing who is asking and running what they
            /// asked: the checks Hasura makes before reading the document, the
            /// schema the role answers from, and the context every resolver
            /// reads the identity from.
            fn prepare_operation(
                app_state: &GraphQLAppState,
                identity: &GraphQLIdentity,
                req: async_graphql::Request,
            ) -> Result<PreparedOperation, serde_json::Value> {
                let GraphQLIdentity {
                    auth_result,
                    hasura_role,
                    session,
                    elevated,
                    headers,
                } = identity.clone();
                let Some(gql) = app_state.gql() else {
                    return Err(postrust_graphql::hasura::unavailable(
                        "the GraphQL schema could not be built; see the server log",
                    ));
                };
                let pairs: Vec<(&str, &str)> = headers
                    .iter()
                    .filter_map(|(name, value)| {
                        value.to_str().ok().map(|value| (name.as_str(), value))
                    })
                    .collect();

                // A protocol neither server implements, which only one of
                // them says. See `hasura::persisted_query`.
                if postrust_graphql::hasura::persisted_query(&req) {
                    return Err(postrust_graphql::hasura::not_supported(
                        "PersistedQueryNotSupported",
                    ));
                }

                // A header Hasura reads itself, before it reads the
                // document. `x-hasura-use-backend-only-permissions: random` is
                // not false, and treating it as false would send a
                // backend-only write down the path meant for everyone else on
                // a header the client thought it had set.
                if let Some(message) = postrust_auth::hasura::unreadable_boolean_header(&pairs[..])
                {
                    return Err(postrust_graphql::hasura::malformed(&message));
                }

                // Which schema answers. A permission is a statement about what
                // exists, so the role decides the shape of the API before it
                // decides anything about a row: a role the document does not
                // name has no schema at all, and is refused here rather than
                // being answered from someone else's.
                let Some(schema) = gql.schema_for(
                    hasura_role.as_deref(),
                    postrust_auth::hasura::backend_only_requested(&pairs[..], elevated),
                ) else {
                    return Err(postrust_graphql::hasura::denied(&format!(
                        "role \"{}\" is not defined in the permissions",
                        hasura_role.as_deref().unwrap_or_default()
                    )));
                };

                // Create SchemaCacheRef from the static Arc<SchemaCache>
                //
                // The unreduced cache, deliberately. The schema above decides
                // what may be asked for; this is what a resolver looks a type
                // up in once something has been asked, and a superset can only
                // answer the same questions.
                let schema_cache_ref = postrust_core::schema_cache::SchemaCacheRef::from_static(
                    (*gql.schema_cache).clone(),
                );

                // Every write in this operation goes into one transaction, and
                // this is the half that decides its fate: a mutation naming
                // several root fields is all-or-nothing, so the second one
                // failing has to take the first one's rows with it. Nothing to
                // settle for a query, which never opens it.
                let write: postrust_graphql::context::SharedWrite = Arc::default();

                let gql_ctx = postrust_graphql::context::GraphQLContext::new(
                    gql.pool.clone(),
                    schema_cache_ref,
                    auth_result,
                )
                .with_session(session)
                .with_identity(hasura_role.clone(), elevated)
                .with_write(Arc::clone(&write))
                .with_config(Arc::clone(&app_state.config));

                // A role may be told it cannot read the schema as data. Not a
                // permission on a table, so it is not settled by which schema
                // answers -- the schema is the thing being withheld.
                //
                // Not withheld from a caller holding the admin secret, whatever
                // role it then names. That is measured rather than assumed: a
                // v2.50.1 reference given `set_graphql_schema_introspection_
                // options` answers `__schema` for the named role when the
                // secret is sent beside the role header, and answers it from
                // that role's own restricted schema. Reading the schema is an
                // administrator's to do; the permissions still apply to it.
                let introspection_disabled_for = hasura_role
                    .as_deref()
                    .filter(|_| !elevated)
                    .filter(|role| gql.config.names.introspection_disabled(role));
                let prepared = postrust_graphql::hasura::prepare(
                    Some(schema),
                    req,
                    introspection_disabled_for,
                );
                let request = match prepared {
                    Ok(request) => request,
                    // Refused before it ran: nothing was written, so there is
                    // nothing to settle.
                    Err((_, errors)) => {
                        let mut response = async_graphql::Response::new(async_graphql::Value::Null);
                        response.errors = errors;
                        return Err(postrust_graphql::hasura::envelope(response));
                    }
                };
                let request = request
                    .data(gql_ctx)
                    .data(gql.pool.clone())
                    .data(Arc::clone(&gql.broker));
                Ok(PreparedOperation {
                    schema: schema.clone(),
                    request,
                    write,
                })
            }

            /// Settle an operation's writes before its answer goes out.
            async fn settle_write(
                write: &postrust_graphql::context::SharedWrite,
                response: &mut async_graphql::Response,
            ) {
                if let Some(tx) = write.lock().await.take() {
                    let settled = if response.errors.is_empty() {
                        tx.commit().await
                    } else {
                        tx.rollback().await
                    };
                    // A commit that fails is a mutation that did not happen,
                    // however well every statement in it went. Saying so is
                    // the whole point of running them together.
                    if let Err(e) = settled {
                        tracing::error!("GraphQL mutation could not be settled: {}", e);
                        response
                            .errors
                            .push(async_graphql::ServerError::new(e.to_string(), None));
                        response.data = async_graphql::Value::Null;
                    }
                }
            }

            async fn one_graphql(
                app_state: GraphQLAppState,
                headers: HeaderMap,
                req: async_graphql::Request,
            ) -> serde_json::Value {
                ensure_keys(&app_state, &headers).await;
                let identity = match graphql_identity(&app_state, headers) {
                    Ok(identity) => identity,
                    Err(refusal) => return refusal,
                };
                let PreparedOperation {
                    schema,
                    request,
                    write,
                } = match prepare_operation(&app_state, &identity, req) {
                    Ok(operation) => operation,
                    Err(refusal) => return refusal,
                };
                // One span for the operation, beneath the request's, named by
                // the operation as the GraphQL conventions have it. A span
                // and not async-graphql's OpenTelemetry extension: that is
                // built against opentelemetry 0.27, and this against 0.33.
                use tracing::Instrument as _;
                let span = tracing::debug_span!(
                    "graphql",
                    "graphql.operation.name" = request.operation_name.as_deref(),
                );
                let mut response = schema.execute(request).instrument(span).await;
                settle_write(&write, &mut response).await;
                postrust_graphql::hasura::envelope(response)
            }

            /// A Hasura-shaped body as the response a WebSocket carries, so a
            /// subscription is refused, and its errors read, exactly as the
            /// same operation over HTTP would be.
            fn hasura_response(body: serde_json::Value) -> async_graphql::Response {
                let data = body
                    .get("data")
                    .cloned()
                    .and_then(|data| async_graphql::Value::from_json(data).ok())
                    .unwrap_or(async_graphql::Value::Null);
                let mut response = async_graphql::Response::new(data);
                for error in body
                    .get("errors")
                    .and_then(serde_json::Value::as_array)
                    .into_iter()
                    .flatten()
                {
                    let message = error
                        .get("message")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or_default();
                    let mut server_error = async_graphql::ServerError::new(message, None);
                    if let Some(fields) = error
                        .get("extensions")
                        .and_then(serde_json::Value::as_object)
                    {
                        let mut extensions = async_graphql::ErrorExtensionValues::default();
                        for (name, value) in fields {
                            if let Ok(value) = async_graphql::Value::from_json(value.clone()) {
                                extensions.set(name, value);
                            }
                        }
                        server_error.extensions = Some(extensions);
                    }
                    response.errors.push(server_error);
                }
                response
            }

            /// The headers a WebSocket connection is authenticated with: the
            /// upgrade request's, overlaid with those the `connection_init`
            /// payload carries. A browser cannot set headers on a WebSocket,
            /// so a client puts `Authorization` in the payload -- Hasura's own
            /// clients under `headers`, others at the top level.
            fn connection_headers(
                mut headers: HeaderMap,
                payload: &serde_json::Value,
            ) -> HeaderMap {
                let nested = payload
                    .get("headers")
                    .and_then(serde_json::Value::as_object);
                for fields in [payload.as_object(), nested].into_iter().flatten() {
                    for (name, value) in fields {
                        let Some(value) = value.as_str() else {
                            continue;
                        };
                        if let (Ok(name), Ok(value)) = (
                            axum::http::HeaderName::from_bytes(name.as_bytes()),
                            axum::http::HeaderValue::from_str(value),
                        ) {
                            headers.insert(name, value);
                        }
                    }
                }
                headers
            }

            /// Runs every WebSocket operation -- query, mutation or
            /// subscription -- as the identity its connection was initialised
            /// with, through the same preparation an HTTP request gets.
            #[derive(Clone)]
            struct WebSocketExecutor(GraphQLAppState);

            impl async_graphql::Executor for WebSocketExecutor {
                async fn execute(
                    &self,
                    request: async_graphql::Request,
                ) -> async_graphql::Response {
                    use async_graphql::futures_util::StreamExt;
                    // Not reached over a WebSocket, which runs everything as a
                    // stream; answered from the stream so the two cannot differ.
                    self.execute_stream(request, None)
                        .next()
                        .await
                        .unwrap_or_else(|| async_graphql::Response::new(async_graphql::Value::Null))
                }

                fn execute_stream(
                    &self,
                    request: async_graphql::Request,
                    session_data: Option<Arc<async_graphql::Data>>,
                ) -> async_graphql::futures_util::stream::BoxStream<'static, async_graphql::Response>
                {
                    use async_graphql::futures_util::{stream, StreamExt};
                    let identity = session_data
                        .as_deref()
                        .and_then(|data| data.get(&std::any::TypeId::of::<GraphQLIdentity>()))
                        .and_then(|identity| identity.downcast_ref::<GraphQLIdentity>())
                        .cloned();
                    // `connection_init` refuses a connection it cannot
                    // identify, so an operation without an identity is one the
                    // protocol let through before initialising. Refused rather
                    // than run as nobody in particular.
                    let Some(identity) = identity else {
                        let refusal =
                            postrust_graphql::hasura::denied("the connection was not initialised");
                        return stream::once(async move { hasura_response(refusal) }).boxed();
                    };
                    match prepare_operation(&self.0, &identity, request) {
                        Err(refusal) => {
                            stream::once(async move { hasura_response(refusal) }).boxed()
                        }
                        Ok(PreparedOperation {
                            schema,
                            request,
                            write,
                        }) => {
                            async_graphql::Executor::execute_stream(&schema, request, session_data)
                                .then(move |mut response| {
                                    let write = Arc::clone(&write);
                                    async move {
                                        settle_write(&write, &mut response).await;
                                        hasura_response(postrust_graphql::hasura::envelope(
                                            response,
                                        ))
                                    }
                                })
                                .boxed()
                        }
                    }
                }
            }

            /// GraphQL over a WebSocket, authenticated as HTTP is. The
            /// identity is settled once, from `connection_init`, and a
            /// connection that would have been refused over HTTP -- a wrong
            /// admin secret, a role the token does not allow -- is refused
            /// there rather than served as the anonymous role.
            async fn graphql_ws(
                AxumState(app_state): AxumState<GraphQLAppState>,
                protocol: async_graphql_axum::GraphQLProtocol,
                headers: HeaderMap,
                ws: axum::extract::ws::WebSocketUpgrade,
            ) -> axum::response::Response {
                ws.protocols(async_graphql::http::ALL_WEBSOCKET_PROTOCOLS)
                    .on_upgrade(move |socket| async move {
                        let init_state = app_state.clone();
                        async_graphql_axum::GraphQLWebSocket::new(
                            socket,
                            WebSocketExecutor(app_state),
                            protocol,
                        )
                        .on_connection_init(move |payload| async move {
                            let headers = connection_headers(headers, &payload);
                            ensure_keys(&init_state, &headers).await;
                            match graphql_identity(&init_state, headers) {
                                Ok(identity) => {
                                    let mut data = async_graphql::Data::default();
                                    data.insert(identity);
                                    Ok(data)
                                }
                                Err(refusal) => Err(async_graphql::Error::new(
                                    refusal
                                        .pointer("/errors/0/message")
                                        .and_then(serde_json::Value::as_str)
                                        .unwrap_or("connection refused"),
                                )),
                            }
                        })
                        .serve()
                        .await
                    })
            }

            // Add GraphQL routes with WebSocket support for subscriptions
            let graphql_router = Router::new()
                .route("/", post(handle_graphql))
                .route("/", get(postrust_graphql::handler::graphql_playground))
                .with_state(graphql_app_state.clone());

            // The same state as HTTP, because a WebSocket is authenticated the
            // same way: `postrust_graphql::handler::graphql_ws_handler` carries
            // no caller identity, so every subscription it served ran without
            // a role, a session or claims.
            let ws_router = Router::new()
                .route("/ws", get(graphql_ws))
                .with_state(graphql_app_state);

            let graphql_app = match host.websockets {
                true => graphql_router.merge(ws_router),
                false => graphql_router,
            };
            // `/v1/graphql` is where a Hasura client sends its queries, and it is
            // the only address most of them can be told about: the endpoint is
            // baked into generated clients and codegen configs. `/api/graphql`
            // keeps working for anything already pointed at it, and
            // `/v1alpha1/graphql` is the address Hasura served before `/v1`
            // and still answers on -- a client old enough to have been
            // pointed there is exactly the one that cannot be repointed.
            app = app
                .nest("/v1/graphql", graphql_app.clone())
                .nest("/v1alpha1/graphql", graphql_app.clone())
                .nest("/api/graphql", graphql_app);
        }
    }

    // PostgREST compatibility mode: also serve the REST surface at the root so
    // canonical PostgREST paths (`/rpc/<name>`, `/<table>`) work in addition to
    // the `/api`-prefixed paths. Explicit routes (`/`, `/_`, `/admin`, `/api`,
    // and whatever was given to `Builder::nest`) still take precedence; only
    // otherwise-unmatched paths hit this fallback.
    if config.compat_mode {
        app = app.fallback(handle_request);
        info!("PostgREST compatibility mode enabled: REST surface also served at /");

        // Key ordering is fixed when the binary is compiled, so asking for
        // compatibility at runtime cannot turn it on. Say so, rather than
        // leaving someone to discover the difference by diffing responses.
        if !cfg!(feature = "compat-key-order") {
            tracing::warn!(
                "compatibility mode: object keys will be returned in alphabetical order, \
                 not in select order as PostgREST returns them. Build with \
                 --features compat-key-order to match, at a cost of up to 15% throughput \
                 on wide rows."
            );
        }
    }

    // Add root info endpoint.
    //
    // Not in compatibility mode: there `/` is part of the API surface -- it is
    // where PostgREST serves the schema description, and where it reports an
    // `Accept-Profile` naming a schema that is not exposed. A directory of
    // this server's own endpoints in its place answers a different question
    // from the one asked.
    if !config.compat_mode {
        app = app.route(
            "/",
            axum::routing::get(|| async {
                Json(serde_json::json!({
                    "name": "postrust",
                    "version": env!("CARGO_PKG_VERSION"),
                    "api": "/api",
                    "custom": "/_",
                    "health": "/_/health",
                    "admin": "/admin",
                    "docs": "/admin/swagger"
                }))
            }),
        );
    }

    // An empty list is "any origin", which is the default and what
    // `PGRST_SERVER_CORS_ORIGINS="*"` asks for. Anything that is not a valid
    // header value is dropped with a warning rather than quietly widening the
    // policy back to `*` -- a typo in an origin should cost that origin, not
    // the restriction.
    let allow_origin = if config.server_cors_origins.is_empty() {
        AllowOrigin::any()
    } else {
        let origins: Vec<HeaderValue> = config
            .server_cors_origins
            .iter()
            .filter_map(|o| match HeaderValue::from_str(o) {
                Ok(v) => Some(v),
                Err(_) => {
                    tracing::warn!("Ignoring CORS origin {:?}: not a valid header value", o);
                    None
                }
            })
            .collect();
        info!("CORS restricted to {} origin(s)", origins.len());
        AllowOrigin::list(origins)
    };

    // Apply CORS and state
    let app = app
        .layer(
            CorsLayer::new()
                .allow_origin(allow_origin)
                .allow_methods([
                    Method::GET,
                    Method::POST,
                    Method::PUT,
                    Method::PATCH,
                    Method::DELETE,
                    Method::OPTIONS,
                    Method::HEAD,
                ])
                .allow_headers(CorsAny)
                .expose_headers(CorsAny),
        )
        // For the routes whose handlers take a body *extractor* -- GraphQL,
        // the admin surface -- which is what this layer reaches. The REST
        // handler takes the whole `Request` and reads the body itself, so it
        // applies `max_body_size` directly; see `postrust_engine::handle`.
        .layer(DefaultBodyLimit::max(config.max_body_size))
        // Outermost, so it runs before the CORS layer -- which answers every
        // OPTIONS itself and never calls what it wraps, so nothing downstream
        // of it can say what a resource allows.
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            app::options_allow,
        ))
        // Outermost of all, so the request span covers everything above --
        // an OPTIONS answered by `options_allow`, a preflight answered by
        // CORS -- and every response, whoever made it, carries the request
        // id. A `Router::layer`, so the router has matched the route before
        // this runs and `MatchedPath` is there to name the span's
        // `http.route`. See `crate::telemetry`.
        .layer(crate::telemetry::RequestTraceLayer)
        .with_state(state.clone());

    Ok(App { router: app, state })
}

/// Reload the REST schema cache on `NOTIFY <channel>`.
///
/// Reconnects on its own: a listening connection is dropped by a restart of
/// the database or anything between, and a reloader that gives up on the first
/// error is one that stops working silently partway through a deployment.
fn spawn_schema_reloader(
    state: Arc<AppState>,
    channel: String,
    reloaded: tokio::sync::watch::Sender<u64>,
) {
    const RETRY: std::time::Duration = std::time::Duration::from_secs(5);

    tokio::spawn(async move {
        loop {
            let mut listener = match sqlx::postgres::PgListener::connect_with(&state.pool).await {
                Ok(listener) => listener,
                Err(e) => {
                    error!("Schema reloader could not connect: {}; retrying", e);
                    tokio::time::sleep(RETRY).await;
                    continue;
                }
            };

            if let Err(e) = listener.listen(&channel).await {
                error!("Schema reloader could not LISTEN on {}: {}", channel, e);
                tokio::time::sleep(RETRY).await;
                continue;
            }
            info!("Listening on channel {} for schema reloads", channel);

            // Inner loop until the connection fails, then reconnect.
            loop {
                match listener.recv().await {
                    Ok(notification) => {
                        info!("Schema reload requested: {:?}", notification.payload());
                        match state.reload_schema().await {
                            Ok(()) => {
                                info!(
                                    "Schema cache reloaded: {}",
                                    state.schema_cache().await.summary()
                                );
                                reloaded.send_modify(|n| *n += 1);
                                #[cfg(feature = "otel")]
                                crate::otel::schema_reloaded(true);
                            }
                            // The old cache is kept. Serving the previous
                            // schema is better than serving none, and the next
                            // notification tries again.
                            Err(e) => {
                                error!("Schema cache reload failed: {}", e);
                                #[cfg(feature = "otel")]
                                crate::otel::schema_reloaded(false);
                            }
                        }
                    }
                    Err(e) => {
                        error!("Schema reloader lost its connection: {}; reconnecting", e);
                        break;
                    }
                }
            }
        }
    });
}

/// Rebuild the GraphQL schema each time the REST schema cache is reloaded.
///
/// The new state is built beside the old one and swapped in only once it is
/// whole: a schema that cannot be built leaves the previous one serving, as a
/// failed reload of the REST cache does. Enum tables are read again, since a
/// migration may add a value; the permission document is kept from start-up,
/// since it is not part of what a migration notifies about. The subscription
/// broker carries over to the new state, so a subscription opened on the old
/// schema keeps receiving notifications, and listens on any table the new
/// one adds.
#[cfg(feature = "graphql")]
fn spawn_graphql_rebuilder(
    state: Arc<AppState>,
    current: Arc<std::sync::RwLock<Option<Arc<postrust_graphql::handler::GraphQLState>>>>,
    mut config: postrust_graphql::schema::SchemaConfig,
    mut reloads: tokio::sync::watch::Receiver<u64>,
) {
    use postrust_graphql::handler::GraphQLState;

    tokio::spawn(async move {
        while reloads.changed().await.is_ok() {
            let cache = Arc::new(state.schema_cache().await.clone());
            config.enum_values = read_enum_values(&state.pool, &cache, &config.names).await;
            let mut next = match GraphQLState::new(state.pool.clone(), cache, config.clone()) {
                Ok(next) => next,
                Err(e) => {
                    error!(
                        "GraphQL schema rebuild failed, keeping the previous one: {}",
                        e
                    );
                    continue;
                }
            };
            let previous = current.read().unwrap_or_else(|e| e.into_inner()).clone();
            if let Err(e) = next.continue_subscriptions(previous.as_deref()).await {
                // The tables already listened to still are; a new one is
                // answered by the refresh until the next reload retries it.
                tracing::warn!("Subscriptions on the rebuilt schema: {}", e);
            }
            *current.write().unwrap_or_else(|e| e.into_inner()) = Some(Arc::new(next));
            info!("GraphQL schema rebuilt");
        }
    });
}

/// The members of every table the GraphQL metadata marks as an enumeration,
/// keyed "schema.table". A table that cannot be read is left out, with a
/// warning, and becomes an ordinary table type.
#[cfg(feature = "graphql")]
async fn read_enum_values(
    pool: &sqlx::PgPool,
    cache: &postrust_core::schema_cache::SchemaCache,
    names: &postrust_graphql::names::NameOverrides,
) -> std::collections::HashMap<String, Vec<(String, Option<String>)>> {
    let mut values_of = std::collections::HashMap::new();
    for (schema, table) in names.enum_tables() {
        let qi = postrust_core::api_request::QualifiedIdentifier::new(&schema, &table);
        let Some(definition) = cache.get_table(&qi) else {
            tracing::warn!(
                "{}.{} is marked as an enumeration but was not found",
                schema,
                table
            );
            continue;
        };
        // One column identifies a member. A composite key names no
        // single value, so there is nothing to call the member.
        let [key_column] = definition.pk_cols.as_slice() else {
            tracing::warn!(
                "{}.{} is marked as an enumeration but its primary key is not one column",
                schema,
                table
            );
            continue;
        };
        // Hasura's convention, and a useful one: a `comment` column
        // describes each value. Read as a role that may read the
        // table, which a NOINHERIT authenticator is not by itself.
        match postrust_core::schema_cache::enum_values(
            pool,
            &schema,
            &table,
            key_column,
            definition.get_column("comment").is_some(),
        )
        .await
        {
            Ok(values) => {
                values_of.insert(format!("{}.{}", schema, table), values);
            }
            Err(e) => {
                tracing::warn!("cannot read the values of {}.{}: {}", schema, table, e)
            }
        }
    }
    values_of
}

/// Mask database URI for logging.
fn mask_db_uri(uri: &str) -> String {
    if let Some(at_pos) = uri.find('@') {
        if let Some(proto_end) = uri.find("://") {
            return format!("{}://***@{}", &uri[..proto_end], &uri[at_pos + 1..]);
        }
    }
    uri.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn routes(prefixes: &[&str]) -> Vec<(String, Router<Arc<AppState>>)> {
        prefixes
            .iter()
            .map(|prefix| (prefix.to_string(), Router::new()))
            .collect()
    }

    fn refusal(prefixes: &[&str]) -> String {
        match mount_points(routes(prefixes)) {
            Ok(_) => panic!("{prefixes:?} was accepted"),
            Err(e) => e.to_string(),
        }
    }

    #[test]
    fn a_prefix_is_normalized() {
        for given in ["hooks", "/hooks", "/hooks/", " /hooks// "] {
            let nests = mount_points(routes(&[given])).unwrap();
            assert_eq!(nests[0].0, "/hooks", "{given:?}");
        }
    }

    #[test]
    fn the_root_and_malformed_prefixes_are_refused() {
        for given in ["", "/", "//", "/a//b", "/{id}", "/files/{*rest}", "/*"] {
            refusal(&[given]);
        }
    }

    #[test]
    fn a_prefix_overlapping_the_servers_own_paths_is_refused() {
        for given in [
            "/api",
            "/api/hooks",
            "/healthz",
            "/v1",
            "/v1/version",
            "/v1/graphql/extra",
            "/v1alpha1",
            "/admin",
            "/_",
            "/_/health",
            "/_/ready/deep",
        ] {
            let refused = refusal(&[given]);
            assert!(refused.contains("serves"), "{given:?}: {refused}");
        }
    }

    #[test]
    fn a_prefix_sharing_only_characters_with_a_reserved_one_is_accepted() {
        let nests = mount_points(routes(&[
            "/apis",
            "/v1/other",
            "/administration",
            "/_/hooks",
            "/__",
        ]))
        .unwrap();
        assert_eq!(nests.len(), 5);
    }

    #[test]
    fn two_prefixes_that_overlap_are_refused() {
        assert!(refusal(&["/hooks", "hooks/"]).contains("nested earlier"));
        assert!(refusal(&["/hooks", "/hooks/stripe"]).contains("nested earlier"));
        assert!(refusal(&["/hooks/stripe", "/hooks"]).contains("nested earlier"));
        assert!(refusal(&["/_/hooks", "/_/hooks"]).contains("nested earlier"));
    }

    /// Whatever `mount_points` accepts, the router takes without panicking.
    /// The built-in routes are laid out as `assemble` lays them out, with
    /// every feature's paths present whether or not the feature is compiled
    /// in, and each nested router serving something, since an empty one
    /// claims no path.
    #[test]
    fn what_is_accepted_does_not_panic_the_router() {
        let stub = || Router::new().route("/", any(|| async {}));
        let nests = mount_points(
            [
                "/_/hooks",
                "/_/jobs/nightly",
                "/hooks",
                "/v1/other",
                "/apis",
            ]
            .into_iter()
            .map(|prefix| (prefix.to_string(), stub()))
            .collect(),
        )
        .unwrap();
        let mut app: Router<Arc<AppState>> = Router::new()
            .nest("/api", Router::new().route("/{*path}", any(|| async {})))
            .route("/healthz", any(|| async {}))
            .route("/v1/version", any(|| async {}))
            .nest(CUSTOM_PREFIX, custom::custom_router());
        for (prefix, router) in nests {
            app = app.nest(&prefix, router);
        }
        let _ = app
            .nest("/admin", stub())
            .nest("/v1/graphql", stub())
            .nest("/v1alpha1/graphql", stub())
            .nest("/api/graphql", stub())
            .route("/", any(|| async {}))
            .fallback(|| async {});
    }
}
