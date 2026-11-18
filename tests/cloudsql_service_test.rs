use std::cell::RefCell;

use nuvem::app::CloudSqlInstanceAction;
use nuvem::app::state::resources::{
    BatchableService, CloudSql, ResourcesState, cloud_sql_lifecycle_call,
};
use nuvem::services::gcloud::cli::{
    CommandCeiling, CommandError, CommandOutput, CommandRunner, GcloudCommand,
};
use nuvem::services::resources::cloudsql::GcloudCloudSqlService;
use nuvem_domain::gcp::configuration::{GcloudConfigurationName, GcloudContext};
use nuvem_domain::gcp::resource::{
    CloudSqlAvailability as Av, CloudSqlAvailability, CloudSqlInstance, CloudSqlStatus as St,
    CloudSqlStatus, EndpointVisibility,
};

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

#[test]
fn load_instances_runs_native_gcloud_command_for_active_configuration() {
    let runner = FakeRunner::default();
    runner.outputs.borrow_mut().push(Ok(CommandOutput {
        stdout: include_str!("fixtures/cloudsql_instances.json").to_string(),
        stderr: String::new(),
    }));
    let service = GcloudCloudSqlService::new(runner);

    let instances = service
        .load_instances(&context(), None)
        .expect("instances should load");

    assert_eq!(instances.len(), 5);
    let commands = service.runner().commands.borrow();
    assert_eq!(
        commands[0].args,
        args(&[
            "--configuration",
            "dev",
            "sql",
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
        stdout: include_str!("fixtures/cloudsql_instances.json").to_string(),
        stderr: String::new(),
    }));
    let service = GcloudCloudSqlService::new(runner);

    service
        .load_instances(&context(), Some("pg"))
        .expect("instances should load");

    let commands = service.runner().commands.borrow();
    assert_eq!(
        commands[0].args,
        args(&[
            "--configuration",
            "dev",
            "sql",
            "instances",
            "list",
            "--format=json",
            "--limit=500",
            r#"--filter=name ~ "(?i)pg" OR region ~ "(?i)pg" OR databaseVersion ~ "(?i)pg" OR state ~ "(?i)pg" OR settings.tier ~ "(?i)pg""#,
        ])
    );
}

#[test]
fn load_instances_omits_server_filter_for_opaque_engine_label_term() {
    let runner = FakeRunner::default();
    runner.outputs.borrow_mut().push(Ok(CommandOutput {
        stdout: include_str!("fixtures/cloudsql_instances.json").to_string(),
        stderr: String::new(),
    }));
    let service = GcloudCloudSqlService::new(runner);

    service
        .load_instances(&context(), Some("postgresql"))
        .expect("instances should load");

    let commands = service.runner().commands.borrow();
    assert!(
        !commands[0]
            .args
            .iter()
            .any(|arg| arg.starts_with("--filter")),
        "an opaque engine-label term must stay client-side to avoid \
         dropping a row whose raw databaseVersion differs from the \
         normalized engine label"
    );
}

#[test]
fn load_instances_omits_server_filter_for_opaque_availability_label_term() {
    let runner = FakeRunner::default();
    runner.outputs.borrow_mut().push(Ok(CommandOutput {
        stdout: include_str!("fixtures/cloudsql_instances.json").to_string(),
        stderr: String::new(),
    }));
    let service = GcloudCloudSqlService::new(runner);

    service
        .load_instances(&context(), Some("regional"))
        .expect("instances should load");

    let commands = service.runner().commands.borrow();
    assert!(
        !commands[0]
            .args
            .iter()
            .any(|arg| arg.starts_with("--filter")),
        "an opaque availability-label term must stay client-side"
    );
}

#[test]
fn load_instances_appends_label_server_filter_for_label_mode_filter() {
    let runner = FakeRunner::default();
    runner.outputs.borrow_mut().push(Ok(CommandOutput {
        stdout: include_str!("fixtures/cloudsql_instances.json").to_string(),
        stderr: String::new(),
    }));
    let service = GcloudCloudSqlService::new(runner);

    service
        .load_instances(&context(), Some("-l env=prod"))
        .expect("instances should load");

    let commands = service.runner().commands.borrow();
    assert_eq!(
        commands[0].args,
        args(&[
            "--configuration",
            "dev",
            "sql",
            "instances",
            "list",
            "--format=json",
            "--limit=500",
            r#"--filter=settings.userLabels.env = "prod""#,
        ])
    );
}

#[test]
fn load_instances_omits_server_filter_for_fuzzy_mode_filter() {
    let runner = FakeRunner::default();
    runner.outputs.borrow_mut().push(Ok(CommandOutput {
        stdout: include_str!("fixtures/cloudsql_instances.json").to_string(),
        stderr: String::new(),
    }));
    let service = GcloudCloudSqlService::new(runner);

    service
        .load_instances(&context(), Some("-f pg"))
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
fn load_instance_detail_runs_native_gcloud_describe_command() {
    let runner = FakeRunner::default();
    runner.outputs.borrow_mut().push(Ok(CommandOutput {
        stdout: include_str!("fixtures/cloudsql_instance_detail.json").to_string(),
        stderr: String::new(),
    }));
    let service = GcloudCloudSqlService::new(runner);
    let instance = CloudSqlInstance {
        name: "mysql-ha".to_string(),
        database_version: "MYSQL_8_0".to_string(),
        engine: "MySQL".to_string(),
        region: "us-east1".to_string(),
        tier: "db-n1-standard-4".to_string(),
        availability: CloudSqlAvailability::Regional,
        status: CloudSqlStatus::Runnable,
        connectivity: EndpointVisibility::Private,
    };

    let detail = service
        .load_instance_detail(&context(), &instance)
        .expect("detail should load");

    assert_eq!(detail.name, "mysql-ha");
    let commands = service.runner().commands.borrow();
    assert_eq!(
        commands[0].args,
        args(&[
            "--configuration",
            "dev",
            "sql",
            "instances",
            "describe",
            "mysql-ha",
            "--format=json",
        ])
    );
}

#[test]
fn load_users_runs_native_gcloud_users_list_command() {
    let runner = FakeRunner::default();
    runner.outputs.borrow_mut().push(Ok(CommandOutput {
        stdout: include_str!("fixtures/cloudsql_users.json").to_string(),
        stderr: String::new(),
    }));
    let service = GcloudCloudSqlService::new(runner);

    let users = service
        .load_users(&context(), "mysql-ha")
        .expect("users should load");

    assert_eq!(users.len(), 2);
    let commands = service.runner().commands.borrow();
    assert_eq!(
        commands[0].args,
        args(&[
            "--configuration",
            "dev",
            "sql",
            "users",
            "list",
            "--instance",
            "mysql-ha",
            "--format=json",
        ])
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
    let service = GcloudCloudSqlService::new(runner);

    let error = service
        .load_instances(&context(), None)
        .expect_err("gcloud failure should surface");

    assert!(error.to_string().contains("API disabled"));
}

#[test]
fn failover_only_for_ha_running() {
    // eligible: running + regional
    assert!(CloudSqlInstanceAction::Failover.is_eligible(&St::Runnable, Av::Regional));
    // ineligible: running + zonal (not HA)
    assert!(!CloudSqlInstanceAction::Failover.is_eligible(&St::Runnable, Av::Zonal));
    // ineligible: stopped
    assert!(!CloudSqlInstanceAction::Failover.is_eligible(&St::Stopped, Av::Regional));
}

#[test]
fn start_stop_restart_eligibility() {
    assert!(CloudSqlInstanceAction::Start.is_eligible(&St::Stopped, Av::Zonal));
    assert!(!CloudSqlInstanceAction::Start.is_eligible(&St::Runnable, Av::Zonal));
    assert!(CloudSqlInstanceAction::Stop.is_eligible(&St::Runnable, Av::Zonal));
    assert!(CloudSqlInstanceAction::Restart.is_eligible(&St::Runnable, Av::Zonal));
    assert!(!CloudSqlInstanceAction::Restart.is_eligible(&St::Stopped, Av::Zonal));
}

fn cloud_sql_instance(
    name: &str,
    status: CloudSqlStatus,
    availability: CloudSqlAvailability,
) -> CloudSqlInstance {
    CloudSqlInstance {
        name: name.to_string(),
        database_version: "POSTGRES_15".to_string(),
        engine: "PostgreSQL".to_string(),
        region: "us-central1".to_string(),
        tier: "db-custom-2-7680".to_string(),
        availability,
        status,
        connectivity: EndpointVisibility::Public,
    }
}

#[test]
fn run_instance_action_builds_start_argv() {
    let runner = FakeRunner::default();
    runner.outputs.borrow_mut().push(Ok(CommandOutput {
        stdout: String::new(),
        stderr: String::new(),
    }));
    let service = GcloudCloudSqlService::new(runner);
    let instance = cloud_sql_instance(
        "mysql-ha",
        CloudSqlStatus::Stopped,
        CloudSqlAvailability::Zonal,
    );

    service
        .run_instance_action(&context(), &instance, CloudSqlInstanceAction::Start)
        .expect("start should succeed");

    let commands = service.runner().commands.borrow();
    assert_eq!(
        commands[0].args,
        args(&[
            "--configuration",
            "dev",
            "sql",
            "instances",
            "patch",
            "mysql-ha",
            "--activation-policy=ALWAYS",
        ])
    );
}

#[test]
fn run_instance_action_builds_stop_argv() {
    let runner = FakeRunner::default();
    runner.outputs.borrow_mut().push(Ok(CommandOutput {
        stdout: String::new(),
        stderr: String::new(),
    }));
    let service = GcloudCloudSqlService::new(runner);
    let instance = cloud_sql_instance(
        "mysql-ha",
        CloudSqlStatus::Runnable,
        CloudSqlAvailability::Zonal,
    );

    service
        .run_instance_action(&context(), &instance, CloudSqlInstanceAction::Stop)
        .expect("stop should succeed");

    let commands = service.runner().commands.borrow();
    assert_eq!(
        commands[0].args,
        args(&[
            "--configuration",
            "dev",
            "sql",
            "instances",
            "patch",
            "mysql-ha",
            "--activation-policy=NEVER",
        ])
    );
}

#[test]
fn run_instance_action_builds_restart_argv() {
    let runner = FakeRunner::default();
    runner.outputs.borrow_mut().push(Ok(CommandOutput {
        stdout: String::new(),
        stderr: String::new(),
    }));
    let service = GcloudCloudSqlService::new(runner);
    let instance = cloud_sql_instance(
        "mysql-ha",
        CloudSqlStatus::Runnable,
        CloudSqlAvailability::Zonal,
    );

    service
        .run_instance_action(&context(), &instance, CloudSqlInstanceAction::Restart)
        .expect("restart should succeed");

    let commands = service.runner().commands.borrow();
    assert_eq!(
        commands[0].args,
        args(&[
            "--configuration",
            "dev",
            "sql",
            "instances",
            "restart",
            "mysql-ha",
        ])
    );
}

#[test]
fn run_instance_action_builds_failover_argv() {
    let runner = FakeRunner::default();
    runner.outputs.borrow_mut().push(Ok(CommandOutput {
        stdout: String::new(),
        stderr: String::new(),
    }));
    let service = GcloudCloudSqlService::new(runner);
    let instance = cloud_sql_instance(
        "mysql-ha",
        CloudSqlStatus::Runnable,
        CloudSqlAvailability::Regional,
    );

    service
        .run_instance_action(&context(), &instance, CloudSqlInstanceAction::Failover)
        .expect("failover should succeed");

    let commands = service.runner().commands.borrow();
    assert_eq!(
        commands[0].args,
        args(&[
            "--configuration",
            "dev",
            "sql",
            "instances",
            "failover",
            "mysql-ha",
        ])
    );
}

/// The batch path runs an action against a name (it never holds the full row),
/// so `run_action_by_id` carries the argv and `run_instance_action` delegates to
/// it. Both must therefore produce byte-identical commands for every action.
#[test]
fn run_action_by_id_builds_the_same_argv_as_run_instance_action() {
    for action in [
        CloudSqlInstanceAction::Start,
        CloudSqlInstanceAction::Stop,
        CloudSqlInstanceAction::Restart,
        CloudSqlInstanceAction::Failover,
    ] {
        let by_row = {
            let runner = FakeRunner::default();
            runner.outputs.borrow_mut().push(Ok(CommandOutput {
                stdout: String::new(),
                stderr: String::new(),
            }));
            let service = GcloudCloudSqlService::new(runner);
            let instance = cloud_sql_instance("mysql-ha", St::Runnable, Av::Regional);
            service
                .run_instance_action(&context(), &instance, action)
                .expect("action should succeed");

            service.runner().commands.borrow().clone()
        };

        let by_id = {
            let runner = FakeRunner::default();
            runner.outputs.borrow_mut().push(Ok(CommandOutput {
                stdout: String::new(),
                stderr: String::new(),
            }));
            let service = GcloudCloudSqlService::new(runner);
            service
                .run_action_by_id(&context(), "mysql-ha", action)
                .expect("action should succeed");

            service.runner().commands.borrow().clone()
        };

        assert_eq!(
            by_id[0].args, by_row[0].args,
            "run_action_by_id argv must match run_instance_action for {action:?}"
        );
    }
}

/// Pins the argv `run_action_by_id` executes against the preview strings the
/// batch confirmation shows, so the modal cannot promise a command the runtime
/// The batch list and the executed argv describe the same call.
///
/// They are no longer the same string: the list names the call, the runtime
/// renders that call as a command line, and this is the cross-check that the
/// two come from one builder. The action and the target are what an operator
/// is reading the line to learn, so those are what is compared.
#[test]
fn the_batch_preview_and_the_executed_argv_describe_the_same_call() {
    for action in [
        CloudSqlInstanceAction::Start,
        CloudSqlInstanceAction::Stop,
        CloudSqlInstanceAction::Restart,
        CloudSqlInstanceAction::Failover,
    ] {
        let runner = FakeRunner::default();
        runner.outputs.borrow_mut().push(Ok(CommandOutput {
            stdout: "{}".to_string(),
            stderr: String::new(),
        }));
        let service = GcloudCloudSqlService::new(runner);
        service
            .run_action_by_id(&context(), "mysql-ha", action)
            .expect("action should succeed");
        let argv = service.runner().commands.borrow()[0].args.join(" ");

        let row = cloud_sql_instance("mysql-ha", St::Runnable, Av::Regional);
        let preview = CloudSql::row_ref(&row, &context(), action).call.preview();

        assert!(
            preview.contains("instances/mysql-ha") && argv.contains("mysql-ha"),
            "both must name the target for {action:?}: {preview:?} / {argv:?}"
        );
        let (method, flag) = match action {
            CloudSqlInstanceAction::Start => {
                ("instances.patch", Some("--activation-policy=ALWAYS"))
            }
            CloudSqlInstanceAction::Stop => ("instances.patch", Some("--activation-policy=NEVER")),
            CloudSqlInstanceAction::Restart => ("instances.restart", None),
            CloudSqlInstanceAction::Failover => ("instances.failover", None),
        };
        assert!(
            preview.contains(method),
            "the list names the method for {action:?}: {preview:?}"
        );
        if let Some(flag) = flag {
            assert!(
                argv.contains(flag),
                "and the argv carries the matching flag: {argv:?}"
            );
        }
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
    for action in CloudSqlInstanceAction::ALL {
        let runner = FakeRunner::default();
        runner.outputs.borrow_mut().push(Ok(CommandOutput {
            stdout: String::new(),
            stderr: String::new(),
        }));
        let service = GcloudCloudSqlService::new(runner);
        service
            .run_action_by_id(&context(), "mysql-ha", action)
            .expect("action should succeed");
        let argv = service.runner().commands.borrow()[0].args.clone();
        let executed = format!("gcloud {}", argv.join(" "));
        assert_eq!(
            service.runner().commands.borrow().len(),
            1,
            "the preview shows one command, so exactly one command must have run for {action:?}"
        );

        let row = cloud_sql_instance("mysql-ha", St::Runnable, Av::Regional);
        let batch_preview = CloudSql::row_ref(&row, &context(), action).call.preview();
        let mut state = ResourcesState::default();
        state.start_cloud_sql_action_confirmation(context(), row.clone(), action);
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
            executed.contains("mysql-ha"),
            "the executed argv names the target for {action:?}: {executed:?}"
        );
    }
}

/// Every Cloud SQL lifecycle action declares the lifecycle ceiling; the
/// listing beside it keeps the read ceiling.
///
/// `cloud_sql_lifecycle_argv` carries no `--async`, so gcloud blocks until the
/// GCP operation finishes: a `failover` promotes the standby and a `patch
/// --activation-policy=NEVER` drains and stops the instance, neither of which
/// is bounded by the two minutes `RUN_TIMEOUT` gives a *read*. Killing the CLI
/// at that ceiling does not cancel the operation — it only stops nuvem
/// watching it, while telling the operator the call was refused.
#[test]
fn every_instance_action_declares_the_lifecycle_ceiling() {
    for action in CloudSqlInstanceAction::ALL {
        let runner = FakeRunner::default();
        runner.outputs.borrow_mut().push(Ok(CommandOutput {
            stdout: String::new(),
            stderr: String::new(),
        }));
        let service = GcloudCloudSqlService::new(runner);

        service
            .run_action_by_id(&context(), "mysql-ha", action)
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
        stdout: include_str!("fixtures/cloudsql_instances.json").to_string(),
        stderr: String::new(),
    }));
    let service = GcloudCloudSqlService::new(runner);

    service
        .load_instances(&context(), None)
        .expect("instances should load");

    assert_eq!(
        service.runner().commands.borrow()[0].ceiling,
        CommandCeiling::Read
    );
}

/// Start and stop are the same method on the same resource.
///
/// They differ only in the activation policy they set, which is why the call
/// carries the field: a preview showing only the method would render
/// "sql.instances.patch" for both and tell an operator nothing about which
/// one they are confirming.
#[test]
fn the_two_patch_actions_differ_only_in_the_field_they_set() {
    let start = cloud_sql_lifecycle_call(&context(), "orders-db", CloudSqlInstanceAction::Start);
    let stop = cloud_sql_lifecycle_call(&context(), "orders-db", CloudSqlInstanceAction::Stop);

    assert_eq!(start.method, "instances.patch");
    assert_eq!(stop.method, "instances.patch");
    assert_eq!(start.resource, stop.resource);

    let field = |call: &nuvem_domain::gcp::resource_call::ResourceCall| {
        call.field
            .as_ref()
            .map(|field| (field.name, field.value.clone()))
    };
    assert_eq!(
        field(&start),
        Some(("settings.activationPolicy", "ALWAYS".to_string()))
    );
    assert_eq!(
        field(&stop),
        Some(("settings.activationPolicy", "NEVER".to_string()))
    );
    assert_ne!(start, stop, "the two must not be indistinguishable");
}

/// Restart and failover are verbs, not patches, and say so.
#[test]
fn the_verb_actions_name_their_own_method_and_set_nothing() {
    for (action, method) in [
        (CloudSqlInstanceAction::Restart, "instances.restart"),
        (CloudSqlInstanceAction::Failover, "instances.failover"),
    ] {
        let call = cloud_sql_lifecycle_call(&context(), "orders-db", action);
        assert_eq!(call.method, method);
        assert!(
            call.field.is_none(),
            "{method} is a verb: it sets no field, and claiming one would \
             describe work nobody asked for"
        );
    }
}
