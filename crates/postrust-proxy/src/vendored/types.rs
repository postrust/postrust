//! Vendored types from rpxy-lib: globals.rs, error.rs, name_exp.rs

use thiserror::Error;

/// Vendored error type (adapted from rpxy error.rs).
#[derive(Error, Debug)]
pub enum ProxyError {
    /// No backend is registered under this id.
    #[error("Backend not found: {0}")]
    BackendNotFound(String),

    /// A route named an upstream that is not in the config.
    #[error("Upstream not found: {0}")]
    UpstreamNotFound(String),

    /// The connection to the upstream could not be made or was lost.
    #[error("Connection error: {0}")]
    Connection(String),

    /// The request could not be rebuilt for the upstream.
    #[error("Request error: {0}")]
    Request(String),

    /// The upstream's response could not be read or relayed.
    #[error("Response error: {0}")]
    Response(String),

    /// The upstream did not answer in time.
    #[error("Timeout")]
    Timeout,

    /// From hyper, on either leg.
    #[error("Hyper error: {0}")]
    Hyper(#[from] hyper::Error),

    /// A request or response that cannot be represented -- an invalid header
    /// name, an unparseable URI.
    #[error("HTTP error: {0}")]
    Http(#[from] hyper::http::Error),

    /// From the socket.
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

/// Server name matching (adapted from rpxy name_exp.rs).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ServerName(
    /// The name, already lowercased by [`ServerName::new`].
    pub String,
);

impl ServerName {
    /// Wrap a host name, lowercasing it.
    ///
    /// Host matching is case-insensitive per RFC 9110 section 4.2.3, and
    /// normalising here means every comparison afterwards can be an equality
    /// check.
    pub fn new(name: impl Into<String>) -> Self {
        Self(name.into().to_lowercase())
    }

    /// Check if the server name matches the given host.
    pub fn matches(&self, host: &str) -> bool {
        let host = host.to_lowercase();

        // Exact match
        if self.0 == host {
            return true;
        }

        // Wildcard match: `*.example.com` matches exactly one label
        // (e.g. `sub.example.com`) but not `example.com` or the multi-level
        // `sub.sub.example.com`, per RFC 6125 wildcard semantics.
        if self.0.starts_with("*.") {
            let suffix = &self.0[2..];
            if host.ends_with(suffix) {
                let prefix_len = host.len() - suffix.len();
                // The prefix must be a single non-empty label followed by a dot,
                // with no interior dots of its own.
                if prefix_len > 0
                    && host.chars().nth(prefix_len - 1) == Some('.')
                    && !host[..prefix_len - 1].contains('.')
                {
                    return true;
                }
            }
        }

        false
    }
}

impl From<&str> for ServerName {
    fn from(s: &str) -> Self {
        Self::new(s)
    }
}

impl From<String> for ServerName {
    fn from(s: String) -> Self {
        Self::new(s)
    }
}

/// Path name matching with longest-prefix support (adapted from rpxy name_exp.rs).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct PathName(
    /// The path, guaranteed by [`PathName::new`] to start with `/`.
    pub String,
);

impl PathName {
    /// Wrap a path, adding a leading `/` if it is missing.
    ///
    /// Without that, a pattern written as `v1` would never match a request
    /// path, which always starts with `/`.
    pub fn new(path: impl Into<String>) -> Self {
        let mut path = path.into();
        // Ensure path starts with /
        if !path.starts_with('/') {
            path = format!("/{}", path);
        }
        Self(path)
    }

    /// Check if this path matches the given request path.
    pub fn matches(&self, request_path: &str) -> bool {
        request_path.starts_with(&self.0)
    }

    /// Get the length of this path (for longest-prefix matching).
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Whether the underlying path string is empty.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Check if the path is empty (just "/").
    pub fn is_root(&self) -> bool {
        self.0 == "/"
    }
}

impl From<&str> for PathName {
    fn from(s: &str) -> Self {
        Self::new(s)
    }
}

impl From<String> for PathName {
    fn from(s: String) -> Self {
        Self::new(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_server_name_exact_match() {
        let name = ServerName::new("example.com");
        assert!(name.matches("example.com"));
        assert!(name.matches("EXAMPLE.COM"));
        assert!(!name.matches("sub.example.com"));
        assert!(!name.matches("example.org"));
    }

    #[test]
    fn test_server_name_wildcard() {
        let name = ServerName::new("*.example.com");
        assert!(name.matches("sub.example.com"));
        assert!(name.matches("api.example.com"));
        assert!(!name.matches("example.com"));
        assert!(!name.matches("sub.sub.example.com")); // Multi-level shouldn't match single wildcard
    }

    #[test]
    fn test_path_name_matching() {
        let path = PathName::new("/api");
        assert!(path.matches("/api"));
        assert!(path.matches("/api/users"));
        assert!(path.matches("/api/users/123"));
        assert!(!path.matches("/other"));
        assert!(!path.matches("/"));
    }

    #[test]
    fn test_path_name_root() {
        let path = PathName::new("/");
        assert!(path.matches("/"));
        assert!(path.matches("/anything"));
        assert!(path.is_root());
    }
}
