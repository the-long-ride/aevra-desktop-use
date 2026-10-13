//! Background control of a real Windows Forms app through the native backend.
//!
//! Builds aevra's `background-controls` fixture (needs the dotnet SDK; the test
//! skips with a note if dotnet is missing), starts the target form and a
//! foreground sentinel that is a copy of the same program, allows only the
//! target by path, and checks that semantic actions reach the target without
//! touching the sentinel.
//!
//! Set `AEVRA_SKIP_NATIVE_FIXTURE=1` to skip it, as CI does everywhere but its
//! dedicated step. The launcher supplies `AEVRA_NATIVE_FIXTURE_DIR` after building
//! under the runner account, so the standard test account only runs the fixture.
#![cfg(windows)]

use desktop_core::{actor, Call, Reply};
use desktop_mcp::apps::EmptyCatalog;
use desktop_mcp::Server;
use desktop_policy::activity::ActivityLog;
use desktop_policy::{AllowedApp, AppIdentity, Config};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

const OWNER: &str = "native-test";
const TARGET_TITLE: &str = "Aevra Background Target Fixture";
const SENTINEL_TITLE: &str = "Aevra Foreground Sentinel";

/// Kills the child when the test ends, however it ends.
struct Guard(Child);

impl Drop for Guard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// Builds the fixture outside `vendor/` and returns the directory holding
/// `background-controls.exe`, or `None` if it cannot be built here.
fn build_fixture(work: &Path) -> Option<PathBuf> {
    if std::env::var("AEVRA_SKIP_NATIVE_FIXTURE").is_ok_and(|v| !v.is_empty()) {
        eprintln!("skipping: AEVRA_SKIP_NATIVE_FIXTURE is set");
        return None;
    }
    if let Some(prepared) = std::env::var_os("AEVRA_NATIVE_FIXTURE_DIR") {
        let out = PathBuf::from(prepared);
        assert!(
            out.is_absolute() && out.join("background-controls.exe").is_file(),
            "Prepared native fixture is missing background-controls.exe: {}",
            out.display()
        );
        return Some(out);
    }
    let project = Path::new(env!("CARGO_MANIFEST_DIR")).join(
        "../../vendor/aevra/helper/tests/fixtures/background-controls/background-controls.csproj",
    );
    if !project.exists() {
        eprintln!("skipping: the aevra fixture is not checked out");
        return None;
    }
    let out = work.join("out");
    let obj = work.join("obj");
    let status = Command::new("dotnet")
        .arg("build")
        .arg(&project)
        .args(["-c", "Release", "--nologo", "-v", "q"])
        .arg("-o")
        .arg(&out)
        .arg(format!("-p:BaseIntermediateOutputPath={}\\", obj.display()))
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status();
    match status {
        Ok(s) if s.success() && out.join("background-controls.exe").exists() => Some(out),
        Ok(s) => {
            eprintln!("skipping: dotnet build failed ({s})");
            None
        }
        Err(e) => {
            eprintln!("skipping: dotnet is not available ({e})");
            None
        }
    }
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        std::fs::copy(entry.path(), to.join(entry.file_name())).unwrap();
    }
}

fn text_json(result: &rmcp::model::CallToolResult) -> Value {
    let text = result
        .content
        .iter()
        .find_map(|c| c.as_text())
        .expect("a text block");
    serde_json::from_str(&text.text).expect("json text")
}

fn error_code(result: &rmcp::model::CallToolResult) -> Option<String> {
    (result.is_error == Some(true))
        .then(|| {
            text_json(result)["error"]["code"]
                .as_str()
                .map(str::to_string)
        })
        .flatten()
}

async fn call(server: &Server, name: &str, args: Value) -> rmcp::model::CallToolResult {
    server.call(OWNER, name, args).await
}

/// The window id of the window titled `title`, waiting for it to appear.
async fn wait_for_window(server: &Server, title: &str) -> Option<String> {
    let deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < deadline {
        let windows = text_json(&call(server, "desktop_windows", json!({})).await);
        let found = windows["result"].as_array().and_then(|all| {
            all.iter()
                .find(|w| w["title"].as_str() == Some(title))
                .and_then(|w| w["windowId"].as_str())
                .map(str::to_string)
        });
        if found.is_some() {
            return found;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    None
}

/// Title of whichever window has the foreground, allowed or not.
async fn foreground_title(server: &Server) -> String {
    match server.backend().call_typed(Call::FocusedWindow).await {
        Ok(Reply::Window(w)) => w.title.unwrap_or_default(),
        other => panic!("could not read the foreground window: {:?}", other.err()),
    }
}

struct Snapshot {
    id: String,
    lease: String,
    nodes: Vec<Value>,
}

impl Snapshot {
    fn find(&self, role: &str, name_part: &str) -> &Value {
        self.nodes
            .iter()
            .find(|n| {
                n["role"].as_str() == Some(role)
                    && n["name"]
                        .as_str()
                        .is_some_and(|name| name.contains(name_part))
            })
            .unwrap_or_else(|| panic!("no {role} named like {name_part:?} in {:#?}", self.nodes))
    }
}

async fn describe(server: &Server, window_id: &str) -> Snapshot {
    let r = call(
        server,
        "desktop_describe",
        json!({"windowId": window_id, "mode": "background"}),
    )
    .await;
    assert!(error_code(&r).is_none(), "describe failed: {r:?}");
    let v = text_json(&r);
    Snapshot {
        id: v["snapshotId"].as_str().unwrap().to_string(),
        lease: v["windowLeaseId"].as_str().unwrap().to_string(),
        nodes: v["nodes"].as_array().unwrap().clone(),
    }
}

fn action_args(window_id: &str, snapshot: &Snapshot, node: &Value) -> Value {
    json!({
        "windowId": window_id,
        "windowLeaseId": snapshot.lease,
        "snapshotId": snapshot.id,
        "ref": node["ref"],
    })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn background_actions_reach_the_target_and_leave_the_foreground_alone() {
    let work = Path::new(env!("CARGO_TARGET_TMPDIR")).join("aevra-fixture");
    let Some(target_dir) = build_fixture(&work) else {
        assert!(
            std::env::var_os("AEVRA_REQUIRE_NATIVE_FIXTURE").is_none(),
            "Required native fixture could not be built; inspect dotnet and fixture checkout"
        );
        return;
    };
    // The sentinel is a copy of the same program, so it is a different app by
    // path and stays outside the allowlist.
    let sentinel_dir = work.join("sentinel");
    copy_dir(&target_dir, &sentinel_dir);
    let target_exe = target_dir.join("background-controls.exe");
    let target_exe = target_exe.canonicalize().unwrap();
    let target_exe_text = target_exe
        .to_string_lossy()
        .trim_start_matches(r"\\?\")
        .to_string();

    let config = Config {
        allowed_apps: vec![AllowedApp {
            id: "app_fixture".into(),
            name: "Background fixture".into(),
            identity: AppIdentity::Exe {
                path: target_exe_text.clone(),
            },
        }],
        ..Config::default()
    };
    let log_dir = tempfile::tempdir().unwrap();
    let server = Server::new(
        actor::native(),
        config,
        ActivityLog::new(log_dir.path()),
        Arc::new(EmptyCatalog),
    );
    let connected = call(&server, "desktop_connect", json!({})).await;
    assert!(error_code(&connected).is_none(), "connect: {connected:?}");

    // Target first, then the sentinel, so the sentinel holds the foreground.
    let _target = Guard(Command::new(&target_exe).spawn().expect("start target"));
    let target_window = wait_for_window(&server, TARGET_TITLE)
        .await
        .expect("the allowed target window is listed");
    let _sentinel = Guard(
        Command::new(sentinel_dir.join("background-controls.exe"))
            .arg("--sentinel")
            .spawn()
            .expect("start sentinel"),
    );
    tokio::time::sleep(Duration::from_secs(2)).await;

    let windows = text_json(&call(&server, "desktop_windows", json!({})).await);
    let titles: Vec<&str> = windows["result"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|w| w["title"].as_str())
        .collect();
    assert!(
        !titles.contains(&SENTINEL_TITLE),
        "the sentinel is not allowed and must not be listed: {titles:?}"
    );

    let snapshot = describe(&server, &target_window).await;
    assert!(
        snapshot
            .nodes
            .iter()
            .any(|n| n["name"].as_str() == Some("Ready")),
        "the status label starts as Ready"
    );
    let password = snapshot.find("edit", "Password Text");
    assert_eq!(
        password["value"], "[redacted]",
        "a password field is never read"
    );

    // Invoke the button in the background.
    let before = foreground_title(&server).await;
    let button = snapshot.find("button", "Invoke Target");
    let r = call(
        &server,
        "desktop_invoke",
        action_args(&target_window, &snapshot, button),
    )
    .await;
    assert!(error_code(&r).is_none(), "invoke failed: {r:?}");
    let after_title = foreground_title(&server).await;
    let invoked = text_json(&r);
    // A WinForms button takes focus when it is clicked, even through UI
    // Automation, so the foreground may move. The contract is that the move is
    // reported, not that it never happens, and that no input reaches the
    // window that had the foreground.
    assert_eq!(
        invoked["focusChanged"].as_bool().unwrap_or(false),
        before != after_title,
        "focusChanged must match the observed foreground ({before:?} -> {after_title:?}): {invoked}"
    );

    // Set the editable box.
    let fresh = describe(&server, &target_window).await;
    let editable = fresh.find("edit", "Editable Text");
    let mut args = action_args(&target_window, &fresh, editable);
    args["value"] = json!("Typed in the background");
    let r = call(&server, "desktop_set_value", args).await;
    assert!(error_code(&r).is_none(), "set value failed: {r:?}");

    // The password field refuses writes, whatever the value.
    let fresh = describe(&server, &target_window).await;
    let password = fresh.find("edit", "Password Text");
    let mut args = action_args(&target_window, &fresh, password);
    args["value"] = json!("placeholder");
    let r = call(&server, "desktop_set_value", args).await;
    assert_eq!(
        error_code(&r).as_deref(),
        Some("DESKTOP_INPUT_REFUSED"),
        "{r:?}"
    );

    // The invoke and the edit both landed.
    let after = describe(&server, &target_window).await;
    assert!(
        after
            .nodes
            .iter()
            .any(|n| n["name"].as_str() == Some("Invoked")),
        "the status label shows the invoke: {:#?}",
        after.nodes
    );
    assert_eq!(
        after.find("edit", "Editable Text")["value"],
        "Typed in the background"
    );

    // The sentinel was never touched. It is outside the allowlist, so look at
    // it through a second server that allows it for reading only.
    let sentinel_exe = sentinel_dir
        .join("background-controls.exe")
        .canonicalize()
        .unwrap();
    let observer = Server::new(
        actor::native(),
        Config {
            allowed_apps: vec![AllowedApp {
                id: "app_sentinel".into(),
                name: "Sentinel".into(),
                identity: AppIdentity::Exe {
                    path: sentinel_exe
                        .to_string_lossy()
                        .trim_start_matches(r"\\?\")
                        .to_string(),
                },
            }],
            ..Config::default()
        },
        ActivityLog::new(log_dir.path()),
        Arc::new(EmptyCatalog),
    );
    let connected = observer.call(OWNER, "desktop_connect", json!({})).await;
    assert!(error_code(&connected).is_none(), "{connected:?}");
    let sentinel_window = wait_for_window(&observer, SENTINEL_TITLE)
        .await
        .expect("the sentinel window is listed for the observer");
    let sentinel = describe(&observer, &sentinel_window).await;
    assert_eq!(
        sentinel.find("edit", "")["value"],
        "SENTINEL_INITIAL",
        "no keystrokes leaked into the foreground window"
    );
}
