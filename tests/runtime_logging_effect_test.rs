use std::sync::Mutex;

use nuvem::app::EffectResult;
use nuvem::services::gcloud::cli::{CommandError, CommandOutput, CommandRunner, GcloudCommand};
use nuvem::services::runtime::session::{load_recent_logs, tail_logs};
use nuvem_domain::gcp::configuration::{GcloudConfigurationName, GcloudContext};
use nuvem_domain::gcp::logging::{LogCursor, LogQuery, LogSeverity, LogTimeRange};

#[derive(Default)]
struct FakeRunner {
    outputs: Mutex<Vec<Result<CommandOutput, CommandError>>>,
}
impl CommandRunner for FakeRunner {
    fn run(&self, _command: GcloudCommand) -> Result<CommandOutput, CommandError> {
        self.outputs.lock().unwrap().remove(0)
    }
}
fn ctx() -> GcloudContext {
    GcloudContext {
        configuration: GcloudConfigurationName::new("dev"),
        account: None,
        project: Some("p".into()),
        region: None,
        zone: None,
    }
}

#[test]
fn load_recent_logs_maps_success() {
    let runner = FakeRunner::default();
    runner.outputs.lock().unwrap().push(Ok(CommandOutput {
        stdout: "[]".into(),
        stderr: String::new(),
    }));
    let result = load_recent_logs(
        &runner,
        None,
        ctx(),
        LogQuery::ProjectLogs,
        LogSeverity::Default,
        LogTimeRange::OneHour,
    );
    assert!(matches!(
        result,
        EffectResult::RecentLogsLoaded { dropped: 0, .. }
    ));
}

#[test]
fn load_recent_logs_maps_command_failure_to_failed_result() {
    let runner = FakeRunner::default();
    runner
        .outputs
        .lock()
        .unwrap()
        .push(Err(CommandError::Failed {
            stdout: String::new(),
            stderr: "PERMISSION_DENIED".into(),
            exit_code: None,
        }));
    let result = load_recent_logs(
        &runner,
        None,
        ctx(),
        LogQuery::ProjectLogs,
        LogSeverity::Default,
        LogTimeRange::OneHour,
    );
    match result {
        EffectResult::RecentLogsLoadFailed { error, .. } => {
            assert!(error.to_string().contains("PERMISSION_DENIED"))
        }
        other => panic!("unexpected {other:?}"),
    }
}

#[test]
fn tail_logs_maps_success() {
    let runner = FakeRunner::default();
    runner.outputs.lock().unwrap().push(Ok(CommandOutput {
        stdout: "[]".into(),
        stderr: String::new(),
    }));
    let cursor = LogCursor {
        timestamp: "t".into(),
        insert_ids: vec![],
    };
    let result = tail_logs(
        &runner,
        None,
        ctx(),
        LogQuery::ProjectLogs,
        LogSeverity::Default,
        cursor,
    );
    assert!(matches!(result, EffectResult::LogsTailed { .. }));
}
