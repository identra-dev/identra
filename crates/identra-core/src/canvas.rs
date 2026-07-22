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
    /// The user has closed this node to changes made by agents.
    ///
    /// It stops agents wiring anything to it, which is a stronger guarantee than it first sounds:
    /// an edge is the bus authorization, so refusing new edges is what keeps this node's transcript
    /// out of reach of an agent that would otherwise wire itself in and read it. The user's own
    /// hands are not restricted, because this is their canvas and the lock is about what happens
    /// while they are not looking.
    #[serde(default)]
    pub locked: bool,
}

/// A wire between two nodes. The edge is also the authorization for the context bus: two nodes
/// share context only when an edge joins them.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Edge {
    pub id: String,
    pub source: String,
    pub target: String,
}

/// The canvas background, chosen per workspace. A lightweight reference, never image bytes: a
/// built-in is an id the frontend draws for itself, a color is its hex, an image is a path into
/// the shared wallpaper library. Keeping bytes out of the canvas is what keeps canvas.json a
/// layout file a person can read and diff.
///
/// Tagged `{"kind": ..., "value": ...}` on the wire because the frontend has to switch on the
/// kind, and a discriminated union is the one shape TypeScript narrows without ceremony.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(tag = "kind", content = "value", rename_all = "lowercase")]
pub enum Wallpaper {
    /// One of the built-in backgrounds, by id. An id this build does not know renders as the
    /// default, so a canvas from a newer Identra still loads here.
    Yaru(String),
    /// A flat color, as a hex string.
    Color(String),
    /// A user image, by absolute path into the wallpaper library. If the file is gone (removed
    /// from the library, or the canvas came from another machine) the frontend falls back to the
    /// plain background rather than erroring: a missing decoration is not a broken board.
    Image(String),
}

impl Default for Wallpaper {
    fn default() -> Self {
        Wallpaper::Yaru("yaru-default".into())
    }
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
    /// The background this workspace wears. Defaulted so every canvas.json written before the
    /// field existed loads as the plain board it always was.
    #[serde(default)]
    pub wallpaper: Wallpaper,
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
            wallpaper: Wallpaper::default(),
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

/// The tag on an exported file. A canvas on disk in a workspace is unwrapped, but a file the user
/// has moved somewhere else has lost all its context, so the file has to say what it is.
const EXPORT_FORMAT: &str = "identra-canvas";
/// Bumped only when an older Identra could not make sense of a newer file. Adding a field with a
/// serde default is not that: `locked` and `seat` both arrived without a bump, because a build that
/// predates them reads such a file correctly and just ignores what it does not know.
const EXPORT_VERSION: u32 = 1;

/// A canvas as a standalone file: the board, wrapped in enough to identify it.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Export {
    pub format: String,
    pub version: u32,
    pub canvas: Canvas,
}

/// Wrap the canvas for export, as pretty JSON with a trailing newline.
///
/// Indented because an exported canvas is a file a person may open, diff, or commit next to their
/// project, and a single line of JSON is none of those things.
pub fn export(canvas: &Canvas) -> String {
    let wrapped = Export {
        format: EXPORT_FORMAT.into(),
        version: EXPORT_VERSION,
        canvas: canvas.clone(),
    };
    let mut text = serde_json::to_string_pretty(&wrapped)
        // A Canvas is plain data with no map keys that could fail to serialize, so this cannot
        // happen. Falling back to the compact form rather than unwrapping keeps the promise that
        // export never loses the user's board.
        .unwrap_or_else(|_| serde_json::to_string(&wrapped).unwrap_or_default());
    text.push('\n');
    text
}

/// What went wrong reading a file the user chose to import.
///
/// Named cases rather than one string, because the UI says something different for each: the wrong
/// file entirely is a mistake to correct, and a newer version is a build to upgrade.
#[derive(Debug, PartialEq)]
pub enum ImportError {
    NotJson(String),
    NotACanvas,
    TooNew(u32),
}

impl std::fmt::Display for ImportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ImportError::NotJson(why) => write!(f, "that file is not valid JSON: {why}"),
            ImportError::NotACanvas => {
                write!(f, "that file is not an Identra canvas export")
            }
            ImportError::TooNew(v) => write!(
                f,
                "that canvas was exported by a newer Identra (format version {v}), so this build \
                 cannot read it. Update Identra and try again."
            ),
        }
    }
}

/// Read an exported canvas back.
///
/// I check the tag before trusting the contents. Serde would happily accept any JSON object as a
/// Canvas, because every field has a default, so a text file or somebody's `package.json` would
/// import as a blank board and silently replace the user's real one. The tag is what makes the
/// difference between importing and destroying.
pub fn import(text: &str) -> Result<Canvas, ImportError> {
    let value: serde_json::Value =
        serde_json::from_str(text).map_err(|e| ImportError::NotJson(e.to_string()))?;
    if value.get("format").and_then(|f| f.as_str()) != Some(EXPORT_FORMAT) {
        return Err(ImportError::NotACanvas);
    }
    let version = value.get("version").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
    if version > EXPORT_VERSION {
        return Err(ImportError::TooNew(version));
    }
    let wrapped: Export = serde_json::from_value(value).map_err(|_| ImportError::NotACanvas)?;
    Ok(wrapped.canvas)
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
                locked: true,
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
            wallpaper: Wallpaper::Image("/home/me/.local/share/identra/wallpapers/a.png".into()),
        };
        save(&dir, &canvas).unwrap();
        assert_eq!(load(&dir), canvas);

        // A canvas.json written before `title` existed still loads, with the default name. The same
        // lines cover `seat` and `wallpaper`, which arrived later still: every canvas on disk today
        // predates them, so they have to read as "no seat, plain board" rather than refusing to
        // load.
        std::fs::write(canvas_path(&dir), r#"{"nodes":[],"edges":[]}"#).unwrap();
        assert_eq!(load(&dir).title, "untitled-workspace");
        assert_eq!(load(&dir).seat, None);
        assert_eq!(load(&dir).wallpaper, Wallpaper::default());

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn an_exported_canvas_comes_back_as_itself() {
        let canvas = Canvas {
            nodes: vec![Node {
                id: "n1".into(),
                kind: "gemini".into(),
                x: 1.0,
                y: 2.0,
                width: 480.0,
                height: 320.0,
                title: "Router".into(),
                cwd: None,
                locked: true,
            }],
            edges: Vec::new(),
            viewport: Viewport::default(),
            title: "Auth refactor".into(),
            seat: Some("n1".into()),
            wallpaper: Wallpaper::Color("#16161d".into()),
        };
        assert_eq!(import(&export(&canvas)), Ok(canvas));
    }

    /// Import replaces the board the user is looking at, so what it refuses matters more than what
    /// it accepts. Every field on Canvas has a serde default, which means any JSON object at all
    /// would deserialize as a blank canvas: without the format tag, importing the wrong file would
    /// quietly wipe the user's real one instead of failing.
    #[test]
    fn import_refuses_anything_that_is_not_ours() {
        assert!(import("not json at all").is_err());
        // A perfectly good JSON object, and the exact shape that would otherwise import as a blank
        // board and destroy the canvas it replaced.
        assert_eq!(
            import(r#"{"name":"some-package","version":"1.0.0"}"#),
            Err(ImportError::NotACanvas)
        );
        // Right shape, no tag. Still refused: the tag is the only thing that says this file was
        // meant for us.
        assert_eq!(
            import(r#"{"canvas":{"nodes":[],"edges":[]}}"#),
            Err(ImportError::NotACanvas)
        );
        // A file from a future build, which is a reason to update rather than a corrupt file.
        assert_eq!(
            import(r#"{"format":"identra-canvas","version":99,"canvas":{}}"#),
            Err(ImportError::TooNew(99))
        );
    }

    /// A canvas exported before a field existed has to keep importing, or every export anyone has
    /// ever taken becomes rubbish the next time a field is added.
    #[test]
    fn an_older_export_still_imports() {
        let old = r#"{"format":"identra-canvas","version":1,"canvas":{
            "nodes":[{"id":"n1","kind":"codex","x":0,"y":0}],"edges":[]}}"#;
        let canvas = import(old).expect("an older export is still ours");
        assert_eq!(canvas.nodes.len(), 1);
        assert_eq!(canvas.nodes[0].width, 480.0, "defaults fill the gaps");
        assert!(!canvas.nodes[0].locked);
        assert_eq!(canvas.seat, None);
        assert_eq!(canvas.wallpaper, Wallpaper::default());
    }
}
