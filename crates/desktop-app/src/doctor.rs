//! `aevra-desktop doctor`: what works on this machine and what does not.

use desktop_core::{actor, Call, Reply};
use desktop_policy::config::{load, validate};
use serde::Serialize;
use serde_json::Value;
use std::net::{Ipv4Addr, TcpListener};
use std::path::Path;
use std::time::Duration;

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    pub version: String,
    pub platform: String,
    pub config_path: String,
    pub config_valid: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub config_problem: Option<String>,
    pub allowed_apps: usize,
    pub http: HttpReport,
    pub backend: BackendReport,
    /// macOS only: whether the Accessibility permission lets the backend connect.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub accessibility_trusted: Option<bool>,
    /// Linux only: whether the AT-SPI bus is reachable.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub atspi_reachable: Option<bool>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct HttpReport {
    pub enabled: bool,
    pub port: u16,
    pub port_free: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BackendReport {
    pub connected: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub capabilities: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<BackendError>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct BackendError {
    pub code: String,
    pub message: String,
}

pub fn port_free(port: u16) -> bool {
    TcpListener::bind((Ipv4Addr::LOCALHOST, port)).is_ok()
}

/// Asks the native backend to connect, with a deadline.
pub fn probe_backend(timeout: Duration) -> BackendReport {
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
    {
        Ok(runtime) => runtime,
        Err(e) => {
            return BackendReport {
                connected: false,
                capabilities: None,
                error: Some(BackendError {
                    code: "DESKTOP_HELPER".into(),
                    message: e.to_string(),
                }),
            }
        }
    };
    let backend = actor::native();
    let outcome = runtime
        .block_on(async { tokio::time::timeout(timeout, backend.call_typed(Call::Connect)).await });
    backend.shutdown();
    match outcome {
        Ok(Ok(Reply::Capabilities(capabilities))) => BackendReport {
            connected: true,
            capabilities: serde_json::to_value(capabilities).ok(),
            error: None,
        },
        Ok(Ok(_)) => failure("DESKTOP_HELPER", "unexpected reply to connect"),
        Ok(Err(e)) => failure(&e.code, &e.message),
        Err(_) => failure("DESKTOP_TIMEOUT", "the backend did not answer in time"),
    }
}

fn failure(code: &str, message: &str) -> BackendReport {
    BackendReport {
        connected: false,
        capabilities: None,
        error: Some(BackendError {
            code: code.to_string(),
            message: message.to_string(),
        }),
    }
}

/// Gathers the report. `backend` is probed by the caller so this stays testable.
pub fn collect(config_path: &Path, backend: BackendReport) -> Report {
    let (config_valid, config_problem, allowed_apps, http) = match load(config_path) {
        Ok(config) => {
            let problems = validate(&config);
            (
                problems.is_empty(),
                (!problems.is_empty()).then(|| problems.join("; ")),
                config.allowed_apps.len(),
                HttpReport {
                    enabled: config.http.enabled,
                    port: config.http.port,
                    port_free: port_free(config.http.port),
                },
            )
        }
        Err(e) => {
            let defaults = desktop_policy::HttpConfig::default();
            (
                false,
                Some(e.to_string()),
                0,
                HttpReport {
                    enabled: false,
                    port: defaults.port,
                    port_free: port_free(defaults.port),
                },
            )
        }
    };
    let connected = backend.connected;
    Report {
        version: env!("CARGO_PKG_VERSION").to_string(),
        platform: desktop_mcp::Platform::name().to_string(),
        config_path: config_path.display().to_string(),
        config_valid,
        config_problem,
        allowed_apps,
        http,
        accessibility_trusted: cfg!(target_os = "macos").then_some(connected),
        atspi_reachable: cfg!(target_os = "linux").then_some(connected),
        backend,
    }
}

/// A human readable report.
pub fn render_text(report: &Report) -> String {
    let mut out = String::new();
    let mut line = |text: String| {
        out.push_str(&text);
        out.push('\n');
    };
    line(format!(
        "aevra-desktop {} on {}",
        report.version, report.platform
    ));
    line(format!(
        "config: {} ({})",
        report.config_path,
        if report.config_valid {
            "valid"
        } else {
            "INVALID"
        }
    ));
    if let Some(problem) = &report.config_problem {
        line(format!("  problem: {problem}"));
    }
    line(format!("allowed apps: {}", report.allowed_apps));
    line(format!(
        "http: {} on port {} ({})",
        if report.http.enabled {
            "enabled"
        } else {
            "disabled"
        },
        report.http.port,
        if report.http.port_free {
            "port free"
        } else {
            "port in use"
        }
    ));
    match (&report.backend.capabilities, &report.backend.error) {
        (Some(capabilities), _) => line(format!("backend: connected, capabilities {capabilities}")),
        (None, Some(error)) => line(format!(
            "backend: not connected, {}: {}",
            error.code, error.message
        )),
        (None, None) => line("backend: not connected".to_string()),
    }
    if let Some(caps) = &report.backend.capabilities {
        for (key, label) in [("capture", "capture"), ("input", "foreground input")] {
            let ready = caps[key].as_bool().unwrap_or(false);
            line(format!(
                "{label}: {}",
                if ready { "ready" } else { "not ready" }
            ));
            if !ready {
                let recovery=match report.platform.as_str() {
                    "macos" if key=="capture"=>"Grant Screen Recording in System Settings > Privacy & Security; reconnect afterward.",
                    "macos"=>"Grant Accessibility in System Settings > Privacy & Security; reconnect afterward.",
                    "linux"=>"For X11, enable AT-SPI, XTest and a compositing window manager. For GNOME Wayland, install/enable the Aevra Desktop Access companion and enable access and approve the helper path in its panel menu, then reconnect. Locking or pausing revokes access. KDE KWin 6.6 needs the native companion built for the installed KWin release, a private helper-path grant with a fresh nonce, and reconnect. No XWayland/uinput fallback.",
                    _=>"Check the interactive desktop and native backend permissions, then reconnect.",
                };
                line(format!("  {recovery}"));
            }
        }
    }
    if let Some(trusted) = report.accessibility_trusted {
        line(format!(
            "accessibility permission: {}",
            if trusted {
                "granted"
            } else {
                "missing, grant it in System Settings, Privacy & Security, Accessibility"
            }
        ));
    }
    if let Some(reachable) = report.atspi_reachable {
        line(format!(
            "AT-SPI: {}",
            if reachable {
                "reachable"
            } else {
                "not reachable, enable the accessibility bus (at-spi2-core)"
            }
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn connected() -> BackendReport {
        BackendReport {
            connected: true,
            capabilities: Some(serde_json::json!({"capture": false})),
            error: None,
        }
    }

    #[test]
    fn a_missing_config_is_valid_and_empty() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        let report = collect(&path, connected());
        assert!(report.config_valid);
        assert_eq!(report.allowed_apps, 0);
        assert!(!report.http.enabled);
        assert_eq!(report.config_path, path.display().to_string());
    }

    #[test]
    fn an_invalid_config_is_reported_not_fatal() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        std::fs::write(&path, "{ nope").unwrap();
        let report = collect(&path, connected());
        assert!(!report.config_valid);
        assert!(report.config_problem.is_some());
        // The user's file is untouched.
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{ nope");
    }

    #[test]
    fn a_busy_port_is_reported() {
        let held = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let port = held.local_addr().unwrap().port();
        assert!(!port_free(port));
        drop(held);
        assert!(port_free(port));
    }

    #[test]
    fn json_uses_camel_case_keys_and_hides_other_platforms_fields() {
        let dir = tempfile::tempdir().unwrap();
        let report = collect(&dir.path().join("config.json"), connected());
        let json = serde_json::to_value(&report).unwrap();
        assert!(json.get("configPath").is_some());
        assert!(json.get("allowedApps").is_some());
        assert!(json["http"]["portFree"].is_boolean());
        assert_eq!(
            json.get("accessibilityTrusted").is_some(),
            cfg!(target_os = "macos")
        );
        assert_eq!(
            json.get("atspiReachable").is_some(),
            cfg!(target_os = "linux")
        );
    }

    #[test]
    fn text_names_the_failing_backend_code() {
        let dir = tempfile::tempdir().unwrap();
        let report = collect(
            &dir.path().join("config.json"),
            failure("DESKTOP_PERMISSION_REQUIRED", "grant access"),
        );
        let text = render_text(&report);
        assert!(text.contains("DESKTOP_PERMISSION_REQUIRED"));
        assert!(text.contains("allowed apps: 0"));
    }
}

#[cfg(test)]
mod native_readiness_tests {
    use super::*;
    #[test]
    fn macos_reports_screen_recording_separately_from_accessibility() {
        let dir = tempfile::tempdir().unwrap();
        let mut report = collect(
            &dir.path().join("config.json"),
            BackendReport {
                connected: true,
                capabilities: Some(serde_json::json!({"capture":false,"input":true})),
                error: None,
            },
        );
        report.platform = "macos".into();
        let text = render_text(&report);
        assert!(text.contains("capture: not ready"));
        assert!(text.contains("Screen Recording"));
        assert!(text.contains("foreground input: ready"));
    }
}
