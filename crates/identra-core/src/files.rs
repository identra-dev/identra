//! Listing and searching the workspace for the Files panel.
//!
//! Same trust rule as the file viewer: every path from the window is checked against the
//! workspace root, canonicalized, before the filesystem answers anything. The listing is lazy,
//! one directory per call, because a panel does not need the whole tree to draw a row.

use serde::Serialize;
use std::io;
use std::path::{Path, PathBuf};

/// One row in the panel. `path` is relative to the workspace root, which keeps the frontend out
/// of the business of joining absolute paths it would then have to be trusted with.
#[derive(Serialize, Debug, PartialEq)]
pub struct Entry {
    pub name: String,
    pub path: String,
    pub dir: bool,
    pub size: u64,
}

/// Directories nobody browses on purpose and nobody wants searched: dependency trees and build
/// output. Hidden entries are skipped separately, which covers `.git` and `.identra`.
const SKIP_DIRS: [&str; 4] = ["node_modules", "target", "dist", "build"];

/// The checked absolute path for a workspace-relative one. Public because the shell needs it for
/// reveal: handing the OS file manager a path is exactly the moment the containment rule matters.
pub fn resolve(root: &Path, rel: &str) -> io::Result<PathBuf> {
    resolved_inside(root, rel)
}

fn resolved_inside(root: &Path, rel: &str) -> io::Result<PathBuf> {
    let joined = root.join(rel);
    let full = joined.canonicalize()?;
    let base = root.canonicalize()?;
    if !full.starts_with(&base) {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "that path is outside this workspace",
        ));
    }
    Ok(full)
}

/// The entries of one directory under the workspace, folders first, names sorted, hidden
/// entries skipped. `rel` is relative to the root; empty means the root itself.
pub fn list(root: &Path, rel: &str) -> io::Result<Vec<Entry>> {
    let dir = resolved_inside(root, rel)?;
    let base = root.canonicalize()?;
    let mut out = Vec::new();
    for entry in std::fs::read_dir(&dir)?.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') {
            continue;
        }
        let Ok(meta) = entry.metadata() else { continue };
        let path = entry
            .path()
            .strip_prefix(&base)
            .map(|p| p.display().to_string())
            .unwrap_or_else(|_| name.clone());
        out.push(Entry {
            name,
            path,
            dir: meta.is_dir(),
            size: meta.len(),
        });
    }
    out.sort_by(|a, b| b.dir.cmp(&a.dir).then(a.name.cmp(&b.name)));
    Ok(out)
}

/// A search hit: the file, and for a content match the line it was on. A name match carries no
/// line, and the panel shows the two shapes differently.
#[derive(Serialize, Debug, PartialEq)]
pub struct Hit {
    pub path: String,
    pub line: Option<u32>,
    pub snippet: Option<String>,
}

/// Files larger than this are not content-searched. Text anyone greps by hand is far smaller;
/// past it the walk is paying to scan build artifacts.
const SEARCH_MAX_BYTES: u64 = 512 * 1024;

/// Search names and text content under the workspace, case-insensitive, capped at `max` hits.
///
/// Hand-rolled walk rather than a crate because the needs are small: skip hidden and dependency
/// directories, cap the depth so a symlink cycle cannot hang it, and stop the moment the cap is
/// reached. Binary files drop out naturally: their bytes do not parse as UTF-8.
pub fn search(root: &Path, query: &str, max: usize) -> Vec<Hit> {
    let needle = query.trim().to_lowercase();
    let mut hits = Vec::new();
    if needle.is_empty() {
        return hits;
    }
    let Ok(base) = root.canonicalize() else {
        return hits;
    };
    walk(&base, &base, &needle, 0, &mut hits, max);
    hits
}

fn walk(base: &Path, dir: &Path, needle: &str, depth: u32, hits: &mut Vec<Hit>, max: usize) {
    if depth > 12 || hits.len() >= max {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        if hits.len() >= max {
            return;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') || SKIP_DIRS.contains(&name.as_str()) {
            continue;
        }
        let path = entry.path();
        let Ok(meta) = entry.metadata() else { continue };
        let rel = path
            .strip_prefix(base)
            .map(|p| p.display().to_string())
            .unwrap_or_else(|_| name.clone());
        if meta.is_dir() {
            walk(base, &path, needle, depth + 1, hits, max);
            continue;
        }
        if name.to_lowercase().contains(needle) {
            hits.push(Hit {
                path: rel.clone(),
                line: None,
                snippet: None,
            });
            continue;
        }
        if meta.len() > SEARCH_MAX_BYTES {
            continue;
        }
        let Ok(bytes) = std::fs::read(&path) else {
            continue;
        };
        let Ok(text) = String::from_utf8(bytes) else {
            continue;
        };
        for (i, line) in text.lines().enumerate() {
            if line.to_lowercase().contains(needle) {
                hits.push(Hit {
                    path: rel,
                    line: Some(i as u32 + 1),
                    snippet: Some(line.trim().chars().take(160).collect()),
                });
                // One hit per file. The panel answers "where is it", and ten rows of the same
                // file answer it no better than one.
                break;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn the_panel_lists_and_searches_only_what_a_person_would_want() {
        let base = std::env::temp_dir().join(format!("identra-files-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        let ws = base.join("ws");
        fs::create_dir_all(ws.join("src")).unwrap();
        fs::create_dir_all(ws.join("node_modules/junk")).unwrap();
        fs::create_dir_all(ws.join(".identra")).unwrap();
        fs::write(ws.join("README.md"), "hello identra").unwrap();
        fs::write(ws.join("src/auth.rs"), "fn verify_cookie() {}\n").unwrap();
        fs::write(ws.join("node_modules/junk/找.txt"), "verify_cookie").unwrap();
        fs::write(base.join("outside.txt"), "x").unwrap();

        let rows = list(&ws, "").unwrap();
        let names: Vec<&str> = rows.iter().map(|e| e.name.as_str()).collect();
        // Folders first, hidden gone. node_modules is listed (browsing it is the user's right),
        // it is only the search that skips it.
        assert_eq!(names, ["node_modules", "src", "README.md"]);
        assert!(rows[1].dir && !rows[2].dir);

        let sub = list(&ws, "src").unwrap();
        assert_eq!(sub[0].path, "src/auth.rs");

        assert!(
            list(&ws, "../").is_err(),
            "the listing cannot leave the root"
        );
        assert!(list(&ws, "nope").is_err());

        // A name hit and a content hit, and the dependency tree's copy is not in the answers.
        let by_name = search(&ws, "AUTH", 50);
        assert_eq!(by_name.len(), 1);
        assert_eq!(by_name[0].path, "src/auth.rs");
        assert_eq!(by_name[0].line, None);

        let by_content = search(&ws, "verify_cookie", 50);
        assert_eq!(by_content.len(), 1, "node_modules stays out of it");
        assert_eq!(by_content[0].line, Some(1));
        assert!(by_content[0]
            .snippet
            .as_deref()
            .unwrap()
            .contains("verify_cookie"));

        // The cap is a hard stop, not a suggestion.
        for i in 0..30 {
            fs::write(ws.join(format!("cap-{i}.txt")), "needle").unwrap();
        }
        assert_eq!(search(&ws, "needle", 10).len(), 10);

        assert!(
            search(&ws, "  ", 10).is_empty(),
            "a blank query finds nothing"
        );

        fs::remove_dir_all(&base).unwrap();
    }
}
