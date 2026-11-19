use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use nuvem::app::projection::{
    FrameHint, NavigatorFocus, project_app_frame, project_resources, project_service_navigator,
};
use nuvem::app::reducer::root::reduce;
use nuvem::app::state::configurations::{
    ConfigurationContextPickerItem, ConfigurationContextPickerKind,
};
use nuvem::app::{
    Action, AppError, AppState, CommandAction, ConfigurationsAction, EffectResult, SystemAction,
    WorkspaceAction,
};
use nuvem::input::Mode;
use nuvem::input::keymap::action_for_key;
use nuvem::services::runtime::session::RuntimeSession;
use nuvem_domain::gcp::configuration::{GcloudConfiguration, GcloudConfigurationName};
use nuvem_domain::gcp::location::{ComputeRegion, ComputeZone};
use nuvem_domain::gcp::project::GcloudProject;
use nuvem_domain::gcp::resource::{
    CloudRunCondition, CloudRunRevision, CloudRunRevisionDetail, CloudRunService,
    CloudRunServiceDetail, CloudRunServiceStatus, CloudRunTrafficTarget, CloudSqlAvailability,
    CloudSqlInstance, CloudSqlStatus, ComputeInstance, ComputeInstanceDetail, EndpointVisibility,
    GkeAddon, GkeCluster, GkeClusterDetail, GkeClusterStatus, GkeNodePool, ResourceServiceKind,
    ResourceStatus, StorageBucket, StorageObject, StorageObjectPage,
};
use ratatui::{Terminal, backend::TestBackend, layout::Rect, style::Color};

/// Operations-log id for results whose test asserts nothing about the
/// operations log. It sits above every id `begin` would hand out in these
/// tests, so such a result can never settle a real entry by accident: `0` would
/// be safe only for as long as no test in this file seeds one, and that is a
/// property of today's file rather than of the constant.
const UNTRACKED_OPERATION_ID: u64 = 9_000;

/// Prefix rendered before a selected row's NAME cell in the compute instance
/// table: the "> " cursor highlight symbol, followed by the (blank, unmarked)
/// leading mark column and its column-spacing gap. See
/// `src/ui/shared/table.rs::render_table`.
const SELECTED_UNMARKED_ROW_PREFIX: &str = ">     ";

/// Prefix rendered before a selected row's NAME cell in tables that opt out of
/// the mark column (GKE, Cloud SQL, logging): just the "> " cursor highlight
/// symbol, with no leading mark column reserved.
const SELECTED_ROW_PREFIX_NO_MARK: &str = "> ";

/// Prefix rendered before a selected, non-favorited row's text in the
/// configuration context picker overlay (project/region/zone picker): the
/// "> " cursor highlight, followed by the (blank, unmarked) favorite marker
/// column and its column-spacing gap. See
/// `src/ui/configurations/mod.rs::picker_lines`.
const CONTEXT_PICKER_SELECTED_UNMARKED_ROW_PREFIX: &str = ">   ";

/// Prefix rendered before a non-selected, non-favorited row's text in the
/// configuration context picker overlay (project/region/zone picker): no
/// cursor highlight, followed by the (blank, unmarked) favorite marker
/// column and its column-spacing gap — four blank columns in total. See
/// `src/ui/configurations/mod.rs::picker_lines`.
const CONTEXT_PICKER_UNSELECTED_UNMARKED_ROW_PREFIX: &str = "    ";

#[test]
fn ui_exports_render_entrypoint() {
    let _render: fn(&mut ratatui::Frame<'_>, &RuntimeSession) = nuvem::ui::render;
}

#[test]
fn resource_projection_api_is_public_to_ui_tests() {
    let app = AppState::new();
    let projection = project_resources(&app.root.resources);
    assert_eq!(projection.service_label, "GCE instances");
}

#[test]
fn home_navigator_projection_effectively_focuses_the_navigator() {
    let app = AppState::new();

    assert_eq!(app.root.navigator.focus(), NavigatorFocus::Resources);
    let projection = project_service_navigator(&app.root, Rect::new(0, 0, 32, 8));

    assert_eq!(projection.focus, NavigatorFocus::Navigator);
}

#[tokio::test]
async fn session_exposes_context_for_header_rendering() {
    let mut session = RuntimeSession::new_for_test(AppState::new());
    session
        .apply_effect_result(EffectResult::ConfigurationsLoaded {
            configurations: vec![GcloudConfiguration {
                name: GcloudConfigurationName::new("dev"),
                is_active: true,
                account: Some("dev@example.com".to_string()),
                project: Some("dev-project".to_string()),
                region: Some("europe-west1".to_string()),
                zone: None,
            }],
        })
        .await;

    let context = session
        .app()
        .root
        .current_context()
        .expect("context exists");
    assert_eq!(context.configuration.as_str(), "dev");
    assert_eq!(context.project.as_deref(), Some("dev-project"));
}

#[tokio::test]
async fn home_view_lists_resources_shortcut() {
    let mut session = RuntimeSession::new_for_test(AppState::new());
    load_active_dev(&mut session).await;

    let backend = TestBackend::new(80, 24);
    let mut terminal = Terminal::new(backend).expect("terminal should initialize");
    terminal
        .draw(|frame| nuvem::ui::render(frame, &session))
        .expect("render should succeed");

    let lines = terminal_lines(&terminal);

    assert_shared_frame(&lines, "Home", "g Resources");
    assert!(lines.iter().any(|line| line.contains("g Resources")));
    assert!(!lines.iter().any(|line| line.contains("g GCE instances")));
}

#[tokio::test]
async fn home_view_keeps_required_context_visible_at_minimum_frame_size() {
    let mut session = RuntimeSession::new_for_test(AppState::new());
    load_active_dev(&mut session).await;

    let lines = render_lines(&session, 60, 12);

    assert!(lines.iter().any(|line| line.contains("Configuration: dev")));
    assert!(
        lines
            .iter()
            .any(|line| line.contains("Project: dev-project"))
    );
}

#[tokio::test]
async fn home_view_exposes_quit_in_the_footer_at_minimum_frame_size() {
    let mut session = RuntimeSession::new_for_test(AppState::new());
    load_active_dev(&mut session).await;

    let lines = render_lines(&session, 60, 12);

    assert!(lines.last().is_some_and(|line| line.contains("q Quit")));
}

#[tokio::test]
async fn configurations_view_scrolls_selected_row_at_minimum_frame_size() {
    let mut session = RuntimeSession::new_for_test(AppState::new());
    session
        .apply_effect_result(EffectResult::ConfigurationsLoaded {
            configurations: (0..4)
                .map(|index| GcloudConfiguration {
                    name: GcloudConfigurationName::new(format!("config-{index}")),
                    is_active: index == 0,
                    account: Some(format!("user-{index}@example.com")),
                    project: Some(format!("project-{index}")),
                    region: None,
                    zone: None,
                })
                .collect(),
        })
        .await;
    session
        .dispatch_action(Action::Workspace(WorkspaceAction::Configurations))
        .await;
    for _ in 0..3 {
        session
            .dispatch_action(Action::Configurations(ConfigurationsAction::SelectNext))
            .await;
    }

    let lines = render_lines(&session, 60, 12);

    assert!(lines.iter().any(|line| line.contains(">   config-3")));
    assert!(!lines.iter().any(|line| line.contains("> * config-0")));
}

#[tokio::test]
async fn configurations_view_marks_selected_row() {
    let mut session = RuntimeSession::new_for_test(AppState::new());
    session
        .apply_effect_result(EffectResult::ConfigurationsLoaded {
            configurations: vec![
                GcloudConfiguration {
                    name: GcloudConfigurationName::new("dev"),
                    is_active: true,
                    account: Some("dev@example.com".to_string()),
                    project: Some("dev-project".to_string()),
                    region: None,
                    zone: None,
                },
                GcloudConfiguration {
                    name: GcloudConfigurationName::new("prod"),
                    is_active: false,
                    account: Some("prod@example.com".to_string()),
                    project: Some("prod-project".to_string()),
                    region: None,
                    zone: None,
                },
            ],
        })
        .await;
    session
        .dispatch_action(Action::Workspace(WorkspaceAction::Configurations))
        .await;
    session
        .dispatch_action(Action::Configurations(ConfigurationsAction::SelectNext))
        .await;

    let backend = TestBackend::new(80, 24);
    let mut terminal = Terminal::new(backend).expect("terminal should initialize");
    terminal
        .draw(|frame| nuvem::ui::render(frame, &session))
        .expect("render should succeed");

    let lines = terminal_lines(&terminal);

    assert_shared_frame(&lines, "Configurations", "r Refresh");
    assert!(lines.iter().any(|line| line.contains(">   prod")));
    assert!(!lines.iter().any(|line| line.contains("> * dev")));
}

#[tokio::test]
async fn configurations_view_renders_region_zone_and_missing_context_columns() {
    let mut session = RuntimeSession::new_for_test(AppState::new());
    session
        .apply_effect_result(EffectResult::ConfigurationsLoaded {
            configurations: vec![
                GcloudConfiguration {
                    name: GcloudConfigurationName::new("dev"),
                    is_active: true,
                    account: Some("dev@example.com".to_string()),
                    project: Some("dev-project".to_string()),
                    region: Some("europe-west1".to_string()),
                    zone: Some("europe-west1-b".to_string()),
                },
                GcloudConfiguration {
                    name: GcloudConfigurationName::new("ops"),
                    is_active: false,
                    account: None,
                    project: None,
                    region: None,
                    zone: None,
                },
            ],
        })
        .await;
    session
        .dispatch_action(Action::Workspace(WorkspaceAction::Configurations))
        .await;

    let backend = TestBackend::new(120, 12);
    let mut terminal = Terminal::new(backend).expect("terminal should initialize");
    terminal
        .draw(|frame| nuvem::ui::render(frame, &session))
        .expect("render should succeed");

    let lines = terminal_lines(&terminal);

    // The cursor is in ratatui's highlight gutter and the active marker is the
    // NAME cell's own prefix, so the active row reads `> * dev` across the two.
    assert!(
        lines.iter().any(|line| {
            line.contains("> * dev")
                && line.contains("dev@example.com")
                && line.contains("dev-project")
                && line.contains("europe-west1")
                && line.contains("europe-west1-b")
        }),
        "{lines:#?}"
    );

    // `ops` has no account, project, region or zone. Each absent value is now
    // its own padded cell carrying the missing marker rather than four dashes
    // run together by a space join, so the assertion counts the markers instead
    // of matching the run they used to form.
    let ops = lines
        .iter()
        .find(|line| line.contains("  ops"))
        .unwrap_or_else(|| panic!("{lines:#?}"));
    assert_eq!(
        ops.split_whitespace().filter(|cell| *cell == "-").count(),
        4,
        "every absent context value keeps its own cell: {ops:?}"
    );
}

#[tokio::test]
async fn configurations_view_fits_long_rows_to_the_body_width() {
    let mut session = RuntimeSession::new_for_test(AppState::new());
    session
        .apply_effect_result(EffectResult::ConfigurationsLoaded {
            configurations: vec![GcloudConfiguration {
                name: GcloudConfigurationName::new(
                    "very-long-configuration-name-that-needs-truncation",
                ),
                is_active: true,
                account: Some("dev@example.com".to_string()),
                project: Some("dev-project".to_string()),
                region: Some("europe-west1".to_string()),
                zone: Some("europe-west1-b".to_string()),
            }],
        })
        .await;
    session
        .dispatch_action(Action::Workspace(WorkspaceAction::Configurations))
        .await;

    let lines = render_lines(&session, 60, 12);

    // The row used to be one formatted string fitted whole, so a long name ran
    // to the body's edge and pushed every other value off it. It is a table
    // cell now: the name is truncated to the NAME column with the
    // table's own `…`, and the columns beside it keep their places.
    // Scoped to the table's own block: the app frame's context header names the
    // active configuration too, and at this width it is truncated the same way,
    // so an unscoped search finds that line first.
    let table: Vec<&String> = lines.iter().filter(|line| line.starts_with('│')).collect();
    let header = table
        .iter()
        .find(|line| line.contains("ACCOUNT"))
        .unwrap_or_else(|| panic!("{lines:#?}"));
    let row = table
        .iter()
        .find(|line| line.contains("very-long-config"))
        .unwrap_or_else(|| panic!("{lines:#?}"));

    assert!(
        row.contains('…'),
        "the name is wider than its column and must be truncated: {row:?}"
    );
    assert!(
        !row.contains("very-long-configuration-name-that-needs-truncation"),
        "the name must not be painted whole at this width: {row:?}"
    );

    let column_of = |line: &str, needle: &str| -> usize {
        let byte = line
            .find(needle)
            .unwrap_or_else(|| panic!("{needle:?} is not on {line:?}"));
        line[..byte].chars().count()
    };
    // A prefix, not the whole address: 60 columns truncates the account cell
    // too, and what is being asserted is where it *starts*.
    assert_eq!(
        column_of(row, "dev@exa"),
        column_of(header, "ACCOUNT"),
        "a long name must not push the account out of its column; \
         row: {row:?}, header: {header:?}"
    );
}

#[tokio::test]
async fn configurations_view_renders_filter_line_and_filtered_empty_state() {
    let mut session = RuntimeSession::new_for_test(AppState::new());
    session
        .apply_effect_result(EffectResult::ConfigurationsLoaded {
            configurations: vec![GcloudConfiguration {
                name: GcloudConfigurationName::new("dev"),
                is_active: true,
                account: Some("dev@example.com".to_string()),
                project: Some("dev-project".to_string()),
                region: Some("europe-west1".to_string()),
                zone: Some("europe-west1-b".to_string()),
            }],
        })
        .await;
    session
        .dispatch_action(Action::Workspace(WorkspaceAction::Configurations))
        .await;
    session
        .dispatch_action(Action::Configurations(ConfigurationsAction::StartFiltering))
        .await;
    session
        .dispatch_action(Action::Configurations(ConfigurationsAction::UpdateFilter(
            "missing".to_string(),
        )))
        .await;

    let backend = TestBackend::new(120, 12);
    let mut terminal = Terminal::new(backend).expect("terminal should initialize");
    terminal
        .draw(|frame| nuvem::ui::render(frame, &session))
        .expect("render should succeed");

    let lines = terminal_lines(&terminal);
    assert!(lines.iter().any(|line| line.contains("Filter: missing_")));

    session
        .dispatch_action(Action::Configurations(ConfigurationsAction::ApplyFilter))
        .await;
    terminal
        .draw(|frame| nuvem::ui::render(frame, &session))
        .expect("render should succeed");

    let lines = terminal_lines(&terminal);
    assert!(
        lines
            .iter()
            .any(|line| line.contains("No configurations match filter \"missing\""))
    );
    assert!(
        lines
            .iter()
            .any(|line| line.contains("Filter: missing") && line.contains("Rows: 0/1"))
    );
}

#[tokio::test]
async fn configurations_create_form_renders_fields_and_focused_row() {
    let mut session = RuntimeSession::new_for_test(AppState::new());
    load_active_dev(&mut session).await;
    session
        .dispatch_action(Action::Workspace(WorkspaceAction::Configurations))
        .await;
    session
        .dispatch_action(Action::Configurations(ConfigurationsAction::StartCreate))
        .await;
    for ch in "dev".chars() {
        session
            .dispatch_action(Action::Configurations(
                ConfigurationsAction::AppendCreateChar(ch),
            ))
            .await;
    }
    session
        .dispatch_action(Action::Configurations(
            ConfigurationsAction::FocusNextCreateField,
        ))
        .await;
    for ch in "dev@example.com".chars() {
        session
            .dispatch_action(Action::Configurations(
                ConfigurationsAction::AppendCreateChar(ch),
            ))
            .await;
    }

    let backend = TestBackend::new(100, 24);
    let mut terminal = Terminal::new(backend).expect("terminal should initialize");
    terminal
        .draw(|frame| nuvem::ui::render(frame, &session))
        .expect("render should succeed");

    let lines = terminal_lines(&terminal);

    assert_shared_frame(&lines, "Create configuration", "Enter Submit");
    assert!(
        lines
            .iter()
            .any(|line| line.contains("Create gcloud configuration"))
    );
    let form_index = line_index(&lines, "Create gcloud configuration");
    let name_index = line_index_after(&lines, form_index, "  Name: dev");
    let account_index = line_index_after(&lines, form_index, "> Account: dev@example.com");
    let project_index = line_index_after(&lines, form_index, "  Project:");
    let region_index = line_index_after(&lines, form_index, "  Region:");
    let zone_index = line_index_after(&lines, form_index, "  Zone:");
    assert!(
        name_index < account_index
            && account_index < project_index
            && project_index < region_index
            && region_index < zone_index
    );
    assert!(
        lines
            .iter()
            .any(|line| line.contains("Enter create") && line.contains("Esc cancel"))
    );
}

#[tokio::test]
async fn configurations_create_form_scrolls_the_focused_field_at_minimum_frame_size() {
    let mut session = RuntimeSession::new_for_test(AppState::new());
    load_active_dev(&mut session).await;
    session
        .dispatch_action(Action::Workspace(WorkspaceAction::Configurations))
        .await;
    session
        .dispatch_action(Action::Configurations(ConfigurationsAction::StartCreate))
        .await;

    let lines = render_lines(&session, 60, 12);
    assert!(lines.iter().any(|line| line.contains("> Name:")));
    assert!(lines.iter().any(|line| line.contains("Enter Submit")));
    assert!(lines.iter().any(|line| line.contains("Esc Cancel")));

    for _ in 0..4 {
        session
            .dispatch_action(Action::Configurations(
                ConfigurationsAction::FocusNextCreateField,
            ))
            .await;
    }

    let lines = render_lines(&session, 60, 12);
    assert!(lines.iter().any(|line| line.contains("> Zone:")));
}

#[tokio::test]
async fn configurations_create_form_renders_status_message_in_shared_region() {
    let mut session = RuntimeSession::new_for_test(AppState::new());
    session
        .dispatch_action(Action::Workspace(WorkspaceAction::Configurations))
        .await;
    session
        .dispatch_action(Action::Configurations(ConfigurationsAction::StartCreate))
        .await;
    session
        .dispatch_action(Action::Configurations(ConfigurationsAction::SubmitCreate))
        .await;

    let backend = TestBackend::new(100, 14);
    let mut terminal = Terminal::new(backend).expect("terminal should initialize");
    terminal
        .draw(|frame| nuvem::ui::render(frame, &session))
        .expect("render should succeed");

    let lines = terminal_lines(&terminal);

    assert!(
        lines
            .iter()
            .any(|line| line.contains("Configuration name is required"))
    );
    assert!(lines.iter().any(|line| line.contains("> Name:")));
    assert!(
        !lines
            .iter()
            .any(|line| line.contains("Status: Configuration name is required"))
    );
}

#[tokio::test]
async fn configurations_context_edit_form_renders_target_fields_and_focused_row() {
    let mut session = RuntimeSession::new_for_test(AppState::new());
    session
        .apply_effect_result(EffectResult::ConfigurationsLoaded {
            configurations: vec![GcloudConfiguration {
                name: GcloudConfigurationName::new("dev"),
                is_active: true,
                account: Some("dev@example.com".to_string()),
                project: Some("dev-project".to_string()),
                region: Some("europe-west1".to_string()),
                zone: Some("europe-west1-b".to_string()),
            }],
        })
        .await;
    session
        .dispatch_action(Action::Workspace(WorkspaceAction::Configurations))
        .await;
    session
        .dispatch_action(Action::Configurations(
            ConfigurationsAction::StartEditActiveContext,
        ))
        .await;
    session
        .dispatch_action(Action::Configurations(
            ConfigurationsAction::FocusNextContextEditField,
        ))
        .await;

    let backend = TestBackend::new(100, 24);
    let mut terminal = Terminal::new(backend).expect("terminal should initialize");
    terminal
        .draw(|frame| nuvem::ui::render(frame, &session))
        .expect("render should succeed");

    let lines = terminal_lines(&terminal);

    assert_shared_frame(&lines, "Edit configuration context", "Enter Submit");
    assert!(
        lines
            .iter()
            .any(|line| line.contains("Edit active gcloud context"))
    );
    assert!(lines.iter().any(|line| line.contains("Configuration: dev")));
    let form_index = line_index(&lines, "Edit active gcloud context");
    let project_index = line_index_after(&lines, form_index, "  Project: dev-project");
    let region_index = line_index_after(&lines, form_index, "> Region: europe-west1");
    let zone_index = line_index_after(&lines, form_index, "  Zone: europe-west1-b");
    assert!(project_index < region_index && region_index < zone_index);
    assert!(
        lines
            .iter()
            .any(|line| line.contains("Enter save") && line.contains("Esc cancel"))
    );
}

#[tokio::test]
async fn configurations_context_edit_form_scrolls_the_focused_field_at_minimum_frame_size() {
    let mut session = RuntimeSession::new_for_test(AppState::new());
    session
        .apply_effect_result(EffectResult::ConfigurationsLoaded {
            configurations: vec![GcloudConfiguration {
                name: GcloudConfigurationName::new("dev"),
                is_active: true,
                account: Some("dev@example.com".to_string()),
                project: Some("dev-project".to_string()),
                region: Some("europe-west1".to_string()),
                zone: Some("europe-west1-b".to_string()),
            }],
        })
        .await;
    session
        .dispatch_action(Action::Workspace(WorkspaceAction::Configurations))
        .await;
    session
        .dispatch_action(Action::Configurations(
            ConfigurationsAction::StartEditActiveContext,
        ))
        .await;
    for _ in 0..2 {
        session
            .dispatch_action(Action::Configurations(
                ConfigurationsAction::FocusNextContextEditField,
            ))
            .await;
    }

    let lines = render_lines(&session, 60, 12);
    assert!(lines.iter().any(|line| line.contains("> Zone:")));
    assert!(lines.iter().any(|line| line.contains("Enter Submit")));
    assert!(lines.iter().any(|line| line.contains("Esc")));
}

#[tokio::test]
async fn configurations_context_edit_form_renders_status_message_in_shared_region() {
    let mut session = RuntimeSession::new_for_test(AppState::new());
    session
        .apply_effect_result(EffectResult::ConfigurationsLoaded {
            configurations: vec![GcloudConfiguration {
                name: GcloudConfigurationName::new("dev"),
                is_active: true,
                account: Some("dev@example.com".to_string()),
                project: None,
                region: None,
                zone: None,
            }],
        })
        .await;
    session
        .dispatch_action(Action::Workspace(WorkspaceAction::Configurations))
        .await;
    session
        .dispatch_action(Action::Configurations(
            ConfigurationsAction::StartEditActiveContext,
        ))
        .await;
    session
        .apply_effect_result(EffectResult::Failed {
            error: AppError::gcloud("edit failed"),
        })
        .await;

    let backend = TestBackend::new(100, 14);
    let mut terminal = Terminal::new(backend).expect("terminal should initialize");
    terminal
        .draw(|frame| nuvem::ui::render(frame, &session))
        .expect("render should succeed");

    let lines = terminal_lines(&terminal);

    assert!(
        lines
            .iter()
            .any(|line| line.contains("unknown error — Retry, or copy the details to investigate"))
    );
    assert!(!lines.iter().any(|line| line.contains("edit failed")));
    assert!(!lines.iter().any(|line| {
        line.contains("Status: unknown error — Retry, or copy the details to investigate")
    }));
    assert!(lines.iter().any(|line| line.contains("> Project:")));
}

#[test]
fn configuration_picker_renders_loading_panel() {
    let app = app_with_open_context_picker(0);
    let session = RuntimeSession::new_for_test(app);

    let lines = render_lines(&session, 100, 24);

    assert_shared_frame(&lines, "Select Project", "/ Filter");
    assert!(lines.iter().any(|line| line.contains("pick project")));
    assert!(
        lines
            .iter()
            .any(|line| line.contains("Loading projects..."))
    );
    assert!(lines.iter().any(|line| line.contains("/ filter")));
    assert!(lines.iter().any(|line| line.contains("r retry")));
    assert!(lines.iter().any(|line| line.contains("Esc back")));
    assert!(!lines.iter().any(|line| line.contains("Ctrl-Space pick")));
}

#[test]
fn configuration_picker_preserves_shared_frame_around_overlay() {
    let app = app_with_open_context_picker(0);
    let session = RuntimeSession::new_for_test(app);

    let lines = render_lines(&session, 100, 24);

    assert_shared_frame(&lines, "Select Project", "/ Filter");
    assert!(lines.iter().any(|line| line.contains("pick project")));
}

#[test]
fn configuration_picker_renders_project_rows_and_selection() {
    let mut app = app_with_open_context_picker(0);
    load_picker_items(
        &mut app,
        ConfigurationContextPickerKind::Project,
        vec![
            ConfigurationContextPickerItem::Project(GcloudProject {
                project_id: "dev-project".to_string(),
                name: "Development".to_string(),
                lifecycle_state: Some("ACTIVE".to_string()),
            }),
            ConfigurationContextPickerItem::Project(GcloudProject {
                project_id: "prod-project".to_string(),
                name: "Production".to_string(),
                lifecycle_state: Some("ACTIVE".to_string()),
            }),
        ],
    );
    let session = RuntimeSession::new_for_test(app);

    let lines = render_lines(&session, 120, 24);

    assert!(lines.iter().any(|line| line.contains(&format!(
        "{CONTEXT_PICKER_SELECTED_UNMARKED_ROW_PREFIX}dev-project"
    )) && line.contains("Development")));
    assert!(
        lines
            .iter()
            .any(|line| line.contains("prod-project") && line.contains("Production"))
    );
}

#[test]
fn configuration_picker_keeps_selected_row_visible_in_a_long_list() {
    let mut app = app_with_open_context_picker(0);
    load_picker_items(
        &mut app,
        ConfigurationContextPickerKind::Project,
        long_project_picker_items(40),
    );

    let initial_lines = render_lines(&RuntimeSession::new_for_test(app.clone()), 100, 24);
    assert!(initial_lines.iter().any(|line| line.contains(&format!(
        "{CONTEXT_PICKER_SELECTED_UNMARKED_ROW_PREFIX}project-00"
    ))));
    assert!(!initial_lines.iter().any(|line| line.contains("project-39")));
    let small_lines = render_lines(&RuntimeSession::new_for_test(app.clone()), 60, 12);
    assert!(small_lines.iter().any(|line| line.contains(&format!(
        "{CONTEXT_PICKER_SELECTED_UNMARKED_ROW_PREFIX}project-00"
    ))));

    for _ in 0..15 {
        reduce(
            &mut app,
            &Action::Configurations(ConfigurationsAction::SelectNextContextPickerItem),
        );
    }
    let middle_lines = render_lines(&RuntimeSession::new_for_test(app.clone()), 100, 24);
    assert!(middle_lines.iter().any(|line| line.contains(&format!(
        "{CONTEXT_PICKER_SELECTED_UNMARKED_ROW_PREFIX}project-15"
    ))));
    assert!(!middle_lines.iter().any(|line| line.contains("project-00")));

    for _ in 0..40 {
        reduce(
            &mut app,
            &Action::Configurations(ConfigurationsAction::SelectNextContextPickerItem),
        );
    }
    let final_lines = render_lines(&RuntimeSession::new_for_test(app), 100, 24);
    assert!(final_lines.iter().any(|line| line.contains(&format!(
        "{CONTEXT_PICKER_SELECTED_UNMARKED_ROW_PREFIX}project-39"
    ))));
    assert!(!final_lines.iter().any(|line| line.contains("project-00")));
}

#[test]
fn configuration_picker_keeps_full_chrome_at_normal_and_narrow_sizes() {
    let mut app = app_with_open_context_picker(0);
    load_picker_items(
        &mut app,
        ConfigurationContextPickerKind::Project,
        long_project_picker_items(40),
    );

    let normal_lines = render_lines(&RuntimeSession::new_for_test(app.clone()), 100, 24);
    let narrow_lines = render_lines(&RuntimeSession::new_for_test(app), 60, 12);

    for lines in [&normal_lines, &narrow_lines] {
        assert!(lines.iter().any(|line| line.contains("pick project")));
        assert!(lines.iter().any(|line| line.contains(&format!(
            "{CONTEXT_PICKER_SELECTED_UNMARKED_ROW_PREFIX}project-00"
        ))));
        assert!(lines.iter().any(|line| line.contains("Enter choose")));
        assert!(lines.iter().any(|line| line.contains("Esc back")));
    }
    assert!(narrow_lines.iter().any(|line| line.contains(&format!(
        "{CONTEXT_PICKER_UNSELECTED_UNMARKED_ROW_PREFIX}project-02"
    ))));
    assert_eq!(line_index(&normal_lines, "pick project"), 4);
}

#[test]
fn configuration_picker_renders_region_and_zone_rows() {
    let mut region_app = app_with_open_context_picker(1);
    load_picker_items(
        &mut region_app,
        ConfigurationContextPickerKind::Region,
        vec![ConfigurationContextPickerItem::Region(ComputeRegion {
            name: "europe-west1".to_string(),
            status: Some("UP".to_string()),
        })],
    );
    let region_session = RuntimeSession::new_for_test(region_app);
    let region_lines = render_lines(&region_session, 120, 24);
    assert!(region_lines.iter().any(|line| line.contains(&format!(
        "{CONTEXT_PICKER_SELECTED_UNMARKED_ROW_PREFIX}europe-west1"
    )) && line.contains("UP")));

    let mut zone_app = app_with_open_context_picker(2);
    load_picker_items(
        &mut zone_app,
        ConfigurationContextPickerKind::Zone,
        vec![ConfigurationContextPickerItem::Zone(ComputeZone {
            name: "europe-west1-b".to_string(),
            region: Some("europe-west1".to_string()),
            status: Some("UP".to_string()),
        })],
    );
    let zone_session = RuntimeSession::new_for_test(zone_app);
    let zone_lines = render_lines(&zone_session, 120, 24);
    assert!(zone_lines.iter().any(|line| {
        line.contains(&format!(
            "{CONTEXT_PICKER_SELECTED_UNMARKED_ROW_PREFIX}europe-west1-b"
        )) && line.contains("europe-west1")
            && line.contains("UP")
    }));
}

#[test]
fn configuration_picker_renders_empty_and_filtered_empty_states() {
    let mut empty_app = app_with_open_context_picker(0);
    load_picker_items(
        &mut empty_app,
        ConfigurationContextPickerKind::Project,
        Vec::new(),
    );
    let empty_session = RuntimeSession::new_for_test(empty_app);
    let empty_lines = render_lines(&empty_session, 100, 24);
    assert!(
        empty_lines
            .iter()
            .any(|line| line.contains("No projects found"))
    );

    let mut filtered_app = app_with_open_context_picker(0);
    load_picker_items(
        &mut filtered_app,
        ConfigurationContextPickerKind::Project,
        vec![ConfigurationContextPickerItem::Project(GcloudProject {
            project_id: "dev-project".to_string(),
            name: "Development".to_string(),
            lifecycle_state: Some("ACTIVE".to_string()),
        })],
    );
    reduce(
        &mut filtered_app,
        &Action::Configurations(ConfigurationsAction::StartContextPickerFilter),
    );
    reduce(
        &mut filtered_app,
        &Action::Configurations(ConfigurationsAction::UpdateContextPickerFilter(
            "missing".to_string(),
        )),
    );
    let filtered_session = RuntimeSession::new_for_test(filtered_app);
    let filtered_lines = render_lines(&filtered_session, 100, 24);
    assert!(
        filtered_lines
            .iter()
            .any(|line| line.contains("No projects match filter \"missing\""))
    );
    assert!(
        filtered_lines
            .iter()
            .any(|line| line.contains("Backspace clear"))
    );
    assert!(
        filtered_lines
            .iter()
            .any(|line| line.contains("Esc cancel filter"))
    );
}

#[test]
fn configuration_picker_cancel_filter_keeps_the_applied_filter_visible() {
    let mut app = app_with_open_context_picker(0);
    load_picker_items(
        &mut app,
        ConfigurationContextPickerKind::Project,
        vec![
            ConfigurationContextPickerItem::Project(GcloudProject {
                project_id: "dev-project".to_string(),
                name: "Development".to_string(),
                lifecycle_state: Some("ACTIVE".to_string()),
            }),
            ConfigurationContextPickerItem::Project(GcloudProject {
                project_id: "prod-project".to_string(),
                name: "Production".to_string(),
                lifecycle_state: Some("ACTIVE".to_string()),
            }),
        ],
    );
    for action in [
        ConfigurationsAction::StartContextPickerFilter,
        ConfigurationsAction::UpdateContextPickerFilter("prod".to_string()),
        ConfigurationsAction::ApplyContextPickerFilter,
        ConfigurationsAction::StartContextPickerFilter,
        ConfigurationsAction::UpdateContextPickerFilter("dev".to_string()),
        ConfigurationsAction::CancelContextPickerFilter,
    ] {
        reduce(&mut app, &Action::Configurations(action));
    }

    let lines = render_lines(&RuntimeSession::new_for_test(app), 100, 24);

    assert!(lines.iter().any(|line| line.contains("Filter: prod")));
    assert!(lines.iter().any(|line| line.contains(&format!(
        "{CONTEXT_PICKER_SELECTED_UNMARKED_ROW_PREFIX}prod-project"
    ))));
    assert!(!lines.iter().any(|line| line.contains("Filter: prod_")));
    assert!(!lines.iter().any(|line| line.contains("Esc cancel filter")));
}

#[test]
fn configuration_picker_renders_degraded_state_with_retry() {
    let mut app = app_with_open_context_picker(0);
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ContextPickerLoadFailed {
            configuration: GcloudConfigurationName::new("dev"),
            kind: ConfigurationContextPickerKind::Project,
            error: AppError::gcloud("permission denied"),
        }),
    );
    assert!(matches!(
        app.root
            .configurations
            .context_picker()
            .map(|picker| picker.status()),
        Some(nuvem::app::state::configurations::ConfigurationContextPickerStatus::Degraded(detail))
            if detail.contains("permission denied")
    ));
    let session = RuntimeSession::new_for_test(app);

    let lines = render_lines(&session, 100, 24);

    assert!(
        lines
            .iter()
            .any(|line| { line.contains("Unable to load projects:") })
    );
    assert!(lines.iter().any(|line| line.contains("r retry")));
    assert!(lines.iter().any(|line| line.contains("/ filter")));
    assert!(lines.iter().any(|line| line.contains("Esc back")));
}

#[test]
fn context_picker_is_rendered_above_shared_frame_chrome() {
    let mut app = app_with_open_context_picker(0);
    load_picker_items(
        &mut app,
        ConfigurationContextPickerKind::Project,
        vec![ConfigurationContextPickerItem::Project(GcloudProject {
            project_id: "dev-project".to_string(),
            name: "Development".to_string(),
            lifecycle_state: Some("ACTIVE".to_string()),
        })],
    );
    let session = RuntimeSession::new_for_test(app);
    let terminal = render_terminal(&session, 60, 12);
    let line = terminal_lines(&terminal)[9].clone();

    assert!(
        line.contains("└"),
        "picker lower border was overwritten: {line:?}"
    );
}

#[test]
fn configuration_picker_truncates_long_rows_without_hiding_picker_controls() {
    let mut app = app_with_open_context_picker(0);
    load_picker_items(
        &mut app,
        ConfigurationContextPickerKind::Project,
        vec![ConfigurationContextPickerItem::Project(GcloudProject {
            project_id: "dev-project".to_string(),
            name: "Development".to_string(),
            lifecycle_state: Some("ACTIVE".to_string()),
        })],
    );
    let session = RuntimeSession::new_for_test(app);

    let lines = render_lines(&session, 100, 24);

    assert!(lines.iter().any(|line| line.contains(&format!(
        "{CONTEXT_PICKER_SELECTED_UNMARKED_ROW_PREFIX}dev-project"
    ))));
    assert!(lines.iter().any(|line| line.contains("Enter choose")));
    assert!(lines.iter().any(|line| line.contains("Esc back")));
    assert!(
        !lines
            .iter()
            .any(|line| line.contains("Picker does not fit in this terminal"))
    );
}

#[tokio::test]
async fn configuration_context_edit_footer_advertises_ctrl_space_pick_value() {
    // The static all-states cheat sheet this test used to check against
    // ("Ctrl-Space  Pick focused context value") no longer exists: help is
    // now contextual and only opens from Normal mode, so it can no longer
    // document a shortcut (Ctrl-Space picking a context field's value) that
    // is only reachable from the context-edit form's own `EditConfiguration`
    // mode. That shortcut is instead surfaced through the real shared-frame
    // footer, sourced from the same binding catalog (`CTX_PICK_VALUE`) that
    // backs the help view — assert it appears there for the context-edit
    // form, which is the only place it's actually reachable.
    let mut session = RuntimeSession::new_for_test(AppState::new());
    session
        .apply_effect_result(EffectResult::ConfigurationsLoaded {
            configurations: vec![GcloudConfiguration {
                name: GcloudConfigurationName::new("dev"),
                is_active: true,
                account: Some("dev@example.com".to_string()),
                project: Some("dev-project".to_string()),
                region: Some("europe-west1".to_string()),
                zone: Some("europe-west1-b".to_string()),
            }],
        })
        .await;
    session
        .dispatch_action(Action::Workspace(WorkspaceAction::Configurations))
        .await;
    session
        .dispatch_action(Action::Configurations(
            ConfigurationsAction::StartEditActiveContext,
        ))
        .await;

    let footer_hints = project_app_frame(&session.app().root).footer_hints;
    assert!(
        footer_hints
            .iter()
            .any(|hint| hint.key == "Ctrl-Space" && hint.label == "Pick value")
    );
}

#[tokio::test]
async fn configurations_list_renders_create_failure_status_after_form_closes() {
    let mut session = RuntimeSession::new_for_test(AppState::new());
    session
        .dispatch_action(Action::Workspace(WorkspaceAction::Configurations))
        .await;
    session
        .dispatch_action(Action::Configurations(ConfigurationsAction::StartCreate))
        .await;
    session
        .dispatch_action(Action::Configurations(ConfigurationsAction::CancelCreate))
        .await;
    session
        .apply_effect_result(EffectResult::ConfigurationCreatedReloadFailed {
            configuration: GcloudConfiguration {
                name: GcloudConfigurationName::new("dev"),
                is_active: true,
                account: Some("dev@example.com".to_string()),
                project: Some("dev-project".to_string()),
                region: None,
                zone: None,
            },
            error: AppError::gcloud("reload failed"),
        })
        .await;

    let backend = TestBackend::new(100, 14);
    let mut terminal = Terminal::new(backend).expect("terminal should initialize");
    terminal
        .draw(|frame| nuvem::ui::render(frame, &session))
        .expect("render should succeed");

    let lines = terminal_lines(&terminal);

    assert!(
        lines
            .iter()
            .any(|line| line.contains("unknown error — Retry, or copy the details to investigate"))
    );
    assert!(!lines.iter().any(|line| line.contains("reload failed")));
    assert!(!lines.iter().any(|line| {
        line.contains("Status: unknown error — Retry, or copy the details to investigate")
    }));
    assert!(lines.iter().any(|line| line.contains("> * dev")));
}

#[tokio::test]
async fn configurations_list_does_not_render_empty_name_status_after_cancel() {
    let mut session = RuntimeSession::new_for_test(AppState::new());
    session
        .dispatch_action(Action::Workspace(WorkspaceAction::Configurations))
        .await;
    session
        .dispatch_action(Action::Configurations(ConfigurationsAction::StartCreate))
        .await;
    session
        .dispatch_action(Action::Configurations(ConfigurationsAction::SubmitCreate))
        .await;
    session
        .dispatch_action(Action::Configurations(ConfigurationsAction::CancelCreate))
        .await;

    let backend = TestBackend::new(100, 14);
    let mut terminal = Terminal::new(backend).expect("terminal should initialize");
    terminal
        .draw(|frame| nuvem::ui::render(frame, &session))
        .expect("render should succeed");

    let lines = terminal_lines(&terminal);

    assert!(
        !lines
            .iter()
            .any(|line| line.contains("Configuration name is required"))
    );
    assert!(
        lines
            .iter()
            .any(|line| line.contains("No configurations loaded. Press r to refresh."))
    );
}

#[tokio::test]
async fn help_overlay_is_visible_when_help_is_toggled() {
    let mut session = RuntimeSession::new_for_test(AppState::new());
    session
        .dispatch_action(Action::System(SystemAction::ToggleHelp))
        .await;

    let backend = TestBackend::new(73, 27);
    let mut terminal = Terminal::new(backend).expect("terminal should initialize");
    terminal
        .draw(|frame| nuvem::ui::render(frame, &session))
        .expect("render should succeed");

    let lines = terminal_lines(&terminal);

    // The old static "nuvem keys" all-states cheat sheet no longer exists:
    // help is now a mode-aware, scrollable overlay built from the same
    // binding catalog that drives the footer (`app::projection::help_model`),
    // grouped into "Current view" / "Global" / "Navigation" sections for
    // whatever view is underneath it (Home, by default here).
    assert!(lines.iter().any(|line| line.contains("help"))); // modal block title
    assert!(lines.iter().any(|line| line.contains("Home"))); // view title
    for expected in [
        "Current view",
        "Global",
        "Navigation",
        "r  Refresh",
        "?  Help",
        "q  Quit",
        "h  Home",
        "c  Configurations",
        "g  Resources",
    ] {
        assert!(
            lines.iter().any(|line| line.contains(expected)),
            "expected help line containing {expected:?}"
        );
    }
    // Sections render in a fixed order: current view, then global, then
    // navigation.
    let current_view_index = line_index(&lines, "Current view");
    let global_index = line_index(&lines, "Global");
    let navigation_index = line_index(&lines, "Navigation");
    assert!(current_view_index < global_index && global_index < navigation_index);
    // Position/overflow indicator row is always present.
    assert!(lines.iter().any(|line| line.contains(" of ")));
}

#[tokio::test]
async fn help_overlay_renders_scrollable_content_at_sizes_that_used_to_clip() {
    // The old fixed 73x22 "nuvem help needs at least 73x22" clip-fallback no
    // longer exists: the scrollable, mode-aware overlay renders bounded,
    // grouped content at any size the app itself supports. 72x22 and 80x21
    // (just below/around the old fallback threshold) now render real help
    // content instead of refusing to draw. The only remaining "too small"
    // case is the app-level minimum (60x12, see `ui::mod::HELP_MIN_WIDTH`/
    // `HELP_MIN_HEIGHT`), below which the whole app — not just help — falls
    // back to a small-terminal message.
    let mut session = RuntimeSession::new_for_test(AppState::new());
    session
        .dispatch_action(Action::System(SystemAction::ToggleHelp))
        .await;

    for (width, height) in [(72u16, 22u16), (80, 21)] {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).expect("terminal should initialize");
        terminal
            .draw(|frame| nuvem::ui::render(frame, &session))
            .expect("render should succeed");

        let lines = terminal_lines(&terminal);

        assert!(
            lines.iter().any(|line| line.contains("help")),
            "expected the help overlay to render at {width}x{height}"
        );
        assert!(
            lines.iter().any(|line| line.contains("Global")),
            "expected grouped help content at {width}x{height}"
        );
        assert!(
            !lines
                .iter()
                .any(|line| line.contains("nuvem help needs at least 73x22")),
            "the removed static clip-fallback must not reappear at {width}x{height}"
        );
    }
}

#[tokio::test]
async fn overlay_modal_chrome_is_semantic_at_80x24() {
    let mut help_session = RuntimeSession::new_for_test(AppState::new());
    help_session
        .dispatch_action(Action::System(SystemAction::ToggleHelp))
        .await;
    assert_modal_chrome(&render_terminal(&help_session, 80, 24), "help");

    let mut command_session = RuntimeSession::new_for_test(AppState::new());
    command_session
        .dispatch_action(Action::Command(nuvem::app::CommandAction::Open))
        .await;
    assert_modal_chrome(&render_terminal(&command_session, 80, 24), "commands");

    let mut resource_session = session_with_loaded_instances().await;
    resource_session
        .dispatch_action(Action::Command(nuvem::app::CommandAction::Open))
        .await;
    resource_session
        .dispatch_action(Action::Command(nuvem::app::CommandAction::Submit))
        .await;
    assert_modal_chrome(
        &render_terminal(&resource_session, 80, 24),
        "confirm GCE action",
    );

    let configuration_session = session_with_activation_confirmation().await;
    assert_modal_chrome(
        &render_terminal(&configuration_session, 80, 24),
        "confirm configuration activation",
    );

    let mut ssh_session = session_with_loaded_instances().await;
    ssh_session
        .dispatch_action(Action::Command(nuvem::app::CommandAction::Open))
        .await;
    for _ in 0..3 {
        ssh_session
            .dispatch_action(Action::Command(nuvem::app::CommandAction::SelectNext))
            .await;
    }
    ssh_session
        .dispatch_action(Action::Command(nuvem::app::CommandAction::Submit))
        .await;
    assert_modal_chrome(&render_terminal(&ssh_session, 80, 24), "command preview");

    let mut picker_app = app_with_open_context_picker(0);
    load_picker_items(
        &mut picker_app,
        ConfigurationContextPickerKind::Project,
        long_project_picker_items(2),
    );
    let picker_session = RuntimeSession::new_for_test(picker_app);
    assert_modal_chrome(&render_terminal(&picker_session, 80, 24), "pick project");
}

#[tokio::test]
async fn small_terminal_overlays_preserve_fallback_messages_at_60x12() {
    let mut help_session = RuntimeSession::new_for_test(AppState::new());
    help_session
        .dispatch_action(Action::System(SystemAction::ToggleHelp))
        .await;
    let help_lines = render_lines(&help_session, 60, 12);
    // 60x12 is the app's own minimum supported size (see
    // `ui::mod::HELP_MIN_WIDTH`/`HELP_MIN_HEIGHT`), so the scrollable,
    // mode-aware help overlay renders real grouped content here — not the
    // old static 73x22 clip-fallback, which no longer exists. At this size
    // the content overflows the viewport, so the position indicator carries
    // a "▼" marker.
    assert!(help_lines.iter().any(|line| line.contains("help")));
    assert!(
        help_lines
            .iter()
            .any(|line| line.contains(" of ") && line.contains('▼'))
    );
    assert!(
        !help_lines
            .iter()
            .any(|line| line.contains("nuvem help needs at least 73x22"))
    );

    let mut resource_session = session_with_loaded_instances().await;
    resource_session
        .dispatch_action(Action::Command(nuvem::app::CommandAction::Open))
        .await;
    resource_session
        .dispatch_action(Action::Command(nuvem::app::CommandAction::SelectNext))
        .await;
    resource_session
        .dispatch_action(Action::Command(nuvem::app::CommandAction::Submit))
        .await;
    let resource_lines = render_lines(&resource_session, 60, 12);
    assert!(
        resource_lines
            .iter()
            .any(|line| line.contains("nuvem GCE confirmation needs a larger terminal"))
    );

    let configuration_lines = render_lines(&session_with_activation_confirmation().await, 60, 12);
    assert!(
        configuration_lines
            .iter()
            .any(|line| { line.contains("nuvem activation confirmation needs a larger terminal") })
    );

    // The SSH preview is deliberately no longer in this inventory. It is short
    // enough to wrap into 60x12 now, so it has no fallback left to
    // preserve, and
    // `ssh_command_preview_renders_whole_at_the_smallest_supported_terminal`
    // states that instead. What is kept here is the modals whose content still
    // does not fit at this size, which is what keeps this test about the
    // fallback rather than about a path nothing reaches.

    // The plugin confirmation belongs in this inventory for a reason the others
    // do not carry: its `Enter` runs the command, so the fallback is what the
    // keymap's visibility guard is reading when it refuses.
    //
    // Its argv is long on purpose: a short one wraps into 60x12 now
    // and takes no fallback at all, which would leave the guard below asserting
    // a path nothing reaches.
    let mut plugin_app = AppState::new();
    plugin_app.root.resources.show_plugin_confirmation(
        "Describe web 01".to_string(),
        vec![
            "compute".to_string(),
            "instances".to_string(),
            "describe".to_string(),
            "web-01".to_string(),
            "--zone=europe-west1-b".to_string(),
            "--project=acme-production-platform".to_string(),
            "--format=json(name,status,machineType,networkInterfaces)".to_string(),
        ],
        false,
    );
    let plugin_lines = render_lines(&RuntimeSession::new_for_test(plugin_app), 60, 12);
    assert!(
        plugin_lines
            .iter()
            .any(|line| line.contains("nuvem plugin confirmation needs a larger terminal"))
    );

    // And the prompt in front of it, for the same reason one step earlier: its
    // `Enter` composes the argv out of values the operator has to be able to
    // read.
    let prompt_lines = render_lines(&RuntimeSession::new_for_test(app_prompting()), 60, 12);
    assert!(
        prompt_lines
            .iter()
            .any(|line| line.contains("nuvem plugin input needs a larger terminal"))
    );
}

/// A prompt with more fields than a 60x12 modal can hold, one of them a
/// dropdown and one of them empty.
fn app_prompting() -> AppState {
    use nuvem::app::state::plugins::PluginId;
    use nuvem::app::state::resources::PluginPrompt;
    use nuvem::config::plugins::{PluginInput, PluginInputKind};

    let mut app = AppState::new();
    app.root.resources.show_plugin_prompt(PluginPrompt::new(
        PluginId(0),
        "Resize web-01".to_string(),
        false,
        vec![
            PluginInput {
                name: "disk_size".to_string(),
                kind: PluginInputKind::String,
                default: Some("100".to_string()),
            },
            PluginInput {
                name: "tier".to_string(),
                kind: PluginInputKind::Dropdown {
                    choices: vec!["db-f1-micro".to_string(), "db-g1-small".to_string()],
                },
                default: Some("db-g1-small".to_string()),
            },
            PluginInput {
                name: "note".to_string(),
                kind: PluginInputKind::String,
                default: None,
            },
        ],
        Vec::new(),
    ));
    app
}

/// The prompt drawn at a size that fits it: the operator sees which field they
/// are editing, what a dropdown currently holds, and why `Enter` is inert.
#[test]
fn the_plugin_prompt_draws_its_fields_at_120x40() {
    let lines = render_lines(&RuntimeSession::new_for_test(app_prompting()), 120, 40);

    assert!(
        lines.iter().any(|line| line.contains("Resize web-01")),
        "the modal names the plugin: {lines:#?}"
    );
    assert!(
        lines.iter().any(|line| line.contains("> disk_size: 100")),
        "the focused field carries the marker: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line.contains("tier: < db-g1-small >")),
        "a dropdown shows its choice, not free text: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line.contains("note: (needs a value)")),
        "an empty field says what it needs: {lines:#?}"
    );
    assert!(
        !lines.iter().any(|line| line.contains("Enter continue")),
        "Enter is inert while a field is empty, so it is not offered: {lines:#?}"
    );
}

#[tokio::test]
async fn overlay_full_preview_renders_complete_content_at_120x40() {
    let mut help_session = RuntimeSession::new_for_test(AppState::new());
    help_session
        .dispatch_action(Action::System(SystemAction::ToggleHelp))
        .await;
    let help_lines = render_lines(&help_session, 120, 40);
    // The old static "nuvem keys" cheat sheet no longer exists; at 120x40
    // the scrollable, mode-aware overlay has ample room to render every
    // grouped section (and, per `render_help_overlay`, a position indicator
    // even when nothing overflows).
    assert!(help_lines.iter().any(|line| line.contains("Global")));
    assert!(help_lines.iter().any(|line| line.contains("Navigation")));
    assert!(help_lines.iter().any(|line| line.contains(" of ")));

    let mut command_session = RuntimeSession::new_for_test(AppState::new());
    command_session
        .dispatch_action(Action::Command(nuvem::app::CommandAction::Open))
        .await;
    let command_lines = render_lines(&command_session, 120, 40);
    assert!(command_lines.iter().any(|line| line.contains("commands")));

    let mut resource_session = session_with_loaded_instances().await;
    resource_session
        .dispatch_action(Action::Command(nuvem::app::CommandAction::Open))
        .await;
    resource_session
        .dispatch_action(Action::Command(nuvem::app::CommandAction::Submit))
        .await;
    let resource_lines = render_lines(&resource_session, 120, 40);
    assert!(resource_lines.iter().any(|line| {
        line.contains("compute.instances.stop zones/europe-west1-b/instances/api-dev-1")
    }));

    let configuration_lines = render_lines(&session_with_activation_confirmation().await, 120, 40);
    assert!(
        configuration_lines
            .iter()
            .any(|line| line.contains("Target: prod"))
    );

    let mut ssh_session = session_with_loaded_instances().await;
    ssh_session
        .dispatch_action(Action::Command(nuvem::app::CommandAction::Open))
        .await;
    for _ in 0..3 {
        ssh_session
            .dispatch_action(Action::Command(nuvem::app::CommandAction::SelectNext))
            .await;
    }
    ssh_session
        .dispatch_action(Action::Command(nuvem::app::CommandAction::Submit))
        .await;
    let ssh_lines = render_lines(&ssh_session, 120, 40);
    assert!(
        ssh_lines
            .iter()
            .any(|line| line.contains("Enter copy   Esc cancel"))
    );

    let mut picker_app = app_with_open_context_picker(0);
    load_picker_items(
        &mut picker_app,
        ConfigurationContextPickerKind::Project,
        long_project_picker_items(2),
    );
    let picker_lines = render_lines(&RuntimeSession::new_for_test(picker_app), 120, 40);
    assert!(picker_lines.iter().any(|line| line.contains(&format!(
        "{CONTEXT_PICKER_SELECTED_UNMARKED_ROW_PREFIX}project-00"
    ))));
}

#[tokio::test]
async fn command_palette_renders_small_overlay_at_60x12() {
    let mut session = session_with_loaded_instances().await;
    session
        .dispatch_action(Action::Command(nuvem::app::CommandAction::Open))
        .await;

    let lines = render_lines(&session, 60, 12);

    assert_shared_frame(&lines, "Resources", "Enter Run");
    assert!(!lines.iter().any(|line| line.contains("r Refresh")));
    assert!(lines.iter().any(|line| line.contains("commands")));
    assert!(
        lines
            .iter()
            .any(|line| line.contains("> Stop selected GCE instance"))
    );
}

#[test]
fn configuration_picker_renders_small_loading_overlay_at_60x12() {
    let app = app_with_open_context_picker(0);
    let session = RuntimeSession::new_for_test(app);

    let lines = render_lines(&session, 60, 12);

    assert_shared_frame(&lines, "Select Project", "/ Filter");
    assert!(lines.iter().any(|line| line.contains("pick project")));
    assert!(
        lines
            .iter()
            .any(|line| line.contains("Loading projects..."))
    );
    assert!(lines.iter().any(|line| line.contains("/ filter")));
    assert!(lines.iter().any(|line| line.contains("r retry")));
    assert!(lines.iter().any(|line| line.contains("Esc back")));
}

#[tokio::test]
async fn resources_view_shows_empty_state_for_loaded_empty_context() {
    let mut session = RuntimeSession::new_for_test(AppState::new());
    session
        .apply_effect_result(EffectResult::ConfigurationsLoaded {
            configurations: vec![GcloudConfiguration {
                name: GcloudConfigurationName::new("dev"),
                is_active: true,
                account: Some("dev@example.com".to_string()),
                project: Some("dev-project".to_string()),
                region: None,
                zone: None,
            }],
        })
        .await;
    let context = session.app().root.current_context().unwrap().clone();
    session
        .apply_effect_result(EffectResult::ComputeInstancesLoaded {
            context,
            instances: Vec::new(),
        })
        .await;
    session
        .dispatch_action(Action::Workspace(WorkspaceAction::Resources))
        .await;

    let backend = TestBackend::new(90, 12);
    let mut terminal = Terminal::new(backend).expect("terminal should initialize");
    terminal
        .draw(|frame| nuvem::ui::render(frame, &session))
        .expect("render should succeed");

    let lines = terminal_lines(&terminal);

    assert!(
        lines
            .iter()
            .any(|line| line.contains("No GCE instances found for dev"))
    );
}

#[tokio::test]
async fn resources_view_renders_gce_list_through_public_render() {
    let session = session_with_loaded_instances().await;

    let lines = render_lines(&session, 80, 24);

    assert_shared_frame(&lines, "Resources", "Enter Open");
    assert!(lines.iter().any(|line| line.contains("Configuration: dev")));
    assert!(
        lines
            .iter()
            .any(|line| line.contains("Home > Resources > GCE instances"))
    );
    assert!(
        lines
            .iter()
            .any(|line| line.contains("Resources / GCE instances"))
    );
    assert!(lines.iter().any(|line| {
        line.contains(&format!("{SELECTED_UNMARKED_ROW_PREFIX}api-dev-1"))
            && line.contains("Running")
            && line.contains("europe-west1-b")
            && line.contains("e2-medium")
            && line.contains("10.0.0.2")
            && line.contains("-")
    }));
}

#[tokio::test]
async fn resources_view_renders_gce_column_headers() {
    let session = session_with_loaded_instances().await;
    let lines = render_lines(&session, 100, 24);
    assert!(
        lines
            .iter()
            .any(|line| line.contains("NAME") && line.contains("STATUS"))
    );
}

#[tokio::test]
async fn resources_view_narrow_width_drops_a_gce_column() {
    // `nuvem::ui::render` refuses to draw below 60x12, and at that minimum
    // width the GCE table's tools line (which already exceeds 60 characters
    // on its own) is clipped before any column would ever need to be
    // dropped, so a full-frame render can't exercise the hidden-column path.
    // Instead, drive `plan_table` directly (the same function
    // `render_resource_table` uses to compute the hint) on the real GCE
    // table model at a width that forces a column out, proving the
    // `hidden_count` the tools line's "+N cols (w)" hint is derived from is
    // actually produced for this dataset.
    let session = session_with_loaded_instances().await;
    let projection = nuvem::app::projection::project_resources(&session.app().root.resources);
    let model = projection
        .table
        .as_ref()
        .expect("GCE table model should be present once instances are loaded");

    let plan = nuvem::ui::shared::table::plan_table(model, 40, false);

    assert!(
        plan.hidden_count > 0,
        "expected at least one GCE column to be hidden at width 40, plan: {plan:?}"
    );
}

#[tokio::test]
async fn resources_view_renders_gke_cluster_rows() {
    let mut session = RuntimeSession::new_for_test(AppState::new());
    load_active_dev(&mut session).await;
    session
        .dispatch_action(Action::Resources(
            nuvem::app::ResourcesAction::SelectService(ResourceServiceKind::GkeClusters),
        ))
        .await;
    let context = session.app().root.current_context().unwrap().clone();
    session
        .apply_effect_result(EffectResult::GkeClustersLoaded {
            context,
            clusters: vec![gke_cluster("platform-dev", "europe-west1")],
        })
        .await;
    session
        .dispatch_action(Action::Workspace(WorkspaceAction::Resources))
        .await;

    let lines = render_lines(&session, 80, 24);

    assert_shared_frame(&lines, "Resources", "Enter Open");
    assert!(lines.iter().any(|line| line.contains("Configuration: dev")));
    assert!(
        lines
            .iter()
            .any(|line| line.contains("Home > Resources > GKE clusters"))
    );
    assert!(
        lines
            .iter()
            .any(|line| line.contains("Resources / GKE clusters"))
    );
    assert!(lines.iter().any(|line| {
        line.contains(&format!("{SELECTED_ROW_PREFIX_NO_MARK}platform-dev"))
            && line.contains("europe-west1")
            && line.contains("1.31.5-gke.1023000")
            && line.contains("Public")
    }));
}

#[tokio::test]
async fn resources_view_renders_gke_empty_state() {
    let mut session = RuntimeSession::new_for_test(AppState::new());
    load_active_dev(&mut session).await;
    session
        .dispatch_action(Action::Resources(
            nuvem::app::ResourcesAction::SelectService(ResourceServiceKind::GkeClusters),
        ))
        .await;
    let context = session.app().root.current_context().unwrap().clone();
    session
        .apply_effect_result(EffectResult::GkeClustersLoaded {
            context,
            clusters: Vec::new(),
        })
        .await;
    session
        .dispatch_action(Action::Workspace(WorkspaceAction::Resources))
        .await;

    let backend = TestBackend::new(100, 12);
    let mut terminal = Terminal::new(backend).expect("terminal should initialize");
    terminal
        .draw(|frame| nuvem::ui::render(frame, &session))
        .expect("render should succeed");

    let lines = terminal_lines(&terminal);

    assert!(
        lines
            .iter()
            .any(|line| line.contains("No GKE clusters found for dev"))
    );
}

/// The Cloud Storage object browser is rendered through
/// `render_resource_table` -> `table_tools_line` once a page has loaded
/// (the empty/loading/filtered paths go through `list_tools_line` instead,
/// which already carried the page hint). This proves the populated-table
/// path also surfaces the "Page k (more available)" hint when the loaded
/// page carried a continuation token.
#[tokio::test]
async fn resources_view_renders_storage_object_browser_page_hint_with_more_available() {
    let mut session = RuntimeSession::new_for_test(AppState::new());
    load_active_dev(&mut session).await;
    session
        .dispatch_action(Action::Resources(
            nuvem::app::ResourcesAction::SelectService(ResourceServiceKind::CloudStorage),
        ))
        .await;
    let context = session.app().root.current_context().unwrap().clone();
    session
        .apply_effect_result(EffectResult::StorageBucketsLoaded {
            context: context.clone(),
            buckets: vec![storage_bucket("assets-bucket")],
        })
        .await;
    session
        .dispatch_action(Action::Resources(nuvem::app::ResourcesAction::OpenSelected))
        .await;
    session
        .apply_effect_result(EffectResult::StorageObjectPageLoaded {
            context,
            bucket: "assets-bucket".to_string(),
            prefix: String::new(),
            append: false,
            page: StorageObjectPage {
                prefixes: Vec::new(),
                objects: vec![storage_object("a.txt"), storage_object("b.txt")],
                next_page_token: Some("TOKEN2".to_string()),
            },
        })
        .await;
    session
        .dispatch_action(Action::Workspace(WorkspaceAction::Resources))
        .await;
    // Hide the service sidebar so the table tools line keeps full width.
    session
        .dispatch_action(Action::Navigator(
            nuvem::app::NavigatorAction::ToggleSidebar,
        ))
        .await;

    let lines = render_lines(&session, 99, 14);

    assert!(lines.iter().any(|line| line.contains("a.txt")));
    assert!(
        lines
            .iter()
            .any(|line| line.contains("Page 1") && line.contains("more available")),
        "expected the populated object-browser table's tools line to carry the page hint, got: {lines:?}"
    );
}

/// Once the listing is exhausted (no further continuation token), the same
/// populated-table tools line must report "end of listing" rather than
/// silently dropping the hint.
#[tokio::test]
async fn resources_view_renders_storage_object_browser_end_of_listing_hint() {
    let mut session = RuntimeSession::new_for_test(AppState::new());
    load_active_dev(&mut session).await;
    session
        .dispatch_action(Action::Resources(
            nuvem::app::ResourcesAction::SelectService(ResourceServiceKind::CloudStorage),
        ))
        .await;
    let context = session.app().root.current_context().unwrap().clone();
    session
        .apply_effect_result(EffectResult::StorageBucketsLoaded {
            context: context.clone(),
            buckets: vec![storage_bucket("assets-bucket")],
        })
        .await;
    session
        .dispatch_action(Action::Resources(nuvem::app::ResourcesAction::OpenSelected))
        .await;
    // Page 1 carries a continuation token (pagination is "in play" for this
    // scope), then page 2 (appended) exhausts it -- this is the only case
    // `page_indicator` treats as meaningful once `page_index == 0` isn't
    // also paired with `has_more == false` from the very first page (a
    // dataset that never needed pagination intentionally shows no hint).
    session
        .apply_effect_result(EffectResult::StorageObjectPageLoaded {
            context: context.clone(),
            bucket: "assets-bucket".to_string(),
            prefix: String::new(),
            append: false,
            page: StorageObjectPage {
                prefixes: Vec::new(),
                objects: vec![storage_object("a.txt")],
                next_page_token: Some("TOKEN2".to_string()),
            },
        })
        .await;
    session
        .apply_effect_result(EffectResult::StorageObjectPageLoaded {
            context,
            bucket: "assets-bucket".to_string(),
            prefix: String::new(),
            append: true,
            page: StorageObjectPage {
                prefixes: Vec::new(),
                objects: vec![storage_object("b.txt")],
                next_page_token: None,
            },
        })
        .await;
    session
        .dispatch_action(Action::Workspace(WorkspaceAction::Resources))
        .await;
    // Hide the service sidebar so the table tools line keeps full width.
    session
        .dispatch_action(Action::Navigator(
            nuvem::app::NavigatorAction::ToggleSidebar,
        ))
        .await;

    let lines = render_lines(&session, 99, 14);

    assert!(lines.iter().any(|line| line.contains("a.txt")));
    assert!(
        lines
            .iter()
            .any(|line| line.contains("Page 2") && line.contains("end of listing")),
        "expected the populated object-browser table's tools line to show end-of-listing, got: {lines:?}"
    );
}

/// Applying a safe-prefix filter in the populated object-browser table
/// labels the tools line "(remote)" so it's clear the filter also narrowed
/// the server-side listing, not just the already-loaded rows.
#[tokio::test]
async fn resources_view_labels_storage_object_browser_filter_scope_remote() {
    let mut session = RuntimeSession::new_for_test(AppState::new());
    load_active_dev(&mut session).await;
    session
        .dispatch_action(Action::Resources(
            nuvem::app::ResourcesAction::SelectService(ResourceServiceKind::CloudStorage),
        ))
        .await;
    let context = session.app().root.current_context().unwrap().clone();
    session
        .apply_effect_result(EffectResult::StorageBucketsLoaded {
            context: context.clone(),
            buckets: vec![storage_bucket("assets-bucket")],
        })
        .await;
    session
        .dispatch_action(Action::Resources(nuvem::app::ResourcesAction::OpenSelected))
        .await;
    session
        .apply_effect_result(EffectResult::StorageObjectPageLoaded {
            context,
            bucket: "assets-bucket".to_string(),
            prefix: String::new(),
            append: false,
            page: StorageObjectPage {
                prefixes: Vec::new(),
                objects: vec![
                    storage_object("photos/img1.png"),
                    storage_object("readme.txt"),
                ],
                next_page_token: None,
            },
        })
        .await;
    session
        .dispatch_action(Action::Resources(nuvem::app::ResourcesAction::StartFilter))
        .await;
    for ch in "photos".chars() {
        session
            .dispatch_action(Action::Resources(
                nuvem::app::ResourcesAction::AppendFilterChar(ch),
            ))
            .await;
    }
    session
        .dispatch_action(Action::Resources(nuvem::app::ResourcesAction::ApplyFilter))
        .await;
    session
        .dispatch_action(Action::Workspace(WorkspaceAction::Resources))
        .await;
    // Hide the service sidebar so the table tools line keeps full width.
    session
        .dispatch_action(Action::Navigator(
            nuvem::app::NavigatorAction::ToggleSidebar,
        ))
        .await;

    let lines = render_lines(&session, 99, 14);

    assert!(
        lines
            .iter()
            .any(|line| line.contains("Filter: photos (remote)")),
        "expected the tools line to label the safe-prefix filter as remote, got: {lines:?}"
    );
    // Cloud Storage keeps a plain substring filter, so it renders no mode
    // chip: a chip there would claim a grammar the service does not have.
    assert_eq!(
        storage_tools_line(&lines),
        "Filter: photos (remote)   Sort: Name ▲   Rows: 1/2   ←/→ col · s sort · w wide",
        "the Cloud Storage tools line must stay byte-identical, got: {lines:?}"
    );
}

/// The single applied-filter tools line, trimmed of the render padding, so a
/// tools line can be asserted verbatim rather than by substring. Service-
/// agnostic despite the name: it reads whichever tools line the current
/// workspace rendered.
///
/// Matched on `Sort: ` as well as `Filter: ` so it stays deterministic: the
/// active-input tools line renders `Filter: photos_` with no `Sort: ` segment,
/// and matching on the filter label alone would silently start pinning that
/// line instead of the applied one this helper exists to assert.
fn storage_tools_line(lines: &[String]) -> String {
    lines
        .iter()
        .find(|line| line.contains("Filter: ") && line.contains("Sort: "))
        .unwrap_or_else(|| panic!("expected a tools line, got: {lines:?}"))
        .trim_end()
        .to_string()
}

/// Applying a filter that is syntactically NOT a safe remote prefix (here,
/// one that starts with `/`) in the populated object-browser table labels
/// the tools line "(local)" -- mirroring the "(remote)" case above -- so
/// it's clear the filter only narrowed the already-loaded rows and did not
/// also reload the server-side listing.
#[tokio::test]
async fn resources_view_labels_storage_object_browser_filter_scope_local() {
    let mut session = RuntimeSession::new_for_test(AppState::new());
    load_active_dev(&mut session).await;
    session
        .dispatch_action(Action::Resources(
            nuvem::app::ResourcesAction::SelectService(ResourceServiceKind::CloudStorage),
        ))
        .await;
    let context = session.app().root.current_context().unwrap().clone();
    session
        .apply_effect_result(EffectResult::StorageBucketsLoaded {
            context: context.clone(),
            buckets: vec![storage_bucket("assets-bucket")],
        })
        .await;
    session
        .dispatch_action(Action::Resources(nuvem::app::ResourcesAction::OpenSelected))
        .await;
    session
        .apply_effect_result(EffectResult::StorageObjectPageLoaded {
            context,
            bucket: "assets-bucket".to_string(),
            prefix: String::new(),
            append: false,
            page: StorageObjectPage {
                prefixes: Vec::new(),
                objects: vec![
                    storage_object("photos/img1.png"),
                    storage_object("readme.txt"),
                ],
                next_page_token: None,
            },
        })
        .await;
    session
        .dispatch_action(Action::Resources(nuvem::app::ResourcesAction::StartFilter))
        .await;
    // A leading `/` fails `is_safe_remote_prefix`, so this stays local-only
    // even though it still matches "photos/img1.png" (which contains "/img").
    for ch in "/img".chars() {
        session
            .dispatch_action(Action::Resources(
                nuvem::app::ResourcesAction::AppendFilterChar(ch),
            ))
            .await;
    }
    session
        .dispatch_action(Action::Resources(nuvem::app::ResourcesAction::ApplyFilter))
        .await;
    session
        .dispatch_action(Action::Workspace(WorkspaceAction::Resources))
        .await;
    // Hide the service sidebar so the table tools line keeps full width.
    session
        .dispatch_action(Action::Navigator(
            nuvem::app::NavigatorAction::ToggleSidebar,
        ))
        .await;

    let lines = render_lines(&session, 99, 14);

    assert!(
        lines
            .iter()
            .any(|line| line.contains("Filter: /img (local)")),
        "expected the tools line to label the unsafe-prefix filter as local, got: {lines:?}"
    );
    // As above: no mode chip on the Cloud Storage tools line.
    assert_eq!(
        storage_tools_line(&lines),
        "Filter: /img (local)   Sort: Name ▲   Rows: 1/2   ←/→ col · s sort · w wide",
        "the Cloud Storage tools line must stay byte-identical, got: {lines:?}"
    );
}

// ---- Task 7: distinct empty/error states ----

/// A bucket-list load failure classified as permission-denied must render a
/// distinct "Permission denied" message, not the generic
/// "Resource rows are stale" state that every other failure kind falls
/// back to.
#[tokio::test]
async fn resources_view_renders_permission_denied_message_for_bucket_list_failure() {
    let mut session = RuntimeSession::new_for_test(AppState::new());
    load_active_dev(&mut session).await;
    session
        .dispatch_action(Action::Resources(
            nuvem::app::ResourcesAction::SelectService(ResourceServiceKind::CloudStorage),
        ))
        .await;
    let context = session.app().root.current_context().unwrap().clone();
    session
        .apply_effect_result(EffectResult::StorageBucketsLoadFailed {
            context,
            error: AppError::gcloud(
                "ERROR: (gcloud.storage.buckets.list) PERMISSION_DENIED: The caller does not have permission".to_string(),
            ),
        })
        .await;
    session
        .dispatch_action(Action::Workspace(WorkspaceAction::Resources))
        .await;

    let lines = render_lines(&session, 90, 12);

    assert!(
        lines.iter().any(|line| line.contains("Permission denied")),
        "expected a distinct permission-denied message, got: {lines:?}"
    );
    assert!(
        !lines
            .iter()
            .any(|line| line.contains("Resource rows are stale")),
        "the permission-denied message must replace the generic stale state, got: {lines:?}"
    );
}

/// An object-browser first-page load failure whose stderr indicates a
/// requester-pays condition must render a distinct "Requester pays"
/// message, not the generic permission-denied message.
#[tokio::test]
async fn resources_view_renders_requester_pays_message_for_object_page_failure() {
    let mut session = RuntimeSession::new_for_test(AppState::new());
    load_active_dev(&mut session).await;
    session
        .dispatch_action(Action::Resources(
            nuvem::app::ResourcesAction::SelectService(ResourceServiceKind::CloudStorage),
        ))
        .await;
    let context = session.app().root.current_context().unwrap().clone();
    session
        .apply_effect_result(EffectResult::StorageBucketsLoaded {
            context: context.clone(),
            buckets: vec![storage_bucket("assets-bucket")],
        })
        .await;
    session
        .dispatch_action(Action::Resources(nuvem::app::ResourcesAction::OpenSelected))
        .await;
    session
        .apply_effect_result(EffectResult::StorageObjectPageLoadFailed {
            context,
            bucket: "assets-bucket".to_string(),
            prefix: String::new(),
            append: false,
            error: AppError::gcloud(
                "userProjectMissing: Bucket is a requester pays bucket but no user project was provided.".to_string(),
            ),
        })
        .await;
    session
        .dispatch_action(Action::Workspace(WorkspaceAction::Resources))
        .await;

    let lines = render_lines(&session, 90, 12);

    assert!(
        lines.iter().any(|line| line.contains("Requester pays")),
        "expected a distinct requester-pays message, got: {lines:?}"
    );
    assert!(
        !lines.iter().any(|line| line.contains("Permission denied")),
        "requester-pays must not read as a generic permission-denied failure, got: {lines:?}"
    );
}

/// An empty prefix (a successful listing with zero rows and no filter
/// applied) must render "This prefix is empty" -- distinct from a filter
/// that matches nothing, which is exercised separately below.
#[tokio::test]
async fn resources_view_renders_empty_prefix_message_for_the_object_browser() {
    let mut session = RuntimeSession::new_for_test(AppState::new());
    load_active_dev(&mut session).await;
    session
        .dispatch_action(Action::Resources(
            nuvem::app::ResourcesAction::SelectService(ResourceServiceKind::CloudStorage),
        ))
        .await;
    let context = session.app().root.current_context().unwrap().clone();
    session
        .apply_effect_result(EffectResult::StorageBucketsLoaded {
            context: context.clone(),
            buckets: vec![storage_bucket("empty-bucket")],
        })
        .await;
    session
        .dispatch_action(Action::Resources(nuvem::app::ResourcesAction::OpenSelected))
        .await;
    session
        .apply_effect_result(EffectResult::StorageObjectPageLoaded {
            context,
            bucket: "empty-bucket".to_string(),
            prefix: String::new(),
            append: false,
            page: StorageObjectPage {
                prefixes: Vec::new(),
                objects: Vec::new(),
                next_page_token: None,
            },
        })
        .await;
    session
        .dispatch_action(Action::Workspace(WorkspaceAction::Resources))
        .await;

    let lines = render_lines(&session, 90, 12);

    assert!(
        lines
            .iter()
            .any(|line| line.contains("This prefix is empty")),
        "expected the distinct empty-prefix message, got: {lines:?}"
    );
}

/// An empty bucket list (a successful listing with zero buckets) must
/// render "No buckets found for <config>" -- distinct from the empty-prefix
/// message above, which only applies once a bucket has been opened.
#[tokio::test]
async fn resources_view_renders_empty_bucket_list_message() {
    let mut session = RuntimeSession::new_for_test(AppState::new());
    load_active_dev(&mut session).await;
    session
        .dispatch_action(Action::Resources(
            nuvem::app::ResourcesAction::SelectService(ResourceServiceKind::CloudStorage),
        ))
        .await;
    let context = session.app().root.current_context().unwrap().clone();
    session
        .apply_effect_result(EffectResult::StorageBucketsLoaded {
            context,
            buckets: Vec::new(),
        })
        .await;
    session
        .dispatch_action(Action::Workspace(WorkspaceAction::Resources))
        .await;

    let lines = render_lines(&session, 90, 12);

    assert!(
        lines
            .iter()
            .any(|line| line.contains("No buckets found for")),
        "expected the distinct empty-bucket-list message, got: {lines:?}"
    );
}

/// A filter that matches nothing must render a "No objects match filter"
/// message -- worded distinctly from the empty-prefix message above, so
/// the two states are never confused.
#[tokio::test]
async fn resources_view_renders_no_match_message_for_object_browser_filter() {
    let mut session = RuntimeSession::new_for_test(AppState::new());
    load_active_dev(&mut session).await;
    session
        .dispatch_action(Action::Resources(
            nuvem::app::ResourcesAction::SelectService(ResourceServiceKind::CloudStorage),
        ))
        .await;
    let context = session.app().root.current_context().unwrap().clone();
    session
        .apply_effect_result(EffectResult::StorageBucketsLoaded {
            context: context.clone(),
            buckets: vec![storage_bucket("assets-bucket")],
        })
        .await;
    session
        .dispatch_action(Action::Resources(nuvem::app::ResourcesAction::OpenSelected))
        .await;
    session
        .apply_effect_result(EffectResult::StorageObjectPageLoaded {
            context,
            bucket: "assets-bucket".to_string(),
            prefix: String::new(),
            append: false,
            page: StorageObjectPage {
                prefixes: Vec::new(),
                objects: vec![storage_object("readme.txt")],
                next_page_token: None,
            },
        })
        .await;
    session
        .dispatch_action(Action::Resources(nuvem::app::ResourcesAction::StartFilter))
        .await;
    // A leading `/` fails `is_safe_remote_prefix`, keeping this filter
    // local-only so it never triggers a real (unconfigured-runner) reload
    // effect in this test session -- see the sibling `_local` filter-scope
    // test above for the same trick.
    for ch in "/missing".chars() {
        session
            .dispatch_action(Action::Resources(
                nuvem::app::ResourcesAction::AppendFilterChar(ch),
            ))
            .await;
    }
    session
        .dispatch_action(Action::Resources(nuvem::app::ResourcesAction::ApplyFilter))
        .await;
    session
        .dispatch_action(Action::Workspace(WorkspaceAction::Resources))
        .await;

    let lines = render_lines(&session, 90, 12);

    assert!(
        lines
            .iter()
            .any(|line| line.contains("No objects match filter \"/missing\"")),
        "expected the distinct no-match message, got: {lines:?}"
    );
    assert!(
        !lines
            .iter()
            .any(|line| line.contains("This prefix is empty")),
        "a no-match filter must not read as an empty prefix, got: {lines:?}"
    );
}

/// A page-append (`NextPage`) failure must keep the already-loaded rows
/// visible and show a non-destructive banner explaining the next page
/// couldn't be fetched, rather than replacing the list with an error state.
#[tokio::test]
async fn resources_view_renders_partial_page_failure_banner_while_keeping_loaded_rows() {
    let mut session = RuntimeSession::new_for_test(AppState::new());
    load_active_dev(&mut session).await;
    session
        .dispatch_action(Action::Resources(
            nuvem::app::ResourcesAction::SelectService(ResourceServiceKind::CloudStorage),
        ))
        .await;
    let context = session.app().root.current_context().unwrap().clone();
    session
        .apply_effect_result(EffectResult::StorageBucketsLoaded {
            context: context.clone(),
            buckets: vec![storage_bucket("assets-bucket")],
        })
        .await;
    session
        .dispatch_action(Action::Resources(nuvem::app::ResourcesAction::OpenSelected))
        .await;
    session
        .apply_effect_result(EffectResult::StorageObjectPageLoaded {
            context: context.clone(),
            bucket: "assets-bucket".to_string(),
            prefix: String::new(),
            append: false,
            page: StorageObjectPage {
                prefixes: Vec::new(),
                objects: vec![storage_object("a.txt"), storage_object("b.txt")],
                next_page_token: Some("TOKEN2".to_string()),
            },
        })
        .await;
    session
        .apply_effect_result(EffectResult::StorageObjectPageLoadFailed {
            context,
            bucket: "assets-bucket".to_string(),
            prefix: String::new(),
            append: true,
            error: AppError::gcloud("ERROR: Could not reach the server: timed out".to_string()),
        })
        .await;
    session
        .dispatch_action(Action::Workspace(WorkspaceAction::Resources))
        .await;
    // Hide the service sidebar so the tools/banner lines keep full width.
    session
        .dispatch_action(Action::Navigator(
            nuvem::app::NavigatorAction::ToggleSidebar,
        ))
        .await;

    let lines = render_lines(&session, 99, 14);

    assert!(
        lines.iter().any(|line| line.contains("a.txt")),
        "already-loaded rows must remain visible after an append failure, got: {lines:?}"
    );
    assert!(
        lines.iter().any(|line| line.contains("b.txt")),
        "already-loaded rows must remain visible after an append failure, got: {lines:?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line.contains("Next page failed") && line.contains("retry")),
        "expected a non-destructive next-page-failed banner, got: {lines:?}"
    );
}

/// An object with a retention policy in effect must show an informational
/// "yes" indicator in a RETENTION column -- never styled or treated as an
/// error/problem state.
#[tokio::test]
async fn resources_view_renders_retention_indicator_for_objects_with_retention() {
    let mut session = RuntimeSession::new_for_test(AppState::new());
    load_active_dev(&mut session).await;
    session
        .dispatch_action(Action::Resources(
            nuvem::app::ResourcesAction::SelectService(ResourceServiceKind::CloudStorage),
        ))
        .await;
    let context = session.app().root.current_context().unwrap().clone();
    session
        .apply_effect_result(EffectResult::StorageBucketsLoaded {
            context: context.clone(),
            buckets: vec![storage_bucket("assets-bucket")],
        })
        .await;
    session
        .dispatch_action(Action::Resources(nuvem::app::ResourcesAction::OpenSelected))
        .await;
    session
        .apply_effect_result(EffectResult::StorageObjectPageLoaded {
            context,
            bucket: "assets-bucket".to_string(),
            prefix: String::new(),
            append: false,
            page: StorageObjectPage {
                prefixes: Vec::new(),
                objects: vec![StorageObject {
                    has_retention: true,
                    ..storage_object("legal-hold.txt")
                }],
                next_page_token: None,
            },
        })
        .await;
    session
        .dispatch_action(Action::Workspace(WorkspaceAction::Resources))
        .await;
    session
        .dispatch_action(Action::Navigator(
            nuvem::app::NavigatorAction::ToggleSidebar,
        ))
        .await;

    // Stay below `WIDE_LAYOUT_MIN_WIDTH` (100) so the list keeps the full
    // frame width instead of splitting into list+preview panes, which would
    // otherwise truncate the RETENTION column header.
    let lines = render_lines(&session, 99, 14);

    assert!(
        lines.iter().any(|line| line.contains("RETENTION")),
        "expected a RETENTION column header, got: {lines:?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line.contains("legal-hold.txt") && line.contains("yes")),
        "expected the retention indicator on the object's row, got: {lines:?}"
    );
    assert!(
        !lines
            .iter()
            .any(|line| line.to_lowercase().contains("error")),
        "a retention indicator must never be styled/worded as an error, got: {lines:?}"
    );
}

#[tokio::test]
async fn resources_view_renders_cloud_run_empty_state() {
    let mut session = RuntimeSession::new_for_test(AppState::new());
    load_active_dev(&mut session).await;
    session
        .dispatch_action(Action::Resources(
            nuvem::app::ResourcesAction::SelectService(ResourceServiceKind::CloudRunServices),
        ))
        .await;
    let context = session.app().root.current_context().unwrap().clone();
    session
        .apply_effect_result(EffectResult::CloudRunServicesLoaded {
            context,
            services: Vec::new(),
        })
        .await;
    session
        .dispatch_action(Action::Workspace(WorkspaceAction::Resources))
        .await;

    let lines = render_lines(&session, 100, 12);

    assert!(
        lines
            .iter()
            .any(|line| line.contains("No Cloud Run services found for dev"))
    );
}

#[tokio::test]
async fn resources_view_shows_loading_state_for_cloud_run_on_first_entry() {
    // Selecting Cloud Run and entering Resources for the first time for this
    // context auto-schedules a background reload (`ensure_active_resource_loaded`
    // -> `schedule_active_refresh`), which calls `begin_load` synchronously
    // before this render. So this scenario was never really `NotLoaded` --
    // `workspace_status` reports `Loading`, and the list renders the
    // spinner/elapsed affordance (Task 4) rather than the generic
    // "Press r to load ..." fallback.
    let mut session = RuntimeSession::new_for_test(AppState::new());
    load_active_dev(&mut session).await;
    session
        .dispatch_action(Action::Resources(
            nuvem::app::ResourcesAction::SelectService(ResourceServiceKind::CloudRunServices),
        ))
        .await;
    session
        .dispatch_action(Action::Workspace(WorkspaceAction::Resources))
        .await;

    let lines = render_lines(&session, 100, 12);

    assert!(
        lines
            .iter()
            .any(|line| renders_loading_affordance(line, "Cloud Run services"))
    );
}

#[tokio::test]
async fn cloud_run_detail_view_renders_through_public_render() {
    let mut session = RuntimeSession::new_for_test(AppState::new());
    load_active_dev(&mut session).await;
    session
        .dispatch_action(Action::Resources(
            nuvem::app::ResourcesAction::SelectService(ResourceServiceKind::CloudRunServices),
        ))
        .await;
    let context = session.app().root.current_context().unwrap().clone();
    session
        .apply_effect_result(EffectResult::CloudRunServicesLoaded {
            context: context.clone(),
            services: vec![cloud_run_service("api", "europe-west1")],
        })
        .await;
    session
        .dispatch_action(Action::Workspace(WorkspaceAction::Resources))
        .await;
    session
        .dispatch_action(Action::Resources(nuvem::app::ResourcesAction::OpenSelected))
        .await;
    session
        .apply_effect_result(EffectResult::CloudRunServiceDetailLoaded {
            context,
            detail: cloud_run_detail("api", "europe-west1"),
        })
        .await;
    // Hide the sidebar so the detail pane keeps the full body width for the tabs.
    session
        .dispatch_action(Action::Navigator(
            nuvem::app::NavigatorAction::ToggleSidebar,
        ))
        .await;

    let lines = render_lines(&session, 120, 18);

    assert!(lines.iter().any(|line| line.contains("Cloud Run / api")));
    assert!(lines.iter().any(|line| {
        line.contains("[Summary]") && line.contains("Traffic") && line.contains("Revisions")
    }));
    assert!(
        lines
            .iter()
            .any(|line| line.contains("URL: https://api-abc.a.run.app"))
    );
}

#[tokio::test]
async fn cloud_run_revision_detail_replaces_service_detail() {
    let mut session = RuntimeSession::new_for_test(AppState::new());
    load_active_dev(&mut session).await;
    session
        .dispatch_action(Action::Resources(
            nuvem::app::ResourcesAction::SelectService(ResourceServiceKind::CloudRunServices),
        ))
        .await;
    let context = session.app().root.current_context().unwrap().clone();
    session
        .apply_effect_result(EffectResult::CloudRunServicesLoaded {
            context: context.clone(),
            services: vec![cloud_run_service("api", "europe-west1")],
        })
        .await;
    session
        .dispatch_action(Action::Workspace(WorkspaceAction::Resources))
        .await;
    session
        .dispatch_action(Action::Resources(nuvem::app::ResourcesAction::OpenSelected))
        .await;
    session
        .apply_effect_result(EffectResult::CloudRunServiceDetailLoaded {
            context: context.clone(),
            detail: cloud_run_detail("api", "europe-west1"),
        })
        .await;
    // Navigate to the Revisions tab (Summary -> Traffic -> Revisions).
    session
        .dispatch_action(Action::Resources(
            nuvem::app::ResourcesAction::SelectNextTab,
        ))
        .await;
    session
        .dispatch_action(Action::Resources(
            nuvem::app::ResourcesAction::SelectNextTab,
        ))
        .await;
    // Drill into the selected revision.
    session
        .dispatch_action(Action::Resources(nuvem::app::ResourcesAction::OpenSelected))
        .await;
    session
        .apply_effect_result(EffectResult::CloudRunRevisionDetailLoaded {
            context,
            detail: cloud_run_revision_detail("api-00001", "europe-west1"),
        })
        .await;
    session
        .dispatch_action(Action::Navigator(
            nuvem::app::NavigatorAction::ToggleSidebar,
        ))
        .await;

    assert!(session.app().root.resources.is_showing_cloud_run_revision());

    let lines = render_lines(&session, 120, 18);

    assert!(
        lines
            .iter()
            .any(|line| line.contains("Cloud Run revision / api-00001"))
    );
    assert!(
        lines
            .iter()
            .any(|line| line.contains("Image: gcr.io/x/api:1"))
    );
    // The service-level detail block title must not be showing.
    assert!(
        !lines
            .iter()
            .any(|line| line.contains("Cloud Run service detail"))
    );
}

#[tokio::test]
async fn switching_away_from_cloud_run_revision_renders_new_service_not_stale_revision() {
    // Regression test: drilling into a Cloud Run revision and then switching
    // to a different service via the sidebar (not Esc) must not leave the
    // stale Cloud Run revision detail bleeding into the new service's pane.
    let mut session = RuntimeSession::new_for_test(AppState::new());
    load_active_dev(&mut session).await;
    session
        .dispatch_action(Action::Resources(
            nuvem::app::ResourcesAction::SelectService(ResourceServiceKind::CloudRunServices),
        ))
        .await;
    let context = session.app().root.current_context().unwrap().clone();
    session
        .apply_effect_result(EffectResult::CloudRunServicesLoaded {
            context: context.clone(),
            services: vec![cloud_run_service("api", "europe-west1")],
        })
        .await;
    session
        .dispatch_action(Action::Workspace(WorkspaceAction::Resources))
        .await;
    session
        .dispatch_action(Action::Resources(nuvem::app::ResourcesAction::OpenSelected))
        .await;
    session
        .apply_effect_result(EffectResult::CloudRunServiceDetailLoaded {
            context: context.clone(),
            detail: cloud_run_detail("api", "europe-west1"),
        })
        .await;
    // Navigate to the Revisions tab (Summary -> Traffic -> Revisions).
    session
        .dispatch_action(Action::Resources(
            nuvem::app::ResourcesAction::SelectNextTab,
        ))
        .await;
    session
        .dispatch_action(Action::Resources(
            nuvem::app::ResourcesAction::SelectNextTab,
        ))
        .await;
    // Drill into the selected revision.
    session
        .dispatch_action(Action::Resources(nuvem::app::ResourcesAction::OpenSelected))
        .await;
    session
        .apply_effect_result(EffectResult::CloudRunRevisionDetailLoaded {
            context: context.clone(),
            detail: cloud_run_revision_detail("api-00001", "europe-west1"),
        })
        .await;
    assert!(session.app().root.resources.is_showing_cloud_run_revision());

    // Switch to GKE via the sidebar (not Esc) and drill into a cluster.
    session
        .dispatch_action(Action::Resources(
            nuvem::app::ResourcesAction::SelectService(ResourceServiceKind::GkeClusters),
        ))
        .await;
    // The state fix must clear the stranded revision view immediately.
    assert!(!session.app().root.resources.is_showing_cloud_run_revision());
    session
        .apply_effect_result(EffectResult::GkeClustersLoaded {
            context: context.clone(),
            clusters: vec![gke_cluster("platform-dev", "europe-west1")],
        })
        .await;
    session
        .dispatch_action(Action::Resources(nuvem::app::ResourcesAction::OpenSelected))
        .await;
    session
        .apply_effect_result(EffectResult::GkeClusterDetailLoaded {
            context,
            detail: gke_detail("platform-dev", "europe-west1"),
        })
        .await;

    let lines = render_lines(&session, 120, 18);

    assert!(
        lines
            .iter()
            .any(|line| line.contains("GKE cluster / platform-dev"))
    );
    // The stale Cloud Run revision view must not bleed into the GKE pane.
    assert!(
        !lines
            .iter()
            .any(|line| line.contains("Cloud Run revision / api-00001"))
    );
}

#[tokio::test]
async fn resources_view_renders_gce_list_safely_at_minimum_frame_size() {
    let session = session_with_loaded_instances().await;

    let lines = render_lines(&session, 60, 12);

    assert!(lines.iter().any(|line| line.contains("Configuration: dev")));
    assert!(
        lines
            .iter()
            .any(|line| line.contains("Project: dev-project"))
    );
    assert!(lines.iter().any(|line| line.contains("[OK]")));
    // The footer's first hint, in priority order: at 60 columns the rest are
    // dropped whole rather than cut.
    assert!(lines.iter().any(|line| line.contains("Enter Open")));
    // At this minimum frame size the NAME column shrinks to its min width,
    // truncating "api-dev-1" to "api-d…"; check the surviving prefix.
    assert!(lines.iter().any(|line| line.contains("api-d")));
}

#[tokio::test]
async fn resources_view_scrolls_selected_gce_row_at_minimum_frame_size() {
    let mut session = RuntimeSession::new_for_test(AppState::new());
    load_active_dev(&mut session).await;
    let context = session.app().root.current_context().unwrap().clone();
    session
        .apply_effect_result(EffectResult::ComputeInstancesLoaded {
            context,
            instances: (0..4)
                .map(|index| ComputeInstance {
                    id: format!("instance-{index}"),
                    name: format!("instance-{index}"),
                    zone: "europe-west1-b".to_string(),
                    status: ResourceStatus::Running,
                    machine_type: "e2-medium".to_string(),
                    internal_ip: Some(format!("10.0.0.{}", index + 1)),
                    external_ip: None,
                })
                .collect(),
        })
        .await;
    session
        .dispatch_action(Action::Workspace(WorkspaceAction::Resources))
        .await;
    for _ in 0..3 {
        session
            .dispatch_action(Action::Resources(nuvem::app::ResourcesAction::SelectNext))
            .await;
    }

    let lines = render_lines(&session, 60, 12);

    // At this minimum frame size the NAME column shrinks to its min width,
    // truncating "instance-3" to "insta…"; check the surviving prefix.
    assert!(
        lines
            .iter()
            .any(|line| line.contains(&format!("{SELECTED_UNMARKED_ROW_PREFIX}insta")))
    );
    assert!(!lines.iter().any(|line| line.contains("> instance-0")));
}

#[tokio::test]
async fn resources_view_scrolls_selected_gke_row_at_minimum_frame_size() {
    let mut session = RuntimeSession::new_for_test(AppState::new());
    load_active_dev(&mut session).await;
    session
        .dispatch_action(Action::Resources(
            nuvem::app::ResourcesAction::SelectService(ResourceServiceKind::GkeClusters),
        ))
        .await;
    let context = session.app().root.current_context().unwrap().clone();
    session
        .apply_effect_result(EffectResult::GkeClustersLoaded {
            context,
            clusters: (0..4)
                .map(|index| gke_cluster(&format!("cluster-{index}"), "europe-west1"))
                .collect(),
        })
        .await;
    session
        .dispatch_action(Action::Workspace(WorkspaceAction::Resources))
        .await;
    for _ in 0..3 {
        session
            .dispatch_action(Action::Resources(nuvem::app::ResourcesAction::SelectNext))
            .await;
    }

    let lines = render_lines(&session, 60, 12);

    assert!(
        lines
            .iter()
            .any(|line| line.contains(&format!("{SELECTED_ROW_PREFIX_NO_MARK}cluster-3")))
    );
    assert!(!lines.iter().any(|line| line.contains("> cluster-0")));
}

#[tokio::test]
async fn resources_view_shows_loading_state_after_context_change() {
    let mut session = RuntimeSession::new_for_test(AppState::new());
    session
        .apply_effect_result(EffectResult::ConfigurationsLoaded {
            configurations: vec![GcloudConfiguration {
                name: GcloudConfigurationName::new("dev"),
                is_active: true,
                account: Some("dev@example.com".to_string()),
                project: Some("dev-project".to_string()),
                region: None,
                zone: None,
            }],
        })
        .await;
    session
        .dispatch_action(Action::Workspace(WorkspaceAction::Resources))
        .await;
    session
        .apply_effect_result(EffectResult::ConfigurationActivated {
            configuration: GcloudConfiguration {
                name: GcloudConfigurationName::new("prod"),
                is_active: true,
                account: Some("prod@example.com".to_string()),
                project: Some("prod-project".to_string()),
                region: None,
                zone: None,
            },
        })
        .await;

    let backend = TestBackend::new(90, 12);
    let mut terminal = Terminal::new(backend).expect("terminal should initialize");
    terminal
        .draw(|frame| nuvem::ui::render(frame, &session))
        .expect("render should succeed");

    let lines = terminal_lines(&terminal);

    // Entering Resources on the newly-activated (never-loaded) prod context
    // auto-schedules a background reload, which calls `begin_load`
    // synchronously before this render (see `ensure_active_resource_loaded`
    // -> `schedule_active_refresh`). `workspace_status` now reports
    // `Loading` here, not `Stale` (Loading takes precedence over
    // the generic stale prompt while a refresh is in flight), and the list
    // renders the spinner/elapsed affordance (Task 4).
    assert!(
        !lines
            .iter()
            .any(|line| line.contains("Resource rows are stale. Press r to refresh."))
    );
    assert!(
        lines
            .iter()
            .any(|line| renders_loading_affordance(line, "GCE instances"))
    );
}

#[tokio::test]
async fn resources_view_shows_action_refresh_status() {
    let mut session = session_with_loaded_instances().await;
    let context = session.app().root.current_context().unwrap().clone();
    session
        .apply_effect_result(EffectResult::ComputeInstanceActionSucceeded {
            context,
            instance_name: "api-dev-1".to_string(),
            instance_zone: "europe-west1-b".to_string(),
            action: nuvem::app::state::resources::ComputeInstanceAction::Start,
            // No operations-log assertion here: this renders the status line.
            operation_id: UNTRACKED_OPERATION_ID,
        })
        .await;

    let lines = render_lines(&session, 120, 12);

    assert!(
        lines
            .iter()
            .any(|line| line.contains("Start accepted for api-dev-1; refreshing GCE instances"))
    );
    assert!(!lines.iter().any(|line| {
        line.contains("Status: Start accepted for api-dev-1; refreshing GCE instances")
    }));
    // The follow-up refresh effect calls `begin_load` synchronously (see
    // `dispatch_background_refresh`), so `workspace_status` now reports
    // `Loading` here, not `Stale` (Loading takes precedence over the
    // generic stale prompt while a refresh is in flight), and the list renders
    // the spinner/elapsed affordance (Task 4).
    assert!(
        !lines
            .iter()
            .any(|line| line.contains("Resource rows are stale. Press r to refresh."))
    );
    assert!(
        lines
            .iter()
            .any(|line| renders_loading_affordance(line, "GCE instances"))
    );
}

#[tokio::test]
async fn resources_view_shows_failed_state_and_status_message_after_resource_refresh_failure() {
    let mut session = RuntimeSession::new_for_test(AppState::new());
    session
        .apply_effect_result(EffectResult::ConfigurationsLoaded {
            configurations: vec![GcloudConfiguration {
                name: GcloudConfigurationName::new("dev"),
                is_active: true,
                account: Some("dev@example.com".to_string()),
                project: Some("dev-project".to_string()),
                region: None,
                zone: None,
            }],
        })
        .await;
    let context = session.app().root.current_context().unwrap().clone();
    session
        .apply_effect_result(EffectResult::ComputeInstancesLoaded {
            context,
            instances: vec![
                ComputeInstance {
                    id: "1001".to_string(),
                    name: "api-dev-1".to_string(),
                    zone: "europe-west1-b".to_string(),
                    status: ResourceStatus::Running,
                    machine_type: "e2-medium".to_string(),
                    internal_ip: Some("10.0.0.4".to_string()),
                    external_ip: Some("34.10.10.10".to_string()),
                },
                ComputeInstance {
                    id: "1002".to_string(),
                    name: "worker-dev-1".to_string(),
                    zone: "europe-west1-c".to_string(),
                    status: ResourceStatus::Stopped,
                    machine_type: "e2-small".to_string(),
                    internal_ip: Some("10.0.0.5".to_string()),
                    external_ip: None,
                },
            ],
        })
        .await;
    session
        .apply_effect_result(EffectResult::ComputeInstancesLoadFailed {
            context: session.app().root.current_context().unwrap().clone(),
            error: nuvem::app::AppError::gcloud("compute API disabled"),
        })
        .await;
    session
        .dispatch_action(Action::Workspace(WorkspaceAction::Resources))
        .await;
    // Entering Resources while stale now auto-retriggers a reload (Task 4); since this
    // session has no gcloud runner configured, that retry immediately fails again with a
    // different (but still Gcloud-flavored) error. Drain it before rendering.
    assert!(session.process_next_effect_result().await);

    let backend = TestBackend::new(120, 12);
    let mut terminal = Terminal::new(backend).expect("terminal should initialize");
    terminal
        .draw(|frame| nuvem::ui::render(frame, &session))
        .expect("render should succeed");

    let lines = terminal_lines(&terminal);

    assert!(
        lines
            .iter()
            .any(|line| line.contains("unknown error — Retry, or copy the details to investigate"))
    );
    assert!(
        !lines
            .iter()
            .any(|line| line.contains("no gcloud command runner configured"))
    );
    assert!(!lines.iter().any(|line| {
        line.contains("Status: unknown error — Retry, or copy the details to investigate")
    }));
    // The drained retry ends in a `Failed` load phase (not `Stale`):
    // `fail_load` sets it directly, and `workspace_status` now reports
    // `Failed` for a failure with no fresh rows to fall back on.
    // The list renders the failure-state affordance: the `load_error`
    // message (here the same sanitized headline already asserted above)
    // plus a "Press r to retry." item (Task 4).
    assert!(
        !lines
            .iter()
            .any(|line| line.contains("Resource rows are stale. Press r to refresh."))
    );
    assert!(
        lines
            .iter()
            .any(|line| line.contains("unknown error — Retry, or copy the details to investigate"))
    );
    assert!(lines.iter().any(|line| line.contains("Press r to retry.")));
    assert!(!lines.iter().any(|line| line.contains("api-dev-1")));
    assert!(!lines.iter().any(|line| line.contains("worker-dev-1")));
}

#[tokio::test]
async fn resources_view_shows_loading_state_for_storage_after_context_change() {
    // Second, cheap Loading-state check (compute is covered above): the
    // Cloud Storage list renderer must render the same spinner/elapsed
    // affordance as every other service (Task 4), using its own "buckets"
    // noun rather than a generic service label.
    //
    // Landing on a configuration now opens *that* configuration's remembered
    // service, so prod is seeded to remember Cloud Storage. Without
    // the seed the activation below would land on prod's default service and
    // this would be a compute test wearing a storage test's name.
    let mut app = AppState::new();
    app.root.remembered_views.set(
        GcloudConfigurationName::new("prod"),
        ResourceServiceKind::CloudStorage,
    );
    let mut session = RuntimeSession::new_for_test(app);
    load_active_dev(&mut session).await;
    session
        .dispatch_action(Action::Resources(
            nuvem::app::ResourcesAction::SelectService(ResourceServiceKind::CloudStorage),
        ))
        .await;
    session
        .dispatch_action(Action::Workspace(WorkspaceAction::Resources))
        .await;
    session
        .apply_effect_result(EffectResult::ConfigurationActivated {
            configuration: GcloudConfiguration {
                name: GcloudConfigurationName::new("prod"),
                is_active: true,
                account: Some("prod@example.com".to_string()),
                project: Some("prod-project".to_string()),
                region: None,
                zone: None,
            },
        })
        .await;

    let lines = render_lines(&session, 90, 12);

    assert!(
        !lines
            .iter()
            .any(|line| line.contains("Resource rows are stale. Press r to refresh."))
    );
    assert!(
        lines
            .iter()
            .any(|line| renders_loading_affordance(line, "buckets"))
    );
}

#[tokio::test]
async fn resources_view_shows_ip_columns_for_loaded_rows() {
    let mut session = RuntimeSession::new_for_test(AppState::new());
    session
        .apply_effect_result(EffectResult::ConfigurationsLoaded {
            configurations: vec![GcloudConfiguration {
                name: GcloudConfigurationName::new("dev"),
                is_active: true,
                account: Some("dev@example.com".to_string()),
                project: Some("dev-project".to_string()),
                region: None,
                zone: None,
            }],
        })
        .await;
    let context = session.app().root.current_context().unwrap().clone();
    session
        .apply_effect_result(EffectResult::ComputeInstancesLoaded {
            context,
            instances: vec![
                ComputeInstance {
                    id: "1001".to_string(),
                    name: "api-dev-1".to_string(),
                    zone: "europe-west1-b".to_string(),
                    status: ResourceStatus::Running,
                    machine_type: "e2-medium".to_string(),
                    internal_ip: Some("10.0.0.4".to_string()),
                    external_ip: Some("34.10.10.10".to_string()),
                },
                ComputeInstance {
                    id: "1002".to_string(),
                    name: "worker-dev-1".to_string(),
                    zone: "europe-west1-c".to_string(),
                    status: ResourceStatus::Stopped,
                    machine_type: "e2-small".to_string(),
                    internal_ip: Some("10.0.0.5".to_string()),
                    external_ip: None,
                },
            ],
        })
        .await;
    session
        .dispatch_action(Action::Workspace(WorkspaceAction::Resources))
        .await;
    // Hide the service sidebar so the resource table occupies the full width;
    // otherwise the sidebar narrows the content pane and truncates these rows.
    session
        .dispatch_action(Action::Navigator(
            nuvem::app::NavigatorAction::ToggleSidebar,
        ))
        .await;

    // Body width kept below WIDE_LAYOUT_MIN_WIDTH so the list column stays
    // full-screen; a wide split would truncate these long rows.
    let backend = TestBackend::new(90, 12);
    let mut terminal = Terminal::new(backend).expect("terminal should initialize");
    terminal
        .draw(|frame| nuvem::ui::render(frame, &session))
        .expect("render should succeed");

    let lines = terminal_lines(&terminal);

    assert!(lines.iter().any(|line| line.contains("api-dev-1")
        && line.contains("10.0.0.4")
        && line.contains("34.10.10.10")));
    assert!(lines.iter().any(|line| line.contains("worker-dev-1")
        && line.contains("10.0.0.5")
        && line.contains(" - ")));
}

#[tokio::test]
async fn resources_view_marks_selected_instance_row() {
    let mut session = RuntimeSession::new_for_test(AppState::new());
    load_dev_instances(&mut session).await;
    session
        .dispatch_action(Action::Workspace(WorkspaceAction::Resources))
        .await;
    session
        .dispatch_action(Action::Resources(nuvem::app::ResourcesAction::SelectNext))
        .await;
    // Hide the service sidebar so the table keeps enough width for the marker.
    session
        .dispatch_action(Action::Navigator(
            nuvem::app::NavigatorAction::ToggleSidebar,
        ))
        .await;

    let backend = TestBackend::new(120, 12);
    let mut terminal = Terminal::new(backend).expect("terminal should initialize");
    terminal
        .draw(|frame| nuvem::ui::render(frame, &session))
        .expect("render should succeed");

    let lines = terminal_lines(&terminal);
    assert!(
        lines
            .iter()
            .any(|line| line.contains(&format!("{SELECTED_UNMARKED_ROW_PREFIX}worker-dev-1")))
    );
}

#[tokio::test]
async fn resources_view_uses_shared_selected_row_style() {
    let mut session = RuntimeSession::new_for_test(AppState::new());
    load_dev_instances(&mut session).await;
    session
        .dispatch_action(Action::Workspace(WorkspaceAction::Resources))
        .await;
    // Hide the service sidebar so the resource table occupies the full width,
    // matching the layout this shared selected-row-style assertion targets.
    session
        .dispatch_action(Action::Navigator(
            nuvem::app::NavigatorAction::ToggleSidebar,
        ))
        .await;

    let backend = TestBackend::new(80, 24);
    let mut terminal = Terminal::new(backend).expect("terminal should initialize");
    terminal
        .draw(|frame| nuvem::ui::render(frame, &session))
        .expect("render should succeed");

    assert_eq!(
        terminal.backend().buffer()[(1, 6)].style().bg,
        Some(Color::Blue)
    );
}

#[test]
fn resources_render_reads_service_values_from_projection() {
    let source = include_str!("../src/ui/resources/mod.rs");
    assert!(source.contains("let projection = project_resources(&session.app().root.resources);"));
    assert!(source.contains("projection.selected_visible"));
    for accessor in [
        "resources.visible_instances()",
        "resources.visible_gke_clusters()",
        "resources.instances()",
        "resources.gke_clusters()",
        "resources.filter_applied()",
        "resources.filter_draft()",
        "resources.sort_label()",
        "resources.selected_visible_index()",
        "resources.detail()",
        "resources.gke_detail()",
        "resources.detail_error()",
        "resources.gke_detail_error()",
    ] {
        assert!(
            !source.contains(accessor),
            "renderer still calls {accessor}"
        );
    }
}

#[tokio::test]
async fn resources_view_marks_default_selected_visible_row_after_name_sort() {
    let mut session = RuntimeSession::new_for_test(AppState::new());
    session
        .apply_effect_result(EffectResult::ConfigurationsLoaded {
            configurations: vec![GcloudConfiguration {
                name: GcloudConfigurationName::new("dev"),
                is_active: true,
                account: Some("dev@example.com".to_string()),
                project: Some("dev-project".to_string()),
                region: None,
                zone: None,
            }],
        })
        .await;
    let context = session.app().root.current_context().unwrap().clone();
    session
        .apply_effect_result(EffectResult::ComputeInstancesLoaded {
            context,
            instances: vec![
                ComputeInstance {
                    id: "1002".to_string(),
                    name: "worker-dev-1".to_string(),
                    zone: "europe-west1-c".to_string(),
                    status: ResourceStatus::Stopped,
                    machine_type: "e2-small".to_string(),
                    internal_ip: Some("10.0.0.5".to_string()),
                    external_ip: None,
                },
                ComputeInstance {
                    id: "1001".to_string(),
                    name: "api-dev-1".to_string(),
                    zone: "europe-west1-b".to_string(),
                    status: ResourceStatus::Running,
                    machine_type: "e2-medium".to_string(),
                    internal_ip: Some("10.0.0.4".to_string()),
                    external_ip: Some("34.10.10.10".to_string()),
                },
            ],
        })
        .await;
    session
        .dispatch_action(Action::Workspace(WorkspaceAction::Resources))
        .await;

    let backend = TestBackend::new(120, 12);
    let mut terminal = Terminal::new(backend).expect("terminal should initialize");
    terminal
        .draw(|frame| nuvem::ui::render(frame, &session))
        .expect("render should succeed");

    let lines = terminal_lines(&terminal);
    // The split preview pane narrows the table enough that the NAME column
    // truncates "api-dev-1" to "api-de…"; check the surviving prefix.
    assert!(
        lines
            .iter()
            .any(|line| line.contains(&format!("{SELECTED_UNMARKED_ROW_PREFIX}api-de")))
    );
    assert!(!lines.iter().any(|line| line.contains("> worker-dev-1")));
}

#[tokio::test]
async fn resources_view_shows_filter_sort_status_and_filtered_rows() {
    let mut session = RuntimeSession::new_for_test(AppState::new());
    load_dev_instances(&mut session).await;
    session
        .dispatch_action(Action::Workspace(WorkspaceAction::Resources))
        .await;
    session
        .dispatch_action(Action::Resources(nuvem::app::ResourcesAction::StartFilter))
        .await;
    for ch in "worker".chars() {
        session
            .dispatch_action(Action::Resources(
                nuvem::app::ResourcesAction::AppendFilterChar(ch),
            ))
            .await;
    }
    session
        .dispatch_action(Action::Resources(nuvem::app::ResourcesAction::ApplyFilter))
        .await;
    session
        .dispatch_action(Action::Resources(
            nuvem::app::ResourcesAction::SelectNextColumn,
        ))
        .await;
    session
        .dispatch_action(Action::Resources(
            nuvem::app::ResourcesAction::SortBySelectedColumn,
        ))
        .await;
    // Hide the service sidebar so the table tools line and rows keep full width.
    session
        .dispatch_action(Action::Navigator(
            nuvem::app::NavigatorAction::ToggleSidebar,
        ))
        .await;

    let backend = TestBackend::new(120, 14);
    let mut terminal = Terminal::new(backend).expect("terminal should initialize");
    terminal
        .draw(|frame| nuvem::ui::render(frame, &session))
        .expect("render should succeed");

    let lines = terminal_lines(&terminal);

    assert!(lines.iter().any(|line| line.contains("Filter: worker")
        && line.contains("Sort: Status")
        && line.contains("Rows: 1/2")));
    assert!(
        lines
            .iter()
            .any(|line| line.contains(&format!("{SELECTED_UNMARKED_ROW_PREFIX}worker-dev-1")))
    );
    assert!(!lines.iter().any(|line| line.contains("api-dev-1")));
}

#[tokio::test]
async fn resources_view_shows_active_filter_input_line() {
    let mut session = RuntimeSession::new_for_test(AppState::new());
    load_dev_instances(&mut session).await;
    session
        .dispatch_action(Action::Workspace(WorkspaceAction::Resources))
        .await;
    session
        .dispatch_action(Action::Resources(nuvem::app::ResourcesAction::StartFilter))
        .await;
    for ch in "api".chars() {
        session
            .dispatch_action(Action::Resources(
                nuvem::app::ResourcesAction::AppendFilterChar(ch),
            ))
            .await;
    }

    let backend = TestBackend::new(120, 14);
    let mut terminal = Terminal::new(backend).expect("terminal should initialize");
    terminal
        .draw(|frame| nuvem::ui::render(frame, &session))
        .expect("render should succeed");

    let lines = terminal_lines(&terminal);

    assert!(lines.iter().any(|line| line.contains("Filter: api_")));
}

/// The mode chip is what makes the prefix grammar discoverable, so it has to
/// track the draft live: the moment `-f` is typed the chip reads `[fuzzy]`,
/// with nothing applied yet.
#[tokio::test]
async fn resources_view_shows_mode_chip_while_the_filter_is_being_typed() {
    let mut session = RuntimeSession::new_for_test(AppState::new());
    load_dev_instances(&mut session).await;
    session
        .dispatch_action(Action::Workspace(WorkspaceAction::Resources))
        .await;
    session
        .dispatch_action(Action::Resources(nuvem::app::ResourcesAction::StartFilter))
        .await;
    for ch in "-f ap".chars() {
        session
            .dispatch_action(Action::Resources(
                nuvem::app::ResourcesAction::AppendFilterChar(ch),
            ))
            .await;
    }

    let lines = render_lines(&session, 120, 14);

    assert!(
        lines
            .iter()
            .any(|line| line.contains("Filter: -f ap_  [fuzzy]")),
        "expected the live mode chip next to the filter draft, got: {lines:?}"
    );
}

/// An uncompilable pattern never drops rows; it reports itself inline next to
/// the applied filter, after the chip.
#[tokio::test]
async fn resources_view_shows_inline_filter_error_for_an_invalid_regex() {
    let mut session = RuntimeSession::new_for_test(AppState::new());
    load_dev_instances(&mut session).await;
    session
        .dispatch_action(Action::Workspace(WorkspaceAction::Resources))
        .await;
    session
        .dispatch_action(Action::Resources(nuvem::app::ResourcesAction::StartFilter))
        .await;
    for ch in "a(b".chars() {
        session
            .dispatch_action(Action::Resources(
                nuvem::app::ResourcesAction::AppendFilterChar(ch),
            ))
            .await;
    }
    session
        .dispatch_action(Action::Resources(nuvem::app::ResourcesAction::ApplyFilter))
        .await;
    // Hide the service sidebar so the table tools line keeps full width.
    session
        .dispatch_action(Action::Navigator(
            nuvem::app::NavigatorAction::ToggleSidebar,
        ))
        .await;

    let lines = render_lines(&session, 120, 14);

    assert!(
        lines
            .iter()
            .any(|line| line.contains("Filter: a(b  [regex]  ⚠ invalid regex: unclosed group")),
        "expected the inline filter error after the chip, got: {lines:?}"
    );
}

/// Types `text` into the resource filter input and applies it.
async fn apply_resource_filter(session: &mut RuntimeSession, text: &str) {
    session
        .dispatch_action(Action::Resources(nuvem::app::ResourcesAction::StartFilter))
        .await;
    for ch in text.chars() {
        session
            .dispatch_action(Action::Resources(
                nuvem::app::ResourcesAction::AppendFilterChar(ch),
            ))
            .await;
    }
    session
        .dispatch_action(Action::Resources(nuvem::app::ResourcesAction::ApplyFilter))
        .await;
}

/// `filter.error` is always about the *applied* text while the chip tracks the
/// *draft*, so the moment the draft diverges the `⚠` would describe a filter
/// the user is no longer looking at -- and, for a fuzzy draft, would claim a
/// regex failure fuzzy mode can never produce. The first keystroke of a fix
/// clears it.
#[tokio::test]
async fn resources_view_clears_the_stale_filter_error_once_the_draft_diverges() {
    let mut session = RuntimeSession::new_for_test(AppState::new());
    load_dev_instances(&mut session).await;
    session
        .dispatch_action(Action::Workspace(WorkspaceAction::Resources))
        .await;
    // Hide the service sidebar so the table tools line keeps full width.
    session
        .dispatch_action(Action::Navigator(
            nuvem::app::NavigatorAction::ToggleSidebar,
        ))
        .await;
    apply_resource_filter(&mut session, "a(b").await;
    // Reopen the input and retype the filter as a fuzzy one.
    session
        .dispatch_action(Action::Resources(nuvem::app::ResourcesAction::StartFilter))
        .await;
    for _ in 0..3 {
        session
            .dispatch_action(Action::Resources(
                nuvem::app::ResourcesAction::BackspaceFilter,
            ))
            .await;
    }
    for ch in "-f ap".chars() {
        session
            .dispatch_action(Action::Resources(
                nuvem::app::ResourcesAction::AppendFilterChar(ch),
            ))
            .await;
    }

    let lines = render_lines(&session, 120, 14);

    assert!(
        lines
            .iter()
            .any(|line| line.contains("Filter: -f ap_  [fuzzy]")),
        "expected the chip to track the new draft, got: {lines:?}"
    );
    assert!(
        !lines.iter().any(|line| line.contains("⚠")),
        "expected the applied-filter error to be gone once the draft diverged, got: {lines:?}"
    );
}

/// The other half of the rule: reopening `/` seeds the draft from the applied
/// text, so nothing has diverged yet and the failure is still worth explaining.
#[tokio::test]
async fn resources_view_keeps_the_filter_error_when_the_filter_is_reopened_unedited() {
    let mut session = RuntimeSession::new_for_test(AppState::new());
    load_dev_instances(&mut session).await;
    session
        .dispatch_action(Action::Workspace(WorkspaceAction::Resources))
        .await;
    // Hide the service sidebar so the table tools line keeps full width.
    session
        .dispatch_action(Action::Navigator(
            nuvem::app::NavigatorAction::ToggleSidebar,
        ))
        .await;
    apply_resource_filter(&mut session, "a(b").await;
    session
        .dispatch_action(Action::Resources(nuvem::app::ResourcesAction::StartFilter))
        .await;

    let lines = render_lines(&session, 120, 14);

    assert!(
        lines
            .iter()
            .any(|line| line.contains("Filter: a(b_  [regex]  ⚠ invalid regex: unclosed group")),
        "expected the unedited draft to keep explaining the applied failure, got: {lines:?}"
    );
}

/// Divergence has to be measured on the *trimmed* text, because both sides of
/// the comparison are already trimmed elsewhere: `ResourcesState::apply_filter`
/// stores the trimmed draft as the applied filter, and `parse_filter` trims
/// again before dispatching. `a(b ` and `a(b` are therefore the same filter and
/// produce the byte-identical error, so a stray trailing space must not make
/// the `⚠` vanish.
#[tokio::test]
async fn resources_view_keeps_the_filter_error_when_the_draft_only_gains_whitespace() {
    let mut session = RuntimeSession::new_for_test(AppState::new());
    load_dev_instances(&mut session).await;
    session
        .dispatch_action(Action::Workspace(WorkspaceAction::Resources))
        .await;
    // Hide the service sidebar so the table tools line keeps full width.
    session
        .dispatch_action(Action::Navigator(
            nuvem::app::NavigatorAction::ToggleSidebar,
        ))
        .await;
    apply_resource_filter(&mut session, "a(b").await;
    session
        .dispatch_action(Action::Resources(nuvem::app::ResourcesAction::StartFilter))
        .await;
    session
        .dispatch_action(Action::Resources(
            nuvem::app::ResourcesAction::AppendFilterChar(' '),
        ))
        .await;

    let lines = render_lines(&session, 120, 14);

    assert!(
        lines
            .iter()
            .any(|line| line.contains("Filter: a(b _  [regex]  ⚠ invalid regex: unclosed group")),
        "expected a trailing space to leave the applied failure explained, got: {lines:?}"
    );
}

/// A filter that matches nothing replaces the table with the empty state, so
/// the chip has to come from the *list* tools line -- exactly the state where
/// naming the mode matters most, since the user is looking at zero rows.
#[tokio::test]
async fn resources_view_shows_the_mode_chip_on_the_no_match_tools_line() {
    let mut session = RuntimeSession::new_for_test(AppState::new());
    load_dev_instances(&mut session).await;
    session
        .dispatch_action(Action::Workspace(WorkspaceAction::Resources))
        .await;
    // Hide the service sidebar so the list tools line keeps full width.
    session
        .dispatch_action(Action::Navigator(
            nuvem::app::NavigatorAction::ToggleSidebar,
        ))
        .await;
    apply_resource_filter(&mut session, "zzzz").await;

    // 99 columns is below `WIDE_LAYOUT_MIN_WIDTH`, so no preview pane shares
    // the tools row and the line can be asserted whole.
    let lines = render_lines(&session, 99, 14);

    // Asserted whole-line, not by substring: a chip appended at the *end* of
    // the tools line instead of right after the filter text would satisfy a
    // `contains` while rendering something nobody designed.
    assert_eq!(
        storage_tools_line(&lines),
        "Filter: zzzz  [regex]   Sort: Name   Rows: 0/2",
        "expected the chip on the no-match tools line, got: {lines:?}"
    );

    // Reopening the input renders the same line through its active branch.
    session
        .dispatch_action(Action::Resources(nuvem::app::ResourcesAction::StartFilter))
        .await;
    let lines = render_lines(&session, 99, 14);

    assert!(
        lines
            .iter()
            .any(|line| line.contains("Filter: zzzz_  [regex]")),
        "expected the chip on the active no-match tools line, got: {lines:?}"
    );
    // `table_tools_line`'s active branch emits that same string, so the line
    // above alone cannot prove the empty-state path was the one rendered.
    // The column/sort hints only ever come from the table line, so their
    // absence is what pins it.
    assert!(
        !lines.iter().any(|line| line.contains("←/→ col · s sort")),
        "expected the no-match empty state, not the table tools line, got: {lines:?}"
    );
}

/// An empty applied filter has no mode to name: `Filter: none` stays bare
/// rather than advertising the default `[regex]` nobody asked for.
#[tokio::test]
async fn resources_view_renders_an_unfiltered_tools_line_without_a_chip() {
    let mut session = RuntimeSession::new_for_test(AppState::new());
    load_dev_instances(&mut session).await;
    session
        .dispatch_action(Action::Workspace(WorkspaceAction::Resources))
        .await;
    // Hide the service sidebar so the table tools line keeps full width.
    session
        .dispatch_action(Action::Navigator(
            nuvem::app::NavigatorAction::ToggleSidebar,
        ))
        .await;

    // 99 columns is below `WIDE_LAYOUT_MIN_WIDTH`, so no preview pane shares
    // the tools row and the line can be asserted whole.
    let lines = render_lines(&session, 99, 14);

    // Whole-line, for the same reason as the no-match case above: a chip
    // appended after the key hints instead of after the filter text would slip
    // past a `contains` on the filter segment alone.
    assert_eq!(
        storage_tools_line(&lines),
        "Filter: none   Sort: Name ▲   Rows: 2/2   ←/→ col · s sort · w wide",
        "expected a bare `Filter: none` with no chip, got: {lines:?}"
    );
}

/// The deliberate other half of that rule: the *draft* line has no
/// empty-filter guard, so opening `/` on an unfiltered table still renders a
/// chip. Idle and typing make different claims — `Filter: none` describes a
/// filter that isn't there, while an open input is a promise about the text
/// you are about to type — and the chip is what makes the grammar
/// discoverable before the first keystroke.
#[tokio::test]
async fn resources_view_shows_the_mode_chip_on_an_empty_filter_draft() {
    let mut session = RuntimeSession::new_for_test(AppState::new());
    load_dev_instances(&mut session).await;
    session
        .dispatch_action(Action::Workspace(WorkspaceAction::Resources))
        .await;
    // Hide the service sidebar so the table tools line keeps full width.
    session
        .dispatch_action(Action::Navigator(
            nuvem::app::NavigatorAction::ToggleSidebar,
        ))
        .await;
    session
        .dispatch_action(Action::Resources(nuvem::app::ResourcesAction::StartFilter))
        .await;

    let lines = render_lines(&session, 120, 14);

    assert!(
        lines.iter().any(|line| line.contains("Filter: _  [regex]")),
        "expected the empty draft to still name its mode, got: {lines:?}"
    );
}

#[tokio::test]
async fn resources_view_shows_no_match_message_for_applied_filter() {
    let mut session = RuntimeSession::new_for_test(AppState::new());
    load_dev_instances(&mut session).await;
    session
        .dispatch_action(Action::Workspace(WorkspaceAction::Resources))
        .await;
    session
        .dispatch_action(Action::Resources(nuvem::app::ResourcesAction::StartFilter))
        .await;
    for ch in "missing".chars() {
        session
            .dispatch_action(Action::Resources(
                nuvem::app::ResourcesAction::AppendFilterChar(ch),
            ))
            .await;
    }
    session
        .dispatch_action(Action::Resources(nuvem::app::ResourcesAction::ApplyFilter))
        .await;

    let backend = TestBackend::new(120, 14);
    let mut terminal = Terminal::new(backend).expect("terminal should initialize");
    terminal
        .draw(|frame| nuvem::ui::render(frame, &session))
        .expect("render should succeed");

    let lines = terminal_lines(&terminal);

    assert!(
        lines
            .iter()
            .any(|line| line.contains("No GCE instances match filter \"missing\""))
    );
    assert!(lines.iter().any(|line| line.contains("Rows: 0/2")));
    assert!(!lines.iter().any(|line| line.contains("api-dev-1")));
    assert!(!lines.iter().any(|line| line.contains("worker-dev-1")));
}

#[tokio::test]
async fn resources_detail_summary_tab_renders_selected_instance_fields() {
    let mut session = RuntimeSession::new_for_test(AppState::new());
    load_dev_instances(&mut session).await;
    session
        .dispatch_action(Action::Resources(nuvem::app::ResourcesAction::OpenSelected))
        .await;
    let context = session.app().root.current_context().unwrap().clone();
    session
        .apply_effect_result(EffectResult::ComputeInstanceDetailLoaded {
            context,
            detail: detail("api-dev-1"),
        })
        .await;
    session
        .dispatch_action(Action::Workspace(WorkspaceAction::Resources))
        .await;

    let lines = render_lines(&session, 80, 24);
    assert_shared_frame(&lines, "Resources", "[ Previous tab");
    assert!(lines.iter().any(|line| line.contains("Configuration: dev")));
    assert!(
        lines
            .iter()
            .any(|line| line.contains("Home > Resources > GCE instances"))
    );
    assert!(
        lines
            .iter()
            .any(|line| line.contains("GCE instance / api-dev-1"))
    );
    assert!(lines.iter().any(|line| line.contains("Summary")));
    assert!(
        lines
            .iter()
            .any(|line| line.contains("[ and ] tabs   n/p field   y copy   Esc/q return to list"))
    );
    assert!(!lines.iter().any(|line| line.contains("[ / ] tabs")));
    assert!(
        lines
            .iter()
            .any(|line| line.contains("Machine type: e2-medium"))
    );
    assert!(
        lines
            .iter()
            .any(|line| line.contains("Creation: 2026-07-03T10:11:12.000-07:00"))
    );
}

#[tokio::test]
async fn resources_detail_summary_shows_content_at_minimum_frame_size() {
    let mut session = RuntimeSession::new_for_test(AppState::new());
    load_dev_instances(&mut session).await;
    session
        .dispatch_action(Action::Resources(nuvem::app::ResourcesAction::OpenSelected))
        .await;
    let context = session.app().root.current_context().unwrap().clone();
    session
        .apply_effect_result(EffectResult::ComputeInstanceDetailLoaded {
            context,
            detail: detail("api-dev-1"),
        })
        .await;
    session
        .dispatch_action(Action::Workspace(WorkspaceAction::Resources))
        .await;

    let lines = render_lines(&session, 60, 12);

    assert!(lines.iter().any(|line| line.contains("Name: api-dev-1")));
    assert!(lines.iter().any(|line| line.contains("Status: Running")));
}

#[tokio::test]
async fn resources_detail_summary_pages_hidden_fields_at_minimum_frame_size() {
    let mut session = RuntimeSession::new_for_test(AppState::new());
    load_dev_instances(&mut session).await;
    session
        .dispatch_action(Action::Resources(nuvem::app::ResourcesAction::OpenSelected))
        .await;
    let context = session.app().root.current_context().unwrap().clone();
    session
        .apply_effect_result(EffectResult::ComputeInstanceDetailLoaded {
            context,
            detail: detail("api-dev-1"),
        })
        .await;
    session
        .dispatch_action(Action::Workspace(WorkspaceAction::Resources))
        .await;

    let initial_lines = render_lines(&session, 60, 12);
    assert!(
        !initial_lines
            .iter()
            .any(|line| line.contains("Creation: 2026-07-03T10:11:12.000-07:00"))
    );

    session
        .dispatch_action(Action::Resources(
            nuvem::app::ResourcesAction::ScrollDetailDown,
        ))
        .await;
    let scrolled_lines = render_lines(&session, 60, 12);

    assert!(
        scrolled_lines
            .iter()
            .any(|line| line.contains("Machine type: e2-medium"))
    );
    assert!(
        scrolled_lines
            .iter()
            .any(|line| line.contains("Creation: 2026-07-03T10:11:12.000-07:00"))
    );
}

#[test]
fn filtered_context_picker_footer_matches_filter_keymap() {
    let mut app = app_with_open_context_picker(0);
    reduce(
        &mut app,
        &Action::Configurations(ConfigurationsAction::StartContextPickerFilter),
    );

    let projection = project_app_frame(&app.root);

    assert_eq!(
        projection.footer_hints,
        vec![
            FrameHint {
                key: "Backspace".to_string(),
                label: "Delete char",
            },
            FrameHint {
                key: "Enter".to_string(),
                label: "Apply",
            },
            FrameHint {
                key: "Esc".to_string(),
                label: "Cancel",
            },
        ]
    );
}

#[test]
fn context_picker_footer_hides_selection_actions_without_an_available_item() {
    let loading_projection = project_app_frame(&app_with_open_context_picker(0).root);
    assert_eq!(
        loading_projection.footer_hints,
        vec![
            FrameHint {
                key: "/".to_string(),
                label: "Filter",
            },
            FrameHint {
                key: "r".to_string(),
                label: "Retry discovery",
            },
            FrameHint {
                key: "Esc".to_string(),
                label: "Cancel",
            },
        ]
    );

    let mut empty_app = app_with_open_context_picker(0);
    load_picker_items(
        &mut empty_app,
        ConfigurationContextPickerKind::Project,
        Vec::new(),
    );
    let empty_projection = project_app_frame(&empty_app.root);
    assert_eq!(
        empty_projection.footer_hints,
        loading_projection.footer_hints
    );

    let mut degraded_app = app_with_open_context_picker(0);
    reduce(
        &mut degraded_app,
        &Action::EffectResult(EffectResult::ContextPickerLoadFailed {
            configuration: GcloudConfigurationName::new("dev"),
            kind: ConfigurationContextPickerKind::Project,
            error: AppError::gcloud("permission denied"),
        }),
    );
    let degraded_projection = project_app_frame(&degraded_app.root);
    assert_eq!(
        degraded_projection.footer_hints,
        loading_projection.footer_hints
    );
}

#[test]
fn context_picker_footer_hides_selection_action_when_filter_has_no_matches() {
    let mut app = app_with_open_context_picker(0);
    load_picker_items(
        &mut app,
        ConfigurationContextPickerKind::Project,
        vec![ConfigurationContextPickerItem::Project(GcloudProject {
            project_id: "dev-project".to_string(),
            name: "Development".to_string(),
            lifecycle_state: Some("ACTIVE".to_string()),
        })],
    );
    for action in [
        ConfigurationsAction::StartContextPickerFilter,
        ConfigurationsAction::UpdateContextPickerFilter("missing".to_string()),
        ConfigurationsAction::ApplyContextPickerFilter,
    ] {
        reduce(&mut app, &Action::Configurations(action));
    }

    let projection = project_app_frame(&app.root);

    assert_eq!(
        projection.footer_hints,
        vec![
            FrameHint {
                key: "/".to_string(),
                label: "Filter",
            },
            FrameHint {
                key: "r".to_string(),
                label: "Retry discovery",
            },
            FrameHint {
                key: "Esc".to_string(),
                label: "Cancel",
            },
        ]
    );
}

#[tokio::test]
async fn resources_detail_loading_uses_shared_loading_style() {
    let loaded_session = session_with_loaded_instances().await;
    let mut app = loaded_session.app().clone();
    reduce(
        &mut app,
        &Action::Resources(nuvem::app::ResourcesAction::OpenSelected),
    );
    let session = RuntimeSession::new_for_test(app);

    let backend = TestBackend::new(80, 24);
    let mut terminal = Terminal::new(backend).expect("terminal should initialize");
    terminal
        .draw(|frame| nuvem::ui::render(frame, &session))
        .expect("render should succeed");

    let buffer = terminal.backend().buffer();
    let loading_row = buffer
        .content()
        .chunks(80)
        .find(|row| {
            row.iter()
                .map(|cell| cell.symbol())
                .collect::<String>()
                .contains("Loading detail")
        })
        .expect("loading detail line should render");
    let loading_cell = loading_row
        .iter()
        .find(|cell| cell.symbol() == "L")
        .expect("loading detail marker should render");
    assert_eq!(loading_cell.style().fg, Some(Color::Yellow));
}

#[tokio::test]
async fn resources_detail_tabs_render_network_disks_metadata_and_json() {
    let mut session = RuntimeSession::new_for_test(AppState::new());
    load_dev_instances(&mut session).await;
    session
        .dispatch_action(Action::Resources(nuvem::app::ResourcesAction::OpenSelected))
        .await;
    let context = session.app().root.current_context().unwrap().clone();
    session
        .apply_effect_result(EffectResult::ComputeInstanceDetailLoaded {
            context,
            detail: detail("api-dev-1"),
        })
        .await;
    session
        .dispatch_action(Action::Workspace(WorkspaceAction::Resources))
        .await;

    let backend = TestBackend::new(120, 18);
    let mut terminal = Terminal::new(backend).expect("terminal should initialize");

    session
        .dispatch_action(Action::Resources(
            nuvem::app::ResourcesAction::SelectNextTab,
        ))
        .await;
    terminal
        .draw(|frame| nuvem::ui::render(frame, &session))
        .expect("render should succeed");
    let lines = terminal_lines(&terminal);
    assert!(lines.iter().any(|line| line.contains("nic0")));
    assert!(lines.iter().any(|line| line.contains("34.10.10.10")));

    session
        .dispatch_action(Action::Resources(
            nuvem::app::ResourcesAction::SelectNextTab,
        ))
        .await;
    terminal
        .draw(|frame| nuvem::ui::render(frame, &session))
        .expect("render should succeed");
    let lines = terminal_lines(&terminal);
    assert!(lines.iter().any(|line| line.contains("PERSISTENT")));
    assert!(lines.iter().any(|line| line.contains("20")));

    session
        .dispatch_action(Action::Resources(
            nuvem::app::ResourcesAction::SelectNextTab,
        ))
        .await;
    terminal
        .draw(|frame| nuvem::ui::render(frame, &session))
        .expect("render should succeed");
    let lines = terminal_lines(&terminal);
    assert!(lines.iter().any(|line| line.contains("env: dev")));
    assert!(lines.iter().any(|line| line.contains("owner: platform")));

    session
        .dispatch_action(Action::Resources(
            nuvem::app::ResourcesAction::SelectNextTab,
        ))
        .await;
    terminal
        .draw(|frame| nuvem::ui::render(frame, &session))
        .expect("render should succeed");
    let lines = terminal_lines(&terminal);
    assert!(
        lines
            .iter()
            .any(|line| line.contains("\"name\": \"api-dev-1\""))
    );
}

#[tokio::test]
async fn resources_detail_failure_renders_error_without_old_payload() {
    let mut session = RuntimeSession::new_for_test(AppState::new());
    load_dev_instances(&mut session).await;
    session
        .dispatch_action(Action::Resources(nuvem::app::ResourcesAction::OpenSelected))
        .await;
    let context = session.app().root.current_context().unwrap().clone();
    session
        .apply_effect_result(EffectResult::ComputeInstanceDetailLoaded {
            context: context.clone(),
            detail: detail("api-dev-1"),
        })
        .await;
    session
        .apply_effect_result(EffectResult::ComputeInstanceDetailLoadFailed {
            context,
            instance_name: "api-dev-1".to_string(),
            instance_zone: "europe-west1-b".to_string(),
            error: nuvem::app::AppError::gcloud("describe failed"),
        })
        .await;
    session
        .dispatch_action(Action::Workspace(WorkspaceAction::Resources))
        .await;

    let backend = TestBackend::new(120, 18);
    let mut terminal = Terminal::new(backend).expect("terminal should initialize");
    terminal
        .draw(|frame| nuvem::ui::render(frame, &session))
        .expect("render should succeed");

    let lines = terminal_lines(&terminal);
    assert!(
        lines
            .iter()
            .any(|line| line.contains("unknown error — Retry, or copy the details to investigate"))
    );
    assert!(!lines.iter().any(|line| line.contains("describe failed")));
    assert!(
        !lines
            .iter()
            .any(|line| line.contains("Machine type: e2-medium"))
    );
}

#[tokio::test]
async fn gke_detail_view_renders_through_public_render() {
    let mut session = RuntimeSession::new_for_test(AppState::new());
    load_active_dev(&mut session).await;
    session
        .dispatch_action(Action::Resources(
            nuvem::app::ResourcesAction::SelectService(ResourceServiceKind::GkeClusters),
        ))
        .await;
    let context = session.app().root.current_context().unwrap().clone();
    session
        .apply_effect_result(EffectResult::GkeClustersLoaded {
            context: context.clone(),
            clusters: vec![gke_cluster("platform-dev", "europe-west1")],
        })
        .await;
    session
        .dispatch_action(Action::Workspace(WorkspaceAction::Resources))
        .await;
    session
        .dispatch_action(Action::Resources(nuvem::app::ResourcesAction::OpenSelected))
        .await;
    session
        .apply_effect_result(EffectResult::GkeClusterDetailLoaded {
            context,
            detail: gke_detail("platform-dev", "europe-west1"),
        })
        .await;

    let lines = render_lines(&session, 80, 24);

    assert_shared_frame(&lines, "Resources", "[ Previous tab");
    assert!(lines.iter().any(|line| line.contains("Configuration: dev")));
    assert!(
        lines
            .iter()
            .any(|line| line.contains("Home > Resources > GKE clusters"))
    );
    assert!(
        lines
            .iter()
            .any(|line| line.contains("GKE cluster / platform-dev"))
    );
    assert!(lines.iter().any(|line| {
        line.contains("[Summary]")
            && line.contains("Networking")
            && line.contains("Node Pools")
            && line.contains("Addons")
            && line.contains("JSON")
    }));
    assert!(
        lines
            .iter()
            .any(|line| line.contains("Master version: 1.31.5-gke.1023000"))
    );
}

#[tokio::test]
async fn gke_detail_view_renders_summary_and_json_tabs() {
    let mut session = RuntimeSession::new_for_test(AppState::new());
    load_active_dev(&mut session).await;
    session
        .dispatch_action(Action::Resources(
            nuvem::app::ResourcesAction::SelectService(ResourceServiceKind::GkeClusters),
        ))
        .await;
    let context = session.app().root.current_context().unwrap().clone();
    session
        .apply_effect_result(EffectResult::GkeClustersLoaded {
            context: context.clone(),
            clusters: vec![gke_cluster("platform-dev", "europe-west1")],
        })
        .await;
    session
        .dispatch_action(Action::Workspace(WorkspaceAction::Resources))
        .await;
    session
        .dispatch_action(Action::Resources(nuvem::app::ResourcesAction::OpenSelected))
        .await;
    session
        .apply_effect_result(EffectResult::GkeClusterDetailLoaded {
            context,
            detail: gke_detail("platform-dev", "europe-west1"),
        })
        .await;
    // Hide the service sidebar so the detail pane keeps the full body width;
    // otherwise the sidebar + wide split narrows it and clips the tab strip.
    session
        .dispatch_action(Action::Navigator(
            nuvem::app::NavigatorAction::ToggleSidebar,
        ))
        .await;

    let backend = TestBackend::new(120, 18);
    let mut terminal = Terminal::new(backend).expect("terminal should initialize");
    terminal
        .draw(|frame| nuvem::ui::render(frame, &session))
        .expect("render should succeed");

    let lines = terminal_lines(&terminal);

    assert!(
        lines
            .iter()
            .any(|line| line.contains("GKE cluster / platform-dev"))
    );
    assert!(lines.iter().any(|line| line.contains("[Summary]")));
    assert!(
        lines
            .iter()
            .any(|line| line.contains("Master version: 1.31.5-gke.1023000"))
    );

    session
        .dispatch_action(Action::Resources(
            nuvem::app::ResourcesAction::SelectPreviousTab,
        ))
        .await;

    let backend = TestBackend::new(120, 18);
    let mut terminal = Terminal::new(backend).expect("terminal should initialize");
    terminal
        .draw(|frame| nuvem::ui::render(frame, &session))
        .expect("render should succeed");
    let lines = terminal_lines(&terminal);

    assert!(lines.iter().any(|line| line.contains("[JSON]")));
    assert!(
        lines
            .iter()
            .any(|line| line.contains("\"name\": \"platform-dev\""))
    );
}

#[tokio::test]
async fn gke_detail_loading_uses_shared_loading_style() {
    let mut loaded_session = RuntimeSession::new_for_test(AppState::new());
    load_active_dev(&mut loaded_session).await;
    loaded_session
        .dispatch_action(Action::Resources(
            nuvem::app::ResourcesAction::SelectService(ResourceServiceKind::GkeClusters),
        ))
        .await;
    let context = loaded_session.app().root.current_context().unwrap().clone();
    loaded_session
        .apply_effect_result(EffectResult::GkeClustersLoaded {
            context,
            clusters: vec![gke_cluster("platform-dev", "europe-west1")],
        })
        .await;
    loaded_session
        .dispatch_action(Action::Workspace(WorkspaceAction::Resources))
        .await;
    let mut app = loaded_session.app().clone();
    reduce(
        &mut app,
        &Action::Resources(nuvem::app::ResourcesAction::OpenSelected),
    );
    let session = RuntimeSession::new_for_test(app);

    let backend = TestBackend::new(80, 24);
    let mut terminal = Terminal::new(backend).expect("terminal should initialize");
    terminal
        .draw(|frame| nuvem::ui::render(frame, &session))
        .expect("render should succeed");

    let buffer = terminal.backend().buffer();
    let loading_row = buffer
        .content()
        .chunks(80)
        .find(|row| {
            row.iter()
                .map(|cell| cell.symbol())
                .collect::<String>()
                .contains("Loading detail")
        })
        .expect("loading detail line should render");
    let loading_cell = loading_row
        .iter()
        .find(|cell| cell.symbol() == "L")
        .expect("loading detail marker should render");
    assert_eq!(loading_cell.style().fg, Some(Color::Yellow));
}

#[tokio::test]
async fn resources_detail_content_is_fitted_to_the_body_width() {
    let mut session = RuntimeSession::new_for_test(AppState::new());
    load_dev_instances(&mut session).await;
    session
        .dispatch_action(Action::Resources(nuvem::app::ResourcesAction::OpenSelected))
        .await;
    let context = session.app().root.current_context().unwrap().clone();
    let mut instance_detail = detail("api-dev-1");
    instance_detail.raw_json =
        "{\n\t\"name\": \"api-dev-1\", \"description\": \"a very long readable JSON value that must be fitted\"\n}"
            .to_string();
    session
        .apply_effect_result(EffectResult::ComputeInstanceDetailLoaded {
            context,
            detail: instance_detail,
        })
        .await;
    session
        .dispatch_action(Action::Workspace(WorkspaceAction::Resources))
        .await;
    session
        .dispatch_action(Action::Resources(
            nuvem::app::ResourcesAction::SelectPreviousTab,
        ))
        .await;

    let lines = render_lines(&session, 60, 14);

    assert!(
        lines
            .iter()
            .any(|line| line.contains("\"name\": \"api-dev-1\""))
    );
    assert!(lines.iter().any(|line| line.contains("...")));
}

#[tokio::test]
async fn gke_detail_failure_renders_error_without_loading_forever() {
    let mut session = RuntimeSession::new_for_test(AppState::new());
    load_active_dev(&mut session).await;
    session
        .dispatch_action(Action::Resources(
            nuvem::app::ResourcesAction::SelectService(ResourceServiceKind::GkeClusters),
        ))
        .await;
    let context = session.app().root.current_context().unwrap().clone();
    session
        .apply_effect_result(EffectResult::GkeClustersLoaded {
            context: context.clone(),
            clusters: vec![gke_cluster("platform-dev", "europe-west1")],
        })
        .await;
    session
        .dispatch_action(Action::Workspace(WorkspaceAction::Resources))
        .await;
    session
        .dispatch_action(Action::Resources(nuvem::app::ResourcesAction::OpenSelected))
        .await;
    session
        .apply_effect_result(EffectResult::GkeClusterDetailLoadFailed {
            context,
            cluster_name: "platform-dev".to_string(),
            cluster_location: "europe-west1".to_string(),
            error: nuvem::app::AppError::gcloud("describe failed"),
        })
        .await;

    // Body width kept below WIDE_LAYOUT_MIN_WIDTH so the detail pane keeps
    // the full body width; a wide split would truncate this error line.
    let backend = TestBackend::new(90, 18);
    let mut terminal = Terminal::new(backend).expect("terminal should initialize");
    terminal
        .draw(|frame| nuvem::ui::render(frame, &session))
        .expect("render should succeed");

    let lines = terminal_lines(&terminal);

    assert!(
        lines
            .iter()
            .any(|line| line.contains("Detail load failed for platform-dev"))
    );
    assert!(
        lines
            .iter()
            .any(|line| line.contains("unknown error — Retry, or copy the details to investigate"))
    );
    assert!(!lines.iter().any(|line| line.contains("describe failed")));
    assert!(
        lines.iter().any(
            |line| line.contains("Press r to refresh the list or Esc/q to return to the list.")
        )
    );
    assert!(
        !lines
            .iter()
            .any(|line| line.contains("Loading detail for platform-dev..."))
    );
}

#[tokio::test]
async fn command_palette_renders_navigation_entries_with_aliases() {
    let mut session = RuntimeSession::new_for_test(AppState::new());
    session
        .apply_effect_result(EffectResult::ConfigurationsLoaded {
            configurations: vec![GcloudConfiguration {
                name: GcloudConfigurationName::new("dev"),
                is_active: true,
                account: Some("dev@example.com".to_string()),
                project: Some("dev-project".to_string()),
                region: Some("europe-west1".to_string()),
                zone: Some("europe-west1-b".to_string()),
            }],
        })
        .await;
    session
        .dispatch_action(Action::Command(CommandAction::Open))
        .await;

    let lines = render_lines(&session, 100, 24);

    assert!(lines.iter().any(|line| line.contains("Home")), "{lines:#?}");
    assert!(
        lines
            .iter()
            .any(|line| line.contains("GKE clusters  (gke, kubernetes, clusters)")),
        "{lines:#?}"
    );
}

#[tokio::test]
async fn command_palette_renders_empty_state_with_query_and_hint() {
    let mut session = RuntimeSession::new_for_test(AppState::new());
    session
        .dispatch_action(Action::Command(CommandAction::Open))
        .await;
    for ch in "zzz".chars() {
        session
            .dispatch_action(Action::Command(CommandAction::AppendChar(ch)))
            .await;
    }

    let lines = render_lines(&session, 100, 24);

    assert!(
        lines
            .iter()
            .any(|line| line.contains("No commands match \"zzz\"")),
        "{lines:#?}"
    );
    assert!(
        lines.iter().any(|line| line.contains("Type to search")),
        "{lines:#?}"
    );
}

#[tokio::test]
async fn command_palette_renders_running_instance_mutations_and_selected_row() {
    let mut session = session_with_loaded_instances().await;
    session
        .dispatch_action(Action::Command(nuvem::app::CommandAction::Open))
        .await;

    let backend = TestBackend::new(100, 20);
    let mut terminal = Terminal::new(backend).expect("terminal should initialize");
    terminal
        .draw(|frame| nuvem::ui::render(frame, &session))
        .expect("render should succeed");

    let lines = terminal_lines(&terminal);
    assert!(lines.iter().any(|line| line.contains("commands")));
    assert!(
        lines
            .iter()
            .any(|line| line.contains("> Stop selected GCE instance"))
    );
    assert!(
        lines
            .iter()
            .any(|line| line.contains("  Reset selected GCE instance"))
    );
    assert!(
        lines
            .iter()
            .any(|line| line.contains("  Suspend selected GCE instance"))
    );
}

#[tokio::test]
async fn command_palette_filter_renders_no_match_state() {
    let mut session = session_with_loaded_instances().await;
    session
        .dispatch_action(Action::Command(nuvem::app::CommandAction::Open))
        .await;
    for ch in "zzz".chars() {
        session
            .dispatch_action(Action::Command(nuvem::app::CommandAction::AppendChar(ch)))
            .await;
    }

    let backend = TestBackend::new(100, 20);
    let mut terminal = Terminal::new(backend).expect("terminal should initialize");
    terminal
        .draw(|frame| nuvem::ui::render(frame, &session))
        .expect("render should succeed");

    let lines = terminal_lines(&terminal);
    assert!(lines.iter().any(|line| line.contains("No commands match")));
}

#[tokio::test]
async fn configuration_activation_confirmation_renders_target_context_and_command_preview() {
    let mut session = RuntimeSession::new_for_test(AppState::new());
    session
        .apply_effect_result(EffectResult::ConfigurationsLoaded {
            configurations: vec![
                GcloudConfiguration {
                    name: GcloudConfigurationName::new("dev"),
                    is_active: true,
                    account: Some("dev@example.com".to_string()),
                    project: Some("dev-project".to_string()),
                    region: Some("europe-west1".to_string()),
                    zone: Some("europe-west1-b".to_string()),
                },
                GcloudConfiguration {
                    name: GcloudConfigurationName::new("prod"),
                    is_active: false,
                    account: Some("prod@example.com".to_string()),
                    project: Some("prod-project".to_string()),
                    region: Some("us-central1".to_string()),
                    zone: Some("us-central1-a".to_string()),
                },
            ],
        })
        .await;
    session
        .dispatch_action(Action::Workspace(WorkspaceAction::Configurations))
        .await;
    session
        .dispatch_action(Action::Configurations(ConfigurationsAction::SelectNext))
        .await;
    session
        .dispatch_action(Action::Configurations(
            ConfigurationsAction::RequestActivateSelected,
        ))
        .await;

    let backend = TestBackend::new(120, 20);
    let mut terminal = Terminal::new(backend).expect("terminal should initialize");
    terminal
        .draw(|frame| nuvem::ui::render(frame, &session))
        .expect("render should succeed");

    let lines = terminal_lines(&terminal);
    for expected in [
        "confirm configuration activation",
        "Target: prod",
        "Account: prod@example.com",
        "Project: prod-project",
        "Region: us-central1",
        "Zone: us-central1-a",
        "gcloud config configurations activate prod",
        "Enter/y confirm   Esc/n cancel",
    ] {
        assert!(
            lines.iter().any(|line| line.contains(expected)),
            "expected confirmation line containing {expected:?}"
        );
    }
}

#[tokio::test]
async fn configuration_activation_confirmation_renders_small_terminal_fallback_when_modal_cannot_fit()
 {
    let mut session = RuntimeSession::new_for_test(AppState::new());
    session
        .apply_effect_result(EffectResult::ConfigurationsLoaded {
            configurations: vec![
                GcloudConfiguration {
                    name: GcloudConfigurationName::new("dev"),
                    is_active: true,
                    account: Some("dev@example.com".to_string()),
                    project: Some("dev-project".to_string()),
                    region: Some("europe-west1".to_string()),
                    zone: Some("europe-west1-b".to_string()),
                },
                GcloudConfiguration {
                    name: GcloudConfigurationName::new("prod"),
                    is_active: false,
                    account: Some("prod@example.com".to_string()),
                    project: Some("prod-project".to_string()),
                    region: Some("us-central1".to_string()),
                    zone: Some("us-central1-a".to_string()),
                },
            ],
        })
        .await;
    session
        .dispatch_action(Action::Workspace(WorkspaceAction::Configurations))
        .await;
    session
        .dispatch_action(Action::Configurations(ConfigurationsAction::SelectNext))
        .await;
    session
        .dispatch_action(Action::Configurations(
            ConfigurationsAction::RequestActivateSelected,
        ))
        .await;

    let backend = TestBackend::new(60, 12);
    let mut terminal = Terminal::new(backend).expect("terminal should initialize");
    terminal
        .draw(|frame| nuvem::ui::render(frame, &session))
        .expect("render should succeed");

    let lines = terminal_lines(&terminal);
    assert!(
        lines
            .iter()
            .any(|line| { line.contains("nuvem activation confirmation needs a larger terminal") })
    );
    assert!(!lines.iter().any(|line| line.contains("Target: prod")));
}

#[test]
fn configuration_confirmation_takes_precedence_over_ssh_preview_for_rendering_and_input() {
    let mut app = AppState::new();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ConfigurationsLoaded {
            configurations: vec![
                GcloudConfiguration {
                    name: GcloudConfigurationName::new("dev"),
                    is_active: true,
                    account: Some("dev@example.com".to_string()),
                    project: Some("dev-project".to_string()),
                    region: Some("europe-west1".to_string()),
                    zone: Some("europe-west1-b".to_string()),
                },
                GcloudConfiguration {
                    name: GcloudConfigurationName::new("prod"),
                    is_active: false,
                    account: Some("prod@example.com".to_string()),
                    project: Some("prod-project".to_string()),
                    region: Some("us-central1".to_string()),
                    zone: Some("us-central1-a".to_string()),
                },
            ],
        }),
    );
    reduce(&mut app, &Action::Workspace(WorkspaceAction::Resources));
    let context = app.root.current_context().expect("active context exists");
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
                internal_ip: Some("10.0.0.2".to_string()),
                external_ip: None,
            }],
        }),
    );
    for action in [
        Action::Command(nuvem::app::CommandAction::Open),
        Action::Command(nuvem::app::CommandAction::SelectNext),
        Action::Command(nuvem::app::CommandAction::SelectNext),
        Action::Command(nuvem::app::CommandAction::SelectNext),
        Action::Command(nuvem::app::CommandAction::Submit),
    ] {
        reduce(&mut app, &action);
    }
    reduce(
        &mut app,
        &Action::Workspace(WorkspaceAction::Configurations),
    );
    reduce(
        &mut app,
        &Action::Configurations(ConfigurationsAction::SelectNext),
    );
    reduce(
        &mut app,
        &Action::Configurations(ConfigurationsAction::RequestActivateSelected),
    );

    assert!(app.root.resources.command_preview().is_some());
    assert!(app.root.configurations.activation_confirmation().is_some());

    let session = RuntimeSession::new_for_test(app);
    let lines = render_lines(&session, 120, 20);
    assert!(
        lines
            .iter()
            .any(|line| line.contains("confirm configuration activation"))
    );
    assert!(lines.iter().any(|line| line.contains("Target: prod")));
    assert!(!lines.iter().any(|line| line.contains("Show SSH command")));

    let context = nuvem::services::runtime::input_context_for_test(
        &session,
        ratatui::layout::Rect::new(0, 0, 120, 20),
    );
    assert_eq!(context.mode, Mode::ConfirmConfigurationActivation);
    assert_eq!(
        action_for_key(
            KeyEvent::new(KeyCode::Enter, KeyModifiers::empty()),
            &context
        ),
        Some(Action::Configurations(
            ConfigurationsAction::ConfirmActivation
        ))
    );
    assert_eq!(
        action_for_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::empty()), &context),
        Some(Action::Configurations(
            ConfigurationsAction::CancelActivation
        ))
    );
}

#[tokio::test]
async fn resource_action_confirmation_renders_target_context_and_command_preview() {
    let mut session = session_with_loaded_instances().await;
    session
        .dispatch_action(Action::Command(nuvem::app::CommandAction::Open))
        .await;
    session
        .dispatch_action(Action::Command(nuvem::app::CommandAction::Submit))
        .await;

    let backend = TestBackend::new(120, 20);
    let mut terminal = Terminal::new(backend).expect("terminal should initialize");
    terminal
        .draw(|frame| nuvem::ui::render(frame, &session))
        .expect("render should succeed");

    let lines = terminal_lines(&terminal);
    assert!(lines.iter().any(|line| line.contains("Confirm GCE action")));
    assert!(
        lines
            .iter()
            .any(|line| line.contains("Action: Stop instance"))
    );
    assert!(
        lines
            .iter()
            .any(|line| line.contains("Instance: api-dev-1"))
    );
    assert!(
        lines
            .iter()
            .any(|line| line.contains("Zone: europe-west1-b"))
    );
    assert!(lines.iter().any(|line| line.contains("Configuration: dev")));
    assert!(
        lines
            .iter()
            .any(|line| line.contains("Project: dev-project"))
    );
    assert!(
        lines
            .iter()
            .any(|line| line.contains("Enter confirm   Esc cancel"))
    );
    // The call, not a command line: the modal names the method and the
    // resource it acts on, and both come from the builder the runtime derives
    // its argv from.
    assert!(lines.iter().any(|line| {
        line.contains("compute.instances.stop")
            && line.contains("zones/europe-west1-b/instances/api-dev-1")
    }));
    assert!(
        !lines.iter().any(|line| line.contains("gcloud ")),
        "the modal no longer claims a command line"
    );
}

#[tokio::test]
async fn resource_action_confirmation_renders_small_terminal_fallback_when_modal_cannot_fit() {
    let mut session = session_with_loaded_instances().await;
    session
        .dispatch_action(Action::Command(nuvem::app::CommandAction::Open))
        .await;
    session
        .dispatch_action(Action::Command(nuvem::app::CommandAction::SelectNext))
        .await;
    session
        .dispatch_action(Action::Command(nuvem::app::CommandAction::Submit))
        .await;

    let backend = TestBackend::new(60, 12);
    let mut terminal = Terminal::new(backend).expect("terminal should initialize");
    terminal
        .draw(|frame| nuvem::ui::render(frame, &session))
        .expect("render should succeed");

    let lines = terminal_lines(&terminal);
    assert!(
        lines
            .iter()
            .any(|line| line.contains("nuvem GCE confirmation needs a larger terminal"))
    );
    assert!(
        !lines
            .iter()
            .any(|line| line.contains("Action: Stop instance"))
    );
}

/// Forty CJK characters, quoted by the command preview, and forty-two ASCII
/// ones, which are not: the two `Command:` lines hold the same number of
/// `char`s and differ by forty terminal cells. Any width between them tells the
/// two measures apart.
const DOUBLE_WIDTH_INSTANCE_NAME: &str =
    "日本語のインスタンス日本語のインスタンス日本語のインスタンス日本語のインスタンス";
/// Exactly as many `char`s as [`DOUBLE_WIDTH_INSTANCE_NAME`], and half its
/// display width.
///
/// The two used to differ by two characters and still produce previews of
/// equal length, because the display quoting the old command line applied put
/// quotes around the double-width name. The preview names a call now and
/// quotes nothing, so the names themselves have to match: a test whose
/// premise depended on quoting would have gone on asserting something else
/// entirely once the quoting went.
const SAME_CHAR_COUNT_ASCII_INSTANCE_NAME: &str = "api-dev-instance-with-a-very-long-name-a";

async fn gce_confirmation_session(instance_name: &str) -> RuntimeSession {
    let mut session = session_with_instance_named(instance_name).await;
    session
        .dispatch_action(Action::Command(nuvem::app::CommandAction::Open))
        .await;
    session
        .dispatch_action(Action::Command(nuvem::app::CommandAction::Submit))
        .await;
    session
}

/// The modal is sized by what the terminal draws, not by how many `char`s a
/// line holds. Across every width where a character count says both modals fit,
/// only the ASCII one actually does; the double-width one overflows, and
/// reporting that it fits would show the operator a truncated command.
#[tokio::test]
async fn modal_sizing_measures_display_width_rather_than_character_count() {
    let wide = gce_confirmation_session(DOUBLE_WIDTH_INSTANCE_NAME).await;
    let ascii = gce_confirmation_session(SAME_CHAR_COUNT_ASCII_INSTANCE_NAME).await;
    let wide_confirmation = wide.app().root.resources.action_confirmation().unwrap();
    let ascii_confirmation = ascii.app().root.resources.action_confirmation().unwrap();
    assert_eq!(
        wide_confirmation.call().preview().chars().count(),
        ascii_confirmation.call().preview().chars().count(),
        "the two previews must be indistinguishable to a character count"
    );

    // The band of widths is derived rather than written down. It used to be
    // 170..=190, chosen for a preview that named a whole gcloud command line;
    // the preview is shorter now, both fit on one row there, and a hardcoded
    // band would have reported the property as gone when what moved was the
    // length. What the property actually says is this: wherever both modals
    // draw, the double-width one never needs fewer rows, and somewhere it
    // needs more. A sizing that counted characters would give them the same
    // height everywhere.
    let mut anywhere_taller = false;
    let mut widths_drawn = 0usize;
    for width in 40..=200u16 {
        let area = Rect::new(0, 0, width, 40);
        let ascii_visible =
            nuvem::ui::resource_action_confirmation_preview_visible(area, ascii_confirmation);
        let wide_visible =
            nuvem::ui::resource_action_confirmation_preview_visible(area, wide_confirmation);
        if !ascii_visible || !wide_visible {
            continue;
        }
        widths_drawn += 1;

        let ascii_rows = nuvem::ui::resource_action_confirmation_rows(area, ascii_confirmation);
        let wide_rows = nuvem::ui::resource_action_confirmation_rows(area, wide_confirmation);
        assert!(
            wide_rows >= ascii_rows,
            "at width {width} the double-width preview took fewer rows than its \
             character-identical ASCII twin, which no measure of display width \
             can produce"
        );
        anywhere_taller |= wide_rows > ascii_rows;
    }

    assert!(
        widths_drawn > 0,
        "the walk found no width at which both modals draw, so it asserted \
         nothing about either"
    );
    assert!(
        anywhere_taller,
        "at no width did the double-width preview need more rows than its \
         character-identical ASCII twin: the sizing is counting characters"
    );
}

/// The end of that path: a preview that does not fit renders the abbreviated
/// notice rather than clipping the command the operator is about to confirm.
#[tokio::test]
async fn resource_action_confirmation_wraps_a_double_width_name_rather_than_refusing() {
    let session = gce_confirmation_session(DOUBLE_WIDTH_INSTANCE_NAME).await;

    let backend = TestBackend::new(180, 40);
    let mut terminal = Terminal::new(backend).expect("terminal should initialize");
    terminal
        .draw(|frame| nuvem::ui::render(frame, &session))
        .expect("render should succeed");

    let lines = terminal_lines(&terminal);
    assert!(
        !lines
            .iter()
            .any(|line| line.contains("nuvem GCE confirmation needs a larger terminal")),
        "a name too wide for one row is wrapped, not refused: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line.contains("Action: Stop instance"))
    );
    // Reassembled across rows: the name is what makes the line too wide, so it
    // is the thing most likely to be split by the wrap.
    let modal: String = lines
        .iter()
        .map(|line| line.trim_matches(|c| c == '│' || c == ' '))
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .concat();
    // Spaces dropped from both sides: a double-width glyph occupies two terminal
    // cells and the backend fills the second with a space, so reading the buffer
    // back character by character can never reproduce the name verbatim.
    let squeeze = |text: &str| text.chars().filter(|c| *c != ' ').collect::<String>();
    assert!(
        squeeze(&modal).contains(&squeeze(DOUBLE_WIDTH_INSTANCE_NAME)),
        "the name itself must still be readable in full: {lines:#?}"
    );
}

/// The companion of the double-width case: an ASCII name of the same character
/// count still renders in full at the same size, so measuring by display width
/// is a no-op for every preview the app renders today.
#[tokio::test]
async fn resource_action_confirmation_renders_an_ascii_name_of_the_same_length() {
    let session = gce_confirmation_session(SAME_CHAR_COUNT_ASCII_INSTANCE_NAME).await;

    let backend = TestBackend::new(180, 40);
    let mut terminal = Terminal::new(backend).expect("terminal should initialize");
    terminal
        .draw(|frame| nuvem::ui::render(frame, &session))
        .expect("render should succeed");

    let lines = terminal_lines(&terminal);
    assert!(
        lines
            .iter()
            .any(|line| line.contains("Action: Stop instance"))
    );
    assert!(lines.iter().any(|line| {
        line.contains(&format!(
            "compute.instances.stop zones/europe-west1-b/instances/{SAME_CHAR_COUNT_ASCII_INSTANCE_NAME}"
        ))
    }));
}

#[tokio::test]
async fn resource_action_confirmation_wraps_a_command_too_wide_for_one_row() {
    let mut session = session_with_loaded_instances().await;
    session
        .dispatch_action(Action::Command(nuvem::app::CommandAction::Open))
        .await;
    session
        .dispatch_action(Action::Command(nuvem::app::CommandAction::SelectNext))
        .await;
    session
        .dispatch_action(Action::Command(nuvem::app::CommandAction::Submit))
        .await;

    let backend = TestBackend::new(80, 30);
    let mut terminal = Terminal::new(backend).expect("terminal should initialize");
    terminal
        .draw(|frame| nuvem::ui::render(frame, &session))
        .expect("render should succeed");

    let lines = terminal_lines(&terminal);

    // This width used to refuse: the modal demanded its ninety-column command
    // line fit one row, so below about 115 columns the operator could neither
    // confirm nor read what they would have confirmed, and the action was
    // simply unavailable. It wraps now, and the guard is left for the
    // sizes where the content genuinely does not fit — see
    // `small_terminal_overlays_preserve_fallback_messages_at_60x12`.
    assert!(
        !lines
            .iter()
            .any(|line| line.contains("nuvem GCE confirmation needs a larger terminal")),
        "the confirmation must draw at 80x30: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line.contains("Action: Reset instance")),
        "the confirmation's fields must be on screen: {lines:#?}"
    );

    // The command in full, across however many rows it takes. Compared with
    // whitespace normalised because `wrap_exact` keeps the space it breaks
    // after on the first row, where the terminal's own padding then makes it
    // unreadable as a break.
    let modal: String = lines
        .iter()
        .map(|line| line.trim_matches(|c| c == '│' || c == ' '))
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    assert!(
        modal.contains("compute.instances.reset zones/europe-west1-b/instances/api-dev-1"),
        "the whole command must be readable: {lines:#?}"
    );
}

#[tokio::test]
async fn cloud_sql_action_confirmation_renders_target_context_and_command_preview() {
    let mut session = session_with_loaded_cloud_sql_instance().await;
    session
        .dispatch_action(Action::Command(nuvem::app::CommandAction::Open))
        .await;
    for ch in "stop selected cloud sql".chars() {
        session
            .dispatch_action(Action::Command(nuvem::app::CommandAction::AppendChar(ch)))
            .await;
    }
    session
        .dispatch_action(Action::Command(nuvem::app::CommandAction::Submit))
        .await;

    let backend = TestBackend::new(120, 20);
    let mut terminal = Terminal::new(backend).expect("terminal should initialize");
    terminal
        .draw(|frame| nuvem::ui::render(frame, &session))
        .expect("render should succeed");

    let lines = terminal_lines(&terminal);
    assert!(
        lines
            .iter()
            .any(|line| line.contains("confirm Cloud SQL action"))
    );
    assert!(!lines.iter().any(|line| line.contains("confirm GCE action")));
    assert!(
        lines
            .iter()
            .any(|line| line.contains("Confirm Cloud SQL action"))
    );
    assert!(
        lines
            .iter()
            .any(|line| line.contains("Action: Stop instance"))
    );
    assert!(lines.iter().any(|line| line.contains("Instance: pg-ha-1")));
    assert!(lines.iter().any(|line| line.contains("Region: us-east1")));
    assert!(lines.iter().any(|line| line.contains("Configuration: dev")));
    assert!(
        lines
            .iter()
            .any(|line| line.contains("Project: dev-project"))
    );
    assert!(
        lines.iter().any(|line| {
            line.contains("Impact: Databases become unavailable and billing pauses.")
        })
    );
    assert!(
        lines
            .iter()
            .any(|line| line.contains("Enter confirm   Esc cancel"))
    );
    // Cloud SQL's stop is a patch, so the field it sets is part of what the
    // operator is confirming: without it this modal and the one for start
    // would read identically.
    assert!(lines.iter().any(|line| {
        line.contains("sql.instances.patch")
            && line.contains("instances/pg-ha-1")
            && line.contains("activationPolicy=NEVER")
    }));
}

#[tokio::test]
async fn resume_confirmation_renders_exact_compute_instances_command() {
    let mut session = session_with_suspended_instance().await;
    session
        .dispatch_action(Action::Command(nuvem::app::CommandAction::Open))
        .await;
    session
        .dispatch_action(Action::Command(nuvem::app::CommandAction::Submit))
        .await;

    let lines = render_lines(&session, 120, 20);
    assert!(lines.iter().any(|line| {
        line.contains("compute.instances.resume zones/europe-west1-b/instances/api-dev-1")
    }));
}

#[tokio::test]
async fn ssh_command_preview_renders_exact_command_and_copy_cancel_keys() {
    let mut session = session_with_loaded_instances().await;
    session
        .dispatch_action(Action::Command(nuvem::app::CommandAction::Open))
        .await;
    for _ in 0..3 {
        session
            .dispatch_action(Action::Command(nuvem::app::CommandAction::SelectNext))
            .await;
    }
    session
        .dispatch_action(Action::Command(nuvem::app::CommandAction::Submit))
        .await;

    let lines = render_lines(&session, 120, 20);
    assert!(lines.iter().any(|line| line.contains("Show SSH command")));
    assert!(lines.iter().any(|line| {
        line.contains(
            "Command: gcloud --configuration dev compute ssh api-dev-1 --zone europe-west1-b",
        )
    }));
    assert!(
        lines
            .iter()
            .any(|line| line.contains("Enter copy   Esc cancel"))
    );
}

#[tokio::test]
async fn ssh_command_preview_renders_whole_at_the_smallest_supported_terminal() {
    let mut session = session_with_loaded_instances().await;
    session
        .dispatch_action(Action::Command(nuvem::app::CommandAction::Open))
        .await;
    for _ in 0..3 {
        session
            .dispatch_action(Action::Command(nuvem::app::CommandAction::SelectNext))
            .await;
    }
    session
        .dispatch_action(Action::Command(nuvem::app::CommandAction::Submit))
        .await;

    // 60x12 is the app's own minimum supported size, and this preview used to
    // take the fallback there because its command did not fit one row. It wraps
    // now, so it is short enough to draw on every terminal nuvem
    // supports — the guard is still reached by the taller modals, which
    // `small_terminal_overlays_preserve_fallback_messages_at_60x12` covers.
    let lines = render_lines(&session, 60, 12);
    assert!(
        !lines
            .iter()
            .any(|line| line.contains("nuvem command preview needs a larger terminal")),
        "the preview must draw at the smallest supported terminal: {lines:#?}"
    );
    assert!(
        lines.iter().any(|line| line.contains("Enter copy")),
        "its controls must be on screen: {lines:#?}"
    );
}

async fn load_dev_instances(session: &mut RuntimeSession) {
    load_active_dev(session).await;
    let context = session.app().root.current_context().unwrap().clone();
    session
        .apply_effect_result(EffectResult::ComputeInstancesLoaded {
            context,
            instances: vec![
                ComputeInstance {
                    id: "1001".to_string(),
                    name: "api-dev-1".to_string(),
                    zone: "europe-west1-b".to_string(),
                    status: ResourceStatus::Running,
                    machine_type: "e2-medium".to_string(),
                    internal_ip: Some("10.0.0.4".to_string()),
                    external_ip: Some("34.10.10.10".to_string()),
                },
                ComputeInstance {
                    id: "1002".to_string(),
                    name: "worker-dev-1".to_string(),
                    zone: "europe-west1-c".to_string(),
                    status: ResourceStatus::Stopped,
                    machine_type: "e2-small".to_string(),
                    internal_ip: Some("10.0.0.5".to_string()),
                    external_ip: None,
                },
            ],
        })
        .await;
}

async fn load_active_dev(session: &mut RuntimeSession) {
    session
        .apply_effect_result(EffectResult::ConfigurationsLoaded {
            configurations: vec![GcloudConfiguration {
                name: GcloudConfigurationName::new("dev"),
                is_active: true,
                account: Some("dev@example.com".to_string()),
                project: Some("dev-project".to_string()),
                region: None,
                zone: None,
            }],
        })
        .await;
}

async fn session_with_activation_confirmation() -> RuntimeSession {
    let mut session = RuntimeSession::new_for_test(AppState::new());
    session
        .apply_effect_result(EffectResult::ConfigurationsLoaded {
            configurations: vec![
                GcloudConfiguration {
                    name: GcloudConfigurationName::new("dev"),
                    is_active: true,
                    account: Some("dev@example.com".to_string()),
                    project: Some("dev-project".to_string()),
                    region: Some("europe-west1".to_string()),
                    zone: Some("europe-west1-b".to_string()),
                },
                GcloudConfiguration {
                    name: GcloudConfigurationName::new("prod"),
                    is_active: false,
                    account: Some("prod@example.com".to_string()),
                    project: Some("prod-project".to_string()),
                    region: Some("us-central1".to_string()),
                    zone: Some("us-central1-a".to_string()),
                },
            ],
        })
        .await;
    session
        .dispatch_action(Action::Workspace(WorkspaceAction::Configurations))
        .await;
    session
        .dispatch_action(Action::Configurations(ConfigurationsAction::SelectNext))
        .await;
    session
        .dispatch_action(Action::Configurations(
            ConfigurationsAction::RequestActivateSelected,
        ))
        .await;
    session
}

async fn session_with_loaded_instances() -> RuntimeSession {
    session_with_instance_named("api-dev-1").await
}

async fn session_with_instance_named(name: &str) -> RuntimeSession {
    let mut session = RuntimeSession::new_for_test(AppState::new());
    session
        .apply_effect_result(EffectResult::ConfigurationsLoaded {
            configurations: vec![GcloudConfiguration {
                name: GcloudConfigurationName::new("dev"),
                is_active: true,
                account: None,
                project: Some("dev-project".to_string()),
                region: Some("europe-west1".to_string()),
                zone: Some("europe-west1-b".to_string()),
            }],
        })
        .await;
    session
        .dispatch_action(Action::Workspace(WorkspaceAction::Resources))
        .await;
    let context = session.app().root.current_context().unwrap().clone();
    session
        .apply_effect_result(EffectResult::ComputeInstancesLoaded {
            context,
            instances: vec![ComputeInstance {
                id: format!("{name}-id"),
                name: name.to_string(),
                zone: "europe-west1-b".to_string(),
                status: ResourceStatus::Running,
                machine_type: "e2-medium".to_string(),
                internal_ip: Some("10.0.0.2".to_string()),
                external_ip: None,
            }],
        })
        .await;
    session
}

async fn session_with_loaded_cloud_sql_instance() -> RuntimeSession {
    let mut session = RuntimeSession::new_for_test(AppState::new());
    session
        .apply_effect_result(EffectResult::ConfigurationsLoaded {
            configurations: vec![GcloudConfiguration {
                name: GcloudConfigurationName::new("dev"),
                is_active: true,
                account: None,
                project: Some("dev-project".to_string()),
                region: Some("us-east1".to_string()),
                zone: None,
            }],
        })
        .await;
    session
        .dispatch_action(Action::Workspace(WorkspaceAction::Resources))
        .await;
    session
        .dispatch_action(Action::Resources(
            nuvem::app::ResourcesAction::SelectService(ResourceServiceKind::CloudSql),
        ))
        .await;
    let context = session.app().root.current_context().unwrap().clone();
    session
        .apply_effect_result(EffectResult::CloudSqlInstancesLoaded {
            context,
            instances: vec![CloudSqlInstance {
                name: "pg-ha-1".to_string(),
                database_version: "POSTGRES_15".to_string(),
                engine: "PostgreSQL".to_string(),
                region: "us-east1".to_string(),
                tier: "db-custom-2-8192".to_string(),
                availability: CloudSqlAvailability::Regional,
                status: CloudSqlStatus::Runnable,
                connectivity: EndpointVisibility::Private,
            }],
        })
        .await;
    session
}

async fn session_with_suspended_instance() -> RuntimeSession {
    let mut session = RuntimeSession::new_for_test(AppState::new());
    session
        .apply_effect_result(EffectResult::ConfigurationsLoaded {
            configurations: vec![GcloudConfiguration {
                name: GcloudConfigurationName::new("dev"),
                is_active: true,
                account: None,
                project: Some("dev-project".to_string()),
                region: Some("europe-west1".to_string()),
                zone: Some("europe-west1-b".to_string()),
            }],
        })
        .await;
    session
        .dispatch_action(Action::Workspace(WorkspaceAction::Resources))
        .await;
    let context = session.app().root.current_context().unwrap().clone();
    session
        .apply_effect_result(EffectResult::ComputeInstancesLoaded {
            context,
            instances: vec![ComputeInstance {
                id: "api-dev-1-id".to_string(),
                name: "api-dev-1".to_string(),
                zone: "europe-west1-b".to_string(),
                status: ResourceStatus::Suspended,
                machine_type: "e2-medium".to_string(),
                internal_ip: Some("10.0.0.2".to_string()),
                external_ip: None,
            }],
        })
        .await;
    session
}

fn detail(name: &str) -> ComputeInstanceDetail {
    use nuvem_domain::gcp::resource::{
        ComputeDisk, ComputeInstanceDetail, ComputeKeyValue, ComputeNetworkInterface,
    };

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
        labels: vec![nuvem_domain::gcp::resource::ComputeKeyValue {
            key: "env".to_string(),
            value: "dev".to_string(),
        }],
        metadata: vec![ComputeKeyValue {
            key: "owner".to_string(),
            value: "platform".to_string(),
        }],
        raw_json: "{\n  \"name\": \"api-dev-1\"\n}".to_string(),
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
            machine_type: Some("e2-standard-4".to_string()),
            autoscaling: Some("1-6".to_string()),
            node_count: Some(3),
        }],
        addons: vec![GkeAddon {
            name: "HTTP load balancing".to_string(),
            enabled: true,
        }],
        labels: Vec::new(),
        raw_json: format!("{{\n  \"name\": \"{name}\"\n}}"),
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

fn cloud_run_service(name: &str, region: &str) -> CloudRunService {
    CloudRunService {
        name: name.to_string(),
        region: region.to_string(),
        url: Some(format!("https://{name}-abc.a.run.app")),
        latest_ready_revision: Some(format!("{name}-00002")),
        latest_created_revision: Some(format!("{name}-00002")),
        traffic_summary: "100% -> LATEST".to_string(),
        status: CloudRunServiceStatus::Ready,
        ready_message: None,
    }
}

fn cloud_run_detail(name: &str, region: &str) -> CloudRunServiceDetail {
    CloudRunServiceDetail {
        name: name.to_string(),
        region: region.to_string(),
        url: Some(format!("https://{name}-abc.a.run.app")),
        status: CloudRunServiceStatus::Ready,
        latest_ready_revision: Some(format!("{name}-00002")),
        latest_created_revision: Some(format!("{name}-00002")),
        traffic: vec![CloudRunTrafficTarget {
            revision_name: Some(format!("{name}-00002")),
            tag: None,
            percent: Some(100),
            latest: true,
            url: None,
        }],
        revisions: vec![CloudRunRevision {
            name: format!("{name}-00001"),
            ready: CloudRunServiceStatus::Ready,
            created: None,
            image: Some("gcr.io/x/api:1".to_string()),
            active_percent: Some(100),
        }],
        conditions: vec![CloudRunCondition {
            type_: "Ready".to_string(),
            status: "True".to_string(),
            reason: None,
            message: None,
            last_transition_time: None,
        }],
        image: Some("gcr.io/x/api:1".to_string()),
        cpu: Some("1000m".to_string()),
        memory: Some("512Mi".to_string()),
        concurrency: Some(80),
        service_account: Some("svc@project.iam".to_string()),
        ingress: Some("all".to_string()),
        min_instances: Some(0),
        max_instances: Some(10),
        labels: Vec::new(),
        raw_json: format!("{{\n  \"name\": \"{name}\"\n}}"),
    }
}

fn cloud_run_revision_detail(name: &str, region: &str) -> CloudRunRevisionDetail {
    CloudRunRevisionDetail {
        name: name.to_string(),
        service: Some("api".to_string()),
        region: region.to_string(),
        ready: CloudRunServiceStatus::Ready,
        conditions: vec![CloudRunCondition {
            type_: "Active".to_string(),
            status: "True".to_string(),
            reason: None,
            message: None,
            last_transition_time: None,
        }],
        image: Some("gcr.io/x/api:1".to_string()),
        cpu: Some("1000m".to_string()),
        memory: Some("512Mi".to_string()),
        concurrency: Some(80),
        min_instances: Some(0),
        max_instances: Some(10),
        service_account: Some("svc@project.iam".to_string()),
        created: Some("2026-07-19T00:00:00Z".to_string()),
        raw_json: format!("{{\n  \"name\": \"{name}\"\n}}"),
    }
}

fn load_config_and_instances(app: &mut AppState) {
    reduce(
        app,
        &Action::EffectResult(EffectResult::ConfigurationsLoaded {
            configurations: vec![GcloudConfiguration {
                name: GcloudConfigurationName::new("dev"),
                is_active: true,
                account: Some("dev@example.com".to_string()),
                project: Some("dev-project".to_string()),
                region: Some("europe-west1".to_string()),
                zone: Some("europe-west1-b".to_string()),
            }],
        }),
    );
    reduce(app, &Action::Workspace(WorkspaceAction::Resources));
    let context = app.root.current_context().expect("active context exists");
    reduce(
        app,
        &Action::EffectResult(EffectResult::ComputeInstancesLoaded {
            context,
            instances: vec![ComputeInstance {
                id: "api-dev-1-id".to_string(),
                name: "api-dev-1".to_string(),
                zone: "europe-west1-b".to_string(),
                status: ResourceStatus::Running,
                machine_type: "e2-medium".to_string(),
                internal_ip: Some("10.0.0.2".to_string()),
                external_ip: None,
            }],
        }),
    );
}

#[test]
fn wide_list_view_shows_preview_pane() {
    let mut app = AppState::new();
    load_config_and_instances(&mut app);
    let session = RuntimeSession::new_for_test(app);

    let lines = render_lines(&session, 120, 30);

    assert!(lines.iter().any(|line| line.contains("api-dev-1")));
    assert!(lines.iter().any(|line| line.contains("Machine Type")));
}

#[test]
fn narrow_list_view_hides_preview_pane() {
    let mut app = AppState::new();
    load_config_and_instances(&mut app);
    let session = RuntimeSession::new_for_test(app);

    let lines = render_lines(&session, 80, 30);

    assert!(lines.iter().any(|line| line.contains("api-dev-1")));
    assert!(!lines.iter().any(|line| line.contains("Machine Type")));
}

#[tokio::test]
async fn wide_detail_view_keeps_list_visible() {
    let mut session = RuntimeSession::new_for_test(AppState::new());
    load_dev_instances(&mut session).await;
    session
        .dispatch_action(Action::Workspace(WorkspaceAction::Resources))
        .await;
    session
        .dispatch_action(Action::Resources(nuvem::app::ResourcesAction::OpenSelected))
        .await;
    // Hide the service sidebar so the wide list+detail split has the full body
    // width; with the sidebar the list column is too narrow to show both rows.
    session
        .dispatch_action(Action::Navigator(
            nuvem::app::NavigatorAction::ToggleSidebar,
        ))
        .await;

    let lines = render_lines(&session, 120, 30);

    assert!(lines.iter().any(|line| line.contains("worker-dev-1")));
}

#[test]
fn resource_detail_footer_includes_back_hint_when_history_present() {
    let mut app = AppState::new();
    load_config_and_instances(&mut app);
    reduce(
        &mut app,
        &Action::Resources(nuvem::app::ResourcesAction::OpenSelected),
    );
    let frame = project_app_frame(&app.root);
    // One Back hint, carrying every key that goes back from a detail pane.
    let backs: Vec<_> = frame
        .footer_hints
        .iter()
        .filter(|h| h.label == "Back")
        .map(|h| h.key.as_str())
        .collect();
    assert_eq!(backs, ["q/Esc/Alt-←"]);
}

#[test]
fn resource_list_footer_omits_forward_hint_without_forward_history() {
    let mut app = AppState::new();
    load_config_and_instances(&mut app);
    let frame = project_app_frame(&app.root);
    assert!(!frame.footer_hints.iter().any(|h| h.label == "Forward"));
}

fn app_with_open_context_picker(focus_steps: usize) -> AppState {
    let mut app = AppState::new();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ConfigurationsLoaded {
            configurations: vec![GcloudConfiguration {
                name: GcloudConfigurationName::new("dev"),
                is_active: true,
                account: Some("dev@example.com".to_string()),
                project: Some("dev-project".to_string()),
                region: Some("europe-west1".to_string()),
                zone: Some("europe-west1-b".to_string()),
            }],
        }),
    );
    reduce(
        &mut app,
        &Action::Workspace(WorkspaceAction::Configurations),
    );
    reduce(
        &mut app,
        &Action::Configurations(ConfigurationsAction::StartEditActiveContext),
    );
    for _ in 0..focus_steps {
        reduce(
            &mut app,
            &Action::Configurations(ConfigurationsAction::FocusNextContextEditField),
        );
    }
    let effects = reduce(
        &mut app,
        &Action::Configurations(ConfigurationsAction::OpenContextPicker),
    );
    assert_eq!(
        effects.len(),
        1,
        "opening a picker should request discovery"
    );
    app
}

fn load_picker_items(
    app: &mut AppState,
    kind: ConfigurationContextPickerKind,
    items: Vec<ConfigurationContextPickerItem>,
) {
    reduce(
        app,
        &Action::EffectResult(EffectResult::ContextPickerLoaded {
            configuration: GcloudConfigurationName::new("dev"),
            kind,
            items,
        }),
    );
}

fn long_project_picker_items(count: usize) -> Vec<ConfigurationContextPickerItem> {
    (0..count)
        .map(|index| {
            ConfigurationContextPickerItem::Project(GcloudProject {
                project_id: format!("project-{index:02}"),
                name: format!("Project {index:02}"),
                lifecycle_state: Some("ACTIVE".to_string()),
            })
        })
        .collect()
}

fn render_lines(session: &RuntimeSession, width: u16, height: u16) -> Vec<String> {
    let terminal = render_terminal(session, width, height);
    terminal_lines(&terminal)
}

/// True if `line` is the shared in-flight load affordance for `label`: a
/// braille spinner frame, the service's own noun, and an elapsed counter.
///
/// Both varying parts are matched by shape rather than by value: the frame
/// advances every 125ms and the counter every second, both measured from
/// `started_at` at render time, so pinning either to its t=0 value makes the
/// assertion fail whenever the suite runs slowly enough under load.
fn renders_loading_affordance(line: &str, label: &str) -> bool {
    let Some((before, after)) = line.split_once(&format!(" Loading {label}… (")) else {
        return false;
    };
    let has_spinner_frame =
        matches!(before.chars().last(), Some(frame) if ('\u{2800}'..='\u{28ff}').contains(&frame));
    has_spinner_frame
        && after.starts_with(|c: char| c.is_ascii_digit())
        && after.contains("s elapsed)")
}

/// Guardrail 3: prove at render level that a Cloud SQL detail built from a
/// cert+password fixture never leaks the cert PEM body or a password into the
/// projected Connectivity, Users, or JSON tab lines.
#[tokio::test]
async fn cloud_sql_detail_never_renders_cert_body_or_password() {
    const DETAIL_JSON: &str = include_str!("fixtures/cloudsql_instance_detail.json");
    const USERS_JSON: &str = include_str!("fixtures/cloudsql_users.json");

    let detail = nuvem::services::resources::cloudsql::parse_instance_detail_json(DETAIL_JSON)
        .expect("detail fixture parses");
    let users = nuvem::services::resources::cloudsql::parse_users_json(USERS_JSON)
        .expect("users fixture parses");

    let mut session = RuntimeSession::new_for_test(AppState::new());
    load_active_dev(&mut session).await;
    session
        .dispatch_action(Action::Resources(
            nuvem::app::ResourcesAction::SelectService(ResourceServiceKind::CloudSql),
        ))
        .await;
    let context = session.app().root.current_context().unwrap().clone();
    session
        .apply_effect_result(EffectResult::CloudSqlInstancesLoaded {
            context: context.clone(),
            instances: vec![CloudSqlInstance {
                name: detail.name.clone(),
                database_version: detail.database_version.clone(),
                engine: detail.engine.clone(),
                region: detail.region.clone(),
                tier: detail.tier.clone(),
                availability: CloudSqlAvailability::Regional,
                status: CloudSqlStatus::Runnable,
                connectivity: EndpointVisibility::Private,
            }],
        })
        .await;
    session
        .dispatch_action(Action::Workspace(WorkspaceAction::Resources))
        .await;
    session
        .dispatch_action(Action::Resources(nuvem::app::ResourcesAction::OpenSelected))
        .await;
    session
        .apply_effect_result(EffectResult::CloudSqlInstanceDetailLoaded {
            context: context.clone(),
            detail: Box::new(detail),
        })
        .await;
    session
        .apply_effect_result(EffectResult::CloudSqlUsersLoaded {
            context,
            instance_name: "mysql-ha".to_string(),
            users,
        })
        .await;

    // Tabs: Summary(0) Connectivity(1) Maintenance(2) Backups(3) Replicas(4)
    // Users(5) Json(6). Sweep every tab and assert no secret ever renders.
    for _ in 0..7 {
        let lines = render_lines(&session, 100, 40).join("\n");
        assert!(
            !lines.contains("SECRETPEM") && !lines.contains("ARRAYSECRETPEM"),
            "cert PEM body leaked into a rendered tab:\n{lines}"
        );
        assert!(
            !lines.contains("BEGIN CERTIFICATE"),
            "cert PEM header leaked into a rendered tab:\n{lines}"
        );
        assert!(
            !lines.contains("SUPERSECRET"),
            "password leaked into a rendered tab:\n{lines}"
        );
        session
            .dispatch_action(Action::Resources(
                nuvem::app::ResourcesAction::SelectNextTab,
            ))
            .await;
    }
}

fn render_terminal(session: &RuntimeSession, width: u16, height: u16) -> Terminal<TestBackend> {
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).expect("terminal should initialize");
    terminal
        .draw(|frame| nuvem::ui::render(frame, session))
        .expect("render should succeed");
    terminal
}

fn terminal_lines(terminal: &Terminal<TestBackend>) -> Vec<String> {
    let buffer = terminal.backend().buffer();
    let width = buffer.area.width as usize;
    buffer
        .content()
        .chunks(width)
        .map(|line| line.iter().map(|cell| cell.symbol()).collect::<String>())
        .collect()
}

fn assert_modal_chrome(terminal: &Terminal<TestBackend>, title: &str) {
    let buffer = terminal.backend().buffer();
    let width = buffer.area.width as usize;
    let title_row = buffer
        .content()
        .chunks(width)
        .find(|row| {
            row.iter()
                .map(|cell| cell.symbol())
                .collect::<String>()
                .contains(title)
        })
        .unwrap_or_else(|| panic!("expected modal title {title:?}"));
    // Located by where the title actually starts, not by the first cell whose
    // symbol happens to equal its first character. These modals
    // draw over the table rather than replacing the screen with a fallback
    // message, so the row carries other text to the left of the title and a
    // first-character scan lands in it.
    let row_text: String = title_row.iter().map(|cell| cell.symbol()).collect();
    let byte = row_text.find(title).expect("modal title on its row");
    let title_index = row_text[..byte].chars().count();
    assert_eq!(
        title_row[title_index].style().fg,
        Some(Color::Cyan),
        "modal title should use the semantic title style"
    );
    assert!(
        buffer
            .content()
            .iter()
            .any(|cell| cell.style().fg == Some(Color::Magenta)),
        "modal border should use the semantic overlay border style"
    );
}

fn assert_shared_frame(lines: &[String], view_name: &str, footer_hint: &str) {
    for expected in [
        "nuvem",
        view_name,
        "Configuration: dev",
        "Project: dev-project",
        "[OK]",
        footer_hint,
    ] {
        assert!(
            lines.iter().any(|line| line.contains(expected)),
            "expected shared frame text containing {expected:?}"
        );
    }
}

fn line_index(lines: &[String], expected: &str) -> usize {
    lines
        .iter()
        .position(|line| line.contains(expected))
        .unwrap_or_else(|| panic!("expected line containing {expected:?}"))
}

fn line_index_after(lines: &[String], start: usize, expected: &str) -> usize {
    lines
        .iter()
        .enumerate()
        .skip(start + 1)
        .find_map(|(index, line)| line.contains(expected).then_some(index))
        .unwrap_or_else(|| panic!("expected line after {start} containing {expected:?}"))
}

#[test]
fn resource_batch_confirmation_lines_list_every_target_with_full_command() {
    use nuvem::app::state::resources::{Compute, ComputeInstanceAction, ResourcesState};
    use nuvem::ui::resources::resource_batch_confirmation_lines;
    use nuvem_domain::gcp::configuration::GcloudContext;

    fn inst(name: &str, status: ResourceStatus) -> ComputeInstance {
        ComputeInstance {
            id: format!("{name}-id"),
            name: name.to_string(),
            zone: "europe-west1-b".to_string(),
            status,
            machine_type: "e2-medium".to_string(),
            internal_ip: None,
            external_ip: None,
        }
    }

    let context = GcloudContext {
        configuration: GcloudConfigurationName::new("dev"),
        account: None,
        project: Some("dev-project".to_string()),
        region: None,
        zone: None,
    };

    let mut state = ResourcesState::default();
    state.set_instances(
        context.clone(),
        vec![
            inst("api", ResourceStatus::Running),
            inst("web", ResourceStatus::Running),
            inst("db", ResourceStatus::Stopped),
        ],
    );
    // Mark all three (visible-sorted: api, db, web).
    state.toggle_mark();
    state.select_next();
    state.toggle_mark();
    state.select_next();
    state.toggle_mark();
    assert_eq!(state.marked_count(), 3);

    state.start_batch_confirmation::<Compute>(context, ComputeInstanceAction::Stop);
    let confirmation = state.batch_confirmation().expect("batch confirmation open");
    let lines = resource_batch_confirmation_lines(confirmation);
    let text = lines.join("\n");

    // Both eligible targets appear by name with their own call, naming the
    // instance and the zone it is in.
    let api_line = lines
        .iter()
        .find(|l| l.starts_with("api: "))
        .expect("api call line present");
    assert!(api_line.contains("compute.instances.stop"));
    assert!(api_line.contains("zones/europe-west1-b/instances/api"));
    let web_line = lines
        .iter()
        .find(|l| l.starts_with("web: "))
        .expect("web call line present");
    assert!(web_line.contains("compute.instances.stop"));
    assert!(web_line.contains("zones/europe-west1-b/instances/web"));

    // Skipped section names the ineligible (Stopped) target.
    assert!(text.contains("Skipped:"));
    assert!(
        lines
            .iter()
            .any(|l| l.starts_with("db ") && l.contains("Stopped"))
    );

    // Destructive action requires the typed-count acknowledgement.
    assert!(lines.iter().any(|l| l == "Type 2 to confirm"));
    assert!(text.contains("Targets: 2 (1 skipped)"));
}

#[test]
fn resource_batch_progress_lines_show_aggregate_and_per_target_status() {
    use nuvem::app::state::resources::{
        BatchAction, BatchTargetStatus, BatchableService, Compute, ComputeInstanceAction,
        ResourcesState,
    };
    use nuvem::ui::resources::resource_batch_progress_lines;
    use nuvem_domain::gcp::configuration::GcloudContext;

    fn inst(name: &str) -> ComputeInstance {
        ComputeInstance {
            id: format!("{name}-id"),
            name: name.to_string(),
            zone: "europe-west1-b".to_string(),
            status: ResourceStatus::Running,
            machine_type: "e2-medium".to_string(),
            internal_ip: None,
            external_ip: None,
        }
    }

    let context = GcloudContext {
        configuration: GcloudConfigurationName::new("dev"),
        account: None,
        project: Some("dev-project".to_string()),
        region: None,
        zone: None,
    };

    let mut state = ResourcesState::default();
    let targets = vec![
        Compute::row_ref(&inst("api"), &context, ComputeInstanceAction::Stop),
        Compute::row_ref(&inst("web"), &context, ComputeInstanceAction::Stop),
    ];
    state.begin_batch_workflow(
        BatchAction::Compute(ComputeInstanceAction::Stop),
        context,
        // No operations log behind this projection test, so the ids are
        // stand-ins. They start well above zero so they can never coincide with
        // a target's index, which is what a positional bug would produce.
        targets
            .into_iter()
            .enumerate()
            .map(|(index, target)| (target, 900 + index as u64))
            .collect(),
    );
    state.resolve_batch_target(
        ResourceServiceKind::ComputeInstances,
        "api",
        Some("europe-west1-b"),
        BatchTargetStatus::Succeeded,
    );

    let workflow = state.batch_workflow().expect("workflow present");
    let lines = resource_batch_progress_lines(workflow);
    let text = lines.join("\n");

    // Aggregate header: one of two done, none failed, one pending.
    assert!(text.contains("1/2"));
    assert!(text.contains("done"));
    assert!(text.contains("failed"));
    assert!(text.contains("pending"));

    // Per-target lines show color-independent status text.
    assert!(
        lines
            .iter()
            .any(|l| l.starts_with("api") && l.contains("OK"))
    );
    assert!(
        lines
            .iter()
            .any(|l| l.starts_with("web") && l.contains("PENDING"))
    );
}

/// A table tools line too wide for its pane drops whole hints, and never runs
/// into the pane beside it.
///
/// The line used to be rendered as a bare `Paragraph`, so ratatui hard-clipped
/// it at the pane edge and the operator saw `←/→ col · s sor`; then it was cut
/// with `...`, which still left `←/→ c...`; and at a width where it just fitted
/// it ran straight into the preview's border as `w wide┌Preview`. Now the
/// least important segments go whole, and a column stays clear before the next
/// pane.
///
/// The width is one the whole line does not fit on, asserted up front so the
/// test cannot pass by proving nothing.
#[tokio::test]
async fn a_table_tools_line_too_wide_for_its_pane_drops_whole_hints() {
    let mut session = RuntimeSession::new_for_test(AppState::new());
    load_dev_instances(&mut session).await;
    session
        .dispatch_action(Action::Workspace(WorkspaceAction::Resources))
        .await;

    let lines = render_lines(&session, TOOLS_LINE_CLIPPING_WIDTH, 40);
    let tools = storage_tools_line(&lines);
    let whole = "Filter: none   Sort: Name ▲   Rows: 2/2   ←/→ col · s sort · w wide";

    // The tools row also carries the sidebar and the preview pane's border, so
    // the line is read from where the tools text starts to where the next block
    // begins.
    let segment: String = tools
        .chars()
        .skip_while(|c| *c != 'F')
        .take_while(|c| *c != '┌')
        .collect();
    assert!(
        segment.ends_with(' '),
        "the tools line must leave a column clear before the next pane: {tools:?}"
    );
    let segment = segment.trim_end();

    assert_ne!(
        segment, whole,
        "this width no longer drops anything from the tools line, so the test \
         proves nothing; got: {tools:?}"
    );
    assert!(
        !segment.contains("..."),
        "the line lost whole segments, so it is not cut: {segment:?}"
    );
    assert!(
        segment.starts_with("Filter: none") && segment.contains("Rows: 2/2"),
        "the filter and the row count are the last to go: {segment:?}"
    );
    for hint in ["←/→ col", "s sort", "w wide"] {
        let shown = segment.contains(hint);
        let cut = hint
            .char_indices()
            .skip(1)
            .any(|(at, _)| segment.ends_with(&hint[..at]));
        assert!(
            shown || !cut,
            "{hint:?} is shown whole or not at all: {segment:?}"
        );
    }
}

/// A terminal wide enough for the sidebar and the preview, and still too narrow
/// for the whole tools line.
///
/// 116 is the narrowest that splits off a preview at all: the split needs a
/// content pane of `WIDE_SPLIT_MIN_CONTENT_WIDTH` (88) after
/// `SERVICE_SIDEBAR_WIDTH` (28) has taken its share. Below it the table has the
/// content pane to itself and the tools line fits, so there is no clipping to
/// assert; above it the table keeps widening until it fits again. This is the
/// bottom of the band where the line is cut off.
const TOOLS_LINE_CLIPPING_WIDTH: u16 = 116;

/// The preview takes what it needs, not a fixed share of the pane.
///
/// `split_wide` used to give the preview 45% of the content pane whether or not
/// it had anything to put there. At 132x40 on GCE instances that left the table
/// 57 columns and truncated almost every cell, while the preview painted five
/// short lines and thirty blank ones. The same table at 100 columns — too narrow
/// for a preview at all — showed ZONE and MACHINE-TYPE whole, so widening the
/// terminal made the table *less* readable.
#[tokio::test]
async fn a_short_preview_leaves_the_table_the_width_it_does_not_need() {
    let mut session = RuntimeSession::new_for_test(AppState::new());
    load_dev_instances(&mut session).await;
    session
        .dispatch_action(Action::Workspace(WorkspaceAction::Resources))
        .await;

    let lines = render_lines(&session, 132, 40);
    let rendered = lines.join("\n");

    assert!(
        rendered.contains("Preview"),
        "this width must still split off a preview pane, or the test asserts \
         nothing about how wide it is; rendered:\n{rendered}"
    );

    // Read out of the table pane, not off the whole row: the preview's own
    // `Zone: europe-west1-b` line lands on the *same screen row* as the table's
    // `api-dev-1`. Searched pane-first rather than row-first for a second
    // reason — the preview's *title* is also `api-dev-1`, painted on the row
    // carrying the table's top border, which a row-first search reaches before
    // any data row.
    let table_cell = lines
        .iter()
        .map(|line| pane(line, 1))
        .find(|cell| cell.contains("api-dev-1"))
        .unwrap_or_else(|| panic!("no instance row in the table pane; rendered:\n{rendered}"));
    assert!(
        table_cell.contains("europe-west1-b"),
        "the ZONE cell is truncated while the preview holds columns it has no \
         content for; table pane: {table_cell:?}"
    );
}

/// The `index`-th bordered pane's content on a row of the wide Resources
/// layout: sidebar, table, then preview. Adjacent borders collapse to `││`, so
/// the empty strings between them are dropped rather than counted.
fn pane(row: &str, index: usize) -> String {
    row.split('│')
        .filter(|segment| !segment.trim().is_empty())
        .nth(index)
        .unwrap_or_default()
        .to_string()
}

/// The other half of the rule: shrinking the pane must not cost the preview
/// anything it was showing.
#[tokio::test]
async fn the_preview_still_shows_its_own_longest_line_whole() {
    let mut session = RuntimeSession::new_for_test(AppState::new());
    load_dev_instances(&mut session).await;
    session
        .dispatch_action(Action::Workspace(WorkspaceAction::Resources))
        .await;

    let rendered = render_lines(&session, 132, 40).join("\n");

    for value in ["Machine Type: e2-medium", "External IP: 34.10.10.10"] {
        assert!(
            rendered.contains(value),
            "the preview must still carry {value:?} whole; rendered:\n{rendered}"
        );
    }
}

/// An overlay leaves nothing of the panes beneath it half-shown.
///
/// Help and the command palette used to be boxes drawn over the panes, so
/// every row they crossed kept the words their edges cut through on either
/// side (`onal)`, `[OK] C`, `l IP: 34.10.10.10`). Now each clears every row
/// it crosses from edge to edge, so on those rows nothing but the overlay is
/// painted. Checked at the four widths the docs and recordings use.
#[tokio::test]
async fn help_and_the_palette_leave_no_fragment_of_the_panes_beneath() {
    for overlay in ["help", "commands"] {
        for (width, height) in [(80, 24), (100, 30), (121, 34), (160, 40)] {
            let mut session = RuntimeSession::new_for_test(AppState::new());
            load_dev_instances(&mut session).await;
            session
                .dispatch_action(Action::Workspace(WorkspaceAction::Resources))
                .await;
            let open = match overlay {
                "help" => Action::System(SystemAction::ToggleHelp),
                _ => Action::Command(CommandAction::Open),
            };
            session.dispatch_action(open).await;

            let lines = render_lines(&session, width, height);
            let top = lines
                .iter()
                .position(|line| line.contains(&format!("┌{overlay}")))
                .unwrap_or_else(|| panic!("no {overlay} box at {width}x{height}:\n{lines:#?}"));
            let left = lines[top]
                .chars()
                .position(|c| c == '┌')
                .expect("the box has a corner");
            let right = lines[top]
                .chars()
                .enumerate()
                .filter(|(_, c)| *c == '┐')
                .map(|(column, _)| column)
                .last()
                .expect("and another");
            let bottom = top
                + lines[top..]
                    .iter()
                    .position(|line| line.chars().nth(left) == Some('└'))
                    .expect("the box closes");
            for (row, line) in lines.iter().enumerate().take(bottom + 1).skip(top) {
                let outside: String = line
                    .chars()
                    .enumerate()
                    .filter(|(column, _)| *column < left || *column > right)
                    .map(|(_, c)| c)
                    .collect();
                assert!(
                    outside.trim().is_empty(),
                    "{overlay} at {width}x{height}: row {row} shows {outside:?} beside the \
                     box:\n{}",
                    lines.join("\n")
                );
            }
        }
    }
}
