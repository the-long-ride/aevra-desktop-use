//! Foreground input: click, type, key and scroll. Windows only.

use super::{arg_str, public_window, unexpected_reply, Output, Server};
use desktop_core::{ActRequest, Call, DesktopError, Reply, ScreenState};
use desktop_policy::activity::ActivityEntry;
use desktop_policy::sensitive::secret_shaped;
use serde_json::{json, Value};

fn act_request(op: &str) -> ActRequest {
    ActRequest {
        expected_target: None,
        op: op.to_string(),
        handle: None,
        x: None,
        y: None,
        text: None,
        keys: None,
        delta_y: None,
    }
}

/// What changed on screen between two reads. Port of aevra's `computeDelta`
/// (`packages/desktop/src/delta.ts`).
pub(crate) fn compute_delta(before: &ScreenState, after: &ScreenState) -> Value {
    json!({
        "focusChanged": before.window.window_id != after.window.window_id,
        "newWindow": after.window_ids.iter().any(|id| !before.window_ids.contains(id)),
        "subtreeChanged": before.signature != after.signature,
    })
}

/// The conservative delta used when the post-action read fails: the action did
/// land, so the caller must not treat the flags as observations.
pub(crate) fn unreliable_delta() -> Value {
    json!({
        "focusChanged": true,
        "newWindow": true,
        "subtreeChanged": true,
        "postActionStateUnknown": true,
    })
}

fn coordinate(args: &Value, key: &str) -> Option<i32> {
    args.get(key)
        .and_then(Value::as_f64)
        .map(|n| n.round() as i32)
}

impl Server {
    pub(super) async fn act_foreground(
        &self,
        name: &str,
        args: &Value,
        entry: &mut ActivityEntry,
    ) -> Result<Output, DesktopError> {
        if !self.platform.foreground_input {
            return Err(DesktopError::new(
                "DESKTOP_INPUT_UNSUPPORTED",
                "No native foreground input adapter is available on this platform.",
            ));
        }
        let (backend, _) = self.connected().await?;
        if !self.session.status().capabilities.is_some_and(|c| c.input) {
            return Err(DesktopError::new("DESKTOP_PERMISSION_REQUIRED", "Native input is not ready. Check desktop doctor and OS permissions, then reconnect."));
        }

        // The window that has focus right now is the one the input lands in.
        let focused_id = self.focused_window_id(&backend).await?;
        let target = self.target_identity(&backend, &focused_id).await?;
        self.authorize(&target, entry)?;
        if self
            .session
            .leases
            .is_window_leased(&target.window.window_id)
        {
            return Err(DesktopError::new(
                "DESKTOP_WINDOW_BUSY",
                "Target window is currently leased for background automation",
            ));
        }

        let mut request = match name {
            "desktop_click" => {
                let mut r = act_request("click");
                if let Some(reference) = arg_str(args, "ref") {
                    let resolved = self.session.resolve_foreground(reference)?;
                    r.handle = Some(resolved.handle);
                    self.session.note_ref_click(resolved.sensitive);
                } else if let (Some(x), Some(y)) = (coordinate(args, "x"), coordinate(args, "y")) {
                    r.x = Some(x);
                    r.y = Some(y);
                    self.session.note_coordinate_click();
                } else {
                    return Err(DesktopError::new(
                        "INVALID_REQUEST",
                        "desktop_click needs a ref, or both x and y",
                    ));
                }
                r
            }
            "desktop_type" => {
                let text = arg_str(args, "text").unwrap_or_default();
                if secret_shaped(text) {
                    return Err(DesktopError::new(
                        "DESKTOP_INPUT_REFUSED",
                        "Aevra will not type secret-shaped data",
                    ));
                }
                if let Some(kind) = self.session.typing_blocked() {
                    return Err(DesktopError::new(
                        "DESKTOP_INPUT_REFUSED",
                        format!("Refusing to type while a {kind} field may have focus"),
                    ));
                }
                let mut r = act_request("type");
                r.text = Some(text.to_string());
                r
            }
            "desktop_key" => {
                let mut r = act_request("key");
                r.keys = Some(arg_str(args, "keys").unwrap_or_default().to_string());
                r
            }
            "desktop_scroll" => {
                let mut r = act_request("scroll");
                r.delta_y = coordinate(args, "deltaY");
                r
            }
            other => {
                return Err(DesktopError::new(
                    "INVALID_REQUEST",
                    format!("Unknown foreground tool: {other}"),
                ))
            }
        };
        request.expected_target = Some(target.clone());
        let Reply::ScreenState(before) = backend.call_typed(Call::ScreenState).await? else {
            return Err(unexpected_reply("screenState"));
        };
        let Reply::Bool(ok) = backend.call_typed(Call::Act(request)).await? else {
            return Err(unexpected_reply("act"));
        };
        // The action has already happened; a failed trailing read must not
        // turn a landed click into an error.
        let delta = match backend.call_typed(Call::ScreenState).await {
            Ok(Reply::ScreenState(after)) => compute_delta(&before, &after),
            _ => unreliable_delta(),
        };
        Ok(Output::Json(json!({
            "ok": ok,
            "delta": delta,
            "window": public_window(&target.window),
        })))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use desktop_core::WindowIdentity;

    fn state(window: &str, ids: &[&str], signature: &str) -> ScreenState {
        ScreenState {
            window: WindowIdentity {
                window_id: window.into(),
                process_name: None,
                executable_path: None,
                title: None,
            },
            window_ids: ids.iter().map(|s| s.to_string()).collect(),
            signature: signature.into(),
        }
    }

    #[test]
    fn delta_reports_focus_new_window_and_subtree() {
        let before = state("1", &["1"], "a");
        assert_eq!(
            compute_delta(&before, &state("1", &["1"], "a")),
            json!({"focusChanged": false, "newWindow": false, "subtreeChanged": false})
        );
        assert_eq!(
            compute_delta(&before, &state("2", &["1", "2"], "b")),
            json!({"focusChanged": true, "newWindow": true, "subtreeChanged": true})
        );
    }

    #[test]
    fn unreliable_delta_is_marked() {
        assert_eq!(unreliable_delta()["postActionStateUnknown"], true);
    }
}
