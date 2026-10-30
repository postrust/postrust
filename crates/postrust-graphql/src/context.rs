//! GraphQL context containing auth, database pool, and schema cache.

use postrust_auth::AuthResult;
use postrust_core::schema_cache::SchemaCacheRef;
use postrust_core::{AppConfig, Session};
use sqlx::PgPool;
use std::collections::HashMap;
use std::sync::Arc;

/// The transaction every write in one operation shares.
///
/// A GraphQL mutation may name several root fields, and the specification
/// resolves them one after another. Hasura runs the whole set in one
/// transaction, so a mutation whose second root field violates a constraint
/// leaves nothing behind from its first -- which is what a client sending
/// "create the order and its lines" is relying on. Opening a transaction per
/// resolver, as this used to, half-applies that mutation and reports failure.
///
/// Opened lazily by the first write and settled once the operation is answered:
/// committed when the response carries no errors, rolled back otherwise. A
/// query never touches it.
pub type SharedWrite = Arc<tokio::sync::Mutex<Option<sqlx::Transaction<'static, sqlx::Postgres>>>>;

/// The `hasura.*` settings a request's session variables and role need.
///
/// Free of the context so it can be read on its own, which is also the only
/// way to test it: a [`GraphQLContext`] needs a connection pool and none of
/// this does.
///
/// The setting name is built from the variable's own name after it has been
/// checked, and the value is bound rather than interpolated: `set_config`
/// takes both as arguments, where `SET LOCAL` would need the value written
/// into the statement.
pub fn session_settings_for(
    session: &HashMap<String, String>,
    role: &str,
) -> Vec<(String, String)> {
    let mut settings: Vec<(String, String)> = session
        .iter()
        .filter(|(name, _)| {
            !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        })
        .map(|(name, value)| (format!("hasura.{}", name), value.clone()))
        .collect();
    // The whole session as one document, for a function that takes it.
    // A setting rather than a bound parameter because the places that call
    // such a function are projections and correlated subselects, which
    // assemble SQL without a parameter list to add to -- and because it is
    // then set exactly where the role is, once per transaction.
    settings.push((
        "hasura.session".to_string(),
        hasura_session_document(session, role).to_string(),
    ));
    // Beside the document, the role on its own, because a policy asking who is
    // calling should not have to parse JSON to find out.
    settings.push(("hasura.role".to_string(), role.to_string()));
    settings
}

/// The session as a function reading `hasura_session` expects it.
///
/// Hasura hands a function a JSON object of the caller's session variables
/// under the names they arrive as -- `x-hasura-role`, `x-hasura-user-id` --
/// which is what a function body indexes into. They are held stripped and
/// lowercased for the settings a row-level policy reads, so the prefix goes
/// back on here.
pub fn hasura_session_document(session: &HashMap<String, String>, role: &str) -> serde_json::Value {
    let mut object = serde_json::Map::new();
    for (name, value) in session {
        object.insert(
            format!("x-hasura-{}", name.replace('_', "-")),
            serde_json::Value::String(value.clone()),
        );
    }
    object.insert(
        "x-hasura-role".to_string(),
        serde_json::Value::String(role.to_string()),
    );
    serde_json::Value::Object(object)
}

/// Context available to all GraphQL resolvers.
pub struct GraphQLContext {
    /// Database connection pool.
    pub pool: PgPool,
    /// Schema cache for table/column metadata.
    pub schema_cache: SchemaCacheRef,
    /// Authentication result with role and claims.
    pub auth: AuthResult,
    /// Session variables carried into the transaction, without their
    /// `x-hasura-` prefix and lowercased: `user_id`, `org_id`.
    ///
    /// This is the half of Hasura's permission model that transfers. Its rules
    /// live in metadata and are compiled into every query; here permissions
    /// live in the database as roles and row level security, and what a policy
    /// needs is the caller's identity. A policy written against
    /// `current_setting('hasura.user_id')` sees what the Hasura permission
    /// would have seen.
    pub session: HashMap<String, String>,
    /// The role the caller speaks as, in Hasura's sense, where that is not the
    /// database role the transaction runs as.
    ///
    /// The two are different things and conflating them was the reason
    /// `hasura_session->>'x-hasura-role'` used to answer with the name of the
    /// connecting database user. Hasura's roles are labels -- `Artist`,
    /// `anonymous` -- that exist in no catalogue and could not be `SET ROLE`
    /// to; what they decide is which permissions apply and what a function
    /// reading the session document sees. Which database user the transaction
    /// runs as is [`AuthResult::role`], and it is settled separately.
    ///
    /// `None` when nothing named a Hasura role, in which case the database
    /// role stands in for it, as it did before the two were told apart.
    pub hasura_role: Option<String>,
    /// Whether the admin secret authenticated this request.
    ///
    /// Not the same question as whether the role is `admin`, which an
    /// administrator gives up as soon as it asks to be treated as someone
    /// else. What this gates is the fields a `backend_only` permission hides:
    /// reachable by a caller that proved it holds the secret, whatever role it
    /// then claims, and by no one else.
    pub elevated: bool,
    /// The transaction every write in this operation shares. See [`SharedWrite`].
    pub write: SharedWrite,
    /// The server's configuration, which a transaction is set up from.
    ///
    /// What [`Self::session`] reads beyond the caller: a role's own `statement_timeout` and isolation level, the server-wide
    /// isolation level and `app_settings`. The same configuration REST's
    /// transactions are set up from, so a function or a policy sees the same
    /// thing whichever of the two called it.
    pub config: Arc<AppConfig>,
}

impl GraphQLContext {
    /// Create a new GraphQL context.
    pub fn new(pool: PgPool, schema_cache: SchemaCacheRef, auth: AuthResult) -> Self {
        Self {
            pool,
            schema_cache,
            auth,
            session: HashMap::new(),
            hasura_role: None,
            elevated: false,
            write: Arc::new(tokio::sync::Mutex::new(None)),
            config: Arc::new(AppConfig::default()),
        }
    }

    /// Set up this request's transactions from `config`.
    ///
    /// Without it they are set up from [`AppConfig::default`]: PostgREST's
    /// defaults, and no role settings.
    pub fn with_config(mut self, config: Arc<AppConfig>) -> Self {
        self.config = config;
        self
    }

    /// Record who the caller is in Hasura's sense, and whether it proved it.
    pub fn with_identity(mut self, role: Option<String>, elevated: bool) -> Self {
        self.hasura_role = role;
        self.elevated = elevated;
        self
    }

    /// The role the caller speaks as: the Hasura role where one was named, and
    /// the database role otherwise.
    pub fn acting_role(&self) -> &str {
        self.hasura_role.as_deref().unwrap_or(&self.auth.role)
    }

    /// Who is asking, for the SQL builders that have to narrow rows to them.
    ///
    /// The Hasura role rather than [`Self::acting_role`]: a permission is
    /// keyed by a role someone wrote into a document, and the database role
    /// standing in for a missing one would name no permission and read as
    /// unrestricted -- which is right, and is worth being explicit about.
    pub fn caller(&self) -> crate::role::Caller<'_> {
        crate::role::Caller {
            role: self.hasura_role.as_deref(),
            session: &self.session,
        }
    }

    /// Share one transaction with whoever is going to settle it.
    ///
    /// The caller keeps the other handle: it is the only thing that knows
    /// whether the operation ended in errors, and so the only thing that can
    /// decide between commit and rollback.
    pub fn with_write(mut self, write: SharedWrite) -> Self {
        self.write = write;
        self
    }

    /// Carry session variables into every transaction this request opens.
    pub fn with_session(mut self, session: HashMap<String, String>) -> Self {
        self.session = session;
        self
    }

    /// What makes a transaction this request's: the statements that turn a
    /// pooled connection into the caller.
    ///
    /// The same [`Session`] a REST request's transaction is set up by, so the
    /// two cannot drift apart: one `SELECT set_config(...)` carrying the
    /// role's own `statement_timeout`, the database role,
    /// the verified JWT as `request.jwt.claims` and `request.jwt.claims.<name>`,
    /// and the `app.settings.*` -- and then, last, the `hasura.*` session
    /// variables, which no REST request has. Ahead of it, the isolation level
    /// where the role's setting or `PGRST_DB_TX_ISOLATION` asks for one other
    /// than PostgreSQL's default.
    ///
    /// The role is [`Self::role`], the database role, never the Hasura role
    /// a caller speaks as -- which exists in no catalogue and goes in only as
    /// `hasura.role` and inside `hasura.session`.
    ///
    /// Unlike REST, the `search_path` is left as the role and the database
    /// set it, as GraphQL has always left it and as Hasura does. An operation
    /// may touch every exposed schema, and the SQL generated for one names
    /// each table it reads with its schema, so the path decides nothing about
    /// which table a field reads. What it does decide is where the
    /// unqualified calls in that SQL -- PostGIS's `ST_AsGeoJSON` among them
    /// -- are found. Setting REST's path would stop finding an extension
    /// installed outside `db_extra_search_path` (in `extensions`, say, as
    /// Supabase installs them) that the role's own path finds, and a GraphQL
    /// query that worked would stop working.
    ///
    /// Nothing about the HTTP request goes in. `request.headers` would carry
    /// the admin secret, which [`postrust_auth::hasura`] keeps out of every
    /// setting a function can read; `request.method` and `request.path` are
    /// the same for every operation and tell a function nothing. And
    /// `db_pre_request` is not called: it is a hook on REST requests, written
    /// against the `request.*` settings it would find missing here, and GraphQL
    /// has never run it.
    ///
    /// Read-write, as a GraphQL transaction always has been, for a query as
    /// for a mutation. A query's SQL is a `SELECT`, and the query root serves
    /// no volatile function, so `SET TRANSACTION READ ONLY` would refuse
    /// little -- but whatever it did refuse, a volatile function a computed
    /// field calls say, is an answer that changes, and that is a change of its
    /// own to make and measure rather than one to carry in with this.
    pub fn session(&self) -> Session<'_> {
        session_settings_for(&self.session, self.acting_role())
            .into_iter()
            .fold(
                Session::new(&self.config, self.role(), self.auth.claim_settings())
                    .pre_request(false)
                    .set_search_path(false),
                |session, (name, value)| session.setting(name, value),
            )
    }

    /// The session as a function reading `hasura_session` expects it.
    ///
    /// Hasura hands a function a JSON object of the caller's session
    /// variables, under the names they arrive as -- `x-hasura-role`,
    /// `x-hasura-user-id` -- which is what a function body indexes into.
    /// [`Self::session`] holds them stripped and lowercased for the settings a
    /// row-level policy reads, so the prefix goes back on here.
    ///
    /// The role is included and is not the caller's to choose: it is what
    /// authenticated the request, or the role an authenticated administrator
    /// asked to be treated as -- never a bare header.
    pub fn hasura_session(&self) -> serde_json::Value {
        hasura_session_document(&self.session, self.acting_role())
    }

    /// The database role this request's transaction runs as.
    ///
    /// Not [`Self::acting_role`], which is who the caller says it is. The
    /// session's `role` setting takes this one, because it is the only one of
    /// the two that has to exist in the catalogue.
    pub fn role(&self) -> &str {
        &self.auth.role
    }

    /// Get a claim value.
    pub fn claim(&self, key: &str) -> Option<&serde_json::Value> {
        self.auth.claims.get(key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_auth() -> AuthResult {
        let mut claims = HashMap::new();
        claims.insert("user_id".into(), serde_json::json!(123));
        claims.insert("role".into(), serde_json::json!("admin"));

        AuthResult {
            role: "authenticated".into(),
            claims,
        }
    }

    #[test]
    fn test_context_role() {
        let auth = create_test_auth();
        // Note: We can't fully test without a pool, but we can test the auth part
        assert_eq!(auth.role, "authenticated");
    }

    #[test]
    fn test_context_claim() {
        let auth = create_test_auth();
        assert_eq!(auth.claims.get("user_id"), Some(&serde_json::json!(123)));
    }

    fn session(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(name, value)| (name.to_string(), value.to_string()))
            .collect()
    }

    fn setting<'a>(settings: &'a [(String, String)], name: &str) -> Option<&'a str> {
        settings
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    }

    #[test]
    fn the_session_document_says_which_role_is_asking() {
        let document = hasura_session_document(&session(&[("user_id", "1")]), "user");
        assert_eq!(document["x-hasura-role"], serde_json::json!("user"));
        assert_eq!(document["x-hasura-user-id"], serde_json::json!("1"));
    }

    #[test]
    fn a_session_name_gets_its_dashes_back_in_the_document() {
        // `SET LOCAL` cannot carry a dash in a setting name, so the variables
        // are held with underscores -- and a function body indexes into the
        // document by the name the header arrived as.
        let document = hasura_session_document(&session(&[("infant_name", "Bittu")]), "infant");
        assert_eq!(document["x-hasura-infant-name"], serde_json::json!("Bittu"));
    }

    #[test]
    fn the_role_is_a_setting_of_its_own_beside_the_document() {
        let settings = session_settings_for(&session(&[("user_id", "1")]), "Artist");
        assert_eq!(setting(&settings, "hasura.role"), Some("Artist"));
        assert_eq!(setting(&settings, "hasura.user_id"), Some("1"));
        // And the document, for a function that takes the whole thing.
        let document: serde_json::Value =
            serde_json::from_str(setting(&settings, "hasura.session").unwrap()).unwrap();
        assert_eq!(document["x-hasura-role"], serde_json::json!("Artist"));
    }

    fn context(auth: AuthResult) -> GraphQLContext {
        GraphQLContext::new(
            PgPool::connect_lazy("postgres://localhost/test").expect("valid test URL"),
            SchemaCacheRef::default(),
            auth,
        )
    }

    /// Every setting a session sends, in the order it sends them, from the
    /// one `SELECT set_config(...)` that carries them.
    fn sent_settings(ctx: &GraphQLContext) -> Vec<(String, String)> {
        let statements = ctx.session().statements();
        let carrying = statements
            .iter()
            .find(|s| s.sql.starts_with("SELECT set_config("))
            .expect("one statement carries the settings");
        carrying
            .params
            .chunks(2)
            .map(|pair| match pair {
                [postrust_sql::SqlParam::Text(name), postrust_sql::SqlParam::Text(value)] => {
                    (name.clone(), value.clone())
                }
                other => panic!("{other:?}"),
            })
            .collect()
    }

    #[tokio::test]
    async fn graphql_transactions_receive_the_verified_jwt_claims_document() {
        let ctx = context(create_test_auth());

        let settings = sent_settings(&ctx);
        let claims: serde_json::Value =
            serde_json::from_str(setting(&settings, "request.jwt.claims").unwrap()).unwrap();

        assert_eq!(claims["user_id"], serde_json::json!(123));
        assert_eq!(claims["role"], serde_json::json!("authenticated"));
        assert_eq!(
            setting(&settings, "request.jwt.claims.user_id"),
            Some("123")
        );
        assert_eq!(
            setting(&settings, "request.jwt.claims.role"),
            Some("authenticated")
        );
    }

    /// The database role is what the transaction runs as, and the Hasura role
    /// goes in only as `hasura.*`. The role's own settings go before it, the
    /// `hasura.*` variables after everything REST sends, and nothing about an
    /// HTTP request goes in at all.
    #[tokio::test]
    async fn a_graphql_session_runs_as_the_database_role_with_the_hasura_variables_last() {
        let mut config = AppConfig::default();
        config.db_schemas = vec!["api".to_string(), "other".to_string()];
        config.db_extra_search_path = vec!["public".to_string(), "extensions".to_string()];
        config.db_pre_request = Some("check_request".to_string());
        let mut role = postrust_core::config::RoleSettings::default();
        role.isolation_level = Some(postrust_core::config::IsolationLevel::RepeatableRead);
        role.statement_timeout = Some(250);
        config
            .role_settings
            .insert("authenticated".to_string(), role);
        config
            .app_settings
            .insert("tenant".to_string(), "acme".to_string());
        let ctx = context(create_test_auth())
            .with_config(Arc::new(config))
            .with_session(session(&[("user_id", "7")]))
            .with_identity(Some("Artist".to_string()), false);

        let statements = ctx.session().statements();
        assert_eq!(
            statements[0].sql, "SET TRANSACTION ISOLATION LEVEL REPEATABLE READ",
            "the role's isolation level"
        );
        assert_eq!(
            statements.len(),
            2,
            "neither read-only nor db_pre_request: {statements:?}"
        );

        let settings = sent_settings(&ctx);
        let names: Vec<&str> = settings.iter().map(|(name, _)| name.as_str()).collect();
        assert_eq!(names[..2], ["statement_timeout", "role"]);
        assert_eq!(
            names[names.len() - 3..],
            ["hasura.user_id", "hasura.session", "hasura.role"]
        );
        // The claims and the app settings between them, in whichever order
        // the claims come.
        let mut middle = names[2..names.len() - 3].to_vec();
        middle.sort_unstable();
        assert_eq!(
            middle,
            [
                "app.settings.tenant",
                "request.jwt.claims",
                "request.jwt.claims.role",
                "request.jwt.claims.user_id",
            ]
        );
        assert_eq!(setting(&settings, "statement_timeout"), Some("250"));
        // Left as the role and the database set it: see `session`.
        assert_eq!(setting(&settings, "search_path"), None);
        assert_eq!(setting(&settings, "role"), Some("authenticated"));
        assert_eq!(setting(&settings, "hasura.role"), Some("Artist"));
    }

    #[test]
    fn a_name_that_could_not_be_a_setting_is_dropped_rather_than_written() {
        // The name goes into the statement and the value is bound, so a name
        // that is not an identifier is the one thing here that could be an
        // injection. It never reaches SQL.
        let settings = session_settings_for(
            &session(&[("user_id", "1"), ("bad'; drop table t; --", "x")]),
            "user",
        );
        assert!(setting(&settings, "hasura.user_id").is_some());
        assert!(settings
            .iter()
            .all(|(name, _)| !name.contains("drop table")));
    }
}
