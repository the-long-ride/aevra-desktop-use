//! The allowlist window. The logic lives in the three view-model modules and is
//! unit tested; this file only draws it.

pub mod activity;
mod activity_cache;
mod activity_controls;
mod activity_history;
mod activity_query;
mod activity_screen;
pub mod apps;
mod apps_screen;
pub mod settings;
mod settings_screen;
mod shell;
mod theme;

use crate::controller::{HttpManager, HttpStatus};
use crate::scan::{AppCatalog, AppEntry, Catalog};
use crate::tray::{Tray, TrayAction};
use desktop_policy::config::{load, save, validate};
use desktop_policy::Config;
use eframe::egui;
use std::path::PathBuf;
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Things other threads ask the window to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Msg {
    /// A second launch, or a click on the tray icon.
    Show,
    Tray(TrayAction),
}

pub struct Services {
    pub config_path: PathBuf,
    pub exe: PathBuf,
    pub catalog: Arc<Catalog>,
    pub http: Arc<HttpManager>,
    pub log: desktop_policy::activity::ActivityLog,
}

/// The config as the window found it at startup.
pub struct Loaded {
    pub config: Config,
    /// Set when the file exists but is not a usable config. The window then
    /// refuses to save, so it can never overwrite the user's file.
    pub problem: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tab {
    Apps,
    Activity,
    Settings,
}

#[derive(Default)]
struct CatalogView {
    entries: Vec<AppEntry>,
    loading: bool,
}

const REFRESH_EVERY: Duration = Duration::from_secs(1);
/// Right after a hide or show, the close request from the last frame is stale.
const STALE_CLOSE_WINDOW: Duration = Duration::from_millis(400);

pub struct GuiApp {
    services: Services,
    config: Config,
    problem: Option<String>,
    inbox: Receiver<Msg>,
    tray: Option<Tray>,
    tab: Tab,
    search: String,
    catalog: Arc<Mutex<CatalogView>>,
    activity: activity_history::History,
    activity_query: activity_query::Query,
    activity_cache: activity_cache::Cache,
    logo: egui::TextureHandle,
    port_text: String,
    reveal_token: bool,
    status: String,
    last_refresh: Instant,
    hidden: bool,
    quitting: bool,
    ignore_close_until: Instant,
    exit_at: Option<Instant>,
}

impl GuiApp {
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        services: Services,
        loaded: Loaded,
        inbox: Receiver<Msg>,
        tray: Option<Tray>,
        exit_after: Option<Duration>,
    ) -> Self {
        theme::install(&cc.egui_ctx);
        let app = Self {
            port_text: services
                .http
                .effective_port(loaded.config.http.port)
                .to_string(),
            config: loaded.config,
            problem: loaded.problem,
            services,
            inbox,
            tray,
            tab: Tab::Apps,
            search: String::new(),
            catalog: Arc::new(Mutex::new(CatalogView::default())),
            activity: activity_history::History::default(),
            activity_query: activity_query::Query::default(),
            activity_cache: activity_cache::Cache::default(),
            logo: super::branding::texture(&cc.egui_ctx),
            reveal_token: false,
            status: String::new(),
            last_refresh: Instant::now() - REFRESH_EVERY,
            hidden: false,
            quitting: false,
            ignore_close_until: Instant::now(),
            exit_at: exit_after.map(|d| Instant::now() + d),
        };
        app.rescan(&cc.egui_ctx, false);
        app
    }

    /// Scans for installed apps on a worker thread.
    fn rescan(&self, ctx: &egui::Context, force: bool) {
        {
            let mut view = self.catalog.lock().unwrap();
            if view.loading {
                return;
            }
            view.loading = true;
        }
        let view = Arc::clone(&self.catalog);
        let catalog = Arc::clone(&self.services.catalog);
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let entries = if force {
                catalog.refresh()
            } else {
                catalog.list()
            };
            let mut view = view.lock().unwrap();
            view.entries = entries;
            view.loading = false;
            ctx.request_repaint();
        });
    }

    /// Saves `next` if it is valid and the on-disk config is not damaged.
    fn commit(&mut self, next: Config) {
        if let Some(problem) = &self.problem {
            self.status = format!(
                "Not saved: the config file is damaged ({problem}). Fix or remove it first."
            );
            return;
        }
        let problems = validate(&next);
        if !problems.is_empty() {
            self.status = format!("Not saved: {}", problems.join("; "));
            return;
        }
        match save(&self.services.config_path, &next) {
            Ok(()) => {
                if let Some(tray) = &self.tray {
                    tray.set_http_enabled(next.http.enabled);
                }
                self.config = next;
                self.status = "Saved.".to_string();
            }
            Err(e) => self.status = format!("Not saved: {e}"),
        }
    }

    /// Picks up edits made outside the window (the file, another process).
    fn reload_from_disk(&mut self) {
        match load(&self.services.config_path) {
            Ok(config) if validate(&config).is_empty() => {
                self.problem = None;
                // The generated token of a file that has none is not an edit.
                if config != self.config && !config.http.token.is_empty() {
                    if let Some(tray) = &self.tray {
                        tray.set_http_enabled(config.http.enabled);
                    }
                    self.port_text = self
                        .services
                        .http
                        .effective_port(config.http.port)
                        .to_string();
                    self.config = config;
                }
            }
            Ok(config) => self.problem = Some(validate(&config).join("; ")),
            Err(e) => self.problem = Some(e.to_string()),
        }
    }

    fn show_window(&mut self, ctx: &egui::Context) {
        self.hidden = false;
        self.ignore_close_until = Instant::now() + STALE_CLOSE_WINDOW;
        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
        ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
        ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
    }

    fn quit(&mut self, ctx: &egui::Context) {
        self.quitting = true;
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    }

    fn toggle_http(&mut self) {
        let mut next = self.config.clone();
        next.http.enabled = !next.http.enabled;
        self.commit(next);
    }
}

impl eframe::App for GuiApp {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        while let Ok(msg) = self.inbox.try_recv() {
            match msg {
                Msg::Show | Msg::Tray(TrayAction::Open) => self.show_window(ctx),
                Msg::Tray(TrayAction::ToggleHttp) => self.toggle_http(),
                Msg::Tray(TrayAction::Quit) => self.quit(ctx),
            }
        }
        if self.exit_at.is_some_and(|at| Instant::now() >= at) {
            self.quit(ctx);
        }
        // Closing the window hides it when there is a tray to bring it back.
        let close_requested = ctx.input(|i| i.viewport().close_requested());
        if close_requested
            && !self.quitting
            && !self.hidden
            && Instant::now() >= self.ignore_close_until
            && self.tray.is_some()
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
            self.hidden = true;
            self.ignore_close_until = Instant::now() + STALE_CLOSE_WINDOW;
        }
        ctx.request_repaint_after(REFRESH_EVERY);
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.activity.poll();
        if self.last_refresh.elapsed() >= REFRESH_EVERY {
            self.last_refresh = Instant::now();
            self.reload_from_disk();
            if self.tab == Tab::Activity {
                self.activity.refresh(
                    ui.ctx(),
                    self.services
                        .config_path
                        .parent()
                        .unwrap_or(std::path::Path::new("."))
                        .to_path_buf(),
                );
            }
        }

        self.draw_shell(ui);
    }
}
