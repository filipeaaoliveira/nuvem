use nuvem::app::projection::table::{CellAlign, CellKind, StatusSemantic};
use nuvem::app::projection::{
    LoadProgressProjection, ResourceDetailProjection, ResourceListProjection,
    ResourceWorkspaceStatus, project_resource_preview, project_resources,
};
use nuvem::app::reducer::root::reduce;
use nuvem::app::state::resources::{
    CloudRunDetailTab, ComputeDetailTab, GkeDetailTab, ResourcesViewMode,
};
use nuvem::app::{Action, AppState, EffectResult, NavigatorAction, ResourcesAction};
use nuvem_domain::gcp::configuration::{
    GcloudConfiguration, GcloudConfigurationName, GcloudContext,
};
use nuvem_domain::gcp::resource::{
    CloudRunService, CloudRunServiceDetail, CloudRunServiceStatus, ComputeDisk, ComputeInstance,
    ComputeInstanceDetail, ComputeNetworkInterface, EndpointVisibility, GkeAddon, GkeCluster,
    GkeClusterDetail, GkeClusterStatus, GkeNodePool, ResourceServiceKind, ResourceStatus,
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

fn cloud_run_service(name: &str, region: &str) -> CloudRunService {
    CloudRunService {
        name: name.to_string(),
        region: region.to_string(),
        url: Some(format!("https://{name}.a.run.app")),
        latest_ready_revision: Some(format!("{name}-00007")),
        latest_created_revision: Some(format!("{name}-00007")),
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
        latest_ready_revision: Some(format!("{name}-00007")),
        latest_created_revision: Some(format!("{name}-00007")),
        traffic: Vec::new(),
        revisions: Vec::new(),
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

#[test]
fn projects_fresh_default_as_not_loaded_active_gce() {
    let app = AppState::new();
    let projection = project_resources(&app.root.resources);

    assert_eq!(projection.service, ResourceServiceKind::ComputeInstances);
    assert_eq!(projection.status, ResourceWorkspaceStatus::NotLoaded);
    assert_eq!(projection.total_rows, 0);
    assert_eq!(projection.visible_rows, 0);
    assert_eq!(projection.selected_visible, None);
    assert!(matches!(projection.list, ResourceListProjection::Compute(rows) if rows.is_empty()));
}

#[test]
fn projects_loaded_gce_shell_and_typed_rows() {
    let app = app_with_gce_rows();
    let projection = project_resources(&app.root.resources);

    assert_eq!(projection.service, ResourceServiceKind::ComputeInstances);
    assert_eq!(projection.service_label, "GCE instances");
    assert_eq!(projection.status, ResourceWorkspaceStatus::Ready);
    assert_eq!(projection.context_label.as_deref(), Some("dev"));
    assert_eq!(projection.total_rows, 1);
    assert_eq!(projection.visible_rows, 1);
    assert_eq!(projection.selected_visible, Some(0));
    assert_eq!(projection.filter.applied, "");
    assert_eq!(projection.sort_label, "Name");
    assert_eq!(projection.view_mode, ResourcesViewMode::List);

    let ResourceListProjection::Compute(rows) = projection.list else {
        panic!("expected compute projection");
    };
    assert_eq!(rows[0].name, "api-dev-1");
    assert_eq!(rows[0].zone, "europe-west1-b");
    assert_eq!(rows[0].status, "Running");
    assert_eq!(rows[0].machine_type, "e2-medium");
    assert_eq!(rows[0].internal_ip.as_deref(), Some("10.0.0.4"));
    assert_eq!(rows[0].external_ip.as_deref(), Some("34.10.10.10"));
}

#[test]
fn resource_projection_keeps_typed_service_when_navigator_selection_differs() {
    let mut app = app_with_gce_rows();
    reduce(&mut app, &Action::Navigator(NavigatorAction::SelectNext));

    let projection = project_resources(&app.root.resources);

    assert_eq!(projection.service, ResourceServiceKind::ComputeInstances);
    assert_eq!(projection.service_label, "GCE instances");
    assert!(matches!(
        projection.list,
        ResourceListProjection::Compute(_)
    ));
}

#[test]
fn projects_filter_draft_applied_and_cycled_sort() {
    let mut app = app_with_gce_rows();
    let context = app.root.current_context().unwrap().clone();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ComputeInstancesLoaded {
            context,
            instances: vec![instance("api-dev-1"), instance("worker-dev-1")],
        }),
    );
    reduce(&mut app, &Action::Resources(ResourcesAction::StartFilter));
    for value in "api".chars() {
        reduce(
            &mut app,
            &Action::Resources(ResourcesAction::AppendFilterChar(value)),
        );
    }
    reduce(&mut app, &Action::Resources(ResourcesAction::ApplyFilter));
    reduce(&mut app, &Action::Resources(ResourcesAction::StartFilter));
    reduce(
        &mut app,
        &Action::Resources(ResourcesAction::AppendFilterChar('-')),
    );
    reduce(
        &mut app,
        &Action::Resources(ResourcesAction::SelectNextColumn),
    );
    reduce(
        &mut app,
        &Action::Resources(ResourcesAction::SortBySelectedColumn),
    );

    let projection = project_resources(&app.root.resources);

    assert!(projection.filter.active);
    assert_eq!(projection.filter.draft, "api-");
    assert_eq!(projection.filter.applied, "api");
    assert_eq!(projection.sort_label, "Status");
    assert_eq!(projection.total_rows, 2);
    assert_eq!(projection.visible_rows, 1);
}

/// Types `text` into the filter input without applying it, so the projection
/// is built from the half-typed draft.
fn app_with_filter_draft(text: &str) -> AppState {
    let mut app = app_with_gce_rows();
    reduce(&mut app, &Action::Resources(ResourcesAction::StartFilter));
    for value in text.chars() {
        reduce(
            &mut app,
            &Action::Resources(ResourcesAction::AppendFilterChar(value)),
        );
    }
    app
}

/// Types `text` into the filter input and applies it, so the filter input is
/// no longer active and the projection is built from the applied text.
fn app_with_filter_applied(text: &str) -> AppState {
    let mut app = app_with_filter_draft(text);
    reduce(&mut app, &Action::Resources(ResourcesAction::ApplyFilter));
    app
}

#[test]
fn projects_filter_mode_label_per_prefix_from_the_draft() {
    // The chip is what makes the grammar discoverable, so it must track the
    // draft as it is typed -- before anything is applied.
    for (draft, expected) in [
        ("api", "regex"),
        ("-f ap", "fuzzy"),
        ("-l env=prod", "label"),
        ("!api", "inverse"),
    ] {
        let app = app_with_filter_draft(draft);
        let projection = project_resources(&app.root.resources);

        assert!(projection.filter.active);
        assert_eq!(
            projection.filter.mode_label, expected,
            "unexpected mode label for draft {draft:?}"
        );
    }
}

#[test]
fn projects_filter_mode_label_per_prefix_from_the_applied_text() {
    for (applied, expected) in [
        ("api", "regex"),
        ("-f ap", "fuzzy"),
        ("-l env=prod", "label"),
        ("!api", "inverse"),
    ] {
        let app = app_with_filter_applied(applied);
        let projection = project_resources(&app.root.resources);

        assert!(!projection.filter.active);
        assert_eq!(
            projection.filter.mode_label, expected,
            "unexpected mode label for applied filter {applied:?}"
        );
    }
}

#[test]
fn projects_filter_error_for_an_uncompilable_regex() {
    let app = app_with_filter_applied("a(b");
    let projection = project_resources(&app.root.resources);

    assert_eq!(
        projection.filter.error.as_deref(),
        Some("invalid regex: unclosed group")
    );
}

#[test]
fn projects_no_filter_error_for_a_valid_regex() {
    let app = app_with_filter_applied("api.*");
    let projection = project_resources(&app.root.resources);

    assert_eq!(projection.filter.error, None);
}

#[test]
fn projects_load_failure_on_cleared_collection_as_empty_failed() {
    // A list-fetch failure with no fresh rows to fall back on now surfaces
    // the dedicated `Failed` status (carrying the diagnostic headline via
    // `load_error`) instead of the generic `Stale` empty state.
    let mut app = app_with_gce_rows();
    let context = app.root.current_context().unwrap().clone();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ComputeInstancesLoadFailed {
            context,
            error: nuvem::app::AppError::gcloud("load failed"),
        }),
    );

    let projection = project_resources(&app.root.resources);

    assert_eq!(projection.status, ResourceWorkspaceStatus::Failed);
    assert!(projection.load_error.is_some());
    assert!(projection.load_progress.is_none());
    assert_eq!(projection.total_rows, 0);
    assert_eq!(projection.visible_rows, 0);
    assert_eq!(projection.selected_visible, None);
    assert!(matches!(projection.list, ResourceListProjection::Compute(rows) if rows.is_empty()));
}

#[test]
fn projects_rows_pending_background_refresh_as_stale_without_load_phase() {
    // A lifecycle action (e.g. Stop) clears rows and marks the collection
    // stale to trigger a background refresh, without ever touching
    // `load_phase`. `workspace_status` must still report `Stale` here (no
    // active Loading/Failed phase competes with it).
    let mut app = app_with_gce_rows();
    app.root
        .resources
        .mark_active_stale_and_clear(ResourceServiceKind::ComputeInstances);

    let projection = project_resources(&app.root.resources);

    assert_eq!(projection.status, ResourceWorkspaceStatus::Stale);
    assert!(projection.load_progress.is_none());
    assert!(projection.load_error.is_none());
}

#[test]
fn begin_load_on_not_loaded_collection_reports_loading_with_fresh_spinner() {
    let mut app = AppState::new();
    app.root.resources.begin_load(
        ResourceServiceKind::ComputeInstances,
        std::time::Instant::now(),
    );

    let projection = project_resources(&app.root.resources);

    assert_eq!(projection.status, ResourceWorkspaceStatus::Loading);
    let progress = projection
        .load_progress
        .expect("loading status carries progress");
    assert_eq!(progress.elapsed_secs, 0);
    assert_eq!(
        progress,
        LoadProgressProjection {
            elapsed_secs: 0,
            spinner_frame: "⠋",
        }
    );
    assert!(projection.load_error.is_none());
}

#[test]
fn begin_load_on_stale_collection_reports_loading() {
    // The in-flight spinner must beat the "press r" stale prompt while a
    // refresh is already running.
    let mut app = app_with_gce_rows();
    app.root
        .resources
        .mark_active_stale_and_clear(ResourceServiceKind::ComputeInstances);
    app.root.resources.begin_load(
        ResourceServiceKind::ComputeInstances,
        std::time::Instant::now(),
    );

    let projection = project_resources(&app.root.resources);

    assert_eq!(projection.status, ResourceWorkspaceStatus::Loading);
    assert!(projection.load_progress.is_some());
}

#[test]
fn begin_load_after_fresh_ready_rows_stays_ready() {
    // A background refresh over a loaded, non-stale context must not blank
    // the list with a spinner.
    let mut app = app_with_gce_rows();
    app.root.resources.begin_load(
        ResourceServiceKind::ComputeInstances,
        std::time::Instant::now(),
    );

    let projection = project_resources(&app.root.resources);

    assert_eq!(projection.status, ResourceWorkspaceStatus::Ready);
    assert!(projection.load_progress.is_none());
    assert!(projection.load_error.is_none());
    assert_eq!(projection.total_rows, 1);
}

#[test]
fn fail_load_on_cleared_collection_reports_failed_with_message() {
    let mut app = AppState::new();
    let context = GcloudContext {
        configuration: GcloudConfigurationName::new("dev"),
        account: None,
        project: Some("dev-project".to_string()),
        region: None,
        zone: None,
    };
    app.root.resources.fail_load(
        ResourceServiceKind::ComputeInstances,
        context,
        "gcloud denied the request".to_string(),
    );

    let projection = project_resources(&app.root.resources);

    assert_eq!(projection.status, ResourceWorkspaceStatus::Failed);
    assert_eq!(
        projection.load_error.as_deref(),
        Some("gcloud denied the request")
    );
    assert!(projection.load_progress.is_none());
}

#[test]
fn projects_gke_rows_without_erasing_service_type() {
    let mut app = app_with_gce_rows();
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
            clusters: vec![gke_cluster("payments", "europe-west1")],
        }),
    );

    let projection = project_resources(&app.root.resources);
    assert_eq!(projection.service, ResourceServiceKind::GkeClusters);
    assert_eq!(projection.service_label, "GKE clusters");
    let ResourceListProjection::Gke(rows) = projection.list else {
        panic!("expected GKE projection");
    };
    assert_eq!(rows[0].name, "payments");
    assert_eq!(rows[0].location, "europe-west1");
    assert_eq!(rows[0].status, "Running");
    assert_eq!(rows[0].node_count, Some(3));
    assert_eq!(rows[0].endpoint_visibility, "Public");
}

#[test]
fn projects_loaded_compute_detail_with_typed_payload() {
    let mut app = app_with_gce_rows();
    reduce(&mut app, &Action::Resources(ResourcesAction::OpenSelected));
    let detail = compute_detail("api-dev-1");
    let context = app.root.current_context().unwrap().clone();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ComputeInstanceDetailLoaded {
            context,
            detail: detail.clone(),
        }),
    );

    let projection = project_resources(&app.root.resources);
    let Some(ResourceDetailProjection::Compute(detail_projection)) = projection.detail else {
        panic!("expected compute detail projection");
    };
    assert_eq!(detail_projection.target_name, "api-dev-1");
    assert_eq!(detail_projection.active_tab, ComputeDetailTab::Summary);
    assert_eq!(detail_projection.detail, Some(&detail));
    assert_eq!(detail_projection.error, None);
}

#[test]
fn does_not_build_list_projection_while_showing_detail() {
    let mut app = app_with_gce_rows();
    reduce(&mut app, &Action::Resources(ResourcesAction::OpenSelected));

    let projection = project_resources(&app.root.resources);

    assert_eq!(projection.view_mode, ResourcesViewMode::Detail);
    assert!(matches!(projection.list, ResourceListProjection::Compute(rows) if rows.is_empty()));
}

#[test]
fn projects_loaded_gke_detail_without_compute_action_state() {
    let mut app = app_with_gce_rows();
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
            context: context.clone(),
            clusters: vec![gke_cluster("payments", "europe-west1")],
        }),
    );
    reduce(&mut app, &Action::Resources(ResourcesAction::OpenSelected));
    reduce(&mut app, &Action::Resources(ResourcesAction::SelectNextTab));
    assert_eq!(
        app.root.resources.active_gke_detail_tab(),
        GkeDetailTab::Networking
    );
    let detail = gke_detail("payments", "europe-west1");
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::GkeClusterDetailLoaded {
            context,
            detail: detail.clone(),
        }),
    );

    let projection = project_resources(&app.root.resources);
    assert_eq!(projection.visible_rows, 1);
    assert!(matches!(projection.list, ResourceListProjection::Gke(rows) if rows.is_empty()));
    let Some(ResourceDetailProjection::Gke(detail_projection)) = projection.detail else {
        panic!("expected GKE detail projection");
    };
    assert_eq!(detail_projection.target_name, "payments");
    assert_eq!(detail_projection.active_tab, GkeDetailTab::Networking);
    assert_eq!(detail_projection.detail, Some(&detail));
    assert_eq!(detail_projection.error, None);
}

fn app_with_cloud_run_rows(services: Vec<CloudRunService>) -> AppState {
    let mut app = app_with_gce_rows();
    reduce(
        &mut app,
        &Action::Resources(ResourcesAction::SelectService(
            ResourceServiceKind::CloudRunServices,
        )),
    );
    let context = app.root.current_context().unwrap().clone();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::CloudRunServicesLoaded { context, services }),
    );
    app
}

#[test]
fn projects_cloud_run_rows_without_erasing_service_type() {
    let app = app_with_cloud_run_rows(vec![cloud_run_service("api", "europe-west1")]);

    let projection = project_resources(&app.root.resources);
    assert_eq!(projection.service, ResourceServiceKind::CloudRunServices);
    let ResourceListProjection::CloudRun(rows) = projection.list else {
        panic!("expected Cloud Run projection");
    };
    assert_eq!(rows[0].name, "api");
    assert_eq!(rows[0].region, "europe-west1");
    assert_eq!(rows[0].status, "Ready");
    assert_eq!(rows[0].status_semantic, StatusSemantic::Healthy);
    assert_eq!(rows[0].url.as_deref(), Some("https://api.a.run.app"));
    assert_eq!(rows[0].latest_ready_revision.as_deref(), Some("api-00007"));
    assert_eq!(rows[0].traffic_summary, "100% -> latest");
}

#[test]
fn cloud_run_list_builds_non_empty_table_model() {
    let app = app_with_cloud_run_rows(vec![
        cloud_run_service("api", "europe-west1"),
        cloud_run_service("web", "us-central1"),
    ]);

    let projection = project_resources(&app.root.resources);
    let table = projection.table.expect("cloud run list has a table model");
    assert_eq!(table.columns.len(), 6);
    assert_eq!(
        table.columns.iter().map(|c| c.header).collect::<Vec<_>>(),
        vec![
            "NAME",
            "STATUS",
            "REGION",
            "URL",
            "LATEST-REVISION",
            "TRAFFIC"
        ]
    );
    assert_eq!(table.rows.len(), 2);
    assert_eq!(table.rows[0].cells[0].text, "api");
    assert_eq!(
        table.rows[0].cells[1].kind,
        CellKind::Status(StatusSemantic::Healthy)
    );
    assert_eq!(table.counts.total, 2);
    // Sortable columns must line up with CloudRunSortField::from_column.
    let sortable: Vec<bool> = table.columns.iter().map(|c| c.sortable).collect();
    assert_eq!(sortable, vec![true, true, true, true, true, false]);
    // Right-aligned traffic is not expected; all left-aligned like GKE textual cols.
    assert!(matches!(table.columns[0].align, CellAlign::Left));
}

#[test]
fn projects_loaded_cloud_run_detail_with_active_tab() {
    let mut app = app_with_cloud_run_rows(vec![cloud_run_service("api", "europe-west1")]);
    reduce(&mut app, &Action::Resources(ResourcesAction::OpenSelected));
    reduce(&mut app, &Action::Resources(ResourcesAction::SelectNextTab));
    assert_eq!(
        app.root.resources.active_cloud_run_detail_tab(),
        CloudRunDetailTab::Traffic
    );
    let detail = cloud_run_detail("api", "europe-west1");
    let context = app.root.current_context().unwrap().clone();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::CloudRunServiceDetailLoaded {
            context,
            detail: detail.clone(),
        }),
    );

    let projection = project_resources(&app.root.resources);
    assert_eq!(projection.view_mode, ResourcesViewMode::Detail);
    assert!(
        matches!(projection.list, ResourceListProjection::CloudRun(rows) if rows.is_empty()),
        "detail mode must not populate list rows"
    );
    let Some(ResourceDetailProjection::CloudRun(detail_projection)) = projection.detail else {
        panic!("expected Cloud Run detail projection");
    };
    assert_eq!(detail_projection.target_name, "api");
    assert_eq!(detail_projection.active_tab, CloudRunDetailTab::Traffic);
    assert_eq!(detail_projection.detail, Some(&detail));
    assert_eq!(detail_projection.error, None);
}

#[test]
fn preview_projects_selected_cloud_run_summary() {
    let app = app_with_cloud_run_rows(vec![cloud_run_service("api", "europe-west1")]);
    let preview =
        project_resource_preview(&app.root.resources).expect("preview for selected service");
    assert_eq!(preview.title, "api");
    assert!(
        preview
            .lines
            .iter()
            .any(|(k, v)| k == "Region" && v == "europe-west1")
    );
    assert!(
        preview
            .lines
            .iter()
            .any(|(k, v)| k == "Traffic" && v == "100% -> latest")
    );
}

#[test]
fn architecture_documents_projection_based_resource_shell() {
    let architecture = include_str!("../ARCHITECTURE.md");
    assert!(
        architecture.contains("Render-facing resource projections are built in src/app/projection")
    );
    assert!(architecture.contains(
        "Add typed service-specific row/detail projections when UI needs derived display data"
    ));
    assert!(architecture.contains(
        "The shared resource table lives in src/ui/shared/table.rs and consumes a TableModel built in src/app/projection/table.rs"
    ));
}

#[test]
fn preview_projects_selected_compute_summary() {
    let mut app = AppState::new();
    load_resource_rows(&mut app, vec![instance("api-a")]);
    let preview = project_resource_preview(&app.root.resources).expect("preview");
    assert_eq!(preview.title, "api-a");
    assert!(
        preview
            .lines
            .iter()
            .any(|(k, v)| k == "Zone" && v == "europe-west1-b")
    );
    assert!(preview.lines.iter().any(|(k, _)| k == "Status"));
    assert!(preview.lines.iter().any(|(k, _)| k == "Machine Type"));
}

#[test]
fn preview_is_none_when_no_rows() {
    let app = AppState::new();
    assert!(project_resource_preview(&app.root.resources).is_none());
}

#[test]
fn projection_reuses_visible_rows_for_selection() {
    let source = include_str!("../src/app/projection/mod.rs");

    assert!(source.contains("selected_visible_index_for_visible_rows"));
    assert!(!source.contains("selected_visible: state.selected_visible_index()"));
}

#[test]
fn gce_table_model_has_typed_columns_and_status_cells() {
    let app = app_with_gce_rows();
    let projection = project_resources(&app.root.resources);
    let table = projection.table.expect("list mode builds a table model");

    let keys: Vec<&str> = table.columns.iter().map(|c| c.key).collect();
    assert_eq!(
        keys,
        vec![
            "name",
            "status",
            "zone",
            "machine-type",
            "internal-ip",
            "external-ip"
        ]
    );
    assert_eq!(table.columns[0].priority, 0);
    assert_eq!(table.counts.total, 1);
    assert_eq!(table.counts.filtered, 1);
    assert_eq!(table.selected_row, Some(0));

    // Status cell carries the shared semantic.
    assert!(matches!(
        table.rows[0].cells[1].kind,
        CellKind::Status(StatusSemantic::Healthy)
    ));
}

#[test]
fn gke_table_model_right_aligns_nodes_and_marks_missing() {
    let mut app = app_with_gce_rows();
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
            clusters: vec![gke_cluster("payments", "europe-west1")],
        }),
    );

    let projection = project_resources(&app.root.resources);
    let table = projection.table.expect("list mode builds a table model");
    let nodes = table.columns.iter().position(|c| c.key == "nodes").unwrap();
    assert_eq!(table.columns[nodes].align, CellAlign::Right);
}

#[test]
fn detail_mode_has_no_table_model() {
    let mut app = app_with_gce_rows();
    reduce(&mut app, &Action::Resources(ResourcesAction::OpenSelected));
    let projection = project_resources(&app.root.resources);
    assert!(projection.table.is_none());
}

#[test]
fn table_rows_reflect_mark_state_and_gke_rows_are_never_marked() {
    let mut app = AppState::new();
    load_resource_rows(
        &mut app,
        vec![instance("api-dev-1"), instance("worker-dev-1")],
    );
    reduce(&mut app, &Action::Resources(ResourcesAction::ToggleMark));

    let projection = project_resources(&app.root.resources);
    let table = projection.table.expect("list mode builds a table model");
    assert!(table.rows[0].marked, "selected instance should be marked");
    assert!(!table.rows[1].marked, "unmarked instance stays unmarked");

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
            clusters: vec![gke_cluster("payments", "europe-west1")],
        }),
    );
    let gke_projection = project_resources(&app.root.resources);
    let gke_table = gke_projection.table.expect("gke table model");
    assert!(!gke_table.rows[0].marked, "GKE rows never carry marks");
}

#[test]
fn selection_survives_sort_direction_reverse() {
    let mut app = app_with_gce_rows();
    let context = app.root.current_context().unwrap().clone();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ComputeInstancesLoaded {
            context,
            instances: vec![instance("api-dev-1"), instance("worker-dev-1")],
        }),
    );
    reduce(&mut app, &Action::Resources(ResourcesAction::SelectNext));
    let before = project_resources(&app.root.resources).selected_visible;
    reduce(
        &mut app,
        &Action::Resources(ResourcesAction::SortBySelectedColumn),
    );
    let after = project_resources(&app.root.resources).selected_visible;
    assert!(before.is_some() && after.is_some());
}

/// Cloud SQL rows are markable, so their table must render the mark
/// column and carry each row's mark state — otherwise a mark would be invisible
/// and the batch confirmation would arrive as a surprise.
#[test]
fn cloud_sql_table_renders_the_mark_column_and_row_mark_state() {
    use nuvem_domain::gcp::resource::{CloudSqlAvailability, CloudSqlInstance, CloudSqlStatus};

    fn sql(name: &str) -> CloudSqlInstance {
        CloudSqlInstance {
            name: name.to_string(),
            database_version: "POSTGRES_15".to_string(),
            engine: "PostgreSQL".to_string(),
            region: "europe-west1".to_string(),
            tier: "db-custom-2-7680".to_string(),
            availability: CloudSqlAvailability::Regional,
            status: CloudSqlStatus::Runnable,
            connectivity: EndpointVisibility::Private,
        }
    }

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
        &Action::EffectResult(EffectResult::CloudSqlInstancesLoaded {
            context,
            instances: vec![sql("archive"), sql("orders")],
        }),
    );
    reduce(&mut app, &Action::Resources(ResourcesAction::ToggleMark));

    let table = project_resources(&app.root.resources)
        .table
        .expect("list mode builds a table model");
    assert!(
        table.show_mark_column,
        "Cloud SQL rows are markable, so the mark column must render"
    );
    assert!(table.rows[0].marked, "selected instance should be marked");
    assert!(!table.rows[1].marked, "unmarked instance stays unmarked");
}
