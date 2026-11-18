//! Secret Manager service layer: the argv nuvem issues, and what it makes of
//! the JSON that comes back.
//!
//! The argv assertions here are not incidental. Secret Manager is the one
//! service where *which subcommands exist* is the feature: see
//! `secrets_no_payload_access_test.rs` for the guarantee they serve.

use nuvem::services::gcloud::cli::{CommandError, CommandOutput, CommandRunner, GcloudCommand};
use nuvem::services::resources::secrets::GcloudSecretsService;
use nuvem_domain::gcp::configuration::{GcloudConfigurationName, GcloudContext};
use nuvem_domain::gcp::resource::SecretVersionState;
use std::cell::RefCell;

fn ctx() -> GcloudContext {
    GcloudContext {
        configuration: GcloudConfigurationName::new("dev"),
        account: None,
        project: Some("dev-project".into()),
        region: None,
        zone: None,
    }
}

/// Answers every invocation with one canned payload and records the argv it
/// was asked to run, so a test can assert on both halves.
struct RecordingRunner {
    stdout: String,
    seen: RefCell<Vec<Vec<String>>>,
}

impl RecordingRunner {
    fn new(stdout: &str) -> Self {
        Self {
            stdout: stdout.to_string(),
            seen: RefCell::new(Vec::new()),
        }
    }

    fn argv(&self) -> Vec<String> {
        self.seen
            .borrow()
            .last()
            .cloned()
            .expect("a command was run")
    }
}

impl CommandRunner for RecordingRunner {
    fn run(&self, command: GcloudCommand) -> Result<CommandOutput, CommandError> {
        self.seen.borrow_mut().push(command.args.clone());
        Ok(CommandOutput {
            stdout: self.stdout.clone(),
            stderr: String::new(),
        })
    }
}

#[test]
fn secrets_list_parses_name_create_time_labels_and_replication() {
    let runner = RecordingRunner::new(include_str!("fixtures/secrets.json"));
    let service = GcloudSecretsService::new(runner);

    let secrets = service
        .load_secrets(&ctx(), None)
        .expect("the fixture parses");

    assert_eq!(secrets.len(), 3, "the fixture holds three secrets");

    // The short id, not the fully-qualified resource path: the resource path is
    // what the API returns and what nobody wants to read in a table cell.
    assert_eq!(secrets[0].name, "api-signing-key");
    assert_eq!(
        secrets[0].create_time.as_deref(),
        Some("2026-02-11T09:14:22Z")
    );
    assert_eq!(
        secrets[0]
            .labels
            .iter()
            .map(|label| (label.key.as_str(), label.value.as_str()))
            .collect::<Vec<_>>(),
        vec![("env", "prod"), ("owner", "platform")],
        "labels are sorted by key so the detail pane does not reshuffle per load"
    );

    assert_eq!(secrets[0].replication, "Automatic");
    assert_eq!(
        secrets[1].replication, "User-managed (europe-west1, europe-west4)",
        "a user-managed policy names its replica locations, which is the part an \
         operator is actually checking"
    );

    assert!(
        secrets[2].labels.is_empty(),
        "a secret with no labels parses to no labels rather than failing"
    );
}

#[test]
fn secret_versions_list_parses_state_including_an_unrecognised_one() {
    let runner = RecordingRunner::new(include_str!("fixtures/secret_versions.json"));
    let service = GcloudSecretsService::new(runner);

    let versions = service
        .load_secret_versions(&ctx(), "api-signing-key")
        .expect("the fixture parses");

    assert_eq!(versions.len(), 4);

    // The version id, not the resource path, for the same reason as the name.
    assert_eq!(versions[0].name, "3");
    assert_eq!(versions[0].state, SecretVersionState::Enabled);
    assert_eq!(versions[1].state, SecretVersionState::Disabled);
    assert_eq!(versions[2].state, SecretVersionState::Destroyed);
    assert_eq!(
        versions[3].state,
        SecretVersionState::Unknown("STATE_UNSPECIFIED".into()),
        "an unrecognised state is carried through verbatim rather than \
         collapsed into one of the known ones"
    );
}

#[test]
fn secret_describe_parses_rotation_and_annotations() {
    let runner = RecordingRunner::new(include_str!("fixtures/secret_detail.json"));
    let service = GcloudSecretsService::new(runner);

    let detail = service
        .load_secret_detail(&ctx(), "api-signing-key")
        .expect("the fixture parses");

    assert_eq!(detail.name, "api-signing-key");
    assert_eq!(
        detail.rotation.as_deref(),
        Some("every 7776000s, next 2026-08-11T09:14:22Z")
    );
    assert_eq!(detail.expire_time.as_deref(), Some("2027-02-11T09:14:22Z"));
    assert_eq!(
        detail
            .annotations
            .iter()
            .map(|entry| (entry.key.as_str(), entry.value.as_str()))
            .collect::<Vec<_>>(),
        vec![("managed-by", "terraform")]
    );
    assert!(
        detail.versions.is_empty(),
        "describe carries no versions; they are a separate, on-demand fetch"
    );
}

#[test]
fn the_list_argv_is_a_single_call_carrying_the_configuration_and_a_row_cap() {
    let runner = RecordingRunner::new("[]");
    let service = GcloudSecretsService::new(runner);

    service
        .load_secrets(&ctx(), None)
        .expect("empty list parses");

    let argv = service.runner().argv();
    assert_eq!(
        argv,
        vec![
            "--configuration",
            "dev",
            "--project",
            "dev-project",
            "secrets",
            "list",
            "--format=json",
            "--limit=500",
        ]
        .into_iter()
        .map(str::to_string)
        .collect::<Vec<_>>(),
    );
    assert_eq!(
        service.runner().seen.borrow().len(),
        1,
        "one list load is one gcloud call: the version count deliberately does \
         not live on the row, because populating it would cost one extra call \
         per secret"
    );
}

/// A label filter must reach gcloud, because nothing else narrows it.
///
/// `CompiledFilter::Label` keeps every row client-side — labels are not on the
/// list row — so if the pushdown is missing, `-l env=prod` returns the whole
/// project, silently and without an error: the inline error only fires for a
/// service whose `label_filter_field` is `None`, and Secret Manager's is not.
#[test]
fn a_label_filter_is_pushed_down_as_a_server_side_filter() {
    let runner = RecordingRunner::new("[]");
    let service = GcloudSecretsService::new(runner);

    service
        .load_secrets(&ctx(), Some("-lenv=prod"))
        .expect("empty list parses");

    assert!(
        service
            .runner()
            .argv()
            .contains(&r#"--filter=labels.env = "prod""#.to_string()),
        "a label filter must be pushed to gcloud: {:?}",
        service.runner().argv()
    );
}

/// Regex mode stays client-side, and that is the safe half of the trade.
///
/// A secret's display values include a derived replication policy and labels
/// rendered `key=value`, neither of which is verbatim in any raw list field, so
/// there is no pushed expression provably a superset of the client match. An
/// unfiltered fetch that the client then narrows keeps every matching row;
/// pushing a non-superset expression would drop some.
#[test]
fn a_regex_filter_is_not_pushed_down() {
    let runner = RecordingRunner::new("[]");
    let service = GcloudSecretsService::new(runner);

    service
        .load_secrets(&ctx(), Some("signing"))
        .expect("empty list parses");

    assert!(
        !service
            .runner()
            .argv()
            .iter()
            .any(|arg| arg.starts_with("--filter")),
        "regex mode has no provable superset to push: {:?}",
        service.runner().argv()
    );
}
