use nuvem::app::projection::HelpModel;
use nuvem::app::projection::binding::{DisplayBinding, HelpSection};
use nuvem::app::projection::help_model;
use nuvem::app::state::root::{RootState, Workspace};
use nuvem::ui::shared::help::{clamp_scroll, help_lines, help_lines_wrapped, render_help_overlay};
use nuvem::ui::theme;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthStr;

// Buffer API (confirmed against pinned ratatui in tests/ui_frame_test.rs):
// index with `buffer[(x, y)].symbol()`; area via `buffer.area`.
fn buffer_text(terminal: &Terminal<TestBackend>) -> String {
    let buf = terminal.backend().buffer();
    let area = buf.area;
    let mut out = String::new();
    for y in 0..area.height {
        for x in 0..area.width {
            out.push_str(buf[(x, y)].symbol());
        }
        out.push('\n');
    }
    out
}

#[test]
fn clamp_scroll_never_exceeds_max_offset() {
    // 30 lines, viewport 10 => max offset 20
    assert_eq!(clamp_scroll(u16::MAX, 30, 10), 20);
    assert_eq!(clamp_scroll(5, 30, 10), 5);
    assert_eq!(clamp_scroll(0, 5, 10), 0); // content shorter than viewport
}

#[test]
fn help_lines_groups_sections_with_styled_headers() {
    let model = help_model(&RootState::default());
    let lines = help_lines(&model);
    let text = lines
        .iter()
        .flat_map(|line| line.spans.iter())
        .map(|span| span.content.as_ref())
        .collect::<Vec<&str>>()
        .join("");
    assert!(text.contains("Global"));
    assert!(text.contains("Navigation"));
}

#[test]
fn help_renders_grouped_sections_at_80x24() {
    let model = help_model(&RootState::default());
    let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
    terminal
        .draw(|f| render_help_overlay(f, &model, 0))
        .unwrap();
    let text = buffer_text(&terminal);
    assert!(text.contains("Global"));
    assert!(text.contains("Navigation"));
    assert!(text.contains("help")); // block title
}

#[test]
fn help_shows_overflow_indicator_when_scrollable_at_60x12() {
    let model = help_model(&RootState::default());
    let mut terminal = Terminal::new(TestBackend::new(60, 12)).unwrap();
    terminal
        .draw(|f| render_help_overlay(f, &model, 0))
        .unwrap();
    let text = buffer_text(&terminal);
    assert!(text.contains("of ")); // "line X-Y of N"
    assert!(text.contains('▼')); // more content below
}

#[test]
fn help_overlay_clears_underlying_content() {
    // Fill the buffer with a sentinel by rendering a full-screen block first,
    // then the overlay, and assert the sentinel does not appear inside the overlay's inner area.
    let model = help_model(&RootState::default());
    let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
    terminal
        .draw(|f| {
            use ratatui::widgets::Paragraph;
            let filler = "X".repeat(80 * 24);
            f.render_widget(Paragraph::new(filler), f.area());
            render_help_overlay(f, &model, 0);
        })
        .unwrap();
    let text = buffer_text(&terminal);
    // The overlay is centered; its border/interior must not be pierced by 'X' filler.
    // Assert at least one full interior row has no 'X' (proves Clear worked).
    assert!(text.lines().any(|l| l.contains('│') && !l.contains('X')));
}

fn render_help_for(workspace: Workspace) -> String {
    let root = RootState {
        workspace,
        ..Default::default()
    };
    let model = help_model(&root);
    let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
    terminal
        .draw(|f| render_help_overlay(f, &model, 0))
        .unwrap();
    buffer_text(&terminal)
}

#[test]
fn help_overlay_content_is_mode_aware_end_to_end() {
    // Rendering the real overlay for two different views must teach each
    // view's own actions — proving the help view is genuinely mode-aware
    // through the full render path, not a static paragraph.
    let resources = render_help_for(Workspace::Resources);
    let configurations = render_help_for(Workspace::Configurations);

    assert!(resources.contains("Toggle sidebar"));
    assert!(resources.contains("Sort column"));
    assert!(!resources.contains("Create configuration"));

    assert!(configurations.contains("Create configuration"));
    assert!(configurations.contains("Edit active context"));
    assert!(!configurations.contains("Toggle sidebar"));

    assert_ne!(resources, configurations);
}

/// Renders the help overlay tall enough that no section falls below the fold,
/// so these assertions test content rather than the scroll viewport.
fn render_tall_help_for(workspace: Workspace) -> String {
    let root = RootState {
        workspace,
        ..Default::default()
    };
    let model = help_model(&root);
    let mut terminal = Terminal::new(TestBackend::new(80, 60)).unwrap();
    terminal
        .draw(|f| render_help_overlay(f, &model, 0))
        .unwrap();
    buffer_text(&terminal)
}

/// The Tunnels help view must teach the workspace's own actions, and the
/// Navigation section must advertise `Ctrl-t` (never a bare `t`, which is the
/// Logging tail toggle).
#[test]
fn help_overlay_teaches_tunnel_actions_and_the_ctrl_t_navigation_key() {
    let tunnels = render_tall_help_for(Workspace::Tunnels);

    assert!(tunnels.contains("Tunnels"), "view title missing");
    assert!(tunnels.contains("Kill tunnel"));
    assert!(tunnels.contains("Dismiss finished"));
    assert!(tunnels.contains("Ctrl-t"));
    assert!(!tunnels.contains("Toggle sidebar"));
}

/// `Ctrl-t` is global, so every workspace's help teaches it.
#[test]
fn every_workspace_help_lists_the_tunnels_navigation_key() {
    for workspace in [
        Workspace::Home,
        Workspace::Configurations,
        Workspace::Resources,
        Workspace::Logging,
        Workspace::Tunnels,
    ] {
        let rendered = render_tall_help_for(workspace);
        assert!(
            rendered.contains("Ctrl-t"),
            "help for {workspace:?} must list Ctrl-t for Tunnels"
        );
    }
}

/// `l` and `L` differ only in case, land in the same workspace, and are shown
/// in *different* sections of the overlay (`l` is Navigation, `L` is current
/// view), so the person who pressed the wrong one cannot compare two adjacent
/// rows. Each label therefore has to carry the whole distinction on its own,
/// including the other key, or the help is useless exactly when it is needed.
///
/// This is the guard for the G3 constraint: two entries both reading "Logs"
/// would satisfy every other test in the suite.
#[test]
fn resources_help_disambiguates_the_two_logging_keys_and_lists_the_jump_key() {
    let rendered = render_tall_help_for(Workspace::Resources);

    // `l`: the workspace, all project logs, and a pointer at `L`.
    assert!(
        rendered.contains("Logging workspace: all project logs (L: selection only)"),
        "help must say `l` is project-wide and name `L`:\n{rendered}"
    );
    // `L`: the selection, and a pointer back at `l`.
    assert!(
        rendered.contains("Logs for the selected resource (l: all project logs)"),
        "help must say `L` is selection-scoped and name `l`:\n{rendered}"
    );
    // Neither may collapse to a bare noun that reads as a duplicate.
    assert!(
        !rendered.contains("l  Logs\n") && !rendered.contains("L  Logs\n"),
        "neither logging key may be labelled with a bare \"Logs\":\n{rendered}"
    );

    // `J`, the related-resource jump picker, is in the same view.
    assert!(
        rendered.contains("Related-resource jumps"),
        "help must list the jump picker key:\n{rendered}"
    );
}

fn model_with_problems(problems: &[&str]) -> HelpModel {
    let mut root = RootState::default();
    root.system
        .set_config_problems(problems.iter().map(|p| (*p).to_string()).collect());
    help_model(&root)
}

fn line_text(line: &Line<'static>) -> String {
    line.spans
        .iter()
        .map(|span| span.content.as_ref())
        .collect::<String>()
}

fn lines_text(lines: &[Line<'static>]) -> Vec<String> {
    lines.iter().map(line_text).collect()
}

/// A configuration with no problems must render exactly the help it rendered
/// before the problems block existed — the block is additive, never a reflow
/// of the binding list. Pinned against a literal expected snapshot (a
/// hand-built model, so it is not brittle to catalogue changes) rather than
/// against the same function's own output, which could not detect a
/// regression.
#[test]
fn help_lines_with_no_config_problems_match_the_pre_existing_shape() {
    let model = HelpModel {
        view_title: "Home".to_string(),
        problems: Vec::new(),
        sections: vec![
            HelpSection {
                title: "Current view",
                bindings: vec![DisplayBinding {
                    key: "j/k".to_string(),
                    label: "Move selection",
                    triggers: Vec::new(),
                }],
            },
            HelpSection {
                title: "Global",
                bindings: vec![DisplayBinding {
                    key: "q".to_string(),
                    label: "Quit",
                    triggers: Vec::new(),
                }],
            },
        ],
    };

    let expected: Vec<Line<'static>> = vec![
        Line::from(Span::styled("Current view", theme::title_style())),
        Line::from(vec![
            Span::styled("j/k".to_string(), theme::key_hint()),
            Span::raw("  "),
            Span::raw("Move selection"),
        ]),
        Line::from(""),
        Line::from(Span::styled("Global", theme::title_style())),
        Line::from(vec![
            Span::styled("q".to_string(), theme::key_hint()),
            Span::raw("  "),
            Span::raw("Quit"),
        ]),
    ];

    assert_eq!(help_lines(&model), expected);
    assert_eq!(help_lines_wrapped(&model, 40), expected);

    // And the real catalogue model opens on a binding section, not the block.
    let real = help_lines(&help_model(&RootState::default()));
    assert_eq!(line_text(&real[0]), "Current view");
    assert!(
        !lines_text(&real)
            .iter()
            .any(|text| text.contains("Config problems"))
    );
}

/// The problems come first, above the binding sections, and the untouched
/// binding lines still follow them byte-for-byte.
#[test]
fn help_lines_lists_config_problems_first_and_leaves_the_bindings_intact() {
    let baseline = help_lines(&help_model(&RootState::default()));
    let lines = help_lines(&model_with_problems(&[
        "columns.run: unknown key `foo`",
        "columns.sql: duplicate key `bar`",
    ]));
    let text = lines_text(&lines);

    assert_eq!(text[0], "Config problems");
    assert_eq!(text[1], "- columns.run: unknown key `foo`");
    assert_eq!(text[2], "- columns.sql: duplicate key `bar`");
    assert_eq!(text[3], "");

    let offset = lines.len() - baseline.len();
    assert_eq!(&lines[offset..], baseline.as_slice());
}

/// The problem lines of a rendered model: everything between the block title
/// and the blank line that closes the block.
fn problem_entry_lines(text: &[String]) -> Vec<String> {
    let blank = text.iter().position(|t| t.is_empty()).unwrap();
    text[1..blank].to_vec()
}

/// Working through problems one at a time only works if an entry boundary is
/// distinguishable from a wrap, so each entry opens with `- ` and its
/// continuations are indented under it — with no blank line between entries,
/// which would cost a scroll line per problem.
#[test]
fn each_config_problem_is_bulleted_with_its_continuations_indented() {
    let lines = help_lines_wrapped(
        &model_with_problems(&[
            "first problem that is long enough to wrap over lines",
            "second problem that is also long enough to wrap",
        ]),
        24,
    );
    let entries = problem_entry_lines(&lines_text(&lines));

    let bullets: Vec<usize> = entries
        .iter()
        .enumerate()
        .filter(|(_, entry)| entry.starts_with("- "))
        .map(|(index, _)| index)
        .collect();
    // Exactly one bullet per problem, and the first entry opens the block.
    assert_eq!(bullets.len(), 2, "{entries:?}");
    assert_eq!(bullets[0], 0, "{entries:?}");
    assert!(
        entries[bullets[1]].contains("second problem"),
        "{entries:?}"
    );

    for entry in &entries {
        assert!(
            entry.starts_with("- ") || entry.starts_with("  "),
            "entry line is neither a bullet nor an indented continuation: {entry:?}"
        );
        assert!(
            !entry.is_empty(),
            "entries must not be blank-line separated"
        );
    }
    assert!(
        entries.len() > 4,
        "expected both problems to wrap: {entries:?}"
    );
}

/// Wrapping is what makes a long problem readable, so it must not silently
/// drop text: every non-whitespace character survives, in order. The bullet
/// and continuation indent this renderer adds are stripped back off before
/// the comparison, so they cannot mask a loss.
#[test]
fn help_lines_wrapped_never_loses_characters() {
    let problem = "columns.resources: the key `service_account_email_address` is not a known column \
         for this service and was skipped; supersededbyanextremelylongunbrokenidentifiername";
    let lines = help_lines_wrapped(&model_with_problems(&[problem]), 24);
    let rendered: String = problem_entry_lines(&lines_text(&lines))
        .iter()
        .map(|entry| {
            entry
                .strip_prefix("- ")
                .or_else(|| entry.strip_prefix("  "))
                .unwrap_or(entry)
                .to_string()
        })
        .collect::<Vec<String>>()
        .join("");
    let strip = |s: &str| s.chars().filter(|c| !c.is_whitespace()).collect::<String>();
    assert_eq!(
        strip(&rendered),
        strip(problem),
        "wrapping dropped characters:\n{rendered}"
    );
}

/// The status line shows a single problem verbatim and a narrow terminal
/// truncates it, so the overlay is the guarantee that nothing is lost. A
/// realistically long composed message must come back whole from a
/// realistically narrow overlay: message *length* must not be able to recreate
/// the information loss that message *count* used to cause.
#[test]
fn a_long_composed_problem_survives_a_narrow_overlay_whole() {
    let problem = "columns.gce: the same service is also configured as columns.compute, \
                   columns.compute_instances; columns.gce decides (blocks are read in \
                   alphabetical key order, not file order); columns.compute is itself invalid \
                   (unknown column \"zzz\"); columns.compute_instances is itself invalid \
                   (unknown sort column \"yyy\")";
    let lines = help_lines_wrapped(&model_with_problems(&[problem]), 60);
    let rendered: String = problem_entry_lines(&lines_text(&lines))
        .iter()
        .map(|entry| {
            entry
                .strip_prefix("- ")
                .or_else(|| entry.strip_prefix("  "))
                .unwrap_or(entry)
                .to_string()
        })
        .collect::<Vec<String>>()
        .join("");
    let strip = |s: &str| s.chars().filter(|c| !c.is_whitespace()).collect::<String>();

    assert!(
        problem.len() > 60,
        "the message must be long enough to need wrapping"
    );
    assert_eq!(
        strip(&rendered),
        strip(problem),
        "the overlay lost part of the problem:\n{rendered}"
    );
}

#[test]
fn help_lines_wrapped_respects_the_requested_width() {
    let problem = "a somewhat long configuration problem message that has to be wrapped";
    let width = 20usize;
    let lines = help_lines_wrapped(
        &model_with_problems(&[problem]),
        u16::try_from(width).unwrap(),
    );
    let entries = problem_entry_lines(&lines_text(&lines));
    assert!(entries.len() > 1, "expected the message to wrap");
    for entry in &entries {
        // The wrapper measures display columns, so the assertion must too.
        assert!(
            UnicodeWidthStr::width(entry.as_str()) <= width,
            "line wider than {width}: {entry:?}"
        );
    }
}

#[test]
fn help_lines_wrapped_hard_splits_words_longer_than_the_width() {
    // Width 5 leaves 3 columns for the text once the bullet/indent is on.
    let problem = "abcdefghij";
    let lines = help_lines_wrapped(&model_with_problems(&[problem]), 5);
    let entries = problem_entry_lines(&lines_text(&lines));
    assert_eq!(entries, ["- abc", "  def", "  ghi", "  j"]);
}

/// A problem message carrying an embedded newline must not reach a buffer cell
/// as a control glyph, including on the short-message path that skips wrapping.
#[test]
fn embedded_newlines_in_a_problem_are_normalised_to_spaces() {
    let lines = help_lines_wrapped(&model_with_problems(&["short\nproblem"]), 40);
    let entries = problem_entry_lines(&lines_text(&lines));
    assert_eq!(entries, ["- short problem"]);
    assert!(
        !entries
            .iter()
            .any(|entry| entry.chars().any(|c| c.is_control()))
    );
}

/// A whole terminal escape sequence must be removed, not disarmed: collapsing
/// the introducing ESC to a space first would leave `[31m` on screen as
/// readable text, which is exactly the residue the sanitizer exists to avoid.
#[test]
fn terminal_escapes_in_a_problem_are_removed_rather_than_left_as_residue() {
    let lines = help_lines_wrapped(
        &model_with_problems(&["columns.run: \u{1b}[31mred\u{1b}[0m value"]),
        60,
    );
    let entries = problem_entry_lines(&lines_text(&lines));

    assert_eq!(entries, ["- columns.run: red value"]);
}

#[test]
fn help_lines_wrapped_survives_absurdly_small_widths() {
    for width in [0u16, 1, 2] {
        let lines = help_lines_wrapped(&model_with_problems(&["a long problem message"]), width);
        assert!(!lines.is_empty(), "width {width} produced no lines");
    }
}

/// End to end: a recorded problem reaches the rendered overlay.
#[test]
fn help_overlay_renders_recorded_config_problems() {
    let model = model_with_problems(&["columns.run: unknown key `foo`"]);
    let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
    terminal
        .draw(|f| render_help_overlay(f, &model, 0))
        .unwrap();
    let text = buffer_text(&terminal);
    assert!(text.contains("Config problems"), "{text}");
    assert!(text.contains("columns.run"), "{text}");
}

/// The jump picker owns the keyboard while it is up, so its own context must
/// teach every key that works there — a modal whose help is the list's help
/// would name keys that do nothing.
#[test]
fn the_jump_picker_context_teaches_only_its_own_keys() {
    use nuvem::app::projection::binding::{HelpContext, context_bindings};

    let labels: Vec<&str> = context_bindings(HelpContext::ResourceJumpPicker)
        .iter()
        .map(|binding| binding.label)
        .collect();

    assert!(labels.contains(&"Move selection"), "{labels:?}");
    assert!(labels.contains(&"Jump to selected"), "{labels:?}");
    assert!(labels.contains(&"Cancel"), "{labels:?}");
    // Nothing from the list behind it: `Enter` in the picker jumps, it does
    // not open the row the resource list has selected.
    assert!(!labels.contains(&"Open"), "{labels:?}");
}
