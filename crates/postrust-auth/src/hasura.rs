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
            .is_some_and(|value| matches!(value.trim().to_ascii_lowercase().as_str(), "true" | "1"))
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

    #[test]
    fn secrets_of_different_lengths_do_not_match() {
        assert!(!constant_time_eq(b"shh", b"shhh"));
        assert!(constant_time_eq(b"shh", b"shh"));
    }
}
