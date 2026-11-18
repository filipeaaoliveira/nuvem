use nuvem::app::reducer::reduce;
use nuvem::app::state::root::Workspace;
use nuvem::app::{
    Action, AppError, Effect, EffectResult, NavAction, ResourcesAction, WorkspaceAction,
};
use nuvem_domain::gcp::configuration::{
    GcloudConfiguration, GcloudConfigurationName, GcloudContext,
};
use nuvem_domain::gcp::resource::{ComputeInstance, ResourceStatus};

fn context() -> GcloudContext {
    GcloudContext {
        configuration: GcloudConfigurationName::new("dev"),
        account: None,
        project: Some("proj".to_string()),
        region: None,
        zone: None,
    }
}

fn config() -> GcloudConfiguration {
    GcloudConfiguration {
        name: GcloudConfigurationName::new("dev"),
        is_active: true,
        account: None,
        project: Some("proj".to_string()),
        region: None,
        zone: None,
    }
}

fn instance(name: &str) -> ComputeInstance {
    ComputeInstance {
        id: name.to_string(),
        name: name.to_string(),
        zone: "us-central1-a".to_string(),
        status: ResourceStatus::Running,
        machine_type: "e2-medium".to_string(),
        internal_ip: Some("10.0.0.2".to_string()),
        external_ip: None,
    }
}

fn app_with_instances() -> nuvem::app::state::root::AppState {
    let mut app = nuvem::app::state::root::AppState::new();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ConfigurationsLoaded {
            configurations: vec![config()],
        }),
    );
    reduce(&mut app, &Action::Workspace(WorkspaceAction::Resources));
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ComputeInstancesLoaded {
            context: context(),
            instances: vec![instance("api-1"), instance("api-2")],
        }),
    );
    app
}

#[test]
fn opening_detail_records_list_in_back_history() {
    let mut app = app_with_instances();
    reduce(&mut app, &Action::Resources(ResourcesAction::OpenSelected));
    assert!(app.root.resources.is_showing_detail());
    assert!(app.root.navigation.can_go_back());
}

#[test]
fn back_from_detail_returns_to_list() {
    let mut app = app_with_instances();
    reduce(&mut app, &Action::Resources(ResourcesAction::OpenSelected));
    reduce(&mut app, &Action::Navigation(NavAction::Back));
    assert!(!app.root.resources.is_showing_detail());
    assert_eq!(app.root.workspace, Workspace::Resources);
    assert!(app.root.navigation.can_go_forward());
}

#[test]
fn forward_reopens_detail_and_emits_load_effect() {
    let mut app = app_with_instances();
    reduce(&mut app, &Action::Resources(ResourcesAction::OpenSelected));
    reduce(&mut app, &Action::Navigation(NavAction::Back));
    let effects = reduce(&mut app, &Action::Navigation(NavAction::Forward));
    assert!(app.root.resources.is_showing_detail());
    assert!(matches!(
        effects.as_slice(),
        [Effect::LoadComputeInstanceDetail { .. }]
    ));
}

#[test]
fn workspace_switch_records_history_and_back_restores() {
    let mut app = app_with_instances(); // currently Resources; back=[Home]
    reduce(
        &mut app,
        &Action::Workspace(WorkspaceAction::Configurations),
    );
    reduce(&mut app, &Action::Navigation(NavAction::Back));
    assert_eq!(app.root.workspace, Workspace::Resources);
}

#[test]
fn back_never_changes_active_configuration() {
    let mut app = app_with_instances();
    let before = app.root.configurations.active().cloned();
    reduce(&mut app, &Action::Resources(ResourcesAction::OpenSelected));
    reduce(&mut app, &Action::Navigation(NavAction::Back));
    reduce(&mut app, &Action::Navigation(NavAction::Forward));
    assert_eq!(app.root.configurations.active().cloned(), before);
}

#[test]
fn forward_to_deleted_detail_resolves_to_list_with_message() {
    let mut app = app_with_instances();
    reduce(&mut app, &Action::Resources(ResourcesAction::OpenSelected)); // detail api-1
    reduce(&mut app, &Action::Navigation(NavAction::Back));
    // reload without api-1
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ComputeInstancesLoaded {
            context: context(),
            instances: vec![instance("api-2")],
        }),
    );
    let effects = reduce(&mut app, &Action::Navigation(NavAction::Forward));
    assert!(!app.root.resources.is_showing_detail());
    assert!(effects.is_empty());
}

/// The other half of the pair above: forward into a detail whose *list* is
/// gone, not just its row.
///
/// A failed refresh clears the rows and the context stamp with them, so the
/// restore has nothing to match against — but "no longer available" would be
/// the wrong thing to say, because the failure, not a deletion, is why. The
/// jump picker pins this wording for its own caller; a back/forward restore is
/// the other one, and nothing else covered it.
///
/// The restore does *not* re-fetch the list. `apply_detail`'s contract is to
/// ensure the list backing the detail is being fetched, but a failed load is
/// already accounted for: the runtime holds it behind the
/// `should_auto_retry`/`gated_scope` backoff, and a fetch from here would be a
/// call that deliberately jumps that queue.
#[test]
fn forward_into_a_service_whose_list_failed_to_load_says_so_rather_than_claiming_it_vanished() {
    let mut app = app_with_instances();
    reduce(&mut app, &Action::Resources(ResourcesAction::OpenSelected)); // detail api-1
    reduce(&mut app, &Action::Navigation(NavAction::Back));
    // The refresh behind that list fails, which discards the rows and the
    // context stamp that says the list was ever loaded.
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ComputeInstancesLoadFailed {
            context: context(),
            error: AppError::gcloud("compute API disabled"),
        }),
    );

    let effects = reduce(&mut app, &Action::Navigation(NavAction::Forward));

    assert!(!app.root.resources.is_showing_detail());
    assert!(
        effects.is_empty(),
        "a failed load is the runtime's to retry, not this path's to re-issue"
    );
    assert_eq!(
        app.root.system.status_message.as_deref(),
        Some("No GCE instance loaded yet, so api-1 cannot be opened")
    );
}

/// The same failing service, held down rather than stepped through once.
///
/// `apply_detail` is the `Alt-←`/`Alt-→` path and keyboard auto-repeat turns a
/// held arrow into dozens of steps a second, so what is pinned is the *total*
/// across the whole sequence rather than a per-step verdict. Counting is also
/// what keeps the status-line assertion honest about how many restores the
/// stack really let through.
#[test]
fn oscillating_history_across_a_failing_services_detail_never_re_fetches_it() {
    let mut app = app_with_instances();
    reduce(&mut app, &Action::Resources(ResourcesAction::OpenSelected));
    reduce(&mut app, &Action::Navigation(NavAction::Back));
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ComputeInstancesLoadFailed {
            context: context(),
            error: AppError::gcloud("compute API disabled"),
        }),
    );

    let mut fetches = 0;
    for _ in 0..20 {
        for action in [NavAction::Forward, NavAction::Back] {
            fetches += reduce(&mut app, &Action::Navigation(action))
                .iter()
                .filter(|effect| matches!(effect, Effect::RefreshComputeInstances { .. }))
                .count();
        }
    }

    assert_eq!(
        fetches, 0,
        "a failing list must not be re-fetched from the history path"
    );
    assert_eq!(
        app.root.system.status_message.as_deref(),
        Some("No GCE instance loaded yet, so api-1 cannot be opened"),
        "the restore must actually have been attempted for the count to mean anything"
    );
}

/// The complement, on the ordinary path: a back/forward restore into a detail
/// whose list *is* loaded emits that resource's detail fetch and nothing more.
/// Without this, the fetch above would be free to become a refetch on every
/// `Alt-←`/`Alt-→` step through history.
#[test]
fn forward_into_a_loaded_detail_emits_only_the_detail_fetch() {
    let mut app = app_with_instances();
    reduce(&mut app, &Action::Resources(ResourcesAction::OpenSelected));
    reduce(&mut app, &Action::Navigation(NavAction::Back));

    let effects = reduce(&mut app, &Action::Navigation(NavAction::Forward));

    assert!(app.root.resources.is_showing_detail());
    assert_eq!(
        effects,
        vec![Effect::LoadComputeInstanceDetail {
            context: context(),
            instance: instance("api-1"),
        }]
    );
}

/// The anti-hammering guarantee has to survive a configuration switch, not
/// just hold in the configuration the app started in.
///
/// A `Failed` load phase outlives the configuration that recorded it, so the
/// predicate that reads it is context-scoped — and the risk of that scoping is
/// the opposite mistake: a failure that stops suppressing anything once *any*
/// switch has happened. This is the previous test again, one configuration
/// later, so that regression fails here rather than in the field.
#[test]
fn oscillating_history_in_a_newly_activated_configuration_still_never_re_fetches() {
    let mut app = app_with_instances();
    let prod = GcloudConfiguration {
        name: GcloudConfigurationName::new("prod"),
        is_active: true,
        account: None,
        project: Some("prod-project".to_string()),
        region: None,
        zone: None,
    };
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ConfigurationActivated {
            configuration: prod,
        }),
    );
    let context = app.root.current_context().expect("context exists");
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ComputeInstancesLoaded {
            context: context.clone(),
            instances: vec![instance("api-1"), instance("api-2")],
        }),
    );
    reduce(&mut app, &Action::Resources(ResourcesAction::OpenSelected));
    reduce(&mut app, &Action::Navigation(NavAction::Back));
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ComputeInstancesLoadFailed {
            context,
            error: AppError::gcloud("compute API disabled"),
        }),
    );

    let mut fetches = 0;
    for _ in 0..20 {
        for action in [NavAction::Forward, NavAction::Back] {
            fetches += reduce(&mut app, &Action::Navigation(action))
                .iter()
                .filter(|effect| matches!(effect, Effect::RefreshComputeInstances { .. }))
                .count();
        }
    }

    assert_eq!(
        fetches, 0,
        "a failure in the active configuration gates repeats there too"
    );
    assert_eq!(
        app.root.system.status_message.as_deref(),
        Some("No GCE instance loaded yet, so api-1 cannot be opened"),
        "the restore must actually have been attempted for the count to mean anything"
    );
}

#[test]
fn configuration_activation_resets_history() {
    let mut app = app_with_instances();
    reduce(&mut app, &Action::Resources(ResourcesAction::OpenSelected));
    assert!(app.root.navigation.can_go_back());
    let other = GcloudConfiguration {
        name: GcloudConfigurationName::new("prod"),
        is_active: true,
        account: None,
        project: Some("p2".to_string()),
        region: None,
        zone: None,
    };
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ConfigurationActivated {
            configuration: other,
        }),
    );
    assert!(!app.root.navigation.can_go_back());
    assert!(!app.root.navigation.can_go_forward());
}

// ---------------------------------------------------------------------------
// Tunnels workspace history
// ---------------------------------------------------------------------------

/// Opening Tunnels is an ordinary forward navigation: it records the previous
/// location, and Back returns there.
#[test]
fn opening_tunnels_records_history_and_back_returns_to_the_previous_workspace() {
    let mut app = app_with_instances();
    assert_eq!(app.root.workspace, Workspace::Resources);

    reduce(&mut app, &Action::Workspace(WorkspaceAction::Tunnels));
    assert_eq!(app.root.workspace, Workspace::Tunnels);
    assert!(app.root.navigation.can_go_back());

    let effects = reduce(&mut app, &Action::Navigation(NavAction::Back));
    assert!(effects.is_empty());
    assert_eq!(app.root.workspace, Workspace::Resources);
    assert!(app.root.navigation.can_go_forward());
}

/// Forward through history restores the Tunnels workspace without fetching
/// anything: the list is maintained by the runtime's supervision results.
#[test]
fn forward_restores_tunnels_without_triggering_a_load() {
    let mut app = app_with_instances();
    reduce(&mut app, &Action::Workspace(WorkspaceAction::Tunnels));
    reduce(&mut app, &Action::Navigation(NavAction::Back));

    let effects = reduce(&mut app, &Action::Navigation(NavAction::Forward));

    assert!(effects.is_empty(), "restoring Tunnels must fetch nothing");
    assert_eq!(app.root.workspace, Workspace::Tunnels);
}

/// Re-selecting the workspace you are already in is not a navigation event, so
/// it must not push a duplicate history entry (this is what `r` does in the
/// Tunnels workspace).
#[test]
fn re_selecting_tunnels_from_tunnels_records_no_history_entry() {
    let mut app = app_with_instances();
    reduce(&mut app, &Action::Workspace(WorkspaceAction::Tunnels));
    reduce(&mut app, &Action::Workspace(WorkspaceAction::Tunnels));

    // One Back leaves Tunnels entirely; a second reaches Home. A duplicate
    // Tunnels entry would have made the first Back a no-op stay.
    reduce(&mut app, &Action::Navigation(NavAction::Back));
    assert_eq!(app.root.workspace, Workspace::Resources);
    reduce(&mut app, &Action::Navigation(NavAction::Back));
    assert_eq!(app.root.workspace, Workspace::Home);
    assert!(!app.root.navigation.can_go_back());
}
