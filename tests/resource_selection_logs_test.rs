//! `L` opens Cloud Logging scoped to the *current selection*,
//! from both the resource list and the resource detail view.
//!
//! Eight services are covered here, named rather than counted: Compute, GKE,
//! Cloud SQL, Cloud Run (service and revision), Cloud Storage, Secret Manager,
//! Cloud Functions, and BigQuery datasets. That is not every service with
//! logs. Pub/Sub topics and subscriptions have a resolver arm and a filter and
//! are pinned by neither this file nor a `render_filter` unit test — a real
//! gap, written down so the enumeration above is not read as a complete one.
//!
//! The Cloud SQL case is the sharp edge: its filter is built from a composite
//! `database_id` of the form `<project>:<instance>`, so a missing or wrong
//! project silently matches zero rows — indistinguishable from "this instance
//! has no logs". Those tests pin the project to the active gcloud context.

use nuvem::app::reducer::root::reduce;
use nuvem::app::state::command::CommandDescriptor;
use nuvem::app::state::resources::StorageView;
use nuvem::app::state::root::Workspace;
use nuvem::app::{
    Action, AppState, CommandAction, Effect, EffectResult, NavAction, ResourcesAction,
    WorkspaceAction,
};
use nuvem_domain::gcp::configuration::{GcloudConfiguration, GcloudConfigurationName};
use nuvem_domain::gcp::logging::{LogQuery, LogResourceKind};
use nuvem_domain::gcp::resource::{
    BigQueryDataset, CloudFunction, CloudFunctionEnvironment, CloudFunctionState,
    CloudRunCondition, CloudRunRevision, CloudRunService, CloudRunServiceDetail,
    CloudRunServiceStatus, CloudSqlAvailability, CloudSqlInstance, CloudSqlStatus, ComputeInstance,
    EndpointVisibility, GkeCluster, GkeClusterStatus, ResourceServiceKind, ResourceStatus, Secret,
    StorageBucket,
};

// ---------------------------------------------------------------------------
// fixtures
// ---------------------------------------------------------------------------

fn config(name: &str, project: Option<&str>) -> GcloudConfiguration {
    GcloudConfiguration {
        name: GcloudConfigurationName::new(name),
        is_active: true,
        account: Some("dev@example.com".to_string()),
        project: project.map(str::to_string),
        region: Some("europe-west1".to_string()),
        zone: Some("europe-west1-b".to_string()),
    }
}

fn app_with_context(project: Option<&str>) -> AppState {
    let mut app = AppState::new();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ConfigurationsLoaded {
            configurations: vec![config("dev", project)],
        }),
    );
    reduce(&mut app, &Action::Workspace(WorkspaceAction::Resources));
    app
}

fn instance(name: &str) -> ComputeInstance {
    ComputeInstance {
        id: format!("{name}-id"),
        name: name.to_string(),
        zone: "europe-west1-b".to_string(),
        status: ResourceStatus::Running,
        machine_type: "e2-medium".to_string(),
        internal_ip: Some("10.0.0.2".to_string()),
        external_ip: None,
    }
}

fn gke_cluster(name: &str) -> GkeCluster {
    GkeCluster {
        name: name.to_string(),
        location: "europe-west1".to_string(),
        status: GkeClusterStatus::Running,
        current_master_version: Some("1.30".to_string()),
        current_node_count: Some(3),
        endpoint_visibility: EndpointVisibility::Public,
    }
}

fn cloud_sql_instance(name: &str) -> CloudSqlInstance {
    CloudSqlInstance {
        name: name.to_string(),
        database_version: "POSTGRES_15".to_string(),
        engine: "PostgreSQL".to_string(),
        region: "us-central1".to_string(),
        tier: "db-custom-2-7680".to_string(),
        availability: CloudSqlAvailability::Zonal,
        status: CloudSqlStatus::Runnable,
        connectivity: EndpointVisibility::Public,
    }
}

fn cloud_run_service(name: &str, region: &str) -> CloudRunService {
    CloudRunService {
        name: name.to_string(),
        region: region.to_string(),
        url: None,
        latest_ready_revision: Some(format!("{name}-00001")),
        latest_created_revision: Some(format!("{name}-00001")),
        traffic_summary: "100% -> latest".to_string(),
        status: CloudRunServiceStatus::Ready,
        ready_message: None,
    }
}

fn revision(name: &str) -> CloudRunRevision {
    CloudRunRevision {
        name: name.to_string(),
        ready: CloudRunServiceStatus::Ready,
        created: None,
        image: Some("gcr.io/x/y:latest".to_string()),
        active_percent: Some(100),
    }
}

fn cloud_run_detail(
    name: &str,
    region: &str,
    revisions: Vec<CloudRunRevision>,
) -> CloudRunServiceDetail {
    CloudRunServiceDetail {
        name: name.to_string(),
        region: region.to_string(),
        url: None,
        status: CloudRunServiceStatus::Ready,
        latest_ready_revision: Some(format!("{name}-00001")),
        latest_created_revision: Some(format!("{name}-00001")),
        traffic: Vec::new(),
        revisions,
        conditions: vec![CloudRunCondition {
            type_: "Ready".to_string(),
            status: "True".to_string(),
            reason: None,
            message: None,
            last_transition_time: None,
        }],
        image: None,
        cpu: None,
        memory: None,
        concurrency: None,
        service_account: None,
        ingress: None,
        min_instances: None,
        max_instances: None,
        labels: Vec::new(),
        raw_json: "{}".to_string(),
    }
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

/// Presses `L`.
fn open_logs_for_selection(app: &mut AppState) -> Vec<Effect> {
    reduce(
        app,
        &Action::Resources(ResourcesAction::OpenLogsForSelection),
    )
}

/// Asserts that the jump entered the Logging workspace with exactly one
/// `LoadRecentLogs` effect carrying `query`.
fn assert_log_jump(app: &AppState, effects: &[Effect], query: LogQuery) {
    assert_eq!(app.root.workspace, Workspace::Logging);
    let context = app.root.current_context().expect("context exists");
    assert_eq!(
        effects,
        [Effect::LoadRecentLogs {
            context,
            query: query.clone(),
            severity: app.root.logging.severity_floor,
            range: app.root.logging.time_range,
        }]
    );
    assert_eq!(app.root.logging.query, query);
}

// ---------------------------------------------------------------------------
// per-service selection resolution
// ---------------------------------------------------------------------------

fn app_with_compute() -> AppState {
    let mut app = app_with_context(Some("dev-project"));
    let context = app.root.current_context().expect("context exists");
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ComputeInstancesLoaded {
            context,
            instances: vec![instance("api-dev-1")],
        }),
    );
    app
}

#[test]
fn selection_logs_for_compute_instance_uses_instance_identity() {
    let mut app = app_with_compute();
    let effects = open_logs_for_selection(&mut app);
    assert_log_jump(
        &app,
        &effects,
        LogQuery::Resource(LogResourceKind::ComputeInstance {
            instance_id: "api-dev-1-id".to_string(),
            instance_name: "api-dev-1".to_string(),
            zone: "europe-west1-b".to_string(),
        }),
    );
}

fn app_with_gke() -> AppState {
    let mut app = app_with_context(Some("dev-project"));
    let context = app.root.current_context().expect("context exists");
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::GkeClustersLoaded {
            context,
            clusters: vec![gke_cluster("platform-dev")],
        }),
    );
    reduce(
        &mut app,
        &Action::Resources(ResourcesAction::SelectService(
            ResourceServiceKind::GkeClusters,
        )),
    );
    app
}

#[test]
fn selection_logs_for_gke_cluster_uses_cluster_identity() {
    let mut app = app_with_gke();
    let effects = open_logs_for_selection(&mut app);
    assert_log_jump(
        &app,
        &effects,
        LogQuery::Resource(LogResourceKind::GkeCluster {
            cluster_name: "platform-dev".to_string(),
            location: "europe-west1".to_string(),
            project: "dev-project".to_string(),
        }),
    );
}

fn app_with_cloud_sql(project: Option<&str>) -> AppState {
    let mut app = app_with_context(project);
    let context = app.root.current_context().expect("context exists");
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::CloudSqlInstancesLoaded {
            context,
            instances: vec![cloud_sql_instance("orders-db")],
        }),
    );
    reduce(
        &mut app,
        &Action::Resources(ResourcesAction::SelectService(
            ResourceServiceKind::CloudSql,
        )),
    );
    app
}

/// The composite `database_id` filter is only correct when the project comes
/// from the *active* context; a wrong project matches zero rows silently.
#[test]
fn selection_logs_for_cloud_sql_instance_carries_the_active_project() {
    let mut app = app_with_cloud_sql(Some("dev-project"));
    let effects = open_logs_for_selection(&mut app);
    assert_log_jump(
        &app,
        &effects,
        LogQuery::Resource(LogResourceKind::CloudSqlInstance {
            instance_name: "orders-db".to_string(),
            project: "dev-project".to_string(),
        }),
    );
    assert!(
        app.root
            .logging
            .query
            .render_filter()
            .contains(r#""dev-project:orders-db""#)
    );
}

/// A context with no project cannot produce a correct `database_id`, so the
/// jump must refuse loudly instead of shipping a filter that matches nothing.
#[test]
fn selection_logs_for_cloud_sql_without_project_refuses_and_reports() {
    let mut app = app_with_cloud_sql(None);
    let effects = open_logs_for_selection(&mut app);

    assert!(effects.is_empty());
    assert_eq!(app.root.workspace, Workspace::Resources);
    assert_eq!(
        app.root.system.status_message.as_deref(),
        Some("Active gcloud context has no project; cannot scope Cloud SQL logs")
    );
}

fn app_with_cloud_functions(project: Option<&str>) -> AppState {
    let mut app = app_with_context(project);
    let context = app.root.current_context().expect("context exists");
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::CloudFunctionsLoaded {
            context,
            functions: vec![CloudFunction {
                name: "thumbnailer".to_string(),
                region: "europe-west1".to_string(),
                state: CloudFunctionState::Active,
                environment: CloudFunctionEnvironment::Gen2,
                url: None,
                update_time: None,
                labels: Vec::new(),
            }],
        }),
    );
    reduce(
        &mut app,
        &Action::Resources(ResourcesAction::SelectService(
            ResourceServiceKind::CloudFunctions,
        )),
    );
    app
}

/// The function's identity is its name *and* its region: the same name in
/// another region is another function, and a query that dropped the region
/// would show both.
#[test]
fn selection_logs_for_cloud_function_use_name_and_region() {
    let mut app = app_with_cloud_functions(Some("dev-project"));
    let effects = open_logs_for_selection(&mut app);
    assert_log_jump(
        &app,
        &effects,
        LogQuery::Resource(LogResourceKind::CloudFunction {
            function_name: "thumbnailer".to_string(),
            region: "europe-west1".to_string(),
        }),
    );
}

/// Unlike Cloud SQL and Secret Manager, the project is not interpolated into
/// the filter, so a context without one still produces a usable jump rather
/// than a refusal.
#[test]
fn selection_logs_for_cloud_function_without_a_project_still_jump() {
    let mut app = app_with_cloud_functions(None);
    let effects = open_logs_for_selection(&mut app);
    assert_log_jump(
        &app,
        &effects,
        LogQuery::Resource(LogResourceKind::CloudFunction {
            function_name: "thumbnailer".to_string(),
            region: "europe-west1".to_string(),
        }),
    );
}

/// With nothing under the cursor the jump refuses by name, so the message says
/// which resource it was looking for.
#[test]
fn selection_logs_for_cloud_functions_with_no_row_refuses_by_name() {
    let mut app = app_with_context(Some("dev-project"));
    reduce(
        &mut app,
        &Action::Resources(ResourcesAction::SelectService(
            ResourceServiceKind::CloudFunctions,
        )),
    );

    let effects = open_logs_for_selection(&mut app);

    assert!(effects.is_empty());
    assert_eq!(app.root.workspace, Workspace::Resources);
    assert_eq!(
        app.root.system.status_message.as_deref(),
        Some("No Cloud Function selected to open logs for")
    );
}

fn app_with_secrets(project: Option<&str>) -> AppState {
    let mut app = app_with_context(project);
    let context = app.root.current_context().expect("context exists");
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::SecretsLoaded {
            context,
            secrets: vec![Secret {
                name: "api-signing-key".to_string(),
                create_time: Some("2026-02-11T09:14:22Z".to_string()),
                labels: Vec::new(),
                replication: "Automatic".to_string(),
            }],
        }),
    );
    reduce(
        &mut app,
        &Action::Resources(ResourcesAction::SelectService(
            ResourceServiceKind::SecretManagerSecrets,
        )),
    );
    app
}

#[test]
fn selection_logs_for_secret_carries_the_active_project() {
    let mut app = app_with_secrets(Some("dev-project"));
    let effects = open_logs_for_selection(&mut app);
    assert_log_jump(
        &app,
        &effects,
        LogQuery::Resource(LogResourceKind::Secret {
            secret_name: "api-signing-key".to_string(),
            project: "dev-project".to_string(),
        }),
    );
}

/// A secret's audit entries are matched by the full resource name, project
/// included, so an empty project builds a filter that cannot match: the jump
/// must refuse loudly rather than land the operator on an empty log view.
#[test]
fn selection_logs_for_secret_without_project_refuses_and_reports() {
    let mut app = app_with_secrets(None);
    let effects = open_logs_for_selection(&mut app);

    assert!(effects.is_empty());
    assert_eq!(app.root.workspace, Workspace::Resources);
    assert_eq!(
        app.root.system.status_message.as_deref(),
        Some("Active gcloud context has no project; cannot scope Secret Manager logs")
    );
}

fn app_with_cloud_run() -> AppState {
    let mut app = app_with_context(Some("dev-project"));
    let context = app.root.current_context().expect("context exists");
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::CloudRunServicesLoaded {
            context,
            services: vec![cloud_run_service("api", "us-central1")],
        }),
    );
    reduce(
        &mut app,
        &Action::Resources(ResourcesAction::SelectService(
            ResourceServiceKind::CloudRunServices,
        )),
    );
    app
}

#[test]
fn selection_logs_for_cloud_run_service_uses_service_identity() {
    let mut app = app_with_cloud_run();
    let effects = open_logs_for_selection(&mut app);
    assert_log_jump(
        &app,
        &effects,
        LogQuery::Resource(LogResourceKind::CloudRunService {
            service_name: "api".to_string(),
            region: "us-central1".to_string(),
            project: "dev-project".to_string(),
        }),
    );
}

/// Drills into the Revisions tab and opens `api-00002`.
fn app_inside_cloud_run_revision() -> AppState {
    let mut app = app_with_cloud_run();
    reduce(&mut app, &Action::Resources(ResourcesAction::OpenSelected));
    let context = app.root.current_context().expect("context exists");
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::CloudRunServiceDetailLoaded {
            context,
            detail: cloud_run_detail(
                "api",
                "us-central1",
                vec![revision("api-00001"), revision("api-00002")],
            ),
        }),
    );
    // Summary -> Traffic -> Revisions.
    reduce(&mut app, &Action::Resources(ResourcesAction::SelectNextTab));
    reduce(&mut app, &Action::Resources(ResourcesAction::SelectNextTab));
    reduce(&mut app, &Action::Resources(ResourcesAction::SelectNext));
    reduce(&mut app, &Action::Resources(ResourcesAction::OpenSelected));
    assert!(app.root.resources.is_showing_cloud_run_revision());
    app
}

#[test]
fn selection_logs_inside_revision_view_uses_the_revision_not_the_service() {
    let mut app = app_inside_cloud_run_revision();
    let effects = open_logs_for_selection(&mut app);
    assert_log_jump(
        &app,
        &effects,
        LogQuery::Resource(LogResourceKind::CloudRunRevision {
            service_name: "api".to_string(),
            revision_name: "api-00002".to_string(),
            region: "us-central1".to_string(),
            project: "dev-project".to_string(),
        }),
    );
}

fn app_with_storage() -> AppState {
    let mut app = app_with_context(Some("dev-project"));
    let context = app.root.current_context().expect("context exists");
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
                storage_bucket("assets-bucket"),
                storage_bucket("logs-bucket"),
            ],
        }),
    );
    app
}

#[test]
fn selection_logs_for_storage_uses_the_selected_bucket() {
    let mut app = app_with_storage();
    let effects = open_logs_for_selection(&mut app);
    assert_log_jump(
        &app,
        &effects,
        LogQuery::Resource(LogResourceKind::StorageBucket {
            bucket_name: "assets-bucket".to_string(),
        }),
    );
}

#[test]
fn selection_logs_inside_object_browser_uses_the_browsed_bucket() {
    let mut app = app_with_storage();
    reduce(&mut app, &Action::Resources(ResourcesAction::SelectNext));
    reduce(&mut app, &Action::Resources(ResourcesAction::OpenSelected));
    assert_eq!(
        app.root.resources.storage().view(),
        StorageView::ObjectBrowser
    );

    let effects = open_logs_for_selection(&mut app);
    assert_log_jump(
        &app,
        &effects,
        LogQuery::Resource(LogResourceKind::StorageBucket {
            bucket_name: "logs-bucket".to_string(),
        }),
    );
}

/// Leaving the object browser does not clear the browsed bucket, so the
/// resolver must key off the *view*, not off a lingering browser target.
#[test]
fn selection_logs_after_leaving_object_browser_uses_the_list_selection() {
    let mut app = app_with_storage();
    reduce(&mut app, &Action::Resources(ResourcesAction::OpenSelected));
    reduce(&mut app, &Action::Resources(ResourcesAction::AscendPrefix));
    assert_eq!(app.root.resources.storage().view(), StorageView::Buckets);
    reduce(&mut app, &Action::Resources(ResourcesAction::SelectNext));

    let effects = open_logs_for_selection(&mut app);
    assert_log_jump(
        &app,
        &effects,
        LogQuery::Resource(LogResourceKind::StorageBucket {
            bucket_name: "logs-bucket".to_string(),
        }),
    );
}

fn bigquery_dataset(dataset_id: &str) -> BigQueryDataset {
    BigQueryDataset {
        dataset_id: dataset_id.to_string(),
        project_id: "dev-project".to_string(),
        location: "europe-west1".to_string(),
        friendly_name: None,
        labels: Vec::new(),
    }
}

fn app_with_bigquery_datasets() -> AppState {
    let mut app = app_with_context(Some("dev-project"));
    let context = app.root.current_context().expect("context exists");
    reduce(
        &mut app,
        &Action::Resources(ResourcesAction::SelectService(
            ResourceServiceKind::BigQueryDatasets,
        )),
    );
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::BigQueryDatasetsLoaded {
            context,
            datasets: vec![
                bigquery_dataset("alpha_events"),
                bigquery_dataset("beta_events"),
            ],
        }),
    );
    app
}

/// The scope is the dataset under the cursor, keyed by its id.
///
/// Both halves of that are pinned by moving the cursor first: the query has to
/// name `beta_events` rather than the first row, and the id rather than the
/// dataset's `project_id`, which every row here shares.
#[test]
fn selection_logs_for_bigquery_dataset_use_the_selected_dataset_id() {
    let mut app = app_with_bigquery_datasets();
    reduce(&mut app, &Action::Resources(ResourcesAction::SelectNext));
    let effects = open_logs_for_selection(&mut app);
    assert_log_jump(
        &app,
        &effects,
        LogQuery::Resource(LogResourceKind::BigQueryDataset {
            dataset_id: "beta_events".to_string(),
        }),
    );
}

/// With nothing under the cursor the jump refuses by name, the way every other
/// list-only service does.
#[test]
fn selection_logs_for_bigquery_datasets_with_no_row_refuses_by_name() {
    let mut app = app_with_context(Some("dev-project"));
    reduce(
        &mut app,
        &Action::Resources(ResourcesAction::SelectService(
            ResourceServiceKind::BigQueryDatasets,
        )),
    );

    let effects = open_logs_for_selection(&mut app);

    assert!(effects.is_empty());
    assert_eq!(app.root.workspace, Workspace::Resources);
    assert_eq!(
        app.root.system.status_message.as_deref(),
        Some("No BigQuery dataset selected to open logs for")
    );
}

// ---------------------------------------------------------------------------
// refusal paths (G4/G10: no silent no-ops, no UI state that lies)
// ---------------------------------------------------------------------------

#[test]
fn selection_logs_with_nothing_selected_reports_and_stays_put() {
    let mut app = app_with_context(Some("dev-project"));
    let context = app.root.current_context().expect("context exists");
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ComputeInstancesLoaded {
            context,
            instances: Vec::new(),
        }),
    );

    let effects = open_logs_for_selection(&mut app);

    assert!(effects.is_empty());
    assert_eq!(app.root.workspace, Workspace::Resources);
    assert_eq!(
        app.root.system.status_message.as_deref(),
        Some("No GCE instance selected to open logs for")
    );
}

#[test]
fn selection_logs_without_context_reports_and_stays_put() {
    let mut app = AppState::new();
    reduce(&mut app, &Action::Workspace(WorkspaceAction::Resources));

    let effects = open_logs_for_selection(&mut app);

    assert!(effects.is_empty());
    assert_eq!(app.root.workspace, Workspace::Resources);
    assert_eq!(
        app.root.system.status_message.as_deref(),
        Some("No active gcloud context to open logs for")
    );
}

// ---------------------------------------------------------------------------
// navigation history (G5)
// ---------------------------------------------------------------------------

#[test]
fn back_after_selection_log_jump_returns_to_the_originating_resource_view() {
    let mut app = app_with_cloud_sql(Some("dev-project"));
    open_logs_for_selection(&mut app);
    assert_eq!(app.root.workspace, Workspace::Logging);
    assert!(app.root.navigation.can_go_back());

    reduce(&mut app, &Action::Navigation(NavAction::Back));

    assert_eq!(app.root.workspace, Workspace::Resources);
    assert_eq!(
        app.root.resources.active_service(),
        ResourceServiceKind::CloudSql
    );
}

#[test]
fn back_after_a_palette_log_jump_returns_to_the_originating_resource_view() {
    let mut app = app_with_compute();
    reduce(&mut app, &Action::Command(CommandAction::Open));
    for ch in CommandDescriptor::OpenLogsForSelected(ResourceServiceKind::ComputeInstances)
        .label()
        .chars()
    {
        reduce(&mut app, &Action::Command(CommandAction::AppendChar(ch)));
    }
    reduce(&mut app, &Action::Command(CommandAction::Submit));
    assert_eq!(app.root.workspace, Workspace::Logging);

    reduce(&mut app, &Action::Navigation(NavAction::Back));

    assert_eq!(app.root.workspace, Workspace::Resources);
}

/// A palette command that does not navigate must not push a history entry —
/// the central `before != after` guard is what keeps `Submit` honest.
#[test]
fn a_non_navigating_palette_command_records_no_history() {
    let mut app = app_with_compute();
    let history_before = app.root.navigation.clone();
    reduce(&mut app, &Action::Command(CommandAction::Open));
    for ch in CommandDescriptor::CopySelectedInstanceName.label().chars() {
        reduce(&mut app, &Action::Command(CommandAction::AppendChar(ch)));
    }
    reduce(&mut app, &Action::Command(CommandAction::Submit));

    assert_eq!(app.root.navigation, history_before);
    assert_eq!(app.root.workspace, Workspace::Resources);
}

// ---------------------------------------------------------------------------
// palette parity (the new descriptors route through the same resolver)
// ---------------------------------------------------------------------------

fn submit_palette_command(app: &mut AppState, descriptor: CommandDescriptor) -> Vec<Effect> {
    reduce(app, &Action::Command(CommandAction::Open));
    for ch in descriptor.label().chars() {
        reduce(app, &Action::Command(CommandAction::AppendChar(ch)));
    }
    assert_eq!(
        app.root.command.selected_visible_command(&app.root),
        Some(descriptor)
    );
    reduce(app, &Action::Command(CommandAction::Submit))
}

#[test]
fn palette_offers_logs_for_the_selected_cloud_sql_instance() {
    let mut app = app_with_cloud_sql(Some("dev-project"));
    let effects = submit_palette_command(
        &mut app,
        CommandDescriptor::OpenLogsForSelected(ResourceServiceKind::CloudSql),
    );
    assert_log_jump(
        &app,
        &effects,
        LogQuery::Resource(LogResourceKind::CloudSqlInstance {
            instance_name: "orders-db".to_string(),
            project: "dev-project".to_string(),
        }),
    );
}

/// With no project in the active context the reducer can only refuse the Cloud
/// SQL jump (the filter's composite `<project>:<instance>` `database_id` would
/// match zero rows), so the palette must not offer it — same gate as the
/// sibling connection-name and tunnel commands, which need the same project.
#[test]
fn palette_hides_cloud_sql_logs_when_the_context_has_no_project() {
    let app = app_with_cloud_sql(None);
    let commands = app.root.command.visible_commands(&app.root);

    assert!(
        !commands.contains(&CommandDescriptor::OpenLogsForSelected(
            ResourceServiceKind::CloudSql
        )),
        "the palette must not offer a log jump the reducer can only refuse: {commands:?}"
    );
    assert!(
        !commands.contains(&CommandDescriptor::CopySelectedCloudSqlConnectionName),
        "sibling commands needing the same project stay hidden too: {commands:?}"
    );
}

#[test]
fn palette_offers_logs_for_the_selected_storage_bucket() {
    let mut app = app_with_storage();
    let effects = submit_palette_command(
        &mut app,
        CommandDescriptor::OpenLogsForSelected(ResourceServiceKind::CloudStorage),
    );
    assert_log_jump(
        &app,
        &effects,
        LogQuery::Resource(LogResourceKind::StorageBucket {
            bucket_name: "assets-bucket".to_string(),
        }),
    );
}

#[test]
fn palette_offers_logs_for_the_open_cloud_run_revision() {
    let mut app = app_inside_cloud_run_revision();
    let effects = submit_palette_command(
        &mut app,
        CommandDescriptor::OpenLogsForSelectedCloudRunRevision,
    );
    assert_log_jump(
        &app,
        &effects,
        LogQuery::Resource(LogResourceKind::CloudRunRevision {
            service_name: "api".to_string(),
            revision_name: "api-00002".to_string(),
            region: "us-central1".to_string(),
            project: "dev-project".to_string(),
        }),
    );
}

// ---------------------------------------------------------------------------
// detail panes: the pane's target, not the list cursor behind it (G10)
// ---------------------------------------------------------------------------

/// With a detail pane open, `j`/`k` still move the list cursor behind it while
/// the pane keeps showing its own target. Scoping the jump to the list cursor
/// would open logs for a resource that is not on screen.
#[test]
fn selection_logs_in_a_detail_pane_target_the_pane_not_the_list_cursor() {
    let mut app = app_with_context(Some("dev-project"));
    let context = app.root.current_context().expect("context exists");
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ComputeInstancesLoaded {
            context,
            instances: vec![instance("api-dev-1"), instance("worker-dev-1")],
        }),
    );
    reduce(&mut app, &Action::Resources(ResourcesAction::OpenSelected));
    assert!(app.root.resources.is_showing_detail());
    reduce(&mut app, &Action::Resources(ResourcesAction::SelectNext));
    assert_eq!(
        app.root.resources.detail_target_name(),
        Some("api-dev-1"),
        "the pane still shows the instance it was opened on"
    );

    let effects = open_logs_for_selection(&mut app);

    assert_log_jump(
        &app,
        &effects,
        LogQuery::Resource(LogResourceKind::ComputeInstance {
            instance_id: "api-dev-1-id".to_string(),
            instance_name: "api-dev-1".to_string(),
            zone: "europe-west1-b".to_string(),
        }),
    );
}

#[test]
fn selection_logs_in_a_gke_detail_pane_target_the_pane_not_the_list_cursor() {
    let mut app = app_with_context(Some("dev-project"));
    let context = app.root.current_context().expect("context exists");
    let mut other = gke_cluster("web-dev");
    other.location = "us-east1".to_string();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::GkeClustersLoaded {
            context,
            clusters: vec![gke_cluster("platform-dev"), other],
        }),
    );
    reduce(
        &mut app,
        &Action::Resources(ResourcesAction::SelectService(
            ResourceServiceKind::GkeClusters,
        )),
    );
    reduce(&mut app, &Action::Resources(ResourcesAction::OpenSelected));
    assert!(app.root.resources.is_showing_detail());
    reduce(&mut app, &Action::Resources(ResourcesAction::SelectNext));
    assert_eq!(
        app.root.resources.gke_detail_target_name(),
        Some("platform-dev"),
        "the pane still shows the cluster it was opened on"
    );

    let effects = open_logs_for_selection(&mut app);

    assert_log_jump(
        &app,
        &effects,
        LogQuery::Resource(LogResourceKind::GkeCluster {
            cluster_name: "platform-dev".to_string(),
            location: "europe-west1".to_string(),
            project: "dev-project".to_string(),
        }),
    );
}

#[test]
fn selection_logs_in_a_cloud_sql_detail_pane_target_the_pane_not_the_list_cursor() {
    let mut app = app_with_context(Some("dev-project"));
    let context = app.root.current_context().expect("context exists");
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::CloudSqlInstancesLoaded {
            context,
            instances: vec![
                cloud_sql_instance("orders-db"),
                cloud_sql_instance("reports-db"),
            ],
        }),
    );
    reduce(
        &mut app,
        &Action::Resources(ResourcesAction::SelectService(
            ResourceServiceKind::CloudSql,
        )),
    );
    reduce(&mut app, &Action::Resources(ResourcesAction::OpenSelected));
    assert!(app.root.resources.is_showing_detail());
    reduce(&mut app, &Action::Resources(ResourcesAction::SelectNext));
    assert_eq!(
        app.root.resources.cloud_sql_detail_target_name(),
        Some("orders-db"),
        "the pane still shows the instance it was opened on"
    );

    let effects = open_logs_for_selection(&mut app);

    assert_log_jump(
        &app,
        &effects,
        LogQuery::Resource(LogResourceKind::CloudSqlInstance {
            instance_name: "orders-db".to_string(),
            project: "dev-project".to_string(),
        }),
    );
}

#[test]
fn selection_logs_in_a_cloud_run_detail_pane_target_the_pane_not_the_list_cursor() {
    let mut app = app_with_context(Some("dev-project"));
    let context = app.root.current_context().expect("context exists");
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::CloudRunServicesLoaded {
            context,
            services: vec![
                cloud_run_service("api", "us-central1"),
                cloud_run_service("worker", "europe-west1"),
            ],
        }),
    );
    reduce(
        &mut app,
        &Action::Resources(ResourcesAction::SelectService(
            ResourceServiceKind::CloudRunServices,
        )),
    );
    reduce(&mut app, &Action::Resources(ResourcesAction::OpenSelected));
    assert!(app.root.resources.is_showing_detail());
    assert!(!app.root.resources.is_showing_cloud_run_revision());
    reduce(&mut app, &Action::Resources(ResourcesAction::SelectNext));
    assert_eq!(
        app.root.resources.cloud_run_detail_target_name(),
        Some("api"),
        "the pane still shows the service it was opened on"
    );

    let effects = open_logs_for_selection(&mut app);

    assert_log_jump(
        &app,
        &effects,
        LogQuery::Resource(LogResourceKind::CloudRunService {
            service_name: "api".to_string(),
            region: "us-central1".to_string(),
            project: "dev-project".to_string(),
        }),
    );
}

#[test]
fn selection_logs_in_a_bucket_metadata_pane_target_the_pane_not_the_list_cursor() {
    let mut app = app_with_storage();
    reduce(
        &mut app,
        &Action::Resources(ResourcesAction::OpenBucketMetadata),
    );
    assert_eq!(
        app.root.resources.storage().view(),
        StorageView::BucketDetail
    );
    reduce(&mut app, &Action::Resources(ResourcesAction::SelectNext));
    assert_eq!(
        app.root.resources.storage().detail_target_name(),
        Some("assets-bucket"),
        "the pane still shows the bucket it was opened on"
    );

    let effects = open_logs_for_selection(&mut app);

    assert_log_jump(
        &app,
        &effects,
        LogQuery::Resource(LogResourceKind::StorageBucket {
            bucket_name: "assets-bucket".to_string(),
        }),
    );
}

/// One `L` press is one navigation, so `Back` must land on the originating
/// resource view with nothing left behind it — a second recorded entry would
/// strand the user mid-history on a view they never visited.
#[test]
fn a_single_selection_log_jump_records_exactly_one_history_entry() {
    let mut app = app_with_compute();
    app.root.navigation.reset();

    open_logs_for_selection(&mut app);
    assert_eq!(app.root.workspace, Workspace::Logging);
    assert!(app.root.navigation.can_go_back());

    reduce(&mut app, &Action::Navigation(NavAction::Back));

    assert_eq!(app.root.workspace, Workspace::Resources);
    assert!(
        !app.root.navigation.can_go_back(),
        "one `L` press must record exactly one history entry"
    );
}
