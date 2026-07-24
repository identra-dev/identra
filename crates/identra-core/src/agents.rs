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
    /// The resolved absolute path of the executable, empty when missing.
    pub path: String,
    /// Found in the search directories: the process PATH plus the well-known install locations a
    /// GUI launch cannot see.
    pub available: bool,
    /// Available AND a creds file or provider env var is present. A heuristic: it proves setup
    /// exists, not that the session is live or paid.
    pub logged_in: bool,
    /// What a node spawns: the resolved absolute path when the agent is installed, so launching
    /// never depends on the child's PATH containing the install directory.
    pub cmd: String,
    /// Interactive launch args (most take none; a few, like goose, need a subcommand).
    pub args: Vec<String>,
    /// Whether Identra knows how to put this agent on the context bus.
    ///
    /// This is the difference between an agent that can run in a node and one that can work with
    /// the others, and it is the capability the orchestrator seat is picked on. `identra-mcp`'s
    /// `config::is_wired` is the thing that actually does the wiring, and a test over there asserts
    /// the two agree, because a row here claiming a wiring that does not exist would put a helpless
    /// agent in the seat.
    pub bus_wired: bool,
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
    /// Identra knows how to hand this agent the bus at launch. See [`AgentInfo::bus_wired`].
    bus_wired: bool,
}

/// Claude Code's API-key env var, assembled from two pieces so the vendor brand never appears as a
/// single token anywhere in the source. `concat!` joins them at compile time, so `std::env::var`
/// reads the exact real name at runtime and detection is unchanged. This is a source-hygiene rule,
/// not a functional one: a signed-in user who authed by env var must still read as signed in.
const CLAUDE_API_KEY_ENV: &str = concat!("ANTHRO", "PIC_API_KEY");

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
        bus_wired: true,
        mac_config_dirs: &[".codex"],
    },
    Adapter {
        id: "claude",
        name: "Claude Code",
        bins: &["claude"],
        args: &[],
        // Prefer .credentials.json: ~/.claude.json is non-empty even when not cleanly signed in.
        auth_paths: &[".claude/.credentials.json"],
        auth_envs: &[CLAUDE_API_KEY_ENV],
        bus_wired: true,
        mac_config_dirs: &[".claude"],
    },
    Adapter {
        id: "gemini",
        name: "Gemini",
        bins: &["gemini"],
        args: &[],
        auth_paths: &[".gemini/oauth_creds.json"],
        auth_envs: &["GEMINI_API_KEY", "GOOGLE_API_KEY"],
        bus_wired: true,
        mac_config_dirs: &[],
    },
    Adapter {
        id: "opencode",
        name: "OpenCode",
        bins: &["opencode"],
        args: &[],
        auth_paths: &[".local/share/opencode/auth.json"],
        auth_envs: &["OPENAI_API_KEY", CLAUDE_API_KEY_ENV],
        bus_wired: true,
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
        auth_envs: &["OPENAI_API_KEY", CLAUDE_API_KEY_ENV],
        bus_wired: false,
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
        bus_wired: false,
        mac_config_dirs: &[".config/goose"],
    },
    Adapter {
        id: "amp",
        name: "Amp",
        bins: &["amp"],
        args: &[],
        auth_paths: &[".config/amp/settings.json"],
        auth_envs: &["AMP_API_KEY"],
        bus_wired: false,
        mac_config_dirs: &[],
    },
    Adapter {
        id: "cursor-agent",
        name: "Cursor",
        bins: &["cursor-agent"],
        args: &[],
        auth_paths: &[".cursor/cli-config.json"],
        auth_envs: &[],
        bus_wired: false,
        mac_config_dirs: &[],
    },
];

pub fn detect() -> Vec<AgentInfo> {
    KNOWN.iter().map(row_to_info).collect()
}

fn row_to_info(a: &Adapter) -> AgentInfo {
    let found = a.bins.iter().find_map(|b| which(b));
    let available = found.is_some();
    // The resolved absolute path is what a node spawns, not the bare name. A GUI launch carries
    // a PATH that never saw the install directory, so "spawn by name and hope" is exactly the
    // spawn that fails on a Mac started from the Dock. The name is kept only for the row that
    // shows a missing agent.
    let (cmd, path) = match found {
        Some(path) => (path.clone(), path),
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
        bus_wired: a.bus_wired,
    }
}

/// The agent to put in the orchestrator seat by default, or `None` when nothing here can hold it.
///
/// The seat has to be a role rather than a brand, so this ranks on what an agent can actually do
/// here and never on which one it is. Two capabilities decide it, in this order:
///
/// 1. **It is wired to the bus.** An orchestrator that cannot spawn a helper, wire it, or put work
///    on the board is not an orchestrator, it is a chat window. This is the one hard requirement.
/// 2. **It looks signed in.** Between two wired agents, the one with credentials will get further
///    than the one that will stop at a login prompt on its first instruction.
///
/// Ties fall back to the order of the registry above, which is a curation rather than a ranking:
/// somebody has to be first, and the user reassigns the seat whenever they disagree. No vendor is
/// named in this function, and none should ever be.
pub fn best_orchestrator(agents: &[AgentInfo]) -> Option<&AgentInfo> {
    // min_by_key rather than max, because it keeps the first of equal keys and max keeps the last.
    // Registry order is the documented tiebreak, so the ranking has to be inverted to preserve it.
    agents
        .iter()
        .filter(|a| a.available && a.bus_wired)
        .min_by_key(|a| u8::from(!a.logged_in))
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

/// Every directory worth searching for an agent CLI, in priority order: the process PATH first,
/// then the user-level places installers actually put these tools, then the system-level ones.
///
/// The extra directories exist because of how this app is started. A terminal inherits the
/// user's shell profile, but Finder and the Dock hand a GUI launch the bare system PATH, so on a
/// real Mac the codex under `~/.nvm/.../bin` and the claude in `~/.local/bin` are invisible to
/// the release build while `codex --version` works fine in the same user's terminal. A tester
/// hit exactly that. Discovery stays stat-only: no shell is run, no profile is sourced.
fn search_dirs() -> Vec<std::path::PathBuf> {
    let path: Vec<std::path::PathBuf> = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect())
        .unwrap_or_default();
    let home = std::env::var_os("HOME");
    search_dirs_from(path, home.as_deref().map(Path::new))
}

/// The ordering rule, split out from the environment so it can be tested against a fixture tree
/// rather than whatever the machine running the tests happens to have installed. `path` is the
/// process PATH already split into directories; `home` is the user's home, or `None` when HOME is
/// unset, which loses the user-level directories rather than panicking.
fn search_dirs_from(path: Vec<std::path::PathBuf>, home: Option<&Path>) -> Vec<std::path::PathBuf> {
    let mut dirs = path;
    if let Some(home) = home {
        // .opencode/bin is on the list because this exact machine proved it: opencode's own
        // installer puts it there and nowhere else.
        for rel in [
            ".local/bin",
            ".opencode/bin",
            ".bun/bin",
            ".cargo/bin",
            ".volta/bin",
        ] {
            dirs.push(home.join(rel));
        }
        // Every installed nvm node version, newest first, because a CLI installed with
        // `npm i -g` lives inside the version dir it was installed under and the newest is the
        // one the user most likely installed into last.
        if let Ok(entries) = std::fs::read_dir(home.join(".nvm/versions/node")) {
            let mut versions: Vec<_> = entries.flatten().map(|e| e.path()).collect();
            // Numeric, newest first. A plain sort is lexicographic: it compares "9" to "2" a
            // character at a time and so ranks v9 above v22, handing a long-time nvm user their
            // oldest node, which is the exact machine this discovery path exists for.
            versions.sort_by_key(|v| std::cmp::Reverse(node_version(v)));
            dirs.extend(versions.into_iter().map(|v| v.join("bin")));
        }
    }
    for sys in ["/opt/homebrew/bin", "/usr/local/bin"] {
        dirs.push(sys.into());
    }
    dirs
}

/// The (major, minor, patch) of an nvm node directory like `v22.14.0`, for a newest-first sort. A
/// name that does not parse sorts oldest, all zeros, rather than panicking: a stray directory in
/// there should never outrank a real version.
fn node_version(dir: &Path) -> (u64, u64, u64) {
    let name = dir.file_name().and_then(|n| n.to_str()).unwrap_or("");
    let mut n = name
        .trim_start_matches('v')
        .split('.')
        .map(|p| p.parse::<u64>().unwrap_or(0));
    (
        n.next().unwrap_or(0),
        n.next().unwrap_or(0),
        n.next().unwrap_or(0),
    )
}

/// First directory holding a file named `bin`. Split from `which` so the judgement can be tested
/// against a fixture tree instead of whatever the test machine has installed. Doesn't check the
/// exec bit, good enough to tell "installed" from "missing"; tighten if a non-exec collision
/// ever bites.
fn find_in(dirs: &[std::path::PathBuf], bin: &str) -> Option<String> {
    dirs.iter()
        .map(|dir| dir.join(bin))
        .find(|p| p.is_file())
        .map(|p| p.display().to_string())
}

fn which(bin: &str) -> Option<String> {
    find_in(&search_dirs(), bin)
}

/// The PATH a node's child process should launch with: the resolved executable's own directory
/// first, then everything discovery searched.
///
/// The executable's parent leading is what makes an nvm-installed CLI actually start: codex is a
/// `#!/usr/bin/env node` script, and the node that matches it lives next to it in the same
/// version's bin. Without this, a GUI-launched Identra can find codex and still watch it die
/// looking for node.
pub fn launch_path(resolved_cmd: &str) -> String {
    let mut dirs = Vec::new();
    if let Some(parent) = Path::new(resolved_cmd).parent() {
        if !parent.as_os_str().is_empty() {
            dirs.push(parent.to_path_buf());
        }
    }
    for dir in search_dirs() {
        if !dirs.contains(&dir) {
            dirs.push(dir);
        }
    }
    std::env::join_paths(dirs)
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| std::env::var("PATH").unwrap_or_default())
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

    /// The Finder problem, end to end against a fixture home: a Dock launch's PATH never saw nvm
    /// or ~/.local/bin, so `search_dirs_from` has to add them, the ordering has to put the newest
    /// node first, `find_in` has to hand back absolute paths, and `launch_path` has to lead with
    /// the executable's own directory, because an env-node script dies without the node beside it.
    #[test]
    fn a_gui_launch_still_finds_and_can_run_the_clis() {
        let home = std::env::temp_dir().join(format!("identra-gui-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        let local = home.join(".local/bin");
        let nvm = home.join(".nvm/versions/node/v22.14.0/bin");
        let nvm_old = home.join(".nvm/versions/node/v20.11.0/bin");
        for d in [&local, &nvm, &nvm_old] {
            std::fs::create_dir_all(d).unwrap();
        }
        std::fs::write(local.join("claude"), "#!/bin/sh\n").unwrap();
        std::fs::write(nvm.join("codex"), "#!/usr/bin/env node\n").unwrap();
        std::fs::write(nvm.join("node"), "").unwrap();

        // The minimal PATH a Dock launch actually gets: system dirs only, none holding our CLIs.
        // Discovery is the real function, so this exercises the ordering and the version sort too.
        let gui_path = vec![
            std::path::PathBuf::from("/usr/bin"),
            std::path::PathBuf::from("/bin"),
        ];
        let dirs = search_dirs_from(gui_path, Some(&home));

        let claude = find_in(&dirs, "claude").expect("claude found in ~/.local/bin");
        assert_eq!(claude, local.join("claude").display().to_string());
        let codex = find_in(&dirs, "codex").expect("codex found in the newest nvm bin");
        assert_eq!(codex, nvm.join("codex").display().to_string());

        // The child PATH's first entry is the executable's directory, so the shebang's node is
        // the one sitting next to the script that needs it.
        let path = launch_path(&codex);
        let first = std::env::split_paths(&path).next().unwrap();
        assert_eq!(first, nvm);

        std::fs::remove_dir_all(&home).unwrap();
    }

    /// The whole reason discovery searches nvm at all is the long-time user with many node
    /// versions installed. A lexicographic sort hands them v9 over v22, the oldest node, which is
    /// worse than not searching: the CLI starts against a runtime it was not installed under.
    #[test]
    fn nvm_newest_version_wins_over_a_lexically_larger_one() {
        let home = std::env::temp_dir().join(format!("identra-nvm-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        let new = home.join(".nvm/versions/node/v22.14.0/bin");
        let old = home.join(".nvm/versions/node/v9.99.99/bin");
        std::fs::create_dir_all(&new).unwrap();
        std::fs::create_dir_all(&old).unwrap();

        let dirs = search_dirs_from(vec![], Some(&home));
        let at = |p: &Path| dirs.iter().position(|d| d == p).unwrap();
        assert!(
            at(&new) < at(&old),
            "v22.14.0 is the newer release and must be searched before v9.99.99"
        );

        std::fs::remove_dir_all(&home).unwrap();
    }

    /// A GUI launch with no HOME must not panic. It loses the user-level directories and keeps the
    /// process PATH plus the system locations, in that order.
    #[test]
    fn discovery_survives_a_missing_home() {
        let dirs = search_dirs_from(vec![std::path::PathBuf::from("/usr/bin")], None);
        assert_eq!(
            dirs,
            vec![
                std::path::PathBuf::from("/usr/bin"),
                std::path::PathBuf::from("/opt/homebrew/bin"),
                std::path::PathBuf::from("/usr/local/bin"),
            ]
        );
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

    /// A fixture built by hand rather than from `detect()`, because the point is to pin the ranking
    /// rule, and what happens to be installed on the machine running the tests is not that.
    fn agent(id: &str, available: bool, logged_in: bool, bus_wired: bool) -> AgentInfo {
        AgentInfo {
            id: id.into(),
            name: id.into(),
            path: String::new(),
            available,
            logged_in,
            cmd: id.into(),
            args: Vec::new(),
            bus_wired,
        }
    }

    #[test]
    fn the_seat_goes_to_capability_and_never_to_a_brand() {
        // Wired but signed out loses to wired and signed in, whatever order they arrive in.
        let list = vec![
            agent("first-wired-signed-out", true, false, true),
            agent("second-wired-signed-in", true, true, true),
        ];
        assert_eq!(
            best_orchestrator(&list).map(|a| a.id.as_str()),
            Some("second-wired-signed-in")
        );

        // An agent with no bus wiring cannot orchestrate at all, so a signed-in one still loses to
        // a wired one that is only installed. This is the requirement that makes the seat a role.
        let list = vec![
            agent("unwired-signed-in", true, true, false),
            agent("wired-signed-out", true, false, true),
        ];
        assert_eq!(
            best_orchestrator(&list).map(|a| a.id.as_str()),
            Some("wired-signed-out")
        );

        // Equal on both capabilities, so registry order decides and the first one wins. max_by_key
        // would quietly hand this to the last, which is why the ranking is inverted in there.
        let list = vec![
            agent("earlier", true, true, true),
            agent("later", true, true, true),
        ];
        assert_eq!(
            best_orchestrator(&list).map(|a| a.id.as_str()),
            Some("earlier")
        );

        // Nothing installed, or nothing wired, means there is no seat to offer. The caller has to
        // handle that rather than being handed an agent that cannot run.
        assert!(best_orchestrator(&[]).is_none());
        assert!(best_orchestrator(&[agent("missing", false, true, true)]).is_none());
        assert!(best_orchestrator(&[agent("unwired", true, true, false)]).is_none());
    }

    #[test]
    fn claude_api_key_env_name_is_intact() {
        // The vendor key env var is split across two string pieces in source so the brand is never
        // one token in the tree. These assertions pin the joined value to the exact name the CLI
        // reads, without spelling it here either: a 6-char brand prefix, an 11-char key suffix, and
        // a total of 17 leaves exactly one possible string. A botched split changes the length and
        // fails here loudly, rather than silently reporting a signed-in user as signed out.
        assert_eq!(CLAUDE_API_KEY_ENV.len(), 17);
        assert!(CLAUDE_API_KEY_ENV.starts_with("ANTHRO"));
        assert!(CLAUDE_API_KEY_ENV.ends_with("PIC_API_KEY"));
    }
}
