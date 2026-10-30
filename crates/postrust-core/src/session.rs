//! What makes a transaction run as the caller.
//!
//! A request does nothing as itself. Its statements run on a pooled
//! connection logged in as the authenticator, and it is a handful of
//! statements at the top of the transaction that turn that connection into
//! the caller's: the role row-level security and grants are checked against,
//! the claims a policy reads back with `current_setting`, the facts about the
//! request a function may want, and the configuration's own pre-request
//! check. Get one of them wrong, or send it outside the transaction, and the
//! query runs as the authenticator with nothing set -- which is to say with
//! no security at all, and no error to say so.
//!
//! [`Session`] is those statements, worked out once. It knows nothing about
//! HTTP or about JWTs: the role and the claims come in as plain values, so it
//! serves a REST request, a GraphQL operation, or a hand-written route that
//! has a user and no request at all. It runs nothing itself either.
//! [`Session::statements`] says what would be sent, which is what the tests
//! look at, and [`Session::apply`] queues it on a [`Tx`].

use crate::config::{AppConfig, IsolationLevel};
use crate::db::Tx;
use indexmap::IndexMap;
use postrust_sql::SqlParam;

/// One statement a [`Session`] sends, with its parameters.
#[derive(Clone, Debug, PartialEq)]
pub struct SessionStatement {
    /// The statement's text.
    pub sql: String,
    /// Its parameters, numbered from `$1` in order.
    pub params: Vec<SqlParam>,
}

impl SessionStatement {
    fn new(sql: impl Into<String>, params: Vec<SqlParam>) -> Self {
        Self {
            sql: sql.into(),
            params,
        }
    }
}

/// The statements that make a transaction run as a caller.
///
/// Built from the caller's role and claim settings and the configuration;
/// everything about an HTTP request is optional and added with the builder
/// methods. A session with no request at all still sets the role, the claims,
/// the `search_path` -- the default schema and `db_extra_search_path` -- and
/// the `app.settings.*`, and still runs `db_pre_request`, so a hand-written
/// route sees what a policy or function expects to see. The request settings
/// it was not given (`request.method`, `request.path`, `request.headers`,
/// `request.cookies`) are left unset, which `current_setting(name, true)`
/// reads as null.
#[derive(Clone, Debug)]
pub struct Session<'a> {
    config: &'a AppConfig,
    role: String,
    claims: Vec<(String, String)>,
    schema: Option<String>,
    method: Option<String>,
    path: Option<String>,
    headers: Option<String>,
    cookies: Option<String>,
    trace_id: Option<String>,
    timezone: Option<String>,
    extra: Vec<(String, String)>,
    isolation: Option<IsolationLevel>,
    read_only: bool,
    pre_request: bool,
    set_search_path: bool,
}

impl<'a> Session<'a> {
    /// A session for `role`, carrying `claims` -- the `request.jwt.claims*`
    /// settings, as `postrust_auth::AuthResult::claim_settings` renders them.
    ///
    /// Read-write, with `db_pre_request` run if one is configured.
    pub fn new(
        config: &'a AppConfig,
        role: impl Into<String>,
        claims: Vec<(String, String)>,
    ) -> Self {
        Self {
            config,
            role: role.into(),
            claims,
            schema: None,
            method: None,
            path: None,
            headers: None,
            cookies: None,
            trace_id: None,
            timezone: None,
            extra: Vec::new(),
            isolation: None,
            read_only: false,
            pre_request: true,
            set_search_path: true,
        }
    }

    /// The schema the request is for, which leads the `search_path`. Without
    /// one it is the configuration's default schema.
    pub fn schema(mut self, schema: impl Into<String>) -> Self {
        self.schema = Some(schema.into());
        self
    }

    /// The request's method and path, as `request.method` and `request.path`.
    pub fn request(mut self, method: impl Into<String>, path: impl Into<String>) -> Self {
        self.method = Some(method.into());
        self.path = Some(path.into());
        self
    }

    /// The request's headers, as `request.headers`.
    pub fn headers(mut self, headers: &IndexMap<String, String>) -> Self {
        self.headers = Some(json_object(headers));
        self
    }

    /// The request's cookies, as `request.cookies`.
    pub fn cookies(mut self, cookies: &IndexMap<String, String>) -> Self {
        self.cookies = Some(json_object(cookies));
        self
    }

    /// The trace the request belongs to, as `request.trace_id`.
    ///
    /// So that a statement seen in `pg_stat_activity` or the server log -- by
    /// `auto_explain`, or a `log_line_prefix` reading the setting -- can be
    /// tied back to the trace of the request that sent it. Unset without one.
    pub fn trace_id(mut self, trace_id: Option<String>) -> Self {
        self.trace_id = trace_id;
        self
    }

    /// The session time zone, from `Prefer: timezone`.
    pub fn timezone(mut self, timezone: Option<String>) -> Self {
        self.timezone = timezone;
        self
    }

    /// One more setting, set after all of the others.
    ///
    /// For what a caller other than REST tells the database about itself --
    /// GraphQL's `hasura.*` session variables, say. The name is bound as a
    /// parameter like every other, so it is PostgreSQL that decides whether it
    /// is a name it will take.
    pub fn setting(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.extra.push((name.into(), value.into()));
        self
    }

    /// The transaction's isolation level, over the role's and the server's.
    ///
    /// Without one it is the role's own setting where it has one, and
    /// `PGRST_DB_TX_ISOLATION` otherwise. For a caller that knows what it
    /// needs of one transaction -- a hand-written route that reads twice and
    /// needs both reads to agree. `None` leaves the choice to the
    /// configuration.
    pub fn isolation(mut self, isolation: Option<IsolationLevel>) -> Self {
        self.isolation = isolation;
        self
    }

    /// Whether the transaction is read-only. See [`Session::statements`].
    pub fn read_only(mut self, read_only: bool) -> Self {
        self.read_only = read_only;
        self
    }

    /// Whether to set `search_path` to the schema and `db_extra_search_path`.
    /// On unless turned off.
    ///
    /// Off for SQL that qualifies every name it means to come from the
    /// API's schemas, and should otherwise resolve names as the role and the
    /// database normally would -- GraphQL's, which calls extension functions
    /// such as PostGIS's unqualified, wherever the database installed them.
    pub fn set_search_path(mut self, set_search_path: bool) -> Self {
        self.set_search_path = set_search_path;
        self
    }

    /// Whether to call `db_pre_request`, where one is configured. On unless
    /// turned off.
    pub fn pre_request(mut self, pre_request: bool) -> Self {
        self.pre_request = pre_request;
        self
    }

    /// The role this session runs as.
    pub fn role(&self) -> &str {
        &self.role
    }

    /// Queue this session's statements on `tx`, to go out ahead of its next
    /// statement -- see [`Tx::queue`]. They have to be the first statements
    /// on the transaction: `SET TRANSACTION` is refused after any other.
    pub fn apply<T: Tx>(&self, tx: &mut T) {
        for statement in self.statements() {
            tx.queue(&statement.sql, &statement.params);
        }
    }

    /// The statements [`Session::apply`] queues, in the order it queues them.
    ///
    /// At most four, and usually two: an isolation level, read-only, every
    /// setting in one `SELECT`, and the pre-request function.
    pub fn statements(&self) -> Vec<SessionStatement> {
        let mut statements = Vec::with_capacity(4);

        // Isolation level. `SET TRANSACTION` is only accepted before the
        // first statement of a transaction, so it goes here.
        //
        // One asked for with `isolation` wins, then a role's own setting, then
        // the server-wide `PGRST_DB_TX_ISOLATION`. All are closed enums
        // rendered by `to_sql` -- never caller text. Issued only when it
        // differs from PostgreSQL's own default, so the common case sends
        // nothing.
        let role_settings = self.config.role_settings.get(&self.role);
        let isolation = self
            .isolation
            .as_ref()
            .or_else(|| role_settings.and_then(|r| r.isolation_level.as_ref()))
            .unwrap_or(&self.config.db_tx_isolation);
        if *isolation != IsolationLevel::ReadCommitted {
            statements.push(SessionStatement::new(
                format!("SET TRANSACTION ISOLATION LEVEL {}", isolation.to_sql()),
                Vec::new(),
            ));
        }

        // A read runs in a read-only transaction.
        //
        // Without this a GET can change data: `GET /rpc/some_volatile_fn`
        // happily executes whatever the function does. PostgreSQL is the
        // right place to enforce it, since it covers anything reachable from
        // the statement -- a volatile function, a trigger, a rule -- rather
        // than only what the planner thought to look at.
        if self.read_only {
            statements.push(SessionStatement::new(
                "SET TRANSACTION READ ONLY",
                Vec::new(),
            ));
        }

        // Everything the caller tells the database about itself, in one
        // statement.
        //
        // These are `SET LOCAL` by another spelling -- `set_config(k, v,
        // true)` -- and a function or an RLS policy reads them back with
        // `current_setting`. Sent as one `SELECT` rather than one statement
        // each, which is a round trip per setting otherwise, and there are
        // eight of them before a request has done any work. The values stay
        // bound rather than interpolated: `set_config` takes both as
        // arguments, where `SET LOCAL` would need the value written into the
        // statement text.
        //
        // The order is PostgREST's: the settings a role carries, then the
        // role, then everything about the request. The role goes on after its
        // own settings so that a `GRANT SET ON PARAMETER` on the
        // authenticator still applies when they are set.
        let mut settings: Vec<(String, String)> = Vec::new();

        // The settings a role carries come first, before the `role` that
        // follows them -- see the note above on `GRANT SET ON PARAMETER`.
        if let Some(timeout) = role_settings.and_then(|r| r.statement_timeout) {
            settings.push(("statement_timeout".to_string(), timeout.to_string()));
        }

        if self.set_search_path {
            settings.push(("search_path".to_string(), self.search_path()));
        }
        settings.push(("role".to_string(), self.role.clone()));
        let request = [
            ("request.method", &self.method),
            ("request.path", &self.path),
            ("request.headers", &self.headers),
            ("request.cookies", &self.cookies),
            ("request.trace_id", &self.trace_id),
        ];
        for (name, value) in request {
            if let Some(value) = value {
                settings.push((name.to_string(), value.clone()));
            }
        }
        settings.extend(self.claims.iter().cloned());

        // `app_settings`, exposed as PostgREST spells them: a function or
        // policy reads one back with `current_setting('app.settings.<key>')`.
        for (key, value) in &self.config.app_settings {
            settings.push((format!("app.settings.{key}"), value.clone()));
        }

        // `Prefer: timezone` is applied to the session, so every value the
        // database renders -- and every `now()` a function calls -- sees it.
        // An unknown zone is PostgreSQL's to reject, and it does, with a
        // message naming the value the client sent.
        if let Some(timezone) = &self.timezone {
            settings.push(("timezone".to_string(), timezone.clone()));
        }

        settings.extend(self.extra.iter().cloned());

        let calls = (0..settings.len())
            .map(|i| format!("set_config(${}, ${}, true)", i * 2 + 1, i * 2 + 2))
            .collect::<Vec<_>>()
            .join(", ");
        let params = settings
            .into_iter()
            .flat_map(|(name, value)| [SqlParam::Text(name), SqlParam::Text(value)])
            .collect();
        statements.push(SessionStatement::new(format!("SELECT {calls}"), params));

        // `db_pre_request`: a function called before the caller's own query,
        // on the same transaction, so it sees every setting above and
        // anything it raises aborts the transaction. PostgREST's use for it
        // is a custom authorisation check that `RAISE`s to refuse.
        //
        // The name is an identifier from the configuration, not from the
        // caller, and is escaped as one -- a schema-qualified name stays two
        // identifiers so `auth.check` does not become `"auth.check"`.
        if let Some(pre_request) = self
            .config
            .db_pre_request
            .as_ref()
            .filter(|_| self.pre_request)
        {
            let qualified = pre_request
                .split('.')
                .map(postrust_sql::escape_ident)
                .collect::<Vec<_>>()
                .join(".");
            statements.push(SessionStatement::new(
                format!("SELECT {qualified}()"),
                Vec::new(),
            ));
        }

        statements
    }

    /// The request's schema, then `db_extra_search_path`, each escaped as an
    /// identifier.
    fn search_path(&self) -> String {
        let schema = self
            .schema
            .as_deref()
            .unwrap_or_else(|| self.config.default_schema());
        std::iter::once(schema)
            .chain(self.config.db_extra_search_path.iter().map(String::as_str))
            .map(postrust_sql::escape_ident)
            .collect::<Vec<_>>()
            .join(", ")
    }
}

/// Render a map of strings as a JSON object.
///
/// PostgreSQL will not accept every header name as a setting name -- a `-` is
/// not allowed in one from version 14 -- so the whole map goes into a single
/// setting as JSON and a function picks out what it wants with `->>`.
fn json_object(entries: &IndexMap<String, String>) -> String {
    serde_json::Value::Object(
        entries
            .iter()
            .map(|(name, value)| (name.clone(), serde_json::Value::String(value.clone())))
            .collect(),
    )
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::RoleSettings;
    use pretty_assertions::assert_eq;

    fn text(values: &[&str]) -> Vec<SqlParam> {
        values
            .iter()
            .map(|v| SqlParam::Text(v.to_string()))
            .collect()
    }

    fn set_config(n: usize) -> String {
        let calls = (0..n)
            .map(|i| format!("set_config(${}, ${}, true)", i * 2 + 1, i * 2 + 2))
            .collect::<Vec<_>>()
            .join(", ");
        format!("SELECT {calls}")
    }

    fn claims() -> Vec<(String, String)> {
        vec![
            (
                "request.jwt.claims".to_string(),
                r#"{"role":"web_anon"}"#.to_string(),
            ),
            (
                "request.jwt.claims.role".to_string(),
                "web_anon".to_string(),
            ),
        ]
    }

    /// The common case: a read on the default configuration sends read-only
    /// and the settings, and nothing else -- no isolation level, since it is
    /// PostgreSQL's default, and no pre-request function, since none is set.
    #[test]
    fn a_plain_read() {
        let config = AppConfig::default();
        let mut headers = IndexMap::new();
        headers.insert("accept".to_string(), "application/json".to_string());
        let mut cookies = IndexMap::new();
        cookies.insert("session".to_string(), "abc".to_string());

        let session = Session::new(&config, "web_anon", claims())
            .schema("api")
            .request("GET", "/items")
            .headers(&headers)
            .cookies(&cookies)
            .timezone(None)
            .read_only(true);

        assert_eq!(
            session.statements(),
            vec![
                SessionStatement::new("SET TRANSACTION READ ONLY", vec![]),
                SessionStatement::new(
                    set_config(8),
                    text(&[
                        "search_path",
                        r#""api", "public""#,
                        "role",
                        "web_anon",
                        "request.method",
                        "GET",
                        "request.path",
                        "/items",
                        "request.headers",
                        r#"{"accept":"application/json"}"#,
                        "request.cookies",
                        r#"{"session":"abc"}"#,
                        "request.jwt.claims",
                        r#"{"role":"web_anon"}"#,
                        "request.jwt.claims.role",
                        "web_anon",
                    ]),
                ),
            ]
        );
    }

    /// Everything at once, in PostgREST's order: the role's isolation level
    /// over the server's, the role's statement timeout before the role, the
    /// app settings after the claims, the time zone last, and then the
    /// pre-request function with each part of its name escaped.
    #[test]
    fn a_write_with_everything_configured() {
        let mut config = AppConfig {
            db_tx_isolation: IsolationLevel::RepeatableRead,
            db_extra_search_path: vec!["ext".to_string()],
            db_pre_request: Some("auth.check".to_string()),
            ..AppConfig::default()
        };
        config.role_settings.insert(
            "editor".to_string(),
            RoleSettings {
                isolation_level: Some(IsolationLevel::Serializable),
                statement_timeout: Some(5000),
            },
        );
        config
            .app_settings
            .insert("jwt_secret".to_string(), "s3cret".to_string());

        let session = Session::new(&config, "editor", claims())
            .schema("api")
            .request("POST", "/items")
            .headers(&IndexMap::new())
            .cookies(&IndexMap::new())
            .timezone(Some("Asia/Kolkata".to_string()));

        assert_eq!(
            session.statements(),
            vec![
                SessionStatement::new("SET TRANSACTION ISOLATION LEVEL SERIALIZABLE", vec![]),
                SessionStatement::new(
                    set_config(11),
                    text(&[
                        "statement_timeout",
                        "5000",
                        "search_path",
                        r#""api", "ext""#,
                        "role",
                        "editor",
                        "request.method",
                        "POST",
                        "request.path",
                        "/items",
                        "request.headers",
                        "{}",
                        "request.cookies",
                        "{}",
                        "request.jwt.claims",
                        r#"{"role":"web_anon"}"#,
                        "request.jwt.claims.role",
                        "web_anon",
                        "app.settings.jwt_secret",
                        "s3cret",
                        "timezone",
                        "Asia/Kolkata",
                    ]),
                ),
                SessionStatement::new(r#"SELECT "auth"."check"()"#, vec![]),
            ]
        );
    }

    /// With no request, the request's settings are left out and the
    /// `search_path` starts at the default schema; the role, the claims and
    /// the pre-request function are all still there.
    #[test]
    fn a_session_with_no_request() {
        let config = AppConfig {
            db_schemas: vec!["api".to_string(), "other".to_string()],
            db_pre_request: Some("check".to_string()),
            ..AppConfig::default()
        };

        let session = Session::new(&config, "alice", vec![]);
        assert_eq!(session.role(), "alice");
        assert_eq!(
            session.statements(),
            vec![
                SessionStatement::new(
                    set_config(2),
                    text(&["search_path", r#""api", "public""#, "role", "alice",]),
                ),
                SessionStatement::new(r#"SELECT "check"()"#, vec![]),
            ]
        );

        // And the pre-request function can be left out.
        assert_eq!(session.pre_request(false).statements().len(), 1);
    }

    /// A caller's own settings go after everything else.
    #[test]
    fn extra_settings_come_last() {
        let config = AppConfig::default();
        let session = Session::new(&config, "alice", vec![])
            .timezone(Some("UTC".to_string()))
            .setting("hasura.role", "alice");
        let statements = session.statements();
        let params = &statements[0].params;
        assert_eq!(
            &params[params.len() - 4..],
            &text(&["timezone", "UTC", "hasura.role", "alice"])[..]
        );
    }

    /// An isolation level asked for wins over the role's and the server's,
    /// and asking for PostgreSQL's default sends nothing, as it does when the
    /// configuration says it.
    #[test]
    fn an_isolation_level_asked_for_wins() {
        let mut config = AppConfig {
            db_tx_isolation: IsolationLevel::RepeatableRead,
            ..AppConfig::default()
        };
        config.role_settings.insert(
            "editor".to_string(),
            RoleSettings {
                isolation_level: Some(IsolationLevel::Serializable),
                statement_timeout: None,
            },
        );
        let first = |session: Session| session.statements()[0].sql.clone();

        assert_eq!(
            first(Session::new(&config, "editor", vec![])),
            "SET TRANSACTION ISOLATION LEVEL SERIALIZABLE"
        );
        assert_eq!(
            first(
                Session::new(&config, "editor", vec![])
                    .isolation(Some(IsolationLevel::RepeatableRead))
            ),
            "SET TRANSACTION ISOLATION LEVEL REPEATABLE READ"
        );
        assert_eq!(
            first(
                Session::new(&config, "editor", vec![])
                    .isolation(Some(IsolationLevel::ReadCommitted))
            ),
            set_config(2)
        );
        assert_eq!(
            first(Session::new(&config, "other", vec![]).isolation(None)),
            "SET TRANSACTION ISOLATION LEVEL REPEATABLE READ"
        );
    }

    /// Beside the other facts about the request, and absent without one.
    #[test]
    fn a_trace_id_is_a_request_setting() {
        let config = AppConfig::default();
        let traced = Session::new(&config, "web_anon", Vec::new())
            .request("GET", "/items")
            .trace_id(Some("4bf92f3577b34da6a3ce929d0e0e4736".to_string()))
            .statements();
        let untraced = Session::new(&config, "web_anon", Vec::new())
            .request("GET", "/items")
            .statements();

        let names = |statements: &[SessionStatement]| -> Vec<String> {
            statements
                .iter()
                .flat_map(|s| s.params.iter())
                .step_by(2)
                .map(|p| match p {
                    SqlParam::Text(name) => name.clone(),
                    other => panic!("{other:?}"),
                })
                .collect()
        };
        assert_eq!(
            names(&traced),
            [
                "search_path",
                "role",
                "request.method",
                "request.path",
                "request.trace_id"
            ]
        );
        assert!(!names(&untraced).contains(&"request.trace_id".to_string()));
    }

    /// Turned off, the `search_path` is left as the role and the database
    /// set it, and nothing else changes.
    #[test]
    fn the_search_path_can_be_left_alone() {
        let config = AppConfig::default();
        let set = Session::new(&config, "web_anon", claims()).statements();
        let left = Session::new(&config, "web_anon", claims())
            .set_search_path(false)
            .statements();
        assert_eq!(
            left,
            vec![SessionStatement::new(
                set_config(3),
                text(&[
                    "role",
                    "web_anon",
                    "request.jwt.claims",
                    r#"{"role":"web_anon"}"#,
                    "request.jwt.claims.role",
                    "web_anon",
                ]),
            )]
        );
        assert_eq!(set[0].params[0], SqlParam::Text("search_path".into()));
    }
}
