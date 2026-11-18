//! Evidence — a batch superseded by a second one settles its
//! operations-log entries instead of stranding them `Pending`.
//!
//! Marks survive batch confirmation, so confirming a second batch over a
//! running one is a plain sequence of UI steps, not an exotic state. Before
//! this ticket the dispatch path replaced the in-flight target list wholesale
//! and bumped the batch generation without settling the outgoing batch: the
//! generation bump then dropped every result of the first batch, and its
//! "Recent operations" entries read `Pending` for the life of the process.
//!
//! The second half of the evidence is the part the ticket did not ask for.
//! Settling the outgoing batch by calling the full cancellation path would also
//! tear down the batch-progress workflow — and by the time the dispatch path
//! runs, that workflow already belongs to the batch the operator just
//! confirmed. Blinding the incoming batch to fix the outgoing one trades one
//! silent failure for another, so it is pinned here too.
//!
//! The assertions always run. The transcript is written only when
//! `NUVEM_EVIDENCE_DIR` is set, and only under that directory: an evidence file at
//! a fixed machine-global path races every other cargo process on the box.
//!
//! Every wait here is a bounded gate released explicitly by the test — no
//! sleeps, no wall-clock assertions, no unbounded parks. An unbounded park
//! would be worse than slow: these waits run on `spawn_blocking` threads, where
//! they are uncancellable and wedge `Runtime::drop` for good.

use std::collections::HashMap;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use nuvem::Config;
use nuvem::app::reducer::root::reduce;
use nuvem::app::state::operations::{OperationOutcome, OperationRecord};
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
/// datasets service took this from 36 to 37, VPC networks from 37 to 38, its
/// subnets from 38 to 39, and its firewall rules from 39 to 40.
const HEIGHT: u16 = 40;

/// Deadline for every gate wait. The test releases its gates within
/// milliseconds, so 30s sits orders of magnitude above any plausible scheduling
/// delay: exceeding it proves the gate wedged, not that the machine was slow.
const GATE_TIMEOUT: Duration = Duration::from_secs(30);

/// Detail carried by the entry of a target that never issued its command.
///
/// It names supersession, not cancellation: on this path the operator cancelled
/// nothing, they confirmed a replacement batch. The outcome is still
/// `Cancelled` — the command provably did not run — but the agent of that is a
/// newer batch. The cancellation path keeps its own wording, pinned by
/// `tests/batch_cancel_evidence.rs`.
const SUPERSEDED_DETAIL: &str = "the batch was superseded by a newer one before this command ran";

/// Detail carried by the entry of a target whose command was already in flight.
/// It names no key on purpose: refresh is user-rebindable.
const SUPERSEDED_UNKNOWN_DETAIL: &str = "the batch was superseded by a newer one after this command was issued; refresh this service to see the resource's actual state";

/// The cancellation path's wording, pinned here as *absent* from this path's
/// entries. Rewording only one path is what was asked for; asserting the
/// new string alone would still pass if both paths had been reworded together.
const CANCELLED_DETAIL: &str = "the batch was cancelled before this command ran";

fn output(stdout: impl Into<String>) -> Result<CommandOutput, CommandError> {
    Ok(CommandOutput {
        stdout: stdout.into(),
        stderr: String::new(),
    })
}

/// Whether `command` is one of a batch's own `compute instances stop` actions —
/// the only thing this test wants to hold open.
///
/// Matched positively on the verb rather than negatively on "not a list", so
/// the gate fails closed. Any *other* gcloud command the runtime happens to
/// emit — the end-of-batch list refresh above all — would, under a negative
/// match, be parked until the gate deadline and turn this test into a
/// 30-second stall followed by a panic about something unrelated.
fn is_gated_action(command: &GcloudCommand) -> bool {
    command
        .args
        .windows(3)
        .any(|window| window == ["compute", "instances", "stop"])
}

/// Whether `command` is a Compute instance listing, i.e. the shape the
/// end-of-batch refresh takes. Matched the same positive way and for the same
/// reason.
fn is_instance_list(command: &GcloudCommand) -> bool {
    command
        .args
        .windows(3)
        .any(|window| window == ["compute", "instances", "list"])
}

/// Parks every batch action command behind an explicit release and answers
/// everything else immediately, so the test can supersede the first batch at
/// the one moment that matters: with a target's gcloud command genuinely in
/// flight.
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
    instance_lists: usize,
}

impl GateRunner {
    fn new(list_output: &str) -> Self {
        Self {
            state: Arc::new(GateState {
                inner: Mutex::new(GateInner {
                    started: 0,
                    release_allowed: 0,
                    action_commands: Vec::new(),
                    instance_lists: 0,
                }),
                signal: Condvar::new(),
                list_output: list_output.to_string(),
            }),
        }
    }

    /// Blocks until `count` actions have reached the runner, giving up at the
    /// deadline rather than parking forever.
    fn wait_until_started(&self, count: usize) {
        self.wait_for("wait_until_started", count, |inner| inner.started);
    }

    /// Blocks until `count` instance listings have reached the runner. The
    /// end-of-batch refresh is dispatched onto a background task, so the
    /// command it issues arrives after `process_next_effect_result` returns and
    /// has to be waited for rather than read.
    fn wait_until_listed(&self, count: usize) {
        self.wait_for("wait_until_listed", count, |inner| inner.instance_lists);
    }

    fn wait_for(&self, what: &str, count: usize, observe: fn(&GateInner) -> usize) {
        let deadline = Instant::now() + GATE_TIMEOUT;
        let mut guard = self.state.inner.lock().unwrap();
        while observe(&guard) < count {
            // Budget the remaining time against the single deadline so a
            // spurious wakeup cannot restart the full timeout.
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                // Format first, then drop the guard: panicking while holding it
                // would poison the mutex and replace any sibling waiter's own
                // diagnostic with `PoisonError`.
                let message = format!(
                    "{what} timed out after {GATE_TIMEOUT:?}: expected {count}, observed {}",
                    observe(&guard)
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

    fn instance_lists(&self) -> usize {
        self.state.inner.lock().unwrap().instance_lists
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
        // Anything that is not a batch action is answered on the spot with the
        // instance-list fixture. Parking it instead would hold a
        // `spawn_blocking` thread nothing in this test ever releases.
        if !is_gated_action(&command) {
            if is_instance_list(&command) {
                let mut guard = self.state.inner.lock().unwrap();
                guard.instance_lists += 1;
                self.state.signal.notify_all();
            }
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

/// Every instance loaded under the dev context, every row marked, and a pending
/// batch-Stop confirmation.
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

/// Re-confirms the still-marked rows, which is exactly what the operator does:
/// confirming a batch leaves the marks in place, so the second batch is two
/// keystrokes away from the first.
async fn confirm_batch_again(session: &mut RuntimeSession) {
    let context = session
        .app()
        .root
        .current_context()
        .expect("context")
        .clone();
    session
        .app_mut()
        .root
        .resources
        .start_batch_confirmation::<Compute>(context, ComputeInstanceAction::Stop);
    session
        .dispatch_action(Action::Resources(ResourcesAction::ConfirmBatchAction))
        .await;
}

/// The operations-log entries seeded so far, keyed by target name. Captured
/// while only the first batch exists, because the second batch seeds a second
/// entry per target: the id is the only handle that tells the two apart.
fn operation_ids_by_target(session: &RuntimeSession) -> HashMap<String, u64> {
    session
        .app()
        .root
        .system
        .operations
        .records()
        .iter()
        .map(|record| (record.target.clone(), record.id))
        .collect()
}

fn record_by_id(session: &RuntimeSession, id: u64) -> OperationRecord {
    session
        .app()
        .root
        .system
        .operations
        .records()
        .iter()
        .find(|record| record.id == id)
        .unwrap_or_else(|| panic!("no operations-log entry with id {id}"))
        .clone()
}

/// Drains exactly `count` effect results. A known count rather than a
/// condition loop: every result this test can produce is accounted for below,
/// so a drain that needs a timeout to stop would be hiding a miscount.
async fn drain_exactly(session: &mut RuntimeSession, count: usize) {
    for index in 0..count {
        assert!(
            session.process_next_effect_result().await,
            "the effect-result channel closed after {index} of {count} results"
        );
    }
}

/// Two marked instances, one action permitted at a time, and a second batch
/// confirmed while the first target's gcloud command is in flight: the first
/// batch's entries must be settled at what is provable, not left claiming work
/// still in progress.
#[tokio::test]
async fn superseded_batch_settles_its_operations_log_entries() {
    let runner = GateRunner::new(include_str!("fixtures/gce_instances.json"));
    let app = marked_batch_app(vec![instance("vm-a"), instance("vm-b")]);
    // One permit, so exactly one target of the first batch can be in flight
    // when the second one lands and the other is provably still parked on the
    // semaphore.
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
        "vm-b".to_string()
    } else {
        "vm-a".to_string()
    };

    let first_batch = operation_ids_by_target(&session);
    assert_eq!(
        session.app().root.system.operations.pending_count(),
        2,
        "both of the first batch's entries should be in flight before it is superseded"
    );
    assert_eq!(
        session.app().root.resources.marked_count(),
        2,
        "marks must survive batch confirmation, or the second confirm is unreachable"
    );

    confirm_batch_again(&mut session).await;

    // The settlement is synchronous with the dispatch, so it is observable
    // before a single result is drained. That matters: from here on every one
    // of the first batch's results is dropped by the generation guard, so
    // whatever is still `Pending` at this instant stays `Pending` forever.
    let started_record = record_by_id(&session, first_batch[&started_name]);
    let unstarted_record = record_by_id(&session, first_batch[&unstarted_name]);
    assert_eq!(
        (started_record.outcome, started_record.detail.clone()),
        (
            OperationOutcome::Unknown,
            Some(SUPERSEDED_UNKNOWN_DETAIL.to_string())
        ),
        "the superseded batch's in-flight target must be reported Unknown, and say what detached it"
    );
    assert_eq!(
        (unstarted_record.outcome, unstarted_record.detail.clone()),
        (
            OperationOutcome::Cancelled,
            Some(SUPERSEDED_DETAIL.to_string())
        ),
        "the superseded batch's unstarted target must be reported Cancelled, and say what detached it"
    );
    // The outcome is shared with the cancellation path; the wording is not.
    for record in [&started_record, &unstarted_record] {
        let detail = record.detail.as_deref().unwrap_or_default();
        assert!(
            !detail.contains("was cancelled"),
            "the supersession path must not tell the operator they cancelled the batch: {detail}"
        );
        assert_ne!(detail, CANCELLED_DETAIL);
    }
    assert_eq!(
        session.app().root.system.operations.pending_count(),
        2,
        "only the incoming batch's two entries may still be pending"
    );

    // Three results and no more: the first batch's in-flight target (dropped by
    // the generation guard), and the incoming batch's two targets. The first
    // batch's unstarted target issues nothing and reports nothing — the
    // cancellation token saw to that — which is what the `started` count below
    // confirms.
    runner.release(3);
    drain_exactly(&mut session, 3).await;

    assert_eq!(
        runner.started(),
        3,
        "the superseded batch's unstarted target must never have issued a command"
    );
    assert_eq!(
        session.app().root.system.operations.pending_count(),
        0,
        "no entry may be left claiming work still in progress"
    );

    // The records are the state; the card is what the operator actually reads.
    // One more result than the three above: the end-of-batch refresh the
    // incoming batch's workflow fires. It is drained so the progress overlay
    // finishes and stops sitting over the Home card.
    drain_exactly(&mut session, 1).await;
    let after_card = operations_card(&render_lines(&session));
    let card = after_card.join("\n");
    assert!(
        card.contains("superseded by a newer one"),
        "the Home card must tell the operator a newer batch took this one over:\n{card}"
    );
    assert!(
        !card.contains("was cancelled"),
        "the Home card must not tell the operator they cancelled anything:\n{card}"
    );

    let report = format!(
        "A superseded batch settles its operations-log entries\n\
         ================================================================================\n\n\
         Two marked GCE instances, batch Stop, one action permitted at a time.\n\
         `{started_name}` holds the permit and its gcloud command is in flight;\n\
         `{unstarted_name}` is still parked on the semaphore and has issued nothing.\n\
         Marks survive confirmation, so the operator confirms the same two rows again.\n\n\
         The second confirmation detaches the first batch: its results will be dropped\n\
         from here on, so this is the last moment at which anything true can be said\n\
         about it. Each entry is settled at what is actually provable:\n\
         --------------------------------------------------------------------------------\n\
         {}\n{}\n\n\
         After the incoming batch drains, nothing is left claiming work in progress:\n\
         --------------------------------------------------------------------------------\n{}\n\n\
         The same moment on the surface the operator reads — the Home overview's\n\
         \"Recent operations\" card, rasterized through the real renderer. The two\n\
         settled entries name the newer batch, not a cancellation nobody performed:\n\
         --------------------------------------------------------------------------------\n{}\n",
        render_record(&started_record),
        render_record(&unstarted_record),
        session
            .app()
            .root
            .system
            .operations
            .records()
            .iter()
            .map(render_record)
            .collect::<Vec<String>>()
            .join("\n"),
        card,
    );

    println!("\n{report}");

    // Written only under NUVEM_EVIDENCE_DIR, never to a fixed global path: a
    // machine-wide location races every other cargo process on the box.
    if let Ok(dir) = std::env::var("NUVEM_EVIDENCE_DIR") {
        let path = std::path::Path::new(&dir).join("batch_supersession_operations.txt");
        std::fs::write(&path, &report).expect("write evidence file");
    }
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
/// Sliced out rather than asserted against the whole screen so wording drawn by
/// some unrelated panel can neither mask a defect nor fake one.
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

fn render_record(record: &OperationRecord) -> String {
    format!(
        "  {:<10} {} {}{}",
        record.outcome.label(),
        record.kind,
        record.target,
        record
            .detail
            .as_ref()
            .map(|detail| format!(" — {detail}"))
            .unwrap_or_default()
    )
}

/// Settling the outgoing batch must not blind the incoming one. The reducer
/// installs the new batch's progress workflow *before* emitting the effect that
/// detaches the old batch, so a settlement that also cleared the workflow would
/// destroy the workflow of the batch the operator just confirmed: no progress
/// overlay, and no end-of-batch refresh.
#[tokio::test]
async fn superseding_batch_keeps_its_workflow_and_still_refreshes() {
    let runner = GateRunner::new(include_str!("fixtures/gce_instances.json"));
    let app = marked_batch_app(vec![instance("vm-a"), instance("vm-b")]);
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

    let first_batch_ids: Vec<u64> = operation_ids_by_target(&session).into_values().collect();
    let lists_before = runner.instance_lists();

    confirm_batch_again(&mut session).await;

    let workflow = session
        .app()
        .root
        .resources
        .batch_workflow()
        .expect("the batch the operator just confirmed must still have a progress workflow");
    assert_eq!(workflow.targets().len(), 2);
    assert_eq!(
        workflow.pending_count(),
        2,
        "the workflow must describe the incoming batch, not a finished one"
    );
    // Identity, not just presence: the workflow's targets carry the
    // operations-log ids seeded for them, so a workflow left over from the
    // first batch would be caught here rather than passing as "some workflow".
    for target in workflow.targets() {
        assert!(
            !first_batch_ids.contains(&target.operation_id),
            "the workflow must own the incoming batch's entries, not the superseded batch's"
        );
    }

    // The stale in-flight target plus the incoming batch's two targets.
    runner.release(3);
    drain_exactly(&mut session, 3).await;
    assert_eq!(
        runner.started(),
        3,
        "the superseded batch's unstarted target must never have issued a command"
    );

    // The refresh is observed where the operator would feel it — a real gcloud
    // listing reaching the runner — rather than by reading the effect queue.
    // It is dispatched onto a background task, so it is waited for, not read.
    let waiter = runner.clone();
    let expected = lists_before + 1;
    tokio::task::spawn_blocking(move || waiter.wait_until_listed(expected))
        .await
        .unwrap();
}
