//! Checks on what tenants send, before any of it is stored.
//!
//! Everything a tenant names -- a domain, a backend address, a header -- ends
//! up somewhere it is trusted: a unique index, a certificate, a connection the
//! proxy opens on the tenant's behalf. The rules live here, in one place, so
//! that every path a value can enter by applies the same ones. Two spellings of
//! one rule is how `VICTIM.test` came to be registrable alongside
//! `victim.test`.

use crate::error::{ProxyError, ProxyResult};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

/// The canonical form of a domain name, or why it cannot be one.
///
/// Lowercased, with one trailing dot removed, and then held to the LDH rule
/// (letters, digits, hyphen) that every name a certificate authority will issue
/// for, and every name a `Host` header can carry unencoded, obeys:
///
/// - ASCII only. An internationalised name has to arrive in its punycode
///   (`xn--`) form. Accepting Unicode would mean picking one of several
///   incompatible normalisations, and two tenants could then register names
///   that render alike and compare different.
/// - at least two labels. A single label resolves through the resolver's search
///   list, which makes it a name for something inside the network, not on it.
/// - each label 1 to 63 characters, not starting or ending with a hyphen; the
///   whole at most 253.
/// - not an IP address, in any spelling a resolver will accept. `127.1` and
///   `0x7f.1` are IPv4 to `inet_aton`, so a name whose last label is numeric
///   (or `0x` hex) is refused: no real top-level domain is.
///
/// Every place a domain enters the system goes through this, so the stored
/// form is the compared form.
pub fn normalize_domain(input: &str) -> ProxyResult<String> {
    if input.is_empty() {
        return Err(invalid("the domain is empty"));
    }
    if !input.is_ascii() {
        return Err(invalid(
            "the domain contains non-ASCII characters; send an internationalised \
             name in its punycode (xn--) form",
        ));
    }
    if input.parse::<IpAddr>().is_ok() || input.contains(':') || input.starts_with('[') {
        return Err(invalid("an IP address is not a domain"));
    }

    let name = input
        .strip_suffix('.')
        .unwrap_or(input)
        .to_ascii_lowercase();
    if name.len() > 253 {
        return Err(invalid("the domain is longer than 253 characters"));
    }

    let labels: Vec<&str> = name.split('.').collect();
    if labels.len() < 2 {
        return Err(invalid(
            "the domain must have at least two labels (a single label is not a public name)",
        ));
    }
    for label in &labels {
        check_label(label)?;
    }

    let last = labels[labels.len() - 1];
    if is_numeric_label(last) {
        return Err(invalid(
            "the domain's last label is numeric, which resolvers read as an IP address",
        ));
    }

    Ok(name)
}

fn check_label(label: &str) -> ProxyResult<()> {
    if label.is_empty() || label.len() > 63 {
        return Err(invalid("each label must be 1 to 63 characters"));
    }
    if label.starts_with('-') || label.ends_with('-') {
        return Err(invalid("a label must not start or end with a hyphen"));
    }
    if !label
        .bytes()
        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
    {
        return Err(invalid(
            "a label may contain only letters, digits and hyphens",
        ));
    }
    Ok(())
}

/// Digits, or `0x` followed by hex digits: what `inet_aton` reads as a number.
fn is_numeric_label(label: &str) -> bool {
    if label.bytes().all(|b| b.is_ascii_digit()) {
        return true;
    }
    match label.strip_prefix("0x") {
        Some(hex) => hex.bytes().all(|b| b.is_ascii_hexdigit()),
        None => false,
    }
}

fn invalid(reason: &str) -> ProxyError {
    ProxyError::Validation(format!("invalid domain: {reason}"))
}

/// Names that are never a tenant's to claim.
///
/// The platform's own hostnames belong here -- the API's, the dashboard's, the
/// one the proxy's TLS listener answers for itself. Registration of these is
/// refused before verification is even attempted: an unverified row for the
/// platform's name is harmless only for as long as every later check remembers
/// to look at `verification_status`, and that is one forgotten check away from
/// a tenant's certificate being served for it.
///
/// [`Default`] reserves the special-use names that never denote a public host
/// (RFC 6761, 6762, 7686, 8375, 9476): `localhost`, `local`, `internal`,
/// `arpa`, `invalid`, `onion`, `alt`, and `localdomain`. Add the platform's own
/// with [`with_exact`](Self::with_exact) and [`with_suffix`](Self::with_suffix).
#[derive(Clone, Debug)]
pub struct ReservedDomains {
    exact: Vec<String>,
    suffixes: Vec<String>,
}

/// Special-use names. Suffixes, so `foo.localhost` is refused with `localhost`.
const SPECIAL_USE: &[&str] = &[
    "localhost",
    "local",
    "localdomain",
    "internal",
    "arpa",
    "invalid",
    "onion",
    "alt",
];

impl Default for ReservedDomains {
    fn default() -> Self {
        Self {
            exact: Vec::new(),
            suffixes: SPECIAL_USE.iter().map(|s| s.to_string()).collect(),
        }
    }
}

impl ReservedDomains {
    /// Nothing reserved at all, not even the special-use names.
    ///
    /// For tests and for embedders that do their own checking. The special-use
    /// names cannot be verified anyway, so leaving them out costs little, but
    /// there is no reason to.
    pub fn none() -> Self {
        Self {
            exact: Vec::new(),
            suffixes: Vec::new(),
        }
    }

    /// Reserve exactly this name, and not its subdomains.
    ///
    /// Panics if `name` is not a valid domain: a reservation that silently
    /// matches nothing is worse than a startup failure.
    pub fn with_exact(mut self, name: &str) -> Self {
        self.exact.push(normalize_reservation(name));
        self
    }

    /// Reserve this name and every name under it.
    ///
    /// `with_suffix("example.com")` reserves `example.com` and
    /// `api.example.com`, not `notexample.com`. Panics on an invalid name, as
    /// [`with_exact`](Self::with_exact) does.
    pub fn with_suffix(mut self, suffix: &str) -> Self {
        self.suffixes.push(normalize_reservation(suffix));
        self
    }

    /// Whether a name, already in [`normalize_domain`] form, is reserved.
    pub fn is_reserved(&self, normalized: &str) -> bool {
        self.exact.iter().any(|e| e == normalized)
            || self.suffixes.iter().any(|s| is_under(normalized, s))
    }
}

/// Lowercase and strip a trailing dot, for a reservation.
///
/// Not [`normalize_domain`]: single-label suffixes (`internal`) are exactly
/// what that refuses, and are legitimate here.
fn normalize_reservation(name: &str) -> String {
    let name = name.strip_suffix('.').unwrap_or(name).to_ascii_lowercase();
    assert!(
        !name.is_empty()
            && name.is_ascii()
            && name.split('.').all(|label| check_label(label).is_ok()),
        "reserved domain {name:?} is not a valid domain name"
    );
    name
}

/// `name` is `suffix` or a subdomain of it, on a label boundary.
fn is_under(name: &str, suffix: &str) -> bool {
    name == suffix
        || name
            .strip_suffix(suffix)
            .is_some_and(|rest| rest.ends_with('.'))
}

/// Whether the proxy must refuse to connect to this address on a tenant's
/// behalf.
///
/// True for everything that is not ordinary public unicast: loopback, private
/// (RFC 1918, and ULA for IPv6), link-local -- which includes
/// `169.254.169.254`, every cloud's metadata service -- CGNAT, unspecified,
/// multicast, broadcast, the documentation and benchmarking ranges, and the
/// IETF-reserved blocks. IPv6 forms that carry an IPv4 address (mapped,
/// compatible, NAT64, 6to4) are judged by the address they carry, since that is
/// where the packet ends up.
///
/// Shared by domain verification, which connects, and backend validation,
/// which stores an address something will connect to later.
pub fn is_forbidden_address(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => is_forbidden_v4(v4),
        IpAddr::V6(v6) => is_forbidden_v6(v6),
    }
}

fn is_forbidden_v4(ip: Ipv4Addr) -> bool {
    let [a, b, c, _] = ip.octets();
    a == 0 // "this network", including 0.0.0.0
        || a == 10
        || a == 127
        || (a == 100 && (64..128).contains(&b)) // CGNAT, 100.64.0.0/10
        || (a == 169 && b == 254) // link-local, and the metadata service
        || (a == 172 && (16..32).contains(&b))
        || (a == 192 && b == 168)
        || (a == 192 && b == 0 && c == 0) // IETF protocol assignments
        || (a == 192 && b == 0 && c == 2) // TEST-NET-1
        || (a == 198 && (18..20).contains(&b)) // benchmarking, 198.18.0.0/15
        || (a == 198 && b == 51 && c == 100) // TEST-NET-2
        || (a == 203 && b == 0 && c == 113) // TEST-NET-3
        || a >= 224 // multicast, reserved, and 255.255.255.255
}

fn is_forbidden_v6(ip: Ipv6Addr) -> bool {
    let s = ip.segments();

    if ip.is_unspecified() || ip.is_loopback() {
        return true;
    }
    // ::ffff:a.b.c.d (mapped) and ::a.b.c.d (the deprecated compatible form):
    // the packet goes to the IPv4 address, so judge that.
    if let Some(v4) = ip.to_ipv4() {
        return is_forbidden_v4(v4);
    }
    // NAT64, 64:ff9b::/96: the gateway forwards to the embedded address.
    if s[0] == 0x64 && s[1] == 0xff9b && s[2..6] == [0, 0, 0, 0] {
        return is_forbidden_v4(embedded_v4(s[6], s[7]));
    }
    // 6to4, 2002::/16: the relay forwards to the address in bits 16..48.
    if s[0] == 0x2002 {
        return is_forbidden_v4(embedded_v4(s[1], s[2]));
    }

    (s[0] == 0x64 && s[1] == 0xff9b && s[2] == 1) // local-use NAT64, 64:ff9b:1::/48
        || (s[0] == 0x100 && s[1..4] == [0, 0, 0]) // discard-only, 100::/64
        // IETF protocol assignments, 2001::/23. Includes Teredo (2001::/32),
        // which tunnels to an address this cannot judge, and benchmarking.
        || (s[0] == 0x2001 && s[1] < 0x200)
        || (s[0] == 0x2001 && s[1] == 0xdb8) // documentation
        || (s[0] & 0xfff0) == 0x3ff0 // documentation, 3fff::/20 (RFC 9637)
        || (s[0] & 0xfe00) == 0xfc00 // unique local, fc00::/7
        || (s[0] & 0xffc0) == 0xfe80 // link-local, fe80::/10
        || (s[0] & 0xffc0) == 0xfec0 // site-local (deprecated), fec0::/10
        || (s[0] & 0xff00) == 0xff00 // multicast
}

fn embedded_v4(high: u16, low: u16) -> Ipv4Addr {
    let [a, b] = high.to_be_bytes();
    let [c, d] = low.to_be_bytes();
    Ipv4Addr::new(a, b, c, d)
}

/// Check a tenant backend's `host:port`, returning it in canonical form.
///
/// Refuses an IP literal in any [forbidden range](is_forbidden_address),
/// `localhost` and the other special-use names, single-label names (which
/// resolve through the search list to hosts inside the network), and anything
/// that is not a valid hostname. A port is required.
///
/// A hostname is checked only for what it says, not for what it resolves to:
/// resolution at write time proves nothing about resolution at connect time,
/// so whatever eventually connects to a tenant backend must check the address
/// it actually connects to, with [`is_forbidden_address`].
pub fn validate_backend_address(address: &str) -> ProxyResult<String> {
    let bad = |reason: &str| ProxyError::Validation(format!("invalid backend address: {reason}"));

    let (host, port) = if let Some(rest) = address.strip_prefix('[') {
        let (host, after) = rest
            .split_once(']')
            .ok_or_else(|| bad("an unterminated IPv6 literal"))?;
        let port = after
            .strip_prefix(':')
            .ok_or_else(|| bad("expected host:port"))?;
        (host, port)
    } else {
        address
            .rsplit_once(':')
            .ok_or_else(|| bad("expected host:port"))?
    };

    let port: u16 = port
        .parse()
        .ok()
        .filter(|p| *p != 0)
        .ok_or_else(|| bad("the port must be a number from 1 to 65535"))?;

    if let Ok(ip) = host.parse::<IpAddr>() {
        if is_forbidden_address(ip) {
            return Err(bad(
                "the address is loopback, private, link-local or otherwise not public",
            ));
        }
        return Ok(match ip {
            IpAddr::V4(v4) => format!("{v4}:{port}"),
            IpAddr::V6(v6) => format!("[{v6}]:{port}"),
        });
    }
    if address.starts_with('[') {
        return Err(bad("the bracketed host is not an IPv6 address"));
    }

    let host =
        normalize_domain(host).map_err(|_| bad("the host is not a valid public hostname"))?;
    if ReservedDomains::default().is_reserved(&host) {
        return Err(bad("the host is a special-use name such as localhost"));
    }
    Ok(format!("{host}:{port}"))
}

/// Check a backend's scheme. Only `http` and `https` mean anything.
pub fn validate_scheme(scheme: &str) -> ProxyResult<()> {
    match scheme {
        "http" | "https" => Ok(()),
        _ => Err(ProxyError::Validation(
            "invalid backend scheme: must be http or https".into(),
        )),
    }
}

/// Headers a tenant may not set on a forwarded request.
///
/// Each of these changes how the request is framed or routed rather than what
/// it says -- a second `Content-Length` or a `Transfer-Encoding` is the raw
/// material of request smuggling, and `Host` decides which upstream virtual
/// host answers. They belong to the proxy.
const PROTECTED_HEADERS: &[&str] = &[
    "host",
    "content-length",
    "transfer-encoding",
    "connection",
    "keep-alive",
    "proxy-connection",
    "te",
    "trailer",
    "upgrade",
];

/// The most headers a route may add or remove. Generous; there to bound the row.
const MAX_HEADERS: usize = 64;

/// Check headers a route adds: valid names and values, none of them protected.
pub fn validate_add_headers<'a>(
    headers: impl IntoIterator<Item = (&'a String, &'a String)>,
) -> ProxyResult<()> {
    let mut count = 0;
    for (name, value) in headers {
        count += 1;
        validate_header_name(name)?;
        if PROTECTED_HEADERS.contains(&name.to_ascii_lowercase().as_str()) {
            return Err(ProxyError::Validation(format!(
                "header {name} is managed by the proxy and cannot be set by a route"
            )));
        }
        // `HeaderValue::from_str` refuses every control character but tab,
        // which is what keeps a CR or LF from splitting the header.
        if value.len() > 8192 || http::HeaderValue::from_str(value).is_err() {
            return Err(ProxyError::Validation(format!(
                "the value for header {name} is not a valid header value"
            )));
        }
    }
    if count > MAX_HEADERS {
        return Err(ProxyError::Validation(format!(
            "at most {MAX_HEADERS} headers may be added"
        )));
    }
    Ok(())
}

/// Check header names a route removes.
pub fn validate_remove_headers(headers: &[String]) -> ProxyResult<()> {
    if headers.len() > MAX_HEADERS {
        return Err(ProxyError::Validation(format!(
            "at most {MAX_HEADERS} headers may be removed"
        )));
    }
    headers
        .iter()
        .try_for_each(|name| validate_header_name(name))
}

fn validate_header_name(name: &str) -> ProxyResult<()> {
    if name.len() > 256 || http::HeaderName::from_bytes(name.as_bytes()).is_err() {
        return Err(ProxyError::Validation(format!(
            "{name:?} is not a valid header name"
        )));
    }
    Ok(())
}

/// Check an upstream's health-check path: absolute, and nothing that could
/// break out of the request line.
pub fn validate_health_check_path(path: &str) -> ProxyResult<()> {
    if !path.starts_with('/')
        || path.len() > 500
        || path
            .bytes()
            .any(|b| b.is_ascii_whitespace() || b.is_ascii_control())
        || !path.is_ascii()
    {
        return Err(ProxyError::Validation(
            "health_check_path must start with / and contain no whitespace or control characters"
                .into(),
        ));
    }
    Ok(())
}

/// Check a route's path pattern and methods.
///
/// A prefix or exact pattern is a path, so it gets the health-check path rules.
/// A regex has to compile -- with a size limit, so that a pattern which would
/// take the matcher megabytes is refused here rather than at match time.
pub fn validate_route_match(
    path_pattern: &str,
    path_type: &crate::saas::types::DomainPathMatchType,
    methods: Option<&[String]>,
) -> ProxyResult<()> {
    use crate::saas::types::DomainPathMatchType;

    match path_type {
        DomainPathMatchType::Regex => {
            if path_pattern.len() > 500
                || regex::RegexBuilder::new(path_pattern)
                    .size_limit(1 << 20)
                    .build()
                    .is_err()
            {
                return Err(ProxyError::Validation(
                    "path_pattern is not a valid regular expression".into(),
                ));
            }
        }
        DomainPathMatchType::Prefix | DomainPathMatchType::Exact => {
            validate_health_check_path(path_pattern).map_err(|_| {
                ProxyError::Validation(
                    "path_pattern must start with / and contain no whitespace or control \
                     characters"
                        .into(),
                )
            })?;
        }
    }

    for method in methods.unwrap_or_default() {
        if method.is_empty() || method.len() > 32 || !method.bytes().all(|b| b.is_ascii_uppercase())
        {
            return Err(ProxyError::Validation(format!(
                "{method:?} is not an HTTP method; use the upper-case name, such as GET"
            )));
        }
    }
    Ok(())
}

/// Check a name a tenant gives a resource: present, and fits its column.
pub fn validate_resource_name(what: &str, name: &str) -> ProxyResult<()> {
    if name.trim().is_empty() || name.len() > 255 || name.chars().any(char::is_control) {
        return Err(ProxyError::Validation(format!(
            "{what} name must be 1 to 255 characters with no control characters"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_domain_is_lowercased_and_loses_one_trailing_dot() {
        assert_eq!(normalize_domain("VICTIM.test").unwrap(), "victim.test");
        assert_eq!(normalize_domain("Victim.Test.").unwrap(), "victim.test");
        assert_eq!(
            normalize_domain("xn--bcher-kva.example").unwrap(),
            "xn--bcher-kva.example"
        );
    }

    #[test]
    fn two_trailing_dots_are_not_one_name() {
        assert!(normalize_domain("victim.test..").is_err());
    }

    #[test]
    fn non_ascii_is_refused_with_a_pointer_to_punycode() {
        let err = normalize_domain("bücher.example").unwrap_err().to_string();
        assert!(err.contains("punycode"), "{err}");
        // Cyrillic 'і', which renders like the Latin one.
        assert!(normalize_domain("vіctim.test").is_err());
    }

    #[test]
    fn ip_literals_are_refused_in_every_spelling() {
        for ip in [
            "127.0.0.1",
            "10.0.0.1",
            "::1",
            "[::1]",
            "::ffff:127.0.0.1",
            "127.1",
            "0x7f.1",
            "0177.0.0.1",
            "1.2.3.4.",
        ] {
            assert!(normalize_domain(ip).is_err(), "{ip} was accepted");
        }
    }

    #[test]
    fn single_labels_and_ldh_violations_are_refused() {
        for name in [
            "",
            "localhost",
            "intranet",
            "-a.test",
            "a-.test",
            "a_b.test",
            "a b.test",
            "a..test",
            ".test",
            "a.test/x",
            "a.test:443",
        ] {
            assert!(normalize_domain(name).is_err(), "{name:?} was accepted");
        }
        let long_label = format!("{}.test", "a".repeat(64));
        assert!(normalize_domain(&long_label).is_err());
        let long_name = format!("{}.test", vec!["a".repeat(60); 5].join("."));
        assert!(long_name.len() > 253);
        assert!(normalize_domain(&long_name).is_err());
    }

    #[test]
    fn reserved_names_cover_their_subdomains_on_a_label_boundary() {
        let reserved = ReservedDomains::default()
            .with_exact("API.Platform.example")
            .with_suffix("platform.example");
        assert!(reserved.is_reserved("platform.example"));
        assert!(reserved.is_reserved("api.platform.example"));
        assert!(reserved.is_reserved("a.b.platform.example"));
        assert!(!reserved.is_reserved("notplatform.example"));
        assert!(reserved.is_reserved("foo.localhost"));
        assert!(reserved.is_reserved("metadata.internal"));
        assert!(!reserved.is_reserved("customer.example"));

        let exact = ReservedDomains::none().with_exact("app.example");
        assert!(exact.is_reserved("app.example"));
        assert!(!exact.is_reserved("x.app.example"));
    }

    #[test]
    #[should_panic]
    fn an_invalid_reservation_fails_loudly() {
        let _ = ReservedDomains::none().with_suffix("bad name");
    }

    #[test]
    fn forbidden_addresses() {
        for ip in [
            "0.0.0.0",
            "10.1.2.3",
            "100.64.0.1",
            "100.127.255.255",
            "127.0.0.1",
            "169.254.169.254",
            "172.16.0.1",
            "172.31.255.255",
            "192.168.1.1",
            "192.0.0.8",
            "192.0.2.1",
            "198.18.0.1",
            "198.51.100.1",
            "203.0.113.1",
            "224.0.0.1",
            "240.0.0.1",
            "255.255.255.255",
            "::",
            "::1",
            "::ffff:127.0.0.1",
            "::ffff:169.254.169.254",
            "::10.0.0.1",
            "64:ff9b::a9fe:a9fe",
            "2002:7f00:1::",
            "2001::1",
            "2001:db8::1",
            "3fff::1",
            "fc00::1",
            "fd12:3456::1",
            "fe80::1",
            "fec0::1",
            "ff02::1",
            "100::1",
        ] {
            let parsed: IpAddr = ip.parse().unwrap();
            assert!(is_forbidden_address(parsed), "{ip} was allowed");
        }
    }

    #[test]
    fn public_addresses_are_allowed() {
        for ip in [
            "1.1.1.1",
            "8.8.8.8",
            "100.63.255.255",
            "100.128.0.0",
            "172.15.255.255",
            "172.32.0.0",
            "93.184.215.14",
            "2606:4700:4700::1111",
            "::ffff:8.8.8.8",
            "64:ff9b::808:808",
            "2002:808:808::",
        ] {
            let parsed: IpAddr = ip.parse().unwrap();
            assert!(!is_forbidden_address(parsed), "{ip} was refused");
        }
    }

    #[test]
    fn backend_addresses() {
        assert_eq!(
            validate_backend_address("API.Example.com:8080").unwrap(),
            "api.example.com:8080"
        );
        assert_eq!(
            validate_backend_address("8.8.8.8:443").unwrap(),
            "8.8.8.8:443"
        );
        assert_eq!(
            validate_backend_address("[2606:4700::1]:443").unwrap(),
            "[2606:4700::1]:443"
        );
        for bad in [
            "169.254.169.254:80",
            "10.0.0.5:5432",
            "127.0.0.1:8080",
            "[::1]:80",
            "[::ffff:127.0.0.1]:80",
            "localhost:80",
            "LOCALHOST:80",
            "db.localhost:80",
            "metadata.google.internal:80",
            "backend:80",
            "2130706433:80",
            "127.1:80",
            "example.com",
            "example.com:0",
            "example.com:99999",
            "example.com:http",
            "[::1:80",
            "[example.com]:80",
            "exa mple.com:80",
            "example.com:80\r\nX: y",
        ] {
            assert!(
                validate_backend_address(bad).is_err(),
                "{bad:?} was accepted"
            );
        }
    }

    #[test]
    fn schemes() {
        assert!(validate_scheme("http").is_ok());
        assert!(validate_scheme("https").is_ok());
        assert!(validate_scheme("ftp").is_err());
        assert!(validate_scheme("HTTP").is_err());
        assert!(validate_scheme("").is_err());
    }

    #[test]
    fn headers() {
        let ok = std::collections::HashMap::from([("X-Tenant".to_string(), "a b".to_string())]);
        assert!(validate_add_headers(&ok).is_ok());

        for (name, value) in [
            ("X-Bad", "a\r\nInjected: yes"),
            ("X-Bad", "a\nb"),
            ("X Bad", "v"),
            ("X-Bad:", "v"),
            ("", "v"),
            ("Host", "evil.test"),
            ("transfer-encoding", "chunked"),
            ("Content-Length", "0"),
        ] {
            let headers = std::collections::HashMap::from([(name.to_string(), value.to_string())]);
            assert!(
                validate_add_headers(&headers).is_err(),
                "{name:?}: {value:?} was accepted"
            );
        }

        assert!(validate_remove_headers(&["x-powered-by".into()]).is_ok());
        assert!(validate_remove_headers(&["bad\r\nname".into()]).is_err());
    }

    #[test]
    fn health_check_paths() {
        assert!(validate_health_check_path("/health").is_ok());
        assert!(validate_health_check_path("/health?full=1").is_ok());
        for bad in ["health", "", "/a b", "/a\r\nHost: x", "/a\tb", "http://x/"] {
            assert!(
                validate_health_check_path(bad).is_err(),
                "{bad:?} was accepted"
            );
        }
    }

    #[test]
    fn route_matches() {
        use crate::saas::types::DomainPathMatchType::*;
        assert!(validate_route_match("/api", &Prefix, Some(&["GET".into()])).is_ok());
        assert!(validate_route_match("^/v[0-9]+/", &Regex, None).is_ok());
        assert!(validate_route_match("api", &Prefix, None).is_err());
        assert!(validate_route_match("(", &Regex, None).is_err());
        assert!(validate_route_match("/", &Exact, Some(&["get".into()])).is_err());
        assert!(validate_route_match("/", &Exact, Some(&["GET\r\n".into()])).is_err());
    }
}
