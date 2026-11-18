//! Cost recommendations and budgets, list only.
//!
//! # Why exit status is read before stdout
//!
//! Every refusal this feature meets — IAM denied, API disabled, budgets denied
//! — writes a valid empty JSON array to stdout and exits non-zero. Reading
//! stdout alone therefore renders a confident "$0 of savings" for a project the
//! operator was never allowed to look at, which is the worst failure a cost
//! screen has: it looks like good news. [`CommandRunner::run`] returns
//! `Err(CommandError::Failed)` for a non-zero exit, and nothing here parses
//! what is in it — the empty array rides along on the error's `stdout`
//! now, but it arrives as a refusal and never as a list, so `?` is still
//! what enforces the rule — and
//! `a_refusal_that_prints_an_empty_array_is_an_error_not_an_empty_list` is what
//! stops a later edit from reaching around it.
//!
//! # Why there is one call per (recommender, location)
//!
//! `recommender recommendations list` requires both `--recommender` and
//! `--location`; there is no list-all call. Which locations are worth asking
//! about is derived from rows nuvem has already listed — see
//! [`nuvem_domain::gcp::finops::query_locations`].

use serde::Deserialize;

use nuvem_domain::gcp::configuration::GcloudContext;
use nuvem_domain::gcp::finops::{
    CostRecommendation, RecommendationState, RecommenderId, parse_duration_seconds,
};
use nuvem_domain::gcp::money::{MONTH_SECONDS, Money};

use crate::services::gcloud::cli::{CommandError, CommandRunner, GcloudCommand};

#[derive(Debug, thiserror::Error)]
pub enum FinOpsLoadError {
    #[error("{0}")]
    Command(#[from] CommandError),
    #[error("invalid FinOps JSON: {0}")]
    Parse(String),
    /// The project is not linked to a billing account, or billing is disabled
    /// on it. A distinct case rather than an empty budget list: "no budgets"
    /// and "nothing is being billed here" are different facts, and only the
    /// second explains why no budget will ever appear.
    #[error("no billing account is linked to this project")]
    NoBillingAccount,
}

/// A budget, as much of one as the API returns.
///
/// There is no `spend` field anywhere on the resource, so there is no gauge to
/// draw: amount and thresholds are the whole honest content. A gauge would need
/// the BigQuery billing export, which v1 does not read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Budget {
    pub display_name: String,
    pub amount: Option<Money>,
    /// `lastPeriodAmount` — the budget tracks whatever the previous period
    /// cost, so there is no figure to show, only the rule.
    pub matches_last_period: bool,
    /// Threshold percentages: the API's `0.5` rendered as `50`.
    pub thresholds: Vec<u32>,
    /// True when `budgetFilter.projects` does not name the active project, so
    /// the amount covers the whole billing account rather than this project.
    pub covers_whole_account: bool,
}

pub struct GcloudFinOpsService<R> {
    runner: R,
}

impl<R> GcloudFinOpsService<R> {
    pub fn new(runner: R) -> Self {
        Self { runner }
    }

    pub fn runner(&self) -> &R {
        &self.runner
    }
}

/// Shared leading args: `--configuration <cfg> [--project <p>]`.
fn base_args(context: &GcloudContext) -> Vec<String> {
    let mut args = vec![
        "--configuration".to_string(),
        context.configuration.as_str().to_string(),
    ];
    if let Some(project) = context.project.as_deref() {
        args.extend(["--project".to_string(), project.to_string()]);
    }
    args
}

/// The exact argv for one `(recommender, location)` probe.
///
/// Exposed so a test can assert the shape without a runner, and so the command
/// preview shows the argv that runs: `--quiet` is here rather than injected
/// below the preview because gcloud interactively offers to enable a disabled
/// API, and a flag added under the preview would make the preview a lie.
pub fn recommendations_argv(
    context: &GcloudContext,
    recommender: RecommenderId,
    location: &str,
) -> Vec<String> {
    let mut args = base_args(context);
    args.extend([
        "recommender".to_string(),
        "recommendations".to_string(),
        "list".to_string(),
        format!("--location={location}"),
        format!("--recommender={}", recommender.id()),
        "--format=json".to_string(),
        "--quiet".to_string(),
    ]);
    args
}

/// Resolves which billing account a project is billed to.
///
/// A separate call because budgets are billing-account scoped while everything
/// else here is project scoped; there is no way to ask for "this project's
/// budgets" directly.
pub fn billing_project_argv(context: &GcloudContext, project: &str) -> Vec<String> {
    vec![
        "--configuration".to_string(),
        context.configuration.as_str().to_string(),
        "billing".to_string(),
        "projects".to_string(),
        "describe".to_string(),
        project.to_string(),
        "--format=json".to_string(),
        "--quiet".to_string(),
    ]
}

/// Budgets are billing-account scoped, not project scoped, which is why this
/// argv carries no `--project` and why [`Budget::covers_whole_account`] exists.
pub fn budgets_argv(context: &GcloudContext, billing_account: &str) -> Vec<String> {
    vec![
        "--configuration".to_string(),
        context.configuration.as_str().to_string(),
        "billing".to_string(),
        "budgets".to_string(),
        "list".to_string(),
        format!("--billing-account={billing_account}"),
        "--format=json".to_string(),
        "--quiet".to_string(),
    ]
}

impl<R: CommandRunner> GcloudFinOpsService<R> {
    pub fn load_recommendations(
        &self,
        context: &GcloudContext,
        recommender: RecommenderId,
        location: &str,
    ) -> Result<Vec<CostRecommendation>, FinOpsLoadError> {
        let argv = recommendations_argv(context, recommender, location);
        let output = self.runner.run(GcloudCommand::new(argv))?;
        parse_recommendations(&output.stdout)
    }

    /// The project's budgets, in two calls: which billing account it is billed
    /// to, then that account's budgets.
    ///
    /// A project with no billing account is [`FinOpsLoadError::NoBillingAccount`]
    /// rather than an empty list, so the card can say why it will stay empty
    /// instead of implying nobody has set a budget.
    pub fn load_budgets(&self, context: &GcloudContext) -> Result<Vec<Budget>, FinOpsLoadError> {
        let project = context
            .project
            .as_deref()
            .ok_or(FinOpsLoadError::NoBillingAccount)?;
        let described = self
            .runner
            .run(GcloudCommand::new(billing_project_argv(context, project)))?;
        let billing_account = billing_account_of(&described.stdout)?;

        let output = self
            .runner
            .run(GcloudCommand::new(budgets_argv(context, &billing_account)))?;
        parse_budgets(&output.stdout, context.project.as_deref())
    }
}

/// The wire shape, list format. Every field defaults: a recommendation missing
/// one still lists, and only a document that is not the expected JSON fails —
/// the same bargain the sibling services strike, and here it is load-bearing
/// because a dropped row is a saving the operator never sees.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawRecommendation {
    #[serde(default)]
    name: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    priority: Option<String>,
    #[serde(default)]
    xor_group_id: Option<String>,
    #[serde(default)]
    last_refresh_time: Option<String>,
    #[serde(default)]
    state_info: Option<RawStateInfo>,
    #[serde(default)]
    primary_impact: Option<RawImpact>,
    #[serde(default)]
    content: Option<RawContent>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawStateInfo {
    #[serde(default)]
    state: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawImpact {
    #[serde(default)]
    cost_projection: Option<RawCostProjection>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawCostProjection {
    #[serde(default)]
    cost: Option<RawMoney>,
    #[serde(default)]
    duration: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawMoney {
    #[serde(default)]
    currency_code: String,
    /// An int64 encoded as a JSON *string*, which is why this is not an i64:
    /// deserializing it as one fails the whole document.
    #[serde(default)]
    units: String,
    #[serde(default)]
    nanos: i32,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawContent {
    #[serde(default)]
    operation_groups: Vec<RawOperationGroup>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawOperationGroup {
    #[serde(default)]
    operations: Vec<RawOperation>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawOperation {
    #[serde(default)]
    resource: Option<String>,
}

fn parse_recommendations(stdout: &str) -> Result<Vec<CostRecommendation>, FinOpsLoadError> {
    let raw: Vec<RawRecommendation> =
        serde_json::from_str(stdout).map_err(|error| FinOpsLoadError::Parse(error.to_string()))?;
    Ok(raw.into_iter().map(cost_recommendation).collect())
}

/// Sign-corrects and normalises in one place.
///
/// The API states a saving as a NEGATIVE cost over the projection's OWN
/// duration. Both are corrected here so no render site has to remember either
/// convention, and a projection whose duration will not parse yields `None`
/// rather than an unscaled figure presented as monthly.
fn cost_recommendation(raw: RawRecommendation) -> CostRecommendation {
    let monthly = raw
        .primary_impact
        .as_ref()
        .and_then(|impact| impact.cost_projection.as_ref())
        .and_then(|projection| {
            let cost = projection.cost.as_ref()?;
            let duration = parse_duration_seconds(projection.duration.as_deref()?)?;
            let money = Money::parse(&cost.units, cost.nanos, &cost.currency_code).ok()?;
            money.negate().scaled(MONTH_SECONDS, duration)
        });

    let target_resources = raw
        .content
        .map(|content| {
            content
                .operation_groups
                .into_iter()
                .flat_map(|group| group.operations)
                .filter_map(|operation| operation.resource)
                .collect()
        })
        .unwrap_or_default();

    CostRecommendation {
        name: raw.name,
        description: raw.description,
        state: RecommendationState::from_api(
            raw.state_info
                .as_ref()
                .and_then(|info| info.state.as_deref()),
        ),
        priority: raw.priority,
        xor_group: raw.xor_group_id,
        monthly,
        last_refresh: raw.last_refresh_time,
        target_resources,
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawBillingProject {
    #[serde(default)]
    billing_account_name: String,
    #[serde(default)]
    billing_enabled: bool,
}

/// `billingEnabled` is checked as well as the name: a project can retain a
/// stale `billingAccountName` with billing switched off, and listing that
/// account's budgets would attribute someone else's spending limits to a
/// project that is not being billed against them.
fn billing_account_of(stdout: &str) -> Result<String, FinOpsLoadError> {
    let raw: RawBillingProject =
        serde_json::from_str(stdout).map_err(|error| FinOpsLoadError::Parse(error.to_string()))?;
    if !raw.billing_enabled || raw.billing_account_name.is_empty() {
        return Err(FinOpsLoadError::NoBillingAccount);
    }
    Ok(raw.billing_account_name)
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawBudget {
    #[serde(default)]
    display_name: String,
    #[serde(default)]
    amount: Option<RawBudgetAmount>,
    #[serde(default)]
    threshold_rules: Vec<RawThresholdRule>,
    #[serde(default)]
    budget_filter: Option<RawBudgetFilter>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawBudgetAmount {
    #[serde(default)]
    specified_amount: Option<RawMoney>,
    /// Present and empty when the budget tracks the last period. There is no
    /// figure inside it, so this is read for presence only.
    #[serde(default)]
    last_period_amount: Option<serde_json::Value>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawThresholdRule {
    #[serde(default)]
    threshold_percent: f64,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawBudgetFilter {
    #[serde(default)]
    projects: Vec<String>,
}

fn parse_budgets(stdout: &str, project: Option<&str>) -> Result<Vec<Budget>, FinOpsLoadError> {
    let raw: Vec<RawBudget> =
        serde_json::from_str(stdout).map_err(|error| FinOpsLoadError::Parse(error.to_string()))?;
    Ok(raw.into_iter().map(|raw| budget(raw, project)).collect())
}

fn budget(raw: RawBudget, project: Option<&str>) -> Budget {
    let amount = raw
        .amount
        .as_ref()
        .and_then(|amount| amount.specified_amount.as_ref())
        .and_then(|money| Money::parse(&money.units, money.nanos, &money.currency_code).ok());
    let matches_last_period = raw
        .amount
        .as_ref()
        .is_some_and(|amount| amount.last_period_amount.is_some());

    // f64 reaches only a percentage label. The no-f64 rule is about money, and
    // a threshold percent is not money.
    let thresholds = raw
        .threshold_rules
        .iter()
        .map(|rule| (rule.threshold_percent * 100.0).round() as u32)
        .collect();

    // Absent filter, absent project, or a project list that does not name this
    // one: all three mean the amount is not about this project alone. Matched
    // on the tail because the filter spells it `projects/<id>`.
    let covers_whole_account = match (project, &raw.budget_filter) {
        (Some(project), Some(filter)) => !filter
            .projects
            .iter()
            .any(|entry| entry == project || entry.ends_with(&format!("/{project}"))),
        _ => true,
    };

    Budget {
        display_name: raw.display_name,
        amount,
        matches_last_period,
        thresholds,
        covers_whole_account,
    }
}
