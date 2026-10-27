//! Writing a file the user asked for, without following a link at its path.
//!
//! The cache and the report go where `--out` and `--report` say, which is by
//! default inside the project's `target/`. A project taken from elsewhere can
//! hold a link there, and `std::fs::write` would follow it and overwrite
//! whatever it names (0048). The text is written to a new file beside the
//! target and renamed over it: `create_new` refuses a link planted at the
//! temporary name, and a rename replaces a link at the target instead of
//! writing through it.

use std::io::Write;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

/// Writes `text` to `path`, replacing whatever is there, a link included.
/// Creates the folders above it.
pub fn replace(path: &Path, text: &str) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    }
    // Named per write, so two writes of one path at once do not share a file.
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(format!(".tmp-{}-{n}", std::process::id()));
    let tmp = path.with_file_name(name);
    // A failure to open leaves the name alone: it may be a file this call did
    // not create, which is the very case `create_new` refuses.
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&tmp)
        .map_err(|e| format!("cannot write {}: {e}", path.display()))?;
    let written = file.write_all(text.as_bytes());
    // Closed before the rename: Windows will not move an open file.
    drop(file);
    written
        .and_then(|()| std::fs::rename(&tmp, path))
        .map_err(|e| {
            // A temporary file left in `target/` is noise; best effort.
            let _ = std::fs::remove_file(&tmp);
            format!("cannot write {}: {e}", path.display())
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(test: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("collin-output-{test}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn it_creates_the_folders_and_replaces_the_file() {
        let d = dir("plain");
        let path = d.join("a/b/out.json");
        replace(&path, "one").unwrap();
        replace(&path, "two").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "two");
        let left: Vec<_> = std::fs::read_dir(d.join("a/b")).unwrap().collect();
        assert_eq!(left.len(), 1, "no temporary file stays: {left:?}");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[cfg(unix)]
    #[test]
    fn a_link_at_the_path_is_replaced_and_what_it_names_is_left_alone() {
        let d = dir("link");
        let victim = d.join("victim.txt");
        std::fs::write(&victim, "mine").unwrap();
        let path = d.join("out.json");
        std::os::unix::fs::symlink(&victim, &path).unwrap();
        replace(&path, "{}").unwrap();
        assert_eq!(std::fs::read_to_string(&victim).unwrap(), "mine");
        assert!(!std::fs::symlink_metadata(&path).unwrap().file_type().is_symlink());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{}");
        let _ = std::fs::remove_dir_all(&d);
    }

    /// Planted at the temporary name, a file is refused and left alone.
    #[test]
    fn a_file_at_the_temporary_name_is_not_removed() {
        let d = dir("planted");
        let path = d.join("out.json");
        // The names are `out.json.tmp-<pid>-<n>`: plant every name the counter can reach in a test run.
        for n in 0..4096 {
            std::fs::write(d.join(format!("out.json.tmp-{}-{n}", std::process::id())), "theirs").unwrap();
        }
        assert!(replace(&path, "{}").is_err());
        let kept = std::fs::read_dir(&d).unwrap().filter(|e| {
            let p = e.as_ref().unwrap().path();
            p.to_string_lossy().contains(".tmp-") && std::fs::read_to_string(p).unwrap() == "theirs"
        });
        assert_eq!(kept.count(), 4096);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_failed_write_leaves_no_temporary_file() {
        let d = dir("fail");
        // A folder at the target: the rename cannot replace it.
        let path = d.join("out.json");
        std::fs::create_dir_all(path.join("inner")).unwrap();
        assert!(replace(&path, "{}").is_err());
        let names: Vec<_> = std::fs::read_dir(&d).unwrap().map(|e| e.unwrap().file_name()).collect();
        assert_eq!(names.len(), 1, "{names:?}");
        let _ = std::fs::remove_dir_all(&d);
    }
}
