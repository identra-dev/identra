//! identra-core, the engine. No UI, no async runtime lock-in.
//!
//! The pieces, each usable on its own:
//! - [`terminal`]: spawn a CLI in a real PTY, stream its output, replay it after a reload.
//! - [`canvas`]: load/save the node layout to `.identra/canvas.json`.
//! - [`agents`]: find which agent CLIs are on PATH.
//! - [`session`]: which conversation an agent is having, so it survives a restart.
//! - [`workspace`]: the folder a canvas and its agents live in.
//! - [`worktree`]: an isolated checkout, so two agents can edit the same repo at once.
//! - [`text`]: read PTY bytes as text, which both the terminal and the bus need.
//! - [`wallpaper`]: the shared image library the canvas backgrounds draw from.
//! - [`settings`]: what is true of this machine, as one small file.
//! - [`devserver`]: which command runs a project's dev server.
//! - [`fileview`]: read a workspace file for the viewer node, and only a workspace file.
//! - [`files`]: list and search the workspace for the Files panel.

pub mod agents;
pub mod canvas;
pub mod devserver;
pub mod files;
pub mod fileview;
pub mod session;
pub mod settings;
pub mod terminal;
pub mod text;
pub mod wallpaper;
pub mod workspace;
pub mod worktree;

pub use agents::{detect, AgentInfo};
pub use canvas::{Canvas, Node, Viewport};
pub use session::Session;
pub use terminal::{Error, Output, TerminalManager};
pub use text::{strip_ansi, tail};
pub use workspace::WorkspaceMeta;
pub use worktree::Isolated;
