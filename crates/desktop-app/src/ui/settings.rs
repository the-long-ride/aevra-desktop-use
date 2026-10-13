//! The Settings tab's logic: the HTTP fields, the token, and the copy-ready
//! client snippets.

use desktop_policy::config::generate_token;
use desktop_policy::Config;
use serde_json::{json, Value};
use std::path::Path;

pub const SERVER_KEY: &str = "aevra-desktop";

/// Parses the port field: 1 to 65535.
pub fn parse_port(text: &str) -> Result<u16, String> {
    let port: u32 = text
        .trim()
        .parse()
        .map_err(|_| "enter a port number".to_string())?;
    u16::try_from(port)
        .ok()
        .filter(|p| *p != 0)
        .ok_or_else(|| "the port must be between 1 and 65535".to_string())
}

/// The token for display: hidden unless `reveal`.
pub fn token_display(token: &str, reveal: bool) -> String {
    if reveal {
        token.to_string()
    } else {
        "•".repeat(token.chars().count().min(24))
    }
}

/// Replaces the HTTP token with a new random one. Returns the new token.
pub fn regenerate_token(config: &mut Config) -> String {
    let token = generate_token();
    config.http.token = token.clone();
    token
}

/// An MCP client config that launches this program over stdio.
pub fn stdio_snippet(exe: &Path) -> String {
    pretty(json!({
        "mcpServers": {
            SERVER_KEY: {
                "command": exe.to_string_lossy(),
                "args": ["mcp"],
            }
        }
    }))
}

/// An MCP client config for the loopback HTTP endpoint. The token is
/// included, so the snippet is only for copying into a client, not for display.
pub fn http_snippet(port: u16, token: &str) -> String {
    pretty(json!({
        "mcpServers": {
            SERVER_KEY: {
                "type": "http",
                "url": format!("http://127.0.0.1:{port}/mcp"),
                "headers": { "Authorization": format!("Bearer {token}") },
            }
        }
    }))
}

fn pretty(value: Value) -> String {
    serde_json::to_string_pretty(&value).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ports_must_be_in_range() {
        assert_eq!(parse_port(" 47851 "), Ok(47851));
        assert_eq!(parse_port("1"), Ok(1));
        assert_eq!(parse_port("65535"), Ok(65535));
        assert!(parse_port("0").is_err());
        assert!(parse_port("65536").is_err());
        assert!(parse_port("-1").is_err());
        assert!(parse_port("http").is_err());
        assert!(parse_port("").is_err());
    }

    #[test]
    fn the_token_is_masked_unless_revealed() {
        assert_eq!(token_display("abc", true), "abc");
        let masked = token_display("abcdefghijklmnopqrstuvwxyz0123456789", false);
        assert!(!masked.contains('a'));
        assert_eq!(masked.chars().count(), 24);
        assert_eq!(token_display("", false), "");
    }

    #[test]
    fn regenerating_changes_the_token_and_nothing_else() {
        let mut config = Config::default();
        config.http.token = "old-placeholder".into();
        config.http.port = 4321;
        let new = regenerate_token(&mut config);
        assert_ne!(new, "old-placeholder");
        assert_eq!(config.http.token, new);
        assert_eq!(new.len(), 43);
        assert_eq!(config.http.port, 4321);
        assert_ne!(regenerate_token(&mut config), new);
    }

    #[test]
    fn the_stdio_snippet_launches_the_mcp_command() {
        let snippet = stdio_snippet(Path::new("/opt/aevra/aevra-desktop"));
        let v: Value = serde_json::from_str(&snippet).unwrap();
        assert_eq!(
            v["mcpServers"][SERVER_KEY]["command"],
            "/opt/aevra/aevra-desktop"
        );
        assert_eq!(v["mcpServers"][SERVER_KEY]["args"], json!(["mcp"]));
    }

    #[test]
    fn the_http_snippet_points_at_loopback_with_a_bearer_header() {
        let snippet = http_snippet(47851, "placeholder-token");
        let v: Value = serde_json::from_str(&snippet).unwrap();
        assert_eq!(
            v["mcpServers"][SERVER_KEY]["url"],
            "http://127.0.0.1:47851/mcp"
        );
        assert_eq!(
            v["mcpServers"][SERVER_KEY]["headers"]["Authorization"],
            "Bearer placeholder-token"
        );
    }
}
