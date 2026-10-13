//! The rmcp binding: one [`Connection`] per MCP client, all sharing one
//! [`Server`]. The connection's uuid is the lease owner, so one client can
//! never use or release another client's leases.

use crate::handlers::Server;
use crate::tools;
use rmcp::model::{
    CallToolRequestParams, CallToolResponse, Implementation, ListToolsResult,
    PaginatedRequestParams, ProtocolVersion, ServerCapabilities, ServerConfig,
};
use rmcp::service::RequestContext;
use rmcp::{ErrorData, RoleServer, ServerHandler, ServiceExt};
use serde_json::Value;
use std::borrow::Cow;
use std::sync::Arc;

pub const SERVER_NAME: &str = "aevra-desktop";

const INSTRUCTIONS: &str = "Controls desktop apps the user has allowed in aevra-desktop. \
Call desktop_connect first. Anything not on the allowlist is refused; tell the user to open \
aevra-desktop and add the app.";

pub struct Connection {
    server: Arc<Server>,
    owner: String,
}

impl Connection {
    pub fn new(server: Arc<Server>) -> Self {
        Self {
            server,
            owner: uuid::Uuid::new_v4().to_string(),
        }
    }
}

impl Drop for Connection {
    /// A closed connection ends its leases and hands their snapshots back.
    fn drop(&mut self) {
        let server = Arc::clone(&self.server);
        let owner = std::mem::take(&mut self.owner);
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            runtime.spawn(async move { server.release_owner(&owner).await });
        }
    }
}

impl ServerHandler for Connection {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new(SERVER_NAME, env!("CARGO_PKG_VERSION")))
            .with_instructions(INSTRUCTIONS)
    }

    /// Sessions carry the connection identity that leases are owned by, so
    /// the sessionless 2026-07-28 revision is not offered.
    fn supported_protocol_versions(&self) -> Cow<'static, [ProtocolVersion]> {
        Cow::Borrowed(ProtocolVersion::known_up_to(&ProtocolVersion::V_2025_11_25))
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        Ok(ListToolsResult::with_all_items(tools::tool_list()))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        let args = request.arguments.map(Value::Object).unwrap_or(Value::Null);
        let result = self.server.call(&self.owner, &request.name, args).await;
        Ok(CallToolResponse::Complete(result))
    }
}

/// Serves MCP over this process's stdin and stdout until the client closes
/// them. Nothing else may write to stdout while this runs.
pub async fn serve_stdio(server: Arc<Server>) -> Result<(), String> {
    let running = Connection::new(server)
        .serve(rmcp::transport::io::stdio())
        .await
        .map_err(|e| e.to_string())?;
    running.waiting().await.map_err(|e| e.to_string())?;
    Ok(())
}
