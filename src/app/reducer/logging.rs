use crate::app::state::root::RootState;
use crate::app::{Effect, LoggingAction};
use nuvem_domain::gcp::logging::{LogPayload, sanitize_log_text};

pub fn reduce(state: &mut RootState, action: &LoggingAction) -> Vec<Effect> {
    match action {
        LoggingAction::Refresh => return refresh(state),
        LoggingAction::ToggleTail => state.logging.toggle_tail(),
        LoggingAction::CycleTimeRange => {
            state.logging.cycle_time_range();
            return load_recent_or_settle(state, UnloadedBuffer::Discard);
        }
        // The floor/range mutators report whether they actually changed the
        // window. A no-op step (clamped severity, already-active bucket) keeps
        // the buffer, so it must not trigger a refetch either.
        LoggingAction::SetTimeRange(range) => {
            if state.logging.set_time_range(*range) {
                return load_recent_or_settle(state, UnloadedBuffer::Discard);
            }
        }
        LoggingAction::RaiseSeverityFloor => {
            if state.logging.raise_severity_floor() {
                return load_recent_or_settle(state, UnloadedBuffer::Discard);
            }
        }
        LoggingAction::LowerSeverityFloor => {
            if state.logging.lower_severity_floor() {
                return load_recent_or_settle(state, UnloadedBuffer::Discard);
            }
        }
        LoggingAction::ToggleWrap => state.logging.toggle_wrap(),
        LoggingAction::SelectNext => state.logging.select_next(),
        LoggingAction::SelectPrevious => state.logging.select_previous(),
        LoggingAction::OpenDetail => state.logging.open_detail(),
        LoggingAction::CloseDetail => state.logging.close_detail(),
        LoggingAction::ScrollDetailDown => state.logging.scroll_detail(1),
        LoggingAction::ScrollDetailUp => state.logging.scroll_detail(-1),
        LoggingAction::StartFilter => state.logging.start_filter(),
        LoggingAction::AppendFilterChar(c) => state.logging.append_filter_char(*c),
        LoggingAction::BackspaceFilter => state.logging.backspace_filter(),
        LoggingAction::ApplyFilter => state.logging.apply_filter(),
        LoggingAction::CancelFilter => state.logging.cancel_filter(),
        LoggingAction::CopySelectedEntry => return copy_selected_entry(state),
        LoggingAction::CopySelectedInsertId => return copy_selected_insert_id(state),
        LoggingAction::SaveToFile => return save_visible_entries(state),
    }
    Vec::new()
}

pub(super) use dispatch::{UnloadedBuffer, load_recent_or_settle};

/// Wraps the recent-logs fetch so that the effect builder has no name outside
/// these braces. `load_recent_effect` is private to this module and only
/// `load_recent_or_settle` and [`UnloadedBuffer`] leave it, so nothing --
/// including the `reduce` arms in the parent file, where a new `LoggingAction`
/// that reloads the window would most plausibly be written -- can dispatch the
/// fetch without also running the settle. Module privacy alone would not do
/// this: it stops other modules, not the rest of this file.
mod dispatch {
    use super::*;

    /// What an existing buffer is worth to a caller whose fetch was never
    /// dispatched. It is the only thing the callers of
    /// [`load_recent_or_settle`] disagree about, so it rides in as a parameter
    /// rather than as a second entry point that could drift from this one.
    pub(in crate::app::reducer) enum UnloadedBuffer {
        /// Drop the rows along with the status.
        Discard,
        /// Keep the rows; settle only the status.
        Keep,
    }

    /// Emits the recent-logs fetch and keeps the buffer's status honest: it
    /// stays `Loading` only when a load was actually dispatched. This is the
    /// only way to reach the fetch at all -- the effect builder is private to
    /// this `dispatch` module and is not re-exported, so no call site anywhere,
    /// in this file or any other, can skip the settle.
    ///
    /// Callers mark the buffer `Loading` before asking for the fetch -- the
    /// window-moving actions in the parent file (severity floor, time bucket)
    /// after clearing the buffer, the navigation reducer when a Back/Forward
    /// restore lands on the Logging location, the workspace switch, and the
    /// log-jump commands. But the fetch yields nothing without a resolved
    /// context and the tail poll skips for the same reason, so without a context
    /// nothing would ever clear that spinner.
    ///
    /// `unloaded` says what to do with the rows when that happens.
    /// [`UnloadedBuffer::Discard`] drops them with the status, which matters for
    /// the callers that can arrive with rows already loaded -- a restore, a
    /// workspace switch -- because they settle to `Initial`, which renders as
    /// "Not loaded", a header that would otherwise contradict the rows below it.
    /// (The window movers clear before they get here, so for them the choice is
    /// moot.) [`UnloadedBuffer::Keep`] is for `Refresh`, which asks for the
    /// current view again rather than for a different one: those rows were
    /// really fetched, an operator may be mid-read, and losing them buys
    /// nothing.
    pub(in crate::app::reducer) fn load_recent_or_settle(
        state: &mut RootState,
        unloaded: UnloadedBuffer,
    ) -> Vec<Effect> {
        let effects = load_recent_effect(state);
        if effects.is_empty() {
            match unloaded {
                UnloadedBuffer::Discard => state.logging.settle_unloaded(),
                UnloadedBuffer::Keep => state.logging.settle_keeping_buffer(),
            }
        }
        effects
    }

    fn load_recent_effect(state: &RootState) -> Vec<Effect> {
        match state.current_context() {
            Some(context) => vec![Effect::LoadRecentLogs {
                context,
                query: state.logging.query.clone(),
                severity: state.logging.severity_floor,
                range: state.logging.time_range,
            }],
            None => Vec::new(),
        }
    }
}

/// Reported when `r` fires with no resolved gcloud context: without one there
/// is no project to scope logs to, so the recent-logs load emits nothing at
/// all. The sibling of `NO_CONTEXT_LOGS_STATUS` in [`super::resources`], which
/// says the same thing for `L` and the palette log jumps.
const NO_CONTEXT_REFRESH_STATUS: &str = "No active gcloud context to refresh logs from";

/// Asks for the current window again, and says so when it cannot.
///
/// The refusal has to be spoken here because of what `Refresh` deliberately
/// does *not* do: it keeps the buffer (see [`UnloadedBuffer::Keep`]), so a
/// refused refresh leaves the screen byte-for-byte as it was and is
/// indistinguishable from a refresh that came back with the same page. The
/// window-moving callers need no such message -- they discard, and the viewer
/// visibly settles to "Not loaded". Discarding here to make the emptiness
/// legible was considered and rejected: those rows were really
/// fetched and an operator may be mid-read of them.
///
/// An empty effect list is the condition, rather than a second
/// `current_context()` test, so the message can only appear when the fetch
/// really was not dispatched.
fn refresh(state: &mut RootState) -> Vec<Effect> {
    let effects = load_recent_or_settle(state, UnloadedBuffer::Keep);
    if effects.is_empty() {
        state
            .system
            .set_error_status_message(NO_CONTEXT_REFRESH_STATUS);
    }
    effects
}

/// Hands the rendered, sanitized visible lines to the runtime writer. An empty
/// buffer (or a filter that hides everything) emits nothing rather than an
/// empty dump file, but still sets a status so the key press is not silent.
fn save_visible_entries(state: &mut RootState) -> Vec<Effect> {
    let lines = state.logging.visible_export_lines();
    if lines.is_empty() {
        state
            .system
            .set_status_message("No log entries to save (nothing is visible)");
        return Vec::new();
    }
    vec![Effect::SaveLogEntries { lines }]
}

fn copy_selected_entry(state: &RootState) -> Vec<Effect> {
    let Some(index) = state.logging.selected else {
        return Vec::new();
    };
    let visible = state.logging.visible_entries();
    let Some(entry) = visible.get(index) else {
        return Vec::new();
    };
    let payload = match &entry.payload {
        LogPayload::Text(text) | LogPayload::Json(text) => sanitize_log_text(text),
        LogPayload::Empty => sanitize_log_text(&entry.message),
    };
    let value = format!(
        "{} {} {}\n{}",
        sanitize_log_text(&entry.timestamp),
        entry.severity.label(),
        sanitize_log_text(&entry.resource_type),
        payload
    );
    vec![Effect::CopyToClipboard {
        value,
        description: "log entry".to_string(),
    }]
}

fn copy_selected_insert_id(state: &RootState) -> Vec<Effect> {
    let Some(index) = state.logging.selected else {
        return Vec::new();
    };
    let visible = state.logging.visible_entries();
    let Some(entry) = visible.get(index) else {
        return Vec::new();
    };
    vec![Effect::CopyToClipboard {
        value: sanitize_log_text(&entry.insert_id),
        description: "insert ID".to_string(),
    }]
}
