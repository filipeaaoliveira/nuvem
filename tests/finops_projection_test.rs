use nuvem::app::projection::finops::project_finops;
use nuvem::app::state::finops::{CardState, CellState};
use nuvem::app::state::root::RootState;
use nuvem::services::resources::finops::Budget;
use nuvem_domain::gcp::configuration::{GcloudConfigurationName, GcloudContext};
use nuvem_domain::gcp::diagnostics::GcloudErrorKind;
use nuvem_domain::gcp::finops::{CostRecommendation, RecommendationState, RecommenderId};
use nuvem_domain::gcp::money::Money;
use std::time::Instant;

fn context() -> GcloudContext {
    GcloudContext {
        configuration: GcloudConfigurationName::new("default"),
        account: None,
        project: Some("proj".to_string()),
        region: None,
        zone: None,
    }
}

fn recommendation(nanos: i128, description: &str) -> CostRecommendation {
    CostRecommendation {
        name: description.to_string(),
        description: description.to_string(),
        state: RecommendationState::Active,
        priority: Some("P2".to_string()),
        xor_group: None,
        monthly: Some(Money {
            currency: "USD".to_string(),
            nanos,
        }),
        last_refresh: None,
        target_resources: Vec::new(),
    }
}

/// Fills every cell of every card with one state.
fn filled(root: &mut RootState, locations: &[&str], cell: impl Fn(&str) -> CellState) {
    let owned: Vec<String> = locations.iter().map(|l| l.to_string()).collect();
    let probes = root.finops.begin_refresh(context(), &owned);
    for (recommender, location, ticket) in probes {
        let state = cell(&location);
        root.finops
            .install_cell(ticket, recommender, &location, state);
    }
}

/// A card whose figure covers one zone out of two says so. An unstated scope
/// is the same false claim as a $0 from a denied call.
#[test]
fn a_partially_covered_card_states_its_coverage() {
    let mut root = RootState::default();
    filled(
        &mut root,
        &["europe-west1-b", "us-central1-a"],
        |location| {
            if location.starts_with("europe") {
                CellState::Ready(vec![recommendation(10_000_000_000, "Stop idle VM")])
            } else {
                CellState::Unavailable {
                    kind: GcloudErrorKind::PermissionDenied,
                    detail: None,
                }
            }
        },
    );

    let projection = project_finops(&root, Instant::now());
    let card = projection
        .cards
        .iter()
        .find(|card| card.title == RecommenderId::IdleVm.label())
        .expect("the idle VM card");

    assert_eq!(card.state, CardState::Ready);
    let coverage = card.coverage_line.as_deref().expect("a coverage line");
    assert!(coverage.contains("europe-west1-b"), "{coverage}");
    assert!(coverage.contains("us-central1-a"), "{coverage}");
    assert!(coverage.contains("permission denied"), "{coverage}");
}

/// A card that answered everywhere carries no coverage qualifier. A qualifier
/// present on every card cannot distinguish a partial answer from a complete
/// one, which is the whole job the line exists to do.
#[test]
fn a_fully_covered_card_carries_no_coverage_qualifier() {
    let mut root = RootState::default();
    filled(&mut root, &["europe-west1-b"], |_| {
        CellState::Ready(vec![recommendation(10_000_000_000, "Stop idle VM")])
    });

    let projection = project_finops(&root, Instant::now());
    assert!(
        projection
            .cards
            .iter()
            .all(|card| card.coverage_line.is_none())
    );
}

/// A disabled API says "enable it". This is what the classifier bought: before it,
/// every one of these read "check IAM roles" for an API that was simply off.
#[test]
fn a_disabled_api_card_offers_the_enable_remedy_not_an_iam_audit() {
    let mut root = RootState::default();
    filled(&mut root, &["europe-west1-b"], |_| CellState::Unavailable {
        kind: GcloudErrorKind::ApiDisabled,
        detail: None,
    });

    let projection = project_finops(&root, Instant::now());
    let card = &projection.cards[0];
    assert_eq!(
        card.state,
        CardState::Unavailable(GcloudErrorKind::ApiDisabled)
    );
    let remedy = card.remedy.as_deref().expect("a remedy");
    assert!(remedy.to_lowercase().contains("enable"), "{remedy}");
    assert!(!remedy.to_lowercase().contains("iam"), "{remedy}");
}

/// A genuine permissions refusal still points at IAM. The two remedies must
/// stay distinguishable, which is the reason the cell carries a kind at all.
#[test]
fn a_permission_denied_card_still_points_at_iam() {
    let mut root = RootState::default();
    filled(&mut root, &["europe-west1-b"], |_| CellState::Unavailable {
        kind: GcloudErrorKind::PermissionDenied,
        detail: None,
    });

    let remedy = project_finops(&root, Instant::now()).cards[0]
        .remedy
        .as_deref()
        .expect("a remedy")
        .to_lowercase();
    assert!(remedy.contains("iam"), "{remedy}");
}

/// A card that was never probed shows no total, and says what would make it
/// probe — the coupling the location fanout buys is on screen, not hidden.
#[test]
fn an_unprobed_card_shows_no_total_and_names_what_is_missing() {
    let root = RootState::default();
    let projection = project_finops(&root, Instant::now());
    assert_eq!(projection.cards.len(), 4);
    assert!(projection.cards.iter().all(|card| card.total.is_none()));
    assert!(
        projection
            .cards
            .iter()
            .all(|card| card.state == CardState::NotProbed)
    );

    let remedy = projection.cards[0]
        .remedy
        .as_deref()
        .expect("a reason")
        .to_lowercase();
    assert!(remedy.contains("resources"), "{remedy}");
}

/// Nothing probed is not a zero total. A "Potential savings: USD 0.00" on an
/// unprobed workspace is exactly the false good news this feature is built to
/// avoid.
#[test]
fn an_unprobed_workspace_does_not_render_a_zero_total() {
    let total_line = project_finops(&RootState::default(), Instant::now()).total_line;
    assert!(!total_line.contains("0.00"), "{total_line}");
    assert!(
        total_line.to_lowercase().contains("not probed"),
        "{total_line}"
    );
}

/// The workspace total is rendered once, rounded once, per currency.
#[test]
fn the_workspace_total_renders_each_currency_once() {
    let mut root = RootState::default();
    filled(&mut root, &["europe-west1-b"], |_| {
        CellState::Ready(vec![recommendation(1_500_000_000, "a")])
    });
    let projection = project_finops(&root, Instant::now());
    assert!(
        projection.total_line.contains("USD 6.00"),
        "four cards at 1.50: {}",
        projection.total_line
    );
}

/// A card in flight shows no total: a figure that is still growing reads as
/// final, and the operator has no way to tell the difference.
#[test]
fn a_loading_card_shows_no_total_yet() {
    let mut root = RootState::default();
    let probes = root
        .finops
        .begin_refresh(context(), &["europe-west1-b".to_string()]);
    let (recommender, location, ticket) = probes[0].clone();
    root.finops.install_cell(
        ticket,
        recommender,
        &location,
        CellState::Ready(vec![recommendation(10_000_000_000, "a")]),
    );

    let projection = project_finops(&root, Instant::now());
    let card = projection
        .cards
        .iter()
        .find(|card| card.title == recommender.label())
        .expect("the card");
    assert_eq!(card.state, CardState::Ready);
    assert!(card.total.is_some(), "its one cell answered");

    let other = projection
        .cards
        .iter()
        .find(|card| card.state == CardState::Loading)
        .expect("a card still in flight");
    assert!(other.total.is_none());
}

/// Rows carry the API's own description verbatim, so the claim stays
/// attributable to Google rather than re-worded here.
#[test]
fn rows_carry_the_apis_own_description_and_a_rendered_figure() {
    let mut root = RootState::default();
    filled(&mut root, &["europe-west1-b"], |_| {
        CellState::Ready(vec![recommendation(
            12_340_000_000,
            "Save cost by stopping idle VM instance vm-1",
        )])
    });

    let card = &project_finops(&root, Instant::now()).cards[0];
    assert_eq!(card.rows.len(), 1);
    assert_eq!(
        card.rows[0].description,
        "Save cost by stopping idle VM instance vm-1"
    );
    assert_eq!(card.rows[0].monthly.as_deref(), Some("USD 12.34"));
    assert_eq!(card.rows[0].priority.as_deref(), Some("P2"));
}

/// Every recommender gets its own card, in a stable order, so the screen does
/// not reshuffle between refreshes.
#[test]
fn every_recommender_has_a_card_in_a_stable_order() {
    let titles: Vec<String> = project_finops(&RootState::default(), Instant::now())
        .cards
        .into_iter()
        .map(|card| card.title)
        .collect();
    assert_eq!(
        titles,
        RecommenderId::ALL
            .iter()
            .map(|id| id.label().to_string())
            .collect::<Vec<_>>()
    );
}

/// The budgets resource carries amount and thresholdRules and no spend field
/// anywhere, so there is no gauge to draw. The card says what the budget IS
/// and never implies a position against it.
#[test]
fn a_budget_shows_its_amount_and_thresholds_and_no_gauge() {
    let mut root = RootState::default();
    root.finops.begin_refresh(context(), &[]);
    root.finops.install_budgets(
        root.finops.current_ticket(),
        nuvem::app::state::finops::BudgetsState::Ready(vec![Budget {
            display_name: "Monthly cap".to_string(),
            amount: Some(Money {
                currency: "EUR".to_string(),
                nanos: 500_000_000_000,
            }),
            matches_last_period: false,
            thresholds: vec![50, 90, 100],
            covers_whole_account: false,
        }]),
    );

    let projection = project_finops(&root, Instant::now());
    assert!(projection.budgets_note.is_none());
    let budget = &projection.budgets[0];
    assert_eq!(budget.display_name, "Monthly cap");
    assert_eq!(budget.amount.as_deref(), Some("EUR 500.00"));
    assert_eq!(budget.thresholds, "50%, 90%, 100%");
    assert!(budget.scope_note.is_none());
}

/// An account-wide budget rendered beside one sandbox project misleads unless
/// its scope is on its face.
#[test]
fn an_account_wide_budget_says_it_is_account_wide() {
    let mut root = RootState::default();
    root.finops.begin_refresh(context(), &[]);
    root.finops.install_budgets(
        root.finops.current_ticket(),
        nuvem::app::state::finops::BudgetsState::Ready(vec![Budget {
            display_name: "Everything".to_string(),
            amount: None,
            matches_last_period: true,
            thresholds: vec![100],
            covers_whole_account: true,
        }]),
    );

    let budget = &project_finops(&root, Instant::now()).budgets[0];
    let note = budget.scope_note.as_deref().expect("a scope note");
    assert!(note.to_lowercase().contains("billing account"), "{note}");
    // lastPeriodAmount has no figure at all, so the rule goes where the amount
    // would be rather than leaving a blank that reads as a missing number.
    assert_eq!(budget.amount.as_deref(), Some("matches last period"));
}

/// Four ways to have no budgets on screen, and each is a different fact. None
/// of them is "nothing to worry about", so none of them may render alike.
#[test]
fn every_empty_budget_state_says_which_kind_of_empty_it_is() {
    use nuvem::app::projection::finops::BudgetsNote;
    use nuvem::app::state::finops::BudgetsState;

    let note_for = |state: BudgetsState| {
        let mut root = RootState::default();
        root.finops.begin_refresh(context(), &[]);
        root.finops
            .install_budgets(root.finops.current_ticket(), state);
        project_finops(&root, Instant::now())
            .budgets_note
            .expect("a note")
    };

    assert_eq!(note_for(BudgetsState::Loading), BudgetsNote::Loading);
    assert_eq!(note_for(BudgetsState::Ready(Vec::new())), BudgetsNote::None);
    assert_eq!(
        note_for(BudgetsState::NoBillingAccount),
        BudgetsNote::NoBillingAccount
    );
    assert_eq!(
        note_for(BudgetsState::Unavailable {
            kind: GcloudErrorKind::PermissionDenied,
            detail: None,
        }),
        BudgetsNote::Unavailable {
            headline: "permission denied".to_string(),
            remedy: "Check IAM roles for this account and project".to_string(),
        }
    );

    // An untouched state is its own case: nothing has asked yet.
    assert_eq!(
        project_finops(&RootState::default(), Instant::now())
            .budgets_note
            .expect("a note"),
        BudgetsNote::NotProbed
    );
}

/// The inventory panel is the same walk Home renders, called rather than
/// duplicated: a second walk would drift the moment a service is added, which
/// is exactly how Cloud Storage, Secret Manager and Cloud Functions each once
/// shipped with no Home entry.
#[test]
fn the_inventory_panel_lists_every_resource_service_from_the_shared_walk() {
    use nuvem::app::projection::project_home_overview;
    use nuvem_domain::gcp::resource::ResourceServiceKind;

    let root = RootState::default();
    let projection = project_finops(&root, Instant::now());
    assert_eq!(
        projection.inventory.len(),
        ResourceServiceKind::ALL.len(),
        "derived from the registry, not enumerated"
    );
    assert_eq!(
        projection.inventory,
        project_home_overview(&root, Instant::now()).inventory,
        "the same walk, not a parallel one"
    );
}
