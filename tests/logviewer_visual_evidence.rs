//! Rendered-frame evidence — the same three follow-ups covered by
//! `logviewer_debt_evidence.rs`, but shown on the surface the operator
//! actually reads: full `nuvem::ui::render` frames driven by real key events
//! through the real keymap, reducer and `RuntimeSession`.
//!
//! The sibling file proves the properties through the public seams; this one
//! answers "what does the screen say", which for a stolen key, a spinner and a
//! mode label is the whole of the user-visible behaviour.
//!
//! Frames are written (ASCII screenshot plus a coloured cell dump, from which
//! the PNGs are rendered) only when `NUVEM_EVIDENCE_DIR` is set. The assertions
//! always run.

use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use nuvem::Config;
use nuvem::app::state::root::Workspace;
use nuvem::app::{Action, AppState, Effect, NavAction, WorkspaceAction};
use nuvem::input::keymap::action_for_key;
use nuvem::services::demo::DemoCommandRunner;
use nuvem::services::gcloud::cli::{CommandError, CommandOutput, CommandRunner, GcloudCommand};
use nuvem::services::runtime::input_context_for_test;
use nuvem::services::runtime::session::RuntimeSession;
use nuvem_domain::gcp::configuration::{GcloudConfiguration, GcloudConfigurationName};
use ratatui::layout::Rect;
use ratatui::{Terminal, backend::TestBackend};

const WIDTH: u16 = 132;
const HEIGHT: u16 = 26;

// ---------------------------------------------------------------------------
// A stand-in `gcloud logging read` — enough corpus that each search pattern
// below keeps a visibly different subset of rows.
// ---------------------------------------------------------------------------

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
    ("INFO", 52, "gce_instance", "[INFO] nginx: started"),
];

struct FakeGcloud {
    argv: Arc<Mutex<Vec<Vec<String>>>>,
}

impl CommandRunner for FakeGcloud {
    fn run(&self, command: GcloudCommand) -> Result<CommandOutput, CommandError> {
        self.argv.lock().unwrap().push(command.args.clone());
        let rows: Vec<String> = CORPUS
            .iter()
            .enumerate()
            .map(|(index, (severity, minutes_ago, resource_type, message))| {
                format!(
                    r#"{{"insertId":"log-{index:03}","timestamp":"{}","severity":"{severity}","logName":"projects/acme-prod/logs/stdout","resource":{{"type":"{resource_type}","labels":{{"instance_id":"6081246938160990230"}}}},"textPayload":{},"labels":{{"env":"prod"}}}}"#,
                    timestamp_for(*minutes_ago),
                    json_string(message),
                )
            })
            .collect();
        Ok(CommandOutput {
            stdout: format!("[{}]", rows.join(",")),
            stderr: String::new(),
        })
    }
}

/// A fixed wall clock so the frames are reproducible.
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
// Frame capture and evidence writing
// ---------------------------------------------------------------------------

fn evidence_dir() -> Option<PathBuf> {
    // No fallback: unset means this test writes nothing at all.
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
    let mut terminal = Terminal::new(TestBackend::new(WIDTH, HEIGHT)).expect("terminal");
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

fn line_containing<'a>(frame: &'a Frame, needle: &str) -> &'a str {
    frame
        .lines
        .iter()
        .find(|line| line.contains(needle))
        .map(String::as_str)
        .unwrap_or_else(|| panic!("no rendered line contains {needle:?}"))
}

// ---------------------------------------------------------------------------
// Driving a session with real key events
// ---------------------------------------------------------------------------

struct Viewer {
    session: RuntimeSession,
}

fn prod_config_row() -> GcloudConfiguration {
    GcloudConfiguration {
        name: GcloudConfigurationName::new("prod"),
        is_active: true,
        account: Some("sre@example.com".to_string()),
        project: Some("acme-prod".to_string()),
        region: Some("europe-west1".to_string()),
        zone: Some("europe-west1-b".to_string()),
    }
}

impl Viewer {
    /// A session in the Logging workspace with a resolved context.
    fn logging(config: Config) -> Viewer {
        let mut app = AppState::new();
        app.root.configurations.set_rows(vec![prod_config_row()]);
        app.root.workspace = Workspace::Logging;
        let session = RuntimeSession::new_with_config_and_runner_for_test(
            config,
            app,
            Box::new(FakeGcloud {
                argv: Arc::new(Mutex::new(Vec::new())),
            }),
        );
        Viewer { session }
    }

    /// A session with NO configuration loaded, so no context can resolve.
    fn contextless() -> Viewer {
        let session = RuntimeSession::new_with_runner_for_test(
            AppState::new(),
            Box::new(FakeGcloud {
                argv: Arc::new(Mutex::new(Vec::new())),
            }),
        );
        Viewer { session }
    }

    fn resolve(&self, key: KeyEvent) -> Option<Action> {
        let context = input_context_for_test(&self.session, Rect::new(0, 0, WIDTH, HEIGHT));
        action_for_key(key, &context)
    }

    async fn press(&mut self, key: KeyEvent) {
        let action = self
            .resolve(key)
            .unwrap_or_else(|| panic!("key {key:?} is unbound in this context"));
        self.session.dispatch_action(action).await;
    }

    async fn press_char(&mut self, ch: char) {
        self.press(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE))
            .await;
    }

    async fn press_fetch(&mut self, key: KeyEvent) {
        self.press(key).await;
        let applied = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            self.session.process_next_effect_result(),
        )
        .await
        .unwrap_or_else(|_| panic!("no result arrived for {key:?}"));
        assert!(applied, "effect-result channel closed");
        while let Ok(true) = tokio::time::timeout(
            std::time::Duration::from_millis(50),
            self.session.process_next_effect_result(),
        )
        .await
        {}
    }

    async fn type_search(&mut self, pattern: &str) {
        self.press_char('/').await;
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

    fn capture(&self) -> Frame {
        capture(&self.session)
    }
}

/// The instance rows currently held, in the order the table would show them.
fn instance_names(session: &RuntimeSession) -> Vec<String> {
    session
        .app()
        .root
        .resources
        .instances()
        .iter()
        .map(|instance| instance.name.clone())
        .collect()
}

/// Row order on screen is the table's own; these assertions are about *which*
/// rows survived the filter, so they compare the set.
fn sorted(mut rows: Vec<String>) -> Vec<String> {
    rows.sort();
    rows
}

fn ctrl(ch: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(ch), KeyModifiers::CONTROL)
}

// ---------------------------------------------------------------------------
// Item 3 — the search-mode label on screen, and the rows it describes
// ---------------------------------------------------------------------------

/// The header names the mode without compiling a regex, and the rows are
/// matched by the cached compile of the *same* source. The screen is where a
/// disagreement between those two would show, so that is where it is pinned.
#[tokio::test]
async fn the_rendered_search_label_names_the_mode_of_the_rows_on_screen() {
    // Scratch only — the collected evidence goes to `evidence_dir()` below.
    let home_dir = tempfile::tempdir().expect("home dir");
    let home = home_dir.path();
    let mut viewer = Viewer::logging(Config {
        log_file: home.join("nuvem.log"),
        ..Config::default()
    });

    viewer
        .press_fetch(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::NONE))
        .await;
    let frame = viewer.capture();
    assert!(line_containing(&frame, "Search:").contains("Search: none"));
    assert_eq!(viewer.visible_rows().len(), CORPUS.len());
    write_frame(
        "vis-01-logging-no-search",
        "Item 3, step 1 — the log viewer with no search applied. The search line \
spells out the grammar's sigils and all 7 fetched rows are on screen.",
        &frame,
    );

    // A bracketed level is a regex character class; the log grammar treats it
    // as a literal substring on purpose, and the label must say so.
    viewer.type_search("[ERROR]").await;
    let frame = viewer.capture();
    let search = line_containing(&frame, "Search:").to_string();
    assert!(
        search.contains("Search: [ERROR] (substring)"),
        "search line: {search}"
    );
    assert_eq!(
        sorted(viewer.visible_rows()),
        vec![
            "[ERROR] checkout: upstream returned 502 after 3 retries",
            "[ERROR] connection refused: upstream 10.0.0.7:8080 unreachable",
        ]
    );
    write_frame(
        "vis-02-search-substring",
        "Item 3, step 2 — `[ERROR]` is labelled (substring), not (regex), and the two \
rows on screen are the ones a literal-substring match keeps. The label is derived from the \
pure-syntax view with no regex compile; the rows come from the cached compile of that same \
source string.",
        &frame,
    );

    viewer.type_search("/ngi?nx/").await;
    let frame = viewer.capture();
    let search = line_containing(&frame, "Search:").to_string();
    assert!(
        search.contains("Search: /ngi?nx/ (regex)"),
        "search line: {search}"
    );
    assert_eq!(
        sorted(viewer.visible_rows()),
        vec![
            "[INFO] nginx: reloaded configuration in 12ms",
            "[INFO] nginx: started",
        ]
    );
    write_frame(
        "vis-03-search-regex",
        "Item 3, step 3 — the /.../ sigil is labelled (regex) and the rows are the \
regex's matches.",
        &frame,
    );

    viewer.type_search("!nginx").await;
    let frame = viewer.capture();
    let search = line_containing(&frame, "Search:").to_string();
    assert!(
        search.contains("Search: !nginx (not substring)"),
        "search line: {search}"
    );
    assert!(
        !viewer
            .visible_rows()
            .iter()
            .any(|row| row.contains("nginx"))
    );
    write_frame(
        "vis-04-search-inverted",
        "Item 3, step 4 — inversion is labelled (not substring) and every nginx row is \
gone from the table.",
        &frame,
    );

    // A pattern that IS syntactically a regex but does not compile: the label
    // must not name a mode the rows were never matched with.
    viewer.type_search("/[a/").await;
    let frame = viewer.capture();
    let search = line_containing(&frame, "Search:").to_string();
    assert!(
        search.contains("Search: /[a/ (") && !search.contains("(regex)"),
        "an uncompilable pattern must show its error, not a mode: {search}"
    );
    assert!(viewer.visible_rows().is_empty());
    write_frame(
        "vis-05-search-invalid-regex",
        "Item 3, step 5 — `/[a/` parses as regex syntax but does not compile. The \
header shows the compile error instead of a mode label, and no rows are claimed to match.",
        &frame,
    );
}

// ---------------------------------------------------------------------------
// Item 2 — a Logging deep link never strands the viewer at a spinner
// ---------------------------------------------------------------------------

/// Restoring the Logging location with no resolved context renders "Not
/// loaded", not a spinner nothing would ever clear. The context-resolved
/// counterpart is rendered beside it so "never spins" cannot be satisfied by
/// never showing a spinner at all.
#[tokio::test]
async fn a_restored_logging_deep_link_renders_settled_not_spinning() {
    let mut viewer = Viewer::contextless();
    viewer
        .session
        .dispatch_action(Action::Workspace(WorkspaceAction::Logging))
        .await;
    viewer
        .session
        .dispatch_action(Action::Workspace(WorkspaceAction::Home))
        .await;
    viewer
        .session
        .dispatch_action(Action::Navigation(NavAction::Back))
        .await;
    assert_eq!(viewer.session.app().root.workspace, Workspace::Logging);
    assert!(viewer.session.app().root.current_context().is_none());

    let frame = viewer.capture();
    let header = line_containing(&frame, "Severity:").to_string();
    assert!(
        header.contains("Not loaded"),
        "the restore must settle, not spin: {header}"
    );
    // The body pane is the largest thing on this screen; a spinner there says
    // the opposite of the header it sits under, so no reading of the frame is
    // true. Assert on the whole frame, not just the header line.
    assert!(
        !frame.lines.iter().any(|line| line.contains("Loading")),
        "nothing on a settled screen may claim a load is running:\n{}",
        frame.lines.join("\n")
    );
    assert!(
        frame
            .lines
            .iter()
            .any(|line| line.contains("No logs loaded")),
        "the body must say what the header says:\n{}",
        frame.lines.join("\n")
    );
    write_frame(
        "vis-06-deep-link-settles",
        "Item 2 — the Logging location restored from navigation history with no \
resolved gcloud context. No fetch was dispatched, so the header settles to 'Not loaded' and \
the body pane says 'No logs loaded'. Before the fix the header read 'Loading...' forever (the \
tail poll skips for the same missing-context reason, so nothing would ever clear it); after \
the first half of the fix the header settled but the body still spun, contradicting it.",
        &frame,
    );

    // The honest spinner: same restore, context resolved, fetch dispatched.
    let mut viewer = Viewer::logging(Config::default());
    viewer
        .session
        .dispatch_action(Action::Workspace(WorkspaceAction::Home))
        .await;
    viewer
        .session
        .dispatch_action(Action::Navigation(NavAction::Back))
        .await;
    assert_eq!(viewer.session.app().root.workspace, Workspace::Logging);
    let frame = viewer.capture();
    let header = line_containing(&frame, "Severity:").to_string();
    assert!(
        header.contains("Loading"),
        "a dispatched load must keep the spinner up: {header}"
    );
    assert!(
        frame
            .lines
            .iter()
            .any(|line| line.contains("Loading logs...")),
        "the body must spin with the header:\n{}",
        frame.lines.join("\n")
    );
    write_frame(
        "vis-07-deep-link-honest-spinner",
        "Item 2, counterpart — the same restore with a resolved context DOES dispatch \
the fetch, so 'Loading...' stands in the header and 'Loading logs...' in the body. The fix \
removed the lying spinner, not the spinner.",
        &frame,
    );
}

/// The very first Logging view, never loaded before, with the entry fetch
/// actually dispatched: it must read as loading everywhere until the result
/// lands. This is the frame a "settle it to Not loaded" over-correction would
/// break, and it is on the normal first-load path every operator takes.
#[tokio::test]
async fn the_first_ever_logging_view_spins_while_its_fetch_is_in_flight() {
    let mut viewer = Viewer::logging(Config::default());
    // Nothing has ever been fetched into this buffer.
    assert!(
        viewer
            .session
            .app()
            .root
            .logging
            .visible_entries()
            .is_empty()
    );
    viewer
        .session
        .dispatch_action(Action::Workspace(WorkspaceAction::Logging))
        .await;
    assert!(
        viewer.session.app().root.current_context().is_some(),
        "this case is the one WITH a context, so the fetch is really dispatched"
    );

    let frame = viewer.capture();
    let header = line_containing(&frame, "Severity:").to_string();
    assert!(
        header.contains("Loading"),
        "the first load is in flight: {header}"
    );
    assert!(
        frame
            .lines
            .iter()
            .any(|line| line.contains("Loading logs...")),
        "a never-yet-loaded buffer with a fetch running must still spin:\n{}",
        frame.lines.join("\n")
    );
    assert!(
        !frame
            .lines
            .iter()
            .any(|line| line.contains("No logs loaded")),
        "'not loaded' here would deny a fetch that IS running:\n{}",
        frame.lines.join("\n")
    );
    write_frame(
        "vis-12-first-load-spins",
        "Item 2, second counterpart — entering Logging for the first time with a \
resolved context. The fetch was dispatched and its result has not arrived, so both the header \
and the body spin. The 'Not loaded' body only ever appears when no load was dispatched.",
        &frame,
    );

    // ...and when the result lands the spinner goes, in both places.
    let applied = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        viewer.session.process_next_effect_result(),
    )
    .await
    .expect("no result arrived for the entry fetch");
    assert!(applied, "effect-result channel closed");
    let frame = viewer.capture();
    assert!(
        !frame.lines.iter().any(|line| line.contains("Loading")),
        "the spinner must clear once the rows are in:\n{}",
        frame.lines.join("\n")
    );
}

// ---------------------------------------------------------------------------
// Item 1 — Ctrl-S is one key, and it is the Logging save key
// ---------------------------------------------------------------------------

/// `Ctrl-S` saves the visible buffer in Logging and is unbound in the resource
/// list, where it used to fire `SortBySelectedColumn` because that arm matched
/// `KeyCode::Char('s')` without looking at the modifiers.
#[tokio::test]
async fn ctrl_s_saves_in_logging_and_does_nothing_in_the_resource_list() {
    // Scratch only — a per-test directory keeps the `log-dumps` count from
    // seeing another concurrent `cargo test` process's dump.
    let home_dir = tempfile::tempdir().expect("home dir");
    let home = home_dir.path();
    let mut viewer = Viewer::logging(Config {
        log_file: home.join("nuvem.log"),
        ..Config::default()
    });
    viewer
        .press_fetch(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::NONE))
        .await;

    viewer.press(ctrl('s')).await;
    let frame = viewer.capture();
    let saved_line = frame
        .lines
        .iter()
        .find(|line| line.contains("Saved") && line.contains("log entries"))
        .cloned()
        .unwrap_or_else(|| {
            panic!(
                "no save confirmation on screen:\n{}",
                frame.lines.join("\n")
            )
        });
    assert!(
        saved_line.contains(&format!("Saved {} log entries", CORPUS.len())),
        "save line: {saved_line}"
    );
    let dumps: Vec<_> = fs::read_dir(home.join("log-dumps"))
        .expect("log-dumps directory")
        .filter_map(Result::ok)
        .collect();
    assert_eq!(dumps.len(), 1, "exactly one dump should have been written");
    write_frame(
        "vis-08-ctrl-s-saves-in-logging",
        "Item 1 — Ctrl-S in the Logging list saves the visible buffer; the status line \
names the file it wrote. Ctrl-S is resolved ahead of every plain-character arm, so it is no \
longer read as the plain `s` severity-floor key.",
        &frame,
    );

    // Same chord in the resource list, over real rows from the demo fixtures:
    // unbound, so the row order on screen does not move. Plain `s` still does.
    let mut resources = Viewer {
        session: RuntimeSession::new_with_runner_for_test(
            AppState::new(),
            Box::new(DemoCommandRunner::new()),
        ),
    };
    resources
        .session
        .dispatch_action(Action::Effect(Effect::LoadConfigurations))
        .await;
    resources
        .session
        .dispatch_action(Action::Workspace(WorkspaceAction::Resources))
        .await;
    while let Ok(true) = tokio::time::timeout(
        std::time::Duration::from_millis(500),
        resources.session.process_next_effect_result(),
    )
    .await
    {}
    let loaded = instance_names(&resources.session);
    assert!(loaded.len() > 1, "need rows to sort: {loaded:?}");
    let frame = resources.capture();
    let before = frame.lines.clone();
    write_frame(
        "vis-09-resources-before-ctrl-s",
        "Item 1 — the Resources workspace with the GCE instance rows loaded, before any \
key is pressed. The footer teaches `s Sort column`.",
        &frame,
    );

    assert!(
        resources.resolve(ctrl('s')).is_none(),
        "Ctrl-S must not reach the resource-list sort arm"
    );
    let frame = resources.capture();
    assert_eq!(
        frame.lines, before,
        "an unbound chord cannot have changed the screen"
    );
    write_frame(
        "vis-10-resources-after-ctrl-s-unchanged",
        "Item 1 — after pressing Ctrl-S here. It resolves to no action at all, so the \
rows are in exactly the order above. Before the fix this same chord fired \
SortBySelectedColumn, because the sort arm matched KeyCode::Char('s') without looking at the \
modifiers — the Logging save key and the resource-list sort key were one chord.",
        &frame,
    );

    resources.press_char('s').await;
    let frame = resources.capture();
    assert_ne!(
        frame.lines, before,
        "plain `s` must still re-sort the resource table"
    );
    write_frame(
        "vis-11-resources-after-plain-s-sorted",
        "Item 1 — and plain `s` still sorts: the same rows come back in a different \
order. Only the modifier-blind dispatch was removed, not the binding.",
        &frame,
    );
}
