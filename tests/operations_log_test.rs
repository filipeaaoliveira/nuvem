use nuvem::app::state::operations::{OperationOutcome, OperationsLog};

#[test]
fn begin_pushes_pending_record_to_front() {
    let mut log = OperationsLog::default();
    let id = log.begin("Start instance", "web-1");
    assert_eq!(log.pending_count(), 1);
    let front = log.records().front().unwrap();
    assert_eq!(front.id, id);
    assert_eq!(front.kind, "Start instance");
    assert_eq!(front.target, "web-1");
    assert_eq!(front.outcome, OperationOutcome::Pending);
}

#[test]
fn resolve_by_id_clears_pending_and_records_the_detail() {
    let mut log = OperationsLog::default();
    let id = log.begin("Start instance", "web-1");
    let resolved = log.resolve_by_id(
        id,
        OperationOutcome::Succeeded,
        Some("accepted".to_string()),
    );
    assert!(resolved);
    assert_eq!(log.pending_count(), 0);
    let front = log.records().front().unwrap();
    assert_eq!(front.outcome, OperationOutcome::Succeeded);
    assert_eq!(front.detail.as_deref(), Some("accepted"));
}

#[test]
fn resolve_by_id_returns_false_on_an_empty_log() {
    let mut log = OperationsLog::default();
    assert!(!log.resolve_by_id(0, OperationOutcome::Failed, None));
}

#[test]
fn resolve_by_id_marks_that_record() {
    let mut log = OperationsLog::default();
    let id = log.begin("Start instance", "web-1");
    log.begin("Start instance", "web-2");
    let resolved = log.resolve_by_id(
        id,
        OperationOutcome::Cancelled,
        Some("not issued".to_string()),
    );
    assert!(resolved);
    assert_eq!(log.pending_count(), 1);
    let record = log.records().iter().find(|r| r.id == id).unwrap();
    assert_eq!(record.outcome, OperationOutcome::Cancelled);
    assert_eq!(record.detail.as_deref(), Some("not issued"));
}

#[test]
fn resolve_by_id_leaves_already_resolved_record_untouched() {
    let mut log = OperationsLog::default();
    let id = log.begin("Start instance", "web-1");
    assert!(log.resolve_by_id(
        id,
        OperationOutcome::Succeeded,
        Some("accepted".to_string())
    ));
    let resolved = log.resolve_by_id(id, OperationOutcome::Unknown, Some("clobber".to_string()));
    assert!(!resolved);
    let record = log.records().iter().find(|r| r.id == id).unwrap();
    assert_eq!(record.outcome, OperationOutcome::Succeeded);
    assert_eq!(record.detail.as_deref(), Some("accepted"));
}

#[test]
fn resolve_by_id_none_detail_does_not_clear_existing_detail_while_pending() {
    // A Pending record's detail is always None right after `begin`, so the only
    // way to reach "Pending with a detail already set" through the public API
    // is to resolve once while re-asserting Pending as the outcome. That is a
    // legitimate call shape (the guard only cares that the record was Pending),
    // and it is the one path that actually exercises the assignment bug: an
    // unconditional `record.detail = detail` would let this second, detail-less
    // call erase the detail the first call recorded.
    let mut log = OperationsLog::default();
    let id = log.begin("Start instance", "web-1");
    assert!(log.resolve_by_id(
        id,
        OperationOutcome::Pending,
        Some("observed detail".to_string())
    ));

    let resolved = log.resolve_by_id(id, OperationOutcome::Pending, None);

    assert!(resolved);
    let record = log.records().iter().find(|r| r.id == id).unwrap();
    assert_eq!(record.outcome, OperationOutcome::Pending);
    assert_eq!(record.detail.as_deref(), Some("observed detail"));
}

#[test]
fn resolve_by_id_second_call_with_none_leaves_resolved_detail_intact() {
    // Once a record is resolved it is no longer Pending, so the guard alone
    // already blocks a second `resolve_by_id` call regardless of what it
    // passes for `detail`. This pins that no-op contract explicitly for the
    // `None` case, which is the shape the fix targets.
    let mut log = OperationsLog::default();
    let id = log.begin("Start instance", "web-1");
    assert!(log.resolve_by_id(
        id,
        OperationOutcome::Succeeded,
        Some("observed detail".to_string())
    ));

    let resolved_again = log.resolve_by_id(id, OperationOutcome::Unknown, None);

    assert!(!resolved_again);
    let record = log.records().iter().find(|r| r.id == id).unwrap();
    assert_eq!(record.outcome, OperationOutcome::Succeeded);
    assert_eq!(record.detail.as_deref(), Some("observed detail"));
}

#[test]
fn resolve_by_id_returns_false_for_unknown_id() {
    let mut log = OperationsLog::default();
    log.begin("Start instance", "web-1");
    assert!(!log.resolve_by_id(999, OperationOutcome::Cancelled, None));
}

#[test]
fn resolve_by_id_to_unknown_clears_pending() {
    let mut log = OperationsLog::default();
    let id = log.begin("Start instance", "web-1");
    assert_eq!(log.pending_count(), 1);
    assert!(log.resolve_by_id(id, OperationOutcome::Unknown, None));
    assert_eq!(log.pending_count(), 0);
}

#[test]
fn terminal_outcomes_have_labels() {
    assert_eq!(OperationOutcome::Cancelled.label(), "Cancelled");
    assert_eq!(OperationOutcome::Unknown.label(), "Unknown");
}

#[test]
fn capacity_reports_the_bound_the_log_actually_evicts_at() {
    // `capacity()` exists so tests can size a generated session against the
    // log's real bound instead of mirroring the literal. That is only sound
    // while the number it reports is the number `begin` starts evicting at, so
    // pin the two to each other rather than to a constant.
    let mut log = OperationsLog::default();
    let capacity = log.capacity();
    assert!(
        capacity > 0,
        "a zero-capacity log could never hold a record"
    );

    for index in 0..capacity {
        log.begin("Start instance", format!("web-{index}"));
    }
    assert_eq!(log.records().len(), capacity);
    assert_eq!(log.records().back().unwrap().target, "web-0");

    log.begin("Start instance", "one-too-many");
    assert_eq!(log.records().len(), capacity);
    assert_eq!(log.records().back().unwrap().target, "web-1");
}

#[test]
fn log_is_capped_and_keeps_most_recent() {
    let mut log = OperationsLog::default();
    for index in 0..15 {
        log.begin("Start instance", format!("web-{index}"));
    }
    assert_eq!(log.records().len(), 10);
    assert_eq!(log.records().front().unwrap().target, "web-14");
    assert_eq!(log.records().back().unwrap().target, "web-5");
}
