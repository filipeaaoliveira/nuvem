use std::time::Instant;

use nuvem::app::AppState;
use nuvem::app::reducer::root::reduce;
use nuvem::app::state::tunnels::{TunnelEntry, TunnelStatus};
use nuvem::app::{Action, Effect, EffectResult, TunnelsAction};
use nuvem::services::runtime::tunnel::TunnelId;
use nuvem_domain::gcp::configuration::{GcloudConfiguration, GcloudConfigurationName};
use nuvem_domain::gcp::tunnel::TunnelKind;

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

/// Seeds one active configuration so `current_context()` is `Some`, mirroring
/// the seeding pattern in `reducer_logging_test.rs`.
fn app_with_active_config() -> AppState {
    let mut app = AppState::new();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ConfigurationsLoaded {
            configurations: vec![config("dev", true, "dev-project")],
        }),
    );
    app
}

/// Inserts one `Starting` tunnel and returns its id. `started_at` is supplied
/// here rather than read from a clock inside the reducer, so the transitions
/// under test stay deterministic.
fn start_tunnel(app: &mut AppState, target: &str, local_port: u16) -> TunnelId {
    let context = app.root.current_context().expect("an active context");
    let id = app.root.tunnels.mint_id();
    app.root.tunnels.insert_starting(TunnelEntry {
        id,
        kind: TunnelKind::IapTcp,
        target_label: target.to_string(),
        local_port,
        remote_label: "22".to_string(),
        status: TunnelStatus::Starting,
        started_at: Instant::now(),
        context,
        message: None,
    });
    id
}

fn status(app: &AppState, id: TunnelId) -> TunnelStatus {
    app.root
        .tunnels
        .entries()
        .iter()
        .find(|entry| entry.id == id)
        .map(|entry| entry.status.clone())
        .expect("the tunnel is still in the list")
}

fn message(app: &AppState, id: TunnelId) -> Option<String> {
    app.root
        .tunnels
        .entries()
        .iter()
        .find(|entry| entry.id == id)
        .map(|entry| entry.message.clone())
        .expect("the tunnel is still in the list")
}

fn result(app: &mut AppState, result: EffectResult) -> Vec<Effect> {
    reduce(app, &Action::EffectResult(result))
}

fn act(app: &mut AppState, action: TunnelsAction) -> Vec<Effect> {
    reduce(app, &Action::Tunnels(action))
}

// --- selection ---------------------------------------------------------

#[test]
fn select_next_and_previous_move_within_the_list() {
    let mut app = app_with_active_config();
    start_tunnel(&mut app, "web-1", 8022);
    start_tunnel(&mut app, "web-2", 8023);

    assert_eq!(app.root.tunnels.selected_index(), 0);
    assert!(act(&mut app, TunnelsAction::SelectNext).is_empty());
    assert_eq!(app.root.tunnels.selected_index(), 1);
    act(&mut app, TunnelsAction::SelectNext);
    assert_eq!(app.root.tunnels.selected_index(), 1, "no wrap past the end");
    act(&mut app, TunnelsAction::SelectPrevious);
    assert_eq!(app.root.tunnels.selected_index(), 0);
}

#[test]
fn selection_actions_on_an_empty_list_do_nothing() {
    let mut app = app_with_active_config();
    assert!(act(&mut app, TunnelsAction::SelectNext).is_empty());
    assert!(act(&mut app, TunnelsAction::SelectPrevious).is_empty());
    assert_eq!(app.root.tunnels.selected_index(), 0);
    assert!(app.root.tunnels.is_empty());
}

// --- kill --------------------------------------------------------------

#[test]
fn killing_a_starting_tunnel_marks_it_stopping_and_asks_the_runtime_to_stop_it() {
    let mut app = app_with_active_config();
    let id = start_tunnel(&mut app, "web-1", 8022);

    let effects = act(&mut app, TunnelsAction::KillSelected);

    assert_eq!(status(&app, id), TunnelStatus::Stopping);
    assert_eq!(effects, vec![Effect::StopTunnel { id }]);
}

#[test]
fn killing_an_active_tunnel_marks_it_stopping_and_asks_the_runtime_to_stop_it() {
    let mut app = app_with_active_config();
    let id = start_tunnel(&mut app, "web-1", 8022);
    result(
        &mut app,
        EffectResult::TunnelReady {
            id,
            local_port: None,
        },
    );

    let effects = act(&mut app, TunnelsAction::KillSelected);

    assert_eq!(status(&app, id), TunnelStatus::Stopping);
    assert_eq!(effects, vec![Effect::StopTunnel { id }]);
}

#[test]
fn killing_a_tunnel_that_is_already_stopping_does_not_ask_again() {
    let mut app = app_with_active_config();
    let id = start_tunnel(&mut app, "web-1", 8022);
    act(&mut app, TunnelsAction::KillSelected);

    let effects = act(&mut app, TunnelsAction::KillSelected);

    assert_eq!(status(&app, id), TunnelStatus::Stopping);
    assert!(effects.is_empty(), "the stop is already in flight");
}

#[test]
fn killing_a_terminal_tunnel_is_a_no_op() {
    let mut app = app_with_active_config();
    let id = start_tunnel(&mut app, "web-1", 8022);
    result(&mut app, EffectResult::TunnelStopped { id });

    let effects = act(&mut app, TunnelsAction::KillSelected);

    assert_eq!(status(&app, id), TunnelStatus::Stopped);
    assert!(effects.is_empty());
}

#[test]
fn killing_with_nothing_selected_is_a_no_op() {
    let mut app = app_with_active_config();
    assert!(act(&mut app, TunnelsAction::KillSelected).is_empty());
}

// --- dismiss -----------------------------------------------------------

#[test]
fn dismissing_a_stopped_tunnel_removes_the_row() {
    let mut app = app_with_active_config();
    let id = start_tunnel(&mut app, "web-1", 8022);
    result(&mut app, EffectResult::TunnelStopped { id });

    let effects = act(&mut app, TunnelsAction::DismissSelected);

    assert!(app.root.tunnels.is_empty());
    assert!(effects.is_empty());
}

#[test]
fn dismissing_a_failed_tunnel_removes_the_row() {
    let mut app = app_with_active_config();
    let id = start_tunnel(&mut app, "web-1", 8022);
    result(
        &mut app,
        EffectResult::TunnelFailed {
            id,
            message: "address already in use".to_string(),
        },
    );

    act(&mut app, TunnelsAction::DismissSelected);

    assert!(app.root.tunnels.is_empty());
}

#[test]
fn dismissing_a_live_tunnel_keeps_it_and_says_to_kill_it_first() {
    let mut app = app_with_active_config();
    let id = start_tunnel(&mut app, "web-1", 8022);
    result(
        &mut app,
        EffectResult::TunnelReady {
            id,
            local_port: None,
        },
    );

    let effects = act(&mut app, TunnelsAction::DismissSelected);

    assert_eq!(status(&app, id), TunnelStatus::Active);
    assert!(effects.is_empty());
    let message = app
        .root
        .system
        .status_message
        .as_deref()
        .expect("a status message explaining why nothing happened");
    assert!(
        message.to_lowercase().contains("kill"),
        "expected the message to tell the user to kill it first, got {message:?}"
    );
}

#[test]
fn dismissing_with_nothing_selected_is_a_no_op() {
    let mut app = app_with_active_config();
    assert!(act(&mut app, TunnelsAction::DismissSelected).is_empty());
    assert!(app.root.tunnels.is_empty());
}

// --- results -----------------------------------------------------------

#[test]
fn tunnel_ready_activates_and_adopts_the_reported_port() {
    let mut app = app_with_active_config();
    let id = start_tunnel(&mut app, "web-1", 8022);

    result(
        &mut app,
        EffectResult::TunnelReady {
            id,
            local_port: Some(8023),
        },
    );

    assert_eq!(status(&app, id), TunnelStatus::Active);
    assert_eq!(app.root.tunnels.entries()[0].local_port, 8023);
}

#[test]
fn tunnel_ready_without_a_reported_port_keeps_the_requested_one() {
    let mut app = app_with_active_config();
    let id = start_tunnel(&mut app, "web-1", 8022);

    result(
        &mut app,
        EffectResult::TunnelReady {
            id,
            local_port: None,
        },
    );

    assert_eq!(app.root.tunnels.entries()[0].local_port, 8022);
}

#[test]
fn the_allocated_port_is_recorded_without_making_the_tunnel_active() {
    let mut app = app_with_active_config();
    let id = start_tunnel(&mut app, "web-1", 0);

    result(
        &mut app,
        EffectResult::TunnelPortAllocated {
            id,
            local_port: 8100,
        },
    );

    assert_eq!(app.root.tunnels.entries()[0].local_port, 8100);
    assert_eq!(status(&app, id), TunnelStatus::Starting);
    assert_eq!(app.root.tunnels.claimed_ports(), vec![8100]);
}

#[test]
fn the_port_the_child_reports_overrides_the_allocated_one() {
    let mut app = app_with_active_config();
    let id = start_tunnel(&mut app, "web-1", 0);
    result(
        &mut app,
        EffectResult::TunnelPortAllocated {
            id,
            local_port: 8100,
        },
    );

    result(
        &mut app,
        EffectResult::TunnelReady {
            id,
            local_port: Some(8101),
        },
    );

    assert_eq!(
        app.root.tunnels.entries()[0].local_port,
        8101,
        "the port the child actually bound wins over the one it was asked for"
    );
}

#[test]
fn tunnel_failed_records_the_message() {
    let mut app = app_with_active_config();
    let id = start_tunnel(&mut app, "web-1", 8022);

    result(
        &mut app,
        EffectResult::TunnelFailed {
            id,
            message: "ERROR: denied by IAP".to_string(),
        },
    );

    assert_eq!(
        status(&app, id),
        TunnelStatus::Failed {
            message: "ERROR: denied by IAP".to_string()
        }
    );
}

/// A notice is diagnostics, not a verdict: the supervisor sends it for a line
/// that looked alarming on a tunnel that is still carrying traffic, so the row
/// has to show the line without the status moving anywhere.
#[test]
fn a_tunnel_notice_shows_the_message_without_changing_the_status() {
    let mut app = app_with_active_config();
    let id = start_tunnel(&mut app, "web-1", 8022);
    result(
        &mut app,
        EffectResult::TunnelReady {
            id,
            local_port: None,
        },
    );

    result(
        &mut app,
        EffectResult::TunnelNotice {
            id,
            message: "channel 3: open failed: connect failed: Connection refused".to_string(),
        },
    );

    assert_eq!(
        status(&app, id),
        TunnelStatus::Active,
        "a notice must never fail a working tunnel"
    );
    assert_eq!(
        message(&app, id).as_deref(),
        Some("channel 3: open failed: connect failed: Connection refused")
    );
}

#[test]
fn a_tunnel_notice_for_an_unknown_id_is_a_no_op() {
    let mut app = app_with_active_config();
    start_tunnel(&mut app, "web-1", 8022);
    let before = app.root.tunnels.clone();

    result(
        &mut app,
        EffectResult::TunnelNotice {
            id: TunnelId(9999),
            message: "boom".to_string(),
        },
    );

    assert_eq!(app.root.tunnels, before);
}

/// The same terminal-is-terminal rule every other late result obeys: a notice
/// from the supervisor can land after the row has already reached its verdict,
/// and it must not overwrite the failure reason the row is being kept for.
#[test]
fn a_tunnel_notice_does_not_overwrite_a_terminal_rows_reason() {
    let mut app = app_with_active_config();
    let id = start_tunnel(&mut app, "web-1", 8022);
    result(
        &mut app,
        EffectResult::TunnelFailed {
            id,
            message: "address already in use".to_string(),
        },
    );

    result(
        &mut app,
        EffectResult::TunnelNotice {
            id,
            message: "late chatter".to_string(),
        },
    );

    assert_eq!(
        status(&app, id),
        TunnelStatus::Failed {
            message: "address already in use".to_string()
        }
    );
    assert_eq!(message(&app, id).as_deref(), Some("address already in use"));
}

#[test]
fn a_tunnel_that_exits_while_starting_failed_before_becoming_ready() {
    let mut app = app_with_active_config();
    let id = start_tunnel(&mut app, "web-1", 8022);

    result(&mut app, EffectResult::TunnelExited { id });

    assert_eq!(
        status(&app, id),
        TunnelStatus::Failed {
            message: "exited before becoming ready".to_string()
        }
    );
}

#[test]
fn a_tunnel_that_exits_while_active_is_simply_stopped() {
    let mut app = app_with_active_config();
    let id = start_tunnel(&mut app, "web-1", 8022);
    result(
        &mut app,
        EffectResult::TunnelReady {
            id,
            local_port: None,
        },
    );

    result(&mut app, EffectResult::TunnelExited { id });

    assert_eq!(status(&app, id), TunnelStatus::Stopped);
}

#[test]
fn a_tunnel_that_exits_while_stopping_is_stopped() {
    let mut app = app_with_active_config();
    let id = start_tunnel(&mut app, "web-1", 8022);
    act(&mut app, TunnelsAction::KillSelected);

    result(&mut app, EffectResult::TunnelExited { id });

    assert_eq!(status(&app, id), TunnelStatus::Stopped);
}

#[test]
fn an_exit_after_a_failure_does_not_overwrite_the_failure_message() {
    let mut app = app_with_active_config();
    let id = start_tunnel(&mut app, "web-1", 8022);
    result(
        &mut app,
        EffectResult::TunnelFailed {
            id,
            message: "address already in use".to_string(),
        },
    );

    result(&mut app, EffectResult::TunnelExited { id });

    assert_eq!(
        status(&app, id),
        TunnelStatus::Failed {
            message: "address already in use".to_string()
        },
        "the child's own reason outlives the bare exit notification"
    );
}

#[test]
fn tunnel_stopped_marks_the_entry_stopped() {
    let mut app = app_with_active_config();
    let id = start_tunnel(&mut app, "web-1", 8022);

    result(&mut app, EffectResult::TunnelStopped { id });

    assert_eq!(status(&app, id), TunnelStatus::Stopped);
}

/// The supervisor task is not the task that reaped the child, so a `TunnelReady`
/// it sent just before the child died can land after the row is already
/// terminal. Resurrecting the row would re-promise its port to the allocator
/// forever and make it undismissable.
#[test]
fn a_late_ready_does_not_resurrect_a_stopped_tunnel() {
    let mut app = app_with_active_config();
    let id = start_tunnel(&mut app, "web-1", 8022);
    result(&mut app, EffectResult::TunnelStopped { id });

    result(
        &mut app,
        EffectResult::TunnelReady {
            id,
            local_port: Some(9000),
        },
    );

    assert_eq!(status(&app, id), TunnelStatus::Stopped);
    assert!(
        app.root.tunnels.claimed_ports().is_empty(),
        "a terminal row must not re-claim its port"
    );
}

#[test]
fn a_late_failure_does_not_reopen_a_stopped_tunnel() {
    let mut app = app_with_active_config();
    let id = start_tunnel(&mut app, "web-1", 8022);
    result(&mut app, EffectResult::TunnelStopped { id });

    result(
        &mut app,
        EffectResult::TunnelFailed {
            id,
            message: "late boom".to_string(),
        },
    );

    assert_eq!(status(&app, id), TunnelStatus::Stopped);
}

/// Killing a `Starting` tunnel whose stderr had already queued a `TunnelFailed`
/// ends with two results in flight for the same row. The stop acknowledgement
/// lands second, and must not erase the reason the row is being kept for.
#[test]
fn a_late_stop_does_not_erase_the_failure_reason() {
    let mut app = app_with_active_config();
    let id = start_tunnel(&mut app, "web-1", 8022);
    result(
        &mut app,
        EffectResult::TunnelFailed {
            id,
            message: "ERROR: denied by IAP".to_string(),
        },
    );

    result(&mut app, EffectResult::TunnelStopped { id });

    assert_eq!(
        status(&app, id),
        TunnelStatus::Failed {
            message: "ERROR: denied by IAP".to_string()
        },
        "a stop acknowledgement must not overwrite a row that already failed"
    );
}

/// `Stopping` is *not* terminal, so moving a terminal row into it would
/// re-claim its port for the allocator and make it undismissable. Nothing
/// reaches this today — `KillSelected` guards on `Starting`/`Active` — so the
/// guard is asserted on the state method directly rather than through an action
/// that cannot express it.
#[test]
fn a_terminal_row_cannot_be_moved_back_into_stopping() {
    let mut app = app_with_active_config();
    let id = start_tunnel(&mut app, "web-1", 8022);
    result(&mut app, EffectResult::TunnelStopped { id });

    app.root.tunnels.mark_stopping(id);

    assert_eq!(status(&app, id), TunnelStatus::Stopped);
    assert!(
        app.root.tunnels.claimed_ports().is_empty(),
        "a terminal row must not re-claim its port"
    );
}

#[test]
fn every_result_for_an_unknown_id_is_a_silent_no_op() {
    let mut app = app_with_active_config();
    let id = start_tunnel(&mut app, "web-1", 8022);
    let unknown = TunnelId(9999);
    let before = app.root.tunnels.clone();

    for effect_result in [
        EffectResult::TunnelReady {
            id: unknown,
            local_port: Some(1234),
        },
        EffectResult::TunnelFailed {
            id: unknown,
            message: "boom".to_string(),
        },
        EffectResult::TunnelExited { id: unknown },
        EffectResult::TunnelStopped { id: unknown },
    ] {
        assert!(result(&mut app, effect_result).is_empty());
    }

    assert_eq!(app.root.tunnels, before);
    assert_eq!(status(&app, id), TunnelStatus::Starting);
}

// --- context scoping ---------------------------------------------------

#[test]
fn tunnels_survive_a_context_switch_and_keep_the_context_they_started_under() {
    // Deliberate divergence from resource state: the child process keeps
    // running across a configuration change, so clearing the list would hide a
    // live process (and lose the only handle that can stop it).
    let mut app = app_with_active_config();
    let id = start_tunnel(&mut app, "web-1", 8022);
    result(
        &mut app,
        EffectResult::TunnelReady {
            id,
            local_port: None,
        },
    );

    result(
        &mut app,
        EffectResult::ConfigurationActivated {
            configuration: config("prod", true, "prod-project"),
        },
    );

    assert_eq!(app.root.tunnels.entries().len(), 1);
    assert_eq!(status(&app, id), TunnelStatus::Active);
    assert_eq!(
        app.root.tunnels.entries()[0].context.project.as_deref(),
        Some("dev-project"),
        "the entry keeps the context it was started under"
    );
}
