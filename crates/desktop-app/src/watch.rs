//! Hot reload of the allowlist config.
//!
//! The directory is watched rather than the file, because [`desktop_policy::config::save`]
//! replaces the file by renaming a temporary one over it, which swaps the
//! inode a file watch would be attached to.

use desktop_policy::config::{load, validate, Config};
use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::path::Path;
use std::sync::mpsc;
use std::time::Duration;

/// Events are coalesced until the directory has been quiet this long.
pub const DEBOUNCE: Duration = Duration::from_millis(200);

/// Why a config on disk was not applied.
fn load_valid(path: &Path) -> Result<Config, String> {
    let config = load(path).map_err(|e| e.to_string())?;
    let problems = validate(&config);
    if problems.is_empty() {
        Ok(config)
    } else {
        Err(problems.join("; "))
    }
}

/// Calls `on_change` with the new config whenever `path` changes to a valid
/// config that differs from the last one applied. An unreadable, malformed or
/// invalid config is reported on stderr and the previous one stays in force.
///
/// Dropping the returned watcher stops the reload thread.
pub fn watch_config(
    path: &Path,
    on_change: impl Fn(Config) + Send + 'static,
) -> notify::Result<RecommendedWatcher> {
    let dir = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
        .to_path_buf();
    std::fs::create_dir_all(&dir)?;
    let file_name = path.file_name().map(|n| n.to_os_string());

    let (tx, rx) = mpsc::channel::<()>();
    let mut watcher = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
        let Ok(event) = event else { return };
        if matches!(event.kind, EventKind::Access(_)) {
            return;
        }
        if event
            .paths
            .iter()
            .any(|p| p.file_name() == file_name.as_deref())
        {
            let _ = tx.send(());
        }
    })?;
    watcher.watch(&dir, RecursiveMode::NonRecursive)?;

    // Capture the baseline before returning; the worker may start after a save.
    let mut last = load_valid(path).ok();
    let path = path.to_path_buf();
    std::thread::Builder::new()
        .name("config-watch".into())
        .spawn(move || {
            // Ends when the watcher, and with it the sender, is dropped.
            while rx.recv().is_ok() {
                while rx.recv_timeout(DEBOUNCE).is_ok() {}
                match load_valid(&path) {
                    Ok(config) => {
                        if last.as_ref() != Some(&config) {
                            last = Some(config.clone());
                            on_change(config);
                        }
                    }
                    Err(problem) => {
                        eprintln!("aevra-desktop: ignoring invalid config, keeping the previous one: {problem}");
                    }
                }
            }
        })
        .map_err(notify::Error::io)?;
    Ok(watcher)
}

#[cfg(test)]
mod tests {
    use super::*;
    use desktop_core::{
        ActRequest, BackendHandle, Capabilities, CaptureRequest, CaptureResult, DescribeRequest,
        DescribeResult, DesktopBackend, ScreenState, WindowIdentity,
    };
    use desktop_mcp::apps::EmptyCatalog;
    use desktop_mcp::Server;
    use desktop_policy::activity::ActivityLog;
    use desktop_policy::config::save;
    use desktop_policy::{AllowedApp, AppIdentity};
    use std::sync::{Arc, Mutex};
    use std::time::Instant;

    fn app(name: &str) -> AllowedApp {
        AllowedApp::new(
            name,
            AppIdentity::Exe {
                path: format!("C:/Example/{name}.exe"),
            },
        )
    }

    fn base_config(apps: Vec<AllowedApp>) -> Config {
        let mut config = Config::default();
        config.http.token = "placeholder-token-for-tests".to_string();
        config.allowed_apps = apps;
        config
    }

    fn collector() -> (Arc<Mutex<Vec<Config>>>, impl Fn(Config) + Send + 'static) {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&seen);
        (seen, move |config| sink.lock().unwrap().push(config))
    }

    fn wait_for(deadline: Duration, mut done: impl FnMut() -> bool) -> bool {
        let start = Instant::now();
        while start.elapsed() < deadline {
            if done() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        done()
    }

    #[test]
    fn saving_a_new_config_triggers_on_change_within_two_seconds() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        save(&path, &base_config(vec![])).unwrap();
        let (seen, on_change) = collector();
        let _watcher = watch_config(&path, on_change).unwrap();

        let next = base_config(vec![app("one")]);
        save(&path, &next).unwrap();
        assert!(wait_for(Duration::from_secs(2), || !seen
            .lock()
            .unwrap()
            .is_empty()));
        assert_eq!(seen.lock().unwrap().last().unwrap(), &next);
    }

    #[test]
    fn invalid_json_does_not_trigger_and_the_next_valid_save_still_does() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        save(&path, &base_config(vec![])).unwrap();
        let (seen, on_change) = collector();
        let _watcher = watch_config(&path, on_change).unwrap();

        std::fs::write(&path, "{ this is not json").unwrap();
        std::thread::sleep(Duration::from_millis(800));
        assert!(seen.lock().unwrap().is_empty());
        // The broken file is left alone, not rewritten.
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "{ this is not json"
        );

        let fixed = base_config(vec![app("two")]);
        save(&path, &fixed).unwrap();
        assert!(wait_for(Duration::from_secs(2), || !seen
            .lock()
            .unwrap()
            .is_empty()));
        assert_eq!(seen.lock().unwrap().last().unwrap(), &fixed);
    }

    #[test]
    fn a_config_that_parses_but_does_not_validate_is_ignored() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        save(&path, &base_config(vec![])).unwrap();
        let (seen, on_change) = collector();
        let _watcher = watch_config(&path, on_change).unwrap();

        let mut bad = base_config(vec![app("dup"), app("dup")]);
        bad.allowed_apps[1].id = bad.allowed_apps[0].id.clone();
        save(&path, &bad).unwrap();
        std::thread::sleep(Duration::from_millis(800));
        assert!(seen.lock().unwrap().is_empty());
    }

    #[test]
    fn a_burst_of_saves_ends_with_the_final_config() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        save(&path, &base_config(vec![])).unwrap();
        let (seen, on_change) = collector();
        let _watcher = watch_config(&path, on_change).unwrap();

        let mut last = base_config(vec![]);
        for n in 0..6 {
            last = base_config(vec![app(&format!("burst{n}"))]);
            save(&path, &last).unwrap();
        }
        assert!(wait_for(Duration::from_secs(2), || seen
            .lock()
            .unwrap()
            .last()
            .is_some_and(|c| c == &last)));
        // Debounced: far fewer reloads than saves.
        assert!(
            seen.lock().unwrap().len() <= 3,
            "{}",
            seen.lock().unwrap().len()
        );
    }

    #[test]
    fn saving_an_unchanged_config_does_not_call_on_change() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        let config = base_config(vec![app("same")]);
        save(&path, &config).unwrap();
        let (seen, on_change) = collector();
        let _watcher = watch_config(&path, on_change).unwrap();
        save(&path, &config).unwrap();
        std::thread::sleep(Duration::from_millis(800));
        assert!(seen.lock().unwrap().is_empty());
    }

    /// A backend that never gets called: these tests only swap configs.
    struct NoBackend;

    impl DesktopBackend for NoBackend {
        fn connect(&self) -> Result<Capabilities, String> {
            Err("not used".into())
        }
        fn windows(&self) -> Result<Vec<WindowIdentity>, String> {
            Err("not used".into())
        }
        fn focused_window(&self) -> Result<WindowIdentity, String> {
            Err("not used".into())
        }
        fn screen_state(&self) -> Result<ScreenState, String> {
            Err("not used".into())
        }
        fn describe(&self, _: DescribeRequest) -> Result<DescribeResult, String> {
            Err("not used".into())
        }
        fn act(&self, _: ActRequest) -> Result<bool, String> {
            Err("not used".into())
        }
        fn capture(&self, _: CaptureRequest) -> Result<CaptureResult, String> {
            Err("not used".into())
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn removing_an_app_on_disk_reaches_the_running_server() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        let kept = app("kept");
        let removed = app("removed");
        let initial = base_config(vec![kept.clone(), removed.clone()]);
        save(&path, &initial).unwrap();

        let server = Arc::new(Server::new(
            BackendHandle::spawn(|| NoBackend),
            initial,
            ActivityLog::new(dir.path()),
            Arc::new(EmptyCatalog),
        ));
        let runtime = tokio::runtime::Handle::current();
        let applying = Arc::clone(&server);
        let _watcher = watch_config(&path, move |config| {
            runtime.block_on(applying.apply_config(config));
        })
        .unwrap();

        save(&path, &base_config(vec![kept.clone()])).unwrap();
        assert!(wait_for(Duration::from_secs(2), || server
            .config()
            .allowed_apps
            .len()
            == 1));
        assert_eq!(server.config().allowed_apps, vec![kept]);
    }
}
