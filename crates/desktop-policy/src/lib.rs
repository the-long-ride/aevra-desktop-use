//! Policy for aevra-desktop: the allowlist config, the gate that applies it,
//! hard refusals for sensitive fields, and the value-free activity log.

pub mod activity;
pub mod config;
pub mod gate;
pub mod paths;
pub mod sensitive;

pub use config::{load, save, validate, AllowedApp, AppIdentity, Config, ConfigError, HttpConfig};
