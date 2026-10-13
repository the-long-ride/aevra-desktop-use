//! Background semantic actions on leased windows: describe, invoke, set value,
//! select, toggle and release.

use super::{
    arg_str, map_after_dispatch, public_node, public_window, unexpected_reply, Output, Server,
};
use crate::leases::BoundNode;
use desktop_core::{
    BackgroundActRequest, Call, DescribeBackgroundRequest, DesktopError, Reply, VerifiedWindowHost,
    WindowInstance,
};
use desktop_policy::activity::ActivityEntry;
use desktop_policy::gate::canonical_path;
use desktop_policy::sensitive::{secret_shaped, sensitive_node};
use serde_json::{json, Value};

/// One node of a background snapshot, as the batch runner needs it.
#[derive(Debug, Clone)]
pub(crate) struct SnapNode {
    pub reference: String,
    pub role: String,
    pub name: String,
    pub toggle_state: Option<String>,
}

pub(crate) struct BackgroundSnapshot {
    pub window_id: String,
    pub window: Value,
    pub public_nodes: Vec<Value>,
    pub nodes: Vec<SnapNode>,
    pub truncated: bool,
    pub snapshot_id: String,
    pub lease_id: String,
    pub expires_at: String,
}

pub(crate) struct BgTarget<'a> {
    pub window_id: &'a str,
    pub lease_id: &'a str,
    pub snapshot_id: &'a str,
    pub reference: &'a str,
}

pub(crate) struct BackgroundOutcome {
    pub json: Value,
    pub focus_changed: bool,
    pub toggle_state: Option<String>,
}

fn same_host(a: Option<&VerifiedWindowHost>, b: Option<&VerifiedWindowHost>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(a), Some(b)) => {
            a.instance == b.instance
                && canonical_path(&a.executable_path) == canonical_path(&b.executable_path)
        }
        _ => false,
    }
}

fn same_instance(a: &WindowInstance, b: &WindowInstance) -> bool {
    a == b
}

fn helper_op(tool: &str) -> Option<&'static str> {
    match tool {
        "desktop_invoke" => Some("invoke"),
        "desktop_set_value" => Some("setValue"),
        "desktop_select" => Some("select"),
        "desktop_toggle" => Some("toggle"),
        _ => None,
    }
}

impl Server {
    /// Gate, lease, describe, re-check, bind. Returns the snapshot with refs.
    pub(crate) async fn describe_background(
        &self,
        owner: &str,
        window_id: &str,
        max_nodes: usize,
        interactive_only: bool,
        entry: &mut ActivityEntry,
    ) -> Result<BackgroundSnapshot, DesktopError> {
        let (backend, epoch) = self.connected().await?;
        let target = self.target_identity(&backend, window_id).await?;
        let app_id = self.authorize(&target, entry)?;

        let acquired = self.session.leases.acquire(
            owner,
            &target.window_instance,
            epoch,
            target.host_application.as_ref(),
            &app_id,
        )?;
        let snapshot_id = uuid::Uuid::new_v4().to_string();

        // Anything that goes wrong from here ends the lease and frees the helper snapshot.
        let abandon = |extra: Option<String>| {
            let mut ids: Vec<String> = extra.into_iter().collect();
            if let Some(released) = self.session.leases.release_lease(&acquired.lease_id) {
                ids.extend(released.snapshot_id);
            }
            ids
        };

        let described = match backend
            .call_typed(Call::DescribeBackground(DescribeBackgroundRequest {
                window_id: window_id.to_string(),
                snapshot_id: snapshot_id.clone(),
                max_nodes,
                interactive_only,
            }))
            .await
        {
            Ok(Reply::DescribeBackground(d)) => d,
            Ok(_) => {
                let ids = abandon(Some(snapshot_id));
                self.free_snapshots(&backend, ids).await;
                return Err(unexpected_reply("describeBackground"));
            }
            Err(e) => {
                let ids = abandon(Some(snapshot_id));
                self.free_snapshots(&backend, ids).await;
                return Err(e);
            }
        };

        if !same_instance(&described.window_instance, &target.window_instance)
            || !same_host(
                described.host_application.as_ref(),
                target.host_application.as_ref(),
            )
        {
            let ids = abandon(Some(snapshot_id));
            self.free_snapshots(&backend, ids).await;
            return Err(DesktopError::new(
                "DESKTOP_TARGET_CHANGED",
                "Target or verified host changed while describing the background window",
            ));
        }

        let sensitive: Vec<Option<&'static str>> =
            described.nodes.iter().map(sensitive_node).collect();
        let bound = match self.session.leases.bind(
            owner,
            &acquired.lease_id,
            &snapshot_id,
            epoch,
            described
                .nodes
                .iter()
                .zip(&sensitive)
                .map(|(n, s)| BoundNode {
                    handle: n.handle.clone(),
                    sensitive: *s,
                })
                .collect(),
        ) {
            Ok(b) => b,
            Err(e) => {
                let ids = abandon(Some(snapshot_id));
                self.free_snapshots(&backend, ids).await;
                return Err(e);
            }
        };
        if let Some(old) = bound.superseded {
            self.free_snapshots(&backend, vec![old]).await;
        }

        let public_nodes = described
            .nodes
            .iter()
            .zip(&bound.refs)
            .zip(&sensitive)
            .map(|((n, r), s)| public_node(n, r, *s))
            .collect();
        let nodes = described
            .nodes
            .iter()
            .zip(&bound.refs)
            .map(|(n, r)| SnapNode {
                reference: r.clone(),
                role: n.role.clone(),
                name: n.name.clone(),
                toggle_state: n.toggle_state.clone(),
            })
            .collect();
        Ok(BackgroundSnapshot {
            window_id: window_id.to_string(),
            window: public_window(&described.window),
            public_nodes,
            nodes,
            truncated: described.truncated,
            snapshot_id,
            lease_id: acquired.lease_id.clone(),
            expires_at: acquired.expires_at(),
        })
    }

    pub(super) async fn act_background_tool(
        &self,
        owner: &str,
        name: &str,
        args: &Value,
        entry: &mut ActivityEntry,
    ) -> Result<Output, DesktopError> {
        let op = helper_op(name).ok_or_else(|| {
            DesktopError::new(
                "INVALID_REQUEST",
                format!("Unknown background tool: {name}"),
            )
        })?;
        let target = BgTarget {
            window_id: arg_str(args, "windowId").unwrap_or_default(),
            lease_id: arg_str(args, "windowLeaseId").unwrap_or_default(),
            snapshot_id: arg_str(args, "snapshotId").unwrap_or_default(),
            reference: arg_str(args, "ref").unwrap_or_default(),
        };
        let outcome = self
            .act_background(owner, op, &target, arg_str(args, "value"), entry)
            .await?;
        Ok(Output::Json(outcome.json))
    }

    /// Resolve the ref, re-read the live target, run the gate against the
    /// current config, then dispatch. The order is deliberate: nothing reaches
    /// the helper before the live identity is checked against the gate.
    pub(crate) async fn act_background(
        &self,
        owner: &str,
        op: &str,
        t: &BgTarget<'_>,
        value: Option<&str>,
        entry: &mut ActivityEntry,
    ) -> Result<BackgroundOutcome, DesktopError> {
        let (backend, epoch) = self.connected().await?;

        // 1. Owner, lease, snapshot, epoch and ref.
        let resolved = self.session.leases.resolve(
            owner,
            t.window_id,
            t.lease_id,
            t.snapshot_id,
            t.reference,
            epoch,
        )?;

        // 2. The live target must be the one the lease was acquired for.
        let live = self
            .target_identity(&backend, &resolved.instance.window_id)
            .await?;
        if !same_instance(&live.window_instance, &resolved.instance)
            || !same_host(live.host_application.as_ref(), resolved.host.as_ref())
        {
            return Err(DesktopError::new(
                "DESKTOP_TARGET_CHANGED",
                "Target or verified host process instance changed",
            ));
        }

        // 3. The gate against the *current* config. A refusal ends the lease.
        if let Err(refusal) = self.authorize(&live, entry) {
            if let Some(released) = self.session.leases.release_lease(&resolved.lease_id) {
                self.free_snapshots(&backend, released.snapshot_id.into_iter().collect())
                    .await;
            }
            return Err(refusal);
        }

        // 4. Hard refusals, before the helper is called.
        if op == "setValue" {
            if let Some(kind) = resolved.sensitive {
                return Err(DesktopError::new(
                    "DESKTOP_INPUT_REFUSED",
                    format!("Refusing to set the value of a {kind} field"),
                ));
            }
            if value.is_some_and(secret_shaped) {
                return Err(DesktopError::new(
                    "DESKTOP_INPUT_REFUSED",
                    "Aevra will not send secret-shaped data through desktop_set_value",
                ));
            }
        }

        // 5. Dispatch. From here on the outcome may be unknown.
        let result = backend
            .call_typed(Call::BackgroundAct(BackgroundActRequest {
                snapshot_id: resolved.snapshot_id.clone(),
                handle: resolved.handle.clone(),
                op: op.to_string(),
                value: if op == "setValue" {
                    value.map(str::to_string)
                } else {
                    None
                },
                expected_instance: resolved.instance.clone(),
                expected_host: resolved.host.clone(),
            }))
            .await;
        // The snapshot is spent whether or not the action worked.
        self.session.leases.invalidate_snapshot(&resolved.lease_id);
        let acted = match result {
            Ok(Reply::BackgroundAct(r)) => r,
            Ok(_) => return Err(unexpected_reply("backgroundAct")),
            Err(e) => return Err(map_after_dispatch(e)),
        };
        if acted.focus_changed {
            self.session.leases.suspend(&resolved.lease_id);
        }

        let mut json = json!({
            "ok": true,
            "outcome": "completed",
            "window": public_window(&live.window),
            "snapshotInvalidated": true,
            "requiresDescribe": true,
            "focusChanged": acted.focus_changed,
        });
        if let Some(state) = &acted.toggle_state {
            json["toggleState"] = json!(state);
        }
        Ok(BackgroundOutcome {
            json,
            focus_changed: acted.focus_changed,
            toggle_state: acted.toggle_state,
        })
    }

    pub(super) async fn release_window(
        &self,
        owner: &str,
        args: &Value,
    ) -> Result<Output, DesktopError> {
        let (backend, _) = self.connected().await?;
        let released = self
            .session
            .leases
            .release(
                owner,
                arg_str(args, "windowId").unwrap_or_default(),
                arg_str(args, "windowLeaseId").unwrap_or_default(),
            )
            .ok_or_else(|| {
                DesktopError::new(
                    "DESKTOP_LEASE_EXPIRED",
                    "Window lease not found, already expired, or owner mismatch",
                )
            })?;
        // The helper's snapshot id, not the lease id: aevra passes the lease id
        // here and so never frees the native snapshot.
        self.free_snapshots(&backend, released.snapshot_id.into_iter().collect())
            .await;
        Ok(Output::Json(json!({"ok": true, "released": true})))
    }
}
