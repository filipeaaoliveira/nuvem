//! Half (i): a list fetch whose result the runtime's generation guard
//! dropped is rescheduled when the user returns to the Resources workspace,
//! without turning that reschedule into a refetch loop and without reopening
//! the persistent-failure path the retry gate closes.

use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use nuvem::app::state::resources::ResourceLoadPhase;
use nuvem::app::{Action, AppState, EffectResult, WorkspaceAction};
use nuvem::services::gcloud::cli::{CommandError, CommandOutput, CommandRunner, GcloudCommand};
use nuvem::services::runtime::session::RuntimeSession;
use nuvem_domain::gcp::configuration::{GcloudConfiguration, GcloudConfigurationName};

/// Deadline for every gate wait in this file, matching
/// `tests/runtime_compute_action_effect_test.rs` and
/// `tests/generation_test.rs`. The gate is released within
/// milliseconds, so 30s sits orders of magnitude above any plausible
/// scheduling delay on a loaded CI runner: exceeding it proves the gate wedged
/// rather than that the machine was slow. The bound matters because the wait
/// runs on a `spawn_blocking` thread, where an unbounded park is uncancellable
/// and makes `Runtime::drop` hang forever when a test panics before releasing.
const GATE_TIMEOUT: Duration = Duration::from_secs(30);

/// The argv tail of `gcloud compute instances list` as the runtime builds it.
fn is_instance_list(command: &GcloudCommand) -> bool {
    command.args.ends_with(&[
        "compute".to_string(),
        "instances".to_string(),
        "list".to_string(),
        "--format=json".to_string(),
        "--limit=500".to_string(),
    ])
}

/// A single-instance `compute instances list` payload, so each fetch can be
/// told apart by the row it lands.
fn list_json(name: &str) -> String {
    format!(
        r#"[
          {{
            "id": "{name}-id",
            "name": "{name}",
            "zone": "https://www.googleapis.com/compute/v1/projects/dev-project/zones/europe-west1-b",
            "status": "RUNNING",
            "machineType": "https://www.googleapis.com/compute/v1/projects/dev-project/zones/europe-west1-b/machineTypes/e2-medium"
          }}
        ]"#
    )
}

fn output(stdout: impl Into<String>) -> Result<CommandOutput, CommandError> {
    Ok(CommandOutput {
        stdout: stdout.into(),
        stderr: String::new(),
    })
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

/// Serves the queued outcome for list call `index`: `Ok(stdout)` or
/// `Err(stderr)`. Stored as strings because `CommandError` is not `Clone`.
/// Overrunning the queue panics rather than falling back to a default, so a
/// test that pins an exact fetch count fails loudly on an extra call instead
/// of silently absorbing it.
fn list_result(
    results: &[Result<String, String>],
    index: usize,
) -> Result<CommandOutput, CommandError> {
    match results.get(index) {
        Some(Ok(stdout)) => output(stdout.clone()),
        Some(Err(stderr)) => Err(CommandError::Failed {
            stdout: String::new(),
            stderr: stderr.clone(),
            exit_code: None,
        }),
        None => panic!(
            "unexpected `compute instances list` call #{} — the test expected at most {}",
            index + 1,
            results.len()
        ),
    }
}

/// Counts `compute instances list` calls and serves the queued payload for
/// each, so a test can pin an EXACT dispatched-fetch count.
#[derive(Clone)]
struct CountingListRunner {
    state: Arc<CountingListRunnerState>,
}

struct CountingListRunnerState {
    list_count: Mutex<usize>,
    list_results: Vec<Result<String, String>>,
}

impl CountingListRunner {
    fn new(list_results: Vec<Result<String, String>>) -> Self {
        Self {
            state: Arc::new(CountingListRunnerState {
                list_count: Mutex::new(0),
                list_results,
            }),
        }
    }

    fn list_count(&self) -> usize {
        *self.state.list_count.lock().unwrap()
    }
}

impl CommandRunner for CountingListRunner {
    fn run(&self, command: GcloudCommand) -> Result<CommandOutput, CommandError> {
        if is_instance_list(&command) {
            let index = {
                let mut count = self.state.list_count.lock().unwrap();
                let index = *count;
                *count += 1;
                index
            };
            return list_result(&self.state.list_results, index);
        }
        output("")
    }
}

/// Holds the first `compute instances list` call open on a bounded `Condvar`
/// gate, mirroring `BlockingListRunner` in
/// `tests/runtime_compute_action_effect_test.rs` (test binaries cannot share
/// helpers). Every wait is deadlined: see [`GATE_TIMEOUT`].
#[derive(Clone)]
struct BlockingListRunner {
    state: Arc<BlockingListRunnerState>,
}

struct BlockingListRunnerState {
    list_count: Mutex<usize>,
    first_list_started: (Mutex<bool>, Condvar),
    release_first_list: (Mutex<bool>, Condvar),
    list_results: Vec<Result<String, String>>,
}

impl BlockingListRunner {
    fn new(list_results: Vec<Result<String, String>>) -> Self {
        Self {
            state: Arc::new(BlockingListRunnerState {
                list_count: Mutex::new(0),
                first_list_started: (Mutex::new(false), Condvar::new()),
                release_first_list: (Mutex::new(false), Condvar::new()),
                list_results,
            }),
        }
    }

    fn list_count(&self) -> usize {
        *self.state.list_count.lock().unwrap()
    }

    /// Blocks until the first list call has begun, giving up at the gate
    /// deadline rather than parking forever.
    fn wait_for_first_list(&self) {
        let deadline = Instant::now() + GATE_TIMEOUT;
        let (started, wake) = &self.state.first_list_started;
        let mut started = started.lock().unwrap();
        while !*started {
            // Budget the remaining time against the single deadline so a
            // spurious wakeup cannot restart the full timeout.
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                // Format first, then release the lock: panicking with the guard
                // still held would poison the mutex and turn any later wait on
                // this gate into a `PoisonError` instead of its own diagnostic.
                let message = format!(
                    "BlockingListRunner::wait_for_first_list timed out after {GATE_TIMEOUT:?}: the first list call never started"
                );
                drop(started);
                panic!("{message}");
            }
            started = wake.wait_timeout(started, remaining).unwrap().0;
        }
    }

    fn release_first_list(&self) {
        let (released, wake) = &self.state.release_first_list;
        *released.lock().unwrap() = true;
        wake.notify_one();
    }
}

impl CommandRunner for BlockingListRunner {
    fn run(&self, command: GcloudCommand) -> Result<CommandOutput, CommandError> {
        if is_instance_list(&command) {
            let list_index = {
                let mut count = self.state.list_count.lock().unwrap();
                let index = *count;
                *count += 1;
                index
            };
            if list_index == 0 {
                let (started, wake) = &self.state.first_list_started;
                *started.lock().unwrap() = true;
                wake.notify_one();

                // This wait runs on the refresh task's `spawn_blocking` thread,
                // where an unbounded park is uncancellable and would wedge
                // `Runtime::drop` for good if a test panicked before releasing.
                // Bound it and panic loudly: a panicking blocking task surfaces
                // as a `JoinError`, a hang does not.
                let deadline = Instant::now() + GATE_TIMEOUT;
                let (released, wake) = &self.state.release_first_list;
                let mut released = released.lock().unwrap();
                while !*released {
                    let remaining = deadline.saturating_duration_since(Instant::now());
                    if remaining.is_zero() {
                        let message = format!(
                            "BlockingListRunner::run timed out after {GATE_TIMEOUT:?} waiting on release_first_list"
                        );
                        drop(released);
                        panic!("{message}");
                    }
                    released = wake.wait_timeout(released, remaining).unwrap().0;
                }
            }
            return list_result(&self.state.list_results, list_index);
        }
        output("")
    }
}

async fn session_with_dev_config(runner: Box<dyn CommandRunner + Send + Sync>) -> RuntimeSession {
    let mut session = RuntimeSession::new_with_runner_for_test(AppState::new(), runner);
    session
        .apply_effect_result(EffectResult::ConfigurationsLoaded {
            configurations: vec![config()],
        })
        .await;
    session
}

fn instance_names(session: &RuntimeSession) -> Vec<String> {
    session
        .app()
        .root
        .resources
        .instances()
        .iter()
        .map(|instance| instance.name.clone())
        .collect()
}

/// Whether the active service's list fetch is in flight. `begin_load` runs
/// synchronously inside `dispatch_background_refresh`, so this flips before
/// `dispatch_action` returns: it is a race-free signal for "a fetch WAS
/// scheduled", unlike the runner's call count, which the worker thread
/// increments later.
fn fetch_in_flight(session: &RuntimeSession) -> bool {
    matches!(
        session.app().root.resources.active_load_phase(),
        ResourceLoadPhase::Loading { .. }
    )
}

/// Leaving the Resources workspace mid-load bumps `scope_generation`, so the
/// in-flight list result loses the guard in `process_next_effect_result` and is
/// dropped. `auto_loaded_scope` used to still hold that scope, so
/// returning early-returned out of `ensure_active_resource_loaded` and the user
/// was left with empty rows and the generic press-r prompt.
#[tokio::test]
async fn returning_after_a_dropped_list_fetch_reschedules_it() {
    let runner = BlockingListRunner::new(vec![
        Ok(list_json("detached-dev-1")),
        Ok(list_json("returned-dev-1")),
    ]);
    let mut session = session_with_dev_config(Box::new(runner.clone())).await;

    session
        .dispatch_action(Action::Workspace(WorkspaceAction::Resources))
        .await;
    let wait_runner = runner.clone();
    tokio::task::spawn_blocking(move || wait_runner.wait_for_first_list())
        .await
        .unwrap();

    // Leave while the fetch is genuinely still running inside the runner.
    session
        .dispatch_action(Action::Workspace(WorkspaceAction::Tunnels))
        .await;
    runner.release_first_list();

    // The detached result arrives and is dropped by the scope guard.
    assert!(session.process_next_effect_result().await);
    assert_eq!(
        instance_names(&session),
        Vec::<String>::new(),
        "the detached fetch's rows must not be applied"
    );
    assert_eq!(
        session.app().root.resources.active_load_phase(),
        &ResourceLoadPhase::Idle,
        "an abandoned load still resolves to Idle, exactly as the detached-load work left it"
    );

    session
        .dispatch_action(Action::Workspace(WorkspaceAction::Resources))
        .await;
    assert!(
        fetch_in_flight(&session),
        "returning to Resources after a dropped fetch must schedule a new one"
    );

    assert!(session.process_next_effect_result().await);
    assert_eq!(
        instance_names(&session),
        vec!["returned-dev-1".to_string()],
        "the rescheduled fetch's rows must land"
    );
    assert_eq!(runner.list_count(), 2);
}

/// The gate-(b)/(d) argument from the design, pinned as a regression test:
/// clearing `auto_loaded_scope` at the drop site must not turn
/// clear -> refetch -> self-drop into a cycle.
///
/// The exact expected count is 4, and every one of the four is forced:
/// - Rounds 0..3 each detach their own fetch by leaving before its result is
///   drained, so each round must schedule exactly one fetch and no more:
///   3 fetches. More than one per round would be the loop this test exists to
///   rule out; fewer would be the bug itself.
/// - The settling round stays in Resources, so its fetch lands rows: 1 fetch.
/// - The two away/back switches afterwards must schedule nothing, because gate
///   (a) still holds: `needs_refresh` is false once fresh rows are in state.
/// So 3 + 1 + 0 = 4 — an upper AND lower bound, not a "reasonably small" one.
#[tokio::test]
async fn repeated_away_and_back_switching_does_not_loop_the_list_fetch() {
    let runner = CountingListRunner::new(vec![
        Ok(list_json("detached-1")),
        Ok(list_json("detached-2")),
        Ok(list_json("detached-3")),
        Ok(list_json("settled-dev-1")),
    ]);
    let mut session = session_with_dev_config(Box::new(runner.clone())).await;

    for round in 0..3 {
        session
            .dispatch_action(Action::Workspace(WorkspaceAction::Resources))
            .await;
        assert!(
            fetch_in_flight(&session),
            "round {round}: returning to Resources after a dropped fetch must schedule exactly one new fetch"
        );
        session
            .dispatch_action(Action::Workspace(WorkspaceAction::Tunnels))
            .await;
        // Drains this round's detached result, which the scope guard drops.
        assert!(session.process_next_effect_result().await);
        assert_eq!(
            instance_names(&session),
            Vec::<String>::new(),
            "round {round}: a detached fetch's rows must not be applied"
        );
    }

    // Settle: stay in Resources so this fetch's result is applied.
    session
        .dispatch_action(Action::Workspace(WorkspaceAction::Resources))
        .await;
    assert!(session.process_next_effect_result().await);
    assert_eq!(instance_names(&session), vec!["settled-dev-1".to_string()]);

    // With fresh rows in state, further switching must schedule nothing.
    for round in 0..2 {
        session
            .dispatch_action(Action::Workspace(WorkspaceAction::Tunnels))
            .await;
        session
            .dispatch_action(Action::Workspace(WorkspaceAction::Resources))
            .await;
        assert!(
            !fetch_in_flight(&session),
            "settled round {round}: fresh rows mean no refresh is needed, so none may be scheduled"
        );
    }

    assert_eq!(
        runner.list_count(),
        4,
        "exactly one fetch per detached round, one settling fetch, and none after"
    );
}

/// A `Persistent` failure never reaches the drop site — it is APPLIED, so it
/// sets `gated_scope` and leaves `auto_loaded_scope` recorded. The worst
/// realistic outcome would be a permanently failing service getting
/// rescheduled forever and hammering gcloud; this pins that it does not, and
/// drives the real gate: `poll_active_resource` consults `gated_scope` only.
#[tokio::test]
async fn a_persistently_failing_service_is_not_rescheduled_by_the_poll_tick() {
    // "permission denied" classifies as `AppErrorKind::PermissionDenied`,
    // whose retryability is `Persistent` — what arms `gated_scope`.
    let runner = CountingListRunner::new(vec![Err("permission denied".to_string())]);
    let mut session = session_with_dev_config(Box::new(runner.clone())).await;

    session
        .dispatch_action(Action::Workspace(WorkspaceAction::Resources))
        .await;
    assert!(session.process_next_effect_result().await);
    assert!(
        session.app().root.resources.compute_problem().is_some(),
        "the persistent failure must be applied, not dropped"
    );
    assert!(
        matches!(
            session.app().root.resources.active_load_phase(),
            ResourceLoadPhase::Failed { .. }
        ),
        "a real, still-valid error keeps the Failed phase"
    );

    // The poll ticker is the path that could hammer gcloud. Run it a bounded
    // number of times: each dispatch would show up synchronously as a new
    // `Loading` phase, and the runner would panic on an unexpected list call.
    for tick in 0..5 {
        session.poll_active_resource().await;
        assert!(
            matches!(
                session.app().root.resources.active_load_phase(),
                ResourceLoadPhase::Failed { .. }
            ),
            "tick {tick}: the gated scope must not be re-dispatched"
        );
    }

    assert_eq!(
        runner.list_count(),
        1,
        "the failing service must be fetched once, not once per poll tick"
    );
}

/// The interaction this actually creates, and the one worth pinning: a
/// persistently failing service whose result is DROPPED, so the new drop-site
/// clear runs and the fetch IS rescheduled — and the retry gate still stops it
/// there.
///
/// This traverses the new code path end to end, which
/// `a_persistently_failing_service_is_not_rescheduled_by_the_poll_tick` does
/// not: that one's failure is applied, never dropped, so `auto_loaded_scope` is
/// never cleared. Here the drop happens first, the clear fires, the reschedule
/// happens, and only then does the gate get its chance.
///
/// **Exact expected count: 2, and every one is forced.**
/// - Fetch 1: entering Resources. Its result is dropped by the scope guard, so
///   it never reaches the apply site and therefore never arms `gated_scope` —
///   design gate (c). A dropped failure is NOT a recorded failure.
/// - Fetch 2: returning to Resources. Exactly one, from the drop-site clear.
///   Zero would be the bug; two or more would be the refetch loop.
/// - Then 0: fetch 2's failure IS applied, so it arms `gated_scope`, and all
///   five poll ticks are gated. This is the bound that matters — an unbounded
///   count here is "a permanently failing service hammers gcloud in a loop".
/// The runner's queue holds exactly 2 entries, so a third call panics rather
/// than being quietly absorbed.
#[tokio::test]
async fn a_dropped_persistent_failure_is_rescheduled_exactly_once_then_gated() {
    // Both list calls fail the same way; "permission denied" classifies as
    // `AppErrorKind::PermissionDenied`, whose retryability is `Persistent`.
    let runner = BlockingListRunner::new(vec![
        Err("permission denied".to_string()),
        Err("permission denied".to_string()),
    ]);
    let mut session = session_with_dev_config(Box::new(runner.clone())).await;

    // Fetch 1 starts and is held open inside the runner.
    session
        .dispatch_action(Action::Workspace(WorkspaceAction::Resources))
        .await;
    let wait_runner = runner.clone();
    tokio::task::spawn_blocking(move || wait_runner.wait_for_first_list())
        .await
        .unwrap();

    // Leave mid-fetch, so its failure result loses the scope guard.
    session
        .dispatch_action(Action::Workspace(WorkspaceAction::Tunnels))
        .await;
    runner.release_first_list();
    assert!(session.process_next_effect_result().await);
    assert!(
        session.app().root.resources.compute_problem().is_none(),
        "a dropped failure must not be applied as a problem"
    );

    // Returning reschedules it exactly once — the new drop-site clear.
    session
        .dispatch_action(Action::Workspace(WorkspaceAction::Resources))
        .await;
    assert!(
        fetch_in_flight(&session),
        "returning after a dropped failure must schedule exactly one refetch"
    );

    // Fetch 2's failure is applied, which is what arms `gated_scope`.
    assert!(session.process_next_effect_result().await);
    assert!(
        session.app().root.resources.compute_problem().is_some(),
        "the second failure lands in-workspace, so it must be applied"
    );
    assert!(
        matches!(
            session.app().root.resources.active_load_phase(),
            ResourceLoadPhase::Failed { .. }
        ),
        "a real, still-valid error keeps the Failed phase"
    );

    // The poll ticker is the path that could hammer gcloud. Each dispatch would
    // show up synchronously as a fresh `Loading` phase (`begin_load` runs before
    // the task is spawned), so this is race-free.
    for tick in 0..5 {
        session.poll_active_resource().await;
        assert!(
            !fetch_in_flight(&session),
            "tick {tick}: the gated scope must not be re-dispatched after a rescheduled failure"
        );
    }

    assert_eq!(
        runner.list_count(),
        2,
        "one fetch on entry, exactly one reschedule on return, and none once the gate is armed"
    );
}
