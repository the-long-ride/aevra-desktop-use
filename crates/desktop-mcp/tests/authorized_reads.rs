mod common;
use common::*;
use serde_json::json;
#[tokio::test]
async fn authorized_reads_refuse_reused_native_identity_without_publishing_data() {
    for tool in ["desktop_capture", "desktop_describe"] {
        let h = harness(
            vec![window(
                "reused",
                NOTEPAD,
                100,
                vec![node("text", "allowed")],
            )],
            vec![allow("notepad", "Notepad", NOTEPAD)],
        );
        connect(&h).await;
        h.state.lock().unwrap().read_replacement = Some(window(
            "reused",
            OTHER,
            101,
            vec![node("text", "DENIED-DATA")],
        ));
        let result = call(&h, tool, json!({"windowId":"reused"})).await;
        assert!(
            is_error(&result),
            "{tool} published replacement data: {result:?}"
        );
        assert!(!format!("{result:?}").contains("DENIED-DATA"));
        assert!(activity_lines(&h).last().unwrap().contains("deny"));
    }
}
#[tokio::test]
async fn authorized_reads_recheck_policy_revoked_during_native_read() {
    for tool in ["desktop_capture", "desktop_describe"] {
        let h = harness(
            vec![window(
                "allowed",
                NOTEPAD,
                100,
                vec![node("text", "allowed")],
            )],
            vec![allow("notepad", "Notepad", NOTEPAD)],
        );
        connect(&h).await;
        let (release, gate) = std::sync::mpsc::channel();
        h.state.lock().unwrap().read_gate = Some(gate);
        let revoke = async {
            let method = if tool == "desktop_capture" {
                "capture"
            } else {
                "describe"
            };
            while !calls(&h).iter().any(|m| m == method) {
                tokio::task::yield_now().await;
            }
            h.server.set_config(desktop_policy::Config::default());
            release.send(()).unwrap();
        };
        let (result, ()) = tokio::join!(call(&h, tool, json!({"windowId":"allowed"})), revoke);
        assert!(
            is_error(&result),
            "{tool} published after allowlist revocation"
        );
    }
}
#[tokio::test]
async fn authorized_reads_refuse_results_after_disconnect() {
    for tool in ["desktop_capture", "desktop_describe"] {
        let h = harness(
            vec![window(
                "allowed",
                NOTEPAD,
                100,
                vec![node("text", "old-session")],
            )],
            vec![allow("notepad", "Notepad", NOTEPAD)],
        );
        connect(&h).await;
        let (release, gate) = std::sync::mpsc::channel();
        h.state.lock().unwrap().read_gate = Some(gate);
        let disconnect = async {
            let method = if tool == "desktop_capture" {
                "capture"
            } else {
                "describe"
            };
            while !calls(&h).iter().any(|m| m == method) {
                tokio::task::yield_now().await;
            }
            let result = call(&h, "desktop_disconnect", json!({})).await;
            assert!(!is_error(&result));
            release.send(()).unwrap();
        };
        let (result, ()) = tokio::join!(call(&h, tool, json!({"windowId":"allowed"})), disconnect);
        assert!(
            is_error(&result),
            "{tool} published disconnected session data: {result:?}"
        );
        assert!(!format!("{result:?}").contains("old-session"));
    }
}

#[tokio::test]
async fn pending_connect_cannot_restore_a_disconnected_session() {
    let h = harness(vec![], vec![]);
    let (release, gate) = std::sync::mpsc::channel();
    h.state.lock().unwrap().connect_gate = Some(gate);
    let disconnect = async {
        // The connect future is polled first and has invalidated the old session.
        tokio::task::yield_now().await;
        let result = call(&h, "desktop_disconnect", json!({})).await;
        assert!(!is_error(&result));
        release.send(()).unwrap();
    };
    let (result, ()) = tokio::join!(call(&h, "desktop_connect", json!({})), disconnect);
    assert!(is_error(&result), "cancelled connect restored a session");
    let status = text_json(&call(&h, "desktop_status", json!({})).await);
    assert_eq!(status["connected"], false);
}
