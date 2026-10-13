use desktop_core::{DescribeRequest, WindowIdentity};
use serde_json::json;

#[test]
fn window_identity_serialises_camel_case_window_id() {
    let identity = WindowIdentity {
        window_id: "42".to_string(),
        process_name: Some("example.exe".to_string()),
        executable_path: None,
        title: None,
    };
    let value = serde_json::to_value(&identity).unwrap();
    assert_eq!(value["windowId"], "42");
    assert!(value.get("executablePath").is_none());
}

#[test]
fn describe_request_deserialises_from_wire_shape() {
    let request =
        serde_json::from_value::<DescribeRequest>(json!({"maxNodes": 5, "interactiveOnly": false}))
            .unwrap();
    assert_eq!(request.max_nodes, 5);
    assert!(!request.interactive_only);
}
