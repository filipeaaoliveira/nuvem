use std::cell::RefCell;
use std::rc::Rc;

use nuvem::services::runtime::interactive::{
    InteractiveLifecycle, InteractiveProcessOutcome, InteractiveProcessRunner,
    InteractiveProcessSpec, execute_interactive_process, gce_ssh_spec, tmux_detected_in,
};
use nuvem_domain::gcp::configuration::{GcloudConfigurationName, GcloudContext};
use nuvem_domain::gcp::resource::{ComputeInstance, ResourceStatus};

fn context() -> GcloudContext {
    GcloudContext {
        configuration: GcloudConfigurationName::new("prod".to_string()),
        account: None,
        project: Some("my-project".to_string()),
        region: None,
        zone: Some("us-central1-a".to_string()),
    }
}

fn instance() -> ComputeInstance {
    ComputeInstance {
        id: "web-1-id".to_string(),
        name: "web-1".to_string(),
        zone: "us-central1-a".to_string(),
        status: ResourceStatus::Running,
        machine_type: "e2-medium".to_string(),
        internal_ip: None,
        external_ip: None,
    }
}

/// Fake runner that records the exact spec it was asked to run and returns a
/// scripted outcome, sharing an event log with the fake lifecycle.
struct FakeRunner {
    log: Rc<RefCell<Vec<String>>>,
    recorded: RefCell<Vec<InteractiveProcessSpec>>,
    outcome: InteractiveProcessOutcome,
}

impl InteractiveProcessRunner for FakeRunner {
    fn run(&self, spec: &InteractiveProcessSpec) -> InteractiveProcessOutcome {
        self.log
            .borrow_mut()
            .push(format!("run:{}", spec.args.join(" ")));
        self.recorded.borrow_mut().push(spec.clone());
        self.outcome.clone()
    }
}

struct FakeLifecycle {
    log: Rc<RefCell<Vec<String>>>,
}

impl InteractiveLifecycle for FakeLifecycle {
    fn before_launch(&mut self) {
        self.log.borrow_mut().push("before".to_string());
    }
    fn after_launch(&mut self) {
        self.log.borrow_mut().push("after".to_string());
    }
}

#[test]
fn gce_ssh_spec_builds_direct_argv() {
    let spec = gce_ssh_spec("gcloud", &context(), &instance());
    assert_eq!(spec.program, "gcloud");
    assert_eq!(
        spec.args,
        vec![
            "--configuration",
            "prod",
            "compute",
            "ssh",
            "web-1",
            "--zone",
            "us-central1-a",
        ]
    );
    assert_eq!(spec.description, "SSH to web-1");
}

#[test]
fn gce_ssh_spec_uses_configured_binary() {
    let spec = gce_ssh_spec("/opt/google-cloud-sdk/bin/gcloud", &context(), &instance());
    assert_eq!(spec.program, "/opt/google-cloud-sdk/bin/gcloud");
}

#[test]
fn execute_runs_lifecycle_around_runner_and_returns_outcome() {
    let log = Rc::new(RefCell::new(Vec::new()));
    let mut lifecycle = FakeLifecycle { log: log.clone() };
    let runner = FakeRunner {
        log: log.clone(),
        recorded: RefCell::new(Vec::new()),
        outcome: InteractiveProcessOutcome::Succeeded,
    };
    let spec = gce_ssh_spec("gcloud", &context(), &instance());

    let outcome = execute_interactive_process(&mut lifecycle, &runner, &spec);

    assert_eq!(outcome, InteractiveProcessOutcome::Succeeded);
    assert_eq!(
        *log.borrow(),
        vec![
            "before".to_string(),
            "run:--configuration prod compute ssh web-1 --zone us-central1-a".to_string(),
            "after".to_string(),
        ]
    );
    assert_eq!(runner.recorded.borrow().len(), 1);
}

#[test]
fn execute_propagates_every_outcome_variant() {
    for outcome in [
        InteractiveProcessOutcome::Succeeded,
        InteractiveProcessOutcome::ExitedNonZero { code: 255 },
        InteractiveProcessOutcome::TerminatedBySignal { signal: 9 },
        InteractiveProcessOutcome::MissingBinary,
        InteractiveProcessOutcome::LaunchFailed {
            message: "boom".to_string(),
        },
    ] {
        let log = Rc::new(RefCell::new(Vec::new()));
        let mut lifecycle = FakeLifecycle { log: log.clone() };
        let runner = FakeRunner {
            log: log.clone(),
            recorded: RefCell::new(Vec::new()),
            outcome: outcome.clone(),
        };
        let spec = gce_ssh_spec("gcloud", &context(), &instance());
        assert_eq!(
            execute_interactive_process(&mut lifecycle, &runner, &spec),
            outcome
        );
        // after_launch always runs even when the child failed.
        assert_eq!(log.borrow().last().map(String::as_str), Some("after"));
    }
}

#[test]
fn subprocess_runner_reports_missing_binary() {
    use nuvem::services::runtime::interactive::SubprocessRunner;
    let outcome = SubprocessRunner.run(&InteractiveProcessSpec {
        program: "definitely-not-a-real-binary-xyzzy".to_string(),
        args: vec![],
        description: "probe".to_string(),
    });
    assert_eq!(outcome, InteractiveProcessOutcome::MissingBinary);
}

#[test]
fn subprocess_runner_maps_success_and_nonzero() {
    use nuvem::services::runtime::interactive::SubprocessRunner;
    assert_eq!(
        SubprocessRunner.run(&InteractiveProcessSpec {
            program: "true".to_string(),
            args: vec![],
            description: "ok".to_string(),
        }),
        InteractiveProcessOutcome::Succeeded
    );
    assert_eq!(
        SubprocessRunner.run(&InteractiveProcessSpec {
            program: "false".to_string(),
            args: vec![],
            description: "fail".to_string(),
        }),
        InteractiveProcessOutcome::ExitedNonZero { code: 1 }
    );
}

#[test]
fn tmux_detected_reflects_env() {
    assert!(tmux_detected_in(Some("/tmp/tmux-1000/default,1234,0")));
    assert!(!tmux_detected_in(Some("")));
    assert!(!tmux_detected_in(None));
}
