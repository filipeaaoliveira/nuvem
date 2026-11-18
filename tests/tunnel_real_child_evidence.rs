//! End-to-end evidence: a *real* child process driven through the
//! real runtime, the real reducers, and the real `nuvem::ui::render`.
//!
//! Every other tunnel test drives the supervisor over fakes. This one spawns an
//! actual process with [`ProcessTunnelLauncher`] — the same seam the app uses —
//! so the branch's headline invariant can be observed rather than asserted
//! against a double: killing a tunnel signals the whole **process group**, so a
//! wrapper script that spawned its own worker (the shape `gcloud` has: a shell
//! wrapper that execs Python) leaves nothing behind.
//!
//! The launcher is wrapped so the argv the runtime planned is spawned against a
//! fixture script instead of the real `gcloud`; everything else on the path —
//! port allocation, spawn with `setsid`, output pumping, line classification,
//! reducers, projection, renderer, shutdown — is the production code.
//!
//! Every wait is bounded and names what it was waiting for.

use std::fs;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use nuvem::app::reducer::root::reduce;
use nuvem::app::state::root::Workspace;
use nuvem::app::state::tunnels::{TunnelEntry, TunnelStatus};
use nuvem::app::{Action, AppState, Effect, EffectResult, TunnelLaunchPlan, TunnelsAction};
use nuvem::services::runtime::session::RuntimeSession;
use nuvem::services::runtime::tunnel::port::BindPortProbe;
use nuvem::services::runtime::tunnel::{
    LaunchedTunnel, ProcessTunnelLauncher, TunnelId, TunnelLaunchError, TunnelLauncher,
};
use nuvem_domain::gcp::configuration::{GcloudConfiguration, GcloudConfigurationName};
use nuvem_domain::gcp::tunnel::{TunnelKind, TunnelSpec};
use ratatui::{Terminal, backend::TestBackend};

/// Ceiling for every "has it happened yet?" poll here. The scenarios settle in
/// milliseconds, so exceeding this proves something wedged rather than that the
/// machine was slow.
const DEADLINE: Duration = Duration::from_secs(30);

/// The readiness timeout the session under test runs with.
///
/// Strictly shorter than [`DEADLINE`] on purpose. Both clocks bound the same
/// window — "did this tunnel ever come up?" — and when they were equal, whichever
/// expired first was a coin flip: half the time the test reported the supervisor's
/// own `TunnelFailed` on the row, and half the time it reported the far less
/// useful "nothing arrived at all". Ordering them means a tunnel that never comes
/// up always fails with the supervisor's own message.
const SESSION_READINESS_TIMEOUT: Duration = Duration::from_secs(20);

/// Bounded park the fixture children use to wait for a file the test writes,
/// expressed as `sleep` seconds and an iteration cap so `/bin/sh` can spell it.
/// The cap is longer than [`DEADLINE`], so the test's own bound is always the one
/// that fires first; it exists only so a child can never park forever.
const CHILD_POLL_SECONDS: &str = "0.1";
const CHILD_POLL_LIMIT: u32 = 600;

// --- test seam ---------------------------------------------------------

/// The real launcher, pointed at a fixture script: the runtime plans a `gcloud`
/// argv, and this swaps only `program` so a real `fork`/`exec` happens without
/// needing a Google Cloud SDK on the machine.
struct ScriptLauncher {
    program: PathBuf,
}

impl TunnelLauncher for ScriptLauncher {
    fn launch(&self, spec: &TunnelSpec) -> Result<LaunchedTunnel, TunnelLaunchError> {
        ProcessTunnelLauncher.launch(&TunnelSpec {
            program: self.program.to_string_lossy().into_owned(),
            args: spec.args.clone(),
            description: spec.description.clone(),
        })
    }
}

fn write_script(dir: &Path, name: &str, body: &str) -> PathBuf {
    let path = dir.join(name);
    let mut file = fs::File::create(&path).expect("fixture script is written");
    file.write_all(body.as_bytes()).expect("script body");
    drop(file);
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).expect("script is executable");
    path
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
        preferred_local_port: None,
        remote_port: 22,
        claimed_ports: Vec::new(),
    }
}

/// A session holding one `Starting` row, exactly as the reducer leaves it when
/// it emits `Effect::StartTunnel`, with the Tunnels workspace focused so the
/// rendered screens are the surface a user would be looking at.
async fn start_against(script: PathBuf) -> (RuntimeSession, TunnelId) {
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

    let mut session = RuntimeSession::new_with_tunnel_runtime_for_test(
        app,
        Box::new(ScriptLauncher { program: script }),
        Box::new(BindPortProbe),
        SESSION_READINESS_TIMEOUT,
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

/// Drains effect results until `done` holds, bounded so a result that never
/// arrives fails the test instead of hanging it.
///
/// [`DEADLINE`] bounds the *whole* drain, and each individual wait gets only the
/// budget left over. One bound rather than one per result, because a child that
/// keeps talking without ever reaching the awaited state would renew a per-result
/// timeout indefinitely — and because whichever of two equal bounds happened to
/// expire first decided which message the failure carried. Either expiry now
/// names the waiter *and* prints the row, which is what says whether the tunnel
/// went somewhere unexpected or nothing happened at all.
async fn drain_until(
    session: &mut RuntimeSession,
    id: TunnelId,
    what: &str,
    done: impl Fn(&TunnelEntry) -> bool,
) {
    let deadline = Instant::now() + DEADLINE;
    while !done(&entry(session, id)) {
        // Formatted before the wait so the message is ready without holding
        // anything borrowed across the panic.
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
                    "timed out after {DEADLINE:?} waiting for {what}: no further effect \
                     result arrived; the row is {row}"
                )),
            "the effect result channel closed while waiting for {what}"
        );
    }
}

// --- process bookkeeping -----------------------------------------------

fn read_pid(path: &Path, what: &str) -> i32 {
    let deadline = Instant::now() + DEADLINE;
    loop {
        if let Ok(text) = fs::read_to_string(path)
            && let Ok(pid) = text.trim().parse::<i32>()
        {
            return pid;
        }
        assert!(
            Instant::now() < deadline,
            "timed out after {DEADLINE:?} waiting for {what} to record its pid at {}",
            path.display()
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn is_alive(pid: i32) -> bool {
    // Signal 0 checks for the process without touching it.
    // SAFETY: `kill` takes no pointers, and signal 0 delivers nothing — it only
    // reports whether the pid exists and may be signalled — so no process is
    // affected whatever `pid` names.
    unsafe { libc::kill(pid, 0) == 0 }
}

fn wait_until_gone(pid: i32, what: &str) {
    let deadline = Instant::now() + DEADLINE;
    while is_alive(pid) {
        assert!(
            Instant::now() < deadline,
            "timed out after {DEADLINE:?}: {what} (pid {pid}) survived the kill — a leaked child"
        );
        std::thread::sleep(Duration::from_millis(10));
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

// --- scenarios ---------------------------------------------------------

/// The headline invariant, observed on real pids: a wrapper script that spawns
/// its own worker is stopped as a whole process group, and a benign line the
/// child prints *after* it came up becomes a visible notice instead of killing
/// a healthy forward.
#[tokio::test(flavor = "multi_thread")]
async fn a_real_child_is_supervised_noticed_and_killed_as_a_whole_process_group() {
    let dir = tempfile::tempdir().expect("tempdir");
    let wrapper_pid = dir.path().join("wrapper.pid");
    let worker_pid = dir.path().join("worker.pid");
    // The gate that makes "after readiness" mean *after the supervisor said so*.
    let readiness_seen = dir.path().join("readiness.seen");
    let script = write_script(
        dir.path(),
        "fake-gcloud",
        &format!(
            "#!/bin/sh\n\
             # Stands in for `gcloud`: a wrapper that runs the real tunnel as its\n\
             # own child, so signalling only the direct child would leak the worker.\n\
             echo $$ > {wrapper}\n\
             sleep 600 &\n\
             echo $! > {worker}\n\
             echo \"Listening on port [8022].\"\n\
             # Park until the test says the banner was CLASSIFIED, not merely\n\
             # written. Readiness goes out on stdout and this line on stderr, and\n\
             # the supervisor merges the two pipes with one reader thread each —\n\
             # a merge that preserves order within a pipe and explicitly not\n\
             # between them. Printing this line straight after the banner raced it\n\
             # into the classifier, and a broad error marker seen *before*\n\
             # readiness is fatal by design: the tunnel died instead of taking a\n\
             # notice. Gating on the test's own signal is what makes this a\n\
             # post-readiness line every run.\n\
             waited=0\n\
             while [ ! -f \"{seen}\" ] && [ $waited -lt {limit} ]; do\n\
             \x20 sleep {poll}\n\
             \x20 waited=$((waited + 1))\n\
             done\n\
             echo \"channel 3: open failed: connect failed: Connection refused\" >&2\n\
             wait\n",
            wrapper = wrapper_pid.display(),
            worker = worker_pid.display(),
            seen = readiness_seen.display(),
            limit = CHILD_POLL_LIMIT,
            poll = CHILD_POLL_SECONDS,
        ),
    );

    let (mut session, id) = start_against(script).await;

    drain_until(&mut session, id, "the child's readiness banner", |entry| {
        entry.status == TunnelStatus::Active
    })
    .await;
    assert_eq!(
        entry(&session, id).local_port,
        8022,
        "the row must carry the port the child itself reported"
    );
    let active = render_lines(&session);

    // Releases the child's park above. Everything the child prints from here on
    // is unambiguously post-readiness, so what follows tests the two-tier policy
    // rather than which of two reader threads happened to be scheduled first.
    fs::write(&readiness_seen, b"active\n").expect("the readiness signal is written");

    drain_until(
        &mut session,
        id,
        "the post-readiness benign line to land as a notice",
        |entry| entry.message.is_some(),
    )
    .await;
    let noticed = render_lines(&session);
    let row = entry(&session, id);
    assert_eq!(
        row.status,
        TunnelStatus::Active,
        "a benign line after readiness must not kill a working forward"
    );
    assert!(
        row.message
            .as_deref()
            .is_some_and(|message| message.contains("open failed")),
        "the notice must carry the child's own words, got {:?}",
        row.message
    );

    let wrapper = read_pid(&wrapper_pid, "the wrapper");
    let worker = read_pid(&worker_pid, "the worker");
    assert_ne!(wrapper, worker, "the worker must be a separate process");
    assert!(
        is_alive(wrapper) && is_alive(worker),
        "both must be running"
    );

    session
        .dispatch_action(Action::Tunnels(TunnelsAction::KillSelected))
        .await;
    drain_until(&mut session, id, "the kill to land on the row", |entry| {
        entry.status == TunnelStatus::Stopped
    })
    .await;
    let stopped = render_lines(&session);

    wait_until_gone(wrapper, "the wrapper process");
    wait_until_gone(worker, "the worker the wrapper spawned");

    write_evidence(
        "16-real-child-process-group.txt",
        &format!(
            "A real child process, supervised and killed through the app\n\
             ================================================================\n\
             fixture child : a /bin/sh wrapper that spawns its own worker (the shape `gcloud` has)\n\
             wrapper pid   : {wrapper} (the direct child nuvem spawned, in its own session via setsid)\n\
             worker pid    : {worker} (the grandchild — only a process-group signal reaches it)\n\
             both alive before the kill : yes\n\
             both gone after Shift-K    : yes (wrapper and worker; nothing leaked)",
        ),
        &[
            (
                "1. the child reported \"Listening on port [8022].\" -> Active on the port it named"
                    .to_string(),
                active,
            ),
            (
                "2. the same child then printed \"channel 3: open failed: ...\" AFTER readiness\n   \
                 -> a visible notice on the row, the tunnel stays Active"
                    .to_string(),
                noticed,
            ),
            (
                "3. Shift-K (kill selected) -> Stopped; wrapper AND worker are both reaped"
                    .to_string(),
                stopped,
            ),
        ],
    );
}

/// Quitting the app is the last moment a tunnel child can still be stopped, and
/// the branch's promise is that none survives it. Observed on real pids, on the
/// same wrapper-plus-worker shape.
#[tokio::test(flavor = "multi_thread")]
async fn quitting_nuvem_shuts_a_live_tunnel_child_and_its_worker_down() {
    let dir = tempfile::tempdir().expect("tempdir");
    let wrapper_pid = dir.path().join("wrapper.pid");
    let worker_pid = dir.path().join("worker.pid");
    let script = write_script(
        dir.path(),
        "fake-gcloud-long-lived",
        &format!(
            "#!/bin/sh\n\
             echo $$ > {wrapper}\n\
             sleep 600 &\n\
             echo $! > {worker}\n\
             echo \"Listening on port [8022].\"\n\
             wait\n",
            wrapper = wrapper_pid.display(),
            worker = worker_pid.display(),
        ),
    );

    let (mut session, id) = start_against(script).await;
    drain_until(&mut session, id, "the child's readiness banner", |entry| {
        entry.status == TunnelStatus::Active
    })
    .await;

    let wrapper = read_pid(&wrapper_pid, "the wrapper");
    let worker = read_pid(&worker_pid, "the worker");
    assert!(
        is_alive(wrapper) && is_alive(worker),
        "both must be running"
    );

    assert!(
        !session.dispatch_action(Action::QuitRequested).await,
        "quit must not be vetoed by a live tunnel"
    );
    wait_until_gone(wrapper, "the wrapper process");
    wait_until_gone(worker, "the worker the wrapper spawned");

    write_evidence(
        "18-quit-shuts-real-children-down.txt",
        &format!(
            "Quitting nuvem with a live tunnel leaves nothing behind\n\
             ================================================================\n\
             wrapper pid : {wrapper} (direct child, own session via setsid)\n\
             worker pid  : {worker} (grandchild)\n\
             both alive while the tunnel was Active : yes\n\
             both gone after `q` (QuitRequested)    : yes",
        ),
        &[(
            "the Tunnels workspace at the moment quit was requested".to_string(),
            render_lines(&session),
        )],
    );
}

/// The other half of the two-tier policy: an always-fatal line fails the tunnel
/// at any time, and the row carries the child's own stderr rather than a
/// generic message.
#[tokio::test(flavor = "multi_thread")]
async fn a_child_that_cannot_bind_fails_the_row_with_its_own_stderr() {
    let dir = tempfile::tempdir().expect("tempdir");
    let script = write_script(
        dir.path(),
        "fake-gcloud-bind-failure",
        "#!/bin/sh\n\
         echo \"bind [127.0.0.1]:8022: Address already in use\" >&2\n\
         exit 1\n",
    );

    let (mut session, id) = start_against(script).await;
    drain_until(&mut session, id, "the bind failure", |entry| {
        matches!(entry.status, TunnelStatus::Failed { .. })
    })
    .await;

    let row = entry(&session, id);
    let TunnelStatus::Failed { message } = &row.status else {
        panic!("expected a failed tunnel, got {:?}", row.status);
    };
    assert!(
        message.contains("Address already in use"),
        "the failure must carry the child's own stderr, got {message:?}"
    );

    write_evidence(
        "17-real-child-bind-failure.txt",
        "A real child that cannot bind its local port\n\
         ================================================================\n\
         fixture child : a /bin/sh script that prints a bind error on stderr and exits 1\n\
         the port probe said the port was free; the child's own bind is what failed\n\
         -> the row goes Failed carrying the child's stderr, instead of hiding the race",
        &[(
            "Tunnels workspace after the child failed to bind".to_string(),
            render_lines(&session),
        )],
    );
}
