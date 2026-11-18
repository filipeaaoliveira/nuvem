//! Responsive width-fitting and rendering for the shared resource table.
//!
//! `plan_table` is a pure function: given a `TableModel`, the available width,
//! and the wide-columns flag, it decides each column's width and visibility.
//! `render_table` draws the planned table with ratatui.

use std::borrow::Cow;

use ratatui::Frame;
use ratatui::layout::{Constraint, Rect};
use ratatui::style::Style;
use ratatui::text::Span;
use ratatui::widgets::{Cell as TuiCell, Row, Table, TableState};
use unicode_truncate::UnicodeTruncateStr;
use unicode_width::UnicodeWidthStr;

use crate::app::projection::table::{
    CellAlign, CellKind, SortDirection, StatusSemantic, TableModel,
};
use crate::ui::theme;

const GAP: u16 = 2;

pub const ROW_SELECT_SYMBOL: &str = "> ";

/// Glyph shown in the leading mark column for a marked row. Two display
/// cells wide so the column keeps a stable width whether or not the row is
/// marked.
const MARK_GLYPH: &str = "\u{25c6} ";
/// Blank filler for the leading mark column on unmarked rows.
const MARK_BLANK: &str = "  ";
/// Width, in terminal cells, of the dedicated leading mark column.
/// Independent of `ROW_SELECT_SYMBOL`: a row can be both cursor-selected
/// (drawn by ratatui in the highlight gutter) and marked (drawn here).
pub const MARK_COLUMN_WIDTH: u16 = 2;
/// Total width callers must reserve for the mark column, including the
/// column-spacing gap between it and the first content column.
pub const MARK_COLUMN_RESERVED_WIDTH: u16 = MARK_COLUMN_WIDTH + GAP;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ColumnLayout {
    pub index: usize,
    pub width: u16,
    pub visible: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TablePlan {
    pub columns: Vec<ColumnLayout>,
    pub hidden_count: usize,
    pub overflow: bool,
}

/// Display width of a string in terminal cells (not byte or char count).
pub fn display_width(text: &str) -> u16 {
    UnicodeWidthStr::width(text).min(u16::MAX as usize) as u16
}

/// Each column's width is the widest thing in it, header included — measured
/// across EVERY row, not just the ones on screen.
///
/// That is what fixes the cost of a frame to the size of the list rather than
/// to the size of the viewport, and it was measured before being left alone.
/// Rendering only the visible window was tried: it reproduces ratatui's
/// scroll-into-view exactly and renders identical frames, and it bought
/// nothing. With `render_table` building NO data rows at all, a 500-row frame
/// still cost roughly two and a half times a 0-row one, because the rows were
/// already built upstream — the projection and the table model — before
/// anything here sees them.
///
/// Windowing upstream is where the cost actually is, and this function is why
/// it is not free: sized from the visible rows alone, a column would change
/// width as the operator scrolled past a longer value. Stable columns are
/// worth more than the microseconds, at four redraws a second.
fn content_widths(model: &TableModel) -> Vec<u16> {
    (0..model.columns.len())
        .map(|c| {
            let header = display_width(model.columns[c].header);
            let cell = model
                .rows
                .iter()
                .map(|row| display_width(&row.cells[c].text))
                .max()
                .unwrap_or(0);
            header.max(cell)
        })
        .collect()
}

fn masked_line_width(widths: &[u16], visible: &[bool]) -> u16 {
    let count = visible.iter().filter(|v| **v).count();
    let sum: u16 = widths
        .iter()
        .zip(visible)
        .filter(|(_, v)| **v)
        .map(|(w, _)| *w)
        .sum();
    sum + GAP * (count.saturating_sub(1)) as u16
}

pub fn plan_table(model: &TableModel, available_width: u16, wide: bool) -> TablePlan {
    let n = model.columns.len();
    let content = content_widths(model);
    let target = |c: usize| -> u16 {
        content[c].clamp(model.columns[c].min_width, model.columns[c].preferred_width)
    };

    if wide {
        let widths: Vec<u16> = (0..n).map(target).collect();
        let overflow = masked_line_width(&widths, &vec![true; n]) > available_width;
        return TablePlan {
            columns: (0..n)
                .map(|c| ColumnLayout {
                    index: c,
                    width: widths[c],
                    visible: true,
                })
                .collect(),
            hidden_count: 0,
            overflow,
        };
    }

    let mut visible = vec![true; n];
    let mut widths: Vec<u16> = (0..n).map(|c| model.columns[c].min_width).collect();

    // The column with the smallest priority value is protected from dropping,
    // and that column is NAME in every table: each catalog gives NAME priority
    // 0, and a configured layout re-derives its priorities from display
    // position with NAME pinned to 0 wherever the user put it
    // (`projection::columns::apply_display_priorities`). So no configured
    // column list can leave the operator with rows they cannot tell apart.
    let survivor = (0..n)
        .min_by_key(|&c| (model.columns[c].priority, c))
        .unwrap_or(0);

    // Drop the lowest-priority visible column until the min-width line fits.
    while masked_line_width(&widths, &visible) > available_width {
        let victim = (0..n)
            .filter(|&c| visible[c] && c != survivor)
            .max_by_key(|&c| (model.columns[c].priority, c));
        match victim {
            Some(c) => visible[c] = false,
            None => break,
        }
    }

    // Grow visible columns toward their targets, highest priority (lowest value) first.
    // Each column is grown to its full target before any lower-priority column
    // receives slack, so a constrained width never partially truncates a
    // higher-priority column (e.g. NAME) in favor of a lower-priority one.
    let mut order: Vec<usize> = (0..n).filter(|&c| visible[c]).collect();
    order.sort_by_key(|&c| (model.columns[c].priority, c));
    let mut slack = available_width.saturating_sub(masked_line_width(&widths, &visible));
    for &c in &order {
        if slack == 0 {
            break;
        }
        let want = target(c).saturating_sub(widths[c]);
        let grant = want.min(slack);
        widths[c] += grant;
        slack -= grant;
    }

    // Safety: guarantee the line never exceeds the available width, even when a
    // single protected column is wider than the terminal.
    while masked_line_width(&widths, &visible) > available_width {
        match (0..n)
            .filter(|&c| visible[c] && widths[c] > 1)
            .max_by_key(|&c| widths[c])
        {
            Some(c) => widths[c] -= 1,
            None => break,
        }
    }

    let hidden_count = visible.iter().filter(|v| !**v).count();
    TablePlan {
        columns: (0..n)
            .map(|c| ColumnLayout {
                index: c,
                width: widths[c],
                visible: visible[c],
            })
            .collect(),
        hidden_count,
        overflow: false,
    }
}

pub fn status_marker(semantic: StatusSemantic) -> &'static str {
    match semantic {
        StatusSemantic::Healthy => "OK",
        StatusSemantic::Busy => "SYNC",
        StatusSemantic::Warning => "WARN",
        StatusSemantic::Degraded => "ERR",
        StatusSemantic::Neutral => "--",
    }
}

fn semantic_style(semantic: StatusSemantic) -> Style {
    match semantic {
        StatusSemantic::Healthy => theme::status_healthy(),
        StatusSemantic::Busy => theme::status_loading(),
        StatusSemantic::Warning => theme::status_stale(),
        StatusSemantic::Degraded => theme::status_error(),
        StatusSemantic::Neutral => theme::muted(),
    }
}

pub fn cell_style(kind: &CellKind) -> Style {
    match kind {
        CellKind::Plain => Style::default(),
        CellKind::Missing => theme::muted(),
        CellKind::Status(semantic) => semantic_style(*semantic),
    }
}

/// Truncates to `width` display columns (with an ellipsis marker) and pads to
/// exactly `width`, honoring alignment.
fn fit_cell(text: &str, width: u16, align: CellAlign) -> String {
    let width = width as usize;
    if width == 0 {
        return String::new();
    }
    // One width scan and one allocation, because this runs once per cell and a
    // full table is a few thousand cells per redraw — at four redraws a second,
    // forever. The straightforward spelling measured each string twice (once to
    // decide whether to truncate, again on the result to size the padding) and
    // allocated three times: the copy, the run of spaces, and the concatenation
    // of the two.
    //
    // `unicode_truncate` already returns the width of what it kept, so the
    // truncating branch needs no second measurement either. The widths are not
    // interchangeable with byte lengths and the `saturating_sub` is not
    // decoration: truncating to `width - 1` can land short of it when the next
    // character is double-width, so the padding is what squares the column up.
    let text_width = display_width(text) as usize;
    let (shown, shown_width): (Cow<'_, str>, usize) = if text_width > width {
        if width == 1 {
            (Cow::Borrowed("…"), 1)
        } else {
            let (head, head_width) = text.unicode_truncate(width - 1);
            (Cow::Owned(format!("{head}…")), head_width + 1)
        }
    } else {
        (Cow::Borrowed(text), text_width)
    };
    let pad = width.saturating_sub(shown_width);
    let mut fitted = String::with_capacity(shown.len() + pad);
    match align {
        CellAlign::Left => {
            fitted.push_str(&shown);
            fitted.extend(std::iter::repeat_n(' ', pad));
        }
        CellAlign::Right => {
            fitted.extend(std::iter::repeat_n(' ', pad));
            fitted.push_str(&shown);
        }
    }
    fitted
}

pub fn render_table(frame: &mut Frame<'_>, area: Rect, model: &TableModel, plan: &TablePlan) {
    let visible: Vec<&ColumnLayout> = plan.columns.iter().filter(|c| c.visible).collect();
    let show_mark = model.show_mark_column;
    let lead = usize::from(show_mark);
    let mut constraints: Vec<Constraint> = Vec::with_capacity(visible.len() + lead);
    if show_mark {
        constraints.push(Constraint::Length(MARK_COLUMN_WIDTH));
    }
    constraints.extend(visible.iter().map(|c| Constraint::Length(c.width)));

    let mut header_cells: Vec<TuiCell<'_>> = Vec::with_capacity(visible.len() + lead);
    if show_mark {
        header_cells.push(TuiCell::from(MARK_BLANK));
    }
    header_cells.extend(visible.iter().map(|layout| {
        let column = &model.columns[layout.index];
        let mut label = column.header.to_string();
        if model.sort.column == layout.index {
            label.push(match model.sort.direction {
                SortDirection::Ascending => '▲',
                SortDirection::Descending => '▼',
            });
        }
        if model.selected_column == layout.index {
            label = format!("[{label}]");
        }
        let style = if model.selected_column == layout.index {
            theme::active_tab()
        } else {
            theme::view_title()
        };
        TuiCell::from(Span::styled(
            fit_cell(&label, layout.width, column.align),
            style,
        ))
    }));
    let header = Row::new(header_cells).style(theme::title_style());

    let rows = model.rows.iter().map(|row| {
        let mut cells: Vec<TuiCell<'_>> = Vec::with_capacity(visible.len() + lead);
        if show_mark {
            let marker = if row.marked { MARK_GLYPH } else { MARK_BLANK };
            cells.push(TuiCell::from(marker));
        }
        cells.extend(visible.iter().map(|layout| {
            let cell = &row.cells[layout.index];
            let column = &model.columns[layout.index];
            let style = cell_style(&cell.kind);
            TuiCell::from(Span::styled(
                fit_cell(&cell.text, layout.width, column.align),
                style,
            ))
        }));
        Row::new(cells)
    });

    let table = Table::new(rows, constraints)
        .header(header)
        .column_spacing(GAP)
        .row_highlight_style(theme::selected_row())
        .highlight_symbol(ROW_SELECT_SYMBOL);

    let mut state = TableState::default();
    state.select(model.selected_row);
    frame.render_stateful_widget(table, area, &mut state);
}

#[cfg(test)]
mod fit_cell_tests {
    use super::*;

    /// The three shapes `fit_cell` has to get exactly right, pinned because
    /// the fast path rewrote how the widths are obtained: the fitting case now
    /// reuses the measurement that decided it fits, and the truncating case
    /// takes the width `unicode_truncate` already returned instead of
    /// measuring its own output. Both are meant to be the same answer as
    /// measuring twice, and every cell in every table depends on it.
    #[test]
    fn a_value_that_fits_is_padded_to_exactly_the_column_width() {
        assert_eq!(fit_cell("web-1", 8, CellAlign::Left), "web-1   ");
        assert_eq!(fit_cell("web-1", 8, CellAlign::Right), "   web-1");
        assert_eq!(fit_cell("exactly8", 8, CellAlign::Left), "exactly8");
    }

    #[test]
    fn a_value_that_does_not_fit_is_truncated_with_an_ellipsis() {
        assert_eq!(fit_cell("europe-west1-b", 8, CellAlign::Left), "europe-…");
        assert_eq!(
            display_width(&fit_cell("europe-west1-b", 8, CellAlign::Left)),
            8
        );
        // A single column has no room for a prefix, only the marker.
        assert_eq!(fit_cell("europe-west1-b", 1, CellAlign::Left), "…");
    }

    /// The case the `saturating_sub` exists for. Truncating to `width - 1`
    /// can stop a column short when the next character is double-width, and
    /// the padding is what squares the column back up — so the result is
    /// still exactly `width` columns even though the text plus the marker is
    /// one narrower.
    #[test]
    fn a_double_width_character_at_the_boundary_still_fills_the_column() {
        // Each CJK character occupies two columns.
        let fitted = fit_cell("日本語のテキスト", 7, CellAlign::Left);

        assert_eq!(
            display_width(&fitted),
            7,
            "a fitted cell is always exactly its column: {fitted:?}"
        );
        assert!(fitted.ends_with('…'), "{fitted:?}");
    }

    #[test]
    fn a_zero_width_column_yields_nothing() {
        assert_eq!(fit_cell("web-1", 0, CellAlign::Left), "");
    }
}
