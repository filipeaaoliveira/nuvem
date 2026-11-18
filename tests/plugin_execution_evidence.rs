//! Evidence: a plugin's confirmation renders the exact argv that will
//! run, and running it is what the modal's `Enter` does.

use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use nuvem::app::effect::GcloudFailure;
use nuvem::app::reducer::root::reduce;
use nuvem::app::state::operations::{OperationOutcome, OperationRecord};
use nuvem::app::state::system::OperationPhase;
use nuvem::app::{Action, AppError, AppState, Effect, EffectResult, ResourcesAction};
use nuvem::input::Mode;
use nuvem::input::keymap::action_for_key;
use nuvem::services::gcloud::cli::{
    CommandError, CommandOutput, CommandProgram, CommandRunner, GcloudCommand,
};
use nuvem::services::runtime::input_context_for_test;
use nuvem::services::runtime::session::RuntimeSession;
use nuvem_domain::gcp::configuration::{
    GcloudConfiguration, GcloudConfigurationName, GcloudContext,
};
use nuvem_domain::gcp::diagnostics::DETAIL_MAX_LEN;
use nuvem_domain::gcp::resource::{ComputeInstance, ResourceStatus};
use ratatui::{Terminal, backend::TestBackend, buffer::Buffer, layout::Rect};

use crate::plugin_preview_evidence::Operator;
use crate::support::frame_svg::buffer_to_svg;

/// A terminal comfortably larger than the modal, so the assertions below are
/// about what the renderer draws rather than about the small-terminal fallback.
const WIDE: (u16, u16) = (120, 40);

/// The caption the fixture plugin is confirmed under.
fn plugin_label() -> String {
    "Describe web 01".to_string()
}

/// The argv the fixture plugin runs. The spaced last element is the fixture
/// that catches argv being flattened through anything shell-shaped.
fn plugin_fixture_argv() -> Vec<String> {
    vec![
        "--configuration".to_string(),
        "dev".to_string(),
        "compute".to_string(),
        "instances".to_string(),
        "describe".to_string(),
        "web 01".to_string(),
    ]
}

fn session_confirming(dangerous: bool) -> RuntimeSession {
    let mut app = AppState::new();
    app.root
        .resources
        .show_plugin_confirmation(plugin_label(), plugin_fixture_argv(), dangerous);
    RuntimeSession::new_for_test(app)
}

/// The same confirmation with an argv too long to fit the smallest terminal
/// nuvem supports, which is what the fallback needs now the
/// modal wraps rather than demanding each line fit one row.
fn session_confirming_a_long_argv() -> RuntimeSession {
    let mut argv = plugin_fixture_argv();
    argv.push("--project=acme-production-platform".to_string());
    argv.push("--format=json(name,status,machineType,networkInterfaces)".to_string());
    let mut app = AppState::new();
    app.root
        .resources
        .show_plugin_confirmation(plugin_label(), argv, false);
    RuntimeSession::new_for_test(app)
}

/// The rows the `run plugin` modal itself occupies, from its titled top border
/// through its bottom border. Negative assertions are made against these rather
/// than the whole screen, so "the modal does not say X" cannot be satisfied by
/// the modal being absent, nor broken by unrelated chrome saying X.
fn modal_lines(screen: &[String]) -> Vec<String> {
    let rows: Vec<Vec<char>> = screen.iter().map(|line| line.chars().collect()).collect();
    let top = screen
        .iter()
        .position(|line| line.contains("run plugin"))
        .unwrap_or_else(|| panic!("the run plugin modal is on screen: {screen:#?}"));
    // The modal clears only its own rectangle, so the workspace body's own
    // border characters survive to the left and right of it on every row it
    // covers. Both bounds are read off the modal's top border and every row is
    // cut to them, which is what makes these lines the modal's and nothing
    // else's.
    let left = rows[top]
        .iter()
        .position(|glyph| *glyph == '\u{250c}')
        .unwrap_or_else(|| panic!("the modal's top border opens it: {screen:#?}"));
    let right = rows[top]
        .iter()
        .position(|glyph| *glyph == '\u{2510}')
        .unwrap_or_else(|| panic!("the modal's top border closes it: {screen:#?}"));

    let mut lines = Vec::new();
    for row in &rows[top..] {
        let line: String = row[left..=right].iter().collect();
        let last = line.starts_with('\u{2514}');
        lines.push(line);
        if last {
            return lines;
        }
    }
    panic!("the modal's bottom border closes it: {screen:#?}");
}

fn render_buffer(session: &RuntimeSession, (width, height): (u16, u16)) -> Buffer {
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).expect("terminal should initialize");
    terminal
        .draw(|frame| nuvem::ui::render(frame, session))
        .expect("render should succeed");
    terminal.backend().buffer().clone()
}

fn lines_of(buffer: &Buffer) -> Vec<String> {
    let width = buffer.area.width as usize;
    buffer
        .content()
        .chunks(width)
        .map(|line| line.iter().map(|cell| cell.symbol()).collect::<String>())
        .collect()
}

fn render_lines(session: &RuntimeSession, size: (u16, u16)) -> Vec<String> {
    lines_of(&render_buffer(session, size))
}

/// A frame written as an image, the way the other evidence binaries in this
/// suite do it: the modal this slice adds is a surface an operator reads, and
/// the assertions below say what it must contain but not what it looks like.
/// Written only when `NUVEM_EVIDENCE_DIR` is set, before the assertions run, so a
/// failing run still leaves the frame that explains the failure.
fn write_frame(name: &str, title: &str, caption: &str, buffer: &Buffer) {
    let Ok(dir) = std::env::var("NUVEM_EVIDENCE_DIR") else {
        return;
    };
    let dir = std::path::PathBuf::from(dir);
    std::fs::create_dir_all(&dir).expect("evidence directory is creatable");
    std::fs::write(
        dir.join(format!("{name}.svg")),
        buffer_to_svg(buffer, title, caption),
    )
    .expect("write svg frame");
    std::fs::write(
        dir.join(format!("{name}.txt")),
        format!("{title}\n{caption}\n\n{}\n", lines_of(buffer).join("\n")),
    )
    .expect("write ascii frame");
}

/// The README's own walkthrough as far as the modal: `:`, the plugin's label
/// typed in, Enter. Driven through the real keymap and reducer rather than by
/// calling `show_plugin_confirmation` directly, because the claim under test is
/// about which modal the *palette* opens.
async fn submit_the_plugin_from_the_palette(operator: &mut Operator) {
    operator.press(KeyCode::Char(':')).await;
    operator.type_text("describe").await;
    operator.press(KeyCode::Enter).await;
}

#[tokio::test]
async fn submitting_a_plugin_from_the_palette_opens_the_confirmation_not_the_copy_only_preview() {
    let mut operator = Operator::at_the_compute_list(false).await;
    submit_the_plugin_from_the_palette(&mut operator).await;

    let resources = &operator.session.app().root.resources;
    assert!(
        resources.plugin_confirmation().is_some(),
        "the palette must open the confirmation that can run"
    );
    assert!(
        resources.command_preview().is_none(),
        "the copy-only preview must not be what a plugin opens once execution exists"
    );
}

/// The whole chain, once, through nothing but real parts: the config parser's
/// plugin, the real keymap and mode resolver, the reducer, the effect, the
/// runtime's blocking dispatch and the `CommandRunner` a real child would be
/// spawned from. Every other test in this file cuts the chain somewhere — the
/// reducer tests stop at the effect, the session tests hand-build one — so this
/// is the only place where "pressing Enter on the confirmation runs the
/// operator's command" is asserted end to end.
///
/// `Enter` is pressed twice for the claim the brief makes and the reducer test
/// can only half-prove: the confirmation is *taken*, so the second press finds
/// nothing and the child runs once. A borrow-instead-of-take would show up here
/// as two runner calls, which is the failure the operator would actually feel.
#[tokio::test]
async fn pressing_enter_on_the_confirmation_runs_the_plugin_once_through_the_real_keymap() {
    let runner = RecordingRunner::default();
    let mut operator =
        Operator::at_the_compute_list_with_runner(false, Box::new(runner.clone())).await;
    // The README's row replaced by one whose name carries a space, for the
    // reason the renderer test uses the same fixture: it is what catches argv
    // flattened through anything shell-shaped, here on the last hop before the
    // child rather than at the modal.
    let context = operator
        .session
        .app()
        .root
        .current_context()
        .expect("the dev configuration resolves")
        .clone();
    operator
        .session
        .apply_effect_result(EffectResult::ComputeInstancesLoaded {
            context,
            instances: vec![ComputeInstance {
                id: "web-01-id".to_string(),
                name: "web 01".to_string(),
                zone: "europe-west1-b".to_string(),
                status: ResourceStatus::Running,
                machine_type: "e2-medium".to_string(),
                internal_ip: Some("10.0.0.2".to_string()),
                external_ip: None,
            }],
        })
        .await;

    operator.press(KeyCode::Char(':')).await;
    operator.type_text("describe").await;
    operator.press(KeyCode::Enter).await;
    assert!(
        operator
            .session
            .app()
            .root
            .resources
            .plugin_confirmation()
            .is_some(),
        "the palette must have left a confirmation to answer"
    );

    operator.press(KeyCode::Enter).await;
    // The run is on a blocking thread; its result reaching the queue is what
    // proves the child was handed the argv. Drained in a loop until this run's
    // own entry settles, rather than once: this operator's session has other
    // results in flight — the compute list the palette walk itself triggered —
    // and a single `recv()` takes whichever arrives first, which under load is
    // the list about half the time.
    //
    // The loop is deterministic, not a retry: the runtime sends exactly one
    // result per run, so an entry still Pending means one is still on its way
    // and `recv()` cannot block forever. The bound turns "no result ever came"
    // into this message instead of a suite that hangs.
    let mut drained = 0;
    while operator
        .session
        .app()
        .root
        .system
        .operations
        .pending_count()
        > 0
    {
        assert!(
            drained < 8,
            "the plugin's entry was still pending after {drained} results"
        );
        assert!(
            operator.session.process_next_effect_result().await,
            "the result channel closed with the plugin's entry still pending"
        );
        drained += 1;
    }
    // The second Enter, and then the action itself. The keypress goes wherever
    // the mode resolver now sends it — the modal is gone, so it opens the row's
    // detail, which is what "the confirmation found nothing" looks like from the
    // keyboard. The dispatched action is the stricter half: it reaches the arm
    // directly, so a confirmation that was borrowed rather than taken would run
    // the child a second time here even though no modal is on screen.
    operator.press(KeyCode::Enter).await;
    operator
        .session
        .dispatch_action(Action::Resources(ResourcesAction::ConfirmPlugin))
        .await;

    // The deterministic half of "exactly once", asserted before the argv count
    // below because it does not depend on a thread having got there yet: the
    // confirm arm seeds its operations-log entry *synchronously*, in the same
    // `reduce` that emits the effect, so a confirmation that was borrowed
    // rather than taken has already left a second `plugin` entry here. The
    // argv count reads a `Mutex` the runner fills from a `spawn_blocking`
    // thread, so on its own it can read 1 while a second run is still on its
    // way to the runner — the assertion is worth what it says only with this
    // one standing next to it.
    let seeded = operator
        .session
        .app()
        .root
        .system
        .operations
        .records()
        .iter()
        .filter(|record| record.kind == "plugin")
        .count();
    assert_eq!(
        seeded, 1,
        "one confirmation is one run: a second ConfirmPlugin must seed no second entry"
    );

    let expected = vec![
        "--configuration",
        "dev",
        "compute",
        "instances",
        "describe",
        "web 01",
        "--zone",
        "europe-west1-b",
    ];
    let calls = runner.argv.lock().unwrap().clone();
    assert_eq!(
        calls.iter().filter(|argv| *argv == &expected).count(),
        1,
        "the plugin's argv must have reached the child exactly once — with the \
         injected configuration first and 'web 01' still one element. Calls were: \
         {calls:?}"
    );
    // The loop above already established that nothing is pending. What this
    // adds is the outcome: the entry the confirm seeded settled as a *success*,
    // so the whole chain — keymap, reducer, effect, blocking dispatch, runner,
    // result arm — closed on the operator's own command rather than on a
    // failure that would also have emptied the pending count.
    let outcome = operator
        .session
        .app()
        .root
        .system
        .operations
        .records()
        .iter()
        .find(|record| record.kind == "plugin")
        .map(|record| record.outcome);
    assert_eq!(
        outcome,
        Some(OperationOutcome::Succeeded),
        "the entry the confirm seeded must have settled on the run's result"
    );
}

/// The plugin here declares `delete` rather than the shared fixture's
/// `describe`: `--read-only` admits a plugin whose declared command
/// path nuvem can prove only reads, so the `describe` fixture reaches its
/// confirmation now and would leave this test refusing nothing.
#[tokio::test]
async fn read_only_refuses_the_plugin_before_any_confirmation_opens() {
    let mut operator = Operator::at_the_compute_list_with_a_mutating_plugin(true).await;
    operator.press(KeyCode::Char(':')).await;
    operator.type_text("delete").await;
    operator.press(KeyCode::Enter).await;

    let root = &operator.session.app().root;
    assert!(
        root.resources.plugin_confirmation().is_none(),
        "read-only must refuse at the palette, leaving nothing to confirm"
    );
    assert!(
        root.resources.command_preview().is_none(),
        "read-only must not fall back to the copy-only preview either"
    );
    // Both assertions above are negative, so a palette filter that matched no
    // row at all would satisfy them having submitted nothing. The refusal
    // sentence is what proves the walk reached the guard, and naming the plugin
    // is what proves it was this row.
    assert_eq!(
        root.system.status_message.as_deref(),
        Some("Delete web-01 is unavailable: read-only mode cannot prove this only reads"),
        "the refusal must have fired and named the plugin the operator submitted"
    );
}

#[test]
fn the_plugin_confirmation_renders_the_exact_argv_including_a_spaced_argument() {
    // A spaced argument is the fixture that catches a renderer flattening argv
    // through anything shell-shaped: it must appear quoted as ONE argument.
    let argv = plugin_fixture_argv();

    let mut state = nuvem::app::state::resources::ResourcesState::default();
    state.show_plugin_confirmation(plugin_label(), argv.clone(), false);
    let confirmation = state.plugin_confirmation().expect("the modal opened");
    let line = confirmation.command_preview();

    // What the modal *holds*, not just what it draws. `PluginConfirmation`
    // exists to carry the argv rather than a rendered line — that is the whole
    // reason the operator reads and the child receives the same object — so the
    // elements it hands back are worth an assertion of their own. Nothing else
    // in the suite reads this accessor: `command_preview` renders from the
    // field, and `plugin_confirmation_submission` destructures it, so an `argv`
    // that answered with nothing at all went unnoticed by every other test here.
    assert_eq!(
        confirmation.argv(),
        argv.as_slice(),
        "the confirmation must carry the offer's argv element for element, with \
         'web 01' still one element"
    );
    assert!(
        line.contains("\"web 01\"") || line.contains("'web 01'"),
        "a spaced argument must render as one quoted argument, line was: {line:?}"
    );
}

#[test]
fn the_confirmation_overlay_draws_the_label_and_the_quoted_argv() {
    let buffer = render_buffer(&session_confirming(false), WIDE);
    write_frame(
        "plugin-confirmation",
        "run plugin",
        "The confirmation the palette opens: the label, and the argv that will run.",
        &buffer,
    );
    let lines = lines_of(&buffer);

    assert!(
        lines.iter().any(|line| line.contains("run plugin")),
        "the modal's own title identifies it as the thing that runs: {lines:#?}"
    );
    assert!(
        lines.iter().any(|line| line.contains("Describe web 01")),
        "the plugin's label is the heading: {lines:#?}"
    );
    assert!(
        lines.iter().any(|line| line
            .contains("gcloud --configuration dev compute instances describe")
            && (line.contains("\"web 01\"") || line.contains("'web 01'"))),
        "the drawn line is the argv, with the spaced argument quoted: {lines:#?}"
    );
}

#[test]
fn an_ordinary_plugin_is_never_told_to_type_a_digit_that_does_nothing() {
    // The keymap's digit and `Backspace` arms are reached for every plugin, but
    // a `Simple` ack ignores them. `ARCHITECTURE.md` requires footer and help
    // text to match real key behaviour, so the modal names neither here.
    let screen = render_lines(&session_confirming(false), WIDE);
    let lines = modal_lines(&screen);

    assert!(
        lines.iter().any(|line| line.contains("Enter run")),
        "an ordinary plugin runs on Enter alone: {lines:#?}"
    );
    // Scoped to the modal rather than the screen: the overlay clears what is
    // behind it today, so a screen-wide negative would pass for that reason
    // rather than for its own, and would stop being a claim about this modal
    // the first time anything else on the frame said "Backspace".
    assert!(
        !lines.iter().any(|line| line.contains("Type 1 to confirm")),
        "nothing has to be typed for an ordinary plugin: {lines:#?}"
    );
    assert!(
        !lines.iter().any(|line| line.contains("Backspace")),
        "Backspace edits an ack an ordinary plugin does not have: {lines:#?}"
    );
}

#[test]
fn a_dangerous_plugin_offers_enter_only_once_the_typed_count_satisfies_the_ack() {
    let mut session = session_confirming(true);

    let before_buffer = render_buffer(&session, WIDE);
    write_frame(
        "dangerous-before-ack",
        "run plugin (dangerous = true)",
        "Enter is not offered: the operator has to type 1 first.",
        &before_buffer,
    );
    let before = modal_lines(&lines_of(&before_buffer));
    assert!(
        before.iter().any(|line| line.contains("Type 1 to confirm")),
        "a dangerous plugin asks for the count first: {before:#?}"
    );
    assert!(
        !before.iter().any(|line| line.contains("Enter run")),
        "Enter is guarded on the ack, so the modal must not offer it yet: {before:#?}"
    );

    session
        .app_mut()
        .root
        .resources
        .plugin_ack_append_digit('1');

    let after_buffer = render_buffer(&session, WIDE);
    write_frame(
        "dangerous-after-ack",
        "run plugin (dangerous = true, 1 typed)",
        "The typed count is shown back and Enter is armed.",
        &after_buffer,
    );
    let after = modal_lines(&lines_of(&after_buffer));
    assert!(
        after
            .iter()
            .any(|line| line.contains("Type 1 to confirm: 1")),
        "what was typed is shown back: {after:#?}"
    );
    assert!(
        after.iter().any(|line| line.contains("Enter run")),
        "the satisfied ack arms Enter, and the modal says so: {after:#?}"
    );
}

/// The `dangerous` guard composed out of its real parts: a real
/// `PluginConfirmation` carrying a `CountRequired` ack, the runtime's own
/// context projection, and the real keymap.
///
/// Both halves exist in isolation and neither joins them.
/// `keymap_test::plugin_confirmation_enter_waits_for_the_acknowledgement` sets
/// `plugin_confirmation_ack_satisfied` by hand, so it proves what the keymap
/// does with the flag but nothing about where the flag comes from;
/// `the_ack_keys_edit_a_dangerous_plugins_typed_count_through_the_reducer`
/// stops at the state. Every test that presses a real `Enter` on a
/// confirmation — `pressing_enter_on_the_confirmation_runs_the_plugin_once_
/// through_the_real_keymap`, which is the ordinary half of this claim and is
/// why no `Simple` case is repeated here — uses an ordinary plugin, whose ack
/// is satisfied from the start. So a projection that answered `true` for every
/// open confirmation (`is_some()` where it now calls `is_ack_satisfied()`)
/// would run a dangerous plugin on a bare `Enter` with the whole suite green.
#[test]
fn a_dangerous_plugin_cannot_be_confirmed_by_a_keypress_alone() {
    let mut session = session_confirming(true);
    let area = Rect::new(0, 0, WIDE.0, WIDE.1);

    let before = input_context_for_test(&session, area);
    assert_eq!(
        before.mode,
        Mode::PluginConfirmation,
        "the open confirmation owns the keys"
    );
    // Asserted so the `None` below is attributable: the modal's other Enter
    // guard is readability, and at this size the modal draws in full, so the
    // acknowledgement is the only thing left holding the key back.
    assert!(
        before.plugin_confirmation_visible,
        "the modal fits at {WIDE:?}, so visibility is not what blocks Enter"
    );
    assert!(
        !before.plugin_confirmation_ack_satisfied,
        "a dangerous plugin's ack starts unsatisfied, and the projection has to say so"
    );
    assert_eq!(
        action_for_key(enter_key(), &before),
        None,
        "Enter must not even dispatch until the acknowledgement is typed"
    );

    session
        .app_mut()
        .root
        .resources
        .plugin_ack_append_digit('1');

    let after = input_context_for_test(&session, area);
    assert!(
        after.plugin_confirmation_ack_satisfied,
        "the typed count has to reach the context the keymap reads"
    );
    assert_eq!(
        action_for_key(enter_key(), &after),
        Some(Action::Resources(ResourcesAction::ConfirmPlugin)),
        "once acknowledged, Enter runs it"
    );
}

fn enter_key() -> KeyEvent {
    KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)
}

/// Records the argv of every command it is handed, so a test can compare what
/// the child was asked to run against what the modal showed. It answers every
/// command with empty stdout: this file asserts on the argv, not on parsing.
#[derive(Clone, Default)]
struct RecordingRunner {
    argv: Arc<Mutex<Vec<Vec<String>>>>,
}

impl CommandRunner for RecordingRunner {
    fn run(&self, command: GcloudCommand) -> Result<CommandOutput, CommandError> {
        // ARCHITECTURE.md's stronger claim: only nuvem's own services can
        // select the runner's program, never a plugin declaration. This
        // runner is installed as the session's, so plugin-declared argv and
        // nuvem's own service commands both reach it, and every one of them
        // is a gcloud command; a declaration that ever grew a binary would
        // land here as `Bq` and fail loudly.
        assert_eq!(
            command.program,
            CommandProgram::Gcloud,
            "a plugin declaration selected a program other than gcloud: {:?}",
            command.args
        );
        self.argv.lock().unwrap().push(command.args.clone());
        Ok(CommandOutput {
            stdout: String::new(),
            stderr: String::new(),
        })
    }
}

/// Holds a command until the test lets it go, standing in for a plugin that
/// takes a long time — or never returns at all. `held` names the argv element
/// that decides which commands are held; every other command answers at once,
/// which is what lets a test hold one of the two commands in flight and settle
/// the other.
#[derive(Clone)]
struct BlockingRunner {
    held: String,
    fail_held: bool,
    released: Arc<(Mutex<bool>, Condvar)>,
}

impl BlockingRunner {
    fn holding(held: &str) -> Self {
        Self {
            held: held.to_string(),
            fail_held: false,
            released: Arc::new((Mutex::new(false), Condvar::new())),
        }
    }

    /// Holds the named command and then answers it with a failure, so a test
    /// can assert on the status line the held command writes when it lands.
    fn holding_then_failing(held: &str) -> Self {
        Self {
            fail_held: true,
            ..Self::holding(held)
        }
    }

    fn release(&self) {
        let (lock, condvar) = &*self.released;
        *lock.lock().unwrap() = true;
        condvar.notify_all();
    }
}

impl CommandRunner for BlockingRunner {
    fn run(&self, command: GcloudCommand) -> Result<CommandOutput, CommandError> {
        if !command.args.contains(&self.held) {
            return Ok(CommandOutput {
                // An empty JSON list, so a resource refresh answered this way
                // parses instead of failing for a reason the test never meant.
                stdout: "[]".to_string(),
                stderr: String::new(),
            });
        }
        let (lock, condvar) = &*self.released;
        let mut released = lock.lock().unwrap();
        while !*released {
            // Bounded, because this runs on a `spawn_blocking` thread where an
            // unbounded park is uncancellable and would hang the whole suite
            // rather than fail this test. The gate is released microseconds
            // later on the run loop, so exceeding this proves it wedged rather
            // than that the machine was slow — and the bound sits under
            // nextest's own per-test terminate timeout so the wedge is reported
            // as this assertion rather than as an anonymous timeout.
            let (guard, timeout) = condvar
                .wait_timeout(released, Duration::from_secs(5))
                .unwrap();
            released = guard;
            assert!(!timeout.timed_out(), "the plugin gate was never released");
        }
        if self.fail_held {
            return Err(CommandError::Failed {
                stdout: String::new(),
                stderr: "ERROR: (gcloud.compute.instances.list) permission denied".to_string(),
                exit_code: Some(1),
            });
        }
        Ok(CommandOutput {
            stdout: "[]".to_string(),
            stderr: String::new(),
        })
    }
}

/// Fails every command, the way a plugin whose gcloud invocation exits non-zero
/// does.
struct FailingRunner;

impl CommandRunner for FailingRunner {
    fn run(&self, _command: GcloudCommand) -> Result<CommandOutput, CommandError> {
        Err(CommandError::Failed {
            stdout: String::new(),
            stderr: "ERROR: (gcloud.compute.instances.describe) not found".to_string(),
            exit_code: Some(1),
        })
    }
}

fn configuration_fixture() -> GcloudConfiguration {
    GcloudConfiguration {
        name: GcloudConfigurationName::new("dev"),
        is_active: true,
        account: None,
        project: Some("dev-project".to_string()),
        region: Some("europe-west1".to_string()),
        zone: Some("europe-west1-b".to_string()),
    }
}

/// A session with an active context and its compute rows already loaded, so the
/// only gcloud command it has any reason to run is the plugin's, and so the
/// plugin's result is the only one queued. Returns the context the run must be
/// confirmed under: a result carrying any other one is not accepted, which is
/// the guard `effect_result_is_accepted` applies.
async fn session_at_a_loaded_context(
    runner: Option<Box<dyn CommandRunner + Send + Sync>>,
) -> (RuntimeSession, GcloudContext) {
    let mut session = match runner {
        Some(runner) => RuntimeSession::new_with_runner_for_test(AppState::new(), runner),
        None => RuntimeSession::new_for_test(AppState::new()),
    };
    session
        .apply_effect_result(EffectResult::ConfigurationsLoaded {
            configurations: vec![configuration_fixture()],
        })
        .await;
    let context = session
        .app()
        .root
        .current_context()
        .expect("the loaded configuration is the active context")
        .clone();
    session
        .apply_effect_result(EffectResult::ComputeInstancesLoaded {
            context: context.clone(),
            instances: Vec::new(),
        })
        .await;
    (session, context)
}

#[tokio::test]
async fn the_child_receives_the_plugins_argv_element_for_element_including_the_spaced_one() {
    // The same spaced argument the renderer test uses, for the same reason: it
    // is the fixture that catches argv being flattened through anything
    // shell-shaped on the way to the child. Joined and re-split, "web 01"
    // becomes two arguments and this assertion is the only one that notices.
    let runner = RecordingRunner::default();
    let argv = plugin_fixture_argv();
    let (mut session, context) = session_at_a_loaded_context(Some(Box::new(runner.clone()))).await;

    session
        .dispatch_action(Action::Effect(Effect::RunPlugin {
            context,
            label: "Describe web 01".to_string(),
            argv: argv.clone(),
            operation_id: 7,
        }))
        .await;
    // The run is on a blocking thread now, so nothing has been handed to the
    // runner yet. Its result reaching the queue is what proves it has.
    assert!(session.process_next_effect_result().await);

    assert_eq!(
        runner.argv.lock().unwrap().as_slice(),
        &[argv],
        "the child's argv must equal the modal's argv, with 'web 01' still one element"
    );
}

#[tokio::test]
async fn a_plugin_run_does_not_block_the_run_loop_on_the_child() {
    // The child is held until the test releases it. If the run were inline,
    // `dispatch_action` could not return and this test would hang rather than
    // fail — which is precisely what a plugin that never returns would do to
    // the TUI.
    let runner = BlockingRunner::holding("describe");
    let (mut session, context) = session_at_a_loaded_context(Some(Box::new(runner.clone()))).await;

    session
        .dispatch_action(Action::Effect(Effect::RunPlugin {
            context,
            label: "Describe web 01".to_string(),
            argv: plugin_fixture_argv(),
            operation_id: 7,
        }))
        .await;

    assert_eq!(
        session.app().root.system.operation.phase,
        OperationPhase::Refreshing,
        "the run loop is back with the plugin still running"
    );
    assert_eq!(session.app().root.system.operation.label, "plugin");

    runner.release();
    assert!(session.process_next_effect_result().await);
    assert_eq!(
        session.app().root.system.operation.phase,
        OperationPhase::Healthy
    );
}

#[tokio::test]
async fn a_run_confirmed_under_a_context_the_operator_has_left_does_not_complete_the_new_one() {
    // The run outlives the context it was confirmed under, because it is on a
    // blocking thread. Its result must still arrive — it is the only thing that
    // can settle the operations-log entry the run seeded — but it must not
    // report the old context's outcome against the operation of the context the
    // operator is looking at now.
    let runner = RecordingRunner::default();
    let (mut session, _) = session_at_a_loaded_context(Some(Box::new(runner.clone()))).await;
    let abandoned = GcloudContext {
        configuration: GcloudConfigurationName::new("prod"),
        account: None,
        project: Some("prod-project".to_string()),
        region: None,
        zone: None,
    };

    session
        .dispatch_action(Action::Effect(Effect::RunPlugin {
            context: abandoned,
            label: "Describe web 01".to_string(),
            argv: plugin_fixture_argv(),
            operation_id: 7,
        }))
        .await;
    assert!(session.process_next_effect_result().await);

    assert_eq!(
        session.app().root.system.operation.phase,
        OperationPhase::Refreshing,
        "a result from the context the operator left must not complete this one's operation"
    );
}

#[tokio::test]
async fn a_plugin_result_does_not_take_over_an_operation_the_operator_started_after_it() {
    // The context is unchanged here, so the context guard passes and cannot be
    // what saves this. What the operator did was start something *else* in the
    // same context — the cross-batch supersession `operation_generation` exists
    // for — and the plugin's result must not report itself over the refresh
    // they are actually waiting on.
    //
    // Only the refresh's own command is held, so it is genuinely in flight when
    // the plugin's result lands, and the plugin's is the only result queued:
    // the ordering here is arranged, not raced.
    let runner = BlockingRunner::holding("list");
    let (mut session, context) = session_at_a_loaded_context(Some(Box::new(runner.clone()))).await;

    session
        .dispatch_action(Action::Effect(Effect::RunPlugin {
            context: context.clone(),
            label: "Describe web 01".to_string(),
            argv: plugin_fixture_argv(),
            operation_id: 7,
        }))
        .await;
    session
        .dispatch_action(Action::Effect(Effect::RefreshComputeInstances {
            context,
            filter: None,
        }))
        .await;
    assert_eq!(
        session.app().root.system.operation.label,
        "GCE instances",
        "the refresh is the operation the status line is reporting"
    );

    assert!(session.process_next_effect_result().await);

    assert_eq!(
        session.app().root.system.operation.label,
        "GCE instances",
        "a superseded plugin run must not rename the in-flight operation"
    );
    assert_eq!(
        session.app().root.system.operation.phase,
        OperationPhase::Refreshing,
        "nor complete it: the refresh has not come back yet"
    );
    // The `operation` fields above are what the runtime's generation gate
    // protects. The status *message* is written by the reducer, which is handed
    // the result unconditionally and sees the context rather than the
    // generation, so it is a second surface reporting the same supersession and
    // has to be asserted separately — the two assertions above stayed green
    // while the line under them read "Describe web 01 finished".
    assert_eq!(
        session.app().root.system.status_message,
        None,
        "a superseded plugin run must not narrate itself over the refresh either"
    );

    // Let the held refresh finish, so no blocking thread is left parked on the
    // gate after the test returns.
    runner.release();
    assert!(session.process_next_effect_result().await);
}

#[tokio::test]
async fn rolling_back_a_superseded_plugins_status_leaves_the_refresh_free_to_write_its_own() {
    // The rollback has to be invisible to the *next* writer too. The refresh
    // captured the status generation when it was dispatched; if the superseded
    // plugin's reduce leaves its bump behind, the refresh reads a generation it
    // does not recognise, concludes something newer owns the status line, and
    // rolls back its own failure — so the operator waits on a refresh that has
    // already failed and never says so.
    let runner = BlockingRunner::holding_then_failing("list");
    let (mut session, context) = session_at_a_loaded_context(Some(Box::new(runner.clone()))).await;

    session
        .dispatch_action(Action::Effect(Effect::RunPlugin {
            context: context.clone(),
            label: "Describe web 01".to_string(),
            argv: plugin_fixture_argv(),
            operation_id: 7,
        }))
        .await;
    session
        .dispatch_action(Action::Effect(Effect::RefreshComputeInstances {
            context,
            filter: None,
        }))
        .await;

    // The superseded plugin's result, rolled back.
    assert!(session.process_next_effect_result().await);
    assert_eq!(
        session.app().root.system.status_message,
        None,
        "the superseded plugin must not have written the status line"
    );

    runner.release();
    assert!(session.process_next_effect_result().await);

    let status = session
        .app()
        .root
        .system
        .status_message
        .clone()
        .expect("the refresh that failed must say so on the status line");
    assert!(
        status.contains("permission denied"),
        "the refresh's own failure must reach the status line, got {status:?}"
    );
    assert_eq!(
        session.app().root.system.status_operation_label.as_deref(),
        Some("GCE instances"),
        "and must be attributed to the refresh the operator was waiting on"
    );
}

#[tokio::test]
async fn a_session_with_no_gcloud_runner_reports_the_plugin_as_failed() {
    let (mut session, context) = session_at_a_loaded_context(None).await;

    session
        .dispatch_action(Action::Effect(Effect::RunPlugin {
            context,
            label: "Describe web 01".to_string(),
            argv: plugin_fixture_argv(),
            operation_id: 7,
        }))
        .await;
    assert!(session.process_next_effect_result().await);

    assert_eq!(
        session.app().root.system.operation.phase,
        OperationPhase::Failed,
        "a run that could not start is a failed run, not a silent one"
    );
    assert_eq!(session.app().root.system.operation.label, "plugin");
}

#[tokio::test]
async fn a_plugin_whose_gcloud_command_fails_is_reported_as_failed() {
    let (mut session, context) = session_at_a_loaded_context(Some(Box::new(FailingRunner))).await;

    session
        .dispatch_action(Action::Effect(Effect::RunPlugin {
            context,
            label: "Describe web 01".to_string(),
            argv: plugin_fixture_argv(),
            operation_id: 7,
        }))
        .await;
    assert!(session.process_next_effect_result().await);

    assert_eq!(
        session.app().root.system.operation.phase,
        OperationPhase::Failed
    );
    assert_eq!(session.app().root.system.operation.label, "plugin");
}

/// An app whose active context resolves and whose plugin confirmation is open,
/// which is the state the palette leaves behind (pinned by
/// `submitting_a_plugin_from_the_palette_opens_the_confirmation_not_the_copy_only_preview`
/// above). The confirmation is seeded directly here rather than driven through
/// the palette because these tests are about what answering it does, and the
/// palette walk would only re-assert the step the test above already owns.
fn app_confirming_a_plugin(dangerous: bool) -> (AppState, GcloudContext) {
    let mut app = AppState::new();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ConfigurationsLoaded {
            configurations: vec![configuration_fixture()],
        }),
    );
    let context = app
        .root
        .current_context()
        .expect("the loaded configuration is the active context")
        .clone();
    app.root
        .resources
        .show_plugin_confirmation(plugin_label(), plugin_fixture_argv(), dangerous);
    (app, context)
}

fn record_for(app: &AppState, operation_id: u64) -> OperationRecord {
    app.root
        .system
        .operations
        .records()
        .iter()
        .find(|record| record.id == operation_id)
        .expect("the entry the confirmation seeded must still be in the log")
        .clone()
}

#[test]
fn confirming_a_plugin_seeds_a_pending_operation_that_its_result_settles_with_the_output() {
    let (mut app, context) = app_confirming_a_plugin(false);

    let effects = reduce(&mut app, &Action::Resources(ResourcesAction::ConfirmPlugin));

    let Some(Effect::RunPlugin { operation_id, .. }) = effects.first() else {
        panic!("confirming must emit exactly one RunPlugin, got: {effects:?}");
    };
    let operation_id = *operation_id;
    assert_eq!(effects.len(), 1, "one confirm is one run: {effects:?}");
    assert_eq!(app.root.system.operations.pending_count(), 1);

    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::PluginRunSucceeded {
            context,
            label: "Describe web 01".to_string(),
            output: "status: RUNNING".to_string(),
            stderr: String::new(),
            operation_id,
        }),
    );

    let record = record_for(&app, operation_id);
    assert_eq!(record.outcome, OperationOutcome::Succeeded);
    assert!(
        record
            .detail
            .as_deref()
            .unwrap_or_default()
            .contains("RUNNING"),
        "the run's output must reach the record's detail, detail was: {:?}",
        record.detail
    );
}

#[test]
fn confirming_twice_runs_the_plugin_once() {
    // The confirmation is taken, not borrowed, so a second Enter finds nothing.
    let (mut app, _) = app_confirming_a_plugin(false);

    let first = reduce(&mut app, &Action::Resources(ResourcesAction::ConfirmPlugin));
    let second = reduce(&mut app, &Action::Resources(ResourcesAction::ConfirmPlugin));

    assert_eq!(first.len(), 1, "the first confirm runs it");
    assert!(
        second.is_empty(),
        "the second must do nothing, got: {second:?}"
    );
    assert_eq!(
        app.root.system.operations.pending_count(),
        1,
        "a second confirm must not seed a second entry either"
    );
}

#[test]
fn the_effect_carries_the_confirmed_argv_and_the_active_context() {
    let (mut app, context) = app_confirming_a_plugin(false);

    let effects = reduce(&mut app, &Action::Resources(ResourcesAction::ConfirmPlugin));

    let Some(Effect::RunPlugin {
        context: run_context,
        label,
        argv,
        ..
    }) = effects.first()
    else {
        panic!("confirming must emit exactly one RunPlugin, got: {effects:?}");
    };
    assert_eq!(*run_context, context);
    assert_eq!(label, "Describe web 01");
    assert_eq!(
        argv.as_slice(),
        plugin_fixture_argv().as_slice(),
        "the argv that runs must be the argv the modal showed, element for element"
    );
    assert!(
        app.root.resources.plugin_confirmation().is_none(),
        "confirming closes the modal"
    );
}

#[test]
fn a_plugins_output_reaches_the_log_sanitized_bounded_and_on_one_line() {
    // gcloud stdout is whatever the operator's own command printed: many lines,
    // no length bound, and credential-shaped values if the command asked for
    // them. The operations card renders `detail` as a single line, so all three
    // are rendering defects as well as a leak.
    let (mut app, context) = app_confirming_a_plugin(false);
    let effects = reduce(&mut app, &Action::Resources(ResourcesAction::ConfirmPlugin));
    let Some(Effect::RunPlugin { operation_id, .. }) = effects.first() else {
        panic!("confirming must emit exactly one RunPlugin, got: {effects:?}");
    };
    let operation_id = *operation_id;

    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::PluginRunSucceeded {
            context,
            label: "Describe web 01".to_string(),
            output: format!(
                "status: RUNNING\naccess_token: ya29.abcdef\n{}",
                "x".repeat(4096)
            ),
            stderr: String::new(),
            operation_id,
        }),
    );

    let detail = record_for(&app, operation_id)
        .detail
        .expect("output must reach the record");
    assert!(
        detail.len() <= DETAIL_MAX_LEN,
        "the detail must be bounded, len was {}: {detail:?}",
        detail.len()
    );
    assert!(
        !detail.contains('\n') && !detail.contains('\r'),
        "a one-line record cannot carry a multi-line detail: {detail:?}"
    );
    assert!(
        !detail.contains("ya29."),
        "a credential-shaped value must be redacted before it is rendered: {detail:?}"
    );
    assert!(
        detail.contains("RUNNING"),
        "the head of the output must survive, so the record still says something: {detail:?}"
    );
}

#[test]
fn a_failed_plugin_settles_as_failed_and_the_record_says_why() {
    let (mut app, context) = app_confirming_a_plugin(false);
    let effects = reduce(&mut app, &Action::Resources(ResourcesAction::ConfirmPlugin));
    let Some(Effect::RunPlugin { operation_id, .. }) = effects.first() else {
        panic!("confirming must emit exactly one RunPlugin, got: {effects:?}");
    };
    let operation_id = *operation_id;

    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::PluginRunFailed {
            context,
            label: "Describe web 01".to_string(),
            error: AppError::Gcloud(GcloudFailure {
                kind_hint: None,
                // Long enough that the sanitized preview is already at
                // DETAIL_MAX_LEN, so anything composed onto it has to be
                // re-bounded rather than assumed to fit.
                detail: format!(
                    "ERROR: (gcloud.compute.instances.describe) PERMISSION_DENIED {}",
                    "x".repeat(4096)
                ),
                exit_code: Some(1),
                command_summary: Some("gcloud compute instances describe".to_string()),
                missing_binary: None,
            }),
            stderr: String::new(),
            operation_id,
        }),
    );

    let record = record_for(&app, operation_id);
    assert_eq!(record.outcome, OperationOutcome::Failed);
    let detail = record.detail.expect("a failure must say why");
    assert!(
        detail.contains("permission denied"),
        "the classified reason must reach the record: {detail:?}"
    );
    assert!(
        detail.contains("PERMISSION_DENIED"),
        "so must gcloud's own words, or the operator cannot tell two failures apart: {detail:?}"
    );
    assert!(
        detail.len() <= DETAIL_MAX_LEN,
        "the failure detail is held to the same single bound as a success's, \
         len was {}: {detail:?}",
        detail.len()
    );
}

#[test]
fn a_result_from_a_context_the_operator_left_still_settles_its_entry() {
    // The run outlives its context, so the entry it seeded would be stuck at
    // Pending forever if settling were gated on the context still matching.
    // What the stale result must NOT do is talk over the status line of the
    // context the operator is looking at now.
    let (mut app, _) = app_confirming_a_plugin(false);
    let effects = reduce(&mut app, &Action::Resources(ResourcesAction::ConfirmPlugin));
    let Some(Effect::RunPlugin { operation_id, .. }) = effects.first() else {
        panic!("confirming must emit exactly one RunPlugin, got: {effects:?}");
    };
    let operation_id = *operation_id;
    app.root
        .system
        .set_status_message("the operator's own screen".to_string());

    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::PluginRunSucceeded {
            context: GcloudContext {
                configuration: GcloudConfigurationName::new("prod"),
                account: None,
                project: Some("prod-project".to_string()),
                region: None,
                zone: None,
            },
            label: "Describe web 01".to_string(),
            output: "status: RUNNING".to_string(),
            stderr: String::new(),
            operation_id,
        }),
    );

    assert_eq!(
        record_for(&app, operation_id).outcome,
        OperationOutcome::Succeeded
    );
    assert_eq!(
        app.root.system.status_message.as_deref(),
        Some("the operator's own screen"),
        "a run from a context the operator left must not report itself over this one"
    );
}

/// The failure arm's half of the claim above. Written separately rather than
/// folded in because the two arms gate the status line independently: the
/// success arm's gate is pinned by the test above and by Task 5's supersession
/// test, and neither of those touches this one — a failure reporting itself
/// over a context the operator has left is the louder of the two, since it
/// names a problem in a configuration they are not looking at.
#[test]
fn a_failure_from_a_context_the_operator_left_settles_without_taking_the_status_line() {
    let (mut app, _) = app_confirming_a_plugin(false);
    let effects = reduce(&mut app, &Action::Resources(ResourcesAction::ConfirmPlugin));
    let Some(Effect::RunPlugin { operation_id, .. }) = effects.first() else {
        panic!("confirming must emit exactly one RunPlugin, got: {effects:?}");
    };
    let operation_id = *operation_id;
    app.root
        .system
        .set_status_message("the operator's own screen".to_string());

    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::PluginRunFailed {
            context: GcloudContext {
                configuration: GcloudConfigurationName::new("prod"),
                account: None,
                project: Some("prod-project".to_string()),
                region: None,
                zone: None,
            },
            label: "Describe web 01".to_string(),
            error: AppError::Gcloud(GcloudFailure {
                kind_hint: None,
                detail: "ERROR: (gcloud.compute.instances.describe) PERMISSION_DENIED".to_string(),
                exit_code: Some(1),
                command_summary: Some("Describe web 01".to_string()),
                missing_binary: None,
            }),
            stderr: String::new(),
            operation_id,
        }),
    );

    assert_eq!(
        record_for(&app, operation_id).outcome,
        OperationOutcome::Failed,
        "the entry must settle however stale the result is"
    );
    assert_eq!(
        app.root.system.status_message.as_deref(),
        Some("the operator's own screen"),
        "a failure from a context the operator left must not report itself over this one"
    );
    // The other half, asserted here so the gate above cannot be "fixed" by
    // widening it: recording the diagnostic is deliberately *not* gated on the
    // context, because the copy-diagnostic key must still hand over the failure
    // of a run the operator themselves started.
    assert_eq!(
        app.root
            .system
            .last_diagnostic()
            .map(|diagnostic| diagnostic.label()),
        Some(std::borrow::Cow::Borrowed("permission denied")),
        "the failure must still be copyable after a context switch"
    );
}

#[test]
fn a_hostile_label_is_display_safe_before_it_is_stored_in_the_log() {
    // A label is a template rendered against the selected row and is exempt
    // from the argv refusals, so a row named with a newline reaches it verbatim.
    // The palette and the modal each sanitize at their own render site; the log
    // entry and the status line read what the confirm arm stored.
    let (mut app, context) = app_confirming_a_plugin(false);
    app.root.resources.show_plugin_confirmation(
        "Describe web-01\u{1b}[2Jwiped".to_string(),
        plugin_fixture_argv(),
        false,
    );

    let effects = reduce(&mut app, &Action::Resources(ResourcesAction::ConfirmPlugin));
    let Some(Effect::RunPlugin {
        label,
        operation_id,
        ..
    }) = effects.first()
    else {
        panic!("confirming must emit exactly one RunPlugin, got: {effects:?}");
    };
    // The label the effect carries is the one the runtime echoes back into the
    // result, and from there into the status line, so it has to be safe too.
    let label = label.clone();
    let operation_id = *operation_id;

    let record = record_for(&app, operation_id);
    assert!(
        !record.target.contains('\u{1b}'),
        "an escape byte must not be stored as the entry's target: {:?}",
        record.target
    );
    assert!(
        record.target.contains("Describe web-01"),
        "the operator must still recognise their own plugin: {:?}",
        record.target
    );

    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::PluginRunSucceeded {
            context,
            label,
            output: String::new(),
            stderr: String::new(),
            operation_id,
        }),
    );
    let message = app
        .root
        .system
        .status_message
        .as_deref()
        .unwrap_or_default()
        .to_string();
    assert!(
        !message.contains('\u{1b}'),
        "nor written into the status line: {message:?}"
    );
}

#[test]
fn confirming_with_no_active_configuration_says_so_instead_of_dropping_the_keypress() {
    // Unreachable from the palette — a row-scoped plugin needs rows, and rows
    // need a context — which is why it is pinned here: a confirmed Enter that
    // does nothing and says nothing is indistinguishable from a dead key.
    let mut app = AppState::new();
    app.root
        .resources
        .show_plugin_confirmation(plugin_label(), plugin_fixture_argv(), false);

    let effects = reduce(&mut app, &Action::Resources(ResourcesAction::ConfirmPlugin));

    assert!(
        effects.is_empty(),
        "there is nothing to run it under: {effects:?}"
    );
    assert_eq!(
        app.root.system.operations.pending_count(),
        0,
        "no entry may be seeded for a run that never started"
    );
    let message = app
        .root
        .system
        .status_message
        .as_deref()
        .expect("the operator must be told why nothing ran");
    assert!(
        message.contains("Describe web 01") && message.contains("configuration"),
        "the message must name the plugin and the reason: {message:?}"
    );
}

#[test]
fn cancelling_the_confirmation_closes_it_and_runs_nothing() {
    let (mut app, _) = app_confirming_a_plugin(false);

    let effects = reduce(
        &mut app,
        &Action::Resources(ResourcesAction::CancelPluginConfirmation),
    );

    assert!(effects.is_empty(), "cancelling runs nothing: {effects:?}");
    assert!(app.root.resources.plugin_confirmation().is_none());
    assert_eq!(
        app.root.system.operations.pending_count(),
        0,
        "a cancelled confirmation seeds no entry to settle"
    );
}

#[test]
fn the_ack_keys_edit_a_dangerous_plugins_typed_count_through_the_reducer() {
    let (mut app, _) = app_confirming_a_plugin(true);

    reduce(
        &mut app,
        &Action::Resources(ResourcesAction::PluginAckAppendDigit('1')),
    );
    assert!(
        app.root
            .resources
            .plugin_confirmation()
            .expect("the modal is still open")
            .is_ack_satisfied(),
        "the digit the operator typed must reach the ack"
    );

    reduce(
        &mut app,
        &Action::Resources(ResourcesAction::PluginAckBackspace),
    );
    assert!(
        !app.root
            .resources
            .plugin_confirmation()
            .expect("the modal is still open")
            .is_ack_satisfied(),
        "backspace must un-arm it again"
    );
}

#[test]
fn a_terminal_too_small_for_the_modal_falls_back_to_a_message() {
    // 60x12 is the app's own minimum supported size, and it is where
    // `ui_projection_test::small_terminal_overlays_preserve_fallback_messages_at_60x12`
    // checks the other overlays' fallbacks.
    //
    // The argv has to be long to reach the fallback at all now: the
    // modal wraps rather than demanding each line fit one row, so a short
    // one draws here and there would be no fallback to evidence.
    let buffer = render_buffer(&session_confirming_a_long_argv(), (60, 12));
    write_frame(
        "small-terminal",
        "run plugin at 60x12",
        "Too small to draw the argv, so the modal says so and only cancels.",
        &buffer,
    );
    let lines = lines_of(&buffer);

    assert!(
        lines
            .iter()
            .any(|line| line.contains("nuvem plugin confirmation needs a larger terminal")),
        "the fallback says why the argv is not shown: {lines:#?}"
    );
}

/// The last double in the chain removed: a *real* child process.
///
/// Every other test here answers the plugin's command from a `CommandRunner`
/// implemented in this file, so "the plugin runs" is asserted against a trait
/// object. This one hands the confirmation to the production
/// `ProcessCommandRunner` pointed at a fixture script instead of the real
/// `gcloud`, so an actual `fork`/`exec` happens: the argv is compared against
/// what the child itself wrote to disk, one argument per line, and the entry in
/// the operations log carries the bytes that child printed.
#[tokio::test]
async fn a_real_child_runs_the_confirmed_argv_and_its_output_reaches_the_operations_log() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::tempdir().expect("a temp dir for the fixture gcloud");
    let recorded = dir.path().join("argv.txt");
    let script = dir.path().join("gcloud");
    std::fs::write(
        &script,
        format!(
            "#!/bin/sh\nfor arg in \"$@\"; do printf '%s\\n' \"$arg\" >> {recorded}; done\n\
             printf 'status: RUNNING\\n'\n",
            recorded = recorded.display()
        ),
    )
    .expect("the fixture script is written");
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755))
        .expect("the fixture script is executable");

    let (app, _) = app_confirming_a_plugin(false);
    let mut session = RuntimeSession::new_with_runner_for_test(
        app,
        Box::new(nuvem::services::gcloud::cli::ProcessCommandRunner::new(
            script.clone(),
        )),
    );

    // The operator's own keypress, resolved through the real input context and
    // keymap rather than dispatched by name.
    let context = input_context_for_test(&session, Rect::new(0, 0, WIDE.0, WIDE.1));
    let action = action_for_key(enter_key(), &context).expect("Enter runs an armed confirmation");
    session.dispatch_action(action).await;

    let mut drained = 0;
    while session.app().root.system.operations.pending_count() > 0 {
        assert!(drained < 8, "the plugin's entry was still pending");
        assert!(
            session.process_next_effect_result().await,
            "the result channel closed with the plugin's entry still pending"
        );
        drained += 1;
    }

    let argv = std::fs::read_to_string(&recorded).expect("the child must have run and recorded");
    assert_eq!(
        argv.lines().collect::<Vec<_>>(),
        plugin_fixture_argv(),
        "the real child's argv must be the modal's argv, with 'web 01' still one argument"
    );

    let record = session
        .app()
        .root
        .system
        .operations
        .records()
        .iter()
        .find(|record| record.kind == "plugin")
        .expect("the confirm seeded an entry")
        .clone();
    assert_eq!(record.outcome, OperationOutcome::Succeeded);
    assert!(
        record
            .detail
            .as_deref()
            .unwrap_or_default()
            .contains("RUNNING"),
        "the child's own stdout must reach the log, detail was: {:?}",
        record.detail
    );

    if let Ok(evidence) = std::env::var("NUVEM_EVIDENCE_DIR") {
        let buffer = render_buffer(&session, WIDE);
        write_frame(
            "after-the-run",
            "after Enter",
            "The modal is gone and the run is in Recent operations.",
            &buffer,
        );
        std::fs::write(
            std::path::PathBuf::from(evidence).join("real-child-transcript.txt"),
            format!(
                "$ gcloud (fixture) argv, one argument per line, as written by the child:\n{argv}\n\
                 operations log entry: kind={} outcome={:?} detail={:?}\n",
                record.kind, record.outcome, record.detail
            ),
        )
        .expect("write transcript");
    }
}
