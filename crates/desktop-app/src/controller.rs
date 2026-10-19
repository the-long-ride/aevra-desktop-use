//! Applies config changes to the running server: the allowlist through
//! [`Server::apply_config`] and the HTTP listener through [`HttpManager`].

use desktop_mcp::{serve_http, HttpError, Server};
use desktop_policy::{Config, HttpConfig};
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::runtime::Handle;
use tokio_util::sync::CancellationToken;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HttpStatus {
    Stopped,
    Running(SocketAddr),
    /// Why the listener could not start, e.g. `HTTP_PORT_IN_USE: 47851`.
    Failed(String),
}

struct Inner {
    applied: Option<HttpConfig>,
    shutdown: Option<CancellationToken>,
    status: HttpStatus,
}

/// Starts, restarts and stops the loopback HTTP listener to match the config.
pub struct HttpManager {
    server: Arc<Server>,
    runtime: Handle,
    inner: Mutex<Inner>,
    port_override: Option<u16>,
}

impl HttpManager {
    pub fn new(server: Arc<Server>, runtime: Handle) -> Self {
        Self::with_port_override(server, runtime, None)
    }

    pub fn with_port_override(server: Arc<Server>, runtime: Handle, port: Option<u16>) -> Self {
        Self {
            server,
            runtime,
            port_override: port,
            inner: Mutex::new(Inner {
                applied: None,
                shutdown: None,
                status: HttpStatus::Stopped,
            }),
        }
    }

    pub fn port_override(&self) -> Option<u16> {
        self.port_override
    }

    /// The active port; persisted configuration stays independent of startup flags.
    pub fn effective_port(&self, configured: u16) -> u16 {
        self.port_override.unwrap_or(configured)
    }

    pub fn status(&self) -> HttpStatus {
        self.inner.lock().unwrap().status.clone()
    }

    /// Makes the listener match `wanted`. A no-op when it already does. Blocks
    /// briefly, so call it from a plain thread, not from async code.
    pub fn reconcile(&self, wanted: &HttpConfig) {
        let wanted = &HttpConfig {
            port: self.effective_port(wanted.port),
            ..wanted.clone()
        };
        let mut inner = self.inner.lock().unwrap();
        if !wanted.enabled {
            Self::stop_locked(&mut inner);
            return;
        }
        let unchanged = inner.applied.as_ref() == Some(wanted)
            && matches!(inner.status, HttpStatus::Running(_));
        if unchanged {
            return;
        }
        let restarting = inner.shutdown.is_some();
        Self::stop_locked(&mut inner);

        // The old listener releases its port asynchronously; give it a moment.
        let attempts = if restarting { 20 } else { 1 };
        let mut attempt = 0;
        let outcome = loop {
            let shutdown = CancellationToken::new();
            let result = self
                .runtime
                .block_on(serve_http(
                    Arc::clone(&self.server),
                    wanted.clone(),
                    shutdown.clone(),
                ))
                .map(|addr| (addr, shutdown));
            attempt += 1;
            if !matches!(result, Err(HttpError::PortInUse(_))) || attempt >= attempts {
                break result;
            }
            std::thread::sleep(Duration::from_millis(100));
        };
        inner.applied = Some(wanted.clone());
        match outcome {
            Ok((addr, shutdown)) => {
                inner.shutdown = Some(shutdown);
                inner.status = HttpStatus::Running(addr);
            }
            Err(error) => {
                inner.shutdown = None;
                inner.status = HttpStatus::Failed(error.to_string());
            }
        }
    }

    pub fn stop(&self) {
        Self::stop_locked(&mut self.inner.lock().unwrap());
    }

    fn stop_locked(inner: &mut Inner) {
        if let Some(shutdown) = inner.shutdown.take() {
            shutdown.cancel();
        }
        inner.applied = None;
        inner.status = HttpStatus::Stopped;
    }
}

/// Applies a freshly loaded config: the allowlist first, then HTTP. Run from a
/// plain thread (the file watcher's), never from async code.
pub fn apply(server: &Arc<Server>, http: &HttpManager, runtime: &Handle, mut config: Config) {
    config.http.port = http.effective_port(config.http.port);
    let http_config = config.http.clone();
    runtime.block_on(server.apply_config(config));
    http.reconcile(&http_config);
}

#[cfg(test)]
#[path = "controller_tests.rs"]
mod tests;
