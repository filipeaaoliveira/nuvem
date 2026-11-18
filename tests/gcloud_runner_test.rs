use std::cell::RefCell;
#[cfg(unix)]
use std::fs;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
#[cfg(unix)]
use std::sync::mpsc;
#[cfg(unix)]
use std::thread;
#[cfg(unix)]
use std::time::Duration;

use nuvem::services::gcloud::cli::{
    CommandCeiling, CommandError, CommandOutput, CommandRunner, GcloudCommand, ProcessCommandRunner,
};
use nuvem::services::gcloud::configurations::{
    GcloudConfigurationDraft, GcloudConfigurationService, GcloudContextEditDraft,
};
use nuvem_domain::gcp::configuration::GcloudConfigurationName;
#[cfg(unix)]
use tokio_util::sync::CancellationToken;

/// `has_unreaped_child` and `wait_until` are shared with
/// `tests/ci_reliability_evidence.rs` rather than copied — see the
/// module's own comment for why that one helper in particular must not exist
/// twice.
use crate::support::process;
#[cfg(unix)]
use process::{has_unreaped_child, wait_until};

#[derive(Default)]
struct FakeRunner {
    outputs: RefCell<Vec<Result<CommandOutput, CommandError>>>,
    commands: RefCell<Vec<GcloudCommand>>,
}

impl CommandRunner for FakeRunner {
    fn run(
        &self,
        command: GcloudCommand,
    ) -> Result<CommandOutput, nuvem::services::gcloud::cli::CommandError> {
        self.commands.borrow_mut().push(command);
        self.outputs.borrow_mut().remove(0)
    }
}

fn output() -> Result<CommandOutput, CommandError> {
    Ok(CommandOutput {
        stdout: String::new(),
        stderr: String::new(),
    })
}

fn args(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| value.to_string()).collect()
}

#[test]
fn load_configurations_runs_native_list_command() {
    let runner = FakeRunner::default();
    runner.outputs.borrow_mut().push(Ok(CommandOutput {
        stdout: include_str!("fixtures/gcloud_configurations.json").to_string(),
        stderr: String::new(),
    }));
    let service = GcloudConfigurationService::new(runner);

    let configs = service.load_configurations().expect("load should succeed");

    assert_eq!(configs.len(), 2);
    let commands = service.runner().commands.borrow();
    assert_eq!(
        commands[0].args,
        args(&["config", "configurations", "list", "--format=json"])
    );
}

#[test]
fn activate_configuration_runs_native_activate_command() {
    let runner = FakeRunner::default();
    runner.outputs.borrow_mut().push(output());
    let service = GcloudConfigurationService::new(runner);

    service
        .activate_configuration(&GcloudConfigurationName::new("prod"))
        .expect("activation should succeed");

    let commands = service.runner().commands.borrow();
    assert_eq!(
        commands[0].args,
        args(&["config", "configurations", "activate", "prod"])
    );
}

#[test]
fn create_configuration_sets_supplied_properties_with_configuration_flag() {
    let runner = FakeRunner::default();
    for _ in 0..4 {
        runner.outputs.borrow_mut().push(output());
    }
    let service = GcloudConfigurationService::new(runner);

    service
        .create_configuration(GcloudConfigurationDraft {
            name: "stage".to_string(),
            account: Some("stage@example.com".to_string()),
            project: Some("stage-project".to_string()),
            region: Some("europe-west1".to_string()),
            zone: None,
        })
        .expect("create should succeed");

    let commands = service.runner().commands.borrow();
    assert_eq!(
        commands[0].args,
        args(&["config", "configurations", "create", "stage"])
    );
    assert_eq!(
        commands[1].args,
        args(&[
            "--configuration",
            "stage",
            "config",
            "set",
            "account",
            "stage@example.com",
        ])
    );
    assert_eq!(
        commands[2].args,
        args(&[
            "--configuration",
            "stage",
            "config",
            "set",
            "project",
            "stage-project",
        ])
    );
    assert_eq!(
        commands[3].args,
        args(&[
            "--configuration",
            "stage",
            "config",
            "set",
            "compute/region",
            "europe-west1",
        ])
    );
}

#[test]
fn update_context_sets_supplied_properties_with_configuration_flag() {
    let runner = FakeRunner::default();
    for _ in 0..3 {
        runner.outputs.borrow_mut().push(output());
    }
    let service = GcloudConfigurationService::new(runner);

    service
        .update_context(GcloudContextEditDraft {
            configuration: GcloudConfigurationName::new("dev"),
            project: Some("dev-project".to_string()),
            region: Some("europe-west1".to_string()),
            zone: Some("europe-west1-b".to_string()),
        })
        .expect("update should succeed");

    let commands = service.runner().commands.borrow();
    assert_eq!(
        commands[0].args,
        args(&[
            "--configuration",
            "dev",
            "config",
            "set",
            "project",
            "dev-project",
        ])
    );
    assert_eq!(
        commands[1].args,
        args(&[
            "--configuration",
            "dev",
            "config",
            "set",
            "compute/region",
            "europe-west1",
        ])
    );
    assert_eq!(
        commands[2].args,
        args(&[
            "--configuration",
            "dev",
            "config",
            "set",
            "compute/zone",
            "europe-west1-b",
        ])
    );
}

#[test]
fn update_context_unsets_blank_properties_with_configuration_flag() {
    let runner = FakeRunner::default();
    for _ in 0..3 {
        runner.outputs.borrow_mut().push(output());
    }
    let service = GcloudConfigurationService::new(runner);

    service
        .update_context(GcloudContextEditDraft {
            configuration: GcloudConfigurationName::new("dev"),
            project: None,
            region: None,
            zone: None,
        })
        .expect("update should succeed");

    let commands = service.runner().commands.borrow();
    assert_eq!(
        commands[0].args,
        args(&["--configuration", "dev", "config", "unset", "project"])
    );
    assert_eq!(
        commands[1].args,
        args(&[
            "--configuration",
            "dev",
            "config",
            "unset",
            "compute/region",
        ])
    );
    assert_eq!(
        commands[2].args,
        args(&["--configuration", "dev", "config", "unset", "compute/zone",])
    );
}

/// Every process these tests background sleeps for 300s, so a gate an order of
/// magnitude below that can only be tripped by a genuine hang, never by
/// scheduling jitter on a loaded runner.
#[cfg(unix)]
const CANCELLATION_GATE: Duration = Duration::from_secs(30);

/// The runner issues its group SIGKILL synchronously, before returning, so a
/// descendant is already dying by the time a test looks at it. This absorbs the
/// kernel's delivery latency and nothing else — spending the full cancellation
/// gate here would just be 30s of confirming a descendant that is never leaving.
#[cfg(unix)]
const DESCENDANT_EXIT_DEADLINE: Duration = Duration::from_secs(2);

/// SIGKILLs the pid it holds when it goes out of scope, unwinding included.
///
/// Every test here backgrounds a `sleep 300`. When a gate trips, the test thread
/// panics while the worker is still parked inside `run_with_cancellation`, so
/// nothing else will ever reap that sleep — and for the `setsid` descendant it is
/// outside the runner's process group, so not even a working group kill reaches
/// it. Without this guard a single failure leaves a process on the box for five
/// minutes, on exactly the run where CI is already unhealthy.
///
/// A test whose runner has already killed the pid disarms the guard via
/// `disarm_once_dead`, so a green run never signals a released pid the kernel
/// may since have handed out again — the same rule the process-group guards in
/// `tests/ci_reliability_evidence.rs` follow. The guard stays armed on every
/// panic path, and on any path where the pid is still alive.
#[cfg(unix)]
struct KillOnDrop {
    pid: libc::pid_t,
    armed: bool,
}

#[cfg(unix)]
impl KillOnDrop {
    fn arm(pid: libc::pid_t) -> Self {
        Self { pid, armed: true }
    }

    fn disarm(&mut self) {
        self.armed = false;
    }

    /// Disarm only once the pid is observed dead, within the same deadline that
    /// bounds the runner's own kill. A pid still alive at the deadline keeps
    /// the guard armed: a broken runner must not turn the disarm into a leak.
    fn disarm_once_dead(&mut self) {
        if wait_until(DESCENDANT_EXIT_DEADLINE, || !pid_is_alive(self.pid)) {
            self.disarm();
        }
    }
}

#[cfg(unix)]
impl Drop for KillOnDrop {
    fn drop(&mut self) {
        if self.armed {
            // SAFETY: `kill` takes no pointers, so this is sound as an FFI call
            // for any pid. It signals the single recorded pid rather than a
            // negated group, so the worst a stale id can do is reach one
            // unrelated process — never the harness's whole group.
            unsafe {
                let _ = libc::kill(self.pid, libc::SIGKILL);
            }
        }
    }
}

#[cfg(unix)]
fn pid_is_alive(pid: libc::pid_t) -> bool {
    // SAFETY: `kill` takes no pointers, and signal 0 only asks whether the pid
    // exists and may be signalled — nothing is delivered, so the probe cannot
    // disturb any process.
    unsafe { libc::kill(pid, 0) == 0 }
}

#[cfg(unix)]
fn parsed_pid(path: &std::path::Path) -> Option<libc::pid_t> {
    fs::read_to_string(path).ok()?.trim().parse().ok()
}

#[cfg(unix)]
fn read_pid(path: &std::path::Path) -> libc::pid_t {
    // Gate on the file PARSING, not on it existing: the shell creates the file
    // before it writes the pid into it, so `exists()` goes true while the
    // contents are still empty and the parse below panics.
    let recorded = wait_until(CANCELLATION_GATE, || parsed_pid(path).is_some());
    assert!(
        recorded,
        "pid file {} never came to hold a pid",
        path.display()
    );
    parsed_pid(path).unwrap_or_else(|| panic!("pid file {} should hold a pid", path.display()))
}

/// A pid file that is created before it is written must not be read in the gap
/// between the two: `read_pid` has to wait for contents, not for existence.
#[cfg(unix)]
#[test]
fn read_pid_waits_for_a_pid_file_that_is_created_before_it_is_written() {
    let directory = tempfile::tempdir().expect("temporary directory should be created");
    let path = directory.path().join("two-step.pid");
    fs::write(&path, "").expect("empty pid file should be created");

    let writer_path = path.clone();
    let writer = thread::spawn(move || {
        // The delay only widens the window that the old `exists()` gate raced;
        // the assertion below does not depend on it, because `read_pid` waits
        // for contents however late they arrive.
        thread::sleep(Duration::from_millis(250));
        fs::write(&writer_path, "4321\n").expect("pid should be written");
    });

    assert_eq!(read_pid(&path), 4321);
    writer.join().expect("writer should finish");
}

#[cfg(unix)]
#[test]
fn process_runner_kills_cancelled_child() {
    let directory = tempfile::tempdir().expect("temporary directory should be created");
    let binary = directory.path().join("gcloud");
    let pid_path = directory.path().join("child.pid");
    // `exec` keeps the pid, so `$$` recorded here is the sleeping process itself.
    fs::write(
        &binary,
        format!(
            "#!/bin/sh\nprintf '%s\\n' \"$$\" > '{}'\nexec sleep 300\n",
            pid_path.display()
        ),
    )
    .expect("test command should be written");
    let mut permissions = fs::metadata(&binary)
        .expect("test command metadata should be available")
        .permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&binary, permissions).expect("test command should be executable");

    let cancellation = CancellationToken::new();
    let worker_cancellation = cancellation.clone();
    let runner = ProcessCommandRunner::new(binary);
    let (sender, receiver) = mpsc::channel();
    let worker = thread::spawn(move || {
        let result = runner.run_with_cancellation(
            GcloudCommand::new(["--configuration", "dev", "projects", "list"]),
            &worker_cancellation,
        );
        let _ = sender.send(result);
    });

    // Reading the pid both arms the cleanup guard and proves the child is up, so
    // cancellation lands on a running process rather than on a spawn in flight.
    let mut cleanup = KillOnDrop::arm(read_pid(&pid_path));
    cancellation.cancel();

    // The child sleeps for 300s, so only a wait on the child can exceed this
    // gate: 30s is a tenth of the sleep, which makes a timeout a hang rather
    // than scheduling jitter.
    let result = receiver
        .recv_timeout(CANCELLATION_GATE)
        .expect("cancellation should not wait for the child to finish");
    worker.join().expect("worker should finish");

    assert!(
        matches!(result, Err(CommandError::Cancelled { .. })),
        "{result:?}"
    );
    cleanup.disarm_once_dead();
}

#[cfg(unix)]
#[test]
fn process_runner_kills_cancelled_process_group() {
    let directory = tempfile::tempdir().expect("temporary directory should be created");
    let pid_path = directory.path().join("backgrounded.pid");
    let script = format!(
        "trap '' HUP; sleep 300 & printf '%s\\n' \"$!\" > '{}'; wait",
        pid_path.display()
    );
    let cancellation = CancellationToken::new();
    let worker_cancellation = cancellation.clone();
    let runner = ProcessCommandRunner::new("/bin/sh".into());
    let (sender, receiver) = mpsc::channel();
    let worker = thread::spawn(move || {
        let result =
            runner.run_with_cancellation(GcloudCommand::new(["-c", &script]), &worker_cancellation);
        let _ = sender.send(result);
    });

    // Killing the sleep also releases the shell parked in `wait`, so this one
    // guard cleans up the whole group if the runner failed to.
    let mut cleanup = KillOnDrop::arm(read_pid(&pid_path));
    cancellation.cancel();

    // Only waiting on the backgrounded 300s sleep can blow through a 30s gate.
    let result = receiver
        .recv_timeout(CANCELLATION_GATE)
        .expect("cancellation should not wait for the backgrounded sleep");
    worker.join().expect("worker should finish");

    assert!(
        matches!(result, Err(CommandError::Cancelled { .. })),
        "{result:?}"
    );
    cleanup.disarm_once_dead();
}

#[cfg(unix)]
#[test]
fn process_runner_bounds_cancellation_when_descendant_keeps_pipes_open() {
    let directory = tempfile::tempdir().expect("temporary directory should be created");
    let pid_path = directory.path().join("descendant.pid");
    let script = format!(
        "perl -MPOSIX -e 'POSIX::setsid(); open my $fh, \">\", $ARGV[0] or die; print $fh \"$$\\n\"; close $fh; exec \"sleep\", \"300\"' '{}' & wait",
        pid_path.display()
    );
    let cancellation = CancellationToken::new();
    let worker_cancellation = cancellation.clone();
    let runner = ProcessCommandRunner::new("/bin/sh".into());
    let (sender, receiver) = mpsc::channel();
    let worker = thread::spawn(move || {
        let result =
            runner.run_with_cancellation(GcloudCommand::new(["-c", &script]), &worker_cancellation);
        let _ = sender.send(result);
    });

    // The descendant `setsid`s out of the runner's process group, so the runner
    // never kills it and this test owns its cleanup unconditionally — the guard
    // has to be armed before anything below can panic, and it stays armed on
    // success too: the descendant is still alive then and the guard's kill is
    // the only thing that reaps it.
    let _cleanup = KillOnDrop::arm(read_pid(&pid_path));
    cancellation.cancel();

    // The setsid descendant holds the inherited pipes open for 300s, so only a
    // wait on it can exceed a 30s gate.
    let result = receiver
        .recv_timeout(CANCELLATION_GATE)
        .expect("cancellation should not wait for inherited pipes");
    worker.join().expect("worker should finish");

    assert!(
        matches!(result, Err(CommandError::Cancelled { .. })),
        "{result:?}"
    );
}

#[cfg(unix)]
#[test]
fn process_runner_terminates_process_group_before_joining_completed_pipes() {
    let directory = tempfile::tempdir().expect("temporary directory should be created");
    let pid_path = directory.path().join("detached.pid");
    let script = format!(
        "sleep 300 & printf '%s\\n' \"$!\" > '{}'; exit 0",
        pid_path.display()
    );
    let runner = ProcessCommandRunner::new("/bin/sh".into());
    let (sender, receiver) = mpsc::channel();
    let worker = thread::spawn(move || {
        let result = runner.run_with_cancellation(
            GcloudCommand::new(["-c", &script]),
            &CancellationToken::new(),
        );
        let _ = sender.send(result);
    });

    let mut cleanup = KillOnDrop::arm(read_pid(&pid_path));

    // The shell exits at once while the detached sleep keeps the inherited pipes
    // open, so PIPE_READ_TIMEOUT decides this test at 1s: returning must depend
    // on the shell, not on the sleep. The gate is hang insurance on top of that —
    // if the runner ever waited on the process group, nothing else would stop it.
    let result = receiver
        .recv_timeout(CANCELLATION_GATE)
        .expect("returning should not wait for the detached sleep");
    worker.join().expect("worker should finish");

    assert!(result.is_ok(), "{result:?}");
    cleanup.disarm_once_dead();
}

#[cfg(unix)]
#[test]
fn process_runner_kills_the_descendant_group_of_a_completed_child() {
    let directory = tempfile::tempdir().expect("temporary directory should be created");
    let pid_path = directory.path().join("descendant.pid");
    // The descendant drops the inherited pipes, so the runner returns Ok either
    // way and the surviving-pid check below is what actually decides this test.
    let script = format!(
        "sleep 300 >/dev/null 2>&1 & printf '%s\\n' \"$!\" > '{}'; exit 0",
        pid_path.display()
    );

    let runner = ProcessCommandRunner::new("/bin/sh".into());
    let (sender, receiver) = mpsc::channel();
    let worker = thread::spawn(move || {
        let result = runner.run_with_cancellation(
            GcloudCommand::new(["-c", &script]),
            &CancellationToken::new(),
        );
        let _ = sender.send(result);
    });

    let pid = read_pid(&pid_path);
    let mut cleanup = KillOnDrop::arm(pid);

    let result = receiver
        .recv_timeout(CANCELLATION_GATE)
        .expect("run_with_cancellation should return once the shell exits");
    worker.join().expect("worker should finish");
    assert!(result.is_ok(), "{result:?}");

    let gone = wait_until(DESCENDANT_EXIT_DEADLINE, || !pid_is_alive(pid));
    assert!(
        gone,
        "descendant pid {pid} survived the call: the completed child's process group was never killed"
    );
    cleanup.disarm();
}

/// The defect at the runner's own seam: the child writes its
/// output and exits, but a descendant keeps a copy of the inherited write end, so
/// the EOF the readers used to wait for never arrives and the caller loses output
/// the child really produced.
///
/// The descendant `setsid`s out of the runner's process group first. An in-group
/// `sleep 300 &` proves nothing here: `terminate_and_reap` signals the group before
/// the readers are awaited, which closes that copy and delivers the EOF anyway —
/// `process_runner_terminates_process_group_before_joining_completed_pipes` is
/// exactly that case, and it passed before this fix. Only a holder the group kill
/// cannot reach reproduces the measured flake, where the surviving copy belongs to
/// a stranger the runner never created.
#[cfg(unix)]
#[test]
fn run_with_cancellation_returns_output_when_a_descendant_holds_the_pipes() {
    let directory = tempfile::tempdir().expect("temporary directory should be created");
    let pid_path = directory.path().join("holder.pid");
    // The descendant keeps the inherited stdout pipe — no redirection — and holds
    // it for 300s, which is what withholds EOF from the runner's reader.
    //
    // The shell waits for the pid file before exiting, and the descendant writes it
    // only after `setsid`. Without that handshake the child can exit while the
    // descendant is still in its process group, where `terminate_and_reap`'s signal
    // reaches it, closes the copy and hands the reader the EOF this test is meant to
    // do without.
    //
    // The shell's wait is bounded at ~10s: if perl or POSIX is missing the
    // backgrounded command dies and the pid file never appears, and an unbounded
    // `until` loop would leave this shell spinning after `read_pid` panics at its own
    // 30s gate — which happens before `KillOnDrop::arm` can run, so nothing would kill
    // it. The bound only stops the spin; `read_pid` is still what fails the test.
    let script = format!(
        "perl -MPOSIX -e 'POSIX::setsid(); open my $fh, \">\", $ARGV[0] or die; print $fh \"$$\\n\"; close $fh; exec \"sleep\", \"300\"' '{pid}' & for _ in $(seq 200); do [ -s '{pid}' ] && break; sleep 0.05; done; printf hello; exit 0",
        pid = pid_path.display()
    );

    let runner = ProcessCommandRunner::new("/bin/sh".into());
    let (sender, receiver) = mpsc::channel();
    let worker = thread::spawn(move || {
        let result = runner.run_with_cancellation(
            GcloudCommand::new(["-c", &script]),
            &CancellationToken::new(),
        );
        let _ = sender.send(result);
    });

    // Outside the runner's process group, so the runner never kills it and this
    // test owns its cleanup unconditionally — armed before anything below can
    // panic, and still armed on success, where the guard's kill is the only thing
    // that ends the descendant.
    let _cleanup = KillOnDrop::arm(read_pid(&pid_path));

    // The descendant holds the pipes for 300s, so only a wait on it can exceed a
    // 30s gate; the assertion below is on the output, not on how long it took.
    let result = receiver
        .recv_timeout(CANCELLATION_GATE)
        .expect("returning should not wait for the descendant holding the pipes");
    worker.join().expect("worker should finish");

    let output = result.expect("the child's output should survive the descendant");
    assert_eq!(output.stdout, "hello");
}

/// `child_has_exited` peeks with `WNOWAIT`, which reports the exit and
/// deliberately leaves the zombie in place so the pid stays reserved until the
/// process group has been signalled. Deferring the reap must not skip it: the
/// ordinary successful invocation — one child, no descendants — has to come back
/// with nothing waitable left.
///
/// This drives `run_with_cancellation` over its public surface only, so it lives
/// here rather than in cli.rs's unit tests: the helper that reads the reap is
/// shared with `tests/ci_reliability_evidence.rs`, which cannot see
/// anything private to the crate. That file pins the same property for a child
/// that leaves a descendant behind; this is the no-descendant case, where the
/// group empties the instant the leader is reaped.
#[cfg(unix)]
#[test]
fn run_with_cancellation_reaps_the_child_it_peeked_at() {
    let directory = tempfile::tempdir().expect("temporary directory should be created");
    let pid_file = directory.path().join("pid");
    let runner = ProcessCommandRunner::new("/bin/sh".into());
    let script = format!("echo $$ > {}", pid_file.display());

    runner
        .run_with_cancellation(
            GcloudCommand::new(["-c", script.as_str()]),
            &CancellationToken::new(),
        )
        .expect("command succeeds");

    // Read directly rather than through `read_pid`: the runner has already
    // reaped the child by the time it returns, so the file is complete and a
    // wait here would hide a runner that returned before the child was done.
    let pid: libc::pid_t = fs::read_to_string(&pid_file)
        .expect("read pid file")
        .trim()
        .parse()
        .expect("parse pid");
    assert!(
        !has_unreaped_child(pid),
        "pid {pid} is still waitable: the WNOWAIT peek left a zombie behind"
    );
}

#[test]
fn update_context_stops_after_region_failure() {
    let runner = FakeRunner::default();
    runner.outputs.borrow_mut().push(output());
    runner.outputs.borrow_mut().push(Err(CommandError::Failed {
        stdout: String::new(),
        stderr: "region update failed".to_string(),
        exit_code: None,
    }));
    runner.outputs.borrow_mut().push(output());
    let service = GcloudConfigurationService::new(runner);

    let error = service
        .update_context(GcloudContextEditDraft {
            configuration: GcloudConfigurationName::new("dev"),
            project: Some("dev-project".to_string()),
            region: Some("europe-west1".to_string()),
            zone: Some("europe-west1-b".to_string()),
        })
        .expect_err("region failure should stop update");

    assert!(error.to_string().contains("region update failed"));
    let commands = service.runner().commands.borrow();
    assert_eq!(commands.len(), 2);
    assert_eq!(
        commands[0].args,
        args(&[
            "--configuration",
            "dev",
            "config",
            "set",
            "project",
            "dev-project",
        ])
    );
    assert_eq!(
        commands[1].args,
        args(&[
            "--configuration",
            "dev",
            "config",
            "set",
            "compute/region",
            "europe-west1",
        ])
    );
}

/// The ceiling a timeout test gives `run`. One second rather than something
/// sub-second so the reported `seconds` is a number the assertion can name: a
/// 200ms ceiling reports `0`, which would prove the variant fired without
/// proving it carried the ceiling that fired it.
///
/// A ceiling this tight is only safe because neither test that uses it execs a
/// *freshly written* file. Both run `/bin/sh -c`, and that is load-bearing on
/// macOS: a script written moments earlier costs seconds on its first exec
/// while the system vets it, and both tests read a pid the child records as
/// its first act. Measured on 2026-08-26 with a probe inside the test — spawn
/// returned in ~1ms and the new script took 2.0s to 3.1s to reach its first
/// `printf`, five runs out of five, with the box otherwise idle. Under a
/// script-file fixture the ceiling therefore killed the child before it ever
/// recorded itself, `read_pid` waited out its whole 30s gate on a file nobody
/// would write, and the test failed as a hang in `run` that was not one. A
/// system binary is already vetted, so `/bin/sh` starts in milliseconds and
/// the pid is recorded long before any ceiling this file sets.
#[cfg(unix)]
const TEST_RUN_TIMEOUT: Duration = Duration::from_secs(1);

/// `run` had no timeout, no kill and no cancellation: it was `Command::output()`,
/// which blocks until the child closes both pipes *and* exits.
///
/// Its sibling `run_with_cancellation` has an owner who can end it. `run` has
/// nobody, so a child that never comes back held that thread for good — and
/// since BigQuery that is a routine shape rather than a deliberate one: opening
/// a dataset fans out one `bq show` per listed row through `run`, four at a
/// time behind a semaphore, and one hung call holds its permit forever while
/// the rest queue behind it.
///
/// The child here sleeps for 300s, so the only way to reach the gate is to wait
/// on it. Returning inside the gate *is* the property; the variant and its
/// `seconds` are what say the deadline is why.
#[cfg(unix)]
#[test]
fn process_runner_times_out_a_child_that_never_exits() {
    let directory = tempfile::tempdir().expect("temporary directory should be created");
    let pid_path = directory.path().join("child.pid");
    // `/bin/sh -c` rather than a script file written here: see
    // `TEST_RUN_TIMEOUT` for why a fresh executable cannot be raced against a
    // one-second ceiling on macOS. `exec` keeps the pid, so `$$` recorded here
    // is the sleeping process itself.
    let script = format!(
        "printf '%s\\n' \"$$\" > '{}'; exec sleep 300",
        pid_path.display()
    );

    let runner = ProcessCommandRunner::new("/bin/sh".into()).with_timeout(TEST_RUN_TIMEOUT);
    let (sender, receiver) = mpsc::channel();
    let worker = thread::spawn(move || {
        let result = runner.run(GcloudCommand::new(["-c", script.as_str()]));
        let _ = sender.send(result);
    });

    // Reading the pid both arms the cleanup guard and proves the child is up,
    // so the deadline lands on a running process rather than on a spawn in
    // flight.
    let mut cleanup = KillOnDrop::arm(read_pid(&pid_path));

    // 30s against a 300s sleep and a 1s ceiling: only a wait on the child can
    // exceed this, which makes a trip here a hang rather than jitter.
    let result = receiver
        .recv_timeout(CANCELLATION_GATE)
        .expect("run should not wait for a child that never exits");
    worker.join().expect("worker should finish");

    assert!(
        matches!(
            result,
            Err(CommandError::TimedOut { seconds, .. }) if seconds == TEST_RUN_TIMEOUT.as_secs()
        ),
        "{result:?}"
    );
    // The child is killed *and reaped*, not abandoned. A `run` that returned
    // while leaving the 300s sleep behind would pass the gate above and still
    // leak a process per hung call — a fan-out's worth of them per dataset —
    // and one that killed without reaping would leave a zombie per call
    // instead, which `pid_is_alive` cannot tell from a running process.
    assert!(
        !has_unreaped_child(cleanup.pid),
        "a timed-out child must be reaped, not left a zombie"
    );
    cleanup.disarm_once_dead();
}

/// The timeout kills the child's whole process group, not just the child.
///
/// This is what `run` gained by going through the same body as
/// `run_with_cancellation` rather than getting a deadline bolted onto
/// `output()`: `output()` has no `process_group(0)`, so the backgrounded
/// `sleep` below would have survived any timeout built around it — still
/// holding the inherited pipes, which is the other half of why `output()` could
/// not be rescued in place.
#[cfg(unix)]
#[test]
fn process_runner_kills_the_timed_out_process_group() {
    let directory = tempfile::tempdir().expect("temporary directory should be created");
    let pid_path = directory.path().join("backgrounded.pid");
    let script = format!(
        "trap '' HUP; sleep 300 & printf '%s\\n' \"$!\" > '{}'; wait",
        pid_path.display()
    );

    let runner = ProcessCommandRunner::new("/bin/sh".into()).with_timeout(TEST_RUN_TIMEOUT);
    let (sender, receiver) = mpsc::channel();
    let worker = thread::spawn(move || {
        let result = runner.run(GcloudCommand::new(["-c", script.as_str()]));
        let _ = sender.send(result);
    });

    let mut cleanup = KillOnDrop::arm(read_pid(&pid_path));

    let result = receiver
        .recv_timeout(CANCELLATION_GATE)
        .expect("the timeout should not wait for the backgrounded sleep");
    worker.join().expect("worker should finish");
    assert!(
        matches!(result, Err(CommandError::TimedOut { .. })),
        "{result:?}"
    );

    assert!(
        wait_until(DESCENDANT_EXIT_DEADLINE, || !pid_is_alive(cleanup.pid)),
        "the backgrounded sleep must die with the group, not outlive the timeout"
    );
    cleanup.disarm();
}

/// The ordinary path still works after the refactor.
///
/// `run` used to read its pipes with `Command::output()` and now drains them
/// through the two reader threads `run_with_cancellation` uses. Every other
/// test in this file that exercises `run` does so through a stub runner, so
/// without this one the change from `output()` to the polled drain has no
/// real-child coverage at all on the success path — and a drain that dropped
/// stdout would fail no assertion here previously.
#[cfg(unix)]
#[test]
fn process_runner_returns_a_real_child_stdout_and_failure_status() {
    let runner = ProcessCommandRunner::new("/bin/sh".into());

    let output = runner
        .run(GcloudCommand::new([
            "-c",
            "printf 'out\\n'; printf 'err\\n' >&2",
        ]))
        .expect("a successful child should return its output");
    assert_eq!(output.stdout, "out\n");
    assert_eq!(output.stderr, "err\n");

    let failed = runner.run(GcloudCommand::new(["-c", "printf 'boom\\n' >&2; exit 3"]));
    assert!(
        matches!(
            &failed,
            Err(CommandError::Failed { stdout, stderr, exit_code })
                if stdout.is_empty() && stderr == "boom\n" && *exit_code == Some(3)
        ),
        "{failed:?}"
    );
}

/// A child whose diagnostic came out on stdout still says what went wrong.
///
/// This is the whole of the defect at the layer that caused it. `bq` writes its
/// error text to *stdout* and leaves stderr empty — verified against the live
/// binary for all three calls nuvem issues (`ls -d`, `ls <dataset>`, `show`) —
/// while `gcloud` writes to stderr. `run_bounded` read stderr, dropped stdout
/// on the floor, and every BigQuery refusal reached the operator as
/// "command exited unsuccessfully: " with nothing after the colon.
///
/// The assertion is on the rendered message rather than on a field, because
/// the message is what the three BigQuery surfaces actually carry: they build
/// their `AppError` from `error.to_string()`, not from the variant. A fix that
/// stored stdout somewhere without teaching `Display` about it would leave
/// this red.
#[cfg(unix)]
#[test]
fn a_failed_child_that_diagnosed_on_stdout_still_reports_why() {
    let runner = ProcessCommandRunner::new("/bin/sh".into());

    let failed = runner
        .run(GcloudCommand::new([
            "-c",
            "printf 'BigQuery error in ls operation: Not found: Dataset p:nope\\n'; exit 2",
        ]))
        .expect_err("a non-zero exit is a failure");

    assert!(
        failed
            .to_string()
            .contains("BigQuery error in ls operation: Not found: Dataset p:nope"),
        "the child's only diagnostic was on stdout and must survive: {failed}"
    );
}

/// Both streams are kept, and stderr is the one that speaks.
///
/// The paired positive to the test above: "the detail is no longer empty" is
/// satisfied by a constant, and a fallback that *replaced* stderr with stdout
/// would satisfy it too — while quietly demoting every gcloud failure, which
/// is the far larger population. So this pins the preference in the direction
/// that has to hold, and pins the two streams as distinct fields rather than
/// as one resolved string: a caller wanting the transcript of a failed child
/// can have it, and the resolution stays a display concern.
#[cfg(unix)]
#[test]
fn a_failed_child_keeps_both_streams_and_stderr_is_the_diagnostic() {
    let runner = ProcessCommandRunner::new("/bin/sh".into());

    let failed = runner
        .run(GcloudCommand::new([
            "-c",
            "printf 'on stdout\\n'; printf 'on stderr\\n' >&2; exit 4",
        ]))
        .expect_err("a non-zero exit is a failure");

    match &failed {
        CommandError::Failed {
            stdout,
            stderr,
            exit_code,
        } => {
            assert_eq!(stdout, "on stdout\n");
            assert_eq!(stderr, "on stderr\n");
            assert_eq!(*exit_code, Some(4));
        }
        other => panic!("{other:?}"),
    }

    let rendered = failed.to_string();
    assert!(rendered.contains("on stderr"), "{rendered}");
    assert!(
        !rendered.contains("on stdout"),
        "a child that used stderr must not have stdout spliced in beside it: {rendered}"
    );
}

/// A `run` child must not inherit nuvem's stdin.
///
/// `run` was `Command::output()` once, and `output()` sets the child's
/// stdin to `Stdio::null()` for you. `spawn()` does not: it inherits, and
/// `run_bounded` configured only stdout and stderr, so the ceiling that was
/// added quietly came with a regression the timeout tests could not see.
///
/// What it costs is specific to a TUI. nuvem holds the terminal in raw mode and
/// reads keys from it with an `EventStream`, so an inherited stdin is the
/// operator's keyboard: a child that reads it takes keystrokes out of the
/// terminal that nuvem is trying to read, and — since nothing types EOF into a
/// raw-mode terminal — sits there doing it until the ceiling kills it, two
/// minutes later.
///
/// The test gives *this* process a stdin the child would want: a pipe with a
/// byte already in it and the write end still open, so a `read` that inherits
/// it succeeds immediately and a `read` that gets `/dev/null` fails
/// immediately. A pipe rather than the harness's own stdin because the harness
/// runs tests under nextest with stdin already at `/dev/null`, where an
/// inheriting child and a null-ed child behave identically and the test would
/// pass against the defect. Both halves are asserted: what the child saw, and
/// that the byte is still in the pipe afterwards — the second is the operator's
/// symptom, a keystroke that nuvem never gets to read.
///
/// fd 0 is process-wide, and this test replaces it for as long as it runs.
/// Under nextest — which both gating tiers require — each test has its own
/// process and that is invisible to everything else. Under bare `cargo test`
/// the whole suite is one process with tests on parallel threads, so this joins
/// the theme arc-swap as process-global state `tests/suite.rs` calls out: no
/// other test reads fd 0, so nothing is known to race with it, but the
/// unisolated path is the one where that is an argument rather than a
/// guarantee. The guard puts fd 0 back and closes the pipe on every exit,
/// panics included, so the leak is bounded to this test either way.
#[cfg(unix)]
#[test]
fn process_runner_gives_the_child_a_null_stdin() {
    /// Owns every descriptor the test opens, including the saved stdin, so a
    /// panic between here and the end restores fd 0 and closes the pipe rather
    /// than leaving the harness reading from it.
    struct StdinGuard {
        saved: libc::c_int,
        read_fd: libc::c_int,
        write_fd: libc::c_int,
    }

    impl Drop for StdinGuard {
        fn drop(&mut self) {
            // SAFETY: none of these calls takes a pointer. Every descriptor
            // named here was produced by this test — `saved` from `dup(0)`, the
            // other two from its `pipe` — and the guard is the sole owner of all
            // three, constructed once and dropped once, so each is closed
            // exactly here and exactly once. The `dup2` restores the real stdin
            // onto fd 0 before `saved` is closed, so fd 0 is never left dangling.
            unsafe {
                libc::dup2(self.saved, 0);
                libc::close(self.saved);
                libc::close(self.write_fd);
                libc::close(self.read_fd);
            }
        }
    }

    let mut pipe_fds = [0 as libc::c_int; 2];
    // SAFETY: `pipe_fds` is a live `[c_int; 2]` on this frame, exactly the
    // two-element array `pipe` writes the new descriptors into.
    assert_eq!(unsafe { libc::pipe(pipe_fds.as_mut_ptr()) }, 0, "pipe");
    // SAFETY: `dup` takes no pointers, and fd 0 is open — this is a test binary
    // the harness runs with a stdin. The returned descriptor is owned by this
    // frame and handed straight to the guard below, which closes it once.
    let saved = unsafe { libc::dup(0) };
    assert!(saved >= 0, "stdin should be saveable");
    let guard = StdinGuard {
        saved,
        read_fd: pipe_fds[0],
        write_fd: pipe_fds[1],
    };

    let keystroke = b"keystroke\n";
    assert_eq!(
        // SAFETY: the pointer and length come from the same live `keystroke`
        // array, so `write` reads only initialised bytes this frame owns, and
        // `guard.write_fd` is the pipe's write end, still open until the guard
        // drops.
        unsafe { libc::write(guard.write_fd, keystroke.as_ptr().cast(), keystroke.len()) },
        keystroke.len() as isize,
        "the pipe should accept the byte the child must not get"
    );
    assert_eq!(
        // SAFETY: `dup2` takes no pointers, and `guard.read_fd` is the pipe's
        // read end, open for as long as the guard lives. Overwriting fd 0 loses
        // nothing: the guard already holds `dup(0)` and restores it on every
        // exit from this test, panics included.
        unsafe { libc::dup2(guard.read_fd, 0) },
        0,
        "dup2 onto stdin"
    );

    let runner = ProcessCommandRunner::new("/bin/sh".into()).with_timeout(TEST_RUN_TIMEOUT);
    let output = runner
        .run(GcloudCommand::new([
            "-c",
            "if IFS= read -r line; then printf 'inherited:%s' \"$line\"; else printf 'closed'; fi",
        ]))
        .expect("a child that reads its stdin should still succeed");

    assert_eq!(
        output.stdout, "closed",
        "the child read nuvem's stdin instead of an empty one"
    );

    // The byte is still readable here, which is the half that says the
    // operator's keystroke was never taken. Non-blocking so a child that did
    // consume it fails this assertion instead of parking the test on an empty
    // pipe.
    // SAFETY: `guard.read_fd` is the pipe's read end, still open here, and
    // `F_GETFL` is the variadic `fcntl` command that takes no further argument.
    let flags = unsafe { libc::fcntl(guard.read_fd, libc::F_GETFL) };
    // SAFETY: same live descriptor, and `F_SETFL` takes exactly one `c_int`,
    // which is what the flags word passed here is — no pointer is involved.
    unsafe { libc::fcntl(guard.read_fd, libc::F_SETFL, flags | libc::O_NONBLOCK) };
    let mut buffer = [0u8; 32];
    // SAFETY: the pointer and length come from the same live `buffer`, so `read`
    // writes only within it, and every byte it may write is a valid `u8`. The
    // descriptor is the guard's still-open read end, now non-blocking, so the
    // call returns rather than parking on an empty pipe.
    let read = unsafe { libc::read(guard.read_fd, buffer.as_mut_ptr().cast(), buffer.len()) };
    assert_eq!(
        read,
        keystroke.len() as isize,
        "the child consumed what was waiting on nuvem's stdin"
    );
    assert_eq!(&buffer[..keystroke.len()], keystroke);
}

/// A command that declares the lifecycle ceiling is held to *it*, not to the
/// read ceiling the runner carries.
///
/// The child here outlives the runner's ceiling and nothing else about the call
/// is unusual, so a `run` that still reached for `self.timeout` kills it and
/// returns `TimedOut` — which is exactly what a Cloud SQL failover used to
/// get, two minutes into an operation that had already fired.
#[cfg(unix)]
#[test]
fn a_lifecycle_command_is_not_held_to_the_read_ceiling() {
    let runner = ProcessCommandRunner::new("/bin/sh".into()).with_timeout(TEST_RUN_TIMEOUT);

    let result = runner.run(
        GcloudCommand::new(["-c", "sleep 2; printf 'operation complete'"])
            .with_ceiling(CommandCeiling::Lifecycle),
    );

    let output = result.expect("a lifecycle command must outlive the read ceiling");
    assert_eq!(output.stdout, "operation complete");
}

/// A lifecycle command run through the cancellable-and-bounded path
/// stops waiting when its token flips, and kills the child it was waiting on.
///
/// This is the shape a batch target takes at shutdown: the operator pressed
/// `q`, the process is on its way out, and the thirty minutes
/// `CommandCeiling::Lifecycle` allows are thirty minutes `Runtime::drop` would
/// otherwise sit through. The child here sleeps for 300s, so returning at all
/// is the whole assertion — a `run` with no token cannot.
///
/// What it does not buy, and what the error must not claim: the GCP operation.
/// A `stop` already accepted proceeds whatever happens to the CLI, which is why
/// the message is asserted to carry the same caveat the read timeout carries.
#[cfg(unix)]
#[test]
fn a_cancelled_lifecycle_command_stops_waiting_and_kills_its_child() {
    let directory = tempfile::tempdir().expect("temporary directory should be created");
    let binary = directory.path().join("gcloud");
    let pid_path = directory.path().join("lifecycle.pid");
    // `exec` keeps the pid, so `$$` recorded here is the sleeping process itself.
    fs::write(
        &binary,
        format!(
            "#!/bin/sh\nprintf '%s\\n' \"$$\" > '{}'\nexec sleep 300\n",
            pid_path.display()
        ),
    )
    .expect("test command should be written");
    let mut permissions = fs::metadata(&binary)
        .expect("test command metadata should be available")
        .permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&binary, permissions).expect("test command should be executable");

    let cancellation = CancellationToken::new();
    let worker_cancellation = cancellation.clone();
    let runner = ProcessCommandRunner::new(binary);
    let (sender, receiver) = mpsc::channel();
    let worker = thread::spawn(move || {
        let result = runner.run_bounded_with_cancellation(
            GcloudCommand::new(["compute", "instances", "stop", "vm-a"])
                .with_ceiling(CommandCeiling::Lifecycle),
            &worker_cancellation,
        );
        let _ = sender.send(result);
    });

    // Reading the pid both arms the cleanup guard and proves the child is up, so
    // cancellation lands on a running process rather than on a spawn in flight.
    let mut cleanup = KillOnDrop::arm(read_pid(&pid_path));
    cancellation.cancel();

    // The child sleeps for 300s and the lifecycle ceiling is 30 minutes, so
    // only a wait on the child can exceed this gate.
    let result = receiver
        .recv_timeout(CANCELLATION_GATE)
        .expect("a cancelled lifecycle command must not wait for its child");
    worker.join().expect("worker should finish");

    let error = match result {
        Err(error @ CommandError::Cancelled { .. }) => error,
        other => panic!("expected a cancelled lifecycle command, got {other:?}"),
    };
    assert!(
        matches!(
            error,
            CommandError::Cancelled {
                ceiling: CommandCeiling::Lifecycle
            }
        ),
        "the error must remember which ceiling it abandoned: {error:?}"
    );
    // The one sentence separating "we gave up waiting" from "nothing happened".
    assert!(
        error
            .to_string()
            .contains("the requested operation may still be running in GCP"),
        "a cancelled lifecycle call must not imply the operation was undone: {error}"
    );
    cleanup.disarm_once_dead();
}

/// A read cancelled the same way says nothing about GCP, because a read that
/// was killed started nothing there. Pairs with the lifecycle assertion above:
/// without this, a caveat appended unconditionally would satisfy that test and
/// print a warning on every cancelled listing, which is how a warning stops
/// being read where it matters.
#[cfg(unix)]
#[test]
fn a_cancelled_read_claims_nothing_about_the_operation() {
    let cancellation = CancellationToken::new();
    cancellation.cancel();
    let runner = ProcessCommandRunner::new("/bin/sh".into());

    let result =
        runner.run_bounded_with_cancellation(GcloudCommand::new(["-c", "exit 0"]), &cancellation);

    let error = match result {
        Err(error @ CommandError::Cancelled { .. }) => error,
        other => panic!("expected a cancelled read, got {other:?}"),
    };
    assert!(
        matches!(
            error,
            CommandError::Cancelled {
                ceiling: CommandCeiling::Read
            }
        ),
        "{error:?}"
    );
    assert_eq!(error.to_string(), "command was cancelled");
}

/// The cancellable path keeps the ceiling. This is the whole reason it is a
/// third method rather than a call to `run_with_cancellation`, which is
/// deliberately unbounded (see `RUN_TIMEOUT`): a batch target handed the
/// unbounded one would trade a shutdown hang for a call with no ceiling at all.
///
/// The token here is live and never flips, so only the deadline can end this.
/// The child outlives the ceiling by seconds rather than by minutes on purpose:
/// a runner that lost the deadline then *returns* — with the child's own output
/// — instead of hanging this test for five minutes, so the failure names the
/// defect rather than tripping a harness timeout.
#[cfg(unix)]
#[test]
fn a_cancellable_run_is_still_held_to_the_commands_ceiling() {
    let runner = ProcessCommandRunner::new("/bin/sh".into()).with_timeout(TEST_RUN_TIMEOUT);
    let cancellation = CancellationToken::new();

    let result = runner.run_bounded_with_cancellation(
        GcloudCommand::new(["-c", "sleep 5; printf 'unbounded'"]),
        &cancellation,
    );

    assert!(
        matches!(
            result,
            Err(CommandError::TimedOut { seconds, ceiling })
                if seconds == TEST_RUN_TIMEOUT.as_secs() && ceiling == CommandCeiling::Read
        ),
        "an uncancelled run must still hit its ceiling: {result:?}"
    );
}

/// The positive half: a cancellable run that nobody cancels spawns the child,
/// waits for it, and hands back what it wrote. Without this, every assertion
/// above is satisfied by a method that never runs anything.
#[cfg(unix)]
#[test]
fn a_cancellable_run_returns_the_childs_output_when_nothing_cancels_it() {
    let runner = ProcessCommandRunner::new("/bin/sh".into()).with_timeout(TEST_RUN_TIMEOUT);
    let cancellation = CancellationToken::new();

    let output = runner
        .run_bounded_with_cancellation(
            GcloudCommand::new(["-c", "printf 'operation complete'"])
                .with_ceiling(CommandCeiling::Lifecycle),
            &cancellation,
        )
        .expect("an uncancelled run should return the child's output");

    assert_eq!(output.stdout, "operation complete");
}
