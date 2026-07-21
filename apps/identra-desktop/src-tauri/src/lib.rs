//! Identra's Tauri shell. Thin: it owns the window, holds the terminal manager, the context bus,
//! and the active workspace, and forwards typed commands to `identra-core`. All the real logic
//! lives in the engine so this file stays boring.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use identra_core::canvas::{self, Canvas};
use identra_core::session;
use identra_core::terminal::{Event as TerminalEvent, TerminalManager};
use identra_core::workspace::{self, WorkspaceMeta};
use identra_core::{detect, AgentInfo};
use identra_mcp::server::Bus;
use identra_memory::Memory;
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_dialog::DialogExt;

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
    /// each CLI reads, and the guide that tells the agents they can work with each other. I do this
    /// on every open because the bus port changes per launch, so a stale file would point a node at
    /// a port nothing is listening on.
    ///
    /// Three files because three CLIs disagree about where config lives. codex needs none, it takes
    /// its bus config as launch arguments.
    fn activate(&self, path: PathBuf) -> Result<(), String> {
        identra_mcp::config::write_mcp_json(&path, self.mcp_port).map_err(|e| e.to_string())?;
        identra_mcp::config::write_gemini_settings(&path, self.mcp_port)
            .map_err(|e| e.to_string())?;
        identra_mcp::config::write_opencode_config(&path, self.mcp_port)
            .map_err(|e| e.to_string())?;
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

/// Pushed once, when a node's agent is gone. The node needs this to stop looking busy.
#[derive(Clone, Serialize)]
struct ExitEvent {
    id: String,
    code: Option<u32>,
}

/// The ring-buffer replay a node reads on (re)attach.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Snapshot {
    data: Vec<u8>,
    last_seq: u64,
}

/// How often to look for each agent's session. Often enough that a conversation started a moment
/// ago is remembered before the user quits, rare enough to be free: it reads a few files in /proc.
const SESSION_SAMPLE: std::time::Duration = std::time::Duration::from_secs(3);

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
    // The extra args carry whatever that CLI cannot take from a file (codex takes the server
    // inline, claude gets pointed at the workspace .mcp.json, gemini needs the folder trusted), and
    // the env carries this node's own secret, which is what the bus reads its identity from.
    // Minting it here, per node, is what stops one agent claiming to be another; it goes into the
    // child's env and never touches the frontend or the disk.
    let mut args = args;

    // Pick the conversation this node was having back up, if it still exists. This goes on before
    // the bus wiring and the ordering is load bearing: claude's --mcp-config takes a list, so
    // anything after it that is not a flag is swallowed as another config path.
    if let Some(previous) = session::load(&workspace, &id) {
        if previous.agent == kind {
            if let Some(resume) = session::resume_args(&previous) {
                args.extend(resume);
            }
        }
    }

    args.extend(identra_mcp::config::launch_args(
        &kind,
        state.mcp_port,
        &workspace,
    ));
    let token = state.bus.issue_token(&id);
    let env = identra_mcp::config::launch_env(&kind, state.mcp_port, &token, &id, &workspace);

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

/// Kill a node's agent. Deleting a node from the canvas is a deliberate act, so its conversation is
/// forgotten too: the alternative is a new node with the same id silently inheriting a dead one's
/// session, which is the wrong conversation arriving from nowhere.
#[tauri::command]
fn terminal_kill(state: State<AppState>, id: String) -> Result<(), String> {
    session::forget(&state.dir(), &id);
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

/// The canvas reporting what it did with a command an agent asked for. The request id is what
/// matches this answer to the agent still waiting on it.
#[tauri::command]
fn canvas_command_result(state: State<AppState>, request_id: String, result: serde_json::Value) {
    state.bus.resolve_canvas(&request_id, result);
}

/// The task board, for the human. The agents coordinate through this and until now it was invisible
/// from the outside: you could watch two terminals scroll and still not know who had what.
#[tauri::command]
fn board_list(state: State<AppState>) -> Result<Vec<identra_mcp::tasks::Task>, String> {
    identra_mcp::tasks::Board::open(&state.dir())?.list()
}

/// The project scope for a memory filter: the workspace folder name, matching how the bus scopes
/// what the agents write, so the panel reads the same pool the agents do.
fn memory_filter(dir: &std::path::Path) -> identra_memory::Filter {
    let user_id = dir
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "workspace".into());
    identra_memory::Filter {
        user_id: Some(user_id),
        ..Default::default()
    }
}

/// What the project has learned, newest first. Same argument as the board: memory that only agents
/// can read is memory the user cannot check, correct, or trust.
#[tauri::command]
fn memory_list(state: State<AppState>, limit: Option<usize>) -> Result<Vec<Memory>, String> {
    let dir = state.dir();
    // Through the bus opener, not Store::open, so the panel sees the same store the agents do,
    // embedder and all. Browsing does not use the embedder, but going through one door means the
    // human's list and the human's search cannot drift apart.
    let store = identra_mcp::server::open_memory(&dir)?;
    store
        .recent(&memory_filter(&dir), limit.unwrap_or(50))
        .map_err(|e| e.to_string())
}

/// Search what the project has learned. Same ranking the agents get: with a model, by meaning;
/// without one, by words. This is why it goes through the bus opener rather than a bare store.
#[tauri::command]
fn memory_search(
    state: State<AppState>,
    query: String,
    limit: Option<usize>,
) -> Result<Vec<Memory>, String> {
    let dir = state.dir();
    let store = identra_mcp::server::open_memory(&dir)?;
    store
        .search(&memory_filter(&dir), &query, limit.unwrap_or(50))
        .map_err(|e| e.to_string())
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
///
/// The slug is looked up among the workspaces that actually exist, rather than joined onto the root
/// and checked afterwards. Joining would mean trusting a name from the window: `../../somewhere`
/// resolves out of the workspaces root, and activating a workspace writes files into it and points
/// every agent there. Choosing from a list I built cannot be made to leave the root at all, and it
/// is no more code than validating the name would be.
#[tauri::command]
fn workspace_open(state: State<AppState>, slug: String) -> Result<Canvas, String> {
    let root = workspaces_root()?;
    let found = workspace::list(&root)
        .into_iter()
        .find(|w| w.slug == slug)
        .ok_or_else(|| format!("no workspace named {slug}"))?;
    let path = PathBuf::from(&found.path);
    state.activate(path.clone())?;
    Ok(canvas::load(&path))
}

/// Folders the user has opened as workspaces before.
#[tauri::command]
fn workspace_recents() -> Vec<WorkspaceMeta> {
    workspace::recents()
}

/// Ask for a folder and open it as a workspace.
///
/// The picker is the authorization, which is why this is one command rather than a pick that hands
/// a path to the window and an open that takes one back. A path that never crosses that boundary is
/// a path the window cannot choose: opening a workspace writes into the folder and points every
/// agent at it, so "the user picked it in a native dialog" has to be the only way one gets here.
///
/// Returns `None` when the dialog is cancelled, which is an answer, not a failure.
#[tauri::command]
async fn workspace_pick_folder(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<Option<WorkspaceMeta>, String> {
    let picked = app
        .dialog()
        .file()
        .set_title("Open a folder as a workspace")
        .blocking_pick_folder();
    let Some(folder) = picked else {
        return Ok(None);
    };
    let path = folder
        .into_path()
        .map_err(|e| format!("that folder cannot be opened: {e}"))?;
    let meta = workspace::adopt(&path).map_err(|e| e.to_string())?;
    state.activate(path)?;
    Ok(Some(meta))
}

/// Reopen a folder from the remembered list.
///
/// Same rule as opening a workspace by slug: the path has to be one already on the list, which the
/// user put there by picking it. A path from the window is not evidence of anything.
#[tauri::command]
fn workspace_open_recent(state: State<AppState>, path: String) -> Result<Canvas, String> {
    let known = workspace::recents()
        .into_iter()
        .find(|w| w.path == path)
        .ok_or_else(|| "that folder is not one you have opened".to_string())?;
    let dir = PathBuf::from(&known.path);
    state.activate(dir.clone())?;
    Ok(canvas::load(&dir))
}

/// Drop a folder from the remembered list. The folder itself is untouched.
#[tauri::command]
fn workspace_forget_recent(path: String) {
    workspace::forget_recent(&PathBuf::from(path));
}

/// Rename a workspace, which moves its folder. If it was the active one, follow it: the old path
/// no longer exists, and every canvas save and every agent launch reads that path.
#[tauri::command]
fn workspace_rename(
    state: State<AppState>,
    slug: String,
    title: String,
) -> Result<WorkspaceMeta, String> {
    let root = workspaces_root()?;
    let was_active = state.dir() == root.join(&slug);
    let meta = workspace::rename(&root, &slug, &title).map_err(|e| e.to_string())?;
    if was_active {
        state.activate(PathBuf::from(&meta.path))?;
    }
    Ok(meta)
}

/// Delete a workspace Identra made, and everything in it. The window asks first: this takes the
/// user's files, not just the canvas.
///
/// The slug is looked up among the workspaces in the root, never joined onto it. Joining is what
/// makes this dangerous: an absolute path replaces the base rather than extending it, so a slug of
/// `/home/me/my-repo` would resolve to exactly that and recursively delete a folder that was only
/// ever adopted. A folder the user already had is not ours to delete at all, and it is not on this
/// list, so it cannot reach the delete.
///
/// Any agent still running in it is killed first. Leaving a PTY alive with its working directory
/// deleted gives an agent that fails every command for a reason it cannot see.
#[tauri::command]
fn workspace_delete(state: State<AppState>, slug: String) -> Result<(), String> {
    let root = workspaces_root()?;
    let found = workspace::list(&root)
        .into_iter()
        .find(|w| w.slug == slug)
        .ok_or_else(|| {
            "that is not a workspace Identra made, so it is not Identra's to delete".to_string()
        })?;
    if state.dir().as_path() == Path::new(&found.path) {
        for id in state.manager.ids() {
            let _ = state.manager.kill(&id);
        }
    }
    workspace::delete(&root, &found.slug).map_err(|e| e.to_string())
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            // The sink emits each output chunk to the webview as it arrives.
            let handle: AppHandle = app.handle().clone();
            let manager = Arc::new(TerminalManager::new(Arc::new(
                move |id: String, event: TerminalEvent| match event {
                    TerminalEvent::Output(out) => {
                        let _ = handle.emit(
                            "terminal://output",
                            OutputEvent {
                                id,
                                seq: out.seq,
                                data: out.data,
                            },
                        );
                    }
                    TerminalEvent::Exit { code } => {
                        let _ = handle.emit("terminal://exit", ExitEvent { id, code });
                    }
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
            // How an agent's canvas request reaches the canvas. The window owns that state and is
            // its only writer, so the bus asks rather than writes, and waits for the answer.
            let canvas_handle: AppHandle = app.handle().clone();
            let emit: identra_mcp::server::Emit = Arc::new(move |cmd| {
                let _ = canvas_handle.emit("canvas://command", cmd);
            });
            let bus = Arc::new(Bus::new(manager.clone(), project_dir.clone(), emit));
            let bus_for_task = bus.clone();
            tauri::async_runtime::spawn(async move {
                if let Err(e) = identra_mcp::server::serve(listener, bus_for_task).await {
                    eprintln!("identra: context bus stopped: {e}");
                }
            });

            // Watch what conversation each agent is having, so closing the app does not throw it
            // away. None of these CLIs will tell you their session id, but each keeps its transcript
            // open, so the answer is readable off the live process. It has to be sampled rather than
            // read once: the id does not exist until the agent has started and opened the file.
            let watcher = manager.clone();
            let watched_dir = project_dir.clone();
            std::thread::spawn(move || loop {
                std::thread::sleep(SESSION_SAMPLE);
                let dir = watched_dir.lock().unwrap().clone();
                for id in watcher.ids() {
                    let Some(pid) = watcher.pid(&id) else {
                        continue;
                    };
                    if let Some(found) = session::detect(pid) {
                        // Cheap and idempotent: the same session rewrites the same file. Only a
                        // change matters, and comparing costs as much as writing.
                        let _ = session::save(&dir, &id, &found);
                    }
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
            canvas_command_result,
            board_list,
            memory_list,
            memory_search,
            workspace_list,
            workspace_create,
            workspace_open,
            workspace_rename,
            workspace_delete,
            workspace_recents,
            workspace_pick_folder,
            workspace_open_recent,
            workspace_forget_recent
        ])
        .run(tauri::generate_context!())
        .expect("error while running Identra");
}
