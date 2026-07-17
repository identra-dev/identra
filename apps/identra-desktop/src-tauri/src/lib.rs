//! Identra's Tauri shell. Thin: it owns the window, holds one [`TerminalManager`] and the
//! project directory in managed state, and forwards typed commands to `identra-core`. All the
//! real logic lives in the engine so this file stays boring.

use std::path::PathBuf;
use std::sync::Arc;

use identra_core::canvas::{self, Canvas};
use identra_core::terminal::{Output, TerminalManager};
use identra_core::{detect, AgentInfo};
use identra_mcp::server::Bus;
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, RunEvent, State};

struct AppState {
    manager: Arc<TerminalManager>,
    project_dir: PathBuf,
    // The context bus lives in this process and shares `manager`. I keep the port so the config
    // writer can point codex at the running server.
    bus: Arc<Bus>,
    mcp_port: u16,
}

/// Pushed to the webview once per output chunk. The node writes `data` straight into xterm.
#[derive(Clone, Serialize)]
struct OutputEvent {
    id: String,
    seq: u64,
    data: Vec<u8>,
}

/// The ring-buffer replay a node reads on (re)attach.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Snapshot {
    data: Vec<u8>,
    last_seq: u64,
}

#[tauri::command]
fn detect_agents() -> Vec<AgentInfo> {
    detect()
}

#[tauri::command]
fn terminal_start(
    state: State<AppState>,
    id: String,
    cmd: String,
    args: Vec<String>,
    cwd: Option<String>,
    rows: u16,
    cols: u16,
) -> Result<(), String> {
    let dir = cwd.unwrap_or_else(|| state.project_dir.display().to_string());
    // Mint this node's bus bearer and hand it to the CLI through the env, so the token never
    // reaches the frontend. A node with no edges gets a token too and simply never uses it.
    let token = state.bus.issue_token(&id);
    let env = [("IDENTRA_BUS_TOKEN".to_string(), token)];
    state
        .manager
        .start(id, &cmd, &args, Some(&dir), &env, rows, cols)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn terminal_input(state: State<AppState>, id: String, data: String) -> Result<(), String> {
    state
        .manager
        .input(&id, data.as_bytes())
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn terminal_resize(state: State<AppState>, id: String, rows: u16, cols: u16) -> Result<(), String> {
    state
        .manager
        .resize(&id, rows, cols)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn terminal_snapshot(state: State<AppState>, id: String) -> Option<Snapshot> {
    state
        .manager
        .snapshot(&id)
        .map(|(data, last_seq)| Snapshot { data, last_seq })
}

#[tauri::command]
fn terminal_kill(state: State<AppState>, id: String) -> Result<(), String> {
    state.manager.kill(&id).map_err(|e| e.to_string())
}

#[tauri::command]
fn canvas_load(state: State<AppState>) -> Canvas {
    canvas::load(&state.project_dir)
}

#[tauri::command]
fn canvas_save(state: State<AppState>, canvas: Canvas) -> Result<(), String> {
    canvas::save(&state.project_dir, &canvas).map_err(|e| e.to_string())
}

/// Write the bus into codex's config so a codex node picks up the three tools at launch. The
/// frontend calls this when the first edge is drawn, before the wired nodes are launched, because
/// codex reads its MCP servers only at startup. Writes an Identra-owned block and backs up the
/// original; the block is removed again on exit.
#[tauri::command]
fn write_agent_mcp_config(state: State<AppState>) -> Result<(), String> {
    let path = identra_mcp::config::codex_config_path()
        .ok_or_else(|| "cannot locate the codex config (no HOME or CODEX_HOME set)".to_string())?;
    identra_mcp::config::write_codex_bus(&path, state.mcp_port).map_err(|e| e.to_string())
}

pub fn run() {
    let app = tauri::Builder::default()
        .setup(|app| {
            // The sink emits each output chunk to the webview as it arrives.
            let handle: AppHandle = app.handle().clone();
            let manager = Arc::new(TerminalManager::new(Arc::new(
                move |id: String, out: Output| {
                    let _ = handle.emit(
                        "terminal://output",
                        OutputEvent {
                            id,
                            seq: out.seq,
                            data: out.data,
                        },
                    );
                },
            )));
            // Project = the dir Identra launched in. A real "open project" picker is
            // its own follow-up; `.identra/canvas.json` lands here for now.
            let project_dir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));

            // Bring the bus up before the window is interactive, so its port is known when the
            // first edge is drawn. It shares `manager`, so a peer read hits the same live PTY.
            let (listener, mcp_port) = identra_mcp::server::bind()?;
            let bus = Arc::new(Bus::new(manager.clone(), project_dir.clone()));
            let bus_for_task = bus.clone();
            tauri::async_runtime::spawn(async move {
                if let Err(e) = identra_mcp::server::serve(listener, bus_for_task).await {
                    eprintln!("identra: context bus stopped: {e}");
                }
            });

            app.manage(AppState {
                manager,
                project_dir,
                bus,
                mcp_port,
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            detect_agents,
            terminal_start,
            terminal_input,
            terminal_resize,
            terminal_snapshot,
            terminal_kill,
            canvas_load,
            canvas_save,
            write_agent_mcp_config
        ])
        .build(tauri::generate_context!())
        .expect("error while building Identra");

    app.run(|_handle, event| {
        // On exit, take Identra's block back out of codex's config so the user's normal CLI is
        // exactly as it was. Best effort: the app is closing, so I report a failure to stderr
        // rather than swallow it, but there is no UI left to show it in.
        if let RunEvent::Exit = event {
            if let Some(path) = identra_mcp::config::codex_config_path() {
                if let Err(e) = identra_mcp::config::restore_codex(&path) {
                    eprintln!("identra: could not restore codex config: {e}");
                }
            }
        }
    });
}
