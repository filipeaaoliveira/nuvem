use std::sync::{Arc, Mutex};

use nuvem::app::state::operations::OperationOutcome;
use nuvem::app::{Action, AppState, CommandAction, EffectResult, ResourcesAction};
use nuvem::services::gcloud::cli::{
    CommandCeiling, CommandError, CommandOutput, CommandRunner, GcloudCommand,
};
use nuvem::services::runtime::session::RuntimeSession;
use nuvem_domain::gcp::configuration::{GcloudConfiguration, GcloudConfigurationName};
use nuvem_domain::gcp::resource::ResourceServiceKind;

const INSTANCES: &str = include_str!("fixtures/cloudsql_instances.json");

/// Thread-safe runner that answers `sql instances list` with a fixture and any
/// `patch` with success, recording every command it receives. It tolerates the
/// extra background list calls the runtime schedules (auto-load + refresh).
#[derive(Clone)]
struct FakeRunner {
    commands: Arc<Mutex<Vec<GcloudCommand>>>,
}

impl FakeRunner {
    fn new() -> Self {
        Self {
            commands: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn commands(&self) -> Vec<GcloudCommand> {
        self.commands.lock().unwrap().clone()
    }
}

impl CommandRunner for FakeRunner {
    fn run(&self, command: GcloudCommand) -> Result<CommandOutput, CommandError> {
        self.commands.lock().unwrap().push(command.clone());
        let args = &command.args;
        if args.windows(3).any(|w| w == ["sql", "instances", "list"]) {
            return Ok(CommandOutput {
                stdout: INSTANCES.to_string(),
                stderr: String::new(),
            });
        }
        Ok(CommandOutput {
            stdout: String::new(),
            stderr: String::new(),
        })
    }
}

fn config() -> GcloudConfiguration {
    GcloudConfiguration {
        name: GcloudConfigurationName::new("dev"),
        is_active: true,
        account: None,
        project: Some("dev-project".to_string()),
        region: Some("us-central1".to_string()),
        zone: Some("us-central1-a".to_string()),
    }
}

fn index_of(commands: &[GcloudCommand], pattern: &[&str]) -> Option<usize> {
    commands.iter().position(|command| {
        command
            .args
            .windows(pattern.len())
            .any(|w| w.iter().map(String::as_str).eq(pattern.iter().copied()))
    })
}

#[tokio::test]
async fn confirmed_stop_patches_activation_policy_and_refreshes_in_background() {
    let runner = FakeRunner::new();
    let mut session =
        RuntimeSession::new_with_runner_for_test(AppState::new(), Box::new(runner.clone()));

    session
        .apply_effect_result(EffectResult::ConfigurationsLoaded {
            configurations: vec![config()],
        })
        .await;

    // Select Cloud SQL and enter Resources so the palette exposes its actions.
    session
        .dispatch_action(Action::Resources(ResourcesAction::SelectService(
            ResourceServiceKind::CloudSql,
        )))
        .await;
    // Drain the background load triggered by selecting the service so its rows
    // land before we drive the palette.
    assert!(session.process_next_effect_result().await);
    session
        .dispatch_action(Action::Workspace(nuvem::app::WorkspaceAction::Resources))
        .await;

    // The first visible row (mysql-ha) is RUNNABLE, so Stop is eligible.
    assert_eq!(
        session
            .app()
            .root
            .resources
            .selected_cloud_sql_instance()
            .map(|instance| instance.name.as_str()),
        Some("mysql-ha")
    );

    // Open the confirmation via the shared command-palette + confirmation path.
    session
        .dispatch_action(Action::Command(CommandAction::Open))
        .await;
    for ch in "Stop selected Cloud SQL instance".chars() {
        session
            .dispatch_action(Action::Command(CommandAction::AppendChar(ch)))
            .await;
    }
    session
        .dispatch_action(Action::Command(CommandAction::Submit))
        .await;
    assert!(
        session.app().root.resources.action_confirmation().is_some(),
        "Stop should open the shared confirmation"
    );

    session
        .dispatch_action(Action::Resources(ResourcesAction::ConfirmPendingAction))
        .await;
    // The action runs on a blocking thread, so its result — and
    // with it the operations-log entry it settles — arrives through the queue.
    assert!(session.process_next_effect_result().await);

    // (a) FakeRunner received the exact patch argv.
    let commands = runner.commands();
    let patch_index = index_of(
        &commands,
        &[
            "sql",
            "instances",
            "patch",
            "mysql-ha",
            "--activation-policy=NEVER",
        ],
    )
    .expect("Stop must issue an activation-policy=NEVER patch");

    // (b) The operation record resolves to Succeeded.
    let record = session
        .app()
        .root
        .system
        .operations
        .records()
        .iter()
        .find(|record| record.kind == "Stop instance" && record.target == "mysql-ha")
        .expect("a Stop operation should be logged");
    assert_eq!(record.outcome, OperationOutcome::Succeeded);

    // (c) A follow-up RefreshCloudSqlInstances is issued after the patch.
    assert!(session.process_next_effect_result().await);
    let commands = runner.commands();
    let refresh_index = commands
        .iter()
        .enumerate()
        .skip(patch_index + 1)
        .find(|(_, command)| {
            command
                .args
                .windows(3)
                .any(|w| w == ["sql", "instances", "list"])
        })
        .map(|(index, _)| index);
    assert!(
        refresh_index.is_some(),
        "a Cloud SQL list refresh must follow the patch, commands: {commands:?}"
    );
}

/// A runner that answers the listing and times out every lifecycle command, the
/// way `run` does when a failover outlives its ceiling: the child is killed, so
/// what comes back is nuvem's own verdict rather than the CLI's.
#[derive(Clone)]
struct TimingOutRunner {
    commands: Arc<Mutex<Vec<GcloudCommand>>>,
}

impl TimingOutRunner {
    fn new() -> Self {
        Self {
            commands: Arc::new(Mutex::new(Vec::new())),
        }
    }
}

impl CommandRunner for TimingOutRunner {
    fn run(&self, command: GcloudCommand) -> Result<CommandOutput, CommandError> {
        self.commands.lock().unwrap().push(command.clone());
        if command
            .args
            .windows(3)
            .any(|w| w == ["sql", "instances", "list"])
        {
            return Ok(CommandOutput {
                stdout: INSTANCES.to_string(),
                stderr: String::new(),
            });
        }
        Err(CommandError::TimedOut {
            seconds: 1800,
            ceiling: CommandCeiling::Lifecycle,
        })
    }
}

/// A timed-out Cloud SQL action is reported as an unknown failure that may
/// still be running, not as a network problem.
///
/// `run_cloud_sql_action_result` built its `AppError` with `AppError::gcloud`,
/// which leaves `kind_hint` empty — so the kind was inferred from the detail,
/// and the detail says "timed out", one of `classify_gcloud_stderr`'s Network
/// needles. The operator was told to check their network connection for a
/// failover nuvem had killed at its own ceiling. The Compute
/// counterpart goes through `error.into()` and was always right, which is why
/// this pins the path that was wrong, end to end rather than over the `From`
/// impl alone.
#[tokio::test]
async fn a_timed_out_cloud_sql_action_is_not_reported_as_a_network_failure() {
    let runner = TimingOutRunner::new();
    let mut session =
        RuntimeSession::new_with_runner_for_test(AppState::new(), Box::new(runner.clone()));

    session
        .apply_effect_result(EffectResult::ConfigurationsLoaded {
            configurations: vec![config()],
        })
        .await;
    session
        .dispatch_action(Action::Resources(ResourcesAction::SelectService(
            ResourceServiceKind::CloudSql,
        )))
        .await;
    assert!(session.process_next_effect_result().await);
    session
        .dispatch_action(Action::Workspace(nuvem::app::WorkspaceAction::Resources))
        .await;

    session
        .dispatch_action(Action::Command(CommandAction::Open))
        .await;
    for ch in "Stop selected Cloud SQL instance".chars() {
        session
            .dispatch_action(Action::Command(CommandAction::AppendChar(ch)))
            .await;
    }
    session
        .dispatch_action(Action::Command(CommandAction::Submit))
        .await;
    session
        .dispatch_action(Action::Resources(ResourcesAction::ConfirmPendingAction))
        .await;
    assert!(session.process_next_effect_result().await);

    let record = session
        .app()
        .root
        .system
        .operations
        .records()
        .iter()
        .find(|record| record.kind == "Stop instance" && record.target == "mysql-ha")
        .expect("a Stop operation should be logged")
        .clone();

    assert_eq!(record.outcome, OperationOutcome::Failed);
    assert_eq!(
        record.detail.as_deref(),
        Some("unknown error"),
        "a call nuvem killed at its own ceiling is not a network failure"
    );
    let status = session
        .app()
        .root
        .system
        .status_message
        .clone()
        .expect("a failed action reports itself");
    assert!(
        status.contains("may still be running"),
        "the operator must be told the operation may have continued server-side: {status}"
    );
}
