use nuvem::app::projection::{NavigatorFocus, project_app_frame, project_service_navigator};
use nuvem::app::reducer::root::reduce;
use nuvem::app::service_registry::{ServiceAvailability, ServiceId};
use nuvem::app::{Action, AppState, NavigatorAction, Workspace};
use ratatui::layout::Rect;

#[test]
fn projects_grouped_services_with_active_selection_and_disabled_reason() {
    let root = nuvem::app::RootState::default();
    let projection = project_service_navigator(&root, Rect::new(0, 0, 32, 8));

    assert_eq!(projection.selected_id, ServiceId::ComputeInstances);
    // Categories are ordered alphabetically, so "Analytics" — BigQuery
    // datasets — is the first header and "Compute" is the
    // third entry, behind that header and its one service.
    assert_eq!(projection.entries[0].category, Some("Analytics"));
    assert!(!projection.entries[0].selectable);
    assert_eq!(projection.entries[2].category, Some("Compute"));
    assert!(!projection.entries[2].selectable);
    assert!(
        projection
            .entries
            .iter()
            .any(|entry| { entry.label == "Cloud Run" && entry.unavailable_reason.is_none() })
    );
}

#[test]
fn navigator_projection_exposes_service_metadata_and_navigation_shell_state() {
    let mut app = AppState::new();
    app.root.workspace = Workspace::Resources;
    reduce(&mut app, &Action::Navigator(NavigatorAction::StartFilter));
    reduce(
        &mut app,
        &Action::Navigator(NavigatorAction::AppendFilterChar('g')),
    );

    let projection = project_service_navigator(&app.root, Rect::new(0, 0, 32, 8));

    assert_eq!(projection.filter.draft, "g");
    assert_eq!(projection.filter.applied, "");
    assert!(projection.filter.active);
    assert!(projection.sidebar_visible);
    assert_eq!(projection.focus, NavigatorFocus::Resources);

    let gce = projection
        .entries
        .iter()
        .find(|entry| entry.id == Some(ServiceId::ComputeInstances))
        .expect("GCE entry should be projected");
    assert!(gce.is_active);
    assert_eq!(gce.availability, Some(ServiceAvailability::Available));
    assert_eq!(gce.unavailable_reason, None);

    let cloud_run = projection
        .entries
        .iter()
        .find(|entry| entry.id == Some(ServiceId::CloudRun))
        .expect("Cloud Run entry should be projected");
    assert!(!cloud_run.is_active);
    assert_eq!(cloud_run.availability, Some(ServiceAvailability::Available));
    assert_eq!(cloud_run.unavailable_reason, None);
}

#[test]
fn category_headers_are_non_selectable_and_selected_service_maps_to_entry_index() {
    let mut app = AppState::new();
    reduce(&mut app, &Action::Navigator(NavigatorAction::SelectNext));

    let projection = project_service_navigator(&app.root, Rect::new(0, 0, 32, 8));

    assert_eq!(projection.selected_id, ServiceId::GkeClusters);
    // Entries 0-1 are the "Analytics" header and BigQuery datasets, 2-3 the
    // "Compute" header and GCE instances, so "Containers" lands at 4.
    assert_eq!(projection.entries[4].category, Some("Containers"));
    assert!(!projection.entries[4].selectable);
    assert_eq!(projection.entries[5].id, Some(ServiceId::GkeClusters));
    assert_eq!(projection.selected_visible, 5);
}

#[test]
fn navigator_viewport_stays_within_body_height_and_keeps_selection_visible() {
    let mut app = AppState::new();
    reduce(&mut app, &Action::Navigator(NavigatorAction::SelectNext));
    reduce(&mut app, &Action::Navigator(NavigatorAction::SelectNext));
    let projection = project_service_navigator(&app.root, Rect::new(0, 0, 32, 3));

    assert!(projection.viewport_end <= projection.entries.len());
    assert!(projection.selected_visible >= projection.viewport_start);
    assert!(projection.selected_visible < projection.viewport_end);
    assert_eq!(projection.viewport_end - projection.viewport_start, 1);
}

#[test]
fn resources_frame_keeps_typed_service_while_navigator_projects_registry_selection() {
    let mut app = AppState::new();
    app.root.workspace = Workspace::Resources;
    reduce(&mut app, &Action::Navigator(NavigatorAction::SelectNext));

    let navigator = project_service_navigator(&app.root, Rect::new(0, 0, 32, 8));
    let frame = project_app_frame(&app.root);

    assert_eq!(navigator.selected_id, ServiceId::GkeClusters);
    assert_eq!(frame.service_label, Some("GCE instances"));
}
