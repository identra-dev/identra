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
    /// Free inside the workspace, sandboxed out of everything else. The folder was opened
    /// deliberately and its contents are under version control; the rest of the machine is not.
    /// This is the default because it is the posture the product is actually for.
    #[default]
    Workspace,
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
            Autonomy::Workspace,
            "agents work inside the folder without asking, by default"
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
        assert_eq!(upgraded.autonomy, Autonomy::Workspace);

        // A file someone hand-edited into garbage is defaults, not a crash and not a refusal to
        // start. One re-toggle rewrites it.
        std::fs::write(&file, "{ not json").unwrap();
        assert_eq!(load_from(&file), Settings::default());

        std::fs::remove_dir_all(&dir).unwrap();
    }
}
