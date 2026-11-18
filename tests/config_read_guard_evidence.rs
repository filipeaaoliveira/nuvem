//! Evidence that the startup config reads refuse what they should not read.
//!
//! `nuvem::config::read`'s own unit tests pin the predicate; these pin that
//! the three loaders actually go through it, which is the half a shared helper
//! can silently stop doing. The plugin loader carries an extra refusal nothing
//! else does — a world-writable declaration file — because it is the only
//! configuration that causes a process to run.

use std::io::Write;

use nuvem::config::plugins::Plugins;
use nuvem::config::problems::ConfigProblems;
use nuvem::config::read::CONFIG_FILE_MAX_BYTES;

/// A minimal plugin file that loads cleanly, so a refusal below can only come
/// from the property under test and never from the document.
const ONE_GOOD_PLUGIN: &str = r#"
version = 1

[[plugins]]
id    = "describe"
scope = "compute"
key   = "d"
label = "Describe {name}"
args  = ["compute", "instances", "describe", "{name}", "--zone", "{zone}"]
"#;

fn load_plugins(path: &std::path::Path) -> (Plugins, ConfigProblems) {
    let mut problems = ConfigProblems::default();
    let plugins = Plugins::load(path, &mut problems);
    (plugins, problems)
}

#[test]
fn a_well_formed_plugin_file_still_loads() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("plugins.toml");
    std::fs::write(&path, ONE_GOOD_PLUGIN).expect("write");

    let (plugins, problems) = load_plugins(&path);

    assert_eq!(plugins.len(), 1, "{:?}", problems.messages());
    assert!(problems.is_empty(), "{:?}", problems.messages());
}

/// The refusal that matters most: a plugin file any local account can append
/// to is a command any local account can get offered, and confirmed by an
/// operator with no reason to suspect the row. Refused rather than warned
/// about, because a warning would leave the entries loaded.
#[cfg(unix)]
#[test]
fn a_world_writable_plugin_file_loads_no_plugins_and_says_why() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("plugins.toml");
    std::fs::write(&path, ONE_GOOD_PLUGIN).expect("write");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o666)).expect("chmod");

    let (plugins, problems) = load_plugins(&path);

    assert_eq!(
        plugins.len(),
        0,
        "a world-writable declaration must not be offered"
    );
    let reported = problems.messages().join("\n");
    assert!(
        reported.contains("world-writable"),
        "the operator must be told which property refused it: {reported}"
    );
    assert!(
        reported.contains("chmod o-w"),
        "and how to fix it: {reported}"
    );
}

/// Group-writable is the `umask 002` case and must keep working: a refusal
/// here would refuse working machines.
#[cfg(unix)]
#[test]
fn a_group_writable_plugin_file_still_loads() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("plugins.toml");
    std::fs::write(&path, ONE_GOOD_PLUGIN).expect("write");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o664)).expect("chmod");

    let (plugins, problems) = load_plugins(&path);

    assert_eq!(plugins.len(), 1, "{:?}", problems.messages());
}

/// The hang, through the real loader: a FIFO at the plugin path used to park
/// `read_to_string` forever with nothing drawn. The assertion is that loading
/// returns at all — and says why it loaded nothing.
#[cfg(unix)]
#[test]
fn a_fifo_plugin_file_is_refused_instead_of_hanging_startup() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("plugins.toml");
    let c_path = std::ffi::CString::new(path.as_os_str().as_encoded_bytes()).expect("cstring");
    // SAFETY: `mkfifo` takes a nul-terminated path and a mode and returns a
    // status; `c_path` outlives the call and no pointer is retained.
    assert_eq!(unsafe { libc::mkfifo(c_path.as_ptr(), 0o644) }, 0, "mkfifo");

    let (plugins, problems) = load_plugins(&path);

    assert_eq!(plugins.len(), 0);
    assert!(
        problems
            .messages()
            .join("\n")
            .contains("not an ordinary file"),
        "{:?}",
        problems.messages()
    );
}

#[test]
fn an_oversized_plugin_file_is_refused_by_size() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("plugins.toml");
    let mut handle = std::fs::File::create(&path).expect("create");
    handle.write_all(ONE_GOOD_PLUGIN.as_bytes()).expect("write");
    // Padded past the ceiling with comment lines, so the document stays valid
    // TOML and only its size can be what refuses it.
    handle
        .write_all(&vec![b'#'; CONFIG_FILE_MAX_BYTES as usize])
        .expect("pad");
    drop(handle);

    let (plugins, problems) = load_plugins(&path);

    assert_eq!(plugins.len(), 0);
    assert!(
        problems.messages().join("\n").contains("ceiling"),
        "{:?}",
        problems.messages()
    );
}

/// An absent plugin file is the overwhelmingly common case and must stay
/// silent — the guard must not turn "no plugins" into a warning.
#[test]
fn an_absent_plugin_file_is_still_silent() {
    let dir = tempfile::tempdir().expect("tempdir");

    let (plugins, problems) = load_plugins(&dir.path().join("plugins.toml"));

    assert_eq!(plugins.len(), 0);
    assert!(problems.is_empty(), "{:?}", problems.messages());
}

/// The config file goes through the same reader, and reports the refusal in
/// its own error shape rather than the plugin file's.
#[cfg(unix)]
#[test]
fn a_fifo_config_file_is_refused_rather_than_read() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("config.toml");
    let c_path = std::ffi::CString::new(path.as_os_str().as_encoded_bytes()).expect("cstring");
    // SAFETY: as above — nul-terminated path, mode, status return.
    assert_eq!(unsafe { libc::mkfifo(c_path.as_ptr(), 0o644) }, 0, "mkfifo");

    let error = nuvem::config::file::load(&path).expect_err("a FIFO is not a config file");

    assert!(
        error.to_string().contains("not an ordinary file"),
        "{error}"
    );
}

/// And so does the remembered-view state file, whose loader has its own error
/// type again.
#[cfg(unix)]
#[test]
fn a_fifo_state_file_is_refused_rather_than_read() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("state.toml");
    let c_path = std::ffi::CString::new(path.as_os_str().as_encoded_bytes()).expect("cstring");
    // SAFETY: as above.
    assert_eq!(unsafe { libc::mkfifo(c_path.as_ptr(), 0o644) }, 0, "mkfifo");

    let error =
        nuvem::config::state::load::<toml::Value>(&path).expect_err("a FIFO is not a state file");

    assert!(error.message.contains("not an ordinary file"), "{error:?}");
}
