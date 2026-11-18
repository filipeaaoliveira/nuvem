//! Cloud Functions, list only.
//!
//! # The two flags that decide what comes back
//!
//! `--regions=-` is gcloud's documented default *value*, and this module passes
//! it anyway. gcloud's own help says the default behaviour is "functions from
//! the region configured in `[functions/region]` property", and every nuvem call
//! carries `--configuration`: a configuration with that property set would
//! otherwise return one region's functions under a heading that claims to show
//! the project's. Passing `-` makes the row set a property of the project rather
//! than of the operator's local gcloud config.
//!
//! `--v2` is what lets one parser read both generations. Without it gcloud
//! returns 1st gen functions in the *v1* format and 2nd gen in the *v2* format,
//! in the same array: the v1 shape spells the lifecycle field `status`, has no
//! `environment` at all, and puts the invoke URL under `httpsTrigger.url`. The
//! raw shapes below are the v2 resource, so a v1 row would parse into a blank
//! with no generation. `--v2` does not hide 1st gen functions; it only changes
//! how they are represented, and `environment` then carries the generation as an
//! actual value instead of as the absence of a field.

use serde::Deserialize;

use nuvem_domain::gcp::configuration::GcloudContext;
use nuvem_domain::gcp::filter::label_filter_field;
use nuvem_domain::gcp::resource::{
    CloudFunction, CloudFunctionEnvironment, CloudFunctionState, ComputeKeyValue,
    ResourceServiceKind,
};

use crate::services::gcloud::cli::{CommandError, CommandRunner, GcloudCommand};
use crate::services::resources::list_filter::{RESOURCE_LIST_LIMIT, pushdown_expression};

/// Verbatim-superset raw fields eligible for the regex pushdown — deliberately
/// none, and staying that way.
///
/// The never-drop-a-row invariant is a claim about *display* values: each of a
/// service's searchable fields must be a verbatim substring of a pushed raw
/// field, or be enumerated in [`OPAQUE_LABELS`]. Nothing but the label mode is
/// pushed for this service, so there is no raw expression whose superset
/// property would have to be asserted over the displayed set — name, region,
/// state, generation, update time and URL, plus the `key=value` labels. The
/// state and generation labels appear in no raw gcloud field, so pushing a
/// clause would mean enumerating each of them in [`OPAQUE_LABELS`].
/// Empty degrades to full client-side narrowing, which is the safe direction.
const FILTER_FIELDS: &[&str] = &[];

/// Empty for the same reason [`FILTER_FIELDS`] is: with nothing pushed there is
/// nothing for the gate to protect.
const OPAQUE_LABELS: &[&str] = &[];

/// The path segment naming a function's region, as in
/// `projects/p/locations/europe-west1/functions/f`.
const LOCATION_SEGMENT: &str = "locations";

#[derive(Debug, thiserror::Error)]
pub enum CloudFunctionsParseError {
    #[error("invalid Cloud Functions JSON: {0}")]
    Json(#[from] serde_json::Error),
}

#[derive(Debug, thiserror::Error)]
pub enum CloudFunctionsLoadError {
    #[error("{0}")]
    Command(#[from] CommandError),
    #[error("{0}")]
    Parse(#[from] CloudFunctionsParseError),
}

pub struct GcloudFunctionsService<R> {
    runner: R,
}

impl<R> GcloudFunctionsService<R> {
    pub fn new(runner: R) -> Self {
        Self { runner }
    }

    pub fn runner(&self) -> &R {
        &self.runner
    }
}

impl<R: CommandRunner> GcloudFunctionsService<R> {
    /// Lists a project's functions across every region, capped at
    /// [`RESOURCE_LIST_LIMIT`] rows and, in label mode, narrowed with a
    /// pushed-down `--filter` on `labels`.
    ///
    /// Only label mode reaches the server. Label mode has no client-side
    /// fallback — `CompiledFilter::Label` keeps every row — so pushing it is
    /// what makes `-l k=v` narrow at all; regex mode narrows client-side, see
    /// `FILTER_FIELDS`.
    pub fn load_functions(
        &self,
        context: &GcloudContext,
        filter: Option<&str>,
    ) -> Result<Vec<CloudFunction>, CloudFunctionsLoadError> {
        let mut args = base_args(context);
        args.extend([
            "functions".to_string(),
            "list".to_string(),
            "--regions=-".to_string(),
            "--v2".to_string(),
            "--format=json".to_string(),
            format!("--limit={RESOURCE_LIST_LIMIT}"),
        ]);
        if let Some(expression) = filter.and_then(|applied| {
            pushdown_expression(
                applied,
                FILTER_FIELDS,
                OPAQUE_LABELS,
                label_filter_field(ResourceServiceKind::CloudFunctions),
            )
        }) {
            args.push(format!("--filter={expression}"));
        }
        let output = self.runner.run(GcloudCommand::new(args))?;
        Ok(parse_functions_json(&output.stdout)?)
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

/// The v2 wire shape of a function, list format.
///
/// Every field defaults, so a row missing any of them still parses; only a
/// document that is not the expected JSON fails the load, which is the same
/// bargain the sibling services strike. An empty table is a claim that the
/// project has no functions, and that must not be how an unreadable answer
/// renders.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawFunction {
    #[serde(default)]
    name: String,
    #[serde(default)]
    state: Option<String>,
    #[serde(default)]
    environment: Option<String>,
    #[serde(default)]
    update_time: Option<String>,
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    labels: std::collections::BTreeMap<String, String>,
}

pub fn parse_functions_json(stdout: &str) -> Result<Vec<CloudFunction>, CloudFunctionsParseError> {
    let raw: Vec<RawFunction> = serde_json::from_str(stdout)?;
    Ok(raw.into_iter().map(function_from_raw).collect())
}

fn function_from_raw(raw: RawFunction) -> CloudFunction {
    CloudFunction {
        name: short_id(&raw.name),
        region: region_from_path(&raw.name),
        state: CloudFunctionState::from_api(raw.state.as_deref()),
        environment: CloudFunctionEnvironment::from_api(raw.environment.as_deref()),
        url: raw.url,
        update_time: raw.update_time,
        labels: key_values(raw.labels),
    }
}

/// The trailing segment of a fully-qualified resource path.
///
/// The API returns `projects/p/locations/r/functions/f`; a table cell wants `f`.
/// A name with no slash is already short and passes through unchanged.
pub fn short_id(resource_path: &str) -> String {
    resource_path
        .rsplit('/')
        .next()
        .unwrap_or(resource_path)
        .to_string()
}

/// The region a function is deployed in, read out of its resource path.
///
/// There is no region field on the resource, and the invoke URL is no
/// substitute: a function that is still deploying has no URL, and its region is
/// exactly what an operator wants to see while it does. A path with no
/// `locations/<region>` pair yields an empty region, matching how the Cloud Run
/// service handles a service whose location metadata is missing.
pub fn region_from_path(resource_path: &str) -> String {
    let mut segments = resource_path.split('/');
    while let Some(segment) = segments.next() {
        if segment == LOCATION_SEGMENT {
            return segments.next().unwrap_or_default().to_string();
        }
    }
    String::new()
}

/// Sorted key/value pairs. Sorted rather than in map order so a reload does not
/// reshuffle them under the operator's cursor.
pub fn key_values(map: std::collections::BTreeMap<String, String>) -> Vec<ComputeKeyValue> {
    map.into_iter()
        .map(|(key, value)| ComputeKeyValue { key, value })
        .collect()
}
