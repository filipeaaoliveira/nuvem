//! The BigQuery datasets table reaches the screen.
//!
//! A service can be in `SERVICE_REGISTRY`, satisfy every `ResourceServiceKind`
//! walk, hold rows in state, and still never paint one — every one of those
//! tests passes and the tab is blank. So these assertions are made on the
//! terminal buffer, through the real reducer and the real renderer, not on the
//! projection model.
//!
//! The datasets table comes first: that it paints, that it heads the three
//! agreed columns, and — the assertions this file exists for — that a dataset
//! whose listing reported no location paints a dash rather than a blank, that
//! a dataset with no labels does the same, and that the six empty branches of
//! `render_bigquery_datasets_list` — loading, failed, stale, ready-and-empty,
//! never-loaded and filtered-out — each say a different thing.
//!
//! Both drill-downs are asserted the same way, in the two sections below it:
//! the tables list `Enter` on a dataset opens, and the column schema `Enter`
//! on a table opens one level further down. Entering must replace what paints,
//! not merely change what state reports. The view is selected by
//! `BigQueryView` comparisons in `state/resources/bigquery.rs` and in the
//! projection, and cargo-mutants flips `==` to `!=` on all of them — so every
//! assertion there names *which* rows are on screen rather than that some list
//! is.
//!
//! The schema section carries the same shape one level down: the four agreed
//! columns, the cells that hold the dotted path, the mode and the missing
//! description, the row count and the preview beside it, the footer hinting
//! only the keys that do something there, and the four empty branches of
//! `render_bigquery_schema_list` — loading, failed, ready-and-empty and
//! filtered-out. There are four rather than six because a stale schema and a
//! never-loaded one cannot arise at that level.

use nuvem::app::projection::columns::{
    BIGQUERY_DATASET_COLUMNS, BIGQUERY_SCHEMA_COLUMNS, BIGQUERY_TABLE_COLUMNS, catalog_meta,
};
use nuvem::app::projection::{project_resource_preview, project_resources};
use nuvem::app::reducer::root::reduce;
use nuvem::app::state::resources::{BigQueryDrillGeneration, BigQueryListingEpoch};
use nuvem::app::state::root::Workspace;
use nuvem::app::{Action, AppState, Effect, EffectResult, ResourcesAction};
use nuvem::services::runtime::session::RuntimeSession;
use nuvem::ui::shared::table::plan_table;
use nuvem_domain::gcp::configuration::{GcloudConfiguration, GcloudConfigurationName};
use nuvem_domain::gcp::resource::{
    BigQueryDataset, BigQuerySchemaField, BigQueryTable, BigQueryTableKind, ComputeKeyValue,
    ResourceServiceKind, TableStat,
};
use ratatui::Terminal;
use ratatui::backend::TestBackend;

const FRAME_WIDTH: u16 = 200;
const FRAME_HEIGHT: u16 = 40;

/// Wide enough that the table area fits the whole catalogue unelided: the
/// labels cell is the widest column and the one the layout is meant to drop
/// first, which is correct behaviour and useless to assert a key path against.
const WIDE_FRAME_WIDTH: u16 = 320;

fn configuration() -> GcloudConfiguration {
    GcloudConfiguration {
        name: GcloudConfigurationName::new("dev"),
        is_active: true,
        account: Some("dev@example.com".to_string()),
        project: Some("dev-project".to_string()),
        region: None,
        zone: None,
    }
}

/// Four datasets, constructed for the cells this table has to paint rather
/// than copied from the capture: `tests/fixtures/bigquery_datasets.json` holds
/// four datasets that are all located and all labelled, so neither empty
/// `LOCATION` nor empty `LABELS` — the two table columns that can be dashed —
/// is a shape the capture contains. One row here is fully populated, one has
/// no friendly name and no labels, one has no location at all, and one is
/// missing both.
///
/// The friendly name is the exception, and it runs the other way: every
/// captured dataset parses to `friendly_name: None`, because
/// `parse_datasets_json` folds the one `"friendlyName": ""` to `None`
/// alongside the three that omit the key. So the dashed preview line *is* what
/// a captured dataset paints. `unlocated` carries the pre-fold `Some("")`
/// anyway — a value the parser cannot emit — because the preview reads the
/// field rather than the parser, and a renderer that dashed `None` while
/// painting a blank line for `Some("")` would pass on the folded shape alone.
///
/// The locations deliberately differ. A fixture whose rows share one location
/// cannot tell a `LOCATION` cell that paints from one that paints a constant,
/// and it cannot tell a location sort from the dataset-id tie-break either.
///
/// `zz_undocumented` is dashed in *both* cells, and it is what makes the
/// cell-scoped assertions below load-bearing: with only the first three rows,
/// asking "does this row carry a dash anywhere" and "does this row's LOCATION
/// cell carry the dash" are the same question, because no row has two dashes
/// to confuse them. It is named to sort last so the rows the preview test
/// walks to by index keep their positions.
fn datasets() -> Vec<BigQueryDataset> {
    vec![
        BigQueryDataset {
            dataset_id: "analytics_events".to_string(),
            project_id: "dev-project".to_string(),
            location: "europe-southwest1".to_string(),
            friendly_name: Some("Analytics events".to_string()),
            labels: vec![
                ComputeKeyValue {
                    key: "dg_data_confidentiality".to_string(),
                    value: "public".to_string(),
                },
                ComputeKeyValue {
                    key: "team".to_string(),
                    value: "platform".to_string(),
                },
            ],
        },
        BigQueryDataset {
            dataset_id: "bare_dataset".to_string(),
            project_id: "dev-project".to_string(),
            location: "US".to_string(),
            friendly_name: None,
            labels: Vec::new(),
        },
        BigQueryDataset {
            dataset_id: "unlocated".to_string(),
            project_id: "dev-project".to_string(),
            location: String::new(),
            friendly_name: Some(String::new()),
            labels: vec![ComputeKeyValue {
                key: "env".to_string(),
                value: "sandbox".to_string(),
            }],
        },
        BigQueryDataset {
            dataset_id: "zz_undocumented".to_string(),
            project_id: "dev-project".to_string(),
            location: String::new(),
            friendly_name: None,
            labels: Vec::new(),
        },
    ]
}

fn app_with_context() -> AppState {
    let mut app = AppState::new();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ConfigurationsLoaded {
            configurations: vec![configuration()],
        }),
    );
    reduce(
        &mut app,
        &Action::Resources(ResourcesAction::SelectService(
            ResourceServiceKind::BigQueryDatasets,
        )),
    );
    app.root.workspace = Workspace::Resources;
    app
}

fn app_showing_datasets() -> AppState {
    let mut app = app_with_context();
    let context = app.root.current_context().expect("active context");
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::BigQueryDatasetsLoaded {
            context,
            datasets: datasets(),
        }),
    );
    app
}

/// A state showing the four fixture datasets with `filter` applied through the
/// real filter actions. Any filter matching no dataset leaves the list with
/// rows loaded and none visible.
fn app_with_filter(filter: &str) -> AppState {
    let mut app = app_showing_datasets();
    reduce(&mut app, &Action::Resources(ResourcesAction::StartFilter));
    for character in filter.chars() {
        reduce(
            &mut app,
            &Action::Resources(ResourcesAction::AppendFilterChar(character)),
        );
    }
    reduce(&mut app, &Action::Resources(ResourcesAction::ApplyFilter));
    app
}

fn painted(app: AppState) -> String {
    painted_at(app, FRAME_WIDTH)
}

fn painted_at(app: AppState, width: u16) -> String {
    let session = RuntimeSession::new_for_test(app);
    let backend = TestBackend::new(width, FRAME_HEIGHT);
    let mut terminal = Terminal::new(backend).expect("test terminal");
    terminal
        .draw(|frame| nuvem::ui::render(frame, &session))
        .expect("render");
    let buffer = terminal.backend().buffer();
    (0..buffer.area.height)
        .map(|y| {
            let mut line = String::new();
            for x in 0..buffer.area.width {
                line.push_str(buffer.cell((x, y)).expect("cell").symbol());
            }
            line.trim_end().to_string()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// The rows of the table pane alone.
///
/// The navigator, the table and the preview are painted side by side, so one
/// buffer line carries a slice of all three — and the preview shows the
/// selected dataset's own id, location and friendly name. A `contains` over the
/// whole line therefore passes on a table that paints nothing at all, which is
/// the failure these tests exist to catch.
fn table_lines(painted: &str) -> Vec<&str> {
    painted
        .lines()
        .filter_map(|line| line.split("││").nth(1))
        .collect()
}

fn table_text(painted: &str) -> String {
    table_lines(painted).join("\n")
}

/// The text of one row of the table pane, found by the dataset id it carries.
fn table_row(painted: &str, dataset: &str) -> String {
    table_lines(painted)
        .into_iter()
        .find(|line| line.contains(dataset))
        .map(str::to_string)
        .unwrap_or_else(|| panic!("the table paints a {dataset} row:\n{painted}"))
}

/// One painted row split back into its cells, in column order.
///
/// The table pads every cell to its column width, so two or more spaces
/// separate cells while a single space is content — a label list is
/// `k=v, k=v`, and a selection marker is `> ` before the identity. Splitting on
/// the padding is what lets an assertion name the column a dash belongs to:
/// `row.contains("-")` cannot, and neither can `any(|cell| cell == "-")` once
/// a row has two dashed cells, which `zz_undocumented` gives this fixture.
fn painted_cells(row: &str) -> Vec<&str> {
    row.trim()
        .trim_start_matches("> ")
        .split("  ")
        .map(str::trim)
        .filter(|cell| !cell.is_empty())
        .collect()
}

/// The painted cell of `dataset`'s row under `column`, named by its catalogue
/// key so the assertion moves with the column order rather than pinning an
/// index by hand.
fn painted_cell(painted: &str, dataset: &str, column: &str) -> String {
    let index = BIGQUERY_DATASET_COLUMNS
        .iter()
        .position(|definition| definition.spec.key == column)
        .unwrap_or_else(|| panic!("the datasets catalogue carries a {column} column"));
    let row = table_row(painted, dataset);
    painted_cells(&row)
        .get(index)
        .unwrap_or_else(|| panic!("the {dataset} row paints a cell for every column, got {row:?}"))
        .to_string()
}

/// The projected cell of `dataset`'s row under `column`: the exact text, which
/// no column width can elide.
fn projected_cell(app: &AppState, dataset: &str, column: &str) -> String {
    let model = project_resources(&app.root.resources)
        .table
        .expect("list mode builds a table model");
    let index = |key: &str| {
        model
            .columns
            .iter()
            .position(|definition| definition.key == key)
            .unwrap_or_else(|| panic!("the datasets table carries a {key} column"))
    };
    let (name_column, wanted) = (index("name"), index(column));
    model
        .rows
        .iter()
        .find(|row| row.cells[name_column].text == dataset)
        .unwrap_or_else(|| panic!("the {dataset} fixture row is projected"))
        .cells[wanted]
        .text
        .clone()
}

// --- The catalogue ---

#[test]
fn the_catalogue_is_the_three_agreed_columns_in_order() {
    let keys = BIGQUERY_DATASET_COLUMNS
        .iter()
        .map(|column| column.spec.key)
        .collect::<Vec<_>>();

    assert_eq!(keys, vec!["name", "location", "labels"]);
}

/// The friendly name has no column, and that is a decision rather than an
/// omission — see [`BIGQUERY_DATASET_COLUMNS`]'s own docs. Pinned so that
/// adding one is a deliberate edit here rather than a silent widening of a
/// table whose narrow-mode budget is already spent.
#[test]
fn the_friendly_name_has_no_column_of_its_own() {
    let headers = BIGQUERY_DATASET_COLUMNS
        .iter()
        .map(|column| column.spec.header)
        .collect::<Vec<_>>();

    assert!(
        !headers.iter().any(|header| header.contains("FRIENDLY")),
        "the friendly name is searchable and shown in the preview, not \
         columned: it is empty on every dataset of the captured listing, got \
         {headers:?}"
    );
}

#[test]
fn labels_is_the_one_column_that_does_not_sort() {
    let unsortable = catalog_meta(ResourceServiceKind::BigQueryDatasets)
        .iter()
        .filter(|column| !column.sortable)
        .map(|column| column.key)
        .collect::<Vec<_>>();

    // A flattened `k=v, k=v` cell sorts by whichever key happens to sort
    // first, which is an order that means nothing.
    assert_eq!(unsortable, vec!["labels"]);
}

// --- The table ---

#[test]
fn the_datasets_table_paints_a_row_per_dataset() {
    let table = table_text(&painted(app_showing_datasets()));

    for id in [
        "analytics_events",
        "bare_dataset",
        "unlocated",
        "zz_undocumented",
    ] {
        assert!(
            table.contains(id),
            "the table must paint {id}, got:\n{table}"
        );
    }
}

#[test]
fn the_table_heads_the_three_columns() {
    let table = table_text(&painted_at(app_showing_datasets(), WIDE_FRAME_WIDTH));

    for header in ["DATASET", "LOCATION", "LABELS"] {
        assert!(
            table.contains(header),
            "the table must head {header}, got:\n{table}"
        );
    }
}

/// The `LOCATION` cell, both of its branches.
///
/// The rows carry different locations on purpose: a fixture whose rows all said
/// `US` would pass on a cell that painted a constant.
///
/// Every assertion names the `location` cell rather than scanning the row.
/// Dataset ids and region names are full of hyphens, so a whole-row search for
/// a dash proves nothing — and once a row is dashed in two cells, as
/// `zz_undocumented` is, even a row-scoped `any(|cell| cell == "-")` is
/// satisfied by the *labels* dash while `LOCATION` paints something else
/// entirely.
///
/// Each dash is asserted twice, on the frame and on the projection, because
/// the two see different things and neither alone is enough. The frame proves
/// the cell reaches the screen, but its cells are recovered by splitting on
/// column padding, so a cell painted *blank* vanishes and its neighbour slides
/// into the index — which is a real failure and one that reads, at the frame,
/// as the neighbour's value appearing under the wrong header. The projection
/// is index-true and cannot be fooled that way, but it does not prove anything
/// painted. A blanked `LOCATION` cell fails the frame assertion on `unlocated`
/// (`env=sandbox` slides left into the location index) and the projection
/// assertion on both rows; either alone would leave one of those two mistakes
/// unpinned.
#[test]
fn a_populated_location_paints_and_an_absent_one_paints_a_dash() {
    let app = app_showing_datasets();
    let painted = painted_at(app_showing_datasets(), WIDE_FRAME_WIDTH);

    assert_eq!(
        painted_cell(&painted, "analytics_events", "location"),
        "europe-southwest1",
        "a dataset's location must reach its own cell verbatim"
    );
    assert_eq!(
        painted_cell(&painted, "bare_dataset", "location"),
        "US",
        "a multi-region location must paint as itself, not as the region of \
         another row"
    );

    for dataset in ["unlocated", "zz_undocumented"] {
        assert_eq!(
            painted_cell(&painted, dataset, "location"),
            "-",
            "a listing that reported no location must paint the dash in the \
             LOCATION cell, which reads as 'not reported' where a blank reads \
             as a rendering fault ({dataset})"
        );
        assert_eq!(
            projected_cell(&app, dataset, "location"),
            "-",
            "the same, at the projection, where no column width can elide it \
             ({dataset})"
        );
    }

    assert!(
        !painted.contains("None"),
        "an absent optional renders as the dash, not as Rust's None:\n{painted}"
    );
}

/// The `LABELS` cell, pinned to its rendered text rather than to the column
/// merely existing.
///
/// Both branches of `bigquery_dataset_labels_cell` are asserted — the
/// flattened `k=v, k=v` form and the no-labels dash — because the function has
/// two of them and returning a constant satisfies neither.
///
/// The populated cell is asserted on the projection as well as the frame: it
/// pins the separator and the pair order exactly, and it cannot be softened by
/// a column width that elides the tail.
#[test]
fn the_labels_cell_paints_its_key_value_pairs_and_a_dash_when_empty() {
    let app = app_showing_datasets();
    let cell = |dataset: &str| projected_cell(&app, dataset, "labels");

    assert_eq!(
        cell("analytics_events"),
        "dg_data_confidentiality=public, team=platform",
        "every label must flatten into the one cell as key=value, comma \
         separated and in key order — not just the first, and not a constant"
    );
    for dataset in ["bare_dataset", "zz_undocumented"] {
        assert_eq!(
            cell(dataset),
            "-",
            "a dataset with no labels must fill its LABELS cell with a dash \
             rather than leave it empty ({dataset})"
        );
    }

    // Through the renderer too: the projection can be right while the cell
    // never reaches the screen, which is what this file exists to catch. The
    // pair asserted here is the short one, on the `unlocated` row: a
    // governance label runs past the column's width and the layout elides its
    // tail, which is the layout working. The exact flattening is pinned on the
    // projection above, where no width can soften it.
    let painted = painted_at(app_showing_datasets(), WIDE_FRAME_WIDTH);
    assert_eq!(
        painted_cell(&painted, "unlocated", "labels"),
        "env=sandbox",
        "a label must paint as key=value in its own cell, which is how an \
         operator reads and searches it"
    );
    for dataset in ["bare_dataset", "zz_undocumented"] {
        assert_eq!(
            painted_cell(&painted, dataset, "labels"),
            "-",
            "a dataset with no labels must paint the dash in the LABELS cell \
             on screen too, not an empty cell and not a dash borrowed from \
             LOCATION ({dataset})"
        );
    }
}

/// A dataset whose `friendlyName` is absent, and one whose `friendlyName` is
/// the empty string, are the two shapes the captured listing's *JSON* has —
/// `parse_datasets_json` folds both to `None`, so only the first survives the
/// parse. `Some("")` is pinned here regardless: the preview reads the field,
/// not the parser, so a renderer that dashed `None` while painting a blank
/// line for `Some("")` would pass on parsed rows alone and break the day the
/// fold moves or another loader fills the field. The preview is where the
/// friendly name is shown, so it is where the dash has to appear — and neither
/// shape may paint a blank line or panic the renderer.
#[test]
fn a_missing_friendly_name_previews_as_a_dash_for_both_of_its_shapes() {
    // Row 1 of the sorted table is `bare_dataset`, whose friendly name is
    // `None`; row 2 is `unlocated`, whose friendly name is `Some("")`.
    for (index, dataset) in [(1usize, "bare_dataset"), (2, "unlocated")] {
        let mut app = app_showing_datasets();
        for _ in 0..index {
            reduce(&mut app, &Action::Resources(ResourcesAction::SelectNext));
        }
        let preview =
            project_resource_preview(&app.root.resources).expect("a selected dataset previews");
        assert_eq!(
            preview.title, dataset,
            "the fixture ordering this test walks to must select {dataset}"
        );
        let friendly = preview
            .lines
            .iter()
            .find(|(label, _)| label == "Friendly name")
            .map(|(_, value)| value.clone())
            .expect("the dataset preview names the friendly name");
        assert_eq!(
            friendly, "-",
            "a dataset with no friendly name must preview the dash rather than \
             a blank that reads as a rendering fault ({dataset})"
        );
    }

    // A populated one still shows itself, or the dash above means nothing.
    let app = app_showing_datasets();
    let preview =
        project_resource_preview(&app.root.resources).expect("a selected dataset previews");
    assert_eq!(preview.title, "analytics_events");
    assert!(
        preview
            .lines
            .iter()
            .any(|(label, value)| label == "Friendly name" && value == "Analytics events"),
        "a dataset that has a friendly name must show it: {:?}",
        preview.lines
    );
}

// --- The empty states ---

/// The three status-selected empty branches of `render_bigquery_datasets_list`.
///
/// Every rendering test above paints a populated table, so nothing else in the
/// suite executes these at all: a wrong branch here shows an operator
/// "loading" when a fetch has failed, or a bare empty list when rows are merely
/// stale.
#[test]
fn the_datasets_list_paints_a_distinct_empty_state_per_status() {
    // Loading: a fetch in flight against a collection with no rows yet.
    let mut loading = app_with_context();
    loading.root.resources.begin_load(
        ResourceServiceKind::BigQueryDatasets,
        std::time::Instant::now(),
    );
    let frame = painted(loading);
    assert!(
        frame.contains("Loading datasets"),
        "an in-flight datasets fetch must paint the loading state, naming the \
         service it is loading:\n{frame}"
    );

    // Failed: the load-failed result for the active context.
    let mut failed = app_with_context();
    let context = failed.root.current_context().expect("active context");
    reduce(
        &mut failed,
        &Action::EffectResult(EffectResult::BigQueryDatasetsLoadFailed {
            context,
            error: nuvem::app::AppError::gcloud("permission denied"),
        }),
    );
    let frame = painted(failed);
    assert!(
        frame.contains("Press r to retry."),
        "a failed datasets fetch must paint the retry affordance, which is what \
         distinguishes it from an empty or stale list:\n{frame}"
    );

    // Stale: rows cleared pending a background refresh, with no load phase.
    let mut stale = app_showing_datasets();
    stale
        .root
        .resources
        .mark_active_stale_and_clear(ResourceServiceKind::BigQueryDatasets);
    let frame = painted(stale);
    assert!(
        frame.contains("Resource rows are stale. Press r to refresh."),
        "stale dataset rows must say they are stale, not read as a project with \
         no datasets in it:\n{frame}"
    );
}

/// The three *non*-status empty branches: a project with no datasets, a list
/// that was never loaded, and a filter that matches nothing.
///
/// The three read as opposite facts to an operator: "this project has no
/// datasets", "nothing has been fetched yet", and "your filter hid them all".
#[test]
fn the_datasets_list_distinguishes_no_datasets_from_not_loaded_from_filtered_out() {
    // Ready with zero rows: the project genuinely has no datasets.
    let mut empty = app_with_context();
    let context = empty.root.current_context().expect("active context");
    reduce(
        &mut empty,
        &Action::EffectResult(EffectResult::BigQueryDatasetsLoaded {
            context,
            datasets: Vec::new(),
        }),
    );
    let frame = painted(empty);
    assert!(
        frame.contains("No BigQuery datasets found for"),
        "a completed fetch that returned nothing must say the configuration \
         has no datasets:\n{frame}"
    );
    assert!(
        !frame.contains("Press r to load BigQuery datasets."),
        "a completed fetch must not invite a load it has already done:\n{frame}"
    );
    assert!(
        !frame.contains("No datasets match filter"),
        "an empty project is not a hidden-by-filter list:\n{frame}"
    );

    // Never loaded: the service is selected but no fetch has completed.
    let frame = painted(app_with_context());
    assert!(
        frame.contains("Press r to load BigQuery datasets."),
        "a list nobody has fetched must offer the load, not report an empty \
         project:\n{frame}"
    );
    assert!(
        !frame.contains("No BigQuery datasets found for"),
        "nothing is known about the project's datasets before a fetch:\n{frame}"
    );
    assert!(
        !frame.contains("No datasets match filter"),
        "an unfetched list is not a hidden-by-filter list:\n{frame}"
    );

    // Filtered out: rows exist, none of them survive the applied filter.
    let frame = painted(app_with_filter("zzz-matches-nothing"));
    assert!(
        frame.contains("No datasets match filter \"zzz-matches-nothing\""),
        "a filter that hides every row must name itself, so the operator knows \
         the rows are hidden rather than absent:\n{frame}"
    );
    assert!(
        !frame.contains("No BigQuery datasets found for"),
        "rows that exist behind a filter must not be reported as absent:\n{frame}"
    );
}

/// The list-tools header on the empty-state path: the line that tells an
/// operator staring at "no matches" what filter is applied and how many rows it
/// hid.
#[test]
fn a_filtered_empty_datasets_list_keeps_its_filter_and_row_count_header() {
    let frame = painted(app_with_filter("zzz-matches-nothing"));

    assert!(
        frame.contains("Filter: zzz-matches-nothing"),
        "the empty-state list must still carry the applied filter in its tools \
         header:\n{frame}"
    );
    assert!(
        frame.contains("Rows: 0/4"),
        "the tools header must report how many of the loaded rows survived the \
         filter, which is what distinguishes a hiding filter from an empty \
         project:\n{frame}"
    );
}

/// A label matches the filter though it has a column, and the friendly name
/// matches though it has none. The second is the half that pays for the column
/// decision: dropping `FRIENDLY NAME` is only safe while an operator who knows
/// a dataset by its display name can still find it.
#[test]
fn the_filter_matches_a_label_and_a_friendly_name_that_has_no_column() {
    let frame = painted(app_with_filter("Analytics events"));
    let table = table_text(&frame);
    assert!(
        table.contains("analytics_events"),
        "a filter typed as the dataset's friendly name must keep its row, \
         which is what makes the missing FRIENDLY NAME column safe:\n{frame}"
    );
    assert!(
        !table.contains("bare_dataset"),
        "the friendly-name filter must still hide the rows that do not match, \
         or it proves nothing:\n{frame}"
    );

    let frame = painted(app_with_filter("env=sandbox"));
    let table = table_text(&frame);
    assert!(
        table.contains("unlocated"),
        "a filter typed as a key=value label must keep the row carrying \
         it:\n{frame}"
    );
    assert!(
        !table.contains("bare_dataset"),
        "the label filter must still hide the unlabelled rows:\n{frame}"
    );
}

// --- The tables drill-down ---

/// Three tables inside `analytics_events`, constructed for the cells the
/// tables table has to paint rather than copied from the capture:
/// `tests/fixtures/bigquery_tables.json` holds two rows that are both `TABLE`
/// and both carry a `creationTime`, so neither a non-`TABLE` kind nor an
/// absent `created` — the two cells that can differ from a constant — is a
/// shape the capture contains.
///
/// The kinds deliberately differ, for the reason the datasets' locations do: a
/// fixture whose rows share one kind cannot tell a `TYPE` cell that paints
/// from one that paints a constant. `undated` is both the unknown kind carried
/// verbatim and the row whose timestamp is missing.
///
/// The three stats carry the three shapes that must not collapse into one
/// another, and they are on *different rows of one list* so that a single
/// painted frame separates them: `bikeshare_trips` has all three, and its
/// numbers are its own; `daily_rollup` has all three and every one of them is
/// a genuine zero; `undated` has none of them, because its `bq show` never
/// answered. A renderer that paints `0` for an unavailable stat is
/// indistinguishable from a correct one until a zero row and an unanswered row
/// are on screen together.
fn tables() -> Vec<BigQueryTable> {
    vec![
        BigQueryTable {
            dataset_id: "analytics_events".to_string(),
            table_id: "bikeshare_trips".to_string(),
            kind: BigQueryTableKind::Table,
            // 2017-05-25T03:43:34Z, the capture's own creationTime.
            created: Some(1_495_683_814_344),
            labels: vec![ComputeKeyValue {
                key: "team".to_string(),
                value: "platform".to_string(),
            }],
            rows: TableStat::Known(1_123_456),
            bytes: TableStat::Known(15_283),
            // 2026-08-21T10:03:33Z — later than this row's `created`, and a
            // different instant from the other row's, so a cell painting the
            // wrong field or a constant cannot pass.
            last_modified: TableStat::Known(1_787_306_613_694),
        },
        BigQueryTable {
            dataset_id: "analytics_events".to_string(),
            table_id: "daily_rollup".to_string(),
            kind: BigQueryTableKind::MaterializedView,
            // 2023-07-17T16:41:33Z, the capture's second row.
            created: Some(1_689_612_093_433),
            labels: Vec::new(),
            // Genuinely empty, and genuinely reported so: the half of the
            // `TableStat` distinction that a renderer dashing every falsy
            // value would get wrong.
            rows: TableStat::Known(0),
            bytes: TableStat::Known(0),
            // 2025-08-12T12:00:00Z.
            last_modified: TableStat::Known(1_755_000_000_000),
        },
        BigQueryTable {
            dataset_id: "analytics_events".to_string(),
            table_id: "undated".to_string(),
            kind: BigQueryTableKind::Other("SNAPSHOT".to_string()),
            created: None,
            labels: Vec::new(),
            // The row whose own `bq show` was refused or is still out: this is
            // the state every row starts in, and it must not read as a table
            // with nothing in it.
            rows: TableStat::Unavailable,
            bytes: TableStat::Unavailable,
            last_modified: TableStat::Unavailable,
        },
    ]
}

/// The datasets list with `analytics_events` entered and `tables` listed
/// inside it, through the real `Enter` action and the real load result.
fn app_showing_tables() -> AppState {
    app_inside_dataset_with(tables())
}

/// The three fixture tables with `filter` applied through the real filter
/// actions, which route to the tables view's own filter while the drill is
/// open.
fn app_with_tables_filter(filter: &str) -> AppState {
    let mut app = app_showing_tables();
    reduce(&mut app, &Action::Resources(ResourcesAction::StartFilter));
    for character in filter.chars() {
        reduce(
            &mut app,
            &Action::Resources(ResourcesAction::AppendFilterChar(character)),
        );
    }
    reduce(&mut app, &Action::Resources(ResourcesAction::ApplyFilter));
    app
}

/// The generation the tables listing among `effects` was issued under.
///
/// The generation is taken from the effect the drill emitted, which is where
/// a result gets its own in the running app.
fn listing_generation(effects: &[Effect]) -> BigQueryDrillGeneration {
    effects
        .iter()
        .find_map(|effect| match effect {
            Effect::LoadBigQueryTables { generation, .. } => Some(*generation),
            _ => None,
        })
        .expect("the drill must have issued a tables listing")
}

/// The listing epoch that same effect was issued under. Taken from the effect
/// for the same reason the generation is: it is where a result gets its epoch
/// in the running app, and the only way to hold an earlier call's after `r`.
fn listing_epoch(effects: &[Effect]) -> BigQueryListingEpoch {
    effects
        .iter()
        .find_map(|effect| match effect {
            Effect::LoadBigQueryTables { listing, .. } => Some(*listing),
            _ => None,
        })
        .expect("the drill must have issued a tables listing")
}

fn app_inside_dataset_with(rows: Vec<BigQueryTable>) -> AppState {
    let mut app = app_showing_datasets();
    let effects = reduce(&mut app, &Action::Resources(ResourcesAction::OpenSelected));
    let context = app.root.current_context().expect("active context");
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::BigQueryTablesLoaded {
            context,
            generation: listing_generation(&effects),
            listing: listing_epoch(&effects),
            tables: rows,
        }),
    );
    app
}

#[test]
fn the_tables_table_paints_a_row_per_table_and_no_dataset_rows() {
    let table = table_text(&painted(app_showing_tables()));

    for id in ["bikeshare_trips", "daily_rollup", "undated"] {
        assert!(
            table.contains(id),
            "the tables table must paint {id}, got:\n{table}"
        );
    }
    // The datasets the drill-down was entered from must be gone: a view
    // comparison flipped from `== Datasets` to `!= Datasets` still paints a
    // list, and only asking which rows are on screen tells the two apart.
    for id in ["bare_dataset", "unlocated", "zz_undocumented"] {
        assert!(
            !table.contains(id),
            "the tables view must not paint the datasets list's {id} row, \
             got:\n{table}"
        );
    }
}

#[test]
fn the_tables_table_heads_the_six_agreed_columns() {
    let table = table_text(&painted_at(app_showing_tables(), WIDE_FRAME_WIDTH));

    for header in ["TABLE", "TYPE", "CREATED", "ROWS", "BYTES", "LAST MODIFIED"] {
        assert!(
            table.contains(header),
            "the tables table must head {header}, got:\n{table}"
        );
    }
    for header in ["LOCATION", "LABELS"] {
        assert!(
            !table.contains(header),
            "the datasets table's {header} column must not head the tables \
             table, got:\n{table}"
        );
    }
}

/// The painted cell of `table`'s row under `column`, named by its catalogue
/// key. The tables sibling of [`painted_cell`], and load-bearing for the same
/// reason: a `-` anywhere in the row says nothing about which cell carries it,
/// and `undated` carries one in `CREATED` while its `TYPE` says `SNAPSHOT`.
fn painted_table_cell(painted: &str, table: &str, column: &str) -> String {
    let index = BIGQUERY_TABLE_COLUMNS
        .iter()
        .position(|definition| definition.spec.key == column)
        .unwrap_or_else(|| panic!("the tables catalogue carries a {column} column"));
    let row = table_row(painted, table);
    painted_cells(&row)
        .get(index)
        .unwrap_or_else(|| panic!("the {table} row paints a cell for every column, got {row:?}"))
        .to_string()
}

/// The `TYPE` cell, all three of the shapes the fixture has.
///
/// Asserted on the frame *and* the projection, for the reason the datasets'
/// `LOCATION` assertions are: the frame proves the cell reaches the screen but
/// recovers cells by splitting on column padding, so a blank cell vanishes and
/// its neighbour slides into the index; the projection is index-true but
/// proves nothing painted.
#[test]
fn the_type_cell_names_each_kind_and_carries_an_unknown_one_verbatim() {
    let app = app_showing_tables();
    let painted = painted_at(app_showing_tables(), WIDE_FRAME_WIDTH);

    for (table, expected) in [
        ("bikeshare_trips", "Table"),
        ("daily_rollup", "Materialized view"),
        ("undated", "SNAPSHOT"),
    ] {
        assert_eq!(
            painted_table_cell(&painted, table, "type"),
            expected,
            "the TYPE cell must name the row's own kind, not a constant \
             ({table})"
        );
        assert_eq!(
            projected_cell(&app, table, "type"),
            expected,
            "the same, at the projection ({table})"
        );
    }
}

/// The `CREATED` cell: the epoch-milliseconds timestamp rendered as the UTC
/// instant it names, and the dash for a row that has none.
///
/// The two populated rows carry different timestamps on purpose. A fixture
/// whose rows shared one cannot tell a cell that formats its own row's value
/// from one that formats a constant, and a wrong-unit conversion (seconds read
/// as milliseconds) lands on 1970 for both, which a same-value fixture would
/// paint identically and hide.
#[test]
fn the_created_cell_renders_the_instant_and_dashes_a_missing_timestamp() {
    let app = app_showing_tables();
    let painted = painted_at(app_showing_tables(), WIDE_FRAME_WIDTH);

    for (table, expected) in [
        ("bikeshare_trips", "2017-05-25T03:43:34Z"),
        ("daily_rollup", "2023-07-17T16:41:33Z"),
    ] {
        assert_eq!(
            painted_table_cell(&painted, table, "created"),
            expected,
            "creationTime is epoch milliseconds and must paint as the UTC \
             instant it names ({table})"
        );
        assert_eq!(
            projected_cell(&app, table, "created"),
            expected,
            "the same, at the projection, where no column width can elide it \
             ({table})"
        );
    }

    assert_eq!(
        painted_table_cell(&painted, "undated", "created"),
        "-",
        "a table whose listing carried no usable creationTime must paint the \
         dash in the CREATED cell, which reads as 'not reported' where a blank \
         reads as a rendering fault"
    );
    assert_eq!(
        projected_cell(&app, "undated", "created"),
        "-",
        "the same, at the projection"
    );
    assert!(
        !painted.contains("None"),
        "an absent optional renders as the dash, not as Rust's None:\n{painted}"
    );
}

/// The three enrichment cells, on one frame that carries all three shapes at
/// once: a row with values, a row whose values are genuine zeros, and a row
/// whose `bq show` never answered.
///
/// This is the assertion the whole `TableStat` type exists to make possible.
/// `Unavailable` is not a zero — it is a fact about the call, not about the
/// table — so `undated` paints the dash in all three cells while `daily_rollup`
/// paints `0` and `0 B` in two of them. Asserting that on one frame is the
/// point: a renderer that resolved absence to zero would still paint a
/// plausible table, and only a zero row standing beside an unanswered row on
/// the same screen tells the two apart.
///
/// Every cell is named by its catalogue key rather than by index, so inserting
/// a column ahead of these moves the assertions with it instead of quietly
/// re-pointing them at a neighbour.
#[test]
fn the_stat_cells_paint_a_value_a_zero_and_a_dash_on_one_frame() {
    let app = app_showing_tables();
    let painted = painted_at(app_showing_tables(), WIDE_FRAME_WIDTH);

    // The row that was enriched. The numbers differ per row and per column, so
    // a cell reading another row's stat, or another column's, fails here.
    for (column, expected) in [
        ("rows", "1123456"),
        ("bytes", "14.9 KiB"),
        ("last modified", "2026-08-21T10:03:33Z"),
    ] {
        assert_eq!(
            painted_table_cell(&painted, "bikeshare_trips", column),
            expected,
            "the {column} cell must paint this row's own answer"
        );
        assert_eq!(
            projected_cell(&app, "bikeshare_trips", column),
            expected,
            "the same, at the projection, where no column width can elide it \
             ({column})"
        );
    }

    // The row whose answers were zeros. A zero is a claim about the data and
    // must survive as one.
    for (column, expected) in [
        ("rows", "0"),
        ("bytes", "0 B"),
        ("last modified", "2025-08-12T12:00:00Z"),
    ] {
        assert_eq!(
            painted_table_cell(&painted, "daily_rollup", column),
            expected,
            "a table that really holds nothing must say so, not dash ({column})"
        );
        assert_eq!(
            projected_cell(&app, "daily_rollup", column),
            expected,
            "the same, at the projection ({column})"
        );
    }

    // The row whose `bq show` never answered. Rendering `0` here would state a
    // fact about the data where the truth is a fact about the call.
    for column in ["rows", "bytes", "last modified"] {
        assert_eq!(
            painted_table_cell(&painted, "undated", column),
            "-",
            "an unavailable stat paints the dash, never a zero ({column})"
        );
        assert_eq!(
            projected_cell(&app, "undated", column),
            "-",
            "the same, at the projection ({column})"
        );
    }

    // And the two must be distinguishable in the other direction too: the
    // zeros the fixture does carry have to be on screen somewhere, or an
    // assertion above could be passing over a table that paints dashes
    // everywhere.
    assert_ne!(
        painted_table_cell(&painted, "daily_rollup", "rows"),
        painted_table_cell(&painted, "undated", "rows"),
        "a zero row count and an unanswered one must not paint the same cell"
    );
}

/// A negative count is a value that *arrived*, so it must not paint the dash.
///
/// BigQuery does not report a negative `numRows` or `numBytes`, but nuvem's own
/// parser accepts one: `stat_from_raw` parses the wire string with `i64`, so a
/// malformed or hostile response yields `TableStat::Known(-1)` and reaches the
/// cells. The dash means "nobody answered", and a row that answered `-1` did
/// answer.
///
/// This is pinned here because nothing else can pin it. `cargo mutants`
/// generates only whole-function replacements for `bigquery_bytes_cell`, so
/// the `u64::try_from` fallback carries no mutant of its own: a later
/// "simplification" to `unwrap_or_default()` would paint `0 B` for a negative
/// — the slice's own defect, an unanswered-looking zero — and survive both the
/// gate and the suite.
#[test]
fn a_negative_count_paints_its_value_rather_than_a_dash_or_a_zero() {
    let app = app_inside_dataset_with(vec![BigQueryTable {
        dataset_id: "analytics_events".to_string(),
        table_id: "impossible".to_string(),
        kind: BigQueryTableKind::Table,
        created: None,
        labels: Vec::new(),
        rows: TableStat::Known(-1),
        bytes: TableStat::Known(-1),
        last_modified: TableStat::Unavailable,
    }]);

    assert_eq!(
        projected_cell(&app, "impossible", "rows"),
        "-1",
        "a negative row count is echoed, not dashed and not zeroed"
    );
    assert_eq!(
        projected_cell(&app, "impossible", "bytes"),
        "-1",
        "a negative byte count falls back to the digits rather than a size"
    );
    assert_ne!(
        projected_cell(&app, "impossible", "bytes"),
        "0 B",
        "a value that arrived must never read as a table of zero bytes"
    );
    assert_eq!(
        projected_cell(&app, "impossible", "last modified"),
        "-",
        "and the row's genuinely unanswered stat still dashes"
    );
}

/// The identity cell, pinned separately from the row merely being on screen:
/// `TABLE` is keyed `name` like every other catalogue's identity column, and a
/// cell that painted the dataset id instead would still put three rows on the
/// screen.
#[test]
fn the_table_cell_carries_the_table_id_not_the_dataset_id() {
    let painted = painted_at(app_showing_tables(), WIDE_FRAME_WIDTH);

    assert_eq!(
        painted_table_cell(&painted, "bikeshare_trips", "name"),
        "bikeshare_trips"
    );
    assert_eq!(
        painted_table_cell(&painted, "daily_rollup", "name"),
        "daily_rollup"
    );
}

/// The row of the painted frame carrying the table's tools line, found by the
/// row count only that line reports.
fn tools_row(painted: &str, rows: &str) -> String {
    painted
        .lines()
        .find(|line| line.contains(rows))
        .unwrap_or_else(|| panic!("the frame paints a tools line reporting {rows}:\n{painted}"))
        .to_string()
}

/// The tools line over the tables table, and the footer under it.
///
/// Two claims are absent by design. There is no `Sort:` segment and no
/// `←/→ col · s sort` hint: the tables list is painted in `bq ls` order and
/// has neither a sorted column nor a column cursor, so an arrow would claim an
/// order the rows do not have and the hints would advertise two controls this
/// table does not offer. The datasets table keeps both, which is what makes
/// their absence here a routing result rather than a deletion.
///
/// The footer is asserted alongside, and it is the half this test first got
/// wrong: the frame's hint row is a separate hardcoded list in
/// `projection::frame`, and a whole-frame `!contains("s sort")` passed over a
/// footer that was still hinting `s Sort column` one row lower on the same
/// screen — the identical false claim, in a different spelling, unasserted.
/// Both spellings are named here now, and the footer's own hint list is
/// checked at the projection, where a label cannot be missed by a substring.
#[test]
fn the_tables_tools_line_and_footer_claim_no_sort_and_no_column_cursor() {
    let tables = painted(app_showing_tables());
    let tools = tools_row(&tables, "Rows: 3/3");
    assert!(
        !tools.contains("Sort:"),
        "an unsorted list must not report a sort: {tools:?}"
    );
    assert!(
        !tools.contains("col ·"),
        "and must not offer the column cursor: {tools:?}"
    );

    // The footer's spelling of the same two claims, over the whole frame:
    // absent anywhere on screen, not merely absent from the tools row.
    for hint in ["Sort column", "Select column"] {
        assert!(
            !tables.contains(hint),
            "the tables view offers no {hint} key, so no row of the frame may \
             hint one:\n{tables}"
        );
    }

    let labels = nuvem::app::projection::project_app_frame(&app_showing_tables().root)
        .footer_hints
        .iter()
        .map(|hint| hint.label)
        .collect::<Vec<_>>();
    assert!(
        labels.contains(&"Leave drill-down"),
        "the footer is where the key out of the drill is found without opening \
         the help overlay: {labels:?}"
    );
    assert!(
        labels.contains(&"Open"),
        "and Enter is hinted again: it descends into the selected table's \
         columns, so the hint promises something the key delivers: {labels:?}"
    );
    for absent in ["Sort column", "Select column"] {
        assert!(
            !labels.contains(&absent),
            "{absent} names a key that does nothing in the tables view: {labels:?}"
        );
    }

    // The datasets list keeps all four, or every absence above is a deletion
    // rather than a routing result.
    let datasets = painted(app_showing_datasets());
    assert!(
        tools_row(&datasets, "Rows: 4/4").contains("Sort: Dataset ▲"),
        "the datasets tools line still reports its sort:\n{datasets}"
    );
    let labels = nuvem::app::projection::project_app_frame(&app_showing_datasets().root)
        .footer_hints
        .iter()
        .map(|hint| hint.label)
        .collect::<Vec<_>>();
    for present in ["Sort column", "Select column", "Open"] {
        assert!(
            labels.contains(&present),
            "the datasets footer still hints {present}: {labels:?}"
        );
    }
    assert!(!labels.contains(&"Leave drill-down"), "{labels:?}");
}

/// The pane title names the dataset the drill is inside. Without it the tables
/// table is three rows with no statement of what they are inside of — the
/// object browser names its bucket and prefix for the same reason.
#[test]
fn the_tables_pane_title_names_the_entered_dataset() {
    let frame = painted(app_showing_tables());

    assert!(
        frame.contains("Resources / BigQuery analytics_events"),
        "the tables pane must name the dataset it is inside:\n{frame}"
    );
    assert!(
        !frame.contains("Resources / BigQuery datasets"),
        "the datasets pane title must be gone while the drill is open:\n{frame}"
    );
}

/// The preview describes the highlighted *table*, not the dataset it is
/// inside: the pane is about the row the cursor is on, and the dataset is
/// already named in the title beside it.
#[test]
fn the_preview_describes_the_selected_table() {
    let mut app = app_showing_tables();
    let preview = project_resource_preview(&app.root.resources).expect("a selected table previews");
    assert_eq!(preview.title, "bikeshare_trips");
    assert!(
        preview
            .lines
            .iter()
            .any(|(label, value)| label == "Labels" && value == "team=platform"),
        "the labels have no column, so the preview is where they are read: {:?}",
        preview.lines
    );

    // It tracks the cursor, or it is the dataset preview wearing a table's
    // name.
    reduce(&mut app, &Action::Resources(ResourcesAction::SelectNext));
    let preview = project_resource_preview(&app.root.resources).expect("a selected table previews");
    assert_eq!(preview.title, "daily_rollup");
    assert!(
        preview
            .lines
            .iter()
            .any(|(label, value)| label == "Labels" && value == "-"),
        "a table with no labels previews the dash: {:?}",
        preview.lines
    );
}

/// The `TYPE` cell is filterable by the word it paints.
///
/// The kind is the one cell whose text is computed rather than carried — it is
/// `BigQueryTableKind::label`, not a field — so it is the one that can drift
/// from what the filter matches. A term an operator can read in a column but
/// cannot filter by is the filter lying about the rows it hid.
#[test]
fn the_filter_matches_the_type_cell_by_the_word_it_paints() {
    let frame = painted(app_with_tables_filter("SNAPSHOT"));
    let table = table_text(&frame);
    assert!(
        table.contains("undated"),
        "a filter typed as the word in the TYPE cell must keep the row \
         carrying it:\n{frame}"
    );
    assert!(
        !table.contains("daily_rollup"),
        "the kind filter must still hide the rows of other kinds, or it proves \
         nothing:\n{frame}"
    );

    // And the table id still matches, so the widening above did not replace
    // what was searchable before.
    let table = table_text(&painted(app_with_tables_filter("daily_rollup")));
    assert!(table.contains("daily_rollup"), "{table}");
    assert!(!table.contains("undated"), "{table}");
}

/// The four empty branches of `render_bigquery_tables_list`, which no other
/// test in the suite executes: every assertion above paints a populated table.
///
/// There are four and not the datasets list's six. A *stale* state cannot
/// arise — leaving the view drops the drill scope, so tables are never held
/// past the context that fetched them — and neither can a *never loaded* one,
/// because entering a dataset is what issues the listing.
#[test]
fn the_tables_list_paints_a_distinct_empty_state_per_outcome() {
    // In flight: entered, with no result yet.
    let mut loading = app_showing_datasets();
    reduce(
        &mut loading,
        &Action::Resources(ResourcesAction::OpenSelected),
    );
    let frame = painted(loading);
    assert!(
        frame.contains("Loading tables"),
        "an in-flight listing must say it is loading tables, not datasets:\n{frame}"
    );
    assert!(
        !frame.contains("elapsed"),
        "and must not carry the shared helper's elapsed counter: it is fed by \
         the datasets collection's load phase, which the tables listing does \
         not have, so it would sit at (0s elapsed) forever:\n{frame}"
    );

    // Failed: the listing came back an error, and the view stays inside the
    // dataset so the operator can retry it.
    let mut failed = app_showing_datasets();
    let effects = reduce(
        &mut failed,
        &Action::Resources(ResourcesAction::OpenSelected),
    );
    let context = failed.root.current_context().expect("active context");
    reduce(
        &mut failed,
        &Action::EffectResult(EffectResult::BigQueryTablesLoadFailed {
            context,
            generation: listing_generation(&effects),
            listing: listing_epoch(&effects),
            error: nuvem::app::AppError::gcloud("permission denied"),
        }),
    );
    let frame = painted(failed);
    assert!(
        frame.contains("Press r to retry."),
        "a failed listing must offer the retry, which is what distinguishes it \
         from a dataset with no tables in it:\n{frame}"
    );
    assert!(
        frame.contains("Resources / BigQuery analytics_events"),
        "a failed listing must leave the view inside the dataset it was \
         about:\n{frame}"
    );

    // Landed empty: the dataset genuinely holds no tables.
    let frame = painted(app_inside_dataset_with(Vec::new()));
    assert!(
        frame.contains("No tables in analytics_events"),
        "a listing that returned nothing must name the dataset it found empty, \
         not report a failure:\n{frame}"
    );
    assert!(
        !frame.contains("Press r to retry."),
        "an empty dataset is not a failed call:\n{frame}"
    );

    // Filtered out: tables exist, none survive the applied filter.
    let mut filtered = app_showing_tables();
    reduce(
        &mut filtered,
        &Action::Resources(ResourcesAction::StartFilter),
    );
    for character in "zzz-matches-nothing".chars() {
        reduce(
            &mut filtered,
            &Action::Resources(ResourcesAction::AppendFilterChar(character)),
        );
    }
    reduce(
        &mut filtered,
        &Action::Resources(ResourcesAction::ApplyFilter),
    );
    let frame = painted(filtered);
    assert!(
        frame.contains("No tables match filter \"zzz-matches-nothing\""),
        "a filter that hides every table must name itself, so the operator \
         knows the rows are hidden rather than absent:\n{frame}"
    );
    assert!(
        !frame.contains("No tables in analytics_events"),
        "tables that exist behind a filter must not be reported as absent:\n{frame}"
    );
    assert!(
        frame.contains("Rows: 0/3"),
        "the tools header must report how many of the listed tables survived \
         the filter:\n{frame}"
    );
    // The empty-state tools header is the one place the tables view's sort
    // *label* is read — the populated table drops the whole segment — so it is
    // asserted here rather than left to the branch that never runs.
    assert!(
        frame.contains("Sort: Listing order"),
        "an unsorted list must say so rather than borrow the datasets list's \
         sort field, which is still `Dataset` behind the drill:\n{frame}"
    );
    assert!(
        !frame.contains("Sort: Dataset"),
        "the datasets list's sort field must not be reported over the tables \
         list:\n{frame}"
    );
}

/// Narrow mode gives up the three enrichment columns first, then `CREATED`,
/// and `TYPE` last of the five.
///
/// Two arguments are pinned here. The catalogue's own paragraph says the kind
/// is what changes what a reader may assume about a row, so it must outrank
/// the date. And the three `bq show` columns go before either, because one
/// refused `bq show` dashes all three for every row at once, while `CREATED`
/// comes from the listing this view already required to succeed — width spent
/// on a column that may be dashes throughout buys less than width spent on one
/// that answered. `plan_table` drops the *largest* priority value first, which
/// is exactly the direction a priority list is easy to write backwards, so the
/// order is pinned by the plan rather than by reading the numbers back.
///
/// Read off the plan at every width from wide to narrow, as the sequence of
/// distinct column sets it passes through, so the claim is the order of the
/// drops rather than the width each one happens at.
#[test]
fn narrowing_drops_the_enrichment_columns_then_created_before_type() {
    let app = app_showing_tables();
    let model = project_resources(&app.root.resources)
        .table
        .expect("the tables view builds a table model");
    let visible = |width: u16| {
        plan_table(&model, width, false)
            .columns
            .iter()
            .filter(|column| column.visible)
            .map(|column| model.columns[column.index].key)
            .collect::<Vec<_>>()
    };
    let mut sequence: Vec<Vec<&str>> = Vec::new();
    for width in (1..=200u16).rev() {
        let keys = visible(width);
        if sequence.last() != Some(&keys) {
            sequence.push(keys);
        }
    }

    assert_eq!(
        sequence,
        vec![
            vec!["name", "type", "created", "rows", "bytes", "last modified"],
            // The first column narrow mode gives up must be LAST MODIFIED,
            vec!["name", "type", "created", "rows", "bytes"],
            // then BYTES,
            vec!["name", "type", "created", "rows"],
            // then ROWS — the three `bq show` columns go before the two the
            // listing itself answered —
            vec!["name", "type", "created"],
            // and only then CREATED: a table whose TYPE went first would tell
            // a reader a view was a table.
            vec!["name", "type"],
            vec!["name"],
        ]
    );
}

// --- The schema view, at the projection ---

/// `analytics_events`' `bikeshare_trips` entered, with `fields` landed as its
/// schema — through the real `Enter` action and the real load result, so the
/// view, the drill scope and the generation are the ones the running app has.
fn app_inside_table_with(fields: Vec<BigQuerySchemaField>) -> AppState {
    let mut app = app_showing_tables();
    let effects = reduce(&mut app, &Action::Resources(ResourcesAction::OpenSelected));
    let context = app.root.current_context().expect("active context");
    let generation = effects
        .iter()
        .find_map(|effect| match effect {
            Effect::LoadBigQuerySchema { generation, .. } => Some(*generation),
            _ => None,
        })
        .expect("entering a table must have issued a schema load");
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::BigQuerySchemaLoaded {
            context,
            generation,
            fields,
        }),
    );
    app
}

/// Five columns: a top-level one carrying a description, a repeated record
/// with none, and the record's children two levels deep — one of them
/// described and one not.
///
/// The described row beside the undescribed one is the shape the `DESCRIPTION`
/// cell is pinned against: a renderer that paints the empty string for an
/// absent description passes any fixture whose rows all carry one. The
/// two-level path is the other: `event_params.value.string_value` is what a
/// flattener that kept only the leaf name, or only the first dot, would get
/// wrong.
fn schema_fields() -> Vec<BigQuerySchemaField> {
    vec![
        BigQuerySchemaField {
            path: "trip_id".to_string(),
            field_type: "STRING".to_string(),
            mode: "REQUIRED".to_string(),
            description: Some("The trip's own id".to_string()),
        },
        BigQuerySchemaField {
            path: "event_params".to_string(),
            field_type: "RECORD".to_string(),
            mode: "REPEATED".to_string(),
            description: None,
        },
        BigQuerySchemaField {
            path: "event_params.key".to_string(),
            field_type: "STRING".to_string(),
            mode: "NULLABLE".to_string(),
            description: None,
        },
        BigQuerySchemaField {
            path: "event_params.value".to_string(),
            field_type: "RECORD".to_string(),
            mode: "NULLABLE".to_string(),
            description: None,
        },
        BigQuerySchemaField {
            path: "event_params.value.string_value".to_string(),
            field_type: "STRING".to_string(),
            mode: "NULLABLE".to_string(),
            description: Some("The parameter's string value".to_string()),
        },
    ]
}

/// The row count over the schema view counts *columns*, not the datasets two
/// levels behind it.
///
/// The tools header's `Rows: n/m` is the projection's `visible_rows` and
/// `total_rows`, and under this view both belong to the schema. Reporting the
/// datasets' total here attributes one list's size to another — the same
/// defect the tables arm exists to prevent, one level down.
#[test]
fn the_row_count_over_the_schema_counts_its_columns() {
    let app = app_inside_table_with(schema_fields());

    let projection = project_resources(&app.root.resources);

    assert_eq!(
        (projection.total_rows, projection.visible_rows),
        (5, 5),
        "the five columns of the entered table are what the header counts, \
         not the four datasets behind the drill"
    );
}

/// The preview beside the schema describes the column the cursor is on.
///
/// Falling through to the datasets arm paints a dataset's location, project
/// and friendly name beside a list of columns — a pane describing a row that
/// is not on screen.
#[test]
fn the_preview_beside_the_schema_describes_the_selected_column() {
    let app = app_inside_table_with(schema_fields());

    let preview =
        project_resource_preview(&app.root.resources).expect("a selected column previews");

    assert_eq!(
        preview.title, "trip_id",
        "the pane is about the highlighted column, not about the dataset the \
         drill started from"
    );
    assert_eq!(
        preview
            .lines
            .iter()
            .map(|(label, value)| (label.as_str(), value.as_str()))
            .collect::<Vec<_>>(),
        vec![
            ("Type", "STRING"),
            ("Mode", "REQUIRED"),
            ("Description", "The trip's own id"),
        ],
    );
}

/// The footer over the schema view hints only keys that do something here.
///
/// A hint is a promise about the key it names. `Sort column` and `Select
/// column` act on the datasets collection behind two levels of drill, and
/// `Enter` opens nothing: a column is the bottom of this hierarchy. The one
/// key this view has that the list above it does not is the way back up, and
/// the footer is where an operator finds it without opening the help overlay.
#[test]
fn the_schema_footer_hints_only_keys_that_do_something_here() {
    let app = app_inside_table_with(schema_fields());

    let labels = nuvem::app::projection::project_app_frame(&app.root)
        .footer_hints
        .iter()
        .map(|hint| hint.label)
        .collect::<Vec<_>>();

    assert!(
        labels.contains(&"Leave drill-down"),
        "Esc leaves the schema for the tables list, so the footer says so: {labels:?}"
    );
    for absent in ["Sort column", "Select column", "Open"] {
        assert!(
            !labels.contains(&absent),
            "{absent} names a key that does nothing in the schema view: {labels:?}"
        );
    }
}

// --- The schema view, on the screen ---

/// The painted cell of `field`'s row under `column`, named by its catalogue
/// key. The schema sibling of [`painted_table_cell`], and keyed for the same
/// reason: an index assertion rots silently the moment a column is inserted,
/// and this catalogue is the newest of the three.
fn painted_schema_cell(painted: &str, field: &str, column: &str) -> String {
    let index = |key: &str| {
        BIGQUERY_SCHEMA_COLUMNS
            .iter()
            .position(|definition| definition.spec.key == key)
            .unwrap_or_else(|| panic!("the schema catalogue carries a {key} column"))
    };
    let (name, wanted) = (index("name"), index(column));
    // The row is found by its `FIELD` cell rather than by a substring of the
    // line, which `table_row` would do: every child of `event_params` carries
    // the parent's path as a prefix, so a substring search for the record
    // itself is satisfied by four different rows.
    let cells = table_lines(painted)
        .into_iter()
        .map(painted_cells)
        .find(|cells| cells.get(name) == Some(&field))
        .unwrap_or_else(|| panic!("the schema table paints a {field} row:\n{painted}"));
    cells
        .get(wanted)
        .unwrap_or_else(|| panic!("the {field} row paints a {column} cell, got {cells:?}"))
        .to_string()
}

/// Entering a table replaces what paints, not merely what state reports.
///
/// The schema view routed to `render_bigquery_datasets_list` for two commits,
/// which painted the datasets two levels behind the drill; once the projection
/// started reporting the schema's own counts, that renderer fell through every
/// empty branch it has into its `unreachable!`, and the frame panicked. So this
/// asserts on the buffer: the fields are on it and the datasets are not.
#[test]
fn entering_a_table_paints_its_columns_rather_than_the_datasets_behind_it() {
    let painted = painted_at(app_inside_table_with(schema_fields()), WIDE_FRAME_WIDTH);
    let table = table_text(&painted);

    for field in ["trip_id", "event_params.value.string_value"] {
        assert!(
            table.contains(field),
            "the schema view must paint the entered table's {field} column, \
             got:\n{table}"
        );
    }
    for dataset in ["bare_dataset", "zz_undocumented"] {
        assert!(
            !table.contains(dataset),
            "the schema view must not paint the datasets list's {dataset} \
             row, got:\n{table}"
        );
    }
    assert!(
        painted.contains("Resources / BigQuery analytics_events.bikeshare_trips"),
        "the pane names the table the columns belong to, the way the tables \
         view names its dataset — a title reading 'BigQuery datasets' over a \
         list of columns says the drill is somewhere it is not, got:\n{painted}"
    );
}

#[test]
fn the_schema_table_heads_the_four_agreed_columns() {
    let table = table_text(&painted_at(
        app_inside_table_with(schema_fields()),
        WIDE_FRAME_WIDTH,
    ));

    for header in ["FIELD", "TYPE", "MODE", "DESCRIPTION"] {
        assert!(
            table.contains(header),
            "the schema table must head {header}, got:\n{table}"
        );
    }
    for header in ["LOCATION", "LABELS", "CREATED", "LAST MODIFIED"] {
        assert!(
            !table.contains(header),
            "{header} heads a list above the schema and must not head this \
             one, got:\n{table}"
        );
    }
}

/// The four cells, in one painted frame.
///
/// One frame rather than four, because what each assertion is really about is
/// the *other* cells of the same row: a nested path proves the `FIELD` cell
/// carries the whole dotted path and not a leaf, `REPEATED` proves `MODE` is
/// the row's own and not the API default, and the absent description beside a
/// present one is the only shape that separates a renderer painting `-` from
/// one painting the empty string. A fixture whose rows all carried a
/// description would pass either way.
#[test]
fn the_schema_cells_carry_the_path_the_mode_and_the_description() {
    let app = app_inside_table_with(schema_fields());
    let painted = painted_at(app_inside_table_with(schema_fields()), WIDE_FRAME_WIDTH);

    assert_eq!(
        painted_schema_cell(&painted, "event_params.value.string_value", "name"),
        "event_params.value.string_value",
        "the FIELD cell is the whole dotted path — that is what makes a \
         nested field findable and what `/event_params` selects a record by"
    );
    assert_eq!(
        painted_schema_cell(&painted, "event_params.value.string_value", "type"),
        "STRING",
        "the TYPE cell is the leaf's own type, not its parent record's"
    );

    assert_eq!(
        painted_schema_cell(&painted, "event_params", "mode"),
        "REPEATED",
        "a repeated record must say so: REPEATED is the fact that makes the \
         rows below it an array rather than a struct"
    );
    assert_eq!(
        projected_cell(&app, "event_params", "mode"),
        "REPEATED",
        "the same, at the projection, where no column width can elide it"
    );

    assert_eq!(
        painted_schema_cell(&painted, "trip_id", "description"),
        "The trip's own id",
        "a described field paints its description"
    );
    assert_eq!(
        painted_schema_cell(&painted, "event_params", "description"),
        "-",
        "a field the schema documents with nothing must paint the dash, which \
         reads as 'no description' where a blank reads as a rendering fault"
    );
    assert_eq!(
        projected_cell(&app, "event_params", "description"),
        "-",
        "the same, at the projection, where an empty cell cannot hide in the \
         padding"
    );
}

/// The four empty branches of `render_bigquery_schema_list`, which nothing
/// else in the suite executes: every schema assertion above paints a populated
/// table.
///
/// The sibling of `the_tables_list_paints_a_distinct_empty_state_per_outcome`,
/// one level down, and there are four here for the same reasons there are four
/// there. A *stale* schema cannot arise — leaving the view drops the scope —
/// and neither can a *never loaded* one, because entering a table is what
/// issues the `bq show`.
///
/// All four name the entered table in the pane title. That is the half a
/// renderer reaching for the collection behind the drill gets wrong, and the
/// half that tells an operator which of the two calls in flight failed.
#[test]
fn the_schema_list_paints_a_distinct_empty_state_per_outcome() {
    // In flight: entered, with no result yet.
    let mut loading = app_showing_tables();
    reduce(
        &mut loading,
        &Action::Resources(ResourcesAction::OpenSelected),
    );
    let frame = painted(loading);
    assert!(
        frame.contains("Loading columns"),
        "an in-flight schema read must say it is loading columns, not tables:\n{frame}"
    );
    assert!(
        !frame.contains("elapsed"),
        "and must not carry the shared helper's elapsed counter: it is fed by \
         the datasets collection's load phase, which the schema read does not \
         have, so it would sit at (0s elapsed) forever:\n{frame}"
    );

    // Failed: the `bq show` came back an error, and the view stays inside the
    // table so the operator can retry it with `r`.
    let mut failed = app_showing_tables();
    let effects = reduce(
        &mut failed,
        &Action::Resources(ResourcesAction::OpenSelected),
    );
    let context = failed.root.current_context().expect("active context");
    let generation = effects
        .iter()
        .find_map(|effect| match effect {
            Effect::LoadBigQuerySchema { generation, .. } => Some(*generation),
            _ => None,
        })
        .expect("entering a table must have issued a schema load");
    reduce(
        &mut failed,
        &Action::EffectResult(EffectResult::BigQuerySchemaLoadFailed {
            context,
            generation,
            error: nuvem::app::AppError::gcloud("permission denied"),
        }),
    );
    let frame = painted(failed);
    assert!(
        frame.contains("Press r to retry."),
        "a refused read must offer the retry, which is what distinguishes it \
         from a table with no columns in it:\n{frame}"
    );
    assert!(
        frame.contains("Resources / BigQuery analytics_events.bikeshare_trips"),
        "a refused read must leave the view inside the table it was about, \
         qualified by its dataset:\n{frame}"
    );

    // Landed empty: the `show` answered, and its schema flattened to nothing.
    let frame = painted(app_inside_table_with(Vec::new()));
    assert!(
        frame.contains("No columns in analytics_events.bikeshare_trips"),
        "a read that returned nothing must name the table it found empty, not \
         report a failure:\n{frame}"
    );
    assert!(
        !frame.contains("Press r to retry."),
        "an empty schema is not a refused call:\n{frame}"
    );

    // Filtered out: columns exist, none survive the applied filter.
    let mut filtered = app_inside_table_with(schema_fields());
    reduce(
        &mut filtered,
        &Action::Resources(ResourcesAction::StartFilter),
    );
    for character in "zzz-matches-nothing".chars() {
        reduce(
            &mut filtered,
            &Action::Resources(ResourcesAction::AppendFilterChar(character)),
        );
    }
    reduce(
        &mut filtered,
        &Action::Resources(ResourcesAction::ApplyFilter),
    );
    let frame = painted(filtered);
    assert!(
        frame.contains("No columns match filter \"zzz-matches-nothing\""),
        "a filter that hides every column must name itself, so the operator \
         knows the rows are hidden rather than absent:\n{frame}"
    );
    assert!(
        !frame.contains("No columns in analytics_events.bikeshare_trips"),
        "columns that exist behind a filter must not be reported as absent:\n{frame}"
    );
    assert!(
        frame.contains("Rows: 0/5"),
        "the tools header must report how many of the five columns survived \
         the filter — the schema's own count, not the three tables or the four \
         datasets behind it:\n{frame}"
    );
}
