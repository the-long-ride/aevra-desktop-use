//! Startup port behavior through the real executable and isolated config files.

use desktop_app::single_instance::{acquire, Acquired};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::process::Command;
use std::time::{Duration, Instant};

const BIN: &str = env!("CARGO_BIN_EXE_aevra-desktop");

#[test]
fn a_custom_port_cannot_be_silently_ignored_by_an_existing_gui() {
    let home = tempfile::tempdir().unwrap();
    let Acquired::First(_instance) = acquire(home.path()).unwrap() else {
        panic!("expected first instance");
    };
    let output = Command::new(BIN)
        .env("AEVRA_DESKTOP_HOME", home.path())
        .args(["--p=48900"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    let error = String::from_utf8_lossy(&output.stderr);
    assert!(error.contains("already running"), "{error}");
    assert!(error.contains("quit"), "{error}");
    assert!(error.contains("48900"), "{error}");
    assert!(!home.path().join("config.json").exists());
}

#[test]
fn bad_startup_ports_fail_before_creating_any_config() {
    for args in [
        vec!["--port"],
        vec!["--p=0"],
        vec!["--port=65536"],
        vec!["--port=1.5"],
        vec!["--p=-1"],
        vec!["--port=48900", "--p", "48901"],
        vec!["mcp", "--port=48900"],
    ] {
        let home = tempfile::tempdir().unwrap();
        let output = Command::new(BIN)
            .env("AEVRA_DESKTOP_HOME", home.path())
            .args(&args)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(2), "{args:?}");
        assert_eq!(std::fs::read_dir(home.path()).unwrap().count(), 0);
    }
}

struct GuiChild(std::process::Child);

impl Drop for GuiChild {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn response(port: u16, path: &str, token: &str) -> String {
    let addr = ([127, 0, 0, 1], port).into();
    let Ok(mut stream) = TcpStream::connect_timeout(&addr, Duration::from_millis(100)) else {
        return String::new();
    };
    stream
        .set_read_timeout(Some(Duration::from_millis(200)))
        .unwrap();
    let _ = write!(stream, "GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nAuthorization: Bearer {token}\r\nConnection: close\r\n\r\n");
    let mut response = String::new();
    let _ = stream.read_to_string(&mut response);
    response
}

fn wait_for(state: &str, mut ready: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !ready() {
        assert!(Instant::now() < deadline, "timed out waiting for {state}");
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
#[ignore = "requires an interactive native desktop; opens the GUI using its exit-after hook"]
fn native_gui_uses_the_requested_port_through_reload_without_persisting_it() {
    use desktop_policy::config::{load, save};
    let home = tempfile::tempdir().unwrap();
    let path = home.path().join("config.json");
    // Reserve distinct ports before launch, so configured and wake-up ports
    // cannot accidentally satisfy the custom endpoint assertion.
    let configured = TcpListener::bind("127.0.0.1:0").unwrap();
    let selected = TcpListener::bind("127.0.0.1:0").unwrap();
    let custom_port = selected.local_addr().unwrap().port();
    let mut config = desktop_policy::Config::default();
    config.http.enabled = true;
    config.http.port = configured.local_addr().unwrap().port();
    config.http.token = "first-startup-test-token".into();
    save(&path, &config).unwrap();
    drop(selected);
    let mut launch = Command::new(BIN);
    launch
        .env("AEVRA_DESKTOP_HOME", home.path())
        .env("AEVRA_DESKTOP_EXIT_AFTER_MS", "30000")
        .arg(format!("--port={custom_port}"));
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        launch.creation_flags(0x08000000); // CREATE_NO_WINDOW: no console helper window.
    }
    let mut child = GuiChild(launch.spawn().unwrap());
    wait_for("HTTP startup on selected port", || {
        assert!(
            child.0.try_wait().unwrap().is_none(),
            "GUI exited before ready"
        );
        response(custom_port, "/healthz", "").contains("200 OK")
    });
    config.http.enabled = false;
    save(&path, &config).unwrap();
    wait_for("HTTP disabled by settings", || {
        response(custom_port, "/healthz", "").is_empty()
    });
    let wake_port: u16 = std::fs::read_to_string(home.path().join("ui.port"))
        .unwrap()
        .parse()
        .unwrap();
    assert_ne!(wake_port, custom_port);
    assert_eq!(load(&path).unwrap().http.port, config.http.port);

    config.http.token = "second-startup-test-token".into();
    config.http.enabled = true;
    let edited = TcpListener::bind("127.0.0.1:0").unwrap();
    config.http.port = edited.local_addr().unwrap().port();
    save(&path, &config).unwrap();
    wait_for("HTTP reenabled with rotated token", || {
        response(custom_port, "/mcp", "first-startup-test-token").contains("401 Unauthorized")
    });
    assert!(response(custom_port, "/healthz", "").contains("200 OK"));
    assert_eq!(load(&path).unwrap().http.port, config.http.port);
    config.http.enabled = false;
    save(&path, &config).unwrap();
    wait_for("HTTP stopped", || {
        response(custom_port, "/healthz", "").is_empty()
    });
}
