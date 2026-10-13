//! `aevra-desktop mcp`: MCP over stdio for a client that launches this program.
//!
//! Stdout carries the protocol and nothing else; every diagnostic goes to
//! stderr. The allowlist is the same file the window edits and is reloaded
//! whenever it changes.

use crate::scan::{AppCatalog, Catalog};
use crate::watch::watch_config;
use desktop_core::actor;
use desktop_mcp::{serve_stdio, Server};
use desktop_policy::activity::ActivityLog;
use desktop_policy::config::{load, validate};
use desktop_policy::paths::{config_file, native_config_dir};
use desktop_policy::Config;
use std::path::Path;
use std::sync::Arc;

/// The config to start with: the file if it is usable, otherwise an empty
/// allowlist (which refuses everything) and a note on stderr.
fn initial_config(path: &Path) -> Config {
    match load(path) {
        Ok(config) => {
            let problems = validate(&config);
            if problems.is_empty() {
                return config;
            }
            eprintln!(
                "aevra-desktop: ignoring the config, nothing is allowed: {}",
                problems.join("; ")
            );
        }
        Err(e) => eprintln!("aevra-desktop: ignoring the config, nothing is allowed: {e}"),
    }
    Config::default()
}

pub fn run() -> Result<(), String> {
    let dir = native_config_dir();
    let path = config_file(&dir);
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .map_err(|e| e.to_string())?;

    let server = Arc::new(Server::new(
        actor::native(),
        initial_config(&path),
        ActivityLog::new(&dir),
        Arc::new(Catalog::native()) as Arc<dyn AppCatalog>,
    ));

    let _watcher = {
        let server = Arc::clone(&server);
        let handle = runtime.handle().clone();
        watch_config(&path, move |config| {
            handle.block_on(server.apply_config(config))
        })
        .map_err(|e| format!("could not watch the config: {e}"))?
    };

    runtime.block_on(serve_stdio(server))
}
