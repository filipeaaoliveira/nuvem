//! Writes a log dump to disk. Deliberately knows nothing about app state,
//! config, or clocks: the caller supplies the destination directory, the
//! filename timestamp, and the already-rendered, already-sanitized lines, so
//! this module is exercisable against a plain temp directory.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// Writes `lines` (one per line, trailing newline) to
/// `<dir>/logs-<timestamp>.log`, creating `dir` and any missing parents.
/// Returns the absolute path of the file written, so callers can report where
/// the dump landed regardless of the process's working directory.
pub fn write_log_dump(dir: &Path, timestamp: &str, lines: &[String]) -> io::Result<PathBuf> {
    fs::create_dir_all(dir)?;
    let path = dir.join(format!("logs-{timestamp}.log"));
    let mut contents = lines.join("\n");
    if !contents.is_empty() {
        contents.push('\n');
    }
    fs::write(&path, contents)?;
    // `absolute` normalizes without touching the filesystem (no symlink
    // resolution), so the reported path still reads the way the user's config
    // spelled it.
    std::path::absolute(&path).or(Ok(path))
}
