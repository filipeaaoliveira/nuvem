//! Configured column layouts as they reach the rendered table (task 3).
//!
//! The crux is alignment: under a reordered *and* subsetted layout, header *i*
//! must still be paired with the cell holding that column's field. The fixture
//! rows below give every field a value distinct from every other field of the
//! same row, and each pairing is checked by *name* — the value expected under a
//! header is looked up by the header's key, never by position — so a layout
//! that resolved for headers but not for cells cannot pass by coincidence.
//!
//! The rest pins the index translation the layout forces: `SortField::column()`
//! keeps meaning a catalog index, while `selected_column` and
//! `TableModel.sort.column` mean a display position.

use std::io::Write;

use clap::Parser;
use nuvem::app::projection::columns::catalog_meta;
use nuvem::app::projection::project_resources;
use nuvem::app::projection::table::{CellKind, SortDirection, StatusSemantic, TableModel};
use nuvem::app::reducer::root::reduce;
use nuvem::app::{Action, AppState, EffectResult, ResourcesAction};
use nuvem::config::columns::ColumnSettings;
use nuvem::ui::shared::table::plan_table;
use nuvem::{Cli, Config};
use nuvem_domain::gcp::configuration::{GcloudConfiguration, GcloudConfigurationName};
use nuvem_domain::gcp::resource::{
    CloudSqlAvailability, CloudSqlInstance, CloudSqlStatus, ComputeInstance, EndpointVisibility,
    ResourceServiceKind, ResourceStatus, StorageBucket,
};

const COMPUTE: ResourceServiceKind = ResourceServiceKind::ComputeInstances;
const CLOUD_SQL: ResourceServiceKind = ResourceServiceKind::CloudSql;
const STORAGE: ResourceServiceKind = ResourceServiceKind::CloudStorage;

/// The validated settings a config file body resolves to, through the real
/// config path — the same values task 4 will hand the state at startup.
fn settings(body: &str) -> ColumnSettings {
    let mut file = tempfile::NamedTempFile::new().expect("temp config file");
    file.write_all(body.as_bytes()).expect("write config");
    let cli = Cli::parse_from([
        "nuvem",
        "--plugins",
        "/nonexistent/nuvem-plugins-test.toml",
        "--config",
        file.path().to_str().expect("utf-8 path"),
    ]);
    let outcome = Config::resolve(&cli);
    assert!(
        outcome.problems.is_empty(),
        "fixture config must validate cleanly: {:?}",
        outcome.problems.messages()
    );
    outcome.config.columns
}

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

/// An app parked on `service` with `settings` installed *before* its rows
/// load — the order the real startup path uses — and one loaded row whose
/// every field differs from every other field of the same row.
fn app_with_rows(service: ResourceServiceKind, settings: ColumnSettings) -> AppState {
    let mut app = AppState::new();
    app.root.resources.set_column_settings(settings);
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
    let context = app.root.current_context().expect("active context").clone();
    let loaded = match service {
        COMPUTE => EffectResult::ComputeInstancesLoaded {
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
        CLOUD_SQL => EffectResult::CloudSqlInstancesLoaded {
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
        STORAGE => EffectResult::StorageBucketsLoaded {
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
        other => panic!("{other:?} has no fixture in this test"),
    };
    reduce(&mut app, &Action::EffectResult(loaded));
    app
}

/// A compute row for the ordering fixture, distinguishable by zone.
fn instance(name: &str, zone: &str) -> ComputeInstance {
    ComputeInstance {
        id: name.to_string(),
        name: name.to_string(),
        zone: zone.to_string(),
        status: ResourceStatus::Running,
        machine_type: "e2-medium".to_string(),
        internal_ip: None,
        external_ip: None,
    }
}

fn table(app: &AppState) -> TableModel {
    project_resources(&app.root.resources)
        .table
        .expect("list mode builds a table model")
}

/// The catalog index of a column key, so tests name columns the way a user
/// does instead of hardcoding catalog positions.
fn index(kind: ResourceServiceKind, key: &str) -> usize {
    catalog_meta(kind)
        .iter()
        .position(|column| column.key == key)
        .expect("test names a real catalog column")
}

/// Every field the fixture compute row carries, by the column key that owns
/// it. The values are mutually distinguishable, so pairing a header with the
/// wrong cell always shows up.
fn compute_fields() -> Vec<(&'static str, &'static str, CellKind)> {
    vec![
        ("name", "api", CellKind::Plain),
        (
            "status",
            "Running",
            CellKind::Status(StatusSemantic::Healthy),
        ),
        ("zone", "europe-west1-b", CellKind::Plain),
        ("machine-type", "e2-medium", CellKind::Plain),
        ("internal-ip", "10.0.0.4", CellKind::Plain),
        ("external-ip", "-", CellKind::Missing),
    ]
}

fn cloud_sql_fields() -> Vec<(&'static str, &'static str, CellKind)> {
    vec![
        ("name", "orders", CellKind::Plain),
        ("engine", "PostgreSQL (POSTGRES_15)", CellKind::Plain),
        ("region", "europe-west1", CellKind::Plain),
        ("tier", "db-custom-2-7680", CellKind::Plain),
        ("availability", "HA (regional)", CellKind::Plain),
        ("connectivity", "Private", CellKind::Plain),
        (
            "status",
            "Runnable",
            CellKind::Status(StatusSemantic::Healthy),
        ),
    ]
}

fn storage_bucket_fields() -> Vec<(&'static str, &'static str, CellKind)> {
    vec![
        ("name", "alpha-bucket", CellKind::Plain),
        ("location", "EU", CellKind::Plain),
        ("class", "STANDARD", CellKind::Plain),
        ("updated", "2026-01-01T00:00:00Z", CellKind::Plain),
        ("versioning", "off", CellKind::Plain),
        ("public-access", "enforced", CellKind::Plain),
    ]
}

/// Asserts the table shows exactly `expected_order`, and that at every index
/// the cell under a header holds the field that header names — looked up by
/// key in `fields`, not by position, so this cannot be satisfied by a
/// consistently-shifted pairing.
fn assert_each_header_owns_its_cell(
    table: &TableModel,
    expected_order: &[&str],
    fields: &[(&'static str, &'static str, CellKind)],
) {
    let keys: Vec<&str> = table.columns.iter().map(|column| column.key).collect();
    assert_eq!(keys, expected_order, "configured display order");

    let row = table.rows.first().expect("fixture loads one row");
    assert_eq!(
        table.columns.len(),
        row.cells.len(),
        "a row has a different number of cells than the table has headers"
    );
    for (position, (column, cell)) in table.columns.iter().zip(&row.cells).enumerate() {
        let (_, text, kind) = fields
            .iter()
            .find(|(key, _, _)| *key == column.key)
            .unwrap_or_else(|| panic!("column {:?} is not a field of the fixture row", column.key));
        assert_eq!(
            (cell.text.as_str(), &cell.kind),
            (*text, kind),
            "display position {position}: header {:?} is paired with another column's cell",
            column.key
        );
    }
}

#[test]
fn a_reordered_subset_pairs_each_compute_header_with_its_own_cell() {
    let app = app_with_rows(
        COMPUTE,
        settings("[columns.compute]\ncolumns = [\"zone\", \"name\", \"status\"]\n"),
    );

    assert_each_header_owns_its_cell(&table(&app), &["zone", "name", "status"], &compute_fields());
}

#[test]
fn a_reordered_subset_pairs_each_cloud_sql_header_with_its_own_cell() {
    let app = app_with_rows(
        CLOUD_SQL,
        settings("[columns.cloudsql]\ncolumns = [\"tier\", \"name\", \"status\"]\n"),
    );

    assert_each_header_owns_its_cell(
        &table(&app),
        &["tier", "name", "status"],
        &cloud_sql_fields(),
    );
}

#[test]
fn a_reordered_subset_pairs_each_storage_bucket_header_with_its_own_cell() {
    // The bucket list is the one table whose headers and cells are still built
    // at two call sites, so it gets the same pairing check as the two the brief
    // names rather than a headers-only one.
    let app = app_with_rows(
        STORAGE,
        settings("[columns.storage]\ncolumns = [\"class\", \"name\", \"location\"]\n"),
    );

    assert_each_header_owns_its_cell(
        &table(&app),
        &["class", "name", "location"],
        &storage_bucket_fields(),
    );
}

#[test]
fn an_unconfigured_service_keeps_the_whole_catalog_in_catalog_order() {
    let app = app_with_rows(COMPUTE, ColumnSettings::empty());
    let table = table(&app);

    let expected: Vec<&str> = catalog_meta(COMPUTE)
        .iter()
        .map(|column| column.key)
        .collect();
    assert_each_header_owns_its_cell(&table, &expected, &compute_fields());
}

#[test]
fn configured_columns_take_their_priority_from_their_display_position() {
    // Cloud Storage's catalog priorities are deliberately non-positional
    // (`location` sits in display position 1 with priority 2), so a positional
    // override is visible here in a way it would not be for another service.
    // `name` is the exception: it holds priority 0 from display position 1,
    // and the columns around it are shifted by one to leave it room.
    let app = app_with_rows(
        STORAGE,
        settings("[columns.storage]\ncolumns = [\"class\", \"name\", \"location\"]\n"),
    );

    let priorities: Vec<(&str, u8)> = table(&app)
        .columns
        .iter()
        .map(|column| (column.key, column.priority))
        .collect();
    assert_eq!(priorities, vec![("class", 1), ("name", 0), ("location", 3)]);
}

/// The keys of the columns a plan leaves visible, in display order.
fn visible_keys(table: &TableModel, width: u16) -> Vec<&str> {
    plan_table(table, width, false)
        .columns
        .iter()
        .filter(|column| column.visible)
        .map(|column| table.columns[column.index].key)
        .collect()
}

/// The distinct column sets the plan passes through from wide to narrow.
fn drop_sequence(table: &TableModel) -> Vec<Vec<&str>> {
    let mut sequence: Vec<Vec<&str>> = Vec::new();
    for width in (1..=200u16).rev() {
        let keys = visible_keys(table, width);
        if sequence.last() != Some(&keys) {
            sequence.push(keys);
        }
    }
    sequence
}

#[test]
fn a_configured_list_ending_in_name_still_keeps_name_in_narrow_mode() {
    // The whole compute catalog, reordered so `name` is last. The first drop
    // must be the rightmost *droppable* column, not `name`. Under plain
    // positional priorities `name` would carry the largest priority value and
    // be the first column dropped, leaving an operator with rows they cannot
    // tell apart while `Enter`, `t` and `y` still act on the selected one.
    let app = app_with_rows(
        COMPUTE,
        settings(
            "[columns.compute]\n\
             columns = [\"status\", \"zone\", \"machine-type\", \"internal-ip\", \
             \"external-ip\", \"name\"]\n",
        ),
    );
    let table = table(&app);

    assert_eq!(
        drop_sequence(&table)[1],
        vec!["status", "zone", "machine-type", "internal-ip", "name"],
        "the rightmost droppable column goes first and `name` stays"
    );
}

#[test]
fn name_survives_a_configured_list_that_ends_in_it_at_every_width() {
    // The drop order walks right-to-left over everything but `name`, and
    // `name` is still there when it is the only column left.
    let app = app_with_rows(
        COMPUTE,
        settings(
            "[columns.compute]\n\
             columns = [\"status\", \"zone\", \"machine-type\", \"internal-ip\", \
             \"external-ip\", \"name\"]\n",
        ),
    );
    let table = table(&app);

    let expected: Vec<Vec<&str>> = vec![
        vec![
            "status",
            "zone",
            "machine-type",
            "internal-ip",
            "external-ip",
            "name",
        ],
        vec!["status", "zone", "machine-type", "internal-ip", "name"],
        vec!["status", "zone", "machine-type", "name"],
        vec!["status", "zone", "name"],
        vec!["status", "name"],
        vec!["name"],
    ];
    assert_eq!(drop_sequence(&table), expected);
    for width in 1..=200u16 {
        assert!(
            visible_keys(&table, width).contains(&"name"),
            "`name` was dropped at width {width}"
        );
    }
}

#[test]
fn an_unconfigured_service_keeps_the_catalogs_own_priorities() {
    // The mirror of the case above: storage's hand-tuned, non-positional
    // priorities must survive untouched when nothing is configured.
    let app = app_with_rows(STORAGE, ColumnSettings::empty());

    let priorities: Vec<(&str, u8)> = table(&app)
        .columns
        .iter()
        .map(|column| (column.key, column.priority))
        .collect();
    assert_eq!(
        priorities,
        vec![
            ("name", 0),
            ("location", 2),
            ("class", 1),
            ("updated", 3),
            ("versioning", 4),
            ("public-access", 5),
        ]
    );
}

#[test]
fn a_sort_only_block_keeps_the_catalogs_own_priorities() {
    // Priorities are positional only for a configured *list*. A block that
    // sets nothing but `sort` leaves the columns exactly where the catalog put
    // them, so it cannot silently change which column narrow mode drops first.
    let app = app_with_rows(
        STORAGE,
        settings("[columns.storage]\nsort = \"location\"\n"),
    );

    let priorities: Vec<(&str, u8)> = table(&app)
        .columns
        .iter()
        .map(|column| (column.key, column.priority))
        .collect();
    assert_eq!(
        priorities,
        vec![
            ("name", 0),
            ("location", 2),
            ("class", 1),
            ("updated", 3),
            ("versioning", 4),
            ("public-access", 5),
        ]
    );
}

#[test]
fn the_column_cursor_starts_on_a_sortable_column() {
    // A valid list may open on a non-sortable column, and a cursor parked
    // there would highlight a header `s` does nothing for. The cursor is
    // anchored to the first sortable display position instead.
    let app = app_with_rows(
        COMPUTE,
        settings("[columns.compute]\ncolumns = [\"internal-ip\", \"name\", \"status\"]\n"),
    );
    let table = table(&app);
    assert_eq!(table.selected_column, 1, "internal-ip -> name");
    assert_eq!(table.columns[table.selected_column].key, "name");

    // And `s` acts on it rather than doing nothing.
    let mut state = app.root.resources;
    state.sort_selected_column();
    assert_eq!(state.sort_label(), "Name");
    assert_eq!(state.sort_direction(), SortDirection::Descending);
}

#[test]
fn a_configured_default_sort_is_in_force_on_the_first_render() {
    let app = app_with_rows(
        COMPUTE,
        settings(
            "[columns.compute]\ncolumns = [\"zone\", \"name\", \"status\"]\nsort = \"status\"\nsort_direction = \"desc\"\n",
        ),
    );
    let table = table(&app);

    assert_eq!(app.root.resources.sort_label(), "Status");
    assert_eq!(table.sort.direction, SortDirection::Descending);
    // The arrow is placed by display position, not by catalog index.
    assert_eq!(table.sort.column, 2);
    assert_eq!(table.columns[table.sort.column].key, "status");
}

#[test]
fn a_configured_default_sort_orders_the_rows_on_the_first_render() {
    // The arrow and the label are state; this is the ordering the operator
    // actually reads. A sort-only block, so the default columns stay.
    let mut app = app_with_rows(
        COMPUTE,
        settings("[columns.compute]\nsort = \"zone\"\nsort_direction = \"desc\"\n"),
    );
    let context = app.root.current_context().expect("active context").clone();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ComputeInstancesLoaded {
            context,
            instances: vec![
                instance("api", "europe-west1-a"),
                instance("db", "europe-west1-c"),
                instance("web", "europe-west1-b"),
            ],
        }),
    );

    let table = table(&app);
    let zone = table
        .columns
        .iter()
        .position(|column| column.key == "zone")
        .expect("zone is displayed");
    let zones: Vec<&str> = table
        .rows
        .iter()
        .map(|row| row.cells[zone].text.as_str())
        .collect();
    assert_eq!(
        zones,
        ["europe-west1-c", "europe-west1-b", "europe-west1-a"]
    );
}

#[test]
fn the_sort_column_of_a_configured_service_is_always_displayed() {
    // Validation rejects a `sort` outside the configured list, so the install
    // path can never leave the collection sorted by a column the layout does
    // not show. Checked here for every service that carries a configured sort.
    let body = r#"
[columns.compute]
columns = ["zone", "name", "status"]
sort = "zone"

[columns.gke]
sort = "location"

[columns.cloudsql]
columns = ["tier", "name", "status"]
sort = "tier"

[columns.run]
columns = ["region", "name"]
sort = "region"

[columns.storage]
columns = ["class", "name", "location"]
sort = "location"
"#;
    let mut state = AppState::new().root.resources;
    state.set_column_settings(settings(body));

    for (kind, sort_key) in [
        (COMPUTE, "zone"),
        (ResourceServiceKind::GkeClusters, "location"),
        (CLOUD_SQL, "tier"),
        (ResourceServiceKind::CloudRunServices, "region"),
        (STORAGE, "location"),
    ] {
        state.select_service(kind);
        let layout = state.column_layout(kind);
        let position = state.sort_column_index();
        assert_eq!(
            layout.get(position).copied(),
            Some(index(kind, sort_key)),
            "{kind:?}: the sort arrow sits on the column actually being sorted"
        );
    }
}

/// Every service's configured sort is actually installed — walked off
/// [`ResourceServiceKind::ALL`] rather than a hand-written list.
///
/// `set_column_settings` enumerates its services by hand, so the compiler
/// cannot notice a new one being left out; a service missing from it parses,
/// validates and stores the user's `sort` and then ignores it in silence,
/// which is exactly the failure the column validation exists to prevent. This
/// test is what notices instead.
#[test]
fn every_service_installs_the_sort_its_config_block_asks_for() {
    /// The config-file key for a service. An exhaustive match, so a new
    /// service has to be named here before this test compiles.
    fn config_key(kind: ResourceServiceKind) -> &'static str {
        match kind {
            ResourceServiceKind::ComputeInstances => "compute",
            ResourceServiceKind::GkeClusters => "gke",
            ResourceServiceKind::CloudSql => "cloudsql",
            ResourceServiceKind::CloudRunServices => "cloudrun",
            ResourceServiceKind::CloudStorage => "storage",
            ResourceServiceKind::SecretManagerSecrets => "secrets",
            ResourceServiceKind::CloudFunctions => "functions",
            ResourceServiceKind::PersistentDisks => "disks",
            ResourceServiceKind::PubSubTopics => "pubsub-topics",
            ResourceServiceKind::PubSubSubscriptions => "pubsub-subscriptions",
            ResourceServiceKind::BigQueryDatasets => "bigquery-datasets",
            ResourceServiceKind::IamServiceAccounts => "service-accounts",
            ResourceServiceKind::IamPolicy => "iam-policy",
            ResourceServiceKind::VpcNetworks => "vpc-networks",
            ResourceServiceKind::VpcSubnets => "vpc-subnets",
            ResourceServiceKind::FirewallRules => "firewall-rules",
        }
    }

    // A sortable column other than `name`, so the configured sort differs from
    // every service's built-in default and a sort that was never installed
    // cannot pass by coincidence.
    let sorts: Vec<(ResourceServiceKind, &'static str)> = ResourceServiceKind::ALL
        .into_iter()
        .map(|kind| {
            let key = catalog_meta(kind)
                .iter()
                .find(|column| column.sortable && column.key != "name")
                .map(|column| column.key)
                .expect("every service has a sortable column besides name");
            (kind, key)
        })
        .collect();

    let body: String = sorts
        .iter()
        .map(|(kind, key)| format!("[columns.{}]\nsort = \"{key}\"\n", config_key(*kind)))
        .collect();
    let mut state = AppState::new().root.resources;
    state.set_column_settings(settings(&body));

    for (kind, key) in sorts {
        state.select_service(kind);
        let layout = state.column_layout(kind);
        let position = state.sort_column_index();
        assert_eq!(
            layout.get(position).copied(),
            Some(index(kind, key)),
            "{kind:?}: the configured sort column `{key}` must be installed, \
             not parsed and then ignored"
        );
    }
}

#[test]
fn the_column_cursor_visits_the_displayed_sortable_columns_in_display_order() {
    // `internal-ip` is displayed but not sortable, so the cursor must step
    // over it; the ends of the *layout* bound the walk, not the catalog's.
    let app = app_with_rows(
        COMPUTE,
        settings(
            "[columns.compute]\ncolumns = [\"zone\", \"name\", \"internal-ip\", \"status\"]\n",
        ),
    );
    let mut state = app.root.resources;

    assert_eq!(state.selected_column(), 0);
    state.select_next_column();
    assert_eq!(state.selected_column(), 1, "zone -> name");
    state.select_next_column();
    assert_eq!(
        state.selected_column(),
        3,
        "name -> status, over internal-ip"
    );
    state.select_next_column();
    assert_eq!(state.selected_column(), 3, "stops at the right-hand end");

    state.select_previous_column();
    assert_eq!(
        state.selected_column(),
        1,
        "status -> name, over internal-ip"
    );
    state.select_previous_column();
    assert_eq!(state.selected_column(), 0, "name -> zone");
    state.select_previous_column();
    assert_eq!(state.selected_column(), 0, "stops at the left-hand end");
}

#[test]
fn sorting_by_a_displayed_column_sorts_by_that_column_and_toggles_on_repeat() {
    let app = app_with_rows(
        COMPUTE,
        settings(
            "[columns.compute]\ncolumns = [\"zone\", \"name\", \"internal-ip\", \"status\"]\n",
        ),
    );
    let mut state = app.root.resources;

    // The default sort field is Name, which now sits at display position 1.
    assert_eq!(state.sort_column_index(), 1);

    // Cursor at display position 3 (`status`, catalog index 1).
    state.select_next_column();
    state.select_next_column();
    assert_eq!(state.selected_column(), 3);

    state.sort_selected_column();
    assert_eq!(state.sort_label(), "Status");
    assert_eq!(state.sort_column_index(), 3);
    assert_eq!(state.sort_direction(), SortDirection::Ascending);

    state.sort_selected_column();
    assert_eq!(state.sort_direction(), SortDirection::Descending);
    assert_eq!(state.sort_column_index(), 3);
}

#[test]
fn the_storage_object_browser_ignores_the_configured_bucket_layout() {
    // Only the bucket list is configurable: the object browser is a detail
    // mode with no column navigation, and keeps its fixed columns and cursor.
    let app = app_with_rows(
        STORAGE,
        settings("[columns.storage]\ncolumns = [\"class\", \"name\", \"location\"]\n"),
    );
    let mut state = app.root.resources;
    state
        .storage_mut()
        .open_object_browser()
        .expect("the fixture bucket is selected");

    let untouched = state.clone();
    state.select_next_column();
    state.select_previous_column();
    state.sort_selected_column();

    assert_eq!(state.selected_column(), 0);
    assert_eq!(state.sort_column_index(), 0);
    assert_eq!(state.sort_direction(), SortDirection::Ascending);
    assert_eq!(
        state, untouched,
        "the browser must ignore every column and sort key"
    );
}
