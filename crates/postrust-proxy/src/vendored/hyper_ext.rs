//! Vendored Hyper extensions from rpxy-lib: hyper_ext/*
//!
//! This module provides body types and utilities for working with Hyper.

use bytes::Bytes;
use http_body_util::{combinators::BoxBody, BodyExt, Empty, Full};

/// Boxed body type used throughout the proxy.
pub type ProxyBody = BoxBody<Bytes, hyper::Error>;

/// Create an empty body.
pub fn empty_body() -> ProxyBody {
    Empty::<Bytes>::new()
        .map_err(|never| match never {})
        .boxed()
}

/// Create a body from bytes.
pub fn full_body(chunk: impl Into<Bytes>) -> ProxyBody {
    Full::new(chunk.into())
        .map_err(|never| match never {})
        .boxed()
}

/// Create a body from a string.
pub fn string_body(s: impl Into<String>) -> ProxyBody {
    full_body(s.into().into_bytes())
}

/// Extension trait for incoming bodies.
pub trait IncomingBodyExt {
    /// Convert to a boxed body.
    fn boxed_body(self) -> ProxyBody;
}

impl IncomingBodyExt for hyper::body::Incoming {
    fn boxed_body(self) -> ProxyBody {
        self.map_err(|e| e).boxed()
    }
}

/// Timer utilities for request timeouts.
pub mod timer {
    use pin_project_lite::pin_project;
    use std::future::Future;
    use std::pin::Pin;
    use std::task::{Context, Poll};
    use std::time::Duration;
    use tokio::time::{sleep, Sleep};

    pin_project! {
        /// A future that resolves after a timeout or when the inner future completes.
        pub struct Timeout<T> {
            #[pin]
            value: T,
            #[pin]
            delay: Sleep,
        }
    }

    impl<T> Timeout<T> {
        pub fn new(value: T, timeout: Duration) -> Self {
            Self {
                value,
                delay: sleep(timeout),
            }
        }
    }

    impl<T: Future> Future for Timeout<T> {
        type Output = Result<T::Output, TimeoutError>;

        fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
            let this = self.project();

            // First, check if the inner future is ready
            if let Poll::Ready(v) = this.value.poll(cx) {
                return Poll::Ready(Ok(v));
            }

            // Then check the timeout
            if this.delay.poll(cx).is_ready() {
                return Poll::Ready(Err(TimeoutError));
            }

            Poll::Pending
        }
    }

    /// Error returned when a timeout expires.
    #[derive(Debug, Clone, Copy)]
    pub struct TimeoutError;

    impl std::fmt::Display for TimeoutError {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "operation timed out")
        }
    }

    impl std::error::Error for TimeoutError {}
}
