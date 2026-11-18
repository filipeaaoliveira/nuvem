//! Cloud Functions reaches the screen.
//!
//! A service can be in `SERVICE_REGISTRY`, satisfy every `ResourceServiceKind`
//! walk, hold rows in state, and still never paint one — every one of those
//! tests passes and the tab is blank. So these assertions are made on the
//! terminal buffer, through the real reducer and the real renderer, not on the
//! projection model.

use nuvem::app::projection::project_resource_preview;
use nuvem::app::projection::table::StatusSemantic;
use nuvem::app::reducer::root::reduce;
use nuvem::app::state::root::Workspace;
use nuvem::app::{Action, AppError, AppState, EffectResult, ResourcesAction};
use nuvem::services::runtime::session::RuntimeSession;
use nuvem_domain::gcp::configuration::{GcloudConfiguration, GcloudConfigurationName};
use nuvem_domain::gcp::resource::{
    CloudFunction, CloudFunctionEnvironment, CloudFunctionState, ResourceServiceKind,
};
use ratatui::Terminal;
use ratatui::backend::TestBackend;

const FRAME_WIDTH: u16 = 200;
const FRAME_HEIGHT: u16 = 40;

/// Wide enough that the table area fits the whole catalogue unelided. At the
/// default width the navigator and the preview take their share and the URL
/// cell — the widest column, and the one the layout is meant to drop first —
/// is truncated with an ellipsis, which is correct behaviour and useless to
/// assert a URL against.
const WIDE_FRAME_WIDTH: u16 = 320;

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

fn functions() -> Vec<CloudFunction> {
    vec![
        CloudFunction {
            name: "thumbnailer".to_string(),
            region: "europe-west1".to_string(),
            state: CloudFunctionState::Active,
            environment: CloudFunctionEnvironment::Gen2,
            url: Some("https://europe-west1-dev.cloudfunctions.net/thumbnailer".to_string()),
            update_time: Some("2026-03-01T10:00:00Z".to_string()),
            labels: Vec::new(),
        },
        CloudFunction {
            name: "archiver".to_string(),
            region: "us-central1".to_string(),
            state: CloudFunctionState::Deploying,
            environment: CloudFunctionEnvironment::Gen1,
            url: None,
            update_time: None,
            labels: Vec::new(),
        },
    ]
}

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
            ResourceServiceKind::CloudFunctions,
        )),
    );
    app.root.workspace = Workspace::Resources;
    app
}

fn app_showing_functions() -> AppState {
    let mut app = app_with_context();
    let context = app.root.current_context().expect("active context");
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::CloudFunctionsLoaded {
            context,
            functions: functions(),
        }),
    );
    app
}

fn painted(app: AppState) -> String {
    painted_at(app, FRAME_WIDTH)
}

fn painted_at(app: AppState, width: u16) -> String {
    let session = RuntimeSession::new_for_test(app);
    let backend = TestBackend::new(width, FRAME_HEIGHT);
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
/// selected function's own name, region, state and generation. A `contains`
/// over the whole line therefore passes on a table that paints nothing at all,
/// which is the failure these tests exist to catch. The panes are separated by
/// their adjacent borders, so the middle segment is the table.
fn table_lines(painted: &str) -> Vec<&str> {
    painted
        .lines()
        .filter_map(|line| line.split("││").nth(1))
        .collect()
}

#[test]
fn the_functions_table_paints_a_row_per_function_with_its_region() {
    let painted = painted(app_showing_functions());

    assert!(
        painted.contains("Resources / Cloud Functions"),
        "the pane must name the service it is showing:\n{painted}"
    );
    assert!(painted.contains("NAME"), "no NAME header:\n{painted}");
    assert!(painted.contains("REGION"), "no REGION header:\n{painted}");
    for (name, region) in [("thumbnailer", "europe-west1"), ("archiver", "us-central1")] {
        // Both cells on one row of the table pane: a name in the table and a
        // region somewhere else on screen would satisfy two `contains` checks
        // while the row itself carried no region at all.
        assert!(
            table_lines(&painted)
                .iter()
                .any(|line| line.contains(name) && line.contains(region)),
            "no row carries {name} beside {region}, so two same-named \
             functions would be indistinguishable:\n{painted}"
        );
    }
}

/// The four cells slice 2 deferred, on the row they belong to. Asserted as one
/// painted line per function for the reason the region check above is: a state
/// painted anywhere on screen satisfies a bare `contains`, and the whole point
/// of a table is which row a value sits in.
#[test]
fn the_functions_table_paints_state_generation_update_time_and_url() {
    let painted = painted_at(app_showing_functions(), WIDE_FRAME_WIDTH);

    for header in ["STATE", "GENERATION", "UPDATED", "URL"] {
        assert!(painted.contains(header), "no {header} header:\n{painted}");
    }

    let rows = table_lines(&painted);
    let deployed = rows
        .iter()
        .find(|line| line.contains("thumbnailer"))
        .unwrap_or_else(|| panic!("no row for thumbnailer:\n{painted}"));
    for cell in [
        "Active",
        "2nd gen",
        "2026-03-01T10:00:00Z",
        "https://europe-west1-dev.cloudfunctions.net/thumbnailer",
    ] {
        assert!(
            deployed.contains(cell),
            "the thumbnailer row must carry {cell:?}:\n{painted}"
        );
    }

    let deploying = rows
        .iter()
        .find(|line| line.contains("archiver"))
        .unwrap_or_else(|| panic!("no row for archiver:\n{painted}"));
    assert!(
        deploying.contains("Deploying"),
        "a function still deploying must say so rather than reading as active:\n{painted}"
    );
    assert!(
        deploying.contains("1st gen"),
        "the generation column must tell the two generations apart:\n{painted}"
    );
    // A function that is still deploying has no URL and no update time yet.
    // The cell an operator reads as "not reported" is a dash, and it is
    // exactly the case a blank cell would make indistinguishable from a
    // column that stopped painting.
    let absent = deploying
        .split_whitespace()
        .filter(|token| *token == "-")
        .count();
    assert_eq!(
        absent, 2,
        "a deploying function's URL and update time must each read as absent:\n{painted}"
    );
}

/// The empty case is not the same screen as the not-yet-loaded case: one says
/// the project has no functions, the other says to press `r`.
#[test]
fn an_empty_project_says_so_rather_than_asking_for_a_refresh() {
    let mut app = app_with_context();
    let context = app.root.current_context().expect("active context");
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::CloudFunctionsLoaded {
            context,
            functions: Vec::new(),
        }),
    );

    let painted = painted(app);

    assert!(
        painted.contains("No Cloud Functions found for"),
        "an empty result must say the project has none:\n{painted}"
    );
}

#[test]
fn a_filter_matching_nothing_says_which_filter_hid_the_rows() {
    let mut app = app_showing_functions();
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
        painted.contains("No Cloud Functions match filter \"zzz\""),
        "a filtered-to-nothing list must name the filter:\n{painted}"
    );
    // The tools line above the list is what says how many rows the filter hid;
    // without it the pane claims nothing matches and gives no way to see that
    // two rows are loaded behind it.
    assert!(
        painted.contains("Rows: 0/2"),
        "the tools line must report the visible/total counts:\n{painted}"
    );
}

/// A failed load must not read as an empty project.
#[test]
fn a_failed_load_paints_the_failure() {
    let mut app = app_with_context();
    let context = app.root.current_context().expect("active context");
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::CloudFunctionsLoadFailed {
            context,
            error: AppError::gcloud("PERMISSION_DENIED".to_string()),
        }),
    );

    let painted = painted(app);

    assert!(
        !painted.contains("No Cloud Functions found for"),
        "a failure must not paint as an empty project:\n{painted}"
    );
    assert!(
        painted.contains("Cloud Functions: permission denied"),
        "the failure must name the service it happened to:\n{painted}"
    );
}

// ---------------------------------------------------------------------------
// The colour behind the STATE cell, and the preview beside the row
// ---------------------------------------------------------------------------

/// A status column earns its keep by reading differently at a glance, and the
/// semantic is what carries that. Walks every documented state so a value the
/// API adds later cannot slip in uncoloured, and pins the two ends that matter
/// most: work in flight is not a failure, and a state nuvem cannot read is not
/// dressed up as one it can.
#[test]
fn every_cloud_function_state_carries_a_semantic_it_can_justify() {
    for state in CloudFunctionState::ALL {
        let expected = match state {
            CloudFunctionState::Active => StatusSemantic::Healthy,
            CloudFunctionState::Deploying | CloudFunctionState::Deleting => StatusSemantic::Busy,
            CloudFunctionState::Detaching => StatusSemantic::Warning,
            CloudFunctionState::Failed | CloudFunctionState::DetachFailed => {
                StatusSemantic::Degraded
            }
            CloudFunctionState::Unspecified | CloudFunctionState::Unknown => {
                StatusSemantic::Neutral
            }
            CloudFunctionState::Unrecognized(_) => {
                unreachable!("CloudFunctionState::ALL holds the unit variants only")
            }
        };
        assert_eq!(
            StatusSemantic::from_cloud_function_state(&state),
            expected,
            "{state:?} is coloured as something it is not"
        );
    }

    assert_eq!(
        StatusSemantic::from_cloud_function_state(&CloudFunctionState::Unrecognized(
            "BAKING".to_string()
        )),
        StatusSemantic::Neutral,
        "a state nuvem cannot read must not be painted as healthy or as broken"
    );
}

/// The preview gives each field a line of its own rather than a table cell
/// competing with five other columns, so the URL survives at widths where its
/// column is elided or dropped. The navigator and the favourite picker paint it
/// beside the selection. Slice 2 gave it one `Region` line because that was the
/// whole catalogue.
#[test]
fn the_preview_carries_every_field_the_row_holds() {
    let mut app = app_showing_functions();
    // Default sort is by name ascending, so the cursor starts on `archiver`.
    reduce(&mut app, &Action::Resources(ResourcesAction::SelectNext));

    let preview = project_resource_preview(&app.root.resources).expect("a row is selected");

    assert_eq!(preview.title, "thumbnailer");
    let lines: Vec<(String, String)> = preview.lines.clone();
    for (key, value) in [
        ("Region", "europe-west1"),
        ("State", "Active"),
        ("Generation", "2nd gen"),
        ("Updated", "2026-03-01T10:00:00Z"),
        (
            "URL",
            "https://europe-west1-dev.cloudfunctions.net/thumbnailer",
        ),
    ] {
        assert!(
            lines.iter().any(|(k, v)| k == key && v == value),
            "the preview must carry {key} = {value:?}, got {lines:?}"
        );
    }
}

/// Every preview line carries API-controlled text, and an unrecognised state
/// carries the API's own string verbatim through `label()`. The STATE cell
/// scrubs control characters; the preview beside it must scrub the same value
/// the same way rather than handing raw bytes to the terminal.
#[test]
fn the_preview_scrubs_control_characters_the_api_sent() {
    let mut app = app_with_context();
    let context = app.root.current_context().expect("active context");
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::CloudFunctionsLoaded {
            context,
            functions: vec![CloudFunction {
                name: "hos\u{7}tile".to_string(),
                region: "europe\u{1b}west1".to_string(),
                state: CloudFunctionState::Unrecognized("BAK\u{9b}ING".to_string()),
                environment: CloudFunctionEnvironment::Gen2,
                url: Some("https://example\u{0}.test/f".to_string()),
                update_time: Some("2026-03-01\u{1b}T10:00:00Z".to_string()),
                labels: Vec::new(),
            }],
        }),
    );

    let preview = project_resource_preview(&app.root.resources).expect("a row is selected");

    assert_eq!(preview.title, "hos\u{FFFD}tile");
    for (key, value) in [
        ("Region", "europe\u{FFFD}west1"),
        ("State", "Bak\u{FFFD}ing"),
        ("Updated", "2026-03-01\u{FFFD}T10:00:00Z"),
        ("URL", "https://example\u{FFFD}.test/f"),
    ] {
        assert!(
            preview.lines.iter().any(|(k, v)| k == key && v == value),
            "the preview must scrub {key} to {value:?}, got {:?}",
            preview.lines
        );
    }
}

/// The absent halves, in the surface that has room to say so. A blank line
/// beside `URL` reads as a URL the preview failed to paint.
#[test]
fn the_preview_says_absent_rather_than_painting_nothing() {
    let app = app_showing_functions();

    let preview = project_resource_preview(&app.root.resources).expect("a row is selected");

    assert_eq!(preview.title, "archiver");
    for key in ["Updated", "URL"] {
        assert!(
            preview.lines.iter().any(|(k, v)| k == key && v == "-"),
            "{key} must read as absent, got {:?}",
            preview.lines
        );
    }
}
