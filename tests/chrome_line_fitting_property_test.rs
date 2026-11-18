//! One invariant over a bug class, rather than a third example.
//!
//! # The contract
//!
//! **A single-line chrome surface that does not fit its pane ends in the
//! overflow marker.** The operator can always tell a complete line from a cut
//! one.
//!
//! "Chrome" here is the app's own single-line furniture — a tools row, a pane
//! title, a tab bar, a hint row — as opposed to table cells, which carry their
//! own per-cell `…` through `fit_cell`, and body paragraphs, which wrap.
//!
//! # Why a property and not another example
//!
//! Three sites have now been found painting one of these rows as a bare
//! `Paragraph` over an unfitted string, so ratatui hard-clipped it and the
//! operator read `←/→ col · s sor` or `Esc/q ret` with nothing to say more was
//! there. Two were the resource table's tools row and the list header;
//! the third is the detail pane's title, tab bar and hint row. Each
//! example test pins one row at one width; the class survives, and the next
//! unfitted `Paragraph::new` will not announce itself either.
//!
//! # How it is checked
//!
//! Truncation is not observable from one frame: a row that stops at the pane
//! edge looks the same whether it was cut or happened to fit. So each row is
//! compared against *itself* rendered on a terminal wide enough to cut nothing.
//! If the two differ, the narrow one must end in a marker.
//!
//! The height is held fixed across the pair, so the vertical layout is the same
//! and a row keeps its meaning; only the width varies, which is the axis the
//! defect lives on.

use std::cell::RefCell;
use std::collections::HashMap;

use proptest::prelude::*;

use nuvem::Config;
use nuvem::app::{Action, AppState, ResourcesAction, WorkspaceAction};
use nuvem::services::demo::DemoCommandRunner;
use nuvem::services::runtime::session::RuntimeSession;
use nuvem_domain::gcp::resource::ResourceServiceKind;
use ratatui::{Terminal, backend::TestBackend};

/// Wide enough that no chrome row in any view is cut. Every row rendered here
/// is the reference "whole" text for that row.
///
/// Sized rather than picked generously: this is the file's most expensive
/// render and it happens once per reference-cache miss. The widest chrome row is
/// Cloud SQL's seven-tab bar at about 77 columns, and the right-hand pane takes
/// at most 45% of the content pane after the 28-column sidebar — so 199 columns
/// is where nothing is cut, and 240 leaves room for a service with more tabs
/// without paying for 400 columns of buffer on every miss.
const UNCUT_WIDTH: u16 = 240;

/// The heights these properties generate.
///
/// A handful, not a range. The defect lives on the *width* axis — height only
/// decides which rows are on screen at all — and every distinct height is a
/// fresh miss in the reference cache, each costing a render at `UNCUT_WIDTH`.
/// Generating `24..=44` made almost every case a miss and took the Resources
/// property past the suite's ten-second per-test budget on CI, where it timed
/// out; locally it merely looked slow. Two heights keep the same coverage of the
/// axis that matters and let the cache do its job.
const REFERENCE_HEIGHTS: [u16; 2] = [24, 40];

/// The markers `fit_text` and `fit_cell` end a truncated string with.
const OVERFLOW_MARKERS: [&str; 2] = ["...", "…"];

fn ends_in_a_marker(text: &str) -> bool {
    OVERFLOW_MARKERS
        .iter()
        .any(|marker| text.trim_end().ends_with(marker))
}

fn render(session: &RuntimeSession, width: u16, height: u16) -> Vec<String> {
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

/// The `index`-th bordered pane's content on a row of the wide Resources
/// layout: sidebar, table, then preview or detail. Adjacent borders collapse to
/// `││`, so the empty strings between them are dropped rather than counted.
fn pane(row: &str, index: usize) -> String {
    row.split('│')
        .filter(|segment| !segment.trim().is_empty())
        .nth(index)
        .unwrap_or_default()
        .trim_end()
        .to_string()
}

/// A located chrome row: the surface's stable name, and the text painted for it.
type ChromeRow = (&'static str, String);

/// The chrome rows of one rendered frame, each paired with a stable name so a
/// failure says which surface broke.
///
/// Located by structure rather than by their text: the text is exactly what the
/// defect changes, so a locator that matched on it would stop finding the row
/// at the widths where it is cut.
fn chrome_rows(lines: &[String]) -> Vec<ChromeRow> {
    let mut rows = Vec::new();

    // The tools row: the frame's own row above the table block, which is the
    // last row before the table pane's top border.
    if let Some(index) = lines.iter().position(|line| pane(line, 1).starts_with('┌'))
        && index > 0
    {
        let above = lines[index - 1].trim_end();
        // Everything from the tools text to the next block's border.
        let tools: String = above
            .chars()
            .skip_while(|character| *character != 'F')
            .take_while(|character| *character != '┌')
            .collect();
        if !tools.trim().is_empty() {
            rows.push(("tools row", tools.trim_end().to_string()));
        }
    }

    // The right-hand pane's first three rows: in Detail view the title, the tab
    // bar and the hint row; in List view the preview's title and its first
    // fields. Located by position rather than by their text, because the text
    // is exactly what the defect changes.
    //
    // The pane's top border shares a row with the tools line and carries no
    // `│` separators, so the first row with any right-pane content is already
    // the row below it.
    if let Some(top) = lines
        .iter()
        .position(|line| !pane(line, 2).trim().is_empty())
    {
        for (offset, name) in [
            (0, "right pane row 1"),
            (1, "right pane row 2"),
            (2, "right pane row 3"),
        ] {
            if let Some(line) = lines.get(top + offset) {
                let content = pane(line, 2);
                if !content.trim().is_empty() && !content.contains('└') {
                    rows.push((name, content));
                }
            }
        }
    }

    rows
}

fn build_session(kind: ResourceServiceKind, detail: bool) -> RuntimeSession {
    let config = Config {
        demo: true,
        ..Config::default()
    };
    let mut app = AppState::new();
    nuvem::services::runtime::apply_startup_config(&mut app, &config);
    let mut session = RuntimeSession::new_with_config_and_runner_for_test(
        config,
        app,
        Box::new(DemoCommandRunner::new()),
    );

    // Driven to quiescence before rendering, so every frame is of a settled
    // view rather than of a spinner.
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime");
    runtime.block_on(async {
        session.initialize().await;
        drain(&mut session).await;
        session
            .dispatch_action(Action::Workspace(WorkspaceAction::Resources))
            .await;
        session
            .dispatch_action(Action::Resources(ResourcesAction::SelectService(kind)))
            .await;
        drain(&mut session).await;
        if detail {
            session
                .dispatch_action(Action::Resources(ResourcesAction::OpenSelected))
                .await;
            drain(&mut session).await;
        }
    });
    session
}

thread_local! {
    /// One session per (service, view mode), built on first use.
    ///
    /// Not per case: building one costs a demo `initialize` plus three drains,
    /// and there are only thirty-two distinct ones, while proptest generates
    /// many more cases than that. Rebuilding per case made the property the
    /// slowest test in the suite by two orders of magnitude, which is a cost the
    /// fast tier pays on every run.
    static SESSIONS: RefCell<HashMap<(usize, bool), RuntimeSession>> =
        RefCell::new(HashMap::new());

    /// The uncut reference rows, per (service, view mode, height). Rendering at
    /// `UNCUT_WIDTH` is the same answer for every generated width, so it is
    /// computed once rather than once per case.
    static REFERENCES: RefCell<HashMap<(usize, bool, u16), Vec<ChromeRow>>> =
        RefCell::new(HashMap::new());
}

fn with_session<R>(index: usize, detail: bool, act: impl FnOnce(&RuntimeSession) -> R) -> R {
    SESSIONS.with(|cache| {
        let mut cache = cache.borrow_mut();
        let session = cache
            .entry((index, detail))
            .or_insert_with(|| build_session(service_kind(index), detail));
        act(session)
    })
}

fn reference_rows(index: usize, detail: bool, height: u16) -> Vec<ChromeRow> {
    REFERENCES.with(|cache| {
        if let Some(rows) = cache.borrow().get(&(index, detail, height)) {
            return rows.clone();
        }
        let rows = with_session(index, detail, |session| {
            chrome_rows(&render(session, UNCUT_WIDTH, height))
        });
        cache
            .borrow_mut()
            .insert((index, detail, height), rows.clone());
        rows
    })
}

/// Drains the effect-result channel until nothing more can arrive.
///
/// Two conditions, both required. The channel must be quiet — a probe finds
/// nothing within five milliseconds — and the session must report no effect
/// task still alive to deliver. Quiet alone was the exit once, and under load
/// it exits early: a demo fetch on a background thread takes longer than the
/// probe when the machine is busy, the drain returns with the detail pane
/// still loading, and an example test that asserts the settled layout fails
/// for a reason that has nothing to do with the layout. The in-flight count
/// is what makes the exit a fact rather than a guess; the probe only paces
/// the loop.
///
/// The iteration cap bounds a genuine hang. If it is reached the session is
/// not settled, and the assertions downstream say so rather than passing
/// over a frame they did not mean to test.
async fn drain(session: &mut RuntimeSession) {
    for _ in 0..4096 {
        let quiet = !matches!(
            tokio::time::timeout(
                std::time::Duration::from_millis(5),
                session.process_next_effect_result(),
            )
            .await,
            Ok(true)
        );
        if quiet && session.effect_tasks_in_flight() == 0 {
            break;
        }
    }
}

fn service_kind(index: usize) -> ResourceServiceKind {
    ResourceServiceKind::ALL[index % ResourceServiceKind::ALL.len()]
}

proptest! {
    // Fewer cases than proptest's default, and the reason is a budget rather
    // than a preference: each distinct (service, view mode) is a real session
    // build, and the suite gives every test ten seconds. At 96 this test timed
    // out on CI, and then failed the mutation gate's own baseline run locally —
    // `cargo mutants` re-runs the suite per mutant, so a test that only fits
    // the budget on an idle machine makes the gate itself flaky. The known
    // killers replay first from the checked-in seeds either way.
    #![proptest_config(ProptestConfig::with_cases(32))]

    /// The invariant, over every service, both view modes, and the widths that
    /// draw the wide layout.
    ///
    /// The lower bound is 116, the narrowest terminal that splits a right-hand
    /// pane off at all; below it there is no second pane and the surfaces this
    /// is about are not on screen.
    #[test]
    fn a_chrome_row_that_does_not_fit_says_so(
        index in 0usize..ResourceServiceKind::ALL.len(),
        width in 116u16..=240,
        height in prop::sample::select(REFERENCE_HEIGHTS.as_slice()),
        detail in any::<bool>(),
    ) {
        let kind = service_kind(index);
        let whole = reference_rows(index, detail, height);
        let shown = with_session(index, detail, |session| {
            chrome_rows(&render(session, width, height))
        });

        for (name, text) in &shown {
            let Some((_, reference)) = whole.iter().find(|(other, _)| other == name) else {
                // The surface is not on screen at the uncut width, so there is
                // no "whole" text to compare against and nothing to claim.
                continue;
            };
            // Only a *strict prefix* of the whole text is a hard clip: that is
            // what ratatui does to a row it was handed too much of, and it is
            // the fingerprint the class leaves. A row that differs any other
            // way is not truncated — `table_tools_line` genuinely says more at
            // narrow widths, appending `+1 cols (w)` once a column has been
            // dropped — and this invariant makes no claim about it.
            if !reference.starts_with(text.as_str()) || text == reference {
                continue;
            }
            prop_assert!(
                ends_in_a_marker(text),
                "{kind:?} at {width}x{height} (detail={detail}): the {name} was cut \
                 with no marker.\n     shown: {text:?}\n     whole: {reference:?}"
            );
        }
    }
}

/// The concrete symptom the property generalises, kept as its own example.
///
/// The property says the class is closed; this says what the class looked like,
/// on the frame a reader can picture. Cloud Run has six tabs and the widest
/// hint, so it is where the pane ran out of room first:
///
/// ```text
/// │Cloud Run / billing-worker                   │
/// │[Summary] | Traffic | Revisions | Configurati│
/// │[ and ] tabs   n/p field   y copy   Esc/q ret│
/// ```
///
/// A fourth tab the operator could not name, a fifth and sixth they could not
/// see, and a hint stopping mid-word.
#[test]
fn the_cloud_run_detail_header_says_when_its_rows_are_cut() {
    let index = ResourceServiceKind::ALL
        .iter()
        .position(|kind| *kind == ResourceServiceKind::CloudRunServices)
        .expect("Cloud Run is a registered service");

    let rows = with_session(index, true, |session| {
        chrome_rows(&render(session, 132, 40))
    });

    for name in ["right pane row 2", "right pane row 3"] {
        let (_, text) = rows
            .iter()
            .find(|(other, _)| *other == name)
            .unwrap_or_else(|| panic!("{name} is not on the frame; rows: {rows:?}"));
        assert!(
            ends_in_a_marker(text),
            "the {name} is cut with no marker: {text:?}"
        );
    }
}

// ---------------------------------------------------------------------------
// FinOps
//
// The workspace was outside this file's range: the locator above walks the
// Resources layout's panes, and FinOps is a stack of full-width cards. Its
// Budgets card was clipping `(covers the whole billing account, not just this
// project)` down to `(cover` — the clause that says the figure above is not
// about this project, on a cost screen.
// ---------------------------------------------------------------------------

/// A FinOps card's content rows: everything inside a full-width block.
fn finops_rows(lines: &[String]) -> Vec<String> {
    lines
        .iter()
        .filter(|line| line.starts_with('│'))
        .map(|line| line.trim_matches('│').trim_end().to_string())
        .filter(|line| !line.is_empty())
        .collect()
}

fn build_finops_session() -> RuntimeSession {
    let config = Config {
        demo: true,
        ..Config::default()
    };
    let mut app = AppState::new();
    nuvem::services::runtime::apply_startup_config(&mut app, &config);
    let mut session = RuntimeSession::new_with_config_and_runner_for_test(
        config,
        app,
        Box::new(DemoCommandRunner::new()),
    );

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime");
    runtime.block_on(async {
        session.initialize().await;
        drain(&mut session).await;
        // FinOps derives its query locations from rows already listed, so the
        // two services it reads them from are visited first. Without this every
        // card is "not probed" and there is no content to clip.
        session
            .dispatch_action(Action::Workspace(WorkspaceAction::Resources))
            .await;
        for kind in [
            ResourceServiceKind::ComputeInstances,
            ResourceServiceKind::PersistentDisks,
        ] {
            session
                .dispatch_action(Action::Resources(ResourcesAction::SelectService(kind)))
                .await;
            drain(&mut session).await;
        }
        session
            .dispatch_action(Action::Workspace(WorkspaceAction::FinOps))
            .await;
        drain(&mut session).await;
    });
    session
}

thread_local! {
    static FINOPS: RefCell<Option<RuntimeSession>> = const { RefCell::new(None) };
}

fn with_finops<R>(act: impl FnOnce(&RuntimeSession) -> R) -> R {
    FINOPS.with(|cache| {
        let mut cache = cache.borrow_mut();
        act(cache.get_or_insert_with(build_finops_session))
    })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(24))]

    /// The same contract, on the FinOps workspace.
    ///
    /// Keyed by prefix rather than by position, unlike the Resources property
    /// above: how many cards are drawn changes with the height, so a row's index
    /// means different things at different sizes. A row that is a *strict
    /// prefix* of a line the uncut render shows was cut, and must say so; a row
    /// that differs any other way is not this invariant's business.
    #[test]
    fn a_finops_row_that_does_not_fit_says_so(
        width in 60u16..=200,
        height in prop::sample::select(REFERENCE_HEIGHTS.as_slice()),
    ) {
        let (shown, whole) = with_finops(|session| {
            (
                finops_rows(&render(session, width, height)),
                finops_rows(&render(session, UNCUT_WIDTH, height)),
            )
        });

        for row in &shown {
            if whole.iter().any(|reference| reference == row) {
                continue;
            }
            let cut = whole
                .iter()
                .any(|reference| reference.starts_with(row.as_str()) && reference != row);
            if !cut {
                continue;
            }
            prop_assert!(
                ends_in_a_marker(row),
                "at {width}x{height} a FinOps row was cut with no marker: {row:?}"
            );
        }
    }
}
