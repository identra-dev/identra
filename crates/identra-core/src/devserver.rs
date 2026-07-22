//! What command runs this project's dev server.
//!
//! The judgement is deliberately shallow: the obvious script names in the obvious files, nothing
//! clever. A project with an unusual setup still has a terminal node the user can type anything
//! into; this only decides what the one-click Run button does, and a wrong guess there costs
//! more than no guess.

use std::path::Path;

/// The command that starts this project's dev server, or `None` when the project does not
/// declare one anywhere this looks.
///
/// package.json wins over a justfile over a Makefile, because where both exist the justfile
/// usually calls the package script anyway, and asking the package manager directly keeps the
/// output shapes (and the URL in them) the ones the sniffer knows.
pub fn command_for(dir: &Path) -> Option<Vec<String>> {
    if let Some(cmd) = package_json_dev(dir) {
        return Some(cmd);
    }
    if has_recipe(&dir.join("justfile"), "dev") || has_recipe(&dir.join("Justfile"), "dev") {
        return Some(vec!["just".into(), "dev".into()]);
    }
    if has_recipe(&dir.join("Makefile"), "dev") {
        return Some(vec!["make".into(), "dev".into()]);
    }
    None
}

/// `<runner> run dev` when package.json has a dev script. The runner is judged by the lockfile,
/// because running someone's project with the wrong package manager is a classic way to break it.
fn package_json_dev(dir: &Path) -> Option<Vec<String>> {
    let text = std::fs::read_to_string(dir.join("package.json")).ok()?;
    let json: serde_json::Value = serde_json::from_str(&text).ok()?;
    json.get("scripts")?.get("dev")?.as_str()?;
    let runner = if dir.join("bun.lock").exists() || dir.join("bun.lockb").exists() {
        "bun"
    } else if dir.join("pnpm-lock.yaml").exists() {
        "pnpm"
    } else if dir.join("yarn.lock").exists() {
        "yarn"
    } else {
        "npm"
    };
    Some(vec![runner.into(), "run".into(), "dev".into()])
}

/// Whether a justfile or Makefile defines the named recipe. A recipe is a line starting with the
/// name followed by `:` or a space-separated dependency list ending in `:`, which one starts-with
/// check covers for both formats without parsing either.
fn has_recipe(file: &Path, name: &str) -> bool {
    let Ok(text) = std::fs::read_to_string(file) else {
        return false;
    };
    text.lines().any(|line| {
        let line = line.trim_end();
        line.strip_prefix(name).is_some_and(|rest| {
            rest.starts_with(':') || (rest.starts_with(' ') && rest.contains(':'))
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn the_dev_command_is_read_from_what_the_project_declares() {
        let dir = std::env::temp_dir().join(format!("identra-dev-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        assert_eq!(command_for(&dir), None, "an empty folder runs nothing");

        // A Makefile is the last resort...
        fs::write(
            dir.join("Makefile"),
            "build:\n\tcc x.c\ndev: build\n\t./serve\n",
        )
        .unwrap();
        assert_eq!(command_for(&dir), Some(vec!["make".into(), "dev".into()]));

        // ...a justfile beats it...
        fs::write(dir.join("justfile"), "# tasks\ndev:\n    bun run dev\n").unwrap();
        assert_eq!(command_for(&dir), Some(vec!["just".into(), "dev".into()]));

        // ...and package.json beats both, with the runner judged by the lockfile.
        fs::write(
            dir.join("package.json"),
            r#"{"scripts":{"dev":"vite","build":"vite build"}}"#,
        )
        .unwrap();
        assert_eq!(
            command_for(&dir),
            Some(vec!["npm".into(), "run".into(), "dev".into()]),
            "no lockfile means npm"
        );
        fs::write(dir.join("bun.lock"), "").unwrap();
        assert_eq!(
            command_for(&dir),
            Some(vec!["bun".into(), "run".into(), "dev".into()])
        );

        // A package.json without a dev script does not shadow the justfile behind it.
        fs::write(dir.join("package.json"), r#"{"scripts":{"build":"tsc"}}"#).unwrap();
        assert_eq!(command_for(&dir), Some(vec!["just".into(), "dev".into()]));

        // A recipe merely mentioning dev is not a dev recipe.
        fs::write(dir.join("justfile"), "deverything:\n    echo no\n").unwrap();
        fs::remove_file(dir.join("Makefile")).unwrap();
        assert_eq!(command_for(&dir), None);

        fs::remove_dir_all(&dir).unwrap();
    }
}
