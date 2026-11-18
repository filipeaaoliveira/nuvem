use std::cell::RefCell;

use nuvem::services::gcloud::cli::{CommandError, CommandOutput, CommandRunner, GcloudCommand};
use nuvem::services::resources::storage::GcloudStorageService;
use nuvem_domain::gcp::configuration::{GcloudConfigurationName, GcloudContext};

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
fn load_buckets_runs_native_gcloud_command_for_active_configuration() {
    let runner = FakeRunner::default();
    runner.outputs.borrow_mut().push(Ok(CommandOutput {
        stdout: include_str!("fixtures/gcs_buckets.json").to_string(),
        stderr: String::new(),
    }));
    let service = GcloudStorageService::new(runner);

    let buckets = service
        .load_buckets(&context())
        .expect("buckets should load");

    assert_eq!(buckets.len(), 3);
    assert_eq!(buckets[0].name, "alpha-bucket");
    assert_eq!(
        service.runner().commands.borrow()[0].args,
        args(&[
            "--configuration",
            "dev",
            "storage",
            "buckets",
            "list",
            "--format=json",
        ])
    );
}

#[test]
fn load_bucket_detail_runs_native_describe_command_for_selected_bucket() {
    let runner = FakeRunner::default();
    runner.outputs.borrow_mut().push(Ok(CommandOutput {
        stdout: include_str!("fixtures/gcs_bucket_detail.json").to_string(),
        stderr: String::new(),
    }));
    let service = GcloudStorageService::new(runner);

    let detail = service
        .load_bucket_detail(&context(), "alpha-bucket")
        .expect("detail should load");

    assert_eq!(detail.name, "alpha-bucket");
    assert_eq!(
        service.runner().commands.borrow()[0].args,
        args(&[
            "--configuration",
            "dev",
            "storage",
            "buckets",
            "describe",
            "gs://alpha-bucket",
            "--format=json",
        ])
    );
}

/// The URL carries `**`: without it `objects list` matches only the objects
/// sitting directly at that path and reports no folders at all, so a bucket
/// whose contents are nested one level down lists as completely empty.
#[test]
fn load_object_page_lists_recursively_and_bounds_with_limit() {
    let runner = FakeRunner::default();
    runner.outputs.borrow_mut().push(Ok(CommandOutput {
        stdout: include_str!("fixtures/gcs_objects.json").to_string(),
        stderr: String::new(),
    }));
    let service = GcloudStorageService::new(runner);

    let page = service
        .load_object_page(&context(), "alpha-bucket", "", None, 50)
        .expect("page should load");

    assert_eq!(page.prefixes.len(), 3);
    assert_eq!(page.objects.len(), 5);
    assert_eq!(page.next_page_token, None);
    assert_eq!(
        service.runner().commands.borrow()[0].args,
        args(&[
            "--configuration",
            "dev",
            "storage",
            "objects",
            "list",
            "gs://alpha-bucket/**",
            "--format=json",
            "--limit=50",
        ])
    );
}

/// A page token is accepted and dropped. `gcloud storage objects list` takes
/// one as input but never emits one, so a token can only ever be a leftover;
/// putting it on the argv would send a token gcloud did not issue.
#[test]
fn load_object_page_ignores_a_page_token_it_can_never_have_been_given() {
    let runner = FakeRunner::default();
    runner.outputs.borrow_mut().push(Ok(CommandOutput {
        stdout: include_str!("fixtures/gcs_objects.json").to_string(),
        stderr: String::new(),
    }));
    let service = GcloudStorageService::new(runner);

    let page = service
        .load_object_page(&context(), "alpha-bucket", "reports/", Some("TOKEN2"), 50)
        .expect("page should load");

    assert_eq!(page.next_page_token, None);
    assert_eq!(
        service.runner().commands.borrow()[0].args,
        args(&[
            "--configuration",
            "dev",
            "storage",
            "objects",
            "list",
            "gs://alpha-bucket/reports/**",
            "--format=json",
            "--limit=50",
        ])
    );
}

#[test]
fn load_object_detail_runs_native_describe_command_for_selected_object() {
    let runner = FakeRunner::default();
    runner.outputs.borrow_mut().push(Ok(CommandOutput {
        stdout: include_str!("fixtures/gcs_object_detail.json").to_string(),
        stderr: String::new(),
    }));
    let service = GcloudStorageService::new(runner);

    let detail = service
        .load_object_detail(&context(), "alpha-bucket", "reports/2024/mañana-测试.txt")
        .expect("detail should load");

    assert_eq!(detail.key, "reports/2024/mañana-测试.txt");
    assert_eq!(
        service.runner().commands.borrow()[0].args,
        args(&[
            "--configuration",
            "dev",
            "storage",
            "objects",
            "describe",
            "gs://alpha-bucket/reports/2024/mañana-测试.txt",
            "--format=json",
        ])
    );
}

#[test]
fn load_buckets_surfaces_gcloud_failures() {
    let runner = FakeRunner::default();
    runner.outputs.borrow_mut().push(Err(CommandError::Failed {
        stdout: String::new(),
        stderr: "storage API disabled".to_string(),
        exit_code: Some(1),
    }));
    let service = GcloudStorageService::new(runner);

    let error = service
        .load_buckets(&context())
        .expect_err("gcloud failure should surface");

    assert!(error.to_string().contains("storage API disabled"));
}

#[test]
fn load_bucket_detail_surfaces_parse_failures() {
    let runner = FakeRunner::default();
    runner.outputs.borrow_mut().push(Ok(CommandOutput {
        stdout: "{not-json".to_string(),
        stderr: String::new(),
    }));
    let service = GcloudStorageService::new(runner);

    let error = service
        .load_bucket_detail(&context(), "alpha-bucket")
        .expect_err("parse failure should surface");

    assert!(error.to_string().contains("invalid Cloud Storage JSON"));
}
