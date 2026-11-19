//! Every overlay leaves nothing of the panes beneath it half-shown.
//!
//! A box drawn over the panes keeps whatever words its edges cut through on
//! the rows it crosses (`│      [│`, `│.0.4   │` around a Stop confirmation).
//! Each overlay is reached here the way an operator reaches it, in a demo
//! session, and the body rows are read back: outside the overlay's box they
//! must be blank. `is_overlay` is an exhaustive match over the input modes, so
//! a new modal mode does not compile until it is classified, and the walk
//! asserts it reached every mode classified as an overlay.

use std::collections::BTreeSet;
use std::time::{Duration, Instant};

use clap::Parser;
use nuvem::app::{
    Action, AppState, CommandAction, ConfigurationsAction, JumpsAction, ResourcesAction,
    SystemAction, WorkspaceAction,
};
use nuvem::input::Mode;
use nuvem::services::runtime::session::RuntimeSession;
use nuvem_domain::gcp::resource::ResourceServiceKind;
use ratatui::layout::Rect;

/// The two sizes the docs and recordings use.
const SIZES: [(u16, u16); 2] = [(80, 24), (121, 34)];

fn is_overlay(mode: Mode) -> bool {
    match mode {
        Mode::Help
        | Mode::CommandPalette
        | Mode::ConfigurationContextPicker
        | Mode::ConfirmConfigurationActivation
        | Mode::ConfirmResourceAction
        | Mode::ResourceCommandPreview
        | Mode::SshLaunchPreview
        | Mode::TunnelLaunchPreview
        | Mode::ConfirmK9sLaunch
        | Mode::PluginConfirmation
        | Mode::PluginPrompt
        | Mode::ResourceJumpPicker
        | Mode::PluginOutput => true,
        Mode::Normal | Mode::Filter | Mode::EditConfiguration | Mode::PluginPrefix => false,
    }
}

const PLUGINS: &str = r#"version = 1

[[plugins]]
id = "probe"
scope = "compute"
key = "p"
label = "Probe {name}"
args = ["compute", "instances", "describe", "{name}"]

[[plugins]]
id = "resize"
scope = "compute"
key = "z"
label = "Note {name}"
args = ["compute", "instances", "add-labels", "{name}", "--labels=note={note}"]

[[plugins.inputs]]
name = "note"
type = "string"
"#;

async fn settle(session: &mut RuntimeSession) {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let in_flight = session.effect_tasks_in_flight();
        let progressed = matches!(
            tokio::time::timeout(
                Duration::from_millis(5),
                session.process_next_effect_result()
            )
            .await,
            Ok(true)
        );
        if !progressed && in_flight == 0 {
            return;
        }
        assert!(Instant::now() < deadline, "the session settles");
    }
}

async fn session_on(kind: ResourceServiceKind, plugins: &std::path::Path) -> RuntimeSession {
    let cli = nuvem::Cli::parse_from([
        "nuvem",
        "--demo",
        "--config",
        "/nonexistent/nuvem.toml",
        "--plugins",
        plugins.to_str().expect("utf-8"),
    ]);
    let config = nuvem::Config::resolve(&cli).config;
    let state_path = nuvem::config::state::state_path_for(&config.config_path);
    let mut app = AppState::new();
    nuvem::services::runtime::apply_startup_config(&mut app, &config);
    let mut session = RuntimeSession::new(config, cli, app, state_path);
    session.initialize().await;
    settle(&mut session).await;
    for action in [
        Action::Workspace(WorkspaceAction::Resources),
        Action::Resources(ResourcesAction::SelectService(kind)),
    ] {
        session.dispatch_action(action).await;
        settle(&mut session).await;
    }
    session
}

async fn act(session: &mut RuntimeSession, actions: Vec<Action>) {
    for action in actions {
        session.dispatch_action(action).await;
        settle(session).await;
    }
}

fn palette(query: &str) -> Vec<Action> {
    let mut actions = vec![Action::Command(CommandAction::Open)];
    actions.extend(
        query
            .chars()
            .map(|c| Action::Command(CommandAction::AppendChar(c))),
    );
    actions.push(Action::Command(CommandAction::Submit));
    actions
}

fn screen(session: &RuntimeSession, width: u16, height: u16) -> Vec<String> {
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, height))
        .expect("terminal");
    terminal
        .draw(|frame| nuvem::ui::render(frame, session))
        .expect("draws");
    let buffer = terminal.backend().buffer();
    (0..height)
        .map(|y| (0..width).map(|x| buffer[(x, y)].symbol()).collect())
        .collect()
}

/// The body rows (below the breadcrumb, above the status line) show nothing
/// but the overlay's own box.
fn assert_backdrop_is_clear(name: &str, lines: &[String]) {
    // Help covers the breadcrumb row too, and a too-small-terminal notice the
    // whole frame; every other overlay starts below the breadcrumb.
    let top = (0..lines.len() - 3)
        .find(|&row| lines[row].contains('┌'))
        .unwrap_or_else(|| panic!("{name}: no overlay box in the body:\n{}", lines.join("\n")));
    let left = lines[top].chars().position(|c| c == '┌').expect("corner");
    let right = lines[top]
        .chars()
        .enumerate()
        .filter(|(_, c)| *c == '┐')
        .map(|(column, _)| column)
        .last()
        .expect("corner");
    for row in top.min(3)..lines.len() - 3 {
        let outside: String = lines[row]
            .chars()
            .enumerate()
            .filter(|(column, _)| *column < left || *column > right)
            .map(|(_, c)| c)
            .collect();
        assert!(
            outside.trim().is_empty(),
            "{name}: row {row} shows {outside:?} beside the box:\n{}",
            lines.join("\n")
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn every_overlay_clears_the_panes_beneath_it() {
    let directory = tempfile::tempdir().expect("tempdir");
    let plugins = directory.path().join("plugins.toml");
    std::fs::write(&plugins, PLUGINS).expect("writes");

    let gce = ResourceServiceKind::ComputeInstances;
    let walks: Vec<(&str, ResourceServiceKind, Vec<Action>)> = vec![
        ("help", gce, vec![Action::System(SystemAction::ToggleHelp)]),
        ("palette", gce, vec![Action::Command(CommandAction::Open)]),
        (
            "context picker",
            gce,
            vec![
                Action::Workspace(WorkspaceAction::Configurations),
                Action::Configurations(ConfigurationsAction::StartEditActiveContext),
                Action::Configurations(ConfigurationsAction::OpenContextPicker),
            ],
        ),
        (
            "activation confirmation",
            gce,
            vec![
                Action::Workspace(WorkspaceAction::Configurations),
                Action::Configurations(ConfigurationsAction::SelectNext),
                Action::Configurations(ConfigurationsAction::RequestActivateSelected),
            ],
        ),
        ("stop confirmation", gce, palette("stop selected")),
        ("command preview", gce, palette("show ssh")),
        ("ssh preview", gce, palette("ssh to")),
        (
            "tunnel preview",
            gce,
            vec![Action::Resources(ResourcesAction::RequestTunnel)],
        ),
        (
            "k9s confirmation",
            ResourceServiceKind::GkeClusters,
            palette("k9s"),
        ),
        ("jump picker", gce, vec![Action::Jumps(JumpsAction::Open)]),
        ("plugin confirmation", gce, palette("probe")),
        ("plugin prompt", gce, palette("note")),
        ("plugin output", gce, {
            let mut actions = palette("probe");
            actions.push(Action::Resources(ResourcesAction::ConfirmPlugin));
            actions.extend(palette("plugin output"));
            actions
        }),
    ];

    let mut reached = BTreeSet::new();
    for (name, kind, actions) in walks {
        let mut session = session_on(kind, &plugins).await;
        act(&mut session, actions).await;
        for (width, height) in SIZES {
            let mode = nuvem::services::runtime::input_context_for_test(
                &session,
                Rect::new(0, 0, width, height),
            )
            .mode;
            assert!(
                is_overlay(mode),
                "{name}: the walk reached {mode:?}, not an overlay"
            );
            reached.insert(format!("{mode:?}"));
            assert_backdrop_is_clear(
                &format!("{name} at {width}x{height}"),
                &screen(&session, width, height),
            );
        }
    }
    let overlays: BTreeSet<String> = Mode::ALL
        .into_iter()
        .filter(|mode| is_overlay(*mode))
        .map(|mode| format!("{mode:?}"))
        .collect();
    assert_eq!(reached, overlays, "every overlay mode is walked");
}
