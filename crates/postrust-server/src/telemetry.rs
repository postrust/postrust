//! A span for every request, and the identifiers that tie its log lines
//! together.
//!
//! [`RequestTraceLayer`] is the outermost layer of the router. For each
//! request it:
//!
//! - takes the client's `x-request-id` if it is usable, or makes one, and
//!   echoes it on the response;
//! - reads the W3C `traceparent` (and `tracestate`) into a
//!   [`TraceContext`], or starts a trace, and puts the context in the
//!   request's extensions for the REST pipeline and GraphQL to find;
//! - opens a span named `request`, at info, whose fields are OpenTelemetry's
//!   HTTP server conventions, and records the status on it when the response
//!   is ready;
//! - logs one line at debug with the status and latency.
//!
//! Field names are the OpenTelemetry semantic conventions, spelled as
//! `tracing-opentelemetry` reads them -- `otel.kind`, `otel.status_code` --
//! so an exporter can be added beneath this without renaming anything.
//!
//! # Why the span is at info and the line at debug
//!
//! The span is what makes an error logged deep in the engine say which
//! request it belonged to: every line written inside it carries `request_id`
//! and `trace_id`. That is worth having at the default level, and it costs
//! the formatting of a handful of borrowed fields per request.
//!
//! The per-request line is not. At the default `info` it would be a line for
//! every request a busy server answers, and the failures worth seeing are
//! already logged where they happen, with their cause -- a query fault at
//! error, a refusal at debug (see `query_error_log` in postrust-engine). A
//! second line at error for every 5xx would say the same thing again with
//! less in it. So the request line is at debug, whatever its status, and a
//! 5xx is marked on the span as `otel.status_code = "ERROR"` for a trace
//! backend to find.

use axum::extract::MatchedPath;
use http::{HeaderValue, Request, Response};
use pin_project_lite::pin_project;
use postrust_core::trace::{valid_request_id, TraceContext, TraceId};
use std::cell::Cell;
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Instant;
use tower::{Layer, Service};
use tracing::field::Empty;
use tracing::Span;

#[cfg(feature = "otel")]
use crate::otel::RequestMeter;

/// Without the `otel` feature there are no metrics to record, and this is
/// nothing at all.
#[cfg(not(feature = "otel"))]
struct RequestMeter;

#[cfg(not(feature = "otel"))]
impl RequestMeter {
    #[inline(always)]
    fn start(_: &http::Method, _: Option<&str>) -> Self {
        RequestMeter
    }

    #[inline(always)]
    fn finish(&self, _: http::StatusCode, _: Instant) {}
}

/// The header a request id travels in, both ways.
pub const REQUEST_ID_HEADER: &str = "x-request-id";

/// Wraps a service so that every request gets a request id, a trace context
/// and a span. See the [module documentation](self).
#[derive(Clone, Copy, Debug, Default)]
pub struct RequestTraceLayer;

impl<S> Layer<S> for RequestTraceLayer {
    type Service = RequestTrace<S>;

    fn layer(&self, inner: S) -> Self::Service {
        RequestTrace { inner }
    }
}

/// The service [`RequestTraceLayer`] produces.
#[derive(Clone, Debug)]
pub struct RequestTrace<S> {
    inner: S,
}

impl<S, B, ResBody> Service<Request<B>> for RequestTrace<S>
where
    S: Service<Request<B>, Response = Response<ResBody>>,
{
    type Response = Response<ResBody>;
    type Error = S::Error;
    type Future = ResponseFuture<S::Future>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, mut request: Request<B>) -> Self::Future {
        let start = Instant::now();
        let request_id = request_id(request.headers());
        let trace = TraceContext::from_headers(request.headers())
            .unwrap_or_else(|| TraceContext::root(new_trace_id()));

        // The span's fields borrow from the request; `info_span!` evaluates
        // them only when a subscriber wants the span, and the trace id is
        // recorded as `Display`, formatted only by whoever writes it out.
        let span = tracing::info_span!(
            "request",
            "http.request.method" = request.method().as_str(),
            "url.path" = request.uri().path(),
            "http.route" = Empty,
            "http.response.status_code" = Empty,
            "otel.kind" = "server",
            "otel.status_code" = Empty,
            // `{method} {route}`, the name an exporter gives the span; only
            // recorded when one is exporting. See `otel::name_request_span`.
            "otel.name" = Empty,
            // Visible ASCII, checked or generated, so `to_str` cannot fail.
            request_id = request_id.to_str().unwrap_or_default(),
            trace_id = %trace.trace_id,
        );
        // The route the router matched, not the path: `/api/{*path}` for every
        // REST resource, so a dashboard grouping by route has a handful of
        // rows rather than one per table and id. Absent where nothing matched
        // -- a 404, or the compatibility-mode fallback -- and then left unset
        // rather than guessed, as the conventions ask.
        let route = request.extensions().get::<MatchedPath>();
        if let Some(route) = route {
            span.record("http.route", route.as_str());
        }
        let meter = RequestMeter::start(request.method(), route.map(MatchedPath::as_str));

        // With the `otel` feature and an exporter, the exported span is
        // named as the conventions name it, and joins the caller's trace --
        // or starts one under this trace id. Neither does anything otherwise.
        // In that order: starting a trace here starts the exported span, and
        // a span already started keeps the name it started with.
        #[cfg(feature = "otel")]
        {
            crate::otel::name_request_span(&span, request.method(), route.map(MatchedPath::as_str));
            crate::otel::join_trace(&span, &trace);
        }

        request.extensions_mut().insert(trace);

        let inner = {
            let _entered = span.enter();
            self.inner.call(request)
        };
        ResponseFuture {
            inner,
            span,
            request_id: Some(request_id),
            start,
            meter,
        }
    }
}

pin_project! {
    /// The response future of [`RequestTrace`].
    pub struct ResponseFuture<F> {
        #[pin]
        inner: F,
        span: Span,
        request_id: Option<HeaderValue>,
        start: Instant,
        // Counts the request as active until this future is dropped, however
        // it ends -- a client that goes away mid-request included.
        meter: RequestMeter,
    }
}

impl<F, ResBody, E> Future for ResponseFuture<F>
where
    F: Future<Output = Result<Response<ResBody>, E>>,
{
    type Output = Result<Response<ResBody>, E>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.project();
        let _entered = this.span.enter();
        let mut response = match this.inner.poll(cx) {
            Poll::Pending => return Poll::Pending,
            Poll::Ready(Ok(response)) => response,
            Poll::Ready(Err(e)) => return Poll::Ready(Err(e)),
        };

        let status = response.status();
        this.span
            .record("http.response.status_code", status.as_u16());
        // Only a 5xx is the server's failure. A 4xx is the client's, and the
        // conventions leave a server span's status unset for one.
        if status.is_server_error() {
            this.span.record("otel.status_code", "ERROR");
        }
        this.meter.finish(status, *this.start);
        tracing::debug!(
            status = status.as_u16(),
            latency_ms = this.start.elapsed().as_secs_f64() * 1000.0,
            "request finished"
        );

        // A response that already names a request id keeps it: a function can
        // set response headers through `response.headers`, and what it said
        // was asked for.
        if let Some(id) = this.request_id.take() {
            response
                .headers_mut()
                .entry(REQUEST_ID_HEADER)
                .or_insert(id);
        }
        Poll::Ready(Ok(response))
    }
}

/// The client's `x-request-id`, if it sent exactly one that can be used as
/// given, or a new one.
///
/// A value that is too long or carries anything but visible ASCII is replaced
/// rather than trimmed: an id that has been altered no longer matches what the
/// client logged, which is the only thing it was for.
pub fn request_id(headers: &http::HeaderMap) -> HeaderValue {
    let mut sent = headers.get_all(REQUEST_ID_HEADER).iter();
    match (sent.next(), sent.next()) {
        (Some(id), None) if valid_request_id(id.as_bytes()) => id.clone(),
        _ => new_request_id(),
    }
}

/// A new request id: a version 4 UUID, in its usual spelling.
pub fn new_request_id() -> HeaderValue {
    let mut bytes = random_u128().to_be_bytes();
    // Version 4, variant RFC 4122: what makes it a UUID and not 128 bits.
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;

    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut text = [0u8; 36];
    let mut at = 0;
    for (i, b) in bytes.iter().enumerate() {
        if matches!(i, 4 | 6 | 8 | 10) {
            text[at] = b'-';
            at += 1;
        }
        text[at] = HEX[(b >> 4) as usize];
        text[at + 1] = HEX[(b & 0x0f) as usize];
        at += 2;
    }
    HeaderValue::from_bytes(&text).expect("hex digits and hyphens are a valid header value")
}

/// A new trace id, for a request that did not arrive as part of one.
pub fn new_trace_id() -> TraceId {
    loop {
        // All zeroes is the one value refused, at a chance of 2^-128.
        if let Some(id) = TraceId::from_bytes(random_u128().to_be_bytes()) {
            return id;
        }
    }
}

thread_local! {
    static RNG_STATE: Cell<u64> = Cell::new(seed());
}

/// A seed from the standard library's per-process random keys. `RandomState`
/// exists to make hash tables unpredictable, so it is seeded from the
/// operating system; hashing anything with it gives a different 64 bits on
/// every thread.
fn seed() -> u64 {
    use std::hash::BuildHasher;
    std::collections::hash_map::RandomState::new().hash_one(0x5eed_u64)
}

/// 128 bits from SplitMix64, kept per thread.
///
/// Not a cryptographic generator, and these ids are not secrets: a client may
/// send any request id it likes, and a trace id is written into every log line
/// and passed to every service downstream. What they need is to be unique and
/// spread evenly, and this gives that for a few nanoseconds, where reading
/// the operating system's generator for each request would be a system call.
fn random_u128() -> u128 {
    fn next(state: &Cell<u64>) -> u64 {
        let s = state.get().wrapping_add(0x9e37_79b9_7f4a_7c15);
        state.set(s);
        let mut z = s;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }
    RNG_STATE.with(|state| ((next(state) as u128) << 64) | next(state) as u128)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_generated_request_id_is_a_v4_uuid() {
        let id = new_request_id();
        let text = id.to_str().unwrap();
        assert_eq!(text.len(), 36);
        let parts: Vec<&str> = text.split('-').collect();
        assert_eq!(
            parts.iter().map(|p| p.len()).collect::<Vec<_>>(),
            [8, 4, 4, 4, 12]
        );
        assert!(parts[2].starts_with('4'));
        assert!(matches!(parts[3].as_bytes()[0], b'8' | b'9' | b'a' | b'b'));
        assert!(valid_request_id(id.as_bytes()));
        assert_ne!(new_request_id(), id);
    }

    #[test]
    fn a_usable_request_id_is_kept() {
        let mut headers = http::HeaderMap::new();
        headers.insert(REQUEST_ID_HEADER, "abc-123".parse().unwrap());
        assert_eq!(request_id(&headers), "abc-123");
    }

    #[test]
    fn an_unusable_request_id_is_replaced() {
        for bad in ["has space", &"x".repeat(129)] {
            let mut headers = http::HeaderMap::new();
            headers.insert(REQUEST_ID_HEADER, bad.parse().unwrap());
            let id = request_id(&headers);
            assert_ne!(id, bad);
            assert_eq!(id.len(), 36);
        }
        // Two ids is no id: there is no telling which the client logged.
        let mut headers = http::HeaderMap::new();
        headers.append(REQUEST_ID_HEADER, "one".parse().unwrap());
        headers.append(REQUEST_ID_HEADER, "two".parse().unwrap());
        assert_eq!(request_id(&headers).len(), 36);
    }

    #[test]
    fn trace_ids_differ() {
        assert_ne!(new_trace_id(), new_trace_id());
    }
}
