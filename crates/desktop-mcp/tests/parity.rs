//! Tool-name, description and schema parity against the vendored aevra
//! TypeScript registry, read as text. `desktop_request_access` is excluded on
//! purpose: the allowlist replaces it.

use desktop_mcp::tools::tool_list;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

const EXCLUDED: &str = "desktop_request_access";

fn vendor_file(name: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../vendor/aevra/packages/mcp-tools/src")
        .join(name);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()))
        .replace("\r\n", "\n")
}

/// A top-level tool entry: lines from `  desktop_x:` up to the next key at the
/// same indent.
fn tool_spans(text: &str) -> BTreeMap<String, Vec<String>> {
    let mut spans: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut current: Option<String> = None;
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("  desktop_") {
            if let Some(end) = rest.find(':') {
                let name = format!("desktop_{}", &rest[..end]);
                if rest[..end]
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c == '_')
                {
                    spans.entry(name.clone()).or_default();
                    current = Some(name);
                    spans
                        .get_mut(current.as_ref().unwrap())
                        .unwrap()
                        .push(line.to_string());
                    continue;
                }
            }
        }
        if line.starts_with("  ") && !line.starts_with("   ") && !line.trim().is_empty() {
            // Another top-level key ends the span.
            current = None;
        }
        if let Some(name) = &current {
            spans.get_mut(name).unwrap().push(line.to_string());
        }
    }
    spans
}

fn quoted_strings(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\'' {
            let mut s = String::new();
            while let Some(n) = chars.next() {
                match n {
                    '\\' => {
                        if let Some(esc) = chars.next() {
                            s.push(esc);
                        }
                    }
                    '\'' => break,
                    other => s.push(other),
                }
            }
            out.push(s);
        }
    }
    out
}

/// `required` entries declared at a tool's top schema level (4-space indent).
fn required_of(span: &[String]) -> BTreeSet<String> {
    span.iter()
        .find(|l| l.starts_with("    required: ["))
        .map(|l| quoted_strings(l).into_iter().collect())
        .unwrap_or_default()
}

/// Property names declared at the tool's `properties` level (6-space indent),
/// with the legacy spread expanded.
fn properties_of(span: &[String]) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let mut in_properties = false;
    for line in span {
        if line.starts_with("    properties: {") {
            in_properties = true;
            continue;
        }
        if in_properties && line.starts_with("    }") {
            in_properties = false;
        }
        if in_properties {
            if line.trim() == "...hostLegacyWorkspaceProperties," {
                out.insert("workspace".to_string());
                out.insert("workspaceId".to_string());
            } else if line.starts_with("      ") && !line.starts_with("       ") {
                if let Some(name) = line.trim().split(':').next() {
                    if name.chars().all(|c| c.is_ascii_alphanumeric()) {
                        out.insert(name.to_string());
                    }
                }
            }
        }
    }
    if span.iter().any(|l| l.contains("hostControlEmptySchema")) {
        out.insert("workspace".to_string());
        out.insert("workspaceId".to_string());
    }
    out
}

fn description_of(span: &[String]) -> Option<String> {
    // `  desktop_x: 'text',` on one line, or the string on the next line.
    let first = span.first()?;
    let mut candidates = quoted_strings(first);
    if candidates.is_empty() {
        candidates = quoted_strings(span.get(1)?);
    }
    candidates.into_iter().next()
}

fn schema_spans() -> BTreeMap<String, Vec<String>> {
    let mut spans = tool_spans(&vendor_file("registry-desktop-schemas.ts"));
    // desktop_act_many lives with the control schemas.
    for (name, span) in tool_spans(&vendor_file("registry-control-schemas.ts")) {
        spans.insert(name, span);
    }
    spans
}

#[test]
fn tool_names_match_aevra_minus_request_access() {
    let theirs: BTreeSet<String> = schema_spans().into_keys().collect();
    assert!(
        theirs.contains(EXCLUDED),
        "the vendored registry changed shape"
    );
    assert_eq!(theirs.len(), 18);
    let mut expected = theirs.clone();
    expected.remove(EXCLUDED);
    let ours: BTreeSet<String> = tool_list().iter().map(|t| t.name.to_string()).collect();
    assert_eq!(ours, expected);
    assert_eq!(ours.len(), 17);

    let described: BTreeSet<String> = tool_spans(&vendor_file("registry-descriptions.ts"))
        .into_keys()
        .collect();
    assert_eq!(
        described, theirs,
        "descriptions and schemas must list the same tools"
    );
}

#[test]
fn descriptions_match_verbatim() {
    let spans = tool_spans(&vendor_file("registry-descriptions.ts"));
    for tool in tool_list() {
        let span = &spans[tool.name.as_ref()];
        let theirs =
            description_of(span).unwrap_or_else(|| panic!("no description for {}", tool.name));
        assert_eq!(
            tool.description.as_deref(),
            Some(theirs.as_str()),
            "{}",
            tool.name
        );
    }
}

#[test]
fn required_arrays_match() {
    let spans = schema_spans();
    for tool in tool_list() {
        let ours: BTreeSet<String> = tool
            .input_schema
            .get("required")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();
        assert_eq!(
            ours,
            required_of(&spans[tool.name.as_ref()]),
            "{}",
            tool.name
        );
    }
}

#[test]
fn property_names_match() {
    let spans = schema_spans();
    for tool in tool_list() {
        let ours: BTreeSet<String> = tool
            .input_schema
            .get("properties")
            .and_then(Value::as_object)
            .map(|o| o.keys().cloned().collect())
            .unwrap_or_default();
        assert_eq!(
            ours,
            properties_of(&spans[tool.name.as_ref()]),
            "{}",
            tool.name
        );
    }
}

#[test]
fn schemas_keep_additional_properties_false() {
    for tool in tool_list() {
        assert_eq!(
            tool.input_schema.get("additionalProperties"),
            Some(&Value::Bool(false)),
            "{}",
            tool.name
        );
    }
}
