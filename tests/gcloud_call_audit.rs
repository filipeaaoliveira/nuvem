//! Counts the `gcloud` invocations a session makes for ordinary flows.
//!
//! A render costs microseconds; a `gcloud` call is a Python program that costs
//! one to two SECONDS, so what a user experiences as speed is almost entirely
//! how many of these nuvem issues and when. This is the subprocess counterpart
//! of `render_bench`: it does not assert a budget, it reports the counts so a
//! change to them is visible.
//!
//! `#[ignore]`d — a measurement, not an assertion. Run it by name:
//!
//! ```text
//! cargo nextest run -E 'test(gcloud_call_audit)' --run-ignored all --no-capture
//! ```

use std::sync::{Arc, Mutex};

use clap::Parser;
use nuvem::app::{Action, AppState, ResourcesAction, WorkspaceAction};
use nuvem::services::gcloud::cli::{CommandError, CommandOutput, CommandRunner, GcloudCommand};
use nuvem::services::runtime::session::RuntimeSession;
use nuvem::{Cli, Config};

/// Records every argv and answers each with empty JSON, which every parser
/// here reads as "no rows" — enough to drive the flow without pretending to
/// be gcloud.
#[derive(Clone, Default)]
struct CountingRunner {
    // `Arc<Mutex<..>>` rather than `Rc<RefCell<..>>`: the runner crosses onto a
    // `spawn_blocking` thread, which is the whole point of the dispatch path
    // being measured.
    commands: Arc<Mutex<Vec<Vec<String>>>>,
}

impl CommandRunner for CountingRunner {
    fn run(&self, command: GcloudCommand) -> Result<CommandOutput, CommandError> {
        self.commands
            .lock()
            .expect("runner mutex")
            .push(command.args.clone());
        // One active configuration, so the session resolves a context and the
        // flows below actually reach the resource loads. Everything else
        // answers with an empty list, which every parser reads as "no rows".
        let stdout = if command.args.iter().any(|a| a == "configurations") {
            include_str!("fixtures/gcloud_configurations.json").to_string()
        } else {
            "[]".to_string()
        };
        Ok(CommandOutput {
            stdout,
            stderr: String::new(),
        })
    }
}

fn session() -> (RuntimeSession, CountingRunner) {
    let runner = CountingRunner::default();
    let cli = Cli::parse_from(["nuvem", "--config", "/nonexistent/nuvem-audit.toml"]);
    let config = Config::resolve(&cli).config;
    let session = RuntimeSession::new_with_config_and_runner_for_test(
        config,
        AppState::new(),
        Box::new(runner.clone()),
    );
    (session, runner)
}

/// Drains every effect the session has queued, so the count reflects the whole
/// cascade a keystroke sets off rather than only its first call.
async fn settle(session: &mut RuntimeSession) {
    // Bounded, because `process_next_effect_result` parks when the queue is
    // empty rather than reporting empty — the timeout is the exit condition,
    // paid once per drain and never while results are flowing.
    for _ in 0..200 {
        match tokio::time::timeout(
            std::time::Duration::from_millis(25),
            session.process_next_effect_result(),
        )
        .await
        {
            Ok(true) => {}
            _ => break,
        }
    }
}

fn report(label: &str, runner: &CountingRunner, from: usize) -> usize {
    let all = runner.commands.lock().expect("runner mutex");
    let issued = &all[from..];
    println!("{label:38} {:>3} call(s)", issued.len());
    for argv in issued {
        println!("      {}", argv.join(" "));
    }
    all.len()
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "a measurement, not an assertion; run it by hand"]
async fn gcloud_calls_per_flow() {
    let (mut session, runner) = session();
    let mut seen = 0;

    session.initialize().await;
    settle(&mut session).await;
    seen = report("startup", &runner, seen);

    session
        .dispatch_action(Action::Workspace(WorkspaceAction::Resources))
        .await;
    settle(&mut session).await;
    seen = report("enter Resources", &runner, seen);

    // Poll ticks with nothing changed: the steady-state cost of leaving the
    // app open, which at the default five-second cadence is what runs all day.
    for _ in 0..3 {
        session.poll_active_resource().await;
        settle(&mut session).await;
    }
    seen = report("3 idle poll ticks", &runner, seen);

    session
        .dispatch_action(Action::Resources(ResourcesAction::Refresh))
        .await;
    settle(&mut session).await;
    seen = report("explicit refresh", &runner, seen);

    // Leaving and coming straight back. The rows are seconds old and already
    // in state, so anything issued here is a re-fetch of what is on screen.
    session
        .dispatch_action(Action::Workspace(WorkspaceAction::Home))
        .await;
    settle(&mut session).await;
    seen = report("leave Resources for Home", &runner, seen);

    session
        .dispatch_action(Action::Workspace(WorkspaceAction::Resources))
        .await;
    settle(&mut session).await;
    seen = report("come straight back", &runner, seen);

    // Switching service and back, which is the commonest navigation there is.
    session
        .dispatch_action(Action::Resources(ResourcesAction::SelectService(
            nuvem_domain::gcp::resource::ResourceServiceKind::GkeClusters,
        )))
        .await;
    settle(&mut session).await;
    seen = report("switch to GKE", &runner, seen);

    session
        .dispatch_action(Action::Resources(ResourcesAction::SelectService(
            nuvem_domain::gcp::resource::ResourceServiceKind::ComputeInstances,
        )))
        .await;
    settle(&mut session).await;
    let _ = report("switch back to GCE", &runner, seen);
}
