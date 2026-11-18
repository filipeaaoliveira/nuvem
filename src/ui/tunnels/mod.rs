//! Renderer for the managed-tunnel workspace.
//!
//! Body content only: the header, breadcrumb, operation status, and footer are
//! painted by `ui::render` around the `body_area` handed in here. Every string
//! rendered comes from `project_tunnels`, which sanitizes the child-derived
//! labels and messages, so nothing here can carry control characters.

use std::time::Instant;

use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::text::Line;
use ratatui::widgets::{List, Paragraph};

use crate::app::projection::{
    TUNNELS_EMPTY_MESSAGE, TunnelsBodyState, TunnelsWorkspaceProjection, project_tunnels,
};
use crate::services::runtime::session::RuntimeSession;
use crate::ui::shared::components::{bordered_block, empty_state, fit_text};
use crate::ui::shared::table::{ROW_SELECT_SYMBOL, display_width, plan_table, render_table};

const BLOCK_TITLE: &str = "Tunnels";

/// Renders the tunnel list inside `area`.
///
/// The clock is read here, at the renderer, and passed into the projection:
/// ages must be derived from a caller-supplied `Instant` so the projection
/// stays a pure function and its tests never sleep.
pub fn render(frame: &mut Frame<'_>, area: Rect, session: &RuntimeSession) {
    if area.width == 0 || area.height == 0 {
        return;
    }

    let projection = project_tunnels(&session.app().root, Instant::now());

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(0)])
        .split(area);
    render_summary(frame, chunks[0], &projection);

    match projection.body_state {
        TunnelsBodyState::Rows => render_table_body(frame, chunks[1], &projection),
        TunnelsBodyState::Empty => render_empty_body(frame, chunks[1]),
    }
}

fn render_summary(frame: &mut Frame<'_>, area: Rect, projection: &TunnelsWorkspaceProjection) {
    if area.height == 0 {
        return;
    }
    let summary = format!(
        "Active: {}  Listed: {}",
        projection.active_count, projection.total_count
    );
    let text = fit_text(&summary, usize::from(area.width));
    frame.render_widget(Paragraph::new(Line::raw(text)), area);
}

fn render_table_body(frame: &mut Frame<'_>, area: Rect, projection: &TunnelsWorkspaceProjection) {
    let block = bordered_block(BLOCK_TITLE);
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

fn render_empty_body(frame: &mut Frame<'_>, area: Rect) {
    let list = List::new(vec![empty_state(TUNNELS_EMPTY_MESSAGE)])
        .block(bordered_block(BLOCK_TITLE))
        .highlight_symbol("");
    frame.render_widget(list, area);
}
