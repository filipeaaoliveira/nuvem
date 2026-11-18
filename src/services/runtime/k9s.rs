//! k9s launch support layered on the generic interactive-process seam
//! (`super::interactive`). The interactive launch itself reuses
//! `execute_interactive_process`; this module only provides the k9s-specific
//! pieces: the best-effort kubectl context probe (for confirmation wording)
//! and the resolved `k9s --context <name>` argv spec.

use super::interactive::InteractiveProcessSpec;
use crate::services::gcloud::cli::{CommandRunner, GcloudCommand};

/// Best-effort list of kubeconfig context names via
/// `kubectl config get-contexts -o name`. Returns `None` when kubectl is
/// absent or errors — callers MUST treat `None` as "unknown" and fall back to
/// running `get-credentials` (which is idempotent), never blocking the launch.
pub fn resolve_existing_contexts(kubectl: &dyn CommandRunner) -> Option<Vec<String>> {
    let output = kubectl
        .run(GcloudCommand::new(["config", "get-contexts", "-o", "name"]))
        .ok()?;
    Some(
        output
            .stdout
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(str::to_string)
            .collect(),
    )
}

/// Resolved argv for `k9s --context <kube_context>`. Passed directly to the OS
/// through the interactive seam — never interpreted by a shell.
pub fn k9s_spec(k9s_binary: &str, kube_context: &str) -> InteractiveProcessSpec {
    InteractiveProcessSpec {
        program: k9s_binary.to_string(),
        args: vec!["--context".to_string(), kube_context.to_string()],
        description: format!("k9s ({kube_context})"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::gcloud::cli::{CommandError, CommandOutput, GcloudCommand};

    struct StubRunner {
        result: Result<CommandOutput, CommandError>,
    }

    impl CommandRunner for StubRunner {
        fn run(&self, _command: GcloudCommand) -> Result<CommandOutput, CommandError> {
            match &self.result {
                Ok(output) => Ok(output.clone()),
                Err(_) => Err(CommandError::Io("kubectl missing".to_string())),
            }
        }
    }

    fn ok(stdout: &str) -> StubRunner {
        StubRunner {
            result: Ok(CommandOutput {
                stdout: stdout.to_string(),
                stderr: String::new(),
            }),
        }
    }

    #[test]
    fn parses_context_names_ignoring_blank_lines() {
        let runner = ok("gke_p_us-central1-a_web\nother\n\n");
        assert_eq!(
            resolve_existing_contexts(&runner),
            Some(vec![
                "gke_p_us-central1-a_web".to_string(),
                "other".to_string(),
            ])
        );
    }

    #[test]
    fn returns_none_when_kubectl_errors() {
        let runner = StubRunner {
            result: Err(CommandError::Io("kubectl missing".to_string())),
        };
        assert_eq!(resolve_existing_contexts(&runner), None);
    }

    #[test]
    fn empty_output_yields_empty_list_not_none() {
        assert_eq!(resolve_existing_contexts(&ok("")), Some(Vec::new()));
    }

    #[test]
    fn k9s_spec_builds_context_argv_without_shell() {
        let spec = k9s_spec("k9s", "gke_p_us-central1-a_web");
        assert_eq!(spec.program, "k9s");
        assert_eq!(
            spec.args,
            vec![
                "--context".to_string(),
                "gke_p_us-central1-a_web".to_string()
            ]
        );
    }
}
