//! Compute Engine persistent disks, list only.
//!
//! # Why one call covers the project
//!
//! `compute disks list` is project-scoped and returns zonal and regional disks
//! in one array, so there is no per-location fan-out to do and no `--zones=-`
//! equivalent to pass. The two kinds are not interchangeable in shape, though:
//! a zonal disk carries `zone` and a regional one carries `region`, and
//! `RawDisk` reads both into one `location` for the reason recorded on
//! [`nuvem_domain::gcp::resource::PersistentDisk`].
//!
//! # What the row set is for
//!
//! The orphan badge is the point of this service, and it is derived here rather
//! than fetched: the API has no `orphan` field, only `users`, and `users` is
//! absent rather than empty when nothing is attached because gcloud omits empty
//! arrays. That absence is the whole signal, which is why every field below
//! defaults instead of erroring — a row that failed to parse would leave a real
//! orphan off a cost-cleanup screen.

use serde::Deserialize;

use nuvem_domain::gcp::configuration::GcloudContext;
use nuvem_domain::gcp::filter::label_filter_field;
use nuvem_domain::gcp::resource::{
    ComputeKeyValue, DiskStatus, PersistentDisk, ResourceServiceKind,
};

use crate::services::gcloud::cli::{CommandError, CommandRunner, GcloudCommand};
use crate::services::resources::list_filter::{RESOURCE_LIST_LIMIT, pushdown_expression};

/// Verbatim-superset raw fields eligible for the regex pushdown — deliberately
/// none, as for Cloud Functions.
///
/// The never-drop-a-row invariant is a claim about *display* values: each
/// searchable field must be a verbatim substring of a pushed raw field, or be
/// enumerated in [`OPAQUE_LABELS`]. Three of this service's searchable values
/// appear in no raw gcloud field at all — the status *label* (`Ready`, not
/// `READY`), the `ORPHAN` badge, which is derived here and exists nowhere on
/// the wire, and the rendered size (`20 GB`, not `"20"`). Pushing a clause
/// would mean enumerating each of them. Empty degrades to full client-side
/// narrowing, which is the safe direction.
const FILTER_FIELDS: &[&str] = &[];

/// Empty for the same reason [`FILTER_FIELDS`] is: with nothing pushed there is
/// nothing for the gate to protect.
const OPAQUE_LABELS: &[&str] = &[];

#[derive(Debug, thiserror::Error)]
pub enum DisksParseError {
    #[error("invalid persistent disks JSON: {0}")]
    Json(#[from] serde_json::Error),
}

#[derive(Debug, thiserror::Error)]
pub enum DisksLoadError {
    #[error("{0}")]
    Command(#[from] CommandError),
    #[error("{0}")]
    Parse(#[from] DisksParseError),
}

pub struct GcloudDisksService<R> {
    runner: R,
}

impl<R> GcloudDisksService<R> {
    pub fn new(runner: R) -> Self {
        Self { runner }
    }

    pub fn runner(&self) -> &R {
        &self.runner
    }
}

impl<R: CommandRunner> GcloudDisksService<R> {
    /// Lists a project's persistent disks, capped at [`RESOURCE_LIST_LIMIT`]
    /// rows and, in label mode, narrowed with a pushed-down `--filter` on
    /// `labels`.
    ///
    /// Only label mode reaches the server, for the reason on `FILTER_FIELDS`.
    pub fn load_disks(
        &self,
        context: &GcloudContext,
        filter: Option<&str>,
    ) -> Result<Vec<PersistentDisk>, DisksLoadError> {
        let mut args = base_args(context);
        args.extend([
            "compute".to_string(),
            "disks".to_string(),
            "list".to_string(),
            "--format=json".to_string(),
            format!("--limit={RESOURCE_LIST_LIMIT}"),
        ]);
        if let Some(expression) = filter.and_then(|applied| {
            pushdown_expression(
                applied,
                FILTER_FIELDS,
                OPAQUE_LABELS,
                label_filter_field(ResourceServiceKind::PersistentDisks),
            )
        }) {
            args.push(format!("--filter={expression}"));
        }
        let output = self.runner.run(GcloudCommand::new(args))?;
        Ok(parse_disks_json(&output.stdout)?)
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

/// The wire shape of a disk, list format.
///
/// Every field defaults, so a row missing any of them still parses; only a
/// document that is not the expected JSON fails the load, which is the same
/// bargain the sibling services strike.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawDisk {
    #[serde(default)]
    name: String,
    #[serde(default)]
    zone: Option<String>,
    #[serde(default)]
    region: Option<String>,
    #[serde(default)]
    status: Option<String>,
    /// A JSON *string*, not a number — the Compute API renders 64-bit integers
    /// as strings. Deserializing it as `u64` would fail the whole document.
    #[serde(default)]
    size_gb: Option<String>,
    #[serde(default, rename = "type")]
    disk_type: Option<String>,
    #[serde(default)]
    users: Vec<String>,
    #[serde(default)]
    labels: std::collections::BTreeMap<String, String>,
}

pub fn parse_disks_json(stdout: &str) -> Result<Vec<PersistentDisk>, DisksParseError> {
    let raw: Vec<RawDisk> = serde_json::from_str(stdout)?;
    Ok(raw.into_iter().map(disk_from_raw).collect())
}

fn disk_from_raw(raw: RawDisk) -> PersistentDisk {
    PersistentDisk {
        name: short_id(&raw.name),
        // `zone` first: a disk has exactly one of the two, so the order only
        // decides which wins in a shape that cannot occur, and zonal is the
        // overwhelmingly common case.
        location: raw
            .zone
            .as_deref()
            .or(raw.region.as_deref())
            .map(short_id)
            .unwrap_or_default(),
        status: DiskStatus::from_api(raw.status.as_deref()),
        // An unparseable size is 0 rather than an error, matching how every
        // other field here degrades. A 0 GB disk does not exist, so the cell
        // reads as the "not reported" it is.
        size_gb: raw
            .size_gb
            .as_deref()
            .and_then(|size| size.parse().ok())
            .unwrap_or(0),
        disk_type: raw.disk_type.as_deref().map(short_id).unwrap_or_default(),
        users: raw.users.iter().map(|user| short_id(user)).collect(),
        labels: key_values(raw.labels),
    }
}

/// The trailing segment of a fully-qualified resource path or URL.
///
/// The API returns `https://.../zones/europe-west1-b`; a table cell wants
/// `europe-west1-b`. A value with no slash is already short and passes through
/// unchanged.
pub fn short_id(resource_path: &str) -> String {
    resource_path
        .rsplit('/')
        .next()
        .unwrap_or(resource_path)
        .to_string()
}

/// Sorted key/value pairs. Sorted rather than in map order so a reload does not
/// reshuffle them under the operator's cursor.
pub fn key_values(map: std::collections::BTreeMap<String, String>) -> Vec<ComputeKeyValue> {
    map.into_iter()
        .map(|(key, value)| ComputeKeyValue { key, value })
        .collect()
}
