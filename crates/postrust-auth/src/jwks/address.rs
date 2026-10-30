//! A JWKS URL, read strictly enough that it cannot mean two things.
//!
//! Only as much of URL syntax as a JWKS URL needs: `http` or `https`, a host
//! (a name, an IPv4 address, or an IPv6 address in brackets), an optional
//! port, and a path with an optional query. Everything else is refused rather
//! than interpreted -- user info (`https://idp.example@evil.example/`),
//! percent-encoding or non-ASCII in the host, a backslash anywhere, spaces and
//! control characters.
//!
//! Strict on purpose. The decision made here -- is this host loopback, is a
//! redirect's host the same one -- is only as good as its agreement with the
//! HTTP client that then connects, and the clients (reqwest, a Worker's Fetch
//! API) follow the WHATWG URL Standard, which is lenient in ways that have
//! been exploited: `\` is a `/`, `127.1` is `127.0.0.1`, an `@` moves the
//! host. A URL that a lenient reader and this one could disagree about is not
//! one this reader accepts. What is left is what identity providers actually
//! publish.
//!
//! Not the `url` crate, which would agree with the clients by construction:
//! its internationalised-domain tables add some 400 KB to the Worker, whose
//! bundle is what every cold start downloads. Refusing what is not ASCII
//! costs nothing a JWKS URL needs.

use std::fmt;
use std::net::{Ipv4Addr, Ipv6Addr};

/// A URL this module accepted.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Address {
    https: bool,
    /// Lowercased; an IPv6 address keeps its brackets.
    host: String,
    port: Option<u16>,
    /// The path and query, from the `/` on. The fragment is dropped: it is
    /// never sent.
    rest: String,
}

impl Address {
    /// Read an absolute `http` or `https` URL.
    pub(super) fn parse(text: &str) -> Result<Self, String> {
        if text
            .chars()
            .any(|c| c.is_control() || c.is_whitespace() || c == '\\')
        {
            return Err("spaces, control characters and backslashes are not accepted".into());
        }
        let (https, after) = if let Some(after) = strip_prefix_ignore_case(text, "https://") {
            (true, after)
        } else if let Some(after) = strip_prefix_ignore_case(text, "http://") {
            (false, after)
        } else {
            return Err(match text.split_once(':') {
                Some((scheme, _)) if !scheme.is_empty() && !scheme.contains('/') => {
                    format!("the scheme must be https, not {scheme}")
                }
                _ => "not an absolute http or https URL".into(),
            });
        };

        let end = after.find(['/', '?', '#']).unwrap_or(after.len());
        let (authority, rest) = after.split_at(end);
        if !authority
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | ':' | '[' | ']'))
        {
            return Err(format!(
                "the host {authority:?} is not accepted: user info, percent-encoding and \
                 non-ASCII names are refused"
            ));
        }
        let (host, port) = split_host_port(authority)?;

        let rest = rest.split('#').next().unwrap_or_default();
        let rest = if rest.is_empty() || rest.starts_with('?') {
            format!("/{rest}")
        } else {
            rest.to_string()
        };
        Ok(Self {
            https,
            host,
            port,
            rest,
        })
    }

    /// The URL a `Location` header names, read against this one.
    pub(super) fn join(&self, location: &str) -> Result<Self, String> {
        let absolute = location
            .split_once(':')
            .is_some_and(|(scheme, _)| is_scheme(scheme));
        if absolute {
            return Self::parse(location);
        }
        let scheme = if self.https { "https:" } else { "http:" };
        if location.starts_with("//") {
            return Self::parse(&format!("{scheme}{location}"));
        }
        let origin = format!("{scheme}//{}", self.authority());
        let path = if location.starts_with('/') {
            location.to_string()
        } else if location.starts_with('?') {
            format!("{}{location}", self.path())
        } else {
            // Relative to the directory the current path is in. Dot segments
            // are the client's to resolve; they cannot change the host.
            let path = self.path();
            format!("{}{location}", &path[..=path.rfind('/').unwrap_or(0)])
        };
        Self::parse(&format!("{origin}{path}"))
    }

    pub(super) fn is_https(&self) -> bool {
        self.https
    }

    pub(super) fn host(&self) -> &str {
        &self.host
    }

    /// `localhost`, `127.0.0.0/8` or `::1`, spelt exactly so: not
    /// `localhost.`, not `127.1`, not a name that resolves to one of them.
    pub(super) fn is_loopback(&self) -> bool {
        if self.host == "localhost" {
            return true;
        }
        if let Some(inner) = self.host.strip_prefix('[') {
            return inner
                .strip_suffix(']')
                .and_then(|ip| ip.parse::<Ipv6Addr>().ok())
                .is_some_and(|ip| ip.is_loopback());
        }
        self.host
            .parse::<Ipv4Addr>()
            .is_ok_and(|ip| ip.is_loopback())
    }

    fn authority(&self) -> String {
        match self.port {
            Some(port) => format!("{}:{port}", self.host),
            None => self.host.clone(),
        }
    }

    fn path(&self) -> &str {
        self.rest.split('?').next().unwrap_or("/")
    }
}

impl fmt::Display for Address {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let scheme = if self.https { "https" } else { "http" };
        write!(f, "{scheme}://{}{}", self.authority(), self.rest)
    }
}

fn strip_prefix_ignore_case<'a>(text: &'a str, prefix: &str) -> Option<&'a str> {
    let head = text.get(..prefix.len())?;
    head.eq_ignore_ascii_case(prefix)
        .then(|| &text[prefix.len()..])
}

/// RFC 3986's `scheme`: a letter, then letters, digits, `+`, `-` and `.`.
fn is_scheme(text: &str) -> bool {
    let mut chars = text.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphabetic())
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
}

fn split_host_port(authority: &str) -> Result<(String, Option<u16>), String> {
    let (host, port) = if authority.starts_with('[') {
        let close = authority
            .find(']')
            .ok_or_else(|| format!("{authority:?}: an IPv6 address without its ']'"))?;
        let (host, after) = authority.split_at(close + 1);
        if host[1..host.len() - 1].parse::<Ipv6Addr>().is_err() {
            return Err(format!("{host} is not an IPv6 address"));
        }
        match after {
            "" => (host, None),
            _ => match after.strip_prefix(':') {
                Some(port) => (host, Some(port)),
                None => return Err(format!("{authority:?} is not a host and port")),
            },
        }
    } else {
        if authority.contains(['[', ']']) {
            return Err(format!("{authority:?} is not a host"));
        }
        match authority.split_once(':') {
            Some((host, port)) => (host, Some(port)),
            None => (authority, None),
        }
    };
    if host.is_empty() || host.starts_with(['.', '-']) || host.contains("..") {
        return Err(format!("{authority:?} names no host"));
    }
    let port = match port {
        None => None,
        Some(port) if !port.is_empty() && port.chars().all(|c| c.is_ascii_digit()) => Some(
            port.parse::<u16>()
                .map_err(|_| format!("{port} is not a port"))?,
        ),
        Some(port) => return Err(format!("{port:?} is not a port")),
    };
    Ok((host.to_ascii_lowercase(), port))
}

#[cfg(test)]
mod tests {
    use super::Address;

    fn parsed(text: &str) -> String {
        Address::parse(text).unwrap().to_string()
    }

    #[test]
    fn what_identity_providers_publish_is_read() {
        for (text, read) in [
            (
                "https://abc.supabase.co/auth/v1/.well-known/jwks.json",
                "https://abc.supabase.co/auth/v1/.well-known/jwks.json",
            ),
            (
                "HTTPS://Tenant.EU.Auth0.com/.well-known/jwks.json#x",
                "https://tenant.eu.auth0.com/.well-known/jwks.json",
            ),
            (
                "https://cognito-idp.us-east-1.amazonaws.com/us-east-1_abc/.well-known/jwks.json",
                "https://cognito-idp.us-east-1.amazonaws.com/us-east-1_abc/.well-known/jwks.json",
            ),
            (
                "https://kc.example:8443/realms/a/protocol/openid-connect/certs?x=1",
                "https://kc.example:8443/realms/a/protocol/openid-connect/certs?x=1",
            ),
            ("http://[::1]:9999", "http://[::1]:9999/"),
            ("https://idp.example?v=2", "https://idp.example/?v=2"),
        ] {
            assert_eq!(parsed(text), read, "{text}");
        }
    }

    #[test]
    fn anything_a_lenient_reader_could_read_otherwise_is_refused() {
        for text in [
            "https://idp.example@evil.example/jwks",
            "https://idp.example\\@evil.example/jwks",
            "https://evil.example\\.idp.example/",
            "https://%65vil.example/",
            "https://bücher.example/",
            "https://idp.example:/jwks",
            "https://idp.example:99999/jwks",
            "https://idp.example:443x/jwks",
            "https://[::1/jwks",
            "https://[not-v6]/jwks",
            "https:///jwks",
            "https://.idp.example/",
            "https://idp..example/",
            "https://idp.example/ jwks",
            "https:/idp.example/jwks",
            "idp.example/jwks",
        ] {
            assert!(Address::parse(text).is_err(), "{text}");
        }
    }

    #[test]
    fn loopback_is_spelt_exactly() {
        for (text, loopback) in [
            ("http://localhost/", true),
            ("http://LOCALHOST:1/", true),
            ("http://127.0.0.1/", true),
            ("http://127.200.0.9/", true),
            ("http://[::1]/", true),
            ("http://localhost./", false),
            ("http://127.1/", false),
            ("http://0x7f.0.0.1/", false),
            ("http://[::ffff:127.0.0.1]/", false),
            ("http://localhost.evil.example/", false),
            ("http://10.0.0.1/", false),
        ] {
            assert_eq!(
                Address::parse(text).unwrap().is_loopback(),
                loopback,
                "{text}"
            );
        }
    }

    #[test]
    fn a_location_is_read_against_the_url_it_answered() {
        let base = Address::parse("https://idp.example:8443/a/b/jwks?x=1").unwrap();
        for (location, joined) in [
            ("/keys", "https://idp.example:8443/keys"),
            ("keys", "https://idp.example:8443/a/b/keys"),
            ("?y=2", "https://idp.example:8443/a/b/jwks?y=2"),
            ("//other.example/keys", "https://other.example/keys"),
            ("http://idp.example/keys", "http://idp.example/keys"),
        ] {
            assert_eq!(
                base.join(location).unwrap().to_string(),
                joined,
                "{location}"
            );
        }
        assert!(base.join("ftp://idp.example/keys").is_err());
        assert!(base.join("/\\evil.example/").is_err());
    }
}
