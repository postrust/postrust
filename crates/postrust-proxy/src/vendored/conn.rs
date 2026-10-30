//! Per-connection bookkeeping for the listeners: the idle timeout, and the
//! connection slot each one holds.
//!
//! Not from rpxy. hyper bounds how long an HTTP/1 request head may take to
//! arrive, but nothing else: a connection that never sends a byte, a TLS
//! handshake that never finishes, and an HTTP/2 connection with no streams
//! open all stay up for as long as the client likes. Enough of those and the
//! proxy runs out of sockets. [`IdleIo`] closes a connection that has had no
//! request in flight for the idle timeout, whatever protocol it speaks.
//!
//! "Idle" means no request in flight, not no bytes moving. An HTTP/2
//! keep-alive PING is traffic, and counting it would keep an otherwise unused
//! connection open forever; and a request waiting on a slow upstream moves no
//! bytes but is not idle, and is bounded by the route's own timeout.

use futures::task::AtomicWaker;
use parking_lot::Mutex;
use std::future::Future;
use std::io;
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::sync::OwnedSemaphorePermit;
use tokio::time::{Instant, Sleep};

/// What one client connection is doing, shared between its socket and the
/// requests being served on it.
pub(crate) struct ConnActivity {
    in_flight: AtomicUsize,
    /// When the last request finished, or the connection was accepted.
    idle_since: Mutex<Instant>,
    /// The socket's task, woken when the connection goes idle so that it
    /// starts its idle timer even if it is parked waiting to read.
    waker: AtomicWaker,
    /// The connection's slot under `server.max_connections`. Held here rather
    /// than by the task that accepted the connection, because an upgraded
    /// connection outlives that task: hyper hands the socket to the tunnel,
    /// and the socket -- this wrapper -- is what has to keep the slot.
    _slot: Option<OwnedSemaphorePermit>,
}

impl ConnActivity {
    pub(crate) fn new(slot: Option<OwnedSemaphorePermit>) -> Arc<Self> {
        Arc::new(Self {
            in_flight: AtomicUsize::new(0),
            idle_since: Mutex::new(Instant::now()),
            waker: AtomicWaker::new(),
            _slot: slot,
        })
    }
}

/// A request in flight on a connection. While any exists, the connection is
/// not idle.
///
/// Held by the request future and then by the response body, so it lasts
/// until the response has been sent; and by a tunnel, which is how a
/// WebSocket that is quiet between messages escapes the connection's idle
/// timeout and is governed by the tunnel's own instead.
pub struct InFlight {
    activity: Arc<ConnActivity>,
}

impl InFlight {
    pub(crate) fn new(activity: &Arc<ConnActivity>) -> Self {
        activity.in_flight.fetch_add(1, Ordering::AcqRel);
        Self {
            activity: activity.clone(),
        }
    }
}

impl Clone for InFlight {
    fn clone(&self) -> Self {
        Self::new(&self.activity)
    }
}

impl Drop for InFlight {
    fn drop(&mut self) {
        // Set the time before the count reaches zero, so a socket that sees
        // zero never reads a stale start for its idle period.
        *self.activity.idle_since.lock() = Instant::now();
        if self.activity.in_flight.fetch_sub(1, Ordering::AcqRel) == 1 {
            self.activity.waker.wake();
        }
    }
}

impl std::fmt::Debug for InFlight {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InFlight").finish_non_exhaustive()
    }
}

/// A client socket that fails with `TimedOut` once its connection has had no
/// request in flight for `timeout`.
///
/// Failing a read or write is how the connection is closed: hyper, or the
/// TLS acceptor, treats the error as the end of the connection.
pub(crate) struct IdleIo<T> {
    inner: T,
    activity: Arc<ConnActivity>,
    timeout: Duration,
    sleep: Pin<Box<Sleep>>,
}

impl<T> IdleIo<T> {
    pub(crate) fn new(inner: T, activity: Arc<ConnActivity>, timeout: Duration) -> Self {
        let deadline = *activity.idle_since.lock() + timeout;
        Self {
            inner,
            activity,
            timeout,
            sleep: Box::pin(tokio::time::sleep_until(deadline)),
        }
    }

    fn poll_idle(&mut self, cx: &mut Context<'_>) -> io::Result<()> {
        self.activity.waker.register(cx.waker());
        if self.activity.in_flight.load(Ordering::Acquire) > 0 {
            return Ok(());
        }
        let deadline = *self.activity.idle_since.lock() + self.timeout;
        if self.sleep.deadline() != deadline {
            self.sleep.as_mut().reset(deadline);
        }
        match self.sleep.as_mut().poll(cx) {
            Poll::Ready(()) => Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "connection idle for longer than server.idle_timeout_secs",
            )),
            Poll::Pending => Ok(()),
        }
    }
}

impl<T: AsyncRead + Unpin> AsyncRead for IdleIo<T> {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        this.poll_idle(cx)?;
        Pin::new(&mut this.inner).poll_read(cx, buf)
    }
}

impl<T: AsyncWrite + Unpin> AsyncWrite for IdleIo<T> {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        let this = self.get_mut();
        this.poll_idle(cx)?;
        Pin::new(&mut this.inner).poll_write(cx, buf)
    }

    fn poll_write_vectored(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &[io::IoSlice<'_>],
    ) -> Poll<io::Result<usize>> {
        let this = self.get_mut();
        this.poll_idle(cx)?;
        Pin::new(&mut this.inner).poll_write_vectored(cx, bufs)
    }

    fn is_write_vectored(&self) -> bool {
        self.inner.is_write_vectored()
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_flush(cx)
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_shutdown(cx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[tokio::test(start_paused = true)]
    async fn an_idle_connection_times_out() {
        let (client, server) = tokio::io::duplex(64);
        let activity = ConnActivity::new(None);
        let mut io = IdleIo::new(server, activity, Duration::from_secs(5));

        let mut buf = [0u8; 8];
        let err = io.read(&mut buf).await.unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::TimedOut);
        drop(client);
    }

    #[tokio::test(start_paused = true)]
    async fn a_request_in_flight_holds_it_open_and_its_end_restarts_the_clock() {
        let (mut client, server) = tokio::io::duplex(64);
        let activity = ConnActivity::new(None);
        let mut io = IdleIo::new(server, activity.clone(), Duration::from_secs(5));

        let request = InFlight::new(&activity);
        let reader = tokio::spawn(async move {
            let mut buf = [0u8; 8];
            let started = Instant::now();
            let n = io.read(&mut buf).await.unwrap();
            assert_eq!(n, 1);
            let err = io.read(&mut buf).await.unwrap_err();
            (started.elapsed(), err.kind())
        });

        // Far longer than the timeout, with a request open: still up.
        tokio::time::sleep(Duration::from_secs(60)).await;
        client.write_all(b"x").await.unwrap();
        tokio::time::sleep(Duration::from_secs(1)).await;
        drop(request);

        let (elapsed, kind) = reader.await.unwrap();
        assert_eq!(kind, io::ErrorKind::TimedOut);
        // Closed five seconds after the request ended, not after it began.
        assert!(elapsed >= Duration::from_secs(66), "{elapsed:?}");
        assert!(elapsed < Duration::from_secs(68), "{elapsed:?}");
    }
}
