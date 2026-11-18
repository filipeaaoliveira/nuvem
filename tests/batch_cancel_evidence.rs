//! Evidence — a cancelled batch settles its operations-log entries
//! instead of stranding them `Pending`.
//!
//! The defect was only ever visible on one surface: the Home overview's
//! "Recent operations" card. Before this ticket, cancelling a batch mid-flight
//! left every unresolved target rendered there as `Pending` for the life of the
//! process — work in progress that nothing would ever finish. This drives a
//! real two-target Compute batch through the real runtime, cancels it with a
//! configuration switch while exactly one target's gcloud command is in flight,
//! and rasterizes the Home card through the real `nuvem::ui::render` before and
//! after the switch.
//!
//! The assertions always run. The transcript is written only when
//! `NUVEM_EVIDENCE_DIR` is set, and only under that directory: an evidence file at
//! a fixed machine-global path races every other cargo process on the box.
//!
//! Every wait here is a bounded gate released explicitly by the test — no
//! sleeps, no wall-clock assertions, no unbounded parks. An unbounded park
//! would be worse than slow: these waits run on `spawn_blocking` threads, where
//! they are uncancellable and wedge `Runtime::drop` for good.

use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use nuvem::Config;
use nuvem::app::reducer::root::reduce;
use nuvem::app::state::operations::OperationOutcome;
use nuvem::app::state::resources::{Compute, ComputeInstanceAction};
use nuvem::app::{Action, AppState, EffectResult, ResourcesAction};
use nuvem::services::gcloud::cli::{CommandError, CommandOutput, CommandRunner, GcloudCommand};
use nuvem::services::runtime::session::RuntimeSession;
use nuvem_domain::gcp::configuration::{GcloudConfiguration, GcloudConfigurationName};
use nuvem_domain::gcp::resource::{ComputeInstance, ResourceStatus};
use ratatui::{Terminal, backend::TestBackend};

/// Wide enough that the `Unknown` entry's detail — the longest string on the
/// card — is rendered in full rather than elided, since the point of the
/// evidence is that the operator can read what to do next.
const WIDTH: u16 = 132;
/// Tall enough that the operations card still shows each entry's detail line
/// after the Home inventory grew: the inventory takes one line per service, so
/// every service added pushes the card down by one row. the BigQuery
/// datasets service took this from 34 to 35, VPC networks from 35 to 36, its
/// subnets from 36 to 37, and its firewall rules from 37 to 38.
const HEIGHT: u16 = 38;

/// Deadline for every gate wait. The test releases its gate within
/// milliseconds, so 30s sits orders of magnitude above any plausible scheduling
/// delay: exceeding it proves the gate wedged, not that the machine was slow.
const GATE_TIMEOUT: Duration = Duration::from_secs(30);

/// Detail carried by the entry of a target that never issued its command.
///
/// This is the cancellation path, where the operator really did take the batch
/// away, so naming cancellation is accurate here and stays as it was. The
/// supersession path shares these outcomes but not this wording; its
/// strings are pinned by `tests/batch_supersession_evidence.rs`.
const CANCELLED_DETAIL: &str = "the batch was cancelled before this command ran";

/// Detail carried by the entry of a target whose command was already in flight.
/// It names no key on purpose: refresh is user-rebindable.
const UNKNOWN_DETAIL: &str = "the batch was cancelled after this command was issued; refresh this service to see the resource's actual state";

fn output(stdout: impl Into<String>) -> Result<CommandOutput, CommandError> {
    Ok(CommandOutput {
        stdout: stdout.into(),
        stderr: String::new(),
    })
}

/// Whether `command` is one of the batch's own `compute instances stop`
/// actions — the only thing this test wants to hold open.
///
/// Matched positively on the verb rather than negatively on "not a list", so
/// the gate fails closed. `apply_effect_result` awaits `dispatch_effect`
/// inline, so any *other* gcloud command the configuration switch happens to
/// emit — a diagnostics probe, another service's load — would, under a
/// negative match, be parked until the gate deadline and turn this test into a
/// 30-second stall followed by a panic about something unrelated.
fn is_gated_action(command: &GcloudCommand) -> bool {
    command
        .args
        .windows(3)
        .any(|window| window == ["compute", "instances", "stop"])
}

/// Parks the batch's own action commands behind an explicit release and answers
/// everything else immediately, so the test can cancel the batch at the one
/// moment that matters: with a target's gcloud command genuinely in flight.
#[derive(Clone)]
struct GateRunner {
    state: Arc<GateState>,
}

struct GateState {
    inner: Mutex<GateInner>,
    signal: Condvar,
    list_output: String,
}

struct GateInner {
    started: usize,
    release_allowed: usize,
    action_commands: Vec<GcloudCommand>,
}

impl GateRunner {
    fn new(list_output: &str) -> Self {
        Self {
            state: Arc::new(GateState {
                inner: Mutex::new(GateInner {
                    started: 0,
                    release_allowed: 0,
                    action_commands: Vec::new(),
                }),
                signal: Condvar::new(),
                list_output: list_output.to_string(),
            }),
        }
    }

    /// Blocks until `count` actions have reached the runner, giving up at the
    /// deadline rather than parking forever.
    fn wait_until_started(&self, count: usize) {
        let deadline = Instant::now() + GATE_TIMEOUT;
        let mut guard = self.state.inner.lock().unwrap();
        while guard.started < count {
            // Budget the remaining time against the single deadline so a
            // spurious wakeup cannot restart the full timeout.
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                // Format first, then drop the guard: panicking while holding it
                // would poison the mutex and replace any sibling waiter's own
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

    fn started(&self) -> usize {
        self.state.inner.lock().unwrap().started
    }

    /// Names of the Compute targets whose command actually reached the runner,
    /// i.e. the ones GCP may already be acting on.
    fn started_targets(&self) -> Vec<String> {
        self.state
            .inner
            .lock()
            .unwrap()
            .action_commands
            .iter()
            .map(|command| {
                let zone_index = command
                    .args
                    .iter()
                    .position(|arg| arg == "--zone")
                    .expect("a compute action command names its zone");
                command.args[zone_index - 1].clone()
            })
            .collect()
    }

    /// Permits every action whose start index is `< up_to` to complete.
    fn release(&self, up_to: usize) {
        let mut guard = self.state.inner.lock().unwrap();
        guard.release_allowed = up_to;
        self.state.signal.notify_all();
    }
}

impl CommandRunner for GateRunner {
    fn run(&self, command: GcloudCommand) -> Result<CommandOutput, CommandError> {
        // Anything that is not one of the batch's own actions is answered on
        // the spot with the instance-list fixture. Parking it instead would
        // hold a `spawn_blocking` thread nothing in this test ever releases.
        if !is_gated_action(&command) {
            return output(self.state.list_output.clone());
        }
        let my_index = {
            let mut guard = self.state.inner.lock().unwrap();
            guard.action_commands.push(command);
            let index = guard.started;
            guard.started += 1;
            self.state.signal.notify_all();
            index
        };
        let deadline = Instant::now() + GATE_TIMEOUT;
        let mut guard = self.state.inner.lock().unwrap();
        while guard.release_allowed <= my_index {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                let message = format!(
                    "gated action {my_index} timed out after {GATE_TIMEOUT:?}: release_allowed is {}",
                    guard.release_allowed
                );
                drop(guard);
                panic!("{message}");
            }
            guard = self.state.signal.wait_timeout(guard, remaining).unwrap().0;
        }
        self.state.signal.notify_all();
        output("")
    }
}

fn config() -> GcloudConfiguration {
    GcloudConfiguration {
        name: GcloudConfigurationName::new("dev"),
        is_active: true,
        account: None,
        project: Some("dev-project".to_string()),
        region: Some("europe-west1".to_string()),
        zone: Some("europe-west1-b".to_string()),
    }
}

fn other_config() -> GcloudConfiguration {
    GcloudConfiguration {
        name: GcloudConfigurationName::new("prod"),
        is_active: true,
        account: None,
        project: Some("prod-project".to_string()),
        region: Some("europe-west1".to_string()),
        zone: Some("europe-west1-b".to_string()),
    }
}

fn instance(name: &str) -> ComputeInstance {
    ComputeInstance {
        id: format!("{name}-id"),
        name: name.to_string(),
        zone: "europe-west1-b".to_string(),
        status: ResourceStatus::Running,
        machine_type: "e2-medium".to_string(),
        internal_ip: None,
        external_ip: None,
    }
}

/// Both instances loaded under the dev context, both marked, and a pending
/// batch-Stop confirmation. The workspace is left at its default (Home), which
/// is the surface this evidence is about: confirming and cancelling a batch
/// changes no workspace, so the card is watched in place.
fn marked_batch_app(instances: Vec<ComputeInstance>) -> AppState {
    let mut app = AppState::new();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ConfigurationsLoaded {
            configurations: vec![config()],
        }),
    );
    let context = app.root.current_context().expect("context").clone();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ComputeInstancesLoaded {
            context: context.clone(),
            instances,
        }),
    );
    let count = app.root.resources.visible_instances().len();
    app.root.resources.toggle_mark();
    for _ in 1..count {
        app.root.resources.select_next();
        app.root.resources.toggle_mark();
    }
    app.root
        .resources
        .start_batch_confirmation::<Compute>(context, ComputeInstanceAction::Stop);
    app
}

/// The rendered terminal, one `String` per row, trailing blanks trimmed.
fn render_lines(session: &RuntimeSession) -> Vec<String> {
    let mut terminal = Terminal::new(TestBackend::new(WIDTH, HEIGHT)).expect("terminal");
    terminal
        .draw(|frame| nuvem::ui::render(frame, session))
        .expect("render");
    let buffer = terminal.backend().buffer();
    (0..buffer.area.height)
        .map(|y| {
            (0..buffer.area.width)
                .map(|x| buffer.cell((x, y)).expect("cell").symbol().to_string())
                .collect::<String>()
                .trim_end()
                .to_string()
        })
        .collect()
}

/// The "Recent operations" card's rows, from its top border to its bottom one.
/// Sliced out rather than asserted against the whole screen so a `Pending`
/// belonging to some unrelated panel can neither mask a defect nor fake one.
fn operations_card(lines: &[String]) -> Vec<String> {
    let start = lines
        .iter()
        // Matched on a prefix: the card's border title is drawn into whatever
        // width the layout gives it and can be clipped.
        .position(|line| line.contains("Recent oper"))
        .unwrap_or_else(|| {
            panic!(
                "the Home overview must render a 'Recent operations' card:\n{}",
                lines.join("\n")
            )
        });
    let mut card: Vec<String> = Vec::new();
    for line in &lines[start..] {
        card.push(line.clone());
        if card.len() > 1 && line.trim_start().starts_with('└') {
            break;
        }
    }
    card
}

/// The operations-log outcome and detail recorded for `target`.
fn operation_for(session: &RuntimeSession, target: &str) -> (OperationOutcome, Option<String>) {
    let record = session
        .app()
        .root
        .system
        .operations
        .records()
        .iter()
        .find(|record| record.target == target)
        .unwrap_or_else(|| panic!("no operations-log entry for {target}"));
    (record.outcome, record.detail.clone())
}

/// Cancel a two-target Compute batch with one command in flight, then read the
/// Home card: one `Cancelled` entry, one `Unknown` entry, both with the detail
/// that says which of the two situations the operator is in, and not a single
/// line still claiming `Pending`.
#[tokio::test]
async fn cancelled_batch_leaves_no_pending_line_on_the_home_operations_card() {
    let runner = GateRunner::new(include_str!("fixtures/gce_instances.json"));
    let app = marked_batch_app(vec![instance("vm-a"), instance("vm-b")]);
    // One permit, so exactly one target can be in flight when the switch lands
    // and the other is provably still parked on the semaphore.
    let session_config = Config {
        batch_concurrency: 1,
        ..Config::default()
    };
    let mut session = RuntimeSession::new_with_config_and_runner_for_test(
        session_config,
        app,
        Box::new(runner.clone()),
    );
    session
        .dispatch_action(Action::Resources(ResourcesAction::ConfirmBatchAction))
        .await;

    let waiter = runner.clone();
    tokio::task::spawn_blocking(move || waiter.wait_until_started(1))
        .await
        .unwrap();

    // Which target won the single permit is the scheduler's business; the
    // runner knows which command it actually answered.
    let started = runner.started_targets();
    assert_eq!(started.len(), 1, "exactly one target should be in flight");
    let started_name = started[0].clone();
    let unstarted_name = if started_name == "vm-a" {
        "vm-b"
    } else {
        "vm-a"
    };

    // The "before" frame is captured whole rather than sliced to the card: the
    // live batch-progress panel is an overlay drawn centred over Home, so it
    // covers part of the card while the batch runs. That is exactly what the
    // operator sees, and the card underneath is asserted against state instead.
    let before_lines = render_lines(&session);
    let before = before_lines.join("\n");
    assert_eq!(
        session.app().root.system.operations.pending_count(),
        2,
        "both seeded entries should be in flight before the cancellation:\n{before}"
    );
    assert!(
        // Not a typo: the overlay clips the card's rows mid-word, so matching
        // the full "Pending  Stop instance …" would assert on text the frame
        // does not contain at this moment.
        before.contains("Pending  Sto"),
        "the card should be showing in-flight entries before the cancellation:\n{before}"
    );

    // Activating another configuration is a context switch, which detaches the
    // batch. This is the path that tears the workflow down inside the reducer,
    // so it also proves the pre-reduce snapshot is doing its job.
    session
        .apply_effect_result(EffectResult::ConfigurationsLoaded {
            configurations: vec![
                GcloudConfiguration {
                    is_active: false,
                    ..config()
                },
                other_config(),
            ],
        })
        .await;

    let after_lines = render_lines(&session);
    let after_card = operations_card(&after_lines);
    let after = after_card.join("\n");

    assert!(
        !after.contains("Pending"),
        "no line of the card may still read Pending after a cancellation:\n{after}"
    );
    assert!(
        after.contains(&format!("Unknown  Stop instance {started_name}")),
        "the in-flight target must be reported Unknown:\n{after}"
    );
    assert!(
        after.contains(&format!("Cancelled  Stop instance {unstarted_name}")),
        "the unstarted target must be reported Cancelled:\n{after}"
    );
    assert!(
        after.contains(UNKNOWN_DETAIL),
        "the Unknown entry must tell the operator to re-check the real state:\n{after}"
    );
    assert!(
        after.contains(CANCELLED_DETAIL),
        "the Cancelled entry must say the command never ran:\n{after}"
    );

    // The card is a projection; pin the state behind it too, so a rendering
    // change cannot quietly turn this into a screenshot test of nothing.
    assert_eq!(session.app().root.system.operations.pending_count(), 0);
    assert_eq!(
        operation_for(&session, &started_name),
        (OperationOutcome::Unknown, Some(UNKNOWN_DETAIL.to_string()))
    );
    assert_eq!(
        operation_for(&session, unstarted_name),
        (
            OperationOutcome::Cancelled,
            Some(CANCELLED_DETAIL.to_string())
        )
    );
    // The mirror of the supersession pin: this path must keep naming the
    // operator's own cancellation, not borrow the newer batch's wording.
    assert!(
        !after.contains("superseded by a newer one"),
        "a batch taken away by a context switch was not superseded:\n{after}"
    );

    // Drain the gated target so no `spawn_blocking` thread is left parked. Its
    // late result is dropped by the generation guard and must not rewrite the
    // settled entries.
    runner.release(2);
    assert!(session.process_next_effect_result().await);
    assert_eq!(runner.started(), 1, "the unstarted target never ran");
    assert_eq!(session.app().root.system.operations.pending_count(), 0);

    let report = format!(
        "A cancelled batch settles its operations-log entries\n\
         ================================================================================\n\n\
         Two marked GCE instances, batch Stop, one action permitted at a time.\n\
         `{started_name}` holds the permit and its gcloud command is in flight;\n\
         `{unstarted_name}` is still parked on the semaphore and has issued nothing.\n\n\
         STEP 1 — batch in flight. The progress overlay sits over Home; the\n\
         \"Recent operations\" card underneath it carries both entries as Pending:\n\
         --------------------------------------------------------------------------------\n{}\n\n\
         STEP 2 — the operator switches gcloud configuration, which detaches the batch.\n\
         Every entry is settled at what is actually provable: `{unstarted_name}` never\n\
         issued a command (Cancelled), `{started_name}`'s command was already issued and\n\
         its result is discarded, so GCP may well be carrying it out (Unknown, with the\n\
         next move spelled out and no key named — refresh is rebindable). No line is left\n\
         claiming work still in progress:\n\
         --------------------------------------------------------------------------------\n{}\n",
        before,
        after_card.join("\n"),
    );

    println!("\n{report}");

    // Written only under NUVEM_EVIDENCE_DIR, never to a fixed global path: a
    // machine-wide location races every other cargo process on the box.
    if let Ok(dir) = std::env::var("NUVEM_EVIDENCE_DIR") {
        let path = std::path::Path::new(&dir).join("batch_cancel_operations_card.txt");
        std::fs::write(&path, &report).expect("write evidence file");
    }
}
