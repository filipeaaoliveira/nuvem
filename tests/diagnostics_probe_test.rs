use nuvem::services::gcloud::cli::{
    CommandError, CommandOutput, CommandProgram, CommandRunner, GcloudCommand,
};
use nuvem::services::gcloud::diagnostics::GcloudDiagnosticsService;
use nuvem_domain::gcp::diagnostics::GcloudErrorKind;

struct StubRunner {
    result: Result<CommandOutput, CommandError>,
}

impl CommandRunner for StubRunner {
    fn run(&self, _command: GcloudCommand) -> Result<CommandOutput, CommandError> {
        match &self.result {
            Ok(output) => Ok(output.clone()),
            Err(CommandError::Io(m)) => Err(CommandError::Io(m.clone())),
            Err(CommandError::Failed {
                stdout,
                stderr,
                exit_code,
            }) => Err(CommandError::Failed {
                stdout: stdout.clone(),
                stderr: stderr.clone(),
                exit_code: *exit_code,
            }),
            Err(CommandError::MissingExecutable { program, detail }) => {
                Err(CommandError::MissingExecutable {
                    program: *program,
                    detail: detail.clone(),
                })
            }
            Err(CommandError::Cancelled { ceiling }) => {
                Err(CommandError::Cancelled { ceiling: *ceiling })
            }
            Err(CommandError::TimedOut { seconds, ceiling }) => Err(CommandError::TimedOut {
                seconds: *seconds,
                ceiling: *ceiling,
            }),
        }
    }
}

#[test]
fn probe_reports_active_account_on_success() {
    let runner = StubRunner {
        result: Ok(CommandOutput {
            stdout: r#"[{"account":"dev@example.com","status":"ACTIVE"},{"account":"old@example.com","status":""}]"#.to_string(),
            stderr: String::new(),
        }),
    };
    let report = GcloudDiagnosticsService::new(runner).probe(None);
    assert!(report.available);
    assert_eq!(report.active_account.as_deref(), Some("dev@example.com"));
    assert!(report.problem.is_none());
}

#[test]
fn probe_reports_unavailable_on_io_error() {
    let runner = StubRunner {
        result: Err(CommandError::Io("No such file: gcloud".to_string())),
    };
    let report = GcloudDiagnosticsService::new(runner).probe(None);
    assert!(!report.available);
    assert!(report.problem.is_some());
}

#[test]
fn probe_classifies_command_failure() {
    let runner = StubRunner {
        result: Err(CommandError::Failed {
            stdout: String::new(),
            stderr: "PERMISSION_DENIED: no".to_string(),
            exit_code: None,
        }),
    };
    let report = GcloudDiagnosticsService::new(runner).probe(None);
    assert!(report.available);
    assert_eq!(
        report.problem.unwrap().kind,
        GcloudErrorKind::PermissionDenied
    );
}

/// The probe reads the stream the child diagnosed on, like every other reader
/// of a `Failed`.
///
/// This probe only ever spawns `gcloud`, which diagnoses on stderr, so the
/// case below cannot arise today. It is pinned anyway because the alternative
/// is a second convention: `failure_diagnostic` exists so that no reader of a
/// failed command has to know which binary it was, and a path that reaches
/// past it into `stderr` is exactly how `bq` produced blank BigQuery errors
/// for a release. The wording is `bq`'s so the pin says which
/// binary it is guarding against.
#[test]
fn probe_reads_a_diagnostic_the_child_wrote_to_stdout() {
    let runner = StubRunner {
        result: Err(CommandError::Failed {
            stdout: "BigQuery error in ls operation: PERMISSION_DENIED: no".to_string(),
            stderr: String::new(),
            exit_code: Some(1),
        }),
    };

    let problem = GcloudDiagnosticsService::new(runner)
        .probe(None)
        .problem
        .expect("a refusal is a problem");

    assert_eq!(
        problem.message,
        "BigQuery error in ls operation: PERMISSION_DENIED: no"
    );
    assert_eq!(problem.kind, GcloudErrorKind::PermissionDenied);
}

#[test]
fn probe_flags_missing_active_account() {
    let runner = StubRunner {
        result: Ok(CommandOutput {
            stdout: "[]".to_string(),
            stderr: String::new(),
        }),
    };
    let report = GcloudDiagnosticsService::new(runner).probe(None);
    assert!(report.available);
    assert!(report.active_account.is_none());
    let problem = report.problem.expect("problem present");
    // Missing an active account is an auth problem, not a missing-project
    // problem; classify it as Unauthenticated so the recovery hint points at
    // `gcloud auth login` rather than the generic Unknown fallback.
    assert_eq!(
        problem.kind,
        nuvem_domain::gcp::diagnostics::GcloudErrorKind::Unauthenticated
    );
    assert_eq!(problem.message, "no active gcloud account");
}

/// The Home health card reads this report, and it is the surface whose whole
/// job is saying which CLI is unhealthy. The probe's `MissingExecutable` arm
/// must therefore carry the program into the problem: without it the card
/// falls back to the binary-agnostic "missing CLI" while claiming to report on
/// a named binary.
///
/// Walked over both programs, expectation from an exhaustive `match`, so a
/// third `CommandProgram` cannot land without a compile failure here. The
/// probe only spawns gcloud today; the `Bq` case is what stops the arm from
/// being "fixed" back to a hard-coded name.
#[test]
fn a_probe_that_cannot_spawn_its_binary_names_that_binary() {
    fn expected_label(program: CommandProgram) -> &'static str {
        match program {
            CommandProgram::Gcloud => "missing gcloud CLI",
            CommandProgram::Bq => "missing bq CLI",
        }
    }

    for program in [CommandProgram::Gcloud, CommandProgram::Bq] {
        let runner = StubRunner {
            result: Err(CommandError::MissingExecutable {
                program,
                detail: "No such file or directory (os error 2)".to_string(),
            }),
        };

        let report = GcloudDiagnosticsService::new(runner).probe(None);

        assert!(!report.available, "{program:?}");
        let problem = report.problem.expect("a missing binary is a problem");
        assert_eq!(
            problem.kind,
            GcloudErrorKind::MissingExecutable,
            "{program:?}"
        );
        assert_eq!(problem.label(), expected_label(program), "{program:?}");
        assert_eq!(
            problem.user_action(),
            format!(
                "Install the {} CLI and ensure it is on PATH",
                program.binary_name()
            ),
            "{program:?}"
        );
    }
}
