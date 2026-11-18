//! Pure decision logic for the bounded Cloud Logging polling tail.
//!
//! `should_tail` is a side-effect-free function that inspects `RootState`
//! (plus the single-in-flight `pending_background_refreshes` counter kept by
//! `RuntimeSession`) and decides what, if anything, the poll tick should do.
//! Keeping the decision pure makes it unit-testable without async plumbing;
//! `RuntimeSession::poll_active_tail` applies the decision by dispatching the
//! corresponding `Effect`.

use crate::app::state::root::{RootState, Workspace};

/// What the poll tick should do for the Cloud Logging tail this cycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TailDecision {
    /// Fetch the next page using the existing cursor.
    Tail,
    /// No cursor yet: load the first page instead.
    LoadFirstPage,
    /// Nothing to do this tick.
    Skip,
}

/// Decides what the poll tick should do for the Cloud Logging tail.
///
/// Callers are expected to have already decremented the backoff counter for
/// this tick (see `LoggingState::decrement_backoff`) before calling this, so
/// that `root.logging.backoff_ticks` reflects the post-decrement value.
pub fn should_tail(root: &RootState, pending_background_refreshes: usize) -> TailDecision {
    if root.workspace != Workspace::Logging {
        return TailDecision::Skip;
    }
    if !root.logging.tail_is_running() {
        return TailDecision::Skip;
    }
    if pending_background_refreshes != 0 {
        return TailDecision::Skip;
    }
    if root.current_context().is_none() {
        return TailDecision::Skip;
    }
    if root.logging.backoff_ticks > 0 {
        return TailDecision::Skip;
    }
    if root.logging.cursor.is_none() {
        return TailDecision::LoadFirstPage;
    }
    TailDecision::Tail
}
