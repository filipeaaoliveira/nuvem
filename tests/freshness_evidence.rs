//! Evidence: renders the k9s-style status/freshness region (issue #22) so a
//! reviewer can see the live "Updated: <age>" line and operation phases exactly
//! as the end user sees them in the TUI. Writes rendered frames to the
//! evidence directory when NUVEM_EVIDENCE_DIR is set.

use chrono::{Duration, Local};
use nuvem::app::projection::{
    AppFrameProjection, FrameContextProjection, FrameOperationProjection,
};
use nuvem::app::state::system::OperationPhase;
use nuvem::ui::shared::frame::render_status;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::layout::Rect;

fn projection(
    phase: OperationPhase,
    label: &str,
    message: Option<&str>,
    last_refresh_at: Option<chrono::DateTime<Local>>,
) -> AppFrameProjection {
    AppFrameProjection {
        app_title: "nuvem",
        view_title: "Resources".to_string(),
        breadcrumbs: Vec::new(),
        context: FrameContextProjection {
            configuration: Some("dev".to_string()),
            account: None,
            project: Some("dev-project".to_string()),
            region: None,
            zone: None,
        },
        service_label: Some("GCE instances"),
        operation: FrameOperationProjection {
            phase,
            label: label.to_string(),
            message: message.map(ToString::to_string),
            message_label: message.map(|_| label.to_string()),
            last_refresh_at,
        },
        footer_hints: Vec::new(),
        footer_pinned_hints: Vec::new(),
    }
}

fn render_lines(projection: &AppFrameProjection, width: u16) -> String {
    let backend = TestBackend::new(width, 2);
    let mut terminal = Terminal::new(backend).expect("terminal should initialize");
    terminal
        .draw(|frame| render_status(frame, Rect::new(0, 0, width, 2), projection))
        .expect("status should render");
    terminal
        .backend()
        .buffer()
        .content()
        .chunks(usize::from(width))
        .map(|row| {
            row.iter()
                .map(|cell| cell.symbol())
                .collect::<String>()
                .trim_end()
                .to_string()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn renders_status_and_freshness_evidence() {
    // The status widget computes the freshness age against the real wall clock
    // (Local::now()) at render time, so anchor the "last successful refresh"
    // timestamps relative to now to exercise the live age formatting.
    let base = Local::now();
    let width = 60u16;

    // 1. First entry into Resources / no successful load yet -> "Never".
    let idle = projection(OperationPhase::Idle, "Ready", None, None);
    let idle_frame = render_lines(&idle, width);

    // 2. First list load auto-fires -> Loading (LOAD), not "Press r to load".
    let loading = projection(
        OperationPhase::Loading,
        "GCE instances",
        Some("Loading GCE instances"),
        None,
    );
    let loading_frame = render_lines(&loading, width);

    // 3. Background poll ticks a subsequent refresh -> Refreshing (SYNC).
    let refreshing = projection(
        OperationPhase::Refreshing,
        "GCE instances",
        Some("Refreshing GCE instances"),
        Some(base - Duration::seconds(12)),
    );
    let refreshing_frame = render_lines(&refreshing, width);

    // 4. Healthy after a refresh, with the live relative freshness age counting up.
    let just_now = render_lines(
        &projection(OperationPhase::Healthy, "GCE instances", None, Some(base)),
        width,
    );
    let twelve_s = render_lines(
        &projection(
            OperationPhase::Healthy,
            "GCE instances",
            None,
            Some(base - Duration::seconds(12)),
        ),
        width,
    );
    let three_m = render_lines(
        &projection(
            OperationPhase::Healthy,
            "GCE instances",
            None,
            Some(base - Duration::seconds(190)),
        ),
        width,
    );

    // Behavioural assertions mirroring the intent.
    assert!(idle_frame.contains("Updated: Never"));
    assert!(loading_frame.contains("[LOAD]"));
    assert!(refreshing_frame.contains("[SYNC]"));
    assert!(just_now.contains("Updated: just now"));
    assert!(twelve_s.contains("Updated: 12s ago"));
    assert!(three_m.contains("Updated: 3m ago"));

    let report = format!(
        "nuvem issue #22 — k9s-style status & live freshness (rendered TUI status region, 60x2)\n\
         (freshness age computed live against the wall clock at render time)\n\n\
         [1] Enter Resources, nothing loaded yet — no more \"Press r to load\":\n{idle}\n\n\
         [2] Auto-load fires the FIRST list (phase = Loading / LOAD):\n{loading}\n\n\
         [3] Background poll refresh in flight (phase = Refreshing / SYNC), age from last success:\n{refreshing}\n\n\
         [4] Healthy after refresh — live relative freshness age counts up:\n\
         just now:\n{just_now}\n\n\
         12 seconds later:\n{twelve_s}\n\n\
         3 minutes later:\n{three_m}\n",
        idle = idle_frame,
        loading = loading_frame,
        refreshing = refreshing_frame,
        just_now = just_now,
        twelve_s = twelve_s,
        three_m = three_m,
    );

    println!("\n{report}");

    if let Ok(dir) = std::env::var("NUVEM_EVIDENCE_DIR") {
        let path = std::path::Path::new(&dir).join("freshness_status_render.txt");
        std::fs::write(&path, report).expect("write evidence file");
    }
}
