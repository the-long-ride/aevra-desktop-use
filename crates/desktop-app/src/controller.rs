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
}

impl HttpManager {
    pub fn new(server: Arc<Server>, runtime: Handle) -> Self {
        Self {
            server,
            runtime,
            inner: Mutex::new(Inner {
                applied: None,
                shutdown: None,
                status: HttpStatus::Stopped,
            }),
        }
    }

    pub fn status(&self) -> HttpStatus {
        self.inner.lock().unwrap().status.clone()
    }

    /// Makes the listener match `wanted`. A no-op when it already does. Blocks
    /// briefly, so call it from a plain thread, not from async code.
    pub fn reconcile(&self, wanted: &HttpConfig) {
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
pub fn apply(server: &Arc<Server>, http: &HttpManager, runtime: &Handle, config: Config) {
    let http_config = config.http.clone();
    runtime.block_on(server.apply_config(config));
    http.reconcile(&http_config);
}

#[cfg(test)]
mod tests {
    use super::*;
    use desktop_core::{
        ActRequest, BackendHandle, Capabilities, CaptureRequest, CaptureResult, DescribeRequest,
        DescribeResult, DesktopBackend, ScreenState, WindowIdentity,
    };
    use desktop_mcp::apps::EmptyCatalog;
    use desktop_policy::activity::ActivityLog;
    use std::net::TcpStream;

    struct NoBackend;

    impl DesktopBackend for NoBackend {
        fn connect(&self) -> Result<Capabilities, String> {
            Err("not used".into())
        }
        fn windows(&self) -> Result<Vec<WindowIdentity>, String> {
            Err("not used".into())
        }
        fn focused_window(&self) -> Result<WindowIdentity, String> {
            Err("not used".into())
        }
        fn screen_state(&self) -> Result<ScreenState, String> {
            Err("not used".into())
        }
        fn describe(&self, _: DescribeRequest) -> Result<DescribeResult, String> {
            Err("not used".into())
        }
        fn act(&self, _: ActRequest) -> Result<bool, String> {
            Err("not used".into())
        }
        fn capture(&self, _: CaptureRequest) -> Result<CaptureResult, String> {
            Err("not used".into())
        }
    }

    fn manager(runtime: &tokio::runtime::Runtime) -> (HttpManager, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let server = Arc::new(Server::new(
            BackendHandle::spawn(|| NoBackend),
            Config::default(),
            ActivityLog::new(dir.path()),
            Arc::new(EmptyCatalog),
        ));
        (HttpManager::new(server, runtime.handle().clone()), dir)
    }

    fn http(port: u16, token: &str) -> HttpConfig {
        HttpConfig {
            enabled: true,
            port,
            token: token.to_string(),
        }
    }

    fn runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .unwrap()
    }

    fn listening(addr: SocketAddr) -> bool {
        TcpStream::connect_timeout(&addr, Duration::from_millis(300)).is_ok()
    }

    #[test]
    fn enabling_starts_the_listener_and_disabling_stops_it() {
        let runtime = runtime();
        let (manager, _dir) = manager(&runtime);
        assert_eq!(manager.status(), HttpStatus::Stopped);

        manager.reconcile(&http(0, "placeholder-token"));
        let HttpStatus::Running(addr) = manager.status() else {
            panic!("expected running, got {:?}", manager.status());
        };
        assert!(listening(addr));

        manager.reconcile(&HttpConfig {
            enabled: false,
            ..http(0, "placeholder-token")
        });
        assert_eq!(manager.status(), HttpStatus::Stopped);
        let mut closed = false;
        for _ in 0..40 {
            if !listening(addr) {
                closed = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        assert!(closed, "listener still accepting after stop");
    }

    #[test]
    fn an_unchanged_config_does_not_restart_the_listener() {
        let runtime = runtime();
        let (manager, _dir) = manager(&runtime);
        let config = http(0, "placeholder-token");
        manager.reconcile(&config);
        let before = manager.status();
        manager.reconcile(&config);
        assert_eq!(manager.status(), before);
    }

    #[test]
    fn a_changed_token_restarts_the_listener() {
        let runtime = runtime();
        let (manager, _dir) = manager(&runtime);
        manager.reconcile(&http(0, "first-placeholder-token"));
        let HttpStatus::Running(first) = manager.status() else {
            panic!("not running");
        };
        manager.reconcile(&http(0, "second-placeholder-token"));
        let HttpStatus::Running(second) = manager.status() else {
            panic!("not running after restart");
        };
        // Port 0 picks a fresh port, so a restart is visible as a new address.
        assert_ne!(first, second);
        assert!(listening(second));
    }

    #[test]
    fn a_taken_port_is_reported_as_failed() {
        let runtime = runtime();
        let (manager, _dir) = manager(&runtime);
        let held = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = held.local_addr().unwrap().port();
        manager.reconcile(&http(port, "placeholder-token"));
        assert_eq!(
            manager.status(),
            HttpStatus::Failed(format!("HTTP_PORT_IN_USE: {port}"))
        );
        // Once the port is free, the next reconcile recovers.
        drop(held);
        manager.reconcile(&http(port, "placeholder-token"));
        assert!(matches!(manager.status(), HttpStatus::Running(_)));
    }

    #[test]
    fn an_empty_token_is_a_failure_not_an_open_listener() {
        let runtime = runtime();
        let (manager, _dir) = manager(&runtime);
        manager.reconcile(&http(0, ""));
        assert!(matches!(manager.status(), HttpStatus::Failed(_)));
    }
}
