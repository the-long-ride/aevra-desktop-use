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

#[test]
fn startup_override_survives_config_changes_without_changing_saved_settings() {
    let runtime = runtime();
    let (default_manager, dir) = manager(&runtime);
    let server = Arc::clone(&default_manager.server);
    let selected = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = selected.local_addr().unwrap().port();
    let manager =
        HttpManager::with_port_override(Arc::clone(&server), runtime.handle().clone(), Some(port));
    drop(selected);
    let path = dir.path().join("config.json");
    let mut saved = Config {
        http: http(47901, "first-test-token"),
        ..Config::default()
    };
    desktop_policy::config::save(&path, &saved).unwrap();
    apply(&server, &manager, runtime.handle(), saved.clone());
    let expected = SocketAddr::from(([127, 0, 0, 1], port));
    assert_eq!(manager.status(), HttpStatus::Running(expected));
    assert!(listening(expected));
    assert_eq!(server.config().http.port, port);

    // Applying persisted edits must keep the effective server port separate,
    // even when a port edit and token rotation trigger a listener restart.
    saved.http.port = 47902;
    saved.http.token = "second-test-token".into();
    desktop_policy::config::save(&path, &saved).unwrap();
    apply(&server, &manager, runtime.handle(), saved.clone());
    assert_eq!(manager.status(), HttpStatus::Running(expected));
    assert_eq!(server.config().http.port, port);
    assert_eq!(server.config().http.token, "second-test-token");
    assert_eq!(
        desktop_policy::config::load(&path).unwrap().http.port,
        47902
    );
    let snippet: serde_json::Value = serde_json::from_str(&crate::ui::settings::http_snippet(
        manager.effective_port(saved.http.port),
        &saved.http.token,
    ))
    .unwrap();
    assert_eq!(
        snippet["mcpServers"]["aevra-desktop"]["url"],
        format!("http://127.0.0.1:{port}/mcp")
    );

    saved.http.enabled = false;
    apply(&server, &manager, runtime.handle(), saved);
    assert_eq!(manager.status(), HttpStatus::Stopped);
    manager.stop();
}
