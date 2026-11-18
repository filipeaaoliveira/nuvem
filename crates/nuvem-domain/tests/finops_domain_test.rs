use nuvem_domain::gcp::finops::{
    CostRecommendation, LocationKind, RecommendationState, RecommenderId, parse_duration_seconds,
    savings_total,
};
use nuvem_domain::gcp::money::Money;

fn saving(nanos: i128, state: RecommendationState, xor: Option<&str>) -> CostRecommendation {
    CostRecommendation {
        name: format!("rec-{nanos}-{xor:?}"),
        description: "test".to_string(),
        state,
        priority: None,
        xor_group: xor.map(str::to_string),
        monthly: Some(Money {
            currency: "USD".to_string(),
            nanos,
        }),
        last_refresh: None,
        target_resources: Vec::new(),
    }
}

#[test]
fn a_google_duration_parses_to_seconds() {
    assert_eq!(parse_duration_seconds("2592000s"), Some(2_592_000));
    assert_eq!(parse_duration_seconds("1296000.5s"), Some(1_296_000));
    assert_eq!(parse_duration_seconds("2592000"), None);
    assert_eq!(parse_duration_seconds(""), None);
}

/// The four cost recommenders differ in what a "location" is, and getting it
/// wrong means querying a zone name against a regional recommender and
/// getting nothing back — indistinguishable from "no savings".
#[test]
fn each_recommender_knows_whether_its_location_is_a_zone_or_a_region() {
    assert_eq!(RecommenderId::IdleVm.location_kind(), LocationKind::Zonal);
    assert_eq!(
        RecommenderId::MachineType.location_kind(),
        LocationKind::Zonal
    );
    assert_eq!(RecommenderId::IdleDisk.location_kind(), LocationKind::Zonal);
    assert_eq!(
        RecommenderId::UnusedAddress.location_kind(),
        LocationKind::Regional
    );
}

#[test]
fn every_recommender_id_is_the_string_the_api_expects() {
    assert_eq!(
        RecommenderId::IdleVm.id(),
        "google.compute.instance.IdleResourceRecommender"
    );
    assert_eq!(
        RecommenderId::MachineType.id(),
        "google.compute.instance.MachineTypeRecommender"
    );
    assert_eq!(
        RecommenderId::IdleDisk.id(),
        "google.compute.disk.IdleResourceRecommender"
    );
    assert_eq!(
        RecommenderId::UnusedAddress.id(),
        "google.compute.address.IdleResourceRecommender"
    );
    assert_eq!(RecommenderId::ALL.len(), 4);
}

/// Every recommender carries its own card heading. A blank or shared one puts
/// a savings figure under a group the operator cannot identify.
#[test]
fn every_recommender_has_its_own_non_empty_label() {
    let labels: Vec<&str> = RecommenderId::ALL.iter().map(|id| id.label()).collect();
    assert!(labels.iter().all(|label| !label.is_empty()));
    let mut unique = labels.clone();
    unique.sort_unstable();
    unique.dedup();
    assert_eq!(unique.len(), labels.len(), "labels are not interchangeable");
    assert_eq!(RecommenderId::IdleVm.label(), "Idle VMs");
    assert_eq!(
        RecommenderId::MachineType.label(),
        "Machine-type right-sizing"
    );
    assert_eq!(RecommenderId::IdleDisk.label(), "Idle disks");
    assert_eq!(RecommenderId::UnusedAddress.label(), "Unused addresses");
}

/// A DISMISSED recommendation is the operator having already said "this one is
/// deliberate". Totalling it argues with them using their own data.
#[test]
fn a_dismissed_recommendation_is_not_a_saving() {
    assert!(!RecommendationState::Dismissed.counts_toward_savings());
    assert!(!RecommendationState::Succeeded.counts_toward_savings());
    assert!(!RecommendationState::Failed.counts_toward_savings());
    assert!(!RecommendationState::Claimed.counts_toward_savings());
    assert!(RecommendationState::Active.counts_toward_savings());
}

/// Each API spelling maps to its own state. A single wrong arm here is
/// invisible on screen and changes a total.
#[test]
fn every_api_state_spelling_maps_to_its_own_variant() {
    assert_eq!(
        RecommendationState::from_api(Some("ACTIVE")),
        RecommendationState::Active
    );
    assert_eq!(
        RecommendationState::from_api(Some("CLAIMED")),
        RecommendationState::Claimed
    );
    assert_eq!(
        RecommendationState::from_api(Some("SUCCEEDED")),
        RecommendationState::Succeeded
    );
    assert_eq!(
        RecommendationState::from_api(Some("FAILED")),
        RecommendationState::Failed
    );
    assert_eq!(
        RecommendationState::from_api(Some("DISMISSED")),
        RecommendationState::Dismissed
    );
}

#[test]
fn an_unknown_state_string_does_not_become_a_saving() {
    assert_eq!(
        RecommendationState::from_api(Some("SOMETHING_NEW")),
        RecommendationState::Unrecognized("SOMETHING_NEW".to_string())
    );
    assert!(!RecommendationState::from_api(Some("SOMETHING_NEW")).counts_toward_savings());
    assert!(!RecommendationState::from_api(None).counts_toward_savings());
}

/// Members of one xorGroupId are alternatives — "only one recommendation
/// within the group is suggested to be applied". Summing them is the most
/// plausible-looking wrong number this feature can produce, because each
/// member is individually correct.
#[test]
fn only_the_largest_member_of_an_xor_group_is_counted() {
    let recommendations = vec![
        saving(30_000_000_000, RecommendationState::Active, Some("grp-a")),
        saving(80_000_000_000, RecommendationState::Active, Some("grp-a")),
        saving(10_000_000_000, RecommendationState::Active, None),
    ];
    let parts = savings_total(&recommendations).into_parts();
    assert_eq!(parts.len(), 1);
    assert_eq!(
        parts[0].nanos, 90_000_000_000,
        "80 (the larger alternative) + 10, not 120"
    );
}

/// The larger member wins whichever order it arrives in. A fold that only
/// ever keeps the first, or only ever the last, passes the test above by
/// accident half the time.
#[test]
fn the_largest_member_wins_from_either_arrival_order() {
    let descending = vec![
        saving(80_000_000_000, RecommendationState::Active, Some("grp-a")),
        saving(30_000_000_000, RecommendationState::Active, Some("grp-a")),
    ];
    assert_eq!(
        savings_total(&descending).into_parts()[0].nanos,
        80_000_000_000
    );
}

/// Two groups are two independent choices. Folding them together would count
/// one alternative from the pair and drop the other group entirely.
#[test]
fn separate_xor_groups_are_counted_separately() {
    let recommendations = vec![
        saving(80_000_000_000, RecommendationState::Active, Some("grp-a")),
        saving(30_000_000_000, RecommendationState::Active, Some("grp-a")),
        saving(50_000_000_000, RecommendationState::Active, Some("grp-b")),
        saving(20_000_000_000, RecommendationState::Active, Some("grp-b")),
    ];
    assert_eq!(
        savings_total(&recommendations).into_parts()[0].nanos,
        130_000_000_000
    );
}

#[test]
fn ungrouped_recommendations_all_count() {
    let recommendations = vec![
        saving(30_000_000_000, RecommendationState::Active, None),
        saving(80_000_000_000, RecommendationState::Active, None),
    ];
    assert_eq!(
        savings_total(&recommendations).into_parts()[0].nanos,
        110_000_000_000
    );
}

#[test]
fn a_dismissed_member_does_not_win_its_xor_group() {
    let recommendations = vec![
        saving(
            80_000_000_000,
            RecommendationState::Dismissed,
            Some("grp-a"),
        ),
        saving(30_000_000_000, RecommendationState::Active, Some("grp-a")),
    ];
    assert_eq!(
        savings_total(&recommendations).into_parts()[0].nanos,
        30_000_000_000
    );
}

/// An xor group whose members are priced in different currencies cannot be
/// compared by amount, so the winner is not a matter of which number is
/// bigger. Keeping the first arrival makes the fold total exactly one
/// alternative per group in each currency it saw, rather than silently
/// ranking 50 EUR against 80 USD.
#[test]
fn an_xor_group_spanning_two_currencies_keeps_one_alternative_per_currency() {
    let mut euros = saving(50_000_000_000, RecommendationState::Active, Some("grp-a"));
    euros.monthly = Some(Money {
        currency: "EUR".to_string(),
        nanos: 50_000_000_000,
    });
    let recommendations = vec![
        saving(80_000_000_000, RecommendationState::Active, Some("grp-a")),
        euros,
    ];

    let mut parts = savings_total(&recommendations).into_parts();
    parts.sort_by(|a, b| a.currency.cmp(&b.currency));
    assert_eq!(parts.len(), 2);
    assert_eq!(parts[0].currency, "EUR");
    assert_eq!(parts[0].nanos, 50_000_000_000);
    assert_eq!(parts[1].currency, "USD");
    assert_eq!(parts[1].nanos, 80_000_000_000);
}

/// A recommendation whose duration could not be normalised has no monthly
/// figure. It is listed, but it must not silently contribute nothing to a
/// total that claims to cover it — the card reports the exclusion count.
#[test]
fn a_recommendation_with_no_monthly_figure_is_excluded_from_the_total() {
    let mut orphan = saving(0, RecommendationState::Active, None);
    orphan.monthly = None;
    let recommendations = vec![
        orphan,
        saving(10_000_000_000, RecommendationState::Active, None),
    ];
    let parts = savings_total(&recommendations).into_parts();
    assert_eq!(parts.len(), 1);
    assert_eq!(parts[0].nanos, 10_000_000_000);
}

/// A grouped recommendation with no monthly figure must not claim the group
/// and shut out the priced alternative beside it.
#[test]
fn an_unpriced_group_member_does_not_displace_a_priced_one() {
    let mut orphan = saving(0, RecommendationState::Active, Some("grp-a"));
    orphan.monthly = None;
    let recommendations = vec![
        orphan,
        saving(10_000_000_000, RecommendationState::Active, Some("grp-a")),
    ];
    let parts = savings_total(&recommendations).into_parts();
    assert_eq!(parts.len(), 1);
    assert_eq!(parts[0].nanos, 10_000_000_000);
}
