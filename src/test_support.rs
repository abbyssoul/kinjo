//! Shared helpers for the crate's unit tests.
//!
//! Several modules need scratch files and directories on disk. Centralising the
//! creation here keeps the per-module test code focused on behaviour and gives
//! every caller the same collision-proof naming scheme.

use std::{
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

/// Monotonic counter mixed into temp paths so two tests that run in the same
/// process at the same nanosecond still get distinct paths.
static COUNTER: AtomicU64 = AtomicU64::new(0);

fn unique_path(tag: &str, suffix: &str) -> PathBuf {
    let seq = COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "kinjo-test-{tag}-{}-{seq}{suffix}",
        std::process::id()
    ))
}

/// Create a fresh, empty scratch directory and return its path. The caller is
/// responsible for removing it (see [`remove`]).
pub fn temp_dir(tag: &str) -> PathBuf {
    let dir = unique_path(tag, "");
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Create a scratch `.toml` file with the given contents and return its path.
pub fn temp_file(tag: &str, contents: &str) -> PathBuf {
    let path = unique_path(tag, ".toml");
    std::fs::write(&path, contents).unwrap();
    path
}

/// Turn a POSIX-style path literal into one the host agrees is absolute.
///
/// Windows only counts a path as absolute once it names a volume, so a bare
/// `/xdg` is relative there. Tests that pin how absolute config homes are
/// handled would silently exercise the rejection branch instead.
pub fn absolute(path: &str) -> PathBuf {
    if cfg!(windows) {
        PathBuf::from(format!("C:{path}"))
    } else {
        PathBuf::from(path)
    }
}

/// Best-effort cleanup of a path created by [`temp_dir`] or [`temp_file`].
/// Tolerates either a file or a directory so callers need not track which.
pub fn remove(path: &std::path::Path) {
    let _ = std::fs::remove_file(path);
    let _ = std::fs::remove_dir_all(path);
}
