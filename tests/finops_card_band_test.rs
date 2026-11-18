//! Every FinOps card that is drawn carries a content row.
//!
//! The band split its height into equal ratios and `render_card` declined only
//! on a zero-height area, so at 80x24 three of four cards were drawn as borders
//! around nothing — while the header above them read `Potential savings: USD
//! 43.45/mo`. An empty card does not read as "too small to show"; it reads as
//! "nothing to report", which is the opposite of what the header says. A cost
//! screen that looks like good news when it is not is the failure
//! `services/resources/finops.rs` opens by warning about.
//!
//! Home solved the same problem: give the band's rows to the cards
//! that can use them and count the ones left out.

use nuvem::Config;
use nuvem::app::{Action, AppState, ResourcesAction, WorkspaceAction};
use nuvem::services::demo::DemoCommandRunner;
use nuvem::services::runtime::session::RuntimeSession;
use nuvem_domain::gcp::finops::RecommenderId;
use nuvem_domain::gcp::resource::ResourceServiceKind;
use ratatui::{Terminal, backend::TestBackend};

async fn drain(session: &mut RuntimeSession) {
    for _ in 0..64 {
        match tokio::time::timeout(
            std::time::Duration::from_millis(25),
            session.process_next_effect_result(),
        )
        .await
        {
            Ok(true) => {}
            _ => break,
        }
    }
}

/// A FinOps workspace with every card populated, which needs the two services
/// its query locations are derived from listed first.
async fn populated_finops() -> RuntimeSession {
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
    session.initialize().await;
    drain(&mut session).await;
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
    session
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

/// The card titles drawn, paired with how many content rows each got.
fn drawn_cards(rendered: &[String]) -> Vec<(String, usize)> {
    let mut cards = Vec::new();
    for kind in RecommenderId::ALL {
        let title = kind.label();
        let Some(top) = rendered
            .iter()
            .position(|line| line.contains(title) && line.contains('┌'))
        else {
            continue;
        };
        let rows = rendered[top + 1..]
            .iter()
            .take_while(|line| !line.contains('└'))
            .filter(|line| !line.trim_matches(['│', ' ']).is_empty())
            .count();
        cards.push((title.to_string(), rows));
    }
    cards
}

#[tokio::test]
async fn every_drawn_finops_card_carries_a_content_row() {
    let session = populated_finops().await;
    for height in 20u16..=40 {
        let rendered = lines(&session, 100, height);
        for (title, rows) in drawn_cards(&rendered) {
            assert!(
                rows > 0,
                "at 100x{height} the {title:?} card is drawn as borders around \
                 nothing; rendered:\n{}",
                rendered.join("\n")
            );
        }
    }
}

/// The cards left out are accounted for, so the operator is not left thinking
/// they saw all four.
#[tokio::test]
async fn the_finops_cards_that_do_not_fit_are_counted() {
    let session = populated_finops().await;
    let rendered = lines(&session, 100, 24);
    let drawn = drawn_cards(&rendered).len();
    let total = RecommenderId::ALL.len();
    if drawn == total {
        return;
    }

    let missing = total - drawn;
    let text = rendered.join("\n");
    assert!(
        text.contains(&format!("+{missing} more")),
        "{drawn} of {total} cards are drawn and the rest are not counted; \
         rendered:\n{text}"
    );
}

/// The header and the cards agree: a named savings figure has a card behind it.
#[tokio::test]
async fn a_named_savings_figure_has_a_card_showing_where_it_comes_from() {
    let session = populated_finops().await;
    for height in 20u16..=40 {
        let rendered = lines(&session, 100, height);
        let text = rendered.join("\n");
        if !text.contains("Potential savings: USD") {
            continue;
        }
        assert!(
            drawn_cards(&rendered).iter().any(|(_, rows)| *rows > 0),
            "the header names a savings figure and no card shows where it comes \
             from, at 100x{height}; rendered:\n{text}"
        );
    }
}

/// The card titles drawn, in registry order.
fn drawn_titles(rendered: &[String]) -> Vec<&'static str> {
    RecommenderId::ALL
        .into_iter()
        .map(RecommenderId::label)
        .filter(|title| {
            rendered
                .iter()
                .any(|line| line.contains(title) && line.contains('┌'))
        })
        .collect()
}

/// More rows never means fewer cards.
///
/// The band's painted count is `rows / MIN_CARD_HEIGHT`, and the shape of that
/// is what this pins: division grows with height, so the count can only ever go
/// up. The mutation gate asked for it — `replace / with %` survived every other
/// test here, because a count that oscillates with height still leaves the drawn
/// cards well formed and still counts the rest. What it breaks is the only thing
/// the arithmetic is for.
#[tokio::test]
async fn more_rows_never_means_fewer_cards() {
    let session = populated_finops().await;
    let mut previous = 0usize;
    for height in 12u16..=60 {
        let drawn = drawn_titles(&lines(&session, 120, height)).len();
        assert!(
            drawn >= previous,
            "at 120x{height} the band drew {drawn} cards, fewer than the \
             {previous} it drew one row shorter"
        );
        previous = drawn;
    }
    assert_eq!(
        previous,
        RecommenderId::ALL.len(),
        "a tall terminal must end up drawing every card"
    );
}

/// A band with room for every card says nothing about missing ones.
///
/// The marker costs a row, so reserving it when nothing is missing takes that
/// row from a card for no reason — and `+0 more` is a claim about a set that is
/// empty. Three of the gate's survivors live here: both `missing > 0` tests and
/// the `painted < total` that decides whether to recompute for the marker's row
/// at all.
#[tokio::test]
async fn a_band_with_room_for_every_card_shows_no_overflow_marker() {
    let session = populated_finops().await;
    // From 27, the first height at which all four cards fit — not from a
    // comfortably tall one. Everything interesting about the band's arithmetic
    // happens at that boundary: above it the count saturates at `total` and a
    // wrong computation is indistinguishable from a right one, which is exactly
    // how `replace / with %` survived a sweep that started at 40.
    for height in 27u16..=60 {
        let rendered = lines(&session, 120, height);
        let drawn = drawn_titles(&rendered);
        assert_eq!(
            drawn.len(),
            RecommenderId::ALL.len(),
            "every card fits at 120x{height}; drawn: {drawn:?}"
        );

        let text = rendered.join("\n");
        assert!(
            !text.contains("+0 more"),
            "an empty overflow set must not be announced at 120x{height}; \
             rendered:\n{text}"
        );
        assert!(
            !text.contains(" more"),
            "nothing is missing at 120x{height}, so no marker belongs on screen; \
             rendered:\n{text}"
        );
    }
}
