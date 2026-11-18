use crate::support::remembered_view::remembered;
use nuvem::app::reducer::root::reduce;
use nuvem::app::state::resources::CloudRunDetailTab;
use nuvem::app::{Action, AppError, AppState, Effect, EffectResult, ResourcesAction};
use nuvem_domain::gcp::configuration::GcloudConfiguration;
use nuvem_domain::gcp::configuration::GcloudConfigurationName;
use nuvem_domain::gcp::diagnostics::GcloudErrorKind;
use nuvem_domain::gcp::resource::{
    CloudRunCondition, CloudRunRevision, CloudRunService, CloudRunServiceDetail,
    CloudRunServiceStatus, ResourceServiceKind,
};

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

fn service(name: &str, region: &str) -> CloudRunService {
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

fn revision(name: &str) -> CloudRunRevision {
    CloudRunRevision {
        name: name.to_string(),
        ready: CloudRunServiceStatus::Ready,
        created: None,
        image: Some("gcr.io/x/y:latest".to_string()),
        active_percent: Some(100),
    }
}

fn detail_with_revisions(
    name: &str,
    region: &str,
    revisions: Vec<CloudRunRevision>,
) -> CloudRunServiceDetail {
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

/// AppState with an active configuration and Cloud Run selected as the active
/// service. Returns the app with no rows loaded yet.
fn app_with_cloud_run_selected() -> AppState {
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
            ResourceServiceKind::CloudRunServices,
        )),
    );
    app
}

/// AppState with Cloud Run selected and `services` loaded into the collection.
fn app_with_cloud_run_services(services: Vec<CloudRunService>) -> AppState {
    let mut app = app_with_cloud_run_selected();
    let context = app.root.current_context().unwrap();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::CloudRunServicesLoaded { context, services }),
    );
    app
}

#[test]
fn select_service_cloud_run_emits_refresh_effect() {
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
            ResourceServiceKind::CloudRunServices,
        )),
    );

    assert_eq!(
        app.root.resources.active_service(),
        ResourceServiceKind::CloudRunServices
    );
    // The save is the service change being remembered against "dev":
    // the choke point in `reduce` learns every service change, whichever path
    // made it.
    assert_eq!(
        effects,
        vec![
            Effect::RefreshCloudRunServices {
                context: app.root.current_context().unwrap(),
            },
            Effect::SaveRememberedViews {
                views: remembered("dev", ResourceServiceKind::CloudRunServices),
            }
        ]
    );
}

#[test]
fn refresh_emits_cloud_run_effect_when_cloud_run_is_active() {
    let mut app = app_with_cloud_run_selected();

    let effects = reduce(&mut app, &Action::Resources(ResourcesAction::Refresh));

    assert_eq!(
        effects,
        vec![Effect::RefreshCloudRunServices {
            context: app.root.current_context().unwrap(),
        }]
    );
}

#[test]
fn open_selected_service_enters_detail_and_emits_detail_and_revisions() {
    let mut app = app_with_cloud_run_services(vec![service("api", "us-central1")]);

    let effects = reduce(&mut app, &Action::Resources(ResourcesAction::OpenSelected));

    assert!(app.root.resources.is_showing_detail());
    let context = app.root.current_context().unwrap();
    assert_eq!(
        effects,
        vec![
            Effect::LoadCloudRunServiceDetail {
                context: context.clone(),
                service: service("api", "us-central1"),
            },
            Effect::LoadCloudRunRevisions {
                context,
                service: service("api", "us-central1"),
            },
        ]
    );
}

#[test]
fn refresh_while_showing_service_detail_reloads_revisions() {
    let mut app = app_with_cloud_run_services(vec![service("api", "us-central1")]);
    reduce(&mut app, &Action::Resources(ResourcesAction::OpenSelected));
    assert!(app.root.resources.is_showing_detail());

    let effects = reduce(&mut app, &Action::Resources(ResourcesAction::Refresh));

    let context = app.root.current_context().unwrap();
    assert_eq!(
        effects,
        vec![
            Effect::RefreshCloudRunServices {
                context: context.clone(),
            },
            Effect::LoadCloudRunServiceDetail {
                context: context.clone(),
                service: service("api", "us-central1"),
            },
            Effect::LoadCloudRunRevisions {
                context,
                service: service("api", "us-central1"),
            },
        ]
    );
    assert!(
        effects
            .iter()
            .any(|effect| matches!(effect, Effect::LoadCloudRunRevisions { .. }))
    );
}

#[test]
fn open_selected_on_revisions_tab_drills_into_selected_revision() {
    let mut app = app_with_cloud_run_services(vec![service("api", "us-central1")]);
    // Open the service detail.
    reduce(&mut app, &Action::Resources(ResourcesAction::OpenSelected));
    // Populate the detail with an authoritative revision list.
    let context = app.root.current_context().unwrap();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::CloudRunServiceDetailLoaded {
            context: context.clone(),
            detail: detail_with_revisions(
                "api",
                "us-central1",
                vec![revision("api-00001"), revision("api-00002")],
            ),
        }),
    );
    // Move to the Revisions tab: Summary -> Traffic -> Revisions.
    reduce(&mut app, &Action::Resources(ResourcesAction::SelectNextTab));
    reduce(&mut app, &Action::Resources(ResourcesAction::SelectNextTab));
    assert_eq!(
        app.root.resources.active_cloud_run_detail_tab(),
        CloudRunDetailTab::Revisions
    );

    let effects = reduce(&mut app, &Action::Resources(ResourcesAction::OpenSelected));

    assert!(app.root.resources.is_showing_cloud_run_revision());
    assert_eq!(
        effects,
        vec![Effect::LoadCloudRunRevisionDetail {
            context,
            region: "us-central1".to_string(),
            revision: "api-00001".to_string(),
        }]
    );
}

#[test]
fn select_next_on_revisions_tab_then_open_drills_into_second_revision() {
    let mut app = app_with_cloud_run_services(vec![service("api", "us-central1")]);
    reduce(&mut app, &Action::Resources(ResourcesAction::OpenSelected));
    let context = app.root.current_context().unwrap();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::CloudRunServiceDetailLoaded {
            context: context.clone(),
            detail: detail_with_revisions(
                "api",
                "us-central1",
                vec![revision("api-00001"), revision("api-00002")],
            ),
        }),
    );
    reduce(&mut app, &Action::Resources(ResourcesAction::SelectNextTab));
    reduce(&mut app, &Action::Resources(ResourcesAction::SelectNextTab));
    assert_eq!(
        app.root.resources.active_cloud_run_detail_tab(),
        CloudRunDetailTab::Revisions
    );
    assert_eq!(app.root.resources.selected_cloud_run_revision_index(), 0);

    reduce(&mut app, &Action::Resources(ResourcesAction::SelectNext));
    assert_eq!(app.root.resources.selected_cloud_run_revision_index(), 1);

    let effects = reduce(&mut app, &Action::Resources(ResourcesAction::OpenSelected));

    assert!(app.root.resources.is_showing_cloud_run_revision());
    assert_eq!(
        effects,
        vec![Effect::LoadCloudRunRevisionDetail {
            context,
            region: "us-central1".to_string(),
            revision: "api-00002".to_string(),
        }]
    );
}

#[test]
fn cloud_run_services_loaded_populates_rows() {
    let app = app_with_cloud_run_services(vec![
        service("api", "us-central1"),
        service("web", "europe-west1"),
    ]);

    assert_eq!(app.root.resources.cloud_run_services().len(), 2);
    assert!(app.root.resources.cloud_run_loaded());
    assert!(!app.root.resources.cloud_run_stale());
}

#[test]
fn cloud_run_services_load_failed_marks_stale_and_records_problem() {
    let mut app = app_with_cloud_run_services(vec![service("api", "us-central1")]);
    let context = app.root.current_context().unwrap();

    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::CloudRunServicesLoadFailed {
            context,
            error: AppError::gcloud("PERMISSION_DENIED: Cloud Run Admin API has not been used"),
        }),
    );

    assert!(app.root.resources.cloud_run_stale());
    assert!(app.root.resources.cloud_run_services().is_empty());
    let problem = app.root.resources.cloud_run_problem().unwrap();
    assert_eq!(problem.kind, GcloudErrorKind::PermissionDenied);
}

#[test]
fn cloud_run_service_detail_loaded_stores_detail_for_open_target() {
    let mut app = app_with_cloud_run_services(vec![service("api", "us-central1")]);
    reduce(&mut app, &Action::Resources(ResourcesAction::OpenSelected));
    let context = app.root.current_context().unwrap();

    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::CloudRunServiceDetailLoaded {
            context,
            detail: detail_with_revisions("api", "us-central1", vec![revision("api-00001")]),
        }),
    );

    let detail = app.root.resources.cloud_run_detail().unwrap();
    assert_eq!(detail.name, "api");
    assert_eq!(detail.revisions.len(), 1);
}

#[test]
fn cloud_run_revisions_loaded_refreshes_open_detail_revision_list() {
    let mut app = app_with_cloud_run_services(vec![service("api", "us-central1")]);
    reduce(&mut app, &Action::Resources(ResourcesAction::OpenSelected));
    let context = app.root.current_context().unwrap();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::CloudRunServiceDetailLoaded {
            context: context.clone(),
            detail: detail_with_revisions("api", "us-central1", vec![revision("api-00001")]),
        }),
    );

    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::CloudRunRevisionsLoaded {
            context,
            service_name: "api".to_string(),
            service_region: "us-central1".to_string(),
            revisions: vec![
                revision("api-00002"),
                revision("api-00001"),
                revision("api-00000"),
            ],
        }),
    );

    let detail = app.root.resources.cloud_run_detail().unwrap();
    assert_eq!(detail.revisions.len(), 3);
    assert_eq!(detail.revisions[0].name, "api-00002");
}

#[test]
fn cloud_run_revision_detail_loaded_stores_detail_while_showing_revision() {
    let mut app = app_with_cloud_run_services(vec![service("api", "us-central1")]);
    reduce(&mut app, &Action::Resources(ResourcesAction::OpenSelected));
    let context = app.root.current_context().unwrap();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::CloudRunServiceDetailLoaded {
            context: context.clone(),
            detail: detail_with_revisions("api", "us-central1", vec![revision("api-00001")]),
        }),
    );
    reduce(&mut app, &Action::Resources(ResourcesAction::SelectNextTab));
    reduce(&mut app, &Action::Resources(ResourcesAction::SelectNextTab));
    reduce(&mut app, &Action::Resources(ResourcesAction::OpenSelected));

    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::CloudRunRevisionDetailLoaded {
            context,
            detail: nuvem_domain::gcp::resource::CloudRunRevisionDetail {
                name: "api-00001".to_string(),
                service: Some("api".to_string()),
                region: "us-central1".to_string(),
                ready: CloudRunServiceStatus::Ready,
                conditions: Vec::new(),
                image: Some("gcr.io/x/y:latest".to_string()),
                cpu: None,
                memory: None,
                concurrency: None,
                min_instances: None,
                max_instances: None,
                service_account: None,
                created: None,
                raw_json: "{}".to_string(),
            },
        }),
    );

    let detail = app.root.resources.cloud_run_revision_detail().unwrap();
    assert_eq!(detail.name, "api-00001");
}
