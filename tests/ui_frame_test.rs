use chrono::{Local, TimeZone};
use nuvem::app::projection::{
    AppFrameProjection, FrameContextProjection, FrameHint, FrameOperationProjection,
    format_last_refresh, project_app_frame,
};
use nuvem::app::reducer::root::reduce;
use nuvem::app::state::system::{OperationPhase, OperationState, SystemState};
use nuvem::app::{
    Action, AppError, AppState, ComputeInstanceAction, EffectResult, NavigatorAction,
    ResourcesAction, RootState, Workspace, WorkspaceAction,
};
use nuvem::services::runtime::session::RuntimeSession;
use nuvem::ui::shared::components::{fit_text, status_line};
use nuvem::ui::shared::frame::{
    render_breadcrumb, render_footer, render_header, render_status, split_frame,
};
use nuvem::ui::theme;
use nuvem_domain::gcp::configuration::{GcloudConfiguration, GcloudConfigurationName};
use nuvem_domain::gcp::resource::{
    ComputeInstance, ResourceServiceKind, ResourceStatus, StorageBucket, StorageObjectPage,
    StoragePrefix,
};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::layout::Rect;
use ratatui::style::Color;

fn configuration(
    name: &str,
    is_active: bool,
    account: Option<&str>,
    project: Option<&str>,
    region: Option<&str>,
    zone: Option<&str>,
) -> GcloudConfiguration {
    GcloudConfiguration {
        name: GcloudConfigurationName::new(name),
        is_active,
        account: account.map(ToString::to_string),
        project: project.map(ToString::to_string),
        region: region.map(ToString::to_string),
        zone: zone.map(ToString::to_string),
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

fn storage_prefix(name: &str) -> StoragePrefix {
    StoragePrefix {
        name: name.to_string(),
    }
}

#[test]
fn system_state_starts_idle_without_a_refresh_time() {
    let state = SystemState::default();
    assert_eq!(state.operation, OperationState::default());
    assert_eq!(state.status_operation_label, None);
    assert_eq!(state.last_refresh_at, None);
}

#[test]
fn system_state_records_test_controlled_refresh_time() {
    let mut state = SystemState::default();
    state.begin_operation(OperationPhase::Refreshing, "GCE instances");
    let refresh_at = Local.with_ymd_and_hms(2026, 7, 16, 14, 30, 0).unwrap();
    state.complete_refresh_operation(OperationPhase::Healthy, "GCE instances", refresh_at);
    assert_eq!(state.operation.phase, OperationPhase::Healthy);
    assert_eq!(state.last_refresh_at, Some(refresh_at));
}

#[test]
fn starting_a_new_operation_clears_previous_status_feedback() {
    let mut state = SystemState::default();
    state.set_error_status_message("previous operation failed");

    state.begin_operation(OperationPhase::Loading, "GCE detail");

    assert_eq!(state.status_message, None);
    assert_eq!(state.status_operation_label, None);
    assert_eq!(state.status_phase, None);
}

#[test]
fn system_state_keeps_refresh_time_when_a_non_refresh_operation_completes() {
    let mut state = SystemState::default();
    state.begin_operation(OperationPhase::Refreshing, "GCE instances");
    let refresh_at = Local.with_ymd_and_hms(2026, 7, 16, 14, 30, 0).unwrap();
    state.complete_refresh_operation(OperationPhase::Healthy, "GCE instances", refresh_at);
    state.begin_operation(OperationPhase::Loading, "GCE detail");
    state.complete_operation(OperationPhase::Healthy, "GCE detail");

    assert_eq!(state.operation.phase, OperationPhase::Healthy);
    assert_eq!(state.operation.label, "GCE detail");
    assert_eq!(state.last_refresh_at, Some(refresh_at));
}

#[test]
fn frame_projection_prioritizes_configuration_and_project() {
    let mut root = RootState {
        workspace: Workspace::Resources,
        ..Default::default()
    };
    root.configurations.set_rows(vec![configuration(
        "dev",
        true,
        Some("dev@example.com"),
        Some("dev-project"),
        Some("europe-west1"),
        Some("europe-west1-b"),
    )]);

    let projection = project_app_frame(&root);

    assert_eq!(projection.view_title, "Resources");
    assert_eq!(projection.context.configuration.as_deref(), Some("dev"));
    assert_eq!(projection.context.project.as_deref(), Some("dev-project"));
    assert_eq!(
        projection.breadcrumbs,
        vec!["Home", "Resources", "GCE instances"]
    );
}

/// The breadcrumb for a Cloud Storage object browser nested two levels deep
/// lists the bucket followed by each prefix segment (without its trailing
/// slash), in drill-down order.
#[test]
fn cloud_storage_breadcrumb_reflects_nested_prefix_path() {
    let mut app = AppState::new();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ConfigurationsLoaded {
            configurations: vec![configuration(
                "dev",
                true,
                Some("dev@example.com"),
                Some("dev-project"),
                Some("europe-west1"),
                Some("europe-west1-b"),
            )],
        }),
    );
    let context = app.root.current_context().unwrap().clone();
    reduce(&mut app, &Action::Workspace(WorkspaceAction::Resources));

    reduce(
        &mut app,
        &Action::Resources(ResourcesAction::SelectService(
            ResourceServiceKind::CloudStorage,
        )),
    );
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::StorageBucketsLoaded {
            context: context.clone(),
            buckets: vec![storage_bucket("alpha-bucket")],
        }),
    );
    reduce(&mut app, &Action::Resources(ResourcesAction::OpenSelected));
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::StorageObjectPageLoaded {
            context: context.clone(),
            bucket: "alpha-bucket".to_string(),
            prefix: String::new(),
            append: false,
            page: StorageObjectPage {
                prefixes: vec![storage_prefix("photos/")],
                objects: Vec::new(),
                next_page_token: None,
            },
        }),
    );
    reduce(&mut app, &Action::Resources(ResourcesAction::OpenSelected));
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::StorageObjectPageLoaded {
            context,
            bucket: "alpha-bucket".to_string(),
            prefix: "photos/".to_string(),
            append: false,
            page: StorageObjectPage {
                prefixes: vec![storage_prefix("photos/2024/")],
                objects: Vec::new(),
                next_page_token: None,
            },
        }),
    );
    reduce(&mut app, &Action::Resources(ResourcesAction::OpenSelected));

    let projection = project_app_frame(&app.root);
    assert_eq!(
        projection.breadcrumbs,
        vec![
            "Home",
            "Resources",
            "Cloud Storage",
            "alpha-bucket",
            "photos",
            "2024",
        ]
    );
}

/// A prefix segment carrying a raw control character must never reach the
/// breadcrumb verbatim -- `sanitize_display_text` replaces it with the
/// Unicode replacement character before the breadcrumb is built.
#[test]
fn cloud_storage_breadcrumb_sanitizes_control_characters_in_prefix_segments() {
    let mut app = AppState::new();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ConfigurationsLoaded {
            configurations: vec![configuration(
                "dev",
                true,
                Some("dev@example.com"),
                Some("dev-project"),
                Some("europe-west1"),
                Some("europe-west1-b"),
            )],
        }),
    );
    let context = app.root.current_context().unwrap().clone();
    reduce(&mut app, &Action::Workspace(WorkspaceAction::Resources));

    reduce(
        &mut app,
        &Action::Resources(ResourcesAction::SelectService(
            ResourceServiceKind::CloudStorage,
        )),
    );
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::StorageBucketsLoaded {
            context: context.clone(),
            buckets: vec![storage_bucket("alpha-bucket")],
        }),
    );
    reduce(&mut app, &Action::Resources(ResourcesAction::OpenSelected));
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::StorageObjectPageLoaded {
            context,
            bucket: "alpha-bucket".to_string(),
            prefix: String::new(),
            append: false,
            page: StorageObjectPage {
                prefixes: vec![storage_prefix("ba\u{7}d/")],
                objects: Vec::new(),
                next_page_token: None,
            },
        }),
    );
    reduce(&mut app, &Action::Resources(ResourcesAction::OpenSelected));

    let projection = project_app_frame(&app.root);
    let last = projection
        .breadcrumbs
        .last()
        .expect("prefix segment breadcrumb entry");
    assert_eq!(last, "ba\u{FFFD}d");
    assert!(
        !last.contains('\u{7}'),
        "raw control character must not leak into the breadcrumb"
    );
}

#[test]
fn resource_frame_reports_marked_count() {
    let mut app = AppState::new();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ConfigurationsLoaded {
            configurations: vec![configuration(
                "dev",
                true,
                Some("dev@example.com"),
                Some("dev-project"),
                Some("europe-west1"),
                Some("europe-west1-b"),
            )],
        }),
    );
    let context = app
        .root
        .current_context()
        .expect("context should exist")
        .clone();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ComputeInstancesLoaded {
            context,
            instances: vec![ComputeInstance {
                id: "api-dev-1-id".to_string(),
                name: "api-dev-1".to_string(),
                zone: "europe-west1-b".to_string(),
                status: ResourceStatus::Running,
                machine_type: "e2-medium".to_string(),
                internal_ip: Some("10.0.0.4".to_string()),
                external_ip: None,
            }],
        }),
    );
    app.root.workspace = Workspace::Resources;
    reduce(
        &mut app,
        &Action::Resources(nuvem::app::ResourcesAction::ToggleMark),
    );

    let projection = project_app_frame(&app.root);
    assert!(
        projection.view_title.contains("Marked: 1"),
        "expected marked count in {:?}",
        projection.view_title
    );
}

#[test]
fn resource_frame_reports_hidden_marked_count_when_filtered() {
    let mut app = AppState::new();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ConfigurationsLoaded {
            configurations: vec![configuration(
                "dev",
                true,
                Some("dev@example.com"),
                Some("dev-project"),
                Some("europe-west1"),
                Some("europe-west1-b"),
            )],
        }),
    );
    let context = app
        .root
        .current_context()
        .expect("context should exist")
        .clone();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ComputeInstancesLoaded {
            context,
            instances: vec![
                ComputeInstance {
                    id: "api-dev-1-id".to_string(),
                    name: "api-dev-1".to_string(),
                    zone: "europe-west1-b".to_string(),
                    status: ResourceStatus::Running,
                    machine_type: "e2-medium".to_string(),
                    internal_ip: Some("10.0.0.4".to_string()),
                    external_ip: None,
                },
                ComputeInstance {
                    id: "worker-dev-1-id".to_string(),
                    name: "worker-dev-1".to_string(),
                    zone: "europe-west1-b".to_string(),
                    status: ResourceStatus::Running,
                    machine_type: "e2-medium".to_string(),
                    internal_ip: Some("10.0.0.5".to_string()),
                    external_ip: None,
                },
            ],
        }),
    );
    app.root.workspace = Workspace::Resources;
    reduce(
        &mut app,
        &Action::Resources(nuvem::app::ResourcesAction::ToggleMark),
    );
    reduce(
        &mut app,
        &Action::Resources(nuvem::app::ResourcesAction::SelectNext),
    );
    reduce(
        &mut app,
        &Action::Resources(nuvem::app::ResourcesAction::ToggleMark),
    );
    reduce(
        &mut app,
        &Action::Resources(nuvem::app::ResourcesAction::StartFilter),
    );
    for value in "api".chars() {
        reduce(
            &mut app,
            &Action::Resources(nuvem::app::ResourcesAction::AppendFilterChar(value)),
        );
    }
    reduce(
        &mut app,
        &Action::Resources(nuvem::app::ResourcesAction::ApplyFilter),
    );

    let projection = project_app_frame(&app.root);
    assert!(
        projection.view_title.contains("Marked: 2"),
        "expected total marked count in {:?}",
        projection.view_title
    );
    assert!(
        projection.view_title.contains("1 hidden"),
        "expected hidden marked count in {:?}",
        projection.view_title
    );
}

#[test]
fn home_footer_exposes_refresh_shortcut() {
    let root = RootState {
        workspace: Workspace::Home,
        ..Default::default()
    };

    let hints = project_app_frame(&root).footer_hints;

    assert!(
        hints
            .iter()
            .any(|hint| hint.key == "r" && hint.label == "Refresh")
    );
}

#[test]
fn frame_projection_starts_with_no_refresh_time() {
    let projection = project_app_frame(&RootState::default());

    assert_eq!(projection.operation.last_refresh_at, None);
    assert_eq!(projection.operation.phase.symbol(), "OK");
}

#[test]
fn frame_split_leaves_a_six_row_body_at_60x12() {
    let areas = split_frame(Rect::new(0, 0, 60, 12));
    assert_eq!(areas.header.height, 2);
    assert_eq!(areas.breadcrumb.height, 1);
    assert_eq!(areas.body.height, 6);
    assert_eq!(areas.status.height, 2);
    assert_eq!(areas.footer.height, 1);
}

#[test]
fn typed_detail_keeps_header_and_footer_in_a_seven_row_body() {
    let mut app = AppState::new();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ConfigurationsLoaded {
            configurations: vec![configuration(
                "dev",
                true,
                Some("dev@example.com"),
                Some("dev-project"),
                Some("europe-west1"),
                Some("europe-west1-b"),
            )],
        }),
    );
    let context = app
        .root
        .current_context()
        .expect("context should exist")
        .clone();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ComputeInstancesLoaded {
            context,
            instances: vec![ComputeInstance {
                id: "api-dev-1-id".to_string(),
                name: "api-dev-1".to_string(),
                zone: "europe-west1-b".to_string(),
                status: ResourceStatus::Running,
                machine_type: "e2-medium".to_string(),
                internal_ip: Some("10.0.0.4".to_string()),
                external_ip: None,
            }],
        }),
    );
    reduce(&mut app, &Action::Workspace(WorkspaceAction::Resources));
    reduce(
        &mut app,
        &Action::Resources(nuvem::app::ResourcesAction::OpenSelected),
    );

    let session = RuntimeSession::new_for_test(app);
    let backend = TestBackend::new(100, 13);
    let mut terminal = Terminal::new(backend).expect("terminal should initialize");
    terminal
        .draw(|frame| nuvem::ui::render(frame, &session))
        .expect("detail should render");

    let text = terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();
    assert!(text.contains("GCE instance / api-dev-1"));
    assert!(text.contains("[ and ] tabs   n/p field   y copy   Esc/q return to list"));
}

#[test]
fn hidden_resources_sidebar_keeps_active_service_in_rendered_frame() {
    let mut app = AppState::new();
    app.root.workspace = Workspace::Resources;
    reduce(&mut app, &Action::Navigator(NavigatorAction::ToggleSidebar));
    let session = RuntimeSession::new_for_test(app);
    let backend = TestBackend::new(100, 24);
    let mut terminal = Terminal::new(backend).expect("terminal should initialize");
    terminal
        .draw(|frame| nuvem::ui::render(frame, &session))
        .expect("render should succeed");

    let text = terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();
    assert!(!text.contains("Service navigator"));
    assert!(text.contains("GCE instances"));
}

#[test]
fn semantic_styles_distinguish_focus_and_failure() {
    assert_ne!(theme::selected_row(), theme::inactive_selected_row());
    assert_eq!(theme::status_error().fg, Some(Color::Red));
}

#[test]
fn fit_text_preserves_fitting_values_and_truncates_with_ellipsis() {
    assert_eq!(fit_text("nuvem", 5), "nuvem");
    assert_eq!(fit_text("nuvem", 0), "");
    assert_eq!(fit_text("glassfish", 6), "gla...");
}

#[test]
fn fit_text_uses_terminal_width_for_wide_characters() {
    assert_eq!(fit_text("界abcd", 5), "界...");
}

#[test]
fn fit_text_keeps_combining_sequences_intact() {
    assert_eq!(fit_text("e\u{301}abcd", 4), "e\u{301}...");
}

#[test]
fn status_line_uses_bracketed_operation_markers_without_changing_semantic_text() {
    for (phase, marker) in [
        (OperationPhase::Healthy, "[OK]"),
        (OperationPhase::Loading, "[LOAD]"),
        (OperationPhase::Degraded, "[STALE]"),
        (OperationPhase::Failed, "[ERR]"),
    ] {
        let line = status_line(phase, Some("Refresh"), Some("timed out"));
        let text = line
            .spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect::<String>();

        assert_eq!(text, format!("{marker} Refresh: timed out"));
    }
}

#[test]
fn status_phase_symbols_render_for_loading_degraded_and_failed_operations() {
    for (phase, symbol) in [
        (OperationPhase::Loading, "[LOAD]"),
        (OperationPhase::Degraded, "[STALE]"),
        (OperationPhase::Failed, "[ERR]"),
    ] {
        let projection = AppFrameProjection {
            app_title: "nuvem",
            view_title: "Resources".to_string(),
            breadcrumbs: Vec::new(),
            context: FrameContextProjection {
                configuration: None,
                account: None,
                project: None,
                region: None,
                zone: None,
            },
            service_label: Some("GCE instances"),
            operation: FrameOperationProjection {
                phase,
                label: "GCE instances".to_string(),
                message: Some("status detail".to_string()),
                message_label: Some("GCE instances".to_string()),
                last_refresh_at: None,
            },
            footer_hints: Vec::new(),
            footer_pinned_hints: Vec::new(),
        };
        let backend = TestBackend::new(60, 2);
        let mut terminal = Terminal::new(backend).expect("terminal should initialize");
        terminal
            .draw(|frame| render_status(frame, Rect::new(0, 0, 60, 2), &projection))
            .expect("status should render");

        let line = terminal
            .backend()
            .buffer()
            .content()
            .chunks(60)
            .next()
            .expect("status line should render")
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(
            line.contains(symbol),
            "expected status symbol {symbol:?} in {line:?}"
        );
        assert!(
            line.contains("GCE instances: status detail"),
            "expected operation label in {line:?}"
        );
    }
}

#[test]
fn status_renders_unassociated_messages_without_the_previous_operation_label() {
    let projection = AppFrameProjection {
        app_title: "nuvem",
        view_title: "Configurations".to_string(),
        breadcrumbs: Vec::new(),
        context: FrameContextProjection {
            configuration: None,
            account: None,
            project: None,
            region: None,
            zone: None,
        },
        service_label: None,
        operation: FrameOperationProjection {
            phase: OperationPhase::Healthy,
            label: "GCE instances".to_string(),
            message: Some("Configuration name is required".to_string()),
            message_label: None,
            last_refresh_at: None,
        },
        footer_hints: Vec::new(),
        footer_pinned_hints: Vec::new(),
    };
    let backend = TestBackend::new(80, 2);
    let mut terminal = Terminal::new(backend).expect("terminal should initialize");

    terminal
        .draw(|frame| render_status(frame, Rect::new(0, 0, 80, 2), &projection))
        .expect("status should render");

    let line = terminal
        .backend()
        .buffer()
        .content()
        .chunks(80)
        .next()
        .expect("status line should render")
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();

    assert!(line.contains("[OK] Configuration name is required"));
    assert!(!line.contains("GCE instances: Configuration name is required"));
}

#[test]
fn unassociated_status_messages_use_message_specific_phases() {
    let mut success = AppState::default();
    success
        .root
        .system
        .begin_operation(OperationPhase::Failed, "GCE instances");
    reduce(
        &mut success,
        &Action::EffectResult(EffectResult::ClipboardCopied {
            description: "instance name".to_string(),
        }),
    );
    assert_eq!(
        project_app_frame(&success.root).operation.phase,
        OperationPhase::Healthy
    );

    let mut failure = AppState::default();
    failure
        .root
        .system
        .begin_operation(OperationPhase::Healthy, "Ready");
    reduce(
        &mut failure,
        &Action::EffectResult(EffectResult::ClipboardCopyFailed {
            description: "instance name".to_string(),
            error: AppError::gcloud("clipboard unavailable"),
        }),
    );
    assert_eq!(
        project_app_frame(&failure.root).operation.phase,
        OperationPhase::Failed
    );
}

#[test]
fn frame_projection_uses_transient_mode_footer_hints() {
    let mut filter_root = RootState {
        workspace: Workspace::Resources,
        ..Default::default()
    };
    filter_root.resources.start_filter();
    let filter_hints = project_app_frame(&filter_root).footer_hints;
    assert!(filter_hints.iter().any(|hint| hint.label == "Apply"));
    assert!(filter_hints.iter().any(|hint| hint.label == "Cancel"));

    let mut palette_root = RootState {
        workspace: Workspace::Resources,
        ..Default::default()
    };
    palette_root.command.open();
    let palette_hints = project_app_frame(&palette_root).footer_hints;
    assert!(palette_hints.iter().any(|hint| hint.label == "Run"));
    assert!(palette_hints.iter().any(|hint| hint.label == "Cancel"));

    let mut confirmation_root = RootState {
        workspace: Workspace::Configurations,
        ..Default::default()
    };
    confirmation_root
        .configurations
        .set_rows(vec![configuration(
            "prod",
            false,
            Some("prod@example.com"),
            Some("prod-project"),
            None,
            None,
        )]);
    assert_eq!(
        confirmation_root
            .configurations
            .start_activation_confirmation(),
        nuvem::app::state::configurations::ActivationConfirmationRequest::Started
    );
    let confirmation_hints = project_app_frame(&confirmation_root).footer_hints;
    assert!(
        confirmation_hints
            .iter()
            .any(|hint| hint.label == "Confirm")
    );
    assert!(confirmation_hints.iter().any(|hint| hint.label == "Cancel"));

    let mut resource_confirmation_root = RootState {
        workspace: Workspace::Resources,
        ..Default::default()
    };
    resource_confirmation_root
        .resources
        .start_action_confirmation(
            configuration("dev", true, None, Some("dev-project"), None, None).context(),
            ComputeInstance {
                id: "api-dev-1-id".to_string(),
                name: "api-dev-1".to_string(),
                zone: "europe-west1-b".to_string(),
                status: ResourceStatus::Running,
                machine_type: "e2-medium".to_string(),
                internal_ip: None,
                external_ip: None,
            },
            ComputeInstanceAction::Stop,
        );
    let resource_confirmation_hints = project_app_frame(&resource_confirmation_root).footer_hints;
    assert_eq!(
        resource_confirmation_hints
            .iter()
            .map(|hint| (hint.key.as_str(), hint.label))
            .collect::<Vec<_>>(),
        vec![("Enter", "Confirm"), ("Esc", "Cancel")]
    );
}

#[test]
fn status_renders_never_before_the_first_completed_refresh() {
    let projection = AppFrameProjection {
        app_title: "nuvem",
        view_title: "Resources".to_string(),
        breadcrumbs: Vec::new(),
        context: FrameContextProjection {
            configuration: None,
            account: None,
            project: None,
            region: None,
            zone: None,
        },
        service_label: Some("GCE instances"),
        operation: FrameOperationProjection {
            phase: OperationPhase::Idle,
            label: "Ready".to_string(),
            message: None,
            message_label: None,
            last_refresh_at: None,
        },
        footer_hints: Vec::new(),
        footer_pinned_hints: Vec::new(),
    };
    let backend = TestBackend::new(60, 2);
    let mut terminal = Terminal::new(backend).expect("terminal should initialize");

    terminal
        .draw(|frame| render_status(frame, Rect::new(0, 0, 60, 2), &projection))
        .expect("status should render");

    let line = terminal
        .backend()
        .buffer()
        .content()
        .chunks(60)
        .nth(1)
        .expect("last refresh line should render")
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();
    assert!(line.contains("Updated: Never"));
}

#[test]
fn footer_hints_are_fitted_to_the_available_width() {
    let projection = AppFrameProjection {
        app_title: "nuvem",
        view_title: "Resources".to_string(),
        breadcrumbs: Vec::new(),
        context: FrameContextProjection {
            configuration: None,
            account: None,
            project: None,
            region: None,
            zone: None,
        },
        service_label: None,
        operation: FrameOperationProjection {
            phase: OperationPhase::Healthy,
            label: "Ready".to_string(),
            message: None,
            message_label: None,
            last_refresh_at: None,
        },
        footer_hints: vec![FrameHint {
            key: "Ctrl-Space".to_string(),
            label: "Pick focused context value",
        }],
        footer_pinned_hints: Vec::new(),
    };
    let backend = TestBackend::new(20, 1);
    let mut terminal = Terminal::new(backend).expect("terminal should initialize");

    terminal
        .draw(|frame| render_footer(frame, Rect::new(0, 0, 20, 1), &projection))
        .expect("footer should render");

    let line = terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();
    assert!(line.contains("Ctrl-Space"));
    assert!(line.contains("..."));
}

#[test]
fn shared_frame_helpers_render_only_their_supplied_rectangles() {
    let projection = AppFrameProjection {
        app_title: "nuvem",
        view_title: "Resources".to_string(),
        breadcrumbs: vec!["Home".to_string(), "Resources".to_string()],
        context: FrameContextProjection {
            configuration: None,
            account: None,
            project: None,
            region: None,
            zone: None,
        },
        service_label: Some("GCE instances"),
        operation: FrameOperationProjection {
            phase: OperationPhase::Healthy,
            label: "Ready".to_string(),
            message: None,
            message_label: None,
            last_refresh_at: None,
        },
        footer_hints: vec![FrameHint {
            key: "r".to_string(),
            label: "Refresh",
        }],
        footer_pinned_hints: Vec::new(),
    };
    let backend = TestBackend::with_lines(["...................."; 8]);
    let mut terminal = Terminal::new(backend).expect("terminal should initialize");

    terminal
        .draw(|frame| {
            render_header(frame, Rect::new(2, 1, 12, 1), &projection);
            render_breadcrumb(frame, Rect::new(2, 2, 12, 1), &projection);
            render_status(frame, Rect::new(2, 3, 12, 1), &projection);
            render_footer(frame, Rect::new(2, 4, 12, 1), &projection);
        })
        .expect("shared frame should render");

    let buffer = terminal.backend().buffer();
    assert_eq!(buffer[(0, 0)].symbol(), ".");
    assert_eq!(buffer[(1, 1)].symbol(), ".");
    assert_eq!(buffer[(2, 1)].symbol(), "n");
    assert_eq!(buffer[(2, 2)].symbol(), "H");
    assert_eq!(buffer[(2, 3)].symbol(), "[");
    assert_eq!(buffer[(2, 4)].symbol(), "r");
    assert_eq!(buffer[(14, 4)].symbol(), ".");
}

#[test]
fn header_keeps_configuration_and_project_visible_with_a_long_configuration_at_60x12() {
    let projection = AppFrameProjection {
        app_title: "nuvem",
        view_title: "Resources".to_string(),
        breadcrumbs: Vec::new(),
        context: FrameContextProjection {
            configuration: Some("very-long-configuration-name-that-needs-truncation".to_string()),
            account: Some("dev@example.com".to_string()),
            project: Some("nuvem-project".to_string()),
            region: Some("europe-west1".to_string()),
            zone: Some("europe-west1-b".to_string()),
        },
        service_label: Some("GCE instances"),
        operation: FrameOperationProjection {
            phase: OperationPhase::Healthy,
            label: "Ready".to_string(),
            message: None,
            message_label: None,
            last_refresh_at: None,
        },
        footer_hints: Vec::new(),
        footer_pinned_hints: Vec::new(),
    };
    let backend = TestBackend::new(60, 12);
    let mut terminal = Terminal::new(backend).expect("terminal should initialize");

    terminal
        .draw(|frame| render_header(frame, Rect::new(0, 0, 60, 2), &projection))
        .expect("header should render");

    let buffer = terminal.backend().buffer();
    let context_line = (0..60)
        .map(|column| buffer[(column, 1)].symbol())
        .collect::<String>();
    assert!(context_line.contains("Configuration: very-long-"));
    assert!(context_line.contains("Project: nuvem-project"));
}

#[test]
fn header_fits_optional_context_values_with_all_separators_at_60x12() {
    let projection = AppFrameProjection {
        app_title: "nuvem",
        view_title: "Resources".to_string(),
        breadcrumbs: Vec::new(),
        context: FrameContextProjection {
            configuration: Some("dev".to_string()),
            account: Some("dev@example.com".to_string()),
            project: Some("dev-project".to_string()),
            region: None,
            zone: None,
        },
        service_label: None,
        operation: FrameOperationProjection {
            phase: OperationPhase::Healthy,
            label: "Ready".to_string(),
            message: None,
            message_label: None,
            last_refresh_at: None,
        },
        footer_hints: Vec::new(),
        footer_pinned_hints: Vec::new(),
    };
    let backend = TestBackend::new(60, 12);
    let mut terminal = Terminal::new(backend).expect("terminal should initialize");

    terminal
        .draw(|frame| render_header(frame, Rect::new(0, 0, 60, 2), &projection))
        .expect("header should render");

    let context_line = terminal
        .backend()
        .buffer()
        .content()
        .chunks(60)
        .nth(1)
        .expect("context line should render")
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();
    let expected_account = fit_text("Account: dev@example.com", 18);
    assert_eq!(
        context_line,
        format!("Configuration: dev  Project: dev-project  {expected_account}")
    );
}

#[test]
fn header_fits_title_metadata_to_the_available_width() {
    let projection = AppFrameProjection {
        app_title: "nuvem",
        view_title: "Very long view metadata".to_string(),
        breadcrumbs: Vec::new(),
        context: FrameContextProjection {
            configuration: None,
            account: None,
            project: None,
            region: None,
            zone: None,
        },
        service_label: None,
        operation: FrameOperationProjection {
            phase: OperationPhase::Healthy,
            label: "Ready".to_string(),
            message: None,
            message_label: None,
            last_refresh_at: None,
        },
        footer_hints: Vec::new(),
        footer_pinned_hints: Vec::new(),
    };
    let backend = TestBackend::new(20, 2);
    let mut terminal = Terminal::new(backend).expect("terminal should initialize");

    terminal
        .draw(|frame| render_header(frame, Rect::new(0, 0, 20, 2), &projection))
        .expect("header should render");

    let title_line = terminal
        .backend()
        .buffer()
        .content()
        .chunks(20)
        .next()
        .expect("title line should render")
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();
    assert_eq!(
        title_line,
        format!("nuvem  {}", fit_text("Very long view metadata", 13))
    );
}

#[test]
fn format_last_refresh_renders_relative_age() {
    let base = Local.with_ymd_and_hms(2026, 7, 16, 14, 30, 0).unwrap();
    assert_eq!(format_last_refresh(base, None), "Never");
    assert_eq!(
        format_last_refresh(base + chrono::Duration::seconds(2), Some(base)),
        "just now"
    );
    assert_eq!(
        format_last_refresh(base + chrono::Duration::seconds(12), Some(base)),
        "12s ago"
    );
    assert_eq!(
        format_last_refresh(base + chrono::Duration::seconds(90), Some(base)),
        "1m ago"
    );
    assert_eq!(
        format_last_refresh(base + chrono::Duration::hours(3), Some(base)),
        "3h ago"
    );
}

/// Cloud SQL rows are markable, so the resource frame reports their
/// marked count exactly as it does for Compute.
#[test]
fn resource_frame_reports_marked_count_for_cloud_sql() {
    use nuvem_domain::gcp::resource::{
        CloudSqlAvailability, CloudSqlInstance, CloudSqlStatus, EndpointVisibility,
    };

    let mut app = AppState::new();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ConfigurationsLoaded {
            configurations: vec![configuration(
                "dev",
                true,
                Some("dev@example.com"),
                Some("dev-project"),
                Some("europe-west1"),
                Some("europe-west1-b"),
            )],
        }),
    );
    reduce(
        &mut app,
        &Action::Resources(ResourcesAction::SelectService(
            ResourceServiceKind::CloudSql,
        )),
    );
    let context = app
        .root
        .current_context()
        .expect("context should exist")
        .clone();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::CloudSqlInstancesLoaded {
            context,
            instances: vec![CloudSqlInstance {
                name: "orders-dev".to_string(),
                database_version: "POSTGRES_15".to_string(),
                engine: "PostgreSQL".to_string(),
                region: "europe-west1".to_string(),
                tier: "db-custom-2-7680".to_string(),
                availability: CloudSqlAvailability::Regional,
                status: CloudSqlStatus::Runnable,
                connectivity: EndpointVisibility::Private,
            }],
        }),
    );
    app.root.workspace = Workspace::Resources;
    reduce(&mut app, &Action::Resources(ResourcesAction::ToggleMark));

    let projection = project_app_frame(&app.root);
    assert!(
        projection.view_title.contains("Marked: 1"),
        "expected marked count in {:?}",
        projection.view_title
    );
}
