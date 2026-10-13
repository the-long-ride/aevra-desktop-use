mod common;

use common::*;
use serde_json::{json, Value};

fn form_window() -> FakeWindow {
    window(
        "1",
        NOTEPAD,
        100,
        vec![
            node("button", "Save"),
            node("button", "Cancel"),
            node("button", "Twin"),
            node("button", "Twin"),
            {
                let mut n = node("edit", "Notes");
                n.value = Some(String::new());
                n
            },
            {
                let mut n = node("edit", "Password");
                n.value = Some(String::new());
                n
            },
            {
                let mut n = node("checkbox", "Dark mode");
                n.toggle_state = Some("on".into());
                n
            },
            {
                let mut n = node("checkbox", "Sync");
                n.toggle_state = Some("off".into());
                n
            },
        ],
    )
}

fn harness_with_form() -> Harness {
    harness(
        vec![form_window()],
        vec![allow("app_notepad", "Notepad", NOTEPAD)],
    )
}

async fn batch(h: &Harness, actions: Value, extra: Value) -> rmcp::model::CallToolResult {
    let mut args = json!({"windowId": "1", "actions": actions});
    for (k, v) in extra.as_object().cloned().unwrap_or_default() {
        args[k] = v;
    }
    call(h, "desktop_act_many", args).await
}

fn by_locator(op: &str, name: &str) -> Value {
    json!({"op": op, "locator": {"role": "button", "name": name}})
}

#[tokio::test]
async fn twenty_one_actions_are_refused_by_the_schema() {
    let h = harness_with_form();
    connect(&h).await;
    let actions: Vec<Value> = (0..21).map(|_| by_locator("invoke", "Save")).collect();
    let r = batch(&h, json!(actions), json!({})).await;
    assert_eq!(error_code(&r), "INVALID_REQUEST");
    assert!(h.state.lock().unwrap().background_acts.is_empty());
    // Twenty is fine.
    let ok: Vec<Value> = (0..20).map(|_| by_locator("invoke", "Save")).collect();
    let r = batch(&h, json!(ok), json!({})).await;
    assert_eq!(text_json(&r)["completed"], true);
}

#[tokio::test]
async fn a_secret_in_a_later_step_refuses_the_whole_batch_before_step_one_runs() {
    let h = harness_with_form();
    connect(&h).await;
    let r = batch(
        &h,
        json!([
            by_locator("invoke", "Save"),
            {"op": "type", "locator": {"role": "edit", "name": "Notes"}, "text": "fine"},
            {"op": "setValue", "locator": {"role": "edit", "name": "Notes"}, "value": "4111 1111 1111 1111"},
        ]),
        json!({}),
    )
    .await;
    assert_eq!(error_code(&r), "INVALID_REQUEST");
    assert_eq!(text_json(&r)["error"]["details"]["stepIndex"], 2);
    let state = h.state.lock().unwrap();
    assert!(state.background_acts.is_empty());
    // Not even a describe happened.
    assert!(!state.calls.contains(&"describeBackground".to_string()));
    drop(state);
    assert!(!activity_lines(&h).join("\n").contains("4111"));
}

#[tokio::test]
async fn a_token_in_type_text_is_refused_too() {
    let h = harness_with_form();
    connect(&h).await;
    let token = format!("ghp_{}", "a".repeat(36));
    let r = batch(
        &h,
        json!([{"op": "type", "locator": {"role": "edit", "name": "Notes"}, "text": token}]),
        json!({}),
    )
    .await;
    assert_eq!(error_code(&r), "INVALID_REQUEST");
    assert!(h.state.lock().unwrap().background_acts.is_empty());
}

#[tokio::test]
async fn invalid_steps_are_refused_with_their_index() {
    let h = harness_with_form();
    connect(&h).await;
    let r = batch(
        &h,
        json!([by_locator("invoke", "Save"), {"op": "explode", "ref": "ref_1"}]),
        json!({}),
    )
    .await;
    assert_eq!(error_code(&r), "INVALID_REQUEST");
    assert_eq!(text_json(&r)["error"]["details"]["stepIndex"], 1);
    let r = batch(&h, json!([{"op": "invoke"}]), json!({})).await;
    assert_eq!(error_code(&r), "INVALID_REQUEST");
}

#[tokio::test]
async fn steps_run_in_order_and_describe_again_after_each_act() {
    let h = harness_with_form();
    connect(&h).await;
    let r = batch(
        &h,
        json!([
            {"id": "save", "op": "click", "locator": {"role": "button", "name": "Save"}},
            {"id": "notes", "op": "type", "locator": {"role": "edit", "name": "Notes"}, "text": "hello"},
            {"id": "cancel", "op": "invoke", "locator": {"role": "button", "name": "Cancel"}},
        ]),
        json!({"requestId": "req-1"}),
    )
    .await;
    let v = text_json(&r);
    assert_eq!(v["requestId"], "req-1");
    assert_eq!(v["completed"], true);
    assert!(v.get("stoppedAt").is_none());
    let results = v["results"].as_array().unwrap();
    assert_eq!(results.len(), 3);
    assert_eq!(results[0]["id"], "save");
    assert_eq!(results[1]["index"], 1);
    assert!(results
        .iter()
        .all(|r| r["ok"] == true && r["outcome"] == "completed"));

    let s = h.state.lock().unwrap();
    let ops: Vec<_> = s.background_acts.iter().map(|a| a.op.as_str()).collect();
    assert_eq!(ops, ["invoke", "setValue", "invoke"]);
    assert_eq!(s.background_acts[1].value.as_deref(), Some("hello"));
    // One describe up front plus one before each following step.
    let describes = s
        .calls
        .iter()
        .filter(|c| *c == "describeBackground")
        .count();
    assert_eq!(describes, 3);
    // Every snapshot was freed or superseded: nothing is left dangling.
    assert_eq!(s.released_snapshots.len(), 2);
}

#[tokio::test]
async fn an_ambiguous_locator_stops_the_run_when_stop_on_error() {
    let h = harness_with_form();
    connect(&h).await;
    let r = batch(
        &h,
        json!([
            by_locator("invoke", "Save"),
            by_locator("invoke", "Twin"),
            by_locator("invoke", "Cancel"),
        ]),
        json!({}),
    )
    .await;
    let v = text_json(&r);
    assert_eq!(v["completed"], false);
    assert_eq!(v["stoppedAt"], 1);
    let results = v["results"].as_array().unwrap();
    assert_eq!(results[0]["ok"], true);
    assert_eq!(results[1]["ok"], false);
    assert_eq!(results[1]["code"], "DESKTOP_REF_STALE");
    assert!(results[1]["message"]
        .as_str()
        .unwrap()
        .contains("matched 2 nodes"));
    assert_eq!(results[2]["outcome"], "skipped");
    assert_eq!(results[2]["reason"], "stopped");
    assert_eq!(h.state.lock().unwrap().background_acts.len(), 1);
}

#[tokio::test]
async fn a_missing_locator_matches_zero_nodes_and_stop_on_error_false_continues() {
    let h = harness_with_form();
    connect(&h).await;
    let r = batch(
        &h,
        json!([
            by_locator("invoke", "Nope"),
            by_locator("invoke", "Twin"),
            by_locator("invoke", "Cancel"),
        ]),
        json!({"stopOnError": false}),
    )
    .await;
    let v = text_json(&r);
    assert_eq!(v["completed"], false);
    assert!(v.get("stoppedAt").is_none());
    let results = v["results"].as_array().unwrap();
    assert!(results[0]["message"]
        .as_str()
        .unwrap()
        .contains("matched 0 nodes"));
    assert_eq!(results[1]["code"], "DESKTOP_REF_STALE");
    assert_eq!(results[2]["ok"], true);
    assert_eq!(h.state.lock().unwrap().background_acts.len(), 1);
}

#[tokio::test]
async fn a_toggle_already_in_the_requested_state_is_skipped() {
    let h = harness_with_form();
    connect(&h).await;
    h.state.lock().unwrap().toggle_state = Some("on".into());
    let r = batch(
        &h,
        json!([
            {"op": "setToggleState", "state": "on", "locator": {"role": "checkbox", "name": "Dark mode"}},
            {"op": "setToggleState", "state": "on", "locator": {"role": "checkbox", "name": "Sync"}},
        ]),
        json!({}),
    )
    .await;
    let v = text_json(&r);
    assert_eq!(v["completed"], true);
    let results = v["results"].as_array().unwrap();
    assert_eq!(results[0]["ok"], true);
    assert_eq!(results[0]["outcome"], "skipped");
    assert_eq!(results[0]["reason"], "alreadyInState");
    assert_eq!(results[1]["outcome"], "completed");
    assert_eq!(results[1]["toggleState"], "on");
    let s = h.state.lock().unwrap();
    assert_eq!(s.background_acts.len(), 1);
    assert_eq!(s.background_acts[0].op, "toggle");
}

#[tokio::test]
async fn a_toggle_that_lands_in_the_wrong_state_is_a_failure() {
    let h = harness_with_form();
    connect(&h).await;
    h.state.lock().unwrap().toggle_state = Some("indeterminate".into());
    let r = batch(
        &h,
        json!([{"op": "setToggleState", "state": "on", "locator": {"role": "checkbox", "name": "Sync"}}]),
        json!({}),
    )
    .await;
    let v = text_json(&r);
    assert_eq!(v["results"][0]["ok"], false);
    assert_eq!(v["stoppedAt"], 0);
}

#[tokio::test]
async fn the_deadline_marks_remaining_steps_skipped() {
    let h = harness_with_form();
    connect(&h).await;
    h.state.lock().unwrap().act_delay_ms = 600;
    let r = batch(
        &h,
        json!([
            by_locator("invoke", "Save"),
            by_locator("invoke", "Cancel"),
            by_locator("invoke", "Save"),
            by_locator("invoke", "Cancel"),
        ]),
        json!({"deadlineMs": 1000}),
    )
    .await;
    let v = text_json(&r);
    let results = v["results"].as_array().unwrap();
    assert_eq!(results[0]["ok"], true);
    assert_eq!(results[1]["ok"], true);
    for r in &results[2..] {
        assert_eq!(r["outcome"], "skipped");
        assert_eq!(r["reason"], "deadline");
        assert_eq!(r["ok"], false);
    }
    assert_eq!(v["completed"], false);
    assert_eq!(v["stoppedAt"], 2);
    assert_eq!(h.state.lock().unwrap().background_acts.len(), 2);
}

#[tokio::test]
async fn a_step_that_times_out_has_an_unknown_outcome_and_spends_the_snapshot() {
    let h = harness_with_form();
    connect(&h).await;
    h.state.lock().unwrap().act_delay_ms = 400;
    let r = batch(
        &h,
        json!([
            {"op": "invoke", "locator": {"role": "button", "name": "Save"}, "timeoutMs": 50},
            by_locator("invoke", "Cancel"),
        ]),
        json!({}),
    )
    .await;
    let v = text_json(&r);
    assert_eq!(v["results"][0]["code"], "DESKTOP_OUTCOME_UNKNOWN");
    assert_eq!(v["results"][1]["outcome"], "skipped");
    assert_eq!(v["stoppedAt"], 0);
}

#[tokio::test]
async fn a_sensitive_field_step_is_refused_and_logged_as_a_denial_without_the_value() {
    let h = harness_with_form();
    connect(&h).await;
    let secret = "correct-horse-battery";
    let r = batch(
        &h,
        json!([{"op": "type", "locator": {"role": "edit", "name": "Password"}, "text": secret}]),
        json!({}),
    )
    .await;
    let v = text_json(&r);
    assert_eq!(v["results"][0]["code"], "DESKTOP_INPUT_REFUSED");
    assert!(h.state.lock().unwrap().background_acts.is_empty());
    let log = activity_lines(&h);
    assert!(!log.join("\n").contains(secret));
    let last: Value = serde_json::from_str(log.last().unwrap()).unwrap();
    assert_eq!(last["tool"], "desktop_act_many");
    assert_eq!(last["verdict"], "deny");
}

#[tokio::test]
async fn a_window_that_is_not_allowed_fails_the_whole_call() {
    let h = harness(vec![form_window()], vec![]);
    connect(&h).await;
    let r = batch(&h, json!([by_locator("invoke", "Save")]), json!({})).await;
    assert_eq!(error_code(&r), "DESKTOP_APP_NOT_ALLOWED");
    assert!(!calls(&h).contains(&"describeBackground".to_string()));
}

#[tokio::test]
async fn a_stale_caller_ref_is_refused() {
    let h = harness_with_form();
    connect(&h).await;
    let r = batch(&h, json!([{"op": "invoke", "ref": "ref_9_9"}]), json!({})).await;
    let v = text_json(&r);
    assert_eq!(v["results"][0]["code"], "DESKTOP_REF_STALE");
}

#[tokio::test]
async fn the_batch_needs_a_connection() {
    let h = harness_with_form();
    let r = batch(&h, json!([by_locator("invoke", "Save")]), json!({})).await;
    assert_eq!(error_code(&r), "DESKTOP_NOT_CONNECTED");
}
