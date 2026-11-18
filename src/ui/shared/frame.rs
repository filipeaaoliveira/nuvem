use chrono::Local;
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use unicode_truncate::UnicodeTruncateStr;

use crate::app::projection::{AppFrameProjection, format_last_refresh};
use crate::ui::shared::components::{fit_text, status_line};
use crate::ui::theme;

const CONTEXT_SEPARATOR: &str = "  ";

/// Terminal rectangles allocated to the shared application frame.
pub struct FrameAreas {
    /// Two-row title and context header.
    pub header: Rect,
    /// One-row navigation breadcrumb.
    pub breadcrumb: Rect,
    /// Flexible workspace body area.
    pub body: Rect,
    /// Two-row operation and refresh status area.
    pub status: Rect,
    /// One-row mode-aware key hint footer.
    pub footer: Rect,
}

/// Splits a terminal area into the shared frame regions.
pub fn split_frame(area: Rect) -> FrameAreas {
    let areas = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),
            Constraint::Length(1),
            Constraint::Min(0),
            Constraint::Length(2),
            Constraint::Length(1),
        ])
        .split(area);

    FrameAreas {
        header: areas[0],
        breadcrumb: areas[1],
        body: areas[2],
        status: areas[3],
        footer: areas[4],
    }
}

pub fn render_header(frame: &mut Frame<'_>, area: Rect, projection: &AppFrameProjection) {
    frame.render_widget(
        Paragraph::new(vec![
            title_line(projection, usize::from(area.width)),
            context_line(projection, usize::from(area.width)),
        ]),
        area,
    );
}

pub fn render_breadcrumb(frame: &mut Frame<'_>, area: Rect, projection: &AppFrameProjection) {
    frame.render_widget(
        Paragraph::new(fit_text(
            &projection.breadcrumbs.join(" > "),
            usize::from(area.width),
        ))
        .style(theme::muted()),
        area,
    );
}

pub fn render_status(frame: &mut Frame<'_>, area: Rect, projection: &AppFrameProjection) {
    let status_label = if projection.operation.message.is_some() {
        projection.operation.message_label.as_deref()
    } else {
        Some(projection.operation.label.as_str())
    };
    let status = status_line(
        projection.operation.phase,
        status_label,
        projection.operation.message.as_deref(),
    );
    let status_text = status
        .spans
        .iter()
        .map(|span| span.content.as_ref())
        .collect::<String>();
    let status_style = status
        .spans
        .first()
        .map(|span| span.style)
        .unwrap_or_default();
    let mut lines = vec![Line::from(Span::styled(
        fit_text(&status_text, usize::from(area.width)),
        status_style,
    ))];
    let age = format_last_refresh(Local::now(), projection.operation.last_refresh_at);
    lines.push(Line::from(Span::styled(
        fit_text(&format!("Updated: {age}"), usize::from(area.width)),
        theme::muted(),
    )));
    frame.render_widget(Paragraph::new(lines), area);
}

/// Columns between the view's own hints and the pinned ones, so the two
/// groups read as two groups rather than as one run of keys.
const PINNED_HINT_GAP: usize = 3;

pub fn render_footer(frame: &mut Frame<'_>, area: Rect, projection: &AppFrameProjection) {
    let total = usize::from(area.width);
    let pinned = fitted_pins(
        &projection.footer_pinned_hints,
        natural_width(&projection.footer_hints),
        total,
    );
    // A mode owning the keyboard pins nothing, and then this is the footer it
    // always was: the whole row belongs to the view's own hints. Taken here as
    // its own case rather than as a zero-width reserve below, so "there are no
    // pins" is a branch a reader (and a test) can see rather than an arithmetic
    // coincidence that happens to add nothing.
    if pinned.is_empty() {
        let spans = hint_spans(&projection.footer_hints, total);
        frame.render_widget(Paragraph::new(Line::from(spans)), area);
        return;
    }

    // The pins' width comes out of the budget the view's hints are fitted into,
    // rather than the pins being appended and dropped by the same
    // left-to-right pass. That order is the feature: the narrow terminal that
    // truncates the view's hints is exactly the one whose operator most needs
    // the way out still on screen.
    let pinned_spans = hint_spans(&pinned, total);
    let pinned_width = spans_width(&pinned_spans);
    let mut spans = hint_spans(
        &projection.footer_hints,
        total.saturating_sub(pinned_width + PINNED_HINT_GAP),
    );

    let padding = total
        .saturating_sub(spans_width(&spans))
        .saturating_sub(pinned_width);
    spans.push(Span::raw(" ".repeat(padding)));
    spans.extend(pinned_spans);

    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

/// The fraction of the row the pins may take once the view's own hints have
/// overflowed it on their own. A third: enough for both pins with their labels
/// on a wide terminal, never so much that the pins become the footer.
const PINNED_HINT_SHARE: usize = 3;

/// The pins that survive at this width, given what the view's own hints want.
///
/// The pins yield before they take. Their allowance is whatever the view's
/// hints leave over, so a row with room for both keeps both, a row with room
/// for one keeps help, and no view hint is ever paid for a pin that could have
/// fitted in the slack. Only where the hints fill the row on their own — where
/// they are going to be truncated whether the pins are there or not — do the
/// pins take a bounded share of it (`PINNED_HINT_SHARE`) rather than collapse
/// to nothing for a truncation that was happening anyway.
///
/// Within the allowance they are dropped from the front — palette first, help
/// last — and the survivor gives up its label before it gives up itself, so a
/// cramped footer ends in a bare `?` rather than in nothing. Help is last out
/// because it is the pin that repays the space: it names the overlay that
/// lists every key the view has, the palette's included.
fn fitted_pins(
    pinned: &[crate::app::projection::FrameHint],
    hints_width: usize,
    total: usize,
) -> Vec<crate::app::projection::FrameHint> {
    let slack = total.saturating_sub(hints_width + PINNED_HINT_GAP);
    let allowance = if slack > 0 {
        slack
    } else {
        (total / PINNED_HINT_SHARE).saturating_sub(PINNED_HINT_GAP)
    };
    let fits = |pins: &[crate::app::projection::FrameHint]| natural_width(pins) <= allowance;

    let mut pins = pinned.to_vec();
    while pins.len() > 1 && !fits(&pins) {
        pins.remove(0);
    }
    if let [only] = pins.as_mut_slice()
        && !fits(std::slice::from_ref(only))
    {
        only.label = "";
    }
    pins
}

/// What `hints` would occupy laid out with no width limit.
fn natural_width(hints: &[crate::app::projection::FrameHint]) -> usize {
    spans_width(&hint_spans(hints, usize::from(u16::MAX)))
}

/// Lays `hints` out left-to-right into `width` columns, dropping whole hints
/// (and truncating the last one that partly fits) once the budget runs out.
fn hint_spans(hints: &[crate::app::projection::FrameHint], width: usize) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    let mut used_width = 0;
    for hint in hints {
        let separator = if used_width == 0 { "" } else { "  " };
        let separator_width = display_width(separator);
        let available_width = width.saturating_sub(used_width + separator_width);
        if available_width == 0 {
            break;
        }

        // Whole or not at all. `fit_text` answers a width of 1, 2 or 3 with
        // that many literal periods, and a key rendered as `.` names no key the
        // operator can press — at 23 columns the Resources footer read
        // `v Toggle sidebar  .   ?`. Breaking rather than skipping
        // keeps the footer a prefix of the hint list: showing the fifth hint
        // while hiding the third would reorder the row without saying so.
        if fit_text(&hint.key, available_width) != hint.key {
            break;
        }
        let key = hint.key.clone();
        let key_width = display_width(&key);
        let label_width = available_width.saturating_sub(key_width + 1);
        // The label may be truncated — `Pick f...` still says something — but
        // not down to punctuation alone. `r ..` is not a shorter way of writing
        // `r Refresh`; the key on its own is already a complete instruction, so
        // the label yields entirely and the columns go to the next hint.
        let label = fit_text(hint.label, label_width);
        let label = if label != hint.label && label.chars().all(|character| character == '.') {
            String::new()
        } else {
            label
        };
        spans.push(Span::raw(separator));
        spans.push(Span::styled(key, theme::key_hint()));
        if !label.is_empty() {
            spans.push(Span::raw(" "));
            spans.push(Span::raw(label.clone()));
        }
        used_width +=
            separator_width + key_width + usize::from(!label.is_empty()) + display_width(&label);
    }
    spans
}

fn spans_width(spans: &[Span<'static>]) -> usize {
    spans.iter().map(|span| display_width(&span.content)).sum()
}

fn context_line(projection: &AppFrameProjection, width: usize) -> Line<'static> {
    let mut context = required_context_values(projection, width);
    add_optional_context_value(
        &mut context,
        "Account",
        projection.context.account.as_deref(),
        width,
    );
    add_optional_context_value(&mut context, "Service", projection.service_label, width);
    add_optional_context_value(
        &mut context,
        "Region",
        projection.context.region.as_deref(),
        width,
    );
    add_optional_context_value(
        &mut context,
        "Zone",
        projection.context.zone.as_deref(),
        width,
    );

    Line::styled(context.join(CONTEXT_SEPARATOR), theme::muted())
}

fn title_line(projection: &AppFrameProjection, width: usize) -> Line<'static> {
    let app_title = fit_text(projection.app_title, width);
    let app_width = display_width(&app_title);
    let separator_width = width
        .saturating_sub(app_width)
        .min(display_width(CONTEXT_SEPARATOR));
    let view_width = width.saturating_sub(app_width + separator_width);

    let mut spans = vec![Span::styled(app_title, theme::app_title())];
    if separator_width > 0 {
        spans.push(Span::raw(fit_text(CONTEXT_SEPARATOR, separator_width)));
    }
    if view_width > 0 {
        spans.push(Span::styled(
            fit_text(&projection.view_title, view_width),
            theme::view_title(),
        ));
    }
    Line::from(spans)
}

fn required_context_values(projection: &AppFrameProjection, width: usize) -> Vec<String> {
    match (
        projection.context.configuration.as_deref(),
        projection.context.project.as_deref(),
    ) {
        (Some(configuration), Some(project)) => {
            let labels_width = display_width("Configuration: ")
                + display_width(CONTEXT_SEPARATOR)
                + display_width("Project: ");
            let value_width = width.saturating_sub(labels_width);
            let configuration_share = value_width / 2;
            let project_share = value_width - configuration_share;
            let configuration_width =
                configuration_share + project_share.saturating_sub(display_width(project));
            let project_width =
                project_share + configuration_share.saturating_sub(display_width(configuration));

            vec![
                format!(
                    "Configuration: {}",
                    fit_text(configuration, configuration_width)
                ),
                format!("Project: {}", fit_text(project, project_width)),
            ]
        }
        (configuration, project) => {
            let mut context = Vec::new();
            add_optional_context_value(&mut context, "Configuration", configuration, width);
            add_optional_context_value(&mut context, "Project", project, width);
            context
        }
    }
}

fn add_optional_context_value(
    context: &mut Vec<String>,
    label: &str,
    value: Option<&str>,
    width: usize,
) {
    if let Some(value) = value {
        let field = format!("{label}: {value}");
        let separator_width = if context.is_empty() {
            0
        } else {
            display_width(CONTEXT_SEPARATOR)
        };
        let context_width = context
            .iter()
            .map(|field| display_width(field))
            .sum::<usize>()
            + context
                .len()
                .saturating_sub(1)
                .saturating_mul(display_width(CONTEXT_SEPARATOR));

        let available_width = width.saturating_sub(context_width + separator_width);
        if available_width > 0 {
            context.push(fit_text(&field, available_width));
        }
    }
}

fn display_width(value: &str) -> usize {
    value.unicode_truncate(usize::MAX).1
}
