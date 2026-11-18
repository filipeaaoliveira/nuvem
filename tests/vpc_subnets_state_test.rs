//! The VPC subnets slice of `ResourcesState` (slice 2).
//!
//! The three seams pinned here are the ones slice 1's mutation sweep found
//! unguarded on the networks table, written up front this time rather than
//! discovered one sweep at a time: the `ResourceService` impl's
//! `matches_filter` (no production caller reaches it — the generic
//! `visible_rows::<S>` path is entered only by the marks machinery), the
//! `visible_*_count` accessor (no generic caller at all), and the cursor
//! comparison in `set_*`, which needs a reload whose ordering changed before
//! it means anything.
//!
//! The `column`/`from_column` round trip is **not** here: it is covered for
//! every service, including this one, by `sort_column_invariant_test`.
//!
//! Everything in this file turns on one fact: a subnet's identity is
//! (name, region), because an auto-mode network calls every one of its subnets
//! `default`.

use nuvem::app::state::resources::{
    ResourceService, ResourcesState, VpcSubnetSortField, VpcSubnets,
};
use nuvem_domain::gcp::configuration::{GcloudConfigurationName, GcloudContext};
use nuvem_domain::gcp::diagnostics::{GcloudDiagnostic, GcloudErrorKind};
use nuvem_domain::gcp::filter::compile_filter;
use nuvem_domain::gcp::resource::{ResourceServiceKind, VpcSubnet};

fn ctx() -> GcloudContext {
    GcloudContext {
        configuration: GcloudConfigurationName::new("dev"),
        account: None,
        project: Some("dev-project".into()),
        region: None,
        zone: None,
    }
}

fn subnet(name: &str, region: &str, range: &str) -> VpcSubnet {
    VpcSubnet {
        name: name.to_string(),
        region: region.to_string(),
        network: "default".to_string(),
        ip_cidr_range: range.to_string(),
        gateway_address: Some("10.128.0.1".to_string()),
        private_google_access: false,
    }
}

fn state_with(subnets: Vec<VpcSubnet>) -> ResourcesState {
    let mut state = ResourcesState::default();
    state.select_service(ResourceServiceKind::VpcSubnets);
    state.set_vpc_subnets(ctx(), subnets);
    state
}

/// The captured shape: three rows, one name, three regions.
fn three_default_subnets() -> Vec<VpcSubnet> {
    vec![
        subnet("default", "us-central1", "10.128.0.0/20"),
        subnet("default", "europe-west1", "10.132.0.0/20"),
        subnet("default", "asia-east1", "10.140.0.0/20"),
    ]
}

// --- Identity ---

/// The region is half the row's identity, so the mark id carries it.
///
/// Three rows sharing a name must produce three distinct ids. An id built from
/// the name alone would make them one row as far as the cursor, the marks and
/// the remembered view are concerned.
#[test]
fn three_subnets_sharing_a_name_have_three_distinct_identities() {
    let ids: Vec<_> = three_default_subnets()
        .iter()
        .map(VpcSubnets::row_id)
        .collect();

    assert_eq!(ids.len(), 3);
    for (index, id) in ids.iter().enumerate() {
        assert_eq!(id.name, "default", "all three share the name");
        assert!(
            !id.location.is_empty(),
            "a subnet's identity must carry its region"
        );
        assert!(
            !ids[..index].contains(id),
            "{id:?} identifies two different subnets"
        );
    }
}

// --- The trait seam ---

/// `VpcSubnets::matches_filter` through the trait, not the free function.
///
/// Nothing production-side reaches this impl: `visible_vpc_subnets` calls the
/// free function, and the generic `visible_rows::<S>` path is entered only by
/// the marks machinery, which dispatches to Compute and Cloud SQL alone. It is
/// a trait obligation with no caller, which is the kind a later refactor gets
/// wrong the first time a generic helper starts dispatching by kind. Asserting
/// a match *and* a non-match on one row excludes both constant answers.
#[test]
fn the_resource_service_impl_filters_rather_than_answering_a_constant() {
    let row = subnet("default", "us-central1", "10.128.0.0/20");
    let matching = compile_filter("us-central1").expect("a literal needle compiles");
    let missing = compile_filter("zzz-nowhere").expect("a literal needle compiles");

    assert!(VpcSubnets::matches_filter(&row, &matching));
    assert!(!VpcSubnets::matches_filter(&row, &missing));
}

/// The filter reaches the region, which is what makes the table usable at all.
///
/// With every row named `default`, the region is the *only* thing that
/// distinguishes one row from another — so a searchable set that omitted it
/// would leave an operator unable to narrow this table by anything meaningful.
#[test]
fn the_filter_narrows_by_region_because_the_name_cannot() {
    let state = {
        let mut state = state_with(three_default_subnets());
        state.start_filter();
        for character in "europe".chars() {
            state.append_filter_char(character);
        }
        state.apply_filter();
        state
    };

    let visible: Vec<&str> = state
        .visible_vpc_subnets()
        .into_iter()
        .map(|subnet| subnet.region.as_str())
        .collect();

    assert_eq!(visible, vec!["europe-west1"]);
}

/// The rendered `On`/`Off` is searchable, because `true` appears in no cell.
#[test]
fn the_filter_narrows_by_the_rendered_private_google_access_word() {
    let mut rows = three_default_subnets();
    rows[1].private_google_access = true;
    let mut state = state_with(rows);

    state.start_filter();
    for character in "On".chars() {
        state.append_filter_char(character);
    }
    state.apply_filter();

    let visible: Vec<&str> = state
        .visible_vpc_subnets()
        .into_iter()
        .map(|subnet| subnet.region.as_str())
        .collect();
    assert_eq!(
        visible,
        vec!["europe-west1"],
        "the one row with the flag set is the one that matches"
    );
}

// --- The visible count ---

/// The `filtered`/`total` the row counter paints, which nothing else reads.
#[test]
fn the_visible_count_counts_the_rows_the_filter_leaves() {
    let mut state = state_with(three_default_subnets());

    assert_eq!(state.visible_vpc_subnet_count(), 3);

    state.start_filter();
    for character in "asia".chars() {
        state.append_filter_char(character);
    }
    state.apply_filter();

    assert_eq!(state.visible_vpc_subnet_count(), 1);
    assert_eq!(
        state.visible_vpc_subnet_count(),
        state.visible_vpc_subnets().len(),
        "the count and the list are two readings of one filter"
    );
}

// --- The cursor across a reload ---

/// A reload keeps the cursor on the same *subnet*, which means matching the
/// region too.
///
/// This is where the networks table's name-only match would be actively wrong
/// rather than merely unpinned: with three rows called `default`, matching on
/// the name puts the cursor on whichever region sorts first, every time.
#[test]
fn a_reload_keeps_the_cursor_on_the_same_region_not_merely_the_same_name() {
    let mut state = state_with(three_default_subnets());

    // Default ordering is by name then region, so the rows read asia-east1,
    // europe-west1, us-central1. Step to `europe-west1`.
    state.select_next();
    assert_eq!(
        state.selected_vpc_subnet().map(|s| s.region.as_str()),
        Some("europe-west1"),
        "precondition: the cursor is on the row this test follows"
    );

    // A reload that inserts a region sorting ahead of it.
    let mut reloaded = three_default_subnets();
    reloaded.push(subnet("default", "africa-south1", "10.200.0.0/20"));
    state.set_vpc_subnets(ctx(), reloaded);

    assert_eq!(
        state.selected_vpc_subnet().map(|s| s.region.as_str()),
        Some("europe-west1"),
        "the cursor follows (name, region); a name-only match would land on \
         whichever region sorted first"
    );
}

/// A subnet that is gone after the reload leaves the cursor in range.
#[test]
fn a_reload_that_drops_the_selected_subnet_clamps_the_cursor() {
    let mut state = state_with(three_default_subnets());
    state.select_next();
    state.select_next();

    state.set_vpc_subnets(ctx(), vec![subnet("only", "us-west1", "10.1.0.0/20")]);

    assert_eq!(
        state.selected_vpc_subnet().map(|s| s.name.as_str()),
        Some("only")
    );
}

// --- Sorting ---

/// The tie-break is (name, region), so rows sharing a name have one order.
///
/// Without the region in the tie-break, the three `default` rows would have no
/// defined order among them and the cursor would move under a reload that
/// returned them in a different sequence.
#[test]
fn rows_sharing_a_name_are_ordered_by_region() {
    let state = state_with(three_default_subnets());

    let regions: Vec<&str> = state
        .visible_vpc_subnets()
        .into_iter()
        .map(|subnet| subnet.region.as_str())
        .collect();

    assert_eq!(regions, vec!["asia-east1", "europe-west1", "us-central1"]);
}

// --- The problem accessor ---

/// A refusal for the active context is reportable.
///
/// The positive half of the pair: a negative assertion about this accessor is
/// worth nothing unless something proves the field can be filled at all — see
/// the note on the networks table's equivalent.
#[test]
fn a_failure_for_this_context_becomes_the_tables_problem() {
    let mut state = state_with(three_default_subnets());
    assert!(state.vpc_subnets_problem().is_none());

    state.set_service_diagnostic(
        ResourceServiceKind::VpcSubnets,
        GcloudDiagnostic::new(
            GcloudErrorKind::PermissionDenied,
            "VPC subnets",
            None,
            None,
            "compute.subnetworks.list denied on dev-project",
        ),
    );

    assert_eq!(
        state
            .vpc_subnets_problem()
            .expect("a diagnostic for this service is reportable")
            .kind,
        GcloudErrorKind::PermissionDenied
    );
}

// --- The sort vocabulary's labels ---

/// Each field's label, which is what the sort indicator paints.
#[test]
fn every_sort_field_has_its_own_label() {
    let labels = [
        VpcSubnetSortField::Name.label(),
        VpcSubnetSortField::Region.label(),
        VpcSubnetSortField::Network.label(),
        VpcSubnetSortField::Range.label(),
        VpcSubnetSortField::Gateway.label(),
        VpcSubnetSortField::PrivateGoogleAccess.label(),
    ];

    assert_eq!(
        labels,
        [
            "Name",
            "Region",
            "Network",
            "Range",
            "Gateway",
            "Private Google access"
        ]
    );
}
