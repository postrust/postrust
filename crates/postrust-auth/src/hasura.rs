//! Hasura's authentication contract.
//!
//! Hasura settles three things about a request before any query runs: whether
//! the caller is authenticated at all, which role it is speaking as, and what
//! session variables that role carries. The first is what makes the other two
//! safe, and reading them apart is the fault this module exists to fix.
//!
//! A caller that holds the admin secret is an administrator, and an
//! administrator may ask to be treated as someone else: `X-Hasura-Role: user`
//! alongside the secret says "answer as `user` would be answered", and the
//! other `x-hasura-*` headers are that role's session variables. Naming a role
//! is therefore something an *authenticated* caller does, not an alternative
//! to authenticating -- a role header arriving on its own is an
//! unauthenticated request, and Hasura refuses it.
//!
//! # Where this deliberately differs from Hasura
//!
//! Hasura configured with no admin secret treats every caller as an
//! administrator, which means an unsecured deployment also lets any caller
//! name its own role and its own identity. This server does not: with no
//! secret configured, `x-hasura-*` headers are ignored entirely and session
//! variables come from a verified token, as they did before this module
//! existed. A policy reading a value the caller chose is not a policy, and the
//! failure is silent -- the query succeeds, against the wrong rows.
//!
//! Nothing is lost by it. Every case in Hasura's corpus that names a role
//! sends the secret beside it, because that is what Hasura's own suite does.

use std::collections::HashMap;

/// The name a caller with the secret speaks as unless it asks for another.
pub const ADMIN_ROLE: &str = "admin";

/// Hasura's own wording, which a migrating client may be matching on.
pub const SECRET_MISSING: &str = "\"x-hasura-admin-secret\" required, but not found";
/// Likewise.
pub const SECRET_INVALID: &str = "invalid x-hasura-admin-secret/x-hasura-access-key";

/// What this server was told about who may claim what.
#[derive(Clone, Debug, Default)]
pub struct HasuraAuthConfig {
    /// The shared secret that authenticates an administrator. Everything in
    /// this module is dark until this is set; see the divergence above.
    pub admin_secret: Option<String>,
    /// The role a request falls to when nothing authenticated it. Hasura's
    /// `HASURA_GRAPHQL_UNAUTHORIZED_ROLE`. Unset means such a request is
    /// refused rather than answered as a stranger.
    pub unauthorized_role: Option<String>,
}

impl HasuraAuthConfig {
    /// Whether this server gates on a secret at all.
    pub fn is_configured(&self) -> bool {
        self.admin_secret.is_some()
    }
}

/// Who the caller is, in Hasura's sense of the word.
///
/// This is not a database role and must not be used as one. The corpus names
/// roles like `Artist` and `anonymous` that exist in no catalogue, and Hasura
/// connects as one database user whatever role a request claims. What the role
/// decides is which permissions apply and what `x-hasura-role` reads as inside
/// a function; which database user the transaction runs as is settled
/// elsewhere and separately.
#[derive(Clone, Debug)]
pub struct HasuraIdentity {
    /// The role the caller speaks as.
    pub role: String,
    /// Session variables, stripped of the `x-hasura-` prefix, lowercased, and
    /// with dashes turned into underscores: `user_id`, `allowed_ids`. The same
    /// shape a verified token's claims are reduced to, because a policy should
    /// not be able to tell which of the two it is reading.
    pub session: HashMap<String, String>,
    /// Whether the admin secret authenticated this request.
    ///
    /// Distinct from `role == "admin"`, which an administrator gives up the
    /// moment it asks to be treated as someone else. Hasura keeps the
    /// distinction for one purpose -- a mutation marked `backend_only` is
    /// reachable only by a caller that proved it holds the secret, whatever
    /// role it then claims -- and that is what this records.
    pub elevated: bool,
}

/// What the admin secret alone can settle about a request.
///
/// Three of the four outcomes are not answers. A secret that is configured and
/// not offered does not refuse the request, because a token or a webhook may
/// still authenticate it; only a secret that is offered and wrong is a
/// refusal, since a caller that tried to be an administrator and failed is not
/// then asked for a token.
#[derive(Clone, Debug)]
pub enum SecretOutcome {
    /// No secret is configured. This server does not gate on one, and no
    /// header is trusted.
    NotConfigured,
    /// The caller holds the secret.
    Accepted(HasuraIdentity),
    /// A secret is configured and none was offered. Something else may
    /// authenticate the request.
    Absent,
    /// A secret is configured and the caller offered the wrong one.
    Rejected,
}

/// Header names that carry the prefix and are not session variables.
///
/// The secret is a credential and would otherwise be readable by any policy
/// and any function that takes the session document. The role is the identity
/// itself rather than one of its attributes, and is reported separately. The
/// backend-only flag is a request about which fields exist, not a fact about
/// the caller.
fn reserved(name: &str) -> bool {
    matches!(
        name,
        "admin-secret" | "access-key" | "role" | "use-backend-only-permissions"
    )
}

/// Reduce `x-hasura-*` headers to session variables.
///
/// Case is not significant in a header name and Hasura treats session variable
/// names the same way, so everything is lowercased on the way in. A repeated
/// header keeps its first value, which is what a single-valued setting can
/// mean.
pub fn session_from_headers(headers: &[(&str, &str)]) -> HashMap<String, String> {
    let mut session = HashMap::new();
    for (name, value) in headers {
        let lowered = name.to_ascii_lowercase();
        let Some(bare) = lowered.strip_prefix("x-hasura-") else {
            continue;
        };
        if bare.is_empty() || reserved(bare) {
            continue;
        }
        session
            .entry(bare.replace('-', "_"))
            .or_insert_with(|| value.to_string());
    }
    session
}

/// The value of one header, by a name compared without regard to case.
fn header<'a>(headers: &[(&'a str, &'a str)], wanted: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case(wanted))
        .map(|(_, value)| *value)
}

/// Settle what the admin secret says about a request.
///
/// `headers` is a slice of name/value pairs rather than any particular header
/// map, which is what lets this crate settle the question without depending on
/// an HTTP library. It is walked more than once.
pub fn from_admin_secret(config: &HasuraAuthConfig, headers: &[(&str, &str)]) -> SecretOutcome {
    let Some(expected) = config.admin_secret.as_deref() else {
        return SecretOutcome::NotConfigured;
    };

    // Both spellings: `x-hasura-access-key` is what the secret was called
    // before v1.0.0-beta.6, and a client old enough to send it is exactly the
    // one that cannot be changed.
    let offered =
        header(headers, "x-hasura-admin-secret").or_else(|| header(headers, "x-hasura-access-key"));

    let Some(offered) = offered else {
        return SecretOutcome::Absent;
    };
    if !constant_time_eq(offered.as_bytes(), expected.as_bytes()) {
        return SecretOutcome::Rejected;
    }

    // Authenticated. From here the caller decides who to be.
    let role = header(headers, "x-hasura-role")
        .filter(|role| !role.is_empty())
        .unwrap_or(ADMIN_ROLE)
        .to_string();

    SecretOutcome::Accepted(HasuraIdentity {
        role,
        session: session_from_headers(headers),
        elevated: true,
    })
}

/// The identity a request falls to when nothing authenticated it.
///
/// `None` means the request is refused, which is what an unset
/// `unauthorized_role` asks for.
pub fn unauthenticated(config: &HasuraAuthConfig) -> Option<HasuraIdentity> {
    config
        .unauthorized_role
        .as_ref()
        .map(|role| HasuraIdentity {
            role: role.clone(),
            session: HashMap::new(),
            elevated: false,
        })
}

/// Whether a caller asked for the fields a `backend_only` permission hides.
///
/// Hasura reads this only from a request the admin secret authenticated, which
/// is the whole point of the flag: a mutation marked backend-only is one a
/// client must not be able to reach by naming a role.
pub fn backend_only_requested(headers: &[(&str, &str)], elevated: bool) -> bool {
    elevated
        && header(headers, "x-hasura-use-backend-only-permissions")
            .and_then(boolean_text)
            .unwrap_or(false)
}

/// What Hasura reads as true and false in a header, and the words it uses when
/// a header is neither.
///
/// Not Rust's `bool` parser and not a truthiness test: the accepted texts are
/// a closed set Hasura documents in the refusal itself, and `1` is not among
/// them. A value outside it is a client mistake, and answering it as `false`
/// would send a backend-only write down the path meant for everyone else --
/// silently, on a header the client thought it had set.
pub fn boolean_text(value: &str) -> Option<bool> {
    match value.trim().to_ascii_lowercase().as_str() {
        "true" | "t" | "yes" | "y" => Some(true),
        "false" | "f" | "no" | "n" => Some(false),
        _ => None,
    }
}

/// The header whose value is read as a boolean, and Hasura's refusal for one
/// that is not.
///
/// The two spaces after the colon and before `False` are Hasura's own, and a
/// client showing this to a person is showing text it already ships.
pub const BACKEND_ONLY_HEADER: &str = "x-hasura-use-backend-only-permissions";

/// Whether a request carries that header with something unreadable in it.
pub fn unreadable_boolean_header<'a>(headers: &[(&'a str, &'a str)]) -> Option<String> {
    let given = header(headers, BACKEND_ONLY_HEADER)?;
    if boolean_text(given).is_some() {
        return None;
    }
    Some(format!(
        "\"{}\":  Not a valid boolean text. True values are \
         [\"true\",\"t\",\"yes\",\"y\"] and  False values are \
         [\"false\",\"f\",\"no\",\"n\"]. All values are case insensitive",
        BACKEND_ONLY_HEADER
    ))
}

/// Hasura's wording for a caller that asked to be a role its token does not
/// list.
pub const ROLE_NOT_ALLOWED: &str = "Your requested role is not in allowed roles";

/// Where Hasura puts its claims in a token unless it is told otherwise.
const NAMESPACE: &str = "https://hasura.io/jwt/claims";

/// One claim, from under the namespace if it is there and from the top level
/// otherwise. Hasura mints the namespaced spelling and accepts both, so both
/// are read here; a name is compared without regard to case, as a header is.
fn claim<'a>(
    claims: &'a HashMap<String, serde_json::Value>,
    wanted: &str,
) -> Option<&'a serde_json::Value> {
    if let Some(serde_json::Value::Object(namespaced)) = claims.get(NAMESPACE) {
        let found = namespaced
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(wanted))
            .map(|(_, value)| value);
        if found.is_some() {
            return found;
        }
    }
    claims
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case(wanted))
        .map(|(_, value)| value)
}

/// The role a verified token names, if it names one.
///
/// `x-hasura-role` where the token fixes the role outright, and
/// `x-hasura-default-role` where it offers a default beside the
/// `x-hasura-allowed-roles` it may choose among -- which is the shape Hasura's
/// own documentation mints.
///
/// `None` leaves the database role standing in, which is what happened before
/// the two were told apart.
pub fn role_from_claims(claims: &HashMap<String, serde_json::Value>) -> Option<String> {
    ["x-hasura-role", "x-hasura-default-role"]
        .into_iter()
        .find_map(|wanted| {
            claim(claims, wanted)
                .and_then(serde_json::Value::as_str)
                .filter(|role| !role.is_empty())
                .map(str::to_string)
        })
}

/// The roles a token allows its bearer to ask to be.
///
/// Empty where the claim is absent or is not a list of names, which is the
/// reading that grants nothing: the list is what widens an identity, so a
/// value that cannot be read as one must not widen it.
pub fn allowed_roles(claims: &HashMap<String, serde_json::Value>) -> Vec<String> {
    match claim(claims, "x-hasura-allowed-roles") {
        Some(serde_json::Value::Array(items)) => items
            .iter()
            .filter_map(serde_json::Value::as_str)
            .map(str::to_string)
            .collect(),
        _ => Vec::new(),
    }
}

/// Who a verified token's bearer speaks as.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TokenRole {
    /// Who the caller is. `None` leaves the database role standing in.
    Is(Option<String>),
    /// The caller asked to be a role its token does not list.
    NotAllowed,
}

/// Settle which role a verified token speaks as, given what the caller asked
/// for.
///
/// A token that allows more than one identity carries both
/// `x-hasura-default-role` -- who the caller is when it asks for nothing --
/// and `x-hasura-allowed-roles`, the list it may ask for instead. The asking
/// is done with an `X-Hasura-Role` header, and the list is what makes reading
/// that header safe: it sits inside the signature, so a caller can choose
/// among the identities it was issued and cannot add one.
///
/// This is the one place a header names a role without the admin secret beside
/// it, and it is safe for that reason alone. A token with no list allows only
/// the role it already names -- an absent list is not permission to be anyone.
pub fn role_for_token(
    claims: &HashMap<String, serde_json::Value>,
    headers: &[(&str, &str)],
) -> TokenRole {
    let named = role_from_claims(claims);

    let Some(asked) = header(headers, "x-hasura-role").filter(|role| !role.is_empty()) else {
        return TokenRole::Is(named);
    };

    // Asking to be who you already are needs no list: a token that fixes one
    // role and a client that names that role agree, and refusing them would
    // break a client that sets the header on every request.
    if named.as_deref() == Some(asked) || allowed_roles(claims).iter().any(|role| role == asked) {
        return TokenRole::Is(Some(asked.to_string()));
    }

    TokenRole::NotAllowed
}

/// Collect `x-hasura-*` claims from a verified token as session variables.
///
/// Hasura puts them under the namespace by default and allows them at the top
/// level; both spellings are read, and the prefix is dropped so a policy names
/// `hasura.user_id` rather than `hasura.x-hasura-user-id`. `x-hasura-role` is
/// left out: the role is what the token was authenticated as, and re-reading
/// it here would let a claim override that decision.
pub fn session_from_claims(claims: &HashMap<String, serde_json::Value>) -> HashMap<String, String> {
    let mut session = HashMap::new();

    let mut take = |key: &str, value: &serde_json::Value| {
        let lowered = key.to_ascii_lowercase();
        let Some(name) = lowered.strip_prefix("x-hasura-") else {
            return;
        };
        if name == "role" || name.is_empty() {
            return;
        }
        // A claim may be a string, a number or a list of ids; the setting is
        // text either way, and a string keeps its own spelling rather than
        // gaining quotes.
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

/// Compare two secrets without letting the time taken say how much of the
/// first one was right.
fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    left.iter()
        .zip(right)
        .fold(0u8, |acc, (a, b)| acc | (a ^ b))
        == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn configured() -> HasuraAuthConfig {
        HasuraAuthConfig {
            admin_secret: Some("shh".into()),
            unauthorized_role: None,
        }
    }

    #[test]
    fn nothing_is_trusted_without_a_secret() {
        let headers = [("x-hasura-role", "user"), ("x-hasura-user-id", "1")];
        let outcome = from_admin_secret(&HasuraAuthConfig::default(), &headers[..]);
        assert!(matches!(outcome, SecretOutcome::NotConfigured));
    }

    #[test]
    fn the_secret_alone_is_an_administrator() {
        let headers = [("X-Hasura-Admin-Secret", "shh")];
        let SecretOutcome::Accepted(identity) = from_admin_secret(&configured(), &headers[..])
        else {
            panic!("the secret should have been accepted");
        };
        assert_eq!(identity.role, "admin");
        assert!(identity.elevated);
        assert!(identity.session.is_empty());
    }

    #[test]
    fn an_administrator_may_ask_to_be_someone_else() {
        let headers = [
            ("X-Hasura-Admin-Secret", "shh"),
            ("X-Hasura-Role", "user"),
            ("X-Hasura-User-Id", "1"),
            ("X-Hasura-Allowed-Ids", "{1,2,3}"),
        ];
        let SecretOutcome::Accepted(identity) = from_admin_secret(&configured(), &headers[..])
        else {
            panic!("the secret should have been accepted");
        };
        assert_eq!(identity.role, "user");
        // Still elevated: the caller proved it holds the secret, and giving up
        // the admin role does not undo that.
        assert!(identity.elevated);
        assert_eq!(
            identity.session.get("user_id").map(String::as_str),
            Some("1")
        );
        assert_eq!(
            identity.session.get("allowed_ids").map(String::as_str),
            Some("{1,2,3}")
        );
        // The credential is not one of the caller's attributes.
        assert!(!identity.session.contains_key("admin_secret"));
        // Nor is the identity itself.
        assert!(!identity.session.contains_key("role"));
    }

    #[test]
    fn a_role_header_on_its_own_is_not_authentication() {
        let headers = [("X-Hasura-Role", "user"), ("X-Hasura-User-Id", "1")];
        assert!(matches!(
            from_admin_secret(&configured(), &headers[..]),
            SecretOutcome::Absent
        ));
    }

    #[test]
    fn a_wrong_secret_is_refused_rather_than_fallen_through() {
        let headers = [("X-Hasura-Admin-Secret", "guess")];
        assert!(matches!(
            from_admin_secret(&configured(), &headers[..]),
            SecretOutcome::Rejected
        ));
    }

    #[test]
    fn the_older_spelling_of_the_secret_is_accepted() {
        let headers = [("X-Hasura-Access-Key", "shh")];
        assert!(matches!(
            from_admin_secret(&configured(), &headers[..]),
            SecretOutcome::Accepted(_)
        ));
    }

    #[test]
    fn an_empty_role_header_is_no_role_at_all() {
        let headers = [("X-Hasura-Admin-Secret", "shh"), ("X-Hasura-Role", "")];
        let SecretOutcome::Accepted(identity) = from_admin_secret(&configured(), &headers[..])
        else {
            panic!("the secret should have been accepted");
        };
        assert_eq!(identity.role, "admin");
    }

    #[test]
    fn session_names_are_lowercased_and_underscored() {
        let session = session_from_headers(&[("X-Hasura-Infant-Name", "Bittu")][..]);
        assert_eq!(
            session.get("infant_name").map(String::as_str),
            Some("Bittu")
        );
    }

    #[test]
    fn headers_without_the_prefix_are_not_session_variables() {
        let session = session_from_headers(&[("Cookie", "refresh_token=x"), ("Accept", "*/*")][..]);
        assert!(session.is_empty());
    }

    #[test]
    fn an_unauthenticated_request_falls_to_the_role_it_was_given() {
        let config = HasuraAuthConfig {
            admin_secret: Some("shh".into()),
            unauthorized_role: Some("anonymous".into()),
        };
        let identity = unauthenticated(&config).expect("a role was configured");
        assert_eq!(identity.role, "anonymous");
        assert!(!identity.elevated);
        assert!(identity.session.is_empty());
    }

    #[test]
    fn without_that_role_an_unauthenticated_request_is_refused() {
        assert!(unauthenticated(&configured()).is_none());
    }

    #[test]
    fn backend_only_is_only_for_a_caller_that_proved_it() {
        let headers = [("X-Hasura-Use-Backend-Only-Permissions", "true")];
        assert!(backend_only_requested(&headers[..], true));
        // The same header from a caller that never held the secret says
        // nothing, which is what makes the flag worth having.
        assert!(!backend_only_requested(&headers[..], false));
    }

    /// The accepted texts are a closed set, and `1` is not in it.
    #[test]
    fn a_header_that_is_not_a_boolean_text_is_neither_true_nor_false() {
        for yes in ["true", "T", "yes", " Y "] {
            assert_eq!(boolean_text(yes), Some(true), "{}", yes);
        }
        for no in ["false", "F", "no", "N"] {
            assert_eq!(boolean_text(no), Some(false), "{}", no);
        }
        for neither in ["1", "0", "random", ""] {
            assert_eq!(boolean_text(neither), None, "{}", neither);
        }

        let headers = [(BACKEND_ONLY_HEADER, "random")];
        let refusal = unreadable_boolean_header(&headers[..]).expect("it is refused");
        assert!(refusal.starts_with("\"x-hasura-use-backend-only-permissions\":  Not a valid"));
        // And a request that does not carry it at all is not a bad request.
        assert_eq!(unreadable_boolean_header(&[][..]), None);
        assert_eq!(
            unreadable_boolean_header(&[(BACKEND_ONLY_HEADER, "yes")][..]),
            None
        );
    }

    fn claims(value: serde_json::Value) -> HashMap<String, serde_json::Value> {
        match value {
            serde_json::Value::Object(map) => map.into_iter().collect(),
            other => panic!("claims must be an object, got {other}"),
        }
    }

    /// The default is who the caller is when it asks for nothing.
    #[test]
    fn a_token_that_asks_for_nothing_speaks_as_its_default() {
        let token = claims(serde_json::json!({
            "https://hasura.io/jwt/claims": {
                "x-hasura-default-role": "user",
                "x-hasura-allowed-roles": ["user", "editor"],
            }
        }));
        assert_eq!(
            role_for_token(&token, &[][..]),
            TokenRole::Is(Some("user".into()))
        );
    }

    /// The list inside the signature is what makes the header safe to read.
    #[test]
    fn a_role_the_token_lists_may_be_asked_for() {
        let token = claims(serde_json::json!({
            "https://hasura.io/jwt/claims": {
                "x-hasura-default-role": "user",
                "x-hasura-allowed-roles": ["user", "editor"],
            }
        }));
        let headers = [("X-Hasura-Role", "editor")];
        assert_eq!(
            role_for_token(&token, &headers[..]),
            TokenRole::Is(Some("editor".into()))
        );
    }

    /// A caller cannot widen what it was issued.
    #[test]
    fn a_role_outside_the_list_is_refused() {
        let token = claims(serde_json::json!({
            "https://hasura.io/jwt/claims": {
                "x-hasura-default-role": "user",
                "x-hasura-allowed-roles": ["user", "editor"],
            }
        }));
        for wanted in ["admin", "Editor", "anonymous"] {
            let headers = [("x-hasura-role", wanted)];
            assert_eq!(
                role_for_token(&token, &headers[..]),
                TokenRole::NotAllowed,
                "{wanted}"
            );
        }
    }

    /// An absent list is not permission to be anyone.
    #[test]
    fn a_token_without_a_list_allows_only_the_role_it_names() {
        let token = claims(serde_json::json!({ "x-hasura-role": "user" }));

        let same = [("x-hasura-role", "user")];
        assert_eq!(
            role_for_token(&token, &same[..]),
            TokenRole::Is(Some("user".into()))
        );

        let other = [("x-hasura-role", "admin")];
        assert_eq!(role_for_token(&token, &other[..]), TokenRole::NotAllowed);
    }

    /// A list that is not a list of names grants nothing.
    #[test]
    fn a_list_that_cannot_be_read_widens_no_identity() {
        for unreadable in [
            serde_json::json!("editor"),
            serde_json::json!({"0": "editor"}),
            serde_json::json!(null),
        ] {
            let token = claims(serde_json::json!({
                "x-hasura-default-role": "user",
                "x-hasura-allowed-roles": unreadable,
            }));
            let headers = [("x-hasura-role", "editor")];
            assert_eq!(role_for_token(&token, &headers[..]), TokenRole::NotAllowed);
        }
    }

    /// Both spellings are read, and the namespace wins where both are present.
    #[test]
    fn a_claim_is_read_from_the_namespace_or_the_top_level() {
        let top = claims(serde_json::json!({
            "x-hasura-default-role": "user",
            "x-hasura-allowed-roles": ["user", "editor"],
        }));
        let headers = [("x-hasura-role", "editor")];
        assert_eq!(
            role_for_token(&top, &headers[..]),
            TokenRole::Is(Some("editor".into()))
        );

        let both = claims(serde_json::json!({
            "https://hasura.io/jwt/claims": { "x-hasura-default-role": "namespaced" },
            "x-hasura-default-role": "top",
        }));
        assert_eq!(role_from_claims(&both), Some("namespaced".into()));
    }

    /// A token naming no role at all leaves the database role standing in.
    #[test]
    fn a_token_that_names_no_role_names_none() {
        let token = claims(serde_json::json!({ "sub": "1" }));
        assert_eq!(role_for_token(&token, &[][..]), TokenRole::Is(None));
    }

    /// The role is reported separately; a claim repeating it must not become a
    /// session variable a policy could read instead.
    #[test]
    fn the_role_claim_is_not_a_session_variable() {
        let token = claims(serde_json::json!({
            "https://hasura.io/jwt/claims": {
                "x-hasura-role": "user",
                "x-hasura-user-id": 7,
                "x-hasura-org-ids": ["1", "2"],
            }
        }));
        let session = session_from_claims(&token);
        assert!(!session.contains_key("role"));
        assert_eq!(session.get("user_id").map(String::as_str), Some("7"));
        assert_eq!(
            session.get("org_ids").map(String::as_str),
            Some("[\"1\",\"2\"]")
        );
    }

    #[test]
    fn secrets_of_different_lengths_do_not_match() {
        assert!(!constant_time_eq(b"shh", b"shhh"));
        assert!(constant_time_eq(b"shh", b"shh"));
    }
}
