//! Giving an agent its own copy of the repo to work in.
//!
//! Two agents editing one checkout overwrite each other, and the only advice that works without
//! isolation is "split the work by file", which is a rule humans have to enforce and agents forget.
//! A git worktree is the real answer: a second checkout of the same repository on its own branch,
//! sharing one object store. It costs a branch and a directory, not a clone.
//!
//! What makes this usable rather than just correct is the parts git does not do. A fresh worktree
//! has no `node_modules` and no `.env`, so the agent's first command fails and it spends its turn
//! debugging the sandbox instead of doing the work. I link the one and copy the other.
//!
//! Nothing here is silent. A repo that is not a repo, a worktree inside a worktree, a dirty branch
//! at merge time: each is a refusal with a reason, because the alternative is an agent quietly
//! working somewhere it should not.

use std::path::{Path, PathBuf};
use std::process::Command;

/// Where an isolated agent ended up, and the branch its work lands on.
#[derive(Debug, Clone, PartialEq)]
pub struct Isolated {
    /// The directory the agent runs in. For a monorepo this is the same subdirectory the caller
    /// asked about, inside the new worktree, not the worktree root.
    pub path: PathBuf,
    pub branch: String,
}

#[derive(Debug)]
pub enum Error {
    /// The path is not inside a git repository, so there is nothing to branch from.
    NotARepo(PathBuf),
    /// Already inside a worktree. Nesting them is a mess to unpick and never what anyone meant.
    AlreadyIsolated,
    /// The branch has uncommitted work, so merging it would land something nobody reviewed.
    Dirty(PathBuf),
    /// The worktree directory is already there.
    Exists(PathBuf),
    /// git said no. The string is git's own stderr, because it explains it better than I would.
    Git(String),
    Io(std::io::Error),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::NotARepo(p) => write!(f, "{} is not inside a git repository", p.display()),
            Error::AlreadyIsolated => write!(
                f,
                "this is already a git worktree, so I am sharing it rather than nesting another"
            ),
            Error::Dirty(p) => write!(
                f,
                "{} has uncommitted changes. Commit them on its branch first",
                p.display()
            ),
            Error::Exists(p) => write!(f, "{} already exists", p.display()),
            Error::Git(e) => write!(f, "git: {e}"),
            Error::Io(e) => write!(f, "io error: {e}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Io(e)
    }
}

/// Run git in `dir` and hand back stdout, or git's own complaint.
fn git(dir: &Path, args: &[&str]) -> Result<String, Error> {
    let out = Command::new("git").current_dir(dir).args(args).output()?;
    if !out.status.success() {
        return Err(Error::Git(
            String::from_utf8_lossy(&out.stderr).trim().to_string(),
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// The repository root that `dir` belongs to.
pub fn repo_root(dir: &Path) -> Result<PathBuf, Error> {
    if !dir.exists() {
        return Err(Error::NotARepo(dir.to_path_buf()));
    }
    git(dir, &["rev-parse", "--show-toplevel"])
        .map(PathBuf::from)
        .map_err(|_| Error::NotARepo(dir.to_path_buf()))
}

/// True when `root` is itself a worktree rather than the main checkout. In a worktree, `.git` is a
/// file pointing at the real store instead of a directory.
fn is_worktree(root: &Path) -> bool {
    root.join(".git").is_file()
}

/// Where isolated checkouts live: out of the user's tree, so a worktree never shows up as junk
/// inside the project they are looking at.
pub fn worktrees_root() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("IDENTRA_WORKTREES_ROOT") {
        return Some(PathBuf::from(dir));
    }
    let home = std::env::var_os("HOME")?;
    Some(
        Path::new(&home)
            .join(".local")
            .join("share")
            .join("identra")
            .join("worktrees"),
    )
}

/// Files a fresh checkout needs but git will not bring: secrets and lockfiles. Copied, not linked,
/// because an agent editing a lockfile in its own branch must not edit the main checkout's.
const CARRY: &[&str] = &[
    ".env",
    ".env.local",
    ".env.development",
    ".env.development.local",
    "bun.lock",
    "bun.lockb",
    "pnpm-lock.yaml",
    "yarn.lock",
    "package-lock.json",
];

/// Give the agent working in `dir` its own checkout on a fresh branch.
///
/// `label` only shapes the branch and folder name, so two agents on the same task still get their
/// own. `slug` is the caller's to make unique; I refuse rather than reuse a directory.
pub fn isolate(dir: &Path, slug: &str) -> Result<Isolated, Error> {
    let root = repo_root(dir)?;
    if is_worktree(&root) {
        return Err(Error::AlreadyIsolated);
    }
    let root_name = root
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "repo".into());
    let base = worktrees_root().ok_or_else(|| Error::NotARepo(dir.to_path_buf()))?;
    let path = base.join(&root_name).join(slug);
    if path.exists() {
        return Err(Error::Exists(path));
    }
    std::fs::create_dir_all(path.parent().unwrap_or(&base))?;

    let branch = format!("identra/{slug}");
    git(
        &root,
        &[
            "worktree",
            "add",
            "-b",
            &branch,
            &path.display().to_string(),
            "HEAD",
        ],
    )?;

    // The agent asked about `dir`, which may be a package inside a monorepo. Put it in the matching
    // place in the new checkout, not at the root, or nothing it runs will find its package.json.
    let relative = dir.strip_prefix(&root).unwrap_or(Path::new(""));
    let workdir = path.join(relative);

    // Both of these are best effort by design: a repo with no node_modules and no .env is a normal
    // repo, and failing the whole isolation because a link could not be made would be worse than
    // an agent running `install` itself. Anything that does fail is visible the moment it is used.
    link_modules(&root, &path);
    link_modules(dir, &workdir);
    carry_files(&root, &path);
    if relative != Path::new("") {
        carry_files(dir, &workdir);
    }

    Ok(Isolated {
        path: workdir,
        branch,
    })
}

/// Point the worktree's `node_modules` at the one that is already installed. A symlink, because a
/// copy of node_modules is gigabytes and minutes, and nothing in there is branch specific.
fn link_modules(from: &Path, to: &Path) {
    let src = from.join("node_modules");
    let dst = to.join("node_modules");
    if !src.is_dir() || dst.exists() || !to.is_dir() {
        return;
    }
    #[cfg(unix)]
    let _ = std::os::unix::fs::symlink(&src, &dst);
}

fn carry_files(from: &Path, to: &Path) {
    if !to.is_dir() {
        return;
    }
    for name in CARRY {
        let src = from.join(name);
        if src.is_file() {
            let _ = std::fs::copy(&src, to.join(name));
        }
    }
}

/// Work the agent left behind that a merge would not carry: changes to tracked files it never
/// committed, and files it created but never added.
///
/// I ignore exactly what I put in the tree myself (the carried secrets and the linked modules) and
/// nothing else. Ignoring all untracked files instead would be easier and wrong: a source file the
/// agent wrote and forgot to add is precisely the work that would vanish, which is the thing this
/// check exists to catch.
fn uncommitted(worktree: &Path) -> Result<Vec<String>, Error> {
    let status = git(worktree, &["status", "--porcelain"])?;
    Ok(status
        .lines()
        .filter_map(|line| line.get(3..).map(str::trim))
        .filter(|path| !CARRY.contains(path) && !path.starts_with("node_modules"))
        .map(str::to_string)
        .collect())
}

/// Land an isolated agent's branch back on the checkout it came from.
///
/// I refuse on uncommitted work rather than committing it myself: what the agent left in the tree
/// is not something I can write a message for, and a merge that invents a commit is worse than a
/// refusal that says what to do.
pub fn merge(worktree: &Path, squash: bool) -> Result<(), Error> {
    if !uncommitted(worktree)?.is_empty() {
        return Err(Error::Dirty(worktree.to_path_buf()));
    }
    let branch = git(worktree, &["rev-parse", "--abbrev-ref", "HEAD"])?;
    // The main checkout is the one worktree whose .git is a directory, and it is where this lands.
    let main = git(
        worktree,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
    )?;
    let main = PathBuf::from(main)
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| Error::Git("cannot find the main checkout".into()))?;

    let mut args = vec!["merge"];
    if squash {
        args.push("--squash");
    } else {
        // --no-ff keeps the agent's work as a distinguishable set of commits rather than melting it
        // into the base branch's history.
        args.push("--no-ff");
    }
    args.push(&branch);
    git(&main, &args)?;
    Ok(())
}

/// Remove an isolated checkout. The branch stays: the work is on it, and deleting someone's commits
/// because their sandbox went away is not a call this should make.
pub fn drop_worktree(worktree: &Path) -> Result<(), Error> {
    let root = repo_root(worktree)?;
    git(
        &root,
        &[
            "worktree",
            "remove",
            "--force",
            &worktree.display().to_string(),
        ],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A real repository with one commit. These tests drive git itself rather than a fake, because
    /// what I need to know is that git accepts these arguments, which a fake cannot tell me.
    fn repo(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("identra-wt-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let run = |args: &[&str]| {
            Command::new("git")
                .current_dir(&dir)
                .args(args)
                .output()
                .unwrap();
        };
        run(&["init", "-q", "-b", "main"]);
        run(&["config", "user.email", "t@example.com"]);
        run(&["config", "user.name", "t"]);
        std::fs::write(dir.join("README.md"), "hello\n").unwrap();
        std::fs::write(dir.join(".env"), "SECRET=1\n").unwrap();
        std::fs::create_dir_all(dir.join("node_modules/left-pad")).unwrap();
        run(&["add", "README.md"]);
        run(&["commit", "-qm", "first"]);
        dir
    }

    #[test]
    fn an_isolated_agent_gets_its_own_branch_and_a_usable_tree() {
        let dir = repo("isolate");
        let root = std::env::temp_dir().join(format!("identra-wtroot-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::env::set_var("IDENTRA_WORKTREES_ROOT", &root);

        let out = isolate(&dir, "helper-1").expect("isolate a normal repo");
        assert_eq!(out.branch, "identra/helper-1");
        assert!(
            out.path.join("README.md").is_file(),
            "it is a real checkout"
        );

        // The two things git does not bring, and without which the agent's first command fails.
        assert!(out.path.join(".env").is_file(), "secrets are carried in");
        assert!(
            out.path.join("node_modules").exists(),
            "node_modules is linked, not missing"
        );

        // Its own branch: work here does not touch what the other agent sees.
        std::fs::write(out.path.join("new.txt"), "work\n").unwrap();
        assert!(
            !dir.join("new.txt").exists(),
            "the main checkout is untouched"
        );

        // A second agent asking for the same name is refused rather than handed the first one's
        // tree, which would be the exact collision this exists to prevent.
        assert!(matches!(isolate(&dir, "helper-1"), Err(Error::Exists(_))));

        // Refuse to nest. Isolating from inside a worktree gives a mess nobody meant to ask for.
        assert!(matches!(
            isolate(&out.path, "helper-2"),
            Err(Error::AlreadyIsolated)
        ));

        // Uncommitted work does not silently land: new.txt above was never added.
        assert!(matches!(merge(&out.path, false), Err(Error::Dirty(_))));
        // And what I carried in myself does not count as the agent's work. Without this the tree
        // is dirty from the moment it is made and a merge could never happen at all.
        assert!(
            uncommitted(&out.path).unwrap() == vec!["new.txt".to_string()],
            "only the agent's own file is outstanding, got {:?}",
            uncommitted(&out.path).unwrap()
        );

        drop_worktree(&out.path).expect("clean up the worktree");
        assert!(!out.path.join("README.md").is_file());

        std::env::remove_var("IDENTRA_WORKTREES_ROOT");
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn committed_work_lands_and_a_non_repo_is_refused() {
        let dir = repo("merge");
        let root = std::env::temp_dir().join(format!("identra-wtroot2-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::env::set_var("IDENTRA_WORKTREES_ROOT", &root);

        let out = isolate(&dir, "worker").unwrap();
        std::fs::write(out.path.join("feature.txt"), "done\n").unwrap();
        for args in [
            vec!["add", "feature.txt"],
            vec!["commit", "-qm", "add the feature"],
        ] {
            Command::new("git")
                .current_dir(&out.path)
                .args(&args)
                .output()
                .unwrap();
        }

        merge(&out.path, false).expect("committed work lands");
        assert!(
            dir.join("feature.txt").is_file(),
            "the agent's work is on the main checkout now"
        );

        // Somewhere that is not a repo has nothing to branch from, and says so.
        let plain = std::env::temp_dir().join(format!("identra-plain-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&plain);
        std::fs::create_dir_all(&plain).unwrap();
        assert!(matches!(isolate(&plain, "x"), Err(Error::NotARepo(_))));

        std::env::remove_var("IDENTRA_WORKTREES_ROOT");
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&plain);
    }
}
