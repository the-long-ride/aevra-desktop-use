//! Linux: freedesktop `.desktop` entries.
//!
//! The parsing is pure and compiled everywhere. Entries that are hidden, or
//! whose `Exec` cannot be resolved to a real executable, are skipped, and so
//! are entries that launch through a generic launcher such as `flatpak` or
//! `env`: allowing those would allow everything they start.

use super::{is_launcher, AppEntry};
use desktop_policy::AppIdentity;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DesktopEntry {
    pub name: String,
    pub exec: Option<String>,
    pub icon: Option<String>,
    pub no_display: bool,
    pub hidden: bool,
    pub kind: Option<String>,
}

/// The `[Desktop Entry]` group of a `.desktop` file. Localised keys
/// (`Name[de]`) and other groups (`[Desktop Action ...]`) are ignored.
pub fn parse_desktop_entry(text: &str) -> DesktopEntry {
    let mut entry = DesktopEntry::default();
    let mut in_group = false;
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') {
            in_group = line == "[Desktop Entry]";
            continue;
        }
        if !in_group {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let value = value.trim();
        match key.trim() {
            "Name" => entry.name = value.to_string(),
            "Exec" => entry.exec = Some(value.to_string()),
            "Icon" => entry.icon = Some(value.to_string()),
            "Type" => entry.kind = Some(value.to_string()),
            "NoDisplay" => entry.no_display = value.eq_ignore_ascii_case("true"),
            "Hidden" => entry.hidden = value.eq_ignore_ascii_case("true"),
            _ => {}
        }
    }
    entry
}

/// Splits an `Exec` value into words, honouring double quotes and backslash
/// escapes, and dropping field codes (`%u`, `%F`, ...). `%%` is a literal `%`.
pub fn exec_words(exec: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut current = String::new();
    let mut started = false;
    let mut in_quotes = false;
    let mut chars = exec.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' => {
                in_quotes = !in_quotes;
                started = true;
            }
            '\\' if in_quotes => {
                if let Some(next) = chars.next() {
                    current.push(next);
                    started = true;
                }
            }
            // `%%` is a literal percent sign; any other `%x` is a field code
            // and is dropped along with its argument slot.
            '%' => {
                if chars.next() == Some('%') {
                    current.push('%');
                    started = true;
                }
            }
            c if c.is_whitespace() && !in_quotes => {
                if started && !current.is_empty() {
                    words.push(std::mem::take(&mut current));
                }
                started = false;
            }
            c => {
                current.push(c);
                started = true;
            }
        }
    }
    if started && !current.is_empty() {
        words.push(current);
    }
    words
}

fn is_executable_file(path: &Path) -> bool {
    let Ok(meta) = std::fs::metadata(path) else {
        return false;
    };
    if !meta.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        meta.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

/// The absolute executable an `Exec` value starts, looking a bare command up
/// in `path_var`. `env VAR=value cmd` is looked through to `cmd`.
pub fn resolve_exec(exec: &str, path_var: &std::ffi::OsStr) -> Option<PathBuf> {
    let words = exec_words(exec);
    let mut words = words.iter().map(String::as_str);
    let mut command = words.next()?;
    if command == "env" || command.ends_with("/env") {
        command = words.find(|w| !w.contains('=') && !w.starts_with('-'))?;
    }
    if command.contains('/') {
        let candidate = PathBuf::from(command);
        return (candidate.is_absolute() && is_executable_file(&candidate)).then_some(candidate);
    }
    std::env::split_paths(path_var)
        .map(|dir| dir.join(command))
        .find(|candidate| is_executable_file(candidate))
}

/// The entry for one `.desktop` file, if it is a visible, launchable app.
pub fn entry_from_desktop_file(text: &str, path_var: &std::ffi::OsStr) -> Option<AppEntry> {
    let parsed = parse_desktop_entry(text);
    if parsed.no_display
        || parsed.hidden
        || parsed.name.is_empty()
        || parsed.kind.as_deref().is_some_and(|k| k != "Application")
    {
        return None;
    }
    let exe = resolve_exec(parsed.exec.as_deref()?, path_var)?;
    let exe = exe.to_string_lossy().into_owned();
    if is_launcher(&exe) {
        return None;
    }
    let icon_path = parsed
        .icon
        .as_deref()
        .map(PathBuf::from)
        .filter(|p| p.is_absolute() && p.is_file());
    Some(AppEntry {
        name: parsed.name,
        identity: AppIdentity::Exe { path: exe },
        icon_path,
        running: false,
    })
}

/// Entries for every `.desktop` file directly inside `dirs`.
pub fn scan_dirs(dirs: &[PathBuf], path_var: &std::ffi::OsStr) -> Vec<AppEntry> {
    let mut entries = Vec::new();
    for dir in dirs {
        let Ok(read) = std::fs::read_dir(dir) else {
            continue;
        };
        for file in read.flatten().map(|e| e.path()) {
            if file.extension().is_none_or(|e| e != "desktop") {
                continue;
            }
            if let Ok(text) = std::fs::read_to_string(&file) {
                entries.extend(entry_from_desktop_file(&text, path_var));
            }
        }
    }
    entries
}

/// `$XDG_DATA_HOME/applications` (default `~/.local/share`) and each of
/// `$XDG_DATA_DIRS` (default `/usr/local/share:/usr/share`).
pub fn application_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    let home_data = std::env::var_os("XDG_DATA_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")));
    dirs.extend(home_data.map(|d| d.join("applications")));
    let data_dirs = std::env::var_os("XDG_DATA_DIRS")
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| "/usr/local/share:/usr/share".into());
    dirs.extend(std::env::split_paths(&data_dirs).map(|d| d.join("applications")));
    dirs
}

pub fn scan() -> Vec<AppEntry> {
    let path_var = std::env::var_os("PATH").unwrap_or_default();
    scan_dirs(&application_dirs(), &path_var)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;

    const EXAMPLE: &str = include_str!("../../tests/fixtures/example.desktop");
    const HIDDEN: &str = include_str!("../../tests/fixtures/hidden.desktop");

    /// A directory on a temporary PATH holding executable files named `names`.
    fn path_with(names: &[&str]) -> (tempfile::TempDir, OsString) {
        let dir = tempfile::tempdir().unwrap();
        for name in names {
            let file = dir.path().join(name);
            std::fs::write(&file, b"#!/bin/sh\n").unwrap();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o755)).unwrap();
            }
        }
        let path_var = std::env::join_paths([dir.path()]).unwrap();
        (dir, path_var)
    }

    #[test]
    fn the_main_group_is_parsed_and_actions_and_locales_are_ignored() {
        let parsed = parse_desktop_entry(EXAMPLE);
        assert_eq!(parsed.name, "Example");
        assert_eq!(parsed.exec.as_deref(), Some("example %U"));
        assert_eq!(parsed.icon.as_deref(), Some("example"));
        assert!(!parsed.no_display && !parsed.hidden);
    }

    #[test]
    fn exec_words_drop_field_codes_and_honour_quotes() {
        assert_eq!(exec_words("example %U"), ["example"]);
        assert_eq!(exec_words("example --flag %f %F %i"), ["example", "--flag"]);
        assert_eq!(
            exec_words(r#""/opt/My App/app" --name "a b" 100%%"#),
            ["/opt/My App/app", "--name", "a b", "100%"]
        );
        assert!(exec_words("   ").is_empty());
    }

    #[test]
    fn exec_resolves_a_bare_command_through_path() {
        let (dir, path_var) = path_with(&["example"]);
        let resolved = resolve_exec("example %U", &path_var).unwrap();
        assert_eq!(resolved, dir.path().join("example"));
        assert_eq!(resolve_exec("missing", &path_var), None);
    }

    #[cfg(unix)]
    #[test]
    fn exec_with_an_absolute_path_must_exist() {
        let (dir, path_var) = path_with(&["tool"]);
        let absolute = dir.path().join("tool");
        let exec = format!("\"{}\" %U", absolute.display());
        assert_eq!(resolve_exec(&exec, &path_var), Some(absolute));
        assert_eq!(resolve_exec("/no/such/dir/tool", &path_var), None);
    }

    #[test]
    fn env_wrappers_are_looked_through() {
        let (dir, path_var) = path_with(&["real", "env"]);
        assert_eq!(
            resolve_exec("env GDK_BACKEND=x11 -u FOO real %U", &path_var),
            // `-u FOO` leaves FOO as the first plain word, which is not on PATH.
            None
        );
        assert_eq!(
            resolve_exec("env GDK_BACKEND=x11 real %U", &path_var),
            Some(dir.path().join("real"))
        );
    }

    #[test]
    fn the_fixture_entry_becomes_an_app_through_a_temporary_path() {
        let (dir, path_var) = path_with(&["example"]);
        let entry = entry_from_desktop_file(EXAMPLE, &path_var).unwrap();
        assert_eq!(entry.name, "Example");
        assert_eq!(
            entry.identity,
            AppIdentity::Exe {
                path: dir.path().join("example").to_string_lossy().into_owned()
            }
        );
    }

    #[test]
    fn hidden_and_unresolvable_entries_are_skipped() {
        let (_dir, path_var) = path_with(&["example"]);
        assert!(entry_from_desktop_file(HIDDEN, &path_var).is_none());
        assert!(entry_from_desktop_file(
            "[Desktop Entry]\nName=X\nHidden=true\nExec=example",
            &path_var
        )
        .is_none());
        assert!(
            entry_from_desktop_file("[Desktop Entry]\nName=X\nExec=nothing-here", &path_var)
                .is_none()
        );
        assert!(entry_from_desktop_file(
            "[Desktop Entry]\nName=X\nType=Link\nExec=example",
            &path_var
        )
        .is_none());
        assert!(entry_from_desktop_file("[Desktop Entry]\nName=X", &path_var).is_none());
    }

    #[test]
    fn launcher_executables_are_not_offered() {
        let (_dir, path_var) = path_with(&["flatpak", "sh"]);
        for exec in ["flatpak run org.example.App", "sh -c app"] {
            let text = format!("[Desktop Entry]\nName=Wrapped\nExec={exec}");
            assert!(
                entry_from_desktop_file(&text, &path_var).is_none(),
                "{exec}"
            );
        }
    }

    #[test]
    fn scanning_directories_reads_only_desktop_files() {
        let (_bin, path_var) = path_with(&["example"]);
        let apps = tempfile::tempdir().unwrap();
        std::fs::write(apps.path().join("example.desktop"), EXAMPLE).unwrap();
        std::fs::write(apps.path().join("hidden.desktop"), HIDDEN).unwrap();
        std::fs::write(apps.path().join("readme.txt"), EXAMPLE).unwrap();
        let entries = scan_dirs(
            &[
                apps.path().to_path_buf(),
                PathBuf::from("/definitely/not/here"),
            ],
            &path_var,
        );
        assert_eq!(entries.len(), 1, "{entries:?}");
        assert_eq!(entries[0].name, "Example");
    }

    #[test]
    fn duplicates_across_directories_collapse() {
        let (_bin, path_var) = path_with(&["example"]);
        let first = tempfile::tempdir().unwrap();
        let second = tempfile::tempdir().unwrap();
        std::fs::write(first.path().join("a.desktop"), EXAMPLE).unwrap();
        std::fs::write(second.path().join("b.desktop"), EXAMPLE).unwrap();
        let entries = scan_dirs(
            &[first.path().to_path_buf(), second.path().to_path_buf()],
            &path_var,
        );
        assert_eq!(entries.len(), 2);
        assert_eq!(super::super::dedupe(entries).len(), 1);
    }
}
