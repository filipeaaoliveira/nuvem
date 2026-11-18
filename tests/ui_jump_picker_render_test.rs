//! Render coverage for the related-resource jump picker overlay (`J`).
//!
//! The picker is the only place the jump labels are ever seen, so what is
//! pinned here is that the offers reach the screen, that the cursor is visible
//! (a modal whose selection you cannot see is a modal you cannot use), and that
//! it disappears again when closed.

use nuvem::app::reducer::root::reduce;
use nuvem::app::state::root::Workspace;
use nuvem::app::{Action, AppState, EffectResult, JumpsAction, WorkspaceAction};
use nuvem::services::runtime::session::RuntimeSession;
use nuvem_domain::gcp::configuration::{GcloudConfiguration, GcloudConfigurationName};
use nuvem_domain::gcp::resource::{ComputeInstance, ResourceStatus};
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
        project: Some("dev-project".to_string()),
        region: Some("europe-west1".to_string()),
        zone: Some("europe-west1-b".to_string()),
    }
}

fn instance(name: &str) -> ComputeInstance {
    ComputeInstance {
        id: format!("{name}-id"),
        name: name.to_string(),
        zone: "europe-west1-b".to_string(),
        status: ResourceStatus::Running,
        machine_type: "e2-medium".to_string(),
        internal_ip: Some("10.0.0.2".to_string()),
        external_ip: None,
    }
}

fn app_with_compute() -> AppState {
    let mut app = AppState::new();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ConfigurationsLoaded {
            configurations: vec![config("dev")],
        }),
    );
    reduce(&mut app, &Action::Workspace(WorkspaceAction::Resources));
    let context = app.root.current_context().expect("context exists");
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ComputeInstancesLoaded {
            context,
            instances: vec![instance("web-01")],
        }),
    );
    assert_eq!(app.root.workspace, Workspace::Resources);
    app
}

fn render_at(app: AppState, width: u16, height: u16) -> String {
    let session = RuntimeSession::new_for_test(app);
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal
        .draw(|frame| nuvem::ui::render(frame, &session))
        .unwrap();
    lines(&terminal).join("\n")
}

fn render(app: AppState) -> String {
    render_at(app, 120, 30)
}

#[test]
fn the_open_picker_shows_every_offer_for_the_selected_instance() {
    let mut app = app_with_compute();
    reduce(&mut app, &Action::Jumps(JumpsAction::Open));

    let rendered = render(app);

    assert!(
        rendered.contains("Network of web-01"),
        "picker should list the Network jump:\n{rendered}"
    );
    assert!(
        rendered.contains("Disks of web-01"),
        "picker should list the Disks jump:\n{rendered}"
    );
}

/// The cursor has to be visible, and it has to be on the row the reducer says
/// is selected — a marker stuck on the first row would be the overlay lying
/// about which jump `Enter` performs.
#[test]
fn the_cursor_marker_follows_the_selected_offer() {
    let mut app = app_with_compute();
    reduce(&mut app, &Action::Jumps(JumpsAction::Open));

    let first = render(app.clone());
    assert!(
        first.contains("> Network of web-01"),
        "cursor should start on the first offer:\n{first}"
    );
    assert!(!first.contains("> Disks of web-01"));

    reduce(&mut app, &Action::Jumps(JumpsAction::SelectNext));
    let second = render(app);
    assert!(
        second.contains("> Disks of web-01"),
        "cursor should follow the selection:\n{second}"
    );
    assert!(!second.contains("> Network of web-01"));
}

#[test]
fn the_picker_teaches_its_own_keys() {
    let mut app = app_with_compute();
    reduce(&mut app, &Action::Jumps(JumpsAction::Open));

    let rendered = render(app);

    assert!(
        rendered.contains("j/k select"),
        "picker should teach movement:\n{rendered}"
    );
    assert!(
        rendered.contains("Enter jump"),
        "picker should teach the jump key:\n{rendered}"
    );
    assert!(
        rendered.contains("Esc cancel"),
        "picker should teach the way out:\n{rendered}"
    );
}

#[test]
fn a_closed_picker_paints_nothing() {
    let app = app_with_compute();

    let rendered = render(app);

    assert!(!rendered.contains("Network of web-01"));
    assert!(!rendered.contains("Disks of web-01"));
}

/// A terminal too narrow for the chrome must still say what the modal is,
/// rather than painting a torn box the user cannot read or act on.
#[test]
fn a_cramped_terminal_falls_back_to_a_readable_message() {
    let mut app = app_with_compute();
    reduce(&mut app, &Action::Jumps(JumpsAction::Open));

    let session = RuntimeSession::new_for_test(app);
    let mut terminal = Terminal::new(TestBackend::new(62, 13)).unwrap();
    terminal
        .draw(|frame| nuvem::ui::render(frame, &session))
        .unwrap();
    let rendered = lines(&terminal).join("\n");

    assert!(
        rendered.contains("more room"),
        "cramped picker should ask for room:\n{rendered}"
    );
}

/// An ultrawide terminal is not exotic, and 60% of one still has to be 60%.
/// The intermediate `width * 60` leaves `u16` above 1092 columns: a debug build
/// panics on the multiply and a release build wraps, which would hand a
/// 1200-column terminal a 64-column modal instead of a 720-column one.
#[test]
fn a_very_wide_terminal_gets_a_picker_sized_to_the_frame() {
    let mut app = app_with_compute();
    reduce(&mut app, &Action::Jumps(JumpsAction::Open));

    let session = RuntimeSession::new_for_test(app);
    let mut terminal = Terminal::new(TestBackend::new(1200, 30)).unwrap();
    terminal
        .draw(|frame| nuvem::ui::render(frame, &session))
        .unwrap();
    let rendered = lines(&terminal);

    assert!(
        rendered
            .iter()
            .any(|line| line.contains("Network of web-01")),
        "a wide picker still lists its offers"
    );

    let title_line = rendered
        .iter()
        .find(|line| line.contains("related jumps"))
        .expect("modal title should be on screen");
    let chars: Vec<char> = title_line.chars().collect();
    let title_at = chars
        .windows("related jumps".chars().count())
        .position(|window| window.iter().collect::<String>() == "related jumps")
        .expect("modal title should be on screen");
    let left = chars[..title_at]
        .iter()
        .rposition(|c| *c == '┌')
        .expect("modal should have a left border");
    let right = title_at
        + chars[title_at..]
            .iter()
            .position(|c| *c == '┐')
            .expect("modal should have a right border");

    assert_eq!(
        right - left + 1,
        720,
        "picker should be 60% of a 1200-column frame"
    );
}

/// How many rules the "more rules than the terminal has rows" case configures.
/// Well past what an 80x24 terminal can paint at once, which is the point.
const MANY_RULES: usize = 30;

fn rule_label(index: usize) -> String {
    format!("Jump rule {index:02} of web-01")
}

/// Thirty rules off the compute service, each with its own label, built through
/// the real config parser rather than by hand: the picker's row count is only
/// unbounded because the *file* is, so the test has to come in through the file.
fn app_with_many_rules() -> AppState {
    let toml_source: String = (0..MANY_RULES)
        .map(|index| {
            format!(
                "[[jumps]]\nfrom = \"compute\"\nto = \"compute:disks\"\n\
                 label = \"Jump rule {index:02} of {{name}}\"\n\n"
            )
        })
        .collect();
    // `toml::from_str`, not `parse()`: since toml 1.0 the `FromStr` impl on
    // `Value` reads a single value rather than a document.
    let document: toml::Value = toml::from_str(&toml_source).expect("fixture toml parses");
    let mut warnings = Vec::new();
    let rules = nuvem::config::jumps::JumpRules::from_file(
        document.get("jumps").expect("fixture has [[jumps]]"),
        &mut warnings,
    );
    assert!(
        warnings.is_empty(),
        "fixture rules should be valid: {warnings:?}"
    );
    assert_eq!(rules.rules().len(), MANY_RULES);

    let mut app = app_with_compute();
    app.root.jump_rules = rules;
    app
}

/// The load-bearing case: more configured rules than an 80x24 terminal can hold
/// at once. Every one of them must still be reachable, in both directions —
/// a window that scrolls down but strands the top is the same bug wearing a
/// different hat.
#[test]
fn every_configured_rule_is_reachable_on_a_short_terminal() {
    let mut app = app_with_many_rules();
    reduce(&mut app, &Action::Jumps(JumpsAction::Open));
    assert_eq!(
        app.root
            .jump_picker
            .as_ref()
            .expect("picker opens")
            .rows()
            .len(),
        MANY_RULES,
        "the fixture must really produce one row per rule"
    );

    let first = render_at(app.clone(), 80, 24);
    assert!(
        first.contains(&format!("> {}", rule_label(0))),
        "the first rule should be painted with the cursor on it:\n{first}"
    );

    for _ in 0..MANY_RULES - 1 {
        reduce(&mut app, &Action::Jumps(JumpsAction::SelectNext));
    }
    let last = render_at(app.clone(), 80, 24);
    assert!(
        last.contains(&format!("> {}", rule_label(MANY_RULES - 1))),
        "the last rule should be painted with the cursor on it:\n{last}"
    );

    for _ in 0..MANY_RULES - 1 {
        reduce(&mut app, &Action::Jumps(JumpsAction::SelectPrevious));
    }
    let back = render_at(app, 80, 24);
    assert!(
        back.contains(&format!("> {}", rule_label(0))),
        "scrolling back must reach the first rule again:\n{back}"
    );
}

/// The height boundary where the claim is tightest. 12 rows is not an arbitrary
/// small number: it is `HELP_MIN_HEIGHT` in `src/ui/mod.rs`, the shortest
/// terminal the app draws a UI on at all — anything shorter paints "nuvem needs
/// at least 60x12" and the picker overlay is never reached. So a picker that
/// opens here opens on *every* terminal the app renders on, which is the whole
/// point of decoupling the fit decision from the rule count. Do not "tidy" this
/// to a rounder size; raising the chrome budget would still leave an 80x24 test
/// green while a 12-row terminal silently went back to refusing a long picker.
///
/// 80 columns clears the footer width, so width is not what is under test here.
///
/// Absence of the fallback is the weaker half of the assertion — an empty box
/// would satisfy it. The load-bearing half is that real rule labels are painted
/// and the cursor reaches them.
#[test]
fn a_long_picker_opens_at_the_shortest_terminal_the_app_draws_on() {
    let mut app = app_with_many_rules();
    reduce(&mut app, &Action::Jumps(JumpsAction::Open));
    assert_eq!(
        app.root
            .jump_picker
            .as_ref()
            .expect("picker opens")
            .rows()
            .len(),
        MANY_RULES,
        "the fixture must really produce one row per rule"
    );

    let first = render_at(app.clone(), 80, 12);
    assert!(
        !first.contains("more room"),
        "the rule count must not refuse the picker at the app's minimum height:\n{first}"
    );
    assert!(
        first.contains(&format!("> {}", rule_label(0))),
        "the first rule should be painted with the cursor on it:\n{first}"
    );

    for _ in 0..MANY_RULES - 1 {
        reduce(&mut app, &Action::Jumps(JumpsAction::SelectNext));
    }
    let last = render_at(app, 80, 12);
    assert!(
        !last.contains("more room"),
        "scrolling must not tip the picker into the resize message:\n{last}"
    );
    assert!(
        last.contains(&format!("> {}", rule_label(MANY_RULES - 1))),
        "the last rule should be reachable at the minimum height:\n{last}"
    );
}

/// The box is sized to its content. A percentage-sized modal holding two rows
/// covers a third of the screen with blank space, hiding the list the jumps are
/// about for no reason.
#[test]
fn the_box_is_only_as_tall_as_the_offers_it_holds() {
    let mut app = app_with_compute();
    reduce(&mut app, &Action::Jumps(JumpsAction::Open));

    let rendered = render(app);
    let rows: Vec<&str> = rendered.lines().collect();
    let top = rows
        .iter()
        .position(|line| line.contains("related jumps"))
        .expect("modal title should be on screen");
    let bottom = rows
        .iter()
        .skip(top + 1)
        .position(|line| line.contains('└'))
        .expect("modal should have a bottom border")
        + top
        + 1;

    // 2 offers + a blank line + the footer + the two borders.
    assert_eq!(
        bottom - top,
        5,
        "modal should hug its content, got:\n{rendered}"
    );
}
