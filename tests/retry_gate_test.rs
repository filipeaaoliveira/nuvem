//! Retry gate (issue #31 AC #4): a `Persistent` load failure for the active
//! resource scope must not be auto-retried by the poll ticker, but manual
//! refresh (`r`) always proceeds and clears the gate.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use nuvem::app::{Action, AppState, EffectResult, ResourcesAction, WorkspaceAction};
use nuvem::services::gcloud::cli::{CommandError, CommandOutput, CommandRunner, GcloudCommand};
use nuvem::services::runtime::session::RuntimeSession;
use nuvem_domain::gcp::configuration::{GcloudConfiguration, GcloudConfigurationName};

/// A runner that pops a queued result per call, in order, regardless of
/// which gcloud command was requested. Mirrors the `FakeRunner` pattern used
/// throughout the runtime effect tests.
#[derive(Clone)]
struct QueueRunner {
    outputs: Arc<Mutex<Vec<Result<CommandOutput, CommandError>>>>,
}

impl QueueRunner {
    fn new(outputs: Vec<Result<CommandOutput, CommandError>>) -> Self {
        Self {
            outputs: Arc::new(Mutex::new(outputs)),
        }
    }
}

impl CommandRunner for QueueRunner {
    fn run(&self, _command: GcloudCommand) -> Result<CommandOutput, CommandError> {
        self.outputs.lock().unwrap().remove(0)
    }
}

fn output(stdout: impl Into<String>) -> Result<CommandOutput, CommandError> {
    Ok(CommandOutput {
        stdout: stdout.into(),
        stderr: String::new(),
    })
}

fn config() -> GcloudConfiguration {
    GcloudConfiguration {
        name: GcloudConfigurationName::new("dev"),
        is_active: true,
        account: None,
        project: Some("dev-project".to_string()),
        region: Some("europe-west1".to_string()),
        zone: Some("europe-west1-b".to_string()),
    }
}

/// Awaits `process_next_effect_result` with a short timeout, returning
/// `true` only if a result actually arrived. Used to prove the poll ticker
/// did *not* schedule a background refresh: if it had, a result would show
/// up on the channel.
async fn result_arrived_within(session: &mut RuntimeSession, millis: u64) -> bool {
    tokio::time::timeout(
        Duration::from_millis(millis),
        session.process_next_effect_result(),
    )
    .await
    .is_ok()
}

#[tokio::test]
async fn persistent_failure_gates_poll_tick_but_manual_refresh_clears_it() {
    let runner = QueueRunner::new(vec![
        // Initial auto-load on entering the Resources workspace: a
        // permission-denied failure, which classifies as `Persistent`.
        Err(CommandError::Failed {
            stdout: String::new(),
            stderr: "permission denied".to_string(),
            exit_code: None,
        }),
        // The manual refresh's list call, which succeeds.
        output(include_str!("fixtures/gce_instances.json")),
    ]);
    let mut session =
        RuntimeSession::new_with_runner_for_test(AppState::new(), Box::new(runner.clone()));
    session
        .apply_effect_result(EffectResult::ConfigurationsLoaded {
            configurations: vec![config()],
        })
        .await;

    // Entering Resources triggers the first auto-load, which fails
    // Persistent; process that result so the gate gets recorded.
    session
        .dispatch_action(Action::Workspace(WorkspaceAction::Resources))
        .await;
    assert!(
        result_arrived_within(&mut session, 2_000).await,
        "initial auto-load result must arrive"
    );
    assert!(session.app().root.resources.compute_problem().is_some());

    // A poll tick while the active scope is gated must not schedule another
    // background refresh at all: no result shows up on the channel.
    session.poll_active_resource().await;
    assert!(
        !result_arrived_within(&mut session, 200).await,
        "poll tick must be gated after a Persistent failure and must not \
         issue another gcloud call"
    );

    // Manual refresh (`r`) always proceeds regardless of the gate, and
    // clears it on success.
    session
        .dispatch_action(Action::Resources(ResourcesAction::Refresh))
        .await;
    assert!(
        result_arrived_within(&mut session, 2_000).await,
        "manual refresh must always issue a gcloud call, gate or no gate"
    );
    assert!(
        session.app().root.resources.compute_problem().is_none(),
        "manual refresh succeeded, so the stale problem must be cleared"
    );
}
