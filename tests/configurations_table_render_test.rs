//! Rendered-frame tests for the Configurations workspace table.
//!
//! The workspace used to build each row with `format!` and a two-space join, so
//! nothing lined up and there was no header: the one screen whose whole job is
//! comparing accounts and projects across configurations was the only list in
//! the app that was not a column table.
//!
//! Every assertion here reads the painted buffer. Alignment is a property of
//! what reaches the screen, and a projection-level check would pass on a model
//! the renderer then lays out ragged.

use nuvem::app::{AppState, EffectResult, Workspace};
use nuvem::services::runtime::session::RuntimeSession;
use nuvem_domain::gcp::configuration::{GcloudConfiguration, GcloudConfigurationName};
use ratatui::{Terminal, backend::TestBackend};

fn configuration(
    name: &str,
    is_active: bool,
    account: &str,
    project: &str,
    region: Option<&str>,
    zone: Option<&str>,
) -> GcloudConfiguration {
    GcloudConfiguration {
        name: GcloudConfigurationName::new(name),
        is_active,
        account: Some(account.to_string()),
        project: Some(project.to_string()),
        region: region.map(ToString::to_string),
        zone: zone.map(ToString::to_string),
    }
}

/// Names of deliberately different lengths: a ragged layout is invisible when
/// every name happens to be the same width.
async fn session_with_configurations() -> RuntimeSession {
    let mut app = AppState::new();
    app.root.workspace = Workspace::Configurations;
    let mut session = RuntimeSession::new_for_test(app);
    session
        .apply_effect_result(EffectResult::ConfigurationsLoaded {
            configurations: vec![
                configuration(
                    "dev",
                    true,
                    "dev@example.com",
                    "dev-project",
                    Some("europe-west1"),
                    Some("europe-west1-b"),
                ),
                configuration(
                    "staging-eu-west",
                    false,
                    "staging-deployer@example.com",
                    "acme-staging-eu",
                    Some("europe-west4"),
                    None,
                ),
                configuration("prod", false, "ops@example.com", "prod-project", None, None),
            ],
        })
        .await;
    session
}

/// Only the rows inside the table's own block.
///
/// The app frame's context header also names the active account and project, so
/// a search over the whole screen finds that line first and asserts nothing
/// about the table.
fn table_rows(session: &RuntimeSession, width: u16, height: u16) -> Vec<String> {
    lines(session, width, height)
        .into_iter()
        .filter(|line| line.starts_with('│'))
        .collect()
}

fn lines(session: &RuntimeSession, width: u16, height: u16) -> Vec<String> {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("terminal");
    terminal
        .draw(|frame| nuvem::ui::render(frame, session))
        .expect("render");
    let buffer = terminal.backend().buffer();
    (0..buffer.area.height)
        .map(|y| {
            (0..buffer.area.width)
                .map(|x| buffer.cell((x, y)).unwrap().symbol().to_string())
                .collect::<String>()
        })
        .collect()
}

/// Where `needle` starts, counted in characters: the rows are bounded by
/// box-drawing glyphs, which are three bytes each, so a byte offset used as a
/// column number lands mid-row.
fn column_of(haystack: &str, needle: &str) -> usize {
    let byte = haystack
        .find(needle)
        .unwrap_or_else(|| panic!("{needle:?} is not on this row: {haystack:?}"));
    haystack[..byte].chars().count()
}

#[tokio::test]
async fn the_configurations_table_has_a_header_naming_every_field() {
    let session = session_with_configurations().await;
    let rendered = lines(&session, 120, 24).join("\n");

    for header in ["NAME", "ACCOUNT", "PROJECT", "REGION", "ZONE"] {
        assert!(
            rendered.contains(header),
            "the table has no {header} header; rendered:\n{rendered}"
        );
    }
}

/// The point of the workspace: reading one field down the rows.
#[tokio::test]
async fn every_configuration_row_starts_its_columns_at_one_column() {
    let session = session_with_configurations().await;
    let rendered = table_rows(&session, 120, 24);

    let header = rendered
        .iter()
        .find(|line| line.contains("ACCOUNT"))
        .expect("the header row must be painted");

    // Every value under the header it belongs to. The three accounts are the
    // claim proper: they are of three different lengths, so a ragged layout
    // cannot put all three in one column by accident.
    let expected: [(&str, &[&str]); 4] = [
        (
            "ACCOUNT",
            &[
                "dev@example.com",
                "staging-deployer@example.com",
                "ops@example.com",
            ],
        ),
        (
            "PROJECT",
            &["dev-project", "acme-staging-eu", "prod-project"],
        ),
        ("REGION", &["europe-west1", "europe-west4"]),
        ("ZONE", &["europe-west1-b"]),
    ];

    for (header_needle, values) in expected {
        let column = column_of(header, header_needle);
        for value in values {
            let row = rendered
                .iter()
                .find(|line| line.contains(value))
                .unwrap_or_else(|| panic!("no row carries {value:?}"));
            assert_eq!(
                column_of(row, value),
                column,
                "{value:?} does not start under its {header_needle} header; \
                 row: {row:?}, header: {header:?}"
            );
        }
    }
}

/// An absent region or zone reads as the app's absent-value dash, not as a gap
/// the eye has to attribute to the right column.
#[tokio::test]
async fn an_absent_context_value_renders_as_the_missing_marker() {
    let session = session_with_configurations().await;
    let rendered = table_rows(&session, 120, 24);

    let header = rendered
        .iter()
        .find(|line| line.contains("REGION"))
        .expect("the header row must be painted");
    let prod = rendered
        .iter()
        .find(|line| line.contains("prod-project"))
        .expect("the prod row must be painted");

    // Read at the column, not with `contains`: every row on this screen carries
    // a hyphen inside some hostname or project id, so a whole-row search would
    // pass on a blank cell.
    for column_header in ["REGION", "ZONE"] {
        let column = column_of(header, column_header);
        let cell: char = prod
            .chars()
            .nth(column)
            .unwrap_or_else(|| panic!("the {column_header} column is off the row: {prod:?}"));
        assert_eq!(
            cell, '-',
            "prod has no {column_header}, so that cell must carry the missing \
             marker; row: {prod:?}, header: {header:?}"
        );
    }
}

/// The active configuration and the cursor stay distinguishable without colour.
#[tokio::test]
async fn the_active_configuration_and_the_cursor_are_both_marked() {
    let session = session_with_configurations().await;
    let rendered = table_rows(&session, 120, 24);

    let dev = rendered
        .iter()
        .find(|line| line.contains("dev@example.com"))
        .expect("the dev row must be painted");
    assert!(
        dev.contains('*'),
        "dev is the active configuration and must be marked as one: {dev:?}"
    );
    assert!(
        dev.contains('>'),
        "dev is the selected row and must carry the cursor: {dev:?}"
    );

    let prod = rendered
        .iter()
        .find(|line| line.contains("ops@example.com"))
        .expect("the prod row must be painted");
    assert!(
        !prod.contains('*'),
        "prod is not active and must not be marked as one: {prod:?}"
    );
}
