//! Writing and removing Identra's block in the user's `~/.codex/config.toml`.
//!
//! Codex reads its MCP server list once at startup, so the bus has to be in the config before an
//! agent launches. I write a single marked block and nothing else, so I can strip exactly that
//! block on exit and leave every other line the user has (including edits made while Identra ran)
//! untouched. I also keep a one-time backup of the original file as a crash safety net, in case
//! the app dies before it restores.

use std::io;
use std::path::{Path, PathBuf};

const START: &str = "# >>> identra bus";
const END: &str = "# <<< identra bus";

/// Where codex reads its config: `$CODEX_HOME/config.toml`, with `$CODEX_HOME` defaulting to
/// `~/.codex`. Returns `None` only if neither variable is set, which should not happen in a normal
/// desktop session.
pub fn codex_config_path() -> Option<PathBuf> {
    if let Some(codex_home) = std::env::var_os("CODEX_HOME") {
        return Some(Path::new(&codex_home).join("config.toml"));
    }
    let home = std::env::var_os("HOME")?;
    Some(Path::new(&home).join(".codex").join("config.toml"))
}

fn backup_path(config: &Path) -> PathBuf {
    let mut s = config.as_os_str().to_os_string();
    s.push(".identra-backup");
    PathBuf::from(s)
}

/// The block codex reads. The token is not written here: `bearer_token_env_var` names an env var
/// that Identra sets on each agent process, so every node authenticates with its own bearer and
/// the secret never sits in a file on disk.
fn block(port: u16) -> String {
    format!(
        "{START}\n\
         [mcp_servers.identra_bus]\n\
         url = \"http://127.0.0.1:{port}/mcp\"\n\
         bearer_token_env_var = \"IDENTRA_BUS_TOKEN\"\n\
         startup_timeout_sec = 10\n\
         {END}\n"
    )
}

/// Everything except a previously written Identra block, so a rewrite never stacks two blocks and
/// a restore removes only what Identra added.
fn strip_block(content: &str) -> String {
    let mut out = String::new();
    let mut skipping = false;
    for line in content.lines() {
        let t = line.trim_start();
        if t.starts_with(START) {
            skipping = true;
            continue;
        }
        if skipping {
            if t.starts_with(END) {
                skipping = false;
            }
            continue;
        }
        out.push_str(line);
        out.push('\n');
    }
    out
}

fn write_atomic(path: &Path, content: &str) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("toml.identra-tmp");
    std::fs::write(&tmp, content)?;
    std::fs::rename(&tmp, path)
}

/// Add or refresh Identra's `[mcp_servers.identra_bus]` block in `config`. Idempotent: any earlier
/// block is stripped first, so repeated calls (a relaunch on a new port) never duplicate it.
pub fn write_codex_bus(config: &Path, port: u16) -> io::Result<()> {
    let existing = std::fs::read_to_string(config).unwrap_or_default();
    // Back up the true original once, before Identra has touched the file.
    let backup = backup_path(config);
    if config.exists() && !backup.exists() {
        std::fs::copy(config, &backup)?;
    }
    let base = strip_block(&existing);
    let base = base.trim_end();
    let content = if base.is_empty() {
        block(port)
    } else {
        format!("{base}\n\n{}", block(port))
    };
    write_atomic(config, &content)
}

/// Put the config back the way Identra found it. If a backup exists it is the user's exact
/// original, so I move it back verbatim, which also drops the backup in one step. If there is no
/// backup, Identra created the file: I strip its block and, if nothing else remains, remove it.
pub fn restore_codex(config: &Path) -> io::Result<()> {
    let backup = backup_path(config);
    if backup.exists() {
        std::fs::rename(&backup, config)?;
        return Ok(());
    }
    if !config.exists() {
        return Ok(());
    }
    let content = std::fs::read_to_string(config)?;
    // Nothing of ours to remove (we never wrote, or the block is already gone): leave the file be
    // so restore is safe to call unconditionally on exit.
    if !content.contains(START) {
        return Ok(());
    }
    let stripped = strip_block(&content);
    if stripped.trim().is_empty() {
        std::fs::remove_file(config)?;
    } else {
        write_atomic(config, &stripped)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_config(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "identra-cfg-{}-{}/config.toml",
            std::process::id(),
            name
        ))
    }

    #[test]
    fn writes_strips_and_restores_without_touching_user_lines() {
        let cfg = temp_config("roundtrip");
        let _ = std::fs::remove_dir_all(cfg.parent().unwrap());

        // A config the user already had, with their own settings.
        let user = "[projects.\"/home/me/app\"]\ntrust_level = \"trusted\"\n";
        std::fs::create_dir_all(cfg.parent().unwrap()).unwrap();
        std::fs::write(&cfg, user).unwrap();

        write_codex_bus(&cfg, 8900).unwrap();
        let after = std::fs::read_to_string(&cfg).unwrap();
        assert!(
            after.contains("trust_level = \"trusted\""),
            "user lines kept"
        );
        assert!(after.contains("[mcp_servers.identra_bus]"));
        assert!(after.contains("http://127.0.0.1:8900/mcp"));
        assert!(backup_path(&cfg).exists(), "original backed up once");

        // Writing again on a new port refreshes in place, never stacks a second block.
        write_codex_bus(&cfg, 9100).unwrap();
        let after2 = std::fs::read_to_string(&cfg).unwrap();
        assert_eq!(after2.matches("[mcp_servers.identra_bus]").count(), 1);
        assert!(after2.contains(":9100/mcp"));

        restore_codex(&cfg).unwrap();
        let restored = std::fs::read_to_string(&cfg).unwrap();
        assert_eq!(
            restored, user,
            "restore returns the file to the user's exact content"
        );
        assert!(!backup_path(&cfg).exists(), "backup cleaned up");

        std::fs::remove_dir_all(cfg.parent().unwrap()).unwrap();
    }

    #[test]
    fn created_file_is_removed_on_restore() {
        let cfg = temp_config("created");
        let _ = std::fs::remove_dir_all(cfg.parent().unwrap());

        // No config existed: Identra creates it.
        write_codex_bus(&cfg, 8900).unwrap();
        assert!(cfg.exists());
        assert!(
            !backup_path(&cfg).exists(),
            "nothing to back up when the file was absent"
        );

        restore_codex(&cfg).unwrap();
        assert!(!cfg.exists(), "a file Identra created is removed cleanly");

        let _ = std::fs::remove_dir_all(cfg.parent().unwrap());
    }
}
