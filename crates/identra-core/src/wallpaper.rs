//! The shared wallpaper library.
//!
//! One directory of images under the user's data dir, shared by every workspace, so an image
//! added once can back any canvas. A workspace's choice is a path into this directory, stored on
//! its `Canvas`; the library itself is nothing but the files. No index, no metadata: the
//! directory listing is the truth, and a removed file simply stops being offered.

use std::hash::{Hash, Hasher};
use std::io;
use std::path::{Path, PathBuf};

/// Extensions the library accepts and lists. The webview resolves the content type from the
/// extension, so an unknown one would be a file the picker offers and the canvas cannot draw.
const IMAGE_EXTS: [&str; 4] = ["png", "jpg", "jpeg", "webp"];

/// Where the library lives: `~/.local/share/identra/wallpapers`, next to the recents file. The
/// path is fixed rather than configurable because the webview's asset scope in `tauri.conf.json`
/// names it too, and a library that moved out from under that scope would list images the canvas
/// is forbidden to load.
pub fn library() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    Some(
        Path::new(&home)
            .join(".local")
            .join("share")
            .join("identra")
            .join("wallpapers"),
    )
}

fn is_image(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| IMAGE_EXTS.contains(&e.to_ascii_lowercase().as_str()))
}

/// Every image in the library, sorted by name. A directory that does not exist yet is an empty
/// library, not an error: nothing has been added.
pub fn list(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file() && is_image(p))
        .collect();
    out.sort();
    out
}

/// Copy an image the user picked into the library and return where it landed.
///
/// The copy is the point: a canvas that referenced the original path would break the day that
/// file moved, and would quietly leak a path from anywhere on disk into a canvas.json someone
/// might export and share. Named by a hash of the bytes so adding the same picture twice lands on
/// the same file instead of growing the library. The hash is not stable across Rust releases,
/// which costs nothing here: the worst case is a duplicate after an upgrade, not a broken
/// reference, because every canvas stores the path it was actually given.
pub fn add(dir: &Path, source: &Path) -> io::Result<PathBuf> {
    if !is_image(source) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "that file is not a png, jpg, or webp image",
        ));
    }
    let bytes = std::fs::read(source)?;
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    bytes.hash(&mut hasher);
    // The extension is lowercased so the same image as photo.PNG and photo.png is one entry.
    let ext = source
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .unwrap_or_else(|| "png".into());
    let target = dir.join(format!("{:016x}.{ext}", hasher.finish()));
    if !target.exists() {
        std::fs::create_dir_all(dir)?;
        // Written whole rather than fs::copy, because the bytes are already in hand for the hash
        // and one write path is one thing to reason about.
        std::fs::write(&target, bytes)?;
    }
    Ok(target)
}

/// Remove an image from the library.
///
/// The path is only trusted after it proves it names a file directly inside the library. This
/// command is reachable from the window, and a window that can delete an arbitrary path is a
/// window that can delete the user's files, so the check is the feature: the file's parent must
/// be the library directory itself, compared canonicalized so `..` segments and symlinks cannot
/// dress an outside path up as an inside one.
pub fn remove(dir: &Path, path: &Path) -> io::Result<()> {
    let file = path.canonicalize()?;
    let library = dir.canonicalize()?;
    if file.parent() != Some(library.as_path()) {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "that file is not in the wallpaper library, so it is not the library's to remove",
        ));
    }
    std::fs::remove_file(&file)
    // A canvas somewhere may still reference the removed file. That is fine by design: the
    // frontend draws a missing image as the plain background, so the cost of removing a wallpaper
    // another workspace uses is that workspace going plain, not an error.
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The library round trip: adding copies the file in under a stable name, adding the same
    /// image again is a no-op, listing sees only images, and remove refuses to reach outside the
    /// library. The refusal is the part that must never regress, because remove is reachable from
    /// the window and the parent check is all that stands between it and the user's files.
    #[test]
    fn the_library_owns_its_files_and_nothing_else() {
        let root = std::env::temp_dir().join(format!("identra-wall-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let lib = root.join("wallpapers");
        let outside = root.join("outside");
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(outside.join("photo.PNG"), b"not really a png").unwrap();
        std::fs::write(outside.join("notes.txt"), b"words").unwrap();

        assert_eq!(
            list(&lib),
            Vec::<PathBuf>::new(),
            "no dir is an empty library"
        );

        let added = add(&lib, &outside.join("photo.PNG")).unwrap();
        assert_eq!(
            added.extension().unwrap(),
            "png",
            "the extension is normalized"
        );
        let again = add(&lib, &outside.join("photo.PNG")).unwrap();
        assert_eq!(added, again, "the same bytes land on the same file");
        assert_eq!(list(&lib), vec![added.clone()]);

        assert!(
            add(&lib, &outside.join("notes.txt")).is_err(),
            "only image files are accepted"
        );

        // A file next to the library, dressed up with a traversal. The canonicalized parent check
        // is what catches it.
        let smuggled = lib.join("..").join("outside").join("photo.PNG");
        assert!(
            remove(&lib, &smuggled).is_err(),
            "remove must not reach outside the library"
        );
        assert!(outside.join("photo.PNG").exists());

        remove(&lib, &added).unwrap();
        assert_eq!(list(&lib), Vec::<PathBuf>::new());

        std::fs::remove_dir_all(&root).unwrap();
    }
}
