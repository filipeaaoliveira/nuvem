//! Evidence that `run_pending_interactive` actually runs the pending command.
//!
//! Every piece it wires — `take_pending_interactive_command`,
//! `resolve_interactive_spec`, `execute_interactive_process`,
//! `apply_interactive_outcome` — has its own tests, and none of them can tell
//! a working glue function from one that does nothing: the glue's whole job
//! is the wiring, and its signature demands the real terminal type. So this
//! test supplies a real terminal — a pseudo-terminal pair, moved onto fd 0
//! and fd 1 for the length of the test — and asserts the two facts a stubbed
//! glue cannot fake: the pending command is consumed, and its outcome lands
//! in the status line.
//!
//! The fd surgery is safe for the same reason gcloud_runner_test.rs's stdin
//! swap is: the tiers run this suite under nextest, one process per test, so
//! no other test shares these descriptors. `ratatui::init` (inside the
//! lifecycle's `after_launch`) enables raw mode on fd 0 and queries a size on
//! fd 1, which is exactly why both ends must be the pty and not nextest's
//! capture pipes — raw mode on a pipe fails and `init` panics.

#![cfg(unix)]

use nuvem::Config;
use nuvem::app::effect::InteractiveCommand;
use nuvem::app::{Action, AppState, Effect};
use nuvem::services::gcloud::cli::{CommandError, CommandOutput, CommandRunner, GcloudCommand};
use nuvem::services::runtime::run_pending_interactive;
use nuvem::services::runtime::session::RuntimeSession;
use nuvem_domain::gcp::configuration::{
    GcloudConfiguration, GcloudConfigurationName, GcloudContext,
};
use nuvem_domain::gcp::resource::{ComputeInstance, ResourceStatus};
use std::os::unix::io::RawFd;
use std::path::PathBuf;

/// Launching an interactive process must not touch the gcloud command runner.
struct UnusedRunner;

impl CommandRunner for UnusedRunner {
    fn run(&self, _command: GcloudCommand) -> Result<CommandOutput, CommandError> {
        panic!("the interactive path must not invoke the gcloud command runner");
    }
}

fn context() -> GcloudContext {
    GcloudConfiguration {
        name: GcloudConfigurationName::new("dev"),
        is_active: true,
        account: Some("dev@example.com".to_string()),
        project: Some("dev-project".to_string()),
        region: None,
        zone: None,
    }
    .context()
}

fn instance() -> ComputeInstance {
    ComputeInstance {
        id: "1001".to_string(),
        name: "api-dev-1".to_string(),
        zone: "europe-west1-b".to_string(),
        status: ResourceStatus::Running,
        machine_type: "e2-medium".to_string(),
        internal_ip: None,
        external_ip: None,
    }
}

/// Restores the saved fds on drop, so an assertion failure mid-test still
/// puts stdin/stdout back before the harness reports it.
struct FdGuard {
    saved_stdin: RawFd,
    saved_stdout: RawFd,
}

impl Drop for FdGuard {
    fn drop(&mut self) {
        // SAFETY: the saved fds are dups of the process's original fd 0 and
        // fd 1, taken below and closed exactly once here after being copied
        // back into place; `dup2` and `close` take no pointers.
        unsafe {
            libc::dup2(self.saved_stdin, 0);
            libc::dup2(self.saved_stdout, 1);
            libc::close(self.saved_stdin);
            libc::close(self.saved_stdout);
        }
    }
}

/// Opens a pty, gives it a real window size, and moves its slave end onto
/// fd 0 and fd 1. Returns the guard that undoes the move and the master end.
fn adopt_pty_as_stdio() -> (FdGuard, RawFd) {
    let mut master: RawFd = -1;
    let mut slave: RawFd = -1;
    let mut size = libc::winsize {
        ws_row: 24,
        ws_col: 80,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    // SAFETY: `openpty` writes the two descriptors into the two live c_ints
    // above and reads the winsize from the live struct; the name argument is
    // allowed to be null.
    //
    // The winsize parameter's declared mutability differs by platform in
    // libc — `*mut` on Apple, `*const` on Linux — so `&mut` is required on
    // one and flagged as unnecessary on the other; the allow covers the
    // Linux build.
    #[allow(clippy::unnecessary_mut_passed)]
    let opened = unsafe {
        libc::openpty(
            &mut master,
            &mut slave,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut size,
        )
    };
    assert_eq!(opened, 0, "openpty");

    // SAFETY: fd 0 and fd 1 are live (the process started with them), `dup`
    // returns fresh descriptors this test owns, and `dup2` replaces 0 and 1
    // with copies of the live slave end.
    let guard = unsafe {
        let saved_stdin = libc::dup(0);
        let saved_stdout = libc::dup(1);
        assert!(saved_stdin >= 0 && saved_stdout >= 0, "dup originals");
        assert_eq!(libc::dup2(slave, 0), 0, "dup2 pty onto stdin");
        assert_eq!(libc::dup2(slave, 1), 1, "dup2 pty onto stdout");
        libc::close(slave);
        FdGuard {
            saved_stdin,
            saved_stdout,
        }
    };
    (guard, master)
}

#[tokio::test]
async fn the_pending_command_is_consumed_and_its_outcome_reaches_the_status_line() {
    let (guard, master) = adopt_pty_as_stdio();

    // Drain the master so the TUI's init/restore escape sequences can never
    // fill the pty buffer and wedge a write. Ends at EOF when the guard's
    // drop closes the last slave copy (fds 0 and 1).
    let drain = std::thread::spawn(move || {
        let mut buffer = [0u8; 4096];
        loop {
            // SAFETY: `master` stays open until this loop sees EOF or an
            // error; the buffer is a live local matching the length passed.
            let read = unsafe { libc::read(master, buffer.as_mut_ptr().cast(), buffer.len()) };
            if read <= 0 {
                break;
            }
        }
        // SAFETY: the loop above is the only reader; the fd is closed once.
        unsafe { libc::close(master) };
    });

    // `/usr/bin/true` stands in for gcloud: the spec's program comes from the
    // configured binary, so the "SSH" child starts, ignores its argv, and
    // exits 0 — a real spawn through the real lifecycle, no live gcloud.
    let config = Config {
        gcloud_binary: PathBuf::from("/usr/bin/true"),
        ..Config::default()
    };
    let mut session = RuntimeSession::new_with_config_and_runner_for_test(
        config,
        AppState::new(),
        Box::new(UnusedRunner),
    );
    session
        .dispatch_action(Action::Effect(Effect::LaunchInteractiveProcess {
            command: InteractiveCommand::GceSsh {
                context: context(),
                instance: instance(),
            },
        }))
        .await;

    let backend = ratatui::backend::CrosstermBackend::new(std::io::stdout());
    let mut terminal = ratatui::Terminal::new(backend).expect("terminal over the pty");

    run_pending_interactive(&mut terminal, &mut session).await;

    assert_eq!(
        session.take_pending_interactive_command(),
        None,
        "the glue must consume the pending command"
    );
    assert_eq!(
        session.app().root.system.status_message.as_deref(),
        Some("SSH to api-dev-1 finished"),
        "the child's outcome must be folded back into the status line"
    );

    // Leave raw mode (set by the lifecycle's after_launch) while fd 0 is
    // still the pty, then give the process its stdio back.
    ratatui::restore();
    drop(guard);
    drain.join().expect("drain thread");
}
