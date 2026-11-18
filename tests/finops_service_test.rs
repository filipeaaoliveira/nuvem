use std::cell::RefCell;

use nuvem::services::gcloud::cli::{CommandError, CommandOutput, CommandRunner, GcloudCommand};
use nuvem::services::resources::finops::{
    FinOpsLoadError, GcloudFinOpsService, billing_project_argv, budgets_argv, recommendations_argv,
};
use nuvem_domain::gcp::configuration::{GcloudConfigurationName, GcloudContext};
use nuvem_domain::gcp::finops::{RecommendationState, RecommenderId};

fn context() -> GcloudContext {
    GcloudContext {
        configuration: GcloudConfigurationName::new("default"),
        account: None,
        project: Some("proj".to_string()),
        region: None,
        zone: None,
    }
}

/// A runner primed with one answer, which it hands over exactly once.
struct StubRunner {
    result: RefCell<Vec<Result<CommandOutput, CommandError>>>,
}

impl StubRunner {
    fn once(result: Result<CommandOutput, CommandError>) -> Self {
        Self {
            result: RefCell::new(vec![result]),
        }
    }

    fn ok(stdout: &str) -> Self {
        Self::once(Ok(CommandOutput {
            stdout: stdout.to_string(),
            stderr: String::new(),
        }))
    }

    /// Answers in the order given. `load_budgets` makes two calls — describe
    /// the project's billing, then list that account's budgets.
    fn sequence(stdouts: &[&str]) -> Self {
        Self {
            result: RefCell::new(
                stdouts
                    .iter()
                    .rev()
                    .map(|stdout| {
                        Ok(CommandOutput {
                            stdout: stdout.to_string(),
                            stderr: String::new(),
                        })
                    })
                    .collect(),
            ),
        }
    }
}

impl CommandRunner for StubRunner {
    fn run(&self, _command: GcloudCommand) -> Result<CommandOutput, CommandError> {
        self.result
            .borrow_mut()
            .pop()
            .expect("stub runner called more times than it was primed")
    }
}

/// THE safety property of this feature, and it is one assertion.
///
/// Every observed refusal mode — IAM denied, API disabled, budgets denied —
/// writes a valid empty JSON array to stdout and exits non-zero. A service
/// that reads stdout and ignores exit status therefore renders a confident
/// "$0 of savings" for a project it was never allowed to look at, which is
/// the worst failure a cost screen has: it looks like good news.
#[test]
fn a_refusal_that_prints_an_empty_array_is_an_error_not_an_empty_list() {
    let runner = StubRunner::once(Err(CommandError::Failed {
        stdout: String::new(),
        stderr: "PERMISSION_DENIED: Recommender API has not been used in project \
                 proj before or it is disabled.\n  reason: SERVICE_DISABLED"
            .to_string(),
        exit_code: Some(1),
    }));
    let service = GcloudFinOpsService::new(&runner);

    let result = service.load_recommendations(&context(), RecommenderId::IdleVm, "europe-west1-b");

    assert!(
        result.is_err(),
        "a non-zero exit must not become an empty recommendation list"
    );
}

/// The complementary half: a zero exit with an empty array really is empty.
#[test]
fn a_successful_empty_array_is_an_empty_list() {
    let runner = StubRunner::ok("[]");
    let service = GcloudFinOpsService::new(&runner);

    let loaded = service
        .load_recommendations(&context(), RecommenderId::IdleVm, "europe-west1-b")
        .expect("a zero exit is a real answer");
    assert!(loaded.is_empty());
}

/// --quiet is in the argv the preview shows, not injected below it: gcloud
/// interactively offers to enable a disabled API, and the runner hands argv to
/// the process verbatim.
#[test]
fn the_argv_names_the_recommender_the_location_and_quiet() {
    let argv = recommendations_argv(&context(), RecommenderId::UnusedAddress, "europe-west1");

    assert!(argv.contains(&"--quiet".to_string()));
    assert!(argv.contains(&"--format=json".to_string()));
    assert!(argv.contains(&"--location=europe-west1".to_string()));
    assert!(
        argv.contains(&"--recommender=google.compute.address.IdleResourceRecommender".to_string())
    );
    assert!(argv.contains(&"--project".to_string()));
    assert!(argv.contains(&"proj".to_string()));
    assert_eq!(
        argv.iter()
            .filter(|arg| arg.starts_with("--location"))
            .count(),
        1
    );
    assert_eq!(
        argv.iter().position(|arg| arg == "recommender"),
        Some(
            argv.iter()
                .position(|arg| arg == "--configuration")
                .unwrap()
                + 4
        ),
        "the subcommand follows the configuration and project flags"
    );
}

/// The projection is stated over its own duration and sign-inverted. A
/// fortnight's -50 USD is a 100 USD monthly saving, positive, once.
#[test]
fn a_cost_projection_becomes_a_positive_monthly_saving() {
    let runner = StubRunner::ok(
        r#"[
          {
            "name": "projects/p/locations/z/recommenders/r/recommendations/abc",
            "description": "Save cost by stopping idle VM",
            "stateInfo": {"state": "ACTIVE"},
            "priority": "P2",
            "lastRefreshTime": "2026-08-06T00:00:00Z",
            "primaryImpact": {
              "costProjection": {
                "cost": {"currencyCode": "USD", "units": "-50", "nanos": 0},
                "duration": "1296000s"
              }
            }
          }
        ]"#,
    );
    let service = GcloudFinOpsService::new(&runner);

    let loaded = service
        .load_recommendations(&context(), RecommenderId::IdleVm, "europe-west1-b")
        .expect("parses");

    assert_eq!(loaded.len(), 1);
    let monthly = loaded[0].monthly.as_ref().expect("has a monthly figure");
    assert_eq!(monthly.nanos, 100_000_000_000);
    assert_eq!(monthly.currency, "USD");
    assert_eq!(loaded[0].state, RecommendationState::Active);
    assert_eq!(loaded[0].priority.as_deref(), Some("P2"));
    assert_eq!(
        loaded[0].last_refresh.as_deref(),
        Some("2026-08-06T00:00:00Z")
    );
}

/// A recommendation with no usable duration is listed but carries no monthly
/// figure, so savings_total leaves it out instead of adding a silent zero.
#[test]
fn a_projection_without_a_usable_duration_has_no_monthly_figure() {
    let runner = StubRunner::ok(
        r#"[
          {
            "name": "projects/p/locations/z/recommenders/r/recommendations/abc",
            "description": "Save cost",
            "stateInfo": {"state": "ACTIVE"},
            "primaryImpact": {
              "costProjection": {
                "cost": {"currencyCode": "USD", "units": "-50", "nanos": 0},
                "duration": "not-a-duration"
              }
            }
          }
        ]"#,
    );
    let service = GcloudFinOpsService::new(&runner);

    let loaded = service
        .load_recommendations(&context(), RecommenderId::IdleVm, "europe-west1-b")
        .expect("parses");
    assert_eq!(loaded.len(), 1);
    assert!(loaded[0].monthly.is_none());
}

/// A recommendation with no cost projection at all is still listed — the
/// Recommender API returns non-cost impacts too, and dropping the row would
/// hide a recommendation rather than merely leaving it out of a total.
#[test]
fn a_recommendation_with_no_cost_projection_is_still_listed() {
    let runner =
        StubRunner::ok(r#"[{"name": "a", "description": "d", "stateInfo": {"state": "ACTIVE"}}]"#);
    let service = GcloudFinOpsService::new(&runner);

    let loaded = service
        .load_recommendations(&context(), RecommenderId::IdleVm, "z")
        .expect("parses");
    assert_eq!(loaded.len(), 1);
    assert!(loaded[0].monthly.is_none());
}

/// xorGroupId travels through, because folding it is the domain's job and the
/// service dropping it would make two alternatives look like two savings.
#[test]
fn the_xor_group_and_target_resources_survive_the_parse() {
    let runner = StubRunner::ok(
        r#"[
          {
            "name": "a",
            "description": "d",
            "xorGroupId": "grp-1",
            "stateInfo": {"state": "ACTIVE"},
            "content": {"operationGroups": [{"operations": [
              {"resource": "//compute.googleapis.com/projects/p/zones/z/instances/vm-1"}
            ]}]},
            "primaryImpact": {"costProjection": {
              "cost": {"currencyCode": "USD", "units": "-1", "nanos": 0},
              "duration": "2592000s"}}
          }
        ]"#,
    );
    let service = GcloudFinOpsService::new(&runner);
    let loaded = service
        .load_recommendations(&context(), RecommenderId::IdleVm, "z")
        .expect("parses");
    assert_eq!(loaded[0].xor_group.as_deref(), Some("grp-1"));
    assert_eq!(
        loaded[0].target_resources,
        vec!["//compute.googleapis.com/projects/p/zones/z/instances/vm-1".to_string()]
    );
}

/// Malformed JSON is a parse error, not an empty list, for the same reason a
/// refusal is not: both would render as "nothing to save".
#[test]
fn malformed_json_is_a_parse_error_not_an_empty_list() {
    let runner = StubRunner::ok("{not json");
    let service = GcloudFinOpsService::new(&runner);
    assert!(
        service
            .load_recommendations(&context(), RecommenderId::IdleVm, "z")
            .is_err()
    );
}

/// The budgets resource carries amount and thresholdRules and no spend field.
/// A budget naming this project is project-scoped; one that does not is
/// account-wide, and saying so is the difference between information and a
/// misleading number beside a sandbox project.
#[test]
fn a_budget_carries_its_amount_thresholds_and_scope() {
    let runner = StubRunner::sequence(&[
        r#"{"billingAccountName": "billingAccounts/XXX", "billingEnabled": true}"#,
        r#"[
          {
            "displayName": "Monthly cap",
            "amount": {"specifiedAmount": {"currencyCode": "EUR", "units": "500", "nanos": 0}},
            "thresholdRules": [{"thresholdPercent": 0.5}, {"thresholdPercent": 1.0}],
            "budgetFilter": {"projects": ["projects/proj"]}
          },
          {
            "displayName": "Everything",
            "amount": {"lastPeriodAmount": {}},
            "thresholdRules": [{"thresholdPercent": 1.0}],
            "budgetFilter": {}
          }
        ]"#,
    ]);
    let service = GcloudFinOpsService::new(&runner);

    let budgets = service.load_budgets(&context()).expect("parses");

    assert_eq!(budgets.len(), 2);
    assert_eq!(budgets[0].display_name, "Monthly cap");
    assert_eq!(
        budgets[0].amount.as_ref().expect("an amount").nanos,
        500_000_000_000
    );
    assert_eq!(budgets[0].thresholds, vec![50, 100]);
    assert!(
        !budgets[0].covers_whole_account,
        "the filter names this project"
    );

    assert!(budgets[1].amount.is_none());
    assert!(budgets[1].matches_last_period);
    assert!(
        budgets[1].covers_whole_account,
        "an empty filter covers the whole billing account"
    );
}

/// A denied budgets call is an error too, and for the same reason.
#[test]
fn a_denied_budgets_call_is_an_error_not_an_empty_budget_list() {
    let runner = StubRunner::once(Err(CommandError::Failed {
        stdout: String::new(),
        stderr: "PERMISSION_DENIED: The caller does not have permission".to_string(),
        exit_code: Some(1),
    }));
    let service = GcloudFinOpsService::new(&runner);
    assert!(service.load_budgets(&context()).is_err());
}

/// The budget filter spells a project either bare or as `projects/<id>`, and
/// a filter naming a DIFFERENT project is account-wide as far as this project
/// is concerned. Suffix matching alone would answer the first case wrongly and
/// tail matching alone would let `other-proj` match `proj`.
#[test]
fn a_budget_filter_matches_a_bare_project_id_and_only_that_project() {
    let runner = StubRunner::sequence(&[
        r#"{"billingAccountName": "billingAccounts/XXX", "billingEnabled": true}"#,
        r#"[
          {"displayName": "bare", "budgetFilter": {"projects": ["proj"]}},
          {"displayName": "qualified", "budgetFilter": {"projects": ["projects/proj"]}},
          {"displayName": "another", "budgetFilter": {"projects": ["projects/other-proj"]}},
          {"displayName": "suffix trap", "budgetFilter": {"projects": ["not-proj"]}}
        ]"#,
    ]);
    let service = GcloudFinOpsService::new(&runner);
    let budgets = service.load_budgets(&context()).expect("parses");

    assert!(
        !budgets[0].covers_whole_account,
        "a bare id is this project"
    );
    assert!(
        !budgets[1].covers_whole_account,
        "projects/<id> is this project"
    );
    assert!(
        budgets[2].covers_whole_account,
        "a different project does not scope this one"
    );
    assert!(
        budgets[3].covers_whole_account,
        "not-proj is a different project, not a suffix match on proj"
    );
}

/// Budgets are billing-account scoped, so this argv carries the account and
/// deliberately no --project: passing one narrows nothing and fails on
/// accounts the project is not under. --quiet is here for the same reason it
/// is on the recommender argv.
#[test]
fn the_budgets_argv_names_the_billing_account_and_no_project() {
    let argv = budgets_argv(&context(), "billingAccounts/XXX");

    assert_eq!(
        argv,
        vec![
            "--configuration".to_string(),
            "default".to_string(),
            "billing".to_string(),
            "budgets".to_string(),
            "list".to_string(),
            "--billing-account=billingAccounts/XXX".to_string(),
            "--format=json".to_string(),
            "--quiet".to_string(),
        ]
    );
    assert!(!argv.contains(&"--project".to_string()));
}

/// A project nothing is billed against will never have budgets, and saying so
/// is different from showing an empty list — which reads as "nobody set one".
#[test]
fn a_project_with_billing_disabled_is_not_an_empty_budget_list() {
    for described in [
        r#"{"billingAccountName": "billingAccounts/XXX", "billingEnabled": false}"#,
        r#"{"billingEnabled": true}"#,
        r#"{}"#,
    ] {
        let runner = StubRunner::sequence(&[described, "[]"]);
        let service = GcloudFinOpsService::new(&runner);
        assert!(
            matches!(
                service.load_budgets(&context()),
                Err(FinOpsLoadError::NoBillingAccount)
            ),
            "{described} should report no billing account"
        );
    }
}

/// A stale billingAccountName with billing switched off must not be used:
/// listing that account's budgets would attribute someone else's spending
/// limits to a project not being billed against them.
#[test]
fn a_billing_account_is_only_used_when_billing_is_enabled() {
    let runner = StubRunner::sequence(&[
        r#"{"billingAccountName": "billingAccounts/STALE", "billingEnabled": false}"#,
        r#"[{"displayName": "someone else's cap"}]"#,
    ]);
    let service = GcloudFinOpsService::new(&runner);
    assert!(matches!(
        service.load_budgets(&context()),
        Err(FinOpsLoadError::NoBillingAccount)
    ));
}

/// The describe call names the project and carries --quiet, like every other
/// argv here.
#[test]
fn the_billing_describe_argv_names_the_project() {
    let argv = billing_project_argv(&context(), "proj");
    assert_eq!(
        argv,
        vec![
            "--configuration".to_string(),
            "default".to_string(),
            "billing".to_string(),
            "projects".to_string(),
            "describe".to_string(),
            "proj".to_string(),
            "--format=json".to_string(),
            "--quiet".to_string(),
        ]
    );
}

/// A context with no project has nothing to describe, so the call is refused
/// rather than issued against whatever gcloud would default to.
#[test]
fn a_context_without_a_project_resolves_no_billing_account() {
    let runner = StubRunner::sequence(&[]);
    let service = GcloudFinOpsService::new(&runner);
    let mut context = context();
    context.project = None;
    assert!(matches!(
        service.load_budgets(&context),
        Err(FinOpsLoadError::NoBillingAccount)
    ));
}
