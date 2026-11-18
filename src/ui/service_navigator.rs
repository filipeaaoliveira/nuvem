use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{List, ListItem, ListState, Paragraph};

use crate::app::projection::{
    NavigatorFocus, ServiceNavigatorEntryProjection, ServiceNavigatorProjection,
};
use crate::ui::shared::components::{bordered_block, fit_text, focused_block};
use crate::ui::shared::table::display_width;
use crate::ui::theme;

/// The cursor marker `List` paints to the left of the selected entry.
///
/// Named rather than written inline at the `highlight_symbol` call, because
/// [`entry_width`] has to subtract exactly this: ratatui reserves the symbol's
/// width on *every* row, selected or not, so a symbol and an item width that
/// disagree truncate each entry twice at two different widths.
const HIGHLIGHT_SYMBOL: &str = "> ";

/// The columns an entry's own text actually gets: the block's inner width less
/// the gutter `HIGHLIGHT_SYMBOL` reserves.
///
/// Fitting to the inner width alone was the defect: `fit_text` appended
/// its `...` at one width and the widget then clipped two columns off the
/// result, so every truncated entry on screen ended in a lone `.`, which reads
/// as a typo rather than as "there is more here".
fn entry_width(area: Rect) -> usize {
    usize::from(area.width.saturating_sub(2))
        .saturating_sub(usize::from(display_width(HIGHLIGHT_SYMBOL)))
}

pub fn render_service_navigator(
    frame: &mut Frame<'_>,
    area: Rect,
    projection: &ServiceNavigatorProjection,
    title: &str,
) {
    if area.width == 0 || area.height == 0 {
        return;
    }

    let inner_width = entry_width(area);
    let start = projection.viewport_start.min(projection.entries.len());
    let end = projection
        .viewport_end
        .min(projection.entries.len())
        .max(start);
    let visible_entries = &projection.entries[start..end];
    let items = if visible_entries.is_empty() {
        vec![ListItem::new(Line::from(Span::styled(
            fit_text(
                &format!("No services match \"{}\"", projection.filter.applied),
                inner_width,
            ),
            theme::muted(),
        )))]
    } else {
        visible_entries
            .iter()
            .map(|entry| render_entry(entry, inner_width))
            .collect::<Vec<_>>()
    };

    let title = navigator_title(title, projection);
    if area.height == 1 {
        let selected_label = projection
            .entries
            .get(projection.selected_visible)
            .map(|entry| entry.label)
            .unwrap_or("No matching services");
        frame.render_widget(
            Paragraph::new(fit_text(
                &format!("{title}: {selected_label}"),
                usize::from(area.width),
            )),
            area,
        );
        return;
    }

    let block = if projection.focus == NavigatorFocus::Navigator {
        focused_block(title)
    } else {
        bordered_block(title)
    };
    let list = List::new(items)
        .block(block)
        .highlight_symbol(HIGHLIGHT_SYMBOL)
        .highlight_style(theme::selected_row());
    let mut state = ListState::default();
    if !visible_entries.is_empty() {
        let selected = projection
            .selected_visible
            .saturating_sub(start)
            .min(visible_entries.len().saturating_sub(1));
        state.select(Some(selected));
    }
    frame.render_stateful_widget(list, area, &mut state);
}

fn navigator_title(title: &str, projection: &ServiceNavigatorProjection) -> Line<'static> {
    let suffix = if projection.filter.active {
        format!(" / {}_", projection.filter.draft)
    } else if !projection.filter.applied.is_empty() {
        format!(" / {}", projection.filter.applied)
    } else {
        String::new()
    };
    Line::from(format!("{title}{suffix}"))
}

fn render_entry(entry: &ServiceNavigatorEntryProjection, width: usize) -> ListItem<'static> {
    if !entry.selectable {
        return ListItem::new(Line::from(Span::styled(
            fit_text(entry.label, width),
            theme::muted(),
        )));
    }

    let mut text = entry.label.to_string();
    if let Some(reason) = entry.unavailable_reason {
        text.push_str("  Unavailable: ");
        text.push_str(reason);
    }

    let style = if entry.is_active {
        theme::active_tab()
    } else {
        ratatui::style::Style::default()
    };
    ListItem::new(Line::from(Span::styled(fit_text(&text, width), style)))
}
