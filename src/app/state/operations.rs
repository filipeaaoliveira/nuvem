use std::collections::VecDeque;

use nuvem_domain::gcp::plugin_output::PluginOutput;

const OPERATIONS_CAPACITY: usize = 10;

/// Lifecycle state of a tracked in-app operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OperationOutcome {
    Pending,
    Succeeded,
    Failed,
    /// The command was never issued, so nothing changed outside the app and the
    /// operator has nothing to re-check.
    Cancelled,
    /// The command was issued but its outcome was never observed. Distinct from
    /// `Failed` because claiming failure for work GCP may well have completed is
    /// a lie the operator would act on.
    Unknown,
}

impl OperationOutcome {
    pub fn label(self) -> &'static str {
        match self {
            Self::Pending => "Pending",
            Self::Succeeded => "Succeeded",
            Self::Failed => "Failed",
            Self::Cancelled => "Cancelled",
            Self::Unknown => "Unknown",
        }
    }
}

/// One tracked operation (an in-app GCE mutation), most recent first in the log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OperationRecord {
    pub id: u64,
    pub kind: String,
    pub target: String,
    pub outcome: OperationOutcome,
    pub detail: Option<String>,
    /// What the command printed, kept as lines.
    ///
    /// Only a plugin run sets this — every built-in operation's output is
    /// parsed into state rather than shown — so the field is `None` for most
    /// records. It lives here rather than in a store of its own because the
    /// log is already the bounded thing that outlives the row, the service
    /// and the workspace a run was launched from: eviction comes free with
    /// the ring, and a side store would need an eviction rule that could
    /// drift out of step with it.
    pub output: Option<PluginOutput>,
}

/// Bounded, most-recent-first log of in-app operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OperationsLog {
    records: VecDeque<OperationRecord>,
    next_id: u64,
    capacity: usize,
}

impl Default for OperationsLog {
    fn default() -> Self {
        Self {
            records: VecDeque::new(),
            next_id: 0,
            capacity: OPERATIONS_CAPACITY,
        }
    }
}

impl OperationsLog {
    /// Records a new pending operation at the front and returns its id.
    pub fn begin(&mut self, kind: impl Into<String>, target: impl Into<String>) -> u64 {
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1);
        self.records.push_front(OperationRecord {
            id,
            kind: kind.into(),
            target: target.into(),
            outcome: OperationOutcome::Pending,
            detail: None,
            output: None,
        });
        while self.records.len() > self.capacity {
            self.records.pop_back();
        }
        id
    }

    /// Resolves one specific record by id, and only while it is still pending.
    ///
    /// A caller holding ids it created can settle exactly its own entries; a
    /// `(kind, target)` lookup could instead settle an unrelated action's
    /// pending entry that happens to share both fields. The pending guard keeps
    /// an already-observed outcome authoritative — a late sweep must never
    /// overwrite what we actually saw. For the same reason, this never
    /// discards what was already observed: a `None` detail means "nothing new
    /// to report", not "clear it", so an existing detail is left in place
    /// rather than assigned over.
    pub fn resolve_by_id(
        &mut self,
        id: u64,
        outcome: OperationOutcome,
        detail: Option<String>,
    ) -> bool {
        if let Some(record) = self
            .records
            .iter_mut()
            .find(|record| record.id == id && record.outcome == OperationOutcome::Pending)
        {
            record.outcome = outcome;
            if let Some(detail) = detail {
                record.detail = Some(detail);
            }
            true
        } else {
            false
        }
    }

    /// Attaches a plugin run's output to its own record.
    ///
    /// Separate from [`OperationsLog::resolve_by_id`] rather than a fourth
    /// parameter on it: five call sites settle operations and exactly one
    /// kind of them has output, so widening the shared signature would make
    /// every other caller pass `None` to say something it has no opinion
    /// about. Unlike `resolve_by_id` this does not require the record to be
    /// pending — settling is what produces the output, so it always arrives
    /// second.
    pub fn attach_output(&mut self, id: u64, output: PluginOutput) -> bool {
        if let Some(record) = self.records.iter_mut().find(|record| record.id == id) {
            record.output = Some(output);
            true
        } else {
            false
        }
    }

    pub fn records(&self) -> &VecDeque<OperationRecord> {
        &self.records
    }

    /// How many records the log holds before it starts evicting the oldest.
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    pub fn pending_count(&self) -> usize {
        self.records
            .iter()
            .filter(|record| record.outcome == OperationOutcome::Pending)
            .count()
    }

    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }
}
