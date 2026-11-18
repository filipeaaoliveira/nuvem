//! Per-service health history: the bounded, context-stamped record of what
//! each service's listing returned, poll after poll.
//!
//! Home already projects every service's *current* row count
//! (`crate::app::projection::home::project_inventory`). What it cannot show
//! is where that count came from — whether nine instances is nine steady, nine
//! climbing, or nine left after twenty. This module is the substrate for that
//! second question, and nothing here renders: it holds the series, and the UI
//! slices read it.
//!
//! # Why every sample carries its context
//!
//! nuvem switches gcloud configuration and project underneath a live session.
//! A ring keyed on the service alone would hold project A's counts and paint
//! them under project B's label the moment the operator switched — and it would
//! look completely plausible, because a sparkline is the one widget where wrong
//! data is indistinguishable from right data. So a sample is
//! `(total, context, at)` and a series is only ever read *for* a context. This
//! is the same scoping [`ResourcesState::service_stale`] applies to freshness
//! and [`ResourcesState::list_load_failed_for`] applies to failures.
//!
//! # Why the sampler observes rather than being called on the result
//!
//! There are thirteen list-result variants and no single reducer arm they all
//! pass through. Hooking each of them would be a hand-written service list of
//! exactly the kind that has bitten before — three services shipped with no Home
//! entry and nothing failed. [`observe_services`] instead walks
//! [`ResourceServiceKind::ALL`] and reads the accessors that are already
//! exhaustive, so a service that forgets to answer does not compile. It can be
//! called as often as the caller likes; [`ServiceHealthHistory::record`]
//! decides what is new.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use nuvem_domain::gcp::configuration::GcloudContext;
use nuvem_domain::gcp::resource::ResourceServiceKind;

use super::resources::{ResourceLoadPhase, ResourcesState};

/// How many samples one *series* keeps — one service, in one context.
///
/// This is the sparkline's width budget, not "as much as we can hold": the
/// Inventory band gives a service one line, and the widest useful spark on that
/// line is a few dozen columns.
///
/// Per (service, context) rather than per service, because that is the unit a
/// series is read at. A ring shared across contexts would give an operator
/// alternating two configurations half a sparkline each, and enough polling in
/// one context would evict another's history entirely — leaving a service that
/// has been watched all day rendering exactly like one never listed.
pub const HISTORY_CAPACITY: usize = 32;

/// How many contexts one service keeps a series for.
///
/// The bound that replaces the one a single shared ring used to provide. It
/// buys an operator moving between a handful of configurations their history
/// back when they return, and it keeps the total cost stated rather than
/// open-ended: thirteen services times this times [`HISTORY_CAPACITY`],
/// forever, is the entire memory cost of the feature.
///
/// Four rather than "all of them": a long session can touch many
/// configurations, and a series for one the operator left an hour ago is not
/// something any surface reads. The least recently sampled context is dropped
/// whole rather than trimmed, so what remains is never a partial series
/// pretending to be a complete one.
pub const HISTORY_CONTEXTS: usize = 4;

/// Which way a service's count last moved.
///
/// Only a direction, not a magnitude. The magnitude is already on the row as
/// the count itself, and the trend is already beside it as the spark; what
/// neither says is "this moved just now", which is the whole content of a
/// delta highlight.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeDirection {
    Rose,
    Fell,
}

/// One recorded listing result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HealthSample {
    /// The context this count was taken in. A series is filtered on it; see
    /// the module docs for why the whole context and not the configuration
    /// name.
    pub context: GcloudContext,
    /// The service's unfiltered row count at that moment — the same number
    /// `InventoryCardProjection::total` carries.
    pub total: usize,
    /// When the sample was recorded, as told to [`ServiceHealthHistory::record`]
    /// by its caller. Never read from a clock in here: the precedent is
    /// [`crate::app::state::tunnels`], where taking `at: Instant` is what makes
    /// the age-dependent behaviour assertable without sleeping.
    pub at: Instant,
}

/// A settled listing result, as [`observe_services`] found it.
///
/// "Settled" is load-bearing: a fetch still in flight and a fetch that failed
/// are both absent from the observation set, because neither produced a count.
/// Recording a zero for a failed poll would paint an outage as a drop to zero,
/// which is a different and much more alarming claim.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceObservation {
    pub kind: ResourceServiceKind,
    pub context: GcloudContext,
    pub total: usize,
    /// The load that produced this result, from
    /// [`ResourcesState::begin_load`]. The identity that lets
    /// [`ServiceHealthHistory::record`] tell a result it has already recorded
    /// from a fresh one — which the count cannot do, since a re-list returning
    /// the same number is a real second sample.
    pub sequence: u64,
}

/// One service's samples in one context.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ContextSeries {
    context: GcloudContext,
    samples: VecDeque<HealthSample>,
}

/// One service's series, most recently sampled last, plus the identity of the
/// last result folded into any of them.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ServiceRing {
    kind: ResourceServiceKind,
    /// At most [`HISTORY_CONTEXTS`] entries, ordered by when each was last
    /// appended to — so the front is the one to drop.
    series: Vec<ContextSeries>,
    /// `None` until this service has recorded anything. Not `0`: zero is a
    /// legitimate sequence for a collection that has never been loaded, so a
    /// zero sentinel would make the first real load look like a repeat.
    ///
    /// Per service rather than per context, because it identifies a *load*, and
    /// load sequences are minted from one counter shared across every service.
    last_recorded: Option<u64>,
}

/// Every service's history.
///
/// Constructed from [`ResourceServiceKind::ALL`], so every registered service
/// has a ring whether or not anything has ever been listed for it — a kind the
/// history did not know about would report "no history" forever, silently.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceHealthHistory {
    rings: Vec<ServiceRing>,
}

impl Default for ServiceHealthHistory {
    fn default() -> Self {
        Self {
            rings: ResourceServiceKind::ALL
                .into_iter()
                .map(|kind| ServiceRing {
                    kind,
                    series: Vec::new(),
                    last_recorded: None,
                })
                .collect(),
        }
    }
}

impl ServiceHealthHistory {
    /// Folds `observation` in, and reports whether it became a new sample.
    ///
    /// Ignores a result already recorded, because the caller is expected to
    /// offer the whole settled set on every reduction rather than once per
    /// landed fetch. Without that guard a busy UI would fill the ring with
    /// copies of a single poll, and the spark would read as a long steady run
    /// that never happened.
    ///
    /// What it does *not* do is decide whether the observation belongs to the
    /// context the operator is looking at. It does not have to: the stamp
    /// travels with the sample and [`Self::samples_in`] filters on read, so a
    /// result that lands just after a context switch is kept under the context
    /// it was actually taken in rather than dropped or mislabelled.
    pub fn record(&mut self, observation: ServiceObservation, at: Instant) -> bool {
        let Some(ring) = self
            .rings
            .iter_mut()
            .find(|ring| ring.kind == observation.kind)
        else {
            return false;
        };
        if ring.last_recorded == Some(observation.sequence) {
            return false;
        }
        ring.last_recorded = Some(observation.sequence);

        // Moved to the back on every append, so the front is always the least
        // recently sampled context and eviction needs no timestamps of its own.
        let existing = ring
            .series
            .iter()
            .position(|series| series.context == observation.context);
        let mut series = match existing {
            Some(index) => ring.series.remove(index),
            None => ContextSeries {
                context: observation.context.clone(),
                samples: VecDeque::new(),
            },
        };
        if series.samples.len() == HISTORY_CAPACITY {
            series.samples.pop_front();
        }
        series.samples.push_back(HealthSample {
            context: observation.context,
            total: observation.total,
            at,
        });
        ring.series.push(series);
        // Dropped whole rather than trimmed: half a context's series read as a
        // complete one, and there is nothing on the line to say otherwise.
        while ring.series.len() > HISTORY_CONTEXTS {
            ring.series.remove(0);
        }
        true
    }

    /// `kind`'s samples taken in `context`, oldest first.
    pub fn samples_in<'a>(
        &'a self,
        kind: ResourceServiceKind,
        context: &'a GcloudContext,
    ) -> impl DoubleEndedIterator<Item = &'a HealthSample> {
        self.rings
            .iter()
            .filter(move |ring| ring.kind == kind)
            .flat_map(|ring| ring.series.iter())
            .filter(move |series| &series.context == context)
            .flat_map(|series| series.samples.iter())
    }

    /// Which way `kind`'s count last moved in `context`, if it moved inside
    /// `window` of `at`.
    ///
    /// `at` and `window` are parameters rather than an `Instant::now()` and a
    /// constant in here, for the reason [`super::tunnels`] takes an `at`: it is
    /// what makes the decay assertable without sleeping. A test that slept for
    /// the window would be slow, and one that shrank the window to avoid
    /// sleeping would not be testing the window.
    ///
    /// The comparison is against the previous sample **in the same context**,
    /// which is what `samples_in` already guarantees: a project switch moves
    /// the counts on screen without anything having changed in either project,
    /// and reading that as a change would flash every service on every switch.
    ///
    /// Only the newest pair is considered. An older change still inside the
    /// window would otherwise keep a marker alive after the count had settled
    /// back — the mark would then say "moving" about something at rest. For the
    /// same reason a repeated count clears it: a poll that returned the same
    /// number is not a change, and a marker that fired on every poll would mean
    /// "we polled", which teaches the operator to stop reading it.
    ///
    /// `None` for a service with a single sample: the first has nothing to have
    /// changed from, and reading it as a rise from zero would flash every
    /// service once at startup — true of the history, false of the
    /// infrastructure.
    pub fn recent_change_in(
        &self,
        kind: ResourceServiceKind,
        context: &GcloudContext,
        at: Instant,
        window: Duration,
    ) -> Option<ChangeDirection> {
        let mut newest = self.samples_in(kind, context).rev();
        let last = newest.next()?;
        let previous = newest.next()?;
        if at.saturating_duration_since(last.at) >= window {
            return None;
        }
        match last.total.cmp(&previous.total) {
            std::cmp::Ordering::Greater => Some(ChangeDirection::Rose),
            std::cmp::Ordering::Less => Some(ChangeDirection::Fell),
            std::cmp::Ordering::Equal => None,
        }
    }

    /// `kind`'s counts in `context`, oldest first — the series a sparkline
    /// draws.
    pub fn series_in(&self, kind: ResourceServiceKind, context: &GcloudContext) -> Vec<usize> {
        self.samples_in(kind, context)
            .map(|sample| sample.total)
            .collect()
    }
}

/// Every service holding a settled listing result, with the identity of the
/// load that produced it.
///
/// Derived from [`ResourceServiceKind::ALL`] rather than from a list of
/// services someone remembered to extend — see the module docs. A service is
/// present when it has a loaded context *and* its load phase is
/// [`ResourceLoadPhase::Idle`]; `Loading` means the rows still on hand belong
/// to the previous result, and `Failed` means there is no count at all.
///
/// A service with a filter applied is skipped entirely, and that is the second
/// thing this function is careful about. A filter is pushed down to the backend
/// as `--filter=<expression>`, so the rows a filtered listing lands are a
/// *subset* of the population rather than the population. Sampled into the same
/// series as the unfiltered ones, typing a filter would draw a cliff and
/// clearing it a recovery, neither of which happened to anything in the
/// project — the context stamp's failure mode, one axis over.
///
/// Skipped rather than recorded under a filter-aware identity, because a
/// filtered series answers a question nobody asks of a health dashboard: "how
/// many instances match `name~web`" is a question about a query, and the
/// Inventory line is about a service. The count only means "how many of these
/// exist" while nothing is narrowing it, so while something is, there is no
/// sample to take. It resumes the moment the filter is cleared; it is a
/// property of what the rows currently mean, not a latch.
///
/// The identity is [`ResourcesState::list_landed_sequence`] — the result held,
/// not the load started. A fetch the runtime's generation guard abandons
/// resolves the phase to `Idle` under a sequence nothing landed, and keying on
/// that would re-present the rows already on hand as a fresh observation,
/// stamped with the present moment for a listing that was thrown away.
pub fn observe_services(resources: &ResourcesState) -> Vec<ServiceObservation> {
    ResourceServiceKind::ALL
        .into_iter()
        .filter(|kind| matches!(resources.load_phase(*kind), ResourceLoadPhase::Idle))
        .filter(|kind| resources.filter_applied_for(*kind).is_empty())
        .filter_map(|kind| {
            let context = resources.loaded_list_context(kind)?.clone();
            Some(ServiceObservation {
                kind,
                context,
                total: resources.service_row_count(kind),
                sequence: resources.list_landed_sequence(kind)?,
            })
        })
        .collect()
}
