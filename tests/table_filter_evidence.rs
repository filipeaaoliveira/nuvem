//! Evidence generator — the multi-mode `/` filter grammar on the
//! resource tables.
//!
//! Drives the feature exactly the way an operator does: real key events (`/`,
//! the literal characters of the filter, Enter) resolved through the real
//! input context and `action_for_key`, reduced by the real reducer, dispatched
//! as real effects through `RuntimeSession` against a stand-in `gcloud` that
//! actually *evaluates* the `--filter` expression it is handed, and rendered
//! through the real `nuvem::ui::render`.
//!
//! Because the fake server honours the pushed expression, a narrowing that
//! failed to reach the argv would visibly fail to narrow the screen, and a
//! pushdown that was not a superset of the client predicate would visibly drop
//! rows. Each step writes an ASCII screenshot plus a colored cell dump
//! (rendered to PNG by the evidence tooling) and appends the exact `gcloud`
//! argv that ran to a transcript, when `NUVEM_EVIDENCE_DIR` is set. Assertions
//! always run.

use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use nuvem::Config;
use nuvem::app::state::root::Workspace;
use nuvem::app::{Action, AppState, ResourcesAction};
use nuvem::input::keymap::action_for_key;
use nuvem::services::gcloud::cli::{CommandError, CommandOutput, CommandRunner, GcloudCommand};
use nuvem::services::runtime::input_context_for_test;
use nuvem::services::runtime::session::RuntimeSession;
use nuvem_domain::gcp::configuration::{GcloudConfiguration, GcloudConfigurationName};
use nuvem_domain::gcp::resource::ResourceServiceKind;
use ratatui::layout::Rect;
use ratatui::{Terminal, backend::TestBackend};

/// Wide enough that the tools line never truncates: the longest inline filter
/// error plus the row counts have to fit in the *content* column, which is
/// what is left after the service navigator and preview panes take their share.
const WIDTH: u16 = 178;
const HEIGHT: u16 = 26;

// ---------------------------------------------------------------------------
// The corpus the stand-in `gcloud` serves
// ---------------------------------------------------------------------------

/// One compute instance as the *server* holds it: raw status string, bare zone
/// and machine-type names (the JSON builder wraps them back into the resource
/// URLs gcloud emits), and the `env`/`team` labels that only exist server-side.
struct Instance {
    name: &'static str,
    zone: &'static str,
    status: &'static str,
    machine: &'static str,
    internal_ip: &'static str,
    external_ip: &'static str,
    env: &'static str,
    team: &'static str,
}

/// The compute corpus.
///
/// `db-replica-01` is deliberately `REPAIRING`: an unmapped raw status parses
/// to `ResourceStatus::Unknown("REPAIRING")`, whose display cell reads
/// `Unknown` while the raw string stays searchable. That row is what proves
/// the never-drop-a-row gate covers a status with no display label of its own.
const INSTANCES: &[Instance] = &[
    Instance {
        name: "api-web-01",
        zone: "europe-west1-b",
        status: "RUNNING",
        machine: "e2-medium",
        internal_ip: "10.0.0.4",
        external_ip: "34.10.10.10",
        env: "prod",
        team: "payments",
    },
    Instance {
        name: "api-web-02",
        zone: "europe-west1-c",
        status: "RUNNING",
        machine: "e2-medium",
        internal_ip: "10.0.0.5",
        external_ip: "",
        env: "prod",
        team: "payments",
    },
    Instance {
        name: "cache-redis-01",
        zone: "europe-west1-b",
        status: "TERMINATED",
        machine: "e2-small",
        internal_ip: "10.0.0.6",
        external_ip: "",
        env: "staging",
        team: "platform",
    },
    Instance {
        name: "db-primary-01",
        zone: "europe-west4-a",
        status: "RUNNING",
        machine: "n2-standard-4",
        internal_ip: "10.0.1.4",
        external_ip: "",
        env: "prod",
        team: "platform",
    },
    Instance {
        name: "db-replica-01",
        zone: "europe-west4-a",
        status: "REPAIRING",
        machine: "n2-standard-4",
        internal_ip: "10.0.1.5",
        external_ip: "",
        env: "prod",
        team: "platform",
    },
    Instance {
        name: "worker-batch-01",
        zone: "us-central1-a",
        status: "SUSPENDED",
        machine: "e2-small",
        internal_ip: "10.0.2.4",
        external_ip: "",
        env: "staging",
        team: "batch",
    },
    Instance {
        name: "web-proxy-01",
        zone: "us-central1-a",
        status: "RUNNING",
        machine: "e2-micro",
        internal_ip: "10.0.2.5",
        external_ip: "35.20.20.20",
        env: "prod",
        team: "platform",
    },
];

/// Cloud Run services: (name, region, URL). Cloud Run exposes no user labels
/// in list output, which is the whole point of the step that filters it by
/// label.
const RUN_SERVICES: &[(&str, &str, &str)] = &[
    (
        "orders-api",
        "europe-west1",
        "https://orders-api-abcde-ew.a.run.app",
    ),
    (
        "checkout-web",
        "europe-west1",
        "https://checkout-web-fghij-ew.a.run.app",
    ),
];

/// Cloud Storage buckets: (name, location, storage class).
const BUCKETS: &[(&str, &str, &str)] = &[
    ("acme-prod-logs", "EUROPE-WEST1", "STANDARD"),
    ("acme-prod-assets", "EU", "STANDARD"),
    ("acme-staging-logs", "US", "NEARLINE"),
];

// ---------------------------------------------------------------------------
// A stand-in `gcloud` that actually evaluates `--filter`
// ---------------------------------------------------------------------------

/// Records every argv, and answers `compute instances list` by *evaluating*
/// the `--filter` expression against the raw corpus fields — the same way the
/// real server would. It understands exactly the two shapes this feature
/// emits:
///
/// - `f ~ "(?i)pat" OR g ~ "(?i)pat" ...` — regex partial match, any clause
/// - `labels.k = "v" AND labels.j = "w"` — label equality, every clause
///
/// Anything else panics rather than silently matching everything, so a
/// malformed pushdown cannot masquerade as a passing screenshot.
struct FakeGcloud {
    argv: Arc<Mutex<Vec<Vec<String>>>>,
}

impl CommandRunner for FakeGcloud {
    fn run(&self, command: GcloudCommand) -> Result<CommandOutput, CommandError> {
        let args = command.args.clone();
        self.argv.lock().unwrap().push(args.clone());
        let expression = args.iter().find_map(|arg| arg.strip_prefix("--filter="));

        let stdout = if args.iter().any(|a| a == "instances") {
            compute_list_json(expression)
        } else if args.iter().any(|a| a == "run") {
            assert!(
                expression.is_none(),
                "Cloud Run must never carry a --filter: {args:?}"
            );
            run_list_json()
        } else if args.iter().any(|a| a == "buckets") {
            assert!(
                expression.is_none(),
                "Cloud Storage must never carry a --filter: {args:?}"
            );
            bucket_list_json()
        } else {
            "[]".to_string()
        };
        Ok(CommandOutput {
            stdout,
            stderr: String::new(),
        })
    }
}

/// Whether one corpus row survives `expression`, evaluated over the *raw*
/// gcloud JSON values (URL-backed zone/machineType, raw status, IPs, labels).
fn row_matches(expression: &str, row: &Instance) -> bool {
    let Instance {
        name,
        zone,
        status,
        machine,
        internal_ip,
        external_ip,
        env,
        team,
    } = row;
    let raw = |field: &str| -> String {
        match field {
            "name" => (*name).to_string(),
            "zone" => {
                format!("https://www.googleapis.com/compute/v1/projects/acme-prod/zones/{zone}")
            }
            "status" => (*status).to_string(),
            "machineType" => format!(
                "https://www.googleapis.com/compute/v1/projects/acme-prod/zones/{zone}/machineTypes/{machine}"
            ),
            "networkInterfaces.networkIP" => (*internal_ip).to_string(),
            "networkInterfaces.accessConfigs.natIP" => (*external_ip).to_string(),
            "labels.env" => (*env).to_string(),
            "labels.team" => (*team).to_string(),
            other => panic!("stand-in gcloud was asked for unknown raw field {other:?}"),
        }
    };

    if expression.contains(" ~ ") {
        expression.split(" OR ").any(|clause| {
            let (field, pattern) = clause
                .split_once(" ~ ")
                .unwrap_or_else(|| panic!("unparsable regex clause {clause:?}"));
            let pattern = pattern.trim().trim_matches('"');
            regex_lite_matches(pattern, &raw(field.trim()))
        })
    } else {
        expression.split(" AND ").all(|clause| {
            let (field, value) = clause
                .split_once(" = ")
                .unwrap_or_else(|| panic!("unparsable label clause {clause:?}"));
            raw(field.trim()) == value.trim().trim_matches('"')
        })
    }
}

/// Case-insensitive partial match for the `(?i)<term>` patterns the pushdown
/// emits. The `is_server_expressible` gate admits only ASCII alphanumerics and
/// `.` `_` `-`, so `.` (any character) is the sole metacharacter that can ever
/// reach here — which is exactly what makes the pushed clause a superset.
fn regex_lite_matches(pattern: &str, value: &str) -> bool {
    let pattern = pattern.strip_prefix("(?i)").unwrap_or(pattern);
    assert!(
        pattern
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-')),
        "pushed pattern {pattern:?} escaped the is_server_expressible character gate"
    );
    let needle: Vec<char> = pattern.chars().collect();
    let hay: Vec<char> = value.to_lowercase().chars().collect();
    let needle: Vec<char> = needle.iter().map(|c| c.to_ascii_lowercase()).collect();
    if needle.len() > hay.len() {
        return false;
    }
    (0..=hay.len() - needle.len()).any(|start| {
        needle
            .iter()
            .enumerate()
            .all(|(offset, want)| *want == '.' || hay[start + offset] == *want)
    })
}

fn compute_list_json(expression: Option<&str>) -> String {
    let rows: Vec<String> = INSTANCES
        .iter()
        .enumerate()
        .filter(|(_, row)| expression.is_none_or(|e| row_matches(e, row)))
        .map(|(index, row)| {
            let Instance {
                name,
                zone,
                status,
                machine,
                internal_ip,
                external_ip,
                env,
                team,
            } = row;
            let access = if external_ip.is_empty() {
                String::new()
            } else {
                format!(r#","accessConfigs":[{{"natIP":"{external_ip}"}}]"#)
            };
            format!(
                r#"{{"id":"{id}","name":"{name}","zone":"https://www.googleapis.com/compute/v1/projects/acme-prod/zones/{zone}","status":"{status}","machineType":"https://www.googleapis.com/compute/v1/projects/acme-prod/zones/{zone}/machineTypes/{machine}","labels":{{"env":"{env}","team":"{team}"}},"networkInterfaces":[{{"networkIP":"{internal_ip}"{access}}}]}}"#,
                id = 1000 + index
            )
        })
        .collect();
    format!("[{}]", rows.join(","))
}

fn run_list_json() -> String {
    let rows: Vec<String> = RUN_SERVICES
        .iter()
        .map(|(name, region, url)| {
            format!(
                r#"{{"apiVersion":"serving.knative.dev/v1","kind":"Service","metadata":{{"name":"{name}","namespace":"482910233714","labels":{{"cloud.googleapis.com/location":"{region}"}}}},"spec":{{"template":{{"metadata":{{}},"spec":{{"containers":[{{"image":"europe-docker.pkg.dev/acme/apps/{name}:1.0.0"}}]}}}}}},"status":{{"url":"{url}","latestReadyRevisionName":"{name}-00007-xyz","conditions":[{{"type":"Ready","status":"True"}}]}}}}"#
            )
        })
        .collect();
    format!("[{}]", rows.join(","))
}

fn bucket_list_json() -> String {
    let rows: Vec<String> = BUCKETS
        .iter()
        .map(|(name, location, class)| {
            format!(
                r#"{{"name":"{name}","location":"{location}","storage_class":"{class}","default_storage_class":"{class}","location_type":"region","time_created":"2026-01-04T10:00:00Z","updated":"2026-07-01T10:00:00Z"}}"#
            )
        })
        .collect();
    format!("[{}]", rows.join(","))
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

fn json_string(value: &str) -> String {
    let escaped = value.replace('\\', "\\\\").replace('"', "\\\"");
    format!("\"{escaped}\"")
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

struct Tui {
    session: RuntimeSession,
    argv: Arc<Mutex<Vec<Vec<String>>>>,
}

impl Tui {
    fn new(config: Config) -> Tui {
        let mut app = AppState::new();
        app.root.configurations.set_rows(vec![GcloudConfiguration {
            name: GcloudConfigurationName::new("prod"),
            is_active: true,
            account: Some("sre@example.com".to_string()),
            project: Some("acme-prod".to_string()),
            region: Some("europe-west1".to_string()),
            zone: Some("europe-west1-b".to_string()),
        }]);
        app.root.workspace = Workspace::Resources;
        let argv = Arc::new(Mutex::new(Vec::new()));
        let session = RuntimeSession::new_with_config_and_runner_for_test(
            config,
            app,
            Box::new(FakeGcloud { argv: argv.clone() }),
        );
        Tui { session, argv }
    }

    /// Sends one real key event through the real input context + keymap.
    async fn press(&mut self, key: KeyEvent) {
        let context = input_context_for_test(&self.session, Rect::new(0, 0, WIDTH, HEIGHT));
        let action = action_for_key(key, &context)
            .unwrap_or_else(|| panic!("key {key:?} is unbound in this context"));
        self.session.dispatch_action(action).await;
    }

    async fn press_char(&mut self, ch: char) {
        let modifiers = if ch.is_ascii_uppercase() || "!".contains(ch) {
            KeyModifiers::SHIFT
        } else {
            KeyModifiers::NONE
        };
        self.press(KeyEvent::new(KeyCode::Char(ch), modifiers))
            .await;
    }

    /// Drains every effect result the last dispatch queued, so the next
    /// screenshot shows the server's answer rather than a spinner.
    async fn settle(&mut self) {
        let applied = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            self.session.process_next_effect_result(),
        )
        .await
        .expect("an effect result should arrive");
        assert!(applied, "effect-result channel closed");
        while let Ok(true) = tokio::time::timeout(
            std::time::Duration::from_millis(50),
            self.session.process_next_effect_result(),
        )
        .await
        {}
    }

    /// The full `/` interaction: open the input, clear the seeded draft, type
    /// the filter, Enter. Every keystroke is a real key event.
    async fn type_filter(&mut self, text: &str) {
        self.press_char('/').await;
        for _ in 0..64 {
            self.press(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE))
                .await;
        }
        for ch in text.chars() {
            self.press_char(ch).await;
        }
        self.press(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
            .await;
    }

    /// Drains whatever the last dispatch queued, without requiring anything.
    /// Applying a Cloud Storage bucket-list filter emits no effect at all —
    /// that filter is purely client-side — so this path must not insist on a
    /// reload the reducer never asked for.
    async fn settle_if_any(&mut self) {
        while let Ok(true) = tokio::time::timeout(
            std::time::Duration::from_millis(100),
            self.session.process_next_effect_result(),
        )
        .await
        {}
    }

    /// Applies a filter and settles whatever reload it triggered. Filters that
    /// push nothing down still re-issue the list command, so every mode ends
    /// with the table showing the server's current answer.
    async fn apply_filter(&mut self, text: &str) {
        self.type_filter(text).await;
        self.settle().await;
    }

    /// [`Self::apply_filter`] for a service whose filter triggers no reload.
    async fn apply_client_only_filter(&mut self, text: &str) {
        self.type_filter(text).await;
        self.settle_if_any().await;
    }

    /// Opens the filter input without applying, for the draft-state frames.
    async fn open_filter_input(&mut self) {
        self.press_char('/').await;
    }

    async fn select_service(&mut self, kind: ResourceServiceKind) {
        self.session
            .dispatch_action(Action::Resources(ResourcesAction::SelectService(kind)))
            .await;
        self.settle().await;
    }

    fn last_argv(&self) -> Vec<String> {
        self.argv
            .lock()
            .unwrap()
            .last()
            .cloned()
            .unwrap_or_default()
    }

    fn last_filter_expression(&self) -> Option<String> {
        self.last_argv()
            .iter()
            .find_map(|arg| arg.strip_prefix("--filter=").map(str::to_string))
    }

    fn capture(&self) -> Frame {
        capture(&self.session)
    }

    fn visible_instances(&self) -> Vec<String> {
        self.session
            .app()
            .root
            .resources
            .visible_instances()
            .into_iter()
            .map(|instance| instance.name.clone())
            .collect()
    }
}

/// The tools line under the table — where the filter text, the mode chip, the
/// inline error and the row counts all live.
fn tools_line(frame: &Frame) -> String {
    frame
        .lines
        .iter()
        .find(|line| line.contains("Filter:"))
        .cloned()
        .unwrap_or_else(|| panic!("no tools line in frame:\n{}", frame.lines.join("\n")))
}

fn names(values: &[&str]) -> Vec<String> {
    values.iter().map(|v| (*v).to_string()).collect()
}

/// Whether `text` appears anywhere in the rendered frame.
fn frame_contains(frame: &Frame, text: &str) -> bool {
    frame.lines.iter().any(|line| line.contains(text))
}

/// Asserts every corpus instance is painted in the frame.
///
/// This is what "malformed input shows every row" means to the operator, and
/// it is stronger evidence than the `Rows: n/n` counter — the names are on
/// screen. The counter is used elsewhere, but a long inline error pushes it
/// past the right edge of the table pane, and truncating a tools line that
/// does not fit is pre-existing behaviour, not something this change alters.
fn assert_every_instance_rendered(frame: &Frame) {
    for row in INSTANCES {
        assert!(
            frame_contains(frame, row.name),
            "row {} must still be on screen:\n{}",
            row.name,
            frame.lines.join("\n")
        );
    }
}

// ---------------------------------------------------------------------------
// The walkthrough
// ---------------------------------------------------------------------------

#[tokio::test]
async fn multi_mode_filter_walkthrough_evidence() {
    // Scratch only — the collected evidence goes to `evidence_dir()` below.
    let home_dir = tempfile::tempdir().expect("home dir");
    let home = home_dir.path();
    let config = Config {
        log_file: home.join("nuvem.log"),
        ..Config::default()
    };
    let mut tui = Tui::new(config);

    append_transcript(
        "Every `gcloud` list command the resource tables issued during the\n\
         walkthrough, recorded at the CommandRunner boundary (the exact argv the process\n\
         would exec). `--filter=` present means the narrowing ran SERVER-side; absent\n\
         means the whole list was downloaded and narrowed CLIENT-side.\n",
    );

    // ---- 1. Baseline: no filter -------------------------------------------
    tui.press_char('r').await;
    tui.settle().await;
    let frame = tui.capture();
    let line = tools_line(&frame);
    assert!(line.contains("Filter: none"), "tools line: {line}");
    assert!(
        !line.contains('['),
        "an empty filter must render no mode chip: {line}"
    );
    assert!(line.contains("Rows: 7/7"), "tools line: {line}");
    assert_eq!(tui.visible_instances().len(), 7);
    assert_eq!(tui.last_filter_expression(), None);
    append_transcript(&format!(
        "\n[1] baseline — no filter applied (7/7 rows)\n    {}",
        shell_form(&tui.last_argv())
    ));
    write_frame(
        "01-baseline-no-filter",
        "Step 1 — the Compute Engine table with NO filter. The tools line reads \
'Filter: none' with no mode chip at all (an idle empty filter makes no claim about a \
mode), 7 of 7 rows are shown, and the gcloud argv carries no --filter: default \
behaviour is unchanged.",
        &frame,
    );

    // ---- 2. The grammar announces itself before the first keystroke --------
    tui.open_filter_input().await;
    let frame = tui.capture();
    let line = tools_line(&frame);
    assert!(
        line.contains("Filter: _  [regex]"),
        "an open, empty input must still name the default mode: {line}"
    );
    write_frame(
        "02-input-open-default-mode",
        "Step 2 — pressing `/` opens the filter input. Even with nothing typed \
the chip already reads [regex], naming the default mode before the first keystroke. \
This is what makes the four-mode grammar discoverable: the operator learns there IS a \
mode to change.",
        &frame,
    );
    tui.press(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
        .await;

    // ---- 3. The chip tracks the draft live, mode by mode --------------------
    for (draft, expected_chip) in [
        ("-f api", "[fuzzy]"),
        ("-l env=prod", "[label]"),
        ("!west", "[inverse]"),
    ] {
        tui.open_filter_input().await;
        for _ in 0..64 {
            tui.press(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE))
                .await;
        }
        for ch in draft.chars() {
            tui.press_char(ch).await;
        }
        let frame = tui.capture();
        let line = tools_line(&frame);
        assert!(
            line.contains(&format!("Filter: {draft}_  {expected_chip}")),
            "draft {draft:?} must show {expected_chip}: {line}"
        );
        tui.press(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
            .await;
    }
    tui.open_filter_input().await;
    for _ in 0..64 {
        tui.press(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE))
            .await;
    }
    for ch in "-l env=prod".chars() {
        tui.press_char(ch).await;
    }
    let frame = tui.capture();
    write_frame(
        "03-draft-mode-chip-live",
        "Step 3 — the chip tracks the DRAFT as it is typed, not just the applied \
filter. Half-way through typing `-l env=prod` the chip already reads [label], so the \
operator sees which of the four modes the prefix selected before committing with Enter. \
The same live tracking was asserted for `-f api` -> [fuzzy] and `!west` -> [inverse].",
        &frame,
    );
    tui.press(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
        .await;

    // ---- 4. Regex mode, pushed down ---------------------------------------
    // `w.b` is the proof that the default really is a regex: as a plain
    // substring it matches nothing at all, as a pattern it matches every
    // "web" row. It is also `is_server_expressible`, so it is pushed.
    tui.apply_filter("w.b").await;
    let frame = tui.capture();
    let line = tools_line(&frame);
    assert!(line.contains("Filter: w.b  [regex]"), "tools line: {line}");
    assert!(line.contains("Rows: 3/3"), "tools line: {line}");
    assert_eq!(
        tui.visible_instances(),
        names(&["api-web-01", "api-web-02", "web-proxy-01"])
    );
    let expression = tui
        .last_filter_expression()
        .expect("an expressible regex term must be pushed to the server");
    assert!(
        expression.starts_with(r#"name ~ "(?i)w.b""#),
        "expression: {expression}"
    );
    append_transcript(&format!(
        "\n[4] regex mode (default, no prefix) `w.b` — PUSHED DOWN, server returned 3 rows\n    {}",
        shell_form(&tui.last_argv())
    ));
    write_frame(
        "04-regex-mode-pushed-down",
        "Step 4 — REGEX MODE (the default: no prefix). The filter `w.b` is a \
pattern, not a substring: no row's text literally contains \"w.b\", yet all three \
\"web\" rows match because `.` is any character. The old plain-substring filter would \
have shown 0 rows here. The term passes the character gate, so it was pushed to the \
server as `--filter=name ~ \"(?i)w.b\" OR ...` — the other four rows were never \
downloaded.",
        &frame,
    );

    // ---- 5. Regex mode, not expressible -> client-side residual ------------
    tui.apply_filter("cache|proxy").await;
    let frame = tui.capture();
    let line = tools_line(&frame);
    assert!(
        line.contains("Filter: cache|proxy  [regex]"),
        "tools line: {line}"
    );
    assert!(line.contains("Rows: 2/7"), "tools line: {line}");
    assert_eq!(
        tui.visible_instances(),
        names(&["cache-redis-01", "web-proxy-01"])
    );
    assert_eq!(
        tui.last_filter_expression(),
        None,
        "alternation is not server-expressible and must not be pushed"
    );
    append_transcript(&format!(
        "\n[5] regex mode `cache|proxy` — NOT expressible, no --filter; all 7 downloaded,\n    \
         client narrowed to 2 (note `Rows: 2/7`)\n    {}",
        shell_form(&tui.last_argv())
    ));
    write_frame(
        "05-regex-client-side-residual",
        "Step 5 — the same regex mode, but `cache|proxy` uses alternation, which \
the server-expressible character gate rejects. No --filter is emitted, all 7 rows are \
downloaded, and the CLIENT narrows to 2. The counts say so out loud: 'Rows: 2/7' \
(2 visible of 7 loaded) versus step 4's 'Rows: 3/3' (the server had already narrowed). \
Same grammar, correct narrowing either way.",
        &frame,
    );

    // ---- 6. The pattern-aware never-drop-a-row gate ------------------------
    // `r.n` matches the derived `Running` status cell and the raw `REPAIRING`
    // string, neither of which a literal-substring gate would have caught.
    tui.apply_filter("r.n").await;
    let frame = tui.capture();
    let line = tools_line(&frame);
    assert!(line.contains("Filter: r.n  [regex]"), "tools line: {line}");
    assert!(line.contains("Rows: 5/7"), "tools line: {line}");
    assert_eq!(
        tui.visible_instances(),
        names(&[
            "api-web-01",
            "api-web-02",
            "db-primary-01",
            "db-replica-01",
            "web-proxy-01",
        ])
    );
    assert_eq!(
        tui.last_filter_expression(),
        None,
        "a pattern that matches an opaque status label must never be pushed"
    );
    append_transcript(&format!(
        "\n[6] regex mode `r.n` — GATED: the pattern matches the derived `Running` label,\n    \
         so pushing it down would have dropped those rows. No --filter; 5 of 7 kept.\n    {}",
        shell_form(&tui.last_argv())
    ));
    write_frame(
        "06-opaque-label-gate-pattern-aware",
        "Step 6 — the never-drop-a-row gate, in its pattern-aware form. `r.n` is \
NOT a substring of \"running\", so the old literal gate would have waved it through and \
pushed it to the server, which searches raw JSON where the derived 'Running' cell does \
not exist — and the four Running rows would have silently vanished. The gate now asks \
the client's own compiled regex, sees the match, and refuses to push. Note also \
db-replica-01, whose STATUS cell paints the Debug shell `Unknown(\"REPAIRING\")`: that \
punctuation is in no raw gcloud field and in no opaque label, so it is deliberately NOT \
what the filter searches — the row survives on the 'Unknown' display label and the raw \
'REPAIRING' string, both of which the pushdown premise covers.",
        &frame,
    );

    // ---- 7. Fuzzy mode -----------------------------------------------------
    tui.apply_filter("-f dbp").await;
    let frame = tui.capture();
    let line = tools_line(&frame);
    assert!(
        line.contains("Filter: -f dbp  [fuzzy]"),
        "tools line: {line}"
    );
    assert!(line.contains("Rows: 2/7"), "tools line: {line}");
    assert_eq!(
        tui.visible_instances(),
        names(&["db-primary-01", "db-replica-01"])
    );
    assert_eq!(
        tui.last_filter_expression(),
        None,
        "fuzzy has no provable superset expression and is never pushed"
    );
    append_transcript(&format!(
        "\n[7] fuzzy mode `-f dbp` — client-only by design, no --filter (2 of 7 kept)\n    {}",
        shell_form(&tui.last_argv())
    ));
    write_frame(
        "07-fuzzy-mode",
        "Step 7 — FUZZY MODE (`-f` prefix). `dbp` matches as a SUBSEQUENCE: d, then \
b, then p, in order but not adjacent — so it finds `db-primary-01` and `db-replica-01` \
(the p of 'replica'). The chip reads [fuzzy]. Subsequence matching has no provably \
superset gcloud expression, so it is deliberately client-only: no --filter is emitted \
and 'Rows: 2/7' shows the full list was downloaded and narrowed here.",
        &frame,
    );

    // ---- 8. Inverse mode ---------------------------------------------------
    tui.apply_filter("!west").await;
    let frame = tui.capture();
    let line = tools_line(&frame);
    assert!(
        line.contains("Filter: !west  [inverse]"),
        "tools line: {line}"
    );
    assert!(line.contains("Rows: 2/7"), "tools line: {line}");
    assert_eq!(
        tui.visible_instances(),
        names(&["web-proxy-01", "worker-batch-01"])
    );
    assert_eq!(tui.last_filter_expression(), None);
    append_transcript(&format!(
        "\n[8] inverse mode `!west` — client-only by design, no --filter (2 of 7 kept)\n    {}",
        shell_form(&tui.last_argv())
    ));
    write_frame(
        "08-inverse-mode",
        "Step 8 — INVERSE MODE (`!` prefix). `!west` keeps only the rows where NO \
searchable field matches `west`, leaving the two us-central1 instances and hiding all \
five europe-west ones. The chip reads [inverse]. Negation over derived display fields \
has no superset expression either, so it too is client-only.",
        &frame,
    );

    // ---- 9. Label mode: server-authoritative -------------------------------
    tui.apply_filter("-l env=prod").await;
    let frame = tui.capture();
    let line = tools_line(&frame);
    assert!(
        line.contains("Filter: -l env=prod  [label]"),
        "tools line: {line}"
    );
    assert!(
        !line.contains('⚠'),
        "a supported, expressible label filter reports no error: {line}"
    );
    assert!(line.contains("Rows: 5/5"), "tools line: {line}");
    assert_eq!(
        tui.visible_instances(),
        names(&[
            "api-web-01",
            "api-web-02",
            "db-primary-01",
            "db-replica-01",
            "web-proxy-01",
        ])
    );
    assert_eq!(
        tui.last_filter_expression().as_deref(),
        Some(r#"labels.env = "prod""#)
    );
    append_transcript(&format!(
        "\n[9] label mode `-l env=prod` — PUSHED DOWN; list rows carry no labels, so the\n    \
         server is the only thing that can narrow this (5/5: everything downloaded is shown)\n    {}",
        shell_form(&tui.last_argv())
    ));
    write_frame(
        "09-label-mode-server-authoritative",
        "Step 9 — LABEL MODE (`-l` prefix), the one mode that only works because \
of the pushdown. List rows carry no labels at all, so there is nothing to match \
client-side; the narrowing is entirely the emitted `--filter=labels.env = \"prod\"`. \
'Rows: 5/5' is the tell: the server returned exactly the five prod-labelled instances \
and the client kept every one of them. cache-redis-01 and worker-batch-01 (env=staging) \
were never downloaded.",
        &frame,
    );

    // ---- 10. Reloads carry the applied filter ------------------------------
    // The regression this fixed: a poll/refresh that passed `filter: None`
    // replaced the narrowed rows with the whole project while the chip still
    // read [label].
    tui.press_char('r').await;
    tui.settle().await;
    let frame = tui.capture();
    let line = tools_line(&frame);
    assert_eq!(
        tui.last_filter_expression().as_deref(),
        Some(r#"labels.env = "prod""#),
        "a reload must re-push the applied filter, not silently disarm it"
    );
    assert!(
        line.contains("Rows: 5/5"),
        "tools line after reload: {line}"
    );
    assert_eq!(tui.visible_instances().len(), 5);
    append_transcript(&format!(
        "\n[10] refresh with `-l env=prod` still applied — the reload RE-PUSHES the filter.\n     \
          Passing filter: None here would have widened the table to all 7 rows while the\n     \
          chip still read [label].\n    {}",
        shell_form(&tui.last_argv())
    ));
    write_frame(
        "10-reload-carries-applied-filter",
        "Step 10 — pressing `r` to refresh while `-l env=prod` is applied. The \
re-issued command STILL carries `--filter=labels.env = \"prod\"` and the table still \
shows the same five rows. This is the data-integrity fix: a reload that dropped the \
filter would have replaced these five with all seven instances in the project while the \
chip still asserted [label] — and an operator batch-acting on 'the filtered rows' would \
have hit machines outside the filter.",
        &frame,
    );

    // ---- 10b. Switching services reloads with the RIGHT service's filter ---
    // Cloud Run gets a filter of its own, so switching back to Compute has two
    // ways to be wrong: dropping Compute's filter, or pushing Cloud Run's.
    tui.select_service(ResourceServiceKind::CloudRunServices)
        .await;
    tui.apply_filter("check").await;
    assert_eq!(
        tui.session
            .app()
            .root
            .resources
            .filter_applied_for(ResourceServiceKind::CloudRunServices),
        "check"
    );
    tui.select_service(ResourceServiceKind::ComputeInstances)
        .await;
    let frame = tui.capture();
    let line = tools_line(&frame);
    assert_eq!(
        tui.last_filter_expression().as_deref(),
        Some(r#"labels.env = "prod""#),
        "switching back must re-push COMPUTE's own filter — not None, not Cloud Run's `check`"
    );
    assert!(
        line.contains("Filter: -l env=prod  [label]"),
        "tools line: {line}"
    );
    assert!(line.contains("Rows: 5/5"), "tools line: {line}");
    assert_eq!(tui.visible_instances().len(), 5);
    append_transcript(&format!(
        "\n[10b] switched Compute -> Cloud Run (filter `check`) -> back to Compute.\n      \
          The Compute reload carries COMPUTE's own `-l env=prod`, not Cloud Run's term\n      \
          and not an empty filter.\n    {}",
        shell_form(&tui.last_argv())
    ));
    write_frame(
        "10b-service-switch-carries-own-filter",
        "Step 10b — the same rule across a service switch. Cloud Run was given its \
own filter (`check`), then the operator switched back to Compute. The reload of Compute \
carries COMPUTE's applied `-l env=prod` — pushing Cloud Run's term would narrow the wrong \
list, and pushing nothing would widen this table back to all 7 instances while the chip \
still read [label]. Each service's filter is its own, and travels with its own reload.",
        &frame,
    );

    // ---- 11. Malformed regex: every row, plus an inline error --------------
    tui.apply_filter("api(").await;
    let frame = tui.capture();
    let line = tools_line(&frame);
    assert!(line.contains("Filter: api(  [regex]"), "tools line: {line}");
    assert!(
        line.contains('⚠'),
        "a malformed regex must report inline: {line}"
    );
    assert!(
        line.contains("⚠ invalid regex: unclosed group"),
        "the error must name the cause on one line: {line}"
    );
    // Every row is still on screen — a bad pattern narrows nothing.
    assert_eq!(tui.visible_instances().len(), 7);
    assert_every_instance_rendered(&frame);
    append_transcript(&format!(
        "\n[11] malformed regex `api(` — non-fatal: inline error, all 7 rows still shown\n     \
          (no panic, no silently-empty table)\n    {}",
        shell_form(&tui.last_argv())
    ));
    write_frame(
        "11-malformed-regex-inline-error",
        "Step 11 — malformed input is NON-FATAL. `api(` is an unclosed regex group. \
The app does not panic and does not blank the table; it shows every row ('Rows: 7/7') \
and says why inline: '⚠ unclosed group'. An empty table behind a bad pattern would be \
indistinguishable from a correct empty result, so the design deliberately shows too much \
and explains itself.",
        &frame,
    );

    // ---- 12. Malformed label selector --------------------------------------
    tui.apply_filter("-l env").await;
    let frame = tui.capture();
    let line = tools_line(&frame);
    assert!(
        line.contains("Filter: -l env  [label]"),
        "tools line: {line}"
    );
    assert!(
        line.contains("⚠ malformed label selector: expected key=value"),
        "tools line: {line}"
    );
    assert_eq!(tui.visible_instances().len(), 7);
    assert_every_instance_rendered(&frame);
    write_frame(
        "12-malformed-label-selector",
        "Step 12 — the same non-fatal treatment for a malformed LABEL selector. \
`-l env` has no `=value`, so the chip still reads [label] but the inline error names the \
exact problem — '⚠ malformed label selector: expected key=value' — and all 7 rows stay \
visible.",
        &frame,
    );

    // ---- 13. Editing the draft retires a stale error ------------------------
    tui.open_filter_input().await;
    let frame = tui.capture();
    let line = tools_line(&frame);
    assert!(
        line.contains("⚠ malformed label selector"),
        "reopening `/` without editing still explains the failure: {line}"
    );
    tui.press(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE))
        .await;
    let frame = tui.capture();
    let line = tools_line(&frame);
    assert!(
        !line.contains('⚠'),
        "the first keystroke of a fix must drop a message that no longer describes the text: {line}"
    );
    assert!(
        line.contains("Filter: -l en_  [label]"),
        "tools line: {line}"
    );
    write_frame(
        "13-error-retires-on-edit",
        "Step 13 — the inline error is scoped to the text it describes. Reopening `/` \
on the broken `-l env` still shows the warning (top frame state), but the first backspace \
of a fix drops it: the message described the APPLIED text, and the draft is no longer that \
text. The [label] chip stays, because it tracks the draft.",
        &frame,
    );
    tui.press(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
        .await;

    // ---- 14. A service with no label pushdown says so ----------------------
    tui.select_service(ResourceServiceKind::CloudRunServices)
        .await;
    tui.apply_filter("-l env=prod").await;
    let frame = tui.capture();
    let line = tools_line(&frame);
    assert!(
        line.contains("⚠ label filtering is not supported for Cloud Run"),
        "tools line: {line}"
    );
    for (name, ..) in RUN_SERVICES {
        assert!(
            frame_contains(&frame, name),
            "the table stays honest about showing everything: {name} must be on screen"
        );
    }
    assert_eq!(
        tui.last_filter_expression(),
        None,
        "Cloud Run has no label field to push"
    );
    append_transcript(&format!(
        "\n[14] `-l env=prod` on Cloud Run — no label field exists to push, so the user is\n     \
          told instead of being shown every row as if they all matched\n    {}",
        shell_form(&tui.last_argv())
    ));
    write_frame(
        "14-cloud-run-label-unsupported",
        "Step 14 — Cloud Run list output exposes no user labels, so there is nothing \
to push down AND nothing to match client-side. Rather than silently returning every \
service as though they all carried env=prod, the table says '⚠ label filtering is not \
supported for Cloud Run'. The failure is visible, not silent.",
        &frame,
    );

    // ---- 15. Cloud Storage is deliberately excluded ------------------------
    tui.select_service(ResourceServiceKind::CloudStorage).await;
    tui.apply_client_only_filter("logs").await;
    let frame = tui.capture();
    let line = tools_line(&frame);
    assert!(line.contains("Filter: logs"), "tools line: {line}");
    assert!(
        !line.contains('['),
        "Cloud Storage must render NO mode chip: {line}"
    );
    assert!(
        !line.contains('⚠'),
        "Cloud Storage reports no grammar errors: {line}"
    );
    assert!(line.contains("Rows: 2/3"), "tools line: {line}");
    write_frame(
        "15-storage-keeps-plain-substring",
        "Step 15 — Cloud Storage is DELIBERATELY excluded from the grammar. It shares \
one filter string with the object browser, which pushes that text down as a literal \
server-side object prefix, so a grammar-aware client predicate would silently drop rows. \
Its filter stays a plain case-insensitive substring ('logs' matches the two *-logs \
buckets) and it renders NO mode chip — the UI never advertises a grammar this service \
does not have.",
        &frame,
    );

    // A grammar prefix here is just literal text, matching nothing, and still
    // no chip and no error.
    tui.apply_client_only_filter("-l env=prod").await;
    let frame = tui.capture();
    let line = tools_line(&frame);
    assert!(!line.contains('['), "still no chip: {line}");
    assert!(!line.contains('⚠'), "still no error: {line}");
    assert!(
        line.contains("Rows: 0/3"),
        "the text is a literal substring here: {line}"
    );
    write_frame(
        "16-storage-prefix-is-literal-text",
        "Step 16 — typing a grammar prefix into Cloud Storage confirms the exclusion \
is total, not cosmetic: `-l env=prod` is treated as literal substring text (matching no \
bucket, 'Rows: 0/3'), with no mode chip and no inline error. The service's behaviour is \
byte-identical to before this change.",
        &frame,
    );
}
