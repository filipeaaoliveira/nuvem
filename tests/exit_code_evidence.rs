//! Evidence that the built binary exits with the statuses `nuvem::exit`
//! documents.
//!
//! Every other test in this suite links the library, so none of them executes
//! `main` — the process's exit status is the one contract the library cannot
//! be asked about. These run the real executable (`CARGO_BIN_EXE_nuvem`, which
//! cargo builds and points at for integration tests) and read what the shell
//! would read.
//!
//! No terminal is attached under a test harness, which is what makes the
//! interesting case free: a TUI launch here takes the same path a pipeline or
//! a CI step takes.

use std::process::{Command, Output, Stdio};

/// Runs the built nuvem with `args` and returns its exit status, with stdio
/// detached so a test can never inherit the harness's own descriptors.
fn status_of(args: &[&str]) -> i32 {
    Command::new(env!("CARGO_BIN_EXE_nuvem"))
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .expect("the built binary runs")
        .code()
        .expect("nuvem exits rather than dying on a signal")
}

/// Runs the built nuvem with `args`, capturing what it printed.
fn output_of(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_nuvem"))
        .args(args)
        .stdin(Stdio::null())
        .output()
        .expect("the built binary runs")
}

/// The failure an operator is likeliest to see must read as one sentence.
///
/// The exit category used to be attached with `.context(…)`, which put it in
/// the error chain — and anyhow renders chains, so the refusal arrived with a
/// `Caused by: exit status 3` paragraph underneath it, reading as though the
/// status were the reason rather than the consequence.
#[test]
fn the_no_terminal_refusal_reads_as_one_sentence() {
    let stderr = String::from_utf8(output_of(&["--demo"]).stderr).expect("utf-8 stderr");

    assert!(
        stderr.contains("nuvem requires an interactive terminal"),
        "{stderr}"
    );
    assert!(
        !stderr.contains("Caused by"),
        "the exit category must not be reported as the cause: {stderr}"
    );
    assert!(
        !stderr.contains("exit status"),
        "the internal category must not reach the operator: {stderr}"
    );
}

/// `info` is the scriptable subcommand: it must work with no terminal, no
/// project, and no gcloud, and say so by exiting 0.
#[test]
fn info_succeeds_without_a_terminal() {
    assert_eq!(status_of(&["info"]), 0);
}

/// The status this whole table exists for. A pipeline or CI step that runs
/// nuvem without a TTY gets a code it can branch on, instead of the `1` that
/// also means "the session broke".
#[test]
fn launching_the_tui_without_a_terminal_reports_no_terminal() {
    assert_eq!(
        status_of(&["--demo"]),
        i32::from(nuvem::exit::NO_TERMINAL),
        "a missing terminal must be distinguishable from a real failure"
    );
}

/// Held to the same code with and without `--demo`: the refusal is about the
/// environment, not about which data source the run would have used.
#[test]
fn the_no_terminal_status_does_not_depend_on_demo_mode() {
    assert_eq!(status_of(&[]), i32::from(nuvem::exit::NO_TERMINAL));
}

/// clap's own code, asserted here because `nuvem::exit::USAGE` publishes it
/// as part of the table — a script reading the table is entitled to it.
#[test]
fn an_unknown_flag_reports_the_usage_status() {
    assert_eq!(
        status_of(&["--definitely-not-a-flag"]),
        i32::from(nuvem::exit::USAGE)
    );
}

/// A log file that cannot be opened is a startup failure: nothing has run, no
/// gcloud command was issued, so a caller retrying is retrying a local
/// problem. The path is under a file rather than a directory, which no
/// platform will let `create_dir_all` fix.
#[test]
fn an_unopenable_log_file_reports_the_startup_status() {
    let dir = tempfile::tempdir().expect("tempdir");
    let blocker = dir.path().join("not-a-directory");
    std::fs::write(&blocker, "").expect("write");
    let log_path = blocker.join("nested").join("nuvem.log");

    assert_eq!(
        status_of(&["--demo", "--log-file", log_path.to_str().expect("utf-8")]),
        i32::from(nuvem::exit::STARTUP),
        "a startup failure must not be reported as a session failure"
    );
}
