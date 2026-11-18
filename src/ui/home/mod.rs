use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, RenderDirection, Sparkline};

use crate::app::projection::{
    GcloudHealthProjection, HomeOverviewProjection, InventoryCardProjection,
    OperationLineProjection, ResourceFreshness, project_home_overview,
};
use crate::app::state::health::ChangeDirection;
use crate::app::state::operations::OperationOutcome;
use crate::services::runtime::session::RuntimeSession;
use crate::ui::shared::components::{bordered_block, fit_spans, fit_text};
use crate::ui::shared::table::display_width;
use crate::ui::theme;
use nuvem_domain::gcp::resource::ResourceServiceKind;

/// The fewest rows the context/health card can take: two borders plus the four
/// context values and the one gcloud status line every Home has.
///
/// Exact for a card with no gcloud detail preview, which is every healthy Home.
/// It used to be the card's *only* height, fixed at 8 — one row wider than that
/// — so the sixth content row was reserved whether or not there was a detail
/// preview to put in it, and every healthy Home carried a blank row at every
/// width. On a short terminal that row was the inventory's.
pub const CONTEXT_CARD_MIN_HEIGHT: u16 = 7;
/// Two border rows plus one line of content: below this a card carries nothing
/// and is not drawn at all, so it is also the floor the inventory band must
/// leave the operations card.
pub const MIN_CARD_HEIGHT: u16 = 3;
/// The Inventory block's own two border rows. The band asks for these plus one
/// row per service and settles for less where the terminal cannot spare it —
/// see `render_inventory_row`, which then says how many services it could not
/// draw.
const INVENTORY_BORDER_LINES: u16 = 2;
/// Columns between the widest service label and whatever follows it. A
/// separator rather than padding: every narrower label is padded out to the
/// same column, so the band reads as a table.
const SERVICE_LABEL_GAP: usize = 2;
/// Minimum area width for the context/health card to spend a second line on the
/// sanitized detail preview under its gcloud status; narrower areas show the
/// status line only (no full overlay panel — see ARCHITECTURE.md's Diagnostics
/// classifier section). Service lines do not consult it: they fit the preview
/// onto the row or drop it — see `inventory_line`.
const DETAIL_PREVIEW_MIN_WIDTH: u16 = 36;
/// Columns one service line reserves, at its right edge, for its count history
/// sparkline.
///
/// Public because a rendered-buffer test has to know which columns are the
/// spark to read bar heights out of them: a blank cell is a zero-height bar, so
/// a test scanning the whole row for bar glyphs would read every space in the
/// label as one.
pub const SPARKLINE_WIDTH: u16 = 12;
/// Area width below which the spark column is not reserved at all.
///
/// The inventory line's own content comes first: label, freshness and — when
/// there is one — a problem headline the operator acts on. Spending a fifth of
/// an 80-column band on a trend while truncating the headline that says what
/// broke would be the wrong trade, so on a narrow terminal the spark is what
/// goes, silently and entirely. It is an enrichment of a line that is complete
/// without it, which is exactly why it may vanish without the band having to
/// say so — unlike a dropped service, which `render_inventory_row` counts.
const SPARKLINE_MIN_AREA_WIDTH: u16 = 96;
/// The caption over the spark column, in the Inventory border.
///
/// Public so a rendered-buffer test names the same text the renderer draws.
pub const SPARKLINE_CAPTION: &str = "Count trend";
/// What a line carries beside a count that has just gone up.
///
/// Public so a rendered-buffer test names the same glyph the renderer draws
/// rather than a copy of it that can drift.
pub const DELTA_ROSE_MARKER: &str = "\u{25b2}";
/// ...and one that has just gone down.
pub const DELTA_FELL_MARKER: &str = "\u{25bc}";

/// Renders the Home overview dashboard: context health, inventory cards, and
/// recent operations. Reflects already-loaded state only; never triggers loads.
pub fn render_home(
    frame: &mut Frame<'_>,
    area: Rect,
    session: &RuntimeSession,
    at: std::time::Instant,
) {
    if area.width == 0 || area.height == 0 {
        return;
    }

    let overview = project_home_overview(&session.app().root, at);

    // Grows to fit every service, but never at the operations card's expense:
    // squeezing that out to gain an inventory line trades one silent omission
    // for another. What the band does not get, `render_inventory_row` counts.
    let wanted = u16::try_from(overview.inventory.len())
        .unwrap_or(u16::MAX)
        .saturating_add(INVENTORY_BORDER_LINES);
    let context_card_height = context_card_height(&overview, area.width);
    let available = area.height.saturating_sub(context_card_height);
    // The band leaves the operations card its minimum only when what is left
    // after that is enough to draw the card. Below that the reservation was
    // spent on a card that declines to paint: at 60x18 the band was floored to
    // `MIN_CARD_HEIGHT` anyway, the operations card was left two rows — under
    // its own minimum — and drew nothing into them, so two rows of a screen
    // that had none to spare stayed blank while the inventory named no service
    // at all.
    let reserved = if available >= MIN_CARD_HEIGHT.saturating_mul(2) {
        MIN_CARD_HEIGHT
    } else {
        0
    };
    let spare = available.saturating_sub(reserved);
    let inventory_height = wanted.min(spare.max(MIN_CARD_HEIGHT));

    // One line, and only when there is one to show. A permanently reserved
    // row would be an empty gap on every Home that has no savings figure.
    let teaser_height = u16::from(overview.savings_teaser.is_some());

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(context_card_height),
            Constraint::Length(inventory_height),
            Constraint::Length(teaser_height),
            Constraint::Min(0),
        ])
        .split(area);

    let context_area = chunks[0];
    let inventory_area = chunks[1];
    let teaser_area = chunks[2];
    let operations_area = chunks[3];

    // No height guard: `savings_teaser` being `Some` is what asked for the row,
    // and on a terminal too short to grant it the layout hands back a
    // zero-height rect that ratatui draws nothing into. A `> 0` check here
    // reads as a safety net and is not one — every value a u16 can hold
    // satisfies it or is the no-op case anyway.
    if let Some(teaser) = &overview.savings_teaser {
        let text = fit_text(teaser, usize::from(teaser_area.width));
        frame.render_widget(Paragraph::new(Line::raw(text)), teaser_area);
    }

    if context_area.height >= MIN_CARD_HEIGHT {
        render_context_card(frame, context_area, &overview);
    }

    if inventory_area.height >= MIN_CARD_HEIGHT && inventory_area.width >= 2 {
        render_inventory_row(frame, inventory_area, &overview);
    }

    if operations_area.height >= MIN_CARD_HEIGHT {
        render_operations_card(frame, operations_area, &overview);
    }
}

/// The rows this card will fill: two borders, the four context values, and
/// however many lines the gcloud status takes at this width.
///
/// Derived from the same [`gcloud_status_lines`] the card renders, rather than
/// restated, so the height and the content cannot disagree about whether the
/// detail preview is there. The card's height therefore moves by one row when a
/// gcloud diagnostic appears — a visible shift, and the deliberate price of not
/// holding a blank row open on every Home that has no diagnostic at all.
fn context_card_height(overview: &HomeOverviewProjection, area_width: u16) -> u16 {
    let content = CONTEXT_VALUE_LINES
        + u16::try_from(gcloud_status_lines(&overview.gcloud, area_width, usize::MAX).len())
            .unwrap_or(1);
    CARD_BORDER_LINES.saturating_add(content)
}

/// Configuration, account, region/zone and project: the four values the context
/// card always lists, before the gcloud status.
const CONTEXT_VALUE_LINES: u16 = 4;
/// A bordered card's top and bottom rows.
const CARD_BORDER_LINES: u16 = 2;

fn render_context_card(frame: &mut Frame<'_>, area: Rect, overview: &HomeOverviewProjection) {
    let width = usize::from(area.width.saturating_sub(2));
    let context = &overview.context;

    let configuration = context.configuration.as_deref().unwrap_or("-");
    let account = context.account.as_deref().unwrap_or("-");
    let project = context.project.as_deref().unwrap_or("-");
    let region_zone = match (context.region.as_deref(), context.zone.as_deref()) {
        (Some(region), Some(zone)) => format!("{region}/{zone}"),
        (Some(region), None) => region.to_string(),
        (None, Some(zone)) => format!("-/{zone}"),
        (None, None) => "-".to_string(),
    };

    let mut lines = vec![
        Line::raw(fit_text(&format!("Configuration: {configuration}"), width)),
        Line::raw(fit_text(&format!("Account: {account}"), width)),
        Line::raw(fit_text(&format!("Region/Zone: {region_zone}"), width)),
        Line::raw(fit_text(&format!("Project: {project}"), width)),
    ];
    lines.extend(gcloud_status_lines(&overview.gcloud, area.width, width));

    frame.render_widget(Paragraph::new(lines).block(bordered_block("Context")), area);
}

/// Renders gcloud health as the headline (`"gcloud: unavailable - {label} —
/// {user action}"`, AC #5) plus, only when `area_width` allows it (AC #6,
/// no full overlay panel), a second muted line with the sanitized bounded
/// detail preview.
fn gcloud_status_lines(
    gcloud: &GcloudHealthProjection,
    area_width: u16,
    width: usize,
) -> Vec<Line<'static>> {
    let (text, style) = if gcloud.probing {
        ("gcloud: checking...".to_string(), theme::status_loading())
    } else if let Some(problem) = gcloud.problem.as_deref() {
        // A problem can be present even when `available == Some(true)` (e.g.
        // an empty/failed `gcloud auth list`); always surface it and never
        // use the healthy/green style when a problem is present (AC #5).
        (
            format!("gcloud: unavailable - {problem}"),
            theme::status_error(),
        )
    } else {
        match gcloud.available {
            None => ("gcloud: unknown".to_string(), theme::muted()),
            Some(true) => {
                let account = gcloud.account.as_deref().unwrap_or("-");
                (
                    format!("gcloud: available ({account})"),
                    theme::status_healthy(),
                )
            }
            Some(false) => (
                "gcloud: unavailable - unavailable".to_string(),
                theme::status_error(),
            ),
        }
    };
    let mut lines = vec![Line::from(Span::styled(fit_text(&text, width), style))];
    if area_width >= DETAIL_PREVIEW_MIN_WIDTH
        && let Some(detail) = &gcloud.detail_preview
    {
        lines.push(Line::from(Span::styled(
            fit_text(detail, width),
            theme::muted(),
        )));
    }
    lines
}

/// Renders the inventory as one line per service inside a single block.
///
/// Not one bordered card per service laid into a fixed column split: that shape
/// drew two cards and dropped every later one on the floor, and it cannot hold
/// seven services at 80 columns however the split is tuned. Where the block is
/// too short for every line, the last line is spent saying how many services
/// are missing — the renderer may run out of room, but it may not go quiet
/// about it.
fn render_inventory_row(frame: &mut Frame<'_>, area: Rect, overview: &HomeOverviewProjection) {
    let spark_width = if area.width >= SPARKLINE_MIN_AREA_WIDTH {
        SPARKLINE_WIDTH
    } else {
        0
    };
    // The spark column is taken out of the text budget rather than drawn over
    // it: a line fitted to the full width and then painted on would have its
    // last twelve columns of text silently replaced by bars, which is how a
    // truncated problem headline turns into a wrong one.
    let width = usize::from(area.width.saturating_sub(2).saturating_sub(spark_width));
    let content_height = usize::from(area.height.saturating_sub(2));
    let total = overview.inventory.len();

    // Painted plus counted equals the registry, by construction rather than by
    // a loop whose exit condition has to be re-derived: the overflow line costs
    // one service row whenever there is an overflow at all.
    let painted = if total <= content_height {
        total
    } else {
        content_height.saturating_sub(1)
    };

    let mut lines: Vec<Line<'static>> = overview
        .inventory
        .iter()
        .take(painted)
        .map(|card| inventory_line(card, width))
        .collect();

    let missing = total - painted;
    if missing > 0 {
        lines.push(Line::from(Span::styled(
            fit_text(&format!("+{missing} more"), width),
            theme::muted(),
        )));
    }

    // The spark column is captioned in the border above it whenever it holds a
    // spark, or its bars are coloured blocks with nothing to say what they
    // measure.
    let mut block = bordered_block("Inventory");
    let any_spark = spark_width > 0
        && overview
            .inventory
            .iter()
            .take(painted)
            .any(|card| !card.history.is_empty());
    if any_spark {
        block = block.title(Line::from(SPARKLINE_CAPTION).right_aligned());
    }
    frame.render_widget(Paragraph::new(lines).block(block), area);

    render_inventory_sparklines(frame, area, overview, painted, spark_width);
}

/// Draws each painted service's history in the spark column, over the
/// paragraph already rendered into `area`.
///
/// One `Sparkline` per row rather than one widget for the band: a sparkline is
/// a widget over a `Rect`, and each service's series is its own. `painted` is
/// passed in rather than recomputed so the sparks cannot disagree with the
/// lines about which services made it onto the screen — a spark under the
/// `+N more` marker would attribute one service's trend to the count of
/// services that are missing.
///
/// A service with an empty history is skipped entirely rather than drawn as a
/// flat line: the projection empties the series precisely when there is nothing
/// honest to say, and a row of ground-level bars would say "steady".
fn render_inventory_sparklines(
    frame: &mut Frame<'_>,
    area: Rect,
    overview: &HomeOverviewProjection,
    painted: usize,
    spark_width: u16,
) {
    if spark_width == 0 {
        return;
    }
    for (index, card) in overview.inventory.iter().take(painted).enumerate() {
        if card.history.is_empty() {
            continue;
        }
        let Ok(offset) = u16::try_from(index) else {
            return;
        };
        let spark = Rect {
            x: area.x + area.width - 1 - spark_width,
            y: area.y + 1 + offset,
            width: spark_width,
            height: 1,
        };
        // Drawn newest-first from the right edge. Two things fall out of that
        // and both are the behaviour we want: a series shorter than the column
        // leaves its *left* cells blank, so the newest sample is always in the
        // same place whatever the history's length; and a series longer than
        // the column loses its *oldest* samples rather than its newest, which
        // is the opposite of what a left-to-right spark would drop.
        let newest_first: Vec<u64> = card.history.iter().rev().copied().collect();
        // No `max`: the series is scaled against its own peak, so the spark
        // reads as a shape and not as a magnitude. The magnitude is the count
        // already on the row, and a fixed ceiling would draw a service with
        // four rows and one with four thousand as the same flat line along the
        // bottom.
        frame.render_widget(
            Sparkline::default()
                .data(newest_first)
                .direction(RenderDirection::RightToLeft)
                .style(theme::status_healthy()),
            spark,
        );
    }
}

/// One service's inventory line: label, freshness, then either its status
/// breakdown or its problem headline.
///
/// The headline is never dropped in favour of the counts (AC #5), and the
/// sanitized detail preview is appended only where the whole of it still fits
/// after the headline (AC #6). That fit test replaces the old per-card
/// `DETAIL_PREVIEW_MIN_WIDTH` guard rather than joining it: on a shared line
/// the label, freshness and headline already spend far more than that minimum,
/// so an area narrow enough to fail the guard can never pass the fit test, and
/// keeping both would be a condition no test could ever exercise. Raw stderr
/// has no path here: `detail_preview` is sanitized upstream.
/// Column budget a service line reserves for its label, so freshness and any
/// problem headline start at the same column on every row.
///
/// Derived from the registry rather than restated as a number. `{:<N}` pads
/// but never truncates, so a label wider than a hand-picked `N` runs straight
/// into the text beside it and loses the gap entirely — which is what a
/// hard-coded 18 did to "Pub/Sub subscriptions" (21 columns) once that service
/// was added, while "BigQuery datasets" at 17 was left with a single space.
/// The number was already at its limit when it was written, and
/// nothing tied it to the labels it was budgeting for.
///
/// Deriving it means the next service widens the column instead of colliding
/// with it. `project_inventory` builds one card per `ResourceServiceKind::ALL`
/// with `service_label = kind.label()`, so this is the exact set of labels the
/// band paints, not an upper bound on it.
fn service_label_width() -> usize {
    ResourceServiceKind::ALL
        .into_iter()
        .map(|kind| usize::from(display_width(kind.label())))
        .max()
        .unwrap_or(0)
        .saturating_add(SERVICE_LABEL_GAP)
}

fn inventory_line(card: &InventoryCardProjection, width: usize) -> Line<'static> {
    let (freshness_text, freshness_style) = match card.freshness {
        ResourceFreshness::NotLoaded => ("Not loaded".to_string(), theme::muted()),
        ResourceFreshness::HealthyEmpty => ("Loaded (empty)".to_string(), theme::status_healthy()),
        ResourceFreshness::Loaded => (format!("{} loaded", card.total), theme::status_healthy()),
        ResourceFreshness::Stale => ("Stale".to_string(), theme::status_stale()),
    };

    let label = format!(
        "{:<width$}",
        card.service_label,
        width = service_label_width()
    );
    let mut used = label.chars().count() + freshness_text.chars().count();
    let mut spans = vec![
        Span::raw(label),
        Span::styled(freshness_text, freshness_style),
    ];

    // Directly after the count, because it is a statement about that count.
    // Two columns, and only while the change is recent: `delta` is already
    // `None` once the projection's window has passed, so nothing here has to
    // ask what time it is.
    if let Some(direction) = card.delta {
        let marker = match direction {
            ChangeDirection::Rose => DELTA_ROSE_MARKER,
            ChangeDirection::Fell => DELTA_FELL_MARKER,
        };
        let text = format!(" {marker}");
        used += text.chars().count();
        spans.push(Span::styled(text, theme::delta()));
    }

    if let Some(problem) = &card.problem {
        let mut text = format!("  {problem}");
        // A truncated preview spends columns to say nothing, so it is appended
        // only when all of it fits: on one shared line the headline is what the
        // operator acts on, so it gets the row's columns first and the preview
        // never takes any of them.
        if let Some(detail) = &card.detail_preview {
            let appended = format!(" ({detail})");
            if used + text.chars().count() + appended.chars().count() <= width {
                text.push_str(&appended);
            }
        }
        spans.push(Span::styled(text, theme::status_error()));
    } else if !card.status_counts.is_empty() {
        let counts = card
            .status_counts
            .iter()
            .map(|(status, count)| format!("{status} {count}"))
            .collect::<Vec<_>>()
            .join("  ");
        spans.push(Span::raw(format!("  {counts}")));
    }

    fit_spans(spans, width)
}

fn render_operations_card(frame: &mut Frame<'_>, area: Rect, overview: &HomeOverviewProjection) {
    let width = usize::from(area.width.saturating_sub(2));
    let content_height = usize::from(area.height.saturating_sub(2));

    let lines: Vec<Line<'static>> = if overview.operations.is_empty() {
        vec![Line::from(Span::styled(
            "No recent operations",
            theme::muted(),
        ))]
    } else {
        let mut lines = Vec::new();
        for operation in &overview.operations {
            lines.push(operation_line(operation, width));
            if let Some(detail) = &operation.detail {
                lines.push(Line::from(Span::styled(
                    fit_text(&format!("  {detail}"), width),
                    theme::muted(),
                )));
            }
            if lines.len() >= content_height {
                break;
            }
        }
        lines
    };

    let mut lines = lines;
    lines.truncate(content_height);

    frame.render_widget(
        Paragraph::new(lines).block(bordered_block("Recent operations")),
        area,
    );
}

/// Styles a recent-operation line by its outcome. The match is exhaustive on
/// purpose: only `Pending` is work in progress, so a `_` fallback would hand
/// the loading style to any outcome added later and paint a settled operation
/// as still running — the cancellation defect, at the pixel level. Making a new
/// outcome a compile error forces that choice to be made deliberately.
fn operation_line(operation: &OperationLineProjection, width: usize) -> Line<'static> {
    let style = match operation.outcome {
        OperationOutcome::Pending => theme::status_loading(),
        OperationOutcome::Succeeded => theme::status_healthy(),
        OperationOutcome::Failed => theme::status_error(),
        // Terminal and nothing left for the operator to act on.
        OperationOutcome::Cancelled => theme::muted(),
        // Terminal, but the operator must go re-check the resource's real
        // state, so it warns rather than fading into the background.
        // `status_stale` is the only role that carries "what you see may not
        // match reality, go refresh" — `status_error` would claim a failure we
        // cannot prove. Accepted trade-off: in the default theme it is
        // `LightYellow` next to `status_loading`'s `Yellow`, close enough on
        // some terminals to be misread as still-in-flight at a glance. The
        // state word on the line disambiguates, and a new palette entry to buy
        // hue separation would cost every theme a role that means nothing else.
        OperationOutcome::Unknown => theme::status_stale(),
    };
    let text = format!("{}  {}", operation.outcome.label(), operation.label);
    Line::from(Span::styled(fit_text(&text, width), style))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::state::root::RootState;
    use nuvem_domain::gcp::resource::ResourceServiceKind;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::layout::Rect;

    fn painted(area: Rect, overview: &HomeOverviewProjection) -> String {
        let mut terminal =
            Terminal::new(TestBackend::new(area.width, area.height)).expect("test terminal");
        terminal
            .draw(|frame| render_inventory_row(frame, area, overview))
            .expect("draw");
        let buffer = terminal.backend().buffer();
        (0..buffer.area.height)
            .map(|y| {
                (0..buffer.area.width)
                    .map(|x| buffer.cell((x, y)).expect("cell").symbol().to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// A gcloud health projection carrying a detail preview, which is the only
    /// field the width gate below decides about.
    fn gcloud_with_detail(detail: &str) -> GcloudHealthProjection {
        GcloudHealthProjection {
            available: Some(true),
            account: Some("dev@example.com".to_string()),
            problem: None,
            user_action: None,
            detail_preview: Some(detail.to_string()),
            probing: false,
        }
    }

    /// The detail preview is a second line, and it is spent only where there
    /// are columns to spend on it: below `DETAIL_PREVIEW_MIN_WIDTH` the status
    /// line itself is already the whole budget, and a technical detail
    /// squeezed in beside it would push out the account the operator is
    /// actually looking for.
    ///
    /// Both sides are asserted, one column apart, because a comparison is only
    /// pinned by the pair — a test at one width cannot tell `>=` from `<`, and
    /// this one arrived as a surviving mutant that every other home-render
    /// test passed straight over.
    #[test]
    fn the_detail_preview_appears_only_from_the_minimum_width_up() {
        let gcloud = gcloud_with_detail("PERMISSION_DENIED on compute.instances.list");

        let at_minimum = gcloud_status_lines(&gcloud, DETAIL_PREVIEW_MIN_WIDTH, 80);
        assert_eq!(
            at_minimum.len(),
            2,
            "at exactly the minimum width the preview is affordable"
        );

        let below_minimum = gcloud_status_lines(&gcloud, DETAIL_PREVIEW_MIN_WIDTH - 1, 80);
        assert_eq!(
            below_minimum.len(),
            1,
            "one column narrower, only the status line fits"
        );
    }

    /// With nothing to preview the width is irrelevant: a wide pane must not
    /// grow an empty second line.
    #[test]
    fn a_missing_detail_preview_adds_no_line_however_wide_the_pane() {
        let mut gcloud = gcloud_with_detail("unused");
        gcloud.detail_preview = None;

        assert_eq!(
            gcloud_status_lines(&gcloud, DETAIL_PREVIEW_MIN_WIDTH * 4, 80).len(),
            1
        );
    }

    /// A card with everything the line can carry, so the width accounting has
    /// something to spend columns on.
    fn card_with(delta: Option<ChangeDirection>, detail: &str) -> InventoryCardProjection {
        InventoryCardProjection {
            kind: ResourceServiceKind::ComputeInstances,
            service_label: "GCE instances",
            freshness: ResourceFreshness::Loaded,
            total: 3,
            status_counts: Vec::new(),
            problem: Some("API disabled".to_string()),
            user_action: None,
            detail_preview: Some(detail.to_string()),
            history: Vec::new(),
            delta,
        }
    }

    /// The marker costs the line the columns it occupies, and the detail
    /// preview is what gives them up.
    ///
    /// The preview is appended only when the whole of it still fits, so a width
    /// that fits it without the marker must drop it with one. Without this the
    /// marker's contribution to the running width is invisible: nothing else
    /// reads `used`, so an accounting slip would let a preview overrun the row
    /// and be cut mid-word by `fit_spans` — the truncated-preview-that-says-
    /// nothing this line is built to avoid.
    #[test]
    fn the_change_marker_costs_the_line_the_columns_it_takes() {
        let detail = "code 7";
        let without = inventory_line(&card_with(None, detail), usize::MAX);
        let width = Line::from(without.spans.clone()).width();

        let unmarked = inventory_line(&card_with(None, detail), width);
        let marked = inventory_line(&card_with(Some(ChangeDirection::Rose), detail), width);

        assert!(
            line_text(&unmarked).contains(detail),
            "the preview fits exactly at this width with no marker; got {:?}",
            line_text(&unmarked)
        );
        // Absent *whole*, not cut short. A preview the accounting let through
        // and `fit_spans` then trimmed would satisfy a `!contains(detail)`
        // check while still spending the row's last columns on half a
        // parenthesis — which is the outcome this accounting exists to prevent,
        // so the assertion has to be able to tell the two apart.
        assert!(
            !line_text(&marked).contains('('),
            "the preview must be dropped whole rather than let through and trimmed; got {:?}",
            line_text(&marked)
        );
    }

    /// ...and it costs only those columns. A line with room for both keeps
    /// both, which is what stops the accounting from being fixed by simply
    /// over-charging the marker.
    #[test]
    fn a_line_with_room_for_both_keeps_the_marker_and_the_preview() {
        let detail = "code 7";
        let marked = inventory_line(&card_with(Some(ChangeDirection::Rose), detail), usize::MAX);
        let width = Line::from(marked.spans.clone()).width();

        let fitted = inventory_line(&card_with(Some(ChangeDirection::Rose), detail), width);

        let text = line_text(&fitted);
        assert!(text.contains(DELTA_ROSE_MARKER), "got {text:?}");
        assert!(text.contains(detail), "got {text:?}");
    }

    fn line_text(line: &Line<'static>) -> String {
        line.spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect()
    }

    /// The marker is painted in its own role, not in the plain style the rest
    /// of the line uses.
    ///
    /// Compared against the label's style rather than against `theme::delta()`:
    /// asserting the marker carries whatever `delta()` returns is satisfied by a
    /// `delta()` that returns nothing at all, which is exactly the regression
    /// worth catching. The claim is that it is *distinguishable*, and every
    /// bundled theme keeps it so — including `no-color`, which distinguishes
    /// with a modifier rather than a hue.
    #[test]
    fn the_change_marker_is_styled_apart_from_the_text_around_it() {
        let line = inventory_line(
            &card_with(Some(ChangeDirection::Rose), "code 7"),
            usize::MAX,
        );

        let marker = line
            .spans
            .iter()
            .find(|span| span.content.contains(DELTA_ROSE_MARKER))
            .expect("the marked line carries a marker span");
        let label = line.spans.first().expect("the line starts with its label");

        assert_ne!(
            marker.style, label.style,
            "the marker is painted in the same style as the plain label, so it reads as text"
        );
    }

    /// A band with room to spare must not claim it left a service out.
    ///
    /// Exercised against `render_inventory_row` directly rather than through
    /// `render_home`: the band is never given more than `ALL.len() + 2` rows
    /// there, so a `+0 more` line would be clipped by the block and never reach
    /// the screen. That clipping is a coincidence of today's height arithmetic,
    /// not the guarantee — the renderer must count an omission only when it
    /// made one, at any height it is handed.
    #[test]
    fn a_band_with_room_to_spare_counts_nothing_as_missing() {
        let root = RootState::default();
        let overview = project_home_overview(&root, std::time::Instant::now());
        let spare_rows =
            u16::try_from(overview.inventory.len()).expect("service count fits a u16") + 10;

        let rendered = painted(Rect::new(0, 0, 120, spare_rows + 2), &overview);

        assert!(
            !rendered.contains("more"),
            "every service was drawn, so nothing may be reported missing; rendered:\n{rendered}"
        );
    }
}
