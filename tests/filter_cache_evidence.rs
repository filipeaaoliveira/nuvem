//! The resource table's applied filter is compiled once per apply
//! and cached, instead of being recompiled on every render.
//!
//! These tests drive the real reducer through the real filter actions, so the
//! applied pattern only ever reaches the state through the one writer that
//! recompiles. They are staleness tests, not "the filter still filters" tests:
//! each one applies a pattern, then applies a *different* pattern through the
//! same path and asserts the rows follow the new one. A cache that stored the
//! new source while keeping the old compiled filter fails them.

use nuvem::app::reducer::root::reduce;
use nuvem::app::{Action, AppState, EffectResult, ResourcesAction};
use nuvem_domain::gcp::configuration::{GcloudConfiguration, GcloudConfigurationName};
use nuvem_domain::gcp::resource::{ComputeInstance, ResourceStatus};

fn config(name: &str, project: &str) -> GcloudConfiguration {
    GcloudConfiguration {
        name: GcloudConfigurationName::new(name),
        is_active: true,
        account: None,
        project: Some(project.to_string()),
        region: None,
        zone: None,
    }
}

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

fn app_with_instances(names: &[&str]) -> AppState {
    let mut app = AppState::new();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ConfigurationsLoaded {
            configurations: vec![config("dev", "dev-project")],
        }),
    );
    let context = app.root.current_context().unwrap().clone();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ComputeInstancesLoaded {
            context,
            instances: names.iter().copied().map(instance).collect(),
        }),
    );
    app
}

/// Types `input` into the active service's filter input and applies it — the
/// only path that writes the applied pattern.
fn apply_filter_input(app: &mut AppState, input: &str) {
    reduce(app, &Action::Resources(ResourcesAction::StartFilter));
    // `StartFilter` seeds the draft with the currently applied pattern, so a
    // re-apply has to clear it the way the user does.
    while !app.root.resources.filter_draft().is_empty() {
        reduce(app, &Action::Resources(ResourcesAction::BackspaceFilter));
    }
    for ch in input.chars() {
        reduce(
            app,
            &Action::Resources(ResourcesAction::AppendFilterChar(ch)),
        );
    }
    reduce(app, &Action::Resources(ResourcesAction::ApplyFilter));
}

fn visible_names(app: &AppState) -> Vec<String> {
    app.root
        .resources
        .visible_instances()
        .into_iter()
        .map(|instance| instance.name.clone())
        .collect()
}

#[test]
fn re_applying_a_different_pattern_recompiles_instead_of_reusing_the_cached_filter() {
    let mut app = app_with_instances(&["api-dev-1", "worker-dev-1", "api-dev-2"]);

    apply_filter_input(&mut app, "^api-");
    assert_eq!(app.root.resources.filter_applied(), "^api-");
    assert_eq!(visible_names(&app), vec!["api-dev-1", "api-dev-2"]);

    // Second apply through the same path. A cache that kept the first
    // compiled filter would still be showing the `api-` rows here.
    apply_filter_input(&mut app, "^worker-");
    assert_eq!(app.root.resources.filter_applied(), "^worker-");
    assert_eq!(visible_names(&app), vec!["worker-dev-1"]);

    // And back to a pattern that narrows nothing.
    apply_filter_input(&mut app, "");
    assert_eq!(app.root.resources.filter_applied(), "");
    assert_eq!(
        visible_names(&app),
        vec!["api-dev-1", "api-dev-2", "worker-dev-1"]
    );
}

#[test]
fn the_row_count_accessor_follows_the_newly_applied_pattern_too() {
    let mut app = app_with_instances(&["api-dev-1", "worker-dev-1", "api-dev-2"]);

    apply_filter_input(&mut app, "^api-");
    assert_eq!(app.root.resources.visible_instance_count(), 2);

    apply_filter_input(&mut app, "^worker-");
    assert_eq!(app.root.resources.visible_instance_count(), 1);
}

/// Error-path parity: a pattern that will not compile narrows *nothing* —
/// every loaded row stays visible — and the compile error is surfaced inline.
/// An empty table behind a bad regex is indistinguishable from a correct empty
/// result, so showing too much beside the error is the safe failure.
#[test]
fn an_uncompilable_pattern_keeps_every_row_visible_and_still_reports_the_error() {
    let mut app = app_with_instances(&["api-dev-1", "worker-dev-1"]);

    apply_filter_input(&mut app, "a(b");

    assert_eq!(visible_names(&app), vec!["api-dev-1", "worker-dev-1"]);
    assert_eq!(app.root.resources.visible_instance_count(), 2);
    assert!(
        app.root
            .resources
            .filter_error()
            .is_some_and(|error| error.contains("invalid regex"))
    );

    // Recovering from the bad pattern must recompile, not keep the fallback.
    apply_filter_input(&mut app, "^api-");
    assert_eq!(app.root.resources.filter_error(), None);
    assert_eq!(visible_names(&app), vec!["api-dev-1"]);
}
