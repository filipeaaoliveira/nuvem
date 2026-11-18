//! The recorded history reaches the Inventory band as a
//! sparkline.
//!
//! Two layers, two properties. The projection must hand the renderer the
//! active context's series and nothing else; the renderer must draw that series
//! as bars whose heights follow it. Asserted on the painted buffer rather than
//! on "a widget was constructed", because the failure this guards against is a
//! spark drawn perfectly out of the wrong data — where a rendered-check passes
//! and the screen lies.

use std::time::Instant;

use nuvem::app::AppState;
use nuvem::app::projection::{SPARKLINE_MIN_SAMPLES, project_home_overview};
use nuvem::app::state::health::ServiceObservation;
use nuvem::app::state::root::RootState;
use nuvem::services::runtime::session::RuntimeSession;
use nuvem::ui::home::{SPARKLINE_CAPTION, SPARKLINE_WIDTH};
use nuvem_domain::gcp::configuration::{
    GcloudConfiguration, GcloudConfigurationName, GcloudContext,
};
use nuvem_domain::gcp::resource::ResourceServiceKind;
use ratatui::{Terminal, backend::TestBackend};

/// The bar glyphs `ratatui`'s `Sparkline` draws, shortest first. A rendered
/// spark is read back through this so a test can talk about bar *heights*
/// rather than about code points.
const BARS: [&str; 9] = [" ", "▁", "▂", "▃", "▄", "▅", "▆", "▇", "█"];

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

/// Writes `totals` into `root`'s history as samples taken in `context`.
///
/// Written through the real `record`, with a distinct sequence per sample, so
/// these fixtures cannot drift from what the runtime's sampler produces.
fn seed(
    root: &mut RootState,
    kind: ResourceServiceKind,
    context: &GcloudContext,
    totals: &[usize],
) {
    let at = Instant::now();
    for (index, total) in totals.iter().enumerate() {
        root.health.record(
            ServiceObservation {
                kind,
                context: context.clone(),
                total: *total,
                sequence: index as u64 + 1,
            },
            at,
        );
    }
}

fn root_on_dev() -> RootState {
    let mut root = RootState::default();
    root.configurations
        .set_rows(vec![configuration("dev", "dev-project")]);
    root
}

fn compute_card_history(root: &RootState) -> Vec<u64> {
    project_home_overview(root, Instant::now())
        .inventory
        .into_iter()
        .find(|card| card.kind == ResourceServiceKind::ComputeInstances)
        .expect("GCE instances has an inventory card")
        .history
}

#[test]
fn the_inventory_card_carries_the_active_contexts_series() {
    let mut root = root_on_dev();
    seed(
        &mut root,
        ResourceServiceKind::ComputeInstances,
        &dev_context(),
        &[3, 5, 4],
    );

    assert_eq!(compute_card_history(&root), vec![3, 5, 4]);
}

/// The hazard slice 1 exists to prevent, checked at the layer that would
/// actually paint it: a series recorded elsewhere must not reach the card for
/// the context on screen.
#[test]
fn samples_from_another_context_never_reach_the_card() {
    let mut root = root_on_dev();
    seed(
        &mut root,
        ResourceServiceKind::ComputeInstances,
        &other_context(),
        &[3, 5, 4],
    );

    assert_eq!(compute_card_history(&root), Vec::<u64>::new());
}

/// One sample is not a trend, and a single bar reads as a level. The card
/// carries nothing until there is a shape to show.
#[test]
fn a_series_shorter_than_the_minimum_is_withheld_from_the_card() {
    let mut root = root_on_dev();
    let short: Vec<usize> = (1..SPARKLINE_MIN_SAMPLES).collect();
    seed(
        &mut root,
        ResourceServiceKind::ComputeInstances,
        &dev_context(),
        &short,
    );

    assert_eq!(compute_card_history(&root), Vec::<u64>::new());
}

/// ...and the sample that reaches the minimum releases the whole series, rather
/// than only the samples taken from then on.
#[test]
fn the_sample_that_reaches_the_minimum_releases_the_whole_series() {
    let mut root = root_on_dev();
    let exactly: Vec<usize> = (1..=SPARKLINE_MIN_SAMPLES).collect();
    seed(
        &mut root,
        ResourceServiceKind::ComputeInstances,
        &dev_context(),
        &exactly,
    );

    assert_eq!(compute_card_history(&root).len(), SPARKLINE_MIN_SAMPLES);
}

#[test]
fn a_card_with_no_history_carries_an_empty_series() {
    let root = root_on_dev();

    assert_eq!(compute_card_history(&root), Vec::<u64>::new());
}

fn painted(session: &RuntimeSession) -> Vec<String> {
    let mut terminal = Terminal::new(TestBackend::new(160, 40)).expect("test terminal");
    terminal
        .draw(|frame| nuvem::ui::render(frame, session))
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

/// The bar heights painted in the GCE instances row's spark column, left to
/// right, with the leading blank cells a short series leaves dropped.
///
/// Read out of the spark column specifically rather than by scanning the whole
/// row for bar glyphs: a blank cell is a zero-height bar, so a whole-row scan
/// would read every space in the label as a bar and drown the signal.
fn row_bars(session: &RuntimeSession, label: &str) -> Vec<usize> {
    let rendered = painted(session);
    let row = rendered
        .iter()
        .find(|row| row.contains(label))
        .unwrap_or_else(|| panic!("no {label} row on screen:\n{}", rendered.join("\n")));
    let cells: Vec<char> = row.chars().collect();
    let right_border = cells
        .iter()
        .rposition(|cell| *cell == '│')
        .expect("the Inventory block's right border");
    let column = &cells[right_border - usize::from(SPARKLINE_WIDTH)..right_border];
    let heights: Vec<usize> = column
        .iter()
        .map(|cell| {
            BARS.iter()
                .position(|bar| *bar == cell.to_string())
                .unwrap_or_else(|| panic!("{cell:?} is not a bar glyph; row was {row:?}"))
        })
        .collect();
    heights
        .iter()
        .skip_while(|height| **height == 0)
        .copied()
        .collect()
}

fn compute_row_bars(session: &RuntimeSession) -> Vec<usize> {
    row_bars(session, ResourceServiceKind::ComputeInstances.label())
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

/// The property, stated as a property: a rising series draws a rising bar
/// sequence. Not "a sparkline was rendered" — a spark drawn from the wrong
/// series would satisfy that, and that is exactly the bug worth catching.
#[test]
fn a_rising_series_draws_a_rising_bar_sequence() {
    let mut session = session_on_dev();
    seed(
        &mut session.app_mut().root,
        ResourceServiceKind::ComputeInstances,
        &dev_context(),
        &[1, 2, 3, 4, 5, 6, 7, 8],
    );

    assert_eq!(
        compute_row_bars(&session),
        vec![1, 2, 3, 4, 5, 6, 7, 8],
        "a strictly rising series must paint strictly rising bars"
    );
}

#[test]
fn a_falling_series_draws_a_falling_bar_sequence() {
    let mut session = session_on_dev();
    seed(
        &mut session.app_mut().root,
        ResourceServiceKind::ComputeInstances,
        &dev_context(),
        &[8, 7, 6, 5, 4, 3, 2, 1],
    );

    assert_eq!(compute_row_bars(&session), vec![8, 7, 6, 5, 4, 3, 2, 1]);
}

/// The rendering half of the context hazard. A series belonging to another
/// context must draw nothing at all — not a flat line, not a faint one.
#[test]
fn a_series_from_another_context_draws_nothing() {
    let mut session = session_on_dev();
    seed(
        &mut session.app_mut().root,
        ResourceServiceKind::ComputeInstances,
        &other_context(),
        &[1, 2, 3, 4, 5, 6, 7, 8],
    );

    assert_eq!(
        compute_row_bars(&session),
        Vec::<usize>::new(),
        "another context's history reached the screen"
    );
}

#[test]
fn a_service_with_no_history_draws_no_bars() {
    let session = session_on_dev();

    assert_eq!(compute_row_bars(&session), Vec::<usize>::new());
}

/// The spark is a shape, not a scale: it is drawn against the series' own
/// maximum, so the same shape at four rows and at four thousand paints the same
/// bars. The magnitude is the count already on the row, and this pins that the
/// spark does not quietly claim to carry it too.
#[test]
fn the_series_is_drawn_against_its_own_maximum_not_an_absolute_one() {
    let mut small = session_on_dev();
    seed(
        &mut small.app_mut().root,
        ResourceServiceKind::ComputeInstances,
        &dev_context(),
        &[1, 2, 3, 4],
    );
    let mut large = session_on_dev();
    seed(
        &mut large.app_mut().root,
        ResourceServiceKind::ComputeInstances,
        &dev_context(),
        &[1000, 2000, 3000, 4000],
    );

    assert_eq!(compute_row_bars(&small), compute_row_bars(&large));
}

/// Every service's spark lands on that service's own line.
///
/// GCE instances is the registry's first entry, so a test that only ever reads
/// its row exercises a row offset of zero — under which every arithmetic slip
/// in the row's vertical placement is a no-op. Two services with deliberately
/// different series is what makes the offset carry a value at all, and it also
/// pins the pairing itself: a spark drawn one line off would attribute one
/// service's trend to another, which is the same class of lie as drawing the
/// wrong context's.
#[test]
fn each_services_spark_lands_on_that_services_own_line() {
    let mut session = session_on_dev();
    seed(
        &mut session.app_mut().root,
        ResourceServiceKind::ComputeInstances,
        &dev_context(),
        &[1, 2, 3, 4, 5, 6, 7, 8],
    );
    seed(
        &mut session.app_mut().root,
        ResourceServiceKind::GkeClusters,
        &dev_context(),
        &[8, 7, 6, 5, 4, 3, 2, 1],
    );

    assert_eq!(
        row_bars(&session, ResourceServiceKind::ComputeInstances.label()),
        vec![1, 2, 3, 4, 5, 6, 7, 8]
    );
    assert_eq!(
        row_bars(&session, ResourceServiceKind::GkeClusters.label()),
        vec![8, 7, 6, 5, 4, 3, 2, 1]
    );
}

/// A service far down the band gets its spark too — the row offset has to hold
/// across the whole registry, not only for the two services at the top.
#[test]
fn the_last_registered_service_gets_its_own_spark() {
    let last = *ResourceServiceKind::ALL
        .last()
        .expect("the registry is not empty");
    let mut session = session_on_dev();
    seed(
        &mut session.app_mut().root,
        last,
        &dev_context(),
        &[2, 4, 6, 8],
    );

    assert_eq!(row_bars(&session, last.label()), vec![2, 4, 6, 8]);
}

/// The spark column is captioned, so its bars say what they measure.
///
/// Without a caption the column was a run of coloured blocks at the right edge
/// of the band with nothing to read them by. The caption sits in the
/// Inventory border, right-aligned so it ends over the column, and only while
/// some row draws a spark: over an empty column it would caption nothing.
#[test]
fn the_spark_column_is_captioned_exactly_while_it_holds_a_spark() {
    let border = |session: &RuntimeSession| {
        painted(session)
            .into_iter()
            .find(|row| row.contains("Inventory"))
            .expect("the Inventory border is on screen")
    };

    let mut session = session_on_dev();
    assert!(
        !border(&session).contains(SPARKLINE_CAPTION),
        "no service has history yet, so there is no column to caption"
    );

    seed(
        &mut session.app_mut().root,
        ResourceServiceKind::ComputeInstances,
        &dev_context(),
        &[3, 5, 4],
    );
    let row = border(&session);
    assert!(
        row.trim_end().ends_with(&format!("{SPARKLINE_CAPTION}┐")),
        "the caption ends over the spark column: {row:?}"
    );
}
