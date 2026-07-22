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
use std::time::Duration;

use tokio::sync::oneshot;

use axum::{
    extract::{ConnectInfo, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::post,
    Json, Router,
};
use serde_json::{json, Value};

use identra_core::canvas;
use identra_core::worktree;
use identra_core::TerminalManager;
use identra_memory as memory;

use crate::{config, get_peer_context, inbox, list_peers, tasks, BusError};

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

/// Browsing hands back more than a search does, because it is the whole picture rather than the
/// answer to one question. Fifty facts is a couple of pages for the agent to read and still far more
/// than a personal project accumulates in a week.
const BROWSE_LIMIT: usize = 50;

/// The embedding model, loaded at most once for the life of the process.
///
/// Opening a store is cheap and I do it per call, but loading a model is not: it is a second of
/// CPU and, the very first time, a download. So the store is per call and the model is per process,
/// which is the only reason `Store` takes an `Arc` here rather than owning its embedder.
///
/// A failure is a downgrade, not an error. No model means recall matches on words, which is worse
/// but works, and it is what someone offline gets. I cache the failure too: if the model is not
/// coming, retrying it on every single memory call would freeze the agent's turn over and over for
/// the same answer.
#[cfg(feature = "fastembed")]
fn shared_embedder() -> Option<std::sync::Arc<dyn memory::Embedder>> {
    use std::sync::OnceLock;
    static MODEL: OnceLock<Option<std::sync::Arc<memory::LocalEmbedder>>> = OnceLock::new();
    MODEL
        .get_or_init(|| {
            // The one thing in Identra that reaches the network, so it gets two ways to say no.
            // The env is for tests and scripts: a suite that pulls 130MB from a model host fails
            // for reasons that have nothing to do with the code, and a workspace build turns this
            // feature on for every crate whether it wanted it or not. The settings file is the
            // user's own switch, written by the settings panel. Read here, at the same gate,
            // because the OnceLock means the answer is fixed for the life of the process anyway
            // and this is the one place it is asked.
            if std::env::var("IDENTRA_EMBEDDINGS").is_ok_and(|v| v == "off") {
                return None;
            }
            if !identra_core::settings::load().embeddings {
                return None;
            }
            match memory::LocalEmbedder::new() {
                Ok(model) => Some(std::sync::Arc::new(model)),
                Err(e) => {
                    eprintln!("identra: recall is matching on words, not meaning: {e}");
                    None
                }
            }
        })
        .clone()
        .map(|m| m as std::sync::Arc<dyn memory::Embedder>)
}

#[cfg(not(feature = "fastembed"))]
fn shared_embedder() -> Option<std::sync::Arc<dyn memory::Embedder>> {
    None
}

/// Open the workspace's memory, creating `.identra/` on the first write.
///
/// I open per call rather than holding a connection on the bus: `Connection` is not `Sync`, these
/// are low frequency calls, and opening SQLite is cheap. A cached connection behind a mutex would
/// be a lock to reason about for no gain I can measure.
///
/// Public because the desktop reads this same memory for the human's panel, and it has to read it
/// the way the agents do: same embedder, so the human searching "auth" and an agent searching it
/// get the same answer. The embedder is a process wide `OnceLock`, so the app and its bus share the
/// one model rather than loading it twice.
pub fn open_memory(project_dir: &std::path::Path) -> Result<memory::Store, String> {
    let path = memory_path(project_dir);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let store = memory::Store::open(path).map_err(|e| e.to_string())?;
    match shared_embedder() {
        Some(model) => store.with_embedder(model).map_err(|e| e.to_string()),
        None => Ok(store),
    }
}

/// A canvas mutation an agent asked for, on its way to the window that owns the canvas.
#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CanvasCommand {
    pub request_id: String,
    pub action: String,
    pub params: Value,
}

/// How a command reaches the canvas. The bus cannot apply one itself: the canvas lives in the
/// window and is its single writer, so this is a seam the Tauri layer fills with a window emit, and
/// a test fills with a closure.
pub type Emit = Arc<dyn Fn(CanvasCommand) + Send + Sync>;

/// How long an agent waits for the canvas before being told to ask its human. Long enough for a
/// busy renderer, short enough that a closed window does not hang the agent's turn.
const CANVAS_TIMEOUT: Duration = Duration::from_secs(20);

/// How long `wait_for_nodes` waits when the caller does not say, and the most it will ever wait.
/// The cap exists because a wait is a held tool call: an agent that asks to wait an hour has
/// stopped being useful and should be told to check back instead.
const DEFAULT_WAIT_SECS: u64 = 120;
const MAX_WAIT_SECS: u64 = 600;
/// Slow enough not to spin, fast enough that a short task does not feel stalled.
const WAIT_POLL: Duration = Duration::from_millis(750);

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
    emit: Emit,
    /// Canvas commands still waiting on the window. The request id correlates the reply back to the
    /// agent that is blocked on it.
    pending: Mutex<HashMap<String, oneshot::Sender<Value>>>,
}

impl Bus {
    pub fn new(
        manager: Arc<TerminalManager>,
        project_dir: Arc<Mutex<PathBuf>>,
        emit: Emit,
    ) -> Self {
        Self {
            manager,
            project_dir,
            tokens: Mutex::new(HashMap::new()),
            emit,
            pending: Mutex::new(HashMap::new()),
        }
    }

    /// The canvas answering a command it was asked to apply. Unknown ids are dropped: a reply to a
    /// request that already timed out is late, not useful.
    pub fn resolve_canvas(&self, request_id: &str, result: Value) {
        if let Some(tx) = self.pending.lock().unwrap().remove(request_id) {
            let _ = tx.send(result);
        }
    }

    /// Ask the canvas to change, and wait for it to say what happened.
    ///
    /// I go through the window rather than writing `canvas.json` myself because the canvas is the
    /// live state: the user is dragging these nodes right now, and a second writer would race the
    /// debounced save and lose one of them. The window applies the change and saves, as it already
    /// does for a human's edit, so an agent's edit and a human's edit take the same path.
    async fn canvas_command(&self, action: &str, params: Value) -> Result<Value, String> {
        let request_id = random_token();
        let (tx, rx) = oneshot::channel();
        self.pending.lock().unwrap().insert(request_id.clone(), tx);
        (self.emit)(CanvasCommand {
            request_id: request_id.clone(),
            action: action.into(),
            params,
        });

        match tokio::time::timeout(CANVAS_TIMEOUT, rx).await {
            Ok(Ok(result)) => Ok(result),
            // The window went away between the emit and the reply.
            Ok(Err(_)) => Err("the canvas closed before it answered".into()),
            Err(_) => {
                // Drop the slot so a late reply does not sit in the map forever.
                self.pending.lock().unwrap().remove(&request_id);
                Err(format!(
                    "the canvas did not answer {action} in time. Its window may be closed or on another workspace: ask your user to bring Identra to the front"
                ))
            }
        }
    }

    /// Mint this node's own secret and remember which node it names. Call it right before the
    /// node's CLI launches and put the result in that process's env, nowhere else: it is the one
    /// thing separating "I am node b" from "I say I am node b".
    ///
    /// Issuing retires whatever this node held before, so a node has exactly one live token at a
    /// time. The launch path kills the old child before starting the new one, so there is never a
    /// process still holding the retired secret, and one-token-per-node means the map cannot grow
    /// without bound across a long session of restarts.
    pub fn issue_token(&self, node_id: &str) -> String {
        let token = random_token();
        let mut tokens = self.tokens.lock().unwrap();
        tokens.retain(|_, owner| owner != node_id);
        tokens.insert(token.clone(), node_id.to_string());
        token
    }

    /// Drop every token naming `node_id`, so nothing can speak as it any more.
    ///
    /// This is what closing a node has to do. Without it a killed agent's secret stays valid for the
    /// life of the app: the edge-gated tools would find no edges and refuse, but the tools that need
    /// no wire (memory, the board, listing the canvas) would still answer a node that no longer
    /// exists. A credential that outlives the thing it names is worth removing even when the blast
    /// radius is small.
    pub fn revoke_node(&self, node_id: &str) {
        self.tokens
            .lock()
            .unwrap()
            .retain(|_, owner| owner != node_id);
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
    match dispatch(&bus, &caller, method, req.get("params")).await {
        Ok(result) => Json(json!({"jsonrpc": "2.0", "id": id, "result": result})).into_response(),
        Err((code, message)) => {
            Json(json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}}))
                .into_response()
        }
    }
}

/// A protocol-level reply. Tool failures are not errors here: they come back as a normal
/// `tools/call` result with `isError: true`, which is what an agent expects to read and react to.
async fn dispatch(
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
        "tools/call" => Ok(call_tool(bus, caller, params).await),
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
            "description": "Send a message to a wired peer. It is queued and waits until they read it, so it is not lost if they are busy, and they are nudged that it arrived. Say what you did, what you changed, and what you need back. If you have nothing to say, say nothing: silence is how a run between agents ends.",
            "inputSchema": {
                "type": "object",
                "properties": {"nodeId": {"type": "string"}, "text": {"type": "string"}},
                "required": ["nodeId", "text"]
            }
        },
        {
            "name": "check_inbox",
            "description": "Read the messages your peers have sent you. Each one is delivered once, so read them when you are nudged that they arrived, and act on them before you carry on.",
            "inputSchema": {"type": "object", "properties": {}, "additionalProperties": false}
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
            "name": "list_memory",
            "description": "Everything this project has learned, newest first, from every agent that worked here before you. Call this once when you start, before you ask the user anything: it is how you find out what was already decided without having to guess what to search for.",
            "inputSchema": {
                "type": "object",
                "properties": {"limit": {"type": "integer"}},
                "additionalProperties": false
            }
        },
        {
            "name": "search_memory",
            "description": "Recall what has already been learned about this project, by you or by any other agent in earlier sessions. Search before asking the user something they may have already answered, and before redoing work someone already rejected. Matching is on words, so if you are not sure how a fact was worded, use list_memory instead.",
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
        },
        {
            "name": "list_canvas",
            "description": "See every node on the canvas and how they are wired, including nodes you are not connected to. Use this to find out who is here before you bring on more agents.",
            "inputSchema": {"type": "object", "properties": {}, "additionalProperties": false}
        },
        {
            "name": "add_terminal",
            "description": "Bring another agent onto the canvas to help, running as its own node. Use this when the work splits into parts that can run at the same time. The new agent is wired to you automatically, so you can send it work as soon as it starts. Put the work on the board first: a helper with nothing to claim just idles.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "agent": {"type": "string", "description": "Agent id, for example codex or claude. Defaults to your own kind."},
                    "title": {"type": "string"},
                    "isolate": {
                        "type": "boolean",
                        "description": "Give the helper its own checkout on its own branch, so it can edit the same files as you without either of you overwriting the other. Use this whenever the work is not cleanly split by file. Its work lands back on your branch when it is merged."
                    }
                }
            }
        },
        {
            "name": "connect_nodes",
            "description": "Wire two nodes together so they can read and message each other. A node reads its tools when it starts, so wire before the other agent launches where you can.",
            "inputSchema": {
                "type": "object",
                "properties": {"from": {"type": "string"}, "to": {"type": "string"}},
                "required": ["from", "to"]
            }
        },
        {
            "name": "add_note",
            "description": "Leave a note on the canvas for your user to read. Use it for something a human should see and decide on, not for talking to another agent.",
            "inputSchema": {
                "type": "object",
                "properties": {"text": {"type": "string"}},
                "required": ["text"]
            }
        },
        {
            "name": "show_file",
            "description": "Open a file in a read-only viewer node on the canvas, wired to you, so your user can look at what you made without scrolling your terminal. Use it to hand over an artifact: a report you wrote, an image, a summary. Only files inside the workspace can be shown.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": {"type": "string", "description": "The file to show, absolute or relative to the workspace."},
                    "title": {"type": "string", "description": "What the node is called. Defaults to the file name."}
                },
                "required": ["path"]
            }
        },
        {
            "name": "get_node_status",
            "description": "Check whether a node is working, waiting, or gone. Read from its output: an agent that is thinking prints, an agent that is done goes quiet. Quiet can also mean it is stuck waiting on its human, so do not read it as success.",
            "inputSchema": {
                "type": "object",
                "properties": {"nodeId": {"type": "string"}},
                "required": ["nodeId"]
            }
        },
        {
            "name": "wait_for_nodes",
            "description": "Block until the named nodes stop working, then carry on. Use it after you hand work to a helper and genuinely cannot proceed without their result. Do not poll in a loop yourself. Going quiet is not the same as succeeding: check their work, or ask them, before you rely on it.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "nodeIds": {"type": "array", "items": {"type": "string"}},
                    "timeoutSec": {"type": "integer"}
                },
                "required": ["nodeIds"]
            }
        },
        {
            "name": "land_work",
            "description": "Merge an isolated helper's branch back onto your checkout and remove its worktree. Use this once a helper you gave its own checkout to (add_terminal with isolate) has finished and committed its work, and you have checked it. Only works on a helper you are wired to. It refuses if the helper has uncommitted changes or the merge conflicts, and leaves the checkout in place for you to sort out.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "nodeId": {"type": "string"},
                    "squash": {"type": "boolean", "description": "Land the helper's commits as one. Defaults to true."}
                },
                "required": ["nodeId"]
            }
        }
    ])
}

async fn call_tool(bus: &Bus, caller: &str, params: Option<&Value>) -> Value {
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
            let title = node_title(&canvas, caller);
            match queue_message(
                &dir,
                caller,
                &title,
                &arg_str("nodeId"),
                &arg_str("text"),
                edges,
                bus,
            ) {
                Ok(msg) => ok_text(&msg),
                Err(e) => err_text(&e),
            }
        }
        "check_inbox" => match read_inbox(&dir, caller) {
            Ok(text) => ok_text(&text),
            Err(e) => err_text(&e),
        },
        // Memory is not edge gated. An edge says who may read your terminal, which is a live,
        // private thing; memory is what the project knows, and every node in the workspace shares
        // it. Gating recall behind a wire would defeat the point: the whole value is that an agent
        // dropped in later, wired to nobody, still starts from what was learned.
        "add_memory" => match remember(&dir, caller, &arg_str("text")) {
            Ok(stored) => ok_text(&stored),
            Err(e) => err_text(&e),
        },
        "list_memory" => match known(&dir, limit_arg(&args, BROWSE_LIMIT)) {
            Ok(held) => ok_text(&held),
            Err(e) => err_text(&e),
        },
        "search_memory" => match recall(&dir, &arg_str("query"), limit_arg(&args, RECALL_LIMIT)) {
            Ok(found) => ok_text(&found),
            Err(e) => err_text(&e),
        },
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
        // The canvas tools ask the window to change something, and wait for it to say what happened.
        // The window owns the canvas: the user is dragging these nodes right now, so a second writer
        // here would race the debounced save. An agent's edit takes the same path a human's does.
        "list_canvas" => {
            // Read only, so I answer from the saved canvas rather than waking the window. Every node
            // is listed, wired or not: an agent deciding whether to bring on help needs to see who
            // is already here, which is not the same question as who it may talk to.
            let peers = list_peers(caller, edges);
            if canvas.nodes.is_empty() {
                return ok_text("the canvas is empty");
            }
            let lines: Vec<String> = canvas
                .nodes
                .iter()
                .map(|n| {
                    let who = if n.id == caller {
                        " (you)"
                    } else if peers.iter().any(|p| p == &n.id) {
                        " (wired to you)"
                    } else {
                        ""
                    };
                    let title = if n.title.is_empty() {
                        &n.kind
                    } else {
                        &n.title
                    };
                    format!("- {} [{}] {}{}", n.id, n.kind, title, who)
                })
                .collect();
            ok_text(&lines.join("\n"))
        }
        "add_terminal" => {
            // Default to my own kind: an agent asking for help without naming one usually wants
            // another of itself, and it is the one agent I know is installed and signed in.
            let my_kind = canvas
                .nodes
                .iter()
                .find(|n| n.id == caller)
                .map(|n| n.kind.clone())
                .unwrap_or_else(|| "codex".into());
            let kind = match args.get("agent").and_then(Value::as_str) {
                Some(a) if !a.is_empty() => a.to_string(),
                _ => my_kind,
            };
            // Isolation happens before the node exists, because the checkout is where it will run.
            // If it fails, say so and stop: dropping the helper into the shared tree anyway is the
            // exact collision the caller asked to avoid.
            let mut cwd = None;
            let mut branch = None;
            if args.get("isolate").and_then(Value::as_bool) == Some(true) {
                let slug = format!("{kind}-{}", &random_token()[..6]);
                let Some(base) = worktree::worktrees_root() else {
                    return err_text("cannot find anywhere to put an isolated checkout");
                };
                match worktree::isolate(&dir, &slug, &base) {
                    Ok(out) => {
                        // A worktree is a checkout of tracked files, and the guide is not one: it is
                        // written into the workspace, so a fresh checkout never has it. Without it
                        // the helper has every bus tool and no idea it has peers, which is the whole
                        // difference between two agents working together and two agents ignoring
                        // each other. Best effort, because a helper with no guide is still better
                        // than no helper.
                        if let Err(e) = config::write_guides(&out.path) {
                            eprintln!("identra: no guide in the isolated checkout: {e}");
                        }
                        cwd = Some(out.path.display().to_string());
                        branch = Some(out.branch);
                    }
                    Err(e) => return err_text(&format!("could not isolate the helper: {e}")),
                }
            }

            let params = json!({
                "kind": kind,
                "title": args.get("title").and_then(Value::as_str),
                "cwd": cwd,
                // Wiring the helper to its spawner is the whole point: an agent that cannot reach
                // the one that called it is not help, it is a stranger.
                "connectTo": caller,
            });
            match bus.canvas_command("add_terminal", params).await {
                Ok(v) => canvas_reply(&v, |id| {
                    match &branch {
                    Some(b) => format!(
                        "added {id} on its own checkout, branch {b}, and wired it to you. It can edit the same files as you without a collision. Put work on the board so it has something to claim."
                    ),
                    None => format!(
                        "added {id} and wired it to you. It shares your working directory, so split the work by file. Put work on the board so it has something to claim."
                    ),
                }
                }),
                Err(e) => err_text(&e),
            }
        }
        "connect_nodes" => {
            let params = json!({"from": arg_str("from"), "to": arg_str("to")});
            match bus.canvas_command("connect_nodes", params).await {
                Ok(v) => canvas_reply(&v, |_| "wired".to_string()),
                Err(e) => err_text(&e),
            }
        }
        "add_note" => {
            let text = arg_str("text");
            if text.trim().is_empty() {
                return err_text("a note needs some text");
            }
            match bus.canvas_command("add_note", json!({"text": text})).await {
                Ok(v) => canvas_reply(&v, |id| format!("left note {id} on the canvas")),
                Err(e) => err_text(&e),
            }
        }
        "show_file" => {
            let path = arg_str("path");
            if path.trim().is_empty() {
                return err_text("say which file to show");
            }
            // Relative resolves against the workspace, because that is where the caller works.
            let full = if std::path::Path::new(&path).is_absolute() {
                std::path::PathBuf::from(&path)
            } else {
                dir.join(&path)
            };
            // Checked here, before anything lands on the canvas, through the same reader the
            // viewer node uses: one authority for what is showable. Refusing now gives the agent
            // a reason it can act on instead of a broken node appearing on the user's board.
            if let Err(e) = identra_core::fileview::read(&dir, &full) {
                return err_text(&format!("that file cannot be shown: {e}"));
            }
            let title = match args.get("title").and_then(Value::as_str) {
                Some(t) if !t.trim().is_empty() => t.to_string(),
                _ => full
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| path.clone()),
            };
            let params = json!({
                "path": full.display().to_string(),
                "title": title,
                // Wired to whoever showed it, so the artifact reads as that agent's work.
                "connectTo": caller,
            });
            match bus.canvas_command("show_file", params).await {
                Ok(v) => canvas_reply(&v, |id| {
                    format!("opened {id} on the canvas, showing the file to your user")
                }),
                Err(e) => err_text(&e),
            }
        }
        "get_node_status" => {
            let id = arg_str("nodeId");
            ok_text(&describe_status(bus, &canvas, &id))
        }
        "land_work" => {
            // Squash by default: a helper's branch is often a scatter of small commits, and the
            // spawner wants the change, not the helper's minute by minute history. An agent that
            // wants those commits kept passes squash false.
            let squash = args.get("squash").and_then(Value::as_bool).unwrap_or(true);
            ok_text(&land_work(&canvas, caller, &arg_str("nodeId"), squash))
        }
        "wait_for_nodes" => {
            let ids: Vec<String> = args
                .get("nodeIds")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(Value::as_str)
                        .map(str::to_string)
                        .collect()
                })
                .unwrap_or_default();
            if ids.is_empty() {
                return err_text("wait_for_nodes needs at least one node id");
            }
            let budget = args
                .get("timeoutSec")
                .and_then(Value::as_u64)
                .unwrap_or(DEFAULT_WAIT_SECS)
                .clamp(5, MAX_WAIT_SECS);
            ok_text(&wait_for(bus, &canvas, &ids, Duration::from_secs(budget)).await)
        }
        other => err_text(&format!("unknown tool: {other}")),
    }
}

/// A node's state in words, with its name, because an agent asked about "node-3" and thinks about
/// "the one writing the tests".
fn describe_status(bus: &Bus, canvas: &canvas::Canvas, id: &str) -> String {
    let name = node_title(canvas, id);
    match bus.manager.status(id) {
        Some(identra_core::terminal::Status::Running) => format!("{name} is working"),
        Some(identra_core::terminal::Status::Idle) => {
            format!("{name} is quiet: it has either finished or is waiting on its human")
        }
        // Worth saying separately, because the two quiet states mean opposite things to a peer. A
        // node that finished may have left work for you; a node stuck on a question has not, and
        // will not until a human answers it, so waiting on it is waiting on a person.
        Some(identra_core::terminal::Status::NeedsInput) => {
            format!("{name} has asked its human something and is waiting for an answer")
        }
        Some(identra_core::terminal::Status::Exited) => format!("{name} has exited"),
        // On the canvas but never launched, or already killed. Both are "not working", which is what
        // the caller actually wants to know.
        None => format!("{name} is not running"),
    }
}

/// True when there is no point waiting on this node any longer: it is quiet, gone, or never ran.
///
/// A node waiting on its human counts as settled, and deliberately so. It is not working and it
/// will not start again until a person answers it, so an agent that kept waiting would be waiting
/// on someone who is not at the keyboard. Better it returns and reads why in `get_node_status`.
fn settled(bus: &Bus, id: &str) -> bool {
    !matches!(
        bus.manager.status(id),
        Some(identra_core::terminal::Status::Running)
    )
}

/// Wait until every named node stops working, or the budget runs out.
///
/// I poll rather than wait on the exit event because finishing a turn is not exiting: an agent that
/// answers and returns to its prompt is done with the work and still very much alive. Quiet is the
/// only signal a PTY gives for that, so quiet is what I watch, and the answer says so rather than
/// claiming the work succeeded.
async fn wait_for(bus: &Bus, canvas: &canvas::Canvas, ids: &[String], budget: Duration) -> String {
    let deadline = std::time::Instant::now() + budget;
    loop {
        if ids.iter().all(|id| settled(bus, id)) {
            break;
        }
        if std::time::Instant::now() >= deadline {
            let busy: Vec<String> = ids
                .iter()
                .filter(|id| !settled(bus, id))
                .map(|id| node_title(canvas, id))
                .collect();
            return format!(
                "still working when the wait ran out: {}. They may be stuck on their human, so check on them rather than waiting again.",
                busy.join(", ")
            );
        }
        tokio::time::sleep(WAIT_POLL).await;
    }
    let lines: Vec<String> = ids
        .iter()
        .map(|id| format!("- {}", describe_status(bus, canvas, id)))
        .collect();
    format!(
        "done waiting.\n{}\n\nQuiet is not the same as finished well. Read what they changed, or ask them, before you build on it.",
        lines.join("\n")
    )
}

/// Turn the canvas's `{ok, id?, error?}` answer into something the agent can act on. A refusal from
/// the canvas is a tool error, not a protocol one: the agent should read it and try something else.
fn canvas_reply(result: &Value, say: impl Fn(&str) -> String) -> Value {
    if result.get("ok").and_then(Value::as_bool) == Some(true) {
        let id = result.get("id").and_then(Value::as_str).unwrap_or("");
        ok_text(&say(id))
    } else {
        err_text(
            result
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or("the canvas refused that"),
        )
    }
}

fn board(project_dir: &std::path::Path) -> Result<tasks::Board, String> {
    tasks::Board::open(project_dir)
}

fn node_title(canvas: &canvas::Canvas, id: &str) -> String {
    canvas
        .nodes
        .iter()
        .find(|n| n.id == id)
        .map(|n| {
            if n.title.is_empty() {
                n.kind.clone()
            } else {
                n.title.clone()
            }
        })
        .unwrap_or_else(|| id.to_string())
}

/// Where a node is running. For an isolated helper this is its worktree, which is what merge and
/// drop need. `None` means the node is not on the canvas or was never given a directory.
fn node_cwd(canvas: &canvas::Canvas, id: &str) -> Option<PathBuf> {
    canvas
        .nodes
        .iter()
        .find(|n| n.id == id)
        .and_then(|n| n.cwd.as_deref())
        .map(PathBuf::from)
}

/// Land an isolated helper's branch back on the checkout it came from, then remove its worktree.
///
/// Isolation was a one way door before this: an agent could hand a helper its own checkout and the
/// helper could commit all day, and the work had nowhere to go. The engine had merge and drop the
/// whole time, tested, with no way for an agent to reach them. This is that way.
///
/// Edge gated, and that is the right gate rather than an accident of reusing one. The wire from the
/// helper to whoever spawned it is what makes this the spawner's work to land: a node you are not
/// wired to is not yours to merge into the main checkout. I drop the worktree only after the merge
/// reports success, so a merge that refuses (a dirty branch, a conflict) leaves the helper's
/// checkout exactly where it was for the agent to sort out.
fn land_work(canvas: &canvas::Canvas, caller: &str, target: &str, squash: bool) -> String {
    if !list_peers(caller, &canvas.edges).contains(&target.to_string()) {
        return "you can only land the work of a helper you are wired to".into();
    }
    let Some(path) = node_cwd(canvas, target) else {
        return format!(
            "{} is not running in a checkout of its own, so there is nothing to land",
            node_title(canvas, target)
        );
    };
    if let Err(e) = worktree::merge(&path, squash) {
        return format!("did not land {}: {e}", node_title(canvas, target));
    }
    // The merge is in. A failure to remove the worktree now is untidy, not lost work, so I report it
    // and still call the landing a success rather than making the agent think the merge did not take.
    match worktree::drop_worktree(&path) {
        Ok(()) => format!(
            "landed {} and cleaned up its checkout",
            node_title(canvas, target)
        ),
        Err(e) => format!(
            "landed {}, but its checkout is still on disk ({e}), remove it by hand",
            node_title(canvas, target)
        ),
    }
}

/// Put a message in the peer's queue, then tell them it is there.
///
/// The queue is the delivery, and the nudge is only a prompt to read it. That split is the point:
/// the nudge races the peer's typing exactly as a raw stdin write would, but it can be garbled or
/// missed without costing anything, because the message itself is waiting either way. Sending the
/// body itself down that channel is what used to make delivery a guess.
fn queue_message(
    project_dir: &std::path::Path,
    caller: &str,
    caller_title: &str,
    peer: &str,
    text: &str,
    edges: &[canvas::Edge],
    bus: &Bus,
) -> Result<String, String> {
    // The wire is still the authorization: no edge, no message. Checking it here keeps the rule in
    // one place for every peer tool.
    if !crate::list_peers(caller, edges).iter().any(|p| p == peer) {
        return Err(bus_err(BusError::NoEdge));
    }
    let queue = inbox::Inbox::open(project_dir)?;
    queue.send(peer, caller, caller_title, text, now())?;

    let waiting = queue.waiting(peer)?;
    // Best effort by design. A peer that is not running yet has nothing to nudge, and that is fine:
    // it reads its queue when it starts. Failing the send here would report a lost message that is
    // not lost.
    let line = format!(
        "\r\n[identra] {waiting} message(s) waiting from your peers. Call check_inbox to read them.\r\n"
    );
    let _ = bus.manager.input(peer, line.as_bytes());

    Ok(format!(
        "queued for {peer}. They will read it when they check their inbox; it is not lost if they are busy."
    ))
}

fn read_inbox(project_dir: &std::path::Path, caller: &str) -> Result<String, String> {
    let queue = inbox::Inbox::open(project_dir)?;
    let messages = queue.drain(caller, now())?;
    if messages.is_empty() {
        return Ok("no new messages".into());
    }
    Ok(inbox::render(&messages))
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

/// Every memory in the project, newest first. This is the one an agent should open a session with.
///
/// I added this because `recall` alone left a fresh agent stuck: to search you must already guess
/// the words the fact was written in, and a search that misses is indistinguishable from a project
/// that knows nothing. Browsing needs no guess. The store holds hundreds of facts for a personal
/// project, so handing over the recent slice is cheaper than making an agent play word games with
/// its own history.
fn known(project_dir: &std::path::Path, limit: usize) -> Result<String, String> {
    let store = open_memory(project_dir)?;
    let scope = memory_scope(project_dir, "");
    let filter = memory::Filter {
        user_id: Some(scope.user_id),
        ..Default::default()
    };
    let held = store.recent(&filter, limit).map_err(|e| e.to_string())?;
    if held.is_empty() {
        return Ok("this project has not learned anything yet".into());
    }
    Ok(bullets(&held))
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
        // Only the word matching path reaches this, and a miss there means the query shared no word
        // with any fact, which is not the same as the project knowing nothing. Saying "nothing
        // remembered" would be a lie the agent acts on, by asking the user something it was already
        // told. I hand back the recent facts instead, so a bad guess degrades to browsing rather
        // than to amnesia.
        let held = store.recent(&filter, limit).map_err(|e| e.to_string())?;
        if held.is_empty() {
            return Ok("this project has not learned anything yet".into());
        }
        return Ok(format!(
            "nothing worded like that. what this project does know, newest first, judge for \
             yourself whether any of it bears on your question:\n{}",
            bullets(&held)
        ));
    }
    // Deliberately not "here is your answer". With a model attached this ranks every fact it holds
    // and returns the top few, so it always returns something, and that something is only ever "the
    // closest I have". I measured whether the score could tell a real answer from a question about
    // a topic this project has never touched, and it cannot: the ranges overlap (the numbers are in
    // local_embedder.rs), so a cutoff would drop true answers to keep out junk. The ordering is the
    // part this model is good at. The reader is also a model, and it can see perfectly well that a
    // fact about JWTs does not answer a question about kubernetes, so I rank honestly and say what
    // these are. What I will not do is dress the nearest row up as the answer, because an agent that
    // believes that acts on it.
    Ok(format!(
        "the closest this project has to that, nearest first. judge for yourself whether it \
         answers your question:\n{}",
        bullets(&hits)
    ))
}

fn bullets(memories: &[memory::Memory]) -> String {
    memories
        .iter()
        .map(|m| format!("- {}", m.content))
        .collect::<Vec<_>>()
        .join("\n")
}

/// An agent that sends `limit: 0` means "no opinion", not "return nothing", so I fall back to the
/// default rather than hand back an empty list it would read as an empty project.
fn limit_arg(args: &Value, default: usize) -> usize {
    args.get("limit")
        .and_then(Value::as_u64)
        .map(|n| n as usize)
        .filter(|n| *n > 0)
        .unwrap_or(default)
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

    /// A bus whose canvas never answers. Fine for every tool that does not touch the canvas.
    fn bus_in(dir: PathBuf) -> Bus {
        let manager = Arc::new(TerminalManager::new(Arc::new(|_id, _out| {})));
        Bus::new(manager, Arc::new(Mutex::new(dir)), Arc::new(|_cmd| {}))
    }

    /// A bus with a stand-in for the window: it answers every canvas command with `reply`, the way
    /// a real canvas would after applying it. The emitter needs the bus it will answer, and the bus
    /// needs the emitter, so I hand the closure a weak slot and fill it once both exist.
    ///
    /// Answering inside the emit is not a cheat: a oneshot accepts its value before anyone awaits
    /// it, so this exercises the same send and receive the real path uses.
    fn bus_with_canvas(dir: PathBuf, reply: Value) -> Arc<Bus> {
        let slot: Arc<Mutex<Option<std::sync::Weak<Bus>>>> = Arc::new(Mutex::new(None));
        let seen = slot.clone();
        let emit: Emit = Arc::new(move |cmd: CanvasCommand| {
            if let Some(bus) = seen.lock().unwrap().as_ref().and_then(|w| w.upgrade()) {
                bus.resolve_canvas(&cmd.request_id, reply.clone());
            }
        });
        let manager = Arc::new(TerminalManager::new(Arc::new(|_id, _out| {})));
        let bus = Arc::new(Bus::new(manager, Arc::new(Mutex::new(dir)), emit));
        *slot.lock().unwrap() = Some(Arc::downgrade(&bus));
        bus
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

    /// A node's credential must not outlive the node. Closing one is the case that matters, and
    /// restarting one is the case that would otherwise pile up tokens for the life of the app.
    #[test]
    fn a_closed_node_cannot_still_speak_as_itself() {
        let bus = bus_in(std::env::temp_dir());
        let a = bus.issue_token("node-a");
        let b = bus.issue_token("node-b");

        bus.revoke_node("node-a");
        assert_eq!(
            bus.node_for(&a),
            None,
            "a closed node's secret stops naming it"
        );
        assert_eq!(
            bus.node_for(&b).as_deref(),
            Some("node-b"),
            "and closing one node leaves every other node alone"
        );

        // Relaunching retires the previous secret rather than leaving both live, so a node has one
        // token at a time and a long session of restarts does not grow the map.
        let first = bus.issue_token("node-c");
        let second = bus.issue_token("node-c");
        assert_ne!(first, second);
        assert_eq!(bus.node_for(&first), None);
        assert_eq!(bus.node_for(&second).as_deref(), Some("node-c"));

        // Revoking something that was never there is a no-op, which is what the delete path needs:
        // a node the user closed before it ever launched has no token to remove.
        bus.revoke_node("node-never-existed");
    }

    #[tokio::test]
    async fn initialize_echoes_version_and_lists_the_tools() {
        let bus = bus_in(std::env::temp_dir());
        let init = dispatch(
            &bus,
            "node-a",
            "initialize",
            Some(&json!({"protocolVersion": "2025-03-26"})),
        )
        .await
        .unwrap();
        assert_eq!(init["protocolVersion"], "2025-03-26");
        assert_eq!(init["serverInfo"]["name"], "identra-bus");

        let list = dispatch(&bus, "node-a", "tools/list", None).await.unwrap();
        let names: Vec<&str> = list["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap())
            .collect();
        // The whole surface an agent is handed, in the order it reads them. Four groups: who is
        // here and how to reach them, what the project knows, what work is up for grabs, and how to
        // change the canvas itself.
        assert_eq!(
            names,
            [
                "list_peers",
                "get_peer_context",
                "send_to_node",
                "check_inbox",
                "add_memory",
                "list_memory",
                "search_memory",
                "add_task",
                "list_tasks",
                "claim_task",
                "complete_task",
                "list_canvas",
                "add_terminal",
                "connect_nodes",
                "add_note",
                "show_file",
                "get_node_status",
                "wait_for_nodes",
                "land_work",
            ]
        );

        assert!(dispatch(&bus, "node-a", "nonsense", None).await.is_err());
    }

    #[tokio::test]
    async fn memory_carries_across_agents_and_sessions() {
        let dir = std::env::temp_dir().join(format!("identra-mem-tool-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let bus = bus_in(dir.clone());

        async fn add(bus: &Bus, caller: &str, text: &str) -> Value {
            call_tool(
                bus,
                caller,
                Some(&json!({"name": "add_memory", "arguments": {"text": text}})),
            )
            .await
        }
        async fn find(bus: &Bus, caller: &str, query: &str) -> Value {
            call_tool(
                bus,
                caller,
                Some(&json!({"name": "search_memory", "arguments": {"query": query}})),
            )
            .await
        }

        let out = add(
            &bus,
            "node-a",
            "we dropped redis and use postgres listen/notify instead",
        )
        .await;
        assert_eq!(out["isError"], false);

        // The payoff: a different node, in a later session, wired to nobody, still knows. This is
        // the whole reason memory is not edge gated.
        let hit = find(&bus, "node-b", "redis").await;
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
            &bus,
            "node-b",
            "we dropped redis and use postgres listen/notify instead",
        )
        .await;
        assert!(again["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("already remembered"));

        // A query about something this project never discussed still hands back what it does hold.
        // I assert the guarantee and not the wording, because the two recall paths word it
        // differently and both honour it: word matching misses and falls back to the recent facts,
        // a model ranks this fact nearest for want of anything closer. What must never happen on
        // either path is an agent being told there is nothing, then asking the user to re-decide
        // something already settled.
        let miss = find(&bus, "node-a", "kubernetes").await;
        let text = miss["content"][0]["text"].as_str().unwrap();
        assert!(
            text.contains("postgres listen/notify"),
            "a query that hits nothing still shows what is known, never amnesia: {text}"
        );

        // Empty text is a caller mistake, and silently storing it would poison recall.
        assert_eq!(add(&bus, "node-a", "   ").await["isError"], true);

        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// The moment the whole product is built around: an agent arrives long after the decision was
    /// made, describes it in its own words, and still starts from what was settled. Recall used to
    /// be word matching only, so this agent was told the project knew nothing and would have gone
    /// back to the user to re decide something already decided.
    #[tokio::test]
    async fn a_new_agent_starts_from_what_it_never_saw_decided() {
        let dir = std::env::temp_dir().join(format!("identra-onboard-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let bus = bus_in(dir.clone());

        call_tool(
            &bus,
            "node-a",
            Some(&json!({"name": "add_memory", "arguments": {"text":
                "The API issues JWT bearer tokens rather than server side sessions, because the \
                 mobile client cannot hold cookies."}})),
        )
        .await;

        // A fresh node opens its session the way the workspace guide tells it to: no query, no
        // guess about wording, just what is known.
        let held = call_tool(
            &bus,
            "node-b",
            Some(&json!({"name": "list_memory", "arguments": {}})),
        )
        .await;
        assert_eq!(held["isError"], false);
        assert!(
            held["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains("JWT bearer tokens"),
            "a fresh agent browsing memory sees the decision: {held}"
        );

        // And the words it would reach for on its own share nothing with how the fact was written.
        // It still must not be told the project knows nothing.
        let asked = call_tool(
            &bus,
            "node-b",
            Some(
                &json!({"name": "search_memory", "arguments": {"query": "how do we handle auth"}}),
            ),
        )
        .await;
        assert!(
            asked["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains("JWT bearer tokens"),
            "asking in its own words still reaches the decision: {asked}"
        );

        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// Isolation used to be a one way door: a helper could commit in its own checkout and the work
    /// had nowhere to go. This drives the whole round trip through the tool, so the door stays open.
    #[test]
    fn a_helpers_committed_work_lands_through_the_tool() {
        let base = std::env::temp_dir().join(format!("identra-land-repo-{}", std::process::id()));
        let wt_root = std::env::temp_dir().join(format!("identra-land-wt-{}", std::process::id()));
        for p in [&base, &wt_root] {
            let _ = std::fs::remove_dir_all(p);
        }
        let git = |dir: &std::path::Path, args: &[&str]| {
            std::process::Command::new("git")
                .current_dir(dir)
                .args(args)
                .output()
                .unwrap();
        };
        std::fs::create_dir_all(&base).unwrap();
        git(&base, &["init", "-q", "-b", "main"]);
        git(&base, &["config", "user.email", "t@example.com"]);
        git(&base, &["config", "user.name", "t"]);
        std::fs::write(base.join("README.md"), "start\n").unwrap();
        git(&base, &["add", "."]);
        git(&base, &["commit", "-qm", "first"]);

        // The helper gets its own checkout, does work there, and commits it on its branch.
        let out = worktree::isolate(&base, "helper-1", &wt_root).unwrap();
        std::fs::write(out.path.join("feature.rs"), "fn added() {}\n").unwrap();
        git(&out.path, &["add", "."]);
        git(&out.path, &["commit", "-qm", "the helper's work"]);

        // The canvas the tool reads: a spawner wired to the helper, the helper running in its
        // worktree. This is what isolation writes when it spawns a helper.
        let canvas = Canvas {
            nodes: vec![
                Node {
                    id: "spawner".into(),
                    kind: "codex".into(),
                    x: 0.0,
                    y: 0.0,
                    width: 480.0,
                    height: 320.0,
                    title: "spawner".into(),
                    cwd: Some(base.display().to_string()),
                    locked: false,
                },
                Node {
                    id: "helper".into(),
                    kind: "codex".into(),
                    x: 1.0,
                    y: 1.0,
                    width: 480.0,
                    height: 320.0,
                    title: "helper".into(),
                    cwd: Some(out.path.display().to_string()),
                    locked: false,
                },
            ],
            edges: vec![Edge {
                id: "e".into(),
                source: "spawner".into(),
                target: "helper".into(),
            }],
            ..Canvas::default()
        };

        // A node the spawner is not wired to cannot be landed, whatever its cwd.
        assert!(land_work(&canvas, "spawner", "stranger", true).contains("wired to"));

        let landed = land_work(&canvas, "spawner", "helper", true);
        assert!(landed.contains("landed"), "unexpected: {landed}");

        // The helper's file is on main now, and its worktree is gone.
        assert!(
            base.join("feature.rs").is_file(),
            "the helper's committed work reached the main checkout"
        );
        assert!(
            !out.path.exists(),
            "the worktree was cleaned up after landing"
        );

        for p in [&base, &wt_root] {
            let _ = std::fs::remove_dir_all(p);
        }
    }

    #[tokio::test]
    async fn waiting_on_a_node_returns_and_never_claims_success() {
        let dir = std::env::temp_dir().join(format!("identra-wait-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        canvas::save(
            &dir,
            &Canvas {
                nodes: vec![Node {
                    id: "b".into(),
                    kind: "codex".into(),
                    x: 0.0,
                    y: 0.0,
                    width: 480.0,
                    height: 320.0,
                    title: "Tests".into(),
                    cwd: None,
                    locked: false,
                }],
                ..Canvas::default()
            },
        )
        .unwrap();
        let bus = bus_in(dir.clone());
        let text = |v: &Value| v["content"][0]["text"].as_str().unwrap().to_string();

        // A node that was never launched is not working, so a wait on it returns rather than
        // hanging for the full budget. Nothing to wait for is a normal answer, not an error.
        let out = call_tool(
            &bus,
            "a",
            Some(&json!({"name": "wait_for_nodes", "arguments": {"nodeIds": ["b"], "timeoutSec": 5}})),
        )
        .await;
        assert_eq!(out["isError"], false);
        assert!(text(&out).contains("done waiting"), "got: {}", text(&out));
        assert!(
            text(&out).contains("Tests"),
            "it names the node, not the id"
        );
        // The one thing this must never imply. Quiet means quiet, and an agent that reads it as
        // "the work is good" will build on something nobody checked.
        assert!(text(&out).contains("not the same as finished well"));

        let status = call_tool(
            &bus,
            "a",
            Some(&json!({"name": "get_node_status", "arguments": {"nodeId": "b"}})),
        )
        .await;
        assert!(text(&status).contains("Tests is not running"));

        // A wait with nothing to wait on is a caller mistake worth saying out loud.
        let empty = call_tool(
            &bus,
            "a",
            Some(&json!({"name": "wait_for_nodes", "arguments": {"nodeIds": []}})),
        )
        .await;
        assert_eq!(empty["isError"], true);

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[tokio::test]
    async fn a_message_survives_a_peer_that_is_not_listening() {
        let dir = std::env::temp_dir().join(format!("identra-msg-tool-{}", std::process::id()));
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
            locked: false,
        };
        canvas::save(
            &dir,
            &Canvas {
                nodes: vec![node("a", "Route"), node("b", "Tests"), node("c", "Docs")],
                edges: vec![Edge {
                    id: "e1".into(),
                    source: "a".into(),
                    target: "b".into(),
                }],
                ..Canvas::default()
            },
        )
        .unwrap();
        let bus = bus_in(dir.clone());
        let text = |v: &Value| v["content"][0]["text"].as_str().unwrap().to_string();

        // b is not running: there is no PTY to write to at all. Under the old stdin push this
        // message was simply gone, and a said "delivered".
        let sent = call_tool(
            &bus,
            "a",
            Some(&json!({"name": "send_to_node", "arguments": {"nodeId": "b", "text": "route is on GET /health"}})),
        )
        .await;
        assert_eq!(
            sent["isError"], false,
            "queueing does not need a live peer: {sent}"
        );
        assert!(text(&sent).contains("not lost if they are busy"));

        // b reads it whenever it gets round to it, with the text exactly as a wrote it.
        let got = call_tool(&bus, "b", Some(&json!({"name": "check_inbox"}))).await;
        assert_eq!(got["isError"], false);
        assert!(text(&got).contains("[Route]: route is on GET /health"));
        // And it is told where that text came from, so a peer cannot pose as its user.
        assert!(text(&got).contains("not from your user"));

        // Delivered once. Otherwise the same message lands in b's context on every single turn.
        let again = call_tool(&bus, "b", Some(&json!({"name": "check_inbox"}))).await;
        assert!(text(&again).contains("no new messages"));

        // The wire is still the authorization: c is on the canvas but not wired to a.
        let refused = call_tool(
            &bus,
            "a",
            Some(&json!({"name": "send_to_node", "arguments": {"nodeId": "c", "text": "hi"}})),
        )
        .await;
        assert_eq!(refused["isError"], true);
        assert!(text(&refused).contains("no edge"));
        assert!(
            text(&call_tool(&bus, "c", Some(&json!({"name": "check_inbox"}))).await)
                .contains("no new messages")
        );

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[tokio::test]
    async fn an_agent_can_bring_on_help_and_is_wired_to_it() {
        let dir = std::env::temp_dir().join(format!("identra-spawn-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        canvas::save(
            &dir,
            &Canvas {
                nodes: vec![Node {
                    id: "a".into(),
                    kind: "claude".into(),
                    x: 0.0,
                    y: 0.0,
                    width: 480.0,
                    height: 320.0,
                    title: "Lead".into(),
                    cwd: None,
                    locked: false,
                }],
                ..Canvas::default()
            },
        )
        .unwrap();

        // Stand in for the window, and record what it was asked for: I want to assert the request,
        // not just the answer, because the request is where the wiring decision lives.
        let seen: Arc<Mutex<Vec<CanvasCommand>>> = Arc::new(Mutex::new(Vec::new()));
        let slot: Arc<Mutex<Option<std::sync::Weak<Bus>>>> = Arc::new(Mutex::new(None));
        let (w, log) = (slot.clone(), seen.clone());
        let emit: Emit = Arc::new(move |cmd: CanvasCommand| {
            log.lock().unwrap().push(cmd.clone());
            if let Some(b) = w.lock().unwrap().as_ref().and_then(|x| x.upgrade()) {
                b.resolve_canvas(&cmd.request_id, json!({"ok": true, "id": "helper-1"}));
            }
        });
        let manager = Arc::new(TerminalManager::new(Arc::new(|_id, _out| {})));
        let bus = Arc::new(Bus::new(manager, Arc::new(Mutex::new(dir.clone())), emit));
        *slot.lock().unwrap() = Some(Arc::downgrade(&bus));

        let out = call_tool(&bus, "a", Some(&json!({"name": "add_terminal"}))).await;
        assert_eq!(out["isError"], false, "spawn should succeed: {out}");
        assert!(out["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("helper-1"));

        // Copy out of the lock before the next await, so the guard never spans one.
        let (action, kind, connect_to, count) = {
            let cmds = seen.lock().unwrap();
            (
                cmds[0].action.clone(),
                cmds[0].params["kind"].clone(),
                cmds[0].params["connectTo"].clone(),
                cmds.len(),
            )
        };
        assert_eq!(count, 1);
        assert_eq!(action, "add_terminal");
        // Defaulting to the caller's own kind: the one agent I know is installed and signed in.
        assert_eq!(kind, "claude");
        // The helper is wired back to whoever asked for it. An agent that cannot reach its caller
        // is not help.
        assert_eq!(connect_to, "a");

        // A canvas that refuses says why, and the agent hears it as a tool error it can act on.
        let refusing =
            bus_with_canvas(dir.clone(), json!({"ok": false, "error": "canvas is full"}));
        let out = call_tool(
            &refusing,
            "a",
            Some(&json!({"name": "add_note", "arguments": {"text": "look at this"}})),
        )
        .await;
        assert_eq!(out["isError"], true);
        assert!(out["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("canvas is full"));

        // Showing a file: a relative path resolves against the workspace, the request carries the
        // absolute path plus the caller to wire to, and a path outside the workspace never
        // reaches the canvas at all. The refusal happens here, on the bus, so the agent gets a
        // reason instead of the user getting a broken node.
        std::fs::write(dir.join("report.md"), "# findings").unwrap();
        let out = call_tool(
            &bus,
            "a",
            Some(&json!({"name": "show_file", "arguments": {"path": "report.md"}})),
        )
        .await;
        assert_eq!(out["isError"], false, "showing a real file works: {out}");
        let (action, shown, connect_to) = {
            let cmds = seen.lock().unwrap();
            let last = cmds.last().unwrap();
            (
                last.action.clone(),
                last.params["path"].clone(),
                last.params["connectTo"].clone(),
            )
        };
        assert_eq!(action, "show_file");
        assert_eq!(
            shown,
            dir.join("report.md").display().to_string(),
            "relative became absolute against the workspace"
        );
        assert_eq!(connect_to, "a", "the artifact is wired to who showed it");

        let before = seen.lock().unwrap().len();
        let out = call_tool(
            &bus,
            "a",
            Some(&json!({"name": "show_file", "arguments": {"path": "/etc/hostname"}})),
        )
        .await;
        assert_eq!(out["isError"], true, "an outside path is refused: {out}");
        assert_eq!(
            seen.lock().unwrap().len(),
            before,
            "and nothing was asked of the canvas"
        );

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[tokio::test]
    async fn two_agents_split_a_board_without_doing_the_same_work() {
        let dir = std::env::temp_dir().join(format!("identra-board-tool-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let bus = bus_in(dir.clone());

        async fn call(bus: &Bus, caller: &str, args: Value) -> Value {
            call_tool(bus, caller, Some(&args)).await
        }
        let text = |v: &Value| v["content"][0]["text"].as_str().unwrap().to_string();

        // One agent plans the work: the test cannot start until the route exists.
        let r = call(&bus, "a", json!({"name": "add_task", "arguments": {"description": "write GET /health in src/health.rs"}})).await;
        assert_eq!(text(&r), "added t1");
        let r = call(&bus, "a", json!({"name": "add_task", "arguments": {"description": "test it in tests/health.rs", "after": [1]}})).await;
        assert_eq!(text(&r), "added t2");

        // The other agent sees the same board. That is the point of it being shared.
        let board = text(&call(&bus, "b", json!({"name": "list_tasks"})).await);
        assert!(board.contains("- t1 [open, claimable] write GET /health"));
        assert!(board.contains("- t2 [blocked by t1]"));

        // a takes the route. b reaching for the test is refused with the reason, so it waits instead
        // of building against a route that does not exist yet.
        //
        // Whether b could take t1 off a depends on the live node set, and no PTY runs in this test,
        // so every claim here looks abandoned. The tasks tests cover that rule with an explicit live
        // set; I assert only what the live set cannot change.
        assert!(text(
            &call(
                &bus,
                "a",
                json!({"name": "claim_task", "arguments": {"id": 1}})
            )
            .await
        )
        .contains("claimed t1"));
        let b_tried = call(
            &bus,
            "b",
            json!({"name": "claim_task", "arguments": {"id": 2}}),
        )
        .await;
        assert_eq!(b_tried["isError"], true);
        assert!(
            text(&b_tried).contains("blocked until t1"),
            "got: {}",
            text(&b_tried)
        );

        // Finishing the route tells a that b now has something to do.
        let done = text(
            &call(
                &bus,
                "a",
                json!({"name": "complete_task", "arguments": {"id": 1, "note": "returns 200 ok"}}),
            )
            .await,
        );
        assert!(done.contains("done t1"));
        assert!(done.contains("now unblocked: t2"), "got: {done}");

        // And b can now take it.
        assert!(text(
            &call(
                &bus,
                "b",
                json!({"name": "claim_task", "arguments": {"id": 2}})
            )
            .await
        )
        .contains("claimed t2"));
        assert!(text(&call(&bus, "b", json!({"name": "list_tasks"})).await)
            .contains("- t1 [done: returns 200 ok]"));

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[tokio::test]
    async fn list_peers_reads_the_edge_and_title_from_the_canvas() {
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
            locked: false,
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
            seat: None,
            wallpaper: Default::default(),
        };
        canvas::save(&dir, &canvas).unwrap();

        let bus = bus_in(dir.clone());
        let out = call_tool(&bus, "a", Some(&json!({"name": "list_peers"}))).await;
        assert_eq!(out["isError"], false);
        let text = out["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("b\tTests"), "peer id and title: got {text}");

        // send to an unwired node is refused by the edge gate, surfaced as a tool error.
        let refused = call_tool(
            &bus,
            "a",
            Some(&json!({"name": "send_to_node", "arguments": {"nodeId": "zzz", "text": "hi"}})),
        )
        .await;
        assert_eq!(refused["isError"], true);

        std::fs::remove_dir_all(&dir).unwrap();
    }
}
