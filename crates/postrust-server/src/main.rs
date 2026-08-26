//! Postrust HTTP Server.
//!
//! A PostgREST-compatible REST API server for PostgreSQL.

use anyhow::Result;
use axum::{http::Method, response::Json, routing::any, Router};
use sqlx::postgres::PgPoolOptions;
use std::sync::Arc;
use tokio::sync::RwLock;
use tower_http::cors::{Any as CorsAny, CorsLayer};
use tracing::info;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

mod app;
mod custom;
mod state;

#[cfg(feature = "admin-ui")]
mod admin;

#[cfg(feature = "admin-ui")]
use axum::routing::{get, post};

use app::handle_request;
use state::AppState;

// musl's built-in allocator contends badly across threads, which shows up on
// exactly the paths that allocate per row. Building the Alpine image with it
// cost roughly 4-6x on the paged and embedded scenarios compared with the same
// code on glibc. glibc builds keep their own allocator, which performs fine.
#[cfg(target_env = "musl")]
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize tracing
    tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::new(
            std::env::var("RUST_LOG").unwrap_or_else(|_| "postrust=info".into()),
        ))
        .with(tracing_subscriber::fmt::layer())
        .init();

    // Load configuration
    let config = postrust_core::AppConfig::from_env();
    info!("Starting Postrust server");
    info!("Database: {}", mask_db_uri(&config.db_uri));

    // Create database pool
    let pool = PgPoolOptions::new()
        .max_connections(config.db_pool_size)
        .connect(&config.db_uri)
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

    // Create app state
    let state = Arc::new(AppState {
        pool,
        schema_cache: RwLock::new(schema_cache),
        config: config.clone(),
        jwt_config: postrust_auth::JwtConfig {
            secret: config.jwt_secret.clone(),
            secret_is_base64: config.jwt_secret_is_base64,
            audience: config.jwt_aud.clone(),
            role_claim_key: config.jwt_role_claim_key.clone(),
            anon_role: config.db_anon_role.clone(),
        },
    });

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
    app = app.nest("/_", custom::custom_router());
    info!("Custom routes enabled at /_");

    // Add admin routes and GraphQL endpoint if feature is enabled
    #[cfg(feature = "admin-ui")]
    {
        use async_graphql_axum::GraphQLBatchRequest as GqlRequest;
        use axum::extract::State as AxumState;
        use axum::http::HeaderMap;
        use postrust_graphql::handler::GraphQLState;
        use postrust_graphql::schema::SchemaConfig;

        info!("Admin UI enabled at /admin");
        app = app.nest("/admin", admin::admin_router());

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
        // rather than reflected. Once, at startup: the values of a set of
        // allowed values are not expected to change under a running server,
        // and a GraphQL enum is part of the schema a client generated against.
        let mut graphql_enum_values: std::collections::HashMap<
            String,
            Vec<(String, Option<String>)>,
        > = std::collections::HashMap::new();
        {
            let cache = state.schema_cache.read().await;
            for (schema, table) in graphql_names.enum_tables() {
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
                // describes each value.
                let comment = if definition.get_column("comment").is_some() {
                    format!("{}::text", postrust_sql::escape_ident("comment"))
                } else {
                    "NULL::text".to_string()
                };
                let sql = format!(
                    "SELECT {}::text, {} FROM {}.{} ORDER BY 1",
                    postrust_sql::escape_ident(key_column),
                    comment,
                    postrust_sql::escape_ident(&schema),
                    postrust_sql::escape_ident(&table)
                );
                match sqlx::query_as::<_, (Option<String>, Option<String>)>(&sql)
                    .fetch_all(&state.pool)
                    .await
                {
                    Ok(rows) => {
                        graphql_enum_values.insert(
                            format!("{}.{}", schema, table),
                            rows.into_iter()
                                .filter_map(|(value, comment)| value.map(|v| (v, comment)))
                                .collect(),
                        );
                    }
                    Err(e) => {
                        tracing::warn!("cannot read the values of {}.{}: {}", schema, table, e)
                    }
                }
            }
        }

        // How often a live query re-reads itself with nothing having
        // notified it. The notifications do the work; this is the floor
        // under what a trigger cannot see -- a view, an embedded row on a
        // table with no trigger, a predicate written against the clock.
        let subscription_refresh_seconds = std::env::var("PGRST_SUBSCRIPTION_REFRESH")
            .ok()
            .and_then(|value| value.trim().parse::<u64>().ok())
            .unwrap_or(30);

        let graphql_config = SchemaConfig {
            enable_subscriptions: true,
            subscription_refresh_seconds,
            max_rows: config.db_max_rows,
            // The GraphQL schema was built for `public` whatever the server
            // was told to expose, so a table in any other schema of
            // `PGRST_DB_SCHEMAS` was reachable over REST and invisible over
            // GraphQL.
            exposed_schemas: config.db_schemas.clone(),
            enum_values: graphql_enum_values,
            names: graphql_names,
            ..SchemaConfig::default()
        };
        // A schema that cannot be built is a reason to serve without GraphQL,
        // not a reason to exit. The REST surface, the admin UI and the health
        // endpoint do not depend on it, and taking the process down with it
        // turns one unrepresentable table into a server that will not start.
        let built = GraphQLState::new(state.pool.clone(), schema_cache_arc.clone(), graphql_config);
        if let Err(e) = &built {
            tracing::error!(
                "GraphQL schema could not be built, serving without it: {}",
                e
            );
        }
        if let Ok(graphql_state) = built {
            let graphql_state = Arc::new(graphql_state);

            // Initialize subscription broker
            if let Err(e) = graphql_state.init_subscriptions().await {
                tracing::warn!("Failed to initialize subscription broker: {}. Subscriptions may not work until triggers are created.", e);
            } else {
                info!("GraphQL subscriptions enabled");
            }

            info!("GraphQL endpoint enabled at /api/graphql");

            // Combined state for GraphQL routes (includes JWT config for auth)
            #[derive(Clone)]
            struct GraphQLAppState {
                gql_state: Arc<GraphQLState>,
                jwt_config: postrust_auth::JwtConfig,
                hasura_auth: postrust_auth::HasuraAuthConfig,
            }

            let graphql_app_state = GraphQLAppState {
                gql_state: graphql_state.clone(),
                jwt_config: state.jwt_config.clone(),
                hasura_auth: postrust_auth::HasuraAuthConfig {
                    admin_secret: config.hasura_admin_secret.clone(),
                    unauthorized_role: config.hasura_unauthorized_role.clone(),
                },
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
                headers: HeaderMap,
                req: GqlRequest,
            ) -> Json<serde_json::Value> {
                match req.0 {
                    async_graphql::BatchRequest::Single(request) => {
                        Json(one_graphql(app_state, headers, request).await)
                    }
                    async_graphql::BatchRequest::Batch(requests) => {
                        let mut answers = Vec::with_capacity(requests.len());
                        for request in requests {
                            answers.push(
                                one_graphql(app_state.clone(), headers.clone(), request).await,
                            );
                        }
                        Json(serde_json::Value::Array(answers))
                    }
                }
            }

            async fn one_graphql(
                app_state: GraphQLAppState,
                headers: HeaderMap,
                req: async_graphql::Request,
            ) -> serde_json::Value {
                // Extract auth header and authenticate
                let auth_header = headers.get("authorization").and_then(|v| v.to_str().ok());

                let token = postrust_auth::authenticate(auth_header, &app_state.jwt_config);
                // A token that was offered and verified. Not the same as
                // `token.is_ok()`, which is also true of the anonymous
                // fallback -- and an anonymous request is precisely the one
                // that has not been authenticated.
                let token_verified = auth_header.is_some() && token.is_ok();

                let auth_result = match token {
                    Ok(auth) => auth,
                    Err(e) => {
                        tracing::debug!("GraphQL auth failed: {}, using anon role", e);
                        postrust_auth::AuthResult {
                            role: app_state
                                .jwt_config
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

                let (hasura_role, session, elevated) =
                    match postrust_auth::hasura::from_admin_secret(
                        &app_state.hasura_auth,
                        &pairs[..],
                    ) {
                        postrust_auth::SecretOutcome::Accepted(identity) => {
                            (Some(identity.role), identity.session, identity.elevated)
                        }
                        postrust_auth::SecretOutcome::Rejected => {
                            return postrust_graphql::hasura::denied(
                                postrust_auth::hasura::SECRET_INVALID,
                            );
                        }
                        // Nothing settled by the secret. A verified token speaks
                        // next, and session variables come from its claims -- the
                        // headers are not read here, because on a server with no
                        // secret to gate them they would let any caller name its
                        // own identity.
                        _ if token_verified => (
                            hasura_role_from_claims(&auth_result.claims),
                            hasura_session_from_claims(&auth_result.claims),
                            false,
                        ),
                        // A secret is configured and nothing authenticated this
                        // request. Answering it as a stranger is a choice the
                        // deployment makes; refusing is the default.
                        _ if app_state.hasura_auth.is_configured() => {
                            match postrust_auth::hasura::unauthenticated(&app_state.hasura_auth) {
                                Some(identity) => (Some(identity.role), identity.session, false),
                                None => {
                                    return postrust_graphql::hasura::denied(
                                        postrust_auth::hasura::SECRET_MISSING,
                                    );
                                }
                            }
                        }
                        // No Hasura auth configured at all: unchanged from before
                        // any of this existed.
                        _ => (
                            hasura_role_from_claims(&auth_result.claims),
                            hasura_session_from_claims(&auth_result.claims),
                            false,
                        ),
                    };

                // Which schema answers. A permission is a statement about what
                // exists, so the role decides the shape of the API before it
                // decides anything about a row: a role the document does not
                // name has no schema at all, and is refused here rather than
                // being answered from someone else's.
                let Some(schema) = app_state.gql_state.schema_for(hasura_role.as_deref()) else {
                    return postrust_graphql::hasura::denied(&format!(
                        "role \"{}\" is not defined in the permissions",
                        hasura_role.as_deref().unwrap_or_default()
                    ));
                };

                // Create SchemaCacheRef from the static Arc<SchemaCache>
                //
                // The unreduced cache, deliberately. The schema above decides
                // what may be asked for; this is what a resolver looks a type
                // up in once something has been asked, and a superset can only
                // answer the same questions.
                let schema_cache_ref = postrust_core::schema_cache::SchemaCacheRef::from_static(
                    (*app_state.gql_state.schema_cache).clone(),
                );

                // Every write in this operation goes into one transaction, and
                // this is the half that decides its fate: a mutation naming
                // several root fields is all-or-nothing, so the second one
                // failing has to take the first one's rows with it. Nothing to
                // settle for a query, which never opens it.
                let write: postrust_graphql::context::SharedWrite = Arc::default();

                let gql_ctx = postrust_graphql::context::GraphQLContext::new(
                    app_state.gql_state.pool.clone(),
                    schema_cache_ref,
                    auth_result,
                )
                .with_session(session)
                .with_identity(hasura_role, elevated)
                .with_write(Arc::clone(&write));

                let prepared = postrust_graphql::hasura::prepare(Some(schema), req);
                let request = match prepared {
                    Ok(request) => request,
                    // Refused before it ran: nothing was written, so there is
                    // nothing to settle.
                    Err((_, errors)) => {
                        let mut response = async_graphql::Response::new(async_graphql::Value::Null);
                        response.errors = errors;
                        return postrust_graphql::hasura::envelope(response);
                    }
                };
                let request = request
                    .data(gql_ctx)
                    .data(app_state.gql_state.pool.clone())
                    .data(Arc::clone(&app_state.gql_state.broker));
                let mut response = schema.execute(request).await;

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

                postrust_graphql::hasura::envelope(response)
            }

            /// The Hasura role a verified token names, if it names one.
            ///
            /// `x-hasura-role` where the token fixes the role outright, and
            /// `x-hasura-default-role` where it offers a default beside the
            /// `x-hasura-allowed-roles` it may choose among -- which is the
            /// shape Hasura's own documentation mints. Either spelling may sit
            /// at the top level or under the namespace, and both are read.
            ///
            /// `None` leaves the database role standing in, which is what
            /// happened before the two were told apart.
            fn hasura_role_from_claims(
                claims: &std::collections::HashMap<String, serde_json::Value>,
            ) -> Option<String> {
                const NAMESPACE: &str = "https://hasura.io/jwt/claims";

                let read = |source: &std::collections::HashMap<String, serde_json::Value>| {
                    for wanted in ["x-hasura-role", "x-hasura-default-role"] {
                        let found = source.iter().find_map(|(key, value)| {
                            (key.to_ascii_lowercase() == wanted)
                                .then(|| value.as_str())
                                .flatten()
                        });
                        if let Some(role) = found.filter(|role| !role.is_empty()) {
                            return Some(role.to_string());
                        }
                    }
                    None
                };

                if let Some(serde_json::Value::Object(namespaced)) = claims.get(NAMESPACE) {
                    let namespaced: std::collections::HashMap<String, serde_json::Value> =
                        namespaced.clone().into_iter().collect();
                    if let Some(role) = read(&namespaced) {
                        return Some(role);
                    }
                }
                read(claims)
            }

            /// Collect `x-hasura-*` claims from a verified token.
            ///
            /// Hasura puts them under `https://hasura.io/jwt/claims` by default and
            /// allows them at the top level; both spellings are read, and the
            /// prefix is dropped so a policy names `hasura.user_id` rather than
            /// `hasura.x-hasura-user-id`. `x-hasura-role` is left out: the role is
            /// what the token was authenticated as, and re-reading it here would
            /// let a claim override that decision.
            fn hasura_session_from_claims(
                claims: &std::collections::HashMap<String, serde_json::Value>,
            ) -> std::collections::HashMap<String, String> {
                const NAMESPACE: &str = "https://hasura.io/jwt/claims";
                let mut session = std::collections::HashMap::new();

                let mut take = |key: &str, value: &serde_json::Value| {
                    let lowered = key.to_ascii_lowercase();
                    let Some(name) = lowered.strip_prefix("x-hasura-") else {
                        return;
                    };
                    if name == "role" || name.is_empty() {
                        return;
                    }
                    // A claim may be a string, a number or a list of ids; the
                    // setting is text either way, and a string keeps its own
                    // spelling rather than gaining quotes.
                    let rendered = match value {
                        serde_json::Value::String(text) => text.clone(),
                        other => other.to_string(),
                    };
                    session.insert(name.replace('-', "_"), rendered);
                };

                if let Some(serde_json::Value::Object(namespaced)) = claims.get(NAMESPACE) {
                    for (key, value) in namespaced {
                        take(key, value);
                    }
                }
                for (key, value) in claims {
                    take(key, value);
                }

                session
            }

            // Add GraphQL routes with WebSocket support for subscriptions
            let graphql_router = Router::new()
                .route("/", post(handle_graphql))
                .route("/", get(postrust_graphql::handler::graphql_playground))
                .with_state(graphql_app_state);

            // WebSocket handler needs separate state (just the GraphQL state)
            let ws_router = Router::new()
                .route("/ws", get(postrust_graphql::handler::graphql_ws_handler))
                .with_state(graphql_state);

            let graphql_app = graphql_router.merge(ws_router);
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
    // the `/api`-prefixed paths. Explicit routes (`/`, `/_`, `/admin`, `/api`)
    // still take precedence; only otherwise-unmatched paths hit this fallback.
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

    // Apply CORS and state
    let app = app
        .layer(
            CorsLayer::new()
                .allow_origin(CorsAny)
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
        // Outermost, so it runs before the CORS layer -- which answers every
        // OPTIONS itself and never calls what it wraps, so nothing downstream
        // of it can say what a resource allows.
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            app::options_allow,
        ))
        .with_state(state);

    // Start server
    let addr = format!("{}:{}", config.server_host, config.server_port);
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    info!("Listening on http://{}", addr);

    // Wrapped so that a request target carrying a raw `>` or `"` reaches the
    // router instead of being refused by the URI parser. See `lenient_uri`.
    axum::serve(postrust_server::lenient_uri::LenientListener(listener), app).await?;

    Ok(())
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
