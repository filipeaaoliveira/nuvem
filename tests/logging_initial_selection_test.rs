//! The Logging list has a cursor from the moment it has rows.
//!
//! `Enter Open entry` is on the footer the instant the workspace opens, and it
//! did nothing until `j` had been pressed once: `selected` started `None`,
//! `open_detail` was a no-op while it was, and the list painted no cursor to say
//! so. `y` and the copy-insert-id key were inert for the same reason and are
//! advertised the same way. Every other list in the app — resources,
//! configurations, tunnels, the jump picker, the palette — shows a cursor from
//! its first frame.

use nuvem::app::reducer::root::reduce;
use nuvem::app::{Action, AppState, LoggingAction, Workspace, WorkspaceAction};
use nuvem_domain::gcp::logging::{LogEntry, LogPayload, LogSeverity};

fn entry(insert_id: &str, message: &str) -> LogEntry {
    LogEntry {
        insert_id: insert_id.to_string(),
        timestamp: "2026-07-19T10:00:00Z".to_string(),
        severity: LogSeverity::Info,
        resource_type: "gce_instance".to_string(),
        resource_labels: Default::default(),
        log_name: "projects/dev-project/logs/stdout".to_string(),
        message: message.to_string(),
        payload: LogPayload::Empty,
        labels: Default::default(),
    }
}

fn app_with_entries(entries: Vec<LogEntry>) -> AppState {
    let mut app = AppState::new();
    app.root.workspace = Workspace::Logging;
    app.root.logging.apply_recent(entries, 0);
    app
}

#[test]
fn a_loaded_log_list_has_a_cursor_without_being_moved() {
    let app = app_with_entries(vec![entry("log-1", "first"), entry("log-2", "second")]);
    assert!(
        app.root.logging.selected.is_some(),
        "the list has rows and no cursor, so the advertised `Enter` is inert"
    );
}

/// The advertised key works on the first press, which is the whole point.
#[test]
fn enter_opens_a_detail_on_a_freshly_loaded_list() {
    let mut app = app_with_entries(vec![entry("log-1", "first"), entry("log-2", "second")]);
    reduce(&mut app, &Action::Logging(LoggingAction::OpenDetail));
    assert!(
        app.root.logging.detail_open,
        "`Enter` did nothing on a freshly loaded list"
    );
}

/// An empty list gets no cursor: there is nothing to point at, and a cursor on
/// no row would be a claim the list cannot keep.
#[test]
fn an_empty_log_list_has_no_cursor() {
    let app = app_with_entries(Vec::new());
    assert!(app.root.logging.selected.is_none());
}

/// The tail does not drag the cursor.
///
/// This is the property that makes an automatic first selection safe: entries
/// arrive at the top, and `resync_selection` anchors by `insert_id` rather than
/// by index, so the entry under the cursor stays under it as the list grows.
/// Without this an automatic selection would slide onto whatever is newest and
/// the operator would lose their place while reading.
#[test]
fn new_entries_do_not_move_the_cursor_off_its_entry() {
    let mut app = app_with_entries(vec![entry("log-1", "first")]);
    let anchored = app.root.logging.selected_insert_id.clone();
    assert!(anchored.is_some(), "the first load must anchor the cursor");

    app.root
        .logging
        .apply_tail(vec![entry("log-9", "newest")], 0);

    assert_eq!(
        app.root.logging.selected_insert_id, anchored,
        "a new entry took the cursor off the entry it was on"
    );

    // Index-agnostic on purpose: which way the visible list is ordered is not
    // this test's claim, and pinning an index here would assert the ordering by
    // accident. What must hold is that the row the cursor is on is still the
    // entry it was anchored to.
    let index = app
        .root
        .logging
        .selected
        .expect("the cursor survived the append");
    let under_cursor = app.root.logging.visible_entries()[index].insert_id.clone();
    assert_eq!(
        Some(under_cursor),
        anchored,
        "the cursor's index no longer points at the entry it is anchored to"
    );
    assert_eq!(
        app.root.logging.visible_entries().len(),
        2,
        "the append must have landed, or nothing was tested"
    );
}

/// Opening the workspace is enough; no keypress is needed in between.
#[test]
fn switching_to_the_logging_workspace_leaves_a_usable_cursor() {
    let mut app = app_with_entries(vec![entry("log-1", "first")]);
    app.root.workspace = Workspace::Home;
    reduce(&mut app, &Action::Workspace(WorkspaceAction::Logging));
    reduce(&mut app, &Action::Logging(LoggingAction::OpenDetail));
    assert!(app.root.logging.detail_open);
}
