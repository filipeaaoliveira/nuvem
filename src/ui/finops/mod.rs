//! Renderer for the FinOps workspace.
//!
//! Body content only: the header, breadcrumb, operation status, and footer are
//! painted by `ui::render` around the `body_area` handed in here. Every string
//! rendered comes from `project_finops`, which has already rounded the money,
//! corrected its sign, and decided which figures may be shown at all.

use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::text::Line;
use ratatui::widgets::{List, ListItem, Paragraph};

use crate::app::projection::finops::{
    BudgetsNote, FinOpsCardProjection, FinOpsProjection, project_finops,
};
use crate::app::projection::{InventoryCardProjection, ResourceFreshness};
use crate::app::state::finops::CardState;
use crate::services::runtime::session::RuntimeSession;
use crate::ui::shared::components::{bordered_block, empty_state, fit_text};
use crate::ui::theme;

/// Rows shown per card before the list is cut. A savings card is a summary,
/// not a table: the operator opens Resources to act on any individual row.
const MAX_ROWS_PER_CARD: usize = 5;

pub fn render(frame: &mut Frame<'_>, area: Rect, session: &RuntimeSession, at: std::time::Instant) {
    // `area()` rather than two comparisons: one zero dimension is enough, and
    // the product says so without a boolean whose two halves cannot be told
    // apart by any test that draws into a real terminal.
    if area.area() == 0 {
        return;
    }

    let projection = project_finops(&session.app().root, at);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(0)])
        .split(area);

    let total = fit_text(&projection.total_line, usize::from(chunks[0].width));
    frame.render_widget(Paragraph::new(Line::raw(total)), chunks[0]);

    // The budget and inventory blocks are fixed slices off the bottom rather
    // than equal shares: each holds a handful of short lines, and giving them a
    // sixth of the screen apiece would take height from cards that have rows.
    let body = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(0),
            Constraint::Length(BUDGET_BLOCK_HEIGHT),
            Constraint::Length(INVENTORY_BLOCK_HEIGHT),
        ])
        .split(chunks[1]);

    render_card_band(frame, body[0], &projection);

    render_budgets(frame, body[1], &projection);
    render_inventory(frame, body[2], &projection);
}

/// Border plus one line: the inventory is a single summary row, not the Home
/// band. Its purpose here is the ticket's "parallel global resource
/// inventory" — a total to read the savings figures against, not a second
/// place to browse resources from.
const INVENTORY_BLOCK_HEIGHT: u16 = 3;

const INVENTORY_BLOCK_TITLE: &str = "Inventory";

fn render_inventory(frame: &mut Frame<'_>, area: Rect, projection: &FinOpsProjection) {
    if area.area() == 0 {
        return;
    }
    // Only services that have actually been listed contribute. An unlisted
    // service counts zero rows, and summing those into a total would state a
    // project size derived mostly from what nobody has opened yet.
    let listed: Vec<&InventoryCardProjection> = projection
        .inventory
        .iter()
        .filter(|card| card.freshness != ResourceFreshness::NotLoaded)
        .collect();

    let line = if listed.is_empty() {
        "nothing listed yet".to_string()
    } else {
        let total: usize = listed.iter().map(|card| card.total).sum();
        format!(
            "{total} resource(s) across {} of {} services listed",
            listed.len(),
            projection.inventory.len()
        )
    };

    frame.render_widget(
        List::new(vec![ListItem::new(Line::raw(line))])
            .block(bordered_block(INVENTORY_BLOCK_TITLE))
            .highlight_symbol(""),
        area,
    );
}

/// Border plus a title line plus three budget lines. Budgets are few and short.
const BUDGET_BLOCK_HEIGHT: u16 = 5;

const BUDGET_BLOCK_TITLE: &str = "Budgets";

fn render_budgets(frame: &mut Frame<'_>, area: Rect, projection: &FinOpsProjection) {
    if area.area() == 0 {
        return;
    }
    let mut lines: Vec<ListItem<'_>> = Vec::new();
    // Fitted for the reason `render_card`'s rows are: a budget line carries the
    // scope note last, and clipping it silently drops the clause saying the
    // amount covers the whole billing account rather than this project.
    let width = usize::from(area.width.saturating_sub(2));
    let row = |text: &str| ListItem::new(Line::raw(fit_text(text, width)));

    if let Some(note) = &projection.budgets_note {
        lines.push(row(&match note {
            BudgetsNote::NotProbed => "not probed".to_string(),
            BudgetsNote::Loading => "loading…".to_string(),
            BudgetsNote::None => "no budgets set on this billing account".to_string(),
            BudgetsNote::NoBillingAccount => {
                "no billing account is linked to this project".to_string()
            }
            BudgetsNote::Unavailable { headline, remedy } => {
                format!("unavailable: {headline} — {remedy}")
            }
        }));
    }

    for budget in &projection.budgets {
        let amount = budget.amount.as_deref().unwrap_or("no amount");
        // No gauge and no percentage-of-spend bar: the API returns no spend,
        // so there is no position to draw against the amount.
        let mut line = format!("{}  {amount}", budget.display_name);
        if !budget.thresholds.is_empty() {
            line.push_str(&format!("  alerts at {}", budget.thresholds));
        }
        if let Some(scope) = &budget.scope_note {
            line.push_str(&format!("  ({scope})"));
        }
        lines.push(row(&line));
    }

    if lines.is_empty() {
        lines.push(empty_state(BUDGET_BLOCK_TITLE));
    }

    frame.render_widget(
        List::new(lines)
            .block(bordered_block(BUDGET_BLOCK_TITLE))
            .highlight_symbol(""),
        area,
    );
}

/// Two border rows plus one line: below this a card carries nothing, and is
/// counted rather than drawn.
const MIN_CARD_HEIGHT: u16 = 3;

/// Draws as many cards as can carry a row, and counts the rest.
///
/// The band used to split its height into equal ratios and let `render_card`
/// decline only on a zero-height area, so at 80x24 three of four cards were
/// drawn as borders around nothing while the header above them read `Potential
/// savings: USD 43.45/mo`. An empty card does not read as "too small
/// to show" — it reads as "nothing to report", which is the opposite of what
/// the header says, and a cost screen that looks like good news when it is not
/// is the failure `services::resources::finops` opens by warning about.
///
/// Same shape as Home's inventory band: what the band cannot paint, it counts.
fn render_card_band(frame: &mut Frame<'_>, area: Rect, projection: &FinOpsProjection) {
    let total = projection.cards.len();
    // No `total == 0 || area.height == 0` guard. `total` is one card per
    // `RecommenderId::ALL`, so it is never zero; and a zero-height band already
    // falls out correctly — nothing is paintable, everything is counted, and the
    // layout hands back zero-height rects that ratatui draws nothing into. The
    // mutation gate is what said so, by surviving `replace || with &&` on a
    // condition neither half of which can change an answer.
    let rows = usize::from(area.height);
    let per_card = usize::from(MIN_CARD_HEIGHT);
    let mut painted = (rows / per_card).min(total);
    // The count needs a row of its own, and taking it may cost the last card
    // the row that made it worth drawing.
    if painted < total {
        painted = (rows.saturating_sub(1) / per_card).min(total);
    }
    let missing = total - painted;

    let marker_rows = u16::from(missing > 0);
    let areas = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(marker_rows)])
        .split(area);

    // No `painted > 0` guard, for the reason the two above it went: with nothing
    // paintable the constraint list is empty, the split yields no rects, and the
    // zip below draws nothing — the same answer the guard gave, which is why the
    // mutation gate could survive `replace > with >=` on it. `share`'s `max(1)`
    // is what keeps the ratio's divisor safe, and that is not a branch's job.
    let share = u32::try_from(painted).unwrap_or(1).max(1);
    let constraints: Vec<Constraint> = (0..painted).map(|_| Constraint::Ratio(1, share)).collect();
    let card_areas = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(areas[0]);
    for (card, card_area) in projection.cards.iter().take(painted).zip(card_areas.iter()) {
        render_card(frame, *card_area, card);
    }

    // No `missing > 0` guard here: `marker_rows` above is what asked for the
    // row, and when nothing is missing the layout hands back a zero-height rect
    // that ratatui draws nothing into. A second check reads as a safety net and
    // is not one — the mutation gate survived `replace > with >=` on it, because
    // nothing could tell the two apart. Same reasoning, and the same wording, as
    // the savings teaser in `ui::home::render_home`.
    frame.render_widget(
        Paragraph::new(Line::raw(fit_text(
            &format!("+{missing} more"),
            usize::from(areas[1].width),
        )))
        .style(theme::muted()),
        areas[1],
    );
}

fn render_card(frame: &mut Frame<'_>, area: Rect, card: &FinOpsCardProjection) {
    if area.height == 0 {
        return;
    }
    let block = bordered_block(card.title.clone());
    // Fitted, not handed to the widget whole: a bare `Line` is clipped at the
    // card's edge with nothing to say it was, so the Budgets row read
    // `... alerts at 100%  (cover` — losing the clause that says the figure is
    // not about this project, on a cost screen.
    let width = usize::from(area.width.saturating_sub(2));
    let row = |text: &str| ListItem::new(Line::raw(fit_text(text, width)));

    let mut lines: Vec<ListItem<'_>> = Vec::new();
    // The dropped-row count rides the headline rather than trailing the list.
    // A card is one of four in a vertical stack, so at ordinary terminal
    // heights a trailing line is clipped by the block — and a count that
    // disappears exactly when there is something to count is worse than none.
    let mut head = headline(card);
    if card.rows.len() > MAX_ROWS_PER_CARD {
        head.push_str(&format!(
            "  (showing {MAX_ROWS_PER_CARD} of {})",
            card.rows.len()
        ));
    }
    lines.push(row(&head));
    if let Some(coverage) = &card.coverage_line {
        lines.push(row(coverage));
    }
    if let Some(remedy) = &card.remedy {
        lines.push(row(remedy));
    }
    for entry in card.rows.iter().take(MAX_ROWS_PER_CARD) {
        let figure = entry.monthly.as_deref().unwrap_or("no monthly figure");
        let priority = entry
            .priority
            .as_deref()
            .map(|value| format!(" [{value}]"))
            .unwrap_or_default();
        lines.push(row(&format!("  {figure}{priority}  {}", entry.description)));
    }
    if lines.is_empty() {
        lines.push(empty_state("no recommendations"));
    }

    frame.render_widget(List::new(lines).block(block).highlight_symbol(""), area);
}

/// The card's one-line verdict. Every non-Ready state names itself rather than
/// leaving a blank that reads as "nothing to save".
fn headline(card: &FinOpsCardProjection) -> String {
    match card.state {
        CardState::NotProbed => "not probed".to_string(),
        CardState::Loading => match &card.total {
            Some(total) => format!("loading… {total}/mo so far"),
            None => "loading…".to_string(),
        },
        CardState::Ready => match &card.total {
            Some(total) => format!("{total}/mo"),
            None => "no monthly figure".to_string(),
        },
        CardState::Empty => "no recommendations".to_string(),
        CardState::Unavailable(kind) => format!("unavailable: {}", kind.label()),
    }
}
