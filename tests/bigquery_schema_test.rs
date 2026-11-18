//! The third level's call: the one `bq show` an `Enter` on a table row issues,
//! and what its two answers do to the view.
//!
//! `tests/bigquery_drilldown_test.rs` owns the transitions between the three
//! lists and `tests/bigquery_enrichment_test.rs` the fan-out one level up.
//! What this file is about is the call between them and the failure split it
//! carries:
//!
//! - **One `Enter` is one call.** Not a fan-out: entering a table asks about
//!   that table and nothing else, and the effect names the row the cursor was
//!   on rather than the dataset behind the drill.
//! - **A refusal fails the drill.** There is nothing partial to show — this
//!   call is the whole content of the view the operator just opened — so the
//!   view keeps the table in its header and paints why, with no rows. That is
//!   the `bq ls` policy, deliberately not the per-row `bq show` policy that
//!   leaves one row's cells unavailable and reports nothing.
//! - **An empty schema is an empty level, not a failure.** `Ok(vec![])` is an
//!   answered call: no problem line, and the view stays on `Schema`. The way
//!   this regresses is an edit that treats "no rows" as "no answer", which
//!   would tell the operator the read failed when it succeeded.
//! - **No answer outlives the drill it was issued under.** A schema for a
//!   table the operator has left is dropped, and so is one answered for a
//!   gcloud context they have switched away from. Both guards are on both
//!   arms.

use nuvem::app::reducer::root::reduce;
use nuvem::app::state::resources::{BigQueryDrillGeneration, BigQueryView};
use nuvem::app::{Action, AppState, Effect, EffectResult, ResourcesAction};
use nuvem_domain::gcp::configuration::{
    GcloudConfiguration, GcloudConfigurationName, GcloudContext,
};
use nuvem_domain::gcp::resource::{
    BigQueryDataset, BigQuerySchemaField, BigQueryTable, BigQueryTableKind, ResourceServiceKind,
    TableStat,
};

fn dataset(dataset_id: &str) -> BigQueryDataset {
    BigQueryDataset {
        dataset_id: dataset_id.to_string(),
        project_id: "dev-project".to_string(),
        location: "US".to_string(),
        friendly_name: None,
        labels: Vec::new(),
    }
}

fn table(dataset_id: &str, table_id: &str) -> BigQueryTable {
    BigQueryTable {
        dataset_id: dataset_id.to_string(),
        table_id: table_id.to_string(),
        kind: BigQueryTableKind::Table,
        created: None,
        labels: Vec::new(),
        rows: TableStat::Unavailable,
        bytes: TableStat::Unavailable,
        last_modified: TableStat::Unavailable,
    }
}

fn field(path: &str) -> BigQuerySchemaField {
    BigQuerySchemaField {
        path: path.to_string(),
        field_type: "STRING".to_string(),
        mode: "NULLABLE".to_string(),
        description: None,
    }
}

fn app_with_datasets(datasets: Vec<BigQueryDataset>) -> AppState {
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
    let context = app.root.current_context().expect("an active context");
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::BigQueryDatasetsLoaded { context, datasets }),
    );
    app
}

fn enter(app: &mut AppState) -> Vec<Effect> {
    reduce(app, &Action::Resources(ResourcesAction::OpenSelected))
}

fn escape(app: &mut AppState) -> Vec<Effect> {
    reduce(app, &Action::Resources(ResourcesAction::LeaveDrillDown))
}

/// Lands `tables` for the listing `entering` asked for, under that listing's
/// own identity — taken off the effect rather than off the live counters, the
/// way a result gets its identity in the running app.
fn land_tables(app: &mut AppState, entering: &[Effect], tables: Vec<BigQueryTable>) {
    let (generation, listing) = entering
        .iter()
        .find_map(|effect| match effect {
            Effect::LoadBigQueryTables {
                generation,
                listing,
                ..
            } => Some((*generation, *listing)),
            _ => None,
        })
        .expect("entering a dataset must issue a tables listing");
    let context = app.root.current_context().expect("an active context");
    reduce(
        app,
        &Action::EffectResult(EffectResult::BigQueryTablesLoaded {
            context,
            generation,
            listing,
            tables,
        }),
    );
}

/// The schema loads among `effects`, as `(dataset_id, table_id, generation)`.
fn schema_loads(effects: &[Effect]) -> Vec<(String, String, BigQueryDrillGeneration)> {
    effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::LoadBigQuerySchema {
                dataset_id,
                table_id,
                generation,
                ..
            } => Some((dataset_id.clone(), table_id.clone(), *generation)),
            _ => None,
        })
        .collect()
}

/// The drill generation the one schema load among `effects` was issued under.
fn schema_generation(effects: &[Effect]) -> BigQueryDrillGeneration {
    let loads = schema_loads(effects);
    assert_eq!(
        loads.len(),
        1,
        "one Enter is one schema call, not none and not several: {effects:?}"
    );
    loads[0].2
}

/// An app inside `table_id`'s schema, waiting on the load, beside the
/// generation that load was issued under.
fn drilled_into_table(
    dataset_id: &str,
    tables: Vec<BigQueryTable>,
) -> (AppState, BigQueryDrillGeneration) {
    let mut app = app_with_datasets(vec![dataset(dataset_id)]);
    let entering = enter(&mut app);
    land_tables(&mut app, &entering, tables);
    let descending = enter(&mut app);
    let generation = schema_generation(&descending);
    (app, generation)
}

fn land_schema(
    app: &mut AppState,
    generation: BigQueryDrillGeneration,
    fields: Vec<BigQuerySchemaField>,
) {
    let context = app.root.current_context().expect("an active context");
    reduce(
        app,
        &Action::EffectResult(EffectResult::BigQuerySchemaLoaded {
            context,
            generation,
            fields,
        }),
    );
}

fn land_refusal(app: &mut AppState, generation: BigQueryDrillGeneration) {
    let context = app.root.current_context().expect("an active context");
    reduce(
        app,
        &Action::EffectResult(EffectResult::BigQuerySchemaLoadFailed {
            context,
            generation,
            error: nuvem::app::AppError::gcloud("permission denied"),
        }),
    );
}

/// A context a result can carry that is not the one the app is on. The drill
/// generation is a per-app counter, so an answer from another gcloud context
/// can carry the live generation exactly, and the context is then the only
/// thing telling the two apart.
fn other_context() -> GcloudContext {
    GcloudContext {
        configuration: GcloudConfigurationName::new("prod"),
        account: None,
        project: Some("prod-project".to_string()),
        region: None,
        zone: None,
    }
}

// --- One Enter is one call ---

/// The effect names the table row the cursor was on, and the dataset that row
/// belongs to. Both halves are needed: `bq show` addresses a table by
/// `dataset.table`, and a call naming only the table id would name a resource
/// that is not unique across the project.
#[test]
fn entering_a_table_issues_one_schema_load_for_that_row() {
    let mut app = app_with_datasets(vec![dataset("sales")]);
    let entering = enter(&mut app);
    land_tables(
        &mut app,
        &entering,
        vec![table("sales", "orders"), table("sales", "returns")],
    );

    let descending = enter(&mut app);

    assert_eq!(
        schema_loads(&descending),
        vec![(
            "sales".to_string(),
            "orders".to_string(),
            schema_generation(&descending)
        )],
        "one Enter is one call, for the selected row alone — a second call \
         here would be a fan-out this level has no reason to have"
    );
    assert_eq!(app.root.resources.bigquery_view(), BigQueryView::Schema);
    assert!(
        app.root.resources.bigquery_schema_loading(),
        "and the view says the call it just promised is in flight"
    );
}

/// The cursor decides which table, not the listing order. A schema load that
/// always named the first row would pass the test above and be wrong for every
/// other row an operator can reach.
#[test]
fn the_schema_load_follows_the_cursor_rather_than_the_first_row() {
    let mut app = app_with_datasets(vec![dataset("sales")]);
    let entering = enter(&mut app);
    land_tables(
        &mut app,
        &entering,
        vec![table("sales", "orders"), table("sales", "returns")],
    );
    reduce(&mut app, &Action::Resources(ResourcesAction::SelectNext));

    let descending = enter(&mut app);

    assert_eq!(
        schema_loads(&descending)
            .into_iter()
            .map(|(_, table_id, _)| table_id)
            .collect::<Vec<_>>(),
        vec!["returns".to_string()],
    );
}

/// A column is the bottom of the hierarchy, so `Enter` inside the schema view
/// issues nothing — and in particular must not be read as a second `Enter` on
/// the table row still selected behind it, which would re-issue the very call
/// the operator is waiting on.
#[test]
fn entering_inside_the_schema_view_issues_no_call() {
    let (mut app, generation) = drilled_into_table("sales", vec![table("sales", "orders")]);
    land_schema(&mut app, generation, vec![field("order_id")]);

    let effects = enter(&mut app);

    assert!(
        effects.is_empty(),
        "a column has nothing under it to open: {effects:?}"
    );
    assert_eq!(
        app.root.resources.bigquery_schema_fields().len(),
        1,
        "and the columns already loaded survive the keypress"
    );
}

// --- The failure split ---

/// A refused `bq show` fails the drill: the problem is painted and the column
/// list is emptied, the way a refused `bq ls` fails the tables listing.
///
/// Columns are landed first so that "emptied" is a real claim rather than a
/// restatement of the starting state. That is the half a best-effort refusal
/// would keep — the per-row `show` one level up leaves the row it failed on
/// exactly as it found it, and giving this call that policy would leave the
/// previous read's columns painted under a problem line saying the read
/// failed.
///
/// The view stays on `Schema` throughout. Dropping back to the tables list on
/// a refusal would take the operator somewhere they did not ask to be and lose
/// the header naming what they tried to open.
#[test]
fn a_refused_schema_read_fails_the_drill() {
    let (mut app, generation) = drilled_into_table("sales", vec![table("sales", "orders")]);
    land_schema(&mut app, generation, vec![field("order_id")]);

    land_refusal(&mut app, generation);

    assert_eq!(
        app.root.resources.bigquery_view(),
        BigQueryView::Schema,
        "the operator asked for this table's columns and is still looking at \
         that question"
    );
    assert!(
        app.root.resources.bigquery_schema_problem().is_some(),
        "a refused read must say so: an empty list with no problem line reads \
         as a table with no columns"
    );
    assert!(
        app.root.resources.bigquery_schema_fields().is_empty(),
        "and the columns the failed read is about must go with it: a problem \
         line over the previous read's rows contradicts them"
    );
    assert!(
        !app.root.resources.bigquery_schema_loading(),
        "and the call is no longer in flight"
    );
    assert_eq!(
        app.root
            .resources
            .bigquery_schema_table()
            .map(|table| table.table_id.as_str()),
        Some("orders"),
        "the header still names the table the refusal is about"
    );
}

/// The other half of the split, and the one an edit collapses by accident: a
/// schema that flattens to no fields is an *answered* call.
///
/// `bq show` succeeded; there is simply nothing under this table to list. A
/// problem line here would report a failure that did not happen, and would
/// send an operator hunting an IAM denial that is not there.
#[test]
fn a_schema_of_no_fields_is_an_empty_level_not_a_failure() {
    let (mut app, generation) = drilled_into_table("sales", vec![table("sales", "orders")]);

    land_schema(&mut app, generation, Vec::new());

    assert!(
        app.root.resources.bigquery_schema_problem().is_none(),
        "an answered call that listed nothing is not a failed call"
    );
    assert_eq!(app.root.resources.bigquery_view(), BigQueryView::Schema);
    assert!(
        app.root
            .resources
            .visible_bigquery_schema_fields()
            .is_empty()
    );
    assert!(
        !app.root.resources.bigquery_schema_loading(),
        "and the view must stop claiming a call is in flight, or the empty \
         level reads as one that never answered"
    );
}

/// An answered schema installs its columns and clears whatever a previous
/// refusal left painted: a problem line surviving a successful read would
/// contradict the rows beside it.
///
/// The ordering is reachable: `r` inside the schema view re-issues the `show`
/// for the table on screen, so a denied read followed by an answered one is
/// what a retry looks like. It is not reachable *under one generation*, which
/// is what this test lands both results under — a real retry mints a fresh
/// one. What the test pins is therefore `set_bigquery_schema`'s own
/// contract — installing rows clears the problem — against a later edit that
/// writes the rows and leaves a stale diagnostic standing over them. It is a
/// unit-level claim about the setter, reached through the reducer arm that
/// calls it.
#[test]
fn an_answered_schema_installs_its_columns_and_clears_the_problem() {
    let (mut app, generation) = drilled_into_table("sales", vec![table("sales", "orders")]);
    land_refusal(&mut app, generation);

    land_schema(
        &mut app,
        generation,
        vec![field("order_id"), field("customer")],
    );

    assert_eq!(
        app.root
            .resources
            .bigquery_schema_fields()
            .iter()
            .map(|field| field.path.as_str())
            .collect::<Vec<_>>(),
        vec!["order_id", "customer"],
    );
    assert!(
        app.root.resources.bigquery_schema_problem().is_none(),
        "the read that just succeeded is the same read the problem was about"
    );
}

// --- No answer outlives the drill it was issued under ---

/// A schema answered for a drill the operator has left does not land, even
/// though it is an answer about the table they are now looking at.
///
/// Re-entering the same table is a second drill wearing the first one's id, so
/// the table id could not tell these apart and the generation is what does.
/// The live load still being in flight is what makes the drop observable: a
/// stale answer that landed would end it.
#[test]
fn a_schema_from_an_earlier_entry_into_the_same_table_is_dropped() {
    let (mut app, first) = drilled_into_table("sales", vec![table("sales", "orders")]);
    // One `Esc` back to the tables list — the ascent is one level per keypress
    // — and one `Enter` back into the same table.
    escape(&mut app);
    let second = schema_generation(&enter(&mut app));
    assert_ne!(
        first, second,
        "a second entry into a table is a second drill, or there is nothing \
         here to tell apart"
    );

    land_schema(&mut app, first, vec![field("stale_column")]);

    assert!(
        app.root.resources.bigquery_schema_fields().is_empty(),
        "the abandoned entry's columns must not fill the live one's list"
    );
    assert!(
        app.root.resources.bigquery_schema_loading(),
        "and the load the view is actually waiting on is still in flight"
    );
}

/// The refusal arm carries the same guard. Without it an abandoned drill's
/// denial would paint a problem over a table that is loading perfectly well.
#[test]
fn a_refusal_from_an_earlier_entry_into_the_same_table_is_dropped_too() {
    let (mut app, first) = drilled_into_table("sales", vec![table("sales", "orders")]);
    escape(&mut app);
    let second = schema_generation(&enter(&mut app));
    assert_ne!(first, second);

    land_refusal(&mut app, first);

    assert!(
        app.root.resources.bigquery_schema_problem().is_none(),
        "a denial for a drill nobody is inside must not fail the drill they \
         are inside"
    );
    assert!(app.root.resources.bigquery_schema_loading());
}

/// A schema answered under a gcloud context the operator has switched away
/// from does not land, and the generation cannot say so: it is a per-app
/// counter, so the other context's answer carries the live one exactly.
#[test]
fn a_schema_answered_for_another_context_does_not_land() {
    let (mut app, generation) = drilled_into_table("sales", vec![table("sales", "orders")]);

    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::BigQuerySchemaLoaded {
            context: other_context(),
            generation,
            fields: vec![field("prod_only_column")],
        }),
    );

    assert!(
        app.root.resources.bigquery_schema_fields().is_empty(),
        "another project's columns must never be painted under this project's \
         table"
    );
    assert!(app.root.resources.bigquery_schema_loading());
}

/// And the refusal arm, for the same reason: a denial in the project the
/// operator left is not a fact about the one they are in.
#[test]
fn a_refusal_from_another_context_does_not_land() {
    let (mut app, generation) = drilled_into_table("sales", vec![table("sales", "orders")]);

    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::BigQuerySchemaLoadFailed {
            context: other_context(),
            generation,
            error: nuvem::app::AppError::gcloud("permission denied"),
        }),
    );

    assert!(
        app.root.resources.bigquery_schema_problem().is_none(),
        "{:?}",
        app.root.resources.bigquery_schema_problem()
    );
    assert!(app.root.resources.bigquery_schema_loading());
}

// --- What the two arms say on the status line ---

/// A refused read reports itself under its own name, on both channels the
/// failure surfaces use.
///
/// Three separate side effects live on that arm and none of them was asserted
/// anywhere: the status message, the operation label it is filed under, and
/// the diagnostic the "copy diagnostic" key reads back. Deleting any one of
/// them left the suite green, which made three lines of shipped behaviour a
/// coverage gap rather than a decision.
///
/// The label matters as much as the message. `BigQuery tables` is the label
/// one level up, and a schema refusal filed under it would tell the operator
/// the *listing* failed while the listing sits behind them intact.
#[test]
fn a_refused_schema_read_names_itself_on_the_status_line_and_the_diagnostic() {
    let (mut app, generation) = drilled_into_table("sales", vec![table("sales", "orders")]);

    land_refusal(&mut app, generation);

    let status = app
        .root
        .system
        .status_message
        .clone()
        .expect("a refused schema read must leave a status message");
    assert_eq!(
        app.root.system.status_operation_label.as_deref(),
        Some("BigQuery schema"),
        "the status line files the refusal under the call that failed, not \
         under the tables listing one level up"
    );
    let diagnostic = app
        .root
        .system
        .last_diagnostic()
        .expect("a refused schema read must record a diagnostic to copy");
    assert_eq!(
        diagnostic.service, "BigQuery schema",
        "and the recorded diagnostic names the same call, since the copy key \
         hands its text to a human reading it out of context"
    );
    assert_eq!(
        status,
        diagnostic.headline(),
        "the line on screen and the diagnostic behind it must be one fact: a \
         status message built from anything else drifts from what the copy \
         key yields"
    );
    assert_eq!(
        app.root
            .resources
            .bigquery_schema_problem()
            .map(|problem| problem.message.clone()),
        Some(diagnostic.headline()),
        "and so must the problem line the empty view paints, which is the \
         same sentence in the third place it appears"
    );
}

/// An answered read clears the status line the previous refusal left.
///
/// Without the clear, a schema that loaded second-time-around paints its rows
/// under a status line still saying the read was denied — the same
/// contradiction `set_bigquery_schema` clears the *problem* to avoid, on the
/// other of the two surfaces. Reachable through `r`, which re-issues the
/// `show` for the table on screen.
#[test]
fn a_landed_schema_clears_the_status_line_a_refusal_left() {
    let (mut app, generation) = drilled_into_table("sales", vec![table("sales", "orders")]);
    land_refusal(&mut app, generation);
    assert!(
        app.root.system.status_message.is_some(),
        "the refusal must have left something for the success to clear, or \
         this test pins nothing"
    );

    land_schema(&mut app, generation, vec![field("order_id")]);

    assert_eq!(
        app.root.system.status_message, None,
        "columns on screen under a status line saying the read was denied is \
         the app contradicting itself"
    );
    assert_eq!(
        app.root.system.status_operation_label, None,
        "and the label goes with the message it labelled"
    );
}

/// Neither guard's *else* branch touches the status line either.
///
/// `clear_status_message` sits inside the context-and-generation guard, so an
/// answer for a table the operator has left must not wipe a status line that
/// is about something else. Hoisting it out of the guard is the edit this
/// catches, and every existing out-of-scope test asserts only on the schema
/// state the hoisted call would leave untouched.
#[test]
fn an_out_of_scope_answer_leaves_the_status_line_alone() {
    let (mut app, stale) = drilled_into_table("sales", vec![table("sales", "orders")]);
    escape(&mut app);
    let live = schema_generation(&enter(&mut app));
    assert_ne!(stale, live);
    land_refusal(&mut app, live);
    let status = app.root.system.status_message.clone();

    land_schema(&mut app, stale, vec![field("order_id")]);

    assert_eq!(
        app.root.system.status_message, status,
        "an answer from a drill the operator has left must not clear a status \
         line reporting the drill they are in"
    );
}
