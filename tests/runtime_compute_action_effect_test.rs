use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use nuvem::app::state::resources::ResourceLoadPhase;
use nuvem::app::state::system::OperationPhase;
use nuvem::app::{
    Action, AppState, Effect, EffectResult, ResourcesAction, Workspace, WorkspaceAction,
};
use nuvem::services::gcloud::cli::{CommandError, CommandOutput, CommandRunner, GcloudCommand};
use nuvem::services::runtime::clipboard::ClipboardWriter;
use nuvem::services::runtime::session::RuntimeSession;
use nuvem_domain::gcp::configuration::{GcloudConfiguration, GcloudConfigurationName};
use nuvem_domain::gcp::resource::{ComputeInstance, ResourceServiceKind, ResourceStatus};

/// Deadline for every gate wait in this file. The tests release their gates
/// within milliseconds, so 30s sits orders of magnitude above any plausible
/// scheduling delay on a loaded CI runner: exceeding it proves the gate wedged
/// rather than that the machine was slow. The bound matters because these waits
/// run on `spawn_blocking` threads, where an unbounded park is uncancellable
/// and makes `Runtime::drop` hang forever when a test panics before releasing.
const GATE_TIMEOUT: Duration = Duration::from_secs(30);

/// Operations-log id for results whose test asserts nothing about the
/// operations log — here, effects dispatched directly, without the reducer that
/// would seed an entry. It sits above every id `begin` would hand out in these
/// tests, so such a result can never settle a real entry by accident: `0` would
/// be safe only for as long as no test in this file seeds one, and that is a
/// property of today's file rather than of the constant.
const UNTRACKED_OPERATION_ID: u64 = 9_000;

#[derive(Clone)]
struct FakeRunner {
    state: Arc<FakeRunnerState>,
}

struct FakeRunnerState {
    outputs: Mutex<Vec<Result<CommandOutput, CommandError>>>,
    commands: Mutex<Vec<GcloudCommand>>,
}

#[derive(Clone, Default)]
struct FakeClipboard {
    values: Arc<Mutex<Vec<String>>>,
    failure: Option<String>,
}

impl FakeClipboard {
    fn failing(error: impl Into<String>) -> Self {
        Self {
            values: Arc::new(Mutex::new(Vec::new())),
            failure: Some(error.into()),
        }
    }

    fn values(&self) -> Vec<String> {
        self.values.lock().unwrap().clone()
    }
}

impl ClipboardWriter for FakeClipboard {
    fn set_text(&self, value: &str) -> Result<(), String> {
        if let Some(error) = &self.failure {
            return Err(error.clone());
        }
        self.values.lock().unwrap().push(value.to_string());
        Ok(())
    }
}

#[derive(Clone)]
struct BlockingListRunner {
    state: Arc<BlockingListRunnerState>,
}

struct BlockingListRunnerState {
    list_count: Mutex<usize>,
    first_list_started: (Mutex<bool>, Condvar),
    release_first_list: (Mutex<bool>, Condvar),
    list_outputs: Vec<String>,
    non_list_error: Option<String>,
    gate_timeout: Duration,
}

impl BlockingListRunner {
    fn new(list_outputs: Vec<String>) -> Self {
        Self::build(list_outputs, None, GATE_TIMEOUT)
    }

    fn with_non_list_error(list_outputs: Vec<String>, non_list_error: Option<&str>) -> Self {
        Self::build(list_outputs, non_list_error, GATE_TIMEOUT)
    }

    /// Same as [`BlockingListRunner::new`] but with an injectable gate
    /// deadline, so the timeout tests can pin the bound without spending the
    /// full `GATE_TIMEOUT`.
    fn with_gate_timeout(list_outputs: Vec<String>, gate_timeout: Duration) -> Self {
        Self::build(list_outputs, None, gate_timeout)
    }

    fn build(
        list_outputs: Vec<String>,
        non_list_error: Option<&str>,
        gate_timeout: Duration,
    ) -> Self {
        Self {
            state: Arc::new(BlockingListRunnerState {
                list_count: Mutex::new(0),
                first_list_started: (Mutex::new(false), Condvar::new()),
                release_first_list: (Mutex::new(false), Condvar::new()),
                list_outputs,
                non_list_error: non_list_error.map(str::to_string),
                gate_timeout,
            }),
        }
    }

    /// Blocks until the first list call has begun, giving up at the gate
    /// deadline rather than parking forever.
    fn wait_for_first_list(&self) {
        let deadline = Instant::now() + self.state.gate_timeout;
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
                    "BlockingListRunner::wait_for_first_list timed out after {:?}: the first list call never started",
                    self.state.gate_timeout
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

    fn list_call_count(&self) -> usize {
        *self.state.list_count.lock().unwrap()
    }
}

impl CommandRunner for BlockingListRunner {
    fn run(&self, command: GcloudCommand) -> Result<CommandOutput, CommandError> {
        if command.args.ends_with(&[
            "compute".to_string(),
            "instances".to_string(),
            "list".to_string(),
            "--format=json".to_string(),
            "--limit=500".to_string(),
        ]) {
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
                // `Runtime::drop` for good if a test panicked before
                // releasing. Bound it and panic loudly: a panicking blocking
                // task surfaces as a `JoinError`, a hang does not.
                let deadline = Instant::now() + self.state.gate_timeout;
                let (released, wake) = &self.state.release_first_list;
                let mut released = released.lock().unwrap();
                while !*released {
                    // Budget the remaining time against the single deadline so
                    // a spurious wakeup cannot restart the full timeout.
                    let remaining = deadline.saturating_duration_since(Instant::now());
                    if remaining.is_zero() {
                        // Format first, then release the lock: panicking with
                        // the guard still held would poison the mutex and turn
                        // any later wait on this gate into a `PoisonError`
                        // instead of its own diagnostic.
                        let message = format!(
                            "BlockingListRunner::run timed out after {:?} waiting on release_first_list",
                            self.state.gate_timeout
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
        if let Some(stderr) = &self.state.non_list_error {
            return Err(CommandError::Failed {
                stdout: String::new(),
                stderr: stderr.clone(),
                exit_code: None,
            });
        }
        output("")
    }
}

/// The `compute instances list` argv tail that `BlockingListRunner` gates on.
fn list_command() -> GcloudCommand {
    GcloudCommand::new([
        "compute",
        "instances",
        "list",
        "--format=json",
        "--limit=500",
    ])
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

/// Both `BlockingListRunner` gates park `spawn_blocking` threads. An unbounded
/// park there is uncancellable, so a test that panics before releasing the gate
/// leaves `Runtime::drop` blocked on it forever and wedges the whole binary.
/// This pins the bound on the start gate: no list call ever runs, so the waiter
/// must give up rather than park.
#[test]
fn blocking_list_runner_wait_for_first_list_gives_up_when_no_list_starts() {
    let gate_timeout = Duration::from_millis(50);
    let runner = BlockingListRunner::with_gate_timeout(Vec::new(), gate_timeout);

    let worker = thread::spawn(move || runner.wait_for_first_list());

    let message = panic_message_of_bounded_wait(worker, gate_timeout);
    assert!(
        message.contains("BlockingListRunner::wait_for_first_list timed out"),
        "panic should name the waiter, got: {message}"
    );
}

/// The release gate is the wait that actually wedges `Runtime::drop`: it runs
/// inside the refresh task's `spawn_blocking`, so an unbounded park there
/// survives the test's unwind. Nothing ever calls `release_first_list`, so the
/// runner must give up instead.
#[test]
fn blocking_list_runner_run_gives_up_when_the_first_list_is_never_released() {
    let gate_timeout = Duration::from_millis(50);
    let runner = BlockingListRunner::with_gate_timeout(Vec::new(), gate_timeout);

    let worker = thread::spawn(move || {
        let _ = runner.run(list_command());
    });

    let message = panic_message_of_bounded_wait(worker, gate_timeout);
    assert!(
        message.contains("BlockingListRunner::run timed out"),
        "panic should name the waiter, got: {message}"
    );
    assert!(
        message.contains("release_first_list"),
        "panic should name the gate it was waiting on, got: {message}"
    );
}

impl FakeRunner {
    fn new(outputs: Vec<Result<CommandOutput, CommandError>>) -> Self {
        Self {
            state: Arc::new(FakeRunnerState {
                outputs: Mutex::new(outputs),
                commands: Mutex::new(Vec::new()),
            }),
        }
    }

    fn commands(&self) -> Vec<GcloudCommand> {
        self.state.commands.lock().unwrap().clone()
    }
}

impl CommandRunner for FakeRunner {
    fn run(&self, command: GcloudCommand) -> Result<CommandOutput, CommandError> {
        self.state.commands.lock().unwrap().push(command);
        self.state.outputs.lock().unwrap().remove(0)
    }
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

#[tokio::test]
async fn start_compute_instance_effect_refreshes_rows_in_background() {
    let runner = FakeRunner::new(vec![
        output(""),
        output(include_str!("fixtures/gce_instances.json")),
    ]);
    let mut session =
        RuntimeSession::new_with_runner_for_test(AppState::new(), Box::new(runner.clone()));
    session
        .apply_effect_result(EffectResult::ConfigurationsLoaded {
            configurations: vec![config()],
        })
        .await;
    let context = session
        .app()
        .root
        .current_context()
        .expect("context")
        .clone();
    session
        .apply_effect_result(EffectResult::ComputeInstancesLoaded {
            context: context.clone(),
            instances: vec![instance("api-dev-1")],
        })
        .await;

    session
        .dispatch_action(Action::Effect(Effect::ComputeInstanceAction {
            context: context.clone(),
            instance: instance("api-dev-1"),
            action: nuvem::app::ComputeInstanceAction::Start,
            operation_id: UNTRACKED_OPERATION_ID,
        }))
        .await;
    // The action runs on a blocking thread; its result reaches
    // the reducer through the queue.
    assert!(session.process_next_effect_result().await);

    assert_eq!(
        session.app().root.system.status_message.as_deref(),
        Some("Start accepted for api-dev-1; refreshing GCE instances")
    );
    assert_eq!(
        session.app().root.system.operation.phase,
        OperationPhase::Refreshing
    );
    assert_eq!(session.app().root.system.operation.label, "GCE instances");
    assert!(session.app().root.resources.is_stale());
    assert_eq!(runner.commands().len(), 1);

    assert!(session.process_next_effect_result().await);

    assert!(!session.app().root.resources.is_stale());
    assert_eq!(session.app().root.resources.instances().len(), 2);
    assert_eq!(session.app().root.system.status_message, None);
    assert_eq!(
        session.app().root.system.operation.phase,
        OperationPhase::Healthy
    );
    assert_eq!(session.app().root.system.operation.label, "GCE instances");
    assert!(session.app().root.system.last_refresh_at.is_some());
    assert_eq!(runner.commands().len(), 2);
    assert_eq!(
        runner.commands()[0].args,
        vec![
            "--configuration",
            "dev",
            "compute",
            "instances",
            "start",
            "api-dev-1",
            "--zone",
            "europe-west1-b",
        ]
        .into_iter()
        .map(str::to_string)
        .collect::<Vec<_>>()
    );
    assert_eq!(
        runner.commands()[1].args,
        vec![
            "--configuration",
            "dev",
            "compute",
            "instances",
            "list",
            "--format=json",
            "--limit=500",
        ]
        .into_iter()
        .map(str::to_string)
        .collect::<Vec<_>>()
    );
}

#[tokio::test]
async fn stop_compute_instance_effect_refreshes_rows_in_background() {
    let runner = FakeRunner::new(vec![
        output(""),
        output(include_str!("fixtures/gce_instances.json")),
    ]);
    let mut session =
        RuntimeSession::new_with_runner_for_test(AppState::new(), Box::new(runner.clone()));
    session
        .apply_effect_result(EffectResult::ConfigurationsLoaded {
            configurations: vec![config()],
        })
        .await;
    let context = session
        .app()
        .root
        .current_context()
        .expect("context")
        .clone();
    session
        .apply_effect_result(EffectResult::ComputeInstancesLoaded {
            context: context.clone(),
            instances: vec![instance("api-dev-1")],
        })
        .await;

    session
        .dispatch_action(Action::Effect(Effect::ComputeInstanceAction {
            context,
            instance: instance("api-dev-1"),
            action: nuvem::app::ComputeInstanceAction::Stop,
            operation_id: UNTRACKED_OPERATION_ID,
        }))
        .await;
    // The action runs on a blocking thread; its result reaches
    // the reducer through the queue.
    assert!(session.process_next_effect_result().await);

    assert_eq!(
        session.app().root.system.status_message.as_deref(),
        Some("Stop accepted for api-dev-1; refreshing GCE instances")
    );
    assert!(session.app().root.resources.is_stale());
    assert_eq!(runner.commands().len(), 1);
    assert!(session.process_next_effect_result().await);
    assert!(!session.app().root.resources.is_stale());
    assert_eq!(session.app().root.resources.instances().len(), 2);
    assert_eq!(session.app().root.system.status_message, None);
    assert_eq!(runner.commands().len(), 2);
}

#[tokio::test]
async fn refresh_failure_keeps_rows_stale_and_reports_error() {
    let runner = FakeRunner::new(vec![
        output(""),
        Err(CommandError::Failed {
            stdout: String::new(),
            stderr: "refresh failed".to_string(),
            exit_code: None,
        }),
    ]);
    let mut session =
        RuntimeSession::new_with_runner_for_test(AppState::new(), Box::new(runner.clone()));
    session
        .apply_effect_result(EffectResult::ConfigurationsLoaded {
            configurations: vec![config()],
        })
        .await;
    let context = session
        .app()
        .root
        .current_context()
        .expect("context")
        .clone();
    session
        .apply_effect_result(EffectResult::ComputeInstancesLoaded {
            context: context.clone(),
            instances: vec![instance("api-dev-1")],
        })
        .await;

    session
        .dispatch_action(Action::Effect(Effect::ComputeInstanceAction {
            context,
            instance: instance("api-dev-1"),
            action: nuvem::app::ComputeInstanceAction::Start,
            operation_id: UNTRACKED_OPERATION_ID,
        }))
        .await;
    // The action runs on a blocking thread; its result reaches
    // the reducer through the queue.
    assert!(session.process_next_effect_result().await);

    assert_eq!(
        session.app().root.system.status_message.as_deref(),
        Some("Start accepted for api-dev-1; refreshing GCE instances")
    );
    assert_eq!(runner.commands().len(), 1);
    assert!(session.process_next_effect_result().await);
    assert!(session.app().root.resources.is_stale());
    assert!(session.app().root.resources.instances().is_empty());
    let status = session.app().root.system.status_message.clone();
    assert_eq!(
        status.as_deref(),
        Some("unknown error — Retry, or copy the details to investigate")
    );
    assert!(
        !status
            .as_deref()
            .unwrap_or_default()
            .contains("refresh failed")
    );
    assert_eq!(runner.commands().len(), 2);
}

#[tokio::test]
async fn stop_compute_instance_effect_sets_failure_status_and_clears_rows() {
    let runner = FakeRunner::new(vec![Err(CommandError::Failed {
        stdout: String::new(),
        stderr: "stop failed".to_string(),
        exit_code: None,
    })]);
    let mut session =
        RuntimeSession::new_with_runner_for_test(AppState::new(), Box::new(runner.clone()));
    session
        .apply_effect_result(EffectResult::ConfigurationsLoaded {
            configurations: vec![config()],
        })
        .await;
    let context = session
        .app()
        .root
        .current_context()
        .expect("context")
        .clone();
    session
        .apply_effect_result(EffectResult::ComputeInstancesLoaded {
            context: context.clone(),
            instances: vec![instance("api-dev-1")],
        })
        .await;
    let refresh_time_before_action = session.app().root.system.last_refresh_at;

    session
        .dispatch_action(Action::Effect(Effect::ComputeInstanceAction {
            context,
            instance: instance("api-dev-1"),
            action: nuvem::app::ComputeInstanceAction::Stop,
            operation_id: UNTRACKED_OPERATION_ID,
        }))
        .await;
    // The action runs on a blocking thread; its result reaches
    // the reducer through the queue.
    assert!(session.process_next_effect_result().await);

    assert!(session.app().root.resources.is_stale());
    assert!(session.app().root.resources.instances().is_empty());
    assert_eq!(session.app().root.resources.selected_instance(), None);
    assert_eq!(
        session.app().root.system.status_message.as_deref(),
        Some("unknown error — Retry, or copy the details to investigate")
    );
    assert_eq!(
        session.app().root.system.operation.phase,
        OperationPhase::Failed
    );
    assert_eq!(session.app().root.system.operation.label, "GCE action");
    assert_eq!(
        session.app().root.system.last_refresh_at,
        refresh_time_before_action
    );
    assert_eq!(
        runner.commands()[0].args,
        vec![
            "--configuration",
            "dev",
            "compute",
            "instances",
            "stop",
            "api-dev-1",
            "--zone",
            "europe-west1-b",
        ]
        .into_iter()
        .map(str::to_string)
        .collect::<Vec<_>>()
    );
}

async fn dispatch_action_for_test(
    action: nuvem::app::ComputeInstanceAction,
    runner: FakeRunner,
) -> RuntimeSession {
    let mut session = RuntimeSession::new_with_runner_for_test(AppState::new(), Box::new(runner));
    session
        .apply_effect_result(EffectResult::ConfigurationsLoaded {
            configurations: vec![config()],
        })
        .await;
    let context = session
        .app()
        .root
        .current_context()
        .expect("context")
        .clone();
    session
        .dispatch_action(Action::Effect(Effect::ComputeInstanceAction {
            context,
            instance: instance("api-dev-1"),
            action,
            operation_id: UNTRACKED_OPERATION_ID,
        }))
        .await;
    // The action itself runs on a blocking thread, so its result
    // reaches the reducer through the queue rather than inside the dispatch.
    assert!(session.process_next_effect_result().await);
    session
}

#[tokio::test]
async fn reset_compute_instance_effect_dispatches_and_succeeds() {
    let runner = FakeRunner::new(vec![output(""), output("")]);
    let session =
        dispatch_action_for_test(nuvem::app::ComputeInstanceAction::Reset, runner.clone()).await;

    assert_eq!(
        session.app().root.system.status_message.as_deref(),
        Some("Reset accepted for api-dev-1; refreshing GCE instances")
    );
    assert_eq!(
        runner.commands()[0].args,
        vec![
            "--configuration",
            "dev",
            "compute",
            "instances",
            "reset",
            "api-dev-1",
            "--zone",
            "europe-west1-b",
        ]
        .into_iter()
        .map(str::to_string)
        .collect::<Vec<_>>()
    );
}

#[tokio::test]
async fn suspend_compute_instance_effect_dispatches_and_succeeds() {
    let runner = FakeRunner::new(vec![output(""), output("")]);
    let session =
        dispatch_action_for_test(nuvem::app::ComputeInstanceAction::Suspend, runner.clone()).await;

    assert_eq!(
        session.app().root.system.status_message.as_deref(),
        Some("Suspend accepted for api-dev-1; refreshing GCE instances")
    );
    assert_eq!(
        runner.commands()[0].args,
        vec![
            "--configuration",
            "dev",
            "compute",
            "instances",
            "suspend",
            "api-dev-1",
            "--zone",
            "europe-west1-b",
        ]
        .into_iter()
        .map(str::to_string)
        .collect::<Vec<_>>()
    );
}

#[tokio::test]
async fn resume_compute_instance_effect_dispatches_and_succeeds() {
    let runner = FakeRunner::new(vec![output(""), output("")]);
    let session =
        dispatch_action_for_test(nuvem::app::ComputeInstanceAction::Resume, runner.clone()).await;

    assert_eq!(
        session.app().root.system.status_message.as_deref(),
        Some("Resume accepted for api-dev-1; refreshing GCE instances")
    );
    assert_eq!(
        runner.commands()[0].args,
        vec![
            "--configuration",
            "dev",
            "compute",
            "instances",
            "resume",
            "api-dev-1",
            "--zone",
            "europe-west1-b",
        ]
        .into_iter()
        .map(str::to_string)
        .collect::<Vec<_>>()
    );
}

#[tokio::test]
async fn copy_to_clipboard_effect_writes_the_value_and_reports_success() {
    let clipboard = FakeClipboard::default();
    let mut session = RuntimeSession::new_with_runner_and_clipboard_for_test(
        AppState::new(),
        Box::new(FakeRunner::new(Vec::new())),
        Box::new(clipboard.clone()),
    );

    session
        .dispatch_action(Action::Effect(Effect::CopyToClipboard {
            value: "gcloud compute ssh api-dev-1".to_string(),
            description: "SSH command".to_string(),
        }))
        .await;

    assert_eq!(clipboard.values(), vec!["gcloud compute ssh api-dev-1"]);
    assert_eq!(
        session.app().root.system.status_message.as_deref(),
        Some("Copied SSH command to clipboard")
    );
}

#[tokio::test]
async fn copy_to_clipboard_effect_reports_writer_failure() {
    let clipboard = FakeClipboard::failing("clipboard unavailable");
    let mut session = RuntimeSession::new_with_runner_and_clipboard_for_test(
        AppState::new(),
        Box::new(FakeRunner::new(Vec::new())),
        Box::new(clipboard),
    );

    session
        .dispatch_action(Action::Effect(Effect::CopyToClipboard {
            value: "api-dev-1".to_string(),
            description: "instance name".to_string(),
        }))
        .await;

    assert_eq!(
        session.app().root.system.status_message.as_deref(),
        Some("unknown error — Retry, or copy the details to investigate")
    );
}

#[tokio::test]
async fn background_refresh_is_ignored_after_switching_contexts_and_returning() {
    let runner = BlockingListRunner::new(vec![include_str!("fixtures/gce_instances.json").into()]);
    let mut session =
        RuntimeSession::new_with_runner_for_test(AppState::new(), Box::new(runner.clone()));
    session
        .apply_effect_result(EffectResult::ConfigurationsLoaded {
            configurations: vec![config()],
        })
        .await;
    let context = session
        .app()
        .root
        .current_context()
        .expect("context")
        .clone();

    session
        .dispatch_action(Action::Effect(Effect::RefreshComputeInstances {
            context,
            filter: None,
        }))
        .await;
    let wait_runner = runner.clone();
    tokio::task::spawn_blocking(move || wait_runner.wait_for_first_list())
        .await
        .unwrap();

    session
        .apply_effect_result(EffectResult::ConfigurationsLoaded {
            configurations: vec![
                GcloudConfiguration {
                    is_active: false,
                    ..config()
                },
                GcloudConfiguration {
                    name: GcloudConfigurationName::new("prod"),
                    is_active: true,
                    project: Some("prod-project".to_string()),
                    ..config()
                },
            ],
        })
        .await;
    session
        .apply_effect_result(EffectResult::ConfigurationsLoaded {
            configurations: vec![config()],
        })
        .await;
    let operation_before_stale_result = session.app().root.system.operation.clone();
    let refresh_time_before_stale_result = session.app().root.system.last_refresh_at;
    runner.release_first_list();

    assert!(session.process_next_effect_result().await);
    assert!(session.app().root.resources.is_stale());
    assert_eq!(
        session.app().root.system.operation,
        operation_before_stale_result
    );
    assert_eq!(
        session.app().root.system.last_refresh_at,
        refresh_time_before_stale_result
    );
}

#[tokio::test]
async fn background_refresh_is_ignored_after_switching_workspace() {
    let runner = BlockingListRunner::new(vec![include_str!("fixtures/gce_instances.json").into()]);
    let mut session =
        RuntimeSession::new_with_runner_for_test(AppState::new(), Box::new(runner.clone()));
    session
        .apply_effect_result(EffectResult::ConfigurationsLoaded {
            configurations: vec![config()],
        })
        .await;
    let context = session
        .app()
        .root
        .current_context()
        .expect("context")
        .clone();

    session
        .dispatch_action(Action::Effect(Effect::RefreshComputeInstances {
            context,
            filter: None,
        }))
        .await;
    let wait_runner = runner.clone();
    tokio::task::spawn_blocking(move || wait_runner.wait_for_first_list())
        .await
        .unwrap();

    session
        .dispatch_action(Action::Workspace(WorkspaceAction::Configurations))
        .await;
    let operation_before_stale_result = session.app().root.system.operation.clone();
    let refresh_time_before_stale_result = session.app().root.system.last_refresh_at;
    runner.release_first_list();

    assert!(session.process_next_effect_result().await);
    assert_eq!(
        session.app().root.system.operation,
        operation_before_stale_result
    );
    assert_eq!(
        session.app().root.system.last_refresh_at,
        refresh_time_before_stale_result
    );
}

#[tokio::test]
async fn background_refresh_is_ignored_after_switching_resource_service() {
    let runner = BlockingListRunner::new(vec![include_str!("fixtures/gce_instances.json").into()]);
    let mut session =
        RuntimeSession::new_with_runner_for_test(AppState::new(), Box::new(runner.clone()));
    session
        .apply_effect_result(EffectResult::ConfigurationsLoaded {
            configurations: vec![config()],
        })
        .await;
    let context = session
        .app()
        .root
        .current_context()
        .expect("context")
        .clone();

    session
        .dispatch_action(Action::Effect(Effect::RefreshComputeInstances {
            context,
            filter: None,
        }))
        .await;
    let wait_runner = runner.clone();
    tokio::task::spawn_blocking(move || wait_runner.wait_for_first_list())
        .await
        .unwrap();

    session
        .dispatch_action(Action::Resources(ResourcesAction::SelectService(
            ResourceServiceKind::GkeClusters,
        )))
        .await;
    // Switching service auto-loads GKE clusters; settle that result before
    // snapshotting so the assertion targets the stale compute refresh below.
    assert!(session.process_next_effect_result().await);
    let operation_before_stale_result = session.app().root.system.operation.clone();
    let refresh_time_before_stale_result = session.app().root.system.last_refresh_at;
    runner.release_first_list();

    assert!(session.process_next_effect_result().await);
    assert_eq!(
        session.app().root.system.operation,
        operation_before_stale_result
    );
    assert_eq!(
        session.app().root.system.last_refresh_at,
        refresh_time_before_stale_result
    );
}

#[tokio::test]
async fn background_refresh_is_ignored_after_a_newer_configuration_operation() {
    let runner = BlockingListRunner::new(vec![include_str!("fixtures/gce_instances.json").into()]);
    let mut session =
        RuntimeSession::new_with_runner_for_test(AppState::new(), Box::new(runner.clone()));
    session
        .apply_effect_result(EffectResult::ConfigurationsLoaded {
            configurations: vec![config()],
        })
        .await;
    let context = session
        .app()
        .root
        .current_context()
        .expect("context")
        .clone();

    session
        .dispatch_action(Action::Effect(Effect::RefreshComputeInstances {
            context,
            filter: None,
        }))
        .await;
    let wait_runner = runner.clone();
    tokio::task::spawn_blocking(move || wait_runner.wait_for_first_list())
        .await
        .unwrap();

    session
        .dispatch_action(Action::Effect(Effect::LoadConfigurations))
        .await;
    let operation_before_stale_result = session.app().root.system.operation.clone();
    let refresh_time_before_stale_result = session.app().root.system.last_refresh_at;
    runner.release_first_list();

    assert!(session.process_next_effect_result().await);
    assert!(session.app().root.resources.instances().is_empty());
    assert_eq!(
        session.app().root.system.operation,
        operation_before_stale_result
    );
    assert_eq!(
        session.app().root.system.last_refresh_at,
        refresh_time_before_stale_result
    );
}

#[tokio::test]
async fn background_refresh_is_ignored_after_a_newer_manual_refresh() {
    let latest_instances = r#"[
      {
        "id": "2001",
        "name": "latest-dev-1",
        "zone": "https://www.googleapis.com/compute/v1/projects/dev-project/zones/europe-west1-b",
        "status": "RUNNING",
        "machineType": "https://www.googleapis.com/compute/v1/projects/dev-project/zones/europe-west1-b/machineTypes/e2-medium"
      }
    ]"#;
    let runner = BlockingListRunner::new(vec![
        include_str!("fixtures/gce_instances.json").into(),
        latest_instances.into(),
    ]);
    let mut session =
        RuntimeSession::new_with_runner_for_test(AppState::new(), Box::new(runner.clone()));
    session
        .apply_effect_result(EffectResult::ConfigurationsLoaded {
            configurations: vec![config()],
        })
        .await;
    let context = session
        .app()
        .root
        .current_context()
        .expect("context")
        .clone();

    session
        .dispatch_action(Action::Effect(Effect::RefreshComputeInstances {
            context: context.clone(),
            filter: None,
        }))
        .await;
    let wait_runner = runner.clone();
    tokio::task::spawn_blocking(move || wait_runner.wait_for_first_list())
        .await
        .unwrap();
    session
        .dispatch_action(Action::Effect(Effect::RefreshComputeInstances {
            context,
            filter: None,
        }))
        .await;

    // The second (non-blocked) refresh resolves first with the latest rows.
    assert!(session.process_next_effect_result().await);
    assert_eq!(
        session.app().root.resources.instances()[0].name,
        "latest-dev-1"
    );

    // Releasing the first (older-generation) refresh: its result is ignored.
    runner.release_first_list();
    assert!(session.process_next_effect_result().await);
    assert_eq!(
        session.app().root.resources.instances()[0].name,
        "latest-dev-1"
    );
}

#[tokio::test]
async fn background_refresh_does_not_clear_a_later_action_failure() {
    let runner = BlockingListRunner::with_non_list_error(
        vec![include_str!("fixtures/gce_instances.json").into()],
        Some("stop failed"),
    );
    let mut session =
        RuntimeSession::new_with_runner_for_test(AppState::new(), Box::new(runner.clone()));
    session
        .apply_effect_result(EffectResult::ConfigurationsLoaded {
            configurations: vec![config()],
        })
        .await;
    let context = session
        .app()
        .root
        .current_context()
        .expect("context")
        .clone();

    session
        .dispatch_action(Action::Effect(Effect::RefreshComputeInstances {
            context: context.clone(),
            filter: None,
        }))
        .await;
    let wait_runner = runner.clone();
    tokio::task::spawn_blocking(move || wait_runner.wait_for_first_list())
        .await
        .unwrap();
    session
        .dispatch_action(Action::Effect(Effect::ComputeInstanceAction {
            context,
            instance: instance("api-dev-1"),
            action: nuvem::app::ComputeInstanceAction::Stop,
            operation_id: UNTRACKED_OPERATION_ID,
        }))
        .await;
    // The action's own result, which is the only thing queued: the refresh is
    // still parked on the gate below. The action runs on a
    // blocking thread, so this is where its failure reaches the status line.
    assert!(session.process_next_effect_result().await);
    assert_eq!(
        session.app().root.system.status_message.as_deref(),
        Some("unknown error — Retry, or copy the details to investigate")
    );
    runner.release_first_list();

    assert!(session.process_next_effect_result().await);
    assert_eq!(
        session.app().root.system.status_message.as_deref(),
        Some("unknown error — Retry, or copy the details to investigate")
    );
}

#[tokio::test]
async fn background_refresh_preserves_newer_reducer_status_feedback() {
    let runner = BlockingListRunner::new(vec![include_str!("fixtures/gce_instances.json").into()]);
    let mut session =
        RuntimeSession::new_with_runner_for_test(AppState::new(), Box::new(runner.clone()));
    session
        .apply_effect_result(EffectResult::ConfigurationsLoaded {
            configurations: vec![config()],
        })
        .await;
    let context = session
        .app()
        .root
        .current_context()
        .expect("context")
        .clone();

    session
        .dispatch_action(Action::Effect(Effect::RefreshComputeInstances {
            context,
            filter: None,
        }))
        .await;
    let wait_runner = runner.clone();
    tokio::task::spawn_blocking(move || wait_runner.wait_for_first_list())
        .await
        .unwrap();

    session
        .dispatch_action(Action::Effect(Effect::CopyToClipboard {
            value: "value".to_string(),
            description: "field".to_string(),
        }))
        .await;
    let status_before_result = session.app().root.system.status_message.clone();
    let phase_before_result = session.app().root.system.status_phase;

    runner.release_first_list();
    assert!(session.process_next_effect_result().await);

    assert_eq!(
        session.app().root.system.status_message,
        status_before_result
    );
    assert_eq!(session.app().root.system.status_phase, phase_before_result);
}

#[tokio::test]
async fn poll_does_not_schedule_a_second_refresh_while_one_is_in_flight() {
    let runner = BlockingListRunner::new(vec![
        include_str!("fixtures/gce_instances.json").into(),
        include_str!("fixtures/gce_instances.json").into(),
    ]);
    let mut session =
        RuntimeSession::new_with_runner_for_test(AppState::new(), Box::new(runner.clone()));
    session
        .apply_effect_result(EffectResult::ConfigurationsLoaded {
            configurations: vec![config()],
        })
        .await;
    session
        .dispatch_action(Action::Workspace(WorkspaceAction::Resources))
        .await;
    // The auto-load's first list call is now blocked in the background.
    let wait_runner = runner.clone();
    tokio::task::spawn_blocking(move || wait_runner.wait_for_first_list())
        .await
        .unwrap();

    // A poll tick while a refresh is in flight must not issue a second command.
    session.poll_active_resource().await;
    assert_eq!(runner.list_call_count(), 1);

    runner.release_first_list();
    assert!(session.process_next_effect_result().await);
}

#[tokio::test]
async fn config_switch_while_a_refresh_is_in_flight_reloads_under_the_new_context() {
    let runner = BlockingListRunner::new(vec![
        include_str!("fixtures/gce_instances.json").into(), // first (blocked) list, old context
        include_str!("fixtures/gce_instances.json").into(), // second list, new context
    ]);
    let mut session =
        RuntimeSession::new_with_runner_for_test(AppState::new(), Box::new(runner.clone()));
    session
        .apply_effect_result(EffectResult::ConfigurationsLoaded {
            configurations: vec![config()],
        })
        .await;

    // Entering Resources auto-loads the active service; its first list call blocks.
    session
        .dispatch_action(Action::Workspace(WorkspaceAction::Resources))
        .await;
    let wait_runner = runner.clone();
    tokio::task::spawn_blocking(move || wait_runner.wait_for_first_list())
        .await
        .unwrap();

    // Switch the active configuration while the first refresh is still in flight.
    session
        .apply_effect_result(EffectResult::ConfigurationsLoaded {
            configurations: vec![other_config()],
        })
        .await;

    // Release the stale in-flight refresh: its result is dropped by the generation guard,
    // and its completion must re-trigger auto-load under the new configuration.
    runner.release_first_list();
    assert!(session.process_next_effect_result().await); // stale dev result dropped; prod load scheduled

    // The prod load must actually happen; a regression would leave nothing to receive here.
    // The scheduled refresh completes in milliseconds; the timeout only exists so a
    // regression that never schedules it fails instead of hanging. Sharing `GATE_TIMEOUT`
    // here widens the old 5s margin well past any scheduling stall on a loaded CI runner,
    // without weakening what the bound proves.
    let applied = tokio::time::timeout(GATE_TIMEOUT, session.process_next_effect_result())
        .await
        .expect("second refresh under the new context must be scheduled after the switch");
    assert!(applied);

    assert_eq!(runner.list_call_count(), 2);
    assert_eq!(session.app().root.resources.instances().len(), 2);
    assert_eq!(
        session
            .app()
            .root
            .resources
            .active_context_label()
            .as_deref(),
        Some("prod")
    );
}

// --- The dispatch plumbing, observed without waiting on a result -----------
//
// Every effect the reducer emits reaches gcloud through `dispatch_effects`,
// and every list refresh through `dispatch_background_refresh` under it. Both
// hand the fetch to a spawned task, so the usual way to observe them is to
// await the result — which is precisely what cannot be asserted on here: a
// plumbing regression makes the result never arrive, and a test that awaits it
// hangs instead of failing. The assertions below read the state the dispatch
// installs *synchronously*, before any task completes, so a severed dispatch
// fails in milliseconds.

/// A manual refresh marks the active service Loading as it dispatches.
///
/// The load phase is what the table renders as "Loading…" instead of "No
/// instances"; it is set where the fetch is spawned, so it is proof the fetch
/// was spawned at all. Asserted before any result is processed, so a dispatch
/// that silently did nothing cannot pass by being slow.
#[tokio::test]
async fn a_manual_refresh_marks_the_active_service_loading_as_it_dispatches() {
    let runner = FakeRunner::new(vec![output(include_str!("fixtures/gce_instances.json"))]);
    let mut session =
        RuntimeSession::new_with_runner_for_test(AppState::new(), Box::new(runner.clone()));
    session
        .apply_effect_result(EffectResult::ConfigurationsLoaded {
            configurations: vec![config()],
        })
        .await;
    // Set directly rather than through `WorkspaceAction::Resources`, which
    // would auto-load and leave a refresh in flight: the point here is that
    // *this* refresh starts one.
    session.app_mut().root.workspace = Workspace::Resources;
    assert_eq!(
        session.app().root.resources.active_load_phase(),
        &ResourceLoadPhase::Idle,
        "the fixture must start with nothing in flight"
    );

    session
        .dispatch_action(Action::Resources(ResourcesAction::Refresh))
        .await;

    assert!(
        matches!(
            session.app().root.resources.active_load_phase(),
            ResourceLoadPhase::Loading { .. }
        ),
        "a dispatched refresh must mark the service loading: {:?}",
        session.app().root.resources.active_load_phase()
    );
}

/// The poll tick refreshes the active service.
///
/// The ticker is the whole of the app's freshness story: with it severed the
/// table keeps painting whatever it last fetched, with no staleness marker,
/// because nothing failed. Observed through the load phase the scheduled
/// refresh installs, so nothing here waits on the fetch itself.
#[tokio::test]
async fn a_poll_tick_starts_a_refresh_of_the_active_service() {
    let runner = FakeRunner::new(vec![output(include_str!("fixtures/gce_instances.json"))]);
    let mut session =
        RuntimeSession::new_with_runner_for_test(AppState::new(), Box::new(runner.clone()));
    session
        .apply_effect_result(EffectResult::ConfigurationsLoaded {
            configurations: vec![config()],
        })
        .await;
    session.app_mut().root.workspace = Workspace::Resources;
    assert_eq!(
        session.app().root.resources.active_load_phase(),
        &ResourceLoadPhase::Idle,
        "the tick must be the only thing that could have started a load"
    );

    session.poll_active_resource().await;

    assert!(
        matches!(
            session.app().root.resources.active_load_phase(),
            ResourceLoadPhase::Loading { .. }
        ),
        "the poll tick must schedule a refresh of the active service: {:?}",
        session.app().root.resources.active_load_phase()
    );
}

/// A confirmed single-row lifecycle action runs off the run loop.
///
/// `Effect::ComputeInstanceAction` used to fall through `dispatch_effects` to
/// `dispatch_effect`, which called the blocking `runner.run` inline — awaited
/// from `dispatch_action` inside the run loop's `select!`. While it blocked
/// there was no `terminal.draw`, no key handling, no poll tick and no
/// termination arm, and the terminal is in raw mode so Ctrl-C arrives as a byte
/// rather than as a signal: the whole TUI froze for as long as the child ran.
/// The ceiling raised what "as long as the child ran" can mean for exactly these
/// two effects — a lifecycle mutation blocks until GCP finishes the operation —
/// so the hop off the loop is what makes that ceiling safe to give them. The
/// plugin dispatch has carried the same argument in its own docstring.
///
/// The observable is the state the result produces, not a timing: the rows are
/// marked stale by the success arm of the reducer. Previously that had
/// already happened by the time `dispatch_action` returned, because the command
/// had already run. Now it happens when the result is pumped off the queue.
#[tokio::test]
async fn a_confirmed_instance_action_is_not_run_on_the_run_loop() {
    let runner = FakeRunner::new(vec![
        output(""),
        output(include_str!("fixtures/gce_instances.json")),
    ]);
    let mut session =
        RuntimeSession::new_with_runner_for_test(AppState::new(), Box::new(runner.clone()));
    session
        .apply_effect_result(EffectResult::ConfigurationsLoaded {
            configurations: vec![config()],
        })
        .await;
    let context = session
        .app()
        .root
        .current_context()
        .expect("context")
        .clone();
    session
        .apply_effect_result(EffectResult::ComputeInstancesLoaded {
            context: context.clone(),
            instances: vec![instance("api-dev-1")],
        })
        .await;

    session
        .dispatch_action(Action::Effect(Effect::ComputeInstanceAction {
            context,
            instance: instance("api-dev-1"),
            action: nuvem::app::ComputeInstanceAction::Start,
            operation_id: UNTRACKED_OPERATION_ID,
        }))
        .await;

    assert!(
        !session.app().root.resources.is_stale(),
        "the action's result must not have been applied by the time dispatch returned"
    );
    assert_ne!(
        session.app().root.system.status_message.as_deref(),
        Some("Start accepted for api-dev-1; refreshing GCE instances"),
        "the run loop reported an outcome it could only have by running the child inline"
    );

    // The result is delivered off the queue like a plugin's, and only then does
    // the reducer see it.
    assert!(session.process_next_effect_result().await);
    assert!(session.app().root.resources.is_stale());
    assert_eq!(
        session.app().root.system.status_message.as_deref(),
        Some("Start accepted for api-dev-1; refreshing GCE instances")
    );
}
