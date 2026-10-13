//! The desktop session: one backend connection per server process, its epoch,
//! the foreground ref table and the background leases. A port of aevra's
//! `DesktopSessionRegistry` (`packages/desktop/src/registry.ts`).

use crate::leases::Leases;
use desktop_core::{BackendHandle, Call, Capabilities, DesktopError, Reply};
use serde::Serialize;
use std::collections::HashMap;
use std::sync::Mutex;

#[derive(Debug, Clone, Serialize)]
pub struct SessionStatus {
    pub connected: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub capabilities: Option<Capabilities>,
}

/// One node of a foreground describe, before refs are minted.
#[derive(Debug, Clone)]
pub struct ForegroundNode {
    pub handle: String,
    pub focused: bool,
    pub sensitive: Option<&'static str>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForegroundRef {
    pub handle: String,
    pub sensitive: Option<&'static str>,
}

/// What is known about the element that has keyboard focus in the foreground.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Focus {
    /// Known from the last describe or a click on a ref.
    Known(Option<&'static str>),
    /// A coordinate click moved focus somewhere the tree does not say.
    Unknown,
}

struct State {
    backend: Option<BackendHandle>,
    capabilities: Option<Capabilities>,
    epoch: u64,
    next_ref: u64,
    refs: HashMap<String, ForegroundRef>,
    focus: Focus,
    /// The first sensitive node of the last foreground describe, if any.
    sensitive_in_view: Option<&'static str>,
}

pub struct Session {
    state: Mutex<State>,
    pub leases: Leases,
}

impl Default for Session {
    fn default() -> Self {
        Self::new()
    }
}

impl Session {
    pub fn new() -> Self {
        Self::with_leases(Leases::new())
    }

    pub fn with_leases(leases: Leases) -> Self {
        Self {
            state: Mutex::new(State {
                backend: None,
                capabilities: None,
                epoch: 0,
                next_ref: 1,
                refs: HashMap::new(),
                focus: Focus::Known(None),
                sensitive_in_view: None,
            }),
            leases,
        }
    }

    /// Reconnects: drops the old connection, bumps the epoch, clears every ref
    /// and lease, then asks the backend to connect.
    pub async fn connect(&self, h: &BackendHandle) -> Result<Capabilities, DesktopError> {
        let epoch = self.invalidate();
        let reply = h.call_typed(Call::Connect).await?;
        let Reply::Capabilities(caps) = reply else {
            return Err(DesktopError::new(
                "DESKTOP_HELPER",
                "backend returned an unexpected reply to connect",
            ));
        };
        let mut s = self.state.lock().unwrap();
        if s.epoch != epoch {
            return Err(Self::session_changed());
        }
        s.backend = Some(h.clone());
        s.capabilities = Some(caps);
        Ok(caps)
    }

    pub fn disconnect(&self) {
        self.invalidate();
    }

    fn invalidate(&self) -> u64 {
        let mut s = self.state.lock().unwrap();
        s.epoch += 1;
        let epoch = s.epoch;
        s.backend = None;
        s.capabilities = None;
        s.refs.clear();
        s.focus = Focus::Known(None);
        s.sensitive_in_view = None;
        drop(s);
        self.leases.reset();
        epoch
    }

    fn session_changed() -> DesktopError {
        DesktopError::new(
            "DESKTOP_NOT_CONNECTED",
            "Desktop session changed during the operation. Connect and retry.",
        )
    }

    pub fn verify_epoch(&self, epoch: u64) -> Result<(), DesktopError> {
        let s = self.state.lock().unwrap();
        if s.epoch != epoch || s.backend.is_none() {
            return Err(Self::session_changed());
        }
        Ok(())
    }

    /// The connected backend and the current epoch.
    pub fn require(&self) -> Result<(BackendHandle, u64), DesktopError> {
        let s = self.state.lock().unwrap();
        match &s.backend {
            Some(b) => Ok((b.clone(), s.epoch)),
            None => Err(DesktopError::new(
                "DESKTOP_NOT_CONNECTED",
                "No desktop session is connected. Call desktop_connect first.",
            )),
        }
    }

    pub fn status(&self) -> SessionStatus {
        let s = self.state.lock().unwrap();
        SessionStatus {
            connected: s.backend.is_some(),
            capabilities: s.capabilities,
        }
    }

    pub fn epoch(&self) -> u64 {
        self.state.lock().unwrap().epoch
    }

    /// Replaces the foreground ref table with refs `ref_<epoch>_<n>` for
    /// `nodes`, in order. The previous refs become stale.
    #[cfg(test)]
    fn set_foreground_refs(&self, nodes: Vec<ForegroundNode>) -> Vec<String> {
        Self::write_foreground_refs(&mut self.state.lock().unwrap(), nodes)
    }

    /// Atomically refuses publication from an invalidated connection.
    pub fn publish_foreground_refs(
        &self,
        epoch: u64,
        nodes: Vec<ForegroundNode>,
    ) -> Result<Vec<String>, DesktopError> {
        let mut s = self.state.lock().unwrap();
        if s.epoch != epoch || s.backend.is_none() {
            return Err(Self::session_changed());
        }
        Ok(Self::write_foreground_refs(&mut s, nodes))
    }

    fn write_foreground_refs(s: &mut State, nodes: Vec<ForegroundNode>) -> Vec<String> {
        s.refs.clear();
        let epoch = s.epoch;
        s.focus = Focus::Known(nodes.iter().find(|n| n.focused).and_then(|n| n.sensitive));
        s.sensitive_in_view = nodes.iter().find_map(|n| n.sensitive);
        let mut out = Vec::with_capacity(nodes.len());
        for node in nodes {
            let r = format!("ref_{epoch}_{}", s.next_ref);
            s.next_ref += 1;
            s.refs.insert(
                r.clone(),
                ForegroundRef {
                    handle: node.handle,
                    sensitive: node.sensitive,
                },
            );
            out.push(r);
        }
        out
    }

    pub fn resolve_foreground(&self, reference: &str) -> Result<ForegroundRef, DesktopError> {
        self.state
            .lock()
            .unwrap()
            .refs
            .get(reference)
            .cloned()
            .ok_or_else(|| {
                DesktopError::new(
                    "DESKTOP_REF_STALE",
                    format!("Ref {reference} is no longer live. Call desktop_describe again."),
                )
            })
    }

    /// A click on a ref moves focus to that element.
    pub fn note_ref_click(&self, sensitive: Option<&'static str>) {
        self.state.lock().unwrap().focus = Focus::Known(sensitive);
    }

    /// A coordinate click moves focus to something the tree does not name.
    pub fn note_coordinate_click(&self) {
        self.state.lock().unwrap().focus = Focus::Unknown;
    }

    /// Why typing must be refused right now, if it must: the focused element is
    /// a sensitive field, or focus is unknown and the last describe showed one.
    pub fn typing_blocked(&self) -> Option<&'static str> {
        let s = self.state.lock().unwrap();
        match s.focus {
            Focus::Known(kind) => kind,
            Focus::Unknown => s.sensitive_in_view,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use desktop_core::DesktopBackend;
    use desktop_core::{
        ActRequest, CaptureRequest, CaptureResult, DescribeRequest, DescribeResult, ScreenState,
        WindowIdentity,
    };

    struct Fake;
    impl DesktopBackend for Fake {
        fn connect(&self) -> Result<Capabilities, String> {
            Ok(Capabilities {
                capture: false,
                tree: true,
                attribution: true,
                input: false,
                background_actions: Some(true),
            })
        }
        fn windows(&self) -> Result<Vec<WindowIdentity>, String> {
            Ok(vec![])
        }
        fn focused_window(&self) -> Result<WindowIdentity, String> {
            Err("none".into())
        }
        fn screen_state(&self) -> Result<ScreenState, String> {
            Err("none".into())
        }
        fn describe(&self, _: DescribeRequest) -> Result<DescribeResult, String> {
            Err("none".into())
        }
        fn act(&self, _: ActRequest) -> Result<bool, String> {
            Ok(true)
        }
        fn capture(&self, _: CaptureRequest) -> Result<CaptureResult, String> {
            Err("none".into())
        }
    }

    fn fg(handle: &str, focused: bool, sensitive: Option<&'static str>) -> ForegroundNode {
        ForegroundNode {
            handle: handle.into(),
            focused,
            sensitive,
        }
    }

    #[test]
    fn require_without_a_connection_is_not_connected() {
        let s = Session::new();
        let err = s.require().err().unwrap();
        assert_eq!(err.code, "DESKTOP_NOT_CONNECTED");
        assert_eq!(
            err.message,
            "No desktop session is connected. Call desktop_connect first."
        );
        assert!(!s.status().connected);
    }

    #[tokio::test]
    async fn connect_bumps_the_epoch_and_reports_capabilities() {
        let s = Session::new();
        let h = BackendHandle::spawn(|| Fake);
        let caps = s.connect(&h).await.unwrap();
        assert!(caps.tree);
        let (_, epoch1) = s.require().unwrap();
        s.connect(&h).await.unwrap();
        let (_, epoch2) = s.require().unwrap();
        assert_eq!(epoch2, epoch1 + 1);
        let status = serde_json::to_value(s.status()).unwrap();
        assert_eq!(status["connected"], true);
        assert_eq!(status["capabilities"]["tree"], true);
        s.disconnect();
        assert!(!s.status().connected);
    }

    #[tokio::test]
    async fn reconnect_clears_refs_and_leases() {
        let s = Session::new();
        let h = BackendHandle::spawn(|| Fake);
        s.connect(&h).await.unwrap();
        let refs = s.set_foreground_refs(vec![fg("h", false, None)]);
        let win = desktop_core::WindowInstance::new("1", 1, "t");
        s.leases.acquire("o", &win, 1, None, "app").unwrap();
        s.connect(&h).await.unwrap();
        assert_eq!(
            s.resolve_foreground(&refs[0]).unwrap_err().code,
            "DESKTOP_REF_STALE"
        );
        assert!(!s.leases.is_window_leased("1"));
    }

    #[test]
    fn foreground_refs_go_stale_on_the_next_describe() {
        let s = Session::new();
        let first = s.set_foreground_refs(vec![fg("h1", false, None), fg("h2", false, None)]);
        assert_eq!(s.resolve_foreground(&first[1]).unwrap().handle, "h2");
        let second = s.set_foreground_refs(vec![fg("h3", false, None)]);
        assert_eq!(
            s.resolve_foreground(&first[0]).unwrap_err().code,
            "DESKTOP_REF_STALE"
        );
        assert_eq!(s.resolve_foreground(&second[0]).unwrap().handle, "h3");
        assert_ne!(first[0], second[0]);
    }

    #[test]
    fn typing_is_blocked_while_a_sensitive_field_has_focus() {
        let s = Session::new();
        assert_eq!(s.typing_blocked(), None);
        let refs =
            s.set_foreground_refs(vec![fg("a", false, None), fg("b", true, Some("password"))]);
        assert_eq!(s.typing_blocked(), Some("password"));
        // Clicking an ordinary element moves focus off it.
        let plain = s.resolve_foreground(&refs[0]).unwrap();
        s.note_ref_click(plain.sensitive);
        assert_eq!(s.typing_blocked(), None);
        // Clicking the sensitive one puts it back.
        let secret = s.resolve_foreground(&refs[1]).unwrap();
        s.note_ref_click(secret.sensitive);
        assert_eq!(s.typing_blocked(), Some("password"));
    }

    #[test]
    fn a_coordinate_click_is_unknown_focus_and_blocks_typing_next_to_a_secret() {
        let s = Session::new();
        s.set_foreground_refs(vec![fg("a", true, None), fg("b", false, Some("otp"))]);
        assert_eq!(s.typing_blocked(), None);
        s.note_coordinate_click();
        assert_eq!(s.typing_blocked(), Some("otp"));
        // With no sensitive field in view, unknown focus is harmless.
        s.set_foreground_refs(vec![fg("a", true, None)]);
        s.note_coordinate_click();
        assert_eq!(s.typing_blocked(), None);
    }
}
