use crate::support::remembered_view::remembered;
use nuvem::app::reducer::root::reduce;
use nuvem::app::service_registry::ServiceId;
use nuvem::app::state::navigator::NavigatorFocus;
use nuvem::app::{Action, AppState, Effect, NavigatorAction, Workspace};
use nuvem_domain::gcp::configuration::{GcloudConfiguration, GcloudConfigurationName};
use nuvem_domain::gcp::resource::ResourceServiceKind;

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

fn app_with_active_configuration() -> AppState {
    let mut app = AppState::new();
    reduce(
        &mut app,
        &Action::EffectResult(nuvem::app::EffectResult::ConfigurationsLoaded {
            configurations: vec![config("dev", true, "dev-project")],
        }),
    );
    app
}

#[test]
fn navigator_moves_and_filters_registered_services() {
    let mut app = AppState::new();
    reduce(&mut app, &Action::Navigator(NavigatorAction::SelectNext));
    assert_eq!(app.root.navigator.selected_id(), ServiceId::GkeClusters);

    reduce(&mut app, &Action::Navigator(NavigatorAction::StartFilter));
    for value in "kubernetes".chars() {
        reduce(
            &mut app,
            &Action::Navigator(NavigatorAction::AppendFilterChar(value)),
        );
    }
    reduce(&mut app, &Action::Navigator(NavigatorAction::ApplyFilter));

    assert_eq!(
        app.root.navigator.visible_ids(),
        vec![ServiceId::GkeClusters]
    );
    assert_eq!(app.root.navigator.selected_id(), ServiceId::GkeClusters);
}

#[test]
fn opening_available_service_switches_workspace_and_loads_it_directly() {
    let mut app = app_with_active_configuration();
    reduce(&mut app, &Action::Navigator(NavigatorAction::SelectNext));
    let effects = reduce(&mut app, &Action::Navigator(NavigatorAction::OpenSelected));

    assert_eq!(app.root.workspace, Workspace::Resources);
    assert_eq!(
        app.root.resources.active_service(),
        ResourceServiceKind::GkeClusters
    );
    // The save is the service change being remembered against "dev":
    // the navigator's open is one of `select_service`'s four call sites, and
    // the choke point in `reduce` covers it without the site knowing.
    assert_eq!(
        effects,
        vec![
            Effect::RefreshGkeClusters {
                context: app.root.current_context().unwrap(),
                filter: None,
            },
            Effect::SaveRememberedViews {
                views: remembered("dev", ResourceServiceKind::GkeClusters),
            }
        ]
    );
}

/// Opening a service from the navigator selects it first, so the load it emits
/// refreshes the *active* service. Under label mode that reload is not a
/// superset the client narrows further — `CompiledFilter::Label` keeps every
/// row — so it must carry that service's own applied filter, or the filter
/// silently stops applying while the tools line still shows it.
#[test]
fn opening_a_service_reloads_it_with_its_own_applied_filter() {
    let mut app = app_with_active_configuration();

    // Apply `-l env=prod` while GCE is the active service.
    reduce(
        &mut app,
        &Action::Resources(nuvem::app::ResourcesAction::StartFilter),
    );
    for value in "-l env=prod".chars() {
        reduce(
            &mut app,
            &Action::Resources(nuvem::app::ResourcesAction::AppendFilterChar(value)),
        );
    }
    reduce(
        &mut app,
        &Action::Resources(nuvem::app::ResourcesAction::ApplyFilter),
    );

    // GKE carries no filter of its own.
    reduce(&mut app, &Action::Navigator(NavigatorAction::SelectNext));
    let gke = reduce(&mut app, &Action::Navigator(NavigatorAction::OpenSelected));
    // Each open also remembers the service it landed on against "dev";
    // the filter each load carries is what this test is about and is
    // unchanged.
    assert_eq!(
        gke,
        vec![
            Effect::RefreshGkeClusters {
                context: app.root.current_context().unwrap(),
                filter: None,
            },
            Effect::SaveRememberedViews {
                views: remembered("dev", ResourceServiceKind::GkeClusters),
            }
        ]
    );

    // Back to GCE: its own applied filter must be pushed again.
    reduce(
        &mut app,
        &Action::Navigator(NavigatorAction::SelectPrevious),
    );
    let compute = reduce(&mut app, &Action::Navigator(NavigatorAction::OpenSelected));
    assert_eq!(
        compute,
        vec![
            Effect::RefreshComputeInstances {
                context: app.root.current_context().unwrap(),
                filter: Some("-l env=prod".to_string()),
            },
            Effect::SaveRememberedViews {
                views: remembered("dev", ResourceServiceKind::ComputeInstances),
            }
        ]
    );
}

#[test]
fn opening_cloud_run_switches_workspace_and_loads_it_directly() {
    // Cloud Run is now Available (Task 10): opening it from the navigator must
    // switch to the Resources workspace and emit its refresh effect, exactly
    // like the other live services.
    //
    // The step count follows the navigator's own order — category first, then
    // `default_sort`, then label — so it moves whenever a service is added in a
    // category that sorts earlier. Pub/Sub's two "Messaging" tables, the
    // three "Networking" VPC tables and the three "Security" services
    // (Secret Manager, then Service accounts, then the IAM policy) all sort
    // between "Databases" and "Serverless", which put Cloud Run twelfth:
    // GCE -> GKE -> Cloud SQL -> Pub/Sub topics -> Pub/Sub subscriptions ->
    // VPC networks -> VPC subnets -> Firewall rules -> Secret Manager ->
    // Service accounts -> IAM policy -> Cloud Run.
    let mut app = app_with_active_configuration();
    for _ in 0..11 {
        reduce(&mut app, &Action::Navigator(NavigatorAction::SelectNext));
    }
    assert_eq!(app.root.navigator.selected_id(), ServiceId::CloudRun);

    let effects = reduce(&mut app, &Action::Navigator(NavigatorAction::OpenSelected));

    assert_eq!(app.root.workspace, Workspace::Resources);
    assert_eq!(
        app.root.resources.active_service(),
        ResourceServiceKind::CloudRunServices
    );
    // Plus the remembered-view save for the service this landed on.
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
fn opening_with_no_visible_filtered_services_does_not_open_previous_selection() {
    let mut app = app_with_active_configuration();
    reduce(&mut app, &Action::Navigator(NavigatorAction::SelectNext));
    reduce(&mut app, &Action::Navigator(NavigatorAction::StartFilter));
    for value in "does-not-match".chars() {
        reduce(
            &mut app,
            &Action::Navigator(NavigatorAction::AppendFilterChar(value)),
        );
    }
    reduce(&mut app, &Action::Navigator(NavigatorAction::ApplyFilter));

    assert!(app.root.navigator.visible_ids().is_empty());
    let effects = reduce(&mut app, &Action::Navigator(NavigatorAction::OpenSelected));

    assert!(effects.is_empty());
    assert_eq!(app.root.workspace, Workspace::Home);
    assert_eq!(
        app.root.resources.active_service(),
        ResourceServiceKind::ComputeInstances
    );
}

#[test]
fn hiding_sidebar_resets_navigator_focus_to_resources() {
    let mut app = AppState::new();
    assert!(app.root.navigator.sidebar_visible());
    reduce(&mut app, &Action::Navigator(NavigatorAction::ToggleFocus));
    assert_eq!(app.root.navigator.focus(), NavigatorFocus::Navigator);
    reduce(&mut app, &Action::Navigator(NavigatorAction::ToggleSidebar));
    assert!(!app.root.navigator.sidebar_visible());
    assert_eq!(app.root.navigator.focus(), NavigatorFocus::Resources);
}
