//! `gang mcp` — a Model Context Protocol server over stdio.
//!
//! This is the "safest substrate for AI-generated tooling" surface: it lets an
//! AI agent discover and reason about a Ganglion fleet through a small,
//! curated toolset, while the substrate's guarantees — signed components,
//! declared capabilities, default-deny policy, and an append-only audit log —
//! mean the agent provably cannot exceed what those mechanisms permit. Every
//! action an agent can drive still flows through the same policy engine and
//! audit trail as the CLI; there is no side door.
//!
//! The first cut exposes **read-only fleet-discovery** tools (status, peers,
//! capabilities, the `gang doctor` egress check, bandwidth profiles) — safe by
//! construction and useful for an agent orienting itself. Mutating tools
//! (deploy/run) are deliberately not exposed here yet: they require live
//! connections and are the tracked next step, and when added they will be
//! policy-checked and audited exactly like `gang deploy` / `gang run`.
//!
//! Transport is line-delimited JSON-RPC 2.0 over stdio (the MCP stdio
//! transport). The message layer — request parsing, result/error envelopes,
//! and the tool catalog — is pure and unit-tested; only [`serve`] touches
//! stdio.

use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

use crate::OutputFormat;

/// MCP protocol version this server implements.
const PROTOCOL_VERSION: &str = "2024-11-05";

/// A parsed JSON-RPC request.
#[derive(Debug, Clone, PartialEq)]
pub struct Request {
    /// Request id (absent for notifications).
    pub id: Option<Value>,
    /// Method name.
    pub method: String,
    /// Params object (may be null).
    pub params: Value,
}

/// Parse a single JSON-RPC message line into a [`Request`]. Returns `None` for
/// malformed input or a message with no `method`.
pub fn parse_message(line: &str) -> Option<Request> {
    let v: Value = serde_json::from_str(line.trim()).ok()?;
    let method = v.get("method")?.as_str()?.to_string();
    Some(Request {
        id: v.get("id").cloned(),
        method,
        params: v.get("params").cloned().unwrap_or(Value::Null),
    })
}

/// Build a JSON-RPC success envelope.
pub fn success(id: &Value, result: Value) -> String {
    json!({"jsonrpc": "2.0", "id": id, "result": result}).to_string()
}

/// Build a JSON-RPC error envelope.
pub fn error(id: &Value, code: i64, message: &str) -> String {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}}).to_string()
}

/// The `initialize` result.
pub fn initialize_result() -> Value {
    json!({
        "protocolVersion": PROTOCOL_VERSION,
        "capabilities": { "tools": {} },
        "serverInfo": { "name": "ganglion", "version": env!("CARGO_PKG_VERSION") },
    })
}

/// A tool exposed to the client.
struct Tool {
    name: &'static str,
    description: &'static str,
}

/// The curated tool catalog (read-only, first cut).
fn catalog() -> Vec<Tool> {
    vec![
        Tool {
            name: "gang_status",
            description: "Summarize this Ganglion install: version, operator identity, \
                          registered peer count, and registered capability count.",
        },
        Tool {
            name: "list_peers",
            description: "List registered robot/relay peers (name, peer id, role, relays).",
        },
        Tool {
            name: "list_capabilities",
            description: "List capabilities in the local registry with their declared \
                          capability groups and authors.",
        },
        Tool {
            name: "network_doctor",
            description: "Run the outbound-reachability check (gang doctor): which egress \
                          paths work and whether a relay is reachable. Optional 'relay' arg \
                          (multiaddr) overrides the configured default.",
        },
        Tool {
            name: "list_bandwidth_profiles",
            description: "List bandwidth profiles for degraded-link streaming.",
        },
    ]
}

/// The `tools/list` result.
pub fn tools_list_result() -> Value {
    let tools: Vec<Value> = catalog()
        .iter()
        .map(|t| {
            json!({
                "name": t.name,
                "description": t.description,
                "inputSchema": tool_input_schema(t.name),
            })
        })
        .collect();
    json!({ "tools": tools })
}

/// Input JSON Schema for a tool. Only `network_doctor` takes an argument.
fn tool_input_schema(name: &str) -> Value {
    match name {
        "network_doctor" => json!({
            "type": "object",
            "properties": {
                "relay": { "type": "string", "description": "Relay multiaddr to test." }
            },
        }),
        _ => json!({ "type": "object", "properties": {} }),
    }
}

/// Wrap tool output text into a `tools/call` result.
fn tool_text_result(text: String) -> Value {
    json!({ "content": [ { "type": "text", "text": text } ] })
}

// --- Tool execution (gathers data as strings; never writes to stdout) --------

/// Execute a tool by name, returning its text output. Errors are returned as
/// `Err(message)` and surfaced to the client as an error result.
async fn call_tool(name: &str, args: &Value) -> Result<String, String> {
    match name {
        "gang_status" => Ok(gather_status_json()),
        "list_peers" => Ok(gather_peers_json()),
        "list_capabilities" => Ok(gather_capabilities_json()),
        "list_bandwidth_profiles" => {
            let profiles = gang_core::bandwidth::BandwidthProfile::builtins();
            serde_json::to_string_pretty(&profiles).map_err(|e| e.to_string())
        }
        "network_doctor" => {
            let relay = args.get("relay").and_then(|r| r.as_str()).map(String::from);
            let report = crate::doctor::gather_report(relay).await;
            serde_json::to_string_pretty(&report).map_err(|e| e.to_string())
        }
        other => Err(format!("unknown tool: {other}")),
    }
}

fn gather_status_json() -> String {
    let version = env!("CARGO_PKG_VERSION");
    let key_path = gang_core::identity::default_key_path();
    let identity = if key_path.exists() {
        match gang_core::identity::Keypair::load(&key_path) {
            Ok(kp) => kp.peer_id().to_string(),
            Err(_) => "present but unreadable".to_string(),
        }
    } else {
        "not generated".to_string()
    };
    let peers =
        gang_core::identity::PeerRegistry::load(&gang_core::identity::default_registry_path())
            .map(|r| r.list().count())
            .unwrap_or(0);
    let caps = gang_core::registry::Registry::open(&crate::commands::registry_dir())
        .map(|r| r.list().len())
        .unwrap_or(0);
    json!({
        "version": version,
        "identity": identity,
        "registered_peers": peers,
        "registered_capabilities": caps,
    })
    .to_string()
}

fn gather_peers_json() -> String {
    let registry =
        gang_core::identity::PeerRegistry::load(&gang_core::identity::default_registry_path())
            .unwrap_or_default();
    let peers: Vec<Value> = registry
        .list()
        .map(|(name, e)| {
            json!({
                "name": name,
                "peer_id": e.peer_id.to_string(),
                "role": format!("{:?}", e.role),
                "relays": e.relay_addrs,
            })
        })
        .collect();
    json!({ "peers": peers }).to_string()
}

fn gather_capabilities_json() -> String {
    match gang_core::registry::Registry::open(&crate::commands::registry_dir()) {
        Ok(reg) => {
            let caps: Vec<Value> = reg
                .list()
                .iter()
                .map(|r| {
                    let declared = reg
                        .get_latest(&r.name)
                        .map(|e| e.declared_capabilities.clone())
                        .unwrap_or_default();
                    json!({
                        "name": r.name,
                        "version": r.latest_version,
                        "author": r.author,
                        "declared_capabilities": declared,
                    })
                })
                .collect();
            json!({ "capabilities": caps }).to_string()
        }
        Err(_) => json!({ "capabilities": [] }).to_string(),
    }
}

// --- stdio serve loop ---------------------------------------------------------

/// Run the MCP server over stdio until stdin closes.
pub async fn serve(format: &OutputFormat) -> anyhow::Result<()> {
    if !matches!(format, OutputFormat::Text) {
        // stdout is the JSON-RPC channel; --json would collide with it.
        anyhow::bail!("`gang mcp` speaks JSON-RPC on stdout; do not combine it with --json.");
    }

    let stdin = tokio::io::stdin();
    let mut lines = BufReader::new(stdin).lines();
    let mut stdout = tokio::io::stdout();

    while let Some(line) = lines.next_line().await? {
        if line.trim().is_empty() {
            continue;
        }
        let Some(req) = parse_message(&line) else {
            continue; // ignore unparseable input
        };

        // Notifications (no id) get no response.
        let Some(id) = req.id.clone() else {
            continue;
        };

        let response = match req.method.as_str() {
            "initialize" => success(&id, initialize_result()),
            "tools/list" => success(&id, tools_list_result()),
            "tools/call" => {
                let name = req
                    .params
                    .get("name")
                    .and_then(|n| n.as_str())
                    .unwrap_or_default()
                    .to_string();
                let args = req.params.get("arguments").cloned().unwrap_or(Value::Null);
                match call_tool(&name, &args).await {
                    Ok(text) => success(&id, tool_text_result(text)),
                    Err(msg) => success(
                        &id,
                        json!({
                            "content": [ { "type": "text", "text": msg } ],
                            "isError": true
                        }),
                    ),
                }
            }
            "ping" => success(&id, json!({})),
            other => error(&id, -32601, &format!("method not found: {other}")),
        };

        stdout.write_all(response.as_bytes()).await?;
        stdout.write_all(b"\n").await?;
        stdout.flush().await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_request_and_notification() {
        let req = parse_message(r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#).unwrap();
        assert_eq!(req.method, "tools/list");
        assert_eq!(req.id, Some(json!(1)));

        let note =
            parse_message(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#).unwrap();
        assert_eq!(note.id, None);

        assert!(parse_message("not json").is_none());
        assert!(parse_message(r#"{"jsonrpc":"2.0","id":1}"#).is_none()); // no method
    }

    #[test]
    fn initialize_advertises_tools_capability() {
        let r = initialize_result();
        assert_eq!(r["protocolVersion"], PROTOCOL_VERSION);
        assert!(r["capabilities"]["tools"].is_object());
        assert_eq!(r["serverInfo"]["name"], "ganglion");
    }

    #[test]
    fn tools_list_has_schemas_and_known_names() {
        let r = tools_list_result();
        let tools = r["tools"].as_array().unwrap();
        assert_eq!(tools.len(), 5);
        for t in tools {
            assert!(t["name"].is_string());
            assert!(t["description"].is_string());
            assert_eq!(t["inputSchema"]["type"], "object");
        }
        let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
        assert!(names.contains(&"network_doctor"));
        assert!(names.contains(&"list_peers"));
    }

    #[test]
    fn doctor_tool_declares_relay_arg() {
        let schema = tool_input_schema("network_doctor");
        assert!(schema["properties"]["relay"].is_object());
        // Other tools take no args.
        assert_eq!(tool_input_schema("list_peers")["properties"], json!({}));
    }

    #[test]
    fn envelopes_are_wellformed() {
        let ok: Value = serde_json::from_str(&success(&json!(7), json!({"x":1}))).unwrap();
        assert_eq!(ok["jsonrpc"], "2.0");
        assert_eq!(ok["id"], 7);
        assert_eq!(ok["result"]["x"], 1);

        let err: Value = serde_json::from_str(&error(&json!(8), -32601, "nope")).unwrap();
        assert_eq!(err["error"]["code"], -32601);
        assert_eq!(err["error"]["message"], "nope");
    }

    #[tokio::test]
    async fn unknown_tool_is_an_error() {
        assert!(call_tool("does_not_exist", &Value::Null).await.is_err());
    }

    #[tokio::test]
    async fn bandwidth_profiles_tool_returns_json() {
        let out = call_tool("list_bandwidth_profiles", &Value::Null)
            .await
            .unwrap();
        assert!(out.contains("lidar-low"));
    }
}
