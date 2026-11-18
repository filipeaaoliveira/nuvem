//! Evidence generator — the Cloud SQL log jump's refusal path.
//!
//! The rest of the ticket is demonstrable against the shipped binary in
//! `--demo` mode, whose fixture context always carries a project. The one
//! user-visible surface that context can never reach is the *refusal*: a
//! Cloud SQL log entry is keyed by a composite `<project>:<instance>`
//! database id, so with no project in the active context the jump must say so
//! instead of emitting a filter that silently matches nothing.
//!
//! Both halves of that decision are driven here through the real keymap and
//! the real reducer and rendered through the real `nuvem::ui::render`, and the
//! resulting terminal is written to the evidence directory as an ASCII
//! screenshot when `NUVEM_EVIDENCE_DIR` is set (the assertions always run).

use std::fs;
use std::path::PathBuf;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use nuvem::app::state::command::CommandDescriptor;
use nuvem::app::state::root::Workspace;
use nuvem::app::{Action, AppState, CommandAction, EffectResult, ResourcesAction, WorkspaceAction};
use nuvem::input::keymap::action_for_key;
use nuvem::services::runtime::input_context_for_test;
use nuvem::services::runtime::session::RuntimeSession;
use nuvem_domain::gcp::configuration::{GcloudConfiguration, GcloudConfigurationName};
use nuvem_domain::gcp::resource::{
    CloudSqlAvailability, CloudSqlInstance, CloudSqlStatus, EndpointVisibility, ResourceServiceKind,
};
use ratatui::layout::Rect;
use ratatui::{Terminal, backend::TestBackend};

const WIDTH: u16 = 132;
const HEIGHT: u16 = 26;

fn evidence_dir() -> Option<PathBuf> {
    let dir = std::env::var("NUVEM_EVIDENCE_DIR").ok()?;
    let path = PathBuf::from(dir);
    fs::create_dir_all(&path).ok()?;
    Some(path)
}

fn write_frame(name: &str, caption: &str, lines: &[String]) {
    let Some(dir) = evidence_dir() else {
        return;
    };
    let width = lines.iter().map(|l| l.chars().count()).max().unwrap_or(0);
    let mut body = format!("{caption}\n\n+{}+\n", "-".repeat(width));
    for line in lines {
        let pad = width - line.chars().count();
        body.push_str(&format!("|{line}{}|\n", " ".repeat(pad)));
    }
    body.push_str(&format!("+{}+\n", "-".repeat(width)));
    fs::write(dir.join(format!("{name}.txt")), body).expect("write ascii frame");
}

fn render_lines(session: &RuntimeSession) -> Vec<String> {
    let mut terminal = Terminal::new(TestBackend::new(WIDTH, HEIGHT)).expect("terminal");
    terminal
        .draw(|frame| nuvem::ui::render(frame, session))
        .expect("render");
    let buffer = terminal.backend().buffer();
    (0..buffer.area.height)
        .map(|y| {
            (0..buffer.area.width)
                .map(|x| buffer.cell((x, y)).expect("cell").symbol().to_string())
                .collect::<String>()
                .trim_end()
                .to_string()
        })
        .collect()
}

fn instance(name: &str) -> CloudSqlInstance {
    CloudSqlInstance {
        name: name.to_string(),
        database_version: "POSTGRES_15".to_string(),
        engine: "PostgreSQL".to_string(),
        region: "us-central1".to_string(),
        tier: "db-custom-2-7680".to_string(),
        availability: CloudSqlAvailability::Zonal,
        status: CloudSqlStatus::Runnable,
        connectivity: EndpointVisibility::Public,
    }
}

/// A Cloud SQL list under an active configuration that has **no** project.
async fn session_without_project() -> RuntimeSession {
    let mut session = RuntimeSession::new_for_test(AppState::new());
    session
        .apply_effect_result(EffectResult::ConfigurationsLoaded {
            configurations: vec![GcloudConfiguration {
                name: GcloudConfigurationName::new("no-project"),
                is_active: true,
                account: Some("dev@example.com".to_string()),
                project: None,
                region: Some("us-central1".to_string()),
                zone: None,
            }],
        })
        .await;
    session
        .dispatch_action(Action::Workspace(WorkspaceAction::Resources))
        .await;
    session
        .dispatch_action(Action::Resources(ResourcesAction::SelectService(
            ResourceServiceKind::CloudSql,
        )))
        .await;
    let context = session
        .app()
        .root
        .current_context()
        .expect("a context resolves even without a project");
    session
        .apply_effect_result(EffectResult::CloudSqlInstancesLoaded {
            context,
            instances: vec![instance("orders-db")],
        })
        .await;
    session
}

/// `L` on a Cloud SQL instance with no project in the context: the status line
/// names the problem and the view does not move.
#[tokio::test]
async fn shift_l_refuses_cloud_sql_logs_without_a_project() {
    let mut session = session_without_project().await;

    let key = KeyEvent::new(KeyCode::Char('L'), KeyModifiers::SHIFT);
    let context = input_context_for_test(&session, Rect::new(0, 0, WIDTH, HEIGHT));
    let action = action_for_key(key, &context).expect("`L` is bound in the resource list");
    session.dispatch_action(action).await;

    let lines = render_lines(&session);
    let screen = lines.join("\n");
    assert_eq!(
        session.app().root.workspace,
        Workspace::Resources,
        "the refusal must leave the user where they were:\n{screen}"
    );
    assert!(
        screen.contains("Active gcloud context has no project; cannot scope Cloud SQL logs"),
        "the refusal must be on screen:\n{screen}"
    );
    write_frame(
        "29-cloudsql-L-refused-without-project",
        "29. `L` on a Cloud SQL instance when the active gcloud context has NO project. A \
Cloud SQL log entry is keyed by a composite <project>:<instance> database id, so a filter \
built without the project would match zero rows and read exactly like 'this instance has no \
logs'. The jump refuses loudly instead, and the Cloud SQL list stays on screen. (This is the \
one jump surface `--demo` cannot reach: its fixture context always has a project.)",
        &lines,
    );
}

/// The palette entry is gated on the same condition, so the command cannot
/// offer what the key refuses.
#[tokio::test]
async fn the_palette_hides_the_cloud_sql_log_jump_without_a_project() {
    let mut session = session_without_project().await;
    session
        .dispatch_action(Action::Command(CommandAction::Open))
        .await;
    for ch in "logs for selected".chars() {
        session
            .dispatch_action(Action::Command(CommandAction::AppendChar(ch)))
            .await;
    }

    let root = &session.app().root;
    let offered: Vec<CommandDescriptor> = root.command.visible_commands(root);
    let lines = render_lines(&session);
    let screen = lines.join("\n");
    assert!(
        !offered.contains(&CommandDescriptor::OpenLogsForSelected(
            ResourceServiceKind::CloudSql
        )),
        "the Cloud SQL log jump must not be offered without a project:\n{screen}"
    );
    assert!(
        !screen.contains("Open logs for selected Cloud SQL instance"),
        "and it must not be on screen either:\n{screen}"
    );
    write_frame(
        "30-palette-hides-cloudsql-log-jump-without-project",
        "30. The command palette, filtered on 'logs for selected' in the same project-less \
context: 'Open logs for selected Cloud SQL instance' is absent, so the palette cannot offer \
what the `L` key refuses.",
        &lines,
    );
}
