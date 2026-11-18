//! The log-dump writer and the `SaveLogEntries` effect it backs: the writer is
//! pure filesystem work over a caller-supplied directory, and the runtime
//! session derives that directory from the resolved config before delegating.

use chrono::{Duration, Local, TimeZone};
use nuvem::Config;
use nuvem::app::{Action, AppState, Effect};
use nuvem::services::gcloud::cli::{CommandError, CommandOutput, CommandRunner, GcloudCommand};
use nuvem::services::runtime::log_export::write_log_dump;
use nuvem::services::runtime::session::{RuntimeSession, format_log_dump_timestamp};

/// The save path never shells out; the session still needs *a* runner.
struct UnusedRunner;
impl CommandRunner for UnusedRunner {
    fn run(&self, command: GcloudCommand) -> Result<CommandOutput, CommandError> {
        panic!("the save effect must not run gcloud, got {command:?}")
    }
}

fn lines() -> Vec<String> {
    vec![
        "t1 ERROR gce_instance disk full".to_string(),
        "t2 INFO cloud_run_revision request served".to_string(),
    ]
}

#[test]
fn write_log_dump_creates_the_directory_and_writes_every_line() {
    let temp = tempfile::tempdir().expect("temp dir");
    let dir = temp.path().join("log-dumps");
    assert!(!dir.exists(), "directory is created by the writer");

    let path = write_log_dump(&dir, "20260722-101500", &lines()).expect("dump written");

    assert_eq!(
        path,
        dir.join("logs-20260722-101500.log"),
        "file is named after the timestamp"
    );
    assert!(
        path.is_absolute(),
        "reported path must be absolute: {path:?}"
    );
    let written = std::fs::read_to_string(&path).expect("dump readable");
    assert_eq!(
        written,
        "t1 ERROR gce_instance disk full\nt2 INFO cloud_run_revision request served\n"
    );
}

#[test]
fn write_log_dump_reports_failure_for_an_unwritable_directory() {
    let temp = tempfile::tempdir().expect("temp dir");
    // A regular file where the dump directory should be: creating the
    // directory must fail rather than panic.
    let blocker = temp.path().join("log-dumps");
    std::fs::write(&blocker, "not a directory").expect("blocker written");

    let error = write_log_dump(&blocker, "20260722-101500", &lines())
        .expect_err("directory creation cannot succeed");
    assert!(
        !error.to_string().is_empty(),
        "error must be renderable: {error:?}"
    );
}

#[tokio::test]
async fn save_log_entries_effect_writes_under_the_cache_dir_and_reports_the_path() {
    let temp = tempfile::tempdir().expect("temp dir");
    let config = Config {
        log_file: temp.path().join("nuvem.log"),
        ..Config::default()
    };
    let mut session = RuntimeSession::new_with_config_and_runner_for_test(
        config,
        AppState::new(),
        Box::new(UnusedRunner),
    );

    session
        .dispatch_action(Action::Effect(Effect::SaveLogEntries { lines: lines() }))
        .await;

    let dumps: Vec<_> = std::fs::read_dir(temp.path().join("log-dumps"))
        .expect("dump directory created")
        .map(|entry| entry.expect("dir entry").path())
        .collect();
    assert_eq!(dumps.len(), 1, "one dump written: {dumps:?}");
    let written = std::fs::read_to_string(&dumps[0]).expect("dump readable");
    assert_eq!(
        written,
        "t1 ERROR gce_instance disk full\nt2 INFO cloud_run_revision request served\n"
    );

    let status = session
        .app()
        .root
        .system
        .status_message
        .clone()
        .expect("status message");
    assert!(
        status.contains("2 log entries"),
        "unexpected status: {status}"
    );
    assert!(
        status.contains(&dumps[0].display().to_string()),
        "status must name the absolute path: {status}"
    );
}

#[tokio::test]
async fn save_log_entries_effect_without_a_config_reports_an_error_instead_of_panicking() {
    let mut session = RuntimeSession::new_for_test(AppState::new());

    session
        .dispatch_action(Action::Effect(Effect::SaveLogEntries { lines: lines() }))
        .await;

    let status = session
        .app()
        .root
        .system
        .status_message
        .clone()
        .expect("status message");
    assert!(
        status.contains("Could not save log entries"),
        "the status must name the operation the user asked for: {status}"
    );
    assert!(
        session.app().root.system.last_diagnostic().is_some(),
        "failure must be copyable as a diagnostic"
    );
}

/// `--log-file nuvem.log` leaves `Path::parent()` as `Some("")`, which joins
/// into a RELATIVE `log-dumps` under the process working directory. Writing to
/// the CWD from a TUI was explicitly ruled out for this feature, so the save
/// must fail loudly instead.
#[tokio::test]
async fn save_log_entries_effect_with_a_bare_log_file_name_does_not_write_to_the_cwd() {
    let config = Config {
        log_file: std::path::PathBuf::from("nuvem.log"),
        ..Config::default()
    };
    let mut session = RuntimeSession::new_with_config_and_runner_for_test(
        config,
        AppState::new(),
        Box::new(UnusedRunner),
    );

    session
        .dispatch_action(Action::Effect(Effect::SaveLogEntries { lines: lines() }))
        .await;

    assert!(
        !std::path::Path::new("log-dumps").exists(),
        "a dump directory must not appear in the working directory"
    );
    let status = session
        .app()
        .root
        .system
        .status_message
        .clone()
        .expect("status message");
    assert!(
        status.contains("Could not save log entries"),
        "the failure must be reported to the user: {status}"
    );
}

#[test]
fn write_log_dump_names_are_distinct_within_the_same_second() {
    let temp = tempfile::tempdir().expect("temp dir");
    let dir = temp.path().join("log-dumps");

    // The session formats the timestamp; two saves inside one second differ
    // only in the millisecond component, and must not land on the same file.
    let first = write_log_dump(&dir, "20260722-101500.123", &lines()).expect("first dump");
    let second =
        write_log_dump(&dir, "20260722-101500.456", &[String::from("later")]).expect("second dump");

    assert_ne!(first, second, "same-second dumps must not collide");
    let earlier = std::fs::read_to_string(&first).expect("first dump readable");
    assert!(
        earlier.contains("disk full"),
        "the earlier dump must survive: {earlier}"
    );
}

#[test]
fn log_dump_timestamps_differ_for_two_saves_in_the_same_second() {
    let base = Local
        .with_ymd_and_hms(2026, 7, 22, 10, 15, 0)
        .single()
        .expect("unambiguous local time");

    let first = format_log_dump_timestamp(base + Duration::milliseconds(123));
    let second = format_log_dump_timestamp(base + Duration::milliseconds(456));

    assert_eq!(
        first, "20260722-101500.123",
        "readable, filename-safe stamp"
    );
    assert_ne!(
        first, second,
        "two saves inside one second must not share a filename"
    );
}
