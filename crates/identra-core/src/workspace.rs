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

    #[test]
    fn slugs_are_safe_folder_names() {
        assert_eq!(slugify("Auth refactor"), "auth-refactor");
        assert_eq!(slugify("  My App!! v2  "), "my-app-v2");
        assert_eq!(slugify("../../etc/passwd"), "etc-passwd"); // no traversal survives
        assert_eq!(slugify(""), DEFAULT_TITLE);
        assert_eq!(slugify("!!!"), DEFAULT_TITLE);
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
