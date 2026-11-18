//! Evidence generator for issue #23 — global command palette navigation.
//!
//! Renders the palette from every view through the real `nuvem::ui::render`
//! entrypoint (the exact surface an end user sees) and writes ASCII
//! "screenshots" of the terminal buffer to the evidence directory when
//! `NUVEM_EVIDENCE_DIR` is set (assertions always run). Also
//! exercises the `:` keybinding end-to-end from home and detail views.

use std::fs;
use std::path::PathBuf;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use nuvem::app::reducer::root::reduce;
use nuvem::app::{
    Action, AppState, CommandAction, EffectResult, ResourcesAction, Workspace, WorkspaceAction,
};
use nuvem::input::keymap::action_for_key;
use nuvem::input::{Context, Mode};
use nuvem::services::runtime::session::RuntimeSession;
use nuvem_domain::gcp::configuration::{GcloudConfiguration, GcloudConfigurationName};
use nuvem_domain::gcp::resource::{
    ComputeInstance, EndpointVisibility, GkeAddon, GkeCluster, GkeClusterDetail, GkeClusterStatus,
    GkeNodePool, ResourceServiceKind, ResourceStatus,
};
use ratatui::buffer::Buffer;
use ratatui::style::Modifier;
use ratatui::{Terminal, backend::TestBackend};

fn evidence_dir() -> Option<PathBuf> {
    let dir = std::env::var("NUVEM_EVIDENCE_DIR").ok()?;
    let path = PathBuf::from(dir);
    fs::create_dir_all(&path).ok()?;
    Some(path)
}

fn write_evidence(name: &str, header: &str, lines: &[String]) {
    let Some(dir) = evidence_dir() else {
        return;
    };
    let mut body = String::new();
    body.push_str(header);
    body.push_str("\n\n");
    let width = lines.first().map(|l| l.chars().count()).unwrap_or(0);
    body.push('+');
    body.push_str(&"-".repeat(width));
    body.push_str("+\n");
    for line in lines {
        body.push('|');
        body.push_str(line);
        body.push_str("|\n");
    }
    body.push('+');
    body.push_str(&"-".repeat(width));
    body.push_str("+\n");
    fs::write(dir.join(name), body).expect("write evidence");
}

fn render_buffer(session: &RuntimeSession, width: u16, height: u16) -> Buffer {
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).expect("terminal");
    terminal
        .draw(|frame| nuvem::ui::render(frame, session))
        .expect("render");
    terminal.backend().buffer().clone()
}

fn buffer_lines(buffer: &Buffer) -> Vec<String> {
    let w = buffer.area.width as usize;
    buffer
        .content()
        .chunks(w)
        .map(|line| line.iter().map(|c| c.symbol()).collect::<String>())
        .collect()
}

fn render_lines(session: &RuntimeSession, width: u16, height: u16) -> Vec<String> {
    buffer_lines(&render_buffer(session, width, height))
}

/// Buffer COLUMN of `needle` in a rendered line. `str::find` returns a byte
/// offset, which the box-drawing characters in the frame make useless as an
/// x coordinate.
fn column_of(line: &str, needle: &str) -> Option<usize> {
    let haystack: Vec<char> = line.chars().collect();
    let pattern: Vec<char> = needle.chars().collect();
    haystack.windows(pattern.len()).position(|w| w == pattern)
}

async fn load_dev(session: &mut RuntimeSession) {
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
}

async fn open(session: &mut RuntimeSession) {
    session
        .dispatch_action(Action::Command(CommandAction::Open))
        .await;
}

async fn type_query(session: &mut RuntimeSession, q: &str) {
    for ch in q.chars() {
        session
            .dispatch_action(Action::Command(CommandAction::AppendChar(ch)))
            .await;
    }
}

/// The palette opens (and is never empty) from the Home view.
#[tokio::test]
async fn evidence_palette_from_home() {
    let mut session = RuntimeSession::new_for_test(AppState::new());
    load_dev(&mut session).await;
    open(&mut session).await;
    let lines = render_lines(&session, 100, 26);
    assert!(lines.iter().any(|l| l.contains("Home")));
    assert!(lines.iter().any(|l| l.contains("Configurations")));
    write_evidence(
        "01-palette-from-home.txt",
        "Issue #23: `:` palette opened from HOME. Never empty — navigation \
commands (Home, Configurations, GCE/GKE, Refresh, Help, Switch project) are \
always present with alias hints.",
        &lines,
    );
}

/// The palette opens from the Configurations view.
#[tokio::test]
async fn evidence_palette_from_configurations() {
    let mut session = RuntimeSession::new_for_test(AppState::new());
    load_dev(&mut session).await;
    session
        .dispatch_action(Action::Workspace(WorkspaceAction::Configurations))
        .await;
    open(&mut session).await;
    let lines = render_lines(&session, 100, 26);
    write_evidence(
        "02-palette-from-configurations.txt",
        "Issue #23: `:` palette opened from CONFIGURATIONS. Global navigation \
command set available here too.",
        &lines,
    );
}

/// The palette from a service list shows context (mutation) commands first,
/// then the always-present navigation commands.
#[tokio::test]
async fn evidence_palette_from_gce_list_context_then_navigation() {
    let mut session = RuntimeSession::new_for_test(AppState::new());
    load_dev(&mut session).await;
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
                status: ResourceStatus::Running,
                machine_type: "e2-medium".to_string(),
                internal_ip: Some("10.0.0.2".to_string()),
                external_ip: None,
            }],
        })
        .await;
    open(&mut session).await;
    let lines = render_lines(&session, 100, 28);
    assert!(lines.iter().any(|l| l.contains("selected GCE instance")));
    assert!(lines.iter().any(|l| l.contains("Home")));
    write_evidence(
        "03-palette-from-gce-list.txt",
        "Issue #23: `:` palette opened from the GCE INSTANCES list. Context-first \
ordering: selected-row mutation commands appear first, then the always-present \
navigation commands.",
        &lines,
    );
}

/// The palette opens from a resource DETAIL view (new `:` binding).
#[tokio::test]
async fn evidence_palette_from_gke_detail() {
    let mut session = RuntimeSession::new_for_test(AppState::new());
    load_dev(&mut session).await;
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
        .dispatch_action(Action::Resources(ResourcesAction::SelectService(
            ResourceServiceKind::GkeClusters,
        )))
        .await;
    session
        .dispatch_action(Action::Resources(ResourcesAction::OpenSelected))
        .await;
    session
        .apply_effect_result(EffectResult::GkeClusterDetailLoaded {
            context,
            detail: gke_detail("platform-dev", "europe-west1"),
        })
        .await;
    open(&mut session).await;
    let lines = render_lines(&session, 100, 28);
    assert!(lines.iter().any(|l| l.contains("Home")));
    write_evidence(
        "04-palette-from-gke-detail.txt",
        "Issue #23: `:` palette opened from a GKE cluster DETAIL view (new detail \
binding). Global navigation lets the user jump out of a detail view directly.",
        &lines,
    );
}

/// Empty results render the typed query plus a recovery hint.
#[tokio::test]
async fn evidence_palette_empty_state() {
    let mut session = RuntimeSession::new_for_test(AppState::new());
    load_dev(&mut session).await;
    open(&mut session).await;
    type_query(&mut session, "zzz").await;
    let lines = render_lines(&session, 100, 26);
    assert!(
        lines
            .iter()
            .any(|l| l.contains("No commands match \"zzz\""))
    );
    assert!(lines.iter().any(|l| l.contains("Type to search")));
    write_evidence(
        "05-palette-empty-state.txt",
        "Issue #23: empty search results show the typed query (\"zzz\") plus a \
concise recovery hint. The palette itself never opens empty.",
        &lines,
    );
}

/// Ranked search: exact-before-prefix-before-substring ordering.
#[tokio::test]
async fn evidence_palette_ranked_search() {
    let mut session = RuntimeSession::new_for_test(AppState::new());
    load_dev(&mut session).await;
    open(&mut session).await;
    type_query(&mut session, "co").await;
    let lines = render_lines(&session, 100, 26);
    write_evidence(
        "06-palette-ranked-search.txt",
        "Issue #23: case-insensitive ranked search over labels + aliases for query \
\"co\" (matches the `compute`/`configs` aliases). Ordering is exact < prefix < \
substring, ties preserving context-first-then-navigation order.",
        &lines,
    );
}

/// The `:` keybinding maps to CommandAction::Open from home, service list,
/// and detail views — end-to-end keymap evidence.
#[test]
fn evidence_colon_binding_opens_palette_everywhere() {
    let colon = KeyEvent::new(KeyCode::Char(':'), KeyModifiers::NONE);
    let mut report = String::new();
    for (label, workspace, detail_open) in [
        ("home", Workspace::Home, false),
        ("configurations", Workspace::Configurations, false),
        ("service-list", Workspace::Resources, false),
        ("resource-detail", Workspace::Resources, true),
    ] {
        let ctx = normal_context(workspace, detail_open);
        let action = action_for_key(colon, &ctx);
        let opens = matches!(action, Some(Action::Command(CommandAction::Open)));
        assert!(opens, "`:` must open palette from {label}");
        report.push_str(&format!("  {label:<16} : => {action:?}\n"));
    }
    write_evidence(
        "07-colon-binding.txt",
        "Issue #23: `:` resolves to Command(Open) from every normal view, \
including the newly-bound home and resource-detail views:",
        &[report],
    );
}

fn normal_context(workspace: Workspace, detail_open: bool) -> Context {
    Context {
        hotkeys: nuvem::config::hotkeys::KeyOverrides::empty(),
        mode: Mode::Normal,
        workspace,
        resources_detail_open: detail_open,
        resources_drilldown_open: false,
        resources_drilldown_opens_another_level: false,
        service_filter_open: false,
        service_sidebar_visible: true,
        service_navigator_focused: false,
        resources_filter_open: false,
        configurations_filter_open: false,
        configurations_filter_draft: String::new(),
        configuration_create_open: false,
        configuration_context_edit_open: false,
        configuration_activation_confirmation_open: false,
        configuration_activation_confirmation_preview_visible: false,
        configuration_context_picker_open: false,
        configuration_context_picker_filter_open: false,
        configuration_context_picker_filter_draft: String::new(),
        command_palette_open: false,
        resource_action_confirmation_open: false,
        resource_action_confirmation_preview_visible: false,
        resource_command_preview_open: false,
        resource_command_preview_visible: false,
        ssh_launch_preview_open: false,
        ssh_launch_preview_visible: false,
        tunnel_launch_preview_open: false,
        tunnel_launch_preview_visible: false,
        logging_filter_open: false,
        logging_detail_open: false,
        logging_filter_draft: String::new(),
        k9s_launch_confirmation_open: false,
        k9s_launch_confirmation_preview_visible: false,
        resource_jump_picker_open: false,
        resource_batch_confirmation_open: false,
        resource_batch_confirmation_visible: false,
        resource_batch_ack_satisfied: false,
        plugin_confirmation_open: false,
        plugin_confirmation_ack_satisfied: false,
        plugin_confirmation_visible: false,
        plugin_prompt_open: false,
        plugin_prompt_satisfied: false,
        plugin_prompt_visible: false,
        plugin_output_open: false,
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

// Keep `reduce` import used even if unused above in some builds.
#[allow(dead_code)]
fn _touch() {
    let mut app = AppState::new();
    let _ = reduce(&mut app, &Action::Command(CommandAction::Open));
}

// ---------------------------------------------------------------------------
// Surfacing read-only in the palette (greyed, not hidden).
// ---------------------------------------------------------------------------

/// A blocked row (a mutating command, under `read_only`) carries the trailing
/// `— read-only` textual marker and is rendered dimmed; an unaffected row
/// (a non-mutating command) carries neither. The modal title also announces
/// the mode.
///
/// The reducer's `Submit` gate (Task 3) is the enforcement; this is only the
/// explanation, so the row must stay present and selectable — never hidden.
/// A session on the GCE list with one stopped instance, read-only on, palette
/// open — the smallest state in which a blocked row is on screen.
async fn read_only_session_with_palette_open() -> RuntimeSession {
    let mut session = RuntimeSession::new_for_test(AppState::new());
    load_dev(&mut session).await;
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
                status: ResourceStatus::Stopped,
                machine_type: "e2-medium".to_string(),
                internal_ip: Some("10.0.0.2".to_string()),
                external_ip: None,
            }],
        })
        .await;
    session.app_mut().root.read_only = true;
    open(&mut session).await;
    session
}

#[tokio::test]
async fn read_only_palette_marks_blocked_rows_and_leaves_others_plain() {
    let session = read_only_session_with_palette_open().await;
    let lines = render_lines(&session, 100, 28);

    // Blocked: a mutating command (Start boots the stopped instance).
    let start_line = lines
        .iter()
        .find(|line| line.contains("Start selected GCE instance"))
        .expect("Start row is present, never hidden");
    assert!(
        start_line.contains("— read-only"),
        "a blocked row must carry the read-only marker: {start_line:?}"
    );

    // Unaffected: a non-mutating navigation command.
    let home_line = lines
        .iter()
        .find(|line| line.contains("Home") && !line.contains("Start"))
        .expect("Home row is present");
    assert!(
        !home_line.contains("— read-only"),
        "a non-mutating row must not carry the read-only marker: {home_line:?}"
    );

    // The modal title announces the mode.
    assert!(
        lines
            .iter()
            .any(|line| line.contains("commands (read-only)"))
    );

    write_evidence(
        "08-palette-read-only-blocked-row.txt",
        "The `:` palette under --read-only. The blocked `Start selected GCE \
instance` row stays present and selectable, carrying a trailing `— read-only` \
marker and dimmed styling; unaffected rows (e.g. `Home`) carry neither. The \
modal title reads `commands (read-only)`.",
        &lines,
    );
}

/// The marker is the load-bearing half of the explanation, so it has to
/// survive the width the label does not. Appending it before truncation loses
/// it first: at ~72 columns the row would render as `Start selected GCE ins...`
/// with only the dim left — the exact "dimmed row with no reason" failure the
/// renderer's own comment calls unacceptable.
///
/// EVERY blocked row on screen is checked, the longest one included. Asserting
/// that *some* row kept its marker passes for a fix that only reserves width
/// for the short labels — and the long labels are precisely the ones the
/// marker is squeezed out of.
#[tokio::test]
async fn the_read_only_marker_survives_a_narrow_terminal() {
    let session = read_only_session_with_palette_open().await;
    let root = &session.app().root;
    // The oracle is the renderer's own predicate, not a re-derivation of it:
    // a copy here would agree with the renderer only until one of them is
    // edited, which is the drift `command_blocked_by_read_only` exists to
    // close. `is_mutating` alone would be wrong outright — a
    // plugin row's answer comes from its declaration, so it would name a
    // reading plugin as blocked and fail here with a truncation message about
    // a row that was never meant to be marked.
    let blocked: Vec<String> = root
        .command
        .visible_commands(root)
        .into_iter()
        .filter(|command| nuvem::app::state::command::command_blocked_by_read_only(*command, root))
        .map(|command| nuvem::app::state::command::palette_display_label(command, root))
        .collect();
    assert!(
        blocked.len() > 1,
        "the scenario needs more than one blocked row for 'every' to mean \
         anything: {blocked:?}"
    );
    let longest = blocked
        .iter()
        .max_by_key(|label| label.chars().count())
        .expect("at least one blocked row")
        .clone();

    for width in [60, 72, 84] {
        let lines = render_lines(&session, width, 28);
        for label in &blocked {
            // A prefix short enough to survive truncation at the narrowest
            // width, which is how the row is located once its tail is gone.
            let key: String = label.chars().take(12).collect();
            let row = lines
                .iter()
                .find(|line| line.contains(&key))
                .unwrap_or_else(|| {
                    panic!("blocked row {label:?} is missing at {width} columns: {lines:?}")
                });
            assert!(
                row.contains("— read-only"),
                "at {width} columns the blocked row {label:?} lost its marker \
                 to truncation, leaving a dimmed row with no reason on it: {row:?}"
            );
        }
    }

    // And the squeeze is real at the narrowest width: the longest blocked row
    // is truncated there, so the loop above is not passing because everything
    // happened to fit.
    let narrow = render_lines(&session, 60, 28);
    let key: String = longest.chars().take(12).collect();
    let row = narrow
        .iter()
        .find(|line| line.contains(&key))
        .expect("longest blocked row is on screen");
    assert!(
        !row.contains(longest.as_str()),
        "the longest blocked label {longest:?} still fits whole at 60 columns, \
         so this test is not exercising the truncation it is about: {row:?}"
    );
}

/// The evidence prose claims the blocked row is dimmed. Collecting only
/// `cell.symbol()` proves nothing about that, so deleting the `Line::styled`
/// branch would leave the whole suite green while the certified behaviour
/// quietly disappeared.
#[tokio::test]
async fn a_blocked_row_is_actually_dimmed_in_the_buffer() {
    let session = read_only_session_with_palette_open().await;
    let buffer = render_buffer(&session, 100, 28);
    let lines = buffer_lines(&buffer);

    let y = lines
        .iter()
        .position(|line| line.contains("Start selected GCE instance"))
        .expect("the blocked Start row is present, never hidden");
    let row = &lines[y];
    let start = column_of(row, "Start").expect("label column located");
    let end =
        column_of(row, "— read-only").expect("marker present") + "— read-only".chars().count();

    for x in start..end {
        let cell = &buffer[(x as u16, y as u16)];
        assert!(
            cell.style().add_modifier.contains(Modifier::DIM),
            "blocked row cell {x} ({:?}) must carry DIM: the marker and the dim \
             are both promised, so both have to be checked",
            cell.symbol()
        );
    }

    // And an unaffected row is not dimmed, so the assertion above is about
    // blocked rows rather than about the whole modal.
    let home_y = lines
        .iter()
        .position(|line| line.contains("Home") && !line.contains("Start"))
        .expect("the Home row is present");
    let home_x = column_of(&lines[home_y], "Home").expect("Home label located");
    assert!(
        !buffer[(home_x as u16, home_y as u16)]
            .style()
            .add_modifier
            .contains(Modifier::DIM),
        "an available row must not be dimmed"
    );
}

/// Submitting the blocked row end-to-end: the palette closes without opening
/// a confirmation, and the status line explains the refusal. This is the
/// operator-visible half of the reducer gate — the render proves the refusal
/// actually reaches the screen instead of living only in `status_message`.
#[tokio::test]
async fn submitting_a_blocked_row_renders_the_refusal_in_the_status_line() {
    let mut session = read_only_session_with_palette_open().await;
    // The palette opens with the blocked `Start selected GCE instance` row
    // selected (the stopped instance's first offer), so Submit hits the gate.
    assert_eq!(
        session
            .app()
            .root
            .command
            .selected_visible_command(&session.app().root)
            .map(|c| c.label().to_string()),
        Some("Start selected GCE instance".to_string()),
        "the scenario submits the blocked Start row"
    );
    session
        .dispatch_action(Action::Command(CommandAction::Submit))
        .await;

    assert!(
        session.app().root.resources.action_confirmation().is_none(),
        "read-only must refuse the submit instead of opening a confirmation"
    );
    let lines = render_lines(&session, 100, 28);
    let status = lines
        .iter()
        .find(|line| line.contains("unavailable: read-only"))
        .expect("the refusal must be visible on screen");
    assert!(
        status.contains("Start selected GCE instance is unavailable: read-only mode refuses"),
        "the refusal names the command the operator submitted: {status:?}"
    );

    write_evidence(
        "09-palette-read-only-refusal-status-line.txt",
        "Enter on the blocked `Start selected GCE instance` row under \
--read-only. The palette closes, no confirmation opens, and the status line \
explains: `Start selected GCE instance is unavailable: read-only mode refuses \
cloud-resource mutations`.",
        &lines,
    );
}

/// Same scenario, mode off: no row anywhere carries the marker, and the title
/// is the plain `commands`.
#[tokio::test]
async fn read_only_off_palette_carries_no_marker_anywhere() {
    let mut session = RuntimeSession::new_for_test(AppState::new());
    load_dev(&mut session).await;
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
                status: ResourceStatus::Stopped,
                machine_type: "e2-medium".to_string(),
                internal_ip: Some("10.0.0.2".to_string()),
                external_ip: None,
            }],
        })
        .await;
    assert!(!session.app().root.read_only, "mode is off by default");
    open(&mut session).await;
    let lines = render_lines(&session, 100, 28);

    assert!(
        !lines.iter().any(|line| line.contains("— read-only")),
        "no row may carry the marker when the mode is off: {lines:?}"
    );
    assert!(lines.iter().any(|line| line.contains("commands")));
    assert!(
        !lines
            .iter()
            .any(|line| line.contains("commands (read-only)"))
    );
}

/// Regression pin: a blocked command stays in `visible_commands` under
/// `read_only` (shown, not hidden). This is the assertion that fails if
/// someone "fixes" the read-only surfacing by filtering `context_commands`
/// instead of styling the row — the reducer gate from Task 3 is the real
/// enforcement, and it does not touch `visible_commands`.
#[test]
fn blocked_commands_stay_in_visible_commands_under_read_only() {
    use nuvem::app::state::command::CommandDescriptor;
    use nuvem_domain::gcp::resource::ResourceStatus as Status;

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
    let context = app.root.current_context().unwrap();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ComputeInstancesLoaded {
            context,
            instances: vec![ComputeInstance {
                id: "api-dev-1-id".to_string(),
                name: "api-dev-1".to_string(),
                zone: "europe-west1-b".to_string(),
                status: Status::Stopped,
                machine_type: "e2-medium".to_string(),
                internal_ip: Some("10.0.0.2".to_string()),
                external_ip: None,
            }],
        }),
    );
    reduce(&mut app, &Action::Workspace(WorkspaceAction::Resources));
    app.root.read_only = true;

    let commands = app.root.command.visible_commands(&app.root);
    assert!(
        commands.contains(&CommandDescriptor::StartSelectedComputeInstance),
        "read_only must not hide a mutating command from the palette: {commands:?}"
    );
}

/// The palette shows what you typed while the search is *working*, not only
/// once it has failed.
///
/// The query used to reach the screen in exactly one case — the empty-state
/// message pinned by `evidence_palette_empty_state` above. While the search was
/// matching, nothing on screen said what had been typed, so the operator could
/// not see their own input, could not tell a swallowed keystroke from a typo,
/// and could not tell why a command they expected was missing.
///
/// Every other filterable surface in the app shows its draft with a trailing
/// `_` cursor: the resource filter (`Filter: dev_`), the configurations filter,
/// the service-navigator filter, the context picker. The palette was the one
/// that did not.
#[tokio::test]
async fn the_palette_shows_the_query_while_it_is_still_matching() {
    let mut session = RuntimeSession::new_for_test(AppState::new());
    load_dev(&mut session).await;
    open(&mut session).await;
    type_query(&mut session, "conf").await;
    let lines = render_lines(&session, 100, 26);

    // The premise: this query matches, so the empty-state message — the one
    // place that already named the query — is not what is being read here.
    assert!(
        !lines.iter().any(|line| line.contains("No commands match")),
        "this query must match, or the test is about the empty state; got: {lines:#?}"
    );

    assert!(
        lines.iter().any(|line| line.contains("conf_")),
        "the palette must show the draft and its cursor while matching; got: {lines:#?}"
    );
    write_evidence(
        "07-palette-query-row.txt",
        "The typed query is on screen while the search is matching, \
carrying the same trailing `_` cursor every other filter in the app uses.",
        &lines,
    );
}

/// The palette's box stays inside the terminal at every height the app draws on.
///
/// The box takes the full height of the body it clears, between the header
/// and the status line, so its height is a function of the frame's layout
/// rather than a percentage of the terminal. This is what keeps that honest at
/// every height, including the short ones where the body is a row or two.
#[tokio::test]
async fn the_palette_box_never_escapes_the_frame() {
    let mut session = RuntimeSession::new_for_test(AppState::new());
    load_dev(&mut session).await;
    open(&mut session).await;

    // Every height the box's arithmetic can behave differently at, and three
    // widths rather than four.
    //
    // The sweep was 1..=60 across four widths — 240 full-frame renders, the
    // widest at 200x60. That was 2.4s idle and over the suite's ten-second
    // per-test budget under the load `cargo mutants` puts on the machine while
    // re-running the suite per mutant, which is where it timed out. The box's
    // height is a percentage of the frame plus one row, so above forty rows
    // there is nothing new to see, and width only decides whether the palette
    // opens at all — it is the *height* this property is about.
    for height in 1u16..=40 {
        for width in [20u16, 60, 100] {
            let lines = render_lines(&session, width, height);
            // The box's own borders, read off the frame. A box that ran past
            // the bottom would keep its opening border and lose its closing
            // one, which is exactly what a missing clamp looks like on screen.
            let opens = lines.iter().any(|line| line.contains("┌commands"));
            if !opens {
                // Too small to draw the palette at all; nothing to claim.
                continue;
            }
            assert!(
                lines.iter().any(|line| line.contains('└')),
                "the palette opened but its bottom border is off the frame at \
                 {width}x{height}: {lines:#?}"
            );
        }
    }
}
