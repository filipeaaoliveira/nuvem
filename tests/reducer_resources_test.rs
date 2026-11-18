use crate::support::remembered_view::remembered;
use nuvem::app::projection::{
    ResourceDetailProjection, ResourceListProjection, StorageDetailProjection,
    StorageRowProjection, project_resources,
};
use nuvem::app::reducer::root::reduce;
use nuvem::app::state::command::CommandDescriptor;
use nuvem::app::state::operations::OperationOutcome;
use nuvem::app::state::plugins::PluginId;
use nuvem::app::state::resources::{
    BatchAction, BatchRowRef, BatchableService, CloudSqlInstanceAction, Compute,
    ComputeInstanceAction, FilterScope, PluginPrompt, ResourceLoadPhase, ResourcesState,
    StorageDetailTab, StorageView, is_safe_remote_prefix,
};
use nuvem::app::{
    Action, AppError, AppState, Effect, EffectResult, ResourcesAction, TunnelLaunchPlan, Workspace,
};
use nuvem::config::plugins::{PluginInput, PluginInputKind};
use nuvem::services::resources::storage::{
    parse_bucket_detail_json, parse_object_detail_json, parse_object_page_json,
};
use nuvem::services::runtime::session::RuntimeSession;
use nuvem_domain::gcp::configuration::{
    GcloudConfiguration, GcloudConfigurationName, GcloudContext,
};
use nuvem_domain::gcp::diagnostics::GcloudErrorKind;
use nuvem_domain::gcp::filter::FilterMode;
use nuvem_domain::gcp::k9s::{CredentialStep, resolve_launch_target};
use nuvem_domain::gcp::resource::{
    CloudSqlAvailability, CloudSqlBackupConfig, CloudSqlInstance, CloudSqlInstanceDetail,
    CloudSqlStatus, CloudSqlUser, ComputeDisk, ComputeInstance, ComputeInstanceDetail,
    ComputeKeyValue, ComputeNetworkInterface, EndpointVisibility, GkeCluster, GkeClusterDetail,
    GkeClusterStatus, ResourceServiceKind, ResourceStatus, Secret, SecretDetail, SecretVersion,
    SecretVersionState, StorageBucket, StorageObject, StorageObjectPage, StoragePrefix,
};
use nuvem_domain::gcp::tunnel::TunnelKind;
use ratatui::{Terminal, backend::TestBackend};

fn config(name: &str, active: bool, project: &str) -> GcloudConfiguration {
    GcloudConfiguration {
        name: GcloudConfigurationName::new(name),
        is_active: active,
        account: None,
        project: Some(project.to_string()),
        region: None,
        zone: None,
    }
}

/// Operations-log id for results whose test asserts nothing about the
/// operations log. It sits above every id these tests seed, so such a result
/// can never settle a real entry by accident and quietly change what a later
/// assertion sees.
const UNTRACKED_OPERATION_ID: u64 = 9_000;

fn instance(name: &str) -> ComputeInstance {
    ComputeInstance {
        id: name.to_string(),
        name: name.to_string(),
        zone: "europe-west1-b".to_string(),
        status: ResourceStatus::Running,
        machine_type: "e2-medium".to_string(),
        internal_ip: None,
        external_ip: None,
    }
}

/// Pairs targets with stand-in operations-log ids for tests that drive workflow
/// state directly, with no operations log behind it. The ids start well above
/// zero so they can never coincide with a target's index: a positional bug
/// would produce `0, 1, 2…`, and these helpers must be inert by construction
/// rather than by coincidence.
fn with_stand_in_ids(targets: Vec<BatchRowRef>) -> Vec<(BatchRowRef, u64)> {
    targets
        .into_iter()
        .enumerate()
        .map(|(index, target)| (target, 900 + index as u64))
        .collect()
}

/// Pairs each target with the id of the operations-log entry the reducer seeded
/// for it, looked up by name rather than assumed from allocation order.
fn pair_with_seeded_ids(app: &AppState, targets: Vec<BatchRowRef>) -> Vec<(BatchRowRef, u64)> {
    targets
        .into_iter()
        .map(|target| {
            let id = app
                .root
                .system
                .operations
                .records()
                .iter()
                .find(|record| record.target == target.name)
                .unwrap_or_else(|| panic!("no operations-log entry for {}", target.name))
                .id;
            (target, id)
        })
        .collect()
}

/// The batch-target projection of [`instance`] for a `Stop` batch.
fn batch_target(name: &str, context: &GcloudContext) -> BatchRowRef {
    Compute::row_ref(&instance(name), context, ComputeInstanceAction::Stop)
}

fn instance_in_zone(name: &str, zone: &str) -> ComputeInstance {
    ComputeInstance {
        zone: zone.to_string(),
        ..instance(name)
    }
}

fn instance_with_fields(
    name: &str,
    zone: &str,
    status: ResourceStatus,
    machine_type: &str,
    internal_ip: Option<&str>,
    external_ip: Option<&str>,
) -> ComputeInstance {
    ComputeInstance {
        id: name.to_string(),
        name: name.to_string(),
        zone: zone.to_string(),
        status,
        machine_type: machine_type.to_string(),
        internal_ip: internal_ip.map(str::to_string),
        external_ip: external_ip.map(str::to_string),
    }
}

fn load_resource_rows(app: &mut AppState, instances: Vec<ComputeInstance>) {
    reduce(
        app,
        &Action::EffectResult(EffectResult::ConfigurationsLoaded {
            configurations: vec![config("dev", true, "dev-project")],
        }),
    );
    let context = app.root.current_context().unwrap().clone();
    reduce(
        app,
        &Action::EffectResult(EffectResult::ComputeInstancesLoaded { context, instances }),
    );
}

/// Types `input` into the active service's filter input and applies it.
fn apply_filter_input(app: &mut AppState, input: &str) {
    reduce(app, &Action::Resources(ResourcesAction::StartFilter));
    for ch in input.chars() {
        reduce(
            app,
            &Action::Resources(ResourcesAction::AppendFilterChar(ch)),
        );
    }
    reduce(app, &Action::Resources(ResourcesAction::ApplyFilter));
}

fn visible_instance_names(app: &AppState) -> Vec<&str> {
    app.root
        .resources
        .visible_instances()
        .into_iter()
        .map(|instance| instance.name.as_str())
        .collect()
}

fn detail(name: &str) -> ComputeInstanceDetail {
    ComputeInstanceDetail {
        name: name.to_string(),
        zone: "europe-west1-b".to_string(),
        status: ResourceStatus::Running,
        machine_type: "e2-medium".to_string(),
        creation_timestamp: Some("2026-07-03T10:11:12.000-07:00".to_string()),
        network_interfaces: vec![ComputeNetworkInterface {
            name: Some("nic0".to_string()),
            network: Some("default".to_string()),
            subnetwork: Some("default".to_string()),
            subnetwork_region: Some("europe-west1".to_string()),
            internal_ip: Some("10.0.0.4".to_string()),
            external_ip: Some("34.10.10.10".to_string()),
        }],
        tags: Vec::new(),
        service_accounts: Vec::new(),
        disks: vec![ComputeDisk {
            name: Some(name.to_string()),
            source: Some(name.to_string()),
            boot: true,
            auto_delete: true,
            disk_type: Some("PERSISTENT".to_string()),
            size_gb: Some("20".to_string()),
        }],
        labels: vec![ComputeKeyValue {
            key: "env".to_string(),
            value: "dev".to_string(),
        }],
        metadata: vec![ComputeKeyValue {
            key: "owner".to_string(),
            value: "platform".to_string(),
        }],
        raw_json: "{\n  \"name\": \"api-dev-1\"\n}".to_string(),
    }
}

fn detail_in_zone(name: &str, zone: &str) -> ComputeInstanceDetail {
    ComputeInstanceDetail {
        zone: zone.to_string(),
        raw_json: format!("{{\n  \"name\": \"{name}\",\n  \"zone\": \"{zone}\"\n}}"),
        ..detail(name)
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

fn gke_cluster_with_fields(
    name: &str,
    location: &str,
    status: GkeClusterStatus,
    version: Option<&str>,
    nodes: Option<u32>,
) -> GkeCluster {
    GkeCluster {
        name: name.to_string(),
        location: location.to_string(),
        status,
        current_master_version: version.map(str::to_string),
        current_node_count: nodes,
        endpoint_visibility: EndpointVisibility::Public,
    }
}

fn gke_detail(name: &str, location: &str) -> GkeClusterDetail {
    GkeClusterDetail {
        name: name.to_string(),
        location: location.to_string(),
        status: GkeClusterStatus::Running,
        current_master_version: Some("1.31.5-gke.1023000".to_string()),
        current_node_version: Some("1.31.5-gke.1023000".to_string()),
        endpoint: Some("34.118.10.20".to_string()),
        private_cluster: false,
        network: Some("shared-vpc".to_string()),
        subnetwork: Some("gke-subnet".to_string()),
        node_pools: Vec::new(),
        addons: Vec::new(),
        labels: Vec::new(),
        raw_json: format!("{{\n  \"name\": \"{name}\"\n}}"),
    }
}

fn app_with_instances() -> AppState {
    let mut app = AppState::new();
    load_resource_rows(
        &mut app,
        vec![instance("api-dev-1"), instance("worker-dev-1")],
    );
    app
}

fn cloud_sql_instance(name: &str) -> CloudSqlInstance {
    CloudSqlInstance {
        name: name.to_string(),
        database_version: "POSTGRES_15".to_string(),
        engine: "PostgreSQL".to_string(),
        region: "europe-west1".to_string(),
        tier: "db-custom-2-7680".to_string(),
        availability: CloudSqlAvailability::Zonal,
        status: CloudSqlStatus::Runnable,
        connectivity: EndpointVisibility::Public,
    }
}

fn cloud_sql_detail(name: &str) -> CloudSqlInstanceDetail {
    CloudSqlInstanceDetail {
        name: name.to_string(),
        database_version: "POSTGRES_15".to_string(),
        engine: "PostgreSQL".to_string(),
        region: "europe-west1".to_string(),
        tier: "db-custom-2-7680".to_string(),
        availability: CloudSqlAvailability::Zonal,
        status: CloudSqlStatus::Runnable,
        activation_policy: Some("ALWAYS".to_string()),
        creation_timestamp: Some("2026-07-03T10:11:12.000Z".to_string()),
        connection_name: Some("dev-project:europe-west1:db".to_string()),
        public_ip: Some("34.10.10.10".to_string()),
        private_ip: None,
        require_ssl: true,
        private_network: None,
        maintenance_window: None,
        maintenance_version: None,
        backup: CloudSqlBackupConfig {
            enabled: true,
            start_time: Some("03:00".to_string()),
            location: None,
            point_in_time_recovery: false,
            transaction_log_retention_days: Some(7),
        },
        replicas: Vec::new(),
        master_instance: None,
        users: Vec::new(),
        labels: Vec::new(),
        raw_json: format!("{{\n  \"name\": \"{name}\"\n}}"),
    }
}

/// Selects Cloud SQL and loads a single instance for the active context.
fn app_with_cloud_sql(instances: Vec<CloudSqlInstance>) -> AppState {
    let mut app = AppState::new();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ConfigurationsLoaded {
            configurations: vec![config("dev", true, "dev-project")],
        }),
    );
    reduce(
        &mut app,
        &Action::Resources(ResourcesAction::SelectService(
            ResourceServiceKind::CloudSql,
        )),
    );
    let context = app.root.current_context().unwrap().clone();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::CloudSqlInstancesLoaded { context, instances }),
    );
    app
}

#[test]
fn configuration_switch_marks_resource_rows_stale() {
    let mut app = AppState::new();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ConfigurationsLoaded {
            configurations: vec![
                config("dev", true, "dev-project"),
                config("prod", false, "prod-project"),
            ],
        }),
    );
    let context = app.root.current_context().unwrap().clone();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ComputeInstancesLoaded {
            context,
            instances: vec![instance("api-dev-1")],
        }),
    );
    assert!(!app.root.resources.is_stale());

    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ConfigurationActivated {
            configuration: config("prod", true, "prod-project"),
        }),
    );

    assert!(app.root.resources.is_stale());
    assert!(app.root.resources.instances().is_empty());
    assert_eq!(
        app.root.current_context().unwrap().configuration.as_str(),
        "prod"
    );
}

#[test]
fn configuration_reload_changing_context_marks_resource_rows_stale() {
    let mut app = AppState::new();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ConfigurationsLoaded {
            configurations: vec![
                config("dev", true, "dev-project"),
                config("prod", false, "prod-project"),
            ],
        }),
    );
    let context = app.root.current_context().unwrap().clone();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ComputeInstancesLoaded {
            context,
            instances: vec![instance("api-dev-1")],
        }),
    );

    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ConfigurationsLoaded {
            configurations: vec![
                config("dev", false, "dev-project"),
                config("prod", true, "prod-project"),
            ],
        }),
    );

    assert!(app.root.resources.is_stale());
    assert!(app.root.resources.instances().is_empty());
    assert_eq!(
        app.root.current_context().unwrap().configuration.as_str(),
        "prod"
    );
}

#[test]
fn compute_load_failure_for_active_context_marks_loaded_rows_stale_and_sets_status() {
    let mut app = AppState::new();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ConfigurationsLoaded {
            configurations: vec![
                config("dev", true, "dev-project"),
                config("prod", false, "prod-project"),
            ],
        }),
    );
    let context = app.root.current_context().unwrap().clone();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ComputeInstancesLoaded {
            context: context.clone(),
            instances: vec![instance("api-dev-1")],
        }),
    );

    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ComputeInstancesLoadFailed {
            context,
            error: nuvem::app::AppError::gcloud("compute API disabled"),
        }),
    );

    assert!(app.root.resources.is_stale());
    assert!(app.root.resources.instances().is_empty());
    assert_eq!(
        app.root.system.status_message.as_deref(),
        Some("unknown error — Retry, or copy the details to investigate")
    );
    assert_eq!(
        app.root.system.status_operation_label.as_deref(),
        Some("GCE instances")
    );
}

#[test]
fn compute_load_failure_stores_sanitized_headline_not_raw_stderr() {
    let mut app = AppState::new();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ConfigurationsLoaded {
            configurations: vec![config("dev", true, "dev-project")],
        }),
    );
    let context = app.root.current_context().unwrap().clone();

    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ComputeInstancesLoadFailed {
            context,
            error: nuvem::app::AppError::Gcloud(nuvem::app::GcloudFailure {
                kind_hint: None,
                detail: "PERMISSION_DENIED forbidden (403) Authorization: Bearer ya29.SECRET"
                    .into(),
                exit_code: Some(1),
                command_summary: None,
                missing_binary: None,
            }),
        }),
    );

    let problem = app.root.resources.compute_problem().unwrap();
    assert_eq!(problem.kind, GcloudErrorKind::PermissionDenied);
    assert!(!problem.message.contains("ya29.SECRET"));

    let status = app.root.system.status_message.clone().unwrap_or_default();
    assert!(status.contains("permission denied"), "status was {status}");
    assert!(!status.contains("ya29.SECRET"), "status was {status}");
    assert!(!status.contains("Bearer"), "status was {status}");

    // The status is the sanitized headline only — no raw Debug leak, no
    // unbounded stderr echoed back to the user.
    assert_eq!(
        status,
        "permission denied — Check IAM roles for this account and project"
    );
}

#[test]
fn compute_load_failure_preserves_loaded_gke_rows_for_same_context() {
    let mut app = app_with_instances();
    let context = app.root.current_context().unwrap().clone();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::GkeClustersLoaded {
            context: context.clone(),
            clusters: vec![gke_cluster("platform-dev", "europe-west1")],
        }),
    );

    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ComputeInstancesLoadFailed {
            context,
            error: AppError::gcloud("compute API disabled"),
        }),
    );

    assert!(app.root.resources.is_stale());
    assert!(app.root.resources.instances().is_empty());
    assert_eq!(app.root.resources.gke_clusters().len(), 1);
    assert_eq!(app.root.resources.gke_clusters()[0].name, "platform-dev");
}

/// A `ComputeInstancesLoadFailed` result must set the active collection's
/// `ResourceLoadPhase` to `Failed`, carrying the same sanitized diagnostic
/// headline already surfaced on the status message (the load-phase
/// channel; must not duplicate/replace `last_diagnostic`/status wiring).
#[test]
fn compute_load_failure_sets_failed_load_phase_with_diagnostic_headline() {
    let mut app = AppState::new();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ConfigurationsLoaded {
            configurations: vec![config("dev", true, "dev-project")],
        }),
    );
    let context = app.root.current_context().unwrap().clone();

    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ComputeInstancesLoadFailed {
            context: context.clone(),
            error: nuvem::app::AppError::gcloud("compute API disabled"),
        }),
    );

    assert_eq!(
        app.root.resources.active_load_phase(),
        &ResourceLoadPhase::Failed {
            message: "unknown error — Retry, or copy the details to investigate".to_string(),
            context,
        }
    );
}

/// After a failed compute load-phase, a subsequent successful
/// `ComputeInstancesLoaded` result must reset the phase back to `Idle`.
#[test]
fn compute_successful_load_after_failure_resets_load_phase_to_idle() {
    let mut app = AppState::new();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ConfigurationsLoaded {
            configurations: vec![config("dev", true, "dev-project")],
        }),
    );
    let context = app.root.current_context().unwrap().clone();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ComputeInstancesLoadFailed {
            context: context.clone(),
            error: nuvem::app::AppError::gcloud("compute API disabled"),
        }),
    );
    assert_ne!(
        app.root.resources.active_load_phase(),
        &ResourceLoadPhase::Idle
    );

    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ComputeInstancesLoaded {
            context,
            instances: vec![instance("api-dev-1")],
        }),
    );

    assert_eq!(
        app.root.resources.active_load_phase(),
        &ResourceLoadPhase::Idle
    );
}

#[test]
fn selecting_resource_rows_clamps_at_list_boundaries() {
    let mut app = AppState::new();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ConfigurationsLoaded {
            configurations: vec![config("dev", true, "dev-project")],
        }),
    );
    let context = app.root.current_context().unwrap().clone();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ComputeInstancesLoaded {
            context,
            instances: vec![instance("api-dev-1"), instance("worker-dev-1")],
        }),
    );

    reduce(&mut app, &Action::Resources(ResourcesAction::SelectNext));
    reduce(&mut app, &Action::Resources(ResourcesAction::SelectNext));

    assert_eq!(app.root.resources.selected_index(), Some(1));

    reduce(
        &mut app,
        &Action::Resources(ResourcesAction::SelectPrevious),
    );
    reduce(
        &mut app,
        &Action::Resources(ResourcesAction::SelectPrevious),
    );

    assert_eq!(app.root.resources.selected_index(), Some(0));
}

#[test]
fn opening_selected_resource_enters_detail_and_emits_load_effect() {
    let mut app = AppState::new();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ConfigurationsLoaded {
            configurations: vec![config("dev", true, "dev-project")],
        }),
    );
    let context = app.root.current_context().unwrap().clone();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ComputeInstancesLoaded {
            context: context.clone(),
            instances: vec![instance("api-dev-1")],
        }),
    );

    let effects = reduce(&mut app, &Action::Resources(ResourcesAction::OpenSelected));

    assert!(app.root.resources.is_showing_detail());
    assert_eq!(
        effects,
        vec![Effect::LoadComputeInstanceDetail {
            context,
            instance: instance("api-dev-1"),
        }]
    );
}

#[test]
fn open_selected_while_showing_compute_detail_is_a_noop() {
    let mut app = AppState::new();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ConfigurationsLoaded {
            configurations: vec![config("dev", true, "dev-project")],
        }),
    );
    let context = app.root.current_context().unwrap().clone();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ComputeInstancesLoaded {
            context,
            instances: vec![instance("api-dev-1")],
        }),
    );
    reduce(&mut app, &Action::Resources(ResourcesAction::OpenSelected));
    assert!(app.root.resources.is_showing_detail());
    reduce(&mut app, &Action::Resources(ResourcesAction::SelectNextTab));
    let tab_before = app.root.resources.active_detail_tab_label().to_string();

    let effects = reduce(&mut app, &Action::Resources(ResourcesAction::OpenSelected));

    assert!(effects.is_empty());
    assert!(app.root.resources.is_showing_detail());
    assert_eq!(
        app.root.resources.active_detail_tab_label(),
        tab_before.as_str()
    );
}

#[test]
fn matching_active_context_detail_load_updates_detail_state() {
    let mut app = AppState::new();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ConfigurationsLoaded {
            configurations: vec![config("dev", true, "dev-project")],
        }),
    );
    let context = app.root.current_context().unwrap().clone();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ComputeInstancesLoaded {
            context: context.clone(),
            instances: vec![instance("api-dev-1")],
        }),
    );

    reduce(&mut app, &Action::Resources(ResourcesAction::OpenSelected));
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ComputeInstanceDetailLoaded {
            context,
            detail: detail("api-dev-1"),
        }),
    );

    assert_eq!(app.root.resources.detail(), Some(&detail("api-dev-1")));
    assert_eq!(app.root.resources.detail_target_name(), Some("api-dev-1"));
    assert_eq!(app.root.resources.detail_error(), None);
}

#[test]
fn stale_same_context_detail_result_for_previous_target_is_ignored() {
    let mut app = AppState::new();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ConfigurationsLoaded {
            configurations: vec![config("dev", true, "dev-project")],
        }),
    );
    let context = app.root.current_context().unwrap().clone();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ComputeInstancesLoaded {
            context: context.clone(),
            instances: vec![instance("api-dev-1"), instance("worker-dev-1")],
        }),
    );

    reduce(&mut app, &Action::Resources(ResourcesAction::OpenSelected));
    reduce(&mut app, &Action::Resources(ResourcesAction::CloseDetail));
    reduce(&mut app, &Action::Resources(ResourcesAction::SelectNext));
    reduce(&mut app, &Action::Resources(ResourcesAction::OpenSelected));

    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ComputeInstanceDetailLoaded {
            context,
            detail: detail("api-dev-1"),
        }),
    );

    assert_eq!(app.root.resources.detail(), None);
    assert_eq!(
        app.root.resources.detail_target_name(),
        Some("worker-dev-1")
    );
    assert_eq!(app.root.resources.detail_error(), None);
}

#[test]
fn stale_same_context_detail_result_for_same_name_different_zone_is_ignored() {
    let mut app = AppState::new();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ConfigurationsLoaded {
            configurations: vec![config("dev", true, "dev-project")],
        }),
    );
    let context = app.root.current_context().unwrap().clone();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ComputeInstancesLoaded {
            context: context.clone(),
            instances: vec![
                instance_in_zone("api-dev-1", "europe-west1-b"),
                instance_in_zone("api-dev-1", "europe-west1-c"),
            ],
        }),
    );

    reduce(&mut app, &Action::Resources(ResourcesAction::SelectNext));
    reduce(&mut app, &Action::Resources(ResourcesAction::OpenSelected));
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ComputeInstanceDetailLoaded {
            context,
            detail: detail_in_zone("api-dev-1", "europe-west1-b"),
        }),
    );

    assert_eq!(app.root.resources.detail(), None);
    assert_eq!(
        app.root
            .resources
            .detail_target()
            .map(|target| target.zone.as_str()),
        Some("europe-west1-c")
    );
    assert_eq!(app.root.resources.detail_error(), None);
}

#[test]
fn detail_tab_navigation_wraps_and_close_returns_to_list() {
    let mut app = AppState::new();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ConfigurationsLoaded {
            configurations: vec![config("dev", true, "dev-project")],
        }),
    );
    let context = app.root.current_context().unwrap().clone();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ComputeInstancesLoaded {
            context,
            instances: vec![instance("api-dev-1")],
        }),
    );
    reduce(&mut app, &Action::Resources(ResourcesAction::OpenSelected));

    reduce(
        &mut app,
        &Action::Resources(ResourcesAction::SelectPreviousTab),
    );
    assert_eq!(app.root.resources.active_detail_tab_label(), "JSON");
    reduce(&mut app, &Action::Resources(ResourcesAction::SelectNextTab));
    assert_eq!(app.root.resources.active_detail_tab_label(), "Summary");
    reduce(&mut app, &Action::Resources(ResourcesAction::CloseDetail));

    assert!(!app.root.resources.is_showing_detail());
}

#[test]
fn stale_context_detail_results_are_ignored() {
    let mut app = AppState::new();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ConfigurationsLoaded {
            configurations: vec![
                config("dev", true, "dev-project"),
                config("prod", false, "prod-project"),
            ],
        }),
    );
    let dev_context = app.root.current_context().unwrap().clone();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ConfigurationActivated {
            configuration: config("prod", true, "prod-project"),
        }),
    );

    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ComputeInstanceDetailLoaded {
            context: dev_context,
            detail: detail("api-dev-1"),
        }),
    );

    assert!(app.root.resources.detail().is_none());
}

#[test]
fn detail_failure_for_active_context_records_error() {
    let mut app = AppState::new();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ConfigurationsLoaded {
            configurations: vec![config("dev", true, "dev-project")],
        }),
    );
    let context = app.root.current_context().unwrap().clone();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ComputeInstancesLoaded {
            context: context.clone(),
            instances: vec![instance("api-dev-1")],
        }),
    );
    reduce(&mut app, &Action::Resources(ResourcesAction::OpenSelected));
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ComputeInstanceDetailLoadFailed {
            context,
            instance_name: "api-dev-1".to_string(),
            instance_zone: "europe-west1-b".to_string(),
            error: nuvem::app::AppError::gcloud("describe failed"),
        }),
    );

    assert_eq!(
        app.root.resources.detail_error(),
        Some("unknown error — Retry, or copy the details to investigate (describe failed)")
    );
    assert_eq!(
        app.root.system.status_message.as_deref(),
        Some("unknown error — Retry, or copy the details to investigate")
    );
}

#[test]
fn stale_same_context_detail_failure_for_same_name_different_zone_is_ignored() {
    let mut app = AppState::new();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ConfigurationsLoaded {
            configurations: vec![config("dev", true, "dev-project")],
        }),
    );
    let context = app.root.current_context().unwrap().clone();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ComputeInstancesLoaded {
            context: context.clone(),
            instances: vec![
                instance_in_zone("api-dev-1", "europe-west1-b"),
                instance_in_zone("api-dev-1", "europe-west1-c"),
            ],
        }),
    );

    reduce(&mut app, &Action::Resources(ResourcesAction::SelectNext));
    reduce(&mut app, &Action::Resources(ResourcesAction::OpenSelected));
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ComputeInstanceDetailLoadFailed {
            context,
            instance_name: "api-dev-1".to_string(),
            instance_zone: "europe-west1-b".to_string(),
            error: nuvem::app::AppError::gcloud("describe failed"),
        }),
    );

    assert_eq!(app.root.resources.detail(), None);
    assert_eq!(app.root.resources.detail_error(), None);
    assert_eq!(app.root.system.status_message, None);
    assert_eq!(
        app.root
            .resources
            .detail_target()
            .map(|target| target.zone.as_str()),
        Some("europe-west1-c")
    );
}

#[test]
fn resource_filter_draft_apply_and_cancel_are_separate() {
    let mut app = AppState::new();
    load_resource_rows(
        &mut app,
        vec![instance("api-dev-1"), instance("worker-dev-1")],
    );

    reduce(&mut app, &Action::Resources(ResourcesAction::StartFilter));
    reduce(
        &mut app,
        &Action::Resources(ResourcesAction::AppendFilterChar('a')),
    );
    reduce(
        &mut app,
        &Action::Resources(ResourcesAction::AppendFilterChar('p')),
    );
    reduce(&mut app, &Action::Resources(ResourcesAction::ApplyFilter));

    assert_eq!(app.root.resources.filter_applied(), "ap");
    assert_eq!(app.root.resources.filter_draft(), "ap");
    assert!(!app.root.resources.is_filter_input_active());
    assert_eq!(
        app.root
            .resources
            .visible_instances()
            .into_iter()
            .map(|instance| instance.name.as_str())
            .collect::<Vec<_>>(),
        vec!["api-dev-1"]
    );

    reduce(&mut app, &Action::Resources(ResourcesAction::StartFilter));
    reduce(
        &mut app,
        &Action::Resources(ResourcesAction::AppendFilterChar('x')),
    );
    reduce(&mut app, &Action::Resources(ResourcesAction::CancelFilter));

    assert_eq!(app.root.resources.filter_applied(), "ap");
    assert_eq!(app.root.resources.filter_draft(), "ap");
    assert!(!app.root.resources.is_filter_input_active());
}

#[test]
fn apply_filter_while_compute_active_emits_refresh_carrying_the_filter() {
    let mut app = app_with_instances();
    let context = app.root.current_context().unwrap().clone();

    reduce(&mut app, &Action::Resources(ResourcesAction::StartFilter));
    reduce(
        &mut app,
        &Action::Resources(ResourcesAction::AppendFilterChar('a')),
    );
    let effects = reduce(&mut app, &Action::Resources(ResourcesAction::ApplyFilter));

    assert_eq!(
        effects,
        vec![Effect::RefreshComputeInstances {
            context,
            filter: Some("a".to_string()),
        }]
    );
}

// --- Which reloads carry the applied filter -------------------------------
//
// A reload is not a superset that later gets narrowed client-side: under label
// mode `CompiledFilter::Label` keeps every row, so whatever the reload
// downloads IS the visible set. An unfiltered reload would therefore silently
// drop a label filter while the tools line still asserts it is in force. The
// rule these tests pin: every reload carries the *refreshed* service's own
// applied filter — never another service's term (that would narrow the wrong
// list), and never `None` while that service has a filter applied (that would
// silently disarm it, with no reload left to restore it once the user
// navigates back).

#[test]
fn switching_service_away_and_back_reloads_compute_with_its_applied_filter() {
    let mut app = app_with_instances();
    apply_filter_input(&mut app, "-l env=prod");
    let context = app.root.current_context().unwrap().clone();

    // Switching away refreshes GKE, whose own applied filter is empty: compute's
    // term must not leak across services.
    let away = reduce(
        &mut app,
        &Action::Resources(ResourcesAction::SelectService(
            ResourceServiceKind::GkeClusters,
        )),
    );
    // Each switch also remembers the service it landed on against "dev";
    // the filter each reload carries is what this test pins and is
    // unchanged.
    assert_eq!(
        away,
        vec![
            Effect::RefreshGkeClusters {
                context: context.clone(),
                filter: None,
            },
            Effect::SaveRememberedViews {
                views: remembered("dev", ResourceServiceKind::GkeClusters),
            }
        ]
    );

    let back = reduce(
        &mut app,
        &Action::Resources(ResourcesAction::SelectService(
            ResourceServiceKind::ComputeInstances,
        )),
    );
    assert_eq!(
        back,
        vec![
            Effect::RefreshComputeInstances {
                context,
                filter: Some("-l env=prod".to_string()),
            },
            Effect::SaveRememberedViews {
                views: remembered("dev", ResourceServiceKind::ComputeInstances),
            }
        ]
    );
}

#[test]
fn compute_follow_up_refresh_carries_the_filter_when_compute_is_active() {
    let mut app = app_with_instances();
    apply_filter_input(&mut app, "-l env=prod");
    let context = app.root.current_context().unwrap().clone();

    let effects = reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ComputeInstanceActionSucceeded {
            context: context.clone(),
            instance_name: "api-dev-1".to_string(),
            instance_zone: "europe-west1-b".to_string(),
            action: ComputeInstanceAction::Stop,
            operation_id: UNTRACKED_OPERATION_ID,
        }),
    );

    assert_eq!(
        effects,
        vec![Effect::RefreshComputeInstances {
            context,
            filter: Some("-l env=prod".to_string()),
        }]
    );
}

#[test]
fn cloud_sql_follow_up_refresh_carries_cloud_sqls_own_filter_when_compute_is_active() {
    let mut app = app_with_instances();
    // Cloud SQL gets its own filter, then the user switches to Compute and
    // applies a different one there.
    reduce(
        &mut app,
        &Action::Resources(ResourcesAction::SelectService(
            ResourceServiceKind::CloudSql,
        )),
    );
    apply_filter_input(&mut app, "-l tier=gold");
    reduce(
        &mut app,
        &Action::Resources(ResourcesAction::SelectService(
            ResourceServiceKind::ComputeInstances,
        )),
    );
    apply_filter_input(&mut app, "-l env=prod");
    let context = app.root.current_context().unwrap().clone();

    let effects = reduce(
        &mut app,
        &Action::EffectResult(EffectResult::CloudSqlInstanceActionSucceeded {
            context: context.clone(),
            instance_name: "orders-dev".to_string(),
            action: CloudSqlInstanceAction::Restart,
            operation_id: UNTRACKED_OPERATION_ID,
        }),
    );

    // Cloud SQL's own filter — not Compute's, and not `None`: the follow-up
    // replaces Cloud SQL's stored rows, so dropping its filter would widen that
    // list to the whole project behind the user's back.
    assert_eq!(
        effects,
        vec![Effect::RefreshCloudSqlInstances {
            context,
            filter: Some("-l tier=gold".to_string()),
        }]
    );
}

#[test]
fn compute_follow_up_refresh_carries_computes_own_filter_after_switching_to_gke() {
    let mut app = app_with_instances();
    apply_filter_input(&mut app, "-l env=prod");
    let context = app.root.current_context().unwrap().clone();

    // The operator switches to GKE before the Stop result lands, and GKE has a
    // filter of its own.
    let away = reduce(
        &mut app,
        &Action::Resources(ResourcesAction::SelectService(
            ResourceServiceKind::GkeClusters,
        )),
    );
    // Plus the remembered-view save for the switch itself.
    assert_eq!(
        away,
        vec![
            Effect::RefreshGkeClusters {
                context: context.clone(),
                filter: None,
            },
            Effect::SaveRememberedViews {
                views: remembered("dev", ResourceServiceKind::GkeClusters),
            }
        ]
    );
    apply_filter_input(&mut app, "-l team=core");

    let effects = reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ComputeInstanceActionSucceeded {
            context: context.clone(),
            instance_name: "api-dev-1".to_string(),
            instance_zone: "europe-west1-b".to_string(),
            action: ComputeInstanceAction::Stop,
            operation_id: UNTRACKED_OPERATION_ID,
        }),
    );

    // Compute's own filter. `None` here would overwrite Compute's stored rows
    // with the whole project, and nothing reloads Compute when the user
    // navigates back (`needs_refresh` is false), so the widened list would sit
    // under a tools line still reading `Filter: -l env=prod  [label]`.
    assert_eq!(
        effects,
        vec![Effect::RefreshComputeInstances {
            context: context.clone(),
            filter: Some("-l env=prod".to_string()),
        }]
    );

    // And a GKE reload in the same state carries GKE's own filter, never
    // Compute's.
    reduce(
        &mut app,
        &Action::Resources(ResourcesAction::SelectService(
            ResourceServiceKind::ComputeInstances,
        )),
    );
    let gke = reduce(
        &mut app,
        &Action::Resources(ResourcesAction::SelectService(
            ResourceServiceKind::GkeClusters,
        )),
    );
    // Plus the remembered-view save for this switch back to GKE.
    assert_eq!(
        gke,
        vec![
            Effect::RefreshGkeClusters {
                context,
                filter: Some("-l team=core".to_string()),
            },
            Effect::SaveRememberedViews {
                views: remembered("dev", ResourceServiceKind::GkeClusters),
            }
        ]
    );
}

#[test]
fn cloud_sql_follow_up_refresh_carries_the_filter_when_cloud_sql_is_active() {
    let mut app = app_with_instances();
    reduce(
        &mut app,
        &Action::Resources(ResourcesAction::SelectService(
            ResourceServiceKind::CloudSql,
        )),
    );
    apply_filter_input(&mut app, "-l env=prod");
    let context = app.root.current_context().unwrap().clone();

    let effects = reduce(
        &mut app,
        &Action::EffectResult(EffectResult::CloudSqlInstanceActionSucceeded {
            context: context.clone(),
            instance_name: "orders-dev".to_string(),
            action: CloudSqlInstanceAction::Restart,
            operation_id: UNTRACKED_OPERATION_ID,
        }),
    );

    assert_eq!(
        effects,
        vec![Effect::RefreshCloudSqlInstances {
            context,
            filter: Some("-l env=prod".to_string()),
        }]
    );
}

#[test]
fn resource_filter_editing_existing_applied_filter_appends_to_draft() {
    let mut app = AppState::new();
    load_resource_rows(&mut app, vec![instance("api"), instance("worker")]);

    reduce(&mut app, &Action::Resources(ResourcesAction::StartFilter));
    for ch in "api".chars() {
        reduce(
            &mut app,
            &Action::Resources(ResourcesAction::AppendFilterChar(ch)),
        );
    }
    reduce(&mut app, &Action::Resources(ResourcesAction::ApplyFilter));

    reduce(&mut app, &Action::Resources(ResourcesAction::StartFilter));
    reduce(
        &mut app,
        &Action::Resources(ResourcesAction::AppendFilterChar('-')),
    );
    reduce(&mut app, &Action::Resources(ResourcesAction::ApplyFilter));

    assert_eq!(app.root.resources.filter_applied(), "api-");
    assert_eq!(app.root.resources.filter_draft(), "api-");
}

#[test]
fn resource_stale_clear_closes_active_filter_input_and_preserves_applied_filter_and_sort() {
    let mut app = AppState::new();
    load_resource_rows(
        &mut app,
        vec![instance("api-dev-1"), instance("worker-dev-1")],
    );

    reduce(&mut app, &Action::Resources(ResourcesAction::StartFilter));
    for ch in "api".chars() {
        reduce(
            &mut app,
            &Action::Resources(ResourcesAction::AppendFilterChar(ch)),
        );
    }
    reduce(&mut app, &Action::Resources(ResourcesAction::ApplyFilter));
    reduce(
        &mut app,
        &Action::Resources(ResourcesAction::SelectNextColumn),
    );
    reduce(
        &mut app,
        &Action::Resources(ResourcesAction::SortBySelectedColumn),
    );

    reduce(&mut app, &Action::Resources(ResourcesAction::StartFilter));
    reduce(
        &mut app,
        &Action::Resources(ResourcesAction::AppendFilterChar('-')),
    );
    assert!(app.root.resources.is_filter_input_active());

    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ConfigurationActivated {
            configuration: config("prod", true, "prod-project"),
        }),
    );

    assert!(app.root.resources.is_stale());
    assert!(app.root.resources.instances().is_empty());
    assert!(!app.root.resources.is_filter_input_active());
    assert_eq!(app.root.resources.filter_applied(), "api");
    assert_eq!(app.root.resources.filter_draft(), "api");
    assert_eq!(app.root.resources.sort_label(), "Status");
}

#[test]
fn resource_filter_matches_all_searchable_fields_case_insensitively() {
    for (query, expected) in [
        ("API", "api-dev-1"),
        ("west1-b", "api-dev-1"),
        ("running", "api-dev-1"),
        ("E2-MED", "api-dev-1"),
        ("10.0.0.4", "api-dev-1"),
        ("34.10", "api-dev-1"),
        ("stopped", "worker-dev-1"),
        ("n2-standard", "worker-dev-1"),
    ] {
        let mut app = AppState::new();
        load_resource_rows(
            &mut app,
            vec![
                instance_with_fields(
                    "api-dev-1",
                    "europe-west1-b",
                    ResourceStatus::Running,
                    "e2-medium",
                    Some("10.0.0.4"),
                    Some("34.10.10.10"),
                ),
                instance_with_fields(
                    "worker-dev-1",
                    "us-central1-a",
                    ResourceStatus::Stopped,
                    "n2-standard-2",
                    Some("10.0.0.5"),
                    None,
                ),
            ],
        );

        reduce(&mut app, &Action::Resources(ResourcesAction::StartFilter));
        for ch in query.chars() {
            reduce(
                &mut app,
                &Action::Resources(ResourcesAction::AppendFilterChar(ch)),
            );
        }
        reduce(&mut app, &Action::Resources(ResourcesAction::ApplyFilter));

        assert_eq!(
            app.root
                .resources
                .visible_instances()
                .into_iter()
                .map(|instance| instance.name.as_str())
                .collect::<Vec<_>>(),
            vec![expected],
            "query {query}"
        );
    }
}

// --- multi-mode filter grammar, observed through the reducer -------------

/// The two-row fixture every mode test below narrows, so regex and inverse can
/// be asserted against the same set.
fn app_with_filter_fixture() -> AppState {
    let mut app = AppState::new();
    load_resource_rows(
        &mut app,
        vec![instance("api-dev-1"), instance("worker-dev-1")],
    );
    app
}

#[test]
fn resource_filter_regex_mode_narrows_partially_and_case_insensitively() {
    for input in ["API", "PI-DEV", "api-.*-1"] {
        let mut app = app_with_filter_fixture();
        apply_filter_input(&mut app, input);

        assert_eq!(
            visible_instance_names(&app),
            vec!["api-dev-1"],
            "input {input}"
        );
        assert_eq!(app.root.resources.filter_mode(), FilterMode::Regex);
        assert_eq!(app.root.resources.filter_error(), None, "input {input}");
    }
}

#[test]
fn resource_filter_fuzzy_mode_narrows_by_subsequence() {
    let mut fuzzy = app_with_filter_fixture();
    apply_filter_input(&mut fuzzy, "-f apd");
    assert_eq!(visible_instance_names(&fuzzy), vec!["api-dev-1"]);
    assert_eq!(fuzzy.root.resources.filter_mode(), FilterMode::Fuzzy);
    assert_eq!(fuzzy.root.resources.filter_error(), None);

    // The same needle as a plain (regex/substring) filter drops that row, so
    // the match above can only come from subsequence matching.
    let mut regex = app_with_filter_fixture();
    apply_filter_input(&mut regex, "apd");
    assert_eq!(visible_instance_names(&regex), Vec::<&str>::new());
}

#[test]
fn resource_filter_inverse_mode_keeps_exactly_the_rows_regex_mode_drops() {
    let mut regex = app_with_filter_fixture();
    apply_filter_input(&mut regex, "api");
    assert_eq!(visible_instance_names(&regex), vec!["api-dev-1"]);

    let mut inverse = app_with_filter_fixture();
    apply_filter_input(&mut inverse, "!api");
    assert_eq!(visible_instance_names(&inverse), vec!["worker-dev-1"]);
    assert_eq!(inverse.root.resources.filter_mode(), FilterMode::Inverse);
    assert_eq!(inverse.root.resources.filter_error(), None);
}

/// Label mode is server-authoritative: list rows carry no labels, so the
/// client keeps every row the server returned.
#[test]
fn resource_filter_label_mode_leaves_every_loaded_row_visible() {
    let mut app = app_with_filter_fixture();
    apply_filter_input(&mut app, "-l env=prod");

    assert_eq!(
        visible_instance_names(&app),
        vec!["api-dev-1", "worker-dev-1"]
    );
    assert_eq!(app.root.resources.filter_mode(), FilterMode::Label);
    assert_eq!(app.root.resources.filter_error(), None);
}

/// An empty table behind a bad regex is indistinguishable from a correct empty
/// result, so a filter that will not compile narrows nothing and reports the
/// reason inline instead.
#[test]
fn resource_filter_invalid_regex_leaves_every_row_visible_and_reports_the_error() {
    let mut app = app_with_filter_fixture();
    apply_filter_input(&mut app, "a(b");

    assert_eq!(
        visible_instance_names(&app),
        vec!["api-dev-1", "worker-dev-1"]
    );
    let error = app
        .root
        .resources
        .filter_error()
        .expect("an uncompilable pattern must report an inline error");
    assert!(
        error.starts_with("invalid regex: "),
        "unexpected filter error: {error}"
    );
}

/// A filter that fails to *parse* (as opposed to failing to compile) is the
/// other unfiltered-plus-error case: `-l foo` has no `=`, so the grammar
/// rejects it before any pattern exists. The table must still show every
/// loaded row, and the parse error must reach the inline report verbatim —
/// otherwise the user sees an unnarrowed table with no explanation.
#[test]
fn resource_filter_error_reports_a_malformed_label_selector_and_keeps_every_row_visible() {
    let mut app = app_with_filter_fixture();
    apply_filter_input(&mut app, "-l foo");

    assert_eq!(
        app.root.resources.filter_error().as_deref(),
        Some("malformed label selector: expected key=value")
    );
    assert_eq!(
        visible_instance_names(&app),
        vec!["api-dev-1", "worker-dev-1"]
    );
}

#[test]
fn resource_filter_error_reports_label_mode_unsupported_per_service() {
    let mut cloud_run = AppState::new();
    reduce(
        &mut cloud_run,
        &Action::EffectResult(EffectResult::ConfigurationsLoaded {
            configurations: vec![config("dev", true, "dev-project")],
        }),
    );
    reduce(
        &mut cloud_run,
        &Action::Resources(ResourcesAction::SelectService(
            ResourceServiceKind::CloudRunServices,
        )),
    );
    apply_filter_input(&mut cloud_run, "-l env=prod");
    assert_eq!(
        cloud_run.root.resources.filter_error().as_deref(),
        Some("label filtering is not supported for Cloud Run")
    );

    let mut compute = app_with_filter_fixture();
    apply_filter_input(&mut compute, "-l env=prod");
    assert_eq!(
        compute.root.resources.filter_error(),
        None,
        "Compute instances do carry labels, so the same input is valid there"
    );
}

/// A dotted label key is rejected by the pushdown (`labels.a.b = "c"` would
/// read as a nested field path) and so is a key carrying any character
/// outside `[A-Za-z0-9._-]`, such as a slash. The state layer must report the
/// same "not expressible" reason for both rather than silently narrowing
/// nothing.
#[test]
fn resource_filter_error_reports_dotted_or_otherwise_unexpressible_label_keys() {
    for input in ["-l a.b=c", "-l e/v=prod"] {
        let mut app = app_with_filter_fixture();
        apply_filter_input(&mut app, input);

        assert_eq!(
            app.root.resources.filter_error().as_deref(),
            Some("label selector is not expressible as a gcloud filter"),
            "input {input}"
        );
        assert_eq!(
            visible_instance_names(&app),
            vec!["api-dev-1", "worker-dev-1"],
            "input {input}"
        );
    }
}

#[test]
fn resource_filter_mode_follows_the_draft_while_the_input_is_active() {
    let mut app = app_with_filter_fixture();
    apply_filter_input(&mut app, "!api");
    assert_eq!(app.root.resources.filter_mode(), FilterMode::Inverse);

    reduce(&mut app, &Action::Resources(ResourcesAction::StartFilter));
    for _ in 0.."!api".len() {
        reduce(
            &mut app,
            &Action::Resources(ResourcesAction::BackspaceFilter),
        );
    }
    for ch in "-f api".chars() {
        reduce(
            &mut app,
            &Action::Resources(ResourcesAction::AppendFilterChar(ch)),
        );
    }
    assert_eq!(
        app.root.resources.filter_mode(),
        FilterMode::Fuzzy,
        "the chip must follow the half-typed draft, not the applied text"
    );

    reduce(&mut app, &Action::Resources(ResourcesAction::CancelFilter));
    assert_eq!(
        app.root.resources.filter_mode(),
        FilterMode::Inverse,
        "cancelling returns the chip to the applied text"
    );
}

#[test]
fn resource_sort_cycle_and_visible_selection_clamp() {
    let mut app = AppState::new();
    load_resource_rows(
        &mut app,
        vec![
            instance_with_fields(
                "worker-dev-1",
                "europe-west1-c",
                ResourceStatus::Stopped,
                "n2-standard-2",
                None,
                None,
            ),
            instance_with_fields(
                "api-dev-1",
                "europe-west1-b",
                ResourceStatus::Running,
                "e2-medium",
                None,
                None,
            ),
        ],
    );

    assert_eq!(app.root.resources.sort_label(), "Name");
    assert_eq!(
        app.root.resources.visible_instances()[0].name.as_str(),
        "api-dev-1"
    );

    // Column 1 = Status
    reduce(
        &mut app,
        &Action::Resources(ResourcesAction::SelectNextColumn),
    );
    reduce(
        &mut app,
        &Action::Resources(ResourcesAction::SortBySelectedColumn),
    );
    assert_eq!(app.root.resources.sort_label(), "Status");
    // Column 2 = Zone
    reduce(
        &mut app,
        &Action::Resources(ResourcesAction::SelectNextColumn),
    );
    reduce(
        &mut app,
        &Action::Resources(ResourcesAction::SortBySelectedColumn),
    );
    assert_eq!(app.root.resources.sort_label(), "Zone");
    // Column 3 = Machine Type
    reduce(
        &mut app,
        &Action::Resources(ResourcesAction::SelectNextColumn),
    );
    reduce(
        &mut app,
        &Action::Resources(ResourcesAction::SortBySelectedColumn),
    );
    assert_eq!(app.root.resources.sort_label(), "Machine Type");
    // Back to column 0 = Name
    reduce(
        &mut app,
        &Action::Resources(ResourcesAction::SelectPreviousColumn),
    );
    reduce(
        &mut app,
        &Action::Resources(ResourcesAction::SelectPreviousColumn),
    );
    reduce(
        &mut app,
        &Action::Resources(ResourcesAction::SelectPreviousColumn),
    );
    reduce(
        &mut app,
        &Action::Resources(ResourcesAction::SortBySelectedColumn),
    );
    assert_eq!(app.root.resources.sort_label(), "Name");

    reduce(&mut app, &Action::Resources(ResourcesAction::SelectNext));
    reduce(&mut app, &Action::Resources(ResourcesAction::StartFilter));
    reduce(
        &mut app,
        &Action::Resources(ResourcesAction::AppendFilterChar('a')),
    );
    reduce(
        &mut app,
        &Action::Resources(ResourcesAction::AppendFilterChar('p')),
    );
    reduce(&mut app, &Action::Resources(ResourcesAction::ApplyFilter));

    assert_eq!(app.root.resources.selected_visible_index(), Some(0));
    assert_eq!(
        app.root
            .resources
            .selected_visible_instance()
            .map(|instance| instance.name.as_str()),
        Some("api-dev-1")
    );
}

#[test]
fn opening_detail_uses_selected_visible_resource_row() {
    let mut app = AppState::new();
    load_resource_rows(
        &mut app,
        vec![instance("api-dev-1"), instance("worker-dev-1")],
    );
    reduce(&mut app, &Action::Resources(ResourcesAction::StartFilter));
    for ch in "worker".chars() {
        reduce(
            &mut app,
            &Action::Resources(ResourcesAction::AppendFilterChar(ch)),
        );
    }
    reduce(&mut app, &Action::Resources(ResourcesAction::ApplyFilter));

    let context = app.root.current_context().unwrap().clone();
    let effects = reduce(&mut app, &Action::Resources(ResourcesAction::OpenSelected));

    assert_eq!(
        effects,
        vec![Effect::LoadComputeInstanceDetail {
            context,
            instance: instance("worker-dev-1"),
        }]
    );
}

#[test]
fn opening_detail_with_no_visible_rows_emits_no_effect() {
    let mut app = AppState::new();
    load_resource_rows(&mut app, vec![instance("api-dev-1")]);
    reduce(&mut app, &Action::Resources(ResourcesAction::StartFilter));
    for ch in "missing".chars() {
        reduce(
            &mut app,
            &Action::Resources(ResourcesAction::AppendFilterChar(ch)),
        );
    }
    reduce(&mut app, &Action::Resources(ResourcesAction::ApplyFilter));

    let effects = reduce(&mut app, &Action::Resources(ResourcesAction::OpenSelected));

    assert!(effects.is_empty());
}

#[test]
fn confirming_pending_start_action_emits_start_effect_and_closes_confirmation() {
    let mut app = app_with_instances();
    let context = app.root.current_context().expect("context exists");
    let instance = app
        .root
        .resources
        .selected_instance()
        .expect("selected")
        .clone();
    app.root.resources.start_action_confirmation(
        context.clone(),
        instance.clone(),
        ComputeInstanceAction::Start,
    );

    let effects = reduce(
        &mut app,
        &Action::Resources(ResourcesAction::ConfirmPendingAction),
    );

    // The effect carries the id of the entry this confirmation seeded, which is
    // what lets its result settle that entry rather than any other pending one
    // for the same instance.
    let operation_id = app.root.system.operations.records().front().unwrap().id;
    assert_eq!(
        effects,
        vec![Effect::ComputeInstanceAction {
            context,
            instance,
            action: ComputeInstanceAction::Start,
            operation_id,
        }]
    );
    assert!(app.root.resources.action_confirmation().is_none());
}

#[test]
fn confirming_pending_stop_action_emits_typed_effect_and_closes_confirmation() {
    let mut app = app_with_instances();
    let context = app.root.current_context().expect("context exists");
    let instance = app
        .root
        .resources
        .selected_instance()
        .expect("selected")
        .clone();
    app.root.resources.start_action_confirmation(
        context.clone(),
        instance.clone(),
        ComputeInstanceAction::Stop,
    );

    let effects = reduce(
        &mut app,
        &Action::Resources(ResourcesAction::ConfirmPendingAction),
    );

    let operation_id = app.root.system.operations.records().front().unwrap().id;
    assert_eq!(
        effects,
        vec![Effect::ComputeInstanceAction {
            context,
            instance,
            action: ComputeInstanceAction::Stop,
            operation_id,
        }]
    );
    assert!(app.root.resources.action_confirmation().is_none());
}

#[test]
fn canceling_pending_resource_action_emits_no_effect() {
    let mut app = app_with_instances();
    let context = app.root.current_context().expect("context exists");
    let instance = app
        .root
        .resources
        .selected_instance()
        .expect("selected")
        .clone();
    app.root
        .resources
        .start_action_confirmation(context, instance, ComputeInstanceAction::Stop);

    let effects = reduce(
        &mut app,
        &Action::Resources(ResourcesAction::CancelPendingAction),
    );

    assert!(effects.is_empty());
    assert!(app.root.resources.action_confirmation().is_none());
}

#[test]
fn stale_clear_resets_compute_collection_and_action_confirmation() {
    let mut app = app_with_instances();
    let context = app.root.current_context().unwrap().clone();
    app.root.resources.start_action_confirmation(
        context,
        instance("api-dev-1"),
        ComputeInstanceAction::Start,
    );
    assert!(app.root.resources.action_confirmation().is_some());

    app.root.resources.mark_all_stale_and_clear();

    assert!(app.root.resources.is_stale());
    assert!(app.root.resources.instances().is_empty());
    assert_eq!(app.root.resources.selected_index(), None);
    assert_eq!(app.root.resources.detail(), None);
    assert_eq!(app.root.resources.action_confirmation(), None);
}

#[test]
fn clipboard_results_only_update_status_message() {
    let mut app = app_with_instances();
    let context = app.root.current_context().expect("context exists").clone();
    let rows = app.root.resources.instances().to_vec();

    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ClipboardCopied {
            description: "instance name".to_string(),
        }),
    );

    assert_eq!(
        app.root.system.status_message.as_deref(),
        Some("Copied instance name to clipboard")
    );
    assert_eq!(app.root.system.status_operation_label, None);
    assert_eq!(app.root.current_context(), Some(context.clone()));
    assert_eq!(app.root.resources.instances(), rows.as_slice());

    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ClipboardCopyFailed {
            description: "instance name".to_string(),
            error: AppError::gcloud("clipboard unavailable"),
        }),
    );

    assert_eq!(
        app.root.system.status_message.as_deref(),
        Some("unknown error — Retry, or copy the details to investigate")
    );
    assert_eq!(app.root.system.status_operation_label, None);
    assert_eq!(app.root.current_context(), Some(context));
    assert_eq!(app.root.resources.instances(), rows.as_slice());
}

#[test]
fn mutation_confirmation_clears_ssh_preview() {
    let mut app = app_with_instances();
    let context = app.root.current_context().expect("context exists").clone();
    let instance = app
        .root
        .resources
        .selected_instance()
        .expect("selected")
        .clone();
    app.root
        .resources
        .show_ssh_command_preview(context.clone(), instance.clone());
    assert!(app.root.resources.command_preview().is_some());

    app.root
        .resources
        .start_action_confirmation(context, instance, ComputeInstanceAction::Stop);

    assert!(app.root.resources.command_preview().is_none());
}

/// One of the mutually exclusive modal slots on `ResourcesState`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Modal {
    ActionConfirmation,
    BatchConfirmation,
    CommandPreview,
    SshLaunchPreview,
    TunnelLaunchPreview,
    K9sLaunchConfirmation,
    PluginConfirmation,
    PluginPrompt,
}

impl Modal {
    const ALL: [Self; 8] = [
        Self::ActionConfirmation,
        Self::BatchConfirmation,
        Self::CommandPreview,
        Self::SshLaunchPreview,
        Self::TunnelLaunchPreview,
        Self::K9sLaunchConfirmation,
        Self::PluginConfirmation,
        Self::PluginPrompt,
    ];

    fn is_open(self, resources: &ResourcesState) -> bool {
        match self {
            Self::ActionConfirmation => resources.action_confirmation().is_some(),
            Self::BatchConfirmation => resources.batch_confirmation().is_some(),
            Self::CommandPreview => resources.command_preview().is_some(),
            Self::SshLaunchPreview => resources.ssh_launch_preview().is_some(),
            Self::TunnelLaunchPreview => resources.tunnel_launch_preview().is_some(),
            Self::K9sLaunchConfirmation => resources.k9s_launch_confirmation().is_some(),
            Self::PluginConfirmation => resources.plugin_confirmation().is_some(),
            Self::PluginPrompt => resources.plugin_prompt().is_some(),
        }
    }
}

fn open_modals(resources: &ResourcesState) -> Vec<Modal> {
    Modal::ALL
        .into_iter()
        .filter(|modal| modal.is_open(resources))
        .collect()
}

/// Every public opener of a modal, and the slot each one fills. Two openers
/// fill the single-row action slot, because the services own their own.
#[derive(Debug, Clone, Copy)]
enum Opener {
    ComputeActionConfirmation,
    CloudSqlActionConfirmation,
    BatchConfirmation,
    CommandPreview,
    SshLaunchPreview,
    TunnelLaunchPreview,
    K9sLaunchConfirmation,
    PluginConfirmation,
    PluginPrompt,
}

impl Opener {
    const ALL: [Self; 9] = [
        Self::ComputeActionConfirmation,
        Self::CloudSqlActionConfirmation,
        Self::BatchConfirmation,
        Self::CommandPreview,
        Self::SshLaunchPreview,
        Self::TunnelLaunchPreview,
        Self::K9sLaunchConfirmation,
        Self::PluginConfirmation,
        Self::PluginPrompt,
    ];

    fn modal(self) -> Modal {
        match self {
            Self::ComputeActionConfirmation | Self::CloudSqlActionConfirmation => {
                Modal::ActionConfirmation
            }
            Self::BatchConfirmation => Modal::BatchConfirmation,
            Self::CommandPreview => Modal::CommandPreview,
            Self::SshLaunchPreview => Modal::SshLaunchPreview,
            Self::TunnelLaunchPreview => Modal::TunnelLaunchPreview,
            Self::K9sLaunchConfirmation => Modal::K9sLaunchConfirmation,
            Self::PluginConfirmation => Modal::PluginConfirmation,
            Self::PluginPrompt => Modal::PluginPrompt,
        }
    }

    fn open(self, app: &mut AppState) {
        let context = app.root.current_context().expect("context exists").clone();
        let instance = app
            .root
            .resources
            .selected_instance()
            .expect("selected")
            .clone();
        match self {
            Self::ComputeActionConfirmation => app.root.resources.start_action_confirmation(
                context,
                instance,
                ComputeInstanceAction::Stop,
            ),
            Self::CloudSqlActionConfirmation => {
                app.root.resources.start_cloud_sql_action_confirmation(
                    context,
                    cloud_sql_instance("db-dev"),
                    CloudSqlInstanceAction::Restart,
                )
            }
            Self::BatchConfirmation => app
                .root
                .resources
                .start_batch_confirmation::<Compute>(context, ComputeInstanceAction::Stop),
            Self::CommandPreview => app.root.resources.show_command_preview(
                "Show command".to_string(),
                "gcloud --configuration dev compute instances describe api-dev-1".to_string(),
                "command".to_string(),
            ),
            Self::SshLaunchPreview => app
                .root
                .resources
                .show_ssh_launch_preview(context, instance),
            Self::TunnelLaunchPreview => app.root.resources.show_tunnel_launch_preview(
                context,
                TunnelLaunchPlan {
                    kind: TunnelKind::IapTcp,
                    target_label: instance.name.clone(),
                    remote_label: "22".to_string(),
                    instance_name: instance.name.clone(),
                    zone: instance.zone.clone(),
                    connection_name: String::new(),
                    preferred_local_port: Some(8022),
                    remote_port: 22,
                    claimed_ports: Vec::new(),
                },
            ),
            Self::K9sLaunchConfirmation => {
                let target =
                    resolve_launch_target(&context, &gke_cluster("platform-dev", "europe-west1-b"))
                        .expect("the seeded context names a project");
                app.root
                    .resources
                    .open_k9s_launch_confirmation(target, CredentialStep::ReuseExisting);
            }
            Self::PluginConfirmation => app.root.resources.show_plugin_confirmation(
                "Describe api-dev-1".to_string(),
                vec![
                    "--configuration".to_string(),
                    "dev".to_string(),
                    "compute".to_string(),
                    "instances".to_string(),
                    "describe".to_string(),
                    "api-dev-1".to_string(),
                ],
                false,
            ),
            // A non-empty input list, because `PluginPrompt`'s focus movement
            // documents a non-empty invariant its callers uphold — a fixture
            // seeded with no fields would exercise the one shape the type is
            // not built for.
            Self::PluginPrompt => app.root.resources.show_plugin_prompt(PluginPrompt::new(
                PluginId(0),
                "Resize api-dev-1".to_string(),
                false,
                vec![PluginInput {
                    name: "disk_size".to_string(),
                    kind: PluginInputKind::String,
                    default: Some("100".to_string()),
                }],
                Vec::new(),
            )),
        }
    }
}

/// Opening any modal over any other leaves exactly one open.
///
/// Two open at once is a state the keymap has no mode for: `render_active_overlay`
/// is a priority chain, so it shows whichever it reaches first while the keymap
/// dispatches for the other, and the second modal takes keys the operator cannot
/// see it taking. Nothing in the type system says the slots are exclusive — one
/// `= None` line per opener per neighbour is all that ever said it.
///
/// The walk is every ordered *pair*, not one combined case: opening all eight
/// together and asserting one survives would still pass with most of the
/// clearing deleted. Pairs are what caught the original hole, where a reviewer
/// deleted a clearing line from any of nine openers and the whole suite stayed
/// green.
///
/// The assertion is "exactly one modal is open", never "this neighbour was
/// cleared", so a modal added to `Modal::ALL` without an opener in
/// `Opener::ALL` fails here rather than going unchecked — and so does an opener
/// that forgets to close the set.
#[test]
fn opening_any_modal_over_any_other_leaves_exactly_one_open() {
    for seed in Opener::ALL {
        for opener in Opener::ALL {
            let mut app = app_with_instances();
            // The batch opener is a no-op with nothing marked, which would make
            // its cases assert about an empty modal set rather than about
            // exclusion. Marking the selected (Running) row up front, once,
            // keeps every case comparable — and marking here rather than inside
            // the opener keeps `Batch` seeding `Batch` from toggling the mark
            // back off.
            app.root.resources.toggle_mark();

            seed.open(&mut app);
            assert_eq!(
                open_modals(&app.root.resources),
                vec![seed.modal()],
                "seeding {seed:?} did not leave exactly its own modal open"
            );

            opener.open(&mut app);
            assert_eq!(
                open_modals(&app.root.resources),
                vec![opener.modal()],
                "{opener:?} opened over {seed:?} did not leave exactly one modal open"
            );
        }
    }
}

#[test]
fn confirm_ssh_launch_emits_interactive_effect() {
    let mut app = app_with_instances();
    let context = app.root.current_context().expect("context exists").clone();
    let instance = app
        .root
        .resources
        .selected_instance()
        .expect("selected")
        .clone();
    app.root
        .resources
        .show_ssh_launch_preview(context.clone(), instance.clone());

    let effects = reduce(
        &mut app,
        &Action::Resources(ResourcesAction::ConfirmSshLaunch),
    );

    assert_eq!(
        effects,
        vec![Effect::LaunchInteractiveProcess {
            command: nuvem::app::effect::InteractiveCommand::GceSsh { context, instance },
        }]
    );
    assert!(app.root.resources.ssh_launch_preview().is_none());
}

#[test]
fn cancel_ssh_launch_clears_preview() {
    let mut app = app_with_instances();
    let context = app.root.current_context().expect("context exists").clone();
    let instance = app
        .root
        .resources
        .selected_instance()
        .expect("selected")
        .clone();
    app.root
        .resources
        .show_ssh_launch_preview(context, instance);

    let effects = reduce(
        &mut app,
        &Action::Resources(ResourcesAction::CancelSshLaunch),
    );

    assert!(effects.is_empty());
    assert!(app.root.resources.ssh_launch_preview().is_none());
}

#[test]
fn generic_collection_preserves_existing_compute_filter_sort_selection() {
    let mut app = AppState::new();
    load_resource_rows(
        &mut app,
        vec![
            instance_with_fields(
                "worker-dev-1",
                "europe-west1-c",
                ResourceStatus::Stopped,
                "e2-small",
                Some("10.0.0.5"),
                None,
            ),
            instance_with_fields(
                "api-dev-1",
                "europe-west1-b",
                ResourceStatus::Running,
                "e2-medium",
                Some("10.0.0.4"),
                Some("34.10.10.10"),
            ),
        ],
    );

    app.root.resources.start_filter();
    for ch in "worker".chars() {
        app.root.resources.append_filter_char(ch);
    }
    app.root.resources.apply_filter();
    app.root.resources.select_next_column();
    app.root.resources.sort_selected_column();

    assert_eq!(app.root.resources.visible_instances().len(), 1);
    assert_eq!(
        app.root
            .resources
            .selected_instance()
            .map(|row| row.name.as_str()),
        Some("worker-dev-1")
    );
    assert_eq!(app.root.resources.sort_label(), "Status");
}

#[test]
fn compute_action_success_marks_rows_stale_and_sets_status() {
    let mut app = app_with_instances();
    let context = app.root.current_context().expect("context exists");

    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ComputeInstanceActionSucceeded {
            context,
            instance_name: "api-dev-1".to_string(),
            instance_zone: "europe-west1-b".to_string(),
            action: ComputeInstanceAction::Stop,
            operation_id: UNTRACKED_OPERATION_ID,
        }),
    );

    assert!(app.root.resources.is_stale());
    assert!(app.root.resources.instances().is_empty());
    assert_eq!(
        app.root.system.status_message.as_deref(),
        Some("Stop accepted for api-dev-1; refreshing GCE instances")
    );
    assert_eq!(
        app.root.system.status_operation_label.as_deref(),
        Some("GCE action")
    );
}

#[test]
fn compute_action_success_starts_context_scoped_refresh() {
    let mut app = app_with_instances();
    let context = app.root.current_context().expect("context exists").clone();

    let effects = reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ComputeInstanceActionSucceeded {
            context: context.clone(),
            instance_name: "api-dev-1".to_string(),
            instance_zone: "europe-west1-b".to_string(),
            action: ComputeInstanceAction::Start,
            operation_id: UNTRACKED_OPERATION_ID,
        }),
    );

    assert_eq!(
        effects,
        vec![Effect::RefreshComputeInstances {
            context,
            filter: None
        }]
    );
    assert!(app.root.resources.is_stale());
    assert!(app.root.resources.instances().is_empty());
    assert_eq!(
        app.root.system.status_message.as_deref(),
        Some("Start accepted for api-dev-1; refreshing GCE instances")
    );
}

#[test]
fn compute_follow_up_refresh_success_restores_rows_and_clears_status() {
    let mut app = app_with_instances();
    let context = app.root.current_context().expect("context exists").clone();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ComputeInstanceActionSucceeded {
            context: context.clone(),
            instance_name: "api-dev-1".to_string(),
            instance_zone: "europe-west1-b".to_string(),
            action: ComputeInstanceAction::Stop,
            operation_id: UNTRACKED_OPERATION_ID,
        }),
    );

    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ComputeInstancesLoaded {
            context,
            instances: vec![instance("api-dev-1")],
        }),
    );

    assert!(!app.root.resources.is_stale());
    assert_eq!(app.root.resources.instances().len(), 1);
    assert_eq!(app.root.system.status_message, None);
}

#[test]
fn complete_batch_workflow_is_cleared_once_follow_up_refresh_applies() {
    use nuvem::app::state::resources::BatchTargetStatus;

    let mut app = app_with_instances();
    let context = app.root.current_context().expect("context exists").clone();
    app.root.resources.begin_batch_workflow(
        BatchAction::Compute(ComputeInstanceAction::Stop),
        context.clone(),
        with_stand_in_ids(vec![batch_target("api-dev-1", &context)]),
    );
    app.root.resources.resolve_batch_target(
        ResourceServiceKind::ComputeInstances,
        "api-dev-1",
        Some("europe-west1-b"),
        BatchTargetStatus::Succeeded,
    );
    assert!(
        app.root
            .resources
            .batch_workflow()
            .expect("workflow present")
            .is_complete()
    );

    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ComputeInstancesLoaded {
            context,
            instances: vec![instance("api-dev-1")],
        }),
    );

    assert!(app.root.resources.batch_workflow().is_none());
}

#[test]
fn incomplete_batch_workflow_survives_a_concurrent_refresh() {
    use nuvem::app::state::resources::BatchTargetStatus;

    let mut app = app_with_instances();
    let context = app.root.current_context().expect("context exists").clone();
    app.root.resources.begin_batch_workflow(
        BatchAction::Compute(ComputeInstanceAction::Stop),
        context.clone(),
        with_stand_in_ids(vec![
            batch_target("api-dev-1", &context),
            batch_target("worker-dev-1", &context),
        ]),
    );
    app.root.resources.resolve_batch_target(
        ResourceServiceKind::ComputeInstances,
        "api-dev-1",
        Some("europe-west1-b"),
        BatchTargetStatus::Succeeded,
    );
    // "worker-dev-1" stays Pending, so the workflow is not complete yet.
    assert!(
        !app.root
            .resources
            .batch_workflow()
            .expect("workflow present")
            .is_complete()
    );

    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ComputeInstancesLoaded {
            context,
            instances: vec![instance("api-dev-1"), instance("worker-dev-1")],
        }),
    );

    assert!(app.root.resources.batch_workflow().is_some());
}

#[test]
fn complete_cloud_sql_batch_workflow_survives_a_compute_refresh() {
    use nuvem::app::state::resources::{BatchTargetStatus, CloudSql, CloudSqlInstanceAction};

    let mut app = app_with_cloud_sql(vec![cloud_sql_instance("orders-dev")]);
    let context = app.root.current_context().expect("context exists").clone();
    app.root.resources.begin_batch_workflow(
        BatchAction::CloudSql(CloudSqlInstanceAction::Stop),
        context.clone(),
        with_stand_in_ids(vec![CloudSql::row_ref(
            &cloud_sql_instance("orders-dev"),
            &context,
            CloudSqlInstanceAction::Stop,
        )]),
    );
    app.root.resources.resolve_batch_target(
        ResourceServiceKind::CloudSql,
        "orders-dev",
        None,
        BatchTargetStatus::Succeeded,
    );

    // A Compute refresh landing before the Cloud SQL one must not dismiss the
    // Cloud SQL batch's progress panel.
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ComputeInstancesLoaded {
            context: context.clone(),
            instances: vec![instance("api-dev-1")],
        }),
    );
    assert!(app.root.resources.batch_workflow().is_some());

    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ComputeInstancesLoadFailed {
            context: context.clone(),
            error: AppError::gcloud("boom".to_string()),
        }),
    );
    assert!(app.root.resources.batch_workflow().is_some());

    // Its own refresh does dismiss it.
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::CloudSqlInstancesLoaded {
            context,
            instances: vec![cloud_sql_instance("orders-dev")],
        }),
    );
    assert!(app.root.resources.batch_workflow().is_none());
}

#[test]
fn complete_compute_batch_workflow_survives_a_cloud_sql_refresh() {
    use nuvem::app::state::resources::BatchTargetStatus;

    let mut app = app_with_instances();
    let context = app.root.current_context().expect("context exists").clone();
    app.root.resources.begin_batch_workflow(
        BatchAction::Compute(ComputeInstanceAction::Stop),
        context.clone(),
        with_stand_in_ids(vec![batch_target("api-dev-1", &context)]),
    );
    app.root.resources.resolve_batch_target(
        ResourceServiceKind::ComputeInstances,
        "api-dev-1",
        Some("europe-west1-b"),
        BatchTargetStatus::Succeeded,
    );

    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::CloudSqlInstancesLoaded {
            context: context.clone(),
            instances: vec![cloud_sql_instance("orders-dev")],
        }),
    );
    assert!(app.root.resources.batch_workflow().is_some());

    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::CloudSqlInstancesLoadFailed {
            context: context.clone(),
            error: AppError::gcloud("boom".to_string()),
        }),
    );
    assert!(app.root.resources.batch_workflow().is_some());

    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ComputeInstancesLoaded {
            context,
            instances: vec![instance("api-dev-1")],
        }),
    );
    assert!(app.root.resources.batch_workflow().is_none());
}

#[test]
fn stale_follow_up_refresh_failure_does_not_change_current_context() {
    let mut app = app_with_instances();
    let old_context = app.root.current_context().expect("context exists").clone();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ConfigurationsLoaded {
            configurations: vec![config("prod", true, "prod-project")],
        }),
    );
    let current_rows = app.root.resources.instances().to_vec();
    app.root.system.status_message = Some("ready".to_string());

    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ComputeInstancesLoadFailed {
            context: old_context,
            error: AppError::gcloud("late refresh failed"),
        }),
    );

    assert_eq!(app.root.resources.instances(), current_rows.as_slice());
    assert_eq!(app.root.system.status_message.as_deref(), Some("ready"));
}

#[test]
fn compute_action_failure_clears_rows_marks_stale_and_sets_status() {
    let mut app = app_with_instances();
    let context = app.root.current_context().expect("context exists");

    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ComputeInstanceActionFailed {
            context,
            instance_name: "api-dev-1".to_string(),
            instance_zone: "europe-west1-b".to_string(),
            action: ComputeInstanceAction::Stop,
            error: AppError::gcloud("stop failed"),
            operation_id: UNTRACKED_OPERATION_ID,
        }),
    );

    assert!(app.root.resources.is_stale());
    assert!(app.root.resources.instances().is_empty());
    assert_eq!(app.root.resources.selected_instance(), None);
    assert_eq!(
        app.root.system.status_message.as_deref(),
        Some("unknown error — Retry, or copy the details to investigate")
    );
    // The raw stderr ("stop failed") must never leak into the user-facing status.
    assert!(
        !app.root
            .system
            .status_message
            .as_deref()
            .unwrap_or_default()
            .contains("stop failed")
    );
}

#[test]
fn compute_action_result_preserves_loaded_gke_detail_for_same_context() {
    let mut app = app_with_instances();
    let context = app.root.current_context().unwrap().clone();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::GkeClustersLoaded {
            context: context.clone(),
            clusters: vec![gke_cluster("platform-dev", "europe-west1")],
        }),
    );
    reduce(
        &mut app,
        &Action::Resources(ResourcesAction::SelectService(
            ResourceServiceKind::GkeClusters,
        )),
    );
    reduce(&mut app, &Action::Resources(ResourcesAction::OpenSelected));
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::GkeClusterDetailLoaded {
            context: context.clone(),
            detail: gke_detail("platform-dev", "europe-west1"),
        }),
    );
    reduce(
        &mut app,
        &Action::Resources(ResourcesAction::SelectService(
            ResourceServiceKind::ComputeInstances,
        )),
    );

    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ComputeInstanceActionSucceeded {
            context,
            instance_name: "api-dev-1".to_string(),
            instance_zone: "europe-west1-b".to_string(),
            action: ComputeInstanceAction::Stop,
            operation_id: UNTRACKED_OPERATION_ID,
        }),
    );

    assert!(app.root.resources.is_stale());
    assert!(app.root.resources.instances().is_empty());
    assert_eq!(app.root.resources.gke_clusters().len(), 1);
    assert_eq!(
        app.root
            .resources
            .gke_detail()
            .map(|detail| detail.name.as_str()),
        Some("platform-dev")
    );
}

#[test]
fn stale_context_compute_action_results_are_ignored() {
    let mut app = app_with_instances();
    let dev_context = app.root.current_context().expect("context exists").clone();
    let original_rows = app.root.resources.instances().to_vec();

    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ConfigurationsLoaded {
            configurations: vec![
                config("dev", false, "dev-project"),
                config("prod", true, "prod-project"),
            ],
        }),
    );
    let prod_context = app.root.current_context().expect("context exists").clone();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ComputeInstancesLoaded {
            context: prod_context,
            instances: original_rows.clone(),
        }),
    );
    app.root.system.status_message = Some("ready".to_string());

    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ComputeInstanceActionSucceeded {
            context: dev_context.clone(),
            instance_name: "api-dev-1".to_string(),
            instance_zone: "europe-west1-b".to_string(),
            action: ComputeInstanceAction::Stop,
            operation_id: UNTRACKED_OPERATION_ID,
        }),
    );
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ComputeInstanceActionFailed {
            context: dev_context,
            instance_name: "api-dev-1".to_string(),
            instance_zone: "europe-west1-b".to_string(),
            action: ComputeInstanceAction::Start,
            error: AppError::gcloud("late failure"),
            operation_id: UNTRACKED_OPERATION_ID,
        }),
    );

    assert!(!app.root.resources.is_stale());
    assert_eq!(app.root.resources.instances(), original_rows);
    assert_eq!(app.root.system.status_message.as_deref(), Some("ready"));
}

#[test]
fn resources_select_service_switches_active_service_and_loads_it() {
    let mut app = AppState::new();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ConfigurationsLoaded {
            configurations: vec![config("dev", true, "dev-project")],
        }),
    );
    assert_eq!(
        app.root.resources.active_service(),
        ResourceServiceKind::ComputeInstances
    );

    let effects = reduce(
        &mut app,
        &Action::Resources(ResourcesAction::SelectService(
            ResourceServiceKind::GkeClusters,
        )),
    );

    assert_eq!(
        app.root.resources.active_service(),
        ResourceServiceKind::GkeClusters
    );
    // Plus the remembered-view save for the switch itself.
    assert_eq!(
        effects,
        vec![
            Effect::RefreshGkeClusters {
                context: app.root.current_context().unwrap().clone(),
                filter: None,
            },
            Effect::SaveRememberedViews {
                views: remembered("dev", ResourceServiceKind::GkeClusters),
            }
        ]
    );
}

#[test]
fn service_switching_preserves_compute_detail_and_filter_state() {
    let mut app = app_with_instances();
    reduce(&mut app, &Action::Resources(ResourcesAction::StartFilter));
    for ch in "api".chars() {
        reduce(
            &mut app,
            &Action::Resources(ResourcesAction::AppendFilterChar(ch)),
        );
    }

    assert!(app.root.resources.is_filter_input_active());
    assert_eq!(app.root.resources.filter_draft(), "api");

    reduce(
        &mut app,
        &Action::Resources(ResourcesAction::SelectService(
            ResourceServiceKind::GkeClusters,
        )),
    );
    reduce(
        &mut app,
        &Action::Resources(ResourcesAction::SelectService(
            ResourceServiceKind::ComputeInstances,
        )),
    );

    assert!(app.root.resources.is_filter_input_active());
    assert_eq!(app.root.resources.filter_draft(), "api");

    reduce(&mut app, &Action::Resources(ResourcesAction::ApplyFilter));
    let effects = reduce(&mut app, &Action::Resources(ResourcesAction::OpenSelected));

    assert!(app.root.resources.is_showing_detail());
    assert_eq!(
        effects,
        vec![Effect::LoadComputeInstanceDetail {
            context: app.root.current_context().unwrap().clone(),
            instance: instance("api-dev-1"),
        }]
    );

    reduce(
        &mut app,
        &Action::Resources(ResourcesAction::SelectService(
            ResourceServiceKind::GkeClusters,
        )),
    );
    reduce(
        &mut app,
        &Action::Resources(ResourcesAction::SelectService(
            ResourceServiceKind::ComputeInstances,
        )),
    );

    assert!(app.root.resources.is_showing_detail());
    assert_eq!(app.root.resources.filter_applied(), "api");
    assert_eq!(app.root.resources.detail_target_name(), Some("api-dev-1"));
}

#[test]
fn refresh_emits_compute_effect_when_compute_is_active() {
    let mut app = AppState::new();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ConfigurationsLoaded {
            configurations: vec![config("dev", true, "dev-project")],
        }),
    );
    reduce(
        &mut app,
        &Action::Resources(ResourcesAction::SelectService(
            ResourceServiceKind::ComputeInstances,
        )),
    );

    let effects = reduce(&mut app, &Action::Resources(ResourcesAction::Refresh));

    assert_eq!(
        effects,
        vec![Effect::RefreshComputeInstances {
            context: app.root.current_context().unwrap().clone(),
            filter: None,
        }]
    );
}

#[test]
fn refresh_emits_gke_effect_when_gke_is_active() {
    let mut app = AppState::new();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ConfigurationsLoaded {
            configurations: vec![config("dev", true, "dev-project")],
        }),
    );
    reduce(
        &mut app,
        &Action::Resources(ResourcesAction::SelectService(
            ResourceServiceKind::GkeClusters,
        )),
    );

    let effects = reduce(&mut app, &Action::Resources(ResourcesAction::Refresh));

    assert_eq!(
        effects,
        vec![Effect::RefreshGkeClusters {
            context: app.root.current_context().unwrap().clone(),
            filter: None,
        }]
    );
}

#[test]
fn gke_sort_cycle_changes_visible_cluster_order() {
    let mut app = AppState::new();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ConfigurationsLoaded {
            configurations: vec![config("dev", true, "dev-project")],
        }),
    );
    let context = app.root.current_context().unwrap().clone();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::GkeClustersLoaded {
            context,
            clusters: vec![
                gke_cluster_with_fields(
                    "omega",
                    "us-central1",
                    GkeClusterStatus::Error,
                    Some("1.31.5-gke.1023000"),
                    Some(9),
                ),
                gke_cluster_with_fields(
                    "alpha",
                    "us-west1",
                    GkeClusterStatus::Running,
                    Some("1.30.9-gke.1127000"),
                    Some(5),
                ),
                gke_cluster_with_fields(
                    "beta",
                    "asia-south1",
                    GkeClusterStatus::Reconciling,
                    Some("1.29.8-gke.1000000"),
                    Some(1),
                ),
            ],
        }),
    );
    reduce(
        &mut app,
        &Action::Resources(ResourcesAction::SelectService(
            ResourceServiceKind::GkeClusters,
        )),
    );

    assert_eq!(
        app.root
            .resources
            .visible_gke_clusters()
            .iter()
            .map(|cluster| cluster.name.as_str())
            .collect::<Vec<_>>(),
        vec!["alpha", "beta", "omega"]
    );
    assert_eq!(
        app.root
            .resources
            .selected_gke_cluster()
            .map(|cluster| cluster.name.as_str()),
        Some("alpha")
    );

    reduce(
        &mut app,
        &Action::Resources(ResourcesAction::SelectNextColumn),
    );
    reduce(
        &mut app,
        &Action::Resources(ResourcesAction::SortBySelectedColumn),
    );

    assert_eq!(app.root.resources.sort_label(), "Status");
    assert_eq!(
        app.root
            .resources
            .visible_gke_clusters()
            .iter()
            .map(|cluster| cluster.name.as_str())
            .collect::<Vec<_>>(),
        vec!["omega", "beta", "alpha"]
    );
    assert_eq!(
        app.root
            .resources
            .selected_gke_cluster()
            .map(|cluster| cluster.name.as_str()),
        Some("omega")
    );

    reduce(
        &mut app,
        &Action::Resources(ResourcesAction::SelectNextColumn),
    );
    reduce(
        &mut app,
        &Action::Resources(ResourcesAction::SortBySelectedColumn),
    );

    assert_eq!(app.root.resources.sort_label(), "Location");
    assert_eq!(
        app.root
            .resources
            .visible_gke_clusters()
            .iter()
            .map(|cluster| cluster.name.as_str())
            .collect::<Vec<_>>(),
        vec!["beta", "omega", "alpha"]
    );
    assert_eq!(
        app.root
            .resources
            .selected_gke_cluster()
            .map(|cluster| cluster.name.as_str()),
        Some("beta")
    );
}

#[test]
fn gke_list_load_success_updates_gke_collection_only() {
    let mut app = app_with_instances();
    let context = app.root.current_context().unwrap().clone();

    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::GkeClustersLoaded {
            context,
            clusters: vec![gke_cluster("platform-dev", "europe-west1")],
        }),
    );

    assert_eq!(app.root.resources.instances().len(), 2);
    assert_eq!(app.root.resources.gke_clusters().len(), 1);
    assert_eq!(app.root.resources.gke_clusters()[0].name, "platform-dev");
}

#[test]
fn opening_selected_gke_cluster_emits_detail_effect() {
    let mut app = AppState::new();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ConfigurationsLoaded {
            configurations: vec![config("dev", true, "dev-project")],
        }),
    );
    let context = app.root.current_context().unwrap().clone();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::GkeClustersLoaded {
            context: context.clone(),
            clusters: vec![gke_cluster("platform-dev", "europe-west1")],
        }),
    );
    reduce(
        &mut app,
        &Action::Resources(ResourcesAction::SelectService(
            ResourceServiceKind::GkeClusters,
        )),
    );

    let effects = reduce(&mut app, &Action::Resources(ResourcesAction::OpenSelected));

    assert!(app.root.resources.is_showing_detail());
    assert_eq!(
        effects,
        vec![Effect::LoadGkeClusterDetail {
            context,
            cluster: gke_cluster("platform-dev", "europe-west1"),
        }]
    );
}

#[test]
fn stale_gke_detail_result_for_previous_target_is_ignored() {
    let mut app = AppState::new();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ConfigurationsLoaded {
            configurations: vec![config("dev", true, "dev-project")],
        }),
    );
    let context = app.root.current_context().unwrap().clone();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::GkeClustersLoaded {
            context: context.clone(),
            clusters: vec![
                gke_cluster("platform-dev", "europe-west1"),
                gke_cluster("private-prod", "europe-west1-b"),
            ],
        }),
    );
    reduce(
        &mut app,
        &Action::Resources(ResourcesAction::SelectService(
            ResourceServiceKind::GkeClusters,
        )),
    );
    reduce(&mut app, &Action::Resources(ResourcesAction::OpenSelected));
    reduce(&mut app, &Action::Resources(ResourcesAction::CloseDetail));
    reduce(&mut app, &Action::Resources(ResourcesAction::SelectNext));
    reduce(&mut app, &Action::Resources(ResourcesAction::OpenSelected));

    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::GkeClusterDetailLoaded {
            context,
            detail: gke_detail("platform-dev", "europe-west1"),
        }),
    );

    assert_eq!(app.root.resources.gke_detail(), None);
    assert_eq!(
        app.root.resources.gke_detail_target_name(),
        Some("private-prod")
    );
}

#[test]
fn command_palette_has_no_compute_actions_when_gke_is_active() {
    let mut app = app_with_instances();
    reduce(
        &mut app,
        &Action::Resources(ResourcesAction::SelectService(
            ResourceServiceKind::GkeClusters,
        )),
    );
    app.root.command.open();

    let commands = app.root.command.visible_commands(&app.root);
    assert!(!commands.contains(&CommandDescriptor::StartSelectedComputeInstance));
    assert!(!commands.contains(&CommandDescriptor::StopSelectedComputeInstance));
    assert!(!commands.contains(&CommandDescriptor::ShowSelectedComputeInstanceSsh));
    // Navigation commands are still available.
    assert!(!commands.is_empty());
}

#[test]
fn confirming_gce_action_records_pending_operation() {
    use nuvem::app::state::operations::OperationOutcome;
    let mut app = app_with_instances();
    let context = app.root.current_context().expect("context exists");
    let instance = app
        .root
        .resources
        .selected_instance()
        .expect("selected")
        .clone();
    app.root.resources.start_action_confirmation(
        context,
        instance.clone(),
        ComputeInstanceAction::Stop,
    );

    let effects = reduce(
        &mut app,
        &Action::Resources(ResourcesAction::ConfirmPendingAction),
    );
    assert!(matches!(
        effects.as_slice(),
        [Effect::ComputeInstanceAction { .. }]
    ));
    let record = app.root.system.operations.records().front().unwrap();
    assert_eq!(record.kind, "Stop instance");
    assert_eq!(record.target, instance.name);
    assert_eq!(record.outcome, OperationOutcome::Pending);
}

#[test]
fn successful_action_result_resolves_operation() {
    use nuvem::app::state::operations::OperationOutcome;
    let mut app = app_with_instances();
    let context = app.root.current_context().expect("context exists");
    let instance = app
        .root
        .resources
        .selected_instance()
        .expect("selected")
        .clone();
    app.root.resources.start_action_confirmation(
        context.clone(),
        instance.clone(),
        ComputeInstanceAction::Stop,
    );
    let effects = reduce(
        &mut app,
        &Action::Resources(ResourcesAction::ConfirmPendingAction),
    );
    let operation_id = single_row_operation_id(&effects);

    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ComputeInstanceActionSucceeded {
            context,
            instance_name: instance.name.clone(),
            instance_zone: instance.zone.clone(),
            action: ComputeInstanceAction::Stop,
            operation_id,
        }),
    );

    let record = app.root.system.operations.records().front().unwrap();
    assert_eq!(record.outcome, OperationOutcome::Succeeded);
}

#[test]
fn action_result_for_superseded_context_still_resolves_operation() {
    use nuvem::app::state::operations::OperationOutcome;
    let mut app = app_with_instances();
    let stale_context = app.root.current_context().expect("context exists");
    let instance = app
        .root
        .resources
        .selected_instance()
        .expect("selected")
        .clone();
    app.root.resources.start_action_confirmation(
        stale_context.clone(),
        instance.clone(),
        ComputeInstanceAction::Stop,
    );
    let effects = reduce(
        &mut app,
        &Action::Resources(ResourcesAction::ConfirmPendingAction),
    );
    let operation_id = single_row_operation_id(&effects);

    // The active configuration changes before the action result arrives, so
    // the result now targets a context that is no longer current.
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ConfigurationActivated {
            configuration: config("prod", true, "prod-project"),
        }),
    );
    assert_ne!(app.root.current_context(), Some(stale_context.clone()));

    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ComputeInstanceActionSucceeded {
            context: stale_context,
            instance_name: instance.name.clone(),
            instance_zone: instance.zone.clone(),
            action: ComputeInstanceAction::Stop,
            operation_id,
        }),
    );

    // The pending operations-log entry must not be left stuck at Pending
    // just because the active context moved on in the meantime.
    let record = app.root.system.operations.records().front().unwrap();
    assert_eq!(record.outcome, OperationOutcome::Succeeded);
}

#[test]
fn failed_load_records_service_problem() {
    let mut app = AppState::new();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ConfigurationsLoaded {
            configurations: vec![config("dev", true, "dev-project")],
        }),
    );
    let context = app.root.current_context().unwrap().clone();

    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ComputeInstancesLoadFailed {
            context,
            error: AppError::gcloud("PERMISSION_DENIED: nope"),
        }),
    );

    assert_eq!(
        app.root.resources.compute_problem().unwrap().kind,
        GcloudErrorKind::PermissionDenied
    );
}

#[test]
fn open_selected_with_gke_active_does_not_open_compute_detail() {
    let mut app = app_with_instances();
    reduce(
        &mut app,
        &Action::Resources(ResourcesAction::SelectService(
            ResourceServiceKind::GkeClusters,
        )),
    );

    let effects = reduce(&mut app, &Action::Resources(ResourcesAction::OpenSelected));

    assert!(effects.is_empty());
    assert!(!app.root.resources.is_showing_detail());
    assert!(app.root.resources.detail_target().is_none());
    assert!(app.root.resources.detail().is_none());
}

#[test]
fn sort_selected_column_reverses_on_repeat_and_resets_on_change() {
    use nuvem::app::projection::table::SortDirection;
    let mut app = app_with_instances();

    // Column 0 is Name and is the default sort column.
    app.root.resources.sort_selected_column();
    assert_eq!(
        app.root.resources.sort_direction(),
        SortDirection::Descending
    );
    app.root.resources.sort_selected_column();
    assert_eq!(
        app.root.resources.sort_direction(),
        SortDirection::Ascending
    );

    // Move to column 1 (Status) and sort: direction resets to ascending.
    app.root.resources.select_next_column();
    app.root.resources.sort_selected_column();
    assert_eq!(app.root.resources.sort_column_index(), 1);
    assert_eq!(
        app.root.resources.sort_direction(),
        SortDirection::Ascending
    );
}

#[test]
fn refresh_restores_selection_by_identity_when_rows_reorder() {
    let mut app = AppState::new();
    load_resource_rows(
        &mut app,
        vec![instance("api-a"), instance("api-b"), instance("api-c")],
    );
    let context = app.root.current_context().unwrap().clone();
    // move selection to api-c (visible order is sorted by name: a,b,c)
    reduce(&mut app, &Action::Resources(ResourcesAction::SelectNext));
    reduce(&mut app, &Action::Resources(ResourcesAction::SelectNext));
    assert_eq!(
        app.root.resources.selected_instance().unwrap().name,
        "api-c"
    );
    // reload with a new instance sorted first; api-c must stay selected
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ComputeInstancesLoaded {
            context,
            instances: vec![
                instance("aaa-0"),
                instance("api-a"),
                instance("api-b"),
                instance("api-c"),
            ],
        }),
    );
    assert_eq!(
        app.root.resources.selected_instance().unwrap().name,
        "api-c"
    );
}

#[test]
fn column_cursor_clamps_within_bounds() {
    let mut app = app_with_instances();
    for _ in 0..10 {
        app.root.resources.select_next_column();
    }
    assert_eq!(app.root.resources.selected_column(), 3);
    for _ in 0..10 {
        app.root.resources.select_previous_column();
    }
    assert_eq!(app.root.resources.selected_column(), 0);
}

#[test]
fn wide_columns_toggle_flips() {
    let mut app = app_with_instances();
    assert!(!app.root.resources.is_wide_columns());
    app.root.resources.toggle_wide_columns();
    assert!(app.root.resources.is_wide_columns());
}

#[test]
fn column_cursor_never_lands_on_unsortable_column() {
    use nuvem::app::state::resources::ComputeInstanceSortField;

    let mut app = app_with_instances();
    for _ in 0..10 {
        app.root.resources.select_next_column();
        assert!(
            ComputeInstanceSortField::from_column(app.root.resources.selected_column()).is_some()
        );
    }
}

#[test]
fn refresh_keeps_open_detail_when_resource_still_present() {
    let mut app = AppState::new();
    load_resource_rows(&mut app, vec![instance("api-a"), instance("api-b")]);
    let context = app.root.current_context().unwrap().clone();
    reduce(&mut app, &Action::Resources(ResourcesAction::OpenSelected)); // detail api-a
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ComputeInstancesLoaded {
            context,
            instances: vec![instance("api-a"), instance("api-b"), instance("api-c")],
        }),
    );
    assert!(app.root.resources.is_showing_detail());
    assert_eq!(app.root.resources.detail_target_name(), Some("api-a"));
}

#[test]
fn refresh_closes_detail_when_resource_deleted() {
    let mut app = AppState::new();
    load_resource_rows(&mut app, vec![instance("api-a"), instance("api-b")]);
    let context = app.root.current_context().unwrap().clone();
    reduce(&mut app, &Action::Resources(ResourcesAction::OpenSelected)); // detail api-a
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ComputeInstancesLoaded {
            context,
            instances: vec![instance("api-b")],
        }),
    );
    assert!(!app.root.resources.is_showing_detail());
}

#[test]
fn refresh_reissues_detail_effect_and_refreshes_open_compute_detail() {
    let mut app = AppState::new();
    load_resource_rows(&mut app, vec![instance("api-a"), instance("api-b")]);
    let context = app.root.current_context().unwrap().clone();
    reduce(&mut app, &Action::Resources(ResourcesAction::OpenSelected)); // detail api-a
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ComputeInstanceDetailLoaded {
            context: context.clone(),
            detail: detail("api-a"),
        }),
    );
    assert!(app.root.resources.is_showing_detail());

    let effects = reduce(&mut app, &Action::Resources(ResourcesAction::Refresh));
    assert_eq!(
        effects,
        vec![
            Effect::RefreshComputeInstances {
                context: context.clone(),
                filter: None,
            },
            Effect::LoadComputeInstanceDetail {
                context: context.clone(),
                instance: instance("api-a"),
            },
        ]
    );

    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ComputeInstancesLoaded {
            context: context.clone(),
            instances: vec![instance("api-a"), instance("api-b"), instance("api-c")],
        }),
    );
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ComputeInstanceDetailLoaded {
            context,
            detail: detail("api-a"),
        }),
    );

    assert!(app.root.resources.is_showing_detail());
    assert_eq!(app.root.resources.detail(), Some(&detail("api-a")));
    assert_eq!(app.root.resources.detail_target_name(), Some("api-a"));
}

#[test]
fn refresh_after_deletion_closes_compute_detail_and_drops_stale_result() {
    let mut app = AppState::new();
    load_resource_rows(&mut app, vec![instance("api-a"), instance("api-b")]);
    let context = app.root.current_context().unwrap().clone();
    reduce(&mut app, &Action::Resources(ResourcesAction::OpenSelected)); // detail api-a
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ComputeInstanceDetailLoaded {
            context: context.clone(),
            detail: detail("api-a"),
        }),
    );

    reduce(&mut app, &Action::Resources(ResourcesAction::Refresh));

    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ComputeInstancesLoaded {
            context: context.clone(),
            instances: vec![instance("api-b")],
        }),
    );
    assert!(!app.root.resources.is_showing_detail());
    assert!(app.root.resources.detail_target().is_none());

    // A late describe result for the removed target must not reopen the detail.
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ComputeInstanceDetailLoaded {
            context,
            detail: detail("api-a"),
        }),
    );
    assert!(!app.root.resources.is_showing_detail());
    assert!(app.root.resources.detail().is_none());
}

fn open_gke_detail(app: &mut AppState, clusters: Vec<GkeCluster>) -> GcloudContext {
    reduce(
        app,
        &Action::EffectResult(EffectResult::ConfigurationsLoaded {
            configurations: vec![config("dev", true, "dev-project")],
        }),
    );
    let context = app.root.current_context().unwrap().clone();
    reduce(
        app,
        &Action::EffectResult(EffectResult::GkeClustersLoaded {
            context: context.clone(),
            clusters,
        }),
    );
    reduce(
        app,
        &Action::Resources(ResourcesAction::SelectService(
            ResourceServiceKind::GkeClusters,
        )),
    );
    reduce(app, &Action::Resources(ResourcesAction::OpenSelected));
    context
}

#[test]
fn refresh_reissues_detail_effect_and_refreshes_open_gke_detail() {
    let mut app = AppState::new();
    let context = open_gke_detail(
        &mut app,
        vec![
            gke_cluster("platform-dev", "europe-west1"),
            gke_cluster("private-prod", "europe-west1-b"),
        ],
    );
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::GkeClusterDetailLoaded {
            context: context.clone(),
            detail: gke_detail("platform-dev", "europe-west1"),
        }),
    );
    assert!(app.root.resources.is_showing_detail());

    let effects = reduce(&mut app, &Action::Resources(ResourcesAction::Refresh));
    assert_eq!(
        effects,
        vec![
            Effect::RefreshGkeClusters {
                context: context.clone(),
                filter: None,
            },
            Effect::LoadGkeClusterDetail {
                context: context.clone(),
                cluster: gke_cluster("platform-dev", "europe-west1"),
            },
        ]
    );

    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::GkeClustersLoaded {
            context: context.clone(),
            clusters: vec![
                gke_cluster("platform-dev", "europe-west1"),
                gke_cluster("private-prod", "europe-west1-b"),
            ],
        }),
    );
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::GkeClusterDetailLoaded {
            context,
            detail: gke_detail("platform-dev", "europe-west1"),
        }),
    );

    assert!(app.root.resources.is_showing_detail());
    assert_eq!(
        app.root.resources.gke_detail(),
        Some(&gke_detail("platform-dev", "europe-west1"))
    );
    assert_eq!(
        app.root.resources.gke_detail_target_name(),
        Some("platform-dev")
    );
}

#[test]
fn refresh_after_deletion_closes_gke_detail_and_drops_stale_result() {
    let mut app = AppState::new();
    let context = open_gke_detail(
        &mut app,
        vec![
            gke_cluster("platform-dev", "europe-west1"),
            gke_cluster("private-prod", "europe-west1-b"),
        ],
    );
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::GkeClusterDetailLoaded {
            context: context.clone(),
            detail: gke_detail("platform-dev", "europe-west1"),
        }),
    );

    reduce(&mut app, &Action::Resources(ResourcesAction::Refresh));

    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::GkeClustersLoaded {
            context: context.clone(),
            clusters: vec![gke_cluster("private-prod", "europe-west1-b")],
        }),
    );
    assert!(!app.root.resources.is_showing_detail());
    assert!(app.root.resources.gke_detail_target().is_none());

    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::GkeClusterDetailLoaded {
            context,
            detail: gke_detail("platform-dev", "europe-west1"),
        }),
    );
    assert!(!app.root.resources.is_showing_detail());
    assert!(app.root.resources.gke_detail().is_none());
}

fn storage_bucket(name: &str) -> StorageBucket {
    StorageBucket {
        name: name.to_string(),
        location: "EU".to_string(),
        location_type: Some("multi-region".to_string()),
        storage_class: "STANDARD".to_string(),
        versioning_enabled: false,
        uniform_bucket_level_access: true,
        public_access_prevention: Some("enforced".to_string()),
        time_created: None,
        updated: Some("2026-01-01T00:00:00Z".to_string()),
    }
}

fn storage_prefix(name: &str) -> StoragePrefix {
    StoragePrefix {
        name: name.to_string(),
    }
}

fn storage_object(key: &str) -> StorageObject {
    StorageObject {
        key: key.to_string(),
        size_bytes: Some(12),
        storage_class: Some("STANDARD".to_string()),
        generation: None,
        updated: Some("2026-01-01T00:00:00Z".to_string()),
        content_type: Some("text/plain".to_string()),
        kms_encrypted: false,
        temporary_hold: false,
        event_based_hold: false,
        has_retention: false,
    }
}

#[test]
fn cloud_storage_select_load_buckets_open_bucket_then_load_object_page() {
    let mut app = AppState::new();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ConfigurationsLoaded {
            configurations: vec![config("dev", true, "dev-project")],
        }),
    );
    let context = app.root.current_context().unwrap().clone();

    // Selecting the Cloud Storage service switches the active service.
    reduce(
        &mut app,
        &Action::Resources(ResourcesAction::SelectService(
            ResourceServiceKind::CloudStorage,
        )),
    );
    assert_eq!(
        app.root.resources.active_service(),
        ResourceServiceKind::CloudStorage
    );

    // Buckets loaded -> visible bucket rows, first (alphabetical) selected.
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::StorageBucketsLoaded {
            context: context.clone(),
            buckets: vec![
                storage_bucket("logs-bucket"),
                storage_bucket("assets-bucket"),
            ],
        }),
    );
    let storage = app.root.resources.storage();
    assert_eq!(storage.view(), StorageView::Buckets);
    assert_eq!(storage.visible_buckets().len(), 2);
    assert_eq!(
        storage.selected_bucket().map(|b| b.name.as_str()),
        Some("assets-bucket")
    );

    // OpenSelected on a bucket returns a first-page object load for it.
    let effects = reduce(&mut app, &Action::Resources(ResourcesAction::OpenSelected));
    assert_eq!(
        effects,
        vec![Effect::LoadStorageObjectPage {
            context: context.clone(),
            bucket: "assets-bucket".to_string(),
            prefix: String::new(),
            page_token: None,
            page_size: 100,
        }]
    );
    assert_eq!(
        app.root.resources.storage().view(),
        StorageView::ObjectBrowser
    );

    // Object page loaded -> browser rows (folders first, then objects) visible.
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::StorageObjectPageLoaded {
            context,
            bucket: "assets-bucket".to_string(),
            prefix: String::new(),
            append: false,
            page: StorageObjectPage {
                prefixes: vec![StoragePrefix {
                    name: "images/".to_string(),
                }],
                objects: vec![StorageObject {
                    key: "readme.txt".to_string(),
                    size_bytes: Some(12),
                    storage_class: Some("STANDARD".to_string()),
                    generation: None,
                    updated: Some("2026-01-01T00:00:00Z".to_string()),
                    content_type: Some("text/plain".to_string()),
                    kms_encrypted: false,
                    temporary_hold: false,
                    event_based_hold: false,
                    has_retention: false,
                }],
                next_page_token: None,
            },
        }),
    );
    let storage = app.root.resources.storage();
    assert_eq!(storage.view(), StorageView::ObjectBrowser);
    assert_eq!(storage.visible_browser_rows().len(), 2);
}

/// Seeds the object browser with a first page that has a next-page token,
/// moves the selection down, then walks through `NextPage` -> append.
/// Verifies: the effect carries the current token, rows grow, the selection
/// index survives the append, and the projection's pagination fields track
/// `has_more`/page index correctly once the listing is exhausted.
#[test]
fn cloud_storage_next_page_appends_and_preserves_selection() {
    let mut app = AppState::new();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ConfigurationsLoaded {
            configurations: vec![config("dev", true, "dev-project")],
        }),
    );
    let context = app.root.current_context().unwrap().clone();

    reduce(
        &mut app,
        &Action::Resources(ResourcesAction::SelectService(
            ResourceServiceKind::CloudStorage,
        )),
    );
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::StorageBucketsLoaded {
            context: context.clone(),
            buckets: vec![storage_bucket("assets-bucket")],
        }),
    );
    reduce(&mut app, &Action::Resources(ResourcesAction::OpenSelected));

    // Page 1: three prefixes + three objects (rows 0..=5), with a
    // continuation token indicating more pages are available.
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::StorageObjectPageLoaded {
            context: context.clone(),
            bucket: "assets-bucket".to_string(),
            prefix: String::new(),
            append: false,
            page: StorageObjectPage {
                prefixes: vec![
                    storage_prefix("images/"),
                    storage_prefix("logs/"),
                    storage_prefix("tmp/"),
                ],
                objects: vec![
                    storage_object("a.txt"),
                    storage_object("b.txt"),
                    storage_object("c.txt"),
                ],
                next_page_token: Some("TOKEN2".to_string()),
            },
        }),
    );
    assert_eq!(app.root.resources.storage().visible_browser_rows().len(), 6);

    // Move the selection to row 5 (the last row of page 1).
    for _ in 0..5 {
        reduce(&mut app, &Action::Resources(ResourcesAction::SelectNext));
    }
    assert_eq!(
        app.root
            .resources
            .storage()
            .selected_visible_index_for_visible_rows(6),
        Some(5)
    );

    let projection = project_resources(&app.root.resources);
    assert!(projection.has_more, "page 1 carried a continuation token");
    assert_eq!(projection.page_index, 0, "page 1 is the first page");

    // NextPage must request the current continuation token and be a no-op
    // if none were available (that case is covered separately below).
    let effects = reduce(&mut app, &Action::Resources(ResourcesAction::NextPage));
    assert_eq!(
        effects,
        vec![Effect::LoadStorageObjectPage {
            context: context.clone(),
            bucket: "assets-bucket".to_string(),
            prefix: String::new(),
            page_token: Some("TOKEN2".to_string()),
            page_size: 100,
        }]
    );

    let projection = project_resources(&app.root.resources);
    assert!(
        projection.has_more,
        "next_page_token from page 1 must still be reflected until the append lands"
    );

    // Page 2: appended, no further continuation token (end of listing).
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::StorageObjectPageLoaded {
            context,
            bucket: "assets-bucket".to_string(),
            prefix: String::new(),
            append: true,
            page: StorageObjectPage {
                prefixes: Vec::new(),
                objects: vec![storage_object("d.txt"), storage_object("e.txt")],
                next_page_token: None,
            },
        }),
    );

    let storage = app.root.resources.storage();
    assert_eq!(storage.visible_browser_rows().len(), 8, "rows grew");
    assert_eq!(
        storage.selected_visible_index_for_visible_rows(8),
        Some(5),
        "selection index survives the append"
    );

    let projection = project_resources(&app.root.resources);
    assert!(
        !projection.has_more,
        "listing is exhausted after page 2 with no token"
    );
    assert_eq!(projection.page_index, 1, "second page increments the index");
    match projection.list {
        ResourceListProjection::Storage(rows) => assert_eq!(rows.len(), 8),
        other => panic!("expected a storage projection, got {other:?}"),
    }
}

/// `NextPage` must be a no-op (no effect) when the browser has no
/// continuation token, whether because a page hasn't loaded yet or because
/// the listing is already exhausted.
#[test]
fn cloud_storage_next_page_without_token_is_a_no_op() {
    let mut app = AppState::new();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ConfigurationsLoaded {
            configurations: vec![config("dev", true, "dev-project")],
        }),
    );
    let context = app.root.current_context().unwrap().clone();

    reduce(
        &mut app,
        &Action::Resources(ResourcesAction::SelectService(
            ResourceServiceKind::CloudStorage,
        )),
    );
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::StorageBucketsLoaded {
            context: context.clone(),
            buckets: vec![storage_bucket("assets-bucket")],
        }),
    );
    reduce(&mut app, &Action::Resources(ResourcesAction::OpenSelected));
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::StorageObjectPageLoaded {
            context,
            bucket: "assets-bucket".to_string(),
            prefix: String::new(),
            append: false,
            page: StorageObjectPage {
                prefixes: Vec::new(),
                objects: vec![storage_object("a.txt")],
                next_page_token: None,
            },
        }),
    );

    let effects = reduce(&mut app, &Action::Resources(ResourcesAction::NextPage));
    assert_eq!(effects, Vec::new());
    assert!(!project_resources(&app.root.resources).has_more);
}

/// `ascend()` from the object-browser root pops back to the bucket list and
/// clears `browser.next_page_token`, so no stale continuation token from the
/// browser session can linger once the view is `StorageView::Buckets`.
/// `NextPage` must also remain a no-op from that view as a defense-in-depth
/// guard, independent of whatever the token happens to hold.
#[test]
fn cloud_storage_next_page_is_a_no_op_from_the_bucket_list_view() {
    let mut app = AppState::new();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ConfigurationsLoaded {
            configurations: vec![config("dev", true, "dev-project")],
        }),
    );
    let context = app.root.current_context().unwrap().clone();

    reduce(
        &mut app,
        &Action::Resources(ResourcesAction::SelectService(
            ResourceServiceKind::CloudStorage,
        )),
    );
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::StorageBucketsLoaded {
            context: context.clone(),
            buckets: vec![storage_bucket("assets-bucket")],
        }),
    );
    reduce(&mut app, &Action::Resources(ResourcesAction::OpenSelected));
    assert_eq!(
        app.root.resources.storage().view(),
        StorageView::ObjectBrowser
    );

    // Load a page with a continuation token, leaving `browser.next_page_token`
    // populated once we leave this view.
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::StorageObjectPageLoaded {
            context,
            bucket: "assets-bucket".to_string(),
            prefix: String::new(),
            append: false,
            page: StorageObjectPage {
                prefixes: Vec::new(),
                objects: vec![storage_object("a.txt")],
                next_page_token: Some("TOKEN2".to_string()),
            },
        }),
    );
    assert!(project_resources(&app.root.resources).has_more);

    // Ascend from the object-browser root back to the bucket list. The stale
    // continuation token from the browser session above must be cleared now
    // that the view is `Buckets`.
    let effects = reduce(&mut app, &Action::Resources(ResourcesAction::AscendPrefix));
    assert_eq!(effects, Vec::new());
    assert_eq!(app.root.resources.storage().view(), StorageView::Buckets);
    assert!(
        !app.root.resources.storage().browser_has_more(),
        "ascending to the bucket list must clear the stale continuation token"
    );

    // Even with the token cleared, `NextPage` must still be a no-op from the
    // bucket-list view (defense-in-depth, independent of the token itself).
    let effects = reduce(&mut app, &Action::Resources(ResourcesAction::NextPage));
    assert_eq!(
        effects,
        Vec::new(),
        "NextPage must not fire LoadStorageObjectPage from the bucket-list view"
    );
}

/// Nested prefix drill-down: descending two levels tracks `prefix_path` and
/// requests the right page each time, ascending one level pops back and
/// reloads that level (resetting the selection for the freshly loaded page),
/// and ascending again from the browser root returns to `StorageView::Buckets`
/// without firing an object-page effect.
#[test]
fn cloud_storage_nested_prefix_descend_and_ascend_tracks_prefix_path() {
    let mut app = AppState::new();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ConfigurationsLoaded {
            configurations: vec![config("dev", true, "dev-project")],
        }),
    );
    let context = app.root.current_context().unwrap().clone();

    reduce(
        &mut app,
        &Action::Resources(ResourcesAction::SelectService(
            ResourceServiceKind::CloudStorage,
        )),
    );
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::StorageBucketsLoaded {
            context: context.clone(),
            buckets: vec![storage_bucket("alpha-bucket")],
        }),
    );
    reduce(&mut app, &Action::Resources(ResourcesAction::OpenSelected));

    // Root page: one common prefix "photos/" plus one object.
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::StorageObjectPageLoaded {
            context: context.clone(),
            bucket: "alpha-bucket".to_string(),
            prefix: String::new(),
            append: false,
            page: StorageObjectPage {
                prefixes: vec![storage_prefix("photos/")],
                objects: vec![storage_object("root.txt")],
                next_page_token: None,
            },
        }),
    );
    assert!(
        app.root
            .resources
            .storage()
            .browser_prefix_path()
            .is_empty()
    );
    assert_eq!(app.root.resources.storage().current_prefix(), "");

    // Descend into "photos/" (prefixes sort first, so row 0 is selected).
    let effects = reduce(&mut app, &Action::Resources(ResourcesAction::OpenSelected));
    assert_eq!(
        effects,
        vec![Effect::LoadStorageObjectPage {
            context: context.clone(),
            bucket: "alpha-bucket".to_string(),
            prefix: "photos/".to_string(),
            page_token: None,
            page_size: 100,
        }]
    );
    assert_eq!(
        app.root.resources.storage().browser_prefix_path(),
        ["photos/".to_string()]
    );
    assert_eq!(app.root.resources.storage().current_prefix(), "photos/");
    assert_eq!(
        app.root.resources.storage().view(),
        StorageView::ObjectBrowser
    );

    // "photos/" page: one nested prefix "photos/2024/".
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::StorageObjectPageLoaded {
            context: context.clone(),
            bucket: "alpha-bucket".to_string(),
            prefix: "photos/".to_string(),
            append: false,
            page: StorageObjectPage {
                prefixes: vec![storage_prefix("photos/2024/")],
                objects: Vec::new(),
                next_page_token: None,
            },
        }),
    );

    // Descend again into "photos/2024/".
    let effects = reduce(&mut app, &Action::Resources(ResourcesAction::OpenSelected));
    assert_eq!(
        effects,
        vec![Effect::LoadStorageObjectPage {
            context: context.clone(),
            bucket: "alpha-bucket".to_string(),
            prefix: "photos/2024/".to_string(),
            page_token: None,
            page_size: 100,
        }]
    );
    assert_eq!(
        app.root.resources.storage().browser_prefix_path(),
        ["photos/".to_string(), "2024/".to_string()]
    );
    assert_eq!(
        app.root.resources.storage().current_prefix(),
        "photos/2024/"
    );

    // "photos/2024/" page: two objects; move selection off row 0 before
    // ascending so we can confirm the reload resets it.
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::StorageObjectPageLoaded {
            context: context.clone(),
            bucket: "alpha-bucket".to_string(),
            prefix: "photos/2024/".to_string(),
            append: false,
            page: StorageObjectPage {
                prefixes: Vec::new(),
                objects: vec![storage_object("a.jpg"), storage_object("b.jpg")],
                next_page_token: None,
            },
        }),
    );
    reduce(&mut app, &Action::Resources(ResourcesAction::SelectNext));
    assert_eq!(
        app.root
            .resources
            .storage()
            .selected_visible_index_for_visible_rows(2),
        Some(1)
    );

    // Ascend one level: back to "photos/", with a reload effect for that
    // level.
    let effects = reduce(&mut app, &Action::Resources(ResourcesAction::AscendPrefix));
    assert_eq!(
        effects,
        vec![Effect::LoadStorageObjectPage {
            context: context.clone(),
            bucket: "alpha-bucket".to_string(),
            prefix: "photos/".to_string(),
            page_token: None,
            page_size: 100,
        }]
    );
    assert_eq!(
        app.root.resources.storage().browser_prefix_path(),
        ["photos/".to_string()]
    );
    assert_eq!(app.root.resources.storage().current_prefix(), "photos/");
    assert_eq!(
        app.root.resources.storage().view(),
        StorageView::ObjectBrowser
    );

    // The reloaded "photos/" level starts with the selection at row 0.
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::StorageObjectPageLoaded {
            context: context.clone(),
            bucket: "alpha-bucket".to_string(),
            prefix: "photos/".to_string(),
            append: false,
            page: StorageObjectPage {
                prefixes: vec![storage_prefix("photos/2024/")],
                objects: Vec::new(),
                next_page_token: None,
            },
        }),
    );
    assert_eq!(
        app.root
            .resources
            .storage()
            .selected_visible_index_for_visible_rows(1),
        Some(0)
    );

    // Ascend again: pops back to the bucket root, with a reload effect for
    // the empty prefix.
    let effects = reduce(&mut app, &Action::Resources(ResourcesAction::AscendPrefix));
    assert_eq!(
        effects,
        vec![Effect::LoadStorageObjectPage {
            context: context.clone(),
            bucket: "alpha-bucket".to_string(),
            prefix: String::new(),
            page_token: None,
            page_size: 100,
        }]
    );
    assert!(
        app.root
            .resources
            .storage()
            .browser_prefix_path()
            .is_empty()
    );
    assert_eq!(
        app.root.resources.storage().view(),
        StorageView::ObjectBrowser
    );

    // Load the root page again, then ascend once more: the browser is now at
    // the root prefix, so ascending returns to the bucket list and fires no
    // object-page effect.
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::StorageObjectPageLoaded {
            context,
            bucket: "alpha-bucket".to_string(),
            prefix: String::new(),
            append: false,
            page: StorageObjectPage {
                prefixes: vec![storage_prefix("photos/")],
                objects: Vec::new(),
                next_page_token: None,
            },
        }),
    );
    let effects = reduce(&mut app, &Action::Resources(ResourcesAction::AscendPrefix));
    assert_eq!(effects, Vec::new(), "ascending at the root fires no effect");
    assert_eq!(app.root.resources.storage().view(), StorageView::Buckets);
    assert!(
        app.root
            .resources
            .storage()
            .browser_prefix_path()
            .is_empty()
    );
}

#[test]
fn is_safe_remote_prefix_matches_spec_cases() {
    assert!(is_safe_remote_prefix("photos"));
    assert!(!is_safe_remote_prefix(""));
    assert!(!is_safe_remote_prefix("a*b"));
    assert!(!is_safe_remote_prefix("a?b"));
    assert!(!is_safe_remote_prefix("a[b"));
    assert!(!is_safe_remote_prefix("/abs"));
    assert!(!is_safe_remote_prefix("a\u{0007}b"));
    assert!(is_safe_remote_prefix("café"));
}

/// Bucket-list `/` filtering is always local: even a substring that would
/// otherwise be a syntactically safe remote prefix never fires an effect.
#[test]
fn cloud_storage_bucket_list_filter_is_local_only_with_no_effect() {
    let mut app = AppState::new();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ConfigurationsLoaded {
            configurations: vec![config("dev", true, "dev-project")],
        }),
    );
    let context = app.root.current_context().unwrap().clone();
    reduce(
        &mut app,
        &Action::Resources(ResourcesAction::SelectService(
            ResourceServiceKind::CloudStorage,
        )),
    );
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::StorageBucketsLoaded {
            context,
            buckets: vec![
                storage_bucket("alpha-bucket"),
                storage_bucket("beta-bucket"),
            ],
        }),
    );

    reduce(&mut app, &Action::Resources(ResourcesAction::StartFilter));
    for ch in "alpha".chars() {
        reduce(
            &mut app,
            &Action::Resources(ResourcesAction::AppendFilterChar(ch)),
        );
    }
    let effects = reduce(&mut app, &Action::Resources(ResourcesAction::ApplyFilter));

    assert_eq!(
        effects,
        Vec::new(),
        "bucket-list filtering must never emit a remote effect, even for an otherwise safe prefix"
    );
    assert_eq!(app.root.resources.filter_scope(), FilterScope::Local);
    let storage = app.root.resources.storage();
    assert_eq!(storage.visible_buckets().len(), 1);
    assert_eq!(
        storage.selected_bucket().map(|b| b.name.as_str()),
        Some("alpha-bucket")
    );
}

/// Cloud Storage keeps the plain-substring filter: the prefix grammar must not
/// leak into it. The object browser reloads on the literal `filter_applied` as
/// a remote prefix (see `is_safe_remote_prefix`), which admits `-f api` and
/// `!api` verbatim, so a grammar-aware client predicate here would drop rows
/// the server had returned.
#[test]
fn cloud_storage_bucket_list_filter_ignores_the_prefix_grammar() {
    for (input, expected) in [
        ("api", vec!["api-bucket"]),
        ("-f api", Vec::new()),
        ("!api", Vec::new()),
    ] {
        let mut app = AppState::new();
        reduce(
            &mut app,
            &Action::EffectResult(EffectResult::ConfigurationsLoaded {
                configurations: vec![config("dev", true, "dev-project")],
            }),
        );
        let context = app.root.current_context().unwrap().clone();
        reduce(
            &mut app,
            &Action::Resources(ResourcesAction::SelectService(
                ResourceServiceKind::CloudStorage,
            )),
        );
        reduce(
            &mut app,
            &Action::EffectResult(EffectResult::StorageBucketsLoaded {
                context,
                buckets: vec![storage_bucket("api-bucket"), storage_bucket("web-bucket")],
            }),
        );

        apply_filter_input(&mut app, input);

        let visible: Vec<&str> = app
            .root
            .resources
            .storage()
            .visible_buckets()
            .into_iter()
            .map(|bucket| bucket.name.as_str())
            .collect();
        assert_eq!(visible, expected, "input {input}");
        assert_eq!(
            app.root.resources.filter_mode(),
            FilterMode::Regex,
            "Cloud Storage has no mode chip, so it always reports the default"
        );
        assert_eq!(app.root.resources.filter_error(), None, "input {input}");
    }
}

/// Opens `alpha-bucket`'s object browser at the root and loads a first page
/// with one folder ("photos/"), one plain object ("readme.txt"), and one
/// Unicode-keyed object ("café/maña.txt") — enough fixture variety to
/// exercise local substring matching (including Unicode) and the remote
/// safe-prefix reload path.
fn app_with_object_browser_open() -> (AppState, GcloudContext) {
    let mut app = AppState::new();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ConfigurationsLoaded {
            configurations: vec![config("dev", true, "dev-project")],
        }),
    );
    let context = app.root.current_context().unwrap().clone();
    reduce(
        &mut app,
        &Action::Resources(ResourcesAction::SelectService(
            ResourceServiceKind::CloudStorage,
        )),
    );
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::StorageBucketsLoaded {
            context: context.clone(),
            buckets: vec![storage_bucket("alpha-bucket")],
        }),
    );
    reduce(&mut app, &Action::Resources(ResourcesAction::OpenSelected));
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::StorageObjectPageLoaded {
            context: context.clone(),
            bucket: "alpha-bucket".to_string(),
            prefix: String::new(),
            append: false,
            page: StorageObjectPage {
                prefixes: vec![storage_prefix("photos/")],
                objects: vec![
                    storage_object("readme.txt"),
                    storage_object("café/maña.txt"),
                ],
                next_page_token: None,
            },
        }),
    );
    (app, context)
}

/// Applying a safe-prefix filter ("photos") in the object browser narrows
/// the already-loaded rows locally *and* issues a fresh first-page reload
/// scoped to `current_prefix + draft`, marking the filter scope remote. The
/// reload response is accepted because `set_object_page` matches against the
/// same effective (filtered) prefix, not the bare current prefix.
#[test]
fn cloud_storage_object_browser_safe_prefix_filter_triggers_remote_reload() {
    let (mut app, context) = app_with_object_browser_open();

    reduce(&mut app, &Action::Resources(ResourcesAction::StartFilter));
    for ch in "photos".chars() {
        reduce(
            &mut app,
            &Action::Resources(ResourcesAction::AppendFilterChar(ch)),
        );
    }
    let effects = reduce(&mut app, &Action::Resources(ResourcesAction::ApplyFilter));

    assert_eq!(
        effects,
        vec![Effect::LoadStorageObjectPage {
            context: context.clone(),
            bucket: "alpha-bucket".to_string(),
            prefix: "photos".to_string(),
            page_token: None,
            page_size: 100,
        }]
    );
    assert_eq!(app.root.resources.filter_scope(), FilterScope::Remote);
    // The local filter narrows the already-loaded rows immediately, ahead of
    // the reload response: only the "photos/" folder matches.
    assert_eq!(app.root.resources.storage().visible_browser_row_count(), 1);

    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::StorageObjectPageLoaded {
            context,
            bucket: "alpha-bucket".to_string(),
            prefix: "photos".to_string(),
            append: false,
            page: StorageObjectPage {
                prefixes: Vec::new(),
                objects: vec![
                    storage_object("photos/img1.png"),
                    storage_object("photos/img2.png"),
                ],
                next_page_token: None,
            },
        }),
    );
    assert_eq!(app.root.resources.storage().visible_browser_row_count(), 2);
}

/// A filter containing glob-ish characters is never a safe remote prefix: it
/// only narrows the already-loaded rows locally, with no reload effect.
#[test]
fn cloud_storage_object_browser_unsafe_glob_filter_is_local_only_with_no_effect() {
    let (mut app, _context) = app_with_object_browser_open();

    reduce(&mut app, &Action::Resources(ResourcesAction::StartFilter));
    for ch in "a*b".chars() {
        reduce(
            &mut app,
            &Action::Resources(ResourcesAction::AppendFilterChar(ch)),
        );
    }
    let effects = reduce(&mut app, &Action::Resources(ResourcesAction::ApplyFilter));

    assert_eq!(
        effects,
        Vec::new(),
        "a glob-ish filter must stay local only"
    );
    assert_eq!(app.root.resources.filter_scope(), FilterScope::Local);
    assert_eq!(app.root.resources.storage().visible_browser_row_count(), 0);
}

/// The local filter is Unicode-aware: it matches a Unicode substring inside
/// an object key case-insensitively.
///
/// Deviation from the task brief's illustrative example: the brief describes
/// applying `"maña"` as producing "no effect" (implying local-only), but per
/// `is_safe_remote_prefix` — whose own required unit-test cases treat
/// `"café"` as safe — a plain word containing no glob characters, no leading
/// slash, and no control characters (Unicode letters included) *is* a
/// syntactically safe remote prefix. So applying `"maña"` also fires the
/// reload effect; this test asserts the actual (self-consistent) contract
/// instead of the brief's example.
#[test]
fn cloud_storage_object_browser_filter_matches_unicode_key_and_is_a_safe_remote_prefix() {
    let (mut app, context) = app_with_object_browser_open();

    reduce(&mut app, &Action::Resources(ResourcesAction::StartFilter));
    for ch in "maña".chars() {
        reduce(
            &mut app,
            &Action::Resources(ResourcesAction::AppendFilterChar(ch)),
        );
    }
    let effects = reduce(&mut app, &Action::Resources(ResourcesAction::ApplyFilter));

    assert_eq!(
        effects,
        vec![Effect::LoadStorageObjectPage {
            context,
            bucket: "alpha-bucket".to_string(),
            prefix: "maña".to_string(),
            page_token: None,
            page_size: 100,
        }]
    );
    assert_eq!(app.root.resources.filter_scope(), FilterScope::Remote);
    // Only "café/maña.txt" contains "maña"; "readme.txt" and "photos/" don't.
    assert_eq!(app.root.resources.storage().visible_browser_row_count(), 1);
}

/// Sanitization guardrail: the local object-browser filter must
/// match the SANITIZED display name (the same text the projection layer
/// renders via `sanitize_display_text`), not the raw, possibly-hostile key.
/// Seeds one object whose key contains a real control character (BEL,
/// `\u{0007}`) and one object whose key already contains the literal
/// replacement character U+FFFD with no actual control byte. Both render
/// identically once sanitized, so a filter typed as the rendered form must
/// match both -- proving the filter is evaluated against what's on screen,
/// not the raw bytes underneath.
#[test]
fn cloud_storage_object_browser_local_filter_matches_sanitized_control_char_display_name() {
    let mut app = AppState::new();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ConfigurationsLoaded {
            configurations: vec![config("dev", true, "dev-project")],
        }),
    );
    let context = app.root.current_context().unwrap().clone();
    reduce(
        &mut app,
        &Action::Resources(ResourcesAction::SelectService(
            ResourceServiceKind::CloudStorage,
        )),
    );
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::StorageBucketsLoaded {
            context: context.clone(),
            buckets: vec![storage_bucket("alpha-bucket")],
        }),
    );
    reduce(&mut app, &Action::Resources(ResourcesAction::OpenSelected));
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::StorageObjectPageLoaded {
            context,
            bucket: "alpha-bucket".to_string(),
            prefix: String::new(),
            append: false,
            page: StorageObjectPage {
                prefixes: Vec::new(),
                objects: vec![
                    // Raw control character (BEL); sanitizes to
                    // "batch\u{FFFD}logs.txt".
                    storage_object("batch\u{0007}logs.txt"),
                    // Literal replacement character already in the key (no
                    // real control byte); sanitizing is idempotent, so this
                    // renders identically to the object above.
                    storage_object("batch\u{FFFD}logs.txt"),
                    storage_object("readme.txt"),
                ],
                next_page_token: None,
            },
        }),
    );

    // Typing the *rendered* (sanitized) form as the filter must match BOTH
    // hostile-key objects, since both display identically once sanitized.
    // (U+FFFD is not a control character, so `AppendFilterChar` accepts it.)
    reduce(&mut app, &Action::Resources(ResourcesAction::StartFilter));
    for ch in "batch\u{FFFD}logs".chars() {
        reduce(
            &mut app,
            &Action::Resources(ResourcesAction::AppendFilterChar(ch)),
        );
    }
    reduce(&mut app, &Action::Resources(ResourcesAction::ApplyFilter));

    assert_eq!(
        app.root.resources.storage().visible_browser_row_count(),
        2,
        "filtering by the sanitized/rendered form must match both hostile-key objects"
    );
}

/// Companion to the sanitized-match test above: `append_filter_char` rejects
/// control characters outright, so a user (or a paste buffer) attempting to
/// type the raw sequence "batch<BEL>logs" ends up with the BEL silently
/// dropped and the applied filter left as the concatenation "batchlogs" --
/// with no replacement character in it. That must NOT spuriously match a key
/// whose sanitized display has a replacement character sitting between
/// "batch" and "logs", proving the raw/dropped form can't accidentally
/// smuggle a match the sanitized rendering wouldn't produce.
#[test]
fn cloud_storage_object_browser_local_filter_raw_control_char_sequence_does_not_spuriously_match() {
    let mut app = AppState::new();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ConfigurationsLoaded {
            configurations: vec![config("dev", true, "dev-project")],
        }),
    );
    let context = app.root.current_context().unwrap().clone();
    reduce(
        &mut app,
        &Action::Resources(ResourcesAction::SelectService(
            ResourceServiceKind::CloudStorage,
        )),
    );
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::StorageBucketsLoaded {
            context: context.clone(),
            buckets: vec![storage_bucket("alpha-bucket")],
        }),
    );
    reduce(&mut app, &Action::Resources(ResourcesAction::OpenSelected));
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::StorageObjectPageLoaded {
            context,
            bucket: "alpha-bucket".to_string(),
            prefix: String::new(),
            append: false,
            page: StorageObjectPage {
                prefixes: Vec::new(),
                objects: vec![
                    storage_object("batch\u{0007}logs.txt"),
                    storage_object("batch\u{FFFD}logs.txt"),
                    storage_object("readme.txt"),
                ],
                next_page_token: None,
            },
        }),
    );

    reduce(&mut app, &Action::Resources(ResourcesAction::StartFilter));
    for ch in "batch\u{0007}logs".chars() {
        reduce(
            &mut app,
            &Action::Resources(ResourcesAction::AppendFilterChar(ch)),
        );
    }
    reduce(&mut app, &Action::Resources(ResourcesAction::ApplyFilter));

    assert_eq!(
        app.root.resources.storage().filter_applied(),
        "batchlogs",
        "the raw control character must be dropped by append_filter_char, not applied verbatim"
    );
    assert_eq!(
        app.root.resources.storage().visible_browser_row_count(),
        0,
        "a filter built from the raw (dropped-control-char) sequence must not spuriously match \
         the sanitized display name"
    );
}

/// While a safe-prefix remote filter is active, `NextPage` must request the
/// next page using the FILTERED effective prefix (`current_prefix() +
/// filter_applied`), not the bare current prefix -- otherwise on-demand
/// pagination would silently fall back to the unfiltered listing once a
/// filter has narrowed the server-side scope.
#[test]
fn cloud_storage_next_page_uses_filtered_effective_prefix_while_remote_filter_active() {
    let (mut app, context) = app_with_object_browser_open();

    reduce(&mut app, &Action::Resources(ResourcesAction::StartFilter));
    for ch in "photos".chars() {
        reduce(
            &mut app,
            &Action::Resources(ResourcesAction::AppendFilterChar(ch)),
        );
    }
    let effects = reduce(&mut app, &Action::Resources(ResourcesAction::ApplyFilter));
    assert_eq!(
        effects,
        vec![Effect::LoadStorageObjectPage {
            context: context.clone(),
            bucket: "alpha-bucket".to_string(),
            prefix: "photos".to_string(),
            page_token: None,
            page_size: 100,
        }]
    );
    assert_eq!(app.root.resources.filter_scope(), FilterScope::Remote);

    // The reload response for the filtered scope carries a continuation
    // token, so `NextPage` has something to request.
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::StorageObjectPageLoaded {
            context: context.clone(),
            bucket: "alpha-bucket".to_string(),
            prefix: "photos".to_string(),
            append: false,
            page: StorageObjectPage {
                prefixes: Vec::new(),
                objects: vec![
                    storage_object("photos/img1.png"),
                    storage_object("photos/img2.png"),
                ],
                next_page_token: Some("PHOTOS_TOKEN".to_string()),
            },
        }),
    );
    assert!(app.root.resources.storage().browser_has_more());

    let effects = reduce(&mut app, &Action::Resources(ResourcesAction::NextPage));
    assert_eq!(
        effects,
        vec![Effect::LoadStorageObjectPage {
            context,
            bucket: "alpha-bucket".to_string(),
            prefix: "photos".to_string(),
            page_token: Some("PHOTOS_TOKEN".to_string()),
            page_size: 100,
        }],
        "NextPage must keep requesting the filtered effective prefix, not the bare current prefix"
    );
}

/// While a safe-prefix remote filter is active, `Refresh` must re-request the
/// FILTERED effective prefix for the object-browser page load, alongside the
/// always-present bucket-list refresh -- otherwise refreshing would silently
/// drop back to the unfiltered listing.
#[test]
fn cloud_storage_refresh_uses_filtered_effective_prefix_while_remote_filter_active() {
    let (mut app, context) = app_with_object_browser_open();

    reduce(&mut app, &Action::Resources(ResourcesAction::StartFilter));
    for ch in "photos".chars() {
        reduce(
            &mut app,
            &Action::Resources(ResourcesAction::AppendFilterChar(ch)),
        );
    }
    reduce(&mut app, &Action::Resources(ResourcesAction::ApplyFilter));
    assert_eq!(app.root.resources.filter_scope(), FilterScope::Remote);

    let effects = reduce(&mut app, &Action::Resources(ResourcesAction::Refresh));
    assert_eq!(
        effects,
        vec![
            Effect::RefreshStorageBuckets {
                context: context.clone(),
            },
            Effect::LoadStorageObjectPage {
                context,
                bucket: "alpha-bucket".to_string(),
                prefix: "photos".to_string(),
                page_token: None,
                page_size: 100,
            },
        ],
        "Refresh must re-request the filtered effective prefix while a remote filter is active"
    );
}

// ---- Task 7: distinct empty/error states ----

/// A bucket-list load failure classified as `PermissionDenied` must surface
/// that kind (not a generic "unknown"/"stale" bucket) on the storage
/// problem, and the projection must expose the same classification plus a
/// human-readable message so the render layer can show a distinct
/// permission-denied message instead of the generic stale/empty state.
#[test]
fn cloud_storage_buckets_load_failed_with_permission_denied_surfaces_permission_problem() {
    let mut app = AppState::new();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ConfigurationsLoaded {
            configurations: vec![config("dev", true, "dev-project")],
        }),
    );
    let context = app.root.current_context().unwrap().clone();
    reduce(
        &mut app,
        &Action::Resources(ResourcesAction::SelectService(
            ResourceServiceKind::CloudStorage,
        )),
    );

    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::StorageBucketsLoadFailed {
            context,
            error: AppError::gcloud(
                "ERROR: (gcloud.storage.buckets.list) PERMISSION_DENIED: The caller does not have permission".to_string(),
            ),
        }),
    );

    let problem = app
        .root
        .resources
        .storage()
        .problem()
        .expect("bucket-list failure must set a storage problem");
    assert_eq!(problem.kind, GcloudErrorKind::PermissionDenied);
    assert!(!app.root.resources.storage().problem_requester_pays());

    let projection = project_resources(&app.root.resources);
    let storage_problem = projection
        .storage_problem
        .expect("projection must surface the classified bucket-list problem");
    assert_eq!(storage_problem.kind, GcloudErrorKind::PermissionDenied);
    assert!(!storage_problem.requester_pays);
    assert!(
        storage_problem
            .message
            .to_lowercase()
            .contains("permission"),
        "expected a permission-denied-style message, got: {}",
        storage_problem.message
    );
}

/// A `StorageBucketsLoadFailed` result must set the active collection's
/// `ResourceLoadPhase` to `Failed`, carrying the same sanitized diagnostic
/// headline surfaced elsewhere (storage problem/status); it must not touch
/// the existing `storage().problem()`/`last_diagnostic` channels.
#[test]
fn storage_load_failure_sets_failed_load_phase_with_diagnostic_headline() {
    let mut app = AppState::new();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ConfigurationsLoaded {
            configurations: vec![config("dev", true, "dev-project")],
        }),
    );
    let context = app.root.current_context().unwrap().clone();
    reduce(
        &mut app,
        &Action::Resources(ResourcesAction::SelectService(
            ResourceServiceKind::CloudStorage,
        )),
    );

    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::StorageBucketsLoadFailed {
            context,
            error: AppError::gcloud(
                "ERROR: (gcloud.storage.buckets.list) PERMISSION_DENIED: The caller does not have permission".to_string(),
            ),
        }),
    );

    match app.root.resources.active_load_phase() {
        ResourceLoadPhase::Failed { message, .. } => {
            assert!(
                message.to_lowercase().contains("permission"),
                "expected a permission-denied-style headline, got: {message}"
            );
        }
        other => panic!("expected Failed load phase, got {other:?}"),
    }
}

/// After a failed storage bucket-list load-phase, a subsequent successful
/// `StorageBucketsLoaded` result must reset the phase back to `Idle`.
#[test]
fn storage_successful_load_after_failure_resets_load_phase_to_idle() {
    let mut app = AppState::new();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ConfigurationsLoaded {
            configurations: vec![config("dev", true, "dev-project")],
        }),
    );
    let context = app.root.current_context().unwrap().clone();
    reduce(
        &mut app,
        &Action::Resources(ResourcesAction::SelectService(
            ResourceServiceKind::CloudStorage,
        )),
    );
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::StorageBucketsLoadFailed {
            context: context.clone(),
            error: AppError::gcloud("bucket list failed"),
        }),
    );
    assert_ne!(
        app.root.resources.active_load_phase(),
        &ResourceLoadPhase::Idle
    );

    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::StorageBucketsLoaded {
            context,
            buckets: vec![storage_bucket("assets-bucket")],
        }),
    );

    assert_eq!(
        app.root.resources.active_load_phase(),
        &ResourceLoadPhase::Idle
    );
}

/// An object-page (first page) load failure whose stderr indicates a
/// requester-pays / `userProjectMissing` condition must be flagged
/// distinctly from a generic permission-denied failure, via a
/// storage-local `requester_pays` flag rather than a new `GcloudErrorKind`
/// variant.
#[test]
fn cloud_storage_object_page_load_failed_with_requester_pays_stderr_sets_distinct_flag() {
    let mut app = AppState::new();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ConfigurationsLoaded {
            configurations: vec![config("dev", true, "dev-project")],
        }),
    );
    let context = app.root.current_context().unwrap().clone();
    reduce(
        &mut app,
        &Action::Resources(ResourcesAction::SelectService(
            ResourceServiceKind::CloudStorage,
        )),
    );
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::StorageBucketsLoaded {
            context: context.clone(),
            buckets: vec![storage_bucket("alpha-bucket")],
        }),
    );
    reduce(&mut app, &Action::Resources(ResourcesAction::OpenSelected));
    assert!(app.root.resources.storage().browser_loading());

    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::StorageObjectPageLoadFailed {
            context,
            bucket: "alpha-bucket".to_string(),
            prefix: String::new(),
            append: false,
            error: AppError::gcloud(
                "userProjectMissing: Bucket is a requester pays bucket but no user project was provided.".to_string(),
            ),
        }),
    );

    let storage = app.root.resources.storage();
    assert!(!storage.browser_loading());
    assert!(
        storage.browser_requester_pays(),
        "requester-pays stderr must set the distinct requester-pays flag"
    );
    let problem = storage
        .browser_problem()
        .expect("first-page failure must set a browser problem");
    assert_eq!(problem.kind, GcloudErrorKind::Unknown);

    let projection = project_resources(&app.root.resources);
    let storage_problem = projection
        .storage_problem
        .expect("projection must surface the browser problem");
    assert!(storage_problem.requester_pays);
}

/// A page-append (`NextPage`) failure must retain the already-loaded rows
/// (a non-destructive banner, not a full-list error state) and set
/// `partial_page_error`, leaving the (unrelated) `browser_problem` alone.
#[test]
fn cloud_storage_object_page_append_failure_retains_rows_and_sets_partial_page_error() {
    let (mut app, context) = app_with_object_browser_open();
    let rows_before = app.root.resources.storage().visible_browser_rows().len();
    assert_eq!(rows_before, 3, "seed data: one folder + two objects");

    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::StorageObjectPageLoadFailed {
            context,
            bucket: "alpha-bucket".to_string(),
            prefix: String::new(),
            append: true,
            error: AppError::gcloud("ERROR: Could not reach the server: timed out".to_string()),
        }),
    );

    let storage = app.root.resources.storage();
    assert_eq!(
        storage.visible_browser_rows().len(),
        rows_before,
        "an append failure must not drop already-loaded rows"
    );
    assert!(
        storage.browser_partial_page_error().is_some(),
        "an append failure must set the partial-page-error banner"
    );
    assert!(
        storage.browser_problem().is_none(),
        "an append (partial-page) failure must not also set the full browser_problem"
    );

    let projection = project_resources(&app.root.resources);
    assert!(projection.partial_page_error.is_some());
    assert!(projection.storage_problem.is_none());
    assert_eq!(projection.visible_rows, rows_before);
}

/// A `StorageObjectPageLoadFailed` for a `(bucket, prefix)` scope the user
/// has since navigated away from must be ignored: it must not clobber the
/// now-current view's loading/problem state. Mirrors the scope check
/// `set_object_page` already performs for successes.
#[test]
fn cloud_storage_object_page_load_failed_for_stale_scope_is_ignored() {
    let (mut app, context) = app_with_object_browser_open();

    // Descend into "photos/" -- this optimistically clears rows and marks
    // the browser loading for the NEW scope (bucket "alpha-bucket", prefix
    // "photos/"), leaving the old scope (prefix "") stale.
    reduce(&mut app, &Action::Resources(ResourcesAction::OpenSelected));
    assert_eq!(
        app.root.resources.storage().current_prefix(),
        "photos/",
        "OpenSelected on the first (folder) row must descend into it"
    );
    assert!(app.root.resources.storage().browser_loading());

    // A failure for the OLD (now-stale) scope must be ignored entirely.
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::StorageObjectPageLoadFailed {
            context: context.clone(),
            bucket: "alpha-bucket".to_string(),
            prefix: String::new(),
            append: false,
            error: AppError::gcloud("PERMISSION_DENIED: nope".to_string()),
        }),
    );
    let storage = app.root.resources.storage();
    assert!(
        storage.browser_loading(),
        "a stale-scope failure must not clear the loading flag for the current scope"
    );
    assert!(
        storage.browser_problem().is_none(),
        "a stale-scope failure must not set a problem on the current scope"
    );

    // A failure for the CURRENT scope must still apply normally.
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::StorageObjectPageLoadFailed {
            context,
            bucket: "alpha-bucket".to_string(),
            prefix: "photos/".to_string(),
            append: false,
            error: AppError::gcloud("PERMISSION_DENIED: nope".to_string()),
        }),
    );
    let storage = app.root.resources.storage();
    assert!(!storage.browser_loading());
    assert_eq!(
        storage.browser_problem().map(|problem| problem.kind),
        Some(GcloudErrorKind::PermissionDenied)
    );
}

/// An empty (zero-row) object page must project as a distinct "empty"
/// state, and applying a filter that matches nothing on a non-empty page
/// must project as a distinct "no match" state -- the two are not the
/// same `total_rows`/`visible_rows` combination.
#[test]
fn cloud_storage_empty_page_and_no_match_filter_project_distinct_states() {
    let mut app = AppState::new();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ConfigurationsLoaded {
            configurations: vec![config("dev", true, "dev-project")],
        }),
    );
    let context = app.root.current_context().unwrap().clone();
    reduce(
        &mut app,
        &Action::Resources(ResourcesAction::SelectService(
            ResourceServiceKind::CloudStorage,
        )),
    );
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::StorageBucketsLoaded {
            context: context.clone(),
            buckets: vec![storage_bucket("empty-bucket")],
        }),
    );
    reduce(&mut app, &Action::Resources(ResourcesAction::OpenSelected));
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::StorageObjectPageLoaded {
            context,
            bucket: "empty-bucket".to_string(),
            prefix: String::new(),
            append: false,
            page: StorageObjectPage {
                prefixes: Vec::new(),
                objects: Vec::new(),
                next_page_token: None,
            },
        }),
    );

    // Empty: no rows at all, no problem, no filter applied.
    let empty_projection = project_resources(&app.root.resources);
    assert_eq!(empty_projection.total_rows, 0);
    assert_eq!(empty_projection.visible_rows, 0);
    assert!(empty_projection.filter.applied.is_empty());
    assert!(empty_projection.storage_problem.is_none());

    // No-match: seed one row, then apply a filter that matches nothing.
    let (mut app, _context) = app_with_object_browser_open();
    reduce(&mut app, &Action::Resources(ResourcesAction::StartFilter));
    for ch in "does-not-exist".chars() {
        reduce(
            &mut app,
            &Action::Resources(ResourcesAction::AppendFilterChar(ch)),
        );
    }
    reduce(&mut app, &Action::Resources(ResourcesAction::ApplyFilter));

    let no_match_projection = project_resources(&app.root.resources);
    assert!(no_match_projection.total_rows > 0);
    assert_eq!(no_match_projection.visible_rows, 0);
    assert!(!no_match_projection.filter.applied.is_empty());
    assert!(no_match_projection.storage_problem.is_none());
}

/// An empty bucket list (a successful `StorageBucketsLoaded` with zero
/// buckets) must project as a distinct "empty" state -- zero rows, no
/// problem, and still parked on the bucket-list view -- rather than being
/// mistaken for a loading state or a filtered-to-nothing state.
#[test]
fn cloud_storage_empty_bucket_list_projects_a_distinct_empty_state() {
    let mut app = AppState::new();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ConfigurationsLoaded {
            configurations: vec![config("dev", true, "dev-project")],
        }),
    );
    let context = app.root.current_context().unwrap().clone();
    reduce(
        &mut app,
        &Action::Resources(ResourcesAction::SelectService(
            ResourceServiceKind::CloudStorage,
        )),
    );
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::StorageBucketsLoaded {
            context,
            buckets: Vec::new(),
        }),
    );

    let storage = app.root.resources.storage();
    assert_eq!(storage.view(), StorageView::Buckets);
    assert_eq!(storage.visible_buckets().len(), 0);
    assert!(storage.selected_bucket().is_none());

    let projection = project_resources(&app.root.resources);
    assert_eq!(projection.total_rows, 0);
    assert_eq!(projection.visible_rows, 0);
    assert!(projection.filter.applied.is_empty());
    assert!(projection.storage_problem.is_none());
}

fn app_with_selected_bucket() -> (AppState, GcloudContext) {
    let mut app = AppState::new();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ConfigurationsLoaded {
            configurations: vec![config("dev", true, "dev-project")],
        }),
    );
    let context = app.root.current_context().unwrap().clone();
    reduce(
        &mut app,
        &Action::Resources(ResourcesAction::SelectService(
            ResourceServiceKind::CloudStorage,
        )),
    );
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::StorageBucketsLoaded {
            context: context.clone(),
            buckets: vec![storage_bucket("alpha-bucket")],
        }),
    );
    (app, context)
}

#[test]
fn cloud_storage_open_bucket_metadata_emits_load_effect_and_enters_detail_view() {
    let (mut app, context) = app_with_selected_bucket();

    let effects = reduce(
        &mut app,
        &Action::Resources(ResourcesAction::OpenBucketMetadata),
    );

    assert_eq!(
        effects,
        vec![Effect::LoadStorageBucketDetail {
            context,
            bucket: storage_bucket("alpha-bucket"),
        }]
    );
    assert_eq!(
        app.root.resources.storage().view(),
        StorageView::BucketDetail
    );
}

/// A `StorageBucketDetailLoaded` result for the active detail target projects
/// a bucket-detail view with Summary/Metadata/JSON tabs; the JSON tab shows
/// the (already KMS-redacted) `raw_json` pretty-printed, and no KMS key
/// material -- the known key-path substring from the fixture -- appears
/// anywhere in the projected detail, including the whole workspace
/// projection's `Debug` output.
#[test]
fn cloud_storage_bucket_detail_loaded_projects_tabs_json_and_no_kms_material() {
    let (mut app, context) = app_with_selected_bucket();
    reduce(
        &mut app,
        &Action::Resources(ResourcesAction::OpenBucketMetadata),
    );

    let detail = parse_bucket_detail_json(include_str!("fixtures/gcs_bucket_detail.json"))
        .expect("bucket detail fixture should parse");
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::StorageBucketDetailLoaded {
            context,
            detail: detail.clone(),
        }),
    );

    assert_eq!(app.root.resources.storage().bucket_detail(), Some(&detail));

    let projection = project_resources(&app.root.resources);
    let ResourceDetailProjection::Storage(StorageDetailProjection::Bucket(bucket_proj)) =
        projection
            .detail
            .as_ref()
            .expect("bucket detail projection should be present")
    else {
        panic!("expected a storage bucket detail projection");
    };
    assert_eq!(bucket_proj.active_tab, StorageDetailTab::Summary);
    let loaded = bucket_proj.detail.expect("bucket detail should be loaded");
    assert!(loaded.raw_json.contains("alpha-bucket"));
    // The fixture's KMS key path must never leak, not even in the JSON tab.
    assert!(!loaded.raw_json.contains("cryptoKeys/gcs-key"));
    let debug = format!("{projection:?}");
    assert!(!debug.contains("cryptoKeys/gcs-key"));

    // Tabs cycle Summary -> Metadata -> JSON.
    reduce(&mut app, &Action::Resources(ResourcesAction::SelectNextTab));
    assert_eq!(
        app.root.resources.storage().bucket_detail_tab(),
        StorageDetailTab::Metadata
    );
    reduce(&mut app, &Action::Resources(ResourcesAction::SelectNextTab));
    assert_eq!(
        app.root.resources.storage().bucket_detail_tab(),
        StorageDetailTab::Json
    );
}

/// A `StorageBucketDetailLoadFailed` result for the active detail target
/// surfaces the formatted error through `bucket_detail_error()` while
/// `detail_target_name()` still names the bucket whose detail failed to load.
/// Pins the error-channel semantics that move to the generic collection: the
/// error target stays active so the failure result lands on the right bucket.
#[test]
fn cloud_storage_bucket_detail_load_failed_surfaces_error_and_keeps_target() {
    let (mut app, context) = app_with_selected_bucket();
    reduce(
        &mut app,
        &Action::Resources(ResourcesAction::OpenBucketMetadata),
    );

    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::StorageBucketDetailLoadFailed {
            context,
            bucket_name: "alpha-bucket".to_string(),
            error: AppError::gcloud("describe bucket failed"),
        }),
    );

    let storage = app.root.resources.storage();
    assert_eq!(storage.view(), StorageView::BucketDetail);
    assert_eq!(storage.bucket_detail(), None);
    assert!(
        storage
            .bucket_detail_error()
            .expect("bucket detail error should be surfaced")
            .contains("describe bucket failed")
    );
    assert_eq!(storage.detail_target_name(), Some("alpha-bucket"));
    assert!(storage.is_active_bucket_detail_target("alpha-bucket"));
}

#[test]
fn cloud_storage_open_selected_object_emits_load_object_detail_effect() {
    let (mut app, context) = app_with_selected_bucket();
    reduce(&mut app, &Action::Resources(ResourcesAction::OpenSelected));
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::StorageObjectPageLoaded {
            context: context.clone(),
            bucket: "alpha-bucket".to_string(),
            prefix: String::new(),
            append: false,
            page: StorageObjectPage {
                prefixes: Vec::new(),
                objects: vec![storage_object("reports/2024/mañana-测试.txt")],
                next_page_token: None,
            },
        }),
    );

    let effects = reduce(&mut app, &Action::Resources(ResourcesAction::OpenSelected));

    assert_eq!(
        effects,
        vec![Effect::LoadStorageObjectDetail {
            context,
            bucket: "alpha-bucket".to_string(),
            object_key: "reports/2024/mañana-测试.txt".to_string(),
        }]
    );
    assert_eq!(
        app.root.resources.storage().view(),
        StorageView::ObjectDetail
    );
}

/// A `StorageObjectDetailLoaded` result for the active detail target projects
/// an object-detail view surfacing size/storage class/generation/content
/// type/updated plus encryption/retention indicators, and (like the bucket
/// case) never surfaces KMS key material.
#[test]
fn cloud_storage_object_detail_loaded_projects_indicators_and_no_kms_material() {
    let (mut app, context) = app_with_selected_bucket();
    reduce(&mut app, &Action::Resources(ResourcesAction::OpenSelected));
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::StorageObjectPageLoaded {
            context: context.clone(),
            bucket: "alpha-bucket".to_string(),
            prefix: String::new(),
            append: false,
            page: StorageObjectPage {
                prefixes: Vec::new(),
                objects: vec![storage_object("reports/2024/mañana-测试.txt")],
                next_page_token: None,
            },
        }),
    );
    reduce(&mut app, &Action::Resources(ResourcesAction::OpenSelected));

    let detail = parse_object_detail_json(include_str!("fixtures/gcs_object_detail.json"))
        .expect("object detail fixture should parse");
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::StorageObjectDetailLoaded {
            context,
            detail: detail.clone(),
        }),
    );

    assert_eq!(app.root.resources.storage().object_detail(), Some(&detail));

    let projection = project_resources(&app.root.resources);
    let ResourceDetailProjection::Storage(StorageDetailProjection::Object(object_proj)) =
        projection
            .detail
            .as_ref()
            .expect("object detail projection should be present")
    else {
        panic!("expected a storage object detail projection");
    };
    assert_eq!(object_proj.active_tab, StorageDetailTab::Summary);
    let loaded = object_proj.detail.expect("object detail should be loaded");
    assert_eq!(loaded.size_bytes, Some(1024));
    assert_eq!(loaded.storage_class.as_deref(), Some("STANDARD"));
    assert_eq!(loaded.generation.as_deref(), Some("1700000000000001"));
    assert_eq!(loaded.content_type.as_deref(), Some("text/plain"));
    assert_eq!(loaded.updated.as_deref(), Some("2024-03-01T10:00:00Z"));
    assert!(loaded.kms_encrypted); // encryption indicator (presence-only)
    assert!(loaded.retention_expiration.is_some()); // retention indicator
    assert!(!loaded.raw_json.contains("cryptoKeys"));
    let debug = format!("{projection:?}");
    assert!(!debug.contains("cryptoKeys"));
}

#[test]
fn cloud_storage_copy_resource_uri_on_bucket_emits_gs_uri() {
    let (mut app, _context) = app_with_selected_bucket();

    let effects = reduce(
        &mut app,
        &Action::Resources(ResourcesAction::CopyResourceUri),
    );

    assert_eq!(
        effects,
        vec![Effect::CopyToClipboard {
            value: "gs://alpha-bucket".to_string(),
            description: "bucket URI".to_string(),
        }]
    );
}

/// `CopyResourceUri` on a selected object row must copy `gs://<bucket>/<key>`
/// with the raw object key -- verbatim, not sanitized -- even though the
/// browser row's rendered cell is sanitized for display.
#[test]
fn cloud_storage_copy_resource_uri_on_object_emits_verbatim_gs_uri() {
    let (mut app, _context) = app_with_object_browser_open();
    // Row 0 is the "photos/" prefix; move to "readme.txt".
    reduce(&mut app, &Action::Resources(ResourcesAction::SelectNext));

    let effects = reduce(
        &mut app,
        &Action::Resources(ResourcesAction::CopyResourceUri),
    );

    assert_eq!(
        effects,
        vec![Effect::CopyToClipboard {
            value: "gs://alpha-bucket/readme.txt".to_string(),
            description: "object URI".to_string(),
        }]
    );
}

/// A hostile common-prefix folder name and object key -- built through the
/// real `parse_object_page_json` parser from JSON carrying raw control bytes
/// via `\u00XX` escapes, exactly as a hostile `gcloud storage` response
/// would -- must never reach the terminal unsanitized: not in the projected
/// object-browser row cell, not in the "current prefix" breadcrumb rendered by
/// the real `render_storage_list` UI path (exercised here through a
/// `TestBackend` terminal, not by calling `sanitize_display_text` directly),
/// and not in the detail header once the leaf object's detail pane is opened.
/// `CopyResourceUri` must still copy the *raw* key bytes verbatim in every
/// case, since clipboard content is never rendered by the TUI and gcloud
/// needs the real key.
#[test]
fn cloud_storage_malicious_prefix_and_object_key_are_sanitized_in_display_but_copy_uri_is_verbatim()
{
    let (mut app, context) = app_with_selected_bucket();
    app.root.workspace = Workspace::Resources;
    reduce(&mut app, &Action::Resources(ResourcesAction::OpenSelected));

    // First page: a hostile folder, derived from an object nested inside it —
    // gcloud reports no folders of its own.
    let first_page_json = "[{\"name\":\"evil\\r\\n\\u001b[2Jname/inner.txt\"}]";
    let first_page =
        parse_object_page_json(first_page_json, "").expect("hostile prefix page should parse");
    let hostile_prefix = first_page.prefixes[0].name.clone();
    assert!(
        hostile_prefix.chars().any(|c| c.is_control()),
        "fixture sanity: the raw prefix must actually carry control bytes"
    );
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::StorageObjectPageLoaded {
            context: context.clone(),
            bucket: "alpha-bucket".to_string(),
            prefix: String::new(),
            append: false,
            page: first_page,
        }),
    );

    // The projected row cell for the hostile prefix carries no control chars.
    let projection = project_resources(&app.root.resources);
    let ResourceListProjection::Storage(rows) = &projection.list else {
        panic!("expected storage rows");
    };
    assert_eq!(rows.len(), 1);
    let StorageRowProjection::Object(row) = &rows[0] else {
        panic!("expected an object-browser row");
    };
    assert!(!row.name.chars().any(|c| c.is_control()));
    assert!(!row.name.contains('\u{1b}'));

    // Descend into the hostile prefix. The state tracks the raw bytes (needed
    // to keep listing/loading correct), but the breadcrumb the UI renders in
    // the "Resources / gs://{bucket}/{prefix}" title bar must carry no
    // control chars. This is checked by actually rendering `render_storage_list`
    // to a `TestBackend` terminal and inspecting the rendered buffer text --
    // not by re-running `sanitize_display_text` inside the assertion, which
    // would prove nothing about the real render path.
    let effects = reduce(&mut app, &Action::Resources(ResourcesAction::OpenSelected));
    assert!(matches!(
        effects.as_slice(),
        [Effect::LoadStorageObjectPage { .. }]
    ));
    let raw_prefix = app.root.resources.storage().current_prefix();
    assert_eq!(raw_prefix, hostile_prefix);

    let session = RuntimeSession::new_for_test(app.clone());
    let terminal = render_terminal(&session, 160, 24);
    let rendered = terminal_text(&terminal);
    assert!(
        !rendered.chars().any(|c| c.is_control()),
        "rendered breadcrumb must carry no control chars: {rendered:?}"
    );
    assert!(!rendered.contains('\u{1b}'));
    assert!(
        rendered.contains("Resources / gs://alpha-bucket/evil"),
        "rendered buffer should still show the sanitized, visible portion of the hostile prefix: {rendered:?}"
    );
    assert!(
        rendered.contains("name/"),
        "rendered buffer should still show the trailing visible portion of the hostile prefix: {rendered:?}"
    );

    // Second page (inside the hostile folder): a leaf object whose full key
    // also carries the raw control bytes, again built through the real
    // parser.
    let leaf_page_json = "[{\"name\":\"evil\\r\\n\\u001b[2Jname/leaf.txt\"}]";
    let leaf_page = parse_object_page_json(leaf_page_json, &hostile_prefix)
        .expect("leaf object page should parse");
    let leaf_key = leaf_page.objects[0].key.clone();
    assert_eq!(leaf_key, format!("{hostile_prefix}leaf.txt"));
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::StorageObjectPageLoaded {
            context: context.clone(),
            bucket: "alpha-bucket".to_string(),
            prefix: raw_prefix.clone(),
            append: false,
            page: leaf_page,
        }),
    );

    let projection = project_resources(&app.root.resources);
    let ResourceListProjection::Storage(rows) = &projection.list else {
        panic!("expected storage rows");
    };
    assert_eq!(rows.len(), 1);
    let StorageRowProjection::Object(leaf_row) = &rows[0] else {
        panic!("expected an object-browser row");
    };
    assert!(!leaf_row.name.chars().any(|c| c.is_control()));
    assert!(!leaf_row.name.contains('\u{1b}'));

    // Opening the leaf object's detail pane loads by the raw key...
    let effects = reduce(&mut app, &Action::Resources(ResourcesAction::OpenSelected));
    assert_eq!(
        effects,
        vec![Effect::LoadStorageObjectDetail {
            context: context.clone(),
            bucket: "alpha-bucket".to_string(),
            object_key: leaf_key.clone(),
        }]
    );
    // ...but the projected detail header carries no control chars.
    let projection = project_resources(&app.root.resources);
    let ResourceDetailProjection::Storage(StorageDetailProjection::Object(object_proj)) =
        projection
            .detail
            .as_ref()
            .expect("object detail projection should be present")
    else {
        panic!("expected a storage object detail projection");
    };
    assert!(!object_proj.target_name.chars().any(|c| c.is_control()));
    assert!(!object_proj.target_name.contains('\u{1b}'));

    // `CopyResourceUri` on the still-selected leaf object copies the URI with
    // its terminal escape sequences stripped. This sink used to be exempt on
    // the grounds that "clipboard content is never rendered by the TUI"; that
    // rationale was reconsidered and REJECTED. It is true and irrelevant: the
    // clipboard is not a terminal, but everything on it is pasted into one --
    // a shell, an editor, a chat, someone else's session -- and that terminal
    // renders it. The other two `y` sinks (the selected field and the raw JSON
    // document) are hardened, so leaving this one raw would make `y` safe two
    // presses out of three.
    //
    // Newlines survive, because the copied URI has to stay faithful to a key
    // that may legitimately contain one; only the escape sequences and the
    // remaining control bytes go. gcloud is unaffected -- the load effect
    // above still addresses the object by its raw key.
    let effects = reduce(
        &mut app,
        &Action::Resources(ResourcesAction::CopyResourceUri),
    );
    let raw_uri = format!("gs://alpha-bucket/{leaf_key}");
    assert!(raw_uri.contains('\r') && raw_uri.contains('\u{1b}'));
    assert_eq!(
        effects,
        vec![Effect::CopyToClipboard {
            value: "gs://alpha-bucket/evil\nname/leaf.txt".to_string(),
            description: "object URI".to_string(),
        }]
    );
    let [Effect::CopyToClipboard { value, .. }] = effects.as_slice() else {
        panic!("expected a single clipboard effect");
    };
    assert!(
        !value.contains('\u{1b}') && !value.contains('\r') && !value.contains('\u{FFFD}'),
        "the copied URI must carry neither an escape introducer nor replacement residue: {value:?}"
    );
    assert!(
        value.contains('\n'),
        "a newline inside the object key must survive onto the clipboard: {value:?}"
    );
}

/// `content_type` is uploader-controllable (unlike e.g. `storage_class`), so
/// a hostile value must be neutralized wherever it's projected for display --
/// here, the object-browser row cell -- exactly like names are.
#[test]
fn cloud_storage_hostile_content_type_is_sanitized_in_projected_row() {
    let (mut app, context) = app_with_selected_bucket();
    reduce(&mut app, &Action::Resources(ResourcesAction::OpenSelected));

    let json = "[{\"name\":\"a.txt\",\"content_type\":\"text/plain\\r\\n\\u001b[2JX-Evil: 1\"}]";
    let page = parse_object_page_json(json, "").expect("hostile content-type page should parse");
    assert!(
        page.objects[0]
            .content_type
            .as_deref()
            .unwrap()
            .chars()
            .any(|c| c.is_control()),
        "fixture sanity: the raw content type must actually carry control bytes"
    );
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::StorageObjectPageLoaded {
            context,
            bucket: "alpha-bucket".to_string(),
            prefix: String::new(),
            append: false,
            page,
        }),
    );

    let projection = project_resources(&app.root.resources);
    let ResourceListProjection::Storage(rows) = &projection.list else {
        panic!("expected storage rows");
    };
    let StorageRowProjection::Object(row) = &rows[0] else {
        panic!("expected an object-browser row");
    };
    assert!(!row.content_type.chars().any(|c| c.is_control()));
    assert!(!row.content_type.contains('\u{1b}'));
    assert!(row.content_type.contains("text/plain"));
}

fn render_terminal(session: &RuntimeSession, width: u16, height: u16) -> Terminal<TestBackend> {
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).expect("terminal should initialize");
    terminal
        .draw(|frame| nuvem::ui::render(frame, session))
        .expect("render should succeed");
    terminal
}

fn terminal_text(terminal: &Terminal<TestBackend>) -> String {
    terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|cell| cell.symbol())
        .collect()
}

#[test]
fn selecting_cloud_sql_then_refresh_emits_refresh_cloud_sql_instances() {
    let mut app = AppState::new();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ConfigurationsLoaded {
            configurations: vec![config("dev", true, "dev-project")],
        }),
    );

    let effects = reduce(
        &mut app,
        &Action::Resources(ResourcesAction::SelectService(
            ResourceServiceKind::CloudSql,
        )),
    );
    assert_eq!(
        app.root.resources.active_service(),
        ResourceServiceKind::CloudSql
    );
    let context = app.root.current_context().unwrap().clone();
    // Plus the remembered-view save for the switch itself; the
    // explicit refresh below learns nothing new and so emits no second save.
    assert_eq!(
        effects,
        vec![
            Effect::RefreshCloudSqlInstances {
                context: context.clone(),
                filter: None
            },
            Effect::SaveRememberedViews {
                views: remembered("dev", ResourceServiceKind::CloudSql),
            }
        ]
    );

    let refresh = reduce(&mut app, &Action::Resources(ResourcesAction::Refresh));
    assert_eq!(
        refresh,
        vec![Effect::RefreshCloudSqlInstances {
            context,
            filter: None
        }]
    );
}

#[test]
fn cloud_sql_instances_loaded_populates_rows() {
    let app = app_with_cloud_sql(vec![
        cloud_sql_instance("db-dev-1"),
        cloud_sql_instance("db-dev-2"),
    ]);
    assert_eq!(app.root.resources.cloud_sql_instances().len(), 2);
    assert_eq!(app.root.resources.cloud_sql_instances()[0].name, "db-dev-1");
    assert!(app.root.resources.cloud_sql_loaded());
}

#[test]
fn opening_selected_cloud_sql_emits_load_detail() {
    let mut app = app_with_cloud_sql(vec![cloud_sql_instance("db-dev-1")]);

    let effects = reduce(&mut app, &Action::Resources(ResourcesAction::OpenSelected));

    assert!(app.root.resources.is_showing_detail());
    let context = app.root.current_context().unwrap().clone();
    assert_eq!(
        effects,
        vec![Effect::LoadCloudSqlInstanceDetail {
            context,
            instance: cloud_sql_instance("db-dev-1"),
        }]
    );
}

#[test]
fn cloud_sql_detail_loaded_updates_detail_state() {
    let mut app = app_with_cloud_sql(vec![cloud_sql_instance("db-dev-1")]);
    reduce(&mut app, &Action::Resources(ResourcesAction::OpenSelected));
    let context = app.root.current_context().unwrap().clone();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::CloudSqlInstanceDetailLoaded {
            context,
            detail: Box::new(cloud_sql_detail("db-dev-1")),
        }),
    );
    assert_eq!(
        app.root
            .resources
            .cloud_sql_detail()
            .map(|d| d.name.as_str()),
        Some("db-dev-1")
    );
}

#[test]
fn opening_cloud_sql_users_tab_with_empty_users_emits_load_users() {
    let mut app = app_with_cloud_sql(vec![cloud_sql_instance("db-dev-1")]);
    reduce(&mut app, &Action::Resources(ResourcesAction::OpenSelected));
    let context = app.root.current_context().unwrap().clone();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::CloudSqlInstanceDetailLoaded {
            context: context.clone(),
            detail: Box::new(cloud_sql_detail("db-dev-1")),
        }),
    );

    // Tabs: Summary, Connectivity, Maintenance, Backups, Replicas, Users, Json.
    // Step backwards once (Summary -> Json), then forwards to reach Users, or
    // simply cycle forward five times to land on Users.
    let mut effects = Vec::new();
    for _ in 0..5 {
        effects = reduce(&mut app, &Action::Resources(ResourcesAction::SelectNextTab));
    }
    assert_eq!(
        effects,
        vec![Effect::LoadCloudSqlUsers {
            context,
            instance: cloud_sql_instance("db-dev-1"),
        }]
    );
}

#[test]
fn cloud_sql_detail_reload_on_users_tab_reissues_load_users() {
    let mut app = app_with_cloud_sql(vec![cloud_sql_instance("db-dev-1")]);
    reduce(&mut app, &Action::Resources(ResourcesAction::OpenSelected));
    let context = app.root.current_context().unwrap().clone();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::CloudSqlInstanceDetailLoaded {
            context: context.clone(),
            detail: Box::new(cloud_sql_detail("db-dev-1")),
        }),
    );
    // Land on the Users tab (consumes the initial lazy-load trigger).
    for _ in 0..5 {
        reduce(&mut app, &Action::Resources(ResourcesAction::SelectNextTab));
    }

    // Refreshing the detail while still on the Users tab replaces the detail
    // with an empty user list, so the users load must be re-issued.
    let effects = reduce(
        &mut app,
        &Action::EffectResult(EffectResult::CloudSqlInstanceDetailLoaded {
            context: context.clone(),
            detail: Box::new(cloud_sql_detail("db-dev-1")),
        }),
    );
    assert_eq!(
        effects,
        vec![Effect::LoadCloudSqlUsers {
            context,
            instance: cloud_sql_instance("db-dev-1"),
        }]
    );
}

#[test]
fn cloud_sql_users_loaded_stores_users_on_active_detail() {
    let mut app = app_with_cloud_sql(vec![cloud_sql_instance("db-dev-1")]);
    reduce(&mut app, &Action::Resources(ResourcesAction::OpenSelected));
    let context = app.root.current_context().unwrap().clone();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::CloudSqlInstanceDetailLoaded {
            context: context.clone(),
            detail: Box::new(cloud_sql_detail("db-dev-1")),
        }),
    );
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::CloudSqlUsersLoaded {
            context,
            instance_name: "db-dev-1".to_string(),
            users: vec![CloudSqlUser {
                name: "postgres".to_string(),
                host: None,
                kind: Some("BUILT_IN".to_string()),
            }],
        }),
    );
    assert_eq!(
        app.root.resources.cloud_sql_detail().map(|d| d.users.len()),
        Some(1)
    );
}

#[test]
fn cloud_sql_load_failure_marks_stale_and_records_problem() {
    let mut app = app_with_cloud_sql(vec![cloud_sql_instance("db-dev-1")]);
    let context = app.root.current_context().unwrap().clone();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::CloudSqlInstancesLoadFailed {
            context,
            error: AppError::gcloud("boom".to_string()),
        }),
    );
    assert!(app.root.resources.cloud_sql_stale());
    assert!(app.root.resources.cloud_sql_problem().is_some());
    assert_eq!(app.root.resources.cloud_sql_instances().len(), 0);
}

#[test]
fn confirming_batch_action_emits_batch_effect_and_records_pending_operations() {
    let mut app = app_with_instances();
    let context = app.root.current_context().expect("context exists").clone();
    // Mark both loaded (Running) instances.
    app.root.resources.toggle_mark();
    app.root.resources.select_next();
    app.root.resources.toggle_mark();
    let targets: Vec<BatchRowRef> = app
        .root
        .resources
        .marked_instances()
        .iter()
        .map(|row| Compute::row_ref(row, &context, ComputeInstanceAction::Stop))
        .collect();
    assert_eq!(targets.len(), 2);
    app.root
        .resources
        .start_batch_confirmation::<Compute>(context.clone(), ComputeInstanceAction::Stop);

    let effects = reduce(
        &mut app,
        &Action::Resources(ResourcesAction::ConfirmBatchAction),
    );

    // Every target rides on the effect paired with the entry begun for it, so
    // its result can settle that entry.
    let targets = pair_with_seeded_ids(&app, targets);
    assert_eq!(
        effects,
        vec![Effect::RunResourceBatch {
            context,
            action: BatchAction::Compute(ComputeInstanceAction::Stop),
            targets,
            concurrency: 0,
        }]
    );
    assert_eq!(app.root.system.operations.pending_count(), 2);
    assert!(app.root.resources.batch_confirmation().is_none());
}

/// Cancelling a batch resolves its stranded log entries by id, so the workflow
/// each confirmation seeds must carry the ids of the entries that same action
/// created — and each id must sit on the target it was begun for, not merely
/// somewhere in the workflow.
#[test]
fn confirming_a_compute_batch_pairs_each_workflow_target_with_the_entry_it_seeded() {
    let mut app = app_with_instances();
    let context = app.root.current_context().expect("context exists").clone();
    app.root.resources.toggle_mark();
    app.root.resources.select_next();
    app.root.resources.toggle_mark();
    app.root
        .resources
        .start_batch_confirmation::<Compute>(context, ComputeInstanceAction::Stop);

    reduce(
        &mut app,
        &Action::Resources(ResourcesAction::ConfirmBatchAction),
    );

    let workflow = app
        .root
        .resources
        .batch_workflow()
        .expect("workflow seeded");
    assert_eq!(workflow.targets().len(), 2);
    for target in workflow.targets() {
        let record = app
            .root
            .system
            .operations
            .records()
            .iter()
            .find(|record| record.id == target.operation_id)
            .unwrap_or_else(|| panic!("no operations-log entry with id {}", target.operation_id));
        assert_eq!(record.target, target.name);
        assert_eq!(record.kind, "Stop instance");
    }
    // Distinct entries, not the same id copied onto every target.
    let ids: Vec<u64> = workflow
        .targets()
        .iter()
        .map(|target| target.operation_id)
        .collect();
    assert_ne!(ids[0], ids[1]);
}

#[test]
fn confirming_a_cloud_sql_batch_pairs_each_workflow_target_with_the_entry_it_seeded() {
    use nuvem::app::state::resources::{CloudSql, CloudSqlInstanceAction};

    let mut app = app_with_cloud_sql(vec![
        cloud_sql_instance("orders-dev"),
        cloud_sql_instance("reports-dev"),
    ]);
    let context = app.root.current_context().expect("context exists").clone();
    app.root.resources.toggle_mark();
    app.root.resources.select_next();
    app.root.resources.toggle_mark();
    app.root
        .resources
        .start_batch_confirmation::<CloudSql>(context, CloudSqlInstanceAction::Stop);
    reduce(
        &mut app,
        &Action::Resources(ResourcesAction::BatchAckAppendDigit('2')),
    );

    reduce(
        &mut app,
        &Action::Resources(ResourcesAction::ConfirmBatchAction),
    );

    let workflow = app
        .root
        .resources
        .batch_workflow()
        .expect("workflow seeded");
    assert_eq!(workflow.targets().len(), 2);
    let paired: Vec<(&str, &str)> = workflow
        .targets()
        .iter()
        .map(|target| {
            let record = app
                .root
                .system
                .operations
                .records()
                .iter()
                .find(|record| record.id == target.operation_id)
                .unwrap_or_else(|| {
                    panic!("no operations-log entry with id {}", target.operation_id)
                });
            (target.name.as_str(), record.target.as_str())
        })
        .collect();
    assert_eq!(
        paired,
        vec![("orders-dev", "orders-dev"), ("reports-dev", "reports-dev")]
    );
}

/// The batch effect carries the confirmed [`BatchRowRef`]s themselves, so a
/// refresh that lands while the confirmation is open (dropping a row the user
/// already confirmed) cannot change or silently drop a target: what was
/// confirmed is exactly what is dispatched.
#[test]
fn confirmed_batch_targets_survive_a_refresh_that_drops_their_rows() {
    let mut app = app_with_instances();
    let context = app.root.current_context().expect("context exists").clone();
    app.root.resources.toggle_mark();
    app.root.resources.select_next();
    app.root.resources.toggle_mark();
    let confirmed: Vec<BatchRowRef> = app
        .root
        .resources
        .marked_instances()
        .iter()
        .map(|row| Compute::row_ref(row, &context, ComputeInstanceAction::Stop))
        .collect();
    assert_eq!(confirmed.len(), 2);
    app.root
        .resources
        .start_batch_confirmation::<Compute>(context.clone(), ComputeInstanceAction::Stop);

    // A refresh lands while the confirmation is open and drops every row.
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ComputeInstancesLoaded {
            context: context.clone(),
            instances: Vec::new(),
        }),
    );

    let effects = reduce(
        &mut app,
        &Action::Resources(ResourcesAction::ConfirmBatchAction),
    );

    let confirmed = pair_with_seeded_ids(&app, confirmed);
    assert_eq!(
        effects,
        vec![Effect::RunResourceBatch {
            context,
            action: BatchAction::Compute(ComputeInstanceAction::Stop),
            targets: confirmed,
            concurrency: 0,
        }]
    );
}

/// The operations-log id a confirmed single-row compute action carries on its
/// effect, so a test can settle exactly the entry that confirmation seeded.
fn single_row_operation_id(effects: &[Effect]) -> u64 {
    match effects {
        [Effect::ComputeInstanceAction { operation_id, .. }] => *operation_id,
        other => panic!("expected one compute action effect, got {other:?}"),
    }
}

/// Confirms a `Stop` batch over the single marked row `web-1` and returns the
/// operations-log id seeded for it.
fn confirm_stop_batch_over_web_1(app: &mut AppState) -> u64 {
    let context = app.root.current_context().expect("context exists").clone();
    app.root.resources.toggle_mark();
    app.root
        .resources
        .start_batch_confirmation::<Compute>(context, ComputeInstanceAction::Stop);
    reduce(app, &Action::Resources(ResourcesAction::ConfirmBatchAction));
    let workflow = app
        .root
        .resources
        .batch_workflow()
        .expect("workflow seeded");
    assert_eq!(workflow.targets().len(), 1);
    workflow.targets()[0].operation_id
}

/// Confirms a single-row `Stop` on `web-1` and returns the operations-log id
/// seeded for it, taken from the effect that carries it onwards.
fn confirm_single_row_stop_on_web_1(app: &mut AppState) -> u64 {
    let context = app.root.current_context().expect("context exists").clone();
    let instance = app
        .root
        .resources
        .instances()
        .iter()
        .find(|row| row.name == "web-1")
        .expect("web-1 loaded")
        .clone();
    app.root
        .resources
        .start_action_confirmation(context, instance, ComputeInstanceAction::Stop);
    let effects = reduce(
        app,
        &Action::Resources(ResourcesAction::ConfirmPendingAction),
    );
    single_row_operation_id(&effects)
}

fn operation_outcome(app: &AppState, id: u64) -> OperationOutcome {
    app.root
        .system
        .operations
        .records()
        .iter()
        .find(|record| record.id == id)
        .unwrap_or_else(|| panic!("no operations-log entry with id {id}"))
        .outcome
}

/// A batch target and a single-row action can be in flight over the same
/// instance at once, and each result must settle the entry its own action
/// seeded. Picking the most recent pending entry that matches `(kind, target)`
/// instead settles whichever was started last, so the batch's result would
/// close the single-row action's entry and strand its own at `Pending`.
#[test]
fn batch_result_settles_its_own_entry_while_a_later_single_row_action_stays_pending() {
    let mut app = AppState::new();
    load_resource_rows(&mut app, vec![instance("web-1"), instance("web-2")]);
    let context = app.root.current_context().expect("context exists").clone();

    let batch_id = confirm_stop_batch_over_web_1(&mut app);
    let single_row_id = confirm_single_row_stop_on_web_1(&mut app);
    assert_ne!(batch_id, single_row_id);

    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ComputeInstanceActionSucceeded {
            context,
            instance_name: "web-1".to_string(),
            instance_zone: "europe-west1-b".to_string(),
            action: ComputeInstanceAction::Stop,
            operation_id: batch_id,
        }),
    );

    assert_eq!(
        operation_outcome(&app, batch_id),
        OperationOutcome::Succeeded
    );
    assert_eq!(
        operation_outcome(&app, single_row_id),
        OperationOutcome::Pending
    );
}

/// The mirror interleaving: the single-row action is started first and the
/// batch second, so the entry started last belongs to the batch. The
/// single-row result must still settle its own entry and leave the batch's
/// pending.
#[test]
fn single_row_result_settles_its_own_entry_while_a_later_batch_stays_pending() {
    let mut app = AppState::new();
    load_resource_rows(&mut app, vec![instance("web-1"), instance("web-2")]);
    let context = app.root.current_context().expect("context exists").clone();

    let single_row_id = confirm_single_row_stop_on_web_1(&mut app);
    let batch_id = confirm_stop_batch_over_web_1(&mut app);
    assert_ne!(batch_id, single_row_id);

    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ComputeInstanceActionSucceeded {
            context,
            instance_name: "web-1".to_string(),
            instance_zone: "europe-west1-b".to_string(),
            action: ComputeInstanceAction::Stop,
            operation_id: single_row_id,
        }),
    );

    assert_eq!(
        operation_outcome(&app, single_row_id),
        OperationOutcome::Succeeded
    );
    assert_eq!(operation_outcome(&app, batch_id), OperationOutcome::Pending);
}

#[test]
fn canceling_batch_action_clears_confirmation_and_emits_no_effect() {
    let mut app = app_with_instances();
    let context = app.root.current_context().expect("context exists").clone();
    app.root.resources.toggle_mark();
    app.root
        .resources
        .start_batch_confirmation::<Compute>(context, ComputeInstanceAction::Stop);
    assert!(app.root.resources.batch_confirmation().is_some());

    let effects = reduce(
        &mut app,
        &Action::Resources(ResourcesAction::CancelBatchAction),
    );

    assert!(effects.is_empty());
    assert!(app.root.resources.batch_confirmation().is_none());
}

#[test]
fn switching_back_to_a_configuration_restores_cached_rows_as_stale() {
    let mut app = AppState::new();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ConfigurationsLoaded {
            configurations: vec![
                config("dev", true, "dev-project"),
                config("prod", false, "prod-project"),
            ],
        }),
    );
    let dev_context = app.root.current_context().unwrap().clone();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ComputeInstancesLoaded {
            context: dev_context,
            instances: vec![instance("api-dev-1"), instance("api-dev-2")],
        }),
    );

    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ConfigurationActivated {
            configuration: config("prod", true, "prod-project"),
        }),
    );
    assert!(app.root.resources.instances().is_empty());

    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ConfigurationActivated {
            configuration: config("dev", true, "dev-project"),
        }),
    );

    assert_eq!(app.root.resources.instances().len(), 2);
    assert_eq!(app.root.resources.instances()[0].name, "api-dev-1");
    assert!(app.root.resources.is_stale());
}

/// Cloud SQL end of the batch path: a marked set plus a destructive action
/// requires a typed count and, once satisfied, confirming emits a single
/// `RunResourceBatch` carrying the Cloud SQL targets and their previews.
#[test]
fn confirming_cloud_sql_batch_action_emits_batch_effect_with_targets_and_previews() {
    use nuvem::app::state::resources::{BatchAck, CloudSql, CloudSqlInstanceAction};

    let mut app = app_with_cloud_sql(vec![
        cloud_sql_instance("orders-dev"),
        cloud_sql_instance("reports-dev"),
    ]);
    let context = app.root.current_context().expect("context exists").clone();
    app.root.resources.toggle_mark();
    app.root.resources.select_next();
    app.root.resources.toggle_mark();
    assert_eq!(app.root.resources.marked_count(), 2);

    app.root
        .resources
        .start_batch_confirmation::<CloudSql>(context.clone(), CloudSqlInstanceAction::Stop);

    let confirmation = app
        .root
        .resources
        .batch_confirmation()
        .expect("batch confirmation open");
    assert_eq!(
        confirmation.ack(),
        &BatchAck::CountRequired {
            typed: String::new()
        }
    );
    assert!(
        !confirmation
            .ack()
            .is_satisfied(confirmation.eligible().len())
    );
    let targets: Vec<BatchRowRef> = confirmation.eligible().to_vec();
    assert_eq!(
        targets
            .iter()
            .map(|target| target.call.preview())
            .collect::<Vec<_>>(),
        vec![
            "sql.instances.patch instances/orders-dev activationPolicy=NEVER".to_string(),
            "sql.instances.patch instances/reports-dev activationPolicy=NEVER".to_string(),
        ]
    );

    reduce(
        &mut app,
        &Action::Resources(ResourcesAction::BatchAckAppendDigit('2')),
    );
    let confirmation = app
        .root
        .resources
        .batch_confirmation()
        .expect("batch confirmation open");
    assert!(
        confirmation
            .ack()
            .is_satisfied(confirmation.eligible().len())
    );

    let effects = reduce(
        &mut app,
        &Action::Resources(ResourcesAction::ConfirmBatchAction),
    );

    let targets = pair_with_seeded_ids(&app, targets);
    assert_eq!(
        effects,
        vec![Effect::RunResourceBatch {
            context,
            action: BatchAction::CloudSql(CloudSqlInstanceAction::Stop),
            targets,
            concurrency: 0,
        }]
    );
    assert_eq!(app.root.system.operations.pending_count(), 2);
    assert!(app.root.resources.batch_confirmation().is_none());
    assert_eq!(
        app.root
            .resources
            .batch_workflow()
            .expect("workflow seeded")
            .targets()
            .len(),
        2
    );
}

/// A marked Cloud SQL row that is ineligible for the action lands in `skipped`
/// with its reason rather than being silently dropped from the batch.
#[test]
fn ineligible_cloud_sql_row_is_skipped_with_a_reason_not_dropped() {
    use nuvem::app::state::resources::{CloudSql, CloudSqlInstanceAction, SkipReason};

    let ha = CloudSqlInstance {
        availability: CloudSqlAvailability::Regional,
        ..cloud_sql_instance("orders-dev")
    };
    let zonal = cloud_sql_instance("reports-dev");
    let mut app = app_with_cloud_sql(vec![ha, zonal]);
    let context = app.root.current_context().expect("context exists").clone();
    app.root.resources.toggle_mark();
    app.root.resources.select_next();
    app.root.resources.toggle_mark();

    app.root
        .resources
        .start_batch_confirmation::<CloudSql>(context, CloudSqlInstanceAction::Failover);

    let confirmation = app
        .root
        .resources
        .batch_confirmation()
        .expect("batch confirmation open");
    assert_eq!(
        confirmation
            .eligible()
            .iter()
            .map(|target| target.name.as_str())
            .collect::<Vec<_>>(),
        vec!["orders-dev"]
    );
    assert_eq!(confirmation.skipped().len(), 1);
    assert_eq!(confirmation.skipped()[0].0.name, "reports-dev");
    // Runnable but zonal: skipped for its availability, so the reason names the
    // HA requirement rather than a status that reads as eligible.
    assert_eq!(
        confirmation.skipped()[0].1,
        SkipReason::RequiresHighAvailability
    );
}

/// `EffectResult::CloudSqlInstanceAction{Succeeded,Failed}` carry no region, so
/// the reducer must resolve their batch target by name alone. Without this the
/// target would stay `Pending` forever and the workflow would never complete.
#[test]
fn cloud_sql_action_results_resolve_their_batch_target_by_name() {
    use nuvem::app::state::resources::{BatchTargetStatus, CloudSql, CloudSqlInstanceAction};

    let mut app = app_with_cloud_sql(vec![
        cloud_sql_instance("orders-dev"),
        cloud_sql_instance("reports-dev"),
    ]);
    let context = app.root.current_context().expect("context exists").clone();
    app.root.resources.toggle_mark();
    app.root.resources.select_next();
    app.root.resources.toggle_mark();
    app.root
        .resources
        .start_batch_confirmation::<CloudSql>(context.clone(), CloudSqlInstanceAction::Restart);
    reduce(
        &mut app,
        &Action::Resources(ResourcesAction::ConfirmBatchAction),
    );

    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::CloudSqlInstanceActionSucceeded {
            context: context.clone(),
            instance_name: "orders-dev".to_string(),
            action: CloudSqlInstanceAction::Restart,
            operation_id: UNTRACKED_OPERATION_ID,
        }),
    );
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::CloudSqlInstanceActionFailed {
            context,
            instance_name: "reports-dev".to_string(),
            action: CloudSqlInstanceAction::Restart,
            error: AppError::gcloud("boom".to_string()),
            operation_id: UNTRACKED_OPERATION_ID,
        }),
    );

    let workflow = app
        .root
        .resources
        .batch_workflow()
        .expect("workflow present");
    assert_eq!(workflow.targets()[0].status, BatchTargetStatus::Succeeded);
    assert!(matches!(
        workflow.targets()[1].status,
        BatchTargetStatus::Failed(_)
    ));
    assert!(workflow.is_complete());
}

// --- Secret Manager: what a late result is allowed to touch -----------------
//
// Every secret result carries the context it was fetched for and the secret it
// is about, and both are re-checked on arrival because both can have moved on:
// the operator can switch configuration, or close one secret and open another,
// while a describe or a versions listing is still in flight. A result applied
// without those checks paints one secret's metadata under another secret's
// name — or one project's under another project's — which is the one failure
// mode an operator has no way to notice.

fn secret(name: &str) -> Secret {
    Secret {
        name: name.to_string(),
        create_time: Some("2026-02-11T09:14:22Z".to_string()),
        labels: Vec::new(),
        replication: "Automatic".to_string(),
    }
}

fn secret_detail(name: &str) -> SecretDetail {
    SecretDetail {
        name: name.to_string(),
        create_time: Some("2026-02-11T09:14:22Z".to_string()),
        labels: Vec::new(),
        annotations: Vec::new(),
        replication: "Automatic".to_string(),
        rotation: None,
        expire_time: None,
        versions: Vec::new(),
        raw_json: "{}".to_string(),
    }
}

fn secret_version(name: &str) -> SecretVersion {
    SecretVersion {
        name: name.to_string(),
        state: SecretVersionState::Enabled,
        create_time: Some("2026-06-01T11:02:45Z".to_string()),
    }
}

fn app_showing_secrets() -> AppState {
    let mut app = AppState::new();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ConfigurationsLoaded {
            configurations: vec![config("dev", true, "dev-project")],
        }),
    );
    let context = app.root.current_context().expect("context exists");
    app.root.workspace = Workspace::Resources;
    reduce(
        &mut app,
        &Action::Resources(ResourcesAction::SelectService(
            ResourceServiceKind::SecretManagerSecrets,
        )),
    );
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::SecretsLoaded {
            context,
            secrets: vec![secret("api-signing-key"), secret("db-password")],
        }),
    );
    app
}

/// An app with `api-signing-key` open, which is what every result below is
/// addressed to — or deliberately not addressed to.
fn app_with_open_secret() -> AppState {
    let mut app = app_showing_secrets();
    reduce(&mut app, &Action::Resources(ResourcesAction::OpenSelected));
    assert!(
        app.root
            .resources
            .is_active_secret_detail_target("api-signing-key"),
        "the fixture must genuinely have api-signing-key open"
    );
    app
}

/// The context a result can carry that is not the one now active.
fn other_context() -> GcloudContext {
    GcloudContext {
        configuration: GcloudConfigurationName::new("prod"),
        account: None,
        project: Some("prod-project".to_string()),
        region: None,
        zone: None,
    }
}

/// Opening a secret fetches its metadata and its versions in one step.
///
/// The version count on the Summary tab comes from the versions listing, so a
/// describe on its own opens a pane that reports "Versions: 0" for a secret
/// that has ten. The two effects are one user action and must be emitted
/// together.
#[test]
fn opening_a_secret_emits_both_its_detail_and_its_versions_load() {
    let mut app = app_showing_secrets();
    let context = app.root.current_context().expect("context exists");

    let effects = reduce(&mut app, &Action::Resources(ResourcesAction::OpenSelected));

    assert_eq!(
        effects,
        vec![
            Effect::LoadSecretDetail {
                context: context.clone(),
                secret: secret("api-signing-key"),
            },
            Effect::LoadSecretVersions {
                context,
                secret: secret("api-signing-key"),
            },
        ]
    );
}

/// Applying a secrets filter re-issues the list carrying the filter.
///
/// Secret Manager pushes a label filter to gcloud, so the reload *is* the
/// narrowing; a filter that emits no reload leaves the previous rows on screen
/// under a chip claiming they were narrowed.
#[test]
fn applying_a_secrets_filter_reissues_the_list_with_the_filter() {
    let mut app = app_showing_secrets();
    let context = app.root.current_context().expect("context exists");

    reduce(&mut app, &Action::Resources(ResourcesAction::StartFilter));
    for ch in "db".chars() {
        reduce(
            &mut app,
            &Action::Resources(ResourcesAction::AppendFilterChar(ch)),
        );
    }
    let effects = reduce(&mut app, &Action::Resources(ResourcesAction::ApplyFilter));

    assert_eq!(
        effects,
        vec![Effect::RefreshSecrets {
            context,
            filter: Some("db".to_string()),
        }]
    );
}

#[test]
fn secret_detail_for_the_open_secret_and_the_active_context_is_installed() {
    let mut app = app_with_open_secret();
    let context = app.root.current_context().expect("context exists");

    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::SecretDetailLoaded {
            context,
            detail: secret_detail("api-signing-key"),
        }),
    );

    assert_eq!(
        app.root
            .resources
            .secret_detail()
            .map(|detail| detail.name.as_str()),
        Some("api-signing-key")
    );
}

/// A describe that resolved for a configuration the operator has since left
/// must not fill the pane. `prod-project`'s metadata under a `dev-project`
/// heading is indistinguishable from the truth on screen.
#[test]
fn secret_detail_from_a_context_the_operator_has_left_is_dropped() {
    let mut app = app_with_open_secret();

    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::SecretDetailLoaded {
            context: other_context(),
            detail: secret_detail("api-signing-key"),
        }),
    );

    assert!(
        app.root.resources.secret_detail().is_none(),
        "a result for another context must not fill the open pane"
    );
}

/// The same check on the other axis: right project, wrong secret.
#[test]
fn secret_detail_for_a_secret_that_is_no_longer_open_is_dropped() {
    let mut app = app_with_open_secret();
    let context = app.root.current_context().expect("context exists");

    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::SecretDetailLoaded {
            context,
            detail: secret_detail("db-password"),
        }),
    );

    assert!(
        app.root.resources.secret_detail().is_none(),
        "db-password's metadata must not be painted under api-signing-key"
    );
}

#[test]
fn a_failed_describe_for_the_open_secret_is_recorded_against_it() {
    let mut app = app_with_open_secret();
    let context = app.root.current_context().expect("context exists");

    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::SecretDetailLoadFailed {
            context,
            secret_name: "api-signing-key".to_string(),
            error: AppError::gcloud("PERMISSION_DENIED: secretmanager.secrets.get".to_string()),
        }),
    );

    assert!(
        app.root
            .resources
            .secret_detail_error()
            .is_some_and(|error| error.contains("PERMISSION_DENIED")),
        "the pane must say why it is empty: {:?}",
        app.root.resources.secret_detail_error()
    );
}

/// A failure that belongs to a context the operator has left must not be shown.
///
/// A `PERMISSION_DENIED` from `prod` painted over an open `dev` secret reads as
/// "you cannot see this secret", which is a claim about the wrong project.
#[test]
fn a_failed_describe_from_another_context_is_dropped() {
    let mut app = app_with_open_secret();

    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::SecretDetailLoadFailed {
            context: other_context(),
            secret_name: "api-signing-key".to_string(),
            error: AppError::gcloud("PERMISSION_DENIED: secretmanager.secrets.get".to_string()),
        }),
    );

    assert!(
        app.root.resources.secret_detail_error().is_none(),
        "another context's failure must not be reported against this secret"
    );
}

#[test]
fn a_failed_describe_for_a_secret_that_is_no_longer_open_is_dropped() {
    let mut app = app_with_open_secret();
    let context = app.root.current_context().expect("context exists");

    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::SecretDetailLoadFailed {
            context,
            secret_name: "db-password".to_string(),
            error: AppError::gcloud("PERMISSION_DENIED: secretmanager.secrets.get".to_string()),
        }),
    );

    assert!(
        app.root.resources.secret_detail_error().is_none(),
        "another secret's failure must not be reported against this one"
    );
}

#[test]
fn versions_for_the_open_secret_and_the_active_context_are_installed() {
    let mut app = app_with_open_secret();
    let context = app.root.current_context().expect("context exists");
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::SecretDetailLoaded {
            context: context.clone(),
            detail: secret_detail("api-signing-key"),
        }),
    );

    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::SecretVersionsLoaded {
            context,
            secret_name: "api-signing-key".to_string(),
            versions: vec![secret_version("3")],
        }),
    );

    assert_eq!(
        app.root
            .resources
            .secret_detail()
            .map(|detail| detail.versions.len()),
        Some(1)
    );
}

/// Versions listed for another project must not land on the open secret: the
/// version ids and states would be another secret's history entirely.
#[test]
fn versions_from_a_context_the_operator_has_left_are_dropped() {
    let mut app = app_with_open_secret();
    let context = app.root.current_context().expect("context exists");
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::SecretDetailLoaded {
            context,
            detail: secret_detail("api-signing-key"),
        }),
    );

    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::SecretVersionsLoaded {
            context: other_context(),
            secret_name: "api-signing-key".to_string(),
            versions: vec![secret_version("3")],
        }),
    );

    assert_eq!(
        app.root
            .resources
            .secret_detail()
            .map(|detail| detail.versions.len()),
        Some(0),
        "another context's versions must not be merged into this detail"
    );
}

#[test]
fn versions_for_a_secret_that_is_no_longer_open_are_dropped() {
    let mut app = app_with_open_secret();
    let context = app.root.current_context().expect("context exists");
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::SecretDetailLoaded {
            context: context.clone(),
            detail: secret_detail("api-signing-key"),
        }),
    );

    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::SecretVersionsLoaded {
            context,
            secret_name: "db-password".to_string(),
            versions: vec![secret_version("3")],
        }),
    );

    assert_eq!(
        app.root
            .resources
            .secret_detail()
            .map(|detail| detail.versions.len()),
        Some(0),
        "db-password's versions must not be merged into api-signing-key"
    );
}

#[test]
fn a_failed_versions_listing_for_the_open_secret_is_recorded_against_it() {
    let mut app = app_with_open_secret();
    let context = app.root.current_context().expect("context exists");

    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::SecretVersionsLoadFailed {
            context,
            secret_name: "api-signing-key".to_string(),
            error: AppError::gcloud("PERMISSION_DENIED: secretmanager.versions.list".to_string()),
        }),
    );

    assert!(
        app.root
            .resources
            .secret_versions_error()
            .is_some_and(|error| error.contains("PERMISSION_DENIED")),
        "the version count must be reported as unavailable, not as zero: {:?}",
        app.root.resources.secret_versions_error()
    );
}

/// The failure of another project's listing must not make this secret's count
/// read "unavailable" — the count that did load is the one on screen.
#[test]
fn a_failed_versions_listing_from_another_context_is_dropped() {
    let mut app = app_with_open_secret();

    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::SecretVersionsLoadFailed {
            context: other_context(),
            secret_name: "api-signing-key".to_string(),
            error: AppError::gcloud("PERMISSION_DENIED: secretmanager.versions.list".to_string()),
        }),
    );

    assert!(
        app.root.resources.secret_versions_error().is_none(),
        "another context's failure must not be reported against this secret"
    );
}

#[test]
fn a_failed_versions_listing_for_a_secret_that_is_no_longer_open_is_dropped() {
    let mut app = app_with_open_secret();
    let context = app.root.current_context().expect("context exists");

    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::SecretVersionsLoadFailed {
            context,
            secret_name: "db-password".to_string(),
            error: AppError::gcloud("PERMISSION_DENIED: secretmanager.versions.list".to_string()),
        }),
    );

    assert!(
        app.root.resources.secret_versions_error().is_none(),
        "another secret's failure must not be reported against this one"
    );
}
