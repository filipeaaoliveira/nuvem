use nuvem_domain::gcp::text::sanitize_rendered_paragraph;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::app::projection::HelpModel;
use crate::app::projection::binding::{
    HELP_CLOSE, HELP_JUMP, HELP_PAGE, HELP_SCROLL, HelpContext, context_bindings,
};
use crate::ui::shared::components::modal_block;
use crate::ui::theme;

/// Minimum inner width/height the overlay needs to render its layout rather
/// than the small-terminal guard message.
const MIN_INNER_WIDTH: u16 = 20;
const MIN_INNER_HEIGHT: u16 = 6;

/// Clamps a requested scroll offset so it never scrolls past the point where
/// the last line of content is flush with the bottom of the viewport.
pub fn clamp_scroll(scroll: u16, total_lines: usize, viewport: u16) -> u16 {
    let max_offset = total_lines.saturating_sub(usize::from(viewport));
    let max_offset = u16::try_from(max_offset).unwrap_or(u16::MAX);
    scroll.min(max_offset)
}

/// Title of the block listing the configuration problems recorded by the most
/// recent config load or reload.
const PROBLEMS_TITLE: &str = "Config problems";

/// Opens each problem entry. Without it a wrap and an entry boundary look
/// identical, and the whole point of the list is working through the problems
/// one at a time.
const PROBLEM_BULLET: &str = "- ";

/// Indents an entry's continuation lines under its bullet. Deliberately not a
/// blank line between entries, which would cost a scroll line per problem.
const PROBLEM_INDENT: &str = "  ";

/// Renders a `HelpModel` into styled, groupable lines without wrapping the
/// configuration problems. Equivalent to [`help_lines_wrapped`] with an
/// unbounded width.
pub fn help_lines(model: &HelpModel) -> Vec<Line<'static>> {
    help_lines_wrapped(model, u16::MAX)
}

/// Renders a `HelpModel` into styled, groupable lines: the recorded
/// configuration problems first (when any), then a styled section header
/// followed by one `key  label` row per binding, with a blank line separating
/// blocks.
///
/// Problem messages are wrapped to `width` here rather than by the paragraph
/// widget, because the overlay's scroll window is line-based and so must count
/// the lines the user actually sees.
pub fn help_lines_wrapped(model: &HelpModel, width: u16) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    if !model.problems.is_empty() {
        lines.push(Line::from(Span::styled(
            PROBLEMS_TITLE,
            theme::title_style(),
        )));
        // The bullet and the continuation indent occupy columns, so the text
        // is wrapped to what is left and the marker joined on afterwards.
        let entry_width = usize::from(width).saturating_sub(PROBLEM_BULLET.len());
        for problem in &model.problems {
            for (index, wrapped) in wrap_text(problem, entry_width).into_iter().enumerate() {
                let marker = if index == 0 {
                    PROBLEM_BULLET
                } else {
                    PROBLEM_INDENT
                };
                lines.push(Line::from(Span::styled(
                    format!("{marker}{wrapped}"),
                    theme::status_error(),
                )));
            }
        }
        lines.push(Line::from(""));
    }
    let section_count = model.sections.len();
    for (index, section) in model.sections.iter().enumerate() {
        lines.push(Line::from(Span::styled(
            section.title,
            theme::title_style(),
        )));
        for binding in &section.bindings {
            lines.push(binding_line(binding.key.clone(), binding.label));
        }
        if index + 1 < section_count {
            lines.push(Line::from(""));
        }
    }
    lines
}

/// Wraps `text` to `width` display columns, breaking at whitespace where the
/// text allows it and hard-splitting a word that is itself wider than the
/// viewport. A width too small to make progress (zero, or narrower than a
/// single wide glyph) falls back to emitting the text as one line rather than
/// looping.
///
/// The guarantee is that no *non-whitespace* character is lost, not that the
/// output reassembles byte for byte: runs of spaces and tabs are collapsed to
/// a single space and leading/trailing whitespace is dropped, since the break
/// points come from `split_whitespace`.
///
/// Newlines, tabs and any other control characters are normalised before the
/// width is measured, so a multi-line message can neither reach a buffer cell
/// as a control glyph nor skip wrapping by way of the fits-already path.
///
/// That normalisation is [`sanitize_rendered_paragraph`]'s whole job — the
/// wrapped-surface policy, as opposed to the single-line one, which would turn
/// a newline into a glyph instead of a break point.
fn wrap_text(text: &str, width: usize) -> Vec<String> {
    let text = sanitize_rendered_paragraph(text);
    let text = text.as_str();

    if width == 0 || UnicodeWidthStr::width(text) <= width {
        return vec![text.to_string()];
    }

    let mut lines: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut current_width = 0usize;

    for word in text.split_whitespace() {
        let word_width = UnicodeWidthStr::width(word);
        if !current.is_empty() && current_width + 1 + word_width > width {
            lines.push(std::mem::take(&mut current));
            current_width = 0;
        }
        if word_width > width {
            // `current` is necessarily empty here: the check above flushed it,
            // because a word wider than the whole width cannot fit beside one.
            let mut chunks = hard_split(word, width);
            if let Some(last) = chunks.pop() {
                current_width = UnicodeWidthStr::width(last.as_str());
                current = last;
            }
            // The trailing chunk stays current so the next word can share its
            // line when it fits.
            lines.extend(chunks);
            continue;
        }
        if !current.is_empty() {
            current.push(' ');
            current_width += 1;
        }
        current.push_str(word);
        current_width += word_width;
    }

    if !current.is_empty() {
        lines.push(current);
    }
    if lines.is_empty() {
        lines.push(String::new());
    }
    lines
}

/// Splits a single word into chunks no wider than `width`, always making
/// progress even when a glyph is wider than `width`.
fn hard_split(word: &str, width: usize) -> Vec<String> {
    let mut chunks: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut current_width = 0usize;
    for ch in word.chars() {
        let char_width = UnicodeWidthChar::width(ch).unwrap_or(0);
        if !current.is_empty() && current_width + char_width > width {
            chunks.push(std::mem::take(&mut current));
            current_width = 0;
        }
        current.push(ch);
        current_width += char_width;
    }
    if !current.is_empty() {
        chunks.push(current);
    }
    chunks
}

fn binding_line(key: String, label: &'static str) -> Line<'static> {
    Line::from(vec![
        Span::styled(key, theme::key_hint()),
        Span::raw("  "),
        Span::raw(label),
    ])
}

/// Renders the bounded, scrollable, grouped help overlay: a centered
/// cleared modal with a title row, a scrolled content window, a
/// position/overflow indicator row, and a control-hint row.
pub fn render_help_overlay(frame: &mut Frame<'_>, model: &HelpModel, scroll: u16) {
    let area = crate::ui::centered_rect(90, 90, frame.area());
    frame.render_widget(Clear, area);

    let block = modal_block("help");
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if inner.width < MIN_INNER_WIDTH || inner.height < MIN_INNER_HEIGHT {
        render_small_terminal_message(frame, inner);
        return;
    }

    let title_row = Rect {
        x: inner.x,
        y: inner.y,
        width: inner.width,
        height: 1,
    };
    let hints_row = Rect {
        x: inner.x,
        y: inner.y + inner.height - 1,
        width: inner.width,
        height: 1,
    };
    let indicator_row = Rect {
        x: inner.x,
        y: inner.y + inner.height - 2,
        width: inner.width,
        height: 1,
    };
    let content_area = Rect {
        x: inner.x,
        y: inner.y + 1,
        width: inner.width,
        height: inner.height - 3,
    };

    frame.render_widget(
        Paragraph::new(Line::styled(model.view_title.clone(), theme::title_style())),
        title_row,
    );

    let lines = help_lines_wrapped(model, content_area.width);
    let total = lines.len();
    let viewport = content_area.height;
    let scroll = clamp_scroll(scroll, total, viewport);
    let start = usize::from(scroll);
    let end = total.min(start + usize::from(viewport));
    let window = lines[start..end].to_vec();
    frame.render_widget(Paragraph::new(window), content_area);

    let mut indicator = format!("line {}-{} of {total}", start + 1, end);
    if start > 0 {
        indicator.push_str(" ▲");
    }
    if end < total {
        indicator.push_str(" ▼");
    }
    frame.render_widget(
        Paragraph::new(Line::styled(indicator, theme::muted())),
        indicator_row,
    );

    frame.render_widget(Paragraph::new(hint_line()), hints_row);
}

fn hint_line() -> Line<'static> {
    let bindings = context_bindings(HelpContext::Help);
    let mut spans = Vec::new();
    for binding in [HELP_SCROLL, HELP_PAGE, HELP_JUMP, HELP_CLOSE] {
        if !bindings.contains(&binding) {
            continue;
        }
        if !spans.is_empty() {
            spans.push(Span::raw("  "));
        }
        spans.push(Span::styled(binding.key, theme::key_hint()));
        spans.push(Span::raw(" "));
        spans.push(Span::raw(binding.label));
    }
    Line::from(spans)
}

fn render_small_terminal_message(frame: &mut Frame<'_>, area: Rect) {
    frame.render_widget(Paragraph::new("help needs a larger terminal"), area);
}
