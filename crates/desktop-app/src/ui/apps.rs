//! The Apps tab's logic: which rows to show and what a toggle does to the
//! config. No egui here, so it is all unit tested.

use crate::scan::{identity_key, AppEntry};
use desktop_policy::{AllowedApp, AppIdentity, Config};
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppRow {
    pub name: String,
    pub identity: AppIdentity,
    pub allowed: bool,
    pub running: bool,
    /// False for allowed apps the scan did not find (custom entries, or apps
    /// uninstalled since). They stay listed so they can be removed.
    pub installed_entry: bool,
}

/// The rows for the Apps tab: every scanned app plus every allowed app the
/// scan did not find, filtered by `search` (case-insensitive, on the name),
/// allowed apps first and then by name.
pub fn apps_view_model(catalog: &[AppEntry], config: &Config, search: &str) -> Vec<AppRow> {
    let needle = search.trim().to_lowercase();
    let allowed_keys: Vec<(String, &AllowedApp)> = config
        .allowed_apps
        .iter()
        .map(|a| (identity_key(&a.identity), a))
        .collect();

    let mut rows: Vec<AppRow> = catalog
        .iter()
        .map(|entry| {
            let key = identity_key(&entry.identity);
            AppRow {
                name: entry.name.clone(),
                identity: entry.identity.clone(),
                allowed: allowed_keys.iter().any(|(k, _)| *k == key),
                running: entry.running,
                installed_entry: true,
            }
        })
        .collect();

    let scanned: Vec<String> = catalog.iter().map(|e| identity_key(&e.identity)).collect();
    for (key, app) in &allowed_keys {
        if !scanned.contains(key) {
            rows.push(AppRow {
                name: app.name.clone(),
                identity: app.identity.clone(),
                allowed: true,
                running: false,
                installed_entry: false,
            });
        }
    }

    rows.retain(|r| needle.is_empty() || r.name.to_lowercase().contains(&needle));
    rows.sort_by(|a, b| {
        b.allowed
            .cmp(&a.allowed)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    rows
}

/// Allows or disallows `identity`. Returns whether the config changed. Allowing
/// an app that is already allowed, or removing one that is not, changes nothing.
pub fn toggle_app(config: &mut Config, name: &str, identity: &AppIdentity, allow: bool) -> bool {
    let key = identity_key(identity);
    let present = config
        .allowed_apps
        .iter()
        .any(|a| identity_key(&a.identity) == key);
    if allow {
        if present {
            return false;
        }
        config
            .allowed_apps
            .push(AllowedApp::new(name, identity.clone()));
        true
    } else {
        let before = config.allowed_apps.len();
        config
            .allowed_apps
            .retain(|a| identity_key(&a.identity) != key);
        config.allowed_apps.len() != before
    }
}

/// What identifies the app, for the row's secondary text.
pub fn identity_label(identity: &AppIdentity) -> String {
    match identity {
        AppIdentity::Exe { path } | AppIdentity::Bundle { path } => path.clone(),
        AppIdentity::Package { family_name } => format!("Store package {family_name}"),
    }
}

/// The name and identity for a file the user picked: a `.app` bundle becomes a
/// `Bundle`, anything else an `Exe`. Directories that are not bundles, and
/// empty paths, are not apps.
pub fn custom_identity(path: &Path) -> Option<(String, AppIdentity)> {
    let text = path.to_string_lossy().trim().to_string();
    if text.is_empty() {
        return None;
    }
    let is_bundle = path.extension().is_some_and(|e| e == "app");
    if path.is_dir() && !is_bundle {
        return None;
    }
    let name = path.file_stem()?.to_string_lossy().into_owned();
    if name.is_empty() {
        return None;
    }
    let identity = if is_bundle {
        AppIdentity::Bundle { path: text }
    } else {
        AppIdentity::Exe { path: text }
    };
    Some((name, identity))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn exe(path: &str) -> AppIdentity {
        AppIdentity::Exe { path: path.into() }
    }

    fn entry(name: &str, path: &str, running: bool) -> AppEntry {
        AppEntry {
            name: name.into(),
            identity: exe(path),
            icon_path: None,
            running,
        }
    }

    fn config_with(apps: &[(&str, &str)]) -> Config {
        Config {
            allowed_apps: apps
                .iter()
                .map(|(name, path)| AllowedApp::new(*name, exe(path)))
                .collect(),
            ..Config::default()
        }
    }

    #[test]
    fn rows_mark_allowed_apps_and_list_allowed_first() {
        let catalog = vec![
            entry("Beta", "/opt/beta", false),
            entry("Alpha", "/opt/alpha", true),
            entry("Gamma", "/opt/gamma", false),
        ];
        let config = config_with(&[("Gamma", "/opt/gamma")]);
        let rows = apps_view_model(&catalog, &config, "");
        let names: Vec<_> = rows.iter().map(|r| (r.name.as_str(), r.allowed)).collect();
        assert_eq!(names, [("Gamma", true), ("Alpha", false), ("Beta", false)]);
        assert!(rows[1].running);
    }

    #[test]
    fn allowed_apps_the_scan_missed_are_still_listed() {
        let config = config_with(&[("Custom Tool", "/opt/custom")]);
        let rows = apps_view_model(&[entry("Alpha", "/opt/alpha", false)], &config, "");
        assert_eq!(rows.len(), 2);
        assert!(!rows[0].installed_entry);
        assert_eq!(rows[0].name, "Custom Tool");
        assert!(rows[0].allowed);
    }

    #[test]
    fn search_filters_by_name_ignoring_case_and_padding() {
        let catalog = vec![
            entry("Notepad", "/opt/notepad", false),
            entry("Calculator", "/opt/calc", false),
        ];
        let rows = apps_view_model(&catalog, &Config::default(), "  NOTE ");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].name, "Notepad");
        assert!(apps_view_model(&catalog, &Config::default(), "zzz").is_empty());
    }

    #[test]
    fn allowing_adds_one_entry_with_a_fresh_id_and_is_idempotent() {
        let mut config = Config::default();
        assert!(toggle_app(&mut config, "Alpha", &exe("/opt/alpha"), true));
        assert_eq!(config.allowed_apps.len(), 1);
        assert!(config.allowed_apps[0].id.starts_with("app_"));
        assert!(!toggle_app(&mut config, "Alpha", &exe("/opt/alpha"), true));
        assert_eq!(config.allowed_apps.len(), 1);
    }

    #[test]
    fn disallowing_removes_every_entry_for_the_identity_and_only_that() {
        let mut config = config_with(&[("A", "/opt/a"), ("B", "/opt/b")]);
        assert!(toggle_app(&mut config, "A", &exe("/opt/a/"), false));
        assert_eq!(config.allowed_apps.len(), 1);
        assert_eq!(config.allowed_apps[0].name, "B");
        assert!(!toggle_app(&mut config, "A", &exe("/opt/a"), false));
    }

    #[test]
    fn toggling_keeps_the_http_settings() {
        let mut config = Config::default();
        config.http.port = 4999;
        config.http.token = "placeholder-token".into();
        toggle_app(&mut config, "Alpha", &exe("/opt/alpha"), true);
        assert_eq!(config.http.port, 4999);
        assert_eq!(config.http.token, "placeholder-token");
    }

    #[test]
    fn identity_labels_show_the_path_or_the_package() {
        assert_eq!(identity_label(&exe("/opt/a")), "/opt/a");
        assert_eq!(
            identity_label(&AppIdentity::Package {
                family_name: "Example.App_abc".into()
            }),
            "Store package Example.App_abc"
        );
    }

    #[test]
    fn a_picked_file_becomes_an_exe_and_a_bundle_becomes_a_bundle() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("tool.exe");
        std::fs::write(&file, b"x").unwrap();
        let (name, identity) = custom_identity(&file).unwrap();
        assert_eq!(name, "tool");
        assert_eq!(
            identity,
            AppIdentity::Exe {
                path: file.to_string_lossy().into_owned()
            }
        );

        let bundle = dir.path().join("Thing.app");
        std::fs::create_dir_all(&bundle).unwrap();
        let (name, identity) = custom_identity(&bundle).unwrap();
        assert_eq!(name, "Thing");
        assert!(matches!(identity, AppIdentity::Bundle { .. }));
    }

    #[test]
    fn plain_directories_and_empty_paths_are_not_apps() {
        let dir = tempfile::tempdir().unwrap();
        assert!(custom_identity(dir.path()).is_none());
        assert!(custom_identity(Path::new("")).is_none());
        assert!(custom_identity(Path::new("   ")).is_none());
    }
}
