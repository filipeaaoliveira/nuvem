use serde::Deserialize;

use nuvem_domain::gcp::configuration::GcloudConfigurationName;
use nuvem_domain::gcp::diagnostics::{
    DiagnosticProblem, GcloudDiagnosticsReport, GcloudErrorKind, classify_gcloud_stderr,
};

use super::cli::{CommandError, CommandRunner, GcloudCommand, failure_diagnostic};

/// Targeted gcloud availability + auth probe. Runs a single cheap command.
pub struct GcloudDiagnosticsService<R> {
    runner: R,
}

#[derive(Debug, Deserialize)]
struct RawAuthAccount {
    account: String,
    #[serde(default)]
    status: String,
}

impl<R> GcloudDiagnosticsService<R> {
    pub fn new(runner: R) -> Self {
        Self { runner }
    }
}

impl<R: CommandRunner> GcloudDiagnosticsService<R> {
    pub fn probe(
        &self,
        configuration: Option<&GcloudConfigurationName>,
    ) -> GcloudDiagnosticsReport {
        let mut args: Vec<String> = Vec::new();
        if let Some(configuration) = configuration {
            args.push("--configuration".to_string());
            args.push(configuration.as_str().to_string());
        }
        args.extend([
            "auth".to_string(),
            "list".to_string(),
            "--format=json".to_string(),
        ]);

        match self.runner.run(GcloudCommand::new(args)) {
            Ok(output) => match serde_json::from_str::<Vec<RawAuthAccount>>(&output.stdout) {
                Ok(accounts) => {
                    let active_account = accounts
                        .into_iter()
                        .find(|account| account.status.eq_ignore_ascii_case("ACTIVE"))
                        .map(|account| account.account);
                    let problem = if active_account.is_none() {
                        Some(DiagnosticProblem::new(
                            GcloudErrorKind::Unauthenticated,
                            "no active gcloud account",
                        ))
                    } else {
                        None
                    };
                    GcloudDiagnosticsReport {
                        available: true,
                        active_account,
                        problem,
                    }
                }
                Err(error) => GcloudDiagnosticsReport {
                    available: true,
                    active_account: None,
                    problem: Some(DiagnosticProblem::new(
                        GcloudErrorKind::Parse,
                        error.to_string(),
                    )),
                },
            },
            Err(CommandError::Io(message)) => GcloudDiagnosticsReport {
                available: false,
                active_account: None,
                problem: Some(DiagnosticProblem::new(GcloudErrorKind::Unknown, message)),
            },
            // `failure_diagnostic` rather than `stderr`: which stream a refusal
            // came out on is the child's habit, not this probe's business, and
            // reaching past it is what left every BigQuery failure blank.
            // The probe spawns only gcloud today, which diagnoses on
            // stderr, so this changes nothing it can currently observe — it
            // stops the convention from having an exception.
            Err(CommandError::Failed { stdout, stderr, .. }) => {
                let diagnostic = failure_diagnostic(&stdout, &stderr);
                GcloudDiagnosticsReport {
                    available: true,
                    active_account: None,
                    problem: Some(DiagnosticProblem::new(
                        classify_gcloud_stderr(diagnostic),
                        diagnostic,
                    )),
                }
            }
            // `program` rather than a hard-coded "gcloud": the probe only ever
            // runs gcloud today, but the name it reports is the one that was
            // actually missing, so this stays right if the probe ever moves.
            Err(CommandError::MissingExecutable { program, detail }) => GcloudDiagnosticsReport {
                available: false,
                active_account: None,
                problem: Some(DiagnosticProblem::missing_executable(
                    program.binary_name(),
                    detail,
                )),
            },
            // The ceiling is dropped here on purpose: a diagnostics probe is a
            // read, so there is no operation in GCP for its caveat to warn
            // about even if a lifecycle command ever reached this arm.
            Err(CommandError::Cancelled { .. }) => GcloudDiagnosticsReport {
                available: true,
                active_account: None,
                problem: Some(DiagnosticProblem::new(
                    GcloudErrorKind::Unknown,
                    "gcloud diagnostics were cancelled",
                )),
            },
            // `available: true` — the binary answered the spawn, it just did
            // not answer in time, so reporting gcloud as absent would send the
            // reader to reinstall something that is installed. `Unknown` is
            // the kind whose retryability is Transient, which is what a
            // timeout is.
            Err(CommandError::TimedOut { seconds, .. }) => GcloudDiagnosticsReport {
                available: true,
                active_account: None,
                problem: Some(DiagnosticProblem::new(
                    GcloudErrorKind::Unknown,
                    format!("gcloud diagnostics timed out after {seconds}s"),
                )),
            },
        }
    }
}
