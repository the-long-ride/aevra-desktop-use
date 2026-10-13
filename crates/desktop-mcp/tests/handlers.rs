mod common;

use common::*;
use desktop_core::{VerifiedWindowHost, WindowInstance};
use desktop_policy::Config;
use serde_json::json;

fn notepad_window(id: &str) -> FakeWindow {
    window(
        id,
        NOTEPAD,
        100,
        vec![
            node("button", "Save"),
            {
                let mut n = node("edit", "Notes");
                n.value = Some("hello".into());
                n
            },
            {
                let mut n = node("edit", "Password");
                n.value = Some("hunter2".into());
                n
            },
        ],
    )
}

fn harness_with_notepad() -> Harness {
    harness(
        vec![
            notepad_window("1"),
            window("2", OTHER, 200, vec![node("button", "Go")]),
        ],
        vec![allow("app_notepad", "Notepad", NOTEPAD)],
    )
}

#[tokio::test]
async fn tools_need_a_connection() {
    let h = harness_with_notepad();
    let r = call(&h, "desktop_windows", json!({})).await;
    assert_eq!(error_code(&r), "DESKTOP_NOT_CONNECTED");
    // Status works without one.
    let status = text_json(&call(&h, "desktop_status", json!({})).await);
    assert_eq!(status["connected"], false);
    assert_eq!(status["allowedApps"], 1);
}

#[tokio::test]
async fn unknown_tools_and_bad_arguments_are_invalid_requests() {
    let h = harness_with_notepad();
    assert_eq!(
        error_code(
            &call(
                &h,
                "desktop_request_access",
                json!({"windowId": "1", "duration": "session"})
            )
            .await
        ),
        "INVALID_REQUEST"
    );
    assert_eq!(
        error_code(&call(&h, "desktop_invoke", json!({"windowId": "1"})).await),
        "INVALID_REQUEST"
    );
    assert_eq!(
        error_code(&call(&h, "desktop_describe", json!({"maxNodes": 0})).await),
        "INVALID_REQUEST"
    );
}

#[tokio::test]
async fn windows_hides_apps_that_are_not_allowed() {
    let h = harness_with_notepad();
    connect(&h).await;
    let v = text_json(&call(&h, "desktop_windows", json!({})).await);
    let ids: Vec<_> = v["result"]
        .as_array()
        .unwrap()
        .iter()
        .map(|w| w["windowId"].as_str().unwrap())
        .collect();
    assert_eq!(ids, ["1"]);
    assert!(v["result"][0].get("executablePath").is_none());
    assert_eq!(v["untrusted"], true);
}

#[tokio::test]
async fn apps_lists_only_allowed_apps() {
    let h = harness_with_notepad();
    let v = text_json(&call(&h, "desktop_apps", json!({})).await);
    assert_eq!(v["apps"].as_array().unwrap().len(), 1);
    assert_eq!(v["apps"][0]["name"], "Notepad");
    assert_eq!(v["apps"][0]["allowed"], true);
}

#[tokio::test]
async fn describing_a_window_that_is_not_allowed_is_refused_before_the_helper_describes() {
    let h = harness_with_notepad();
    connect(&h).await;
    let r = call(&h, "desktop_describe", json!({"windowId": "2"})).await;
    assert_eq!(error_code(&r), "DESKTOP_APP_NOT_ALLOWED");
    let message = text_json(&r)["error"]["message"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(message.contains("Open aevra-desktop"), "{message}");
    assert!(!calls(&h).contains(&"describe".to_string()));
    let bg = call(
        &h,
        "desktop_describe",
        json!({"windowId": "2", "mode": "background"}),
    )
    .await;
    assert_eq!(error_code(&bg), "DESKTOP_APP_NOT_ALLOWED");
    assert!(!calls(&h).contains(&"describeBackground".to_string()));
}

#[tokio::test]
async fn foreground_describe_returns_refs_and_redacts_sensitive_values() {
    let h = harness_with_notepad();
    connect(&h).await;
    let v = text_json(&call(&h, "desktop_describe", json!({"windowId": "1"})).await);
    let nodes = v["nodes"].as_array().unwrap();
    assert_eq!(nodes.len(), 3);
    assert!(nodes[0]["ref"].as_str().unwrap().starts_with("ref_"));
    assert!(nodes[0].get("handle").is_none());
    assert_eq!(nodes[1]["value"], "hello");
    assert_eq!(nodes[2]["value"], "[redacted]");
    assert!(!v.to_string().contains("hunter2"));
}

// Review Focus 1
#[tokio::test]
async fn webview2_window_is_judged_by_its_verified_host() {
    let webview = r"C:/Program Files/Microsoft/EdgeWebView/msedgewebview2.exe";
    let host_exe = r"C:/Apps/Quota/quota.exe";
    let mut w = window("5", webview, 300, vec![node("button", "Go")]);
    w.host = Some(VerifiedWindowHost {
        executable_path: host_exe.into(),
        instance: WindowInstance::new("6", 301, "host-started"),
    });
    let mut unverified = window("7", webview, 302, vec![node("button", "Go")]);
    unverified.host = None;

    // The runtime itself is allowlisted; the host is not.
    let h = harness(
        vec![w.clone(), unverified.clone()],
        vec![allow("runtime", "WebView2", webview)],
    );
    connect(&h).await;
    for id in ["5", "7"] {
        let r = call(
            &h,
            "desktop_describe",
            json!({"windowId": id, "mode": "background"}),
        )
        .await;
        assert_eq!(error_code(&r), "DESKTOP_APP_NOT_ALLOWED", "window {id}");
    }
    let listed = text_json(&call(&h, "desktop_windows", json!({})).await);
    assert!(listed["result"].as_array().unwrap().is_empty());

    // Allow the host and the verified window works; the unverified one still does not.
    let h = harness(vec![w, unverified], vec![allow("host", "Quota", host_exe)]);
    connect(&h).await;
    let ok = call(
        &h,
        "desktop_describe",
        json!({"windowId": "5", "mode": "background"}),
    )
    .await;
    assert!(!is_error(&ok), "{ok:?}");
    let still = call(
        &h,
        "desktop_describe",
        json!({"windowId": "7", "mode": "background"}),
    )
    .await;
    assert_eq!(error_code(&still), "DESKTOP_APP_NOT_ALLOWED");
}

// Review Focus 2
#[tokio::test]
async fn own_window_is_refused_even_when_allowlisted() {
    let own = r"C:\Apps\aevra-desktop\aevra-desktop.exe";
    let h = harness(
        vec![window("9", own, SELF_PID, vec![node("button", "Allow")])],
        vec![allow("self", "aevra-desktop", own)],
    );
    connect(&h).await;
    for args in [
        json!({"windowId": "9"}),
        json!({"windowId": "9", "mode": "background"}),
    ] {
        let r = call(&h, "desktop_describe", args).await;
        assert_eq!(error_code(&r), "DESKTOP_INPUT_REFUSED");
    }
    let typed = call(&h, "desktop_type", json!({"text": "hello"})).await;
    assert_eq!(error_code(&typed), "DESKTOP_INPUT_REFUSED");
    let captured = call(&h, "desktop_capture", json!({"windowId": "9"})).await;
    assert_eq!(error_code(&captured), "DESKTOP_INPUT_REFUSED");
    assert!(h.state.lock().unwrap().acts.is_empty());
}

#[tokio::test]
async fn background_describe_then_invoke_requires_a_fresh_describe() {
    let h = harness_with_notepad();
    connect(&h).await;
    let (snapshot, lease, refs) = describe_bg(&h, "1").await;
    let r = call(
        &h,
        "desktop_invoke",
        json!({"windowId": "1", "windowLeaseId": lease, "snapshotId": snapshot, "ref": refs[0]}),
    )
    .await;
    assert!(!is_error(&r), "{r:?}");
    let v = text_json(&r);
    assert_eq!(v["ok"], true);
    assert_eq!(v["requiresDescribe"], true);
    assert_eq!(v["snapshotInvalidated"], true);
    assert_eq!(v["focusChanged"], false);
    let acts = h.state.lock().unwrap().background_acts.clone();
    assert_eq!(acts.len(), 1);
    assert_eq!(acts[0].op, "invoke");
    assert!(acts[0].handle.starts_with("bg:"));
    // The same ref no longer resolves.
    let again = call(
        &h,
        "desktop_invoke",
        json!({"windowId": "1", "windowLeaseId": lease, "snapshotId": snapshot, "ref": refs[0]}),
    )
    .await;
    assert_eq!(error_code(&again), "DESKTOP_REF_STALE");
    // A fresh describe works again on the same lease.
    let (snapshot2, lease2, _) = describe_bg(&h, "1").await;
    assert_eq!(lease2, lease);
    assert_ne!(snapshot2, snapshot);
}

#[tokio::test]
async fn a_focus_change_suspends_the_lease() {
    let h = harness_with_notepad();
    connect(&h).await;
    h.state.lock().unwrap().focus_changes = true;
    let (snapshot, lease, refs) = describe_bg(&h, "1").await;
    let r = call(
        &h,
        "desktop_toggle",
        json!({"windowId": "1", "windowLeaseId": lease, "snapshotId": snapshot, "ref": refs[0]}),
    )
    .await;
    assert_eq!(text_json(&r)["focusChanged"], true);
    let (s2, _, r2) = describe_bg(&h, "1").await;
    // A fresh describe un-suspends it.
    let ok = call(
        &h,
        "desktop_select",
        json!({"windowId": "1", "windowLeaseId": lease, "snapshotId": s2, "ref": r2[0]}),
    )
    .await;
    assert!(!is_error(&ok), "{ok:?}");
}

#[tokio::test]
async fn a_helper_error_during_dispatch_keeps_its_code() {
    let h = harness_with_notepad();
    connect(&h).await;
    let (snapshot, lease, refs) = describe_bg(&h, "1").await;
    h.state.lock().unwrap().fail_background_act =
        Some("DESKTOP_ELEMENT_DISABLED: element is disabled".into());
    let r = call(
        &h,
        "desktop_invoke",
        json!({"windowId": "1", "windowLeaseId": lease, "snapshotId": snapshot, "ref": refs[0]}),
    )
    .await;
    assert_eq!(error_code(&r), "DESKTOP_ELEMENT_DISABLED");
}

#[tokio::test]
async fn a_second_owner_gets_window_busy() {
    let h = harness_with_notepad();
    connect(&h).await;
    describe_bg(&h, "1").await;
    let r = h
        .server
        .call(
            "owner-2",
            "desktop_describe",
            json!({"windowId": "1", "mode": "background"}),
        )
        .await;
    assert_eq!(error_code(&r), "DESKTOP_WINDOW_BUSY");
    assert!(
        text_json(&r)["error"]["details"]["retryAfterMs"]
            .as_u64()
            .unwrap()
            >= 1000
    );
    // And foreground input into a leased window is refused too.
    h.state.lock().unwrap().focused = "1".into();
    let click = call(&h, "desktop_key", json!({"keys": "Enter"})).await;
    assert_eq!(error_code(&click), "DESKTOP_WINDOW_BUSY");
}

#[tokio::test]
async fn target_changing_during_background_describe_releases_everything() {
    let h = harness_with_notepad();
    connect(&h).await;
    h.state.lock().unwrap().swap_instance_on_describe =
        Some(WindowInstance::new("1", 4242, "other"));
    let r = call(
        &h,
        "desktop_describe",
        json!({"windowId": "1", "mode": "background"}),
    )
    .await;
    assert_eq!(error_code(&r), "DESKTOP_TARGET_CHANGED");
    let s = h.state.lock().unwrap();
    assert_eq!(s.released_snapshots, s.described_snapshots);
    drop(s);
    assert!(!h.server.session.leases.is_window_leased("1"));
}

// Review Focus 3
#[tokio::test]
async fn removing_an_app_while_a_lease_is_live_refuses_the_next_act_and_releases_the_lease() {
    let h = harness_with_notepad();
    connect(&h).await;
    let (snapshot, lease, refs) = describe_bg(&h, "1").await;
    // The user removes Notepad from the allowlist.
    h.server.set_config(Config::default());
    let r = call(
        &h,
        "desktop_invoke",
        json!({"windowId": "1", "windowLeaseId": lease, "snapshotId": snapshot, "ref": refs[0]}),
    )
    .await;
    assert_eq!(error_code(&r), "DESKTOP_APP_NOT_ALLOWED");
    assert!(h.state.lock().unwrap().background_acts.is_empty());
    assert!(!h.server.session.leases.is_window_leased("1"));
    assert_eq!(h.state.lock().unwrap().released_snapshots, vec![snapshot]);
}

// Review Focus 3, through the hot-reload entry point rather than a raw swap.
#[tokio::test]
async fn apply_config_without_the_app_refuses_the_next_invoke_and_frees_the_snapshot() {
    let h = harness_with_notepad();
    connect(&h).await;
    let (snapshot, lease, refs) = describe_bg(&h, "1").await;
    h.server.apply_config(Config::default()).await;
    // The lease went with the app, and so did the helper snapshot.
    assert!(!h.server.session.leases.is_window_leased("1"));
    assert_eq!(
        h.state.lock().unwrap().released_snapshots,
        vec![snapshot.clone()]
    );
    let r = call(
        &h,
        "desktop_invoke",
        json!({"windowId": "1", "windowLeaseId": lease, "snapshotId": snapshot, "ref": refs[0]}),
    )
    .await;
    assert!(matches!(
        error_code(&r).as_str(),
        "DESKTOP_APP_NOT_ALLOWED" | "DESKTOP_LEASE_EXPIRED"
    ));
    assert!(h.state.lock().unwrap().background_acts.is_empty());
    // A fresh describe is refused by the gate.
    let again = call(
        &h,
        "desktop_describe",
        json!({"windowId": "1", "mode": "background"}),
    )
    .await;
    assert_eq!(error_code(&again), "DESKTOP_APP_NOT_ALLOWED");
}

// Review Focus 4
#[tokio::test]
async fn set_value_on_a_password_field_is_refused_before_the_helper_and_logged_without_the_value() {
    let h = harness_with_notepad();
    connect(&h).await;
    let (snapshot, lease, refs) = describe_bg(&h, "1").await;
    let secret = "correct-horse-battery";
    let r = call(
        &h,
        "desktop_set_value",
        json!({"windowId": "1", "windowLeaseId": lease, "snapshotId": snapshot, "ref": refs[2], "value": secret}),
    )
    .await;
    assert_eq!(error_code(&r), "DESKTOP_INPUT_REFUSED");
    assert!(h.state.lock().unwrap().background_acts.is_empty());

    let log = activity_lines(&h).join("\n");
    assert!(!log.contains(secret), "{log}");
    let last: serde_json::Value = serde_json::from_str(activity_lines(&h).last().unwrap()).unwrap();
    assert_eq!(last["tool"], "desktop_set_value");
    assert_eq!(last["verdict"], "deny");
    assert_eq!(last["code"], "DESKTOP_INPUT_REFUSED");
    assert_eq!(last["argLengths"]["value"], secret.len());
    assert_eq!(last["app"], "Notepad");
}

#[tokio::test]
async fn set_value_with_a_secret_shaped_value_is_refused() {
    let h = harness_with_notepad();
    connect(&h).await;
    let (snapshot, lease, refs) = describe_bg(&h, "1").await;
    let card = "4111 1111 1111 1111";
    let r = call(
        &h,
        "desktop_set_value",
        json!({"windowId": "1", "windowLeaseId": lease, "snapshotId": snapshot, "ref": refs[1], "value": card}),
    )
    .await;
    assert_eq!(error_code(&r), "DESKTOP_INPUT_REFUSED");
    assert!(h.state.lock().unwrap().background_acts.is_empty());
    assert!(!activity_lines(&h).join("\n").contains("4111"));
}

#[tokio::test]
async fn set_value_on_an_ordinary_field_reaches_the_helper_with_the_value() {
    let h = harness_with_notepad();
    connect(&h).await;
    let (snapshot, lease, refs) = describe_bg(&h, "1").await;
    let r = call(
        &h,
        "desktop_set_value",
        json!({"windowId": "1", "windowLeaseId": lease, "snapshotId": snapshot, "ref": refs[1], "value": "plain text"}),
    )
    .await;
    assert!(!is_error(&r), "{r:?}");
    let acts = h.state.lock().unwrap().background_acts.clone();
    assert_eq!(acts[0].op, "setValue");
    assert_eq!(acts[0].value.as_deref(), Some("plain text"));
    assert!(!activity_lines(&h).join("\n").contains("plain text"));
}

// Review Focus 6
#[tokio::test]
async fn release_window_frees_the_snapshot_not_the_lease() {
    let h = harness_with_notepad();
    connect(&h).await;
    let (snapshot, lease, _) = describe_bg(&h, "1").await;
    let r = call(
        &h,
        "desktop_release_window",
        json!({"windowId": "1", "windowLeaseId": lease}),
    )
    .await;
    assert_eq!(text_json(&r), json!({"ok": true, "released": true}));
    let released = h.state.lock().unwrap().released_snapshots.clone();
    assert_eq!(released, vec![snapshot]);
    assert!(!released.contains(&lease));
    let again = call(
        &h,
        "desktop_release_window",
        json!({"windowId": "1", "windowLeaseId": lease}),
    )
    .await;
    assert_eq!(error_code(&again), "DESKTOP_LEASE_EXPIRED");
}

#[tokio::test]
async fn foreground_type_of_a_secret_shaped_value_is_refused() {
    let h = harness_with_notepad();
    connect(&h).await;
    let token = format!("ghp_{}", "a".repeat(36));
    let r = call(&h, "desktop_type", json!({"text": token})).await;
    assert_eq!(error_code(&r), "DESKTOP_INPUT_REFUSED");
    assert!(h.state.lock().unwrap().acts.is_empty());
    assert!(!activity_lines(&h).join("\n").contains("ghp_"));
}

#[tokio::test]
async fn typing_after_clicking_a_password_field_is_refused() {
    let h = harness_with_notepad();
    connect(&h).await;
    let v = text_json(&call(&h, "desktop_describe", json!({"windowId": "1"})).await);
    let password_ref = v["nodes"][2]["ref"].as_str().unwrap().to_string();
    let notes_ref = v["nodes"][1]["ref"].as_str().unwrap().to_string();
    let click = call(&h, "desktop_click", json!({"ref": password_ref})).await;
    assert!(!is_error(&click), "{click:?}");
    let blocked = call(&h, "desktop_type", json!({"text": "abc"})).await;
    assert_eq!(error_code(&blocked), "DESKTOP_INPUT_REFUSED");
    call(&h, "desktop_click", json!({"ref": notes_ref})).await;
    let ok = call(&h, "desktop_type", json!({"text": "abc"})).await;
    assert!(!is_error(&ok), "{ok:?}");
}

#[tokio::test]
async fn foreground_click_returns_a_delta_and_the_window() {
    let h = harness_with_notepad();
    connect(&h).await;
    let r = call(&h, "desktop_click", json!({"x": 10, "y": 20})).await;
    let v = text_json(&r);
    assert_eq!(v["ok"], true);
    assert_eq!(v["delta"]["subtreeChanged"], true);
    assert_eq!(v["delta"]["focusChanged"], false);
    assert_eq!(v["window"]["windowId"], "1");
    let acts = h.state.lock().unwrap().acts.clone();
    assert_eq!((acts[0].x, acts[0].y), (Some(10), Some(20)));
    // A stale ref is refused without acting.
    let stale = call(&h, "desktop_click", json!({"ref": "ref_1_999"})).await;
    assert_eq!(error_code(&stale), "DESKTOP_REF_STALE");
    let neither = call(&h, "desktop_click", json!({})).await;
    assert_eq!(error_code(&neither), "INVALID_REQUEST");
}

#[tokio::test]
async fn foreground_input_is_unsupported_where_the_platform_says_so() {
    let mut h = harness_with_notepad();
    h.server = h.server.with_platform(desktop_mcp::Platform {
        foreground_input: false,
        capture: false,
    });
    connect(&h).await;
    for (tool, args) in [
        ("desktop_click", json!({"x": 1, "y": 1})),
        ("desktop_type", json!({"text": "a"})),
        ("desktop_key", json!({"keys": "Enter"})),
        ("desktop_scroll", json!({"deltaY": 3})),
    ] {
        let r = call(&h, tool, args).await;
        assert_eq!(error_code(&r), "DESKTOP_INPUT_UNSUPPORTED", "{tool}");
    }
    let capture = call(&h, "desktop_capture", json!({"windowId": "1"})).await;
    assert_eq!(error_code(&capture), "DESKTOP_CAPTURE_UNSUPPORTED");
    let c = calls(&h);
    assert!(!c.contains(&"act".to_string()));
    assert!(!c.contains(&"capture".to_string()));
}

#[cfg(not(windows))]
#[tokio::test]
async fn native_platform_off_windows_offers_foreground_adapter() {
    assert!(desktop_mcp::Platform::native().foreground_input);
}

#[cfg(windows)]
#[tokio::test]
async fn native_platform_on_windows_supports_foreground_input_and_capture() {
    let p = desktop_mcp::Platform::native();
    assert!(p.foreground_input && p.capture);
}

#[tokio::test]
async fn capture_returns_an_image_block_and_gates_the_window() {
    let h = harness_with_notepad();
    connect(&h).await;
    let r = call(&h, "desktop_capture", json!({"windowId": "1"})).await;
    assert!(!is_error(&r), "{r:?}");
    let image = r
        .content
        .iter()
        .find_map(|c| c.as_image())
        .expect("an image block");
    assert_eq!(image.mime_type, "image/jpeg");
    assert_eq!(image.data, "/9j/4AAQ");
    let denied = call(&h, "desktop_capture", json!({"windowId": "2"})).await;
    assert_eq!(error_code(&denied), "DESKTOP_APP_NOT_ALLOWED");
    let full = call(&h, "desktop_capture", json!({})).await;
    assert_eq!(error_code(&full), "DESKTOP_APP_NOT_ALLOWED");
    assert!(text_json(&full)["error"]["message"]
        .as_str()
        .unwrap()
        .contains("full-screen capture"));
}

#[tokio::test]
async fn every_call_writes_exactly_one_activity_entry_without_values() {
    let h = harness_with_notepad();
    call(&h, "desktop_status", json!({})).await;
    connect(&h).await;
    call(&h, "desktop_windows", json!({})).await;
    call(&h, "desktop_type", json!({"text": "do-not-log-this"})).await;
    call(&h, "desktop_nope", json!({})).await;
    let lines = activity_lines(&h);
    assert_eq!(lines.len(), 5);
    assert!(!lines.join("\n").contains("do-not-log-this"));
}

#[tokio::test]
async fn reconnecting_clears_leases_and_frees_their_snapshots() {
    let h = harness_with_notepad();
    connect(&h).await;
    let (snapshot, _, _) = describe_bg(&h, "1").await;
    connect(&h).await;
    assert!(!h.server.session.leases.is_window_leased("1"));
    assert_eq!(h.state.lock().unwrap().released_snapshots, vec![snapshot]);
}

#[tokio::test]
async fn apply_config_releases_leases_of_removed_apps_only() {
    let h = harness(
        vec![
            notepad_window("1"),
            window("2", OTHER, 200, vec![node("button", "Go")]),
        ],
        vec![
            allow("app_notepad", "Notepad", NOTEPAD),
            allow("app_other", "Other", OTHER),
        ],
    );
    connect(&h).await;
    let (snapshot1, _, _) = describe_bg(&h, "1").await;
    describe_bg(&h, "2").await;
    let mut next = (*h.server.config()).clone();
    next.allowed_apps.retain(|a| a.id != "app_notepad");
    h.server.apply_config(next).await;
    assert!(!h.server.session.leases.is_window_leased("1"));
    assert!(h.server.session.leases.is_window_leased("2"));
    assert_eq!(h.state.lock().unwrap().released_snapshots, vec![snapshot1]);
}

#[test]
fn portable_platforms_offer_native_adapters_without_claiming_permission() {
    for os in ["windows", "macos", "linux"] {
        let p = desktop_mcp::Platform::for_os(os);
        assert!(p.foreground_input && p.capture, "{os}");
    }
    let p = desktop_mcp::Platform::for_os("unknown");
    assert!(!p.foreground_input && !p.capture);
}

#[tokio::test]
async fn unavailable_backend_permissions_refuse_input_and_capture_without_native_effects() {
    let h = harness_with_notepad();
    h.state.lock().unwrap().capabilities = Some(desktop_core::Capabilities {
        input: false,
        capture: false,
        tree: true,
        attribution: true,
        background_actions: Some(true),
    });
    connect(&h).await;
    let r = call(&h, "desktop_key", json!({"keys":"Enter"})).await;
    assert_eq!(error_code(&r), "DESKTOP_PERMISSION_REQUIRED");
    let r = call(&h, "desktop_capture", json!({"windowId":"1"})).await;
    assert_eq!(error_code(&r), "DESKTOP_PERMISSION_REQUIRED");
    let c = calls(&h);
    assert!(!c.iter().any(|v| v == "act" || v == "capture"));
}
