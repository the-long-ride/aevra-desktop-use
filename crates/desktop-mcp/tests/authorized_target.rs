mod common;
use common::*;
use serde_json::json;

#[tokio::test]
async fn focus_switch_after_authorization_never_dispatches_input_to_blocked_app() {
    for (tool, args) in [
        ("desktop_click", json!({"x":30,"y":40})),
        ("desktop_type", json!({"text":"notes"})),
        ("desktop_key", json!({"keys":"ctrl+a"})),
        ("desktop_scroll", json!({"deltaY":120})),
    ] {
        let h = harness(
            vec![
                window("allowed", NOTEPAD, 100, vec![]),
                window("blocked", OTHER, 200, vec![]),
            ],
            vec![allow("notepad", "Notepad", NOTEPAD)],
        );
        connect(&h).await;
        h.state.lock().unwrap().focus_on_screen_state = Some("blocked".into());
        let reply = call(&h, tool, args).await;
        assert_eq!(error_code(&reply), "DESKTOP_TARGET_CHANGED", "{tool}");
        assert!(
            h.state.lock().unwrap().acts.is_empty(),
            "{tool} sent native input"
        );
        let audit: serde_json::Value =
            serde_json::from_str(activity_lines(&h).last().unwrap()).unwrap();
        assert_eq!(audit["app"], "Notepad");
        assert_eq!(audit["code"], "DESKTOP_TARGET_CHANGED");
    }
}
