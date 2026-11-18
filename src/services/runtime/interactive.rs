//! Typed, tty-independent interactive-process launch: an argv spec, a runner
//! trait (real impl attaches the child to the inherited terminal, no shell),
//! and a terminal-lifecycle seam so the run loop can suspend/restore the TUI
//! around the child. `execute_interactive_process` sequences lifecycle around
//! the runner and is the unit-tested orchestrator.

use nuvem_domain::gcp::configuration::GcloudContext;
use nuvem_domain::gcp::resource::ComputeInstance;

/// A fully-resolved argv to run as an interactive child. Passed directly to
/// the OS — never interpreted by a shell.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InteractiveProcessSpec {
    pub program: String,
    pub args: Vec<String>,
    pub description: String,
}

/// Terminal-independent outcome of a launch attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InteractiveProcessOutcome {
    Succeeded,
    ExitedNonZero { code: i32 },
    TerminatedBySignal { signal: i32 },
    MissingBinary,
    LaunchFailed { message: String },
}

/// Runs a resolved [`InteractiveProcessSpec`] to completion.
pub trait InteractiveProcessRunner {
    fn run(&self, spec: &InteractiveProcessSpec) -> InteractiveProcessOutcome;
}

/// Restores/reinitializes the terminal around a launch. `before_launch` hands
/// the terminal to the child (cooked mode, main screen, cursor shown, mouse
/// off, original panic hook); `after_launch` re-enters the TUI and forces a
/// redraw.
pub trait InteractiveLifecycle {
    fn before_launch(&mut self);
    fn after_launch(&mut self);
}

/// Sequences the terminal lifecycle around the runner. This ordering
/// (`before_launch` -> `run` -> `after_launch`) is the invariant the
/// lifecycle tests pin.
pub fn execute_interactive_process<L, R>(
    lifecycle: &mut L,
    runner: &R,
    spec: &InteractiveProcessSpec,
) -> InteractiveProcessOutcome
where
    L: InteractiveLifecycle,
    R: InteractiveProcessRunner,
{
    lifecycle.before_launch();
    let outcome = runner.run(spec);
    lifecycle.after_launch();
    outcome
}

/// Builds the argv for `gcloud compute ssh <instance> --zone <zone>` under the
/// given configuration. `program` is the configured gcloud binary so the
/// preview shows exactly what will run.
pub fn gce_ssh_spec(
    gcloud_binary: &str,
    context: &GcloudContext,
    instance: &ComputeInstance,
) -> InteractiveProcessSpec {
    InteractiveProcessSpec {
        program: gcloud_binary.to_string(),
        args: vec![
            "--configuration".to_string(),
            context.configuration.as_str().to_string(),
            "compute".to_string(),
            "ssh".to_string(),
            instance.name.clone(),
            "--zone".to_string(),
            instance.zone.clone(),
        ],
        description: format!("SSH to {}", instance.name),
    }
}

/// Tmux is considered present when `$TMUX` is set and non-empty. Pure helper
/// over the env value so it is deterministic in tests.
pub fn tmux_detected_in(tmux_env: Option<&str>) -> bool {
    tmux_env.is_some_and(|value| !value.is_empty())
}

/// Real runner: spawns the argv with inherited stdio (child owns the terminal)
/// and maps the wait result to an outcome. No shell is involved.
pub struct SubprocessRunner;

impl InteractiveProcessRunner for SubprocessRunner {
    fn run(&self, spec: &InteractiveProcessSpec) -> InteractiveProcessOutcome {
        use std::process::Command;
        match Command::new(&spec.program).args(&spec.args).status() {
            Ok(status) if status.success() => InteractiveProcessOutcome::Succeeded,
            Ok(status) => match status.code() {
                Some(code) => InteractiveProcessOutcome::ExitedNonZero { code },
                None => InteractiveProcessOutcome::TerminatedBySignal {
                    signal: exit_signal(&status),
                },
            },
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                InteractiveProcessOutcome::MissingBinary
            }
            Err(error) => InteractiveProcessOutcome::LaunchFailed {
                message: error.to_string(),
            },
        }
    }
}

/// The boxed panic hook produced by [`std::panic::take_hook`].
type SavedPanicHook = Box<dyn Fn(&std::panic::PanicHookInfo<'_>) + Sync + Send + 'static>;

/// Real terminal lifecycle around a launch. Restores the terminal to cooked
/// mode and the original panic hook before the child runs, then reinitializes
/// the alternate-screen TUI and forces a full redraw afterward.
pub struct RatatuiLifecycle<'a> {
    terminal: &'a mut ratatui::DefaultTerminal,
    saved_hook: Option<SavedPanicHook>,
}

impl<'a> RatatuiLifecycle<'a> {
    pub fn new(terminal: &'a mut ratatui::DefaultTerminal) -> Self {
        Self {
            terminal,
            saved_hook: None,
        }
    }
}

impl InteractiveLifecycle for RatatuiLifecycle<'_> {
    fn before_launch(&mut self) {
        // Swap our restore-on-panic hook out for the process's previous hook so
        // a panic while the child owns the terminal does not double-restore.
        self.saved_hook = Some(std::panic::take_hook());
        ratatui::restore();
    }

    fn after_launch(&mut self) {
        *self.terminal = ratatui::init();
        let _ = self.terminal.clear();
        crate::services::runtime::terminal::install_panic_restore_hook();
        self.saved_hook = None;
    }
}

#[cfg(unix)]
fn exit_signal(status: &std::process::ExitStatus) -> i32 {
    use std::os::unix::process::ExitStatusExt;
    status.signal().unwrap_or(0)
}

#[cfg(not(unix))]
fn exit_signal(_status: &std::process::ExitStatus) -> i32 {
    0
}
