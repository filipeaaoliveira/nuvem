use std::cell::RefCell;

use nuvem::services::gcloud::cli::{CommandError, CommandOutput, CommandRunner, GcloudCommand};
use nuvem::services::resources::gke::GcloudGkeService;
use nuvem_domain::gcp::configuration::{GcloudConfigurationName, GcloudContext};
use nuvem_domain::gcp::resource::{EndpointVisibility, GkeCluster, GkeClusterStatus};

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

fn cluster() -> GkeCluster {
    GkeCluster {
        name: "platform-dev".to_string(),
        location: "europe-west1".to_string(),
        status: GkeClusterStatus::Running,
        current_master_version: Some("1.31.5-gke.1023000".to_string()),
        current_node_count: Some(6),
        endpoint_visibility: EndpointVisibility::Public,
    }
}

fn args(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| value.to_string()).collect()
}

#[test]
fn load_clusters_runs_native_gcloud_command_for_active_configuration() {
    let runner = FakeRunner::default();
    runner.outputs.borrow_mut().push(Ok(CommandOutput {
        stdout: include_str!("fixtures/gke_clusters.json").to_string(),
        stderr: String::new(),
    }));
    let service = GcloudGkeService::new(runner);

    let clusters = service
        .load_clusters(&context(), None)
        .expect("clusters should load");

    assert_eq!(clusters.len(), 3);
    assert_eq!(clusters[0].name, "platform-dev");
    assert_eq!(
        service.runner().commands.borrow()[0].args,
        args(&[
            "--configuration",
            "dev",
            "container",
            "clusters",
            "list",
            "--format=json",
            "--limit=500",
        ])
    );
}

#[test]
fn load_clusters_appends_server_filter_for_expressible_term() {
    let runner = FakeRunner::default();
    runner.outputs.borrow_mut().push(Ok(CommandOutput {
        stdout: include_str!("fixtures/gke_clusters.json").to_string(),
        stderr: String::new(),
    }));
    let service = GcloudGkeService::new(runner);

    service
        .load_clusters(&context(), Some("prod"))
        .expect("clusters should load");

    assert_eq!(
        service.runner().commands.borrow()[0].args,
        args(&[
            "--configuration",
            "dev",
            "container",
            "clusters",
            "list",
            "--format=json",
            "--limit=500",
            r#"--filter=name ~ "(?i)prod" OR location ~ "(?i)prod" OR zone ~ "(?i)prod" OR status ~ "(?i)prod" OR currentMasterVersion ~ "(?i)prod""#,
        ])
    );
}

#[test]
fn load_clusters_omits_server_filter_for_opaque_endpoint_visibility_term() {
    let runner = FakeRunner::default();
    runner.outputs.borrow_mut().push(Ok(CommandOutput {
        stdout: include_str!("fixtures/gke_clusters.json").to_string(),
        stderr: String::new(),
    }));
    let service = GcloudGkeService::new(runner);

    service
        .load_clusters(&context(), Some("private"))
        .expect("clusters should load");

    assert!(
        !service.runner().commands.borrow()[0]
            .args
            .iter()
            .any(|arg| arg.starts_with("--filter")),
        "an opaque endpoint-visibility term must stay client-side"
    );
}

#[test]
fn load_clusters_omits_server_filter_for_purely_numeric_term() {
    let runner = FakeRunner::default();
    runner.outputs.borrow_mut().push(Ok(CommandOutput {
        stdout: include_str!("fixtures/gke_clusters.json").to_string(),
        stderr: String::new(),
    }));
    let service = GcloudGkeService::new(runner);

    service
        .load_clusters(&context(), Some("3"))
        .expect("clusters should load");

    assert!(
        !service.runner().commands.borrow()[0]
            .args
            .iter()
            .any(|arg| arg.starts_with("--filter")),
        "a purely numeric term must stay client-side (it could match the \
         derived node-count column, which has no ~-safe raw string field)"
    );
}

#[test]
fn load_clusters_omits_server_filter_for_numeric_pattern_that_can_match_a_node_count() {
    let runner = FakeRunner::default();
    runner.outputs.borrow_mut().push(Ok(CommandOutput {
        stdout: include_str!("fixtures/gke_clusters.json").to_string(),
        stderr: String::new(),
    }));
    let service = GcloudGkeService::new(runner);

    service
        .load_clusters(&context(), Some("1.27"))
        .expect("clusters should load");

    assert!(
        !service.runner().commands.borrow()[0]
            .args
            .iter()
            .any(|arg| arg.starts_with("--filter")),
        "`.` matches a digit, so a digits-and-dots pattern can still match \
         the derived node-count column and must stay client-side"
    );
}

#[test]
fn load_clusters_appends_label_server_filter_for_label_mode_filter() {
    let runner = FakeRunner::default();
    runner.outputs.borrow_mut().push(Ok(CommandOutput {
        stdout: include_str!("fixtures/gke_clusters.json").to_string(),
        stderr: String::new(),
    }));
    let service = GcloudGkeService::new(runner);

    service
        .load_clusters(&context(), Some("-l env=prod"))
        .expect("clusters should load");

    assert_eq!(
        service.runner().commands.borrow()[0].args,
        args(&[
            "--configuration",
            "dev",
            "container",
            "clusters",
            "list",
            "--format=json",
            "--limit=500",
            r#"--filter=resourceLabels.env = "prod""#,
        ])
    );
}

#[test]
fn load_clusters_omits_server_filter_for_fuzzy_mode_filter() {
    let runner = FakeRunner::default();
    runner.outputs.borrow_mut().push(Ok(CommandOutput {
        stdout: include_str!("fixtures/gke_clusters.json").to_string(),
        stderr: String::new(),
    }));
    let service = GcloudGkeService::new(runner);

    service
        .load_clusters(&context(), Some("-f prod"))
        .expect("clusters should load");

    assert!(
        !service.runner().commands.borrow()[0]
            .args
            .iter()
            .any(|arg| arg.starts_with("--filter")),
        "a fuzzy filter has no server-side equivalent and must stay client-side"
    );
}

#[test]
fn load_cluster_detail_runs_native_describe_command_for_selected_cluster() {
    let runner = FakeRunner::default();
    runner.outputs.borrow_mut().push(Ok(CommandOutput {
        stdout: include_str!("fixtures/gke_cluster_detail.json").to_string(),
        stderr: String::new(),
    }));
    let service = GcloudGkeService::new(runner);

    let detail = service
        .load_cluster_detail(&context(), &cluster())
        .expect("detail should load");

    assert_eq!(detail.name, "platform-dev");
    assert_eq!(
        service.runner().commands.borrow()[0].args,
        args(&[
            "--configuration",
            "dev",
            "container",
            "clusters",
            "describe",
            "platform-dev",
            "--location",
            "europe-west1",
            "--format=json",
        ])
    );
}

#[test]
fn load_clusters_surfaces_gcloud_failures() {
    let runner = FakeRunner::default();
    runner.outputs.borrow_mut().push(Err(CommandError::Failed {
        stdout: String::new(),
        stderr: "container API disabled".to_string(),
        exit_code: None,
    }));
    let service = GcloudGkeService::new(runner);

    let error = service
        .load_clusters(&context(), None)
        .expect_err("gcloud failure should surface");

    assert!(error.to_string().contains("container API disabled"));
}

#[test]
fn load_cluster_detail_surfaces_parse_failures() {
    let runner = FakeRunner::default();
    runner.outputs.borrow_mut().push(Ok(CommandOutput {
        stdout: "{not-json".to_string(),
        stderr: String::new(),
    }));
    let service = GcloudGkeService::new(runner);

    let error = service
        .load_cluster_detail(&context(), &cluster())
        .expect_err("parse failure should surface");

    assert!(error.to_string().contains("invalid GKE cluster JSON"));
}
