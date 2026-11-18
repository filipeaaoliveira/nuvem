use nuvem::app::state::finops::{CardState, CellState, FinOpsState};
use nuvem_domain::gcp::configuration::{GcloudConfigurationName, GcloudContext};
use nuvem_domain::gcp::diagnostics::GcloudErrorKind;
use nuvem_domain::gcp::finops::{CostRecommendation, RecommendationState, RecommenderId};
use nuvem_domain::gcp::money::Money;

fn context() -> GcloudContext {
    GcloudContext {
        configuration: GcloudConfigurationName::new("default"),
        account: None,
        project: Some("proj".to_string()),
        region: None,
        zone: None,
    }
}

fn recommendation(nanos: i128) -> CostRecommendation {
    CostRecommendation {
        name: format!("rec-{nanos}"),
        description: "d".to_string(),
        state: RecommendationState::Active,
        priority: None,
        xor_group: None,
        monthly: Some(Money {
            currency: "USD".to_string(),
            nanos,
        }),
        last_refresh: None,
        target_resources: Vec::new(),
    }
}

/// With nothing listed there is nothing to query, and a card that has not been
/// probed says so. It must not read as Empty: "no idle VMs" and "we never
/// looked" are different claims and only one of them is good news.
#[test]
fn a_card_with_no_locations_to_query_is_not_probed_rather_than_empty() {
    let mut state = FinOpsState::default();
    let probes = state.begin_refresh(context(), &[]);
    assert!(probes.is_empty());
    assert_eq!(
        state.card_state(RecommenderId::IdleVm),
        CardState::NotProbed
    );
}

/// One probe per (recommender, location). Two zones in one region means two
/// zonal probes per zonal recommender and one regional probe.
#[test]
fn a_refresh_issues_one_probe_per_recommender_and_location() {
    let mut state = FinOpsState::default();
    let probes = state.begin_refresh(
        context(),
        &["europe-west1-b".to_string(), "europe-west1-c".to_string()],
    );

    let zonal = probes
        .iter()
        .filter(|(recommender, _, _)| *recommender == RecommenderId::IdleVm)
        .count();
    let regional = probes
        .iter()
        .filter(|(recommender, _, _)| *recommender == RecommenderId::UnusedAddress)
        .count();
    assert_eq!(zonal, 2, "one per zone");
    assert_eq!(regional, 1, "two zones collapse to one region");
    assert_eq!(probes.len(), 7, "three zonal recommenders x 2, plus 1");
}

/// A card fills while its other cells are still in flight — the ticket's
/// "granular per-card loading", one level finer.
#[test]
fn a_card_stays_loading_while_any_cell_is_in_flight_and_still_shows_what_answered() {
    let mut state = FinOpsState::default();
    let probes = state.begin_refresh(
        context(),
        &["europe-west1-b".to_string(), "us-central1-a".to_string()],
    );
    let (_, location, ticket) = probes
        .iter()
        .find(|(recommender, _, _)| *recommender == RecommenderId::IdleVm)
        .cloned()
        .expect("a zonal probe");

    assert!(state.install_cell(
        ticket,
        RecommenderId::IdleVm,
        &location,
        CellState::Ready(vec![recommendation(10_000_000_000)]),
    ));

    assert_eq!(state.card_state(RecommenderId::IdleVm), CardState::Loading);
    assert_eq!(state.recommendations(RecommenderId::IdleVm).len(), 1);
}

/// A denied zone cannot blank a card another zone answered. This is why the
/// state is per cell rather than per card.
#[test]
fn a_refused_cell_does_not_blank_a_card_another_cell_answered() {
    let mut state = FinOpsState::default();
    let probes = state.begin_refresh(
        context(),
        &["europe-west1-b".to_string(), "us-central1-a".to_string()],
    );
    for (recommender, location, ticket) in probes {
        if recommender != RecommenderId::IdleVm {
            continue;
        }
        let cell = if location == "europe-west1-b" {
            CellState::Ready(vec![recommendation(10_000_000_000)])
        } else {
            CellState::Unavailable {
                kind: GcloudErrorKind::PermissionDenied,
                detail: None,
            }
        };
        state.install_cell(ticket, recommender, &location, cell);
    }

    assert_eq!(state.card_state(RecommenderId::IdleVm), CardState::Ready);
    let coverage = state.coverage(RecommenderId::IdleVm);
    assert_eq!(coverage.answered, vec!["europe-west1-b".to_string()]);
    assert_eq!(coverage.refused.len(), 1);
    assert_eq!(coverage.refused[0].0, "us-central1-a");
    assert_eq!(coverage.refused[0].1, GcloudErrorKind::PermissionDenied);
}

/// All cells refused is Unavailable, carrying a kind so the card can say
/// "enable the API" where that is the truth and "check IAM" where it is not.
#[test]
fn a_card_whose_cells_all_refused_is_unavailable_with_the_kind() {
    let mut state = FinOpsState::default();
    let probes = state.begin_refresh(context(), &["europe-west1-b".to_string()]);
    for (recommender, location, ticket) in probes {
        state.install_cell(
            ticket,
            recommender,
            &location,
            CellState::Unavailable {
                kind: GcloudErrorKind::ApiDisabled,
                detail: None,
            },
        );
    }
    assert_eq!(
        state.card_state(RecommenderId::IdleVm),
        CardState::Unavailable(GcloudErrorKind::ApiDisabled)
    );
}

/// Every cell answered with nothing is a real fact and renders as Empty. A
/// Ready cell that happens to hold no rows is the same fact, not a Ready card.
#[test]
fn a_card_whose_cells_all_answered_with_nothing_is_empty() {
    let mut state = FinOpsState::default();
    let probes = state.begin_refresh(context(), &["europe-west1-b".to_string()]);
    for (recommender, location, ticket) in probes {
        state.install_cell(ticket, recommender, &location, CellState::Empty);
    }
    assert_eq!(state.card_state(RecommenderId::IdleVm), CardState::Empty);

    let mut mixed = FinOpsState::default();
    let probes = mixed.begin_refresh(context(), &["europe-west1-b".to_string()]);
    for (recommender, location, ticket) in probes {
        mixed.install_cell(ticket, recommender, &location, CellState::Ready(Vec::new()));
    }
    assert_eq!(mixed.card_state(RecommenderId::IdleVm), CardState::Empty);
}

/// One refused cell beside one that answered nothing is not Unavailable: a
/// location did answer, and the card is Empty with its coverage stating the
/// gap.
#[test]
fn a_refusal_beside_an_answered_nothing_is_empty_with_coverage_not_unavailable() {
    let mut state = FinOpsState::default();
    let probes = state.begin_refresh(
        context(),
        &["europe-west1-b".to_string(), "us-central1-a".to_string()],
    );
    for (recommender, location, ticket) in probes {
        if recommender != RecommenderId::IdleVm {
            continue;
        }
        let cell = if location == "europe-west1-b" {
            CellState::Empty
        } else {
            CellState::Unavailable {
                kind: GcloudErrorKind::PermissionDenied,
                detail: None,
            }
        };
        state.install_cell(ticket, recommender, &location, cell);
    }
    assert_eq!(state.card_state(RecommenderId::IdleVm), CardState::Empty);
    assert_eq!(state.coverage(RecommenderId::IdleVm).refused.len(), 1);
}

/// A result from a superseded refresh is dropped rather than folded in. A
/// stale failure otherwise reads as a failure of the configuration now
/// active, which was never tried — and money makes that bug expensive.
#[test]
fn a_result_from_a_superseded_refresh_is_dropped() {
    let mut state = FinOpsState::default();
    let first = state.begin_refresh(context(), &["europe-west1-b".to_string()]);
    let (recommender, location, stale_ticket) = first[0].clone();

    state.begin_refresh(context(), &["europe-west1-b".to_string()]);

    let installed = state.install_cell(
        stale_ticket,
        recommender,
        &location,
        CellState::Ready(vec![recommendation(99_000_000_000)]),
    );
    assert!(!installed, "a superseded ticket must not install");
    assert!(state.recommendations(recommender).is_empty());
}

/// A result for a cell this refresh never issued is dropped too — the fanout
/// is derived per refresh, so a location that was queried last time may not
/// exist in the current grid.
#[test]
fn a_result_for_a_location_this_refresh_did_not_query_is_dropped() {
    let mut state = FinOpsState::default();
    let probes = state.begin_refresh(context(), &["europe-west1-b".to_string()]);
    let (recommender, _, ticket) = probes[0].clone();

    assert!(!state.install_cell(
        ticket,
        recommender,
        "us-central1-a",
        CellState::Ready(vec![recommendation(1)]),
    ));
    assert!(state.recommendations(recommender).is_empty());
}

/// The workspace total is the per-currency sum across every card, with each
/// xor group counted once — savings_total's contract, applied to the grid.
#[test]
fn the_total_sums_every_ready_cell_once() {
    let mut state = FinOpsState::default();
    let probes = state.begin_refresh(context(), &["europe-west1-b".to_string()]);
    for (recommender, location, ticket) in probes {
        state.install_cell(
            ticket,
            recommender,
            &location,
            CellState::Ready(vec![recommendation(10_000_000_000)]),
        );
    }
    let parts = state.total().into_parts();
    assert_eq!(parts.len(), 1);
    assert_eq!(parts[0].nanos, 40_000_000_000, "four recommenders, 10 each");
}

/// A refresh replaces the grid rather than adding to it, so a location that
/// disappeared from the resource rows stops being reported as covered.
#[test]
fn a_refresh_replaces_the_previous_grid() {
    let mut state = FinOpsState::default();
    let probes = state.begin_refresh(context(), &["europe-west1-b".to_string()]);
    for (recommender, location, ticket) in probes {
        state.install_cell(
            ticket,
            recommender,
            &location,
            CellState::Ready(vec![recommendation(10_000_000_000)]),
        );
    }

    state.begin_refresh(context(), &["us-central1-a".to_string()]);
    assert!(state.recommendations(RecommenderId::IdleVm).is_empty());
    assert_eq!(state.card_state(RecommenderId::IdleVm), CardState::Loading);
    assert!(state.coverage(RecommenderId::IdleVm).answered.is_empty());
}

/// The grid remembers which context it was filled for, so a later refresh can
/// tell a stale grid from an empty one without consulting the reducer.
#[test]
fn the_grid_records_the_context_it_was_refreshed_for() {
    let mut state = FinOpsState::default();
    assert!(state.context().is_none());

    state.begin_refresh(context(), &["europe-west1-b".to_string()]);
    assert_eq!(
        state.context().and_then(|context| context.project.clone()),
        Some("proj".to_string())
    );
}
