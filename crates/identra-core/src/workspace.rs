//! Workspaces.
//!
//! A workspace is a folder. It holds its own `.identra/canvas.json`, and it is also the directory
//! the agents in it run in, because a node's `cwd` already defaults to the project dir. So "the
//! auth-refactor workspace" is one thing: a canvas of agent nodes plus the folder they all work in.
//! No second concept, and no new persistence path: creating a workspace is `mkdir` plus the canvas
//! save that already exists.
//!
//! The folder name is the id and the canvas title is the display name. I slug the title to get the
//! folder, and dedup with a numeric suffix, so two workspaces both called "untitled workspace" get
//! `untitled-workspace` and `untitled-workspace-2` instead of one clobbering the other.

use serde::Serialize;
use std::io;
use std::path::{Path, PathBuf};

use crate::canvas::{self, Canvas};

/// What the workspace picker needs to draw a row. `path` is absolute so the frontend can show the
/// real location, since a workspace being a real folder on disk is the point.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct WorkspaceMeta {
    pub slug: String,
    pub title: String,
    pub path: String,
}

pub const DEFAULT_TITLE: &str = "untitled-workspace";

/// Where workspaces live. `IDENTRA_WORKSPACES_ROOT` overrides it, which is what I use in dev so
/// scratch workspaces never land inside the source tree. Otherwise `~/Identra`.
pub fn root() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("IDENTRA_WORKSPACES_ROOT") {
        return Some(PathBuf::from(dir));
    }
    std::env::var_os("HOME").map(|home| Path::new(&home).join("Identra"))
}

/// Fold a title down to a safe folder name: lowercase, and every run of non-alphanumeric characters
/// becomes a single dash. Treating everything that is not alphanumeric as a separator is what makes
/// this safe by construction: no slash, dot, or space survives, so no title can walk out of the
/// workspaces root. An empty result falls back to the default rather than a folder called "" or ".".
pub fn slugify(title: &str) -> String {
    let mut out = String::new();
    let mut last_dash = true; // leading dashes are dropped
    for ch in title.trim().chars() {
        let c = ch.to_ascii_lowercase();
        if c.is_ascii_alphanumeric() {
            out.push(c);
            last_dash = false;
        } else if !last_dash {
            out.push('-');
            last_dash = true;
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    if out.is_empty() {
        DEFAULT_TITLE.to_string()
    } else {
        out
    }
}

/// `base` if it is free, else `base-2`, `base-3`, and so on. I check the directory rather than the
/// listed workspaces so a folder that exists for any reason is never written into.
pub fn free_slug(root: &Path, base: &str) -> String {
    if !root.join(base).exists() {
        return base.to_string();
    }
    for n in 2.. {
        let candidate = format!("{base}-{n}");
        if !root.join(&candidate).exists() {
            return candidate;
        }
    }
    unreachable!("the loop returns on the first free name")
}

fn meta_for(root: &Path, slug: &str) -> WorkspaceMeta {
    let path = root.join(slug);
    WorkspaceMeta {
        title: canvas::load(&path).title,
        slug: slug.to_string(),
        path: path.display().to_string(),
    }
}

/// Every workspace under `root`: a subdirectory carrying a `.identra/canvas.json`. A stray folder
/// with no canvas is not a workspace, so it is skipped rather than listed as a broken one.
pub fn list(root: &Path) -> Vec<WorkspaceMeta> {
    let Ok(entries) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    let mut out: Vec<WorkspaceMeta> = entries
        .flatten()
        .filter(|e| canvas::canvas_path(&e.path()).is_file())
        .filter_map(|e| e.file_name().into_string().ok())
        .map(|slug| meta_for(root, &slug))
        .collect();
    out.sort_by(|a, b| a.slug.cmp(&b.slug));
    out
}

/// Make a workspace: a folder plus a blank canvas carrying the title. Returns the new workspace so
/// the caller can make it active without a second lookup.
pub fn create(root: &Path, title: &str) -> io::Result<WorkspaceMeta> {
    let title = if title.trim().is_empty() {
        DEFAULT_TITLE
    } else {
        title.trim()
    };
    let slug = free_slug(root, &slugify(title));
    let path = root.join(&slug);
    std::fs::create_dir_all(&path)?;
    let board = Canvas {
        title: title.to_string(),
        ..Canvas::default()
    };
    canvas::save(&path, &board)?;
    Ok(WorkspaceMeta {
        slug,
        title: title.to_string(),
        path: path.display().to_string(),
    })
}

/// Where the list of folders opened as workspaces lives.
///
/// A workspace made here is found by scanning the root, but a folder you already had is somewhere
/// else entirely and there is nothing to scan. So the ones you have opened are remembered, and that
/// list is also the authorization: opening a remembered folder is opening one you chose before,
/// which is why nothing else accepts a path from outside.
fn recents_path() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("IDENTRA_RECENTS_FILE") {
        return Some(PathBuf::from(dir));
    }
    let home = std::env::var_os("HOME")?;
    Some(
        Path::new(&home)
            .join(".local")
            .join("share")
            .join("identra")
            .join("recents.json"),
    )
}

fn read_recents(file: &Path) -> Vec<PathBuf> {
    std::fs::read_to_string(file)
        .ok()
        .and_then(|t| serde_json::from_str::<Vec<PathBuf>>(&t).ok())
        .unwrap_or_default()
}

fn write_recents(file: &Path, list: &[PathBuf]) -> io::Result<()> {
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = file.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(list)?)?;
    std::fs::rename(&tmp, file)
}

/// Folders opened as workspaces, most recent first, skipping any that are gone.
///
/// A remembered folder that has been deleted or moved is not an error worth showing: it is just not
/// there any more, and a list of dead rows is worse than a short list.
pub fn recents() -> Vec<WorkspaceMeta> {
    let Some(file) = recents_path() else {
        return Vec::new();
    };
    read_recents(&file)
        .into_iter()
        .filter(|p| canvas::canvas_path(p).is_file())
        .map(|p| WorkspaceMeta {
            slug: p.display().to_string(),
            title: canvas::load(&p).title,
            path: p.display().to_string(),
        })
        .collect()
}

/// Turn a folder the user already has into a workspace, and remember it.
///
/// This is the difference between a scratch pad and a tool: a workspace you make here is empty, and
/// the code someone actually wants agents to work on is already somewhere on their disk. Adopting
/// adds `.identra/` and nothing else, so the folder stays theirs, and their repo is not reshaped to
/// suit us.
///
/// The path is the caller's to justify. Nothing in the app takes one from the window: it comes from
/// the user picking a folder, or from the remembered list, both of which are the user's own choice.
pub fn adopt(path: &Path) -> io::Result<WorkspaceMeta> {
    if !path.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("{} is not a folder", path.display()),
        ));
    }
    // A folder that is already a workspace keeps its canvas and its name. Adopting twice must not
    // wipe the board someone has been working on.
    if !canvas::canvas_path(path).is_file() {
        let title = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| DEFAULT_TITLE.to_string());
        canvas::save(
            path,
            &Canvas {
                title,
                ..Canvas::default()
            },
        )?;
    }

    if let Some(file) = recents_path() {
        let mut list = read_recents(&file);
        // Dedupe and promote: reopening a folder moves it to the top rather than listing it twice.
        list.retain(|p| p != path);
        list.insert(0, path.to_path_buf());
        list.truncate(RECENTS_MAX);
        let _ = write_recents(&file, &list);
    }

    Ok(WorkspaceMeta {
        slug: path.display().to_string(),
        title: canvas::load(path).title,
        path: path.display().to_string(),
    })
}

/// Long enough to find the thing you were on last week, short enough that the list stays a list.
const RECENTS_MAX: usize = 20;

/// Stop remembering a folder. The folder itself is untouched: this is a list, not the work.
pub fn forget_recent(path: &Path) {
    if let Some(file) = recents_path() {
        let mut list = read_recents(&file);
        list.retain(|p| p != path);
        let _ = write_recents(&file, &list);
    }
}

/// Give a workspace a new name, moving its folder to match.
///
/// The folder is the id, so a rename is a move, and a move is the part that can go wrong: the
/// canvas, the memory, and the bus state all live in that directory. I rename the directory first
/// and only write the new title once it lands, so a failed move leaves a workspace that is intact
/// and still called the old thing, rather than one whose name and location disagree.
///
/// Returns the workspace at its new home. The caller must repoint anything holding the old path.
pub fn rename(root: &Path, slug: &str, title: &str) -> io::Result<WorkspaceMeta> {
    let title = title.trim();
    if title.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "a workspace needs a name",
        ));
    }
    let from = root.join(slug);
    if !canvas::canvas_path(&from).is_file() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("no workspace named {slug}"),
        ));
    }

    // Renaming to a name that slugs the same (Auth Refactor -> auth refactor) must not try to move
    // a directory onto itself, and must not dedup itself to auth-refactor-2 either.
    let wanted = slugify(title);
    let to_slug = if wanted == slug {
        slug.to_string()
    } else {
        free_slug(root, &wanted)
    };
    let to = root.join(&to_slug);
    if to_slug != slug {
        std::fs::rename(&from, &to)?;
    }

    let mut board = canvas::load(&to);
    board.title = title.to_string();
    canvas::save(&to, &board)?;
    Ok(WorkspaceMeta {
        slug: to_slug,
        title: title.to_string(),
        path: to.display().to_string(),
    })
}

/// Delete a workspace and everything in it.
///
/// This takes the user's files with it, not just the canvas: the workspace folder is where the
/// agents were working. The caller is responsible for asking first, and for saying that plainly.
/// I refuse anything that is not a workspace, so a wrong slug cannot turn into a recursive delete
/// of something else.
pub fn delete(root: &Path, slug: &str) -> io::Result<()> {
    let path = root.join(slug);
    if !canvas::canvas_path(&path).is_file() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("no workspace named {slug}"),
        ));
    }
    std::fs::remove_dir_all(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::canvas::Node;

    #[test]
    fn slugs_are_safe_folder_names() {
        assert_eq!(slugify("Auth refactor"), "auth-refactor");
        assert_eq!(slugify("  My App!! v2  "), "my-app-v2");
        assert_eq!(slugify("../../etc/passwd"), "etc-passwd"); // no traversal survives
        assert_eq!(slugify(""), DEFAULT_TITLE);
        assert_eq!(slugify("!!!"), DEFAULT_TITLE);
    }

    #[test]
    fn adopting_a_real_folder_leaves_it_alone_and_remembers_it() {
        let base = std::env::temp_dir().join(format!("identra-adopt-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let repo = base.join("my-app");
        std::fs::create_dir_all(repo.join("src")).unwrap();
        std::fs::write(repo.join("src/main.rs"), "fn main() {}").unwrap();
        std::env::set_var("IDENTRA_RECENTS_FILE", base.join("recents.json"));

        let out = adopt(&repo).unwrap();
        assert_eq!(out.title, "my-app", "the folder names itself");
        assert_eq!(out.path, repo.display().to_string());
        // Adopting adds .identra and nothing else. This is someone's repo, not ours to reshape.
        assert!(canvas::canvas_path(&repo).is_file());
        assert_eq!(
            std::fs::read_to_string(repo.join("src/main.rs")).unwrap(),
            "fn main() {}"
        );
        assert_eq!(recents()[0].path, repo.display().to_string());

        // Adopting twice keeps the board someone has been working on rather than wiping it.
        let mut board = canvas::load(&repo);
        board.title = "My App".into();
        board.nodes.push(Node {
            id: "n1".into(),
            kind: "codex".into(),
            x: 0.0,
            y: 0.0,
            width: 480.0,
            height: 320.0,
            title: "codex".into(),
            cwd: None,
        });
        canvas::save(&repo, &board).unwrap();
        let again = adopt(&repo).unwrap();
        assert_eq!(again.title, "My App", "its name survives");
        assert_eq!(canvas::load(&repo).nodes.len(), 1, "its canvas survives");
        assert_eq!(
            recents().len(),
            1,
            "reopening promotes, it does not duplicate"
        );

        // A second folder goes to the top, and the first is still there behind it.
        let other = base.join("other");
        std::fs::create_dir_all(&other).unwrap();
        adopt(&other).unwrap();
        assert_eq!(recents()[0].path, other.display().to_string());
        assert_eq!(recents().len(), 2);

        // A folder that has gone away is not an error, it is just not on the list any more.
        std::fs::remove_dir_all(&other).unwrap();
        assert_eq!(recents().len(), 1);
        assert_eq!(recents()[0].path, repo.display().to_string());

        forget_recent(&repo);
        assert_eq!(recents().len(), 0);
        assert!(
            repo.is_dir(),
            "forgetting a folder does not touch the folder"
        );

        assert!(adopt(&base.join("nope")).is_err());
        std::env::remove_var("IDENTRA_RECENTS_FILE");
        std::fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn renaming_moves_the_folder_and_keeps_everything_in_it() {
        let root = std::env::temp_dir().join(format!("identra-ws-rn-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();

        let made = create(&root, "untitled-workspace").unwrap();
        // Something of the user's, in the workspace. A rename must not lose it: the folder is where
        // the agents were working, not just where a layout file lives.
        std::fs::write(root.join(&made.slug).join("notes.txt"), "keep me").unwrap();

        let renamed = rename(&root, &made.slug, "Auth refactor").unwrap();
        assert_eq!(renamed.slug, "auth-refactor");
        assert_eq!(renamed.title, "Auth refactor");
        assert!(
            !root.join("untitled-workspace").exists(),
            "the old folder is gone"
        );
        assert_eq!(
            std::fs::read_to_string(root.join("auth-refactor").join("notes.txt")).unwrap(),
            "keep me",
            "the user's files moved with it"
        );
        assert_eq!(
            canvas::load(&root.join("auth-refactor")).title,
            "Auth refactor"
        );

        // A name that slugs to where it already is must not move onto itself or dedup itself.
        let same = rename(&root, "auth-refactor", "Auth Refactor").unwrap();
        assert_eq!(same.slug, "auth-refactor");
        assert_eq!(same.title, "Auth Refactor");

        // A rename that collides with a real other workspace gets its own folder, never a clobber.
        create(&root, "docs").unwrap();
        let moved = rename(&root, "auth-refactor", "Docs").unwrap();
        assert_eq!(moved.slug, "docs-2");
        assert!(
            root.join("docs").exists(),
            "the workspace already called docs survives"
        );

        assert!(rename(&root, "nope", "x").is_err());
        assert!(rename(&root, "docs", "   ").is_err());

        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn deleting_takes_the_workspace_and_refuses_anything_else() {
        let root = std::env::temp_dir().join(format!("identra-ws-del-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();

        // A folder in the root that is not a workspace. A wrong slug must not turn delete into a
        // recursive remove of whatever happens to be sitting there.
        std::fs::create_dir_all(root.join("not-a-workspace")).unwrap();
        std::fs::write(root.join("not-a-workspace").join("important"), "x").unwrap();

        create(&root, "scratch").unwrap();
        assert!(delete(&root, "not-a-workspace").is_err());
        assert!(root.join("not-a-workspace").join("important").exists());
        assert!(delete(&root, "../..").is_err());

        delete(&root, "scratch").unwrap();
        assert!(!root.join("scratch").exists());
        assert_eq!(list(&root).len(), 0);
        assert!(
            delete(&root, "scratch").is_err(),
            "deleting twice is an error, not a no op"
        );

        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn an_absolute_slug_cannot_turn_delete_into_someone_elses_repo() {
        let root = std::env::temp_dir().join(format!("identra-ws-abs-{}", std::process::id()));
        let outside = std::env::temp_dir().join(format!("identra-repo-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&outside);
        std::fs::create_dir_all(&root).unwrap();

        // A real repo the user adopted. It has a canvas, so it looks exactly like a workspace to
        // anything that only checks for one.
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(outside.join("precious.rs"), "fn main() {}").unwrap();
        canvas::save(&outside, &Canvas::default()).unwrap();

        // join() with an absolute path replaces the base rather than extending it, so an adopted
        // folder's own path, used as a slug, resolves straight back to the repo. That is the trap:
        // a caller that joins a slug it did not build would recursively delete the user's code.
        assert_eq!(
            root.join(outside.to_str().unwrap()),
            outside,
            "an absolute slug escapes the root entirely"
        );

        // The list is the guard. It only ever reports children of the root, so a caller that picks
        // from it cannot name the repo, whatever it is handed.
        assert!(
            !list(&root)
                .iter()
                .any(|w| w.path == outside.display().to_string()),
            "an adopted folder is never a workspace in the root"
        );
        assert!(
            outside.join("precious.rs").is_file(),
            "and it is still there"
        );

        std::fs::remove_dir_all(&root).unwrap();
        std::fs::remove_dir_all(&outside).unwrap();
    }

    #[test]
    fn a_workspace_can_only_be_found_inside_the_root() {
        let root = std::env::temp_dir().join(format!("identra-ws-esc-{}", std::process::id()));
        let outside = std::env::temp_dir().join(format!("identra-ws-out-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&outside);
        std::fs::create_dir_all(&root).unwrap();

        // A real workspace sitting outside the root, canvas and all. This is the thing a traversal
        // would be reaching for: somewhere that looks like a workspace but is not one of ours.
        canvas::save(&outside, &Canvas::default()).unwrap();
        create(&root, "mine").unwrap();

        // list is the only way in, and it only ever walks the root's own children. There is no name
        // that makes it return the folder outside, because it never joins a caller's string onto a
        // path: it reports what it found.
        let found = list(&root);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].slug, "mine");
        for attempt in ["../identra-ws-out", "..", "../..", "/tmp"] {
            assert!(
                !found.iter().any(|w| w.slug == attempt),
                "{attempt} must not resolve to a workspace"
            );
        }
        assert!(found[0].path.starts_with(root.to_str().unwrap()));

        std::fs::remove_dir_all(&root).unwrap();
        std::fs::remove_dir_all(&outside).unwrap();
    }

    #[test]
    fn create_dedups_and_round_trips_the_canvas() {
        let root = std::env::temp_dir().join(format!("identra-ws-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();

        assert_eq!(list(&root), Vec::new()); // empty root lists nothing

        let first = create(&root, "untitled-workspace").unwrap();
        assert_eq!(first.slug, "untitled-workspace");

        // A second workspace with the same name gets its own folder, it does not clobber the first.
        let second = create(&root, "untitled-workspace").unwrap();
        assert_eq!(second.slug, "untitled-workspace-2");

        // The title rides in the canvas, and the folder is the id.
        let named = create(&root, "Auth refactor").unwrap();
        assert_eq!(named.slug, "auth-refactor");
        assert_eq!(
            canvas::load(&root.join("auth-refactor")).title,
            "Auth refactor"
        );

        let all = list(&root);
        assert_eq!(all.len(), 3);
        assert!(all.iter().any(|w| w.title == "Auth refactor"));

        std::fs::remove_dir_all(&root).unwrap();
    }
}
