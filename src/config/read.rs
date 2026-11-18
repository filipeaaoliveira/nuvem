//! Reading a configuration file without trusting what is at the path.
//!
//! The three files nuvem reads at startup — the config, the plugin
//! declarations, and the remembered-view state — were each read straight into
//! memory with `read_to_string`. That is fine for a file and wrong for
//! anything else that can live at a path:
//!
//! * A **FIFO** never ends. `read_to_string` on one blocks until a writer
//!   appears and closes, so a named pipe left at `config.toml` hangs nuvem
//!   before it draws a frame, with no timeout and nothing on screen.
//! * A **very large file** is read whole before anything looks at it, so a
//!   multi-gigabyte file is a multi-gigabyte allocation during startup.
//!
//! Neither is a plausible accident, and neither needs an attacker: both are a
//! misconfiguration away, and both fail in a way that gives the operator
//! nothing to go on. The reader below refuses them by name instead.
//!
//! **Symlinks are followed, deliberately.** Dotfile managers (stow, chezmoi,
//! and hand-rolled ones) all symlink config files into place, so refusing
//! symlinks would refuse the normal setup of a large fraction of the people
//! who write a config at all. What is checked is what the link *resolves to*.
//!
//! **The check races the read, deliberately.** Stat-then-read can in principle
//! see a different file than it opens. Closing that window means opening
//! first and asking the handle — but opening is itself the hang: `open` on a
//! FIFO blocks waiting for a writer, before there is a handle to interrogate.
//! Since this guards against a static misconfiguration rather than a process
//! racing us on our own config directory, the order that avoids the hang is
//! the right one.

use std::path::Path;

/// The ceiling for any configuration file nuvem reads.
///
/// These are hand-written TOML documents; the largest realistic one is a few
/// kilobytes of themes and plugins. A megabyte is far enough above that to
/// never be reached by a real file, and low enough that hitting it costs
/// nothing.
pub const CONFIG_FILE_MAX_BYTES: u64 = 1024 * 1024;

/// Reads `path`, or explains why it will not.
///
/// `Ok(None)` means the file is absent, which every caller treats as "use
/// defaults" rather than as a problem. `Err` carries a sentence for the
/// operator; callers wrap it in whichever problem type they already report.
pub fn read_config_file(path: &Path) -> Result<Option<String>, String> {
    // `metadata` rather than `symlink_metadata`: see the module docs on why
    // links are followed and only their target is judged.
    let metadata = match std::fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(err.to_string()),
    };

    if !metadata.is_file() {
        // Named without guessing which kind it is: the operator can see that
        // for themselves, and the useful half of the sentence is that nuvem
        // will not read it.
        return Err("not an ordinary file; refusing to read it".to_string());
    }

    if metadata.len() > CONFIG_FILE_MAX_BYTES {
        return Err(format!(
            "{} bytes exceeds the {CONFIG_FILE_MAX_BYTES}-byte configuration ceiling; refusing to read it",
            metadata.len()
        ));
    }

    // No second NotFound arm here. A file that passed the stat above and was
    // gone by this read did vanish underneath us, and reporting that as "no
    // file, using defaults" would hide it; the operator gets the OS's sentence
    // instead.
    std::fs::read_to_string(path)
        .map(Some)
        .map_err(|err| err.to_string())
}

/// Whether `path` is writable by users other than its owner and group.
///
/// Only asked about the plugin file, and only on unix. A world-writable
/// `plugins.toml` means any local account can add a command that nuvem will
/// offer and, on confirmation, run — so it is refused rather than warned
/// about.
///
/// The file's own mode is the whole check. A world-writable *directory* is the
/// deeper hole (it lets a stranger replace the file outright, mode and all),
/// but the platform config directory is not somewhere a normal setup leaves
/// open, and walking every ancestor would turn a startup read into a policy
/// engine. Group-writable is likewise left alone: `umask 002` setups are
/// common and legitimate, and refusing them would refuse working machines.
/// One function with the platform split inside it, rather than a `cfg`'d pair.
/// A second function body under `#[cfg(not(unix))]` compiles on no platform
/// this project supports, so nothing could ever execute it — and a mutant
/// planted in it survives every test by construction.
pub fn is_world_writable(path: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(path)
            .map(|metadata| metadata.permissions().mode() & 0o002 != 0)
            .unwrap_or(false)
    }
    // Non-unix has no mode bits to read, so the plugin file carries no
    // permission refusal there.
    #[cfg(not(unix))]
    {
        let _ = path;
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// The ceiling's magnitude is part of the contract: it must sit far above
    /// any hand-written TOML and far below a size worth allocating blindly.
    /// Pinned against an independent literal, so a slipped operator turning
    /// 1 MiB into 2 KiB or a gigabyte fails by name.
    #[test]
    fn the_configuration_ceiling_is_one_mebibyte() {
        assert_eq!(CONFIG_FILE_MAX_BYTES, 1_048_576);
    }

    /// A stat failure that is not "absent" must stay an error: reading it as
    /// absence would silently drop a config the operator can see on disk.
    /// `file/child` is the everyday way to produce one — a path component
    /// that is not a directory.
    #[test]
    fn a_stat_failure_other_than_absence_is_reported() {
        let dir = tempfile::tempdir().expect("tempdir");
        let file = dir.path().join("config.toml");
        std::fs::write(&file, "").expect("write");

        let error = read_config_file(&file.join("child.toml"))
            .expect_err("a path under a regular file cannot be stat'd");

        assert!(!error.is_empty(), "the OS reason must be carried through");
    }

    #[test]
    fn an_absent_file_is_not_an_error() {
        let dir = tempfile::tempdir().expect("tempdir");

        assert_eq!(read_config_file(&dir.path().join("nothing.toml")), Ok(None));
    }

    #[test]
    fn an_ordinary_file_reads_back_whole() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "poll_interval_secs = 30\n").expect("write");

        assert_eq!(
            read_config_file(&path),
            Ok(Some("poll_interval_secs = 30\n".to_string()))
        );
    }

    /// A directory is the everyday non-file: `--config` pointed at a folder
    /// used to surface as a bare OS error, and now says what nuvem did.
    #[test]
    fn a_directory_is_refused_as_not_an_ordinary_file() {
        let dir = tempfile::tempdir().expect("tempdir");

        let error = read_config_file(dir.path()).expect_err("a directory is not readable as TOML");

        assert!(error.contains("not an ordinary file"), "{error}");
    }

    #[test]
    fn a_file_over_the_ceiling_is_refused_by_size() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("huge.toml");
        let mut handle = std::fs::File::create(&path).expect("create");
        handle
            .write_all(&vec![b'#'; CONFIG_FILE_MAX_BYTES as usize + 1])
            .expect("write");
        drop(handle);

        let error = read_config_file(&path).expect_err("over the ceiling");

        assert!(error.contains("ceiling"), "{error}");
    }

    /// The boundary is inclusive: a file *at* the ceiling is a file nuvem
    /// still reads, so the refusal cannot be triggered by a document that
    /// happens to land exactly on it.
    #[test]
    fn a_file_at_exactly_the_ceiling_is_still_read() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("big.toml");
        std::fs::write(&path, vec![b'#'; CONFIG_FILE_MAX_BYTES as usize]).expect("write");

        let contents = read_config_file(&path).expect("at the ceiling is not over it");

        assert_eq!(
            contents.map(|text| text.len()),
            Some(CONFIG_FILE_MAX_BYTES as usize)
        );
    }

    /// A symlink to a real config is the dotfile-manager case, and must read
    /// as the file it points at.
    #[cfg(unix)]
    #[test]
    fn a_symlink_to_an_ordinary_file_is_followed() {
        let dir = tempfile::tempdir().expect("tempdir");
        let target = dir.path().join("real.toml");
        let link = dir.path().join("config.toml");
        std::fs::write(&target, "theme = \"dark\"\n").expect("write");
        std::os::unix::fs::symlink(&target, &link).expect("symlink");

        assert_eq!(
            read_config_file(&link),
            Ok(Some("theme = \"dark\"\n".to_string()))
        );
    }

    /// The hang this module exists to prevent. Without the file-type check
    /// `read_to_string` parks here forever waiting for a writer; the assertion
    /// is that the call returns at all.
    #[cfg(unix)]
    #[test]
    fn a_fifo_is_refused_instead_of_blocking_forever() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("config.toml");
        let c_path = std::ffi::CString::new(path.as_os_str().as_encoded_bytes()).expect("cstring");
        // SAFETY: `mkfifo` takes a nul-terminated path and a mode and returns
        // a status; `c_path` outlives the call, and no pointer is retained.
        assert_eq!(unsafe { libc::mkfifo(c_path.as_ptr(), 0o644) }, 0, "mkfifo");

        let error = read_config_file(&path).expect_err("a FIFO is not a config file");

        assert!(error.contains("not an ordinary file"), "{error}");
    }

    #[cfg(unix)]
    #[test]
    fn an_ordinary_mode_is_not_world_writable() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("plugins.toml");
        std::fs::write(&path, "").expect("write");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).expect("chmod");

        assert!(!is_world_writable(&path));
    }

    #[cfg(unix)]
    #[test]
    fn a_world_writable_mode_is_reported() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("plugins.toml");
        std::fs::write(&path, "").expect("write");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o666)).expect("chmod");

        assert!(is_world_writable(&path));
    }

    /// Group-writable is deliberately allowed: `umask 002` is a normal
    /// configuration and refusing it would refuse working machines.
    #[cfg(unix)]
    #[test]
    fn a_group_writable_mode_is_allowed() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("plugins.toml");
        std::fs::write(&path, "").expect("write");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o664)).expect("chmod");

        assert!(!is_world_writable(&path));
    }
}
