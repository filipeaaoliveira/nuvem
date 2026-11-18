//! A service's line marks the direction its count just moved,
//! for a short window after it moved.
//!
//! k9s's `internal/ui/deltas.go` flashes a cell when a polled value changes.
//! The same idea over the counts Home already polls: the change is read out of
//! the history slice 1 records, so it inherits that slice's context scoping for
//! free — a count that "changed" only because the operator switched projects
//! did not change.
//!
//! The clock is a parameter everywhere below. `src/app/state/tunnels.rs` is the
//! precedent, and it is what makes the decay window assertable without
//! sleeping: a test that slept for the window would be slow, and a test that
//! shortened the window to avoid sleeping would not be testing the window.

use std::time::{Duration, Instant};

use nuvem::app::AppState;
use nuvem::app::projection::{DELTA_HIGHLIGHT_WINDOW, project_home_overview};
use nuvem::app::state::health::{ChangeDirection, ServiceHealthHistory, ServiceObservation};
use nuvem::app::state::root::RootState;
use nuvem::services::runtime::session::RuntimeSession;
use nuvem::ui::home::{DELTA_FELL_MARKER, DELTA_ROSE_MARKER};
use nuvem_domain::gcp::configuration::{
    GcloudConfiguration, GcloudConfigurationName, GcloudContext,
};
use nuvem_domain::gcp::resource::ResourceServiceKind;
use ratatui::{Terminal, backend::TestBackend};

const COMPUTE: ResourceServiceKind = ResourceServiceKind::ComputeInstances;

fn configuration(name: &str, project: &str) -> GcloudConfiguration {
    GcloudConfiguration {
        name: GcloudConfigurationName::new(name),
        is_active: true,
        account: None,
        project: Some(project.to_string()),
        region: None,
        zone: None,
    }
}

fn dev_context() -> GcloudContext {
    GcloudContext {
        configuration: GcloudConfigurationName::new("dev"),
        account: None,
        project: Some("dev-project".to_string()),
        region: None,
        zone: None,
    }
}

fn other_context() -> GcloudContext {
    GcloudContext {
        configuration: GcloudConfigurationName::new("other"),
        account: None,
        project: Some("other-project".to_string()),
        region: None,
        zone: None,
    }
}

/// Records one sample, at `at`, under a sequence taken from `step` so no two
/// calls in a test collide and get dropped as a repeat.
fn record(
    history: &mut ServiceHealthHistory,
    context: &GcloudContext,
    total: usize,
    step: u64,
    at: Instant,
) {
    history.record(
        ServiceObservation {
            kind: COMPUTE,
            context: context.clone(),
            total,
            sequence: step,
        },
        at,
    );
}

#[test]
fn a_count_that_went_up_reports_that_it_rose() {
    let start = Instant::now();
    let context = dev_context();
    let mut history = ServiceHealthHistory::default();
    record(&mut history, &context, 3, 1, start);
    record(&mut history, &context, 7, 2, start);

    assert_eq!(
        history.recent_change_in(COMPUTE, &context, start, DELTA_HIGHLIGHT_WINDOW),
        Some(ChangeDirection::Rose)
    );
}

#[test]
fn a_count_that_went_down_reports_that_it_fell() {
    let start = Instant::now();
    let context = dev_context();
    let mut history = ServiceHealthHistory::default();
    record(&mut history, &context, 7, 1, start);
    record(&mut history, &context, 3, 2, start);

    assert_eq!(
        history.recent_change_in(COMPUTE, &context, start, DELTA_HIGHLIGHT_WINDOW),
        Some(ChangeDirection::Fell)
    );
}

/// A poll that returned the same count is not a change. Flashing on every poll
/// would make the marker mean "we polled", which is not information the
/// operator needs and is exactly the noise that teaches people to stop reading
/// a highlight.
#[test]
fn a_repeated_count_is_not_a_change() {
    let start = Instant::now();
    let context = dev_context();
    let mut history = ServiceHealthHistory::default();
    for step in 1..=4 {
        record(&mut history, &context, 5, step, start);
    }

    assert_eq!(
        history.recent_change_in(COMPUTE, &context, start, DELTA_HIGHLIGHT_WINDOW),
        None
    );
}

/// The first sample has nothing to have changed from. Reading it as a rise from
/// zero would flash every service once at startup, which says "everything just
/// appeared" — true of the history, and false of the infrastructure.
#[test]
fn the_first_sample_of_a_series_is_not_a_change() {
    let start = Instant::now();
    let context = dev_context();
    let mut history = ServiceHealthHistory::default();
    record(&mut history, &context, 9, 1, start);

    assert_eq!(
        history.recent_change_in(COMPUTE, &context, start, DELTA_HIGHLIGHT_WINDOW),
        None
    );
}

#[test]
fn a_change_inside_the_window_is_still_reported() {
    let start = Instant::now();
    let context = dev_context();
    let mut history = ServiceHealthHistory::default();
    record(&mut history, &context, 3, 1, start);
    record(&mut history, &context, 7, 2, start);

    let almost = start + DELTA_HIGHLIGHT_WINDOW - Duration::from_millis(1);
    assert_eq!(
        history.recent_change_in(COMPUTE, &context, almost, DELTA_HIGHLIGHT_WINDOW),
        Some(ChangeDirection::Rose)
    );
}

/// The decay. A highlight that never faded would be a permanent mark on every
/// service that has ever moved, which after an hour is every service.
#[test]
fn a_change_older_than_the_window_has_decayed() {
    let start = Instant::now();
    let context = dev_context();
    let mut history = ServiceHealthHistory::default();
    record(&mut history, &context, 3, 1, start);
    record(&mut history, &context, 7, 2, start);

    let after = start + DELTA_HIGHLIGHT_WINDOW;
    assert_eq!(
        history.recent_change_in(COMPUTE, &context, after, DELTA_HIGHLIGHT_WINDOW),
        None,
        "the window is a duration the change is inside, not one it is at the end of"
    );
}

/// Only the newest change counts. An older change still inside the window would
/// otherwise keep a marker alive after the count had settled back.
#[test]
fn a_settled_count_stops_being_marked_even_while_the_older_change_is_recent() {
    let start = Instant::now();
    let context = dev_context();
    let mut history = ServiceHealthHistory::default();
    record(&mut history, &context, 3, 1, start);
    record(&mut history, &context, 7, 2, start);
    record(&mut history, &context, 7, 3, start);

    assert_eq!(
        history.recent_change_in(COMPUTE, &context, start, DELTA_HIGHLIGHT_WINDOW),
        None
    );
}

/// The hazard again, at the layer that would misread it: a count is compared
/// against the previous count *in the same context*. Interleaved contexts would
/// otherwise make a project switch look like a change in the project switched
/// to.
#[test]
fn a_change_is_measured_against_the_previous_sample_in_the_same_context() {
    let start = Instant::now();
    let dev = dev_context();
    let other = other_context();
    let mut history = ServiceHealthHistory::default();
    record(&mut history, &dev, 3, 1, start);
    record(&mut history, &other, 900, 2, start);
    record(&mut history, &dev, 3, 3, start);

    assert_eq!(
        history.recent_change_in(COMPUTE, &dev, start, DELTA_HIGHLIGHT_WINDOW),
        None,
        "dev's count never moved; only the context did"
    );
}

#[test]
fn a_service_with_no_history_at_all_reports_no_change() {
    let history = ServiceHealthHistory::default();

    for kind in ResourceServiceKind::ALL {
        assert_eq!(
            history.recent_change_in(kind, &dev_context(), Instant::now(), DELTA_HIGHLIGHT_WINDOW),
            None,
            "{kind:?} claimed a change with no history"
        );
    }
}

fn root_on_dev() -> RootState {
    let mut root = RootState::default();
    root.configurations
        .set_rows(vec![configuration("dev", "dev-project")]);
    root
}

fn compute_delta(root: &RootState, at: Instant) -> Option<ChangeDirection> {
    project_home_overview(root, at)
        .inventory
        .into_iter()
        .find(|card| card.kind == COMPUTE)
        .expect("GCE instances has an inventory card")
        .delta
}

#[test]
fn the_inventory_card_carries_a_recent_change_and_then_stops_carrying_it() {
    let start = Instant::now();
    let mut root = root_on_dev();
    record(&mut root.health, &dev_context(), 3, 1, start);
    record(&mut root.health, &dev_context(), 7, 2, start);

    assert_eq!(compute_delta(&root, start), Some(ChangeDirection::Rose));
    assert_eq!(compute_delta(&root, start + DELTA_HIGHLIGHT_WINDOW), None);
}

fn painted(session: &RuntimeSession) -> Vec<String> {
    let mut terminal = Terminal::new(TestBackend::new(160, 40)).expect("test terminal");
    terminal
        .draw(|frame| nuvem::ui::render_at(frame, session, Instant::now()))
        .expect("draw");
    let buffer = terminal.backend().buffer();
    (0..buffer.area.height)
        .map(|y| {
            (0..buffer.area.width)
                .map(|x| buffer.cell((x, y)).expect("cell").symbol().to_string())
                .collect::<String>()
        })
        .collect()
}

fn compute_row(session: &RuntimeSession) -> String {
    let rendered = painted(session);
    rendered
        .iter()
        .find(|row| row.contains(COMPUTE.label()))
        .unwrap_or_else(|| panic!("no GCE instances row on screen:\n{}", rendered.join("\n")))
        .clone()
}

fn session_on_dev() -> RuntimeSession {
    let mut session = RuntimeSession::new_for_test(AppState::new());
    session
        .app_mut()
        .root
        .configurations
        .set_rows(vec![configuration("dev", "dev-project")]);
    session
}

#[test]
fn a_count_that_just_rose_is_marked_on_the_painted_row() {
    let start = Instant::now();
    let mut session = session_on_dev();
    let root = &mut session.app_mut().root;
    record(&mut root.health, &dev_context(), 3, 1, start);
    record(&mut root.health, &dev_context(), 7, 2, start);

    let row = compute_row(&session);

    assert!(
        row.contains(DELTA_ROSE_MARKER),
        "the risen count carries no marker; row was {row:?}"
    );
    assert!(
        !row.contains(DELTA_FELL_MARKER),
        "a risen count must not be marked as fallen; row was {row:?}"
    );
}

#[test]
fn a_count_that_just_fell_is_marked_on_the_painted_row() {
    let start = Instant::now();
    let mut session = session_on_dev();
    let root = &mut session.app_mut().root;
    record(&mut root.health, &dev_context(), 7, 1, start);
    record(&mut root.health, &dev_context(), 3, 2, start);

    let row = compute_row(&session);

    assert!(
        row.contains(DELTA_FELL_MARKER),
        "the fallen count carries no marker; row was {row:?}"
    );
    assert!(
        !row.contains(DELTA_ROSE_MARKER),
        "a fallen count must not be marked as risen; row was {row:?}"
    );
}

#[test]
fn a_steady_count_is_not_marked_on_the_painted_row() {
    let start = Instant::now();
    let mut session = session_on_dev();
    let root = &mut session.app_mut().root;
    for step in 1..=3 {
        record(&mut root.health, &dev_context(), 5, step, start);
    }

    let row = compute_row(&session);

    assert!(!row.contains(DELTA_ROSE_MARKER), "row was {row:?}");
    assert!(!row.contains(DELTA_FELL_MARKER), "row was {row:?}");
}

/// The decayed marker leaves the screen, not just the projection.
#[test]
fn a_change_older_than_the_window_leaves_no_marker_on_screen() {
    let start = Instant::now();
    let mut session = session_on_dev();
    let root = &mut session.app_mut().root;
    let long_ago = start
        .checked_sub(DELTA_HIGHLIGHT_WINDOW + Duration::from_secs(60))
        .expect("the monotonic clock is well past the window at test time");
    record(&mut root.health, &dev_context(), 3, 1, long_ago);
    record(&mut root.health, &dev_context(), 7, 2, long_ago);

    let row = compute_row(&session);

    assert!(!row.contains(DELTA_ROSE_MARKER), "row was {row:?}");
    assert!(!row.contains(DELTA_FELL_MARKER), "row was {row:?}");
}
