//! The plugin output viewer: which retained run is on screen, and where in it.
//!
//! A run's output belongs to its operations-log record
//! ([`crate::app::state::operations::OperationRecord`]) — the bounded thing
//! that already outlives the row, the service and the workspace a run was
//! launched from. The viewer takes a copy of the one run it is showing, so
//! paging is a re-read of the log rather than a second store to keep in step
//! with it, and closing the viewer drops the copy.

use nuvem_domain::gcp::plugin_output::{OutputStream, PluginOutput};

use super::operations::{OperationOutcome, OperationsLog};

/// One run the viewer can show: its record's identity plus its output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RetainedRun<'a> {
    /// The operations-log record this run settled into. A run's name, not its
    /// place: the log grows at the front, so an index means a different run
    /// the moment anything else starts.
    pub id: u64,
    pub label: &'a str,
    pub outcome: OperationOutcome,
    pub output: &'a PluginOutput,
}

/// Every retained run, most recent first — the order the log itself holds.
pub fn retained_runs(operations: &OperationsLog) -> Vec<RetainedRun<'_>> {
    operations
        .records()
        .iter()
        .filter_map(|record| {
            record.output.as_ref().map(|output| RetainedRun {
                id: record.id,
                label: record.target.as_str(),
                outcome: record.outcome,
                output,
            })
        })
        .collect()
}

/// Lines one page-scroll moves, and the tail the last page shows.
///
/// A fixed step, not the terminal's height: neither the reducer nor this type
/// has a viewport, and a step that changed with the window would make the
/// same key mean different things on two terminals.
pub const PLUGIN_OUTPUT_PAGE_STEP: usize = 20;

/// What marks a stderr line wherever one is shown or copied.
pub const STDERR_MARKER: &str = "stderr │ ";

/// Where the operator is: which run, and how far down it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginOutputViewer {
    /// The record whose output is on screen.
    ///
    /// Not its index. A plugin run settles asynchronously and the viewer is
    /// opened to read a run that already finished, so another one arriving
    /// underneath it is ordinary — and it enters the log at the front, moving
    /// every index down by one. Holding an index meant the viewer relabelled
    /// itself onto a run it was not showing.
    id: u64,
    scroll: usize,
    label: String,
    outcome: OperationOutcome,
    /// The run's output itself, not its parts. Holding `lines` and
    /// `dropped_bytes` separately meant re-exposing accessors the domain type
    /// already owns and already has pinned.
    output: PluginOutput,
}

impl PluginOutputViewer {
    /// Opens on the most recent retained run, or on nothing when no run has
    /// output to show.
    pub fn open(operations: &OperationsLog) -> Option<Self> {
        Self::at(operations, 0)
    }

    fn at(operations: &OperationsLog, position: usize) -> Option<Self> {
        let runs = retained_runs(operations);
        let run = runs.get(position)?;
        Some(Self {
            id: run.id,
            scroll: 0,
            label: run.label.to_string(),
            outcome: run.outcome,
            output: run.output.clone(),
        })
    }

    pub fn label(&self) -> &str {
        &self.label
    }

    pub fn outcome(&self) -> OperationOutcome {
        self.outcome
    }

    /// The run on screen. Its lines, its truncation and how that truncation
    /// reads are the domain type's own.
    pub fn output(&self) -> &PluginOutput {
        &self.output
    }

    /// Where the run on screen sits among those the log still holds, or
    /// `None` once the log has evicted it.
    ///
    /// The viewer keeps showing an evicted run — it holds its own copy of the
    /// output, and dropping it mid-read would be worse than outliving its
    /// record — but it stops claiming a place in a list it has left.
    pub fn position(&self, operations: &OperationsLog) -> Option<usize> {
        retained_runs(operations)
            .iter()
            .position(|run| run.id == self.id)
    }

    pub fn scroll(&self) -> usize {
        self.scroll
    }

    /// Moves to the next older retained run, from its first line.
    ///
    /// Stops at the oldest rather than wrapping: the count is small and
    /// wrapping would make "have I seen them all" unanswerable without
    /// counting.
    pub fn older(&mut self, operations: &OperationsLog) {
        let Some(position) = self.position(operations) else {
            return;
        };
        if let Some(next) = Self::at(operations, position + 1) {
            *self = next;
        }
    }

    /// Moves to the next newer retained run, from its first line.
    ///
    /// A run that settled while the viewer was open is newer than the one on
    /// screen, so this reaches it: the position is resolved against the log
    /// as it is now, not as it was when the viewer opened.
    pub fn newer(&mut self, operations: &OperationsLog) {
        let Some(position) = self.position(operations).and_then(|at| at.checked_sub(1)) else {
            return;
        };
        if let Some(next) = Self::at(operations, position) {
            *self = next;
        }
    }

    /// Scrolls down `lines`, stopping where the last page of the run begins.
    ///
    /// Clamping to the last *line* let a three-line run scroll until one line
    /// sat above a screen of blank rows. The reducer has no viewport — that is
    /// why the page step is a constant — so the step is also the only honest
    /// stand-in for one here: a run shorter than a page does not scroll at
    /// all, and a longer one stops with its tail on screen.
    pub fn scroll_down(&mut self, lines: usize) {
        let last_page = self
            .output
            .lines()
            .len()
            .saturating_sub(PLUGIN_OUTPUT_PAGE_STEP);
        self.scroll = (self.scroll + lines).min(last_page);
    }

    pub fn scroll_up(&mut self, lines: usize) {
        self.scroll = self.scroll.saturating_sub(lines);
    }

    /// The whole run as text, for the clipboard: the escape hatch for output
    /// past the retention bound is copying what was kept.
    ///
    /// Marked the way the viewer draws it. The mark is not decoration — which
    /// stream a line came from is what tells a warning from an answer — and a
    /// failed run, the one `y` exists for, is where the two interleave.
    pub fn to_clipboard_text(&self) -> String {
        self.output
            .lines()
            .iter()
            .map(|line| match line.stream {
                OutputStream::Stdout => line.text.clone(),
                OutputStream::Stderr => format!("{STDERR_MARKER}{}", line.text),
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}
