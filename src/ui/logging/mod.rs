//! Renderer for the Cloud Logging workspace.
//!
//! All displayed text originates from `project_logging`, which sanitizes every
//! log-derived string, so nothing rendered here can carry control characters.

use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::text::Line;
use ratatui::widgets::{Paragraph, Wrap};

use crate::app::projection::{
    LogDetailView, LoggingBodyState, LoggingHeaderProjection, LoggingWorkspaceProjection,
    project_logging,
};
use crate::services::runtime::session::RuntimeSession;
use crate::ui::shared::components::{bordered_block, fit_text};
use crate::ui::shared::table::{ROW_SELECT_SYMBOL, display_width, plan_table, render_table};
use crate::ui::theme;

pub fn render(frame: &mut Frame<'_>, area: Rect, session: &RuntimeSession) {
    if area.width == 0 || area.height == 0 {
        return;
    }

    let projection = project_logging(&session.app().root);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(0)])
        .split(area);
    let header_area = chunks[0];
    let body_area = chunks[1];

    render_header(frame, header_area, &projection);

    if let Some(detail) = &projection.detail {
        render_detail(frame, body_area, detail);
        return;
    }

    match &projection.body_state {
        LoggingBodyState::Rows => render_table_body(frame, body_area, &projection),
        LoggingBodyState::Loading => render_body_message(frame, body_area, "Loading logs..."),
        LoggingBodyState::NotLoaded => render_body_message(frame, body_area, "No logs loaded"),
        LoggingBodyState::Empty => render_body_message(frame, body_area, "No matching log entries"),
        LoggingBodyState::Degraded { message } => render_body_message(frame, body_area, message),
    }
}

fn render_header(frame: &mut Frame<'_>, area: Rect, projection: &LoggingWorkspaceProjection) {
    if area.height == 0 {
        return;
    }
    let width = usize::from(area.width);
    let header = &projection.header;
    // The severity floor and the time bucket are real server-side narrowings
    // with no other on-screen signal, so they sit ahead of the query summary:
    // on a narrow pane `fit_text` must eat the query before it eats the reason
    // entries are missing.
    let summary = format!(
        "Project: {}  Severity: {}  Range: {} [{}]  Query: {}  {}  {}  Updated: {}",
        header.project,
        header.severity_floor,
        header.time_range,
        header.time_range_bucket,
        header.query,
        header.tail,
        header.status,
        header.freshness,
    );
    let mut lines = vec![Line::raw(fit_text(&summary, width))];
    lines.push(Line::raw(fit_text(
        &format!("Filter: {}", header.filter),
        width,
    )));
    lines.push(Line::raw(fit_text(&search_line(header), width)));
    frame.render_widget(Paragraph::new(lines), area);
}

/// The search line: the draft while typing, otherwise the applied pattern with
/// the mode it selected (or its parse error). With nothing applied it spells
/// out the grammar's sigils, which are the only place the syntax is
/// discoverable from — a bare term is a literal substring, so the sigils are
/// what unlock fuzzy, regex, and inversion.
fn search_line(header: &LoggingHeaderProjection) -> String {
    if header.filter_active {
        return format!("Search: {}_", header.filter_draft);
    }
    if header.applied_filter.is_empty() {
        return "Search: none  (text | ~fuzzy | /regex/ | !invert)".to_string();
    }
    match (&header.search_error, &header.search_mode) {
        (Some(error), _) => format!("Search: {} ({})", header.applied_filter, error),
        (None, Some(mode)) => format!("Search: {} ({})", header.applied_filter, mode),
        (None, None) => format!("Search: {}", header.applied_filter),
    }
}

fn render_table_body(frame: &mut Frame<'_>, area: Rect, projection: &LoggingWorkspaceProjection) {
    let block = bordered_block("Logs");
    let inner = block.inner(area);
    let marker_w = display_width(ROW_SELECT_SYMBOL);
    let plan = plan_table(
        &projection.table,
        inner.width.saturating_sub(marker_w),
        false,
    );
    frame.render_widget(block, area);
    render_table(frame, inner, &projection.table, &plan);
}

fn render_body_message(frame: &mut Frame<'_>, area: Rect, message: &str) {
    let block = bordered_block("Logs");
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }
    let text = fit_text(message, usize::from(inner.width));
    frame.render_widget(Paragraph::new(Line::styled(text, theme::muted())), inner);
}

fn render_detail(frame: &mut Frame<'_>, area: Rect, detail: &LogDetailView) {
    let block = bordered_block("Log entry");
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }
    let width = usize::from(inner.width);
    // When wrapping, emit each line's full (already-sanitized) text and let
    // ratatui reflow it; otherwise truncate to the pane width via `fit_text`.
    let wrap = detail.wrap;
    let line = |text: String| -> Line<'static> {
        if wrap {
            Line::raw(text)
        } else {
            Line::raw(fit_text(&text, width))
        }
    };
    let mut lines: Vec<Line<'static>> = vec![
        line(format!("Severity: {}", detail.severity)),
        line(format!("Timestamp: {}", detail.timestamp)),
        line(format!("Resource: {}", detail.resource_type)),
        line(format!("Log: {}", detail.log_name)),
        line(format!("Message: {}", detail.message)),
    ];

    if !detail.resource_labels.is_empty() {
        lines.push(line("Resource labels:".to_string()));
        for label in &detail.resource_labels {
            lines.push(line(format!("  {}: {}", label.key, label.value)));
        }
    }

    if !detail.labels.is_empty() {
        lines.push(line("Labels:".to_string()));
        for label in &detail.labels {
            lines.push(line(format!("  {}: {}", label.key, label.value)));
        }
    }

    if !detail.payload_lines.is_empty() {
        lines.push(line("Payload:".to_string()));
        for payload_line in &detail.payload_lines {
            lines.push(line(payload_line.clone()));
        }
    }

    let mut paragraph = Paragraph::new(lines).scroll((detail.scroll, 0));
    if wrap {
        paragraph = paragraph.wrap(Wrap { trim: false });
    }
    frame.render_widget(paragraph, inner);
}
