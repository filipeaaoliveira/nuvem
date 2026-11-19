use chrono::{DateTime, Local};

use crate::app::state::diagnostics::OverviewDiagnostics;
use crate::app::state::operations::OperationsLog;
use crate::app::state::plugin_output::PluginOutputViewer;
use nuvem_domain::gcp::diagnostics::{GcloudDiagnostic, sanitize_detail};

/// Semantic operation state shown in the shared application frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OperationPhase {
    Idle,
    Loading,
    Refreshing,
    Healthy,
    Degraded,
    Failed,
}

impl OperationPhase {
    /// Returns the text marker paired with this phase in the status area.
    pub fn symbol(self) -> &'static str {
        match self {
            Self::Idle | Self::Healthy => "OK",
            Self::Loading => "LOAD",
            Self::Refreshing => "SYNC",
            Self::Degraded => "STALE",
            Self::Failed => "ERR",
        }
    }
}

/// Current operation phase and its human-readable label.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OperationState {
    pub phase: OperationPhase,
    pub label: String,
}

impl Default for OperationState {
    fn default() -> Self {
        Self {
            phase: OperationPhase::Idle,
            label: "Ready".to_string(),
        }
    }
}

/// Application-level status and overlay state used by the renderer.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SystemState {
    pub show_help: bool,
    pub help_scroll: u16,
    pub status_message: Option<String>,
    pub status_operation_label: Option<String>,
    pub status_phase: Option<OperationPhase>,
    pub status_generation: u64,
    pub operation: OperationState,
    pub last_refresh_at: Option<DateTime<Local>>,
    pub operations: OperationsLog,
    pub diagnostics: OverviewDiagnostics,
    /// The plugin output viewer, open or absent. Private so the
    /// only ways in are the two methods below: opening it depends on the
    /// operations log, and an open viewer with no run behind it is not a
    /// state this type should be able to represent.
    plugin_output: Option<PluginOutputViewer>,
    last_diagnostic: Option<GcloudDiagnostic>,
    config_problems: Vec<String>,
    /// What the status line showed when a background operation began, while
    /// that operation is the only thing to have changed it. See
    /// [`Self::begin_background_operation`].
    background: Option<BackgroundStatus>,
}

/// The fields the status line is drawn from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusLineState {
    pub operation: OperationState,
    pub status_message: Option<String>,
    pub status_operation_label: Option<String>,
    pub status_phase: Option<OperationPhase>,
}

/// A background operation in flight, and the status line it is not to touch.
#[derive(Debug, Clone, PartialEq, Eq)]
struct BackgroundStatus {
    /// The line as it stood before the operation began.
    shown: StatusLineState,
    /// The operation as it was begun, and the status generation after it: the
    /// line stays `shown` only while both still hold, so anything else that
    /// writes the line — a key, a result, another operation — is seen.
    operation: OperationState,
    status_generation: u64,
}

impl SystemState {
    /// Opens the viewer on the most recent retained run, if there is one.
    ///
    /// No success flag, because the one caller cannot observe a failure: the
    /// palette resolves its descriptor from `visible_commands` at submit
    /// time, and that list only carries `ShowPluginOutput` while a record
    /// with output is in the log. An empty open is unrepresentable rather
    /// than handled — a branch nothing can take is a branch no test can pin.
    pub fn open_plugin_output(&mut self) {
        self.plugin_output = PluginOutputViewer::open(&self.operations);
    }

    pub fn close_plugin_output(&mut self) {
        self.plugin_output = None;
    }

    pub fn plugin_output(&self) -> Option<&PluginOutputViewer> {
        self.plugin_output.as_ref()
    }

    /// The viewer and the log together, so a move between runs can read the
    /// log the viewer indexes into without a second borrow of `self`.
    pub fn plugin_output_and_log(&mut self) -> Option<(&mut PluginOutputViewer, &OperationsLog)> {
        let viewer = self.plugin_output.as_mut()?;
        Some((viewer, &self.operations))
    }

    /// Records the most recently rendered sanitized diagnostic, used by the
    /// "copy diagnostic" key so the clipboard payload matches what is shown.
    pub fn set_last_diagnostic(&mut self, diagnostic: GcloudDiagnostic) {
        self.last_diagnostic = Some(diagnostic);
    }

    /// Returns the most recently rendered sanitized diagnostic, when any
    /// failure surface has recorded one.
    pub fn last_diagnostic(&self) -> Option<&GcloudDiagnostic> {
        self.last_diagnostic.as_ref()
    }

    /// Replaces the recorded configuration problems wholesale, so a reload of
    /// a fixed configuration (an empty vec) clears every previously shown
    /// problem instead of leaving stale entries in the help overlay.
    pub fn set_config_problems(&mut self, problems: Vec<String>) {
        self.config_problems = problems;
    }

    /// Returns the configuration problems recorded by the most recent load or
    /// reload, in the order they were reported.
    pub fn config_problems(&self) -> &[String] {
        &self.config_problems
    }

    pub fn set_status_message(&mut self, message: impl Into<String>) {
        self.status_message = Some(message.into());
        self.status_operation_label = None;
        self.status_phase = Some(OperationPhase::Healthy);
        self.status_generation = self.status_generation.wrapping_add(1);
    }

    pub fn set_error_status_message(&mut self, message: impl Into<String>) {
        self.status_message = Some(message.into());
        self.status_operation_label = None;
        self.status_phase = Some(OperationPhase::Failed);
        self.status_generation = self.status_generation.wrapping_add(1);
    }

    pub fn clear_status_message(&mut self) {
        self.status_message = None;
        self.status_operation_label = None;
        self.status_phase = None;
        self.status_generation = self.status_generation.wrapping_add(1);
    }

    pub fn set_operation_status_message(
        &mut self,
        label: impl Into<String>,
        message: impl Into<String>,
    ) {
        // The status line is a display surface, so the chokepoint runs here
        // rather than being trusted to every caller: a message built from a
        // `Debug` rendering of an error can carry a child's raw stderr.
        self.status_message = Some(sanitize_detail(&message.into()));
        self.status_operation_label = Some(label.into());
        self.status_phase = None;
        self.status_generation = self.status_generation.wrapping_add(1);
    }

    pub fn associate_status_message_with_operation(&mut self, label: impl Into<String>) {
        if self.status_message.is_some() {
            self.status_operation_label = Some(label.into());
            self.status_phase = None;
            self.status_generation = self.status_generation.wrapping_add(1);
        }
    }

    /// The fields the status line shows: the live ones, except while a
    /// background operation is the only thing to have changed them since it
    /// began, when they are the ones from before it began.
    pub fn status_line(&self) -> StatusLineState {
        match self.holding_background() {
            Some(background) => background.shown.clone(),
            None => StatusLineState {
                operation: self.operation.clone(),
                status_message: self.status_message.clone(),
                status_operation_label: self.status_operation_label.clone(),
                status_phase: self.status_phase,
            },
        }
    }

    fn holding_background(&self) -> Option<&BackgroundStatus> {
        self.background.as_ref().filter(|background| {
            background.operation == self.operation
                && background.status_generation == self.status_generation
        })
    }

    /// Starts an operation nobody asked for — a poll of the list on screen —
    /// exactly as [`Self::begin_operation`] (or, with `preserve_status`,
    /// [`Self::begin_follow_up_operation`]) would, but leaves the status line
    /// showing what it showed before.
    ///
    /// A poll that changes nothing is not news, and flashing `[SYNC]` over a
    /// list the operator is reading, every few seconds, says otherwise. The
    /// line moves again as soon as anything else writes it: the poll's own
    /// result (a failure is news), a key, or another operation starting.
    pub fn begin_background_operation(
        &mut self,
        phase: OperationPhase,
        label: impl Into<String>,
        preserve_status: bool,
    ) {
        let shown = self.status_line();
        if preserve_status {
            self.begin_follow_up_operation(phase, label);
        } else {
            self.begin_operation(phase, label);
        }
        self.background = Some(BackgroundStatus {
            shown,
            operation: self.operation.clone(),
            status_generation: self.status_generation,
        });
    }

    /// Starts an operation and clears any previous detailed status message.
    pub fn begin_operation(&mut self, phase: OperationPhase, label: impl Into<String>) {
        self.clear_status_message();
        self.set_operation(phase, label);
    }

    /// Starts a follow-up operation while preserving an associated status message.
    pub fn begin_follow_up_operation(&mut self, phase: OperationPhase, label: impl Into<String>) {
        // Forgotten outright rather than left to the check in
        // `holding_background`: with no message to re-label this writes
        // nothing that check can see when the operation it begins has the
        // phase and label the background one had.
        self.background = None;
        let label = label.into();
        self.set_operation(phase, label.clone());
        if self.status_message.is_some() && self.status_operation_label.is_some() {
            self.status_operation_label = Some(label);
            self.status_phase = None;
            self.status_generation = self.status_generation.wrapping_add(1);
        }
    }

    fn set_operation(&mut self, phase: OperationPhase, label: impl Into<String>) {
        self.operation = OperationState {
            phase,
            label: label.into(),
        };
    }

    /// Records the completed phase and its human-readable label.
    pub fn complete_operation(&mut self, phase: OperationPhase, label: impl Into<String>) {
        self.operation = OperationState {
            phase,
            label: label.into(),
        };
    }

    /// Records a completed refresh and the timestamp supplied by the runtime.
    pub fn complete_refresh_operation(
        &mut self,
        phase: OperationPhase,
        label: impl Into<String>,
        timestamp: DateTime<Local>,
    ) {
        self.complete_operation(phase, label);
        self.last_refresh_at = Some(timestamp);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shown(state: &SystemState) -> (OperationPhase, String, Option<String>) {
        let line = state.status_line();
        (
            line.status_phase.unwrap_or(line.operation.phase),
            line.operation.label,
            line.status_message,
        )
    }

    /// A background operation leaves the line as it stood, and the line moves
    /// again with the first thing that writes it: here, the operation's own
    /// completion.
    #[test]
    fn a_background_operation_leaves_the_line_until_it_completes() {
        let mut state = SystemState::default();
        state.begin_operation(OperationPhase::Refreshing, "GCE instances");
        state.complete_operation(OperationPhase::Healthy, "GCE instances");
        let settled = shown(&state);

        state.begin_background_operation(OperationPhase::Refreshing, "GCE instances", false);
        assert_eq!(shown(&state), settled);

        state.complete_operation(OperationPhase::Degraded, "GCE instances");
        assert_eq!(shown(&state).0, OperationPhase::Degraded);
    }

    /// A message written while it runs is shown, and so is an operation the
    /// operator starts.
    #[test]
    fn anything_else_that_writes_the_line_is_shown() {
        let mut state = SystemState::default();
        state.begin_background_operation(OperationPhase::Refreshing, "GCE instances", false);
        state.set_status_message("Copied");
        assert_eq!(shown(&state).2.as_deref(), Some("Copied"));

        state.begin_background_operation(OperationPhase::Refreshing, "GCE instances", false);
        state.begin_operation(OperationPhase::Refreshing, "GCE instances");
        assert_eq!(shown(&state).0, OperationPhase::Refreshing);
    }

    /// A follow-up with the background operation's own phase and label, after
    /// it completed, is shown too, though it writes nothing else.
    #[test]
    fn a_follow_up_after_a_background_operation_is_shown() {
        let mut state = SystemState::default();
        state.complete_operation(OperationPhase::Healthy, "GCE instances");
        state.begin_background_operation(OperationPhase::Refreshing, "GCE instances", false);
        state.complete_operation(OperationPhase::Healthy, "GCE instances");
        state.begin_follow_up_operation(OperationPhase::Refreshing, "GCE instances");
        assert_eq!(shown(&state).0, OperationPhase::Refreshing);
    }

    #[test]
    fn config_problems_start_empty() {
        assert!(SystemState::default().config_problems().is_empty());
    }

    #[test]
    fn set_config_problems_replaces_the_whole_list() {
        let mut state = SystemState::default();
        state.set_config_problems(vec!["first".to_string(), "second".to_string()]);
        assert_eq!(state.config_problems(), ["first", "second"]);

        state.set_config_problems(vec!["only".to_string()]);
        assert_eq!(state.config_problems(), ["only"]);
    }

    #[test]
    fn setting_an_empty_list_clears_previously_recorded_problems() {
        let mut state = SystemState::default();
        state.set_config_problems(vec!["stale".to_string()]);
        state.set_config_problems(Vec::new());
        assert!(
            state.config_problems().is_empty(),
            "a fixed config must not leave stale problems on screen"
        );
    }
}
