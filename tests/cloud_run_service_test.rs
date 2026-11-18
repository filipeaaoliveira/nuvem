use std::cell::RefCell;

use nuvem::services::gcloud::cli::{CommandError, CommandOutput, CommandRunner, GcloudCommand};
use nuvem::services::resources::cloud_run::GcloudCloudRunService;
use nuvem_domain::gcp::configuration::{GcloudConfigurationName, GcloudContext};
use nuvem_domain::gcp::resource::{CloudRunService, CloudRunServiceStatus};

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

fn context_with_project() -> GcloudContext {
    GcloudContext {
        configuration: GcloudConfigurationName::new("dev"),
        account: Some("dev@example.com".to_string()),
        project: Some("dev-project".to_string()),
        region: None,
        zone: None,
    }
}

fn context_without_project() -> GcloudContext {
    GcloudContext {
        configuration: GcloudConfigurationName::new("dev"),
        account: Some("dev@example.com".to_string()),
        project: None,
        region: None,
        zone: None,
    }
}

fn service_row() -> CloudRunService {
    CloudRunService {
        name: "orders-api".to_string(),
        region: "europe-west1".to_string(),
        url: Some("https://orders-api-abcde-ew.a.run.app".to_string()),
        latest_ready_revision: Some("orders-api-00007-tabc".to_string()),
        latest_created_revision: Some("orders-api-00007-tabc".to_string()),
        traffic_summary: "orders-api-00007-tabc 100%".to_string(),
        status: CloudRunServiceStatus::Ready,
        ready_message: None,
    }
}

fn args(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| value.to_string()).collect()
}

fn ok(stdout: &str) -> Result<CommandOutput, CommandError> {
    Ok(CommandOutput {
        stdout: stdout.to_string(),
        stderr: String::new(),
    })
}

#[test]
fn load_services_lists_across_all_regions_with_project() {
    let runner = FakeRunner::default();
    runner
        .outputs
        .borrow_mut()
        .push(ok(include_str!("fixtures/cloud_run_services.json")));
    let service = GcloudCloudRunService::new(runner);

    let services = service
        .load_services(&context_with_project())
        .expect("services should load");

    assert_eq!(services.len(), 5);
    assert_eq!(
        service.runner().commands.borrow()[0].args,
        args(&[
            "--configuration",
            "dev",
            "--project",
            "dev-project",
            "run",
            "services",
            "list",
            "--platform=managed",
            "--format=json",
            "--limit=500",
        ])
    );
}

#[test]
fn load_services_omits_project_when_absent() {
    let runner = FakeRunner::default();
    runner
        .outputs
        .borrow_mut()
        .push(ok(include_str!("fixtures/cloud_run_services.json")));
    let service = GcloudCloudRunService::new(runner);

    service
        .load_services(&context_without_project())
        .expect("services should load");

    assert_eq!(
        service.runner().commands.borrow()[0].args,
        args(&[
            "--configuration",
            "dev",
            "run",
            "services",
            "list",
            "--platform=managed",
            "--format=json",
            "--limit=500",
        ])
    );
}

#[test]
fn load_service_detail_targets_the_services_own_region() {
    let runner = FakeRunner::default();
    runner
        .outputs
        .borrow_mut()
        .push(ok(include_str!("fixtures/cloud_run_service_detail.json")));
    let service = GcloudCloudRunService::new(runner);

    service
        .load_service_detail(&context_with_project(), &service_row())
        .expect("detail should load");

    assert_eq!(
        service.runner().commands.borrow()[0].args,
        args(&[
            "--configuration",
            "dev",
            "--project",
            "dev-project",
            "run",
            "services",
            "describe",
            "orders-api",
            "--platform=managed",
            "--region",
            "europe-west1",
            "--format=json",
        ])
    );
}

#[test]
fn load_revisions_scopes_to_service_and_region() {
    let runner = FakeRunner::default();
    runner
        .outputs
        .borrow_mut()
        .push(ok(include_str!("fixtures/cloud_run_revisions.json")));
    let service = GcloudCloudRunService::new(runner);

    service
        .load_revisions(&context_with_project(), &service_row())
        .expect("revisions should load");

    assert_eq!(
        service.runner().commands.borrow()[0].args,
        args(&[
            "--configuration",
            "dev",
            "--project",
            "dev-project",
            "run",
            "revisions",
            "list",
            "--platform=managed",
            "--region",
            "europe-west1",
            "--service",
            "orders-api",
            "--format=json",
        ])
    );
}

#[test]
fn load_revision_detail_uses_supplied_region_without_project() {
    let runner = FakeRunner::default();
    runner
        .outputs
        .borrow_mut()
        .push(ok(include_str!("fixtures/cloud_run_revision_detail.json")));
    let service = GcloudCloudRunService::new(runner);

    service
        .load_revision_detail(
            &context_without_project(),
            "europe-west1",
            "orders-api-00007-tabc",
        )
        .expect("revision detail should load");

    assert_eq!(
        service.runner().commands.borrow()[0].args,
        args(&[
            "--configuration",
            "dev",
            "run",
            "revisions",
            "describe",
            "orders-api-00007-tabc",
            "--platform=managed",
            "--region",
            "europe-west1",
            "--format=json",
        ])
    );
}

#[test]
fn load_services_surfaces_gcloud_failures() {
    let runner = FakeRunner::default();
    runner.outputs.borrow_mut().push(Err(CommandError::Failed {
        stdout: String::new(),
        stderr: "Cloud Run Admin API disabled".to_string(),
        exit_code: None,
    }));
    let service = GcloudCloudRunService::new(runner);

    let error = service
        .load_services(&context_with_project())
        .expect_err("gcloud failure should surface");

    assert!(error.to_string().contains("API disabled"));
}

#[test]
fn load_service_detail_surfaces_parse_failures() {
    let runner = FakeRunner::default();
    runner.outputs.borrow_mut().push(ok("{not-json"));
    let service = GcloudCloudRunService::new(runner);

    let error = service
        .load_service_detail(&context_with_project(), &service_row())
        .expect_err("parse failure should surface");

    assert!(error.to_string().contains("invalid Cloud Run"));
}
