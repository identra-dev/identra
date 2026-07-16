//! Identra's Tauri shell. Thin: it owns the window, holds one [`TerminalManager`] and the
//! project directory in managed state, and forwards typed commands to `identra-core`. All the
//! real logic lives in the engine so this file stays boring.

use std::path::PathBuf;
use std::sync::Arc;

use identra_core::canvas::{self, Canvas};
use identra_core::terminal::{Output, TerminalManager};
use identra_core::{detect, AgentInfo};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};

struct AppState {
    manager: Arc<TerminalManager>,
    project_dir: PathBuf,
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
    state
        .manager
        .start(id, &cmd, &args, Some(&dir), rows, cols)
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
            // Project = the dir Identra launched in. A real "open project" picker is
            // its own follow-up; `.identra/canvas.json` lands here for now.
            let project_dir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
            app.manage(AppState {
                manager,
                project_dir,
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
            canvas_save
        ])
        .run(tauri::generate_context!())
        .expect("error while running Identra");
}
