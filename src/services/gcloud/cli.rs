use std::path::PathBuf;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::{Duration, Instant};

#[cfg(unix)]
use std::os::unix::process::CommandExt;

use tokio_util::sync::CancellationToken;

const PIPE_READ_TIMEOUT: Duration = Duration::from_secs(1);

/// How long a [`CommandRunner::run`] call waits for a *read* — a listing, a
/// `describe`, a `bq show` — before killing it and reporting a timeout.
///
/// Reads are what this number was argued from, and they are now all
/// it applies to: a command that blocks on a GCP operation rather than on a
/// read declares [`CommandCeiling::Lifecycle`] instead. See [`CommandCeiling`]
/// for why a single ceiling could not serve both.
///
/// `run` used to be `Command::output()`, which blocks until the child closes
/// both pipes *and* exits, with no timeout, no kill and no cancellation. Its
/// sibling `run_with_cancellation` has an owner who can end it; `run` has
/// nobody, so a child that never closes its pipes held that thread forever.
/// Since BigQuery, that is not a rare shape: opening a dataset fans
/// out one `bq show` per listed row, up to `RESOURCE_LIST_LIMIT` of them, four
/// at a time behind a semaphore and every one of them through `run`. One hung
/// call holds its permit for good and the fan-out stalls behind it.
///
/// Two minutes is deliberately far above any healthy call and far below
/// forever. It is a ceiling on a hang, not a latency budget: a `bq show`
/// answers in seconds and the slowest listings this app issues are tens of
/// seconds on a large project, so nothing that works reaches this. Tightening
/// it to a number real calls approach would convert slow-but-working into
/// failed, which is the failure this must not trade for.
///
/// `run_with_cancellation` is deliberately *not* bounded by it. A plugin
/// command can legitimately run for minutes, and it has an operator who can
/// end it — a ceiling there would cut off work someone is waiting on. See
/// `run_bounded`, which takes the deadline as an argument so the asymmetry is
/// written down rather than implied.
const RUN_TIMEOUT: Duration = Duration::from_secs(120);

/// The ceiling a lifecycle mutation is held to instead of [`RUN_TIMEOUT`].
///
/// Thirty minutes is not a latency budget any more than the read ceiling is; it
/// is the same kind of number one step further out. `gcloud sql instances
/// failover` and `gcloud compute instances stop` block until GCP reports the
/// operation finished, and how long that takes is the guest's and the service's
/// business: a large VM's stop waits out the guest OS shutdown, and a Cloud SQL
/// `patch --activation-policy=NEVER` drains connections first. Minutes are
/// ordinary there and the tail is long, so the ceiling is placed where only a
/// call that will never answer can reach it.
///
/// What it does *not* buy: a bound on the operation. Killing the CLI ends
/// nuvem's view of it and nothing else — the operation runs to completion in
/// GCP either way, which is why a call that hits this ceiling reports that it
/// may still be running rather than reporting a refusal.
const LIFECYCLE_TIMEOUT: Duration = Duration::from_secs(30 * 60);

/// Which ceiling a [`GcloudCommand`] is held to, and therefore what hitting it
/// means.
///
/// `run` has one deadline for every caller. The argument for it was
/// about *read* latency — a `bq show` answers in seconds, so two minutes can
/// only be reached by a call that is stuck — and it does not transfer to a
/// command that is not reading anything. Two of `run`'s callers are lifecycle
/// mutations whose argv carries no `--async`, so gcloud blocks until GCP
/// finishes the operation; held to the read ceiling, a Cloud SQL failover was
/// SIGKILLed two minutes into an operation that had already fired, and the
/// operator was told the call was refused.
///
/// The ceiling therefore belongs to the command rather than to the runner: the
/// call site that knows the argv is the only one that knows which kind of wait
/// it is buying. This is deliberately a two-variant enum and not a `Duration`
/// per command — the choice is a category with a rationale attached, and a
/// caller free to invent a number would be free to invent it without one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CommandCeiling {
    /// A read: a listing, a `describe`, a `bq show`. Held to the runner's own
    /// ceiling, which is `RUN_TIMEOUT` outside tests. The default, so a call
    /// site that says nothing is treated as the fan-out shape `RUN_TIMEOUT`
    /// bounds.
    #[default]
    Read,
    /// A mutation that blocks on a GCP operation. Held to
    /// `LIFECYCLE_TIMEOUT`, and a timeout here cannot be reported as a call
    /// that did nothing.
    Lifecycle,
}

impl CommandCeiling {
    /// The wall-clock ceiling for this command, given the runner's read
    /// ceiling.
    ///
    /// `Lifecycle` ignores the argument on purpose. The runner's ceiling is
    /// movable — [`ProcessCommandRunner::with_timeout`] exists so a test need
    /// not sit out two minutes — and a lifecycle command that followed it
    /// there would be killed mid-operation by a runner configured for
    /// something else entirely.
    fn duration(self, read: Duration) -> Duration {
        match self {
            Self::Read => read,
            Self::Lifecycle => LIFECYCLE_TIMEOUT,
        }
    }

    /// What a call abandoned at this ceiling has to admit, appended to the
    /// message.
    ///
    /// Shared by the two ways a call ends without a verdict — the deadline
    /// and a cancellation — because the admission is a
    /// property of the command, not of who stopped waiting. Both killed the
    /// CLI; neither touched the operation, and a lifecycle command that had
    /// already reached GCP is still being carried out there.
    ///
    /// Empty for a read: it started nothing, so there is nothing to warn
    /// about, and a warning printed everywhere is a warning nobody reads where
    /// it matters.
    pub(crate) fn abandoned_call_caveat(self) -> &'static str {
        match self {
            Self::Read => "",
            Self::Lifecycle => "; the requested operation may still be running in GCP",
        }
    }
}

/// Which binary a [`GcloudCommand`] spawns.
///
/// nuvem shelled to exactly one program until BigQuery, which has no GA
/// gcloud surface: `gcloud alpha bq` is the only gcloud path to it, and this
/// repo runs no alpha command. `bq` is GA and ships with the same SDK.
///
/// This is not the boundary #171 drew. That decision is about the argv a
/// *plugin* may declare in `plugins.toml`, which stays gcloud-only; config
/// has always carried `k9s_binary`, `kubectl_binary` and
/// `cloud_sql_proxy_binary`, so core nuvem already spawns four binaries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandProgram {
    Gcloud,
    Bq,
}

impl CommandProgram {
    /// The binary this program spawns by default, and the name an error
    /// message uses for it.
    pub fn binary_name(self) -> &'static str {
        match self {
            Self::Gcloud => "gcloud",
            Self::Bq => "bq",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GcloudCommand {
    pub args: Vec<String>,
    /// Which binary runs this argv. Defaulted by [`new`](Self::new) so the
    /// 71 call sites that predate BigQuery keep meaning gcloud.
    pub program: CommandProgram,
    /// Environment entries set on the child, in call order.
    ///
    /// Exists because `bq` has no `--configuration` flag. Every service today
    /// prefixes `--configuration <cfg>`; `bq` resolves its configuration by
    /// shelling to `gcloud config config-helper`, so the only way to aim it
    /// at a named configuration is `CLOUDSDK_ACTIVE_CONFIG_NAME` in its
    /// environment. A `Vec` rather than a map: the set is one or two entries,
    /// and order is worth keeping stable for tests that read it back. A key
    /// repeated in this vec resolves last-wins when applied to the child.
    pub env: Vec<(String, String)>,
    /// How long `run` will wait for this command. Defaulted to
    /// [`CommandCeiling::Read`] by [`new`](Self::new), which is what every
    /// listing and `describe` in the app wants; the two lifecycle argvs say
    /// otherwise for themselves.
    pub ceiling: CommandCeiling,
}

impl GcloudCommand {
    pub fn new(args: impl IntoIterator<Item = impl Into<String>>) -> Self {
        Self {
            args: args.into_iter().map(Into::into).collect(),
            program: CommandProgram::Gcloud,
            env: Vec::new(),
            ceiling: CommandCeiling::Read,
        }
    }

    /// The same argv aimed at the `bq` binary.
    pub fn bq(args: impl IntoIterator<Item = impl Into<String>>) -> Self {
        Self {
            program: CommandProgram::Bq,
            ..Self::new(args)
        }
    }

    /// The same command held to a different ceiling.
    ///
    /// Called at the two lifecycle call sites and nowhere else. Reads say
    /// nothing and get [`CommandCeiling::Read`], which is the safe default in
    /// the direction that matters: a command wrongly left at the read ceiling
    /// is one an operator can reissue, while a read wrongly given the lifecycle
    /// ceiling can hold a fan-out permit for half an hour.
    pub fn with_ceiling(mut self, ceiling: CommandCeiling) -> Self {
        self.ceiling = ceiling;
        self
    }

    /// Sets one environment entry on the child, appending to any already set.
    pub fn with_env(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.env.push((key.into(), value.into()));
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandOutput {
    pub stdout: String,
    pub stderr: String,
}

#[derive(Debug, thiserror::Error)]
pub enum CommandError {
    /// The only variant that names its program: its entire content is which
    /// binary was not found, so reporting gcloud for a missing `bq` sends
    /// the reader to reinstall a binary that is present. `Failed` carries the
    /// child's own output and `Cancelled` names no binary at all, so neither
    /// needs the program. `Io` is not settled: its spawn-path payloads are
    /// interpolated from the command's program, but the reader-timeout and
    /// reader-disconnected payloads built in `receive_pipe_result` still say
    /// "gcloud" unconditionally on a path `bq` now takes — `program` is not in
    /// scope there, and threading it is carried to a later slice.
    #[error("{} CLI not found: {detail}", program.binary_name())]
    MissingExecutable {
        program: CommandProgram,
        detail: String,
    },
    #[error("failed to run command: {0}")]
    Io(String),
    /// A child that ran and refused. Both of its streams ride along, because
    /// which one carries the reason is a property of the *binary*, not of the
    /// call site.
    ///
    /// `gcloud` diagnoses on stderr. `bq` — the second binary nuvem spawns,
    /// added for BigQuery — diagnoses on **stdout** and leaves stderr empty, on
    /// every call nuvem makes of it (`ls -d`, `ls <dataset>`, `show`, all
    /// verified against the live binary on 2026-08-25). This variant carried
    /// stderr alone and `run_bounded` dropped the child's stdout, so every
    /// BigQuery refusal rendered as "command exited unsuccessfully: " with
    /// nothing after the colon, and the classifier that decides retryability
    /// was handed an empty string.
    ///
    /// The narrower fix — the BigQuery service resolving
    /// this for itself — is not reachable from where it would have to live:
    /// the service sees a `CommandError`, and the stdout it would fall back to
    /// had already been discarded before the variant was built. Whichever
    /// layer resolves it, the bytes have to survive to here first, and once
    /// they do, keeping them as a field rather than folding them into `stderr`
    /// is what stops a caller from having to guess which stream it is reading.
    ///
    /// [`failure_diagnostic`] is the resolution, and it prefers stderr, so a
    /// gcloud failure renders exactly as it always did.
    #[error(
        "command exited unsuccessfully: {}",
        failure_diagnostic(stdout, stderr)
    )]
    Failed {
        stdout: String,
        stderr: String,
        exit_code: Option<i32>,
    },
    /// The call was abandoned by its owner before the child produced a verdict:
    /// a plugin the operator cancelled, or a lifecycle action still in flight
    /// when the process began shutting down.
    ///
    /// The ceiling rides along for the same reason it rides along on
    /// [`Self::TimedOut`], and the distinction between the three failures worth
    /// keeping straight is this: `Failed` is a call GCP *refused*, `TimedOut`
    /// is one nuvem stopped waiting on at a deadline, and this is one nuvem
    /// stopped waiting on because somebody decided to. All three killed the
    /// CLI and none of them undid anything in GCP, so a lifecycle command says
    /// so here too — cancelling the call is not cancelling the operation.
    #[error("command was cancelled{}", ceiling.abandoned_call_caveat())]
    Cancelled { ceiling: CommandCeiling },
    /// The child outlived its [`CommandCeiling`] and was killed. Distinct from
    /// `Io` because it is the one failure nuvem itself caused: nothing went
    /// wrong with the pipe or the spawn, the call simply did not come back.
    /// Callers that render a reason want to say so, and the two exhaustive
    /// matches on this enum both had to be visited to add it, which is the
    /// point of a variant over another `Io` string.
    ///
    /// The ceiling rides along because it is what says how much this failure
    /// admits to. A read that timed out started nothing; a lifecycle command
    /// that timed out may have started something that is still running, and a
    /// message that did not say so read as a refusal.
    #[error("command timed out after {seconds}s{}", ceiling.abandoned_call_caveat())]
    TimedOut {
        seconds: u64,
        ceiling: CommandCeiling,
    },
}

/// Which of a failed child's two streams carries the reason it failed.
///
/// stderr wins whenever it holds anything but whitespace; stdout is the
/// fallback. That order is not a preference between the streams, it is which
/// answer is safe when both are non-empty: a `gcloud` failure that also
/// printed a partial JSON document to stdout must still read as its stderr
/// line, and the population of gcloud calls is far larger than the three `bq`
/// ones. The fallback only ever fires where there is nothing to displace.
///
/// Whitespace rather than emptiness is the test because a child that exits
/// non-zero having written a bare newline to stderr has said nothing, and
/// "\n" is not a reason. It returns the *raw* stdout when it falls back —
/// unsanitized and unbounded, like the stderr beside it. Both are sanitized
/// and truncated at display time by `sanitize_detail`, and doing it twice
/// would put this function in the business of formatting.
///
/// Note what this does NOT buy: it restores the *message*, not a better
/// classification. All three captured `bq` wordings classify `Unknown` —
/// exactly what the empty string classified as before this — so the poll
/// ticker still treats a missing table as transient and retries it. That is a
/// gap in `classify_gcloud_stderr`, which has no not-found category at all,
/// and it is not this function's to close. See
/// `crates/nuvem-domain/tests/diagnostics_classify_test.rs`.
pub fn failure_diagnostic<'a>(stdout: &'a str, stderr: &'a str) -> &'a str {
    if stderr.trim().is_empty() {
        stdout
    } else {
        stderr
    }
}

fn map_spawn_error(program: CommandProgram, error: std::io::Error) -> CommandError {
    if error.kind() == std::io::ErrorKind::NotFound {
        CommandError::MissingExecutable {
            program,
            detail: error.to_string(),
        }
    } else {
        CommandError::Io(error.to_string())
    }
}

pub trait CommandRunner {
    fn run(&self, command: GcloudCommand) -> Result<CommandOutput, CommandError>;

    fn run_with_cancellation(
        &self,
        command: GcloudCommand,
        cancellation: &CancellationToken,
    ) -> Result<CommandOutput, CommandError> {
        if cancellation.is_cancelled() {
            Err(CommandError::Cancelled {
                ceiling: command.ceiling,
            })
        } else {
            self.run(command)
        }
    }

    /// [`Self::run`]'s ceiling *and* [`Self::run_with_cancellation`]'s token,
    /// which is a combination neither of the other two offers.
    ///
    /// The lifecycle actions need both and could take neither of the existing
    /// methods. `run` has no owner who can end it, so a batch of VM stops held
    /// `Runtime::drop` for up to `LIFECYCLE_TIMEOUT` after the operator
    /// pressed `q` — the terminal already restored, `SIGTERM` already swallowed
    /// by the tokio handler, and only `kill -9` left.
    /// `run_with_cancellation` is deliberately unbounded, so reaching for it
    /// would have traded that hang for a lifecycle call with no ceiling at all.
    ///
    /// What cancelling buys is that nuvem stops waiting. It is not an abort: a
    /// `stop` gcloud has already handed to GCP runs to completion there, and
    /// [`CommandError::Cancelled`] carries the ceiling so the message can say
    /// so rather than implying the operation was undone.
    fn run_bounded_with_cancellation(
        &self,
        command: GcloudCommand,
        cancellation: &CancellationToken,
    ) -> Result<CommandOutput, CommandError> {
        if cancellation.is_cancelled() {
            Err(CommandError::Cancelled {
                ceiling: command.ceiling,
            })
        } else {
            self.run(command)
        }
    }
}

impl<T: CommandRunner + ?Sized> CommandRunner for &T {
    fn run(&self, command: GcloudCommand) -> Result<CommandOutput, CommandError> {
        (*self).run(command)
    }

    fn run_with_cancellation(
        &self,
        command: GcloudCommand,
        cancellation: &CancellationToken,
    ) -> Result<CommandOutput, CommandError> {
        (*self).run_with_cancellation(command, cancellation)
    }

    fn run_bounded_with_cancellation(
        &self,
        command: GcloudCommand,
        cancellation: &CancellationToken,
    ) -> Result<CommandOutput, CommandError> {
        (*self).run_bounded_with_cancellation(command, cancellation)
    }
}

#[derive(Debug, Clone)]
pub struct ProcessCommandRunner {
    gcloud: PathBuf,
    bq: PathBuf,
    /// The ceiling [`CommandRunner::run`] holds a *read* to. Defaulted to
    /// [`RUN_TIMEOUT`] by both constructors and overridable by
    /// [`with_timeout`](Self::with_timeout), which exists so a test can assert
    /// the timeout fires without sitting out two minutes to do it.
    ///
    /// A command declaring [`CommandCeiling::Lifecycle`] is not held to this at
    /// any value: see [`CommandCeiling::duration`].
    timeout: Duration,
}

impl ProcessCommandRunner {
    /// A runner built from one caller-supplied path, which fills the gcloud
    /// slot, leaving the `bq` slot at a bare `bq` PATH lookup. The one-argument
    /// shape is kept because 13 call sites across six files outside this module
    /// use it, and none of them has a bq path to offer.
    pub fn new(gcloud: PathBuf) -> Self {
        Self {
            gcloud,
            bq: PathBuf::from("bq"),
            timeout: RUN_TIMEOUT,
        }
    }

    /// A runner with both binaries named — what the session builds from the
    /// resolved config.
    pub fn with_binaries(gcloud: PathBuf, bq: PathBuf) -> Self {
        Self {
            gcloud,
            bq,
            timeout: RUN_TIMEOUT,
        }
    }

    /// The same runner holding `run` to a different ceiling.
    ///
    /// Only tests set this today. It is here rather than a `#[cfg(test)]`
    /// helper because the tests that need it are integration tests in a
    /// separate crate, and because a ceiling that cannot be moved is a
    /// ceiling nothing can demonstrate: without it, asserting that the timeout
    /// fires means waiting out `RUN_TIMEOUT`.
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// The single place a program becomes a path. Both `run` and
    /// `run_with_cancellation` go through it, so neither can be wired to a
    /// program the other ignores.
    fn binary(&self, program: CommandProgram) -> &PathBuf {
        match program {
            CommandProgram::Gcloud => &self.gcloud,
            CommandProgram::Bq => &self.bq,
        }
    }
}

impl ProcessCommandRunner {
    /// The one spawn-poll-drain body, shared by both trait methods.
    ///
    /// There used to be two: `run` was a bare `Command::output()` and
    /// `run_with_cancellation` was this. That split cost `run` three things it
    /// silently needed — the deadline, the process-group kill, and the
    /// poll-based pipe draining that is the reason `run_with_cancellation`
    /// carries a comment about the macOS non-CLOEXEC pipe leak. `output()`
    /// concludes on EOF, so a concurrent spawn that inherited this child's pipe
    /// withholds that EOF and `output()` waits on it forever. That is not a
    /// hypothetical shape for a fan-out that spawns four at a time.
    ///
    /// `cancellation` and `deadline` are both optional and both end the call
    /// the same way — kill the group, reap, drain what was written, refuse.
    /// Every caller supplies at least one of them; supplying neither is the
    /// state this function exists to make impossible to reach by accident,
    /// which is why the two call sites below are the only ones.
    fn run_bounded(
        &self,
        command: GcloudCommand,
        cancellation: Option<&CancellationToken>,
        deadline: Option<Instant>,
        timeout: Duration,
    ) -> Result<CommandOutput, CommandError> {
        // Read off the command before its args are consumed, so the two arms
        // that end the call without a verdict — the deadline and the
        // cancellation — can both say which ceiling it was holding.
        let ceiling = command.ceiling;
        if cancellation.is_some_and(CancellationToken::is_cancelled) {
            return Err(CommandError::Cancelled { ceiling });
        }

        let mut process = Command::new(self.binary(command.program));
        process.args(&command.args);
        for (key, value) in &command.env {
            process.env(key, value);
        }
        #[cfg(unix)]
        process.process_group(0);
        let mut child = process
            // `Command::output()` did this for us and `spawn()` does not, so
            // `run` lost it silently when it moved onto this body — and
            // `run_with_cancellation`, which was always a `spawn`, never had it
            // at all. nuvem holds the terminal in raw mode and reads keys off
            // it with an `EventStream`; an inherited stdin is therefore the
            // operator's keyboard, and a child that reads it takes keystrokes
            // nuvem is waiting for and blocks until the ceiling kills it,
            // because nothing sends EOF to a raw-mode terminal. The
            // invocations that are *meant* to talk to the operator — `gcloud
            // compute ssh`, the Cloud SQL proxy, k9s — do not come through
            // here at all: `InteractiveProcessRunner` spawns those, after the
            // TUI has released the terminal.
            //
            // This does change behaviour for one caller rather than only
            // restoring it: a plugin's argv is arbitrary and
            // operator-declared, and `run_with_cancellation` shares this body,
            // so a plugin that used to inherit stdin now gets `/dev/null` and
            // sees EOF. That is the outcome to want. nuvem owns the terminal
            // while a plugin runs, so a prompt it printed would be drawn over
            // and the keystrokes answering it would be the ones nuvem is
            // reading; what looked like an interactive plugin was a plugin
            // stealing input. Null stdin turns that into the fast, legible
            // failure the plugin's own error message can describe.
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| map_spawn_error(command.program, error))?;

        let stdout = child.stdout.take().ok_or_else(|| {
            CommandError::Io(format!(
                "{} stdout pipe unavailable",
                command.program.binary_name()
            ))
        })?;
        let stderr = child.stderr.take().ok_or_else(|| {
            CommandError::Io(format!(
                "{} stderr pipe unavailable",
                command.program.binary_name()
            ))
        })?;
        // Shared with both readers before either starts, and stored only once
        // `terminate_and_reap` has returned — on every exit below. That ordering is
        // the whole guarantee: once it is true, the reaped child has written
        // everything it will ever write, so a reader that finds its pipe empty has
        // read all of the child's output. Storing it before the reap, or skipping an
        // exit, would let a reader conclude while the child was still writing.
        //
        // It says nothing about other writers: a group member that has received the
        // asynchronous kill(-pgid) but is not yet dead, or a descendant that setsid'd
        // out of the group entirely, can still write afterwards, and whatever they
        // write once we have concluded is dropped. That is deliberate — the window
        // for group members is sub-millisecond, the bytes come from a process we just
        // killed rather than from the reaped child whose output callers want, and
        // waiting instead produced an Io error that discarded *all* output, the
        // child's included.
        let owner_done = Arc::new(AtomicBool::new(false));
        let stdout_reader = spawn_pipe_reader(stdout, Arc::clone(&owner_done));
        let stderr_reader = spawn_pipe_reader(stderr, Arc::clone(&owner_done));

        loop {
            if cancellation.is_some_and(CancellationToken::is_cancelled) {
                let _ = terminate_and_reap(&mut child);
                owner_done.store(true, Ordering::Release);
                cleanup_pipe_readers(stdout_reader, stderr_reader, PIPE_READ_TIMEOUT);
                return Err(CommandError::Cancelled { ceiling });
            }

            // The deadline is checked in the same loop as cancellation and
            // ends the call the same way, because it is the same event from
            // the child's point of view: killed by its owner having produced
            // no verdict. What differs is only who decided.
            if deadline.is_some_and(|deadline| Instant::now() >= deadline) {
                let _ = terminate_and_reap(&mut child);
                owner_done.store(true, Ordering::Release);
                cleanup_pipe_readers(stdout_reader, stderr_reader, PIPE_READ_TIMEOUT);
                return Err(CommandError::TimedOut {
                    seconds: timeout.as_secs(),
                    ceiling,
                });
            }

            match child_has_exited(&mut child) {
                Ok(true) => break,
                Ok(false) => thread::sleep(Duration::from_millis(10)),
                Err(error) => {
                    let _ = terminate_and_reap(&mut child);
                    owner_done.store(true, Ordering::Release);
                    cleanup_pipe_readers(stdout_reader, stderr_reader, PIPE_READ_TIMEOUT);
                    return Err(CommandError::Io(error.to_string()));
                }
            }
        }

        let (_, reaped) = terminate_and_reap(&mut child);
        owner_done.store(true, Ordering::Release);
        let status = match reaped {
            Ok(status) => status,
            Err(error) => {
                cleanup_pipe_readers(stdout_reader, stderr_reader, PIPE_READ_TIMEOUT);
                return Err(CommandError::Io(error.to_string()));
            }
        };

        let (stdout, stderr) =
            receive_pipe_results(stdout_reader, stderr_reader, PIPE_READ_TIMEOUT)?;
        let stdout = String::from_utf8_lossy(&stdout).to_string();
        let stderr = String::from_utf8_lossy(&stderr).to_string();

        if status.success() {
            Ok(CommandOutput { stdout, stderr })
        } else {
            // Both streams, not the one this used to keep: see
            // [`CommandError::Failed`]. The child is reaped and its pipes are
            // drained by now, so stdout is in hand either way — it used to be
            // decoded on the line above and then dropped.
            Err(CommandError::Failed {
                stdout,
                stderr,
                exit_code: status.code(),
            })
        }
    }
}

impl CommandRunner for ProcessCommandRunner {
    /// Bounded by the command's own [`CommandCeiling`] and by nothing else:
    /// this is the method with no owner to cancel it, and the ceiling
    /// it holds a call to is the one that call asked for.
    fn run(&self, command: GcloudCommand) -> Result<CommandOutput, CommandError> {
        let timeout = command.ceiling.duration(self.timeout);
        self.run_bounded(command, None, Some(Instant::now() + timeout), timeout)
    }

    /// Cancellable and deliberately *unbounded*. See `RUN_TIMEOUT` for why
    /// this one keeps no ceiling: it has an operator who can end it, and the
    /// commands that reach it are the ones allowed to take minutes.
    fn run_with_cancellation(
        &self,
        command: GcloudCommand,
        cancellation: &CancellationToken,
    ) -> Result<CommandOutput, CommandError> {
        self.run_bounded(command, Some(cancellation), None, self.timeout)
    }

    /// Both bounds at once, which `run_bounded` has always been able to carry
    /// and nothing asked it for until shutdown cancellation.
    fn run_bounded_with_cancellation(
        &self,
        command: GcloudCommand,
        cancellation: &CancellationToken,
    ) -> Result<CommandOutput, CommandError> {
        let timeout = command.ceiling.duration(self.timeout);
        self.run_bounded(
            command,
            Some(cancellation),
            Some(Instant::now() + timeout),
            timeout,
        )
    }
}

/// Report whether the child has exited *without* reaping it, so its pid stays
/// reserved until `terminate_process_tree` has signalled the process group.
///
/// `try_wait` reaps. Once the reaped leader's group has no members left, the pid
/// — and with it the process-group id, which equals the pid here because the
/// child is spawned with `process_group(0)` — is free for the kernel to reuse,
/// so the `kill(-pid, SIGKILL)` that follows can land on an unrelated recycled
/// group. That is the ordinary case: every gcloud invocation that leaves nothing
/// behind. `waitid` with `WNOWAIT` leaves the child a zombie, which keeps the pid
/// ours until we reap deliberately.
#[cfg(unix)]
fn child_has_exited(child: &mut Child) -> std::io::Result<bool> {
    loop {
        // SAFETY: `siginfo_t` is a plain C struct of integers, pointers and
        // padding with no niche and no validity requirement, so all-zero is a
        // valid value for it. `waitid` fills it in below, and the only field
        // read afterwards is `si_signo`, which the zeroing leaves at 0 — the
        // "no child reported" sentinel the result check relies on.
        let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
        // SAFETY: `info` is a live, properly aligned `siginfo_t` owned by this
        // frame for the whole call, and `child.id()` names a child of this
        // process that nothing has reaped yet — `WNOWAIT` is precisely what
        // keeps it waitable — so the `P_PID` target is a pid still reserved to
        // us rather than one the kernel may have recycled.
        let result = unsafe {
            libc::waitid(
                libc::P_PID,
                child.id() as libc::id_t,
                &mut info,
                libc::WEXITED | libc::WNOWAIT | libc::WNOHANG,
            )
        };

        if result == -1 {
            let error = std::io::Error::last_os_error();
            // The caller treats an error as fatal and kills a gcloud tree that is
            // still running, so EINTR must not escape. `try_wait` retried via
            // `cvt_r`; SA_RESTART only masks this until a handler is registered
            // without it.
            if error.kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            return Err(error);
        }

        // A zeroed `si_signo` means WNOHANG found nothing to report yet.
        return Ok(info.si_signo != 0);
    }
}

/// Non-unix has no peek-without-reaping primitive, so `try_wait` reaps here and
/// the pid is released before `terminate_process_tree` runs. The pid-reservation
/// guarantee documented above is unix-only; elsewhere the group kill can still
/// stray onto a recycled pid.
#[cfg(not(unix))]
fn child_has_exited(child: &mut Child) -> std::io::Result<bool> {
    child.try_wait().map(|status| status.is_some())
}

/// Signal the child's whole process tree, reporting what the *group* signal
/// returned. See `terminate_and_reap` for why that result is worth returning.
fn terminate_process_tree(child: &mut Child) -> std::io::Result<()> {
    #[cfg(unix)]
    let signalled = {
        let process_group = -(child.id() as libc::pid_t);
        // SAFETY: `kill` takes no pointers, so the call itself only needs to be
        // an FFI call; what the block has to get right is the target. The child
        // is spawned with `process_group(0)`, so it leads a group whose id is
        // its own pid, and `child_has_exited` peeks with `WNOWAIT` rather than
        // reaping — the pid, and with it the group id, is therefore still
        // reserved to us and cannot name a group the kernel has since recycled.
        if unsafe { libc::kill(process_group, libc::SIGKILL) } == 0 {
            Ok(())
        } else {
            Err(std::io::Error::last_os_error())
        }
    };
    #[cfg(windows)]
    let signalled = {
        let _ = Command::new("taskkill")
            .args(["/T", "/F", "/PID", &child.id().to_string()])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        Ok(())
    };
    #[cfg(not(any(unix, windows)))]
    let signalled = Ok(());

    let _ = child.kill();
    signalled
}

/// Signal the child's process group, then reap the child. Returns the group
/// signal's result and the reap's, in that order.
///
/// These two steps are one operation and must never be split. The process-group
/// id equals the child's pid, because we spawn with `process_group(0)`, and the
/// kernel reserves that pid only while the child is un-reaped. Reap first and,
/// once the group has no members left — the ordinary case, every invocation that
/// leaves nothing behind — the pgid is released, so the `kill(-pid, SIGKILL)`
/// that follows can land on an unrelated group that the kernel has since given
/// the same id. Signalling first keeps the leader a zombie member of its own
/// group, which is what makes the target provably ours.
///
/// The group signal's result is returned rather than dropped because the `ESRCH`
/// it reports on a released pgid is the only evidence of that ordering visible
/// from outside this function; nothing else about the two orders differs.
/// `terminate_and_reap_signals_the_group_while_the_pid_is_still_reserved` asserts
/// on it. Production has no use for the error itself, so callers discard it.
fn terminate_and_reap(child: &mut Child) -> (std::io::Result<()>, std::io::Result<ExitStatus>) {
    let signalled = terminate_process_tree(child);
    let reaped = child.wait();
    (signalled, reaped)
}

/// A pipe a reader thread can consume. On unix the reader also needs the raw fd,
/// because it polls and reads it directly rather than going through `read_to_end`.
#[cfg(unix)]
trait PipeSource: std::io::Read + std::os::unix::io::AsRawFd + Send + 'static {}
#[cfg(unix)]
impl<T: std::io::Read + std::os::unix::io::AsRawFd + Send + 'static> PipeSource for T {}

#[cfg(not(unix))]
trait PipeSource: std::io::Read + Send + 'static {}
#[cfg(not(unix))]
impl<T: std::io::Read + Send + 'static> PipeSource for T {}

/// Spawn a thread that drains `pipe` and reports the bytes on the returned channel.
///
/// `owner_done` is the caller's promise that no process it owns can still write to
/// the pipe: it is set only after the child has been signalled and reaped. The
/// reader uses it to conclude on a drained pipe instead of waiting for an EOF that
/// a stranger holding an inherited write-end copy can withhold indefinitely.
///
/// The parameter is required rather than optional so that the handle has to be
/// threaded through to every reader. Be clear about what that does and does not
/// buy: it makes a reader with no flag at all impossible to construct, but nothing
/// in the type forces a caller to ever *set* the flag it passed, and a reader whose
/// flag stays false is exactly the EOF-only reader this change exists to remove.
/// One store is pinned by a test:
/// `run_with_cancellation_returns_output_when_a_descendant_holds_the_pipes` fails if
/// the success path stops storing it. The stores on the cancellation exit and the
/// peek-error exit are pinned by nothing — delete either and the suite stays green,
/// because both route through `cleanup_pipe_readers`, which discards the readers'
/// result. The residual risk is bounded: those paths return an error whose output is
/// thrown away regardless, so the cost of losing a store there is up to
/// `PIPE_READ_TIMEOUT` of latency, not lost output. Treat them as a convention this
/// code keeps, not as a guarantee anything enforces.
fn spawn_pipe_reader(
    pipe: impl PipeSource,
    owner_done: Arc<AtomicBool>,
) -> Receiver<Result<Vec<u8>, String>> {
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let _ = sender.send(read_pipe(pipe, owner_done));
    });
    receiver
}

fn receive_pipe_result(
    receiver: Receiver<Result<Vec<u8>, String>>,
    name: &str,
    timeout: Duration,
) -> Result<Vec<u8>, CommandError> {
    match receiver.recv_timeout(timeout) {
        Ok(Ok(output)) => Ok(output),
        Ok(Err(error)) => Err(CommandError::Io(error)),
        Err(mpsc::RecvTimeoutError::Timeout) => {
            Err(CommandError::Io(format!("gcloud {name} reader timed out")))
        }
        Err(mpsc::RecvTimeoutError::Disconnected) => Err(CommandError::Io(format!(
            "gcloud {name} reader disconnected"
        ))),
    }
}

fn receive_pipe_results(
    stdout_reader: Receiver<Result<Vec<u8>, String>>,
    stderr_reader: Receiver<Result<Vec<u8>, String>>,
    timeout: Duration,
) -> Result<(Vec<u8>, Vec<u8>), CommandError> {
    let stdout = receive_pipe_result(stdout_reader, "stdout", timeout);
    let stderr = receive_pipe_result(stderr_reader, "stderr", timeout);

    match (stdout, stderr) {
        (Ok(stdout), Ok(stderr)) => Ok((stdout, stderr)),
        (Err(error), _) | (_, Err(error)) => Err(error),
    }
}

fn cleanup_pipe_readers(
    stdout_reader: Receiver<Result<Vec<u8>, String>>,
    stderr_reader: Receiver<Result<Vec<u8>, String>>,
    timeout: Duration,
) {
    let _ = receive_pipe_results(stdout_reader, stderr_reader, timeout);
}

/// How long a drained-but-not-closed pipe parks in `poll` before the loop
/// re-examines `owner_done`. Only a bound on latency: a writer that produces bytes
/// wakes the `poll` immediately.
#[cfg(unix)]
const PIPE_POLL_INTERVAL_MS: libc::c_int = 10;

#[cfg(unix)]
fn set_nonblocking(fd: std::os::unix::io::RawFd) -> std::io::Result<()> {
    // SAFETY: `fcntl` is variadic and `F_GETFL` takes no further argument, so
    // passing none matches the command. `fd` is borrowed from the pipe end the
    // caller owns and keeps open across this call, so the descriptor is live.
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags == -1 {
        return Err(std::io::Error::last_os_error());
    }
    // SAFETY: same live `fd`, and `F_SETFL` takes exactly one `c_int` argument,
    // which is what the flags word passed here is — no pointer is involved.
    if unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } == -1 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

/// Block up to `PIPE_POLL_INTERVAL_MS` waiting for `fd` to become readable. `EINTR` and
/// a timeout are both "nothing to report"; the caller retries either way.
#[cfg(unix)]
fn wait_for_readable(fd: std::os::unix::io::RawFd) -> std::io::Result<()> {
    let mut descriptor = libc::pollfd {
        fd,
        events: libc::POLLIN,
        revents: 0,
    };
    // SAFETY: `descriptor` is a single live, initialised `pollfd` on this frame,
    // and the count of 1 passed alongside it is exactly how many the pointer
    // addresses; `poll` writes only `revents`, within that one struct. Its `fd`
    // is the pipe end the caller owns and keeps open across the call.
    if unsafe { libc::poll(&mut descriptor, 1, PIPE_POLL_INTERVAL_MS) } == -1 {
        let error = std::io::Error::last_os_error();
        if error.kind() != std::io::ErrorKind::Interrupted {
            return Err(error);
        }
    }
    Ok(())
}

/// Drain `pipe` until it holds nothing this process owns.
///
/// `read_to_end` returns only at EOF, and EOF needs *every* copy of the write end
/// closed — including copies we never created. Any concurrent spawn transiently
/// copies the fd table, so a stranger can hold one and withhold EOF for as long as
/// it lives. That is the measured cause of the `reader timed out` flake.
///
/// So the loop concludes on a fact we own instead. `owner_done` is loaded *before*
/// each read attempt, and that ordering is what makes this sound: if it was true at
/// time `S`, the child had already been signalled and reaped before `S`, so every
/// byte the reaped child wrote had reached the pipe before `S`. A read issued after
/// `S` that returns `WouldBlock` therefore proves the pipe holds nothing of the
/// child's, and we can stop without truncating its output. EOF still ends the read on
/// its own, exactly as before, whether or not the flag is ever set.
///
/// Anything a surviving group member — kill(-pgid) is asynchronous — or an
/// out-of-group descendant writes after `S` is dropped. Accepted deliberately: that
/// window is sub-millisecond for group members, the bytes come from a process we just
/// killed rather than from the reaped child whose output callers want, and waiting on
/// EOF instead produced an Io error that discarded *all* output, the child's included.
#[cfg(unix)]
fn read_pipe(pipe: impl PipeSource, owner_done: Arc<AtomicBool>) -> Result<Vec<u8>, String> {
    read_pipe_capped(pipe, owner_done, PIPE_OUTPUT_MAX_BYTES)
}

/// Upper bound on what one pipe read will buffer, applied *at the pipe* rather
/// than after the fact. Every downstream bound (`PluginOutput::capture`'s
/// 256 KiB, the parsers' own limits) trims a `Vec` this function has already
/// paid for in full, so without this a child writing 10 GB to stdout costs
/// 10 GB of memory before any of those caps run.
///
/// 64 MiB is far above any output this program asks for — the largest
/// legitimate reads are `--format=json` resource pages of a few MiB — so
/// hitting it means a runaway or hostile child, and the read fails loudly
/// rather than truncating: a silently cut JSON document would surface as a
/// baffling parse error, while this error names the cause. The child may then
/// block on its next write; that is the state the per-command
/// `CommandCeiling` already exists to kill.
///
/// Unix-only, like the drain-flake fix above it: the non-unix reader below
/// stays unbounded, on a platform this program does not support.
#[cfg(unix)]
const PIPE_OUTPUT_MAX_BYTES: usize = 64 * 1024 * 1024;

#[cfg(unix)]
fn read_pipe_capped(
    mut pipe: impl PipeSource,
    owner_done: Arc<AtomicBool>,
    max_bytes: usize,
) -> Result<Vec<u8>, String> {
    let fd = pipe.as_raw_fd();
    set_nonblocking(fd).map_err(|error| error.to_string())?;

    let mut output = Vec::new();
    let mut buffer = [0u8; 8192];
    loop {
        let owner_was_done = owner_done.load(Ordering::Acquire);

        match pipe.read(&mut buffer) {
            Ok(0) => return Ok(output),
            Ok(read) => {
                output.extend_from_slice(&buffer[..read]);
                if output.len() > max_bytes {
                    return Err(format!(
                        "output exceeded the {max_bytes}-byte pipe ceiling; \
                         refusing to buffer more"
                    ));
                }
            }
            Err(error) => match error.kind() {
                std::io::ErrorKind::WouldBlock => {
                    if owner_was_done {
                        return Ok(output);
                    }
                    wait_for_readable(fd).map_err(|error| error.to_string())?;
                }
                std::io::ErrorKind::Interrupted => continue,
                _ => return Err(error.to_string()),
            },
        }
    }
}

/// Non-unix has no `poll`-and-retry loop here, so the read still ends only at EOF
/// and `owner_done` is unused. The stranger-holds-the-write-end flake is unfixed
/// there.
#[cfg(not(unix))]
fn read_pipe(mut pipe: impl PipeSource, _owner_done: Arc<AtomicBool>) -> Result<Vec<u8>, String> {
    let mut output = Vec::new();
    pipe.read_to_end(&mut output)
        .map(|_| output)
        .map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::time::Instant;

    #[cfg(unix)]
    fn pid_is_reserved(pid: libc::pid_t) -> bool {
        // SAFETY: `kill` takes no pointers, and signal 0 performs only the
        // existence-and-permission check without delivering anything, so no
        // process is disturbed whatever `pid` turns out to name.
        unsafe { libc::kill(pid, 0) == 0 }
    }

    /// Poll `condition` on the production loop's cadence until `deadline`.
    ///
    /// A copy of `tests/support/process.rs`'s: a unit test module inside `src/`
    /// cannot reach a `tests/` support file, and this one is the cheap kind of
    /// duplication — a bounded poll loop, whose reason for existing any reader
    /// re-derives on sight. `has_unreaped_child`, which is not, lives in that
    /// support module only, and the test that needed it moved to
    /// `tests/gcloud_runner_test.rs` where it can be shared.
    #[cfg(unix)]
    fn wait_until(deadline: Duration, mut condition: impl FnMut() -> bool) -> bool {
        let started = Instant::now();
        while started.elapsed() < deadline {
            if condition() {
                return true;
            }
            thread::sleep(Duration::from_millis(10));
        }
        condition()
    }

    #[cfg(unix)]
    #[test]
    fn observing_child_exit_leaves_the_pid_reserved_for_the_group_signal() {
        let mut child = Command::new("/bin/sh")
            .args(["-c", "exit 0"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn /bin/sh");
        let pid = child.id() as libc::pid_t;

        // Bounded so a peek that never reports exit fails this test rather than
        // hanging the job to CI's global timeout. `/bin/sh -c "exit 0"` needs
        // microseconds; 10s absorbs any scheduling stall on a loaded runner.
        let exited = wait_until(Duration::from_secs(10), || {
            child_has_exited(&mut child).expect("peek exit")
        });
        assert!(
            exited,
            "child_has_exited never reported the exit of pid {pid}"
        );

        let reserved = pid_is_reserved(pid);
        let status = child.wait().expect("reap child");

        assert!(status.success(), "{status:?}");
        assert!(
            reserved,
            "pid {pid} was released before terminate_process_tree could signal the group"
        );
    }

    /// The ordering pin for `terminate_and_reap`. The child here leaves no
    /// descendants — the ordinary successful invocation — so its process group
    /// holds nothing but the leader. Signal before reaping and the group still
    /// exists, because the un-reaped leader is a zombie member of it. Reap first
    /// and the group is gone, the signal reports `ESRCH`, and the pgid is back in
    /// the kernel's pool to be handed to somebody else.
    ///
    /// Assert on the absence of `ESRCH` rather than on success: signalling a
    /// zombie-only group is `EPERM` on macOS and succeeds on Linux, but neither
    /// reports a group that is not there.
    #[cfg(unix)]
    #[test]
    fn terminate_and_reap_signals_the_group_while_the_pid_is_still_reserved() {
        // `process_group(0)` is what makes the pgid equal the child's pid, so the
        // spawn has to match production or the group under test never exists.
        let mut child = Command::new("/bin/sh")
            .args(["-c", "exit 0"])
            .process_group(0)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn /bin/sh");
        let pid = child.id();

        let exited = wait_until(Duration::from_secs(10), || {
            child_has_exited(&mut child).expect("peek exit")
        });
        assert!(
            exited,
            "child_has_exited never reported the exit of pid {pid}"
        );

        let (signalled, reaped) = terminate_and_reap(&mut child);

        assert!(reaped.is_ok(), "{reaped:?}");
        let errno = signalled
            .as_ref()
            .err()
            .and_then(std::io::Error::raw_os_error);
        assert_ne!(
            errno,
            Some(libc::ESRCH),
            "the process group of pid {pid} was already gone when we signalled it, so the child was reaped first and the pgid was free to be reused: {signalled:?}"
        );
    }

    /// A pipe whose ends are `FD_CLOEXEC`, so an unrelated concurrent spawn in this
    /// test binary cannot inherit a copy and become a second, unintended stranger.
    #[cfg(unix)]
    fn cloexec_pipe() -> (libc::c_int, libc::c_int) {
        let mut ends = [0 as libc::c_int; 2];
        // SAFETY: `ends` is a live `[c_int; 2]` on this frame, which is exactly
        // the two-element array `pipe` writes the new descriptors into.
        assert_eq!(unsafe { libc::pipe(ends.as_mut_ptr()) }, 0, "pipe");
        for end in ends {
            set_cloexec(end);
        }
        (ends[0], ends[1])
    }

    #[cfg(unix)]
    fn set_cloexec(fd: libc::c_int) {
        assert_ne!(
            // SAFETY: `fd` is one of the two ends `cloexec_pipe` has just
            // created and neither has been closed yet, so the descriptor is
            // live; `F_SETFD` takes one `c_int` argument, which is what the
            // flag passed to this variadic call is.
            unsafe { libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC) },
            -1,
            "fcntl FD_CLOEXEC"
        );
    }

    /// Run `/bin/sh -c "printf hello"` with its stdout on a copy of `write_end`, and
    /// reap it. A real writer that produced output and then exited.
    ///
    /// The copy is made with `F_DUPFD_CLOEXEC`, so it is `FD_CLOEXEC` from the
    /// instant it exists — never inheritable, not even for the window between a
    /// `dup` and a follow-up `F_SETFD`. Leaving it inheritable — the obvious way to
    /// hand it to this child — reopens the hole `cloexec_pipe` closes, because an
    /// inheritable fd is inherited by *every* process spawned while it is open, not
    /// only the intended one. Any other test spawning concurrently would then hold
    /// a copy of the write end, and
    /// `reader_still_concludes_at_eof_when_the_flag_is_never_set` — which has no
    /// `owner_done` to fall back on — would wait for an EOF that copy prevents. The
    /// intended writer still gets it: `Stdio` dup2's the fd onto the child's
    /// stdout, and dup2 does not carry `FD_CLOEXEC` onto the new descriptor.
    #[cfg(unix)]
    fn write_hello_and_reap(write_end: libc::c_int) {
        use std::os::unix::io::FromRawFd;

        // SAFETY: `write_end` is the caller's still-open write end — this
        // function never closes it — and `F_DUPFD_CLOEXEC` takes one `c_int`,
        // the lowest acceptable descriptor number, which is what is passed. The
        // returned descriptor is a fresh one this frame owns; it is checked
        // against -1 below before anything takes ownership of it.
        let child_stdout = unsafe { libc::fcntl(write_end, libc::F_DUPFD_CLOEXEC, 0) };
        assert_ne!(child_stdout, -1, "dup write end");
        let mut child = Command::new("/bin/sh")
            .args(["-c", "printf hello"])
            // SAFETY: `child_stdout` is the fresh descriptor `F_DUPFD_CLOEXEC`
            // returned above, asserted non-negative, and no other value aliases
            // it — the dup deliberately made a copy so that ownership could
            // move here. `Stdio` becomes its sole owner and closes it once the
            // spawn has dup2'd it onto the child's stdout.
            .stdout(unsafe { Stdio::from_raw_fd(child_stdout) })
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn /bin/sh");
        let status = child.wait().expect("reap /bin/sh");
        assert!(status.success(), "{status:?}");
    }

    /// The defect shape measured on this branch: a stranger holds a copy of the write
    /// end that the runner never created and cannot see, so EOF never arrives even
    /// though the process we own has written its output and been reaped. Here the test
    /// itself is that stranger, holding `write_end` open across the whole read.
    ///
    /// Waiting on EOF, this times out. Concluding on "the pipe is drained and no
    /// process we own can still write" — `owner_done` set after the reap — returns the
    /// child's bytes.
    /// Pins the park in `wait_for_readable`: on a pipe holding no bytes, the
    /// call must sit in `poll` for the poll interval rather than return at
    /// once. Time is the only observable it has — every return path is
    /// `Ok(())` — and the reader loop's CPU cost rests on that park: a
    /// `wait_for_readable` that returns immediately keeps every other test
    /// green while turning the drain loop into a busy-spin. The floor sits
    /// below `PIPE_POLL_INTERVAL_MS` to absorb coarse clocks; an immediate
    /// return is microseconds and misses it by three orders of magnitude.
    #[cfg(unix)]
    #[test]
    fn wait_for_readable_parks_on_a_silent_pipe_and_wakes_on_bytes() {
        let (read_end, write_end) = cloexec_pipe();

        let started = Instant::now();
        wait_for_readable(read_end).expect("poll on a silent pipe");
        let parked = started.elapsed();
        assert!(
            parked >= Duration::from_millis(8),
            "wait_for_readable returned after {parked:?} on a pipe with no \
             bytes; the drain loop relies on it parking ~{PIPE_POLL_INTERVAL_MS}ms"
        );

        assert_eq!(
            // SAFETY: `write_end` is the live write end created above and not
            // yet closed; the buffer is one live byte on this frame, matching
            // the count of 1 passed beside it.
            unsafe { libc::write(write_end, [b'x'].as_ptr().cast(), 1) },
            1,
            "write one byte"
        );
        wait_for_readable(read_end).expect("poll on a readable pipe");

        for end in [read_end, write_end] {
            // SAFETY: both ends were created by `cloexec_pipe` above, are owned
            // by this test alone, and each is closed exactly once here.
            assert_eq!(unsafe { libc::close(end) }, 0, "close pipe end");
        }
    }

    /// The ceiling's magnitude is part of the contract: it must dwarf every
    /// legitimate read (multi-MiB JSON pages) while still bounding a runaway
    /// child. Pinned against the independently written-out value so an edit
    /// that accidentally turns 64 MiB into 66 KiB or 1 MiB — the shapes a
    /// slipped operator produces — fails by name.
    #[cfg(unix)]
    #[test]
    fn the_pipe_ceiling_is_sixty_four_mebibytes() {
        assert_eq!(PIPE_OUTPUT_MAX_BYTES, 67_108_864);
    }

    /// The ceiling exists to bound memory at the pipe, before any downstream
    /// cap sees the bytes. Exercised with a tiny injected cap so the boundary
    /// is pinned on both sides — at the cap is a normal read, one byte past
    /// it is a named refusal — without pushing 64 MiB through a test pipe.
    #[cfg(unix)]
    #[test]
    fn reading_stops_with_a_named_error_one_byte_past_the_ceiling() {
        use std::io::Write as _;
        use std::os::unix::io::FromRawFd as _;

        let (read_end, write_end) = cloexec_pipe();
        // SAFETY: `cloexec_pipe` minted both descriptors above; each is moved
        // into exactly one owning `File`, which becomes its single closer.
        let mut writer = unsafe { std::fs::File::from_raw_fd(write_end) };
        writer.write_all(&[b'x'; 1025]).expect("write over the cap");
        drop(writer);

        // SAFETY: as above — `read_end` is live and this `File` takes sole
        // ownership of it.
        let reader = unsafe { std::fs::File::from_raw_fd(read_end) };
        let error = read_pipe_capped(reader, Arc::new(AtomicBool::new(false)), 1024)
            .expect_err("1025 bytes must not fit under a 1024-byte ceiling");
        assert!(error.contains("ceiling"), "unexpected error: {error}");
    }

    #[cfg(unix)]
    #[test]
    fn an_output_of_exactly_the_ceiling_is_returned_whole() {
        use std::io::Write as _;
        use std::os::unix::io::FromRawFd as _;

        let (read_end, write_end) = cloexec_pipe();
        // SAFETY: same single-owner transfer as the over-cap test above.
        let mut writer = unsafe { std::fs::File::from_raw_fd(write_end) };
        writer.write_all(&[b'x'; 1024]).expect("write at the cap");
        drop(writer);

        // SAFETY: as above.
        let reader = unsafe { std::fs::File::from_raw_fd(read_end) };
        let output = read_pipe_capped(reader, Arc::new(AtomicBool::new(false)), 1024)
            .expect("an at-cap output is not an overflow");
        assert_eq!(output.len(), 1024);
    }

    #[cfg(unix)]
    #[test]
    fn reader_concludes_when_the_pipe_is_drained_and_no_owned_writer_remains() {
        use std::fs::File;
        use std::os::unix::io::FromRawFd;

        let (read_end, write_end) = cloexec_pipe();
        write_hello_and_reap(write_end);

        let owner_done = Arc::new(AtomicBool::new(false));
        let reader = spawn_pipe_reader(
            // SAFETY: `read_end` is the read end `cloexec_pipe` returned, still
            // open and held by nothing else — only the write end has been
            // handed out, and only as a dup. The `File` becomes its sole owner
            // and is the one thing that ever closes it.
            unsafe { File::from_raw_fd(read_end) },
            Arc::clone(&owner_done),
        );
        // Exactly what `run_with_cancellation` does once `terminate_and_reap` returns.
        owner_done.store(true, Ordering::Release);

        let result = receive_pipe_result(reader, "stdout", PIPE_READ_TIMEOUT);

        // SAFETY: this test is the stranger holding `write_end`, so the
        // descriptor is still open here — `write_hello_and_reap` gave the child
        // a dup, never this one — and no `File` or `Stdio` owns it. This is
        // therefore its single close, and nothing can use it afterwards.
        assert_eq!(unsafe { libc::close(write_end) }, 0, "close write end");
        assert_eq!(
            result.expect("reader should conclude on the drained pipe"),
            b"hello",
        );
    }

    /// The flag is an additional way to conclude, not a replacement for EOF. Here every
    /// write-end copy is gone and `owner_done` is never set, so only the EOF path can
    /// finish this read.
    #[cfg(unix)]
    #[test]
    fn reader_still_concludes_at_eof_when_the_flag_is_never_set() {
        use std::fs::File;
        use std::os::unix::io::FromRawFd;

        let (read_end, write_end) = cloexec_pipe();
        write_hello_and_reap(write_end);
        // SAFETY: `write_end` is still open — the child was given a dup of it,
        // which the reap has already closed — and unowned, so this is its
        // single close. Nothing below names the descriptor again.
        assert_eq!(unsafe { libc::close(write_end) }, 0, "close write end");

        // A flag the test never sets: the reader holds one, as every reader now must,
        // but only the EOF path can conclude this read.
        let owner_done = Arc::new(AtomicBool::new(false));
        // SAFETY: `read_end` is the read end from `cloexec_pipe`, still open and
        // held by nothing else; the `File` takes sole ownership and closes it
        // when the reader thread drops it.
        let reader = spawn_pipe_reader(unsafe { File::from_raw_fd(read_end) }, owner_done);

        let output = receive_pipe_result(reader, "stdout", PIPE_READ_TIMEOUT)
            .expect("reader should conclude at EOF");

        assert_eq!(output, b"hello");
    }

    #[cfg(unix)]
    #[test]
    fn successful_command_returns_stdout_and_stderr() {
        let runner = ProcessCommandRunner::new(PathBuf::from("/bin/sh"));

        let output = runner
            .run_with_cancellation(
                GcloudCommand::new(["-c", "printf hello; printf oops >&2"]),
                &CancellationToken::new(),
            )
            .expect("command succeeds");

        assert_eq!(output.stdout, "hello");
        assert_eq!(output.stderr, "oops");
    }

    #[cfg(unix)]
    #[test]
    fn non_zero_exit_reports_its_exit_code() {
        let runner = ProcessCommandRunner::new(PathBuf::from("/bin/sh"));

        let error = runner
            .run_with_cancellation(
                GcloudCommand::new(["-c", "printf nope >&2; exit 3"]),
                &CancellationToken::new(),
            )
            .unwrap_err();

        match error {
            CommandError::Failed {
                stdout,
                stderr,
                exit_code,
            } => {
                assert_eq!(stdout, "");
                assert_eq!(stderr, "nope");
                assert_eq!(exit_code, Some(3));
            }
            other => panic!("{other:?}"),
        }
    }

    /// An executable that reports the argv the process actually received, one
    /// bracketed field per argument. It is the invoked binary rather than a
    /// script handed to `/bin/sh -c`, so nothing parses the arguments before it
    /// sees them: `command.args` arrive verbatim as `$1..$n`, and an argument
    /// injected below the `CommandRunner` boundary shows up as an extra field
    /// whether or not a shell would have accepted it as an option.
    #[cfg(unix)]
    fn argv_echo_binary(directory: &std::path::Path) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;

        let path = directory.join("argv-echo");
        std::fs::write(
            &path,
            "#!/bin/sh\nfor arg in \"$@\"; do printf '[%s]' \"$arg\"; done\n",
        )
        .expect("write argv echo executable");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
            .expect("mark argv echo executable");
        path
    }

    /// The argv a lifecycle preview promises: a realistic gcloud invocation,
    /// plus one argument containing a space so a mismatch also catches argv
    /// being flattened through a shell.
    #[cfg(unix)]
    const ARGV_ECHO_ARGS: [&str; 6] = [
        "--configuration",
        "prod",
        "compute",
        "instances",
        "stop",
        "web 1",
    ];

    #[cfg(unix)]
    fn argv_echo_command() -> GcloudCommand {
        GcloudCommand::new(ARGV_ECHO_ARGS)
    }

    /// Derived from the same fixture argv the command is built from, so the
    /// tests compare the process argv against what the caller actually built.
    #[cfg(unix)]
    fn argv_echo_expected() -> String {
        ARGV_ECHO_ARGS
            .iter()
            .map(|arg| format!("[{arg}]"))
            .collect()
    }

    /// The floor under every argv/preview parity test in the repo. Those tests
    /// compare the argv a fake `CommandRunner` *received* against the rendered
    /// "Command:" line, so they see nothing that happens below this boundary: a
    /// global `--quiet`, `--verbosity` or `--impersonate-service-account`
    /// prepended here would leave every parity test green while every preview
    /// understated what actually ran. These two assert that `ProcessCommandRunner`
    /// itself hands the argv it was given to the process verbatim; a decorator
    /// wrapping the runner at a call site is a separate bypass they do not cover.
    #[cfg(unix)]
    #[test]
    fn run_hands_the_command_argv_to_the_process_verbatim() {
        let directory = tempfile::tempdir().expect("tempdir");
        let runner = ProcessCommandRunner::new(argv_echo_binary(directory.path()));

        let output = runner.run(argv_echo_command()).expect("command succeeds");

        assert_eq!(
            output.stdout,
            argv_echo_expected(),
            "run altered the argv it was given; every command preview now understates what runs"
        );
    }

    #[cfg(unix)]
    #[test]
    fn run_with_cancellation_hands_the_command_argv_to_the_process_verbatim() {
        let directory = tempfile::tempdir().expect("tempdir");
        let runner = ProcessCommandRunner::new(argv_echo_binary(directory.path()));

        let output = runner
            .run_with_cancellation(argv_echo_command(), &CancellationToken::new())
            .expect("command succeeds");

        assert_eq!(
            output.stdout,
            argv_echo_expected(),
            "run_with_cancellation altered the argv it was given; every command preview now understates what runs"
        );
    }

    #[test]
    fn missing_gcloud_binary_reports_gcloud() {
        let runner =
            ProcessCommandRunner::new(std::path::PathBuf::from("definitely-not-a-real-binary-xyz"));

        let err = runner.run(GcloudCommand::new(["version"])).unwrap_err();

        match err {
            CommandError::MissingExecutable { program, .. } => {
                assert_eq!(program, CommandProgram::Gcloud);
            }
            other => panic!("{other:?}"),
        }
        assert!(
            runner
                .run(GcloudCommand::new(["version"]))
                .unwrap_err()
                .to_string()
                .starts_with("gcloud CLI not found"),
            "message should name gcloud"
        );
    }

    /// The point of the variant carrying a program at all: a missing `bq`
    /// used to report a missing gcloud, sending the reader to reinstall a
    /// binary that is present.
    #[test]
    fn missing_bq_binary_reports_bq() {
        let runner = ProcessCommandRunner::with_binaries(
            std::path::PathBuf::from("/bin/sh"),
            std::path::PathBuf::from("definitely-not-a-real-binary-xyz"),
        );

        let err = runner.run(GcloudCommand::bq(["ls"])).unwrap_err();

        match err {
            CommandError::MissingExecutable { program, .. } => {
                assert_eq!(program, CommandProgram::Bq);
            }
            other => panic!("{other:?}"),
        }
        assert!(
            runner
                .run(GcloudCommand::bq(["ls"]))
                .unwrap_err()
                .to_string()
                .starts_with("bq CLI not found"),
            "message should name bq"
        );
    }

    /// The same claim on the other spawn path. `run_with_cancellation` is the
    /// path every resource load takes, so a wrong "install gcloud" here is the
    /// one that would actually reach a user — and no mutation operator
    /// substitutes an enum value into an argument position, so only a test
    /// that drives this path pins it.
    #[test]
    fn missing_bq_binary_reports_bq_with_cancellation() {
        let runner = ProcessCommandRunner::with_binaries(
            std::path::PathBuf::from("/bin/sh"),
            std::path::PathBuf::from("definitely-not-a-real-binary-xyz"),
        );

        let err = runner
            .run_with_cancellation(GcloudCommand::bq(["ls"]), &CancellationToken::new())
            .unwrap_err();

        match err {
            CommandError::MissingExecutable { program, .. } => {
                assert_eq!(program, CommandProgram::Bq);
            }
            other => panic!("{other:?}"),
        }
        assert!(
            runner
                .run_with_cancellation(GcloudCommand::bq(["ls"]), &CancellationToken::new())
                .unwrap_err()
                .to_string()
                .starts_with("bq CLI not found"),
            "message should name bq"
        );
    }

    #[test]
    fn pipe_reader_timeout_returns_without_waiting_for_a_reader() {
        let (_sender, receiver) = mpsc::channel();

        let result = receive_pipe_result(receiver, "stdout", Duration::from_millis(20));

        assert!(matches!(
            result,
            Err(CommandError::Io(message)) if message == "gcloud stdout reader timed out"
        ));
    }

    #[test]
    fn cleanup_pipe_readers_bounds_both_reader_waits() {
        let (_stdout_sender, stdout_receiver) = mpsc::channel();
        let (_stderr_sender, stderr_receiver) = mpsc::channel();
        let (done_sender, done_receiver) = mpsc::channel();
        let worker = thread::spawn(move || {
            cleanup_pipe_readers(stdout_receiver, stderr_receiver, Duration::from_millis(20));
            let _ = done_sender.send(());
        });

        // Neither sender ever sends, so cleanup returns only because both reader
        // waits are bounded. Gating on the worker rather than on elapsed time
        // means an unbounded wait fails here while a scheduling stall does not.
        done_receiver
            .recv_timeout(Duration::from_secs(30))
            .expect("cleanup_pipe_readers should not wait on a reader that never sends");
        worker.join().expect("worker should finish");
    }

    /// The defaults every existing call site relies on. `new` is used at
    /// every call site that says nothing about a program or an environment
    /// (71 of them predating BigQuery), and this pins what their silence
    /// means.
    #[test]
    fn new_defaults_to_gcloud_with_no_environment() {
        let command = GcloudCommand::new(["version"]);

        assert_eq!(command.program, CommandProgram::Gcloud);
        assert!(command.env.is_empty(), "{:?}", command.env);
        assert_eq!(command.args, vec!["version".to_string()]);
    }

    #[test]
    fn bq_selects_the_bq_program_with_no_environment() {
        let command = GcloudCommand::bq(["ls", "-d"]);

        assert_eq!(command.program, CommandProgram::Bq);
        assert!(command.env.is_empty(), "{:?}", command.env);
        assert_eq!(command.args, vec!["ls".to_string(), "-d".to_string()]);
    }

    #[test]
    fn with_env_appends_in_call_order_and_keeps_the_program() {
        let command = GcloudCommand::bq(["ls"])
            .with_env("CLOUDSDK_ACTIVE_CONFIG_NAME", "prod")
            .with_env("CLOUDSDK_CORE_PROJECT", "example");

        assert_eq!(command.program, CommandProgram::Bq);
        assert_eq!(
            command.env,
            vec![
                (
                    "CLOUDSDK_ACTIVE_CONFIG_NAME".to_string(),
                    "prod".to_string()
                ),
                ("CLOUDSDK_CORE_PROJECT".to_string(), "example".to_string()),
            ]
        );
    }

    #[test]
    fn binary_name_reports_the_program_each_variant_spawns() {
        assert_eq!(CommandProgram::Gcloud.binary_name(), "gcloud");
        assert_eq!(CommandProgram::Bq.binary_name(), "bq");
    }

    /// Two stand-ins with distinguishable output, so a test can tell which
    /// binary the runner actually spawned rather than only that *a* binary ran.
    #[cfg(unix)]
    fn named_echo_binary(directory: &std::path::Path, name: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;

        let path = directory.join(name);
        std::fs::write(&path, format!("#!/bin/sh\nprintf 'ran {name}'\n"))
            .expect("write named echo executable");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
            .expect("mark named echo executable");
        path
    }

    #[cfg(unix)]
    #[test]
    fn run_spawns_the_binary_the_program_selects() {
        let directory = tempfile::tempdir().expect("tempdir");
        let runner = ProcessCommandRunner::with_binaries(
            named_echo_binary(directory.path(), "fake-gcloud"),
            named_echo_binary(directory.path(), "fake-bq"),
        );

        let gcloud = runner
            .run(GcloudCommand::new(["version"]))
            .expect("gcloud command succeeds");
        let bq = runner
            .run(GcloudCommand::bq(["ls"]))
            .expect("bq command succeeds");

        assert_eq!(gcloud.stdout, "ran fake-gcloud");
        assert_eq!(bq.stdout, "ran fake-bq");
    }

    /// The cancellable path is the one every resource load uses. A program
    /// selector wired into `run` alone would pass the test above and reach no
    /// real caller.
    #[cfg(unix)]
    #[test]
    fn run_with_cancellation_spawns_the_binary_the_program_selects() {
        let directory = tempfile::tempdir().expect("tempdir");
        let runner = ProcessCommandRunner::with_binaries(
            named_echo_binary(directory.path(), "fake-gcloud"),
            named_echo_binary(directory.path(), "fake-bq"),
        );

        let gcloud = runner
            .run_with_cancellation(GcloudCommand::new(["version"]), &CancellationToken::new())
            .expect("gcloud command succeeds");
        let bq = runner
            .run_with_cancellation(GcloudCommand::bq(["ls"]), &CancellationToken::new())
            .expect("bq command succeeds");

        assert_eq!(gcloud.stdout, "ran fake-gcloud");
        assert_eq!(bq.stdout, "ran fake-bq");
    }

    #[test]
    fn new_defaults_the_bq_binary_to_the_path_lookup() {
        let runner = ProcessCommandRunner::new(PathBuf::from("/usr/bin/gcloud"));

        assert_eq!(
            runner.binary(CommandProgram::Gcloud),
            &PathBuf::from("/usr/bin/gcloud")
        );
        assert_eq!(runner.binary(CommandProgram::Bq), &PathBuf::from("bq"));
    }

    /// The bq path gets the same argv pin the gcloud path has carried since
    /// the parity work: every command preview in the repo sits above this boundary,
    /// so an argument injected here would leave every parity test green.
    #[cfg(unix)]
    #[test]
    fn run_with_cancellation_hands_the_bq_argv_to_the_process_verbatim() {
        let directory = tempfile::tempdir().expect("tempdir");
        let runner = ProcessCommandRunner::with_binaries(
            PathBuf::from("definitely-not-a-real-binary-xyz"),
            argv_echo_binary(directory.path()),
        );

        let output = runner
            .run_with_cancellation(GcloudCommand::bq(ARGV_ECHO_ARGS), &CancellationToken::new())
            .expect("command succeeds");

        assert_eq!(
            output.stdout,
            argv_echo_expected(),
            "run_with_cancellation altered the bq argv it was given"
        );
    }

    /// A stand-in that reports one environment variable, so a test can prove
    /// the entry reached the process rather than only that it was stored on
    /// the command.
    #[cfg(unix)]
    fn env_echo_binary(directory: &std::path::Path) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;

        let path = directory.join("env-echo");
        std::fs::write(
            &path,
            "#!/bin/sh\nprintf '[%s]' \"${NUVEM_TEST_ENV_PROBE:-unset}\"\n",
        )
        .expect("write env echo executable");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
            .expect("mark env echo executable");
        path
    }

    #[cfg(unix)]
    #[test]
    fn run_sets_the_command_environment_on_the_child() {
        let directory = tempfile::tempdir().expect("tempdir");
        let runner = ProcessCommandRunner::with_binaries(
            PathBuf::from("definitely-not-a-real-binary-xyz"),
            env_echo_binary(directory.path()),
        );

        let output = runner
            .run(GcloudCommand::bq(["ls"]).with_env("NUVEM_TEST_ENV_PROBE", "prod"))
            .expect("command succeeds");

        assert_eq!(output.stdout, "[prod]");
    }

    #[cfg(unix)]
    #[test]
    fn run_with_cancellation_sets_the_command_environment_on_the_child() {
        let directory = tempfile::tempdir().expect("tempdir");
        let runner = ProcessCommandRunner::with_binaries(
            PathBuf::from("definitely-not-a-real-binary-xyz"),
            env_echo_binary(directory.path()),
        );

        let output = runner
            .run_with_cancellation(
                GcloudCommand::bq(["ls"]).with_env("NUVEM_TEST_ENV_PROBE", "prod"),
                &CancellationToken::new(),
            )
            .expect("command succeeds");

        assert_eq!(output.stdout, "[prod]");
    }

    /// The negative half. Without it, a runner that unconditionally set the
    /// variable to `prod` would pass both tests above.
    #[cfg(unix)]
    #[test]
    fn a_command_with_no_environment_leaves_the_child_environment_alone() {
        let directory = tempfile::tempdir().expect("tempdir");
        let runner = ProcessCommandRunner::with_binaries(
            PathBuf::from("definitely-not-a-real-binary-xyz"),
            env_echo_binary(directory.path()),
        );

        let output = runner
            .run(GcloudCommand::bq(["ls"]))
            .expect("command succeeds");

        assert_eq!(output.stdout, "[unset]");
    }

    /// The same negative half for the cancellable path, which is the one
    /// every resource load takes. Without it, a runner that set the variable
    /// unconditionally here alone would pass every other test in the module.
    #[cfg(unix)]
    #[test]
    fn a_cancellable_command_with_no_environment_leaves_the_child_environment_alone() {
        let directory = tempfile::tempdir().expect("tempdir");
        let runner = ProcessCommandRunner::with_binaries(
            PathBuf::from("definitely-not-a-real-binary-xyz"),
            env_echo_binary(directory.path()),
        );

        let output = runner
            .run_with_cancellation(GcloudCommand::bq(["ls"]), &CancellationToken::new())
            .expect("command succeeds");

        assert_eq!(output.stdout, "[unset]");
    }

    /// The two ceilings are different numbers, and the lifecycle one is far
    /// above the read one.
    ///
    /// Asserted as a relation rather than as a literal: the point is not that
    /// the lifecycle ceiling is thirty minutes, it is that it is nowhere near a
    /// number a real Cloud SQL failover or a large VM stop can reach, which is
    /// the property tuning either constant must not quietly lose. A lifecycle
    /// ceiling that drifted down to the read ceiling would restore the original
    /// defect exactly.
    #[test]
    fn the_lifecycle_ceiling_is_far_above_the_read_ceiling() {
        assert!(
            LIFECYCLE_TIMEOUT > RUN_TIMEOUT,
            "a lifecycle mutation must not be held to the ceiling written for a read"
        );
        assert!(
            LIFECYCLE_TIMEOUT >= Duration::from_secs(15 * 60),
            "the slowest lifecycle operations run for many minutes; {LIFECYCLE_TIMEOUT:?} is not a ceiling on a hang"
        );
    }

    /// Which ceiling each kind of command is held to, and — the half a mutant
    /// can quietly delete — that the lifecycle ceiling ignores the runner's.
    ///
    /// The runner's ceiling is the read ceiling, movable per runner so a test
    /// need not sit out two minutes. A lifecycle command is not held to it at
    /// any value, which is what stops a short-ceiling runner from killing an
    /// operation mid-flight.
    #[test]
    fn only_a_read_is_held_to_the_runner_s_own_ceiling() {
        let runner_ceiling = Duration::from_millis(5);

        assert_eq!(
            CommandCeiling::Read.duration(runner_ceiling),
            runner_ceiling
        );
        assert_eq!(
            CommandCeiling::Lifecycle.duration(runner_ceiling),
            LIFECYCLE_TIMEOUT
        );
    }

    /// ARCHITECTURE.md must not claim an ended call keeps what the child wrote.
    ///
    /// Both ended-call paths go through `cleanup_pipe_readers`, which drops the
    /// drained bytes with `let _ =`, and neither `Cancelled` nor `TimedOut` has
    /// a field that could carry them anywhere. The sentence claiming otherwise
    /// described a behaviour no code in this file has ever had, and
    /// the comment above `owner_done` says the opposite three screens up.
    #[test]
    fn architecture_does_not_claim_an_ended_call_keeps_the_child_s_output() {
        const ARCHITECTURE: &str = include_str!("../../../ARCHITECTURE.md");

        assert!(
            !ARCHITECTURE.contains("keeps whatever the child had already written"),
            "an ended call discards the drained output; no `CommandError` variant can carry it"
        );
        assert!(
            ARCHITECTURE.contains("drains and discards"),
            "ARCHITECTURE.md no longer says what an ended call does with the child's output"
        );
    }

    /// The ceiling both documents put on a `run` call, compared with
    /// [`RUN_TIMEOUT`].
    ///
    /// A unit test inside the module rather than one under `tests/`, for the
    /// reason the fan-out width's pin in `src/services/runtime/session.rs`
    /// gives: `RUN_TIMEOUT` is a private tuning number and an integration test
    /// is a separate crate, so binding it there would mean making it `pub` to
    /// satisfy a documentation test.
    ///
    /// Both documents spell the ceiling as words (`two minute ceiling`), so a
    /// bare search for `"120"` would find nothing and a search for `"two"`
    /// would match a dozen unrelated sentences. The word is read out of the
    /// claim's own idiom and compared with the constant spelled out, and every
    /// occurrence is compared rather than the first — the same shape, and the
    /// same limits, as the two multi-site pins the BigQuery slice left: no count
    /// is asserted, so this reads as "no documented ceiling disagrees with the
    /// constant", not as "the ceiling is still written down in both".
    #[test]
    fn the_docs_document_the_real_run_timeout() {
        const FEATURES: &str = include_str!("../../../docs/features.md");
        const ARCHITECTURE: &str = include_str!("../../../ARCHITECTURE.md");
        /// The number words the prose could plausibly be spelling, indexed by
        /// the value they spell: `MINUTE_WORDS[2]` is `"two"`.
        const MINUTE_WORDS: [&str; 11] = [
            "zero", "one", "two", "three", "four", "five", "six", "seven", "eight", "nine", "ten",
        ];
        const IDIOM: &str = " minute ceiling";

        let minutes = RUN_TIMEOUT.as_secs() / 60;
        assert_eq!(
            RUN_TIMEOUT.as_secs() % 60,
            0,
            "the prose states the ceiling in whole minutes; a RUN_TIMEOUT that is not a \
             whole number of them needs the sentences rewritten, not this test relaxed"
        );
        let expected = *MINUTE_WORDS
            .get(usize::try_from(minutes).expect("the ceiling fits a usize"))
            .unwrap_or_else(|| {
                panic!(
                    "the prose spells the ceiling as a word; teach this test how to spell \
                     {minutes} before setting RUN_TIMEOUT to it"
                )
            });

        for (name, document) in [
            ("docs/features.md", FEATURES),
            ("ARCHITECTURE.md", ARCHITECTURE),
        ] {
            let mut documented: Vec<String> = Vec::new();
            let mut rest = document;
            while let Some(at) = rest.find(IDIOM) {
                let word = rest[..at]
                    .split_whitespace()
                    .next_back()
                    .unwrap_or_default()
                    .to_ascii_lowercase();
                documented.push(word);
                rest = &rest[at + IDIOM.len()..];
            }

            assert!(
                !documented.is_empty(),
                "{name} no longer states the ceiling a `run` call is held to, which is the \
                 only thing bounding what one hung call costs a fan-out"
            );
            for (index, word) in documented.iter().enumerate() {
                assert_eq!(
                    word,
                    expected,
                    "{name}: ceiling claim #{} of {} documents a timeout RUN_TIMEOUT does \
                     not keep",
                    index + 1,
                    documented.len()
                );
            }
        }
    }
}
