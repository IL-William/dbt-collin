//! Stamps the binary with the commit it was built from.
//!
//! The version in `Cargo.toml` moves only at a release, so two builds between
//! releases print the same one. The commit tells them apart, and survives in
//! the checkout `cargo install --git` builds from, which keeps its `.git`. A
//! tree with no repository of its own prints the version alone.
//!
//! The short hash rather than `git describe`: between a bump and its tag,
//! describe names the previous tag, and `collin 0.2.0 (v0.1.0-1-g...)` says two
//! versions at once.

use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let commit = repository(&root).map(|repo| stamp(&root, &repo)).unwrap_or_default();
    println!("cargo:rustc-env=COLLIN_COMMIT={commit}");
}

/// The repository's own git directories, or nothing when the workspace is not
/// the top of one. Without that check git walks up, and a tree copied into
/// another repository would print that repository's commit.
struct Git {
    dir: PathBuf,
    common: PathBuf,
}

fn repository(root: &Path) -> Option<Git> {
    let out = git(root, &["rev-parse", "--path-format=absolute", "--show-toplevel", "--git-dir", "--git-common-dir"])?;
    let mut lines = out.lines().map(PathBuf::from);
    let (top, dir, common) = (lines.next()?, lines.next()?, lines.next()?);
    if top.canonicalize().ok()? != root.canonicalize().ok()? {
        return None;
    }
    Some(Git { dir, common })
}

fn stamp(root: &Path, repo: &Git) -> String {
    // A worktree keeps HEAD and its index under its own git directory, and the
    // refs under the common one, so `.git` alone would be a file and watch
    // nothing. Only paths that exist: cargo treats a missing one as changed,
    // which would rebuild the crate on every invocation.
    let watched = [
        repo.dir.join("HEAD"),
        repo.dir.join("index"),
        repo.common.join("refs"),
        repo.common.join("packed-refs"),
        // An edit to a source changes neither HEAD nor the index, so without
        // these `-dirty` would stay whatever the last rerun found.
        root.join("crates"),
        root.join("Cargo.toml"),
        root.join("Cargo.lock"),
    ];
    for path in watched.iter().filter(|p| p.exists()) {
        println!("cargo:rerun-if-changed={}", path.display());
    }
    let Some(hash) = git(root, &["rev-parse", "--short", "HEAD"]) else {
        return String::new();
    };
    // Without the optional locks status leaves the index alone. Rewriting it
    // would touch a watched path and rerun this script on the next build.
    let dirty = git(root, &["--no-optional-locks", "status", "--porcelain", "--untracked-files=no"])
        .is_some_and(|s| !s.is_empty());
    if dirty {
        format!("{hash}-dirty")
    } else {
        hash
    }
}

fn git(root: &Path, args: &[&str]) -> Option<String> {
    Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
}
