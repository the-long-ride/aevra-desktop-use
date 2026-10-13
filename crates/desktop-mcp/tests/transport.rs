mod common;

use common::*;
use desktop_mcp::{serve_http, HttpError};
use desktop_policy::HttpConfig;
use rmcp::model::{CallToolRequestParams, ClientCapabilities, ClientConfig, Implementation};
use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;
use rmcp::transport::StreamableHttpClientTransport;
use rmcp::ServiceExt;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

const TOKEN: &str = "test-token-0123456789-test-token-0123456789x";

fn client_config() -> ClientConfig {
    ClientConfig::new(
        ClientCapabilities::default(),
        Implementation::new("transport-test", "0.0.0"),
    )
}

fn shared_server() -> (Arc<desktop_mcp::Server>, Harness) {
    let mut h = harness(
        vec![window("1", NOTEPAD, 100, vec![node("button", "Save")])],
        vec![allow("app_notepad", "Notepad", NOTEPAD)],
    );
    // Move the server out; the harness keeps the state and temp dir alive.
    let server = std::mem::replace(&mut h.server, harness(vec![], vec![]).server);
    (Arc::new(server), h)
}

async fn start_http(server: Arc<desktop_mcp::Server>) -> (SocketAddr, CancellationToken) {
    let shutdown = CancellationToken::new();
    let addr = serve_http(
        server,
        HttpConfig {
            enabled: true,
            port: 0,
            token: TOKEN.to_string(),
        },
        shutdown.clone(),
    )
    .await
    .expect("http server starts");
    (addr, shutdown)
}

#[tokio::test]
async fn stdio_style_transport_lists_all_seventeen_tools() {
    let (server, _h) = shared_server();
    let (client_io, server_io) = tokio::io::duplex(1 << 16);
    let connection = desktop_mcp::Connection::new(server);
    let server_task = tokio::spawn(async move {
        let running = connection.serve(server_io).await.expect("server handshake");
        let _ = running.waiting().await;
    });

    let client = client_config()
        .serve(client_io)
        .await
        .expect("client handshake");
    let info = client.peer_info().expect("server info");
    assert_eq!(
        info.server_info.as_ref().expect("server identity").name,
        "aevra-desktop"
    );
    assert!(info.capabilities.tools.is_some());

    let tools = client.list_all_tools().await.unwrap();
    assert_eq!(tools.len(), 17);
    assert!(tools.iter().any(|t| t.name == "desktop_act_many"));
    assert!(!tools.iter().any(|t| t.name == "desktop_request_access"));

    let status = client
        .call_tool(CallToolRequestParams::new("desktop_status"))
        .await
        .unwrap();
    assert_ne!(status.is_error, Some(true));
    client.cancel().await.ok();
    let _ = server_task.await;
}

#[tokio::test]
async fn each_connection_owns_its_own_leases() {
    let (server, h) = shared_server();
    let mut clients = Vec::new();
    for _ in 0..2 {
        let (client_io, server_io) = tokio::io::duplex(1 << 16);
        let connection = desktop_mcp::Connection::new(Arc::clone(&server));
        tokio::spawn(async move {
            if let Ok(running) = connection.serve(server_io).await {
                let _ = running.waiting().await;
            }
        });
        clients.push(client_config().serve(client_io).await.unwrap());
    }
    let call = |c: &rmcp::service::RunningService<rmcp::RoleClient, ClientConfig>,
                name: &'static str,
                args: serde_json::Value| {
        let params = CallToolRequestParams::new(name)
            .with_arguments(args.as_object().cloned().unwrap_or_default());
        let peer = c.peer().clone();
        async move { peer.call_tool(params).await.unwrap() }
    };
    call(&clients[0], "desktop_connect", serde_json::json!({})).await;
    let first = call(
        &clients[0],
        "desktop_describe",
        serde_json::json!({"windowId": "1", "mode": "background"}),
    )
    .await;
    assert_ne!(first.is_error, Some(true));
    let second = call(
        &clients[1],
        "desktop_describe",
        serde_json::json!({"windowId": "1", "mode": "background"}),
    )
    .await;
    assert_eq!(second.is_error, Some(true));
    assert!(second
        .content
        .iter()
        .filter_map(|c| c.as_text())
        .any(|t| t.text.contains("DESKTOP_WINDOW_BUSY")));

    // Closing the first connection hands its lease and snapshot back.
    clients.remove(0).cancel().await.ok();
    for _ in 0..50 {
        if !h.state.lock().unwrap().released_snapshots.is_empty() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    assert_eq!(h.state.lock().unwrap().released_snapshots.len(), 1);
    assert!(!server.session.leases.is_window_leased("1"));
}

fn url(addr: SocketAddr, path: &str) -> String {
    format!("http://{addr}{path}")
}

fn initialize_body() -> serde_json::Value {
    serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "protocolVersion": "2025-06-18",
            "capabilities": {},
            "clientInfo": {"name": "t", "version": "0"}
        }
    })
}

fn mcp_post(client: &reqwest::Client, addr: SocketAddr) -> reqwest::RequestBuilder {
    client
        .post(url(addr, "/mcp"))
        .header("Accept", "application/json, text/event-stream")
        .json(&initialize_body())
}

#[tokio::test]
async fn http_rejects_missing_wrong_and_unsafe_requests() {
    let (server, _h) = shared_server();
    let (addr, shutdown) = start_http(server).await;
    let client = reqwest::Client::new();

    let none = mcp_post(&client, addr).send().await.unwrap();
    assert_eq!(none.status(), 401);

    // Same length as the real token, one character different.
    let mut wrong = TOKEN.to_string();
    wrong.pop();
    wrong.push('y');
    assert_eq!(wrong.len(), TOKEN.len());
    let wrong = mcp_post(&client, addr)
        .bearer_auth(wrong)
        .send()
        .await
        .unwrap();
    assert_eq!(wrong.status(), 401);

    let not_bearer = mcp_post(&client, addr)
        .header("Authorization", TOKEN)
        .send()
        .await
        .unwrap();
    assert_eq!(not_bearer.status(), 401);

    let evil_origin = mcp_post(&client, addr)
        .bearer_auth(TOKEN)
        .header("Origin", "https://evil.test")
        .send()
        .await
        .unwrap();
    assert_eq!(evil_origin.status(), 403);

    let lookalike_origin = mcp_post(&client, addr)
        .bearer_auth(TOKEN)
        .header("Origin", "http://127.0.0.1.evil.test")
        .send()
        .await
        .unwrap();
    assert_eq!(lookalike_origin.status(), 403);

    let evil_host = mcp_post(&client, addr)
        .bearer_auth(TOKEN)
        .header("Host", "evil.test")
        .send()
        .await
        .unwrap();
    assert_eq!(evil_host.status(), 403);

    let wrong_port = mcp_post(&client, addr)
        .bearer_auth(TOKEN)
        .header("Host", format!("127.0.0.1:{}", addr.port().wrapping_add(1)))
        .send()
        .await
        .unwrap();
    assert_eq!(wrong_port.status(), 403);

    // Loopback origins and no origin at all are fine.
    let ok = mcp_post(&client, addr)
        .bearer_auth(TOKEN)
        .header("Origin", format!("http://localhost:{}", addr.port()))
        .send()
        .await
        .unwrap();
    assert!(ok.status().is_success(), "{}", ok.status());
    shutdown.cancel();
}

#[tokio::test]
async fn http_healthz_needs_no_token_but_still_checks_the_host() {
    let (server, _h) = shared_server();
    let (addr, shutdown) = start_http(server).await;
    let client = reqwest::Client::new();
    let ok = client.get(url(addr, "/healthz")).send().await.unwrap();
    assert_eq!(ok.status(), 200);
    assert_eq!(
        ok.json::<serde_json::Value>().await.unwrap(),
        serde_json::json!({"ok": true})
    );
    let evil = client
        .get(url(addr, "/healthz"))
        .header("Host", "evil.test")
        .send()
        .await
        .unwrap();
    assert_eq!(evil.status(), 403);
    shutdown.cancel();
}

#[tokio::test]
async fn http_with_a_valid_token_lists_all_seventeen_tools() {
    let (server, _h) = shared_server();
    let (addr, shutdown) = start_http(server).await;
    let transport = StreamableHttpClientTransport::from_config(
        StreamableHttpClientTransportConfig::with_uri(url(addr, "/mcp")).auth_header(TOKEN),
    );
    let client = client_config()
        .serve(transport)
        .await
        .expect("handshake over http");
    let tools = client.list_all_tools().await.unwrap();
    assert_eq!(tools.len(), 17);
    client.cancel().await.ok();
    shutdown.cancel();
}

#[tokio::test]
async fn http_refuses_to_start_without_a_token() {
    let (server, _h) = shared_server();
    let err = serve_http(
        server,
        HttpConfig {
            enabled: true,
            port: 0,
            token: String::new(),
        },
        CancellationToken::new(),
    )
    .await
    .unwrap_err();
    assert!(matches!(err, HttpError::MissingToken));
}

#[tokio::test]
async fn a_port_that_is_taken_reports_http_port_in_use() {
    let held = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = held.local_addr().unwrap().port();
    let (server, _h) = shared_server();
    let err = serve_http(
        server,
        HttpConfig {
            enabled: true,
            port,
            token: TOKEN.to_string(),
        },
        CancellationToken::new(),
    )
    .await
    .unwrap_err();
    assert!(matches!(err, HttpError::PortInUse(p) if p == port));
    assert_eq!(err.to_string(), format!("HTTP_PORT_IN_USE: {port}"));
}

#[tokio::test]
async fn the_http_listener_binds_loopback_only() {
    let (server, _h) = shared_server();
    let (addr, shutdown) = start_http(server).await;
    assert!(addr.ip().is_loopback());
    assert_eq!(addr.ip().to_string(), "127.0.0.1");
    shutdown.cancel();
}
