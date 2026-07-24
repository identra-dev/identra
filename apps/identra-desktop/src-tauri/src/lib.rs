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

/// Which agent should hold the orchestrator seat on this machine, by id, or `None` when nothing
/// installed can hold it.
///
/// The ranking lives in the engine rather than in the UI so it is testable and so there is one
/// answer, and it ranks on capability rather than on brand. The UI treats this as a default it
/// offers, never as a decision: the user reassigns the seat whenever they want.
#[tauri::command]
fn default_orchestrator() -> Option<String> {
    identra_core::agents::best_orchestrator(&detect()).map(|a| a.id.clone())
}

/// The briefing the seat agent is given before the user's first instruction. The UI sends it, so
/// the UI has to be able to read it, but the text belongs next to the workspace guide it builds on.
#[tauri::command]
fn seat_brief() -> &'static str {
    identra_mcp::config::SEAT_BRIEF
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
    let mut env = identra_mcp::config::launch_env(&kind, state.mcp_port, &token, &id, &workspace);
    // The child's PATH leads with the executable's own directory plus everything discovery
    // searched. A GUI launch inherits the bare system PATH, and codex is an env-node script, so
    // without this an agent found under nvm spawns and immediately dies looking for node.
    env.push(("PATH".into(), identra_core::agents::launch_path(&cmd)));

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

/// What a node is doing right now, asked rather than pushed.
///
/// The node already knows it is running, because output is arriving, and it already knows it has
/// exited, because the engine told it. The one thing it cannot see for itself is whether the quiet
/// it just fell into is an agent that finished or an agent waiting on an answer, and that needs the
/// tail of the transcript. So the node asks exactly once, at the moment it settles, rather than the
/// engine pushing a fourth event or anything polling on a timer.
#[tauri::command]
fn terminal_status(state: State<AppState>, id: String) -> Option<identra_core::terminal::Status> {
    state.manager.status(&id)
}

/// Tear a node down completely. Everything a node owns has to go here, because this is the only
/// path a closed node takes: the PTY and its child, the conversation it was resuming, and its bus
/// credential. Leaving any one of them behind is the difference between closing a node and hiding
/// it.
///
/// The conversation goes deliberately. Closing a node is a considered act, and keeping the session
/// would mean a later node reusing that id silently inherits a dead one's history, which is the
/// wrong conversation arriving from nowhere.
#[tauri::command]
fn terminal_kill(state: State<AppState>, id: String) -> Result<(), String> {
    session::forget(&state.dir(), &id);
    state.bus.revoke_node(&id);
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
/// Write the current board to a file the user picks. `Ok(false)` means they cancelled the dialog,
/// which is not a failure and must not be reported as one.
///
/// The canvas comes from the window rather than from disk, so what is exported is the board on
/// screen including any change that has not hit the debounced save yet. Exporting a slightly stale
/// file would be a bug nobody would think to look for.
#[tauri::command]
async fn canvas_export(app: AppHandle, canvas: Canvas) -> Result<bool, String> {
    let Some(target) = app
        .dialog()
        .file()
        .set_title("Export this canvas")
        .set_file_name("canvas.identra.json")
        .add_filter("Identra canvas", &["json"])
        .blocking_save_file()
    else {
        return Ok(false);
    };
    let path = target
        .into_path()
        .map_err(|e| format!("that location cannot be written: {e}"))?;
    std::fs::write(path, canvas::export(&canvas))
        .map_err(|e| format!("could not write it: {e}"))?;
    Ok(true)
}

/// Read a canvas from a file the user picks, and make it this workspace's board.
///
/// `Ok(None)` is a cancelled dialog. Anything that is not one of our exports is refused by name,
/// because every field on a Canvas has a default and so any JSON object at all would otherwise
/// import as a blank board and replace the real one.
///
/// The imported board is saved before it is returned. Import is the one operation that discards
/// what was there, so leaving the new state only in the window would mean a crash before the next
/// debounced save loses both the old board and the new one.
#[tauri::command]
async fn canvas_import(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<Option<Canvas>, String> {
    let Some(chosen) = app
        .dialog()
        .file()
        .set_title("Import a canvas")
        .add_filter("Identra canvas", &["json"])
        .blocking_pick_file()
    else {
        return Ok(None);
    };
    let path = chosen
        .into_path()
        .map_err(|e| format!("that file cannot be opened: {e}"))?;
    let text = std::fs::read_to_string(&path).map_err(|e| format!("could not read it: {e}"))?;
    let imported = canvas::import(&text).map_err(|e| e.to_string())?;
    canvas::save(&state.dir(), &imported).map_err(|e| e.to_string())?;
    Ok(Some(imported))
}

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

/// The first fact a workspace ever learns is worth surfacing once: it is the moment the promise
/// that any agent you open already knows this project first comes true here. This records that the
/// reveal has happened, durably, in the workspace's own `.identra/`, so it fires exactly once per
/// workspace and never again, surviving reinstalls. Returns true only on the call that created the
/// marker, so the window opens the panel on that call and never re-opens on a later fact.
#[tauri::command]
fn memory_reveal_once(state: State<AppState>) -> bool {
    let marker = state.dir().join(".identra").join("revealed");
    if let Some(parent) = marker.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    // create_new: two quick calls cannot both claim the reveal, the loser gets AlreadyExists and
    // false. Any other error (an unwritable workspace) also reads as "not the first", which is the
    // safe way to be wrong: at worst the panel does not auto-open, it never double-opens.
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&marker)
        .is_ok()
}

/// The command that starts this workspace's dev server, or null when the project does not
/// declare one. The UI shows a Run button exactly when this answers.
#[tauri::command]
fn dev_command(state: State<AppState>) -> Option<Vec<String>> {
    identra_core::devserver::command_for(&state.dir())
}

/// Read a file for the viewer node. The engine refuses anything that does not resolve inside
/// the active workspace, which is what makes it safe to accept a path from the window: on this
/// command a path is an agent's word as easily as the user's.
#[tauri::command]
fn file_read(
    state: State<AppState>,
    path: String,
) -> Result<identra_core::fileview::FileView, String> {
    identra_core::fileview::read(&state.dir(), Path::new(&path)).map_err(|e| e.to_string())
}

/// One directory of the workspace, for the Files panel. `rel` is workspace-relative; the engine
/// refuses anything that resolves outside.
#[tauri::command]
fn files_list(
    state: State<AppState>,
    rel: String,
) -> Result<Vec<identra_core::files::Entry>, String> {
    identra_core::files::list(&state.dir(), &rel).map_err(|e| e.to_string())
}

/// Names and text content under the workspace, case-insensitive, capped in the engine.
#[tauri::command]
fn files_search(state: State<AppState>, query: String) -> Vec<identra_core::files::Hit> {
    identra_core::files::search(&state.dir(), &query, 50)
}

/// Show a workspace file in the OS file manager. The path is checked before anything is spawned,
/// and what opens is the containing folder: revealing is about where the file lives.
#[tauri::command]
fn file_reveal(state: State<AppState>, rel: String) -> Result<(), String> {
    let full = identra_core::files::resolve(&state.dir(), &rel).map_err(|e| e.to_string())?;
    #[cfg(target_os = "macos")]
    let spawned = std::process::Command::new("open")
        .arg("-R")
        .arg(&full)
        .spawn();
    #[cfg(not(target_os = "macos"))]
    let spawned = std::process::Command::new("xdg-open")
        .arg(full.parent().unwrap_or(&full))
        .spawn();
    spawned
        .map(|_| ())
        .map_err(|e| format!("could not open the file manager: {e}"))
}

/// What is true of this machine, for the settings panel to show.
#[tauri::command]
fn settings_get() -> identra_core::settings::Settings {
    identra_core::settings::load()
}

/// Write the settings. The panel notes that the embeddings choice lands at the next launch: the
/// engine reads it once per process, at the first memory call, so a mid-session toggle is a
/// promise about next time rather than a lie about now.
#[tauri::command]
fn settings_set(settings: identra_core::settings::Settings) -> Result<(), String> {
    identra_core::settings::save(&settings).map_err(|e| e.to_string())
}

/// The wallpaper library's images, as absolute paths. The frontend turns each into an asset URL;
/// the built-in backgrounds never appear here because they are drawn from CSS, not from files.
#[tauri::command]
fn wallpapers_list() -> Vec<String> {
    let Some(dir) = identra_core::wallpaper::library() else {
        return Vec::new();
    };
    identra_core::wallpaper::list(&dir)
        .into_iter()
        .map(|p| p.display().to_string())
        .collect()
}

/// Ask for an image and copy it into the library. Same rule as every other dialog command: the
/// picker is the authorization, so the chosen path never crosses the webview boundary in either
/// direction until it has been copied into the one directory the asset scope serves.
///
/// `Ok(None)` is a cancelled dialog, which is an answer, not a failure.
#[tauri::command]
async fn wallpaper_add(app: AppHandle) -> Result<Option<String>, String> {
    let Some(dir) = identra_core::wallpaper::library() else {
        return Err("cannot find a home directory for the wallpaper library".into());
    };
    let Some(chosen) = app
        .dialog()
        .file()
        .set_title("Add a wallpaper")
        .add_filter("Images", &["png", "jpg", "jpeg", "webp"])
        .blocking_pick_file()
    else {
        return Ok(None);
    };
    let source = chosen
        .into_path()
        .map_err(|e| format!("that file cannot be opened: {e}"))?;
    let stored = identra_core::wallpaper::add(&dir, &source)
        .map_err(|e| format!("that image could not be added: {e}"))?;
    Ok(Some(stored.display().to_string()))
}

/// Remove an image from the library. The engine refuses any path that is not directly inside the
/// library directory, which is what makes it safe to accept a path from the window at all.
#[tauri::command]
fn wallpaper_remove(path: String) -> Result<(), String> {
    let Some(dir) = identra_core::wallpaper::library() else {
        return Err("cannot find the wallpaper library".into());
    };
    identra_core::wallpaper::remove(&dir, Path::new(&path)).map_err(|e| e.to_string())
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

/// Clone a git repository into the workspaces root and open it as a workspace.
///
/// Async, because a clone takes as long as the network says it does and the window must not
/// freeze behind it. The URL is pasted text and stays data all the way down: the engine hands it
/// to git after `--`, never through a shell.
#[tauri::command]
async fn workspace_clone(state: State<'_, AppState>, url: String) -> Result<WorkspaceMeta, String> {
    let root = workspaces_root()?;
    let meta = workspace::clone_repo(&root, &url).map_err(|e| e.to_string())?;
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
        .plugin(tauri_plugin_updater::Builder::new().build())
        .setup(|app| {
            // The window is built here rather than declared in tauri.conf.json for exactly one
            // reason: the navigation guard only exists on the builder. The shell's own webview
            // must never become someone's webpage, and a tester proved it can: an unsandboxed
            // frame plus WebKit's backspace-goes-back walked the whole window off to an external
            // site. The frontend closes both of those doors; this closes the class, whatever
            // surface asks for a navigation in the future.
            tauri::WebviewWindowBuilder::new(app, "main", Default::default())
                .title("Identra")
                .inner_size(1280.0, 820.0)
                .min_inner_size(800.0, 560.0)
                .on_navigation(|url| match url.scheme() {
                    // The app itself, packaged, and its asset protocol.
                    "tauri" | "asset" => true,
                    // The vite dev server during `tauri dev`, and nothing else over http.
                    "http" | "https" => cfg!(dev) && url.host_str() == Some("localhost"),
                    "about" => true,
                    _ => false,
                })
                .build()?;
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
            default_orchestrator,
            seat_brief,
            terminal_start,
            terminal_input,
            terminal_resize,
            terminal_snapshot,
            terminal_status,
            terminal_kill,
            canvas_load,
            canvas_save,
            canvas_export,
            canvas_import,
            canvas_command_result,
            board_list,
            memory_list,
            memory_search,
            memory_reveal_once,
            dev_command,
            file_read,
            file_reveal,
            files_list,
            files_search,
            settings_get,
            settings_set,
            wallpapers_list,
            wallpaper_add,
            wallpaper_remove,
            workspace_list,
            workspace_clone,
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
