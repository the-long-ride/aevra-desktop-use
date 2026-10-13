//! The installed-app catalog the server consults for `desktop_apps`. The
//! platform scanners in `desktop-app` implement it.

use desktop_policy::AppIdentity;
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppEntry {
    pub name: String,
    pub identity: AppIdentity,
    pub icon_path: Option<PathBuf>,
    pub running: bool,
}

pub trait AppCatalog: Send + Sync {
    fn list(&self) -> Vec<AppEntry>;
}

/// A catalog that knows no apps. `desktop_apps` then reports every allowed app
/// as not running.
pub struct EmptyCatalog;

impl AppCatalog for EmptyCatalog {
    fn list(&self) -> Vec<AppEntry> {
        Vec::new()
    }
}
