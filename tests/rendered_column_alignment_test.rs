//! Configured columns as *painted characters*.
//!
//! `tests/column_layout_test.rs` stops at the projected `TableModel`: it proves
//! header *i* and cell *i* agree about which field they carry. That is a model
//! property. It says nothing about the glyphs the operator reads, because the
//! model is only half the pipeline — `plan_table` still assigns widths and
//! `render_table` still pairs each cell with a layout entry. A defect confined
//! to that render layer (cells zipped against the layout list off by one, one
//! row's cells taken from another row, a width recomputed per row instead of
//! taken from the plan) leaves the model perfectly coherent while the screen
//! shows a value under the wrong header, and every model-level test stays green.
//!
//! So this test asserts on the terminal buffer. It resolves a real config file
//! through the real config path, installs it through the real startup path,
//! renders the real UI into a `TestBackend`, then derives each header's
//! character span from the painted header line and checks that the slice of
//! each painted row under that span holds the value that header's key owns —
//! at the exact offset the column's alignment demands, and holding no other
//! value the case knows about.
//!
//! The file is one assertion engine (`check_case`) plus thin `#[test]` entry
//! points. A `Case` describes one rendered table surface: its `[columns.*]`
//! block, the column keys that block must produce, the values every fixture row
//! must paint, and a setup step that feeds those rows in through the real
//! reducer. The engine never learns the row types, so a service whose rows are
//! `GkeCluster` or a storage object page plugs in the same way a
//! `ComputeInstance` does.
//!
//! Six surfaces are covered, and they are not of equal value. Five of them —
//! the four configurable lists and the storage bucket list — guard a
//! construction that is already safe by design: headers and cells come from
//! one walk over one catalog (`projection::columns::table_parts`), so column
//! *i*'s header and column *i*'s cell are the same array element. The Cloud
//! Storage object browser is the exception and the reason this file is worth
//! its length: it is the only table whose header list and cell list are two
//! parallel arrays a human keeps in step by hand. See
//! [`storage_object_case`].

use std::io::Write;

use clap::Parser;
use nuvem::app::projection::project_resources;
use nuvem::app::projection::table::CellAlign;
use nuvem::app::reducer::root::reduce;
use nuvem::app::state::root::Workspace;
use nuvem::app::{Action, AppState, EffectResult, ResourcesAction};
use nuvem::services::runtime::apply_startup_config;
use nuvem::services::runtime::session::RuntimeSession;
use nuvem::{Cli, Config};
use nuvem_domain::gcp::configuration::{
    GcloudConfiguration, GcloudConfigurationName, GcloudContext,
};
use nuvem_domain::gcp::resource::{
    CloudRunService, CloudRunServiceStatus, CloudSqlAvailability, CloudSqlInstance, CloudSqlStatus,
    ComputeInstance, EndpointVisibility, GkeCluster, GkeClusterStatus, ResourceServiceKind,
    ResourceStatus, StorageBucket, StorageObject, StorageObjectPage, StoragePrefix,
};
use ratatui::Terminal;
use ratatui::backend::TestBackend;

/// One `[columns.<section>]` block. The section id is the config vocabulary
/// (`compute` / `gke` / `cloudsql` / `cloudrun` / `storage`, see
/// `config::config_service_id`), not the service enum, because the file is what
/// the operator writes and the file is what this test wants to exercise.
struct ColumnsBlock {
    section: &'static str,
    body: &'static str,
}

/// The expected painted values of one row, as `(column key, painted text)`.
type ExpectedRow = &'static [(&'static str, &'static str)];

/// One rendered table surface: a service, a configured layout, and the rows the
/// operator should see under it.
struct Case {
    /// Named in every panic message, because the engine is shared and a bare
    /// "column 2 painted the wrong thing" would not say which surface broke.
    name: &'static str,
    /// The service's column block, or `None` for a surface whose columns are
    /// not configurable at all — the Cloud Storage object browser hardcodes
    /// `storage_object_columns()` and ignores `[columns.storage]`, so a block
    /// there would assert nothing and mislead the next reader.
    columns: Option<ColumnsBlock>,
    /// The column keys the block must produce, in configured display order.
    /// Asserted against the projected model *before* anything is checked on the
    /// buffer: if the configured layout never reached the model, the paint
    /// assertions below would be checking the wrong columns and passing.
    expected_keys: &'static [&'static str],
    /// Every fixture row, each as its own `(key, painted text)` list. Every row
    /// must carry a `name` entry — that is how the engine finds the row's line
    /// in the frame.
    rows: &'static [ExpectedRow],
    /// Selects the service and feeds the fixture rows in through the real
    /// reducer. A plain function pointer rather than a generic parameter: the
    /// surfaces carry unrelated row types (`ComputeInstance`, `GkeCluster`, a
    /// storage object page, …) and a `Case` that named its row type would need
    /// a different engine per service. The step gets the active context because
    /// every `*Loaded` effect result is scoped to one, and it may dispatch more
    /// than one action — the object browser is reached by loading buckets,
    /// opening the selection, then delivering the object page.
    setup: fn(&mut AppState, &GcloudContext),
}

/// Width chosen with headroom. The table pane is only a slice of the frame —
/// the service sidebar and the preview pane take the rest — so the frame has to
/// be much wider than any one table's appetite. At 320x30 the table pane
/// measures 153 character cells. The hungriest surface here is the object
/// browser's seven columns, which want at most 32 + 10 + 12 + 22 + 10 + 18 + 9
/// display cells plus six 2-cell gaps and the mark and selection gutters,
/// around 130, so nothing is dropped or truncated for want of room. That
/// matters because `plan_table` drops low-priority columns when the min-width
/// line does not fit and `fit_cell` truncates a value to a prefix plus an
/// ellipsis, and either would let the assertions below pass for the wrong
/// reason. The engine does not *rely* on this arithmetic staying true: it
/// checks that every header and every full value is actually painted inside the
/// table pane, so a frame that became too narrow fails loudly instead of
/// quietly certifying less.
const FRAME_WIDTH: u16 = 320;
const FRAME_HEIGHT: u16 = 30;

// ---------------------------------------------------------------------------
// Cases
// ---------------------------------------------------------------------------

/// Both a reorder and a subset of the compute catalog, whose order is
/// name, status, zone, machine-type, internal-ip, external-ip. Reordering alone
/// would let a renderer that ignores the layout still paint plausibly; dropping
/// two columns means the painted spans cannot match catalog order by accident
/// either.
const COMPUTE_COLUMNS_BODY: &str = "columns = [\"zone\", \"internal-ip\", \"name\", \"status\"]\n";

/// Three rows, because a one-row table cannot fail the way real tables fail.
///
/// INVARIANT: every value below is pairwise non-substring with every other
/// value in the *whole case* — not merely within a row. No value may be
/// contained in any other, in either direction. The engine asserts that a
/// column's slice holds its own value and no other value the case knows about,
/// which is what makes "row 2 painted row 1's cell" red rather than green, and
/// that degrades into decoration the moment one value contains another. Do not
/// "tidy" these into realistic-looking values that share a prefix
/// (`10.0.0.4` / `10.0.0.40`, `europe-west1-b` / `europe-west1`). The engine
/// enforces the invariant at runtime, case-wide.
///
/// `status` is not free text: the projection paints `format!("{:?}", status)`
/// of a `ResourceStatus`, so the only choices are Running / Stopped /
/// Suspended / Pending. The statuses are fixed first and everything else is
/// picked around them.
///
/// The widest value of each column deliberately lands in a *different* row —
/// the widest internal-ip in the first row, the widest name in the second, the
/// widest zone in the third. Column widths come from `content_widths`, a max
/// over all rows, so a defect that recomputes a width from one row's own
/// content must actually shift a glyph; if every row's values were the same
/// length a per-row recompute would be invisible. (Four columns cannot have
/// four distinct widest rows in a three-row fixture; `status` shares the first
/// row, and its values are narrower than its header anyway.)
///
/// Every value also stays inside its column's `preferred_width` — zone caps at
/// 18, internal-ip at 15 — because a longer one would be painted as an
/// ellipsised prefix and the engine would fail on truncation rather than on the
/// property it is here to check.
const COMPUTE_ROWS: &[ExpectedRow] = &[
    &[
        ("zone", "us-west4-b"),
        ("internal-ip", "100.64.255.201"),
        ("name", "vault-node"),
        ("status", "Suspended"),
    ],
    &[
        ("zone", "asia-south2-c"),
        ("internal-ip", "192.168.5.7"),
        ("name", "ledger-worker-primary"),
        ("status", "Stopped"),
    ],
    &[
        ("zone", "europe-southwest1"),
        ("internal-ip", "172.31.6.8"),
        ("name", "kiosk-7"),
        ("status", "Pending"),
    ],
];

fn compute_case() -> Case {
    Case {
        name: "compute instances, list mode",
        columns: Some(ColumnsBlock {
            section: "compute",
            body: COMPUTE_COLUMNS_BODY,
        }),
        expected_keys: &["zone", "internal-ip", "name", "status"],
        rows: COMPUTE_ROWS,
        setup: |app, context| {
            reduce(
                app,
                &Action::Resources(ResourcesAction::SelectService(
                    ResourceServiceKind::ComputeInstances,
                )),
            );
            reduce(
                app,
                &Action::EffectResult(EffectResult::ComputeInstancesLoaded {
                    context: context.clone(),
                    instances: compute_instances(),
                }),
            );
        },
    }
}

/// The fixture instances, built from [`COMPUTE_ROWS`] so the painted
/// expectations and the loaded rows cannot drift apart.
fn compute_instances() -> Vec<ComputeInstance> {
    COMPUTE_ROWS
        .iter()
        .map(|row| {
            let field = |key: &str| {
                row.iter()
                    .find(|(row_key, _)| *row_key == key)
                    .map(|(_, value)| value.to_string())
                    .unwrap_or_else(|| panic!("compute fixture row is missing {key:?}"))
            };
            let status = match field("status").as_str() {
                "Running" => ResourceStatus::Running,
                "Stopped" => ResourceStatus::Stopped,
                "Suspended" => ResourceStatus::Suspended,
                "Pending" => ResourceStatus::Pending,
                other => panic!("{other:?} is not a ResourceStatus Debug rendering"),
            };
            ComputeInstance {
                id: field("name"),
                name: field("name"),
                zone: field("zone"),
                status,
                // Not a configured column, so never painted in the table: kept
                // uniform so it cannot become an accidental discriminator.
                machine_type: "e2-medium".to_string(),
                internal_ip: Some(field("internal-ip")),
                external_ip: None,
            }
        })
        .collect()
}

#[test]
fn compute_list_paints_every_value_under_its_own_configured_header() {
    check_case(&compute_case());
}

// --- GKE clusters ----------------------------------------------------------

/// Reorder *and* subset of the GKE catalog (name, status, location, version,
/// nodes, endpoint): version and endpoint are dropped, and `nodes` sits second
/// rather than fifth. `nodes` is the only right-aligned column in any catalog,
/// so this is the one configurable list that exercises the engine's
/// right-aligned span rule — and it is deliberately not last, so that rule is
/// checked against a real neighbour on both sides instead of against the pane
/// edge.
const GKE_COLUMNS_BODY: &str = "columns = [\"location\", \"nodes\", \"name\", \"status\"]\n";

/// `status` is `format!("{:?}", GkeClusterStatus)`, whose variants are Running
/// / Reconciling / Stopping / Error / Unknown(_) — not compute's set, so the
/// statuses are picked first and everything else is picked around them.
///
/// Widest location in row 0, widest name in row 1, widest status in row 2, so
/// a width recomputed from one row's own content has to move a glyph. `nodes`
/// cannot join that rotation: its header is wider than any node count, so the
/// column is header-sized whatever the rows hold.
///
/// The numeric column is the trap in this fixture. Node counts are short, so
/// they collide with any other value that happens to contain the same digits;
/// 308 / 47 / 6 share no digit run with each other or with the locations'
/// `5`, `1` and `2`.
const GKE_ROWS: &[ExpectedRow] = &[
    &[
        ("location", "northamerica-west1"),
        ("nodes", "308"),
        ("name", "hopper-mesh"),
        ("status", "Error"),
    ],
    &[
        ("location", "us-east5-c"),
        ("nodes", "47"),
        ("name", "quarry-fleet"),
        ("status", "Stopping"),
    ],
    &[
        ("location", "me-central2-b"),
        ("nodes", "6"),
        ("name", "zealot-ring"),
        ("status", "Reconciling"),
    ],
];

fn gke_case() -> Case {
    Case {
        name: "gke clusters, list mode",
        columns: Some(ColumnsBlock {
            section: "gke",
            body: GKE_COLUMNS_BODY,
        }),
        expected_keys: &["location", "nodes", "name", "status"],
        rows: GKE_ROWS,
        setup: |app, context| {
            reduce(
                app,
                &Action::Resources(ResourcesAction::SelectService(
                    ResourceServiceKind::GkeClusters,
                )),
            );
            reduce(
                app,
                &Action::EffectResult(EffectResult::GkeClustersLoaded {
                    context: context.clone(),
                    clusters: gke_clusters(),
                }),
            );
        },
    }
}

/// Built from [`GKE_ROWS`], so the painted expectations and the loaded rows
/// cannot drift apart.
fn gke_clusters() -> Vec<GkeCluster> {
    GKE_ROWS
        .iter()
        .map(|row| {
            let field = |key: &str| owned(row, key, "gke fixture");
            let status = match field("status").as_str() {
                "Running" => GkeClusterStatus::Running,
                "Reconciling" => GkeClusterStatus::Reconciling,
                "Stopping" => GkeClusterStatus::Stopping,
                "Error" => GkeClusterStatus::Error,
                other => panic!("{other:?} is not a GkeClusterStatus Debug rendering"),
            };
            GkeCluster {
                name: field("name"),
                location: field("location"),
                status,
                // Not configured columns, so never painted: kept uniform (and
                // absent) so they cannot become accidental discriminators.
                current_master_version: None,
                current_node_count: Some(
                    field("nodes")
                        .parse()
                        .expect("the nodes fixture value is what the projection paints, a u32"),
                ),
                endpoint_visibility: EndpointVisibility::Public,
            }
        })
        .collect()
}

#[test]
fn gke_list_paints_every_value_under_its_own_configured_header() {
    check_case(&gke_case());
}

// --- Cloud SQL instances ---------------------------------------------------

/// Reorder and subset of the Cloud SQL catalog (name, engine, region, tier,
/// availability, connectivity, status).
const CLOUD_SQL_COLUMNS_BODY: &str = "columns = [\"tier\", \"engine\", \"name\", \"status\"]\n";

/// `engine` is the reason this service is here: its cell is the only catalog
/// cell that is not a field copy. `cloud_sql_engine_cell` composes it from two
/// projection fields (`engine` and `database_version`), so the expected text
/// below is the composed string, and [`cloud_sql_instances`] splits it back
/// into the two fields it was composed from.
///
/// `status` is `cloud_sql_status_label`, not a `Debug` rendering: Runnable /
/// Stopped / Pending create / Maintenance / Failed / Suspended.
///
/// Widest tier in row 0, widest engine and status in row 1, widest name in row
/// 2. Four columns cannot each own a distinct widest row in a three-row
/// fixture; status shares row 1.
///
/// Every composed engine string stays inside `engine`'s 20-cell
/// `preferred_width` — `PostgreSQL (POSTGRES_15)` would be 24 and would be
/// painted ellipsised, failing this test on truncation rather than on
/// alignment.
const CLOUD_SQL_ROWS: &[ExpectedRow] = &[
    &[
        ("tier", "db-custom-2-7680"),
        ("engine", "MySQL (MYSQL_8_0)"),
        ("name", "orders-primary"),
        ("status", "Failed"),
    ],
    &[
        ("tier", "db-f1-micro"),
        ("engine", "MariaDB (MARIA_10_6)"),
        ("name", "warehouse-archive"),
        ("status", "Maintenance"),
    ],
    &[
        ("tier", "db-n1-highmem-4"),
        ("engine", "Postgres (PG_14)"),
        ("name", "billing-replica-eu"),
        ("status", "Runnable"),
    ],
];

fn cloud_sql_case() -> Case {
    Case {
        name: "cloud sql instances, list mode",
        columns: Some(ColumnsBlock {
            section: "cloudsql",
            body: CLOUD_SQL_COLUMNS_BODY,
        }),
        expected_keys: &["tier", "engine", "name", "status"],
        rows: CLOUD_SQL_ROWS,
        setup: |app, context| {
            reduce(
                app,
                &Action::Resources(ResourcesAction::SelectService(
                    ResourceServiceKind::CloudSql,
                )),
            );
            reduce(
                app,
                &Action::EffectResult(EffectResult::CloudSqlInstancesLoaded {
                    context: context.clone(),
                    instances: cloud_sql_instances(),
                }),
            );
        },
    }
}

/// Built from [`CLOUD_SQL_ROWS`]. The engine cell is the one expectation that
/// is not a field copy, so it is taken apart here exactly the way
/// `cloud_sql_engine_cell` puts it together — `Engine (VERSION)`. If the
/// composition ever changed, the painted text would stop matching the fixture
/// and this case would fail, which is the point of covering it.
fn cloud_sql_instances() -> Vec<CloudSqlInstance> {
    CLOUD_SQL_ROWS
        .iter()
        .map(|row| {
            let field = |key: &str| owned(row, key, "cloud sql fixture");
            let composed = field("engine");
            let (engine, database_version) = composed
                .split_once(" (")
                .map(|(engine, version)| {
                    (
                        engine.to_string(),
                        version
                            .strip_suffix(')')
                            .expect("composed engine cell ends with ')'")
                            .to_string(),
                    )
                })
                .unwrap_or_else(|| panic!("{composed:?} is not an `Engine (VERSION)` cell"));
            let status = match field("status").as_str() {
                "Runnable" => CloudSqlStatus::Runnable,
                "Stopped" => CloudSqlStatus::Stopped,
                "Pending create" => CloudSqlStatus::PendingCreate,
                "Maintenance" => CloudSqlStatus::Maintenance,
                "Failed" => CloudSqlStatus::Failed,
                "Suspended" => CloudSqlStatus::Suspended,
                other => panic!("{other:?} is not a cloud_sql_status_label rendering"),
            };
            CloudSqlInstance {
                name: field("name"),
                engine,
                database_version,
                tier: field("tier"),
                status,
                // Not configured columns, so never painted: kept uniform.
                region: "europe-west1".to_string(),
                availability: CloudSqlAvailability::Zonal,
                connectivity: EndpointVisibility::Private,
            }
        })
        .collect()
}

#[test]
fn cloud_sql_list_paints_every_value_under_its_own_configured_header() {
    check_case(&cloud_sql_case());
}

// --- Cloud Run services ----------------------------------------------------

/// Reorder and subset of the Cloud Run catalog (name, status, region, url,
/// latest-revision, traffic).
const CLOUD_RUN_COLUMNS_BODY: &str =
    "columns = [\"region\", \"latest-revision\", \"name\", \"status\"]\n";

/// `status` is `format!("{:?}", CloudRunServiceStatus)`: Ready / NotReady /
/// Reconciling / Failed / Unknown(_). `Ready` and `NotReady` can never appear
/// in the same fixture — one contains the other, and the engine's cross-row
/// exclusion would be unfalsifiable for both.
///
/// Widest region in row 0, widest revision and status in row 1, widest name in
/// row 2.
const CLOUD_RUN_ROWS: &[ExpectedRow] = &[
    &[
        ("region", "asia-northeast3"),
        ("latest-revision", "notify-00007-abc"),
        ("name", "checkout-api"),
        ("status", "Failed"),
    ],
    &[
        ("region", "us-central1"),
        ("latest-revision", "gateway-00042-xyz"),
        ("name", "audit-sink"),
        ("status", "Reconciling"),
    ],
    &[
        ("region", "europe-north2"),
        ("latest-revision", "ingest-00913-qrs"),
        ("name", "media-transcoder"),
        ("status", "Ready"),
    ],
];

fn cloud_run_case() -> Case {
    Case {
        name: "cloud run services, list mode",
        columns: Some(ColumnsBlock {
            section: "cloudrun",
            body: CLOUD_RUN_COLUMNS_BODY,
        }),
        expected_keys: &["region", "latest-revision", "name", "status"],
        rows: CLOUD_RUN_ROWS,
        setup: |app, context| {
            reduce(
                app,
                &Action::Resources(ResourcesAction::SelectService(
                    ResourceServiceKind::CloudRunServices,
                )),
            );
            reduce(
                app,
                &Action::EffectResult(EffectResult::CloudRunServicesLoaded {
                    context: context.clone(),
                    services: cloud_run_services(),
                }),
            );
        },
    }
}

/// Built from [`CLOUD_RUN_ROWS`].
fn cloud_run_services() -> Vec<CloudRunService> {
    CLOUD_RUN_ROWS
        .iter()
        .map(|row| {
            let field = |key: &str| owned(row, key, "cloud run fixture");
            let status = match field("status").as_str() {
                "Ready" => CloudRunServiceStatus::Ready,
                "NotReady" => CloudRunServiceStatus::NotReady,
                "Reconciling" => CloudRunServiceStatus::Reconciling,
                "Failed" => CloudRunServiceStatus::Failed,
                other => panic!("{other:?} is not a CloudRunServiceStatus Debug rendering"),
            };
            CloudRunService {
                name: field("name"),
                region: field("region"),
                latest_ready_revision: Some(field("latest-revision")),
                status,
                // Not configured columns, so never painted: kept uniform.
                url: None,
                latest_created_revision: None,
                traffic_summary: "100% latest".to_string(),
                ready_message: None,
            }
        })
        .collect()
}

#[test]
fn cloud_run_list_paints_every_value_under_its_own_configured_header() {
    check_case(&cloud_run_case());
}

// --- Cloud Storage bucket list ---------------------------------------------

/// Reorder and subset of the bucket catalog (name, location, class, updated,
/// versioning, public-access). The bucket list is a configurable table and
/// goes through `table_parts` like the other three; the object browser below
/// it does not.
const STORAGE_BUCKET_COLUMNS_BODY: &str =
    "columns = [\"class\", \"updated\", \"name\", \"location\"]\n";

/// Widest name in row 1, widest location in row 2. Neither `class` nor
/// `updated` can join that rotation: every storage class that fits the
/// column's 12-cell `preferred_width` is 7 or 8 characters wide, and RFC-3339
/// timestamps are all exactly 20. Two columns carrying a real width spread is
/// enough for a per-row width recompute to have to move a glyph.
const STORAGE_BUCKET_ROWS: &[ExpectedRow] = &[
    &[
        ("class", "STANDARD"),
        ("updated", "2026-01-14T09:31:00Z"),
        ("name", "tf-state-prod"),
        ("location", "EUROPE-WEST4"),
    ],
    &[
        ("class", "ARCHIVE"),
        ("updated", "2025-11-03T22:07:45Z"),
        ("name", "release-artifacts"),
        ("location", "US-WEST2"),
    ],
    &[
        ("class", "NEARLINE"),
        ("updated", "2024-07-28T16:52:39Z"),
        ("name", "customer-exports"),
        ("location", "ASIA-SOUTHEAST1"),
    ],
];

fn storage_bucket_case() -> Case {
    Case {
        name: "cloud storage buckets, list mode",
        columns: Some(ColumnsBlock {
            section: "storage",
            body: STORAGE_BUCKET_COLUMNS_BODY,
        }),
        expected_keys: &["class", "updated", "name", "location"],
        rows: STORAGE_BUCKET_ROWS,
        setup: |app, context| {
            reduce(
                app,
                &Action::Resources(ResourcesAction::SelectService(
                    ResourceServiceKind::CloudStorage,
                )),
            );
            reduce(
                app,
                &Action::EffectResult(EffectResult::StorageBucketsLoaded {
                    context: context.clone(),
                    buckets: storage_buckets(),
                }),
            );
        },
    }
}

/// Built from [`STORAGE_BUCKET_ROWS`].
fn storage_buckets() -> Vec<StorageBucket> {
    STORAGE_BUCKET_ROWS
        .iter()
        .map(|row| {
            let field = |key: &str| owned(row, key, "storage bucket fixture");
            StorageBucket {
                name: field("name"),
                location: field("location"),
                storage_class: field("class"),
                updated: Some(field("updated")),
                // Not configured columns, so never painted: kept uniform.
                location_type: None,
                versioning_enabled: false,
                uniform_bucket_level_access: true,
                public_access_prevention: None,
                time_created: None,
            }
        })
        .collect()
}

#[test]
fn storage_bucket_list_paints_every_value_under_its_own_configured_header() {
    check_case(&storage_bucket_case());
}

// --- Cloud Storage object browser ------------------------------------------

/// **This is the load-bearing case in the file.** The other five guard a
/// construction that is already safe by design: every configurable list builds
/// its headers and its cells from one walk over one catalog
/// (`columns::table_parts`), so header *i* and cell *i* are the same array
/// element and cannot desync by hand. The object browser is the single table
/// in the codebase that does not: `storage_object_columns()` and
/// `storage_object_rows()` (both in `src/app/projection/mod.rs`) are two
/// hand-maintained parallel arrays, kept in step by a human remembering to
/// edit both. Inserting a column in one and not the other silently shifts
/// every cell to its right under the wrong header. Anyone trimming this file
/// should keep this case.
///
/// It is also the only surface here with no `[columns.*]` block at all — the
/// browser is a detail mode with no column navigation and no sorting, and it
/// ignores `[columns.storage]`, which configures the bucket list behind it.
///
/// Both row kinds the browser paints are covered: a common-prefix (folder) row
/// and object rows. A prefix row carries nothing but a name — the projection
/// leaves its six other fields empty — and its painted name gains a trailing
/// `/` from `storage_object_rows`. An empty expectation is not a hole in the
/// case: the engine asserts such a cell is painted blank, which is what makes
/// "the folder row borrowed an object's size" red.
///
/// `size` is right-aligned, and its text is `format_object_size` output, not a
/// raw byte count: 971 bytes paints as `971 B` and 6 GiB as `6.0 GiB`.
const STORAGE_OBJECT_ROWS: &[ExpectedRow] = &[
    &[
        ("name", "captures/"),
        ("size", ""),
        ("class", ""),
        ("updated", ""),
        ("encryption", ""),
        ("content-type", ""),
        ("retention", ""),
    ],
    &[
        ("name", "manifest.yaml"),
        ("size", "971 B"),
        ("class", "STANDARD"),
        ("updated", "2026-02-09T04:18:27Z"),
        ("encryption", "Google"),
        ("content-type", "text/yaml"),
        ("retention", "yes"),
    ],
    &[
        ("name", "snapshot.tar.zst"),
        ("size", "6.0 GiB"),
        ("class", "COLDLINE"),
        ("updated", "2023-12-30T11:45:06Z"),
        ("encryption", "CMEK"),
        ("content-type", "application/x-tar"),
        ("retention", ""),
    ],
];

/// The bucket the browser is opened on. Only one is loaded so that
/// `OpenSelected` cannot open a different one than the object page is
/// delivered for — the reducer drops a page whose (bucket, prefix) does not
/// match the browser's current scope, and a dropped page would leave an empty
/// table rather than a wrong one.
const STORAGE_OBJECT_BUCKET: &str = "vault-cold-store";

fn storage_object_case() -> Case {
    Case {
        name: "cloud storage object browser",
        columns: None,
        expected_keys: &[
            "name",
            "size",
            "class",
            "updated",
            "encryption",
            "content-type",
            "retention",
        ],
        rows: STORAGE_OBJECT_ROWS,
        setup: |app, context| {
            reduce(
                app,
                &Action::Resources(ResourcesAction::SelectService(
                    ResourceServiceKind::CloudStorage,
                )),
            );
            reduce(
                app,
                &Action::EffectResult(EffectResult::StorageBucketsLoaded {
                    context: context.clone(),
                    buckets: vec![StorageBucket {
                        name: STORAGE_OBJECT_BUCKET.to_string(),
                        location: "EUROPE-WEST4".to_string(),
                        location_type: None,
                        storage_class: "STANDARD".to_string(),
                        versioning_enabled: false,
                        uniform_bucket_level_access: true,
                        public_access_prevention: None,
                        time_created: None,
                        updated: None,
                    }],
                }),
            );
            // The real path into the browser: opening the selected bucket is
            // what switches the view, and only then does a page load apply.
            reduce(app, &Action::Resources(ResourcesAction::OpenSelected));
            reduce(
                app,
                &Action::EffectResult(EffectResult::StorageObjectPageLoaded {
                    context: context.clone(),
                    bucket: STORAGE_OBJECT_BUCKET.to_string(),
                    prefix: String::new(),
                    append: false,
                    page: storage_object_page(),
                }),
            );
        },
    }
}

/// Built from [`STORAGE_OBJECT_ROWS`]: a row whose `size` expectation is empty
/// is a common prefix (the projection blanks every other field for one), and
/// any other row is an object. The painted `size`, `encryption` and
/// `retention` texts are derived quantities, so the fixture maps each expected
/// text back to the input that produces it; if `format_object_size` or either
/// label ever changed, the painted cell would stop matching and this case would
/// fail rather than silently certify the new rendering.
fn storage_object_page() -> StorageObjectPage {
    let mut prefixes = Vec::new();
    let mut objects = Vec::new();
    for row in STORAGE_OBJECT_ROWS {
        let field = |key: &str| owned(row, key, "storage object fixture");
        let name = field("name");
        if field("size").is_empty() {
            prefixes.push(StoragePrefix { name });
            continue;
        }
        let size_bytes = match field("size").as_str() {
            "971 B" => 971,
            "6.0 GiB" => 6 * 1024 * 1024 * 1024,
            other => panic!("{other:?} has no byte count in the object fixture"),
        };
        objects.push(StorageObject {
            key: name,
            size_bytes: Some(size_bytes),
            storage_class: Some(field("class")),
            updated: Some(field("updated")),
            content_type: Some(field("content-type")),
            kms_encrypted: match field("encryption").as_str() {
                "CMEK" => true,
                "Google" => false,
                other => panic!("{other:?} is not an encryption label"),
            },
            has_retention: match field("retention").as_str() {
                "yes" => true,
                "" => false,
                other => panic!("{other:?} is not a retention label"),
            },
            generation: None,
            temporary_hold: false,
            event_based_hold: false,
        });
    }
    StorageObjectPage {
        prefixes,
        objects,
        next_page_token: None,
    }
}

#[test]
fn storage_object_browser_paints_every_value_under_its_own_hardcoded_header() {
    check_case(&storage_object_case());
}

// ---------------------------------------------------------------------------
// Engine
// ---------------------------------------------------------------------------

fn configuration() -> GcloudConfiguration {
    GcloudConfiguration {
        name: GcloudConfigurationName::new("dev"),
        is_active: true,
        account: Some("dev@example.com".to_string()),
        project: Some("dev-project".to_string()),
        region: Some("europe-west1".to_string()),
        zone: Some("europe-west1-b".to_string()),
    }
}

/// Composes the config file text from the case's blocks. Composition lives
/// here, not in the case, so that a file carrying several
/// `[columns.<service>]` blocks stays one function's business and every case
/// keeps supplying nothing but its own body.
fn config_text<'a>(blocks: impl IntoIterator<Item = &'a ColumnsBlock>) -> String {
    let mut text = String::from("version = 1\n");
    for block in blocks {
        text.push_str(&format!("\n[columns.{}]\n{}", block.section, block.body));
    }
    text
}

/// Character-wise `find`, so a box-drawing glyph elsewhere on the line can
/// never split a byte index down the middle.
fn find_chars(hay: &[char], needle: &str, from: usize) -> Option<usize> {
    let needle: Vec<char> = needle.chars().collect();
    if needle.is_empty() || from + needle.len() > hay.len() {
        return None;
    }
    (from..=hay.len() - needle.len()).find(|&start| hay[start..start + needle.len()] == needle[..])
}

/// Clamped character slice: painted lines are right-trimmed, so a span derived
/// from the header line can run past the end of a shorter row line.
fn slice_chars(chars: &[char], from: usize, to: usize) -> String {
    let from = from.min(chars.len());
    let to = to.min(chars.len()).max(from);
    chars[from..to].iter().collect()
}

/// The expected value of `key` in `row`.
fn value_of(row: ExpectedRow, key: &str, case: &str) -> &'static str {
    row.iter()
        .find(|(row_key, _)| *row_key == key)
        .map(|(_, value)| *value)
        .unwrap_or_else(|| panic!("case {case}: no fixture value for column {key:?}"))
}

/// The expected value of `key` in `row`, owned — for the fixture builders,
/// which construct their service's structs from the expected-row table so the
/// painted expectations and the loaded rows cannot drift apart.
fn owned(row: ExpectedRow, key: &str, case: &str) -> String {
    value_of(row, key, case).to_string()
}

/// The painted extent of one column, in character offsets into a frame line.
struct Span {
    start: usize,
    end: usize,
}

fn check_case(case: &Case) {
    // The fixture constraint is case-wide, not per row: a column's slice must
    // hold no other value *anywhere in the case*, including values belonging to
    // other rows, so equality or containment between any two values would make
    // that exclusion unfalsifiable.
    //
    // An empty expectation is not a value: it says the cell is painted blank,
    // which is what a Cloud Storage prefix row does in six of its seven
    // columns. It carries no identity — every string contains it — so it stays
    // out of this pool, and its cell is checked for blankness instead.
    let all_values: Vec<(usize, &str, &str)> = case
        .rows
        .iter()
        .enumerate()
        .flat_map(|(index, row)| row.iter().map(move |(key, value)| (index, *key, *value)))
        .filter(|(_, _, value)| !value.is_empty())
        .collect();
    for (row, key, value) in &all_values {
        for (other_row, other_key, other_value) in &all_values {
            if row == other_row && key == other_key {
                continue;
            }
            assert!(
                !value.contains(other_value),
                "case {}: fixture values must be pairwise non-substring case-wide, but row \
                 {row}'s {key} {value:?} contains row {other_row}'s {other_key} {other_value:?}",
                case.name
            );
        }
    }

    // The real config path: a real file, parsed and validated by `Config::resolve`.
    let mut file = tempfile::NamedTempFile::new().expect("temp config file");
    file.write_all(config_text(case.columns.iter()).as_bytes())
        .expect("write temp config");
    file.flush().expect("flush temp config");
    let cli = Cli::parse_from([
        "nuvem",
        "--plugins",
        "/nonexistent/nuvem-plugins-test.toml",
        "--config",
        file.path().to_str().expect("utf-8 path"),
    ]);
    let outcome = Config::resolve(&cli);
    // Without this a typo in the fixture would silently fall back to the
    // catalog defaults and the test would assert about the wrong columns.
    assert!(
        outcome.problems.is_empty(),
        "case {}: fixture config must validate cleanly: {:?}",
        case.name,
        outcome.problems.messages()
    );
    let config = outcome.config;

    // The real startup path, the same call `run()` makes, rather than poking
    // the column settings into the state directly.
    let mut app = AppState::new();
    apply_startup_config(&mut app, &config);
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ConfigurationsLoaded {
            configurations: vec![configuration()],
        }),
    );
    let context = app.root.current_context().expect("active context");
    (case.setup)(&mut app, &context);
    app.root.workspace = Workspace::Resources;

    // The header labels to look for, in display order, and each column's
    // alignment, taken from the model the renderer is about to draw. Only the
    // search strings and the alignment come from the model; every assertion
    // below is made on painted characters.
    let model = project_resources(&app.root.resources)
        .table
        .expect("list mode builds a table model");
    let headers: Vec<String> = model
        .columns
        .iter()
        .map(|column| column.header.to_string())
        .collect();
    let aligns: Vec<CellAlign> = model.columns.iter().map(|column| column.align).collect();
    let keys: Vec<String> = model
        .columns
        .iter()
        .map(|column| column.key.to_string())
        .collect();
    assert_eq!(
        keys, case.expected_keys,
        "case {}: the configured layout must reach the model before the paint is worth checking",
        case.name
    );

    let session = RuntimeSession::new_with_cli_and_config_for_test(cli, config, app);
    let backend = TestBackend::new(FRAME_WIDTH, FRAME_HEIGHT);
    let mut terminal = Terminal::new(backend).expect("test terminal");
    terminal
        .draw(|frame| nuvem::ui::render(frame, &session))
        .expect("render");
    let buffer = terminal.backend().buffer();
    let lines: Vec<Vec<char>> = (0..buffer.area.height)
        .map(|y| {
            let mut line = String::new();
            for x in 0..buffer.area.width {
                line.push_str(buffer.cell((x, y)).expect("cell").symbol());
            }
            line.trim_end().chars().collect()
        })
        .collect();
    let dump = lines
        .iter()
        .map(|line| line.iter().collect::<String>())
        .collect::<Vec<_>>()
        .join("\n");
    let name = case.name;

    // Not finding a line must abort loudly: a test that silently skips its
    // assertions is worse than no test.
    let (header_index, header_chars) = lines
        .iter()
        .enumerate()
        .find(|(_, line)| {
            let text: String = line.iter().collect();
            headers.iter().all(|header| text.contains(header.as_str()))
        })
        .map(|(index, line)| (index, line.clone()))
        .unwrap_or_else(|| {
            panic!("case {name}: no line carries every header {headers:?} in frame:\n{dump}")
        });
    let header_text: String = header_chars.iter().collect();

    // Header anchors, searched left to right with a running cursor so a
    // repeated substring cannot claim an earlier column's span. Two decorations
    // sit inside a column's own cell and so belong to its anchor: the column
    // the cursor sits on is painted bracketed (`[NAME]`), and the sort column
    // gets a `▲`/`▼` appended. The bracket moves the cell's left edge one
    // character left; the arrow and the closing bracket together move its right
    // edge up to two characters right.
    let mut anchors: Vec<(usize, usize)> = Vec::new();
    let mut cursor = 0usize;
    for header in &headers {
        let start = find_chars(&header_chars, header, cursor).unwrap_or_else(|| {
            panic!(
                "case {name}: header {header:?} not painted after column {cursor} in \
                 frame:\n{dump}"
            )
        });
        cursor = start + header.chars().count();
        let mut end = cursor;
        if matches!(header_chars.get(end), Some('\u{25b2}' | '\u{25bc}')) {
            end += 1;
        }
        if header_chars.get(end) == Some(&']') {
            end += 1;
        }
        let start = if start > 0 && header_chars[start - 1] == '[' {
            start - 1
        } else {
            start
        };
        anchors.push((start, end));
    }
    assert_eq!(
        anchors.len(),
        case.expected_keys.len(),
        "case {name}: every configured column must have a painted span in frame:\n{dump}"
    );

    // A left-aligned header is flush with its cell's left edge, so its anchor
    // start *is* the cell's start. A right-aligned header is flush with the
    // cell's right edge instead, and the padding to its left is invisible — the
    // painted line cannot reveal where such a cell begins. So its span starts
    // at its right edge less the widest thing the column has to hold (its
    // header or any of its expected values). That is never further right than
    // the true cell start, because the cell is at least that wide, and never
    // far enough left to swallow the neighbour's value, because the neighbour's
    // value begins at the neighbour's own cell start, which lies left of this
    // cell's start.
    let widest = |position: usize| -> usize {
        let key = &keys[position];
        case.rows
            .iter()
            .map(|row| value_of(row, key, name).chars().count())
            .chain(std::iter::once(headers[position].chars().count()))
            .max()
            .unwrap_or(0)
    };
    assert!(
        matches!(aligns[0], CellAlign::Left),
        "case {name}: the engine derives the table pane's left edge from the first column's cell \
         start, which the painted line only reveals for a left-aligned column"
    );
    let starts: Vec<usize> = (0..anchors.len())
        .map(|position| match aligns[position] {
            CellAlign::Left => anchors[position].0,
            CellAlign::Right => anchors[position].1.saturating_sub(widest(position)),
        })
        .collect();

    // The table pane's column range, derived once and applied to *every* line
    // before anything is looked for in it. The pane starts at the first
    // column's cell and ends at the next vertical rule after the last header.
    // The preview pane to the right paints its own `Zone: ...` and
    // `Status: ...` lines on these very rows, so an unbounded search could both
    // latch onto a preview line when looking for a row and be satisfied by the
    // preview when checking that a value was painted in full.
    let pane_start = starts[0];
    let pane_end = find_chars(&header_chars, "\u{2502}", anchors[anchors.len() - 1].1)
        .unwrap_or(header_chars.len());
    assert!(
        pane_start < pane_end,
        "case {name}: table pane range {pane_start}..{pane_end} is empty in frame:\n{dump}"
    );

    let spans: Vec<Span> = (0..starts.len())
        .map(|position| Span {
            start: starts[position],
            end: match aligns[position] {
                // A right-aligned cell ends exactly where its header ends.
                CellAlign::Right => anchors[position].1,
                // A left-aligned cell runs to wherever the next column begins
                // (the two-cell gap is padding either way), or to the pane rule
                // for the last column.
                CellAlign::Left => starts.get(position + 1).copied().unwrap_or(pane_end),
            },
        })
        .collect();

    // Every row is located by its own `name`, and no two rows may resolve to
    // the same painted line. Row *order* is deliberately not asserted: which
    // row comes first is the sort's business, not this test's.
    let mut resolved: Vec<(usize, usize)> = Vec::new();
    for (index, row) in case.rows.iter().enumerate() {
        let row_name = value_of(row, "name", name);
        let line = lines
            .iter()
            .enumerate()
            .skip(header_index + 1)
            .find(|(_, line)| slice_chars(line, pane_start, pane_end).contains(row_name))
            .map(|(line_index, _)| line_index)
            .unwrap_or_else(|| {
                panic!(
                    "case {name}: no row carrying {row_name:?} below the header inside the table \
                     pane in frame:\n{dump}"
                )
            });
        if let Some((other, _)) = resolved.iter().find(|(_, other_line)| *other_line == line) {
            panic!(
                "case {name}: rows {other} and {index} both resolved to painted line {line} — the \
                 same line cannot carry two rows in frame:\n{dump}"
            );
        }
        resolved.push((index, line));
    }

    for (index, line_index) in &resolved {
        let row = case.rows[*index];
        let row_chars = &lines[*line_index];
        let row_text: String = row_chars.iter().collect();
        let painted_pane = slice_chars(row_chars, pane_start, pane_end);

        // No value was painted as a truncated prefix. Bounded to the table
        // pane, so `fit_cell`'s `asia-sou…` cannot be excused by the preview
        // pane spelling the value out beside it.
        for (key, value) in row.iter().filter(|(_, value)| !value.is_empty()) {
            assert!(
                painted_pane.contains(value),
                "case {name}: row {index} column {key}'s value {value:?} must be painted in full \
                 inside the table pane, pane painted: {painted_pane:?}\nheader: {header_text}\n\
                 row:    {row_text}\nframe:\n{dump}"
            );
        }

        for (position, key) in keys.iter().enumerate() {
            let span = &spans[position];
            let cell = slice_chars(
                row_chars,
                span.start.max(pane_start),
                span.end.min(pane_end),
            );
            let expected = value_of(row, key, name);
            let header = &headers[position];

            // Exact offset, not mere containment: containment alone lets a
            // value shifted a few cells inside a padded column pass. A
            // left-aligned cell's text starts exactly at the cell's start; a
            // right-aligned cell's text ends exactly at the cell's end, with
            // only the inter-column gap painted after it.
            // A blank expectation is an assertion in its own right: the column
            // is painted empty for this row. It is what a folder row in the
            // object browser looks like in every column but NAME, and asserting
            // it is what makes "the folder row borrowed the object row's size"
            // a failure.
            if expected.is_empty() {
                assert!(
                    cell.trim().is_empty(),
                    "case {name}: row {index} column {position} (header {header:?}, key {key:?}) \
                     must be painted blank, painted {cell:?}\nheader: {header_text}\nrow:    \
                     {row_text}\nframe:\n{dump}"
                );
                continue;
            }

            match aligns[position] {
                CellAlign::Left => assert!(
                    cell.starts_with(expected),
                    "case {name}: row {index} column {position} (header {header:?}, key {key:?}) \
                     must paint {expected:?} flush with the column's left edge, painted \
                     {cell:?}\nheader: {header_text}\nrow:    {row_text}\nframe:\n{dump}"
                ),
                // No `trim_end()` here. A right-aligned header is flush with
                // its cell's right edge, so the anchor's end *is* that edge and
                // the value must end there with nothing after it. Tolerating
                // trailing spaces would tolerate a cell whose text sits one
                // character left inside its own span — padding computed wrong
                // on the padded side, which is the defect shape right alignment
                // is exposed to and the only one it has that left alignment
                // does not.
                CellAlign::Right => assert!(
                    cell.ends_with(expected),
                    "case {name}: row {index} column {position} (header {header:?}, key {key:?}) \
                     must paint {expected:?} flush with the column's right edge, painted \
                     {cell:?}\nheader: {header_text}\nrow:    {row_text}\nframe:\n{dump}"
                ),
            }

            // Cross-row exclusion: the slice may hold no other value the case
            // knows about — not another column of this row, and not any value
            // of any other row. This is what makes a row-mixing defect (row 2
            // painting row 1's cell) red rather than green.
            for (other_index, other_key, other_value) in &all_values {
                if other_index == index && other_key == key {
                    continue;
                }
                assert!(
                    !cell.contains(other_value),
                    "case {name}: row {index} column {position} (header {header:?}, key {key:?}) \
                     painted row {other_index}'s {other_key} value {other_value:?}: {cell:?}\n\
                     header: {header_text}\nrow:    {row_text}\nframe:\n{dump}"
                );
            }
        }
    }
}
