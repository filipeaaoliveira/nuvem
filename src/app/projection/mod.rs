//! Render-facing projections for the resource workspace.
//!
//! Projections keep shared list/detail rendering independent from the typed
//! state and service-specific row and detail models.

pub mod binding;
pub mod columns;
pub mod configurations;
pub mod detail_body;
pub mod finops;
mod frame;
pub(crate) mod home;
pub mod logging;
pub mod table;
pub mod tunnels;

pub use crate::app::state::navigator::NavigatorFocus;
pub use binding::{
    Binding, DisplayBinding, HelpContext, HelpModel, HelpSection, KeyTrigger,
    context_bindings_with, help_model, help_model_with,
};
pub use configurations::project_configurations_table;
pub use frame::{
    AppFrameProjection, FrameContextProjection, FrameHint, FrameOperationProjection,
    drilled_list_name, format_last_refresh, project_app_frame, project_app_frame_with,
};
pub use home::{
    DELTA_HIGHLIGHT_WINDOW, GcloudHealthProjection, HomeOverviewProjection,
    InventoryCardProjection, OperationLineProjection, ResourceFreshness, SPARKLINE_MIN_SAMPLES,
    project_home_overview,
};
pub use logging::{
    LogDetailView, LogLabelProjection, LoggingBodyState, LoggingHeaderProjection,
    LoggingWorkspaceProjection, project_logging,
};
pub use table::{
    Cell, CellAlign, ColumnFit, NO_COLUMN, RowCounts, SortDirection, SortSpec, StatusSemantic,
    TableColumn, TableModel, TableRow,
};
pub use tunnels::{
    TUNNELS_EMPTY_MESSAGE, TunnelsBodyState, TunnelsWorkspaceProjection, project_tunnels,
};

use crate::app::service_registry::{ServiceAvailability, ServiceId, ServiceTarget, descriptor};
use crate::app::state::resources::{
    BigQueryView, CloudRunDetailTab, CloudRunRevisionTab, CloudSqlDetailTab, ComputeDetailTab,
    DISABLED_BADGE, FilterScope, GkeDetailTab, ORPHAN_BADGE, ResourceLoadPhase, ResourcesState,
    ResourcesViewMode, SecretDetailTab, StorageBrowserRow, StorageDetailTab, StorageView,
    dead_letter_label, disk_attached_label, disk_size_label, firewall_source_or_dest_label,
    firewall_targets_label, parent_topic_label, private_google_access_label,
    service_account_state_label,
};
use crate::app::state::root::{RootState, Workspace};
use chrono::DateTime;
use nuvem_domain::gcp::diagnostics::GcloudErrorKind;
use nuvem_domain::gcp::resource::{
    BigQueryDataset, BigQuerySchemaField, BigQueryTable, CloudFunction, CloudRunRevisionDetail,
    CloudRunServiceDetail, CloudSqlAvailability, CloudSqlInstanceDetail, ComputeInstanceDetail,
    ComputeKeyValue, EndpointVisibility, FirewallAction, FirewallDirection, FirewallRule,
    GkeClusterDetail, IamPolicyMember, PersistentDisk, PubSubSubscription, PubSubSubscriptionState,
    PubSubTopic, ResourceServiceKind, Secret, SecretDetail, ServiceAccount, StorageBucketDetail,
    StorageObjectDetail, TableStat, VpcNetwork, VpcRoutingMode, VpcSubnet,
};
use nuvem_domain::gcp::text::sanitize_display_text;
use ratatui::layout::Rect;

#[derive(Debug, Clone, PartialEq, Eq)]
/// Render-facing service navigator state and bounded list viewport.
pub struct ServiceNavigatorProjection {
    pub entries: Vec<ServiceNavigatorEntryProjection>,
    pub selected_id: ServiceId,
    pub selected_visible: usize,
    pub filter: NavigatorFilterProjection,
    pub sidebar_visible: bool,
    pub focus: NavigatorFocus,
    pub viewport_start: usize,
    pub viewport_end: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// A category header or display-ready service row in the navigator.
pub struct ServiceNavigatorEntryProjection {
    pub id: Option<ServiceId>,
    pub category: Option<&'static str>,
    pub label: &'static str,
    pub selectable: bool,
    pub availability: Option<ServiceAvailability>,
    pub unavailable_reason: Option<&'static str>,
    pub is_active: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Draft and applied service-search state exposed to the navigator renderer.
pub struct NavigatorFilterProjection {
    pub active: bool,
    pub draft: String,
    pub applied: String,
}

/// Projects registry-backed services into grouped, render-ready navigator data.
pub fn project_service_navigator(root: &RootState, area: Rect) -> ServiceNavigatorProjection {
    let selected_id = root.navigator.selected_id();
    let active_service = root.resources.active_service();
    let focus = if root.workspace == Workspace::Home {
        NavigatorFocus::Navigator
    } else {
        root.navigator.focus()
    };
    let mut entries = Vec::new();
    let mut selected_visible = 0;
    let mut previous_category = None;

    for service_id in root.navigator.visible_ids() {
        let service = descriptor(service_id);
        if previous_category != Some(service.category) {
            entries.push(ServiceNavigatorEntryProjection {
                id: None,
                category: Some(service.category),
                label: service.category,
                selectable: false,
                availability: None,
                unavailable_reason: None,
                is_active: false,
            });
            previous_category = Some(service.category);
        }

        if service_id == selected_id {
            selected_visible = entries.len();
        }

        entries.push(ServiceNavigatorEntryProjection {
            id: Some(service.id),
            category: None,
            label: service.label,
            selectable: true,
            availability: Some(service.availability),
            unavailable_reason: service.unavailable_reason(),
            is_active: service.target.and_then(ServiceTarget::resource_kind)
                == Some(active_service),
        });
    }

    let viewport_size = usize::from(area.height.saturating_sub(2)).max(1);
    let (viewport_start, viewport_end) = if entries.is_empty() {
        (0, 0)
    } else {
        let max_start = entries.len().saturating_sub(viewport_size);
        let viewport_start = selected_visible
            .saturating_sub(viewport_size.saturating_sub(1))
            .min(max_start);
        let viewport_end = (viewport_start + viewport_size).min(entries.len());
        (viewport_start, viewport_end)
    };

    ServiceNavigatorProjection {
        entries,
        selected_id,
        selected_visible,
        filter: NavigatorFilterProjection {
            active: root.navigator.is_filter_input_active(),
            draft: root.navigator.filter_draft().to_string(),
            applied: root.navigator.filter_applied().to_string(),
        },
        sidebar_visible: root.navigator.sidebar_visible(),
        focus,
        viewport_start,
        viewport_end,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// Loading state for the active resource collection.
pub enum ResourceWorkspaceStatus {
    NotLoaded,
    Loading,
    Ready,
    Stale,
    Failed,
}

/// Braille spinner frames for the `Loading` status affordance. The active
/// frame is derived from elapsed time (see [`project_resources`]) rather
/// than a stored counter, so repeated projections stay pure.
const SPINNER_FRAMES: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

#[derive(Debug, Clone, PartialEq, Eq)]
/// Live fetch progress for the active collection (status == Loading).
pub struct LoadProgressProjection {
    pub elapsed_secs: u64,
    pub spinner_frame: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Filter state exposed to the resource list shell.
pub struct ResourceFilterProjection {
    pub active: bool,
    pub draft: String,
    pub applied: String,
    /// Lowercase mode name for the filter chip: "regex" | "fuzzy" | "label" |
    /// "inverse".
    pub mode_label: &'static str,
    /// Non-fatal inline filter error, rendered next to the filter text.
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Shared resource-shell metadata plus the active service projection.
pub struct ResourceWorkspaceProjection<'a> {
    pub service: ResourceServiceKind,
    pub service_label: &'static str,
    pub status: ResourceWorkspaceStatus,
    /// Live fetch progress, `Some` iff `status` is `Loading`.
    pub load_progress: Option<LoadProgressProjection>,
    /// The `ResourceLoadPhase::Failed` message, `Some` iff `status` is
    /// `Failed`.
    pub load_error: Option<String>,
    pub context_label: Option<String>,
    pub view_mode: ResourcesViewMode,
    pub filter: ResourceFilterProjection,
    /// Whether the applied filter is local-only, or (Cloud Storage object
    /// browser only) also narrowed the server-side listing via a safe
    /// prefix reload.
    pub filter_scope: FilterScope,
    pub sort_label: &'static str,
    pub total_rows: usize,
    pub visible_rows: usize,
    pub selected_visible: Option<usize>,
    pub wide_columns: bool,
    /// True when another page of rows can be fetched on demand (currently
    /// only meaningful for the Cloud Storage object browser; `false` for
    /// every other service and view).
    pub has_more: bool,
    /// Zero-based index of the most recently loaded page (currently only
    /// meaningful for the Cloud Storage object browser; `0` otherwise).
    pub page_index: usize,
    pub list: ResourceListProjection,
    pub detail: Option<ResourceDetailProjection<'a>>,
    pub table: Option<TableModel>,
    /// Cloud Storage only: the classified problem for the current list
    /// scope (bucket list, or the object browser's first page), rendered
    /// instead of the generic empty/stale state when present. See
    /// [`StorageProblemProjection`].
    pub storage_problem: Option<StorageProblemProjection>,
    /// Cloud Storage only: a page-append (`NextPage`) failure message.
    /// Already-loaded rows remain visible; this is a non-destructive banner
    /// rather than a full-list error state.
    pub partial_page_error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Cloud Storage only: a classified list-load failure, distinguishing a
/// requester-pays access condition from a generic (e.g. permission-denied)
/// gcloud failure so each renders a distinct message.
pub struct StorageProblemProjection {
    pub kind: GcloudErrorKind,
    pub message: String,
    pub requester_pays: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Service-specific list rows prepared for rendering.
pub enum ResourceListProjection {
    Compute(Vec<ComputeInstanceProjection>),
    Gke(Vec<GkeClusterProjection>),
    CloudSql(Vec<CloudSqlInstanceProjection>),
    CloudRun(Vec<CloudRunServiceProjection>),
    Storage(Vec<StorageRowProjection>),
    Secrets(Vec<SecretRowProjection>),
    CloudFunctions(Vec<CloudFunctionRowProjection>),
    PersistentDisks(Vec<PersistentDiskRowProjection>),
    PubSubTopics(Vec<PubSubTopicRowProjection>),
    PubSubSubscriptions(Vec<PubSubSubscriptionRowProjection>),
    VpcNetworks(Vec<VpcNetworkRowProjection>),
    VpcSubnets(Vec<VpcSubnetRowProjection>),
    FirewallRules(Vec<FirewallRuleRowProjection>),
    BigQueryDatasets(Vec<BigQueryDatasetRowProjection>),
    /// The tables listed inside an entered dataset. A second variant rather
    /// than a second `ResourceServiceKind`: the drill-down lives inside the
    /// datasets kind (see `state::resources::bigquery`), and it is the view,
    /// not the service, that decides which of the two lists is projected.
    BigQueryTables(Vec<BigQueryTableRowProjection>),
    /// The columns of an entered table, a third variant for the reason the
    /// second is one: the schema is a view of the datasets kind, not a kind of
    /// its own, and it is the view that decides which of the three lists is
    /// projected.
    BigQuerySchema(Vec<BigQuerySchemaFieldRowProjection>),
    IamServiceAccounts(Vec<ServiceAccountRowProjection>),
    IamPolicy(Vec<IamPolicyRowProjection>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// A single display-ready row in the Cloud Storage list, which is either a
/// bucket (bucket list view) or a browser entry (object browser view). All
/// name/key/prefix cells are already sanitized for terminal display.
pub enum StorageRowProjection {
    Bucket(StorageBucketRowProjection),
    Object(StorageObjectRowProjection),
}

impl StorageRowProjection {
    /// The bucket row, if this is one. Lets the bucket list feed
    /// [`columns::table_parts`] the row type its catalog is written against.
    fn as_bucket(&self) -> Option<&StorageBucketRowProjection> {
        match self {
            Self::Bucket(bucket) => Some(bucket),
            Self::Object(_) => None,
        }
    }

    /// The object-browser row, if this is one.
    fn as_object(&self) -> Option<&StorageObjectRowProjection> {
        match self {
            Self::Object(object) => Some(object),
            Self::Bucket(_) => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Display-ready Cloud Storage bucket row.
pub struct StorageBucketRowProjection {
    pub name: String,
    pub location: String,
    pub storage_class: String,
    pub updated: String,
    pub versioning: String,
    pub public_access: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Display-ready object-browser row (a folder prefix or a concrete object).
pub struct StorageObjectRowProjection {
    pub is_prefix: bool,
    pub name: String,
    pub size: String,
    pub storage_class: String,
    pub updated: String,
    pub encryption: String,
    pub content_type: String,
    /// Informational indicator (not an error/problem state): "yes" when the
    /// object has a retention policy in effect, empty otherwise. Folder
    /// (prefix) rows never carry retention and are always empty.
    pub retention: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Display-ready GCE instance row.
pub struct ComputeInstanceProjection {
    pub name: String,
    pub status: String,
    pub status_semantic: StatusSemantic,
    pub zone: String,
    pub machine_type: String,
    pub internal_ip: Option<String>,
    pub external_ip: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Display-ready GKE cluster row.
pub struct GkeClusterProjection {
    pub name: String,
    pub status: String,
    pub status_semantic: StatusSemantic,
    pub location: String,
    pub version: Option<String>,
    pub node_count: Option<u32>,
    pub endpoint_visibility: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Display-ready Cloud SQL instance row.
pub struct CloudSqlInstanceProjection {
    pub name: String,
    pub engine: String,
    pub database_version: String,
    pub region: String,
    pub tier: String,
    pub availability: String,
    pub connectivity: String,
    pub status: String,
    pub status_semantic: StatusSemantic,
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Display-ready Cloud Run service row.
pub struct CloudRunServiceProjection {
    pub name: String,
    pub status: String,
    pub status_semantic: StatusSemantic,
    pub region: String,
    pub url: Option<String>,
    pub latest_ready_revision: Option<String>,
    pub traffic_summary: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// A display-ready Secret Manager row.
///
/// Every field is metadata. There is no payload field here and there must never
/// be one — see `tests/secrets_no_payload_access_test.rs`.
pub struct PubSubTopicRowProjection {
    pub name: String,
    /// The count already rendered: digits, or the dash that stands for a count
    /// the subscriptions call could not supply. Rendered here rather than in
    /// the column so the filter and the cell agree on what the row says.
    pub subscriptions: String,
    pub retention: Option<String>,
    pub kms_key: Option<String>,
    /// Labels flattened to `key=value` pairs for a single cell.
    pub labels: String,
}

/// Display-ready VPC network row.
///
/// Four cells, because the table has four columns. The description is
/// searchable off the row itself and is deliberately not projected here — it
/// is unbounded free text with no column (see
/// [`columns::VPC_NETWORK_COLUMNS`]) — and a field on this struct that no
/// column reads is a cell waiting to disagree with the filter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VpcNetworkRowProjection {
    pub name: String,
    /// `AUTO`, `CUSTOM` or `LEGACY`, rendered once by
    /// [`nuvem_domain::gcp::resource::VpcSubnetMode::label`] so the cell and
    /// the filter cannot disagree.
    pub mode: String,
    /// `REGIONAL`, `GLOBAL`, or whatever the API sent that this build does not
    /// recognise. `None` where the API reported no routing config at all,
    /// which paints as a dash rather than as the `REGIONAL` default — a
    /// default is a claim about how the network was created, not about what
    /// the API said.
    pub routing: Option<String>,
    /// How many subnets the network has, already rendered. `0` is a real
    /// answer here and is painted as `0`, not as a dash: a custom-mode network
    /// with no subnets is a finding, and the API listed its subnets and found
    /// none rather than declining to say.
    pub subnets: String,
}

/// Display-ready VPC subnet row.
///
/// Six cells, because the table has six columns. `region` and `network` are
/// the *ids*, already reduced from the self-links the API sent — the reduction
/// happens in the parser, so this projection never sees a URL and cannot
/// disagree with the filter about what a network is called.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VpcSubnetRowProjection {
    pub name: String,
    /// The region id. Not optional and not droppable: it is half the row's
    /// identity, since an auto-mode network calls every subnet `default`.
    pub region: String,
    pub network: String,
    /// The primary IPv4 range, as the API spelled it.
    pub range: String,
    /// The gateway address, absent where the API reported none.
    pub gateway: Option<String>,
    /// `On` or `Off`, rendered once by `private_google_access_label` so the
    /// cell and the filter cannot disagree. Not a dash for `false`: the API
    /// documents an absent flag as off, so `Off` is an answer rather than a
    /// missing value.
    pub private_google_access: String,
}

/// Display-ready firewall rule row.
///
/// The `disabled` state is **not** a field here: it is folded into `name` as a
/// leading badge, exactly as the IAM table folds `PUBLIC` and `DELETED` into
/// its member cell. The identity column is the one `plan_table` may never
/// drop, so a badge on it survives every terminal width — and "this rule is
/// not in force" is a fact that must not be the first thing lost, because a
/// disabled DENY rule reads as a live one in every other column.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FirewallRuleRowProjection {
    /// The rule's name, prefixed with [`DISABLED_BADGE`] when it is not in
    /// force.
    pub name: String,
    pub network: String,
    pub direction: Option<String>,
    /// Already rendered. `None` where the API reported no priority, which
    /// paints a dash rather than the `1000` default.
    pub priority: Option<String>,
    /// `ALLOW`, `DENY`, or `None` where the rule carried neither key — which
    /// paints a dash rather than defaulting to the common case. A rule the
    /// parser could not read must not read as one that permits traffic.
    pub action: Option<String>,
    /// The target tags or service accounts, or `all instances` where the rule
    /// scopes by neither. Never blank: an unscoped rule has the widest
    /// possible reach, and a blank cell reads as the narrowest.
    pub targets: String,
    pub source_or_dest: Option<String>,
    /// Whether the row is styled as degraded. A disabled rule is not an error,
    /// but it is a row that does not do what it says, which is the same thing
    /// this styling means everywhere else in the application.
    pub disabled: bool,
}

/// Display-ready BigQuery dataset row.
///
/// Three cells, because the table has three columns. The friendly name is
/// searchable off the row itself and is deliberately not projected here — see
/// [`columns::BIGQUERY_DATASET_COLUMNS`] for why it has no column — and a
/// field on this struct that no column reads is a cell waiting to disagree
/// with the filter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BigQueryDatasetRowProjection {
    /// The dataset id — the row's identity, so it is keyed `name` in the
    /// column catalogue even though the header reads `DATASET`.
    pub name: String,
    /// `None` where the listing reported no location, so the cell paints the
    /// dash rather than a blank that would read as a rendering fault.
    pub location: Option<String>,
    /// Labels flattened to `key=value` pairs for a single cell, or the dash
    /// where the dataset carries none.
    pub labels: String,
}

/// Display-ready BigQuery table row: one entry of `bq ls <dataset>`, plus the
/// three numbers that row's own `bq show` filled in.
///
/// Six cells, because the tables table has six columns. The labels are
/// searchable off the row and deliberately not projected here — see
/// [`columns::BIGQUERY_TABLE_COLUMNS`] for why they have no column — and a
/// field no column reads is a cell waiting to disagree with the filter.
///
/// The last three are `Option<String>` and not `String` for the reason
/// [`TableStat`] is not an `i64`: `None` is a fact about the call — the `show`
/// was refused, or has not answered yet — and it has to stay distinguishable
/// from a table that really holds zero rows. Resolving the absence here, once,
/// is what keeps every formatter below it out of the question: a value that is
/// not there never reaches one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BigQueryTableRowProjection {
    /// The table id — the row's identity, so it is keyed `name` in the column
    /// catalogue even though the header reads `TABLE`.
    pub name: String,
    /// The kind's reader-facing word, already resolved by
    /// `BigQueryTableKind::label`, so an unrecognised kind reaches the cell as
    /// BigQuery spelled it.
    pub kind: String,
    /// `None` where the listing carried no usable `creationTime`, so the cell
    /// paints the dash rather than a blank that would read as a rendering
    /// fault.
    pub created: Option<String>,
    /// The row count in plain digits, or `None` where this table's `bq show`
    /// did not answer with one. `Some("0")` is a table with no rows in it; a
    /// dash is a table nobody counted.
    pub rows: Option<String>,
    /// The byte size, humanised by the same `format_object_size` the Cloud
    /// Storage object browser's `SIZE` column uses, so two byte counts in one
    /// binary are spelled one way.
    pub bytes: Option<String>,
    /// `lastModifiedTime` — epoch milliseconds like `created` — rendered as
    /// the same UTC instant that column is.
    pub last_modified: Option<String>,
}

/// Display-ready BigQuery schema row: one field of the entered table's
/// flattened schema.
///
/// Four cells, one per fact `bq show` carries about a field, and the same four
/// `schema_field_searchable_fields` filters on — so a row the filter keeps is a
/// row whose match an operator can see. All four have a column, in this order,
/// in [`columns::BIGQUERY_SCHEMA_COLUMNS`]. The description is the cell that
/// can run long, so it is the one the layout elides first; the preview beside
/// the list carries it unelided either way.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BigQuerySchemaFieldRowProjection {
    /// The dotted path — the row's identity, so it is keyed `name` the way the
    /// other two BigQuery lists key theirs.
    pub name: String,
    pub field_type: String,
    /// `NULLABLE`, `REQUIRED` or `REPEATED`, never empty: an omitted mode
    /// arrives from the parser as the API's documented `NULLABLE` default.
    pub mode: String,
    /// `None` where the field carried no description, so the cell paints the
    /// dash rather than a blank. An empty description stays `Some("")` and is
    /// not folded into the absence — see `BigQuerySchemaField`.
    pub description: Option<String>,
}

/// Display-ready Pub/Sub subscription row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PubSubSubscriptionRowProjection {
    pub name: String,
    /// The parent topic's short id, or the API's `_deleted-topic_` sentinel
    /// carried verbatim. Rendered here rather than in the column so the cell
    /// and the filter agree on what the row says.
    pub topic: String,
    /// `Pull` or `Push`, rendered once by
    /// [`nuvem_domain::gcp::resource::PubSubDelivery::label`]. The push
    /// endpoint has no column — it is searchable only.
    pub delivery: String,
    pub state: String,
    /// How the `STATE` cell is styled. `RESOURCE_ERROR` is the reason this
    /// field exists: the subscription is alive and its every other column
    /// reads normally, while Pub/Sub cannot deliver to it. Spelling that state
    /// in plain text and leaving it unstyled would bury the one row on the
    /// table an operator needs to find.
    pub status_semantic: StatusSemantic,
    /// The ack deadline in seconds, already rendered, or `None` where the API
    /// returned no value at all. `None` paints as a dash rather than `0`,
    /// which would be a specific and wrong claim.
    pub ack_deadline: Option<String>,
    pub retention: Option<String>,
    /// The dead-letter topic's short id, absent when no policy is configured.
    pub dead_letter: Option<String>,
    /// Labels flattened to `key=value` pairs for a single cell.
    pub labels: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Display-ready IAM service account row.
pub struct ServiceAccountRowProjection {
    /// The email. It is the account's identity — unique within the project and
    /// what every other resource names the account by — so it rides in the
    /// field the identity column reads, even though the header says `EMAIL`.
    pub name: String,
    pub display_name: Option<String>,
    /// `Enabled`/`Disabled`, rendered once by [`service_account_state_label`]
    /// so the cell and the filter cannot disagree about the word.
    pub state: String,
    pub unique_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Display-ready IAM policy binding row: one (role, member) pair.
pub struct IamPolicyRowProjection {
    /// The member exactly as the policy holds it. It is the row's identity, so
    /// it rides in the field the identity column reads even though the header
    /// says `MEMBER`.
    pub name: String,
    /// The role, verbatim — `roles/x` and `projects/p/roles/x` are different
    /// grants and neither is shortened to its trailing segment.
    pub role: String,
    /// The member kind's label, rendered once by the domain type so the cell
    /// and the filter cannot disagree about the word.
    pub kind: String,
    pub condition: Option<String>,
    /// Whether the principal has since been deleted. A `bool` rather than the
    /// badge text the way `PersistentDiskRowProjection::orphan` carries it,
    /// because these two facts share one cell: the member column composes
    /// whichever badges apply, and a row can wear both.
    pub deleted: bool,
    /// Whether the grant is held by an unbounded population — the finding a
    /// policy audit exists to surface. Badged in the member cell, for the same
    /// reason and by the same code as `deleted`.
    pub is_public: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecretRowProjection {
    pub name: String,
    pub created: String,
    pub replication: String,
    /// Labels flattened to `key=value` pairs for a single cell.
    pub labels: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// A display-ready Cloud Functions row.
///
/// `name` and `region` together identify it — a function's name is unique
/// within a region, not within a project. `updated` and `url` stay `Option`
/// rather than collapsing to a dash here, so the cell that reports "the API
/// did not report one" is a `Cell::missing` the renderer can style, not a
/// hyphen indistinguishable from a URL that literally is one.
pub struct PersistentDiskRowProjection {
    pub name: String,
    pub location: String,
    pub status: String,
    pub status_semantic: StatusSemantic,
    /// `ORPHAN` for a badged row and empty for every other, rather than an
    /// `Option`: the badge's absence is not "the API did not report one" — it
    /// is the positive answer "something is using this disk" — so a
    /// `Cell::missing` would style a known fact as a gap in the data.
    pub orphan: String,
    pub size: String,
    pub disk_type: String,
    pub attached: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// A display-ready persistent-disk row.
///
/// `name` and `location` together identify it — a disk's name is unique within
/// its zone or region, not within a project.
pub struct CloudFunctionRowProjection {
    pub name: String,
    pub region: String,
    pub state: String,
    pub status_semantic: StatusSemantic,
    pub generation: String,
    pub updated: Option<String>,
    pub url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Borrowed secret detail data for the detail-pane renderer.
pub struct SecretDetailProjection<'a> {
    pub target_name: String,
    pub active_tab: SecretDetailTab,
    pub scroll: u16,
    pub detail: Option<&'a SecretDetail>,
    pub error: Option<&'a str>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Typed detail projection for the active resource service.
pub enum ResourceDetailProjection<'a> {
    Compute(ComputeDetailProjection<'a>),
    Gke(GkeDetailProjection<'a>),
    CloudSql(CloudSqlDetailProjection<'a>),
    CloudRun(CloudRunDetailProjection<'a>),
    Storage(StorageDetailProjection<'a>),
    Secrets(SecretDetailProjection<'a>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Typed Cloud Storage detail projection (bucket metadata or object metadata).
pub enum StorageDetailProjection<'a> {
    Bucket(StorageBucketDetailProjection<'a>),
    Object(StorageObjectDetailProjection<'a>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Borrowed bucket-detail data for the resource detail renderer.
pub struct StorageBucketDetailProjection<'a> {
    pub target_name: String,
    pub active_tab: StorageDetailTab,
    pub scroll: u16,
    pub detail: Option<&'a StorageBucketDetail>,
    pub error: Option<&'a str>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Borrowed object-detail data for the resource detail renderer.
pub struct StorageObjectDetailProjection<'a> {
    pub target_name: String,
    pub active_tab: StorageDetailTab,
    pub scroll: u16,
    pub detail: Option<&'a StorageObjectDetail>,
    pub error: Option<&'a str>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Borrowed GCE detail data for the resource detail renderer.
pub struct ComputeDetailProjection<'a> {
    pub target_name: String,
    pub active_tab: ComputeDetailTab,
    pub scroll: u16,
    pub detail: Option<&'a ComputeInstanceDetail>,
    pub error: Option<&'a str>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Borrowed GKE detail data for the resource detail renderer.
pub struct GkeDetailProjection<'a> {
    pub target_name: String,
    pub active_tab: GkeDetailTab,
    pub scroll: u16,
    pub detail: Option<&'a GkeClusterDetail>,
    pub error: Option<&'a str>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Borrowed Cloud SQL detail data for the resource detail renderer.
pub struct CloudSqlDetailProjection<'a> {
    pub target_name: String,
    pub active_tab: CloudSqlDetailTab,
    pub scroll: u16,
    pub detail: Option<&'a CloudSqlInstanceDetail>,
    pub error: Option<&'a str>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Borrowed Cloud Run service detail data for the resource detail renderer.
pub struct CloudRunDetailProjection<'a> {
    pub target_name: String,
    pub active_tab: CloudRunDetailTab,
    pub scroll: u16,
    pub selected_revision: usize,
    pub detail: Option<&'a CloudRunServiceDetail>,
    pub error: Option<&'a str>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Borrowed Cloud Run revision detail data for the nested revision drill-down
/// renderer. Exposed separately via [`project_cloud_run_revision`] since the
/// revision view is Cloud-Run-specific and layers on top of the service detail.
pub struct CloudRunRevisionProjection<'a> {
    pub target_name: String,
    pub active_tab: CloudRunRevisionTab,
    pub scroll: u16,
    pub detail: Option<&'a CloudRunRevisionDetail>,
    pub error: Option<&'a str>,
}

/// Projects typed resource state into the shared render-facing workspace shell.
pub fn project_resources<'a>(state: &'a ResourcesState) -> ResourceWorkspaceProjection<'a> {
    let view_mode = if state.is_showing_detail() {
        ResourcesViewMode::Detail
    } else {
        ResourcesViewMode::List
    };
    build_workspace_projection(state, view_mode)
}

/// Projects the resource list rows regardless of whether the detail view is
/// active. Used by the wide adaptive layout to keep the list pane visible
/// alongside the detail pane; the `detail` field is always `None`.
pub fn project_resource_list(state: &ResourcesState) -> ResourceWorkspaceProjection<'_> {
    build_workspace_projection(state, ResourcesViewMode::List)
}

fn build_workspace_projection(
    state: &ResourcesState,
    view_mode: ResourcesViewMode,
) -> ResourceWorkspaceProjection<'_> {
    let service = state.active_service();
    let (list, total_rows, visible_rows, detail) = match service {
        ResourceServiceKind::ComputeInstances => {
            let total_rows = state.instances().len();
            match view_mode {
                ResourcesViewMode::List => {
                    let rows = state
                        .visible_instances()
                        .into_iter()
                        .map(|instance| ComputeInstanceProjection {
                            name: instance.name.clone(),
                            status: sanitize_display_text(&instance.status.label()),
                            status_semantic: StatusSemantic::from_resource_status(&instance.status),
                            zone: instance.zone.clone(),
                            machine_type: instance.machine_type.clone(),
                            internal_ip: instance.internal_ip.clone(),
                            external_ip: instance.external_ip.clone(),
                        })
                        .collect::<Vec<_>>();
                    let visible_rows = rows.len();
                    (
                        ResourceListProjection::Compute(rows),
                        total_rows,
                        visible_rows,
                        None,
                    )
                }
                ResourcesViewMode::Detail => (
                    ResourceListProjection::Compute(Vec::new()),
                    total_rows,
                    state.visible_instance_count(),
                    Some(ResourceDetailProjection::Compute(ComputeDetailProjection {
                        target_name: state
                            .detail_target_name()
                            .unwrap_or("selected instance")
                            .to_string(),
                        active_tab: state.active_detail_tab(),
                        scroll: state.detail_scroll(),
                        detail: state.detail(),
                        error: state.detail_error(),
                    })),
                ),
            }
        }
        ResourceServiceKind::GkeClusters => {
            let total_rows = state.gke_clusters().len();
            match view_mode {
                ResourcesViewMode::List => {
                    let rows = state
                        .visible_gke_clusters()
                        .into_iter()
                        .map(|cluster| GkeClusterProjection {
                            name: cluster.name.clone(),
                            status: sanitize_display_text(&cluster.status.label()),
                            status_semantic: StatusSemantic::from_gke_status(&cluster.status),
                            location: cluster.location.clone(),
                            version: cluster.current_master_version.clone(),
                            node_count: cluster.current_node_count,
                            endpoint_visibility: endpoint_visibility_label(
                                cluster.endpoint_visibility,
                            ),
                        })
                        .collect::<Vec<_>>();
                    let visible_rows = rows.len();
                    (
                        ResourceListProjection::Gke(rows),
                        total_rows,
                        visible_rows,
                        None,
                    )
                }
                ResourcesViewMode::Detail => (
                    ResourceListProjection::Gke(Vec::new()),
                    total_rows,
                    state.visible_gke_cluster_count(),
                    Some(ResourceDetailProjection::Gke(GkeDetailProjection {
                        target_name: state
                            .gke_detail_target_name()
                            .unwrap_or("selected cluster")
                            .to_string(),
                        active_tab: state.active_gke_detail_tab(),
                        scroll: state.detail_scroll(),
                        detail: state.gke_detail(),
                        error: state.gke_detail_error(),
                    })),
                ),
            }
        }
        ResourceServiceKind::CloudSql => {
            let total_rows = state.cloud_sql_instances().len();
            match view_mode {
                ResourcesViewMode::List => {
                    let rows = state
                        .visible_cloud_sql_instances()
                        .into_iter()
                        .map(|instance| CloudSqlInstanceProjection {
                            name: instance.name.clone(),
                            engine: instance.engine.clone(),
                            database_version: instance.database_version.clone(),
                            region: instance.region.clone(),
                            tier: instance.tier.clone(),
                            availability: cloud_sql_availability_label(instance.availability),
                            connectivity: endpoint_visibility_label(instance.connectivity),
                            status: cloud_sql_status_label(&instance.status),
                            status_semantic: StatusSemantic::from_cloud_sql_status(
                                &instance.status,
                            ),
                        })
                        .collect::<Vec<_>>();
                    let visible_rows = rows.len();
                    (
                        ResourceListProjection::CloudSql(rows),
                        total_rows,
                        visible_rows,
                        None,
                    )
                }
                ResourcesViewMode::Detail => (
                    ResourceListProjection::CloudSql(Vec::new()),
                    total_rows,
                    state.visible_cloud_sql_instance_count(),
                    Some(ResourceDetailProjection::CloudSql(
                        CloudSqlDetailProjection {
                            target_name: state
                                .cloud_sql_detail_target_name()
                                .unwrap_or("selected instance")
                                .to_string(),
                            active_tab: state.active_cloud_sql_detail_tab(),
                            scroll: state.detail_scroll(),
                            detail: state.cloud_sql_detail(),
                            error: state.cloud_sql_detail_error(),
                        },
                    )),
                ),
            }
        }
        ResourceServiceKind::CloudRunServices => {
            let total_rows = state.cloud_run_services().len();
            match view_mode {
                ResourcesViewMode::List => {
                    let rows = state
                        .visible_cloud_run_services()
                        .into_iter()
                        .map(|service| CloudRunServiceProjection {
                            name: service.name.clone(),
                            status: sanitize_display_text(&service.status.label()),
                            status_semantic: StatusSemantic::from_cloud_run_status(&service.status),
                            region: service.region.clone(),
                            url: service.url.clone(),
                            latest_ready_revision: service.latest_ready_revision.clone(),
                            traffic_summary: service.traffic_summary.clone(),
                        })
                        .collect::<Vec<_>>();
                    let visible_rows = rows.len();
                    (
                        ResourceListProjection::CloudRun(rows),
                        total_rows,
                        visible_rows,
                        None,
                    )
                }
                ResourcesViewMode::Detail => (
                    ResourceListProjection::CloudRun(Vec::new()),
                    total_rows,
                    state.visible_cloud_run_service_count(),
                    Some(ResourceDetailProjection::CloudRun(
                        CloudRunDetailProjection {
                            target_name: state
                                .cloud_run_detail_target_name()
                                .unwrap_or("selected service")
                                .to_string(),
                            active_tab: state.active_cloud_run_detail_tab(),
                            scroll: state.detail_scroll(),
                            selected_revision: state.selected_cloud_run_revision_index(),
                            detail: state.cloud_run_detail(),
                            error: state.cloud_run_detail_error(),
                        },
                    )),
                ),
            }
        }
        ResourceServiceKind::CloudFunctions => {
            let total_rows = state.cloud_functions().len();
            let rows = state
                .visible_cloud_functions()
                .into_iter()
                .map(cloud_function_row)
                .collect::<Vec<_>>();
            let visible_rows = rows.len();
            // No detail pane in this slice, so both view modes project the
            // list — `is_showing_detail` is `false` for this service, and
            // `view_mode` cannot be `Detail` while it is.
            (
                ResourceListProjection::CloudFunctions(rows),
                total_rows,
                visible_rows,
                None,
            )
        }
        ResourceServiceKind::PersistentDisks => {
            let total_rows = state.persistent_disks().len();
            let rows = state
                .visible_persistent_disks()
                .into_iter()
                .map(persistent_disk_row)
                .collect::<Vec<_>>();
            let visible_rows = rows.len();
            // No detail pane in this slice, so both view modes project the
            // list — `is_showing_detail` is `false` for this service, and
            // `view_mode` cannot be `Detail` while it is.
            (
                ResourceListProjection::PersistentDisks(rows),
                total_rows,
                visible_rows,
                None,
            )
        }
        ResourceServiceKind::SecretManagerSecrets => {
            let total_rows = state.secrets().len();
            match view_mode {
                ResourcesViewMode::List => {
                    let rows = state
                        .visible_secrets()
                        .into_iter()
                        .map(secret_row)
                        .collect::<Vec<_>>();
                    let visible_rows = rows.len();
                    (
                        ResourceListProjection::Secrets(rows),
                        total_rows,
                        visible_rows,
                        None,
                    )
                }
                ResourcesViewMode::Detail => (
                    ResourceListProjection::Secrets(Vec::new()),
                    total_rows,
                    state.visible_secret_count(),
                    Some(ResourceDetailProjection::Secrets(SecretDetailProjection {
                        target_name: state
                            .secret_detail_target_name()
                            .unwrap_or("selected secret")
                            .to_string(),
                        active_tab: state.active_secret_detail_tab(),
                        scroll: state.detail_scroll(),
                        detail: state.secret_detail(),
                        error: state.secret_detail_error(),
                    })),
                ),
            }
        }
        // List-only in this slice: no describe, no detail pane, so the
        // projection never carries one (see [`PubSubTopics`]).
        ResourceServiceKind::PubSubTopics => {
            let total_rows = state.pubsub_topics().len();
            let rows = match view_mode {
                ResourcesViewMode::List => state
                    .visible_pubsub_topics()
                    .into_iter()
                    .map(pubsub_topic_row)
                    .collect::<Vec<_>>(),
                ResourcesViewMode::Detail => Vec::new(),
            };
            let visible_rows = rows.len();
            (
                ResourceListProjection::PubSubTopics(rows),
                total_rows,
                visible_rows,
                None,
            )
        }
        ResourceServiceKind::VpcNetworks => {
            let total_rows = state.vpc_networks().len();
            let rows = match view_mode {
                ResourcesViewMode::List => state
                    .visible_vpc_networks()
                    .into_iter()
                    .map(vpc_network_row)
                    .collect::<Vec<_>>(),
                ResourcesViewMode::Detail => Vec::new(),
            };
            let visible_rows = rows.len();
            (
                ResourceListProjection::VpcNetworks(rows),
                total_rows,
                visible_rows,
                None,
            )
        }
        ResourceServiceKind::VpcSubnets => {
            let total_rows = state.vpc_subnets().len();
            let rows = match view_mode {
                ResourcesViewMode::List => state
                    .visible_vpc_subnets()
                    .into_iter()
                    .map(vpc_subnet_row)
                    .collect::<Vec<_>>(),
                ResourcesViewMode::Detail => Vec::new(),
            };
            let visible_rows = rows.len();
            (
                ResourceListProjection::VpcSubnets(rows),
                total_rows,
                visible_rows,
                None,
            )
        }
        ResourceServiceKind::FirewallRules => {
            let total_rows = state.firewall_rules().len();
            let rows = match view_mode {
                ResourcesViewMode::List => state
                    .visible_firewall_rules()
                    .into_iter()
                    .map(firewall_rule_row)
                    .collect::<Vec<_>>(),
                ResourcesViewMode::Detail => Vec::new(),
            };
            let visible_rows = rows.len();
            (
                ResourceListProjection::FirewallRules(rows),
                total_rows,
                visible_rows,
                None,
            )
        }
        // List-only, like topics: no describe, no detail pane, so the
        // projection never carries one (see [`PubSubSubscriptions`]).
        ResourceServiceKind::PubSubSubscriptions => {
            let total_rows = state.pubsub_subscriptions().len();
            let rows = match view_mode {
                ResourcesViewMode::List => state
                    .visible_pubsub_subscriptions()
                    .into_iter()
                    .map(pubsub_subscription_row)
                    .collect::<Vec<_>>(),
                ResourcesViewMode::Detail => Vec::new(),
            };
            let visible_rows = rows.len();
            (
                ResourceListProjection::PubSubSubscriptions(rows),
                total_rows,
                visible_rows,
                None,
            )
        }
        // List-only, like Pub/Sub: no describe, no detail pane, so the
        // projection never carries one (see `BigQuery`).
        // Three lists behind one service kind: the entered table's columns when
        // a table has been entered, the entered dataset's tables when a dataset
        // has been, the datasets otherwise. Every count here is the entered
        // view's own — a header reporting the datasets' row total while another
        // list is on screen would attribute one list's size to the other.
        ResourceServiceKind::BigQueryDatasets if state.bigquery_view() == BigQueryView::Schema => {
            let total_rows = state.bigquery_schema_fields().len();
            let rows = state
                .visible_bigquery_schema_fields()
                .into_iter()
                .map(bigquery_schema_field_row)
                .collect::<Vec<_>>();
            let visible_rows = rows.len();
            (
                ResourceListProjection::BigQuerySchema(rows),
                total_rows,
                visible_rows,
                None,
            )
        }
        ResourceServiceKind::BigQueryDatasets if state.bigquery_view() == BigQueryView::Tables => {
            let total_rows = state.bigquery_tables().len();
            let rows = state
                .visible_bigquery_tables()
                .into_iter()
                .map(bigquery_table_row)
                .collect::<Vec<_>>();
            let visible_rows = rows.len();
            (
                ResourceListProjection::BigQueryTables(rows),
                total_rows,
                visible_rows,
                None,
            )
        }
        ResourceServiceKind::BigQueryDatasets => {
            let total_rows = state.bigquery_datasets().len();
            let rows = match view_mode {
                ResourcesViewMode::List => state
                    .visible_bigquery_datasets()
                    .into_iter()
                    .map(bigquery_dataset_row)
                    .collect::<Vec<_>>(),
                ResourcesViewMode::Detail => Vec::new(),
            };
            let visible_rows = rows.len();
            (
                ResourceListProjection::BigQueryDatasets(rows),
                total_rows,
                visible_rows,
                None,
            )
        }
        ResourceServiceKind::IamServiceAccounts => {
            let total_rows = state.service_accounts().len();
            let rows = match view_mode {
                ResourcesViewMode::List => state
                    .visible_service_accounts()
                    .into_iter()
                    .map(service_account_row)
                    .collect::<Vec<_>>(),
                ResourcesViewMode::Detail => Vec::new(),
            };
            let visible_rows = rows.len();
            (
                ResourceListProjection::IamServiceAccounts(rows),
                total_rows,
                visible_rows,
                None,
            )
        }
        ResourceServiceKind::IamPolicy => {
            let total_rows = state.policy_members().len();
            let rows = match view_mode {
                ResourcesViewMode::List => state
                    .visible_policy_members()
                    .into_iter()
                    .map(policy_member_row)
                    .collect::<Vec<_>>(),
                ResourcesViewMode::Detail => Vec::new(),
            };
            let visible_rows = rows.len();
            (
                ResourceListProjection::IamPolicy(rows),
                total_rows,
                visible_rows,
                None,
            )
        }
        ResourceServiceKind::CloudStorage => {
            let storage = state.storage();
            let bucket_list = storage_is_bucket_list(state);
            let (total_rows, list_rows) = if bucket_list {
                let rows = storage
                    .visible_buckets()
                    .into_iter()
                    .map(storage_bucket_row)
                    .collect::<Vec<_>>();
                (storage.buckets().len(), rows)
            } else {
                let prefix = storage.current_prefix();
                let rows = storage
                    .visible_browser_rows()
                    .into_iter()
                    .map(|row| storage_browser_row(row, &prefix))
                    .collect::<Vec<_>>();
                (storage.browser_row_total(), rows)
            };
            match view_mode {
                ResourcesViewMode::List => {
                    let visible_rows = list_rows.len();
                    (
                        ResourceListProjection::Storage(list_rows),
                        total_rows,
                        visible_rows,
                        None,
                    )
                }
                ResourcesViewMode::Detail => {
                    let visible_rows = if bucket_list {
                        storage.visible_bucket_count()
                    } else {
                        storage.visible_browser_row_count()
                    };
                    let target_name = sanitize_display_text(
                        storage.detail_target_name().unwrap_or("selected resource"),
                    );
                    let detail = if storage.view() == StorageView::ObjectDetail {
                        ResourceDetailProjection::Storage(StorageDetailProjection::Object(
                            StorageObjectDetailProjection {
                                target_name,
                                active_tab: storage.object_detail_tab(),
                                scroll: storage.detail_scroll(),
                                detail: storage.object_detail(),
                                error: storage.object_detail_error(),
                            },
                        ))
                    } else {
                        ResourceDetailProjection::Storage(StorageDetailProjection::Bucket(
                            StorageBucketDetailProjection {
                                target_name,
                                active_tab: storage.bucket_detail_tab(),
                                scroll: storage.detail_scroll(),
                                detail: storage.bucket_detail(),
                                error: storage.bucket_detail_error(),
                            },
                        ))
                    };
                    (
                        ResourceListProjection::Storage(Vec::new()),
                        total_rows,
                        visible_rows,
                        Some(detail),
                    )
                }
            }
        }
    };

    // Pagination is currently only meaningful for the Cloud Storage object
    // browser (not the bucket list, and not the other services), which is
    // the only surface with an on-demand `NextPage` action.
    let (has_more, page_index) = match service {
        ResourceServiceKind::CloudStorage if !storage_is_bucket_list(state) => {
            let storage = state.storage();
            (storage.browser_has_more(), storage.browser_page_index())
        }
        _ => (false, 0),
    };

    // Classified list-load problems are Cloud Storage only: a bucket-list
    // failure surfaces `storage.problem()`, an object-browser first-page
    // failure surfaces `storage.browser_problem()`, and a page-append
    // (`NextPage`) failure is a separate non-destructive banner that never
    // clears already-loaded rows.
    let (storage_problem, partial_page_error) = match service {
        ResourceServiceKind::CloudStorage => {
            let storage = state.storage();
            if storage_is_bucket_list(state) {
                let problem = storage.problem().map(|problem| StorageProblemProjection {
                    kind: problem.kind,
                    message: sanitize_display_text(&problem.message),
                    requester_pays: storage.problem_requester_pays(),
                });
                (problem, None)
            } else {
                let problem = storage
                    .browser_problem()
                    .map(|problem| StorageProblemProjection {
                        kind: problem.kind,
                        message: sanitize_display_text(&problem.message),
                        requester_pays: storage.browser_requester_pays(),
                    });
                let partial_page_error = storage
                    .browser_partial_page_error()
                    .map(sanitize_display_text);
                (problem, partial_page_error)
            }
        }
        _ => (None, None),
    };

    let selected_visible = state.selected_visible_index_for_visible_rows(visible_rows);
    // Matched on the view mode first and then exhaustively over every list
    // variant, rather than through a catch-all. The `_` this replaces was
    // absorbing more than the detail mode it was written for: a list variant
    // added without a model here compiled and silently projected no table,
    // which is what left the schema view with rows the renderer could not
    // paint. A variant added now is a compiler error instead.
    let table =
        match view_mode {
            // The detail pane is painted from `detail`; the list behind it is not
            // on screen, so it builds no model.
            ResourcesViewMode::Detail => None,
            ResourcesViewMode::List => {
                match &list {
                    ResourceListProjection::Compute(rows) => Some(compute_table_model(
                        state,
                        rows,
                        total_rows,
                        selected_visible,
                    )),
                    ResourceListProjection::Gke(rows) => {
                        Some(gke_table_model(state, rows, total_rows, selected_visible))
                    }
                    ResourceListProjection::CloudSql(rows) => Some(cloud_sql_table_model(
                        state,
                        rows,
                        total_rows,
                        selected_visible,
                    )),
                    ResourceListProjection::CloudRun(rows) => Some(cloud_run_table_model(
                        state,
                        rows,
                        total_rows,
                        selected_visible,
                    )),
                    ResourceListProjection::Storage(rows) => Some(storage_table_model(
                        state,
                        rows,
                        total_rows,
                        selected_visible,
                    )),
                    ResourceListProjection::Secrets(rows) => Some(secrets_table_model(
                        state,
                        rows,
                        total_rows,
                        selected_visible,
                    )),
                    ResourceListProjection::CloudFunctions(rows) => Some(
                        cloud_functions_table_model(state, rows, total_rows, selected_visible),
                    ),
                    ResourceListProjection::PersistentDisks(rows) => Some(
                        persistent_disks_table_model(state, rows, total_rows, selected_visible),
                    ),
                    ResourceListProjection::PubSubTopics(rows) => Some(pubsub_topics_table_model(
                        state,
                        rows,
                        total_rows,
                        selected_visible,
                    )),
                    ResourceListProjection::PubSubSubscriptions(rows) => Some(
                        pubsub_subscriptions_table_model(state, rows, total_rows, selected_visible),
                    ),
                    ResourceListProjection::VpcNetworks(rows) => Some(vpc_networks_table_model(
                        state,
                        rows,
                        total_rows,
                        selected_visible,
                    )),
                    ResourceListProjection::VpcSubnets(rows) => Some(vpc_subnets_table_model(
                        state,
                        rows,
                        total_rows,
                        selected_visible,
                    )),
                    ResourceListProjection::FirewallRules(rows) => Some(
                        firewall_rules_table_model(state, rows, total_rows, selected_visible),
                    ),
                    ResourceListProjection::BigQueryDatasets(rows) => Some(
                        bigquery_datasets_table_model(state, rows, total_rows, selected_visible),
                    ),
                    ResourceListProjection::BigQueryTables(rows) => Some(
                        bigquery_tables_table_model(rows, total_rows, selected_visible),
                    ),
                    ResourceListProjection::BigQuerySchema(rows) => Some(
                        bigquery_schema_table_model(rows, total_rows, selected_visible),
                    ),
                    ResourceListProjection::IamServiceAccounts(rows) => Some(
                        service_accounts_table_model(state, rows, total_rows, selected_visible),
                    ),
                    ResourceListProjection::IamPolicy(rows) => Some(iam_policy_table_model(
                        state,
                        rows,
                        total_rows,
                        selected_visible,
                    )),
                }
            }
        };

    let status = workspace_status(state);

    ResourceWorkspaceProjection {
        service,
        service_label: service.label(),
        status,
        load_progress: load_progress_projection(state, status),
        load_error: load_error_projection(state, status),
        context_label: state.active_context_label(),
        view_mode,
        filter: ResourceFilterProjection {
            active: state.is_filter_input_active(),
            draft: state.filter_draft().to_string(),
            applied: state.filter_applied().to_string(),
            // Both accessors reparse the filter text -- `filter_error` no
            // longer recompiles it, it reads the compile error off the cache --
            // so each is called exactly once per projection and the value is
            // carried into the render layer from here.
            mode_label: state.filter_mode().label(),
            error: state.filter_error(),
        },
        filter_scope: state.filter_scope(),
        sort_label: state.sort_label(),
        total_rows,
        visible_rows,
        selected_visible,
        wide_columns: state.is_wide_columns(),
        has_more,
        page_index,
        storage_problem,
        partial_page_error,
        list,
        detail,
        table,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Render-facing summary of the currently highlighted resource row, built
/// only from already-loaded summary fields (no detail/describe data).
pub struct ResourcePreviewProjection {
    pub title: String,
    pub lines: Vec<(String, String)>,
}

/// Projects the highlighted resource row's summary fields for preview
/// rendering. Returns `None` when no row is highlighted.
pub fn project_resource_preview(resources: &ResourcesState) -> Option<ResourcePreviewProjection> {
    match resources.active_service() {
        ResourceServiceKind::ComputeInstances => {
            let instance = resources.selected_instance()?;
            Some(ResourcePreviewProjection {
                title: instance.name.clone(),
                lines: vec![
                    (
                        "Status".to_string(),
                        sanitize_display_text(&instance.status.label()),
                    ),
                    ("Zone".to_string(), instance.zone.clone()),
                    ("Machine Type".to_string(), instance.machine_type.clone()),
                    (
                        "Internal IP".to_string(),
                        instance
                            .internal_ip
                            .clone()
                            .unwrap_or_else(|| "-".to_string()),
                    ),
                    (
                        "External IP".to_string(),
                        instance
                            .external_ip
                            .clone()
                            .unwrap_or_else(|| "-".to_string()),
                    ),
                ],
            })
        }
        ResourceServiceKind::GkeClusters => {
            let cluster = resources.selected_gke_cluster()?;
            Some(ResourcePreviewProjection {
                title: cluster.name.clone(),
                lines: vec![
                    (
                        "Status".to_string(),
                        sanitize_display_text(&cluster.status.label()),
                    ),
                    ("Location".to_string(), cluster.location.clone()),
                    (
                        "Version".to_string(),
                        cluster
                            .current_master_version
                            .clone()
                            .unwrap_or_else(|| "-".to_string()),
                    ),
                    (
                        "Nodes".to_string(),
                        cluster
                            .current_node_count
                            .map(|n| n.to_string())
                            .unwrap_or_else(|| "-".to_string()),
                    ),
                    (
                        "Endpoint".to_string(),
                        endpoint_visibility_label(cluster.endpoint_visibility),
                    ),
                ],
            })
        }
        ResourceServiceKind::CloudSql => {
            let instance = resources.selected_cloud_sql_instance()?;
            Some(ResourcePreviewProjection {
                title: instance.name.clone(),
                lines: vec![
                    (
                        "Status".to_string(),
                        cloud_sql_status_label(&instance.status),
                    ),
                    (
                        "Engine".to_string(),
                        columns::cloud_sql_engine_cell(
                            &instance.engine,
                            &instance.database_version,
                        ),
                    ),
                    ("Region".to_string(), instance.region.clone()),
                    ("Tier".to_string(), instance.tier.clone()),
                    (
                        "Availability".to_string(),
                        cloud_sql_availability_label(instance.availability),
                    ),
                    (
                        "Connectivity".to_string(),
                        endpoint_visibility_label(instance.connectivity),
                    ),
                ],
            })
        }
        ResourceServiceKind::CloudRunServices => {
            let service = resources.selected_cloud_run_service()?;
            Some(ResourcePreviewProjection {
                title: service.name.clone(),
                lines: vec![
                    (
                        "Status".to_string(),
                        sanitize_display_text(&service.status.label()),
                    ),
                    ("Region".to_string(), service.region.clone()),
                    (
                        "URL".to_string(),
                        service.url.clone().unwrap_or_else(|| "-".to_string()),
                    ),
                    (
                        "Latest Revision".to_string(),
                        service
                            .latest_ready_revision
                            .clone()
                            .unwrap_or_else(|| "-".to_string()),
                    ),
                    ("Traffic".to_string(), service.traffic_summary.clone()),
                ],
            })
        }
        ResourceServiceKind::PersistentDisks => {
            let disk = resources.selected_persistent_disk()?;
            // The whole row, because this preview is the surface a list-only
            // service has instead of a detail pane. "Attached" gets a line of
            // its own: a disk with four users is unreadable in a table cell,
            // and those names are exactly what an operator checking a badge
            // wants to see.
            Some(ResourcePreviewProjection {
                title: sanitize_display_text(&disk.name),
                lines: vec![
                    (
                        "Location".to_string(),
                        sanitize_display_text(&disk.location),
                    ),
                    (
                        "Status".to_string(),
                        sanitize_display_text(&disk.status.label()),
                    ),
                    ("Size".to_string(), disk_size_label(disk.size_gb)),
                    ("Type".to_string(), sanitize_display_text(&disk.disk_type)),
                    (
                        "Attached".to_string(),
                        if disk.users.is_empty() {
                            "-".to_string()
                        } else {
                            sanitize_display_text(&disk_attached_label(&disk.users))
                        },
                    ),
                    (
                        "Orphan".to_string(),
                        if disk.is_orphan() { "yes" } else { "no" }.to_string(),
                    ),
                ],
            })
        }
        ResourceServiceKind::CloudFunctions => {
            let function = resources.selected_cloud_function()?;
            // The whole row, because this preview is the surface a list-only
            // service has instead of a detail pane: the URL gets a line of its
            // own rather than a table cell competing with five other columns.
            Some(ResourcePreviewProjection {
                title: sanitize_display_text(&function.name),
                lines: vec![
                    (
                        "Region".to_string(),
                        sanitize_display_text(&function.region),
                    ),
                    (
                        "State".to_string(),
                        sanitize_display_text(&function.state.label()),
                    ),
                    (
                        "Generation".to_string(),
                        sanitize_display_text(&function.environment.label()),
                    ),
                    (
                        "Updated".to_string(),
                        function
                            .update_time
                            .as_deref()
                            .map(sanitize_display_text)
                            .unwrap_or_else(|| "-".to_string()),
                    ),
                    (
                        "URL".to_string(),
                        function
                            .url
                            .as_deref()
                            .map(sanitize_display_text)
                            .unwrap_or_else(|| "-".to_string()),
                    ),
                ],
            })
        }
        ResourceServiceKind::PubSubTopics => {
            let topic = resources.selected_pubsub_topic()?;
            Some(ResourcePreviewProjection {
                title: topic.name.clone(),
                lines: vec![
                    (
                        "Subscriptions".to_string(),
                        topic.subscription_count.label(),
                    ),
                    (
                        "Retention".to_string(),
                        topic
                            .message_retention
                            .clone()
                            .unwrap_or_else(|| "-".to_string()),
                    ),
                    (
                        "KMS key".to_string(),
                        topic.kms_key.clone().unwrap_or_else(|| "-".to_string()),
                    ),
                ],
            })
        }
        ResourceServiceKind::VpcNetworks => {
            let network = resources.selected_vpc_network()?;
            Some(ResourcePreviewProjection {
                title: network.name.clone(),
                lines: vec![
                    ("Mode".to_string(), network.subnet_mode.label().to_string()),
                    (
                        "Routing".to_string(),
                        // The pane spells an unreported routing mode rather
                        // than painting the table's dash: a preview has the
                        // width to say which of "no value" it means, and
                        // "unspecified" is a different fact from a value the
                        // API sent that this build does not recognise.
                        match network.routing_mode {
                            VpcRoutingMode::Unspecified => "Unspecified".to_string(),
                            _ => network.routing_mode.label().to_string(),
                        },
                    ),
                    ("Subnets".to_string(), network.subnet_count.to_string()),
                    (
                        // The one field with no column of its own: unbounded
                        // free text, and the only thing on this resource that
                        // says why the network exists.
                        "Description".to_string(),
                        network
                            .description
                            .as_deref()
                            .map(sanitize_display_text)
                            .unwrap_or_else(|| "-".to_string()),
                    ),
                ],
            })
        }
        ResourceServiceKind::VpcSubnets => {
            let subnet = resources.selected_vpc_subnet()?;
            Some(ResourcePreviewProjection {
                // The region leads the title, because the name does not
                // identify the row: a pane headed `default` over a table of
                // 43 rows called `default` says nothing about which one.
                title: format!("{} ({})", subnet.name, subnet.region),
                lines: vec![
                    (
                        "Network".to_string(),
                        sanitize_display_text(&subnet.network),
                    ),
                    (
                        "Range".to_string(),
                        sanitize_display_text(&subnet.ip_cidr_range),
                    ),
                    (
                        "Gateway".to_string(),
                        subnet
                            .gateway_address
                            .as_deref()
                            .map(sanitize_display_text)
                            .unwrap_or_else(|| "-".to_string()),
                    ),
                    (
                        "Private Google access".to_string(),
                        private_google_access_label(subnet).to_string(),
                    ),
                ],
            })
        }
        ResourceServiceKind::FirewallRules => {
            let rule = resources.selected_firewall_rule()?;
            Some(ResourcePreviewProjection {
                // The disabled state leads the title as well as badging the
                // row, because the pane is where an operator reads the rule in
                // full and it is the fact that changes what everything under
                // it means.
                title: if rule.disabled {
                    format!("{} ({DISABLED_BADGE})", rule.name)
                } else {
                    rule.name.clone()
                },
                lines: vec![
                    ("Network".to_string(), sanitize_display_text(&rule.network)),
                    (
                        "Direction".to_string(),
                        sanitize_display_text(&rule.direction.label()),
                    ),
                    (
                        "Action".to_string(),
                        // Spelled out where the column abbreviates to nothing:
                        // a rule the parser could not read says so here rather
                        // than showing the dash and leaving an operator to
                        // guess which way it failed.
                        match rule.action {
                            FirewallAction::Unspecified => {
                                "Unspecified (the rule carried neither allowed nor denied)"
                                    .to_string()
                            }
                            other => other.label().to_string(),
                        },
                    ),
                    (
                        "Priority".to_string(),
                        rule.priority
                            .map(|priority| priority.to_string())
                            .unwrap_or_else(|| "-".to_string()),
                    ),
                    (
                        "Targets".to_string(),
                        sanitize_display_text(&firewall_targets_label(rule)),
                    ),
                    (
                        "Source/dest".to_string(),
                        firewall_source_or_dest_label(rule)
                            .as_deref()
                            .map(sanitize_display_text)
                            .unwrap_or_else(|| "-".to_string()),
                    ),
                ],
            })
        }
        ResourceServiceKind::PubSubSubscriptions => {
            let subscription = resources.selected_pubsub_subscription()?;
            Some(ResourcePreviewProjection {
                title: subscription.name.clone(),
                lines: vec![
                    (
                        "Topic".to_string(),
                        sanitize_display_text(&parent_topic_label(subscription)),
                    ),
                    (
                        "Delivery".to_string(),
                        // The endpoint is worth the preview's width even though
                        // it has no column: a push subscription in
                        // `RESOURCE_ERROR` is usually a broken endpoint, and
                        // this is the only place it is legible.
                        match subscription.delivery.endpoint() {
                            Some(endpoint) => {
                                format!("Push -> {}", sanitize_display_text(endpoint))
                            }
                            None => "Pull".to_string(),
                        },
                    ),
                    (
                        "State".to_string(),
                        // Sanitized because `PubSubSubscriptionState::Unrecognized`
                        // carries whatever string the API returned, verbatim, and
                        // this is the one field on the pane that is not a value
                        // this binary chose. The row builder sanitizes it for the
                        // same reason; the two must not disagree.
                        sanitize_display_text(&subscription.state.label()),
                    ),
                    (
                        "Ack deadline".to_string(),
                        subscription
                            .ack_deadline_seconds
                            .map(|seconds| format!("{seconds}s"))
                            .unwrap_or_else(|| "-".to_string()),
                    ),
                    (
                        "Retention".to_string(),
                        subscription
                            .message_retention
                            .clone()
                            .unwrap_or_else(|| "-".to_string()),
                    ),
                    (
                        "Dead letter".to_string(),
                        subscription
                            .dead_letter
                            .as_ref()
                            .map(|policy| {
                                let topic = sanitize_display_text(
                                    &dead_letter_label(subscription).unwrap_or_default(),
                                );
                                match policy.max_delivery_attempts {
                                    Some(attempts) => {
                                        format!("{topic} after {attempts} attempts")
                                    }
                                    None => topic,
                                }
                            })
                            .unwrap_or_else(|| "-".to_string()),
                    ),
                ],
            })
        }
        // The preview shows what the three columns do not: the friendly name
        // an operator knows the dataset by — which has no column deliberately,
        // for the reason `BIGQUERY_DATASET_COLUMNS` gives — and the project
        // that owns it, worth naming even on a single-project table because a
        // dataset id is only unique inside one. The location repeats the
        // column, since a preview that named only the absent facts would read
        // as a fragment; the labels do not, because they have a column of
        // their own and would be the widest line here for no gain.
        // The tables view previews the table the cursor is on, not the dataset
        // it is inside: the pane describes the highlighted row, and the
        // dataset is already named in the pane title beside it. Labels are
        // here rather than in a column for the reason
        // `BIGQUERY_TABLE_COLUMNS` records — a `dataplex-*` set runs past 300
        // characters — and the preview is where a long value costs the table
        // nothing.
        // The schema view previews the column the cursor is on. `DESCRIPTION`
        // is the reason the pane earns its place here: a field's description is
        // free text and the one cell of this row that runs long, so the list
        // shows the short facts and the pane shows the sentence.
        ResourceServiceKind::BigQueryDatasets
            if resources.bigquery_view() == BigQueryView::Schema =>
        {
            let field = resources.selected_bigquery_schema_field()?;
            Some(ResourcePreviewProjection {
                title: sanitize_display_text(&field.path),
                lines: vec![
                    ("Type".to_string(), sanitize_display_text(&field.field_type)),
                    ("Mode".to_string(), sanitize_display_text(&field.mode)),
                    (
                        "Description".to_string(),
                        field
                            .description
                            .as_deref()
                            .map_or_else(|| "-".to_string(), sanitize_display_text),
                    ),
                ],
            })
        }
        ResourceServiceKind::BigQueryDatasets
            if resources.bigquery_view() == BigQueryView::Tables =>
        {
            let table = resources.selected_bigquery_table()?;
            Some(ResourcePreviewProjection {
                title: sanitize_display_text(&table.table_id),
                lines: vec![
                    (
                        "Type".to_string(),
                        sanitize_display_text(table.kind.label()),
                    ),
                    (
                        "Created".to_string(),
                        bigquery_instant_cell(table.created).unwrap_or_else(|| "-".to_string()),
                    ),
                    (
                        "Dataset".to_string(),
                        sanitize_display_text(&table.dataset_id),
                    ),
                    (
                        "Labels".to_string(),
                        sanitize_display_text(&bigquery_labels_cell(&table.labels)),
                    ),
                ],
            })
        }
        ResourceServiceKind::BigQueryDatasets => {
            let dataset = resources.selected_bigquery_dataset()?;
            Some(ResourcePreviewProjection {
                title: dataset.dataset_id.clone(),
                lines: vec![
                    (
                        "Location".to_string(),
                        if dataset.location.is_empty() {
                            "-".to_string()
                        } else {
                            sanitize_display_text(&dataset.location)
                        },
                    ),
                    (
                        "Project".to_string(),
                        sanitize_display_text(&dataset.project_id),
                    ),
                    (
                        "Friendly name".to_string(),
                        dataset
                            .friendly_name
                            .as_deref()
                            // `""` means the same thing as the key being
                            // absent, and it is a shape the captured listing
                            // really has. The parser folds it to `None`, but
                            // the domain type still permits it and a blank
                            // preview line reads as a rendering fault, so the
                            // emptiness is refused here too rather than only
                            // upstream.
                            .filter(|name| !name.is_empty())
                            .map(sanitize_display_text)
                            .unwrap_or_else(|| "-".to_string()),
                    ),
                ],
            })
        }
        ResourceServiceKind::IamServiceAccounts => {
            let account = resources.selected_service_account()?;
            Some(ResourcePreviewProjection {
                title: sanitize_display_text(&account.email),
                lines: vec![
                    (
                        "Display name".to_string(),
                        account
                            .display_name
                            .as_deref()
                            .map(sanitize_display_text)
                            .unwrap_or_else(|| "-".to_string()),
                    ),
                    (
                        "State".to_string(),
                        service_account_state_label(account).to_string(),
                    ),
                    (
                        "Unique ID".to_string(),
                        sanitize_display_text(&account.unique_id),
                    ),
                ],
            })
        }
        ResourceServiceKind::IamPolicy => {
            let member = resources.selected_policy_member()?;
            Some(ResourcePreviewProjection {
                title: sanitize_display_text(&member.raw_member),
                lines: vec![
                    ("Role".to_string(), sanitize_display_text(&member.role)),
                    (
                        "Type".to_string(),
                        sanitize_display_text(member.kind.label()),
                    ),
                    (
                        "Condition".to_string(),
                        member
                            .condition
                            .as_deref()
                            .map(sanitize_display_text)
                            .unwrap_or_else(|| "-".to_string()),
                    ),
                ],
            })
        }
        ResourceServiceKind::SecretManagerSecrets => {
            let secret = resources.selected_secret()?;
            Some(ResourcePreviewProjection {
                title: secret.name.clone(),
                lines: vec![
                    (
                        "Created".to_string(),
                        secret
                            .create_time
                            .clone()
                            .unwrap_or_else(|| "-".to_string()),
                    ),
                    ("Replication".to_string(), secret.replication.clone()),
                    ("Labels".to_string(), secret_labels_cell(secret)),
                ],
            })
        }
        ResourceServiceKind::CloudStorage => {
            let storage = resources.storage();
            if storage_is_bucket_list(resources) {
                let bucket = storage.selected_bucket()?;
                Some(ResourcePreviewProjection {
                    title: sanitize_display_text(&bucket.name),
                    lines: vec![
                        ("Location".to_string(), bucket.location.clone()),
                        ("Class".to_string(), bucket.storage_class.clone()),
                        (
                            "Versioning".to_string(),
                            if bucket.versioning_enabled {
                                "on".to_string()
                            } else {
                                "off".to_string()
                            },
                        ),
                        (
                            "Public access".to_string(),
                            bucket
                                .public_access_prevention
                                .clone()
                                .unwrap_or_else(|| "-".to_string()),
                        ),
                    ],
                })
            } else {
                let prefix = storage.current_prefix();
                let rows = storage.visible_browser_rows();
                let selected = storage.selected_visible_index()?;
                match rows.get(selected)? {
                    StorageBrowserRow::Prefix(item) => Some(ResourcePreviewProjection {
                        title: sanitize_display_text(&storage_display_name(&item.name, &prefix)),
                        lines: vec![("Kind".to_string(), "Folder".to_string())],
                    }),
                    StorageBrowserRow::Object(item) => Some(ResourcePreviewProjection {
                        title: sanitize_display_text(&storage_display_name(&item.key, &prefix)),
                        lines: vec![
                            (
                                "Size".to_string(),
                                item.size_bytes
                                    .map(format_object_size)
                                    .unwrap_or_else(|| "-".to_string()),
                            ),
                            (
                                "Class".to_string(),
                                item.storage_class
                                    .clone()
                                    .unwrap_or_else(|| "-".to_string()),
                            ),
                            (
                                "Updated".to_string(),
                                item.updated.clone().unwrap_or_else(|| "-".to_string()),
                            ),
                        ],
                    }),
                }
            }
        }
    }
}

/// Projects the nested Cloud Run revision drill-down for the detail renderer.
///
/// Returns `None` unless a revision view is active. Exposed as a standalone
/// projection (mirroring [`project_resource_preview`]) because the revision
/// view is Cloud-Run-specific and layers on top of the shared detail shell.
pub fn project_cloud_run_revision(
    resources: &ResourcesState,
) -> Option<CloudRunRevisionProjection<'_>> {
    if !resources.is_showing_cloud_run_revision() {
        return None;
    }
    Some(CloudRunRevisionProjection {
        target_name: resources
            .cloud_run_revision_target()
            .map(|revision| revision.name.clone())
            .unwrap_or_else(|| "selected revision".to_string()),
        active_tab: resources.active_cloud_run_revision_tab(),
        scroll: resources.cloud_run_revision_scroll(),
        detail: resources.cloud_run_revision_detail(),
        error: resources.cloud_run_revision_detail_error(),
    })
}

fn workspace_status(state: &ResourcesState) -> ResourceWorkspaceStatus {
    let fresh_rows = state.has_loaded_context() && !state.is_stale();
    match state.active_load_phase() {
        ResourceLoadPhase::Loading { .. } if !fresh_rows => ResourceWorkspaceStatus::Loading,
        ResourceLoadPhase::Failed { .. } if !fresh_rows => ResourceWorkspaceStatus::Failed,
        _ if state.is_stale() => ResourceWorkspaceStatus::Stale,
        _ if state.has_loaded_context() => ResourceWorkspaceStatus::Ready,
        _ => ResourceWorkspaceStatus::NotLoaded,
    }
}

/// Live fetch progress for the active collection, `Some` iff `status` is
/// `Loading`. Elapsed time and the derived spinner frame are computed at
/// projection time from `ResourceLoadPhase::Loading`'s `started_at`, so no
/// frame counter needs to be stored.
fn load_progress_projection(
    state: &ResourcesState,
    status: ResourceWorkspaceStatus,
) -> Option<LoadProgressProjection> {
    if status != ResourceWorkspaceStatus::Loading {
        return None;
    }
    let ResourceLoadPhase::Loading { started_at } = state.active_load_phase() else {
        return None;
    };
    let elapsed = started_at.elapsed();
    let frame_index = (elapsed.as_millis() / 125) as usize % SPINNER_FRAMES.len();
    Some(LoadProgressProjection {
        elapsed_secs: elapsed.as_secs(),
        spinner_frame: SPINNER_FRAMES[frame_index],
    })
}

/// The active collection's `ResourceLoadPhase::Failed` message, `Some` iff
/// `status` is `Failed`.
fn load_error_projection(
    state: &ResourcesState,
    status: ResourceWorkspaceStatus,
) -> Option<String> {
    if status != ResourceWorkspaceStatus::Failed {
        return None;
    }
    let ResourceLoadPhase::Failed { message, .. } = state.active_load_phase() else {
        return None;
    };
    Some(message.clone())
}

fn endpoint_visibility_label(visibility: EndpointVisibility) -> String {
    match visibility {
        EndpointVisibility::Public => "Public".to_string(),
        EndpointVisibility::Private => "Private".to_string(),
        EndpointVisibility::Unknown => "Unknown".to_string(),
    }
}

fn cloud_sql_availability_label(availability: CloudSqlAvailability) -> String {
    availability.label().to_string()
}

fn cloud_sql_status_label(status: &nuvem_domain::gcp::resource::CloudSqlStatus) -> String {
    sanitize_display_text(&status.label())
}

/// Builds one service's header specs and data rows from its catalog and the
/// effective layout in state. The layout is resolved once, here, and feeds
/// both halves, so a reordered or subsetted table cannot pair a header with
/// another column's cell.
fn table_parts_for<R, S>(
    state: &ResourcesState,
    kind: ResourceServiceKind,
    catalog: &[columns::ColumnDef<R, S>],
    rows: &[R],
    marked: impl Fn(&R) -> bool,
) -> (Vec<TableColumn>, Vec<TableRow>) {
    // `kind` and `catalog` are two spellings of the same service, and nothing
    // in the types ties them together: a copy-pasted call that kept the wrong
    // `kind` would resolve another service's layout, and `displayed` would
    // quietly skip the out-of-range indices rather than misalign anything, so
    // the table would just lose columns with no test to notice.
    debug_assert_eq!(
        catalog.len(),
        columns::catalog_meta(kind).len(),
        "{kind:?} was given another service's column catalog"
    );
    let (table_columns, table_rows) =
        columns::table_parts(catalog, state.column_layout(kind), rows.iter(), marked);
    (
        with_display_priorities(state, kind, table_columns),
        table_rows,
    )
}

/// Gives a configured service's columns positional priorities, and leaves an
/// unconfigured service's exactly as its catalog wrote them (design §6).
fn with_display_priorities(
    state: &ResourcesState,
    kind: ResourceServiceKind,
    mut table_columns: Vec<TableColumn>,
) -> Vec<TableColumn> {
    if state.has_configured_columns(kind) {
        columns::apply_display_priorities(&mut table_columns);
    }
    table_columns
}

fn compute_table_model(
    state: &ResourcesState,
    rows: &[ComputeInstanceProjection],
    total_rows: usize,
    selected_visible: Option<usize>,
) -> TableModel {
    let (table_columns, table_rows) = table_parts_for(
        state,
        ResourceServiceKind::ComputeInstances,
        columns::COMPUTE_COLUMNS,
        rows,
        |row| state.is_instance_marked(&row.name, &row.zone),
    );
    TableModel {
        columns: table_columns,
        rows: table_rows,
        selected_row: selected_visible,
        selected_column: state.selected_column(),
        sort: SortSpec {
            column: state.sort_column_index(),
            direction: state.sort_direction(),
        },
        counts: RowCounts {
            filtered: rows.len(),
            total: total_rows,
        },
        show_mark_column: true,
    }
}

/// True when the Cloud Storage service is showing the bucket list (or a bucket
/// detail whose backing list is the bucket list) rather than the object browser.
fn storage_is_bucket_list(state: &ResourcesState) -> bool {
    matches!(
        state.storage().view(),
        StorageView::Buckets | StorageView::BucketDetail
    )
}

/// Returns a browser entry's name relative to the current prefix (folder or
/// object segment), stripping the shared prefix path.
fn storage_display_name(name: &str, current_prefix: &str) -> String {
    name.strip_prefix(current_prefix)
        .unwrap_or(name)
        .to_string()
}

/// Formats an object size in bytes into a compact human-readable string.
fn format_object_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut size = bytes as f64;
    let mut unit = 0;
    while size >= 1024.0 && unit < UNITS.len() - 1 {
        size /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{size:.1} {}", UNITS[unit])
    }
}

fn storage_bucket_row(bucket: &nuvem_domain::gcp::resource::StorageBucket) -> StorageRowProjection {
    StorageRowProjection::Bucket(StorageBucketRowProjection {
        name: sanitize_display_text(&bucket.name),
        location: bucket.location.clone(),
        storage_class: bucket.storage_class.clone(),
        updated: bucket.updated.clone().unwrap_or_default(),
        versioning: if bucket.versioning_enabled {
            "on".to_string()
        } else {
            "off".to_string()
        },
        public_access: bucket
            .public_access_prevention
            .clone()
            .unwrap_or_else(|| "-".to_string()),
    })
}

fn storage_browser_row(row: StorageBrowserRow<'_>, current_prefix: &str) -> StorageRowProjection {
    match row {
        StorageBrowserRow::Prefix(prefix) => {
            StorageRowProjection::Object(StorageObjectRowProjection {
                is_prefix: true,
                name: sanitize_display_text(&storage_display_name(&prefix.name, current_prefix)),
                size: String::new(),
                storage_class: String::new(),
                updated: String::new(),
                encryption: String::new(),
                content_type: String::new(),
                retention: String::new(),
            })
        }
        StorageBrowserRow::Object(object) => {
            StorageRowProjection::Object(StorageObjectRowProjection {
                is_prefix: false,
                name: sanitize_display_text(&storage_display_name(&object.key, current_prefix)),
                size: object
                    .size_bytes
                    .map(format_object_size)
                    .unwrap_or_default(),
                storage_class: object.storage_class.clone().unwrap_or_default(),
                updated: object.updated.clone().unwrap_or_default(),
                encryption: if object.kms_encrypted {
                    "CMEK".to_string()
                } else {
                    "Google".to_string()
                },
                // `content_type` is uploader-controllable metadata (unlike
                // e.g. `storage_class`), so it's sanitized the same way names
                // are before it reaches the table cell.
                content_type: object
                    .content_type
                    .as_deref()
                    .map(sanitize_display_text)
                    .unwrap_or_default(),
                retention: if object.has_retention {
                    "yes".to_string()
                } else {
                    String::new()
                },
            })
        }
    }
}

fn storage_object_columns() -> Vec<TableColumn> {
    vec![
        TableColumn {
            key: "name",
            header: "NAME",
            min_width: 6,
            preferred_width: 32,
            priority: 0,
            align: CellAlign::Left,
            sortable: false,
            fit: ColumnFit::Whole,
        },
        TableColumn {
            key: "size",
            header: "SIZE",
            min_width: 4,
            preferred_width: 10,
            priority: 2,
            align: CellAlign::Right,
            sortable: false,
            fit: ColumnFit::Whole,
        },
        TableColumn {
            key: "class",
            header: "CLASS",
            min_width: 5,
            preferred_width: 12,
            priority: 3,
            align: CellAlign::Left,
            sortable: false,
            fit: ColumnFit::Whole,
        },
        TableColumn {
            key: "updated",
            header: "UPDATED",
            min_width: 8,
            preferred_width: 22,
            priority: 1,
            align: CellAlign::Left,
            sortable: false,
            fit: ColumnFit::Whole,
        },
        TableColumn {
            key: "encryption",
            header: "ENCRYPTION",
            min_width: 6,
            preferred_width: 10,
            priority: 4,
            align: CellAlign::Left,
            sortable: false,
            fit: ColumnFit::Whole,
        },
        TableColumn {
            key: "content-type",
            header: "CONTENT-TYPE",
            min_width: 6,
            preferred_width: 18,
            priority: 5,
            align: CellAlign::Left,
            sortable: false,
            fit: ColumnFit::Whole,
        },
        TableColumn {
            key: "retention",
            header: "RETENTION",
            min_width: 4,
            preferred_width: 9,
            priority: 6,
            align: CellAlign::Left,
            sortable: false,
            fit: ColumnFit::Whole,
        },
    ]
}

/// The object browser's rows, in the fixed order [`storage_object_columns`]
/// heads. Not configurable and not catalog-driven: the browser is a detail
/// mode with no column navigation and no sorting.
fn storage_object_rows(rows: &[StorageRowProjection]) -> Vec<TableRow> {
    rows.iter()
        .filter_map(StorageRowProjection::as_object)
        .map(|object| {
            let name = if object.is_prefix {
                format!("{}/", object.name.trim_end_matches('/'))
            } else {
                object.name.clone()
            };
            TableRow {
                cells: vec![
                    Cell::plain(name),
                    Cell::plain(object.size.clone()),
                    Cell::plain(object.storage_class.clone()),
                    Cell::plain(object.updated.clone()),
                    Cell::plain(object.encryption.clone()),
                    Cell::plain(object.content_type.clone()),
                    Cell::plain(object.retention.clone()),
                ],
                marked: false,
            }
        })
        .collect()
}

fn storage_table_model(
    state: &ResourcesState,
    rows: &[StorageRowProjection],
    total_rows: usize,
    selected_visible: Option<usize>,
) -> TableModel {
    // The bucket list is a configurable table and goes through `table_parts`
    // like the other four services, so its headers and its cells come from one
    // walk over one (catalog, layout) pair. The object browser behind it is a
    // detail mode, not a configurable list: it keeps its hardcoded columns.
    let (table_columns, table_rows) = if storage_is_bucket_list(state) {
        let (specs, bucket_rows) = columns::table_parts(
            columns::STORAGE_BUCKET_COLUMNS,
            state.column_layout(ResourceServiceKind::CloudStorage),
            rows.iter().filter_map(StorageRowProjection::as_bucket),
            |_| false,
        );
        (
            with_display_priorities(state, ResourceServiceKind::CloudStorage, specs),
            bucket_rows,
        )
    } else {
        (storage_object_columns(), storage_object_rows(rows))
    };
    // The list projection builds one kind of row per view, so filtering to the
    // view's own kind above drops nothing.
    debug_assert_eq!(
        table_rows.len(),
        rows.len(),
        "the storage list mixed bucket rows and object rows"
    );
    TableModel {
        columns: table_columns,
        rows: table_rows,
        selected_row: selected_visible,
        selected_column: state.selected_column(),
        sort: SortSpec {
            column: state.sort_column_index(),
            direction: state.sort_direction(),
        },
        counts: RowCounts {
            filtered: rows.len(),
            total: total_rows,
        },
        show_mark_column: false,
    }
}

fn gke_table_model(
    state: &ResourcesState,
    rows: &[GkeClusterProjection],
    total_rows: usize,
    selected_visible: Option<usize>,
) -> TableModel {
    let (table_columns, table_rows) = table_parts_for(
        state,
        ResourceServiceKind::GkeClusters,
        columns::GKE_COLUMNS,
        rows,
        |_| false,
    );
    TableModel {
        columns: table_columns,
        rows: table_rows,
        selected_row: selected_visible,
        selected_column: state.selected_column(),
        sort: SortSpec {
            column: state.sort_column_index(),
            direction: state.sort_direction(),
        },
        counts: RowCounts {
            filtered: rows.len(),
            total: total_rows,
        },
        show_mark_column: false,
    }
}

fn cloud_sql_table_model(
    state: &ResourcesState,
    rows: &[CloudSqlInstanceProjection],
    total_rows: usize,
    selected_visible: Option<usize>,
) -> TableModel {
    let (table_columns, table_rows) = table_parts_for(
        state,
        ResourceServiceKind::CloudSql,
        columns::CLOUD_SQL_COLUMNS,
        rows,
        |row| state.is_cloud_sql_instance_marked(&row.name, &row.region),
    );
    TableModel {
        columns: table_columns,
        rows: table_rows,
        selected_row: selected_visible,
        selected_column: state.selected_column(),
        sort: SortSpec {
            column: state.sort_column_index(),
            direction: state.sort_direction(),
        },
        counts: RowCounts {
            filtered: rows.len(),
            total: total_rows,
        },
        // Cloud SQL rows are markable: the batch actions need the
        // mark column to render, or a mark would be invisible.
        show_mark_column: true,
    }
}

/// Flattens a secret's labels into one `key=value, key=value` cell, or `-` when
/// it has none. Sorted upstream, so the cell is stable across reloads.
/// Labels flattened to one cell, or a dash when there are none.
fn pubsub_labels_cell(topic: &PubSubTopic) -> String {
    if topic.labels.is_empty() {
        return "-".to_string();
    }
    topic
        .labels
        .iter()
        .map(|label| format!("{}={}", label.key, label.value))
        .collect::<Vec<_>>()
        .join(", ")
}

fn vpc_network_row(network: &VpcNetwork) -> VpcNetworkRowProjection {
    VpcNetworkRowProjection {
        name: sanitize_display_text(&network.name),
        // `VpcSubnetMode::label` and `VpcRoutingMode::label` are the single
        // places that decide these spellings, so the cell and the filter
        // cannot disagree about them.
        mode: network.subnet_mode.label().to_string(),
        // An unspecified routing mode becomes `None` here so the cell paints
        // its dash. Sanitised because an unrecognised mode is a string the API
        // chose, not one of ours.
        routing: match network.routing_mode {
            VpcRoutingMode::Unspecified => None,
            _ => Some(sanitize_display_text(&network.routing_mode.label())),
        },
        subnets: network.subnet_count.to_string(),
    }
}

fn firewall_rule_row(rule: &FirewallRule) -> FirewallRuleRowProjection {
    FirewallRuleRowProjection {
        // The badge leads the cell rather than trailing it, for the reason the
        // IAM table's do: the name column is the one that elides first when it
        // is too narrow for its content, and a trailing badge would be the
        // first thing lost on exactly the rows it exists to mark.
        name: if rule.disabled {
            format!("{DISABLED_BADGE} {}", sanitize_display_text(&rule.name))
        } else {
            sanitize_display_text(&rule.name)
        },
        network: sanitize_display_text(&rule.network),
        direction: match rule.direction {
            FirewallDirection::Unspecified => None,
            _ => Some(sanitize_display_text(&rule.direction.label())),
        },
        priority: rule.priority.map(|priority| priority.to_string()),
        // `Unspecified` becomes `None` so the cell paints its dash. Rendering
        // it as an empty `Plain` cell would look the same on a buffer and read
        // as a value, and defaulting it to `ALLOW` would be a claim the
        // document never made.
        action: match rule.action {
            FirewallAction::Unspecified => None,
            other => Some(other.label().to_string()),
        },
        targets: sanitize_display_text(&firewall_targets_label(rule)),
        source_or_dest: firewall_source_or_dest_label(rule)
            .as_deref()
            .map(sanitize_display_text),
        disabled: rule.disabled,
    }
}

fn vpc_subnet_row(subnet: &VpcSubnet) -> VpcSubnetRowProjection {
    VpcSubnetRowProjection {
        name: sanitize_display_text(&subnet.name),
        region: sanitize_display_text(&subnet.region),
        network: sanitize_display_text(&subnet.network),
        range: sanitize_display_text(&subnet.ip_cidr_range),
        gateway: subnet.gateway_address.as_deref().map(sanitize_display_text),
        private_google_access: private_google_access_label(subnet).to_string(),
    }
}

fn pubsub_topic_row(topic: &PubSubTopic) -> PubSubTopicRowProjection {
    PubSubTopicRowProjection {
        name: sanitize_display_text(&topic.name),
        // `SubscriptionCount::label` is the single place that decides what an
        // unavailable count looks like, so the cell and the filter cannot
        // disagree about it.
        subscriptions: topic.subscription_count.label(),
        retention: topic.message_retention.clone(),
        kms_key: topic.kms_key.clone().map(|key| sanitize_display_text(&key)),
        labels: sanitize_display_text(&pubsub_labels_cell(topic)),
    }
}

/// Labels flattened to one cell, or a dash when there are none.
fn pubsub_subscription_labels_cell(subscription: &PubSubSubscription) -> String {
    if subscription.labels.is_empty() {
        return "-".to_string();
    }
    subscription
        .labels
        .iter()
        .map(|label| format!("{}={}", label.key, label.value))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Styling for a subscription's state.
///
/// `Unspecified` is neutral rather than healthy: proto3 JSON omits zero-valued
/// enums, so an absent `state` reads as `Unspecified` — that is a fact the API
/// did not report, not a subscription confirmed well.
fn subscription_status_semantic(state: &PubSubSubscriptionState) -> StatusSemantic {
    match state {
        PubSubSubscriptionState::Active => StatusSemantic::Healthy,
        PubSubSubscriptionState::ResourceError => StatusSemantic::Degraded,
        PubSubSubscriptionState::Unspecified | PubSubSubscriptionState::Unrecognized(_) => {
            StatusSemantic::Neutral
        }
    }
}

fn pubsub_subscription_row(subscription: &PubSubSubscription) -> PubSubSubscriptionRowProjection {
    PubSubSubscriptionRowProjection {
        name: sanitize_display_text(&subscription.name),
        topic: sanitize_display_text(&parent_topic_label(subscription)),
        // `PubSubDelivery::label` and `PubSubSubscriptionState::label` are the
        // single places that decide these spellings, so the cell and the
        // filter cannot disagree about them.
        delivery: subscription.delivery.label().to_string(),
        state: sanitize_display_text(&subscription.state.label()),
        status_semantic: subscription_status_semantic(&subscription.state),
        // Rendered bare, without a `s` suffix: the column is right-aligned and
        // headed `ACK(S)`, and a unit in every cell would cost width the
        // numbers need. An absent deadline stays `None` so the cell paints a
        // dash rather than a `0` the API never returned.
        ack_deadline: subscription
            .ack_deadline_seconds
            .map(|seconds| seconds.to_string()),
        retention: subscription.message_retention.clone(),
        dead_letter: dead_letter_label(subscription).map(|topic| sanitize_display_text(&topic)),
        labels: sanitize_display_text(&pubsub_subscription_labels_cell(subscription)),
    }
}

/// Labels flattened to one cell, or a dash when there are none.
///
/// The pairs arrive key-ordered from the parser's `BTreeMap`, so the cell is
/// stable across reloads and the `k=v` an operator reads is the `k=v` the
/// filter matches.
fn bigquery_labels_cell(labels: &[ComputeKeyValue]) -> String {
    if labels.is_empty() {
        return "-".to_string();
    }
    labels
        .iter()
        .map(|label| format!("{}={}", label.key, label.value))
        .collect::<Vec<_>>()
        .join(", ")
}

fn bigquery_dataset_row(dataset: &BigQueryDataset) -> BigQueryDatasetRowProjection {
    BigQueryDatasetRowProjection {
        name: sanitize_display_text(&dataset.dataset_id),
        // An absent location parses as the empty string, and the empty string
        // is what the cell must paint as a dash — so the emptiness is resolved
        // here, once, rather than by each reader of the row.
        location: Some(dataset.location.as_str())
            .filter(|location| !location.is_empty())
            .map(sanitize_display_text),
        labels: sanitize_display_text(&bigquery_labels_cell(&dataset.labels)),
    }
}

/// A BigQuery epoch-milliseconds field — `creationTime` or
/// `lastModifiedTime` — rendered as the UTC instant it names, or `None` where
/// there was nothing usable to render.
///
/// One function for both, so `CREATED` and `LAST MODIFIED` cannot come to mean
/// two different clocks on one row. UTC and `Z`-suffixed to match the
/// `CREATED` column on the secrets table, which paints the API's own UTC
/// string: two `CREATED` columns in one binary that meant different clocks
/// would be worse than either choice alone.
///
/// `from_timestamp_millis` rejects a value too large to be a date, so a
/// nonsense timestamp dashes rather than panicking or painting a year the API
/// never sent. The steps upstream have already dashed the absent, empty and
/// non-numeric cases — the listing's parse for `created`, and
/// [`TableStat::Unavailable`] for `last_modified`.
///
/// That last case is the one exception to the rule [`bigquery_bytes_cell`]
/// states: a value that arrived normally must not borrow the dash's "nobody
/// answered" meaning, which is why a negative byte count prints its digits.
/// An out-of-range instant has no digits to fall back to — there is no string
/// for a date outside the calendar — so it dashes, and an operator cannot
/// tell it from a refused call. It takes a wire value no BigQuery response
/// carries to reach.
fn bigquery_instant_cell(millis: Option<i64>) -> Option<String> {
    millis
        .and_then(DateTime::from_timestamp_millis)
        .map(|instant| instant.format("%Y-%m-%dT%H:%M:%SZ").to_string())
}

/// A byte count humanised for the `BYTES` column, or `None` where this table's
/// `bq show` did not answer with one.
///
/// [`TableStat::known`] runs first and the formatter only sees a number that
/// arrived, which is the whole ordering: an absent stat must reach the cell's
/// placeholder without any formatter having had an opinion about it.
///
/// `numBytes` is an int64 and [`format_object_size`] takes a `u64`. BigQuery
/// reports no negative byte count, but this parser accepts one: `stat_from_raw`
/// parses the wire string with `i64`, so a malformed response reaches here as
/// `Known(-1)`. It is echoed verbatim rather than dashed — the dash means
/// "nobody answered", and a value that did arrive must not borrow that
/// meaning. `a_negative_count_paints_its_value_rather_than_a_dash_or_a_zero`
/// pins that, because this fallback has no mutant of its own.
fn bigquery_bytes_cell(bytes: TableStat) -> Option<String> {
    bytes
        .known()
        .map(|value| u64::try_from(value).map_or_else(|_| value.to_string(), format_object_size))
}

fn bigquery_table_row(table: &BigQueryTable) -> BigQueryTableRowProjection {
    BigQueryTableRowProjection {
        name: sanitize_display_text(&table.table_id),
        kind: sanitize_display_text(table.kind.label()),
        created: bigquery_instant_cell(table.created),
        // `known()` before `to_string()`, and before any formatting below it:
        // `TableStat::Unavailable` becomes `None` here and the cell paints the
        // dash, where a `0` would claim the table is empty.
        rows: table.rows.known().map(|rows| rows.to_string()),
        bytes: bigquery_bytes_cell(table.bytes),
        last_modified: bigquery_instant_cell(table.last_modified.known()),
    }
}

fn bigquery_schema_field_row(field: &BigQuerySchemaField) -> BigQuerySchemaFieldRowProjection {
    BigQuerySchemaFieldRowProjection {
        name: sanitize_display_text(&field.path),
        field_type: sanitize_display_text(&field.field_type),
        mode: sanitize_display_text(&field.mode),
        description: field.description.as_deref().map(sanitize_display_text),
    }
}

fn service_account_row(account: &ServiceAccount) -> ServiceAccountRowProjection {
    ServiceAccountRowProjection {
        name: sanitize_display_text(&account.email),
        display_name: account.display_name.as_deref().map(sanitize_display_text),
        state: service_account_state_label(account).to_string(),
        unique_id: sanitize_display_text(&account.unique_id),
    }
}

fn policy_member_row(member: &IamPolicyMember) -> IamPolicyRowProjection {
    IamPolicyRowProjection {
        name: sanitize_display_text(&member.raw_member),
        role: sanitize_display_text(&member.role),
        kind: sanitize_display_text(member.kind.label()),
        condition: member.condition.as_deref().map(sanitize_display_text),
        deleted: member.deleted,
        is_public: member.is_public(),
    }
}

fn secret_labels_cell(secret: &Secret) -> String {
    if secret.labels.is_empty() {
        return "-".to_string();
    }
    secret
        .labels
        .iter()
        .map(|label| format!("{}={}", label.key, label.value))
        .collect::<Vec<_>>()
        .join(", ")
}

fn secret_row(secret: &Secret) -> SecretRowProjection {
    SecretRowProjection {
        name: sanitize_display_text(&secret.name),
        created: secret
            .create_time
            .clone()
            .unwrap_or_else(|| "-".to_string()),
        replication: secret.replication.clone(),
        labels: sanitize_display_text(&secret_labels_cell(secret)),
    }
}

fn persistent_disk_row(disk: &PersistentDisk) -> PersistentDiskRowProjection {
    PersistentDiskRowProjection {
        name: sanitize_display_text(&disk.name),
        location: sanitize_display_text(&disk.location),
        status: sanitize_display_text(&disk.status.label()),
        status_semantic: StatusSemantic::from_disk_status(&disk.status),
        orphan: if disk.is_orphan() {
            ORPHAN_BADGE.to_string()
        } else {
            String::new()
        },
        size: disk_size_label(disk.size_gb),
        disk_type: sanitize_display_text(&disk.disk_type),
        attached: sanitize_display_text(&disk_attached_label(&disk.users)),
    }
}

fn persistent_disks_table_model(
    state: &ResourcesState,
    rows: &[PersistentDiskRowProjection],
    total_rows: usize,
    selected_visible: Option<usize>,
) -> TableModel {
    let (table_columns, table_rows) = table_parts_for(
        state,
        ResourceServiceKind::PersistentDisks,
        columns::PERSISTENT_DISK_COLUMNS,
        rows,
        |_| false,
    );
    TableModel {
        columns: table_columns,
        rows: table_rows,
        selected_row: selected_visible,
        selected_column: state.selected_column(),
        sort: SortSpec {
            column: state.sort_column_index(),
            direction: state.sort_direction(),
        },
        counts: RowCounts {
            filtered: rows.len(),
            total: total_rows,
        },
        // Read-only service: no lifecycle actions, so no marks and no gutter
        // for them.
        show_mark_column: false,
    }
}

fn cloud_function_row(function: &CloudFunction) -> CloudFunctionRowProjection {
    CloudFunctionRowProjection {
        name: sanitize_display_text(&function.name),
        region: sanitize_display_text(&function.region),
        state: sanitize_display_text(&function.state.label()),
        status_semantic: StatusSemantic::from_cloud_function_state(&function.state),
        generation: sanitize_display_text(&function.environment.label()),
        updated: function.update_time.as_deref().map(sanitize_display_text),
        url: function.url.as_deref().map(sanitize_display_text),
    }
}

fn cloud_functions_table_model(
    state: &ResourcesState,
    rows: &[CloudFunctionRowProjection],
    total_rows: usize,
    selected_visible: Option<usize>,
) -> TableModel {
    let (table_columns, table_rows) = table_parts_for(
        state,
        ResourceServiceKind::CloudFunctions,
        columns::CLOUD_FUNCTION_COLUMNS,
        rows,
        |_| false,
    );
    TableModel {
        columns: table_columns,
        rows: table_rows,
        selected_row: selected_visible,
        selected_column: state.selected_column(),
        sort: SortSpec {
            column: state.sort_column_index(),
            direction: state.sort_direction(),
        },
        counts: RowCounts {
            filtered: rows.len(),
            total: total_rows,
        },
        // Read-only browsing: no lifecycle actions, so nothing to mark rows
        // for.
        show_mark_column: false,
    }
}

fn pubsub_subscriptions_table_model(
    state: &ResourcesState,
    rows: &[PubSubSubscriptionRowProjection],
    total_rows: usize,
    selected_visible: Option<usize>,
) -> TableModel {
    let (table_columns, table_rows) = table_parts_for(
        state,
        ResourceServiceKind::PubSubSubscriptions,
        columns::PUBSUB_SUBSCRIPTION_COLUMNS,
        rows,
        |_| false,
    );
    TableModel {
        columns: table_columns,
        rows: table_rows,
        selected_row: selected_visible,
        selected_column: state.selected_column(),
        sort: SortSpec {
            column: state.sort_column_index(),
            direction: state.sort_direction(),
        },
        counts: RowCounts {
            filtered: rows.len(),
            total: total_rows,
        },
        // Read-only browsing: no lifecycle actions, so nothing to mark rows
        // for.
        show_mark_column: false,
    }
}

fn bigquery_datasets_table_model(
    state: &ResourcesState,
    rows: &[BigQueryDatasetRowProjection],
    total_rows: usize,
    selected_visible: Option<usize>,
) -> TableModel {
    let (table_columns, table_rows) = table_parts_for(
        state,
        ResourceServiceKind::BigQueryDatasets,
        columns::BIGQUERY_DATASET_COLUMNS,
        rows,
        |_| false,
    );
    TableModel {
        columns: table_columns,
        rows: table_rows,
        selected_row: selected_visible,
        selected_column: state.selected_column(),
        sort: SortSpec {
            column: state.sort_column_index(),
            direction: state.sort_direction(),
        },
        counts: RowCounts {
            filtered: rows.len(),
            total: total_rows,
        },
        // Read-only browsing: no lifecycle actions, so nothing to mark rows
        // for.
        show_mark_column: false,
    }
}

/// The tables table.
///
/// Takes no [`ResourcesState`], unlike every sibling above it, and that is the
/// point: the sort spec and the column cursor on `ResourcesState` belong to
/// the *datasets* collection, which is a different table with different
/// columns. Reading them here would head the tables table with a sort arrow
/// over `TABLE` because the datasets list is sorted by dataset id — a claim
/// about the order of rows that arrive in `bq ls` order and are never sorted.
/// [`NO_COLUMN`] indexes no column, so no header carries an arrow and none is
/// bracketed as selected.
fn bigquery_tables_table_model(
    rows: &[BigQueryTableRowProjection],
    total_rows: usize,
    selected_visible: Option<usize>,
) -> TableModel {
    // The identity layout, written here rather than resolved from state: the
    // tables view is not a configurable service (it has no
    // `ResourceServiceKind`, so `config::columns` cannot name it), and every
    // catalogue column shows.
    let layout: Vec<usize> = (0..columns::BIGQUERY_TABLE_COLUMNS.len()).collect();
    let (table_columns, table_rows) = columns::table_parts(
        columns::BIGQUERY_TABLE_COLUMNS,
        &layout,
        rows.iter(),
        |_| false,
    );
    TableModel {
        columns: table_columns,
        rows: table_rows,
        selected_row: selected_visible,
        selected_column: NO_COLUMN,
        sort: SortSpec {
            column: NO_COLUMN,
            direction: SortDirection::Ascending,
        },
        counts: RowCounts {
            filtered: rows.len(),
            total: total_rows,
        },
        // Read-only browsing: no lifecycle actions, so nothing to mark rows
        // for.
        show_mark_column: false,
    }
}

/// The schema table.
///
/// Takes no [`ResourcesState`] for the reason [`bigquery_tables_table_model`]
/// does not, one level further down: the sort spec and the column cursor on
/// `ResourcesState` belong to the *datasets* collection, and reading them here
/// would head `FIELD` with a sort arrow because the datasets list is sorted by
/// dataset id — a claim about the order of rows that arrive in schema order
/// and are never sorted. [`NO_COLUMN`] indexes no column, so no header carries
/// an arrow and none is bracketed as selected.
fn bigquery_schema_table_model(
    rows: &[BigQuerySchemaFieldRowProjection],
    total_rows: usize,
    selected_visible: Option<usize>,
) -> TableModel {
    // The identity layout, written here rather than resolved from state: the
    // schema view is not a configurable service (it has no
    // `ResourceServiceKind`, so `config::columns` cannot name it), and every
    // catalogue column shows.
    let layout: Vec<usize> = (0..columns::BIGQUERY_SCHEMA_COLUMNS.len()).collect();
    let (table_columns, table_rows) = columns::table_parts(
        columns::BIGQUERY_SCHEMA_COLUMNS,
        &layout,
        rows.iter(),
        |_| false,
    );
    TableModel {
        columns: table_columns,
        rows: table_rows,
        selected_row: selected_visible,
        selected_column: NO_COLUMN,
        sort: SortSpec {
            column: NO_COLUMN,
            direction: SortDirection::Ascending,
        },
        counts: RowCounts {
            filtered: rows.len(),
            total: total_rows,
        },
        // Read-only browsing: no lifecycle actions, so nothing to mark rows
        // for.
        show_mark_column: false,
    }
}

fn vpc_networks_table_model(
    state: &ResourcesState,
    rows: &[VpcNetworkRowProjection],
    total_rows: usize,
    selected_visible: Option<usize>,
) -> TableModel {
    let (table_columns, table_rows) = table_parts_for(
        state,
        ResourceServiceKind::VpcNetworks,
        columns::VPC_NETWORK_COLUMNS,
        rows,
        |_| false,
    );
    TableModel {
        columns: table_columns,
        rows: table_rows,
        selected_row: selected_visible,
        selected_column: state.selected_column(),
        sort: SortSpec {
            column: state.sort_column_index(),
            direction: state.sort_direction(),
        },
        counts: RowCounts {
            filtered: rows.len(),
            total: total_rows,
        },
        // Read-only browsing: no lifecycle actions, so nothing to mark rows
        // for.
        show_mark_column: false,
    }
}

fn firewall_rules_table_model(
    state: &ResourcesState,
    rows: &[FirewallRuleRowProjection],
    total_rows: usize,
    selected_visible: Option<usize>,
) -> TableModel {
    let (table_columns, table_rows) = table_parts_for(
        state,
        ResourceServiceKind::FirewallRules,
        columns::FIREWALL_RULE_COLUMNS,
        rows,
        |_| false,
    );
    TableModel {
        columns: table_columns,
        rows: table_rows,
        selected_row: selected_visible,
        selected_column: state.selected_column(),
        sort: SortSpec {
            column: state.sort_column_index(),
            direction: state.sort_direction(),
        },
        counts: RowCounts {
            filtered: rows.len(),
            total: total_rows,
        },
        // Read-only browsing: nuvem never edits a firewall rule, so there is
        // nothing to mark rows for — and this is the table where a lifecycle
        // action would be the most dangerous button in the application.
        show_mark_column: false,
    }
}

fn vpc_subnets_table_model(
    state: &ResourcesState,
    rows: &[VpcSubnetRowProjection],
    total_rows: usize,
    selected_visible: Option<usize>,
) -> TableModel {
    let (table_columns, table_rows) = table_parts_for(
        state,
        ResourceServiceKind::VpcSubnets,
        columns::VPC_SUBNET_COLUMNS,
        rows,
        |_| false,
    );
    TableModel {
        columns: table_columns,
        rows: table_rows,
        selected_row: selected_visible,
        selected_column: state.selected_column(),
        sort: SortSpec {
            column: state.sort_column_index(),
            direction: state.sort_direction(),
        },
        counts: RowCounts {
            filtered: rows.len(),
            total: total_rows,
        },
        // Read-only browsing: no lifecycle actions, so nothing to mark rows
        // for.
        show_mark_column: false,
    }
}

fn pubsub_topics_table_model(
    state: &ResourcesState,
    rows: &[PubSubTopicRowProjection],
    total_rows: usize,
    selected_visible: Option<usize>,
) -> TableModel {
    let (table_columns, table_rows) = table_parts_for(
        state,
        ResourceServiceKind::PubSubTopics,
        columns::PUBSUB_TOPIC_COLUMNS,
        rows,
        |_| false,
    );
    TableModel {
        columns: table_columns,
        rows: table_rows,
        selected_row: selected_visible,
        selected_column: state.selected_column(),
        sort: SortSpec {
            column: state.sort_column_index(),
            direction: state.sort_direction(),
        },
        counts: RowCounts {
            filtered: rows.len(),
            total: total_rows,
        },
        // Read-only browsing: no lifecycle actions, so nothing to mark rows
        // for.
        show_mark_column: false,
    }
}

fn service_accounts_table_model(
    state: &ResourcesState,
    rows: &[ServiceAccountRowProjection],
    total_rows: usize,
    selected_visible: Option<usize>,
) -> TableModel {
    let (table_columns, table_rows) = table_parts_for(
        state,
        ResourceServiceKind::IamServiceAccounts,
        columns::SERVICE_ACCOUNT_COLUMNS,
        rows,
        |_| false,
    );
    TableModel {
        columns: table_columns,
        rows: table_rows,
        selected_row: selected_visible,
        selected_column: state.selected_column(),
        sort: SortSpec {
            column: state.sort_column_index(),
            direction: state.sort_direction(),
        },
        counts: RowCounts {
            filtered: rows.len(),
            total: total_rows,
        },
        // Read-only browsing: no lifecycle actions, so nothing to mark rows
        // for.
        show_mark_column: false,
    }
}

fn iam_policy_table_model(
    state: &ResourcesState,
    rows: &[IamPolicyRowProjection],
    total_rows: usize,
    selected_visible: Option<usize>,
) -> TableModel {
    let (table_columns, table_rows) = table_parts_for(
        state,
        ResourceServiceKind::IamPolicy,
        columns::IAM_POLICY_COLUMNS,
        rows,
        |_| false,
    );
    TableModel {
        columns: table_columns,
        rows: table_rows,
        selected_row: selected_visible,
        selected_column: state.selected_column(),
        sort: SortSpec {
            column: state.sort_column_index(),
            direction: state.sort_direction(),
        },
        counts: RowCounts {
            filtered: rows.len(),
            total: total_rows,
        },
        // Read-only browsing: no lifecycle actions, so nothing to mark rows
        // for.
        show_mark_column: false,
    }
}

fn secrets_table_model(
    state: &ResourcesState,
    rows: &[SecretRowProjection],
    total_rows: usize,
    selected_visible: Option<usize>,
) -> TableModel {
    let (table_columns, table_rows) = table_parts_for(
        state,
        ResourceServiceKind::SecretManagerSecrets,
        columns::SECRET_COLUMNS,
        rows,
        |_| false,
    );
    TableModel {
        columns: table_columns,
        rows: table_rows,
        selected_row: selected_visible,
        selected_column: state.selected_column(),
        sort: SortSpec {
            column: state.sort_column_index(),
            direction: state.sort_direction(),
        },
        counts: RowCounts {
            filtered: rows.len(),
            total: total_rows,
        },
        // Secret Manager is metadata-browsing only: no lifecycle actions, so
        // nothing to mark rows for.
        show_mark_column: false,
    }
}

fn cloud_run_table_model(
    state: &ResourcesState,
    rows: &[CloudRunServiceProjection],
    total_rows: usize,
    selected_visible: Option<usize>,
) -> TableModel {
    let (table_columns, table_rows) = table_parts_for(
        state,
        ResourceServiceKind::CloudRunServices,
        columns::CLOUD_RUN_COLUMNS,
        rows,
        |_| false,
    );
    TableModel {
        columns: table_columns,
        rows: table_rows,
        selected_row: selected_visible,
        selected_column: state.selected_column(),
        sort: SortSpec {
            column: state.sort_column_index(),
            direction: state.sort_direction(),
        },
        counts: RowCounts {
            filtered: rows.len(),
            total: total_rows,
        },
        show_mark_column: false,
    }
}
