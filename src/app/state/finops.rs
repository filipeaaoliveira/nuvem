//! The FinOps grid: one cell per `(recommender, location)` probe.
//!
//! The unit of failure is finer than a card. A card ("Idle VMs") is one
//! recommender across many zones, and `europe-west1-b` can answer while
//! `us-central1-a` is denied. A card-level state cannot express that, and
//! rolling it up to "failed" throws away the zones that answered — so the
//! state is per cell and the card state is a fold over its cells.

use nuvem_domain::gcp::configuration::GcloudContext;
use nuvem_domain::gcp::diagnostics::GcloudErrorKind;
use nuvem_domain::gcp::finops::{
    CostRecommendation, RecommenderId, query_locations, savings_total,
};
use nuvem_domain::gcp::money::MoneyTotal;

use crate::services::resources::finops::Budget;

/// One `(recommender, location)` probe.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CellState {
    /// Effect dispatched, no result yet.
    Loading,
    /// The call answered with rows.
    Ready(Vec<CostRecommendation>),
    /// The call answered, and the answer was genuinely nothing.
    /// Only ever installed from exit status 0. Never from an empty stdout.
    Empty,
    /// The call was refused. Carries the classified kind so the card can
    /// render "enable the API" differently from "check IAM". The kind alone,
    /// so a missing-binary refusal reads "missing CLI" — see [`CardState`].
    Unavailable {
        kind: GcloudErrorKind,
        detail: Option<String>,
    },
}

/// The budget card's own state.
///
/// Separate from the recommendation grid rather than another cell in it: the
/// Budgets API is a different service with its own refusal, and folding it in
/// would let a denied budgets call blank a savings card that answered.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum BudgetsState {
    /// Nothing has asked yet.
    #[default]
    NotProbed,
    Loading,
    Ready(Vec<Budget>),
    /// The project is not billed against any account, so no budget will ever
    /// appear here. Not the same as an empty list, which means nobody set one.
    NoBillingAccount,
    /// The kind alone, for the same reason and with the same consequence as
    /// [`CardState::Unavailable`]: the budgets headline and remedy read
    /// "missing CLI" for a missing binary, and the health card is what names
    /// it.
    Unavailable {
        kind: GcloudErrorKind,
        detail: Option<String>,
    },
}

/// A card's state, folded from its cells.
///
/// FinOps stores the bare kind, not the failure carrier, so its surfaces render
/// `GcloudErrorKind::label`/`user_action` alone. A missing-binary failure
/// therefore reads the binary-agnostic "missing CLI" here rather than naming
/// `gcloud` or `bq` — deliberately. Only a carrier
/// (`GcloudFailure`/`DiagnosticProblem`/`GcloudDiagnostic`) records which
/// binary was missing, and the Home health card is the surface that has one:
/// the availability probe names it there, in the same frame, so the reader is
/// never left without the word. Keeping "missing gcloud CLI" in the kind's own
/// label instead would have been precise and, for a missing `bq`, wrong.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CardState {
    /// No cell exists, because no location was worth querying. Distinct from
    /// [`CardState::Empty`] on purpose: "we never looked" is not "there is
    /// nothing to save", and only one of those is good news.
    NotProbed,
    Loading,
    Ready,
    Empty,
    Unavailable(GcloudErrorKind),
}

/// What a card's figure actually covers.
///
/// A card that totals three zones out of five says so on its face. A total
/// whose scope is unstated is the same false claim as a `$0` from a denied
/// call, one step further along.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Coverage {
    pub answered: Vec<String>,
    pub refused: Vec<(String, GcloudErrorKind)>,
}

/// Identity of one in-flight refresh.
///
/// Mirrors `ResourceLoadTicket`: a private sequence minted on dispatch and
/// compared on arrival, so a result from a previous gcloud configuration is
/// dropped rather than folded into the current one. Plain `Copy` data with no
/// borrows, so the runtime can move it into a spawned task and hand it back.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FinOpsTicket {
    sequence: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FinOpsState {
    context: Option<GcloudContext>,
    cells: Vec<(RecommenderId, String, CellState)>,
    budgets: BudgetsState,
    sequence: u64,
}

impl FinOpsState {
    /// Clears the grid, seeds one `Loading` cell per probe, and returns the
    /// probes to dispatch.
    ///
    /// An empty return is not a failure: it means nothing has been listed yet,
    /// and every card then reads [`CardState::NotProbed`] rather than a total
    /// of zero.
    pub fn begin_refresh(
        &mut self,
        context: GcloudContext,
        seen_locations: &[String],
    ) -> Vec<(RecommenderId, String, FinOpsTicket)> {
        self.sequence += 1;
        self.context = Some(context);
        self.cells.clear();
        self.budgets = BudgetsState::Loading;

        let ticket = FinOpsTicket {
            sequence: self.sequence,
        };
        let mut probes = Vec::new();
        for recommender in RecommenderId::ALL {
            for location in query_locations(recommender, seen_locations) {
                self.cells
                    .push((recommender, location.clone(), CellState::Loading));
                probes.push((recommender, location, ticket));
            }
        }
        probes
    }

    /// Folds one probe's result in. Returns false — and changes nothing — when
    /// the ticket belongs to a superseded refresh, or when the current grid has
    /// no such cell because the fanout was derived differently this time.
    pub fn install_cell(
        &mut self,
        ticket: FinOpsTicket,
        recommender: RecommenderId,
        location: &str,
        cell: CellState,
    ) -> bool {
        if ticket.sequence != self.sequence {
            return false;
        }
        let Some(slot) = self
            .cells
            .iter_mut()
            .find(|(id, at, _)| *id == recommender && at == location)
        else {
            return false;
        };
        slot.2 = cell;
        true
    }

    /// The ticket the current refresh minted. Used by the reducer to tag the
    /// budgets effect, which is issued alongside the cells rather than by
    /// `begin_refresh` — budgets are billing-account scoped and have no
    /// `(recommender, location)` to be a cell of.
    pub fn current_ticket(&self) -> FinOpsTicket {
        FinOpsTicket {
            sequence: self.sequence,
        }
    }

    /// Folds the budgets result in, under the same ticket discipline as a
    /// cell: a result from a superseded refresh changes nothing.
    pub fn install_budgets(&mut self, ticket: FinOpsTicket, budgets: BudgetsState) -> bool {
        if ticket.sequence != self.sequence {
            return false;
        }
        self.budgets = budgets;
        true
    }

    pub fn budgets(&self) -> &BudgetsState {
        &self.budgets
    }

    pub fn context(&self) -> Option<&GcloudContext> {
        self.context.as_ref()
    }

    fn cells_of(
        &self,
        recommender: RecommenderId,
    ) -> impl Iterator<Item = &(RecommenderId, String, CellState)> {
        self.cells
            .iter()
            .filter(move |(id, _, _)| *id == recommender)
    }

    /// Folds the card's cells into one state.
    ///
    /// The order of the branches is the design: any cell still in flight keeps
    /// the card Loading (so it fills progressively), a card whose every cell
    /// was refused is Unavailable, and anything else that answered makes the
    /// card Ready or Empty. A single refusal beside a real answer therefore
    /// never blanks the card — it shows up in [`Self::coverage`] instead.
    pub fn card_state(&self, recommender: RecommenderId) -> CardState {
        let mut any = false;
        let mut loading = false;
        let mut ready = false;
        let mut refused_kind = None;
        let mut all_refused = true;

        for (_, _, cell) in self.cells_of(recommender) {
            any = true;
            match cell {
                CellState::Loading => {
                    loading = true;
                    all_refused = false;
                }
                CellState::Ready(rows) => {
                    all_refused = false;
                    if !rows.is_empty() {
                        ready = true;
                    }
                }
                CellState::Empty => all_refused = false,
                CellState::Unavailable { kind, .. } => {
                    refused_kind.get_or_insert(*kind);
                }
            }
        }

        if !any {
            return CardState::NotProbed;
        }
        if loading {
            return CardState::Loading;
        }
        if all_refused {
            // Unreachable with a None: `all_refused` survives only when every
            // cell took the Unavailable arm, and each of those sets the kind.
            return CardState::Unavailable(refused_kind.unwrap_or(GcloudErrorKind::Unknown));
        }
        if ready {
            return CardState::Ready;
        }
        CardState::Empty
    }

    /// Which locations answered and which refused. A cell still in flight is
    /// neither: reporting it as refused would make a mid-refresh card claim a
    /// gap that may not exist.
    pub fn coverage(&self, recommender: RecommenderId) -> Coverage {
        let mut coverage = Coverage::default();
        for (_, location, cell) in self.cells_of(recommender) {
            match cell {
                CellState::Ready(_) | CellState::Empty => coverage.answered.push(location.clone()),
                CellState::Unavailable { kind, .. } => {
                    coverage.refused.push((location.clone(), *kind))
                }
                CellState::Loading => {}
            }
        }
        coverage
    }

    pub fn recommendations(&self, recommender: RecommenderId) -> Vec<&CostRecommendation> {
        self.cells_of(recommender)
            .filter_map(|(_, _, cell)| match cell {
                CellState::Ready(rows) => Some(rows.iter()),
                _ => None,
            })
            .flatten()
            .collect()
    }

    /// Per-currency total across every card, each xor group counted once.
    pub fn total(&self) -> MoneyTotal {
        let owned: Vec<CostRecommendation> = RecommenderId::ALL
            .iter()
            .flat_map(|recommender| self.recommendations(*recommender))
            .cloned()
            .collect();
        savings_total(&owned)
    }
}
