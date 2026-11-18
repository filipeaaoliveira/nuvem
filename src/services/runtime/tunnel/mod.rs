//! Managed tunnel runtime: the long-lived background children (IAP tunnels,
//! `ssh -L` forwards, Cloud SQL Auth Proxy) that outlive a single command run
//! and therefore need their own local-port allocation and supervision seams,
//! separate from the one-shot `super::interactive` launches.
//!
//! This module owns the supervision seam and the one invariant it exists to
//! guarantee: **a tunnel child is never leaked**. Three decisions carry it, and
//! none of them is a style preference.
//!
//! 1. [`TunnelControl::shutdown`] is synchronous and bounded. [`Drop`] cannot
//!    await, so an async shutdown awaited from [`TunnelRegistry`]'s destructor
//!    would never run and every tunnel would survive the app.
//! 2. The child is spawned into a session of its own (`setsid`) and shutdown
//!    signals the whole **process group**, not the child. `gcloud` is a wrapper
//!    script that execs Python: signalling the direct child would reap the
//!    script and leave the real tunnel running. Process groups are a unix
//!    concept, so the fallback elsewhere is explicitly best-effort and says so
//!    loudly — see `stop_child`.
//! 3. Shutdown escalates on a clock it controls: `SIGTERM`, poll up to `grace`,
//!    then `SIGKILL`. Nothing here can block a destructor indefinitely.
//!
//! Output is a merged line stream over both pipes. `next_line() == None` is the
//! only end-of-output signal, and it happens exactly when both reader threads
//! have dropped their sender, so nothing else may hold a sender clone.

use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use nuvem_domain::gcp::tunnel::TunnelSpec;

pub mod port;

/// Grace [`TunnelRegistry::drop`] gives each child. Short on purpose: the
/// destructor runs while the process is already on its way out, so it must
/// signal, reap what it can, and not linger. This grace is spent per child, so
/// registry teardown is bounded but not constant: worst case it is O(N * grace)
/// for N still-registered tunnels, not O(grace).
const DROP_SHUTDOWN_GRACE: Duration = Duration::from_secs(2);

/// How often a bounded shutdown re-checks whether the child is gone. Short
/// enough that a well-behaved tunnel is reaped almost immediately, long enough
/// not to spin a core while waiting.
const SHUTDOWN_POLL_INTERVAL: Duration = Duration::from_millis(25);

/// How long shutdown waits for the kernel to deliver `SIGKILL` and let the
/// child be reaped. A process cannot ignore `SIGKILL`, so this only covers the
/// delivery window; when even this expires the shutdown reports `Failed`
/// instead of blocking a destructor forever.
#[cfg(unix)]
const KILL_REAP_GRACE: Duration = Duration::from_millis(500);

/// Longest single line handed out by the reader. A child that never emits a
/// newline (a progress spinner, a wedged pipe) would otherwise grow the
/// reader's buffer without bound; past this many bytes the line is cut and the
/// remainder arrives as the next line.
const MAX_LINE_BYTES: usize = 4096;

/// Identifies one managed tunnel for the lifetime of the app run. Defined here
/// because the registry is what makes an id meaningful; every other layer
/// imports it from this module.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TunnelId(pub u64);

impl fmt::Display for TunnelId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.0)
    }
}

/// How a shutdown attempt ended. `Terminated` means the child took the polite
/// signal; `Killed` means it had to be forced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TunnelShutdown {
    /// The child was already gone when shutdown was asked for.
    AlreadyExited,
    /// The child exited within `grace` after the termination signal.
    Terminated,
    /// The child ignored the termination signal and was killed.
    Killed,
    /// Neither signalling nor reaping worked; the message says what failed.
    Failed { message: String },
}

/// Stops one running tunnel child.
///
/// `shutdown` is **synchronous and bounded on purpose**. [`Drop`] cannot await,
/// so an `async fn shutdown` awaited from a destructor would never run and
/// every tunnel would leak its child process when the app exits or the owning
/// state is torn down. This is why the whole supervision seam is built on
/// blocking calls with an explicit `grace` rather than on futures: a blocking
/// call is the only kind a destructor can actually complete.
///
/// Implementations must be idempotent — a second `shutdown` on a child that is
/// already gone returns [`TunnelShutdown::AlreadyExited`] rather than failing.
pub trait TunnelControl: Send + Sync {
    /// Stops the child: terminate, wait up to `grace`, then kill. Blocks for at
    /// most roughly `grace` plus a short reap window.
    fn shutdown(&self, grace: Duration) -> TunnelShutdown;
}

/// The child's merged output, one line at a time.
///
/// `next_line` **blocks** until a line is available, so it belongs on a thread
/// of its own. `None` is the single EOF signal the supervisor waits on: it
/// means the child closed both pipes and no further line can ever arrive.
pub trait TunnelOutput: Send {
    /// Blocking read of the child's next output line. `None` at EOF.
    fn next_line(&mut self) -> Option<String>;
}

/// [`TunnelOutput`] over an mpsc receiver fed by one reader thread per pipe.
///
/// EOF works by sender arithmetic: each reader thread owns a clone of the
/// sender and drops it when its pipe hits EOF, so `recv` fails — and
/// `next_line` yields `None` — only once *both* pipes are done. A sender clone
/// kept alive anywhere else would silently turn that into an infinite wait.
struct ChannelTunnelOutput {
    lines: std::sync::mpsc::Receiver<String>,
}

impl TunnelOutput for ChannelTunnelOutput {
    fn next_line(&mut self) -> Option<String> {
        self.lines.recv().ok()
    }
}

/// Merges two pipes into one line stream. Order *between* the pipes is not
/// defined — two independent threads race — and nothing may depend on it; order
/// *within* each pipe is preserved.
fn merged_line_output(
    stdout: impl std::io::Read + Send + 'static,
    stderr: impl std::io::Read + Send + 'static,
) -> ChannelTunnelOutput {
    let (sender, lines) = std::sync::mpsc::channel();
    let stderr_sender = sender.clone();
    // Each thread moves in its own sender clone and drops it on return; the
    // original `sender` is moved into the stdout thread, so no clone outlives
    // the pipes and the receiver really does disconnect.
    std::thread::spawn(move || forward_lines(stdout, &sender));
    std::thread::spawn(move || forward_lines(stderr, &stderr_sender));
    ChannelTunnelOutput { lines }
}

/// Reads `pipe` to EOF, sending one `String` per line with the trailing line
/// terminator removed. Stops early once the receiver is gone (the tunnel was
/// shut down) so the thread cannot outlive interest in its output.
fn forward_lines(pipe: impl std::io::Read, sender: &std::sync::mpsc::Sender<String>) {
    use std::io::{BufRead, Read};

    let mut reader = std::io::BufReader::new(pipe);
    let mut buffer = Vec::new();
    loop {
        buffer.clear();
        // `take` caps one line's worth of buffering; an over-long line comes
        // out in `MAX_LINE_BYTES` chunks rather than growing without bound.
        let read = reader
            .by_ref()
            .take(MAX_LINE_BYTES as u64)
            .read_until(b'\n', &mut buffer);
        match read {
            Ok(0) => return,
            Ok(_) => {}
            Err(error) => {
                tracing::debug!(%error, "tunnel output pipe read failed");
                return;
            }
        }
        // Lossy on purpose: a child that prints one bad byte should still have
        // the rest of its line surface, not lose the line entirely.
        let line = String::from_utf8_lossy(strip_line_ending(&buffer)).into_owned();
        if sender.send(line).is_err() {
            return;
        }
    }
}

/// Trims one trailing `\r\n`, `\n`, or lone `\r`. Only the terminator: bytes
/// inside the line are the child's, and the display layer sanitizes them.
fn strip_line_ending(line: &[u8]) -> &[u8] {
    let line = line.strip_suffix(b"\n").unwrap_or(line);
    line.strip_suffix(b"\r").unwrap_or(line)
}

/// A running tunnel: its merged output stream and the handle that stops it.
/// The control is an `Arc` because the registry has to keep it after the
/// output has been handed to a reader thread.
pub struct LaunchedTunnel {
    pub output: Box<dyn TunnelOutput>,
    pub control: Arc<dyn TunnelControl>,
}

/// Why a tunnel never started. Split out from the child's own failures (a
/// tunnel that starts and then dies reports through its output) because
/// "gcloud is not installed" is a different sentence to the user.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TunnelLaunchError {
    #[error("the tunnel binary could not be found")]
    MissingBinary,
    #[error("the tunnel process could not be started: {message}")]
    LaunchFailed { message: String },
}

/// Starts a tunnel child. A seam so the session can be tested against fakes
/// without spawning anything; [`ProcessTunnelLauncher`] is the real one.
pub trait TunnelLauncher: Send + Sync {
    fn launch(&self, spec: &TunnelSpec) -> Result<LaunchedTunnel, TunnelLaunchError>;
}

/// Real launcher: spawns the resolved argv with piped stdout/stderr and a null
/// stdin (a background tunnel must never be able to steal the terminal or block
/// on a prompt), in a process group of its own so shutdown can reach the whole
/// tree. No shell is involved.
pub struct ProcessTunnelLauncher;

impl TunnelLauncher for ProcessTunnelLauncher {
    fn launch(&self, spec: &TunnelSpec) -> Result<LaunchedTunnel, TunnelLaunchError> {
        use std::process::{Command, Stdio};

        let mut command = Command::new(&spec.program);
        command
            .args(&spec.args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            // SAFETY: this closure runs in the forked child between `fork` and
            // `exec`, where only async-signal-safe calls are allowed. `setsid`
            // is one, and constructing the error reads `errno` without
            // allocating or locking.
            unsafe {
                command.pre_exec(|| {
                    // The child leads its own session and process group, so
                    // shutdown can signal the group and reach everything the
                    // tunnel forks. `gcloud` is a wrapper script that execs
                    // Python: signalling only the direct child would leave the
                    // real tunnel running. A new session also detaches the
                    // child from our controlling terminal, so a Ctrl-C aimed at
                    // the TUI does not silently kill background tunnels.
                    if libc::setsid() == -1 {
                        // Failing the launch is the safe answer: without our
                        // own process group there is no way to guarantee the
                        // tunnel can be stopped again.
                        return Err(std::io::Error::last_os_error());
                    }
                    Ok(())
                });
            }
        }
        #[cfg(not(unix))]
        tracing::warn!(
            program = %spec.program,
            "tunnel shutdown on this platform is best-effort: only the direct \
             child process can be stopped, so processes it spawns may survive"
        );

        let mut child = command.spawn().map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                TunnelLaunchError::MissingBinary
            } else {
                TunnelLaunchError::LaunchFailed {
                    message: error.to_string(),
                }
            }
        })?;

        let (Some(stdout), Some(stderr)) = (child.stdout.take(), child.stderr.take()) else {
            // Both pipes were requested, so this should be unreachable — but a
            // child we cannot read from must still not be left running.
            let control = ProcessTunnelControl::new(child);
            let outcome = control.shutdown(DROP_SHUTDOWN_GRACE);
            tracing::warn!(
                ?outcome,
                "stopped a tunnel child whose pipes were unavailable"
            );
            return Err(TunnelLaunchError::LaunchFailed {
                message: "the tunnel process pipes were unavailable".to_string(),
            });
        };

        tracing::info!(
            program = %spec.program,
            description = %spec.description,
            pid = child.id(),
            "tunnel process started"
        );
        Ok(LaunchedTunnel {
            output: Box::new(merged_line_output(stdout, stderr)),
            control: Arc::new(ProcessTunnelControl::new(child)),
        })
    }
}

/// How long a demo tunnel's output parks before re-checking whether it has been
/// shut down. The shutdown also notifies, so this is only the backstop that
/// keeps the park bounded: no wait here is ever open-ended, which is what stops
/// a demo tunnel from wedging a `spawn_blocking` thread forever.
const DEMO_WAIT_POLL_INTERVAL: Duration = Duration::from_millis(100);

/// Demo-mode launcher (the same seam, applied to tunnels): produces a tunnel
/// that behaves like a healthy child — one kind-appropriate readiness banner,
/// then output that stays open until it is stopped — without spawning anything.
///
/// It is installed exactly where `DemoCommandRunner` is, so demo mode can show
/// the tunnel workspace with no `gcloud`, no Cloud SQL proxy, and no network.
pub struct DemoTunnelLauncher;

impl TunnelLauncher for DemoTunnelLauncher {
    fn launch(&self, spec: &TunnelSpec) -> Result<LaunchedTunnel, TunnelLaunchError> {
        let gate = Arc::new(DemoTunnelGate::default());
        tracing::info!(
            description = %spec.description,
            "demo tunnel started (no process was spawned)"
        );
        Ok(LaunchedTunnel {
            output: Box::new(DemoTunnelOutput {
                banner: Some(demo_banner(spec)),
                gate: Arc::clone(&gate),
            }),
            control: gate,
        })
    }
}

/// The one line a demo tunnel prints, chosen from the argv so it matches what
/// the real child for that kind would say — the classifier is shared, so a
/// wrong banner would leave the tunnel `Starting` until the readiness deadline.
fn demo_banner(spec: &TunnelSpec) -> String {
    const LOCAL_HOST_PORT: &str = "--local-host-port=";
    if spec.args.iter().any(|arg| arg == "start-iap-tunnel") {
        let port = spec
            .args
            .iter()
            .find_map(|arg| arg.strip_prefix(LOCAL_HOST_PORT))
            .and_then(|host_port| host_port.rsplit(':').next())
            .unwrap_or("0");
        return format!("Listening on port [{port}].");
    }
    if spec.args.iter().any(|arg| arg == "--address") {
        return "The proxy has started successfully and is ready for new connections!".to_string();
    }
    // `ssh -N` prints nothing on success and is `OnSpawn`, so this line must
    // stay unremarkable: anything ready- or error-shaped would be a lie.
    "demo forward established".to_string()
}

/// Shared stop flag behind a demo tunnel: the control sets it, the output waits
/// on it. `Condvar` rather than a plain poll so a shutdown releases the output
/// immediately instead of at the next poll tick.
#[derive(Default)]
struct DemoTunnelGate {
    stopped: std::sync::Mutex<bool>,
    changed: std::sync::Condvar,
}

impl TunnelControl for DemoTunnelGate {
    fn shutdown(&self, _grace: Duration) -> TunnelShutdown {
        let mut stopped = self
            .stopped
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if *stopped {
            return TunnelShutdown::AlreadyExited;
        }
        *stopped = true;
        self.changed.notify_all();
        TunnelShutdown::Terminated
    }
}

/// A demo tunnel's output: the banner once, then a bounded park until the gate
/// is stopped, then EOF. Never sleeps open-ended, so the supervisor's pump
/// thread is always releasable.
struct DemoTunnelOutput {
    banner: Option<String>,
    gate: Arc<DemoTunnelGate>,
}

impl TunnelOutput for DemoTunnelOutput {
    fn next_line(&mut self) -> Option<String> {
        if let Some(banner) = self.banner.take() {
            return Some(banner);
        }
        let mut stopped = self
            .gate
            .stopped
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        while !*stopped {
            let (guard, _timeout) = self
                .gate
                .changed
                .wait_timeout(stopped, DEMO_WAIT_POLL_INTERVAL)
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            stopped = guard;
        }
        None
    }
}

/// Control over one spawned child. The `Mutex` is what makes shutdown safely
/// idempotent: `try_wait`, signal, and reap happen as one critical section, so
/// two callers can never race into signalling a pid that has already been
/// reaped (and possibly reused by the OS for something else).
struct ProcessTunnelControl {
    child: std::sync::Mutex<std::process::Child>,
}

impl ProcessTunnelControl {
    fn new(child: std::process::Child) -> Self {
        Self {
            child: std::sync::Mutex::new(child),
        }
    }
}

impl TunnelControl for ProcessTunnelControl {
    fn shutdown(&self, grace: Duration) -> TunnelShutdown {
        let mut child = match self.child.lock() {
            Ok(child) => child,
            // A panic elsewhere must not be allowed to strand a live child:
            // take the guard anyway and stop the process.
            Err(poisoned) => poisoned.into_inner(),
        };
        match child.try_wait() {
            Ok(Some(_)) => return TunnelShutdown::AlreadyExited,
            Ok(None) => {}
            Err(error) => {
                return TunnelShutdown::Failed {
                    message: format!("could not check the tunnel process: {error}"),
                };
            }
        }
        stop_child(&mut child, grace)
    }
}

/// Escalating, bounded stop: `SIGTERM` to the child's whole process group, poll
/// for up to `grace`, then `SIGKILL` to the group. The group — not the child —
/// because `gcloud` is a wrapper script that execs Python, so signalling the
/// direct child alone would leave the real tunnel running. Residual limitation:
/// `wait_for_exit` reports success as soon as the *direct* child is reaped, so
/// a wrapper that forks a persistent grandchild rather than exec'ing in place
/// could report `Terminated` without SIGKILL ever being tried — the group still
/// receives SIGTERM either way, and `gcloud` execs in place, so this does not
/// bite in practice.
#[cfg(unix)]
fn stop_child(child: &mut std::process::Child, grace: Duration) -> TunnelShutdown {
    // Negative pid means "the process group led by this pid", which is exactly
    // the group `setsid` put the child in at launch.
    let group = -(child.id() as libc::pid_t);

    if let Err(error) = signal_group(group, libc::SIGTERM) {
        tracing::warn!(%error, "SIGTERM to the tunnel process group failed");
    }
    if wait_for_exit(child, grace) {
        return TunnelShutdown::Terminated;
    }

    if let Err(error) = signal_group(group, libc::SIGKILL) {
        return TunnelShutdown::Failed {
            message: format!("the tunnel ignored SIGTERM and SIGKILL failed: {error}"),
        };
    }
    if wait_for_exit(child, KILL_REAP_GRACE) {
        TunnelShutdown::Killed
    } else {
        TunnelShutdown::Failed {
            message: "the tunnel process did not exit after SIGKILL".to_string(),
        }
    }
}

/// Best-effort, unix-only-by-design fallback. Without process groups this
/// reaches **only the direct child**: anything it spawned (for `gcloud`, the
/// Python process behind the wrapper script) can survive, so a non-unix build
/// cannot promise the no-leak invariant. It warns rather than failing quietly,
/// and `ProcessTunnelLauncher::launch` warns again at spawn time.
#[cfg(not(unix))]
fn stop_child(child: &mut std::process::Child, _grace: Duration) -> TunnelShutdown {
    tracing::warn!(
        "stopping only the direct tunnel child: process-group shutdown is \
         unix-only, so processes it spawned may be left running"
    );
    match child.kill() {
        Ok(()) => {
            let _ = child.wait();
            TunnelShutdown::Killed
        }
        Err(error) => TunnelShutdown::Failed {
            message: format!("could not kill the tunnel process: {error}"),
        },
    }
}

/// Sends `signal` to the process group `group` (a negated pid).
#[cfg(unix)]
fn signal_group(group: libc::pid_t, signal: libc::c_int) -> std::io::Result<()> {
    // SAFETY: `kill` takes no pointers and cannot invalidate memory; `group`
    // is the negated pid of a child this process spawned and has not reaped
    // (the caller holds the child lock and checked `try_wait` first).
    let sent = unsafe { libc::kill(group, signal) };
    if sent == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

/// Polls `try_wait` until the child is reaped or `grace` runs out. Bounded by
/// construction — the caller may be a destructor, which must not linger.
#[cfg(unix)]
fn wait_for_exit(child: &mut std::process::Child, grace: Duration) -> bool {
    let deadline = std::time::Instant::now() + grace;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => return true,
            Ok(None) => {}
            Err(error) => {
                tracing::warn!(%error, "could not wait for the tunnel process");
                return false;
            }
        }
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        if remaining.is_zero() {
            return false;
        }
        std::thread::sleep(SHUTDOWN_POLL_INTERVAL.min(remaining));
    }
}

/// Every live tunnel's control handle, keyed by id, with the guarantee that
/// dropping the registry stops every child still in it.
///
/// The registry exists for exactly that destructor: a tunnel child outlives the
/// command that started it, so without a single owner that shuts them all down
/// on the way out, a panic or an early return would leave `gcloud` tunnels
/// running with nothing left to stop them.
#[derive(Default)]
pub struct TunnelRegistry {
    controls: HashMap<TunnelId, Arc<dyn TunnelControl>>,
}

impl TunnelRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Takes ownership of a live tunnel's control handle. Re-registering an id
    /// replaces the previous handle, which is only reachable if ids are reused;
    /// the allocator never does.
    pub fn register(&mut self, id: TunnelId, control: Arc<dyn TunnelControl>) {
        self.controls.insert(id, control);
    }

    /// A handle on one live tunnel, leaving it registered.
    ///
    /// This is what a stop that runs *off* the run loop takes: shutting a child
    /// down takes seconds, and for that whole window the registry must still
    /// hold a handle to it, or a quit (or the [`Drop`] backstop) in the middle
    /// of a stop would have nothing left to reap. Reaching the same control
    /// twice is safe by [`TunnelControl`]'s idempotence contract.
    pub fn control(&self, id: TunnelId) -> Option<Arc<dyn TunnelControl>> {
        self.controls.get(&id).map(Arc::clone)
    }

    /// Shuts one tunnel down and deregisters it, so it can never be signalled
    /// twice. `None` means no tunnel with that id is registered, and nothing
    /// was signalled.
    ///
    /// [`TunnelShutdown::Failed`] is the exception: the child may well still be
    /// running, and this control is the only handle that can ever stop it, so
    /// it is put back rather than dropped. Deregistering it would mean neither
    /// the quit path nor the [`Drop`] backstop could try again — the one way
    /// this module's no-leak invariant fails silently. A control that *panics*
    /// is treated as exactly that kind of failure — see
    /// `shutdown_without_unwinding` — rather than being allowed to unwind
    /// through the caller.
    pub fn shutdown(&mut self, id: TunnelId, grace: Duration) -> Option<TunnelShutdown> {
        let control = self.controls.remove(&id)?;
        let outcome = shutdown_without_unwinding(&control, grace);
        if matches!(outcome, TunnelShutdown::Failed { .. }) {
            self.controls.insert(id, control);
        }
        Some(outcome)
    }

    /// Shuts every registered tunnel down, in id order, and empties the
    /// registry of everything that stopped. Idempotent for those: a second call
    /// finds nothing left and does nothing. A control whose shutdown *failed*
    /// stays registered on purpose (see [`TunnelRegistry::shutdown`]), so a
    /// second call — the [`Drop`] backstop, in practice — retries exactly the
    /// children that are still unaccounted for.
    pub fn shutdown_all(&mut self, grace: Duration) -> Vec<(TunnelId, TunnelShutdown)> {
        let mut ids: Vec<TunnelId> = self.controls.keys().copied().collect();
        // `HashMap` iteration order is unspecified; shutting down (and
        // reporting) in id order keeps the sequence reproducible.
        ids.sort_unstable();
        ids.into_iter()
            .filter_map(|id| self.shutdown(id, grace).map(|outcome| (id, outcome)))
            .collect()
    }

    pub fn len(&self) -> usize {
        self.controls.len()
    }

    pub fn is_empty(&self) -> bool {
        self.controls.is_empty()
    }
}

/// Runs one control's shutdown, turning an unwind into
/// [`TunnelShutdown::Failed`] instead of letting it escape.
///
/// A panicking control is not merely a lost outcome here. This runs under
/// [`TunnelRegistry::shutdown_all`], so an escaping unwind would abandon the
/// id-ordered loop and leave every later tunnel registered and never
/// signalled — precisely the leak this module exists to prevent — and from
/// [`TunnelRegistry::drop`] it would unwind out of a destructor, which aborts
/// the process when a panic is already in flight. `Failed` is the honest
/// outcome: nobody observed the child stop, and the caller keeps the control
/// registered so quit or the [`Drop`] backstop can try again.
///
/// `AssertUnwindSafe` is what makes the borrow compile, and the invariant that
/// justifies it is that a caught unwind leaves this control's internal state
/// possibly inconsistent, so it must never be touched again on this path: the
/// outcome is recorded, the control is re-registered untouched, and nothing
/// here reads through the reference afterwards.
fn shutdown_without_unwinding(control: &Arc<dyn TunnelControl>, grace: Duration) -> TunnelShutdown {
    let attempt =
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| control.shutdown(grace)));
    attempt.unwrap_or_else(|payload| {
        let reason = payload
            .downcast_ref::<&str>()
            .map(|text| (*text).to_string())
            .or_else(|| payload.downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "a payload of unknown type".to_string());
        TunnelShutdown::Failed {
            message: format!("the tunnel shutdown panicked: {reason}"),
        }
    })
}

impl Drop for TunnelRegistry {
    /// Best-effort, blocking shutdown of everything still registered. This is
    /// the last line of defence against leaking a tunnel child, and the reason
    /// [`TunnelControl::shutdown`] is not async.
    fn drop(&mut self) {
        for (id, outcome) in self.shutdown_all(DROP_SHUTDOWN_GRACE) {
            match outcome {
                TunnelShutdown::Failed { message } => {
                    tracing::warn!(tunnel_id = %id, %message, "tunnel shutdown on drop failed")
                }
                outcome => tracing::debug!(tunnel_id = %id, ?outcome, "tunnel shut down on drop"),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nuvem_domain::gcp::tunnel::{
        cloud_sql_proxy_spec, iap_tunnel_spec, ssh_local_forward_spec,
    };
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    /// Fake control that counts `shutdown` calls and records the grace it was
    /// given, so "was this child shut down, once, with a bounded grace?" is
    /// answerable without a real process.
    struct RecordingControl {
        calls: Arc<AtomicUsize>,
        graces: Arc<Mutex<Vec<Duration>>>,
    }

    impl RecordingControl {
        fn new() -> (Arc<Self>, Arc<AtomicUsize>, Arc<Mutex<Vec<Duration>>>) {
            let calls = Arc::new(AtomicUsize::new(0));
            let graces = Arc::new(Mutex::new(Vec::new()));
            let control = Arc::new(Self {
                calls: Arc::clone(&calls),
                graces: Arc::clone(&graces),
            });
            (control, calls, graces)
        }
    }

    impl TunnelControl for RecordingControl {
        fn shutdown(&self, grace: Duration) -> TunnelShutdown {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.graces
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .push(grace);
            TunnelShutdown::Terminated
        }
    }

    /// What the panicking control below unwinds with, so a test can prove the
    /// control's own reason survived into the recorded outcome.
    const PANICKING_SHUTDOWN: &str = "the terminate-poll-kill sequence panicked";

    /// Fake control that unwinds on *every* `shutdown`, standing in for a
    /// future `TunnelControl` that panics somewhere in its terminate-poll-kill
    /// sequence. Every call, not just the first: a control that misbehaved once
    /// would let the registry's retry path look healthy for the wrong reason.
    struct PanickingControl {
        calls: Arc<AtomicUsize>,
    }

    impl PanickingControl {
        fn new() -> (Arc<Self>, Arc<AtomicUsize>) {
            let calls = Arc::new(AtomicUsize::new(0));
            let control = Arc::new(Self {
                calls: Arc::clone(&calls),
            });
            (control, calls)
        }
    }

    impl TunnelControl for PanickingControl {
        fn shutdown(&self, _grace: Duration) -> TunnelShutdown {
            // Recorded before the unwind, so a test can still tell that this
            // control was reached at all.
            self.calls.fetch_add(1, Ordering::SeqCst);
            panic!("{PANICKING_SHUTDOWN}");
        }
    }

    /// Upper bound on how long any test here waits for a reader thread. Only
    /// ever proves "this does not hang": the readers work on in-memory buffers
    /// and finish in microseconds, so seconds of margin cost nothing and the
    /// bound cannot flake on a loaded machine.
    const TEST_DEADLINE: Duration = Duration::from_secs(5);

    /// Runs `body` on a worker thread and fails with a message naming what was
    /// stuck instead of hanging forever if it never finishes.
    fn within_deadline<T: Send + 'static>(
        what: &str,
        body: impl FnOnce() -> T + Send + 'static,
    ) -> T {
        let (done, finished) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = done.send(body());
        });
        match finished.recv_timeout(TEST_DEADLINE) {
            Ok(value) => value,
            Err(_) => panic!("{what} did not finish within {TEST_DEADLINE:?}"),
        }
    }

    /// Drains a tunnel's output to EOF. Bounded, so a reader that never signals
    /// EOF fails the test instead of hanging it.
    fn drain(mut output: Box<dyn TunnelOutput>) -> Vec<String> {
        within_deadline("draining tunnel output to EOF", move || {
            let mut lines = Vec::new();
            while let Some(line) = output.next_line() {
                lines.push(line);
            }
            lines
        })
    }

    /// Lines read from a single pipe, with the other pipe empty.
    fn lines_from(pipe: &'static [u8]) -> Vec<String> {
        drain(Box::new(merged_line_output(pipe, &[][..])))
    }

    #[test]
    fn tunnel_id_displays_its_number() {
        assert_eq!(TunnelId(7).to_string(), "7");
    }

    #[test]
    fn shutdown_reaches_exactly_the_addressed_control() {
        let (first, first_calls, first_graces) = RecordingControl::new();
        let (second, second_calls, _) = RecordingControl::new();
        let mut registry = TunnelRegistry::new();
        registry.register(TunnelId(1), first);
        registry.register(TunnelId(2), second);

        let outcome = registry.shutdown(TunnelId(1), Duration::from_millis(250));

        assert_eq!(outcome, Some(TunnelShutdown::Terminated));
        assert_eq!(first_calls.load(Ordering::SeqCst), 1);
        assert_eq!(second_calls.load(Ordering::SeqCst), 0);
        assert_eq!(
            first_graces.lock().expect("lock").as_slice(),
            [Duration::from_millis(250)],
            "the caller's grace must reach the control unchanged"
        );
        assert_eq!(registry.len(), 1, "a shut-down tunnel is deregistered");
    }

    #[test]
    fn shutdown_with_an_unknown_id_returns_none_and_touches_nothing() {
        let (control, calls, _) = RecordingControl::new();
        let mut registry = TunnelRegistry::new();
        registry.register(TunnelId(1), control);

        assert_eq!(
            registry.shutdown(TunnelId(99), Duration::from_secs(1)),
            None
        );
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        assert_eq!(registry.len(), 1);
    }

    #[test]
    fn shutdown_all_reaches_every_control_exactly_once() {
        let (first, first_calls, _) = RecordingControl::new();
        let (second, second_calls, _) = RecordingControl::new();
        let mut registry = TunnelRegistry::new();
        registry.register(TunnelId(2), second);
        registry.register(TunnelId(1), first);

        let outcomes = registry.shutdown_all(Duration::from_millis(10));

        assert_eq!(
            outcomes,
            vec![
                (TunnelId(1), TunnelShutdown::Terminated),
                (TunnelId(2), TunnelShutdown::Terminated),
            ],
            "outcomes come back in id order regardless of registration order"
        );
        assert_eq!(first_calls.load(Ordering::SeqCst), 1);
        assert_eq!(second_calls.load(Ordering::SeqCst), 1);
        assert!(registry.is_empty());
    }

    #[test]
    fn a_control_that_panics_does_not_strand_the_tunnels_registered_after_it() {
        // The registry shuts down in id order, so an unwind escaping the loop
        // would leave every higher id registered and never signalled — the leak
        // this module exists to prevent — and from `Drop` it would unwind out of
        // a destructor. The panic message this test provokes is printed by the
        // default hook during the run; that output is expected.
        let (panicking, panicking_calls) = PanickingControl::new();
        let (later, later_calls, _) = RecordingControl::new();
        let mut registry = TunnelRegistry::new();
        registry.register(TunnelId(1), panicking);
        registry.register(TunnelId(2), later);

        let outcomes = registry.shutdown_all(Duration::from_millis(10));

        assert_eq!(panicking_calls.load(Ordering::SeqCst), 1);
        assert_eq!(
            later_calls.load(Ordering::SeqCst),
            1,
            "the tunnel after the panicking one must still have been signalled"
        );
        assert_eq!(outcomes.len(), 2, "every id is still reported, in id order");
        assert_eq!(outcomes[0].0, TunnelId(1));
        assert!(
            matches!(&outcomes[0].1, TunnelShutdown::Failed { message } if message.contains(PANICKING_SHUTDOWN)),
            "a caught unwind is recorded as a failed shutdown carrying its own reason, got {:?}",
            outcomes[0].1
        );
        assert_eq!(outcomes[1], (TunnelId(2), TunnelShutdown::Terminated));
        assert_eq!(
            registry.len(),
            1,
            "a shutdown that failed keeps its control registered, so something can still try again"
        );
    }

    #[test]
    fn a_second_shutdown_all_does_not_shut_anything_down_twice() {
        let (control, calls, _) = RecordingControl::new();
        let mut registry = TunnelRegistry::new();
        registry.register(TunnelId(1), control);

        assert_eq!(registry.shutdown_all(Duration::from_millis(10)).len(), 1);
        assert!(
            registry.shutdown_all(Duration::from_millis(10)).is_empty(),
            "the map is empty the second time, so there is nothing to signal"
        );
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn dropping_the_registry_shuts_down_every_registered_child() {
        // THE invariant of this module: a tunnel child is never leaked. `Drop`
        // is synchronous, which is exactly why `TunnelControl::shutdown` is a
        // blocking method — an async shutdown could not be awaited here.
        let (first, first_calls, first_graces) = RecordingControl::new();
        let (second, second_calls, _) = RecordingControl::new();
        let mut registry = TunnelRegistry::new();
        registry.register(TunnelId(1), first);
        registry.register(TunnelId(2), second);

        drop(registry);

        assert_eq!(
            first_calls.load(Ordering::SeqCst),
            1,
            "the first child must have been shut down by the destructor"
        );
        assert_eq!(
            second_calls.load(Ordering::SeqCst),
            1,
            "the second child must have been shut down by the destructor"
        );
        assert_eq!(
            first_graces.lock().expect("lock").as_slice(),
            [DROP_SHUTDOWN_GRACE],
            "the destructor uses a short bounded grace so exit does not linger"
        );
    }

    #[test]
    fn dropping_an_already_drained_registry_shuts_nothing_down_again() {
        let (control, calls, _) = RecordingControl::new();
        let mut registry = TunnelRegistry::new();
        registry.register(TunnelId(1), control);
        registry.shutdown_all(Duration::from_millis(10));

        drop(registry);

        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn a_trailing_newline_does_not_produce_a_phantom_empty_line() {
        assert_eq!(lines_from(b"first\nsecond\n"), ["first", "second"]);
    }

    #[test]
    fn a_final_line_without_a_terminator_is_still_delivered() {
        // Children that die mid-write leave exactly this shape, and the last
        // line is usually the error message worth showing.
        assert_eq!(
            lines_from(b"first\nno newline here"),
            ["first", "no newline here"]
        );
    }

    #[test]
    fn crlf_line_endings_are_stripped() {
        // Windows-style output would otherwise leave a trailing \r, which
        // `sanitize_display_text` turns into a replacement character.
        assert_eq!(lines_from(b"first\r\nsecond\r\n"), ["first", "second"]);
    }

    #[test]
    fn a_lone_trailing_carriage_return_is_stripped() {
        assert_eq!(lines_from(b"progress\r"), ["progress"]);
    }

    #[test]
    fn an_interior_carriage_return_is_left_alone() {
        // Only the line terminator is trimmed: what is inside the line is the
        // child's business, and the display layer sanitizes it downstream.
        assert_eq!(lines_from(b"10%\r20%\n"), ["10%\r20%"]);
    }

    #[test]
    fn blank_lines_are_preserved() {
        assert_eq!(lines_from(b"first\n\nsecond\n"), ["first", "", "second"]);
    }

    #[test]
    fn invalid_utf8_is_replaced_rather_than_dropping_the_line() {
        assert_eq!(lines_from(b"ba\xffd\n"), ["ba\u{FFFD}d"]);
    }

    #[test]
    fn an_endless_line_is_cut_at_the_byte_cap_instead_of_buffering_without_bound() {
        // A child that never emits a newline must not be able to grow the
        // reader's buffer until the app is killed by the OOM killer.
        let mut endless = vec![b'x'; MAX_LINE_BYTES + 10];
        endless.push(b'\n');
        let output = merged_line_output(std::io::Cursor::new(endless), &[][..]);
        let lines = drain(Box::new(output));
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].len(), MAX_LINE_BYTES);
        assert_eq!(lines[1].len(), 10);
    }

    #[test]
    fn output_yields_both_pipes_and_reports_eof_once_both_are_closed() {
        // The EOF signal the supervisor relies on: `next_line` returns `None`
        // only when BOTH reader threads have dropped their sender, so a leaked
        // sender clone would hang the supervisor forever. Interleaving between
        // the two pipes is not guaranteed, so only the set is asserted.
        let output = merged_line_output(&b"out-1\nout-2\n"[..], &b"err-1\n"[..]);
        let mut lines = drain(Box::new(output));
        lines.sort();
        assert_eq!(lines, ["err-1", "out-1", "out-2"]);
    }

    #[test]
    fn output_of_two_empty_pipes_is_immediately_at_eof() {
        assert!(lines_from(b"").is_empty());
    }

    /// Fake output that yields queued lines and then EOF, so a supervisor can
    /// be driven over a scripted transcript without a child process.
    struct ScriptedOutput {
        lines: std::collections::VecDeque<String>,
    }

    impl ScriptedOutput {
        fn new(lines: &[&str]) -> Self {
            Self {
                lines: lines.iter().map(|line| line.to_string()).collect(),
            }
        }
    }

    impl TunnelOutput for ScriptedOutput {
        fn next_line(&mut self) -> Option<String> {
            self.lines.pop_front()
        }
    }

    #[test]
    fn a_launched_tunnel_can_be_assembled_from_fakes_and_registered() {
        // Pins that the seam Task 5 consumes is fully fakeable: a
        // `LaunchedTunnel` built from doubles drains to EOF, and its control
        // is an `Arc<dyn TunnelControl>` the registry accepts and shuts down.
        let (control, calls, _) = RecordingControl::new();
        let launched = LaunchedTunnel {
            output: Box::new(ScriptedOutput::new(&["listening on port [4180]"])),
            control,
        };
        let LaunchedTunnel { output, control } = launched;
        assert_eq!(drain(output), ["listening on port [4180]"]);

        let mut registry = TunnelRegistry::new();
        registry.register(TunnelId(1), control);
        drop(registry);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn launching_a_nonexistent_binary_reports_a_missing_binary() {
        // Spawns nothing: the point is that the spawn *fails*, and that a
        // missing binary is distinguishable from every other launch failure so
        // the UI can say "install gcloud" instead of showing an errno.
        let spec = TunnelSpec {
            program: "nuvem-no-such-tunnel-binary".to_string(),
            args: vec!["--version".to_string()],
            description: "missing binary".to_string(),
        };
        let error = ProcessTunnelLauncher
            .launch(&spec)
            .err()
            .expect("a nonexistent binary cannot launch");
        assert_eq!(error, TunnelLaunchError::MissingBinary);
    }

    /// The demo tunnel must look like a real one to the supervisor: one
    /// kind-appropriate readiness banner, then output that stays open until the
    /// control is shut down. A banner that did not classify as `Ready` would
    /// leave every demo tunnel stuck in `Starting` until the readiness deadline.
    #[test]
    fn a_demo_iap_tunnel_emits_a_ready_banner_carrying_its_local_port() {
        use nuvem_domain::gcp::tunnel::{TunnelKind, TunnelSignal, classify_tunnel_line};

        let spec = iap_tunnel_spec("gcloud", "dev", "web-1", "us-central1-a", 22, 8022);
        let LaunchedTunnel { mut output, .. } = DemoTunnelLauncher
            .launch(&spec)
            .expect("the demo launcher never fails");

        let banner = within_deadline("reading the demo tunnel banner", move || {
            let line = output.next_line();
            // Keep the output alive until the deadline helper returns, or the
            // reader thread would drop it and hide a banner that never came.
            drop(output);
            line
        })
        .expect("the demo tunnel must emit a banner");
        assert_eq!(
            classify_tunnel_line(TunnelKind::IapTcp, &banner),
            TunnelSignal::Ready {
                local_port: Some(8022)
            }
        );
    }

    #[test]
    fn a_demo_cloud_sql_proxy_emits_the_proxy_ready_banner() {
        use nuvem_domain::gcp::tunnel::{TunnelKind, TunnelSignal, classify_tunnel_line};

        let spec = cloud_sql_proxy_spec("cloud-sql-proxy", "proj:us-central1:db-1", 5432);
        let LaunchedTunnel { mut output, .. } = DemoTunnelLauncher
            .launch(&spec)
            .expect("the demo launcher never fails");

        let banner = within_deadline("reading the demo proxy banner", move || {
            let line = output.next_line();
            drop(output);
            line
        })
        .expect("the demo proxy must emit a banner");
        assert_eq!(
            classify_tunnel_line(TunnelKind::CloudSqlProxy, &banner),
            TunnelSignal::Ready { local_port: None }
        );
    }

    #[test]
    fn a_demo_ssh_forward_emits_nothing_that_looks_like_readiness_or_failure() {
        // `ssh -N` is `OnSpawn`: it never announces itself, so the demo line
        // must stay `Ignored` rather than inventing a banner the real child
        // would never print.
        use nuvem_domain::gcp::tunnel::{TunnelKind, TunnelSignal, classify_tunnel_line};

        let spec = ssh_local_forward_spec("gcloud", "dev", "web-1", "us-central1-a", 9200, 9200);
        let LaunchedTunnel { mut output, .. } = DemoTunnelLauncher
            .launch(&spec)
            .expect("the demo launcher never fails");

        let line = within_deadline("reading the demo ssh forward line", move || {
            let line = output.next_line();
            drop(output);
            line
        })
        .expect("the demo tunnel must emit a line");
        assert_eq!(
            classify_tunnel_line(TunnelKind::SshLocalForward, &line),
            TunnelSignal::Ignored
        );
    }

    #[test]
    fn a_demo_tunnel_stays_open_until_it_is_shut_down_and_then_reports_eof() {
        // The invariant the supervisor depends on: no EOF while the tunnel is
        // meant to be live, and a prompt EOF once it is stopped. Both halves are
        // bounded, so a demo tunnel that parked forever fails instead of hanging.
        let spec = iap_tunnel_spec("gcloud", "dev", "web-1", "us-central1-a", 22, 8022);
        let LaunchedTunnel {
            mut output,
            control,
        } = DemoTunnelLauncher
            .launch(&spec)
            .expect("the demo launcher never fails");

        let (reached_eof, at_eof) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            while let Some(_line) = output.next_line() {}
            let _ = reached_eof.send(());
        });
        assert!(
            at_eof.recv_timeout(Duration::from_millis(200)).is_err(),
            "a demo tunnel that was never stopped must not reach EOF"
        );

        assert_eq!(
            control.shutdown(Duration::from_secs(1)),
            TunnelShutdown::Terminated
        );
        at_eof
            .recv_timeout(TEST_DEADLINE)
            .expect("shutting a demo tunnel down must release its output promptly");
        assert_eq!(
            control.shutdown(Duration::from_secs(1)),
            TunnelShutdown::AlreadyExited,
            "shutdown is idempotent, exactly like the process control"
        );
    }

    #[test]
    fn len_and_is_empty_track_registrations() {
        let mut registry = TunnelRegistry::new();
        assert!(registry.is_empty());
        registry.register(TunnelId(1), RecordingControl::new().0);
        registry.register(TunnelId(2), RecordingControl::new().0);
        assert_eq!(registry.len(), 2);
        assert!(!registry.is_empty());
    }
}
