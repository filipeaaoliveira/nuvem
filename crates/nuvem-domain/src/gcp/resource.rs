//! The resources nuvem lists: one flat type per table row, one per detail pane.
//!
//! Each type is the API's answer already reduced to the facts a surface paints.
//! Statuses are enums with an `Unknown`/`Unrecognized` arm carrying the raw
//! string, so a value the API adds later renders as itself instead of becoming
//! a panic or a silent default. Detail types carry a `raw_json` beside the
//! parsed fields, which is what the detail pane falls back to for anything not
//! modelled here.
//!
//! [`ResourceServiceKind`] names the services themselves, and its `ALL` is the
//! list that roughly fifteen exhaustiveness walks across this workspace iterate
//! to prove every per-service site is wired.

use std::borrow::Cow;

use serde::{Deserialize, Serialize};

/// The label a status or state value nuvem does not recognise renders as.
///
/// An API adds enum values over time, and a value this build has no variant
/// for is kept verbatim in an `Unknown`/`Unrecognized` arm so it renders as
/// itself rather than as a guess. "As itself" means readable, though: a
/// `MAINTENANCE_WINDOW` cell is spelled `Maintenance window`, the way the
/// labels beside it are spelled, and never as the Rust value that carries it.
///
/// Only an API constant is respelled: a value with no lowercase letter in it,
/// such as `SUSPENDED` or `GEN_3`. Anything else is the API's own prose and is
/// returned as it came. A value with no letter or digit in it is `Unknown`,
/// because a blank cell in a status column reads as "no problem".
pub fn readable_api_value(raw: &str) -> Cow<'_, str> {
    let trimmed = raw.trim();
    // A value with no letter or digit in it (`_`, `--`) would respell to
    // blanks, and a blank status cell reads as "nothing wrong".
    if !trimmed.chars().any(char::is_alphanumeric) {
        return Cow::Borrowed("Unknown");
    }
    if trimmed.chars().any(char::is_lowercase) {
        return Cow::Borrowed(trimmed);
    }
    let spaced = trimmed.replace(['_', '-'], " ").to_lowercase();
    let mut characters = spaced.chars();
    let label = match characters.next() {
        Some(first) => first.to_uppercase().chain(characters).collect(),
        None => String::new(),
    };
    Cow::Owned(label)
}

/// Declares [`ResourceServiceKind`] and generates its `ALL` from the same
/// variant list, so the two cannot diverge.
///
/// `ALL` is what roughly fifteen exhaustiveness walks across this workspace
/// iterate to prove that a service is wired at every per-service site. Written
/// by hand as a fixed-size array, it is the one enumeration site where a
/// missed variant is silent *and* disables all the others at once: the walks
/// keep passing, because they are walking a list the new service is not on.
/// Generating both from one list removes the place the omission could happen —
/// there is nowhere to declare a variant except inside the invocation, and the
/// invocation regenerates `ALL`.
///
/// Asserting `ALL.len() == N` would not achieve this. It is the same
/// remembered invariant with an extra step, because the author who forgot to
/// extend `ALL` also forgot to bump the number.
///
/// Deliberately separate from `lifecycle_actions!`, which does the same job
/// for the per-service action enums: that macro lives in the binary crate and
/// this enum lives here, and a second small macro is better than one macro
/// carrying two shapes.
///
/// The generated `ALL` is deliberately the only item this produces: every
/// other method on this enum is an exhaustive `match`, which the compiler
/// already forces open when a variant appears.
macro_rules! service_kinds {
    (
        $(#[$enum_meta:meta])*
        pub enum $name:ident {
            $($variant:ident),+ $(,)?
        }
    ) => {
        $(#[$enum_meta])*
        pub enum $name {
            $($variant,)+
        }

        impl $name {
            /// Every resource service kind, in declaration order, generated
            /// from the enum's own variant list by `service_kinds!` so a
            /// caller that must cover the whole set walks the enum instead of
            /// re-listing it.
            pub const ALL: [Self; [$(stringify!($variant)),+].len()] =
                [$(Self::$variant),+];
        }
    };
}

service_kinds! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
    pub enum ResourceServiceKind {
        ComputeInstances,
        GkeClusters,
        CloudSql,
        CloudRunServices,
        CloudStorage,
        SecretManagerSecrets,
        CloudFunctions,
        PersistentDisks,
        PubSubTopics,
        PubSubSubscriptions,
        BigQueryDatasets,
        IamServiceAccounts,
        IamPolicy,
        VpcNetworks,
        VpcSubnets,
        FirewallRules,
    }
}

impl ResourceServiceKind {
    /// The service's name as a heading shows it — a product name, plural. See
    /// [`selection_noun`](Self::selection_noun) for the singular form a
    /// sentence about one resource needs.
    pub fn label(self) -> &'static str {
        match self {
            Self::ComputeInstances => "GCE instances",
            Self::GkeClusters => "GKE clusters",
            Self::CloudSql => "Cloud SQL",
            Self::CloudRunServices => "Cloud Run",
            Self::CloudStorage => "Cloud Storage",
            Self::SecretManagerSecrets => "Secret Manager",
            Self::CloudFunctions => "Cloud Functions",
            Self::PersistentDisks => "Persistent disks",
            Self::PubSubTopics => "Pub/Sub topics",
            Self::PubSubSubscriptions => "Pub/Sub subscriptions",
            Self::BigQueryDatasets => "BigQuery datasets",
            Self::IamServiceAccounts => "Service accounts",
            Self::IamPolicy => "IAM policy",
            Self::VpcNetworks => "VPC networks",
            Self::VpcSubnets => "VPC subnets",
            Self::FirewallRules => "Firewall rules",
        }
    }

    /// One of this service's resources, singular — the noun a message uses to
    /// name what it was looking for and did not find (`"No GCE instance
    /// selected to jump from"`, `"... to open logs for"`).
    ///
    /// Beside [`label`](Self::label) rather than in the feature that first
    /// needed it: it is a total function of the kind with no state behind it,
    /// and one table is what keeps two features from naming the same resource
    /// differently the first time a service is renamed or added.
    pub fn selection_noun(self) -> &'static str {
        match self {
            Self::ComputeInstances => "GCE instance",
            Self::GkeClusters => "GKE cluster",
            Self::CloudSql => "Cloud SQL instance",
            Self::CloudRunServices => "Cloud Run service",
            Self::CloudStorage => "Cloud Storage bucket",
            Self::SecretManagerSecrets => "secret",
            Self::CloudFunctions => "Cloud Function",
            Self::PersistentDisks => "persistent disk",
            Self::PubSubTopics => "Pub/Sub topic",
            Self::PubSubSubscriptions => "Pub/Sub subscription",
            Self::BigQueryDatasets => "BigQuery dataset",
            Self::IamServiceAccounts => "service account",
            Self::IamPolicy => "policy binding",
            Self::VpcNetworks => "VPC network",
            Self::VpcSubnets => "VPC subnet",
            Self::FirewallRules => "firewall rule",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ResourceStatus {
    Running,
    Stopped,
    Suspended,
    Pending,
    Unknown(String),
}

impl ResourceStatus {
    /// The status as a table cell and a detail field show it.
    pub fn label(&self) -> Cow<'_, str> {
        match self {
            Self::Running => Cow::Borrowed("Running"),
            Self::Stopped => Cow::Borrowed("Stopped"),
            Self::Suspended => Cow::Borrowed("Suspended"),
            Self::Pending => Cow::Borrowed("Pending"),
            Self::Unknown(raw) => readable_api_value(raw),
        }
    }
}

/// A Compute Engine instance's table row, from `compute instances list`.
///
/// Not asked for the network tags and service accounts a firewall rule can
/// select on; those are on [`ComputeInstanceDetail`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ComputeInstance {
    pub id: String,
    pub name: String,
    pub zone: String,
    pub status: ResourceStatus,
    pub machine_type: String,
    pub internal_ip: Option<String>,
    pub external_ip: Option<String>,
}

/// One instance described in full, from `compute instances describe`.
///
/// Beyond the row: the disks and NICs, the label and metadata pairs, the two
/// firewall selectors, and `raw_json` for everything not modelled here.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ComputeInstanceDetail {
    pub name: String,
    pub zone: String,
    pub status: ResourceStatus,
    pub machine_type: String,
    pub creation_timestamp: Option<String>,
    pub network_interfaces: Vec<ComputeNetworkInterface>,
    /// The instance's network tags — `tags.items` on the wire.
    ///
    /// One of the three things a firewall rule can select an instance by, and
    /// on the *detail* rather than the row because `compute instances list`
    /// is not asked for them. An empty list is the common case and means the
    /// instance is selected only by rules that name no target at all.
    pub tags: Vec<String>,
    /// The service accounts attached to the instance, by email — the second
    /// selector a firewall rule can use.
    ///
    /// Emails only. The scopes each account was granted sit beside them on the
    /// wire and are not carried: a firewall rule selects on the identity, and
    /// nothing here asks what that identity may do.
    pub service_accounts: Vec<String>,
    pub disks: Vec<ComputeDisk>,
    pub labels: Vec<ComputeKeyValue>,
    pub metadata: Vec<ComputeKeyValue>,
    pub raw_json: String,
}

/// One network interface of an instance: what it is attached to, and the
/// addresses it holds.
///
/// The network and subnet are ids shortened from their self-links, with the
/// subnet's region kept beside it because a subnet name alone is not an
/// identity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ComputeNetworkInterface {
    pub name: Option<String>,
    /// The network's id, shortened from its self-link. No region beside it, and
    /// none is discarded: a network's link is `.../global/networks/<name>`, so
    /// the name is the whole identity within its project.
    pub network: Option<String>,
    /// The subnet's id, shortened from its self-link.
    ///
    /// **Not** an identity on its own, which is why [`subnetwork_region`] sits
    /// beside it: an auto-mode network calls every one of its subnets
    /// `default`, so this name alone matches one row per region.
    ///
    /// [`subnetwork_region`]: Self::subnetwork_region
    pub subnetwork: Option<String>,
    /// The region half of the subnet's identity, read out of the same
    /// self-link.
    ///
    /// Carried as its own field rather than by keeping the whole link, so the
    /// two existing readers of [`subnetwork`](Self::subnetwork) — the detail
    /// pane's Network tab and the `{subnetwork}` jump field — keep rendering
    /// the short name they already rendered. `None` for a link that names no
    /// region, which is also what an absent subnetwork gives.
    pub subnetwork_region: Option<String>,
    pub internal_ip: Option<String>,
    pub external_ip: Option<String>,
}

/// A disk as one instance attaches it.
///
/// The attachment's facts, not the disk resource — `boot` and `auto_delete`
/// describe this attachment, and the same disk seen through the Persistent
/// Disks service is a [`PersistentDisk`] instead.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ComputeDisk {
    pub name: Option<String>,
    pub source: Option<String>,
    pub boot: bool,
    pub auto_delete: bool,
    pub disk_type: Option<String>,
    pub size_gb: Option<String>,
}

/// One label or metadata entry. A pair rather than a map entry, so the order
/// the API returned is the order that renders.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ComputeKeyValue {
    pub key: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum GkeClusterStatus {
    Running,
    Reconciling,
    Stopping,
    Error,
    Unknown(String),
}

impl GkeClusterStatus {
    /// The status as a table cell and a detail field show it, for a cluster
    /// and for a node pool alike.
    pub fn label(&self) -> Cow<'_, str> {
        match self {
            Self::Running => Cow::Borrowed("Running"),
            Self::Reconciling => Cow::Borrowed("Reconciling"),
            Self::Stopping => Cow::Borrowed("Stopping"),
            Self::Error => Cow::Borrowed("Error"),
            Self::Unknown(raw) => readable_api_value(raw),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EndpointVisibility {
    Public,
    Private,
    Unknown,
}

/// A GKE cluster's table row.
///
/// `location` is a zone for a zonal cluster and a region for a regional one.
/// The two are not told apart here because `--location` takes either, so the
/// distinction has no consumer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GkeCluster {
    pub name: String,
    pub location: String,
    pub status: GkeClusterStatus,
    pub current_master_version: Option<String>,
    pub current_node_count: Option<u32>,
    pub endpoint_visibility: EndpointVisibility,
}

/// One cluster described in full: its node pools and addons, plus the
/// networking facts — `private_cluster`, `endpoint` — that decide whether it
/// is reachable at all.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GkeClusterDetail {
    pub name: String,
    pub location: String,
    pub status: GkeClusterStatus,
    pub current_master_version: Option<String>,
    pub current_node_version: Option<String>,
    pub endpoint: Option<String>,
    pub private_cluster: bool,
    pub network: Option<String>,
    pub subnetwork: Option<String>,
    pub node_pools: Vec<GkeNodePool>,
    pub addons: Vec<GkeAddon>,
    pub labels: Vec<ComputeKeyValue>,
    pub raw_json: String,
}

/// One node pool of a cluster. Carries its own version and machine type,
/// which can differ from the cluster's control plane and from the other
/// pools'.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GkeNodePool {
    pub name: String,
    pub status: GkeClusterStatus,
    pub version: Option<String>,
    pub machine_type: Option<String>,
    pub autoscaling: Option<String>,
    pub node_count: Option<u32>,
}

/// One entry of a cluster's addon config: the addon as the API names it, and
/// whether this cluster has it turned on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GkeAddon {
    pub name: String,
    pub enabled: bool,
}

/// A Cloud Storage bucket's table row.
///
/// `uniform_bucket_level_access` and `public_access_prevention` are on the row
/// rather than only on the detail: together they decide whether an object ACL
/// can still make something in the bucket world-readable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StorageBucket {
    pub name: String,
    pub location: String,
    pub location_type: Option<String>,
    pub storage_class: String,
    pub versioning_enabled: bool,
    pub uniform_bucket_level_access: bool,
    pub public_access_prevention: Option<String>,
    pub time_created: Option<String>,
    pub updated: Option<String>,
}

/// One bucket described in full: the row's settings plus retention,
/// encryption, and `requester_pays` — which changes who is billed for reads,
/// and makes a listing fail outright for a caller that did not opt in.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StorageBucketDetail {
    pub name: String,
    pub location: String,
    pub location_type: Option<String>,
    pub storage_class: String,
    pub versioning_enabled: bool,
    pub uniform_bucket_level_access: bool,
    pub public_access_prevention: Option<String>,
    pub requester_pays: bool,
    pub retention_period: Option<String>,
    pub default_kms_key_present: bool,
    pub labels: Vec<ComputeKeyValue>,
    pub time_created: Option<String>,
    pub updated: Option<String>,
    pub raw_json: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CloudRunServiceStatus {
    Ready,
    NotReady,
    Reconciling,
    Failed,
    Unknown(String),
}

impl CloudRunServiceStatus {
    /// The status as a table cell and a detail field show it, for a service
    /// and for a revision alike.
    pub fn label(&self) -> Cow<'_, str> {
        match self {
            Self::Ready => Cow::Borrowed("Ready"),
            Self::NotReady => Cow::Borrowed("Not ready"),
            Self::Reconciling => Cow::Borrowed("Reconciling"),
            Self::Failed => Cow::Borrowed("Failed"),
            Self::Unknown(raw) => readable_api_value(raw),
        }
    }
}

/// A Cloud Run service's table row.
///
/// `traffic_summary` is the split already rendered for a cell; the targets
/// themselves are on [`CloudRunServiceDetail`]. `ready_message` is the API's
/// own explanation of a status that is not ready.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CloudRunService {
    pub name: String,
    pub region: String,
    pub url: Option<String>,
    pub latest_ready_revision: Option<String>,
    pub latest_created_revision: Option<String>,
    pub traffic_summary: String,
    pub status: CloudRunServiceStatus,
    pub ready_message: Option<String>,
}

/// One slice of a service's traffic split: which revision takes what
/// percentage, or the latest one via `latest`, plus the tagged URL if the
/// slice has a tag.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CloudRunTrafficTarget {
    pub revision_name: Option<String>,
    pub tag: Option<String>,
    pub percent: Option<u32>,
    pub latest: bool,
    pub url: Option<String>,
}

/// One entry of a service's or revision's status conditions, as the API
/// states it. This is what explains a not-ready status when no other field
/// does.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CloudRunCondition {
    pub type_: String,
    pub status: String,
    pub reason: Option<String>,
    pub message: Option<String>,
    pub last_transition_time: Option<String>,
}

/// One revision of a service, as the service's detail lists it.
/// `active_percent` is its share of the traffic split, `None` for a revision
/// the split does not mention.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CloudRunRevision {
    pub name: String,
    pub ready: CloudRunServiceStatus,
    pub created: Option<String>,
    pub image: Option<String>,
    pub active_percent: Option<u32>,
}

/// One Cloud Run service described in full: its traffic split, its revisions,
/// its status conditions, and the container and scaling settings the service
/// reports.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CloudRunServiceDetail {
    pub name: String,
    pub region: String,
    pub url: Option<String>,
    pub status: CloudRunServiceStatus,
    pub latest_ready_revision: Option<String>,
    pub latest_created_revision: Option<String>,
    pub traffic: Vec<CloudRunTrafficTarget>,
    pub revisions: Vec<CloudRunRevision>,
    pub conditions: Vec<CloudRunCondition>,
    pub image: Option<String>,
    pub cpu: Option<String>,
    pub memory: Option<String>,
    pub concurrency: Option<u32>,
    pub service_account: Option<String>,
    pub ingress: Option<String>,
    pub min_instances: Option<u32>,
    pub max_instances: Option<u32>,
    pub labels: Vec<ComputeKeyValue>,
    pub raw_json: String,
}

/// A common prefix in an object listing — the "folder" a delimiter-scoped
/// list returns in place of the objects beneath it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoragePrefix {
    pub name: String,
}

/// One object in a bucket listing.
///
/// The hold and retention flags are carried because they are why a delete
/// fails; nothing else in the row would explain the refusal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StorageObject {
    pub key: String,
    pub size_bytes: Option<u64>,
    pub storage_class: Option<String>,
    pub generation: Option<String>,
    pub updated: Option<String>,
    pub content_type: Option<String>,
    pub kms_encrypted: bool,
    pub temporary_hold: bool,
    pub event_based_hold: bool,
    pub has_retention: bool,
}

/// One page of an object listing: the prefixes and objects at this level,
/// plus the token that fetches the next page — `None` when this was the
/// last.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StorageObjectPage {
    pub prefixes: Vec<StoragePrefix>,
    pub objects: Vec<StorageObject>,
    pub next_page_token: Option<String>,
}

/// One object described in full: the listing's facts plus its checksums and
/// retention expiry, with `raw_json` for the rest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StorageObjectDetail {
    pub key: String,
    pub size_bytes: Option<u64>,
    pub storage_class: Option<String>,
    pub generation: Option<String>,
    pub updated: Option<String>,
    pub content_type: Option<String>,
    pub kms_encrypted: bool,
    pub temporary_hold: bool,
    pub event_based_hold: bool,
    pub retention_expiration: Option<String>,
    pub md5_hash: Option<String>,
    pub crc32c: Option<String>,
    pub raw_json: String,
}

/// One revision described in full, fetched on its own rather than through its
/// service — so it carries the same container and scaling settings for a
/// revision that may no longer be serving any traffic.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CloudRunRevisionDetail {
    pub name: String,
    pub service: Option<String>,
    pub region: String,
    pub ready: CloudRunServiceStatus,
    pub conditions: Vec<CloudRunCondition>,
    pub image: Option<String>,
    pub cpu: Option<String>,
    pub memory: Option<String>,
    pub concurrency: Option<u32>,
    pub min_instances: Option<u32>,
    pub max_instances: Option<u32>,
    pub service_account: Option<String>,
    pub created: Option<String>,
    pub raw_json: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CloudSqlStatus {
    Runnable,
    Stopped,
    PendingCreate,
    Maintenance,
    Failed,
    Suspended,
    Unknown(String),
}

impl CloudSqlStatus {
    /// The status as a table cell and a detail field show it.
    pub fn label(&self) -> Cow<'_, str> {
        match self {
            Self::Runnable => Cow::Borrowed("Runnable"),
            Self::Stopped => Cow::Borrowed("Stopped"),
            Self::PendingCreate => Cow::Borrowed("Pending create"),
            Self::Maintenance => Cow::Borrowed("Maintenance"),
            Self::Failed => Cow::Borrowed("Failed"),
            Self::Suspended => Cow::Borrowed("Suspended"),
            Self::Unknown(raw) => readable_api_value(raw),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CloudSqlAvailability {
    Zonal,
    Regional,
    Unknown,
}

impl CloudSqlAvailability {
    /// The availability column's text. "HA (regional)" rather than the API's
    /// word, because the question the column answers is whether a zone outage
    /// takes the instance down, not which scope it was provisioned at.
    pub fn label(self) -> &'static str {
        match self {
            Self::Zonal => "Zonal",
            Self::Regional => "HA (regional)",
            Self::Unknown => "unknown",
        }
    }

    /// Whether the instance survives losing a zone. Only `Regional` does —
    /// an unknown availability is not assumed to be highly available.
    pub fn is_ha(self) -> bool {
        matches!(self, Self::Regional)
    }
}

/// A Cloud SQL instance's table row.
///
/// `database_version` is the API's raw enum (`POSTGRES_15`) and `engine` the
/// product name derived from it; both are kept so the column can read as a
/// product without losing the exact version the API stated.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CloudSqlInstance {
    pub name: String,
    pub database_version: String, // raw, e.g. "POSTGRES_15"
    pub engine: String,           // normalized, e.g. "PostgreSQL"
    pub region: String,
    pub tier: String,
    pub availability: CloudSqlAvailability,
    pub status: CloudSqlStatus,
    pub connectivity: EndpointVisibility,
}

/// When Google may restart the instance for maintenance: day and hour as the
/// API numbers them, plus the update track that decides how early this
/// instance receives a release.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CloudSqlMaintenanceWindow {
    pub day: Option<u32>,
    pub hour: Option<u32>,
    pub update_track: Option<String>,
}

/// An instance's backup settings.
///
/// `enabled` covers only the daily backup. How far back a restore can actually
/// reach is `point_in_time_recovery` together with
/// `transaction_log_retention_days`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CloudSqlBackupConfig {
    pub enabled: bool,
    pub start_time: Option<String>,
    pub location: Option<String>,
    pub point_in_time_recovery: bool,
    pub transaction_log_retention_days: Option<u32>,
}

/// A database user on an instance: its name, the host it is scoped to, and
/// its kind. Never a password — nuvem does not issue a call that returns one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CloudSqlUser {
    pub name: String,
    pub host: Option<String>,
    pub kind: Option<String>,
} // NEVER a password

/// One Cloud SQL instance described in full: how it is reached (connection
/// name, which IPs exist, whether SSL is required), its backup and maintenance
/// settings, and its replication relationships.
///
/// `users` is lazily filled and stays empty until the Users tab is opened, so
/// an empty list is not evidence the instance has no users.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CloudSqlInstanceDetail {
    pub name: String,
    pub database_version: String,
    pub engine: String,
    pub region: String,
    pub tier: String,
    pub availability: CloudSqlAvailability,
    pub status: CloudSqlStatus,
    pub activation_policy: Option<String>,
    pub creation_timestamp: Option<String>,
    pub connection_name: Option<String>,
    pub public_ip: Option<String>,
    pub private_ip: Option<String>,
    pub require_ssl: bool,
    pub private_network: Option<String>,
    pub maintenance_window: Option<CloudSqlMaintenanceWindow>,
    pub maintenance_version: Option<String>,
    pub backup: CloudSqlBackupConfig,
    pub replicas: Vec<String>,
    pub master_instance: Option<String>,
    pub users: Vec<CloudSqlUser>, // lazy-loaded; empty until Users tab opened
    pub labels: Vec<ComputeKeyValue>,
    pub raw_json: String, // REDACTED describe JSON
}

/// The lifecycle state of one Cloud Function.
///
/// Every value the v2 API documents, and one arm — [`Self::Unrecognized`] — for
/// anything it adds later. `UNKNOWN` is one of the documented values, so it has
/// its own unit variant: "the API has lost track of this function" and "nuvem
/// does not recognise this string" are different facts, and one arm for both
/// would lose the first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CloudFunctionState {
    Unspecified,
    Active,
    Failed,
    Deploying,
    Deleting,
    Unknown,
    Detaching,
    DetachFailed,
    Unrecognized(String),
}

impl CloudFunctionState {
    /// Every unit variant, for callers that walk the states. Kept in sync by
    /// hand: the fixed length means adding a variant here is a deliberate edit,
    /// not something the compiler will demand.
    pub const ALL: [Self; 8] = [
        Self::Unspecified,
        Self::Active,
        Self::Failed,
        Self::Deploying,
        Self::Deleting,
        Self::Unknown,
        Self::Detaching,
        Self::DetachFailed,
    ];

    /// The API's own string for this state. Round-trips
    /// [`CloudFunctionState::from_api`], an unrecognised value included, since
    /// that arm carries the string it was given.
    pub fn api_value(&self) -> &str {
        match self {
            Self::Unspecified => "STATE_UNSPECIFIED",
            Self::Active => "ACTIVE",
            Self::Failed => "FAILED",
            Self::Deploying => "DEPLOYING",
            Self::Deleting => "DELETING",
            Self::Unknown => "UNKNOWN",
            Self::Detaching => "DETACHING",
            Self::DetachFailed => "DETACH_FAILED",
            Self::Unrecognized(raw) => raw,
        }
    }

    /// The one-word rendering for a table cell.
    pub fn label(&self) -> Cow<'_, str> {
        match self {
            Self::Unspecified => Cow::Borrowed("Unspecified"),
            Self::Active => Cow::Borrowed("Active"),
            Self::Failed => Cow::Borrowed("Failed"),
            Self::Deploying => Cow::Borrowed("Deploying"),
            Self::Deleting => Cow::Borrowed("Deleting"),
            Self::Unknown => Cow::Borrowed("Unknown"),
            Self::Detaching => Cow::Borrowed("Detaching"),
            Self::DetachFailed => Cow::Borrowed("Detach failed"),
            Self::Unrecognized(raw) => readable_api_value(raw),
        }
    }

    /// Reads the API's `state` string.
    ///
    /// An absent field is [`Self::Unspecified`] rather than an error or a
    /// recognised value: `STATE_UNSPECIFIED` is the proto zero value, and
    /// proto3 JSON omits zero-valued enums, so a missing `state` *is* that
    /// value. Anything else unrecognised is carried through verbatim.
    pub fn from_api(raw: Option<&str>) -> Self {
        match raw {
            None | Some("STATE_UNSPECIFIED") => Self::Unspecified,
            Some("ACTIVE") => Self::Active,
            Some("FAILED") => Self::Failed,
            Some("DEPLOYING") => Self::Deploying,
            Some("DELETING") => Self::Deleting,
            Some("UNKNOWN") => Self::Unknown,
            Some("DETACHING") => Self::Detaching,
            Some("DETACH_FAILED") => Self::DetachFailed,
            Some(other) => Self::Unrecognized(other.to_string()),
        }
    }
}

/// Which Cloud Functions runtime generation a function is deployed on.
///
/// Three documented values, not a boolean: a `bool` has nowhere to put
/// `ENVIRONMENT_UNSPECIFIED` and would report it as one of the generations.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CloudFunctionEnvironment {
    Unspecified,
    Gen1,
    Gen2,
    Unrecognized(String),
}

impl CloudFunctionEnvironment {
    /// Every unit variant, kept in sync by hand for the same reason
    /// [`CloudFunctionState::ALL`] is.
    pub const ALL: [Self; 3] = [Self::Unspecified, Self::Gen1, Self::Gen2];

    /// The API's own string for this environment, round-tripping
    /// [`CloudFunctionEnvironment::from_api`].
    pub fn api_value(&self) -> &str {
        match self {
            Self::Unspecified => "ENVIRONMENT_UNSPECIFIED",
            Self::Gen1 => "GEN_1",
            Self::Gen2 => "GEN_2",
            Self::Unrecognized(raw) => raw,
        }
    }

    /// The generation as the console words it ("1st gen"), not as the API
    /// spells it.
    pub fn label(&self) -> Cow<'_, str> {
        match self {
            Self::Unspecified => Cow::Borrowed("unspecified"),
            Self::Gen1 => Cow::Borrowed("1st gen"),
            Self::Gen2 => Cow::Borrowed("2nd gen"),
            Self::Unrecognized(raw) => readable_api_value(raw),
        }
    }

    /// Reads the API's `environment` string; an absent field is
    /// `ENVIRONMENT_UNSPECIFIED`, for the reason given on
    /// [`CloudFunctionState::from_api`].
    pub fn from_api(raw: Option<&str>) -> Self {
        match raw {
            None | Some("ENVIRONMENT_UNSPECIFIED") => Self::Unspecified,
            Some("GEN_1") => Self::Gen1,
            Some("GEN_2") => Self::Gen2,
            Some(other) => Self::Unrecognized(other.to_string()),
        }
    }
}

/// A Cloud Function's list row.
///
/// `region` is not a field on the API resource: it is the `locations/<region>`
/// segment of the resource path, which is also where `name` comes from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CloudFunction {
    pub name: String,
    pub region: String,
    pub state: CloudFunctionState,
    pub environment: CloudFunctionEnvironment,
    pub url: Option<String>,
    pub update_time: Option<String>,
    pub labels: Vec<ComputeKeyValue>,
}

/// The lifecycle state of one persistent disk.
///
/// Every value the Compute API documents, plus [`Self::Unrecognized`] for
/// anything it adds later. The distinction between the transient states and
/// [`Self::Ready`] is what [`PersistentDisk::is_orphan`] turns on, so these are
/// separate variants rather than a `ready: bool`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum DiskStatus {
    Creating,
    Restoring,
    Failed,
    Ready,
    Deleting,
    Unavailable,
    Unrecognized(String),
}

impl DiskStatus {
    /// Every unit variant, kept in sync by hand for the same reason
    /// [`CloudFunctionState::ALL`] is.
    pub const ALL: [Self; 6] = [
        Self::Creating,
        Self::Restoring,
        Self::Failed,
        Self::Ready,
        Self::Deleting,
        Self::Unavailable,
    ];

    /// The API's own string for this status, round-tripping
    /// [`DiskStatus::from_api`].
    pub fn api_value(&self) -> &str {
        match self {
            Self::Creating => "CREATING",
            Self::Restoring => "RESTORING",
            Self::Failed => "FAILED",
            Self::Ready => "READY",
            Self::Deleting => "DELETING",
            Self::Unavailable => "UNAVAILABLE",
            Self::Unrecognized(raw) => raw,
        }
    }

    /// The one-word rendering for a table cell.
    pub fn label(&self) -> Cow<'_, str> {
        match self {
            Self::Creating => Cow::Borrowed("Creating"),
            Self::Restoring => Cow::Borrowed("Restoring"),
            Self::Failed => Cow::Borrowed("Failed"),
            Self::Ready => Cow::Borrowed("Ready"),
            Self::Deleting => Cow::Borrowed("Deleting"),
            Self::Unavailable => Cow::Borrowed("Unavailable"),
            Self::Unrecognized(raw) => readable_api_value(raw),
        }
    }

    /// Reads the API's `status` string.
    ///
    /// An absent field is [`Self::Unrecognized`] with an empty string rather
    /// than a guess at [`Self::Ready`]: `status` is always present on a real
    /// disk resource, so its absence means the row is not what we think it is,
    /// and [`PersistentDisk::is_orphan`] must not badge a row we could not read.
    pub fn from_api(raw: Option<&str>) -> Self {
        match raw {
            Some("CREATING") => Self::Creating,
            Some("RESTORING") => Self::Restoring,
            Some("FAILED") => Self::Failed,
            Some("READY") => Self::Ready,
            Some("DELETING") => Self::Deleting,
            Some("UNAVAILABLE") => Self::Unavailable,
            Some(other) => Self::Unrecognized(other.to_string()),
            None => Self::Unrecognized(String::new()),
        }
    }
}

/// A persistent disk's list row.
///
/// `location`, `disk_type` and each entry of `users` are the trailing segments
/// of the full resource URLs the API returns, not the URLs themselves — a table
/// cell wants `europe-west1-b`, not 90 characters of `googleapis.com` prefix.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersistentDisk {
    pub name: String,
    /// The zone of a zonal disk or the region of a regional one. One field
    /// rather than two because `compute disks list` returns both kinds in one
    /// array and a regional disk carries `region` where a zonal one carries
    /// `zone` — a `zone`-only field would leave every regional disk located
    /// nowhere, under a column header claiming to hold its zone.
    pub location: String,
    pub status: DiskStatus,
    pub size_gb: u64,
    pub disk_type: String,
    /// The instances this disk is attached to, by short name. Empty is the
    /// honest representation of the API's answer: gcloud omits empty arrays,
    /// so an absent `users` and an empty `users` are the same fact on the wire.
    pub users: Vec<String>,
    pub labels: Vec<ComputeKeyValue>,
}

impl PersistentDisk {
    /// Whether this disk is billed storage that nothing is using.
    ///
    /// Unattached **and** `READY`. The status conjunct is not incidental and
    /// must not be relaxed to "not deleting": a disk in `CREATING` or
    /// `RESTORING` also has no users, but it is mid-operation rather than
    /// waste, and badging it on a cost-cleanup screen tells the operator to
    /// delete something that is being built. `READY` is the state in which
    /// "nothing is using this, and nothing is going to start using it on its
    /// own" is a claim this badge can stand behind.
    ///
    /// Knowingly conservative: an unattached `FAILED` disk *is* waste and this
    /// returns `false` for it. A missing badge costs an operator a scroll; a
    /// wrong badge costs them a disk. The status column still shows `Failed`,
    /// so the operator can reach that conclusion themselves — which is why
    /// status is a column and not only a predicate input.
    pub fn is_orphan(&self) -> bool {
        self.users.is_empty() && self.status == DiskStatus::Ready
    }
}

/// How many subscriptions are attached to a topic, or why that is not known.
///
/// # Why this is not a `usize`
///
/// A topic resource does not carry its subscription count; the count is
/// derived by listing the project's subscriptions and grouping them by the
/// `topic` each one names. That is a *second* list call, and it is the one
/// call in the topics load that an operator may not be allowed to make:
/// `pubsub.topics.list` granted with `pubsub.subscriptions.list` denied is an
/// ordinary read-only IAM shape, not an exotic one.
///
/// A `usize` would have to render that case as `0`, which is a specific claim
/// — "this topic has no subscribers" — and the wrong one. So the absence is a
/// value the table can render as unknown, and the topics list stays useful on
/// its other four columns when the enrichment call fails.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SubscriptionCount {
    Known(usize),
    /// The subscriptions list call did not answer, so no count can be shown.
    Unavailable,
}

impl SubscriptionCount {
    /// The table cell: the number, or a dash that is not a zero.
    pub fn label(self) -> String {
        match self {
            Self::Known(count) => count.to_string(),
            Self::Unavailable => "—".to_string(),
        }
    }

    /// The count, or `None` when the subscriptions list did not answer — the
    /// distinction [`SubscriptionCount::label`] paints as a dash rather than
    /// as a zero.
    pub fn known(self) -> Option<usize> {
        match self {
            Self::Known(count) => Some(count),
            Self::Unavailable => None,
        }
    }
}

/// A Pub/Sub topic's list row.
///
/// # `name` and `full_name`
///
/// `name` is the short id an operator reads and types (`orders`); `full_name`
/// is `projects/p/topics/orders`, which is what a *subscription* names when it
/// points at its topic. Both are carried because the join between the two
/// lists happens on the long form while every display and jump uses the short
/// one, and deriving either from the other at the join site is how a topic
/// whose id happens to be a suffix of another's gets the wrong subscriptions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PubSubTopic {
    pub name: String,
    pub full_name: String,
    pub subscription_count: SubscriptionCount,
    pub message_retention: Option<String>,
    pub kms_key: Option<String>,
    pub labels: Vec<ComputeKeyValue>,
}

/// Where a subscription's messages go: the subscriber pulls them, or Pub/Sub
/// pushes them to an endpoint.
///
/// The API expresses this as the presence or absence of `pushConfig`, which
/// makes "pull" the absence of a field. Naming it a variant instead means the
/// table renders a fact rather than a blank, and the endpoint travels with the
/// case that has one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PubSubDelivery {
    Pull,
    Push { endpoint: String },
}

impl PubSubDelivery {
    /// The delivery column's one word. The endpoint is deliberately not part
    /// of it — see [`PubSubDelivery::endpoint`].
    pub fn label(&self) -> &str {
        match self {
            Self::Pull => "Pull",
            Self::Push { .. } => "Push",
        }
    }

    /// The push endpoint, or `None` for a pull subscription, which has no
    /// endpoint rather than an empty one.
    pub fn endpoint(&self) -> Option<&str> {
        match self {
            Self::Pull => None,
            Self::Push { endpoint } => Some(endpoint),
        }
    }
}

/// The state of one subscription.
///
/// `RESOURCE_ERROR` is the case worth having a name for: the subscription
/// exists but Pub/Sub cannot deliver to it — a deleted push endpoint's service
/// account, most often — and it is invisible in every other column.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PubSubSubscriptionState {
    Unspecified,
    Active,
    ResourceError,
    Unrecognized(String),
}

impl PubSubSubscriptionState {
    /// Every unit variant, for callers that walk the states. Kept in sync by
    /// hand: the fixed length means adding a variant here is a deliberate
    /// edit, not something the compiler will demand.
    pub const ALL: [Self; 3] = [Self::Unspecified, Self::Active, Self::ResourceError];

    /// The API's own string for this state, round-tripping
    /// [`PubSubSubscriptionState::from_api`].
    pub fn api_value(&self) -> &str {
        match self {
            Self::Unspecified => "STATE_UNSPECIFIED",
            Self::Active => "ACTIVE",
            Self::ResourceError => "RESOURCE_ERROR",
            Self::Unrecognized(raw) => raw,
        }
    }

    /// The state column's text, spaced for reading ("Resource error") rather
    /// than the API's `RESOURCE_ERROR`.
    pub fn label(&self) -> Cow<'_, str> {
        match self {
            Self::Unspecified => Cow::Borrowed("Unspecified"),
            Self::Active => Cow::Borrowed("Active"),
            Self::ResourceError => Cow::Borrowed("Resource error"),
            Self::Unrecognized(raw) => readable_api_value(raw),
        }
    }

    /// Reads the API's `state` string.
    ///
    /// An absent field is [`Self::Unspecified`] for the same reason
    /// [`CloudFunctionState::from_api`] treats it that way: `STATE_UNSPECIFIED`
    /// is the proto zero value and proto3 JSON omits zero-valued enums, so a
    /// missing `state` *is* that value.
    pub fn from_api(raw: Option<&str>) -> Self {
        match raw {
            None | Some("STATE_UNSPECIFIED") => Self::Unspecified,
            Some("ACTIVE") => Self::Active,
            Some("RESOURCE_ERROR") => Self::ResourceError,
            Some(other) => Self::Unrecognized(other.to_string()),
        }
    }
}

/// A subscription's dead-letter policy, flattened to the two facts an operator
/// reads: where dead letters go and after how many attempts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PubSubDeadLetterPolicy {
    pub dead_letter_topic: String,
    pub max_delivery_attempts: Option<u32>,
}

/// One subscription.
///
/// Not a `ResourceServiceKind` of its own in this slice: subscriptions are
/// reached by drilling into their topic. They *are* first-class and listable —
/// `gcloud pubsub subscriptions list` returns every subscription in the
/// project, each naming its parent `topic`, which is exactly why the topics
/// load can group them client-side in one call rather than one call per topic.
/// That also means a flat top-level subscriptions table is later an additive
/// change: the row type and the list call are already the shape it would need,
/// and only the variant would be new.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PubSubSubscription {
    pub name: String,
    pub full_name: String,
    /// The full resource name of the parent topic, as the API returns it. A
    /// subscription whose topic has been deleted reads `_deleted-topic_`,
    /// which is a real value rather than an error, and matches no topic row.
    pub topic: String,
    pub delivery: PubSubDelivery,
    pub ack_deadline_seconds: Option<u32>,
    pub message_retention: Option<String>,
    pub dead_letter: Option<PubSubDeadLetterPolicy>,
    pub state: PubSubSubscriptionState,
    pub labels: Vec<ComputeKeyValue>,
}

/// One BigQuery dataset.
///
/// The flat row of the datasets table, and the whole of what this slice
/// models: a dataset's tables and their schemas are separate resources
/// reached by separate calls, and neither is a field here.
///
/// `location` is a plain `String` rather than an enum or an `Option`. A
/// dataset always has one, but the value is an open set — multi-regions
/// (`US`, `EU`), regions (`europe-west1`) and new ones as they ship — so
/// there is nothing to close over. An absent one parses as empty and renders
/// as the table's `-` placeholder, because a row missing its location is
/// still a dataset an operator wants to see listed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BigQueryDataset {
    pub dataset_id: String,
    pub project_id: String,
    pub location: String,
    pub friendly_name: Option<String>,
    pub labels: Vec<ComputeKeyValue>,
}

/// What kind of thing a BigQuery "table" listing row actually is.
///
/// `Other(String)` is here rather than a fallback to [`Self::Table`] because
/// BigQuery's `type` is an open set: snapshots, clones and external tables all
/// arrive through the same listing, and new kinds ship without a client
/// change. Folding an unrecognised value into `Table` would put a word on
/// screen that the API did not say, so an unknown kind is carried verbatim and
/// the reader sees what BigQuery reported.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum BigQueryTableKind {
    Table,
    View,
    MaterializedView,
    External,
    Other(String),
}

impl BigQueryTableKind {
    /// What the `TYPE` column paints.
    ///
    /// The four known kinds get a reader's spelling rather than the API's
    /// (`MATERIALIZED_VIEW` becomes `Materialized view`); `Other` is returned
    /// verbatim, uppercase and underscores included, because a kind this
    /// binary does not know is a word only BigQuery can spell.
    pub fn label(&self) -> &str {
        match self {
            Self::Table => "Table",
            Self::View => "View",
            Self::MaterializedView => "Materialized view",
            Self::External => "External",
            Self::Other(raw) => raw,
        }
    }
}

/// One number a `bq show` reports about a table, or an explicit statement that
/// it does not.
///
/// # Why this is not an `i64`, and not an `Option<i64>` either
///
/// A table's row count, byte size and last-modified time are not on the
/// dataset's listing at all. Each comes from a per-table `bq show`, which is a
/// second call per row, and it is a call that can be refused on its own:
/// `bigquery.tables.list` granted with `bigquery.tables.get` denied is an
/// ordinary read-only IAM shape. The response can also answer without the
/// field, or answer with a value that is not a number.
///
/// A plain `i64` would have to render all of that as `0` — "this table is
/// empty", a claim about the data where the truth is a claim about the call.
/// So the absence is a variant, and a table holding genuinely zero rows
/// (`Known(0)`) is a different value from a table whose `show` never answered
/// ([`Self::Unavailable`]). [`SubscriptionCount`] makes the same distinction
/// for the same reason.
///
/// The payload is an `i64` because BigQuery's `numRows`, `numBytes` and
/// `lastModifiedTime` are int64 fields — sent over the wire as JSON *strings*,
/// and so reparsed rather than deserialized as numbers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TableStat {
    Known(i64),
    /// This table has no usable value for the field. Parsing a `show` yields
    /// it when the field was absent or what came back was not a number; a row
    /// also holds it for a `show` that never answered, because a call that
    /// fails merges nothing and leaves the row as it started. It is not a
    /// zero, and a caller must not render it as one.
    Unavailable,
}

impl TableStat {
    /// The number, or `None` — for a caller that formats the value itself (a
    /// byte size, a timestamp) and needs the absence kept separate from a zero
    /// it would otherwise print.
    pub fn known(self) -> Option<i64> {
        match self {
            Self::Known(value) => Some(value),
            Self::Unavailable => None,
        }
    }
}

/// The three numbers one `bq show` answers with, before they are merged onto
/// the row they describe.
///
/// A struct rather than a tuple so the merge site cannot silently swap two of
/// them: `rows` and `bytes` are both plain counts of the same type, and an
/// order mistake between them would be invisible.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct BigQueryTableStats {
    pub rows: TableStat,
    pub bytes: TableStat,
    pub last_modified: TableStat,
}

/// One table inside a BigQuery dataset, as the dataset's listing reports it.
///
/// The listing fills most of it. `bq ls <dataset>` answers identity, kind,
/// creation time and labels — plus a constant `kind` of `"bigquery#table"`,
/// and a display `id` of the form `project:dataset.table`. That `id` does
/// carry one fact no field below holds, the project, and it is left out on
/// purpose: a table's project is the context's project, so a per-row copy
/// would be a value the row could disagree with the context about. What it
/// does *not* answer is row counts, byte sizes and the last-modified time:
/// those come from a per-table `bq show`, one call per listed row, and they
/// land in the three [`TableStat`] fields below. A row therefore starts
/// [`TableStat::Unavailable`] on all three and stays that way until its own
/// `show` answers — which is what a row holds while the calls are still out,
/// and what it keeps if its `show` is refused.
///
/// `created` is `Option<i64>` for two reasons at once. BigQuery sends
/// `creationTime` as a *string* of epoch milliseconds (`"1495683814344"`), so
/// it has to be reparsed rather than deserialized as a number; and a row whose
/// timestamp is unusable is still a table an operator wants listed. Absent,
/// empty, non-numeric and out-of-range-for-`i64` all resolve to `None` and a
/// rendered placeholder rather than failing the whole load.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BigQueryTable {
    pub dataset_id: String,
    pub table_id: String,
    pub kind: BigQueryTableKind,
    pub created: Option<i64>,
    pub labels: Vec<ComputeKeyValue>,
    /// Rows, bytes, and the epoch milliseconds of the last modification —
    /// each filled by this table's own `bq show`. See [`TableStat`] for why an
    /// unfilled one is not a zero.
    pub rows: TableStat,
    pub bytes: TableStat,
    pub last_modified: TableStat,
}

/// One field of a BigQuery table's schema, flattened out of the tree the API
/// answers with.
///
/// # Why this is flat when the wire shape is a tree
///
/// `bq show` reports a schema as nested `fields` arrays: a `RECORD` carries
/// its children inside itself, arbitrarily deep. The drill-down that displays
/// it is a table of rows, so the tree is walked once at the parse and every
/// field — parents included — becomes a row. A record is *not* replaced by its
/// children: `event_params` being `REPEATED` is a fact about the record and
/// about nothing below it, so dropping the parent row would drop the fact.
///
/// `path` is what makes a row identifiable once the nesting is gone. It is the
/// dotted ancestry (`event_params.value.string_value`), not the leaf name,
/// because leaf names repeat freely across records in one table — and the
/// dotted spelling is the one BigQuery itself accepts in a query, so the value
/// on screen is the value an operator can paste.
///
/// `field_type` and `mode` are plain `String`s rather than enums for the
/// reason [`BigQueryTableKind`] carries `Other`: both are open sets that grow
/// server-side (`BIGNUMBER`, `INTERVAL`, `RANGE`), and naming an unrecognised
/// type after a known one would put a word on screen the API never said. There
/// is nothing here to close over, so nothing is closed.
///
/// `description` is an `Option` because BigQuery distinguishes a field with no
/// description from one whose description is the empty string. Folding the two
/// together at the parse would destroy the distinction before any caller could
/// decide it does not matter.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BigQuerySchemaField {
    /// The dotted ancestry ending in this field's own name. A top-level field
    /// is its bare name, with no leading separator.
    pub path: String,
    pub field_type: String,
    /// `NULLABLE`, `REQUIRED` or `REPEATED`. A field omitting the key on the
    /// wire arrives here as `NULLABLE`, which is the API's documented default
    /// — an empty string would paint a blank cell that reads as a rendering
    /// fault.
    pub mode: String,
    pub description: Option<String>,
}

/// The lifecycle state of one secret version.
///
/// Metadata, and the only per-version fact this binary ever holds. There is
/// deliberately no variant, field, or sibling type that could carry a payload —
/// see the module note on [`SecretVersion`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SecretVersionState {
    Enabled,
    Disabled,
    Destroyed,
    Unknown(String),
}

impl SecretVersionState {
    /// The version's state as the detail pane shows it. An unrecognised state
    /// renders as the raw string rather than as a guess at which of the three
    /// known states it resembles.
    pub fn label(&self) -> Cow<'_, str> {
        match self {
            Self::Enabled => Cow::Borrowed("Enabled"),
            Self::Disabled => Cow::Borrowed("Disabled"),
            Self::Destroyed => Cow::Borrowed("Destroyed"),
            Self::Unknown(raw) => readable_api_value(raw),
        }
    }
}

/// One version of a secret: which version it is, what state it is in, and when
/// it was created.
///
/// **This type has no payload field, and must never acquire one.** nuvem browses
/// Secret Manager metadata and structurally never issues the call that returns
/// payload material, so a payload cannot render — not because it is redacted
/// downstream, but because that call does not exist in this binary. Adding a
/// field here that could hold secret material would defeat it, and
/// `tests/secrets_no_payload_access_test.rs` is what stops that.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecretVersion {
    pub name: String,
    pub state: SecretVersionState,
    pub create_time: Option<String>,
}

/// A secret's list row: its identity and the metadata worth scanning a table
/// for. No version count — the Secret resource does not carry one, and
/// synthesising it would cost one extra call per row on every list load.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Secret {
    pub name: String,
    pub create_time: Option<String>,
    pub labels: Vec<ComputeKeyValue>,
    /// Pre-rendered replication policy, e.g. `Automatic` or
    /// `User-managed (europe-west1, europe-west4)`.
    pub replication: String,
}

/// A secret's detail pane. `versions` is filled by a separate, on-demand fetch
/// once the operator opens the Versions tab, which is also where the version
/// count comes from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecretDetail {
    pub name: String,
    pub create_time: Option<String>,
    pub labels: Vec<ComputeKeyValue>,
    pub annotations: Vec<ComputeKeyValue>,
    pub replication: String,
    pub rotation: Option<String>,
    pub expire_time: Option<String>,
    pub versions: Vec<SecretVersion>,
    pub raw_json: String,
}

/// A service account's list row.
///
/// The email is the identity: it is unique within the project, it is what every
/// other resource names a service account by (`CloudRunServiceDetail`'s
/// `service_account`, a policy binding's `serviceAccount:` member), and it is
/// what an operator types. `unique_id` is carried beside it because it is the
/// only handle that survives the account being deleted and recreated under the
/// same email — two different accounts, one string — which is exactly the
/// confusion an audit needs to be able to resolve.
///
/// `name` (`projects/p/serviceAccounts/email`) and `oauth2ClientId` are
/// deliberately absent: the first is the email with a prefix this table already
/// knows, and the second is `uniqueId` again under a second name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServiceAccount {
    pub email: String,
    pub display_name: Option<String>,
    /// Whether the account is disabled. A disabled account still holds every
    /// role it was granted, so this is not derivable from the policy and is
    /// the one column that distinguishes "cannot authenticate" from "can".
    pub disabled: bool,
    pub unique_id: String,
}

/// What kind of principal an IAM policy member names.
///
/// # Why this is not `String::split(':')`
///
/// Three of the shapes the API returns are not `kind:id`, and each one is
/// mis-read by a naive split:
///
/// - `allUsers` and `allAuthenticatedUsers` carry **no colon at all**. A split
///   reads the whole string as the kind and leaves the identity empty — which
///   would render the two most security-relevant rows in a project policy as
///   blanks.
/// - `deleted:` is a prefix layered *on top of* another kind
///   (`deleted:serviceAccount:x@y?uid=1`), so the first segment is not the
///   principal type.
/// - `principal://…` and `principalSet://…` (workforce and workload identity
///   federation) contain further colons inside the identifier.
///
/// [`Self::Unrecognized`] carries the verbatim first segment rather than
/// collapsing to a catch-all label, for the reason every other `from_api` in
/// this module does: a member type Google adds later must still be readable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum IamMemberKind {
    User,
    ServiceAccount,
    Group,
    Domain,
    /// `allUsers` — anyone on the internet, with or without a Google account.
    AllUsers,
    /// `allAuthenticatedUsers` — any authenticated Google account, including
    /// accounts in no way connected to this organisation.
    AllAuthenticatedUsers,
    /// A single federated identity (`principal://…`).
    Principal,
    /// A set of federated identities (`principalSet://…`).
    PrincipalSet,
    Unrecognized(String),
}

impl IamMemberKind {
    /// Every unit variant, kept in sync by hand for the same reason
    /// [`CloudFunctionState::ALL`] is.
    pub const ALL: [Self; 8] = [
        Self::User,
        Self::ServiceAccount,
        Self::Group,
        Self::Domain,
        Self::AllUsers,
        Self::AllAuthenticatedUsers,
        Self::Principal,
        Self::PrincipalSet,
    ];

    /// The prefix as the API writes it, which is also the whole member string
    /// for the two prefixless kinds.
    pub fn api_value(&self) -> &str {
        match self {
            Self::User => "user",
            Self::ServiceAccount => "serviceAccount",
            Self::Group => "group",
            Self::Domain => "domain",
            Self::AllUsers => "allUsers",
            Self::AllAuthenticatedUsers => "allAuthenticatedUsers",
            Self::Principal => "principal",
            Self::PrincipalSet => "principalSet",
            Self::Unrecognized(raw) => raw,
        }
    }

    /// The table cell.
    pub fn label(&self) -> &str {
        match self {
            Self::User => "User",
            Self::ServiceAccount => "Service account",
            Self::Group => "Group",
            Self::Domain => "Domain",
            Self::AllUsers => "All users",
            Self::AllAuthenticatedUsers => "All authenticated users",
            Self::Principal => "Principal",
            Self::PrincipalSet => "Principal set",
            Self::Unrecognized(raw) => raw,
        }
    }

    /// Whether this kind names an unbounded population rather than an identity
    /// this project controls.
    ///
    /// The predicate behind [`IamPolicyMember::is_public`] — see there for why
    /// `allAuthenticatedUsers` counts.
    pub fn is_public(&self) -> bool {
        matches!(self, Self::AllUsers | Self::AllAuthenticatedUsers)
    }

    /// Reads the type prefix of a member string.
    ///
    /// Takes the whole member because two kinds *are* the whole member and one
    /// is reached only after stripping `deleted:` — see the type docs.
    pub fn from_member(member: &str) -> Self {
        match member {
            "allUsers" => return Self::AllUsers,
            "allAuthenticatedUsers" => return Self::AllAuthenticatedUsers,
            _ => {}
        }
        match member.split_once(':') {
            Some(("user", _)) => Self::User,
            Some(("serviceAccount", _)) => Self::ServiceAccount,
            Some(("group", _)) => Self::Group,
            Some(("domain", _)) => Self::Domain,
            Some(("principal", _)) => Self::Principal,
            Some(("principalSet", _)) => Self::PrincipalSet,
            Some((other, _)) => Self::Unrecognized(other.to_string()),
            // No colon and not one of the two prefixless kinds: the whole
            // string is all we have to name it by.
            None => Self::Unrecognized(member.to_string()),
        }
    }
}

/// One (role, member) pair from a project's IAM policy.
///
/// # Why one row per pair and not one row per binding
///
/// The wire shape is one binding per role carrying a `members` array, and 74
/// bindings can hold 188 members. Rendering that shape directly puts a list of
/// principals inside a single cell, and the two questions a policy browser
/// exists to answer both become unanswerable by the table's own filter:
/// "who holds this role" is readable but not selectable, and "what can this
/// principal do" — the question an incident starts with — cannot be filtered
/// for at all, because the email is one of ten values in a cell rather than a
/// row of its own.
///
/// Flattening at parse time makes both a `/` away. It is also the shape
/// gcloud's own `--flatten="bindings[].members"` produces, so it is the
/// projection an operator already reaches for by hand.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IamPolicyMember {
    /// The role, verbatim.
    ///
    /// Not shortened to its trailing segment the way resource URLs elsewhere in
    /// this module are: a predefined role reads `roles/storage.admin` and a
    /// custom one reads `projects/p/roles/storage.admin`, and reducing both to
    /// `storage.admin` would render two different grants — one Google defines
    /// and one this project defines — as the same string.
    pub role: String,
    pub kind: IamMemberKind,
    /// The principal, with its type prefix and any `deleted:` marker removed;
    /// for [`IamMemberKind::AllUsers`] and
    /// [`IamMemberKind::AllAuthenticatedUsers`], the whole member string, since
    /// those name a population rather than carry an identity.
    pub principal: String,
    /// Whether the binding names a principal that has since been deleted
    /// (`deleted:` prefix).
    ///
    /// Its own field rather than an [`IamMemberKind`] variant because it is
    /// orthogonal to the kind — a deleted user and a deleted service account
    /// are different rows — and because a grant to a deleted principal is
    /// stale policy an audit wants to find, which means it has to be a fact a
    /// column can show rather than one folded into another column's value.
    pub deleted: bool,
    /// The title of the binding's IAM condition, when it is conditional.
    ///
    /// Carried because a conditional grant is a *narrower* grant, and a row
    /// that omits the condition overstates what the principal can do. The
    /// title rather than the CEL expression: the expression is a paragraph and
    /// this is a table cell.
    pub condition: Option<String>,
    /// The member exactly as the API returned it, so the row can be searched
    /// and read in the form the policy actually holds.
    pub raw_member: String,
}

impl IamPolicyMember {
    /// Reads one entry of a binding's `members` array into a row.
    ///
    /// Here rather than in the gcloud adapter because it is a total function of
    /// three wire strings with no I/O behind it, and because the three
    /// not-`kind:id` shapes documented on [`IamMemberKind`] and the `deleted:`
    /// layer are one piece of knowledge that should not be split across two
    /// crates.
    pub fn from_wire(role: &str, raw_member: &str, condition: Option<&str>) -> Self {
        // `deleted:` wraps another whole member, so it comes off before the
        // kind is read; anything after this point is an ordinary member string.
        let (deleted, undeleted) = match raw_member.strip_prefix("deleted:") {
            Some(rest) => (true, rest),
            None => (false, raw_member),
        };
        let kind = IamMemberKind::from_member(undeleted);
        let principal = if kind.is_public() {
            // `allUsers` names a population, not an identity: there is no
            // suffix to take, and the whole token *is* what the cell should
            // read.
            undeleted.to_string()
        } else {
            // Splits once, so the colons inside a `principal://` URL stay in
            // the identifier. A member with no colon at all keeps its whole
            // text rather than becoming an empty cell.
            undeleted
                .split_once(':')
                .map(|(_, identity)| identity.to_string())
                .unwrap_or_else(|| undeleted.to_string())
        };
        Self {
            role: role.to_string(),
            kind,
            principal,
            deleted,
            // An empty title is no title: a conditional binding with a blank
            // title would otherwise render an empty cell that reads exactly
            // like an unconditional grant while claiming to be one.
            condition: condition
                .map(str::trim)
                .filter(|title| !title.is_empty())
                .map(str::to_string),
            raw_member: raw_member.to_string(),
        }
    }

    /// Whether this grant is held by an unbounded population.
    ///
    /// `allAuthenticatedUsers` counts alongside `allUsers`: it excludes
    /// anonymous callers, but "any Google account" is every Gmail address in
    /// the world, so treating it as bounded because it says *authenticated*
    /// is the mistake this predicate exists to prevent.
    ///
    /// Deliberately says nothing about how dangerous the *role* is — a public
    /// `roles/storage.objectViewer` may be exactly what a project intends.
    /// This reports the population, and the role stays a column so the
    /// operator draws the conclusion.
    pub fn is_public(&self) -> bool {
        self.kind.is_public()
    }
}

/// How a VPC network gets its subnets — the `MODE` column.
///
/// Derived from `autoCreateSubnetworks`, never from gcloud's synthesised
/// `x_gcloud_subnet_mode`. That key carries the very string this table paints,
/// which is what makes it tempting; the `x_` prefix is gcloud declaring it a
/// presentation field of its own rather than something the API promises, and
/// nothing commits it to keeping one.
///
/// [`Legacy`](Self::Legacy) is the arm that has never been seen from the real
/// API here: a legacy network omits `autoCreateSubnetworks` entirely, and the
/// account this repo captures from can reach none. It is modelled from the
/// published `compute/v1/networks` reference rather than folded into
/// [`Custom`](Self::Custom), because the two differ in the one thing an
/// operator would do next — a custom network has subnets to go and look at,
/// and a legacy network cannot have any at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum VpcSubnetMode {
    Auto,
    Custom,
    Legacy,
}

impl VpcSubnetMode {
    /// The cell, spelled as gcloud spells it. Upper case because this is a
    /// mode name an operator matches against `gcloud compute networks list`
    /// output, not prose.
    pub fn label(self) -> &'static str {
        match self {
            Self::Auto => "AUTO",
            Self::Custom => "CUSTOM",
            Self::Legacy => "LEGACY",
        }
    }

    /// Reads the API's `autoCreateSubnetworks`.
    ///
    /// An absent field is [`Legacy`](Self::Legacy) rather than the `false` a
    /// `#[serde(default)] bool` would have produced. That default is the
    /// defect this constructor exists to prevent: it would paint a legacy
    /// network `CUSTOM`, sending an operator to look for subnets that cannot
    /// exist.
    pub fn from_api(auto_create_subnetworks: Option<bool>) -> Self {
        match auto_create_subnetworks {
            Some(true) => Self::Auto,
            Some(false) => Self::Custom,
            None => Self::Legacy,
        }
    }
}

/// A network's dynamic routing mode — the `ROUTING` column.
///
/// Derived from `routingConfig.routingMode`, never from gcloud's synthesised
/// `x_gcloud_bgp_routing_mode`, for the reason recorded on [`VpcSubnetMode`].
///
/// [`Unspecified`](Self::Unspecified) is an absent `routingConfig`, and it is
/// deliberately not `REGIONAL`. Regional is the API's *default* for a network
/// created without one, but a default is a claim about creation and this cell
/// is a claim about what the API reported — the [`PubSubSubscriptionState`]
/// shape, for the same reason.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum VpcRoutingMode {
    Unspecified,
    Regional,
    Global,
    Unrecognized(String),
}

impl VpcRoutingMode {
    /// The cell. An unspecified mode paints as the empty string so the column
    /// renders its dash rather than a word the API never sent.
    pub fn label(&self) -> Cow<'_, str> {
        match self {
            Self::Unspecified => Cow::Borrowed(""),
            Self::Regional => Cow::Borrowed("REGIONAL"),
            Self::Global => Cow::Borrowed("GLOBAL"),
            // This column spells the API's constants as the API does, so an
            // unrecognised one is spelled the same way.
            Self::Unrecognized(raw) => Cow::Borrowed(raw),
        }
    }

    /// Reads the API's `routingConfig.routingMode`.
    pub fn from_api(raw: Option<&str>) -> Self {
        match raw {
            None | Some("") => Self::Unspecified,
            Some("REGIONAL") => Self::Regional,
            Some("GLOBAL") => Self::Global,
            Some(other) => Self::Unrecognized(other.to_string()),
        }
    }
}

/// One VPC network, flattened to the four facts the table paints.
///
/// A network has no gateway: `gatewayAddress` is a *subnet* field, and the capture this was
/// modelled on carries none at the network level. The parenthetical was wrong
/// and the columns here are what the resource actually has.
///
/// `subnet_count` rather than the `subnetworks` array itself: the captured
/// `default` network carries 43 self-link URLs, and 43 URLs is not a cell. The
/// subnets themselves are a table of their own (slice 2), which is
/// where an operator goes to read them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VpcNetwork {
    pub name: String,
    pub subnet_mode: VpcSubnetMode,
    pub routing_mode: VpcRoutingMode,
    pub subnet_count: usize,
    /// The network's description, which is free text an operator writes and
    /// the only field on this resource that says *why* the network exists. No
    /// column of its own — it is unbounded prose and the four columns that do
    /// exist are what a network is identified by — but it is searchable and
    /// the preview pane renders it.
    pub description: Option<String>,
}

/// One VPC subnet, flattened to the six facts the table paints.
///
/// # Regional, unlike its network
///
/// A subnet's identity is (name, region) and not the name alone. That is not
/// an edge case to guard against: an auto-mode network gets one subnet per
/// region, **all called `default`**, so the captured project returns 43 rows
/// sharing one name. A row keyed on the name collapses all 43 into one.
///
/// # Two fields that arrive as URLs
///
/// `region` and `network` are stored here already reduced to their last path
/// segment. The API sends both as full self-links
/// (`https://.../projects/p/regions/us-central1`), which is not a table cell
/// at any terminal width; the reduction happens once, in the parser, so no
/// caller can forget it and no two cells can disagree about what a network is
/// called.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VpcSubnet {
    pub name: String,
    /// The region id, not the self-link.
    pub region: String,
    /// The parent network's id, not the self-link.
    pub network: String,
    /// The primary IPv4 range, as `10.128.0.0/20`. The API's own spelling;
    /// nothing here reformats a CIDR.
    pub ip_cidr_range: String,
    /// The subnet's gateway address, absent where the API reported none.
    pub gateway_address: Option<String>,
    /// Whether instances without external IPs can reach Google APIs.
    ///
    /// A plain `bool` rather than an `Option`, unlike a network's subnet mode:
    /// the API documents an absent flag as off, and this is a setting rather
    /// than a discriminator between two shapes of resource. Painting `Off` for
    /// an absent field is what lets an operator scan the column; a dash would
    /// make them wonder which rows were unreported.
    pub private_google_access: bool,
}

/// What a firewall rule does to matching traffic — the `ACTION` column.
///
/// # One field, because the API gives one answer
///
/// A rule carries `allowed` **or** `denied`, never both and never neither.
/// Modelling that as two `Option<Vec<_>>` fields would let the type represent
/// four states where the API sends two, and every reader would have to decide
/// afresh what "both" and "neither" mean. One field that holds one answer
/// cannot be read two ways.
///
/// # Why [`Unspecified`](Self::Unspecified) exists rather than a default
///
/// The obvious default is `Allow` — it is overwhelmingly the common case — and
/// it is the one that must not be taken. This is the table whose entire job is
/// to say what reaches a workload; a rule the parser cannot read, painted
/// `ALLOW`, tells an operator that traffic is permitted on the strength of a
/// document that said no such thing. A dash says "this rule could not be
/// read", which is true and actionable. So both the impossible shapes — no key
/// and both keys — land here rather than being guessed at.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FirewallAction {
    Allow,
    Deny,
    Unspecified,
}

/// How a firewall rule selects the instances it applies to — the `TARGETS`
/// column, and one of the three inputs to "does this rule target this
/// instance".
///
/// One field holding one answer, for the reason [`FirewallAction`] is one:
/// `targetTags` and `targetServiceAccounts` are alternative scopings that
/// Google's `compute/v1/firewalls` reference documents as mutually exclusive,
/// so two free vectors would represent four states where the API sends three,
/// and every reader would have to decide again what "both" means.
///
/// [`AllInstances`](Self::AllInstances) is the one worth naming. A rule with
/// no selector applies to **every** instance in its network — the widest
/// possible scope, and the exact opposite of what an empty list looks like.
/// A matcher that read it as "targets nothing" would silently drop every
/// default rule in the project, DENY rules included.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum FirewallTargets {
    /// Neither selector present: every instance in the rule's network.
    AllInstances,
    /// Scoped to instances carrying any of these network tags.
    Tags(Vec<String>),
    /// Scoped to instances running as any of these service accounts, by email.
    ServiceAccounts(Vec<String>),
    /// Both selectors present at once — a document the reference says the API
    /// does not send.
    ///
    /// Named rather than resolved, exactly as [`FirewallAction::Unspecified`]
    /// names its impossible pair: choosing one of the two here would be a
    /// guess about a rule whose scope is genuinely unclear, and a wrong guess
    /// is a rule missing from a list an operator is reading for safety.
    Both {
        tags: Vec<String>,
        service_accounts: Vec<String>,
    },
}

impl FirewallTargets {
    /// Reads the two lists the API may carry.
    ///
    /// Absent and empty are the same answer, and that is an assumption rather
    /// than an observation: no captured rule in
    /// `tests/fixtures/vpc_firewall_rules.json` carries an empty array — every
    /// one either omits the key or lists at least one entry — so nothing here
    /// distinguishes `"targetTags": []` from no `targetTags` at all. The
    /// reference defines "not specified" as applying to all instances, and
    /// this is the single line that would change if an empty array ever turned
    /// out to mean something else.
    pub fn from_api(tags: Vec<String>, service_accounts: Vec<String>) -> Self {
        match (tags.is_empty(), service_accounts.is_empty()) {
            (true, true) => Self::AllInstances,
            (false, true) => Self::Tags(tags),
            (true, false) => Self::ServiceAccounts(service_accounts),
            (false, false) => Self::Both {
                tags,
                service_accounts,
            },
        }
    }

    /// Every selector value, whichever kind scopes the rule — what the
    /// `TARGETS` cell joins and what a filter searches.
    ///
    /// Deliberately loses which kind they are: a display cell and a substring
    /// filter do not care, and the matcher never reads this because for it the
    /// distinction is the whole question.
    pub fn values(&self) -> Vec<&str> {
        match self {
            Self::AllInstances => Vec::new(),
            Self::Tags(values) | Self::ServiceAccounts(values) => {
                values.iter().map(String::as_str).collect()
            }
            Self::Both {
                tags,
                service_accounts,
            } => tags
                .iter()
                .chain(service_accounts)
                .map(String::as_str)
                .collect(),
        }
    }
}

/// The three facts about an instance a firewall rule can select on.
///
/// Its own type rather than a borrowed [`ComputeInstanceDetail`], because the
/// matcher is a function of exactly these three values and nothing else. A
/// signature taking the whole detail would let a future arm reach for the
/// instance's zone or status, and neither has any bearing on whether a rule
/// applies — that is the kind of quiet widening a security-relevant predicate
/// must not be able to grow.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FirewallInstanceScope {
    /// The network the instance's interface is attached to, shortened to its
    /// id — the same spelling `FirewallRule::network` carries.
    pub network: String,
    pub tags: Vec<String>,
    /// Service account emails.
    pub service_accounts: Vec<String>,
}

impl FirewallInstanceScope {
    /// The scope of an instance's *primary* network interface, or `None` for an
    /// instance that has no network to be governed on.
    ///
    /// `first()` for the same reason `{network}` and `{internal_ip}` use it:
    /// an instance with several interfaces sits on several networks, and a rule
    /// list built from a different interface than the one the rest of the pane
    /// describes would be about a network the operator is not looking at.
    /// Naming the others needs a per-interface vocabulary nothing here has.
    ///
    /// `None` rather than a scope on an empty network id. An empty network
    /// would match any rule whose own network was empty, which is a match
    /// nobody asked for; refusing to build a scope means the jump is not
    /// offered at all.
    pub fn from_detail(detail: &ComputeInstanceDetail) -> Option<Self> {
        let network = detail
            .network_interfaces
            .first()?
            .network
            .as_ref()
            .filter(|network| !network.is_empty())?;
        Some(Self {
            network: network.clone(),
            tags: detail.tags.clone(),
            service_accounts: detail.service_accounts.clone(),
        })
    }
}

/// Whether `rule` applies to the instance described by `scope`.
///
/// # The network gate comes first and is absolute
///
/// A firewall rule lives in exactly one network and cannot reach an instance
/// in another, whatever its target selector says. So a rule on a different
/// network is never a match, including one with no selector at all — that
/// rule applies to every instance in *its* network, not to every instance.
///
/// # What "no selector" means
///
/// [`FirewallTargets::AllInstances`] matches every instance in the network,
/// which is the widest scope and the one most easily read backwards. It is
/// also the common case: six of the seven rules in the captured fixture have
/// no selector, so a matcher that treated an empty selector as "targets
/// nothing" would return almost nothing and look plausible doing it.
///
/// # What this does NOT answer
///
/// Whether traffic actually reaches the instance. That depends on priority
/// ordering between overlapping rules, on the direction, on ports and
/// protocols, and on whether the rule is disabled — none of which this reads.
/// This answers only "is this instance in this rule's target set", which is
/// the question the jump asks. A caller rendering the result must keep
/// showing action, direction and disabled state, because a rule that targets
/// an instance and is switched off targets it and does nothing.
pub fn rule_targets_instance(rule: &FirewallRule, scope: &FirewallInstanceScope) -> bool {
    if rule.network != scope.network {
        return false;
    }
    match &rule.targets {
        FirewallTargets::AllInstances => true,
        FirewallTargets::Tags(tags) => any_matches(tags, &scope.tags),
        FirewallTargets::ServiceAccounts(accounts) => {
            any_matches(accounts, &scope.service_accounts)
        }
        // The impossible document. Either selector matching is enough, which
        // is the reading that cannot drop a rule from a safety listing — the
        // failure mode here is a rule shown that does not apply, not a DENY
        // silently missing. A caller that needs certainty about such a rule
        // has to read the rule itself, which is what the jump lands them on.
        FirewallTargets::Both {
            tags,
            service_accounts,
        } => {
            any_matches(tags, &scope.tags) || any_matches(service_accounts, &scope.service_accounts)
        }
    }
}

/// Whether any selector value is one the instance carries.
///
/// Exact, case-sensitive comparison. Network tags are lowercase by GCP's own
/// validation and service account emails are compared as the API spells them;
/// nothing here normalises, because a matcher that folded case would claim a
/// rule applies to an instance whose tag merely resembles the selector.
fn any_matches(selectors: &[String], carried: &[String]) -> bool {
    selectors
        .iter()
        .any(|selector| carried.iter().any(|value| value == selector))
}

impl FirewallAction {
    /// The cell, upper case as gcloud prints it. `Unspecified` contributes no
    /// word, so the column paints its dash.
    pub fn label(self) -> &'static str {
        match self {
            Self::Allow => "ALLOW",
            Self::Deny => "DENY",
            Self::Unspecified => "",
        }
    }

    /// Reads the presence of the two keys.
    ///
    /// Exactly one present is the answer; anything else is
    /// [`Unspecified`](Self::Unspecified) — see the note on the type for why
    /// neither impossible case is guessed at.
    pub fn from_api(has_allowed: bool, has_denied: bool) -> Self {
        match (has_allowed, has_denied) {
            (true, false) => Self::Allow,
            (false, true) => Self::Deny,
            _ => Self::Unspecified,
        }
    }
}

/// Which way a firewall rule looks — the `DIRECTION` column.
///
/// It also decides which range list the `SOURCE/DEST` column paints: an
/// INGRESS rule carries `sourceRanges`, an EGRESS rule `destinationRanges`,
/// and neither carries the other.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum FirewallDirection {
    Ingress,
    Egress,
    Unspecified,
    Unrecognized(String),
}

impl FirewallDirection {
    /// The `DIRECTION` column's text, upper-case as GCP writes it. Empty for
    /// `Unspecified`, so a rule whose direction the API omitted paints a blank
    /// rather than a word that would claim something.
    pub fn label(&self) -> Cow<'_, str> {
        match self {
            Self::Ingress => Cow::Borrowed("INGRESS"),
            Self::Egress => Cow::Borrowed("EGRESS"),
            Self::Unspecified => Cow::Borrowed(""),
            // This column spells the API's constants as the API does, so an
            // unrecognised one is spelled the same way.
            Self::Unrecognized(raw) => Cow::Borrowed(raw),
        }
    }

    pub fn from_api(raw: Option<&str>) -> Self {
        match raw {
            None | Some("") => Self::Unspecified,
            Some("INGRESS") => Self::Ingress,
            Some("EGRESS") => Self::Egress,
            Some(other) => Self::Unrecognized(other.to_string()),
        }
    }
}

/// One VPC firewall rule, flattened to the facts the table paints.
///
/// `disabled` is the field this row exists to keep visible. A disabled DENY
/// rule reads exactly like a live DENY rule in every other column — same name,
/// same direction, same priority, same ranges — while traffic an operator
/// believes is blocked is flowing. The projection puts it on the identity
/// column as a badge rather than in a column of its own, so no terminal width
/// can drop it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FirewallRule {
    pub name: String,
    /// The parent network's id, not the self-link.
    pub network: String,
    pub direction: FirewallDirection,
    /// Lower numbers win. `None` where the API reported none, which paints a
    /// dash rather than the `1000` default — a default is a claim about the
    /// rule's creation, not about what this listing said.
    pub priority: Option<u32>,
    pub action: FirewallAction,
    /// Which instances in the network the rule applies to.
    pub targets: FirewallTargets,
    /// `sourceRanges` for an INGRESS rule, `destinationRanges` for an EGRESS
    /// one — the rule only ever carries the one matching its direction.
    pub source_or_dest: Vec<String>,
    pub disabled: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cloud_sql_status_roundtrips_and_availability_labels() {
        let s = CloudSqlStatus::Maintenance;
        let json = serde_json::to_string(&s).unwrap();
        assert_eq!(serde_json::from_str::<CloudSqlStatus>(&json).unwrap(), s);
        assert_eq!(CloudSqlAvailability::Regional.label(), "HA (regional)");
        assert_eq!(CloudSqlAvailability::Zonal.label(), "Zonal");
    }

    /// The orphan badge is a cost-cleanup claim, so it is pinned over the
    /// whole `users` x `status` cross-product rather than on the happy path.
    /// The `CREATING`/`RESTORING` rows are the ones that matter: those disks
    /// have no users *yet*, and badging them tells an operator to delete
    /// something that is being built.
    #[test]
    fn only_an_unattached_ready_disk_is_an_orphan() {
        let cases = [
            (vec![], DiskStatus::Ready, true),
            (vec![], DiskStatus::Creating, false),
            (vec![], DiskStatus::Restoring, false),
            (vec![], DiskStatus::Failed, false),
            (vec![], DiskStatus::Deleting, false),
            (vec![], DiskStatus::Unavailable, false),
            (vec![], DiskStatus::Unrecognized("WEIRD".to_string()), false),
            (vec!["web-01".to_string()], DiskStatus::Ready, false),
            (vec!["web-01".to_string()], DiskStatus::Creating, false),
        ];
        for (users, status, expected) in cases {
            let disk = PersistentDisk {
                name: "d".to_string(),
                location: "europe-west1-b".to_string(),
                status: status.clone(),
                size_gb: 20,
                disk_type: "pd-ssd".to_string(),
                users: users.clone(),
                labels: Vec::new(),
            };
            assert_eq!(
                disk.is_orphan(),
                expected,
                "users={users:?} status={status:?}"
            );
        }
    }

    /// The three member shapes that are not `kind:id`, which is the whole
    /// reason `IamMemberKind::from_member` exists rather than a `split(':')`
    /// at the call site. `allUsers` is the row that matters most: read by a
    /// split it becomes a kind with an empty identity, so the one binding an
    /// audit is looking for renders as a blank.
    #[test]
    fn the_prefixless_compound_and_url_member_shapes_all_read_back() {
        let cases = [
            ("user:alice@example.com", IamMemberKind::User),
            (
                "serviceAccount:sa@p.iam.gserviceaccount.com",
                IamMemberKind::ServiceAccount,
            ),
            ("group:team@example.com", IamMemberKind::Group),
            ("domain:example.com", IamMemberKind::Domain),
            ("allUsers", IamMemberKind::AllUsers),
            (
                "allAuthenticatedUsers",
                IamMemberKind::AllAuthenticatedUsers,
            ),
            (
                "principal://iam.googleapis.com/locations/global/workforcePools/p/subject/s",
                IamMemberKind::Principal,
            ),
            (
                "principalSet://iam.googleapis.com/projects/1/locations/global/workloadIdentityPools/p/*",
                IamMemberKind::PrincipalSet,
            ),
            (
                "futureKind:whatever",
                IamMemberKind::Unrecognized("futureKind".to_string()),
            ),
            (
                "bareword",
                IamMemberKind::Unrecognized("bareword".to_string()),
            ),
        ];
        for (member, expected) in cases {
            assert_eq!(
                IamMemberKind::from_member(member),
                expected,
                "member {member:?}"
            );
        }
    }

    /// `deleted:` is a prefix layered on top of another kind, so the kind of a
    /// deleted principal is the *second* segment. Read as the first, every
    /// deleted grant in a policy collapses into one indistinguishable bucket.
    #[test]
    fn a_deleted_prefix_is_not_itself_a_member_kind() {
        assert_eq!(
            IamMemberKind::from_member("deleted:serviceAccount:sa@p.iam.gserviceaccount.com?uid=1"),
            IamMemberKind::Unrecognized("deleted".to_string()),
            "from_member reads the outer prefix; stripping `deleted:` is the parser's job"
        );
    }

    /// The public predicate is a claim on a security screen, so it is pinned
    /// over every kind rather than on the two that answer true. The
    /// `allAuthenticatedUsers` row is the one worth the test: it excludes
    /// anonymous callers, which is what makes reading it as bounded tempting
    /// and wrong.
    #[test]
    fn only_the_two_unbounded_populations_are_public() {
        for kind in IamMemberKind::ALL {
            let expected = matches!(
                kind,
                IamMemberKind::AllUsers | IamMemberKind::AllAuthenticatedUsers
            );
            assert_eq!(kind.is_public(), expected, "{kind:?}");
        }
        assert!(!IamMemberKind::Unrecognized("principal".to_string()).is_public());
    }

    /// Every kind renders a distinct, non-empty cell. Two kinds sharing a
    /// label would make a policy table claim a user and a group were the same
    /// principal type.
    #[test]
    fn every_member_kind_has_its_own_label_and_api_value() {
        let mut labels: Vec<String> = Vec::new();
        let mut api_values: Vec<String> = Vec::new();
        for kind in IamMemberKind::ALL {
            let (label, api_value) = (kind.label().to_string(), kind.api_value().to_string());
            assert!(!label.is_empty(), "{kind:?} has no label");
            assert!(!labels.contains(&label), "{label:?} twice");
            assert!(!api_values.contains(&api_value), "{api_value:?} twice");
            // The prefix must be the one `from_member` reads back, or the
            // table's kind column and the parser disagree about the same wire
            // string.
            assert_eq!(
                IamMemberKind::from_member(&format!("{api_value}:x")),
                if kind.is_public() {
                    IamMemberKind::Unrecognized(api_value.clone())
                } else {
                    kind.clone()
                },
                "api_value of {kind:?} does not round-trip through from_member"
            );
            labels.push(label);
            api_values.push(api_value);
        }
    }

    /// The whole member grammar, read end to end into a row. Each case is a
    /// shape a `split(':')` gets wrong, and each one is asserted on the
    /// *identity* rather than only on the kind: a row whose kind is right and
    /// whose principal is blank still lists, so nothing but an assertion on
    /// the value catches it.
    #[test]
    fn a_member_row_carries_its_identity_kind_and_deleted_marker() {
        let cases = [
            // (raw member, kind, principal, deleted)
            (
                "user:alice@example.com",
                IamMemberKind::User,
                "alice@example.com",
                false,
            ),
            (
                "serviceAccount:sa@p.iam.gserviceaccount.com",
                IamMemberKind::ServiceAccount,
                "sa@p.iam.gserviceaccount.com",
                false,
            ),
            // The population kinds keep their whole token: there is no suffix,
            // and an empty cell here is the audit finding going missing.
            ("allUsers", IamMemberKind::AllUsers, "allUsers", false),
            (
                "allAuthenticatedUsers",
                IamMemberKind::AllAuthenticatedUsers,
                "allAuthenticatedUsers",
                false,
            ),
            // `deleted:` comes off before the kind is read, and the `?uid=`
            // disambiguator stays: it is the only thing telling two deleted
            // accounts at the same address apart.
            (
                "deleted:serviceAccount:sa@p.iam.gserviceaccount.com?uid=123",
                IamMemberKind::ServiceAccount,
                "sa@p.iam.gserviceaccount.com?uid=123",
                true,
            ),
            (
                "deleted:user:bob@example.com?uid=9",
                IamMemberKind::User,
                "bob@example.com?uid=9",
                true,
            ),
            // The colons inside the URL belong to the identifier.
            (
                "principalSet://iam.googleapis.com/projects/1/locations/global/workloadIdentityPools/p/*",
                IamMemberKind::PrincipalSet,
                "//iam.googleapis.com/projects/1/locations/global/workloadIdentityPools/p/*",
                false,
            ),
            // A federated subject is an arbitrary string from the external
            // identity provider and may itself contain colons. Only the
            // *first* colon is the kind separator, so this is the case that
            // distinguishes splitting from the front from splitting from the
            // back — with a single-colon member the two agree, and a test
            // built only from those pins nothing about the direction.
            (
                "principal://iam.googleapis.com/locations/global/workforcePools/p/subject/user:alice@example.com",
                IamMemberKind::Principal,
                "//iam.googleapis.com/locations/global/workforcePools/p/subject/user:alice@example.com",
                false,
            ),
            (
                "deleted:principal://iam.googleapis.com/locations/global/workforcePools/p/subject/x:y",
                IamMemberKind::Principal,
                "//iam.googleapis.com/locations/global/workforcePools/p/subject/x:y",
                true,
            ),
            (
                "bareword",
                IamMemberKind::Unrecognized("bareword".to_string()),
                "bareword",
                false,
            ),
        ];
        for (raw, kind, principal, deleted) in cases {
            let row = IamPolicyMember::from_wire("roles/viewer", raw, None);
            assert_eq!(row.kind, kind, "kind of {raw:?}");
            assert_eq!(row.principal, principal, "principal of {raw:?}");
            assert_eq!(row.deleted, deleted, "deleted of {raw:?}");
            assert_eq!(row.raw_member, raw, "raw_member of {raw:?}");
            assert_eq!(row.role, "roles/viewer");
            assert_eq!(row.condition, None);
        }
    }

    /// A conditional grant is a narrower grant, so the title has to survive to
    /// the row. A blank title is not one: it would paint a cell identical to an
    /// unconditional row's while the binding is in fact conditional.
    #[test]
    fn a_condition_title_survives_and_a_blank_one_is_no_condition() {
        let with = IamPolicyMember::from_wire("roles/viewer", "user:a@b.c", Some("expires 2027"));
        assert_eq!(with.condition.as_deref(), Some("expires 2027"));
        for blank in ["", "   "] {
            assert_eq!(
                IamPolicyMember::from_wire("roles/viewer", "user:a@b.c", Some(blank)).condition,
                None,
                "blank title {blank:?} must not read as a condition"
            );
        }
    }

    /// The public badge, end to end from the wire, over every member shape.
    /// The `deleted:allUsers` row is the one that would slip a naive
    /// implementation: the marker changes the string the kind is read from.
    #[test]
    fn only_the_unbounded_population_rows_are_public() {
        let cases = [
            ("allUsers", true),
            ("allAuthenticatedUsers", true),
            ("deleted:allUsers", true),
            ("user:alice@example.com", false),
            ("group:team@example.com", false),
            ("domain:example.com", false),
            ("serviceAccount:sa@p.iam.gserviceaccount.com", false),
            ("deleted:user:bob@example.com?uid=9", false),
            ("bareword", false),
        ];
        for (raw, expected) in cases {
            assert_eq!(
                IamPolicyMember::from_wire("roles/owner", raw, None).is_public(),
                expected,
                "member {raw:?}"
            );
        }
    }

    /// A custom role and a predefined role can share a trailing segment, so the
    /// role is carried verbatim. Shortening it would render two different
    /// grants — one Google defines, one this project defines — identically.
    #[test]
    fn a_role_is_carried_verbatim_so_a_custom_role_is_distinguishable() {
        let predefined = IamPolicyMember::from_wire("roles/storage.admin", "user:a@b.c", None);
        let custom =
            IamPolicyMember::from_wire("projects/p/roles/storage.admin", "user:a@b.c", None);
        assert_eq!(predefined.role, "roles/storage.admin");
        assert_eq!(custom.role, "projects/p/roles/storage.admin");
        assert_ne!(predefined.role, custom.role);
    }

    /// Two features word their refusals from this table (`L`'s "No GCE
    /// instance selected to open logs for", `J`'s "... to jump from"), so a
    /// noun shared by two services would make one of those messages name the
    /// wrong thing.
    #[test]
    fn every_service_has_its_own_selection_noun() {
        let mut seen: Vec<&str> = Vec::new();
        for kind in ResourceServiceKind::ALL {
            let noun = kind.selection_noun();
            assert!(!noun.is_empty(), "{kind:?} has no selection noun");
            assert!(!seen.contains(&noun), "{noun:?} names two services");
            seen.push(noun);
        }
    }
}
