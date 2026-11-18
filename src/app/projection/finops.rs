//! The FinOps workspace projection.
//!
//! Every number this produces is already rounded, already sign-corrected and
//! already scoped: a card totalling two zones out of four carries the coverage
//! line saying so, because a total whose scope is unstated is the same false
//! claim as a `$0` from a denied call, one step further along.

use std::time::Instant;

use nuvem_domain::gcp::finops::{RecommenderId, savings_total};

use crate::app::projection::InventoryCardProjection;
use crate::app::state::finops::{BudgetsState, CardState, Coverage};
use crate::app::state::root::RootState;
use crate::services::resources::finops::Budget;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FinOpsRowProjection {
    /// The API's own free-form English, verbatim, so the claim stays
    /// attributable to Google rather than re-worded by this app.
    pub description: String,
    pub monthly: Option<String>,
    pub priority: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FinOpsCardProjection {
    pub title: String,
    pub state: CardState,
    pub total: Option<String>,
    pub coverage_line: Option<String>,
    pub rows: Vec<FinOpsRowProjection>,
    pub remedy: Option<String>,
}

/// One budget, as much of one as the API returns.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BudgetProjection {
    pub display_name: String,
    /// The amount, or the rule where there is no amount to show. `None` only
    /// when the budget carries neither, which the API should not produce.
    pub amount: Option<String>,
    pub thresholds: String,
    /// Present when the budget covers the whole billing account rather than
    /// this project.
    pub scope_note: Option<String>,
}

/// What the budget card says when it has no budgets to list. Each of these is
/// a different fact and none of them is "nothing to worry about".
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BudgetsNote {
    NotProbed,
    Loading,
    /// The account has no budgets configured.
    None,
    /// Nothing is billed against this project, so no budget will ever appear.
    NoBillingAccount,
    Unavailable {
        headline: String,
        remedy: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FinOpsProjection {
    pub total_line: String,
    pub cards: Vec<FinOpsCardProjection>,
    pub budgets: Vec<BudgetProjection>,
    /// The global resource inventory, from the same walk Home renders. Called
    /// rather than duplicated: a second walk would drift the moment a service
    /// is added, which is exactly how three services once shipped with no Home
    /// entry.
    pub inventory: Vec<InventoryCardProjection>,
    /// `Some` whenever `budgets` is empty, saying which kind of empty it is.
    pub budgets_note: Option<BudgetsNote>,
}

/// What the workspace shows when no cell has been probed.
///
/// Deliberately not a zero: "Potential savings: USD 0.00" on a workspace that
/// never looked is the false good news this whole feature is built to avoid.
const NOT_PROBED_TOTAL: &str = "Potential savings: not probed";

/// Why a card has nothing to show, when the reason is the fanout rather than a
/// refusal. The coupling the derived-location fanout buys is on screen here
/// rather than left for the operator to deduce from an empty card.
const NOT_PROBED_REMEDY: &str =
    "Open Resources and list instances or disks first — FinOps queries the locations they are in.";

/// Projects the FinOps workspace as of `at`.
///
/// `at` rides through only because the shared inventory walk carries decaying
/// delta markers; nothing else here is time-dependent. It is threaded
/// rather than read locally so the FinOps inventory and Home's cannot disagree
/// about what "just now" means inside one frame.
pub fn project_finops(root: &RootState, at: Instant) -> FinOpsProjection {
    let cards = RecommenderId::ALL
        .iter()
        .map(|recommender| card(root, *recommender))
        .collect();

    let parts = root.finops.total().into_parts();
    let total_line = if parts.is_empty() {
        NOT_PROBED_TOTAL.to_string()
    } else {
        let rendered: Vec<String> = parts.iter().map(|part| part.render()).collect();
        format!("Potential savings: {}/mo", rendered.join(" + "))
    };

    let (budgets, budgets_note) = budgets(root.finops.budgets());

    FinOpsProjection {
        total_line,
        cards,
        budgets,
        budgets_note,
        inventory: crate::app::projection::home::project_inventory(root, at),
    }
}

/// There is no spend gauge here and there cannot be one: the budgets resource
/// carries `amount` and `thresholdRules` and no spend field anywhere, so the
/// card says what the budget IS and never implies a position against it. A
/// gauge would need the BigQuery billing export.
fn budgets(state: &BudgetsState) -> (Vec<BudgetProjection>, Option<BudgetsNote>) {
    match state {
        BudgetsState::NotProbed => (Vec::new(), Some(BudgetsNote::NotProbed)),
        BudgetsState::Loading => (Vec::new(), Some(BudgetsNote::Loading)),
        BudgetsState::NoBillingAccount => (Vec::new(), Some(BudgetsNote::NoBillingAccount)),
        BudgetsState::Unavailable { kind, .. } => (
            Vec::new(),
            Some(BudgetsNote::Unavailable {
                headline: kind.label().to_string(),
                remedy: kind.user_action().to_string(),
            }),
        ),
        BudgetsState::Ready(budgets) if budgets.is_empty() => (Vec::new(), Some(BudgetsNote::None)),
        BudgetsState::Ready(budgets) => (budgets.iter().map(budget).collect(), None),
    }
}

fn budget(budget: &Budget) -> BudgetProjection {
    // lastPeriodAmount has no figure at all, so the rule goes where the amount
    // would be rather than leaving a blank that reads as a missing number.
    let amount = match (&budget.amount, budget.matches_last_period) {
        (Some(money), _) => Some(money.render()),
        (None, true) => Some("matches last period".to_string()),
        (None, false) => None,
    };
    BudgetProjection {
        display_name: budget.display_name.clone(),
        amount,
        thresholds: budget
            .thresholds
            .iter()
            .map(|percent| format!("{percent}%"))
            .collect::<Vec<_>>()
            .join(", "),
        scope_note: budget
            .covers_whole_account
            .then(|| "covers the whole billing account, not just this project".to_string()),
    }
}

fn card(root: &RootState, recommender: RecommenderId) -> FinOpsCardProjection {
    let state = root.finops.card_state(recommender);
    let owned: Vec<_> = root
        .finops
        .recommendations(recommender)
        .into_iter()
        .cloned()
        .collect();

    // A total only where the card is settled. A figure that is still growing
    // reads as final, and nothing on screen would say otherwise.
    let total = match state {
        CardState::Ready => {
            let parts = savings_total(&owned).into_parts();
            (!parts.is_empty()).then(|| {
                parts
                    .iter()
                    .map(|part| part.render())
                    .collect::<Vec<_>>()
                    .join(" + ")
            })
        }
        _ => None,
    };

    let rows = owned
        .iter()
        .map(|recommendation| FinOpsRowProjection {
            description: recommendation.description.clone(),
            monthly: recommendation.monthly.as_ref().map(|money| money.render()),
            priority: recommendation.priority.clone(),
        })
        .collect();

    FinOpsCardProjection {
        title: recommender.label().to_string(),
        state,
        total,
        coverage_line: coverage_line(&root.finops.coverage(recommender)),
        rows,
        remedy: remedy(state),
    }
}

/// Stated only where it can be wrong.
///
/// A card with nothing refused needs no qualifier, and a qualifier present on
/// every card cannot distinguish a partial answer from a complete one — which
/// is the entire job this line exists to do.
///
/// The line renders `kind.label()` alone, because `Coverage` records kinds and
/// not carriers: a refusal caused by a missing binary reads "missing CLI" here.
/// See `state::finops::CardState` for why that is the deliberate direction.
fn coverage_line(coverage: &Coverage) -> Option<String> {
    if coverage.refused.is_empty() {
        return None;
    }
    let refused: Vec<String> = coverage
        .refused
        .iter()
        .map(|(location, kind)| format!("{location} ({})", kind.label()))
        .collect();
    let answered = if coverage.answered.is_empty() {
        "no locations".to_string()
    } else {
        coverage.answered.join(", ")
    };
    Some(format!(
        "covers {answered}; no answer from {}",
        refused.join(", ")
    ))
}

/// The remedy comes from `GcloudErrorKind::user_action` rather than from
/// strings kept here: those sentences are the ones every other degraded
/// surface shows, and a later fix is what made the disabled-API one correct.
fn remedy(state: CardState) -> Option<String> {
    match state {
        CardState::Unavailable(kind) => Some(kind.user_action().to_string()),
        CardState::NotProbed => Some(NOT_PROBED_REMEDY.to_string()),
        CardState::Loading | CardState::Ready | CardState::Empty => None,
    }
}
