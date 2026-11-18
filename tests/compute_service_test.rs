use std::cell::RefCell;

use nuvem::app::state::resources::{
    BatchableService, Compute, ComputeInstanceAction, ResourcesState, compute_lifecycle_argv,
    compute_lifecycle_call,
};
use nuvem::services::gcloud::cli::{
    CommandCeiling, CommandError, CommandOutput, CommandRunner, GcloudCommand,
};
use nuvem::services::resources::compute::GcloudComputeService;
use nuvem_domain::gcp::configuration::{GcloudConfigurationName, GcloudContext};
use nuvem_domain::gcp::resource::{ComputeInstance, ResourceStatus};

#[derive(Default)]
struct FakeRunner {
    outputs: RefCell<Vec<Result<CommandOutput, CommandError>>>,
    commands: RefCell<Vec<GcloudCommand>>,
}

impl CommandRunner for FakeRunner {
    fn run(&self, command: GcloudCommand) -> Result<CommandOutput, CommandError> {
        self.commands.borrow_mut().push(command);
        self.outputs.borrow_mut().remove(0)
    }
}

fn context() -> GcloudContext {
    GcloudContext {
        configuration: GcloudConfigurationName::new("dev"),
        account: Some("dev@example.com".to_string()),
        project: Some("dev-project".to_string()),
        region: None,
        zone: None,
    }
}

fn args(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| value.to_string()).collect()
}

fn instance() -> ComputeInstance {
    ComputeInstance {
        id: "1001".to_string(),
        name: "api-dev-1".to_string(),
        zone: "europe-west1-b".to_string(),
        status: ResourceStatus::Running,
        machine_type: "e2-medium".to_string(),
        internal_ip: Some("10.0.0.4".to_string()),
        external_ip: Some("34.10.10.10".to_string()),
    }
}

fn instance_named(name: &str, zone: &str) -> ComputeInstance {
    ComputeInstance {
        id: format!("{name}-id"),
        name: name.to_string(),
        zone: zone.to_string(),
        status: ResourceStatus::Running,
        machine_type: "e2-medium".to_string(),
        internal_ip: None,
        external_ip: None,
    }
}

#[test]
fn load_instances_runs_native_gcloud_command_for_active_configuration() {
    let runner = FakeRunner::default();
    runner.outputs.borrow_mut().push(Ok(CommandOutput {
        stdout: include_str!("fixtures/gce_instances.json").to_string(),
        stderr: String::new(),
    }));
    let service = GcloudComputeService::new(runner);

    let instances = service
        .load_instances(&context(), None)
        .expect("instances should load");

    assert_eq!(instances.len(), 2);
    assert_eq!(instances[0].name, "api-dev-1");
    let commands = service.runner().commands.borrow();
    assert_eq!(
        commands[0].args,
        args(&[
            "--configuration",
            "dev",
            "compute",
            "instances",
            "list",
            "--format=json",
            "--limit=500",
        ])
    );
}

#[test]
fn load_instances_appends_server_filter_for_expressible_term() {
    let runner = FakeRunner::default();
    runner.outputs.borrow_mut().push(Ok(CommandOutput {
        stdout: include_str!("fixtures/gce_instances.json").to_string(),
        stderr: String::new(),
    }));
    let service = GcloudComputeService::new(runner);

    service
        .load_instances(&context(), Some("api"))
        .expect("instances should load");

    let commands = service.runner().commands.borrow();
    assert_eq!(
        commands[0].args,
        args(&[
            "--configuration",
            "dev",
            "compute",
            "instances",
            "list",
            "--format=json",
            "--limit=500",
            r#"--filter=name ~ "(?i)api" OR zone ~ "(?i)api" OR status ~ "(?i)api" OR machineType ~ "(?i)api" OR networkInterfaces.networkIP ~ "(?i)api" OR networkInterfaces.accessConfigs.natIP ~ "(?i)api""#,
        ])
    );
}

#[test]
fn load_instances_omits_server_filter_for_opaque_status_label_term() {
    let runner = FakeRunner::default();
    runner.outputs.borrow_mut().push(Ok(CommandOutput {
        stdout: include_str!("fixtures/gce_instances.json").to_string(),
        stderr: String::new(),
    }));
    let service = GcloudComputeService::new(runner);

    service
        .load_instances(&context(), Some("stopped"))
        .expect("instances should load");

    let commands = service.runner().commands.borrow();
    assert!(
        !commands[0]
            .args
            .iter()
            .any(|arg| arg.starts_with("--filter")),
        "an opaque status term must stay client-side to avoid dropping a \
         row whose raw status differs from its display label"
    );
}

#[test]
fn load_instances_appends_server_filter_for_ip_term() {
    let runner = FakeRunner::default();
    runner.outputs.borrow_mut().push(Ok(CommandOutput {
        stdout: include_str!("fixtures/gce_instances.json").to_string(),
        stderr: String::new(),
    }));
    let service = GcloudComputeService::new(runner);

    service
        .load_instances(&context(), Some("10.0"))
        .expect("instances should load");

    let commands = service.runner().commands.borrow();
    assert!(
        commands[0].args.iter().any(|arg| {
            arg.starts_with("--filter")
                && arg.contains(r#"networkInterfaces.networkIP ~ "(?i)10.0""#)
        }),
        "expected a --filter token containing the networkIP clause in {:?}",
        commands[0].args
    );
}

#[test]
fn load_instances_appends_label_server_filter_for_label_mode_filter() {
    let runner = FakeRunner::default();
    runner.outputs.borrow_mut().push(Ok(CommandOutput {
        stdout: include_str!("fixtures/gce_instances.json").to_string(),
        stderr: String::new(),
    }));
    let service = GcloudComputeService::new(runner);

    service
        .load_instances(&context(), Some("-l env=prod"))
        .expect("instances should load");

    let commands = service.runner().commands.borrow();
    assert_eq!(
        commands[0].args,
        args(&[
            "--configuration",
            "dev",
            "compute",
            "instances",
            "list",
            "--format=json",
            "--limit=500",
            r#"--filter=labels.env = "prod""#,
        ])
    );
}

#[test]
fn load_instances_omits_server_filter_for_fuzzy_mode_filter() {
    let runner = FakeRunner::default();
    runner.outputs.borrow_mut().push(Ok(CommandOutput {
        stdout: include_str!("fixtures/gce_instances.json").to_string(),
        stderr: String::new(),
    }));
    let service = GcloudComputeService::new(runner);

    service
        .load_instances(&context(), Some("-f api"))
        .expect("instances should load");

    let commands = service.runner().commands.borrow();
    assert!(
        !commands[0]
            .args
            .iter()
            .any(|arg| arg.starts_with("--filter")),
        "a fuzzy filter has no server-side equivalent and must stay client-side"
    );
}

#[test]
fn load_instances_omits_server_filter_for_non_expressible_term() {
    let runner = FakeRunner::default();
    runner.outputs.borrow_mut().push(Ok(CommandOutput {
        stdout: include_str!("fixtures/gce_instances.json").to_string(),
        stderr: String::new(),
    }));
    let service = GcloudComputeService::new(runner);

    service
        .load_instances(&context(), Some("api dev"))
        .expect("instances should load");

    let commands = service.runner().commands.borrow();
    assert!(
        !commands[0]
            .args
            .iter()
            .any(|arg| arg.starts_with("--filter"))
    );
}

#[test]
fn load_instances_surfaces_gcloud_failures() {
    let runner = FakeRunner::default();
    runner.outputs.borrow_mut().push(Err(CommandError::Failed {
        stdout: String::new(),
        stderr: "API disabled".to_string(),
        exit_code: None,
    }));
    let service = GcloudComputeService::new(runner);

    let error = service
        .load_instances(&context(), None)
        .expect_err("gcloud failure should surface");

    assert!(error.to_string().contains("API disabled"));
}

#[test]
fn load_instance_detail_runs_native_describe_command_for_selected_instance() {
    let runner = FakeRunner::default();
    runner.outputs.borrow_mut().push(Ok(CommandOutput {
        stdout: include_str!("fixtures/gce_instance_detail.json").to_string(),
        stderr: String::new(),
    }));
    let service = GcloudComputeService::new(runner);

    let detail = service
        .load_instance_detail(&context(), &instance())
        .expect("detail should load");

    assert_eq!(detail.name, "api-dev-1");
    let commands = service.runner().commands.borrow();
    assert_eq!(
        commands[0].args,
        args(&[
            "--configuration",
            "dev",
            "compute",
            "instances",
            "describe",
            "api-dev-1",
            "--zone",
            "europe-west1-b",
            "--format=json",
        ])
    );
}

#[test]
fn load_instance_detail_surfaces_parse_failures() {
    let runner = FakeRunner::default();
    runner.outputs.borrow_mut().push(Ok(CommandOutput {
        stdout: "{not-json".to_string(),
        stderr: String::new(),
    }));
    let service = GcloudComputeService::new(runner);

    let error = service
        .load_instance_detail(&context(), &instance())
        .expect_err("parse failure should surface");

    assert!(error.to_string().contains("invalid compute instance JSON"));
}

#[test]
fn start_instance_runs_native_gcloud_command_for_active_configuration() {
    let runner = FakeRunner::default();
    runner.outputs.borrow_mut().push(Ok(CommandOutput {
        stdout: String::new(),
        stderr: String::new(),
    }));
    let service = GcloudComputeService::new(runner);

    service
        .start_instance(&context(), &instance_named("api-dev-1", "europe-west1-b"))
        .expect("start should succeed");

    let commands = service.runner().commands.borrow();
    assert_eq!(
        commands[0].args,
        args(&[
            "--configuration",
            "dev",
            "compute",
            "instances",
            "start",
            "api-dev-1",
            "--zone",
            "europe-west1-b",
        ])
    );
}

#[test]
fn stop_instance_runs_native_gcloud_command_for_active_configuration() {
    let runner = FakeRunner::default();
    runner.outputs.borrow_mut().push(Ok(CommandOutput {
        stdout: String::new(),
        stderr: String::new(),
    }));
    let service = GcloudComputeService::new(runner);

    service
        .stop_instance(&context(), &instance_named("api-dev-1", "europe-west1-b"))
        .expect("stop should succeed");

    let commands = service.runner().commands.borrow();
    assert_eq!(
        commands[0].args,
        args(&[
            "--configuration",
            "dev",
            "compute",
            "instances",
            "stop",
            "api-dev-1",
            "--zone",
            "europe-west1-b",
        ])
    );
}

/// The batch path only carries a target's name and location, not a whole
/// `ComputeInstance`. `run_action_by_id` must issue byte-for-byte the argv
/// `run_instance_action` issues for the same instance.
#[test]
fn run_action_by_id_runs_the_same_argv_as_run_instance_action() {
    for action in [
        ComputeInstanceAction::Start,
        ComputeInstanceAction::Stop,
        ComputeInstanceAction::Reset,
        ComputeInstanceAction::Suspend,
        ComputeInstanceAction::Resume,
    ] {
        let by_instance = {
            let runner = FakeRunner::default();
            runner.outputs.borrow_mut().push(Ok(CommandOutput {
                stdout: String::new(),
                stderr: String::new(),
            }));
            let service = GcloudComputeService::new(runner);
            service
                .run_instance_action(&context(), &instance(), action)
                .expect("action should succeed");

            service.runner().commands.borrow()[0].args.clone()
        };

        let runner = FakeRunner::default();
        runner.outputs.borrow_mut().push(Ok(CommandOutput {
            stdout: String::new(),
            stderr: String::new(),
        }));
        let service = GcloudComputeService::new(runner);
        service
            .run_action_by_id(&context(), "api-dev-1", "europe-west1-b", action)
            .expect("action should succeed");

        assert_eq!(service.runner().commands.borrow()[0].args, by_instance);
    }
}

#[test]
fn new_instance_actions_run_exact_native_gcloud_commands() {
    for (action, verb) in [
        (ComputeInstanceAction::Reset, "reset"),
        (ComputeInstanceAction::Suspend, "suspend"),
        (ComputeInstanceAction::Resume, "resume"),
    ] {
        let runner = FakeRunner::default();
        runner.outputs.borrow_mut().push(Ok(CommandOutput {
            stdout: String::new(),
            stderr: String::new(),
        }));
        let service = GcloudComputeService::new(runner);

        service
            .run_instance_action(&context(), &instance(), action)
            .expect("action should succeed");

        assert_eq!(
            service.runner().commands.borrow()[0].args,
            args(&[
                "--configuration",
                "dev",
                "compute",
                "instances",
                verb,
                "api-dev-1",
                "--zone",
                "europe-west1-b",
            ])
        );
    }
}

/// Pins the argv `run_action_by_id` actually sends to the runner against both
/// rendered command previews (the single-row confirmation and the batch
/// `row_ref`), for every lifecycle action. Deriving the expected string from
/// the *captured* argv rather than a literal means re-inlining argv inside
/// `run_action_by_id` (bypassing the shared builder) turns this red, even if
/// a maintainer dutifully updates every argv-literal assertion elsewhere.
#[test]
fn run_action_by_id_argv_matches_the_rendered_command_previews() {
    for action in ComputeInstanceAction::ALL {
        let runner = FakeRunner::default();
        runner.outputs.borrow_mut().push(Ok(CommandOutput {
            stdout: String::new(),
            stderr: String::new(),
        }));
        let service = GcloudComputeService::new(runner);
        service
            .run_action_by_id(&context(), "api-dev-1", "europe-west1-b", action)
            .expect("action should succeed");
        let argv = service.runner().commands.borrow()[0].args.clone();
        let executed = format!("gcloud {}", argv.join(" "));
        assert_eq!(
            service.runner().commands.borrow().len(),
            1,
            "the preview shows one command, so exactly one command must have run for {action:?}"
        );

        let row = instance_named("api-dev-1", "europe-west1-b");
        let batch_preview = Compute::row_ref(&row, &context(), action).call.preview();
        let mut state = ResourcesState::default();
        state.start_action_confirmation(context(), row.clone(), action);
        let single_row_preview = state
            .action_confirmation()
            .expect("single-row confirmation open")
            .call()
            .preview();
        // The modal and the batch list must name the same call, and the argv
        // the runtime runs must be that call rendered as a command line. The
        // two are no longer the same string -- that is this phase -- so the
        // parity is asserted as agreement about the action and the target
        // rather than as byte equality.
        assert_eq!(
            single_row_preview, batch_preview,
            "the single-row modal and the batch list must promise the same \
             call for {action:?}"
        );
        // And the argv the runtime actually ran has to be that same call
        // rendered as a command line. Byte equality is gone by design; naming
        // the same target is the half that still has to hold, or the modal
        // and the process are describing different work.
        assert!(
            executed.contains("api-dev-1"),
            "the executed argv names the target for {action:?}: {executed:?}"
        );
    }
}

#[test]
fn new_instance_action_wrappers_delegate_to_exact_native_gcloud_commands() {
    for (action, verb) in [
        (ComputeInstanceAction::Reset, "reset"),
        (ComputeInstanceAction::Suspend, "suspend"),
        (ComputeInstanceAction::Resume, "resume"),
    ] {
        let runner = FakeRunner::default();
        runner.outputs.borrow_mut().push(Ok(CommandOutput {
            stdout: String::new(),
            stderr: String::new(),
        }));
        let service = GcloudComputeService::new(runner);

        match action {
            ComputeInstanceAction::Reset => service
                .reset_instance(&context(), &instance())
                .expect("reset should succeed"),
            ComputeInstanceAction::Suspend => service
                .suspend_instance(&context(), &instance())
                .expect("suspend should succeed"),
            ComputeInstanceAction::Resume => service
                .resume_instance(&context(), &instance())
                .expect("resume should succeed"),
            ComputeInstanceAction::Start | ComputeInstanceAction::Stop => unreachable!(),
        }

        assert_eq!(
            service.runner().commands.borrow()[0].args,
            args(&[
                "--configuration",
                "dev",
                "compute",
                "instances",
                verb,
                "api-dev-1",
                "--zone",
                "europe-west1-b",
            ])
        );
    }
}

/// Every GCE lifecycle action declares the lifecycle ceiling; the listing
/// beside it keeps the read ceiling.
///
/// `compute_lifecycle_argv` carries no `--async`, so gcloud blocks until the
/// GCP operation finishes. A large instance's `stop` waits out the guest's own
/// shutdown before the operation completes, which routinely outlives the two
/// minutes `RUN_TIMEOUT` gives a *read* — and being killed there is worse than
/// waiting: the operator is told the call was refused while the stop carries
/// on server-side. Walking `ALL` rather than a hand-listed set is
/// what makes a sixth action inherit the decision instead of the default.
#[test]
fn every_instance_action_declares_the_lifecycle_ceiling() {
    for action in ComputeInstanceAction::ALL {
        let runner = FakeRunner::default();
        runner.outputs.borrow_mut().push(Ok(CommandOutput {
            stdout: String::new(),
            stderr: String::new(),
        }));
        let service = GcloudComputeService::new(runner);

        service
            .run_action_by_id(&context(), "api-dev-1", "europe-west1-b", action)
            .expect("action should succeed");

        assert_eq!(
            service.runner().commands.borrow()[0].ceiling,
            CommandCeiling::Lifecycle,
            "{action:?} blocks on a GCP operation and must not be held to the read ceiling"
        );
    }
}

/// The other half of the same claim: a read is still a read. A fix that gave
/// every command the lifecycle ceiling would satisfy the walk above while
/// removing the bound on the fan-out `RUN_TIMEOUT` exists for.
#[test]
fn listing_instances_keeps_the_read_ceiling() {
    let runner = FakeRunner::default();
    runner.outputs.borrow_mut().push(Ok(CommandOutput {
        stdout: include_str!("fixtures/gce_instances.json").to_string(),
        stderr: String::new(),
    }));
    let service = GcloudComputeService::new(runner);

    service
        .load_instances(&context(), None)
        .expect("instances should load");

    assert_eq!(
        service.runner().commands.borrow()[0].ceiling,
        CommandCeiling::Read
    );
}

/// The call and the argv say the same thing.
///
/// This is the parity property moved up a level: the argv is a rendering of
/// the call, so a change to one that is not a change to the other shows up
/// here. Every verb is walked, because a per-verb mapping is exactly where an
/// arm gets forgotten.
#[test]
fn every_compute_verb_derives_its_argv_from_its_call() {
    for action in ComputeInstanceAction::ALL {
        let call = compute_lifecycle_call(&context(), "checkout-api-3", "europe-west4-b", action);
        let argv = compute_lifecycle_argv(&context(), "checkout-api-3", "europe-west4-b", action);

        assert_eq!(call.service, "compute");
        assert_eq!(
            call.method,
            format!("instances.{}", action.command_verb()),
            "the call's method is the verb the argv runs"
        );
        assert_eq!(
            call.resource, "projects/dev-project/zones/europe-west4-b/instances/checkout-api-3",
            "the resource path names the same instance the argv names"
        );
        assert!(
            call.field.is_none(),
            "a compute verb sets no field; only a patch does"
        );

        assert!(
            argv.contains(&action.command_verb().to_string()),
            "the argv still carries the verb it always did"
        );
        assert!(
            argv.contains(&"checkout-api-3".to_string())
                && argv.contains(&"europe-west4-b".to_string()),
            "and still names the instance and its zone"
        );
    }
}
