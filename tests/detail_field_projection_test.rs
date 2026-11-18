//! `project_detail_body` must pick the same body the detail renderer draws,
//! for every service and for the Cloud Run revision drill-down that layers on
//! top of it.

use nuvem::app::projection::detail_body::{DetailBody, project_detail_body};
use nuvem::app::reducer::root::reduce;
use nuvem::app::state::resources::{CloudRunRevisionTab, StorageView};
use nuvem::app::{Action, AppError, AppState, EffectResult, ResourcesAction};
use nuvem::services::resources::storage::{parse_bucket_detail_json, parse_object_detail_json};
use nuvem_domain::gcp::configuration::{GcloudConfiguration, GcloudConfigurationName};
use nuvem_domain::gcp::resource::{
    CloudRunRevision, CloudRunRevisionDetail, CloudRunService, CloudRunServiceDetail,
    CloudRunServiceStatus, CloudSqlAvailability, CloudSqlBackupConfig, CloudSqlInstance,
    CloudSqlInstanceDetail, CloudSqlStatus, ComputeDisk, ComputeInstance, ComputeInstanceDetail,
    ComputeKeyValue, ComputeNetworkInterface, EndpointVisibility, GkeAddon, GkeCluster,
    GkeClusterDetail, GkeClusterStatus, GkeNodePool, ResourceServiceKind, ResourceStatus, Secret,
    SecretDetail, StorageBucket, StorageObject, StorageObjectPage,
};

fn key_value(key: &str, value: &str) -> ComputeKeyValue {
    ComputeKeyValue {
        key: key.to_string(),
        value: value.to_string(),
    }
}

fn config(name: &str, active: bool, project: &str) -> GcloudConfiguration {
    GcloudConfiguration {
        name: GcloudConfigurationName::new(name),
        is_active: active,
        account: Some(format!("{name}@example.com")),
        project: Some(project.to_string()),
        region: Some("europe-west1".to_string()),
        zone: Some("europe-west1-b".to_string()),
    }
}

fn instance(name: &str) -> ComputeInstance {
    ComputeInstance {
        id: name.to_string(),
        name: name.to_string(),
        zone: "europe-west1-b".to_string(),
        status: ResourceStatus::Running,
        machine_type: "e2-medium".to_string(),
        internal_ip: Some("10.0.0.4".to_string()),
        external_ip: Some("34.10.10.10".to_string()),
    }
}

fn compute_detail(name: &str) -> ComputeInstanceDetail {
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
        labels: Vec::new(),
        metadata: Vec::new(),
        raw_json: format!(r#"{{"name":"{name}"}}"#),
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
        node_pools: vec![GkeNodePool {
            name: "default-pool".to_string(),
            status: GkeClusterStatus::Running,
            version: Some("1.31.5-gke.1023000".to_string()),
            machine_type: Some("e2-medium".to_string()),
            autoscaling: None,
            node_count: Some(3),
        }],
        addons: vec![GkeAddon {
            name: "HttpLoadBalancing".to_string(),
            enabled: true,
        }],
        labels: Vec::new(),
        raw_json: format!(r#"{{"name":"{name}"}}"#),
    }
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

fn cloud_run_service(name: &str, region: &str) -> CloudRunService {
    CloudRunService {
        name: name.to_string(),
        region: region.to_string(),
        url: Some(format!("https://{name}.a.run.app")),
        latest_ready_revision: Some(format!("{name}-00001")),
        latest_created_revision: Some(format!("{name}-00001")),
        traffic_summary: "100% -> latest".to_string(),
        status: CloudRunServiceStatus::Ready,
        ready_message: None,
    }
}

fn cloud_run_detail(name: &str, region: &str) -> CloudRunServiceDetail {
    CloudRunServiceDetail {
        name: name.to_string(),
        region: region.to_string(),
        url: Some(format!("https://{name}.a.run.app")),
        status: CloudRunServiceStatus::Ready,
        latest_ready_revision: Some(format!("{name}-00001")),
        latest_created_revision: Some(format!("{name}-00001")),
        traffic: Vec::new(),
        revisions: vec![CloudRunRevision {
            name: format!("{name}-00001"),
            ready: CloudRunServiceStatus::Ready,
            created: None,
            image: Some("gcr.io/dev/api:latest".to_string()),
            active_percent: Some(100),
        }],
        conditions: Vec::new(),
        image: Some("gcr.io/dev/api:latest".to_string()),
        cpu: Some("1".to_string()),
        memory: Some("512Mi".to_string()),
        concurrency: Some(80),
        service_account: None,
        ingress: Some("all".to_string()),
        min_instances: Some(0),
        max_instances: Some(100),
        labels: Vec::new(),
        raw_json: format!(r#"{{"name":"{name}"}}"#),
    }
}

fn cloud_run_revision_detail(name: &str, region: &str) -> CloudRunRevisionDetail {
    CloudRunRevisionDetail {
        name: name.to_string(),
        service: Some("api".to_string()),
        region: region.to_string(),
        ready: CloudRunServiceStatus::Ready,
        conditions: Vec::new(),
        image: Some("gcr.io/dev/api:revision".to_string()),
        cpu: None,
        memory: None,
        concurrency: None,
        min_instances: None,
        max_instances: None,
        service_account: None,
        created: None,
        raw_json: format!(r#"{{"name":"{name}"}}"#),
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

/// AppState with an active configuration and Compute rows loaded.
fn app_with_gce_rows() -> AppState {
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
    app
}

fn select_service(app: &mut AppState, service: ResourceServiceKind) {
    reduce(
        app,
        &Action::Resources(ResourcesAction::SelectService(service)),
    );
}

fn open_selected(app: &mut AppState) {
    reduce(app, &Action::Resources(ResourcesAction::OpenSelected));
}

fn next_tab(app: &mut AppState) {
    reduce(app, &Action::Resources(ResourcesAction::SelectNextTab));
}

/// Asserts the body exposes `label`/`value` as a copyable field.
fn assert_copyable(body: &DetailBody, label: &str, value: &str) {
    let fields = body.fields();
    assert!(
        fields
            .iter()
            .any(|(_, field)| field.label == label && field.value == value),
        "expected copyable {label}={value}, got: {fields:?}"
    );
}

#[test]
fn no_detail_open_projects_no_body() {
    let app = app_with_gce_rows();
    assert!(project_detail_body(&app.root.resources).is_none());
}

#[test]
fn compute_detail_still_loading_projects_no_body() {
    let mut app = app_with_gce_rows();
    open_selected(&mut app);

    assert!(app.root.resources.is_showing_detail());
    assert!(project_detail_body(&app.root.resources).is_none());
}

#[test]
fn compute_detail_error_projects_no_body() {
    let mut app = app_with_gce_rows();
    open_selected(&mut app);
    let context = app.root.current_context().unwrap().clone();
    // The payload lands first, so the body below is real and the error has
    // something to take precedence over.
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ComputeInstanceDetailLoaded {
            context: context.clone(),
            detail: compute_detail("api-dev-1"),
        }),
    );
    assert!(project_detail_body(&app.root.resources).is_some());

    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ComputeInstanceDetailLoadFailed {
            context,
            instance_name: "api-dev-1".to_string(),
            instance_zone: "europe-west1-b".to_string(),
            error: AppError::gcloud("describe failed"),
        }),
    );

    assert!(app.root.resources.detail_error().is_some());
    assert!(project_detail_body(&app.root.resources).is_none());
}

#[test]
fn compute_detail_projects_body_for_the_active_tab() {
    let mut app = app_with_gce_rows();
    open_selected(&mut app);
    let context = app.root.current_context().unwrap().clone();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ComputeInstanceDetailLoaded {
            context,
            detail: compute_detail("api-dev-1"),
        }),
    );
    // Summary -> Network.
    next_tab(&mut app);

    let body = project_detail_body(&app.root.resources).expect("compute detail body");
    assert_copyable(&body, "External IP", "34.10.10.10");
}

#[test]
fn gke_detail_projects_body_for_the_active_tab() {
    let mut app = app_with_gce_rows();
    select_service(&mut app, ResourceServiceKind::GkeClusters);
    let context = app.root.current_context().unwrap().clone();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::GkeClustersLoaded {
            context: context.clone(),
            clusters: vec![gke_cluster("payments", "europe-west1")],
        }),
    );
    open_selected(&mut app);
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::GkeClusterDetailLoaded {
            context,
            detail: gke_detail("payments", "europe-west1"),
        }),
    );
    // Summary -> Networking.
    next_tab(&mut app);

    let body = project_detail_body(&app.root.resources).expect("gke detail body");
    assert_copyable(&body, "Endpoint", "34.118.10.20");
}

#[test]
fn cloud_sql_detail_projects_body_for_the_active_tab() {
    let mut app = app_with_gce_rows();
    select_service(&mut app, ResourceServiceKind::CloudSql);
    let context = app.root.current_context().unwrap().clone();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::CloudSqlInstancesLoaded {
            context: context.clone(),
            instances: vec![cloud_sql_instance("db-dev-1")],
        }),
    );
    open_selected(&mut app);
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::CloudSqlInstanceDetailLoaded {
            context,
            detail: Box::new(cloud_sql_detail("db-dev-1")),
        }),
    );
    // Summary -> Connectivity.
    next_tab(&mut app);

    let body = project_detail_body(&app.root.resources).expect("cloud sql detail body");
    assert_copyable(&body, "Connection name", "dev-project:europe-west1:db");
}

#[test]
fn cloud_run_detail_projects_body_for_the_active_tab() {
    let mut app = app_with_gce_rows();
    select_service(&mut app, ResourceServiceKind::CloudRunServices);
    let context = app.root.current_context().unwrap().clone();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::CloudRunServicesLoaded {
            context: context.clone(),
            services: vec![cloud_run_service("api", "europe-west1")],
        }),
    );
    open_selected(&mut app);
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::CloudRunServiceDetailLoaded {
            context,
            detail: cloud_run_detail("api", "europe-west1"),
        }),
    );
    // Summary -> Traffic -> Revisions -> Configuration.
    next_tab(&mut app);
    next_tab(&mut app);
    next_tab(&mut app);

    let body = project_detail_body(&app.root.resources).expect("cloud run detail body");
    assert_copyable(&body, "Image", "gcr.io/dev/api:latest");
}

/// Opens the Cloud Run revision drill-down over a loaded service detail, with
/// the revision's own payload still in flight.
fn app_with_cloud_run_revision_pending() -> AppState {
    let mut app = app_with_gce_rows();
    select_service(&mut app, ResourceServiceKind::CloudRunServices);
    let context = app.root.current_context().unwrap().clone();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::CloudRunServicesLoaded {
            context: context.clone(),
            services: vec![cloud_run_service("api", "europe-west1")],
        }),
    );
    open_selected(&mut app);
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::CloudRunServiceDetailLoaded {
            context: context.clone(),
            detail: cloud_run_detail("api", "europe-west1"),
        }),
    );
    // Summary -> Traffic -> Revisions, then drill into the selected revision.
    next_tab(&mut app);
    next_tab(&mut app);
    open_selected(&mut app);
    app
}

/// The same drill-down with the revision detail delivered.
fn app_with_cloud_run_revision_open() -> AppState {
    let mut app = app_with_cloud_run_revision_pending();
    let context = app.root.current_context().unwrap().clone();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::CloudRunRevisionDetailLoaded {
            context,
            detail: cloud_run_revision_detail("api-00001", "europe-west1"),
        }),
    );
    app
}

#[test]
fn cloud_run_revision_drill_down_without_its_detail_projects_no_body() {
    let app = app_with_cloud_run_revision_pending();

    assert!(app.root.resources.is_showing_cloud_run_revision());
    // The service detail is still loaded underneath, but the drill-down owns
    // the pane: an empty pane, not the service body, until the revision lands.
    assert!(app.root.resources.cloud_run_detail().is_some());
    assert!(app.root.resources.cloud_run_revision_detail().is_none());
    assert!(project_detail_body(&app.root.resources).is_none());
}

#[test]
fn cloud_run_revision_drill_down_projects_the_revision_body() {
    let app = app_with_cloud_run_revision_open();

    assert!(app.root.resources.is_showing_cloud_run_revision());
    let body = project_detail_body(&app.root.resources).expect("cloud run revision body");
    // The revision's own image, not the service's.
    assert_copyable(&body, "Name", "api-00001");
    assert_copyable(&body, "Image", "gcr.io/dev/api:revision");
}

#[test]
fn cloud_run_revision_drill_down_follows_the_revision_tab() {
    let mut app = app_with_cloud_run_revision_open();
    // Summary -> Conditions -> Json. The revision view has its own tab cycle,
    // separate from the service detail's `SelectNextTab`.
    app.root.resources.select_next_cloud_run_revision_tab();
    app.root.resources.select_next_cloud_run_revision_tab();
    assert_eq!(
        app.root.resources.active_cloud_run_revision_tab(),
        CloudRunRevisionTab::Json
    );

    let body = project_detail_body(&app.root.resources).expect("cloud run revision body");
    assert_eq!(body.raw_json.as_deref(), Some(r#"{"name":"api-00001"}"#));
}

#[test]
fn cloud_run_revision_view_never_bleeds_into_another_services_pane() {
    let mut app = app_with_cloud_run_revision_open();
    // Leave Cloud Run for a service with its own open, loaded detail pane.
    select_service(&mut app, ResourceServiceKind::ComputeInstances);
    open_selected(&mut app);
    let context = app.root.current_context().unwrap().clone();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ComputeInstanceDetailLoaded {
            context,
            detail: compute_detail("api-dev-1"),
        }),
    );

    let body = project_detail_body(&app.root.resources).expect("compute detail body");
    assert_copyable(&body, "Name", "api-dev-1");
    assert!(
        !body
            .fields()
            .iter()
            .any(|(_, field)| field.value.contains("api-00001")),
        "the stale revision view must not reach the Compute pane, got: {:?}",
        body.fields()
    );
}

/// AppState with Cloud Storage selected and one bucket loaded.
fn app_with_buckets() -> AppState {
    let mut app = app_with_gce_rows();
    select_service(&mut app, ResourceServiceKind::CloudStorage);
    let context = app.root.current_context().unwrap().clone();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::StorageBucketsLoaded {
            context,
            buckets: vec![storage_bucket("alpha-bucket")],
        }),
    );
    app
}

#[test]
fn storage_bucket_detail_projects_the_bucket_body() {
    let mut app = app_with_buckets();
    reduce(
        &mut app,
        &Action::Resources(ResourcesAction::OpenBucketMetadata),
    );
    let context = app.root.current_context().unwrap().clone();
    let detail = parse_bucket_detail_json(include_str!("fixtures/gcs_bucket_detail.json"))
        .expect("bucket detail fixture should parse");
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::StorageBucketDetailLoaded { context, detail }),
    );

    assert_eq!(
        app.root.resources.storage().view(),
        StorageView::BucketDetail
    );
    let body = project_detail_body(&app.root.resources).expect("storage bucket detail body");
    assert_copyable(&body, "Name", "alpha-bucket");
    assert_copyable(&body, "Storage class", "STANDARD");
}

#[test]
fn storage_object_detail_projects_the_object_body() {
    let mut app = app_with_buckets();
    open_selected(&mut app);
    let context = app.root.current_context().unwrap().clone();
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
    open_selected(&mut app);
    let detail = parse_object_detail_json(include_str!("fixtures/gcs_object_detail.json"))
        .expect("object detail fixture should parse");
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::StorageObjectDetailLoaded { context, detail }),
    );

    assert_eq!(
        app.root.resources.storage().view(),
        StorageView::ObjectDetail
    );
    let body = project_detail_body(&app.root.resources).expect("storage object detail body");
    assert_copyable(&body, "Key", "reports/2024/mañana-测试.txt");
    assert_copyable(&body, "Generation", "1700000000000001");
}

/// The Labels tab of a secret carries both sections and makes each entry
/// copyable.
///
/// `secret_key_value_lines` survived being replaced with an empty vector: the
/// tab is the only place a secret's labels and annotations are readable, and an
/// empty body reads as a secret that has neither.
#[test]
fn secret_labels_tab_projects_labels_and_annotations() {
    let mut app = app_with_gce_rows();
    select_service(&mut app, ResourceServiceKind::SecretManagerSecrets);
    let context = app.root.current_context().unwrap().clone();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::SecretsLoaded {
            context: context.clone(),
            secrets: vec![Secret {
                name: "api-signing-key".to_string(),
                create_time: Some("2026-02-11T09:14:22Z".to_string()),
                labels: vec![key_value("env", "prod")],
                replication: "Automatic".to_string(),
            }],
        }),
    );
    open_selected(&mut app);
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::SecretDetailLoaded {
            context,
            detail: SecretDetail {
                name: "api-signing-key".to_string(),
                create_time: Some("2026-02-11T09:14:22Z".to_string()),
                labels: vec![key_value("env", "prod")],
                annotations: vec![key_value("owner", "platform")],
                replication: "Automatic".to_string(),
                rotation: None,
                expire_time: None,
                versions: Vec::new(),
                raw_json: r#"{"name":"api-signing-key"}"#.to_string(),
            },
        }),
    );
    // Summary -> Versions -> Labels.
    next_tab(&mut app);
    next_tab(&mut app);

    let body = project_detail_body(&app.root.resources).expect("secret detail body");
    let lines = body.display_lines();
    assert!(
        lines.iter().any(|line| line == "Labels:")
            && lines.iter().any(|line| line == "Annotations:"),
        "both sections must be headed, so an empty one is still identifiable: {lines:?}"
    );
    assert_copyable(&body, "env", "prod");
    assert_copyable(&body, "owner", "platform");
}

/// A secret with neither labels nor annotations still projects both headings
/// and an explicit "none" under each — the empty-vector mutant projects nothing
/// at all, which the operator cannot tell apart from a failed tab.
#[test]
fn secret_labels_tab_projects_none_for_empty_sections() {
    let mut app = app_with_gce_rows();
    select_service(&mut app, ResourceServiceKind::SecretManagerSecrets);
    let context = app.root.current_context().unwrap().clone();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::SecretsLoaded {
            context: context.clone(),
            secrets: vec![Secret {
                name: "bare-secret".to_string(),
                create_time: None,
                labels: Vec::new(),
                replication: "Automatic".to_string(),
            }],
        }),
    );
    open_selected(&mut app);
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::SecretDetailLoaded {
            context,
            detail: SecretDetail {
                name: "bare-secret".to_string(),
                create_time: None,
                labels: Vec::new(),
                annotations: Vec::new(),
                replication: "Automatic".to_string(),
                rotation: None,
                expire_time: None,
                versions: Vec::new(),
                raw_json: r#"{"name":"bare-secret"}"#.to_string(),
            },
        }),
    );
    next_tab(&mut app);
    next_tab(&mut app);

    let body = project_detail_body(&app.root.resources).expect("secret detail body");
    assert_eq!(
        body.display_lines(),
        vec![
            "Labels:".to_string(),
            "  none".to_string(),
            String::new(),
            "Annotations:".to_string(),
            "  none".to_string(),
        ],
        "an empty Labels tab must still say which section is empty"
    );
}
