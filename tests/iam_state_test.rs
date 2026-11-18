//! IAM service accounts' slice of `ResourcesState`: the collection, the sort,
//! the filter, and the absence of a detail pane.
//!
//! The parse is `tests/iam_service_test.rs` and is not re-asserted here. What
//! this file is about is what the state does with rows once it has them —
//! most of all, that the disabled flag behaves like the fact an access review
//! is scanning for: it sorts as its own group, it filters by the exact word
//! the cell paints, and an account with no display name behaves like an empty
//! name rather than scattering through the order.
//!
//! There is no detail pane in this slice, so there is nothing here about one
//! beyond the assertion that `Enter` stays a quiet no-op.

use nuvem::app::projection::SortDirection;
use nuvem::app::reducer::root::reduce;
use nuvem::app::state::command::CommandDescriptor;
use nuvem::app::state::resources::{
    IamServiceAccounts, ResourceService, ResourcesState, ServiceAccountSortField,
};
use nuvem::app::state::root::Workspace;
use nuvem::app::{Action, AppState, CommandAction, EffectResult, ResourcesAction};
use nuvem_domain::gcp::configuration::{
    GcloudConfiguration, GcloudConfigurationName, GcloudContext,
};
use nuvem_domain::gcp::filter::compile_filter;
use nuvem_domain::gcp::resource::{ResourceServiceKind, ServiceAccount};

fn ctx() -> GcloudContext {
    GcloudContext {
        configuration: GcloudConfigurationName::new("dev"),
        account: None,
        project: Some("dev-project".into()),
        region: None,
        zone: None,
    }
}

/// A second context, for the freshness assertions: rows loaded under one
/// configuration say nothing about another.
fn other_ctx() -> GcloudContext {
    GcloudContext {
        configuration: GcloudConfigurationName::new("prod"),
        account: None,
        project: Some("prod-project".into()),
        region: None,
        zone: None,
    }
}

fn account(local: &str) -> ServiceAccount {
    ServiceAccount {
        email: format!("{local}@dev-project.iam.gserviceaccount.com"),
        display_name: Some(format!("{local} display")),
        disabled: false,
        unique_id: "100000000000000000001".to_string(),
    }
}

/// A state holding `accounts`, with no filter applied.
fn state_with(accounts: Vec<ServiceAccount>) -> ResourcesState {
    let mut state = ResourcesState::default();
    state.select_service(ResourceServiceKind::IamServiceAccounts);
    state.set_service_accounts(ctx(), accounts);
    state
}

fn visible_emails(state: &ResourcesState) -> Vec<String> {
    state
        .visible_service_accounts()
        .into_iter()
        .map(|account| account.email.clone())
        .collect()
}

fn visible_locals(state: &ResourcesState) -> Vec<String> {
    visible_emails(state)
        .into_iter()
        .map(|email| email.split('@').next().expect("an email").to_string())
        .collect()
}

// --- The collection ---

#[test]
fn a_reload_keeps_the_cursor_on_the_account_it_was_already_on() {
    let mut state = state_with(vec![account("alpha"), account("beta"), account("gamma")]);
    state.select_next();
    assert_eq!(
        state
            .selected_service_account()
            .map(|account| account.email.clone()),
        Some(account("beta").email)
    );

    // The same accounts, listed in a different order.
    state.set_service_accounts(
        ctx(),
        vec![account("gamma"), account("beta"), account("alpha")],
    );

    assert_eq!(
        state
            .selected_service_account()
            .map(|account| account.email.clone()),
        Some(account("beta").email),
        "the cursor follows the account's email, not the row index"
    );
}

#[test]
fn a_reload_that_drops_the_selected_account_clamps_the_cursor() {
    let mut state = state_with(vec![account("alpha"), account("beta")]);
    state.select_next();

    state.set_service_accounts(ctx(), vec![account("alpha")]);

    assert_eq!(
        state
            .selected_service_account()
            .map(|account| account.email.clone()),
        Some(account("alpha").email),
        "a cursor past the end of the new list must not dangle"
    );
}

#[test]
fn marking_the_accounts_stale_empties_the_rows_and_drops_the_context() {
    let mut state = state_with(vec![account("alpha")]);

    state.mark_service_accounts_stale_and_clear();

    assert!(state.service_accounts().is_empty());
    assert!(!state.has_loaded_context());
}

#[test]
fn selecting_next_on_an_empty_account_list_does_nothing() {
    // The `len > 0` guard is load-bearing, not decoration: without it the
    // clamp below computes `len - 1` on an empty list, which underflows.
    let mut state = state_with(Vec::new());

    state.select_next();

    assert!(state.selected_service_account().is_none());
    assert_eq!(state.selected_visible_index(), None);
}

#[test]
fn selecting_next_at_the_last_account_stays_on_it() {
    let mut state = state_with(vec![account("alpha"), account("beta")]);

    // Past the end twice over: the cursor clamps to the last row rather than
    // running off it, which is what `.min(len - 1)` is for.
    state.select_next();
    state.select_next();
    state.select_next();

    assert_eq!(
        state
            .selected_service_account()
            .map(|account| account.email.clone()),
        Some(account("beta").email),
        "the cursor must clamp to the last account, not run past it"
    );
}

#[test]
fn selecting_previous_at_the_first_account_stays_on_it() {
    let mut state = state_with(vec![account("alpha"), account("beta")]);

    state.select_previous();
    state.select_previous();

    assert_eq!(
        state
            .selected_service_account()
            .map(|account| account.email.clone()),
        Some(account("alpha").email)
    );
}

// --- The sort ---

#[test]
fn sorting_by_state_groups_the_disabled_accounts_at_the_bottom() {
    let mut disabled = account("retired");
    disabled.disabled = true;
    let mut state = state_with(vec![account("zz-live"), disabled, account("aa-live")]);

    // Move the column cursor onto `state` (catalogue index 2) and sort by it,
    // the same path the `s` key takes.
    state.select_next_column();
    state.select_next_column();
    state.sort_selected_column();

    // `false < true`, so ascending puts the enabled accounts first and one
    // keypress brings the disabled tail into view.
    assert_eq!(
        visible_locals(&state),
        vec![
            "aa-live".to_string(),
            "zz-live".to_string(),
            "retired".to_string()
        ],
        "ascending state sort must group the disabled accounts last, with the \
         email breaking ties inside each group"
    );
}

#[test]
fn an_absent_display_name_sorts_as_empty_rather_than_scattering() {
    let mut unnamed = account("unnamed");
    unnamed.display_name = None;
    let mut aardvark = account("aardvark");
    aardvark.display_name = Some("Aardvark".to_string());
    let mut zebra = account("zebra");
    zebra.display_name = Some("Zebra".to_string());
    let mut state = state_with(vec![zebra, unnamed, aardvark]);

    // Column 1 is `display_name`.
    state.select_next_column();
    state.sort_selected_column();

    assert_eq!(
        visible_locals(&state),
        vec![
            "unnamed".to_string(),
            "aardvark".to_string(),
            "zebra".to_string()
        ],
        "an account with no display name must sort as an empty name — grouped \
         at one end — not wherever a None placeholder happens to land"
    );
}

#[test]
fn accounts_with_the_same_display_name_order_by_email() {
    let mut first = account("aa-worker");
    first.display_name = Some("Worker".to_string());
    let mut second = account("zz-worker");
    second.display_name = Some("Worker".to_string());
    let mut state = state_with(vec![second.clone(), first.clone()]);

    state.select_next_column();
    state.sort_selected_column();

    assert_eq!(
        visible_locals(&state),
        vec!["aa-worker".to_string(), "zz-worker".to_string()],
        "the email breaks every tie, so equal display names still yield a \
         stable, total order"
    );
}

// --- The filter ---

#[test]
fn the_filter_matches_every_cell_the_table_paints() {
    let mut retired = account("retired");
    retired.disabled = true;
    retired.display_name = Some("Old deploy identity".to_string());
    retired.unique_id = "104405669981102690742".to_string();
    let mut vault = account("sa-vault");
    vault.display_name = Some("Vault node pool".to_string());
    vault.unique_id = "110727432291788759739".to_string();
    let rows = vec![retired, vault];

    for (term, expected) in [
        ("sa-vault", "sa-vault"),
        ("Old deploy", "retired"),
        ("Disabled", "retired"),
        ("110727432291788759739", "sa-vault"),
    ] {
        // A fresh state per term: `start_filter` seeds the draft from the
        // applied filter, so reusing one state would AND the terms together.
        let mut state = state_with(rows.clone());
        state.start_filter();
        for character in term.chars() {
            state.append_filter_char(character);
        }
        state.apply_filter();
        assert_eq!(
            visible_locals(&state),
            vec![expected.to_string()],
            "filtering on {term:?} must narrow to {expected:?}"
        );
    }
}

#[test]
fn filtering_on_enabled_does_not_match_the_disabled_rows_by_substring() {
    // "Enabled" is not a substring of "Disabled", and the state cell is the
    // only place either word appears — so the filter separates the two
    // populations exactly, which is the query an access review runs.
    let mut retired = account("retired");
    retired.disabled = true;
    let mut state = state_with(vec![account("live"), retired]);

    state.start_filter();
    for character in "Enabled".chars() {
        state.append_filter_char(character);
    }
    state.apply_filter();

    assert_eq!(
        visible_locals(&state),
        vec!["live".to_string()],
        "the filter searches the state label the cell shows, and only the \
         enabled row carries the word Enabled"
    );
}

// --- The reducer path ---

fn config() -> GcloudConfiguration {
    GcloudConfiguration {
        name: GcloudConfigurationName::new("dev"),
        is_active: true,
        account: None,
        project: Some("dev-project".to_string()),
        region: None,
        zone: None,
    }
}

fn app_showing_accounts() -> AppState {
    let mut app = AppState::new();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ConfigurationsLoaded {
            configurations: vec![config()],
        }),
    );
    reduce(
        &mut app,
        &Action::Resources(ResourcesAction::SelectService(
            ResourceServiceKind::IamServiceAccounts,
        )),
    );
    app
}

#[test]
fn a_loaded_result_installs_the_rows_for_the_active_context() {
    let mut app = app_showing_accounts();
    let context = app.root.current_context().expect("context exists");

    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ServiceAccountsLoaded {
            context,
            accounts: vec![account("deployer")],
        }),
    );

    assert_eq!(app.root.resources.service_accounts().len(), 1);
}

#[test]
fn pressing_enter_on_an_account_does_nothing_at_all() {
    let mut app = app_showing_accounts();
    let context = app.root.current_context().expect("context exists");
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ServiceAccountsLoaded {
            context,
            accounts: vec![account("deployer")],
        }),
    );

    let effects = reduce(&mut app, &Action::Resources(ResourcesAction::OpenSelected));

    // List-only in this slice, the shape Cloud Functions and Pub/Sub shipped
    // in. `Enter` has to be a *quiet* no-op: no effect to load a pane, and no
    // pane half opened for one to fill.
    assert!(
        effects.is_empty(),
        "Enter must emit no effect at all: {effects:?}"
    );
    assert!(
        !app.root.resources.is_showing_detail(),
        "Enter must not leave a detail pane open on nothing"
    );
}

// --- Freshness ---
//
// `needs_refresh`'s arm is `context != Some(context) || stale`, and on the
// Pub/Sub slice both operators survived mutation — the `!=` to `==` and the
// `||` to `&&` — because nothing loaded the same rows twice or crossed a
// context. A wrong answer here either refetches on every frame or serves
// another project's accounts.

#[test]
fn accounts_need_a_refresh_before_any_context_is_loaded() {
    let mut state = ResourcesState::default();
    state.select_service(ResourceServiceKind::IamServiceAccounts);

    assert!(
        state.needs_refresh(&ctx()),
        "an account list that has never loaded must ask for a refresh"
    );
}

#[test]
fn accounts_need_no_refresh_for_the_context_they_were_loaded_for() {
    let state = state_with(vec![account("deployer")]);

    assert!(
        !state.needs_refresh(&ctx()),
        "freshly loaded, non-stale accounts for the very same context must \
         not be refetched"
    );
}

#[test]
fn accounts_need_a_refresh_for_a_context_they_were_not_loaded_for() {
    let state = state_with(vec![account("deployer")]);

    assert!(
        state.needs_refresh(&other_ctx()),
        "accounts loaded under one context say nothing about another context, \
         so a different context must ask for a refresh even while the rows \
         are not stale"
    );
}

#[test]
fn stale_accounts_need_a_refresh_even_for_their_own_context() {
    let mut state = state_with(vec![account("deployer")]);
    state.mark_service_accounts_stale_and_clear();

    assert!(
        state.needs_refresh(&ctx()),
        "rows marked stale must be refetched for the context that loaded \
         them, which is the whole point of marking them"
    );
}

// --- The visible count and the cursor ---

/// `visible_service_account_count` is the `total`/`filtered` the table's row
/// counter paints, and no test above reads it — the assertions all walk
/// `visible_service_accounts` instead. Both constant replacements are excluded
/// by asserting a count that is neither `0` nor `1`, and the filtered case
/// pins that it counts what survives the filter rather than the whole
/// collection. (Its Pub/Sub twin survived exactly those two mutants.)
#[test]
fn the_visible_count_counts_the_rows_the_filter_leaves() {
    let mut state = state_with(vec![
        account("deployer"),
        account("deployer-ci"),
        account("auditor"),
    ]);

    assert_eq!(
        state.visible_service_account_count(),
        3,
        "with no filter applied every loaded account is visible"
    );

    state.start_filter();
    for character in "deployer".chars() {
        state.append_filter_char(character);
    }
    state.apply_filter();

    assert_eq!(
        state.visible_service_account_count(),
        2,
        "the count must follow the filter, or the row counter contradicts the \
         rows the table paints"
    );
    assert_eq!(
        state.visible_service_account_count(),
        state.visible_service_accounts().len(),
        "the count and the list must never disagree: they are two readings of \
         one filter"
    );
}

/// `(visible_rows > 0).then_some(..)` for the accounts arm, pinned on both
/// sides of the boundary — the Pub/Sub twin survived mutation to `==`, `<`
/// and `>=`, two of which are constant for a `usize`.
#[test]
fn the_selected_index_is_a_row_while_rows_are_visible_and_nothing_when_none_are() {
    let mut state = state_with(vec![account("deployer"), account("auditor")]);

    assert_eq!(
        state.selected_index(),
        Some(0),
        "a loaded list starts with its first row selected; None here leaves \
         the table with nothing highlighted"
    );

    // A filter matching no account leaves the rows loaded and none visible,
    // which is the only way to reach zero visible rows without also emptying
    // the collection.
    state.start_filter();
    for character in "zzz-matches-nothing".chars() {
        state.append_filter_char(character);
    }
    state.apply_filter();

    assert!(
        visible_emails(&state).is_empty(),
        "the fixture needs a filter that hides every row"
    );
    assert_eq!(
        state.selected_index(),
        None,
        "with no visible rows there is no row to select, and reporting one \
         points the cursor at a row the operator cannot see"
    );
}

// --- The sort field ---

/// `ServiceAccountSortField::label` and `from_column`, pinned per variant.
///
/// On the Pub/Sub slice, `label` survived replacement by both `""` and a
/// constant, and `from_column` survived deletion of three of its four `Some`
/// arms: the sort tests drive sorting through `ResourcesAction`, which never
/// spells the label or the column index. A wrong `from_column` silently sorts
/// by the wrong column when the operator picks one.
#[test]
fn every_sortable_column_maps_to_its_field_and_carries_its_own_label() {
    let by_column = [
        (0, ServiceAccountSortField::Email, "Email"),
        (1, ServiceAccountSortField::DisplayName, "Display name"),
        (2, ServiceAccountSortField::Disabled, "State"),
        (3, ServiceAccountSortField::UniqueId, "Unique ID"),
    ];

    for (index, field, label) in by_column {
        assert_eq!(
            ServiceAccountSortField::from_column(index),
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
    // above if it happens to equal one of them, and four columns that read
    // the same tell the operator nothing.
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
        ServiceAccountSortField::from_column(4),
        None,
        "the accounts catalogue has exactly four columns, every one of them \
         sortable; a fifth field would sort by a column that is not there"
    );
    assert_eq!(ServiceAccountSortField::COLUMN_COUNT, 4);
}

// --- The diagnostic ---

/// `service_accounts_problem` is what the home overview reads to show the
/// service as degraded, and the reducer tests above assert rows rather than
/// the problem, so the accessor could report a healthy service forever. Both
/// transitions are pinned: a failed load raises the problem, a successful one
/// clears it.
#[test]
fn a_failed_load_raises_a_problem_and_a_later_success_clears_it() {
    let mut app = app_showing_accounts();
    let context = app.root.current_context().expect("active context");
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ServiceAccountsLoadFailed {
            context: context.clone(),
            error: nuvem::app::AppError::gcloud("permission denied"),
        }),
    );

    assert!(
        app.root.resources.service_accounts_problem().is_some(),
        "a failed accounts load must leave a problem for the overview to \
         show, or a degraded service reads as a healthy one"
    );

    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ServiceAccountsLoaded {
            context,
            accounts: vec![account("deployer")],
        }),
    );

    assert!(
        app.root.resources.service_accounts_problem().is_none(),
        "a successful load must clear the previous problem"
    );
}

// --- The generic service interface ---

/// `ResourceService` is how generic code reaches a service without a
/// per-service match, and the accounts impl has no caller in this slice — the
/// `visible_*` accessors call the filter and comparator directly. Unexercised
/// is not the same as free to be wrong: the day a generic path does reach it,
/// it must answer what the tab already answers, so the methods are pinned
/// against the accessors the tab itself uses. (Pub/Sub's `matches_filter`
/// survived replacement by both `true` and `false` for exactly this reason.)
#[test]
fn the_generic_service_impl_answers_what_the_tab_answers() {
    let filter = compile_filter("deployer").expect("a compilable filter");
    let matching = account("deployer");
    let other = account("auditor");

    assert!(
        IamServiceAccounts::matches_filter(&matching, &filter),
        "the trait must keep the rows the list keeps"
    );
    assert!(
        !IamServiceAccounts::matches_filter(&other, &filter),
        "the trait must narrow the same rows the list does, or a generic \
         caller sees a different table than the one on screen"
    );

    assert_eq!(
        IamServiceAccounts::compare(
            &matching,
            &other,
            ServiceAccountSortField::Email,
            SortDirection::Ascending
        ),
        std::cmp::Ordering::Greater,
        "the trait must order rows the way the table does"
    );

    // An account is global within its project and the email alone is its
    // identity — there is no location to qualify it with, which is the
    // property the cursor-keeping on reload depends on.
    assert_eq!(
        IamServiceAccounts::row_id(&matching),
        IamServiceAccounts::row_id(&matching.clone())
    );
    assert_ne!(
        IamServiceAccounts::row_id(&matching),
        IamServiceAccounts::row_id(&other),
        "two accounts with different emails must not share one identity"
    );
}

// --- The command palette ---

/// A service account is not a log source, so — unlike every log-bearing
/// sibling — the palette must never offer the account-scoped log jump, even
/// in exactly the state where a log-bearing service would offer its own:
/// service active, context resolved, row selected.
///
/// The refusal lives in the reducer's log-target arm, and this is the state
/// where a wrongly-added `LogResourceKind` variant (the transform script's
/// one wrong answer) would first become visible.
#[test]
fn no_log_command_is_offered_even_with_an_account_selected() {
    let mut app = app_showing_accounts();
    let context = app.root.current_context().expect("active context");
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ServiceAccountsLoaded {
            context,
            accounts: vec![account("deployer")],
        }),
    );
    app.root.workspace = Workspace::Resources;
    reduce(&mut app, &Action::Command(CommandAction::Open));

    assert!(
        app.root.resources.selected_service_account().is_some(),
        "the fixture needs a selected account, or the absence below is the \
         no-selection case rather than the no-logs case"
    );
    let offered = app.root.command.visible_commands(&app.root);
    assert!(
        !offered.contains(&CommandDescriptor::OpenLogsForSelected(
            ResourceServiceKind::IamServiceAccounts
        )),
        "an account is an identity that appears in log entries, not a log \
         source; offering the jump would run a query that can only return \
         zero rows"
    );
}
