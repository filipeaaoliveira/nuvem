//! Render coverage for favorite marking in the configuration context picker
//! overlay (project/region/zone picker opened from context edit).
//!
//! Favorites are already sorted first by `ConfigurationContextPicker::
//! visible_items` (see `src/app/state/configurations.rs`); this file pins that
//! the *overlay* paints a visible difference for a favorite row, that the
//! marker sits in a fixed column so a favorite never shifts its own or its
//! neighbours' text, and that a row which is both selected and a favorite
//! shows the caret and the marker together, caret first.

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

fn config(name: &str) -> GcloudConfiguration {
    GcloudConfiguration {
        name: GcloudConfigurationName::new(name),
        is_active: true,
        account: Some("dev@example.com".to_string()),
        project: Some("seed-project".to_string()),
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

/// Opens the project picker on a fresh app, with `alpha-project` favorited
/// and `bravo-project` not. Both project ids and both display names are the
/// same length on purpose, so the two rows' natural text is the same width
/// before any marker is added — any difference the test later finds between
/// the two rows' text columns is the marker's doing, not a coincidence of the
/// fixture data.
fn app_with_project_picker() -> AppState {
    let mut app = AppState::new();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ConfigurationsLoaded {
            configurations: vec![config("dev")],
        }),
    );
    app.root
        .configurations
        .set_favorites(favorites(&["alpha-project", "charlie-project"]));
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
            items: vec![
                // Favorites-first ordering (see `visible_items`) puts alpha
                // and charlie ahead of bravo; `selected_visible` starts at 0,
                // so alpha is both the selection and a favorite, charlie is a
                // favorite that is *not* selected, and bravo is neither —
                // between them the three rows cover every caret/marker
                // combination this overlay can paint.
                project("alpha-project", "Alpha Corp"),
                project("bravo-project", "Bravo Corp"),
                project("charlie-project", "Charlie Corp"),
            ],
        }),
    );
    app
}

fn render(app: AppState) -> Vec<String> {
    let session = RuntimeSession::new_for_test(app);
    let mut terminal = Terminal::new(TestBackend::new(100, 24)).unwrap();
    terminal
        .draw(|frame| nuvem::ui::render(frame, &session))
        .unwrap();
    lines(&terminal)
}

/// The needle's position as a *character* column, not a byte offset — the
/// overlay's box-drawing border characters are multi-byte in UTF-8, so a
/// byte offset would drift out of step with `line.chars()` as soon as one of
/// those appears earlier in the line.
fn text_column(line: &str, needle: &str) -> usize {
    let byte_at = line
        .find(needle)
        .unwrap_or_else(|| panic!("expected {needle:?} in line {line:?}"));
    line[..byte_at].chars().count()
}

/// A favorite row and a non-favorite row must differ by more than just their
/// text: the overlay has to paint something that says "this one is
/// favorited" the operator can see without knowing the sort order pins
/// favorites first. Pinned at the marker column itself (not just "the two
/// lines differ somewhere", which the two different project ids would give
/// for free even with no marker at all).
///
/// Compares two rows that are both unselected — `charlie-project` (a
/// favorite) against `bravo-project` (not) — so the selection caret cannot
/// be mistaken for the favorite marker this test is actually pinning.
#[test]
fn a_favorite_row_is_visually_marked_and_a_non_favorite_row_is_not() {
    let app = app_with_project_picker();
    let rendered = render(app).join("\n");

    let favorite_line = rendered
        .lines()
        .find(|line| line.contains("charlie-project"))
        .expect("unselected favorite row should be painted");
    let non_favorite_line = rendered
        .lines()
        .find(|line| line.contains("bravo-project"))
        .expect("non-favorite row should be painted");
    assert!(
        !favorite_line.contains('>') && !non_favorite_line.contains('>'),
        "both rows compared here must be unselected, or the caret would be \
mistaken for the favorite marker:\ncharlie: {favorite_line:?}\nbravo:   {non_favorite_line:?}"
    );

    let favorite_col = text_column(favorite_line, "charlie-project");
    let non_favorite_col = text_column(non_favorite_line, "bravo-project");

    let favorite_chars: Vec<char> = favorite_line.chars().collect();
    let non_favorite_chars: Vec<char> = non_favorite_line.chars().collect();
    let favorite_marker = favorite_chars[favorite_col - 2];
    let non_favorite_marker = non_favorite_chars[non_favorite_col - 2];

    assert_ne!(
        favorite_marker, non_favorite_marker,
        "the marker column should differ between a favorite row ({favorite_line:?}) \
and a non-favorite row ({non_favorite_line:?})"
    );
    assert_eq!(
        non_favorite_marker, ' ',
        "a non-favorite row's marker column should be blank: {non_favorite_line:?}"
    );
}

/// The marker occupies a fixed column on every row: a favorite's own text
/// must start at the same screen column as a non-favorite's text, not one
/// column further right because a marker glyph pushed it over.
#[test]
fn the_marker_column_does_not_shift_either_rows_text() {
    let app = app_with_project_picker();
    let rendered = render(app).join("\n");

    let favorite_line = rendered
        .lines()
        .find(|line| line.contains("alpha-project"))
        .expect("favorite row should be painted");
    let non_favorite_line = rendered
        .lines()
        .find(|line| line.contains("bravo-project"))
        .expect("non-favorite row should be painted");

    let favorite_col = text_column(favorite_line, "alpha-project");
    let non_favorite_col = text_column(non_favorite_line, "bravo-project");

    assert_eq!(
        favorite_col, non_favorite_col,
        "favorite row text column {favorite_col} should match non-favorite row text column {non_favorite_col}:\n{rendered}"
    );

    // Same starting column and (per the fixture) the same natural text
    // length, so the painted spans read out to the same width too. Worked
    // out over `chars()`, not byte offsets — the overlay's box-drawing
    // border is multi-byte in UTF-8 and would otherwise throw a naive byte
    // slice off a char boundary.
    let favorite_chars: Vec<char> = favorite_line.chars().collect();
    let non_favorite_chars: Vec<char> = non_favorite_line.chars().collect();
    let favorite_tail: String = favorite_chars[favorite_col..].iter().collect();
    let non_favorite_tail: String = non_favorite_chars[non_favorite_col..].iter().collect();
    let favorite_span = favorite_tail
        .find("  ACTIVE")
        .map(|i| favorite_tail[..i].chars().count() + "  ACTIVE".chars().count())
        .expect("favorite row should carry its full line, including status");
    let non_favorite_span = non_favorite_tail
        .find("  ACTIVE")
        .map(|i| non_favorite_tail[..i].chars().count() + "  ACTIVE".chars().count())
        .expect("non-favorite row should carry its full line, including status");
    assert_eq!(
        favorite_span, non_favorite_span,
        "both rows' text spans should render to the same width:\n{rendered}"
    );
}

/// The selection caret must still work once a marker column exists, and a
/// row that is both the current selection and a favorite must show both, in
/// the documented order: caret, then favorite marker, then row text.
#[test]
fn a_row_that_is_both_selected_and_favorite_shows_the_caret_then_the_marker() {
    let app = app_with_project_picker();
    let rendered = render(app).join("\n");

    // alpha-project sorts first (favorites-first) and starts selected.
    let favorite_line = rendered
        .lines()
        .find(|line| line.contains("alpha-project"))
        .expect("favorite row should be painted");

    let favorite_chars: Vec<char> = favorite_line.chars().collect();
    let caret_at = favorite_chars
        .iter()
        .position(|c| *c == '>')
        .expect("caret should be painted");
    let text_at = text_column(favorite_line, "alpha-project");
    let between: String = favorite_chars[caret_at + 1..text_at].iter().collect();

    assert_eq!(
        between.trim(),
        "*",
        "expected exactly the favorite marker between the caret and the row text, got {between:?} in {favorite_line:?}"
    );
    assert!(
        caret_at < text_at,
        "caret must precede the favorite marker and the row text"
    );
}
