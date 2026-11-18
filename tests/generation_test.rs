//! Half (ii): the background-result supersession stamp is bumped at
//! most once per `dispatch_effects` batch, so a batch cannot invalidate its own
//! later effects, while a *newer* batch still supersedes an older in-flight
//! fetch. The stamp was `resource_generation` and `operation_generation`
//! together until the first was removed as subsumed by the second; these
//! tests are unchanged by that and now pin `operation_generation` alone.
//!
//! The third property of the same stamp: the bump is *lazy* — it
//! happens on the first effect of the batch that opens a status-bar operation,
//! so a batch that opens none leaves in-flight fetches alone. All three are
//! properties of one `if` in `dispatch_effects`, so they live together here.

use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use nuvem::app::{Action, AppState, Effect, EffectResult, ResourcesAction};
use nuvem::services::gcloud::cli::{CommandError, CommandOutput, CommandRunner, GcloudCommand};
use nuvem::services::runtime::clipboard::ClipboardWriter;
use nuvem::services::runtime::session::RuntimeSession;
use nuvem_domain::gcp::configuration::{GcloudConfiguration, GcloudConfigurationName};

/// Deadline for every gate wait in this file, matching
/// `tests/runtime_compute_action_effect_test.rs`. The gate is released within
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

fn is_instance_describe(command: &GcloudCommand) -> bool {
    command.args.windows(3).any(|window| {
        window
            == [
                "compute".to_string(),
                "instances".to_string(),
                "describe".to_string(),
            ]
    })
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

/// Routes by argv rather than by call order: the list fetch runs on a
/// background thread while the detail describe runs inline, so an
/// order-indexed queue would race.
#[derive(Clone)]
struct RoutingRunner {
    state: Arc<RoutingRunnerState>,
}

struct RoutingRunnerState {
    list_outputs: Vec<String>,
    list_count: Mutex<usize>,
    detail_output: String,
}

impl RoutingRunner {
    fn new(list_outputs: Vec<String>, detail_output: String) -> Self {
        Self {
            state: Arc::new(RoutingRunnerState {
                list_outputs,
                list_count: Mutex::new(0),
                detail_output,
            }),
        }
    }
}

impl CommandRunner for RoutingRunner {
    fn run(&self, command: GcloudCommand) -> Result<CommandOutput, CommandError> {
        if is_instance_list(&command) {
            let index = {
                let mut count = self.state.list_count.lock().unwrap();
                let index = *count;
                *count += 1;
                index
            };
            return output(
                self.state
                    .list_outputs
                    .get(index)
                    .cloned()
                    .unwrap_or_default(),
            );
        }
        if is_instance_describe(&command) {
            return output(self.state.detail_output.clone());
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
    list_outputs: Vec<String>,
}

impl BlockingListRunner {
    fn new(list_outputs: Vec<String>) -> Self {
        Self {
            state: Arc::new(BlockingListRunnerState {
                list_count: Mutex::new(0),
                first_list_started: (Mutex::new(false), Condvar::new()),
                release_first_list: (Mutex::new(false), Condvar::new()),
                list_outputs,
            }),
        }
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
            return output(
                self.state
                    .list_outputs
                    .get(list_index)
                    .cloned()
                    .unwrap_or_default(),
            );
        }
        output("")
    }
}

/// Records what a `CopyToClipboard` effect wrote, so the intervening batch in
/// the laziness test is proven to have carried a real effect rather than to
/// have reduced to nothing.
#[derive(Clone, Default)]
struct RecordingClipboard {
    values: Arc<Mutex<Vec<String>>>,
}

impl RecordingClipboard {
    fn values(&self) -> Vec<String> {
        self.values.lock().unwrap().clone()
    }
}

impl ClipboardWriter for RecordingClipboard {
    fn set_text(&self, value: &str) -> Result<(), String> {
        self.values.lock().unwrap().push(value.to_string());
        Ok(())
    }
}

async fn session_with_dev_config(runner: Box<dyn CommandRunner + Send + Sync>) -> RuntimeSession {
    activate_dev_config(RuntimeSession::new_with_runner_for_test(
        AppState::new(),
        runner,
    ))
    .await
}

async fn session_with_dev_config_and_clipboard(
    runner: Box<dyn CommandRunner + Send + Sync>,
    clipboard: Box<dyn ClipboardWriter + Send + Sync>,
) -> RuntimeSession {
    activate_dev_config(RuntimeSession::new_with_runner_and_clipboard_for_test(
        AppState::new(),
        runner,
        clipboard,
    ))
    .await
}

async fn activate_dev_config(mut session: RuntimeSession) -> RuntimeSession {
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

/// `ResourcesAction::Refresh` with a detail view open emits
/// `RefreshComputeInstances` *and* `LoadComputeInstanceDetail` in one batch.
/// Both open a status-bar operation, so a per-effect bump of the supersession
/// stamp let the detail load invalidate the list fetch its own batch had just
/// started.
#[tokio::test]
async fn refresh_with_a_detail_view_open_applies_the_list_result() {
    let runner = RoutingRunner::new(
        vec![list_json("api-dev-1"), list_json("refreshed-dev-1")],
        include_str!("fixtures/gce_instance_detail.json").to_string(),
    );
    let mut session = session_with_dev_config(Box::new(runner)).await;

    session
        .dispatch_action(Action::Resources(ResourcesAction::Refresh))
        .await;
    assert!(session.process_next_effect_result().await);
    assert_eq!(instance_names(&session), vec!["api-dev-1".to_string()]);

    session
        .dispatch_action(Action::Resources(ResourcesAction::OpenSelected))
        .await;
    assert!(
        session.app().root.resources.detail().is_some(),
        "the detail view must be open for this batch to contain two resource effects"
    );

    session
        .dispatch_action(Action::Resources(ResourcesAction::Refresh))
        .await;
    assert!(session.process_next_effect_result().await);

    assert_eq!(
        instance_names(&session),
        vec!["refreshed-dev-1".to_string()],
        "the list result of a refresh batch must not be dropped by that same batch's detail load"
    );
}

/// The property the per-batch bump must not trade away: a *newer* batch still
/// supersedes an older in-flight list fetch. Applying the stale rows instead
/// would swap a stale-drop bug for a stale-apply bug, which is strictly worse.
#[tokio::test]
async fn a_newer_refresh_batch_still_supersedes_an_older_in_flight_list_fetch() {
    let runner = BlockingListRunner::new(vec![list_json("stale-dev-1"), list_json("latest-dev-1")]);
    let mut session = session_with_dev_config(Box::new(runner.clone())).await;

    session
        .dispatch_action(Action::Resources(ResourcesAction::Refresh))
        .await;
    let wait_runner = runner.clone();
    tokio::task::spawn_blocking(move || wait_runner.wait_for_first_list())
        .await
        .unwrap();

    // A second batch, dispatched while the first fetch is still gated.
    session
        .dispatch_action(Action::Resources(ResourcesAction::Refresh))
        .await;

    // The ungated second fetch resolves first.
    assert!(session.process_next_effect_result().await);
    assert_eq!(instance_names(&session), vec!["latest-dev-1".to_string()]);

    runner.release_first_list();
    assert!(session.process_next_effect_result().await);
    assert_eq!(
        instance_names(&session),
        vec!["latest-dev-1".to_string()],
        "the older batch's rows must stay dropped once a newer batch has refreshed the list"
    );
}

/// The bump is lazy, and that laziness is load-bearing: it happens on
/// the first effect of the batch that opens a status-bar operation, not
/// unconditionally at the top of `dispatch_effects`. So a batch that opens no
/// operation — a clipboard copy, a log export, a tunnel start — must leave an
/// in-flight background fetch alone. Moving the bump to the top of
/// `dispatch_effects` leaves the rest of this file green while silently
/// converting every such batch into a supersession event: the user's list fetch
/// would be dropped, and with it the rows they were owed.
#[tokio::test]
async fn a_batch_that_opens_no_operation_does_not_supersede_an_in_flight_fetch() {
    let runner = BlockingListRunner::new(vec![list_json("in-flight-dev-1")]);
    let clipboard = RecordingClipboard::default();
    let mut session = session_with_dev_config_and_clipboard(
        Box::new(runner.clone()),
        Box::new(clipboard.clone()),
    )
    .await;

    session
        .dispatch_action(Action::Resources(ResourcesAction::Refresh))
        .await;
    let wait_runner = runner.clone();
    tokio::task::spawn_blocking(move || wait_runner.wait_for_first_list())
        .await
        .unwrap();

    // The intervening batch: one effect, and `effect_operation` returns `None`
    // for it, so nothing about it should touch the supersession stamp.
    session
        .dispatch_action(Action::Effect(Effect::CopyToClipboard {
            value: "api-dev-1".to_string(),
            description: "instance name".to_string(),
        }))
        .await;
    assert_eq!(
        clipboard.values(),
        vec!["api-dev-1".to_string()],
        "the intervening batch must really have dispatched an effect, or this test proves nothing"
    );

    runner.release_first_list();
    assert!(session.process_next_effect_result().await);
    assert_eq!(
        instance_names(&session),
        vec!["in-flight-dev-1".to_string()],
        "a batch that opens no status-bar operation must not drop an in-flight list fetch"
    );
}
