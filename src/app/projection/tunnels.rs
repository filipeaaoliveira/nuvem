//! Render-facing projection for the managed-tunnel workspace.
//!
//! Two properties are deliberate.
//!
//! **No clock is read here.** AGE is derived from each entry's `started_at`
//! against an `Instant` the caller supplies, so the whole projection is a pure
//! function of `(state, now)` and every age can be asserted without sleeping.
//!
//! **Every string is sanitized.** Target and remote labels come from resource
//! names and a tunnel's message originates in child-process output, so all of
//! them pass through `nuvem_domain::gcp::text::sanitize_display_text` before
//! they can reach the renderer.

use std::time::{Duration, Instant};

use nuvem_domain::gcp::text::sanitize_display_text;

use crate::app::projection::table::{
    Cell, CellAlign, NO_COLUMN, RowCounts, SortDirection, SortSpec, StatusSemantic, TableColumn,
    TableModel, TableRow,
};
use crate::app::state::root::RootState;
use crate::app::state::tunnels::{TunnelEntry, TunnelStatus};

/// Shown instead of the table when no tunnel has ever been started. The `t`
/// it names is the resource-row key that starts one.
pub const TUNNELS_EMPTY_MESSAGE: &str =
    "No tunnels. Press `t` on a GCE or Cloud SQL row to start one.";

/// Stands in for the LOCAL cell while a freshly started tunnel has not yet
/// reported the port it bound. See [`local_cell`].
const LOCAL_PORT_PENDING: &str = "—";

/// Which body the renderer should paint. There is no loading or degraded
/// state: a tunnel's status lives on its own row, so the list itself is only
/// ever empty or populated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TunnelsBodyState {
    Empty,
    Rows,
}

/// The full render-facing projection for `Workspace::Tunnels`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TunnelsWorkspaceProjection {
    pub table: TableModel,
    pub body_state: TunnelsBodyState,
    /// Tunnels known to be serving traffic (`TunnelsState::active_count`).
    pub active_count: usize,
    /// Every row in the list, including finished ones awaiting dismissal.
    pub total_count: usize,
}

/// Projects the tunnel list into the shared render-facing table model, ageing
/// each row against `now`.
pub fn project_tunnels(root: &RootState, now: Instant) -> TunnelsWorkspaceProjection {
    let tunnels = &root.tunnels;
    let rows: Vec<TableRow> = tunnels
        .entries()
        .iter()
        .map(|entry| tunnel_row(entry, now))
        .collect();

    let body_state = if rows.is_empty() {
        TunnelsBodyState::Empty
    } else {
        TunnelsBodyState::Rows
    };
    let selected_row = (!rows.is_empty()).then(|| tunnels.selected_index());
    let count = rows.len();

    TunnelsWorkspaceProjection {
        table: TableModel {
            columns: tunnel_columns(),
            rows,
            selected_row,
            // The tunnel list is insertion-ordered (oldest first) and has
            // neither a column cursor nor a user-facing sort: `←`/`→` and `s`
            // are not bound here. `NO_COLUMN` therefore points both fields at
            // an index no column has, so `render_table` decorates no header
            // with a `[...]` selection bracket or a sort arrow — claiming
            // either would advertise a control the workspace does not offer.
            selected_column: NO_COLUMN,
            sort: SortSpec {
                column: NO_COLUMN,
                direction: SortDirection::Ascending,
            },
            counts: RowCounts {
                filtered: count,
                total: count,
            },
            show_mark_column: false,
        },
        body_state,
        active_count: tunnels.active_count(),
        total_count: count,
    }
}

fn tunnel_row(entry: &TunnelEntry, now: Instant) -> TableRow {
    TableRow {
        cells: vec![
            Cell::status(status_label(&entry.status), status_semantic(&entry.status)),
            Cell::plain(entry.kind.label()),
            Cell::plain(local_cell(entry)),
            Cell::plain(sanitize_display_text(&target_cell(entry))),
            Cell::plain(format_age(now.saturating_duration_since(entry.started_at))),
            Cell::plain(
                entry
                    .message
                    .as_deref()
                    .map(sanitize_display_text)
                    .unwrap_or_default(),
            ),
        ],
        marked: false,
    }
}

/// An entry is inserted before its supervised child reports the port it bound,
/// and carries `local_port == 0` until then. `localhost:0` would name a port
/// nothing is listening on, so the cell holds a placeholder until the real port
/// is known.
fn local_cell(entry: &TunnelEntry) -> String {
    if entry.local_port == 0 {
        LOCAL_PORT_PENDING.to_string()
    } else {
        format!("localhost:{}", entry.local_port)
    }
}

/// The target cell pairs the near-side resource with the far side it forwards
/// to, since neither alone identifies what the tunnel reaches.
fn target_cell(entry: &TunnelEntry) -> String {
    if entry.remote_label.is_empty() {
        entry.target_label.clone()
    } else {
        format!("{} -> {}", entry.target_label, entry.remote_label)
    }
}

fn status_label(status: &TunnelStatus) -> &'static str {
    match status {
        TunnelStatus::Starting => "Starting",
        TunnelStatus::Active => "Active",
        TunnelStatus::Stopping => "Stopping",
        TunnelStatus::Failed { .. } => "Failed",
        TunnelStatus::Stopped => "Stopped",
    }
}

/// Maps a tunnel's state onto the shared status semantics, which
/// `ui::shared::table` renders through the semantic theme functions:
/// `Busy` -> `status_loading`, `Healthy` -> `status_healthy`,
/// `Degraded` -> `status_error`, `Neutral` -> `muted`.
fn status_semantic(status: &TunnelStatus) -> StatusSemantic {
    match status {
        TunnelStatus::Starting | TunnelStatus::Stopping => StatusSemantic::Busy,
        TunnelStatus::Active => StatusSemantic::Healthy,
        TunnelStatus::Failed { .. } => StatusSemantic::Degraded,
        TunnelStatus::Stopped => StatusSemantic::Neutral,
    }
}

/// Compact relative age: `12s`, `4m`, `2h`. Deliberately one unit wide — the
/// column is a glanceable "how long has this been up", not a duration readout.
fn format_age(elapsed: Duration) -> String {
    let seconds = elapsed.as_secs();
    if seconds < 60 {
        format!("{seconds}s")
    } else if seconds < 3600 {
        format!("{}m", seconds / 60)
    } else {
        format!("{}h", seconds / 3600)
    }
}

fn tunnel_columns() -> Vec<TableColumn> {
    vec![
        TableColumn {
            key: "status",
            header: "STATUS",
            min_width: 6,
            preferred_width: 10,
            priority: 0,
            align: CellAlign::Left,
            sortable: false,
        },
        TableColumn {
            key: "kind",
            header: "KIND",
            min_width: 4,
            preferred_width: 10,
            priority: 1,
            align: CellAlign::Left,
            sortable: false,
        },
        TableColumn {
            key: "local",
            header: "LOCAL",
            min_width: 6,
            preferred_width: 16,
            priority: 2,
            align: CellAlign::Left,
            sortable: false,
        },
        TableColumn {
            key: "target",
            header: "TARGET",
            min_width: 8,
            preferred_width: 32,
            priority: 3,
            align: CellAlign::Left,
            sortable: false,
        },
        TableColumn {
            key: "age",
            header: "AGE",
            min_width: 3,
            preferred_width: 6,
            priority: 4,
            align: CellAlign::Right,
            sortable: false,
        },
        TableColumn {
            key: "message",
            header: "MESSAGE",
            min_width: 8,
            preferred_width: 40,
            priority: 5,
            align: CellAlign::Left,
            sortable: false,
        },
    ]
}
