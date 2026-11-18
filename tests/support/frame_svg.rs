//! Renders a ratatui buffer to a standalone SVG, so a terminal frame produced
//! by the real `nuvem::ui::render` path can be looked at as an image instead of
//! being described in prose.
//!
//! `tests/config_evidence.rs` carries its own copy of this for issue #36; this
//! one lives under `support/` so the newer evidence binaries can share it
//! without becoming test targets of their own.
#![allow(dead_code)]

use ratatui::buffer::Buffer;
use ratatui::style::{Color, Modifier};

fn color_css(color: Color, fallback: &str) -> String {
    match color {
        Color::Reset => fallback.to_string(),
        Color::Black => "#000000".into(),
        Color::Red => "#cd0000".into(),
        Color::Green => "#00cd00".into(),
        Color::Yellow => "#cdcd00".into(),
        Color::Blue => "#0000ee".into(),
        Color::Magenta => "#cd00cd".into(),
        Color::Cyan => "#00cdcd".into(),
        Color::Gray => "#e5e5e5".into(),
        Color::DarkGray => "#7f7f7f".into(),
        Color::LightRed => "#ff0000".into(),
        Color::LightGreen => "#00ff00".into(),
        Color::LightYellow => "#ffff00".into(),
        Color::LightBlue => "#5c5cff".into(),
        Color::LightMagenta => "#ff00ff".into(),
        Color::LightCyan => "#00ffff".into(),
        Color::White => "#ffffff".into(),
        Color::Rgb(r, g, b) => format!("#{r:02x}{g:02x}{b:02x}"),
        Color::Indexed(i) => format!("var(--idx-{i}, {fallback})"),
    }
}

fn escape(symbol: &str) -> String {
    match symbol {
        "<" => "&lt;".to_string(),
        ">" => "&gt;".into(),
        "&" => "&amp;".into(),
        other => other.to_string(),
    }
}

/// A background rect grid plus one `<text>` per non-blank cell, preserving
/// fg/bg/bold/reversed so the cursor row and the theme are visible.
pub(crate) fn buffer_to_svg(buffer: &Buffer, title: &str, caption: &str) -> String {
    let area = buffer.area;
    let (cw, ch) = (8.4_f64, 17.0_f64); // cell width/height in px (monospace)
    let pad = 14.0;
    let top = 52.0;
    // A square canvas with the frame in its top-left: macOS `qlmanage` (the
    // only SVG rasteriser on a stock Mac) renders into a square thumbnail and
    // clips a wide drawing's right-hand columns, so the slack is what keeps the
    // preview pane and the modal's right border in the image.
    let frame_width = pad * 2.0 + cw * area.width as f64;
    let frame_height = top + pad + ch * area.height as f64;
    let width = frame_width.max(frame_height);
    let height = width;
    let mut rects = String::new();
    let mut texts = String::new();
    for y in 0..area.height {
        for x in 0..area.width {
            let cell = &buffer[(x, y)];
            let reversed = cell.modifier.contains(Modifier::REVERSED);
            let (mut fg, mut bg) = (cell.fg, cell.bg);
            if reversed {
                std::mem::swap(&mut fg, &mut bg);
            }
            let px = pad + cw * x as f64;
            let py = top + ch * y as f64;
            let bg_css = color_css(bg, "#101010");
            if bg_css != "#101010" || reversed {
                rects.push_str(&format!(
                    "<rect x=\"{px:.2}\" y=\"{py:.2}\" width=\"{cw:.2}\" height=\"{ch:.2}\" fill=\"{bg_css}\"/>"
                ));
            }
            let sym = cell.symbol();
            if !sym.trim().is_empty() {
                let fg_css = color_css(fg, "#d0d0d0");
                let weight = if cell.modifier.contains(Modifier::BOLD) {
                    " font-weight=\"700\""
                } else {
                    ""
                };
                texts.push_str(&format!(
                    "<text x=\"{px:.2}\" y=\"{:.2}\" fill=\"{fg_css}\"{weight}>{}</text>",
                    py + ch - 4.0,
                    escape(sym)
                ));
            }
        }
    }
    format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{width:.0}\" height=\"{height:.0}\" \
viewBox=\"0 0 {width:.0} {height:.0}\" font-family=\"SF Mono,Menlo,Consolas,monospace\" font-size=\"14\">\
<rect width=\"100%\" height=\"100%\" fill=\"#101010\"/>\
<text x=\"{pad}\" y=\"20\" fill=\"#eeeeee\" font-family=\"sans-serif\" font-size=\"15\" font-weight=\"700\">{title}</text>\
<text x=\"{pad}\" y=\"40\" fill=\"#9aa0a6\" font-family=\"sans-serif\" font-size=\"13\">{caption}</text>\
{rects}{texts}</svg>"
    )
}
