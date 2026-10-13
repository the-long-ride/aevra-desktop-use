//! The activity log: one JSON line per tool call, never any argument values.

use crate::paths::activity_file;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};

pub const ROTATE_AT_BYTES: u64 = 5 * 1024 * 1024;

pub const ALLOW: &str = "allow";
pub const DENY: &str = "deny";
pub const ERROR: &str = "error";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActivityEntry {
    /// RFC 3339, UTC.
    pub ts: String,
    pub tool: String,
    /// The allowed app's name, when the call reached one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window_title_len: Option<usize>,
    /// `allow`, `deny` or `error`.
    pub verdict: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    /// Argument name to the length of its string (or the size of its array or
    /// object). Values are never stored.
    #[serde(default)]
    pub arg_lengths: BTreeMap<String, usize>,
}

impl ActivityEntry {
    pub fn new(tool: impl Into<String>, verdict: &str) -> Self {
        Self {
            ts: rfc3339_now(),
            tool: tool.into(),
            app: None,
            window_title_len: None,
            verdict: verdict.to_string(),
            code: None,
            arg_lengths: BTreeMap::new(),
        }
    }
}

/// A draft entry for `tool` with the arguments reduced to lengths: every
/// string argument becomes its character count, arrays and objects their size.
/// Numbers, booleans and nulls are dropped. The verdict starts as `allow`.
pub fn entry_for(tool: &str, args: &Value) -> ActivityEntry {
    let mut entry = ActivityEntry::new(tool, ALLOW);
    if let Some(map) = args.as_object() {
        for (key, value) in map {
            let len = match value {
                Value::String(s) => Some(s.chars().count()),
                Value::Array(a) => Some(a.len()),
                Value::Object(o) => Some(o.len()),
                _ => None,
            };
            if let Some(len) = len {
                entry.arg_lengths.insert(key.clone(), len);
            }
        }
    }
    entry
}

pub struct ActivityLog {
    dir: PathBuf,
    rotate_at: u64,
}

impl ActivityLog {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self {
            dir: dir.into(),
            rotate_at: ROTATE_AT_BYTES,
        }
    }

    fn current(&self) -> PathBuf {
        activity_file(&self.dir)
    }

    fn rotated(&self, n: u32) -> PathBuf {
        let mut name = self.current().into_os_string();
        name.push(format!(".{n}"));
        PathBuf::from(name)
    }

    /// Appends one JSON line. Failures are returned but callers should not let
    /// them fail the tool call.
    ///
    /// Rotation is best-effort between the tray process and any `mcp` process:
    /// a rename that fails is ignored and the append still happens.
    pub fn append(&self, entry: &ActivityEntry) -> std::io::Result<()> {
        std::fs::create_dir_all(&self.dir)?;
        let size = std::fs::metadata(self.current())
            .map(|m| m.len())
            .unwrap_or(0);
        if size >= self.rotate_at {
            let _ = std::fs::remove_file(self.rotated(2));
            let _ = std::fs::rename(self.rotated(1), self.rotated(2));
            let _ = std::fs::rename(self.current(), self.rotated(1));
        }
        let mut line = serde_json::to_vec(entry).map_err(std::io::Error::other)?;
        line.push(b'\n');
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.current())?;
        // One write call keeps concurrent appenders from interleaving a line.
        file.write_all(&line)
    }

    /// All retained records, oldest first, including both rotated files.
    pub fn retained(&self) -> Vec<ActivityEntry> {
        let mut entries = read_entries(&self.rotated(2));
        entries.extend(read_entries(&self.rotated(1)));
        entries.extend(read_entries(&self.current()));
        entries
    }

    /// The last `n` entries, oldest first. Unreadable lines are skipped.
    pub fn tail(&self, n: usize) -> Vec<ActivityEntry> {
        let mut entries = read_entries(&self.current());
        if entries.len() < n {
            let mut older = read_entries(&self.rotated(1));
            older.append(&mut entries);
            entries = older;
        }
        let skip = entries.len().saturating_sub(n);
        entries.split_off(skip)
    }
}

fn read_entries(path: &Path) -> Vec<ActivityEntry> {
    std::fs::read_to_string(path)
        .map(|text| {
            text.lines()
                .filter_map(|l| serde_json::from_str::<ActivityEntry>(l).ok())
                .collect()
        })
        .unwrap_or_default()
}

/// `YYYY-MM-DDTHH:MM:SS.mmmZ` for the current time.
pub fn rfc3339_now() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    rfc3339(now.as_secs() as i64, now.subsec_millis())
}

/// Civil date from days since 1970-01-01 (Howard Hinnant's algorithm).
pub fn rfc3339(secs: i64, millis: u32) -> String {
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let mut year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    if month <= 2 {
        year += 1;
    }
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.{millis:03}Z",
        rem / 3_600,
        rem % 3_600 / 60,
        rem % 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn typed_text_is_replaced_by_its_length() {
        let entry = entry_for("desktop_type", &json!({"text": "hunter2"}));
        let line = serde_json::to_string(&entry).unwrap();
        assert!(!line.contains("hunter2"), "{line}");
        assert_eq!(entry.arg_lengths["text"], 7);
        assert!(line.contains("\"argLengths\":{\"text\":7}"), "{line}");
    }

    #[test]
    fn non_string_arguments_never_leak_values() {
        let entry = entry_for(
            "desktop_click",
            &json!({"x": 123456, "ref": "ref_bg_1_2", "list": [1, 2, 3], "flag": true}),
        );
        let line = serde_json::to_string(&entry).unwrap();
        assert!(!line.contains("123456"));
        assert!(!line.contains("ref_bg_1_2"));
        assert_eq!(entry.arg_lengths["ref"], 10);
        assert_eq!(entry.arg_lengths["list"], 3);
        assert!(!entry.arg_lengths.contains_key("flag"));
    }

    #[test]
    fn rfc3339_known_instants() {
        assert_eq!(rfc3339(0, 0), "1970-01-01T00:00:00.000Z");
        assert_eq!(rfc3339(1_700_000_000, 5), "2023-11-14T22:13:20.005Z");
        assert_eq!(rfc3339(951_782_400, 0), "2000-02-29T00:00:00.000Z");
        assert!(rfc3339_now().ends_with('Z'));
    }

    #[test]
    fn tail_returns_the_last_entries_oldest_first() {
        let dir = tempfile::tempdir().unwrap();
        let log = ActivityLog::new(dir.path());
        for i in 0..6 {
            log.append(&ActivityEntry::new(format!("tool_{i}"), ALLOW))
                .unwrap();
        }
        let tail = log.tail(3);
        let tools: Vec<_> = tail.iter().map(|e| e.tool.as_str()).collect();
        assert_eq!(tools, ["tool_3", "tool_4", "tool_5"]);
        assert_eq!(log.tail(100).len(), 6);
    }

    #[test]
    fn tail_skips_corrupt_lines_and_survives_a_missing_file() {
        let dir = tempfile::tempdir().unwrap();
        let log = ActivityLog::new(dir.path());
        assert!(log.tail(5).is_empty());
        log.append(&ActivityEntry::new("a", ALLOW)).unwrap();
        std::fs::OpenOptions::new()
            .append(true)
            .open(dir.path().join("activity.jsonl"))
            .unwrap()
            .write_all(b"not json\n")
            .unwrap();
        log.append(&ActivityEntry::new("b", DENY)).unwrap();
        assert_eq!(log.tail(5).len(), 2);
    }

    #[test]
    fn rotation_keeps_two_files_and_never_a_third() {
        let dir = tempfile::tempdir().unwrap();
        let log = ActivityLog::new(dir.path());
        // Each line is about 64 KiB, so about 190 of them is well over two
        // rotations' worth.
        let mut big = ActivityEntry::new("x".repeat(64 * 1024), ALLOW);
        for i in 0..190 {
            big.code = Some(i.to_string());
            log.append(&big).unwrap();
        }
        let base = dir.path().join("activity.jsonl");
        assert!(base.exists());
        assert!(dir.path().join("activity.jsonl.1").exists());
        assert!(dir.path().join("activity.jsonl.2").exists());
        assert!(!dir.path().join("activity.jsonl.3").exists());
        let size = std::fs::metadata(&base).unwrap().len();
        assert!(size < ROTATE_AT_BYTES + 70 * 1024, "{size}");
    }
    #[test]
    fn retained_reads_both_rotations_and_skips_corruption() {
        let dir = tempfile::tempdir().unwrap();
        let log = ActivityLog::new(dir.path());
        for (file, tool) in [
            ("activity.jsonl.2", "oldest"),
            ("activity.jsonl.1", "middle"),
            ("activity.jsonl", "newest"),
        ] {
            let line = serde_json::to_string(&ActivityEntry::new(tool, ALLOW)).unwrap();
            std::fs::write(dir.path().join(file), format!("bad line\n{line}\n")).unwrap();
        }
        let tools: Vec<_> = log.retained().into_iter().map(|e| e.tool).collect();
        assert_eq!(tools, ["oldest", "middle", "newest"]);
        assert!(ActivityLog::new(dir.path().join("missing"))
            .retained()
            .is_empty());
    }
}
