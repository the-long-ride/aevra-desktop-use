use desktop_core::actor::{BackendHandle, Call, Reply};
use desktop_core::{
    ActRequest, BackgroundActRequest, BackgroundActResult, Capabilities, CaptureRequest,
    CaptureResult, DescribeRequest, DescribeResult, DesktopBackend, ScreenState, WindowIdentity,
};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

fn identity(id: &str) -> WindowIdentity {
    WindowIdentity {
        window_id: id.to_string(),
        process_name: Some("example.exe".into()),
        executable_path: Some(r"C:\Example\example.exe".into()),
        title: None,
    }
}

/// Holds `Rc` and `RefCell`, so it is neither `Send` nor `Sync`: it can only
/// work if the actor builds it on its own thread.
struct FakeBackend {
    calls: Rc<Cell<u32>>,
    seen: RefCell<Vec<usize>>,
}

impl FakeBackend {
    fn new() -> Self {
        Self {
            calls: Rc::new(Cell::new(0)),
            seen: RefCell::new(Vec::new()),
        }
    }
}

impl DesktopBackend for FakeBackend {
    fn connect(&self) -> Result<Capabilities, String> {
        self.calls.set(self.calls.get() + 1);
        Ok(Capabilities {
            capture: false,
            tree: true,
            attribution: true,
            input: false,
            background_actions: Some(true),
        })
    }
    fn windows(&self) -> Result<Vec<WindowIdentity>, String> {
        Ok(vec![identity("1"), identity("2")])
    }
    fn focused_window(&self) -> Result<WindowIdentity, String> {
        Ok(identity("1"))
    }
    fn screen_state(&self) -> Result<ScreenState, String> {
        Ok(ScreenState {
            window: identity("1"),
            window_ids: vec!["1".into()],
            signature: "sig".into(),
        })
    }
    fn describe(&self, request: DescribeRequest) -> Result<DescribeResult, String> {
        if request.max_nodes == 999 {
            panic!("backend exploded");
        }
        self.seen.borrow_mut().push(request.max_nodes);
        Ok(DescribeResult {
            window: identity("1"),
            nodes: Vec::new(),
            truncated: request.max_nodes.is_multiple_of(2),
            snapshot_id: Some(format!("seq-{}", self.seen.borrow().len())),
            window_lease_id: None,
            lease_expires_at: None,
        })
    }
    fn act(&self, request: ActRequest) -> Result<bool, String> {
        match request.op.as_str() {
            "stale" => Err("DESKTOP_REF_STALE: gone".into()),
            "plain" => Err("boom".into()),
            "unknown" => Err("DESKTOP_MADE_UP: x".into()),
            _ => Ok(true),
        }
    }
    fn capture(&self, _request: CaptureRequest) -> Result<CaptureResult, String> {
        Err("DESKTOP_CAPTURE_UNSUPPORTED: not here".into())
    }
    fn background_act(
        &self,
        _request: BackgroundActRequest,
    ) -> Result<BackgroundActResult, String> {
        Err("DESKTOP_PATTERN_UNSUPPORTED".into())
    }
}

fn act(op: &str) -> Call {
    Call::Act(ActRequest {
        expected_target: None,
        op: op.to_string(),
        handle: None,
        x: None,
        y: None,
        text: None,
        keys: None,
        delta_y: None,
    })
}

fn describe(max_nodes: usize) -> Call {
    Call::Describe(DescribeRequest {
        expected_target: None,
        window_id: None,
        max_nodes,
        interactive_only: false,
        mode: None,
    })
}

fn handle() -> BackendHandle {
    BackendHandle::spawn(FakeBackend::new)
}

#[tokio::test]
async fn windows_returns_two_windows() {
    let value = handle().call(Call::Windows).await.unwrap();
    assert_eq!(value.as_array().unwrap().len(), 2);
    assert_eq!(value[0]["windowId"], "1");
}

#[tokio::test]
async fn typed_reply_keeps_the_backend_types() {
    match handle().call_typed(Call::FocusedWindow).await.unwrap() {
        Reply::Window(w) => assert_eq!(w.window_id, "1"),
        other => panic!("unexpected reply {other:?}"),
    }
}

#[tokio::test]
async fn helper_coded_error_keeps_its_code() {
    let err = handle().call(act("stale")).await.unwrap_err();
    assert_eq!(err.code, "DESKTOP_REF_STALE");
    assert_eq!(err.message, "gone");
}

#[tokio::test]
async fn plain_helper_error_becomes_desktop_helper() {
    let err = handle().call(act("plain")).await.unwrap_err();
    assert_eq!(err.code, "DESKTOP_HELPER");
    assert_eq!(err.message, "boom");
}

#[tokio::test]
async fn unknown_helper_code_becomes_desktop_helper() {
    let err = handle().call(act("unknown")).await.unwrap_err();
    assert_eq!(err.code, "DESKTOP_HELPER");
}

#[tokio::test]
async fn code_without_a_message_is_still_recognised() {
    let err = handle()
        .call(Call::BackgroundAct(BackgroundActRequest {
            snapshot_id: "s".into(),
            handle: "h".into(),
            op: "invoke".into(),
            value: None,
            expected_instance: desktop_core::WindowInstance::new("1", 1, "t"),
            expected_host: None,
        }))
        .await
        .unwrap_err();
    assert_eq!(err.code, "DESKTOP_PATTERN_UNSUPPORTED");
}

#[tokio::test]
async fn a_panicking_call_reports_driver_died_and_the_next_call_succeeds() {
    let h = handle();
    let err = h.call(describe(999)).await.unwrap_err();
    assert_eq!(err.code, "DESKTOP_DRIVER_DIED");
    let value = h.call(describe(2)).await.unwrap();
    assert_eq!(value["window"]["windowId"], "1");
}

#[tokio::test]
async fn hundred_concurrent_calls_run_fifo() {
    let h = handle();
    let pending: Vec<_> = (1..=100usize).map(|n| h.submit(describe(n))).collect();
    let mut sequence = Vec::new();
    for reply in pending {
        let value = reply.await.unwrap();
        sequence.push(value["snapshotId"].as_str().unwrap().to_string());
    }
    let expected: Vec<String> = (1..=100).map(|n| format!("seq-{n}")).collect();
    assert_eq!(sequence, expected);
}

#[tokio::test]
async fn calls_after_shutdown_are_refused() {
    let h = handle();
    h.call(Call::Connect).await.unwrap();
    h.shutdown();
    let err = h.call(Call::Connect).await.unwrap_err();
    assert_eq!(err.code, "DESKTOP_DRIVER_DIED");
}
