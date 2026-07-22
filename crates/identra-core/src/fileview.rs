//! Reading a file for the viewer node.
//!
//! The path arrives from the window, and from there it can be an agent's word as easily as the
//! user's, so the containment check is the point of this module: only files that really resolve
//! inside the active workspace are readable, compared canonicalized so a symlink or a `..` can
//! not dress an outside file up as an inside one. Everything else is classification, so the
//! frontend can show a file as what it is instead of guessing.

use serde::Serialize;
use std::io;
use std::path::Path;

/// Bigger than this and the viewer shows a size, not the content. The node is for reading an
/// artifact, not for streaming a log; 2MB of text is already far past what anyone reads in a
/// tile, and the bytes cross the IPC boundary as JSON.
const MAX_BYTES: u64 = 2 * 1024 * 1024;

/// Extensions the webview can draw as an image. Judged by extension, same as the wallpaper
/// library: a wrong guess here shows a broken image state, not a security problem, because the
/// bytes never execute.
const IMAGE_EXTS: [&str; 6] = ["png", "jpg", "jpeg", "webp", "gif", "svg"];

/// What the viewer renders. Tagged for the frontend to switch on.
///
/// Image bytes ride as a plain byte vector, which serializes as a JSON array: heavy for what it
/// is, but local, one-shot, and capped by MAX_BYTES. The frontend turns them into a blob URL,
/// which keeps base64 out of both sides.
#[derive(Serialize, Debug, PartialEq)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum FileView {
    Text { name: String, text: String },
    Image { name: String, bytes: Vec<u8> },
    Binary { name: String, size: u64 },
    TooBig { name: String, size: u64 },
}

fn name_of(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

/// Read `path` for viewing, refusing anything that does not resolve inside `workspace`.
pub fn read(workspace: &Path, path: &Path) -> io::Result<FileView> {
    let file = path.canonicalize()?;
    let root = workspace.canonicalize()?;
    if !file.starts_with(&root) {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "that file is outside this workspace, so the viewer will not open it",
        ));
    }
    let name = name_of(&file);
    let size = std::fs::metadata(&file)?.len();
    if size > MAX_BYTES {
        return Ok(FileView::TooBig { name, size });
    }
    let bytes = std::fs::read(&file)?;
    let is_image = file
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| IMAGE_EXTS.contains(&e.to_ascii_lowercase().as_str()));
    if is_image {
        return Ok(FileView::Image { name, bytes });
    }
    match String::from_utf8(bytes) {
        Ok(text) => Ok(FileView::Text { name, text }),
        // Not UTF-8 is the honest definition of "not text" here. Lossy-decoding a database or an
        // executable into mojibake helps nobody; saying what it is does.
        Err(e) => Ok(FileView::Binary {
            name,
            size: e.as_bytes().len() as u64,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn the_viewer_reads_inside_the_workspace_and_nowhere_else() {
        let base = std::env::temp_dir().join(format!("identra-fv-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        let ws = base.join("ws");
        fs::create_dir_all(ws.join("docs")).unwrap();
        fs::write(base.join("secret.txt"), "not yours").unwrap();
        fs::write(ws.join("docs/report.md"), "# Findings\nplain text").unwrap();
        fs::write(ws.join("shot.png"), [0x89, 0x50, 0x4e, 0x47]).unwrap();
        fs::write(ws.join("blob.bin"), [0x00, 0xff, 0x00, 0x10]).unwrap();

        match read(&ws, &ws.join("docs/report.md")).unwrap() {
            FileView::Text { name, text } => {
                assert_eq!(name, "report.md");
                assert!(text.contains("Findings"));
            }
            other => panic!("expected text, got {other:?}"),
        }
        match read(&ws, &ws.join("shot.png")).unwrap() {
            FileView::Image { name, bytes } => {
                assert_eq!(name, "shot.png");
                assert_eq!(bytes.len(), 4);
            }
            other => panic!("expected image, got {other:?}"),
        }
        assert!(matches!(
            read(&ws, &ws.join("blob.bin")).unwrap(),
            FileView::Binary { .. }
        ));

        // The refusals are the feature. A traversal that resolves outside, and the same file
        // named directly: both denied, because an agent can say any path it likes.
        assert!(read(&ws, &ws.join("docs/../../secret.txt")).is_err());
        assert!(read(&ws, &base.join("secret.txt")).is_err());
        // A symlink inside the workspace pointing out resolves out, so it is out.
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(base.join("secret.txt"), ws.join("looks-local.txt"))
                .unwrap();
            assert!(read(&ws, &ws.join("looks-local.txt")).is_err());
        }
        // A missing file is an io error with the OS's own words, not a panic.
        assert!(read(&ws, &ws.join("nope.txt")).is_err());

        fs::remove_dir_all(&base).unwrap();
    }
}
