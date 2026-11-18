//! Evidence: CI reliability sweep — the two properties the sweep bought.
//!
//! 1. `ProcessCommandRunner::run_with_cancellation` signals the child's process
//!    group *before* reaping the child, so the group kill can never stray onto a
//!    process group the kernel has recycled onto the same id.
//! 2. Each log-dump run writes under its own home, so `dumps.len() == 1` cannot
//!    be perturbed by a second cargo test process running at the same time.
//!
//! What this file does and does not pin, for property 1: the ordering inside
//! `terminate_and_reap` is pinned by
//! `terminate_and_reap_signals_the_group_while_the_pid_is_still_reserved` in
//! src/services/gcloud/cli.rs, which is where those private functions can be
//! called directly. Here `measure` reproduces both orders at the syscall level to
//! ILLUSTRATE why the order matters — it does not call the runner's internals and
//! would not notice them being reordered. What this file does drive end to end is
//! `run_with_cancellation` itself: `run_production_path` leaves a descendant in
//! the child's group and requires the runner to have killed it.
//!
//! Assertions always run. The transcript is written only when NUVEM_EVIDENCE_DIR is
//! set, and only under that directory.

use nuvem::Config;
use nuvem::app::{Action, AppState, Effect};
use nuvem::services::gcloud::cli::{
    CommandError, CommandOutput, CommandRunner, GcloudCommand, ProcessCommandRunner,
};
use nuvem::services::runtime::session::RuntimeSession;
use std::path::PathBuf;

#[cfg(unix)]
use std::os::unix::process::CommandExt;
#[cfg(unix)]
use std::process::{Child, Command, Stdio};
#[cfg(unix)]
use std::time::Duration;
#[cfg(unix)]
use tokio_util::sync::CancellationToken;

/// `has_unreaped_child` and `wait_until` are shared with
/// `tests/gcloud_runner_test.rs` rather than copied — see the module's own
/// comment for why that one helper in particular must not exist twice.
use crate::support::process;
#[cfg(unix)]
use process::{has_unreaped_child, wait_until};

// ---------------------------------------------------------------------------
// Property 1 — the pid stays reserved until the group has been signalled.
// ---------------------------------------------------------------------------

/// The two orders the runner could use. Production uses `SignalThenReap`; the
/// pre-sweep code reaped inside `try_wait` and so behaved as `ReapThenSignal`.
#[cfg(unix)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Order {
    SignalThenReap,
    ReapThenSignal,
}

/// Kills the child's group and reaps it however the test exits. Armed before the
/// first assertion that can panic so a failure never leaves a process behind.
#[cfg(unix)]
struct GroupGuard {
    pid: libc::pid_t,
    armed: bool,
}

#[cfg(unix)]
impl GroupGuard {
    fn arm(pid: libc::pid_t) -> Self {
        Self { pid, armed: true }
    }

    fn disarm(&mut self) {
        self.armed = false;
    }
}

#[cfg(unix)]
impl Drop for GroupGuard {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        // SAFETY: neither call takes a pointer except `&mut status`, which is a
        // live `c_int` on this frame. The group kill's target is the burden, and
        // the guard is armed only after the test has proven `self.pid` leads its
        // own group and that the group is not the harness's; the reap below is
        // the first thing to release the id, so at the moment of the signal it
        // is still ours and cannot have been handed to an unrelated group.
        unsafe {
            libc::kill(-self.pid, libc::SIGKILL);
            let mut status = 0;
            libc::waitpid(self.pid, &mut status, 0);
        }
    }
}

/// Kills one specific pid however the test exits. Used where the group is not
/// yet proven to be ours: a stray descendant still must not outlive a failure.
#[cfg(unix)]
struct PidGuard {
    pid: libc::pid_t,
    armed: bool,
}

#[cfg(unix)]
impl PidGuard {
    fn arm(pid: libc::pid_t) -> Self {
        Self { pid, armed: true }
    }

    fn disarm(&mut self) {
        self.armed = false;
    }
}

#[cfg(unix)]
impl Drop for PidGuard {
    fn drop(&mut self) {
        if self.armed {
            // SAFETY: `kill` takes no pointers, so this is sound as an FFI call
            // for any pid. The single pid — not a negated group — is the point:
            // this guard is armed where the group is not yet proven ours, so it
            // can only ever reach the one descendant the test recorded.
            unsafe { libc::kill(self.pid, libc::SIGKILL) };
        }
    }
}

/// Peek at the child without reaping it — what `child_has_exited` does.
#[cfg(unix)]
fn peek_exited(pid: libc::pid_t) -> bool {
    // SAFETY: `siginfo_t` is a plain C struct of integers, pointers and padding
    // with no niche, so all-zero is a valid value. The zeroing also sets the
    // `si_signo` this function reads to 0, the "nothing reported" sentinel it
    // has to distinguish from a real exit.
    let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
    // SAFETY: `info` is a live, aligned `siginfo_t` owned by this frame for the
    // whole call, and `pid` names a child of this test process that nothing has
    // reaped — `WNOWAIT` is what keeps it that way — so the `P_PID` target is a
    // pid still reserved to us rather than a recycled one.
    let result = unsafe {
        libc::waitid(
            libc::P_PID,
            pid as libc::id_t,
            &mut info,
            libc::WEXITED | libc::WNOWAIT | libc::WNOHANG,
        )
    };
    result == 0 && info.si_signo != 0
}

/// `None` on success, otherwise the errno the group signal reported.
///
/// The two legs deliberately pass different signals and must not be folded
/// back together. The production leg sends SIGKILL because its job — like
/// `terminate_process_tree`'s — is to actually clear a group whose id is still
/// ours. The control has already released the id by reaping, so a live SIGKILL
/// there would itself be the stray-kill defect this branch fixes, fired for
/// real on every test run; it probes with signal 0 instead. The kernel runs
/// the same target lookup either way — measured on this host: EPERM from both
/// signals while the zombie-only group exists, ESRCH from both once the reap
/// releases the id — so the errno discrimination is unchanged, but nothing can
/// be delivered to whatever group the kernel may since have given the id to.
#[cfg(unix)]
fn signal_group(pgid: libc::pid_t, signal: libc::c_int) -> Option<i32> {
    // SAFETY: `kill` takes no pointers; the target is what needs justifying.
    // Both call sites assert `pgid` equals the child's pid and differs from
    // `getpgrp()` before calling, and the child is left un-reaped, so the id is
    // still reserved to us — the negation cannot address a recycled group, and
    // in particular not the harness's own.
    if unsafe { libc::kill(-pgid, signal) } == 0 {
        None
    } else {
        std::io::Error::last_os_error().raw_os_error()
    }
}

/// The pids the kernel still counts as members of `pgid`. Asked of `ps` rather
/// than assumed: an assertion about an *empty* group is worthless if the group
/// turns out to hold a live descendant, which passes both orders for the wrong
/// reason.
#[cfg(unix)]
fn group_members(pgid: libc::pid_t) -> Vec<libc::pid_t> {
    // Listed and filtered here rather than asked of `ps -g`, whose argument is a
    // session on Linux and a process group on macOS.
    let output = Command::new("ps")
        .args(["-eo", "pid=,pgid="])
        .output()
        .expect("ps must be available to inspect process groups");
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let pid: libc::pid_t = fields.next()?.parse().ok()?;
            let group: libc::pid_t = fields.next()?.parse().ok()?;
            (group == pgid).then_some(pid)
        })
        .collect()
}

#[cfg(unix)]
#[derive(Debug)]
struct Measurement {
    pid: libc::pid_t,
    pgid: libc::pid_t,
    strays: Vec<libc::pid_t>,
    reserved_at_exit: bool,
    reaped_ok: bool,
    owned_at_signal: bool,
    signal_errno: Option<i32>,
    unreaped_after: bool,
}

#[cfg(unix)]
impl Measurement {
    fn errno_label(&self) -> String {
        match self.signal_errno {
            None => "0 (accepted: a signalable group with this id exists)".to_string(),
            Some(libc::ESRCH) => "ESRCH (no such process group)".to_string(),
            Some(libc::EPERM) => {
                "EPERM (group exists, zombie-only: nothing signalable)".to_string()
            }
            Some(other) => format!("errno {other}"),
        }
    }
}

/// Run one ordinary, successful invocation to completion under `order` and
/// record what the group signal saw.
#[cfg(unix)]
fn measure(order: Order) -> Measurement {
    // `process_group(0)` is what makes the pgid equal the pid; without it the
    // group under test is the test harness's own and nothing here means anything.
    // The child reports its own pgid because `getpgid` fails once it is a zombie,
    // and then exits leaving no descendants — the ordinary successful invocation.
    let directory = tempfile::tempdir().expect("temp dir");
    let pgid_file = directory.path().join("pgid");
    let mut child: Child = Command::new("/bin/sh")
        .args([
            "-c",
            format!("ps -o pgid= -p $$ > {}", pgid_file.display()).as_str(),
        ])
        .process_group(0)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn /bin/sh");
    let pid = child.id() as libc::pid_t;
    let mut guard = GroupGuard::arm(pid);

    let exited = wait_until(Duration::from_secs(10), || peek_exited(pid));
    assert!(
        exited,
        "the non-reaping peek never reported pid {pid} exiting"
    );

    let pgid: libc::pid_t = std::fs::read_to_string(&pgid_file)
        .expect("read the pgid the child reported")
        .trim()
        .parse()
        .expect("parse pgid");
    // Ownership is proven HERE, before anything is signalled. A group kill aimed
    // at a group we do not own takes out this test process, cargo, and every
    // sibling test binary, with no assertion message — so these checks are
    // useless anywhere below `signal_group`.
    // SAFETY: `getpgrp` takes no arguments, touches no memory and cannot fail;
    // it is unsafe only because it is an FFI call.
    let own_group = unsafe { libc::getpgrp() };
    assert_ne!(
        pgid, own_group,
        "child pid {pid} is in the test harness's own process group: signalling it would kill this run and its siblings"
    );
    assert_eq!(
        pgid, pid,
        "child pid {pid} does not lead its own process group, so the group about to be signalled is not ours"
    );
    let members = group_members(pgid);
    assert!(
        members.contains(&pid),
        "ps does not list pid {pid} in group {pgid} ({members:?}): either the peek reaped it and the id is already released, or the membership listing is broken — an empty group here would prove nothing either way"
    );
    let strays: Vec<_> = members
        .into_iter()
        .filter(|member| *member != pid)
        .collect();
    // SAFETY: `kill` takes no pointers, and signal 0 only asks whether the pid
    // exists and may be signalled — nothing is delivered, so this cannot disturb
    // any process even if the pid had been recycled.
    let reserved_at_exit = unsafe { libc::kill(pid, 0) } == 0;

    // `owned_at_signal` is the deterministic half of this measurement: an
    // un-reaped child is one we provably still own, so the pgid cannot have been
    // handed to anyone else. `peek_exited` asks without reaping — `waitpid` would
    // reap the zombie and destroy the very property under measurement.
    //
    // The reap is recorded rather than unwrapped so a peek that reaped by
    // mistake fails on the reservation assertion, which names that, instead of
    // on an ECHILD nobody can read.
    let (signal_errno, reaped_ok, owned_at_signal) = match order {
        Order::SignalThenReap => {
            let owned = peek_exited(pid);
            let errno = signal_group(pgid, libc::SIGKILL);
            (errno, child.wait().is_ok(), owned)
        }
        Order::ReapThenSignal => {
            let reaped_ok = child.wait().is_ok();
            let owned = peek_exited(pid);
            (signal_group(pgid, 0), reaped_ok, owned)
        }
    };
    guard.disarm();

    Measurement {
        pid,
        pgid,
        strays,
        reserved_at_exit,
        reaped_ok,
        owned_at_signal,
        signal_errno,
        unreaped_after: has_unreaped_child(pid),
    }
}

#[cfg(unix)]
#[derive(Debug)]
struct ProductionRun {
    pid: libc::pid_t,
    pgid: libc::pid_t,
    descendant: libc::pid_t,
}

/// One real `run_with_cancellation`, driven end to end over a command that
/// outlives itself: the child backgrounds a long `sleep` into its own group and
/// exits. The runner must have taken the whole group down and reaped the child it
/// peeked at — `WNOWAIT` defers the reap, it does not cancel it.
///
/// The descendant redirects its stdio to /dev/null so the runner's pipe readers
/// still see EOF when the child exits; a descendant holding the pipe open would
/// fail the call on the read timeout instead, for an unrelated reason.
#[cfg(unix)]
fn run_production_path() -> ProductionRun {
    let directory = tempfile::tempdir().expect("temp dir");
    let identity_file = directory.path().join("identity");
    let runner = ProcessCommandRunner::new(PathBuf::from("/bin/sh"));
    let script = format!(
        "sleep 300 </dev/null >/dev/null 2>&1 & echo $$ > {file}; \
         ps -o pgid= -p $$ >> {file}; echo $! >> {file}",
        file = identity_file.display()
    );

    runner
        .run_with_cancellation(
            GcloudCommand::new(["-c", script.as_str()]),
            &CancellationToken::new(),
        )
        .expect("command succeeds");

    let identity = std::fs::read_to_string(&identity_file).expect("read identity file");
    let mut fields = identity
        .split_whitespace()
        .map(|field| field.parse().expect("parse pid"));
    let run = ProductionRun {
        pid: fields.next().expect("child pid"),
        pgid: fields.next().expect("child pgid"),
        descendant: fields.next().expect("descendant pid"),
    };

    // Armed before the ownership assertions, which run before anything may signal
    // a *group*: if they fire, the descendant still must not survive them.
    let mut descendant_guard = PidGuard::arm(run.descendant);

    // Same rule as in `measure`: prove the group is ours before the cleanup guard
    // can ever signal it. While the descendant lives it holds the pgid reserved,
    // so the id cannot have been handed to anybody else.
    assert_ne!(
        run.pgid,
        // SAFETY: `getpgrp` takes no arguments, touches no memory and cannot
        // fail; it is unsafe only because it is an FFI call.
        unsafe { libc::getpgrp() },
        "run_with_cancellation left the child in the test harness's own process group"
    );
    assert_eq!(
        run.pgid, run.pid,
        "run_with_cancellation must spawn the child as its own group leader, or `kill(-pid)` targets a group it does not own"
    );

    let mut guard = GroupGuard::arm(run.pgid);
    let cleared = wait_until(Duration::from_secs(10), || {
        group_members(run.pgid).is_empty()
    });
    if !cleared {
        // Named rather than guessed: an un-reaped child is a member of the group
        // too, so "not empty" has two possible causes and the reader needs to
        // know which pids are still there.
        let leftovers = group_members(run.pgid);
        panic!(
            "run_with_cancellation returned but group {} still holds {:?} (child {}, descendant {}): the group signal never reached the descendant, or the child was never reaped",
            run.pgid, leftovers, run.pid, run.descendant
        );
    }
    guard.disarm();
    descendant_guard.disarm();

    run
}

#[cfg(unix)]
#[test]
fn group_signal_precedes_the_reap_so_the_pgid_is_never_recycled() {
    let production = measure(Order::SignalThenReap);
    let pre_sweep = measure(Order::ReapThenSignal);
    let run = run_production_path();

    assert!(
        production.strays.is_empty(),
        "the group must hold nothing but the leader, or the signal would find a target either way: {:?}",
        production.strays
    );
    assert!(
        production.reserved_at_exit,
        "pid {} was already released at the moment its exit was observed",
        production.pid
    );
    assert!(
        production.owned_at_signal,
        "pid {} had already been reaped when the group was signalled: the id was no longer provably ours",
        production.pid
    );
    assert_ne!(
        production.signal_errno,
        Some(libc::ESRCH),
        "the process group of pid {} was gone when we signalled it: the child had been reaped first and the pgid was free for the kernel to hand out again",
        production.pid
    );
    assert!(
        production.reaped_ok,
        "the deliberate reap of pid {} failed",
        production.pid
    );
    assert!(
        !production.unreaped_after,
        "pid {} is still waitable: the deferred reap never happened",
        production.pid
    );

    assert!(
        pre_sweep.strays.is_empty(),
        "the control's group must also hold nothing but the leader: {:?}",
        pre_sweep.strays
    );
    // Deterministic, unlike an assertion on ESRCH: whether the released id has
    // been taken by somebody else yet is a race, but whether we still own the
    // child is a fact. The errno the signal actually got is recorded in the
    // transcript as an observation.
    assert!(
        !pre_sweep.owned_at_signal,
        "reaping pid {} first did NOT give up ownership of the id, so this measurement cannot tell the two orders apart and proves nothing",
        pre_sweep.pid
    );

    assert!(
        !has_unreaped_child(run.pid),
        "run_with_cancellation left pid {} un-reaped after peeking at it with WNOWAIT",
        run.pid
    );

    let report = format!(
        "nuvem — signal the process group before reaping the child\n\
         \n\
         INVARIANT\n\
         `run_with_cancellation` spawns gcloud with `process_group(0)`, so the child's\n\
         process-group id IS its pid. On exit the runner kills that group with\n\
         `kill(-pid, SIGKILL)`. The kernel reserves the pid only while the child is\n\
         un-reaped, so the group kill must be issued BEFORE the reap. Reap first and, on\n\
         every invocation that leaves no descendants — the ordinary successful case, so\n\
         this path is hot — the group empties, the kernel takes the id back, and the\n\
         SIGKILL that follows can land on an unrelated process group that has since been\n\
         given the same id.\n\
         \n\
         CORRECTION — do not re-chase this\n\
         `kill(pid, 0)` returning ESRCH after the reap does NOT show the pgid is gone,\n\
         and a working probe built on it already misled one careful investigation. The\n\
         kernel keeps a pid reserved while it is the pgid of a group that still has\n\
         members, so reaping the leader releases nothing while a descendant lives. The\n\
         release is observable only when the group is EMPTY, and then it is observable\n\
         directly, in the return value of `kill(-pgid, SIGKILL)`. Assert on the absence\n\
         of ESRCH rather than on a specific success value: signalling a zombie-only group\n\
         is EPERM on macOS and succeeds on Linux, but neither reports a group that is not\n\
         there.\n\
         \n\
         ILLUSTRATION, NOT THE PIN — read this before quoting the numbers below\n\
         The rows that follow are the two orders reproduced at the syscall level, to show\n\
         what the kernel does with the id. They do NOT call the runner's internals:\n\
         `terminate_and_reap` and `child_has_exited` are private to cli.rs and are pinned\n\
         where they can be called directly, by\n\
         `terminate_and_reap_signals_the_group_while_the_pid_is_still_reserved` in\n\
         src/services/gcloud/cli.rs. Reorder the runner and this section keeps passing;\n\
         that unit test is what fails. What this file drives end to end is\n\
         `run_with_cancellation` itself, in PRODUCTION PATH below.\n\
         \n\
         MEASUREMENT — one `/bin/sh` child per row, spawned with `process_group(0)` as\n\
         production spawns gcloud, run to exit leaving no descendants; then the group\n\
         signalled in the order shown. The production row sends SIGKILL, as production\n\
         does. The control probes with signal 0: its id is already released, so a live\n\
         SIGKILL there would itself be the stray kill under discussion, fired for real\n\
         once per test run — and the kernel's target lookup, which is what the errno\n\
         reports, is the same for both signals. The child reports its own pgid on the way out,\n\
         because `getpgid` fails once it is a zombie. That the group is ours — pgid == pid,\n\
         pgid is not the harness's own group, and `ps -eo pid=,pgid=` really does list the\n\
         child in it — is asserted BEFORE anything is signalled, because a group kill aimed\n\
         at the wrong group kills the test run with no message to read afterwards.\n\
         \n\
         The asserted fact per row is ownership at the moment of signalling: an un-reaped\n\
         child is one the kernel still holds the id for. The errno is recorded alongside\n\
         it as an observation, not asserted for the control — whether a released id has\n\
         already been handed to somebody else is a race, and this branch exists to reject\n\
         reasoning like that.\n\
         \n\
         [production] signal, then reap\n\
         \x20 pid {prod_pid}, pgid {prod_pgid} (== pid: the child leads its own group)\n\
         \x20 other members of the group at exit: {prod_strays:?}\n\
         \x20 pid reserved when the exit was observed: {prod_reserved}\n\
         \x20 still an un-reaped child of ours when signalled: {prod_owned}\n\
         \x20 observed: kill(-{prod_pgid}, SIGKILL) -> {prod_errno}\n\
         \x20 un-reaped child left behind afterwards: {prod_unreaped}\n\
         \n\
         [pre-sweep control] reap, then signal\n\
         \x20 pid {pre_pid}, pgid {pre_pgid}\n\
         \x20 other members of the group at exit: {pre_strays:?}\n\
         \x20 still an un-reaped child of ours when signalled: {pre_owned}\n\
         \x20 observed: kill(-{pre_pgid}, 0) -> {pre_errno}\n\
         \n\
         The control is the whole point: the same child, the same empty group, and the\n\
         probe goes out with the id no longer ours — back in the kernel's pool, free to\n\
         be handed to an unrelated process group. The old code sent SIGKILL from exactly\n\
         this position on every successful invocation.\n\
         \n\
         MECHANISM (as implemented in src/services/gcloud/cli.rs)\n\
         `child_has_exited` peeks with `waitid(P_PID, .., WEXITED|WNOWAIT|WNOHANG)`, which\n\
         reports the exit and leaves the zombie in place; `try_wait`, which it replaced,\n\
         reaped. `terminate_and_reap` then signals the group and only afterwards calls\n\
         `child.wait()`. `terminate_process_tree` returns the group kill's result instead\n\
         of discarding it, which is what lets that unit test assert on the ordering at all.\n\
         \n\
         PRODUCTION PATH — the part of this file that exercises the real runner\n\
         One `run_with_cancellation` over a command that backgrounds `sleep 300` into its\n\
         own group and exits: child pid {run_pid}, pgid {run_pgid} (it leads its own group),\n\
         descendant {run_descendant}. After the call returned, group {run_pgid} was empty —\n\
         the runner's group signal reached the descendant, not just the child — and\n\
         waitpid(WNOHANG) reports ECHILD for {run_pid}: deferring the reap does not skip it.\n",
        prod_pid = production.pid,
        prod_pgid = production.pgid,
        prod_strays = production.strays,
        prod_reserved = production.reserved_at_exit,
        prod_owned = production.owned_at_signal,
        prod_errno = production.errno_label(),
        prod_unreaped = production.unreaped_after,
        pre_pid = pre_sweep.pid,
        pre_pgid = pre_sweep.pgid,
        pre_strays = pre_sweep.strays,
        pre_owned = pre_sweep.owned_at_signal,
        pre_errno = pre_sweep.errno_label(),
        run_pid = run.pid,
        run_pgid = run.pgid,
        run_descendant = run.descendant,
    );

    println!("\n{report}");
    write_evidence("process_group_ordering.txt", &report);
}

// ---------------------------------------------------------------------------
// Property 2 — every run owns its dump directory.
// ---------------------------------------------------------------------------

/// The save path never shells out; the session still needs *a* runner.
struct UnusedRunner;
impl CommandRunner for UnusedRunner {
    fn run(&self, command: GcloudCommand) -> Result<CommandOutput, CommandError> {
        panic!("the save effect must not run gcloud, got {command:?}")
    }
}

async fn save_one_dump(home: &std::path::Path) -> Vec<PathBuf> {
    let config = Config {
        log_file: home.join("nuvem.log"),
        ..Config::default()
    };
    let mut session = RuntimeSession::new_with_config_and_runner_for_test(
        config,
        AppState::new(),
        Box::new(UnusedRunner),
    );

    session
        .dispatch_action(Action::Effect(Effect::SaveLogEntries {
            lines: vec!["t1 ERROR gce_instance disk full".to_string()],
        }))
        .await;

    let dump_directory = home.join("log-dumps");
    let mut dumps: Vec<_> = std::fs::read_dir(&dump_directory)
        .unwrap_or_else(|error| {
            panic!(
                "the run's dump directory {dump_directory:?} does not exist ({error}): the save path must write under the parent of its own config.log_file, or the count below is measured on somebody else's directory"
            )
        })
        .map(|entry| entry.expect("dir entry").path())
        .collect();
    dumps.sort();
    dumps
}

#[tokio::test]
async fn two_log_dump_runs_in_distinct_temp_dirs_each_see_exactly_one_dump() {
    // The production property the walkthroughs' per-test TempDir homes rely on:
    // a run's dump directory follows its own `config.log_file`, so its dump count
    // depends on nothing but its own config. Two TempDirs stand in for two cargo
    // test processes racing on one machine.
    let first_home = tempfile::tempdir().expect("temp dir");
    let second_home = tempfile::tempdir().expect("temp dir");

    let first = save_one_dump(first_home.path()).await;
    let second = save_one_dump(second_home.path()).await;

    assert_ne!(
        first_home.path(),
        second_home.path(),
        "the two runs must not share a scratch home"
    );
    assert_eq!(
        first.len(),
        1,
        "the first run must see only its own dump: {first:?}"
    );
    assert_eq!(
        second.len(),
        1,
        "the second run must see only its own dump: {second:?}"
    );

    let report = format!(
        "nuvem — every evidence run owns its dump directory\n\
         \n\
         INVARIANT\n\
         The evidence walkthroughs assert `dumps.len() == 1` over the log-dumps directory\n\
         of the scratch home they save into. That holds only if no other process can name\n\
         the same directory. They used to build the home as\n\
         `evidence_dir().unwrap_or_else(temp_dir).join(\"nuvem-home\")` — one fixed path per\n\
         machine — so two overlapping cargo test runs wrote two dumps into it and each\n\
         assertion saw 2. Each walkthrough now owns a `tempfile::TempDir` bound for the\n\
         whole test.\n\
         \n\
         MEASUREMENT — two independent runs of the SaveLogEntries path, one per TempDir:\n\
         \x20 run 1 home: {first_home}\n\
         \x20 run 1 sees: {first:?}\n\
         \x20 run 2 home: {second_home}\n\
         \x20 run 2 sees: {second:?}\n\
         \n\
         Neither listing can be perturbed by the other, so the count is a property of the\n\
         run rather than of what else the machine happens to be doing. Collected evidence\n\
         is unaffected: transcripts still go to NUVEM_EVIDENCE_DIR, which is nobody's scratch\n\
         home.\n",
        first_home = first_home.path().display(),
        second_home = second_home.path().display(),
        first = first,
        second = second,
    );

    println!("\n{report}");
    write_evidence("dump_directory_isolation.txt", &report);
}

/// Transcripts are written only when NUVEM_EVIDENCE_DIR is set, and only under it.
fn write_evidence(name: &str, report: &str) {
    if let Ok(dir) = std::env::var("NUVEM_EVIDENCE_DIR") {
        let path = std::path::Path::new(&dir).join(name);
        std::fs::write(&path, report).expect("write evidence file");
    }
}
