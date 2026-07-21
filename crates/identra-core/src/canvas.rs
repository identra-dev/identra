//! The canvas store. The layout is the source of truth and lives in the user's project at
//! `.identra/canvas.json`. Writes are atomic (temp file + rename); debouncing is the caller's
//! job: the UI already knows when a drag ends, so it throttles there instead of here.

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
    /// The node holding the orchestrator seat, if the user has opened the command center here.
    ///
    /// One id on the canvas rather than a flag on a node, because "there is at most one seat" is
    /// then a fact about the shape of the data instead of a rule some future code has to remember.
    /// Reassigning is one write, and a seat pointing at a node the user has since closed reads as
    /// no seat, which is what it is.
    ///
    /// The seat is a role, not a capability: it holds nothing the bus does not already offer every
    /// node. It is here so the canvas can remember which node the command bar talks to.
    #[serde(default)]
    pub seat: Option<String>,
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
            // No seat until the user opens the command center. A blank board has nothing to
            // orchestrate, and picking an agent for them before they ask is a node they did not
            // want and are paying for.
            seat: None,
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

/// Read the saved canvas, or an empty one if there is nothing valid on disk. Never fails: a missing
/// or unreadable file means a blank board, not a crash.
///
/// A file that exists but will not parse is moved to `canvas.json.bak` first, and this is the whole
/// point of the function. Returning a blank board leaves the app one debounced save away from
/// renaming a fresh canvas over the only copy of the user's board, so the bad parse would eat the
/// work rather than report it. Moving it aside costs a rename and means the layout is still on disk
/// for someone to fish out. It is also why the corrupt file goes away rather than being copied: if
/// it stayed, every later load would warn about a file nothing will ever read again.
pub fn load(project_dir: &Path) -> Canvas {
    let path = canvas_path(project_dir);
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        // No file is the first run in a workspace, which is not worth a word.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Canvas::default(),
        Err(e) => {
            eprintln!(
                "identra: cannot read {}, starting blank: {e}",
                path.display()
            );
            return Canvas::default();
        }
    };
    match serde_json::from_str(&text) {
        Ok(canvas) => canvas,
        Err(e) => {
            let bak = path.with_extension("json.bak");
            match std::fs::rename(&path, &bak) {
                Ok(()) => eprintln!(
                    "identra: {} did not parse ({e}), kept it as {} and started blank",
                    path.display(),
                    bak.display()
                ),
                // I could not move it, so I must not let the caller save over it.
                Err(move_err) => eprintln!(
                    "identra: {} did not parse ({e}) and could not be kept ({move_err}). \
                     Copy it somewhere before you touch this workspace again",
                    path.display()
                ),
            }
            Canvas::default()
        }
    }
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

    /// The board someone spent a week arranging must survive a file that will not parse. Without
    /// the bak, load returns blank and the next debounced save renames straight over the only copy
    /// they had, so a truncated write during a crash quietly costs them the work. It has to still be
    /// on disk afterwards.
    #[test]
    fn a_canvas_that_will_not_parse_is_kept_not_eaten() {
        let dir = std::env::temp_dir().join(format!("identra-canvas-bad-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join(".identra")).unwrap();
        let path = canvas_path(&dir);
        // Half a write, which is what a crash mid-save leaves behind.
        std::fs::write(&path, r#"{"nodes":[{"id":"n1","kind":"co"#).unwrap();

        assert_eq!(load(&dir), Canvas::default(), "a bad parse starts blank");

        let bak = path.with_extension("json.bak");
        assert!(bak.exists(), "the unparseable canvas is kept as .bak");
        assert!(
            std::fs::read_to_string(&bak).unwrap().contains("\"n1\""),
            "and it is kept whole, so the layout can be recovered from it"
        );

        // The save that follows a blank load must not be able to reach it.
        save(&dir, &Canvas::default()).unwrap();
        assert!(
            bak.exists(),
            "saving over a blank board leaves the .bak alone"
        );

        std::fs::remove_dir_all(&dir).unwrap();
    }

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
            seat: Some("n1".into()),
        };
        save(&dir, &canvas).unwrap();
        assert_eq!(load(&dir), canvas);

        // A canvas.json written before `title` existed still loads, with the default name. The same
        // line covers `seat`, which arrived later still: every canvas on disk today predates it, so
        // it has to read as "no seat" rather than refusing to load.
        std::fs::write(canvas_path(&dir), r#"{"nodes":[],"edges":[]}"#).unwrap();
        assert_eq!(load(&dir).title, "untitled-workspace");
        assert_eq!(load(&dir).seat, None);

        std::fs::remove_dir_all(&dir).unwrap();
    }
}
