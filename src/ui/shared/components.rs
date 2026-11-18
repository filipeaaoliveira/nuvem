use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, ListItem};
use unicode_truncate::UnicodeTruncateStr;
use unicode_width::UnicodeWidthStr;

use crate::app::state::system::OperationPhase;
use crate::ui::theme;

pub fn selected_row_style(selected: bool) -> Style {
    if selected {
        theme::selected_row()
    } else {
        Style::default()
    }
}

pub fn bordered_block(title: impl Into<Line<'static>>) -> Block<'static> {
    pane_block(title, false)
}

pub fn focused_block(title: impl Into<Line<'static>>) -> Block<'static> {
    pane_block(title, true)
}

fn pane_block(title: impl Into<Line<'static>>, focused: bool) -> Block<'static> {
    Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(if focused {
            theme::focused_border()
        } else {
            theme::border()
        })
}

pub fn modal_block(title: impl Into<String>) -> Block<'static> {
    Block::default()
        .title(Line::from(Span::styled(title.into(), theme::title_style())))
        .borders(Borders::ALL)
        .border_style(theme::overlay_border())
}

pub fn tab_line<'a, I>(tabs: I) -> Line<'static>
where
    I: IntoIterator<Item = (bool, &'a str)>,
{
    let mut spans = Vec::new();
    for (index, (active, label)) in tabs.into_iter().enumerate() {
        if index > 0 {
            spans.push(Span::raw(" | "));
        }
        let label = if active {
            format!("[{label}]")
        } else {
            label.to_string()
        };
        let style = if active {
            theme::active_tab()
        } else {
            theme::inactive_tab()
        };
        spans.push(Span::styled(label, style));
    }
    Line::from(spans)
}

pub fn empty_state(message: impl Into<String>) -> ListItem<'static> {
    ListItem::new(message.into()).style(theme::muted())
}

pub fn loading_state(message: impl Into<String>) -> ListItem<'static> {
    ListItem::new(message.into()).style(theme::status_loading())
}

pub fn loading_line(message: impl Into<String>) -> Line<'static> {
    Line::from(Span::styled(message.into(), theme::status_loading()))
}

pub fn stale_state(message: impl Into<String>) -> ListItem<'static> {
    ListItem::new(message.into()).style(theme::status_stale())
}

/// Same styling as [`error_state`] (below), but as a [`ListItem`] for use in
/// the same empty-state `List` as [`empty_state`]/[`loading_state`]/
/// [`stale_state`] -- rendered when a list-scoped load actually failed
/// rather than succeeded empty.
pub fn error_state_item(message: impl Into<String>) -> ListItem<'static> {
    ListItem::new(message.into()).style(theme::status_error())
}

pub fn error_state(message: impl Into<String>) -> Line<'static> {
    Line::from(Span::styled(message.into(), theme::status_error()))
}

pub fn status_line(
    phase: OperationPhase,
    label: Option<&str>,
    message: Option<&str>,
) -> Line<'static> {
    let text = match message {
        Some(message) => match label {
            Some(label) => format!("[{}] {label}: {message}", phase.symbol()),
            None => format!("[{}] {message}", phase.symbol()),
        },
        None => match label {
            Some(label) => format!("[{}] {label}", phase.symbol()),
            None => format!("[{}]", phase.symbol()),
        },
    };

    Line::from(Span::styled(text, status_style(phase)))
}

pub fn fit_text(value: &str, width: usize) -> String {
    let (fitted, _) = value.unicode_truncate(width);
    if fitted == value {
        return value.to_string();
    }
    if width == 0 {
        return String::new();
    }
    if width <= 3 {
        return ".".repeat(width);
    }

    let (fitted, _) = value.unicode_truncate(width - 3);
    format!("{fitted}...")
}

fn status_style(phase: OperationPhase) -> Style {
    match phase {
        OperationPhase::Idle | OperationPhase::Healthy => theme::status_healthy(),
        OperationPhase::Loading | OperationPhase::Refreshing => theme::status_loading(),
        OperationPhase::Degraded => theme::status_stale(),
        OperationPhase::Failed => theme::status_error(),
    }
}

/// Trims a line to `width` span by span, keeping each surviving span's style.
///
/// Not `fit_text` over the flattened text: that returns one unstyled string, so
/// a terminal narrow enough to truncate the line would also drop the error
/// colour off a problem headline — the width at which the signal matters most.
/// The detail pane's tab bar wants it for the same reason: the active tab is
/// bracketed *and* styled, and a flattened fit would paint the whole bar in one
/// colour at exactly the widths where the tabs run off the pane.
pub fn fit_spans(spans: Vec<Span<'static>>, width: usize) -> Line<'static> {
    let length_of = |span: &Span<'static>| span.content.chars().count();
    let total: usize = spans.iter().map(length_of).sum();
    if total <= width {
        return Line::from(spans);
    }

    // Below the marker's own width there is nothing readable to preserve, so
    // this degrades to `fit_text`'s answer over the flattened text — which is
    // that many dots. Style is lost, and at three columns there is no signal
    // left to carry.
    if width <= OVERFLOW_MARKER.len() {
        let flattened: String = spans.iter().map(|span| span.content.as_ref()).collect();
        return Line::from(fit_text(&flattened, width));
    }

    // The marker's room comes out of the budget up front rather than being
    // appended to whatever survived. A cut that lands exactly on a span
    // boundary would otherwise stop after a whole span and read as a complete
    // line: the detail pane's tab bar showed `[Summary] | Network | Disks |
    // Metadata` with a fifth tab, JSON, silently gone.
    let budget = width - OVERFLOW_MARKER.len();
    let mut remaining = budget;
    let mut fitted = Vec::with_capacity(spans.len() + 1);
    for span in spans {
        if remaining == 0 {
            break;
        }
        let length = length_of(&span);
        if length <= remaining {
            remaining -= length;
            fitted.push(span);
            continue;
        }
        // Truncated without a marker of its own: the one appended below is the
        // line's, and two in a row would spend columns saying the same thing.
        let (head, _) = span.content.unicode_truncate(remaining);
        fitted.push(Span::styled(head.to_string(), span.style));
        remaining = 0;
    }
    fitted.push(Span::raw(OVERFLOW_MARKER));
    Line::from(fitted)
}

/// What a fitted string ends in when it did not fit. Shared with [`fit_text`],
/// which is what makes a truncated single-span line and a truncated multi-span
/// one look the same to the operator.
const OVERFLOW_MARKER: &str = "...";

/// Wraps `text` to `width` display columns without altering a single character.
///
/// Deliberately not [`crate::ui::shared::help`]'s `wrap_text`, which normalises
/// and collapses runs of whitespace: that is right for a prose paragraph and
/// wrong here. These lines carry the exact argv a confirmation is about to run,
/// and a modal that silently collapsed two spaces inside an argument would
/// misrepresent the command in the one surface whose whole job is showing it
/// truthfully.
///
/// So this only ever *splits*: `wrap_exact(text, w).concat() == text` for every
/// input, which `wrap_exact_never_alters_the_text` states over generated ones.
/// A break is taken after the last space that fits, so a token is not split
/// when it need not be; a token longer than `width` is split mid-token, because
/// the alternative is a line wider than the pane.
pub fn wrap_exact(text: &str, width: usize) -> Vec<String> {
    if width == 0 || UnicodeWidthStr::width(text) <= width {
        return vec![text.to_string()];
    }

    let mut lines = Vec::new();
    let mut rest = text;
    while UnicodeWidthStr::width(rest) > width {
        let (head, _) = rest.unicode_truncate(width);
        // `head` is empty only when a single glyph is wider than the whole
        // width. Taking one character then keeps the loop making progress
        // instead of spinning on a prefix that never fits.
        let boundary = if head.is_empty() {
            rest.chars()
                .next()
                .map(char::len_utf8)
                .unwrap_or(rest.len())
        } else {
            head.len()
        };
        let split = rest[..boundary]
            .rfind(' ')
            .map(|index| index + 1)
            .unwrap_or(boundary);
        let (line, tail) = rest.split_at(split);
        lines.push(line.to_string());
        rest = tail;
    }
    if !rest.is_empty() {
        lines.push(rest.to_string());
    }
    lines
}
