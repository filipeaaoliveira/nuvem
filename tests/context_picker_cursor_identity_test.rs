//! The configuration context picker's cursor must stay on the *item* the
//! operator is looking at across a filter change, not on the *row index* it
//! happened to occupy.
//!
//! `ConfigurationContextPicker` stores its selection as an index into the
//! ordered visible view (`selected_visible`). Any filter change that removes a
//! row ABOVE the cursor shifts every row below it up by one, so an in-bounds
//! index silently points at a different item than the operator selected.
//! Favorites make this fire routinely rather than rarely: a favorite is by
//! construction hoisted above the cursor, so "a row above the cursor
//! disappeared" is the ordinary case, not an edge case.
//!
//! Covered from the reducer down, plus one render assertion — the caret the
//! operator actually sees is what the bug is about, so at least one case is
//! pinned on painted output rather than on state alone.

use nuvem::app::reducer::root::reduce;
use nuvem::app::state::configurations::{
    ConfigurationContextPickerItem, ConfigurationContextPickerKind,
};
use nuvem::app::{Action, AppState, ConfigurationsAction, EffectResult};
use nuvem::config::favorites::FavoriteScopes;
use nuvem::config::problems::ConfigProblems;
use nuvem::services::runtime::session::RuntimeSession;
use nuvem_domain::gcp::configuration::{GcloudConfiguration, GcloudConfigurationName};
use nuvem_domain::gcp::project::GcloudProject;
use ratatui::{Terminal, backend::TestBackend};

fn config(project: &str) -> GcloudConfiguration {
    GcloudConfiguration {
        name: GcloudConfigurationName::new("dev"),
        is_active: true,
        account: Some("dev@example.com".to_string()),
        project: Some(project.to_string()),
        region: None,
        zone: None,
    }
}

fn project(project_id: &str, name: &str) -> ConfigurationContextPickerItem {
    ConfigurationContextPickerItem::Project(GcloudProject {
        project_id: project_id.to_string(),
        name: name.to_string(),
        lifecycle_state: Some("ACTIVE".to_string()),
    })
}

fn favorites(project_ids: &[&str]) -> FavoriteScopes {
    let mut table = toml::value::Table::new();
    table.insert(
        "projects".to_string(),
        toml::Value::Array(
            project_ids
                .iter()
                .map(|id| toml::Value::String(id.to_string()))
                .collect(),
        ),
    );
    let mut problems = ConfigProblems::default();
    let scopes = FavoriteScopes::from_file(&toml::Value::Table(table), &mut problems);
    assert!(problems.is_empty(), "fixture favorites must be valid");
    scopes
}

/// Opens the project picker with `active_project` as the configuration's
/// current project — which is what the picker resolves `initial_value`
/// against, so it is also where the cursor starts.
fn app_with_picker(
    active_project: &str,
    favorite_ids: &[&str],
    items: Vec<ConfigurationContextPickerItem>,
) -> AppState {
    let mut app = AppState::new();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ConfigurationsLoaded {
            configurations: vec![config(active_project)],
        }),
    );
    app.root
        .configurations
        .set_favorites(favorites(favorite_ids));
    reduce(
        &mut app,
        &Action::Configurations(ConfigurationsAction::StartEditActiveContext),
    );
    reduce(
        &mut app,
        &Action::Configurations(ConfigurationsAction::OpenContextPicker),
    );
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ContextPickerLoaded {
            configuration: GcloudConfigurationName::new("dev"),
            kind: ConfigurationContextPickerKind::Project,
            items,
        }),
    );
    app
}

fn type_filter(app: &mut AppState, value: &str) {
    reduce(
        app,
        &Action::Configurations(ConfigurationsAction::StartContextPickerFilter),
    );
    reduce(
        app,
        &Action::Configurations(ConfigurationsAction::UpdateContextPickerFilter(
            value.to_string(),
        )),
    );
}

fn selected(app: &AppState) -> Option<&ConfigurationContextPickerItem> {
    app.root
        .configurations
        .context_picker()
        .expect("picker is open")
        .selected_item()
}

fn render(app: AppState) -> Vec<String> {
    let session = RuntimeSession::new_for_test(app);
    let mut terminal = Terminal::new(TestBackend::new(100, 24)).unwrap();
    terminal
        .draw(|frame| nuvem::ui::render(frame, &session))
        .unwrap();
    let buffer = terminal.backend().buffer();
    (0..buffer.area.height)
        .map(|y| {
            (0..buffer.area.width)
                .map(|x| buffer.cell((x, y)).unwrap().symbol().to_string())
                .collect::<String>()
        })
        .collect()
}

/// The ticket's probe. Favorited `zulu-project` is hoisted to the top, so the
/// cursor on `node-one` sits at visible index 1. Typing `node` drops the
/// favorite — a row strictly above the cursor — and `node-one` becomes index
/// 0. The cursor must follow `node-one`, not stay at index 1 (which is now
/// `node-two`).
#[test]
fn a_filter_that_removes_a_hoisted_favorite_keeps_the_cursor_on_the_same_item() {
    let mut app = app_with_picker(
        "node-one",
        &["zulu-project"],
        vec![
            project("node-one", "Node One"),
            project("node-two", "Node Two"),
            project("zulu-project", "Zulu Corp"),
        ],
    );
    assert_eq!(
        selected(&app),
        Some(&project("node-one", "Node One")),
        "cursor should open on the configuration's current project"
    );

    type_filter(&mut app, "node");

    assert_eq!(
        selected(&app),
        Some(&project("node-one", "Node One")),
        "the cursor must stay on node-one after the favorite above it was filtered out"
    );
}

/// Same defect through the painted caret: the row the operator sees selected
/// after typing must still be `node-one`. State alone could agree while the
/// overlay drew the caret elsewhere, and the caret is what the operator acts
/// on.
#[test]
fn the_painted_caret_stays_on_the_same_row_across_a_filter_change() {
    let mut app = app_with_picker(
        "node-one",
        &["zulu-project"],
        vec![
            project("node-one", "Node One"),
            project("node-two", "Node Two"),
            project("zulu-project", "Zulu Corp"),
        ],
    );
    type_filter(&mut app, "node");
    let rendered = render(app);

    let caret_line = rendered
        .iter()
        .find(|line| line.contains('>') && line.contains("node-"))
        .expect("a picker row should be painted with the selection caret");
    assert!(
        caret_line.contains("node-one"),
        "caret should be on the node-one row, got {caret_line:?} in:\n{}",
        rendered.join("\n")
    );
    let two_line = rendered
        .iter()
        .find(|line| line.contains("node-two"))
        .expect("node-two should still be painted");
    assert!(
        !two_line.contains('>'),
        "node-two must not carry the caret: {two_line:?}"
    );
}

/// The defect is not favorites-specific: removing any row above the cursor
/// shifts it. No favorites here at all — `alpha-one` is dropped by the filter
/// while the cursor sits on `bravo-two`.
#[test]
fn a_filter_that_removes_a_plain_row_above_the_cursor_keeps_the_cursor() {
    let mut app = app_with_picker(
        "bravo-two",
        &[],
        vec![
            project("alpha-one", "Alpha One"),
            project("bravo-two", "Bravo Two"),
            project("bravo-three", "Bravo Three"),
        ],
    );
    assert_eq!(selected(&app), Some(&project("bravo-two", "Bravo Two")));

    type_filter(&mut app, "bravo");

    assert_eq!(
        selected(&app),
        Some(&project("bravo-two", "Bravo Two")),
        "the cursor must stay on bravo-two after alpha-one was filtered out"
    );
}

/// Cancelling a filter widens the visible view back out, re-inserting rows
/// above the cursor. That is the same shift in the other direction and must
/// also keep the cursor on its item.
#[test]
fn cancelling_a_filter_keeps_the_cursor_on_the_same_item() {
    let mut app = app_with_picker(
        "node-one",
        &["zulu-project"],
        vec![
            project("node-one", "Node One"),
            project("node-two", "Node Two"),
            project("zulu-project", "Zulu Corp"),
        ],
    );
    type_filter(&mut app, "node");
    reduce(
        &mut app,
        &Action::Configurations(ConfigurationsAction::SelectNextContextPickerItem),
    );
    assert_eq!(
        selected(&app),
        Some(&project("node-two", "Node Two")),
        "fixture should leave the cursor on the second visible row"
    );

    reduce(
        &mut app,
        &Action::Configurations(ConfigurationsAction::CancelContextPickerFilter),
    );

    assert_eq!(
        selected(&app),
        Some(&project("node-two", "Node Two")),
        "the cursor must stay on node-two when the cancelled filter restores rows above it"
    );
}

/// When the filter removes the selected item itself there is no identity left
/// to follow, so the cursor falls back to the nearest in-bounds row. Pinned so
/// the fallback cannot regress into an out-of-bounds index (which reads as "no
/// selection" to the overlay).
#[test]
fn filtering_out_the_selected_item_leaves_the_cursor_in_bounds() {
    let mut app = app_with_picker(
        "zulu-project",
        &[],
        vec![
            project("node-one", "Node One"),
            project("node-two", "Node Two"),
            project("zulu-project", "Zulu Corp"),
        ],
    );
    assert_eq!(selected(&app), Some(&project("zulu-project", "Zulu Corp")));

    type_filter(&mut app, "node");

    let picker = app
        .root
        .configurations
        .context_picker()
        .expect("picker is open");
    assert_eq!(picker.visible_items().len(), 2);
    assert!(
        picker.selected_item().is_some(),
        "a visible row must stay selected after the selected item was filtered out"
    );
}
