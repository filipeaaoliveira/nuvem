//! The tunnels the app believes are running: one [`TunnelEntry`] per managed
//! child (IAP tunnel, `ssh -L` forward, Cloud SQL Auth Proxy), the selection
//! the workspace list moves through, and the id allocator the runtime keys its
//! registry by.
//!
//! Two properties of this list are decisions, not accidents.
//!
//! **Tunnels are not cleared on a context switch.** Every other resource list
//! is scoped to the active gcloud context and dropped when it changes, because
//! the rows are a cached view of that context. A tunnel is not a view: it is a
//! child process that keeps running when the user activates a different
//! configuration. Clearing the list would hide a live process and leave nothing
//! that can stop it, so entries survive and each one keeps the
//! [`GcloudContext`] it was started under (that is what the `context` field is
//! for) rather than borrowing whichever context happens to be active now.
//!
//! **A terminal entry has released its local port.** `Failed` and `Stopped`
//! mean no child is bound to `local_port` any more, so
//! [`TunnelsState::claimed_ports`] excludes them and a later tunnel may reuse
//! the port. The entry itself stays in the list until the user dismisses it —
//! the failure message is the whole reason it is worth showing.
//!
//! No method here reads the clock: `started_at` is supplied by the caller at
//! insert time, so every transition is deterministic and testable.

use std::time::Instant;

use nuvem_domain::gcp::configuration::GcloudContext;
use nuvem_domain::gcp::tunnel::TunnelKind;

use crate::services::runtime::tunnel::TunnelId;

/// Where one tunnel is in its life. `Failed` and `Stopped` are terminal: no
/// child is left to signal, and the port is free again.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TunnelStatus {
    /// Spawned, not yet known to be serving traffic.
    Starting,
    /// Serving traffic on `local_port`.
    Active,
    /// A stop was requested and the child has not been reaped yet.
    Stopping,
    /// The child died or never came up. `message` is the sanitized reason.
    Failed { message: String },
    /// The child was stopped as asked.
    Stopped,
}

impl TunnelStatus {
    /// Whether no child is running any more. Terminal entries are dismissible,
    /// are not killable, and hold no local port.
    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Failed { .. } | Self::Stopped)
    }
}

/// One managed tunnel, as the app models it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TunnelEntry {
    pub id: TunnelId,
    pub kind: TunnelKind,
    /// What the tunnel points at, for display (instance or Cloud SQL instance).
    pub target_label: String,
    /// The loopback port clients connect to. Replaced by the port the child
    /// actually reports, when it reports one.
    pub local_port: u16,
    /// The far side of the tunnel, for display.
    pub remote_label: String,
    pub status: TunnelStatus,
    /// When the tunnel was started, supplied by the caller (never read from the
    /// clock inside this module).
    pub started_at: Instant,
    /// The context the tunnel was started under — deliberately kept, since the
    /// active context may have moved on while the child kept running.
    pub context: GcloudContext,
    /// The latest human-readable note about this entry: the failure reason
    /// while `Failed`, cleared once the tunnel comes up.
    pub message: Option<String>,
}

/// Every tunnel this app run has started, newest last, plus the workspace
/// selection and the id allocator.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TunnelsState {
    entries: Vec<TunnelEntry>,
    selected: usize,
    next_id: u64,
}

impl TunnelsState {
    /// Allocates the next tunnel id. Strictly increasing and never reused, so
    /// a late result addressed to a removed tunnel can never land on a
    /// different one.
    pub fn mint_id(&mut self) -> TunnelId {
        self.next_id += 1;
        TunnelId(self.next_id)
    }

    /// Appends a newly launched tunnel. The status is forced to `Starting`:
    /// nothing else is true of a tunnel that has only just been asked for.
    pub fn insert_starting(&mut self, mut entry: TunnelEntry) {
        entry.status = TunnelStatus::Starting;
        self.entries.push(entry);
    }

    /// Records the port the runtime allocated for a tunnel that has not been
    /// launched yet, so [`TunnelsState::claimed_ports`] promises it from that
    /// moment on. The status is untouched: an allocated port says where the
    /// child will listen, not that anything is listening.
    ///
    /// A terminal row is left alone: see `TunnelsState::live_entry_mut`.
    pub fn record_allocated_port(&mut self, id: TunnelId, local_port: u16) {
        if let Some(entry) = self.live_entry_mut(id) {
            entry.local_port = local_port;
        }
    }

    /// Marks the tunnel up, adopting the port the child reported when it
    /// reported one — the child's actual port wins over the requested one.
    ///
    /// A terminal row is left alone: see `TunnelsState::live_entry_mut`.
    pub fn mark_active(&mut self, id: TunnelId, local_port: Option<u16>) {
        let Some(entry) = self.live_entry_mut(id) else {
            return;
        };
        entry.status = TunnelStatus::Active;
        if let Some(port) = local_port {
            entry.local_port = port;
        }
        entry.message = None;
    }

    /// Records that a stop is in flight. Not terminal: the port stays claimed
    /// until the child is actually gone — which is exactly why a terminal row
    /// must not enter it. See `TunnelsState::live_entry_mut`.
    pub fn mark_stopping(&mut self, id: TunnelId) {
        if let Some(entry) = self.live_entry_mut(id) {
            entry.status = TunnelStatus::Stopping;
        }
    }

    /// Records a note about a tunnel that is still doing its job — the row's
    /// MESSAGE moves, its status does not. `message` must already be sanitized
    /// and bounded: it originates in child output.
    ///
    /// A terminal row is left alone: see `TunnelsState::live_entry_mut`. Its
    /// message is the reason it is still on screen.
    pub fn set_notice(&mut self, id: TunnelId, message: impl Into<String>) {
        if let Some(entry) = self.live_entry_mut(id) {
            entry.message = Some(message.into());
        }
    }

    /// Records a failure. `message` must already be sanitized and bounded — it
    /// originates in child output.
    ///
    /// A terminal row is left alone: see `TunnelsState::live_entry_mut`.
    pub fn mark_failed(&mut self, id: TunnelId, message: impl Into<String>) {
        let message = message.into();
        if let Some(entry) = self.live_entry_mut(id) {
            entry.status = TunnelStatus::Failed {
                message: message.clone(),
            };
            entry.message = Some(message);
        }
    }

    /// Records a clean stop. A terminal row is left alone: see
    /// `TunnelsState::live_entry_mut`. In particular a stop acknowledgement
    /// that lands after the child's own stderr already failed the row must not
    /// replace that reason with a bare `Stopped`.
    pub fn mark_stopped(&mut self, id: TunnelId) {
        if let Some(entry) = self.live_entry_mut(id) {
            entry.status = TunnelStatus::Stopped;
        }
    }

    /// Drops the entry, keeping the selection in range: it stays on the same
    /// index (which now shows the row that followed) and only moves when that
    /// index no longer exists. An unknown id changes nothing.
    pub fn remove(&mut self, id: TunnelId) {
        let Some(index) = self.entries.iter().position(|entry| entry.id == id) else {
            return;
        };
        self.entries.remove(index);
        self.selected = self.selected.min(self.entries.len().saturating_sub(1));
    }

    pub fn entries(&self) -> &[TunnelEntry] {
        &self.entries
    }

    pub fn selected_index(&self) -> usize {
        self.selected
    }

    pub fn selected(&self) -> Option<&TunnelEntry> {
        self.entries.get(self.selected)
    }

    pub fn select_next(&mut self) {
        if self.selected + 1 < self.entries.len() {
            self.selected += 1;
        }
    }

    pub fn select_previous(&mut self) {
        self.selected = self.selected.saturating_sub(1);
    }

    /// The local ports currently promised to live tunnels, for the port
    /// allocator. Terminal entries are excluded: their child is gone, so the
    /// port is free even though the row is still on screen.
    pub fn claimed_ports(&self) -> Vec<u16> {
        self.entries
            .iter()
            .filter(|entry| !entry.status.is_terminal())
            .map(|entry| entry.local_port)
            .collect()
    }

    /// How many tunnels are known to be serving traffic. Starting and stopping
    /// tunnels are not counted — neither can carry a connection.
    pub fn active_count(&self) -> usize {
        self.entries
            .iter()
            .filter(|entry| entry.status == TunnelStatus::Active)
            .count()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    fn entry_mut(&mut self, id: TunnelId) -> Option<&mut TunnelEntry> {
        self.entries.iter_mut().find(|entry| entry.id == id)
    }

    /// The entry for `id`, but only while it is still non-terminal.
    ///
    /// **Terminal is terminal.** The runtime supervises a tunnel from a task of
    /// its own, so a result it sent just before the child died can land after
    /// the row has already reached `Failed`/`Stopped` — a `TunnelReady` racing a
    /// `TunnelStopped` from the kill path is the ordinary case. Applying it
    /// would flip a reaped tunnel back to `Active`: the row would lie, its port
    /// would stay promised to the allocator forever, and the user could no
    /// longer dismiss it.
    ///
    /// Every status transition on this type goes through this accessor —
    /// `mark_active`, `mark_stopping`, `mark_failed`, `mark_stopped`, plus
    /// `record_allocated_port` and `set_notice` — rather than repeating the
    /// check, so the guard cannot be forgotten at a call site. The only
    /// unguarded mutator left is [`TunnelsState::insert_starting`], which
    /// appends a new row and can therefore resurrect nothing.
    /// `apply_exited` makes the same decision from the reducer, where the
    /// *reason* also depends on the status.
    fn live_entry_mut(&mut self, id: TunnelId) -> Option<&mut TunnelEntry> {
        self.entry_mut(id)
            .filter(|entry| !entry.status.is_terminal())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nuvem_domain::gcp::configuration::{GcloudConfigurationName, GcloudContext};
    use std::time::Instant;

    fn context() -> GcloudContext {
        GcloudContext {
            configuration: GcloudConfigurationName::new("dev"),
            account: None,
            project: Some("dev-project".to_string()),
            region: None,
            zone: None,
        }
    }

    /// Builds an entry the way the reducer will: an id minted from the state,
    /// a caller-supplied `Instant`, and no message yet.
    fn entry(state: &mut TunnelsState, port: u16, at: Instant) -> TunnelEntry {
        TunnelEntry {
            id: state.mint_id(),
            kind: TunnelKind::IapTcp,
            target_label: format!("web-{port}"),
            local_port: port,
            remote_label: "22".to_string(),
            status: TunnelStatus::Starting,
            started_at: at,
            context: context(),
            message: None,
        }
    }

    fn state_with(ports: &[u16]) -> TunnelsState {
        let at = Instant::now();
        let mut state = TunnelsState::default();
        for port in ports {
            let entry = entry(&mut state, *port, at);
            state.insert_starting(entry);
        }
        state
    }

    fn ports(state: &TunnelsState) -> Vec<u16> {
        state.entries().iter().map(|e| e.local_port).collect()
    }

    #[test]
    fn a_fresh_state_is_empty_and_selects_nothing() {
        let state = TunnelsState::default();
        assert!(state.is_empty());
        assert_eq!(state.selected_index(), 0);
        assert!(state.selected().is_none());
        assert_eq!(state.active_count(), 0);
        assert!(state.claimed_ports().is_empty());
    }

    #[test]
    fn minted_ids_are_monotonic_and_never_reused_after_a_removal() {
        let mut state = TunnelsState::default();
        let first = state.mint_id();
        let second = state.mint_id();
        assert!(second > first, "ids must increase");

        let at = Instant::now();
        let entry = entry(&mut state, 5432, at);
        let third = entry.id;
        state.insert_starting(entry);
        state.remove(third);
        assert!(
            state.mint_id() > third,
            "removing an entry must not free its id for reuse"
        );
    }

    #[test]
    fn insert_starting_appends_and_forces_the_starting_status() {
        let at = Instant::now();
        let mut state = TunnelsState::default();
        let mut entry = entry(&mut state, 5432, at);
        entry.status = TunnelStatus::Stopped;
        state.insert_starting(entry);

        assert_eq!(state.entries().len(), 1);
        assert_eq!(state.entries()[0].status, TunnelStatus::Starting);
        assert_eq!(state.selected().map(|e| e.local_port), Some(5432));
    }

    #[test]
    fn mark_active_adopts_the_reported_local_port() {
        let mut state = state_with(&[5432]);
        let id = state.entries()[0].id;
        state.mark_active(id, Some(5433));
        assert_eq!(state.entries()[0].status, TunnelStatus::Active);
        assert_eq!(state.entries()[0].local_port, 5433);
    }

    #[test]
    fn mark_active_without_a_reported_port_keeps_the_requested_one() {
        let mut state = state_with(&[5432]);
        let id = state.entries()[0].id;
        state.mark_active(id, None);
        assert_eq!(state.entries()[0].local_port, 5432);
    }

    #[test]
    fn mark_active_clears_a_stale_message() {
        // Seeded on a live entry rather than by failing it first: a `Failed`
        // entry is terminal and `mark_active` deliberately no longer touches it.
        let at = Instant::now();
        let mut state = TunnelsState::default();
        let mut entry = entry(&mut state, 5432, at);
        entry.message = Some("boom".to_string());
        state.insert_starting(entry);
        let id = state.entries()[0].id;

        state.mark_active(id, None);

        assert_eq!(state.entries()[0].message, None);
    }

    #[test]
    fn a_terminal_entry_is_never_reopened_by_a_late_result() {
        let mut state = state_with(&[5432, 5433]);
        let ids: Vec<TunnelId> = state.entries().iter().map(|e| e.id).collect();
        state.mark_stopped(ids[0]);
        state.mark_failed(ids[1], "boom");

        state.mark_active(ids[0], Some(9000));
        state.mark_failed(ids[0], "late boom");
        state.mark_active(ids[1], Some(9001));

        assert_eq!(state.entries()[0].status, TunnelStatus::Stopped);
        assert_eq!(state.entries()[0].local_port, 5432);
        assert_eq!(
            state.entries()[1].status,
            TunnelStatus::Failed {
                message: "boom".to_string()
            }
        );
        assert!(
            state.claimed_ports().is_empty(),
            "a terminal entry must not re-claim its port"
        );
    }

    #[test]
    fn mark_failed_records_the_message_on_the_status_and_the_entry() {
        let mut state = state_with(&[5432]);
        let id = state.entries()[0].id;
        state.mark_failed(id, "address already in use");
        assert_eq!(
            state.entries()[0].status,
            TunnelStatus::Failed {
                message: "address already in use".to_string()
            }
        );
        assert_eq!(
            state.entries()[0].message.as_deref(),
            Some("address already in use")
        );
    }

    #[test]
    fn mark_stopping_and_mark_stopped_move_through_the_terminal_state() {
        let mut state = state_with(&[5432]);
        let id = state.entries()[0].id;
        state.mark_stopping(id);
        assert_eq!(state.entries()[0].status, TunnelStatus::Stopping);
        state.mark_stopped(id);
        assert_eq!(state.entries()[0].status, TunnelStatus::Stopped);
    }

    #[test]
    fn marking_an_unknown_id_changes_nothing() {
        let mut state = state_with(&[5432]);
        let before = state.clone();
        let unknown = TunnelId(9999);
        state.mark_active(unknown, Some(1));
        state.mark_stopping(unknown);
        state.mark_stopped(unknown);
        state.mark_failed(unknown, "boom");
        state.remove(unknown);
        assert_eq!(state, before);
    }

    #[test]
    fn claimed_ports_covers_live_entries_only() {
        let mut state = state_with(&[5432, 5433, 5434, 5435, 5436]);
        let ids: Vec<TunnelId> = state.entries().iter().map(|e| e.id).collect();
        // 5432 stays Starting.
        state.mark_active(ids[1], None);
        state.mark_stopping(ids[2]);
        state.mark_failed(ids[3], "boom");
        state.mark_stopped(ids[4]);

        assert_eq!(
            state.claimed_ports(),
            vec![5432, 5433, 5434],
            "a terminal tunnel has released its port"
        );
    }

    #[test]
    fn active_count_counts_only_active_entries() {
        let mut state = state_with(&[5432, 5433, 5434]);
        let ids: Vec<TunnelId> = state.entries().iter().map(|e| e.id).collect();
        state.mark_active(ids[0], None);
        state.mark_active(ids[1], None);
        state.mark_stopping(ids[2]);
        assert_eq!(state.active_count(), 2);
    }

    #[test]
    fn selection_moves_within_bounds_and_does_not_wrap() {
        let mut state = state_with(&[1, 2, 3]);
        assert_eq!(state.selected_index(), 0);
        state.select_previous();
        assert_eq!(state.selected_index(), 0, "no wrap off the top");
        state.select_next();
        state.select_next();
        assert_eq!(state.selected_index(), 2);
        state.select_next();
        assert_eq!(state.selected_index(), 2, "no wrap off the bottom");
        state.select_previous();
        assert_eq!(state.selected_index(), 1);
    }

    #[test]
    fn selection_on_an_empty_list_is_a_no_op() {
        let mut state = TunnelsState::default();
        state.select_next();
        state.select_previous();
        assert_eq!(state.selected_index(), 0);
        assert!(state.selected().is_none());
    }

    #[test]
    fn removing_the_last_row_moves_selection_to_the_new_last_row() {
        let mut state = state_with(&[1, 2, 3]);
        state.select_next();
        state.select_next();
        let id = state.entries()[2].id;
        state.remove(id);
        assert_eq!(ports(&state), vec![1, 2]);
        assert_eq!(state.selected_index(), 1);
        assert_eq!(state.selected().map(|e| e.local_port), Some(2));
    }

    #[test]
    fn removing_a_middle_row_keeps_the_index_and_shows_the_following_row() {
        let mut state = state_with(&[1, 2, 3]);
        state.select_next();
        let id = state.entries()[1].id;
        state.remove(id);
        assert_eq!(ports(&state), vec![1, 3]);
        assert_eq!(state.selected_index(), 1);
        assert_eq!(state.selected().map(|e| e.local_port), Some(3));
    }

    #[test]
    fn removing_the_only_row_leaves_an_empty_list_selected_at_zero() {
        let mut state = state_with(&[1]);
        let id = state.entries()[0].id;
        state.remove(id);
        assert!(state.is_empty());
        assert_eq!(state.selected_index(), 0);
        assert!(state.selected().is_none());
    }

    #[test]
    fn is_terminal_is_true_for_failed_and_stopped_only() {
        assert!(!TunnelStatus::Starting.is_terminal());
        assert!(!TunnelStatus::Active.is_terminal());
        assert!(!TunnelStatus::Stopping.is_terminal());
        assert!(TunnelStatus::Stopped.is_terminal());
        assert!(
            TunnelStatus::Failed {
                message: "boom".to_string()
            }
            .is_terminal()
        );
    }
}
