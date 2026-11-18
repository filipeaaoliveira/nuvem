use std::collections::BTreeMap;

use serde::Deserialize;

use nuvem_domain::gcp::configuration::GcloudContext;
use nuvem_domain::gcp::resource::{
    CloudRunCondition, CloudRunRevision, CloudRunRevisionDetail, CloudRunService,
    CloudRunServiceDetail, CloudRunServiceStatus, CloudRunTrafficTarget, ComputeKeyValue,
};

use crate::services::gcloud::cli::{CommandError, CommandRunner, GcloudCommand};
use crate::services::resources::list_filter::RESOURCE_LIST_LIMIT;

const LOCATION_LABEL: &str = "cloud.googleapis.com/location";
const LOCATION_ANNOTATION: &str = "run.googleapis.com/location";
const SERVICE_LABEL: &str = "serving.knative.dev/service";
const INGRESS_ANNOTATION: &str = "run.googleapis.com/ingress";
const MIN_SCALE_ANNOTATION: &str = "autoscaling.knative.dev/minScale";
const MAX_SCALE_ANNOTATION: &str = "autoscaling.knative.dev/maxScale";

#[derive(Debug, thiserror::Error)]
pub enum CloudRunParseError {
    #[error("invalid Cloud Run JSON: {0}")]
    Json(#[from] serde_json::Error),
}

#[derive(Debug, thiserror::Error)]
pub enum CloudRunLoadError {
    #[error("{0}")]
    Command(#[from] CommandError),
    #[error("{0}")]
    Parse(#[from] CloudRunParseError),
}

pub struct GcloudCloudRunService<R> {
    runner: R,
}

impl<R> GcloudCloudRunService<R> {
    pub fn new(runner: R) -> Self {
        Self { runner }
    }

    pub fn runner(&self) -> &R {
        &self.runner
    }
}

impl<R: CommandRunner> GcloudCloudRunService<R> {
    /// Lists Cloud Run services, capped at [`RESOURCE_LIST_LIMIT`] rows. The
    /// Knative list shape isn't reliably server-filter-expressible, so
    /// unlike the other resource loaders this does not push a `--filter`
    /// down; the caller keeps narrowing client-side.
    pub fn load_services(
        &self,
        context: &GcloudContext,
    ) -> Result<Vec<CloudRunService>, CloudRunLoadError> {
        let mut args = base_args(context);
        args.extend([
            "run".to_string(),
            "services".to_string(),
            "list".to_string(),
            "--platform=managed".to_string(),
            "--format=json".to_string(),
            format!("--limit={RESOURCE_LIST_LIMIT}"),
        ]);
        let output = self.runner.run(GcloudCommand::new(args))?;
        Ok(parse_services_json(&output.stdout)?)
    }

    pub fn load_service_detail(
        &self,
        context: &GcloudContext,
        service: &CloudRunService,
    ) -> Result<CloudRunServiceDetail, CloudRunLoadError> {
        let mut args = base_args(context);
        args.extend([
            "run".to_string(),
            "services".to_string(),
            "describe".to_string(),
            service.name.clone(),
            "--platform=managed".to_string(),
            "--region".to_string(),
            service.region.clone(),
            "--format=json".to_string(),
        ]);
        let output = self.runner.run(GcloudCommand::new(args))?;
        Ok(parse_service_detail_json(&output.stdout)?)
    }

    pub fn load_revisions(
        &self,
        context: &GcloudContext,
        service: &CloudRunService,
    ) -> Result<Vec<CloudRunRevision>, CloudRunLoadError> {
        let mut args = base_args(context);
        args.extend([
            "run".to_string(),
            "revisions".to_string(),
            "list".to_string(),
            "--platform=managed".to_string(),
            "--region".to_string(),
            service.region.clone(),
            "--service".to_string(),
            service.name.clone(),
            "--format=json".to_string(),
        ]);
        let output = self.runner.run(GcloudCommand::new(args))?;
        Ok(parse_revisions_json(&output.stdout)?)
    }

    pub fn load_revision_detail(
        &self,
        context: &GcloudContext,
        region: &str,
        revision: &str,
    ) -> Result<CloudRunRevisionDetail, CloudRunLoadError> {
        let mut args = base_args(context);
        args.extend([
            "run".to_string(),
            "revisions".to_string(),
            "describe".to_string(),
            revision.to_string(),
            "--platform=managed".to_string(),
            "--region".to_string(),
            region.to_string(),
            "--format=json".to_string(),
        ]);
        let output = self.runner.run(GcloudCommand::new(args))?;
        Ok(parse_revision_detail_json(&output.stdout)?)
    }
}

/// Shared leading args: `--configuration <cfg> [--project <p>]`.
fn base_args(context: &GcloudContext) -> Vec<String> {
    let mut args = vec![
        "--configuration".to_string(),
        context.configuration.as_str().to_string(),
    ];
    if let Some(project) = context.project.as_deref() {
        args.push("--project".to_string());
        args.push(project.to_string());
    }
    args
}

// ---------------------------------------------------------------------------
// Raw gcloud JSON shapes (Cloud Run v1 / Knative).
// ---------------------------------------------------------------------------

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawMetadata {
    #[serde(default)]
    name: String,
    #[serde(default)]
    labels: BTreeMap<String, String>,
    #[serde(default)]
    annotations: BTreeMap<String, String>,
    creation_timestamp: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawCondition {
    #[serde(rename = "type")]
    type_: String,
    #[serde(default)]
    status: String,
    reason: Option<String>,
    message: Option<String>,
    last_transition_time: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawTraffic {
    revision_name: Option<String>,
    tag: Option<String>,
    percent: Option<u32>,
    #[serde(default)]
    latest_revision: bool,
    url: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawContainer {
    image: Option<String>,
    resources: Option<RawResources>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawResources {
    #[serde(default)]
    limits: BTreeMap<String, String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawContainerSpec {
    #[serde(default)]
    containers: Vec<RawContainer>,
    container_concurrency: Option<u32>,
    service_account_name: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawServiceStatus {
    url: Option<String>,
    latest_ready_revision_name: Option<String>,
    latest_created_revision_name: Option<String>,
    #[serde(default)]
    traffic: Vec<RawTraffic>,
    #[serde(default)]
    conditions: Vec<RawCondition>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawTemplate {
    #[serde(default)]
    metadata: RawMetadata,
    #[serde(default)]
    spec: RawContainerSpec,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawServiceSpec {
    template: Option<RawTemplate>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawService {
    #[serde(default)]
    metadata: RawMetadata,
    #[serde(default)]
    spec: RawServiceSpec,
    #[serde(default)]
    status: RawServiceStatus,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawRevisionStatus {
    #[serde(default)]
    conditions: Vec<RawCondition>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawRevision {
    #[serde(default)]
    metadata: RawMetadata,
    #[serde(default)]
    spec: RawContainerSpec,
    #[serde(default)]
    status: RawRevisionStatus,
}

// ---------------------------------------------------------------------------
// Parsers.
// ---------------------------------------------------------------------------

pub fn parse_services_json(json: &str) -> Result<Vec<CloudRunService>, CloudRunParseError> {
    let raw: Vec<RawService> = serde_json::from_str(json)?;
    Ok(raw.into_iter().map(build_service_row).collect())
}

pub fn parse_service_detail_json(json: &str) -> Result<CloudRunServiceDetail, CloudRunParseError> {
    let raw: RawService = serde_json::from_str(json)?;
    let value: serde_json::Value = serde_json::from_str(json)?;

    let region = region_from_metadata(&raw.metadata);
    let status = normalize_status(&raw.status.conditions);
    let template = raw.spec.template;
    let template_spec = template.as_ref().map(|t| &t.spec);
    let container = template_spec.and_then(|spec| spec.containers.first());

    let (min_instances, max_instances) = template
        .as_ref()
        .map(|t| scale_annotations(&t.metadata.annotations))
        .unwrap_or((None, None));

    let traffic = raw
        .status
        .traffic
        .iter()
        .map(traffic_target)
        .collect::<Vec<_>>();
    let revisions = derive_revisions_from_status(&raw.status);
    let conditions = raw.status.conditions.iter().map(condition).collect();

    Ok(CloudRunServiceDetail {
        name: raw.metadata.name.clone(),
        region,
        url: raw.status.url,
        status,
        latest_ready_revision: raw.status.latest_ready_revision_name,
        latest_created_revision: raw.status.latest_created_revision_name,
        traffic,
        revisions,
        conditions,
        image: container.and_then(|c| c.image.clone()),
        cpu: container.and_then(container_cpu),
        memory: container.and_then(container_memory),
        concurrency: template_spec.and_then(|spec| spec.container_concurrency),
        service_account: template_spec.and_then(|spec| spec.service_account_name.clone()),
        ingress: raw.metadata.annotations.get(INGRESS_ANNOTATION).cloned(),
        min_instances,
        max_instances,
        labels: key_values(&raw.metadata.labels),
        raw_json: serde_json::to_string_pretty(&value)?,
    })
}

pub fn parse_revisions_json(json: &str) -> Result<Vec<CloudRunRevision>, CloudRunParseError> {
    let raw: Vec<RawRevision> = serde_json::from_str(json)?;
    Ok(raw.into_iter().map(build_revision_row).collect())
}

pub fn parse_revision_detail_json(
    json: &str,
) -> Result<CloudRunRevisionDetail, CloudRunParseError> {
    let raw: RawRevision = serde_json::from_str(json)?;
    let value: serde_json::Value = serde_json::from_str(json)?;

    let region = region_from_metadata(&raw.metadata);
    let ready = normalize_status(&raw.status.conditions);
    let container = raw.spec.containers.first();
    let (min_instances, max_instances) = scale_annotations(&raw.metadata.annotations);

    Ok(CloudRunRevisionDetail {
        name: raw.metadata.name.clone(),
        service: raw.metadata.labels.get(SERVICE_LABEL).cloned(),
        region,
        ready,
        conditions: raw.status.conditions.iter().map(condition).collect(),
        image: container.and_then(|c| c.image.clone()),
        cpu: container.and_then(container_cpu),
        memory: container.and_then(container_memory),
        concurrency: raw.spec.container_concurrency,
        min_instances,
        max_instances,
        service_account: raw.spec.service_account_name,
        created: raw.metadata.creation_timestamp,
        raw_json: serde_json::to_string_pretty(&value)?,
    })
}

// ---------------------------------------------------------------------------
// Row builders / helpers.
// ---------------------------------------------------------------------------

fn build_service_row(raw: RawService) -> CloudRunService {
    let status = normalize_status(&raw.status.conditions);
    let ready_message = ready_condition(&raw.status.conditions).and_then(|c| c.message.clone());

    CloudRunService {
        name: raw.metadata.name.clone(),
        region: region_from_metadata(&raw.metadata),
        url: raw.status.url,
        latest_ready_revision: raw.status.latest_ready_revision_name,
        latest_created_revision: raw.status.latest_created_revision_name,
        traffic_summary: traffic_summary(&raw.status.traffic),
        status,
        ready_message,
    }
}

fn build_revision_row(raw: RawRevision) -> CloudRunRevision {
    let ready = normalize_status(&raw.status.conditions);
    let image = raw.spec.containers.first().and_then(|c| c.image.clone());

    CloudRunRevision {
        name: raw.metadata.name,
        ready,
        created: raw.metadata.creation_timestamp,
        image,
        active_percent: None,
    }
}

/// Best-effort revision rows embedded in a service `describe` response, derived
/// from the traffic split. A follow-up `revisions list` refreshes true health.
fn derive_revisions_from_status(status: &RawServiceStatus) -> Vec<CloudRunRevision> {
    let latest_ready = status.latest_ready_revision_name.as_deref();
    let mut seen: Vec<String> = Vec::new();
    let mut revisions = Vec::new();

    for target in &status.traffic {
        let Some(name) = target.revision_name.clone() else {
            continue;
        };
        if seen.contains(&name) {
            continue;
        }
        seen.push(name.clone());
        let ready = if Some(name.as_str()) == latest_ready {
            CloudRunServiceStatus::Ready
        } else {
            CloudRunServiceStatus::Unknown(String::new())
        };
        revisions.push(CloudRunRevision {
            name,
            ready,
            created: None,
            image: None,
            active_percent: target.percent,
        });
    }

    revisions
}

fn ready_condition(conditions: &[RawCondition]) -> Option<&RawCondition> {
    conditions.iter().find(|c| c.type_ == "Ready")
}

/// Normalize the `Ready` condition into a domain status.
///
/// - `status == "True"` => `Ready`
/// - `status == "False"` with a reason => `Failed`, without => `NotReady`
/// - other status while `RoutesReady`/`ConfigurationsReady` are in progress
///   => `Reconciling`
/// - anything else => `Unknown(status_string)`
fn normalize_status(conditions: &[RawCondition]) -> CloudRunServiceStatus {
    let Some(ready) = ready_condition(conditions) else {
        return CloudRunServiceStatus::Unknown(String::new());
    };

    match ready.status.as_str() {
        "True" => CloudRunServiceStatus::Ready,
        "False" => {
            if ready.reason.is_some() {
                CloudRunServiceStatus::Failed
            } else {
                CloudRunServiceStatus::NotReady
            }
        }
        other => {
            if in_progress(conditions) {
                CloudRunServiceStatus::Reconciling
            } else {
                CloudRunServiceStatus::Unknown(other.to_string())
            }
        }
    }
}

fn in_progress(conditions: &[RawCondition]) -> bool {
    conditions.iter().any(|c| {
        matches!(c.type_.as_str(), "RoutesReady" | "ConfigurationsReady") && c.status != "True"
    })
}

/// Highest-percent traffic target rendered `"<revision> <pct>%"`, else `"-"`.
fn traffic_summary(traffic: &[RawTraffic]) -> String {
    traffic_summary_from(
        &traffic
            .iter()
            .filter_map(|t| {
                t.revision_name
                    .as_deref()
                    .map(|name| (name, i64::from(t.percent.unwrap_or(0))))
            })
            .collect::<Vec<_>>(),
    )
}

/// The busiest target, rendered `"<revision> <n>%"`, or `"-"` when nothing
/// carries traffic. Shared with the native path: the cell shows this string
/// verbatim, so the two backends must not word it differently.
/// `i64` because the two backends carry the percentage differently — the
/// CLI's JSON as an unsigned value, the client as a signed one — and the
/// widest of the two loses nothing.
pub fn traffic_summary_from(targets: &[(&str, i64)]) -> String {
    targets
        .iter()
        .max_by_key(|(_, percent)| *percent)
        .map(|(revision, percent)| format!("{revision} {percent}%"))
        .unwrap_or_else(|| "-".to_string())
}

fn region_from_metadata(metadata: &RawMetadata) -> String {
    metadata
        .labels
        .get(LOCATION_LABEL)
        .or_else(|| metadata.annotations.get(LOCATION_ANNOTATION))
        .cloned()
        .unwrap_or_default()
}

fn scale_annotations(annotations: &BTreeMap<String, String>) -> (Option<u32>, Option<u32>) {
    let min = annotations
        .get(MIN_SCALE_ANNOTATION)
        .and_then(|value| value.parse().ok());
    let max = annotations
        .get(MAX_SCALE_ANNOTATION)
        .and_then(|value| value.parse().ok());
    (min, max)
}

fn container_cpu(container: &RawContainer) -> Option<String> {
    container
        .resources
        .as_ref()
        .and_then(|r| r.limits.get("cpu").cloned())
}

fn container_memory(container: &RawContainer) -> Option<String> {
    container
        .resources
        .as_ref()
        .and_then(|r| r.limits.get("memory").cloned())
}

fn traffic_target(raw: &RawTraffic) -> CloudRunTrafficTarget {
    CloudRunTrafficTarget {
        revision_name: raw.revision_name.clone(),
        tag: raw.tag.clone(),
        percent: raw.percent,
        latest: raw.latest_revision,
        url: raw.url.clone(),
    }
}

fn condition(raw: &RawCondition) -> CloudRunCondition {
    CloudRunCondition {
        type_: raw.type_.clone(),
        status: raw.status.clone(),
        reason: raw.reason.clone(),
        message: raw.message.clone(),
        last_transition_time: raw.last_transition_time.clone(),
    }
}

fn key_values(map: &BTreeMap<String, String>) -> Vec<ComputeKeyValue> {
    map.iter()
        .map(|(key, value)| ComputeKeyValue {
            key: key.clone(),
            value: value.clone(),
        })
        .collect()
}
