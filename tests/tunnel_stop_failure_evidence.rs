//! Rendered evidence for the never-stuck-row guarantee: a stop whose blocking
//! task unwinds still leaves the user a **terminal**, dismissible row.
//!
//! The kill is fire-and-forget so a terminate-poll-kill sequence cannot freeze
//! the run loop, which means the stop task's own send is the only thing left
//! that can ever resolve the row. If that task panics and nothing is sent, the
//! row stays `Stopping`: not terminal, so `d` refuses to dismiss it, its port
//! stays claimed for the rest of the session, and a second `K` is refused by
//! the `Starting | Active` guard — the user's only way out is restarting the
//! app. The scenario below drives that path through the real session, the real
//! reducers and the real renderer, and captures the four screens a user would
//! actually see.
//!
//! Every wait here is bounded and names what it was waiting for.

use std::collections::VecDeque;
use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use nuvem::app::reducer::root::reduce;
use nuvem::app::state::root::Workspace;
use nuvem::app::state::tunnels::{TunnelEntry, TunnelStatus};
use nuvem::app::{Action, AppState, Effect, EffectResult, TunnelLaunchPlan, TunnelsAction};
use nuvem::services::runtime::session::RuntimeSession;
use nuvem::services::runtime::tunnel::port::PortProbe;
use nuvem::services::runtime::tunnel::{
    LaunchedTunnel, TunnelControl, TunnelId, TunnelLaunchError, TunnelLauncher, TunnelOutput,
    TunnelShutdown,
};
use nuvem_domain::gcp::configuration::{GcloudConfiguration, GcloudConfigurationName};
use nuvem_domain::gcp::tunnel::{TunnelKind, TunnelSpec};
use ratatui::{Terminal, backend::TestBackend};

/// Ceiling for every wait here. The scenario settles in milliseconds, so
/// exceeding this proves something wedged rather than that the machine was slow
/// — and "wedged" is exactly the defect this file is about.
const DEADLINE: Duration = Duration::from_secs(30);

/// What the scripted control unwinds with, so the rendered MESSAGE column can
/// be shown to carry the stop's own reason rather than a generic sentence.
const PANIC_REASON: &str = "a TunnelControl::shutdown that panics";

// --- doubles -----------------------------------------------------------

/// A bounded park keeping the fake child's output open, so the tunnel stays up
/// until the test is done with it. Never an unbounded wait: this runs on a
/// `spawn_blocking` thread, where an uncancellable park wedges `Runtime::drop`.
struct OutputGate {
    released: Mutex<bool>,
    signal: Condvar,
}

impl OutputGate {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            released: Mutex::new(false),
            signal: Condvar::new(),
        })
    }

    fn wait(&self) {
        let deadline = Instant::now() + DEADLINE;
        let mut released = self.released.lock().unwrap();
        while !*released {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                drop(released);
                panic!("the fake tunnel output was never released within {DEADLINE:?}");
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

/// Stands in for a `TunnelControl` whose terminate-poll-kill sequence grows a
/// panic path. Only the *first* shutdown unwinds: the later ones come from the
/// quit and `Drop` backstops, and a destructor that unwinds while the test
/// thread is already panicking aborts the process instead of failing a test.
struct PanickingControl {
    calls: Mutex<usize>,
}

impl TunnelControl for PanickingControl {
    fn shutdown(&self, _grace: Duration) -> TunnelShutdown {
        let mut calls = self
            .calls
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *calls += 1;
        let first = *calls == 1;
        drop(calls);
        if first {
            panic!("{PANIC_REASON}");
        }
        TunnelShutdown::AlreadyExited
    }
}

/// Yields the child's readiness banner and then parks, the way a healthy tunnel
/// holds its pipes open for as long as it is forwarding.
struct GatedOutput {
    lines: VecDeque<String>,
    gate: Arc<OutputGate>,
}

impl TunnelOutput for GatedOutput {
    fn next_line(&mut self) -> Option<String> {
        if let Some(line) = self.lines.pop_front() {
            return Some(line);
        }
        self.gate.wait();
        None
    }
}

struct PanickingLauncher {
    output: Mutex<Option<GatedOutput>>,
}

impl TunnelLauncher for PanickingLauncher {
    fn launch(&self, _spec: &TunnelSpec) -> Result<LaunchedTunnel, TunnelLaunchError> {
        let output = self
            .output
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take()
            .expect("this launcher is scripted for exactly one launch");
        Ok(LaunchedTunnel {
            output: Box::new(output),
            control: Arc::new(PanickingControl {
                calls: Mutex::new(0),
            }),
        })
    }
}

/// Every port is free: this scenario is about the stop, not about allocation.
struct FreeProbe;

impl PortProbe for FreeProbe {
    fn is_free(&self, _port: u16) -> bool {
        true
    }

    fn ephemeral(&self) -> Option<u16> {
        Some(8022)
    }
}

// --- fixtures ----------------------------------------------------------

fn configuration() -> GcloudConfiguration {
    GcloudConfiguration {
        name: GcloudConfigurationName::new("dev"),
        is_active: true,
        account: Some("dev@example.com".to_string()),
        project: Some("dev-project".to_string()),
        region: None,
        zone: None,
    }
}

fn plan() -> TunnelLaunchPlan {
    TunnelLaunchPlan {
        kind: TunnelKind::IapTcp,
        target_label: "worker-dev-1".to_string(),
        remote_label: "22".to_string(),
        instance_name: "worker-dev-1".to_string(),
        zone: "europe-west1-c".to_string(),
        connection_name: "dev-project:us-central1:pg-standalone".to_string(),
        preferred_local_port: Some(8022),
        remote_port: 22,
        claimed_ports: Vec::new(),
    }
}

/// A session holding one `Starting` row with the Tunnels workspace focused, so
/// every rendered screen is the surface a user would be looking at.
async fn start(gate: Arc<OutputGate>) -> (RuntimeSession, TunnelId) {
    let mut app = AppState::new();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ConfigurationsLoaded {
            configurations: vec![configuration()],
        }),
    );
    app.root.workspace = Workspace::Tunnels;
    let context = app.root.current_context().expect("an active context");
    let id = app.root.tunnels.mint_id();
    app.root.tunnels.insert_starting(TunnelEntry {
        id,
        kind: TunnelKind::IapTcp,
        target_label: "worker-dev-1".to_string(),
        local_port: 0,
        remote_label: "22".to_string(),
        status: TunnelStatus::Starting,
        started_at: Instant::now(),
        context: context.clone(),
        message: None,
    });

    let launcher = PanickingLauncher {
        output: Mutex::new(Some(GatedOutput {
            lines: VecDeque::from(vec!["Listening on port [8022].".to_string()]),
            gate,
        })),
    };
    let mut session = RuntimeSession::new_with_tunnel_runtime_for_test(
        app,
        Box::new(launcher),
        Box::new(FreeProbe),
        Duration::from_secs(20),
    );
    session
        .dispatch_action(Action::Effect(Effect::StartTunnel {
            id,
            context,
            plan: plan(),
        }))
        .await;
    (session, id)
}

fn entry(session: &RuntimeSession, id: TunnelId) -> TunnelEntry {
    session
        .app()
        .root
        .tunnels
        .entries()
        .iter()
        .find(|entry| entry.id == id)
        .cloned()
        .expect("the tunnel is still in the list")
}

/// Drains effect results until `done` holds, bounded across the whole drain so
/// a child that keeps talking without ever reaching the awaited state cannot
/// renew a per-result timeout indefinitely.
async fn drain_until(
    session: &mut RuntimeSession,
    id: TunnelId,
    what: &str,
    done: impl Fn(&TunnelEntry) -> bool,
) {
    let deadline = Instant::now() + DEADLINE;
    while !done(&entry(session, id)) {
        let row = format!("{:?}", entry(session, id));
        let remaining = deadline.saturating_duration_since(Instant::now());
        assert!(
            !remaining.is_zero(),
            "timed out after {DEADLINE:?} waiting for {what}; the row is {row}"
        );
        assert!(
            tokio::time::timeout(remaining, session.process_next_effect_result())
                .await
                .unwrap_or_else(|_| panic!(
                    "timed out after {DEADLINE:?} waiting for {what}: no further effect result \
                     arrived; the row is {row}"
                )),
            "the effect result channel closed while waiting for {what}"
        );
    }
}

// --- rendering + evidence ----------------------------------------------

fn render_lines(session: &RuntimeSession) -> Vec<String> {
    let backend = TestBackend::new(112, 16);
    let mut terminal = Terminal::new(backend).expect("terminal");
    terminal
        .draw(|frame| nuvem::ui::render(frame, session))
        .expect("render");
    let buffer = terminal.backend().buffer();
    (0..buffer.area.height)
        .map(|y| {
            (0..buffer.area.width)
                .map(|x| buffer.cell((x, y)).unwrap().symbol().to_string())
                .collect::<String>()
                .trim_end()
                .to_string()
        })
        .collect()
}

fn evidence_dir() -> Option<PathBuf> {
    let path = PathBuf::from(std::env::var("NUVEM_EVIDENCE_DIR").ok()?);
    fs::create_dir_all(&path).ok()?;
    Some(path)
}

fn write_evidence(name: &str, header: &str, sections: &[(String, Vec<String>)]) {
    let Some(dir) = evidence_dir() else {
        return;
    };
    let mut body = String::new();
    body.push_str(header);
    body.push_str("\n\n");
    for (title, lines) in sections {
        body.push_str(title);
        body.push('\n');
        let width = lines.iter().map(|l| l.chars().count()).max().unwrap_or(0);
        body.push('+');
        body.push_str(&"-".repeat(width));
        body.push_str("+\n");
        for line in lines {
            let pad = width - line.chars().count();
            body.push('|');
            body.push_str(line);
            body.push_str(&" ".repeat(pad));
            body.push_str("|\n");
        }
        body.push('+');
        body.push_str(&"-".repeat(width));
        body.push_str("+\n\n");
    }
    fs::write(dir.join(name), body).expect("write evidence");
}

// --- scenario ----------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn a_stop_that_panics_leaves_a_terminal_row_the_user_can_dismiss() {
    let gate = OutputGate::new();
    let (mut session, id) = start(Arc::clone(&gate)).await;

    // The child reports ready, so the readiness deadline is disarmed: from here
    // the dispatched stop is the only thing that can ever resolve this row.
    drain_until(&mut session, id, "the child's readiness banner", |entry| {
        entry.status == TunnelStatus::Active
    })
    .await;
    let port = entry(&session, id).local_port;
    let active = render_lines(&session);

    // Shift-K on the selected row.
    session
        .dispatch_action(Action::Tunnels(TunnelsAction::KillSelected))
        .await;
    assert_eq!(entry(&session, id).status, TunnelStatus::Stopping);
    let stopping = render_lines(&session);

    // Bounded: without a result this receive would park forever, which is the
    // stuck row this scenario exists to rule out.
    let received = tokio::time::timeout(DEADLINE, session.process_next_effect_result())
        .await
        .unwrap_or_else(|_| {
            panic!(
                "no result within {DEADLINE:?}: the stop's blocking task panicked and the row is \
                 stuck in {:?}, with nothing left that could ever resolve it",
                entry(&session, id).status
            )
        });
    assert!(received, "the effect result channel is still open");
    let resolved = render_lines(&session);

    let row = entry(&session, id);
    assert!(
        matches!(&row.status, TunnelStatus::Failed { message } if message.contains(PANIC_REASON)),
        "a panicked stop leaves the child's fate unknown, so the row must say the stop failed and \
         carry its reason rather than claim a clean stop nobody observed, got {:?}",
        row.status
    );
    assert!(
        row.status.is_terminal(),
        "the row must be terminal, or `d` will not dismiss it"
    );
    assert!(
        !session.app().root.tunnels.claimed_ports().contains(&port),
        "a resolved row must release port {port}, or the allocator avoids it all session"
    );

    // `d` on a terminal row: the recovery that a stuck `Stopping` row denied.
    session
        .dispatch_action(Action::Tunnels(TunnelsAction::DismissSelected))
        .await;
    assert!(
        session.app().root.tunnels.entries().is_empty(),
        "the dismissed row must be gone from the list"
    );
    let dismissed = render_lines(&session);

    write_evidence(
        "19-panicking-stop-still-reaches-a-terminal-row.txt",
        &format!(
            "A tunnel stop whose blocking task panics still leaves a terminal row\n\
             ================================================================\n\
             the stop is dispatched, not awaited, so its own send is the only thing that can\n\
             resolve the row. this run's control unwinds with:\n\
             \x20 \"{PANIC_REASON}\"\n\
             so the blocking task's join fails and there is no outcome to report. the status the\n\
             row ends on, in full (the MESSAGE column elides its tail at the column width):\n\
             \x20 {status}\n\n\
             without the fix the row stays in screen 2 forever: STOPPING is not terminal, so\n\
             `d` refuses it, port {port} stays claimed for the session, and a second Shift-K is\n\
             refused by the Starting|Active guard — only restarting the app clears it.\n\n\
             with the fix the join error is turned into a FAILED row carrying the panic's own\n\
             reason (not STOPPED: nobody observed a clean stop), which is terminal, so the port\n\
             is released and `d` works. the control stays registered either way, so quit and the\n\
             Drop backstop can still reach the child.",
            status = format_args!("{:?}", row.status),
        ),
        &[
            (
                "1. the child reported \"Listening on port [8022].\" -> Active".to_string(),
                active,
            ),
            (
                "2. Shift-K -> Stopping, the stop is in flight off the run loop\n   \
                 (before the fix, this screen was the end of the story)"
                    .to_string(),
                stopping,
            ),
            (
                "3. the stop's blocking task panicked -> FAILED carrying the stop's own reason,\n   \
                 terminal, port released"
                    .to_string(),
                resolved,
            ),
            (
                "4. `d` dismisses the terminal row -> the list is empty again".to_string(),
                dismissed,
            ),
        ],
    );

    gate.release();
}
