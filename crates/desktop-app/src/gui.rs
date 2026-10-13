//! The default mode: tray icon, allowlist window, and (when enabled) the
//! loopback HTTP server, all in one process.

use crate::controller::{self, HttpManager};
use crate::scan::{AppCatalog, Catalog};
use crate::single_instance::{acquire, bind_listener, signal_existing, Acquired};
use crate::tray::{self, Tray};
use crate::ui::{GuiApp, Loaded, Msg, Services};
use crate::watch::watch_config;
use desktop_core::actor;
use desktop_mcp::Server;
use desktop_policy::activity::ActivityLog;
use desktop_policy::config::{generate_token, load, save, validate};
use desktop_policy::paths::{config_file, native_config_dir};
use desktop_policy::Config;
use eframe::egui;
use std::path::Path;
use std::sync::mpsc;
use std::sync::{Arc, OnceLock};
use std::time::Duration;
use tray_icon::menu::MenuEvent;
use tray_icon::TrayIconEvent;

/// Test hook: quit by itself after this many milliseconds.
pub const EXIT_AFTER_ENV: &str = "AEVRA_DESKTOP_EXIT_AFTER_MS";

/// The config for the window and the server, creating the file on first run so
/// the HTTP token is stable. A damaged file is left alone and the program runs
/// with an empty allowlist, which refuses everything.
fn load_config(path: &Path) -> Loaded {
    match load(path) {
        Ok(config) => {
            let problems = validate(&config);
            if !problems.is_empty() {
                return damaged(problems.join("; "));
            }
            if needs_token_saved(path) {
                if let Err(e) = save(path, &config) {
                    eprintln!("aevra-desktop: could not write {}: {e}", path.display());
                }
            }
            Loaded {
                config,
                problem: None,
            }
        }
        Err(e) => damaged(e.to_string()),
    }
}

fn damaged(problem: String) -> Loaded {
    eprintln!("aevra-desktop: the config file is damaged, running with nothing allowed: {problem}");
    let mut config = Config::default();
    config.http.token = generate_token();
    Loaded {
        config,
        problem: Some(problem),
    }
}

/// True when there is no config file yet, or it has no HTTP token.
fn needs_token_saved(path: &Path) -> bool {
    match std::fs::read_to_string(path) {
        Ok(text) => serde_json::from_str::<Config>(&text)
            .map(|c| c.http.token.is_empty())
            .unwrap_or(false),
        Err(_) => true,
    }
}

pub fn run() -> Result<(), String> {
    let dir = native_config_dir();
    let path = config_file(&dir);

    let mut instance =
        match acquire(&dir).map_err(|e| format!("could not take the UI lock: {e}"))? {
            Acquired::AlreadyRunning => {
                if let Err(e) = signal_existing(&dir) {
                    eprintln!("aevra-desktop: it is already running, but it did not answer: {e}");
                }
                return Ok(());
            }
            Acquired::First(instance) => instance,
        };
    let listener =
        bind_listener(&dir).map_err(|e| format!("could not open the wake-up socket: {e}"))?;

    // The window is the only thing that needs the console gone.
    #[cfg(windows)]
    unsafe {
        let _ = windows::Win32::System::Console::FreeConsole();
    }

    let loaded = load_config(&path);
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .map_err(|e| e.to_string())?;

    let catalog = Arc::new(Catalog::native());
    let server = Arc::new(Server::new(
        actor::native(),
        loaded.config.clone(),
        ActivityLog::new(&dir),
        Arc::clone(&catalog) as Arc<dyn AppCatalog>,
    ));
    let http = Arc::new(HttpManager::new(
        Arc::clone(&server),
        runtime.handle().clone(),
    ));
    http.reconcile(&loaded.config.http);

    // Edits to the file, from the window or elsewhere, reach the server here.
    let _watcher = {
        let server = Arc::clone(&server);
        let http = Arc::clone(&http);
        let handle = runtime.handle().clone();
        watch_config(&path, move |config| {
            controller::apply(&server, &http, &handle, config);
        })
        .map_err(|e| format!("could not watch the config: {e}"))?
    };

    let (tx, rx) = mpsc::channel::<Msg>();
    let repaint: Arc<OnceLock<egui::Context>> = Arc::new(OnceLock::new());
    {
        let tx = tx.clone();
        let repaint = Arc::clone(&repaint);
        instance.on_show(
            move || {
                let _ = tx.send(Msg::Show);
                if let Some(ctx) = repaint.get() {
                    ctx.request_repaint();
                }
            },
            listener,
        );
    }

    let exit_after = std::env::var(EXIT_AFTER_ENV)
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .map(Duration::from_millis);
    let services = Services {
        config_path: path.clone(),
        exe: std::env::current_exe().unwrap_or_else(|_| "aevra-desktop".into()),
        catalog,
        http: Arc::clone(&http),
        log: ActivityLog::new(&dir),
    };
    let http_enabled = loaded.config.http.enabled;

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Aevra Desktop")
            .with_icon(crate::branding::icon(64))
            .with_inner_size([980.0, 720.0])
            .with_min_inner_size([600.0, 460.0]),
        ..Default::default()
    };

    let result = eframe::run_native(
        "aevra-desktop",
        options,
        Box::new(move |cc| {
            let _ = repaint.set(cc.egui_ctx.clone());
            let tray = match Tray::new(http_enabled) {
                Ok(tray) => Some(tray),
                Err(e) => {
                    eprintln!("aevra-desktop: no tray icon, closing the window will quit: {e}");
                    None
                }
            };
            if let Some(tray) = &tray {
                let ids = tray.ids();
                let menu_tx = tx.clone();
                let ctx = cc.egui_ctx.clone();
                MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
                    if let Some(action) = ids.action(&event.id) {
                        let _ = menu_tx.send(Msg::Tray(action));
                        ctx.request_repaint();
                    }
                }));
                let click_tx = tx.clone();
                let ctx = cc.egui_ctx.clone();
                TrayIconEvent::set_event_handler(Some(move |event: TrayIconEvent| {
                    if tray::is_open_click(&event) {
                        let _ = click_tx.send(Msg::Show);
                        ctx.request_repaint();
                    }
                }));
            }
            Ok(Box::new(GuiApp::new(
                cc, services, loaded, rx, tray, exit_after,
            )))
        }),
    );
    http.stop();
    drop(instance);
    result.map_err(|e| e.to_string())
}
