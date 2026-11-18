use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use nuvem::Config;
use nuvem::app::reducer::root::reduce;
use nuvem::app::state::operations::OperationOutcome;
use nuvem::app::state::resources::{
    CloudSql, CloudSqlInstanceAction, Compute, ComputeInstanceAction,
};
use nuvem::app::{Action, AppError, AppState, EffectResult, ResourcesAction, WorkspaceAction};
use nuvem::services::gcloud::cli::{CommandError, CommandOutput, CommandRunner, GcloudCommand};
use nuvem::services::runtime::session::RuntimeSession;
use nuvem_domain::gcp::configuration::{GcloudConfiguration, GcloudConfigurationName};
use nuvem_domain::gcp::resource::{
    CloudSqlAvailability, CloudSqlInstance, CloudSqlStatus, ComputeInstance, EndpointVisibility,
    ResourceServiceKind, ResourceStatus,
};

/// Deadline for every gate wait in this file. The tests release their gates
/// within milliseconds, so 30s sits orders of magnitude above any plausible
/// scheduling delay on a loaded CI runner: exceeding it proves the gate wedged
/// rather than that the machine was slow. The bound matters because these waits
/// run on `spawn_blocking` threads, where an unbounded park is uncancellable
/// and makes `Runtime::drop` hang forever when a test panics before releasing.
const GATE_TIMEOUT: Duration = Duration::from_secs(30);

fn output(stdout: impl Into<String>) -> Result<CommandOutput, CommandError> {
    Ok(CommandOutput {
        stdout: stdout.into(),
        stderr: String::new(),
    })
}

/// Whether `command` is a *compute* instance-list refresh. Matches on the
/// service and list verb plus the row cap rather than on the argv tail, because
/// a list that carries a pushed-down filter appends `--filter=...` after
/// `--limit=500`. `compute` is what keeps this from also matching a Cloud SQL
/// list (`sql instances list --format=json --limit=500`).
fn is_list_command(command: &GcloudCommand) -> bool {
    let has = |value: &str| command.args.iter().any(|arg| arg == value);
    has("compute") && has("instances") && has("list") && has("--limit=500")
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

/// Records action commands and returns a canned list payload, optionally
/// failing named targets. Backs the happy-path and partial-failure cases.
#[derive(Clone)]
struct RecordingRunner {
    state: Arc<RecordingState>,
}

struct RecordingState {
    list_output: String,
    fail_names: Vec<String>,
    action_commands: Mutex<Vec<GcloudCommand>>,
    list_commands: Mutex<Vec<GcloudCommand>>,
}

impl RecordingRunner {
    fn new(list_output: &str, fail_names: &[&str]) -> Self {
        Self {
            state: Arc::new(RecordingState {
                list_output: list_output.to_string(),
                fail_names: fail_names.iter().map(|name| name.to_string()).collect(),
                action_commands: Mutex::new(Vec::new()),
                list_commands: Mutex::new(Vec::new()),
            }),
        }
    }

    fn action_commands(&self) -> Vec<GcloudCommand> {
        self.state.action_commands.lock().unwrap().clone()
    }

    fn list_commands(&self) -> Vec<GcloudCommand> {
        self.state.list_commands.lock().unwrap().clone()
    }

    fn list_count(&self) -> usize {
        self.state.list_commands.lock().unwrap().len()
    }
}

impl CommandRunner for RecordingRunner {
    fn run(&self, command: GcloudCommand) -> Result<CommandOutput, CommandError> {
        if is_list_command(&command) {
            self.state.list_commands.lock().unwrap().push(command);
            return output(self.state.list_output.clone());
        }
        let fails = self
            .state
            .fail_names
            .iter()
            .any(|name| command.args.iter().any(|arg| arg == name));
        self.state.action_commands.lock().unwrap().push(command);
        if fails {
            return Err(CommandError::Failed {
                stdout: String::new(),
                stderr: "action failed".to_string(),
                exit_code: None,
            });
        }
        output("")
    }
}

/// Runs actions normally except it panics while executing a named target's
/// action command. Because `run` executes inside the batch task's
/// `spawn_blocking`, panicking here forces a real `JoinError` on `.await`,
/// exercising the same join-error path as a genuine blocking-task panic
/// rather than simulating it via a `CommandError`.
#[derive(Clone)]
struct PanicRunner {
    state: Arc<PanicState>,
}

struct PanicState {
    list_output: String,
    panic_names: Vec<String>,
    action_commands: Mutex<Vec<GcloudCommand>>,
    list_count: Mutex<usize>,
}

impl PanicRunner {
    fn new(list_output: &str, panic_names: &[&str]) -> Self {
        Self {
            state: Arc::new(PanicState {
                list_output: list_output.to_string(),
                panic_names: panic_names.iter().map(|name| name.to_string()).collect(),
                action_commands: Mutex::new(Vec::new()),
                list_count: Mutex::new(0),
            }),
        }
    }

    fn action_commands(&self) -> Vec<GcloudCommand> {
        self.state.action_commands.lock().unwrap().clone()
    }

    fn list_count(&self) -> usize {
        *self.state.list_count.lock().unwrap()
    }
}

impl CommandRunner for PanicRunner {
    fn run(&self, command: GcloudCommand) -> Result<CommandOutput, CommandError> {
        if is_list_command(&command) {
            *self.state.list_count.lock().unwrap() += 1;
            return output(self.state.list_output.clone());
        }
        let panics = self
            .state
            .panic_names
            .iter()
            .any(|name| command.args.iter().any(|arg| arg == name));
        self.state.action_commands.lock().unwrap().push(command);
        if panics {
            panic!("simulated action task panic");
        }
        output("")
    }
}

/// A runner that gates each action command behind an explicit release, so tests
/// can observe how many actions are in flight simultaneously.
#[derive(Clone)]
struct GateRunner {
    state: Arc<GateState>,
}

struct GateState {
    inner: Mutex<GateInner>,
    signal: Condvar,
    list_output: String,
    gate_timeout: Duration,
}

struct GateInner {
    started: usize,
    in_flight: usize,
    max_in_flight: usize,
    release_allowed: usize,
    action_commands: Vec<GcloudCommand>,
    /// Every list command the runner answered, in arrival order. Recorded so a
    /// test can tell *which* service's list refresh a completed batch fired,
    /// not merely how many ran.
    list_commands: Vec<GcloudCommand>,
}

impl GateRunner {
    fn new(list_output: &str) -> Self {
        Self::with_gate_timeout(list_output, GATE_TIMEOUT)
    }

    /// Same as [`GateRunner::new`] but with an injectable gate deadline, so the
    /// timeout tests can pin the bound without spending the full
    /// `GATE_TIMEOUT`.
    fn with_gate_timeout(list_output: &str, gate_timeout: Duration) -> Self {
        Self {
            state: Arc::new(GateState {
                inner: Mutex::new(GateInner {
                    started: 0,
                    in_flight: 0,
                    max_in_flight: 0,
                    release_allowed: 0,
                    action_commands: Vec::new(),
                    list_commands: Vec::new(),
                }),
                signal: Condvar::new(),
                list_output: list_output.to_string(),
                gate_timeout,
            }),
        }
    }

    /// Blocks until `count` actions have started, giving up at the gate
    /// deadline rather than parking forever.
    fn wait_until_started(&self, count: usize) {
        let deadline = Instant::now() + self.state.gate_timeout;
        let mut guard = self.state.inner.lock().unwrap();
        while guard.started < count {
            // Budget the remaining time against the single deadline so a
            // spurious wakeup cannot restart the full timeout.
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                // Format first, then release the lock: panicking with the guard
                // still held would poison the mutex and replace the sibling
                // waiters' own named diagnostics with `PoisonError`.
                let message = format!(
                    "GateRunner::wait_until_started timed out after {:?}: expected {count} started actions, observed {}",
                    self.state.gate_timeout, guard.started
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

    fn max_in_flight(&self) -> usize {
        self.state.inner.lock().unwrap().max_in_flight
    }

    fn action_command_count(&self) -> usize {
        self.state.inner.lock().unwrap().action_commands.len()
    }

    fn action_commands(&self) -> Vec<GcloudCommand> {
        self.state.inner.lock().unwrap().action_commands.clone()
    }

    fn list_commands(&self) -> Vec<GcloudCommand> {
        self.state.inner.lock().unwrap().list_commands.clone()
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
        // Every list command answers immediately, whatever the service: only
        // batch *actions* are gated. Gating a background list instead would
        // park a `spawn_blocking` thread nothing in the test ever releases.
        if is_list_command(&command) || is_cloud_sql_list_command(&command) {
            self.state.inner.lock().unwrap().list_commands.push(command);
            return output(self.state.list_output.clone());
        }
        let my_index = {
            let mut guard = self.state.inner.lock().unwrap();
            guard.action_commands.push(command);
            let index = guard.started;
            guard.started += 1;
            guard.in_flight += 1;
            if guard.in_flight > guard.max_in_flight {
                guard.max_in_flight = guard.in_flight;
            }
            self.state.signal.notify_all();
            index
        };
        // This wait runs on the batch task's `spawn_blocking` thread, where an
        // unbounded park is uncancellable and would wedge `Runtime::drop` for
        // good if a test panicked before releasing. Bound it and panic loudly:
        // a panicking blocking task surfaces as a `JoinError`, a hang does not.
        let deadline = Instant::now() + self.state.gate_timeout;
        let mut guard = self.state.inner.lock().unwrap();
        while guard.release_allowed <= my_index {
            // Budget the remaining time against the single deadline so a
            // spurious wakeup cannot restart the full timeout.
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                // Several gated actions share one deadline, so they expire
                // together. Format first, then release the lock: poisoning it
                // here would turn every sibling's `wait_timeout(..).unwrap()`
                // into a `PoisonError` panic and bury the real diagnostic.
                let message = format!(
                    "GateRunner::run timed out after {:?} waiting for release of target index {my_index}: release_allowed is {}",
                    self.state.gate_timeout, guard.release_allowed
                );
                drop(guard);
                panic!("{message}");
            }
            guard = self.state.signal.wait_timeout(guard, remaining).unwrap().0;
        }
        guard.in_flight -= 1;
        self.state.signal.notify_all();
        output("")
    }
}

/// Joins a gate waiter that is expected to give up, and returns the message it
/// panicked with. `gate_timeout` is the budget the caller injected into the
/// waiter; the poll ceiling is derived from it rather than hardcoded so the two
/// numbers cannot drift apart. 200x is the multiple: it matches the margin the
/// other bounds on this branch run at (250x-1000x) and lands at 10s for the
/// 50ms gate these tests inject, well clear of scheduling jitter on a loaded CI
/// runner. The loop exits the instant the waiter finishes, so a generous
/// ceiling costs nothing on the passing path -- only an unbounded wait, which
/// never finishes, pays it, and that trips the assertion below instead of
/// hanging the whole binary.
fn panic_message_of_bounded_wait(worker: thread::JoinHandle<()>, gate_timeout: Duration) -> String {
    let ceiling = gate_timeout * 200;
    let started = Instant::now();
    while !worker.is_finished() && started.elapsed() < ceiling {
        thread::sleep(Duration::from_millis(10));
    }
    assert!(
        worker.is_finished(),
        "a bounded gate wait must give up instead of parking forever"
    );
    let payload = worker.join().expect_err("a timed-out gate wait must panic");
    *payload
        .downcast::<String>()
        .expect("a gate timeout panic should carry a formatted message")
}

/// Both `GateRunner` waits run on `spawn_blocking` threads. An unbounded park
/// there is uncancellable, so a test that panics before releasing the gate
/// leaves `Runtime::drop` blocked on it forever and wedges the whole binary.
/// This pins the bound on the start gate: nothing ever starts an action, so the
/// waiter must give up and name what it was waiting for.
#[test]
fn gate_runner_wait_until_started_gives_up_when_no_action_starts() {
    let gate_timeout = Duration::from_millis(50);
    let runner = GateRunner::with_gate_timeout("", gate_timeout);

    let worker = thread::spawn(move || runner.wait_until_started(1));

    let message = panic_message_of_bounded_wait(worker, gate_timeout);
    assert!(
        message.contains("GateRunner::wait_until_started timed out"),
        "panic should name the waiter, got: {message}"
    );
    assert!(
        message.contains("expected 1") && message.contains("observed 0"),
        "panic should name the expected and observed counts, got: {message}"
    );
}

/// The release gate is the wait that actually wedges `Runtime::drop`: it runs
/// inside the batch task's `spawn_blocking`, so an unbounded park there
/// survives the test's unwind. Nothing ever calls `release`, so the runner must
/// give up and name the stuck target index.
#[test]
fn gate_runner_run_gives_up_when_the_action_is_never_released() {
    let gate_timeout = Duration::from_millis(50);
    let runner = GateRunner::with_gate_timeout("", gate_timeout);

    let worker = thread::spawn(move || {
        let _ = runner.run(GcloudCommand::new([
            "compute",
            "instances",
            "stop",
            "vm-a",
            "--zone",
            "europe-west1-b",
        ]));
    });

    let message = panic_message_of_bounded_wait(worker, gate_timeout);
    assert!(
        message.contains("GateRunner::run timed out"),
        "panic should name the waiter, got: {message}"
    );
    assert!(
        message.contains("target index 0"),
        "panic should name the stuck target index, got: {message}"
    );
}

/// A batch runs several actions at once, so when the gate stays shut every one
/// of their `spawn_blocking` threads expires on the same budget at nearly the
/// same moment. The bound only earns its keep if each of them says what it was
/// stuck on: an earlier version panicked while still holding the `MutexGuard`,
/// which poisoned `state.inner` and left the siblings unwrapping a
/// `PoisonError` instead - the diagnostic meant to explain a CI hang was
/// crowded out by noise from the first thread to give up. This pins that every
/// waiter reports its own stuck target and none is swallowed by the poison.
///
/// Nothing here depends on the threads expiring in any particular order, or on
/// them overlapping at all: each computes its own deadline when it reaches the
/// gate, and a budget that has already run out still takes the same named-panic
/// branch (`saturating_duration_since` clamps to zero rather than underflowing,
/// and the check runs before any `wait_timeout`). A thread that arrives late on
/// a stalled runner therefore fails the same way, not a different way.
#[test]
fn gate_runner_run_timeouts_do_not_poison_the_gate_for_siblings() {
    let gate_timeout = Duration::from_millis(50);
    let runner = GateRunner::with_gate_timeout("", gate_timeout);

    // Four concurrent actions, none of them ever released.
    let workers: Vec<_> = (0..4)
        .map(|_| {
            let runner = runner.clone();
            thread::spawn(move || {
                let _ = runner.run(GcloudCommand::new([
                    "compute",
                    "instances",
                    "stop",
                    "vm-a",
                    "--zone",
                    "europe-west1-b",
                ]));
            })
        })
        .collect();

    for worker in workers {
        let message = panic_message_of_bounded_wait(worker, gate_timeout);
        assert!(
            message.contains("GateRunner::run timed out"),
            "every waiter must report its own named diagnostic, got: {message}"
        );
        assert!(
            !message.contains("PoisonError"),
            "no waiter may be crowded out by a poisoned-mutex panic, got: {message}"
        );
    }

    // The gate is still lockable after four panics; a poisoned mutex would
    // instead panic inside this `lock().unwrap()`.
    assert_eq!(runner.started(), 4);
}

/// Builds an app with `instances` loaded under the dev context, every instance
/// marked, and a pending batch confirmation for `action`.
fn marked_batch_app(instances: Vec<ComputeInstance>, action: ComputeInstanceAction) -> AppState {
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
    if count > 0 {
        app.root.resources.toggle_mark();
        for _ in 1..count {
            app.root.resources.select_next();
            app.root.resources.toggle_mark();
        }
    }
    app.root
        .resources
        .start_batch_confirmation::<Compute>(context, action);
    app
}

async fn start_batch<R: CommandRunner + Send + Sync + 'static>(
    app: AppState,
    runner: R,
    config: Option<Config>,
) -> RuntimeSession {
    let mut session = match config {
        Some(config) => {
            RuntimeSession::new_with_config_and_runner_for_test(config, app, Box::new(runner))
        }
        None => RuntimeSession::new_with_runner_for_test(app, Box::new(runner)),
    };
    session
        .dispatch_action(Action::Resources(ResourcesAction::ConfirmBatchAction))
        .await;
    session
}

/// Extracts the `(verb, name)` pair from an action command shaped like
/// `... compute instances <verb> <name> --zone <zone>`.
fn action_verb_and_name(command: &GcloudCommand) -> (String, String) {
    let zone_index = command
        .args
        .iter()
        .position(|arg| arg == "--zone")
        .expect("--zone present");
    (
        command.args[zone_index - 2].clone(),
        command.args[zone_index - 1].clone(),
    )
}

#[tokio::test]
async fn batch_stop_runs_each_target_and_refreshes_once() {
    let runner = RecordingRunner::new(include_str!("fixtures/gce_instances.json"), &[]);
    let app = marked_batch_app(
        vec![instance("vm-a"), instance("vm-b")],
        ComputeInstanceAction::Stop,
    );
    let mut session = start_batch(app, runner.clone(), None).await;

    // Two per-target results complete the workflow.
    assert!(session.process_next_effect_result().await);
    assert!(session.process_next_effect_result().await);

    let workflow = session
        .app()
        .root
        .resources
        .batch_workflow()
        .expect("workflow present");
    assert_eq!(workflow.ok_count(), 2);
    assert_eq!(workflow.fail_count(), 0);
    assert!(workflow.is_complete());

    // The single follow-up list refresh applies and dismisses the now-complete
    // batch-progress workflow.
    assert!(session.process_next_effect_result().await);
    assert!(session.app().root.resources.batch_workflow().is_none());

    let succeeded = session
        .app()
        .root
        .system
        .operations
        .records()
        .iter()
        .filter(|record| record.outcome == OperationOutcome::Succeeded)
        .count();
    assert_eq!(succeeded, 2);

    // argv: exactly two `stop <name> --zone ...` commands, and one list refresh.
    let mut pairs: Vec<(String, String)> = runner
        .action_commands()
        .iter()
        .map(action_verb_and_name)
        .collect();
    pairs.sort();
    assert_eq!(
        pairs,
        vec![
            ("stop".to_string(), "vm-a".to_string()),
            ("stop".to_string(), "vm-b".to_string()),
        ]
    );
    assert_eq!(runner.list_count(), 1);
}

/// The end-of-batch refresh reloads the active service, so it must carry the
/// applied filter. Label mode narrows only through the pushed `--filter`
/// (`CompiledFilter::Label` keeps every loaded row), so an unfiltered reload
/// here would replace the operator's `-l env=prod` table with the whole
/// project the instant a batch Stop finished — while the tools line still
/// claims the label filter is in force.
#[tokio::test]
async fn completed_batch_refresh_carries_the_active_label_filter() {
    let runner = RecordingRunner::new(include_str!("fixtures/gce_instances.json"), &[]);
    let mut app = marked_batch_app(
        vec![instance("vm-a"), instance("vm-b")],
        ComputeInstanceAction::Stop,
    );
    // Applying a label filter keeps every loaded row visible client-side, so
    // the marks made by `marked_batch_app` survive it.
    reduce(&mut app, &Action::Resources(ResourcesAction::StartFilter));
    for value in "-l env=prod".chars() {
        reduce(
            &mut app,
            &Action::Resources(ResourcesAction::AppendFilterChar(value)),
        );
    }
    reduce(&mut app, &Action::Resources(ResourcesAction::ApplyFilter));

    let mut session = start_batch(app, runner.clone(), None).await;
    assert!(session.process_next_effect_result().await);
    assert!(session.process_next_effect_result().await);
    assert!(session.process_next_effect_result().await);

    let lists = runner.list_commands();
    let refresh = lists.last().expect("the batch must end with one refresh");
    assert!(
        refresh
            .args
            .contains(&r#"--filter=labels.env = "prod""#.to_string()),
        "the end-of-batch refresh must carry the active label filter, got {:?}",
        refresh.args
    );
}

#[tokio::test]
async fn batch_respects_bounded_concurrency() {
    let runner = GateRunner::new(include_str!("fixtures/gce_instances.json"));
    let app = marked_batch_app(
        vec![instance("vm-a"), instance("vm-b")],
        ComputeInstanceAction::Stop,
    );
    let session_config = Config {
        batch_concurrency: 1,
        ..Config::default()
    };
    let mut session = start_batch(app, runner.clone(), Some(session_config)).await;

    // Only the first target may start; the second is parked on the semaphore.
    let waiter = runner.clone();
    tokio::task::spawn_blocking(move || waiter.wait_until_started(1))
        .await
        .unwrap();
    assert_eq!(runner.started(), 1);
    assert_eq!(runner.max_in_flight(), 1);

    // Releasing the first lets the second start; still only one at a time.
    runner.release(1);
    let waiter = runner.clone();
    tokio::task::spawn_blocking(move || waiter.wait_until_started(2))
        .await
        .unwrap();
    assert_eq!(runner.max_in_flight(), 1);
    runner.release(2);

    assert!(session.process_next_effect_result().await);
    assert!(session.process_next_effect_result().await);

    assert_eq!(runner.max_in_flight(), 1);
    let workflow = session
        .app()
        .root
        .resources
        .batch_workflow()
        .expect("workflow present");
    assert_eq!(workflow.ok_count(), 2);
    assert!(workflow.is_complete());

    // The follow-up list refresh applies and dismisses the completed workflow.
    assert!(session.process_next_effect_result().await);
    assert!(session.app().root.resources.batch_workflow().is_none());
}

#[tokio::test]
async fn batch_partial_failure_does_not_abort_remaining_targets() {
    let runner = RecordingRunner::new(include_str!("fixtures/gce_instances.json"), &["vm-b"]);
    let app = marked_batch_app(
        vec![instance("vm-a"), instance("vm-b")],
        ComputeInstanceAction::Stop,
    );
    let mut session = start_batch(app, runner.clone(), None).await;

    assert!(session.process_next_effect_result().await);
    assert!(session.process_next_effect_result().await);

    let workflow = session
        .app()
        .root
        .resources
        .batch_workflow()
        .expect("workflow present");
    assert_eq!(workflow.ok_count(), 1);
    assert_eq!(workflow.fail_count(), 1);
    assert!(workflow.is_complete());

    // The follow-up list refresh applies and dismisses the completed workflow.
    assert!(session.process_next_effect_result().await);
    assert!(session.app().root.resources.batch_workflow().is_none());

    // Both targets ran: the failure of one did not abort the other.
    assert_eq!(runner.action_commands().len(), 2);

    let records = session.app().root.system.operations.records();
    assert_eq!(
        records
            .iter()
            .filter(|record| record.outcome == OperationOutcome::Succeeded)
            .count(),
        1
    );
    assert_eq!(
        records
            .iter()
            .filter(|record| record.outcome == OperationOutcome::Failed)
            .count(),
        1
    );
}

/// When a batch target's action task panics, the join in
/// `dispatch_resource_batch` must still resolve that target instead
/// of silently dropping it. Without the fix, the target's batch-progress
/// entry and operations-log entry would stay `Pending` forever, the workflow
/// would never reach `is_complete()`, and the single follow-up
/// `RefreshComputeInstances` would never fire. This forces a real join panic
/// (not just a `CommandError`) by panicking inside the runner, which executes
/// on the batch task's `spawn_blocking` thread.
#[tokio::test]
async fn batch_target_resolves_as_failed_when_action_task_panics() {
    let runner = PanicRunner::new(include_str!("fixtures/gce_instances.json"), &["vm-b"]);
    let app = marked_batch_app(
        vec![instance("vm-a"), instance("vm-b")],
        ComputeInstanceAction::Stop,
    );
    let mut session = start_batch(app, runner.clone(), None).await;

    // Two per-target results complete the workflow.
    assert!(session.process_next_effect_result().await);
    assert!(session.process_next_effect_result().await);

    let workflow = session
        .app()
        .root
        .resources
        .batch_workflow()
        .expect("workflow present");
    assert_eq!(workflow.ok_count(), 1);
    assert_eq!(workflow.fail_count(), 1);
    assert!(workflow.is_complete());

    // The single follow-up list refresh applies and dismisses the completed
    // workflow.
    assert!(session.process_next_effect_result().await);
    assert!(session.app().root.resources.batch_workflow().is_none());

    let records = session.app().root.system.operations.records();
    assert_eq!(
        records
            .iter()
            .filter(|record| record.outcome == OperationOutcome::Pending)
            .count(),
        0,
        "no operations-log entry should be stranded at Pending"
    );
    assert_eq!(
        records
            .iter()
            .filter(|record| record.outcome == OperationOutcome::Succeeded)
            .count(),
        1
    );
    assert_eq!(
        records
            .iter()
            .filter(|record| record.outcome == OperationOutcome::Failed)
            .count(),
        1
    );

    // Both targets ran; the panicking target did not abort the other.
    assert_eq!(runner.action_commands().len(), 2);
    // The follow-up refresh fired exactly once, proving the workflow completed.
    assert_eq!(runner.list_count(), 1);
}

#[tokio::test]
async fn batch_results_are_dropped_after_switching_configuration() {
    let runner = GateRunner::new(include_str!("fixtures/gce_instances.json"));
    let app = marked_batch_app(
        vec![instance("vm-a"), instance("vm-b")],
        ComputeInstanceAction::Stop,
    );
    let session_config = Config {
        batch_concurrency: 1,
        ..Config::default()
    };
    let mut session = start_batch(app, runner.clone(), Some(session_config)).await;

    let waiter = runner.clone();
    tokio::task::spawn_blocking(move || waiter.wait_until_started(1))
        .await
        .unwrap();

    // Switch the active configuration while the first target is still running.
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
    let prod_context = session
        .app()
        .root
        .current_context()
        .expect("context")
        .clone();
    session
        .apply_effect_result(EffectResult::ComputeInstancesLoaded {
            context: prod_context,
            instances: vec![instance("prod-vm")],
        })
        .await;
    let rows_before = session.app().root.resources.instances().to_vec();

    // The workflow is detached by the context switch.
    assert!(session.app().root.resources.batch_workflow().is_none());

    // Release the in-flight target; its late result must be dropped.
    runner.release(2);
    assert!(session.process_next_effect_result().await);

    assert_eq!(session.app().root.resources.instances(), rows_before);
    assert!(session.app().root.resources.batch_workflow().is_none());
    // The second target was cancelled before it could start.
    assert_eq!(runner.started(), 1);
    assert_eq!(runner.action_command_count(), 1);
}

#[tokio::test]
async fn batch_does_not_start_pending_targets_after_context_switch() {
    let runner = GateRunner::new("");
    let app = marked_batch_app(
        vec![instance("vm-a"), instance("vm-b")],
        ComputeInstanceAction::Stop,
    );
    let session_config = Config {
        batch_concurrency: 1,
        ..Config::default()
    };
    let mut session = start_batch(app, runner.clone(), Some(session_config)).await;

    let waiter = runner.clone();
    tokio::task::spawn_blocking(move || waiter.wait_until_started(1))
        .await
        .unwrap();

    // A context switch cancels the batch.
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

    runner.release(2);
    // The first target's late result arrives and is dropped by the guard.
    assert!(session.process_next_effect_result().await);

    // Exactly one action ran; the not-yet-started target never executed.
    assert_eq!(runner.started(), 1);
    assert_eq!(runner.action_command_count(), 1);
}

#[tokio::test]
async fn batch_workflow_is_cleared_after_scope_switch() {
    let runner = GateRunner::new("");
    let app = marked_batch_app(
        vec![instance("vm-a"), instance("vm-b")],
        ComputeInstanceAction::Stop,
    );
    let session_config = Config {
        batch_concurrency: 1,
        ..Config::default()
    };
    let mut session = start_batch(app, runner.clone(), Some(session_config)).await;

    let waiter = runner.clone();
    tokio::task::spawn_blocking(move || waiter.wait_until_started(1))
        .await
        .unwrap();

    // The batch-progress workflow is live while the first target runs.
    assert!(session.app().root.resources.batch_workflow().is_some());

    // Switching workspace is a scope change that detaches the batch; the
    // render-only progress panel must not strand over the new view.
    session
        .dispatch_action(Action::Workspace(WorkspaceAction::Configurations))
        .await;

    assert!(session.app().root.resources.batch_workflow().is_none());

    runner.release(2);
    // The first target's late result arrives and is dropped by the guard.
    assert!(session.process_next_effect_result().await);

    assert!(session.app().root.resources.batch_workflow().is_none());
    // The second target was cancelled before it could start.
    assert_eq!(runner.started(), 1);
    assert_eq!(runner.action_command_count(), 1);
}

/// Detail carried by the entry of a target that never issued its command.
const CANCELLED_DETAIL: &str = "the batch was cancelled before this command ran";

/// Detail carried by the entry of a target whose command was already in flight.
/// It must point the operator at their next move without naming a key: the
/// refresh binding is user-rebindable, so a hardcoded key would be a lie.
const UNKNOWN_DETAIL: &str = "the batch was cancelled after this command was issued; refresh this service to see the resource's actual state";

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

fn pending_operation_count(session: &RuntimeSession) -> usize {
    session.app().root.system.operations.pending_count()
}

/// Names of the Compute targets whose action command actually reached the
/// runner, i.e. the ones GCP may already be acting on.
fn started_compute_targets(runner: &GateRunner) -> Vec<String> {
    runner
        .action_commands()
        .iter()
        .map(|command| action_verb_and_name(command).1)
        .collect()
}

/// Cancelling a batch must settle every entry it seeded, and settle each one at
/// what we can actually prove: the target whose command was already issued is
/// `Unknown` (GCP may well be carrying it out), the target that never got past
/// the cancellation check is `Cancelled`. Neither may stay `Pending`, which is
/// the defect this pins.
#[tokio::test]
async fn cancelled_batch_resolves_started_unknown_and_unstarted_cancelled() {
    let runner = GateRunner::new("");
    let app = marked_batch_app(
        vec![instance("vm-a"), instance("vm-b")],
        ComputeInstanceAction::Stop,
    );
    let session_config = Config {
        batch_concurrency: 1,
        ..Config::default()
    };
    let mut session = start_batch(app, runner.clone(), Some(session_config)).await;

    let waiter = runner.clone();
    tokio::task::spawn_blocking(move || waiter.wait_until_started(1))
        .await
        .unwrap();

    // Which target won the single permit is the scheduler's business; the
    // runner knows which command it actually answered.
    let started = started_compute_targets(&runner);
    assert_eq!(started.len(), 1, "exactly one target should be in flight");
    let started_name = started[0].clone();
    let unstarted_name = if started_name == "vm-a" {
        "vm-b"
    } else {
        "vm-a"
    };

    // Switch the active configuration while the first target is still running.
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

    assert_eq!(
        pending_operation_count(&session),
        0,
        "a cancelled batch must not strand any operations-log entry Pending"
    );
    assert_eq!(
        operation_for(&session, &started_name),
        (OperationOutcome::Unknown, Some(UNKNOWN_DETAIL.to_string())),
        "the in-flight target's command was already issued"
    );
    assert_eq!(
        operation_for(&session, unstarted_name),
        (
            OperationOutcome::Cancelled,
            Some(CANCELLED_DETAIL.to_string())
        ),
        "the unstarted target issued no command at all"
    );

    // Drain the gated target so the runtime has nothing parked on it. The
    // target reported `Cancelled` must still have issued no command by the time
    // everything has settled — an entry that says "the command did not run" is
    // only true if no command ever runs, including afterwards.
    runner.release(2);
    assert!(session.process_next_effect_result().await);
    assert_eq!(runner.started(), 1);
    assert_eq!(
        started_compute_targets(&runner),
        vec![started_name],
        "no command may be issued for a target reported Cancelled"
    );
}

/// G2 exclusion pin. A worker and the cancellation cannot both claim the same
/// target, so a target whose command is in flight can never be reported
/// `Cancelled` — reporting "the command did not run" while GCP tears the
/// instance down is the contradiction this ticket exists to remove. The pin
/// holds regardless of how the two threads interleave: it is the claim being
/// exclusive, not any ordering between separate reads and writes, that makes
/// the `Unknown` verdict the only one available once a command is on its way.
/// The late result that arrives when the gate opens is dropped by the
/// generation guard and must not rewrite the settled entry either.
#[tokio::test]
async fn in_flight_batch_target_is_never_reported_cancelled() {
    let runner = GateRunner::new("");
    let app = marked_batch_app(
        vec![instance("vm-a"), instance("vm-b")],
        ComputeInstanceAction::Stop,
    );
    let session_config = Config {
        batch_concurrency: 1,
        ..Config::default()
    };
    let mut session = start_batch(app, runner.clone(), Some(session_config)).await;

    let waiter = runner.clone();
    tokio::task::spawn_blocking(move || waiter.wait_until_started(1))
        .await
        .unwrap();
    let started_name = started_compute_targets(&runner)
        .pop()
        .expect("one target in flight");

    session
        .dispatch_action(Action::Workspace(WorkspaceAction::Configurations))
        .await;

    let settled = operation_for(&session, &started_name);
    assert_eq!(
        settled,
        (OperationOutcome::Unknown, Some(UNKNOWN_DETAIL.to_string())),
        "a target whose command is in flight must never be reported Cancelled"
    );

    // Its command completes after the cancellation; the stale result is dropped
    // and the settled entry stays exactly as it was.
    runner.release(2);
    assert!(session.process_next_effect_result().await);
    assert_eq!(operation_for(&session, &started_name), settled);
    assert_eq!(pending_operation_count(&session), 0);
}

/// Re-activating the configuration that is *already* active resolves
/// to the same context, so the reducer reports no context change and the
/// cancellation never runs — but the reducer's own teardown
/// (`mark_all_stale_and_clear`) still nulls the batch-progress workflow. A
/// later scope switch then cancels a batch whose workflow is already gone. As
/// long as the operations-log ids live only on that workflow, they vanish with
/// it and every entry the batch seeded stays `Pending` for the life of the
/// process. Holding each target's id on the session instead is what makes the
/// sweep survive this sequence.
#[tokio::test]
async fn batch_entries_settle_when_the_workflow_was_torn_down_before_cancellation() {
    let runner = GateRunner::new("");
    let app = marked_batch_app(
        vec![instance("vm-a"), instance("vm-b")],
        ComputeInstanceAction::Stop,
    );
    let session_config = Config {
        batch_concurrency: 1,
        ..Config::default()
    };
    let mut session = start_batch(app, runner.clone(), Some(session_config)).await;

    let waiter = runner.clone();
    tokio::task::spawn_blocking(move || waiter.wait_until_started(1))
        .await
        .unwrap();
    let started_name = started_compute_targets(&runner)
        .pop()
        .expect("one target in flight");
    let unstarted_name = if started_name == "vm-a" {
        "vm-b"
    } else {
        "vm-a"
    };

    // Re-activating the already-active configuration: same context out, so the
    // reducer takes no cancelling branch, yet the workflow is torn down.
    session
        .apply_effect_result(EffectResult::ConfigurationActivated {
            configuration: config(),
        })
        .await;
    assert!(
        session.app().root.resources.batch_workflow().is_none(),
        "the reducer's context teardown nulls the workflow without cancelling"
    );

    // Now a scope switch cancels the batch, with no workflow left to read.
    session
        .dispatch_action(Action::Resources(ResourcesAction::SelectService(
            ResourceServiceKind::CloudSql,
        )))
        .await;

    assert_eq!(
        pending_operation_count(&session),
        0,
        "a cancelled batch must settle every entry it seeded, even when the workflow was torn down first"
    );
    assert_eq!(
        operation_for(&session, &started_name),
        (OperationOutcome::Unknown, Some(UNKNOWN_DETAIL.to_string())),
        "the in-flight target's command was already issued"
    );
    assert_eq!(
        operation_for(&session, unstarted_name),
        (
            OperationOutcome::Cancelled,
            Some(CANCELLED_DETAIL.to_string())
        ),
        "the unstarted target issued no command at all"
    );

    // Drain the gated target; its late result is dropped and must not reopen or
    // rewrite anything that has already been settled.
    runner.release(2);
    assert!(session.process_next_effect_result().await);
    assert_eq!(pending_operation_count(&session), 0);
    assert_eq!(
        started_compute_targets(&runner),
        vec![started_name],
        "no command may be issued for a target reported Cancelled"
    );
}

/// The cancellation sweep walks *every* target of the batch, not only the ones
/// still Pending, and the single thing that makes that safe is
/// `OperationsLog::resolve_by_id`'s pending-only guard: an entry that already
/// carries a proven outcome is left exactly as it is. Drop that guard and a
/// target the operator was already told had succeeded would be rewritten to
/// `Cancelled` — "the command did not run" — while GCP has in fact run it.
#[tokio::test]
async fn cancellation_leaves_an_already_settled_batch_target_untouched() {
    let runner = GateRunner::new("");
    let app = marked_batch_app(
        vec![instance("vm-a"), instance("vm-b"), instance("vm-c")],
        ComputeInstanceAction::Stop,
    );
    let session_config = Config {
        batch_concurrency: 1,
        ..Config::default()
    };
    let mut session = start_batch(app, runner.clone(), Some(session_config)).await;

    // Run the first target to completion and land its real result, so its entry
    // is settled with its true outcome before any cancellation happens.
    let waiter = runner.clone();
    tokio::task::spawn_blocking(move || waiter.wait_until_started(1))
        .await
        .unwrap();
    let settled_name = started_compute_targets(&runner)
        .pop()
        .expect("one target in flight");
    runner.release(1);
    assert!(session.process_next_effect_result().await);
    let settled = operation_for(&session, &settled_name);
    assert_eq!(
        settled,
        (
            OperationOutcome::Succeeded,
            Some("Stop accepted".to_string())
        ),
        "the first target's own result settles its entry"
    );

    // The freed permit carries the second target's command to the runner; the
    // third never gets a permit at all.
    let waiter = runner.clone();
    tokio::task::spawn_blocking(move || waiter.wait_until_started(2))
        .await
        .unwrap();
    let started = started_compute_targets(&runner);
    let in_flight_name = started[1].clone();
    let unstarted_name = ["vm-a", "vm-b", "vm-c"]
        .into_iter()
        .find(|name| !started.iter().any(|started| started == name))
        .expect("one target never started");

    session
        .dispatch_action(Action::Workspace(WorkspaceAction::Configurations))
        .await;

    assert_eq!(
        operation_for(&session, &settled_name),
        settled,
        "the sweep must not overwrite an entry that already carries its true outcome"
    );
    assert_eq!(
        operation_for(&session, &in_flight_name),
        (OperationOutcome::Unknown, Some(UNKNOWN_DETAIL.to_string())),
        "the target whose command was in flight can only be reported Unknown"
    );
    assert_eq!(
        operation_for(&session, unstarted_name),
        (
            OperationOutcome::Cancelled,
            Some(CANCELLED_DETAIL.to_string())
        ),
        "the target that never got a permit issued no command"
    );
    assert_eq!(pending_operation_count(&session), 0);

    // Drain the gated target so nothing is parked on the gate; its late result
    // is dropped by the generation guard and rewrites nothing either.
    runner.release(3);
    assert!(session.process_next_effect_result().await);
    assert_eq!(operation_for(&session, &settled_name), settled);
}

#[tokio::test]
async fn completed_batch_workflow_is_cleared_when_follow_up_refresh_fails() {
    let runner = RecordingRunner::new(include_str!("fixtures/gce_instances.json"), &[]);
    let app = marked_batch_app(
        vec![instance("vm-a"), instance("vm-b")],
        ComputeInstanceAction::Stop,
    );
    let mut session = start_batch(app, runner.clone(), None).await;

    // Two per-target results complete the workflow.
    assert!(session.process_next_effect_result().await);
    assert!(session.process_next_effect_result().await);

    let workflow = session
        .app()
        .root
        .resources
        .batch_workflow()
        .expect("workflow present");
    assert!(workflow.is_complete());

    // The single follow-up refresh fails; the completed batch-progress panel
    // must still be dismissed rather than stranded permanently.
    let context = session
        .app()
        .root
        .current_context()
        .expect("context")
        .clone();
    session
        .apply_effect_result(EffectResult::ComputeInstancesLoadFailed {
            context,
            error: AppError::gcloud("refresh failed".to_string()),
        })
        .await;

    assert!(session.app().root.resources.batch_workflow().is_none());
}

// ---------------------------------------------------------------------------
// Cloud SQL
//
// The same bounded fan-out, driven through `BatchAction::CloudSql`. These pin
// that every confirmed Cloud SQL target actually reaches a runner and resolves:
// the runner selection for this arm used to be `unreachable!()`.
// ---------------------------------------------------------------------------

const CLOUD_SQL_INSTANCES: &str = include_str!("fixtures/cloudsql_instances.json");

fn is_cloud_sql_list_command(command: &GcloudCommand) -> bool {
    command
        .args
        .windows(3)
        .any(|window| window == ["sql", "instances", "list"])
}

/// Target instance name of a `sql instances <verb> <name> [flags…]` action
/// command. The name is the argument right after the verb, which is *not*
/// always the last one: `Start`/`Stop` are `instances patch <name>
/// --activation-policy=…`, so reading `args.last()` would silently start
/// asserting on the policy flag if the action under test changed.
fn cloud_sql_action_target(command: &GcloudCommand) -> String {
    let verb_index = command
        .args
        .windows(2)
        .position(|window| window == ["sql", "instances"])
        .map(|index| index + 2)
        .unwrap_or_else(|| {
            panic!("not a Cloud SQL instances command: {:?}", command.args);
        });
    command
        .args
        .get(verb_index + 1)
        .cloned()
        .unwrap_or_else(|| panic!("no target name in: {:?}", command.args))
}

/// Cloud SQL twin of [`RecordingRunner`]: answers `sql instances list` with the
/// fixture, records every action command, and fails the named targets.
#[derive(Clone)]
struct CloudSqlRecordingRunner {
    state: Arc<RecordingState>,
}

impl CloudSqlRecordingRunner {
    fn new(fail_names: &[&str]) -> Self {
        Self {
            state: Arc::new(RecordingState {
                list_output: CLOUD_SQL_INSTANCES.to_string(),
                fail_names: fail_names.iter().map(|name| name.to_string()).collect(),
                action_commands: Mutex::new(Vec::new()),
                list_commands: Mutex::new(Vec::new()),
            }),
        }
    }

    fn action_commands(&self) -> Vec<GcloudCommand> {
        self.state.action_commands.lock().unwrap().clone()
    }

    fn list_count(&self) -> usize {
        self.state.list_commands.lock().unwrap().len()
    }
}

impl CommandRunner for CloudSqlRecordingRunner {
    fn run(&self, command: GcloudCommand) -> Result<CommandOutput, CommandError> {
        if is_cloud_sql_list_command(&command) {
            self.state.list_commands.lock().unwrap().push(command);
            return output(self.state.list_output.clone());
        }
        let fails = self
            .state
            .fail_names
            .iter()
            .any(|name| command.args.iter().any(|arg| arg == name));
        self.state.action_commands.lock().unwrap().push(command);
        if fails {
            return Err(CommandError::Failed {
                stdout: String::new(),
                stderr: "action failed".to_string(),
                exit_code: None,
            });
        }
        output("")
    }
}

fn cloud_sql_instance(name: &str) -> CloudSqlInstance {
    CloudSqlInstance {
        name: name.to_string(),
        database_version: "POSTGRES_15".to_string(),
        engine: "PostgreSQL".to_string(),
        region: "europe-west1".to_string(),
        tier: "db-custom-2-7680".to_string(),
        availability: CloudSqlAvailability::Regional,
        status: CloudSqlStatus::Runnable,
        connectivity: EndpointVisibility::Private,
    }
}

/// Cloud SQL twin of [`marked_batch_app`].
fn marked_cloud_sql_batch_app(
    instances: Vec<CloudSqlInstance>,
    action: CloudSqlInstanceAction,
) -> AppState {
    let mut app = AppState::new();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ConfigurationsLoaded {
            configurations: vec![config()],
        }),
    );
    reduce(
        &mut app,
        &Action::Resources(ResourcesAction::SelectService(
            ResourceServiceKind::CloudSql,
        )),
    );
    let context = app.root.current_context().expect("context").clone();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::CloudSqlInstancesLoaded {
            context: context.clone(),
            instances,
        }),
    );
    let count = app.root.resources.visible_cloud_sql_instances().len();
    if count > 0 {
        app.root.resources.toggle_mark();
        for _ in 1..count {
            app.root.resources.select_next();
            app.root.resources.toggle_mark();
        }
    }
    assert_eq!(app.root.resources.marked_count(), count);
    app.root
        .resources
        .start_batch_confirmation::<CloudSql>(context, action);
    app
}

#[tokio::test]
async fn cloud_sql_batch_restart_runs_each_target_and_refreshes_once() {
    let runner = CloudSqlRecordingRunner::new(&[]);
    let app = marked_cloud_sql_batch_app(
        vec![cloud_sql_instance("orders"), cloud_sql_instance("reports")],
        CloudSqlInstanceAction::Restart,
    );
    let mut session = start_batch(app, runner.clone(), None).await;

    // Two per-target results complete the workflow.
    assert!(session.process_next_effect_result().await);
    assert!(session.process_next_effect_result().await);

    let workflow = session
        .app()
        .root
        .resources
        .batch_workflow()
        .expect("workflow present");
    assert_eq!(workflow.ok_count(), 2);
    assert_eq!(workflow.fail_count(), 0);
    assert!(workflow.is_complete());

    // The single follow-up Cloud SQL refresh dismisses the completed workflow.
    assert!(session.process_next_effect_result().await);
    assert!(session.app().root.resources.batch_workflow().is_none());

    let records = session.app().root.system.operations.records();
    assert_eq!(
        records
            .iter()
            .filter(|record| record.outcome == OperationOutcome::Pending)
            .count(),
        0,
        "no operations-log entry should be stranded at Pending"
    );
    assert_eq!(
        records
            .iter()
            .filter(|record| record.outcome == OperationOutcome::Succeeded)
            .count(),
        2
    );

    // argv: exactly two `sql instances restart <name>` commands.
    let mut commands: Vec<Vec<String>> = runner
        .action_commands()
        .iter()
        .map(|command| command.args.clone())
        .collect();
    commands.sort();
    assert_eq!(
        commands,
        vec![
            vec![
                "--configuration".to_string(),
                "dev".to_string(),
                "sql".to_string(),
                "instances".to_string(),
                "restart".to_string(),
                "orders".to_string(),
            ],
            vec![
                "--configuration".to_string(),
                "dev".to_string(),
                "sql".to_string(),
                "instances".to_string(),
                "restart".to_string(),
                "reports".to_string(),
            ],
        ]
    );
    assert_eq!(runner.list_count(), 1);
}

#[tokio::test]
async fn cloud_sql_batch_partial_failure_does_not_abort_remaining_targets() {
    let runner = CloudSqlRecordingRunner::new(&["reports"]);
    let app = marked_cloud_sql_batch_app(
        vec![cloud_sql_instance("orders"), cloud_sql_instance("reports")],
        CloudSqlInstanceAction::Stop,
    );
    let mut session = start_batch(app, runner.clone(), None).await;

    assert!(session.process_next_effect_result().await);
    assert!(session.process_next_effect_result().await);

    let workflow = session
        .app()
        .root
        .resources
        .batch_workflow()
        .expect("workflow present");
    assert_eq!(workflow.ok_count(), 1);
    assert_eq!(workflow.fail_count(), 1);
    assert!(workflow.is_complete());

    assert!(session.process_next_effect_result().await);
    assert!(session.app().root.resources.batch_workflow().is_none());

    // Both targets ran: the failure of one did not abort the other.
    assert_eq!(runner.action_commands().len(), 2);
    let records = session.app().root.system.operations.records();
    assert_eq!(
        records
            .iter()
            .filter(|record| record.outcome == OperationOutcome::Failed)
            .count(),
        1
    );
}

/// The whole Cloud SQL fan-out through a single gated runner, end to end: the
/// batch never exceeds the session's configured concurrency, every confirmed
/// target reaches the runner and resolves into the workflow, and exactly one
/// completion refresh fires once the last target resolves — a Cloud SQL list,
/// not the Compute one.
///
/// The refresh assertion is the part the other Cloud SQL cases cannot make.
/// `apply_batch_effect_result` derives the completion refresh from the live
/// workflow's `action()`; a runner that only counts its own
/// service's lists cannot distinguish "no refresh" from "the wrong service's
/// refresh". Recording the argv of every list the runner answered pins the
/// service, and the count pins that the refresh fires once rather than once per
/// resolved target.
///
/// Three targets against a limit of two also pin the bound with a target that
/// must wait for a permit that a *completed* peer released, which two-target,
/// limit-one cases do not reach.
///
/// Every gate wait here is bounded by `GATE_TIMEOUT` and released explicitly by
/// the test; nothing asserts on elapsed wall-clock time.
#[tokio::test]
async fn cloud_sql_batch_fans_out_bounded_and_refreshes_cloud_sql_once() {
    let runner = GateRunner::new(CLOUD_SQL_INSTANCES);
    let app = marked_cloud_sql_batch_app(
        vec![
            cloud_sql_instance("orders"),
            cloud_sql_instance("reports"),
            cloud_sql_instance("ledger"),
        ],
        CloudSqlInstanceAction::Restart,
    );
    let session_config = Config {
        batch_concurrency: 2,
        ..Config::default()
    };
    let mut session = start_batch(app, runner.clone(), Some(session_config)).await;

    // Two targets hold the only two permits; the third is parked on the
    // semaphore instead of running alongside them.
    let waiter = runner.clone();
    tokio::task::spawn_blocking(move || waiter.wait_until_started(2))
        .await
        .unwrap();
    assert_eq!(runner.started(), 2);
    assert_eq!(runner.max_in_flight(), 2);
    assert_eq!(
        runner.action_command_count(),
        2,
        "the third target must not reach the runner while both permits are held"
    );

    // Releasing both frees the permits the third target is waiting on; still
    // never three actions at once.
    runner.release(2);
    let waiter = runner.clone();
    tokio::task::spawn_blocking(move || waiter.wait_until_started(3))
        .await
        .unwrap();
    assert_eq!(runner.max_in_flight(), 2);
    runner.release(3);

    // Each target resolves into the workflow.
    assert!(session.process_next_effect_result().await);
    assert!(session.process_next_effect_result().await);
    assert!(session.process_next_effect_result().await);

    let workflow = session
        .app()
        .root
        .resources
        .batch_workflow()
        .expect("workflow present");
    assert_eq!(workflow.ok_count(), 3);
    assert_eq!(workflow.fail_count(), 0);
    assert!(workflow.is_complete());
    assert_eq!(runner.max_in_flight(), 2);

    // The one completion refresh applies and dismisses the workflow.
    assert!(session.process_next_effect_result().await);
    assert!(session.app().root.resources.batch_workflow().is_none());

    let list_commands = runner.list_commands();
    assert_eq!(
        list_commands.len(),
        1,
        "exactly one completion refresh, not one per resolved target: {:?}",
        list_commands
            .iter()
            .map(|command| command.args.clone())
            .collect::<Vec<_>>()
    );
    assert!(
        is_cloud_sql_list_command(&list_commands[0]),
        "the completion refresh must reload Cloud SQL, got: {:?}",
        list_commands[0].args
    );

    // Every confirmed target reached the runner, exactly once each.
    let mut names: Vec<String> = runner
        .action_commands()
        .iter()
        .map(cloud_sql_action_target)
        .collect();
    names.sort();
    assert_eq!(names, vec!["ledger", "orders", "reports"]);

    let records = session.app().root.system.operations.records();
    assert_eq!(
        records
            .iter()
            .filter(|record| record.outcome == OperationOutcome::Pending)
            .count(),
        0,
        "no operations-log entry should be stranded at Pending"
    );
    assert_eq!(
        records
            .iter()
            .filter(|record| record.outcome == OperationOutcome::Succeeded)
            .count(),
        3
    );
}

/// A panicking Cloud SQL target must still resolve, so no target is stranded
/// `Pending` and the workflow can complete.
#[tokio::test]
async fn cloud_sql_batch_target_resolves_as_failed_when_action_task_panics() {
    let runner = CloudSqlPanicRunner::new(&["reports"]);
    let app = marked_cloud_sql_batch_app(
        vec![cloud_sql_instance("orders"), cloud_sql_instance("reports")],
        CloudSqlInstanceAction::Restart,
    );
    let mut session = start_batch(app, runner.clone(), None).await;

    assert!(session.process_next_effect_result().await);
    assert!(session.process_next_effect_result().await);

    let workflow = session
        .app()
        .root
        .resources
        .batch_workflow()
        .expect("workflow present");
    assert_eq!(workflow.ok_count(), 1);
    assert_eq!(workflow.fail_count(), 1);
    assert!(workflow.is_complete());

    assert!(session.process_next_effect_result().await);
    assert!(session.app().root.resources.batch_workflow().is_none());

    let records = session.app().root.system.operations.records();
    assert_eq!(
        records
            .iter()
            .filter(|record| record.outcome == OperationOutcome::Pending)
            .count(),
        0,
        "no operations-log entry should be stranded at Pending"
    );
}

/// Cloud SQL twin of [`PanicRunner`].
#[derive(Clone)]
struct CloudSqlPanicRunner {
    state: Arc<PanicState>,
}

impl CloudSqlPanicRunner {
    fn new(panic_names: &[&str]) -> Self {
        Self {
            state: Arc::new(PanicState {
                list_output: CLOUD_SQL_INSTANCES.to_string(),
                panic_names: panic_names.iter().map(|name| name.to_string()).collect(),
                action_commands: Mutex::new(Vec::new()),
                list_count: Mutex::new(0),
            }),
        }
    }
}

impl CommandRunner for CloudSqlPanicRunner {
    fn run(&self, command: GcloudCommand) -> Result<CommandOutput, CommandError> {
        if is_cloud_sql_list_command(&command) {
            *self.state.list_count.lock().unwrap() += 1;
            return output(self.state.list_output.clone());
        }
        let panics = self
            .state
            .panic_names
            .iter()
            .any(|name| command.args.iter().any(|arg| arg == name));
        self.state.action_commands.lock().unwrap().push(command);
        if panics {
            panic!("simulated action task panic");
        }
        output("")
    }
}

/// A failed follow-up refresh must still dismiss a completed Cloud SQL
/// batch-progress panel, exactly as it does for Compute — otherwise the panel
/// strands over the list permanently.
#[tokio::test]
async fn completed_cloud_sql_batch_workflow_is_cleared_when_follow_up_refresh_fails() {
    let runner = CloudSqlRecordingRunner::new(&[]);
    let app = marked_cloud_sql_batch_app(
        vec![cloud_sql_instance("orders"), cloud_sql_instance("reports")],
        CloudSqlInstanceAction::Restart,
    );
    let mut session = start_batch(app, runner.clone(), None).await;

    assert!(session.process_next_effect_result().await);
    assert!(session.process_next_effect_result().await);
    assert!(
        session
            .app()
            .root
            .resources
            .batch_workflow()
            .expect("workflow present")
            .is_complete()
    );

    let context = session
        .app()
        .root
        .current_context()
        .expect("context")
        .clone();
    session
        .apply_effect_result(EffectResult::CloudSqlInstancesLoadFailed {
            context,
            error: AppError::gcloud("refresh failed".to_string()),
        })
        .await;

    assert!(session.app().root.resources.batch_workflow().is_none());
}

/// The Cloud SQL twin of `cancelled_batch_resolves_started_unknown_and_
/// unstarted_cancelled`, cancelled by a scope switch rather than a
/// configuration switch. Both services share one cancellation path, and this is
/// what keeps it that way: a Compute-only fix would leave Cloud SQL batches
/// stranding entries `Pending`.
#[tokio::test]
async fn cancelled_cloud_sql_batch_resolves_started_unknown_and_unstarted_cancelled() {
    let runner = GateRunner::new(CLOUD_SQL_INSTANCES);
    let app = marked_cloud_sql_batch_app(
        vec![cloud_sql_instance("orders"), cloud_sql_instance("reports")],
        CloudSqlInstanceAction::Restart,
    );
    let session_config = Config {
        batch_concurrency: 1,
        ..Config::default()
    };
    let mut session = start_batch(app, runner.clone(), Some(session_config)).await;

    let waiter = runner.clone();
    tokio::task::spawn_blocking(move || waiter.wait_until_started(1))
        .await
        .unwrap();

    let started: Vec<String> = runner
        .action_commands()
        .iter()
        .map(cloud_sql_action_target)
        .collect();
    assert_eq!(started.len(), 1, "exactly one target should be in flight");
    let started_name = started[0].clone();
    let unstarted_name = if started_name == "orders" {
        "reports"
    } else {
        "orders"
    };

    // Switching workspace is a scope change, which cancels the batch.
    session
        .dispatch_action(Action::Workspace(WorkspaceAction::Configurations))
        .await;

    assert_eq!(
        pending_operation_count(&session),
        0,
        "a cancelled Cloud SQL batch must not strand any entry Pending"
    );
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

    runner.release(2);
    assert!(session.process_next_effect_result().await);
    assert_eq!(runner.started(), 1);
}
