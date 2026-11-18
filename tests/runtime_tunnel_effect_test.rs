//! Runtime wiring for managed tunnels (Task 5): the session launches a
//! child, supervises its output, and stops it again.
//!
//! Every wait in this file is bounded. The supervisor pumps tunnel output from
//! a `spawn_blocking` thread, where an unbounded park is uncancellable and
//! wedges `Runtime::drop` for good when a test panics before releasing a gate.
//! So each gate carries a deadline and panics with a message naming
//! the waiter and what it was stuck on, and every "did this happen yet?" poll
//! has a ceiling.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use nuvem::app::reducer::root::reduce;
use nuvem::app::state::tunnels::{TunnelEntry, TunnelStatus};
use nuvem::app::{Action, AppState, Effect, EffectResult, TunnelLaunchPlan, TunnelsAction};
use nuvem::services::runtime::session::RuntimeSession;
use nuvem::services::runtime::tunnel::port::PortProbe;
use nuvem::services::runtime::tunnel::{
    LaunchedTunnel, TunnelControl, TunnelId, TunnelLaunchError, TunnelLauncher, TunnelOutput,
    TunnelShutdown,
};
use nuvem_domain::gcp::configuration::{
    GcloudConfiguration, GcloudConfigurationName, GcloudContext,
};
use nuvem_domain::gcp::tunnel::{TunnelKind, TunnelSpec};

/// Deadline for every gate wait and every "has it happened yet?" poll here.
/// The scenarios release their gates within milliseconds, so 30s sits orders of
/// magnitude above any plausible scheduling delay on a loaded CI runner:
/// exceeding it proves something wedged rather than that the machine was slow.
const GATE_TIMEOUT: Duration = Duration::from_secs(30);

/// Readiness deadline injected into the timeout scenario. It bounds no test
/// work: the scripted lines are delivered before the supervisor is ever polled
/// (see the emission latch), so nothing has to fit inside it — it only decides
/// how long that scenario costs.
const SHORT_READINESS_TIMEOUT: Duration = Duration::from_millis(150);

/// One scheduler tick: long enough to make the runtime park and poll the
/// supervisor, and deliberately not a budget for anything. The scenario that
/// uses it injects a zero readiness deadline, so an armed deadline is strictly
/// earlier than this wait and fires first whatever value this holds.
const SUPERVISOR_TICK: Duration = Duration::from_millis(1);

/// What the scripted failing shutdown reports, so the assertion can prove the
/// shutdown's own reason reached the row rather than a generic message.
const SHUTDOWN_FAILURE: &str = "could not signal the tunnel process group";

/// What the scripted panicking shutdown unwinds with. Deliberately recognisable
/// in the stderr the panicking blocking task prints during that test run: the
/// noise is expected, and this names where it came from.
const PANICKING_SHUTDOWN: &str = "a TunnelControl::shutdown that panics";

// --- doubles -----------------------------------------------------------

/// A control that records its shutdowns and releases the output gate, the way a
/// real child's dying pipes end the pump. Without that release a supervisor
/// that shut the child down would still leave its pump thread parked.
struct RecordingControl {
    calls: AtomicUsize,
    /// Shutdowns that ran to completion, as opposed to merely started. The
    /// stalling variant below is in flight for as long as these two differ,
    /// which is what lets a test say "the kill had not finished yet".
    completions: AtomicUsize,
    graces: Mutex<Vec<Duration>>,
    gate: Option<Arc<OutputGate>>,
    /// Parks the *first* shutdown until a second caller reaches the same child,
    /// modelling a slow terminate-poll-kill sequence. The release is driven by
    /// that second caller rather than by elapsed time, so a scenario about
    /// "something else reached the child mid-kill" cannot pass by accident.
    stall: Option<Arc<OutputGate>>,
    /// What every shutdown reports. A failing one deliberately leaves the gate
    /// closed: a shutdown that failed means the child is still there with its
    /// pipes open.
    outcome: TunnelShutdown,
    /// Unwinds instead of returning an outcome, standing in for a future
    /// `TunnelControl` that panics somewhere in its terminate-poll-kill
    /// sequence. Like the failing variant it leaves the gate closed: a shutdown
    /// that never finished has not closed the child's pipes.
    panics: bool,
}

impl RecordingControl {
    /// The well-behaved control every named constructor below starts from, each
    /// mutating the single field it is named for. A new behavioural dimension
    /// is then a field and a default here, not an edit to every constructor
    /// that has nothing to do with it.
    fn base(gate: Option<Arc<OutputGate>>) -> Self {
        Self {
            calls: AtomicUsize::new(0),
            completions: AtomicUsize::new(0),
            graces: Mutex::new(Vec::new()),
            gate,
            stall: None,
            outcome: TunnelShutdown::Terminated,
            panics: false,
        }
    }

    fn new(gate: Option<Arc<OutputGate>>) -> Arc<Self> {
        Arc::new(Self::base(gate))
    }

    fn failing(gate: Option<Arc<OutputGate>>) -> Arc<Self> {
        let mut control = Self::base(gate);
        control.outcome = TunnelShutdown::Failed {
            message: SHUTDOWN_FAILURE.to_string(),
        };
        Arc::new(control)
    }

    /// A control that unwinds inside `shutdown`, which is what makes the
    /// blocking stop task's join fail.
    fn panicking(gate: Option<Arc<OutputGate>>) -> Arc<Self> {
        let mut control = Self::base(gate);
        control.panics = true;
        Arc::new(control)
    }

    fn stalling(gate: Option<Arc<OutputGate>>, stall: Arc<OutputGate>) -> Arc<Self> {
        let mut control = Self::base(gate);
        control.stall = Some(stall);
        Arc::new(control)
    }

    fn shutdowns(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }

    fn completed_shutdowns(&self) -> usize {
        self.completions.load(Ordering::SeqCst)
    }

    fn graces(&self) -> Vec<Duration> {
        self.graces
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }
}

impl TunnelControl for RecordingControl {
    fn shutdown(&self, grace: Duration) -> TunnelShutdown {
        let call = self.calls.fetch_add(1, Ordering::SeqCst) + 1;
        self.graces
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(grace);
        if self.panics && call == 1 {
            // The call is recorded first, and the `graces` guard above is
            // already dropped, so this unwind neither hides the call from the
            // test nor poisons a mutex the test still reads. Only the first
            // call panics: the later ones come from the quit and `Drop`
            // backstops, and a destructor that unwinds while the test thread is
            // already panicking aborts the process instead of failing a test.
            panic!("{PANICKING_SHUTDOWN}");
        }
        if matches!(self.outcome, TunnelShutdown::Failed { .. }) {
            return self.outcome.clone();
        }
        if let Some(stall) = &self.stall {
            if call == 1 {
                stall.wait();
            } else {
                // The second caller is the one that frees the first: a real
                // control serializes them on its child lock and hands this one
                // an already-reaped child.
                stall.release();
                self.completions.fetch_add(1, Ordering::SeqCst);
                return TunnelShutdown::AlreadyExited;
            }
        }
        if let Some(gate) = &self.gate {
            gate.release();
        }
        self.completions.fetch_add(1, Ordering::SeqCst);
        self.outcome.clone()
    }
}

/// A bounded park for a tunnel output that must stay open. Never an unbounded
/// `Condvar::wait`: this runs on a `spawn_blocking` thread.
struct OutputGate {
    released: Mutex<bool>,
    signal: Condvar,
    timeout: Duration,
    /// What this gate is waiting for, named in the timeout panic: there is more
    /// than one gate in flight per scenario, so a wedged wait has to say which.
    what: &'static str,
}

impl OutputGate {
    fn new(what: &'static str, timeout: Duration) -> Arc<Self> {
        Arc::new(Self {
            released: Mutex::new(false),
            signal: Condvar::new(),
            timeout,
            what,
        })
    }

    /// Blocks until released, giving up at the deadline with a message naming
    /// this waiter instead of parking forever.
    fn wait(&self) {
        let deadline = Instant::now() + self.timeout;
        let mut released = self.released.lock().unwrap();
        while !*released {
            // Budget against a single deadline so a spurious wakeup cannot
            // restart the full timeout.
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                // Format first, then drop the guard: panicking while holding it
                // poisons the mutex and replaces any sibling waiter's own named
                // diagnostic with a bare `PoisonError`.
                let message = format!(
                    "OutputGate::wait timed out after {:?}: {}",
                    self.timeout, self.what
                );
                drop(released);
                panic!("{message}");
            }
            released = self.signal.wait_timeout(released, remaining).unwrap().0;
        }
    }

    fn release(&self) {
        let mut released = self.released.lock().unwrap();
        *released = true;
        self.signal.notify_all();
    }
}

/// Output that yields its scripted lines and then either reports EOF or parks
/// on the gate, depending on whether the scenario needs a tunnel that stays up.
struct ScriptedOutput {
    lines: VecDeque<String>,
    gate: Option<Arc<OutputGate>>,
    /// Released the moment the script has been handed over in full, so a test
    /// can order itself after the child's output instead of timing it.
    emitted: Option<Arc<OutputGate>>,
}

impl TunnelOutput for ScriptedOutput {
    fn next_line(&mut self) -> Option<String> {
        if let Some(line) = self.lines.pop_front() {
            return Some(line);
        }
        if let Some(emitted) = &self.emitted {
            emitted.release();
        }
        if let Some(gate) = &self.gate {
            gate.wait();
        }
        None
    }
}

/// A launcher for exactly one launch: it hands out the prepared output and a
/// recording control, or fails with the scripted launch error. A second launch
/// is a test bug, so it panics rather than quietly reusing the script.
struct FakeLauncher {
    output: Mutex<Option<ScriptedOutput>>,
    error: Option<TunnelLaunchError>,
    control: Arc<RecordingControl>,
    specs: Mutex<Vec<TunnelSpec>>,
}

impl FakeLauncher {
    /// Yields `lines`, then EOF.
    fn yielding(lines: &[&str]) -> Arc<Self> {
        Arc::new(Self {
            output: Mutex::new(Some(ScriptedOutput {
                lines: lines.iter().map(|line| line.to_string()).collect(),
                gate: None,
                emitted: None,
            })),
            error: None,
            control: RecordingControl::new(None),
            specs: Mutex::new(Vec::new()),
        })
    }

    /// Yields `lines`, then stays open until the returned gate is released —
    /// which the control does on shutdown, like a real child's pipes closing.
    fn gated(lines: &[&str]) -> (Arc<Self>, Arc<OutputGate>) {
        let (launcher, gate, _emitted) = Self::gated_after_emitting(lines);
        (launcher, gate)
    }

    /// [`FakeLauncher::gated`], plus a latch the pump releases once the whole
    /// script has been handed over and it is parked on the output gate. A test
    /// that must run after the child's output waits on that latch rather than
    /// on elapsed time.
    fn gated_after_emitting(lines: &[&str]) -> (Arc<Self>, Arc<OutputGate>, Arc<OutputGate>) {
        let gate = OutputGate::new("the tunnel output was never released", GATE_TIMEOUT);
        let emitted = OutputGate::new("the scripted output was never emitted", GATE_TIMEOUT);
        let launcher = Arc::new(Self {
            output: Mutex::new(Some(ScriptedOutput {
                lines: lines.iter().map(|line| line.to_string()).collect(),
                gate: Some(Arc::clone(&gate)),
                emitted: Some(Arc::clone(&emitted)),
            })),
            error: None,
            control: RecordingControl::new(Some(Arc::clone(&gate))),
            specs: Mutex::new(Vec::new()),
        });
        (launcher, gate, emitted)
    }

    /// [`FakeLauncher::gated`] whose control reports a failed shutdown, the way
    /// a child that could not be signalled or reaped within the grace does.
    fn gated_with_a_failing_shutdown(lines: &[&str]) -> (Arc<Self>, Arc<OutputGate>) {
        let gate = OutputGate::new("the tunnel output was never released", GATE_TIMEOUT);
        let launcher = Arc::new(Self {
            output: Mutex::new(Some(ScriptedOutput {
                lines: lines.iter().map(|line| line.to_string()).collect(),
                gate: Some(Arc::clone(&gate)),
                emitted: None,
            })),
            error: None,
            control: RecordingControl::failing(Some(Arc::clone(&gate))),
            specs: Mutex::new(Vec::new()),
        });
        (launcher, gate)
    }

    /// [`FakeLauncher::gated`] whose control panics inside `shutdown`, standing
    /// in for a `TunnelControl` implementation that grows a panic path later.
    fn gated_with_a_panicking_shutdown(lines: &[&str]) -> (Arc<Self>, Arc<OutputGate>) {
        let gate = OutputGate::new("the tunnel output was never released", GATE_TIMEOUT);
        let launcher = Arc::new(Self {
            output: Mutex::new(Some(ScriptedOutput {
                lines: lines.iter().map(|line| line.to_string()).collect(),
                gate: Some(Arc::clone(&gate)),
                emitted: None,
            })),
            error: None,
            control: RecordingControl::panicking(Some(Arc::clone(&gate))),
            specs: Mutex::new(Vec::new()),
        });
        (launcher, gate)
    }

    /// [`FakeLauncher::gated`] whose control parks its first shutdown, so a
    /// test can hold a stop in flight and act while it is still running. The
    /// second returned gate is that stall.
    fn gated_with_a_stalling_shutdown(
        lines: &[&str],
    ) -> (Arc<Self>, Arc<OutputGate>, Arc<OutputGate>) {
        let gate = OutputGate::new("the tunnel output was never released", GATE_TIMEOUT);
        let stall = OutputGate::new("the in-flight kill was never released", GATE_TIMEOUT);
        let launcher = Arc::new(Self {
            output: Mutex::new(Some(ScriptedOutput {
                lines: lines.iter().map(|line| line.to_string()).collect(),
                gate: Some(Arc::clone(&gate)),
                emitted: None,
            })),
            error: None,
            control: RecordingControl::stalling(Some(Arc::clone(&gate)), Arc::clone(&stall)),
            specs: Mutex::new(Vec::new()),
        });
        (launcher, gate, stall)
    }

    fn failing(error: TunnelLaunchError) -> Arc<Self> {
        Arc::new(Self {
            output: Mutex::new(None),
            error: Some(error),
            control: RecordingControl::new(None),
            specs: Mutex::new(Vec::new()),
        })
    }

    fn specs(&self) -> Vec<TunnelSpec> {
        self.specs
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }
}

impl TunnelLauncher for FakeLauncher {
    fn launch(&self, spec: &TunnelSpec) -> Result<LaunchedTunnel, TunnelLaunchError> {
        self.specs
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(spec.clone());
        if let Some(error) = &self.error {
            return Err(error.clone());
        }
        let output = self
            .output
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take()
            .expect("FakeLauncher scripts exactly one launch");
        Ok(LaunchedTunnel {
            output: Box::new(output),
            control: Arc::clone(&self.control) as Arc<dyn TunnelControl>,
        })
    }
}

/// Port probe over an explicit busy set plus a scripted ephemeral queue, so
/// allocation outcomes are chosen rather than depending on the machine.
struct FakeProbe {
    busy: Vec<u16>,
    ephemeral: Mutex<VecDeque<Option<u16>>>,
}

impl FakeProbe {
    fn everything_free() -> Self {
        Self {
            busy: Vec::new(),
            ephemeral: Mutex::new(VecDeque::new()),
        }
    }

    fn nothing_available() -> Self {
        Self {
            // The whole walk-up window plus both ephemeral attempts fail, which
            // is the only way `allocate_local_port` gives up.
            busy: (0..=u16::MAX).collect(),
            ephemeral: Mutex::new(VecDeque::from(vec![None, None])),
        }
    }

    fn busy(ports: &[u16]) -> Self {
        Self {
            busy: ports.to_vec(),
            ephemeral: Mutex::new(VecDeque::new()),
        }
    }

    /// Hands out `ports` as the OS-assigned ephemeral ports, in order — the
    /// path a tunnel with no preferred port takes.
    fn handing_out_ephemeral(ports: &[u16]) -> Self {
        Self {
            busy: Vec::new(),
            ephemeral: Mutex::new(ports.iter().copied().map(Some).collect()),
        }
    }
}

impl PortProbe for FakeProbe {
    fn is_free(&self, port: u16) -> bool {
        !self.busy.contains(&port)
    }

    fn ephemeral(&self) -> Option<u16> {
        self.ephemeral
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .pop_front()
            .flatten()
    }
}

// --- fixtures ----------------------------------------------------------

fn configuration() -> GcloudConfiguration {
    GcloudConfiguration {
        name: GcloudConfigurationName::new("dev"),
        is_active: true,
        account: None,
        project: Some("dev-project".to_string()),
        region: None,
        zone: None,
    }
}

/// An app with an active context and one `Starting` tunnel holding `local_port`,
/// exactly as the reducer leaves it when it emits `Effect::StartTunnel`. `0` is
/// what it really inserts for a kind with no preferred port.
fn app_with_starting_tunnel_holding(
    kind: TunnelKind,
    local_port: u16,
) -> (AppState, TunnelId, GcloudContext) {
    let mut app = AppState::new();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ConfigurationsLoaded {
            configurations: vec![configuration()],
        }),
    );
    let context = app.root.current_context().expect("an active context");
    let id = app.root.tunnels.mint_id();
    app.root.tunnels.insert_starting(TunnelEntry {
        id,
        kind,
        target_label: "web-1".to_string(),
        local_port,
        remote_label: "22".to_string(),
        status: TunnelStatus::Starting,
        started_at: Instant::now(),
        context: context.clone(),
        message: None,
    });
    (app, id, context)
}

fn plan(kind: TunnelKind) -> TunnelLaunchPlan {
    TunnelLaunchPlan {
        kind,
        target_label: "web-1".to_string(),
        remote_label: "22".to_string(),
        instance_name: "web-1".to_string(),
        zone: "us-central1-a".to_string(),
        connection_name: "dev-project:us-central1:db-1".to_string(),
        preferred_local_port: Some(8022),
        remote_port: 22,
        claimed_ports: Vec::new(),
    }
}

fn status(session: &RuntimeSession, id: TunnelId) -> TunnelStatus {
    session
        .app()
        .root
        .tunnels
        .entries()
        .iter()
        .find(|entry| entry.id == id)
        .map(|entry| entry.status.clone())
        .expect("the tunnel is still in the list")
}

fn local_port(session: &RuntimeSession, id: TunnelId) -> u16 {
    session
        .app()
        .root
        .tunnels
        .entries()
        .iter()
        .find(|entry| entry.id == id)
        .map(|entry| entry.local_port)
        .expect("the tunnel is still in the list")
}

fn message(session: &RuntimeSession, id: TunnelId) -> Option<String> {
    session
        .app()
        .root
        .tunnels
        .entries()
        .iter()
        .find(|entry| entry.id == id)
        .map(|entry| entry.message.clone())
        .expect("the tunnel is still in the list")
}

fn failure_message(session: &RuntimeSession, id: TunnelId) -> String {
    match status(session, id) {
        TunnelStatus::Failed { message } => message,
        other => panic!("expected a failed tunnel, got {other:?}"),
    }
}

/// Starts a tunnel through the real effect path and drains the port-allocation
/// result, which the runtime queues before it launches anything. Every scenario
/// that gets a port sees it first, so draining it here leaves each test looking
/// at the result it is actually about.
async fn start(
    kind: TunnelKind,
    launcher: Arc<FakeLauncher>,
    probe: FakeProbe,
    readiness_timeout: Duration,
) -> (RuntimeSession, TunnelId) {
    // The row starts on `0` — what `confirm_tunnel_launch` really inserts
    // before anything is allocated — so the drain below can be checked rather
    // than assumed.
    let (mut session, id) =
        start_with_plan(plan(kind), 0, launcher, probe, readiness_timeout).await;
    assert!(
        session.process_next_effect_result().await,
        "the allocated port is reported before the child is launched"
    );
    // What the allocation, and only the allocation, does: it moves the port off
    // the placeholder and leaves the status alone. An open channel proves
    // nothing about which result came out of it, and a bare "the port moved"
    // would also be satisfied by a readiness result arriving first.
    assert_ne!(
        local_port(&session, id),
        0,
        "the drained result must have been the port allocation"
    );
    assert_eq!(
        status(&session, id),
        TunnelStatus::Starting,
        "the port allocation must not move the status: knowing the port is not being up"
    );
    (session, id)
}

/// [`start`] without that drain, and with the launch plan and the row's initial
/// port chosen by the caller — for the kinds that have no preferred port to
/// start from, and for the scenario where no port can be allocated at all.
async fn start_with_plan(
    plan: TunnelLaunchPlan,
    row_port: u16,
    launcher: Arc<FakeLauncher>,
    probe: FakeProbe,
    readiness_timeout: Duration,
) -> (RuntimeSession, TunnelId) {
    let kind = plan.kind;
    let (app, id, context) = app_with_starting_tunnel_holding(kind, row_port);
    let mut session = RuntimeSession::new_with_tunnel_runtime_for_test(
        app,
        Box::new(LauncherHandle(launcher)),
        Box::new(probe),
        readiness_timeout,
    );
    session
        .dispatch_action(Action::Effect(Effect::StartTunnel { id, context, plan }))
        .await;
    (session, id)
}

/// Lets the test keep its own handle on the launcher while the session owns
/// one: the session takes a `Box<dyn TunnelLauncher>`, so the shared `Arc` is
/// wrapped rather than moved.
struct LauncherHandle(Arc<FakeLauncher>);

impl TunnelLauncher for LauncherHandle {
    fn launch(&self, spec: &TunnelSpec) -> Result<LaunchedTunnel, TunnelLaunchError> {
        self.0.launch(spec)
    }
}

/// Waits for the supervisor's shutdown, which is sent *after* the failure
/// result and therefore may not have landed when the result is applied.
/// Bounded: a shutdown that never happens fails the test instead of hanging it.
fn wait_for_shutdowns(control: &RecordingControl, expected: usize, what: &str) {
    let deadline = Instant::now() + GATE_TIMEOUT;
    while control.shutdowns() < expected {
        if Instant::now() >= deadline {
            panic!(
                "wait_for_shutdowns timed out after {GATE_TIMEOUT:?}: expected {expected} \
                 shutdowns after {what}, observed {}",
                control.shutdowns()
            );
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// Waits for shutdowns that have run to completion, not merely started — the
/// distinction the stalling control exists to expose. Bounded, like every other
/// wait here.
fn wait_for_completed_shutdowns(control: &RecordingControl, expected: usize, what: &str) {
    let deadline = Instant::now() + GATE_TIMEOUT;
    while control.completed_shutdowns() < expected {
        if Instant::now() >= deadline {
            panic!(
                "wait_for_completed_shutdowns timed out after {GATE_TIMEOUT:?}: expected \
                 {expected} completed shutdowns after {what}, observed {}",
                control.completed_shutdowns()
            );
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

// --- the gate's own bound ----------------------------------------------

/// The output gate runs on the supervisor's `spawn_blocking` thread, where an
/// unbounded park survives the test's unwind and wedges `Runtime::drop`. This
/// pins that it gives up and names what it was stuck on instead.
#[test]
fn the_output_gate_gives_up_instead_of_parking_forever() {
    let gate_timeout = Duration::from_millis(50);
    let gate = OutputGate::new("the tunnel output was never released", gate_timeout);
    let waiter = Arc::clone(&gate);

    let worker = std::thread::spawn(move || waiter.wait());

    // 200x the injected bound: generous enough to absorb scheduling jitter on a
    // loaded runner, and the loop exits the moment the waiter finishes, so the
    // ceiling only costs anything when the wait really is unbounded.
    let ceiling = gate_timeout * 200;
    let started = Instant::now();
    while !worker.is_finished() && started.elapsed() < ceiling {
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(
        worker.is_finished(),
        "a bounded gate wait must give up instead of parking forever"
    );
    let payload = worker.join().expect_err("a timed-out gate wait must panic");
    let message = payload
        .downcast::<String>()
        .expect("a gate timeout panic should carry a formatted message");
    assert!(
        message.contains("OutputGate::wait timed out"),
        "the panic must name the waiter, got: {message}"
    );
}

// --- launch and readiness ----------------------------------------------

#[tokio::test]
async fn a_readiness_banner_makes_the_tunnel_active_on_the_port_the_child_reported() {
    let launcher = FakeLauncher::yielding(&["Listening on port [8022]."]);
    let (mut session, id) = start(
        TunnelKind::IapTcp,
        Arc::clone(&launcher),
        FakeProbe::everything_free(),
        Duration::from_secs(20),
    )
    .await;

    assert!(session.process_next_effect_result().await);

    assert_eq!(status(&session, id), TunnelStatus::Active);
    assert_eq!(session.app().root.tunnels.entries()[0].local_port, 8022);
}

#[tokio::test]
async fn an_on_spawn_tunnel_is_active_without_any_output() {
    // `ssh -N` prints nothing on success, so a healthy spawn is the only signal
    // there will ever be — and the readiness deadline is never armed for it.
    let (launcher, gate) = FakeLauncher::gated(&[]);
    let (mut session, id) = start(
        TunnelKind::SshLocalForward,
        Arc::clone(&launcher),
        FakeProbe::everything_free(),
        // Zero, which makes the check below an ordering statement rather than a
        // timing one: a deadline armed for an `OnSpawn` kind has already expired
        // the first time the supervisor is polled, so its `TunnelFailed` is
        // queued before the child's output is even released.
        Duration::ZERO,
    )
    .await;

    // The `TunnelReady` queued synchronously at launch, before the supervisor
    // even exists. Draining it shows the row goes `Active` with no output at
    // all — but on its own it says nothing about the deadline, which is why the
    // test cannot stop here.
    assert!(session.process_next_effect_result().await);
    assert_eq!(status(&session, id), TunnelStatus::Active);

    // Hands the runtime back so the supervisor is polled while its output is
    // still parked on the gate — the one moment at which a wrongly armed
    // deadline has nothing to compete with and queues its `TunnelFailed`. The
    // ordering comes from the two deadlines (zero, then this one), not from
    // this wait being generous: any positive value does the same job, so it is
    // one scheduler tick rather than a budget for work.
    tokio::time::sleep(SUPERVISOR_TICK).await;

    // Releasing the output ends the pump, so the supervisor's EOF branch queues
    // exactly one more result — and this drain lands on whichever result the
    // supervisor queued *first*. Correctly implemented that is the exit. With
    // the deadline wrongly armed it is the expired deadline's `TunnelFailed`,
    // and the supervisor has stopped the child on its own besides.
    gate.release();
    assert!(session.process_next_effect_result().await);
    assert_eq!(
        status(&session, id),
        TunnelStatus::Stopped,
        "an `OnSpawn` tunnel may only end by its child exiting, never on a readiness deadline"
    );
    assert_eq!(
        launcher.control.shutdowns(),
        0,
        "nothing may shut a healthy `OnSpawn` child down: its deadline is never armed"
    );
}

#[tokio::test]
async fn the_allocated_port_is_the_one_written_into_the_child_argv() {
    // The preferred port is busy, so allocation walks up — and the argv must
    // carry the port that was actually allocated, not the one that was asked
    // for, or the child would bind a port nothing points at.
    let (launcher, gate) = FakeLauncher::gated(&[]);
    let (session, _id) = start(
        TunnelKind::IapTcp,
        Arc::clone(&launcher),
        FakeProbe::busy(&[8022]),
        Duration::from_secs(20),
    )
    .await;

    let specs = launcher.specs();
    assert_eq!(specs.len(), 1, "exactly one child is launched");
    assert!(
        specs[0]
            .args
            .contains(&"--local-host-port=localhost:8023".to_string()),
        "expected the walked-up port in the argv, got {:?}",
        specs[0].args
    );
    drop(session);
    gate.release();
}

/// `claimed_ports` is the app's only defence against handing the same loopback
/// port to two tunnels started seconds apart — the OS cannot help, because a
/// child that has not `bind()`ed yet leaves its port looking free. IAP and
/// `ssh -L` have no preferred port, so the row holds `0` until something writes
/// the allocation back; until it does, `claimed_ports` protects nothing.
#[tokio::test]
async fn the_allocated_port_is_claimed_in_state_before_the_child_reports_ready() {
    // Nothing is ever emitted, so the only thing that can have moved the row's
    // port is the allocation itself.
    let (launcher, gate) = FakeLauncher::gated(&[]);
    let mut plan = plan(TunnelKind::IapTcp);
    plan.preferred_local_port = None;
    let (mut session, id) = start_with_plan(
        plan,
        // What `confirm_tunnel_launch` really inserts for a kind with no
        // preferred port.
        0,
        Arc::clone(&launcher),
        FakeProbe::handing_out_ephemeral(&[8100]),
        Duration::from_secs(20),
    )
    .await;

    assert!(session.process_next_effect_result().await);

    assert_eq!(
        local_port(&session, id),
        8100,
        "the port the allocator chose must be in the row before the child says anything"
    );
    assert_eq!(
        status(&session, id),
        TunnelStatus::Starting,
        "knowing the port is not the same as being up"
    );
    assert_eq!(
        session.app().root.tunnels.claimed_ports(),
        vec![8100],
        "a tunnel started while this one is still coming up must see the port promised"
    );
    gate.release();
}

// --- failure paths -----------------------------------------------------

#[tokio::test]
async fn an_error_line_fails_the_tunnel_and_shuts_the_child_down() {
    let launcher = FakeLauncher::yielding(&["ERROR: denied by IAP\u{7}"]);
    let (mut session, id) = start(
        TunnelKind::IapTcp,
        Arc::clone(&launcher),
        FakeProbe::everything_free(),
        Duration::from_secs(20),
    )
    .await;

    assert!(session.process_next_effect_result().await);

    assert_eq!(
        failure_message(&session, id),
        "ERROR: denied by IAP\u{fffd}",
        "the child's line reaches state sanitized"
    );
    wait_for_shutdowns(&launcher.control, 1, "a failing output line");
}

/// The liberal classifier is what stops a banner drift from hanging a tunnel in
/// `Starting` forever, so before readiness an error line must still be fatal —
/// and fatal while the child is still alive, not merely because it exited.
#[tokio::test]
async fn an_error_line_before_readiness_fails_the_tunnel_and_shuts_the_live_child_down() {
    let (launcher, gate) = FakeLauncher::gated(&["ERROR: denied by IAP"]);
    let (mut session, id) = start(
        TunnelKind::IapTcp,
        Arc::clone(&launcher),
        FakeProbe::everything_free(),
        Duration::from_secs(20),
    )
    .await;

    assert!(session.process_next_effect_result().await);

    assert_eq!(
        failure_message(&session, id),
        "ERROR: denied by IAP",
        "a tunnel that fails before it is ready keeps the child's own reason"
    );
    wait_for_shutdowns(
        &launcher.control,
        1,
        "a failing output line before readiness",
    );
    gate.release();
}

/// The other half of that policy: once a tunnel is serving traffic, a line the
/// classifier calls an error is ordinary chatter from a working child (`ssh`
/// reporting one refused forwarded connection, `gcloud`'s own
/// `...failed.` spinner text) and must never tear the tunnel down.
#[tokio::test]
async fn an_error_line_after_readiness_leaves_the_tunnel_up_and_shuts_nothing_down() {
    // `ssh -L` is `OnSpawn`, so it is already `Active` when the line arrives —
    // the exact shape of the reported failure.
    let launcher =
        FakeLauncher::yielding(&["channel 3: open failed: connect failed: Connection refused"]);
    let (mut session, id) = start(
        TunnelKind::SshLocalForward,
        Arc::clone(&launcher),
        FakeProbe::everything_free(),
        Duration::from_secs(20),
    )
    .await;

    // The `TunnelReady` queued synchronously at launch.
    assert!(session.process_next_effect_result().await);
    assert_eq!(status(&session, id), TunnelStatus::Active);

    // The supervisor sees the error line and then EOF. The line is surfaced as
    // a notice rather than acted on, so the first further result leaves the row
    // exactly where it was — a `TunnelFailed` here would mean it was (wrongly)
    // treated as fatal.
    assert!(session.process_next_effect_result().await);
    assert_eq!(
        status(&session, id),
        TunnelStatus::Active,
        "a benign stderr line on a ready tunnel must not fail it"
    );
    assert_eq!(
        message(&session, id).as_deref(),
        Some("channel 3: open failed: connect failed: Connection refused"),
        "the line must reach the row's MESSAGE, or a healthy-then-broken tunnel \
         ends with no diagnostics at all"
    );

    // Then the child's own EOF, which is the only thing that ended this one.
    assert!(session.process_next_effect_result().await);
    assert_eq!(
        status(&session, id),
        TunnelStatus::Stopped,
        "only the child exiting ended this tunnel"
    );
    assert_eq!(
        launcher.control.shutdowns(),
        0,
        "nothing may tear down a working forward because of one refused connection"
    );
}

/// Tier one of the same policy: a line that means the tunnel cannot carry
/// traffic is fatal whenever it arrives. `ssh -L` is `OnSpawn`, so it is ready
/// from the instant it spawns and this is the *only* tier that can ever fail
/// it — without it a forward that could not bind sits `Active` forever.
#[tokio::test]
async fn an_always_fatal_line_after_readiness_fails_the_tunnel_and_shuts_the_child_down() {
    // Gated, so the child is still alive when the line lands: the tunnel has to
    // be failed *and* torn down, not merely observed to have exited.
    let (launcher, gate) = FakeLauncher::gated(&["bind [127.0.0.1]:5432: Address already in use"]);
    let (mut session, id) = start(
        TunnelKind::SshLocalForward,
        Arc::clone(&launcher),
        FakeProbe::everything_free(),
        Duration::from_secs(20),
    )
    .await;

    // The `TunnelReady` queued synchronously at launch.
    assert!(session.process_next_effect_result().await);
    assert_eq!(status(&session, id), TunnelStatus::Active);

    assert!(session.process_next_effect_result().await);

    assert_eq!(
        failure_message(&session, id),
        "bind [127.0.0.1]:5432: Address already in use",
        "a forward that could not bind must fail carrying the child's own reason"
    );
    wait_for_shutdowns(&launcher.control, 1, "an always-fatal line after readiness");
    gate.release();
}

#[tokio::test]
async fn output_that_ends_before_readiness_fails_the_tunnel() {
    let launcher = FakeLauncher::yielding(&[]);
    let (mut session, id) = start(
        TunnelKind::IapTcp,
        Arc::clone(&launcher),
        FakeProbe::everything_free(),
        Duration::from_secs(20),
    )
    .await;

    assert!(session.process_next_effect_result().await);

    assert_eq!(
        status(&session, id),
        TunnelStatus::Failed {
            message: "exited before becoming ready".to_string()
        }
    );
}

/// The readiness-timeout path: a child that comes up, says nothing useful, and
/// never reports ready must not leave the tunnel stuck in `Starting` forever.
#[tokio::test]
async fn a_tunnel_that_never_reports_ready_fails_on_the_readiness_deadline() {
    let (launcher, _gate, emitted) = FakeLauncher::gated_after_emitting(&[
        "Testing if tunnel connection works.\u{7}",
        "Still working on it.",
    ]);
    let (mut session, id) = start(
        TunnelKind::IapTcp,
        Arc::clone(&launcher),
        FakeProbe::everything_free(),
        SHORT_READINESS_TIMEOUT,
    )
    .await;

    // The scripted output is handed over in full here, while the test still
    // holds the runtime thread — so both lines are already queued for the
    // supervisor before it is ever polled, and the deadline below no longer has
    // to be long enough for anything to be pumped inside it. Bounded like every
    // other wait in this file: the latch panics naming itself instead of
    // parking forever.
    emitted.wait();

    assert!(session.process_next_effect_result().await);

    let message = failure_message(&session, id);
    assert!(
        message.contains("did not become ready"),
        "the failure must name the timeout, got: {message}"
    );
    assert!(
        message.contains("Still working on it."),
        "the failure must carry the captured output tail, got: {message}"
    );
    assert!(
        message.contains("Testing if tunnel connection works.\u{fffd}"),
        "the captured tail must be sanitized, got: {message}"
    );
    wait_for_shutdowns(&launcher.control, 1, "the readiness deadline expiring");
}

#[tokio::test]
async fn a_missing_binary_fails_the_tunnel_and_registers_nothing() {
    let launcher = FakeLauncher::failing(TunnelLaunchError::MissingBinary);
    let (mut session, id) = start(
        TunnelKind::IapTcp,
        Arc::clone(&launcher),
        FakeProbe::everything_free(),
        Duration::from_secs(20),
    )
    .await;

    assert!(session.process_next_effect_result().await);

    let message = failure_message(&session, id);
    assert!(
        message.contains("binary not found"),
        "a missing binary must say so rather than showing an errno, got: {message}"
    );
    // Nothing was registered, so quitting has nothing to shut down.
    assert!(!session.dispatch_action(Action::QuitRequested).await);
    assert_eq!(launcher.control.shutdowns(), 0);
}

#[tokio::test]
async fn a_launch_failure_reports_the_underlying_reason() {
    let launcher = FakeLauncher::failing(TunnelLaunchError::LaunchFailed {
        message: "resource temporarily unavailable".to_string(),
    });
    let (mut session, id) = start(
        TunnelKind::IapTcp,
        Arc::clone(&launcher),
        FakeProbe::everything_free(),
        Duration::from_secs(20),
    )
    .await;

    assert!(session.process_next_effect_result().await);

    assert!(
        failure_message(&session, id).contains("resource temporarily unavailable"),
        "the launch error must reach the user"
    );
}

#[tokio::test]
async fn no_available_port_fails_the_tunnel_without_launching_anything() {
    let launcher = FakeLauncher::yielding(&["Listening on port [8022]."]);
    // No allocation succeeds here, so there is no port result to drain: the
    // failure is the first thing the runtime reports.
    let (mut session, id) = start_with_plan(
        plan(TunnelKind::IapTcp),
        8022,
        Arc::clone(&launcher),
        FakeProbe::nothing_available(),
        Duration::from_secs(20),
    )
    .await;

    assert!(session.process_next_effect_result().await);

    let message = failure_message(&session, id);
    assert!(
        message.contains("port"),
        "the failure must say the problem was the local port, got: {message}"
    );
    assert!(
        launcher.specs().is_empty(),
        "no child may be launched when no port could be allocated"
    );
}

// --- stopping ----------------------------------------------------------

#[tokio::test]
async fn stopping_a_tunnel_shuts_the_child_down_and_marks_it_stopped() {
    let (launcher, _gate) = FakeLauncher::gated(&[]);
    let (mut session, id) = start(
        TunnelKind::IapTcp,
        Arc::clone(&launcher),
        FakeProbe::everything_free(),
        Duration::from_secs(20),
    )
    .await;

    // The real path: the workspace kill marks the row `Stopping` and emits
    // `Effect::StopTunnel`.
    session
        .dispatch_action(Action::Tunnels(TunnelsAction::KillSelected))
        .await;

    // The stop runs off the run loop, so it is dispatched by the time the key
    // is handled and completes shortly after.
    wait_for_shutdowns(&launcher.control, 1, "the kill");
    assert!(
        launcher
            .control
            .graces()
            .iter()
            .all(|grace| *grace > Duration::ZERO && *grace <= Duration::from_secs(10)),
        "the stop grace must be bounded and non-zero, got {:?}",
        launcher.control.graces()
    );
    // Both the stop acknowledgement and the child's EOF mark the row `Stopped`,
    // and they race; whichever lands first is enough.
    assert!(session.process_next_effect_result().await);
    assert_eq!(status(&session, id), TunnelStatus::Stopped);
}

/// The kill runs *off* the run loop, and the no-leak invariant has to survive
/// that: while a stop is still in flight the control must stay registered, or
/// nothing left in the app — not quit, not the `Drop` backstop — can reach the
/// child. Handing the whole registry to the blocking shutdown would leave a
/// quit during the kill with an empty registry and a live child.
#[tokio::test]
async fn quitting_while_a_kill_is_in_flight_still_reaches_the_child() {
    let (launcher, gate, stall) = FakeLauncher::gated_with_a_stalling_shutdown(&[]);
    let (mut session, id) = start(
        TunnelKind::IapTcp,
        Arc::clone(&launcher),
        FakeProbe::everything_free(),
        Duration::from_secs(20),
    )
    .await;

    session
        .dispatch_action(Action::Tunnels(TunnelsAction::KillSelected))
        .await;

    // Honest from the first frame after the key, while the child is still
    // being reaped.
    assert_eq!(status(&session, id), TunnelStatus::Stopping);
    wait_for_shutdowns(&launcher.control, 1, "the kill");
    assert_eq!(
        launcher.control.completed_shutdowns(),
        0,
        "the run loop must come back while the kill is still in flight"
    );

    assert!(
        !session.dispatch_action(Action::QuitRequested).await,
        "quitting still ends the session"
    );
    assert!(
        launcher.control.shutdowns() >= 2,
        "quit must still reach a child whose kill is in flight, observed {} shutdowns",
        launcher.control.shutdowns()
    );

    wait_for_completed_shutdowns(&launcher.control, 1, "quit released the in-flight kill");
    stall.release();
    gate.release();
}

/// A stop that failed must say so and keep its handle. Reporting `Stopped`
/// would release the port and dismiss the row for a child that is probably
/// still alive, and deregistering the control would leave neither quit nor the
/// `Drop` backstop anything to reach it with.
#[tokio::test]
async fn a_failed_shutdown_marks_the_row_failed_and_keeps_the_child_reachable() {
    let (launcher, gate) = FakeLauncher::gated_with_a_failing_shutdown(&[]);
    let (mut session, id) = start(
        TunnelKind::IapTcp,
        Arc::clone(&launcher),
        FakeProbe::everything_free(),
        Duration::from_secs(20),
    )
    .await;

    session
        .dispatch_action(Action::Tunnels(TunnelsAction::KillSelected))
        .await;
    wait_for_shutdowns(&launcher.control, 1, "the kill");
    let after_kill = launcher.control.shutdowns();

    assert!(session.process_next_effect_result().await);
    assert_eq!(
        failure_message(&session, id),
        SHUTDOWN_FAILURE,
        "a stop that did not work must say so rather than reporting a clean stop"
    );

    assert!(
        !session.dispatch_action(Action::QuitRequested).await,
        "quitting still ends the session"
    );
    assert!(
        launcher.control.shutdowns() > after_kill,
        "a child whose shutdown failed must stay registered so the quit backstop reaches it"
    );

    gate.release();
}

/// The stop is dispatched, not awaited, so the blocking task's own send is the
/// only thing left that can resolve the row. If that task unwinds, nothing is
/// sent and the row stays `Stopping` — which is not terminal, so `d` refuses to
/// dismiss it, its port stays in `claimed_ports` for the rest of the session,
/// and a second `K` is refused by the `Starting | Active` guard. The only way
/// out would be restarting the app. No shipped `TunnelControl` panics today;
/// this pins that the guarantee does not rest on that staying true.
///
/// The panicking blocking task prints its own panic to stderr during this test.
/// That output is expected and does not fail the run: `spawn_blocking` catches
/// the unwind and reports it as a join error.
#[tokio::test]
async fn a_shutdown_that_panics_still_drives_the_row_to_a_terminal_status() {
    // The child reports ready, so the readiness deadline is disarmed and the
    // gate holds its output open: from here the dispatched stop is the *only*
    // thing that can ever resolve this row. Killing a `Starting` row instead
    // would let the readiness timeout resolve it and the test would pass
    // without the fix.
    let (launcher, gate) =
        FakeLauncher::gated_with_a_panicking_shutdown(&["Listening on port [8022]."]);
    let (mut session, id) = start(
        TunnelKind::IapTcp,
        Arc::clone(&launcher),
        FakeProbe::everything_free(),
        Duration::from_secs(20),
    )
    .await;
    assert!(session.process_next_effect_result().await);
    assert_eq!(status(&session, id), TunnelStatus::Active);
    let port = local_port(&session, id);

    session
        .dispatch_action(Action::Tunnels(TunnelsAction::KillSelected))
        .await;
    assert_eq!(status(&session, id), TunnelStatus::Stopping);
    wait_for_shutdowns(&launcher.control, 1, "the kill");

    // Bounded, like every other wait here: without a result the receive would
    // park forever, which is the failure this test is about and not a hang the
    // suite should inherit.
    let outcome = tokio::time::timeout(GATE_TIMEOUT, session.process_next_effect_result()).await;
    let received = outcome.unwrap_or_else(|_| {
        panic!(
            "no result within {GATE_TIMEOUT:?}: a stop whose blocking task panicked left the \
             row in {:?}, with nothing left that could ever resolve it",
            status(&session, id)
        )
    });
    assert!(received, "the effect-result channel is still open");

    // Not merely `is_terminal()`: `Stopped` is terminal too, so that would pass
    // just as well if the join error were reported as a clean stop. The row must
    // say the stop *failed*, and carry the panic's own reason into the MESSAGE
    // column, because a panicked stop leaves the child's fate genuinely unknown.
    let status = status(&session, id);
    assert!(
        matches!(&status, TunnelStatus::Failed { message } if message.contains(PANICKING_SHUTDOWN)),
        "a panicking shutdown must fail the row with its own reason rather than claim a clean \
         stop nobody observed, got {status:?}"
    );
    assert!(
        !session.app().root.tunnels.claimed_ports().contains(&port),
        "a resolved row must release its port, or the allocator avoids {port} for the session"
    );

    gate.release();
}

#[tokio::test]
async fn quitting_the_session_shuts_down_a_live_tunnel() {
    // The no-leak invariant at the app boundary: a tunnel child outlives every
    // command run, so quit is the last chance to stop it.
    let (launcher, _gate) = FakeLauncher::gated(&["Listening on port [8022]."]);
    let (mut session, id) = start(
        TunnelKind::IapTcp,
        Arc::clone(&launcher),
        FakeProbe::everything_free(),
        Duration::from_secs(20),
    )
    .await;
    assert!(session.process_next_effect_result().await);
    assert_eq!(status(&session, id), TunnelStatus::Active);

    assert!(
        !session.dispatch_action(Action::QuitRequested).await,
        "quitting still ends the session"
    );

    assert_eq!(
        launcher.control.shutdowns(),
        1,
        "the live tunnel's child must have been shut down before the session returned"
    );
}
