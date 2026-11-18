use nuvem::app::AppError;
use nuvem::services::gcloud::cli::{CommandCeiling, CommandError, CommandProgram};
use nuvem::services::gcloud::configurations::GcloudConfigurationError;
use nuvem_domain::gcp::diagnostics::GcloudErrorKind;

#[test]
fn gcloud_error_is_classified_from_message() {
    let error = AppError::gcloud("PERMISSION_DENIED: nope");
    assert_eq!(error.kind(), GcloudErrorKind::PermissionDenied);
}

#[test]
fn parse_error_maps_to_parse_kind() {
    assert_eq!(
        AppError::Parse("bad json".to_string()).kind(),
        GcloudErrorKind::Parse
    );
}

#[test]
fn missing_context_maps_to_missing_project() {
    assert_eq!(
        AppError::MissingContext.kind(),
        GcloudErrorKind::MissingProject
    );
}

#[test]
fn command_error_missing_executable_maps_to_missing_executable_kind() {
    let err: AppError = CommandError::MissingExecutable {
        program: CommandProgram::Gcloud,
        detail: "gcloud not found".into(),
    }
    .into();
    assert_eq!(err.kind(), GcloudErrorKind::MissingExecutable);
}

#[test]
fn command_error_cancelled_maps_to_cancelled_kind() {
    let err: AppError = CommandError::Cancelled {
        ceiling: CommandCeiling::Read,
    }
    .into();
    assert_eq!(err.kind(), GcloudErrorKind::Cancelled);
}

#[test]
fn command_error_failed_preserves_exit_code_and_classifies_stderr() {
    let err: AppError = CommandError::Failed {
        stdout: String::new(),
        stderr: "PERMISSION_DENIED: forbidden (403)".into(),
        exit_code: Some(1),
    }
    .into();
    assert_eq!(err.kind(), GcloudErrorKind::PermissionDenied);
    assert_eq!(err.exit_code(), Some(1));
}

/// The failure an operator reads is the stream the child diagnosed on.
///
/// `bq` writes its errors to stdout and exits non-zero with an empty stderr,
/// so this conversion used to build a `GcloudFailure` whose entire
/// `detail` was `""` — and `AppError::kind` then classified that empty string
/// instead of the message. The wording below is verbatim from the live binary
/// (`bq show` against a table that does not exist, 2026-08-25).
///
/// The equality is on the whole string rather than a `contains`: "the detail
/// is no longer empty" is satisfied by any constant, and only the exact
/// message proves the child's own bytes made it through unmodified.
#[test]
fn command_error_failed_carries_a_diagnostic_the_child_wrote_to_stdout() {
    let message = "BigQuery error in show operation: Not found: Table p:dataset.table".to_string();

    let err: AppError = CommandError::Failed {
        stdout: message.clone(),
        stderr: String::new(),
        exit_code: Some(2),
    }
    .into();

    assert_eq!(err.detail(), message);
    assert_eq!(err.exit_code(), Some(2));
}

/// The paired positive: a child that used stderr is still read from stderr.
///
/// This is the direction that had to survive the fix. Every gcloud failure —
/// far and away the larger population — diagnoses on stderr, and some of them
/// have a partial document on stdout beside it; a fallback that preferred
/// stdout, or that glued the two together, would have degraded all of them to
/// buy the three `bq` calls. The stdout here is a truncated JSON fragment for
/// that reason, and the assertion is a whole-string equality rather than a
/// `contains`: a resolution that concatenated the two streams would satisfy a
/// `contains` and fail this.
#[test]
fn a_child_that_used_stderr_is_still_classified_from_stderr() {
    let err: AppError = CommandError::Failed {
        stdout: "{\"partial\": ".into(),
        stderr: "ERROR: (gcloud.compute.instances.list) PERMISSION_DENIED: forbidden".into(),
        exit_code: Some(1),
    }
    .into();

    assert_eq!(
        err.detail(),
        "ERROR: (gcloud.compute.instances.list) PERMISSION_DENIED: forbidden"
    );
    assert_eq!(err.kind(), GcloudErrorKind::PermissionDenied);
}

/// Whitespace on stderr is not a reason, so the fallback still fires.
///
/// A child that exits non-zero having written a bare newline to stderr has
/// said nothing, and an emptiness test spelled `is_empty()` would hand the
/// operator that newline and call the job done — the same blank screen
/// this is about, one byte heavier.
#[test]
fn a_whitespace_only_stderr_does_not_displace_the_stdout_diagnostic() {
    let err: AppError = CommandError::Failed {
        stdout: "BigQuery error in ls operation: Not found: Dataset p:nope".into(),
        stderr: "\n".into(),
        exit_code: Some(2),
    }
    .into();

    assert_eq!(
        err.detail(),
        "BigQuery error in ls operation: Not found: Dataset p:nope"
    );
}

#[test]
fn created_configuration_setup_failed_preserves_kind_and_exit_code() {
    let err: AppError = GcloudConfigurationError::CreatedConfigurationSetupFailed {
        name: "my-config".into(),
        source: CommandError::Failed {
            stdout: String::new(),
            stderr: "PERMISSION_DENIED: forbidden (403)".into(),
            exit_code: Some(1),
        },
    }
    .into();
    assert_eq!(err.kind(), GcloudErrorKind::PermissionDenied);
    assert_eq!(err.exit_code(), Some(1));
}

#[test]
fn plain_gcloud_error_classifies_from_detail() {
    let err = AppError::gcloud("Compute Engine API has not been used");
    assert_eq!(err.kind(), GcloudErrorKind::ApiDisabled);
}

/// The defect this pins: `CommandError::MissingExecutable` has named the right
/// binary since slice 1, but the conversion into `AppError` dropped the
/// program, and the kind's own label said "missing gcloud CLI" for every
/// binary. A user whose `bq` was missing was told to install a `gcloud` that
/// was, necessarily, already installed and working.
///
/// The negative assertion is the one that failed before the fix.
#[test]
fn a_missing_bq_binary_tells_the_user_to_install_bq_not_gcloud() {
    let error: AppError = CommandError::MissingExecutable {
        program: CommandProgram::Bq,
        detail: "No such file or directory (os error 2)".into(),
    }
    .into();

    let rendered = error.diagnostic("BigQuery datasets").headline();

    assert!(rendered.contains("bq"), "{rendered}");
    assert!(
        !rendered.contains("gcloud"),
        "a missing bq must not send the reader to reinstall gcloud: {rendered}"
    );
}

/// The other half of the same claim, and the reason it is a separate test: a
/// fix that renamed every missing-binary message to `bq` would satisfy the
/// test above.
#[test]
fn a_missing_gcloud_binary_still_names_gcloud() {
    let error: AppError = CommandError::MissingExecutable {
        program: CommandProgram::Gcloud,
        detail: "No such file or directory (os error 2)".into(),
    }
    .into();

    let rendered = error.diagnostic("Compute instances").headline();

    assert!(rendered.contains("gcloud"), "{rendered}");
    assert!(
        !rendered.contains("bq"),
        "a missing gcloud must not name bq: {rendered}"
    );
}

/// A walk over every `CommandProgram`, in the only form the compiler enforces
/// for an enum with no generated `ALL`: the expectation comes from an
/// exhaustive `match`, so a third program cannot be added without this test
/// failing to compile until its missing-binary wording is decided here.
#[test]
fn every_program_s_missing_binary_diagnostic_names_its_own_binary() {
    fn expected(program: CommandProgram) -> &'static str {
        match program {
            CommandProgram::Gcloud => "gcloud",
            CommandProgram::Bq => "bq",
        }
    }

    let programs = [CommandProgram::Gcloud, CommandProgram::Bq];
    for program in programs {
        let error: AppError = CommandError::MissingExecutable {
            program,
            detail: "No such file or directory (os error 2)".into(),
        }
        .into();
        let diagnostic = error.diagnostic("service");
        let rendered = diagnostic.headline();

        assert_eq!(expected(program), program.binary_name(), "{program:?}");
        assert!(
            rendered.contains(program.binary_name()),
            "{program:?}: {rendered}"
        );
        for other in programs {
            if other.binary_name() != program.binary_name() {
                assert!(
                    !rendered.contains(other.binary_name()),
                    "{program:?} named {other:?}: {rendered}"
                );
            }
        }
        // The binary is named beside the classification, not inside it: both
        // programs still classify as the same kind, which is what the app
        // compares and stores.
        assert_eq!(diagnostic.kind, GcloudErrorKind::MissingExecutable);
        assert_eq!(diagnostic.user_action(), expected_action(program));
        // The batch-progress and operations-log rows render the error's own
        // label rather than a diagnostic; it names the binary too.
        assert_eq!(
            error.label(),
            format!("missing {} CLI", program.binary_name())
        );
        assert_eq!(error.kind().label(), "missing CLI");
    }
}

fn expected_action(program: CommandProgram) -> String {
    format!(
        "Install the {} CLI and ensure it is on PATH",
        program.binary_name()
    )
}

/// A failure that reaches the UI with no program recorded — every kind that is
/// not a missing executable, and any missing-executable path that did not
/// carry one — stays vague rather than guessing a binary. Vague is the
/// fail-safe direction; "install gcloud" was the wrong-answer direction.
#[test]
fn a_missing_binary_with_no_recorded_program_names_no_binary() {
    let diagnostic = nuvem_domain::gcp::diagnostics::GcloudDiagnostic::new(
        GcloudErrorKind::MissingExecutable,
        "service",
        None,
        None,
        "not found",
    );

    let rendered = diagnostic.headline();

    assert!(!rendered.contains("gcloud"), "{rendered}");
    assert!(!rendered.contains("bq"), "{rendered}");
    assert!(rendered.contains("missing CLI"), "{rendered}");
}

/// A timed-out command is an `Unknown`, not a `Network` failure.
///
/// `AppError::gcloud` leaves `kind_hint` empty, so the kind was inferred from
/// the detail — and the detail says "timed out", which is one of the needles
/// `classify_gcloud_stderr` reads as a network problem. Both kinds are
/// Transient, so nothing about the retry gate changed; what changed was every
/// operator-facing word. nuvem killed the child itself, at its own ceiling, and
/// then told the operator to check their network connection.
#[test]
fn a_timed_out_command_is_unknown_rather_than_a_network_failure() {
    let error: AppError = CommandError::TimedOut {
        seconds: 120,
        ceiling: CommandCeiling::Read,
    }
    .into();

    assert_eq!(error.kind(), GcloudErrorKind::Unknown);
    assert_ne!(error.kind(), GcloudErrorKind::Network);
    assert_eq!(
        error.kind().retryability(),
        nuvem_domain::gcp::diagnostics::Retryability::Transient,
        "a call that has not answered is still safe for the poll ticker to reissue"
    );
}

/// A timed-out *lifecycle* command must not read as a clean refusal.
///
/// Killing the CLI does not cancel the GCP operation it started: a failover
/// that has already fired keeps going, and the instance the operator is looking
/// at will change under them whatever this message says. The one thing nuvem
/// can honestly report is that it stopped watching, so the detail says the
/// operation may still be running rather than leaving "timed out" to be read as
/// "nothing happened".
#[test]
fn a_timed_out_lifecycle_command_says_the_operation_may_still_be_running() {
    let error: AppError = CommandError::TimedOut {
        seconds: 1800,
        ceiling: CommandCeiling::Lifecycle,
    }
    .into();

    let detail = error.detail().to_lowercase();
    assert!(detail.contains("timed out"), "{detail}");
    assert!(
        detail.contains("may still be running"),
        "a killed lifecycle command must not present as a call that did nothing: {detail}"
    );
}

/// The counterpart that keeps the sentence above meaningful: a read that timed
/// out started nothing server-side, and warning about it there would train the
/// reader to ignore the warning where it matters.
#[test]
fn a_timed_out_read_makes_no_server_side_claim() {
    let error: AppError = CommandError::TimedOut {
        seconds: 120,
        ceiling: CommandCeiling::Read,
    }
    .into();

    let detail = error.detail().to_lowercase();
    assert!(detail.contains("timed out"), "{detail}");
    assert!(!detail.contains("may still be running"), "{detail}");
}

/// The third outcome, and the one added last: a lifecycle call nuvem stopped
/// waiting on because somebody decided to, rather than because a deadline
/// passed.
///
/// It has to be distinguishable from the timeout above at the layer the
/// operator reads, not only inside `CommandError`. The two carry the same
/// caveat and must not carry the same verb: "timed out" says nuvem waited out a
/// ceiling, "cancelled" says nuvem walked away, and the retryability differs
/// with them — `Cancelled` is Persistent because a person decided to stop,
/// where a timeout is Transient and the poll ticker may reissue it.
#[test]
fn a_cancelled_lifecycle_command_says_the_operation_may_still_be_running() {
    let error: AppError = CommandError::Cancelled {
        ceiling: CommandCeiling::Lifecycle,
    }
    .into();

    let detail = error.detail().to_lowercase();
    assert!(detail.contains("cancelled"), "{detail}");
    assert!(
        !detail.contains("timed out"),
        "an abandoned call is not a call that ran out of time: {detail}"
    );
    assert!(
        detail.contains("may still be running"),
        "cancelling the call is not cancelling the operation, and the detail has \
         to say so: {detail}"
    );
    assert_eq!(error.kind(), GcloudErrorKind::Cancelled);
}

/// The counterpart, for the same reason the timeout has one: a cancelled read
/// started nothing in GCP, so it claims nothing about it.
#[test]
fn a_cancelled_read_makes_no_server_side_claim() {
    let error: AppError = CommandError::Cancelled {
        ceiling: CommandCeiling::Read,
    }
    .into();

    let detail = error.detail().to_lowercase();
    assert!(detail.contains("cancelled"), "{detail}");
    assert!(!detail.contains("may still be running"), "{detail}");
}
