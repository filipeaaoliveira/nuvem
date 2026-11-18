use nuvem::app::reducer::root::reduce;
use nuvem::app::state::navigation::{
    DetailTab, NavLocation, NavigationState, ResourceDetailLocation, ResourceKey, ResourceLocation,
};
use nuvem::app::state::resources::{CloudRunDetailTab, ComputeDetailTab};
use nuvem::app::{
    Action, AppState, Effect, EffectResult, NavAction, ResourcesAction, WorkspaceAction,
};
use nuvem_domain::gcp::configuration::{GcloudConfiguration, GcloudConfigurationName};
use nuvem_domain::gcp::resource::{
    CloudRunCondition, CloudRunRevision, CloudRunService, CloudRunServiceDetail,
    CloudRunServiceStatus, ResourceServiceKind,
};

fn list(service: ResourceServiceKind) -> NavLocation {
    NavLocation::Resources(ResourceLocation {
        service,
        detail: None,
    })
}

#[allow(dead_code)]
fn detail(name: &str, zone: &str) -> NavLocation {
    NavLocation::Resources(ResourceLocation {
        service: ResourceServiceKind::ComputeInstances,
        detail: Some(ResourceDetailLocation {
            key: ResourceKey::Compute {
                name: name.to_string(),
                zone: zone.to_string(),
            },
            tab: DetailTab::Compute(ComputeDetailTab::Summary),
            revision: None,
        }),
    })
}

#[test]
fn record_pushes_previous_and_clears_forward() {
    let mut nav = NavigationState::default();
    assert!(!nav.can_go_back());
    nav.record(NavLocation::Home);
    nav.record(list(ResourceServiceKind::ComputeInstances));
    assert!(nav.can_go_back());
    assert!(!nav.can_go_forward());
}

#[test]
fn back_then_forward_round_trips() {
    let mut nav = NavigationState::default();
    nav.record(NavLocation::Home); // back = [Home], we are at list
    let current = list(ResourceServiceKind::ComputeInstances);
    let back = nav.go_back(current.clone());
    assert_eq!(back, Some(NavLocation::Home));
    assert!(nav.can_go_forward());
    let forward = nav.go_forward(NavLocation::Home);
    assert_eq!(forward, Some(current));
    assert!(!nav.can_go_forward());
}

#[test]
fn back_on_empty_history_returns_none() {
    let mut nav = NavigationState::default();
    assert_eq!(nav.go_back(NavLocation::Home), None);
    assert_eq!(nav.go_forward(NavLocation::Home), None);
}

#[test]
fn new_forward_navigation_clears_forward_stack() {
    let mut nav = NavigationState::default();
    nav.record(NavLocation::Home);
    let _ = nav.go_back(list(ResourceServiceKind::ComputeInstances)); // forward = [list]
    assert!(nav.can_go_forward());
    nav.record(NavLocation::Home); // a new forward nav clears forward
    assert!(!nav.can_go_forward());
}

#[test]
fn reset_clears_both_stacks() {
    let mut nav = NavigationState::default();
    nav.record(NavLocation::Home);
    let _ = nav.go_back(list(ResourceServiceKind::GkeClusters));
    nav.reset();
    assert!(!nav.can_go_back());
    assert!(!nav.can_go_forward());
}

// --- Cloud Run navigation history (reducer-backed) ---

fn cr_config() -> GcloudConfiguration {
    GcloudConfiguration {
        name: GcloudConfigurationName::new("dev"),
        is_active: true,
        account: None,
        project: Some("dev-project".to_string()),
        region: None,
        zone: None,
    }
}

fn cr_service(name: &str, region: &str) -> CloudRunService {
    CloudRunService {
        name: name.to_string(),
        region: region.to_string(),
        url: Some(format!("https://{name}.run.app")),
        latest_ready_revision: Some(format!("{name}-00001")),
        latest_created_revision: Some(format!("{name}-00001")),
        traffic_summary: "100% -> latest".to_string(),
        status: CloudRunServiceStatus::Ready,
        ready_message: None,
    }
}

fn cr_revision(name: &str) -> CloudRunRevision {
    CloudRunRevision {
        name: name.to_string(),
        ready: CloudRunServiceStatus::Ready,
        created: None,
        image: Some("gcr.io/x/y:latest".to_string()),
        active_percent: Some(100),
    }
}

fn cr_detail(name: &str, region: &str, revisions: Vec<CloudRunRevision>) -> CloudRunServiceDetail {
    CloudRunServiceDetail {
        name: name.to_string(),
        region: region.to_string(),
        url: Some(format!("https://{name}.run.app")),
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

/// AppState with Cloud Run selected and one service ("api") loaded.
fn cloud_run_app() -> AppState {
    let mut app = AppState::new();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ConfigurationsLoaded {
            configurations: vec![cr_config()],
        }),
    );
    reduce(&mut app, &Action::Workspace(WorkspaceAction::Resources));
    reduce(
        &mut app,
        &Action::Resources(ResourcesAction::SelectService(
            ResourceServiceKind::CloudRunServices,
        )),
    );
    let context = app.root.current_context().unwrap();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::CloudRunServicesLoaded {
            context,
            services: vec![cr_service("api", "us-central1")],
        }),
    );
    app
}

/// Opens the "api" service detail and drills into revision "api-00001".
fn cloud_run_app_showing_revision() -> AppState {
    let mut app = cloud_run_app();
    reduce(&mut app, &Action::Resources(ResourcesAction::OpenSelected));
    let context = app.root.current_context().unwrap();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::CloudRunServiceDetailLoaded {
            context,
            detail: cr_detail(
                "api",
                "us-central1",
                vec![cr_revision("api-00001"), cr_revision("api-00002")],
            ),
        }),
    );
    // Summary -> Traffic -> Revisions.
    reduce(&mut app, &Action::Resources(ResourcesAction::SelectNextTab));
    reduce(&mut app, &Action::Resources(ResourcesAction::SelectNextTab));
    // Drill into the selected revision.
    reduce(&mut app, &Action::Resources(ResourcesAction::OpenSelected));
    assert!(app.root.resources.is_showing_cloud_run_revision());
    app
}

#[test]
fn cloud_run_open_service_detail_records_list_in_history() {
    let mut app = cloud_run_app();
    reduce(&mut app, &Action::Resources(ResourcesAction::OpenSelected));
    assert!(app.root.resources.is_showing_detail());
    assert!(app.root.navigation.can_go_back());
}

#[test]
fn cloud_run_back_from_service_detail_returns_to_list() {
    let mut app = cloud_run_app();
    reduce(&mut app, &Action::Resources(ResourcesAction::OpenSelected));
    reduce(&mut app, &Action::Navigation(NavAction::Back));
    assert!(!app.root.resources.is_showing_detail());
}

#[test]
fn cloud_run_forward_reopens_service_detail_and_loads() {
    let mut app = cloud_run_app();
    reduce(&mut app, &Action::Resources(ResourcesAction::OpenSelected));
    reduce(&mut app, &Action::Navigation(NavAction::Back));
    let effects = reduce(&mut app, &Action::Navigation(NavAction::Forward));
    assert!(app.root.resources.is_showing_detail());
    assert!(
        effects.iter().any(|e| matches!(
            e,
            Effect::LoadCloudRunServiceDetail { service, .. } if service.name == "api"
        )),
        "expected a service detail load effect, got {effects:?}"
    );
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, Effect::LoadCloudRunRevisions { .. }))
    );
}

#[test]
fn cloud_run_back_from_revision_returns_to_service_detail_on_revisions_tab() {
    let mut app = cloud_run_app_showing_revision();
    reduce(&mut app, &Action::Navigation(NavAction::Back));
    assert!(!app.root.resources.is_showing_cloud_run_revision());
    assert!(app.root.resources.is_showing_detail());
    assert_eq!(
        app.root.resources.active_cloud_run_detail_tab(),
        CloudRunDetailTab::Revisions
    );
    assert_eq!(
        app.root
            .resources
            .cloud_run_detail_target()
            .map(|s| s.name.as_str()),
        Some("api")
    );
}

#[test]
fn cloud_run_forward_reopens_revision_and_loads_revision_detail() {
    let mut app = cloud_run_app_showing_revision();
    reduce(&mut app, &Action::Navigation(NavAction::Back));
    let effects = reduce(&mut app, &Action::Navigation(NavAction::Forward));
    assert!(app.root.resources.is_showing_cloud_run_revision());
    assert_eq!(
        app.root
            .resources
            .cloud_run_revision_target()
            .map(|r| r.name.as_str()),
        Some("api-00001")
    );
    assert!(
        effects.iter().any(|e| matches!(
            e,
            Effect::LoadCloudRunRevisionDetail { revision, region, .. }
                if revision == "api-00001" && region == "us-central1"
        )),
        "expected a revision detail load effect, got {effects:?}"
    );
}
