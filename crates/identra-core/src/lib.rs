//! identra-core — the engine. No UI, no async runtime lock-in.
//!
//! Three pieces, each usable on its own:
//! - [`terminal`]: spawn a CLI in a real PTY, stream its output, replay it after a reload.
//! - [`canvas`]: load/save the node layout to `.identra/canvas.json`.
//! - [`agents`]: find which agent CLIs are on PATH.

pub mod agents;
pub mod canvas;
pub mod terminal;

pub use agents::{detect, AgentInfo};
pub use canvas::{Canvas, Node, Viewport};
pub use terminal::{Error, Output, TerminalManager};
