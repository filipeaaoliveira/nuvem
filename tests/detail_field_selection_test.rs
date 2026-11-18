//! The detail-pane field cursor: `n`/`p` movement over the copyable fields of
//! the body currently on screen, `y` copying the selected field, and the
//! clearing of the cursor at every site where the visible field list changes.

use nuvem::app::projection::detail_body::project_detail_body;
use nuvem::app::reducer::root::reduce;
use nuvem::app::{Action, AppState, Effect, EffectResult, NavAction, ResourcesAction};
use nuvem::services::resources::storage::{parse_bucket_detail_json, parse_object_detail_json};
use nuvem_domain::gcp::configuration::{GcloudConfiguration, GcloudConfigurationName};
use nuvem_domain::gcp::resource::{
    CloudRunRevision, CloudRunRevisionDetail, CloudRunService, CloudRunServiceDetail,
    CloudRunServiceStatus, ComputeDisk, ComputeInstance, ComputeInstanceDetail, ComputeKeyValue,
    ComputeNetworkInterface, GkeCluster, GkeClusterStatus, ResourceServiceKind, ResourceStatus,
    StorageBucket, StorageObject, StorageObjectPage,
};

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
        raw_json: format!("{{\"name\":\"{name}\"}}"),
    }
}

fn gke_cluster(name: &str, location: &str) -> GkeCluster {
    GkeCluster {
        name: name.to_string(),
        location: location.to_string(),
        status: GkeClusterStatus::Running,
        current_master_version: Some("1.31.5-gke.1023000".to_string()),
        current_node_count: Some(3),
        endpoint_visibility: nuvem_domain::gcp::resource::EndpointVisibility::Public,
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
        raw_json: format!("{{\"name\":\"{name}\"}}"),
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
        raw_json: format!("{{\"name\":\"{name}\"}}"),
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

fn dispatch(app: &mut AppState, action: ResourcesAction) -> Vec<Effect> {
    reduce(app, &Action::Resources(action))
}

fn next_field(app: &mut AppState) -> Vec<Effect> {
    dispatch(app, ResourcesAction::SelectNextDetailField)
}

fn previous_field(app: &mut AppState) -> Vec<Effect> {
    dispatch(app, ResourcesAction::SelectPreviousDetailField)
}

fn field_index(app: &AppState) -> Option<usize> {
    app.root.resources.detail_field_index()
}

/// Number of copyable fields in the body currently on screen.
fn field_count(app: &AppState) -> usize {
    project_detail_body(&app.root.resources)
        .map(|body| body.fields().len())
        .unwrap_or(0)
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

/// AppState with the Compute detail pane open on a loaded instance.
fn app_with_compute_detail() -> AppState {
    let mut app = app_with_gce_rows();
    dispatch(&mut app, ResourcesAction::OpenSelected);
    let context = app.root.current_context().unwrap().clone();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ComputeInstanceDetailLoaded {
            context,
            detail: compute_detail("api-dev-1"),
        }),
    );
    app
}

/// AppState with the Cloud Run service detail open on the Revisions tab.
fn app_with_cloud_run_revisions_tab() -> AppState {
    let mut app = app_with_gce_rows();
    dispatch(
        &mut app,
        ResourcesAction::SelectService(ResourceServiceKind::CloudRunServices),
    );
    let context = app.root.current_context().unwrap().clone();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::CloudRunServicesLoaded {
            context: context.clone(),
            services: vec![cloud_run_service("api", "europe-west1")],
        }),
    );
    dispatch(&mut app, ResourcesAction::OpenSelected);
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::CloudRunServiceDetailLoaded {
            context,
            detail: cloud_run_detail("api", "europe-west1"),
        }),
    );
    // Summary -> Traffic -> Revisions.
    dispatch(&mut app, ResourcesAction::SelectNextTab);
    dispatch(&mut app, ResourcesAction::SelectNextTab);
    app
}

/// AppState with Cloud Storage selected and one bucket loaded.
fn app_with_buckets() -> AppState {
    let mut app = app_with_gce_rows();
    dispatch(
        &mut app,
        ResourcesAction::SelectService(ResourceServiceKind::CloudStorage),
    );
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

/// AppState with the Cloud Storage bucket metadata pane open and loaded.
fn app_with_bucket_detail() -> AppState {
    let mut app = app_with_buckets();
    dispatch(&mut app, ResourcesAction::OpenBucketMetadata);
    let context = app.root.current_context().unwrap().clone();
    let detail = parse_bucket_detail_json(include_str!("fixtures/gcs_bucket_detail.json"))
        .expect("bucket detail fixture should parse");
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::StorageBucketDetailLoaded { context, detail }),
    );
    app
}

/// AppState with the Cloud Storage object detail pane open and loaded.
fn app_with_object_detail() -> AppState {
    let mut app = app_with_buckets();
    dispatch(&mut app, ResourcesAction::OpenSelected);
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
    dispatch(&mut app, ResourcesAction::OpenSelected);
    let detail = parse_object_detail_json(include_str!("fixtures/gcs_object_detail.json"))
        .expect("object detail fixture should parse");
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::StorageObjectDetailLoaded { context, detail }),
    );
    app
}

// ---- cursor movement ----

#[test]
fn select_next_detail_field_from_no_selection_selects_the_first_field() {
    let mut app = app_with_compute_detail();
    assert_eq!(field_index(&app), None);

    let effects = next_field(&mut app);

    assert_eq!(field_index(&app), Some(0));
    assert!(effects.is_empty());
}

#[test]
fn select_previous_detail_field_from_no_selection_selects_the_last_field() {
    let mut app = app_with_compute_detail();
    let last = field_count(&app) - 1;

    previous_field(&mut app);

    assert_eq!(field_index(&app), Some(last));
}

#[test]
fn select_next_detail_field_wraps_past_the_last_field() {
    let mut app = app_with_compute_detail();
    let count = field_count(&app);
    for _ in 0..count {
        next_field(&mut app);
    }
    assert_eq!(field_index(&app), Some(count - 1));

    next_field(&mut app);

    assert_eq!(field_index(&app), Some(0));
}

#[test]
fn select_previous_detail_field_wraps_before_the_first_field() {
    let mut app = app_with_compute_detail();
    next_field(&mut app);
    assert_eq!(field_index(&app), Some(0));

    previous_field(&mut app);

    assert_eq!(field_index(&app), Some(field_count(&app) - 1));
}

#[test]
fn selecting_a_detail_field_is_a_no_op_without_a_detail_pane() {
    let mut app = app_with_gce_rows();

    let next_effects = next_field(&mut app);
    let previous_effects = previous_field(&mut app);

    assert_eq!(field_index(&app), None);
    assert!(next_effects.is_empty());
    assert!(previous_effects.is_empty());
}

#[test]
fn selecting_a_detail_field_is_a_no_op_on_a_body_without_copyable_fields() {
    let mut app = app_with_compute_detail();
    // Summary -> Network -> Disks -> Metadata; the fixture has no labels or
    // metadata, so the tab renders only headings and placeholders.
    for _ in 0..3 {
        dispatch(&mut app, ResourcesAction::SelectNextTab);
    }
    assert_eq!(field_count(&app), 0);

    next_field(&mut app);
    previous_field(&mut app);

    assert_eq!(field_index(&app), None);
}

#[test]
fn selecting_a_detail_field_is_a_no_op_on_a_json_tab() {
    let mut app = app_with_compute_detail();
    // Summary -> Network -> Disks -> Metadata -> Json.
    for _ in 0..4 {
        dispatch(&mut app, ResourcesAction::SelectNextTab);
    }
    let body = project_detail_body(&app.root.resources).expect("json body");
    assert!(body.raw_json.is_some());

    next_field(&mut app);

    assert_eq!(field_index(&app), None);
}

// ---- cursor clearing ----

#[test]
fn switching_detail_tabs_clears_the_field_cursor() {
    let mut app = app_with_compute_detail();
    next_field(&mut app);
    assert_eq!(field_index(&app), Some(0));

    dispatch(&mut app, ResourcesAction::SelectNextTab);
    assert_eq!(field_index(&app), None);

    next_field(&mut app);
    dispatch(&mut app, ResourcesAction::SelectPreviousTab);
    assert_eq!(field_index(&app), None);
}

#[test]
fn closing_the_detail_pane_clears_the_field_cursor() {
    let mut app = app_with_compute_detail();
    next_field(&mut app);
    assert_eq!(field_index(&app), Some(0));

    dispatch(&mut app, ResourcesAction::CloseDetail);

    assert_eq!(field_index(&app), None);
}

#[test]
fn opening_a_detail_pane_clears_the_field_cursor() {
    let mut app = app_with_compute_detail();
    next_field(&mut app);
    dispatch(&mut app, ResourcesAction::CloseDetail);
    // A cursor left behind by any earlier pane must not survive the next open.
    app.root.resources.set_detail_field_index(Some(2));

    dispatch(&mut app, ResourcesAction::OpenSelected);

    assert_eq!(field_index(&app), None);
}

#[test]
fn a_detail_pane_that_vanishes_on_refresh_clears_the_field_cursor() {
    let mut app = app_with_compute_detail();
    next_field(&mut app);
    assert_eq!(field_index(&app), Some(0));

    // The open instance is gone from the refreshed list, so its pane closes.
    let context = app.root.current_context().unwrap().clone();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ComputeInstancesLoaded {
            context,
            instances: vec![instance("api-dev-2")],
        }),
    );

    assert!(!app.root.resources.is_showing_detail());
    assert_eq!(field_index(&app), None);
}

#[test]
fn switching_the_active_service_clears_the_field_cursor() {
    let mut app = app_with_compute_detail();
    next_field(&mut app);
    assert_eq!(field_index(&app), Some(0));

    dispatch(
        &mut app,
        ResourcesAction::SelectService(ResourceServiceKind::GkeClusters),
    );

    assert_eq!(field_index(&app), None);
}

#[test]
fn returning_to_the_service_list_clears_the_field_cursor() {
    let mut app = app_with_compute_detail();
    next_field(&mut app);

    app.root
        .resources
        .show_service_list(ResourceServiceKind::ComputeInstances);

    assert_eq!(field_index(&app), None);
}

#[test]
fn entering_the_cloud_run_revision_drill_down_clears_the_field_cursor() {
    let mut app = app_with_cloud_run_revisions_tab();
    next_field(&mut app);
    assert_eq!(field_index(&app), Some(0));

    dispatch(&mut app, ResourcesAction::OpenSelected);

    assert!(app.root.resources.is_showing_cloud_run_revision());
    assert_eq!(field_index(&app), None);
}

/// AppState drilled into the Cloud Run revision detail, loaded.
fn app_with_cloud_run_revision_open() -> AppState {
    let mut app = app_with_cloud_run_revisions_tab();
    dispatch(&mut app, ResourcesAction::OpenSelected);
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
fn switching_revision_tabs_clears_the_field_cursor() {
    let mut app = app_with_cloud_run_revision_open();
    next_field(&mut app);
    assert_eq!(field_index(&app), Some(0));

    app.root.resources.select_next_cloud_run_revision_tab();
    assert_eq!(field_index(&app), None);

    app.root.resources.select_previous_cloud_run_revision_tab();
    next_field(&mut app);
    app.root.resources.select_previous_cloud_run_revision_tab();
    assert_eq!(field_index(&app), None);
}

#[test]
fn leaving_the_cloud_run_revision_drill_down_clears_the_field_cursor() {
    let mut app = app_with_cloud_run_revision_open();
    next_field(&mut app);
    assert_eq!(field_index(&app), Some(0));

    // Back leaves the revision sub-location for the service detail beneath it.
    reduce(&mut app, &Action::Navigation(NavAction::Back));

    assert!(!app.root.resources.is_showing_cloud_run_revision());
    assert_eq!(field_index(&app), None);
}

#[test]
fn cloud_storage_view_transitions_clear_the_field_cursor() {
    let mut app = app_with_bucket_detail();
    next_field(&mut app);
    assert_eq!(field_index(&app), Some(0));

    // Bucket detail -> object browser: the pane is gone.
    dispatch(&mut app, ResourcesAction::OpenSelected);
    assert_eq!(field_index(&app), None);

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
    app.root.resources.set_detail_field_index(Some(1));

    // Object browser -> object detail: a new pane with a new field list.
    dispatch(&mut app, ResourcesAction::OpenSelected);
    assert_eq!(field_index(&app), None);

    let detail = parse_object_detail_json(include_str!("fixtures/gcs_object_detail.json"))
        .expect("object detail fixture should parse");
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::StorageObjectDetailLoaded { context, detail }),
    );
    next_field(&mut app);
    assert_eq!(field_index(&app), Some(0));

    // Ascending leaves the object detail for the bucket list.
    dispatch(&mut app, ResourcesAction::AscendPrefix);
    assert_eq!(field_index(&app), None);
}

// ---- copying the selected field ----

#[test]
fn copy_resource_uri_with_a_selected_field_copies_that_field() {
    let mut app = app_with_compute_detail();
    // Fields on the Compute Summary tab: Name, Status, Zone, ...
    next_field(&mut app);
    next_field(&mut app);
    next_field(&mut app);

    let effects = dispatch(&mut app, ResourcesAction::CopyResourceUri);

    assert_eq!(
        effects,
        vec![Effect::CopyToClipboard {
            value: "europe-west1-b".to_string(),
            description: "field \"Zone\"".to_string(),
        }]
    );
}

#[test]
fn copy_resource_uri_without_a_selection_still_copies_the_bucket_uri() {
    let mut app = app_with_bucket_detail();
    assert_eq!(field_index(&app), None);

    let effects = dispatch(&mut app, ResourcesAction::CopyResourceUri);

    assert_eq!(
        effects,
        vec![Effect::CopyToClipboard {
            value: "gs://alpha-bucket".to_string(),
            description: "bucket URI".to_string(),
        }]
    );
}

#[test]
fn copy_resource_uri_on_a_json_tab_copies_the_whole_document() {
    let mut app = app_with_compute_detail();
    // Summary -> Network -> Disks -> Metadata -> Json.
    for _ in 0..4 {
        dispatch(&mut app, ResourcesAction::SelectNextTab);
    }

    let effects = dispatch(&mut app, ResourcesAction::CopyResourceUri);

    assert_eq!(
        effects,
        vec![Effect::CopyToClipboard {
            value: "{\"name\":\"api-dev-1\"}".to_string(),
            description: "JSON document".to_string(),
        }]
    );
}

#[test]
fn copy_resource_uri_on_a_bucket_json_tab_copies_the_document_not_the_bucket_uri() {
    let mut app = app_with_bucket_detail();
    // Summary -> Metadata -> JSON.
    dispatch(&mut app, ResourcesAction::SelectNextTab);
    dispatch(&mut app, ResourcesAction::SelectNextTab);
    let raw_json = app
        .root
        .resources
        .storage()
        .bucket_detail()
        .expect("bucket detail should be loaded")
        .raw_json
        .clone();

    let effects = dispatch(&mut app, ResourcesAction::CopyResourceUri);

    // Deliberate precedence: the open pane wins over the list-view URI copy, so
    // on a JSON tab with nothing selected `y` yanks the document rather than
    // `gs://alpha-bucket`.
    assert_eq!(
        effects,
        vec![Effect::CopyToClipboard {
            value: raw_json,
            description: "JSON document".to_string(),
        }]
    );
}

#[test]
fn copy_resource_uri_on_an_object_json_tab_copies_the_document_not_the_object_uri() {
    let mut app = app_with_object_detail();
    // Summary -> Metadata -> JSON.
    dispatch(&mut app, ResourcesAction::SelectNextTab);
    dispatch(&mut app, ResourcesAction::SelectNextTab);
    let raw_json = app
        .root
        .resources
        .storage()
        .object_detail()
        .expect("object detail should be loaded")
        .raw_json
        .clone();

    let effects = dispatch(&mut app, ResourcesAction::CopyResourceUri);

    // Same precedence for objects: the document, not the `gs://` object URI.
    assert_eq!(
        effects,
        vec![Effect::CopyToClipboard {
            value: raw_json,
            description: "JSON document".to_string(),
        }]
    );
}

#[test]
fn copy_resource_uri_in_a_detail_pane_without_fields_is_a_no_op() {
    let mut app = app_with_compute_detail();
    // The Metadata tab of this fixture has neither copyable fields nor a raw
    // document, and Compute has no URI to fall back to.
    for _ in 0..3 {
        dispatch(&mut app, ResourcesAction::SelectNextTab);
    }

    assert!(dispatch(&mut app, ResourcesAction::CopyResourceUri).is_empty());
}

#[test]
fn a_stale_field_index_is_clamped_to_the_body_on_screen() {
    let mut app = app_with_compute_detail();
    let last = field_count(&app) - 1;
    app.root.resources.set_detail_field_index(Some(99));

    let effects = dispatch(&mut app, ResourcesAction::CopyResourceUri);

    let body = project_detail_body(&app.root.resources).expect("compute detail body");
    let (_, field) = body.fields()[last];
    assert_eq!(
        effects,
        vec![Effect::CopyToClipboard {
            value: field.value.clone(),
            description: format!("field \"{}\"", field.label),
        }]
    );

    // Moving from a stale index lands inside the body, never past its end.
    next_field(&mut app);
    assert_eq!(field_index(&app), Some(0));
}

#[test]
fn gke_detail_fields_are_selectable_and_copyable() {
    let mut app = app_with_gce_rows();
    dispatch(
        &mut app,
        ResourcesAction::SelectService(ResourceServiceKind::GkeClusters),
    );
    let context = app.root.current_context().unwrap().clone();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::GkeClustersLoaded {
            context: context.clone(),
            clusters: vec![gke_cluster("payments", "europe-west1")],
        }),
    );
    dispatch(&mut app, ResourcesAction::OpenSelected);
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::GkeClusterDetailLoaded {
            context,
            detail: nuvem_domain::gcp::resource::GkeClusterDetail {
                name: "payments".to_string(),
                location: "europe-west1".to_string(),
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
                raw_json: "{\"name\":\"payments\"}".to_string(),
            },
        }),
    );

    previous_field(&mut app);
    let effects = dispatch(&mut app, ResourcesAction::CopyResourceUri);

    assert_eq!(
        effects,
        vec![Effect::CopyToClipboard {
            value: "1.31.5-gke.1023000".to_string(),
            description: "field \"Node version\"".to_string(),
        }]
    );
}

// ---- paging releases the cursor ----

#[test]
fn paging_the_detail_pane_clears_the_field_cursor() {
    let mut app = app_with_compute_detail();
    next_field(&mut app);
    assert_eq!(field_index(&app), Some(0));

    dispatch(&mut app, ResourcesAction::ScrollDetailDown);

    // Ensure-visible re-pins the viewport to the selected line on every frame,
    // so a page that left the selection behind would be undone immediately.
    // Paging therefore drops the cursor -- which is also how the operator
    // deselects a field.
    assert_eq!(field_index(&app), None);

    next_field(&mut app);
    assert_eq!(field_index(&app), Some(0));
    dispatch(&mut app, ResourcesAction::ScrollDetailUp);
    assert_eq!(field_index(&app), None);
}

#[test]
fn paging_a_cloud_storage_detail_pane_clears_the_field_cursor() {
    let mut app = app_with_bucket_detail();
    next_field(&mut app);
    assert!(field_index(&app).is_some());

    dispatch(&mut app, ResourcesAction::ScrollDetailDown);

    assert_eq!(field_index(&app), None);
}

// ---- sanitizing the two new sinks ----

/// AppState on the Metadata tab of an instance whose metadata is hostile:
/// instance metadata is writable by anyone with the permission, so its keys
/// and values are untrusted on both the clipboard and the status line.
fn app_with_hostile_metadata() -> AppState {
    let mut app = app_with_gce_rows();
    dispatch(&mut app, ResourcesAction::OpenSelected);
    let context = app.root.current_context().unwrap().clone();
    let mut detail = compute_detail("api-dev-1");
    detail.metadata = vec![
        // A real, legitimately multi-line value with an escape sequence
        // smuggled into the middle of it.
        ComputeKeyValue {
            key: "startup-script".to_string(),
            value: "#!/bin/bash\n\u{1b}[31mecho hello\n\techo bye".to_string(),
        },
        ComputeKeyValue {
            key: "boot\u{1b}[2Jed\nkey".to_string(),
            value: "plain".to_string(),
        },
        ComputeKeyValue {
            key: "hidden".to_string(),
            value: "\u{1b}[2J\u{1b}[H".to_string(),
        },
    ];
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ComputeInstanceDetailLoaded { context, detail }),
    );
    // Summary -> Network -> Disks -> Metadata.
    for _ in 0..3 {
        dispatch(&mut app, ResourcesAction::SelectNextTab);
    }
    app
}

#[test]
fn copying_a_multi_line_value_keeps_its_lines_and_drops_escape_sequences() {
    let mut app = app_with_hostile_metadata();
    next_field(&mut app);

    let effects = dispatch(&mut app, ResourcesAction::CopyResourceUri);

    // Newlines and tabs survive -- a startup script pasted back has to be the
    // script -- while the escape sequence a paste would feed to the terminal
    // does not.
    assert_eq!(
        effects,
        vec![Effect::CopyToClipboard {
            value: "#!/bin/bash\necho hello\n\techo bye".to_string(),
            description: "field \"startup-script\"".to_string(),
        }]
    );
}

#[test]
fn a_hostile_field_label_is_stripped_before_it_reaches_the_status_line() {
    let mut app = app_with_hostile_metadata();
    next_field(&mut app);
    next_field(&mut app);

    let effects = dispatch(&mut app, ResourcesAction::CopyResourceUri);

    // The label is rendered back as `Copied field "..." to clipboard`, a
    // single-line surface: the escape sequence is gone and the newline is
    // replaced, exactly as every other rendered string is treated.
    assert_eq!(
        effects,
        vec![Effect::CopyToClipboard {
            value: "plain".to_string(),
            description: "field \"booted\u{FFFD}key\"".to_string(),
        }]
    );
}

#[test]
fn a_value_that_is_nothing_but_escape_sequences_is_not_copyable() {
    let app = app_with_hostile_metadata();

    // Three metadata entries, but the one whose whole value is an escape
    // sequence sanitizes down to nothing and joins the placeholders as
    // non-copyable.
    assert_eq!(field_count(&app), 2);
}

#[test]
fn the_display_and_clipboard_sinks_are_sanitized_differently_on_purpose() {
    // One value carrying BOTH an escape sequence and newlines. The pane and the
    // clipboard must treat it differently, and this test exists so that
    // collapsing the two sanitizers into one fails loudly: the pane is a single
    // buffer line and can hold neither, while a multi-line `startup-script`
    // that arrives on the clipboard flattened is silently corrupted.
    let mut app = app_with_hostile_metadata();
    next_field(&mut app);

    let body = project_detail_body(&app.root.resources).expect("compute metadata body");
    let (line, _) = body.fields()[0];

    assert_eq!(
        body.lines[line].text,
        "startup-script: #!/bin/bash\u{FFFD}echo hello\u{FFFD}\u{FFFD}echo bye",
        "the rendered line must hold neither the escape sequence nor the newlines"
    );

    let effects = dispatch(&mut app, ResourcesAction::CopyResourceUri);

    assert_eq!(
        effects,
        vec![Effect::CopyToClipboard {
            value: "#!/bin/bash\necho hello\n\techo bye".to_string(),
            description: "field \"startup-script\"".to_string(),
        }],
        "the clipboard keeps the newlines and tabs it dropped on screen"
    );
}

#[test]
fn a_hostile_metadata_key_never_reaches_the_rendered_line() {
    let app = app_with_hostile_metadata();

    let body = project_detail_body(&app.root.resources).expect("compute metadata body");

    assert!(
        body.lines
            .iter()
            .all(|line| !line.text.contains('\u{1b}') && !line.text.contains('\n')),
        "no detail line may carry an escape introducer or a newline into a \
         buffer cell, got: {:?}",
        body.lines.iter().map(|line| &line.text).collect::<Vec<_>>()
    );
    assert!(
        body.lines
            .iter()
            .any(|line| line.text.starts_with("booted\u{FFFD}key: plain")),
        "the hostile key should still be shown, just defanged"
    );
}
