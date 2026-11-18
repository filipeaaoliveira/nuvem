use nuvem::app::AppState;
use nuvem::app::reducer::root::reduce;
use nuvem::app::state::logging::{LoggingStatus, TailState};
use nuvem::app::state::root::Workspace;
use nuvem::app::state::system::OperationPhase;
use nuvem::app::{
    Action, CommandAction, Effect, EffectResult, LoggingAction, NavAction, ResourcesAction,
    WorkspaceAction,
};
use nuvem_domain::gcp::configuration::{GcloudConfiguration, GcloudConfigurationName};
use nuvem_domain::gcp::logging::{LogSeverity, LogTimeRange};

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
/// the `ConfigurationsLoaded` seeding pattern in `reducer_resources_test.rs`.
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

#[test]
fn entering_logging_workspace_requests_recent_logs() {
    let mut app = app_with_active_config();
    let effects = reduce(&mut app, &Action::Workspace(WorkspaceAction::Logging));
    assert_eq!(app.root.workspace, Workspace::Logging);
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, Effect::LoadRecentLogs { .. }))
    );
}

/// The severity floor is a SERVER-side narrowing: the effect must carry the
/// value currently in state (set directly here -- the actions that change it
/// land later) rather than hardcoding the `Default` sentinel.
#[test]
fn recent_logs_effect_carries_the_state_severity_floor() {
    let mut app = app_with_active_config();
    reduce(&mut app, &Action::Workspace(WorkspaceAction::Logging));
    app.root.logging.severity_floor = LogSeverity::Error;

    let effects = reduce(&mut app, &Action::Logging(LoggingAction::Refresh));

    let severity = effects.iter().find_map(|effect| match effect {
        Effect::LoadRecentLogs { severity, .. } => Some(*severity),
        _ => None,
    });
    assert_eq!(severity, Some(LogSeverity::Error));
}

#[test]
fn toggle_tail_action_flips_state() {
    let mut app = app_with_active_config();
    reduce(&mut app, &Action::Workspace(WorkspaceAction::Logging));
    reduce(&mut app, &Action::Logging(LoggingAction::ToggleTail));
    assert_eq!(app.root.logging.tail, TailState::Paused);
}

#[test]
fn refresh_action_requests_recent_logs() {
    let mut app = app_with_active_config();
    reduce(&mut app, &Action::Workspace(WorkspaceAction::Logging));
    let effects = reduce(&mut app, &Action::Logging(LoggingAction::Refresh));
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, Effect::LoadRecentLogs { .. }))
    );
}

#[test]
fn entering_logging_workspace_with_active_config_sets_loading() {
    let mut app = app_with_active_config();
    reduce(&mut app, &Action::Workspace(WorkspaceAction::Logging));
    assert_eq!(app.root.logging.status, LoggingStatus::Loading);
}

#[test]
fn entering_logging_workspace_without_context_does_not_get_stuck_loading() {
    let mut app = AppState::new();
    assert!(app.root.current_context().is_none());
    let effects = reduce(&mut app, &Action::Workspace(WorkspaceAction::Logging));
    assert_eq!(app.root.workspace, Workspace::Logging);
    assert_ne!(app.root.logging.status, LoggingStatus::Loading);
    assert!(
        !effects
            .iter()
            .any(|e| matches!(e, Effect::LoadRecentLogs { .. }))
    );
}

/// Same invariant as the workspace-entry guard above, for the actions that
/// move the query window: each one clears the buffer and marks it `Loading`,
/// but with no context there is no fetch to clear the spinner and the tail
/// poll skips as well. An action that dispatched no load must not leave the
/// viewer at `Loading`.
#[test]
fn window_changing_actions_without_context_do_not_get_stuck_loading() {
    for action in [
        LoggingAction::RaiseSeverityFloor,
        LoggingAction::LowerSeverityFloor,
        LoggingAction::SetTimeRange(LogTimeRange::SixHours),
        LoggingAction::CycleTimeRange,
    ] {
        let mut app = AppState::new();
        app.root.workspace = Workspace::Logging;
        // A mid-range floor so both the raise and the lower step actually move.
        app.root.logging.severity_floor = LogSeverity::Error;
        assert!(app.root.current_context().is_none());

        let effects = reduce(&mut app, &Action::Logging(action.clone()));

        assert!(
            !effects
                .iter()
                .any(|e| matches!(e, Effect::LoadRecentLogs { .. })),
            "{action:?} cannot load without a context"
        );
        assert_ne!(
            app.root.logging.status,
            LoggingStatus::Loading,
            "{action:?} left the viewer spinning with nothing to clear it"
        );
    }
}

/// Walks real navigation history -- enter Logging, leave for Home, then Back --
/// so the restore fires through the same path a deep link takes.
fn app_navigated_back_into_logging(mut app: AppState) -> (AppState, Vec<Effect>) {
    reduce(&mut app, &Action::Workspace(WorkspaceAction::Logging));
    reduce(&mut app, &Action::Workspace(WorkspaceAction::Home));
    let effects = reduce(&mut app, &Action::Navigation(NavAction::Back));
    assert_eq!(app.root.workspace, Workspace::Logging);
    (app, effects)
}

/// Same invariant as the workspace-entry and window-move guards, for a history
/// restore into Logging: the nav arm marks the buffer `Loading` before asking
/// for the fetch, but with no context there is no fetch and the tail poll skips
/// too, so nothing would ever clear the spinner.
#[test]
fn navigating_back_into_logging_without_context_does_not_get_stuck_loading() {
    let (app, effects) = app_navigated_back_into_logging(AppState::new());

    assert!(app.root.current_context().is_none());
    assert!(
        !effects
            .iter()
            .any(|e| matches!(e, Effect::LoadRecentLogs { .. })),
        "a restore cannot load without a context"
    );
    assert_eq!(
        app.root.logging.status,
        LoggingStatus::Initial,
        "the restore left the viewer spinning with nothing to clear it"
    );
}

/// Settling to `Initial` is only honest over an empty buffer: `Initial` renders
/// as "Not loaded", so leaving rows behind puts a header that says nothing was
/// loaded directly above the rows the operator is reading. The restore path is
/// the one caller that reaches the settle with a populated buffer (the window
/// movers clear it first), so it has to leave the two agreeing.
///
/// The detail pane goes with the rows. It is opened here through the real
/// actions -- select a row, open the detail -- because the restore is reachable
/// with the pane up: nothing closes it on a workspace switch, and the globals
/// that switch workspaces resolve before the Logging dispatch. `detail_open` is
/// read straight off state to route every key press, so a pane left "open" over
/// a dropped buffer is a keyboard mode with nothing on screen behind it: the
/// list keys go dead and `Esc` closes an invisible pane instead of navigating
/// Back.
#[test]
fn navigating_back_into_logging_without_context_does_not_leave_rows_under_not_loaded() {
    let mut app = AppState::new();
    reduce(&mut app, &Action::Workspace(WorkspaceAction::Logging));
    // A buffer loaded while the context still resolved; the context is gone by
    // the time the restore runs.
    app.root
        .logging
        .apply_recent(vec![entry("a", "t1"), entry("b", "t2")], 0);
    reduce(&mut app, &Action::Logging(LoggingAction::SelectNext));
    reduce(&mut app, &Action::Logging(LoggingAction::OpenDetail));
    reduce(&mut app, &Action::Logging(LoggingAction::ScrollDetailDown));
    assert!(
        app.root.logging.detail_open,
        "the scenario needs the detail pane genuinely open before the restore"
    );
    reduce(&mut app, &Action::Workspace(WorkspaceAction::Home));

    let effects = reduce(&mut app, &Action::Navigation(NavAction::Back));

    assert_eq!(app.root.workspace, Workspace::Logging);
    assert!(app.root.current_context().is_none());
    assert!(
        !effects
            .iter()
            .any(|e| matches!(e, Effect::LoadRecentLogs { .. })),
        "a restore cannot load without a context"
    );
    assert_eq!(app.root.logging.status, LoggingStatus::Initial);
    assert!(
        app.root.logging.entries.is_empty(),
        "\"Not loaded\" must not sit above rows the operator can still read"
    );
    assert!(app.root.logging.visible_entries().is_empty());
    assert!(
        app.root.logging.selected.is_none() && app.root.logging.cursor.is_none(),
        "the selection and page cursor are derived from the dropped buffer"
    );
    assert!(
        !app.root.logging.detail_open,
        "a detail pane over a dropped buffer is a keyboard mode with nothing on screen"
    );
    assert_eq!(
        app.root.logging.detail_scroll, 0,
        "the closed pane must not keep the scroll offset of the dropped entry"
    );
}

/// The other half of the guard above: when a load IS dispatched the spinner has
/// something to clear it, so the restore must keep the buffer at `Loading`.
#[test]
fn navigating_back_into_logging_with_context_loads_and_stays_loading() {
    let (app, effects) = app_navigated_back_into_logging(app_with_active_config());

    assert!(
        effects
            .iter()
            .any(|e| matches!(e, Effect::LoadRecentLogs { .. }))
    );
    assert_eq!(app.root.logging.status, LoggingStatus::Loading);
}

fn entry(id: &str, ts: &str) -> nuvem_domain::gcp::logging::LogEntry {
    nuvem_domain::gcp::logging::LogEntry {
        insert_id: id.to_string(),
        timestamp: ts.to_string(),
        severity: LogSeverity::Error,
        resource_type: "gce_instance".to_string(),
        resource_labels: Default::default(),
        log_name: "l".to_string(),
        message: format!("msg-{id}"),
        payload: nuvem_domain::gcp::logging::LogPayload::Empty,
        labels: Default::default(),
    }
}

/// Logging workspace with an active context and a populated buffer.
fn app_in_logging_with_entries() -> AppState {
    let mut app = app_with_active_config();
    reduce(&mut app, &Action::Workspace(WorkspaceAction::Logging));
    app.root
        .logging
        .apply_recent(vec![entry("a", "t1"), entry("b", "t2")], 0);
    app
}

/// Drops the active configuration the way a `gcloud config` reload that no
/// longer lists it would, so `current_context()` stops resolving while the
/// Logging buffer stays exactly as the operator left it.
fn lose_the_context(app: &mut AppState) {
    reduce(
        app,
        &Action::EffectResult(EffectResult::ConfigurationsLoaded {
            configurations: Vec::new(),
        }),
    );
    assert!(app.root.current_context().is_none());
}

fn buffered_insert_ids(app: &AppState) -> Vec<&str> {
    app.root
        .logging
        .entries
        .iter()
        .map(|entry| entry.insert_id.as_str())
        .collect()
}

/// `r` means "get me this again", not "throw away what I have". When the
/// context stops resolving the refetch cannot be dispatched, and the rows
/// already on screen were really fetched -- an operator may be mid-read of the
/// error that sent them here. Clearing them would delete the only copy of that
/// evidence in exchange for nothing.
#[test]
fn refreshing_without_a_context_keeps_the_rows_on_screen() {
    let mut app = app_in_logging_with_entries();
    lose_the_context(&mut app);

    let effects = reduce(&mut app, &Action::Logging(LoggingAction::Refresh));

    assert!(
        !effects
            .iter()
            .any(|e| matches!(e, Effect::LoadRecentLogs { .. })),
        "a refresh cannot load without a context"
    );
    assert_ne!(
        app.root.logging.status,
        LoggingStatus::Loading,
        "the refresh left the viewer spinning with nothing to clear it"
    );
    assert_eq!(
        buffered_insert_ids(&app),
        vec!["a", "b"],
        "a refresh that dispatched nothing emptied the rows the operator was reading"
    );
}

/// Keeping the rows (above) is what makes the refusal invisible: nothing on
/// screen moves, so a refresh that was refused for want of a context looks
/// exactly like one that came back with the same page. Since the buffer
/// deliberately stays put, the status line is the only place left to say why,
/// and it must name the missing context the way `L` already does.
#[test]
fn refreshing_without_a_context_reports_the_missing_context() {
    let mut app = app_in_logging_with_entries();
    lose_the_context(&mut app);
    app.root.system.clear_status_message();

    reduce(&mut app, &Action::Logging(LoggingAction::Refresh));

    assert_eq!(
        app.root.system.status_message.as_deref(),
        Some("No active gcloud context to refresh logs from"),
        "the refresh was refused silently: nothing moved on screen and nothing said why"
    );
    assert_eq!(
        app.root.system.status_phase,
        Some(OperationPhase::Failed),
        "a refusal reported as a healthy status reads as a refresh that succeeded"
    );
    assert_eq!(
        buffered_insert_ids(&app),
        vec!["a", "b"],
        "reporting the refusal cost the operator the rows it was explaining"
    );
}

/// The other half: a refresh that really dispatched its fetch must not claim a
/// missing context. Without this the fix could report the refusal
/// unconditionally and still pass the test above, leaving a failure status over
/// every successful `r`.
#[test]
fn refreshing_with_a_context_reports_no_refusal() {
    let mut app = app_in_logging_with_entries();
    app.root.system.clear_status_message();

    let effects = reduce(&mut app, &Action::Logging(LoggingAction::Refresh));

    assert!(
        effects
            .iter()
            .any(|e| matches!(e, Effect::LoadRecentLogs { .. })),
        "the fetch this test is about was never dispatched"
    );
    assert_eq!(
        app.root.system.status_message, None,
        "a refresh that dispatched its fetch reported a refusal that did not happen"
    );
}

/// The spinner the refresh has to be able to clear: entering the Logging
/// workspace marks a populated buffer `Loading` and asks for the fetch, and the
/// context can stop resolving before that fetch ever lands. With no context the
/// tail poll skips too, so nothing else will clear it -- `r` is the operator's
/// one remaining move, and it must settle the header to what is actually on
/// screen rather than leave a spinner over rows that are already rendered.
#[test]
fn refreshing_a_stranded_spinner_settles_it_to_the_rows_on_screen() {
    let mut app = app_in_logging_with_entries();
    reduce(&mut app, &Action::Workspace(WorkspaceAction::Logging));
    assert_eq!(app.root.logging.status, LoggingStatus::Loading);
    lose_the_context(&mut app);

    let effects = reduce(&mut app, &Action::Logging(LoggingAction::Refresh));

    assert!(
        !effects
            .iter()
            .any(|e| matches!(e, Effect::LoadRecentLogs { .. })),
        "a refresh cannot load without a context"
    );
    assert_eq!(
        app.root.logging.status,
        LoggingStatus::Loaded,
        "the header still claims a fetch is in flight over rows that are already rendered"
    );
    assert_eq!(
        buffered_insert_ids(&app),
        vec!["a", "b"],
        "settling the spinner cost the operator the rows it was spinning over"
    );
}

/// The same settle with nothing rendered underneath: `Loaded` over an empty
/// buffer would advertise a page that does not exist, so an unloadable refresh
/// on an empty viewer reports "not loaded, nothing in flight".
#[test]
fn refreshing_a_stranded_spinner_with_no_rows_settles_to_not_loaded() {
    let mut app = app_with_active_config();
    reduce(&mut app, &Action::Workspace(WorkspaceAction::Logging));
    assert_eq!(app.root.logging.status, LoggingStatus::Loading);
    assert!(app.root.logging.entries.is_empty());
    lose_the_context(&mut app);

    reduce(&mut app, &Action::Logging(LoggingAction::Refresh));

    assert_eq!(
        app.root.logging.status,
        LoggingStatus::Initial,
        "an empty viewer that never loaded must not claim a loaded page"
    );
}

/// Only the spinner is a lie worth overwriting. `HealthyEmpty` is a real
/// last-known outcome -- a fetch ran and the window genuinely held no entries --
/// and a refresh that dispatched nothing learned nothing that contradicts it.
/// Replacing it with "not loaded" would trade a true statement for a vaguer one
/// and hide that the empty view is an answer rather than an absence.
#[test]
fn refreshing_without_a_context_does_not_overwrite_an_honest_status() {
    let mut app = app_with_active_config();
    reduce(&mut app, &Action::Workspace(WorkspaceAction::Logging));
    app.root.logging.apply_recent(Vec::new(), 0);
    assert_eq!(app.root.logging.status, LoggingStatus::HealthyEmpty);
    lose_the_context(&mut app);

    reduce(&mut app, &Action::Logging(LoggingAction::Refresh));

    assert_eq!(
        app.root.logging.status,
        LoggingStatus::HealthyEmpty,
        "a refresh that learned nothing downgraded a fetch that found no entries to \"not loaded\""
    );
}

/// The palette's "Open project logs" marks the buffer `Loading` before asking
/// for the fetch, exactly like the workspace-entry and window-move paths, so it
/// carries the same obligation: a viewer left at `Loading` with no load
/// dispatched spins forever, because the tail poll skips for the same missing
/// context.
///
/// Both halves of that obligation are checked here. With a context the spinner
/// is honest -- a load really was dispatched. With no context the command is not
/// offered at all, which is what keeps the spinner out of reach today; the
/// routing behind it is the second lock, for whenever that listing rule changes.
#[test]
fn opening_project_logs_never_spins_without_a_dispatched_load() {
    let mut app = app_with_active_config();
    let effects = submit_palette_command(&mut app, "Open project logs");

    assert_eq!(app.root.workspace, Workspace::Logging);
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, Effect::LoadRecentLogs { .. })),
        "the palette command must load the logs it just switched to"
    );
    assert_eq!(
        app.root.logging.status,
        LoggingStatus::Loading,
        "a dispatched load must leave the spinner up until its page lands"
    );

    let mut app = AppState::new();
    assert!(app.root.current_context().is_none());
    let effects = submit_palette_command(&mut app, "Open project logs");

    assert!(
        effects.is_empty(),
        "unexpected effects with no context: {effects:?}"
    );
    assert_ne!(
        app.root.logging.status,
        LoggingStatus::Loading,
        "the palette left the viewer spinning with nothing to clear it"
    );
}

/// Types `query` into an open palette and submits, mirroring
/// `select_command_from_open_palette` in `reducer_command_test.rs` -- minus its
/// assertion that the command is visible, because whether the palette offers a
/// command with no resolvable context is exactly what the caller is probing.
fn submit_palette_command(app: &mut AppState, query: &str) -> Vec<Effect> {
    reduce(app, &Action::Command(CommandAction::Open));
    for ch in query.chars() {
        reduce(app, &Action::Command(CommandAction::AppendChar(ch)));
    }
    reduce(app, &Action::Command(CommandAction::Submit))
}

/// Same obligation for the `L` key and every palette log jump behind it: they
/// share one resolver, and it must not be able to leave the viewer at `Loading`
/// when it dispatched no fetch. With no context it reports the refusal instead,
/// and the buffer it never touched stays readable.
#[test]
fn opening_logs_for_the_selection_never_spins_without_a_dispatched_load() {
    let mut app = app_in_logging_with_entries();
    lose_the_context(&mut app);

    let effects = reduce(
        &mut app,
        &Action::Resources(ResourcesAction::OpenLogsForSelection),
    );

    assert!(
        !effects
            .iter()
            .any(|e| matches!(e, Effect::LoadRecentLogs { .. })),
        "a log jump cannot load without a context"
    );
    assert_ne!(
        app.root.logging.status,
        LoggingStatus::Loading,
        "the log jump left the viewer spinning with nothing to clear it"
    );
    assert_eq!(
        buffered_insert_ids(&app),
        vec!["a", "b"],
        "a refused log jump took the rows the operator was reading"
    );
}

#[test]
fn raising_the_severity_floor_reloads_with_the_new_floor() {
    let mut app = app_in_logging_with_entries();
    let effects = reduce(
        &mut app,
        &Action::Logging(LoggingAction::RaiseSeverityFloor),
    );
    assert_eq!(app.root.logging.severity_floor, LogSeverity::Debug);
    assert!(app.root.logging.entries.is_empty());
    let severity = effects.iter().find_map(|effect| match effect {
        Effect::LoadRecentLogs { severity, .. } => Some(*severity),
        _ => None,
    });
    assert_eq!(severity, Some(LogSeverity::Debug));
}

#[test]
fn raising_the_severity_floor_at_the_ceiling_neither_reloads_nor_clears() {
    let mut app = app_in_logging_with_entries();
    app.root.logging.severity_floor = LogSeverity::Emergency;
    let effects = reduce(
        &mut app,
        &Action::Logging(LoggingAction::RaiseSeverityFloor),
    );
    assert_eq!(app.root.logging.entries.len(), 2);
    assert!(effects.is_empty(), "unexpected effects: {effects:?}");
}

#[test]
fn lowering_the_severity_floor_reloads_with_the_new_floor() {
    let mut app = app_in_logging_with_entries();
    app.root.logging.severity_floor = LogSeverity::Error;
    let effects = reduce(
        &mut app,
        &Action::Logging(LoggingAction::LowerSeverityFloor),
    );
    assert_eq!(app.root.logging.severity_floor, LogSeverity::Warning);
    let severity = effects.iter().find_map(|effect| match effect {
        Effect::LoadRecentLogs { severity, .. } => Some(*severity),
        _ => None,
    });
    assert_eq!(severity, Some(LogSeverity::Warning));
}

#[test]
fn lowering_the_severity_floor_at_the_sentinel_neither_reloads_nor_clears() {
    let mut app = app_in_logging_with_entries();
    let effects = reduce(
        &mut app,
        &Action::Logging(LoggingAction::LowerSeverityFloor),
    );
    assert_eq!(app.root.logging.severity_floor, LogSeverity::Default);
    assert_eq!(app.root.logging.entries.len(), 2);
    assert!(effects.is_empty(), "unexpected effects: {effects:?}");
}

#[test]
fn selecting_a_new_time_range_reloads_with_that_range() {
    let mut app = app_in_logging_with_entries();
    let effects = reduce(
        &mut app,
        &Action::Logging(LoggingAction::SetTimeRange(LogTimeRange::OneMinute)),
    );
    assert_eq!(app.root.logging.time_range, LogTimeRange::OneMinute);
    assert!(app.root.logging.entries.is_empty());
    let range = effects.iter().find_map(|effect| match effect {
        Effect::LoadRecentLogs { range, .. } => Some(*range),
        _ => None,
    });
    assert_eq!(range, Some(LogTimeRange::OneMinute));
}

#[test]
fn selecting_the_active_time_range_neither_reloads_nor_clears() {
    let mut app = app_in_logging_with_entries();
    let active = app.root.logging.time_range;
    let effects = reduce(
        &mut app,
        &Action::Logging(LoggingAction::SetTimeRange(active)),
    );
    assert_eq!(app.root.logging.entries.len(), 2);
    assert!(effects.is_empty(), "unexpected effects: {effects:?}");
}

/// Two loads in flight for different severity floors resolve last-request-wins
/// by identity, not last-arrival-wins: the page fetched for the looser floor
/// the user has already left is dropped even though it lands last.
#[test]
fn a_stale_severity_floor_page_is_rejected_even_when_it_arrives_last() {
    let mut app = app_in_logging_with_entries();

    let loose = reduce(
        &mut app,
        &Action::Logging(LoggingAction::RaiseSeverityFloor),
    );
    let strict = reduce(
        &mut app,
        &Action::Logging(LoggingAction::RaiseSeverityFloor),
    );
    let loose = load_effect(&loose);
    let strict = load_effect(&strict);
    assert_ne!(loose, strict, "the two loads must differ in identity");

    reduce(
        &mut app,
        &Action::EffectResult(loaded(&strict, vec![entry("strict", "t9")])),
    );
    reduce(
        &mut app,
        &Action::EffectResult(loaded(&loose, vec![entry("loose", "t1")])),
    );

    let ids: Vec<&str> = app
        .root
        .logging
        .entries
        .iter()
        .map(|entry| entry.insert_id.as_str())
        .collect();
    assert_eq!(
        ids,
        vec!["strict"],
        "the stale looser-floor page overwrote the current one"
    );
}

/// A page for the current context/query but a time bucket the user has since
/// left is dropped for the same reason.
#[test]
fn a_stale_time_bucket_page_is_rejected() {
    let mut app = app_in_logging_with_entries();
    let stale = load_effect(&reduce(
        &mut app,
        &Action::Logging(LoggingAction::SetTimeRange(LogTimeRange::OneMinute)),
    ));
    reduce(
        &mut app,
        &Action::Logging(LoggingAction::SetTimeRange(LogTimeRange::SixHours)),
    );

    reduce(
        &mut app,
        &Action::EffectResult(loaded(&stale, vec![entry("stale", "t1")])),
    );

    assert!(
        app.root.logging.entries.is_empty(),
        "a page for an abandoned bucket was applied: {:?}",
        app.root.logging.entries
    );
}

/// A tail poll issued under the old floor, overtaken by the fresh page the
/// user's `s` press triggered: the late tail page must be rejected. Applying
/// it would append entries below the floor the header now advertises AND
/// regress the tail cursor, neither of which self-corrects.
#[test]
fn a_tail_page_from_the_previous_floor_is_rejected_after_a_fresh_page_lands() {
    let mut app = app_in_logging_with_entries();
    let stale_tail = tail_effect(&app);

    let fresh = load_effect(&reduce(
        &mut app,
        &Action::Logging(LoggingAction::RaiseSeverityFloor),
    ));
    reduce(
        &mut app,
        &Action::EffectResult(loaded(&fresh, vec![entry("fresh", "t9")])),
    );
    let cursor_after_fresh = app.root.logging.cursor.clone();

    reduce(
        &mut app,
        &Action::EffectResult(tailed(&stale_tail, vec![entry("sub-floor", "t1")])),
    );

    let ids: Vec<&str> = app
        .root
        .logging
        .entries
        .iter()
        .map(|entry| entry.insert_id.as_str())
        .collect();
    assert_eq!(
        ids,
        vec!["fresh"],
        "a tail page from the abandoned floor was appended"
    );
    assert_eq!(
        app.root.logging.cursor, cursor_after_fresh,
        "the rejected tail page regressed the tail cursor"
    );
}

/// The `TailLogs` effect a poll tick would emit for the current state, built
/// the same way `poll_active_tail` builds it.
fn tail_effect(app: &AppState) -> Effect {
    Effect::TailLogs {
        context: app.root.current_context().expect("an active context"),
        query: app.root.logging.query.clone(),
        severity: app.root.logging.severity_floor,
        cursor: app
            .root
            .logging
            .cursor
            .clone()
            .expect("a populated buffer has a cursor"),
    }
}

/// The single `LoadRecentLogs` a narrowing key press emits.
fn load_effect(effects: &[Effect]) -> Effect {
    effects
        .iter()
        .find(|effect| matches!(effect, Effect::LoadRecentLogs { .. }))
        .expect("a moved window dispatches a load")
        .clone()
}

/// The result the runtime would send back for `effect`, echoing its window
/// identity the way `load_recent_logs` does.
fn loaded(effect: &Effect, entries: Vec<nuvem_domain::gcp::logging::LogEntry>) -> EffectResult {
    let Effect::LoadRecentLogs {
        context,
        query,
        severity,
        range,
    } = effect
    else {
        panic!("not a log load: {effect:?}");
    };
    EffectResult::RecentLogsLoaded {
        context: context.clone(),
        query: query.clone(),
        severity: *severity,
        range: *range,
        entries,
        dropped: 0,
    }
}

/// The result the runtime would send back for a tail `effect`, echoing its
/// identity the way `tail_logs` does. The tail is cursor-driven, so there is
/// no range on it.
fn tailed(effect: &Effect, entries: Vec<nuvem_domain::gcp::logging::LogEntry>) -> EffectResult {
    let Effect::TailLogs {
        context,
        query,
        severity,
        ..
    } = effect
    else {
        panic!("not a log tail: {effect:?}");
    };
    EffectResult::LogsTailed {
        context: context.clone(),
        query: query.clone(),
        severity: *severity,
        entries,
        dropped: 0,
    }
}

#[test]
fn save_to_file_carries_the_rendered_visible_lines() {
    let mut app = app_in_logging_with_entries();
    let effects = reduce(&mut app, &Action::Logging(LoggingAction::SaveToFile));
    assert_eq!(
        effects,
        vec![Effect::SaveLogEntries {
            lines: vec![
                "t1 ERROR gce_instance msg-a".to_string(),
                "t2 ERROR gce_instance msg-b".to_string(),
            ]
        }]
    );
}

#[test]
fn save_to_file_emits_nothing_when_there_is_nothing_visible() {
    let mut app = app_with_active_config();
    reduce(&mut app, &Action::Workspace(WorkspaceAction::Logging));
    let effects = reduce(&mut app, &Action::Logging(LoggingAction::SaveToFile));
    assert!(effects.is_empty(), "unexpected effects: {effects:?}");
    let status = app
        .root
        .system
        .status_message
        .clone()
        .expect("an empty save must still explain itself");
    assert!(
        status.contains("No log entries to save"),
        "unexpected status: {status}"
    );
}
