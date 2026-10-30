//! W3C trace context, and the request identifier that travels beside it.
//!
//! A request that arrives with a `traceparent` header is part of a trace
//! somebody else started: a gateway, a client library, another service. Its
//! trace id is kept so the lines this server logs -- and, once spans are
//! exported, the spans themselves -- join that trace rather than starting an
//! unrelated one. A request without one starts a trace of its own.
//!
//! Hand-written rather than taken from the `opentelemetry` crates, which this
//! side does not otherwise need: parsing `traceparent` is a fixed-width format
//! of four fields, and the rules for refusing one are short enough to read
//! here. See <https://www.w3.org/TR/trace-context/>.
//!
//! Nothing here generates an identifier. That needs randomness, and where it
//! comes from is a host's business -- the server has a thread to keep a
//! generator on, a Cloudflare Worker has the JavaScript runtime's. So this
//! module only parses, validates and formats, and builds on wasm32 as it does
//! everywhere else.

use std::fmt;

/// A trace id: sixteen bytes, never all zero.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TraceId([u8; 16]);

impl TraceId {
    /// The id these bytes spell, or `None` for all zeroes, which the
    /// specification reserves to mean "no trace".
    pub fn from_bytes(bytes: [u8; 16]) -> Option<Self> {
        (bytes != [0; 16]).then_some(Self(bytes))
    }

    /// The id's bytes.
    pub fn to_bytes(self) -> [u8; 16] {
        self.0
    }
}

// Formatted when a subscriber asks for it, not before: a span records this as
// `Display`, so a request whose span nobody is listening to never pays for the
// thirty-two characters.
impl fmt::Display for TraceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write_hex(f, &self.0)
    }
}

/// A span id: eight bytes, never all zero. Here, the id of the caller's span
/// that this request is a child of.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SpanId([u8; 8]);

impl SpanId {
    /// The id these bytes spell, or `None` for all zeroes.
    pub fn from_bytes(bytes: [u8; 8]) -> Option<Self> {
        (bytes != [0; 8]).then_some(Self(bytes))
    }

    /// The id's bytes.
    pub fn to_bytes(self) -> [u8; 8] {
        self.0
    }
}

impl fmt::Display for SpanId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write_hex(f, &self.0)
    }
}

fn write_hex(f: &mut fmt::Formatter<'_>, bytes: &[u8]) -> fmt::Result {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    // Sixteen bytes at most, so thirty-two characters on the stack.
    let mut buf = [0u8; 32];
    for (i, b) in bytes.iter().enumerate() {
        buf[i * 2] = HEX[(b >> 4) as usize];
        buf[i * 2 + 1] = HEX[(b & 0x0f) as usize];
    }
    // Only ASCII hex digits were written.
    f.write_str(std::str::from_utf8(&buf[..bytes.len() * 2]).map_err(|_| fmt::Error)?)
}

/// The trace a request belongs to.
///
/// Put in the request's extensions by the host before the request reaches the
/// REST pipeline or GraphQL, so anything downstream can find it without
/// parsing the headers again.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct TraceContext {
    /// The trace the request is part of.
    pub trace_id: TraceId,
    /// The caller's span, when the request said which it was. `None` when the
    /// trace started here.
    pub parent_id: Option<SpanId>,
    /// `trace-flags` as the caller sent them; bit 0 is "sampled". Zero when
    /// the trace started here.
    pub flags: u8,
    /// `tracestate`, carried as it came, for whatever propagates the trace on.
    /// Only kept beside a `traceparent` that parsed: the specification has it
    /// mean nothing on its own.
    pub tracestate: Option<http::HeaderValue>,
}

impl TraceContext {
    /// A trace that starts with this request.
    pub fn root(trace_id: TraceId) -> Self {
        Self {
            trace_id,
            parent_id: None,
            flags: 0,
            tracestate: None,
        }
    }

    /// The trace the request's `traceparent` and `tracestate` name, or `None`
    /// when there is no `traceparent` or it cannot be used.
    ///
    /// More than one `traceparent` is refused rather than one of them picked:
    /// the specification leaves no way to tell which is meant. Several
    /// `tracestate` headers are one list split across lines, and are joined.
    pub fn from_headers(headers: &http::HeaderMap) -> Option<Self> {
        let mut values = headers.get_all("traceparent").iter();
        let traceparent = values.next()?;
        if values.next().is_some() {
            return None;
        }
        let (trace_id, parent_id, flags) = parse_traceparent(traceparent.as_bytes())?;

        let mut states = headers.get_all("tracestate").iter();
        let tracestate = match (states.next(), states.next()) {
            (None, _) => None,
            (Some(only), None) => Some(only.clone()),
            // Rare enough that allocating to join them is not worth avoiding.
            (Some(first), Some(second)) => {
                let mut joined = first.as_bytes().to_vec();
                for value in std::iter::once(second).chain(states) {
                    joined.push(b',');
                    joined.extend_from_slice(value.as_bytes());
                }
                http::HeaderValue::from_bytes(&joined).ok()
            }
        };

        Some(Self {
            trace_id,
            parent_id: Some(parent_id),
            flags,
            tracestate,
        })
    }

    /// Whether the caller said this trace is being recorded.
    pub fn sampled(&self) -> bool {
        self.flags & 0x01 != 0
    }
}

/// Parse a `traceparent` value: `version-traceid-parentid-flags`, as
/// lowercase hex.
///
/// Refused, as the specification says it must be: version `ff`; a trace id or
/// parent id of all zeroes; uppercase hex; a version-`00` value with anything
/// after the flags. A version this side does not know is read as `00` would
/// be, provided what follows the flags starts a new field -- the
/// specification's provision for versions not yet written.
pub fn parse_traceparent(value: &[u8]) -> Option<(TraceId, SpanId, u8)> {
    if value.len() < 55 {
        return None;
    }
    let version = hex_byte(&value[0..2])?;
    if version == 0xff {
        return None;
    }
    if version == 0x00 && value.len() != 55 {
        return None;
    }
    if value.len() > 55 && value[55] != b'-' {
        return None;
    }
    if value[2] != b'-' || value[35] != b'-' || value[52] != b'-' {
        return None;
    }

    let mut trace = [0u8; 16];
    for (i, byte) in trace.iter_mut().enumerate() {
        *byte = hex_byte(&value[3 + i * 2..5 + i * 2])?;
    }
    let mut parent = [0u8; 8];
    for (i, byte) in parent.iter_mut().enumerate() {
        *byte = hex_byte(&value[36 + i * 2..38 + i * 2])?;
    }
    let flags = hex_byte(&value[53..55])?;

    Some((
        TraceId::from_bytes(trace)?,
        SpanId::from_bytes(parent)?,
        flags,
    ))
}

/// Two lowercase hex digits. Uppercase is refused: the specification spells
/// every field in lowercase, and a header that does not is not one it wrote.
fn hex_byte(pair: &[u8]) -> Option<u8> {
    fn digit(c: u8) -> Option<u8> {
        match c {
            b'0'..=b'9' => Some(c - b'0'),
            b'a'..=b'f' => Some(c - b'a' + 10),
            _ => None,
        }
    }
    Some((digit(pair[0])? << 4) | digit(pair[1])?)
}

/// The longest `x-request-id` taken from a client. Long enough for any
/// identifier scheme in use -- a UUID is 36, a ULID 26, a Heroku or Envoy id
/// well under this -- and short enough that the header cannot be used to make
/// every log line carry a payload.
pub const MAX_REQUEST_ID_LEN: usize = 128;

/// Whether a client's `x-request-id` can be used as given: between one and
/// [`MAX_REQUEST_ID_LEN`] bytes, every one of them visible ASCII.
///
/// Visible ASCII and nothing else, because the id is written into log lines
/// and echoed back as a header: no spaces to split a log field on, no control
/// characters, nothing a terminal or a log shipper might interpret.
pub fn valid_request_id(id: &[u8]) -> bool {
    (1..=MAX_REQUEST_ID_LEN).contains(&id.len()) && id.iter().all(|b| (0x21..=0x7e).contains(b))
}

#[cfg(test)]
mod tests {
    use super::*;

    const VALID: &str = "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01";

    #[test]
    fn a_valid_traceparent_parses() {
        let (trace, parent, flags) = parse_traceparent(VALID.as_bytes()).unwrap();
        assert_eq!(trace.to_string(), "4bf92f3577b34da6a3ce929d0e0e4736");
        assert_eq!(parent.to_string(), "00f067aa0ba902b7");
        assert_eq!(flags, 0x01);
    }

    #[test]
    fn version_ff_is_refused() {
        let value = VALID.replacen("00", "ff", 1);
        assert_eq!(parse_traceparent(value.as_bytes()), None);
    }

    #[test]
    fn all_zero_ids_are_refused() {
        let zero_trace = "00-00000000000000000000000000000000-00f067aa0ba902b7-01";
        let zero_parent = "00-4bf92f3577b34da6a3ce929d0e0e4736-0000000000000000-01";
        assert_eq!(parse_traceparent(zero_trace.as_bytes()), None);
        assert_eq!(parse_traceparent(zero_parent.as_bytes()), None);
    }

    #[test]
    fn malformed_values_are_refused() {
        for value in [
            "",
            "00",
            // Uppercase hex.
            "00-4BF92F3577B34DA6A3CE929D0E0E4736-00f067aa0ba902b7-01",
            // A separator missing.
            "00-4bf92f3577b34da6a3ce929d0e0e4736x00f067aa0ba902b7-01",
            // Not hex.
            "00-4bf92f3577b34da6a3ce929d0e0e473g-00f067aa0ba902b7-01",
            // Too short.
            "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-1",
            // Version 00 with something after the flags.
            "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01-extra",
            // A later version whose trailing data does not start a field.
            "cc-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01x",
        ] {
            assert_eq!(parse_traceparent(value.as_bytes()), None, "{value:?}");
        }
    }

    #[test]
    fn a_later_version_is_read_as_00() {
        let value = "cc-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01-what-the-future-holds";
        let (trace, _, _) = parse_traceparent(value.as_bytes()).unwrap();
        assert_eq!(trace.to_string(), "4bf92f3577b34da6a3ce929d0e0e4736");
        // And exactly the version-00 length is fine too.
        assert!(parse_traceparent(VALID.replacen("00", "01", 1).as_bytes()).is_some());
    }

    #[test]
    fn trace_context_comes_from_headers() {
        let mut headers = http::HeaderMap::new();
        headers.insert("traceparent", VALID.parse().unwrap());
        headers.append("tracestate", "rojo=00f067aa0ba902b7".parse().unwrap());
        headers.append("tracestate", "congo=t61rcWkgMzE".parse().unwrap());
        let context = TraceContext::from_headers(&headers).unwrap();
        assert_eq!(
            context.trace_id.to_string(),
            "4bf92f3577b34da6a3ce929d0e0e4736"
        );
        assert!(context.sampled());
        assert_eq!(
            context.tracestate.unwrap(),
            "rojo=00f067aa0ba902b7,congo=t61rcWkgMzE"
        );
    }

    #[test]
    fn two_traceparents_are_no_traceparent() {
        let mut headers = http::HeaderMap::new();
        headers.append("traceparent", VALID.parse().unwrap());
        headers.append("traceparent", VALID.parse().unwrap());
        assert_eq!(TraceContext::from_headers(&headers), None);
    }

    #[test]
    fn tracestate_alone_means_nothing() {
        let mut headers = http::HeaderMap::new();
        headers.insert("tracestate", "rojo=00f067aa0ba902b7".parse().unwrap());
        assert_eq!(TraceContext::from_headers(&headers), None);
    }

    #[test]
    fn request_ids_are_bounded_visible_ascii() {
        assert!(valid_request_id(b"0f8fad5b-d9cb-469f-a165-70867728950e"));
        assert!(valid_request_id(b"a"));
        assert!(valid_request_id(&[b'x'; MAX_REQUEST_ID_LEN]));
        assert!(!valid_request_id(b""));
        assert!(!valid_request_id(&[b'x'; MAX_REQUEST_ID_LEN + 1]));
        assert!(!valid_request_id(b"has space"));
        assert!(!valid_request_id(b"tab\there"));
        assert!(!valid_request_id("caf\u{e9}".as_bytes()));
    }
}
