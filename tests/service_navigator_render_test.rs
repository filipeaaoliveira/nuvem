use nuvem::app::projection::{
    NavigatorFilterProjection, ServiceNavigatorEntryProjection, ServiceNavigatorProjection,
};
use nuvem::app::reducer::root::reduce;
use nuvem::app::service_registry::{ServiceAvailability, ServiceId, service_descriptors};
use nuvem::app::{Action, AppState, NavigatorAction, NavigatorFocus, Workspace};
use nuvem::services::runtime::session::RuntimeSession;
use nuvem::ui::service_navigator::render_service_navigator;
use ratatui::{Terminal, backend::TestBackend};

/// The cursor marker `List` paints beside the selected entry, mirroring
/// `service_navigator::HIGHLIGHT_SYMBOL`, which is private to the renderer.
const HIGHLIGHT_SYMBOL: &str = "> ";

#[test]
fn navigator_renders_grouped_services_and_no_unavailable_reason() {
    // The service navigator lives in the Resources sidebar (Home is the
    // overview dashboard). Cloud Run is an available service, so it renders
    // like GCE/GKE with no "Unavailable:" text.
    //
    // The sidebar shows labels only. It used to append each service's search
    // aliases, and at the 28-column sidebar not one of the sixteen fitted:
    // every row was cut mid-word, so `gce, c...` advertised a second term the
    // reader could neither read nor type. The aliases still match in `/`
    // search and are listed whole in the `:` palette, which is where a string
    // long enough to be worth reading belongs.
    //
    // The terminal is tall enough to hold every entry rather than 24 rows:
    // the registry is eight categories and thirteen services once BigQuery
    // datasets is included, so at 24 rows the sidebar viewport scrolls and
    // Cloud Run — filed under "Serverless", near the end — falls off the
    // bottom with the cursor at the top. The claim here is about how an
    // available service *renders*, not about where the viewport happens to
    // stop, so the height is set to keep the row on screen.
    let mut app = AppState::new();
    app.root.workspace = Workspace::Resources;
    let session = RuntimeSession::new_for_test(app);
    let terminal = render_terminal(&session, 100, 32);
    let text = terminal_text(&terminal);

    assert!(text.contains("Compute"));
    assert!(text.contains("GCE instances"));
    assert!(text.contains("Containers"));
    assert!(text.contains("Cloud Run"));
    assert!(!text.contains("Unavailable:"));
}

#[test]
fn resources_narrow_sidebar_shows_cloud_run_as_available() {
    // 32 rows rather than 24 for the reason recorded on
    // `navigator_renders_grouped_services_and_no_unavailable_reason`:
    // the full registry no longer fits in 24, and this assertion is about the
    // Cloud Run row's content, not the viewport's height.
    let mut app = AppState::new();
    app.root.workspace = Workspace::Resources;
    let session = RuntimeSession::new_for_test(app);
    let lines = terminal_lines(&render_terminal(&session, 100, 32));
    let cloud_run_line = lines
        .iter()
        .find(|line| line.contains("Cloud Run"))
        .expect("Cloud Run row should render in the sidebar");

    assert!(!cloud_run_line.contains("Unavailable:"));
}

#[test]
fn navigator_renders_unavailable_reason_for_an_unavailable_service() {
    // Cloud Run is now the only registry service and it is Available, so the
    // "Unavailable: <reason>" render path (service_navigator::render_entry /
    // ServiceNavigatorEntryProjection::unavailable_reason) is no longer
    // exercised by any registry-backed test. Build a synthetic projection
    // with an unavailable entry directly so this rendering path stays
    // covered.
    let projection = ServiceNavigatorProjection {
        entries: vec![
            ServiceNavigatorEntryProjection {
                id: None,
                category: Some("Compute"),
                label: "Compute",
                selectable: false,
                availability: None,
                unavailable_reason: None,
                is_active: false,
            },
            ServiceNavigatorEntryProjection {
                id: Some(ServiceId::ComputeInstances),
                category: None,
                label: "GCE instances",
                selectable: true,
                availability: Some(ServiceAvailability::Unavailable("coming soon")),
                unavailable_reason: Some("coming soon"),
                is_active: false,
            },
        ],
        selected_id: ServiceId::ComputeInstances,
        selected_visible: 1,
        filter: NavigatorFilterProjection {
            active: false,
            draft: String::new(),
            applied: String::new(),
        },
        sidebar_visible: true,
        focus: NavigatorFocus::Navigator,
        viewport_start: 0,
        viewport_end: 2,
    };

    let backend = TestBackend::new(100, 24);
    let mut terminal = Terminal::new(backend).expect("terminal should initialize");
    terminal
        .draw(|frame| {
            render_service_navigator(frame, frame.area(), &projection, "Service navigator")
        })
        .expect("render should succeed");
    let lines = terminal_lines(&terminal);
    let gce_line = lines
        .iter()
        .find(|line| line.contains("GCE instances"))
        .expect("GCE row should render");
    let label_offset = gce_line
        .find("GCE instances")
        .expect("GCE row should show its label");
    let unavailable_offset = gce_line
        .find("Unavailable:")
        .expect("GCE row should show its unavailable reason");

    assert!(gce_line.contains("coming soon"));
    // The reason is a suffix of the label, not a replacement for it: a row
    // that showed only the reason would still satisfy the two checks above.
    assert!(label_offset < unavailable_offset);
}

#[test]
fn navigator_marks_the_selected_service_with_selected_row_style() {
    let mut app = AppState::new();
    app.root.workspace = Workspace::Resources;
    let session = RuntimeSession::new_for_test(app);
    let terminal = render_terminal(&session, 100, 24);
    let buffer = terminal.backend().buffer();
    let selected_cell = buffer
        .content()
        .iter()
        .enumerate()
        .find(|(_, cell)| cell.symbol() == "G" && cell.bg == ratatui::style::Color::Blue)
        .map(|(index, _)| index)
        .expect("selected GCE row should use selected style");

    assert_eq!(buffer.content()[selected_cell].symbol(), "G");
}

#[test]
fn navigator_renders_an_empty_search_result_message() {
    let mut app = AppState::new();
    app.root.workspace = Workspace::Resources;
    reduce(&mut app, &Action::Navigator(NavigatorAction::StartFilter));
    reduce(
        &mut app,
        &Action::Navigator(NavigatorAction::AppendFilterChar('x')),
    );
    reduce(&mut app, &Action::Navigator(NavigatorAction::ApplyFilter));
    let session = RuntimeSession::new_for_test(app);
    let text = terminal_text(&render_terminal(&session, 100, 24));

    assert!(text.contains("No services match \"x\""));
}

#[test]
fn resources_uses_a_sidebar_when_the_body_can_fit_it() {
    let mut app = AppState::new();
    app.root.workspace = Workspace::Resources;
    let session = RuntimeSession::new_for_test(app);
    let terminal = render_terminal(&session, 100, 24);
    let text = terminal_text(&terminal);

    assert!(text.contains("Service navigator"));
    assert!(text.contains("Resources / GCE instances"));
}

#[test]
fn resources_uses_a_vertical_navigator_when_the_body_is_narrow() {
    let mut app = AppState::new();
    app.root.workspace = Workspace::Resources;
    let session = RuntimeSession::new_for_test(app);
    let terminal = render_terminal(&session, 60, 12);
    let lines = terminal_lines(&terminal);
    let navigator_line = lines
        .iter()
        .position(|line| line.contains("Service navigator"))
        .expect("narrow resources should render navigator");
    let content_line = lines
        .iter()
        .position(|line| line.contains("Resources / GCE instances"))
        .expect("narrow resources should render content");

    assert!(navigator_line < content_line);
}

#[test]
fn resources_hides_sidebar_without_hiding_active_service_identity() {
    let mut app = AppState::new();
    app.root.workspace = Workspace::Resources;
    reduce(&mut app, &Action::Navigator(NavigatorAction::ToggleSidebar));
    let session = RuntimeSession::new_for_test(app);
    let terminal = render_terminal(&session, 100, 24);
    let text = terminal_text(&terminal);

    assert!(!text.contains("Service navigator"));
    assert!(text.contains("GCE instances"));
    assert!(text.contains("Resources / GCE instances"));
}

fn render_terminal(session: &RuntimeSession, width: u16, height: u16) -> Terminal<TestBackend> {
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).expect("terminal should initialize");
    terminal
        .draw(|frame| nuvem::ui::render(frame, session))
        .expect("render should succeed");
    terminal
}

fn terminal_text(terminal: &Terminal<TestBackend>) -> String {
    terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|cell| cell.symbol())
        .collect()
}

fn terminal_lines(terminal: &Terminal<TestBackend>) -> Vec<String> {
    let buffer = terminal.backend().buffer();
    let width = usize::from(buffer.area.width);
    buffer
        .content()
        .chunks(width)
        .map(|line| line.iter().map(|cell| cell.symbol()).collect())
        .collect()
}

/// A truncated sidebar entry ends in the full overflow marker.
///
/// The defect was that entries were fitted to the block's inner width
/// — borders only — while `List` also reserves the width of `highlight_symbol`
/// on every row, selected or not. So each entry was truncated twice, at two
/// different widths, and the second truncation ate two of `fit_text`'s three
/// dots. Every truncated entry on screen ended in a lone `.`, which reads as a
/// typo rather than as "there is more here".
///
/// Asserted on the painted buffer: the whole defect lives in the gap between
/// the width the text was fitted to and the width the widget painted it at, so
/// a unit test on `render_entry` cannot see it.
///
/// The overflow used to come from the registry, whose alias lists were long
/// enough that the real sidebar truncated on every run. The sidebar shows
/// labels only now, and no registry label overflows 28 columns, so the row is
/// built here instead. That is not a weaker test: the defect was never about
/// aliases, it was about the two widths disagreeing, and a synthetic row makes
/// the overflow a stated premise rather than a property of the registry that a
/// shorter label could silently retire.
#[test]
fn a_truncated_sidebar_entry_keeps_its_whole_overflow_marker() {
    const WIDTH: u16 = 24;
    // Longer than the entry width (WIDTH less two border columns and the two
    // the cursor gutter reserves), so it has to be cut.
    const LONG_LABEL: &str = "A service label that does not fit";
    assert!(
        LONG_LABEL.len() > usize::from(WIDTH),
        "the premise of this test is a label the sidebar cannot fit"
    );

    let projection = ServiceNavigatorProjection {
        entries: vec![ServiceNavigatorEntryProjection {
            id: Some(ServiceId::ComputeInstances),
            category: None,
            label: LONG_LABEL,
            selectable: true,
            availability: Some(ServiceAvailability::Available),
            unavailable_reason: None,
            is_active: false,
        }],
        selected_id: ServiceId::ComputeInstances,
        selected_visible: 0,
        filter: NavigatorFilterProjection {
            active: false,
            draft: String::new(),
            applied: String::new(),
        },
        sidebar_visible: true,
        focus: NavigatorFocus::Navigator,
        viewport_start: 0,
        viewport_end: 1,
    };

    let backend = TestBackend::new(WIDTH, 4);
    let mut terminal = Terminal::new(backend).expect("terminal should initialize");
    terminal
        .draw(|frame| {
            render_service_navigator(frame, frame.area(), &projection, "Service navigator")
        })
        .expect("render should succeed");
    let lines = terminal_lines(&terminal);

    let row = lines
        .iter()
        .find(|line| line.contains("A service label"))
        .expect("the long entry should render");
    let content: String = row.chars().skip(1).take_while(|c| *c != '│').collect();
    let trimmed = content.trim_end();

    assert!(
        trimmed.ends_with('.'),
        "this test proves nothing unless the entry was truncated: {trimmed:?}"
    );
    assert!(
        trimmed.ends_with("..."),
        "a truncated sidebar entry ends mid-marker: {trimmed:?}"
    );
}

/// Every selectable sidebar row is its service's label and nothing else.
///
/// The row used to carry the service's search aliases after the label, and at
/// the 28-column sidebar not one of the sixteen alias lists fitted: `fit_text`
/// cut every one of them mid-word, so the column advertised terms like
/// `gce, c...` that the reader could neither finish reading nor type. The
/// aliases still match in `/` search and are listed whole in the `:` palette.
///
/// Asserted as equality against the whole registry rather than as the absence
/// of particular alias strings. Absence is the weaker shape here and would not
/// have held: re-appending the aliases at this width paints `gce, c...`, so a
/// test looking for `compute engine` would pass while the column was ruined
/// again. Equality fails on any suffix, truncated or not.
///
/// `Unavailable: <reason>` is the one suffix a row may legitimately carry, so
/// unavailable services are checked for the prefix instead — and there are
/// none in the registry today, which is why that arm is not the claim.
#[test]
fn a_sidebar_row_shows_its_service_label_and_nothing_after_it() {
    let mut app = AppState::new();
    app.root.workspace = Workspace::Resources;
    let session = RuntimeSession::new_for_test(app);
    let lines = terminal_lines(&render_terminal(&session, 100, 40));

    // The sidebar is the first block on each row; take its content up to its
    // own right border, not the whole line, which also carries the table.
    let rows: Vec<String> = lines
        .iter()
        .filter(|line| line.starts_with('│'))
        .map(|line| {
            let content: String = line.chars().skip(1).take_while(|c| *c != '│').collect();
            let content = content.trim();
            // `List` paints the cursor gutter on the selected row. That is the
            // widget's, not the entry's, and stripping it is what keeps this
            // an assertion about `render_entry` rather than about which row
            // happens to be selected.
            content
                .strip_prefix(HIGHLIGHT_SYMBOL.trim_end())
                .unwrap_or(content)
                .trim()
                .to_string()
        })
        .collect();
    assert!(
        !rows.is_empty(),
        "no sidebar rows painted:\n{}",
        lines.join("\n")
    );

    for service in service_descriptors() {
        let label = service.label;
        match service.unavailable_reason() {
            None => assert!(
                rows.iter().any(|row| row == label),
                "the sidebar row for {label:?} carries something after the label, or is \
                 missing; rows: {rows:?}"
            ),
            Some(reason) => assert!(
                rows.iter()
                    .any(|row| *row == format!("{label}  Unavailable: {reason}")),
                "the sidebar row for the unavailable {label:?} is not label + reason; \
                 rows: {rows:?}"
            ),
        }
    }
}
