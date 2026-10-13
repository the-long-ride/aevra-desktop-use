//! The `desktop_*` MCP tools, ported from aevra's Worker dispatch, behind the
//! allowlist gate.

pub mod apps;
pub mod handlers;
pub mod http;
pub mod leases;
pub mod rmcp_server;
pub mod session;
pub mod tools;

pub use handlers::{Output, Platform, Server};
pub use http::{serve_http, HttpError};
pub use rmcp_server::{serve_stdio, Connection};
