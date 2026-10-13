//! Session, listing, describe and capture tools.

use super::{
    arg_str, public_node, public_window, unexpected_reply, untrusted, Output, Platform, Server,
};
use crate::session::ForegroundNode;
use desktop_core::{
    BackendHandle, Call, CaptureRequest, DescribeRequest, DesktopError, Reply, WindowIdentity,
};
use desktop_policy::activity::ActivityEntry;
use desktop_policy::gate::window_visible;
use desktop_policy::sensitive::sensitive_node;
use serde_json::{json, Value};

pub(crate) const DEFAULT_MAX_NODES: usize = 500;

pub(crate) fn max_nodes_arg(args: &Value, default: usize) -> usize {
    args.get("maxNodes")
        .and_then(Value::as_f64)
        .map(|n| n as usize)
        .unwrap_or(default)
        .clamp(1, 5000)
}

impl Server {
    /// The connected backend and epoch, after freeing snapshots of leases that
    /// expired since the last call.
    pub(crate) async fn connected(&self) -> Result<(BackendHandle, u64), DesktopError> {
        let (backend, epoch) = self.session.require()?;
        self.free_expired(&backend).await;
        self.session.verify_epoch(epoch)?;
        Ok((backend, epoch))
    }

    pub(super) fn status(&self) -> Result<Output, DesktopError> {
        let mut value = serde_json::to_value(self.session.status())
            .map_err(|e| DesktopError::new("DESKTOP_HELPER", e.to_string()))?;
        value["allowedApps"] = json!(self.config().allowed_apps.len());
        value["platform"] = json!(Platform::name());
        Ok(Output::Json(value))
    }

    pub(super) async fn connect(&self) -> Result<Output, DesktopError> {
        // A reconnect drops every lease, so hand their helper snapshots back first.
        if let Ok((backend, _)) = self.session.require() {
            let ids = self.session.leases.drain_all();
            self.free_snapshots(&backend, ids).await;
        }
        let capabilities = self.session.connect(&self.backend).await?;
        Ok(Output::Json(
            json!({"connected": true, "capabilities": capabilities}),
        ))
    }

    pub(super) async fn disconnect(&self) -> Result<Output, DesktopError> {
        if let Ok((backend, _)) = self.session.require() {
            let ids = self.session.leases.drain_all();
            self.free_snapshots(&backend, ids).await;
        }
        self.session.disconnect();
        Ok(Output::Json(json!({"connected": false})))
    }

    /// The allowed apps, each with whether it is running. The installed-app
    /// inventory is never listed: only apps the user allowed are visible.
    pub(super) async fn apps_tool(&self) -> Result<Output, DesktopError> {
        // A scan can take seconds on first use; keep it off the async workers.
        let apps = std::sync::Arc::clone(&self.apps);
        let catalog = tokio::task::spawn_blocking(move || apps.list())
            .await
            .unwrap_or_default();
        let apps: Vec<Value> = self
            .config()
            .allowed_apps
            .iter()
            .map(|app| {
                let running = catalog
                    .iter()
                    .any(|c| c.running && c.identity == app.identity);
                json!({"name": app.name, "running": running, "allowed": true})
            })
            .collect();
        Ok(Output::Json(untrusted(json!({"apps": apps}))))
    }

    pub(super) async fn windows(&self, _entry: &mut ActivityEntry) -> Result<Output, DesktopError> {
        let (backend, epoch) = self.connected().await?;
        let Reply::Windows(all) = backend.call_typed(Call::Windows).await? else {
            return Err(unexpected_reply("windows"));
        };
        self.session.verify_epoch(epoch)?;
        let config = self.config();
        let visible: Vec<Value> = all
            .iter()
            .filter(|w| window_visible(&config, w, self.self_pid))
            .map(public_window)
            .collect();
        Ok(Output::Json(untrusted(json!({"result": visible}))))
    }

    pub(super) async fn describe(
        &self,
        owner: &str,
        args: &Value,
        entry: &mut ActivityEntry,
    ) -> Result<Output, DesktopError> {
        let max_nodes = max_nodes_arg(args, DEFAULT_MAX_NODES);
        let interactive_only = args
            .get("interactiveOnly")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        if arg_str(args, "mode") == Some("background") {
            let window_id = arg_str(args, "windowId").ok_or_else(|| {
                DesktopError::new(
                    "DESKTOP_TARGET_CHANGED",
                    "Window ID is required for background describe",
                )
            })?;
            let snapshot = self
                .describe_background(owner, window_id, max_nodes, interactive_only, entry)
                .await?;
            return Ok(Output::Json(untrusted(json!({
                "window": snapshot.window,
                "nodes": snapshot.public_nodes,
                "truncated": snapshot.truncated,
                "snapshotId": snapshot.snapshot_id,
                "windowLeaseId": snapshot.lease_id,
                "leaseExpiresAt": snapshot.expires_at,
            }))));
        }
        self.describe_foreground(
            arg_str(args, "windowId"),
            max_nodes,
            interactive_only,
            entry,
        )
        .await
    }

    async fn describe_foreground(
        &self,
        window_id: Option<&str>,
        max_nodes: usize,
        interactive_only: bool,
        entry: &mut ActivityEntry,
    ) -> Result<Output, DesktopError> {
        let (backend, epoch) = self.connected().await?;
        let window_id = match window_id {
            Some(id) => id.to_string(),
            None => self.focused_window_id(&backend).await?,
        };
        let target = self.target_identity(&backend, &window_id).await?;
        self.authorize(&target, entry)?;
        let Reply::Describe(result) = backend
            .call_typed(Call::Describe(DescribeRequest {
                expected_target: Some(target.clone()),
                window_id: Some(window_id),
                max_nodes,
                interactive_only,
                mode: Some("foreground".to_string()),
            }))
            .await?
        else {
            return Err(unexpected_reply("describe"));
        };
        self.verify_read_result(&backend, &target, Some(&result.window), entry)
            .await?;
        let sensitive: Vec<Option<&'static str>> =
            result.nodes.iter().map(sensitive_node).collect();
        let refs = self.session.publish_foreground_refs(
            epoch,
            result
                .nodes
                .iter()
                .zip(&sensitive)
                .map(|(n, s)| ForegroundNode {
                    handle: n.handle.clone(),
                    focused: n.focused,
                    sensitive: *s,
                })
                .collect(),
        )?;
        let nodes: Vec<Value> = result
            .nodes
            .iter()
            .zip(&refs)
            .zip(&sensitive)
            .map(|((n, r), s)| public_node(n, r, *s))
            .collect();
        Ok(Output::Json(untrusted(json!({
            "window": public_window(&result.window),
            "nodes": nodes,
            "truncated": result.truncated,
        }))))
    }

    pub(crate) async fn focused_window_id(
        &self,
        backend: &BackendHandle,
    ) -> Result<String, DesktopError> {
        match backend.call_typed(Call::FocusedWindow).await? {
            Reply::Window(WindowIdentity { window_id, .. }) => Ok(window_id),
            _ => Err(unexpected_reply("focusedWindow")),
        }
    }

    pub(super) async fn capture(
        &self,
        args: &Value,
        entry: &mut ActivityEntry,
    ) -> Result<Output, DesktopError> {
        if !self.platform.capture {
            return Err(DesktopError::new(
                "DESKTOP_CAPTURE_UNSUPPORTED",
                "Screen capture is not available on this platform",
            ));
        }
        let (backend, epoch) = self.connected().await?;
        if !self
            .session
            .status()
            .capabilities
            .is_some_and(|c| c.capture)
        {
            return Err(DesktopError::new("DESKTOP_PERMISSION_REQUIRED", "Native capture is not ready. Check desktop doctor and Screen Recording/display permissions, then reconnect."));
        }
        let Some(window_id) = arg_str(args, "windowId") else {
            return Err(DesktopError::new(
                "DESKTOP_APP_NOT_ALLOWED",
                "full-screen capture is not allowed in allowlist mode",
            ));
        };
        let target = self.target_identity(&backend, window_id).await?;
        self.authorize(&target, entry)?;
        let Reply::Capture(result) = backend
            .call_typed(Call::Capture(CaptureRequest {
                expected_target: Some(target.clone()),
                window_id: Some(window_id.to_string()),
            }))
            .await?
        else {
            return Err(unexpected_reply("capture"));
        };
        self.verify_read_result(&backend, &target, result.window.as_ref(), entry)
            .await?;
        self.session.verify_epoch(epoch)?;
        let (mime, data) = split_data_uri(&result.image_data_uri)?;
        Ok(Output::Image {
            data,
            mime,
            details: untrusted(json!({
                "devicePixelRatio": result.device_pixel_ratio,
                "window": result.window.as_ref().map(public_window),
            })),
        })
    }
}

/// `data:image/jpeg;base64,AAAA` into `("image/jpeg", "AAAA")`.
fn split_data_uri(uri: &str) -> Result<(String, String), DesktopError> {
    uri.strip_prefix("data:")
        .and_then(|rest| rest.split_once(";base64,"))
        .map(|(mime, data)| (mime.to_string(), data.to_string()))
        .ok_or_else(|| DesktopError::new("DESKTOP_HELPER", "capture returned an unreadable image"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn data_uri_splits() {
        let (mime, data) = split_data_uri("data:image/jpeg;base64,Zm9v").unwrap();
        assert_eq!((mime.as_str(), data.as_str()), ("image/jpeg", "Zm9v"));
        assert!(split_data_uri("nope").is_err());
    }

    #[test]
    fn max_nodes_defaults_and_clamps() {
        assert_eq!(max_nodes_arg(&json!({}), 500), 500);
        assert_eq!(max_nodes_arg(&json!({"maxNodes": 0}), 500), 1);
        assert_eq!(max_nodes_arg(&json!({"maxNodes": 99999}), 500), 5000);
        assert_eq!(max_nodes_arg(&json!({"maxNodes": 7}), 500), 7);
    }
}
