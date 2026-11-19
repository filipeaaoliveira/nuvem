//! Guards on the per-service column catalogs (task 1).
//!
//! Two things are pinned here. First, the catalogs and the per-service
//! `SortField` enums encode the same fact twice — a column's index — so every
//! catalog is checked against `SortField::column()` / `from_column()` position
//! by position. Second, the *rendered* default table: the column specs each
//! service's `TableModel` carries are written out literally, captured from the
//! behaviour before the catalogs existed. Later tasks reorder and subset the
//! catalog under configuration; with no configuration present these lists must
//! not move.

use std::convert::Infallible;

use nuvem::app::projection::columns::{
    BIGQUERY_DATASET_COLUMNS, BIGQUERY_SCHEMA_COLUMNS, BIGQUERY_TABLE_COLUMNS,
    CLOUD_FUNCTION_COLUMNS, CLOUD_RUN_COLUMNS, CLOUD_SQL_COLUMNS, COMPUTE_COLUMNS, ColumnDef,
    GKE_COLUMNS, IDENTITY_COLUMN, PERSISTENT_DISK_COLUMNS, SECRET_COLUMNS, STORAGE_BUCKET_COLUMNS,
    catalog_meta,
};
use nuvem::app::projection::project_resources;
use nuvem::app::projection::table::{CellAlign, CellKind, StatusSemantic, TableColumn, TableModel};
use nuvem::app::reducer::root::reduce;
use nuvem::app::state::resources::{
    BigQuerySortField, CloudFunctionSortField, CloudRunSortField, CloudSqlInstanceSortField,
    ComputeInstanceSortField, GkeSortField, SecretSortField, StorageSortField,
};
use nuvem::app::{Action, AppState, EffectResult, ResourcesAction};
use nuvem_domain::gcp::configuration::{GcloudConfiguration, GcloudConfigurationName};
use nuvem_domain::gcp::resource::{
    BigQueryDataset, CloudFunction, CloudFunctionEnvironment, CloudFunctionState, CloudRunService,
    CloudRunServiceStatus, CloudSqlAvailability, CloudSqlInstance, CloudSqlStatus, ComputeInstance,
    ComputeKeyValue, DiskStatus, EndpointVisibility, FirewallAction, FirewallDirection,
    FirewallRule, FirewallTargets, GkeCluster, GkeClusterStatus, IamPolicyMember, PersistentDisk,
    PubSubDeadLetterPolicy, PubSubDelivery, PubSubSubscription, PubSubSubscriptionState,
    PubSubTopic, ResourceServiceKind, ResourceStatus, Secret, ServiceAccount, StorageBucket,
    SubscriptionCount, VpcNetwork, VpcRoutingMode, VpcSubnet, VpcSubnetMode,
};

/// One row of the golden default column list: everything `TableColumn` carries.
type Golden = (&'static str, &'static str, u16, u16, u8, CellAlign, bool);

fn golden(columns: &[TableColumn]) -> Vec<Golden> {
    columns
        .iter()
        .map(|column| {
            (
                column.key,
                column.header,
                column.min_width,
                column.preferred_width,
                column.priority,
                column.align,
                column.sortable,
            )
        })
        .collect()
}

/// Asserts that a catalog agrees with its service's `SortField` enum position
/// by position, that its keys are unique and non-empty, and that each key is
/// the lowercased header (the vocabulary column configuration will use).
///
/// The identity column is the one exception to the key/header rule: its key is
/// pinned to [`IDENTITY_COLUMN`] because `config::columns` matches on it, while
/// its header is free to say what the reader needs — `DATASET` for BigQuery,
/// `MEMBER` for the policy table. Every other column must still be keyed by its
/// own header.
fn assert_catalog_invariants<R, S>(
    service: &str,
    catalog: &[ColumnDef<R, S>],
    from_column: fn(usize) -> Option<S>,
    column: fn(S) -> usize,
    column_count: usize,
) where
    S: Copy + PartialEq + std::fmt::Debug,
{
    assert_eq!(
        catalog.len(),
        column_count,
        "{service}: SortField::COLUMN_COUNT must match the catalog length"
    );

    let mut seen: Vec<&str> = Vec::new();
    for (index, def) in catalog.iter().enumerate() {
        assert!(
            !def.spec.key.is_empty(),
            "{service}: column {index} has an empty key"
        );
        assert!(
            !seen.contains(&def.spec.key),
            "{service}: duplicate column key {:?}",
            def.spec.key
        );
        seen.push(def.spec.key);
        assert!(
            def.spec.key == def.spec.header.to_lowercase() || def.spec.key == IDENTITY_COLUMN,
            "{service}: column key {:?} must be the lowercased header {:?}",
            def.spec.key,
            def.spec.header
        );

        assert_eq!(
            def.spec.sortable,
            def.sort.is_some(),
            "{service}: column {:?} sortable flag disagrees with its sort field",
            def.spec.key
        );
        assert_eq!(
            def.sort,
            from_column(index),
            "{service}: column {:?} disagrees with SortField::from_column({index})",
            def.spec.key
        );
        if let Some(field) = def.sort {
            assert_eq!(
                column(field),
                index,
                "{service}: SortField::column() disagrees with the catalog index of {:?}",
                def.spec.key
            );
        }
    }
}

#[test]
fn compute_catalog_agrees_with_its_sort_field() {
    assert_catalog_invariants(
        "compute",
        COMPUTE_COLUMNS,
        ComputeInstanceSortField::from_column,
        ComputeInstanceSortField::column,
        ComputeInstanceSortField::COLUMN_COUNT,
    );
}

#[test]
fn gke_catalog_agrees_with_its_sort_field() {
    assert_catalog_invariants(
        "gke",
        GKE_COLUMNS,
        GkeSortField::from_column,
        GkeSortField::column,
        GkeSortField::COLUMN_COUNT,
    );
}

#[test]
fn cloud_sql_catalog_agrees_with_its_sort_field() {
    assert_catalog_invariants(
        "cloudsql",
        CLOUD_SQL_COLUMNS,
        CloudSqlInstanceSortField::from_column,
        CloudSqlInstanceSortField::column,
        CloudSqlInstanceSortField::COLUMN_COUNT,
    );
}

#[test]
fn cloud_run_catalog_agrees_with_its_sort_field() {
    assert_catalog_invariants(
        "cloudrun",
        CLOUD_RUN_COLUMNS,
        CloudRunSortField::from_column,
        CloudRunSortField::column,
        CloudRunSortField::COLUMN_COUNT,
    );
}

#[test]
fn secrets_catalog_agrees_with_its_sort_field() {
    assert_catalog_invariants(
        "secrets",
        SECRET_COLUMNS,
        SecretSortField::from_column,
        SecretSortField::column,
        SecretSortField::COLUMN_COUNT,
    );
}

#[test]
fn storage_bucket_catalog_agrees_with_its_sort_field() {
    assert_catalog_invariants(
        "storage",
        STORAGE_BUCKET_COLUMNS,
        StorageSortField::from_column,
        StorageSortField::column,
        StorageSortField::COLUMN_COUNT,
    );
}

#[test]
fn cloud_functions_catalog_agrees_with_its_sort_field() {
    assert_catalog_invariants(
        "functions",
        CLOUD_FUNCTION_COLUMNS,
        CloudFunctionSortField::from_column,
        CloudFunctionSortField::column,
        CloudFunctionSortField::COLUMN_COUNT,
    );
}

/// The datasets catalogue paints three columns and only two of them sort, so
/// `COLUMN_COUNT` is the catalogue length rather than the variant count. Held
/// here rather than in a comment beside the constant, which is how a stale
/// value survived the slice that introduced it.
#[test]
fn bigquery_catalog_agrees_with_its_sort_field() {
    assert_catalog_invariants(
        "bigquery",
        BIGQUERY_DATASET_COLUMNS,
        BigQuerySortField::from_column,
        BigQuerySortField::column,
        BigQuerySortField::COLUMN_COUNT,
    );
}

/// The tables drill-down's catalogue: six columns, none of them sortable.
///
/// It has no `SortField` and therefore no `COLUMN_COUNT` to disagree with it.
/// That is deliberate: a count constant is a sort field's — it is the bound
/// `from_column` is total below — and the tables view has no sort field,
/// because it is a drill-down list with no column navigation (the second type
/// parameter is [`Infallible`], so `sort: None` is the only value a column can
/// carry). Minting a constant anyway would be minting exactly the artefact
/// the BigQuery slice shipped disagreeing with its catalogue for a whole slice:
/// one that nothing in `src/` reads. The length and the identity of the six
/// columns are pinned here against the catalogue itself instead.
#[test]
fn the_bigquery_tables_catalogue_is_six_unsortable_columns() {
    assert_catalog_invariants(
        "bigquery tables",
        BIGQUERY_TABLE_COLUMNS,
        // Uninhabited, so this is the only total definition: no index sorts.
        |_| None,
        |field: Infallible| match field {},
        6,
    );

    let columns = BIGQUERY_TABLE_COLUMNS
        .iter()
        .map(|column| (column.spec.key, column.spec.header))
        .collect::<Vec<_>>();
    assert_eq!(
        columns,
        vec![
            ("name", "TABLE"),
            ("type", "TYPE"),
            ("created", "CREATED"),
            ("rows", "ROWS"),
            ("bytes", "BYTES"),
            ("last modified", "LAST MODIFIED"),
        ],
        "the tables table is the three columns `bq ls <dataset>` answers — \
         TABLE, TYPE and CREATED — followed by the three each row's own `bq \
         show` fills in, in that order"
    );
}

/// The schema drill-down's catalogue: four columns, none of them sortable.
///
/// Unsortable for the reason the tables catalogue is, one level up — the
/// schema view has no sort field, so the second type parameter is
/// [`Infallible`] and `sort: None` is the only value a column can carry — and
/// unsorted for a reason of its own: schema order is what nests a record's
/// children under it, and any ordering imposed here would scatter them.
#[test]
fn the_bigquery_schema_catalogue_is_four_unsortable_columns() {
    assert_catalog_invariants(
        "bigquery schema",
        BIGQUERY_SCHEMA_COLUMNS,
        // Uninhabited, so this is the only total definition: no index sorts.
        |_| None,
        |field: Infallible| match field {},
        4,
    );

    let columns = BIGQUERY_SCHEMA_COLUMNS
        .iter()
        .map(|column| (column.spec.key, column.spec.header))
        .collect::<Vec<_>>();
    assert_eq!(
        columns,
        vec![
            ("name", "FIELD"),
            ("type", "TYPE"),
            ("mode", "MODE"),
            ("description", "DESCRIPTION"),
        ],
        "the schema table is the four facts a table's `bq show` response \
         carries about each field, in the order it carries them"
    );
}

/// The kind-erased view config validation will consume must be derived from the
/// typed catalogs, not maintained beside them.
#[test]
fn catalog_meta_mirrors_the_typed_catalogs() {
    let cases: Vec<(ResourceServiceKind, Vec<(&str, bool)>)> = vec![
        (
            ResourceServiceKind::ComputeInstances,
            COMPUTE_COLUMNS
                .iter()
                .map(|d| (d.spec.key, d.spec.sortable))
                .collect(),
        ),
        (
            ResourceServiceKind::GkeClusters,
            GKE_COLUMNS
                .iter()
                .map(|d| (d.spec.key, d.spec.sortable))
                .collect(),
        ),
        (
            ResourceServiceKind::CloudSql,
            CLOUD_SQL_COLUMNS
                .iter()
                .map(|d| (d.spec.key, d.spec.sortable))
                .collect(),
        ),
        (
            ResourceServiceKind::CloudRunServices,
            CLOUD_RUN_COLUMNS
                .iter()
                .map(|d| (d.spec.key, d.spec.sortable))
                .collect(),
        ),
        (
            ResourceServiceKind::CloudStorage,
            STORAGE_BUCKET_COLUMNS
                .iter()
                .map(|d| (d.spec.key, d.spec.sortable))
                .collect(),
        ),
        (
            ResourceServiceKind::SecretManagerSecrets,
            SECRET_COLUMNS
                .iter()
                .map(|d| (d.spec.key, d.spec.sortable))
                .collect(),
        ),
        (
            ResourceServiceKind::CloudFunctions,
            CLOUD_FUNCTION_COLUMNS
                .iter()
                .map(|d| (d.spec.key, d.spec.sortable))
                .collect(),
        ),
        (
            ResourceServiceKind::PersistentDisks,
            PERSISTENT_DISK_COLUMNS
                .iter()
                .map(|d| (d.spec.key, d.spec.sortable))
                .collect(),
        ),
    ];

    for (kind, expected) in cases {
        let meta: Vec<(&str, bool)> = catalog_meta(kind)
            .iter()
            .map(|m| (m.key, m.sortable))
            .collect();
        assert_eq!(
            meta, expected,
            "{kind:?}: metadata drifted from the catalog"
        );
    }
}

/// `catalog_meta` is memoized; repeated calls must hand back the same slice
/// rather than rebuilding (and leaking) one per call.
#[test]
fn catalog_meta_is_built_once_per_kind() {
    for kind in ResourceServiceKind::ALL {
        let first = catalog_meta(kind);
        let second = catalog_meta(kind);
        assert!(
            std::ptr::eq(first, second),
            "{kind:?}: metadata should be memoized"
        );
    }
}

// --- Golden default tables -------------------------------------------------

fn config(name: &str, project: &str) -> GcloudConfiguration {
    GcloudConfiguration {
        name: GcloudConfigurationName::new(name),
        is_active: true,
        account: Some(format!("{name}@example.com")),
        project: Some(project.to_string()),
        region: Some("europe-west1".to_string()),
        zone: Some("europe-west1-b".to_string()),
    }
}

/// An app parked on `service` with one loaded row, ready to project a table.
///
/// Every field the row's columns read has a value distinct from every other
/// field of the same row, so no header/cell pairing can slip by a position and
/// still match. Exactly one optional column per service is deliberately absent
/// (compute's external IP, GKE's node count, Cloud Run's latest revision) so
/// the alignment check pins a `Missing` cell too.
fn app_with_rows(service: ResourceServiceKind) -> AppState {
    let mut app = AppState::new();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ConfigurationsLoaded {
            configurations: vec![config("dev", "dev-project")],
        }),
    );
    reduce(
        &mut app,
        &Action::Resources(ResourcesAction::SelectService(service)),
    );
    let context = app.root.current_context().unwrap().clone();
    let loaded = match service {
        ResourceServiceKind::ComputeInstances => EffectResult::ComputeInstancesLoaded {
            context,
            instances: vec![ComputeInstance {
                id: "api".to_string(),
                name: "api".to_string(),
                zone: "europe-west1-b".to_string(),
                status: ResourceStatus::Running,
                machine_type: "e2-medium".to_string(),
                internal_ip: Some("10.0.0.4".to_string()),
                external_ip: None,
            }],
        },
        ResourceServiceKind::GkeClusters => EffectResult::GkeClustersLoaded {
            context,
            clusters: vec![GkeCluster {
                name: "payments".to_string(),
                location: "europe-west1".to_string(),
                status: GkeClusterStatus::Running,
                current_master_version: Some("1.31.5-gke.1023000".to_string()),
                current_node_count: None,
                endpoint_visibility: EndpointVisibility::Public,
            }],
        },
        ResourceServiceKind::PersistentDisks => EffectResult::PersistentDisksLoaded {
            context,
            disks: vec![PersistentDisk {
                name: "detached-legacy".to_string(),
                location: "europe-west1-b".to_string(),
                status: DiskStatus::Ready,
                size_gb: 20,
                disk_type: "pd-ssd".to_string(),
                users: Vec::new(),
                labels: Vec::new(),
            }],
        },
        ResourceServiceKind::CloudFunctions => EffectResult::CloudFunctionsLoaded {
            context,
            functions: vec![CloudFunction {
                name: "thumbnailer".to_string(),
                region: "europe-west1".to_string(),
                state: CloudFunctionState::Active,
                environment: CloudFunctionEnvironment::Gen2,
                url: Some("https://europe-west1-dev.cloudfunctions.net/thumbnailer".to_string()),
                update_time: Some("2026-03-01T10:00:00Z".to_string()),
                labels: Vec::new(),
            }],
        },
        ResourceServiceKind::PubSubTopics => EffectResult::PubSubTopicsLoaded {
            context,
            topics: vec![PubSubTopic {
                name: "orders".to_string(),
                full_name: "projects/dev-project/topics/orders".to_string(),
                subscription_count: SubscriptionCount::Known(2),
                message_retention: Some("604800s".to_string()),
                kms_key: None,
                labels: Vec::new(),
            }],
        },
        ResourceServiceKind::VpcNetworks => EffectResult::VpcNetworksLoaded {
            context,
            networks: vec![VpcNetwork {
                name: "default".to_string(),
                subnet_mode: VpcSubnetMode::Auto,
                routing_mode: VpcRoutingMode::Regional,
                subnet_count: 43,
                description: None,
            }],
        },
        ResourceServiceKind::VpcSubnets => EffectResult::VpcSubnetsLoaded {
            context,
            subnets: vec![VpcSubnet {
                name: "default".to_string(),
                region: "us-central1".to_string(),
                network: "default".to_string(),
                ip_cidr_range: "10.128.0.0/20".to_string(),
                gateway_address: Some("10.128.0.1".to_string()),
                private_google_access: false,
            }],
        },
        // Captured shape: an INGRESS ALLOW rule scoped by source range, which
        // is what every rule in the fixture project looks like. The EGRESS,
        // DENY, target-tag and disabled variants are constructed rather than
        // captured, and they are exercised where their parsing is pinned
        // (tests/vpc_firewall_service_test.rs) rather than here.
        ResourceServiceKind::FirewallRules => EffectResult::FirewallRulesLoaded {
            context,
            rules: vec![FirewallRule {
                name: "default-allow-icmp".to_string(),
                network: "default".to_string(),
                direction: FirewallDirection::Ingress,
                priority: Some(65534),
                action: FirewallAction::Allow,
                targets: FirewallTargets::AllInstances,
                source_or_dest: vec!["0.0.0.0/0".to_string()],
                disabled: false,
            }],
        },
        ResourceServiceKind::PubSubSubscriptions => EffectResult::PubSubSubscriptionsLoaded {
            context,
            // Deliberately a push subscription with a dead-letter policy and no
            // retention: it exercises the `Missing` cell the alignment check
            // needs while leaving every other column populated.
            subscriptions: vec![PubSubSubscription {
                name: "orders-audit".to_string(),
                full_name: "projects/dev-project/subscriptions/orders-audit".to_string(),
                topic: "projects/dev-project/topics/orders".to_string(),
                delivery: PubSubDelivery::Push {
                    endpoint: "https://audit.example.com/pubsub".to_string(),
                },
                ack_deadline_seconds: Some(10),
                message_retention: None,
                dead_letter: Some(PubSubDeadLetterPolicy {
                    dead_letter_topic: "projects/dev-project/topics/dead-letters".to_string(),
                    max_delivery_attempts: Some(5),
                }),
                state: PubSubSubscriptionState::Active,
                labels: Vec::new(),
            }],
        },
        // The location is deliberately absent, so this row exercises the
        // `Missing` cell the alignment check needs while the identity column
        // stays populated.
        ResourceServiceKind::BigQueryDatasets => EffectResult::BigQueryDatasetsLoaded {
            context,
            datasets: vec![BigQueryDataset {
                dataset_id: "analytics_events".to_string(),
                project_id: "dev-project".to_string(),
                location: String::new(),
                friendly_name: Some("Analytics events".to_string()),
                labels: Vec::new(),
            }],
        },
        ResourceServiceKind::IamServiceAccounts => EffectResult::ServiceAccountsLoaded {
            context,
            accounts: vec![ServiceAccount {
                email: "deployer@dev-project.iam.gserviceaccount.com".to_string(),
                display_name: Some("Deployer".to_string()),
                disabled: false,
                unique_id: "100000000000000000001".to_string(),
            }],
        },
        // A public binding, so the row exercises the badged spelling of the
        // identity cell and its `Status` kind: the badge rides inside that
        // cell rather than in a column of its own, so an unbadged fixture
        // would leave the styling that makes an auditor look at the row
        // pinned nowhere.
        ResourceServiceKind::IamPolicy => EffectResult::IamPolicyLoaded {
            context,
            members: vec![IamPolicyMember::from_wire(
                "roles/artifactregistry.admin",
                "allUsers",
                None,
            )],
        },
        ResourceServiceKind::CloudSql => EffectResult::CloudSqlInstancesLoaded {
            context,
            instances: vec![CloudSqlInstance {
                name: "orders".to_string(),
                database_version: "POSTGRES_15".to_string(),
                engine: "PostgreSQL".to_string(),
                region: "europe-west1".to_string(),
                tier: "db-custom-2-7680".to_string(),
                availability: CloudSqlAvailability::Regional,
                status: CloudSqlStatus::Runnable,
                connectivity: EndpointVisibility::Private,
            }],
        },
        ResourceServiceKind::CloudRunServices => EffectResult::CloudRunServicesLoaded {
            context,
            services: vec![CloudRunService {
                name: "api".to_string(),
                region: "europe-west1".to_string(),
                url: Some("https://api.a.run.app".to_string()),
                latest_ready_revision: None,
                latest_created_revision: Some("api-00007".to_string()),
                traffic_summary: "100% -> latest".to_string(),
                status: CloudRunServiceStatus::Ready,
                ready_message: None,
            }],
        },
        ResourceServiceKind::CloudStorage => EffectResult::StorageBucketsLoaded {
            context,
            buckets: vec![StorageBucket {
                name: "alpha-bucket".to_string(),
                location: "EU".to_string(),
                location_type: Some("multi-region".to_string()),
                storage_class: "STANDARD".to_string(),
                versioning_enabled: false,
                uniform_bucket_level_access: true,
                public_access_prevention: Some("enforced".to_string()),
                time_created: None,
                updated: Some("2026-01-01T00:00:00Z".to_string()),
            }],
        },
        ResourceServiceKind::SecretManagerSecrets => EffectResult::SecretsLoaded {
            context,
            secrets: vec![Secret {
                name: "api-signing-key".to_string(),
                create_time: Some("2026-02-11T09:14:22Z".to_string()),
                labels: vec![ComputeKeyValue {
                    key: "env".to_string(),
                    value: "prod".to_string(),
                }],
                replication: "Automatic".to_string(),
            }],
        },
    };
    reduce(&mut app, &Action::EffectResult(loaded));
    app
}

fn table(service: ResourceServiceKind) -> TableModel {
    let app = app_with_rows(service);
    project_resources(&app.root.resources)
        .table
        .expect("list mode builds a table model")
}

#[test]
fn compute_default_table_columns_are_unchanged() {
    assert_eq!(
        golden(&table(ResourceServiceKind::ComputeInstances).columns),
        vec![
            ("name", "NAME", 6, 28, 0, CellAlign::Left, true),
            ("status", "STATUS", 6, 10, 1, CellAlign::Left, true),
            ("zone", "ZONE", 6, 18, 2, CellAlign::Left, true),
            (
                "machine-type",
                "MACHINE-TYPE",
                8,
                18,
                3,
                CellAlign::Left,
                true
            ),
            (
                "internal-ip",
                "INTERNAL-IP",
                8,
                15,
                4,
                CellAlign::Left,
                false
            ),
            (
                "external-ip",
                "EXTERNAL-IP",
                8,
                15,
                5,
                CellAlign::Left,
                false
            ),
        ]
    );
}

#[test]
fn gke_default_table_columns_are_unchanged() {
    assert_eq!(
        golden(&table(ResourceServiceKind::GkeClusters).columns),
        vec![
            ("name", "NAME", 6, 28, 0, CellAlign::Left, true),
            ("status", "STATUS", 6, 12, 1, CellAlign::Left, true),
            ("location", "LOCATION", 6, 18, 2, CellAlign::Left, true),
            ("version", "VERSION", 7, 20, 3, CellAlign::Left, true),
            ("nodes", "NODES", 5, 6, 4, CellAlign::Right, true),
            ("endpoint", "ENDPOINT", 7, 10, 5, CellAlign::Left, false),
        ]
    );
}

#[test]
fn cloud_sql_default_table_columns_are_unchanged() {
    assert_eq!(
        golden(&table(ResourceServiceKind::CloudSql).columns),
        vec![
            ("name", "NAME", 6, 26, 0, CellAlign::Left, true),
            ("engine", "ENGINE", 7, 20, 2, CellAlign::Left, true),
            ("region", "REGION", 6, 16, 3, CellAlign::Left, true),
            ("tier", "TIER", 6, 18, 4, CellAlign::Left, true),
            (
                "availability",
                "AVAILABILITY",
                6,
                14,
                5,
                CellAlign::Left,
                true
            ),
            (
                "connectivity",
                "CONNECTIVITY",
                7,
                12,
                6,
                CellAlign::Left,
                false
            ),
            // STATUS is shown last but dropped second-to-last: on a narrow
            // terminal a Cloud SQL row's state is worth more than its tier.
            ("status", "STATUS", 6, 12, 1, CellAlign::Left, true),
        ]
    );
}

#[test]
fn cloud_run_default_table_columns_are_unchanged() {
    assert_eq!(
        golden(&table(ResourceServiceKind::CloudRunServices).columns),
        vec![
            ("name", "NAME", 6, 28, 0, CellAlign::Left, true),
            ("status", "STATUS", 6, 12, 1, CellAlign::Left, true),
            ("region", "REGION", 6, 16, 2, CellAlign::Left, true),
            ("url", "URL", 8, 32, 3, CellAlign::Left, true),
            (
                "latest-revision",
                "LATEST-REVISION",
                10,
                22,
                4,
                CellAlign::Left,
                true
            ),
            ("traffic", "TRAFFIC", 7, 20, 5, CellAlign::Left, false),
        ]
    );
}

#[test]
fn storage_bucket_default_table_columns_are_unchanged() {
    assert_eq!(
        golden(&table(ResourceServiceKind::CloudStorage).columns),
        vec![
            ("name", "NAME", 6, 30, 0, CellAlign::Left, true),
            ("location", "LOCATION", 6, 16, 2, CellAlign::Left, true),
            ("class", "CLASS", 6, 12, 1, CellAlign::Left, true),
            ("updated", "UPDATED", 8, 22, 3, CellAlign::Left, true),
            ("versioning", "VERSIONING", 4, 10, 4, CellAlign::Left, false),
            (
                "public-access",
                "PUBLIC-ACCESS",
                6,
                16,
                5,
                CellAlign::Left,
                false
            ),
        ]
    );
}

// --- Header/cell alignment -------------------------------------------------

/// What a header must be paired with: the column key, and the text *and* kind
/// of the cell rendered under it. Kind is part of the pairing because a cell
/// that keeps its text but loses its `Status`/`Missing` kind silently loses its
/// colour — a swap the text alone cannot see.
type Aligned = (&'static str, String, CellKind);

fn aligned(table: &TableModel) -> Vec<Aligned> {
    let row = table.rows.first().expect("fixture loads one row");
    assert_eq!(
        table.columns.len(),
        row.cells.len(),
        "a row has a different number of cells than the table has headers"
    );
    table
        .columns
        .iter()
        .zip(&row.cells)
        .map(|(column, cell)| (column.key, cell.text.clone(), cell.kind.clone()))
        .collect()
}

fn plain(key: &'static str, text: &str) -> Aligned {
    (key, text.to_string(), CellKind::Plain)
}

fn missing(key: &'static str) -> Aligned {
    (key, "-".to_string(), CellKind::Missing)
}

fn status(key: &'static str, text: &str, semantic: StatusSemantic) -> Aligned {
    (key, text.to_string(), CellKind::Status(semantic))
}

/// Header *i* and cell *i* must describe the same field, for every configurable
/// service — including Cloud Storage's bucket list, whose headers and cells are
/// the one place the projection still reads the catalog at two points.
///
/// The fixture values are mutually distinguishable within each row, so a
/// pairing that slipped by one position cannot pass by coincidence, and the
/// cell *kind* is asserted alongside the text, so a `Cell::status` quietly
/// downgraded to `Cell::plain` fails here rather than in a user's terminal.
#[test]
fn every_service_pairs_each_header_with_its_own_cell() {
    let cases: Vec<(ResourceServiceKind, Vec<Aligned>)> = vec![
        (
            ResourceServiceKind::ComputeInstances,
            vec![
                plain("name", "api"),
                status("status", "Running", StatusSemantic::Healthy),
                plain("zone", "europe-west1-b"),
                plain("machine-type", "e2-medium"),
                plain("internal-ip", "10.0.0.4"),
                missing("external-ip"),
            ],
        ),
        (
            ResourceServiceKind::GkeClusters,
            vec![
                plain("name", "payments"),
                status("status", "Running", StatusSemantic::Healthy),
                plain("location", "europe-west1"),
                plain("version", "1.31.5-gke.1023000"),
                missing("nodes"),
                plain("endpoint", "Public"),
            ],
        ),
        (
            ResourceServiceKind::CloudSql,
            vec![
                plain("name", "orders"),
                plain("engine", "PostgreSQL (POSTGRES_15)"),
                plain("region", "europe-west1"),
                plain("tier", "db-custom-2-7680"),
                plain("availability", "HA (regional)"),
                plain("connectivity", "Private"),
                status("status", "Runnable", StatusSemantic::Healthy),
            ],
        ),
        (
            ResourceServiceKind::CloudRunServices,
            vec![
                plain("name", "api"),
                status("status", "Ready", StatusSemantic::Healthy),
                plain("region", "europe-west1"),
                plain("url", "https://api.a.run.app"),
                missing("latest-revision"),
                plain("traffic", "100% -> latest"),
            ],
        ),
        (
            ResourceServiceKind::CloudFunctions,
            vec![
                plain("name", "thumbnailer"),
                plain("region", "europe-west1"),
                status("state", "Active", StatusSemantic::Healthy),
                plain("generation", "2nd gen"),
                plain("updated", "2026-03-01T10:00:00Z"),
                plain(
                    "url",
                    "https://europe-west1-dev.cloudfunctions.net/thumbnailer",
                ),
            ],
        ),
        (
            // The policy table's identity cell is the one place a badge is
            // composed into another column's value, so both halves are pinned
            // here: the text (badge, space, member) and the `Status` kind that
            // colours the row an audit is meant to spot. Downgraded to
            // `Cell::plain`, the words survive and the colour does not.
            ResourceServiceKind::IamPolicy,
            vec![
                status("name", "PUBLIC allUsers", StatusSemantic::Warning),
                plain("role", "roles/artifactregistry.admin"),
                plain("kind", "All users"),
                missing("condition"),
            ],
        ),
        (
            ResourceServiceKind::CloudStorage,
            vec![
                plain("name", "alpha-bucket"),
                plain("location", "EU"),
                plain("class", "STANDARD"),
                plain("updated", "2026-01-01T00:00:00Z"),
                plain("versioning", "off"),
                plain("public-access", "enforced"),
            ],
        ),
    ];

    for (kind, expected) in cases {
        let table = table(kind);
        assert_eq!(
            aligned(&table),
            expected,
            "{kind:?}: a header is paired with another column's cell"
        );
    }
}
