//! The bus as a minimal MCP server over loopback HTTP.
//!
//! Codex 0.144 speaks the MCP streamable-HTTP transport: it POSTs JSON-RPC to one endpoint and,
//! for a server that never pushes its own notifications, accepts a plain `application/json`
//! response per request. Every bus tool is pure request/response, so that is all I implement here:
//! `initialize`, `tools/list`, `tools/call`, and a 202 for the one notification codex sends. No
//! SSE, no session state. That is far less surface than wiring a full MCP SDK for a handful of
//! tools, and every byte on the wire is under test.
//!
//! Two kinds of tool live here, and they are gated differently on purpose. The peer tools are about
//! another agent's live terminal, so an edge on the canvas has to authorize them. The memory tools
//! are about the project itself, so they are open to every node in the workspace: an agent wired to
//! nobody still inherits what was learned, which is the only reason memory is worth having.
//!
//! Identity is one header and it is a secret, not a name. Identra mints a token per node, sets it
//! as `IDENTRA_BUS_TOKEN` on that node's process, and the CLI expands it into a header itself. The
//! bus maps the token back to the node id, so who you are is something you prove, not something you
//! claim.
//!
//! I do not take the caller's node id from a header or a tool argument, and this is the whole point.
//! An agent has a shell: if the id were self-asserted, any node could curl this port claiming to be
//! a peer, read that peer's context, and send messages under its name. A per-node secret cannot be
//! forged that way, because a node only ever holds its own.
//!
//! An env-sourced header is also the only mechanism every one of these CLIs shares, so one config
//! serves every node while the value still differs per node. The edge on the canvas is the second
//! gate: even a proven caller only reads or messages nodes it is wired to.

use std::collections::HashMap;
use std::io;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use axum::{
    extract::{ConnectInfo, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::post,
    Json, Router,
};
use serde_json::{json, Value};

use identra_core::canvas;
use identra_core::TerminalManager;
use identra_memory as memory;

use crate::{config, get_peer_context, list_peers, send_to_node, tasks, BusError};

/// Memories live beside the canvas, inside the workspace, because they are about that project. A
/// workspace you delete takes its memory with it, which is the behaviour I want: no orphaned facts
/// about a project that is gone.
pub fn memory_path(project_dir: &std::path::Path) -> PathBuf {
    project_dir.join(".identra").join("memory.db")
}

/// Every agent in a workspace writes into one shared pool, so the dedup key is the project and not
/// the agent. Two agents learning the same thing is one memory, and a fresh agent reads what every
/// earlier one learned. That sharing is the whole point of having memory at all.
const MEMORY_AGENT: &str = "workspace";

/// How many memories a recall returns unless the caller asks for fewer. Enough to be useful in a
/// prompt, small enough not to bury the agent's actual task.
const RECALL_LIMIT: usize = 10;

/// Open the workspace's memory, creating `.identra/` on the first write.
///
/// I open per call rather than holding a connection on the bus: `Connection` is not `Sync`, these
/// are low frequency calls, and opening SQLite is cheap. A cached connection behind a mutex would
/// be a lock to reason about for no gain I can measure.
fn open_memory(project_dir: &std::path::Path) -> Result<memory::Store, String> {
    let path = memory_path(project_dir);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    memory::Store::open(path).map_err(|e| e.to_string())
}

/// Shared bus state. Holds the same `TerminalManager` the Tauri commands hold, so a peer's live
/// transcript and stdin are the exact ones on the canvas, and reads the canvas fresh per call so a
/// wire pulled mid-session takes effect immediately.
///
/// `project_dir` is shared with the Tauri layer rather than copied, because switching workspace has
/// to move the bus too: the tools must read the canvas the user is actually looking at.
pub struct Bus {
    manager: Arc<TerminalManager>,
    project_dir: Arc<Mutex<PathBuf>>,
    /// Secret to node id. The map is the identity: holding a token is the only way to be that node.
    tokens: Mutex<HashMap<String, String>>,
}

impl Bus {
    pub fn new(manager: Arc<TerminalManager>, project_dir: Arc<Mutex<PathBuf>>) -> Self {
        Self {
            manager,
            project_dir,
            tokens: Mutex::new(HashMap::new()),
        }
    }

    /// Mint this node's own secret and remember which node it names. Call it right before the
    /// node's CLI launches and put the result in that process's env, nowhere else: it is the one
    /// thing separating "I am node b" from "I say I am node b".
    ///
    /// A relaunched node gets a fresh token and the old one stays mapped. That is harmless, the old
    /// token names the same node, and it saves tracking which of a node's launches is current.
    pub fn issue_token(&self, node_id: &str) -> String {
        let token = random_token();
        self.tokens
            .lock()
            .unwrap()
            .insert(token.clone(), node_id.to_string());
        token
    }

    fn node_for(&self, token: &str) -> Option<String> {
        self.tokens.lock().unwrap().get(token).cloned()
    }
}

/// Bind the loopback listener now so the caller gets the port before the server task runs: it needs
/// the port to write the agent's MCP config, and the config has to exist before any agent launches.
pub fn bind() -> io::Result<(std::net::TcpListener, u16)> {
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0))?;
    let port = listener.local_addr()?.port();
    Ok((listener, port))
}

/// Serve the bus on an already-bound listener until the process ends.
pub async fn serve(listener: std::net::TcpListener, bus: Arc<Bus>) -> io::Result<()> {
    listener.set_nonblocking(true)?;
    let listener = tokio::net::TcpListener::from_std(listener)?;
    axum::serve(
        listener,
        router(bus).into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await
}

fn router(bus: Arc<Bus>) -> Router {
    // GET on the same path is the SSE channel a client may open; a request/response server is
    // allowed to refuse it, and codex does not need it.
    Router::new()
        .route("/mcp", post(handle).get(reject_get))
        .with_state(bus)
}

async fn reject_get() -> Response {
    StatusCode::METHOD_NOT_ALLOWED.into_response()
}

async fn handle(
    State(bus): State<Arc<Bus>>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    body: Json<Value>,
) -> Response {
    // Binding to 127.0.0.1 already keeps this loopback-only. I re-check so a later change to the
    // bind address cannot silently expose the bus, and inject peers into a running agent's stdin,
    // to the network.
    if !peer.ip().is_loopback() {
        return StatusCode::FORBIDDEN.into_response();
    }
    // The token is the identity: it names the caller, so there is nothing here for a node to lie
    // about. An unknown token is not a node I launched, so it gets nothing.
    let Some(caller) = header(&headers, config::TOKEN_HEADER).and_then(|t| bus.node_for(&t)) else {
        return StatusCode::UNAUTHORIZED.into_response();
    };

    let req = body.0;
    let method = req.get("method").and_then(Value::as_str).unwrap_or("");
    // A message with no id is a notification (the `initialized` handshake). Nothing to answer.
    let Some(id) = req.get("id").cloned() else {
        return StatusCode::ACCEPTED.into_response();
    };
    match dispatch(&bus, &caller, method, req.get("params")) {
        Ok(result) => Json(json!({"jsonrpc": "2.0", "id": id, "result": result})).into_response(),
        Err((code, message)) => {
            Json(json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}}))
                .into_response()
        }
    }
}

/// A protocol-level reply. Tool failures are not errors here: they come back as a normal
/// `tools/call` result with `isError: true`, which is what an agent expects to read and react to.
fn dispatch(
    bus: &Bus,
    caller: &str,
    method: &str,
    params: Option<&Value>,
) -> Result<Value, (i64, String)> {
    match method {
        "initialize" => Ok(json!({
            // Echo the client's protocol version so I agree with whatever codex negotiates.
            "protocolVersion": params
                .and_then(|p| p.get("protocolVersion"))
                .and_then(Value::as_str)
                .unwrap_or("2025-06-18"),
            "capabilities": {"tools": {}},
            "serverInfo": {"name": "identra-bus", "version": env!("CARGO_PKG_VERSION")},
        })),
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({"tools": tool_specs()})),
        "tools/call" => Ok(call_tool(bus, caller, params)),
        other => Err((-32601, format!("method not found: {other}"))),
    }
}

fn tool_specs() -> Value {
    json!([
        {
            "name": "list_peers",
            "description": "List the node ids you are wired to on the canvas. Only wired peers can be read or messaged.",
            "inputSchema": {"type": "object", "properties": {}, "additionalProperties": false}
        },
        {
            "name": "get_peer_context",
            "description": "Read the recent terminal transcript of a wired peer node so you can see what it just did.",
            "inputSchema": {
                "type": "object",
                "properties": {"nodeId": {"type": "string"}},
                "required": ["nodeId"]
            }
        },
        {
            "name": "send_to_node",
            "description": "Send a line of text into a wired peer node's terminal. It arrives prefixed with your node name.",
            "inputSchema": {
                "type": "object",
                "properties": {"nodeId": {"type": "string"}, "text": {"type": "string"}},
                "required": ["nodeId", "text"]
            }
        },
        {
            "name": "add_memory",
            "description": "Remember a durable fact about this project so any agent here, now or later, can recall it. Good memories are decisions, constraints, conventions, and approaches that were tried and rejected. Write one self-contained fact per call, with no pronouns, so it still makes sense to an agent that was not here. Do not store secrets.",
            "inputSchema": {
                "type": "object",
                "properties": {"text": {"type": "string"}},
                "required": ["text"]
            }
        },
        {
            "name": "search_memory",
            "description": "Recall what has already been learned about this project, by you or by any other agent in earlier sessions. Search before asking the user something they may have already answered, and before redoing work someone already rejected.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": {"type": "string"},
                    "limit": {"type": "integer"}
                },
                "required": ["query"]
            }
        },
        {
            "name": "add_task",
            "description": "Put a piece of work on the shared board so any agent here can take it. Use `after` to name tasks that must finish first, so nobody starts something that is not ready. Describe one piece of work per task, and name the files it owns.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "description": {"type": "string"},
                    "after": {"type": "array", "items": {"type": "integer"}}
                },
                "required": ["description"]
            }
        },
        {
            "name": "list_tasks",
            "description": "See the shared board: what is open, who is on what, what is blocked, and what is finished.",
            "inputSchema": {"type": "object", "properties": {}, "additionalProperties": false}
        },
        {
            "name": "claim_task",
            "description": "Take a task before you start it, so no other agent does the same work. Omit the id to take the oldest task that is ready. Claiming is atomic: if you get it, it is yours.",
            "inputSchema": {
                "type": "object",
                "properties": {"id": {"type": "integer"}}
            }
        },
        {
            "name": "complete_task",
            "description": "Mark your task finished, with a short note on what you did. This is what unblocks the tasks waiting on it, so do it as soon as the work is done rather than at the end.",
            "inputSchema": {
                "type": "object",
                "properties": {"id": {"type": "integer"}, "note": {"type": "string"}},
                "required": ["id"]
            }
        }
    ])
}

fn call_tool(bus: &Bus, caller: &str, params: Option<&Value>) -> Value {
    let name = params
        .and_then(|p| p.get("name"))
        .and_then(Value::as_str)
        .unwrap_or("");
    let args = params
        .and_then(|p| p.get("arguments"))
        .cloned()
        .unwrap_or_else(|| json!({}));
    let arg_str = |key: &str| {
        args.get(key)
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string()
    };

    // Fresh every call: the canvas is the current topology, and a peer's title travels with it.
    let dir = bus.project_dir.lock().unwrap().clone();
    let canvas = canvas::load(&dir);
    let edges = &canvas.edges;

    match name {
        "list_peers" => {
            let lines: Vec<String> = list_peers(caller, edges)
                .iter()
                .map(|id| match canvas.nodes.iter().find(|n| &n.id == id) {
                    Some(n) if !n.title.is_empty() => format!("{id}\t{}", n.title),
                    _ => id.clone(),
                })
                .collect();
            ok_text(&if lines.is_empty() {
                "no wired peers".to_string()
            } else {
                lines.join("\n")
            })
        }
        "get_peer_context" => {
            match get_peer_context(caller, &arg_str("nodeId"), edges, &*bus.manager) {
                Ok(ctx) => ok_text(&ctx),
                Err(e) => err_text(&bus_err(e)),
            }
        }
        "send_to_node" => {
            let title = canvas
                .nodes
                .iter()
                .find(|n| n.id == caller)
                .map(|n| n.title.clone())
                .unwrap_or_default();
            match send_to_node(
                caller,
                &title,
                &arg_str("nodeId"),
                &arg_str("text"),
                edges,
                &*bus.manager,
            ) {
                Ok(()) => ok_text("delivered"),
                Err(e) => err_text(&bus_err(e)),
            }
        }
        // Memory is not edge gated. An edge says who may read your terminal, which is a live,
        // private thing; memory is what the project knows, and every node in the workspace shares
        // it. Gating recall behind a wire would defeat the point: the whole value is that an agent
        // dropped in later, wired to nobody, still starts from what was learned.
        "add_memory" => match remember(&dir, caller, &arg_str("text")) {
            Ok(stored) => ok_text(&stored),
            Err(e) => err_text(&e),
        },
        "search_memory" => {
            let limit = args
                .get("limit")
                .and_then(Value::as_u64)
                .map(|n| n as usize)
                .filter(|n| *n > 0)
                .unwrap_or(RECALL_LIMIT);
            match recall(&dir, &arg_str("query"), limit) {
                Ok(found) => ok_text(&found),
                Err(e) => err_text(&e),
            }
        }
        // The board, like memory, is workspace wide rather than edge gated. An edge is about
        // reading a peer's private terminal; work everyone can pick up is the opposite of private.
        "add_task" => {
            let after: Vec<i64> = args
                .get("after")
                .and_then(Value::as_array)
                .map(|a| a.iter().filter_map(Value::as_i64).collect())
                .unwrap_or_default();
            match board(&dir).and_then(|b| b.add(&arg_str("description"), &after, now())) {
                Ok(id) => ok_text(&format!("added t{id}")),
                Err(e) => err_text(&e),
            }
        }
        "list_tasks" => match board(&dir).and_then(|b| b.list()) {
            Ok(list) if list.is_empty() => ok_text("the board is empty"),
            Ok(list) => ok_text(
                &list
                    .iter()
                    .map(tasks::render)
                    .collect::<Vec<_>>()
                    .join("\n"),
            ),
            Err(e) => err_text(&e),
        },
        "claim_task" => {
            // A claim held by a node that is gone is not a lock, so the live set decides what can
            // be taken back. The manager is the only thing that knows which nodes still run.
            let live = bus.manager.ids();
            let wanted = args.get("id").and_then(Value::as_i64);
            match board(&dir).and_then(|b| b.claim(caller, wanted, &live)) {
                Ok(t) => ok_text(&format!("claimed t{}: {}", t.id, t.description)),
                Err(e) => err_text(&e),
            }
        }
        "complete_task" => {
            let Some(id) = args.get("id").and_then(Value::as_i64) else {
                return err_text("complete_task needs the id of the task you finished");
            };
            let note = args.get("note").and_then(Value::as_str);
            match board(&dir).and_then(|b| b.complete(id, note, now())) {
                Ok(unblocked) if unblocked.is_empty() => ok_text(&format!("done t{id}")),
                Ok(unblocked) => ok_text(&format!(
                    "done t{id}. now unblocked: {}",
                    unblocked
                        .iter()
                        .map(|t| format!("t{} ({})", t.id, t.description))
                        .collect::<Vec<_>>()
                        .join(", ")
                )),
                Err(e) => err_text(&e),
            }
        }
        other => err_text(&format!("unknown tool: {other}")),
    }
}

fn board(project_dir: &std::path::Path) -> Result<tasks::Board, String> {
    tasks::Board::open(project_dir)
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// The workspace folder name is the project's identity for memory. I use it rather than the full
/// path so a workspace that gets moved keeps what it learned.
fn memory_scope(project_dir: &std::path::Path, caller: &str) -> memory::Scope {
    memory::Scope {
        user_id: project_dir
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "workspace".into()),
        agent_id: MEMORY_AGENT.into(),
        // The run is the node that learned it. It buys provenance in the history log without
        // splitting the shared pool, since the dedup key does not include it.
        run_id: caller.into(),
    }
}

fn remember(project_dir: &std::path::Path, caller: &str, text: &str) -> Result<String, String> {
    let text = text.trim();
    if text.is_empty() {
        return Err("nothing to remember: text was empty".into());
    }
    let store = open_memory(project_dir)?;
    let stored = store
        .add(&memory_scope(project_dir, caller), text)
        .map_err(|e| e.to_string())?;
    // An empty result means the fact was already held. That is a success, not a failure, but the
    // agent should hear it so it does not keep trying to store the same thing.
    Ok(if stored.is_empty() {
        "already remembered, nothing new stored".into()
    } else {
        format!("remembered: {text}")
    })
}

fn recall(project_dir: &std::path::Path, query: &str, limit: usize) -> Result<String, String> {
    let store = open_memory(project_dir)?;
    let scope = memory_scope(project_dir, "");
    // Filter on the project only. Any agent's memories, from any session, are in scope: recall that
    // stopped at your own sessions would just be a worse transcript.
    let filter = memory::Filter {
        user_id: Some(scope.user_id),
        ..Default::default()
    };
    let hits = store
        .search(&filter, query, limit)
        .map_err(|e| e.to_string())?;
    if hits.is_empty() {
        return Ok("nothing remembered about that yet".into());
    }
    Ok(hits
        .iter()
        .map(|m| format!("- {}", m.content))
        .collect::<Vec<_>>()
        .join("\n"))
}

fn ok_text(text: &str) -> Value {
    json!({"content": [{"type": "text", "text": text}], "isError": false})
}

fn err_text(text: &str) -> Value {
    json!({"content": [{"type": "text", "text": text}], "isError": true})
}

fn bus_err(e: BusError) -> String {
    match e {
        BusError::NoEdge => "no edge to that node: draw a wire to it first".into(),
        BusError::NoPeer => "that peer is wired but not running yet".into(),
        BusError::Input(s) => format!("could not deliver: {s}"),
    }
}

fn header(headers: &HeaderMap, name: &str) -> Option<String> {
    headers.get(name)?.to_str().ok().map(str::to_string)
}

/// 16 bytes of `/dev/urandom`, hex encoded. That source is always present on the Linux and macOS
/// targets Identra ships to; the fallback only keeps a failed read from minting an all-zero token,
/// it is not a strong secret.
fn random_token() -> String {
    use std::io::Read;
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);

    let mut buf = [0u8; 16];
    let read_ok = std::fs::File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut buf))
        .is_ok();
    if !read_ok {
        let n = COUNTER.fetch_add(1, Ordering::Relaxed) ^ u64::from(std::process::id());
        buf[..8].copy_from_slice(&n.to_le_bytes());
    }
    buf.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use identra_core::canvas::{Canvas, Edge, Node};

    fn bus_in(dir: PathBuf) -> Bus {
        let manager = Arc::new(TerminalManager::new(Arc::new(|_id, _out| {})));
        Bus::new(manager, Arc::new(Mutex::new(dir)))
    }

    #[test]
    fn a_token_names_exactly_one_node_and_nothing_else_does() {
        let bus = bus_in(std::env::temp_dir());
        let a = bus.issue_token("node-a");
        let b = bus.issue_token("node-b");

        assert_eq!(bus.node_for(&a).as_deref(), Some("node-a"));
        assert_eq!(bus.node_for(&b).as_deref(), Some("node-b"));
        // Node a holds only its own secret, so it has nothing it could present to become node b.
        // That is the whole impersonation defence: identity is proven, never asserted.
        assert_ne!(a, b);
        assert_eq!(bus.node_for("not-a-token"), None);
        assert_eq!(a.len(), 32, "16 random bytes, hex encoded");

        let mut headers = HeaderMap::new();
        headers.insert(config::TOKEN_HEADER, a.parse().unwrap());
        assert_eq!(
            header(&headers, config::TOKEN_HEADER).as_deref(),
            Some(a.as_str())
        );
        assert_eq!(header(&HeaderMap::new(), config::TOKEN_HEADER), None);
    }

    #[test]
    fn initialize_echoes_version_and_lists_the_tools() {
        let bus = bus_in(std::env::temp_dir());
        let init = dispatch(
            &bus,
            "node-a",
            "initialize",
            Some(&json!({"protocolVersion": "2025-03-26"})),
        )
        .unwrap();
        assert_eq!(init["protocolVersion"], "2025-03-26");
        assert_eq!(init["serverInfo"]["name"], "identra-bus");

        let list = dispatch(&bus, "node-a", "tools/list", None).unwrap();
        let names: Vec<&str> = list["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap())
            .collect();
        assert_eq!(
            names,
            [
                "list_peers",
                "get_peer_context",
                "send_to_node",
                "add_memory",
                "search_memory",
                "add_task",
                "list_tasks",
                "claim_task",
                "complete_task"
            ]
        );

        assert!(dispatch(&bus, "node-a", "nonsense", None).is_err());
    }

    #[test]
    fn memory_carries_across_agents_and_sessions() {
        let dir = std::env::temp_dir().join(format!("identra-mem-tool-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let bus = bus_in(dir.clone());

        let add = |caller: &str, text: &str| {
            call_tool(
                &bus,
                caller,
                Some(&json!({"name": "add_memory", "arguments": {"text": text}})),
            )
        };
        let find = |caller: &str, query: &str| {
            call_tool(
                &bus,
                caller,
                Some(&json!({"name": "search_memory", "arguments": {"query": query}})),
            )
        };

        let out = add(
            "node-a",
            "we dropped redis and use postgres listen/notify instead",
        );
        assert_eq!(out["isError"], false);

        // The payoff: a different node, in a later session, wired to nobody, still knows. This is
        // the whole reason memory is not edge gated.
        let hit = find("node-b", "redis");
        assert_eq!(hit["isError"], false);
        assert!(
            hit["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains("postgres listen/notify"),
            "a later agent should recall what an earlier one learned: {hit}"
        );

        // Re learning a known fact is a no op, so a chatty agent cannot fill the pool with copies.
        let again = add(
            "node-b",
            "we dropped redis and use postgres listen/notify instead",
        );
        assert!(again["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("already remembered"));

        // A miss says so plainly rather than returning an empty list the agent has to interpret.
        let miss = find("node-a", "kubernetes");
        assert!(miss["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("nothing remembered"));

        // Empty text is a caller mistake, and silently storing it would poison recall.
        assert_eq!(add("node-a", "   ")["isError"], true);

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn two_agents_split_a_board_without_doing_the_same_work() {
        let dir = std::env::temp_dir().join(format!("identra-board-tool-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let bus = bus_in(dir.clone());

        let call = |caller: &str, args: Value| call_tool(&bus, caller, Some(&args));
        let text = |v: &Value| v["content"][0]["text"].as_str().unwrap().to_string();

        // One agent plans the work: the test cannot start until the route exists.
        let r = call(
            "a",
            json!({"name": "add_task", "arguments": {"description": "write GET /health in src/health.rs"}}),
        );
        assert_eq!(text(&r), "added t1");
        let r = call(
            "a",
            json!({"name": "add_task", "arguments": {"description": "test it in tests/health.rs", "after": [1]}}),
        );
        assert_eq!(text(&r), "added t2");

        // The other agent sees the same board. That is the point of it being shared.
        let board = text(&call("b", json!({"name": "list_tasks"})));
        assert!(board.contains("- t1 [open, claimable] write GET /health"));
        assert!(board.contains("- t2 [blocked by t1]"));

        // a takes the route. b reaching for the test is refused with the reason, so it waits instead
        // of building against a route that does not exist yet.
        //
        // Whether b could take t1 off a depends on the live node set, and no PTY runs in this test,
        // so every claim here looks abandoned. The tasks tests cover that rule with an explicit live
        // set; I assert only what the live set cannot change.
        assert!(text(&call(
            "a",
            json!({"name": "claim_task", "arguments": {"id": 1}})
        ))
        .contains("claimed t1"));
        let b_tried = call("b", json!({"name": "claim_task", "arguments": {"id": 2}}));
        assert_eq!(b_tried["isError"], true);
        assert!(
            text(&b_tried).contains("blocked until t1"),
            "got: {}",
            text(&b_tried)
        );

        // Finishing the route tells a that b now has something to do.
        let done = text(&call(
            "a",
            json!({"name": "complete_task", "arguments": {"id": 1, "note": "returns 200 ok"}}),
        ));
        assert!(done.contains("done t1"));
        assert!(done.contains("now unblocked: t2"), "got: {done}");

        // And b can now take it.
        assert!(text(&call(
            "b",
            json!({"name": "claim_task", "arguments": {"id": 2}})
        ))
        .contains("claimed t2"));
        assert!(
            text(&call("b", json!({"name": "list_tasks"}))).contains("- t1 [done: returns 200 ok]")
        );

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn list_peers_reads_the_edge_and_title_from_the_canvas() {
        let dir = std::env::temp_dir().join(format!("identra-bus-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let node = |id: &str, title: &str| Node {
            id: id.into(),
            kind: "codex".into(),
            x: 0.0,
            y: 0.0,
            width: 480.0,
            height: 320.0,
            title: title.into(),
            cwd: None,
        };
        let canvas = Canvas {
            nodes: vec![node("a", "Route"), node("b", "Tests")],
            edges: vec![Edge {
                id: "e1".into(),
                source: "a".into(),
                target: "b".into(),
            }],
            viewport: Default::default(),
            title: "test".into(),
        };
        canvas::save(&dir, &canvas).unwrap();

        let bus = bus_in(dir.clone());
        let out = call_tool(&bus, "a", Some(&json!({"name": "list_peers"})));
        assert_eq!(out["isError"], false);
        let text = out["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("b\tTests"), "peer id and title: got {text}");

        // send to an unwired node is refused by the edge gate, surfaced as a tool error.
        let refused = call_tool(
            &bus,
            "a",
            Some(&json!({"name": "send_to_node", "arguments": {"nodeId": "zzz", "text": "hi"}})),
        );
        assert_eq!(refused["isError"], true);

        std::fs::remove_dir_all(&dir).unwrap();
    }
}
