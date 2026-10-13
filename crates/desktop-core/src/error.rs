//! The error shape that crosses the MCP boundary: `{code, message}`.

use serde::Serialize;
use serde_json::{json, Value};

/// Helper error codes aevra's supervisor lets through unchanged
/// (`ALLOWED_HELPER_ERROR_CODES` in `packages/desktop/src/helper-process.ts`).
/// Any other `DESKTOP_*` code a helper emits is reported as `DESKTOP_HELPER`.
pub const ALLOWED_HELPER_CODES: &[&str] = &[
    "DESKTOP_BACKGROUND_UNSUPPORTED",
    "DESKTOP_CAPTURE_UNSUPPORTED",
    "DESKTOP_PERMISSION_REQUIRED",
    "DESKTOP_PROVIDER_UNAVAILABLE",
    "DESKTOP_PATTERN_UNSUPPORTED",
    "DESKTOP_ELEMENT_DISABLED",
    "DESKTOP_VALUE_READ_ONLY",
    "DESKTOP_REF_STALE",
    "DESKTOP_TARGET_CHANGED",
    "DESKTOP_WINDOW_BUSY",
    "DESKTOP_LEASE_EXPIRED",
    "DESKTOP_INPUT_REFUSED",
    "DESKTOP_OUTCOME_UNKNOWN",
    "DESKTOP_FOCUS_CHANGED",
    "DESKTOP_DRIVER_DIED",
    "DESKTOP_TIMEOUT",
    "DESKTOP_HELPER",
];

/// Codes aevra-desktop adds on top of aevra's set.
pub const ADDED_CODES: &[&str] = &[
    "DESKTOP_APP_NOT_ALLOWED",
    "DESKTOP_INPUT_UNSUPPORTED",
    "DESKTOP_NOT_CONNECTED",
    "INVALID_REQUEST",
];

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DesktopError {
    pub code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<Value>,
}

impl DesktopError {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            details: None,
        }
    }

    pub fn with_details(mut self, details: Value) -> Self {
        self.details = Some(details);
        self
    }

    /// Parses a helper error string the way the helper's `map_err_line`
    /// (`main.rs`) and aevra's `parseHelperError` do between them.
    ///
    /// - `"DESKTOP_X: msg"` splits at the first `:` and keeps `DESKTOP_X` when
    ///   it is an allowed code, otherwise the code becomes `DESKTOP_HELPER`.
    /// - A bare `"DESKTOP_X"` is both code and message.
    /// - Anything else is `DESKTOP_HELPER` with the whole text as message.
    pub fn from_helper(text: String) -> Self {
        let (raw_code, message) = match text.split_once(':') {
            Some((prefix, rest)) if prefix.trim().starts_with("DESKTOP_") => {
                (Some(prefix.trim().to_string()), rest.trim().to_string())
            }
            _ if text.starts_with("DESKTOP_") => (Some(text.clone()), text.clone()),
            _ => (None, text),
        };
        match raw_code {
            Some(code) if ALLOWED_HELPER_CODES.contains(&code.as_str()) => Self::new(code, message),
            Some(_) | None => Self::new("DESKTOP_HELPER", message),
        }
    }

    pub fn driver_died(message: impl Into<String>) -> Self {
        Self::new("DESKTOP_DRIVER_DIED", message)
    }

    /// The JSON body returned to MCP clients: `{"error":{"code","message"}}`.
    pub fn to_error_body(&self) -> Value {
        match &self.details {
            Some(details) => {
                json!({"error": {"code": self.code, "message": self.message, "details": details}})
            }
            None => json!({"error": {"code": self.code, "message": self.message}}),
        }
    }
}

impl std::fmt::Display for DesktopError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for DesktopError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allowed_code_with_message() {
        let e = DesktopError::from_helper("DESKTOP_REF_STALE: gone".into());
        assert_eq!(
            (e.code.as_str(), e.message.as_str()),
            ("DESKTOP_REF_STALE", "gone")
        );
    }

    #[test]
    fn message_may_contain_further_colons() {
        let e = DesktopError::from_helper("DESKTOP_HELPER: a: b".into());
        assert_eq!(e.message, "a: b");
    }

    #[test]
    fn unknown_desktop_code_is_helper() {
        let e = DesktopError::from_helper("DESKTOP_MADE_UP: x".into());
        assert_eq!(
            (e.code.as_str(), e.message.as_str()),
            ("DESKTOP_HELPER", "x")
        );
    }

    #[test]
    fn bare_code_is_code_and_message() {
        let e = DesktopError::from_helper("DESKTOP_TIMEOUT".into());
        assert_eq!(
            (e.code.as_str(), e.message.as_str()),
            ("DESKTOP_TIMEOUT", "DESKTOP_TIMEOUT")
        );
    }

    #[test]
    fn plain_text_keeps_the_whole_message() {
        let e = DesktopError::from_helper("failed: no window".into());
        assert_eq!(e.code, "DESKTOP_HELPER");
        assert_eq!(e.message, "failed: no window");
    }

    #[test]
    fn error_body_has_code_and_message_only_by_default() {
        let body = DesktopError::new("DESKTOP_NOT_CONNECTED", "x").to_error_body();
        assert_eq!(
            body,
            json!({"error": {"code": "DESKTOP_NOT_CONNECTED", "message": "x"}})
        );
    }
}
