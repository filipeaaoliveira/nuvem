//! Evidence: a plugin's output is retained as lines and read in a
//! viewer, rather than flattened into 200 bytes of one operations-log line.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use nuvem::app::reducer::root::reduce;
use nuvem::app::state::command::CommandDescriptor;
use nuvem::app::state::operations::OperationRecord;
use nuvem::app::{Action, AppError, AppState, CommandAction, Effect, EffectResult, SystemAction};
use nuvem::input::keymap::action_for_key;
use nuvem::input::{Context, Mode};
use nuvem::services::runtime::session::RuntimeSession;
use nuvem_domain::gcp::configuration::{GcloudConfigurationName, GcloudContext};
use nuvem_domain::gcp::diagnostics::DETAIL_MAX_LEN;
use nuvem_domain::gcp::plugin_output::OutputStream;
use ratatui::{Terminal, backend::TestBackend};

use crate::support::keymap_context::base_context;

fn context() -> GcloudContext {
    GcloudContext {
        configuration: GcloudConfigurationName::new("dev"),
        account: Some("dev@example.com".to_string()),
        project: Some("acme-dev".to_string()),
        region: None,
        zone: Some("europe-west1-b".to_string()),
    }
}

fn record_for(app: &AppState, id: u64) -> &OperationRecord {
    app.root
        .system
        .operations
        .records()
        .iter()
        .find(|record| record.id == id)
        .expect("the run's own record")
}

/// The multi-line shape a `describe` actually returns.
const DESCRIBE: &str = "name: web-01\nmachineType: e2-standard-2\nstatus: RUNNING";

/// A run of `count` distinct lines, for the scrolling tests: longer than one
/// page, so the last-page clamp is not what they end up measuring.
fn long_run(count: usize) -> String {
    (0..count)
        .map(|index| format!("disk-{index:02}"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Settles one plugin run, so a test can build a log with several in it.
fn run_settled(app: &mut AppState, label: &str, stdout: &str) {
    let id = app.root.system.operations.begin("plugin", label);
    reduce(
        app,
        &Action::EffectResult(EffectResult::PluginRunSucceeded {
            context: context(),
            label: label.to_string(),
            output: stdout.to_string(),
            stderr: String::new(),
            operation_id: id,
        }),
    );
}

fn render_lines(session: &RuntimeSession, (width, height): (u16, u16)) -> Vec<String> {
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).expect("terminal should initialize");
    terminal
        .draw(|frame| nuvem::ui::render(frame, session))
        .expect("render should succeed");
    let buffer = terminal.backend().buffer().clone();
    let width = buffer.area.width as usize;
    buffer
        .content()
        .chunks(width)
        .map(|line| line.iter().map(|cell| cell.symbol()).collect::<String>())
        .collect()
}

/// The keymap context an open viewer produces.
fn viewer_context() -> Context {
    let mut context = base_context();
    context.mode = Mode::PluginOutput;
    context.plugin_output_open = true;
    context
}

#[test]
fn the_viewers_own_keys_reach_it_and_nothing_else_does() {
    let context = viewer_context();
    let press = |code| action_for_key(KeyEvent::new(code, KeyModifiers::NONE), &context);

    assert_eq!(
        press(KeyCode::Down),
        Some(Action::System(SystemAction::ScrollPluginOutputDown)),
    );
    assert_eq!(
        press(KeyCode::Up),
        Some(Action::System(SystemAction::ScrollPluginOutputUp)),
    );
    assert_eq!(
        press(KeyCode::Char('j')),
        Some(Action::System(SystemAction::ScrollPluginOutputDown)),
    );
    assert_eq!(
        press(KeyCode::Char('k')),
        Some(Action::System(SystemAction::ScrollPluginOutputUp)),
    );
    assert_eq!(
        press(KeyCode::PageDown),
        Some(Action::System(SystemAction::ScrollPluginOutputPageDown)),
    );
    assert_eq!(
        press(KeyCode::PageUp),
        Some(Action::System(SystemAction::ScrollPluginOutputPageUp)),
    );
    assert_eq!(
        press(KeyCode::Char('n')),
        Some(Action::System(SystemAction::PluginOutputOlder)),
    );
    assert_eq!(
        press(KeyCode::Char('p')),
        Some(Action::System(SystemAction::PluginOutputNewer)),
    );
    assert_eq!(
        press(KeyCode::Char('y')),
        Some(Action::System(SystemAction::CopyPluginOutput)),
    );
    assert_eq!(
        press(KeyCode::Esc),
        Some(Action::System(SystemAction::ClosePluginOutput)),
    );
    // `q` quits from Normal. Reaching that from inside the viewer would end
    // the session on a key pressed to leave an overlay.
    assert_eq!(press(KeyCode::Char('q')), None);
    // `r` refreshes the view underneath, which is not the one on screen.
    assert_eq!(press(KeyCode::Char('r')), None);

    // A letter only binds bare. `Ctrl-n`, `Alt-y` and friends belong to the
    // chord sweep pinned elsewhere, and a viewer that claimed them would take
    // them from it silently.
    let chord = |code, modifiers| action_for_key(KeyEvent::new(code, modifiers), &context);
    assert_eq!(chord(KeyCode::Char('n'), KeyModifiers::CONTROL), None);
    assert_eq!(chord(KeyCode::Char('p'), KeyModifiers::CONTROL), None);
    assert_eq!(chord(KeyCode::Char('y'), KeyModifiers::ALT), None);
    assert_eq!(chord(KeyCode::Char('j'), KeyModifiers::CONTROL), None);
    assert_eq!(chord(KeyCode::Char('k'), KeyModifiers::ALT), None);
}

#[test]
fn copying_the_run_on_screen_hands_over_its_lines_not_its_flattened_form() {
    let mut app = AppState::new();
    run_settled(&mut app, "Describe web-01", DESCRIBE);
    open_viewer_through_the_palette(&mut app);

    let effects = reduce(&mut app, &Action::System(SystemAction::CopyPluginOutput));

    let value = effects
        .iter()
        .find_map(|effect| match effect {
            Effect::CopyToClipboard { value, .. } => Some(value.as_str()),
            _ => None,
        })
        .expect("copying emits a clipboard effect");
    assert_eq!(
        value, DESCRIBE,
        "the clipboard gets the run as it was printed"
    );
}

#[test]
fn the_viewer_draws_the_run_as_lines_not_as_one_flattened_string() {
    let mut app = AppState::new();
    run_settled(&mut app, "Describe web-01", DESCRIBE);
    open_viewer_through_the_palette(&mut app);
    let session = RuntimeSession::new_for_test(app);

    let screen = render_lines(&session, (120, 40));

    // Each field on its own row is the whole point: today's single log line
    // would put all three on one row, in whatever order the join produced.
    let row_of = |needle: &str| {
        screen
            .iter()
            .position(|line| line.contains(needle))
            .unwrap_or_else(|| panic!("{needle:?} is not on screen: {screen:#?}"))
    };
    let name = row_of("name: web-01");
    let machine = row_of("machineType: e2-standard-2");
    let status = row_of("status: RUNNING");
    assert!(
        name < machine && machine < status,
        "the lines are drawn in order, one per row",
    );
    assert!(
        screen.iter().any(|line| line.contains("Describe web-01")),
        "the viewer names the run it is showing: {screen:#?}",
    );
    // Three lines on a 40-row terminal: the whole run is visible, which is
    // the condition the truncation notice shares. It must still take *both* —
    // a run this short was never cut.
    assert!(
        !screen.iter().any(|line| line.contains("output truncated")),
        "a run that fits the screen whole claims it was cut: {screen:#?}",
    );
}

#[test]
fn the_viewer_opens_on_the_most_recent_run_and_pages_back_through_the_rest() {
    let mut app = AppState::new();
    run_settled(&mut app, "Describe web-01", DESCRIBE);
    run_settled(&mut app, "List disks", "disk-a\ndisk-b");

    open_viewer_through_the_palette(&mut app);

    let viewer = app.root.system.plugin_output().expect("the viewer is open");
    assert_eq!(viewer.label(), "List disks", "opens on the most recent run");

    reduce(&mut app, &Action::System(SystemAction::PluginOutputOlder));
    let viewer = app.root.system.plugin_output().expect("still open");
    assert_eq!(viewer.label(), "Describe web-01");
    assert_eq!(
        viewer
            .output()
            .lines()
            .first()
            .map(|line| line.text.as_str()),
        Some("name: web-01"),
        "a run is shown from its first line",
    );
}

#[test]
fn a_failed_run_keeps_the_whole_stderr_the_detail_line_had_to_cut() {
    let mut app = AppState::new();
    let id = app.root.system.operations.begin("plugin", "Resize web-01");
    // Longer than DETAIL_MAX_LEN, which is the point: the log line is bounded
    // at 200 bytes and a gcloud error trailer is routinely longer.
    let trailer = format!(
        "ERROR: (gcloud.compute.instances.set-machine-type) Could not fetch resource:\n{}",
        " - the machine type was not found in this zone".repeat(20),
    );

    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::PluginRunFailed {
            context: context(),
            label: "Resize web-01".to_string(),
            error: AppError::gcloud("PERMISSION_DENIED"),
            stderr: trailer.clone(),
            operation_id: id,
        }),
    );

    let record = record_for(&app, id);
    let output = record.output.as_ref().expect("stderr was retained");
    assert!(output.lines().len() > 1, "the trailer kept its lines");
    assert!(
        output
            .lines()
            .iter()
            .all(|line| line.stream == OutputStream::Stderr),
        "a failure's output is stderr",
    );
    let kept: usize = output.lines().iter().map(|line| line.text.len()).sum();
    assert!(
        kept > DETAIL_MAX_LEN,
        "kept {kept} bytes, which is no more than the log line's own bound",
    );
}

#[test]
fn a_settled_run_keeps_its_output_as_lines_on_the_operations_record() {
    let mut app = AppState::new();
    let id = app
        .root
        .system
        .operations
        .begin("plugin", "Describe web-01");

    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::PluginRunSucceeded {
            context: context(),
            label: "Describe web-01".to_string(),
            output: DESCRIBE.to_string(),
            stderr: String::new(),
            operation_id: id,
        }),
    );

    let record = record_for(&app, id);
    let output = record.output.as_ref().expect("output was retained");
    let texts: Vec<&str> = output
        .lines()
        .iter()
        .map(|line| line.text.as_str())
        .collect();
    assert_eq!(
        texts,
        vec![
            "name: web-01",
            "machineType: e2-standard-2",
            "status: RUNNING"
        ],
    );
}

#[test]
fn scrolling_moves_by_a_line_and_stops_at_each_end() {
    let mut app = AppState::new();
    // Longer than one page, or the last-page clamp holds it at the top and
    // this would be testing that instead.
    run_settled(&mut app, "List disks", &long_run(30));
    open_viewer_through_the_palette(&mut app);
    let scroll = |app: &AppState| app.root.system.plugin_output().expect("open").scroll();

    // Already at the top, so up does nothing rather than wrapping to the end.
    reduce(
        &mut app,
        &Action::System(SystemAction::ScrollPluginOutputUp),
    );
    assert_eq!(scroll(&app), 0, "scrolling up at the top stays at the top");

    reduce(
        &mut app,
        &Action::System(SystemAction::ScrollPluginOutputDown),
    );
    assert_eq!(scroll(&app), 1, "down moves exactly one line");
    reduce(
        &mut app,
        &Action::System(SystemAction::ScrollPluginOutputDown),
    );
    reduce(
        &mut app,
        &Action::System(SystemAction::ScrollPluginOutputDown),
    );
    // 30 lines, a 20-line page: the last page starts at line 10.
    for _ in 0..20 {
        reduce(
            &mut app,
            &Action::System(SystemAction::ScrollPluginOutputDown),
        );
    }
    assert_eq!(
        scroll(&app),
        10,
        "down ran past the start of the last page, into blank space",
    );

    reduce(
        &mut app,
        &Action::System(SystemAction::ScrollPluginOutputUp),
    );
    assert_eq!(scroll(&app), 9, "up moves exactly one line back");
}

#[test]
fn paging_between_runs_goes_both_ways_and_stops_at_both_ends() {
    let mut app = AppState::new();
    run_settled(&mut app, "Describe web-01", DESCRIBE);
    run_settled(&mut app, "List disks", "disk-a");
    open_viewer_through_the_palette(&mut app);
    let at = |app: &AppState| {
        let viewer = app.root.system.plugin_output().expect("open");
        (
            viewer.position(&app.root.system.operations),
            viewer.label().to_string(),
        )
    };

    assert_eq!(at(&app), (Some(0), "List disks".to_string()));

    // Newer at the newest end does nothing: there is nothing newer to show.
    reduce(&mut app, &Action::System(SystemAction::PluginOutputNewer));
    assert_eq!(at(&app), (Some(0), "List disks".to_string()));

    reduce(&mut app, &Action::System(SystemAction::PluginOutputOlder));
    assert_eq!(at(&app), (Some(1), "Describe web-01".to_string()));

    // Older at the oldest end likewise stays put rather than wrapping.
    reduce(&mut app, &Action::System(SystemAction::PluginOutputOlder));
    assert_eq!(at(&app), (Some(1), "Describe web-01".to_string()));

    reduce(&mut app, &Action::System(SystemAction::PluginOutputNewer));
    assert_eq!(
        at(&app),
        (Some(0), "List disks".to_string()),
        "paging back returns to the newer run",
    );
}

#[test]
fn esc_closes_the_viewer() {
    let mut app = AppState::new();
    run_settled(&mut app, "Describe web-01", DESCRIBE);
    open_viewer_through_the_palette(&mut app);
    assert!(app.root.system.plugin_output().is_some());

    reduce(&mut app, &Action::System(SystemAction::ClosePluginOutput));

    assert!(
        app.root.system.plugin_output().is_none(),
        "the viewer stayed open after the key that closes it",
    );
}

#[test]
fn a_run_evicted_between_the_palette_and_the_submit_is_reported_not_shown() {
    // The palette offers the entry on the evidence of the log, and the log is
    // a bounded ring: ten further operations between drawing the palette and
    // submitting evict the record the entry was offered for.
    let mut app = AppState::new();
    run_settled(&mut app, "Describe web-01", DESCRIBE);
    reduce(&mut app, &Action::Command(CommandAction::Open));
    for character in "plugin output".chars() {
        reduce(
            &mut app,
            &Action::Command(CommandAction::AppendChar(character)),
        );
    }
    for index in 0..app.root.system.operations.capacity() {
        app.root
            .system
            .operations
            .begin("compute", format!("unrelated {index}"));
    }

    assert!(
        !app.root
            .command
            .visible_commands(&app.root)
            .contains(&CommandDescriptor::ShowPluginOutput),
        "the entry is still offered for a record the log has dropped",
    );

    reduce(&mut app, &Action::Command(CommandAction::Submit));

    assert!(
        app.root.system.plugin_output().is_none(),
        "an empty viewer opened over an evicted run",
    );
}

#[test]
fn the_palette_offers_the_viewer_only_once_a_run_has_output_and_answers_to_its_aliases() {
    let mut app = AppState::new();
    let offered = |app: &AppState| {
        app.root
            .command
            .visible_commands(&app.root)
            .contains(&CommandDescriptor::ShowPluginOutput)
    };
    assert!(!offered(&app), "offered before any plugin has run");

    run_settled(&mut app, "Describe web-01", DESCRIBE);

    assert!(offered(&app), "not offered once a run has output");
    assert!(
        CommandDescriptor::ShowPluginOutput
            .aliases()
            .contains(&"plugin output"),
        "the command answers to nothing typed",
    );
}

#[test]
fn a_truncated_run_says_how_much_it_dropped_at_the_end_of_what_it_kept() {
    let mut app = AppState::new();
    // Two lines past the bound, so the viewer holds a truncated run whose
    // dropped count is worth reading.
    let oversized = format!("{}\n{}", "x".repeat(300_000), "y".repeat(4_000));
    run_settled(&mut app, "Describe web-01", &oversized);
    open_viewer_through_the_palette(&mut app);
    let viewer = app.root.system.plugin_output().expect("open");
    assert!(viewer.output().is_truncated());
    let session = RuntimeSession::new_for_test(app);

    let screen = render_lines(&session, (120, 40));

    // The kept head is one enormous line, so the truncation notice is the
    // second row of the body and on screen without scrolling.
    assert!(
        screen
            .iter()
            .any(|line| line.contains("output truncated") && line.contains("KiB")),
        "nothing on screen says the output was cut: {screen:#?}",
    );
}

#[test]
fn the_footer_counts_the_line_and_the_run_the_operator_is_on() {
    let mut app = AppState::new();
    run_settled(&mut app, "List disks", &long_run(30));
    open_viewer_through_the_palette(&mut app);
    reduce(
        &mut app,
        &Action::System(SystemAction::ScrollPluginOutputDown),
    );
    let session = RuntimeSession::new_for_test(app);

    let screen = render_lines(&session, (120, 40));

    // Anchored on the border, not merely present somewhere on the row: the
    // footer is drawn into the block's bottom border, so it has to start one
    // column inside it. A column out and it either overwrites the corner or
    // hangs outside the box, and `contains` alone cannot tell.
    assert!(
        screen
            .iter()
            .any(|line| line.contains("\u{2514}run 1 of 1") && line.contains("line 2 of 30")),
        "the footer is not drawn just inside the box's bottom-left corner: {screen:#?}",
    );
    assert!(
        !screen.iter().any(|line| line.contains("output truncated")),
        "a run that was never cut claims it was: {screen:#?}",
    );
}

#[test]
fn a_run_that_printed_nothing_is_not_offered_and_cannot_be_opened() {
    // `MissingExecutable`, `Io` and the task-failure path all settle with an
    // empty stderr, so capturing unconditionally attaches an output with no
    // lines in it — offered in the palette, opening on nothing.
    let mut app = AppState::new();
    let id = app
        .root
        .system
        .operations
        .begin("plugin", "Describe web-01");
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::PluginRunFailed {
            context: context(),
            label: "Describe web-01".to_string(),
            error: AppError::gcloud("gcloud CLI not found"),
            stderr: String::new(),
            operation_id: id,
        }),
    );

    assert!(
        !app.root
            .command
            .visible_commands(&app.root)
            .contains(&CommandDescriptor::ShowPluginOutput),
        "a run with nothing to show is offered in the palette",
    );

    // Not offered, so it cannot be submitted; opening it directly is the
    // belt-and-braces half of the same guarantee.
    app.root.system.open_plugin_output();
    assert!(
        app.root.system.plugin_output().is_none(),
        "the viewer opened on a run that printed nothing",
    );
}

/// Runs the `Plugin output` entry the way an operator does: open the palette,
/// type enough to select it, submit.
///
/// The palette is the only way into the viewer, so a test that reaches it any
/// other way is not evidence about the running app.
fn open_viewer_through_the_palette(app: &mut AppState) {
    reduce(app, &Action::Command(CommandAction::Open));
    for character in "plugin output".chars() {
        reduce(app, &Action::Command(CommandAction::AppendChar(character)));
    }
    assert_eq!(
        app.root
            .command
            .selected_visible_command(&app.root)
            .map(|command| command.label().to_string()),
        Some("Plugin output".to_string()),
        "the palette does not select the entry this test is about",
    );
    reduce(app, &Action::Command(CommandAction::Submit));
}

#[test]
fn the_palette_is_how_the_viewer_opens() {
    let mut app = AppState::new();
    run_settled(&mut app, "Describe web-01", DESCRIBE);

    open_viewer_through_the_palette(&mut app);

    let viewer = app
        .root
        .system
        .plugin_output()
        .expect("submitting the palette entry did not open the viewer");
    assert_eq!(viewer.label(), "Describe web-01");
}

#[test]
fn a_run_settling_while_the_viewer_is_open_does_not_move_it_onto_another_run() {
    // Plugin runs are asynchronous and the viewer is opened to read a run
    // that has already finished, so a second run settling underneath it is
    // the ordinary case rather than a race corner. An index into the log is
    // not a name for a run: the log grows at the front.
    let mut app = AppState::new();
    run_settled(&mut app, "Describe web-01", DESCRIBE);
    open_viewer_through_the_palette(&mut app);
    assert_eq!(
        app.root.system.plugin_output().expect("open").label(),
        "Describe web-01"
    );

    run_settled(&mut app, "List disks", "disk-a");

    let viewer = app.root.system.plugin_output().expect("still open");
    assert_eq!(
        viewer.label(),
        "Describe web-01",
        "the viewer moved onto a run that settled underneath it",
    );
    assert_eq!(
        viewer.position(&app.root.system.operations),
        Some(1),
        "the viewer still calls itself the newest run, though a newer one arrived",
    );

    // The newer run is reachable, and going back returns to the one that was
    // on screen — neither of which holds if position is a stale index.
    reduce(&mut app, &Action::System(SystemAction::PluginOutputNewer));
    assert_eq!(
        app.root.system.plugin_output().expect("open").label(),
        "List disks",
        "the run that settled underneath is unreachable without reopening",
    );
    reduce(&mut app, &Action::System(SystemAction::PluginOutputOlder));
    assert_eq!(
        app.root.system.plugin_output().expect("open").label(),
        "Describe web-01",
    );
}

#[test]
fn copying_a_run_keeps_stderr_distinguishable_from_the_answer() {
    // The renderer marks stderr because which stream a line came from is what
    // tells a warning from an answer. A clipboard blob that drops the mark
    // loses exactly that, on the run type `y` exists for.
    let mut app = AppState::new();
    let id = app.root.system.operations.begin("plugin", "Resize web-01");
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::PluginRunFailed {
            context: context(),
            label: "Resize web-01".to_string(),
            error: AppError::gcloud("PERMISSION_DENIED"),
            stderr: "ERROR: machine type not found".to_string(),
            operation_id: id,
        }),
    );
    open_viewer_through_the_palette(&mut app);

    let effects = reduce(&mut app, &Action::System(SystemAction::CopyPluginOutput));

    let value = effects
        .iter()
        .find_map(|effect| match effect {
            Effect::CopyToClipboard { value, .. } => Some(value.as_str()),
            _ => None,
        })
        .expect("copying emits a clipboard effect");
    assert_eq!(value, "stderr │ ERROR: machine type not found");
}

#[test]
fn a_run_shorter_than_a_page_does_not_scroll_into_blank_space() {
    // Clamping to the last line let a three-line run scroll until one line
    // sat above a screen of nothing. The reducer has no viewport, so the page
    // step is the only honest stand-in for one.
    let mut app = AppState::new();
    run_settled(&mut app, "List disks", "disk-a\ndisk-b\ndisk-c");
    open_viewer_through_the_palette(&mut app);

    for _ in 0..5 {
        reduce(
            &mut app,
            &Action::System(SystemAction::ScrollPluginOutputPageDown),
        );
    }

    assert_eq!(
        app.root.system.plugin_output().expect("open").scroll(),
        0,
        "a run shorter than one page scrolled away from its own first line",
    );
}
