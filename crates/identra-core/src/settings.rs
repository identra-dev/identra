//! App-level settings. One small file at `~/.config/identra/settings.json`, read where a
//! decision needs it and written by the settings panel. Per-workspace choices (title, wallpaper,
//! the seat) live on the canvas instead; this file is only for what is true of the machine.

use serde::{Deserialize, Serialize};
use std::io;
use std::path::{Path, PathBuf};

/// How much an agent may do before it stops and asks a human.
///
/// A canvas is several agents working at once, and every one of them stopping on its own approval
/// prompt turns the canvas into a queue of things waiting for the same person. That is the failure
/// this setting exists to name: the prompts are individually reasonable and collectively the reason
/// nobody leaves the app running.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum Autonomy {
    /// Each agent CLI's own default. Every edit and every command waits for a click.
    Ask,
    /// Every prompt off, in every CLI that has a switch for it: approvals, sandboxing, directory
    /// trust, and MCP server consent. This is the default, and it is the strongest thing each CLI
    /// offers, so it is worth being plain about what it costs.
    ///
    /// The middle setting this replaced was the sandboxed one: free inside the workspace, walled
    /// out of the rest of the machine. It was the better posture and it did not survive contact
    /// with the product, because the prompts it left standing were the ones that actually blocked
    /// people. Codex stopped on a directory-trust dialog before it would read a word; the others
    /// stopped on shell commands and on consenting to the bus. On a canvas of parallel agents each
    /// of those is a separate dialog on a separate node, and the orchestrator sits behind one of
    /// them holding the instruction the user typed.
    ///
    /// So the trade is stated rather than hidden: an agent running under this can reach anything
    /// the user can. Run it on work you have committed, and [`Ask`](Autonomy::Ask) is one click
    /// away in Settings for when you would rather be asked.
    #[default]
    #[serde(alias = "workspace")]
    Bypass,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Settings {
    /// Whether recall may use the local embedding model. On matches by meaning but fetches the
    /// model (about 130MB) the first time; off matches by words and never touches the network.
    /// This is the one thing in Identra that reaches the network, which is why it is the first
    /// setting the panel got.
    #[serde(default = "default_true")]
    pub embeddings: bool,
    /// What an agent may do without asking. See [`Autonomy`].
    ///
    /// `#[serde(default)]` matters on an upgrade: a settings file written before this field existed
    /// has to keep loading, and it lands on `Workspace` with everything else it had intact.
    #[serde(default)]
    pub autonomy: Autonomy,
}

fn default_true() -> bool {
    true
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            embeddings: true,
            autonomy: Autonomy::default(),
        }
    }
}

/// `~/.config/identra/settings.json`. XDG config rather than the data dir, because this is
/// configuration the user could reasonably edit by hand, not state the app accumulates.
pub fn path() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    Some(
        Path::new(&home)
            .join(".config")
            .join("identra")
            .join("settings.json"),
    )
}

/// Read settings from a file, or the defaults when there is nothing readable. A file that will
/// not parse also reads as defaults rather than being moved aside the way a canvas is: this is
/// one boolean a user can re-toggle in a click, not a week of board layout, and the next save
/// overwrites it.
pub fn load_from(file: &Path) -> Settings {
    std::fs::read_to_string(file)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

/// Write settings atomically, same temp and rename dance as the canvas, because a truncated
/// settings file on a crash would otherwise cost the user their choices for no reason.
pub fn save_to(file: &Path, settings: &Settings) -> io::Result<()> {
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = file.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(settings)?)?;
    std::fs::rename(&tmp, file)
}

/// The settings of this machine. No home directory means the defaults, which is the only honest
/// answer a read can give.
pub fn load() -> Settings {
    path().as_deref().map(load_from).unwrap_or_default()
}

pub fn save(settings: &Settings) -> io::Result<()> {
    let Some(file) = path() else {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "cannot find a home directory for settings",
        ));
    };
    save_to(&file, settings)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Through the path-taking pair rather than a scratch HOME, because tests run in one process
    // and mutating HOME would race every other test that reads it.
    #[test]
    fn settings_survive_the_round_trip_and_default_sanely() {
        let dir = std::env::temp_dir().join(format!("identra-set-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let file = dir.join("settings.json");

        assert_eq!(load_from(&file), Settings::default(), "no file, defaults");
        assert!(
            load_from(&file).embeddings,
            "recall by meaning is the shipped default"
        );

        assert_eq!(
            load_from(&file).autonomy,
            Autonomy::Bypass,
            "agents start with every prompt off, by default"
        );

        save_to(
            &file,
            &Settings {
                embeddings: false,
                autonomy: Autonomy::Ask,
            },
        )
        .unwrap();
        assert!(
            !load_from(&file).embeddings,
            "the toggle comes back as written"
        );
        assert_eq!(load_from(&file).autonomy, Autonomy::Ask);

        // A settings file written before `autonomy` existed still loads, keeps what it did say, and
        // lands on the default for what it did not. Without this an upgrade would silently reset
        // someone's embeddings choice, which is the kind of thing that reads as data loss.
        std::fs::write(&file, r#"{"embeddings":false}"#).unwrap();
        let upgraded = load_from(&file);
        assert!(!upgraded.embeddings, "the old field survives the upgrade");
        assert_eq!(upgraded.autonomy, Autonomy::Bypass);

        // The name this setting used to have. A file written by an older Identra still says
        // "workspace", and it has to keep loading rather than falling back to the default and
        // quietly discarding the embeddings choice sitting next to it.
        std::fs::write(&file, r#"{"embeddings":false,"autonomy":"workspace"}"#).unwrap();
        let renamed = load_from(&file);
        assert_eq!(renamed.autonomy, Autonomy::Bypass);
        assert!(!renamed.embeddings, "its neighbour survives the rename too");

        // And "ask" still means ask. Whatever else moves, the way out has to keep working.
        std::fs::write(&file, r#"{"autonomy":"ask"}"#).unwrap();
        assert_eq!(load_from(&file).autonomy, Autonomy::Ask);

        // A file someone hand-edited into garbage is defaults, not a crash and not a refusal to
        // start. One re-toggle rewrites it.
        std::fs::write(&file, "{ not json").unwrap();
        assert_eq!(load_from(&file), Settings::default());

        std::fs::remove_dir_all(&dir).unwrap();
    }
}
