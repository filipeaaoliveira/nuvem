//! Evidence — pressing `q` while a lifecycle action is in flight
//! exits the process instead of holding it for up to `LIFECYCLE_TIMEOUT`.
//!
//! The defect was invisible by construction. `flush_remembered_view_saves` and
//! `ratatui::restore()` both complete, `run()` returns, and only then does
//! `#[tokio::main]`'s `Runtime::drop` park on the `spawn_blocking` thread the
//! batch target is sitting on — with the terminal already restored, so nothing
//! on screen says why. `TerminationSignals::install` has registered tokio
//! handlers for SIGTERM and SIGHUP by then, so `kill` is swallowed too and only
//! `kill -9` gets out. The wait used to be bounded at `RUN_TIMEOUT`;
//! scoping the ceiling to the command raised it to thirty minutes, which is
//! right for the operation and wrong for a shutdown.
//!
//! What is proven here, in the order the tests appear:
//!
//! 1. the batch path uses a cancellable call rather than `run`, and the token
//!    the session flips at shutdown is the one the in-flight target holds;
//! 2. the same for the Cloud SQL half of the batch runner, which is a separate
//!    closure and would not be covered by the Compute one;
//! 3. a batch nobody cancels still issues every command and still reports every
//!    outcome — without which (1) and (2) are satisfied by a batch that runs
//!    nothing;
//! 4. `Runtime::drop` itself returns promptly, against a real child that would
//!    otherwise outlive it by five minutes.
//!
//! What none of it buys, and what the code must not claim: the GCP operation. A
//! `stop` gcloud has already handed over runs to completion whatever happens to
//! the CLI. Cancelling the call stops nuvem waiting; it does not undo anything.
//!
//! Every wait here is a bounded gate released explicitly by the test — no
//! sleeps and no unbounded parks, which on a `spawn_blocking` thread are
//! uncancellable and wedge `Runtime::drop` for good, which is the very failure
//! under test.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use nuvem::Config;
use nuvem::app::reducer::root::reduce;
use nuvem::app::state::operations::OperationOutcome;
use nuvem::app::state::resources::{
    CloudSql, CloudSqlInstanceAction, Compute, ComputeInstanceAction,
};
use nuvem::app::{Action, AppState, EffectResult, ResourcesAction};
use nuvem::services::gcloud::cli::{
    CommandCeiling, CommandError, CommandOutput, CommandRunner, GcloudCommand, ProcessCommandRunner,
};
use nuvem::services::runtime::session::RuntimeSession;
use nuvem_domain::gcp::configuration::{GcloudConfiguration, GcloudConfigurationName};
use nuvem_domain::gcp::resource::{
    CloudSqlAvailability, CloudSqlInstance, CloudSqlStatus, ComputeInstance, EndpointVisibility,
    ResourceServiceKind, ResourceStatus,
};
use tokio_util::sync::CancellationToken;

/// Deadline for every gate wait. The test releases its gate within
/// milliseconds, so 30s sits orders of magnitude above any plausible scheduling
/// delay: exceeding it proves the gate wedged, not that the machine was slow.
const GATE_TIMEOUT: Duration = Duration::from_secs(30);

fn empty_output() -> Result<CommandOutput, CommandError> {
    Ok(CommandOutput {
        stdout: String::new(),
        stderr: String::new(),
    })
}

/// Whether `command` is one of the batch's own lifecycle mutations — a
/// `compute instances <verb>` or a `sql instances <verb>` — which is the only
/// thing these fixtures hold open.
///
/// Derived from the argv rather than from `command.ceiling`, which the tests
/// below assert on: a gate keyed to the ceiling would be satisfied by whatever
/// the ceiling happened to be, and could not catch it being dropped.
///
/// Anything else the session emits is answered on the spot instead of being
/// parked until a deadline, which would turn a clear assertion into a
/// 30-second stall about something unrelated. The two reads a resource service
/// issues under the same nouns — `list` and `describe` — are excluded by name
/// for exactly that reason.
fn is_lifecycle_action(command: &GcloudCommand) -> bool {
    let mutates_instances = command
        .args
        .windows(2)
        .any(|window| window == ["compute", "instances"] || window == ["sql", "instances"]);
    let reads = command
        .args
        .iter()
        .any(|arg| arg == "list" || arg == "describe");
    mutates_instances && !reads
}

/// A runner whose lifecycle actions never return on their own, and that honours
/// cancellation the way `ProcessCommandRunner` does: by polling the token while
/// it waits.
///
/// `run` — the path with no token — is where the defect lived, so it is
/// recorded rather than made to panic: a panic inside `spawn_blocking` is
/// reported as a join error and the test would then fail on a gate deadline
/// with a message about the wrong thing. Counting it lets the assertion say
/// exactly which door the batch went through.
#[derive(Clone, Default)]
struct ShutdownRunner {
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
    abandoned: usize,
    uncancellable_runs: usize,
    ceilings: Vec<CommandCeiling>,
}

impl ShutdownRunner {
    /// Blocks until `count` lifecycle actions have reached the runner, giving
    /// up at the deadline rather than parking forever.
    fn wait_until_started(&self, count: usize) {
        self.wait_for("started", count, |inner| inner.started);
    }

    /// Blocks until `count` lifecycle actions have given up waiting.
    fn wait_until_abandoned(&self, count: usize) {
        self.wait_for("abandoned", count, |inner| inner.abandoned);
    }

    fn wait_for(&self, what: &str, count: usize, read: impl Fn(&RunnerInner) -> usize) {
        let deadline = Instant::now() + GATE_TIMEOUT;
        let mut guard = self.state.inner.lock().unwrap();
        while read(&guard) < count {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                // Format first, then drop the guard: panicking while holding it
                // would poison the mutex and replace a sibling waiter's own
                // diagnostic with `PoisonError`.
                let message = format!(
                    "waiting for {count} {what} timed out after {GATE_TIMEOUT:?}: observed {}",
                    read(&guard)
                );
                drop(guard);
                panic!("{message}");
            }
            guard = self.state.signal.wait_timeout(guard, remaining).unwrap().0;
        }
    }

    fn uncancellable_runs(&self) -> usize {
        self.state.inner.lock().unwrap().uncancellable_runs
    }

    fn ceilings(&self) -> Vec<CommandCeiling> {
        self.state.inner.lock().unwrap().ceilings.clone()
    }
}

impl CommandRunner for ShutdownRunner {
    fn run(&self, command: GcloudCommand) -> Result<CommandOutput, CommandError> {
        if !is_lifecycle_action(&command) {
            return empty_output();
        }
        {
            let mut guard = self.state.inner.lock().unwrap();
            guard.uncancellable_runs += 1;
            guard.started += 1;
            guard.ceilings.push(command.ceiling);
            self.state.signal.notify_all();
        }
        // Bounded rather than forever: this call has no token, which is the
        // defect, and an unbounded park here would wedge the blocking thread
        // for the life of the test binary instead of failing one test.
        std::thread::sleep(Duration::from_millis(200));
        empty_output()
    }

    fn run_bounded_with_cancellation(
        &self,
        command: GcloudCommand,
        cancellation: &CancellationToken,
    ) -> Result<CommandOutput, CommandError> {
        if !is_lifecycle_action(&command) {
            return empty_output();
        }
        {
            let mut guard = self.state.inner.lock().unwrap();
            guard.started += 1;
            guard.ceilings.push(command.ceiling);
            self.state.signal.notify_all();
        }
        let deadline = Instant::now() + GATE_TIMEOUT;
        while !cancellation.is_cancelled() {
            if Instant::now() >= deadline {
                panic!("the action was never cancelled within {GATE_TIMEOUT:?}");
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        let mut guard = self.state.inner.lock().unwrap();
        guard.abandoned += 1;
        self.state.signal.notify_all();
        Err(CommandError::Cancelled {
            ceiling: command.ceiling,
        })
    }
}

/// A runner that answers every lifecycle action at once, for the half of the
/// evidence that is about the batch still working.
#[derive(Clone, Default)]
struct RecordingRunner {
    issued: Arc<Mutex<Vec<Vec<String>>>>,
    cancellable_calls: Arc<AtomicUsize>,
}

impl RecordingRunner {
    fn issued(&self) -> Vec<Vec<String>> {
        self.issued.lock().unwrap().clone()
    }

    fn cancellable_calls(&self) -> usize {
        self.cancellable_calls.load(Ordering::SeqCst)
    }
}

impl CommandRunner for RecordingRunner {
    fn run(&self, command: GcloudCommand) -> Result<CommandOutput, CommandError> {
        if is_lifecycle_action(&command) {
            self.issued.lock().unwrap().push(command.args.clone());
        }
        empty_output()
    }

    fn run_bounded_with_cancellation(
        &self,
        command: GcloudCommand,
        cancellation: &CancellationToken,
    ) -> Result<CommandOutput, CommandError> {
        if cancellation.is_cancelled() {
            return Err(CommandError::Cancelled {
                ceiling: command.ceiling,
            });
        }
        self.cancellable_calls.fetch_add(1, Ordering::SeqCst);
        self.run(command)
    }
}

fn configuration() -> GcloudConfiguration {
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

/// Every named instance loaded under the dev context, all marked, and a pending
/// batch-Stop confirmation waiting for `ConfirmBatchAction`.
fn marked_batch_app(names: &[&str]) -> AppState {
    let mut app = AppState::new();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ConfigurationsLoaded {
            configurations: vec![configuration()],
        }),
    );
    let context = app.root.current_context().expect("context").clone();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ComputeInstancesLoaded {
            context: context.clone(),
            instances: names.iter().map(|name| instance(name)).collect(),
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

/// Cloud SQL twin of [`marked_batch_app`]. It exists because
/// `BatchTargetRunner::for_action` has one arm per batchable service and each
/// arm builds its own closure: a token threaded through the Compute arm alone
/// would leave Cloud SQL — whose failover is the *longest* of these commands —
/// still holding the exit.
fn marked_cloud_sql_batch_app(names: &[&str]) -> AppState {
    let mut app = AppState::new();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ConfigurationsLoaded {
            configurations: vec![configuration()],
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
            instances: names.iter().map(|name| cloud_sql_instance(name)).collect(),
        }),
    );
    let count = app.root.resources.visible_cloud_sql_instances().len();
    app.root.resources.toggle_mark();
    for _ in 1..count {
        app.root.resources.select_next();
        app.root.resources.toggle_mark();
    }
    app.root
        .resources
        .start_batch_confirmation::<CloudSql>(context, CloudSqlInstanceAction::Stop);
    app
}

fn one_permit_session(
    app: AppState,
    runner: Box<dyn CommandRunner + Send + Sync>,
) -> RuntimeSession {
    RuntimeSession::new_with_config_and_runner_for_test(
        Config {
            batch_concurrency: 1,
            ..Config::default()
        },
        app,
        runner,
    )
}

fn single_target_session(runner: Box<dyn CommandRunner + Send + Sync>) -> RuntimeSession {
    one_permit_session(marked_batch_app(&["vm-a"]), runner)
}

/// The threading assertion: a batch target holds the session's shutdown token,
/// so flipping it releases a call that would otherwise sit out the thirty
/// minutes `CommandCeiling::Lifecycle` allows.
///
/// Two things are asserted rather than one, because "shutdown no longer hangs"
/// is also satisfied by a batch that never calls gcloud at all: the target went
/// through the *cancellable* door (`uncancellable_runs` is zero), and it gave up
/// waiting once the session's own token flipped.
#[tokio::test]
async fn a_batch_target_in_flight_is_abandoned_when_the_process_shuts_down() {
    let runner = ShutdownRunner::default();
    let mut session = single_target_session(Box::new(runner.clone()));

    session
        .dispatch_action(Action::Resources(ResourcesAction::ConfirmBatchAction))
        .await;
    let waiter = runner.clone();
    tokio::task::spawn_blocking(move || waiter.wait_until_started(1))
        .await
        .unwrap();

    assert_eq!(
        runner.uncancellable_runs(),
        0,
        "a batch target must issue its command through the cancellable path, \
         or nothing can release it at shutdown"
    );
    // The ceiling has to survive the new path. `run_with_cancellation` is
    // deliberately unbounded, so a fix that reached for it would trade the
    // shutdown hang for a lifecycle call with no ceiling at all.
    assert_eq!(
        runner.ceilings(),
        vec![CommandCeiling::Lifecycle],
        "a batch stop is a lifecycle command whichever method carries it"
    );

    session.cancel_lifecycle_actions_for_shutdown();

    let waiter = runner.clone();
    tokio::task::spawn_blocking(move || waiter.wait_until_abandoned(1))
        .await
        .unwrap();
}

/// The same claim for the other batchable service, because the two are separate
/// closures. Cloud SQL is the one that most needs it: a failover or an
/// activation-policy patch drains connections first, so it sits at the long end
/// of the thirty minutes rather than near the short one.
#[tokio::test]
async fn a_cloud_sql_batch_target_in_flight_is_abandoned_when_the_process_shuts_down() {
    let runner = ShutdownRunner::default();
    let mut session = one_permit_session(
        marked_cloud_sql_batch_app(&["orders"]),
        Box::new(runner.clone()),
    );

    session
        .dispatch_action(Action::Resources(ResourcesAction::ConfirmBatchAction))
        .await;
    let waiter = runner.clone();
    tokio::task::spawn_blocking(move || waiter.wait_until_started(1))
        .await
        .unwrap();

    assert_eq!(
        runner.uncancellable_runs(),
        0,
        "a Cloud SQL batch target must issue its command through the cancellable \
         path, or nothing can release it at shutdown"
    );
    assert_eq!(
        runner.ceilings(),
        vec![CommandCeiling::Lifecycle],
        "a Cloud SQL stop is a lifecycle command whichever method carries it"
    );

    session.cancel_lifecycle_actions_for_shutdown();

    let waiter = runner.clone();
    tokio::task::spawn_blocking(move || waiter.wait_until_abandoned(1))
        .await
        .unwrap();
}

/// The positive half. Every assertion above is satisfied by a batch that issues
/// nothing, so this pins that the ordinary path — nobody cancelling anything —
/// still runs each target's command and still settles each target's entry.
#[tokio::test]
async fn a_batch_nobody_cancels_still_runs_every_target_and_reports_it() {
    let runner = RecordingRunner::default();
    let mut session = RuntimeSession::new_with_config_and_runner_for_test(
        Config::default(),
        marked_batch_app(&["vm-a", "vm-b"]),
        Box::new(runner.clone()),
    );

    session
        .dispatch_action(Action::Resources(ResourcesAction::ConfirmBatchAction))
        .await;

    let drain = async {
        while session.app().root.system.operations.pending_count() > 0 {
            session.process_next_effect_result().await;
        }
    };
    tokio::time::timeout(GATE_TIMEOUT, drain)
        .await
        .expect("every batch target reports exactly one result");

    let issued = runner.issued();
    assert_eq!(
        issued.len(),
        2,
        "both marked targets must have had their command issued: {issued:?}"
    );
    for name in ["vm-a", "vm-b"] {
        assert!(
            issued.iter().any(|args| args.iter().any(|arg| arg == name)),
            "no stop command was issued for {name}: {issued:?}"
        );
    }
    let outcomes: Vec<OperationOutcome> = session
        .app()
        .root
        .system
        .operations
        .records()
        .iter()
        .map(|record| record.outcome)
        .collect();
    assert_eq!(
        outcomes,
        vec![OperationOutcome::Succeeded, OperationOutcome::Succeeded],
        "a batch nobody cancelled reports both targets as done"
    );
    // The working path and the abandonable one are the same call, not a
    // shutdown-only detour beside a `run` the ordinary batch still takes. If
    // they were two, this count would be zero here and the tests above would
    // still pass — and the token would only ever be honoured by whichever path
    // shutdown happened to reach.
    assert_eq!(
        runner.cancellable_calls(),
        2,
        "both targets ran through the same cancellable call the shutdown path uses"
    );
}

/// The operator-visible claim, against a real child: `Runtime::drop` returns.
///
/// The fake gcloud here `exec`s a five-minute sleep for the batch's stop and
/// exits at once for anything else, which is the shape of the real thing — a
/// `stop` blocks until GCP reports the operation finished. `Runtime::drop`
/// blocks on every started blocking task, so this drop used to wait
/// the full five minutes, and against a real `gcloud` it would have waited out
/// `LIFECYCLE_TIMEOUT`.
///
/// A wall-clock assertion, which the rest of this suite avoids, because the
/// wall clock is the whole complaint: there is no state to observe here, only
/// the operator's process failing to exit. The gate is two orders of magnitude
/// below the sleep it is distinguishing from, so it cannot be tripped by load.
#[cfg(unix)]
#[test]
fn dropping_the_runtime_after_a_shutdown_cancellation_does_not_wait_for_the_child() {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;

    let directory = tempfile::tempdir().expect("temporary directory should be created");
    let binary = directory.path().join("gcloud");
    let pid_path = directory.path().join("stop.pid");
    // `exec` keeps the pid, so `$$` recorded here is the sleeping process
    // itself. Only the stop sleeps: any other command this session issues would
    // otherwise be an uncancellable `run` holding the drop for its own ceiling,
    // and the measurement below would be about the wrong call.
    fs::write(
        &binary,
        format!(
            "#!/bin/sh\nfor arg in \"$@\"; do\n  if [ \"$arg\" = stop ]; then\n    printf '%s\\n' \"$$\" > '{}'\n    exec sleep 300\n  fi\ndone\nexit 0\n",
            pid_path.display()
        ),
    )
    .expect("test command should be written");
    let mut permissions = fs::metadata(&binary)
        .expect("test command metadata should be available")
        .permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&binary, permissions).expect("test command should be executable");

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("runtime should build");
    let mut session = single_target_session(Box::new(ProcessCommandRunner::new(binary)));
    runtime
        .block_on(session.dispatch_action(Action::Resources(ResourcesAction::ConfirmBatchAction)));

    // Gate on the pid file parsing, not on it existing: the shell creates it
    // before it writes into it. Reading it also proves the child is up, so the
    // cancellation below lands on a running process rather than on a spawn in
    // flight.
    let started = Instant::now();
    let child = loop {
        if let Some(pid) = fs::read_to_string(&pid_path)
            .ok()
            .and_then(|text| text.trim().parse::<libc::pid_t>().ok())
        {
            break pid;
        }
        assert!(
            started.elapsed() < GATE_TIMEOUT,
            "the batch's gcloud child never recorded a pid"
        );
        std::thread::sleep(Duration::from_millis(10));
    };

    // What `run()` does on the way out, in the same order: cancel, then let the
    // runtime go.
    session.cancel_lifecycle_actions_for_shutdown();
    drop(session);
    let dropping = Instant::now();
    drop(runtime);
    let elapsed = dropping.elapsed();

    assert!(
        elapsed < GATE_TIMEOUT,
        "Runtime::drop waited {elapsed:?} on an in-flight lifecycle action; \
         the operator's `q` hangs until the command's ceiling"
    );
    // The child is killed rather than merely abandoned: leaving it running
    // would make this a test that leaks a process every time it passes.
    let deadline = Instant::now() + Duration::from_secs(2);
    // SAFETY: `kill` takes no pointers, and signal 0 only probes for the pid's
    // existence without delivering anything — so polling it here cannot disturb
    // whatever process holds the id, abandoned child or not.
    while unsafe { libc::kill(child, 0) } == 0 {
        assert!(
            Instant::now() < deadline,
            "the abandoned child survived the cancellation"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}
