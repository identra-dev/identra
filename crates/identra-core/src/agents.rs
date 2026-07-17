//! Which agent CLIs are installed, and whether each one looks signed in. A node offers to run
//! one of these; if it's missing, the node says so instead of pretending to work.
//!
//! Detection is existence and size only. We stat a creds file or check an env var is set, and we
//! never open or copy a token. That is the whole "reuse what you already have, store nothing"
//! guarantee: the CLI Identra launches inherits the same env, so what we probe and what it reads
//! are the same thing.

use serde::Serialize;
use std::path::Path;

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct AgentInfo {
    pub id: String,
    pub name: String,
    pub path: String,
    /// Found on PATH.
    pub available: bool,
    /// Available AND a creds file or provider env var is present. A heuristic: it proves setup
    /// exists, not that the session is live or paid.
    pub logged_in: bool,
    /// The binary a node spawns for this agent.
    pub cmd: String,
    /// Interactive launch args (most take none; a few, like goose, need a subcommand).
    pub args: Vec<String>,
}

/// One agent the dock can offer. A row of facts, no code branch: the per-CLI quirks (extra
/// launch arg, where the login lives) live here as data so growing the list never touches logic.
struct Adapter {
    id: &'static str,
    name: &'static str,
    /// Names to look for on PATH; first hit wins.
    bins: &'static [&'static str],
    /// Interactive launch args.
    args: &'static [&'static str],
    /// HOME-relative creds/config paths that mean "set up" if they exist and are non-empty.
    auth_paths: &'static [&'static str],
    /// Env vars that mean "set up" if set and non-empty.
    auth_envs: &'static [&'static str],
    /// HOME-relative config dirs that mean "set up" on macOS if they hold anything.
    ///
    /// On macOS several of these CLIs keep the actual credential in the Keychain, so there is no
    /// creds file to find and the file check says "not signed in" to someone who is. A configured
    /// directory is the only on-disk evidence left. It is a weaker signal than a creds file, and it
    /// is the honest one available without reading a Keychain we have no business opening.
    mac_config_dirs: &'static [&'static str],
}

/// Agents Identra knows how to spawn. The installed four front the dock; the rest render as
/// "missing" with the same row shape, so they go live the moment `which` finds them on a box.
const KNOWN: &[Adapter] = &[
    Adapter {
        id: "codex",
        name: "Codex",
        bins: &["codex"],
        args: &[],
        auth_paths: &[".codex/auth.json"],
        auth_envs: &["OPENAI_API_KEY", "CODEX_API_KEY"],
        mac_config_dirs: &[".codex"],
    },
    Adapter {
        id: "claude",
        name: "Claude Code",
        bins: &["claude"],
        args: &[],
        // Prefer .credentials.json: ~/.claude.json is non-empty even when not cleanly signed in.
        auth_paths: &[".claude/.credentials.json"],
        auth_envs: &["ANTHROPIC_API_KEY"],
        mac_config_dirs: &[".claude"],
    },
    Adapter {
        id: "gemini",
        name: "Gemini",
        bins: &["gemini"],
        args: &[],
        auth_paths: &[".gemini/oauth_creds.json"],
        auth_envs: &["GEMINI_API_KEY", "GOOGLE_API_KEY"],
        mac_config_dirs: &[],
    },
    Adapter {
        id: "opencode",
        name: "OpenCode",
        bins: &["opencode"],
        args: &[],
        auth_paths: &[".local/share/opencode/auth.json"],
        auth_envs: &["OPENAI_API_KEY", "ANTHROPIC_API_KEY"],
        mac_config_dirs: &[],
    },
    // Not installed on this box; here so the dock shows them as missing and they light up on
    // any machine that has them. No login concept for aider: it is pure API key.
    Adapter {
        id: "aider",
        name: "Aider",
        bins: &["aider"],
        args: &[],
        auth_paths: &[],
        auth_envs: &["OPENAI_API_KEY", "ANTHROPIC_API_KEY"],
        mac_config_dirs: &[],
    },
    Adapter {
        id: "goose",
        name: "Goose",
        bins: &["goose"],
        args: &["session"],
        // Creds live in the OS keyring; a configured provider is the strongest on-disk signal.
        auth_paths: &[".config/goose/config.yaml"],
        auth_envs: &["GOOSE_PROVIDER"],
        mac_config_dirs: &[".config/goose"],
    },
    Adapter {
        id: "amp",
        name: "Amp",
        bins: &["amp"],
        args: &[],
        auth_paths: &[".config/amp/settings.json"],
        auth_envs: &["AMP_API_KEY"],
        mac_config_dirs: &[],
    },
    Adapter {
        id: "cursor-agent",
        name: "Cursor",
        bins: &["cursor-agent"],
        args: &[],
        auth_paths: &[".cursor/cli-config.json"],
        auth_envs: &[],
        mac_config_dirs: &[],
    },
];

pub fn detect() -> Vec<AgentInfo> {
    KNOWN.iter().map(row_to_info).collect()
}

fn row_to_info(a: &Adapter) -> AgentInfo {
    let found = a
        .bins
        .iter()
        .find_map(|b| which(b).map(|p| ((*b).to_string(), p)));
    let available = found.is_some();
    let (cmd, path) = match found {
        Some((bin, path)) => (bin, path),
        None => (a.bins[0].to_string(), String::new()),
    };
    AgentInfo {
        id: a.id.into(),
        name: a.name.into(),
        path,
        available,
        logged_in: available && auth_present(a.auth_paths, a.auth_envs, a.mac_config_dirs),
        cmd,
        args: a.args.iter().map(|s| (*s).to_string()).collect(),
    }
}

/// True if any auth env is set and non-empty, or any HOME-relative auth path exists and is
/// non-empty, or (macOS only) one of the config dirs holds anything.
///
/// Stat only. I never open a creds file, which is the whole "reuse what you already have, store
/// nothing" guarantee, and it is also why the macOS branch reads a directory listing rather than
/// `~/.claude.json`: the thing that would prove a login there is inside the file.
fn auth_present(paths: &[&str], envs: &[&str], mac_config_dirs: &[&str]) -> bool {
    if envs
        .iter()
        .any(|e| std::env::var(e).map(|v| !v.is_empty()).unwrap_or(false))
    {
        return true;
    }
    let Some(home) = std::env::var_os("HOME") else {
        return false;
    };
    let home = Path::new(&home);
    if paths.iter().any(|rel| {
        std::fs::metadata(home.join(rel))
            .map(|m| m.len() > 0)
            .unwrap_or(false)
    }) {
        return true;
    }
    // macOS only. On Linux these same CLIs write a creds file, so the check above already answered,
    // and widening it there would turn "I ran this once" into "I am signed in" for no gain.
    #[cfg(target_os = "macos")]
    {
        mac_config_dirs
            .iter()
            .any(|rel| non_empty_dir(&home.join(rel)))
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = mac_config_dirs;
        false
    }
}

/// A directory that exists and holds at least one entry.
#[cfg(target_os = "macos")]
fn non_empty_dir(path: &Path) -> bool {
    std::fs::read_dir(path)
        .map(|mut entries| entries.next().is_some())
        .unwrap_or(false)
}

/// First entry on PATH that's a file named `bin`. Doesn't check the exec bit,
/// good enough to tell "installed" from "missing"; tighten if a non-exec collision ever bites.
fn which(bin: &str) -> Option<String> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(bin))
        .find(|p| p.is_file())
        .map(|p| p.display().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_a_binary_that_exists() {
        // `sh` is on PATH on every unix box the dev env runs on.
        assert!(which("sh").is_some());
        assert!(which("this-binary-does-not-exist-identra").is_none());
        assert_eq!(detect().len(), KNOWN.len());
    }

    #[test]
    fn auth_present_reads_env_and_stats_files() {
        // A set, non-empty env var counts as "set up".
        std::env::set_var("IDENTRA_TEST_AUTH_ENV", "x");
        assert!(auth_present(&[], &["IDENTRA_TEST_AUTH_ENV"], &[]));
        std::env::set_var("IDENTRA_TEST_AUTH_ENV", "");
        assert!(!auth_present(&[], &["IDENTRA_TEST_AUTH_ENV"], &[]));
        std::env::remove_var("IDENTRA_TEST_AUTH_ENV");
        // A path that cannot exist under HOME is not "set up".
        assert!(!auth_present(&[".identra-nope/does-not-exist"], &[], &[]));
    }
}
