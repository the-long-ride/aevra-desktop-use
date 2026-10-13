//! Streamable HTTP on loopback only, behind a bearer token. A guard runs in
//! front of rmcp's service and rejects, in this order: a `Host` that is not
//! this loopback listener (DNS rebinding), a browser `Origin` that is not
//! loopback, and a missing or wrong token. Only `GET /healthz` skips the token.

use crate::handlers::Server;
use crate::rmcp_server::Connection;
use axum::extract::{Request, State};
use axum::http::{header, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use desktop_policy::HttpConfig;
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::streamable_http_server::{StreamableHttpServerConfig, StreamableHttpService};
use serde_json::json;
use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;
use subtle::ConstantTimeEq;
use tokio_util::sync::CancellationToken;

#[derive(Debug)]
pub enum HttpError {
    /// Another process holds the port.
    PortInUse(u16),
    Bind(std::io::Error),
    /// The server will not start without a token.
    MissingToken,
}

impl std::fmt::Display for HttpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::PortInUse(port) => write!(f, "HTTP_PORT_IN_USE: {port}"),
            Self::Bind(e) => write!(f, "HTTP_BIND_FAILED: {e}"),
            Self::MissingToken => {
                write!(f, "HTTP_TOKEN_MISSING: refusing to serve without a token")
            }
        }
    }
}

impl std::error::Error for HttpError {}

struct Guard {
    port: u16,
    token: Vec<u8>,
}

impl Guard {
    fn host_ok(&self, host: &str) -> bool {
        let host = host.to_ascii_lowercase();
        host == format!("127.0.0.1:{}", self.port) || host == format!("localhost:{}", self.port)
    }

    fn token_ok(&self, header_value: &str) -> bool {
        let Some(presented) = header_value.strip_prefix("Bearer ") else {
            return false;
        };
        presented.as_bytes().ct_eq(&self.token).into()
    }
}

/// `http://127.0.0.1[:port]` or `http://localhost[:port]`, nothing more.
fn origin_ok(origin: &str) -> bool {
    let Some(rest) = origin.strip_prefix("http://") else {
        return false;
    };
    let (host, port) = match rest.split_once(':') {
        Some((host, port)) => (host, Some(port)),
        None => (rest, None),
    };
    (host == "127.0.0.1" || host == "localhost")
        && port.is_none_or(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
}

fn refuse(status: StatusCode, message: &str) -> Response {
    (status, Json(json!({"error": message}))).into_response()
}

async fn guard(State(guard): State<Arc<Guard>>, request: Request, next: Next) -> Response {
    let headers = request.headers();
    let host = headers.get(header::HOST).and_then(|v| v.to_str().ok());
    if !host.is_some_and(|h| guard.host_ok(h)) {
        return refuse(StatusCode::FORBIDDEN, "host not allowed");
    }
    if let Some(origin) = headers.get(header::ORIGIN) {
        if !origin.to_str().is_ok_and(origin_ok) {
            return refuse(StatusCode::FORBIDDEN, "origin not allowed");
        }
    }
    if request.uri().path() != "/healthz" {
        let authorized = headers
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| guard.token_ok(v));
        if !authorized {
            return refuse(StatusCode::UNAUTHORIZED, "missing or invalid bearer token");
        }
    }
    next.run(request).await
}

/// Binds `127.0.0.1:<port>` (port 0 picks a free one) and serves until
/// `shutdown` is cancelled. Returns the bound address.
pub async fn serve_http(
    server: Arc<Server>,
    config: HttpConfig,
    shutdown: CancellationToken,
) -> Result<SocketAddr, HttpError> {
    if config.token.is_empty() {
        return Err(HttpError::MissingToken);
    }
    let listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, config.port))
        .await
        .map_err(|e| match e.kind() {
            std::io::ErrorKind::AddrInUse => HttpError::PortInUse(config.port),
            _ => HttpError::Bind(e),
        })?;
    let addr = listener.local_addr().map_err(HttpError::Bind)?;

    let mcp = StreamableHttpService::new(
        move || Ok(Connection::new(Arc::clone(&server))),
        Arc::new(LocalSessionManager::default()),
        StreamableHttpServerConfig::default().with_cancellation_token(shutdown.child_token()),
    );
    let guard_state = Arc::new(Guard {
        port: addr.port(),
        token: config.token.into_bytes(),
    });
    let app = Router::new()
        .route("/healthz", get(|| async { Json(json!({"ok": true})) }))
        .nest_service("/mcp", mcp)
        .layer(middleware::from_fn_with_state(guard_state, guard));

    tokio::spawn(async move {
        let _ = axum::serve(listener, app)
            .with_graceful_shutdown(async move { shutdown.cancelled().await })
            .await;
    });
    Ok(addr)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn origins_must_be_plain_loopback() {
        assert!(origin_ok("http://127.0.0.1:3000"));
        assert!(origin_ok("http://localhost"));
        assert!(origin_ok("http://localhost:47851"));
        assert!(!origin_ok("https://localhost:3000"));
        assert!(!origin_ok("http://127.0.0.1.evil.test"));
        assert!(!origin_ok("http://evil.test"));
        assert!(!origin_ok("http://localhost:abc"));
        assert!(!origin_ok("null"));
    }

    #[test]
    fn token_compare_needs_the_bearer_prefix_and_the_exact_token() {
        let g = Guard {
            port: 1,
            token: b"abc".to_vec(),
        };
        assert!(g.token_ok("Bearer abc"));
        assert!(!g.token_ok("Bearer abd"));
        assert!(!g.token_ok("Bearer abcd"));
        assert!(!g.token_ok("abc"));
        assert!(!g.token_ok("bearer abc"));
    }

    #[test]
    fn hosts_must_carry_the_bound_port() {
        let g = Guard {
            port: 4000,
            token: vec![1],
        };
        assert!(g.host_ok("127.0.0.1:4000"));
        assert!(g.host_ok("LOCALHOST:4000"));
        assert!(!g.host_ok("127.0.0.1:4001"));
        assert!(!g.host_ok("evil.test"));
        assert!(!g.host_ok("127.0.0.1"));
    }
}
