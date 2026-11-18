//! Where the remembered-view map is written.
//!
//! A trait rather than a direct call to [`crate::config::state::save`], for the
//! same reason [`super::clipboard::ClipboardWriter`] is one: it lets the
//! reducer-to-runtime path be tested without a filesystem, so the tests that
//! matter — that a save happens exactly once, and that a failure latches — do
//! not depend on being able to create an unwritable directory.

use std::path::PathBuf;

use crate::app::state::remembered_view::RememberedViews;
use crate::config::state;

pub trait RememberedViewStore: Send + Sync {
    fn save(&self, views: &RememberedViews) -> Result<(), String>;
}

/// The production store: the state file beside the config file in use.
#[derive(Debug, Clone)]
pub struct FileRememberedViewStore {
    path: PathBuf,
}

impl FileRememberedViewStore {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }
}

impl RememberedViewStore for FileRememberedViewStore {
    fn save(&self, views: &RememberedViews) -> Result<(), String> {
        // The path is part of the message, not dropped: the user is told about
        // this failure exactly once per session (the reducer latches), so that
        // one line has to say which file could not be written — a `--config`
        // override means it is not necessarily the one they would guess.
        state::save(&self.path, views)
            .map_err(|err| format!("{}: {}", err.path.display(), err.message))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The user is told about a failed save once and never again, so that one
    /// status line has to name the file. Dropping the path leaves them with
    /// "File exists (os error 17)" and nothing to act on.
    #[test]
    fn a_failed_save_reports_the_path_it_could_not_write() {
        let dir = tempfile::tempdir().expect("temp dir");
        // A regular file where the state file's parent directory should be:
        // `create_dir_all` fails, so the save fails without needing permission
        // games that behave differently under root.
        let blocker = dir.path().join("blocker");
        std::fs::write(&blocker, "not a directory").expect("write blocker");
        let path = blocker.join("state.toml");
        let store = FileRememberedViewStore::new(path.clone());

        let message = store
            .save(&RememberedViews::default())
            .expect_err("saving under a regular file must fail");

        assert!(
            message.contains(&path.display().to_string()),
            "the failure must name the file it could not write, got {message:?}"
        );
    }
}
