//! Detail-pane body model: display text paired with the copy payload each
//! line carries.
//!
//! Detail bodies are built here, in the projection layer, so the reducer can
//! reason about which lines hold copyable values without depending on `src/ui`
//! (see ARCHITECTURE.md). Rendering stays in `src/ui/resources`.

use crate::app::state::resources::{
    CloudRunDetailTab, CloudRunRevisionTab, CloudSqlDetailTab, ComputeDetailTab, GkeDetailTab,
    ResourcesState, SecretDetailTab, StorageDetailTab, StorageView,
};
use nuvem_domain::gcp::resource::{
    CloudRunCondition, CloudRunRevision, CloudRunRevisionDetail, CloudRunServiceDetail,
    CloudRunTrafficTarget, CloudSqlInstanceDetail, CloudSqlUser, ComputeDisk,
    ComputeInstanceDetail, ComputeKeyValue, ComputeNetworkInterface, GkeAddon, GkeClusterDetail,
    GkeNodePool, ResourceServiceKind, SecretDetail, SecretVersion, StorageBucketDetail,
    StorageObjectDetail,
};
use nuvem_domain::gcp::text::{sanitize_clipboard_text, sanitize_rendered_text};

/// Placeholder rendered in place of a value the API did not report. A line
/// showing it holds nothing worth yanking.
const MISSING_VALUE: &str = "-";

/// Marker prefix drawn on the selected Cloud Run revision row. Mirrors
/// `ui::shared::table::ROW_SELECT_SYMBOL`, which the projection layer must not
/// import (`src/app` never depends on `src/ui` — see ARCHITECTURE.md); a
/// parity test in `src/ui/resources` pins the two together.
pub const REVISION_SELECT_MARKER: &str = "> ";
/// Blank filler of the same display width, so unselected revision rows stay
/// aligned with the selected one. The same parity test pins its width to the
/// renderer's symbol, so widening the symbol cannot leave the filler behind.
pub const REVISION_UNSELECTED_MARKER: &str = "  ";

/// The copy payload a detail line carries, if any.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CopyableField {
    pub label: String,
    pub value: String,
}

/// One rendered detail-pane line. `text` is what the pane displays; `copy` is
/// present only when the line holds a value worth yanking.
///
/// The two fields reach two different sinks and are sanitized by two different
/// policies, on purpose: `text` becomes one buffer line, so it loses escape
/// sequences *and* every control character, while `copy` goes to the system
/// clipboard and keeps its newlines and tabs. Folding the two policies into one
/// would either let an escape reach the terminal or flatten a multi-line
/// `startup-script` on copy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DetailLine {
    pub text: String,
    pub copy: Option<CopyableField>,
}

impl DetailLine {
    /// A `"{label}: {value}"` line whose value is the copy payload. Missing
    /// values (empty or the `-` placeholder) carry no payload.
    pub fn field(label: impl Into<String>, value: impl Into<String>) -> Self {
        let label = label.into();
        let value = value.into();
        Self {
            text: sanitize_rendered_text(&format!("{label}: {value}")),
            copy: copy_payload(label, value),
        }
    }

    /// A line with no copy payload: headings, blank separators, raw-document
    /// lines, and empty-state text.
    pub fn text(text: impl Into<String>) -> Self {
        Self {
            text: sanitize_rendered_text(&text.into()),
            copy: None,
        }
    }

    /// A line whose display form is not `"{label}: {value}"` but which still
    /// holds a copyable value. Same missing-value suppression as `field`.
    pub fn labeled(
        text: impl Into<String>,
        label: impl Into<String>,
        value: impl Into<String>,
    ) -> Self {
        Self {
            text: sanitize_rendered_text(&text.into()),
            copy: copy_payload(label.into(), value.into()),
        }
    }
}

/// Builds the copy payload for a label/value pair, suppressing it when the
/// value is absent.
///
/// Both halves are sanitized here, for the sink each one reaches, so no
/// formatter can forget: the label is rendered back to the operator on the
/// status line (`Copied field "..." to clipboard`) and is stripped of escape
/// sequences and every control character; the value goes to the system
/// clipboard and keeps its newlines and tabs, since multi-line metadata is
/// ordinary, but loses the escape sequences that a paste would otherwise feed
/// back into a terminal. Labels and values on GCE/GKE instances are settable by
/// anyone who can write instance labels or metadata, so neither is trusted.
fn copy_payload(label: String, value: String) -> Option<CopyableField> {
    let value = sanitize_clipboard_text(&value);
    if value.is_empty() || value == MISSING_VALUE {
        return None;
    }
    Some(CopyableField {
        label: sanitize_rendered_text(&label),
        value,
    })
}

/// The full body of one detail tab.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DetailBody {
    pub lines: Vec<DetailLine>,
    /// Set on JSON tabs only: the whole raw document, copyable as a unit.
    pub raw_json: Option<String>,
}

impl DetailBody {
    /// The raw JSON tab: every document line is displayed verbatim and none of
    /// them is individually copyable — the document is yanked as a unit.
    pub fn from_raw_json(raw: &str) -> Self {
        Self {
            lines: raw.lines().map(DetailLine::text).collect(),
            // Same clipboard policy as a single field's value: the document
            // keeps its line structure and loses any escape sequence.
            raw_json: Some(sanitize_clipboard_text(raw)),
        }
    }

    /// The text the pane renders, one entry per line.
    pub fn display_lines(&self) -> Vec<String> {
        self.lines.iter().map(|line| line.text.clone()).collect()
    }

    /// Every copyable line, as `(line index, payload)` in display order.
    pub fn fields(&self) -> Vec<(usize, &CopyableField)> {
        self.lines
            .iter()
            .enumerate()
            .filter_map(|(index, line)| line.copy.as_ref().map(|copy| (index, copy)))
            .collect()
    }
}

impl From<Vec<DetailLine>> for DetailBody {
    fn from(lines: Vec<DetailLine>) -> Self {
        Self {
            lines,
            raw_json: None,
        }
    }
}

/// Builds the body of whichever detail pane is currently on screen, or `None`
/// when no detail pane is open (or its payload has not arrived yet).
///
/// Mirrors what `ui::resources::render_detail` chooses to render, so the
/// reducer and the renderer can never disagree about which body is on screen.
pub fn project_detail_body(resources: &ResourcesState) -> Option<DetailBody> {
    if !resources.is_showing_detail() {
        return None;
    }

    // The Cloud Run revision drill-down layers on top of the service detail and
    // takes the pane over entirely. Only while Cloud Run is the active service:
    // a stale revision view must not bleed into another service's pane.
    if resources.active_service() == ResourceServiceKind::CloudRunServices
        && resources.is_showing_cloud_run_revision()
    {
        let detail = loaded(
            resources.cloud_run_revision_detail(),
            resources.cloud_run_revision_detail_error(),
        )?;
        return Some(cloud_run_revision_detail_body(
            detail,
            resources.active_cloud_run_revision_tab(),
        ));
    }

    match resources.active_service() {
        ResourceServiceKind::ComputeInstances => {
            let detail = loaded(resources.detail(), resources.detail_error())?;
            Some(compute_detail_body(detail, resources.active_detail_tab()))
        }
        ResourceServiceKind::GkeClusters => {
            let detail = loaded(resources.gke_detail(), resources.gke_detail_error())?;
            Some(gke_detail_body(detail, resources.active_gke_detail_tab()))
        }
        ResourceServiceKind::CloudSql => {
            let detail = loaded(
                resources.cloud_sql_detail(),
                resources.cloud_sql_detail_error(),
            )?;
            Some(cloud_sql_detail_body(
                detail,
                resources.active_cloud_sql_detail_tab(),
            ))
        }
        ResourceServiceKind::CloudRunServices => {
            let detail = loaded(
                resources.cloud_run_detail(),
                resources.cloud_run_detail_error(),
            )?;
            Some(cloud_run_detail_body(
                detail,
                resources.active_cloud_run_detail_tab(),
                resources.selected_cloud_run_revision_index(),
            ))
        }
        // No detail pane exists for Cloud Functions, persistent disks,
        // Pub/Sub, BigQuery or service accounts yet: `is_showing_detail` is
        // `false` for all of them, so this is the body of a pane that is
        // never open.
        ResourceServiceKind::CloudFunctions
        | ResourceServiceKind::PersistentDisks
        | ResourceServiceKind::PubSubTopics
        | ResourceServiceKind::VpcNetworks
        | ResourceServiceKind::VpcSubnets
        | ResourceServiceKind::FirewallRules
        | ResourceServiceKind::PubSubSubscriptions
        | ResourceServiceKind::BigQueryDatasets
        | ResourceServiceKind::IamServiceAccounts
        | ResourceServiceKind::IamPolicy => None,
        ResourceServiceKind::SecretManagerSecrets => {
            let detail = loaded(resources.secret_detail(), resources.secret_detail_error())?;
            Some(secret_detail_body(
                detail,
                resources.active_secret_detail_tab(),
                resources.secret_versions_error(),
            ))
        }
        ResourceServiceKind::CloudStorage => {
            let storage = resources.storage();
            if storage.view() == StorageView::ObjectDetail {
                let detail = loaded(storage.object_detail(), storage.object_detail_error())?;
                Some(storage_object_detail_body(
                    detail,
                    storage.object_detail_tab(),
                ))
            } else {
                let detail = loaded(storage.bucket_detail(), storage.bucket_detail_error())?;
                Some(storage_bucket_detail_body(
                    detail,
                    storage.bucket_detail_tab(),
                ))
            }
        }
    }
}

/// The payload the pane will draw, or `None` when it is drawing an error or a
/// loading state instead — neither carries copyable fields.
fn loaded<'a, T>(detail: Option<&'a T>, error: Option<&str>) -> Option<&'a T> {
    if error.is_some() {
        return None;
    }
    detail
}

pub fn compute_detail_body(detail: &ComputeInstanceDetail, tab: ComputeDetailTab) -> DetailBody {
    match tab {
        ComputeDetailTab::Summary => vec![
            DetailLine::field("Name", &detail.name),
            DetailLine::field("Status", format!("{:?}", detail.status)),
            DetailLine::field("Zone", &detail.zone),
            DetailLine::field("Machine type", &detail.machine_type),
            DetailLine::field(
                "Creation",
                detail.creation_timestamp.as_deref().unwrap_or("-"),
            ),
        ]
        .into(),
        ComputeDetailTab::Network => network_lines(&detail.network_interfaces).into(),
        ComputeDetailTab::Disks => disk_lines(&detail.disks).into(),
        ComputeDetailTab::Metadata => key_value_lines("Labels", &detail.labels)
            .into_iter()
            .chain(key_value_lines("Metadata", &detail.metadata))
            .collect::<Vec<_>>()
            .into(),
        ComputeDetailTab::Json => DetailBody::from_raw_json(&detail.raw_json),
    }
}

pub fn gke_detail_body(detail: &GkeClusterDetail, active_tab: GkeDetailTab) -> DetailBody {
    match active_tab {
        GkeDetailTab::Summary => vec![
            DetailLine::field("Name", &detail.name),
            DetailLine::field("Status", format!("{:?}", detail.status)),
            DetailLine::field("Location", &detail.location),
            DetailLine::field(
                "Master version",
                detail.current_master_version.as_deref().unwrap_or("-"),
            ),
            DetailLine::field(
                "Node version",
                detail.current_node_version.as_deref().unwrap_or("-"),
            ),
        ]
        .into(),
        GkeDetailTab::Networking => vec![
            DetailLine::field("Endpoint", detail.endpoint.as_deref().unwrap_or("-")),
            DetailLine::field("Private endpoint", yes_no(detail.private_cluster)),
            DetailLine::field("Network", detail.network.as_deref().unwrap_or("-")),
            DetailLine::field("Subnetwork", detail.subnetwork.as_deref().unwrap_or("-")),
        ]
        .into(),
        GkeDetailTab::NodePools => gke_node_pool_lines(&detail.node_pools).into(),
        GkeDetailTab::Addons => gke_addon_lines(&detail.addons)
            .into_iter()
            .chain(key_value_lines("Labels", &detail.labels))
            .collect::<Vec<_>>()
            .into(),
        GkeDetailTab::Json => DetailBody::from_raw_json(&detail.raw_json),
    }
}

pub fn cloud_sql_detail_body(
    detail: &CloudSqlInstanceDetail,
    tab: CloudSqlDetailTab,
) -> DetailBody {
    match tab {
        CloudSqlDetailTab::Summary => vec![
            DetailLine::field("Name", &detail.name),
            DetailLine::field("Status", format!("{:?}", detail.status)),
            DetailLine::field("Engine", &detail.engine),
            DetailLine::field("Database version", &detail.database_version),
            DetailLine::field("Region", &detail.region),
            DetailLine::field("Tier", &detail.tier),
            DetailLine::field("Availability", detail.availability.label()),
            DetailLine::field(
                "Activation policy",
                detail.activation_policy.as_deref().unwrap_or("-"),
            ),
            DetailLine::field(
                "Creation",
                detail.creation_timestamp.as_deref().unwrap_or("-"),
            ),
        ]
        .into(),
        CloudSqlDetailTab::Connectivity => vec![
            DetailLine::field(
                "Connection name",
                detail.connection_name.as_deref().unwrap_or("-"),
            ),
            DetailLine::field("Public IP", detail.public_ip.as_deref().unwrap_or("-")),
            DetailLine::field("Private IP", detail.private_ip.as_deref().unwrap_or("-")),
            DetailLine::field(
                "Private network",
                detail.private_network.as_deref().unwrap_or("-"),
            ),
            DetailLine::field("Require SSL", yes_no(detail.require_ssl)),
        ]
        .into(),
        CloudSqlDetailTab::Maintenance => cloud_sql_maintenance_lines(detail).into(),
        CloudSqlDetailTab::Backups => cloud_sql_backup_lines(detail).into(),
        CloudSqlDetailTab::Replicas => cloud_sql_replica_lines(detail).into(),
        CloudSqlDetailTab::Users => cloud_sql_user_lines(&detail.users).into(),
        CloudSqlDetailTab::Json => DetailBody::from_raw_json(&detail.raw_json),
    }
}

pub fn cloud_run_detail_body(
    detail: &CloudRunServiceDetail,
    tab: CloudRunDetailTab,
    selected_revision: usize,
) -> DetailBody {
    match tab {
        CloudRunDetailTab::Summary => vec![
            DetailLine::field("Name", &detail.name),
            DetailLine::field("Region", &detail.region),
            DetailLine::field("URL", detail.url.as_deref().unwrap_or("-")),
            DetailLine::field("Status", format!("{:?}", detail.status)),
            DetailLine::field(
                "Latest ready revision",
                detail.latest_ready_revision.as_deref().unwrap_or("-"),
            ),
            DetailLine::field(
                "Latest created revision",
                detail.latest_created_revision.as_deref().unwrap_or("-"),
            ),
        ]
        .into(),
        CloudRunDetailTab::Traffic => cloud_run_traffic_lines(&detail.traffic).into(),
        CloudRunDetailTab::Revisions => {
            cloud_run_revision_lines(&detail.revisions, selected_revision).into()
        }
        CloudRunDetailTab::Configuration => vec![
            DetailLine::field("Image", detail.image.as_deref().unwrap_or("-")),
            DetailLine::field("CPU", detail.cpu.as_deref().unwrap_or("-")),
            DetailLine::field("Memory", detail.memory.as_deref().unwrap_or("-")),
            DetailLine::field("Concurrency", opt_number(detail.concurrency)),
            DetailLine::field(
                "Service account",
                detail.service_account.as_deref().unwrap_or("-"),
            ),
            DetailLine::field("Ingress", detail.ingress.as_deref().unwrap_or("-")),
            DetailLine::field("Min instances", opt_number(detail.min_instances)),
            DetailLine::field("Max instances", opt_number(detail.max_instances)),
        ]
        .into(),
        CloudRunDetailTab::Conditions => cloud_run_condition_lines(&detail.conditions).into(),
        CloudRunDetailTab::Json => DetailBody::from_raw_json(&detail.raw_json),
    }
}

/// The secret detail pane.
///
/// Every line is metadata. The Versions tab lists a version's id, state and
/// creation time — and nothing else, because nothing else is ever fetched. The
/// JSON tab renders the describe response, which has no payload field in it to
/// begin with. See `tests/secrets_no_payload_access_test.rs`.
/// `versions_error` is why the separate `secrets versions list` call failed,
/// when it did. It is threaded all the way here because an empty version list
/// is otherwise indistinguishable from a version list nobody was allowed to
/// read, and the two mean opposite things to an operator: listing versions
/// needs an IAM permission that listing secrets does not, so a permission
/// failure here is ordinary rather than exotic.
pub fn secret_detail_body(
    detail: &SecretDetail,
    tab: SecretDetailTab,
    versions_error: Option<&str>,
) -> DetailBody {
    match tab {
        SecretDetailTab::Summary => vec![
            DetailLine::field("Name", &detail.name),
            DetailLine::field("Created", detail.create_time.as_deref().unwrap_or("-")),
            DetailLine::field("Replication", &detail.replication),
            DetailLine::field("Rotation", detail.rotation.as_deref().unwrap_or("none")),
            DetailLine::field("Expires", detail.expire_time.as_deref().unwrap_or("never")),
            DetailLine::field(
                "Versions",
                match versions_error {
                    Some(_) => "unavailable".to_string(),
                    None => detail.versions.len().to_string(),
                },
            ),
        ]
        .into(),
        SecretDetailTab::Versions => secret_version_lines(&detail.versions, versions_error).into(),
        SecretDetailTab::Labels => {
            secret_key_value_lines(&detail.labels, &detail.annotations).into()
        }
        SecretDetailTab::Json => DetailBody::from_raw_json(&detail.raw_json),
    }
}

/// One line per version: which version, what state, when it was created.
///
/// The version count the list row deliberately does not carry is simply the
/// length of this list, which is why it is also on the Summary tab.
///
/// A failed fetch reports the failure instead of "No versions.": the empty
/// list would otherwise assert, as fact, that a secret has no versions when
/// all that is known is that they could not be listed.
fn secret_version_lines(versions: &[SecretVersion], error: Option<&str>) -> Vec<DetailLine> {
    if let Some(error) = error {
        return vec![
            DetailLine::text("Versions could not be listed."),
            DetailLine::text(format!("  {error}")),
        ];
    }

    if versions.is_empty() {
        return vec![DetailLine::text("No versions.")];
    }

    versions
        .iter()
        .map(|version| {
            let line = format!(
                "{}  {}  {}",
                version.name,
                version.state.label(),
                version.create_time.as_deref().unwrap_or("-")
            );
            DetailLine::labeled(line, "Version", &version.name)
        })
        .collect()
}

fn secret_key_value_lines(
    labels: &[ComputeKeyValue],
    annotations: &[ComputeKeyValue],
) -> Vec<DetailLine> {
    let mut lines = Vec::new();
    lines.push(DetailLine::text("Labels:"));
    if labels.is_empty() {
        lines.push(DetailLine::text("  none"));
    } else {
        lines.extend(
            labels
                .iter()
                .map(|label| DetailLine::field(&label.key, &label.value)),
        );
    }
    lines.push(DetailLine::text(String::new()));
    lines.push(DetailLine::text("Annotations:"));
    if annotations.is_empty() {
        lines.push(DetailLine::text("  none"));
    } else {
        lines.extend(
            annotations
                .iter()
                .map(|entry| DetailLine::field(&entry.key, &entry.value)),
        );
    }
    lines
}

pub fn cloud_run_revision_detail_body(
    detail: &CloudRunRevisionDetail,
    tab: CloudRunRevisionTab,
) -> DetailBody {
    match tab {
        CloudRunRevisionTab::Summary => vec![
            DetailLine::field("Name", &detail.name),
            DetailLine::field("Service", detail.service.as_deref().unwrap_or("-")),
            DetailLine::field("Region", &detail.region),
            DetailLine::field("Ready", format!("{:?}", detail.ready)),
            DetailLine::field("Image", detail.image.as_deref().unwrap_or("-")),
            DetailLine::field("CPU", detail.cpu.as_deref().unwrap_or("-")),
            DetailLine::field("Memory", detail.memory.as_deref().unwrap_or("-")),
            DetailLine::field("Concurrency", opt_number(detail.concurrency)),
            DetailLine::field("Min instances", opt_number(detail.min_instances)),
            DetailLine::field("Max instances", opt_number(detail.max_instances)),
            DetailLine::field(
                "Service account",
                detail.service_account.as_deref().unwrap_or("-"),
            ),
            DetailLine::field("Created", detail.created.as_deref().unwrap_or("-")),
        ]
        .into(),
        CloudRunRevisionTab::Conditions => cloud_run_condition_lines(&detail.conditions).into(),
        CloudRunRevisionTab::Json => DetailBody::from_raw_json(&detail.raw_json),
    }
}

pub fn storage_bucket_detail_body(
    detail: &StorageBucketDetail,
    tab: StorageDetailTab,
) -> DetailBody {
    match tab {
        StorageDetailTab::Summary => vec![
            DetailLine::field("Name", &detail.name),
            DetailLine::field("Location", &detail.location),
            DetailLine::field(
                "Location type",
                detail.location_type.as_deref().unwrap_or("-"),
            ),
            DetailLine::field("Storage class", &detail.storage_class),
            DetailLine::field("Versioning", yes_no(detail.versioning_enabled)),
            DetailLine::field(
                "Uniform bucket-level access",
                yes_no(detail.uniform_bucket_level_access),
            ),
            DetailLine::field(
                "Public access prevention",
                detail.public_access_prevention.as_deref().unwrap_or("-"),
            ),
            DetailLine::field("Requester pays", yes_no(detail.requester_pays)),
            DetailLine::field(
                "Retention period",
                detail.retention_period.as_deref().unwrap_or("-"),
            ),
            // Only presence is ever surfaced; the KMS key name is never shown.
            DetailLine::field("Default KMS key", yes_no(detail.default_kms_key_present)),
            DetailLine::field("Created", detail.time_created.as_deref().unwrap_or("-")),
            DetailLine::field("Updated", detail.updated.as_deref().unwrap_or("-")),
        ]
        .into(),
        StorageDetailTab::Metadata => key_value_lines("Labels", &detail.labels).into(),
        StorageDetailTab::Json => DetailBody::from_raw_json(&detail.raw_json),
    }
}

pub fn storage_object_detail_body(
    detail: &StorageObjectDetail,
    tab: StorageDetailTab,
) -> DetailBody {
    match tab {
        StorageDetailTab::Summary => vec![
            DetailLine::field("Key", &detail.key),
            DetailLine::field(
                "Size",
                detail
                    .size_bytes
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| "-".to_string()),
            ),
            DetailLine::field(
                "Storage class",
                detail.storage_class.as_deref().unwrap_or("-"),
            ),
            DetailLine::field("Generation", detail.generation.as_deref().unwrap_or("-")),
            DetailLine::field("Updated", detail.updated.as_deref().unwrap_or("-")),
            DetailLine::field(
                "Content type",
                detail.content_type.as_deref().unwrap_or("-"),
            ),
            // Only presence is ever surfaced; the KMS key name is never shown.
            DetailLine::field("KMS encrypted", yes_no(detail.kms_encrypted)),
            DetailLine::field("Temporary hold", yes_no(detail.temporary_hold)),
            DetailLine::field("Event-based hold", yes_no(detail.event_based_hold)),
            DetailLine::field(
                "Retention expiration",
                detail.retention_expiration.as_deref().unwrap_or("-"),
            ),
        ]
        .into(),
        StorageDetailTab::Metadata => vec![
            DetailLine::field("MD5", detail.md5_hash.as_deref().unwrap_or("-")),
            DetailLine::field("CRC32C", detail.crc32c.as_deref().unwrap_or("-")),
        ]
        .into(),
        StorageDetailTab::Json => DetailBody::from_raw_json(&detail.raw_json),
    }
}

fn cloud_run_traffic_lines(targets: &[CloudRunTrafficTarget]) -> Vec<DetailLine> {
    if targets.is_empty() {
        return vec![DetailLine::text("No traffic targets")];
    }

    targets
        .iter()
        .map(|target| {
            let revision = if target.latest {
                "LATEST".to_string()
            } else {
                target
                    .revision_name
                    .clone()
                    .unwrap_or_else(|| "-".to_string())
            };
            let reported_percent = target.percent.map(|value| format!("{value}%"));
            let percent = reported_percent.clone().unwrap_or_else(|| "-%".to_string());
            let mut line = format!("{revision} {percent}");
            if let Some(tag) = target.tag.as_deref() {
                line.push_str(&format!(" [{tag}]"));
            }
            // The revision names the split; the percent is the value worth
            // yanking, without the optional tag suffix. A row whose percent
            // was never reported displays `-%`, and one with no revision name
            // is labelled `-`; neither placeholder is worth putting on the
            // clipboard, so such rows carry no payload at all.
            match reported_percent {
                Some(percent) if revision != MISSING_VALUE => {
                    DetailLine::labeled(line, revision, percent)
                }
                _ => DetailLine::text(line),
            }
        })
        .collect()
}

fn cloud_run_revision_lines(revisions: &[CloudRunRevision], selected: usize) -> Vec<DetailLine> {
    if revisions.is_empty() {
        return vec![DetailLine::text("No revisions.")];
    }

    revisions
        .iter()
        .enumerate()
        .map(|(index, revision)| {
            let percent = revision
                .active_percent
                .map(|value| format!("{value}%"))
                .unwrap_or_else(|| "-%".to_string());
            let marker = if index == selected {
                REVISION_SELECT_MARKER
            } else {
                REVISION_UNSELECTED_MARKER
            };
            let line = format!(
                "{marker}{}  {:?}  {}",
                revision.name, revision.ready, percent
            );
            DetailLine::labeled(line, "Revision", &revision.name)
        })
        .collect()
}

fn cloud_run_condition_lines(conditions: &[CloudRunCondition]) -> Vec<DetailLine> {
    if conditions.is_empty() {
        return vec![DetailLine::text("No conditions.")];
    }

    conditions
        .iter()
        .map(|condition| {
            // The status (plus its optional reason and message) is the value;
            // the condition type is its label.
            let mut value = condition.status.clone();
            if let Some(reason) = condition.reason.as_deref() {
                value.push_str(&format!(" ({reason})"));
            }
            if let Some(message) = condition.message.as_deref() {
                value.push_str(&format!(" {message}"));
            }
            DetailLine::labeled(
                format!("{}: {value}", condition.type_),
                &condition.type_,
                value,
            )
        })
        .collect()
}

fn cloud_sql_maintenance_lines(detail: &CloudSqlInstanceDetail) -> Vec<DetailLine> {
    let mut lines = Vec::new();
    match detail.maintenance_window.as_ref() {
        Some(window) => {
            lines.push(DetailLine::field(
                "Day",
                window
                    .day
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| "-".to_string()),
            ));
            lines.push(DetailLine::field(
                "Hour",
                window
                    .hour
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| "-".to_string()),
            ));
            lines.push(DetailLine::field(
                "Update track",
                window.update_track.as_deref().unwrap_or("-"),
            ));
        }
        None => lines.push(DetailLine::text("No maintenance window configured.")),
    }
    lines.push(DetailLine::field(
        "Maintenance version",
        detail.maintenance_version.as_deref().unwrap_or("-"),
    ));
    lines
}

fn cloud_sql_backup_lines(detail: &CloudSqlInstanceDetail) -> Vec<DetailLine> {
    let backup = &detail.backup;
    vec![
        DetailLine::field("Enabled", yes_no(backup.enabled)),
        DetailLine::field("Start time", backup.start_time.as_deref().unwrap_or("-")),
        DetailLine::field("Location", backup.location.as_deref().unwrap_or("-")),
        DetailLine::field(
            "Point-in-time recovery",
            yes_no(backup.point_in_time_recovery),
        ),
        DetailLine::field(
            "Transaction log retention (days)",
            backup
                .transaction_log_retention_days
                .map(|value| value.to_string())
                .unwrap_or_else(|| "-".to_string()),
        ),
    ]
}

fn cloud_sql_replica_lines(detail: &CloudSqlInstanceDetail) -> Vec<DetailLine> {
    let mut lines = vec![DetailLine::field(
        "Master instance",
        detail.master_instance.as_deref().unwrap_or("-"),
    )];
    if detail.replicas.is_empty() {
        lines.push(DetailLine::text("No read replicas."));
    } else {
        lines.push(DetailLine::text("Replicas:"));
        lines.extend(
            detail
                .replicas
                .iter()
                .map(|replica| DetailLine::labeled(format!("- {replica}"), "Replica", replica)),
        );
    }
    lines
}

fn cloud_sql_user_lines(users: &[CloudSqlUser]) -> Vec<DetailLine> {
    if users.is_empty() {
        return vec![DetailLine::text("No users loaded.")];
    }

    users
        .iter()
        .flat_map(|user| {
            vec![
                DetailLine::field("Name", &user.name),
                DetailLine::field("Host", user.host.as_deref().unwrap_or("-")),
                DetailLine::field("Type", user.kind.as_deref().unwrap_or("-")),
                DetailLine::text(String::new()),
            ]
        })
        .collect()
}

fn network_lines(interfaces: &[ComputeNetworkInterface]) -> Vec<DetailLine> {
    if interfaces.is_empty() {
        return vec![DetailLine::text("No network interfaces.")];
    }

    interfaces
        .iter()
        .enumerate()
        .flat_map(|(index, interface)| {
            let mut lines = Vec::new();
            if index > 0 {
                lines.push(DetailLine::text(String::new()));
            }
            lines.push(DetailLine::field(
                "Name",
                interface.name.as_deref().unwrap_or("-"),
            ));
            lines.push(DetailLine::field(
                "Network",
                interface.network.as_deref().unwrap_or("-"),
            ));
            lines.push(DetailLine::field(
                "Subnetwork",
                interface.subnetwork.as_deref().unwrap_or("-"),
            ));
            lines.push(DetailLine::field(
                "Internal IP",
                interface.internal_ip.as_deref().unwrap_or("-"),
            ));
            lines.push(DetailLine::field(
                "External IP",
                interface.external_ip.as_deref().unwrap_or("-"),
            ));
            lines
        })
        .collect()
}

fn disk_lines(disks: &[ComputeDisk]) -> Vec<DetailLine> {
    if disks.is_empty() {
        return vec![DetailLine::text("No disks.")];
    }

    disks
        .iter()
        .enumerate()
        .flat_map(|(index, disk)| {
            let mut lines = Vec::new();
            if index > 0 {
                lines.push(DetailLine::text(String::new()));
            }
            lines.push(DetailLine::field(
                "Name",
                disk.name.as_deref().unwrap_or("-"),
            ));
            lines.push(DetailLine::field(
                "Source",
                disk.source.as_deref().unwrap_or("-"),
            ));
            lines.push(DetailLine::field(
                "Type",
                disk.disk_type.as_deref().unwrap_or("-"),
            ));
            lines.push(DetailLine::field(
                "Size (GB)",
                disk.size_gb.as_deref().unwrap_or("-"),
            ));
            lines.push(DetailLine::field("Boot", yes_no(disk.boot)));
            lines.push(DetailLine::field("Auto-delete", yes_no(disk.auto_delete)));
            lines
        })
        .collect()
}

fn gke_node_pool_lines(node_pools: &[GkeNodePool]) -> Vec<DetailLine> {
    if node_pools.is_empty() {
        return vec![DetailLine::text("No node pools.")];
    }

    node_pools
        .iter()
        .enumerate()
        .flat_map(|(index, node_pool)| {
            let mut lines = Vec::new();
            if index > 0 {
                lines.push(DetailLine::text(String::new()));
            }
            lines.push(DetailLine::field("Name", &node_pool.name));
            lines.push(DetailLine::field(
                "Status",
                format!("{:?}", node_pool.status),
            ));
            lines.push(DetailLine::field(
                "Version",
                node_pool.version.as_deref().unwrap_or("-"),
            ));
            lines.push(DetailLine::field(
                "Machine type",
                node_pool.machine_type.as_deref().unwrap_or("-"),
            ));
            lines.push(DetailLine::field(
                "Autoscaling",
                node_pool.autoscaling.as_deref().unwrap_or("-"),
            ));
            lines.push(DetailLine::field(
                "Node count",
                node_pool
                    .node_count
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| "-".to_string()),
            ));
            lines
        })
        .collect()
}

fn gke_addon_lines(addons: &[GkeAddon]) -> Vec<DetailLine> {
    if addons.is_empty() {
        return vec![DetailLine::text("No addons.")];
    }

    addons
        .iter()
        .map(|addon| DetailLine::field(&addon.name, yes_no(addon.enabled)))
        .collect()
}

fn key_value_lines(title: &str, values: &[ComputeKeyValue]) -> Vec<DetailLine> {
    let mut lines = vec![DetailLine::text(format!("{title}:"))];
    if values.is_empty() {
        lines.push(DetailLine::text("-"));
        return lines;
    }

    lines.extend(
        values
            .iter()
            .map(|entry| DetailLine::field(&entry.key, &entry.value)),
    );
    lines
}

fn yes_no(value: bool) -> &'static str {
    if value { "yes" } else { "no" }
}

fn opt_number(value: Option<u32>) -> String {
    value
        .map(|value| value.to_string())
        .unwrap_or_else(|| "-".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use nuvem_domain::gcp::resource::{
        CloudRunServiceStatus, CloudSqlAvailability, CloudSqlBackupConfig,
        CloudSqlMaintenanceWindow, CloudSqlStatus, GkeClusterStatus, ResourceStatus,
    };

    /// The words themselves are the contract: every boolean field a detail
    /// body renders goes through this helper, so an operator scanning a pane
    /// reads `yes`/`no`, not `true`/`false` or a blank. The detail-walk tests
    /// above assert field presence, not these exact strings, which is why the
    /// helper needs its own pin.
    #[test]
    fn booleans_render_as_yes_and_no() {
        assert_eq!(yes_no(true), "yes");
        assert_eq!(yes_no(false), "no");
    }

    fn compute_instance_detail() -> ComputeInstanceDetail {
        ComputeInstanceDetail {
            name: "api-dev-1".to_string(),
            zone: "europe-west1-b".to_string(),
            status: ResourceStatus::Running,
            machine_type: "e2-medium".to_string(),
            creation_timestamp: Some("2024-01-02T03:04:05Z".to_string()),
            network_interfaces: vec![ComputeNetworkInterface {
                name: Some("nic0".to_string()),
                network: Some("default".to_string()),
                subnetwork: Some("default-euw1".to_string()),
                subnetwork_region: Some("europe-west1".to_string()),
                internal_ip: Some("10.0.0.2".to_string()),
                external_ip: Some("34.1.2.3".to_string()),
            }],
            tags: Vec::new(),
            service_accounts: Vec::new(),
            disks: vec![ComputeDisk {
                name: Some("boot".to_string()),
                source: Some("projects/dev/disks/boot".to_string()),
                boot: true,
                auto_delete: true,
                disk_type: Some("PERSISTENT".to_string()),
                size_gb: Some("10".to_string()),
            }],
            labels: vec![ComputeKeyValue {
                key: "env".to_string(),
                value: "prod".to_string(),
            }],
            metadata: Vec::new(),
            raw_json: "{\n  \"name\": \"api-dev-1\"\n}".to_string(),
        }
    }

    fn gke_cluster_detail() -> GkeClusterDetail {
        GkeClusterDetail {
            name: "cluster-a".to_string(),
            location: "europe-west1".to_string(),
            status: GkeClusterStatus::Running,
            current_master_version: Some("1.29.1".to_string()),
            current_node_version: Some("1.29.1".to_string()),
            endpoint: Some("10.0.0.1".to_string()),
            private_cluster: true,
            network: Some("default".to_string()),
            subnetwork: Some("default-euw1".to_string()),
            node_pools: vec![GkeNodePool {
                name: "pool-1".to_string(),
                status: GkeClusterStatus::Running,
                version: Some("1.29.1".to_string()),
                machine_type: Some("e2-standard-4".to_string()),
                autoscaling: Some("1-3".to_string()),
                node_count: Some(2),
            }],
            addons: vec![GkeAddon {
                name: "HttpLoadBalancing".to_string(),
                enabled: true,
            }],
            labels: vec![ComputeKeyValue {
                key: "env".to_string(),
                value: "prod".to_string(),
            }],
            raw_json: "{\n  \"name\": \"cluster-a\"\n}".to_string(),
        }
    }

    fn cloud_sql_instance_detail() -> CloudSqlInstanceDetail {
        CloudSqlInstanceDetail {
            name: "db".to_string(),
            database_version: "POSTGRES_15".to_string(),
            engine: "POSTGRES".to_string(),
            region: "europe-west1".to_string(),
            tier: "db-custom-2-7680".to_string(),
            availability: CloudSqlAvailability::Regional,
            status: CloudSqlStatus::Runnable,
            activation_policy: Some("ALWAYS".to_string()),
            creation_timestamp: Some("2024-01-02T03:04:05Z".to_string()),
            connection_name: Some("dev-project:europe-west1:db".to_string()),
            public_ip: Some("34.9.9.9".to_string()),
            private_ip: None,
            require_ssl: true,
            private_network: None,
            maintenance_window: Some(CloudSqlMaintenanceWindow {
                day: Some(3),
                hour: Some(4),
                update_track: Some("stable".to_string()),
            }),
            maintenance_version: Some("POSTGRES_15_R2".to_string()),
            backup: CloudSqlBackupConfig {
                enabled: true,
                start_time: Some("02:00".to_string()),
                location: Some("eu".to_string()),
                point_in_time_recovery: true,
                transaction_log_retention_days: Some(7),
            },
            replicas: vec!["db-replica".to_string()],
            master_instance: None,
            users: vec![CloudSqlUser {
                name: "postgres".to_string(),
                host: None,
                kind: Some("BUILT_IN".to_string()),
            }],
            labels: Vec::new(),
            raw_json: "{\n  \"name\": \"db\"\n}".to_string(),
        }
    }

    #[test]
    fn field_renders_label_colon_value() {
        let line = DetailLine::field("Region", "europe-west1");
        assert_eq!(line.text, "Region: europe-west1");
        assert_eq!(
            line.copy,
            Some(CopyableField {
                label: "Region".to_string(),
                value: "europe-west1".to_string(),
            })
        );
    }

    #[test]
    fn field_suppresses_copy_for_empty_and_placeholder_values() {
        assert!(DetailLine::field("URL", "").copy.is_none());
        assert!(DetailLine::field("URL", "-").copy.is_none());
        assert!(DetailLine::field("URL", "https://x").copy.is_some());
    }

    #[test]
    fn text_lines_never_carry_copy() {
        let line = DetailLine::text("Labels:");
        assert_eq!(line.text, "Labels:");
        assert!(line.copy.is_none());
    }

    #[test]
    fn labeled_keeps_display_text_and_copy_payload() {
        let line = DetailLine::labeled("LATEST 20% [tag]", "LATEST", "20%");
        assert_eq!(line.text, "LATEST 20% [tag]");
        assert_eq!(
            line.copy,
            Some(CopyableField {
                label: "LATEST".to_string(),
                value: "20%".to_string(),
            })
        );
        assert!(
            DetailLine::labeled("- none -", "Revision", "-")
                .copy
                .is_none()
        );
    }

    #[test]
    fn fields_returns_display_order_indices_and_skips_non_copyable() {
        let body = DetailBody {
            lines: vec![
                DetailLine::text("Labels:"),
                DetailLine::field("env", "prod"),
                DetailLine::text(String::new()),
                DetailLine::field("Missing", "-"),
                DetailLine::field("Zone", "europe-west1-b"),
                DetailLine::text("No disks."),
            ],
            raw_json: None,
        };

        let fields = body.fields();
        assert_eq!(fields.len(), 2);
        assert_eq!(fields[0].0, 1);
        assert_eq!(fields[0].1.label, "env");
        assert_eq!(fields[1].0, 4);
        assert_eq!(fields[1].1.value, "europe-west1-b");
        assert_eq!(
            body.display_lines(),
            vec![
                "Labels:".to_string(),
                "env: prod".to_string(),
                String::new(),
                "Missing: -".to_string(),
                "Zone: europe-west1-b".to_string(),
                "No disks.".to_string(),
            ]
        );
    }

    fn cloud_run_service_detail() -> CloudRunServiceDetail {
        CloudRunServiceDetail {
            name: "api".to_string(),
            region: "europe-west1".to_string(),
            url: Some("https://api-abc.a.run.app".to_string()),
            status: CloudRunServiceStatus::Ready,
            latest_ready_revision: Some("api-00002".to_string()),
            latest_created_revision: Some("api-00003".to_string()),
            traffic: vec![
                CloudRunTrafficTarget {
                    revision_name: Some("api-00002".to_string()),
                    tag: Some("stable".to_string()),
                    percent: Some(80),
                    latest: false,
                    url: None,
                },
                CloudRunTrafficTarget {
                    revision_name: None,
                    tag: None,
                    percent: Some(20),
                    latest: true,
                    url: None,
                },
            ],
            revisions: vec![CloudRunRevision {
                name: "api-00002".to_string(),
                ready: CloudRunServiceStatus::Ready,
                created: None,
                image: Some("gcr.io/x/api:1".to_string()),
                active_percent: Some(80),
            }],
            conditions: vec![CloudRunCondition {
                type_: "Ready".to_string(),
                status: "True".to_string(),
                reason: Some("Deployed".to_string()),
                message: Some("Service is ready".to_string()),
                last_transition_time: None,
            }],
            image: Some("gcr.io/x/api:1".to_string()),
            cpu: Some("1000m".to_string()),
            memory: Some("512Mi".to_string()),
            concurrency: Some(80),
            service_account: Some("svc@project.iam".to_string()),
            ingress: Some("all".to_string()),
            min_instances: Some(0),
            max_instances: Some(10),
            labels: Vec::new(),
            raw_json: "{\n  \"name\": \"api\"\n}".to_string(),
        }
    }

    fn cloud_run_revision_detail() -> CloudRunRevisionDetail {
        CloudRunRevisionDetail {
            name: "api-00002".to_string(),
            service: Some("api".to_string()),
            region: "europe-west1".to_string(),
            ready: CloudRunServiceStatus::Ready,
            conditions: vec![CloudRunCondition {
                type_: "Active".to_string(),
                status: "True".to_string(),
                reason: None,
                message: None,
                last_transition_time: None,
            }],
            image: Some("gcr.io/x/api:1".to_string()),
            cpu: Some("1000m".to_string()),
            memory: Some("512Mi".to_string()),
            concurrency: Some(80),
            min_instances: Some(0),
            max_instances: Some(10),
            service_account: Some("svc@project.iam".to_string()),
            created: Some("2026-07-19T00:00:00Z".to_string()),
            raw_json: "{\n  \"name\": \"api-00002\"\n}".to_string(),
        }
    }

    fn hostile_bucket_detail() -> StorageBucketDetail {
        StorageBucketDetail {
            name: "alpha-bucket".to_string(),
            location: "US".to_string(),
            location_type: Some("multi-region".to_string()),
            storage_class: "STANDARD".to_string(),
            versioning_enabled: true,
            uniform_bucket_level_access: true,
            public_access_prevention: Some("enforced".to_string()),
            requester_pays: true,
            retention_period: Some("2592000".to_string()),
            default_kms_key_present: true,
            labels: vec![
                ComputeKeyValue {
                    key: "team\r\n\u{1b}[2J".to_string(),
                    value: "platform\r\n\u{1b}[2J".to_string(),
                },
                ComputeKeyValue {
                    key: "env".to_string(),
                    value: "prod".to_string(),
                },
            ],
            time_created: Some("2024-01-02T03:04:05Z".to_string()),
            updated: Some("2024-02-03T04:05:06Z".to_string()),
            raw_json: "{\n  \"name\": \"alpha-bucket\"\n}".to_string(),
        }
    }

    fn hostile_object_detail() -> StorageObjectDetail {
        StorageObjectDetail {
            key: "reports/leaf.txt".to_string(),
            size_bytes: Some(1024),
            storage_class: Some("STANDARD".to_string()),
            generation: Some("1".to_string()),
            updated: Some("2024-03-01T10:00:00Z".to_string()),
            content_type: Some("text/plain\r\n\u{1b}[2JX-Evil: 1".to_string()),
            kms_encrypted: true,
            temporary_hold: false,
            event_based_hold: false,
            retention_expiration: None,
            md5_hash: Some("XUFAKrxLKna5cZ2REBfFkg==".to_string()),
            crc32c: Some("AAAAAA==".to_string()),
            raw_json: "{\n  \"name\": \"reports/leaf.txt\"\n}".to_string(),
        }
    }

    #[test]
    fn cloud_run_detail_lines_summary_tab_shows_labelled_fields() {
        let detail = cloud_run_service_detail();
        let lines = cloud_run_detail_body(&detail, CloudRunDetailTab::Summary, 0).display_lines();
        assert!(lines.iter().any(|line| line == "Name: api"));
        assert!(lines.iter().any(|line| line == "Region: europe-west1"));
        assert!(
            lines
                .iter()
                .any(|line| line == "URL: https://api-abc.a.run.app")
        );
        assert!(lines.iter().any(|line| line.starts_with("Status: ")));
        assert!(
            lines
                .iter()
                .any(|line| line.contains("Latest ready revision: api-00002"))
        );
        assert!(
            lines
                .iter()
                .any(|line| line.contains("Latest created revision: api-00003"))
        );
    }

    #[test]
    fn cloud_run_detail_lines_summary_url_falls_back_to_dash() {
        let mut detail = cloud_run_service_detail();
        detail.url = None;
        let lines = cloud_run_detail_body(&detail, CloudRunDetailTab::Summary, 0).display_lines();
        assert!(lines.iter().any(|line| line == "URL: -"));
    }

    #[test]
    fn cloud_run_detail_lines_traffic_tab_lists_targets_with_percent() {
        let detail = cloud_run_service_detail();
        let lines = cloud_run_detail_body(&detail, CloudRunDetailTab::Traffic, 0).display_lines();
        assert!(
            lines
                .iter()
                .any(|line| line.contains("api-00002") && line.contains("80%"))
        );
        assert!(
            lines
                .iter()
                .any(|line| line.contains("LATEST") && line.contains("20%"))
        );
    }

    #[test]
    fn cloud_run_detail_lines_traffic_tab_suppresses_copy_for_placeholder_targets() {
        let mut detail = cloud_run_service_detail();
        detail.traffic = vec![
            // No percent reported: the row displays the `-%` placeholder.
            CloudRunTrafficTarget {
                revision_name: Some("api-00002".to_string()),
                tag: None,
                percent: None,
                latest: false,
                url: None,
            },
            // Neither a revision name nor a percent: both halves are placeholders.
            CloudRunTrafficTarget {
                revision_name: None,
                tag: None,
                percent: None,
                latest: false,
                url: None,
            },
            // No revision name, but a real percent: the label is a placeholder.
            CloudRunTrafficTarget {
                revision_name: None,
                tag: None,
                percent: Some(20),
                latest: false,
                url: None,
            },
        ];
        let body = cloud_run_detail_body(&detail, CloudRunDetailTab::Traffic, 0);
        assert_eq!(
            body.display_lines(),
            vec![
                "api-00002 -%".to_string(),
                "- -%".to_string(),
                "- 20%".to_string(),
            ],
            "display must stay byte-identical when the copy payload is suppressed"
        );
        assert!(
            body.fields().is_empty(),
            "placeholder traffic rows must not be copyable, got: {:?}",
            body.fields()
        );
    }

    #[test]
    fn cloud_run_detail_lines_traffic_tab_reports_empty() {
        let mut detail = cloud_run_service_detail();
        detail.traffic.clear();
        let lines = cloud_run_detail_body(&detail, CloudRunDetailTab::Traffic, 0).display_lines();
        assert_eq!(lines, vec!["No traffic targets".to_string()]);
    }

    #[test]
    fn cloud_run_detail_lines_revisions_tab_lists_revision_names() {
        let detail = cloud_run_service_detail();
        let lines = cloud_run_detail_body(&detail, CloudRunDetailTab::Revisions, 0).display_lines();
        assert!(
            lines
                .iter()
                .any(|line| line.contains("api-00002") && line.contains("80%"))
        );
    }

    #[test]
    fn cloud_run_detail_lines_revisions_tab_marks_selected_row_only() {
        let mut detail = cloud_run_service_detail();
        detail.revisions = vec![
            CloudRunRevision {
                name: "api-00002".to_string(),
                ready: CloudRunServiceStatus::Ready,
                created: None,
                image: None,
                active_percent: Some(80),
            },
            CloudRunRevision {
                name: "api-00003".to_string(),
                ready: CloudRunServiceStatus::Ready,
                created: None,
                image: None,
                active_percent: Some(20),
            },
        ];

        let lines = cloud_run_detail_body(&detail, CloudRunDetailTab::Revisions, 1).display_lines();
        let first = lines
            .iter()
            .find(|line| line.contains("api-00002"))
            .expect("first revision row");
        let second = lines
            .iter()
            .find(|line| line.contains("api-00003"))
            .expect("second revision row");
        assert!(!first.starts_with(REVISION_SELECT_MARKER));
        assert!(second.starts_with(REVISION_SELECT_MARKER));
    }

    #[test]
    fn cloud_run_detail_lines_selection_marker_only_on_revisions_tab() {
        let detail = cloud_run_service_detail();
        let lines = cloud_run_detail_body(&detail, CloudRunDetailTab::Summary, 0).display_lines();
        assert!(
            !lines
                .iter()
                .any(|line| line.starts_with(REVISION_SELECT_MARKER))
        );
    }

    #[test]
    fn cloud_run_detail_lines_configuration_tab_shows_image_and_limits() {
        let detail = cloud_run_service_detail();
        let lines =
            cloud_run_detail_body(&detail, CloudRunDetailTab::Configuration, 0).display_lines();
        assert!(lines.iter().any(|line| line == "Image: gcr.io/x/api:1"));
        assert!(lines.iter().any(|line| line == "CPU: 1000m"));
        assert!(lines.iter().any(|line| line == "Memory: 512Mi"));
        assert!(lines.iter().any(|line| line == "Concurrency: 80"));
        assert!(
            lines
                .iter()
                .any(|line| line == "Service account: svc@project.iam")
        );
        assert!(lines.iter().any(|line| line == "Ingress: all"));
        assert!(lines.iter().any(|line| line == "Min instances: 0"));
        assert!(lines.iter().any(|line| line == "Max instances: 10"));
    }

    #[test]
    fn cloud_run_detail_lines_configuration_tab_uses_dash_for_missing() {
        let mut detail = cloud_run_service_detail();
        detail.image = None;
        detail.max_instances = None;
        let lines =
            cloud_run_detail_body(&detail, CloudRunDetailTab::Configuration, 0).display_lines();
        assert!(lines.iter().any(|line| line == "Image: -"));
        assert!(lines.iter().any(|line| line == "Max instances: -"));
    }

    #[test]
    fn cloud_run_detail_lines_conditions_tab_formats_type_status_reason() {
        let detail = cloud_run_service_detail();
        let lines =
            cloud_run_detail_body(&detail, CloudRunDetailTab::Conditions, 0).display_lines();
        assert!(lines.iter().any(|line| line.contains("Ready")
            && line.contains("True")
            && line.contains("Deployed")
            && line.contains("Service is ready")));
    }

    #[test]
    fn cloud_run_detail_lines_json_tab_yields_raw_json() {
        let detail = cloud_run_service_detail();
        let lines = cloud_run_detail_body(&detail, CloudRunDetailTab::Json, 0).display_lines();
        assert!(lines.iter().any(|line| line.contains("\"name\": \"api\"")));
    }

    #[test]
    fn cloud_run_revision_detail_lines_summary_shows_name_and_image() {
        let detail = cloud_run_revision_detail();
        let lines =
            cloud_run_revision_detail_body(&detail, CloudRunRevisionTab::Summary).display_lines();
        assert!(lines.iter().any(|line| line == "Name: api-00002"));
        assert!(lines.iter().any(|line| line == "Image: gcr.io/x/api:1"));
        assert!(lines.iter().any(|line| line.starts_with("Ready: ")));
    }

    #[test]
    fn cloud_run_revision_detail_lines_conditions_and_json() {
        let detail = cloud_run_revision_detail();
        let conditions = cloud_run_revision_detail_body(&detail, CloudRunRevisionTab::Conditions)
            .display_lines();
        assert!(
            conditions
                .iter()
                .any(|line| line.contains("Active") && line.contains("True"))
        );
        let json =
            cloud_run_revision_detail_body(&detail, CloudRunRevisionTab::Json).display_lines();
        assert!(
            json.iter()
                .any(|line| line.contains("\"name\": \"api-00002\""))
        );
    }

    #[test]
    fn storage_bucket_detail_metadata_tab_sanitizes_hostile_label_keys_and_values() {
        let detail = hostile_bucket_detail();
        let body = storage_bucket_detail_body(&detail, StorageDetailTab::Metadata);
        let lines = body.display_lines();

        assert!(
            !lines
                .iter()
                .any(|line| line.chars().any(|c| c.is_control())),
            "expected no control characters in rendered label lines, got: {lines:?}"
        );
        assert!(
            !lines.iter().any(|line| line.contains('\u{1b}')),
            "expected no raw ESC byte in rendered label lines, got: {lines:?}"
        );
        // Non-hostile label content is preserved.
        assert!(lines.iter().any(|line| line.contains("env: prod")));
        // The clipboard half of the same invariant: labels and values handed to
        // the copy path are sanitized too.
        assert_fields_carry_no_escape_sequences(&body);
        assert!(
            body.fields()
                .iter()
                .any(|(_, field)| field.label == "env" && field.value == "prod"),
            "expected the benign label to stay copyable, got: {:?}",
            body.fields()
        );
    }

    #[test]
    fn storage_object_detail_summary_tab_sanitizes_hostile_content_type() {
        let detail = hostile_object_detail();
        let body = storage_object_detail_body(&detail, StorageDetailTab::Summary);
        let lines = body.display_lines();

        assert!(
            !lines
                .iter()
                .any(|line| line.chars().any(|c| c.is_control())),
            "expected no control characters in rendered summary lines, got: {lines:?}"
        );
        assert!(
            !lines.iter().any(|line| line.contains('\u{1b}')),
            "expected no raw ESC byte in rendered summary lines, got: {lines:?}"
        );
        assert!(
            lines
                .iter()
                .any(|line| line.starts_with("Content type:") && line.contains("text/plain"))
        );
        // The clipboard half of the same invariant: the copy payload is
        // sanitized, not just the text the pane draws.
        assert_fields_carry_no_escape_sequences(&body);
        assert!(
            body.fields()
                .iter()
                .any(|(_, field)| field.label == "Content type"),
            "expected the content type to stay copyable, got: {:?}",
            body.fields()
        );
    }

    #[test]
    fn storage_bucket_detail_never_surfaces_kms_key_material() {
        let mut detail = hostile_bucket_detail();
        detail.raw_json =
            "{\n  \"encryption\": {\n    \"defaultKmsKeyName\": \"<redacted>\"\n  }\n}".to_string();
        for tab in StorageDetailTab::ALL {
            let lines = storage_bucket_detail_body(&detail, tab).display_lines();
            assert!(
                !lines.iter().any(|line| line.contains("cryptoKeys/gcs-key")),
                "KMS key path leaked in {tab:?} tab: {lines:?}"
            );
        }
    }

    #[test]
    fn storage_object_detail_never_surfaces_kms_key_material() {
        let mut detail = hostile_object_detail();
        detail.raw_json = "{\n  \"kmsKeyName\": \"<redacted>\"\n}".to_string();
        for tab in StorageDetailTab::ALL {
            let lines = storage_object_detail_body(&detail, tab).display_lines();
            assert!(
                !lines.iter().any(|line| line.contains("cryptoKeys/gcs-key")),
                "KMS key path leaked in {tab:?} tab: {lines:?}"
            );
        }
    }

    /// Asserts no copy payload carries an escape sequence, in either half.
    /// Sanitization must live in the projection, not the renderer, or the
    /// clipboard path would ship escape sequences the pane never displays.
    ///
    /// The two halves are held to their own sink's policy. The label is
    /// rendered back on the status line, so it keeps no control character at
    /// all. The value goes to the clipboard, so newlines and tabs are exempt —
    /// a multi-line value has to paste back whole — but nothing else survives,
    /// and it must be *stripped* rather than mangled: a `U+FFFD` in the value
    /// would mean an introducer was replaced before the escape stripper ran,
    /// leaving readable `[2J` residue on the clipboard.
    fn assert_fields_carry_no_escape_sequences(body: &DetailBody) {
        let fields = body.fields();
        let introducer = |text: &str| {
            text.chars().any(|c| {
                matches!(
                    c,
                    '\u{1b}' | '\u{90}' | '\u{98}' | '\u{9b}' | '\u{9d}'..='\u{9f}'
                )
            })
        };
        assert!(
            !fields
                .iter()
                .any(|(_, field)| introducer(&field.label) || introducer(&field.value)),
            "expected no escape introducer in copyable fields, got: {fields:?}"
        );
        assert!(
            !fields
                .iter()
                .any(|(_, field)| field.label.chars().any(|c| c.is_control())),
            "expected no control characters in copyable field labels, got: {fields:?}"
        );
        assert!(
            !fields.iter().any(|(_, field)| field
                .value
                .chars()
                .any(|c| c.is_control() && c != '\n' && c != '\t')),
            "expected no control characters beyond newlines and tabs in copyable \
             field values, got: {fields:?}"
        );
        assert!(
            !fields
                .iter()
                .any(|(_, field)| field.value.contains('\u{FFFD}')),
            "expected copyable field values to be stripped, not mangled into \
             replacement residue, got: {fields:?}"
        );
    }

    /// Asserts the body exposes `label`/`value` as a copyable field.
    fn assert_copyable(body: &DetailBody, label: &str, value: &str) {
        let fields = body.fields();
        assert!(
            fields
                .iter()
                .any(|(_, field)| field.label == label && field.value == value),
            "expected copyable {label}={value}, got: {fields:?}"
        );
    }

    #[test]
    fn compute_network_tab_exposes_external_ip_as_a_copyable_field() {
        let detail = compute_instance_detail();
        let body = compute_detail_body(&detail, ComputeDetailTab::Network);
        assert_copyable(&body, "External IP", "34.1.2.3");
    }

    #[test]
    fn cloud_sql_connectivity_tab_exposes_connection_name_as_a_copyable_field() {
        let detail = cloud_sql_instance_detail();
        let body = cloud_sql_detail_body(&detail, CloudSqlDetailTab::Connectivity);
        assert_copyable(&body, "Connection name", "dev-project:europe-west1:db");
    }

    #[test]
    fn cloud_run_configuration_tab_exposes_image_and_service_account() {
        let detail = cloud_run_service_detail();
        let body = cloud_run_detail_body(&detail, CloudRunDetailTab::Configuration, 0);
        assert_copyable(&body, "Image", "gcr.io/x/api:1");
        assert_copyable(&body, "Service account", "svc@project.iam");
    }

    #[test]
    fn gke_networking_tab_exposes_endpoint_as_a_copyable_field() {
        let detail = gke_cluster_detail();
        let body = gke_detail_body(&detail, GkeDetailTab::Networking);
        assert_copyable(&body, "Endpoint", "10.0.0.1");
    }

    #[test]
    fn storage_bucket_summary_tab_exposes_bucket_name_as_a_copyable_field() {
        let detail = hostile_bucket_detail();
        let body = storage_bucket_detail_body(&detail, StorageDetailTab::Summary);
        assert_copyable(&body, "Name", "alpha-bucket");
    }

    #[test]
    fn from_raw_json_yields_no_fields_and_sets_raw_json() {
        let raw = "{\n  \"name\": \"api\"\n}";
        let body = DetailBody::from_raw_json(raw);
        assert_eq!(
            body.display_lines(),
            vec![
                "{".to_string(),
                "  \"name\": \"api\"".to_string(),
                "}".to_string(),
            ]
        );
        assert!(body.fields().is_empty());
        assert_eq!(body.raw_json.as_deref(), Some(raw));
    }

    #[test]
    fn an_error_hides_a_payload_that_is_still_in_state() {
        let detail = compute_instance_detail();
        // Every service's state setter clears the payload when it records an
        // error, so this pair can only be built directly — the guard is what
        // keeps the pane empty if a future setter ever stops doing that.
        assert!(loaded(Some(&detail), Some("describe failed")).is_none());
        assert!(loaded(Some(&detail), None).is_some());
        assert!(loaded::<ComputeInstanceDetail>(None, None).is_none());
    }

    // ---- every tab of every service is copyable ----
    //
    // The fixtures below are fully populated, so a tab with no copyable field
    // is a real gap in the feature, not an artefact of missing test data.

    /// Asserts a non-JSON tab offers at least one field to yank.
    fn assert_tab_has_fields(body: &DetailBody, tab: &str) {
        assert!(
            !body.fields().is_empty(),
            "the {tab} tab exposes no copyable field, lines: {:?}",
            body.display_lines()
        );
    }

    /// Asserts a JSON tab yanks the document as a unit and nothing per line.
    fn assert_json_tab(body: &DetailBody, raw_json: &str) {
        assert_eq!(body.raw_json.as_deref(), Some(raw_json));
        assert!(body.fields().is_empty());
    }

    #[test]
    fn every_compute_detail_tab_exposes_copyable_fields() {
        let mut detail = compute_instance_detail();
        detail.metadata = vec![ComputeKeyValue {
            key: "startup-script".to_string(),
            value: "#!/bin/sh".to_string(),
        }];

        for tab in ComputeDetailTab::ALL {
            let body = compute_detail_body(&detail, tab);
            match tab {
                ComputeDetailTab::Json => assert_json_tab(&body, &detail.raw_json),
                _ => assert_tab_has_fields(&body, tab.label()),
            }
        }
    }

    #[test]
    fn every_gke_detail_tab_exposes_copyable_fields() {
        let detail = gke_cluster_detail();

        for tab in GkeDetailTab::ALL {
            let body = gke_detail_body(&detail, tab);
            match tab {
                GkeDetailTab::Json => assert_json_tab(&body, &detail.raw_json),
                _ => assert_tab_has_fields(&body, tab.label()),
            }
        }
    }

    #[test]
    fn every_cloud_sql_detail_tab_exposes_copyable_fields() {
        let mut detail = cloud_sql_instance_detail();
        detail.master_instance = Some("db-primary".to_string());

        for tab in CloudSqlDetailTab::ALL {
            let body = cloud_sql_detail_body(&detail, tab);
            match tab {
                CloudSqlDetailTab::Json => assert_json_tab(&body, &detail.raw_json),
                _ => assert_tab_has_fields(&body, tab.label()),
            }
        }
    }

    #[test]
    fn every_cloud_run_detail_tab_exposes_copyable_fields() {
        let detail = cloud_run_service_detail();

        for tab in CloudRunDetailTab::ALL {
            let body = cloud_run_detail_body(&detail, tab, 0);
            match tab {
                CloudRunDetailTab::Json => assert_json_tab(&body, &detail.raw_json),
                _ => assert_tab_has_fields(&body, tab.label()),
            }
        }
    }

    #[test]
    fn every_cloud_run_revision_tab_exposes_copyable_fields() {
        let detail = cloud_run_revision_detail();

        for tab in CloudRunRevisionTab::ALL {
            let body = cloud_run_revision_detail_body(&detail, tab);
            match tab {
                CloudRunRevisionTab::Json => assert_json_tab(&body, &detail.raw_json),
                _ => assert_tab_has_fields(&body, tab.label()),
            }
        }
    }

    #[test]
    fn every_storage_bucket_detail_tab_exposes_copyable_fields() {
        let detail = hostile_bucket_detail();

        for tab in StorageDetailTab::ALL {
            let body = storage_bucket_detail_body(&detail, tab);
            match tab {
                StorageDetailTab::Json => assert_json_tab(&body, &detail.raw_json),
                _ => assert_tab_has_fields(&body, tab.label()),
            }
        }
    }

    #[test]
    fn every_storage_object_detail_tab_exposes_copyable_fields() {
        let mut detail = hostile_object_detail();
        detail.retention_expiration = Some("2025-01-01T00:00:00Z".to_string());

        for tab in StorageDetailTab::ALL {
            let body = storage_object_detail_body(&detail, tab);
            match tab {
                StorageDetailTab::Json => assert_json_tab(&body, &detail.raw_json),
                _ => assert_tab_has_fields(&body, tab.label()),
            }
        }
    }
}
