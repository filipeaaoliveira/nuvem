//! Cloud Functions service layer: the argv nuvem issues, and what it makes of
//! the JSON that comes back.
//!
//! The argv assertions are load-bearing rather than incidental. `--regions=-`
//! and `--v2` are the two flags that decide *which* functions come back and in
//! *which* of two wire formats, and neither is visible in the parsed result —
//! drop them and every test below still passes while the tab shows one region's
//! functions in a shape this module cannot read.

use nuvem::services::gcloud::cli::{CommandError, CommandOutput, CommandRunner, GcloudCommand};
use nuvem::services::resources::functions::GcloudFunctionsService;
use nuvem_domain::gcp::configuration::{GcloudConfigurationName, GcloudContext};
use nuvem_domain::gcp::resource::{CloudFunctionEnvironment, CloudFunctionState};
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

/// Answers every invocation with one canned payload and records the argv it was
/// asked to run, so a test can assert on both halves.
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

fn loaded() -> Vec<nuvem_domain::gcp::resource::CloudFunction> {
    let runner = RecordingRunner::new(include_str!("fixtures/cloud_functions.json"));
    GcloudFunctionsService::new(runner)
        .load_functions(&ctx(), None)
        .expect("the fixture parses")
}

#[test]
fn the_list_parses_the_short_name_url_and_update_time() {
    let functions = loaded();

    assert_eq!(functions.len(), 7, "the fixture holds seven functions");

    // The short id, not the fully-qualified resource path: the resource path is
    // what the API returns and what nobody wants to read in a table cell.
    assert_eq!(functions[0].name, "api-gateway");
    assert_eq!(
        functions[0].url.as_deref(),
        Some("https://europe-west1-dev-project.cloudfunctions.net/api-gateway")
    );
    assert_eq!(
        functions[0].update_time.as_deref(),
        Some("2026-07-14T08:31:05Z")
    );
    assert_eq!(
        functions[0]
            .labels
            .iter()
            .map(|label| (label.key.as_str(), label.value.as_str()))
            .collect::<Vec<_>>(),
        vec![("env", "prod"), ("team", "platform")],
        "labels are sorted by key so a reload does not reshuffle them"
    );

    assert_eq!(
        functions[2].url, None,
        "a function still deploying has no invoke URL yet, and that parses"
    );
    assert!(
        functions[1].labels.is_empty(),
        "a function with no labels parses to no labels rather than failing"
    );
}

/// Region is not a field on the v2 resource — the ticket says it is. It is the
/// `locations/<region>` segment of the resource path, and reading it any other
/// way (say, the leading segment of the invoke URL's host) breaks on the row
/// that has no URL.
#[test]
fn the_region_is_read_out_of_the_resource_path() {
    let functions = loaded();

    assert_eq!(functions[0].region, "europe-west1");
    assert_eq!(functions[1].region, "us-central1");
    assert_eq!(
        functions[2].region, "europe-west4",
        "a function with no invoke URL still has a region: it is in the path"
    );
    assert_eq!(functions[3].region, "asia-northeast1");
}

/// The eight documented states, and a ninth case the document does not cover.
///
/// `UNKNOWN` is a state the API itself reports, so it cannot share an arm with
/// "nuvem does not recognise this string" — a function the API says it has lost
/// track of and a function whose state string is new to nuvem are different
/// facts, and collapsing them loses the first.
#[test]
fn every_documented_state_parses_and_an_undocumented_one_is_carried_verbatim() {
    let functions = loaded();

    assert_eq!(functions[0].state, CloudFunctionState::Active);
    assert_eq!(functions[2].state, CloudFunctionState::Deploying);
    assert_eq!(functions[3].state, CloudFunctionState::DetachFailed);
    assert_eq!(
        functions[4].state,
        CloudFunctionState::Unknown,
        "the API's own UNKNOWN is a documented state, not an unrecognised string"
    );
    assert_eq!(
        functions[5].state,
        CloudFunctionState::Unrecognized("SUSPENDED".into()),
        "a state nuvem does not know is carried through verbatim, never \
         collapsed into a known one"
    );
    assert_eq!(
        functions[6].state,
        CloudFunctionState::Unspecified,
        "an absent state is STATE_UNSPECIFIED: proto3 JSON omits the zero value"
    );
}

/// Generation is a three-valued enum, not the boolean the ticket implies. A
/// `bool` has nowhere to put `ENVIRONMENT_UNSPECIFIED` and would report it as
/// 1st gen.
#[test]
fn the_environment_keeps_its_unspecified_value_distinct_from_gen1() {
    let functions = loaded();

    assert_eq!(functions[0].environment, CloudFunctionEnvironment::Gen2);
    assert_eq!(functions[1].environment, CloudFunctionEnvironment::Gen1);
    assert_eq!(
        functions[3].environment,
        CloudFunctionEnvironment::Unspecified,
        "ENVIRONMENT_UNSPECIFIED is neither generation"
    );
    assert_eq!(
        functions[5].environment,
        CloudFunctionEnvironment::Unrecognized("GEN_3".into()),
        "a generation nuvem does not know is carried through verbatim"
    );
    assert_eq!(
        functions[6].environment,
        CloudFunctionEnvironment::Unspecified,
        "an absent environment is ENVIRONMENT_UNSPECIFIED"
    );
}

/// Walks the variants rather than the cases a reader thought to list, so a
/// variant added later without a parse arm fails here instead of quietly
/// reading as `Unrecognized`.
///
/// The expected pairs are spelled out rather than asserted to be merely
/// non-empty and distinct: those two properties survive every label being
/// replaced by a different string, which is exactly the mutant a state column
/// would show to an operator.
#[test]
fn every_state_variant_round_trips_through_its_api_string_and_has_a_label() {
    let expected = [
        (
            CloudFunctionState::Unspecified,
            "STATE_UNSPECIFIED",
            "Unspecified",
        ),
        (CloudFunctionState::Active, "ACTIVE", "Active"),
        (CloudFunctionState::Failed, "FAILED", "Failed"),
        (CloudFunctionState::Deploying, "DEPLOYING", "Deploying"),
        (CloudFunctionState::Deleting, "DELETING", "Deleting"),
        (CloudFunctionState::Unknown, "UNKNOWN", "Unknown"),
        (CloudFunctionState::Detaching, "DETACHING", "Detaching"),
        (
            CloudFunctionState::DetachFailed,
            "DETACH_FAILED",
            "Detach failed",
        ),
    ];

    assert_eq!(
        CloudFunctionState::ALL.len(),
        expected.len(),
        "a state variant was added without an api string and a label here",
    );

    for (state, api_value, label) in &expected {
        assert!(
            CloudFunctionState::ALL.contains(state),
            "{state:?} is missing from CloudFunctionState::ALL, so nothing walks it",
        );
        assert_eq!(state.api_value(), *api_value);
        assert_eq!(state.label(), *label);
        assert_eq!(
            CloudFunctionState::from_api(Some(api_value)),
            *state,
            "{state:?} must parse back from the API string it names",
        );
    }

    assert_eq!(
        CloudFunctionState::from_api(Some("MOLTEN")),
        CloudFunctionState::Unrecognized("MOLTEN".into()),
    );
    assert_eq!(
        CloudFunctionState::Unrecognized("MOLTEN".into()).label(),
        "MOLTEN",
        "an unrecognised state renders as what the API said, not as a guess",
    );
}

#[test]
fn every_environment_variant_round_trips_through_its_api_string_and_has_a_label() {
    let expected = [
        (
            CloudFunctionEnvironment::Unspecified,
            "ENVIRONMENT_UNSPECIFIED",
            "unspecified",
        ),
        (CloudFunctionEnvironment::Gen1, "GEN_1", "1st gen"),
        (CloudFunctionEnvironment::Gen2, "GEN_2", "2nd gen"),
    ];

    assert_eq!(
        CloudFunctionEnvironment::ALL.len(),
        expected.len(),
        "an environment variant was added without an api string and a label here",
    );

    for (environment, api_value, label) in &expected {
        assert!(
            CloudFunctionEnvironment::ALL.contains(environment),
            "{environment:?} is missing from CloudFunctionEnvironment::ALL",
        );
        assert_eq!(environment.api_value(), *api_value);
        assert_eq!(environment.label(), *label);
        assert_eq!(
            CloudFunctionEnvironment::from_api(Some(api_value)),
            *environment,
            "{environment:?} must parse back from the API string it names",
        );
    }

    assert_eq!(
        CloudFunctionEnvironment::from_api(Some("GEN_9")),
        CloudFunctionEnvironment::Unrecognized("GEN_9".into()),
    );
    assert_eq!(
        CloudFunctionEnvironment::Unrecognized("GEN_9".into()).label(),
        "GEN_9",
    );
}

/// The two flags that decide the row set and the wire format.
///
/// `--regions=-` is the documented default value, and passing it anyway is the
/// point: gcloud's own help says the default is "the region configured in
/// `[functions/region]`", and every nuvem call carries `--configuration`. A
/// configuration with that property set would otherwise return one region's
/// functions under a heading claiming to show the project's.
///
/// `--v2` makes both generations come back in one format. Without it gcloud
/// returns 1st gen functions in the v1 shape — `status` instead of `state`, no
/// `environment` at all, the URL under `httpsTrigger.url` — in the same array
/// as the v2 rows, and this module's parser would read those rows as an
/// unspecified-everything blank.
#[test]
fn the_list_argv_pins_all_regions_and_the_v2_format() {
    let runner = RecordingRunner::new("[]");
    let service = GcloudFunctionsService::new(runner);

    service
        .load_functions(&ctx(), None)
        .expect("empty list parses");

    assert_eq!(
        service.runner().argv(),
        vec![
            "--configuration",
            "dev",
            "--project",
            "dev-project",
            "functions",
            "list",
            "--regions=-",
            "--v2",
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
        "one list load is one gcloud call",
    );
}

/// A label filter must reach gcloud, because nothing else narrows it:
/// `CompiledFilter::Label` keeps every row client-side, so a missing pushdown
/// returns the whole project silently and without an error.
#[test]
fn a_label_filter_is_pushed_down_as_a_server_side_filter() {
    let runner = RecordingRunner::new("[]");
    let service = GcloudFunctionsService::new(runner);

    service
        .load_functions(&ctx(), Some("-lenv=prod"))
        .expect("empty list parses");

    assert!(
        service
            .runner()
            .argv()
            .contains(&r#"--filter=labels.env = "prod""#.to_string()),
        "a label filter must be pushed to gcloud: {:?}",
        service.runner().argv(),
    );
}

/// Regex mode stays client-side. The never-drop-a-row invariant is a claim
/// about display values, and this service has none yet — the projection is a
/// later slice. An unfiltered fetch narrowed client-side keeps every matching
/// row; a pushed expression that is not a provable superset drops some.
#[test]
fn a_regex_filter_is_not_pushed_down() {
    let runner = RecordingRunner::new("[]");
    let service = GcloudFunctionsService::new(runner);

    service
        .load_functions(&ctx(), Some("gateway"))
        .expect("empty list parses");

    assert!(
        !service
            .runner()
            .argv()
            .iter()
            .any(|arg| arg.starts_with("--filter")),
        "regex mode has no provable superset to push: {:?}",
        service.runner().argv(),
    );
}

/// Malformed JSON fails the load rather than reading as an empty project: an
/// empty table is a claim that there are no functions, which is a different
/// fact from "nuvem could not read the answer".
#[test]
fn malformed_json_is_an_error_not_an_empty_list() {
    let runner = RecordingRunner::new("{ this is not the list you are looking for");
    let service = GcloudFunctionsService::new(runner);

    let error = service
        .load_functions(&ctx(), None)
        .expect_err("malformed JSON does not parse");

    assert!(
        error.to_string().contains("invalid Cloud Functions JSON"),
        "the error names the service it came from: {error}",
    );
}
