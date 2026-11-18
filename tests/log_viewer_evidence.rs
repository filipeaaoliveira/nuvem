//! Evidence generator — Cloud Logging viewer narrowing + export.
//!
//! Drives the four shipped features end-to-end the way a user does: real key
//! events resolved through `action_for_key` against the real input context,
//! reduced by the real reducer, dispatched as real effects through
//! `RuntimeSession` against a stand-in `gcloud` that honours the filter and
//! `--freshness` argv it is given, and rendered through the real
//! `nuvem::ui::render`. Each step writes an ASCII screenshot plus a colored
//! cell dump (rendered to PNG by the evidence tooling) and appends the exact
//! `gcloud` argv that ran to a transcript, when `NUVEM_EVIDENCE_DIR` is set.
//! Assertions always run.

use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use nuvem::Config;
use nuvem::app::AppState;
use nuvem::app::state::root::Workspace;
use nuvem::input::keymap::action_for_key;
use nuvem::services::gcloud::cli::{CommandError, CommandOutput, CommandRunner, GcloudCommand};
use nuvem::services::runtime::input_context_for_test;
use nuvem::services::runtime::session::RuntimeSession;
use nuvem_domain::gcp::configuration::{GcloudConfiguration, GcloudConfigurationName};
use nuvem_domain::gcp::logging::LogSeverity;
use ratatui::layout::Rect;
use ratatui::{Terminal, backend::TestBackend};

const WIDTH: u16 = 132;
const HEIGHT: u16 = 30;

// ---------------------------------------------------------------------------
// A stand-in `gcloud logging read` that actually obeys the argv it is handed.
// ---------------------------------------------------------------------------

/// One corpus row: (severity, minutes before the fixed "now", resource type,
/// message).
const CORPUS: &[(&str, u32, &str, &str)] = &[
    (
        "ERROR",
        1,
        "cloud_run_revision",
        "[ERROR] checkout: upstream returned 502 after 3 retries",
    ),
    (
        "WARNING",
        2,
        "gce_instance",
        "[WARN] disk usage on / is at 82% (threshold 80%)",
    ),
    (
        "INFO",
        4,
        "gce_instance",
        "[INFO] nginx: reloaded configuration in 12ms",
    ),
    (
        "ERROR",
        7,
        "k8s_container",
        "[ERROR] connection refused: upstream 10.0.0.7:8080 unreachable",
    ),
    (
        "CRITICAL",
        11,
        "cloud_run_revision",
        "[CRITICAL] payments: database pool exhausted, shedding load",
    ),
    (
        "INFO",
        13,
        "k8s_container",
        "[INFO] scaled deployment api from 3 to 5 replicas",
    ),
    (
        "DEBUG",
        22,
        "gce_instance",
        "[DEBUG] cache warm-up finished for 1024 keys",
    ),
    (
        "WARNING",
        31,
        "cloud_run_revision",
        "[WARN] request took 1204ms (p99 budget 800ms)",
    ),
    (
        "ERROR",
        44,
        "k8s_container",
        "[ERROR] readiness probe failed: HTTP 503 from /healthz",
    ),
    ("INFO", 52, "gce_instance", "[INFO] nginx: started"),
    (
        "NOTICE",
        58,
        "gce_instance",
        "[NOTICE] systemd: reloading configuration",
    ),
];

/// Stand-in for the `gcloud` binary. Records every argv it is asked to run and
/// answers `logging read` from [`CORPUS`], applying the `severity>=` clause in
/// the positional filter and the `--freshness` window itself — so an entry the
/// viewer never asked the server for is never returned, and a narrowing that
/// failed to reach the argv would visibly fail to narrow the screen.
struct FakeGcloud {
    argv: Arc<Mutex<Vec<Vec<String>>>>,
}

impl CommandRunner for FakeGcloud {
    fn run(&self, command: GcloudCommand) -> Result<CommandOutput, CommandError> {
        let args = command.args.clone();
        self.argv.lock().unwrap().push(args.clone());

        let floor = args
            .iter()
            .find_map(|arg| {
                arg.split(" AND ")
                    .find_map(|c| c.strip_prefix("severity>="))
            })
            .map(LogSeverity::from_api)
            .unwrap_or(LogSeverity::Default);
        let window_minutes = args
            .iter()
            .find_map(|arg| arg.strip_prefix("--freshness="))
            .map(parse_freshness_minutes)
            .unwrap_or(60);

        let mut rows: Vec<String> = Vec::new();
        for (index, (severity, minutes_ago, resource_type, message)) in CORPUS.iter().enumerate() {
            if LogSeverity::from_api(severity).rank() < floor.rank() {
                continue;
            }
            if *minutes_ago > window_minutes {
                continue;
            }
            rows.push(format!(
                r#"{{"insertId":"log-{index:03}","timestamp":"{}","severity":"{severity}","logName":"projects/acme-prod/logs/stdout","resource":{{"type":"{resource_type}","labels":{{"instance_id":"6081246938160990230"}}}},"textPayload":{},"labels":{{"env":"prod"}}}}"#,
                timestamp_for(*minutes_ago),
                json_string(message),
            ));
        }
        // gcloud is asked for `--order=desc`; the corpus is newest-first
        // already, so it goes out as-is.
        Ok(CommandOutput {
            stdout: format!("[{}]", rows.join(",")),
            stderr: String::new(),
        })
    }
}

fn parse_freshness_minutes(value: &str) -> u32 {
    let (digits, unit) = value.split_at(value.len() - 1);
    let amount: u32 = digits.parse().expect("freshness amount");
    match unit {
        "m" => amount,
        "h" => amount * 60,
        other => panic!("unexpected freshness unit {other}"),
    }
}

/// A fixed wall clock so screenshots are reproducible: 2026-07-22T12:00:00Z
/// minus `minutes_ago`.
fn timestamp_for(minutes_ago: u32) -> String {
    let total = 12 * 60 - minutes_ago as i32;
    format!(
        "2026-07-22T{:02}:{:02}:00.000000000Z",
        total / 60,
        total % 60
    )
}

fn json_string(value: &str) -> String {
    let escaped = value.replace('\\', "\\\\").replace('"', "\\\"");
    format!("\"{escaped}\"")
}

// ---------------------------------------------------------------------------
// Evidence writing
// ---------------------------------------------------------------------------

fn evidence_dir() -> Option<PathBuf> {
    let dir = std::env::var("NUVEM_EVIDENCE_DIR").ok()?;
    let path = PathBuf::from(dir);
    fs::create_dir_all(&path).ok()?;
    Some(path)
}

struct Frame {
    lines: Vec<String>,
    /// One `(text, fg, bold)` run per contiguous same-style span, per row.
    runs: Vec<Vec<(String, String, bool)>>,
}

fn capture(session: &RuntimeSession) -> Frame {
    let backend = TestBackend::new(WIDTH, HEIGHT);
    let mut terminal = Terminal::new(backend).expect("terminal");
    terminal
        .draw(|frame| nuvem::ui::render(frame, session))
        .expect("render");
    let buffer = terminal.backend().buffer();
    let mut lines = Vec::new();
    let mut runs = Vec::new();
    for y in 0..buffer.area.height {
        let mut line = String::new();
        let mut row: Vec<(String, String, bool)> = Vec::new();
        for x in 0..buffer.area.width {
            let cell = buffer.cell((x, y)).expect("cell");
            let symbol = cell.symbol().to_string();
            line.push_str(&symbol);
            let style = (
                format!("{:?}", cell.fg),
                cell.modifier.contains(ratatui::style::Modifier::BOLD),
            );
            match row.last_mut() {
                Some(last) if last.1 == style.0 && last.2 == style.1 => last.0.push_str(&symbol),
                _ => row.push((symbol, style.0, style.1)),
            }
        }
        lines.push(line.trim_end().to_string());
        runs.push(row);
    }
    Frame { lines, runs }
}

/// Writes an ASCII screenshot plus a JSON cell dump (consumed by the PNG
/// renderer) for one step of the walkthrough.
fn write_frame(name: &str, caption: &str, frame: &Frame) {
    let Some(dir) = evidence_dir() else {
        return;
    };
    let width = frame
        .lines
        .iter()
        .map(|l| l.chars().count())
        .max()
        .unwrap_or(0);
    let mut body = format!("{caption}\n\n+{}+\n", "-".repeat(width));
    for line in &frame.lines {
        let pad = width - line.chars().count();
        body.push_str(&format!("|{line}{}|\n", " ".repeat(pad)));
    }
    body.push_str(&format!("+{}+\n", "-".repeat(width)));
    fs::write(dir.join(format!("{name}.txt")), body).expect("write ascii frame");

    let mut json = String::from("{\n");
    json.push_str(&format!("  \"caption\": {},\n", json_string(caption)));
    json.push_str("  \"rows\": [\n");
    for (index, row) in frame.runs.iter().enumerate() {
        let cells: Vec<String> = row
            .iter()
            .map(|(text, fg, bold)| {
                format!(
                    "{{\"t\":{},\"fg\":{},\"b\":{}}}",
                    json_string(text),
                    json_string(fg),
                    bold
                )
            })
            .collect();
        json.push_str(&format!(
            "    [{}]{}\n",
            cells.join(","),
            if index + 1 == frame.runs.len() {
                ""
            } else {
                ","
            }
        ));
    }
    json.push_str("  ]\n}\n");
    fs::write(dir.join(format!("{name}.json")), json).expect("write cell dump");
}

fn append_transcript(text: &str) {
    let Some(dir) = evidence_dir() else {
        return;
    };
    let path = dir.join("gcloud-argv-transcript.txt");
    let mut existing = fs::read_to_string(&path).unwrap_or_default();
    existing.push_str(text);
    existing.push('\n');
    fs::write(path, existing).expect("write transcript");
}

fn shell_form(args: &[String]) -> String {
    let rendered: Vec<String> = args
        .iter()
        .map(|arg| {
            if arg.contains(' ') || arg.contains('"') {
                format!("'{arg}'")
            } else {
                arg.clone()
            }
        })
        .collect();
    format!("gcloud {}", rendered.join(" "))
}

// ---------------------------------------------------------------------------
// Driving the session with real key events
// ---------------------------------------------------------------------------

struct Viewer {
    session: RuntimeSession,
    argv: Arc<Mutex<Vec<Vec<String>>>>,
}

impl Viewer {
    fn new(config: Config) -> Viewer {
        let mut app = AppState::new();
        app.root.configurations.set_rows(vec![GcloudConfiguration {
            name: GcloudConfigurationName::new("prod"),
            is_active: true,
            account: Some("sre@example.com".to_string()),
            project: Some("acme-prod".to_string()),
            region: Some("europe-west1".to_string()),
            zone: Some("europe-west1-b".to_string()),
        }]);
        app.root.workspace = Workspace::Logging;
        let argv = Arc::new(Mutex::new(Vec::new()));
        let session = RuntimeSession::new_with_config_and_runner_for_test(
            config,
            app,
            Box::new(FakeGcloud { argv: argv.clone() }),
        );
        Viewer { session, argv }
    }

    /// Sends one real key event through the real input context + keymap.
    async fn press(&mut self, key: KeyEvent) {
        let context = input_context_for_test(&self.session, Rect::new(0, 0, WIDTH, HEIGHT));
        let action = action_for_key(key, &context)
            .unwrap_or_else(|| panic!("key {key:?} is unbound in this context"));
        self.session.dispatch_action(action).await;
    }

    /// A key press that moves the query window: after dispatching, the fetch it
    /// triggered is awaited and applied, so the next screenshot shows the
    /// server's answer rather than the spinner.
    async fn press_fetch(&mut self, key: KeyEvent) {
        self.press(key).await;
        let applied = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            self.session.process_next_effect_result(),
        )
        .await
        .unwrap_or_else(|_| panic!("no log fetch arrived for {key:?}"));
        assert!(applied, "effect-result channel closed");
        // Anything else already queued (a follow-up load) is applied too.
        while let Ok(true) = tokio::time::timeout(
            std::time::Duration::from_millis(50),
            self.session.process_next_effect_result(),
        )
        .await
        {}
    }

    async fn press_char(&mut self, ch: char) {
        self.press(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE))
            .await;
    }

    /// A plain character key that moves the query window (`s`, `S`, `0`..`6`,
    /// `r`).
    async fn press_char_fetch(&mut self, ch: char) {
        self.press_fetch(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE))
            .await;
    }

    async fn type_search(&mut self, pattern: &str) {
        self.press_char('/').await;
        // `/` seeds the draft with the applied pattern; clear it first.
        for _ in 0..48 {
            self.press(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE))
                .await;
        }
        for ch in pattern.chars() {
            self.press(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE))
                .await;
        }
        self.press(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
            .await;
    }

    fn last_argv(&self) -> Vec<String> {
        self.argv
            .lock()
            .unwrap()
            .last()
            .cloned()
            .unwrap_or_default()
    }

    fn capture(&self) -> Frame {
        capture(&self.session)
    }

    fn visible_rows(&self) -> Vec<String> {
        self.session
            .app()
            .root
            .logging
            .visible_entries()
            .into_iter()
            .map(|entry| entry.message.clone())
            .collect()
    }
}

fn header_line(frame: &Frame) -> String {
    frame
        .lines
        .iter()
        .find(|line| line.contains("Severity:"))
        .cloned()
        .unwrap_or_default()
}

fn search_line(frame: &Frame) -> String {
    frame
        .lines
        .iter()
        .find(|line| line.contains("Search:"))
        .cloned()
        .unwrap_or_default()
}

/// The whole walkthrough in the order a user would perform it.
#[tokio::test]
async fn walkthrough_evidence() {
    // Scratch only — the collected evidence goes to `evidence_dir()` below. A
    // per-test directory keeps the `log-dumps` count from seeing another
    // concurrent `cargo test` process's dump.
    let home_dir = tempfile::tempdir().expect("home dir");
    let home = home_dir.path();
    let config = Config {
        log_file: home.join("nuvem.log"),
        ..Config::default()
    };
    let mut viewer = Viewer::new(config);

    append_transcript(
        "Every `gcloud logging read` the viewer issued during the walkthrough,\n\
         recorded at the CommandRunner boundary (the exact argv the process would exec).\n",
    );

    // ---- 1. Baseline: no severity floor, default 1h window -----------------
    viewer.press_char_fetch('r').await;
    let frame = viewer.capture();
    let header = header_line(&frame);
    assert!(header.contains("Severity: all"), "header: {header}");
    assert!(header.contains("Range: 1h [4]"), "header: {header}");
    let argv = viewer.last_argv();
    assert!(
        !argv.iter().any(|arg| arg.contains("severity>=")),
        "no floor must leave the argv untouched: {argv:?}"
    );
    assert!(argv.contains(&"--freshness=1h".to_string()), "{argv:?}");
    assert_eq!(viewer.visible_rows().len(), 11);
    append_transcript(&format!(
        "\n[1] baseline — no severity floor, default 1h window (11 entries)\n    {}",
        shell_form(&argv)
    ));
    write_frame(
        "01-baseline-no-floor-1h",
        "Step 1 — the log viewer as it behaves with NO narrowing applied. \
Severity reads 'all' (the LogSeverity::Default sentinel), Range reads '1h [4]' \
(the default bucket and the digit that selects it), and the emitted gcloud argv \
carries no severity clause at all, so default behaviour is byte-identical to before.",
        &frame,
    );

    // ---- 2. `s` raises the severity floor to ERROR -------------------------
    for _ in 0..5 {
        viewer.press_char_fetch('s').await;
    }
    let frame = viewer.capture();
    let header = header_line(&frame);
    assert!(header.contains("Severity: ERROR"), "header: {header}");
    let argv = viewer.last_argv();
    assert!(
        argv.contains(&"severity>=ERROR".to_string()),
        "the floor must be pushed into the gcloud filter: {argv:?}"
    );
    let rows = viewer.visible_rows();
    assert_eq!(rows.len(), 4, "server-side narrowing to ERROR+: {rows:?}");
    assert!(
        rows.iter()
            .all(|row| row.contains("ERROR") || row.contains("CRITICAL"))
    );
    append_transcript(&format!(
        "\n[2] `s` x5 raises the floor to ERROR — narrowing is SERVER-SIDE (4 entries)\n    {}",
        shell_form(&argv)
    ));
    write_frame(
        "02-severity-floor-error",
        "Step 2 — pressing `s` five times raises the severity floor to ERROR. \
The header reads 'Severity: ERROR' and the re-issued gcloud command now carries \
the positional filter `severity>=ERROR`: this is real server-side narrowing, not \
display filtering — the INFO/DEBUG/NOTICE/WARNING entries were never fetched.",
        &frame,
    );

    // ---- 3. `S` lowers the floor back to no-floor --------------------------
    for _ in 0..5 {
        viewer.press_char_fetch('S').await;
    }
    let frame = viewer.capture();
    assert!(header_line(&frame).contains("Severity: all"));
    let argv = viewer.last_argv();
    assert!(!argv.iter().any(|arg| arg.contains("severity>=")));
    assert_eq!(viewer.visible_rows().len(), 11);
    append_transcript(&format!(
        "\n[3] `S` x5 lowers the floor back to the no-floor sentinel (11 entries again)\n    {}",
        shell_form(&argv)
    ));
    write_frame(
        "03-severity-floor-lowered-back",
        "Step 3 — `S` walks the floor back down; at 'all' the severity clause \
disappears from the argv again and every entry returns.",
        &frame,
    );

    // ---- 4. Digit `2` jumps straight to the 15m bucket ---------------------
    viewer.press_char_fetch('2').await;
    let frame = viewer.capture();
    let header = header_line(&frame);
    assert!(header.contains("Range: 15m [2]"), "header: {header}");
    let argv = viewer.last_argv();
    assert!(argv.contains(&"--freshness=15m".to_string()), "{argv:?}");
    let rows = viewer.visible_rows();
    assert_eq!(rows.len(), 6, "15m window: {rows:?}");
    append_transcript(&format!(
        "\n[4] digit `2` selects the 15m since-bucket (6 entries within the window)\n    {}",
        shell_form(&argv)
    ));
    write_frame(
        "04-since-bucket-15m",
        "Step 4 — k9s-style since-time buckets. Pressing `2` jumps straight to \
the 15m window: the header reads 'Range: 15m [2]' and gcloud is re-invoked with \
`--freshness=15m`, so only the six entries inside the window come back.",
        &frame,
    );

    // Digits 0-6 each map to their documented bucket, live.
    let mut bucket_report = String::from(
        "The seven since-buckets, each driven by its digit key through the \
real keymap/reducer/runtime, showing the --freshness argv gcloud actually received:\n\n",
    );
    for (digit, expected) in [
        ('0', "1m"),
        ('1', "5m"),
        ('2', "15m"),
        ('3', "30m"),
        ('4', "1h"),
        ('5', "6h"),
        ('6', "24h"),
    ] {
        viewer.press_char_fetch(digit).await;
        let argv = viewer.last_argv();
        assert!(
            argv.contains(&format!("--freshness={expected}")),
            "digit {digit} must fetch {expected}: {argv:?}"
        );
        let header = header_line(&viewer.capture());
        assert!(
            header.contains(&format!("Range: {expected} [{digit}]")),
            "header: {header}"
        );
        bucket_report.push_str(&format!(
            "  key `{digit}`  ->  header \"Range: {expected} [{digit}]\"  ->  {}\n",
            shell_form(&argv)
        ));
    }
    append_transcript(&format!("\n[5] every since-bucket\n{bucket_report}"));

    // Back to 1h for the search walkthrough.
    viewer.press_char_fetch('4').await;
    assert_eq!(viewer.visible_rows().len(), 11);

    // ---- 5. Search grammar -------------------------------------------------
    viewer.type_search("[ERROR]").await;
    let frame = viewer.capture();
    let line = search_line(&frame);
    assert!(
        line.contains("Search: [ERROR] (substring)"),
        "search: {line}"
    );
    let rows = viewer.visible_rows();
    assert_eq!(rows.len(), 3, "bracketed level search: {rows:?}");
    assert!(rows.iter().all(|row| row.contains("[ERROR]")));
    write_frame(
        "05-search-substring-bracketed-level",
        "Step 5 — the `/` search defaults to a case-insensitive SUBSTRING, which \
is the deliberate divergence from the sibling resource-table filter's regex-first \
grammar. Typing the most common log search there is — a bracketed level, `[ERROR]` \
— matches literally: exactly the three [ERROR] lines, and the header confirms the \
mode it chose ('substring').",
        &frame,
    );

    viewer.type_search("/[ERROR]/").await;
    let frame = viewer.capture();
    let line = search_line(&frame);
    assert!(line.contains("(regex)"), "search: {line}");
    let regex_rows = viewer.visible_rows().len();
    assert!(
        regex_rows > 3,
        "the same text as a regex is a character class and over-matches: {regex_rows}"
    );
    write_frame(
        "06-search-regex-shows-why-substring-is-default",
        &format!(
            "Step 6 — the SAME text opted explicitly into regex with `/…/`. \
`[ERROR]` is a character class there, so it matches {regex_rows} of the 11 rows \
instead of the 3 the user meant — the concrete reason substring is the default \
and regex is opt-in. The header names the mode ('regex') so the reading is never \
guessed at.",
        ),
        &frame,
    );

    viewer.type_search("~dbpool").await;
    let frame = viewer.capture();
    let line = search_line(&frame);
    assert!(line.contains("(fuzzy)"), "search: {line}");
    let rows = viewer.visible_rows();
    assert_eq!(rows.len(), 1, "fuzzy subsequence: {rows:?}");
    assert!(rows[0].contains("database pool exhausted"));
    write_frame(
        "07-search-fuzzy",
        "Step 7 — `~` switches to a fuzzy subsequence match: `~dbpool` finds \
'…payments: database pool exhausted…' without the user typing the words in full. \
Header: 'Search: ~dbpool (fuzzy)'.",
        &frame,
    );

    viewer.type_search("!nginx").await;
    let frame = viewer.capture();
    let line = search_line(&frame);
    assert!(line.contains("(not substring)"), "search: {line}");
    let rows = viewer.visible_rows();
    assert_eq!(
        rows.len(),
        9,
        "inversion hides the two nginx rows: {rows:?}"
    );
    assert!(rows.iter().all(|row| !row.contains("nginx")));
    write_frame(
        "08-search-inverted",
        "Step 8 — a leading `!` inverts any of the three modes. `!nginx` hides \
the two nginx lines and keeps the other nine; the header spells out the composed \
mode ('not substring').",
        &frame,
    );

    viewer.type_search("/[unclosed/").await;
    let frame = viewer.capture();
    let line = search_line(&frame);
    assert!(line.contains("invalid regex"), "search: {line}");
    assert!(
        viewer.visible_rows().is_empty(),
        "an unparseable pattern shows zero rows rather than everything"
    );
    write_frame(
        "09-search-invalid-regex-reports-the-reason",
        "Step 9 — an INVALID regex shows zero rows (never silently 'everything') \
and the header reports the reason in readable prose rather than a raw compiler \
dump, so the user can fix the pattern.",
        &frame,
    );

    // ---- 6. Ctrl-S exports exactly what is visible -------------------------
    viewer.type_search("[ERROR]").await;
    assert_eq!(viewer.visible_rows().len(), 3);
    viewer
        .press(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL))
        .await;
    let frame = viewer.capture();
    let status = viewer
        .session
        .app()
        .root
        .system
        .status_message
        .clone()
        .expect("save status message");
    assert!(status.contains("3 log entries"), "status: {status}");

    let dumps: Vec<PathBuf> = fs::read_dir(home.join("log-dumps"))
        .expect("log-dumps directory created beside the log file")
        .map(|entry| entry.expect("dir entry").path())
        .collect();
    assert_eq!(dumps.len(), 1, "one dump written: {dumps:?}");
    let dump_path = &dumps[0];
    assert!(dump_path.is_absolute());
    assert!(
        status.contains(&dump_path.display().to_string()),
        "the status must name the absolute path: {status}"
    );
    let written = fs::read_to_string(dump_path).expect("dump readable");
    let dumped_lines: Vec<&str> = written.lines().collect();
    assert_eq!(
        dumped_lines.len(),
        3,
        "the dump holds the FILTERED buffer, not all 11 entries: {written}"
    );
    assert!(dumped_lines.iter().all(|line| line.contains("[ERROR]")));
    assert!(
        dump_path.starts_with(home),
        "the dump lands beside the log file, never in the CWD: {dump_path:?}"
    );
    write_frame(
        "10-export-ctrl-s",
        "Step 10 — Ctrl-S saves the CURRENTLY VISIBLE (filtered) buffer. The \
status line reports how many entries were written and the ABSOLUTE path of the \
dump, which lands in a `log-dumps` directory beside the app's own log file — \
never in the working directory.",
        &frame,
    );
    // ---- 7. The new keys are discoverable from the help overlay ----------
    viewer.press_char('?').await;
    let frame = viewer.capture();
    let joined = frame.lines.join("\n");
    for hint in [
        "Raise severity floor",
        "Lower severity floor",
        "Since bucket",
        "Save visible logs",
    ] {
        assert!(joined.contains(hint), "help overlay is missing {hint}");
    }
    write_frame(
        "12-help-overlay-teaches-the-new-keys",
        "Step 11 — the four new controls are discoverable, not folklore: the \
help overlay (`?`) in the log list teaches `s`/`S` (raise/lower the severity \
floor), `0-6` (since bucket, one entry for the whole contiguous digit row) and \
`Ctrl-S` (save visible logs).",
        &frame,
    );

    append_transcript(&format!(
        "\n[6] Ctrl-S with the `[ERROR]` search applied\n    status: {status}\n    file:   {}\n    contents:\n{}",
        dump_path.display(),
        written
            .lines()
            .map(|line| format!("      {line}"))
            .collect::<Vec<_>>()
            .join("\n")
    ));
}

/// Ctrl-S with a log file that names no directory reports a failure rather
/// than dropping a dump into whatever directory the TUI was started in.
#[tokio::test]
async fn export_refuses_the_working_directory_evidence() {
    let config = Config {
        log_file: PathBuf::from("nuvem.log"),
        ..Config::default()
    };
    let mut viewer = Viewer::new(config);
    viewer.press_char_fetch('r').await;
    assert_eq!(viewer.visible_rows().len(), 11);

    viewer
        .press(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL))
        .await;
    let frame = viewer.capture();
    let status = viewer
        .session
        .app()
        .root
        .system
        .status_message
        .clone()
        .unwrap_or_default();
    assert!(
        status.to_lowercase().contains("save") || status.to_lowercase().contains("nowhere"),
        "the failure must name the save: {status}"
    );
    assert!(
        !PathBuf::from("log-dumps").exists(),
        "a relative log-dumps directory must never be created in the CWD"
    );
    write_frame(
        "11-export-failure-never-writes-to-cwd",
        &format!(
            "With a bare `--log-file nuvem.log` there is no directory for the \
dump to sit beside. Writing to the working directory was explicitly ruled out as \
surprising from a TUI, so Ctrl-S reports a SAVE FAILURE instead of falling back: \
\"{status}\". No `log-dumps` directory is created in the CWD.",
        ),
        &frame,
    );
}
