//! Tool definitions: names, descriptions and input schemas copied from aevra
//! v1.2.1 (`packages/mcp-tools/src/registry-desktop-schemas.ts`,
//! `registry-control-schemas.ts` and `registry-descriptions.ts`), minus
//! `desktop_request_access`. `tests/parity.rs` compares them with the vendored
//! TypeScript so the copies cannot drift unnoticed.

use rmcp::model::{JsonObject, Tool};
use serde_json::{json, Map, Value};
use std::sync::Arc;

pub const DESKTOP_APPS: &str = "desktop_apps";
pub const MAX_ACT_MANY_ACTIONS: usize = 20;

fn string_prop(description: &str) -> Value {
    json!({"type": "string", "description": description})
}

/// `hostLegacyWorkspaceProperties`: accepted and ignored, kept for compatibility.
fn legacy_properties() -> Map<String, Value> {
    let mut m = Map::new();
    m.insert(
        "workspace".into(),
        string_prop(
            "Deprecated compatibility field; ignored for host browser and desktop authorization.",
        ),
    );
    m.insert(
        "workspaceId".into(),
        string_prop(
            "Deprecated compatibility field; ignored for host browser and desktop authorization.",
        ),
    );
    m
}

fn object_schema(extra: Vec<(&str, Value)>, required: &[&str]) -> Value {
    let mut properties = legacy_properties();
    for (name, schema) in extra {
        properties.insert(name.to_string(), schema);
    }
    let mut schema = Map::new();
    schema.insert("type".into(), json!("object"));
    schema.insert("properties".into(), Value::Object(properties));
    if !required.is_empty() {
        schema.insert("required".into(), json!(required));
    }
    schema.insert("additionalProperties".into(), json!(false));
    Value::Object(schema)
}

fn background_target_props() -> Vec<(&'static str, Value)> {
    vec![
        ("windowId", string_prop("Target window id.")),
        (
            "windowLeaseId",
            string_prop("Window lease id from desktop_describe in background mode."),
        ),
        (
            "snapshotId",
            string_prop("Snapshot id from desktop_describe in background mode."),
        ),
    ]
}

fn background_schema(ref_description: &str, with_value: bool) -> Value {
    let mut props = background_target_props();
    props.push(("ref", string_prop(ref_description)));
    let mut required = vec!["windowId", "windowLeaseId", "snapshotId", "ref"];
    if with_value {
        props.push(("value", string_prop("Text value to set on the element.")));
        required.push("value");
    }
    object_schema(props, &required)
}

/// `(name, description, input schema)` for every exposed tool.
fn definitions() -> Vec<(&'static str, &'static str, Value)> {
    vec![
        (
            "desktop_status",
            "Show the current Aevra desktop session, connection, and window state. Requires a host desktop grant, not a workspace selection.",
            object_schema(vec![], &[]),
        ),
        (
            "desktop_connect",
            "Connect to the local Aevra desktop control helper for this session.",
            object_schema(vec![], &[]),
        ),
        (
            "desktop_disconnect",
            "Disconnect the local Aevra desktop control session.",
            object_schema(vec![], &[]),
        ),
        (
            "desktop_apps",
            "List apps in scope for desktop.control, resolved from the current desktop policy.",
            object_schema(vec![], &[]),
        ),
        (
            "desktop_windows",
            "List top-level desktop windows visible to the connected desktop session.",
            object_schema(vec![], &[]),
        ),
        (
            "desktop_describe",
            "Read the accessibility tree of one desktop window, redacted for secret-shaped text.",
            object_schema(
                vec![
                    (
                        "windowId",
                        string_prop("Target window id. Defaults to the focused window."),
                    ),
                    (
                        "maxNodes",
                        json!({"type": "integer", "minimum": 1, "maximum": 5000}),
                    ),
                    (
                        "interactiveOnly",
                        json!({"type": "boolean", "description": "Return only interactive nodes."}),
                    ),
                    (
                        "mode",
                        json!({
                            "type": "string",
                            "enum": ["foreground", "background"],
                            "description": "Describe mode: foreground (default) or background.",
                        }),
                    ),
                ],
                &[],
            ),
        ),
        (
            "desktop_capture",
            "Take a screenshot of one desktop window.",
            object_schema(
                vec![(
                    "windowId",
                    string_prop("Target window id. Defaults to the focused window."),
                )],
                &[],
            ),
        ),
        (
            "desktop_click",
            "Click at an element ref or screen coordinate in the focused desktop window.",
            object_schema(
                vec![
                    (
                        "ref",
                        string_prop("Element ref from a previous desktop_describe."),
                    ),
                    (
                        "x",
                        json!({"type": "number", "description": "Screen x-coordinate, when no ref is available."}),
                    ),
                    (
                        "y",
                        json!({"type": "number", "description": "Screen y-coordinate, when no ref is available."}),
                    ),
                ],
                &[],
            ),
        ),
        (
            "desktop_type",
            "Type text into an element ref or the focused desktop element.",
            object_schema(
                vec![(
                    "text",
                    string_prop("Text to type into whichever element currently has focus."),
                )],
                &["text"],
            ),
        ),
        (
            "desktop_key",
            "Send a key or key combination to the focused desktop window.",
            object_schema(
                vec![(
                    "keys",
                    string_prop("Key or key combination to send, e.g. \"Enter\" or \"Ctrl+A\"."),
                )],
                &["keys"],
            ),
        ),
        (
            "desktop_scroll",
            "Scroll an element ref or the focused desktop window.",
            object_schema(
                vec![(
                    "deltaY",
                    json!({
                        "type": "number",
                        "description": "Vertical scroll delta, delivered wherever the mouse cursor currently is.",
                    }),
                )],
                &["deltaY"],
            ),
        ),
        (
            "desktop_invoke",
            "Invoke a supported UI element in a background desktop window.",
            background_schema("Element ref to invoke.", false),
        ),
        (
            "desktop_set_value",
            "Set text value on a supported UI element in a background desktop window.",
            background_schema("Element ref to set value on.", true),
        ),
        (
            "desktop_select",
            "Select an item element in a background desktop window.",
            background_schema("Element ref to select.", false),
        ),
        (
            "desktop_toggle",
            "Toggle the check or toggle state of an element in a background desktop window.",
            background_schema("Element ref to toggle.", false),
        ),
        (
            "desktop_release_window",
            "Release an acquired background desktop window lease.",
            object_schema(
                vec![
                    ("windowId", string_prop("Target window id.")),
                    ("windowLeaseId", string_prop("Window lease id to release.")),
                ],
                &["windowId", "windowLeaseId"],
            ),
        ),
        (
            "desktop_act_many",
            "Execute host-granted semantic desktop actions without selecting a workspace or synthesizing host cursor, keyboard, clipboard, or focus input.",
            object_schema(
                vec![
                    ("windowId", string_prop("Target desktop window id.")),
                    (
                        "requestId",
                        string_prop("Optional idempotency key for the batch."),
                    ),
                    (
                        "actions",
                        json!({
                            "type": "array",
                            "minItems": 1,
                            "maxItems": MAX_ACT_MANY_ACTIONS,
                            "items": {"type": "object"},
                            "description": "Ordered shared-semantic desktop actions. Each action uses a current ref or exact unique locator and one of invoke, click, setValue, type, select, setToggleState. Optional preconditions/postcondition remain finite typed predicates.",
                        }),
                    ),
                    (
                        "stopOnError",
                        json!({
                            "type": "boolean",
                            "description": "Default true. When true, each action depends on the prior action.",
                        }),
                    ),
                    (
                        "deadlineMs",
                        json!({
                            "type": "integer",
                            "minimum": 1000,
                            "maximum": 60000,
                            "description": "Overall local execution deadline.",
                        }),
                    ),
                    (
                        "maxOutputTokens",
                        json!({
                            "type": "integer",
                            "minimum": 1,
                            "maximum": 4000,
                            "description": "Approximate bound for the returned semantic delta.",
                        }),
                    ),
                ],
                &["windowId", "actions"],
            ),
        ),
    ]
}

/// The 17 exposed tools: aevra's 18 desktop tools minus `desktop_request_access`.
pub fn tool_list() -> Vec<Tool> {
    definitions()
        .into_iter()
        .map(|(name, description, schema)| {
            let Value::Object(map) = schema else {
                unreachable!("schemas are objects")
            };
            Tool::new(name, description, Arc::new(map) as Arc<JsonObject>)
        })
        .collect()
}

/// The input schema of `name`, for argument validation.
pub fn schema_for(name: &str) -> Option<Value> {
    definitions()
        .into_iter()
        .find(|(n, _, _)| *n == name)
        .map(|(_, _, schema)| schema)
}

/// Validates `value` against the JSON Schema subset these tools use: `type`,
/// `enum`, `minimum`, `maximum`, `minItems`, `maxItems`, `required`,
/// `properties`, `additionalProperties: false` and `items.type`. Returns a
/// message naming the offending path.
pub fn validate(schema: &Value, value: &Value) -> Result<(), String> {
    check(schema, value, "arguments")
}

fn type_matches(expected: &str, value: &Value) -> bool {
    match expected {
        "object" => value.is_object(),
        "array" => value.is_array(),
        "string" => value.is_string(),
        "boolean" => value.is_boolean(),
        "number" => value.is_number(),
        "integer" => {
            value.is_i64()
                || value.is_u64()
                || value
                    .as_f64()
                    .is_some_and(|f| f.fract() == 0.0 && f.is_finite())
        }
        _ => true,
    }
}

fn check(schema: &Value, value: &Value, path: &str) -> Result<(), String> {
    let Some(schema) = schema.as_object() else {
        return Ok(());
    };
    if let Some(expected) = schema.get("type").and_then(Value::as_str) {
        if !type_matches(expected, value) {
            return Err(format!("{path} must be of type {expected}"));
        }
    }
    if let Some(options) = schema.get("enum").and_then(Value::as_array) {
        if !options.contains(value) {
            return Err(format!(
                "{path} must be one of {}",
                Value::Array(options.clone())
            ));
        }
    }
    if let Some(n) = value.as_f64() {
        if let Some(min) = schema.get("minimum").and_then(Value::as_f64) {
            if n < min {
                return Err(format!("{path} must be at least {min}"));
            }
        }
        if let Some(max) = schema.get("maximum").and_then(Value::as_f64) {
            if n > max {
                return Err(format!("{path} must be at most {max}"));
            }
        }
    }
    if let Some(items) = value.as_array() {
        if let Some(min) = schema.get("minItems").and_then(Value::as_u64) {
            if (items.len() as u64) < min {
                return Err(format!("{path} must have at least {min} items"));
            }
        }
        if let Some(max) = schema.get("maxItems").and_then(Value::as_u64) {
            if items.len() as u64 > max {
                return Err(format!("{path} must have at most {max} items"));
            }
        }
        if let Some(item_schema) = schema.get("items") {
            for (i, item) in items.iter().enumerate() {
                check(item_schema, item, &format!("{path}[{i}]"))?;
            }
        }
    }
    if let Some(object) = value.as_object() {
        if let Some(required) = schema.get("required").and_then(Value::as_array) {
            for key in required.iter().filter_map(Value::as_str) {
                if !object.contains_key(key) {
                    return Err(format!("{path} is missing required property {key}"));
                }
            }
        }
        let properties = schema.get("properties").and_then(Value::as_object);
        for (key, child) in object {
            match properties.and_then(|p| p.get(key)) {
                Some(child_schema) => check(child_schema, child, &format!("{path}.{key}"))?,
                None if schema.get("additionalProperties") == Some(&Value::Bool(false)) => {
                    return Err(format!("{path} has unknown property {key}"));
                }
                None => {}
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn there_are_seventeen_tools_and_no_request_access() {
        let names: Vec<_> = tool_list()
            .into_iter()
            .map(|t| t.name.to_string())
            .collect();
        assert_eq!(names.len(), 17);
        assert!(!names.contains(&"desktop_request_access".to_string()));
        assert!(names.contains(&"desktop_act_many".to_string()));
    }

    #[test]
    fn schemas_keep_additional_properties_false_and_legacy_fields() {
        for tool in tool_list() {
            let schema = Value::Object((*tool.input_schema).clone());
            assert_eq!(schema["additionalProperties"], false, "{}", tool.name);
            assert!(
                schema["properties"]["workspace"].is_object(),
                "{}",
                tool.name
            );
            assert!(
                schema["properties"]["workspaceId"].is_object(),
                "{}",
                tool.name
            );
        }
    }

    #[test]
    fn validation_covers_the_keywords_in_use() {
        let describe = schema_for("desktop_describe").unwrap();
        assert!(validate(&describe, &json!({})).is_ok());
        assert!(validate(&describe, &json!({"maxNodes": 5, "mode": "background"})).is_ok());
        assert!(validate(&describe, &json!({"maxNodes": 0})).is_err());
        assert!(validate(&describe, &json!({"maxNodes": 5001})).is_err());
        assert!(validate(&describe, &json!({"maxNodes": 1.5})).is_err());
        assert!(validate(&describe, &json!({"mode": "sideways"})).is_err());
        assert!(validate(&describe, &json!({"nope": 1})).is_err());
        assert!(validate(&describe, &json!("text")).is_err());
        let invoke = schema_for("desktop_invoke").unwrap();
        let err = validate(&invoke, &json!({"windowId": "1"})).unwrap_err();
        assert!(err.contains("missing required"), "{err}");
    }

    #[test]
    fn act_many_allows_at_most_twenty_actions() {
        let schema = schema_for("desktop_act_many").unwrap();
        let action = json!({"op": "invoke", "ref": "r"});
        let ok = json!({"windowId": "1", "actions": vec![action.clone(); 20]});
        let too_many = json!({"windowId": "1", "actions": vec![action.clone(); 21]});
        assert!(validate(&schema, &ok).is_ok());
        assert!(validate(&schema, &too_many).is_err());
        assert!(validate(&schema, &json!({"windowId": "1", "actions": []})).is_err());
        assert!(validate(&schema, &json!({"windowId": "1", "actions": ["x"]})).is_err());
    }
}
