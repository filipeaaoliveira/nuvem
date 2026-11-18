//! The VPC networks slice of `ResourcesState`: the collection, the sort
//! vocabulary, the filter, and the seams that only the generic machinery
//! reaches (slice 1).
//!
//! The parse is `tests/vpc_service_test.rs` and the painting is
//! `tests/vpc_networks_rendering_test.rs`; neither is re-asserted here. What
//! this file covers is the layer between them, and specifically the three
//! places the mutation sweep found nothing was looking:
//!
//! * the `ResourceService` impl's `matches_filter`, which no *production*
//!   caller reaches for this service — `visible_vpc_networks` calls the free
//!   function directly, and the generic `visible_rows::<S>` path is only
//!   entered by the marks machinery, which networks have no lifecycle action
//!   to use. The trait still demands the method, so it is asserted through
//!   the trait rather than left to be believed;
//! * the `from_column`/`column` pair, whose arms the rendering tests exercised
//!   only for `SUBNETS`; and
//! * `visible_vpc_network_count`, which is the `filtered/total` the row
//!   counter paints and which nothing else reads.

use nuvem::app::state::resources::{
    ResourceService, ResourcesState, VpcNetworkSortField, VpcNetworks,
};
use nuvem_domain::gcp::configuration::{GcloudConfigurationName, GcloudContext};
use nuvem_domain::gcp::diagnostics::{GcloudDiagnostic, GcloudErrorKind};
use nuvem_domain::gcp::filter::compile_filter;
use nuvem_domain::gcp::resource::{ResourceServiceKind, VpcNetwork, VpcRoutingMode, VpcSubnetMode};

fn ctx() -> GcloudContext {
    GcloudContext {
        configuration: GcloudConfigurationName::new("dev"),
        account: None,
        project: Some("dev-project".into()),
        region: None,
        zone: None,
    }
}

fn network(name: &str, mode: VpcSubnetMode, subnets: usize) -> VpcNetwork {
    VpcNetwork {
        name: name.to_string(),
        subnet_mode: mode,
        routing_mode: VpcRoutingMode::Regional,
        subnet_count: subnets,
        description: None,
    }
}

fn state_with(networks: Vec<VpcNetwork>) -> ResourcesState {
    let mut state = ResourcesState::default();
    state.select_service(ResourceServiceKind::VpcNetworks);
    state.set_vpc_networks(ctx(), networks);
    state
}

fn three_networks() -> Vec<VpcNetwork> {
    vec![
        network("default", VpcSubnetMode::Auto, 43),
        network("prod-vpc", VpcSubnetMode::Custom, 7),
        network("prod-vpc-dr", VpcSubnetMode::Custom, 0),
    ]
}

// --- The trait seam ---

/// `VpcNetworks::matches_filter` survived replacement by both `true` and
/// `false`.
///
/// Nothing production-side reaches it: `visible_vpc_networks` calls the free
/// `network_matches_filter`, and the generic `visible_rows::<S>` that would go
/// through the trait is entered only by the marks machinery, which dispatches
/// to Compute and Cloud SQL alone. So the impl is an obligation of the trait
/// with no caller — and an obligation nobody checks is one a later refactor
/// can quietly get wrong, the moment some generic helper *does* start
/// dispatching by kind.
///
/// Both constants are excluded by asserting a match and a non-match through
/// the trait itself, on the same row.
#[test]
fn the_resource_service_impl_filters_rather_than_answering_a_constant() {
    let row = network("prod-vpc", VpcSubnetMode::Custom, 7);
    let matching = compile_filter("prod").expect("a literal needle compiles");
    let missing = compile_filter("staging").expect("a literal needle compiles");

    assert!(
        VpcNetworks::matches_filter(&row, &matching),
        "a needle in the name must match through the trait, not only through \
         the free function"
    );
    assert!(
        !VpcNetworks::matches_filter(&row, &missing),
        "a needle in no searchable field must not match: an impl answering \
         `true` would show every row under every filter"
    );
}

/// The trait's filter reaches the *derived* labels too, which is the half a
/// searchable set built from the wire shape would miss: `CUSTOM` is a rendered
/// word and the row carries a boolean.
#[test]
fn the_resource_service_impl_filters_on_the_rendered_mode() {
    let custom = network("prod-vpc", VpcSubnetMode::Custom, 7);
    let auto = network("default", VpcSubnetMode::Auto, 43);
    let needle = compile_filter("CUSTOM").expect("a literal needle compiles");

    assert!(VpcNetworks::matches_filter(&custom, &needle));
    assert!(!VpcNetworks::matches_filter(&auto, &needle));
}

// --- The sort vocabulary ---

/// Every catalogue index maps to its field and back.
///
/// The rendering tests drive the column cursor onto `SUBNETS` only, so the
/// other three arms of `from_column` were unexercised and both it and
/// `column` survived mutation — `column` could answer a constant `1` for
/// every field and nothing noticed. The round trip pins the pair in both
/// directions at once: a deleted arm breaks the forward leg, a constant
/// return breaks the reverse.
#[test]
fn every_column_index_round_trips_through_its_sort_field() {
    let expected = [
        (0, VpcNetworkSortField::Name),
        (1, VpcNetworkSortField::Mode),
        (2, VpcNetworkSortField::Routing),
        (3, VpcNetworkSortField::Subnets),
    ];

    for (index, field) in expected {
        assert_eq!(
            VpcNetworkSortField::from_column(index),
            Some(field),
            "catalogue index {index} must resolve to {field:?}"
        );
        assert_eq!(
            field.column(),
            index,
            "{field:?} must report the index it was resolved from"
        );
    }

    assert_eq!(
        VpcNetworkSortField::COLUMN_COUNT,
        expected.len(),
        "the table has one sortable field per column, because it carries no \
         labels cell"
    );
    assert_eq!(
        VpcNetworkSortField::from_column(VpcNetworkSortField::COLUMN_COUNT),
        None,
        "an index past the catalogue is not a column and sorts by nothing"
    );
}

/// Each field's label, which is what the table's sort indicator paints.
#[test]
fn every_sort_field_has_its_own_label() {
    let labels = [
        VpcNetworkSortField::Name.label(),
        VpcNetworkSortField::Mode.label(),
        VpcNetworkSortField::Routing.label(),
        VpcNetworkSortField::Subnets.label(),
    ];

    assert_eq!(labels, ["Name", "Mode", "Routing", "Subnets"]);
}

// --- The visible count ---

/// `visible_vpc_network_count` survived replacement by both `0` and `1`.
///
/// It is the `filtered`/`total` the row counter paints, and nothing else
/// reads it — every other assertion walks `visible_vpc_networks` instead.
/// Both constants are excluded by asserting a count that is neither, and the
/// filtered case pins that it counts what survives the filter rather than the
/// whole collection.
#[test]
fn the_visible_count_counts_the_rows_the_filter_leaves() {
    let mut state = state_with(three_networks());

    assert_eq!(
        state.visible_vpc_network_count(),
        3,
        "with no filter applied every loaded network is visible"
    );

    state.start_filter();
    for character in "prod".chars() {
        state.append_filter_char(character);
    }
    state.apply_filter();

    assert_eq!(
        state.visible_vpc_network_count(),
        2,
        "the count must follow the filter, or the row counter contradicts the \
         rows the table paints"
    );
    assert_eq!(
        state.visible_vpc_network_count(),
        state.visible_vpc_networks().len(),
        "the count and the list must never disagree: they are two readings of \
         one filter"
    );
}

// --- The cursor across a reload ---

/// A refresh keeps the cursor on the network it was on, not on the row index.
///
/// `set_vpc_networks` compares the previously selected *name* against the
/// freshly loaded list; the comparison survived inversion, because nothing
/// reloaded a list whose ordering had changed. This is the failure that
/// matters on a poll: rows arrive in whatever order the API returns them, and
/// an operator reading a row watches a different network slide under their
/// cursor between one refresh and the next.
#[test]
fn a_reload_keeps_the_cursor_on_the_same_network_not_the_same_index() {
    let mut state = state_with(three_networks());

    // Move to `prod-vpc`: the default ordering is by name, so the rows read
    // default, prod-vpc, prod-vpc-dr and the cursor lands on index 1.
    state.select_next();
    assert_eq!(
        state
            .selected_vpc_network()
            .map(|network| network.name.as_str()),
        Some("prod-vpc"),
        "precondition: the cursor starts on the network this test follows"
    );

    // A reload that inserts a row *before* it, so the index it used to be at
    // now names a different network.
    let mut reloaded = three_networks();
    reloaded.push(network("aaa-first", VpcSubnetMode::Auto, 1));
    state.set_vpc_networks(ctx(), reloaded);

    assert_eq!(
        state
            .selected_vpc_network()
            .map(|network| network.name.as_str()),
        Some("prod-vpc"),
        "the cursor follows the network by name; keeping the index would land \
         it on a different row"
    );
}

/// A network that is gone after the reload leaves the cursor in range rather
/// than pointing past the end.
#[test]
fn a_reload_that_drops_the_selected_network_clamps_the_cursor() {
    let mut state = state_with(three_networks());
    state.select_next();
    state.select_next();

    state.set_vpc_networks(ctx(), vec![network("only-one", VpcSubnetMode::Auto, 1)]);

    assert_eq!(
        state
            .selected_vpc_network()
            .map(|network| network.name.as_str()),
        Some("only-one"),
        "a cursor past the end of the new list is clamped into it"
    );
}

// --- The problem accessor ---

/// `vpc_networks_problem` survived replacement by `None`.
///
/// The only assertion reading it was a *negative* one — that another
/// configuration's refusal does not become this table's problem — which a
/// constant `None` satisfies trivially. The positive case is what makes that
/// negative mean anything: a refusal for the active context has to be
/// reportable, or the table fails silently.
///
/// # The general rule, because this is a class and not an incident
///
/// **A negative assertion is only worth anything beside a positive one that
/// proves the thing it denies is possible at all.** "This is empty" over a
/// field that is *always* empty is a tautology wearing a test's clothes, and
/// it passes just as happily when the accessor has been reduced to a
/// constant. The pair is the unit: assert that the field can be filled, then
/// assert what does not fill it.
///
/// This repo has shipped the same shape once before in its documentation
/// pins, where an `absent` needle with no matching `presence` needle left a
/// deleted sentence undetected. A service adding a `problem` field
/// should copy both halves of this, not the negative one.
#[test]
fn a_failure_for_this_context_becomes_the_tables_problem() {
    let mut state = state_with(three_networks());
    assert!(
        state.vpc_networks_problem().is_none(),
        "precondition: a loaded table has no problem to report"
    );

    state.set_service_diagnostic(
        ResourceServiceKind::VpcNetworks,
        GcloudDiagnostic::new(
            GcloudErrorKind::PermissionDenied,
            "VPC networks",
            None,
            None,
            "compute.networks.list denied on dev-project",
        ),
    );

    let problem = state
        .vpc_networks_problem()
        .expect("a diagnostic set for this service is reportable through it");
    assert_eq!(
        problem.kind,
        GcloudErrorKind::PermissionDenied,
        "the refusal reaches the table as the kind it was, so the operator is \
         told to fix a permission rather than to retry"
    );
}

/// A filter matching nothing leaves the count at zero while the collection
/// keeps its rows — the state behind the "no networks match" pane.
#[test]
fn a_filter_matching_nothing_empties_the_count_but_not_the_collection() {
    let mut state = state_with(three_networks());

    state.start_filter();
    for character in "zzz".chars() {
        state.append_filter_char(character);
    }
    state.apply_filter();

    assert_eq!(state.visible_vpc_network_count(), 0);
    assert_eq!(
        state.vpc_networks().len(),
        3,
        "narrowing hides rows; it does not discard them"
    );
}
