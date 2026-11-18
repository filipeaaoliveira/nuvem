//! One walk over every registered [`ResourceService`] pinning that its
//! `matches_filter` reads the row it was handed: a row whose text the filter
//! names survives, and the same row survives no filter that names something
//! it does not say.
//!
//! # Why a walk and not a pin per service
//!
//! `matches_filter` is a trait obligation with almost no production caller.
//! The generic path that would exercise it,
//! `ResourcesState::visible_rows::<S>`, is reached by exactly three services:
//! the persistent disks accessor calls it, and the marks machinery dispatches
//! to Compute and Cloud SQL. The other thirteen `visible_*` accessors call
//! the free `*_matches_filter` function directly and never go through the
//! impl at all, so for those services it was reachable only from a test that
//! remembered to call it — and mutation found the consequence: VPC
//! networks' `matches_filter` survived as **both** `true` and `false`,
//! because nothing anywhere called it. An obligation with no caller breaks
//! the first time a generic helper does start dispatching by kind, and this
//! repo adds such helpers routinely.
//!
//! A copy of this assertion per service would leave the same hole open for
//! the next service somebody adds. This walk covers all of them, including
//! the ones nobody has written yet: the coverage test below fails when a
//! service kind exists that the walk does not probe.
//!
//! # Why the trait needed [`ResourceService::sample_row`]
//!
//! `matches_filter` takes `Self::Row`, and each service's row is a different
//! type, so a walk cannot build the argument the way
//! `sort_column_invariant_test` builds nothing at all (its subject is index
//! arithmetic over the column catalogue). The row constructor is the whole of
//! what this ticket adds to production: without it there is no generic way to
//! reach `matches_filter` for a service, which is exactly the property that
//! let VPC's implementation ship untested.
//!
//! # Why one row and two filters, not two rows
//!
//! `matches_filter` is a function of the pair, so "a row that matches matches
//! and a row that does not match does not" is the same statement as "this row
//! matches a filter naming its own text and not one naming text it does not
//! carry". Holding the row fixed and varying the filter keeps the second
//! constructor — and the second chance to write a row that accidentally
//! carries the absent needle — out of the trait.
//!
//! # What it catches
//!
//! The two constant bodies mutation generates, per service, in one run:
//! `-> true` makes the absent needle match a row that never mentions it (the
//! operator's filter narrows nothing), and `-> false` makes the row's own
//! text fail to find it (the operator's filter empties the table). Each
//! direction was confirmed by hand-mutating every service's `matches_filter`
//! in turn and checking that this walk names that service and no other.
//!
//! The mutation gate is no evidence either way here, and saying so is part of
//! the record: the only mutants this change generates are the sixteen
//! `sample_row -> Default::default()` bodies, and every one of them is
//! unviable, because no row type implements `Default`. The gate closes green
//! having tested nothing about the walk. The hand-mutation run above is what
//! measured it.
//!
//! What it does NOT buy: it says nothing about *which* fields a row is
//! searchable by beyond the one the needle sits in — dropping a status or a
//! location from a service's searchable fields leaves this walk green. That
//! is `filter_searchable_fields_test`'s subject, and it is deliberately
//! narrower than this one: this walk pins that the predicate is wired to the
//! row at all, for every service.

use nuvem::app::state::resources::{
    BigQuery, CloudFunctions, CloudRun, CloudSql, Compute, FirewallRules, Gke, IamPolicy,
    IamServiceAccounts, PersistentDisks, PubSubSubscriptions, PubSubTopics, ResourceService,
    SAMPLE_ROW_NEEDLE, Secrets, Storage, VpcNetworks, VpcSubnets,
};
use nuvem_domain::gcp::filter::{CompiledFilter, compile_filter};
use nuvem_domain::gcp::resource::ResourceServiceKind;

/// Text no sample row carries, in any field, for any service. Held to the
/// same alphabet as [`SAMPLE_ROW_NEEDLE`] so it is a literal under the
/// default regex mode rather than a pattern with its own meaning.
const ABSENT_NEEDLE: &str = "nosamplerowsaysthis";

/// Compiles a needle the way the `visible_*` accessors do. `CompiledFilter`
/// owns case-insensitivity, so neither needle above needs lowercasing.
fn compiled(input: &str) -> CompiledFilter {
    compile_filter(input).expect("test filter input must compile")
}

/// What one service's `matches_filter` answered for its own sample row, in
/// both directions.
struct Probe {
    kind: ResourceServiceKind,
    /// Whether the row matched a filter naming text the row carries. A
    /// service answering `false` here empties its table for a filter that
    /// should have found the row.
    matched_its_own_text: bool,
    /// Whether the row matched a filter naming text no row carries. A service
    /// answering `true` here narrows nothing, whatever the operator types.
    matched_absent_text: bool,
}

/// Runs both directions against `S`, without asserting: the caller collects
/// every service's answer so one run can name every offender rather than
/// aborting on the first. The VPC slice found that a walk that stops at the
/// first failure reports one service when several are broken.
fn probe<S: ResourceService>() -> Probe {
    let row = S::sample_row();
    Probe {
        kind: S::KIND,
        matched_its_own_text: S::matches_filter(&row, &compiled(SAMPLE_ROW_NEEDLE)),
        matched_absent_text: S::matches_filter(&row, &compiled(ABSENT_NEEDLE)),
    }
}

/// Every service the walk knows about.
///
/// Hand-listed rather than iterated over `ResourceServiceKind::ALL`, because
/// each service's `Row` is a different type and a runtime loop cannot name
/// sixteen of them. That is what makes
/// [`the_walk_probes_every_registered_service`] load-bearing rather than
/// decorative: it is the only thing standing between this list and the next
/// service somebody adds.
fn probes() -> Vec<Probe> {
    vec![
        probe::<Compute>(),
        probe::<Gke>(),
        probe::<CloudSql>(),
        probe::<CloudRun>(),
        probe::<Storage>(),
        probe::<Secrets>(),
        probe::<CloudFunctions>(),
        probe::<PersistentDisks>(),
        probe::<PubSubTopics>(),
        probe::<PubSubSubscriptions>(),
        probe::<BigQuery>(),
        probe::<IamServiceAccounts>(),
        probe::<IamPolicy>(),
        probe::<VpcNetworks>(),
        probe::<VpcSubnets>(),
        probe::<FirewallRules>(),
    ]
}

/// Every service's `matches_filter` answers from the row it was handed.
#[test]
fn every_service_filters_its_sample_row_by_what_the_row_says() {
    let mut broken: Vec<String> = Vec::new();

    for probe in probes() {
        let kind = probe.kind;
        if !probe.matched_its_own_text {
            broken.push(format!(
                "  {kind:?}: its sample row carries {SAMPLE_ROW_NEEDLE:?} and \
                 matches_filter rejected it, so filtering this table by what a \
                 row says hides that row"
            ));
        }
        if probe.matched_absent_text {
            broken.push(format!(
                "  {kind:?}: its sample row carries no {ABSENT_NEEDLE:?} and \
                 matches_filter accepted it, so filtering this table narrows \
                 nothing"
            ));
        }
    }

    assert!(
        broken.is_empty(),
        "{} service(s) filter rows by something other than the row:\n{}",
        broken.len(),
        broken.join("\n")
    );
}

/// The walk probes every registered service.
///
/// Adding a service kind without adding it to [`probes`] fails here, naming
/// the kind: the list above is hand-written and nothing else in the build
/// would notice the omission. Reported as a list rather than a count so the
/// author adding the service reads which one is missing.
#[test]
fn the_walk_probes_every_registered_service() {
    let probed: Vec<ResourceServiceKind> = probes().iter().map(|probe| probe.kind).collect();

    let missing: Vec<String> = ResourceServiceKind::ALL
        .into_iter()
        .filter(|kind| !probed.contains(kind))
        .map(|kind| format!("  {kind:?}"))
        .collect();
    assert!(
        missing.is_empty(),
        "{} service kind(s) are never probed, so their matches_filter is \
         asserted by nothing generic:\n{}",
        missing.len(),
        missing.join("\n")
    );

    // A kind probed twice would let the list *look* complete while a third
    // service is missing, since the count alone cannot tell the two apart.
    let duplicated: Vec<String> = ResourceServiceKind::ALL
        .into_iter()
        .filter(|kind| {
            probed
                .iter()
                .filter(|probed_kind| *probed_kind == kind)
                .count()
                > 1
        })
        .map(|kind| format!("  {kind:?}"))
        .collect();
    assert!(
        duplicated.is_empty(),
        "{} service kind(s) are probed more than once, which hides a missing \
         one behind a complete-looking count:\n{}",
        duplicated.len(),
        duplicated.join("\n")
    );
}
