//! Host-header matching for the well-known challenge endpoints.
//!
//! Split out from the handler so it can be tested without a database or a
//! running listener, which is most of what there is to get wrong here.

/// Whether a request's `Host` header names `domain`.
///
/// The comparison has to be deliberate rather than `==`, because a `Host`
/// header is not a bare domain:
///
/// - it may carry a port (`example.com:8443`), which is not part of the name;
/// - it is case-insensitive, per RFC 9110 section 4.2.3;
/// - it may be an IPv6 literal in brackets (`[::1]:8443`), where the colons
///   inside the brackets are not a port separator;
/// - it may carry a fully-qualified trailing dot (`example.com.`), which names
///   the same host as `example.com`.
///
/// Anything that does not parse as a host is not a match. Returning `false` on
/// doubt is the safe direction: the caller uses this to decide whether to hand
/// out a challenge secret.
pub fn host_matches(host_header: Option<&str>, domain: &str) -> bool {
    let Some(host) = host_header else {
        return false;
    };
    let Some(name) = strip_port(host.trim()) else {
        return false;
    };
    if name.is_empty() || domain.is_empty() {
        return false;
    }
    normalize(name) == normalize(domain)
}

/// Drop a `:port` suffix, leaving the host name.
///
/// Returns `None` for input that is not a plausible host, including an
/// unterminated IPv6 literal and a port that is not digits.
fn strip_port(host: &str) -> Option<&str> {
    if let Some(rest) = host.strip_prefix('[') {
        // IPv6 literal: the colons before the closing bracket are part of the
        // address, so the port can only follow the bracket.
        let end = rest.find(']')?;
        match rest[end + 1..].strip_prefix(':') {
            // A bracketed address followed by anything that is not a port.
            Some(port) if !is_port(port) => return None,
            None if end + 1 != rest.len() => return None,
            _ => {}
        }
        // Keep the brackets: that is how the authority form names the address,
        // and how a configured domain would have to be written to match.
        return Some(&host[..end + 2]);
    }

    match host.split_once(':') {
        // A second colon outside brackets is not a valid authority.
        Some((_, port)) if !is_port(port) => None,
        Some((name, _)) => Some(name),
        None => Some(host),
    }
}

/// The digits of a port, with the `:` already removed. At least one, all ASCII.
fn is_port(port: &str) -> bool {
    !port.is_empty() && port.bytes().all(|b| b.is_ascii_digit())
}

/// Lowercase, and drop one fully-qualified trailing dot.
fn normalize(name: &str) -> String {
    name.strip_suffix('.').unwrap_or(name).to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_match() {
        assert!(host_matches(Some("example.com"), "example.com"));
    }

    #[test]
    fn mismatch_is_refused() {
        assert!(!host_matches(Some("evil.test"), "example.com"));
    }

    #[test]
    fn missing_header_is_refused() {
        assert!(!host_matches(None, "example.com"));
    }

    #[test]
    fn port_is_not_part_of_the_name() {
        assert!(host_matches(Some("example.com:8443"), "example.com"));
        assert!(host_matches(Some("example.com:80"), "example.com"));
    }

    #[test]
    fn comparison_is_case_insensitive() {
        assert!(host_matches(Some("ExAmPlE.CoM"), "example.com"));
        assert!(host_matches(Some("example.com"), "EXAMPLE.COM"));
    }

    #[test]
    fn trailing_dot_names_the_same_host() {
        assert!(host_matches(Some("example.com."), "example.com"));
        assert!(host_matches(Some("example.com.:443"), "example.com"));
    }

    #[test]
    fn ipv6_literal_keeps_its_brackets() {
        assert!(host_matches(Some("[::1]:8443"), "[::1]"));
        assert!(host_matches(Some("[::1]"), "[::1]"));
    }

    #[test]
    fn unterminated_ipv6_literal_is_refused() {
        assert!(!host_matches(Some("[::1:8443"), "[::1]"));
    }

    #[test]
    fn non_numeric_port_is_refused() {
        assert!(!host_matches(Some("example.com:https"), "example.com"));
    }

    #[test]
    fn a_second_colon_is_refused() {
        assert!(!host_matches(Some("example.com:80:80"), "example.com"));
    }

    #[test]
    fn empty_inputs_are_refused() {
        assert!(!host_matches(Some(""), "example.com"));
        assert!(!host_matches(Some("example.com"), ""));
        assert!(!host_matches(Some(":8443"), "example.com"));
    }

    #[test]
    fn surrounding_whitespace_is_tolerated() {
        assert!(host_matches(Some("  example.com  "), "example.com"));
    }

    #[test]
    fn a_prefix_of_the_domain_is_not_a_match() {
        // The bug this guards against is a `starts_with` implementation.
        assert!(!host_matches(Some("example.com.evil.test"), "example.com"));
        assert!(!host_matches(Some("notexample.com"), "example.com"));
    }
}
