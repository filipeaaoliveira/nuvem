//! Durable storage for state the application learns, separate from the config
//! the user writes by hand.
//!
//! This module is a durability mechanism and nothing else. It is generic over
//! the payload — it has no idea what a "last view" or a "favorite" is — so the
//! features that need memory across sessions supply their own type and this
//! module only promises to store it and hand it back. Deliberately: a writer
//! that ships with its first consumer bolted on is how the five decisions
//! below get skipped.
//!
//! # The five decisions
//!
//! Each of these is a place persistence can fail silently, so each is settled
//! here rather than left for the first consumer to improvise.
//!
//! **1. Separation — state lives in its own file.** State goes to
//! [`default_state_path`] (`state.toml`), never into the user's `config.toml`.
//! Rewriting a hand-edited TOML would clobber their comments, key order and
//! formatting; no serializer round-trips those, so an automatic rewrite
//! silently destroys work the user did by hand. The state file is the *sibling
//! of the config file in use*: [`state_path_for`] derives it from whatever
//! config path the caller resolved, which is how a `--config <path>` override
//! is honoured, and [`default_state_path`] is the no-override convenience that
//! applies it to [`crate::config::default_config_path`]. There is one
//! derivation, so the two paths cannot drift and the repo keeps exactly one
//! path convention. An XDG state directory was considered and
//! rejected: `dirs::state_dir()` is `None` on macOS and Windows, so it buys a
//! three-way platform-dependent chain, and it would put state somewhere
//! `nuvem info` does not print.
//!
//! **2. Atomicity — temp file plus rename.** [`save`] writes to a sibling
//! `state.toml.tmp.<pid>.<n>`, fsyncs it, renames it over the destination,
//! then fsyncs the directory. A crash therefore leaves either the old state or
//! the new state, never a half-written file. Three details carry weight. The
//! temp file is a *sibling*, so the rename cannot cross a filesystem boundary
//! — a cross-device rename fails, defeating the point. Its name carries both
//! the process id and a per-save counter: the pid keeps two nuvem instances
//! apart, and the counter keeps two saves inside one instance apart, which the
//! pid alone cannot do. Sharing a staging file would let two writers interleave
//! and the rename would then publish the mixture atomically — a corrupt state
//! file delivered by the very mechanism meant to prevent one. And the
//! *directory* fsync is not optional: without it the rename can be lost on a
//! crash even though the file's own contents were synced, which reintroduces
//! exactly the hole the dance exists to close. One consequence is accepted
//! knowingly: a process killed between creating the temp file and the rename
//! leaves a `state.toml.tmp.<pid>.<n>` behind, and nothing ever reaps it. That
//! is directory clutter, not a correctness problem — [`load`] reads the exact
//! destination path and never scans siblings. A reaper was deliberately not
//! added: a glob-based sweep of `state.toml.tmp.*` cannot tell debris from a
//! concurrent writer's live staging file, so it would delete the bytes another
//! process is about to rename into place — a worse failure than the clutter it
//! cleans up.
//!
//! **3. Unwritable path — degrade loudly.** A read-only filesystem, wrong
//! permissions or a missing parent directory must never panic and must never
//! pass silently. A missing parent is created; anything else becomes a
//! [`ConfigProblemSource::State`] warning through the existing
//! [`crate::config::problems`] mechanism, via [`load_or_default`] and
//! [`save_reporting_problems`]. The app keeps running on defaults. Losing
//! persisted state is not worth failing the app over — but it is always worth
//! saying out loud, because a feature built on state that silently never saves
//! is indistinguishable from a feature that is broken.
//!
//! **4. Corrupt or future-version state — ignore and warn.** The file carries
//! a top-level `version`, mirroring the config file's own convention: a
//! missing version reads as the current one, and a version above [`VERSION`]
//! is refused. The version is parsed *before* the payload, because a payload
//! written by a newer nuvem may well deserialize cleanly into this build's
//! shape while meaning something else entirely — checking the payload first
//! would adopt it. This is what stops a state file written by a newer nuvem
//! from breaking an older one: the older build ignores it, warns, and starts
//! from defaults rather than misreading it or crashing.
//!
//! **5. Concurrent instances — last write wins, deliberately.** Two nuvem
//! processes on one machine will race, and this module does not serialize
//! them. That is a decision, not an oversight. Because every write is a rename
//! of a fully-written file, a reader never observes a torn or partial state
//! file no matter how the writes interleave; the only possible loss is one
//! instance's update being overwritten by another's. For the state this is
//! meant to hold — the last view, favourites, cached query results — losing
//! the older instance's update is acceptable and is what most terminal
//! applications do. An advisory lock file was considered and rejected: it
//! would trade this bounded, benign loss for a stale-lock failure mode where a
//! crashed process leaves a lock that prevents the *next* launch from saving
//! anything at all, which is strictly worse than the problem it solves. If a
//! future consumer genuinely cannot tolerate a lost update, the fix is a
//! read-modify-write with a compare-and-swap on that consumer's own field, not
//! a global lock.
//!
//! # Relationship to `--read-only`
//!
//! [`crate::config::ResolvedConfig::read_only`] does **not** suppress state
//! writes. It is documented as a promise about cloud-resource mutations only
//! and explicitly does not claim the app makes no local changes — activating a
//! gcloud configuration, opening tunnels and writing the clipboard all still
//! work under it. Writing this file is a local change of the same kind.

use std::fs;
use std::io;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::config::problems::{ConfigProblemSource, ConfigProblems};

/// The envelope version this build writes, and the highest it can read.
pub const VERSION: u64 = 1;

/// Why a state file could not be read or written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StateError {
    pub path: PathBuf,
    pub message: String,
}

impl StateError {
    fn new(path: &Path, message: impl Into<String>) -> Self {
        Self {
            path: path.to_path_buf(),
            message: message.into(),
        }
    }
}

/// The on-disk envelope as written: the version, then the payload table.
#[derive(Serialize)]
struct Envelope<'a, T> {
    version: u64,
    data: &'a T,
}

/// The envelope as read back.
#[derive(Deserialize)]
struct StoredEnvelope<T> {
    data: T,
}

/// The envelope's version alone, read before the payload so that a file from a
/// newer nuvem is reported as a version we cannot read rather than as whatever
/// the payload happens to mis-parse into.
#[derive(Deserialize)]
struct VersionProbe {
    version: Option<u64>,
}

/// Persist `data` to `path`, replacing whatever is there.
///
/// The write is atomic: the bytes land in a sibling temp file which is then
/// renamed over the destination, so a crash can leave the old state or the new
/// state but never a half-written file. See the module docs for why the temp
/// file is a sibling and why the directory is synced after the rename.
pub fn save<T: Serialize>(path: &Path, data: &T) -> Result<(), StateError> {
    let rendered = toml::to_string(&Envelope {
        version: VERSION,
        data,
    })
    .map_err(|err| StateError::new(path, err.to_string()))?;

    let parent = path
        .parent()
        .ok_or_else(|| StateError::new(path, "state path has no parent directory"))?;
    // A missing parent is one of the unwritable-path cases: create it rather
    // than fail, and let a genuinely unwritable location fail below.
    fs::create_dir_all(parent).map_err(|err| StateError::new(path, err.to_string()))?;

    // A sibling, so the rename stays within one filesystem: a cross-device
    // rename fails, which would defeat the whole dance. See `temp_path` for how
    // two concurrent savers are kept off each other's staging file.
    let temp = temp_path(path, parent);

    if let Err(err) = write_all_synced(&temp, rendered.as_bytes()) {
        let _ = fs::remove_file(&temp);
        return Err(StateError::new(path, err.to_string()));
    }

    if let Err(err) = fs::rename(&temp, path) {
        let _ = fs::remove_file(&temp);
        return Err(StateError::new(path, err.to_string()));
    }

    // Sync the directory so the rename itself survives a crash; without this
    // the durable temp file can be renamed into a directory entry that is
    // still only in the page cache, losing the update the fsync above bought.
    //
    // Deliberately best-effort: the rename has already succeeded, so the state
    // is written and readable. Failing the save here would report a loss that
    // did not happen.
    if let Ok(handle) = fs::File::open(parent) {
        let _ = handle.sync_all();
    }

    Ok(())
}

/// The sibling temp path a save stages its bytes in.
fn temp_path(path: &Path, parent: &Path) -> PathBuf {
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "state.toml".to_string());
    // The process id keeps two nuvem instances apart; the counter keeps two
    // saves inside one instance apart, which the pid alone cannot do.
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let ticket = NEXT.fetch_add(1, Ordering::Relaxed);
    parent.join(format!("{name}.tmp.{}.{ticket}", std::process::id()))
}

/// Write `bytes` to `path` and fsync them, so the rename that follows promotes
/// durable content rather than a page-cache promise.
fn write_all_synced(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut handle = fs::File::create(path)?;
    handle.write_all(bytes)?;
    handle.sync_all()
}

/// Read the state stored at `path`.
pub fn load<T: DeserializeOwned>(path: &Path) -> Result<Option<T>, StateError> {
    let contents = match crate::config::read::read_config_file(path) {
        Ok(Some(contents)) => contents,
        Ok(None) => return Ok(None),
        Err(message) => return Err(StateError::new(path, message)),
    };

    // Version first: a payload written by a newer nuvem may parse cleanly into
    // this build's shape while meaning something else entirely.
    let probe = toml::from_str::<VersionProbe>(&contents)
        .map_err(|err| StateError::new(path, err.message().to_string()))?;
    // A missing version is read as the current one, matching the config
    // file's forward-friendly convention.
    let version = probe.version.unwrap_or(VERSION);
    if version > VERSION {
        return Err(StateError::new(
            path,
            format!(
                "state version {version} was written by a newer nuvem (this build reads {VERSION}); ignoring it and starting from defaults"
            ),
        ));
    }

    toml::from_str::<StoredEnvelope<T>>(&contents)
        .map(|envelope| Some(envelope.data))
        .map_err(|err| StateError::new(path, err.message().to_string()))
}

/// The `state.toml` sitting beside `config_path`.
///
/// This is how a caller honours a `--config <path>` override: state follows the
/// config file actually in use, not the default location.
pub fn state_path_for(config_path: &Path) -> PathBuf {
    match config_path.parent() {
        Some(directory) => directory.join(STATE_FILE_NAME),
        None => PathBuf::from(STATE_FILE_NAME),
    }
}

/// `dirs::config_dir()/nuvem/state.toml`, the state path when no `--config`
/// override is in play.
///
/// A separate *file*, not a separate directory: see decision 1 in the module
/// docs for why the user's `config.toml` is never rewritten.
pub fn default_state_path() -> PathBuf {
    // Derived from the config path rather than rebuilding its fallback chain,
    // so the two cannot drift apart if that chain is ever changed.
    state_path_for(&crate::config::default_config_path())
}

/// The state file's name, alongside the config file's `config.toml`.
const STATE_FILE_NAME: &str = "state.toml";

/// Read the state at `path`, degrading to `T::default()` and a warning in
/// `problems` when it cannot be used.
///
/// An absent file is the ordinary first-run case and reports nothing. A
/// corrupt file, an unreadable one, and one written by a newer nuvem all
/// degrade the same way: defaults, plus a warning naming the file.
pub fn load_or_default<T: DeserializeOwned + Default>(
    path: &Path,
    problems: &mut ConfigProblems,
) -> T {
    match load::<T>(path) {
        Ok(Some(state)) => state,
        Ok(None) => T::default(),
        Err(err) => {
            problems.push(
                ConfigProblemSource::State,
                format!(
                    "{}: {}; starting from defaults",
                    err.path.display(),
                    err.message
                ),
            );
            T::default()
        }
    }
}

/// Persist `data` to `path`, degrading to a warning in `problems` when the
/// write fails. Losing persisted state is never worth failing the app over,
/// but it is always worth saying out loud.
pub fn save_reporting_problems<T: Serialize>(path: &Path, data: &T, problems: &mut ConfigProblems) {
    if let Err(err) = save(path, data) {
        problems.push(
            ConfigProblemSource::State,
            format!(
                "{}: {}; state was not saved",
                err.path.display(),
                err.message
            ),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;

    /// A payload standing in for whatever a future feature persists. The
    /// writer is deliberately generic, so this test-local type exercises it
    /// without the module knowing about any feature.
    #[derive(Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
    struct Payload {
        last_view: String,
        count: u32,
    }

    #[test]
    fn round_trips_a_payload_through_a_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.toml");
        let written = Payload {
            last_view: "compute".to_string(),
            count: 7,
        };

        save(&path, &written).unwrap();
        let read: Option<Payload> = load(&path).unwrap();

        assert_eq!(read, Some(written));
    }

    #[test]
    fn a_state_file_from_a_newer_nuvem_is_refused_rather_than_misread() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.toml");
        // Deliberately parseable as a Payload: the version, not the payload
        // shape, is what has to stop this being adopted.
        fs::write(
            &path,
            "version = 2\n[data]\nlast_view = \"compute\"\ncount = 7\n",
        )
        .unwrap();

        let outcome = load::<Payload>(&path);

        assert!(
            outcome.is_err(),
            "a version this build cannot read must be refused, got {outcome:?}"
        );
        let message = outcome.unwrap_err().message;
        assert!(
            message.contains('2'),
            "the warning should name the version found, got {message:?}"
        );
    }

    /// A crash between "destination truncated" and "bytes written" is what
    /// leaves a half-written state file. Replacing the destination by rename
    /// closes that window: the old inode stays complete and readable until the
    /// new one is fully written, so a reader mid-save sees one or the other and
    /// never a truncated file. A same-inode replacement would mean the bytes
    /// were written over the destination in place, which is exactly the hole.
    #[test]
    fn saving_replaces_the_file_rather_than_truncating_it_in_place() {
        use std::os::unix::fs::MetadataExt;

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.toml");

        save(&path, &Payload::default()).unwrap();
        let first = fs::metadata(&path).unwrap().ino();

        save(
            &path,
            &Payload {
                last_view: "storage".to_string(),
                count: 1,
            },
        )
        .unwrap();
        let second = fs::metadata(&path).unwrap().ino();

        assert_ne!(
            first, second,
            "the destination was written in place, so a crash mid-write could tear it"
        );
    }

    /// The temp file is an implementation detail of the atomic swap and must
    /// not survive a successful save, or the state directory accumulates one
    /// per write.
    #[test]
    fn a_successful_save_leaves_no_temporary_file_behind() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.toml");

        save(&path, &Payload::default()).unwrap();

        let leftovers: Vec<_> = fs::read_dir(dir.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .filter(|name| name != "state.toml")
            .collect();
        assert!(
            leftovers.is_empty(),
            "a successful save should leave only the state file, found {leftovers:?}"
        );
    }

    /// A previous process that died mid-save leaves its temp file behind. That
    /// debris must not change what a later read returns.
    #[test]
    fn a_temp_file_left_by_a_crashed_write_does_not_affect_the_next_read() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.toml");
        let good = Payload {
            last_view: "compute".to_string(),
            count: 7,
        };
        save(&path, &good).unwrap();
        fs::write(
            dir.path().join("state.toml.tmp.999999"),
            "{ truncated garbage",
        )
        .unwrap();

        let read: Option<Payload> = load(&path).unwrap();

        assert_eq!(read, Some(good));
    }

    /// The four failure paths below are the ones that must degrade loudly:
    /// defaults plus a warning through `problems.rs`, never a panic and never
    /// silence.
    #[test]
    fn a_corrupt_state_file_degrades_to_defaults_with_a_warning() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.toml");
        fs::write(&path, "this is : not = toml [[[").unwrap();
        let mut problems = ConfigProblems::default();

        let read: Payload = load_or_default(&path, &mut problems);

        assert_eq!(
            read,
            Payload::default(),
            "a corrupt file must not be adopted"
        );
        assert_eq!(
            problems.iter().map(|p| p.source).collect::<Vec<_>>(),
            vec![ConfigProblemSource::State],
            "the corruption must be surfaced through problems.rs, not swallowed"
        );
    }

    #[test]
    fn a_future_version_degrades_to_defaults_with_a_warning() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.toml");
        fs::write(
            &path,
            "version = 999\n[data]\nlast_view = \"compute\"\ncount = 7\n",
        )
        .unwrap();
        let mut problems = ConfigProblems::default();

        let read: Payload = load_or_default(&path, &mut problems);

        assert_eq!(read, Payload::default());
        assert_eq!(problems.len(), 1, "a future version must warn exactly once");
        assert!(
            problems.messages()[0].contains("999"),
            "the warning should name the version found, got {:?}",
            problems.messages()[0]
        );
    }

    /// "Absent" and "there but unreadable" must not collapse into each other.
    /// Absent is a silent first run; unreadable is a real problem, and
    /// reporting it as absent would hide a permissions fault behind the
    /// ordinary empty-state path — state would silently never load and the
    /// feature above it would look merely forgetful.
    ///
    /// Relies on the process not being root: root ignores mode bits, so the
    /// read would succeed and this test would FAIL rather than quietly pass —
    /// the failure mode is loud. CI runs non-root on ubuntu-latest.
    #[test]
    fn an_unreadable_state_file_warns_rather_than_looking_absent() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.toml");
        save(&path, &Payload::default()).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o000)).unwrap();
        let mut problems = ConfigProblems::default();

        let read: Payload = load_or_default(&path, &mut problems);

        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        assert_eq!(read, Payload::default());
        assert_eq!(
            problems.iter().map(|p| p.source).collect::<Vec<_>>(),
            vec![ConfigProblemSource::State],
            "an unreadable state file must warn, not pass as a first run"
        );
    }

    /// An absent state file is the ordinary first-run case, not a problem.
    #[test]
    fn a_missing_state_file_is_silent() {
        let dir = tempfile::tempdir().unwrap();
        let mut problems = ConfigProblems::default();

        let read: Payload = load_or_default(&dir.path().join("absent.toml"), &mut problems);

        assert_eq!(read, Payload::default());
        assert!(
            problems.is_empty(),
            "a first run has no state yet; that is not a problem to report"
        );
    }

    /// Relies on the process not being root: root ignores mode bits, so the
    /// write would succeed and this test would FAIL rather than quietly pass —
    /// the failure mode is loud. CI runs non-root on ubuntu-latest.
    #[test]
    fn an_unwritable_directory_warns_rather_than_panicking() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let locked = dir.path().join("locked");
        fs::create_dir(&locked).unwrap();
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o500)).unwrap();
        let path = locked.join("state.toml");
        let mut problems = ConfigProblems::default();

        save_reporting_problems(&path, &Payload::default(), &mut problems);

        // Restore before the assertions so a failure still cleans up.
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o700)).unwrap();
        assert_eq!(
            problems.iter().map(|p| p.source).collect::<Vec<_>>(),
            vec![ConfigProblemSource::State],
            "an unwritable state directory must be surfaced, not swallowed"
        );
    }

    #[test]
    fn a_successful_save_reports_no_problems() {
        let dir = tempfile::tempdir().unwrap();
        let mut problems = ConfigProblems::default();

        save_reporting_problems(
            &dir.path().join("state.toml"),
            &Payload::default(),
            &mut problems,
        );

        assert!(problems.is_empty(), "a clean save must not warn");
    }

    /// The process id separates two nuvem processes, but not two saves inside
    /// one — and nuvem is async, so two features saving at once is reachable.
    /// Sharing a staging file would let their writes interleave and the rename
    /// would then publish the mixture atomically, which is a corrupt state file
    /// arriving through the very mechanism meant to prevent one.
    #[test]
    fn two_saves_in_one_process_stage_through_different_temp_files() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.toml");

        let first = temp_path(&path, dir.path());
        let second = temp_path(&path, dir.path());

        assert_ne!(
            first, second,
            "concurrent saves in one process would share a staging file"
        );
    }

    /// Decision 1: state is never written into the file the user hand-edits,
    /// because rewriting it would clobber their comments and formatting.
    #[test]
    fn the_state_file_is_not_the_users_config_file() {
        assert_ne!(
            default_state_path(),
            crate::config::default_config_path(),
            "state must never be written into the config the user hand-edits"
        );
    }

    /// A `--config <path>` override moves the config file, so state has to move
    /// with it; deriving state from the default location instead would write it
    /// somewhere the caller never asked for.
    #[test]
    fn state_sits_beside_the_config_file_it_is_given() {
        let dir = tempfile::tempdir().unwrap();

        let state = state_path_for(&dir.path().join("config.toml"));

        assert_eq!(state, dir.path().join("state.toml"));
    }

    /// The default path must be the general derivation applied to the default
    /// config path, not a second copy of it that could drift.
    #[test]
    fn the_default_state_path_is_the_default_config_paths_sibling() {
        assert_eq!(
            default_state_path(),
            state_path_for(&crate::config::default_config_path())
        );
    }

    #[test]
    fn the_state_file_sits_alongside_the_config_file() {
        let state = default_state_path();

        assert_eq!(state.file_name().unwrap(), "state.toml");
        assert_eq!(
            state.parent(),
            crate::config::default_config_path().parent(),
            "state follows the config file's location convention"
        );
    }
}
