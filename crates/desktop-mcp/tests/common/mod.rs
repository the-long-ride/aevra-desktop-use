//! A scriptable fake desktop backend plus helpers shared by the handler tests.
//! The fake holds an `Rc` so it cannot be `Send`: it only works because the
//! actor builds it on its own thread.
#![allow(dead_code)]

use desktop_core::{
    ActRequest, BackendHandle, BackgroundActRequest, BackgroundActResult, Capabilities,
    CaptureRequest, CaptureResult, DescribeBackgroundRequest, DescribeBackgroundResult,
    DescribeNode, DescribeRequest, DescribeResult, DesktopBackend,
    ReleaseBackgroundSnapshotRequest, ScreenState, TargetIdentityRequest, TargetIdentityResult,
    VerifiedWindowHost, WindowIdentity, WindowInstance,
};
use desktop_mcp::apps::EmptyCatalog;
use desktop_mcp::{Platform, Server};
use desktop_policy::activity::ActivityLog;
use desktop_policy::{AllowedApp, AppIdentity, Config};
use serde_json::Value;
use std::rc::Rc;
use std::sync::{Arc, Mutex};

pub const SELF_PID: u32 = 7777;
pub const OWNER: &str = "owner-1";

#[derive(Clone)]
pub struct FakeWindow {
    pub identity: WindowIdentity,
    pub instance: WindowInstance,
    pub host: Option<VerifiedWindowHost>,
    pub nodes: Vec<DescribeNode>,
}

#[derive(Default)]
pub struct FakeState {
    pub capabilities: Option<Capabilities>,
    pub windows: Vec<FakeWindow>,
    pub focused: String,
    pub focus_on_screen_state: Option<String>,
    /// Names of every backend method called, in order.
    pub calls: Vec<String>,
    pub acts: Vec<ActRequest>,
    pub background_acts: Vec<BackgroundActRequest>,
    pub released_snapshots: Vec<String>,
    pub described_snapshots: Vec<String>,
    pub focus_changes: bool,
    pub toggle_state: Option<String>,
    pub signature: u32,
    pub read_replacement: Option<FakeWindow>,
    pub connect_gate: Option<std::sync::mpsc::Receiver<()>>,
    pub read_gate: Option<std::sync::mpsc::Receiver<()>>,
    /// Instance reported by describeBackground instead of the real one.
    pub swap_instance_on_describe: Option<WindowInstance>,
    pub fail_background_act: Option<String>,
    /// Milliseconds each backgroundAct takes, to exercise batch deadlines.
    pub act_delay_ms: u64,
}

pub type Shared = Arc<Mutex<FakeState>>;

struct FakeBackend {
    state: Shared,
    _not_send: Rc<()>,
}

impl FakeBackend {
    fn record(&self, call: &str) {
        self.state.lock().unwrap().calls.push(call.to_string());
    }

    fn before_read(&self) {
        let gate = {
            let mut state = self.state.lock().unwrap();
            if let Some(replacement) = state.read_replacement.take() {
                let index = state
                    .windows
                    .iter()
                    .position(|w| w.identity.window_id == replacement.identity.window_id)
                    .unwrap();
                state.windows[index] = replacement;
            }
            state.read_gate.take()
        };
        if let Some(gate) = gate {
            gate.recv().unwrap();
        }
    }
    fn find(&self, id: &str) -> Result<FakeWindow, String> {
        self.state
            .lock()
            .unwrap()
            .windows
            .iter()
            .find(|w| w.identity.window_id == id)
            .cloned()
            .ok_or_else(|| "DESKTOP_TARGET_CHANGED: window no longer exists".to_string())
    }
}

impl DesktopBackend for FakeBackend {
    fn connect(&self) -> Result<Capabilities, String> {
        let gate = self.state.lock().unwrap().connect_gate.take();
        if let Some(gate) = gate {
            gate.recv().unwrap();
        }
        self.record("connect");
        if let Some(caps) = self.state.lock().unwrap().capabilities {
            return Ok(caps);
        }
        Ok(Capabilities {
            capture: true,
            tree: true,
            attribution: true,
            input: true,
            background_actions: Some(true),
        })
    }

    fn windows(&self) -> Result<Vec<WindowIdentity>, String> {
        self.record("windows");
        Ok(self
            .state
            .lock()
            .unwrap()
            .windows
            .iter()
            .map(|w| w.identity.clone())
            .collect())
    }

    fn focused_window(&self) -> Result<WindowIdentity, String> {
        self.record("focusedWindow");
        let focused = self.state.lock().unwrap().focused.clone();
        self.find(&focused).map(|w| w.identity)
    }

    fn screen_state(&self) -> Result<ScreenState, String> {
        self.record("screenState");
        {
            let mut state = self.state.lock().unwrap();
            if let Some(id) = state.focus_on_screen_state.take() {
                state.focused = id;
            }
        }
        let focused = self.state.lock().unwrap().focused.clone();
        let window = self.find(&focused)?.identity;
        let s = self.state.lock().unwrap();
        Ok(ScreenState {
            window,
            window_ids: s
                .windows
                .iter()
                .map(|w| w.identity.window_id.clone())
                .collect(),
            signature: s.signature.to_string(),
        })
    }

    fn describe(&self, request: DescribeRequest) -> Result<DescribeResult, String> {
        self.record("describe");
        self.before_read();
        let id = match request.window_id {
            Some(id) => id,
            None => self.state.lock().unwrap().focused.clone(),
        };
        let window = self.find(&id)?;
        let truncated = window.nodes.len() > request.max_nodes;
        let nodes = window
            .nodes
            .iter()
            .take(request.max_nodes)
            .enumerate()
            .map(|(i, n)| DescribeNode {
                handle: format!("fg:{id}:{i}"),
                ..n.clone()
            })
            .collect();
        Ok(DescribeResult {
            window: window.identity,
            nodes,
            truncated,
            snapshot_id: None,
            window_lease_id: None,
            lease_expires_at: None,
        })
    }

    fn act(&self, request: ActRequest) -> Result<bool, String> {
        self.record("act");
        let focused = self.state.lock().unwrap().focused.clone();
        let window = self.find(&focused)?;
        desktop_core::require_authorized_target(
            request.expected_target.as_ref(),
            &TargetIdentityResult {
                window: window.identity,
                window_instance: window.instance,
                host_application: window.host,
            },
        )?;
        let mut s = self.state.lock().unwrap();
        s.acts.push(request);
        s.signature += 1;
        Ok(true)
    }

    fn capture(&self, request: CaptureRequest) -> Result<CaptureResult, String> {
        self.record("capture");
        self.before_read();
        let window = match request.window_id {
            Some(id) => Some(self.find(&id)?.identity),
            None => None,
        };
        Ok(CaptureResult {
            image_data_uri: "data:image/jpeg;base64,/9j/4AAQ".to_string(),
            device_pixel_ratio: 1.0,
            window,
        })
    }

    fn target_identity(
        &self,
        request: TargetIdentityRequest,
    ) -> Result<TargetIdentityResult, String> {
        self.record("targetIdentity");
        let w = self.find(&request.window_id)?;
        Ok(TargetIdentityResult {
            window: w.identity,
            window_instance: w.instance,
            host_application: w.host,
        })
    }

    fn describe_background(
        &self,
        request: DescribeBackgroundRequest,
    ) -> Result<DescribeBackgroundResult, String> {
        self.record("describeBackground");
        let w = self.find(&request.window_id)?;
        let mut s = self.state.lock().unwrap();
        s.described_snapshots.push(request.snapshot_id.clone());
        let instance = s
            .swap_instance_on_describe
            .clone()
            .unwrap_or_else(|| w.instance.clone());
        let truncated = w.nodes.len() > request.max_nodes;
        let nodes = w
            .nodes
            .iter()
            .take(request.max_nodes)
            .enumerate()
            .map(|(i, n)| DescribeNode {
                handle: format!("bg:{}:{i}", request.snapshot_id),
                ..n.clone()
            })
            .collect();
        Ok(DescribeBackgroundResult {
            window: w.identity,
            window_instance: instance,
            host_application: w.host,
            nodes,
            truncated,
        })
    }

    fn release_background_snapshot(
        &self,
        request: ReleaseBackgroundSnapshotRequest,
    ) -> Result<bool, String> {
        self.record("releaseBackgroundSnapshot");
        self.state
            .lock()
            .unwrap()
            .released_snapshots
            .push(request.snapshot_id);
        Ok(true)
    }

    fn background_act(&self, request: BackgroundActRequest) -> Result<BackgroundActResult, String> {
        self.record("backgroundAct");
        let delay = self.state.lock().unwrap().act_delay_ms;
        if delay > 0 {
            std::thread::sleep(std::time::Duration::from_millis(delay));
        }
        let mut s = self.state.lock().unwrap();
        s.background_acts.push(request);
        if let Some(err) = s.fail_background_act.take() {
            return Err(err);
        }
        Ok(BackgroundActResult {
            ok: true,
            outcome: "invoked".into(),
            focus_changed: s.focus_changes,
            toggle_state: s.toggle_state.clone(),
        })
    }
}

pub fn node(role: &str, name: &str) -> DescribeNode {
    DescribeNode {
        handle: String::new(),
        role: role.into(),
        name: name.into(),
        value: None,
        enabled: true,
        focused: false,
        supported_actions: Some(vec!["invoke".into()]),
        read_only: None,
        toggle_state: None,
    }
}

pub fn window(id: &str, exe: &str, pid: u32, nodes: Vec<DescribeNode>) -> FakeWindow {
    FakeWindow {
        identity: WindowIdentity {
            window_id: id.into(),
            process_name: exe.rsplit(['\\', '/']).next().map(str::to_string),
            executable_path: Some(exe.into()),
            title: Some(format!("Window {id}")),
        },
        instance: WindowInstance::new(id, pid, "started-1"),
        host: None,
        nodes,
    }
}

pub fn allow(id: &str, name: &str, exe: &str) -> AllowedApp {
    AllowedApp {
        id: id.into(),
        name: name.into(),
        identity: AppIdentity::Exe { path: exe.into() },
    }
}

pub struct Harness {
    pub server: Server,
    pub state: Shared,
    pub dir: tempfile::TempDir,
}

pub const NOTEPAD: &str = r"C:\Apps\Notepad\notepad.exe";
pub const OTHER: &str = r"C:\Apps\Other\other.exe";

/// A server over a fake backend with `windows`, allowing `allowed` apps.
pub fn harness(windows: Vec<FakeWindow>, allowed: Vec<AllowedApp>) -> Harness {
    let focused = windows
        .first()
        .map(|w| w.identity.window_id.clone())
        .unwrap_or_default();
    let state: Shared = Arc::new(Mutex::new(FakeState {
        windows,
        focused,
        ..FakeState::default()
    }));
    let shared = Arc::clone(&state);
    let backend = BackendHandle::spawn(move || FakeBackend {
        state: Arc::clone(&shared),
        _not_send: Rc::new(()),
    });
    let dir = tempfile::tempdir().unwrap();
    let config = Config {
        allowed_apps: allowed,
        ..Config::default()
    };
    let server = Server::new(
        backend,
        config,
        ActivityLog::new(dir.path()),
        Arc::new(EmptyCatalog),
    )
    .with_self_pid(SELF_PID)
    .with_platform(Platform {
        foreground_input: true,
        capture: true,
    });
    Harness { server, state, dir }
}

/// The first text block of a tool result, parsed as JSON.
pub fn text_json(result: &rmcp::model::CallToolResult) -> Value {
    let text = result
        .content
        .iter()
        .find_map(|c| c.as_text())
        .expect("a text block");
    serde_json::from_str(&text.text).expect("json text")
}

pub fn is_error(result: &rmcp::model::CallToolResult) -> bool {
    result.is_error == Some(true)
}

pub fn error_code(result: &rmcp::model::CallToolResult) -> String {
    assert!(is_error(result), "expected an error result: {result:?}");
    text_json(result)["error"]["code"]
        .as_str()
        .unwrap()
        .to_string()
}

pub fn calls(h: &Harness) -> Vec<String> {
    h.state.lock().unwrap().calls.clone()
}

pub fn activity_lines(h: &Harness) -> Vec<String> {
    std::fs::read_to_string(h.dir.path().join("activity.jsonl"))
        .unwrap_or_default()
        .lines()
        .map(str::to_string)
        .collect()
}

pub async fn call(h: &Harness, name: &str, args: Value) -> rmcp::model::CallToolResult {
    h.server.call(OWNER, name, args).await
}

pub async fn connect(h: &Harness) {
    let r = call(h, "desktop_connect", serde_json::json!({})).await;
    assert!(!is_error(&r), "connect failed: {r:?}");
}

/// Connects, runs a background describe on `window_id` and returns
/// `(snapshotId, windowLeaseId, refs)`.
pub async fn describe_bg(h: &Harness, window_id: &str) -> (String, String, Vec<String>) {
    let r = call(
        h,
        "desktop_describe",
        serde_json::json!({"windowId": window_id, "mode": "background"}),
    )
    .await;
    assert!(!is_error(&r), "background describe failed: {r:?}");
    let v = text_json(&r);
    let refs = v["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n["ref"].as_str().unwrap().to_string())
        .collect();
    (
        v["snapshotId"].as_str().unwrap().to_string(),
        v["windowLeaseId"].as_str().unwrap().to_string(),
        refs,
    )
}
