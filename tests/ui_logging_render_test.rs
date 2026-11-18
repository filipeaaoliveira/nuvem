use nuvem::app::AppState;
use nuvem::app::state::root::Workspace;
use nuvem::services::runtime::session::RuntimeSession;
use nuvem_domain::gcp::logging::{
    LogEntry, LogPayload, LogQuery, LogResourceKind, LogSeverity, LogTimeRange,
};
use ratatui::{Terminal, backend::TestBackend};

fn lines(terminal: &Terminal<TestBackend>) -> Vec<String> {
    let buffer = terminal.backend().buffer();
    (0..buffer.area.height)
        .map(|y| {
            (0..buffer.area.width)
                .map(|x| buffer.cell((x, y)).unwrap().symbol().to_string())
                .collect::<String>()
        })
        .collect()
}

fn entry(msg: &str) -> LogEntry {
    LogEntry {
        insert_id: "a".into(),
        timestamp: "2026-07-19T10:00:00Z".into(),
        severity: LogSeverity::Error,
        resource_type: "gce_instance".into(),
        resource_labels: Default::default(),
        log_name: "projects/p/logs/syslog".into(),
        message: msg.into(),
        payload: LogPayload::Empty,
        labels: Default::default(),
    }
}

#[test]
fn logging_render_sanitizes_and_shows_severity() {
    let mut app = AppState::new();
    app.root.workspace = Workspace::Logging;
    app.root
        .logging
        .apply_recent(vec![entry("bad\u{1b}[2Jrow")], 0);
    let session = RuntimeSession::new_for_test(app);

    let mut terminal = Terminal::new(TestBackend::new(120, 30)).unwrap();
    terminal
        .draw(|frame| nuvem::ui::render(frame, &session))
        .unwrap();
    let rendered = lines(&terminal).join("\n");

    assert!(!rendered.contains('\u{1b}'), "ESC byte leaked into buffer");
    assert!(rendered.contains("ERROR"), "severity label not rendered");
}

/// The detail view renders payload text and label values directly (not
/// through the table sanitization path exercised above), so it needs its
/// own coverage: `log_detail_view` must sanitize payload lines and label
/// key/value strings, otherwise a malicious log entry could smuggle an ESC
/// sequence into the terminal via the expanded entry view.
#[test]
fn logging_detail_render_sanitizes_payload_and_labels() {
    let mut entry = entry("trigger message");
    entry.payload = LogPayload::Text("payload\u{1b}[2Jline".into());
    entry
        .labels
        .insert("env".into(), "prod\u{1b}[2Jvalue".into());

    let mut app = AppState::new();
    app.root.workspace = Workspace::Logging;
    app.root.logging.apply_recent(vec![entry], 0);
    app.root.logging.selected = Some(0);
    app.root.logging.detail_open = true;
    let session = RuntimeSession::new_for_test(app);

    let mut terminal = Terminal::new(TestBackend::new(120, 30)).unwrap();
    terminal
        .draw(|frame| nuvem::ui::render(frame, &session))
        .unwrap();
    let rendered = lines(&terminal).join("\n");

    assert!(
        !rendered.contains('\u{1b}'),
        "ESC byte leaked into detail view buffer"
    );
    assert!(rendered.contains("Log entry"), "detail view not rendered");
}

/// With `wrap` enabled, a payload line longer than the pane width must be
/// reflowed across multiple rows so its tail is still visible, rather than
/// truncated with an ellipsis.
#[test]
fn logging_detail_wrap_shows_full_long_payload() {
    let long_line = format!("{}ZZENDMARKER", "x".repeat(130));
    let mut entry = entry("trigger message");
    entry.payload = LogPayload::Text(long_line);

    let mut app = AppState::new();
    app.root.workspace = Workspace::Logging;
    app.root.logging.apply_recent(vec![entry], 0);
    app.root.logging.selected = Some(0);
    app.root.logging.detail_open = true;
    app.root.logging.wrap = true;
    let session = RuntimeSession::new_for_test(app);

    let mut terminal = Terminal::new(TestBackend::new(120, 30)).unwrap();
    terminal
        .draw(|frame| nuvem::ui::render(frame, &session))
        .unwrap();
    // Join rows without a separator: the wrapped tail lands intact on the
    // next row, so the end marker is present in the buffer as a whole.
    let rendered: String = lines(&terminal).join("");

    assert!(
        rendered.contains("ZZENDMARKER"),
        "wrapped payload tail not rendered (was it truncated?)"
    );
}

/// Without `wrap`, the same over-long payload line is truncated with an
/// ellipsis, so its tail marker is absent.
#[test]
fn logging_detail_no_wrap_truncates_long_payload() {
    let long_line = format!("{}ZZENDMARKER", "x".repeat(130));
    let mut entry = entry("trigger message");
    entry.payload = LogPayload::Text(long_line);

    let mut app = AppState::new();
    app.root.workspace = Workspace::Logging;
    app.root.logging.apply_recent(vec![entry], 0);
    app.root.logging.selected = Some(0);
    app.root.logging.detail_open = true;
    app.root.logging.wrap = false;
    let session = RuntimeSession::new_for_test(app);

    let mut terminal = Terminal::new(TestBackend::new(120, 30)).unwrap();
    terminal
        .draw(|frame| nuvem::ui::render(frame, &session))
        .unwrap();
    let rendered: String = lines(&terminal).join("");

    assert!(
        !rendered.contains("ZZENDMARKER"),
        "expected truncated payload without wrap"
    );
}

/// The header teaches the whole narrowing toolkit: the current severity floor,
/// the since bucket's digit next to its label, and — with no search applied —
/// the search grammar's sigils, which are otherwise undiscoverable.
#[test]
fn logging_header_shows_floor_bucket_and_search_grammar_hint() {
    let mut app = AppState::new();
    app.root.workspace = Workspace::Logging;
    let session = RuntimeSession::new_for_test(app);

    let mut terminal = Terminal::new(TestBackend::new(120, 30)).unwrap();
    terminal
        .draw(|frame| nuvem::ui::render(frame, &session))
        .unwrap();
    let rendered = lines(&terminal).join("\n");

    assert!(
        rendered.contains("Severity: all"),
        "no-floor sentinel not rendered as `all`: {rendered}"
    );
    assert!(
        rendered.contains("Range: 1h [4]"),
        "bucket digit not rendered beside the range label: {rendered}"
    );
    for sigil in ["~fuzzy", "/regex/", "!invert"] {
        assert!(
            rendered.contains(sigil),
            "search grammar hint is missing {sigil}: {rendered}"
        );
    }
}

/// With a floor and a search applied, the header reports both, and the search
/// hint gives way to the mode the typed pattern actually selected.
#[test]
fn logging_header_shows_applied_floor_and_search_mode() {
    let mut app = AppState::new();
    app.root.workspace = Workspace::Logging;
    app.root.logging.severity_floor = LogSeverity::Error;
    app.root.logging.time_range = LogTimeRange::FiveMinutes;
    app.root.logging.start_filter();
    app.root.logging.filter_draft = "~api".to_string();
    app.root.logging.apply_filter();
    let session = RuntimeSession::new_for_test(app);

    let mut terminal = Terminal::new(TestBackend::new(120, 30)).unwrap();
    terminal
        .draw(|frame| nuvem::ui::render(frame, &session))
        .unwrap();
    let rendered = lines(&terminal).join("\n");

    assert!(
        rendered.contains("Severity: ERROR"),
        "severity floor not rendered: {rendered}"
    );
    assert!(
        rendered.contains("Range: 5m [1]"),
        "selected bucket not rendered: {rendered}"
    );
    assert!(
        rendered.contains("Search: ~api (fuzzy)"),
        "search mode not rendered: {rendered}"
    );
}

/// Every header line goes through `fit_text`, so a pane too narrow for the
/// text truncates with an ellipsis rather than spilling past the header block.
#[test]
fn logging_header_lines_are_truncated_to_the_pane_width() {
    let mut app = AppState::new();
    app.root.workspace = Workspace::Logging;
    app.root.logging.start_filter();
    app.root.logging.filter_draft = "z".repeat(200);
    app.root.logging.apply_filter();
    let session = RuntimeSession::new_for_test(app);

    // The narrowest width the frame still renders a workspace at.
    let width = 60;
    let mut terminal = Terminal::new(TestBackend::new(width, 30)).unwrap();
    terminal
        .draw(|frame| nuvem::ui::render(frame, &session))
        .unwrap();
    let rows = lines(&terminal);

    let header_rows: Vec<&String> = rows
        .iter()
        .filter(|row| row.starts_with("Project:") || row.starts_with("Search:"))
        .collect();
    assert_eq!(
        header_rows.len(),
        2,
        "expected the summary and search header rows, got {rows:?}"
    );
    for row in header_rows {
        assert_eq!(
            row.chars().count(),
            usize::from(width),
            "header row is not exactly one pane wide: {row:?}"
        );
        assert!(
            row.trim_end().ends_with("..."),
            "over-long header row was not truncated: {row:?}"
        );
    }
}

/// The severity floor and time bucket are server-side narrowings with no other
/// on-screen signal, so a pane too narrow for the whole summary must still
/// show them — otherwise a user sees fewer log lines with no visible reason.
/// They sit ahead of the query summary precisely so `fit_text` drops the query
/// first.
#[test]
fn narrow_header_keeps_the_severity_floor_and_bucket_visible() {
    let mut app = AppState::new();
    app.root.workspace = Workspace::Logging;
    app.root.logging.severity_floor = LogSeverity::Error;
    app.root.logging.query = LogQuery::Resource(LogResourceKind::ComputeInstance {
        instance_id: "1001".to_string(),
        instance_name: "a-very-long-compute-instance-name-that-crowds-the-header".to_string(),
        zone: "europe-west1-b".to_string(),
    });
    let session = RuntimeSession::new_for_test(app);

    // The narrowest width the frame still renders a workspace at.
    let mut terminal = Terminal::new(TestBackend::new(60, 30)).unwrap();
    terminal
        .draw(|frame| nuvem::ui::render(frame, &session))
        .unwrap();
    let rendered = lines(&terminal).join("\n");

    assert!(
        rendered.contains("Severity: ERROR"),
        "the floor was truncated away on a narrow pane: {rendered}"
    );
    assert!(
        rendered.contains("Range: 1h [4]"),
        "the bucket was truncated away on a narrow pane: {rendered}"
    );
}
