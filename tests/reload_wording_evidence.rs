//! Visual evidence — one reload, two broken `[columns.*]` blocks,
//! two different tails, each tied to what its service is actually showing.
//!
//! `tests/columns_reload_test.rs` pins the sentences as strings. This binary
//! answers the reviewer's other question: what does a user actually see. The
//! frames go through the real `nuvem::ui::render` path — the compute table
//! still drawn with the kept columns while the help overlay says "keeping the
//! column settings already in use" about it, and "ignoring this service's
//! column settings" about gke, which never had any running.
//!
//! Assertions run unconditionally; frames are written only when
//! `NUVEM_EVIDENCE_DIR` is set, and only under that directory.

use std::fs;
use std::io::Write;
use std::path::PathBuf;

use clap::Parser;
use nuvem::app::reducer::root::reduce;
use nuvem::app::{Action, AppState, Effect, EffectResult, ResourcesAction, WorkspaceAction};
use nuvem::services::runtime::session::RuntimeSession;
use nuvem::services::runtime::{apply_startup_config, input_context_for_test};
use nuvem::{Cli, Config};
use nuvem_domain::gcp::configuration::{GcloudConfiguration, GcloudConfigurationName};
use nuvem_domain::gcp::resource::{ComputeInstance, ResourceServiceKind, ResourceStatus};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

use crate::support::frame_svg;

use frame_svg::buffer_to_svg;

const WIDTH: u16 = 120;
const HEIGHT: u16 = 30;

/// The list compute runs when the reload arrives — away from the catalog
/// default, so a kept list and a fallen-back one cannot be confused.
const RUNNING: &str =
    "version = 1\n\n[columns.compute]\ncolumns = [\"zone\", \"name\", \"status\"]\n";

/// The reload body: compute's block breaks while its settings are running,
/// gke's breaks with nothing running. Same file, same reload, two tails.
const BROKEN_BOTH: &str = "version = 1\n\n\
    [columns.compute]\ncolumns = [\"name\", \"zzz\"]\n\n\
    [columns.gke]\ncolumns = [\"zzz\"]\n";

const KEPT: &str = "keeping the column settings already in use";
const FALLBACK: &str = "ignoring this service's column settings";

fn write_config(body: &str) -> tempfile::NamedTempFile {
    let mut file = tempfile::NamedTempFile::new().expect("create temp config");
    file.write_all(body.as_bytes()).expect("write temp config");
    file.flush().expect("flush temp config");
    file
}

fn configuration() -> GcloudConfiguration {
    GcloudConfiguration {
        name: GcloudConfigurationName::new("dev"),
        is_active: true,
        account: Some("dev@example.com".to_string()),
        project: Some("dev-project".to_string()),
        region: Some("europe-west1".to_string()),
        zone: Some("europe-west1-b".to_string()),
    }
}

fn instance(name: &str, status: ResourceStatus, zone: &str) -> ComputeInstance {
    ComputeInstance {
        id: name.to_string(),
        name: name.to_string(),
        zone: zone.to_string(),
        status,
        machine_type: "e2-medium".to_string(),
        internal_ip: Some("10.0.0.4".to_string()),
        external_ip: None,
    }
}

/// A session on the compute list, wired the way `run()` wires the real one,
/// with the `RUNNING` columns installed and rows on screen.
fn session_with_compute_rows(path: &std::path::Path) -> RuntimeSession {
    let cli = Cli::parse_from([
        "nuvem",
        "--plugins",
        "/nonexistent/nuvem-plugins-test.toml",
        "--config",
        path.to_str().expect("utf-8 path"),
    ]);
    let outcome = Config::resolve(&cli);
    assert!(
        outcome.problems.is_empty(),
        "the starting config must validate cleanly: {:?}",
        outcome.problems.messages()
    );
    let config = outcome.config;

    let mut app = AppState::new();
    apply_startup_config(&mut app, &config);
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ConfigurationsLoaded {
            configurations: vec![configuration()],
        }),
    );
    reduce(&mut app, &Action::Workspace(WorkspaceAction::Resources));
    reduce(
        &mut app,
        &Action::Resources(ResourcesAction::SelectService(
            ResourceServiceKind::ComputeInstances,
        )),
    );
    let context = app.root.current_context().expect("active context").clone();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ComputeInstancesLoaded {
            context,
            instances: vec![
                instance("api", ResourceStatus::Suspended, "europe-west1-a"),
                instance("db", ResourceStatus::Stopped, "europe-west1-b"),
                instance("web", ResourceStatus::Pending, "europe-west1-c"),
            ],
        }),
    );

    RuntimeSession::new_with_cli_and_config_for_test(cli, config, app)
}

fn draw(session: &RuntimeSession) -> Buffer {
    let mut terminal = Terminal::new(TestBackend::new(WIDTH, HEIGHT)).expect("terminal init");
    terminal
        .draw(|frame| nuvem::ui::render(frame, session))
        .expect("render");
    terminal.backend().buffer().clone()
}

fn text(buffer: &Buffer) -> String {
    (0..buffer.area.height)
        .map(|y| {
            (0..buffer.area.width)
                .map(|x| buffer[(x, y)].symbol().to_string())
                .collect::<String>()
                .trim_end()
                .to_string()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// The frame's text with box-drawing and wrapping collapsed away, so a
/// sentence the overlay wraps over two lines can still be searched for whole.
fn flat_text(buffer: &Buffer) -> String {
    text(buffer)
        .split(|c: char| !c.is_ascii())
        .flat_map(str::split_whitespace)
        .collect::<Vec<_>>()
        .join(" ")
}

/// Sends one real key event through the real input context and keymap, the
/// way the event loop does.
async fn press(session: &mut RuntimeSession, ch: char) {
    let key = crossterm::event::KeyEvent::new(
        crossterm::event::KeyCode::Char(ch),
        crossterm::event::KeyModifiers::NONE,
    );
    let context = input_context_for_test(session, Rect::new(0, 0, WIDTH, HEIGHT));
    let action = nuvem::input::keymap::action_for_key(key, &context)
        .unwrap_or_else(|| panic!("key {ch:?} is unbound in this context"));
    session.dispatch_action(action).await;
}

fn evidence_dir() -> Option<PathBuf> {
    let dir = std::env::var("NUVEM_EVIDENCE_DIR").ok()?;
    let path = PathBuf::from(dir);
    fs::create_dir_all(&path).ok()?;
    Some(path)
}

fn write_frames(frames: &[(&str, String, String, Buffer)]) {
    let Some(dir) = evidence_dir() else {
        eprintln!("NUVEM_EVIDENCE_DIR not set; skipping evidence render");
        return;
    };
    for (name, title, caption, buffer) in frames {
        let svg = buffer_to_svg(buffer, title, caption);
        fs::write(dir.join(name), svg).expect("write svg frame");
        let ascii = format!("{title}\n{caption}\n\n{}\n", text(buffer));
        fs::write(
            dir.join(format!("{}.txt", name.trim_end_matches(".svg"))),
            ascii,
        )
        .expect("write ascii frame");
    }
}

/// The end-user walkthrough: compute runs its configured columns, a reload
/// breaks compute's block and a never-running gke block in the same file, and
/// the user reads the table, the status line, and the help overlay.
#[tokio::test]
async fn each_broken_block_describes_its_own_outcome_on_screen() {
    let file = write_config(RUNNING);
    let mut session = session_with_compute_rows(file.path());

    std::fs::write(file.path(), BROKEN_BOTH).expect("rewrite temp config");
    session
        .dispatch_action(Action::Effect(Effect::ReloadConfig))
        .await;

    let mut frames: Vec<(&str, String, String, Buffer)> = Vec::new();

    // ---- 1. The table after the reload: the kept columns, loudly ----------
    let table_frame = draw(&session);
    let rendered = text(&table_frame);
    assert!(
        rendered.contains("config reloaded (2 problems; press ? for details)"),
        "the status line counts both problems:\n{rendered}"
    );
    let header_row = rendered
        .lines()
        .find(|line| line.contains("ZONE"))
        .unwrap_or_else(|| panic!("the compute table draws its header:\n{rendered}"));
    let positions: Vec<usize> = ["ZONE", "NAME", "STATUS"]
        .iter()
        .map(|key| header_row.find(key).expect("kept column on screen"))
        .collect();
    assert!(
        positions.windows(2).all(|pair| pair[0] < pair[1]),
        "the running order zone, name, status survives the broken reload: {header_row:?}"
    );
    frames.push((
        "01-table-after-broken-reload.svg",
        "1. After a reload that breaks [columns.compute] and [columns.gke]".to_string(),
        "The compute table still shows the running ZONE, NAME, STATUS order; the status \
         line counts 2 problems and points at help."
            .to_string(),
        table_frame,
    ));

    // ---- 2. The help overlay: one sentence per service, each true ---------
    press(&mut session, '?').await;
    let overlay_frame = draw(&session);
    let flat = flat_text(&overlay_frame);
    assert!(
        flat.contains(&format!(
            "columns.compute: unknown column \"zzz\"; ignoring this block, {KEPT}"
        )),
        "compute's sentence matches the table still on screen:\n{flat}"
    );
    assert!(
        flat.contains(&format!("columns.gke: unknown column \"zzz\"; {FALLBACK}")),
        "gke never had running settings, so its sentence still says fallback:\n{flat}"
    );
    frames.push((
        "02-help-overlay-two-tails.svg",
        "2. The help overlay after pressing ?".to_string(),
        "Same reload, two sentences: compute (settings were running) reads \"keeping the \
         column settings already in use\"; gke (nothing running) reads \"ignoring this \
         service's column settings\"."
            .to_string(),
        overlay_frame,
    ));

    write_frames(&frames);
}
