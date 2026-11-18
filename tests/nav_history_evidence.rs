//! Evidence generator for issue #26 — navigation history, breadcrumbs, and
//! adaptive list+detail layouts.
//!
//! Every frame here is produced through the real `nuvem::ui::render` entrypoint
//! — the exact surface an end user sees — driven through `RuntimeSession` with
//! the same actions the keymap dispatches. ASCII "screenshots" of the terminal
//! buffer are written to the evidence directory (when
//! `NUVEM_EVIDENCE_DIR` is set) so a reviewer can
//! see the breadcrumb path, back/forward footer hints, the wide split
//! list+detail layout, and the narrow full-screen drill-down as rendered.

use std::fs;
use std::path::PathBuf;

use nuvem::app::projection::project_app_frame;
use nuvem::app::{Action, AppState, EffectResult, NavAction, ResourcesAction, WorkspaceAction};
use nuvem::services::runtime::session::RuntimeSession;
use nuvem_domain::gcp::configuration::{GcloudConfiguration, GcloudConfigurationName};
use nuvem_domain::gcp::resource::{ComputeInstance, ResourceStatus};
use ratatui::{Terminal, backend::TestBackend};

fn render_lines(session: &RuntimeSession, width: u16, height: u16) -> Vec<String> {
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).expect("terminal should initialize");
    terminal
        .draw(|frame| nuvem::ui::render(frame, session))
        .expect("render should succeed");
    let buffer = terminal.backend().buffer();
    let w = buffer.area.width as usize;
    buffer
        .content()
        .chunks(w)
        .map(|row| {
            row.iter()
                .map(|cell| cell.symbol())
                .collect::<String>()
                .trim_end()
                .to_string()
        })
        .collect()
}

fn frame_box(title: &str, width: u16, lines: &[String]) -> String {
    let mut body = String::new();
    body.push_str(title);
    body.push('\n');
    body.push('+');
    body.push_str(&"-".repeat(width as usize));
    body.push_str("+\n");
    for line in lines {
        let mut padded = line.clone();
        while padded.chars().count() < width as usize {
            padded.push(' ');
        }
        body.push('|');
        body.push_str(&padded);
        body.push_str("|\n");
    }
    body.push('+');
    body.push_str(&"-".repeat(width as usize));
    body.push_str("+\n");
    body
}

fn breadcrumb_of(session: &RuntimeSession) -> String {
    project_app_frame(&session.app().root)
        .breadcrumbs
        .join(" > ")
}

fn footer_hints_of(session: &RuntimeSession) -> String {
    project_app_frame(&session.app().root)
        .footer_hints
        .iter()
        .map(|h| format!("{}:{}", h.key, h.label))
        .collect::<Vec<_>>()
        .join("  ")
}

async fn load_active_dev(session: &mut RuntimeSession) {
    session
        .apply_effect_result(EffectResult::ConfigurationsLoaded {
            configurations: vec![GcloudConfiguration {
                name: GcloudConfigurationName::new("dev"),
                is_active: true,
                account: Some("dev@example.com".to_string()),
                project: Some("dev-project".to_string()),
                region: None,
                zone: None,
            }],
        })
        .await;
}

async fn load_dev_instances(session: &mut RuntimeSession) {
    load_active_dev(session).await;
    let context = session.app().root.current_context().unwrap().clone();
    session
        .apply_effect_result(EffectResult::ComputeInstancesLoaded {
            context,
            instances: vec![
                ComputeInstance {
                    id: "1001".to_string(),
                    name: "api-dev-1".to_string(),
                    zone: "europe-west1-b".to_string(),
                    status: ResourceStatus::Running,
                    machine_type: "e2-medium".to_string(),
                    internal_ip: Some("10.0.0.4".to_string()),
                    external_ip: Some("34.10.10.10".to_string()),
                },
                ComputeInstance {
                    id: "1002".to_string(),
                    name: "worker-dev-1".to_string(),
                    zone: "europe-west1-c".to_string(),
                    status: ResourceStatus::Stopped,
                    machine_type: "e2-small".to_string(),
                    internal_ip: Some("10.0.0.5".to_string()),
                    external_ip: None,
                },
            ],
        })
        .await;
}

#[tokio::test]
async fn navigation_history_and_adaptive_layout_evidence() {
    let mut report = String::new();
    report.push_str(
        "nuvem issue #26 — navigation history, breadcrumbs & adaptive list+detail layouts\n\
         (all frames rendered through the real nuvem::ui::render TUI entrypoint)\n\n",
    );

    // ---------------------------------------------------------------------
    // WIDE terminal (160x30): Resources list view.
    // Breadcrumb shows "Home > Resources > GCE instances"; body splits into a
    // left list + right summary-only preview of the highlighted row (built from
    // already-loaded rows, no extra gcloud describe call).
    // ---------------------------------------------------------------------
    let mut session = RuntimeSession::new_for_test(AppState::new());
    load_dev_instances(&mut session).await;
    session
        .dispatch_action(Action::Workspace(WorkspaceAction::Resources))
        .await;

    let wide_list = render_lines(&session, 160, 30);
    report.push_str(&format!(
        "[1] WIDE (160 cols) list view — breadcrumb + split list|preview pane (no forward history yet)\n\
         breadcrumb : {}\n    footer   : {}\n\n{}\n",
        breadcrumb_of(&session),
        footer_hints_of(&session),
        frame_box("rendered TUI (160x30):", 160, &wide_list),
    ));

    // ---------------------------------------------------------------------
    // Drill into the highlighted resource (OpenSelected == Enter).
    // This records a forward-navigation history entry; the breadcrumb now
    // includes the resource identity + active detail tab, and the wide layout
    // keeps the list visible on the left with the tabbed detail on the right.
    // ---------------------------------------------------------------------
    session
        .dispatch_action(Action::Resources(ResourcesAction::OpenSelected))
        .await;

    let wide_detail = render_lines(&session, 160, 30);
    report.push_str(&format!(
        "[2] WIDE detail view after Enter — breadcrumb gains resource + tab, back hint appears\n\
         breadcrumb : {}\n    footer   : {}\n\n{}\n",
        breadcrumb_of(&session),
        footer_hints_of(&session),
        frame_box("rendered TUI (160x30):", 160, &wide_detail),
    ));

    // Snapshot config identity before navigating back — must not change.
    let config_before = format!("{:?}", session.app().root.current_context());

    // ---------------------------------------------------------------------
    // Esc / Alt-Left == NavAction::Back — return to the prior list entry.
    // gcloud configuration MUST be unchanged; a forward hint now appears.
    // ---------------------------------------------------------------------
    session
        .dispatch_action(Action::Navigation(NavAction::Back))
        .await;
    let config_after = format!("{:?}", session.app().root.current_context());

    // Sampled here rather than at the assertion below: the claim is "after
    // Back a forward hint is offered", and by then the session has navigated
    // Forward again, so the forward entry it names is gone. Reading it at the
    // point the sentence is about is what makes the assertion about the hint
    // rather than about the render width that happened to still show it.
    let hints_after_back = footer_hints_of(&session);
    let after_back = render_lines(&session, 160, 30);
    report.push_str(&format!(
        "[3] After Back (Esc / Alt-Left) — returned to list; FORWARD hint now offered\n\
         breadcrumb : {}\n    footer   : {}\n\
         active gcloud config before Back : {}\n\
         active gcloud config after  Back : {}\n\
         => configuration unchanged by history navigation: {}\n\n{}\n",
        breadcrumb_of(&session),
        footer_hints_of(&session),
        config_before,
        config_after,
        config_before == config_after,
        frame_box("rendered TUI (160x30):", 160, &after_back),
    ));

    // ---------------------------------------------------------------------
    // Alt-Right == NavAction::Forward — traverse forward back into the detail.
    // ---------------------------------------------------------------------
    session
        .dispatch_action(Action::Navigation(NavAction::Forward))
        .await;
    let after_forward = render_lines(&session, 160, 30);
    report.push_str(&format!(
        "[4] After Forward (Alt-Right) — detail restored by resource identity\n\
         breadcrumb : {}\n    footer   : {}\n\n{}\n",
        breadcrumb_of(&session),
        footer_hints_of(&session),
        frame_box("rendered TUI (160x30):", 160, &after_forward),
    ));

    // ---------------------------------------------------------------------
    // NARROW terminal (80x30): the list view stays full-screen — no preview
    // pane (below WIDE_LAYOUT_MIN_WIDTH=100).
    // ---------------------------------------------------------------------
    let mut narrow = RuntimeSession::new_for_test(AppState::new());
    load_dev_instances(&mut narrow).await;
    narrow
        .dispatch_action(Action::Workspace(WorkspaceAction::Resources))
        .await;
    let narrow_list = render_lines(&narrow, 80, 30);
    report.push_str(&format!(
        "[5] NARROW (80 cols) list view — full-screen, NO preview pane (below split threshold)\n\
         breadcrumb : {}\n\n{}\n",
        breadcrumb_of(&narrow),
        frame_box("rendered TUI (80x30):", 80, &narrow_list),
    ));

    narrow
        .dispatch_action(Action::Resources(ResourcesAction::OpenSelected))
        .await;
    let narrow_detail = render_lines(&narrow, 80, 30);
    report.push_str(&format!(
        "[6] NARROW detail view after Enter — full-screen drill-down (list hidden)\n\
         breadcrumb : {}\n\n{}\n",
        breadcrumb_of(&narrow),
        frame_box("rendered TUI (80x30):", 80, &narrow_detail),
    ));

    // Behavioural assertions mirroring the intent's acceptance criteria.
    assert!(
        wide_list.iter().any(|l| l.contains("Home > Resources")),
        "breadcrumb path must render in the shared frame"
    );
    assert!(
        wide_list.iter().any(|l| l.contains("Machine Type")),
        "wide list must show the summary preview pane"
    );
    assert!(
        !narrow_list.iter().any(|l| l.contains("Machine Type")),
        "narrow list must NOT show the preview pane"
    );
    assert_eq!(
        config_before, config_after,
        "history navigation (Back) must not change the active gcloud configuration"
    );
    assert!(
        hints_after_back.contains("Forward"),
        "a forward hint must appear after navigating Back, got {hints_after_back:?}"
    );

    println!("\n{report}");

    if let Ok(dir) = std::env::var("NUVEM_EVIDENCE_DIR") {
        let dir = PathBuf::from(dir);
        fs::create_dir_all(&dir).expect("evidence dir");
        fs::write(dir.join("nav_history_render.txt"), &report).expect("write evidence file");
    }
}
