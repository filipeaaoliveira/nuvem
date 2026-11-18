//! Cloud Functions' slice of `ResourcesState`, and the reducer path that
//! fills it.
//!
//! Slice 1 parsed `functions list`; this is where the parsed rows are held.
//! The parse itself is `tests/cloud_functions_service_test.rs` and is not
//! re-asserted here.

use nuvem::app::projection::table::SortDirection;
use nuvem::app::reducer::root::reduce;
use nuvem::app::state::resources::{
    CloudFunctionSortField, CloudFunctions, ResourceLoadPhase, ResourceService, ResourcesState,
};
use nuvem::app::{Action, AppError, AppState, Effect, EffectResult, ResourcesAction};
use nuvem_domain::gcp::configuration::{
    GcloudConfiguration, GcloudConfigurationName, GcloudContext,
};
use nuvem_domain::gcp::diagnostics::{DiagnosticProblem, GcloudErrorKind};
use nuvem_domain::gcp::filter::compile_filter;
use nuvem_domain::gcp::resource::{
    CloudFunction, CloudFunctionEnvironment, CloudFunctionState, ComputeKeyValue,
    ResourceServiceKind,
};
use std::time::Instant;

const FUNCTIONS: ResourceServiceKind = ResourceServiceKind::CloudFunctions;

fn ctx() -> GcloudContext {
    GcloudContext {
        configuration: GcloudConfigurationName::new("dev"),
        account: None,
        project: Some("dev-project".into()),
        region: None,
        zone: None,
    }
}

fn other_ctx() -> GcloudContext {
    GcloudContext {
        configuration: GcloudConfigurationName::new("prod"),
        account: None,
        project: Some("prod-project".into()),
        region: None,
        zone: None,
    }
}

fn function(name: &str, region: &str) -> CloudFunction {
    CloudFunction {
        name: name.into(),
        region: region.into(),
        state: CloudFunctionState::Active,
        environment: CloudFunctionEnvironment::Gen2,
        url: Some(format!(
            "https://{region}-dev-project.cloudfunctions.net/{name}"
        )),
        update_time: Some("2026-01-01T00:00:00Z".into()),
        labels: vec![ComputeKeyValue {
            key: "env".into(),
            value: "prod".into(),
        }],
    }
}

fn loaded(rows: &[(&str, &str)]) -> ResourcesState {
    let mut state = ResourcesState::default();
    state.select_service(FUNCTIONS);
    state.set_cloud_functions(
        ctx(),
        rows.iter()
            .map(|(name, region)| function(name, region))
            .collect(),
    );
    state
}

/// Moves the column cursor onto `index` and sorts by it, through the same
/// keys an operator would press. Nothing here reaches past the public API to
/// install a sort field the UI could not select.
fn sort_by_column(state: &mut ResourcesState, index: usize) {
    for _ in 0..index {
        state.select_next_column();
    }
    state.sort_selected_column();
}

fn selected(state: &ResourcesState) -> Option<(String, String)> {
    state
        .selected_cloud_function()
        .map(|function| (function.name.clone(), function.region.clone()))
}

/// An app with the `dev` configuration active and Cloud Functions on screen.
fn app_showing_functions() -> AppState {
    let mut app = AppState::default();
    app.root.configurations.set_active(GcloudConfiguration {
        name: GcloudConfigurationName::new("dev"),
        is_active: true,
        account: None,
        project: Some("dev-project".into()),
        region: None,
        zone: None,
    });
    app.root.resources.select_service(FUNCTIONS);
    app
}

// ---------------------------------------------------------------------------
// Holding the rows
// ---------------------------------------------------------------------------

#[test]
fn delivering_functions_stamps_the_context_and_clears_staleness() {
    let mut state = ResourcesState::default();
    state.select_service(FUNCTIONS);
    state.begin_load(FUNCTIONS, Instant::now());

    state.set_cloud_functions(ctx(), vec![function("thumbnailer", "europe-west1")]);

    assert_eq!(state.cloud_functions().len(), 1);
    assert_eq!(state.loaded_list_context(FUNCTIONS), Some(&ctx()));
    assert!(!state.is_stale());
    assert_eq!(
        state.load_phase(FUNCTIONS),
        &ResourceLoadPhase::Idle,
        "a delivered list resolves the load phase, or the spinner ticks forever"
    );
    assert!(!state.needs_refresh(&ctx()));
    assert!(
        state.needs_refresh(&other_ctx()),
        "rows loaded for one configuration must not read as loaded for another"
    );
}

#[test]
fn marking_functions_stale_drops_the_rows_and_the_context_stamp() {
    let mut state = loaded(&[("thumbnailer", "europe-west1")]);

    state.mark_active_stale_and_clear(FUNCTIONS);

    assert!(state.cloud_functions().is_empty());
    assert_eq!(state.loaded_list_context(FUNCTIONS), None);
    assert!(state.is_stale());
}

/// A refresh keeps the cursor on the function it was on, and a function is
/// identified by name *and* region: two regions can hold a `thumbnailer`.
#[test]
fn a_refresh_keeps_the_cursor_on_the_same_function_in_the_same_region() {
    let mut state = loaded(&[
        ("thumbnailer", "europe-west1"),
        ("thumbnailer", "us-central1"),
    ]);
    state.select_next();
    assert_eq!(
        selected(&state),
        Some(("thumbnailer".into(), "us-central1".into()))
    );

    // A new row sorts ahead of both, so a cursor kept by index would move.
    state.set_cloud_functions(
        ctx(),
        vec![
            function("archiver", "europe-west1"),
            function("thumbnailer", "europe-west1"),
            function("thumbnailer", "us-central1"),
        ],
    );

    assert_eq!(
        selected(&state),
        Some(("thumbnailer".into(), "us-central1".into())),
        "the cursor followed the row's position instead of the row"
    );
}

#[test]
fn a_cursor_past_the_end_of_a_shorter_list_lands_on_its_last_row() {
    let mut state = loaded(&[
        ("archiver", "europe-west1"),
        ("thumbnailer", "europe-west1"),
    ]);
    state.select_next();

    state.set_cloud_functions(ctx(), vec![function("archiver", "europe-west1")]);

    assert_eq!(
        selected(&state),
        Some(("archiver".into(), "europe-west1".into()))
    );
}

/// The highlighted row index the table renders its cursor from: a row when
/// there is one, and nothing at all when the list is empty — an index into an
/// empty list would highlight a row that is not there.
#[test]
fn the_cursor_index_exists_only_while_a_row_does() {
    let mut state = loaded(&[
        ("archiver", "europe-west1"),
        ("thumbnailer", "europe-west1"),
    ]);
    assert_eq!(state.selected_visible_index(), Some(0));
    state.select_next();
    assert_eq!(state.selected_visible_index(), Some(1));

    state.mark_active_stale_and_clear(FUNCTIONS);

    assert_eq!(
        state.selected_visible_index(),
        None,
        "an empty list has no row to highlight"
    );
}

/// `j`/`k` are dispatched unconditionally from the list, so both must be
/// no-ops on an empty one rather than moving a cursor onto a row that is not
/// there — or, with the length guard loosened, computing an index from an
/// empty list at all.
#[test]
fn moving_the_cursor_on_an_empty_functions_list_does_nothing() {
    let mut state = ResourcesState::default();
    state.select_service(FUNCTIONS);

    state.select_next();
    state.select_previous();

    assert_eq!(state.selected_visible_index(), None);
    assert!(state.selected_cloud_function().is_none());
}

/// And on a loaded list the cursor stops at the last row rather than running
/// past it.
#[test]
fn the_cursor_stops_at_the_last_function() {
    let mut state = loaded(&[
        ("archiver", "europe-west1"),
        ("thumbnailer", "europe-west1"),
    ]);

    state.select_next();
    state.select_next();
    state.select_next();

    assert_eq!(state.selected_visible_index(), Some(1));
    assert_eq!(
        selected(&state),
        Some(("thumbnailer".into(), "europe-west1".into()))
    );
}

/// The same question asked against the *projected* row count, which is what
/// the table actually drew: zero drawn rows means no highlight, any drawn row
/// means the cursor.
#[test]
fn the_cursor_index_for_projected_rows_follows_the_drawn_row_count() {
    let mut state = loaded(&[
        ("archiver", "europe-west1"),
        ("thumbnailer", "europe-west1"),
    ]);
    state.select_next();

    assert_eq!(state.selected_visible_index_for_visible_rows(0), None);
    assert_eq!(state.selected_visible_index_for_visible_rows(1), Some(1));
    assert_eq!(state.selected_visible_index_for_visible_rows(2), Some(1));
}

// ---------------------------------------------------------------------------
// Filtering and sorting
// ---------------------------------------------------------------------------

#[test]
fn the_filter_narrows_on_name_region_and_labels() {
    for (pattern, expected) in [
        ("thumb", vec![("thumbnailer", "europe-west1")]),
        ("us-central1", vec![("resizer", "us-central1")]),
        (
            "env=prod",
            vec![("resizer", "us-central1"), ("thumbnailer", "europe-west1")],
        ),
    ] {
        let mut state = loaded(&[("thumbnailer", "europe-west1"), ("resizer", "us-central1")]);
        state.start_filter();
        for ch in pattern.chars() {
            state.append_filter_char(ch);
        }
        state.apply_filter();

        let visible: Vec<(String, String)> = state
            .visible_cloud_functions()
            .into_iter()
            .map(|function| (function.name.clone(), function.region.clone()))
            .collect();
        let expected: Vec<(String, String)> = expected
            .into_iter()
            .map(|(name, region)| (name.to_string(), region.to_string()))
            .collect();
        assert_eq!(visible, expected, "filter {pattern:?}");
        assert_eq!(state.visible_cloud_function_count(), expected.len());
    }
}

/// Name is not a total order over this row set, so region breaks the tie —
/// otherwise two same-named functions sort in whatever order they arrived and
/// the table reshuffles between refreshes.
#[test]
fn same_named_functions_sort_by_region() {
    let state = loaded(&[
        ("thumbnailer", "us-central1"),
        ("thumbnailer", "europe-west1"),
    ]);

    let regions: Vec<String> = state
        .visible_cloud_functions()
        .into_iter()
        .map(|function| function.region.clone())
        .collect();

    assert_eq!(regions, vec!["europe-west1", "us-central1"]);
}

#[test]
fn sorting_by_the_region_column_orders_by_region_and_reverses() {
    let mut state = loaded(&[("thumbnailer", "us-central1"), ("archiver", "europe-west1")]);
    state.select_next_column();
    state.sort_selected_column();

    assert_eq!(state.sort_direction(), SortDirection::Ascending);
    assert_eq!(
        state
            .visible_cloud_functions()
            .into_iter()
            .map(|function| function.region.clone())
            .collect::<Vec<_>>(),
        vec!["europe-west1", "us-central1"]
    );

    state.sort_selected_column();
    assert_eq!(state.sort_direction(), SortDirection::Descending);
    assert_eq!(
        state
            .visible_cloud_functions()
            .into_iter()
            .map(|function| function.region.clone())
            .collect::<Vec<_>>(),
        vec!["us-central1", "europe-west1"]
    );
}

/// State and generation sort by the word the column paints, not by the API
/// value behind it and not by a `{:?}` shell. The header carries the sort
/// arrow, so an operator sorting by STATE expects the column to read in order;
/// `GEN_1`/`GEN_2` and `DETACH_FAILED` would order by strings that appear
/// nowhere on screen.
#[test]
fn state_and_generation_sort_by_the_label_the_column_paints() {
    let states = [
        // Chosen so API-value order and label order disagree: by api_value
        // `DETACH_FAILED` precedes `DETACHING`, by label `Detach failed`
        // precedes `Detaching` — and `ACTIVE` precedes both while `Active`
        // also does, so the discriminating pair is the two D's.
        CloudFunctionState::Detaching,
        CloudFunctionState::DetachFailed,
        CloudFunctionState::Active,
    ];
    let mut state = ResourcesState::default();
    state.select_service(FUNCTIONS);
    state.set_cloud_functions(
        ctx(),
        states
            .iter()
            .enumerate()
            .map(|(index, function_state)| {
                let mut row = function(&format!("fn-{index}"), "europe-west1");
                row.state = function_state.clone();
                row
            })
            .collect(),
    );

    sort_by_column(&mut state, CloudFunctionSortField::State.column());
    assert_eq!(
        state
            .visible_cloud_functions()
            .into_iter()
            .map(|function| function.state.label().to_string())
            .collect::<Vec<_>>(),
        vec!["Active", "Detach failed", "Detaching"],
        "STATE must order by the labels it paints"
    );

    let environments = [
        CloudFunctionEnvironment::Gen2,
        CloudFunctionEnvironment::Unspecified,
        CloudFunctionEnvironment::Gen1,
    ];
    let mut state = ResourcesState::default();
    state.select_service(FUNCTIONS);
    state.set_cloud_functions(
        ctx(),
        environments
            .iter()
            .enumerate()
            .map(|(index, environment)| {
                let mut row = function(&format!("fn-{index}"), "europe-west1");
                row.environment = environment.clone();
                row
            })
            .collect(),
    );

    sort_by_column(&mut state, CloudFunctionSortField::Generation.column());
    assert_eq!(
        state
            .visible_cloud_functions()
            .into_iter()
            .map(|function| function.environment.label().to_string())
            .collect::<Vec<_>>(),
        vec!["1st gen", "2nd gen", "unspecified"],
        "GENERATION must order by the labels it paints"
    );
}

/// Update times sort chronologically, and a function that has never reported
/// one groups at one end rather than being scattered through the list.
#[test]
fn updated_sorts_chronologically_with_the_absent_times_together() {
    let times = [
        Some("2026-03-01T10:00:00Z"),
        None,
        Some("2026-01-05T08:30:00Z"),
        None,
    ];
    let mut state = ResourcesState::default();
    state.select_service(FUNCTIONS);
    state.set_cloud_functions(
        ctx(),
        times
            .iter()
            .enumerate()
            .map(|(index, time)| {
                let mut row = function(&format!("fn-{index}"), "europe-west1");
                row.update_time = time.map(str::to_string);
                row
            })
            .collect(),
    );

    sort_by_column(&mut state, CloudFunctionSortField::Updated.column());
    assert_eq!(
        state
            .visible_cloud_functions()
            .into_iter()
            .map(|function| function.update_time.clone())
            .collect::<Vec<_>>(),
        vec![
            None,
            None,
            Some("2026-01-05T08:30:00Z".to_string()),
            Some("2026-03-01T10:00:00Z".to_string()),
        ],
        "an unreported update time must not scatter through the ordering"
    );
}

/// The `/` filter searches what the table shows. Slice 2's table showed name
/// and region, so those were the searchable fields; this slice paints four
/// more cells, and a term an operator can read on screen that matches nothing
/// is the filter lying about the row it hid.
#[test]
fn the_filter_narrows_on_every_cell_the_table_paints() {
    let mut row = function("thumbnailer", "europe-west1");
    row.state = CloudFunctionState::DetachFailed;
    row.environment = CloudFunctionEnvironment::Gen1;
    row.update_time = Some("2026-03-01T10:00:00Z".into());
    row.url = Some("https://europe-west1-dev.cloudfunctions.net/thumbnailer".into());

    for term in [
        "Detach failed",
        "1st gen",
        "2026-03-01",
        "cloudfunctions.net/thumbnailer",
    ] {
        let mut state = ResourcesState::default();
        state.select_service(FUNCTIONS);
        state.set_cloud_functions(ctx(), vec![row.clone()]);
        state.start_filter();
        for ch in term.chars() {
            state.append_filter_char(ch);
        }
        state.apply_filter();

        assert_eq!(
            state.visible_cloud_function_count(),
            1,
            "the filter must find the row by {term:?}, which the table paints"
        );
    }
}

// ---------------------------------------------------------------------------
// No detail pane
// ---------------------------------------------------------------------------

/// Every detail-pane key is dispatched unconditionally from the generic
/// resource list, so each has to be a no-op here rather than opening an empty
/// pane or moving a tab cursor that has no tabs behind it.
#[test]
fn a_function_has_no_detail_pane_to_open_scroll_or_tab_through() {
    let mut app = app_showing_functions();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::CloudFunctionsLoaded {
            context: ctx(),
            functions: vec![function("thumbnailer", "europe-west1")],
        }),
    );
    assert!(app.root.resources.selected_cloud_function().is_some());

    let effects = reduce(&mut app, &Action::Resources(ResourcesAction::OpenSelected));

    assert!(
        effects.is_empty(),
        "Enter on a function must not fetch a detail nobody can render"
    );
    assert!(!app.root.resources.is_showing_detail());

    // The tab and scroll keys reach the same collection.
    app.root.resources.select_next_tab();
    app.root.resources.select_previous_tab();
    app.root.resources.scroll_detail_down();
    app.root.resources.scroll_detail_up();
    app.root.resources.close_detail();

    assert!(!app.root.resources.is_showing_detail());
    assert_eq!(app.root.resources.detail_scroll(), 0);
    assert_eq!(
        app.root.resources.active_detail_tab_label(),
        "",
        "with no tabs there is no tab name, and the empty label is what the \
         breadcrumb reads when a pane it cannot open is asked to name itself"
    );
    assert_eq!(
        app.root
            .resources
            .selected_cloud_function()
            .map(|f| f.name.clone()),
        Some("thumbnailer".to_string()),
        "none of those keys may disturb the list behind the pane that is not there"
    );
}

/// The sort field is the table's own vocabulary: its label is the word the
/// status line shows after `Sort:`, and `from_column`/`column` are how a
/// column position and a sort field find each other. A wrong answer here sorts
/// by one column while the header arrow sits on another.
#[test]
fn every_sortable_column_maps_to_its_field_and_back() {
    for (index, field, label) in [
        (0, CloudFunctionSortField::Name, "Name"),
        (1, CloudFunctionSortField::Region, "Region"),
        (2, CloudFunctionSortField::State, "State"),
        (3, CloudFunctionSortField::Generation, "Generation"),
        (4, CloudFunctionSortField::Updated, "Updated"),
    ] {
        assert_eq!(CloudFunctionSortField::from_column(index), Some(field));
        assert_eq!(field.column(), index);
        assert_eq!(field.label(), label);
    }
    assert_eq!(
        CloudFunctionSortField::from_column(CloudFunctionSortField::COLUMN_COUNT),
        None,
        "a column past the catalogue is not sortable"
    );

    // And the label is what the list actually reports, not a string only this
    // test reads.
    let mut state = loaded(&[("thumbnailer", "europe-west1")]);
    assert_eq!(state.sort_label(), "Name");
    state.select_next_column();
    state.sort_selected_column();
    assert_eq!(state.sort_label(), "Region");
}

/// A recorded diagnostic problem is readable until the next successful load.
///
/// The only way to observe `set_service_problem`'s Cloud Functions arm, and
/// the reason the arm exists: an empty list with a permission failure behind
/// it must be able to say which failure, rather than reading as a project with
/// no functions.
#[test]
fn a_recorded_problem_is_readable_until_the_next_successful_load() {
    let mut state = loaded(&[]);
    state.set_service_problem(
        FUNCTIONS,
        DiagnosticProblem::new(GcloudErrorKind::PermissionDenied, "denied"),
    );

    let problem = state
        .cloud_functions_problem()
        .expect("the recorded problem must be readable");
    assert_eq!(problem.kind, GcloudErrorKind::PermissionDenied);
    assert_eq!(problem.message, "denied");

    state.set_cloud_functions(ctx(), vec![function("thumbnailer", "europe-west1")]);

    assert!(
        state.cloud_functions_problem().is_none(),
        "a successful load must clear the previous problem"
    );
}

// ---------------------------------------------------------------------------
// The generic service interface
// ---------------------------------------------------------------------------

/// `ResourceService` is how generic code reaches a service without a
/// per-service match, and Cloud Functions' impl has no caller in this slice —
/// marks are the only consumer and this service has none. Unexercised is not
/// the same as free to be wrong: the day a generic path does reach it, it must
/// answer what the tab already answers, so the three methods are pinned
/// against the accessors the tab itself uses.
#[test]
fn the_generic_service_impl_answers_what_the_tab_answers() {
    let filter = compile_filter("thumb").expect("a compilable filter");
    let matching = function("thumbnailer", "europe-west1");
    let other = function("archiver", "europe-west1");

    assert!(CloudFunctions::matches_filter(&matching, &filter));
    assert!(
        !CloudFunctions::matches_filter(&other, &filter),
        "the trait must narrow the same rows the list does, or a generic \
         caller sees a different table than the one on screen"
    );

    assert_eq!(
        CloudFunctions::compare(
            &matching,
            &other,
            CloudFunctionSortField::Name,
            SortDirection::Ascending
        ),
        std::cmp::Ordering::Greater,
        "the trait must order rows the way the table does"
    );

    // Identity is name *and* region: same name, different region, different
    // row — the property the list's cursor-keeping depends on.
    let elsewhere = function("thumbnailer", "us-central1");
    assert_eq!(
        CloudFunctions::row_id(&matching),
        CloudFunctions::row_id(&matching.clone())
    );
    assert_ne!(
        CloudFunctions::row_id(&matching),
        CloudFunctions::row_id(&elsewhere),
        "two regions' same-named functions must not share one identity"
    );
}

// ---------------------------------------------------------------------------
// The load path
// ---------------------------------------------------------------------------

#[test]
fn refreshing_the_functions_list_asks_for_functions() {
    let mut app = app_showing_functions();

    let effects = reduce(&mut app, &Action::Resources(ResourcesAction::Refresh));

    assert_eq!(
        effects,
        vec![Effect::RefreshCloudFunctions {
            context: ctx(),
            filter: None
        }],
        "the refresh must name Cloud Functions, not whichever service was \
         active before"
    );
}

/// The applied filter rides along, because label mode narrows on the server
/// only: dropping it here returns the whole project under a filtered heading.
#[test]
fn a_refresh_carries_the_applied_filter() {
    let mut app = app_showing_functions();
    app.root.resources.start_filter();
    for ch in "-l env=prod".chars() {
        app.root.resources.append_filter_char(ch);
    }
    app.root.resources.apply_filter();

    let effects = reduce(&mut app, &Action::Resources(ResourcesAction::Refresh));

    assert_eq!(
        effects,
        vec![Effect::RefreshCloudFunctions {
            context: ctx(),
            filter: Some("-l env=prod".to_string())
        }]
    );
}

#[test]
fn a_delivered_result_installs_the_rows() {
    let mut app = app_showing_functions();

    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::CloudFunctionsLoaded {
            context: ctx(),
            functions: vec![function("thumbnailer", "europe-west1")],
        }),
    );

    assert_eq!(
        app.root
            .resources
            .cloud_functions()
            .iter()
            .map(|function| function.name.clone())
            .collect::<Vec<_>>(),
        vec!["thumbnailer".to_string()],
    );
    assert_eq!(
        app.root.resources.loaded_list_context(FUNCTIONS),
        Some(&ctx())
    );
}

/// A result for a configuration the operator has already switched away from
/// must not paint that project's functions under this one's header.
#[test]
fn a_result_for_another_context_is_ignored() {
    let mut app = app_showing_functions();

    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::CloudFunctionsLoaded {
            context: other_ctx(),
            functions: vec![function("thumbnailer", "europe-west1")],
        }),
    );

    assert!(app.root.resources.cloud_functions().is_empty());
    assert_eq!(app.root.resources.loaded_list_context(FUNCTIONS), None);
}

#[test]
fn a_failed_load_clears_the_rows_and_records_the_failure_against_its_context() {
    let mut app = app_showing_functions();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::CloudFunctionsLoaded {
            context: ctx(),
            functions: vec![function("thumbnailer", "europe-west1")],
        }),
    );

    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::CloudFunctionsLoadFailed {
            context: ctx(),
            error: AppError::gcloud("PERMISSION_DENIED".to_string()),
        }),
    );

    assert!(app.root.resources.cloud_functions().is_empty());
    assert!(
        app.root.resources.list_load_failed_for(FUNCTIONS, &ctx()),
        "the failure must be recorded against the context it happened under, \
         or a configuration switch leaves it reading as the new one's failure"
    );
    assert!(
        !app.root
            .resources
            .list_load_failed_for(FUNCTIONS, &other_ctx())
    );
}
