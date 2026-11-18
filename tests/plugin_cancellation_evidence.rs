//! Evidence: the operator can cancel a plugin that never returns, and
//! the operations-log entry it leaves behind says something true about whether
//! the command was ever issued.
//!
//! Every wait here is a bounded gate released explicitly by the test — no
//! sleeps and no unbounded parks, which on a `spawn_blocking` thread are
//! uncancellable and wedge `Runtime::drop` for good.

use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyModifiers};
use nuvem::app::state::operations::OperationOutcome;
use nuvem::app::{Action, AppState, SystemAction, WorkspaceAction};
use nuvem::services::gcloud::cli::{
    CommandCeiling, CommandError, CommandOutput, CommandRunner, GcloudCommand,
};
use nuvem::services::runtime::session::RuntimeSession;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use tokio_util::sync::CancellationToken;

use crate::plugin_preview_evidence::Operator;
use crate::support::frame_svg::buffer_to_svg;

/// Deadline for every gate wait. The test releases its gate within
/// milliseconds, so 30s sits orders of magnitude above any plausible
/// scheduling delay: exceeding it proves the gate wedged, not that the machine
/// was slow.
const GATE_TIMEOUT: Duration = Duration::from_secs(30);

#[tokio::test]
async fn cancelling_with_no_plugin_running_says_so_instead_of_doing_nothing() {
    let mut session = RuntimeSession::new_for_test(AppState::new());

    session
        .dispatch_action(Action::System(SystemAction::CancelRunningPlugins))
        .await;

    // The key is reachable from every normal view, so most presses of it will
    // land with nothing running. Silence there is indistinguishable from a
    // dropped keypress, which is the one thing the operator cannot debug.
    assert_eq!(
        session.app().root.system.status_message.as_deref(),
        Some("No plugin is running"),
        "cancelling with an empty registry must report that, not fall silent"
    );
    assert_eq!(
        session.app().root.system.operations.pending_count(),
        0,
        "nothing was running, so nothing may be settled"
    );
}

/// A plugin that never returns on its own, and that honours cancellation the
/// way `ProcessCommandRunner` does: by polling the token while it waits.
///
/// `run` is deliberately left as a park with no exit, because that is the
/// defect under test — if the production code calls `run` instead of
/// `run_with_cancellation`, this test must hang-then-fail on its deadline
/// rather than pass.
///
/// It records into `issued` before looking at the token, which makes it a
/// fixture that always issues and not a model of the production contract:
/// `ProcessCommandRunner` checks the token before it spawns, so a real run
/// handed an already-cancelled token may never issue a command at all. That
/// difference is deliberate — a fixture that could also skip the spawn would
/// make "was a command issued?" unanswerable, and answering it is the whole job
/// of the assertions below.
#[derive(Clone, Default)]
struct NeverReturningRunner {
    state: Arc<RunnerState>,
}

#[derive(Default)]
struct RunnerState {
    inner: Mutex<RunnerInner>,
    signal: Condvar,
}

#[derive(Default)]
struct RunnerInner {
    started: usize,
    issued: Vec<Vec<String>>,
}

impl NeverReturningRunner {
    /// Blocks until `count` commands have reached the runner, giving up at the
    /// deadline rather than parking forever.
    fn wait_until_started(&self, count: usize) {
        let deadline = Instant::now() + GATE_TIMEOUT;
        let mut guard = self.state.inner.lock().unwrap();
        while guard.started < count {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                // Format first, then drop the guard: panicking while holding it
                // would poison the mutex and replace a sibling waiter's own
                // diagnostic with `PoisonError`.
                let message = format!(
                    "wait_until_started timed out after {GATE_TIMEOUT:?}: expected {count}, observed {}",
                    guard.started
                );
                drop(guard);
                panic!("{message}");
            }
            guard = self.state.signal.wait_timeout(guard, remaining).unwrap().0;
        }
    }

    fn issued(&self) -> Vec<Vec<String>> {
        self.state.inner.lock().unwrap().issued.clone()
    }
}

impl CommandRunner for NeverReturningRunner {
    fn run(&self, _command: GcloudCommand) -> Result<CommandOutput, CommandError> {
        panic!("a plugin run must go through run_with_cancellation, not run");
    }

    fn run_with_cancellation(
        &self,
        command: GcloudCommand,
        cancellation: &CancellationToken,
    ) -> Result<CommandOutput, CommandError> {
        {
            let mut guard = self.state.inner.lock().unwrap();
            guard.issued.push(command.args.clone());
            guard.started += 1;
            self.state.signal.notify_all();
        }
        let deadline = Instant::now() + GATE_TIMEOUT;
        while !cancellation.is_cancelled() {
            if Instant::now() >= deadline {
                panic!("the run was never cancelled within {GATE_TIMEOUT:?}");
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        Err(CommandError::Cancelled {
            ceiling: CommandCeiling::Read,
        })
    }
}

/// Confirms the README's plugin from the palette and waits until its argv has
/// actually reached the runner, so the cancel below lands with a child
/// genuinely in flight rather than with one still on its way to a thread.
async fn run_the_plugin(operator: &mut Operator, runner: &NeverReturningRunner) {
    operator.press(KeyCode::Char(':')).await;
    operator.type_text("describe").await;
    operator.press(KeyCode::Enter).await;
    operator.press(KeyCode::Enter).await;

    let waiter = runner.clone();
    tokio::task::spawn_blocking(move || waiter.wait_until_started(1))
        .await
        .unwrap();
}

#[tokio::test]
async fn cancelling_a_plugin_already_in_flight_settles_its_entry_unknown() {
    let runner = NeverReturningRunner::default();
    let mut operator =
        Operator::at_the_compute_list_with_runner(false, Box::new(runner.clone())).await;
    run_the_plugin(&mut operator, &runner).await;

    operator
        .session
        .dispatch_action(Action::System(SystemAction::CancelRunningPlugins))
        .await;

    let record = operator
        .session
        .app()
        .root
        .system
        .operations
        .records()
        .iter()
        .find(|record| record.kind == "plugin")
        .expect("the confirmed plugin seeded an entry")
        .clone();
    // `Unknown`, not `Cancelled`: the command was issued, so the operator has
    // something to re-check. `Cancelled` is documented as "the command was
    // never issued, so nothing changed outside the app", which would be a lie
    // about a child we killed mid-run.
    assert_eq!(record.outcome, OperationOutcome::Unknown);
    assert_eq!(
        record.detail.as_deref(),
        Some(
            "the plugin was cancelled after its command was issued; it may have already changed something"
        ),
    );
    assert_eq!(
        operator
            .session
            .app()
            .root
            .system
            .operations
            .pending_count(),
        0,
        "no entry may be left Pending by a cancellation"
    );
    assert_eq!(
        runner.issued().len(),
        1,
        "exactly one command reached the runner"
    );
    // The log entry is in a pane the operator may not have open, so this line
    // is the whole of the immediate feedback that the key did anything. It
    // counts what was cancelled, and one run is reported in the singular —
    // "Cancelled 1 running plugins" for the ordinary case would be the first
    // thing a reader distrusted about the rest of the message.
    assert_eq!(
        operator.session.app().root.system.status_message.as_deref(),
        Some("Cancelled the running plugin"),
    );
}

#[tokio::test]
async fn a_run_that_loses_the_claim_issues_nothing_and_settles_cancelled() {
    let runner = NeverReturningRunner::default();
    let mut operator =
        Operator::at_the_compute_list_with_runner(false, Box::new(runner.clone())).await;

    // Confirmed, then cancelled without waiting for the runner: the worker is
    // on a blocking thread that may not have claimed yet. Whichever side wins,
    // the pairing must hold — issued ⇒ Unknown, not issued ⇒ Cancelled — and
    // the runner's own record of what it was asked to run is the arbiter.
    operator.press(KeyCode::Char(':')).await;
    operator.type_text("describe").await;
    operator.press(KeyCode::Enter).await;
    operator.press(KeyCode::Enter).await;
    operator
        .session
        .dispatch_action(Action::System(SystemAction::CancelRunningPlugins))
        .await;
    // Gated on the worker being finished, not merely on the cancel having run.
    // Without this the loser's `issued` is read before it has had the chance to
    // issue anything, and a worker that skipped its claim entirely — the one
    // defect the claim exists to prevent — would satisfy the pairing below by
    // being slow rather than by being correct.
    wait_until_the_run_reports_itself_cancelled(&mut operator).await;

    let record = operator
        .session
        .app()
        .root
        .system
        .operations
        .records()
        .iter()
        .find(|record| record.kind == "plugin")
        .expect("the confirmed plugin seeded an entry")
        .clone();
    assert_ne!(
        record.outcome,
        OperationOutcome::Pending,
        "a cancellation settles every registered run, whoever won the claim"
    );
    let issued = runner.issued().len();
    match record.outcome {
        OperationOutcome::Unknown => assert_eq!(
            issued, 1,
            "an entry reported Unknown must have had its command issued"
        ),
        OperationOutcome::Cancelled => assert_eq!(
            issued, 0,
            "an entry reported Cancelled must never have issued a command"
        ),
        other => panic!("a cancelled plugin settles Unknown or Cancelled, not {other:?}"),
    }
}

/// Parks until the cancelled run's own result has been applied.
///
/// `"{label} cancelled"` can only be written by the `PluginRunCancelled`
/// reducer arm, and both sides of the claim end there — the worker that lost it
/// returns without issuing, the one that won returns `CommandError::Cancelled`
/// once the token flips — so this gates on the worker being finished rather
/// than on which side won. Other results share the channel, hence the loop.
async fn wait_until_the_run_reports_itself_cancelled(operator: &mut Operator) {
    let gate = async {
        while operator.session.app().root.system.status_message.as_deref()
            != Some("Describe web-01 cancelled")
        {
            operator.session.process_next_effect_result().await;
        }
    };
    tokio::time::timeout(GATE_TIMEOUT, gate)
        .await
        .expect("every cancelled run reports exactly one result, and none arrived");
}

#[tokio::test]
async fn the_cancelled_runs_late_result_neither_reopens_the_entry_nor_records_a_diagnostic() {
    let runner = NeverReturningRunner::default();
    let mut operator =
        Operator::at_the_compute_list_with_runner(false, Box::new(runner.clone())).await;
    run_the_plugin(&mut operator, &runner).await;

    operator
        .session
        .dispatch_action(Action::System(SystemAction::CancelRunningPlugins))
        .await;

    // The worker returns `CommandError::Cancelled` once the token flips. Gating
    // on its own reported result — not on a drained count — is what makes the
    // assertions below about the *late* result rather than about the cancel
    // that preceded it.
    wait_until_the_run_reports_itself_cancelled(&mut operator).await;

    let record = operator
        .session
        .app()
        .root
        .system
        .operations
        .records()
        .iter()
        .find(|record| record.kind == "plugin")
        .expect("the confirmed plugin seeded an entry")
        .clone();
    // The cancellation settled this entry; `resolve_by_id` is pending-guarded,
    // so the late result cannot overwrite what the claim proved.
    assert_eq!(record.outcome, OperationOutcome::Unknown);
    // The point of the separate result variant. `PluginRunFailed` records a
    // diagnostic unconditionally and by design, so routing a cancel through it
    // would park a fabricated failure under the copy-diagnostic key.
    assert!(
        operator
            .session
            .app()
            .root
            .system
            .last_diagnostic()
            .is_none(),
        "cancelling a plugin is not a gcloud failure and must record no diagnostic"
    );
}

#[tokio::test]
async fn cancelling_two_in_flight_plugins_settles_both_and_counts_them() {
    let runner = NeverReturningRunner::default();
    let mut operator =
        Operator::at_the_compute_list_with_runner(false, Box::new(runner.clone())).await;
    run_the_plugin(&mut operator, &runner).await;
    // A plugin does not block the palette, so a second confirm while the first
    // hangs is the ordinary way an operator ends up with two. Gated on both
    // having reached the runner, so the count below is of runs in flight rather
    // than of runs the registry happened to have seen.
    operator.press(KeyCode::Char(':')).await;
    operator.type_text("describe").await;
    operator.press(KeyCode::Enter).await;
    operator.press(KeyCode::Enter).await;
    let waiter = runner.clone();
    tokio::task::spawn_blocking(move || waiter.wait_until_started(2))
        .await
        .unwrap();

    operator
        .session
        .dispatch_action(Action::System(SystemAction::CancelRunningPlugins))
        .await;

    assert_eq!(
        operator
            .session
            .app()
            .root
            .system
            .operations
            .pending_count(),
        0,
        "one cancellation settles every run, not just the newest"
    );
    // The plural half of the message, and the only place the count is read
    // back: a cancel that reached one of the two would still leave the other's
    // entry stranded, and the operator's only clue would be this number.
    assert_eq!(
        operator.session.app().root.system.status_message.as_deref(),
        Some("Cancelled 2 running plugins"),
    );
}

/// The only test here that goes through the keymap: every other one dispatches
/// the action, so a binding that was never added, or one shadowed by the view
/// the operator is actually in, would leave them all green.
#[tokio::test]
async fn pressing_ctrl_x_cancels_the_running_plugin_through_the_real_keymap() {
    let runner = NeverReturningRunner::default();
    let mut operator =
        Operator::at_the_compute_list_with_runner(false, Box::new(runner.clone())).await;
    run_the_plugin(&mut operator, &runner).await;

    assert_eq!(
        operator
            .session
            .app()
            .root
            .system
            .operations
            .pending_count(),
        1,
        "the plugin must be in flight before the key is pressed"
    );

    operator
        .press_with(KeyCode::Char('x'), KeyModifiers::CONTROL)
        .await;

    assert_eq!(
        operator
            .session
            .app()
            .root
            .system
            .operations
            .pending_count(),
        0,
        "Ctrl-x must settle the entry a hung plugin would otherwise strand"
    );
}

/// A plugin that returns the moment it is asked to run.
///
/// The counterpart to `NeverReturningRunner`: the registry's other obligation
/// is to forget a run that ended on its own, and only a run that can end proves
/// it.
#[derive(Clone, Default)]
struct ImmediateRunner;

impl CommandRunner for ImmediateRunner {
    fn run(&self, _command: GcloudCommand) -> Result<CommandOutput, CommandError> {
        panic!("a plugin run must go through run_with_cancellation, not run");
    }

    fn run_with_cancellation(
        &self,
        _command: GcloudCommand,
        _cancellation: &CancellationToken,
    ) -> Result<CommandOutput, CommandError> {
        Ok(CommandOutput {
            stdout: "the plugin printed this".to_string(),
            stderr: String::new(),
        })
    }
}

#[tokio::test]
async fn a_finished_run_leaves_the_registry_so_the_next_cancel_says_nothing_is_running() {
    let mut operator =
        Operator::at_the_compute_list_with_runner(false, Box::new(ImmediateRunner)).await;
    operator.press(KeyCode::Char(':')).await;
    operator.type_text("describe").await;
    operator.press(KeyCode::Enter).await;
    operator.press(KeyCode::Enter).await;

    // Parked on the entry settling, which is the moment the run's result has
    // been applied and therefore the moment the registry is meant to have
    // forgotten it.
    let gate = async {
        while operator
            .session
            .app()
            .root
            .system
            .operations
            .records()
            .iter()
            .any(|record| record.kind == "plugin" && record.outcome == OperationOutcome::Pending)
        {
            operator.session.process_next_effect_result().await;
        }
    };
    tokio::time::timeout(GATE_TIMEOUT, gate)
        .await
        .expect("the run reports its own result, and none arrived");

    operator
        .session
        .dispatch_action(Action::System(SystemAction::CancelRunningPlugins))
        .await;

    // A registry that only ever grows would have this claim a run that ended
    // seconds ago: the log entry is safe either way, because `resolve_by_id`
    // will not reopen a settled record, but the operator would be told a
    // cancellation happened that did not — and the honest empty answer would
    // stay unreachable until the false one had drained the registry.
    assert_eq!(
        operator.session.app().root.system.status_message.as_deref(),
        Some("No plugin is running"),
        "a run that finished is no longer running, so there is nothing to cancel"
    );
}

#[tokio::test]
async fn quitting_with_a_plugin_in_flight_releases_its_worker_without_settling_the_entry() {
    let runner = NeverReturningRunner::default();
    let mut operator =
        Operator::at_the_compute_list_with_runner(false, Box::new(runner.clone())).await;
    run_the_plugin(&mut operator, &runner).await;

    let keep_running = operator
        .session
        .dispatch_action(Action::QuitRequested)
        .await;
    assert!(!keep_running, "QuitRequested ends the run loop");

    // The shutdown path settles nothing: the process is going away, and an
    // outcome written here would be an answer nobody is left to read. The
    // status line is not the operator's cancel key either — it belongs to a
    // session that keeps running, and this one does not.
    let record = operator
        .session
        .app()
        .root
        .system
        .operations
        .records()
        .iter()
        .find(|record| record.kind == "plugin")
        .expect("the confirmed plugin seeded an entry")
        .clone();
    assert_eq!(record.outcome, OperationOutcome::Pending);
    let status = operator.session.app().root.system.status_message.clone();
    assert_ne!(status.as_deref(), Some("Cancelled the running plugin"));
    assert_ne!(status.as_deref(), Some("No plugin is running"));

    // The property the whole fix exists for: the worker is off its blocking
    // thread before the runtime is dropped. Without the token flip this parks
    // until the fixture's own deadline, which is what a wedged `Runtime::drop`
    // does to an operator who pressed `q` instead of Ctrl-x.
    wait_until_the_run_reports_itself_cancelled(&mut operator).await;
    assert_eq!(
        runner.issued().len(),
        1,
        "exactly one command reached the runner"
    );
}

/// Frame size for the rendered evidence below. Wide and tall enough that the
/// Home "Recent operations" card shows each entry's detail line in full rather
/// than elided, since what the operator is meant to read is the detail.
///
/// The height tracks the Home inventory, which takes one line per service, so
/// every service added pushes the card down a row. The firewall rules
/// service took it from 35 to 36.
const FRAME_WIDTH: u16 = 140;
const FRAME_HEIGHT: u16 = 36;

fn draw(session: &RuntimeSession) -> Buffer {
    let mut terminal =
        Terminal::new(TestBackend::new(FRAME_WIDTH, FRAME_HEIGHT)).expect("terminal init");
    terminal
        .draw(|frame| nuvem::ui::render(frame, session))
        .expect("render");
    terminal.backend().buffer().clone()
}

fn frame_text(buffer: &Buffer) -> String {
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

/// Written only under `NUVEM_EVIDENCE_DIR`, never to a fixed machine-global path:
/// that would race every other cargo process on the box.
fn write_frame(name: &str, title: &str, caption: &str, buffer: &Buffer) {
    let Ok(dir) = std::env::var("NUVEM_EVIDENCE_DIR") else {
        eprintln!("NUVEM_EVIDENCE_DIR not set; skipping evidence render");
        return;
    };
    let dir = std::path::PathBuf::from(dir);
    std::fs::create_dir_all(&dir).expect("evidence dir");
    std::fs::write(
        dir.join(format!("{name}.svg")),
        buffer_to_svg(buffer, title, caption),
    )
    .expect("write svg frame");
    std::fs::write(
        dir.join(format!("{name}.txt")),
        format!("{title}\n{caption}\n\n{}\n", frame_text(buffer)),
    )
    .expect("write ascii frame");
}

/// The whole feature as the operator sees it, drawn through the real
/// `nuvem::ui::render`: a plugin stuck `Pending` on the Home dashboard, the same
/// card after `Ctrl-x`, and the help overlay that advertises the key.
///
/// The assertions are on the rendered frame, not on state, because a binding
/// present in the catalog but filtered out of every overlay section, or an
/// outcome settled but never drawn, would leave the state-level tests above
/// green while the operator saw nothing.
#[tokio::test]
async fn the_operator_sees_the_stuck_run_the_cancel_and_the_advertised_key() {
    let runner = NeverReturningRunner::default();
    let mut operator =
        Operator::at_the_compute_list_with_runner(false, Box::new(runner.clone())).await;
    run_the_plugin(&mut operator, &runner).await;

    operator
        .session
        .dispatch_action(Action::Workspace(WorkspaceAction::Home))
        .await;
    let before = draw(&operator.session);
    let before_text = frame_text(&before);
    assert!(
        before_text.contains("Pending"),
        "the hung plugin's entry must read Pending before the cancel:\n{before_text}"
    );
    write_frame(
        "01_plugin_stuck_pending",
        "A plugin that never returns",
        "Home / Recent operations: the confirmed plugin is Pending and, before this change, stayed that way for the life of the process.",
        &before,
    );

    operator
        .press_with(KeyCode::Char('x'), KeyModifiers::CONTROL)
        .await;
    let after = draw(&operator.session);
    let after_text = frame_text(&after);
    assert!(
        after_text.contains("Cancelled the running plugin"),
        "the status line is the operator's immediate feedback:\n{after_text}"
    );
    assert!(
        !after_text.contains("Pending"),
        "the cancelled entry must no longer be drawn Pending:\n{after_text}"
    );
    assert!(
        after_text.contains("cancelled after its command was issued"),
        "the entry must say what the operator has to re-check:\n{after_text}"
    );
    write_frame(
        "02_after_ctrl_x",
        "Ctrl-x cancels the running plugin",
        "The status line reports the cancellation and the log entry settles Unknown: the command was issued, so it may already have changed something.",
        &after,
    );

    operator.press(KeyCode::Char('?')).await;
    let help = draw(&operator.session);
    let help_text = frame_text(&help);
    assert!(
        help_text.contains("Ctrl-x") && help_text.contains("Cancel running plugin"),
        "a key nobody is told about is a key nobody presses:\n{help_text}"
    );
    write_frame(
        "03_help_overlay",
        "The key is advertised",
        "Help overlay, Global section: Ctrl-x  Cancel running plugin.",
        &help,
    );

    // Leaves the worker off its blocking thread before the runtime is dropped.
    operator.press(KeyCode::Esc).await;
    wait_until_the_run_reports_itself_cancelled(&mut operator).await;
}
