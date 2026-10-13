//! Installed-app discovery. Each platform module knows where apps are
//! registered; all of the parsing is pure and compiled everywhere so it is
//! tested on every host. Only the functions that read the real machine are
//! reached through [`native_scan`].

pub mod linux;
pub mod macos;
pub mod windows;

use desktop_policy::gate::{canonical_path, identity_matches_path};
use desktop_policy::AppIdentity;
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

pub use desktop_mcp::apps::{AppCatalog, AppEntry};

/// How long a scan result is reused.
pub const CACHE_TTL: Duration = Duration::from_secs(30);

type Scanner = Box<dyn Fn() -> Vec<AppEntry> + Send + Sync>;

/// A cached view of the installed apps, scanned on demand.
pub struct Catalog {
    scanner: Scanner,
    ttl: Duration,
    cache: Mutex<Option<(Instant, Vec<AppEntry>)>>,
}

impl Catalog {
    /// Scans this machine's real app registrations.
    pub fn native() -> Self {
        Self::with_scanner(native_scan, CACHE_TTL)
    }

    pub fn with_scanner(
        scanner: impl Fn() -> Vec<AppEntry> + Send + Sync + 'static,
        ttl: Duration,
    ) -> Self {
        Self {
            scanner: Box::new(scanner),
            ttl,
            cache: Mutex::new(None),
        }
    }

    /// Rescans now and replaces the cache.
    pub fn refresh(&self) -> Vec<AppEntry> {
        let mut cache = self.cache.lock().unwrap();
        let entries = (self.scanner)();
        *cache = Some((Instant::now(), entries.clone()));
        entries
    }
}

impl AppCatalog for Catalog {
    fn list(&self) -> Vec<AppEntry> {
        {
            let cache = self.cache.lock().unwrap();
            if let Some((at, entries)) = cache.as_ref() {
                if at.elapsed() < self.ttl {
                    return entries.clone();
                }
            }
        }
        self.refresh()
    }
}

/// Registered apps with `running` filled in and duplicates merged.
pub fn native_scan() -> Vec<AppEntry> {
    let mut entries = if cfg!(windows) {
        windows::scan()
    } else if cfg!(target_os = "macos") {
        macos::scan()
    } else {
        linux::scan()
    };
    mark_running(&mut entries, &running_exe_paths());
    dedupe(entries)
}

/// Executable paths of every running process whose path the OS will tell us.
pub fn running_exe_paths() -> Vec<String> {
    use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, RefreshKind, System, UpdateKind};
    let kind = ProcessRefreshKind::nothing().with_exe(UpdateKind::OnlyIfNotSet);
    let mut system = System::new_with_specifics(RefreshKind::nothing().with_processes(kind));
    system.refresh_processes_specifics(ProcessesToUpdate::All, true, kind);
    system
        .processes()
        .values()
        .filter_map(|p| p.exe())
        .map(|p| p.to_string_lossy().into_owned())
        .collect()
}

/// Sets `running` on each entry, by the same identity rules the gate uses.
pub fn mark_running(entries: &mut [AppEntry], running_exes: &[String]) {
    for entry in entries {
        entry.running = running_exes
            .iter()
            .any(|exe| identity_matches_path(&entry.identity, exe));
    }
}

/// The key two entries share when they are the same app.
pub fn identity_key(identity: &AppIdentity) -> String {
    match identity {
        AppIdentity::Exe { path } => format!("exe:{}", canonical_path(path)),
        AppIdentity::Bundle { path } => format!("bundle:{}", canonical_path(path)),
        AppIdentity::Package { family_name } => {
            format!("package:{}", family_name.trim().to_lowercase())
        }
    }
}

/// Merges entries with the same canonical identity (first name wins, a
/// running or iconed duplicate contributes those) and sorts by name.
pub fn dedupe(entries: Vec<AppEntry>) -> Vec<AppEntry> {
    let mut seen: HashMap<String, usize> = HashMap::new();
    let mut merged: Vec<AppEntry> = Vec::new();
    for entry in entries {
        let key = identity_key(&entry.identity);
        match seen.get(&key) {
            Some(&at) => {
                let kept = &mut merged[at];
                kept.running |= entry.running;
                if kept.icon_path.is_none() {
                    kept.icon_path = entry.icon_path;
                }
                if kept.name.trim().is_empty() {
                    kept.name = entry.name;
                }
            }
            None => {
                seen.insert(key, merged.len());
                merged.push(entry);
            }
        }
    }
    merged.sort_by_key(|e| e.name.to_lowercase());
    merged
}

/// Names that launch other programs. Allowing one would allow everything it
/// starts, so a registration whose executable is one of these is not offered.
pub(crate) const LAUNCHER_EXECUTABLES: &[&str] = &[
    "env",
    "sh",
    "bash",
    "dash",
    "zsh",
    "fish",
    "python",
    "python3",
    "perl",
    "ruby",
    "node",
    "java",
    "wine",
    "flatpak",
    "snap",
    "xdg-open",
    "gtk-launch",
    "cmd.exe",
    "powershell.exe",
    "pwsh.exe",
    "wscript.exe",
    "cscript.exe",
    "rundll32.exe",
    "explorer.exe",
];

pub(crate) fn is_launcher(exe_path: &str) -> bool {
    let name = exe_path
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or_default()
        .to_lowercase();
    LAUNCHER_EXECUTABLES.contains(&name.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    fn exe(name: &str, path: &str) -> AppEntry {
        AppEntry {
            name: name.into(),
            identity: AppIdentity::Exe { path: path.into() },
            icon_path: None,
            running: false,
        }
    }

    #[test]
    fn duplicates_collapse_and_keep_what_each_knows() {
        let mut a = exe("Example", "/opt/example/example");
        let mut b = exe("example (copy)", "/opt/example/example/");
        b.running = true;
        b.icon_path = Some("/opt/example/icon.png".into());
        a.running = false;
        let other = exe("Another", "/opt/another/another");
        let merged = dedupe(vec![a, other, b]);
        assert_eq!(merged.len(), 2);
        let example = merged.iter().find(|e| e.name == "Example").unwrap();
        assert!(example.running);
        assert!(example.icon_path.is_some());
        // Sorted by name.
        assert_eq!(merged[0].name, "Another");
    }

    #[test]
    fn package_identities_dedupe_by_family_name_ignoring_case() {
        let package = |family: &str| AppEntry {
            name: "Store App".into(),
            identity: AppIdentity::Package {
                family_name: family.into(),
            },
            icon_path: None,
            running: false,
        };
        assert_eq!(
            dedupe(vec![package("Pub.App_abc"), package("pub.app_ABC")]).len(),
            1
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_paths_dedupe_ignoring_case_and_separators() {
        let merged = dedupe(vec![
            exe("A", r"C:\Program Files\Example\example.exe"),
            exe("B", "c:/program files/example/EXAMPLE.EXE"),
        ]);
        assert_eq!(merged.len(), 1);
    }

    #[cfg(windows)]
    #[test]
    fn running_is_marked_by_canonical_path() {
        let mut entries = vec![
            exe("Example", r"C:\Program Files\Example\example.exe"),
            exe("Idle", r"C:\Program Files\Idle\idle.exe"),
        ];
        mark_running(
            &mut entries,
            &[r"c:\program files\example\EXAMPLE.EXE".to_string()],
        );
        assert!(entries[0].running);
        assert!(!entries[1].running);
    }

    #[cfg(not(windows))]
    #[test]
    fn running_is_marked_by_exact_path_and_bundle_prefix() {
        let mut entries = vec![
            exe("Example", "/opt/example/example"),
            exe("Idle", "/opt/idle/idle"),
            AppEntry {
                name: "Bundle".into(),
                identity: AppIdentity::Bundle {
                    path: "/Applications/Bundle.app".into(),
                },
                icon_path: None,
                running: false,
            },
        ];
        mark_running(
            &mut entries,
            &[
                "/opt/example/example".to_string(),
                "/Applications/Bundle.app/Contents/MacOS/Bundle".to_string(),
            ],
        );
        assert!(entries[0].running);
        assert!(!entries[1].running);
        assert!(entries[2].running);
    }

    #[test]
    fn launchers_are_recognised_by_file_name() {
        assert!(is_launcher("/usr/bin/flatpak"));
        assert!(is_launcher(r"C:\Windows\System32\CMD.EXE"));
        assert!(!is_launcher("/usr/bin/gimp"));
    }

    #[test]
    fn the_catalog_reuses_a_fresh_scan_and_refresh_forces_a_new_one() {
        let scans = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&scans);
        let catalog = Catalog::with_scanner(
            move || {
                counter.fetch_add(1, Ordering::SeqCst);
                vec![exe("Example", "/opt/example/example")]
            },
            Duration::from_secs(60),
        );
        assert_eq!(catalog.list().len(), 1);
        assert_eq!(catalog.list().len(), 1);
        assert_eq!(scans.load(Ordering::SeqCst), 1);
        catalog.refresh();
        assert_eq!(scans.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn the_catalog_rescans_after_the_ttl() {
        let scans = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&scans);
        let catalog = Catalog::with_scanner(
            move || {
                counter.fetch_add(1, Ordering::SeqCst);
                Vec::new()
            },
            Duration::ZERO,
        );
        catalog.list();
        catalog.list();
        assert_eq!(scans.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn the_native_scan_runs_on_this_machine_without_panicking() {
        let entries = native_scan();
        // Contents depend on the machine; every entry must at least be usable.
        for entry in &entries {
            assert!(!entry.name.trim().is_empty(), "{entry:?}");
        }
        eprintln!("native scan found {} apps", entries.len());
    }
}
