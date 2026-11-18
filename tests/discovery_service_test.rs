use std::cell::RefCell;

use nuvem::services::gcloud::cli::{CommandError, CommandOutput, CommandRunner, GcloudCommand};
use nuvem::services::gcloud::discovery::{GcloudDiscoveryError, GcloudDiscoveryService};
use nuvem_domain::gcp::configuration::GcloudConfigurationName;

#[derive(Default)]
struct FakeRunner {
    outputs: RefCell<Vec<Result<CommandOutput, CommandError>>>,
    commands: RefCell<Vec<GcloudCommand>>,
}

impl FakeRunner {
    fn with_outputs(outputs: Vec<Result<CommandOutput, CommandError>>) -> Self {
        Self {
            outputs: RefCell::new(outputs),
            commands: RefCell::new(Vec::new()),
        }
    }
}

impl CommandRunner for FakeRunner {
    fn run(&self, command: GcloudCommand) -> Result<CommandOutput, CommandError> {
        self.commands.borrow_mut().push(command);
        self.outputs.borrow_mut().remove(0)
    }
}

fn output(stdout: impl Into<String>) -> Result<CommandOutput, CommandError> {
    Ok(CommandOutput {
        stdout: stdout.into(),
        stderr: String::new(),
    })
}

fn configuration() -> GcloudConfigurationName {
    GcloudConfigurationName::new("dev")
}

#[test]
fn load_projects_uses_active_configuration_and_parses_project_fields() {
    let runner =
        FakeRunner::with_outputs(vec![output(include_str!("fixtures/gcloud_projects.json"))]);
    let service = GcloudDiscoveryService::new(runner);

    let projects = service
        .load_projects(&configuration())
        .expect("projects should parse");

    assert_eq!(projects[0].project_id, "dev-project");
    assert_eq!(projects[0].name, "Development");
    assert_eq!(projects[0].lifecycle_state.as_deref(), Some("ACTIVE"));
    assert_eq!(
        service.runner().commands.borrow()[0].args,
        vec![
            "--configuration",
            "dev",
            "projects",
            "list",
            "--format=json",
        ]
    );
}

#[test]
fn load_regions_uses_active_configuration_and_parses_status() {
    let runner = FakeRunner::with_outputs(vec![output(include_str!(
        "fixtures/gcloud_compute_regions.json"
    ))]);
    let service = GcloudDiscoveryService::new(runner);

    let regions = service
        .load_regions(&configuration())
        .expect("regions should parse");

    assert_eq!(regions[0].name, "europe-west1");
    assert_eq!(regions[0].status.as_deref(), Some("UP"));
    assert_eq!(
        service.runner().commands.borrow()[0].args,
        vec![
            "--configuration",
            "dev",
            "compute",
            "regions",
            "list",
            "--format=json",
        ]
    );
}

#[test]
fn load_zones_uses_active_configuration_and_reduces_region_url() {
    let runner = FakeRunner::with_outputs(vec![output(include_str!(
        "fixtures/gcloud_compute_zones.json"
    ))]);
    let service = GcloudDiscoveryService::new(runner);

    let zones = service
        .load_zones(&configuration())
        .expect("zones should parse");

    assert_eq!(zones[0].name, "europe-west1-b");
    assert_eq!(zones[0].region.as_deref(), Some("europe-west1"));
    assert_eq!(zones[0].status.as_deref(), Some("UP"));
    assert_eq!(
        service.runner().commands.borrow()[0].args,
        vec![
            "--configuration",
            "dev",
            "compute",
            "zones",
            "list",
            "--format=json",
        ]
    );
}

#[test]
fn empty_discovery_arrays_parse_as_empty_vectors() {
    let runner = FakeRunner::with_outputs(vec![output("[]"), output("[]"), output("[]")]);
    let service = GcloudDiscoveryService::new(runner);
    let configuration = configuration();

    assert!(service.load_projects(&configuration).unwrap().is_empty());
    assert!(service.load_regions(&configuration).unwrap().is_empty());
    assert!(service.load_zones(&configuration).unwrap().is_empty());
}

#[test]
fn malformed_discovery_json_returns_parse_error() {
    let runner = FakeRunner::with_outputs(vec![output("not-json")]);
    let service = GcloudDiscoveryService::new(runner);

    let error = service
        .load_projects(&configuration())
        .expect_err("malformed JSON should fail");

    assert!(matches!(error, GcloudDiscoveryError::Parse(_)));
}

#[test]
fn discovery_command_failure_returns_command_error() {
    let runner = FakeRunner::with_outputs(vec![Err(CommandError::Failed {
        stdout: String::new(),
        stderr: "permission denied".to_string(),
        exit_code: None,
    })]);
    let service = GcloudDiscoveryService::new(runner);

    let error = service
        .load_zones(&configuration())
        .expect_err("command failure should fail");

    assert!(matches!(error, GcloudDiscoveryError::Command(_)));
}
