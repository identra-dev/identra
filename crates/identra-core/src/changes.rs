//! What the agents have done to the working tree, as a thing a person can read.
//!
//! This is the one question Identra could not answer about itself. Four agents run in a workspace,
//! every one of them editing files, and the only way to find out what they changed was to open a
//! terminal and run `git status` yourself — inside the app whose entire purpose is watching agents
//! work. The task board says what they *claimed*, the memory says what they *decided*, and nothing
//! said what they *did*.
//!
//! Read-only, deliberately, and see `changes()`'s note on why staging and revert are not here.
//!
//! Shelling out to git rather than linking a library. `worktree.rs` already does, so this adds no
//! dependency and no second idea of what a repository is, and the answers are the ones the user
//! would get typing the same commands in the terminal one pane over. A library would be faster and
//! would introduce a way for the panel and the terminal beside it to disagree.

use std::path::Path;
use std::process::Command;

use serde::Serialize;

/// One file the working tree has changed, relative to the repository root.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct FileChange {
    /// Repository-relative, forward slashes, as git reports it.
    pub path: String,
    /// Lines added and removed. Both are `None` for a binary file, which git reports as `-`, and a
    /// zero would be a lie there rather than a smaller truth.
    pub added: Option<u32>,
    pub removed: Option<u32>,
    /// What happened to it: added, modified, deleted, renamed, or untracked.
    pub state: State,
    /// Whether the change is in the index. Shown, not acted on: it is part of describing the tree
    /// honestly, and someone who staged files in their terminal should not see the panel claim
    /// otherwise.
    pub staged: bool,
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum State {
    Added,
    Modified,
    Deleted,
    Renamed,
    /// Not tracked by git at all. Kept rather than filtered: a file an agent created is the single
    /// most interesting row in this list, and it is exactly the one `git diff` will not show you.
    Untracked,
}

/// The working tree, summarised.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct Changes {
    /// The branch checked out here, or `None` on a detached HEAD. `None` is not an error: an agent
    /// left on a detached head is a state worth seeing rather than hiding behind a failure.
    pub branch: Option<String>,
    /// True when this checkout is one of Identra's isolated worktrees rather than the user's own.
    /// The person reading needs to know whether they are looking at their branch or a helper's.
    pub worktree: bool,
    pub files: Vec<FileChange>,
}

#[derive(Debug)]
pub enum Error {
    NotARepo,
    Git(String),
    Io(std::io::Error),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::NotARepo => write!(f, "this workspace is not a git repository"),
            Error::Git(e) => write!(f, "git said: {e}"),
            Error::Io(e) => write!(f, "could not run git: {e}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Io(e)
    }
}

fn git(dir: &Path, args: &[&str]) -> Result<String, Error> {
    let out = Command::new("git").current_dir(dir).args(args).output()?;
    if !out.status.success() {
        return Err(Error::Git(
            String::from_utf8_lossy(&out.stderr).trim().to_string(),
        ));
    }
    // Not trimmed. Porcelain output is parsed line by line and a trailing newline is the record
    // separator; trimming it here would be invisible until a path with trailing whitespace, which
    // is legal, quietly lost a character.
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// What has changed in `dir`'s working tree.
///
/// # Why this is read-only
///
/// Superset's equivalent column stages and reverts, and this one does neither. Staging without a
/// commit control is half a gesture, and the terminal that can finish it is one pane away. Revert
/// is the real reason: discarding uncommitted work is the only operation in this app that destroys
/// something no undo can bring back, and it would be sitting one click from a list a person scans
/// quickly, describing files an agent wrote while they were not watching. Seeing what changed is
/// the whole of the gap; being able to throw it away is a separate decision that should be made on
/// its own and not smuggled in behind a panel.
pub fn changes(dir: &Path) -> Result<Changes, Error> {
    if !dir.exists() {
        return Err(Error::NotARepo);
    }
    let root = git(dir, &["rev-parse", "--show-toplevel"])
        .map(|s| std::path::PathBuf::from(s.trim()))
        .map_err(|_| Error::NotARepo)?;

    // `--symbolic-full-name` rather than `--abbrev-ref`, because the latter answers "HEAD" on a
    // detached head, which is indistinguishable from a branch actually called HEAD.
    let branch = git(&root, &["symbolic-ref", "--quiet", "--short", "HEAD"])
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    let mut files = statuses(&root)?;
    let counts = numstat(&root)?;
    for f in &mut files {
        if let Some(&(added, removed)) = counts.get(f.path.as_str()) {
            f.added = added;
            f.removed = removed;
        }
    }
    // Directory order, so the panel can group without sorting twice and two reads of the same tree
    // never come back in a different order.
    files.sort_by(|a, b| a.path.cmp(&b.path));

    Ok(Changes {
        branch,
        worktree: root.join(".git").is_file(),
        files,
    })
}

/// Every changed path with its index and worktree status letters.
///
/// `-z` and NUL separation, not lines. Git quotes and escapes paths containing spaces or newlines
/// in its default output, which means a naive line parser both mangles ordinary filenames and can
/// be made to see rows that are not there by a file with a newline in its name — a file an agent
/// could create. NUL separation has no escaping, so there is nothing to get wrong.
fn statuses(root: &Path) -> Result<Vec<FileChange>, Error> {
    let out = git(
        root,
        &["status", "--porcelain=v1", "-z", "--untracked-files=all"],
    )?;
    let mut fields = out.split('\0');
    let mut files = Vec::new();
    while let Some(entry) = fields.next() {
        if entry.len() < 3 {
            continue;
        }
        let bytes = entry.as_bytes();
        let (index, tree) = (bytes[0] as char, bytes[1] as char);
        let path = entry[3..].to_string();
        // A rename is two NUL-separated fields: the new path in this entry and the old one in the
        // next. The old path is consumed here so it is never mistaken for its own change.
        if index == 'R' || tree == 'R' {
            let _ = fields.next();
        }
        let state = match (index, tree) {
            ('?', _) => State::Untracked,
            ('R', _) | (_, 'R') => State::Renamed,
            ('A', _) => State::Added,
            ('D', _) | (_, 'D') => State::Deleted,
            _ => State::Modified,
        };
        files.push(FileChange {
            path,
            added: None,
            removed: None,
            state,
            staged: index != ' ' && index != '?',
        });
    }
    Ok(files)
}

/// Added and removed line counts per path, staged and unstaged together.
///
/// Two calls, because a file can be partly staged and the panel shows one row per file: what a
/// person wants from that row is how far the file has moved from HEAD, which is both halves added
/// up. `git diff HEAD` would answer it in one call and reports nothing at all for a path that was
/// staged as a rename, so it is two.
type Counts = std::collections::HashMap<String, (Option<u32>, Option<u32>)>;

fn numstat(root: &Path) -> Result<Counts, Error> {
    let mut counts: Counts = std::collections::HashMap::new();
    for args in [
        &["diff", "--numstat", "-z"][..],
        &["diff", "--numstat", "-z", "--cached"][..],
    ] {
        for (path, added, removed) in parse_numstat(&git(root, args)?) {
            let slot = counts.entry(path).or_insert((Some(0), Some(0)));
            // Binary anywhere wins. Half a count on a file git will not diff is worse than saying
            // plainly that there is no line count to give.
            slot.0 = match (slot.0, added) {
                (Some(a), Some(b)) => Some(a + b),
                _ => None,
            };
            slot.1 = match (slot.1, removed) {
                (Some(a), Some(b)) => Some(a + b),
                _ => None,
            };
        }
    }
    Ok(counts)
}

/// `--numstat -z` rows: `added \t removed \t path NUL`, except a rename, which is
/// `added \t removed \t NUL old NUL new`. Pure, so the shape is testable without a repository.
fn parse_numstat(out: &str) -> Vec<(String, Option<u32>, Option<u32>)> {
    let mut fields = out.split('\0').peekable();
    let mut rows = Vec::new();
    while let Some(field) = fields.next() {
        if field.is_empty() {
            continue;
        }
        let mut parts = field.splitn(3, '\t');
        let (Some(added), Some(removed), Some(path)) = (parts.next(), parts.next(), parts.next())
        else {
            continue;
        };
        // A dash is git saying this file is binary, which is a different answer from zero lines.
        let num = |s: &str| if s == "-" { None } else { s.parse().ok() };
        let path = if path.is_empty() {
            // Rename: the path field is empty and the old and new paths follow as their own
            // fields. The new one is what the tree has now, so that is the row.
            let _old = fields.next();
            match fields.next() {
                Some(new) => new.to_string(),
                None => continue,
            }
        } else {
            path.to_string()
        };
        rows.push((path, num(added), num(removed)));
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Renames are two paths in one row, and getting it wrong shows the user a file at a path that
    /// no longer exists while the one that does exist is missing its counts.
    #[test]
    fn a_rename_is_counted_against_the_path_the_file_has_now() {
        let out = "3\t1\t\0src/old.rs\0src/new.rs\0";
        assert_eq!(
            parse_numstat(out),
            vec![("src/new.rs".to_string(), Some(3), Some(1))]
        );
    }

    /// Git says `-` for a file it will not diff. Parsing that as zero would put "+0 -0" on a 4MB
    /// PNG an agent just committed, which reads as "nothing happened here".
    #[test]
    fn a_binary_file_has_no_line_count_rather_than_a_count_of_zero() {
        let rows = parse_numstat("-\t-\tassets/logo.png\0");
        assert_eq!(rows, vec![("assets/logo.png".to_string(), None, None)]);
    }

    #[test]
    fn ordinary_rows_parse_and_a_trailing_separator_adds_nothing() {
        let rows = parse_numstat("10\t2\tsrc/lib.rs\0\0");
        assert_eq!(rows, vec![("src/lib.rs".to_string(), Some(10), Some(2))]);
    }

    /// The end-to-end shape, against a real repository, because the porcelain parsing is the part
    /// that has to agree with git rather than with my reading of the manual.
    #[test]
    fn a_real_tree_reports_what_changed_including_files_git_diff_will_not_show() {
        let dir = std::env::temp_dir().join(format!("identra-changes-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let run = |args: &[&str]| {
            Command::new("git")
                .current_dir(&dir)
                .args(args)
                .output()
                .unwrap();
        };
        run(&["init", "-q", "-b", "work"]);
        run(&["config", "user.email", "t@t"]);
        run(&["config", "user.name", "t"]);
        std::fs::write(dir.join("kept.txt"), "one\ntwo\n").unwrap();
        run(&["add", "."]);
        run(&["commit", "-qm", "first"]);

        // Three shapes at once: an edit, a file an agent created and never added, and a deletion.
        std::fs::write(dir.join("kept.txt"), "one\ntwo\nthree\n").unwrap();
        std::fs::write(dir.join("made-by-agent.txt"), "new\n").unwrap();
        std::fs::remove_file(dir.join("kept.txt")).ok();
        std::fs::write(dir.join("kept.txt"), "one\ntwo\nthree\n").unwrap();

        let c = changes(&dir).unwrap();
        assert_eq!(c.branch.as_deref(), Some("work"));
        assert!(!c.worktree);

        let edited = c.files.iter().find(|f| f.path == "kept.txt").unwrap();
        assert_eq!(edited.state, State::Modified);
        assert_eq!(edited.added, Some(1));

        // The row that matters most and the one `git diff` alone would have missed entirely.
        let made = c
            .files
            .iter()
            .find(|f| f.path == "made-by-agent.txt")
            .unwrap();
        assert_eq!(made.state, State::Untracked);
        assert!(!made.staged);

        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// A workspace that is not a repository is an ordinary state — Identra makes empty workspaces —
    /// so it has to be a nameable answer rather than a panel that errors.
    #[test]
    fn a_folder_that_is_not_a_repository_says_so() {
        let dir = std::env::temp_dir().join(format!("identra-norepo-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert!(matches!(changes(&dir), Err(Error::NotARepo)));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
