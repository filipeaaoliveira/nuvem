#[allow(unused_imports)]
use std::collections::VecDeque;

use nuvem::app::state::logging::{
    LOG_BACKOFF_MAX_TICKS, LOG_BUFFER_CAP, LoggingState, LoggingStatus, TailState,
};
use nuvem_domain::gcp::diagnostics::GcloudErrorKind;
use nuvem_domain::gcp::logging::{LogEntry, LogPayload, LogSeverity, LogTimeRange};

fn entry(id: &str, ts: &str) -> LogEntry {
    LogEntry {
        insert_id: id.to_string(),
        timestamp: ts.to_string(),
        severity: LogSeverity::Info,
        resource_type: "gce_instance".to_string(),
        resource_labels: Default::default(),
        log_name: "l".to_string(),
        message: format!("msg-{id}"),
        payload: LogPayload::Empty,
        labels: Default::default(),
    }
}

#[test]
fn apply_recent_sets_loaded_and_cursor() {
    let mut state = LoggingState::default();
    state.apply_recent(vec![entry("a", "t1"), entry("b", "t2")], 0);
    assert_eq!(state.status, LoggingStatus::Loaded);
    assert_eq!(state.entries.len(), 2);
    let cursor = state.cursor.as_ref().expect("cursor");
    assert_eq!(cursor.timestamp, "t2");
}

#[test]
fn apply_recent_cursor_points_to_newest_entry() {
    // Ascending input (oldest -> newest); back() must be the newest and the
    // cursor timestamp must equal the newest entry's timestamp so the tail
    // advances rather than re-reading the whole window.
    let mut state = LoggingState::default();
    state.apply_recent(
        vec![entry("a", "t1"), entry("b", "t2"), entry("c", "t3")],
        0,
    );
    assert_eq!(state.entries.back().unwrap().insert_id, "c");
    let cursor = state.cursor.as_ref().expect("cursor");
    assert_eq!(cursor.timestamp, "t3");
}

#[test]
fn apply_tail_advances_cursor_to_strictly_newer_entry() {
    let mut state = LoggingState::default();
    state.apply_recent(vec![entry("a", "t1"), entry("b", "t2")], 0);
    assert_eq!(state.cursor.as_ref().unwrap().timestamp, "t2");
    state.apply_tail(vec![entry("c", "t3")], 0);
    assert_eq!(state.entries.back().unwrap().insert_id, "c");
    assert_eq!(state.cursor.as_ref().unwrap().timestamp, "t3");
}

#[test]
fn apply_tail_surfaces_same_timestamp_unseen_entry() {
    // The tail cursor uses an inclusive `timestamp>=` boundary, so gcloud
    // re-fetches entries sharing the newest nanosecond timestamp. An already
    // seen one is deduplicated, but a NEW entry at that same timestamp must be
    // surfaced rather than silently dropped.
    let mut state = LoggingState::default();
    state.apply_recent(vec![entry("a", "t1"), entry("b", "t2")], 0);
    assert_eq!(state.cursor.as_ref().unwrap().timestamp, "t2");
    // "b" shares the cursor timestamp and is deduped; "c" shares it too but is
    // unseen and must be appended.
    state.apply_tail(vec![entry("b", "t2"), entry("c", "t2")], 0);
    let ids: Vec<_> = state.entries.iter().map(|e| e.insert_id.clone()).collect();
    assert_eq!(ids, vec!["a", "b", "c"]);
}

#[test]
fn apply_recent_empty_is_healthy_empty() {
    let mut state = LoggingState::default();
    state.apply_recent(vec![], 0);
    assert_eq!(state.status, LoggingStatus::HealthyEmpty);
}

#[test]
fn apply_recent_partial_reports_dropped() {
    let mut state = LoggingState::default();
    state.apply_recent(vec![entry("a", "t1")], 3);
    assert_eq!(state.status, LoggingStatus::PartialParse { dropped: 3 });
}

#[test]
fn apply_tail_dedups_by_insert_id() {
    let mut state = LoggingState::default();
    state.apply_recent(vec![entry("a", "t1"), entry("b", "t2")], 0);
    // "b" is a duplicate; only "c" is appended.
    state.apply_tail(vec![entry("b", "t2"), entry("c", "t3")], 0);
    let ids: Vec<_> = state.entries.iter().map(|e| e.insert_id.clone()).collect();
    assert_eq!(ids, vec!["a", "b", "c"]);
}

#[test]
fn ring_buffer_caps_and_prunes_seen() {
    let mut state = LoggingState::default();
    let entries: Vec<_> = (0..(LOG_BUFFER_CAP + 50))
        .map(|i| entry(&format!("id-{i}"), &format!("t-{i:06}")))
        .collect();
    state.apply_recent(entries, 0);
    assert_eq!(state.entries.len(), LOG_BUFFER_CAP);
    // Oldest dropped; a new tail entry with an evicted id is treated as unseen.
    let before = state.entries.len();
    state.apply_tail(vec![entry("id-0", "t-999999")], 0);
    assert_eq!(state.entries.len(), before); // still capped
    assert_eq!(state.entries.back().unwrap().insert_id, "id-0");
}

#[test]
fn quota_error_sets_backoff_that_decrements_to_zero() {
    let mut state = LoggingState::default();
    state.record_error(GcloudErrorKind::Quota);
    assert_eq!(
        state.status,
        LoggingStatus::Degraded {
            kind: GcloudErrorKind::Quota
        }
    );
    assert!(state.backoff_ticks >= 1);
    // Repeated quota errors grow backoff but never exceed the cap.
    for _ in 0..20 {
        state.record_error(GcloudErrorKind::Quota);
    }
    assert!(state.backoff_ticks <= LOG_BACKOFF_MAX_TICKS);
    // Ticks decrement to zero.
    let mut guard = 0;
    while !state.decrement_backoff() {
        guard += 1;
        assert!(guard < 100);
    }
    assert_eq!(state.backoff_ticks, 0);
}

#[test]
fn non_quota_error_does_not_set_backoff() {
    let mut state = LoggingState::default();
    state.record_error(GcloudErrorKind::PermissionDenied);
    assert_eq!(state.backoff_ticks, 0);
    assert_eq!(
        state.status,
        LoggingStatus::Degraded {
            kind: GcloudErrorKind::PermissionDenied
        }
    );
}

#[test]
fn toggle_tail_switches_running_paused() {
    let mut state = LoggingState::default();
    assert_eq!(state.tail, TailState::Running);
    state.toggle_tail();
    assert_eq!(state.tail, TailState::Paused);
}

#[test]
fn selection_follows_entry_by_id_when_front_evicted() {
    let mut state = LoggingState::default();
    let entries: Vec<_> = (0..LOG_BUFFER_CAP)
        .map(|i| entry(&format!("id-{i}"), &format!("t-{i:06}")))
        .collect();
    state.apply_recent(entries, 0);
    assert_eq!(state.entries.len(), LOG_BUFFER_CAP);

    // Land on a known entry that is not at the very front, so its index will
    // shift (but not vanish) when the front is evicted.
    //
    // Ten steps, not eleven: the load seeds the cursor on row 0, so
    // the first `select_next` now moves to row 1 rather than arriving at row 0.
    for _ in 0..10 {
        state.select_next();
    }
    assert_eq!(state.selected, Some(10));
    let selected_id = state.visible_entries()[state.selected.unwrap()]
        .insert_id
        .clone();
    assert_eq!(selected_id, "id-10");

    // Append strictly-newer entries at the back, forcing front eviction that
    // shifts every remaining entry's index down.
    let new_entries: Vec<_> = (0..5)
        .map(|i| entry(&format!("new-{i}"), &format!("t-{:06}", LOG_BUFFER_CAP + i)))
        .collect();
    state.apply_tail(new_entries, 0);
    assert_eq!(state.entries.len(), LOG_BUFFER_CAP);

    // Selection must still resolve to the SAME entry, not a different one
    // that happens to now occupy the old index.
    let resolved = &state.visible_entries()[state.selected.expect("selection preserved")];
    assert_eq!(resolved.insert_id, selected_id);
}

#[test]
fn selection_clears_when_selected_entry_is_evicted() {
    let mut state = LoggingState::default();
    let entries: Vec<_> = (0..LOG_BUFFER_CAP)
        .map(|i| entry(&format!("id-{i}"), &format!("t-{i:06}")))
        .collect();
    state.apply_recent(entries, 0);
    assert_eq!(state.entries.len(), LOG_BUFFER_CAP);

    // The oldest entry (front of the buffer) is where the load leaves the
    // cursor, so there is nothing to press to get there. What this
    // test is about is what happens when that entry is evicted, which is
    // unchanged.
    assert_eq!(state.selected, Some(0));
    assert_eq!(state.selected_insert_id, Some("id-0".to_string()));

    // Append enough new entries to evict the selected (oldest) entry.
    let new_entries: Vec<_> = (0..5)
        .map(|i| entry(&format!("new-{i}"), &format!("t-{:06}", LOG_BUFFER_CAP + i)))
        .collect();
    state.apply_tail(new_entries, 0);

    // Selection must clear rather than silently drift to a different entry.
    assert_eq!(state.selected, None);
    assert_eq!(state.selected_insert_id, None);
}

#[test]
fn visible_entries_filter_is_case_insensitive_substring() {
    let mut state = LoggingState::default();
    state.apply_recent(vec![entry("a", "t1"), entry("b", "t2")], 0);
    state.start_filter();
    for ch in "MSG-A".chars() {
        state.append_filter_char(ch);
    }
    state.apply_filter();
    let visible = state.visible_entries();
    assert_eq!(visible.len(), 1);
    assert_eq!(visible[0].insert_id, "a");
}

/// A log entry with fully controllable searchable/exportable fields, for the
/// filter and export-rendering tests below.
fn detailed_entry(
    id: &str,
    ts: &str,
    severity: LogSeverity,
    resource: &str,
    message: &str,
) -> LogEntry {
    LogEntry {
        insert_id: id.to_string(),
        timestamp: ts.to_string(),
        severity,
        resource_type: resource.to_string(),
        resource_labels: Default::default(),
        log_name: "l".to_string(),
        message: message.to_string(),
        payload: LogPayload::Empty,
        labels: Default::default(),
    }
}

/// Drives the real edit flow: open the prompt (which seeds the draft with the
/// applied filter), clear it, type `pattern`, apply.
fn with_filter(state: &mut LoggingState, pattern: &str) {
    state.start_filter();
    while !state.filter_draft.is_empty() {
        state.backspace_filter();
    }
    for ch in pattern.chars() {
        state.append_filter_char(ch);
    }
    state.apply_filter();
}

fn visible_ids(state: &LoggingState) -> Vec<String> {
    state
        .visible_entries()
        .iter()
        .map(|entry| entry.insert_id.clone())
        .collect()
}

fn populated() -> LoggingState {
    let mut state = LoggingState::default();
    state.apply_recent(vec![entry("a", "t1"), entry("b", "t2")], 0);
    // The load seeds the cursor on the first row, so the
    // `select_next()` that used to be here would now step *off* it. The
    // fixture's contract is "a populated list with a selection", which the load
    // already satisfies.
    debug_assert_eq!(state.selected, Some(0));
    state
}

#[test]
fn raise_severity_floor_steps_up_and_clears_the_buffer_for_a_refetch() {
    let mut state = populated();
    assert!(state.raise_severity_floor(), "floor changed");
    assert_eq!(state.severity_floor, LogSeverity::Debug);
    assert!(state.entries.is_empty());
    assert!(state.seen.is_empty());
    assert!(state.cursor.is_none());
    assert_eq!(state.selected, None);
    assert_eq!(state.selected_insert_id, None);
    assert_eq!(state.status, LoggingStatus::Loading);
}

#[test]
fn raise_severity_floor_at_the_ceiling_keeps_the_buffer_intact() {
    let mut state = populated();
    state.severity_floor = LogSeverity::Emergency;
    assert!(!state.raise_severity_floor(), "clamped step is a no-op");
    assert_eq!(state.severity_floor, LogSeverity::Emergency);
    assert_eq!(state.entries.len(), 2);
    assert!(state.cursor.is_some());
    assert_eq!(state.status, LoggingStatus::Loaded);
    assert_eq!(state.selected, Some(0));
}

#[test]
fn lower_severity_floor_steps_down_and_clears_the_buffer_for_a_refetch() {
    let mut state = populated();
    state.severity_floor = LogSeverity::Error;
    assert!(state.lower_severity_floor(), "floor changed");
    assert_eq!(state.severity_floor, LogSeverity::Warning);
    assert!(state.entries.is_empty());
    assert_eq!(state.status, LoggingStatus::Loading);
}

#[test]
fn lower_severity_floor_at_the_no_floor_sentinel_keeps_the_buffer_intact() {
    let mut state = populated();
    assert_eq!(state.severity_floor, LogSeverity::Default);
    assert!(!state.lower_severity_floor(), "clamped step is a no-op");
    assert_eq!(state.severity_floor, LogSeverity::Default);
    assert_eq!(state.entries.len(), 2);
    assert_eq!(state.status, LoggingStatus::Loaded);
}

#[test]
fn set_time_range_to_a_new_bucket_clears_the_buffer_for_a_refetch() {
    let mut state = populated();
    assert_ne!(state.time_range, LogTimeRange::OneMinute);
    assert!(
        state.set_time_range(LogTimeRange::OneMinute),
        "range changed"
    );
    assert_eq!(state.time_range, LogTimeRange::OneMinute);
    assert!(state.entries.is_empty());
    assert!(state.seen.is_empty());
    assert!(state.cursor.is_none());
    assert_eq!(state.selected, None);
    assert_eq!(state.status, LoggingStatus::Loading);
}

#[test]
fn set_time_range_to_the_active_bucket_keeps_the_buffer_intact() {
    let mut state = populated();
    let active = state.time_range;
    assert!(!state.set_time_range(active), "unchanged range is a no-op");
    assert_eq!(state.entries.len(), 2);
    assert!(state.cursor.is_some());
    assert_eq!(state.status, LoggingStatus::Loaded);
    assert_eq!(state.selected, Some(0));
}

fn filter_fixture() -> LoggingState {
    let mut state = LoggingState::default();
    state.apply_recent(
        vec![
            detailed_entry("a", "t1", LogSeverity::Error, "gce_instance", "disk full"),
            detailed_entry(
                "b",
                "t2",
                LogSeverity::Info,
                "cloud_run_revision",
                "request served",
            ),
            detailed_entry(
                "c",
                "t3",
                LogSeverity::Warning,
                "k8s_container",
                "retrying fetch",
            ),
        ],
        0,
    );
    state
}

#[test]
fn visible_entries_matches_a_substring_across_message_severity_and_resource() {
    let mut state = filter_fixture();
    with_filter(&mut state, "disk");
    assert_eq!(visible_ids(&state), vec!["a"]);

    let mut state = filter_fixture();
    with_filter(&mut state, "warning");
    assert_eq!(visible_ids(&state), vec!["c"]);

    let mut state = filter_fixture();
    with_filter(&mut state, "cloud_run");
    assert_eq!(visible_ids(&state), vec!["b"]);
}

#[test]
fn visible_entries_matches_a_fuzzy_subsequence() {
    let mut state = filter_fixture();
    with_filter(&mut state, "~dkfl");
    assert_eq!(visible_ids(&state), vec!["a"]);
}

#[test]
fn visible_entries_matches_a_regex() {
    let mut state = filter_fixture();
    with_filter(&mut state, "/re(quest|trying)/");
    assert_eq!(visible_ids(&state), vec!["b", "c"]);
}

#[test]
fn visible_entries_inverts_with_a_leading_bang() {
    let mut state = filter_fixture();
    with_filter(&mut state, "!disk");
    assert_eq!(visible_ids(&state), vec!["b", "c"]);
}

#[test]
fn an_invalid_regex_filter_shows_no_entries_and_records_the_error() {
    let mut state = filter_fixture();
    with_filter(&mut state, "/[unclosed/");
    assert!(state.visible_entries().is_empty());
    let error = state
        .applied_filter_error()
        .expect("parse error recorded for the header");
    assert!(error.contains("regex"), "unexpected message: {error}");
}

/// The compiled filter is cached beside its source, so every apply must
/// recompile: a cache that kept the first compile would keep answering with
/// the first pattern's rows while `applied_filter()` reported the second.
#[test]
fn every_apply_recompiles_the_cached_filter() {
    let mut state = filter_fixture();

    with_filter(&mut state, "disk");
    assert_eq!(state.applied_filter(), "disk");
    assert_eq!(visible_ids(&state), vec!["a"]);

    with_filter(&mut state, "request");
    assert_eq!(state.applied_filter(), "request");
    assert_eq!(
        visible_ids(&state),
        vec!["b"],
        "visible entries must follow the newly applied pattern, not a stale compile"
    );

    // A different kind, so a cache keyed on nothing at all cannot coincide.
    with_filter(&mut state, "/re(quest|trying)/");
    assert_eq!(state.applied_filter(), "/re(quest|trying)/");
    assert_eq!(visible_ids(&state), vec!["b", "c"]);

    // Back to no filter: the cache must go back to matching everything.
    with_filter(&mut state, "");
    assert_eq!(state.applied_filter(), "");
    assert_eq!(visible_ids(&state), vec!["a", "b", "c"]);

    // An unparseable pattern hides every row and records the error.
    with_filter(&mut state, "/[unclosed/");
    assert_eq!(state.applied_filter(), "/[unclosed/");
    assert!(state.visible_entries().is_empty());
    assert!(state.applied_filter_error().is_some());

    // And a valid one after it must not inherit the failed compile.
    with_filter(&mut state, "disk");
    assert_eq!(state.applied_filter_error(), None);
    assert_eq!(visible_ids(&state), vec!["a"]);
}

#[test]
fn cancelling_a_filter_edit_leaves_the_cached_filter_untouched() {
    let mut state = filter_fixture();
    with_filter(&mut state, "disk");

    state.start_filter();
    state.append_filter_char('x');
    state.cancel_filter();

    assert_eq!(state.applied_filter(), "disk");
    assert_eq!(
        state.filter_draft, "disk",
        "the draft reverts to the source"
    );
    assert_eq!(visible_ids(&state), vec!["a"]);
}

/// The parse error is a fact the compiled cache already owns, so it must be
/// read through an accessor that asks the cache — not copied into a public
/// field a caller could write out from under it. Reading it any other way is
/// how the header's mode label ends up describing a pattern the rows were never
/// matched against.
#[test]
fn the_applied_filter_error_is_read_from_the_compiled_cache() {
    let mut state = filter_fixture();

    with_filter(&mut state, "/[unclosed/");
    let error = state
        .applied_filter_error()
        .expect("the accessor must report the cache's parse error");
    assert!(error.contains("regex"), "unexpected message: {error}");

    with_filter(&mut state, "disk");
    assert_eq!(
        state.applied_filter_error(),
        None,
        "a pattern that compiles has no error to report"
    );
}

#[test]
fn applying_a_valid_filter_clears_a_previously_recorded_error() {
    let mut state = filter_fixture();
    with_filter(&mut state, "/[unclosed/");
    assert!(state.applied_filter_error().is_some());
    with_filter(&mut state, "disk");
    assert_eq!(state.applied_filter_error(), None);
    assert_eq!(visible_ids(&state), vec!["a"]);
}

#[test]
fn visible_export_lines_render_every_visible_entry_oldest_first() {
    let state = filter_fixture();
    assert_eq!(
        state.visible_export_lines(),
        vec![
            "t1 ERROR gce_instance disk full".to_string(),
            "t2 INFO cloud_run_revision request served".to_string(),
            "t3 WARNING k8s_container retrying fetch".to_string(),
        ]
    );
}

#[test]
fn visible_export_lines_follow_the_active_filter() {
    let mut state = filter_fixture();
    with_filter(&mut state, "disk");
    assert_eq!(
        state.visible_export_lines(),
        vec!["t1 ERROR gce_instance disk full".to_string()]
    );
}

#[test]
fn visible_export_lines_strip_control_characters_from_every_field() {
    let mut state = LoggingState::default();
    state.apply_recent(
        vec![detailed_entry(
            "a",
            "t1\u{1b}[2J",
            LogSeverity::Error,
            "gce\u{7}_instance",
            "line one\nline\ttwo\u{1b}[31m",
        )],
        0,
    );
    assert_eq!(
        state.visible_export_lines(),
        vec!["t1[2J ERROR gce_instance line oneline two[31m".to_string()]
    );
}
