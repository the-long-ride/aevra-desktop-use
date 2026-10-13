//! `desktop_act_many`: an ordered batch of semantic background actions on one
//! window. Every step goes through the same gate and lease checks as the
//! single-action tools; the batch only adds up-front validation, locator
//! resolution and ordering.

use super::background::{BackgroundSnapshot, BgTarget};
use super::{arg_str, Output, Server};
use desktop_core::DesktopError;
use desktop_policy::activity::ActivityEntry;
use desktop_policy::sensitive::secret_shaped;
use serde_json::{json, Map, Value};
use std::time::{Duration, Instant};

const MAX_SNAPSHOT_NODES: usize = 2000;
const DEFAULT_STEP_TIMEOUT_MS: u64 = 5_000;
const DEFAULT_DEADLINE_MS: u64 = 30_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Op {
    Invoke,
    Click,
    SetValue,
    Type,
    Select,
    SetToggleState,
}

impl Op {
    fn parse(op: &str) -> Option<Self> {
        Some(match op {
            "invoke" => Self::Invoke,
            "click" => Self::Click,
            "setValue" => Self::SetValue,
            "type" => Self::Type,
            "select" => Self::Select,
            "setToggleState" => Self::SetToggleState,
            _ => return None,
        })
    }

    /// The operation name the helper understands. `setToggleState` is a
    /// `toggle` once the current state is known to differ.
    fn helper_op(self) -> &'static str {
        match self {
            Self::Invoke | Self::Click => "invoke",
            Self::SetValue | Self::Type => "setValue",
            Self::Select => "select",
            Self::SetToggleState => "toggle",
        }
    }
}

enum StepTarget {
    Ref(String),
    Locator { role: String, name: String },
}

struct Step {
    id: Option<String>,
    op: Op,
    target: StepTarget,
    /// `value` for setValue, `text` for type.
    value: Option<String>,
    /// Requested state for setToggleState.
    state: String,
    timeout: Duration,
}

fn invalid(index: usize, message: impl Into<String>) -> DesktopError {
    DesktopError::new("INVALID_REQUEST", message).with_details(json!({"stepIndex": index}))
}

/// Parses every step and refuses secret-shaped data before anything runs.
fn parse_steps(args: &Value) -> Result<Vec<Step>, DesktopError> {
    let actions = args
        .get("actions")
        .and_then(Value::as_array)
        .ok_or_else(|| DesktopError::new("INVALID_REQUEST", "actions must be an array"))?;
    let mut steps = Vec::with_capacity(actions.len());
    for (index, raw) in actions.iter().enumerate() {
        let op_name = arg_str(raw, "op").unwrap_or_default();
        let op = Op::parse(op_name).ok_or_else(|| {
            invalid(
                index,
                format!("Unsupported desktop batch action: {op_name}"),
            )
        })?;
        let target = if let Some(reference) = arg_str(raw, "ref").filter(|r| !r.is_empty()) {
            StepTarget::Ref(reference.to_string())
        } else if let (Some(role), Some(name)) = (
            raw.pointer("/locator/role").and_then(Value::as_str),
            raw.pointer("/locator/name").and_then(Value::as_str),
        ) {
            StepTarget::Locator {
                role: role.to_string(),
                name: name.to_string(),
            }
        } else {
            return Err(invalid(
                index,
                "Each desktop batch action requires ref or locator",
            ));
        };
        let value = match op {
            Op::Type => Some(arg_str(raw, "text").unwrap_or_default().to_string()),
            Op::SetValue | Op::Select => arg_str(raw, "value").map(str::to_string),
            _ => None,
        };
        if value.as_deref().is_some_and(secret_shaped) {
            return Err(invalid(
                index,
                format!(
                    "Aevra will not send secret-shaped data through desktop_act_many ({op_name})"
                ),
            ));
        }
        let timeout_ms = raw
            .get("timeoutMs")
            .and_then(Value::as_f64)
            .map(|n| n as u64)
            .unwrap_or(DEFAULT_STEP_TIMEOUT_MS)
            .clamp(50, 30_000);
        steps.push(Step {
            id: arg_str(raw, "id").map(str::to_string),
            op,
            target,
            value,
            state: if arg_str(raw, "state") == Some("off") {
                "off".to_string()
            } else {
                "on".to_string()
            },
            timeout: Duration::from_millis(timeout_ms),
        });
    }
    Ok(steps)
}

/// The ref a step points at: either given, or the one node matching exactly.
fn resolve<'a>(
    snapshot: &'a BackgroundSnapshot,
    target: &StepTarget,
) -> Result<&'a super::background::SnapNode, DesktopError> {
    match target {
        StepTarget::Ref(reference) => snapshot
            .nodes
            .iter()
            .find(|n| &n.reference == reference)
            .ok_or_else(|| {
                DesktopError::new(
                    "DESKTOP_REF_STALE",
                    "ref is not part of the current snapshot; use a locator in a batch",
                )
            }),
        StepTarget::Locator { role, name } => {
            let mut matches = snapshot
                .nodes
                .iter()
                .filter(|n| &n.role == role && &n.name == name);
            match (matches.next(), matches.next()) {
                (Some(only), None) => Ok(only),
                (None, _) => Err(DesktopError::new(
                    "DESKTOP_REF_STALE",
                    "locator matched 0 nodes",
                )),
                (Some(_), Some(_)) => {
                    let count = snapshot
                        .nodes
                        .iter()
                        .filter(|n| &n.role == role && &n.name == name)
                        .count();
                    Err(DesktopError::new(
                        "DESKTOP_REF_STALE",
                        format!("locator matched {count} nodes"),
                    ))
                }
            }
        }
    }
}

fn result_entry(step: &Step, index: usize) -> Map<String, Value> {
    let mut m = Map::new();
    if let Some(id) = &step.id {
        m.insert("id".into(), json!(id));
    }
    m.insert("index".into(), json!(index));
    m
}

fn failed(step: &Step, index: usize, error: &DesktopError) -> Value {
    let mut m = result_entry(step, index);
    m.insert("ok".into(), json!(false));
    m.insert("code".into(), json!(error.code));
    m.insert("message".into(), json!(error.message));
    Value::Object(m)
}

fn skipped(step: &Step, index: usize, reason: &str) -> Value {
    let mut m = result_entry(step, index);
    m.insert("ok".into(), json!(false));
    m.insert("outcome".into(), json!("skipped"));
    m.insert("reason".into(), json!(reason));
    Value::Object(m)
}

impl Server {
    pub(super) async fn act_many(
        &self,
        owner: &str,
        args: &Value,
        entry: &mut ActivityEntry,
    ) -> Result<Output, DesktopError> {
        let window_id = arg_str(args, "windowId")
            .filter(|w| !w.is_empty())
            .ok_or_else(|| {
                DesktopError::new("INVALID_REQUEST", "desktop_act_many requires windowId")
            })?;
        let steps = parse_steps(args)?;
        let stop_on_error = args
            .get("stopOnError")
            .and_then(Value::as_bool)
            .unwrap_or(true);
        let deadline = Duration::from_millis(
            args.get("deadlineMs")
                .and_then(Value::as_f64)
                .map(|n| n as u64)
                .unwrap_or(DEFAULT_DEADLINE_MS)
                .clamp(1_000, 60_000),
        );
        let started = Instant::now();

        // Observe first: a refusal here (not allowed, own window, busy) fails
        // the whole call before anything is touched.
        let mut snapshot = Some(
            self.describe_background(owner, window_id, MAX_SNAPSHOT_NODES, false, entry)
                .await?,
        );

        let mut results: Vec<Value> = Vec::with_capacity(steps.len());
        let mut stopped_at: Option<usize> = None;
        let mut all_ok = true;
        let mut first_failure: Option<String> = None;

        for (index, step) in steps.iter().enumerate() {
            if started.elapsed() >= deadline {
                stopped_at.get_or_insert(index);
                all_ok = false;
                results.push(skipped(step, index, "deadline"));
                continue;
            }
            if stopped_at.is_some() {
                results.push(skipped(step, index, "stopped"));
                continue;
            }

            // A spent snapshot is replaced just in time, so a skipped step
            // costs no describe.
            if snapshot.is_none() {
                match self
                    .describe_background(owner, window_id, MAX_SNAPSHOT_NODES, false, entry)
                    .await
                {
                    Ok(next) => snapshot = Some(next),
                    Err(error) => {
                        record_failure(&mut first_failure, &error);
                        results.push(failed(step, index, &error));
                        all_ok = false;
                        if stop_on_error {
                            stopped_at = Some(index);
                        }
                        continue;
                    }
                }
            }
            let current = snapshot.as_ref().expect("snapshot was just ensured");

            let outcome = self.run_step(owner, step, current, entry).await;
            match outcome {
                Ok(StepOutcome::AlreadyInState) => {
                    let mut m = result_entry(step, index);
                    m.insert("ok".into(), json!(true));
                    m.insert("outcome".into(), json!("skipped"));
                    m.insert("reason".into(), json!("alreadyInState"));
                    results.push(Value::Object(m));
                }
                Ok(StepOutcome::Completed {
                    toggle_state,
                    focus_changed,
                }) => {
                    // The act spent the snapshot; the next step describes again.
                    snapshot = None;
                    let mut m = result_entry(step, index);
                    m.insert("ok".into(), json!(true));
                    m.insert("outcome".into(), json!("completed"));
                    if let Some(state) = toggle_state {
                        m.insert("toggleState".into(), json!(state));
                    }
                    if focus_changed {
                        m.insert("focusChanged".into(), json!(true));
                    }
                    results.push(Value::Object(m));
                }
                Err(StepFailure { error, spent }) => {
                    if spent {
                        snapshot = None;
                    }
                    record_failure(&mut first_failure, &error);
                    results.push(failed(step, index, &error));
                    all_ok = false;
                    if stop_on_error {
                        stopped_at = Some(index);
                    }
                }
            }
        }
        entry.code = first_failure;

        let mut out = Map::new();
        if let Some(id) = arg_str(args, "requestId") {
            out.insert("requestId".into(), json!(id));
        }
        out.insert("results".into(), Value::Array(results));
        out.insert("completed".into(), json!(all_ok));
        if let Some(index) = stopped_at {
            out.insert("stoppedAt".into(), json!(index));
        }
        Ok(Output::Json(Value::Object(out)))
    }

    async fn run_step(
        &self,
        owner: &str,
        step: &Step,
        snapshot: &BackgroundSnapshot,
        entry: &mut ActivityEntry,
    ) -> Result<StepOutcome, StepFailure> {
        let node = resolve(snapshot, &step.target).map_err(|error| StepFailure {
            error,
            spent: false,
        })?;
        if step.op == Op::SetToggleState {
            match node.toggle_state.as_deref() {
                Some(state) if state == step.state => return Ok(StepOutcome::AlreadyInState),
                Some(_) => {}
                None => {
                    return Err(StepFailure {
                        error: DesktopError::new(
                            "DESKTOP_PATTERN_UNSUPPORTED",
                            "node does not expose a toggle state",
                        ),
                        spent: false,
                    })
                }
            }
        }
        let target = BgTarget {
            window_id: &snapshot.window_id,
            lease_id: &snapshot.lease_id,
            snapshot_id: &snapshot.snapshot_id,
            reference: &node.reference,
        };
        let value = match step.op {
            Op::SetValue | Op::Type => step.value.as_deref(),
            _ => None,
        };
        let acted = tokio::time::timeout(
            step.timeout,
            self.act_background(owner, step.op.helper_op(), &target, value, entry),
        )
        .await;
        match acted {
            Ok(Ok(done)) => {
                if step.op == Op::SetToggleState {
                    if let Some(now) = done.toggle_state.as_deref() {
                        if now != step.state {
                            return Err(StepFailure {
                                error: DesktopError::new(
                                    "DESKTOP_HELPER",
                                    format!("toggle ended in state {now}, not {}", step.state),
                                ),
                                spent: true,
                            });
                        }
                    }
                }
                Ok(StepOutcome::Completed {
                    toggle_state: done.toggle_state,
                    focus_changed: done.focus_changed,
                })
            }
            Ok(Err(error)) => {
                // Anything that got as far as the helper spent the snapshot;
                // earlier refusals did not.
                let spent = !matches!(
                    error.code.as_str(),
                    "DESKTOP_REF_STALE"
                        | "DESKTOP_LEASE_EXPIRED"
                        | "DESKTOP_APP_NOT_ALLOWED"
                        | "DESKTOP_INPUT_REFUSED"
                        | "DESKTOP_TARGET_CHANGED"
                        | "DESKTOP_WINDOW_BUSY"
                );
                Err(StepFailure { error, spent })
            }
            Err(_elapsed) => {
                // The dispatched call keeps running on the backend thread, so
                // the snapshot cannot be trusted and the outcome is unknown.
                self.session.leases.invalidate_snapshot(&snapshot.lease_id);
                Err(StepFailure {
                    error: DesktopError::new(
                        "DESKTOP_OUTCOME_UNKNOWN",
                        "Desktop action outcome is unknown: the step timed out after dispatch",
                    ),
                    spent: true,
                })
            }
        }
    }
}

enum StepOutcome {
    Completed {
        toggle_state: Option<String>,
        focus_changed: bool,
    },
    AlreadyInState,
}

struct StepFailure {
    error: DesktopError,
    /// The snapshot was consumed and the next step must describe again.
    spent: bool,
}

/// The first failure's code goes on the activity entry, so a refused step
/// shows up as a denial in the log even though the batch itself returned.
fn record_failure(first: &mut Option<String>, error: &DesktopError) {
    if first.is_none() {
        *first = Some(error.code.clone());
    }
}
