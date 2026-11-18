use nuvem::app::projection::logging::{LoggingBodyState, project_logging};
use nuvem::app::state::root::{RootState, Workspace};
use nuvem_domain::gcp::diagnostics::GcloudErrorKind;
use nuvem_domain::gcp::logging::{
    LogEntry, LogPayload, LogQuery, LogResourceKind, LogSeverity, LogTimeRange,
};

fn entry_with_message(msg: &str) -> LogEntry {
    LogEntry {
        insert_id: "a".into(),
        timestamp: "2026-07-19T10:00:00Z".into(),
        severity: LogSeverity::Error,
        resource_type: "gce_instance".into(),
        resource_labels: Default::default(),
        log_name: "l".into(),
        message: msg.into(),
        payload: LogPayload::Empty,
        labels: Default::default(),
    }
}

#[test]
fn projection_sanitizes_row_message() {
    let mut root = RootState {
        workspace: Workspace::Logging,
        ..Default::default()
    };
    root.logging
        .apply_recent(vec![entry_with_message("bad\u{1b}[2Jrow")], 0);
    let projection = project_logging(&root);
    let joined = projection
        .table
        .rows
        .iter()
        .flat_map(|r| r.cells.iter().map(|c| c.text.clone()))
        .collect::<Vec<_>>()
        .join(" ");
    assert!(!joined.contains('\u{1b}'));
}

#[test]
fn projection_header_shows_query_and_tail_state() {
    let root = RootState {
        workspace: Workspace::Logging,
        ..Default::default()
    };
    let projection = project_logging(&root);
    assert!(projection.header.query.contains("project logs"));
}

#[test]
fn degraded_with_entries_keeps_rows_and_shows_status() {
    let mut root = RootState {
        workspace: Workspace::Logging,
        ..Default::default()
    };
    root.logging
        .apply_recent(vec![entry_with_message("row one")], 0);
    root.logging.record_error(GcloudErrorKind::Quota);
    let projection = project_logging(&root);
    // Rows stay visible on a transient failure...
    assert_eq!(projection.body_state, LoggingBodyState::Rows);
    // ...and the degraded state is surfaced in the header status.
    assert!(projection.header.status.contains("Degraded"));
}

#[test]
fn degraded_with_no_entries_shows_degraded_body() {
    let mut root = RootState {
        workspace: Workspace::Logging,
        ..Default::default()
    };
    root.logging.record_error(GcloudErrorKind::Quota);
    let projection = project_logging(&root);
    match projection.body_state {
        LoggingBodyState::Degraded { message } => {
            assert!(message.contains("degraded"));
        }
        other => panic!("expected degraded body, got {other:?}"),
    }
}

#[test]
fn header_shows_exact_generated_filter_for_resource_query() {
    let mut root = RootState {
        workspace: Workspace::Logging,
        ..Default::default()
    };
    root.logging.query = LogQuery::Resource(LogResourceKind::ComputeInstance {
        instance_id: "1001".to_string(),
        instance_name: "api-dev-1".to_string(),
        zone: "europe-west1-b".to_string(),
    });
    let projection = project_logging(&root);
    assert!(
        projection
            .header
            .filter
            .contains("resource.type=\"gce_instance\"")
    );
    assert!(
        projection
            .header
            .filter
            .contains("resource.labels.instance_id=\"1001\"")
    );
}

#[test]
fn header_shows_placeholder_filter_for_project_logs() {
    let root = RootState {
        workspace: Workspace::Logging,
        ..Default::default()
    };
    let projection = project_logging(&root);
    assert_eq!(projection.header.filter, "(all recent project logs)");
}

#[test]
fn header_reports_severity_floor_and_bucket_digit() {
    let mut root = RootState {
        workspace: Workspace::Logging,
        ..Default::default()
    };
    // `LogSeverity::Default` is the no-floor sentinel, shown as `all`.
    let projection = project_logging(&root);
    assert_eq!(projection.header.severity_floor, "all");
    assert_eq!(projection.header.time_range, "1h");
    assert_eq!(projection.header.time_range_bucket, '4');

    root.logging.severity_floor = LogSeverity::Error;
    root.logging.time_range = LogTimeRange::FiveMinutes;
    let projection = project_logging(&root);
    assert_eq!(projection.header.severity_floor, "ERROR");
    assert_eq!(projection.header.time_range, "5m");
    assert_eq!(projection.header.time_range_bucket, '1');
}

#[test]
fn header_describes_the_applied_search_mode() {
    let mut root = RootState {
        workspace: Workspace::Logging,
        ..Default::default()
    };
    // No filter applied: there is no mode to report.
    assert_eq!(project_logging(&root).header.search_mode, None);

    // A bare term is a literal substring, not a regex.
    apply_search(&mut root, "[ERROR]");
    let projection = project_logging(&root);
    assert_eq!(projection.header.search_mode.as_deref(), Some("substring"));
    assert_eq!(projection.header.search_error, None);

    apply_search(&mut root, "~api");
    assert_eq!(
        project_logging(&root).header.search_mode.as_deref(),
        Some("fuzzy")
    );

    apply_search(&mut root, "!/timeout/");
    assert_eq!(
        project_logging(&root).header.search_mode.as_deref(),
        Some("not regex")
    );
}

/// A whitespace-only search narrows nothing, so the header must not claim a
/// mode: `(substring)` beside a view that still shows every row reads as an
/// active filter that is not there.
#[test]
fn header_reports_no_mode_for_a_search_that_narrows_nothing() {
    let mut root = RootState {
        workspace: Workspace::Logging,
        ..Default::default()
    };
    root.logging
        .apply_recent(vec![entry_with_message("row one")], 0);

    apply_search(&mut root, "   ");

    let projection = project_logging(&root);
    assert_eq!(projection.header.search_mode, None);
    assert_eq!(projection.header.search_error, None);
    assert_eq!(
        projection.table.rows.len(),
        1,
        "a whitespace-only search filters nothing"
    );
}

#[test]
fn header_surfaces_an_invalid_search_pattern_error() {
    let mut root = RootState {
        workspace: Workspace::Logging,
        ..Default::default()
    };
    apply_search(&mut root, "/([/");
    let projection = project_logging(&root);
    assert_eq!(projection.header.search_mode, None);
    let error = projection
        .header
        .search_error
        .expect("an unparseable pattern is reported in the header");
    assert!(
        error.contains("invalid regex"),
        "unexpected error text: {error}"
    );
}

/// The header labels the mode from the pattern's *syntax*, which cannot know
/// whether a regex compiles — `/([/ ` is syntactically a regex either way. The
/// recorded parse error is what suppresses the label, so the two must move
/// together: error present, no label; error cleared, label back.
#[test]
fn an_unparseable_pattern_suppresses_the_mode_label_until_it_parses_again() {
    let mut root = RootState {
        workspace: Workspace::Logging,
        ..Default::default()
    };

    apply_search(&mut root, "/([/");
    let projection = project_logging(&root);
    assert_eq!(
        projection.header.search_mode, None,
        "a pattern that does not compile has no mode to report"
    );
    assert!(projection.header.search_error.is_some());

    // The same syntax, now valid: the label returns and the error clears.
    apply_search(&mut root, "/(a)/");
    let projection = project_logging(&root);
    assert_eq!(projection.header.search_mode.as_deref(), Some("regex"));
    assert_eq!(projection.header.search_error, None);

    // And inverted, still unparseable.
    apply_search(&mut root, "!/([/");
    let projection = project_logging(&root);
    assert_eq!(projection.header.search_mode, None);
    assert!(projection.header.search_error.is_some());
}

/// Types `pattern` into the filter draft and applies it, the same path the
/// filter-entry keys drive.
fn apply_search(root: &mut RootState, pattern: &str) {
    root.logging.start_filter();
    root.logging.filter_draft = pattern.to_string();
    root.logging.apply_filter();
}

/// The two never-yet-loaded states must not read the same way. `Initial` means
/// no fetch was dispatched, so the body says so instead of spinning for a
/// result nobody asked for -- and it agrees with the header on the same frame.
#[test]
fn unloaded_buffer_renders_not_loaded_in_body_and_header() {
    let mut root = RootState {
        workspace: Workspace::Logging,
        ..Default::default()
    };
    // The state a restore into Logging with no resolvable context settles to.
    root.logging.settle_unloaded();

    let projection = project_logging(&root);
    assert_eq!(projection.body_state, LoggingBodyState::NotLoaded);
    assert_eq!(projection.header.status, "Not loaded");
}

/// ...and a load that WAS dispatched still spins, in the body as well as the
/// header. Without this the fix would trade one contradiction for the opposite
/// lie on every first load.
#[test]
fn dispatched_load_renders_loading_in_body_and_header() {
    let mut root = RootState {
        workspace: Workspace::Logging,
        ..Default::default()
    };
    // `cycle_time_range` is one of the callers that marks the buffer `Loading`
    // before asking for the fetch.
    root.logging.cycle_time_range();

    let projection = project_logging(&root);
    assert_eq!(projection.body_state, LoggingBodyState::Loading);
    assert_eq!(projection.header.status, "Loading...");
}

/// The background tail poll is the one path that starts a first page without a
/// reducer having marked the buffer, so it marks it itself -- otherwise the
/// body would deny a fetch that is in flight.
#[test]
fn first_page_started_by_the_tail_poll_renders_loading() {
    let mut root = RootState {
        workspace: Workspace::Logging,
        ..Default::default()
    };
    root.logging.settle_unloaded();
    root.logging.begin_first_load();

    let projection = project_logging(&root);
    assert_eq!(projection.body_state, LoggingBodyState::Loading);
    assert_eq!(projection.header.status, "Loading...");

    // A status that already reports a real outcome is not overwritten per tick.
    root.logging.apply_recent(Vec::new(), 0);
    let before = project_logging(&root).header.status.clone();
    root.logging.begin_first_load();
    assert_eq!(project_logging(&root).header.status, before);
}
