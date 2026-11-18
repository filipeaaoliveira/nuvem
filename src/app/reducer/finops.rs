//! The FinOps reducer.
//!
//! Locations come from rows the Resources workspace has already listed
//! (design spec open question 1, resolved to option (c)). Nothing listed means
//! nothing dispatched, and every card renders `NotProbed` — never a guessed
//! zone and never a `$0`.

use crate::app::action::FinOpsAction;
use crate::app::effect::{Effect, EffectResult};
use crate::app::state::finops::{BudgetsState, CellState};
use crate::app::state::root::RootState;

pub fn reduce(root: &mut RootState, action: &FinOpsAction) -> Vec<Effect> {
    match action {
        FinOpsAction::Refresh => refresh(root),
    }
}

fn refresh(root: &mut RootState) -> Vec<Effect> {
    // No active configuration means no project to ask about. Dispatching
    // anyway would query whatever gcloud defaults to, and attribute the answer
    // to a context this app is not showing.
    let Some(context) = root.current_context() else {
        return Vec::new();
    };
    let seen = root.resources.listed_locations();
    let probes = root.finops.begin_refresh(context.clone(), &seen);

    // The budgets call goes out even when no location was worth probing: it is
    // billing-account scoped, so it has nothing to do with which zones hold
    // rows, and a project with no listed resources can still have a budget.
    let mut effects = vec![Effect::LoadBudgets {
        context: context.clone(),
        ticket: root.finops.current_ticket(),
    }];
    effects.extend(probes.into_iter().map(|(recommender, location, ticket)| {
        Effect::LoadCostRecommendations {
            context: context.clone(),
            recommender,
            location,
            ticket,
        }
    }));
    effects
}

/// Folds one probe's result into the grid. Returns whether anything changed,
/// so a superseded result costs no redraw.
pub fn apply_result(root: &mut RootState, result: EffectResult) -> bool {
    match result {
        EffectResult::CostRecommendationsLoaded {
            ticket,
            recommender,
            location,
            recommendations,
        } => {
            // `Empty` is reachable only from a successful call, and this arm IS
            // that success: the service layer turns a non-zero exit into the
            // Failed result below and never into an empty vector, because every
            // refusal this API has writes `[]` to stdout.
            let cell = if recommendations.is_empty() {
                CellState::Empty
            } else {
                CellState::Ready(recommendations)
            };
            root.finops
                .install_cell(ticket, recommender, &location, cell)
        }
        EffectResult::CostRecommendationsFailed {
            ticket,
            recommender,
            location,
            error,
        } => {
            let cell = CellState::Unavailable {
                kind: error.kind(),
                detail: Some(error.to_string()),
            };
            root.finops
                .install_cell(ticket, recommender, &location, cell)
        }
        EffectResult::BudgetsLoaded { ticket, budgets } => root
            .finops
            .install_budgets(ticket, BudgetsState::Ready(budgets)),
        EffectResult::BudgetsFailed { ticket, error } => {
            // "no billing account" is a fact about the project, not a failure
            // to read one, so it gets its own state rather than an error the
            // operator is invited to retry.
            let state = if error.to_string().contains(NO_BILLING_ACCOUNT) {
                BudgetsState::NoBillingAccount
            } else {
                BudgetsState::Unavailable {
                    kind: error.kind(),
                    detail: Some(error.to_string()),
                }
            };
            root.finops.install_budgets(ticket, state)
        }
        _ => false,
    }
}

/// The sentence `FinOpsLoadError::NoBillingAccount` renders to. Matched rather
/// than re-tested at the raise site because the error crosses an `AppError`
/// boundary that flattens every variant to a string.
const NO_BILLING_ACCOUNT: &str = "no billing account is linked to this project";
