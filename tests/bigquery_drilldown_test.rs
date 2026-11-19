//! The BigQuery drill-downs: what `Enter` on a dataset, `Enter` on a table and
//! `Esc` inside either do to the view, the drill scope, and the call that level
//! issues.
//!
//! `tests/bigquery_state_test.rs` is about the datasets table itself and
//! `tests/bigquery_service_test.rs` about the `bq` argv. What this file is
//! about is the transitions between the lists — that entering records *which*
//! dataset or table and asks for exactly its contents, that leaving drops what
//! was loaded below the level arrived at and nothing above it, and that each
//! list keeps its own cursor and filter.

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
use nuvem::app::reducer::root::reduce;
use nuvem::app::state::resources::{BigQueryDrillGeneration, BigQueryListingEpoch, BigQueryView};
use nuvem::app::{Action, AppState, Effect, EffectResult, NavAction, ResourcesAction, Workspace};
use nuvem::input::Context;
use nuvem::input::keymap::action_for_key;
use nuvem_domain::gcp::configuration::{GcloudConfiguration, GcloudConfigurationName};
use nuvem_domain::gcp::resource::{
    BigQuerySchemaField, BigQueryTable, BigQueryTableKind, ResourceServiceKind, TableStat,
};

use crate::support::keymap_context::base_context;

use nuvem_domain::gcp::resource::BigQueryDataset;

fn dataset(dataset_id: &str) -> BigQueryDataset {
    BigQueryDataset {
        dataset_id: dataset_id.to_string(),
        project_id: "dev-project".to_string(),
        location: "US".to_string(),
        friendly_name: None,
        labels: Vec::new(),
    }
}

pub(crate) fn table(dataset_id: &str, table_id: &str) -> BigQueryTable {
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

/// An app on the BigQuery datasets table with `datasets` loaded.
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

/// The generation the tables listing among `effects` was issued under.
///
/// Tests take it from the effect rather than reading the live counter: the
/// effect is where a result gets its generation in the running app, and it is
/// the only way to hold on to an *earlier* drill's after the view has moved
/// on.
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

pub(crate) fn schema_field(path: &str) -> BigQuerySchemaField {
    BigQuerySchemaField {
        path: path.to_string(),
        field_type: "STRING".to_string(),
        mode: "NULLABLE".to_string(),
        description: None,
    }
}

/// Drills into `dataset_id` and lands `tables` in it.
pub(crate) fn drilled_in(dataset_id: &str, tables: Vec<BigQueryTable>) -> AppState {
    let mut app = app_with_datasets(vec![dataset(dataset_id)]);
    let effects = enter(&mut app);
    let context = app.root.current_context().expect("an active context");
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::BigQueryTablesLoaded {
            context,
            generation: listing_generation(&effects),
            listing: listing_epoch(&effects),
            tables,
        }),
    );
    app
}

// --- Entering ---

#[test]
fn enter_on_a_selected_dataset_shows_its_tables_and_asks_for_exactly_them() {
    // `analytics` sorts first, so it is the row the cursor starts on.
    let mut app = app_with_datasets(vec![dataset("sales"), dataset("analytics")]);

    let effects = enter(&mut app);

    assert_eq!(app.root.resources.bigquery_view(), BigQueryView::Tables);
    assert_eq!(
        app.root
            .resources
            .bigquery_tables_dataset()
            .map(|dataset| dataset.dataset_id.as_str()),
        Some("analytics"),
        "the view must record which dataset it is inside, or a landing \
         listing has no scope to be checked against"
    );
    match effects.as_slice() {
        [Effect::LoadBigQueryTables { dataset_id, .. }] => assert_eq!(
            dataset_id, "analytics",
            "the listing must name the dataset the cursor was on, not another"
        ),
        other => panic!("expected one tables listing, got {other:?}"),
    }
}

/// The empty-list case, which is not the same as "the first row".
///
/// `selected_bigquery_dataset` returns `None` when nothing is visible, and an
/// `open_selected_bigquery_dataset` that transitioned first and resolved the
/// row afterwards would show a tables view with no dataset behind it.
#[test]
fn enter_with_nothing_selected_does_nothing_at_all() {
    let mut app = app_with_datasets(Vec::new());

    let effects = enter(&mut app);

    assert_eq!(
        app.root.resources.bigquery_view(),
        BigQueryView::Datasets,
        "an Enter that resolved no dataset must not move the view"
    );
    assert!(
        app.root.resources.bigquery_tables_dataset().is_none(),
        "and must record no drill scope"
    );
    assert!(
        effects.is_empty(),
        "and must issue no listing call: {effects:?}"
    );
}

/// `Enter` inside the tables view descends one more level, into the selected
/// table's columns — it used to open nothing, because a table has no detail
/// pane.
///
/// What is asserted here is that the second `Enter` is *not* read as a second
/// `Enter` on the dataset row still selected behind the drill: the view moves
/// to the schema, and the tables already listed stay listed. The schema call
/// itself, and everything its answer does, is
/// `tests/bigquery_schema_test.rs`.
#[test]
fn enter_inside_the_tables_view_descends_into_the_selected_table() {
    let mut app = drilled_in("sales", vec![table("sales", "orders")]);

    let effects = enter(&mut app);

    assert_eq!(app.root.resources.bigquery_view(), BigQueryView::Schema);
    assert!(
        !effects
            .iter()
            .any(|effect| matches!(effect, Effect::LoadBigQueryTables { .. })),
        "the dataset row behind the drill must not be re-entered: {effects:?}"
    );
    assert_eq!(
        app.root.resources.bigquery_tables().len(),
        1,
        "and the tables already listed must survive the descent"
    );
}

// --- Leaving ---

#[test]
fn esc_returns_to_the_datasets_list() {
    let mut app = drilled_in("sales", vec![table("sales", "orders")]);

    let effects = escape(&mut app);

    assert_eq!(app.root.resources.bigquery_view(), BigQueryView::Datasets);
    assert!(
        effects.is_empty(),
        "leaving a drill-down reloads nothing: {effects:?}"
    );
}

/// The invariant `BigQueryDrilldownState::set_view` exists to hold, and the
/// one no mutation operator reaches: assigning the view field directly in this
/// transition leaves the previous dataset's tables loaded.
///
/// Without it, `Esc` then `Enter` on a different dataset paints `sales`'s
/// tables under `logs`'s header until the fresh listing returns.
#[test]
fn leaving_a_dataset_drops_the_tables_that_were_listed_inside_it() {
    let mut app = drilled_in("sales", vec![table("sales", "orders")]);

    escape(&mut app);

    assert!(
        app.root.resources.bigquery_tables().is_empty(),
        "the rows listed for the dataset just left must not survive it"
    );
    assert!(
        app.root.resources.bigquery_tables_dataset().is_none(),
        "and neither must the drill scope naming that dataset"
    );
}

#[test]
fn esc_on_the_datasets_list_leaves_the_view_where_it_is() {
    let mut app = app_with_datasets(vec![dataset("sales")]);

    escape(&mut app);

    assert_eq!(app.root.resources.bigquery_view(), BigQueryView::Datasets);
}

// --- The key that reaches it ---

fn press(code: KeyCode) -> KeyEvent {
    KeyEvent {
        code,
        modifiers: KeyModifiers::empty(),
        kind: KeyEventKind::Press,
        state: KeyEventState::empty(),
    }
}

/// The resources workspace with the *tables* drill open or closed.
///
/// `resources_drilldown_opens_another_level` moves with the flag rather than
/// being a second parameter, because every test below that passes `true` is
/// about the tables view — the level that has the schema under it. The schema
/// view is the other combination and needs the real derivation:
/// [`drilled_context`] builds it from live state rather than from a literal,
/// so a fixture cannot claim a depth the state does not have.
fn resources_context(drilldown_open: bool) -> Context {
    Context {
        workspace: Workspace::Resources,
        resources_drilldown_open: drilldown_open,
        resources_drilldown_opens_another_level: drilldown_open,
        ..base_context()
    }
}

/// The keymap context for whatever drill `app` actually has on screen, read
/// through the same two methods `input_context` reads.
fn drilled_context(app: &AppState) -> Context {
    Context {
        workspace: Workspace::Resources,
        resources_drilldown_open: app.root.resources.is_showing_drilldown(),
        resources_drilldown_opens_another_level: app.root.resources.drilldown_opens_another_level(),
        ..base_context()
    }
}

#[test]
fn esc_leaves_the_drilldown_rather_than_the_workspace() {
    assert_eq!(
        action_for_key(press(KeyCode::Esc), &resources_context(true)),
        Some(Action::Resources(ResourcesAction::LeaveDrillDown)),
        "inside a drill-down, Esc's back is the list it was entered from"
    );
}

/// The keys the drill-down refuses, in both directions.
///
/// `s` and `←`/`→` sort and walk the columns of the datasets collection
/// *behind* the drill, which nothing on screen shows moving. Refusing them at
/// the keymap is what lets the help catalogue and the frame footer drop them
/// without lying by omission — and it is what keeps
/// `keymap_help_parity_test` meaningful here, since that sweep pairs
/// catalogue entries with dispatch.
///
/// `Enter` was a third refusal and is not one any more; see
/// `enter_inside_the_drilldown_opens_the_selected_tables_schema` below.
#[test]
fn the_drilldown_refuses_the_keys_that_would_move_the_list_behind_it() {
    for code in [KeyCode::Char('s'), KeyCode::Left, KeyCode::Right] {
        assert_eq!(
            action_for_key(press(code), &resources_context(true)),
            None,
            "{code:?} acts on the datasets collection behind the drill, so it \
             must dispatch nothing while the tables view is on screen"
        );
        assert!(
            action_for_key(press(code), &resources_context(false)).is_some(),
            "{code:?} must still work on the list itself, or the refusal above \
             is a deletion rather than a guard"
        );
    }

    // `w` is the counter-example that keeps the three above from reading as
    // "the drill-down refuses everything": wide mode is about the table that
    // *is* on screen, so it keeps working.
    assert_eq!(
        action_for_key(press(KeyCode::Char('w')), &resources_context(true)),
        action_for_key(press(KeyCode::Char('w')), &resources_context(false)),
    );
}

/// The key that reaches the third level at all.
///
/// `Enter` inside the drill used to dispatch nothing, because a table row
/// opened nothing. It opens that table's column schema now, so the guard is
/// gone — and with it gone this is the only thing standing between the schema
/// level and being state no keypress can reach. Restore the `!drilldown`
/// guard on the `Enter` arm of `resource_content_key` and this fails.
#[test]
fn enter_inside_the_drilldown_opens_the_selected_tables_schema() {
    assert_eq!(
        action_for_key(press(KeyCode::Enter), &resources_context(true)),
        Some(Action::Resources(ResourcesAction::OpenSelected)),
        "inside the tables drill, Enter descends into the selected table"
    );
    assert_eq!(
        action_for_key(press(KeyCode::Enter), &resources_context(false)),
        Some(Action::Resources(ResourcesAction::OpenSelected)),
        "and it still opens the row on the list above"
    );
}

/// The guard is load-bearing in the other direction too: with no drill-down
/// open, `Esc` must still be history navigation.
#[test]
fn esc_outside_a_drilldown_is_still_history_navigation() {
    assert_eq!(
        action_for_key(press(KeyCode::Esc), &resources_context(false)),
        Some(Action::Navigation(NavAction::Back))
    );
}

/// The workspace conjunct of the guard, on its own axis.
///
/// `input_context` sets `resources_drilldown_open` from resources state
/// unconditionally, so an operator who drills into a dataset and then opens
/// the Logging workspace carries the flag `true` out of the resources
/// workspace with them. Only `workspace == Resources` keeps `Esc` from
/// leaving a drill-down that is no longer on screen.
#[test]
fn esc_in_another_workspace_ignores_an_open_drilldown() {
    let carried = Context {
        workspace: Workspace::Logging,
        resources_drilldown_open: true,
        ..base_context()
    };
    let plain = Context {
        resources_drilldown_open: false,
        ..carried.clone()
    };

    let action = action_for_key(press(KeyCode::Esc), &carried);

    assert_ne!(
        action,
        Some(Action::Resources(ResourcesAction::LeaveDrillDown)),
        "a drill-down the operator has left the workspace for must not claim Esc"
    );
    assert_eq!(
        action,
        action_for_key(press(KeyCode::Esc), &plain),
        "and the carried flag must change nothing outside the resources workspace"
    );
}

/// The flag the keymap reads is derived from the view, so it cannot disagree
/// with it — but only if `is_showing_drilldown` actually asks about the view.
#[test]
fn the_drilldown_flag_follows_the_view() {
    let mut app = drilled_in("sales", vec![table("sales", "orders")]);
    assert!(app.root.resources.is_showing_drilldown());

    escape(&mut app);

    assert!(!app.root.resources.is_showing_drilldown());
}

/// Another service's list is not a drill-down, whatever BigQuery's view says.
#[test]
fn another_service_is_never_showing_a_drilldown() {
    let mut app = drilled_in("sales", vec![table("sales", "orders")]);

    reduce(
        &mut app,
        &Action::Resources(ResourcesAction::SelectService(
            ResourceServiceKind::PubSubTopics,
        )),
    );

    assert!(!app.root.resources.is_showing_drilldown());
}

/// `-` and `Backspace` are bound for the whole resources list and refused per
/// service by the reducer, so an operator carrying the Cloud Storage habit
/// gets silence unless BigQuery answers them too. The drill is one level
/// deep, so this is the same leave `Esc` performs, not Storage's
/// level-at-a-time ascent.
#[test]
fn ascend_also_leaves_the_drilldown() {
    let mut app = drilled_in("sales", vec![table("sales", "orders")]);

    assert_eq!(
        action_for_key(press(KeyCode::Char('-')), &resources_context(true)),
        Some(Action::Resources(ResourcesAction::AscendPrefix)),
        "`-` is the key the habit reaches for"
    );
    let effects = reduce(&mut app, &Action::Resources(ResourcesAction::AscendPrefix));

    assert_eq!(app.root.resources.bigquery_view(), BigQueryView::Datasets);
    assert!(
        app.root.resources.bigquery_tables().is_empty(),
        "and it must leave through the same setter Esc does, dropping the scope"
    );
    assert!(effects.is_empty(), "reloading nothing: {effects:?}");
}

// --- The two lists are two lists ---

#[test]
fn the_cursor_in_the_tables_view_is_not_the_datasets_cursor() {
    let mut app = app_with_datasets(vec![dataset("analytics"), dataset("sales")]);
    // Off the first row, so the datasets cursor is somewhere a shared cursor
    // could be seen to move away from.
    reduce(&mut app, &Action::Resources(ResourcesAction::SelectNext));
    let effects = enter(&mut app);
    let context = app.root.current_context().expect("an active context");
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::BigQueryTablesLoaded {
            context,
            generation: listing_generation(&effects),
            listing: listing_epoch(&effects),
            tables: vec![table("sales", "orders"), table("sales", "returns")],
        }),
    );

    reduce(&mut app, &Action::Resources(ResourcesAction::SelectNext));

    assert_eq!(
        app.root
            .resources
            .selected_bigquery_table()
            .map(|table| table.table_id.as_str()),
        Some("returns"),
        "j must move the cursor of the list on screen"
    );
    escape(&mut app);
    assert_eq!(
        app.root
            .resources
            .selected_bigquery_dataset()
            .map(|dataset| dataset.dataset_id.as_str()),
        Some("sales"),
        "and must have left the datasets cursor on the dataset that was \
         entered: a shared cursor would have been moved by the j above"
    );
}

#[test]
fn the_filter_in_the_tables_view_is_not_the_datasets_filter() {
    let mut app = drilled_in(
        "sales",
        vec![table("sales", "orders"), table("sales", "returns")],
    );

    reduce(&mut app, &Action::Resources(ResourcesAction::StartFilter));
    for character in "orders".chars() {
        reduce(
            &mut app,
            &Action::Resources(ResourcesAction::AppendFilterChar(character)),
        );
    }
    reduce(&mut app, &Action::Resources(ResourcesAction::ApplyFilter));

    assert_eq!(
        app.root.resources.visible_bigquery_tables().len(),
        1,
        "the applied filter must narrow the tables that are on screen"
    );
    assert_eq!(app.root.resources.filter_applied(), "orders");

    escape(&mut app);

    assert_eq!(
        app.root.resources.filter_applied(),
        "",
        "the datasets list must come back unfiltered: the term was typed at \
         the tables view, which keeps its own"
    );
    assert_eq!(app.root.resources.visible_bigquery_datasets().len(), 1);
}

/// `visible_bigquery_table_count` survived replacement by both `0` and `1`.
///
/// This is `visible_bigquery_dataset_count`'s defect one drill-down down: the
/// count is the `total`/`filtered` pair the tables table's row counter paints,
/// and every assertion above reads `visible_bigquery_tables` instead. Both
/// constants are excluded by asserting counts that are neither, which is why
/// the filtered case leaves two rows rather than one.
#[test]
fn the_visible_table_count_counts_the_tables_the_filter_leaves() {
    let mut app = drilled_in(
        "sales",
        vec![
            table("sales", "orders"),
            table("sales", "orders_archive"),
            table("sales", "returns"),
        ],
    );

    assert_eq!(
        app.root.resources.visible_bigquery_table_count(),
        3,
        "with no filter applied every listed table is visible"
    );

    reduce(&mut app, &Action::Resources(ResourcesAction::StartFilter));
    for character in "orders".chars() {
        reduce(
            &mut app,
            &Action::Resources(ResourcesAction::AppendFilterChar(character)),
        );
    }
    reduce(&mut app, &Action::Resources(ResourcesAction::ApplyFilter));

    assert_eq!(
        app.root.resources.visible_bigquery_table_count(),
        2,
        "the count must follow the filter, or the row counter contradicts the \
         rows the table paints"
    );
    assert_eq!(
        app.root.resources.visible_bigquery_table_count(),
        app.root.resources.visible_bigquery_tables().len(),
        "the count and the list must never disagree: they are two readings of \
         one filter"
    );
}

// --- Landing a listing ---

/// A slow listing for a dataset the operator has since left must not land.
/// Cloud Storage's object pages carry the same guard, and for the same
/// reason: the rows would be painted under a header naming a different
/// dataset.
#[test]
fn a_listing_for_a_dataset_already_left_does_not_land() {
    let mut app = app_with_datasets(vec![dataset("sales"), dataset("logs")]);
    let effects = enter(&mut app);
    let context = app.root.current_context().expect("an active context");
    escape(&mut app);

    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::BigQueryTablesLoaded {
            context,
            generation: listing_generation(&effects),
            listing: listing_epoch(&effects),
            tables: vec![table("sales", "orders")],
        }),
    );

    assert!(
        app.root.resources.bigquery_tables().is_empty(),
        "a listing the view is no longer inside must be dropped"
    );
}

/// Re-entering the *same* dataset is a second drill, and the first drill's
/// listing belongs to the first.
///
/// The dataset id cannot tell the two apart: it is the same string both
/// times, so a scope check keyed on it accepts the first drill's answer over
/// the second drill's state. The visible cost is small while the payload is
/// only a row list, and it is the mechanism by which a per-row enrichment
/// fanned out from one drill would land against another's rows.
#[test]
fn a_listing_from_an_earlier_drill_into_the_same_dataset_does_not_land() {
    let mut app = app_with_datasets(vec![dataset("sales")]);
    let first = enter(&mut app);
    let context = app.root.current_context().expect("an active context");
    escape(&mut app);
    let second = enter(&mut app);
    assert_ne!(
        listing_generation(&first),
        listing_generation(&second),
        "two drills into one dataset must be issued under two generations, or \
         nothing downstream can tell their answers apart"
    );

    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::BigQueryTablesLoaded {
            context,
            generation: listing_generation(&first),
            listing: listing_epoch(&first),
            tables: vec![table("sales", "orders")],
        }),
    );

    assert!(
        app.root.resources.bigquery_tables().is_empty(),
        "the first drill's listing must not land in the second drill"
    );
    assert!(
        app.root.resources.bigquery_tables_loading(),
        "and the second drill's own listing must still read as in flight"
    );
}

#[test]
fn a_failed_listing_stays_in_the_dataset_and_says_why() {
    let mut app = app_with_datasets(vec![dataset("sales")]);
    let effects = enter(&mut app);
    let context = app.root.current_context().expect("an active context");

    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::BigQueryTablesLoadFailed {
            context,
            generation: listing_generation(&effects),
            listing: listing_epoch(&effects),
            error: nuvem::app::AppError::gcloud("permission denied"),
        }),
    );

    assert_eq!(
        app.root.resources.bigquery_view(),
        BigQueryView::Tables,
        "a failed listing must not eject the operator from the dataset they \
         asked about"
    );
    assert!(
        app.root.resources.bigquery_tables_problem().is_some(),
        "an empty list with no problem reads as a dataset with no tables"
    );
    assert!(
        !app.root.resources.bigquery_tables_loading(),
        "and the load must not still read as in flight"
    );
}

/// The failure path carries the same scope guard as the success path, and
/// nothing was reading it.
///
/// `a_listing_for_a_dataset_already_left_does_not_land` pins the conjunct in
/// the `BigQueryTablesLoaded` arm; the `BigQueryTablesLoadFailed` arm's
/// survived becoming `||`, because every failure the suite delivered was for
/// the drill on screen. A stale failure that lands clears the rows the
/// operator is reading and paints an abandoned call's error over them, and
/// the listing they are actually waiting on stops reading as in flight.
#[test]
fn a_failed_listing_from_an_abandoned_drill_does_not_land() {
    let mut app = app_with_datasets(vec![dataset("sales")]);
    let abandoned = enter(&mut app);
    escape(&mut app);
    let live = enter(&mut app);
    let context = app.root.current_context().expect("an active context");
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::BigQueryTablesLoaded {
            context: context.clone(),
            generation: listing_generation(&live),
            listing: listing_epoch(&live),
            tables: vec![table("sales", "orders")],
        }),
    );

    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::BigQueryTablesLoadFailed {
            context,
            generation: listing_generation(&abandoned),
            listing: listing_epoch(&abandoned),
            error: nuvem::app::AppError::gcloud("permission denied"),
        }),
    );

    assert!(
        app.root.resources.bigquery_tables_problem().is_none(),
        "the failure is a drill this view has left, so it must not become \
         this view's problem"
    );
    assert_eq!(
        app.root.resources.visible_bigquery_tables().len(),
        1,
        "and it must not clear the rows the operator is reading"
    );
}

#[test]
fn a_landed_listing_clears_the_in_flight_flag() {
    let mut app = app_with_datasets(vec![dataset("sales")]);
    let effects = enter(&mut app);
    assert!(
        app.root.resources.bigquery_tables_loading(),
        "entering a dataset promises a listing call, so the view is loading"
    );

    let context = app.root.current_context().expect("an active context");
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::BigQueryTablesLoaded {
            context,
            generation: listing_generation(&effects),
            listing: listing_epoch(&effects),
            tables: vec![table("sales", "orders")],
        }),
    );

    assert!(!app.root.resources.bigquery_tables_loading());
    assert_eq!(app.root.resources.visible_bigquery_tables().len(), 1);
}

/// Switching context clears the datasets list, and the drill scope is a
/// dataset of the project being left.
#[test]
fn clearing_the_datasets_list_leaves_the_tables_view() {
    let mut app = drilled_in("sales", vec![table("sales", "orders")]);

    app.root.resources.mark_bigquery_datasets_stale_and_clear();

    assert_eq!(app.root.resources.bigquery_view(), BigQueryView::Datasets);
    assert!(app.root.resources.bigquery_tables().is_empty());
    assert!(app.root.resources.bigquery_tables_dataset().is_none());
}

/// `r` inside the tables view retries *that* listing.
///
/// The failure state paints "Press r to retry.", and this is the arm that
/// makes the sentence true: without the second effect `r` would refresh the
/// datasets list behind the drill and leave the failed listing exactly as it
/// was, an affordance that does nothing and an error that never clears.
#[test]
fn refresh_inside_the_tables_view_re_issues_the_listing_for_that_dataset() {
    let mut app = app_with_datasets(vec![dataset("sales")]);
    let effects = enter(&mut app);
    let context = app.root.current_context().expect("an active context");
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::BigQueryTablesLoadFailed {
            context,
            generation: listing_generation(&effects),
            listing: listing_epoch(&effects),
            error: nuvem::app::AppError::gcloud("permission denied"),
        }),
    );

    let retry = reduce(&mut app, &Action::Resources(ResourcesAction::Refresh));

    assert!(
        retry.iter().any(|effect| matches!(
            effect,
            Effect::LoadBigQueryTables { dataset_id, .. } if dataset_id == "sales"
        )),
        "the retry must ask for the tables of the dataset the view is inside, \
         got {retry:?}"
    );
    assert_eq!(
        listing_generation(&retry),
        listing_generation(&effects),
        "a retry is another call for the drill on screen, not a new drill: a \
         fresh generation here would make the answer the operator is waiting \
         for undeliverable"
    );
    assert!(
        app.root.resources.bigquery_tables_problem().is_none(),
        "the failure being retried must stop painting, or the operator reads \
         the old error while the new call is in flight"
    );
    assert!(
        app.root.resources.bigquery_tables_loading(),
        "and the view must read as loading again"
    );
}

/// The same key on the datasets list asks for datasets and nothing else: the
/// tables effect is gated on the drill being open, not merely on the service
/// being BigQuery.
#[test]
fn refresh_on_the_datasets_list_asks_for_no_tables() {
    let mut app = app_with_datasets(vec![dataset("sales")]);

    let effects = reduce(&mut app, &Action::Resources(ResourcesAction::Refresh));

    assert!(
        effects
            .iter()
            .any(|effect| matches!(effect, Effect::RefreshBigQueryDatasets { .. })),
        "got {effects:?}"
    );
    assert!(
        !effects
            .iter()
            .any(|effect| matches!(effect, Effect::LoadBigQueryTables { .. })),
        "there is no dataset entered, so there is no listing to retry: \
         {effects:?}"
    );
}

/// The drill-down's `Esc` is in the help catalog, and reaches the help view.
///
/// Task 2 shipped the binding with no `HelpContext` that sets
/// `resources_drilldown_open`, so `keymap_help_parity_test` swept every
/// context without ever seeing the arm — a green parity gate there was silence
/// rather than coverage, and an `Esc` that means something new and appears in
/// no catalog is a key an operator cannot discover. This asserts the whole
/// path: the real drilled-in state resolves to the drill-down help context,
/// and the model built from it teaches the key.
#[test]
fn the_help_view_teaches_the_key_that_leaves_the_drilldown() {
    let mut app = drilled_in("sales", vec![table("sales", "orders")]);
    app.root.workspace = Workspace::Resources;

    let labels = nuvem::app::projection::help_model(&app.root)
        .sections
        .iter()
        .flat_map(|section| section.bindings.iter())
        .map(|binding| (binding.key.clone(), binding.label))
        .collect::<Vec<_>>();

    assert!(
        labels
            .iter()
            .any(|(key, label)| key == "Esc" && *label == "Leave drill-down"),
        "the operator must be able to find the key that leaves the tables \
         view: {labels:?}"
    );
    assert!(
        !labels
            .iter()
            .any(|(key, label)| key == "Esc/Alt-←" && *label == "Back"),
        "and it must not also be taught as history navigation, which is what \
         Esc does everywhere else in the resources workspace and does not do \
         here: {labels:?}"
    );
    // `Enter` is taught here, and was not: it dispatches inside the drill now
    // and opens the selected table's schema, so dropping it from this
    // catalogue would hide the only key that reaches the third level.
    assert!(
        labels.iter().any(|(_, label)| *label == "Open"),
        "the key that descends into a table's columns must be discoverable: \
         {labels:?}"
    );
    // The negative half, and the one no sweep can reach: `covered_by_catalog`
    // pairs a catalogue entry with a *dispatched trigger*, so a label naming a
    // key that dispatches nothing is invisible to the parity gate in both
    // directions. These two were removed from the frame footer for being
    // false promises; the help overlay is the more authoritative surface and
    // was still teaching them.
    for absent in ["Sort column", "Select column"] {
        assert!(
            !labels.iter().any(|(_, label)| *label == absent),
            "{absent} names a key that does nothing inside the drill-down, so \
             the help overlay must not teach it: {labels:?}"
        );
    }

    // Outside the drill it is the other way round, or the assertion above is
    // about the catalogue rather than about the drill-down.
    escape(&mut app);
    let labels = nuvem::app::projection::help_model(&app.root)
        .sections
        .iter()
        .flat_map(|section| section.bindings.iter())
        .map(|binding| (binding.key.clone(), binding.label))
        .collect::<Vec<_>>();
    assert!(
        labels
            .iter()
            .any(|(key, label)| key == "Esc/Alt-←" && *label == "Back"),
        "{labels:?}"
    );
    assert!(
        !labels.iter().any(|(_, label)| *label == "Leave drill-down"),
        "{labels:?}"
    );
}

/// The help overlay names the view it is open over, and the drill-down is a
/// view of its own.
///
/// `view_title`'s `HelpContext::ResourceDrilldown` arm survived deletion: the
/// catch-all below it answers "Help", so the overlay opened inside a dataset
/// would be titled with the generic fallback while teaching the drill-down's
/// bindings — including the `Esc` that means something different here. The
/// list case is asserted alongside it, or this pins the catch-all rather than
/// the arm.
///
/// Both drilled levels are asserted, and the schema level is here because it
/// reproduced the same survivor once it got a context of its own:
/// the two levels resolve to different `HelpContext` variants and shared one
/// title, so an arm covering only the level this test reached left the other
/// falling through to "Help" with nothing to say so. They share one arm now,
/// which is what makes a single deletion visible from either fixture.
#[test]
fn the_help_overlay_is_titled_for_the_drilldown_it_is_open_over() {
    let mut app = drilled_in("sales", vec![table("sales", "orders")]);
    app.root.workspace = Workspace::Resources;

    assert_eq!(
        nuvem::app::projection::help_model(&app.root).view_title,
        "Resource drill-down",
        "the overlay must name the drill-down, not fall through to the \
         catch-all title"
    );

    let schema = {
        let mut app = drilled_into_table(
            "sales",
            vec![table("sales", "orders")],
            vec![schema_field("order_id")],
        );
        app.root.workspace = Workspace::Resources;
        nuvem::app::projection::help_model(&app.root).view_title
    };
    assert_eq!(
        schema, "Resource drill-down",
        "the level below is a drill-down too, and titling it with the \
         catch-all is what a missing arm looks like"
    );

    escape(&mut app);

    assert_eq!(
        nuvem::app::projection::help_model(&app.root).view_title,
        "Resources",
        "and the title must follow the view back out of the drill"
    );
}

// --- What the drill-down's cursor and filter accessors return ---
//
// `the_cursor_in_the_tables_view_is_not_the_datasets_cursor` and
// `the_filter_in_the_tables_view_is_not_the_datasets_filter` assert
// *independence*: they move the tables side and check the datasets side is
// undisturbed. That kills a shared-field bug and nothing else — every
// accessor below survived having its body replaced wholesale, because no test
// read what the tables arm actually returns. These do.

/// The tables cursor clamps to the last row, and `min(len - 1)` is what does
/// it.
///
/// `bigquery_select_next`'s tables arm survived `len - 1` becoming `len + 1`
/// and `len / 1`: both let the cursor walk past the end, where
/// `selected_bigquery_table` resolves to nothing and the table paints a
/// selection on no row. Moving from row 0 to row 1 catches neither, so this
/// presses past the end and asserts the cursor sat down on the last row.
#[test]
fn the_tables_cursor_stops_on_the_last_row() {
    let mut app = drilled_in(
        "sales",
        vec![
            table("sales", "orders"),
            table("sales", "orders_archive"),
            table("sales", "returns"),
        ],
    );

    for _ in 0..5 {
        reduce(&mut app, &Action::Resources(ResourcesAction::SelectNext));
    }

    assert_eq!(
        app.root.resources.selected_visible_index(),
        Some(2),
        "the cursor must clamp to the last row rather than walk off the end"
    );
    assert_eq!(
        app.root
            .resources
            .selected_bigquery_table()
            .map(|table| table.table_id.as_str()),
        Some("returns"),
        "and the row it clamped to must still resolve to a table"
    );

    reduce(
        &mut app,
        &Action::Resources(ResourcesAction::SelectPrevious),
    );

    assert_eq!(
        app.root.resources.selected_visible_index(),
        Some(1),
        "and the cursor must be somewhere real to step back from"
    );
}

/// An empty tables list has no cursor, and asking it to move must not fall
/// into the arithmetic.
///
/// Two mutants live here. `bigquery_selected_visible_index`'s tables arm
/// survived losing its `!`, which reports a selected row for the empty list
/// and none for a loaded one. And `bigquery_select_next`'s `len > 0` survived
/// becoming `len >= 0`, which runs the clamp on an empty list and underflows
/// `len - 1` — a dataset with no tables is an ordinary thing to drill into.
#[test]
fn an_empty_tables_list_has_no_cursor_and_no_row_to_move_to() {
    let mut app = drilled_in("sales", Vec::new());

    assert_eq!(
        app.root.resources.selected_visible_index(),
        None,
        "an empty list has no selected row"
    );

    reduce(&mut app, &Action::Resources(ResourcesAction::SelectNext));

    assert_eq!(
        app.root.resources.selected_visible_index(),
        None,
        "and moving in it must leave it that way"
    );
    assert!(app.root.resources.selected_bigquery_table().is_none());
}

/// The filter surface inside the drill-down, one step at a time.
///
/// `bigquery_filter_input_active`, `bigquery_filter_draft`,
/// `bigquery_start_filter`, `bigquery_backspace_filter` and
/// `bigquery_cancel_filter` all survived being emptied out — replaced by
/// `false`, by `""`, by `"xyzzy"`, by `()`. Nothing asserted what the tables
/// arm returns, only that the datasets arm was left alone. This drives the
/// real sequence an operator types and reads the accessor back at every step.
#[test]
fn the_filter_in_the_tables_view_is_typed_backspaced_and_cancelled() {
    let mut app = drilled_in(
        "sales",
        vec![table("sales", "orders"), table("sales", "returns")],
    );

    assert!(
        !app.root.resources.is_filter_input_active(),
        "the drill-down opens with no filter input"
    );

    reduce(&mut app, &Action::Resources(ResourcesAction::StartFilter));

    assert!(
        app.root.resources.is_filter_input_active(),
        "`/` inside the tables view must open the tables view's own input"
    );
    assert_eq!(
        app.root.resources.filter_draft(),
        "",
        "seeded from the applied filter, which is empty here"
    );

    for character in "orders".chars() {
        reduce(
            &mut app,
            &Action::Resources(ResourcesAction::AppendFilterChar(character)),
        );
    }

    assert_eq!(
        app.root.resources.filter_draft(),
        "orders",
        "the draft must be the text that was typed, not a constant"
    );

    reduce(
        &mut app,
        &Action::Resources(ResourcesAction::BackspaceFilter),
    );

    assert_eq!(
        app.root.resources.filter_draft(),
        "order",
        "backspace must take a character off the tables draft"
    );

    reduce(&mut app, &Action::Resources(ResourcesAction::CancelFilter));

    assert!(
        !app.root.resources.is_filter_input_active(),
        "cancelling must close the input, or the view keeps eating keys"
    );
    assert_eq!(
        app.root.resources.filter_draft(),
        "",
        "and the abandoned draft must go back to the applied filter"
    );
    assert_eq!(
        app.root.resources.visible_bigquery_table_count(),
        2,
        "a cancelled filter narrows nothing"
    );
}

/// A pattern that will not compile reports its error inside the drill-down.
///
/// `bigquery_applied_filter_error` survived replacement by `None`, by
/// `Some(String::new())` and by `Some("xyzzy".into())` — the tables arm feeds
/// `ResourcesState::filter_error`, which is the inline message under the
/// filter line, and the drill-down never asked for it. The valid-pattern case
/// is what excludes the two `Some` constants; the bad-pattern case excludes
/// `None`.
#[test]
fn a_bad_filter_pattern_in_the_tables_view_reports_its_own_error() {
    let mut app = drilled_in(
        "sales",
        vec![table("sales", "orders"), table("sales", "returns")],
    );

    apply_filter(&mut app, "a(b");

    assert!(
        app.root
            .resources
            .filter_error()
            .is_some_and(|error| error.contains("invalid regex")),
        "the tables view must surface its own compile error, got {:?}",
        app.root.resources.filter_error()
    );
    assert_eq!(
        app.root.resources.visible_bigquery_table_count(),
        2,
        "and an uncompilable pattern narrows nothing, as everywhere else"
    );

    apply_filter(&mut app, "orders");

    assert_eq!(
        app.root.resources.filter_error(),
        None,
        "a pattern that compiles must clear the error rather than report an \
         empty or borrowed one"
    );
    assert_eq!(app.root.resources.visible_bigquery_table_count(), 1);
}

/// Types `pattern` into the view's filter input and applies it.
fn apply_filter(app: &mut AppState, pattern: &str) {
    reduce(app, &Action::Resources(ResourcesAction::StartFilter));
    for _ in 0..app.root.resources.filter_draft().chars().count() {
        reduce(app, &Action::Resources(ResourcesAction::BackspaceFilter));
    }
    for character in pattern.chars() {
        reduce(
            app,
            &Action::Resources(ResourcesAction::AppendFilterChar(character)),
        );
    }
    reduce(app, &Action::Resources(ResourcesAction::ApplyFilter));
}

// --- Merging an enrichment ---

/// The marker the enrichment tests write into a row.
///
/// A real stat field, and one a listing always leaves [`TableStat::Unavailable`]
/// — so a row carrying it was enriched, and a row that was not still reads
/// exactly as the listing left it. What this setter is about is the identity
/// match and the fields it leaves alone, but writing the field the fan-out
/// really writes means these tests keep testing the merge as it is used
/// rather than as it once had to be approximated.
const MERGED_MARKER: TableStat = TableStat::Known(1_787_306_613_694);

/// An enrichment is an answer about one row, so it must not move the cursor
/// or re-narrow the list the operator is reading.
///
/// The listing setter cannot serve this: it resets the cursor to 0, which is
/// right for a fresh listing and wrong here — an answer about the third row
/// would yank the operator back to the first.
#[test]
fn an_enrichment_merges_into_the_row_it_names_and_leaves_the_view_alone() {
    let mut app = drilled_in(
        "sales",
        vec![
            table("sales", "orders"),
            table("sales", "orders_archive"),
            table("sales", "returns"),
        ],
    );
    apply_filter(&mut app, "orders");
    reduce(&mut app, &Action::Resources(ResourcesAction::SelectNext));
    assert_eq!(
        app.root
            .resources
            .selected_bigquery_table()
            .map(|table| table.table_id.as_str()),
        Some("orders_archive"),
        "precondition: the cursor is on the second visible row"
    );

    app.root
        .resources
        .merge_bigquery_table_enrichment("orders_archive", |row| row.last_modified = MERGED_MARKER);
    // A row the filter is hiding is still a listed row, and a `bq show` was
    // issued for it too.
    app.root
        .resources
        .merge_bigquery_table_enrichment("returns", |row| row.last_modified = MERGED_MARKER);

    assert_eq!(
        app.root
            .resources
            .selected_bigquery_table()
            .map(|table| table.table_id.as_str()),
        Some("orders_archive"),
        "an answer about one row must leave the cursor on the row the \
         operator put it on"
    );
    assert_eq!(
        app.root.resources.visible_bigquery_table_count(),
        2,
        "and the applied filter must still narrow exactly as it did"
    );
    assert_eq!(app.root.resources.filter_applied(), "orders");
    assert_eq!(
        app.root
            .resources
            .bigquery_tables()
            .iter()
            .find(|row| row.table_id == "orders_archive")
            .map(|row| row.last_modified),
        Some(MERGED_MARKER),
        "the named row must actually carry the enrichment"
    );
    assert_eq!(
        app.root
            .resources
            .bigquery_tables()
            .iter()
            .find(|row| row.table_id == "returns")
            .map(|row| row.last_modified),
        Some(MERGED_MARKER),
        "and so must a row the filter is hiding: it is listed, so a show was \
         issued for it"
    );
    assert_eq!(
        app.root
            .resources
            .bigquery_tables()
            .iter()
            .find(|row| row.table_id == "orders")
            .map(|row| row.last_modified),
        Some(TableStat::Unavailable),
        "the row nothing was merged for must be untouched"
    );
}

/// A `bq show` can outlive the row it was issued for: the listing is
/// re-issued by `r`, and the new listing need not contain every table the old
/// one did.
///
/// Such a result is dropped. Appending it would paint a row that exists in no
/// listing — no kind, no created time, no labels — and the rows are `bq ls`'s
/// answer, never an enrichment's.
#[test]
fn an_enrichment_for_a_table_no_longer_listed_is_dropped_rather_than_appended() {
    let mut app = drilled_in(
        "sales",
        vec![table("sales", "orders"), table("sales", "returns")],
    );
    let retry = reduce(&mut app, &Action::Resources(ResourcesAction::Refresh));
    let context = app.root.current_context().expect("an active context");
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::BigQueryTablesLoaded {
            context,
            generation: listing_generation(&retry),
            listing: listing_epoch(&retry),
            tables: vec![table("sales", "orders")],
        }),
    );

    app.root
        .resources
        .merge_bigquery_table_enrichment("returns", |row| row.last_modified = MERGED_MARKER);

    assert_eq!(
        app.root
            .resources
            .bigquery_tables()
            .iter()
            .map(|row| row.table_id.as_str())
            .collect::<Vec<_>>(),
        vec!["orders"],
        "an enrichment must never add a row: the listing says which tables \
         exist"
    );
    assert_eq!(
        app.root
            .resources
            .bigquery_tables()
            .iter()
            .find(|row| row.table_id == "orders")
            .map(|row| row.last_modified),
        Some(TableStat::Unavailable),
        "and it must not land on some other row either: the id it names is \
         the row it is about"
    );
}

/// A `bq show` can land while an `r` retry of the listing is in flight — `r`
/// keeps the rows painted and does not bump the drill generation, so the
/// enrichment is still this drill's and still has a row to land on.
///
/// It must not clear the in-flight flag. The listing setter does, correctly:
/// it is the listing arriving. An enrichment is a different call, and clearing
/// the flag would stop the retry reading as in flight while it still is.
#[test]
fn an_enrichment_does_not_end_a_listing_that_is_still_in_flight() {
    let mut app = drilled_in("sales", vec![table("sales", "orders")]);
    reduce(&mut app, &Action::Resources(ResourcesAction::Refresh));
    assert!(
        app.root.resources.bigquery_tables_loading(),
        "precondition: the retry is in flight"
    );

    app.root
        .resources
        .merge_bigquery_table_enrichment("orders", |row| row.last_modified = MERGED_MARKER);

    assert!(
        app.root.resources.bigquery_tables_loading(),
        "the listing this view is waiting on is still in flight: an answer \
         about one row does not end it"
    );
    assert_eq!(
        app.root
            .resources
            .bigquery_tables()
            .iter()
            .find(|row| row.table_id == "orders")
            .map(|row| row.last_modified),
        Some(MERGED_MARKER),
        "and the enrichment still landed on the row it names"
    );
}

// --- The schema level ---

/// Entering a table records which table the view is inside and mints a request
/// naming exactly it.
///
/// The dataset id is on the request as well as the table id: `bq show` needs
/// both, and a request carrying only the table id would name a resource that
/// is not unique across the project.
#[test]
fn enter_on_a_selected_table_shows_its_schema_and_asks_for_exactly_it() {
    let mut app = drilled_in("sales", vec![table("sales", "orders")]);

    let request = app
        .root
        .resources
        .open_selected_bigquery_table()
        .expect("entering the selected table must mint a schema load");

    assert_eq!(app.root.resources.bigquery_view(), BigQueryView::Schema);
    assert_eq!(
        app.root
            .resources
            .bigquery_schema_table()
            .map(|table| table.table_id.as_str()),
        Some("orders"),
        "the view must record which table it is inside, or a landing schema \
         has no scope to be checked against"
    );
    assert_eq!(request.dataset_id, "sales");
    assert_eq!(request.table_id, "orders");
    assert!(
        app.root.resources.bigquery_schema_loading(),
        "and the load it just asked for is in flight"
    );
}

/// The empty-list case, which is not the same as "the first row": a tables
/// list the filter has emptied has no selected table, and entering must not
/// move the view to a schema with no table behind it.
#[test]
fn enter_with_no_table_selected_does_nothing_at_all() {
    let mut app = drilled_in("sales", Vec::new());

    let request = app.root.resources.open_selected_bigquery_table();

    assert!(request.is_none(), "there was no table to ask about");
    assert_eq!(
        app.root.resources.bigquery_view(),
        BigQueryView::Tables,
        "an Enter that resolved no table must not move the view"
    );
    assert!(app.root.resources.bigquery_schema_table().is_none());
}

/// A column is the bottom of the hierarchy, so `Enter` inside the schema opens
/// nothing — and in particular must not re-enter, which would reset the schema
/// scope to the *table* row still selected behind the view.
///
/// The refused request is the live pin: delete the `view != Tables` guard from
/// `open_selected_bigquery_table` and the first assertion fails. The second is
/// deliberately not a live pin today — `restore_invariant` clears nothing on
/// arriving at `Schema`, so a re-entry would leave the columns painted anyway.
/// It is here against a later edit that makes arrival at a level clear that
/// level, which would turn a refused keypress into a wiped list.
#[test]
fn enter_inside_the_schema_view_opens_nothing() {
    let mut app = drilled_in("sales", vec![table("sales", "orders")]);
    app.root.resources.open_selected_bigquery_table();
    app.root
        .resources
        .set_bigquery_schema(vec![schema_field("order_id")]);

    let request = app.root.resources.open_selected_bigquery_table();

    assert!(request.is_none(), "a column has nothing under it to open");
    assert_eq!(
        app.root.resources.bigquery_schema_fields().len(),
        1,
        "and the columns already loaded must survive the keypress"
    );
}

/// The cursor inside the schema stops on the last column.
///
/// The sibling of `the_tables_cursor_stops_on_the_last_row`, and it exists
/// because nothing was moving the cursor in this view at all:
/// `bigquery_select_next`'s schema arm survived `selected + 1` becoming
/// `selected - 1`, and `len > 0` becoming both `len < 0` and `len == 0`. The
/// first two leave the cursor where it was; the third runs the clamp only on
/// an empty list, where `len - 1` underflows. Moving from row 0 to row 1
/// catches the first pair and not the clamp, so this presses past the end and
/// asserts the cursor sat down on the last column.
///
/// The column it lands on is asserted by name rather than by index alone: an
/// index that clamps correctly while `selected_bigquery_schema_field` resolves
/// against the wrong collection is the shape a three-list service invites, and
/// the datasets list two levels behind this one has a row 2 as well.
#[test]
fn the_schema_cursor_stops_on_the_last_column() {
    let mut app = drilled_in("sales", vec![table("sales", "orders")]);
    app.root.resources.open_selected_bigquery_table();
    app.root.resources.set_bigquery_schema(vec![
        schema_field("order_id"),
        schema_field("customer"),
        schema_field("customer.city"),
    ]);

    for _ in 0..5 {
        reduce(&mut app, &Action::Resources(ResourcesAction::SelectNext));
    }

    assert_eq!(
        app.root.resources.selected_visible_index(),
        Some(2),
        "the cursor must clamp to the last column rather than walk off the end"
    );
    assert_eq!(
        app.root
            .resources
            .selected_bigquery_schema_field()
            .map(|field| field.path.as_str()),
        Some("customer.city"),
        "and the row it clamped to must still resolve to a column of this \
         table, not to whatever sits at that index two levels up"
    );

    reduce(
        &mut app,
        &Action::Resources(ResourcesAction::SelectPrevious),
    );

    assert_eq!(
        app.root.resources.selected_visible_index(),
        Some(1),
        "and the cursor must be somewhere real to step back from"
    );
    assert_eq!(
        app.root
            .resources
            .selected_bigquery_schema_field()
            .map(|field| field.path.as_str()),
        Some("customer")
    );
}

/// An empty schema has no cursor, and asking it to move must not fall into the
/// arithmetic.
///
/// The `len > 0` guard is what stands between an empty level and `len - 1`
/// underflowing, and an empty level is an ordinary thing here: a `bq show`
/// whose schema flattens to nothing is an answered call, not a failed one.
#[test]
fn an_empty_schema_has_no_cursor_and_no_column_to_move_to() {
    let mut app = drilled_in("sales", vec![table("sales", "orders")]);
    app.root.resources.open_selected_bigquery_table();
    app.root.resources.set_bigquery_schema(Vec::new());

    assert_eq!(
        app.root.resources.selected_visible_index(),
        None,
        "an empty level has no selected row"
    );

    reduce(&mut app, &Action::Resources(ResourcesAction::SelectNext));

    assert_eq!(
        app.root.resources.selected_visible_index(),
        None,
        "and moving in it must leave it that way"
    );
    assert!(
        app.root
            .resources
            .selected_bigquery_schema_field()
            .is_none()
    );
}

/// Leaving the schema comes back to the tables list *as it was*, not to a
/// tables list that has to be listed again.
///
/// This is the ascent that distinguishes a three-level hierarchy from two
/// two-level ones. `restore_invariant` clears what lives below the view
/// arrived at, so arriving at `Tables` must leave `Tables`' own rows alone —
/// clearing them would make every `Esc` out of a schema re-issue a `bq ls`
/// for a dataset whose rows are already in hand.
#[test]
fn esc_from_the_schema_returns_to_the_tables_that_are_still_listed() {
    let mut app = drilled_in("sales", vec![table("sales", "orders")]);
    app.root.resources.open_selected_bigquery_table();

    app.root.resources.leave_bigquery_schema();

    assert_eq!(app.root.resources.bigquery_view(), BigQueryView::Tables);
    assert_eq!(
        app.root.resources.bigquery_tables().len(),
        1,
        "the tables listed for the dataset are still listed: the schema was \
         below them, not instead of them"
    );
    assert_eq!(
        app.root
            .resources
            .bigquery_tables_dataset()
            .map(|dataset| dataset.dataset_id.as_str()),
        Some("sales"),
        "and the drill is still inside the dataset it was inside"
    );
}

/// The same table entered twice is two drills, and they must be told apart.
///
/// The generation is the whole identity of a schema load, so if re-entry did
/// not mint a new one, the first entry's `bq show` would still be in scope
/// when it returned and would land on the second entry's view.
#[test]
fn the_same_table_entered_twice_is_two_drills_with_different_generations() {
    let mut app = drilled_in("sales", vec![table("sales", "orders")]);

    let first = app
        .root
        .resources
        .open_selected_bigquery_table()
        .expect("the first entry mints a load");
    app.root.resources.leave_bigquery_schema();
    let second = app
        .root
        .resources
        .open_selected_bigquery_table()
        .expect("the second entry mints another");

    assert_eq!(
        first.table_id, second.table_id,
        "precondition: it is the same table both times, so the table id \
         cannot be what tells the two drills apart"
    );
    assert_ne!(
        first.generation, second.generation,
        "re-entering is a second drill and must not inherit the first \
         drill's identity"
    );
    assert!(
        !app.root
            .resources
            .matches_bigquery_schema_scope(first.generation),
        "so the first drill's schema load is out of scope for the second"
    );
}

/// Leaving the table takes its schema load out of scope — and the guard is
/// what has to say so.
///
/// Asked of `matches_bigquery_schema_scope` directly rather than by landing
/// the result and inspecting the rows, for the reason
/// `an_enrichment_for_a_drill_that_was_left_is_out_of_scope` records in
/// `tests/bigquery_enrichment_test.rs`: the ascent also empties the schema
/// rows, so "the rows are empty afterwards" passes with the guard replaced by
/// `true` and is a fact about the clearing rather than about the check.
#[test]
fn a_schema_for_a_table_that_was_left_is_out_of_scope() {
    let mut app = drilled_in("sales", vec![table("sales", "orders")]);
    let abandoned = app
        .root
        .resources
        .open_selected_bigquery_table()
        .expect("entering mints a load");

    assert!(
        app.root
            .resources
            .matches_bigquery_schema_scope(abandoned.generation),
        "the load for the table on screen is in scope, or the assertion \
         below is testing nothing"
    );

    app.root.resources.leave_bigquery_schema();

    assert!(
        !app.root
            .resources
            .matches_bigquery_schema_scope(abandoned.generation),
        "the table that show was issued for has been left, so its answer is \
         out of scope"
    );
}

/// The invariant the ascent to `Tables` exists to hold: a table entered after
/// another must not show the previous table's columns.
///
/// Delete the schema-clearing line from
/// `BigQueryDrilldownState::restore_invariant` and this fails at the first
/// assertion — `Esc` leaves `payments`' columns loaded, and re-entering paints
/// them under `orders`' header until the fresh `bq show` returns.
#[test]
fn re_entering_a_table_does_not_paint_the_previous_schema() {
    let mut app = drilled_in(
        "sales",
        vec![table("sales", "orders"), table("sales", "payments")],
    );
    app.root.resources.open_selected_bigquery_table();
    app.root
        .resources
        .set_bigquery_schema(vec![schema_field("order_id")]);
    assert_eq!(
        app.root.resources.bigquery_schema_fields().len(),
        1,
        "precondition: there is a column loaded to lose, or every assertion \
         below passes over a schema that was never populated"
    );

    app.root.resources.leave_bigquery_schema();

    assert!(
        app.root.resources.bigquery_schema_fields().is_empty(),
        "the columns loaded for the table just left must not survive it"
    );
    assert!(
        app.root.resources.bigquery_schema_table().is_none(),
        "and neither must the schema scope naming that table"
    );

    app.root.resources.open_selected_bigquery_table();

    assert!(
        app.root.resources.bigquery_schema_fields().is_empty(),
        "so the fresh entry starts with nothing painted rather than with the \
         previous table's columns"
    );
}

/// Leaving the *dataset* clears both levels below it, not just the one
/// immediately below.
///
/// A schema still loaded on the datasets list is a scope with no view to be
/// about, and would be painted by the next `Enter` into any table of any
/// dataset before its own load returned.
#[test]
fn leaving_the_dataset_drops_the_schema_as_well_as_the_tables() {
    let mut app = drilled_in("sales", vec![table("sales", "orders")]);
    app.root.resources.open_selected_bigquery_table();
    app.root
        .resources
        .set_bigquery_schema(vec![schema_field("order_id")]);
    assert_eq!(
        (
            app.root.resources.bigquery_tables().len(),
            app.root.resources.bigquery_schema_fields().len()
        ),
        (1, 1),
        "precondition: both levels have a row to lose, or the assertions \
         below pass over collections that were never populated"
    );

    app.root.resources.mark_bigquery_datasets_stale_and_clear();

    assert_eq!(app.root.resources.bigquery_view(), BigQueryView::Datasets);
    assert!(
        app.root.resources.bigquery_tables().is_empty(),
        "the tables go with the dataset"
    );
    assert!(
        app.root.resources.bigquery_schema_fields().is_empty(),
        "and so do the columns two levels down"
    );
    assert!(app.root.resources.bigquery_schema_table().is_none());
}

/// The schema list has its own filter and its own cursor, kept apart from the
/// tables list's the way the tables list's are kept apart from the datasets
/// list's.
#[test]
fn the_filter_in_the_schema_view_is_not_the_tables_filter() {
    let mut app = drilled_in("sales", vec![table("sales", "orders")]);
    apply_filter(&mut app, "orders");
    app.root.resources.open_selected_bigquery_table();
    app.root.resources.set_bigquery_schema(vec![
        schema_field("order_id"),
        schema_field("event_params"),
        schema_field("event_params.key"),
    ]);

    assert_eq!(
        app.root.resources.visible_bigquery_schema_field_count(),
        3,
        "the tables filter must not narrow the columns"
    );

    apply_filter(&mut app, "event_params");

    assert_eq!(
        app.root
            .resources
            .visible_bigquery_schema_fields()
            .iter()
            .map(|field| field.path.as_str())
            .collect::<Vec<_>>(),
        vec!["event_params", "event_params.key"],
        "a path filter selects the record and everything nested inside it"
    );

    app.root.resources.leave_bigquery_schema();

    assert_eq!(
        app.root.resources.visible_bigquery_table_count(),
        1,
        "and the tables list comes back with the filter it had, not the \
         schema's"
    );
}

// --- The three guards the schema view made wrong ---

/// Drills into `dataset_id`, into the first table listed inside it, and lands
/// `fields` as that table's schema.
pub(crate) fn drilled_into_table(
    dataset_id: &str,
    tables: Vec<BigQueryTable>,
    fields: Vec<BigQuerySchemaField>,
) -> AppState {
    let mut app = drilled_in(dataset_id, tables);
    let effects = enter(&mut app);
    let context = app.root.current_context().expect("an active context");
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
    assert_eq!(
        app.root.resources.bigquery_view(),
        BigQueryView::Schema,
        "precondition: the fixture is inside the schema view"
    );
    app
}

/// `Esc` in the schema view goes up a level, and the flag the keymap reads is
/// what decides that.
///
/// The columns list is a drill-down by the same definition the tables list is:
/// a list reached from a row of the list above it, which `Esc` leaves for that
/// list rather than leaving the workspace. With the flag reading false here,
/// `Esc` is history navigation and an operator two levels down is thrown out
/// of the Resources workspace by the key that means "back".
#[test]
fn esc_in_the_schema_view_leaves_the_drill_rather_than_the_workspace() {
    let app = drilled_into_table(
        "sales",
        vec![table("sales", "orders")],
        vec![schema_field("order_id")],
    );

    let context = drilled_context(&app);

    assert_eq!(
        action_for_key(press(KeyCode::Esc), &context),
        Some(Action::Resources(ResourcesAction::LeaveDrillDown)),
        "two levels down, Esc's back is still the list above — not the way \
         out of the workspace"
    );
}

/// The ascent is one level at a time, so the schema view leaves for the tables
/// list it was entered from.
///
/// Landing on the datasets list from here skips the level in between: the
/// operator asked to go up one and arrived two, with the tables listing they
/// were reading dropped along the way.
#[test]
fn esc_from_the_schema_view_lands_on_the_tables_list() {
    let mut app = drilled_into_table(
        "sales",
        vec![table("sales", "orders")],
        vec![schema_field("order_id")],
    );

    let effects = escape(&mut app);

    assert_eq!(app.root.resources.bigquery_view(), BigQueryView::Tables);
    assert_eq!(
        app.root.resources.bigquery_tables().len(),
        1,
        "and the tables it comes back to are the ones already listed, not a \
         list that has to be fetched again"
    );
    assert!(
        effects.is_empty(),
        "so nothing is re-issued on the way up: {effects:?}"
    );
}

/// `r` retries the call the view on screen is showing, and in the schema view
/// that is the `bq show` for the table entered — not a `bq ls` for the dataset
/// two levels up.
///
/// A listing issued from here lands *while the columns are on screen*: the
/// retry deliberately does not bump the drill generation, so the tables scope
/// check passes and the rows install behind a view that is not showing them.
#[test]
fn r_in_the_schema_view_retries_the_schema_and_not_the_tables_listing() {
    let mut app = drilled_into_table(
        "sales",
        vec![table("sales", "orders")],
        vec![schema_field("order_id")],
    );

    let effects = reduce(&mut app, &Action::Resources(ResourcesAction::Refresh));

    assert!(
        !effects
            .iter()
            .any(|effect| matches!(effect, Effect::LoadBigQueryTables { .. })),
        "the dataset behind two levels must not be re-listed: {effects:?}"
    );
    match effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::LoadBigQuerySchema {
                dataset_id,
                table_id,
                ..
            } => Some((dataset_id.as_str(), table_id.as_str())),
            _ => None,
        })
        .collect::<Vec<_>>()
        .as_slice()
    {
        [("sales", "orders")] => {}
        other => panic!("expected one schema retry for sales.orders, got {other:?} in {effects:?}"),
    }
    assert_eq!(
        app.root.resources.bigquery_view(),
        BigQueryView::Schema,
        "and the retry stays where it is: r reloads the view, it does not leave it"
    );
}

/// The *dataset* opener refuses the schema view, which is the other half of
/// what makes `Enter` on a column a no-op.
///
/// `enter_inside_the_schema_view_opens_nothing` pins the table opener's
/// refusal; this pins the one above it. The reducer calls both in order, so
/// with this guard gone the dataset row still selected behind two levels would
/// be re-entered: the view would drop to the tables list, the columns on
/// screen would be cleared, and a `bq ls` would be issued for a dataset whose
/// tables are already in hand.
#[test]
fn enter_inside_the_schema_view_does_not_re_enter_the_dataset_behind_it() {
    // The schema view is reached through the state API rather than through the
    // `Enter` action, deliberately: the reducer's `Enter` arm calls the very
    // opener under test first, so a fixture built with it collapses under the
    // mutation this pins instead of failing the assertion below.
    let mut app = drilled_in("sales", vec![table("sales", "orders")]);
    app.root.resources.open_selected_bigquery_table();
    app.root
        .resources
        .set_bigquery_schema(vec![schema_field("order_id")]);
    assert_eq!(
        app.root.resources.bigquery_view(),
        BigQueryView::Schema,
        "precondition: the columns are what is on screen"
    );

    let request = app.root.resources.open_selected_bigquery_dataset();

    assert!(
        request.is_none(),
        "the dataset two levels up is not what Enter opens here"
    );
    assert_eq!(
        app.root.resources.bigquery_view(),
        BigQueryView::Schema,
        "and the view stays on the columns"
    );
    assert_eq!(
        app.root.resources.bigquery_schema_fields().len(),
        1,
        "which are still the ones loaded, not cleared by a transition"
    );
}
