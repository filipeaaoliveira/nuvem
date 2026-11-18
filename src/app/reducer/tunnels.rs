use crate::app::state::root::RootState;
use crate::app::state::tunnels::TunnelStatus;
use crate::app::{Effect, TunnelsAction};
use crate::services::runtime::tunnel::TunnelId;

/// What a tunnel that died before it ever came up is recorded as. A bare exit
/// carries no reason of its own, so this is the reason.
const EXITED_BEFORE_READY: &str = "exited before becoming ready";

pub fn reduce(state: &mut RootState, action: &TunnelsAction) -> Vec<Effect> {
    match action {
        TunnelsAction::SelectNext => state.tunnels.select_next(),
        TunnelsAction::SelectPrevious => state.tunnels.select_previous(),
        TunnelsAction::KillSelected => return kill_selected(state),
        TunnelsAction::DismissSelected => dismiss_selected(state),
    }
    Vec::new()
}

/// Asks the runtime to stop the selected tunnel and marks it `Stopping` so the
/// row says so while the child is being reaped.
///
/// Only a `Starting` or `Active` tunnel has a stop worth requesting: a terminal
/// one has no child left, and a `Stopping` one already has a stop in flight, so
/// both are silent no-ops rather than a second signal.
fn kill_selected(state: &mut RootState) -> Vec<Effect> {
    let Some(entry) = state.tunnels.selected() else {
        return Vec::new();
    };
    if !matches!(entry.status, TunnelStatus::Starting | TunnelStatus::Active) {
        return Vec::new();
    }
    let id = entry.id;
    state.tunnels.mark_stopping(id);
    vec![Effect::StopTunnel { id }]
}

/// Drops the selected row. A live tunnel is not dismissible: removing it would
/// discard the only handle that can stop its child, so the user is told to kill
/// it first instead.
fn dismiss_selected(state: &mut RootState) {
    let Some(entry) = state.tunnels.selected() else {
        return;
    };
    if !entry.status.is_terminal() {
        state
            .system
            .set_status_message("Kill this tunnel before dismissing it");
        return;
    }
    let id = entry.id;
    state.tunnels.remove(id);
}

/// Applies a child exit, whose meaning depends on what the tunnel was doing:
/// a tunnel that dies while `Starting` never came up and is a failure, while
/// one that dies after it was `Active` (or was being stopped) simply ended.
/// An already-terminal entry keeps the reason it already has, and an unknown id
/// changes nothing.
pub(crate) fn apply_exited(state: &mut RootState, id: TunnelId) {
    let Some(entry) = state.tunnels.entries().iter().find(|entry| entry.id == id) else {
        return;
    };
    match entry.status {
        TunnelStatus::Starting => state.tunnels.mark_failed(id, EXITED_BEFORE_READY),
        TunnelStatus::Active | TunnelStatus::Stopping => state.tunnels.mark_stopped(id),
        TunnelStatus::Failed { .. } | TunnelStatus::Stopped => {}
    }
}
