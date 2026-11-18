//! Evidence renderer (issue #36): drives the REAL `nuvem::ui::render` path with
//! configured themes and rebound hotkeys, then writes each rendered frame to an
//! HTML file (true colors + rebound key labels) under the evidence
//! directory. It exists to produce reviewer-visible visual artifacts of the
//! config-driven UI, but the claims those artifacts illustrate are not just
//! illustrated — they are asserted.
//!
//! Every assertion runs unconditionally. When `NUVEM_EVIDENCE_DIR` is set the
//! frames are also written out; with it unset the test writes nothing at all
//! and never writes outside that directory.

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use clap::Parser;
use nuvem::Cli;
use nuvem::app::reducer::root::reduce;
use nuvem::app::{Action, AppState, SystemAction, Workspace};
use nuvem::config::Config;
use nuvem::config::hotkeys::KeyOverrides;
use nuvem::config::theme::{Theme, ThemeChoice, set_active};
use nuvem::services::runtime::session::RuntimeSession;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::style::{Color, Modifier};

/// The evidence directory, or `None` when `NUVEM_EVIDENCE_DIR` is unset.
///
/// There is deliberately no fallback location: with the variable unset this
/// test writes nothing, so it cannot leave files behind on a developer's
/// machine or collide with a concurrent run.
fn evidence_dir() -> Option<PathBuf> {
    let dir = std::env::var("NUVEM_EVIDENCE_DIR").ok()?;
    let path = PathBuf::from(dir);
    fs::create_dir_all(&path).ok()?;
    Some(path)
}

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

/// Render a ratatui buffer to a standalone HTML page (one colored <span> per
/// cell run), preserving fg/bg/bold/reversed so themes are visible.
fn buffer_to_html(buffer: &Buffer, title: &str) -> String {
    let area = buffer.area;
    let mut body = String::new();
    for y in 0..area.height {
        for x in 0..area.width {
            let cell = &buffer[(x, y)];
            let reversed = cell.modifier.contains(Modifier::REVERSED);
            let (mut fg, mut bg) = (cell.fg, cell.bg);
            if reversed {
                std::mem::swap(&mut fg, &mut bg);
            }
            let fg_css = color_css(fg, "#d0d0d0");
            let bg_css = color_css(bg, "#101010");
            let mut style = format!("color:{fg_css};background:{bg_css};");
            if cell.modifier.contains(Modifier::BOLD) {
                style.push_str("font-weight:700;");
            }
            if cell.modifier.contains(Modifier::UNDERLINED) {
                style.push_str("text-decoration:underline;");
            }
            let sym = match cell.symbol() {
                "" | " " => "&nbsp;".to_string(),
                "<" => "&lt;".into(),
                ">" => "&gt;".into(),
                "&" => "&amp;".into(),
                other => other.to_string(),
            };
            body.push_str(&format!("<span style=\"{style}\">{sym}</span>"));
        }
        body.push('\n');
    }
    format!(
        "<!doctype html><meta charset=\"utf-8\"><title>{title}</title>\
<body style=\"background:#101010;margin:16px\">\
<h2 style=\"font-family:sans-serif;color:#ddd\">{title}</h2>\
<pre style=\"font:14px/1.15 'SF Mono',Menlo,Consolas,monospace;\
display:inline-block;background:#101010;padding:12px;border-radius:6px\">{body}</pre>\
</body>"
    )
}

/// Render a ratatui buffer to an SVG (a background rect grid plus one <text>
/// per cell), so `rsvg-convert`/ImageMagick can rasterize a true-color image.
fn buffer_to_svg(buffer: &Buffer, title: &str) -> String {
    let area = buffer.area;
    let (cw, ch) = (8.4_f64, 17.0_f64); // cell width/height in px (monospace)
    let pad = 14.0;
    let top = 34.0;
    let width = pad * 2.0 + cw * area.width as f64;
    let height = top + pad + ch * area.height as f64;
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
                let esc = match sym {
                    "<" => "&lt;".to_string(),
                    ">" => "&gt;".into(),
                    "&" => "&amp;".into(),
                    other => other.to_string(),
                };
                let fg_css = color_css(fg, "#d0d0d0");
                let weight = if cell.modifier.contains(Modifier::BOLD) {
                    " font-weight=\"700\""
                } else {
                    ""
                };
                let deco = if cell.modifier.contains(Modifier::UNDERLINED) {
                    " text-decoration=\"underline\""
                } else {
                    ""
                };
                texts.push_str(&format!(
                    "<text x=\"{:.2}\" y=\"{:.2}\" fill=\"{fg_css}\"{weight}{deco}>{esc}</text>",
                    px,
                    py + ch - 4.0
                ));
            }
        }
    }
    format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{width:.0}\" height=\"{height:.0}\" \
viewBox=\"0 0 {width:.0} {height:.0}\" font-family=\"SF Mono,Menlo,Consolas,monospace\" font-size=\"14\">\
<rect width=\"100%\" height=\"100%\" fill=\"#101010\"/>\
<text x=\"{pad}\" y=\"22\" fill=\"#cccccc\" font-family=\"sans-serif\" font-size=\"14\">{title}</text>\
{rects}{texts}</svg>"
    )
}

fn dump_text(buffer: &Buffer) -> String {
    let area = buffer.area;
    let mut out = String::new();
    for y in 0..area.height {
        for x in 0..area.width {
            out.push_str(buffer[(x, y)].symbol());
        }
        out.push('\n');
    }
    out
}

fn base_app() -> AppState {
    let mut app = AppState::new();
    app.root.workspace = Workspace::Home;
    app
}

fn config_with(theme: ThemeChoice, hotkeys: KeyOverrides) -> Config {
    Config {
        theme,
        hotkeys,
        ..Config::default()
    }
}

fn render(app: AppState, config: Config, w: u16, h: u16) -> Buffer {
    set_active(Theme::for_choice(config.theme));
    let cli = Cli::parse_from(["nuvem"]);
    let session = RuntimeSession::new_with_cli_and_config_for_test(cli, config, app);
    let backend = TestBackend::new(w, h);
    let mut terminal = Terminal::new(backend).expect("terminal init");
    terminal
        .draw(|frame| nuvem::ui::render(frame, &session))
        .expect("render");
    terminal.backend().buffer().clone()
}

fn footer_row(text: &str) -> String {
    // The footer is the last non-blank rendered row.
    text.lines()
        .rev()
        .map(|l| l.trim_end())
        .find(|l| !l.trim().is_empty())
        .unwrap_or_default()
        .to_string()
}

/// True when one of the footer's `"  "`-separated hints is exactly
/// `"{key} {label}"`.
///
/// `render_footer` joins each `key label` hint with a two-space separator, so
/// splitting on `"  "` isolates whole hints. Anchoring to a whole hint — not a
/// bare substring search over the row — matters because a hint that grew a
/// modifier prefix (`Ctrl-r Refresh`) still contains `r Refresh` as a plain
/// substring; splitting is what tells the two apart.
fn footer_shows(footer: &str, key: &str, label: &str) -> bool {
    let target = format!("{key} {label}");
    footer.split("  ").any(|hint| hint == target)
}

/// True when the dump contains a help-overlay row teaching exactly `key` for
/// `label` — i.e. a `"│{key}  {label}"` substring.
///
/// `binding_line` renders a row as `key`, then a two-space separator, then
/// `label`, and the row's content column sits directly against the modal's
/// own left border — so the character immediately before a real binding's
/// key is always `│`, no matter how many other borders and pipes precede it
/// earlier in the same line (the background page behind the modal keeps its
/// own borders and truncated sidebar labels, and those bleed into the
/// margins outside the modal's column). Anchoring the search to that leading
/// `│` is what a bare `"{key}  {label}"` substring search cannot do: nothing
/// immediately after any `│` in a modifier-prefixed row like
/// `Ctrl-r  Refresh` spells `r  Refresh`, so such a row cannot satisfy a
/// check written for the bare `r`.
fn help_teaches(text: &str, key: &str, label: &str) -> bool {
    text.contains(&format!("│{key}  {label}"))
}

/// The row that teaches `label` under whatever key it currently has, for
/// failure messages: the text right after some `│` in the row starts with a
/// space-free token, then two spaces, then `label`. This is deliberately
/// looser than `help_teaches` — it doesn't know the expected key, only the
/// label — so a failing assertion can show what the row actually says
/// instead of just that it didn't say the expected thing.
fn help_row_for<'a>(text: &'a str, label: &str) -> Option<&'a str> {
    text.lines().map(|l| l.trim_end()).find(|line| {
        line.match_indices('│').any(|(idx, _)| {
            let after = &line[idx + '│'.len_utf8()..];
            match after.find("  ") {
                Some(sep) if sep > 0 => after[sep + 2..].starts_with(label),
                _ => false,
            }
        })
    })
}

#[test]
fn render_config_driven_frames_to_html() {
    // 1) Default theme, default hotkeys -> footer shows the default `r refresh`.
    let default_buf = render(
        base_app(),
        config_with(ThemeChoice::Default, KeyOverrides::empty()),
        100,
        20,
    );

    // 2) High-contrast theme + rebound hotkeys (home->H, refresh->R) ->
    //    footer/help reflect the rebound keys and colors change.
    let mut raw = BTreeMap::new();
    raw.insert("home".to_string(), "H".to_string());
    raw.insert("refresh".to_string(), "R".to_string());
    let overrides = KeyOverrides::from_file(&raw).expect("sample overrides valid");
    let hc_buf = render(
        base_app(),
        config_with(ThemeChoice::HighContrast, overrides.clone()),
        100,
        20,
    );

    // 3) Help overlay under overrides -> the rebound keys are taught in help.
    let mut help_app = base_app();
    reduce(&mut help_app, &Action::System(SystemAction::ToggleHelp));
    let help_buf = render(
        help_app.clone(),
        config_with(ThemeChoice::Default, overrides),
        100,
        28,
    );

    // 3b) The same help overlay under default hotkeys, rendered only to prove
    //     the rebound assertions below are differential rather than always-true.
    let help_default_buf = render(
        help_app,
        config_with(ThemeChoice::Default, KeyOverrides::empty()),
        100,
        28,
    );

    // 4) No-color theme -> no fg/bg colors, relies on markers + emphasis.
    let nocolor_buf = render(
        base_app(),
        config_with(ThemeChoice::NoColor, KeyOverrides::empty()),
        100,
        20,
    );

    let default_text = dump_text(&default_buf);
    let hc_text = dump_text(&hc_buf);
    let help_text = dump_text(&help_buf);
    let help_default_text = dump_text(&help_default_buf);

    // ---- Footer: default hotkeys show lowercase `r`, never the rebound `R`.
    let default_footer = footer_row(&default_text);
    assert!(
        footer_shows(&default_footer, "r", "Refresh"),
        "default footer row must show the hint 'r Refresh', got: {default_footer}"
    );
    assert!(
        !footer_shows(&default_footer, "R", "Refresh"),
        "default footer row must not show the rebound hint 'R Refresh', got: {default_footer}"
    );

    // ---- Footer: rebound hotkeys show uppercase `R`, never the default `r`.
    let hc_footer = footer_row(&hc_text);
    assert!(
        footer_shows(&hc_footer, "R", "Refresh"),
        "rebound footer row must show the hint 'R Refresh', got: {hc_footer}"
    );
    assert!(
        !footer_shows(&hc_footer, "r", "Refresh"),
        "rebound footer row must not show the default hint 'r Refresh', got: {hc_footer}"
    );

    // ---- Help overlay: default hotkeys teach the default letters.
    assert!(
        help_teaches(&help_default_text, "h", "Home"),
        "default help overlay must teach 'h  Home', closest row was: {:?}",
        help_row_for(&help_default_text, "Home")
    );
    assert!(
        help_teaches(&help_default_text, "r", "Refresh"),
        "default help overlay must teach 'r  Refresh', closest row was: {:?}",
        help_row_for(&help_default_text, "Refresh")
    );

    // ---- Help overlay: rebound hotkeys teach the rebound letters, and only
    //      those — a config that failed to reach the overlay would still pass
    //      the two assertions above but fail these.
    assert!(
        help_teaches(&help_text, "H", "Home"),
        "rebound help overlay must teach 'H  Home', closest row was: {:?}",
        help_row_for(&help_text, "Home")
    );
    assert!(
        help_teaches(&help_text, "R", "Refresh"),
        "rebound help overlay must teach 'R  Refresh', closest row was: {:?}",
        help_row_for(&help_text, "Refresh")
    );
    assert!(
        !help_teaches(&help_text, "h", "Home"),
        "rebound help overlay must not still teach the default 'h  Home', row was: {:?}",
        help_row_for(&help_text, "Home")
    );
    assert!(
        !help_teaches(&help_text, "r", "Refresh"),
        "rebound help overlay must not still teach the default 'r  Refresh', row was: {:?}",
        help_row_for(&help_text, "Refresh")
    );

    // ---- No-color theme: every cell renders as Color::Reset — no fg/bg leaks
    //      through, so the frame relies entirely on markers and emphasis.
    let nocolor_area = nocolor_buf.area;
    let mut offending = None;
    'scan: for y in 0..nocolor_area.height {
        for x in 0..nocolor_area.width {
            let cell = &nocolor_buf[(x, y)];
            if cell.fg != Color::Reset || cell.bg != Color::Reset {
                offending = Some((x, y, cell.fg, cell.bg));
                break 'scan;
            }
        }
    }
    assert!(
        offending.is_none(),
        "NoColor frame must render every cell with fg == bg == Color::Reset; \
         first offending cell was at {:?}",
        offending
    );

    let Some(dir) = evidence_dir() else {
        return;
    };

    let writes = [
        (
            "01_home_default_theme.html",
            &default_buf,
            "Home — default theme, default hotkeys (footer: r refresh)",
        ),
        (
            "02_home_high_contrast_rebound.html",
            &hc_buf,
            "Home — high-contrast theme + rebound hotkeys (footer: R refresh)",
        ),
        (
            "03_help_overlay_rebound_hotkeys.html",
            &help_buf,
            "Help overlay — rebound hotkeys (H home, R refresh)",
        ),
        (
            "04_home_no_color_theme.html",
            &nocolor_buf,
            "Home — no-color theme (no fg/bg, marker + emphasis only)",
        ),
    ];
    for (name, buf, title) in writes {
        fs::write(dir.join(name), buffer_to_html(buf, title)).expect("write html");
        let svg_name = name.replace(".html", ".svg");
        fs::write(dir.join(svg_name), buffer_to_svg(buf, title)).expect("write svg");
    }

    // Plain-text footer/help extracts for the PR body.
    fs::write(
        dir.join("footer_and_help_text.txt"),
        format!(
            "=== Default hotkeys — footer row ===\n{}\n\n=== Rebound (home->H, refresh->R) — footer row ===\n{}\n\n=== Help overlay under rebound hotkeys ===\n{}\n",
            footer_row(&default_text),
            footer_row(&hc_text),
            help_text,
        ),
    )
    .expect("write text");
}
