//! identra-core, the engine. No UI, no async runtime lock-in.
//!
//! Seven pieces, each usable on its own:
//! - [`terminal`]: spawn a CLI in a real PTY, stream its output, replay it after a reload.
//! - [`canvas`]: load/save the node layout to `.identra/canvas.json`.
//! - [`agents`]: find which agent CLIs are on PATH.
//! - [`session`]: which conversation an agent is having, so it survives a restart.
//! - [`workspace`]: the folder a canvas and its agents live in.
//! - [`worktree`]: an isolated checkout, so two agents can edit the same repo at once.
//! - [`text`]: read PTY bytes as text, which both the terminal and the bus need.

pub mod agents;
pub mod canvas;
pub mod session;
pub mod terminal;
pub mod text;
pub mod workspace;
pub mod worktree;

pub use agents::{detect, AgentInfo};
pub use canvas::{Canvas, Node, Viewport};
pub use session::Session;
pub use terminal::{Error, Output, TerminalManager};
pub use text::{strip_ansi, tail};
pub use workspace::WorkspaceMeta;
pub use worktree::Isolated;
