//! The canvas store. The layout is the source of truth and lives in the user's project at
//! `.identra/canvas.json`. Writes are atomic (temp file + rename); debouncing is the caller's
//! job — the UI already knows when a drag ends, so it throttles there instead of here.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// One node on the board. For now every node is a terminal running an agent CLI.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Node {
    pub id: String,
    #[serde(default = "codex_kind")]
    pub kind: String,
    pub x: f64,
    pub y: f64,
    #[serde(default = "default_w")]
    pub width: f64,
    #[serde(default = "default_h")]
    pub height: f64,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub cwd: Option<String>,
}

/// A wire between two nodes. The edge is also the authorization for the context bus: two nodes
/// share context only when an edge joins them.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Edge {
    pub id: String,
    pub source: String,
    pub target: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Viewport {
    pub x: f64,
    pub y: f64,
    pub zoom: f64,
}

impl Default for Viewport {
    fn default() -> Self {
        Viewport {
            x: 0.0,
            y: 0.0,
            zoom: 1.0,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Canvas {
    #[serde(default)]
    pub nodes: Vec<Node>,
    // #[serde(default)] so an older canvas.json with no edges still loads.
    #[serde(default)]
    pub edges: Vec<Edge>,
    #[serde(default)]
    pub viewport: Viewport,
    /// The workspace's display name. The folder is the id, this is what the user reads and edits,
    /// so it lives with the canvas rather than in a second metadata file.
    #[serde(default = "default_title")]
    pub title: String,
}

// I write Default by hand because the derived one would give an empty title, and a canvas with no
// name is not a state I want anywhere: a blank board is still "untitled-workspace".
impl Default for Canvas {
    fn default() -> Self {
        Canvas {
            nodes: Vec::new(),
            edges: Vec::new(),
            viewport: Viewport::default(),
            title: default_title(),
        }
    }
}

fn default_title() -> String {
    "untitled-workspace".into()
}

fn codex_kind() -> String {
    "codex".into()
}
fn default_w() -> f64 {
    480.0
}
fn default_h() -> f64 {
    320.0
}

pub fn canvas_path(project_dir: &Path) -> PathBuf {
    project_dir.join(".identra").join("canvas.json")
}

/// Read the saved canvas, or an empty one if there's nothing valid on disk. Never fails —
/// a missing or corrupt file just means a blank board, not a crash.
pub fn load(project_dir: &Path) -> Canvas {
    std::fs::read_to_string(canvas_path(project_dir))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

/// Write the canvas atomically. `create_dir_all` makes `.identra/` on first save.
pub fn save(project_dir: &Path, canvas: &Canvas) -> std::io::Result<()> {
    let path = canvas_path(project_dir);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(canvas)?)?;
    std::fs::rename(&tmp, &path) // atomic on the same filesystem
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn save_then_load_roundtrips() {
        let dir = std::env::temp_dir().join(format!("identra-canvas-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);

        assert_eq!(load(&dir), Canvas::default()); // missing file -> blank board

        let canvas = Canvas {
            nodes: vec![Node {
                id: "n1".into(),
                kind: "codex".into(),
                x: 12.0,
                y: 34.0,
                width: 480.0,
                height: 320.0,
                title: "codex".into(),
                cwd: Some("/tmp".into()),
            }],
            edges: vec![Edge {
                id: "e1".into(),
                source: "n1".into(),
                target: "n2".into(),
            }],
            viewport: Viewport {
                x: -100.0,
                y: 50.0,
                zoom: 1.5,
            },
            title: "Auth refactor".into(),
        };
        save(&dir, &canvas).unwrap();
        assert_eq!(load(&dir), canvas);

        // A canvas.json written before `title` existed still loads, with the default name.
        std::fs::write(canvas_path(&dir), r#"{"nodes":[],"edges":[]}"#).unwrap();
        assert_eq!(load(&dir).title, "untitled-workspace");

        std::fs::remove_dir_all(&dir).unwrap();
    }
}
