//! Integration coverage for the "Open in k9s" workflow (issue #29): palette
//! gating, the kubectl-probe -> confirmation -> get-credentials -> interactive
//! launch flow, cancellation, credential failure, and stale-config dismissal.

use std::sync::{Arc, Mutex};

use nuvem::Config;
use nuvem::app::effect::InteractiveCommand;
use nuvem::app::reducer::root::reduce;
use nuvem::app::state::command::CommandDescriptor;
use nuvem::app::{
    Action, AppState, CommandAction, Effect, EffectResult, ResourcesAction, WorkspaceAction,
};
use nuvem::services::gcloud::cli::{CommandError, CommandOutput, CommandRunner, GcloudCommand};
use nuvem::services::runtime::session::RuntimeSession;
use nuvem_domain::gcp::configuration::{GcloudConfiguration, GcloudConfigurationName};
use nuvem_domain::gcp::k9s::{
    CredentialStep, K9sLaunchPlan, K9sLaunchTarget, resolve_launch_target,
};
use nuvem_domain::gcp::resource::{
    EndpointVisibility, GkeCluster, GkeClusterStatus, ResourceServiceKind,
};

// ---------- shared builders ----------

fn config(name: &str, active: bool, project: Option<&str>) -> GcloudConfiguration {
    GcloudConfiguration {
        name: GcloudConfigurationName::new(name),
        is_active: active,
        account: Some("dev@example.com".to_string()),
        project: project.map(Into::into),
        region: Some("europe-west1".to_string()),
        zone: Some("europe-west1-b".to_string()),
    }
}

fn gke_cluster(name: &str, location: &str) -> GkeCluster {
    GkeCluster {
        name: name.to_string(),
        location: location.to_string(),
        status: GkeClusterStatus::Running,
        current_master_version: Some("1.31.5-gke.1023000".to_string()),
        current_node_count: Some(3),
        endpoint_visibility: EndpointVisibility::Public,
    }
}

/// Resources workspace, GKE service active, one cluster loaded and selected.
fn app_with_selected_gke(project: Option<&str>, location: &str) -> AppState {
    let mut app = AppState::new();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ConfigurationsLoaded {
            configurations: vec![config("dev", true, project)],
        }),
    );
    reduce(&mut app, &Action::Workspace(WorkspaceAction::Resources));
    reduce(
        &mut app,
        &Action::Resources(ResourcesAction::SelectService(
            ResourceServiceKind::GkeClusters,
        )),
    );
    let context = app.root.current_context().unwrap().clone();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::GkeClustersLoaded {
            context,
            clusters: vec![gke_cluster("platform-dev", location)],
        }),
    );
    app
}

fn target(project: &str, location: &str) -> K9sLaunchTarget {
    let context = config("dev", true, Some(project)).context();
    resolve_launch_target(&context, &gke_cluster("platform-dev", location))
        .expect("target resolves")
}

// ---------- AC1: palette gating ----------

#[test]
fn palette_offers_k9s_when_gke_cluster_selected_with_project_and_location() {
    let mut app = app_with_selected_gke(Some("dev-project"), "europe-west1");
    assert!(app.root.resources.selected_gke_cluster().is_some());
    reduce(&mut app, &Action::Command(CommandAction::Open));
    let commands = app.root.command.visible_commands(&app.root);
    assert!(
        commands.contains(&CommandDescriptor::OpenSelectedGkeClusterInK9s),
        "expected the Open-in-k9s command to be offered, got {commands:?}"
    );
}

#[test]
fn palette_omits_k9s_when_project_missing() {
    let mut app = app_with_selected_gke(None, "europe-west1");
    reduce(&mut app, &Action::Command(CommandAction::Open));
    let commands = app.root.command.visible_commands(&app.root);
    assert!(!commands.contains(&CommandDescriptor::OpenSelectedGkeClusterInK9s));
}

#[test]
fn palette_omits_k9s_when_location_empty() {
    let mut app = app_with_selected_gke(Some("dev-project"), "");
    reduce(&mut app, &Action::Command(CommandAction::Open));
    let commands = app.root.command.visible_commands(&app.root);
    assert!(!commands.contains(&CommandDescriptor::OpenSelectedGkeClusterInK9s));
}

// ---------- AC2 / AC9: confirmation reuse vs get-credentials ----------

#[test]
fn prepared_with_matching_context_opens_reuse_confirmation() {
    let mut app = app_with_selected_gke(Some("dev-project"), "europe-west1");
    let target = target("dev-project", "europe-west1");
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::K9sLaunchPrepared {
            existing_contexts: Some(vec![target.kube_context.clone()]),
            target,
        }),
    );
    let confirmation = app
        .root
        .resources
        .k9s_launch_confirmation()
        .expect("confirmation open");
    assert_eq!(
        confirmation.credential_step(),
        CredentialStep::ReuseExisting
    );
}

#[test]
fn prepared_without_context_or_probe_opens_get_credentials_confirmation() {
    for existing in [None, Some(vec!["other".to_string()])] {
        let mut app = app_with_selected_gke(Some("dev-project"), "us-central1-a");
        let target = target("dev-project", "us-central1-a");
        reduce(
            &mut app,
            &Action::EffectResult(EffectResult::K9sLaunchPrepared {
                existing_contexts: existing,
                target,
            }),
        );
        assert_eq!(
            app.root
                .resources
                .k9s_launch_confirmation()
                .unwrap()
                .credential_step(),
            CredentialStep::RunGetCredentials
        );
    }
}

// ---------- AC4: cancel before any command ----------

#[test]
fn cancel_clears_confirmation_without_effects() {
    let mut app = app_with_selected_gke(Some("dev-project"), "europe-west1");
    let target = target("dev-project", "europe-west1");
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::K9sLaunchPrepared {
            existing_contexts: None,
            target,
        }),
    );
    let effects = reduce(
        &mut app,
        &Action::Resources(ResourcesAction::CancelK9sLaunch),
    );
    assert!(effects.is_empty());
    assert!(app.root.resources.k9s_launch_confirmation().is_none());
}

// ---------- confirm -> RunK9sCredentials ----------

#[test]
fn confirm_emits_run_credentials_and_clears_modal() {
    let mut app = app_with_selected_gke(Some("dev-project"), "europe-west1");
    let target = target("dev-project", "europe-west1");
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::K9sLaunchPrepared {
            existing_contexts: None,
            target: target.clone(),
        }),
    );
    let effects = reduce(
        &mut app,
        &Action::Resources(ResourcesAction::ConfirmK9sLaunch),
    );
    assert_eq!(
        effects,
        vec![Effect::RunK9sCredentials {
            plan: K9sLaunchPlan {
                target,
                credential_step: CredentialStep::RunGetCredentials,
            }
        }]
    );
    assert!(app.root.resources.k9s_launch_confirmation().is_none());
}

/// The palette's submit gate cannot see a confirmation that was already open
/// when the mode came on, and this modal's confirm is what dispatches
/// `get-credentials` against the cluster. Read-only has to refuse at the
/// confirm too — the sibling guards on the compute, batch, and SSH confirms
/// each have this test; without it the k9s arm could be deleted with the
/// suite still green.
#[test]
fn read_only_turned_on_after_the_k9s_confirmation_opened_refuses_the_confirm() {
    let mut app = app_with_selected_gke(Some("dev-project"), "europe-west1");
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::K9sLaunchPrepared {
            existing_contexts: None,
            target: target("dev-project", "europe-west1"),
        }),
    );
    assert!(
        app.root.resources.k9s_launch_confirmation().is_some(),
        "the confirmation opens while the mode is still off"
    );

    // A config reload turns the mode on with the modal already live.
    app.root.read_only = true;

    let effects = reduce(
        &mut app,
        &Action::Resources(ResourcesAction::ConfirmK9sLaunch),
    );

    assert!(
        effects.is_empty(),
        "confirming an already-open k9s launch must not run get-credentials \
         against the cluster: {effects:?}"
    );
    assert!(
        app.root.resources.k9s_launch_confirmation().is_none(),
        "the refused confirmation must close, not sit there re-refusing"
    );
    assert_eq!(
        app.root.system.status_message.as_deref(),
        Some(
            "Open selected GKE cluster in k9s is unavailable: read-only mode refuses \
             cloud-resource mutations"
        )
    );
    assert_eq!(
        app.root.system.status_phase,
        Some(nuvem::app::state::system::OperationPhase::Healthy),
        "a refusal is the mode doing its job, not an operation that failed"
    );
}

// ---------- credentials-ready -> interactive launch; failed -> status only ----------

#[test]
fn credentials_ready_launches_k9s_interactively() {
    let mut app = AppState::new();
    let effects = reduce(
        &mut app,
        &Action::EffectResult(EffectResult::K9sCredentialsReady {
            kube_context: "gke_dev-project_europe-west1_platform-dev".to_string(),
            cluster_name: "platform-dev".to_string(),
        }),
    );
    assert_eq!(
        effects,
        vec![Effect::LaunchInteractiveProcess {
            command: InteractiveCommand::K9s {
                kube_context: "gke_dev-project_europe-west1_platform-dev".to_string(),
            }
        }]
    );
}

/// The palette stays usable while `get-credentials` runs, and a config reload
/// can turn read-only on inside that window. The credentials result is then a
/// launch into a live cluster that no earlier gate saw: the palette submit and
/// the confirm both happened while the mode was still off. The launch point
/// itself has to refuse.
#[test]
fn read_only_turned_on_while_credentials_ran_refuses_the_launch() {
    let mut app = AppState::new();
    app.root.read_only = true;

    let effects = reduce(
        &mut app,
        &Action::EffectResult(EffectResult::K9sCredentialsReady {
            kube_context: "gke_dev-project_europe-west1_platform-dev".to_string(),
            cluster_name: "platform-dev".to_string(),
        }),
    );

    assert!(
        !effects
            .iter()
            .any(|effect| matches!(effect, Effect::LaunchInteractiveProcess { .. })),
        "an in-flight k9s launch must not spawn a read-write cluster UI once the \
         mode is on: {effects:?}"
    );
    assert_eq!(
        app.root.system.status_message.as_deref(),
        Some(
            "Open selected GKE cluster in k9s is unavailable: read-only mode refuses \
             cloud-resource mutations"
        ),
        "the refusal has to explain itself, not swallow the launch silently"
    );
    assert_eq!(
        app.root.system.status_phase,
        Some(nuvem::app::state::system::OperationPhase::Healthy),
        "a refusal is the mode doing its job, not an operation that failed"
    );
}

#[test]
fn credentials_failure_sets_status_and_launches_nothing() {
    let mut app = AppState::new();
    let effects = reduce(
        &mut app,
        &Action::EffectResult(EffectResult::K9sCredentialsFailed {
            cluster_name: "platform-dev".to_string(),
            error: nuvem::app::AppError::gcloud("permission denied"),
        }),
    );
    assert!(effects.is_empty());
    let status = app.root.system.status_message.clone().unwrap_or_default();
    assert!(status.contains("platform-dev"), "status was: {status}");
    assert!(status.contains("permission denied"), "status was: {status}");
}

// ---------- AC9: stale configuration change dismisses the modal ----------

#[test]
fn changing_active_context_dismisses_open_confirmation() {
    let mut app = app_with_selected_gke(Some("dev-project"), "europe-west1");
    let target = target("dev-project", "europe-west1");
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::K9sLaunchPrepared {
            existing_contexts: None,
            target,
        }),
    );
    assert!(app.root.resources.k9s_launch_confirmation().is_some());
    // A configuration reload that changes the active project changes the context.
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ConfigurationsLoaded {
            configurations: vec![config("dev", true, Some("other-project"))],
        }),
    );
    assert!(
        app.root.resources.k9s_launch_confirmation().is_none(),
        "stale configuration change must dismiss the k9s confirmation"
    );
}

// ---------- session-level: kubectl probe + get-credentials ----------

#[derive(Default)]
struct RecordingRunner {
    calls: Mutex<Vec<Vec<String>>>,
    result: Mutex<Option<Result<CommandOutput, CommandError>>>,
}

impl RecordingRunner {
    fn with(result: Result<CommandOutput, CommandError>) -> Arc<Self> {
        Arc::new(Self {
            calls: Mutex::new(Vec::new()),
            result: Mutex::new(Some(result)),
        })
    }
    fn calls(&self) -> Vec<Vec<String>> {
        self.calls.lock().unwrap().clone()
    }
}

struct SharedRunner(Arc<RecordingRunner>);

impl CommandRunner for SharedRunner {
    fn run(&self, command: GcloudCommand) -> Result<CommandOutput, CommandError> {
        self.0.calls.lock().unwrap().push(command.args.clone());
        self.0
            .result
            .lock()
            .unwrap()
            .take()
            .unwrap_or_else(|| Ok(ok_output("")))
    }
}

fn ok_output(stdout: &str) -> CommandOutput {
    CommandOutput {
        stdout: stdout.to_string(),
        stderr: String::new(),
    }
}

fn session_with(gcloud: Arc<RecordingRunner>, kubectl: Arc<RecordingRunner>) -> RuntimeSession {
    RuntimeSession::new_with_k9s_runners_for_test(
        Config::default(),
        AppState::new(),
        Box::new(SharedRunner(gcloud)),
        Box::new(SharedRunner(kubectl)),
    )
}

#[tokio::test]
async fn prepare_reports_existing_context_from_kubectl() {
    let target = target("dev-project", "europe-west1");
    let gcloud = RecordingRunner::with(Ok(ok_output("")));
    let kubectl =
        RecordingRunner::with(Ok(ok_output(&format!("{}\nother\n", target.kube_context))));
    let mut session = session_with(gcloud, kubectl.clone());

    session
        .dispatch_action(Action::Effect(Effect::PrepareK9sLaunch {
            target: target.clone(),
        }))
        .await;

    assert_eq!(
        kubectl.calls()[0],
        vec!["config", "get-contexts", "-o", "name"]
    );
    assert_eq!(
        session
            .app()
            .root
            .resources
            .k9s_launch_confirmation()
            .unwrap()
            .credential_step(),
        CredentialStep::ReuseExisting
    );
}

#[tokio::test]
async fn prepare_falls_back_to_get_credentials_when_kubectl_errors_or_malformed() {
    // kubectl error -> None -> RunGetCredentials.
    for result in [
        Err(CommandError::Io("no kubectl".to_string())),
        Ok(ok_output("some-unrelated-noise")),
    ] {
        let target = target("dev-project", "europe-west1");
        let session = session_with(
            RecordingRunner::with(Ok(ok_output(""))),
            RecordingRunner::with(result),
        );
        let mut session = session;
        session
            .dispatch_action(Action::Effect(Effect::PrepareK9sLaunch {
                target: target.clone(),
            }))
            .await;
        assert_eq!(
            session
                .app()
                .root
                .resources
                .k9s_launch_confirmation()
                .unwrap()
                .credential_step(),
            CredentialStep::RunGetCredentials
        );
    }
}

#[tokio::test]
async fn run_credentials_success_stashes_interactive_launch() {
    let target = target("dev-project", "us-central1-a");
    let gcloud = RecordingRunner::with(Ok(ok_output("")));
    let mut session = session_with(gcloud.clone(), RecordingRunner::with(Ok(ok_output(""))));

    session
        .dispatch_action(Action::Effect(Effect::RunK9sCredentials {
            plan: K9sLaunchPlan {
                target: target.clone(),
                credential_step: CredentialStep::RunGetCredentials,
            },
        }))
        .await;

    // get-credentials ran with the exact argv, then the launch was stashed.
    assert_eq!(gcloud.calls()[0][4], "get-credentials");
    assert_eq!(
        session.take_pending_interactive_command(),
        Some(InteractiveCommand::K9s {
            kube_context: target.kube_context,
        })
    );
}

#[tokio::test]
async fn run_credentials_reuse_skips_get_credentials() {
    let target = target("dev-project", "us-central1-a");
    let gcloud = RecordingRunner::with(Ok(ok_output("")));
    let mut session = session_with(gcloud.clone(), RecordingRunner::with(Ok(ok_output(""))));

    session
        .dispatch_action(Action::Effect(Effect::RunK9sCredentials {
            plan: K9sLaunchPlan {
                target: target.clone(),
                credential_step: CredentialStep::ReuseExisting,
            },
        }))
        .await;

    assert!(
        gcloud.calls().is_empty(),
        "reuse must not run get-credentials"
    );
    assert_eq!(
        session.take_pending_interactive_command(),
        Some(InteractiveCommand::K9s {
            kube_context: target.kube_context,
        })
    );
}

#[tokio::test]
async fn run_credentials_failure_prevents_launch() {
    let target = target("dev-project", "us-central1-a");
    let gcloud = RecordingRunner::with(Err(CommandError::Failed {
        stdout: String::new(),
        stderr: "permission denied".to_string(),
        exit_code: None,
    }));
    let mut session = session_with(gcloud, RecordingRunner::with(Ok(ok_output(""))));

    session
        .dispatch_action(Action::Effect(Effect::RunK9sCredentials {
            plan: K9sLaunchPlan {
                target,
                credential_step: CredentialStep::RunGetCredentials,
            },
        }))
        .await;

    assert!(
        session.take_pending_interactive_command().is_none(),
        "a failed credential step must not launch k9s"
    );
    let status = session
        .app()
        .root
        .system
        .status_message
        .clone()
        .unwrap_or_default();
    assert!(status.contains("permission denied"), "status was: {status}");
}
