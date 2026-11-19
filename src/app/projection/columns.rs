//! Per-service column catalogs.
//!
//! Each configurable resource service has exactly one catalog listing every
//! column it can show. A catalog entry carries both the column's
//! [`TableColumn`] spec *and* the function that extracts that column's [`Cell`]
//! from a projected row, so a column's header and its cells are the same array
//! element. Both the header row and every data row are produced by one walk
//! over the same slice ([`table_parts`]), which is what lets a later reorder or
//! subset of that slice stay aligned by construction rather than by hand.
//!
//! Two catalogs here are not *configurable* services, and the distinction is
//! worth naming. [`catalog_meta`] and [`identity_layout`] are keyed by
//! [`ResourceServiceKind`], which is what `config::columns` validates a user's
//! column list against, and this file defines sixteen catalogs against
//! fourteen kinds: the odd ones out are [`BIGQUERY_TABLE_COLUMNS`] and
//! [`BIGQUERY_SCHEMA_COLUMNS`], the two BigQuery drill-downs, which have no
//! kind of their own and so can never be reordered or subsetted by
//! configuration. They keep their catalogs here anyway, because what a catalog
//! buys is the property above — header *i* and cell *i* from one array element
//! — and that is worth having whether or not a layout can move them.
//!
//! Several tables in this binary still pair their headers and cells by hand,
//! outside this file and outside the machinery above — the Cloud Storage
//! object browser (`storage_object_columns` / `storage_object_rows`), the
//! tunnels list (`tunnel_columns` / `tunnel_row`), the logs table
//! (`log_columns` against the cells built inline in `log_table_model`) — each
//! a pair of independently maintained lists that a new column has to be added
//! to twice. Deliberately not a count: this sentence has been wrong twice
//! already, once at "the one table" and once at "two", and a number here is a
//! fact about the whole binary maintained in a file that cannot see it. The
//! members are named so the next reader can check the claim without
//! searching, and finding a fourth makes the sentence more true rather than
//! false.

use std::convert::Infallible;
use std::sync::OnceLock;

use nuvem_domain::gcp::resource::ResourceServiceKind;

use crate::app::projection::table::{
    Cell, CellAlign, ColumnFit, StatusSemantic, TableColumn, TableRow,
};
use crate::app::projection::{
    BigQueryDatasetRowProjection, BigQuerySchemaFieldRowProjection, BigQueryTableRowProjection,
    CloudFunctionRowProjection, CloudRunServiceProjection, CloudSqlInstanceProjection,
    ComputeInstanceProjection, FirewallRuleRowProjection, GkeClusterProjection,
    IamPolicyRowProjection, PersistentDiskRowProjection, PubSubSubscriptionRowProjection,
    PubSubTopicRowProjection, SecretRowProjection, ServiceAccountRowProjection,
    StorageBucketRowProjection, VpcNetworkRowProjection, VpcSubnetRowProjection,
};
use crate::app::state::resources::{
    BigQuerySortField, CloudFunctionSortField, CloudRunSortField, CloudSqlInstanceSortField,
    ComputeInstanceSortField, DELETED_BADGE, DiskSortField, FirewallSortField, GkeSortField,
    IamPolicySortField, PUBLIC_BADGE, PubSubSubscriptionSortField, PubSubTopicSortField,
    SecretSortField, ServiceAccountSortField, StorageSortField, VpcNetworkSortField,
    VpcSubnetSortField,
};

/// One column of one service: how it is headed, what it sorts by, and how its
/// cell is rendered for a single projected row.
pub struct ColumnDef<R, S: 'static> {
    pub spec: TableColumn,
    /// The sort field this column sorts by; `None` for a non-sortable column.
    pub sort: Option<S>,
    pub cell: fn(&R) -> Cell,
}

/// Kind-erased catalog entry, for callers that need column names without the
/// row type — config validation and column-layout resolution.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ColumnMeta {
    pub key: &'static str,
    pub sortable: bool,
}

/// The catalog entries a layout shows, in display order.
///
/// An index the catalog does not have is skipped rather than panicking, and
/// every walk of a (catalog, layout) pair goes through here — so headers and
/// cells skip exactly the same entries and cannot desync even on an index
/// no validation produced.
fn displayed<'a, R, S>(
    catalog: &'a [ColumnDef<R, S>],
    layout: &'a [usize],
) -> impl Iterator<Item = &'a ColumnDef<R, S>> {
    layout.iter().filter_map(|&index| catalog.get(index))
}

/// The column specs `layout` shows, in display order.
pub fn column_specs<R, S>(catalog: &[ColumnDef<R, S>], layout: &[usize]) -> Vec<TableColumn> {
    displayed(catalog, layout)
        .map(|def| def.spec.clone())
        .collect()
}

/// One row's cells, in display order — the same order [`column_specs`] emits
/// headers in, from the same catalog and the same layout.
pub fn row_cells<R, S>(catalog: &[ColumnDef<R, S>], layout: &[usize], row: &R) -> Vec<Cell> {
    displayed(catalog, layout)
        .map(|def| (def.cell)(row))
        .collect()
}

/// Builds a table's header specs and its data rows from one catalog slice and
/// one layout, so header *i* and cell *i* always come from the same catalog
/// entry. `marked` answers whether a row is marked for a batch operation — a
/// property of the row, not of any column.
///
/// `rows` is an iterator rather than a slice so a caller whose row vector is an
/// enum over several row types (Cloud Storage: buckets *or* browser entries)
/// can still build both halves here, from one binding, instead of walking the
/// catalog twice by hand.
pub fn table_parts<'r, R: 'r, S>(
    catalog: &[ColumnDef<R, S>],
    layout: &[usize],
    rows: impl IntoIterator<Item = &'r R>,
    marked: impl Fn(&R) -> bool,
) -> (Vec<TableColumn>, Vec<TableRow>) {
    let table_rows = rows
        .into_iter()
        .map(|row| TableRow {
            cells: row_cells(catalog, layout, row),
            marked: marked(row),
        })
        .collect();
    (column_specs(catalog, layout), table_rows)
}

/// The column that tells one row from another. Every catalog has it, a
/// configured list is rejected without it ([`crate::config::columns`]), and
/// [`apply_display_priorities`] pins it so narrow mode can never drop it.
pub const IDENTITY_COLUMN: &str = "name";

/// Derives each column's `priority` from its display position, so narrow mode
/// drops the rightmost column of the list the user chose and protects their
/// leftmost (design §6) — with one exception: the identity column ([`name`])
/// takes priority 0 wherever the user put it, and every other column takes
/// `position + 1`.
///
/// The exception is what keeps a configured list from rendering an unusable
/// table. `plan_table` protects the column with the *smallest* priority value
/// and drops the largest first, so under plain positional priorities a list
/// ending in `name` would lose `name` first: rows no operator can tell apart,
/// while `Enter` and the lifecycle commands still act on the selected one.
/// Shifting the rest by one keeps the user's order deciding the drop order
/// among the columns that may actually be dropped.
///
/// Only for a service whose column list *is* configured: the catalogs' own
/// priorities are deliberately non-positional in places (a storage bucket's
/// `location` sits in display position 1 with priority 2) and an unconfigured
/// table must keep them.
///
/// [`name`]: IDENTITY_COLUMN
pub fn apply_display_priorities(columns: &mut [TableColumn]) {
    for (position, column) in columns.iter_mut().enumerate() {
        column.priority = if column.key == IDENTITY_COLUMN {
            0
        } else {
            // No catalog is anywhere near `u8::MAX` columns long; a layout that
            // somehow were would simply leave its tail equally droppable.
            u8::try_from(position + 1).unwrap_or(u8::MAX)
        };
    }
}

/// Catalog metadata for a configurable service, in catalog order. Derived from
/// the typed catalogs on first use, so it cannot drift from them.
pub fn catalog_meta(kind: ResourceServiceKind) -> &'static [ColumnMeta] {
    fn meta<R, S>(catalog: &[ColumnDef<R, S>]) -> Vec<ColumnMeta> {
        catalog
            .iter()
            .map(|def| ColumnMeta {
                key: def.spec.key,
                sortable: def.spec.sortable,
            })
            .collect()
    }

    static COMPUTE: OnceLock<Vec<ColumnMeta>> = OnceLock::new();
    static GKE: OnceLock<Vec<ColumnMeta>> = OnceLock::new();
    static CLOUD_SQL: OnceLock<Vec<ColumnMeta>> = OnceLock::new();
    static CLOUD_RUN: OnceLock<Vec<ColumnMeta>> = OnceLock::new();
    static STORAGE: OnceLock<Vec<ColumnMeta>> = OnceLock::new();
    static SECRETS: OnceLock<Vec<ColumnMeta>> = OnceLock::new();
    static CLOUD_FUNCTIONS: OnceLock<Vec<ColumnMeta>> = OnceLock::new();
    static PERSISTENT_DISKS: OnceLock<Vec<ColumnMeta>> = OnceLock::new();
    static PUBSUB_TOPICS: OnceLock<Vec<ColumnMeta>> = OnceLock::new();
    static PUBSUB_SUBSCRIPTIONS: OnceLock<Vec<ColumnMeta>> = OnceLock::new();
    static VPC_NETWORKS: OnceLock<Vec<ColumnMeta>> = OnceLock::new();
    static VPC_SUBNETS: OnceLock<Vec<ColumnMeta>> = OnceLock::new();
    static FIREWALL_RULES: OnceLock<Vec<ColumnMeta>> = OnceLock::new();
    static BIGQUERY_DATASETS: OnceLock<Vec<ColumnMeta>> = OnceLock::new();
    static SERVICE_ACCOUNTS: OnceLock<Vec<ColumnMeta>> = OnceLock::new();
    static IAM_POLICY: OnceLock<Vec<ColumnMeta>> = OnceLock::new();

    match kind {
        ResourceServiceKind::ComputeInstances => COMPUTE.get_or_init(|| meta(COMPUTE_COLUMNS)),
        ResourceServiceKind::GkeClusters => GKE.get_or_init(|| meta(GKE_COLUMNS)),
        ResourceServiceKind::CloudSql => CLOUD_SQL.get_or_init(|| meta(CLOUD_SQL_COLUMNS)),
        ResourceServiceKind::CloudRunServices => CLOUD_RUN.get_or_init(|| meta(CLOUD_RUN_COLUMNS)),
        ResourceServiceKind::CloudStorage => STORAGE.get_or_init(|| meta(STORAGE_BUCKET_COLUMNS)),
        ResourceServiceKind::SecretManagerSecrets => SECRETS.get_or_init(|| meta(SECRET_COLUMNS)),
        ResourceServiceKind::CloudFunctions => {
            CLOUD_FUNCTIONS.get_or_init(|| meta(CLOUD_FUNCTION_COLUMNS))
        }
        ResourceServiceKind::PersistentDisks => {
            PERSISTENT_DISKS.get_or_init(|| meta(PERSISTENT_DISK_COLUMNS))
        }
        ResourceServiceKind::PubSubTopics => {
            PUBSUB_TOPICS.get_or_init(|| meta(PUBSUB_TOPIC_COLUMNS))
        }
        ResourceServiceKind::VpcNetworks => VPC_NETWORKS.get_or_init(|| meta(VPC_NETWORK_COLUMNS)),
        ResourceServiceKind::VpcSubnets => VPC_SUBNETS.get_or_init(|| meta(VPC_SUBNET_COLUMNS)),
        ResourceServiceKind::FirewallRules => {
            FIREWALL_RULES.get_or_init(|| meta(FIREWALL_RULE_COLUMNS))
        }
        ResourceServiceKind::PubSubSubscriptions => {
            PUBSUB_SUBSCRIPTIONS.get_or_init(|| meta(PUBSUB_SUBSCRIPTION_COLUMNS))
        }
        ResourceServiceKind::BigQueryDatasets => {
            BIGQUERY_DATASETS.get_or_init(|| meta(BIGQUERY_DATASET_COLUMNS))
        }
        ResourceServiceKind::IamServiceAccounts => {
            SERVICE_ACCOUNTS.get_or_init(|| meta(SERVICE_ACCOUNT_COLUMNS))
        }
        ResourceServiceKind::IamPolicy => IAM_POLICY.get_or_init(|| meta(IAM_POLICY_COLUMNS)),
    }
}

/// A service's layout when nothing is configured: every catalog column, in
/// catalog order. Memoized alongside [`catalog_meta`], so a caller that needs
/// the default layout can borrow one rather than build one per call.
pub fn identity_layout(kind: ResourceServiceKind) -> &'static [usize] {
    static LAYOUTS: OnceLock<Vec<(ResourceServiceKind, Vec<usize>)>> = OnceLock::new();

    LAYOUTS
        .get_or_init(|| {
            ResourceServiceKind::ALL
                .into_iter()
                .map(|service| (service, (0..catalog_meta(service).len()).collect()))
                .collect()
        })
        .iter()
        .find(|(service, _)| *service == kind)
        .map(|(_, layout)| layout.as_slice())
        .expect("ResourceServiceKind::ALL covers every service kind")
}

pub static COMPUTE_COLUMNS: &[ColumnDef<ComputeInstanceProjection, ComputeInstanceSortField>] = &[
    ColumnDef {
        spec: TableColumn {
            key: "name",
            header: "NAME",
            min_width: 6,
            preferred_width: 28,
            priority: 0,
            align: CellAlign::Left,
            sortable: true,
            fit: ColumnFit::Whole,
        },
        sort: Some(ComputeInstanceSortField::Name),
        cell: |row| Cell::plain(row.name.clone()),
    },
    ColumnDef {
        spec: TableColumn {
            key: "status",
            header: "STATUS",
            min_width: 6,
            preferred_width: 10,
            priority: 1,
            align: CellAlign::Left,
            sortable: true,
            fit: ColumnFit::Whole,
        },
        sort: Some(ComputeInstanceSortField::Status),
        cell: |row| Cell::status(row.status.clone(), row.status_semantic),
    },
    ColumnDef {
        spec: TableColumn {
            key: "zone",
            header: "ZONE",
            min_width: 6,
            preferred_width: 18,
            priority: 2,
            align: CellAlign::Left,
            sortable: true,
            fit: ColumnFit::Whole,
        },
        sort: Some(ComputeInstanceSortField::Zone),
        cell: |row| Cell::plain(row.zone.clone()),
    },
    ColumnDef {
        spec: TableColumn {
            key: "machine-type",
            header: "MACHINE-TYPE",
            min_width: 8,
            preferred_width: 18,
            priority: 3,
            align: CellAlign::Left,
            sortable: true,
            fit: ColumnFit::Whole,
        },
        sort: Some(ComputeInstanceSortField::MachineType),
        cell: |row| Cell::plain(row.machine_type.clone()),
    },
    ColumnDef {
        spec: TableColumn {
            key: "internal-ip",
            header: "INTERNAL-IP",
            min_width: 8,
            preferred_width: 15,
            priority: 4,
            align: CellAlign::Left,
            sortable: false,
            fit: ColumnFit::Whole,
        },
        sort: None,
        cell: |row| Cell::optional(row.internal_ip.clone()),
    },
    ColumnDef {
        spec: TableColumn {
            key: "external-ip",
            header: "EXTERNAL-IP",
            min_width: 8,
            preferred_width: 15,
            priority: 5,
            align: CellAlign::Left,
            sortable: false,
            fit: ColumnFit::Whole,
        },
        sort: None,
        cell: |row| Cell::optional(row.external_ip.clone()),
    },
];

pub static GKE_COLUMNS: &[ColumnDef<GkeClusterProjection, GkeSortField>] = &[
    ColumnDef {
        spec: TableColumn {
            key: "name",
            header: "NAME",
            min_width: 6,
            preferred_width: 28,
            priority: 0,
            align: CellAlign::Left,
            sortable: true,
            fit: ColumnFit::Whole,
        },
        sort: Some(GkeSortField::Name),
        cell: |row| Cell::plain(row.name.clone()),
    },
    ColumnDef {
        spec: TableColumn {
            key: "status",
            header: "STATUS",
            min_width: 6,
            preferred_width: 12,
            priority: 1,
            align: CellAlign::Left,
            sortable: true,
            fit: ColumnFit::Whole,
        },
        sort: Some(GkeSortField::Status),
        cell: |row| Cell::status(row.status.clone(), row.status_semantic),
    },
    ColumnDef {
        spec: TableColumn {
            key: "location",
            header: "LOCATION",
            min_width: 6,
            preferred_width: 18,
            priority: 2,
            align: CellAlign::Left,
            sortable: true,
            fit: ColumnFit::Whole,
        },
        sort: Some(GkeSortField::Location),
        cell: |row| Cell::plain(row.location.clone()),
    },
    ColumnDef {
        spec: TableColumn {
            key: "version",
            header: "VERSION",
            min_width: 7,
            preferred_width: 20,
            priority: 3,
            align: CellAlign::Left,
            sortable: true,
            fit: ColumnFit::Whole,
        },
        sort: Some(GkeSortField::Version),
        cell: |row| Cell::optional(row.version.clone()),
    },
    ColumnDef {
        spec: TableColumn {
            key: "nodes",
            header: "NODES",
            min_width: 5,
            preferred_width: 6,
            priority: 4,
            align: CellAlign::Right,
            sortable: true,
            fit: ColumnFit::Whole,
        },
        sort: Some(GkeSortField::Nodes),
        cell: |row| Cell::optional(row.node_count.map(|count| count.to_string())),
    },
    ColumnDef {
        spec: TableColumn {
            key: "endpoint",
            header: "ENDPOINT",
            min_width: 7,
            preferred_width: 10,
            priority: 5,
            align: CellAlign::Left,
            sortable: false,
            fit: ColumnFit::Whole,
        },
        sort: None,
        cell: |row| Cell::plain(row.endpoint_visibility.clone()),
    },
];

pub static CLOUD_SQL_COLUMNS: &[ColumnDef<
    CloudSqlInstanceProjection,
    CloudSqlInstanceSortField,
>] = &[
    ColumnDef {
        spec: TableColumn {
            key: "name",
            header: "NAME",
            min_width: 6,
            preferred_width: 26,
            priority: 0,
            align: CellAlign::Left,
            sortable: true,
            fit: ColumnFit::Whole,
        },
        sort: Some(CloudSqlInstanceSortField::Name),
        cell: |row| Cell::plain(row.name.clone()),
    },
    ColumnDef {
        spec: TableColumn {
            key: "engine",
            header: "ENGINE",
            min_width: 7,
            preferred_width: 24,
            priority: 2,
            align: CellAlign::Left,
            sortable: true,
            fit: ColumnFit::Whole,
        },
        sort: Some(CloudSqlInstanceSortField::Engine),
        cell: |row| Cell::plain(cloud_sql_engine_cell(&row.engine, &row.database_version)),
    },
    ColumnDef {
        spec: TableColumn {
            key: "region",
            header: "REGION",
            min_width: 6,
            preferred_width: 16,
            priority: 3,
            align: CellAlign::Left,
            sortable: true,
            fit: ColumnFit::Whole,
        },
        sort: Some(CloudSqlInstanceSortField::Region),
        cell: |row| Cell::plain(row.region.clone()),
    },
    ColumnDef {
        spec: TableColumn {
            key: "tier",
            header: "TIER",
            min_width: 6,
            preferred_width: 18,
            priority: 4,
            align: CellAlign::Left,
            sortable: true,
            fit: ColumnFit::Whole,
        },
        sort: Some(CloudSqlInstanceSortField::Tier),
        cell: |row| Cell::plain(row.tier.clone()),
    },
    ColumnDef {
        spec: TableColumn {
            key: "availability",
            header: "AVAILABILITY",
            min_width: 6,
            preferred_width: 14,
            priority: 5,
            align: CellAlign::Left,
            sortable: true,
            fit: ColumnFit::Whole,
        },
        sort: Some(CloudSqlInstanceSortField::Availability),
        cell: |row| Cell::plain(row.availability.clone()),
    },
    ColumnDef {
        spec: TableColumn {
            key: "connectivity",
            header: "CONNECTIVITY",
            min_width: 7,
            preferred_width: 12,
            priority: 6,
            align: CellAlign::Left,
            sortable: false,
            fit: ColumnFit::Whole,
        },
        sort: None,
        cell: |row| Cell::plain(row.connectivity.clone()),
    },
    ColumnDef {
        spec: TableColumn {
            key: "status",
            header: "STATUS",
            min_width: 6,
            preferred_width: 12,
            priority: 1,
            align: CellAlign::Left,
            sortable: true,
            fit: ColumnFit::Whole,
        },
        sort: Some(CloudSqlInstanceSortField::Status),
        cell: |row| Cell::status(row.status.clone(), row.status_semantic),
    },
];

pub static CLOUD_RUN_COLUMNS: &[ColumnDef<CloudRunServiceProjection, CloudRunSortField>] = &[
    ColumnDef {
        spec: TableColumn {
            key: "name",
            header: "NAME",
            min_width: 6,
            preferred_width: 28,
            priority: 0,
            align: CellAlign::Left,
            sortable: true,
            fit: ColumnFit::Whole,
        },
        sort: Some(CloudRunSortField::Name),
        cell: |row| Cell::plain(row.name.clone()),
    },
    ColumnDef {
        spec: TableColumn {
            key: "status",
            header: "STATUS",
            min_width: 6,
            preferred_width: 12,
            priority: 1,
            align: CellAlign::Left,
            sortable: true,
            fit: ColumnFit::Whole,
        },
        sort: Some(CloudRunSortField::Status),
        cell: |row| Cell::status(row.status.clone(), row.status_semantic),
    },
    ColumnDef {
        spec: TableColumn {
            key: "region",
            header: "REGION",
            min_width: 6,
            preferred_width: 16,
            priority: 2,
            align: CellAlign::Left,
            sortable: true,
            fit: ColumnFit::Whole,
        },
        sort: Some(CloudRunSortField::Region),
        cell: |row| Cell::plain(row.region.clone()),
    },
    ColumnDef {
        spec: TableColumn {
            key: "url",
            header: "URL",
            min_width: 8,
            preferred_width: 32,
            priority: 3,
            align: CellAlign::Left,
            sortable: true,
            fit: ColumnFit::Cut,
        },
        sort: Some(CloudRunSortField::Url),
        cell: |row| Cell::optional(row.url.clone()),
    },
    ColumnDef {
        spec: TableColumn {
            key: "latest-revision",
            header: "LATEST-REVISION",
            min_width: 10,
            preferred_width: 22,
            priority: 4,
            align: CellAlign::Left,
            sortable: true,
            fit: ColumnFit::Whole,
        },
        sort: Some(CloudRunSortField::LatestRevision),
        cell: |row| Cell::optional(row.latest_ready_revision.clone()),
    },
    ColumnDef {
        spec: TableColumn {
            key: "traffic",
            header: "TRAFFIC",
            min_width: 7,
            preferred_width: 20,
            priority: 5,
            align: CellAlign::Left,
            sortable: false,
            fit: ColumnFit::Cut,
        },
        sort: None,
        cell: |row| Cell::plain(row.traffic_summary.clone()),
    },
];

pub static STORAGE_BUCKET_COLUMNS: &[ColumnDef<StorageBucketRowProjection, StorageSortField>] = &[
    ColumnDef {
        spec: TableColumn {
            key: "name",
            header: "NAME",
            min_width: 6,
            preferred_width: 30,
            priority: 0,
            align: CellAlign::Left,
            sortable: true,
            fit: ColumnFit::Whole,
        },
        sort: Some(StorageSortField::Name),
        cell: |row| Cell::plain(row.name.clone()),
    },
    ColumnDef {
        spec: TableColumn {
            key: "location",
            header: "LOCATION",
            min_width: 6,
            preferred_width: 16,
            priority: 2,
            align: CellAlign::Left,
            sortable: true,
            fit: ColumnFit::Whole,
        },
        sort: Some(StorageSortField::Location),
        cell: |row| Cell::plain(row.location.clone()),
    },
    ColumnDef {
        spec: TableColumn {
            key: "class",
            header: "CLASS",
            min_width: 6,
            preferred_width: 12,
            priority: 1,
            align: CellAlign::Left,
            sortable: true,
            fit: ColumnFit::Whole,
        },
        sort: Some(StorageSortField::StorageClass),
        cell: |row| Cell::plain(row.storage_class.clone()),
    },
    ColumnDef {
        spec: TableColumn {
            key: "updated",
            header: "UPDATED",
            min_width: 8,
            preferred_width: 22,
            priority: 3,
            align: CellAlign::Left,
            sortable: true,
            fit: ColumnFit::Whole,
        },
        sort: Some(StorageSortField::Updated),
        cell: |row| Cell::plain(row.updated.clone()),
    },
    ColumnDef {
        spec: TableColumn {
            key: "versioning",
            header: "VERSIONING",
            min_width: 4,
            preferred_width: 10,
            priority: 4,
            align: CellAlign::Left,
            sortable: false,
            fit: ColumnFit::Whole,
        },
        sort: None,
        cell: |row| Cell::plain(row.versioning.clone()),
    },
    ColumnDef {
        spec: TableColumn {
            key: "public-access",
            header: "PUBLIC-ACCESS",
            min_width: 6,
            preferred_width: 16,
            priority: 5,
            align: CellAlign::Left,
            sortable: false,
            fit: ColumnFit::Whole,
        },
        sort: None,
        cell: |row| Cell::plain(row.public_access.clone()),
    },
];

/// The secrets catalog.
///
/// There is deliberately no version-count column. The Secret resource the list
/// call returns carries no such field, so a column here would have to be filled
/// by one extra `secrets versions list` call *per secret* on every list load —
/// and one list load being one gcloud call is an invariant every other service
/// in this repo holds. The count lives in the detail pane's Versions tab, which
/// fetches on demand for the one secret the operator opened.
///
/// There is likewise no payload column, and there can never be one: nothing in
/// this binary fetches a payload. See `tests/secrets_no_payload_access_test.rs`.
pub static SECRET_COLUMNS: &[ColumnDef<SecretRowProjection, SecretSortField>] = &[
    ColumnDef {
        spec: TableColumn {
            key: "name",
            header: "NAME",
            min_width: 6,
            preferred_width: 30,
            priority: 0,
            align: CellAlign::Left,
            sortable: true,
            fit: ColumnFit::Whole,
        },
        sort: Some(SecretSortField::Name),
        cell: |row| Cell::plain(row.name.clone()),
    },
    ColumnDef {
        spec: TableColumn {
            key: "created",
            header: "CREATED",
            min_width: 8,
            preferred_width: 22,
            priority: 1,
            align: CellAlign::Left,
            sortable: true,
            fit: ColumnFit::Whole,
        },
        sort: Some(SecretSortField::Created),
        cell: |row| Cell::plain(row.created.clone()),
    },
    ColumnDef {
        spec: TableColumn {
            key: "replication",
            header: "REPLICATION",
            min_width: 8,
            preferred_width: 28,
            priority: 2,
            align: CellAlign::Left,
            sortable: true,
            fit: ColumnFit::Cut,
        },
        sort: Some(SecretSortField::Replication),
        cell: |row| Cell::plain(row.replication.clone()),
    },
    ColumnDef {
        spec: TableColumn {
            key: "labels",
            header: "LABELS",
            min_width: 6,
            preferred_width: 26,
            priority: 3,
            align: CellAlign::Left,
            sortable: false,
            fit: ColumnFit::Cut,
        },
        sort: None,
        cell: |row| Cell::plain(row.labels.clone()),
    },
];

/// The Cloud Functions table.
///
/// `name` and `region` lead, because either alone is ambiguous: a function's
/// name is unique within its region only. Then the three fields an operator
/// checks a function against — what it is doing, which generation it runs on,
/// when it last changed — and the URL last, since it is the widest cell and
/// the first the layout should drop on a narrow terminal.
///
/// `url` is the one column with no sort field. Cloud Run's is sortable and
/// this is not a drift: a Cloud Run URL carries a per-service hash, so its
/// order is information the other columns do not hold, while a function's is
/// `https://<region>-<project>.cloudfunctions.net/<name>` — sorting by it
/// sorts by region and then by name, which the first two columns already do.
pub static CLOUD_FUNCTION_COLUMNS: &[ColumnDef<
    CloudFunctionRowProjection,
    CloudFunctionSortField,
>] = &[
    ColumnDef {
        spec: TableColumn {
            key: "name",
            header: "NAME",
            min_width: 6,
            preferred_width: 30,
            priority: 0,
            align: CellAlign::Left,
            sortable: true,
            fit: ColumnFit::Whole,
        },
        sort: Some(CloudFunctionSortField::Name),
        cell: |row| Cell::plain(row.name.clone()),
    },
    ColumnDef {
        spec: TableColumn {
            key: "region",
            header: "REGION",
            min_width: 6,
            preferred_width: 16,
            priority: 1,
            align: CellAlign::Left,
            sortable: true,
            fit: ColumnFit::Whole,
        },
        sort: Some(CloudFunctionSortField::Region),
        cell: |row| Cell::plain(row.region.clone()),
    },
    ColumnDef {
        spec: TableColumn {
            key: "state",
            header: "STATE",
            min_width: 6,
            preferred_width: 14,
            priority: 2,
            align: CellAlign::Left,
            sortable: true,
            fit: ColumnFit::Whole,
        },
        sort: Some(CloudFunctionSortField::State),
        cell: |row| Cell::status(row.state.clone(), row.status_semantic),
    },
    ColumnDef {
        spec: TableColumn {
            key: "generation",
            header: "GENERATION",
            min_width: 7,
            preferred_width: 12,
            priority: 3,
            align: CellAlign::Left,
            sortable: true,
            fit: ColumnFit::Whole,
        },
        sort: Some(CloudFunctionSortField::Generation),
        cell: |row| Cell::plain(row.generation.clone()),
    },
    ColumnDef {
        spec: TableColumn {
            key: "updated",
            header: "UPDATED",
            min_width: 8,
            preferred_width: 22,
            priority: 4,
            align: CellAlign::Left,
            sortable: true,
            fit: ColumnFit::Whole,
        },
        sort: Some(CloudFunctionSortField::Updated),
        cell: |row| Cell::optional(row.updated.clone()),
    },
    ColumnDef {
        spec: TableColumn {
            key: "url",
            header: "URL",
            min_width: 8,
            // A Cloud Functions invoke URL is
            // `https://<region>-<project>.cloudfunctions.net/<name>` — wider
            // than Cloud Run's 32, because every part of it is spelled out
            // rather than hashed.
            preferred_width: 64,
            priority: 5,
            align: CellAlign::Left,
            sortable: false,
            fit: ColumnFit::Cut,
        },
        sort: None,
        cell: |row| Cell::optional(row.url.clone()),
    },
];

/// The persistent disks table.
///
/// Identity first — name, then location, because a disk's name is unique within
/// its zone or region only. `orphan` sits immediately after `status` because
/// the two are read together: the badge is a claim about a disk that is
/// otherwise healthy, and separating them would let an operator read `ORPHAN`
/// without the `Ready` that qualified it. Then the two facts that size the
/// waste, and `attached` last: it is the widest cell and the first the layout
/// should drop on a narrow terminal, and its content is exactly what `orphan`
/// already summarises for the rows that matter.
///
/// `orphan` is the one column with no sort field. It has two values, so sorting
/// it groups the table into "badged" and "not badged" — which is what the
/// filter already does, precisely and without disturbing the ordering the
/// operator chose. `attached` sorts, and an unattached disk's empty cell groups
/// the orphans together there too.
pub static PERSISTENT_DISK_COLUMNS: &[ColumnDef<PersistentDiskRowProjection, DiskSortField>] = &[
    ColumnDef {
        spec: TableColumn {
            key: "name",
            header: "NAME",
            min_width: 6,
            preferred_width: 30,
            priority: 0,
            align: CellAlign::Left,
            sortable: true,
            fit: ColumnFit::Whole,
        },
        sort: Some(DiskSortField::Name),
        cell: |row| Cell::plain(row.name.clone()),
    },
    ColumnDef {
        spec: TableColumn {
            key: "location",
            header: "LOCATION",
            min_width: 6,
            preferred_width: 18,
            priority: 2,
            align: CellAlign::Left,
            sortable: true,
            fit: ColumnFit::Whole,
        },
        sort: Some(DiskSortField::Location),
        cell: |row| Cell::plain(row.location.clone()),
    },
    ColumnDef {
        spec: TableColumn {
            key: "status",
            header: "STATUS",
            min_width: 6,
            preferred_width: 12,
            priority: 3,
            align: CellAlign::Left,
            sortable: true,
            fit: ColumnFit::Whole,
        },
        sort: Some(DiskSortField::Status),
        cell: |row| Cell::status(row.status.clone(), row.status_semantic),
    },
    ColumnDef {
        spec: TableColumn {
            key: "orphan",
            header: "ORPHAN",
            min_width: 6,
            preferred_width: 8,
            // Priority 1: after the name and ahead of everything else. This
            // column is why the service exists, so it must survive a narrow
            // terminal that has already dropped the type and the attachment
            // list.
            priority: 1,
            align: CellAlign::Left,
            sortable: false,
            fit: ColumnFit::Whole,
        },
        sort: None,
        cell: |row| {
            if row.orphan.is_empty() {
                Cell::plain(String::new())
            } else {
                Cell::status(row.orphan.clone(), StatusSemantic::Warning)
            }
        },
    },
    ColumnDef {
        spec: TableColumn {
            key: "size",
            header: "SIZE",
            min_width: 4,
            preferred_width: 10,
            priority: 4,
            // Right-aligned so the magnitudes line up: an operator scanning for
            // the biggest waste reads the column as a bar chart, which a
            // left-aligned `9 GB` beside `200 GB` defeats.
            align: CellAlign::Right,
            sortable: true,
            fit: ColumnFit::Whole,
        },
        sort: Some(DiskSortField::Size),
        cell: |row| Cell::plain(row.size.clone()),
    },
    ColumnDef {
        spec: TableColumn {
            key: "type",
            header: "TYPE",
            min_width: 6,
            preferred_width: 14,
            priority: 5,
            align: CellAlign::Left,
            sortable: true,
            fit: ColumnFit::Whole,
        },
        sort: Some(DiskSortField::Type),
        cell: |row| Cell::plain(row.disk_type.clone()),
    },
    ColumnDef {
        spec: TableColumn {
            key: "attached",
            header: "ATTACHED",
            min_width: 8,
            // A regional disk can name two instances, and an instance name is
            // routinely 40 characters of GKE node.
            preferred_width: 44,
            priority: 6,
            align: CellAlign::Left,
            sortable: true,
            fit: ColumnFit::Cut,
        },
        sort: Some(DiskSortField::Attached),
        cell: |row| Cell::plain(row.attached.clone()),
    },
];

/// Combines engine and database version into a single display cell, reading
/// the version out of the API's constant: `POSTGRES_15` is "PostgreSQL 15",
/// `MYSQL_8_0_31` is "MySQL 8.0.31", `SQLSERVER_2019_STANDARD` is
/// "SQL Server 2019 Standard".
///
/// The constant spelled the engine a second time, so the old cell, "PostgreSQL
/// (POSTGRES_15)", was cut at common widths before it reached the one part
/// that differed between rows. A constant with no version after its engine
/// prefix is shown as it came, so nothing is lost; the detail pane keeps the
/// raw value either way. The list's preview pane reads its `Engine` line from
/// here too, so the two never name the same instance differently.
pub(crate) fn cloud_sql_engine_cell(engine: &str, database_version: &str) -> String {
    let Some((_, version)) = database_version.split_once('_') else {
        return match database_version {
            "" => engine.to_string(),
            raw => format!("{engine} ({raw})"),
        };
    };
    let mut numbers = Vec::new();
    let mut words = Vec::new();
    for token in version.split('_').filter(|token| !token.is_empty()) {
        if words.is_empty() && token.chars().all(|c| c.is_ascii_digit()) {
            numbers.push(token.to_string());
        } else {
            let lower = token.to_lowercase();
            let mut chars = lower.chars();
            words.push(match chars.next() {
                Some(first) => first.to_uppercase().chain(chars).collect(),
                None => String::new(),
            });
        }
    }
    let mut parts = vec![engine.to_string()];
    if !numbers.is_empty() {
        parts.push(numbers.join("."));
    }
    parts.extend(words);
    parts.join(" ")
}

/// The Pub/Sub topics table.
///
/// `name` leads and is the whole of a topic's identity: a topic is global and
/// its id is unique within the project, so there is no location column to
/// qualify it with.
///
/// `subscriptions` comes second because it is the column an operator scans for
/// — a topic nothing is subscribed to is the interesting row, and it is
/// invisible in every other cell. It is also the one column derived from a
/// second call, so it is the one that can read as unknown while its neighbours
/// are fine; `SubscriptionCount::label` renders that as a dash rather than a
/// zero, which would be a specific and wrong claim.
///
/// `labels` is last and is the one column with no sort field, matching every
/// other table that carries them: a flattened `k=v, k=v` cell sorts by whichever
/// key happens to sort first, which is an order that means nothing.
/// The subscriptions table's catalogue.
///
/// `NAME` and `TOPIC` lead because they are the two identities an operator
/// navigates by: which subscription, and what it is attached to. `TOPIC`
/// paints the short id, and a subscription whose topic has been deleted paints
/// the API's `_deleted-topic_` sentinel verbatim rather than a blank — an
/// orphan is a finding, not a missing value.
///
/// `DELIVERY` and `STATE` come next as the two facts that decide whether the
/// subscription is working. `STATE` is the only styled cell here: a
/// `RESOURCE_ERROR` subscription reads normally in every other column while
/// Pub/Sub cannot deliver to it, so it carries its semantic rather than
/// rendering as plain text like the rest.
///
/// The push endpoint is deliberately **not** a column. It is a URL as long as
/// a KMS key path and only ever set on push rows, so a column for it would be
/// mostly dashes and would push the facts that matter off a narrow terminal.
/// It is searchable, and the preview pane renders it in full.
///
/// `ACK(S)` is right-aligned and bare of units for the same reason `SUBS` is
/// on the topics table: the header carries the unit once so the cells do not
/// carry it on every row.
///
/// `labels` is last and is the one column with no sort field, matching every
/// other table that carries them: a flattened `k=v, k=v` cell sorts by
/// whichever key happens to sort first, which is an order that means nothing.
pub static PUBSUB_SUBSCRIPTION_COLUMNS: &[ColumnDef<
    PubSubSubscriptionRowProjection,
    PubSubSubscriptionSortField,
>] = &[
    ColumnDef {
        spec: TableColumn {
            key: "name",
            header: "NAME",
            min_width: 6,
            preferred_width: 30,
            priority: 0,
            align: CellAlign::Left,
            sortable: true,
            fit: ColumnFit::Whole,
        },
        sort: Some(PubSubSubscriptionSortField::Name),
        cell: |row| Cell::plain(row.name.clone()),
    },
    ColumnDef {
        spec: TableColumn {
            key: "topic",
            header: "TOPIC",
            min_width: 6,
            preferred_width: 24,
            priority: 1,
            align: CellAlign::Left,
            sortable: true,
            fit: ColumnFit::Whole,
        },
        sort: Some(PubSubSubscriptionSortField::Topic),
        cell: |row| Cell::plain(row.topic.clone()),
    },
    ColumnDef {
        spec: TableColumn {
            key: "delivery",
            header: "DELIVERY",
            min_width: 4,
            preferred_width: 8,
            priority: 3,
            align: CellAlign::Left,
            sortable: true,
            fit: ColumnFit::Whole,
        },
        sort: Some(PubSubSubscriptionSortField::Delivery),
        cell: |row| Cell::plain(row.delivery.clone()),
    },
    ColumnDef {
        spec: TableColumn {
            key: "state",
            header: "STATE",
            min_width: 5,
            preferred_width: 14,
            priority: 2,
            align: CellAlign::Left,
            sortable: true,
            fit: ColumnFit::Whole,
        },
        sort: Some(PubSubSubscriptionSortField::State),
        cell: |row| Cell::status(row.state.clone(), row.status_semantic),
    },
    ColumnDef {
        spec: TableColumn {
            key: "ack_deadline",
            header: "ACK(S)",
            min_width: 6,
            preferred_width: 6,
            priority: 5,
            align: CellAlign::Right,
            sortable: true,
            fit: ColumnFit::Whole,
        },
        sort: Some(PubSubSubscriptionSortField::AckDeadline),
        cell: |row| Cell::optional(row.ack_deadline.clone()),
    },
    ColumnDef {
        spec: TableColumn {
            key: "retention",
            header: "RETENTION",
            min_width: 6,
            preferred_width: 12,
            priority: 6,
            align: CellAlign::Left,
            sortable: true,
            fit: ColumnFit::Whole,
        },
        sort: Some(PubSubSubscriptionSortField::Retention),
        cell: |row| Cell::optional(row.retention.clone()),
    },
    ColumnDef {
        spec: TableColumn {
            key: "dead_letter",
            header: "DEAD LETTER",
            min_width: 8,
            preferred_width: 20,
            priority: 4,
            align: CellAlign::Left,
            sortable: true,
            fit: ColumnFit::Whole,
        },
        sort: Some(PubSubSubscriptionSortField::DeadLetter),
        cell: |row| Cell::optional(row.dead_letter.clone()),
    },
    ColumnDef {
        spec: TableColumn {
            key: "labels",
            header: "LABELS",
            min_width: 6,
            preferred_width: 24,
            priority: 7,
            align: CellAlign::Left,
            sortable: false,
            fit: ColumnFit::Cut,
        },
        sort: None,
        cell: |row| Cell::plain(row.labels.clone()),
    },
];

pub static PUBSUB_TOPIC_COLUMNS: &[ColumnDef<PubSubTopicRowProjection, PubSubTopicSortField>] = &[
    ColumnDef {
        spec: TableColumn {
            key: "name",
            header: "NAME",
            min_width: 6,
            preferred_width: 30,
            priority: 0,
            align: CellAlign::Left,
            sortable: true,
            fit: ColumnFit::Whole,
        },
        sort: Some(PubSubTopicSortField::Name),
        cell: |row| Cell::plain(row.name.clone()),
    },
    ColumnDef {
        spec: TableColumn {
            key: "subscriptions",
            header: "SUBS",
            min_width: 4,
            preferred_width: 6,
            priority: 1,
            align: CellAlign::Right,
            sortable: true,
            fit: ColumnFit::Whole,
        },
        sort: Some(PubSubTopicSortField::Subscriptions),
        cell: |row| Cell::plain(row.subscriptions.clone()),
    },
    ColumnDef {
        spec: TableColumn {
            key: "retention",
            header: "RETENTION",
            min_width: 6,
            preferred_width: 12,
            priority: 2,
            align: CellAlign::Left,
            sortable: true,
            fit: ColumnFit::Whole,
        },
        sort: Some(PubSubTopicSortField::Retention),
        cell: |row| Cell::optional(row.retention.clone()),
    },
    ColumnDef {
        spec: TableColumn {
            key: "kms_key",
            header: "KMS KEY",
            min_width: 8,
            // A key path is
            // `projects/p/locations/l/keyRings/r/cryptoKeys/k` — long, and the
            // first cell the layout should drop on a narrow terminal after
            // labels.
            preferred_width: 48,
            priority: 4,
            align: CellAlign::Left,
            sortable: true,
            fit: ColumnFit::Cut,
        },
        sort: Some(PubSubTopicSortField::KmsKey),
        cell: |row| Cell::optional(row.kms_key.clone()),
    },
    ColumnDef {
        spec: TableColumn {
            key: "labels",
            header: "LABELS",
            min_width: 6,
            preferred_width: 26,
            priority: 3,
            align: CellAlign::Left,
            sortable: false,
            fit: ColumnFit::Cut,
        },
        sort: None,
        cell: |row| Cell::plain(row.labels.clone()),
    },
];

/// The VPC networks table: name, subnet mode, routing mode, subnet count.
///
/// These four are the whole resource as an operator reads it. There is no
/// `gateway` column to give: `gatewayAddress` is a
/// *subnet* field, and a network carries no address at all.
///
/// `MODE` and `ROUTING` are both derived — from `autoCreateSubnetworks` and
/// `routingConfig.routingMode` — and never from the `x_gcloud_subnet_mode` and
/// `x_gcloud_bgp_routing_mode` keys gcloud synthesises beside them, which
/// carry these exact strings. See `crate::services::resources::vpc`.
///
/// `SUBNETS` is right-aligned and is a count rather than a list: the captured
/// `default` network has 43, each a full self-link URL. It keeps a low
/// priority so a narrow terminal drops it before either mode — the count is
/// the one cell here an operator can get elsewhere, by opening the subnets
/// table.
///
/// There is no `LABELS` column, and that is the one way this table differs
/// from the Pub/Sub topics table it is otherwise shaped like: the `compute/v1`
/// networks resource has no `labels` field at all. Every column here sorts,
/// because a flattened `k=v` cell is the only kind whose ordering would mean
/// nothing and this table has none.
///
/// The description has no column either — unbounded free text would push all
/// four of these off a narrow terminal — but it is searchable and the preview
/// pane renders it.
pub static VPC_NETWORK_COLUMNS: &[ColumnDef<VpcNetworkRowProjection, VpcNetworkSortField>] = &[
    ColumnDef {
        spec: TableColumn {
            key: "name",
            header: "NAME",
            min_width: 6,
            preferred_width: 30,
            priority: 0,
            align: CellAlign::Left,
            sortable: true,
            fit: ColumnFit::Whole,
        },
        sort: Some(VpcNetworkSortField::Name),
        cell: |row| Cell::plain(row.name.clone()),
    },
    ColumnDef {
        spec: TableColumn {
            key: "mode",
            header: "MODE",
            min_width: 4,
            // `CUSTOM` and `LEGACY` are the longest values, at six.
            preferred_width: 8,
            priority: 1,
            align: CellAlign::Left,
            sortable: true,
            fit: ColumnFit::Whole,
        },
        sort: Some(VpcNetworkSortField::Mode),
        cell: |row| Cell::plain(row.mode.clone()),
    },
    ColumnDef {
        spec: TableColumn {
            key: "routing",
            header: "ROUTING",
            min_width: 7,
            preferred_width: 10,
            priority: 2,
            align: CellAlign::Left,
            sortable: true,
            fit: ColumnFit::Whole,
        },
        sort: Some(VpcNetworkSortField::Routing),
        cell: |row| Cell::optional(row.routing.clone()),
    },
    ColumnDef {
        spec: TableColumn {
            key: "subnets",
            header: "SUBNETS",
            min_width: 7,
            preferred_width: 7,
            priority: 3,
            align: CellAlign::Right,
            sortable: true,
            fit: ColumnFit::Whole,
        },
        sort: Some(VpcNetworkSortField::Subnets),
        cell: |row| Cell::plain(row.subnets.clone()),
    },
];

/// The VPC subnets table: name, region, network, range, gateway, private
/// Google access.
///
/// `REGION` sits at priority 1, immediately behind the identity column and
/// ahead of everything else, because it is **half the identity**: an auto-mode
/// network calls every one of its subnets `default`, so a table that dropped
/// `REGION` on a narrow terminal would paint 43 rows an operator cannot tell
/// apart. That is the one place this catalogue's priorities differ from what a
/// left-to-right reading would give.
///
/// `REGION` and `NETWORK` paint ids, not the self-link URLs the API sends —
/// the reduction happens once in the parser (see
/// `crate::services::resources::vpc`), so no cell here can disagree with
/// another about what a network is called.
///
/// `PRIVATE GOOGLE ACCESS` renders `On`/`Off` rather than a dash for `false`:
/// the API documents an absent flag as off, so `Off` is an answer and a dash
/// would invite an operator to wonder which rows went unreported.
///
/// Like the networks table, there is no `LABELS` column — the `compute/v1`
/// subnetworks resource has no `labels` field — so every column here sorts.
/// The purpose and stack type have no column either: both are near-constant
/// across a project's subnets, and the preview pane has room for what a table
/// column would spend width on.
pub static VPC_SUBNET_COLUMNS: &[ColumnDef<VpcSubnetRowProjection, VpcSubnetSortField>] = &[
    ColumnDef {
        spec: TableColumn {
            key: "name",
            header: "NAME",
            min_width: 6,
            preferred_width: 24,
            priority: 0,
            align: CellAlign::Left,
            sortable: true,
            fit: ColumnFit::Whole,
        },
        sort: Some(VpcSubnetSortField::Name),
        cell: |row| Cell::plain(row.name.clone()),
    },
    ColumnDef {
        spec: TableColumn {
            key: "region",
            header: "REGION",
            min_width: 6,
            // `northamerica-northeast2` is the longest region id in the
            // captured project, at 23.
            preferred_width: 24,
            priority: 1,
            align: CellAlign::Left,
            sortable: true,
            fit: ColumnFit::Whole,
        },
        sort: Some(VpcSubnetSortField::Region),
        cell: |row| Cell::plain(row.region.clone()),
    },
    ColumnDef {
        spec: TableColumn {
            key: "network",
            header: "NETWORK",
            min_width: 6,
            preferred_width: 20,
            priority: 2,
            align: CellAlign::Left,
            sortable: true,
            fit: ColumnFit::Whole,
        },
        sort: Some(VpcSubnetSortField::Network),
        cell: |row| Cell::plain(row.network.clone()),
    },
    ColumnDef {
        spec: TableColumn {
            key: "range",
            header: "RANGE",
            min_width: 9,
            // `255.255.255.255/32` is 18.
            preferred_width: 18,
            priority: 3,
            align: CellAlign::Left,
            sortable: true,
            fit: ColumnFit::Whole,
        },
        sort: Some(VpcSubnetSortField::Range),
        cell: |row| Cell::plain(row.range.clone()),
    },
    ColumnDef {
        spec: TableColumn {
            key: "gateway",
            header: "GATEWAY",
            min_width: 7,
            preferred_width: 15,
            priority: 5,
            align: CellAlign::Left,
            sortable: true,
            fit: ColumnFit::Whole,
        },
        sort: Some(VpcSubnetSortField::Gateway),
        cell: |row| Cell::optional(row.gateway.clone()),
    },
    ColumnDef {
        spec: TableColumn {
            key: "private_google_access",
            header: "PGA",
            min_width: 3,
            // Headed `PGA` rather than `PRIVATE GOOGLE ACCESS`: the full
            // phrase is 21 characters to carry a three-character cell, and it
            // is the abbreviation the console and the gcloud flag both use.
            preferred_width: 3,
            priority: 4,
            align: CellAlign::Left,
            sortable: true,
            fit: ColumnFit::Whole,
        },
        sort: Some(VpcSubnetSortField::PrivateGoogleAccess),
        cell: |row| Cell::plain(row.private_google_access.clone()),
    },
];

/// The firewall rules table: name, network, direction, priority, action,
/// targets, source/dest.
///
/// `ACTION` sits at priority 1 — behind only the identity column and ahead of
/// everything else — because it is the answer the table exists to give. A
/// firewall table that dropped `ALLOW`/`DENY` on a narrow terminal would list
/// the rules without saying what any of them does.
///
/// The rule's **disabled** state is not a column at all. It rides the identity
/// cell as a leading badge (`DISABLED name`), because the identity column is
/// the one `apply_display_priorities` pins and `plan_table` may never drop: a
/// disabled DENY rule that lost its badge reads as a live DENY rule, and an
/// operator concludes traffic is blocked that is flowing. That is the worst
/// misreading this application can produce, so it is put where no width can
/// remove it and no configuration can reorder it away.
///
/// `ACTION` paints a dash for a rule carrying neither `allowed` nor `denied`.
/// That is deliberate and it is not a cosmetic choice: the alternative is
/// defaulting to `ALLOW`, which would report a rule the parser could not read
/// as one that permits traffic.
///
/// `SOURCE/DEST` is one column rather than two because a rule carries the list
/// matching its direction and never the other — two columns would leave one of
/// them empty on every row.
///
/// Every column sorts, and `PRIORITY` sorts numerically: lower wins, so
/// ascending is the order traffic actually meets the rules in.
pub static FIREWALL_RULE_COLUMNS: &[ColumnDef<FirewallRuleRowProjection, FirewallSortField>] = &[
    ColumnDef {
        spec: TableColumn {
            key: "name",
            header: "NAME",
            min_width: 6,
            // Wide enough for `DISABLED ` plus a real rule name.
            preferred_width: 34,
            priority: 0,
            align: CellAlign::Left,
            sortable: true,
            fit: ColumnFit::Whole,
        },
        sort: Some(FirewallSortField::Name),
        cell: |row| {
            if row.disabled {
                // Styled, not merely spelled: a disabled rule is a row that
                // does not do what it says, which is what this semantic means
                // everywhere else in the application.
                Cell::status(row.name.clone(), StatusSemantic::Degraded)
            } else {
                Cell::plain(row.name.clone())
            }
        },
    },
    ColumnDef {
        spec: TableColumn {
            key: "network",
            header: "NETWORK",
            min_width: 6,
            preferred_width: 18,
            priority: 4,
            align: CellAlign::Left,
            sortable: true,
            fit: ColumnFit::Whole,
        },
        sort: Some(FirewallSortField::Network),
        cell: |row| Cell::plain(row.network.clone()),
    },
    ColumnDef {
        spec: TableColumn {
            key: "direction",
            header: "DIRECTION",
            min_width: 7,
            preferred_width: 9,
            priority: 2,
            align: CellAlign::Left,
            sortable: true,
            fit: ColumnFit::Whole,
        },
        sort: Some(FirewallSortField::Direction),
        cell: |row| Cell::optional(row.direction.clone()),
    },
    ColumnDef {
        spec: TableColumn {
            key: "priority",
            header: "PRIORITY",
            min_width: 8,
            preferred_width: 8,
            priority: 5,
            align: CellAlign::Right,
            sortable: true,
            fit: ColumnFit::Whole,
        },
        sort: Some(FirewallSortField::Priority),
        cell: |row| Cell::optional(row.priority.clone()),
    },
    ColumnDef {
        spec: TableColumn {
            key: "action",
            header: "ACTION",
            min_width: 5,
            preferred_width: 6,
            priority: 1,
            align: CellAlign::Left,
            sortable: true,
            fit: ColumnFit::Whole,
        },
        sort: Some(FirewallSortField::Action),
        cell: |row| Cell::optional(row.action.clone()),
    },
    ColumnDef {
        spec: TableColumn {
            key: "targets",
            header: "TARGETS",
            min_width: 7,
            // A service-account target is a full email, which runs long.
            preferred_width: 32,
            priority: 6,
            align: CellAlign::Left,
            sortable: true,
            fit: ColumnFit::Cut,
        },
        sort: Some(FirewallSortField::Targets),
        cell: |row| Cell::plain(row.targets.clone()),
    },
    ColumnDef {
        spec: TableColumn {
            key: "source_or_dest",
            header: "SOURCE/DEST",
            min_width: 9,
            preferred_width: 24,
            priority: 3,
            align: CellAlign::Left,
            sortable: true,
            fit: ColumnFit::Cut,
        },
        sort: Some(FirewallSortField::SourceOrDest),
        cell: |row| Cell::optional(row.source_or_dest.clone()),
    },
];

/// The IAM service accounts table: email, display name, state, unique id.
///
/// The identity column's key is the literal [`IDENTITY_COLUMN`] (`"name"`)
/// while its header says `EMAIL`: the email *is* the account's identity — what
/// every other resource names it by — and the key is what the column-layout
/// config and the identity-pinning rule match on, so it cannot vary by
/// service.
///
/// `state` renders `service_account_state_label`'s word — `Enabled` or
/// `Disabled` — rather than a badge on the disabled rows: "this identity
/// cannot authenticate" is a fact worth reading directly off every row of an
/// access review, not an exception to flag.
///
/// Every column sorts. None of these cells is a flattened map, so there is no
/// labels-style column whose ordering would mean nothing.
pub static SERVICE_ACCOUNT_COLUMNS: &[ColumnDef<
    ServiceAccountRowProjection,
    ServiceAccountSortField,
>] = &[
    ColumnDef {
        spec: TableColumn {
            key: "name",
            header: "EMAIL",
            min_width: 6,
            // `sa-name@project.iam.gserviceaccount.com` runs long; the email
            // is the row's identity, so it gets the room first.
            preferred_width: 44,
            priority: 0,
            align: CellAlign::Left,
            sortable: true,
            fit: ColumnFit::Whole,
        },
        sort: Some(ServiceAccountSortField::Email),
        cell: |row| Cell::plain(row.name.clone()),
    },
    ColumnDef {
        spec: TableColumn {
            key: "display_name",
            header: "DISPLAY NAME",
            min_width: 6,
            preferred_width: 28,
            priority: 2,
            align: CellAlign::Left,
            sortable: true,
            fit: ColumnFit::Cut,
        },
        sort: Some(ServiceAccountSortField::DisplayName),
        cell: |row| Cell::optional(row.display_name.clone()),
    },
    ColumnDef {
        spec: TableColumn {
            key: "state",
            header: "STATE",
            min_width: 5,
            preferred_width: 8,
            // Second in drop order only to the email: the disabled rows are
            // what an access review scans for, and they are invisible in
            // every other cell.
            priority: 1,
            align: CellAlign::Left,
            sortable: true,
            fit: ColumnFit::Whole,
        },
        sort: Some(ServiceAccountSortField::Disabled),
        cell: |row| Cell::plain(row.state.clone()),
    },
    ColumnDef {
        spec: TableColumn {
            key: "unique_id",
            header: "UNIQUE ID",
            min_width: 8,
            // A 21-digit opaque token: stable where the email is reusable, so
            // it earns its cell, but it is the first to drop on a narrow
            // terminal.
            preferred_width: 21,
            priority: 3,
            align: CellAlign::Left,
            sortable: true,
            fit: ColumnFit::Whole,
        },
        sort: Some(ServiceAccountSortField::UniqueId),
        cell: |row| Cell::plain(row.unique_id.clone()),
    },
];

/// The datasets table's columns.
///
/// Three. The dataset id is the identity — keyed the literal
/// [`IDENTITY_COLUMN`] (`"name"`) while its header says `DATASET`, because the
/// key is what the column-layout config and the identity-pinning rule match on
/// and so cannot vary by service — the location is the other fact an operator
/// scans a dataset list for, since it decides which region a query can join
/// against, and the labels are what a governed BigQuery estate tells its
/// datasets apart by.
///
/// The friendly name has no column, which is the call the BigQuery slice left to
/// this task. It is not that it is optional — `DISPLAY NAME` on the service
/// accounts table is optional and earns its cell — it is that it is
/// *unpopulated*: in the captured `bq ls -d` listing at
/// `tests/fixtures/bigquery_datasets.json`, one dataset of four carries the
/// key at all and it carries the empty string, so the column would be a column
/// of dashes bidding for width against labels that every one of those four
/// datasets does set. It stays searchable — an operator who knows a dataset by
/// its display name can still type it — and the preview pane spells it out
/// beside the location and the project, where it costs the table nothing.
/// Searchable-without-a-column is the safe direction of the never-drop-a-row
/// rule: it can only widen what matches, never hide a row that is on screen.
///
/// `labels` is the one column here that does not sort, for the reason its
/// siblings on secrets and Pub/Sub do not: a flattened `k=v, k=v` cell would
/// sort by whichever key happens to sort first, which is an order that means
/// nothing.
pub static BIGQUERY_DATASET_COLUMNS: &[ColumnDef<
    BigQueryDatasetRowProjection,
    BigQuerySortField,
>] = &[
    ColumnDef {
        spec: TableColumn {
            key: "name",
            header: "DATASET",
            min_width: 6,
            preferred_width: 30,
            priority: 0,
            align: CellAlign::Left,
            sortable: true,
            fit: ColumnFit::Whole,
        },
        sort: Some(BigQuerySortField::DatasetId),
        cell: |row| Cell::plain(row.name.clone()),
    },
    ColumnDef {
        spec: TableColumn {
            key: "location",
            header: "LOCATION",
            min_width: 6,
            // `europe-southwest1` is the long spelling; a multi-region is two
            // characters, so the column is sized for the region form.
            preferred_width: 18,
            priority: 1,
            align: CellAlign::Left,
            sortable: true,
            fit: ColumnFit::Whole,
        },
        sort: Some(BigQuerySortField::Location),
        // `Cell::optional` rather than `plain`: a dataset whose listing
        // carried no location parses as the empty string, and an empty cell
        // reads as a rendering fault where the dash reads as "not reported".
        cell: |row| Cell::optional(row.location.clone()),
    },
    ColumnDef {
        spec: TableColumn {
            key: "labels",
            header: "LABELS",
            min_width: 6,
            // Governance labels run long — `dg_data_confidentiality=public` is
            // 30 characters on its own — so this is the widest cell and, at
            // the lowest priority of the three, the first the layout drops.
            preferred_width: 30,
            priority: 2,
            align: CellAlign::Left,
            sortable: false,
            fit: ColumnFit::Cut,
        },
        sort: None,
        // `Cell::plain`: the projection has already resolved a dataset with no
        // labels to the dash, so there is no absence left for `optional` to
        // paint.
        cell: |row| Cell::plain(row.labels.clone()),
    },
];

/// The tables table's columns: the drill-down inside a dataset.
///
/// Six, and none of them sortable — the second type parameter is
/// [`Infallible`], so `sort: None` is the only value the field can take and
/// the unsortability is a property of the type rather than of six fields
/// that happen to be `None`. The tables view is a drill-down list with no
/// column navigation and no sort field, the same shape the Cloud Storage
/// object browser has; there is therefore no `SortField` here and no
/// `COLUMN_COUNT` beside it. A count constant is a sort field's
/// (`from_column`'s domain bound), and a fresh one that no code in `src/`
/// reads is exactly the artefact the BigQuery slice shipped disagreeing with its
/// catalogue for a whole slice. The catalogue's length and shape are pinned
/// directly instead, in `tests/column_catalog_test.rs`.
///
/// The table id is the identity, keyed the literal [`IDENTITY_COLUMN`] like
/// every other catalogue's; `TYPE` is what tells a view from a table from a
/// snapshot, which is the fact that changes what a reader may assume about a
/// row; and `CREATED` is the only other thing `bq ls <dataset>` answers.
///
/// `ROWS`, `BYTES` and `LAST MODIFIED` come from somewhere else: each row's
/// own `bq show`, one call per listed table, issued after the listing lands.
/// That is why they are last in the catalogue and last in priority — a `show`
/// refused on `bigquery.tables.get` dashes all three for every row at once,
/// while the two columns before them come from the listing this view already
/// required to succeed, so narrow width buys more spent there. Each cell is
/// [`Cell::optional`] over a projection field that is already `None` for an
/// unavailable stat: the placeholder is reached before any formatter runs, and
/// a `0` from this catalogue always means a table that reported zero.
///
/// Labels have no column here though the dataset table gives them one. They
/// are searchable off the row, the safe direction of the never-drop-a-row
/// rule, and the reason is width: the captured listing's labelled row carries
/// six `dataplex-*` keys whose flattened cell runs past 300 characters, which
/// would bid against `TABLE` for the whole width of the pane.
pub static BIGQUERY_TABLE_COLUMNS: &[ColumnDef<BigQueryTableRowProjection, Infallible>] = &[
    ColumnDef {
        spec: TableColumn {
            // Keyed "name" because that is `IDENTITY_COLUMN`; the header says
            // what the reader sees.
            key: "name",
            header: "TABLE",
            min_width: 6,
            preferred_width: 34,
            priority: 0,
            align: CellAlign::Left,
            sortable: false,
            fit: ColumnFit::Whole,
        },
        sort: None,
        cell: |row| Cell::plain(row.name.clone()),
    },
    ColumnDef {
        spec: TableColumn {
            key: "type",
            header: "TYPE",
            min_width: 4,
            // `Materialized view` is the long known spelling at 17; an
            // unrecognised kind arrives verbatim and may be longer, and the
            // layout elides it rather than this column growing for a kind
            // nobody has seen.
            preferred_width: 18,
            // Ahead of `CREATED`, which is the order the paragraph above
            // argues for: `plan_table` drops the largest priority first, so
            // the column that changes what a reader may assume about a row
            // outranks the one that dates it.
            priority: 1,
            align: CellAlign::Left,
            sortable: false,
            fit: ColumnFit::Whole,
        },
        sort: None,
        cell: |row| Cell::plain(row.kind.clone()),
    },
    ColumnDef {
        spec: TableColumn {
            key: "created",
            header: "CREATED",
            min_width: 8,
            preferred_width: 22,
            priority: 2,
            align: CellAlign::Left,
            sortable: false,
            fit: ColumnFit::Whole,
        },
        sort: None,
        // `Cell::optional`: a row whose `creationTime` was absent, empty or
        // unparseable is still a table worth listing, and the dash reads as
        // "not reported" where a blank reads as a rendering fault.
        cell: |row| Cell::optional(row.created.clone()),
    },
    ColumnDef {
        spec: TableColumn {
            key: "rows",
            header: "ROWS",
            min_width: 4,
            // A billion rows is ten digits; the count is plain digits with no
            // grouping, matching every other count column in the binary.
            preferred_width: 12,
            priority: 3,
            // Right-aligned for the reason the disks table's `SIZE` is: an
            // operator scanning for the big tables reads the column as a bar
            // chart, which a left-aligned `9` beside `1123456` defeats.
            align: CellAlign::Right,
            sortable: false,
            fit: ColumnFit::Whole,
        },
        sort: None,
        // `Cell::optional`, and the `None` it dashes is `TableStat::Unavailable`
        // — this table's `bq show` was refused or has not answered. A `0` here
        // would say the table is empty, which is a different claim and one
        // nothing has established.
        cell: |row| Cell::optional(row.rows.clone()),
    },
    ColumnDef {
        spec: TableColumn {
            key: "bytes",
            header: "BYTES",
            // `999.9 GiB` is nine characters, and the header is five.
            min_width: 5,
            preferred_width: 10,
            priority: 4,
            align: CellAlign::Right,
            sortable: false,
            fit: ColumnFit::Whole,
        },
        sort: None,
        // Humanised upstream by the same helper the Cloud Storage object
        // browser's `SIZE` column uses; `0 B` is a table with nothing in it and
        // the dash is one nobody measured.
        cell: |row| Cell::optional(row.bytes.clone()),
    },
    ColumnDef {
        spec: TableColumn {
            // The key is the lowercased header, space included, as every
            // catalogue key checked by `assert_catalog_invariants` is. Nothing
            // parses this one: the tables view has no `ResourceServiceKind`, so
            // it is absent from `catalog_meta` and no configured column list
            // can name it.
            key: "last modified",
            header: "LAST MODIFIED",
            min_width: 13,
            // `2026-08-21T10:03:33Z` is twenty, the same instant `CREATED`
            // paints and the same width it asks for.
            preferred_width: 22,
            priority: 5,
            align: CellAlign::Left,
            sortable: false,
            fit: ColumnFit::Whole,
        },
        sort: None,
        cell: |row| Cell::optional(row.last_modified.clone()),
    },
];

/// The schema drill-down's columns: the four facts a table's `bq show`
/// response carries about each field.
///
/// Unsortable and unsorted, and the second is the stronger claim. The rows
/// arrive in schema order, and that order is information: a record's children
/// follow it, so `event_params` is read together with the fields nested inside
/// it. Any ordering imposed here — alphabetical by path, grouped by type —
/// scatters a record's children away from the record. The tables level above
/// leaves `bq ls` order alone for the same reason, and like it this view has
/// no `ResourceServiceKind`, so the second type parameter is [`Infallible`]
/// and `sort: None` is the only value a column can carry.
pub static BIGQUERY_SCHEMA_COLUMNS: &[ColumnDef<BigQuerySchemaFieldRowProjection, Infallible>] = &[
    ColumnDef {
        spec: TableColumn {
            // Keyed "name" because that is `IDENTITY_COLUMN`; the header says
            // what the reader sees, and what the reader sees is the whole
            // dotted path rather than the leaf name.
            key: "name",
            header: "FIELD",
            min_width: 5,
            // Nested paths are what this column is wide for:
            // `event_params.value.string_value` is thirty-one, and a schema three
            // records deep runs past that. It is priority 0, so `plan_table`
            // fills it before anything else takes slack.
            preferred_width: 40,
            priority: 0,
            align: CellAlign::Left,
            sortable: false,
            fit: ColumnFit::Whole,
        },
        sort: None,
        cell: |row| Cell::plain(row.name.clone()),
    },
    ColumnDef {
        spec: TableColumn {
            key: "type",
            header: "TYPE",
            min_width: 4,
            // `TIMESTAMP` and `GEOGRAPHY` are nine, and an unrecognised type
            // arrives verbatim from the API.
            preferred_width: 12,
            priority: 1,
            align: CellAlign::Left,
            sortable: false,
            fit: ColumnFit::Whole,
        },
        sort: None,
        cell: |row| Cell::plain(row.field_type.clone()),
    },
    ColumnDef {
        spec: TableColumn {
            key: "mode",
            header: "MODE",
            min_width: 4,
            // The three modes the API documents — `NULLABLE`, `REQUIRED` and
            // `REPEATED` — are all eight.
            preferred_width: 10,
            priority: 2,
            align: CellAlign::Left,
            sortable: false,
            fit: ColumnFit::Whole,
        },
        sort: None,
        cell: |row| Cell::plain(row.mode.clone()),
    },
    ColumnDef {
        spec: TableColumn {
            key: "description",
            header: "DESCRIPTION",
            // The header is eleven and the cell is free text of no bounded
            // length, so this is the column the layout elides.
            min_width: 11,
            preferred_width: 44,
            // Largest priority, and `plan_table` drops the largest first: a
            // narrow frame keeps the three facts that identify a field and
            // spends what is left on the prose about it. The preview pane
            // beside the list carries the description unelided either way.
            priority: 3,
            align: CellAlign::Left,
            sortable: false,
            fit: ColumnFit::Cut,
        },
        sort: None,
        // `Cell::optional`: a field the schema documents with nothing paints
        // the dash, which reads as "no description" where a blank reads as a
        // rendering fault. An *empty* description stays `Some("")` all the way
        // from the parser and is not folded into the absence — see
        // `BigQuerySchemaFieldRowProjection`. The text itself was sanitized at
        // the projection, as every free-text cell in this binary is.
        cell: |row| Cell::optional(row.description.clone()),
    },
];

/// The policy table's columns. Four, deliberately: this is a 188-row table and
/// `deleted` / `is_public` ride as badges rather than columns, so the
/// narrow-mode budget is spent on the values that tell rows apart.
pub static IAM_POLICY_COLUMNS: &[ColumnDef<IamPolicyRowProjection, IamPolicySortField>] = &[
    ColumnDef {
        spec: TableColumn {
            // Keyed "name" because that is IDENTITY_COLUMN and
            // `config::columns` rejects a configured list without it; the
            // header is what the reader sees.
            key: "name",
            header: "MEMBER",
            min_width: 6,
            // `serviceAccount:sa@project.iam.gserviceaccount.com` is the long
            // spelling, and the member is the row's identity, so it gets the
            // room first.
            //
            // The badges ride inside this cell and yet the width is *not*
            // raised to fit badge-plus-member: a real `deleted:` member runs
            // past 100 characters with its uid, so no width buys that
            // property, and this column is priority 0 — `plan_table` grows it
            // to its full target before ROLE receives any slack at all, so
            // every character added here comes out of the role an audit reads
            // beside the member. The badge leads the cell, so it survives the
            // elision that a long member takes.
            preferred_width: 48,
            priority: 0,
            align: CellAlign::Left,
            sortable: true,
            fit: ColumnFit::Whole,
        },
        sort: Some(IamPolicySortField::Member),
        cell: iam_policy_member_cell,
    },
    ColumnDef {
        spec: TableColumn {
            key: "role",
            header: "ROLE",
            min_width: 6,
            // `projects/p/roles/custom.name` is the long spelling; the role is
            // carried verbatim, so the column has to have room for it rather
            // than eliding a custom role into its predefined twin.
            //
            // Priority 0 beside MEMBER: a binding is a member *and* a role,
            // and a member holding two roles reads as one row twice when this
            // column is dropped. `plan_table` keeps every column at the
            // identity priority, and still gives MEMBER its room first.
            preferred_width: 40,
            priority: 0,
            align: CellAlign::Left,
            sortable: true,
            fit: ColumnFit::Whole,
        },
        sort: Some(IamPolicySortField::Role),
        cell: |row| Cell::plain(row.role.clone()),
    },
    ColumnDef {
        spec: TableColumn {
            key: "kind",
            header: "TYPE",
            min_width: 4,
            preferred_width: 16,
            priority: 2,
            align: CellAlign::Left,
            sortable: true,
            fit: ColumnFit::Whole,
        },
        sort: Some(IamPolicySortField::Kind),
        cell: |row| Cell::plain(row.kind.clone()),
    },
    ColumnDef {
        spec: TableColumn {
            key: "condition",
            header: "CONDITION",
            min_width: 4,
            preferred_width: 24,
            priority: 3,
            align: CellAlign::Left,
            sortable: true,
            fit: ColumnFit::Cut,
        },
        sort: Some(IamPolicySortField::Condition),
        // `Cell::optional` paints a dash for the unconditional majority, the
        // way an absent display name does on the service accounts table —
        // never Rust's `None`.
        cell: |row| Cell::optional(row.condition.clone()),
    },
];

/// The policy table's identity cell: the badges the row wears, then the member.
///
/// The badges annotate the identity rather than taking columns of their own —
/// a four-column table is what leaves the member and the role room to be read
/// — and they lead the cell for two reasons. A left edge is what an audit
/// scans down, and the cell is the one that elides when the terminal narrows,
/// so a trailing badge would be the first thing lost on exactly the rows it
/// exists to mark.
///
/// Badged rows carry [`StatusSemantic::Warning`], the same styling the disks
/// table gives an `ORPHAN` row: an unbounded grant and a grant to a principal
/// that no longer exists are both stale policy, not errors.
///
/// Both badges can land on one cell — `deleted:allUsers` is a member the wire
/// permits — so they are joined rather than chosen between.
fn iam_policy_member_cell(row: &IamPolicyRowProjection) -> Cell {
    let mut badges: Vec<&str> = Vec::new();
    if row.is_public {
        badges.push(PUBLIC_BADGE);
    }
    if row.deleted {
        badges.push(DELETED_BADGE);
    }
    if badges.is_empty() {
        return Cell::plain(row.name.clone());
    }
    Cell::status(
        format!("{} {}", badges.join(" "), row.name),
        StatusSemantic::Warning,
    )
}

#[cfg(test)]
mod engine_cell_tests {
    use super::cloud_sql_engine_cell;

    #[test]
    fn the_engine_cell_reads_the_version_out_of_the_constant() {
        for (engine, version, cell) in [
            ("PostgreSQL", "POSTGRES_15", "PostgreSQL 15"),
            ("MySQL", "MYSQL_8_0", "MySQL 8.0"),
            ("MySQL", "MYSQL_8_0_31", "MySQL 8.0.31"),
            (
                "SQL Server",
                "SQLSERVER_2019_STANDARD",
                "SQL Server 2019 Standard",
            ),
            ("SQL Server", "SQLSERVER_2022_WEB", "SQL Server 2022 Web"),
            // Nothing to read a version out of: shown as it came.
            ("PostgreSQL", "POSTGRES", "PostgreSQL (POSTGRES)"),
            ("PostgreSQL", "", "PostgreSQL"),
        ] {
            assert_eq!(cloud_sql_engine_cell(engine, version), cell, "{version}");
        }
    }
}
