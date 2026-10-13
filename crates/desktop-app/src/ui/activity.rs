//! The Activity tab's rows, from the value-free activity log.

use desktop_policy::activity::{ActivityEntry, ALLOW, DENY};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Ok,
    Denied,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActivityRow {
    pub timestamp: String,
    pub tool: String,
    pub app: String,
    pub outcome: String,
    pub tone: Tone,
}

/// Newest first. The log never holds values, only lengths, so a row cannot
/// show what was typed.
pub fn rows(entries: &[ActivityEntry]) -> Vec<ActivityRow> {
    entries
        .iter()
        .rev()
        .map(|e| {
            let tone = match e.verdict.as_str() {
                v if v == ALLOW => Tone::Ok,
                v if v == DENY => Tone::Denied,
                _ => Tone::Error,
            };
            let outcome = match (&e.code, tone) {
                (Some(code), _) => code.clone(),
                (None, Tone::Ok) => "ok".to_string(),
                (None, _) => e.verdict.clone(),
            };
            ActivityRow {
                timestamp: e.ts.clone(),
                tool: e.tool.clone(),
                app: e.app.clone().unwrap_or_else(|| "-".to_string()),
                outcome,
                tone,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(tool: &str, verdict: &str, code: Option<&str>, app: Option<&str>) -> ActivityEntry {
        let mut e = ActivityEntry::new(tool, verdict);
        e.ts = "2026-10-13T08:15:30.123Z".into();
        e.code = code.map(str::to_string);
        e.app = app.map(str::to_string);
        e
    }

    #[test]
    fn rows_are_newest_first_with_a_tone_per_verdict() {
        let entries = vec![
            entry("desktop_windows", "allow", None, None),
            entry(
                "desktop_invoke",
                "deny",
                Some("DESKTOP_APP_NOT_ALLOWED"),
                Some("Notepad"),
            ),
            entry("desktop_describe", "error", Some("DESKTOP_TIMEOUT"), None),
        ];
        let rows = rows(&entries);
        assert_eq!(rows[0].tool, "desktop_describe");
        assert_eq!(rows[0].tone, Tone::Error);
        assert_eq!(rows[1].tone, Tone::Denied);
        assert_eq!(rows[1].app, "Notepad");
        assert_eq!(rows[1].outcome, "DESKTOP_APP_NOT_ALLOWED");
        assert_eq!(rows[2].tone, Tone::Ok);
        assert_eq!(rows[2].outcome, "ok");
        assert_eq!(rows[2].app, "-");
    }

    #[test]
    fn an_empty_log_gives_no_rows() {
        assert!(rows(&[]).is_empty());
    }
}
