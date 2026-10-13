//! Where aevra-desktop keeps its files.

use std::path::{Path, PathBuf};

/// Directory name under the platform's config root.
pub const APP_DIR: &str = "aevra-desktop";
/// Environment variable that replaces the whole config directory.
pub const HOME_OVERRIDE: &str = "AEVRA_DESKTOP_HOME";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Os {
    Windows,
    MacOs,
    Linux,
}

impl Os {
    pub fn current() -> Os {
        if cfg!(windows) {
            Os::Windows
        } else if cfg!(target_os = "macos") {
            Os::MacOs
        } else {
            Os::Linux
        }
    }
}

/// The config directory for `os`, reading variables through `env`.
///
/// - `AEVRA_DESKTOP_HOME`, when set and non-empty, is used as is.
/// - Windows: `%APPDATA%\aevra-desktop`.
/// - macOS: `~/Library/Application Support/aevra-desktop`.
/// - Linux: `$XDG_CONFIG_HOME/aevra-desktop`, default `~/.config/aevra-desktop`.
pub fn config_dir(env: &dyn Fn(&str) -> Option<String>, os: Os) -> PathBuf {
    let get = |key: &str| env(key).filter(|v| !v.trim().is_empty());
    if let Some(dir) = get(HOME_OVERRIDE) {
        return PathBuf::from(dir);
    }
    let home = || {
        get("HOME")
            .or_else(|| get("USERPROFILE"))
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."))
    };
    match os {
        Os::Windows => get("APPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|| home().join("AppData").join("Roaming"))
            .join(APP_DIR),
        Os::MacOs => home()
            .join("Library")
            .join("Application Support")
            .join(APP_DIR),
        Os::Linux => get("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| home().join(".config"))
            .join(APP_DIR),
    }
}

/// The config directory for the running process.
pub fn native_config_dir() -> PathBuf {
    config_dir(&|key| std::env::var(key).ok(), Os::current())
}

pub fn config_file(dir: &Path) -> PathBuf {
    dir.join("config.json")
}

pub fn activity_file(dir: &Path) -> PathBuf {
    dir.join("activity.jsonl")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn env(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let map: HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        move |key| map.get(key).cloned()
    }

    #[test]
    fn windows_uses_appdata() {
        let dir = config_dir(&env(&[("APPDATA", "ROAMING")]), Os::Windows);
        assert_eq!(dir, PathBuf::from("ROAMING").join("aevra-desktop"));
    }

    #[test]
    fn macos_uses_application_support() {
        let dir = config_dir(&env(&[("HOME", "HOMEDIR")]), Os::MacOs);
        assert_eq!(
            dir,
            PathBuf::from("HOMEDIR")
                .join("Library")
                .join("Application Support")
                .join("aevra-desktop")
        );
    }

    #[test]
    fn linux_prefers_xdg_config_home() {
        let dir = config_dir(
            &env(&[("XDG_CONFIG_HOME", "XDG"), ("HOME", "HOMEDIR")]),
            Os::Linux,
        );
        assert_eq!(dir, PathBuf::from("XDG").join("aevra-desktop"));
    }

    #[test]
    fn linux_falls_back_to_dot_config() {
        let dir = config_dir(&env(&[("HOME", "HOMEDIR")]), Os::Linux);
        assert_eq!(
            dir,
            PathBuf::from("HOMEDIR")
                .join(".config")
                .join("aevra-desktop")
        );
    }

    #[test]
    fn override_wins_on_every_os() {
        for os in [Os::Windows, Os::MacOs, Os::Linux] {
            let dir = config_dir(
                &env(&[
                    ("AEVRA_DESKTOP_HOME", "OVERRIDE"),
                    ("APPDATA", "ROAMING"),
                    ("HOME", "HOMEDIR"),
                ]),
                os,
            );
            assert_eq!(dir, PathBuf::from("OVERRIDE"));
        }
    }

    #[test]
    fn blank_override_is_ignored() {
        let dir = config_dir(
            &env(&[("AEVRA_DESKTOP_HOME", "  "), ("APPDATA", "ROAMING")]),
            Os::Windows,
        );
        assert_eq!(dir, PathBuf::from("ROAMING").join("aevra-desktop"));
    }

    #[test]
    fn file_names() {
        let dir = PathBuf::from("D");
        assert_eq!(config_file(&dir), dir.join("config.json"));
        assert_eq!(activity_file(&dir), dir.join("activity.jsonl"));
    }
}
