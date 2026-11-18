//! The BigQuery slice of `ResourcesState`: the datasets collection, the filter
//! and the sort over it, and what a load result does to it.
//!
//! The `bq ls` argv and its parse are `tests/bigquery_service_test.rs` and are
//! not re-asserted here. What this file is about is what the state does with
//! rows once it has them — that they are visible and selectable, that `/`
//! narrows them client-side because `bq` accepts no server-side filter, that
//! the sort orders by the column it claims to, and that a refresh keeps the
//! cursor on the dataset the operator left it on rather than on an index.
//!
//! Several tests here exist because their subject would otherwise be an
//! accessor with no caller in `src/` and no test — a shape mutation testing
//! finds first. Each of those docstrings names the mutant it kills, so a later
//! reader can tell a regression test from coverage padding.

use nuvem::app::reducer::root::reduce;
use nuvem::app::state::resources::{BigQuery, BigQuerySortField, ResourceService, ResourcesState};
use nuvem::app::{Action, AppState, EffectResult, ResourcesAction};
use nuvem_domain::gcp::configuration::{
    GcloudConfiguration, GcloudConfigurationName, GcloudContext,
};
use nuvem_domain::gcp::filter::compile_filter;
use nuvem_domain::gcp::resource::{BigQueryDataset, ResourceServiceKind};

fn ctx() -> GcloudContext {
    GcloudContext {
        configuration: GcloudConfigurationName::new("dev"),
        account: None,
        project: Some("dev-project".into()),
        region: None,
        zone: None,
    }
}

fn dataset(dataset_id: &str) -> BigQueryDataset {
    BigQueryDataset {
        dataset_id: dataset_id.to_string(),
        project_id: "dev-project".to_string(),
        location: "US".to_string(),
        friendly_name: None,
        labels: Vec::new(),
    }
}

#[test]
fn a_loaded_dataset_list_is_visible_and_selectable() {
    let mut state = ResourcesState::default();
    state.select_service(ResourceServiceKind::BigQueryDatasets);

    state.set_bigquery_datasets(ctx(), vec![dataset("a"), dataset("b")]);

    assert_eq!(state.visible_bigquery_datasets().len(), 2);
    assert_eq!(
        state
            .selected_bigquery_dataset()
            .map(|dataset| dataset.dataset_id.as_str()),
        Some("a")
    );
}

#[test]
fn a_filter_narrows_the_dataset_list_locally() {
    let mut state = ResourcesState::default();
    state.select_service(ResourceServiceKind::BigQueryDatasets);
    state.set_bigquery_datasets(ctx(), vec![dataset("sales"), dataset("logs")]);

    state.start_filter();
    for character in "log".chars() {
        state.append_filter_char(character);
    }
    state.apply_filter();

    assert_eq!(state.visible_bigquery_datasets().len(), 1);
    assert_eq!(
        state
            .selected_bigquery_dataset()
            .map(|dataset| dataset.dataset_id.as_str()),
        Some("logs")
    );
}

// --- The visible count ---

/// `visible_bigquery_dataset_count` survived replacement by both `0` and `1`.
///
/// It is the `total`/`filtered` pair the table's row counter paints, and no
/// test above reads it — both assertions walk `visible_bigquery_datasets`
/// instead. Both constants are excluded by asserting a count that is neither,
/// and the filtered case pins that it counts what survives the filter rather
/// than the whole collection.
#[test]
fn the_visible_count_counts_the_rows_the_filter_leaves() {
    let mut state = ResourcesState::default();
    state.select_service(ResourceServiceKind::BigQueryDatasets);
    state.set_bigquery_datasets(
        ctx(),
        vec![dataset("sales"), dataset("sales_staging"), dataset("logs")],
    );

    assert_eq!(
        state.visible_bigquery_dataset_count(),
        3,
        "with no filter applied every loaded dataset is visible"
    );

    state.start_filter();
    for character in "sales".chars() {
        state.append_filter_char(character);
    }
    state.apply_filter();

    assert_eq!(
        state.visible_bigquery_dataset_count(),
        2,
        "the count must follow the filter, or the row counter contradicts the \
         rows the table paints"
    );
    assert_eq!(
        state.visible_bigquery_dataset_count(),
        state.visible_bigquery_datasets().len(),
        "the count and the list must never disagree: they are two readings of \
         one filter"
    );
}

// --- The sort ---

/// `compare_datasets` survived several mutations at once, and each assertion
/// below excludes one: replacing the `Location` arm's comparison with
/// `Ordering::Equal` (the location ordering), dropping the `then_with`
/// tie-break (the two `US` rows), and dropping the `Descending` reversal.
///
/// The location column is the only sortable column besides the identity, so a
/// `Location` arm that quietly compared dataset ids instead would look right
/// on any single-location fixture — which is what the real `bq` listing of a
/// public project is. Two locations and a tie inside one of them is the
/// smallest fixture that can tell the two apart.
#[test]
fn sorting_by_location_orders_by_location_and_breaks_ties_by_dataset_id() {
    let mut state = ResourcesState::default();
    state.select_service(ResourceServiceKind::BigQueryDatasets);
    state.set_bigquery_datasets(
        ctx(),
        vec![
            located("zeta", "US"),
            located("beta", "EU"),
            located("alpha", "US"),
        ],
    );

    // Column 1 is `location` in the catalogue: one step right of the identity
    // column, then sort it. Installs the field ascending.
    state.select_next_column();
    state.sort_selected_column();

    assert_eq!(
        dataset_ids(&state),
        vec!["beta", "alpha", "zeta"],
        "EU sorts before US, and the two US rows break their tie by dataset id"
    );

    state.sort_selected_column();

    assert_eq!(
        dataset_ids(&state),
        vec!["zeta", "alpha", "beta"],
        "a second press reverses the whole ordering, tie-break included"
    );
}

/// The default sort, pinned so `compare_datasets`' `DatasetId` arm cannot be
/// replaced by the `Location` comparison without a test noticing.
///
/// The fixture has to fight the tie-break to do that. An all-`US` fixture
/// looks like it isolates the id — and does the opposite: the swapped arm
/// returns `Equal` for every pair, `then_with` breaks every tie by dataset id,
/// and the mutant reproduces the expected order exactly. So one row carries a
/// location that sorts *before* the others while its id sorts *last*: under
/// the real arm `zeta` ends up third, under the swapped one its `EU` drags it
/// to the front. The orders disagree, which is the whole requirement.
#[test]
fn the_default_sort_orders_by_dataset_id() {
    let mut state = ResourcesState::default();
    state.select_service(ResourceServiceKind::BigQueryDatasets);
    state.set_bigquery_datasets(
        ctx(),
        vec![
            located("zeta", "EU"),
            located("alpha", "US"),
            located("mid", "US"),
        ],
    );

    assert_eq!(
        dataset_ids(&state),
        vec!["alpha", "mid", "zeta"],
        "the default sort is by dataset id: `zeta` sorts last despite its \
         location sorting first"
    );
}

// --- The sort field ---

/// `BigQuerySortField::label`, `from_column` and `column`, pinned per variant.
///
/// `label` survived replacement by both `""` and a constant, `from_column`
/// survived deletion of its column-0 arm, and `column` survived replacement by
/// the constant `1`. Nothing above reads any of them: the sort tests drive
/// sorting through the column cursor, which never spells the label or the
/// index. A wrong `from_column` silently sorts by a column the operator did
/// not pick, and a wrong `label` names a different one in the status line.
///
/// Modelled on Pub/Sub topics' `every_sortable_column_maps_to_its_field_and_carries_its_own_label`.
#[test]
fn every_sortable_column_maps_to_its_field_and_carries_its_own_label() {
    let by_column = [
        (0, BigQuerySortField::DatasetId, "Dataset"),
        (1, BigQuerySortField::Location, "Location"),
    ];

    for (index, field, label) in by_column {
        assert_eq!(
            BigQuerySortField::from_column(index),
            Some(field),
            "column {index} must map to {field:?}; a dropped arm sorts by \
             whatever the fallback picks"
        );
        assert_eq!(
            field.label(),
            label,
            "{field:?} must carry its own label, or the sort indicator names \
             the wrong column"
        );
        assert_eq!(
            field.column(),
            index,
            "`column` and `from_column` must be inverses, or the sort \
             indicator lands on a different column than the one sorting"
        );
    }

    // The labels must also be distinct: a constant satisfies every assertion
    // above if it happens to equal one of them, and two columns that read the
    // same tell the operator nothing.
    let labels = by_column
        .iter()
        .map(|(_, field, _)| field.label())
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(
        labels.len(),
        by_column.len(),
        "every sort field needs its own label"
    );

    assert_eq!(
        BigQuerySortField::from_column(2),
        None,
        "column 2 is `labels`, the one catalogue column with no sort field; a \
         field here would sort by a flattened `k=v` cell"
    );
}

// --- The cursor across a reload ---

/// `set_bigquery_datasets`' cursor-retention branch, pinned on both sides.
///
/// The `previously_selected` lookup survived being replaced by `None` — the
/// `unwrap_or_else` fallback clamps the old index, and on a list whose rows
/// keep their positions that produces the same answer. A reload that *moves*
/// the selected row is what tells the two apart: retention follows the dataset,
/// the fallback follows the index.
#[test]
fn a_reload_keeps_the_cursor_on_the_dataset_it_was_on() {
    let mut state = ResourcesState::default();
    state.select_service(ResourceServiceKind::BigQueryDatasets);
    state.set_bigquery_datasets(ctx(), vec![dataset("alpha"), dataset("omega")]);
    state.select_next();

    assert_eq!(
        state
            .selected_bigquery_dataset()
            .map(|d| d.dataset_id.as_str()),
        Some("omega"),
        "the cursor starts on the row the reload must keep it on"
    );

    // `omega` is still listed but now sorts last of three, so index 1 is a
    // different row than it was.
    state.set_bigquery_datasets(
        ctx(),
        vec![dataset("alpha"), dataset("beta"), dataset("omega")],
    );

    assert_eq!(
        state
            .selected_bigquery_dataset()
            .map(|d| d.dataset_id.as_str()),
        Some("omega"),
        "a refresh must not move the cursor onto a different dataset under the \
         operator's hands"
    );
}

/// The other side of the same branch: when the selected dataset is gone, the
/// fallback clamps into the new list rather than leaving the cursor past its
/// end. Excludes replacing the `min` clamp with the raw index.
#[test]
fn a_reload_that_drops_the_selected_dataset_clamps_into_the_new_list() {
    let mut state = ResourcesState::default();
    state.select_service(ResourceServiceKind::BigQueryDatasets);
    state.set_bigquery_datasets(
        ctx(),
        vec![dataset("alpha"), dataset("beta"), dataset("omega")],
    );
    state.select_next();
    state.select_next();

    state.set_bigquery_datasets(ctx(), vec![dataset("alpha")]);

    assert_eq!(
        state
            .selected_bigquery_dataset()
            .map(|d| d.dataset_id.as_str()),
        Some("alpha"),
        "a cursor left past the end of a shorter list selects nothing at all"
    );
}

// --- The diagnostic ---

/// `bigquery_datasets_problem` survived replacement by `None`.
///
/// It is what the Home overview reads to show a service as degraded, and the
/// reducer asserts the *status message* rather than the problem, so the
/// accessor could report a healthy service forever. Both transitions are
/// pinned: a failed load raises the problem, a successful one clears it.
#[test]
fn a_failed_load_raises_a_problem_and_a_later_success_clears_it() {
    let mut app = app_showing_datasets();
    let context = app.root.current_context().expect("active context");

    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::BigQueryDatasetsLoadFailed {
            context: context.clone(),
            error: nuvem::app::AppError::gcloud("permission denied"),
        }),
    );

    assert!(
        app.root.resources.bigquery_datasets_problem().is_some(),
        "a failed datasets load must leave a problem for the overview to show, \
         or a degraded service reads as a healthy one"
    );

    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::BigQueryDatasetsLoaded {
            context,
            datasets: vec![dataset("alpha")],
        }),
    );

    assert!(
        app.root.resources.bigquery_datasets_problem().is_none(),
        "a successful load must clear the previous problem"
    );
}

// --- The generic service interface ---

/// `ResourceService` is how generic code reaches a service without a
/// per-service match, and BigQuery's impl has no caller in this slice — the
/// `visible_*` accessors call `dataset_matches_filter` and `compare_datasets`
/// directly. Unexercised is not the same as free to be wrong: the day a
/// generic path does reach it, it must answer what the tab already answers.
///
/// `matches_filter` would survive replacement by both `true` and `false`
/// without this; `row_id` would survive having the location folded in, which
/// is exactly the mistake its doc comment argues against.
#[test]
fn the_generic_service_impl_answers_what_the_tab_answers() {
    let filter = compile_filter("sales").expect("a compilable filter");
    let matching = dataset("sales");
    let other = dataset("logs");

    assert!(
        BigQuery::matches_filter(&matching, &filter),
        "a dataset whose id carries the term must match"
    );
    assert!(
        !BigQuery::matches_filter(&other, &filter),
        "a dataset that carries the term nowhere must not match"
    );
    assert_eq!(BigQuery::KIND, ResourceServiceKind::BigQueryDatasets);
    assert_eq!(
        BigQuery::row_id(&located("sales", "US")),
        BigQuery::row_id(&located("sales", "EU")),
        "the dataset id alone is the identity: folding the location in would \
         give one dataset two identities the day its listing came back without \
         one"
    );
}

// --- Helpers for the assertions above ---

fn located(dataset_id: &str, location: &str) -> BigQueryDataset {
    BigQueryDataset {
        location: location.to_string(),
        ..dataset(dataset_id)
    }
}

fn dataset_ids(state: &ResourcesState) -> Vec<&str> {
    state
        .visible_bigquery_datasets()
        .into_iter()
        .map(|d| d.dataset_id.as_str())
        .collect()
}

fn app_showing_datasets() -> AppState {
    let mut app = AppState::new();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ConfigurationsLoaded {
            configurations: vec![GcloudConfiguration {
                name: GcloudConfigurationName::new("dev"),
                is_active: true,
                account: None,
                project: Some("dev-project".to_string()),
                region: None,
                zone: None,
            }],
        }),
    );
    reduce(
        &mut app,
        &Action::Resources(ResourcesAction::SelectService(
            ResourceServiceKind::BigQueryDatasets,
        )),
    );
    app
}
