//! Row content, status mapping, and age formatting for the Tunnels workspace
//! projection.
//!
//! Every age assertion supplies its own `now`, so no test here ever sleeps:
//! `project_tunnels` derives AGE from `started_at` against the caller's
//! `Instant` rather than reading the clock itself.

use std::time::{Duration, Instant};

use nuvem::app::projection::StatusSemantic;
use nuvem::app::projection::table::CellKind;
use nuvem::app::projection::tunnels::{TUNNELS_EMPTY_MESSAGE, TunnelsBodyState, project_tunnels};
use nuvem::app::state::root::{RootState, Workspace};
use nuvem::app::state::tunnels::{TunnelEntry, TunnelStatus};
use nuvem_domain::gcp::configuration::{GcloudConfigurationName, GcloudContext};
use nuvem_domain::gcp::tunnel::TunnelKind;

fn context() -> GcloudContext {
    GcloudContext {
        configuration: GcloudConfigurationName::new("dev"),
        account: None,
        project: Some("dev-project".to_string()),
        region: None,
        zone: None,
    }
}

/// A root state in the Tunnels workspace holding one entry per supplied
/// `(kind, target, remote, port, status)` tuple, all started at `started_at`.
fn root_with(
    entries: Vec<(TunnelKind, &str, &str, u16, TunnelStatus)>,
    started_at: Instant,
) -> RootState {
    let mut root = RootState {
        workspace: Workspace::Tunnels,
        ..Default::default()
    };
    for (kind, target, remote, port, status) in entries {
        let id = root.tunnels.mint_id();
        let message = match &status {
            TunnelStatus::Failed { message } => Some(message.clone()),
            _ => None,
        };
        let entry = TunnelEntry {
            id,
            kind,
            target_label: target.to_string(),
            local_port: port,
            remote_label: remote.to_string(),
            status: status.clone(),
            started_at,
            context: context(),
            message,
        };
        root.tunnels.insert_starting(entry);
        // `insert_starting` forces `Starting`; re-apply the requested status
        // through the state's own transitions so the row under test is the one
        // the reducer would produce.
        match status {
            TunnelStatus::Starting => {}
            TunnelStatus::Active => root.tunnels.mark_active(id, None),
            TunnelStatus::Stopping => root.tunnels.mark_stopping(id),
            TunnelStatus::Stopped => root.tunnels.mark_stopped(id),
            TunnelStatus::Failed { message } => root.tunnels.mark_failed(id, message),
        }
    }
    root
}

fn cell(root: &RootState, now: Instant, row: usize, column: usize) -> String {
    project_tunnels(root, now).table.rows[row].cells[column]
        .text
        .clone()
}

#[test]
fn columns_are_status_kind_local_target_age_message() {
    let now = Instant::now();
    let root = root_with(Vec::new(), now);
    let headers: Vec<&str> = project_tunnels(&root, now)
        .table
        .columns
        .iter()
        .map(|column| column.header)
        .collect();
    assert_eq!(
        headers,
        vec!["STATUS", "KIND", "LOCAL", "TARGET", "AGE", "MESSAGE"]
    );
}

#[test]
fn a_row_shows_status_kind_local_port_and_target() {
    let now = Instant::now();
    let root = root_with(
        vec![(
            TunnelKind::IapTcp,
            "web-1",
            "22",
            5432,
            TunnelStatus::Active,
        )],
        now,
    );
    assert_eq!(cell(&root, now, 0, 0), "Active");
    assert_eq!(cell(&root, now, 0, 1), "IAP");
    assert_eq!(cell(&root, now, 0, 2), "localhost:5432");
    assert_eq!(cell(&root, now, 0, 3), "web-1 -> 22");
}

/// A tunnel is inserted before its supervised child reports the port it bound,
/// so `local_port == 0` means "not known yet". Rendering it as `localhost:0`
/// would claim a port nothing is listening on, so the cell stays a placeholder
/// until the real port arrives.
#[test]
fn the_local_cell_is_a_placeholder_until_the_bound_port_is_known() {
    let now = Instant::now();
    let root = root_with(
        vec![
            (TunnelKind::IapTcp, "a", "22", 0, TunnelStatus::Starting),
            (TunnelKind::IapTcp, "b", "22", 15432, TunnelStatus::Active),
        ],
        now,
    );
    assert_eq!(cell(&root, now, 0, 2), "—");
    assert_eq!(cell(&root, now, 1, 2), "localhost:15432");
}

#[test]
fn status_cells_carry_the_semantic_the_theme_maps_to_each_state() {
    let now = Instant::now();
    let root = root_with(
        vec![
            (TunnelKind::IapTcp, "a", "22", 1, TunnelStatus::Starting),
            (TunnelKind::IapTcp, "b", "22", 2, TunnelStatus::Active),
            (TunnelKind::IapTcp, "c", "22", 3, TunnelStatus::Stopping),
            (
                TunnelKind::IapTcp,
                "d",
                "22",
                4,
                TunnelStatus::Failed {
                    message: "boom".to_string(),
                },
            ),
            (TunnelKind::IapTcp, "e", "22", 5, TunnelStatus::Stopped),
        ],
        now,
    );
    let projection = project_tunnels(&root, now);
    let semantics: Vec<_> = projection
        .table
        .rows
        .iter()
        .map(|row| match &row.cells[0].kind {
            CellKind::Status(semantic) => *semantic,
            other => panic!("status cell must be a status cell, got {other:?}"),
        })
        .collect();
    assert_eq!(
        semantics,
        vec![
            // Busy -> theme::status_loading
            StatusSemantic::Busy,
            // Healthy -> theme::status_healthy
            StatusSemantic::Healthy,
            StatusSemantic::Busy,
            // Degraded -> theme::status_error
            StatusSemantic::Degraded,
            // Neutral -> theme::muted
            StatusSemantic::Neutral,
        ]
    );
}

#[test]
fn status_labels_name_each_state() {
    let now = Instant::now();
    let root = root_with(
        vec![
            (TunnelKind::IapTcp, "a", "22", 1, TunnelStatus::Starting),
            (
                TunnelKind::SshLocalForward,
                "b",
                "22",
                2,
                TunnelStatus::Active,
            ),
            (
                TunnelKind::CloudSqlProxy,
                "c",
                "5432",
                3,
                TunnelStatus::Stopping,
            ),
            (
                TunnelKind::IapTcp,
                "d",
                "22",
                4,
                TunnelStatus::Failed {
                    message: "boom".to_string(),
                },
            ),
            (TunnelKind::IapTcp, "e", "22", 5, TunnelStatus::Stopped),
        ],
        now,
    );
    let labels: Vec<String> = (0..5).map(|row| cell(&root, now, row, 0)).collect();
    assert_eq!(
        labels,
        vec!["Starting", "Active", "Stopping", "Failed", "Stopped"]
    );
}

#[test]
fn kind_cells_use_the_domain_badge() {
    let now = Instant::now();
    let root = root_with(
        vec![
            (TunnelKind::IapTcp, "a", "22", 1, TunnelStatus::Active),
            (
                TunnelKind::SshLocalForward,
                "b",
                "22",
                2,
                TunnelStatus::Active,
            ),
            (
                TunnelKind::CloudSqlProxy,
                "c",
                "5432",
                3,
                TunnelStatus::Active,
            ),
        ],
        now,
    );
    let kinds: Vec<String> = (0..3).map(|row| cell(&root, now, row, 1)).collect();
    assert_eq!(kinds, vec!["IAP", "SSH -L", "SQL proxy"]);
}

#[test]
fn age_is_derived_from_the_caller_supplied_instant() {
    let started_at = Instant::now();
    let root = root_with(
        vec![(
            TunnelKind::IapTcp,
            "web-1",
            "22",
            5432,
            TunnelStatus::Active,
        )],
        started_at,
    );
    let age = |after: Duration| cell(&root, started_at + after, 0, 4);

    assert_eq!(age(Duration::from_secs(0)), "0s");
    assert_eq!(age(Duration::from_secs(12)), "12s");
    assert_eq!(age(Duration::from_secs(59)), "59s");
    assert_eq!(age(Duration::from_secs(60)), "1m");
    assert_eq!(age(Duration::from_secs(4 * 60 + 30)), "4m");
    assert_eq!(age(Duration::from_secs(3599)), "59m");
    assert_eq!(age(Duration::from_secs(2 * 3600)), "2h");
}

/// A `now` earlier than `started_at` (a caller passing a stale instant) must
/// not panic; it reads as a zero-second age.
#[test]
fn an_age_measured_before_the_start_is_zero_rather_than_a_panic() {
    let started_at = Instant::now() + Duration::from_secs(60);
    let root = root_with(
        vec![(
            TunnelKind::IapTcp,
            "web-1",
            "22",
            5432,
            TunnelStatus::Active,
        )],
        started_at,
    );
    assert_eq!(
        cell(&root, started_at - Duration::from_secs(30), 0, 4),
        "0s"
    );
}

#[test]
fn the_message_column_shows_a_failure_reason_and_is_empty_otherwise() {
    let now = Instant::now();
    let root = root_with(
        vec![
            (
                TunnelKind::IapTcp,
                "a",
                "22",
                1,
                TunnelStatus::Failed {
                    message: "address already in use".to_string(),
                },
            ),
            (TunnelKind::IapTcp, "b", "22", 2, TunnelStatus::Active),
        ],
        now,
    );
    assert_eq!(cell(&root, now, 0, 5), "address already in use");
    assert_eq!(cell(&root, now, 1, 5), "");
}

/// Tunnel messages and labels come from child-process output, so every cell
/// must go through `sanitize_display_text` before it reaches the renderer.
#[test]
fn every_cell_is_sanitized() {
    let now = Instant::now();
    let root = root_with(
        vec![(
            TunnelKind::IapTcp,
            "web\u{1b}[2J-1",
            "2\u{1b}[2J2",
            5432,
            TunnelStatus::Failed {
                message: "bad\u{1b}[2Jreason".to_string(),
            },
        )],
        now,
    );
    let joined = project_tunnels(&root, now).table.rows[0]
        .cells
        .iter()
        .map(|cell| cell.text.clone())
        .collect::<Vec<_>>()
        .join(" ");
    assert!(!joined.contains('\u{1b}'), "ESC leaked into a tunnel row");
}

#[test]
fn an_empty_list_projects_the_empty_body_state_with_the_start_hint() {
    let now = Instant::now();
    let root = root_with(Vec::new(), now);
    let projection = project_tunnels(&root, now);
    assert_eq!(projection.body_state, TunnelsBodyState::Empty);
    assert!(projection.table.rows.is_empty());
    assert_eq!(
        TUNNELS_EMPTY_MESSAGE,
        "No tunnels. Press `t` on a GCE or Cloud SQL row to start one."
    );
}

#[test]
fn a_populated_list_projects_the_rows_body_state_and_the_selection() {
    let now = Instant::now();
    let mut root = root_with(
        vec![
            (TunnelKind::IapTcp, "a", "22", 1, TunnelStatus::Active),
            (TunnelKind::IapTcp, "b", "22", 2, TunnelStatus::Active),
        ],
        now,
    );
    root.tunnels.select_next();
    let projection = project_tunnels(&root, now);
    assert_eq!(projection.body_state, TunnelsBodyState::Rows);
    assert_eq!(projection.table.selected_row, Some(1));
    assert_eq!(projection.table.counts.total, 2);
    assert_eq!(projection.table.counts.filtered, 2);
}

/// The header summary counts only tunnels that can actually carry traffic,
/// matching `TunnelsState::active_count`.
#[test]
fn the_header_counts_active_tunnels_against_the_total() {
    let now = Instant::now();
    let root = root_with(
        vec![
            (TunnelKind::IapTcp, "a", "22", 1, TunnelStatus::Active),
            (TunnelKind::IapTcp, "b", "22", 2, TunnelStatus::Starting),
            (TunnelKind::IapTcp, "c", "22", 3, TunnelStatus::Stopped),
        ],
        now,
    );
    let projection = project_tunnels(&root, now);
    assert_eq!(projection.active_count, 1);
    assert_eq!(projection.total_count, 3);
}
