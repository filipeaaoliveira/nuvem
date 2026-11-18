//! Render-facing projection for the Home overview dashboard.

use crate::app::projection::frame::{FrameContextProjection, project_frame_context};
use crate::app::state::health::ChangeDirection;
use crate::app::state::operations::{OperationOutcome, OperationRecord};
use crate::app::state::root::RootState;
use std::borrow::Cow;
use std::time::{Duration, Instant};

use nuvem_domain::gcp::configuration::GcloudContext;
use nuvem_domain::gcp::diagnostics::{DiagnosticProblem, GcloudDiagnostic, sanitize_detail};
use nuvem_domain::gcp::resource::ResourceServiceKind;

/// How many samples a service needs before its history is offered for
/// rendering.
///
/// Two, and the bar is deliberately this low rather than absent. One sample is
/// not a trend: a single bar is drawn at full height against its own maximum,
/// so it reads as "at the top of its range" - a claim about a shape nobody has
/// observed yet. Withholding it leaves a freshly-listed service showing its
/// count with no spark beside it, which says "not enough history" honestly,
/// where a flat line would say "steady".
///
/// Enforced in the projection rather than in the renderer so the rule is
/// assertable without a terminal, and so a second surface rendering the same
/// cards cannot quietly apply a different one.
pub const SPARKLINE_MIN_SAMPLES: usize = 2;

/// How long a service's line keeps its change marker after the count moved.
///
/// Long enough to be seen on a glance that lands a moment after the poll, short
/// enough that it is gone by the next one at any polling interval an operator
/// would choose. A marker that outlived its change would end up on every
/// service that has ever moved, which after an hour is every service, and a
/// permanent mark is not a highlight.
pub const DELTA_HIGHLIGHT_WINDOW: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// Freshness of a service's loaded inventory relative to the active context.
pub enum ResourceFreshness {
    NotLoaded,
    HealthyEmpty,
    Loaded,
    Stale,
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// A single inventory card's projected content.
pub struct InventoryCardProjection {
    /// The service this entry describes. Carried so a consumer can pair a
    /// rendered line back to its service without matching on the label text.
    pub kind: ResourceServiceKind,
    pub service_label: &'static str,
    pub freshness: ResourceFreshness,
    pub total: usize,
    pub status_counts: Vec<(String, usize)>,
    /// `"{label} — {user action}"`, always visible when a problem is present
    /// (AC #5). Never raw stderr/Debug output (AC #3).
    pub problem: Option<String>,
    /// The recovery hint alone, for surfaces that render it separately from
    /// `problem`.
    ///
    /// `Cow` because a missing-binary hint names the binary and so must be
    /// built; every other kind's hint is still the static literal.
    pub user_action: Option<Cow<'static, str>>,
    /// Sanitized, bounded one-line technical detail (AC #6), shown inline
    /// only where layout width allows; never replaces `problem`.
    pub detail_preview: Option<String>,
    /// This service's recent totals **in the active context**, oldest first -
    /// the series the Home sparkline draws.
    ///
    /// Filtered on the active context here, at the one place that knows what
    /// it is, so no renderer can draw a series recorded under a configuration
    /// the operator has since left. That is the failure the whole feature is
    /// shaped around: a spark drawn out of another project's counts looks
    /// exactly like a correct one.
    ///
    /// Empty when the service has fewer than [`SPARKLINE_MIN_SAMPLES`], and
    /// empty is the renderer's instruction to draw nothing at all. `u64`
    /// because that is what `ratatui`'s `Sparkline` takes; the counts
    /// themselves are `usize` row counts.
    pub history: Vec<u64>,
    /// Which way this service's count moved, while that move is recent enough
    /// to be worth marking — `None` once it has decayed, and `None`
    /// for a count that has not moved.
    ///
    /// A direction only. The magnitude is the count itself, the trend is the
    /// spark; what neither says is "this moved just now".
    pub delta: Option<ChangeDirection>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// A single recent-operation line.
pub struct OperationLineProjection {
    pub label: String,
    /// The typed outcome rather than its rendered label: the render layer
    /// styles terminal outcomes differently from in-flight ones, and a string
    /// forces that decision through a `_` fallback where a newly added outcome
    /// would silently inherit whatever style the fallback happens to be. Call
    /// `outcome.label()` for the display text.
    pub outcome: OperationOutcome,
    pub detail: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Gcloud availability/auth health prepared for the overview.
pub struct GcloudHealthProjection {
    pub available: Option<bool>,
    pub account: Option<String>,
    /// `"{label} — {user action}"`, mirroring `InventoryCardProjection::problem`.
    pub problem: Option<String>,
    /// The recovery hint alone. `Cow` for the same reason as
    /// [`InventoryCardProjection::user_action`].
    pub user_action: Option<Cow<'static, str>>,
    /// Sanitized, bounded one-line technical detail, shown inline only where
    /// layout width allows.
    pub detail_preview: Option<String>,
    pub probing: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Everything the Home dashboard needs to render.
pub struct HomeOverviewProjection {
    pub context: FrameContextProjection,
    pub gcloud: GcloudHealthProjection,
    pub inventory: Vec<InventoryCardProjection>,
    pub operations: Vec<OperationLineProjection>,
    /// One line pointing at the FinOps workspace, and only when there is a
    /// figure behind it.
    pub savings_teaser: Option<String>,
}

/// Projects root application state into the Home overview dashboard, as of
/// `at`.
///
/// `at` is a parameter because the delta markers decay: the projection is a
/// function of the state *and the moment it is read*, and taking the clock here
/// rather than reading it inside is what lets a test assert the decay without
/// sleeping through it. The clock read belongs to the render loop, which does
/// it once per frame — see [`crate::ui::render_at`].
pub fn project_home_overview(root: &RootState, at: Instant) -> HomeOverviewProjection {
    let context = project_frame_context(root);

    let gcloud = match root.system.diagnostics.report() {
        Some(report) => {
            let (problem, user_action, detail_preview) = report
                .problem
                .as_ref()
                .map(problem_fields)
                .map_or((None, None, None), |(headline, action, detail)| {
                    (Some(headline), Some(action), detail)
                });
            GcloudHealthProjection {
                available: Some(report.available),
                account: report.active_account.clone(),
                problem,
                user_action,
                detail_preview,
                probing: root.system.diagnostics.is_in_flight(),
            }
        }
        None => GcloudHealthProjection {
            available: None,
            account: None,
            problem: None,
            user_action: None,
            detail_preview: None,
            probing: root.system.diagnostics.is_in_flight(),
        },
    };

    let inventory = project_inventory(root, at);

    let operations = root
        .system
        .operations
        .records()
        .iter()
        .map(operation_line)
        .collect();

    HomeOverviewProjection {
        context,
        gcloud,
        inventory,
        operations,
        savings_teaser: savings_teaser(root),
    }
}

/// Home's contract is "is my context healthy and what is in it", so FinOps
/// gets one line here rather than four cards.
///
/// `None` unless there is a non-zero figure behind it. An unprobed or refused
/// FinOps state shows nothing: "Potential savings: USD 0.00" on Home would be
/// the same false good news the cost screen exists to avoid, and a zero total
/// is indistinguishable from a total nobody was allowed to compute.
fn savings_teaser(root: &RootState) -> Option<String> {
    let parts = root.finops.total().into_parts();
    if parts.is_empty() || parts.iter().all(|part| part.nanos == 0) {
        return None;
    }
    let rendered: Vec<String> = parts.iter().map(|part| part.render()).collect();
    Some(format!(
        "Potential savings: {}/mo — Ctrl-f for detail",
        rendered.join(" + ")
    ))
}

/// The global resource inventory: one card per service, derived from the
/// registry rather than enumerated.
///
/// A hand-written list is how Cloud Storage, Secret Manager and Cloud
/// Functions each shipped with no Home entry, none of which failed anything.
/// Shared with the FinOps workspace, which renders the same walk
/// rather than a second one that would drift the moment a service is added.
pub(crate) fn project_inventory(root: &RootState, at: Instant) -> Vec<InventoryCardProjection> {
    let context = root.current_context();
    ResourceServiceKind::ALL
        .into_iter()
        .map(|kind| {
            inventory_card(
                kind,
                root.resources.has_loaded_list(kind),
                root.resources.service_stale(kind),
                root.resources.service_row_count(kind),
                root.resources.service_status_counts(kind),
                root.resources.service_diagnostic(kind),
                service_pulse(root, kind, context.as_ref(), at),
            )
        })
        .collect()
}

/// `kind`'s drawable series in `context`, or empty when there is not enough of
/// one to be worth drawing.
///
/// Empty with no active context at all, rather than falling back to whatever
/// the history happens to hold: with no configuration active there is no
/// project those counts could belong to, so any series shown would sit under a
/// label naming a context that is not on screen.
/// The two parts of a card that come out of the health history.
///
/// One value rather than two arguments because they are one answer: both are
/// read from the same ring, for the same context, and a caller that had one
/// without the other would be describing a service by a trend with no "just
/// moved", or the reverse.
struct ServicePulse {
    history: Vec<u64>,
    delta: Option<ChangeDirection>,
}

/// `kind`'s drawable series and recent change in `context`.
///
/// Empty and `None` with no active context at all, rather than falling back to
/// whatever the history happens to hold: with no configuration active there is
/// no project those counts could belong to, so anything shown would sit under a
/// label naming a context that is not on screen.
fn service_pulse(
    root: &RootState,
    kind: ResourceServiceKind,
    context: Option<&GcloudContext>,
    at: Instant,
) -> ServicePulse {
    let Some(context) = context else {
        return ServicePulse {
            history: Vec::new(),
            delta: None,
        };
    };
    let series = root.health.series_in(kind, context);
    // A single sample is not a trend: one bar is drawn at full height against
    // its own maximum, so it reads as "at the top of its range" — a claim about
    // a shape nobody has observed yet.
    let history = if series.len() < SPARKLINE_MIN_SAMPLES {
        Vec::new()
    } else {
        series.into_iter().map(|total| total as u64).collect()
    };
    ServicePulse {
        history,
        delta: root
            .health
            .recent_change_in(kind, context, at, DELTA_HIGHLIGHT_WINDOW),
    }
}

/// `total` is passed rather than summed from `status_counts`: a service with
/// no lifecycle-status shape (Cloud Storage, Secret Manager) sums to zero
/// however many rows it holds, which the freshness rule below would read as
/// "loaded (empty)".
fn inventory_card(
    kind: ResourceServiceKind,
    loaded: bool,
    stale: bool,
    total: usize,
    status_counts: Vec<(String, usize)>,
    diagnostic: Option<&GcloudDiagnostic>,
    pulse: ServicePulse,
) -> InventoryCardProjection {
    let freshness = if stale {
        ResourceFreshness::Stale
    } else if !loaded {
        ResourceFreshness::NotLoaded
    } else if total == 0 {
        ResourceFreshness::HealthyEmpty
    } else {
        ResourceFreshness::Loaded
    };
    let (problem, user_action, detail_preview) = match diagnostic {
        Some(diagnostic) => {
            let preview = diagnostic.detail_preview();
            (
                Some(diagnostic.headline()),
                Some(diagnostic.user_action()),
                (!preview.is_empty()).then(|| preview.to_string()),
            )
        }
        None => (None, None, None),
    };
    InventoryCardProjection {
        kind,
        service_label: kind.label(),
        freshness,
        total,
        status_counts,
        problem,
        user_action,
        detail_preview,
        history: pulse.history,
        delta: pulse.delta,
    }
}

fn operation_line(record: &OperationRecord) -> OperationLineProjection {
    OperationLineProjection {
        label: format!("{} {}", record.kind, record.target),
        outcome: record.outcome,
        detail: record.detail.clone(),
    }
}

/// Projects a `DiagnosticProblem` (the gcloud availability/auth probe's
/// carrier — it predates the sanitized `GcloudDiagnostic`, see
/// `services::gcloud::diagnostics`) into the same headline/action/preview
/// shape as `GcloudDiagnostic`. Re-sanitizes `message` defensively: the probe
/// stores raw stderr there, so this is the chokepoint that keeps AC #3
/// (never leak raw stderr/tokens to the UI) true for the health card too.
fn problem_fields(problem: &DiagnosticProblem) -> (String, Cow<'static, str>, Option<String>) {
    let label = problem.label();
    let user_action = problem.user_action();
    let headline = format!("{label} — {user_action}");
    let detail = sanitize_detail(&problem.message);
    let detail_preview = (!detail.is_empty()).then_some(detail);
    (headline, user_action, detail_preview)
}
