//! Accepting the request targets PostgREST clients actually send.
//!
//! `select=data->>id` is ordinary PostgREST usage: it is what curl puts on the
//! wire and what the PostgREST documentation shows. Haskell's Warp, which
//! serves PostgREST, hands those bytes to the application unexamined.
//!
//! hyper builds an [`http::Uri`] from the request target, and that rejects
//! three printable characters -- `"`, `<` and `>` -- along with everything
//! from `0x80` up. httparse, which reads the request line first, is happy with
//! all of them. So the request is answered `400` with an empty body before any
//! routing, handler, or error formatting of ours runs, and there is no point
//! above the connection where it can be caught.
//!
//! This adapter percent-encodes those bytes in the request target, and only
//! there. The query string is percent-decoded when it is parsed, so nothing
//! above this sees a difference.
//!
//! # Not touching anything else
//!
//! A request line appears only at the start of a message, and mistaking body
//! bytes for one would corrupt the body. So the adapter follows the framing:
//! it reads each request line, then the header block, and uses `Content-Length`
//! to know how many body bytes to pass through before another request line can
//! begin.
//!
//! The moment framing becomes something it does not model -- a chunked body, a
//! header block too large to buffer, a request line it cannot make sense of --
//! it stops rewriting for the remainder of the connection and copies bytes
//! through verbatim. That degrades to exactly hyper's own behaviour. It is
//! never left guessing whether the bytes in hand are a request line.

use std::io;
use std::pin::Pin;
use std::task::{Context, Poll};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

/// Bytes that httparse accepts in a request target but `http::Uri` rejects,
/// or reads as something other than part of the path and query.
///
/// Checked against http 1.4 by parsing a URI containing each byte in turn.
///
/// `#` is not rejected -- it is read as starting a fragment, and everything
/// after it disappears from the query. A fragment is a client-side notion that
/// never reaches a server, so a `#` in a request target is an ordinary
/// character of the query: `?select=data->!@#$%^&*_d` names a JSON key that
/// happens to contain one.
fn needs_encoding(b: u8) -> bool {
    matches!(b, b'"' | b'<' | b'>' | b'#') || b >= 0x80
}

/// Cap on a buffered request line or header line.
///
/// Beyond this the adapter stops rewriting rather than buffering without
/// bound; hyper applies its own, smaller limits once it sees the bytes.
const MAX_LINE: usize = 64 * 1024;

/// The bytes a client sends to open an HTTP/2 connection without negotiating.
///
/// Nothing serves HTTP/2 here today: `axum::serve` builds on hyper-util's
/// automatic builder, which speaks it only with `hyper-util/http2` on, and
/// this workspace does not enable it -- so such a connection is refused
/// before it starts.
///
/// Recognised anyway, because of what happens if it ever is enabled, which is
/// the kind of thing feature unification does without anyone deciding it.
/// Everything after this preface is binary frames, and HPACK sets the high bit
/// constantly; read as request lines they are full of bytes this adapter would
/// percent-encode, so the connections would be quietly corrupted rather than
/// refused. This costs a comparison of at most one byte per connection -- no
/// HTTP method but `PRI` shares even a first character with it.
const H2_PREFACE: &[u8] = b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n";

#[derive(Debug)]
enum State {
    /// At the very start of the connection, deciding whether this is HTTP/2.
    Preface,
    /// At the start of a message, reading the request line.
    RequestLine,
    /// Reading the header block that follows a request line.
    Headers { content_length: u64 },
    /// Copying a body through; this many bytes still to come.
    Body(u64),
    /// Framing is no longer modelled: copy everything, rewrite nothing.
    PassThrough,
}

/// A connection whose request targets are normalised as they are read.
#[derive(Debug)]
pub struct LenientStream<T> {
    inner: T,
    state: State,
    /// Bytes processed and waiting to be handed to the reader.
    out: Vec<u8>,
    out_pos: usize,
    /// A request or header line still being accumulated.
    line: Vec<u8>,
}

impl<T> LenientStream<T> {
    pub fn new(inner: T) -> Self {
        Self {
            inner,
            state: State::Preface,
            out: Vec::new(),
            out_pos: 0,
            line: Vec::new(),
        }
    }

    /// Give up on framing: flush what is buffered and copy through from here.
    fn give_up(&mut self) {
        let line = std::mem::take(&mut self.line);
        self.out.extend_from_slice(&line);
        self.state = State::PassThrough;
    }

    /// Feed freshly read bytes through the state machine.
    fn process(&mut self, mut data: &[u8]) {
        while !data.is_empty() {
            match self.state {
                State::PassThrough => {
                    self.out.extend_from_slice(data);
                    return;
                }
                State::Preface => {
                    // Held back rather than emitted, because until enough of
                    // it has arrived to rule the preface out these bytes might
                    // not be a request line at all.
                    let want = H2_PREFACE.len() - self.line.len();
                    let take = std::cmp::min(want, data.len());
                    self.line.extend_from_slice(&data[..take]);

                    if !H2_PREFACE.starts_with(&self.line) {
                        // An ordinary request line, and one byte is usually
                        // enough to know it. Read it as one, from the start.
                        self.state = State::RequestLine;
                        let mut buffered = std::mem::take(&mut self.line);
                        buffered.extend_from_slice(&data[take..]);
                        self.process(&buffered);
                        return;
                    }

                    data = &data[take..];
                    if self.line.len() == H2_PREFACE.len() {
                        self.give_up();
                    }
                }
                State::Body(remaining) => {
                    let take = std::cmp::min(remaining, data.len() as u64) as usize;
                    self.out.extend_from_slice(&data[..take]);
                    data = &data[take..];
                    let left = remaining - take as u64;
                    self.state = if left == 0 {
                        State::RequestLine
                    } else {
                        State::Body(left)
                    };
                }
                State::RequestLine | State::Headers { .. } => {
                    match data.iter().position(|&b| b == b'\n') {
                        Some(idx) => {
                            self.line.extend_from_slice(&data[..=idx]);
                            data = &data[idx + 1..];
                            self.finish_line();
                        }
                        None => {
                            self.line.extend_from_slice(data);
                            data = &[];
                            if self.line.len() > MAX_LINE {
                                self.give_up();
                            }
                        }
                    }
                }
            }
        }
    }

    /// Handle one complete line, newline included.
    fn finish_line(&mut self) {
        let line = std::mem::take(&mut self.line);

        match self.state {
            State::RequestLine => {
                self.out.extend_from_slice(&normalize_request_line(&line));
                self.state = State::Headers { content_length: 0 };
            }
            State::Headers { content_length } => {
                let is_blank = matches!(line.as_slice(), b"\r\n" | b"\n");
                if is_blank {
                    self.out.extend_from_slice(&line);
                    self.state = if content_length == 0 {
                        State::RequestLine
                    } else {
                        State::Body(content_length)
                    };
                    return;
                }

                // A chunked body is framed by the body itself, which this does
                // not read, so where the next message starts is unknown.
                if header_is(&line, b"transfer-encoding") {
                    self.out.extend_from_slice(&line);
                    self.state = State::PassThrough;
                    return;
                }

                let content_length = match header_value(&line, b"content-length") {
                    Some(value) => match std::str::from_utf8(value)
                        .ok()
                        .and_then(|v| v.trim().parse::<u64>().ok())
                    {
                        Some(len) => len,
                        // Unparseable framing: stop guessing.
                        None => {
                            self.out.extend_from_slice(&line);
                            self.state = State::PassThrough;
                            return;
                        }
                    },
                    None => content_length,
                };

                self.out.extend_from_slice(&line);
                self.state = State::Headers { content_length };
            }
            State::Preface | State::Body(_) | State::PassThrough => {
                unreachable!("not a line state")
            }
        }
    }
}

/// Whether a header line names the given (lowercase) header.
fn header_is(line: &[u8], name: &[u8]) -> bool {
    match line.iter().position(|&b| b == b':') {
        Some(colon) => line[..colon].eq_ignore_ascii_case(name),
        None => false,
    }
}

/// The value of a header line, if it names the given (lowercase) header.
fn header_value<'a>(line: &'a [u8], name: &[u8]) -> Option<&'a [u8]> {
    let colon = line.iter().position(|&b| b == b':')?;
    line[..colon]
        .eq_ignore_ascii_case(name)
        .then(|| &line[colon + 1..])
}

/// Percent-encode the target of a request line, leaving the rest alone.
///
/// A line that is not shaped like `METHOD SP TARGET SP VERSION` is returned
/// unchanged, for hyper to reject as it sees fit.
fn normalize_request_line(line: &[u8]) -> Vec<u8> {
    let end = line.len() - trailing_newline_len(line);
    let head = &line[..end];

    let (Some(first), Some(last)) = (
        head.iter().position(|&b| b == b' '),
        head.iter().rposition(|&b| b == b' '),
    ) else {
        return line.to_vec();
    };
    if first == last {
        return line.to_vec();
    }

    let target = &head[first + 1..last];
    if !target.iter().copied().any(needs_encoding) {
        return line.to_vec();
    }

    let mut out = Vec::with_capacity(line.len() + 16);
    out.extend_from_slice(&head[..=first]);
    for &b in target {
        if needs_encoding(b) {
            out.extend_from_slice(format!("%{:02X}", b).as_bytes());
        } else {
            out.push(b);
        }
    }
    out.extend_from_slice(&head[last..]);
    out.extend_from_slice(&line[end..]);
    out
}

fn trailing_newline_len(line: &[u8]) -> usize {
    if line.ends_with(b"\r\n") {
        2
    } else if line.ends_with(b"\n") {
        1
    } else {
        0
    }
}

impl<T: AsyncRead + Unpin> AsyncRead for LenientStream<T> {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        dst: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let me = self.get_mut();

        loop {
            if me.out_pos < me.out.len() {
                let take = std::cmp::min(dst.remaining(), me.out.len() - me.out_pos);
                dst.put_slice(&me.out[me.out_pos..me.out_pos + take]);
                me.out_pos += take;
                if me.out_pos == me.out.len() {
                    me.out.clear();
                    me.out_pos = 0;
                }
                return Poll::Ready(Ok(()));
            }

            // Once framing is no longer tracked there is nothing to do but
            // hand the reader the socket directly.
            if matches!(me.state, State::PassThrough) && me.line.is_empty() {
                return Pin::new(&mut me.inner).poll_read(cx, dst);
            }

            let mut scratch = [0u8; 8192];
            let mut read = ReadBuf::new(&mut scratch);
            match Pin::new(&mut me.inner).poll_read(cx, &mut read) {
                Poll::Ready(Ok(())) => {
                    let filled = read.filled();
                    if filled.is_empty() {
                        // End of stream. Anything half-read is still owed to
                        // the reader, so it is not dropped silently.
                        if !me.line.is_empty() {
                            me.give_up();
                            continue;
                        }
                        return Poll::Ready(Ok(()));
                    }
                    let owned = filled.to_vec();
                    me.process(&owned);
                }
                Poll::Ready(Err(e)) => return Poll::Ready(Err(e)),
                Poll::Pending => return Poll::Pending,
            }
        }
    }
}

impl<T: AsyncWrite + Unpin> AsyncWrite for LenientStream<T> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.inner).poll_write(cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}

/// A [`TcpListener`](tokio::net::TcpListener) whose connections normalise
/// request targets.
pub struct LenientListener(pub tokio::net::TcpListener);

impl axum::serve::Listener for LenientListener {
    type Io = LenientStream<tokio::net::TcpStream>;
    type Addr = std::net::SocketAddr;

    async fn accept(&mut self) -> (Self::Io, Self::Addr) {
        loop {
            match self.0.accept().await {
                Ok((stream, addr)) => return (LenientStream::new(stream), addr),
                // Mirrors axum's own listener: a failed accept is transient,
                // and spinning on it would burn a core.
                Err(e) => {
                    tracing::debug!("accept failed: {e}");
                    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                }
            }
        }
    }

    fn local_addr(&self) -> io::Result<Self::Addr> {
        self.0.local_addr()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(chunks: &[&[u8]]) -> Vec<u8> {
        let mut s = LenientStream::new(tokio::io::empty());
        for chunk in chunks {
            s.process(chunk);
        }
        s.out
    }

    #[test]
    fn encodes_only_what_the_uri_parser_rejects() {
        let out = run(&[b"GET /t?select=data->>id HTTP/1.1\r\n\r\n"]);
        assert_eq!(
            String::from_utf8(out).unwrap(),
            "GET /t?select=data-%3E%3Eid HTTP/1.1\r\n\r\n"
        );
    }

    #[test]
    fn encodes_quotes() {
        let out = run(&[b"GET /t?a=in.(\"x\") HTTP/1.1\r\n\r\n"]);
        assert!(String::from_utf8(out).unwrap().contains("in.(%22x%22)"));
    }

    #[test]
    fn leaves_an_ordinary_request_untouched() {
        let raw: &[u8] = b"GET /t?a=eq.1 HTTP/1.1\r\nHost: x\r\n\r\n";
        assert_eq!(run(&[raw]), raw.to_vec());
    }

    #[test]
    fn percent_escapes_already_present_are_not_re_encoded() {
        let raw: &[u8] = b"GET /t?select=data-%3E%3Eid HTTP/1.1\r\n\r\n";
        assert_eq!(run(&[raw]), raw.to_vec());
    }

    #[test]
    fn a_body_is_never_rewritten() {
        // The body is a request line character for character. It must survive.
        let body = b"GET /evil?x=a>b HTTP/1.1";
        let mut raw = Vec::new();
        raw.extend_from_slice(b"POST /t HTTP/1.1\r\nContent-Length: 24\r\n\r\n");
        raw.extend_from_slice(body);
        let out = run(&[&raw]);
        assert_eq!(out, raw, "body bytes were altered");
    }

    #[test]
    fn the_request_after_a_body_is_still_rewritten() {
        let mut raw = Vec::new();
        raw.extend_from_slice(b"POST /t HTTP/1.1\r\nContent-Length: 2\r\n\r\nhi");
        raw.extend_from_slice(b"GET /t?s=a->b HTTP/1.1\r\n\r\n");
        let out = String::from_utf8(run(&[&raw])).unwrap();
        assert!(out.contains("\r\n\r\nhi"), "body changed: {out}");
        assert!(
            out.contains("s=a-%3Eb"),
            "second request not rewritten: {out}"
        );
    }

    #[test]
    fn a_chunked_body_stops_rewriting_rather_than_guessing() {
        let mut raw = Vec::new();
        raw.extend_from_slice(b"POST /t HTTP/1.1\r\nTransfer-Encoding: chunked\r\n\r\n");
        raw.extend_from_slice(b"2\r\nhi\r\n0\r\n\r\nGET /t?s=a->b HTTP/1.1\r\n\r\n");
        // Passed through verbatim: not rewritten, but not corrupted either.
        assert_eq!(run(&[&raw]), raw);
    }

    #[test]
    fn a_request_split_across_reads_is_still_rewritten() {
        let out = run(&[b"GET /t?s=da", b"ta->>id HT", b"TP/1.1\r\n\r\n"]);
        assert_eq!(
            String::from_utf8(out).unwrap(),
            "GET /t?s=data-%3E%3Eid HTTP/1.1\r\n\r\n"
        );
    }

    #[test]
    fn keeps_pipelined_requests_aligned() {
        let out = String::from_utf8(run(&[
            b"GET /a?s=x->y HTTP/1.1\r\n\r\nGET /b?s=p->q HTTP/1.1\r\n\r\n",
        ]))
        .unwrap();
        assert!(out.contains("/a?s=x-%3Ey"), "{out}");
        assert!(out.contains("/b?s=p-%3Eq"), "{out}");
    }

    #[test]
    fn a_malformed_request_line_is_left_for_hyper_to_reject() {
        let raw: &[u8] = b"nonsense\r\n";
        assert_eq!(run(&[raw]), raw.to_vec());
    }

    /// HPACK sets the high bit constantly, so a frame read as a request line
    /// is full of bytes this would otherwise percent-encode.
    #[test]
    fn an_http2_connection_is_passed_through_untouched() {
        let mut raw = Vec::new();
        raw.extend_from_slice(H2_PREFACE);
        // A SETTINGS frame, then header-block bytes with the high bit set and
        // a `>` and a newline among them, which is what would be mangled.
        raw.extend_from_slice(b"\x00\x00\x00\x04\x00\x00\x00\x00\x00");
        raw.extend_from_slice(b"\x82\x86\x84\x41\x8a > \n\xff\xfe HTTP/1.1\r\n");
        assert_eq!(run(&[&raw]), raw);
    }

    /// The preface is decided on the bytes as they arrive, not on a whole
    /// read landing at once.
    #[test]
    fn an_http2_preface_split_across_reads_is_still_recognised() {
        let out = run(&[b"PRI * HTTP/2.0\r\n", b"\r\nSM\r\n\r\n", b"\xff\xfe->\n"]);
        assert_eq!(
            out,
            [H2_PREFACE, b"\xff\xfe->\n"].concat(),
            "an HTTP/2 connection was rewritten"
        );
    }

    /// Holding bytes back to check for the preface must not lose them.
    #[test]
    fn a_request_beginning_like_the_preface_is_read_as_http1() {
        let out = run(&[b"PRI /t?s=a->b HTTP/1.1\r\n\r\n"]);
        assert_eq!(
            String::from_utf8(out).unwrap(),
            "PRI /t?s=a-%3Eb HTTP/1.1\r\n\r\n"
        );
    }

    /// Including where it is split inside the shared prefix.
    #[test]
    fn a_request_beginning_like_the_preface_survives_being_split() {
        let out = run(&[b"P", b"OST /t?s=a->b HTTP/1.1\r\n", b"\r\n"]);
        assert_eq!(
            String::from_utf8(out).unwrap(),
            "POST /t?s=a-%3Eb HTTP/1.1\r\n\r\n"
        );
    }
}
