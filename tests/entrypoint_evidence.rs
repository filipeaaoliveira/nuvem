//! Pins the two process-entrypoint bodies that no library-level test reached:
//! `nuvem::run` (src/lib.rs) and the binary's `main` (src/main.rs).
//!
//! Both were surviving mutants in the scheduled mutation sweep's shard 0, and
//! both survive as *whole-body* replacements — `cargo mutants` rewrites each
//! function to `Ok(())` rather than editing a single statement. A unit test
//! inside the library cannot see either: `nuvem::run` is only ever called by
//! `main`, and `main` is only ever called by the OS. So the assertions here
//! are deliberately made from outside each entrypoint — one calls the public
//! `run` and inspects what comes back, the other spawns the built binary and
//! inspects what it wrote and what it exited with.

use clap::Parser;
use nuvem::config::problems::ConfigProblems;
use nuvem::{Cli, Config, ConfigOutcome};

/// `nuvem::run` is a one-line delegation to `services::runtime::run`, and a
/// body replaced by `Ok(())` is indistinguishable from a successful session
/// unless something the real runtime does is observable from the caller.
///
/// The runtime's first fallible step is `logging::initialize_tracing`, which
/// creates the log file's parent directory. Pointing `log_file` *inside* a
/// regular file makes that step fail deterministically — no TTY, no gcloud, no
/// terminal takeover — so the error travelling back out of `nuvem::run` is
/// proof the delegation happened. The mutant returns `Ok(())` and fails here.
///
/// The assertion insists on the *startup* category and on an `io::Error` of a
/// directory-creation kind rather than merely "is an error": the runtime's
/// other early exit (`ensure_interactive_terminal`) reports
/// `exit::NO_TERMINAL` and carries no source at all, so this cannot be
/// satisfied by the wrong startup step. `mkdir` over an existing regular file
/// is `EEXIST` per POSIX, while a deeper path component being a file yields
/// `ENOTDIR`; both are accepted because which one surfaces depends on the
/// path, not on the code under test.
#[tokio::test]
async fn library_run_delegates_to_the_runtime_and_surfaces_its_startup_error() {
    let dir = tempfile::tempdir().expect("temp dir");
    let blocker = dir.path().join("not-a-directory");
    std::fs::write(&blocker, b"").expect("write the blocking regular file");

    // Bare argv on purpose: the `ConfigOutcome` below is hand-built, and the
    // `cli` is only ever read by `RuntimeSession::new`, which the failing
    // `initialize_tracing` never reaches. Flags here would only imply an
    // influence they do not have.
    let cli = Cli::parse_from(["nuvem"]);
    let outcome = ConfigOutcome {
        config: Config {
            log_file: blocker.join("nuvem.log"),
            ..Config::default()
        },
        problems: ConfigProblems::default(),
        load_error: None,
    };

    let error = nuvem::run(outcome, cli)
        .await
        .expect_err("a log file nested inside a regular file cannot be opened");

    assert_eq!(
        nuvem::exit::code_for(&error),
        nuvem::exit::STARTUP,
        "expected the startup step to fail, got: {error:#}"
    );
    let categorised = error
        .downcast_ref::<nuvem::exit::Categorised>()
        .unwrap_or_else(|| panic!("expected a categorised startup failure, got: {error:#}"));
    let io = std::error::Error::source(categorised)
        .and_then(|source| source.downcast_ref::<std::io::Error>())
        .unwrap_or_else(|| panic!("expected the runtime's I/O failure, got: {error:#}"));
    assert!(
        matches!(
            io.kind(),
            std::io::ErrorKind::AlreadyExists | std::io::ErrorKind::NotADirectory
        ),
        "expected the log directory creation to fail, got {:?}: {error:#}",
        io.kind()
    );
    let blocker_meta = std::fs::metadata(&blocker).expect("the blocking file survives");
    assert!(
        blocker_meta.is_file() && blocker_meta.len() == 0,
        "the failed startup must leave the blocking file untouched, not write the log into it"
    );
}

/// `main`'s body replaced by `Ok(())` turns the binary into a silent, always
/// successful no-op. Nothing inside the library can tell the difference, so
/// this runs the real executable.
///
/// `info` is the one subcommand that completes without a terminal: it prints
/// the resolved paths and returns. A binary that printed nothing would pass a
/// bare exit-status check, so the report's own content is what is asserted.
#[test]
fn the_binary_runs_the_info_subcommand_and_prints_the_resolved_report() {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_nuvem"))
        .args(["info", "--config", "/nonexistent/nuvem.toml"])
        .output()
        .expect("the built binary runs");

    assert!(
        output.status.success(),
        "`nuvem info` must exit cleanly, got {:?} with stderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    for expected in ["config:", "/nonexistent/nuvem.toml", "not found", "theme:"] {
        assert!(
            stdout.contains(expected),
            "`nuvem info` output is missing {expected:?}; got: {stdout:?}"
        );
    }
}

/// The narrower half of the same pin, aimed at the argument parse itself: a
/// `main` that never reaches `Cli::parse()` accepts anything and exits 0,
/// where the real binary rejects an unknown flag with clap's usage error.
#[test]
fn the_binary_rejects_an_unknown_flag_instead_of_exiting_silently() {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_nuvem"))
        .arg("--not-a-real-nuvem-flag")
        .output()
        .expect("the built binary runs");

    assert!(
        !output.status.success(),
        "an unknown flag must not exit successfully; stdout was: {}",
        String::from_utf8_lossy(&output.stdout)
    );

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("--not-a-real-nuvem-flag"),
        "clap must name the offending flag; stderr was: {stderr:?}"
    );
    assert!(
        stderr.contains("Usage"),
        "clap must print usage on a parse failure; stderr was: {stderr:?}"
    );
}
