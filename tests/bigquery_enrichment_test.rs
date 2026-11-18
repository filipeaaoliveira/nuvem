//! The per-table enrichment fan-out: one `bq show` per listed row, issued
//! once the dataset's listing lands, merged onto the row it was asked about.
//!
//! `tests/bigquery_drilldown_test.rs` is about the listing call and the drill
//! it belongs to; this file is about the N calls that follow it. Three
//! properties are pinned here that nothing else pins:
//!
//! - **The fan-out is exactly the listing.** One call per listed row, for that
//!   row's id, and none for anything else.
//! - **A partial failure stays partial.** A refused `show` leaves that row's
//!   three stats unavailable and touches no other row and no other column,
//!   while a failed `ls` still fails the whole load. The two calls behind this
//!   table have different policies on purpose, and the way that regresses is a
//!   later edit giving them one.
//! - **No result outlives the listing it was fanned out from.** Leaving the
//!   dataset lands none of the shows in flight, and neither does a show issued
//!   before an `r` retry. It takes both halves of the identity to say that:
//!   see `an_enrichment_for_a_drill_that_was_left_is_out_of_scope` for the
//!   half the drill generation answers, and
//!   `a_show_issued_before_a_retry_does_not_land` for the half only the
//!   listing epoch can.

use std::sync::{Arc, Mutex};

use nuvem::app::reducer::root::reduce;
use nuvem::app::state::resources::{BigQueryDrillGeneration, BigQueryListingEpoch, BigQueryView};
use nuvem::app::{Action, AppState, Effect, EffectResult, ResourcesAction};
use nuvem::services::gcloud::cli::{CommandError, CommandOutput, CommandRunner, GcloudCommand};
use nuvem::services::runtime::session::RuntimeSession;
use nuvem_domain::gcp::configuration::{
    GcloudConfiguration, GcloudConfigurationName, GcloudContext,
};
use nuvem_domain::gcp::resource::{
    BigQueryDataset, BigQueryTable, BigQueryTableKind, BigQueryTableStats, ResourceServiceKind,
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
        created: Some(1_495_683_814_344),
        labels: Vec::new(),
        rows: TableStat::Unavailable,
        bytes: TableStat::Unavailable,
        last_modified: TableStat::Unavailable,
    }
}

fn stats(rows: i64, bytes: i64, last_modified: i64) -> BigQueryTableStats {
    BigQueryTableStats {
        rows: TableStat::Known(rows),
        bytes: TableStat::Known(bytes),
        last_modified: TableStat::Known(last_modified),
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

fn refresh(app: &mut AppState) -> Vec<Effect> {
    reduce(app, &Action::Resources(ResourcesAction::Refresh))
}

/// The generation the tables listing among `effects` was issued under, taken
/// from the effect rather than from the live counter for the reason
/// `tests/bigquery_drilldown_test.rs` records: it is the only way to hold an
/// earlier drill's identity after the view has moved on.
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

/// Every `bq show` the effects ask for, as `(dataset_id, table_id)`.
fn show_targets(effects: &[Effect]) -> Vec<(String, String)> {
    effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::LoadBigQueryTableStats {
                dataset_id,
                table_id,
                ..
            } => Some((dataset_id.clone(), table_id.clone())),
            _ => None,
        })
        .collect()
}

/// The identity one fan-out was issued under: which drill, and which listing
/// call inside it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct FanOutScope {
    generation: BigQueryDrillGeneration,
    listing: BigQueryListingEpoch,
}

/// The scope one fan-out was issued under. Every show of one fan-out carries
/// the same one, because one landed listing is one fan-out.
fn fan_out_scope(effects: &[Effect]) -> FanOutScope {
    let scopes: Vec<FanOutScope> = effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::LoadBigQueryTableStats {
                generation,
                listing,
                ..
            } => Some(FanOutScope {
                generation: *generation,
                listing: *listing,
            }),
            _ => None,
        })
        .collect();
    let first = *scopes.first().expect("the fan-out must have been issued");
    assert!(
        scopes.iter().all(|scope| *scope == first),
        "one landed listing is one fan-out, so its calls share one identity: \
         {scopes:?}"
    );
    first
}

/// Drills into `dataset_id`, lands `tables` in it, and returns the app beside
/// the effects the landing produced — which is where the fan-out is.
fn drilled_in_and_listed(dataset_id: &str, tables: Vec<BigQueryTable>) -> (AppState, Vec<Effect>) {
    let mut app = app_with_datasets(vec![dataset(dataset_id)]);
    let entering = enter(&mut app);
    let context = app.root.current_context().expect("an active context");
    let landed = reduce(
        &mut app,
        &Action::EffectResult(EffectResult::BigQueryTablesLoaded {
            context,
            generation: listing_generation(&entering),
            listing: listing_epoch(&entering),
            tables,
        }),
    );
    (app, landed)
}

/// Lands one table's answered stats, as the fan-out of `scope` asked for.
fn land_stats(app: &mut AppState, table_id: &str, scope: FanOutScope, stats: BigQueryTableStats) {
    let action = Action::EffectResult(EffectResult::BigQueryTableStatsLoaded {
        context: app.root.current_context().expect("an active context"),
        table_id: table_id.to_string(),
        generation: scope.generation,
        listing: scope.listing,
        stats,
    });
    reduce(app, &action);
}

/// Lands one table's refused `show`, as the fan-out of `scope` asked for.
fn land_refusal(app: &mut AppState, table_id: &str, scope: FanOutScope) {
    let action = Action::EffectResult(EffectResult::BigQueryTableStatsLoadFailed {
        context: app.root.current_context().expect("an active context"),
        table_id: table_id.to_string(),
        generation: scope.generation,
        listing: scope.listing,
        error: nuvem::app::AppError::gcloud("permission denied"),
    });
    reduce(app, &action);
}

/// A context a result can carry that is not the one the app is on. Same drill
/// identity, different gcloud configuration: the generation and the listing
/// epoch are per-app counters, so a result from another context can carry
/// numbers that match the live fan-out's exactly, and the context is then the
/// only thing telling the two apart.
fn other_context() -> GcloudContext {
    GcloudContext {
        configuration: GcloudConfigurationName::new("prod"),
        account: None,
        project: Some("prod-project".to_string()),
        region: None,
        zone: None,
    }
}

/// Lands one table's answered stats under `context` rather than the live one.
fn land_stats_from(
    app: &mut AppState,
    context: GcloudContext,
    table_id: &str,
    scope: FanOutScope,
    stats: BigQueryTableStats,
) {
    let action = Action::EffectResult(EffectResult::BigQueryTableStatsLoaded {
        context,
        table_id: table_id.to_string(),
        generation: scope.generation,
        listing: scope.listing,
        stats,
    });
    reduce(app, &action);
}

/// Lands one table's refused `show` under `context` rather than the live one.
fn land_refusal_from(
    app: &mut AppState,
    context: GcloudContext,
    table_id: &str,
    scope: FanOutScope,
) {
    let action = Action::EffectResult(EffectResult::BigQueryTableStatsLoadFailed {
        context,
        table_id: table_id.to_string(),
        generation: scope.generation,
        listing: scope.listing,
        error: nuvem::app::AppError::gcloud("permission denied"),
    });
    reduce(app, &action);
}

fn row<'a>(app: &'a AppState, table_id: &str) -> &'a BigQueryTable {
    app.root
        .resources
        .bigquery_tables()
        .iter()
        .find(|row| row.table_id == table_id)
        .unwrap_or_else(|| panic!("no row for {table_id}"))
}

// --- The fan-out is exactly the listing ---

#[test]
fn a_landed_listing_asks_for_every_row_it_listed_and_nothing_else() {
    let (_app, landed) = drilled_in_and_listed(
        "sales",
        vec![
            table("sales", "orders"),
            table("sales", "returns"),
            table("sales", "customers"),
        ],
    );

    assert_eq!(
        show_targets(&landed),
        vec![
            ("sales".to_string(), "orders".to_string()),
            ("sales".to_string(), "returns".to_string()),
            ("sales".to_string(), "customers".to_string()),
        ],
        "three listed tables are three shows, one per row, each for its own \
         id: a fan-out that skips a row leaves it permanently unavailable, and \
         one that asks twice pays twice"
    );
}

/// A filter narrows what is painted, never what is asked for. The rows it
/// hides are still listed rows, and a fan-out over the visible ones would
/// leave them permanently unavailable — visibly so the moment the operator
/// clears the filter, with no call in flight to fill them.
#[test]
fn a_filter_over_the_table_does_not_narrow_the_fan_out() {
    let mut app = app_with_datasets(vec![dataset("sales")]);
    let entering = enter(&mut app);
    reduce(&mut app, &Action::Resources(ResourcesAction::StartFilter));
    for character in "orders".chars() {
        reduce(
            &mut app,
            &Action::Resources(ResourcesAction::AppendFilterChar(character)),
        );
    }
    reduce(&mut app, &Action::Resources(ResourcesAction::ApplyFilter));

    let context = app.root.current_context().expect("an active context");
    let landed = reduce(
        &mut app,
        &Action::EffectResult(EffectResult::BigQueryTablesLoaded {
            context,
            generation: listing_generation(&entering),
            listing: listing_epoch(&entering),
            tables: vec![
                table("sales", "orders"),
                table("sales", "returns"),
                table("sales", "customers"),
            ],
        }),
    );

    assert_eq!(
        app.root.resources.visible_bigquery_table_count(),
        1,
        "the filter must actually be narrowing, or this test proves nothing"
    );
    assert_eq!(
        show_targets(&landed).len(),
        3,
        "every listed row is asked about, filtered or not: {landed:?}"
    );
}

#[test]
fn a_listing_with_no_rows_asks_for_nothing() {
    let (_app, landed) = drilled_in_and_listed("sales", Vec::new());

    assert!(
        show_targets(&landed).is_empty(),
        "an empty dataset has nothing to enrich: {landed:?}"
    );
}

// --- Each answer lands on the row it was about ---

#[test]
fn each_result_merges_into_its_own_row() {
    let (mut app, landed) = drilled_in_and_listed(
        "sales",
        vec![
            table("sales", "orders"),
            table("sales", "returns"),
            table("sales", "customers"),
        ],
    );
    let listing = fan_out_scope(&landed);

    land_stats(
        &mut app,
        "orders",
        listing,
        stats(101, 15_283, 1_787_306_613_694),
    );
    land_stats(
        &mut app,
        "returns",
        listing,
        stats(7, 512, 1_787_306_613_000),
    );

    assert_eq!(row(&app, "orders").rows, TableStat::Known(101));
    assert_eq!(row(&app, "orders").bytes, TableStat::Known(15_283));
    assert_eq!(
        row(&app, "orders").last_modified,
        TableStat::Known(1_787_306_613_694)
    );
    assert_eq!(row(&app, "returns").rows, TableStat::Known(7));
    assert_eq!(row(&app, "returns").bytes, TableStat::Known(512));
    assert_eq!(
        row(&app, "returns").last_modified,
        TableStat::Known(1_787_306_613_000)
    );
    assert_eq!(
        row(&app, "customers").rows,
        TableStat::Unavailable,
        "the row whose show has not answered yet holds no other row's numbers"
    );
}

#[test]
fn a_result_for_a_table_the_listing_never_returned_is_dropped() {
    let (mut app, landed) = drilled_in_and_listed("sales", vec![table("sales", "orders")]);
    let listing = fan_out_scope(&landed);

    land_stats(&mut app, "ghost", listing, stats(9, 9, 9));

    assert_eq!(
        app.root.resources.bigquery_tables().len(),
        1,
        "an enrichment is an answer about a row, never a reason for one to \
         exist"
    );
}

// --- Both halves: a partial failure is partial, a total failure is total ---

/// `bigquery.tables.list` granted with `bigquery.tables.get` denied is an
/// ordinary read-only IAM shape, so a refused `show` is not a failed table.
/// It costs that row its three numbers and nothing else — not the row, not
/// its other columns, not its neighbours, not the pane.
#[test]
fn a_refused_show_costs_that_row_its_stats_and_nothing_else() {
    let (mut app, landed) = drilled_in_and_listed(
        "sales",
        vec![
            table("sales", "orders"),
            table("sales", "returns"),
            table("sales", "customers"),
        ],
    );
    let listing = fan_out_scope(&landed);

    land_stats(
        &mut app,
        "orders",
        listing,
        stats(101, 15_283, 1_787_306_613_694),
    );
    land_refusal(&mut app, "returns", listing);

    let refused = row(&app, "returns");
    assert_eq!(
        (refused.rows, refused.bytes, refused.last_modified),
        (
            TableStat::Unavailable,
            TableStat::Unavailable,
            TableStat::Unavailable
        ),
        "a refused show must leave all three unavailable — never a zero, \
         which claims the table is empty"
    );
    assert_eq!(
        refused.kind,
        BigQueryTableKind::Table,
        "the columns the listing filled are still the listing's"
    );
    assert_eq!(refused.created, Some(1_495_683_814_344));

    assert_eq!(
        row(&app, "orders").rows,
        TableStat::Known(101),
        "one refused show must not undo another row's answer"
    );
    assert_eq!(
        app.root.resources.bigquery_tables().len(),
        3,
        "and it must not drop a row"
    );
    assert!(
        app.root.resources.bigquery_tables_problem().is_none(),
        "a refused per-row show is not the table's problem: painting one would \
         replace three listed rows with an error"
    );
    assert_eq!(
        app.root.resources.bigquery_view(),
        BigQueryView::Tables,
        "and it must not eject the operator from the dataset"
    );
}

/// The other half. A failed `ls` has no partial answer to keep: an empty
/// table would claim the dataset has no tables, which a denied permission
/// cannot support.
#[test]
fn a_failed_listing_still_fails_the_whole_load_and_fans_out_nothing() {
    let mut app = app_with_datasets(vec![dataset("sales")]);
    let entering = enter(&mut app);
    let context = app.root.current_context().expect("an active context");

    let effects = reduce(
        &mut app,
        &Action::EffectResult(EffectResult::BigQueryTablesLoadFailed {
            context,
            generation: listing_generation(&entering),
            listing: listing_epoch(&entering),
            error: nuvem::app::AppError::gcloud("permission denied"),
        }),
    );

    assert!(
        app.root.resources.bigquery_tables_problem().is_some(),
        "a failed listing is the table's problem, unlike a failed show"
    );
    assert!(
        app.root.resources.bigquery_tables().is_empty(),
        "and it keeps no rows"
    );
    assert!(
        show_targets(&effects).is_empty(),
        "with no rows listed there is nothing to enrich: {effects:?}"
    );
}

// --- Cancellation: a result outlives neither its drill nor its listing ---

/// Leaving the drill takes its fan-out out of scope — and the guard is what
/// has to say so.
///
/// Asserting only that the rows are empty afterwards is not a test of
/// cancellation: it passes with the scope check replaced by `true`, because
/// `set_view` empties `tables.rows` and an accepted enrichment then finds no
/// row to merge into. That makes "the rows are empty" a fact about `set_view`
/// and about nothing else, and it is the reason the drill generation looked
/// unkillable here. So this asks
/// `ResourcesState::matches_bigquery_enrichment_scope` directly: after `Esc`
/// the abandoned fan-out's identity must be refused, whatever the rows
/// happen to be. Delete the generation conjunct and this fails, because `Esc`
/// leaves the listing epoch exactly as it was.
#[test]
fn an_enrichment_for_a_drill_that_was_left_is_out_of_scope() {
    let (mut app, landed) = drilled_in_and_listed("sales", vec![table("sales", "orders")]);
    let abandoned = fan_out_scope(&landed);

    assert!(
        app.root
            .resources
            .matches_bigquery_enrichment_scope(abandoned.generation, abandoned.listing),
        "the fan-out of the listing on screen is in scope, or the assertion \
         below is testing nothing"
    );

    escape(&mut app);

    assert!(
        !app.root
            .resources
            .matches_bigquery_enrichment_scope(abandoned.generation, abandoned.listing),
        "the drill those shows were issued for is gone, so their answers are \
         out of scope — the epoch alone cannot say that, because `Esc` mints \
         no listing and so leaves the epoch live"
    );

    land_stats(&mut app, "orders", abandoned, stats(101, 15_283, 1));

    assert!(
        app.root.resources.bigquery_tables().is_empty(),
        "leaving drops the drill's rows, and an enrichment must not resurrect \
         one"
    );
}

/// The same dataset entered twice is two drills, and the first drill's shows
/// belong to the first. The table ids are identical across the two listings,
/// so the setter's id match cannot tell them apart.
///
/// Either counter drops it: leaving and re-entering bumps the generation
/// twice and mints a fresh epoch, so this pins the guard as a whole rather
/// than either conjunct. `an_enrichment_for_a_drill_that_was_left_is_out_of_scope`
/// is what isolates the generation.
#[test]
fn a_show_from_an_earlier_drill_into_the_same_dataset_does_not_land() {
    let (mut app, first) = drilled_in_and_listed("sales", vec![table("sales", "orders")]);
    let abandoned = fan_out_scope(&first);

    escape(&mut app);
    let entering = enter(&mut app);
    let context = app.root.current_context().expect("an active context");
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::BigQueryTablesLoaded {
            context,
            generation: listing_generation(&entering),
            listing: listing_epoch(&entering),
            tables: vec![table("sales", "orders")],
        }),
    );

    land_stats(&mut app, "orders", abandoned, stats(101, 15_283, 1));

    assert_eq!(
        row(&app, "orders").rows,
        TableStat::Unavailable,
        "the first drill's answer must not fill the second drill's row"
    );
}

/// The identity mechanism this task added, and the reason it had to.
///
/// A generation identifies a *drill*: `r` re-lists the same dataset inside
/// the drill it is already in and deliberately does not bump it. So a show
/// issued before the retry still matches the generation when it returns, and
/// the setter's id match does not stop it — `orders` is `orders` in both
/// listings. The listing epoch is what separates the two calls: it is bumped
/// by every listing request, entry and retry alike, and a show carries the
/// epoch of the listing it fanned out from.
#[test]
fn a_show_issued_before_a_retry_does_not_land() {
    let mut app = app_with_datasets(vec![dataset("sales")]);
    let entering = enter(&mut app);
    let context = app.root.current_context().expect("an active context");
    let first = reduce(
        &mut app,
        &Action::EffectResult(EffectResult::BigQueryTablesLoaded {
            context,
            generation: listing_generation(&entering),
            listing: listing_epoch(&entering),
            tables: vec![table("sales", "orders")],
        }),
    );
    let before_retry = fan_out_scope(&first);

    let retry = refresh(&mut app);
    let context = app.root.current_context().expect("an active context");
    let after = reduce(
        &mut app,
        &Action::EffectResult(EffectResult::BigQueryTablesLoaded {
            context,
            generation: listing_generation(&retry),
            listing: listing_epoch(&retry),
            tables: vec![table("sales", "orders")],
        }),
    );
    let after_retry = fan_out_scope(&after);

    assert_eq!(
        listing_generation(&entering),
        listing_generation(&retry),
        "a retry stays inside the drill it retries, so both listings are \
         issued under one generation — which is why the generation cannot be \
         what separates the two fan-outs"
    );
    assert_ne!(
        before_retry.listing, after_retry.listing,
        "a second listing is a second epoch: without that the two fan-outs \
         are indistinguishable"
    );

    land_stats(&mut app, "orders", before_retry, stats(101, 15_283, 1));
    assert_eq!(
        row(&app, "orders").rows,
        TableStat::Unavailable,
        "the discarded listing's show must not fill the retry's row"
    );

    land_stats(&mut app, "orders", after_retry, stats(7, 512, 2));
    assert_eq!(
        row(&app, "orders").rows,
        TableStat::Known(7),
        "and the retry's own show must land, or `r` would leave the table \
         permanently blank"
    );
}

/// `r` pressed while the first `bq ls` is still in flight, which is the case
/// the epoch on the *listing* exists for.
///
/// Both listings are issued under one generation — a retry stays inside its
/// drill — so with the generation as the listing's whole identity both landed:
/// the discarded call installed its rows and fanned out over them, and because
/// each fan-out took the identity its listing was accepted under, the
/// abandoned shows carried the live epoch and merged into the retry's rows.
/// The dataset also paid two full fan-outs. The epoch on
/// `BigQueryTablesLoaded` is what refuses the first listing outright.
#[test]
fn a_listing_replaced_by_a_retry_before_it_landed_does_not_land() {
    let mut app = app_with_datasets(vec![dataset("sales")]);
    let entering = enter(&mut app);
    // No listing has landed yet: this is `r` over a table that is still
    // loading, which is exactly what an operator does when it is slow.
    let retry = refresh(&mut app);

    assert_eq!(
        listing_generation(&entering),
        listing_generation(&retry),
        "a retry stays inside the drill it retries, so the generation cannot \
         be what separates the two listings"
    );
    assert_ne!(
        listing_epoch(&entering),
        listing_epoch(&retry),
        "each listing request mints its own epoch, or this test proves nothing"
    );

    let context = app.root.current_context().expect("an active context");
    let superseded = reduce(
        &mut app,
        &Action::EffectResult(EffectResult::BigQueryTablesLoaded {
            context: context.clone(),
            generation: listing_generation(&entering),
            listing: listing_epoch(&entering),
            tables: vec![table("sales", "orders")],
        }),
    );

    assert!(
        app.root.resources.bigquery_tables().is_empty(),
        "the operator asked for this call to be discarded, so its rows are \
         not the ones to paint"
    );
    assert!(
        show_targets(&superseded).is_empty(),
        "and a discarded listing fans out over nothing: it would double the \
         call count and its answers would land on the retry's rows: \
         {superseded:?}"
    );

    let landed = reduce(
        &mut app,
        &Action::EffectResult(EffectResult::BigQueryTablesLoaded {
            context,
            generation: listing_generation(&retry),
            listing: listing_epoch(&retry),
            tables: vec![table("sales", "orders")],
        }),
    );

    assert_eq!(
        app.root.resources.bigquery_tables().len(),
        1,
        "the retry's own listing still lands, or `r` would leave the table \
         empty forever"
    );
    let scope = fan_out_scope(&landed);
    assert_eq!(
        scope.listing,
        listing_epoch(&retry),
        "and its fan-out is stamped with the listing that produced the rows, \
         not with whichever listing happened to be live"
    );

    land_stats(&mut app, "orders", scope, stats(7, 512, 2));
    assert_eq!(row(&app, "orders").rows, TableStat::Known(7));
}

/// A refused show is gated exactly like a successful one. If it were not, a
/// stale failure would be a no-op today and a silent way to blank a row the
/// moment the failure arm learns to write anything.
#[test]
fn a_refused_show_from_an_earlier_listing_is_dropped_too() {
    let (mut app, first) = drilled_in_and_listed("sales", vec![table("sales", "orders")]);
    let before_retry = fan_out_scope(&first);

    let retry = refresh(&mut app);
    let context = app.root.current_context().expect("an active context");
    let after = reduce(
        &mut app,
        &Action::EffectResult(EffectResult::BigQueryTablesLoaded {
            context,
            generation: listing_generation(&retry),
            listing: listing_epoch(&retry),
            tables: vec![table("sales", "orders")],
        }),
    );
    let after_retry = fan_out_scope(&after);
    land_stats(&mut app, "orders", after_retry, stats(7, 512, 2));

    land_refusal(&mut app, "orders", before_retry);

    assert_eq!(
        row(&app, "orders").rows,
        TableStat::Known(7),
        "an abandoned listing's failure is not this listing's news"
    );
}

/// Both identities are checked on both arms, and the drill's is not one of
/// them: the generation and the listing epoch count this app's drills, so they
/// say nothing about which gcloud context the answer came back from. Switching
/// configuration is a whole different project's BigQuery, and its answers must
/// not be painted onto the rows now on screen.
///
/// The two tests below hold the drill identity fixed at the live fan-out's and
/// vary only the context, so the context check is the only thing that can drop
/// the result.
#[test]
fn a_show_answered_for_another_gcloud_context_does_not_land() {
    let (mut app, landed) = drilled_in_and_listed("sales", vec![table("sales", "orders")]);
    let scope = fan_out_scope(&landed);
    land_stats(&mut app, "orders", scope, stats(7, 512, 2));

    land_stats_from(
        &mut app,
        other_context(),
        "orders",
        scope,
        stats(999, 999, 999),
    );

    assert_eq!(
        row(&app, "orders").rows,
        TableStat::Known(7),
        "another project's table happens to be called sales.orders too, and \
         its row count is not this one's"
    );
    assert_eq!(
        row(&app, "orders").bytes,
        TableStat::Known(512),
        "the byte size is this context's answer for the same reason"
    );
}

/// A refusal from another context is dropped for the same reason a success is,
/// and it is the more damaging direction: the failure arm blanks three cells,
/// so a refusal that landed across contexts would erase numbers the live
/// context answered and leave the row reading `-` — a claim that *this*
/// project's `show` was denied, which it was not.
#[test]
fn a_refusal_from_another_gcloud_context_does_not_blank_a_row() {
    let (mut app, landed) = drilled_in_and_listed("sales", vec![table("sales", "orders")]);
    let scope = fan_out_scope(&landed);
    land_stats(&mut app, "orders", scope, stats(7, 512, 2));

    land_refusal_from(&mut app, other_context(), "orders", scope);

    assert_eq!(
        row(&app, "orders").rows,
        TableStat::Known(7),
        "a denial in another configuration is not news about this row"
    );
    assert_eq!(
        row(&app, "orders").bytes,
        TableStat::Known(512),
        "and it must not blank the cells this context filled"
    );
    assert_eq!(
        row(&app, "orders").last_modified,
        TableStat::Known(2),
        "all three cells, because the arm writes all three"
    );
}

// --- Through the session, where the calls actually go out ---

/// A runner that answers by argv rather than by call order, because the fan-out
/// runs its calls concurrently and their order is not fixed.
struct BqRunner {
    commands: Mutex<Vec<Vec<String>>>,
}

impl CommandRunner for BqRunner {
    fn run(&self, command: GcloudCommand) -> Result<CommandOutput, CommandError> {
        let args = command.args.clone();
        self.commands
            .lock()
            .expect("commands mutex poisoned")
            .push(args.clone());
        let stdout = if args.iter().any(|arg| arg == "ls") {
            include_str!("fixtures/bigquery_tables.json").to_string()
        } else if args.iter().any(|arg| arg.ends_with(".bikeshare_stations")) {
            show_json(101, 15_283)
        } else if args.iter().any(|arg| arg.ends_with(".bikeshare_trips")) {
            show_json(72, 4_096)
        } else {
            panic!("unexpected bq argv: {args:?}")
        };
        Ok(CommandOutput {
            stdout,
            stderr: String::new(),
        })
    }
}

/// A `bq show` response with the two counts this test tells the rows apart by.
/// Both are wire strings, as BigQuery sends int64.
fn show_json(rows: i64, bytes: i64) -> String {
    format!(
        r#"{{"numRows": "{rows}", "numBytes": "{bytes}", "lastModifiedTime": "1787306613694"}}"#
    )
}

/// The whole path in one test: entering a dataset lists its tables, the listing
/// fans out into one `bq show` per row, and each answer lands on its own row.
///
/// The reducer-level tests above cannot see this, because the effects they
/// assert on are never executed. What this adds is that the session actually
/// issues the calls — through the background dispatch, so the run loop is free
/// while they run — and that the argv each one carries is a `show` for that
/// row's own table.
#[tokio::test]
async fn the_session_issues_one_show_per_listed_table_and_merges_each_answer() {
    let runner = Arc::new(BqRunner {
        commands: Mutex::new(Vec::new()),
    });
    let mut session = RuntimeSession::new_with_runner_for_test(
        AppState::new(),
        Box::new(SharedBqRunner(runner.clone())),
    );
    session
        .apply_effect_result(EffectResult::ConfigurationsLoaded {
            configurations: vec![GcloudConfiguration {
                name: GcloudConfigurationName::new("dev"),
                is_active: true,
                account: None,
                project: Some("dev-project".to_string()),
                region: None,
                zone: None,
            }],
        })
        .await;
    session
        .dispatch_action(Action::Resources(ResourcesAction::SelectService(
            ResourceServiceKind::BigQueryDatasets,
        )))
        .await;
    let context = session
        .app()
        .root
        .current_context()
        .expect("an active context");
    session
        .apply_effect_result(EffectResult::BigQueryDatasetsLoaded {
            context,
            datasets: vec![dataset("austin_bikeshare")],
        })
        .await;

    session
        .dispatch_action(Action::Resources(ResourcesAction::OpenSelected))
        .await;
    // Three queued results: the datasets refresh that selecting the service
    // asked for, and one per fanned-out show. The shows are answered off the
    // run loop, so each arrives on the result channel rather than inline with
    // the listing, and the order the three arrive in is not fixed.
    //
    // Each wait is bounded because `process_next_effect_result` awaits: a
    // change that queues one result fewer would otherwise hang here until
    // nextest's `terminate-after` killed the whole test binary, which reports
    // as a timeout on whichever test was slowest rather than as this one
    // waiting for a fourth result that is never coming.
    for index in 0..3 {
        let next = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            session.process_next_effect_result(),
        )
        .await
        .unwrap_or_else(|_| {
            panic!("no queued effect result {index} arrived: the listing and its two shows are three, so a fourth wait means one of them was never queued")
        });
        assert!(next);
    }

    let rows = session.app().root.resources.bigquery_tables();
    assert_eq!(rows.len(), 2, "the listing's rows: {rows:?}");
    let stations = rows
        .iter()
        .find(|row| row.table_id == "bikeshare_stations")
        .expect("the listed stations table");
    let trips = rows
        .iter()
        .find(|row| row.table_id == "bikeshare_trips")
        .expect("the listed trips table");
    assert_eq!(
        (stations.rows, stations.bytes),
        (TableStat::Known(101), TableStat::Known(15_283)),
        "each row keeps the answer to its own show"
    );
    assert_eq!(
        (trips.rows, trips.bytes),
        (TableStat::Known(72), TableStat::Known(4_096)),
    );

    let commands = runner.commands.lock().expect("commands mutex poisoned");
    let shows: Vec<&Vec<String>> = commands
        .iter()
        .filter(|args| args.iter().any(|arg| arg == "show"))
        .collect();
    assert_eq!(
        shows.len(),
        2,
        "two listed tables are two shows, no more: {commands:?}"
    );
    // As a set: the calls run concurrently, so which one the runner recorded
    // first is not a property worth pinning.
    let mut targets: Vec<&String> = shows
        .iter()
        .map(|args| args.last().expect("a show names a table"))
        .collect();
    targets.sort();
    assert_eq!(
        targets,
        vec![
            "dev-project:austin_bikeshare.bikeshare_stations",
            "dev-project:austin_bikeshare.bikeshare_trips",
        ],
        "each show names its own table, fully qualified: {commands:?}"
    );
}

/// Shares one runner between the session and the test's assertions.
struct SharedBqRunner(Arc<BqRunner>);

impl CommandRunner for SharedBqRunner {
    fn run(&self, command: GcloudCommand) -> Result<CommandOutput, CommandError> {
        self.0.run(command)
    }
}
