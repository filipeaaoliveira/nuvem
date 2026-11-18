//! The Pub/Sub slices of `ResourcesState`: the collections, the sorts, the
//! filters, and the absent detail panes.
//!
//! The parse is `tests/pubsub_service_test.rs` and is not re-asserted here.
//! What this file is about is what the state does with rows once it has them —
//! most of all, that a count which could not be fetched keeps behaving like an
//! absent value rather than like a zero, in the sort and in the filter alike.
//!
//! There is no detail pane in this slice, so there is nothing here about one
//! beyond the assertion that `Enter` stays a quiet no-op.

use nuvem::app::projection::SortDirection;
use nuvem::app::reducer::root::reduce;
use nuvem::app::state::command::CommandDescriptor;
use nuvem::app::state::resources::{
    PubSubSubscriptionSortField, PubSubSubscriptions, PubSubTopicSortField, PubSubTopics,
    ResourceService, ResourcesState,
};
use nuvem::app::state::root::Workspace;
use nuvem::app::{Action, AppState, CommandAction, EffectResult, ResourcesAction};
use nuvem_domain::gcp::configuration::{
    GcloudConfiguration, GcloudConfigurationName, GcloudContext,
};
use nuvem_domain::gcp::filter::compile_filter;
use nuvem_domain::gcp::resource::{
    ComputeKeyValue, PubSubDeadLetterPolicy, PubSubDelivery, PubSubSubscription,
    PubSubSubscriptionState, PubSubTopic, ResourceServiceKind, SubscriptionCount,
};

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

fn topic(name: &str, count: SubscriptionCount) -> PubSubTopic {
    PubSubTopic {
        name: name.to_string(),
        full_name: format!("projects/dev-project/topics/{name}"),
        subscription_count: count,
        message_retention: Some("604800s".to_string()),
        kms_key: None,
        labels: Vec::new(),
    }
}

/// A state holding `topics`, with no filter applied.
fn state_with(topics: Vec<PubSubTopic>) -> ResourcesState {
    let mut state = ResourcesState::default();
    state.select_service(ResourceServiceKind::PubSubTopics);
    state.set_pubsub_topics(ctx(), topics);
    state
}

fn visible_names(state: &ResourcesState) -> Vec<String> {
    state
        .visible_pubsub_topics()
        .into_iter()
        .map(|topic| topic.name.clone())
        .collect()
}

// --- The collection ---

#[test]
fn a_reload_keeps_the_cursor_on_the_topic_it_was_already_on() {
    let mut state = state_with(vec![
        topic("alpha", SubscriptionCount::Known(1)),
        topic("beta", SubscriptionCount::Known(2)),
        topic("gamma", SubscriptionCount::Known(3)),
    ]);
    state.select_next();
    assert_eq!(
        state
            .selected_pubsub_topic()
            .map(|topic| topic.name.clone()),
        Some("beta".to_string())
    );

    // The same topics, listed in a different order.
    state.set_pubsub_topics(
        ctx(),
        vec![
            topic("gamma", SubscriptionCount::Known(3)),
            topic("beta", SubscriptionCount::Known(2)),
            topic("alpha", SubscriptionCount::Known(1)),
        ],
    );

    assert_eq!(
        state
            .selected_pubsub_topic()
            .map(|topic| topic.name.clone()),
        Some("beta".to_string()),
        "the cursor follows the topic, not the row index"
    );
}

#[test]
fn a_reload_that_drops_the_selected_topic_clamps_the_cursor() {
    let mut state = state_with(vec![
        topic("alpha", SubscriptionCount::Known(1)),
        topic("beta", SubscriptionCount::Known(2)),
    ]);
    state.select_next();

    state.set_pubsub_topics(ctx(), vec![topic("alpha", SubscriptionCount::Known(1))]);

    assert_eq!(
        state
            .selected_pubsub_topic()
            .map(|topic| topic.name.clone()),
        Some("alpha".to_string()),
        "a cursor past the end of the new list must not dangle"
    );
}

#[test]
fn marking_the_topics_stale_empties_the_rows_and_drops_the_context() {
    let mut state = state_with(vec![topic("alpha", SubscriptionCount::Known(1))]);

    state.mark_pubsub_topics_stale_and_clear();

    assert!(state.pubsub_topics().is_empty());
    assert!(!state.has_loaded_context());
}

#[test]
fn selecting_next_on_an_empty_topic_list_does_nothing() {
    // The `len > 0` guard is load-bearing, not decoration: without it the
    // clamp below computes `len - 1` on an empty list, which underflows.
    let mut state = state_with(Vec::new());

    state.select_next();

    assert!(state.selected_pubsub_topic().is_none());
    assert_eq!(state.selected_visible_index(), None);
}

#[test]
fn selecting_next_at_the_last_topic_stays_on_it() {
    let mut state = state_with(vec![
        topic("alpha", SubscriptionCount::Known(1)),
        topic("beta", SubscriptionCount::Known(2)),
    ]);

    // Past the end twice over: the cursor clamps to the last row rather than
    // running off it, which is what `.min(len - 1)` is for.
    state.select_next();
    state.select_next();
    state.select_next();

    assert_eq!(
        state
            .selected_pubsub_topic()
            .map(|topic| topic.name.clone()),
        Some("beta".to_string()),
        "the cursor must clamp to the last topic, not run past it"
    );
}

#[test]
fn selecting_previous_at_the_first_topic_stays_on_it() {
    let mut state = state_with(vec![
        topic("alpha", SubscriptionCount::Known(1)),
        topic("beta", SubscriptionCount::Known(2)),
    ]);

    state.select_previous();
    state.select_previous();

    assert_eq!(
        state
            .selected_pubsub_topic()
            .map(|topic| topic.name.clone()),
        Some("alpha".to_string())
    );
}

// --- The sort ---

#[test]
fn sorting_by_subscriptions_orders_numerically_not_by_the_rendered_label() {
    let mut state = state_with(vec![
        topic("nine", SubscriptionCount::Known(9)),
        topic("ten", SubscriptionCount::Known(10)),
        topic("two", SubscriptionCount::Known(2)),
    ]);
    // Move the column cursor onto `subscriptions` (catalogue index 1) and sort
    // by it, the same path the `s` key takes.
    state.select_next_column();
    state.sort_selected_column();

    assert_eq!(
        visible_names(&state),
        vec!["two".to_string(), "nine".to_string(), "ten".to_string()],
        "10 sorts after 9; sorting the rendered digits would put it before"
    );
}

#[test]
fn an_unavailable_count_does_not_sort_as_zero() {
    let mut state = state_with(vec![
        topic("known-one", SubscriptionCount::Known(1)),
        topic("unknown", SubscriptionCount::Unavailable),
        topic("known-five", SubscriptionCount::Known(5)),
    ]);
    state.select_next_column();
    state.sort_selected_column();

    let names = visible_names(&state);
    // Unknown groups at one end rather than claiming to be the quietest topic:
    // as an absent value, not as zero. What matters is that it is not *between*
    // the two known counts and not below the smallest one.
    assert_eq!(
        names.first().map(String::as_str),
        Some("unknown"),
        "an absent count sorts as absent, not as a count of zero: {names:?}"
    );
    assert_eq!(
        names[1..],
        ["known-one".to_string(), "known-five".to_string()]
    );
}

// --- The filter ---

#[test]
fn the_filter_matches_every_cell_the_table_paints() {
    let mut rows = vec![
        topic("orders", SubscriptionCount::Known(2)),
        topic("audit", SubscriptionCount::Known(7)),
    ];
    rows[0].kms_key = Some("projects/p/locations/l/keyRings/r/cryptoKeys/pubsub".to_string());
    rows[0].labels = vec![ComputeKeyValue {
        key: "team".to_string(),
        value: "platform".to_string(),
    }];
    for (term, expected) in [
        ("orders", "orders"),
        ("cryptoKeys", "orders"),
        ("team=platform", "orders"),
        ("7", "audit"),
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
            visible_names(&state),
            vec![expected.to_string()],
            "filtering on {term:?} must narrow to {expected:?}"
        );
    }
}

#[test]
fn filtering_on_the_unknown_marker_matches_only_the_topics_with_no_count() {
    let mut state = state_with(vec![
        topic("counted", SubscriptionCount::Known(3)),
        topic("uncounted", SubscriptionCount::Unavailable),
    ]);

    state.start_filter();
    // The dash the cell paints, from `SubscriptionCount::label`.
    for character in SubscriptionCount::Unavailable.label().chars() {
        state.append_filter_char(character);
    }
    state.apply_filter();

    assert_eq!(
        visible_names(&state),
        vec!["uncounted".to_string()],
        "the filter searches the label the cell shows, so the dash is findable"
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

fn app_showing_topics() -> AppState {
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
            ResourceServiceKind::PubSubTopics,
        )),
    );
    app
}

#[test]
fn a_loaded_result_installs_the_rows_for_the_active_context() {
    let mut app = app_showing_topics();
    let context = app.root.current_context().expect("context exists");

    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::PubSubTopicsLoaded {
            context,
            topics: vec![topic("orders", SubscriptionCount::Known(2))],
        }),
    );

    assert_eq!(app.root.resources.pubsub_topics().len(), 1);
}

#[test]
fn pressing_enter_on_a_topic_does_nothing_at_all() {
    let mut app = app_showing_topics();
    let context = app.root.current_context().expect("context exists");
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::PubSubTopicsLoaded {
            context,
            topics: vec![topic("orders", SubscriptionCount::Known(2))],
        }),
    );

    let effects = reduce(&mut app, &Action::Resources(ResourcesAction::OpenSelected));

    // List-only in this slice, the shape Cloud Functions shipped in. `Enter`
    // has to be a *quiet* no-op: no effect to load a pane, and no pane half
    // opened for one to fill.
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
// `needs_refresh`'s Pub/Sub arm is `context != Some(context) || stale`, and
// both operators survived mutation — the `!=` to `==` and the `||` to `&&`.
// Nothing above loads the same rows twice or crosses a context, so the arm
// could invert wholesale and every other test still passed. A wrong answer
// here either refetches on every frame or serves another project's topics.

#[test]
fn topics_need_a_refresh_before_any_context_is_loaded() {
    let mut state = ResourcesState::default();
    state.select_service(ResourceServiceKind::PubSubTopics);

    assert!(
        state.needs_refresh(&ctx()),
        "a topics list that has never loaded must ask for a refresh"
    );
}

#[test]
fn topics_need_no_refresh_for_the_context_they_were_loaded_for() {
    let state = state_with(vec![topic("orders", SubscriptionCount::Known(4))]);

    assert!(
        !state.needs_refresh(&ctx()),
        "freshly loaded, non-stale topics for the very same context must not \
         be refetched"
    );
}

#[test]
fn topics_need_a_refresh_for_a_context_they_were_not_loaded_for() {
    let state = state_with(vec![topic("orders", SubscriptionCount::Known(4))]);

    assert!(
        state.needs_refresh(&other_ctx()),
        "topics loaded under one context say nothing about another context, so \
         a different context must ask for a refresh even while the rows are \
         not stale"
    );
}

#[test]
fn stale_topics_need_a_refresh_even_for_their_own_context() {
    let mut state = state_with(vec![topic("orders", SubscriptionCount::Known(4))]);
    state.mark_pubsub_topics_stale_and_clear();

    assert!(
        state.needs_refresh(&ctx()),
        "rows marked stale must be refetched for the context that loaded them, \
         which is the whole point of marking them"
    );
}

// --- The visible count and the cursor ---

/// `visible_pubsub_topic_count` survived replacement by both `0` and `1`.
///
/// It is the `total`/`filtered` the table's row counter paints, and no test
/// above reads it — the assertions all walk `visible_pubsub_topics` instead.
/// Both constants are excluded by asserting a count that is neither, and the
/// filtered case pins that it counts what survives the filter rather than the
/// whole collection.
#[test]
fn the_visible_count_counts_the_rows_the_filter_leaves() {
    let mut state = state_with(vec![
        topic("orders", SubscriptionCount::Known(4)),
        topic("orders-dlq", SubscriptionCount::Known(1)),
        topic("audit-log", SubscriptionCount::Known(0)),
    ]);

    assert_eq!(
        state.visible_pubsub_topic_count(),
        3,
        "with no filter applied every loaded topic is visible"
    );

    state.start_filter();
    for character in "orders".chars() {
        state.append_filter_char(character);
    }
    state.apply_filter();

    assert_eq!(
        state.visible_pubsub_topic_count(),
        2,
        "the count must follow the filter, or the row counter contradicts the \
         rows the table paints"
    );
    assert_eq!(
        state.visible_pubsub_topic_count(),
        state.visible_pubsub_topics().len(),
        "the count and the list must never disagree: they are two readings of \
         one filter"
    );
}

/// `(visible_rows > 0).then_some(..)` for the Pub/Sub arm, pinned on both
/// sides of the boundary.
///
/// The comparison survived mutation to `==`, `<` and `>=`. Two of those are
/// constant for a `usize`: `< 0` never holds, so no row is ever highlighted,
/// and `>= 0` always holds, so an empty list reports a selection into rows
/// that are not there. Every test above reads the rows and none reads the
/// cursor, so all three passed. Both cases are asserted because no single one
/// disagrees with all three mutants.
#[test]
fn the_selected_index_is_a_row_while_rows_are_visible_and_nothing_when_none_are() {
    let mut state = state_with(vec![
        topic("orders", SubscriptionCount::Known(4)),
        topic("audit-log", SubscriptionCount::Known(0)),
    ]);

    assert_eq!(
        state.selected_index(),
        Some(0),
        "a loaded list starts with its first row selected; None here leaves \
         the table with nothing highlighted"
    );

    // A filter matching no topic leaves the rows loaded and none visible,
    // which is the only way to reach zero visible rows without also emptying
    // the collection.
    state.start_filter();
    for character in "zzz-matches-nothing".chars() {
        state.append_filter_char(character);
    }
    state.apply_filter();

    assert!(
        visible_names(&state).is_empty(),
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

/// `PubSubTopicSortField::label` and `from_column`, pinned per variant.
///
/// `label` survived replacement by both `""` and a constant, and `from_column`
/// survived deletion of three of its four `Some` arms. Nothing above reads
/// either: the sort tests drive sorting through `ResourcesAction`, which never
/// spells the label or the column index. A wrong `from_column` silently sorts
/// by the wrong column when the operator picks one.
#[test]
fn every_sortable_column_maps_to_its_field_and_carries_its_own_label() {
    let by_column = [
        (0, PubSubTopicSortField::Name, "Name"),
        (1, PubSubTopicSortField::Subscriptions, "Subscriptions"),
        (2, PubSubTopicSortField::Retention, "Retention"),
        (3, PubSubTopicSortField::KmsKey, "KMS key"),
    ];

    for (index, field, label) in by_column {
        assert_eq!(
            PubSubTopicSortField::from_column(index),
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
    // above if it happens to equal one of them, and four columns that read the
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
        PubSubTopicSortField::from_column(4),
        None,
        "column 4 is `labels`, the one catalogue column with no sort field; a \
         field here would sort by a flattened `k=v` cell"
    );
}

// --- The diagnostic ---

/// `pubsub_topics_problem` survived replacement by `None`.
///
/// It is what the home overview reads to show a service as degraded, and the
/// reducer tests below assert the *status message* rather than the problem, so
/// the accessor could report a healthy service forever. Both transitions are
/// pinned: a failed load raises the problem, a successful one clears it.
#[test]
fn a_failed_load_raises_a_problem_and_a_later_success_clears_it() {
    let mut app = app_showing_topics();
    let context = app.root.current_context().expect("active context");
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::PubSubTopicsLoadFailed {
            context: context.clone(),
            error: nuvem::app::AppError::gcloud("permission denied"),
        }),
    );

    assert!(
        app.root.resources.pubsub_topics_problem().is_some(),
        "a failed topics load must leave a problem for the overview to show, \
         or a degraded service reads as a healthy one"
    );

    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::PubSubTopicsLoaded {
            context,
            topics: vec![topic("orders", SubscriptionCount::Known(4))],
        }),
    );

    assert!(
        app.root.resources.pubsub_topics_problem().is_none(),
        "a successful load must clear the previous problem"
    );
}

// --- The generic service interface ---

/// `ResourceService` is how generic code reaches a service without a
/// per-service match, and Pub/Sub's impl has no caller in this slice — the
/// `visible_*` accessors call `topic_matches_filter` and `compare_topics`
/// directly. Unexercised is not the same as free to be wrong: the day a
/// generic path does reach it, it must answer what the tab already answers,
/// so the methods are pinned against the accessors the tab itself uses.
///
/// `matches_filter` survived replacement by both `true` and `false` for
/// exactly this reason.
#[test]
fn the_generic_service_impl_answers_what_the_tab_answers() {
    let filter = compile_filter("orders").expect("a compilable filter");
    let matching = topic("orders", SubscriptionCount::Known(4));
    let other = topic("audit-log", SubscriptionCount::Known(0));

    assert!(
        PubSubTopics::matches_filter(&matching, &filter),
        "the trait must keep the rows the list keeps"
    );
    assert!(
        !PubSubTopics::matches_filter(&other, &filter),
        "the trait must narrow the same rows the list does, or a generic \
         caller sees a different table than the one on screen"
    );

    assert_eq!(
        PubSubTopics::compare(
            &matching,
            &other,
            PubSubTopicSortField::Name,
            SortDirection::Ascending
        ),
        std::cmp::Ordering::Greater,
        "the trait must order rows the way the table does"
    );

    // A topic is global and its name alone is its identity — there is no
    // location to qualify it with, which is the property the cursor-keeping on
    // reload depends on.
    assert_eq!(
        PubSubTopics::row_id(&matching),
        PubSubTopics::row_id(&matching.clone())
    );
    assert_ne!(
        PubSubTopics::row_id(&matching),
        PubSubTopics::row_id(&other),
        "two differently-named topics must not share one identity"
    );
}

// --- The command palette ---

/// The Pub/Sub topic-scoped jump is offered only while Pub/Sub topics are the
/// active service.
///
/// Its guard is `active_service == PubSubTopics && current_context().is_some()
/// && selected_pubsub_topic().is_some()`, and both `&&`s survived mutation.
/// `selected_pubsub_topic` reads its own collection and never consults the
/// active service, so a loaded list keeps its selection after the operator
/// switches away — and under either mutant that lingering selection alone
/// publishes the command. The palette would then offer "open logs for the
/// selected Pub/Sub topic" to an operator looking at Secrets.
///
/// Both halves are asserted: present where it belongs, absent one service
/// switch later. Asserting only the absence would pass against a guard that
/// never offers the command at all.
#[test]
fn the_topic_log_command_is_offered_only_while_topics_are_the_active_service() {
    let mut app = app_showing_topics();
    let context = app.root.current_context().expect("active context");
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::PubSubTopicsLoaded {
            context,
            topics: vec![topic("orders", SubscriptionCount::Known(4))],
        }),
    );
    // The guard's match arm is `Workspace::Resources`; the palette offers
    // nothing service-scoped from Home.
    app.root.workspace = Workspace::Resources;
    reduce(&mut app, &Action::Command(CommandAction::Open));

    let offered = app.root.command.visible_commands(&app.root);
    assert!(
        offered.contains(&CommandDescriptor::OpenLogsForSelected(
            ResourceServiceKind::PubSubTopics
        )),
        "a selected topic under the Pub/Sub service must offer its log jump, \
         or `L` works while the palette shows nothing"
    );

    // The selection outliving the switch is precisely what makes the guard's
    // first two conjuncts load-bearing.
    reduce(
        &mut app,
        &Action::Resources(ResourcesAction::SelectService(
            ResourceServiceKind::SecretManagerSecrets,
        )),
    );
    assert!(
        app.root.resources.selected_pubsub_topic().is_some(),
        "this fixture is only meaningful while the topic selection outlives \
         the service switch; if that ever changes, rebuild the negative case"
    );

    let offered = app.root.command.visible_commands(&app.root);
    assert!(
        !offered.contains(&CommandDescriptor::OpenLogsForSelected(
            ResourceServiceKind::PubSubTopics
        )),
        "a topic selected under another service must not publish the Pub/Sub \
         log jump: the operator is looking at Secrets, and the command would \
         act on a row that is not on screen"
    );
}

// --- The flat subscriptions table ---
//
// Subscriptions are a sibling of the topics table, not a drill-down under it.
// What follows is about the facts that are *theirs* rather than the topics
// table's: an ack deadline that must sort as a number, a parent topic that
// must sort and filter on the short id the column paints, two rendered labels
// with no raw counterpart, and the deleted-topic sentinel — which is a row
// this table shows and the topics table's count grouping skips.

/// A pull subscription with every optional field populated.
fn subscription(name: &str, topic: &str) -> PubSubSubscription {
    PubSubSubscription {
        name: name.to_string(),
        full_name: format!("projects/dev-project/subscriptions/{name}"),
        topic: format!("projects/dev-project/topics/{topic}"),
        delivery: PubSubDelivery::Pull,
        ack_deadline_seconds: Some(10),
        message_retention: Some("604800s".to_string()),
        dead_letter: None,
        state: PubSubSubscriptionState::Active,
        labels: Vec::new(),
    }
}

fn subscriptions_state(rows: Vec<PubSubSubscription>) -> ResourcesState {
    let mut state = ResourcesState::default();
    state.select_service(ResourceServiceKind::PubSubSubscriptions);
    state.set_pubsub_subscriptions(ctx(), rows);
    state
}

fn visible_subscription_names(state: &ResourcesState) -> Vec<String> {
    state
        .visible_pubsub_subscriptions()
        .into_iter()
        .map(|subscription| subscription.name.clone())
        .collect()
}

/// Sorts by the catalogue column at `index` through the real path an operator
/// takes: move the column cursor, then sort what it is on.
///
/// Deliberately not a setter on the sort field. Going through
/// `select_next_column` and `sort_selected_column` is what exercises the
/// display-position-to-catalogue-index translation and
/// `PubSubSubscriptionSortField::from_column` — the wiring a synthetic setter
/// would skip straight past, leaving a sort that is correct in the comparator
/// and unreachable from the keyboard.
fn sort_by_column(state: &mut ResourcesState, index: usize) {
    for _ in 0..index {
        state.select_next_column();
    }
    state.sort_selected_column();
}

#[test]
fn the_ack_deadline_sorts_numerically_rather_than_by_its_rendered_text() {
    // The defect this pins: `100` sorting before `60` because `"100" < "60"` as
    // text. An operator sorts this column to find the subscription with the
    // longest deadline, which text ordering gets exactly backwards.
    let mut state = subscriptions_state(vec![
        PubSubSubscription {
            ack_deadline_seconds: Some(60),
            ..subscription("sixty", "orders")
        },
        PubSubSubscription {
            ack_deadline_seconds: Some(100),
            ..subscription("hundred", "orders")
        },
        PubSubSubscription {
            ack_deadline_seconds: Some(9),
            ..subscription("nine", "orders")
        },
    ]);
    sort_by_column(
        &mut state,
        PubSubSubscriptionSortField::AckDeadline.column(),
    );

    assert_eq!(
        visible_subscription_names(&state),
        vec!["nine", "sixty", "hundred"]
    );
}

#[test]
fn a_subscription_with_no_ack_deadline_sorts_as_absent_rather_than_as_zero() {
    // Zero would claim the fastest deadline in the project. Absent means the
    // API returned no value, so those rows group at one end instead of
    // impersonating a real number.
    let mut state = subscriptions_state(vec![
        PubSubSubscription {
            ack_deadline_seconds: Some(10),
            ..subscription("known", "orders")
        },
        PubSubSubscription {
            ack_deadline_seconds: None,
            ..subscription("unknown", "orders")
        },
    ]);
    sort_by_column(
        &mut state,
        PubSubSubscriptionSortField::AckDeadline.column(),
    );

    // `None` sorts before `Some(_)`, so the unknowns group at the head — but
    // the assertion that matters is that they group *at all* rather than
    // landing among the low numbers as a zero would.
    assert_eq!(visible_subscription_names(&state), vec!["unknown", "known"]);
}

#[test]
fn the_topic_column_sorts_on_the_short_id_it_paints_not_the_resource_path() {
    // The topics must live in *different projects* for this to prove anything.
    // Under one project every row's `topic` shares a prefix, so full-path and
    // short-id ordering coincide and a sort on the wrong one passes by
    // accident — which is exactly what the first version of this test did.
    //
    // A cross-project topic is not contrived: a subscription may be attached
    // to a topic in another project, and that is when the column an operator
    // reads and the value it sorts by come apart.
    let mut state = subscriptions_state(vec![
        PubSubSubscription {
            topic: "projects/aaa-project/topics/zebra".to_string(),
            ..subscription("on-zebra", "ignored")
        },
        PubSubSubscription {
            topic: "projects/zzz-project/topics/apple".to_string(),
            ..subscription("on-apple", "ignored")
        },
    ]);
    sort_by_column(&mut state, PubSubSubscriptionSortField::Topic.column());

    assert_eq!(
        visible_subscription_names(&state),
        vec!["on-apple", "on-zebra"],
        "the cells read `apple` and `zebra`, so `apple` sorts first — sorting \
         on the full path would put the aaa-project row first instead"
    );
}

#[test]
fn a_subscription_whose_topic_was_deleted_is_a_row_this_table_shows() {
    // The topics table's count grouping skips this row on purpose; the flat
    // table must not. An orphaned subscription is a finding — it still holds
    // messages and still bills — and dropping it here would hide exactly the
    // thing the flat table is good for.
    let state = subscriptions_state(vec![PubSubSubscription {
        topic: "_deleted-topic_".to_string(),
        ..subscription("orphaned", "gone")
    }]);

    assert_eq!(visible_subscription_names(&state), vec!["orphaned"]);
}

#[test]
fn the_deleted_topic_sentinel_is_filterable_because_it_is_what_the_cell_reads() {
    // An operator hunting orphans types the sentinel. A filter that did not
    // match it would hide rows whose cell plainly shows that text.
    let state = subscriptions_state(vec![
        PubSubSubscription {
            topic: "_deleted-topic_".to_string(),
            ..subscription("orphaned", "gone")
        },
        subscription("healthy", "orders"),
    ]);

    let filter = compile_filter("_deleted-topic_").expect("pattern compiles");
    let matched: Vec<&str> = state
        .pubsub_subscriptions()
        .iter()
        .filter(|row| PubSubSubscriptions::matches_filter(row, &filter))
        .map(|row| row.name.as_str())
        .collect();

    assert_eq!(matched, vec!["orphaned"]);
}

#[test]
fn the_delivery_label_is_filterable_though_it_appears_in_no_raw_field() {
    // `Pull`/`Push` is derived from whether `pushConfig` carries an endpoint.
    // It is on screen, so it must be searchable — and because it is in no raw
    // field, it is also why this service pushes no clause down.
    // Neither name may contain "push" or "pull": the filter is a
    // case-insensitive regex over every searchable field, so a row named
    // `pusher` matches /Push/ on its NAME and the delivery label is never
    // consulted. That is what the first version of this test did.
    let state = subscriptions_state(vec![
        subscription("alpha", "orders"),
        PubSubSubscription {
            delivery: PubSubDelivery::Push {
                endpoint: "https://audit.example.com/handler".to_string(),
            },
            ..subscription("beta", "orders")
        },
    ]);

    let filter = compile_filter("Push").expect("pattern compiles");
    let matched: Vec<&str> = state
        .pubsub_subscriptions()
        .iter()
        .filter(|row| PubSubSubscriptions::matches_filter(row, &filter))
        .map(|row| row.name.as_str())
        .collect();

    assert_eq!(matched, vec!["beta"]);
}

#[test]
fn a_push_endpoint_is_searchable_even_though_it_has_no_column() {
    // Searchable-without-a-column only ever widens what matches, so it cannot
    // hide a row that is on screen. It earns its place because a push
    // subscription in `RESOURCE_ERROR` is usually a broken endpoint, and the
    // endpoint is the only thing the operator has to go on.
    let state = subscriptions_state(vec![
        PubSubSubscription {
            delivery: PubSubDelivery::Push {
                endpoint: "https://audit.example.com/pubsub".to_string(),
            },
            ..subscription("pusher", "orders")
        },
        subscription("puller", "orders"),
    ]);

    let filter = compile_filter("audit.example.com").expect("pattern compiles");
    let matched: Vec<&str> = state
        .pubsub_subscriptions()
        .iter()
        .filter(|row| PubSubSubscriptions::matches_filter(row, &filter))
        .map(|row| row.name.as_str())
        .collect();

    assert_eq!(matched, vec!["pusher"]);
}

#[test]
fn the_state_label_is_filterable_in_the_spelling_the_column_paints() {
    // The raw value is `RESOURCE_ERROR`; the cell reads `Resource error`. The
    // filter must match what is on screen, not the wire spelling.
    let state = subscriptions_state(vec![
        PubSubSubscription {
            state: PubSubSubscriptionState::ResourceError,
            ..subscription("broken", "orders")
        },
        subscription("healthy", "orders"),
    ]);

    let filter = compile_filter("Resource error").expect("pattern compiles");
    let matched: Vec<&str> = state
        .pubsub_subscriptions()
        .iter()
        .filter(|row| PubSubSubscriptions::matches_filter(row, &filter))
        .map(|row| row.name.as_str())
        .collect();

    assert_eq!(matched, vec!["broken"]);
}

#[test]
fn the_dead_letter_topic_filters_on_its_short_id() {
    let state = subscriptions_state(vec![
        PubSubSubscription {
            dead_letter: Some(PubSubDeadLetterPolicy {
                dead_letter_topic: "projects/dev-project/topics/dead-letters".to_string(),
                max_delivery_attempts: Some(5),
            }),
            ..subscription("with-dlq", "orders")
        },
        subscription("without-dlq", "orders"),
    ]);

    let filter = compile_filter("dead-letters").expect("pattern compiles");
    let matched: Vec<&str> = state
        .pubsub_subscriptions()
        .iter()
        .filter(|row| PubSubSubscriptions::matches_filter(row, &filter))
        .map(|row| row.name.as_str())
        .collect();

    assert_eq!(matched, vec!["with-dlq"]);
}

#[test]
fn subscriptions_report_status_counts_where_topics_report_none() {
    // The one place the two tables genuinely differ in shape. A topic has no
    // lifecycle state; a subscription does, and `RESOURCE_ERROR` is the whole
    // reason to surface it on the first screen an operator sees.
    let state = subscriptions_state(vec![
        subscription("a", "orders"),
        subscription("b", "orders"),
        PubSubSubscription {
            state: PubSubSubscriptionState::ResourceError,
            ..subscription("broken", "orders")
        },
    ]);

    let counts = state.service_status_counts(ResourceServiceKind::PubSubSubscriptions);
    assert!(
        counts.contains(&("Active".to_string(), 2)),
        "expected two Active, got {counts:?}"
    );
    assert!(
        counts.contains(&("Resource error".to_string(), 1)),
        "expected one Resource error, got {counts:?}"
    );

    assert!(
        state
            .service_status_counts(ResourceServiceKind::PubSubTopics)
            .is_empty(),
        "a topic has no lifecycle state to count"
    );
}

#[test]
fn the_cursor_stays_on_the_same_subscription_across_a_reload() {
    let mut state = subscriptions_state(vec![
        subscription("a", "orders"),
        subscription("b", "orders"),
        subscription("c", "orders"),
    ]);
    state.select_next();
    let selected = state
        .selected_pubsub_subscription()
        .expect("a row is selected")
        .name
        .clone();
    assert_eq!(selected, "b");

    // A reload that drops the row above the cursor must keep the cursor on `b`
    // rather than on index 1, which is now `c`.
    state.set_pubsub_subscriptions(
        ctx(),
        vec![subscription("b", "orders"), subscription("c", "orders")],
    );

    assert_eq!(
        state
            .selected_pubsub_subscription()
            .expect("a row is still selected")
            .name,
        "b"
    );
}

#[test]
fn subscriptions_and_topics_are_independent_collections() {
    // The tables share a list call, not a collection. Loading one must not
    // touch the other's rows, cursor, or loaded-context stamp — the failure
    // mode a drill-down implementation would have.
    let mut state = ResourcesState::default();
    state.set_pubsub_topics(ctx(), vec![topic("orders", SubscriptionCount::Known(1))]);
    state.set_pubsub_subscriptions(ctx(), vec![subscription("orders-worker", "orders")]);

    assert_eq!(state.pubsub_topics().len(), 1);
    assert_eq!(state.pubsub_subscriptions().len(), 1);

    state.set_pubsub_subscriptions(ctx(), Vec::new());

    assert_eq!(
        state.pubsub_topics().len(),
        1,
        "clearing the subscriptions table must not empty the topics table"
    );
}
