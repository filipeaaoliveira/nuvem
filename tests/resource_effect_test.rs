use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use nuvem::app::state::resources::ResourceLoadPhase;
use nuvem::app::state::system::OperationPhase;
use nuvem::app::{Action, AppError, AppState, EffectResult, ResourcesAction, WorkspaceAction};
use nuvem::services::gcloud::cli::{CommandError, CommandOutput, CommandRunner, GcloudCommand};
use nuvem::services::runtime::session::RuntimeSession;
use nuvem_domain::gcp::configuration::{GcloudConfiguration, GcloudConfigurationName};
use nuvem_domain::gcp::resource::ResourceServiceKind;

struct FakeRunner {
    outputs: Mutex<Vec<Result<CommandOutput, CommandError>>>,
    commands: Mutex<Vec<GcloudCommand>>,
}

impl FakeRunner {
    fn new(outputs: Vec<Result<CommandOutput, CommandError>>) -> Self {
        Self {
            outputs: Mutex::new(outputs),
            commands: Mutex::new(Vec::new()),
        }
    }
}

struct SharedRunner(Arc<FakeRunner>);

impl SharedRunner {
    fn new(runner: Arc<FakeRunner>) -> Self {
        Self(runner)
    }
}

impl CommandRunner for SharedRunner {
    fn run(&self, command: GcloudCommand) -> Result<CommandOutput, CommandError> {
        self.0
            .commands
            .lock()
            .expect("commands mutex poisoned")
            .push(command);
        self.0
            .outputs
            .lock()
            .expect("outputs mutex poisoned")
            .remove(0)
    }
}

fn output(stdout: impl Into<String>) -> Result<CommandOutput, CommandError> {
    Ok(CommandOutput {
        stdout: stdout.into(),
        stderr: String::new(),
    })
}

fn active_dev() -> GcloudConfiguration {
    GcloudConfiguration {
        name: GcloudConfigurationName::new("dev"),
        is_active: true,
        account: Some("dev@example.com".to_string()),
        project: Some("dev-project".to_string()),
        region: None,
        zone: None,
    }
}

#[tokio::test]
async fn stale_result_does_not_complete_the_current_operation() {
    let mut session = RuntimeSession::new_for_test(AppState::new());
    session
        .apply_effect_result(EffectResult::ConfigurationsLoaded {
            configurations: vec![active_dev()],
        })
        .await;
    let operation_before_stale_result = session.app().root.system.operation.clone();
    let refresh_time_before_stale_result = session.app().root.system.last_refresh_at;
    let stale_context = nuvem_domain::gcp::configuration::GcloudContext {
        configuration: GcloudConfigurationName::new("stale"),
        ..active_dev().context()
    };

    session
        .apply_effect_result(EffectResult::ComputeInstancesLoadFailed {
            context: stale_context,
            error: AppError::gcloud("stale result"),
        })
        .await;

    assert_eq!(
        session.app().root.system.operation,
        operation_before_stale_result
    );
    assert_eq!(
        session.app().root.system.last_refresh_at,
        refresh_time_before_stale_result
    );
}

#[tokio::test]
async fn refresh_loads_instances_through_native_gcloud_runner() {
    let runner = Arc::new(FakeRunner::new(vec![output(include_str!(
        "fixtures/gce_instances.json"
    ))]));
    let mut session = RuntimeSession::new_with_runner_for_test(
        AppState::new(),
        Box::new(SharedRunner::new(runner.clone())),
    );
    session
        .apply_effect_result(EffectResult::ConfigurationsLoaded {
            configurations: vec![active_dev()],
        })
        .await;

    session
        .dispatch_action(Action::Resources(ResourcesAction::Refresh))
        .await;
    assert!(session.process_next_effect_result().await);

    assert_eq!(session.app().root.resources.instances().len(), 2);
    assert_eq!(
        session.app().root.resources.instances()[0].name,
        "api-dev-1"
    );
    assert_eq!(
        runner.commands.lock().expect("commands mutex poisoned")[0].args,
        vec![
            "--configuration".to_string(),
            "dev".to_string(),
            "compute".to_string(),
            "instances".to_string(),
            "list".to_string(),
            "--format=json".to_string(),
            "--limit=500".to_string(),
        ]
    );
}

#[tokio::test]
async fn applying_filter_pushes_server_side_filter_to_the_compute_list_command() {
    let runner = Arc::new(FakeRunner::new(vec![
        output(include_str!("fixtures/gce_instances.json")),
        output(include_str!("fixtures/gce_instances.json")),
    ]));
    let mut session = RuntimeSession::new_with_runner_for_test(
        AppState::new(),
        Box::new(SharedRunner::new(runner.clone())),
    );
    session
        .apply_effect_result(EffectResult::ConfigurationsLoaded {
            configurations: vec![active_dev()],
        })
        .await;

    session
        .dispatch_action(Action::Resources(ResourcesAction::Refresh))
        .await;
    assert!(session.process_next_effect_result().await);

    session
        .dispatch_action(Action::Resources(ResourcesAction::StartFilter))
        .await;
    session
        .dispatch_action(Action::Resources(ResourcesAction::AppendFilterChar('a')))
        .await;
    session
        .dispatch_action(Action::Resources(ResourcesAction::ApplyFilter))
        .await;
    assert!(session.process_next_effect_result().await);

    let commands = runner.commands.lock().expect("commands mutex poisoned");
    let last = commands.last().expect("at least one command was recorded");
    assert!(
        last.args.contains(
            &r#"--filter=name ~ "(?i)a" OR zone ~ "(?i)a" OR status ~ "(?i)a" OR machineType ~ "(?i)a" OR networkInterfaces.networkIP ~ "(?i)a" OR networkInterfaces.accessConfigs.natIP ~ "(?i)a""#
                .to_string()
        ),
        "expected a --filter token in {:?}",
        last.args
    );
    assert!(
        last.args.contains(&"--limit=500".to_string()),
        "expected a --limit=500 token in {:?}",
        last.args
    );
}

/// Bounded gate for one effect result (never an unbounded wait, never
/// a tight wall-clock assertion). 30s is a generous ceiling for an in-process
/// fake runner; it only ever fires when the result genuinely never arrives.
async fn next_result_within_30s(session: &mut RuntimeSession) -> bool {
    tokio::time::timeout(
        Duration::from_secs(30),
        session.process_next_effect_result(),
    )
    .await
    .expect("an effect result must arrive within the bounded gate")
}

/// The blocker: the background poll always refreshes the ACTIVE service,
/// and under label mode there is no client-side narrowing left to re-apply —
/// `CompiledFilter::Label` keeps every row, so whatever the poll downloads IS
/// the visible set. An unfiltered poll would replace a label-filtered table
/// with the whole project while the tools line still reads `[label]`.
#[tokio::test]
async fn background_poll_pushes_the_active_label_filter_to_the_list_command() {
    let runner = Arc::new(FakeRunner::new(vec![
        output(include_str!("fixtures/gce_instances.json")),
        output(include_str!("fixtures/gce_instances.json")),
        output(include_str!("fixtures/gce_instances.json")),
        output(include_str!("fixtures/gce_instances.json")),
    ]));
    let mut session = RuntimeSession::new_with_runner_for_test(
        AppState::new(),
        Box::new(SharedRunner::new(runner.clone())),
    );
    session
        .apply_effect_result(EffectResult::ConfigurationsLoaded {
            configurations: vec![active_dev()],
        })
        .await;

    // Entering Resources auto-loads GCE unfiltered; drain that result so the
    // poll gate sees no in-flight refresh.
    session
        .dispatch_action(Action::Workspace(WorkspaceAction::Resources))
        .await;
    assert!(next_result_within_30s(&mut session).await);

    session
        .dispatch_action(Action::Resources(ResourcesAction::StartFilter))
        .await;
    for value in "-l env=prod".chars() {
        session
            .dispatch_action(Action::Resources(ResourcesAction::AppendFilterChar(value)))
            .await;
    }
    session
        .dispatch_action(Action::Resources(ResourcesAction::ApplyFilter))
        .await;
    assert!(next_result_within_30s(&mut session).await);

    let before_poll = runner
        .commands
        .lock()
        .expect("commands mutex poisoned")
        .len();

    session.poll_active_resource().await;
    assert!(next_result_within_30s(&mut session).await);

    let commands = runner.commands.lock().expect("commands mutex poisoned");
    let polled = commands
        .get(before_poll)
        .expect("the poll tick must issue a list command");
    assert!(
        polled
            .args
            .contains(&r#"--filter=labels.env = "prod""#.to_string()),
        "the background poll must carry the active label filter, got {:?}",
        polled.args
    );
}

#[tokio::test]
async fn select_service_loads_gke_clusters_through_native_gcloud_runner() {
    let runner = Arc::new(FakeRunner::new(vec![output(include_str!(
        "fixtures/gke_clusters.json"
    ))]));
    let mut session = RuntimeSession::new_with_runner_for_test(
        AppState::new(),
        Box::new(SharedRunner::new(runner.clone())),
    );
    session
        .apply_effect_result(EffectResult::ConfigurationsLoaded {
            configurations: vec![active_dev()],
        })
        .await;
    session
        .dispatch_action(Action::Resources(ResourcesAction::SelectService(
            ResourceServiceKind::GkeClusters,
        )))
        .await;

    assert!(session.process_next_effect_result().await);

    assert_eq!(session.app().root.resources.gke_clusters().len(), 3);
    assert_eq!(
        runner.commands.lock().expect("commands mutex poisoned")[0].args,
        vec![
            "--configuration".to_string(),
            "dev".to_string(),
            "container".to_string(),
            "clusters".to_string(),
            "list".to_string(),
            "--format=json".to_string(),
            "--limit=500".to_string(),
        ]
    );
}

#[tokio::test]
async fn opening_selected_gke_cluster_loads_detail_through_native_gcloud_runner() {
    let runner = Arc::new(FakeRunner::new(vec![
        output(include_str!("fixtures/gke_clusters.json")),
        output(include_str!("fixtures/gke_cluster_detail.json")),
    ]));
    let mut session = RuntimeSession::new_with_runner_for_test(
        AppState::new(),
        Box::new(SharedRunner::new(runner.clone())),
    );
    session
        .apply_effect_result(EffectResult::ConfigurationsLoaded {
            configurations: vec![active_dev()],
        })
        .await;
    session
        .dispatch_action(Action::Resources(ResourcesAction::SelectService(
            ResourceServiceKind::GkeClusters,
        )))
        .await;
    assert!(session.process_next_effect_result().await);
    session
        .dispatch_action(Action::Resources(ResourcesAction::SelectNext))
        .await;

    session
        .dispatch_action(Action::Resources(ResourcesAction::OpenSelected))
        .await;

    assert_eq!(
        session
            .app()
            .root
            .resources
            .gke_detail()
            .map(|detail| detail.name.as_str()),
        Some("platform-dev")
    );
    assert_eq!(
        runner.commands.lock().expect("commands mutex poisoned")[1].args,
        vec![
            "--configuration".to_string(),
            "dev".to_string(),
            "container".to_string(),
            "clusters".to_string(),
            "describe".to_string(),
            "platform-dev".to_string(),
            "--location".to_string(),
            "europe-west1".to_string(),
            "--format=json".to_string(),
        ]
    );
}

#[tokio::test]
async fn refresh_command_failure_sets_gcloud_status_and_loads_no_rows() {
    let runner = Arc::new(FakeRunner::new(vec![Err(CommandError::Failed {
        stdout: String::new(),
        stderr: "compute API disabled".to_string(),
        exit_code: None,
    })]));
    let mut session = RuntimeSession::new_with_runner_for_test(
        AppState::new(),
        Box::new(SharedRunner::new(runner.clone())),
    );
    session
        .apply_effect_result(EffectResult::ConfigurationsLoaded {
            configurations: vec![active_dev()],
        })
        .await;

    session
        .dispatch_action(Action::Resources(ResourcesAction::Refresh))
        .await;
    assert!(session.process_next_effect_result().await);

    assert!(session.app().root.resources.instances().is_empty());
    let status = session
        .app()
        .root
        .system
        .status_message
        .as_deref()
        .unwrap_or_default();
    assert_eq!(
        status, "unknown error — Retry, or copy the details to investigate",
        "status was {status}"
    );
    assert!(
        !status.contains("compute API disabled"),
        "status leaked raw stderr: {status}"
    );
    assert_eq!(
        session.app().root.system.operation.phase,
        OperationPhase::Degraded
    );
    assert_eq!(session.app().root.system.operation.label, "GCE instances");
    assert!(session.app().root.system.last_refresh_at.is_some());
    assert_eq!(
        runner
            .commands
            .lock()
            .expect("commands mutex poisoned")
            .len(),
        1
    );
}

/// FIX 2 (AC#2): a compute-list failure's diagnostic must carry a safe,
/// static command summary so `copy_text()` includes a `command:` line, and
/// that summary must never leak the instance name/project/account/flags. The
/// subject is the summary alone: `copy_text()` also carries a `detail:` line
/// of sanitized stderr, which may legitimately name a resource.
#[tokio::test]
async fn compute_list_failure_records_command_summary_with_no_instance_leak() {
    let runner = Arc::new(FakeRunner::new(vec![Err(CommandError::Failed {
        stdout: String::new(),
        stderr: "permission denied for instance my-secret-instance-42".to_string(),
        exit_code: Some(1),
    })]));
    let mut session = RuntimeSession::new_with_runner_for_test(
        AppState::new(),
        Box::new(SharedRunner::new(runner.clone())),
    );
    session
        .apply_effect_result(EffectResult::ConfigurationsLoaded {
            configurations: vec![active_dev()],
        })
        .await;

    session
        .dispatch_action(Action::Resources(ResourcesAction::Refresh))
        .await;
    assert!(session.process_next_effect_result().await);

    let diag = session
        .app()
        .root
        .system
        .last_diagnostic()
        .expect("last_diagnostic should be set after the failure");
    assert_eq!(
        diag.command_summary.as_deref(),
        Some("gcloud compute instances list")
    );
    let copy_text = diag.copy_text();
    assert!(copy_text.contains("command: gcloud compute instances list"));
    let command_line = copy_text
        .lines()
        .find(|l| l.starts_with("command: "))
        .expect("copy_text carries a command line");
    assert!(!command_line.contains("my-secret-instance-42"));
}

#[tokio::test]
async fn refresh_with_malformed_json_sets_parse_status_and_loads_no_rows() {
    let runner = Arc::new(FakeRunner::new(vec![output("{not-json")]));
    let mut session = RuntimeSession::new_with_runner_for_test(
        AppState::new(),
        Box::new(SharedRunner::new(runner.clone())),
    );
    session
        .apply_effect_result(EffectResult::ConfigurationsLoaded {
            configurations: vec![active_dev()],
        })
        .await;

    session
        .dispatch_action(Action::Resources(ResourcesAction::Refresh))
        .await;
    assert!(session.process_next_effect_result().await);

    assert!(session.app().root.resources.instances().is_empty());
    assert_eq!(
        session.app().root.system.status_message.as_deref(),
        Some("parse error — Update nuvem — gcloud output did not match the expected schema")
    );
    assert_eq!(
        runner
            .commands
            .lock()
            .expect("commands mutex poisoned")
            .len(),
        1
    );
}

#[tokio::test]
async fn refresh_command_failure_after_success_marks_existing_rows_stale() {
    let runner = Arc::new(FakeRunner::new(vec![
        output(include_str!("fixtures/gce_instances.json")),
        Err(CommandError::Failed {
            stdout: String::new(),
            stderr: "compute API disabled".to_string(),
            exit_code: None,
        }),
    ]));
    let mut session = RuntimeSession::new_with_runner_for_test(
        AppState::new(),
        Box::new(SharedRunner::new(runner.clone())),
    );
    session
        .apply_effect_result(EffectResult::ConfigurationsLoaded {
            configurations: vec![active_dev()],
        })
        .await;

    session
        .dispatch_action(Action::Resources(ResourcesAction::Refresh))
        .await;
    assert!(session.process_next_effect_result().await);
    assert_eq!(session.app().root.resources.instances().len(), 2);
    assert!(!session.app().root.resources.is_stale());

    session
        .dispatch_action(Action::Resources(ResourcesAction::Refresh))
        .await;
    assert!(session.process_next_effect_result().await);

    assert!(session.app().root.resources.is_stale());
    assert!(session.app().root.resources.instances().is_empty());
    let status = session
        .app()
        .root
        .system
        .status_message
        .as_deref()
        .unwrap_or_default();
    assert_eq!(
        status, "unknown error — Retry, or copy the details to investigate",
        "status was {status}"
    );
    assert!(
        !status.contains("compute API disabled"),
        "status leaked raw stderr: {status}"
    );
    assert_eq!(
        runner
            .commands
            .lock()
            .expect("commands mutex poisoned")
            .len(),
        2
    );
}

#[tokio::test]
async fn refresh_parse_failure_after_success_marks_existing_rows_stale() {
    let runner = Arc::new(FakeRunner::new(vec![
        output(include_str!("fixtures/gce_instances.json")),
        output("{not-json"),
    ]));
    let mut session = RuntimeSession::new_with_runner_for_test(
        AppState::new(),
        Box::new(SharedRunner::new(runner.clone())),
    );
    session
        .apply_effect_result(EffectResult::ConfigurationsLoaded {
            configurations: vec![active_dev()],
        })
        .await;

    session
        .dispatch_action(Action::Resources(ResourcesAction::Refresh))
        .await;
    assert!(session.process_next_effect_result().await);
    assert_eq!(session.app().root.resources.instances().len(), 2);
    assert!(!session.app().root.resources.is_stale());

    session
        .dispatch_action(Action::Resources(ResourcesAction::Refresh))
        .await;
    assert!(session.process_next_effect_result().await);

    assert!(session.app().root.resources.is_stale());
    assert!(session.app().root.resources.instances().is_empty());
    let status = session
        .app()
        .root
        .system
        .status_message
        .as_deref()
        .unwrap_or_default();
    assert_eq!(
        status, "parse error — Update nuvem — gcloud output did not match the expected schema",
        "status was {status}"
    );
    assert_eq!(
        runner
            .commands
            .lock()
            .expect("commands mutex poisoned")
            .len(),
        2
    );
}

#[tokio::test]
async fn opening_selected_instance_loads_detail_through_native_gcloud_runner() {
    let runner = Arc::new(FakeRunner::new(vec![
        output(include_str!("fixtures/gce_instances.json")),
        output(include_str!("fixtures/gce_instance_detail.json")),
    ]));
    let mut session = RuntimeSession::new_with_runner_for_test(
        AppState::new(),
        Box::new(SharedRunner::new(runner.clone())),
    );
    session
        .apply_effect_result(EffectResult::ConfigurationsLoaded {
            configurations: vec![active_dev()],
        })
        .await;
    session
        .dispatch_action(Action::Resources(ResourcesAction::Refresh))
        .await;
    assert!(session.process_next_effect_result().await);
    let refresh_time_before_detail = session.app().root.system.last_refresh_at;

    session
        .dispatch_action(Action::Resources(ResourcesAction::OpenSelected))
        .await;

    let detail = session
        .app()
        .root
        .resources
        .detail()
        .expect("detail should load");
    assert_eq!(detail.name, "api-dev-1");
    assert_eq!(detail.machine_type, "e2-medium");
    assert_eq!(
        detail.creation_timestamp.as_deref(),
        Some("2026-07-03T10:11:12.000-07:00")
    );
    assert_eq!(detail.disks.len(), 2);
    assert_eq!(detail.disks[1].size_gb.as_deref(), Some("100"));
    assert!(
        detail
            .labels
            .iter()
            .any(|entry| entry.key == "env" && entry.value == "dev")
    );
    assert!(
        detail
            .metadata
            .iter()
            .any(|entry| entry.key == "owner" && entry.value == "platform")
    );
    assert!(
        detail
            .raw_json
            .contains("\"creationTimestamp\": \"2026-07-03T10:11:12.000-07:00\"")
    );
    assert_eq!(
        session.app().root.system.last_refresh_at,
        refresh_time_before_detail
    );
    let commands = runner.commands.lock().expect("commands mutex poisoned");
    assert_eq!(
        commands[1].args,
        vec![
            "--configuration".to_string(),
            "dev".to_string(),
            "compute".to_string(),
            "instances".to_string(),
            "describe".to_string(),
            "api-dev-1".to_string(),
            "--zone".to_string(),
            "europe-west1-b".to_string(),
            "--format=json".to_string(),
        ]
    );
}

#[tokio::test]
async fn detail_command_failure_sets_gcloud_detail_error() {
    let runner = Arc::new(FakeRunner::new(vec![
        output(include_str!("fixtures/gce_instances.json")),
        Err(CommandError::Failed {
            stdout: String::new(),
            stderr: "describe failed".to_string(),
            exit_code: None,
        }),
    ]));
    let mut session = RuntimeSession::new_with_runner_for_test(
        AppState::new(),
        Box::new(SharedRunner::new(runner.clone())),
    );
    session
        .apply_effect_result(EffectResult::ConfigurationsLoaded {
            configurations: vec![active_dev()],
        })
        .await;
    session
        .dispatch_action(Action::Resources(ResourcesAction::Refresh))
        .await;
    assert!(session.process_next_effect_result().await);

    session
        .dispatch_action(Action::Resources(ResourcesAction::OpenSelected))
        .await;

    assert!(session.app().root.resources.detail().is_none());
    let status = session
        .app()
        .root
        .system
        .status_message
        .as_deref()
        .unwrap_or_default();
    assert_eq!(
        status, "unknown error — Retry, or copy the details to investigate",
        "status was {status}"
    );
    assert!(
        !status.contains("describe failed"),
        "status leaked raw stderr: {status}"
    );
    assert_eq!(
        session.app().root.resources.detail_error(),
        Some("unknown error — Retry, or copy the details to investigate (describe failed)")
    );
}

#[tokio::test]
async fn detail_parse_failure_sets_parse_detail_error() {
    let runner = Arc::new(FakeRunner::new(vec![
        output(include_str!("fixtures/gce_instances.json")),
        output("{not-json"),
    ]));
    let mut session = RuntimeSession::new_with_runner_for_test(
        AppState::new(),
        Box::new(SharedRunner::new(runner.clone())),
    );
    session
        .apply_effect_result(EffectResult::ConfigurationsLoaded {
            configurations: vec![active_dev()],
        })
        .await;
    session
        .dispatch_action(Action::Resources(ResourcesAction::Refresh))
        .await;
    assert!(session.process_next_effect_result().await);

    session
        .dispatch_action(Action::Resources(ResourcesAction::OpenSelected))
        .await;

    let status = session
        .app()
        .root
        .system
        .status_message
        .as_deref()
        .unwrap_or_default();
    assert_eq!(
        status, "parse error — Update nuvem — gcloud output did not match the expected schema",
        "status was {status}"
    );
    // The detail panel appends a bounded, sanitized preview of the underlying
    // serde error in parentheses after the shared headline; status itself
    // stays headline-only.
    let detail_error = session
        .app()
        .root
        .resources
        .detail_error()
        .expect("detail error recorded");
    assert!(
        detail_error.starts_with(status),
        "detail error was {detail_error}"
    );
    assert!(
        detail_error.len() > status.len(),
        "detail error was {detail_error}"
    );
}

struct PanicRunner;

impl CommandRunner for PanicRunner {
    fn run(&self, _command: GcloudCommand) -> Result<CommandOutput, CommandError> {
        panic!("simulated background task panic");
    }
}

/// FIX 5 (correctness): a background-refresh task-join error (e.g. the
/// blocking task panicked) must be labeled for the *active* service at the
/// time of the refresh, not hardcoded to `ComputeInstancesLoadFailed` even
/// when the active service is GKE.
#[tokio::test]
async fn background_refresh_join_error_labels_the_active_gke_scope_correctly() {
    let mut session =
        RuntimeSession::new_with_runner_for_test(AppState::new(), Box::new(PanicRunner));
    session
        .apply_effect_result(EffectResult::ConfigurationsLoaded {
            configurations: vec![active_dev()],
        })
        .await;
    session
        .dispatch_action(Action::Resources(ResourcesAction::SelectService(
            ResourceServiceKind::GkeClusters,
        )))
        .await;

    assert!(session.process_next_effect_result().await);

    // The join-error path must still report a GkeClustersLoadFailed (the
    // active scope), never a ComputeInstancesLoadFailed.
    assert!(session.app().root.resources.gke_clusters().is_empty());
    let diag = session
        .app()
        .root
        .resources
        .service_diagnostic(ResourceServiceKind::GkeClusters);
    assert!(
        diag.is_some(),
        "expected a GKE diagnostic to be recorded for the join-error path"
    );
    assert!(
        session
            .app()
            .root
            .resources
            .service_diagnostic(ResourceServiceKind::ComputeInstances)
            .is_none(),
        "join-error path must not mislabel a GKE-scope failure as a Compute failure"
    );
}

#[tokio::test]
async fn refresh_gke_effect_loads_clusters_in_the_background() {
    use nuvem::app::Effect;
    let runner = Arc::new(FakeRunner::new(vec![
        output(include_str!("fixtures/gke_clusters.json")),
        output(include_str!("fixtures/gke_clusters.json")),
    ]));
    let mut session = RuntimeSession::new_with_runner_for_test(
        AppState::new(),
        Box::new(SharedRunner::new(runner.clone())),
    );
    session
        .apply_effect_result(EffectResult::ConfigurationsLoaded {
            configurations: vec![active_dev()],
        })
        .await;
    session
        .dispatch_action(Action::Resources(ResourcesAction::SelectService(
            ResourceServiceKind::GkeClusters,
        )))
        .await;
    assert!(session.process_next_effect_result().await);
    let context = session.app().root.current_context().expect("context");

    session
        .dispatch_action(Action::Effect(Effect::RefreshGkeClusters {
            context,
            filter: None,
        }))
        .await;
    assert!(session.process_next_effect_result().await);

    assert_eq!(session.app().root.resources.gke_clusters().len(), 3);
    assert_eq!(
        runner.commands.lock().expect("commands mutex poisoned")[0].args,
        vec![
            "--configuration".to_string(),
            "dev".to_string(),
            "container".to_string(),
            "clusters".to_string(),
            "list".to_string(),
            "--format=json".to_string(),
            "--limit=500".to_string(),
        ]
    );
}

#[tokio::test]
async fn entering_resources_auto_loads_active_service_without_r() {
    let runner = Arc::new(FakeRunner::new(vec![output(include_str!(
        "fixtures/gce_instances.json"
    ))]));
    let mut session = RuntimeSession::new_with_runner_for_test(
        AppState::new(),
        Box::new(SharedRunner::new(runner.clone())),
    );
    session
        .apply_effect_result(EffectResult::ConfigurationsLoaded {
            configurations: vec![active_dev()],
        })
        .await;

    session
        .dispatch_action(Action::Workspace(WorkspaceAction::Resources))
        .await;
    assert!(session.process_next_effect_result().await);

    assert_eq!(session.app().root.resources.instances().len(), 2);
    assert_eq!(
        runner
            .commands
            .lock()
            .expect("commands mutex poisoned")
            .len(),
        1
    );
}

#[tokio::test]
async fn failed_auto_load_does_not_refire_on_the_same_scope() {
    let runner = Arc::new(FakeRunner::new(vec![Err(CommandError::Failed {
        stdout: String::new(),
        stderr: "compute API disabled".to_string(),
        exit_code: None,
    })]));
    let mut session = RuntimeSession::new_with_runner_for_test(
        AppState::new(),
        Box::new(SharedRunner::new(runner.clone())),
    );
    session
        .apply_effect_result(EffectResult::ConfigurationsLoaded {
            configurations: vec![active_dev()],
        })
        .await;

    session
        .dispatch_action(Action::Workspace(WorkspaceAction::Resources))
        .await;
    assert!(session.process_next_effect_result().await);
    // A no-op action in the same scope must not schedule another load.
    session
        .dispatch_action(Action::Resources(ResourcesAction::SelectNext))
        .await;

    assert_eq!(session.app().root.resources.instances().len(), 0);
    assert_eq!(
        runner
            .commands
            .lock()
            .expect("commands mutex poisoned")
            .len(),
        1
    );
}

#[tokio::test]
async fn switching_service_auto_loads_the_newly_active_service() {
    let runner = Arc::new(FakeRunner::new(vec![
        output(include_str!("fixtures/gce_instances.json")),
        output(include_str!("fixtures/gke_clusters.json")),
    ]));
    let mut session = RuntimeSession::new_with_runner_for_test(
        AppState::new(),
        Box::new(SharedRunner::new(runner.clone())),
    );
    session
        .apply_effect_result(EffectResult::ConfigurationsLoaded {
            configurations: vec![active_dev()],
        })
        .await;
    session
        .dispatch_action(Action::Workspace(WorkspaceAction::Resources))
        .await;
    assert!(session.process_next_effect_result().await);
    assert_eq!(session.app().root.resources.instances().len(), 2);

    session
        .dispatch_action(Action::Resources(ResourcesAction::SelectService(
            ResourceServiceKind::GkeClusters,
        )))
        .await;
    assert!(session.process_next_effect_result().await);

    assert_eq!(session.app().root.resources.gke_clusters().len(), 3);
    let commands = runner.commands.lock().expect("commands mutex poisoned");
    assert_eq!(commands.len(), 2);
    assert_eq!(commands[1].args[2], "container");
}

#[tokio::test]
async fn activating_a_configuration_reloads_under_the_new_context() {
    let other = GcloudConfiguration {
        name: GcloudConfigurationName::new("prod"),
        is_active: true,
        account: Some("prod@example.com".to_string()),
        project: Some("prod-project".to_string()),
        region: None,
        zone: None,
    };
    let runner = Arc::new(FakeRunner::new(vec![
        output(include_str!("fixtures/gce_instances.json")),
        output(include_str!("fixtures/gce_instances.json")),
    ]));
    let mut session = RuntimeSession::new_with_runner_for_test(
        AppState::new(),
        Box::new(SharedRunner::new(runner.clone())),
    );
    session
        .apply_effect_result(EffectResult::ConfigurationsLoaded {
            configurations: vec![active_dev()],
        })
        .await;
    session
        .dispatch_action(Action::Workspace(WorkspaceAction::Resources))
        .await;
    assert!(session.process_next_effect_result().await);

    // Simulate the active configuration changing to prod.
    session
        .apply_effect_result(EffectResult::ConfigurationsLoaded {
            configurations: vec![other],
        })
        .await;
    assert!(session.process_next_effect_result().await);

    let commands = runner.commands.lock().expect("commands mutex poisoned");
    assert_eq!(commands.len(), 2);
    assert_eq!(commands[1].args[1], "prod");
}

// ---------------------------------------------------------------------------
// Cloud Run (issue #32) — runtime dispatch + logs seam.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn select_service_loads_cloud_run_services_through_native_gcloud_runner() {
    let runner = Arc::new(FakeRunner::new(vec![output(include_str!(
        "fixtures/cloud_run_services.json"
    ))]));
    let mut session = RuntimeSession::new_with_runner_for_test(
        AppState::new(),
        Box::new(SharedRunner::new(runner.clone())),
    );
    session
        .apply_effect_result(EffectResult::ConfigurationsLoaded {
            configurations: vec![active_dev()],
        })
        .await;
    session
        .dispatch_action(Action::Resources(ResourcesAction::SelectService(
            ResourceServiceKind::CloudRunServices,
        )))
        .await;

    assert!(session.process_next_effect_result().await);

    assert_eq!(session.app().root.resources.cloud_run_services().len(), 5);
    assert_eq!(
        session.app().root.resources.cloud_run_services()[0].name,
        "orders-api"
    );
    assert_eq!(
        runner.commands.lock().expect("commands mutex poisoned")[0].args,
        vec![
            "--configuration".to_string(),
            "dev".to_string(),
            "--project".to_string(),
            "dev-project".to_string(),
            "run".to_string(),
            "services".to_string(),
            "list".to_string(),
            "--platform=managed".to_string(),
            "--format=json".to_string(),
            "--limit=500".to_string(),
        ]
    );
}

#[tokio::test]
async fn opening_selected_cloud_run_service_loads_detail_and_revisions() {
    let runner = Arc::new(FakeRunner::new(vec![
        output(include_str!("fixtures/cloud_run_services.json")),
        output(include_str!("fixtures/cloud_run_service_detail.json")),
        output(include_str!("fixtures/cloud_run_revisions.json")),
    ]));
    let mut session = RuntimeSession::new_with_runner_for_test(
        AppState::new(),
        Box::new(SharedRunner::new(runner.clone())),
    );
    session
        .apply_effect_result(EffectResult::ConfigurationsLoaded {
            configurations: vec![active_dev()],
        })
        .await;
    session
        .dispatch_action(Action::Resources(ResourcesAction::SelectService(
            ResourceServiceKind::CloudRunServices,
        )))
        .await;
    assert!(session.process_next_effect_result().await);

    // Services sort by name ascending, so orders-api is the 4th visible row.
    for _ in 0..3 {
        session
            .dispatch_action(Action::Resources(ResourcesAction::SelectNext))
            .await;
    }

    session
        .dispatch_action(Action::Resources(ResourcesAction::OpenSelected))
        .await;

    // Detail (describe) landed.
    assert_eq!(
        session
            .app()
            .root
            .resources
            .cloud_run_detail()
            .map(|detail| detail.name.as_str()),
        Some("orders-api")
    );
    // Revisions (revisions list) merged into the detail.
    assert_eq!(
        session
            .app()
            .root
            .resources
            .cloud_run_detail()
            .map(|detail| detail.revisions.len()),
        Some(2)
    );

    let commands = runner.commands.lock().expect("commands mutex poisoned");
    assert_eq!(
        commands[1].args,
        vec![
            "--configuration".to_string(),
            "dev".to_string(),
            "--project".to_string(),
            "dev-project".to_string(),
            "run".to_string(),
            "services".to_string(),
            "describe".to_string(),
            "orders-api".to_string(),
            "--platform=managed".to_string(),
            "--region".to_string(),
            "europe-west1".to_string(),
            "--format=json".to_string(),
        ]
    );
    assert_eq!(
        commands[2].args,
        vec![
            "--configuration".to_string(),
            "dev".to_string(),
            "--project".to_string(),
            "dev-project".to_string(),
            "run".to_string(),
            "revisions".to_string(),
            "list".to_string(),
            "--platform=managed".to_string(),
            "--region".to_string(),
            "europe-west1".to_string(),
            "--service".to_string(),
            "orders-api".to_string(),
            "--format=json".to_string(),
        ]
    );
}

/// Holds the `ordinal`-th (1-based) compute list command a runner sees until
/// the test releases it, so two list fetches can be made to overlap
/// deterministically.
///
/// Gating by call ordinal rather than by how the command happens to be spelled
/// keeps the gate from silently drifting: it records whether it ever engaged
/// ([`Self::await_engaged`]) so a test whose second fetch stops happening fails
/// on the gate instead of proving nothing, and it waits for the release with a
/// timeout so a never-released gate fails instead of hanging forever.
struct ComputeListGate {
    ordinal: usize,
    calls: AtomicUsize,
    engaged: AtomicBool,
    release: Mutex<Option<std::sync::mpsc::Receiver<()>>>,
}

impl ComputeListGate {
    fn holding_call(ordinal: usize, release: std::sync::mpsc::Receiver<()>) -> Arc<Self> {
        Arc::new(Self {
            ordinal,
            calls: AtomicUsize::new(0),
            engaged: AtomicBool::new(false),
            release: Mutex::new(Some(release)),
        })
    }

    /// Called by the runner for every compute list command; blocks the gated
    /// one until the test releases it.
    fn arrive(&self) {
        if self.calls.fetch_add(1, Ordering::SeqCst) + 1 != self.ordinal {
            return;
        }
        let release = self
            .release
            .lock()
            .expect("release mutex poisoned")
            .take()
            .expect("gate engages only once");
        self.engaged.store(true, Ordering::SeqCst);
        release
            .recv_timeout(Duration::from_secs(30))
            .expect("gated compute list command was never released");
    }

    /// Waits (bounded) for the gated command to reach the runner, so the test
    /// never asserts on an overlap that did not happen.
    async fn await_engaged(&self) {
        for _ in 0..3_000 {
            if self.engaged.load(Ordering::SeqCst) {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("the gated compute list command never reached the runner");
    }

    /// Waits (bounded) for `count` compute list commands to have reached the
    /// runner.
    ///
    /// Fetches run on independently spawned tasks, so "dispatched" does not
    /// mean "arrived". A test that starts a second fetch before the first one
    /// has actually reached the runner would let the call ordinal latch onto
    /// whichever arrived first and hold the wrong fetch; awaiting the earlier
    /// arrivals makes the ordinal mean what the test thinks it means.
    async fn await_calls(&self, count: usize) {
        for _ in 0..3_000 {
            if self.calls.load(Ordering::SeqCst) >= count {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!(
            "only {} of {count} expected compute list commands reached the runner",
            self.calls.load(Ordering::SeqCst)
        );
    }
}

fn is_compute_list(command: &GcloudCommand) -> bool {
    command
        .args
        .windows(3)
        .any(|window| window[0] == "compute" && window[1] == "instances" && window[2] == "list")
}

/// The `auth list` diagnostics probe `WorkspaceAction::Home` fires.
fn is_auth_probe(command: &GcloudCommand) -> bool {
    command
        .args
        .windows(2)
        .any(|window| window[0] == "auth" && window[1] == "list")
}

/// Serves the shared fixture queue like [`SharedRunner`], with two additions
/// the detached-load tests need:
///
/// * it answers the `auth list` diagnostics probe that
///   `WorkspaceAction::Home` fires, so the probe neither drains the fixture
///   queue nor panics the blocking pool;
/// * given a [`ComputeListGate`] it can hold one compute list command (the
///   newer of two overlapping fetches) until the test releases it, so the only
///   result the session can receive meanwhile is the older, superseded fetch's.
struct DetachedLoadRunner {
    inner: Arc<FakeRunner>,
    gate: Option<Arc<ComputeListGate>>,
}

impl DetachedLoadRunner {
    fn new(inner: Arc<FakeRunner>) -> Self {
        Self { inner, gate: None }
    }

    fn with_gate(mut self, gate: Arc<ComputeListGate>) -> Self {
        self.gate = Some(gate);
        self
    }

    fn record(&self, command: GcloudCommand) {
        self.inner
            .commands
            .lock()
            .expect("commands mutex poisoned")
            .push(command);
    }
}

impl CommandRunner for DetachedLoadRunner {
    fn run(&self, command: GcloudCommand) -> Result<CommandOutput, CommandError> {
        if command.args.iter().any(|arg| arg == "auth") {
            // Recorded like every other command, so assertions over the command
            // log see the probe instead of silently missing it.
            self.record(command);
            return output(r#"[{"account":"dev@example.com","status":"ACTIVE"}]"#);
        }
        if let Some(gate) = &self.gate
            && is_compute_list(&command)
        {
            gate.arrive();
        }
        self.record(command);
        self.inner
            .outputs
            .lock()
            .expect("outputs mutex poisoned")
            .remove(0)
    }
}

fn compute_load_phase(session: &RuntimeSession) -> &ResourceLoadPhase {
    session
        .app()
        .root
        .resources
        .load_phase(ResourceServiceKind::ComputeInstances)
}

/// Generous bound for a single `process_next_effect_result()`: the tests below
/// complete in milliseconds, so this only ever fires on a broken assumption.
const EFFECT_RESULT_TIMEOUT: Duration = Duration::from_secs(30);

/// Awaits one queued effect result under a bound. `process_next_effect_result`
/// awaits an unbounded `recv()`, so a test that expects more results than were
/// actually queued would hang forever instead of failing; `label` names the
/// awaited result so a timeout says which one never arrived.
async fn process_next_effect_result(session: &mut RuntimeSession, label: &str) -> bool {
    tokio::time::timeout(EFFECT_RESULT_TIMEOUT, session.process_next_effect_result())
        .await
        .unwrap_or_else(|_| {
            panic!(
                "timed out after {EFFECT_RESULT_TIMEOUT:?} awaiting effect result \
                 ({label}): no result was queued, so this test's expectation of how \
                 many results the runtime produces is wrong"
            )
        })
}

/// Leaving the Resources workspace while a list fetch is in flight
/// detaches that fetch — the generation guard drops its result. Nothing else
/// resolved the `Loading` phase it installed (`ensure_active_resource_loaded`
/// bails outside the Resources workspace and so reschedules nothing), which
/// left the spinner ticking forever until a manual refresh.
#[tokio::test]
async fn leaving_resources_mid_load_resolves_the_detached_load_phase() {
    let runner = Arc::new(FakeRunner::new(vec![output(include_str!(
        "fixtures/gce_instances.json"
    ))]));
    let mut session = RuntimeSession::new_with_runner_for_test(
        AppState::new(),
        Box::new(DetachedLoadRunner::new(runner.clone())),
    );
    session
        .apply_effect_result(EffectResult::ConfigurationsLoaded {
            configurations: vec![active_dev()],
        })
        .await;

    // Entering Resources auto-schedules the active service's list fetch.
    session
        .dispatch_action(Action::Workspace(WorkspaceAction::Resources))
        .await;
    assert!(
        matches!(
            compute_load_phase(&session),
            ResourceLoadPhase::Loading { .. }
        ),
        "entering Resources must put the active service in the Loading phase"
    );

    session
        .dispatch_action(Action::Workspace(WorkspaceAction::Home))
        .await;
    // Still Loading: the fetch really is in flight, so this is the honest
    // state. Clearing it *here* is the rejected switch-site fix — it also
    // suppresses the Loading affordance a fresh fetch must show right after a
    // context change.
    assert!(
        matches!(
            compute_load_phase(&session),
            ResourceLoadPhase::Loading { .. }
        ),
        "switching workspace must not pre-emptively clear a still-in-flight load"
    );

    // Two results are now queued: the detached compute refresh and the
    // diagnostics probe Home fired. The generation guard drops the refresh,
    // and the load it abandoned must resolve so the spinner goes away.
    assert!(
        process_next_effect_result(
            &mut session,
            "1 of 2: detached compute refresh / Home's diagnostics probe, either order",
        )
        .await
    );
    assert!(
        process_next_effect_result(
            &mut session,
            "2 of 2: detached compute refresh / Home's diagnostics probe, either order",
        )
        .await
    );
    assert_eq!(
        compute_load_phase(&session),
        &ResourceLoadPhase::Idle,
        "a dropped detached result must resolve the load phase it left behind"
    );
    // And nothing reschedules the fetch: outside the Resources workspace
    // `ensure_active_resource_loaded` bails, so the resolved phase is stable
    // and no further command is issued. `SelectNext` alone would not reschedule
    // even inside Resources — the command log below is what pins this.
    session
        .dispatch_action(Action::Resources(ResourcesAction::SelectNext))
        .await;
    assert_eq!(compute_load_phase(&session), &ResourceLoadPhase::Idle);
    let commands = runner.commands.lock().expect("commands mutex poisoned");
    // The two commands are recorded by independently spawned background tasks,
    // so their order in the log is not deterministic — assert over what the log
    // contains, never over positions. What must hold is that exactly these two
    // commands were ever issued: one compute list (the detached fetch, never
    // rescheduled) and one auth probe.
    let compute_lists = commands.iter().filter(|c| is_compute_list(c)).count();
    let auth_probes = commands.iter().filter(|c| is_auth_probe(c)).count();
    assert_eq!(
        (commands.len(), compute_lists, auth_probes),
        (2, 1, 1),
        "expected exactly the detached compute list and Home's auth probe, in either order, \
         got {commands:?}"
    );
}

/// The ordering guarantee: the reset at the drop site is keyed by the load
/// ticket, so a dropped result can only resolve the load it actually started.
/// A newer load for the same service survives the older one's drop, and is
/// itself still resolvable when its own result is dropped.
#[tokio::test]
async fn dropped_result_resolves_only_the_load_it_started() {
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let runner = Arc::new(FakeRunner::new(vec![
        output(include_str!("fixtures/gce_instances.json")),
        output(include_str!("fixtures/gce_instances.json")),
    ]));
    // Hold the *second* compute list command — the one `ApplyFilter` starts.
    let gate = ComputeListGate::holding_call(2, release_rx);
    let mut session = RuntimeSession::new_with_runner_for_test(
        AppState::new(),
        Box::new(DetachedLoadRunner::new(runner.clone()).with_gate(gate.clone())),
    );
    session
        .apply_effect_result(EffectResult::ConfigurationsLoaded {
            configurations: vec![active_dev()],
        })
        .await;

    // Load #1: the auto-load on entering Resources (unfiltered).
    session
        .dispatch_action(Action::Workspace(WorkspaceAction::Resources))
        .await;
    // Load #1 has to be *inside* the runner before load #2 starts: both run on
    // independently spawned tasks, and the gate holds the second compute list
    // it sees. Without this wait a slow load #1 would let load #2 arrive first
    // and be released while the older fetch is held instead — the assertions
    // below would then be about the wrong load.
    gate.await_calls(1).await;
    // Load #2: applying a filter supersedes it with a second compute fetch,
    // still held inside the runner. It bumps `operation_generation`, so #1's
    // result is stale by the time it is processed.
    session
        .dispatch_action(Action::Resources(ResourcesAction::StartFilter))
        .await;
    session
        .dispatch_action(Action::Resources(ResourcesAction::AppendFilterChar('a')))
        .await;
    session
        .dispatch_action(Action::Resources(ResourcesAction::ApplyFilter))
        .await;
    // The overlap this test needs only exists once #2 is actually held inside
    // the runner; without that the assertions below would prove nothing.
    gate.await_engaged().await;

    // Dropping #1 must not touch the phase #2 now owns.
    assert!(process_next_effect_result(&mut session, "superseded load #1").await);
    assert!(
        matches!(
            compute_load_phase(&session),
            ResourceLoadPhase::Loading { .. }
        ),
        "a superseded fetch's drop must not clear the newer load's phase"
    );

    // Detach #2 as well, then let it finish: its own drop resolves its phase.
    session
        .dispatch_action(Action::Workspace(WorkspaceAction::Home))
        .await;
    release_tx.send(()).expect("gated runner still listening");
    assert!(
        process_next_effect_result(
            &mut session,
            "1 of 2: released load #2 / Home's diagnostics probe, either order",
        )
        .await
    );
    assert!(
        process_next_effect_result(
            &mut session,
            "2 of 2: released load #2 / Home's diagnostics probe, either order",
        )
        .await
    );
    assert_eq!(
        compute_load_phase(&session),
        &ResourceLoadPhase::Idle,
        "the newer load must resolve when its own result is dropped"
    );
}
