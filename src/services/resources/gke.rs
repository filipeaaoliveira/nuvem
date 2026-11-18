use std::collections::BTreeMap;

use serde::Deserialize;

use nuvem_domain::gcp::configuration::GcloudContext;
use nuvem_domain::gcp::filter::label_filter_field;
use nuvem_domain::gcp::resource::{
    ComputeKeyValue, EndpointVisibility, GkeAddon, GkeCluster, GkeClusterDetail, GkeClusterStatus,
    GkeNodePool, ResourceServiceKind,
};

use crate::services::gcloud::cli::{CommandError, CommandRunner, GcloudCommand};
use crate::services::resources::list_filter::{RESOURCE_LIST_LIMIT, pushdown_expression};

/// Verbatim-superset raw fields eligible for the server-side filter pushdown
/// in [`GcloudGkeService::load_clusters`]. Deliberately excludes
/// `currentNodeCount`: it is a derived numeric column with no `~`-safe raw
/// string counterpart (a `~` regex clause on an integer field risks a gcloud
/// runtime error), so purely-numeric terms are gated client-side instead —
/// see the numeric check in `load_clusters`. See
/// `crate::services::resources::list_filter` for the never-drop-a-row
/// reasoning.
const FILTER_FIELDS: &[&str] = &["name", "location", "zone", "status", "currentMasterVersion"];

/// Opaque display labels for [`GkeClusterStatus`] and [`EndpointVisibility`]
/// (derived, not verbatim in the raw JSON pushed above) — a term that is a
/// substring of one of these is gated out of the pushdown so it stays
/// client-side-only.
const OPAQUE_LABELS: &[&str] = &[
    "running",
    "reconciling",
    "stopping",
    "error",
    "unknown",
    "public",
    "private",
];

#[derive(Debug, thiserror::Error)]
pub enum GkeParseError {
    #[error("invalid GKE cluster JSON: {0}")]
    Json(#[from] serde_json::Error),
}

#[derive(Debug, thiserror::Error)]
pub enum GkeLoadError {
    #[error("{0}")]
    Command(#[from] CommandError),
    #[error("{0}")]
    Parse(#[from] GkeParseError),
}

pub struct GcloudGkeService<R> {
    runner: R,
}

impl<R> GcloudGkeService<R> {
    pub fn new(runner: R) -> Self {
        Self { runner }
    }

    pub fn runner(&self) -> &R {
        &self.runner
    }
}

impl<R: CommandRunner> GcloudGkeService<R> {
    /// Lists GKE clusters, capped at [`RESOURCE_LIST_LIMIT`] rows and, when
    /// `filter` is server-expressible, narrowed with a pushed-down
    /// `--filter` expression. The caller still re-applies the exact
    /// client-side filter to the (superset) result.
    pub fn load_clusters(
        &self,
        context: &GcloudContext,
        filter: Option<&str>,
    ) -> Result<Vec<GkeCluster>, GkeLoadError> {
        let mut args: Vec<String> = vec![
            "--configuration".to_string(),
            context.configuration.as_str().to_string(),
            "container".to_string(),
            "clusters".to_string(),
            "list".to_string(),
            "--format=json".to_string(),
            format!("--limit={RESOURCE_LIST_LIMIT}"),
        ];
        let expression = filter.and_then(|applied| {
            let trimmed = applied.trim();
            // Node count is a derived numeric column with no ~-safe raw
            // string field; a term that could match it stays client-side
            // rather than risk dropping a row. Since the term is a regex and
            // `.` matches any character, "could match a digit string" means
            // "is made only of digits and dots" — `1.27` matches the node
            // count `1127`, so it is gated too.
            if !trimmed.is_empty() && trimmed.chars().all(|c| c.is_ascii_digit() || c == '.') {
                return None;
            }
            pushdown_expression(
                applied,
                FILTER_FIELDS,
                OPAQUE_LABELS,
                label_filter_field(ResourceServiceKind::GkeClusters),
            )
        });
        if let Some(expression) = expression {
            args.push(format!("--filter={expression}"));
        }
        let output = self.runner.run(GcloudCommand::new(args))?;
        Ok(parse_clusters_json(&output.stdout)?)
    }

    pub fn load_cluster_detail(
        &self,
        context: &GcloudContext,
        cluster: &GkeCluster,
    ) -> Result<GkeClusterDetail, GkeLoadError> {
        let output = self.runner.run(GcloudCommand::new([
            "--configuration",
            context.configuration.as_str(),
            "container",
            "clusters",
            "describe",
            cluster.name.as_str(),
            "--location",
            cluster.location.as_str(),
            "--format=json",
        ]))?;
        Ok(parse_cluster_detail_json(&output.stdout)?)
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawCluster {
    name: String,
    location: Option<String>,
    zone: Option<String>,
    status: String,
    current_master_version: Option<String>,
    current_node_count: Option<u32>,
    endpoint: Option<String>,
    private_cluster_config: Option<RawPrivateClusterConfig>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawClusterDetail {
    name: String,
    location: Option<String>,
    zone: Option<String>,
    status: String,
    current_master_version: Option<String>,
    current_node_version: Option<String>,
    endpoint: Option<String>,
    network: Option<String>,
    subnetwork: Option<String>,
    #[serde(default)]
    resource_labels: BTreeMap<String, String>,
    private_cluster_config: Option<RawPrivateClusterConfig>,
    #[serde(default)]
    node_pools: Vec<RawNodePool>,
    addons_config: Option<RawAddonsConfig>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawPrivateClusterConfig {
    #[serde(default)]
    enable_private_endpoint: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawNodePool {
    name: String,
    status: String,
    version: Option<String>,
    initial_node_count: Option<u32>,
    config: Option<RawNodePoolConfig>,
    autoscaling: Option<RawNodePoolAutoscaling>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawNodePoolConfig {
    machine_type: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawNodePoolAutoscaling {
    #[serde(default)]
    enabled: bool,
    min_node_count: Option<u32>,
    max_node_count: Option<u32>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawAddonsConfig {
    http_load_balancing: Option<RawDisabledAddon>,
    horizontal_pod_autoscaling: Option<RawDisabledAddon>,
    gce_persistent_disk_csi_driver_config: Option<RawEnabledAddon>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawDisabledAddon {
    #[serde(default)]
    disabled: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawEnabledAddon {
    #[serde(default)]
    enabled: bool,
}

pub fn parse_clusters_json(json: &str) -> Result<Vec<GkeCluster>, GkeParseError> {
    let raw: Vec<RawCluster> = serde_json::from_str(json)?;
    Ok(raw
        .into_iter()
        .map(|item| {
            let private = item
                .private_cluster_config
                .as_ref()
                .map(|config| config.enable_private_endpoint)
                .unwrap_or(false);

            GkeCluster {
                name: item.name,
                location: item.location.or(item.zone).unwrap_or_default(),
                status: parse_cluster_status(&item.status),
                current_master_version: item.current_master_version,
                current_node_count: item.current_node_count,
                endpoint_visibility: endpoint_visibility(item.endpoint.as_ref(), private),
            }
        })
        .collect())
}

pub fn parse_cluster_detail_json(json: &str) -> Result<GkeClusterDetail, GkeParseError> {
    let raw: RawClusterDetail = serde_json::from_str(json)?;
    let formatted: serde_json::Value = serde_json::from_str(json)?;
    let private_cluster = raw
        .private_cluster_config
        .as_ref()
        .map(|config| config.enable_private_endpoint)
        .unwrap_or(false);
    let labels = raw
        .resource_labels
        .into_iter()
        .map(|(key, value)| ComputeKeyValue { key, value })
        .collect();

    Ok(GkeClusterDetail {
        name: raw.name,
        location: raw.location.or(raw.zone).unwrap_or_default(),
        status: parse_cluster_status(&raw.status),
        current_master_version: raw.current_master_version,
        current_node_version: raw.current_node_version,
        endpoint: raw.endpoint,
        private_cluster,
        network: raw.network.map(|value| tail_segment(&value)),
        subnetwork: raw.subnetwork.map(|value| tail_segment(&value)),
        node_pools: raw
            .node_pools
            .into_iter()
            .map(|pool| GkeNodePool {
                name: pool.name,
                status: parse_cluster_status(&pool.status),
                version: pool.version,
                machine_type: pool.config.and_then(|config| config.machine_type),
                autoscaling: pool.autoscaling.map(format_autoscaling),
                node_count: pool.initial_node_count,
            })
            .collect(),
        addons: parse_addons(raw.addons_config),
        labels,
        raw_json: serde_json::to_string_pretty(&formatted)?,
    })
}

pub fn parse_cluster_status(value: &str) -> GkeClusterStatus {
    match value {
        "RUNNING" => GkeClusterStatus::Running,
        "RECONCILING" => GkeClusterStatus::Reconciling,
        "STOPPING" => GkeClusterStatus::Stopping,
        "ERROR" => GkeClusterStatus::Error,
        other => GkeClusterStatus::Unknown(other.to_string()),
    }
}

pub fn endpoint_visibility(endpoint: Option<&String>, private: bool) -> EndpointVisibility {
    if private {
        EndpointVisibility::Private
    } else if endpoint.is_some() {
        EndpointVisibility::Public
    } else {
        EndpointVisibility::Unknown
    }
}

pub fn tail_segment(value: &str) -> String {
    value.rsplit('/').next().unwrap_or(value).to_string()
}

fn format_autoscaling(autoscaling: RawNodePoolAutoscaling) -> String {
    autoscaling_label(
        autoscaling.enabled,
        autoscaling.min_node_count,
        autoscaling.max_node_count,
    )
}

/// The autoscaling cell, from the three values both backends carry.
///
/// Shared so a pool reads the same whichever path filled it; a second copy of
/// the "off" wording is how the two would drift.
pub fn autoscaling_label(enabled: bool, min: Option<u32>, max: Option<u32>) -> String {
    if !enabled {
        "off".to_string()
    } else {
        format!("{}-{}", min.unwrap_or_default(), max.unwrap_or_default())
    }
}

/// The addon rows, in the order the pane lists them.
///
/// Each entry is `(display name, Some(enabled))`, or `None` where the cluster
/// did not report that addon at all -- which is not the same as reporting it
/// off, and is why an absent block yields no row rather than a disabled one.
pub fn addons_from(entries: &[(&str, Option<bool>)]) -> Vec<GkeAddon> {
    entries
        .iter()
        .filter_map(|(name, enabled)| {
            enabled.map(|enabled| GkeAddon {
                name: (*name).to_string(),
                enabled,
            })
        })
        .collect()
}

fn parse_addons(addons: Option<RawAddonsConfig>) -> Vec<GkeAddon> {
    let Some(addons) = addons else {
        return Vec::new();
    };

    addons_from(&[
        (
            HTTP_LOAD_BALANCING,
            addons.http_load_balancing.map(|addon| !addon.disabled),
        ),
        (
            HORIZONTAL_POD_AUTOSCALING,
            addons
                .horizontal_pod_autoscaling
                .map(|addon| !addon.disabled),
        ),
        (
            GCE_PD_CSI_DRIVER,
            addons
                .gce_persistent_disk_csi_driver_config
                .map(|addon| addon.enabled),
        ),
    ])
}

/// The addon display names, shared so the two backends cannot disagree.
pub const HTTP_LOAD_BALANCING: &str = "HTTP load balancing";
pub const HORIZONTAL_POD_AUTOSCALING: &str = "Horizontal Pod Autoscaling";
pub const GCE_PD_CSI_DRIVER: &str = "GCE Persistent Disk CSI Driver";
