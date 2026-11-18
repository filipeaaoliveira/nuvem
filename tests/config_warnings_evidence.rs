//! Evidence: records what a user with a broken `[columns.*]` config
//! actually ends up looking at — the config body they wrote, the message each
//! service composes from its *final* outcome, the status line that gets
//! surfaced, and the `Config problems` block the help overlay renders.
//!
//! The two deferred shapes are the point: before the change a
//! per-iteration warning could outlive its truth, and only one of the collected
//! warnings was ever shown. Each case here goes through the real
//! `Config::resolve` → `surface_config_problems` → `help_model_with` →
//! `help_lines_wrapped` path, so the recorded text is the rendered text.
//!
//! Nothing here observes time: the evidence is entirely a function of the
//! config body. Writes to the evidence directory when
//! NUVEM_EVIDENCE_DIR is set.

use std::io::Write;

use clap::Parser;
use nuvem::app::AppState;
use nuvem::app::projection::binding::help_model_with;
use nuvem::services::runtime::surface_config_problems;
use nuvem::ui::shared::help::help_lines_wrapped;
use nuvem::{Cli, Config};

/// The overlay's inner width in an 80-column terminal, so the recorded block
/// wraps the way it does on a normal screen rather than at some width chosen
/// to make the evidence look tidy.
const HELP_WIDTH: u16 = 68;

/// One case's worth of evidence, all of it derived from `body`.
struct Case {
    title: &'static str,
    body: &'static str,
    messages: Vec<String>,
    status: String,
    help_section: Vec<String>,
}

fn write_config(body: &str) -> tempfile::NamedTempFile {
    let mut file = tempfile::NamedTempFile::new().expect("create temp config");
    file.write_all(body.as_bytes()).expect("write temp config");
    file.flush().expect("flush temp config");
    file
}

/// The `Config problems` block of a rendered help model: the title line and
/// every entry line, up to the blank line that closes the block. Empty when
/// the model records no problems, which is itself the interesting outcome for
/// a config that resolved cleanly.
fn help_section(lines: &[ratatui::text::Line<'static>]) -> Vec<String> {
    let text: Vec<String> = lines
        .iter()
        .map(|line| {
            line.spans
                .iter()
                .map(|span| span.content.as_ref())
                .collect::<String>()
        })
        .collect();
    if text.first().map(String::as_str) != Some("Config problems") {
        return Vec::new();
    }
    let end = text.iter().position(String::is_empty).unwrap_or(text.len());
    text[..end].to_vec()
}

/// Drives the real startup path for `body` and collects everything a reviewer
/// needs to see.
fn evidence(title: &'static str, body: &'static str) -> Case {
    let file = write_config(body);
    let cli = Cli::parse_from([
        "nuvem",
        "--plugins",
        "/nonexistent/nuvem-plugins-test.toml",
        "--config",
        file.path().to_str().unwrap(),
    ]);
    let outcome = Config::resolve(&cli);
    assert!(
        outcome.load_error.is_none(),
        "{title}: the fixture must load, so the evidence is about field-level \
         problems and not a rejected file: {:?}",
        outcome.load_error
    );

    let messages: Vec<String> = outcome
        .problems
        .messages()
        .iter()
        .map(|m| (*m).to_string())
        .collect();

    let mut app = AppState::new();
    let hotkeys = outcome.config.hotkeys.clone();
    surface_config_problems(
        &mut app,
        &outcome.problems.clone().into_messages(),
        None,
        &hotkeys,
    );
    let status = app
        .root
        .system
        .status_message
        .clone()
        .unwrap_or_else(|| "<no status message>".to_string());

    let model = help_model_with(&app.root, &hotkeys);
    let help_section = help_section(&help_lines_wrapped(&model, HELP_WIDTH));

    Case {
        title,
        body,
        messages,
        status,
        help_section,
    }
}

fn render(case: &Case) -> String {
    format!(
        "=== {title} ===\n\n\
         --- config body ---\n{body}\n\
         --- composed messages ({count}) ---\n{messages}\n\n\
         --- surfaced status line ---\n{status}\n\n\
         --- help overlay, `Config problems` block (wrapped to {width} columns) ---\n{help}\n",
        title = case.title,
        body = case.body,
        count = case.messages.len(),
        messages = case
            .messages
            .iter()
            .map(|m| format!("{m:?}"))
            .collect::<Vec<_>>()
            .join("\n"),
        status = case.status,
        width = HELP_WIDTH,
        help = case.help_section.join("\n"),
    )
}

#[test]
fn records_what_a_broken_columns_config_surfaces() {
    // Shape (a) from the deferral: two valid keys for one service and
    // an invalid third. `gce` sorts last, so it decides — and it is rejected,
    // which takes the two valid blocks down with it. The single composed
    // message has to say so; naming `compute_instances` as deciding (what the
    // per-iteration warning used to say) would describe a table that does not
    // exist, because the service ends on its built-in columns.
    let shape_a = evidence(
        "Shape (a) — two valid keys displaced by an invalid winner",
        "version = 1\n\n\
         [columns.compute]\ncolumns = [\"name\", \"status\"]\n\n\
         [columns.compute_instances]\ncolumns = [\"name\", \"zone\"]\n\n\
         [columns.gce]\ncolumns = [\"name\", \"zzz\"]\n",
    );

    assert_eq!(
        shape_a.messages,
        [
            "columns.gce: unknown column \"zzz\"; the same service is also configured as \
          columns.compute, columns.compute_instances, which are discarded with it \
          (blocks are read in alphabetical key order, not file order); \
          ignoring this service's column settings"
        ],
        "one message per service, composed from the winner's outcome"
    );
    assert_eq!(
        shape_a.status,
        format!("config warning: {}", shape_a.messages[0]),
        "a single problem keeps the earlier wording"
    );
    assert_eq!(shape_a.help_section[0], "Config problems");
    assert!(
        shape_a.help_section.len() > 2,
        "the message is long enough to wrap over several entry lines: {:?}",
        shape_a.help_section
    );

    // Shape (b): the invalid key is the one that loses. `gce` sorts after
    // `compute` and validates, so the table *is* configured — the message says
    // which key won, and must not claim the service fell back.
    let shape_b = evidence(
        "Shape (b) — an invalid key displaced by a valid winner",
        "version = 1\n\n\
         [columns.compute]\ncolumns = [\"name\", \"zzz\"]\n\n\
         [columns.gce]\ncolumns = [\"name\", \"zone\"]\n",
    );

    assert_eq!(
        shape_b.messages,
        [
            "columns.gce: the same service is also configured as columns.compute; \
          columns.gce decides (blocks are read in alphabetical key order, not file order); \
          columns.compute is itself invalid (unknown column \"zzz\")"
        ],
    );
    assert!(
        !shape_b.messages[0].contains("ignoring this service's column settings"),
        "the winning block configures the table, so nothing was ignored: {:?}",
        shape_b.messages[0]
    );
    assert_eq!(
        shape_b.status,
        format!("config warning: {}", shape_b.messages[0])
    );

    // Two services, each broken on its own: the case this exists for. Both
    // messages are collected, the status line names the count and points at
    // the help key, and the overlay lists both.
    let two_services = evidence(
        "Two services, both broken — the count plus the full list",
        "version = 1\n\n\
         [columns.compute]\ncolumns = [\"name\", \"zzz\"]\n\n\
         [columns.storage]\ncolumns = [\"location\"]\n",
    );

    assert_eq!(
        two_services.messages,
        [
            "columns.compute: unknown column \"zzz\"; ignoring this service's column settings",
            "columns.storage: column list must include \"name\"; \
             ignoring this service's column settings",
        ],
    );
    assert_eq!(
        two_services.status, "config: 2 problems (press ? for details)",
        "neither problem is dropped, and the status says where the list is"
    );
    let entries = two_services.help_section[1..].join("\n");
    assert!(entries.contains("columns.compute"), "{entries}");
    assert!(entries.contains("columns.storage"), "{entries}");

    let report = format!(
        "nuvem — every config problem is surfaced, and each service's\n\
         message is composed from its final outcome\n\n\
         {a}\n{b}\n{c}",
        a = render(&shape_a),
        b = render(&shape_b),
        c = render(&two_services),
    );

    println!("\n{report}");

    if let Ok(dir) = std::env::var("NUVEM_EVIDENCE_DIR") {
        let path = std::path::Path::new(&dir).join("config_warnings_surfaced.txt");
        std::fs::write(&path, report).expect("write evidence file");
    }
}

// ---------------------------------------------------------------------------
// Rendered evidence: the frames a user with a broken config actually looks at.
// ---------------------------------------------------------------------------

/// A screenshot of the whole terminal, plus the per-row style runs a colored
/// rendering of it needs.
struct Screenshot {
    lines: Vec<String>,
    runs: Vec<Vec<(String, String, bool)>>,
}

const SCREEN_WIDTH: u16 = 100;
const SCREEN_HEIGHT: u16 = 26;

fn capture(session: &nuvem::services::runtime::session::RuntimeSession) -> Screenshot {
    let backend = ratatui::backend::TestBackend::new(SCREEN_WIDTH, SCREEN_HEIGHT);
    let mut terminal = ratatui::Terminal::new(backend).expect("terminal");
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
            let fg = format!("{:?}", cell.fg);
            let bold = cell.modifier.contains(ratatui::style::Modifier::BOLD);
            match row.last_mut() {
                Some(last) if last.1 == fg && last.2 == bold => last.0.push_str(&symbol),
                _ => row.push((symbol, fg, bold)),
            }
        }
        lines.push(line.trim_end().to_string());
        runs.push(row);
    }
    Screenshot { lines, runs }
}

fn json_string(value: &str) -> String {
    let escaped = value.replace('\\', "\\\\").replace('"', "\\\"");
    format!("\"{escaped}\"")
}

/// Writes an ASCII screenshot and a JSON cell dump (which the PNG renderer
/// consumes) for one frame of the walkthrough.
fn write_screenshot(name: &str, caption: &str, shot: &Screenshot) {
    let Ok(dir) = std::env::var("NUVEM_EVIDENCE_DIR") else {
        return;
    };
    let dir = std::path::PathBuf::from(dir);
    std::fs::create_dir_all(&dir).expect("evidence dir");

    let width = usize::from(SCREEN_WIDTH);
    let mut body = format!("{caption}\n\n+{}+\n", "-".repeat(width));
    for line in &shot.lines {
        let pad = width.saturating_sub(line.chars().count());
        body.push_str(&format!("|{line}{}|\n", " ".repeat(pad)));
    }
    body.push_str(&format!("+{}+\n", "-".repeat(width)));
    std::fs::write(dir.join(format!("{name}.txt")), body).expect("write ascii frame");

    let mut json = String::from("{\n");
    json.push_str(&format!("  \"caption\": {},\n", json_string(caption)));
    json.push_str("  \"rows\": [\n");
    for (index, row) in shot.runs.iter().enumerate() {
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
        let comma = if index + 1 == shot.runs.len() {
            ""
        } else {
            ","
        };
        json.push_str(&format!("    [{}]{comma}\n", cells.join(",")));
    }
    json.push_str("  ]\n}\n");
    std::fs::write(dir.join(format!("{name}.json")), json).expect("write cell dump");
}

/// Sends one real key event through the real input context and keymap, the way
/// the event loop does.
async fn press(session: &mut nuvem::services::runtime::session::RuntimeSession, ch: char) {
    let key = crossterm::event::KeyEvent::new(
        crossterm::event::KeyCode::Char(ch),
        crossterm::event::KeyModifiers::NONE,
    );
    let context = nuvem::services::runtime::input_context_for_test(
        session,
        ratatui::layout::Rect::new(0, 0, SCREEN_WIDTH, SCREEN_HEIGHT),
    );
    let action = nuvem::input::keymap::action_for_key(key, &context)
        .unwrap_or_else(|| panic!("key {ch:?} is unbound in this context"));
    session.dispatch_action(action).await;
}

fn status_line(shot: &Screenshot) -> String {
    shot.lines
        .iter()
        .rev()
        .find(|line| line.contains("config"))
        .cloned()
        .unwrap_or_default()
}

/// The end-user walkthrough: start on a config broken in two places, look at
/// the status line, press the help key, read the whole list, fix the file,
/// reload, and watch the block go away.
#[tokio::test]
async fn a_user_with_two_broken_services_sees_both_and_can_clear_them() {
    let broken = "version = 1\n\n\
         [columns.compute]\ncolumns = [\"name\", \"zzz\"]\n\n\
         [columns.storage]\ncolumns = [\"location\"]\n";
    let fixed = "version = 1\n\n\
         [columns.compute]\ncolumns = [\"name\", \"zone\", \"status\"]\n";

    let file = write_config(broken);
    let path = file.path().to_path_buf();
    let cli = Cli::parse_from([
        "nuvem",
        "--plugins",
        "/nonexistent/nuvem-plugins-test.toml",
        "--config",
        path.to_str().unwrap(),
    ]);
    let outcome = Config::resolve(&cli);
    let hotkeys = outcome.config.hotkeys.clone();

    let mut app = AppState::new();
    nuvem::services::runtime::apply_startup_config(&mut app, &outcome.config);
    surface_config_problems(
        &mut app,
        &outcome.problems.clone().into_messages(),
        None,
        &hotkeys,
    );
    let mut session =
        nuvem::services::runtime::session::RuntimeSession::new_with_cli_and_config_for_test(
            cli,
            outcome.config.clone(),
            app,
        );

    let startup = capture(&session);
    assert!(
        status_line(&startup).contains("config: 2 problems (press ? for details)"),
        "the status line must count both problems and point at help:\n{}",
        startup.lines.join("\n")
    );
    write_screenshot(
        "01-startup-status",
        "1. Startup with two broken [columns.*] blocks: the status line counts \
         both problems and names the running help key.",
        &startup,
    );

    press(&mut session, '?').await;
    let overlay = capture(&session);
    let overlay_text = overlay.lines.join("\n");
    assert!(overlay_text.contains("Config problems"), "{overlay_text}");
    assert!(overlay_text.contains("columns.compute"), "{overlay_text}");
    assert!(overlay_text.contains("columns.storage"), "{overlay_text}");
    write_screenshot(
        "02-help-overlay",
        "2. After pressing ?: the help overlay opens on a `Config problems` \
         block listing every problem, bulleted, above the usual bindings.",
        &overlay,
    );

    // Close help, fix the file, reload it the way the `:` command does.
    press(&mut session, '?').await;
    std::fs::write(&path, fixed).expect("rewrite config");
    session
        .dispatch_action(nuvem::app::Action::Effect(nuvem::app::Effect::ReloadConfig))
        .await;

    let reloaded = capture(&session);
    assert!(
        status_line(&reloaded).contains("configuration reloaded"),
        "{}",
        reloaded.lines.join("\n")
    );
    write_screenshot(
        "03-reload-clean",
        "3. The file is fixed and reloaded: the status line says so.",
        &reloaded,
    );

    press(&mut session, '?').await;
    let cleared = capture(&session);
    let cleared_text = cleared.lines.join("\n");
    assert!(
        !cleared_text.contains("Config problems"),
        "a fixed config must leave nothing behind:\n{cleared_text}"
    );
    write_screenshot(
        "04-help-cleared",
        "4. Help reopened after the clean reload: the `Config problems` block \
         is gone, the bindings are untouched.",
        &cleared,
    );
}

/// The pointer names the key the running config binds help to, so a user who
/// rebound it is not sent to a key that does nothing.
#[tokio::test]
async fn the_status_pointer_names_the_rebound_help_key() {
    let file = write_config(
        "version = 1\n\n\
         [hotkeys]\nhelp = \"H\"\n\n\
         [columns.compute]\ncolumns = [\"name\", \"zzz\"]\n\n\
         [columns.storage]\ncolumns = [\"location\"]\n",
    );
    let cli = Cli::parse_from([
        "nuvem",
        "--plugins",
        "/nonexistent/nuvem-plugins-test.toml",
        "--config",
        file.path().to_str().unwrap(),
    ]);
    let outcome = Config::resolve(&cli);
    let hotkeys = outcome.config.hotkeys.clone();

    let mut app = AppState::new();
    nuvem::services::runtime::apply_startup_config(&mut app, &outcome.config);
    surface_config_problems(
        &mut app,
        &outcome.problems.clone().into_messages(),
        None,
        &hotkeys,
    );
    let mut session =
        nuvem::services::runtime::session::RuntimeSession::new_with_cli_and_config_for_test(
            cli,
            outcome.config.clone(),
            app,
        );

    let startup = capture(&session);
    let status = status_line(&startup);
    assert!(
        status.contains("config: 2 problems (press H for details)"),
        "the pointer must name the rebound key, got: {status:?}"
    );
    write_screenshot(
        "05-rebound-help-key",
        "5. With `[hotkeys] help = \"H\"`, the pointer names H — never a \
         hardcoded ?.",
        &startup,
    );

    // And H is the key that actually opens the overlay listing the problems.
    press(&mut session, 'H').await;
    let overlay = capture(&session);
    let overlay_text = overlay.lines.join("\n");
    assert!(overlay_text.contains("Config problems"), "{overlay_text}");
    write_screenshot(
        "06-rebound-help-overlay",
        "6. Pressing H opens the same overlay, with both problems listed.",
        &overlay,
    );
}
