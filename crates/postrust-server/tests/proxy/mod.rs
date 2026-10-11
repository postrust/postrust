//! A TCP proxy in front of the test database, for making a connection do what
//! a healthy one never does on cue: wait, or break.
//!
//! Shared by the test files that need one (`mod proxy;`). Each server build
//! reaches the database differently -- the sqlx pool by default, a connection
//! per request with `pg-socket` -- and the proxy stands in front of both.

#![allow(dead_code)] // each test file uses part of it

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::watch;

/// What the proxy does to the connections it accepts.
#[derive(Clone, Copy)]
pub enum Behaviour {
    /// The first connection is held, unanswered, until [`Proxy::open`]; every
    /// other one is passed straight through.
    HoldTheFirst,
    /// Every connection is passed through until it has logged in, and closed
    /// when the client sends its first statement.
    CloseAtTheFirstStatement,
}

/// A proxy to `upstream`, listening on a port of its own.
pub struct Proxy {
    pub port: u16,
    accepted: Arc<AtomicUsize>,
    open: watch::Sender<bool>,
    task: tokio::task::JoinHandle<()>,
}

impl Proxy {
    pub async fn start(upstream: (String, u16), behaviour: Behaviour) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let accepted = Arc::new(AtomicUsize::new(0));
        let (open, opened) = watch::channel(false);
        let task = tokio::spawn({
            let accepted = accepted.clone();
            async move {
                while let Ok((client, _)) = listener.accept().await {
                    let first = accepted.fetch_add(1, Ordering::SeqCst) == 0;
                    let upstream = upstream.clone();
                    let mut opened = opened.clone();
                    tokio::spawn(async move {
                        if first && matches!(behaviour, Behaviour::HoldTheFirst) {
                            let _ = opened.wait_for(|open| *open).await;
                        }
                        let Ok(server) = TcpStream::connect(upstream).await else {
                            return;
                        };
                        match behaviour {
                            Behaviour::HoldTheFirst => pass(client, server).await,
                            Behaviour::CloseAtTheFirstStatement => {
                                cut_at_the_first_statement(client, server).await
                            }
                        }
                    });
                }
            }
        });
        Self {
            port,
            accepted,
            open,
            task,
        }
    }

    /// How many connections it has accepted.
    pub fn accepted(&self) -> usize {
        self.accepted.load(Ordering::SeqCst)
    }

    /// Let the held connection through.
    pub fn open(&self) {
        let _ = self.open.send(true);
    }

    /// `url` -- a `postgres://` URL -- with its host and port this proxy's.
    pub fn url(&self, url: &str) -> String {
        let (scheme, rest) = url.split_once("://").expect("a URL");
        let (login, rest) = match rest.split_once('@') {
            Some((login, rest)) => (format!("{login}@"), rest),
            None => (String::new(), rest),
        };
        let path = rest.find('/').map_or("", |at| &rest[at..]);
        format!("{scheme}://{login}127.0.0.1:{}{path}", self.port)
    }
}

impl Drop for Proxy {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// The host and port of a `postgres://` URL.
pub fn host_and_port(url: &str) -> (String, u16) {
    let rest = url.split_once("://").expect("a URL").1;
    let rest = rest.split_once('@').map_or(rest, |(_, rest)| rest);
    let authority = rest.split('/').next().unwrap_or(rest);
    match authority.rsplit_once(':') {
        Some((host, port)) => (host.to_string(), port.parse().expect("a port")),
        None => (authority.to_string(), 5432),
    }
}

async fn pass(mut client: TcpStream, mut server: TcpStream) {
    let _ = tokio::io::copy_bidirectional(&mut client, &mut server).await;
}

/// Pass the login through, and close both ends when the client sends a
/// statement: a `Query`, `Parse` or `Bind`.
async fn cut_at_the_first_statement(client: TcpStream, server: TcpStream) {
    let (mut from_client, mut to_client) = client.into_split();
    let (mut from_server, mut to_server) = server.into_split();
    let answers =
        tokio::spawn(async move { tokio::io::copy(&mut from_server, &mut to_client).await });

    let forward = async {
        // Untyped: an `SSLRequest` (which PostgreSQL answers on its own) or
        // the startup message.
        loop {
            let mut length = [0u8; 4];
            from_client.read_exact(&mut length).await?;
            let mut body = vec![0u8; (u32::from_be_bytes(length) as usize).saturating_sub(4)];
            from_client.read_exact(&mut body).await?;
            to_server.write_all(&length).await?;
            to_server.write_all(&body).await?;
            let code = body.get(..4).map(|c| u32::from_be_bytes(c.try_into().unwrap()));
            if !matches!(code, Some(80877103) | Some(80877104)) {
                break;
            }
        }
        // Typed, from here on.
        loop {
            let mut header = [0u8; 5];
            from_client.read_exact(&mut header).await?;
            if matches!(header[0], b'Q' | b'P' | b'B') {
                return Ok::<_, std::io::Error>(());
            }
            let length = u32::from_be_bytes(header[1..].try_into().unwrap()) as usize;
            let mut body = vec![0u8; length.saturating_sub(4)];
            from_client.read_exact(&mut body).await?;
            to_server.write_all(&header).await?;
            to_server.write_all(&body).await?;
        }
    };
    let _ = forward.await;
    answers.abort();
}
