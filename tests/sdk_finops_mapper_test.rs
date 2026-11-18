//! The native Recommender mapper, field by field and branch by branch.
//!
//! A mutation sweep reported `map_recommendation` as untested: nothing pinned
//! what a generated `Recommendation` becomes, so a body replaced wholesale, a
//! flipped `==`, a dropped `!`, or a silently unscaled cost all survived. The
//! tests below plant a distinct, non-default value for every field of the row
//! and assert the exact `CostRecommendation`, and every conditional — absent
//! impact, a non-cost projection, a missing cost, a missing or zero duration,
//! each state and priority arm, an empty xor group, an empty operation
//! resource — is exercised from both sides with outputs that differ.

use google_cloud_recommender_v1::model::{
    CostProjection, Impact, Operation, OperationGroup, Recommendation, RecommendationContent,
    RecommendationStateInfo, SecurityProjection, recommendation::Priority,
    recommendation_state_info::State,
};
use google_cloud_wkt::{Duration, Timestamp};

use nuvem::services::sdk::finops::map_recommendation;
use nuvem_domain::gcp::finops::{CostRecommendation, RecommendationState};
use nuvem_domain::gcp::money::Money;

/// Seconds in the 30-day month `map_recommendation` normalises to, spelled out
/// here rather than imported so a change to the constant shows up as a failing
/// expectation instead of silently moving every figure below.
const MONTH: i64 = 2_592_000;
const DAY: i64 = 86_400;

fn timestamp(seconds: i64, nanos: i32) -> Timestamp {
    Timestamp::new(seconds, nanos).expect("a valid timestamp")
}

fn duration(seconds: i64) -> Duration {
    Duration::new(seconds, 0).expect("a valid duration")
}

/// A `CostProjection` carrying a `google.type.Money`.
///
/// That money type lives in `google-cloud-type`, a transitive dependency this
/// crate does not name, so the value is taken from the field's own default
/// rather than constructed through an import.
fn cost_projection(units: i64, nanos: i32, currency: &str) -> CostProjection {
    let mut cost = CostProjection::new().cost.unwrap_or_default();
    cost.units = units;
    cost.nanos = nanos;
    cost.currency_code = currency.to_string();
    CostProjection::new().set_cost(cost)
}

/// The whole primary impact for a cost recommendation: an amount stated over
/// `seconds`, which is what the mapper has to normalise to a month.
fn cost_impact(units: i64, nanos: i32, currency: &str, seconds: i64) -> Impact {
    let projection = cost_projection(units, nanos, currency).set_duration(duration(seconds));
    Impact::new().set_cost_projection(projection)
}

fn money(currency: &str, nanos: i128) -> Money {
    Money {
        currency: currency.to_string(),
        nanos,
    }
}

/// A recommendation whose only interesting part is its primary impact.
fn with_impact(impact: Impact) -> Recommendation {
    Recommendation::new().set_primary_impact(impact)
}

fn operation(resource: &str) -> Operation {
    Operation::new().set_resource(resource)
}

// ------------------------------------------------------- whole-row identity

/// Kills "replace body with `Default::default()`" and any per-field drop:
/// every field of the row carries a value distinct from its type's default and
/// from every other field's, and the monthly figure is scaled by a factor of
/// thirty so an unscaled passthrough is a different number.
#[test]
fn a_recommendation_maps_each_field_to_its_own_planted_value() {
    let name = "projects/ledger-prod/locations/europe-west1-b/recommenders/\
        google.compute.instance.IdleResourceRecommender/recommendations/9d1f-7c20";
    let instance = "//compute.googleapis.com/projects/ledger-prod/zones/europe-west1-b/\
        instances/batch-runner-07";
    let disk = "//compute.googleapis.com/projects/ledger-prod/zones/europe-west1-b/\
        disks/batch-runner-07-boot";

    let recommendation = Recommendation::new()
        .set_name(name)
        .set_description("Save cost by stopping idle VM instance 'batch-runner-07'.")
        .set_priority(Priority::P2)
        .set_xor_group_id("xor-idle-vm-7")
        .set_last_refresh_time(timestamp(1_712_345_678, 0))
        .set_state_info(RecommendationStateInfo::new().set_state(State::Claimed))
        .set_primary_impact(cost_impact(-7, -500_000_000, "EUR", DAY))
        .set_content(RecommendationContent::new().set_operation_groups([
            OperationGroup::new().set_operations([operation(instance)]),
            OperationGroup::new().set_operations([operation(disk)]),
        ]));

    assert_eq!(
        map_recommendation(&recommendation),
        CostRecommendation {
            name: name.to_string(),
            description: "Save cost by stopping idle VM instance 'batch-runner-07'.".to_string(),
            state: RecommendationState::Claimed,
            priority: Some("P2".to_string()),
            xor_group: Some("xor-idle-vm-7".to_string()),
            monthly: Some(money("EUR", 225_000_000_000)),
            last_refresh: Some("2024-04-05T19:34:38Z".to_string()),
            target_resources: vec![instance.to_string(), disk.to_string()],
        }
    );
}

/// The other side of every optional field: a model that sets none of them must
/// map to absences and to an empty list, never to a `Some` of the default or
/// to a `Some("")`. Pins that a body replaced with `Some(Default)` — which
/// would fill `monthly`, `xor_group` and `last_refresh` — is not this row.
#[test]
fn an_empty_recommendation_maps_to_absences_rather_than_empty_values() {
    assert_eq!(
        map_recommendation(&Recommendation::new()),
        CostRecommendation {
            name: String::new(),
            description: String::new(),
            state: RecommendationState::Unrecognized(String::new()),
            priority: None,
            xor_group: None,
            monthly: None,
            last_refresh: None,
            target_resources: Vec::new(),
        }
    );
}

/// The API's own identifier and its own English are shown verbatim: neither is
/// shortened to a basename and neither is re-worded, so a body replaced with
/// `"xyzzy"` or with the empty string is a different row.
#[test]
fn the_name_and_description_pass_through_verbatim() {
    let name = "projects/42/locations/us-central1/recommenders/\
        google.compute.address.IdleResourceRecommender/recommendations/abc-123";
    let description = "Address 'legacy-nat-ip' has been unused for 30 days; release it.";
    let row = map_recommendation(
        &Recommendation::new()
            .set_name(name)
            .set_description(description),
    );

    assert_eq!(row.name, name);
    assert_eq!(row.description, description);
}

// -------------------------------------------------------------- xor_group

/// `non_empty` is the only thing between the proto's unset string and a
/// `Some("")` in a savings total that would then treat every ungrouped
/// recommendation as members of one group. Both sides, one assertion apart.
#[test]
fn an_xor_group_is_present_only_when_the_api_sent_one() {
    assert_eq!(
        map_recommendation(&Recommendation::new().set_xor_group_id("")).xor_group,
        None
    );
    assert_eq!(
        map_recommendation(&Recommendation::new().set_xor_group_id("group-a")).xor_group,
        Some("group-a".to_string())
    );
}

// ------------------------------------------------------------------ state

/// Every named state arm, each one a different domain state. A match replaced
/// by any single arm, or a guard forced to `true`/`false`, collapses these
/// onto one value.
#[test]
fn each_named_state_maps_to_its_own_domain_state() {
    let cases = [
        (State::Active, RecommendationState::Active),
        (State::Claimed, RecommendationState::Claimed),
        (State::Succeeded, RecommendationState::Succeeded),
        (State::Failed, RecommendationState::Failed),
        (State::Dismissed, RecommendationState::Dismissed),
    ];

    for (api, expected) in cases {
        let recommendation = Recommendation::new()
            .set_state_info(RecommendationStateInfo::new().set_state(api.clone()));
        assert_eq!(
            map_recommendation(&recommendation).state,
            expected,
            "state {api:?} mapped to the wrong domain state"
        );
    }
}

/// The fallback arm, with a name this app has never heard of: it is kept
/// verbatim rather than folded into `Active`, because a state that cannot be
/// classified must not be counted into a savings total.
#[test]
fn an_unrecognised_state_name_is_kept_verbatim() {
    let recommendation = Recommendation::new()
        .set_state_info(RecommendationStateInfo::new().set_state(State::from("ARCHIVED")));

    assert_eq!(
        map_recommendation(&recommendation).state,
        RecommendationState::Unrecognized("ARCHIVED".to_string())
    );
}

/// The proto's own "unset" spelling is a name like any other, and lands in the
/// fallback rather than in `Active`.
#[test]
fn the_unspecified_state_lands_in_the_unrecognised_arm() {
    let recommendation = Recommendation::new()
        .set_state_info(RecommendationStateInfo::new().set_state(State::Unspecified));

    assert_eq!(
        map_recommendation(&recommendation).state,
        RecommendationState::Unrecognized("STATE_UNSPECIFIED".to_string())
    );
}

/// Two ways for the state to have no name at all — no `state_info` at all, and
/// a state that arrived as an unknown integer — both reach `from_api(None)`.
/// A `and_then` replaced with `Some(Default)` would answer `Active` here.
#[test]
fn a_state_with_no_name_maps_to_an_empty_unrecognised_state() {
    assert_eq!(
        map_recommendation(&Recommendation::new()).state,
        RecommendationState::Unrecognized(String::new())
    );

    let numeric = Recommendation::new()
        .set_state_info(RecommendationStateInfo::new().set_state(State::from(77)));
    assert_eq!(
        map_recommendation(&numeric).state,
        RecommendationState::Unrecognized(String::new())
    );
}

// --------------------------------------------------------------- priority

/// Every priority arm that names a priority.
///
/// The unspecified arm is deliberately absent from this list and pinned on its
/// own below: it is the enum's default, so every recommendation that states no
/// priority arrives carrying it, and printing its name put
/// `[PRIORITY_UNSPECIFIED]` on the card beside the cost where the CLI path
/// printed nothing at all.
#[test]
fn each_priority_maps_to_the_api_name_for_that_arm() {
    let cases = [
        (Priority::P1, "P1"),
        (Priority::P2, "P2"),
        (Priority::P3, "P3"),
        (Priority::P4, "P4"),
    ];

    for (api, expected) in cases {
        let recommendation = Recommendation::new().set_priority(api.clone());
        assert_eq!(
            map_recommendation(&recommendation).priority,
            Some(expected.to_string()),
            "priority {api:?} mapped to the wrong name"
        );
    }
}

/// The unspecified arm reads as no priority rather than as a priority named
/// `PRIORITY_UNSPECIFIED`, matching the CLI path, which omits the field.
#[test]
fn an_unspecified_priority_is_absent_rather_than_a_sentinel_name() {
    assert_eq!(
        map_recommendation(&Recommendation::new().set_priority(Priority::Unspecified)).priority,
        None
    );
}

/// A priority that arrived as an unknown integer has no name, and the row says
/// so rather than inventing one. The other side of `.map(...)` above.
#[test]
fn a_priority_with_no_name_maps_to_none() {
    assert_eq!(
        map_recommendation(&Recommendation::new().set_priority(Priority::from(99))).priority,
        None
    );
}

// ---------------------------------------------------------------- monthly

/// The headline conversion: the API states a saving as a NEGATIVE cost over
/// the projection's own duration, and the row holds a POSITIVE monthly figure.
/// A daily projection of -7.50 EUR is 225.00 EUR a month, so a dropped
/// `negate` (-225.00) and a dropped `scaled` (7.50) are both different numbers.
#[test]
fn a_negative_daily_cost_becomes_a_positive_monthly_saving() {
    let row = map_recommendation(&with_impact(cost_impact(-7, -500_000_000, "EUR", DAY)));

    assert_eq!(row.monthly, Some(money("EUR", 225_000_000_000)));
    assert_eq!(
        row.monthly.expect("a monthly figure").render(),
        "EUR 225.00"
    );
}

/// The sign correction runs in both directions: a projection that reports an
/// INCREASE arrives positive and must come out negative, so a card showing it
/// as a saving is impossible.
#[test]
fn a_positive_cost_projection_becomes_a_negative_monthly_figure() {
    let row = map_recommendation(&with_impact(cost_impact(2, 0, "USD", MONTH)));

    assert_eq!(row.monthly, Some(money("USD", -2_000_000_000)));
}

/// A projection stated over three months is divided, not multiplied: -300.00
/// USD over 90 days is 100.00 USD a month. With the division inverted or the
/// operands swapped this is 900.00, not 100.00.
#[test]
fn a_projection_longer_than_a_month_is_scaled_down() {
    let row = map_recommendation(&with_impact(cost_impact(-300, 0, "USD", 3 * MONTH)));

    assert_eq!(row.monthly, Some(money("USD", 100_000_000_000)));
}

/// Sub-unit nanos are part of the amount, not decoration: a cost of exactly
/// -0.25 with zero units still produces a quarter of a unit a month. Pins that
/// `cost.nanos` reaches `Money::parse` rather than a `0` default.
#[test]
fn the_sub_unit_nanos_of_a_cost_reach_the_monthly_figure() {
    let row = map_recommendation(&with_impact(cost_impact(0, -250_000_000, "USD", MONTH)));

    assert_eq!(row.monthly, Some(money("USD", 250_000_000)));
    assert_eq!(row.monthly.expect("a monthly figure").render(), "USD 0.25");
}

/// The currency code is the API's own and is never converted or defaulted to
/// dollars; two currencies, asserted apart, pin that it is carried rather than
/// chosen.
#[test]
fn the_currency_code_is_carried_through_unconverted() {
    assert_eq!(
        map_recommendation(&with_impact(cost_impact(-5, 0, "JPY", MONTH))).monthly,
        Some(money("JPY", 5_000_000_000))
    );
    assert_eq!(
        map_recommendation(&with_impact(cost_impact(-5, 0, "BRL", MONTH))).monthly,
        Some(money("BRL", 5_000_000_000))
    );
}

/// The `duration == 0` guard: a zero-second projection cannot be normalised,
/// and the row says "no figure" rather than presenting an unscaled one. With
/// the comparison flipped to `!=`, every non-zero duration above would answer
/// `None` instead — which is what those tests pin from the other side.
#[test]
fn a_zero_duration_projection_yields_no_monthly_figure() {
    let row = map_recommendation(&with_impact(cost_impact(-9, 0, "USD", 0)));

    assert_eq!(row.monthly, None);
}

/// A projection with a cost but no duration at all: the amount is over an
/// unknown period, so presenting it as monthly would be a fabricated claim.
#[test]
fn a_projection_without_a_duration_yields_no_monthly_figure() {
    let impact = Impact::new().set_cost_projection(cost_projection(-9, 0, "USD"));

    assert_eq!(map_recommendation(&with_impact(impact)).monthly, None);
}

/// The mirror case — a duration but no cost — reaches the `cost.as_ref()?`
/// early return rather than a zero amount.
#[test]
fn a_projection_without_a_cost_yields_no_monthly_figure() {
    let impact =
        Impact::new().set_cost_projection(CostProjection::new().set_duration(duration(MONTH)));

    assert_eq!(map_recommendation(&with_impact(impact)).monthly, None);
}

/// The non-cost arm of the projection match. A security projection carries no
/// money, and the wildcard arm is what keeps it out of the savings column.
#[test]
fn a_non_cost_projection_yields_no_monthly_figure() {
    let impact = Impact::new().set_security_projection(SecurityProjection::new());

    assert_eq!(map_recommendation(&with_impact(impact)).monthly, None);
}

/// An impact that names a service but carries no projection at all.
#[test]
fn an_impact_without_a_projection_yields_no_monthly_figure() {
    let impact = Impact::new().set_service("compute.googleapis.com");

    assert_eq!(map_recommendation(&with_impact(impact)).monthly, None);
}

/// No primary impact: the first `as_ref()` in the chain, from the side that
/// short-circuits.
#[test]
fn a_recommendation_without_a_primary_impact_yields_no_monthly_figure() {
    assert_eq!(map_recommendation(&Recommendation::new()).monthly, None);
}

/// Only the PRIMARY impact is a saving. Additional impacts are explicitly
/// "may be positive or negative" side effects of optimising for the primary
/// category, so a cost projection sitting only there is not the row's figure.
#[test]
fn a_cost_projection_in_an_additional_impact_is_ignored() {
    let recommendation =
        Recommendation::new().set_additional_impact([cost_impact(-40, 0, "USD", MONTH)]);

    assert_eq!(map_recommendation(&recommendation).monthly, None);
}

// -------------------------------------------------------- target_resources

/// Every operation of every group, flattened in the order the API listed them.
/// Two groups of two, all four resources distinct: a `flat_map` that kept only
/// the first group, or an iteration that reversed, is a different vector.
#[test]
fn target_resources_flatten_every_operation_group_in_order() {
    let recommendation = Recommendation::new().set_content(
        RecommendationContent::new().set_operation_groups([
            OperationGroup::new().set_operations([operation("//res/one"), operation("//res/two")]),
            OperationGroup::new()
                .set_operations([operation("//res/three"), operation("//res/four")]),
        ]),
    );

    assert_eq!(
        map_recommendation(&recommendation).target_resources,
        vec![
            "//res/one".to_string(),
            "//res/two".to_string(),
            "//res/three".to_string(),
            "//res/four".to_string(),
        ]
    );
}

/// The `!operation.resource.is_empty()` filter, from both sides in one row: an
/// operation that names nothing is dropped and the named ones survive. With
/// the `!` deleted the surviving list is `[""]`, which is a different vector
/// and a resource column showing a blank row.
#[test]
fn an_operation_with_an_empty_resource_is_dropped() {
    let recommendation =
        Recommendation::new().set_content(RecommendationContent::new().set_operation_groups([
            OperationGroup::new().set_operations([
                operation("//res/kept-first"),
                operation(""),
                operation("//res/kept-last"),
            ]),
        ]));

    assert_eq!(
        map_recommendation(&recommendation).target_resources,
        vec![
            "//res/kept-first".to_string(),
            "//res/kept-last".to_string()
        ]
    );
}

/// The `unwrap_or_default` side: a recommendation with no content at all has
/// no targets, rather than a one-element list of the default string.
#[test]
fn a_recommendation_without_content_has_no_target_resources() {
    assert_eq!(
        map_recommendation(&Recommendation::new()).target_resources,
        Vec::<String>::new()
    );
}

/// Content with no operation groups is the empty case reached through the
/// `Some` branch instead of through `unwrap_or_default`.
#[test]
fn content_without_operation_groups_has_no_target_resources() {
    let recommendation = Recommendation::new().set_content(RecommendationContent::new());

    assert_eq!(
        map_recommendation(&recommendation).target_resources,
        Vec::<String>::new()
    );
}

/// The row's targets come from the operations, not from the recommendation's
/// own `target_resources` field: the two are planted with different values, so
/// reading the wrong one is a visible swap rather than a silent equivalence.
#[test]
fn target_resources_come_from_the_operations_not_the_top_level_field() {
    let recommendation = Recommendation::new()
        .set_target_resources(["//top-level/ignored"])
        .set_content(RecommendationContent::new().set_operation_groups([
            OperationGroup::new().set_operations([operation("//operation/used")]),
        ]));

    assert_eq!(
        map_recommendation(&recommendation).target_resources,
        vec!["//operation/used".to_string()]
    );
}

// ------------------------------------------------------------ last_refresh

/// The refresh time is rendered the way the CLI path spells a timestamp, so a
/// projection that sorts these as strings sees one shape regardless of which
/// backend filled the row.
#[test]
fn a_last_refresh_time_renders_in_the_cli_timestamp_shape() {
    let recommendation = Recommendation::new().set_last_refresh_time(timestamp(1_700_000_000, 0));

    assert_eq!(
        map_recommendation(&recommendation).last_refresh,
        Some("2023-11-14T22:13:20Z".to_string())
    );
}

/// Sub-second precision is dropped rather than rounded up: a stamp 999999999
/// nanos into its second still names that second, so the string never reports
/// a second the event had not reached.
#[test]
fn sub_second_nanos_do_not_move_the_rendered_refresh_second() {
    let recommendation =
        Recommendation::new().set_last_refresh_time(timestamp(1_700_000_000, 999_999_999));

    assert_eq!(
        map_recommendation(&recommendation).last_refresh,
        Some("2023-11-14T22:13:20Z".to_string())
    );
}

/// The absent side: no refresh time is `None`, not the epoch rendered as a
/// date. A body replaced with `Some(Default)` would answer 1970 here.
#[test]
fn a_recommendation_without_a_refresh_time_has_no_last_refresh() {
    assert_eq!(
        map_recommendation(&Recommendation::new()).last_refresh,
        None
    );
}
