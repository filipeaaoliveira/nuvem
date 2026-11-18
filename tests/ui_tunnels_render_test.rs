//! Render coverage for the Tunnels workspace body: the populated table, the
//! empty state, and the shared chrome the workspace must not repaint.

use std::time::Instant;

use nuvem::app::AppState;
use nuvem::app::state::root::Workspace;
use nuvem::app::state::tunnels::{TunnelEntry, TunnelStatus};
use nuvem::services::runtime::session::RuntimeSession;
use nuvem_domain::gcp::configuration::{GcloudConfigurationName, GcloudContext};
use nuvem_domain::gcp::tunnel::TunnelKind;
use ratatui::{Terminal, backend::TestBackend};

fn lines(terminal: &Terminal<TestBackend>) -> Vec<String> {
    let buffer = terminal.backend().buffer();
    (0..buffer.area.height)
        .map(|y| {
            (0..buffer.area.width)
                .map(|x| buffer.cell((x, y)).unwrap().symbol().to_string())
                .collect::<String>()
        })
        .collect()
}

fn context() -> GcloudContext {
    GcloudContext {
        configuration: GcloudConfigurationName::new("dev"),
        account: None,
        project: Some("dev-project".to_string()),
        region: None,
        zone: None,
    }
}

fn app_with(entries: Vec<(TunnelKind, &str, &str, u16, TunnelStatus)>) -> AppState {
    let mut app = AppState::new();
    app.root.workspace = Workspace::Tunnels;
    let started_at = Instant::now();
    for (kind, target, remote, port, status) in entries {
        let id = app.root.tunnels.mint_id();
        app.root.tunnels.insert_starting(TunnelEntry {
            id,
            kind,
            target_label: target.to_string(),
            local_port: port,
            remote_label: remote.to_string(),
            status: TunnelStatus::Starting,
            started_at,
            context: context(),
            message: None,
        });
        match status {
            TunnelStatus::Starting => {}
            TunnelStatus::Active => app.root.tunnels.mark_active(id, None),
            TunnelStatus::Stopping => app.root.tunnels.mark_stopping(id),
            TunnelStatus::Stopped => app.root.tunnels.mark_stopped(id),
            TunnelStatus::Failed { message } => app.root.tunnels.mark_failed(id, message),
        }
    }
    app
}

fn render(app: AppState) -> String {
    let session = RuntimeSession::new_for_test(app);
    let mut terminal = Terminal::new(TestBackend::new(120, 30)).unwrap();
    terminal
        .draw(|frame| nuvem::ui::render(frame, &session))
        .unwrap();
    lines(&terminal).join("\n")
}

#[test]
fn populated_view_shows_headers_local_port_target_and_status() {
    let rendered = render(app_with(vec![
        (
            TunnelKind::IapTcp,
            "web-1",
            "22",
            5432,
            TunnelStatus::Active,
        ),
        (
            TunnelKind::CloudSqlProxy,
            "orders-db",
            "5432",
            15432,
            TunnelStatus::Failed {
                message: "address already in use".to_string(),
            },
        ),
    ]));

    for header in ["STATUS", "KIND", "LOCAL", "TARGET", "AGE", "MESSAGE"] {
        assert!(rendered.contains(header), "missing {header} column header");
    }
    assert!(rendered.contains("localhost:5432"), "local port not shown");
    assert!(rendered.contains("web-1 -> 22"), "target not shown");
    assert!(rendered.contains("Active"), "Active status not shown");
    assert!(rendered.contains("Failed"), "Failed status not shown");
    assert!(
        rendered.contains("address already in use"),
        "failure message not shown"
    );
    assert!(rendered.contains("SQL proxy"), "kind badge not shown");
}

#[test]
fn populated_view_summarizes_active_against_listed() {
    let rendered = render(app_with(vec![
        (TunnelKind::IapTcp, "a", "22", 1, TunnelStatus::Active),
        (TunnelKind::IapTcp, "b", "22", 2, TunnelStatus::Stopped),
    ]));
    assert!(
        rendered.contains("Active: 1") && rendered.contains("Listed: 2"),
        "summary line missing from body"
    );
}

#[test]
fn empty_view_shows_the_start_hint() {
    let rendered = render(app_with(Vec::new()));
    assert!(
        rendered.contains("No tunnels. Press `t` on a GCE or Cloud SQL row"),
        "empty-state hint not rendered"
    );
}

/// Labels and messages reach the renderer from child-process output, so an
/// escape sequence must never survive into the terminal buffer. The sanitized
/// text is asserted present as well as ESC-free: dropping the cell outright
/// would also remove the ESC, and that is not sanitization.
#[test]
fn render_sanitizes_child_supplied_labels_and_messages() {
    let rendered = render(app_with(vec![(
        TunnelKind::IapTcp,
        "web\u{1b}[2J-1",
        "22",
        5432,
        TunnelStatus::Failed {
            message: "bad\u{1b}[2Jreason".to_string(),
        },
    )]));
    assert!(!rendered.contains('\u{1b}'), "ESC byte leaked into buffer");
    // `sanitize_display_text` replaces each control character with U+FFFD and
    // keeps everything around it, so both cells must still be on screen.
    assert!(
        rendered.contains("web\u{fffd}[2J-1 -> 22"),
        "sanitized target missing: the cell was dropped, not sanitized"
    );
    assert!(
        rendered.contains("bad\u{fffd}[2Jreason"),
        "sanitized message missing: the cell was dropped, not sanitized"
    );
}

/// The workspace owns its body only: the shared frame around it must still be
/// painted, and the tunnel renderer must not have drawn over it.
#[test]
fn the_shared_frame_survives_around_the_tunnel_body() {
    let rendered = render(app_with(vec![(
        TunnelKind::IapTcp,
        "web-1",
        "22",
        5432,
        TunnelStatus::Active,
    )]));
    assert!(rendered.contains("nuvem"), "app title missing");
    // The footer hints come from `project_app_frame`'s Tunnels metadata and are
    // the chrome nothing in the body paints. A bare `contains("Tunnels")` would
    // prove nothing here: the body block's own title is "Tunnels" too.
    assert!(rendered.contains("Kill tunnel"), "footer hint missing");
    assert!(rendered.contains("Move selection"), "footer hint missing");
}
