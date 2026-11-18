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

    /// Starts an operation and clears any previous detailed status message.
    pub fn begin_operation(&mut self, phase: OperationPhase, label: impl Into<String>) {
        self.clear_status_message();
        self.set_operation(phase, label);
    }

    /// Starts a follow-up operation while preserving an associated status message.
    pub fn begin_follow_up_operation(&mut self, phase: OperationPhase, label: impl Into<String>) {
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
