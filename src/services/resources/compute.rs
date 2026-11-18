use std::collections::BTreeMap;

use serde::Deserialize;
use tokio_util::sync::CancellationToken;

use nuvem_domain::gcp::configuration::GcloudContext;
use nuvem_domain::gcp::filter::label_filter_field;
use nuvem_domain::gcp::resource::{
    ComputeDisk, ComputeInstance, ComputeInstanceDetail, ComputeKeyValue, ComputeNetworkInterface,
    ResourceServiceKind, ResourceStatus,
};

use crate::app::state::resources::{ComputeInstanceAction, compute_lifecycle_argv};
use crate::services::gcloud::cli::{CommandCeiling, CommandError, CommandRunner, GcloudCommand};
use crate::services::resources::list_filter::{RESOURCE_LIST_LIMIT, pushdown_expression};

/// Verbatim-superset raw fields eligible for the server-side filter pushdown
/// in [`GcloudComputeService::load_instances`]. See
/// `crate::services::resources::list_filter` for the never-drop-a-row
/// reasoning.
const FILTER_FIELDS: &[&str] = &[
    "name",
    "zone",
    "status",
    "machineType",
    "networkInterfaces.networkIP",
    "networkInterfaces.accessConfigs.natIP",
];

/// Opaque display labels for [`ResourceStatus`] (derived, not verbatim in the
/// raw JSON pushed above) — a term that is a substring of one of these is
/// gated out of the pushdown so it stays client-side-only.
const OPAQUE_LABELS: &[&str] = &["running", "stopped", "suspended", "pending", "unknown"];

#[derive(Debug, thiserror::Error)]
pub enum ComputeParseError {
    #[error("invalid compute instance JSON: {0}")]
    Json(#[from] serde_json::Error),
}

#[derive(Debug, thiserror::Error)]
pub enum ComputeLoadError {
    #[error("{0}")]
    Command(#[from] CommandError),
    #[error("{0}")]
    Parse(#[from] ComputeParseError),
}

pub struct GcloudComputeService<R> {
    runner: R,
}

impl<R> GcloudComputeService<R> {
    pub fn new(runner: R) -> Self {
        Self { runner }
    }

    pub fn runner(&self) -> &R {
        &self.runner
    }
}

impl<R: CommandRunner> GcloudComputeService<R> {
    /// Lists compute instances, capped at [`RESOURCE_LIST_LIMIT`] rows and,
    /// when `filter` is server-expressible, narrowed with a pushed-down
    /// `--filter` expression. The caller still re-applies the exact
    /// client-side filter to the (superset) result.
    pub fn load_instances(
        &self,
        context: &GcloudContext,
        filter: Option<&str>,
    ) -> Result<Vec<ComputeInstance>, ComputeLoadError> {
        let mut args: Vec<String> = vec![
            "--configuration".to_string(),
            context.configuration.as_str().to_string(),
            "compute".to_string(),
            "instances".to_string(),
            "list".to_string(),
            "--format=json".to_string(),
            format!("--limit={RESOURCE_LIST_LIMIT}"),
        ];
        if let Some(expression) = filter.and_then(|applied| {
            pushdown_expression(
                applied,
                FILTER_FIELDS,
                OPAQUE_LABELS,
                label_filter_field(ResourceServiceKind::ComputeInstances),
            )
        }) {
            args.push(format!("--filter={expression}"));
        }
        let output = self.runner.run(GcloudCommand::new(args))?;
        Ok(parse_instances_json(&output.stdout)?)
    }

    pub fn load_instance_detail(
        &self,
        context: &GcloudContext,
        instance: &ComputeInstance,
    ) -> Result<ComputeInstanceDetail, ComputeLoadError> {
        let output = self.runner.run(GcloudCommand::new([
            "--configuration",
            context.configuration.as_str(),
            "compute",
            "instances",
            "describe",
            instance.name.as_str(),
            "--zone",
            instance.zone.as_str(),
            "--format=json",
        ]))?;
        Ok(parse_instance_detail_json(&output.stdout)?)
    }

    pub fn start_instance(
        &self,
        context: &GcloudContext,
        instance: &ComputeInstance,
    ) -> Result<(), ComputeLoadError> {
        self.run_instance_action(context, instance, ComputeInstanceAction::Start)
    }

    pub fn stop_instance(
        &self,
        context: &GcloudContext,
        instance: &ComputeInstance,
    ) -> Result<(), ComputeLoadError> {
        self.run_instance_action(context, instance, ComputeInstanceAction::Stop)
    }

    pub fn reset_instance(
        &self,
        context: &GcloudContext,
        instance: &ComputeInstance,
    ) -> Result<(), ComputeLoadError> {
        self.run_instance_action(context, instance, ComputeInstanceAction::Reset)
    }

    pub fn suspend_instance(
        &self,
        context: &GcloudContext,
        instance: &ComputeInstance,
    ) -> Result<(), ComputeLoadError> {
        self.run_instance_action(context, instance, ComputeInstanceAction::Suspend)
    }

    pub fn resume_instance(
        &self,
        context: &GcloudContext,
        instance: &ComputeInstance,
    ) -> Result<(), ComputeLoadError> {
        self.run_instance_action(context, instance, ComputeInstanceAction::Resume)
    }

    pub fn run_instance_action(
        &self,
        context: &GcloudContext,
        instance: &ComputeInstance,
        action: ComputeInstanceAction,
    ) -> Result<(), ComputeLoadError> {
        self.run_action_by_id(context, &instance.name, &instance.zone, action)
    }

    /// Runs an instance action against a target identified only by `(name,
    /// zone)`, which is all a batch target carries. Issues exactly the argv
    /// [`GcloudComputeService::run_instance_action`] issues, so the batch and
    /// single-row paths cannot drift apart.
    ///
    /// Delegates to [`Self::run_action_by_id_with_cancellation`] with a token
    /// nobody holds, rather than carrying a second copy of the argv: a runner
    /// handed a fresh token behaves exactly as `run` does, and one builder
    /// means the cancellable and uncancellable forms cannot drift either.
    pub fn run_action_by_id(
        &self,
        context: &GcloudContext,
        name: &str,
        zone: &str,
        action: ComputeInstanceAction,
    ) -> Result<(), ComputeLoadError> {
        self.run_action_by_id_with_cancellation(
            context,
            name,
            zone,
            action,
            &CancellationToken::new(),
        )
    }

    /// The same action, abandonable by whoever holds `cancellation`.
    ///
    /// The runtime hands this the session's shutdown token so an action still
    /// in flight when the operator presses `q` does not hold `Runtime::drop`
    /// for the thirty minutes its ceiling allows.
    ///
    /// Cancelling does not cancel the operation. gcloud is killed and nuvem
    /// stops waiting; a `stop` GCP has already accepted proceeds regardless,
    /// which is what [`CommandError::Cancelled`]'s lifecycle caveat says.
    pub fn run_action_by_id_with_cancellation(
        &self,
        context: &GcloudContext,
        name: &str,
        zone: &str,
        action: ComputeInstanceAction,
        cancellation: &CancellationToken,
    ) -> Result<(), ComputeLoadError> {
        // The lifecycle ceiling, not the read one: the argv carries no
        // `--async`, so gcloud blocks until GCP finishes the operation, and a
        // large instance's `stop` waits out the guest OS shutdown well past the
        // two minutes a listing is given. The cancellable call keeps
        // that ceiling — `run_with_cancellation` would have dropped it.
        self.runner.run_bounded_with_cancellation(
            GcloudCommand::new(compute_lifecycle_argv(context, name, zone, action))
                .with_ceiling(CommandCeiling::Lifecycle),
            cancellation,
        )?;
        Ok(())
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawInstance {
    id: String,
    name: String,
    zone: String,
    status: String,
    machine_type: String,
    #[serde(default)]
    network_interfaces: Vec<RawNetworkInterface>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawNetworkInterface {
    name: Option<String>,
    network: Option<String>,
    subnetwork: Option<String>,
    #[serde(rename = "networkIP")]
    network_ip: Option<String>,
    #[serde(default)]
    access_configs: Vec<RawAccessConfig>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawAccessConfig {
    #[serde(rename = "natIP")]
    nat_ip: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawInstanceDetail {
    name: String,
    zone: String,
    status: String,
    machine_type: String,
    creation_timestamp: Option<String>,
    #[serde(default)]
    labels: BTreeMap<String, String>,
    #[serde(default)]
    metadata: RawMetadata,
    #[serde(default)]
    network_interfaces: Vec<RawNetworkInterface>,
    #[serde(default)]
    disks: Vec<RawDisk>,
    /// Optional rather than `#[serde(default)]`, so "the key was not there"
    /// stays distinguishable from "the key was there and empty" at the
    /// boundary instead of being erased before anyone can ask.
    ///
    /// The two are folded to the same answer one function down, and that fold
    /// is the deliberate step: for a full `instances describe` response an
    /// absent `tags` really does mean the instance has none. It would *not*
    /// mean that for a partial projection or a response a permission filter
    /// had trimmed — GCP masks denials rather than announcing them — and this
    /// is the seam where a third answer would go if nuvem ever reads instances
    /// that way.
    tags: Option<RawTags>,
    service_accounts: Option<Vec<RawServiceAccount>>,
}

/// `tags` is an object with an `items` list, not a bare list — the shape
/// `compute/v1/instances` documents. The wrapper also carries a fingerprint,
/// which is a concurrency token for writes and nothing this application reads.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawTags {
    #[serde(default)]
    items: Vec<String>,
}

/// Only the email. The `scopes` list beside it says what the account may do,
/// which is an authorization question; a firewall rule selects on the identity
/// alone.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawServiceAccount {
    #[serde(default)]
    email: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawMetadata {
    #[serde(default)]
    items: Vec<RawMetadataItem>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawMetadataItem {
    key: String,
    value: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawDisk {
    #[serde(rename = "deviceName")]
    device_name: Option<String>,
    source: Option<String>,
    #[serde(default)]
    boot: bool,
    #[serde(default)]
    auto_delete: bool,
    #[serde(rename = "type")]
    disk_type: Option<String>,
    #[serde(rename = "diskSizeGb")]
    disk_size_gb: Option<String>,
}

pub fn parse_instances_json(json: &str) -> Result<Vec<ComputeInstance>, ComputeParseError> {
    let raw: Vec<RawInstance> = serde_json::from_str(json)?;
    Ok(raw
        .into_iter()
        .map(|item| {
            let first_network = item.network_interfaces.first();
            ComputeInstance {
                id: item.id,
                name: item.name,
                zone: tail_segment(&item.zone),
                status: parse_status(&item.status),
                machine_type: tail_segment(&item.machine_type),
                internal_ip: first_network.and_then(|network| network.network_ip.clone()),
                external_ip: first_network
                    .and_then(|network| network.access_configs.first())
                    .and_then(|access| access.nat_ip.clone()),
            }
        })
        .collect())
}

pub fn parse_instance_detail_json(json: &str) -> Result<ComputeInstanceDetail, ComputeParseError> {
    let raw: RawInstanceDetail = serde_json::from_str(json)?;
    let formatted: serde_json::Value = serde_json::from_str(json)?;
    let labels = raw
        .labels
        .into_iter()
        .map(|(key, value)| ComputeKeyValue { key, value })
        .collect();
    let metadata = raw
        .metadata
        .items
        .into_iter()
        .map(|item| ComputeKeyValue {
            key: item.key,
            value: item.value.unwrap_or_default(),
        })
        .collect();

    Ok(ComputeInstanceDetail {
        name: raw.name,
        zone: tail_segment(&raw.zone),
        status: parse_status(&raw.status),
        machine_type: tail_segment(&raw.machine_type),
        creation_timestamp: raw.creation_timestamp,
        network_interfaces: raw
            .network_interfaces
            .into_iter()
            .map(|network| ComputeNetworkInterface {
                name: network.name,
                network: network.network.map(|value| tail_segment(&value)),
                subnetwork: network.subnetwork.as_deref().map(tail_segment),
                subnetwork_region: network
                    .subnetwork
                    .as_deref()
                    .and_then(|value| segment_after(value, "regions")),
                internal_ip: network.network_ip,
                external_ip: network
                    .access_configs
                    .first()
                    .and_then(|access| access.nat_ip.clone()),
            })
            .collect(),
        disks: raw
            .disks
            .into_iter()
            .map(|disk| ComputeDisk {
                name: disk.device_name,
                source: disk.source.map(|value| tail_segment(&value)),
                boot: disk.boot,
                auto_delete: disk.auto_delete,
                disk_type: disk.disk_type,
                size_gb: disk.disk_size_gb,
            })
            .collect(),
        // Absent and present-but-empty both mean "this instance carries none",
        // which is what a full `describe` response makes them mean. See the
        // note on `RawInstanceDetail::tags` for when they would have to differ.
        tags: raw.tags.map(|tags| tags.items).unwrap_or_default(),
        // Emails only, and empties dropped: an account entry with no email
        // identifies nothing, and carrying it would let a firewall rule whose
        // selector rendered empty appear to match.
        service_accounts: raw
            .service_accounts
            .unwrap_or_default()
            .into_iter()
            .map(|account| account.email)
            .filter(|email| !email.is_empty())
            .collect(),
        labels,
        metadata,
        raw_json: serde_json::to_string_pretty(&formatted)?,
    })
}

/// Shared with the native path so both backends land on the same
/// `ResourceStatus` for the same wire word; duplicating this is how the two
/// would drift.
pub fn parse_status(value: &str) -> ResourceStatus {
    match value {
        "RUNNING" => ResourceStatus::Running,
        "TERMINATED" | "STOPPED" => ResourceStatus::Stopped,
        "SUSPENDED" => ResourceStatus::Suspended,
        "PROVISIONING" | "STAGING" | "STOPPING" | "SUSPENDING" | "RESUMING" => {
            ResourceStatus::Pending
        }
        other => ResourceStatus::Unknown(other.to_string()),
    }
}

pub fn tail_segment(value: &str) -> String {
    value.rsplit('/').next().unwrap_or(value).to_string()
}

/// The path segment following `marker` in a GCP self-link, if there is one.
///
/// Read structurally — "the segment after `regions`" — rather than by counting
/// back from the end, so a link nested differently than
/// `.../regions/<r>/subnetworks/<s>` either answers correctly or answers
/// `None`. Counting positions would keep answering, with whatever segment
/// happened to sit there.
///
/// `None` is the safe verdict and the caller treats it as one: a subnet region
/// that cannot be read leaves `{subnetwork_region}` absent, so a jump keyed on
/// it produces no offer rather than an offer that lands on the wrong row.
pub fn segment_after(value: &str, marker: &str) -> Option<String> {
    let mut segments = value.split('/');
    segments.find(|segment| *segment == marker)?;
    segments
        .next()
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}
