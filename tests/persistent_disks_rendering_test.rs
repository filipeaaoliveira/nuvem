//! Persistent disks reach the screen, and the orphan badge is styled as a
//! warning when it is there and absent when it is not.
//!
//! A service can be in `SERVICE_REGISTRY`, satisfy every `ResourceServiceKind`
//! walk, hold rows in state, and still never paint one. So these assertions are
//! made on the terminal buffer, through the real reducer and the real renderer,
//! not on the projection model — except the badge's *semantic*, which is a
//! styling fact the buffer's symbols cannot carry and which is therefore
//! asserted on the cell it is attached to.

use nuvem::app::projection::project_resources;
use nuvem::app::projection::table::{Cell, CellKind, StatusSemantic};
use nuvem::app::reducer::root::reduce;
use nuvem::app::state::root::Workspace;
use nuvem::app::{Action, AppError, AppState, EffectResult, ResourcesAction};
use nuvem::services::runtime::session::RuntimeSession;
use nuvem_domain::gcp::configuration::{GcloudConfiguration, GcloudConfigurationName};
use nuvem_domain::gcp::resource::{DiskStatus, PersistentDisk, ResourceServiceKind};
use ratatui::Terminal;
use ratatui::backend::TestBackend;

const FRAME_WIDTH: u16 = 200;
const FRAME_HEIGHT: u16 = 40;

fn configuration() -> GcloudConfiguration {
    GcloudConfiguration {
        name: GcloudConfigurationName::new("dev"),
        is_active: true,
        account: Some("dev@example.com".to_string()),
        project: Some("dev-project".to_string()),
        region: None,
        zone: None,
    }
}

fn disk(name: &str, size_gb: u64, status: DiskStatus, users: &[&str]) -> PersistentDisk {
    PersistentDisk {
        name: name.to_string(),
        location: "europe-west1-b".to_string(),
        status,
        size_gb,
        disk_type: "pd-ssd".to_string(),
        users: users.iter().map(|user| user.to_string()).collect(),
        labels: Vec::new(),
    }
}

fn disks() -> Vec<PersistentDisk> {
    vec![
        disk("attached-boot", 200, DiskStatus::Ready, &["web-01"]),
        disk("detached-legacy", 20, DiskStatus::Ready, &[]),
        disk("being-built", 50, DiskStatus::Creating, &[]),
    ]
}

/// The disks pane on screen with a context resolved and nothing loaded yet.
fn app_with_context() -> AppState {
    let mut app = AppState::new();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ConfigurationsLoaded {
            configurations: vec![configuration()],
        }),
    );
    reduce(
        &mut app,
        &Action::Resources(ResourcesAction::SelectService(
            ResourceServiceKind::PersistentDisks,
        )),
    );
    app.root.workspace = Workspace::Resources;
    app
}

fn app_showing_disks() -> AppState {
    let mut app = app_with_context();
    let context = app.root.current_context().expect("active context");
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::PersistentDisksLoaded {
            context,
            disks: disks(),
        }),
    );
    app
}

fn painted(app: AppState) -> String {
    let session = RuntimeSession::new_for_test(app);
    let backend = TestBackend::new(FRAME_WIDTH, FRAME_HEIGHT);
    let mut terminal = Terminal::new(backend).expect("test terminal");
    terminal
        .draw(|frame| nuvem::ui::render(frame, &session))
        .expect("render");
    let buffer = terminal.backend().buffer();
    (0..buffer.area.height)
        .map(|y| {
            let mut line = String::new();
            for x in 0..buffer.area.width {
                line.push_str(buffer.cell((x, y)).expect("cell").symbol());
            }
            line.trim_end().to_string()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// The rows of the table pane alone.
///
/// The navigator, the table and the preview are painted side by side, so one
/// buffer line carries a slice of all three — and the preview shows the
/// selected disk's own name, location and status. A `contains` over the whole
/// line therefore passes on a table that paints nothing at all, which is the
/// failure these tests exist to catch.
fn table_lines(painted: &str) -> Vec<&str> {
    painted
        .lines()
        .filter_map(|line| line.split("││").nth(1))
        .collect()
}

/// The `orphan` cell of the projected row for `name`, read out of the real
/// table model rather than by calling a `ColumnDef` closure directly — so the
/// column has to be in the catalog, and paired with the header it claims, for
/// this to find anything.
fn orphan_cell(app: &AppState, name: &str) -> Cell {
    let model = project_resources(&app.root.resources)
        .table
        .expect("list mode builds a table model");
    let orphan_at = model
        .columns
        .iter()
        .position(|column| column.key == "orphan")
        .expect("the disks table has an orphan column");
    let name_at = model
        .columns
        .iter()
        .position(|column| column.key == "name")
        .expect("the disks table has a name column");
    model
        .rows
        .iter()
        .find(|row| row.cells[name_at].text == name)
        .unwrap_or_else(|| panic!("no projected row named {name}"))
        .cells[orphan_at]
        .clone()
}

#[test]
fn the_disks_table_paints_a_row_per_disk_with_its_size_and_attachment() {
    let painted = painted(app_showing_disks());

    assert!(
        painted.contains("Resources / Persistent disks"),
        "the pane must name the service it is showing:\n{painted}"
    );
    for header in ["NAME", "ORPHAN", "SIZE", "ATTACHED"] {
        assert!(painted.contains(header), "no {header} header:\n{painted}");
    }

    // Name and size on the *same* row of the table pane: a name in the table
    // and a size somewhere else on screen would satisfy two `contains` checks
    // while the row itself carried no size at all.
    for (name, size) in [
        ("attached-boot", "200 GB"),
        ("detached-legacy", "20 GB"),
        ("being-built", "50 GB"),
    ] {
        assert!(
            table_lines(&painted)
                .iter()
                .any(|line| line.contains(name) && line.contains(size)),
            "no table row carries {name} with {size}:\n{painted}"
        );
    }

    // The attachment is on the attached disk's row and on no other.
    let lines = table_lines(&painted);
    let rows_with_user: Vec<&&str> = lines
        .iter()
        .filter(|line| line.contains("web-01"))
        .collect();
    assert_eq!(
        rows_with_user.len(),
        1,
        "exactly one row is attached to web-01:\n{painted}"
    );
    assert!(rows_with_user[0].contains("attached-boot"));
}

/// The badge is painted on the orphan and on nothing else.
#[test]
fn only_the_orphan_row_paints_the_badge() {
    let painted = painted(app_showing_disks());
    let lines = table_lines(&painted);
    let badged: Vec<&&str> = lines
        .iter()
        .filter(|line| line.contains("ORPHAN") && !line.contains("NAME"))
        .collect();

    assert_eq!(
        badged.len(),
        1,
        "exactly one row wears the badge — the attached disk and the CREATING \
         disk must not:\n{painted}"
    );
    assert!(
        badged[0].contains("detached-legacy"),
        "the badge landed on the wrong row:\n{painted}"
    );
}

/// The badge carries `Warning`, so it is coloured as the call to action it is.
///
/// A `Cell::plain` would paint the same six characters in body text and pass
/// every buffer assertion above, so the semantic is asserted directly: it is
/// the difference between a column an operator's eye is drawn to and one they
/// scroll past.
#[test]
fn the_orphan_cell_carries_the_warning_semantic_and_a_used_disk_carries_none() {
    let app = app_showing_disks();

    let orphan = orphan_cell(&app, "detached-legacy");
    assert_eq!(orphan.text, "ORPHAN");
    assert_eq!(
        orphan.kind,
        CellKind::Status(StatusSemantic::Warning),
        "the badge must be styled as a warning, not as body text"
    );

    let attached = orphan_cell(&app, "attached-boot");
    assert_eq!(attached.text, "");
    assert_eq!(
        attached.kind,
        CellKind::Plain,
        "an in-use disk's orphan cell must be blank body text — not `Missing`, \
         which would style a known fact as a gap in the data"
    );

    // A transient disk is equally unattached and equally unbadged.
    let transient = orphan_cell(&app, "being-built");
    assert_eq!(transient.text, "");
    assert_eq!(transient.kind, CellKind::Plain);
}

// ---------------------------------------------------------------------------
// The states the pane can be in besides "here are your rows".
//
// The mutation gate found every one of these unpinned: the renderer's six
// state branches were copied from the Cloud Functions shape and only the
// populated-table path had a test, so mutants that inverted `status ==
// Loading`, `status == Failed`, `status == Stale` and the `&&` joining
// "no rows" to "load succeeded" all survived. Each screen tells the operator
// something different about a table with no rows in it, and confusing any two
// of them is the failure that matters here.

/// An empty project is not the same screen as one that has not loaded: one
/// says the project has no disks, the other says to press `r`.
#[test]
fn an_empty_project_says_so_rather_than_asking_for_a_refresh() {
    let mut app = app_with_context();
    let context = app.root.current_context().expect("active context");
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::PersistentDisksLoaded {
            context,
            disks: Vec::new(),
        }),
    );

    let painted = painted(app);

    assert!(
        painted.contains("No persistent disks found for"),
        "an empty result must say the project has none:\n{painted}"
    );
    assert!(
        !painted.contains("Press r to load persistent disks."),
        "a project known to be empty must not ask for the load it already did:\n{painted}"
    );
}

/// Before any load, the pane asks for one — and must not claim the project is
/// empty, which is the same zero-row table with the opposite meaning.
#[test]
fn a_never_loaded_list_asks_for_a_refresh_rather_than_claiming_emptiness() {
    let painted = painted(app_with_context());

    assert!(
        painted.contains("Press r to load persistent disks."),
        "a never-loaded pane must ask for the load:\n{painted}"
    );
    assert!(
        !painted.contains("No persistent disks found for"),
        "not having looked is not the same as having found none:\n{painted}"
    );
}

/// A failed load must not read as an empty project — the difference between
/// "you have no orphans" and "we could not tell".
#[test]
fn a_failed_load_paints_the_failure_and_not_an_empty_project() {
    let mut app = app_with_context();
    let context = app.root.current_context().expect("active context");
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::PersistentDisksLoadFailed {
            context,
            error: AppError::gcloud("PERMISSION_DENIED".to_string()),
        }),
    );

    let painted = painted(app);

    assert!(
        !painted.contains("No persistent disks found for"),
        "a failure must not paint as an empty project:\n{painted}"
    );
    assert!(
        painted.contains("permission denied"),
        "the failure must name what went wrong:\n{painted}"
    );
}

/// A filter that hides every row names itself, and the tools line still
/// reports how many rows are loaded behind it.
#[test]
fn a_filter_matching_nothing_says_which_filter_hid_the_rows() {
    let mut app = app_showing_disks();
    reduce(&mut app, &Action::Resources(ResourcesAction::StartFilter));
    for character in "zzz".chars() {
        reduce(
            &mut app,
            &Action::Resources(ResourcesAction::AppendFilterChar(character)),
        );
    }
    reduce(&mut app, &Action::Resources(ResourcesAction::ApplyFilter));

    let painted = painted(app);

    assert!(
        painted.contains("No persistent disks match filter \"zzz\""),
        "a filtered-to-nothing list must name the filter:\n{painted}"
    );
    assert!(
        !painted.contains("No persistent disks found for"),
        "rows hidden by a filter are not a project with no disks:\n{painted}"
    );
    assert!(
        painted.contains("Rows: 0/3"),
        "the tools line must report visible/total so the hidden rows are visible \
         as a count:\n{painted}"
    );
}

/// Rows carried over from a previous context are marked stale rather than
/// presented as the current project's disks.
#[test]
fn a_stale_list_says_so_rather_than_painting_the_previous_context() {
    let mut app = app_showing_disks();
    app.root.resources.mark_all_stale_and_clear();

    let painted = painted(app);

    assert!(
        painted.contains("stale"),
        "a stale pane must say the rows are stale:\n{painted}"
    );
    assert!(
        !painted.contains("detached-legacy"),
        "a stale pane must not still paint the previous context's disks:\n{painted}"
    );
}
