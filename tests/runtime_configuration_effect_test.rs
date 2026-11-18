use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use nuvem::app::state::configurations::{
    ConfigurationContextPickerItem, ConfigurationContextPickerKind,
    ConfigurationContextPickerStatus,
};
use nuvem::app::state::system::OperationPhase;
use nuvem::app::{Action, AppError, AppState, ConfigurationsAction, Effect, EffectResult};
use nuvem::services::gcloud::cli::{
    CommandCeiling, CommandError, CommandOutput, CommandRunner, GcloudCommand,
};
use nuvem::services::gcloud::configurations::{GcloudConfigurationDraft, GcloudContextEditDraft};
use nuvem::services::runtime::session::RuntimeSession;
use nuvem_domain::gcp::configuration::{GcloudConfiguration, GcloudConfigurationName};
use nuvem_domain::gcp::resource::{ComputeInstance, ResourceStatus};

/// Deadline for every gate wait in this file. The test releases its gate within
/// milliseconds, so 30s sits orders of magnitude above any plausible scheduling
/// delay on a loaded CI runner: exceeding it proves the gate wedged rather than
/// that the machine was slow. The bound matters because these waits run on
/// `spawn_blocking` threads, where an unbounded park is uncancellable and makes
/// `Runtime::drop` hang forever when a test panics before releasing.
const GATE_TIMEOUT: Duration = Duration::from_secs(30);

/// Deadline for observing that a dropped session's cancellation token reached
/// the runner. Deliberately a separate constant from [`GATE_TIMEOUT`] despite
/// sharing a value: this bounds an *observation* of asynchronous shutdown, not
/// a gate a test itself releases, so the two can be retuned independently and
/// neither name has to stretch to cover the other.
const CANCELLATION_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Clone)]
struct FakeRunner {
    state: Arc<FakeRunnerState>,
}

struct FakeRunnerState {
    outputs: Mutex<Vec<Result<CommandOutput, CommandError>>>,
    commands: Mutex<Vec<GcloudCommand>>,
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

    fn remaining_output_count(&self) -> usize {
        self.state.outputs.lock().unwrap().len()
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

fn config(name: &str, active: bool, project: &str) -> GcloudConfiguration {
    GcloudConfiguration {
        name: GcloudConfigurationName::new(name),
        is_active: active,
        account: None,
        project: Some(project.to_string()),
        region: None,
        zone: None,
    }
}

fn config_with_context(
    name: &str,
    active: bool,
    account: Option<&str>,
    project: Option<&str>,
    region: Option<&str>,
    zone: Option<&str>,
) -> GcloudConfiguration {
    GcloudConfiguration {
        name: GcloudConfigurationName::new(name),
        is_active: active,
        account: account.map(str::to_string),
        project: project.map(str::to_string),
        region: region.map(str::to_string),
        zone: zone.map(str::to_string),
    }
}

#[tokio::test]
async fn project_picker_effect_runs_project_discovery_command() {
    let runner = FakeRunner::new(vec![output(include_str!("fixtures/gcloud_projects.json"))]);
    let runner_handle = runner.clone();
    let mut session = RuntimeSession::new_with_runner_for_test(AppState::new(), Box::new(runner));
    session
        .apply_effect_result(EffectResult::ConfigurationsLoaded {
            configurations: vec![config("dev", true, "dev-project")],
        })
        .await;

    session
        .dispatch_action(Action::Configurations(
            ConfigurationsAction::StartEditActiveContext,
        ))
        .await;
    session
        .dispatch_action(Action::Configurations(
            ConfigurationsAction::OpenContextPicker,
        ))
        .await;
    session.process_next_effect_result().await;

    let picker = session
        .app()
        .root
        .configurations
        .context_picker()
        .expect("project picker is open");
    assert_eq!(picker.kind(), ConfigurationContextPickerKind::Project);
    assert_eq!(picker.status(), &ConfigurationContextPickerStatus::Ready);
    assert_eq!(
        picker.selected_item(),
        Some(&ConfigurationContextPickerItem::Project(
            nuvem_domain::gcp::project::GcloudProject {
                project_id: "dev-project".to_string(),
                name: "Development".to_string(),
                lifecycle_state: Some("ACTIVE".to_string()),
            }
        ))
    );
    assert_eq!(
        runner_handle.commands()[0].args,
        vec![
            "--configuration",
            "dev",
            "projects",
            "list",
            "--format=json"
        ]
    );
}

#[tokio::test]
async fn region_picker_effect_runs_region_discovery_command() {
    let runner = FakeRunner::new(vec![output(include_str!(
        "fixtures/gcloud_compute_regions.json"
    ))]);
    let runner_handle = runner.clone();
    let mut session = RuntimeSession::new_with_runner_for_test(AppState::new(), Box::new(runner));
    session
        .apply_effect_result(EffectResult::ConfigurationsLoaded {
            configurations: vec![config("dev", true, "dev-project")],
        })
        .await;

    session
        .dispatch_action(Action::Configurations(
            ConfigurationsAction::StartEditActiveContext,
        ))
        .await;
    session
        .dispatch_action(Action::Configurations(
            ConfigurationsAction::FocusNextContextEditField,
        ))
        .await;
    session
        .dispatch_action(Action::Configurations(
            ConfigurationsAction::OpenContextPicker,
        ))
        .await;
    session.process_next_effect_result().await;

    let picker = session
        .app()
        .root
        .configurations
        .context_picker()
        .expect("region picker is open");
    assert_eq!(picker.kind(), ConfigurationContextPickerKind::Region);
    assert_eq!(picker.status(), &ConfigurationContextPickerStatus::Ready);
    assert_eq!(picker.visible_items().len(), 2);
    assert_eq!(
        runner_handle.commands()[0].args,
        vec![
            "--configuration",
            "dev",
            "compute",
            "regions",
            "list",
            "--format=json"
        ]
    );
}

#[tokio::test]
async fn zone_picker_effect_runs_zone_discovery_command() {
    let runner = FakeRunner::new(vec![output(include_str!(
        "fixtures/gcloud_compute_zones.json"
    ))]);
    let runner_handle = runner.clone();
    let mut session = RuntimeSession::new_with_runner_for_test(AppState::new(), Box::new(runner));
    session
        .apply_effect_result(EffectResult::ConfigurationsLoaded {
            configurations: vec![config("dev", true, "dev-project")],
        })
        .await;

    session
        .dispatch_action(Action::Configurations(
            ConfigurationsAction::StartEditActiveContext,
        ))
        .await;
    for _ in 0..2 {
        session
            .dispatch_action(Action::Configurations(
                ConfigurationsAction::FocusNextContextEditField,
            ))
            .await;
    }
    session
        .dispatch_action(Action::Configurations(
            ConfigurationsAction::OpenContextPicker,
        ))
        .await;
    session.process_next_effect_result().await;

    let picker = session
        .app()
        .root
        .configurations
        .context_picker()
        .expect("zone picker is open");
    assert_eq!(picker.kind(), ConfigurationContextPickerKind::Zone);
    assert_eq!(picker.status(), &ConfigurationContextPickerStatus::Ready);
    assert_eq!(picker.visible_items().len(), 2);
    assert_eq!(
        runner_handle.commands()[0].args,
        vec![
            "--configuration",
            "dev",
            "compute",
            "zones",
            "list",
            "--format=json"
        ]
    );
}

#[tokio::test]
async fn picker_discovery_failure_keeps_picker_degraded() {
    let runner = FakeRunner::new(vec![Err(CommandError::Failed {
        stdout: String::new(),
        stderr: "permission denied".to_string(),
        exit_code: None,
    })]);
    let mut session = RuntimeSession::new_with_runner_for_test(AppState::new(), Box::new(runner));
    session
        .apply_effect_result(EffectResult::ConfigurationsLoaded {
            configurations: vec![config("dev", true, "dev-project")],
        })
        .await;

    session
        .dispatch_action(Action::Configurations(
            ConfigurationsAction::StartEditActiveContext,
        ))
        .await;
    let draft = session
        .app()
        .root
        .configurations
        .context_edit_form()
        .expect("context form is open")
        .draft()
        .clone();
    session
        .dispatch_action(Action::Configurations(
            ConfigurationsAction::OpenContextPicker,
        ))
        .await;
    session.process_next_effect_result().await;

    assert!(matches!(
        session
            .app()
            .root
            .configurations
            .context_picker()
            .expect("picker remains open")
            .status(),
        ConfigurationContextPickerStatus::Degraded(message) if message.contains("permission denied")
    ));
    assert_eq!(
        session
            .app()
            .root
            .configurations
            .context_edit_form()
            .expect("context form remains open")
            .draft(),
        &draft
    );
    assert_eq!(session.app().root.system.status_message, None);
}

#[tokio::test]
async fn picker_retry_starts_a_new_discovery_request() {
    let runner = FakeRunner::new(vec![
        Err(CommandError::Failed {
            stdout: String::new(),
            stderr: "temporary failure".to_string(),
            exit_code: None,
        }),
        output(include_str!("fixtures/gcloud_projects.json")),
    ]);
    let runner_handle = runner.clone();
    let mut session = RuntimeSession::new_with_runner_for_test(AppState::new(), Box::new(runner));
    session
        .apply_effect_result(EffectResult::ConfigurationsLoaded {
            configurations: vec![config("dev", true, "dev-project")],
        })
        .await;
    session
        .dispatch_action(Action::Configurations(
            ConfigurationsAction::StartEditActiveContext,
        ))
        .await;
    session
        .dispatch_action(Action::Configurations(
            ConfigurationsAction::OpenContextPicker,
        ))
        .await;
    session.process_next_effect_result().await;
    assert!(matches!(
        session
            .app()
            .root
            .configurations
            .context_picker()
            .expect("picker should remain open")
            .status(),
        ConfigurationContextPickerStatus::Degraded(_)
    ));

    session
        .dispatch_action(Action::Configurations(
            ConfigurationsAction::RetryContextPicker,
        ))
        .await;
    assert_eq!(
        session
            .app()
            .root
            .configurations
            .context_picker()
            .expect("picker should be loading")
            .status(),
        &ConfigurationContextPickerStatus::Loading
    );
    session.process_next_effect_result().await;

    assert_eq!(
        session
            .app()
            .root
            .configurations
            .context_picker()
            .expect("picker should reload")
            .status(),
        &ConfigurationContextPickerStatus::Ready
    );
    assert_eq!(runner_handle.commands().len(), 2);
}

/// Discovery must not block dispatch: the picker has to be renderable as
/// `Loading` while its command is still running. The guarantee is an ordering
/// one, so it is proved by ordering rather than by a clock - the gated runner
/// parks inside `run` until this test releases it, so `is_in_flight()` is a
/// fact about where the command actually is, not an inference from elapsed
/// time. Asserting `Loading` on its own would be weaker: the discovery result
/// is only applied by `process_next_effect_result`, so the picker still reads
/// `Loading` after the command has finished - that assertion alone would pass
/// even if dispatch had blocked on the command.
#[tokio::test]
async fn picker_discovery_returns_loading_while_the_command_is_still_in_flight() {
    let runner = GatedRunner::new();
    let mut session =
        RuntimeSession::new_with_runner_for_test(AppState::new(), Box::new(runner.clone()));
    session
        .apply_effect_result(EffectResult::ConfigurationsLoaded {
            configurations: vec![config("dev", true, "dev-project")],
        })
        .await;
    session
        .dispatch_action(Action::Configurations(
            ConfigurationsAction::StartEditActiveContext,
        ))
        .await;

    session
        .dispatch_action(Action::Configurations(
            ConfigurationsAction::OpenContextPicker,
        ))
        .await;

    // Dispatch returned; wait until the discovery command has provably reached
    // the gate, so "still in flight" below is an observation and not a guess.
    let waiter = runner.clone();
    // Resume the waiter's own panic instead of `unwrap`-ing the `JoinError`:
    // `JoinError`'s `Debug` renders as `JoinError::Panic(Id(n), ..)`, so
    // `unwrap` would make that the primary failure line and bury the
    // `GatedRunner::wait_until_entered timed out ...` message the waiter exists
    // to produce.
    match tokio::task::spawn_blocking(move || waiter.wait_until_entered()).await {
        Ok(()) => {}
        Err(error) => std::panic::resume_unwind(error.into_panic()),
    }

    // The command entered `run` and only this test can release it, so it is
    // still in flight for the whole window below: observing `Loading` here
    // proves dispatch returned before the command finished.
    //
    // The assert is a tripwire that documents the invariant, not the mechanism
    // that enforces it: `entered` is already guaranteed by the
    // `wait_until_entered` above and only this thread sets `released`, so it
    // cannot fire today. The gate's structure is what actually enforces the
    // ordering - if dispatch ever blocks on the command, the dispatch above
    // never returns and the failure surfaces as the named
    // `GatedRunner::run timed out ...` panic at the gate deadline, not here.
    assert!(
        runner.is_in_flight(),
        "the discovery command must still be parked at the gate"
    );
    assert_eq!(
        session
            .app()
            .root
            .configurations
            .context_picker()
            .expect("picker is open")
            .status(),
        &ConfigurationContextPickerStatus::Loading
    );

    session
        .dispatch_action(Action::Configurations(
            ConfigurationsAction::CancelContextPicker,
        ))
        .await;
    assert!(session.app().root.configurations.context_picker().is_none());
    // Let the gated command finish so its result still arrives and the tail
    // assertions below run against a completed effect.
    runner.release();
    assert!(session.process_next_effect_result().await);
    assert!(session.app().root.configurations.context_picker().is_none());
}

/// `GatedRunner::wait_until_entered` runs on a `spawn_blocking` thread, so an
/// unbounded park there survives the test's unwind and wedges `Runtime::drop`.
/// Nothing ever enters `run`, so the waiter must give up and name itself.
#[test]
fn gated_runner_wait_until_entered_gives_up_when_no_command_starts() {
    let gate_timeout = Duration::from_millis(50);
    let runner = GatedRunner::with_gate_timeout(gate_timeout);

    let worker = thread::spawn(move || runner.wait_until_entered());

    let message = panic_message_of_bounded_wait(worker, gate_timeout);
    assert!(
        message.contains("GatedRunner::wait_until_entered timed out"),
        "panic should name the waiter, got: {message}"
    );
}

/// The release gate is the wait that actually wedges `Runtime::drop`: it runs
/// inside the discovery effect's `spawn_blocking`, where an unbounded park is
/// uncancellable. Nothing ever calls `release`, so the runner must give up and
/// say what it was waiting for.
#[test]
fn gated_runner_run_gives_up_when_the_command_is_never_released() {
    let gate_timeout = Duration::from_millis(50);
    let runner = GatedRunner::with_gate_timeout(gate_timeout);

    let worker = thread::spawn(move || {
        let _ = runner.run(GcloudCommand::new(["projects", "list", "--format=json"]));
    });

    let message = panic_message_of_bounded_wait(worker, gate_timeout);
    assert!(
        message.contains("GatedRunner::run timed out"),
        "panic should name the waiter, got: {message}"
    );
}

#[tokio::test]
async fn dropping_session_cancels_picker_discovery() {
    let cancelled = Arc::new(AtomicBool::new(false));
    let runner = CancellationAwareRunner {
        cancelled: cancelled.clone(),
    };
    let mut session = RuntimeSession::new_with_runner_for_test(AppState::new(), Box::new(runner));
    session
        .apply_effect_result(EffectResult::ConfigurationsLoaded {
            configurations: vec![config("dev", true, "dev-project")],
        })
        .await;
    session
        .dispatch_action(Action::Configurations(
            ConfigurationsAction::StartEditActiveContext,
        ))
        .await;
    session
        .dispatch_action(Action::Configurations(
            ConfigurationsAction::OpenContextPicker,
        ))
        .await;

    drop(session);
    // `CancellationAwareRunner` polls its token from a `spawn_blocking` thread,
    // so a busy `yield_now()` spin here competes with it for the same core and
    // can starve the very thread it is waiting on; poll on a short sleep
    // instead. The bound proves cancellation is observed at all, not how fast:
    // `CANCELLATION_TIMEOUT` is orders of magnitude above the runner's own 5ms
    // poll interval, so only a genuinely wedged shutdown can exceed it on a
    // loaded CI runner.
    tokio::time::timeout(CANCELLATION_TIMEOUT, async {
        while !cancelled.load(Ordering::Acquire) {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("picker discovery should observe session shutdown");
}

/// A runner that parks inside `run` until the test releases it, and records
/// that it was entered. It replaces a real sleep: the test can then observe
/// "the command is still running" as a fact about the gate instead of inferring
/// it from a wall-clock budget that a loaded CI runner can blow through.
#[derive(Clone)]
struct GatedRunner {
    state: Arc<GateState>,
}

struct GateState {
    inner: Mutex<GateInner>,
    signal: Condvar,
    gate_timeout: Duration,
}

struct GateInner {
    entered: bool,
    released: bool,
}

impl GatedRunner {
    fn new() -> Self {
        Self::with_gate_timeout(GATE_TIMEOUT)
    }

    /// Same as [`GatedRunner::new`] but with an injectable gate deadline, so the
    /// timeout tests can pin the bound without spending the full `GATE_TIMEOUT`.
    fn with_gate_timeout(gate_timeout: Duration) -> Self {
        Self {
            state: Arc::new(GateState {
                inner: Mutex::new(GateInner {
                    entered: false,
                    released: false,
                }),
                signal: Condvar::new(),
                gate_timeout,
            }),
        }
    }

    /// Blocks until the command has entered `run`, giving up at the gate
    /// deadline rather than parking forever.
    fn wait_until_entered(&self) {
        let deadline = Instant::now() + self.state.gate_timeout;
        let mut guard = self.state.inner.lock().unwrap();
        while !guard.entered {
            // Budget the remaining time against the single deadline so a
            // spurious wakeup cannot restart the full timeout.
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                // Format first, then release the lock: panicking with the guard
                // still held would poison the mutex and replace the gated
                // command's own named diagnostic with a `PoisonError`.
                let message = format!(
                    "GatedRunner::wait_until_entered timed out after {:?}: no command reached the gate",
                    self.state.gate_timeout
                );
                drop(guard);
                panic!("{message}");
            }
            guard = self.state.signal.wait_timeout(guard, remaining).unwrap().0;
        }
    }

    /// Reports precisely one thing: *the first command reached the gate and has
    /// not been released yet*. It is not a general "is some command in flight"
    /// predicate. `entered` latches and never resets, so once
    /// [`GatedRunner::release`] has been called this returns `false` forever --
    /// even if a later command is genuinely parked at the gate. Only valid
    /// before `release`; a test that has not released yet can rely on it
    /// staying true.
    fn is_in_flight(&self) -> bool {
        let guard = self.state.inner.lock().unwrap();
        guard.entered && !guard.released
    }

    /// Permits the gated command to complete.
    fn release(&self) {
        let mut guard = self.state.inner.lock().unwrap();
        guard.released = true;
        self.state.signal.notify_all();
    }
}

impl CommandRunner for GatedRunner {
    fn run(&self, _command: GcloudCommand) -> Result<CommandOutput, CommandError> {
        // This wait runs on the discovery effect's `spawn_blocking` thread,
        // where an unbounded park is uncancellable and would wedge
        // `Runtime::drop` for good if a test panicked before releasing. Bound it
        // and panic loudly: a panicking blocking task surfaces as a `JoinError`,
        // a hang does not.
        let deadline = Instant::now() + self.state.gate_timeout;
        let mut guard = self.state.inner.lock().unwrap();
        guard.entered = true;
        self.state.signal.notify_all();
        while !guard.released {
            // Budget the remaining time against the single deadline so a
            // spurious wakeup cannot restart the full timeout.
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                // Format first, then release the lock: poisoning it here would
                // turn the test thread's own `lock().unwrap()` into a
                // `PoisonError` panic and bury the real diagnostic.
                let message = format!(
                    "GatedRunner::run timed out after {:?} waiting to be released",
                    self.state.gate_timeout
                );
                drop(guard);
                panic!("{message}");
            }
            guard = self.state.signal.wait_timeout(guard, remaining).unwrap().0;
        }
        drop(guard);
        output(include_str!("fixtures/gcloud_projects.json"))
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

#[derive(Clone)]
struct CancellationAwareRunner {
    cancelled: Arc<AtomicBool>,
}

impl CommandRunner for CancellationAwareRunner {
    fn run(&self, _command: GcloudCommand) -> Result<CommandOutput, CommandError> {
        Ok(CommandOutput {
            stdout: String::new(),
            stderr: String::new(),
        })
    }

    fn run_with_cancellation(
        &self,
        _command: GcloudCommand,
        cancellation: &tokio_util::sync::CancellationToken,
    ) -> Result<CommandOutput, CommandError> {
        while !cancellation.is_cancelled() {
            std::thread::sleep(Duration::from_millis(5));
        }
        self.cancelled.store(true, Ordering::Release);
        Err(CommandError::Cancelled {
            ceiling: CommandCeiling::Read,
        })
    }
}

fn instance(name: &str) -> ComputeInstance {
    ComputeInstance {
        id: name.to_string(),
        name: name.to_string(),
        zone: "europe-west1-b".to_string(),
        status: ResourceStatus::Running,
        machine_type: "e2-medium".to_string(),
        internal_ip: None,
        external_ip: None,
    }
}

#[tokio::test]
async fn session_loads_configurations_through_injected_runner() {
    let runner = FakeRunner::new(vec![output(include_str!(
        "fixtures/gcloud_configurations.json"
    ))]);
    let mut session = RuntimeSession::new_with_runner_for_test(AppState::new(), Box::new(runner));

    session
        .dispatch_action(Action::Effect(Effect::LoadConfigurations))
        .await;

    assert_eq!(
        session
            .app()
            .root
            .current_context()
            .unwrap()
            .configuration
            .as_str(),
        "dev"
    );
    assert_eq!(
        session.app().root.system.operation.phase,
        OperationPhase::Healthy
    );
    assert_eq!(session.app().root.system.operation.label, "configurations");
    assert!(session.app().root.system.last_refresh_at.is_some());
}

#[tokio::test]
async fn configuration_reload_failures_record_the_completed_refresh_time() {
    let configuration = config("dev", true, "dev-project");
    let results = [
        EffectResult::ConfigurationActivationReloadFailed {
            configuration: configuration.clone(),
            error: AppError::gcloud("activation reload failed"),
        },
        EffectResult::ConfigurationCreatedReloadFailed {
            configuration: configuration.clone(),
            error: AppError::gcloud("create reload failed"),
        },
        EffectResult::ConfigurationContextUpdateReloadFailed {
            configuration,
            error: AppError::gcloud("context reload failed"),
        },
    ];

    for result in results {
        let mut session = RuntimeSession::new_for_test(AppState::new());
        session.apply_effect_result(result).await;

        assert!(session.app().root.system.last_refresh_at.is_some());
    }
}

#[tokio::test]
async fn startup_activates_requested_configuration_before_initial_load() {
    let runner = FakeRunner::new(vec![
        output(""),
        output(
            r#"[
                {
                    "name": "dev",
                    "is_active": false,
                    "properties": { "core": { "project": "dev-project" } }
                },
                {
                    "name": "prod",
                    "is_active": true,
                    "properties": { "core": { "project": "prod-project" } }
                }
            ]"#,
        ),
    ]);
    let config = nuvem::Config {
        starting_configuration: Some("prod".to_string()),
        ..nuvem::Config::default()
    };
    let mut session = RuntimeSession::new_with_config_and_runner_for_test(
        config,
        AppState::new(),
        Box::new(runner),
    );

    session.initialize().await;

    assert_eq!(
        session
            .app()
            .root
            .current_context()
            .unwrap()
            .configuration
            .as_str(),
        "prod"
    );
    assert_eq!(session.app().root.configurations.rows().len(), 2);
}

#[tokio::test]
async fn startup_reports_error_when_requested_configuration_activation_fails() {
    let runner = FakeRunner::new(vec![Err(CommandError::Failed {
        stdout: String::new(),
        stderr: "configuration not found".to_string(),
        exit_code: None,
    })]);
    let config = nuvem::Config {
        starting_configuration: Some("missing".to_string()),
        ..nuvem::Config::default()
    };
    let mut session = RuntimeSession::new_with_config_and_runner_for_test(
        config,
        AppState::new(),
        Box::new(runner),
    );

    session.initialize().await;

    assert!(session.app().root.current_context().is_none());
    let status = session.app().root.system.status_message.clone();
    assert_eq!(
        status.as_deref(),
        Some("unknown error — Retry, or copy the details to investigate")
    );
    assert!(
        !status
            .as_deref()
            .unwrap_or_default()
            .contains("configuration not found")
    );
}

#[tokio::test]
async fn startup_activation_reload_failure_keeps_requested_context_visible() {
    let runner = FakeRunner::new(vec![
        output(""),
        Err(CommandError::Failed {
            stdout: String::new(),
            stderr: "reload failed".to_string(),
            exit_code: None,
        }),
    ]);
    let config = nuvem::Config {
        starting_configuration: Some("prod".to_string()),
        ..nuvem::Config::default()
    };
    let mut session = RuntimeSession::new_with_config_and_runner_for_test(
        config,
        AppState::new(),
        Box::new(runner),
    );

    session.initialize().await;

    assert_eq!(
        session
            .app()
            .root
            .current_context()
            .unwrap()
            .configuration
            .as_str(),
        "prod"
    );
    assert!(session.app().root.resources.is_stale());
    let status = session.app().root.system.status_message.clone();
    assert_eq!(
        status.as_deref(),
        Some("unknown error — Retry, or copy the details to investigate")
    );
    assert!(
        !status
            .as_deref()
            .unwrap_or_default()
            .contains("reload failed")
    );
}

#[tokio::test]
async fn activation_reload_failure_applies_known_active_configuration() {
    let runner = FakeRunner::new(vec![
        output(""),
        Err(CommandError::Failed {
            stdout: String::new(),
            stderr: "reload failed".to_string(),
            exit_code: None,
        }),
    ]);
    let mut session = RuntimeSession::new_with_runner_for_test(AppState::new(), Box::new(runner));
    session
        .apply_effect_result(EffectResult::ConfigurationsLoaded {
            configurations: vec![
                config("dev", true, "dev-project"),
                config("prod", false, "prod-project"),
            ],
        })
        .await;
    let context = session.app().root.current_context().unwrap().clone();
    session
        .apply_effect_result(EffectResult::ComputeInstancesLoaded {
            context,
            instances: vec![instance("api-dev-1")],
        })
        .await;

    session
        .dispatch_action(Action::Effect(Effect::ActivateConfiguration {
            name: GcloudConfigurationName::new("prod"),
        }))
        .await;

    assert_eq!(
        session
            .app()
            .root
            .current_context()
            .unwrap()
            .configuration
            .as_str(),
        "prod"
    );
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
            .contains("reload failed")
    );
}

#[tokio::test]
async fn activation_reload_rejects_requested_configuration_that_remains_inactive() {
    let runner = FakeRunner::new(vec![
        output(""),
        output(
            r#"[
                {
                    "name": "dev",
                    "is_active": true,
                    "properties": { "core": { "project": "dev-project" } }
                },
                {
                    "name": "prod",
                    "is_active": false,
                    "properties": { "core": { "project": "prod-project" } }
                }
            ]"#,
        ),
    ]);
    let mut session = RuntimeSession::new_with_runner_for_test(AppState::new(), Box::new(runner));
    session
        .apply_effect_result(EffectResult::ConfigurationsLoaded {
            configurations: vec![
                config("dev", true, "dev-project"),
                config("prod", false, "prod-project"),
            ],
        })
        .await;

    session
        .dispatch_action(Action::Effect(Effect::ActivateConfiguration {
            name: GcloudConfigurationName::new("prod"),
        }))
        .await;

    assert_eq!(
        session
            .app()
            .root
            .current_context()
            .expect("fallback context should remain available")
            .configuration
            .as_str(),
        "prod"
    );
    let status = session.app().root.system.status_message.clone();
    assert_eq!(
        status.as_deref(),
        Some("unknown error — Retry, or copy the details to investigate")
    );
    assert!(
        !status
            .as_deref()
            .unwrap_or_default()
            .contains("not uniquely active")
    );
}

#[tokio::test]
async fn activation_reload_rejects_multiple_active_configurations() {
    let runner = FakeRunner::new(vec![
        output(""),
        output(
            r#"[
                {
                    "name": "dev",
                    "is_active": true,
                    "properties": { "core": { "project": "dev-project" } }
                },
                {
                    "name": "prod",
                    "is_active": true,
                    "properties": { "core": { "project": "prod-project" } }
                }
            ]"#,
        ),
    ]);
    let mut session = RuntimeSession::new_with_runner_for_test(AppState::new(), Box::new(runner));
    session
        .apply_effect_result(EffectResult::ConfigurationsLoaded {
            configurations: vec![
                config("dev", true, "dev-project"),
                config("prod", false, "prod-project"),
            ],
        })
        .await;

    session
        .dispatch_action(Action::Effect(Effect::ActivateConfiguration {
            name: GcloudConfigurationName::new("prod"),
        }))
        .await;

    assert_eq!(
        session
            .app()
            .root
            .current_context()
            .expect("fallback context should remain available")
            .configuration
            .as_str(),
        "prod"
    );
    let status = session.app().root.system.status_message.clone();
    assert_eq!(
        status.as_deref(),
        Some("unknown error — Retry, or copy the details to investigate")
    );
    assert!(
        !status
            .as_deref()
            .unwrap_or_default()
            .contains("not uniquely active")
    );
}

#[tokio::test]
async fn create_configuration_applies_reloaded_active_configuration_list() {
    let runner = FakeRunner::new(vec![
        output(""),
        output(
            r#"[
                {
                    "name": "dev",
                    "is_active": false,
                    "properties": { "core": { "project": "dev-project" } }
                },
                {
                    "name": "staging",
                    "is_active": true,
                    "properties": { "core": { "project": "staging-project" } }
                }
            ]"#,
        ),
    ]);
    let mut session = RuntimeSession::new_with_runner_for_test(AppState::new(), Box::new(runner));
    session
        .apply_effect_result(EffectResult::ConfigurationsLoaded {
            configurations: vec![config("dev", true, "dev-project")],
        })
        .await;
    let context = session.app().root.current_context().unwrap().clone();
    session
        .apply_effect_result(EffectResult::ComputeInstancesLoaded {
            context,
            instances: vec![instance("api-dev-1")],
        })
        .await;

    session
        .dispatch_action(Action::Effect(Effect::CreateConfiguration {
            draft: GcloudConfigurationDraft {
                name: "staging".to_string(),
                account: None,
                project: None,
                region: None,
                zone: None,
            },
        }))
        .await;

    assert_eq!(
        session
            .app()
            .root
            .current_context()
            .unwrap()
            .configuration
            .as_str(),
        "staging"
    );
    assert_eq!(session.app().root.configurations.rows().len(), 2);
    assert!(session.app().root.resources.is_stale());
    assert!(session.app().root.resources.instances().is_empty());
}

#[tokio::test]
async fn create_configuration_property_failure_reloads_native_configuration_state() {
    let runner = FakeRunner::new(vec![
        output(""),
        Err(CommandError::Failed {
            stdout: String::new(),
            stderr: "project failed".to_string(),
            exit_code: Some(7),
        }),
        output(
            r#"[
                {
                    "name": "dev",
                    "is_active": false,
                    "properties": { "core": { "project": "dev-project" } }
                },
                {
                    "name": "staging",
                    "is_active": true,
                    "properties": { "core": {} }
                }
            ]"#,
        ),
    ]);
    let mut session = RuntimeSession::new_with_runner_for_test(AppState::new(), Box::new(runner));
    session
        .apply_effect_result(EffectResult::ConfigurationsLoaded {
            configurations: vec![config("dev", true, "dev-project")],
        })
        .await;
    let context = session.app().root.current_context().unwrap().clone();
    session
        .apply_effect_result(EffectResult::ComputeInstancesLoaded {
            context,
            instances: vec![instance("api-dev-1")],
        })
        .await;

    session
        .dispatch_action(Action::Effect(Effect::CreateConfiguration {
            draft: GcloudConfigurationDraft {
                name: "staging".to_string(),
                account: None,
                project: Some("staging-project".to_string()),
                region: None,
                zone: None,
            },
        }))
        .await;

    assert_eq!(
        session
            .app()
            .root
            .current_context()
            .unwrap()
            .configuration
            .as_str(),
        "staging"
    );
    assert_eq!(session.app().root.configurations.rows().len(), 2);
    assert!(session.app().root.resources.is_stale());
    assert!(session.app().root.resources.instances().is_empty());
    let status = session.app().root.system.status_message.clone();
    assert_eq!(
        status.as_deref(),
        Some("unknown error — Retry, or copy the details to investigate")
    );
    // The status is the sanitized headline only — the raw stderr ("project
    // failed") must never leak into the user-facing status message.
    assert!(
        !status
            .as_deref()
            .unwrap_or_default()
            .contains("project failed")
    );
    // Proves the CreatedConfigurationSetupFailed call site routes through the
    // `From<GcloudConfigurationError>` conversion (via `.into()`), which preserves the
    // inner CommandError's exit_code, rather than building an AppError manually
    // (which always drops it). The exit code is preserved in the sanitized
    // diagnostic recorded for the copy key, even though it is not rendered in
    // the status headline.
    assert_eq!(
        session
            .app()
            .root
            .system
            .last_diagnostic()
            .and_then(|diagnostic| diagnostic.exit_code),
        Some(7)
    );
}

#[tokio::test]
async fn update_configuration_context_applies_reloaded_configuration_list() {
    let runner = FakeRunner::new(vec![
        output(""),
        output(""),
        output(""),
        output(
            r#"[
                {
                    "name": "dev",
                    "is_active": true,
                    "properties": {
                        "core": { "project": "new-project" },
                        "compute": {
                            "region": "europe-west1",
                            "zone": "europe-west1-b"
                        }
                    }
                }
            ]"#,
        ),
    ]);
    let mut session = RuntimeSession::new_with_runner_for_test(AppState::new(), Box::new(runner));
    session
        .apply_effect_result(EffectResult::ConfigurationsLoaded {
            configurations: vec![config("dev", true, "old-project")],
        })
        .await;
    let context = session.app().root.current_context().unwrap().clone();
    session
        .apply_effect_result(EffectResult::ComputeInstancesLoaded {
            context,
            instances: vec![instance("api-dev-1")],
        })
        .await;

    session
        .dispatch_action(Action::Effect(Effect::UpdateConfigurationContext {
            draft: GcloudContextEditDraft {
                configuration: GcloudConfigurationName::new("dev"),
                project: Some("new-project".to_string()),
                region: Some("europe-west1".to_string()),
                zone: Some("europe-west1-b".to_string()),
            },
        }))
        .await;

    let context = session.app().root.current_context().unwrap();
    assert_eq!(context.configuration.as_str(), "dev");
    assert_eq!(context.project.as_deref(), Some("new-project"));
    assert_eq!(context.region.as_deref(), Some("europe-west1"));
    assert_eq!(context.zone.as_deref(), Some("europe-west1-b"));
    assert!(session.app().root.resources.is_stale());
    assert!(session.app().root.resources.instances().is_empty());
    assert!(session.app().root.system.status_message.is_none());
}

#[tokio::test]
async fn update_configuration_context_failure_sets_status_without_closing_known_context() {
    let runner = FakeRunner::new(vec![Err(CommandError::Failed {
        stdout: String::new(),
        stderr: "project update failed".to_string(),
        exit_code: None,
    })]);
    let mut session = RuntimeSession::new_with_runner_for_test(AppState::new(), Box::new(runner));
    session
        .apply_effect_result(EffectResult::ConfigurationsLoaded {
            configurations: vec![config("dev", true, "old-project")],
        })
        .await;

    session
        .dispatch_action(Action::Effect(Effect::UpdateConfigurationContext {
            draft: GcloudContextEditDraft {
                configuration: GcloudConfigurationName::new("dev"),
                project: Some("new-project".to_string()),
                region: Some("europe-west1".to_string()),
                zone: Some("europe-west1-b".to_string()),
            },
        }))
        .await;

    let context = session.app().root.current_context().unwrap();
    assert_eq!(context.configuration.as_str(), "dev");
    assert_eq!(context.project.as_deref(), Some("old-project"));
    let status = session.app().root.system.status_message.clone();
    assert_eq!(
        status.as_deref(),
        Some("unknown error — Retry, or copy the details to investigate")
    );
    assert!(
        !status
            .as_deref()
            .unwrap_or_default()
            .contains("project update failed")
    );
}

#[tokio::test]
async fn update_configuration_context_region_failure_reloads_native_configuration_state() {
    let runner = FakeRunner::new(vec![
        output(""),
        Err(CommandError::Failed {
            stdout: String::new(),
            stderr: "region update failed".to_string(),
            exit_code: None,
        }),
        output(
            r#"[
                {
                    "name": "dev",
                    "is_active": true,
                    "properties": {
                        "core": { "project": "new-project" },
                        "compute": {
                            "region": "europe-west1",
                            "zone": "europe-west1-b"
                        }
                    }
                }
            ]"#,
        ),
    ]);
    let runner_handle = runner.clone();
    let mut session = RuntimeSession::new_with_runner_for_test(AppState::new(), Box::new(runner));
    session
        .apply_effect_result(EffectResult::ConfigurationsLoaded {
            configurations: vec![config("dev", true, "old-project")],
        })
        .await;
    let context = session.app().root.current_context().unwrap().clone();
    session
        .apply_effect_result(EffectResult::ComputeInstancesLoaded {
            context,
            instances: vec![instance("api-dev-1")],
        })
        .await;

    session
        .dispatch_action(Action::Effect(Effect::UpdateConfigurationContext {
            draft: GcloudContextEditDraft {
                configuration: GcloudConfigurationName::new("dev"),
                project: Some("new-project".to_string()),
                region: Some("europe-west1".to_string()),
                zone: Some("europe-west1-b".to_string()),
            },
        }))
        .await;

    let context = session.app().root.current_context().unwrap();
    assert_eq!(context.configuration.as_str(), "dev");
    assert_eq!(context.project.as_deref(), Some("new-project"));
    assert_eq!(context.region.as_deref(), Some("europe-west1"));
    assert_eq!(context.zone.as_deref(), Some("europe-west1-b"));
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
            .contains("region update failed")
    );

    let commands = runner_handle.commands();
    assert_eq!(commands.len(), 3);
    assert_eq!(
        commands[0].args,
        vec![
            "--configuration",
            "dev",
            "config",
            "set",
            "project",
            "new-project",
        ]
    );
    assert_eq!(
        commands[1].args,
        vec![
            "--configuration",
            "dev",
            "config",
            "set",
            "compute/region",
            "europe-west1",
        ]
    );
    assert_eq!(
        commands[2].args,
        vec!["config", "configurations", "list", "--format=json",]
    );
    assert_eq!(runner_handle.remaining_output_count(), 0);
}

#[tokio::test]
async fn update_configuration_context_region_failure_reload_failure_uses_applied_context() {
    let runner = FakeRunner::new(vec![
        output(""),
        Err(CommandError::Failed {
            stdout: String::new(),
            stderr: "region update failed".to_string(),
            exit_code: None,
        }),
        Err(CommandError::Failed {
            stdout: String::new(),
            stderr: "reload failed".to_string(),
            exit_code: None,
        }),
    ]);
    let runner_handle = runner.clone();
    let mut session = RuntimeSession::new_with_runner_for_test(AppState::new(), Box::new(runner));
    session
        .apply_effect_result(EffectResult::ConfigurationsLoaded {
            configurations: vec![config_with_context(
                "dev",
                true,
                Some("dev@example.com"),
                Some("old-project"),
                Some("us-central1"),
                Some("us-central1-a"),
            )],
        })
        .await;
    let context = session.app().root.current_context().unwrap().clone();
    session
        .apply_effect_result(EffectResult::ComputeInstancesLoaded {
            context,
            instances: vec![instance("api-dev-1")],
        })
        .await;

    session
        .dispatch_action(Action::Effect(Effect::UpdateConfigurationContext {
            draft: GcloudContextEditDraft {
                configuration: GcloudConfigurationName::new("dev"),
                project: Some("new-project".to_string()),
                region: Some("europe-west1".to_string()),
                zone: Some("europe-west1-b".to_string()),
            },
        }))
        .await;

    let context = session.app().root.current_context().unwrap();
    assert_eq!(context.configuration.as_str(), "dev");
    assert_eq!(context.account.as_deref(), Some("dev@example.com"));
    assert_eq!(context.project.as_deref(), Some("new-project"));
    assert_eq!(context.region.as_deref(), Some("us-central1"));
    assert_eq!(context.zone.as_deref(), Some("us-central1-a"));
    assert!(session.app().root.resources.is_stale());
    assert!(session.app().root.resources.instances().is_empty());
    let status = session.app().root.system.status_message.clone();
    assert_eq!(
        status.as_deref(),
        Some("unknown error — Retry, or copy the details to investigate")
    );
    let status_text = status.unwrap_or_default();
    assert!(!status_text.contains("region update failed"));
    assert!(!status_text.contains("reload failed"));

    let commands = runner_handle.commands();
    assert_eq!(commands.len(), 3);
    assert_eq!(
        commands[2].args,
        vec!["config", "configurations", "list", "--format=json",]
    );
    assert_eq!(runner_handle.remaining_output_count(), 0);
}

#[tokio::test]
async fn update_configuration_context_zone_failure_reload_failure_uses_applied_context() {
    let runner = FakeRunner::new(vec![
        output(""),
        output(""),
        Err(CommandError::Failed {
            stdout: String::new(),
            stderr: "zone update failed".to_string(),
            exit_code: None,
        }),
        Err(CommandError::Failed {
            stdout: String::new(),
            stderr: "reload failed".to_string(),
            exit_code: None,
        }),
    ]);
    let runner_handle = runner.clone();
    let mut session = RuntimeSession::new_with_runner_for_test(AppState::new(), Box::new(runner));
    session
        .apply_effect_result(EffectResult::ConfigurationsLoaded {
            configurations: vec![config_with_context(
                "dev",
                true,
                Some("dev@example.com"),
                Some("old-project"),
                Some("us-central1"),
                Some("us-central1-a"),
            )],
        })
        .await;
    let context = session.app().root.current_context().unwrap().clone();
    session
        .apply_effect_result(EffectResult::ComputeInstancesLoaded {
            context,
            instances: vec![instance("api-dev-1")],
        })
        .await;

    session
        .dispatch_action(Action::Effect(Effect::UpdateConfigurationContext {
            draft: GcloudContextEditDraft {
                configuration: GcloudConfigurationName::new("dev"),
                project: Some("new-project".to_string()),
                region: Some("europe-west1".to_string()),
                zone: Some("europe-west1-b".to_string()),
            },
        }))
        .await;

    let context = session.app().root.current_context().unwrap();
    assert_eq!(context.configuration.as_str(), "dev");
    assert_eq!(context.account.as_deref(), Some("dev@example.com"));
    assert_eq!(context.project.as_deref(), Some("new-project"));
    assert_eq!(context.region.as_deref(), Some("europe-west1"));
    assert_eq!(context.zone.as_deref(), Some("us-central1-a"));
    assert!(session.app().root.resources.is_stale());
    assert!(session.app().root.resources.instances().is_empty());
    let status = session.app().root.system.status_message.clone();
    assert_eq!(
        status.as_deref(),
        Some("unknown error — Retry, or copy the details to investigate")
    );
    let status_text = status.unwrap_or_default();
    assert!(!status_text.contains("zone update failed"));
    assert!(!status_text.contains("reload failed"));

    let commands = runner_handle.commands();
    assert_eq!(commands.len(), 4);
    assert_eq!(
        commands[3].args,
        vec!["config", "configurations", "list", "--format=json",]
    );
    assert_eq!(runner_handle.remaining_output_count(), 0);
}

#[tokio::test]
async fn update_configuration_context_reload_failure_preserves_known_context_with_status() {
    let runner = FakeRunner::new(vec![
        output(""),
        output(""),
        output(""),
        Err(CommandError::Failed {
            stdout: String::new(),
            stderr: "reload failed".to_string(),
            exit_code: None,
        }),
    ]);
    let mut session = RuntimeSession::new_with_runner_for_test(AppState::new(), Box::new(runner));
    session
        .apply_effect_result(EffectResult::ConfigurationsLoaded {
            configurations: vec![config_with_context(
                "dev",
                true,
                Some("dev@example.com"),
                Some("old-project"),
                None,
                None,
            )],
        })
        .await;
    let context = session.app().root.current_context().unwrap().clone();
    session
        .apply_effect_result(EffectResult::ComputeInstancesLoaded {
            context,
            instances: vec![instance("api-dev-1")],
        })
        .await;

    session
        .dispatch_action(Action::Effect(Effect::UpdateConfigurationContext {
            draft: GcloudContextEditDraft {
                configuration: GcloudConfigurationName::new("dev"),
                project: Some("new-project".to_string()),
                region: Some("europe-west1".to_string()),
                zone: Some("europe-west1-b".to_string()),
            },
        }))
        .await;

    let context = session.app().root.current_context().unwrap();
    assert_eq!(context.configuration.as_str(), "dev");
    assert_eq!(context.account.as_deref(), Some("dev@example.com"));
    assert_eq!(context.project.as_deref(), Some("new-project"));
    assert_eq!(context.region.as_deref(), Some("europe-west1"));
    assert_eq!(context.zone.as_deref(), Some("europe-west1-b"));
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
            .contains("reload failed")
    );
}
