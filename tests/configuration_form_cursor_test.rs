//! The configuration forms mark where typing lands.
//!
//! They were the only text inputs in the app that showed no insertion point.
//! Every other one carries a trailing `_`: the resource filter (`Filter: dev_`),
//! the configurations filter, the service-navigator filter, the context
//! picker's filter, and the command palette's query row.
//!
//! Two things follow from having no cursor, and the second is why this is not
//! cosmetic: an empty focused field is indistinguishable from a label, and
//! trailing whitespace is invisible. These values become a gcloud configuration
//! name and a project id, and the form is the last place a stray trailing space
//! can be seen before it is submitted.

use nuvem::app::{Action, AppState, ConfigurationsAction, Workspace};
use nuvem::services::runtime::session::RuntimeSession;
use nuvem_domain::gcp::configuration::{GcloudConfiguration, GcloudConfigurationName};
use ratatui::{Terminal, backend::TestBackend};

async fn session_in_configurations() -> RuntimeSession {
    let mut app = AppState::new();
    app.root.workspace = Workspace::Configurations;
    let mut session = RuntimeSession::new_for_test(app);
    session
        .apply_effect_result(nuvem::app::EffectResult::ConfigurationsLoaded {
            configurations: vec![GcloudConfiguration {
                name: GcloudConfigurationName::new("dev"),
                is_active: true,
                account: Some("dev@example.com".to_string()),
                project: Some("dev-project".to_string()),
                region: Some("europe-west1".to_string()),
                zone: Some("europe-west1-b".to_string()),
            }],
        })
        .await;
    session
}

fn lines(session: &RuntimeSession) -> Vec<String> {
    let mut terminal = Terminal::new(TestBackend::new(120, 24)).expect("terminal");
    terminal
        .draw(|frame| nuvem::ui::render(frame, session))
        .expect("render");
    let buffer = terminal.backend().buffer();
    (0..buffer.area.height)
        .map(|y| {
            (0..buffer.area.width)
                .map(|x| buffer.cell((x, y)).unwrap().symbol().to_string())
                .collect::<String>()
                .trim_end()
                .to_string()
        })
        .collect()
}

/// A field row: the focus marker, the label, and whatever follows the colon.
fn field_rows(rendered: &[String]) -> Vec<String> {
    rendered
        .iter()
        .filter(|line| line.contains(american_colon()) && line.starts_with('│'))
        .map(|line| line.trim_matches('│').trim_end().to_string())
        .collect()
}

fn american_colon() -> &'static str {
    ": "
}

#[tokio::test]
async fn the_focused_create_field_carries_a_cursor_and_the_others_do_not() {
    let mut session = session_in_configurations().await;
    session
        .dispatch_action(Action::Configurations(ConfigurationsAction::StartCreate))
        .await;
    for ch in "stag".chars() {
        session
            .dispatch_action(Action::Configurations(
                ConfigurationsAction::AppendCreateChar(ch),
            ))
            .await;
    }

    let rendered = lines(&session);
    let rows = field_rows(&rendered);
    assert!(!rows.is_empty(), "no field rows painted: {rendered:#?}");

    let mut focused = 0;
    for row in &rows {
        let trimmed = row.trim_start();
        if trimmed.starts_with("> ") {
            focused += 1;
            assert!(
                row.ends_with('_'),
                "the focused field has no cursor: {row:?}"
            );
        } else {
            assert!(
                !row.ends_with('_'),
                "an unfocused field claims a cursor: {row:?}"
            );
        }
    }
    assert_eq!(focused, 1, "exactly one field is focused: {rows:?}");
}

/// The case the marker is load-bearing for.
#[tokio::test]
async fn a_trailing_space_in_a_field_is_visible() {
    let mut session = session_in_configurations().await;
    session
        .dispatch_action(Action::Configurations(ConfigurationsAction::StartCreate))
        .await;
    for ch in "prod ".chars() {
        session
            .dispatch_action(Action::Configurations(
                ConfigurationsAction::AppendCreateChar(ch),
            ))
            .await;
    }
    let with_space = field_rows(&lines(&session));

    let mut session = session_in_configurations().await;
    session
        .dispatch_action(Action::Configurations(ConfigurationsAction::StartCreate))
        .await;
    for ch in "prod".chars() {
        session
            .dispatch_action(Action::Configurations(
                ConfigurationsAction::AppendCreateChar(ch),
            ))
            .await;
    }
    let without = field_rows(&lines(&session));

    assert_ne!(
        with_space, without,
        "a trailing space renders identically to no trailing space, which is \
         what the cursor is there to prevent"
    );
}

/// Tab takes the cursor with it.
#[tokio::test]
async fn the_cursor_follows_focus_between_fields() {
    let mut session = session_in_configurations().await;
    session
        .dispatch_action(Action::Configurations(ConfigurationsAction::StartCreate))
        .await;
    let first = field_rows(&lines(&session));
    session
        .dispatch_action(Action::Configurations(
            ConfigurationsAction::FocusNextCreateField,
        ))
        .await;
    let second = field_rows(&lines(&session));

    let cursor_row = |rows: &[String]| -> usize {
        rows.iter()
            .position(|row| row.ends_with('_'))
            .unwrap_or_else(|| panic!("no field carries the cursor: {rows:?}"))
    };
    assert_ne!(
        cursor_row(&first),
        cursor_row(&second),
        "Tab moved focus but left the cursor behind"
    );
}

#[tokio::test]
async fn the_focused_context_edit_field_carries_a_cursor() {
    let mut session = session_in_configurations().await;
    session
        .dispatch_action(Action::Configurations(
            ConfigurationsAction::StartEditActiveContext,
        ))
        .await;

    let rendered = lines(&session);
    let rows = field_rows(&rendered);
    let focused: Vec<&String> = rows
        .iter()
        .filter(|row| row.trim_start().starts_with("> "))
        .collect();
    assert_eq!(focused.len(), 1, "exactly one field is focused: {rows:?}");
    assert!(
        focused[0].ends_with('_'),
        "the focused field has no cursor: {:?}",
        focused[0]
    );
}
