//! Identra's Tauri shell. Thin: it owns the window, holds the terminal manager, the context bus,
//! and the active workspace, and forwards typed commands to `identra-core`. All the real logic
//! lives in the engine so this file stays boring.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use identra_core::canvas::{self, Canvas};
use identra_core::terminal::{Output, TerminalManager};
use identra_core::workspace::{self, WorkspaceMeta};
use identra_core::{detect, AgentInfo};
use identra_mcp::server::Bus;
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};

struct AppState {
    manager: Arc<TerminalManager>,
    /// The active workspace directory. Shared with the bus (not copied) so switching workspace
    /// moves both: the tools have to read the canvas the user is actually looking at.
    project_dir: Arc<Mutex<PathBuf>>,
    bus: Arc<Bus>,
    mcp_port: u16,
}

impl AppState {
    fn dir(&self) -> PathBuf {
        self.project_dir.lock().unwrap().clone()
    }

    /// Point the app at a workspace and make sure that folder is ready for agents: the bus config
    /// claude reads, and the guide that tells the agents they can work with each other. I do this
    /// on every open because the bus port changes per launch.
    fn activate(&self, path: PathBuf) -> Result<(), String> {
        identra_mcp::config::write_mcp_json(&path, self.mcp_port).map_err(|e| e.to_string())?;
        identra_mcp::config::write_guides(&path).map_err(|e| e.to_string())?;
        *self.project_dir.lock().unwrap() = path;
        Ok(())
    }
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

fn workspaces_root() -> Result<PathBuf, String> {
    workspace::root().ok_or_else(|| "cannot find a home directory for workspaces".to_string())
}

#[tauri::command]
fn detect_agents() -> Vec<AgentInfo> {
    detect()
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
fn terminal_start(
    state: State<AppState>,
    id: String,
    kind: String,
    cmd: String,
    args: Vec<String>,
    cwd: Option<String>,
    rows: u16,
    cols: u16,
) -> Result<(), String> {
    let workspace = state.dir();
    let dir = cwd.unwrap_or_else(|| workspace.display().to_string());

    // Put this node on the bus at launch, because every CLI reads its MCP servers once at startup.
    // The extra args carry the server (codex takes it inline, claude gets pointed at the workspace
    // .mcp.json), and the env carries who this node is. The token never touches the frontend.
    let mut args = args;
    args.extend(identra_mcp::config::launch_args(
        &kind,
        state.mcp_port,
        &workspace,
    ));
    let env = identra_mcp::config::launch_env(state.mcp_port, state.bus.token(), &id);

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
    canvas::load(&state.dir())
}

#[tauri::command]
fn canvas_save(state: State<AppState>, canvas: Canvas) -> Result<(), String> {
    canvas::save(&state.dir(), &canvas).map_err(|e| e.to_string())
}

#[tauri::command]
fn workspace_list() -> Result<Vec<WorkspaceMeta>, String> {
    let root = workspaces_root()?;
    // The root not existing yet is not an error, it just means no workspaces.
    let _ = std::fs::create_dir_all(&root);
    Ok(workspace::list(&root))
}

/// Make a workspace and open it. The folder is the workspace, and it is also the directory the
/// agents in it will run in.
#[tauri::command]
fn workspace_create(
    state: State<AppState>,
    title: Option<String>,
) -> Result<WorkspaceMeta, String> {
    let root = workspaces_root()?;
    std::fs::create_dir_all(&root).map_err(|e| e.to_string())?;
    let meta = workspace::create(&root, title.as_deref().unwrap_or(workspace::DEFAULT_TITLE))
        .map_err(|e| e.to_string())?;
    state.activate(PathBuf::from(&meta.path))?;
    Ok(meta)
}

/// Switch to an existing workspace and hand back its canvas.
#[tauri::command]
fn workspace_open(state: State<AppState>, slug: String) -> Result<Canvas, String> {
    let path = workspaces_root()?.join(&slug);
    if !canvas::canvas_path(&path).is_file() {
        return Err(format!("no workspace named {slug}"));
    }
    state.activate(path.clone())?;
    Ok(canvas::load(&path))
}

pub fn run() {
    tauri::Builder::default()
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

            // Until the user picks a workspace there is no canvas to read, so the active dir starts
            // at the workspaces root and every canvas command is a no-op blank board. The frontend
            // shows the workspace picker in that state.
            let start_dir = workspace::root().unwrap_or_else(|| PathBuf::from("."));
            let project_dir = Arc::new(Mutex::new(start_dir));

            // Bring the bus up before the window is interactive, so its port is known when a
            // workspace opens and writes the config the agents read at launch.
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
            workspace_list,
            workspace_create,
            workspace_open
        ])
        .run(tauri::generate_context!())
        .expect("error while running Identra");
}
