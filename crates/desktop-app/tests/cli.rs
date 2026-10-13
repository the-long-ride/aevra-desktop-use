//! Runs the built binary. These tests use a throwaway config directory and
//! never open a window.

use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};
use std::time::Duration;

const BIN: &str = env!("CARGO_BIN_EXE_aevra-desktop");

fn command(home: &std::path::Path) -> Command {
    let mut command = Command::new(BIN);
    command.env("AEVRA_DESKTOP_HOME", home);
    command
}

#[test]
fn version_prints_the_crate_version() {
    let home = tempfile::tempdir().unwrap();
    let out = command(home.path()).arg("--version").output().unwrap();
    assert!(out.status.success());
    assert_eq!(
        String::from_utf8_lossy(&out.stdout).trim(),
        format!("aevra-desktop {}", env!("CARGO_PKG_VERSION"))
    );
}

#[test]
fn help_lists_the_commands() {
    let home = tempfile::tempdir().unwrap();
    let out = command(home.path()).arg("--help").output().unwrap();
    assert!(out.status.success());
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("aevra-desktop mcp"));
    assert!(text.contains("doctor"));
}

#[test]
fn an_unknown_command_fails_with_usage_on_stderr() {
    let home = tempfile::tempdir().unwrap();
    let out = command(home.path()).arg("frobnicate").output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(out.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("frobnicate"));
    assert!(stderr.contains("USAGE"));
}

#[test]
fn doctor_json_reports_the_config_path_in_the_override_directory() {
    let home = tempfile::tempdir().unwrap();
    let out = command(home.path())
        .args(["doctor", "--json"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&out.stdout).expect("valid json");
    let config_path = report["configPath"].as_str().expect("configPath");
    assert!(
        std::path::Path::new(config_path).starts_with(home.path()),
        "{config_path}"
    );
    assert_eq!(report["configValid"], true);
    assert_eq!(report["allowedApps"], 0);
    assert!(report["http"]["port"].is_number());
    assert!(report["backend"]["connected"].is_boolean());
    // Reading the report must not create the config.
    assert!(!home.path().join("config.json").exists());
}

#[test]
fn doctor_text_mentions_the_allowlist() {
    let home = tempfile::tempdir().unwrap();
    let out = command(home.path()).arg("doctor").output().unwrap();
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stdout).contains("allowed apps: 0"));
}

#[test]
fn doctor_flags_a_damaged_config_without_touching_it() {
    let home = tempfile::tempdir().unwrap();
    let file = home.path().join("config.json");
    std::fs::write(&file, "{ broken").unwrap();
    let out = command(home.path())
        .args(["doctor", "--json"])
        .output()
        .unwrap();
    assert!(out.status.success());
    let report: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(report["configValid"], false);
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "{ broken");
}

fn read_json_line(reader: &mut impl BufRead) -> serde_json::Value {
    let mut line = String::new();
    let n = reader.read_line(&mut line).expect("read a line");
    assert!(n > 0, "the server closed stdout early");
    serde_json::from_str(line.trim())
        .unwrap_or_else(|e| panic!("stdout line is not JSON ({e}): {line:?}"))
}

#[test]
fn mcp_mode_answers_the_protocol_on_stdout_and_nothing_else() {
    let home = tempfile::tempdir().unwrap();
    let mut child = command(home.path())
        .arg("mcp")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());

    writeln!(
        stdin,
        "{}",
        serde_json::json!({
            "jsonrpc": "2.0", "id": 1, "method": "initialize",
            "params": {
                "protocolVersion": "2025-06-18",
                "capabilities": {},
                "clientInfo": {"name": "cli-test", "version": "0"}
            }
        })
    )
    .unwrap();
    let init = read_json_line(&mut stdout);
    assert_eq!(init["id"], 1);
    assert_eq!(init["result"]["serverInfo"]["name"], "aevra-desktop");

    writeln!(
        stdin,
        "{}",
        serde_json::json!({"jsonrpc": "2.0", "method": "notifications/initialized"})
    )
    .unwrap();
    writeln!(
        stdin,
        "{}",
        serde_json::json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"})
    )
    .unwrap();
    let tools = read_json_line(&mut stdout);
    assert_eq!(tools["id"], 2);
    assert_eq!(tools["result"]["tools"].as_array().unwrap().len(), 17);

    // desktop_apps lists only allowed apps, and none are allowed here.
    writeln!(
        stdin,
        "{}",
        serde_json::json!({
            "jsonrpc": "2.0", "id": 3, "method": "tools/call",
            "params": {"name": "desktop_apps", "arguments": {}}
        })
    )
    .unwrap();
    let apps = read_json_line(&mut stdout);
    assert_eq!(apps["id"], 3);
    let text = apps["result"]["content"][0]["text"].as_str().unwrap();
    let parsed: serde_json::Value = serde_json::from_str(text).unwrap();
    assert_eq!(parsed["apps"], serde_json::json!([]));

    // Closing stdin ends the session; whatever is left on stdout must still be protocol.
    drop(stdin);
    let mut rest = String::new();
    std::io::Read::read_to_string(&mut stdout, &mut rest).unwrap();
    for line in rest.lines().filter(|l| !l.trim().is_empty()) {
        serde_json::from_str::<serde_json::Value>(line)
            .unwrap_or_else(|e| panic!("non-protocol output on stdout ({e}): {line:?}"));
    }

    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    loop {
        match child.try_wait().unwrap() {
            Some(status) => {
                assert!(status.success(), "{status:?}");
                break;
            }
            None if std::time::Instant::now() > deadline => {
                let _ = child.kill();
                panic!("mcp did not exit after stdin closed");
            }
            None => std::thread::sleep(Duration::from_millis(50)),
        }
    }
}
