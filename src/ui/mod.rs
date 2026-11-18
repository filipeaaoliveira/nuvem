//! Rendering, and nothing else.
//!
//! `render` checks the terminal size, picks the active workspace, builds the
//! projections from [`crate::app`], and delegates to the per-workspace
//! modules. It owns the shared application frame — header, breadcrumb, body,
//! status, footer, overlays — and hands each workspace renderer only the body
//! rectangle.
//!
//! Nothing here mutates state or starts work; a widget that wants something to
//! happen dispatches an action instead. Reusable display helpers belong under
//! `shared/` and semantic visual roles under `theme`, so a colour or a
//! truncation rule is defined once rather than per workspace.
//!
//! The small-terminal fallback is a correctness requirement, not a nicety:
//! without it the shared frame lays out into rectangles it does not fit.

pub mod configurations;
pub mod finops;
pub mod home;
pub mod logging;
pub mod resources;
pub mod service_navigator;
pub mod shared;
pub mod theme;
pub mod tunnels;

use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::Line;
use ratatui::widgets::{Clear, Paragraph};

use crate::app::Workspace;
use crate::app::projection::{binding, project_app_frame_with};
use crate::app::state::configurations::ConfigurationActivationConfirmation;
use crate::app::state::plugin_output::{self, STDERR_MARKER};
use crate::app::state::resources::{
    BatchAck, BatchActionConfirmation, BatchWorkflow, Draft, K9sLaunchConfirmation,
    PluginConfirmation, PluginPrompt, ResourceActionConfirmation, ResourceCommandPreview,
    SshLaunchPreview, TunnelLaunchPreview,
};
use crate::config::hotkeys::KeyOverrides;
use crate::services::runtime::session::RuntimeSession;
use crate::ui::shared::components::{fit_text, modal_block, wrap_exact};
use nuvem_domain::gcp::diagnostics::sanitize_detail;
use nuvem_domain::gcp::k9s::CredentialStep;
use nuvem_domain::gcp::plugin_output::OutputStream;
use nuvem_domain::gcp::text::sanitize_display_text;
use nuvem_domain::gcp::tunnel::{
    TunnelKind, TunnelSpec, cloud_sql_proxy_spec, iap_tunnel_spec, ssh_local_forward_spec,
};
use unicode_width::UnicodeWidthStr;

/// Trailing marker on a palette row that read-only mode refuses. Kept as one
/// constant because its display width is what the row's label is fitted
/// around — the two must never drift apart.
const READ_ONLY_MARKER: &str = "  \u{2014} read-only";

/// The prompt modal's title, shared by the modal and the message that stands
/// in for it when the terminal is too small, so the operator sees one name for
/// one thing.
const PLUGIN_PROMPT_TITLE: &str = "plugin input";

const HELP_MIN_WIDTH: u16 = 60;
const HELP_MIN_HEIGHT: u16 = 12;
const CONFIRMATION_MODAL_PERCENT_X: u16 = 80;
const CONFIRMATION_MODAL_PERCENT_Y: u16 = 60;

/// Renders a frame, reading the clock once so everything time-dependent in it
/// agrees.
///
/// The clock read lives here rather than inside the projections for the reason
/// `session.reduce`'s health sample does: it is a runtime-layer concern, and
/// keeping it out of the projection is what lets a decaying marker be asserted
/// without sleeping.
pub fn render(frame: &mut Frame<'_>, session: &RuntimeSession) {
    render_at(frame, session, std::time::Instant::now());
}

pub fn render_at(frame: &mut Frame<'_>, session: &RuntimeSession, at: std::time::Instant) {
    let area = frame.area();
    if area.width < HELP_MIN_WIDTH || area.height < HELP_MIN_HEIGHT {
        shared::layout::render_small_terminal_message(frame);
        return;
    }

    let overrides = session_hotkeys(session);
    let projection = project_app_frame_with(&session.app().root, &overrides);
    let areas = shared::frame::split_frame(area);
    shared::frame::render_header(frame, areas.header, &projection);
    shared::frame::render_breadcrumb(frame, areas.breadcrumb, &projection);

    match session.app().root.workspace {
        Workspace::Home => home::render_home(frame, areas.body, session, at),
        Workspace::Configurations => configurations::render(frame, areas.body, session),
        Workspace::Resources => resources::render(frame, areas.body, session),
        Workspace::Logging => logging::render(frame, areas.body, session),
        Workspace::Tunnels => tunnels::render(frame, areas.body, session),
        Workspace::FinOps => finops::render(frame, areas.body, session, at),
    }

    shared::frame::render_status(frame, areas.status, &projection);
    shared::frame::render_footer(frame, areas.footer, &projection);
    render_active_overlay(frame, session);
}

fn render_active_overlay(frame: &mut Frame<'_>, session: &RuntimeSession) {
    if session.app().root.system.show_help {
        render_help(frame, session);
        return;
    }

    // Directly below help and above every confirmation, matching the mode
    // precedence in `services::runtime::input_context`: whichever overlay
    // renders is the one that must own the keys.
    if session.app().root.system.plugin_output().is_some() {
        render_plugin_output(frame, session);
        return;
    }

    if session.app().root.configurations.context_picker().is_some() {
        configurations::render_context_picker_overlay(frame, session);
        return;
    }

    if session.app().root.resources.action_confirmation().is_some() {
        render_resource_action_confirmation(frame, session);
        return;
    }

    if session.app().root.resources.batch_confirmation().is_some() {
        render_resource_batch_confirmation(frame, session);
        return;
    }

    if session
        .app()
        .root
        .configurations
        .activation_confirmation()
        .is_some()
    {
        render_configuration_activation_confirmation(frame, session);
        return;
    }

    // Immediately above the confirmation it feeds, matching the mode
    // precedence in `services::runtime::input_context`: whichever overlay
    // renders is the one that must own the keys. The two are never open at
    // once — the prompt is taken as the confirmation is shown — so this rank
    // is not resolving an overlap; it makes the prompt exclusive against
    // everything the confirmation is already exclusive against.
    if session.app().root.resources.plugin_prompt().is_some() {
        render_plugin_prompt(frame, session);
        return;
    }

    // Ahead of the command preview and matching the mode precedence in
    // `services::runtime::input_context`: whichever overlay renders is the one
    // that must own the keys. A plugin confirmation outranks the copy-only
    // preview because its `Enter` runs the command rather than copying it.
    if session.app().root.resources.plugin_confirmation().is_some() {
        render_plugin_confirmation(frame, session);
        return;
    }

    if session.app().root.resources.command_preview().is_some() {
        render_resource_command_preview(frame, session);
        return;
    }

    if session.app().root.resources.ssh_launch_preview().is_some() {
        render_ssh_launch_preview(frame, session);
        return;
    }

    if session
        .app()
        .root
        .resources
        .k9s_launch_confirmation()
        .is_some()
    {
        render_k9s_launch_confirmation(frame, session);
        return;
    }

    // Deliberately placed after the k9s launch confirmation and before the
    // command palette (ARCHITECTURE.md "Overlay precedence"): it is a launch
    // confirmation like the ones above it, and like them it must outrank the
    // palette, since the palette is one of the ways it gets opened.
    if session
        .app()
        .root
        .resources
        .tunnel_launch_preview()
        .is_some()
    {
        render_tunnel_launch_preview(frame, session);
        return;
    }

    // Ranked with the launch modals above it and ahead of the palette, matching
    // the mode precedence in `services::runtime::input_context`: whichever
    // overlay renders is the one that must own the keys.
    if session.app().root.jump_picker.is_some() {
        resources::render_jump_picker_overlay(frame, session);
        return;
    }

    if session.app().root.command.is_open() {
        render_command_palette(frame, session);
        return;
    }

    // A render-only progress glance: it never changes the input mode, so keys
    // (including opening any of the overlays above) keep working normally
    // while a batch runs. It shows only once nothing else claims the frame.
    if session.app().root.resources.batch_workflow().is_some() {
        render_resource_batch_progress(frame, session);
    }
}

fn render_help(frame: &mut Frame<'_>, session: &RuntimeSession) {
    let overrides = session_hotkeys(session);
    let model = crate::app::projection::help_model_with(&session.app().root, &overrides);
    let scroll = session.app().root.system.help_scroll;
    shared::help::render_help_overlay(frame, &model, scroll);
}

/// The session's effective hotkey overrides, or defaults when no config is
/// attached (e.g. in headless tests).
fn session_hotkeys(session: &RuntimeSession) -> KeyOverrides {
    session
        .config()
        .map(|config| config.hotkeys.clone())
        .unwrap_or_default()
}

/// The plugin output viewer: what a run printed, as lines.
///
/// Wide and tall rather than sized to its content, unlike the confirmation
/// modals: those are read at a glance and this is scrolled through, so a box
/// that changed size with the run would move the text under the operator as
/// they paged between runs.
fn render_plugin_output(frame: &mut Frame<'_>, session: &RuntimeSession) {
    let Some(viewer) = session.app().root.system.plugin_output() else {
        return;
    };
    let runs = plugin_output::retained_runs(&session.app().root.system.operations).len();

    let area = centered_rect(88, 80, frame.area());
    let inner_width = usize::from(area.width.saturating_sub(2));
    // Two chrome rows inside the borders: the heading and the footer.
    let body_height = usize::from(area.height.saturating_sub(2)).saturating_sub(2);

    let mut lines: Vec<Line<'_>> = Vec::with_capacity(body_height + 2);
    lines.push(Line::from(fit_text(
        &format!(
            "{} · {}",
            sanitize_detail(viewer.label()),
            viewer.outcome().label()
        ),
        inner_width,
    )));

    let shown = viewer
        .output()
        .lines()
        .iter()
        .skip(viewer.scroll())
        .take(body_height);
    for line in shown {
        // Marked, not coloured alone: a `no-color` theme would otherwise make
        // the two streams indistinguishable, and which stream a line came
        // from is what tells a warning from an answer.
        let text = match line.stream {
            OutputStream::Stdout => line.text.clone(),
            OutputStream::Stderr => format!("{STDERR_MARKER}{}", line.text),
        };
        lines.push(Line::from(fit_text(&text, inner_width)));
    }

    if viewer.output().is_truncated()
        && viewer.scroll() + body_height >= viewer.output().lines().len()
    {
        lines.push(Line::from(fit_text(
            &format!(
                "── output truncated, {} dropped ──",
                viewer.output().dropped_bytes_display()
            ),
            inner_width,
        )));
    }

    // A run the log has since evicted keeps its output on screen but has no
    // place left to report, and inventing one would be a lie about a list it
    // is no longer in.
    let place = match viewer.position(&session.app().root.system.operations) {
        Some(position) => format!("run {} of {}", position + 1, runs),
        None => "run no longer in the log".to_string(),
    };
    let position = format!(
        "{place} · line {} of {}",
        (viewer.scroll() + 1).min(viewer.output().lines().len().max(1)),
        viewer.output().lines().len(),
    );
    let footer = format!("{position} · ↑↓ scroll · n/p run · y copy · Esc close");

    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(lines).block(modal_block("plugin output")),
        area,
    );
    // Drawn into the block's bottom border row, the way a scroll position is
    // conventionally shown, so it costs no body line.
    let footer_area = Rect {
        x: area.x + 1,
        y: area.y + area.height.saturating_sub(1),
        width: area.width.saturating_sub(2),
        height: 1,
    };
    frame.render_widget(Paragraph::new(fit_text(&footer, inner_width)), footer_area);
}

fn render_command_palette(frame: &mut Frame<'_>, session: &RuntimeSession) {
    let root = &session.app().root;
    let overrides = session_hotkeys(session);
    let commands = root.command.visible_commands(root);
    let selected = root.command.selected_visible_command(root);
    let area = palette_area(frame.area());
    let inner_width = usize::from(area.width.saturating_sub(2));
    let command_lines: Vec<Line<'_>> = if commands.is_empty() {
        vec![
            Line::from(fit_text(
                &format!("No commands match \"{}\"", root.command.filter()),
                inner_width,
            )),
            Line::from(fit_text(
                "Type to search · try gce, gke, home, refresh",
                inner_width,
            )),
        ]
    } else {
        commands
            .into_iter()
            .map(|command| {
                let prefix = if Some(command) == selected {
                    "> "
                } else {
                    "  "
                };
                // Sanitized because a plugin's row is captioned by its own
                // `label` template rendered against the selected row, and a
                // label is deliberately exempt from the argv refusals
                // (`app::state::plugins::resolve`: an unrenderable caption
                // costs the caption, not the offer). A row value carrying a
                // newline or an ESC would therefore reach this cell verbatim
                // and be written to the terminal as a control byte. The jump
                // picker's row label, which carries the same exemption, is
                // sanitized at its own render site for the same reason
                // (`ui::resources::render_jump_picker_overlay`).
                let raw_label = crate::app::state::command::palette_display_label(command, root);
                let label = sanitize_display_text(&raw_label);
                let aliases = command.aliases();
                let text = if aliases.is_empty() {
                    format!("{prefix}{label}")
                } else {
                    format!("{prefix}{label}  ({})", aliases.join(", "))
                };
                // Blocked commands (read-only, mutating) get a trailing textual
                // marker AND dimmed styling. The marker is the load-bearing
                // half of this explanation, the dim is the polish: DIM is
                // invisible on some terminals and carries no reason on its
                // own, so a row that lost the marker but kept the dim would
                // look identical to an available one to a reviewer who
                // "simplified" it away — the operator would see a blocked row
                // with no clue why. Keep both.
                let blocked =
                    crate::app::state::command::command_blocked_by_read_only(command, root);
                let line = if blocked {
                    // The label is fitted to the space left AFTER the marker,
                    // then the marker is appended. Appending first and fitting
                    // the whole string drops the marker at exactly the widths
                    // where it matters most — a 72-column pane leaves
                    // "Fail over selected Cloud SQL insta..." with the reason
                    // truncated away. Appending after a bare `fit_text` is the
                    // same bug wearing a hat: the line then overruns the
                    // paragraph and ratatui clips the tail instead.
                    //
                    // Width is measured in display columns, not bytes: the
                    // em-dash is three bytes and one column.
                    //
                    // No "narrower than the marker" case is handled, because
                    // there is none to reach: `render` shows the small-terminal
                    // message below `HELP_MIN_WIDTH` (60) columns, and this
                    // modal is 70% of that at its smallest — 42 columns, 40
                    // inside the border, against a 13-column marker. The
                    // narrow-terminal test renders at exactly 60. The
                    // `saturating_sub` is there so a future layout change
                    // degrades to a marker-only row instead of panicking.
                    let marker_width = UnicodeWidthStr::width(READ_ONLY_MARKER);
                    let mut line = fit_text(&text, inner_width.saturating_sub(marker_width));
                    line.push_str(READ_ONLY_MARKER);
                    line
                } else {
                    fit_text(&text, inner_width)
                };
                let line = row_with_trailing_key(
                    line,
                    binding::command_key(command, root, &overrides).as_deref(),
                    inner_width,
                );
                if blocked {
                    Line::styled(line, Style::default().add_modifier(Modifier::DIM))
                } else {
                    Line::from(line)
                }
            })
            .collect::<Vec<_>>()
    };
    // The typed query, above the results and always present.
    //
    // It used to reach the screen in exactly one case — the "No commands match"
    // message below — which is backwards: the query appeared only once it had
    // failed and was hidden the whole time it was working. The operator could
    // not see their own input, could not tell a swallowed keystroke from a
    // typo, and could not tell why a command they expected was missing.
    //
    // `{draft}_` is the shape every other filterable surface in the app uses —
    // the resource filter, the configurations filter, the service navigator,
    // the context picker — so the trailing cursor means here what it means
    // there. The empty-state message keeps naming the query as well: this row
    // is the input, that line is the result.
    let query = format!("{PALETTE_QUERY_LABEL}{}_", root.command.filter());
    let mut lines = vec![Line::from(fit_text(&query, inner_width))];
    lines.extend(command_lines);

    let title = if root.read_only {
        "commands (read-only)"
    } else {
        "commands"
    };
    frame.render_widget(Clear, area);
    frame.render_widget(Paragraph::new(lines).block(modal_block(title)), area);
}

/// Rows the query line takes at the top of the palette.
const PALETTE_QUERY_ROWS: u16 = 1;

/// The palette's box: the shared percentage modal plus the query row's own
/// height.
///
/// Grown rather than shared, so the query row is paid for out of chrome instead
/// of out of a command. Without this the box kept its old height and the last
/// command in it fell off the bottom — silently, since the palette has no
/// overflow marker of its own.
fn palette_area(frame: Rect) -> Rect {
    let base = centered_rect(70, 45, frame);
    let height = base
        .height
        .saturating_add(PALETTE_QUERY_ROWS)
        .min(frame.height);
    // `y` is left where `centered_rect` put it. A clamp here would read as the
    // safety net that keeps the taller box inside the frame, and it is not one:
    // `centered_rect` leaves `(frame.height - base.height)` split above and
    // below, so there is always at least as much room below the box as above
    // it, and one more row cannot reach the bottom. The mutation gate is what
    // said so — `delete field y` survived, because nothing could make the
    // clamp fire. `the_palette_box_never_escapes_the_frame` pins the property
    // the clamp was reaching for, at every height the app draws on, so a change
    // to `centered_rect` or to `PALETTE_QUERY_ROWS` fails a test rather than
    // painting outside the frame.
    Rect { height, ..base }
}

/// Names the palette's query row. "Search" rather than "Filter" because that is
/// the word the palette's own empty-state hint already uses ("Type to search").
const PALETTE_QUERY_LABEL: &str = "Search: ";

/// Blank columns kept between a palette row's caption and the key that also
/// runs it, so the two never read as one string.
const PALETTE_KEY_GAP: usize = 1;

/// Right-aligns `key` on `line` inside `inner_width`, or returns the line
/// unchanged where there is no key or no room for one.
///
/// The key goes to the right edge so the keys form a column an eye can run
/// down, rather than a suffix that moves with every caption length. It is
/// applied after the read-only marker and after the caption has been fitted,
/// so it never costs the marker its place: a row too narrow for both keeps the
/// marker — which says why the row is refused — and drops the key, which is a
/// convenience.
///
/// Widths are display columns throughout, not bytes: a rebound key can be a
/// multi-byte token (`Ctrl-←`), and a byte count would overrun the row it is
/// meant to be right-aligned inside.
///
/// A row that would leave no gap at all is left alone rather than rendered
/// with the key against the caption, because `Refresh (refresh, reload)r`
/// reads as part of the caption and not as a key.
fn row_with_trailing_key(line: String, key: Option<&str>, inner_width: usize) -> String {
    let Some(key) = key else {
        return line;
    };
    let line_width = UnicodeWidthStr::width(line.as_str());
    let key_width = UnicodeWidthStr::width(key);
    if line_width + PALETTE_KEY_GAP + key_width > inner_width {
        return line;
    }
    let padding = inner_width - line_width - key_width;
    format!("{line}{}{key}", " ".repeat(padding))
}

fn render_resource_action_confirmation(frame: &mut Frame<'_>, session: &RuntimeSession) {
    let Some(confirmation) = session.app().root.resources.action_confirmation() else {
        return;
    };
    if !resource_action_confirmation_preview_visible(frame.area(), confirmation) {
        render_resource_action_confirmation_small_terminal_message(frame, confirmation);
        return;
    }
    let lines = resource_action_confirmation_lines(confirmation);
    let area = resource_action_confirmation_area(frame.area());
    let title = format!("confirm {} action", confirmation.resource_kind_label());
    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(modal_text(area, &lines)).block(modal_block(&title)),
        area,
    );
}

fn render_resource_batch_confirmation(frame: &mut Frame<'_>, session: &RuntimeSession) {
    let Some(confirmation) = session.app().root.resources.batch_confirmation() else {
        return;
    };
    if !resource_batch_confirmation_preview_visible(frame.area(), confirmation) {
        render_resource_batch_confirmation_small_terminal_message(frame, confirmation);
        return;
    }
    let lines = resources::resource_batch_confirmation_lines(confirmation);
    let area = resource_batch_confirmation_area(frame.area());
    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(modal_text(area, &lines))
            .scroll((confirmation.batch_confirm_scroll(), 0))
            .block(modal_block(confirmation.action().modal_border_title())),
        area,
    );
}

/// Gates rendering the batch modal. Unlike the single-row confirmation the
/// full target list can exceed the modal height (it scrolls), so this only
/// requires enough room to legibly render the header block and ack prompt.
pub fn resource_batch_confirmation_preview_visible(
    area: Rect,
    confirmation: &BatchActionConfirmation,
) -> bool {
    let header: Vec<String> = resources::resource_batch_confirmation_lines(confirmation)
        .into_iter()
        .take(7)
        .collect();
    paragraph_fits(resource_batch_confirmation_area(area), &header)
}

fn resource_batch_confirmation_area(area: Rect) -> Rect {
    centered_rect(
        CONFIRMATION_MODAL_PERCENT_X,
        CONFIRMATION_MODAL_PERCENT_Y,
        area,
    )
}

fn render_resource_batch_confirmation_small_terminal_message(
    frame: &mut Frame<'_>,
    confirmation: &BatchActionConfirmation,
) {
    let area = frame.area();
    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(confirmation.action().small_terminal_message())
            .block(modal_block(confirmation.action().modal_border_title())),
        area,
    );
}

fn render_resource_batch_progress(frame: &mut Frame<'_>, session: &RuntimeSession) {
    let Some(workflow) = session.app().root.resources.batch_workflow() else {
        return;
    };
    if !resource_batch_progress_visible(frame.area(), workflow) {
        render_resource_batch_progress_small_terminal_message(frame, workflow);
        return;
    }
    let lines = resources::resource_batch_progress_lines(workflow);
    let area = resource_batch_progress_area(frame.area());
    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(lines.join("\n")).block(modal_block("batch progress")),
        area,
    );
}

/// Gates rendering the batch progress panel. Only the header and the first
/// couple of per-target lines need to fit; `resource_batch_progress_lines`
/// itself caps the full per-target list to a readable size.
///
/// Deliberately still measured unwrapped, unlike every confirmation and preview
/// beside it. This panel's gate reads three lines and lets the rest
/// run past the bottom, so widening what counts as "fits" here would let it open
/// on a terminal too short to show the `+N more` line that says how many targets
/// are not listed — trading an honest refusal for a panel that under-reports a
/// running batch. Making the target cap respond to the available height is what
/// that would need, and it is not what the wrapping work was about.
///
/// `lines_fit_width` rather than a fuller check, for the reason recorded
/// there: width is the only thing that decides an answer here.
pub fn resource_batch_progress_visible(area: Rect, workflow: &BatchWorkflow) -> bool {
    let header: Vec<String> = resources::resource_batch_progress_lines(workflow)
        .into_iter()
        .take(3)
        .collect();
    lines_fit_width(resource_batch_progress_area(area), &header)
}

fn resource_batch_progress_area(area: Rect) -> Rect {
    centered_rect(
        CONFIRMATION_MODAL_PERCENT_X,
        CONFIRMATION_MODAL_PERCENT_Y,
        area,
    )
}

fn render_resource_batch_progress_small_terminal_message(
    frame: &mut Frame<'_>,
    workflow: &BatchWorkflow,
) {
    let area = frame.area();
    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(workflow.action().progress_small_terminal_message())
            .block(modal_block("batch progress")),
        area,
    );
}

fn render_configuration_activation_confirmation(frame: &mut Frame<'_>, session: &RuntimeSession) {
    let Some(confirmation) = session.app().root.configurations.activation_confirmation() else {
        return;
    };
    if !configuration_activation_confirmation_preview_visible(frame.area(), confirmation) {
        render_configuration_activation_confirmation_small_terminal_message(frame);
        return;
    }
    let lines = configurations::activation_confirmation_lines(confirmation);
    let area = configuration_activation_confirmation_area(frame.area());
    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(modal_text(area, &lines))
            .block(modal_block("confirm configuration activation")),
        area,
    );
}

pub fn configuration_activation_confirmation_preview_visible(
    area: Rect,
    confirmation: &ConfigurationActivationConfirmation,
) -> bool {
    let lines = configurations::activation_confirmation_lines(confirmation);
    paragraph_fits(configuration_activation_confirmation_area(area), &lines)
}

fn configuration_activation_confirmation_area(area: Rect) -> Rect {
    centered_rect(
        CONFIRMATION_MODAL_PERCENT_X,
        CONFIRMATION_MODAL_PERCENT_Y,
        area,
    )
}

fn render_configuration_activation_confirmation_small_terminal_message(frame: &mut Frame<'_>) {
    let area = frame.area();
    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new("nuvem activation confirmation needs a larger terminal")
            .block(modal_block("confirm configuration activation")),
        area,
    );
}

/// How many rows `confirmation` takes in `area` once wrapped.
///
/// Public for `modal_sizing_measures_display_width_rather_than_character_count`,
/// which states that a double-width name takes more rows than a
/// character-identical ASCII one. Before wrapping, that property showed up as one
/// fitting and the other not; with wrapping it shows up as a row count, so the
/// test needs the count.
pub fn resource_action_confirmation_rows(
    area: Rect,
    confirmation: &ResourceActionConfirmation,
) -> usize {
    let lines = resource_action_confirmation_lines(confirmation);
    modal_lines(resource_action_confirmation_area(area), &lines).len()
}

pub fn resource_action_confirmation_preview_visible(
    area: Rect,
    confirmation: &ResourceActionConfirmation,
) -> bool {
    let lines = resource_action_confirmation_lines(confirmation);
    paragraph_fits(resource_action_confirmation_area(area), &lines)
}

fn resource_action_confirmation_area(area: Rect) -> Rect {
    centered_rect(
        CONFIRMATION_MODAL_PERCENT_X,
        CONFIRMATION_MODAL_PERCENT_Y,
        area,
    )
}

fn render_resource_action_confirmation_small_terminal_message(
    frame: &mut Frame<'_>,
    confirmation: &ResourceActionConfirmation,
) {
    let area = frame.area();
    let label = confirmation.resource_kind_label();
    let message = format!("nuvem {label} confirmation needs a larger terminal");
    let title = format!("confirm {label} action");
    frame.render_widget(Clear, area);
    frame.render_widget(Paragraph::new(message).block(modal_block(&title)), area);
}

fn render_plugin_confirmation(frame: &mut Frame<'_>, session: &RuntimeSession) {
    let Some(confirmation) = session.app().root.resources.plugin_confirmation() else {
        return;
    };
    if !plugin_confirmation_visible(frame.area(), confirmation) {
        render_plugin_confirmation_small_terminal_message(frame);
        return;
    }
    let lines = plugin_confirmation_lines(confirmation);
    let area = resource_command_preview_area(frame.area());
    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(modal_text(area, &lines)).block(modal_block("run plugin")),
        area,
    );
}

pub fn plugin_confirmation_visible(area: Rect, confirmation: &PluginConfirmation) -> bool {
    let lines = plugin_confirmation_lines(confirmation);
    paragraph_fits(resource_command_preview_area(area), &lines)
}

fn render_plugin_prompt(frame: &mut Frame<'_>, session: &RuntimeSession) {
    let Some(prompt) = session.app().root.resources.plugin_prompt() else {
        return;
    };
    if !plugin_prompt_visible(frame.area(), prompt) {
        render_plugin_prompt_small_terminal_message(frame);
        return;
    }
    let lines = plugin_prompt_lines(prompt);
    let area = resource_command_preview_area(frame.area());
    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(modal_text(area, &lines)).block(modal_block(PLUGIN_PROMPT_TITLE)),
        area,
    );
}

/// Whether the typed-input prompt's modal fits the terminal, which is what the
/// keymap gates its `Enter` on.
///
/// Measured off `plugin_prompt_lines` — the same vector `render_plugin_prompt`
/// draws — so the answer cannot drift from what is on screen. An operator must
/// never submit values they could not read, and a field added later changes
/// both sides at once precisely because there is only one side.
pub fn plugin_prompt_visible(area: Rect, prompt: &PluginPrompt) -> bool {
    let lines = plugin_prompt_lines(prompt);
    paragraph_fits(resource_command_preview_area(area), &lines)
}

fn render_plugin_prompt_small_terminal_message(frame: &mut Frame<'_>) {
    let area = frame.area();
    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new("nuvem plugin input needs a larger terminal")
            .block(modal_block(PLUGIN_PROMPT_TITLE)),
        area,
    );
}

/// The modal's lines: one `String` per rendered line, the contract
/// `paragraph_fits` sizes the box from and `Paragraph::new(modal_text(area, &lines))`
/// draws.
///
/// Heading, field names and field values are all sanitized for the reason
/// [`plugin_confirmation_lines`] sanitizes its heading: any of them could
/// carry a control character out of the plugin declaration, and one embedded
/// newline would make this vector's length disagree with the number of lines
/// the paragraph draws — which is the number `plugin_prompt_visible` gates
/// `Enter` on.
///
/// The marker column is what tells the operator which field their keystrokes
/// reach, and a field holding something it cannot send says so rather than
/// leaving `Enter` looking broken — empty, or non-empty and unparseable, are
/// the same refusal here and each names what would satisfy it.
///
/// The key line names only keys that do something *right now*, the rule
/// `ARCHITECTURE.md` states for footer and help text: `Enter` appears once the
/// prompt is satisfied, and the choice keys only while the focused field is a
/// dropdown or a bool — they are ungated in the keymap but inert on a text
/// field.
fn plugin_prompt_lines(prompt: &PluginPrompt) -> Vec<String> {
    let mut lines = vec![sanitize_display_text(prompt.label()), String::new()];
    for (index, field) in prompt.fields().iter().enumerate() {
        let marker = if index == prompt.focus() { '>' } else { ' ' };
        let name = sanitize_display_text(&field.name);
        let value = match &field.draft {
            // The rule names what it wants rather than what is wrong with what
            // is there, and the typed text stays beside it: correcting the
            // value is the only way out of the refusal, so blanking the field
            // — or rewriting it to what it "should" say — would take away the
            // one thing the operator is editing. An empty field lands here too,
            // since no rule accepts nothing.
            Draft::Text { text, rule } if !rule.accepts(text) => {
                let wanted = rule.wanted();
                if text.is_empty() {
                    format!("({wanted})")
                } else {
                    format!("{} ({wanted})", sanitize_display_text(text))
                }
            }
            Draft::Text { text, .. } => sanitize_display_text(text),
            // Total by construction, as `Draft::value` documents: `choices` is
            // non-empty and `selected` only ever moves modulo its length.
            Draft::Choice { choices, selected } => {
                format!("< {} >", sanitize_display_text(&choices[*selected]))
            }
            // The same shape as a choice, and deliberately the argv text
            // rather than a friendlier `yes`/`no`: this modal is one step from
            // a confirmation whose whole job is showing the exact argv, so a
            // field reading `yes` while sending `true` would make the operator
            // reconcile two spellings of their own answer.
            Draft::Toggle { on } => format!("< {on} >"),
        };
        lines.push(format!("{marker} {name}: {value}"));
    }
    lines.push(String::new());
    lines.push(plugin_prompt_key_line(prompt));
    lines
}

fn plugin_prompt_key_line(prompt: &PluginPrompt) -> String {
    let mut keys = Vec::new();
    if prompt.is_satisfied() {
        keys.push("Enter continue");
    }
    if matches!(
        prompt
            .fields()
            .get(prompt.focus())
            .map(|field| &field.draft),
        Some(Draft::Choice { .. } | Draft::Toggle { .. })
    ) {
        keys.push("Left/Right choice");
    }
    if prompt.fields().len() > 1 {
        keys.push("Tab next field");
    }
    keys.push("Esc cancel");
    keys.join("   ")
}

fn render_plugin_confirmation_small_terminal_message(frame: &mut Frame<'_>) {
    let area = frame.area();
    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new("nuvem plugin confirmation needs a larger terminal")
            .block(modal_block("run plugin")),
        area,
    );
}

/// The modal's lines: one `String` per rendered line, the contract
/// `paragraph_fits` sizes the box from and `Paragraph::new(modal_text(area, &lines))`
/// draws.
///
/// The heading is sanitized for the reason spelled out over
/// [`resource_command_preview_lines`]: a plugin's label is a template rendered
/// against the selected row and is deliberately exempt from the argv refusals,
/// so a row value carrying a newline would make this vector's length disagree
/// with the number of lines the paragraph draws. The command line is not
/// sanitized — it carries its own display quoting from `gcloud_command_line`,
/// and the operator is approving exactly the bytes the argv holds.
///
/// The last lines name only keys that do something *right now*, which is the
/// rule `ARCHITECTURE.md` states for footer and help text:
///
/// - an ordinary plugin's ack is [`BatchAck::Simple`], so the digit and
///   `Backspace` arms in the keymap are reached but do nothing. Naming them
///   here would tell the operator to type something inert, so they are not
///   named; the per-context help catalog still lists them because it is
///   per-context rather than per-plugin and cannot make that distinction.
/// - a dangerous plugin's `Enter` is guarded on the typed count, so `Enter` is
///   named only once the count satisfies the ack. Until then the line offers
///   the two keys that do work: `Backspace` and `Esc`.
fn plugin_confirmation_lines(confirmation: &PluginConfirmation) -> Vec<String> {
    let mut lines = vec![
        sanitize_display_text(confirmation.label()),
        String::new(),
        format!("Command: {}", confirmation.command_preview()),
        String::new(),
    ];
    match confirmation.ack() {
        BatchAck::Simple => lines.push("Enter run   Esc cancel".to_string()),
        BatchAck::CountRequired { typed } => {
            lines.push(format!("Type 1 to confirm: {typed}"));
            lines.push(if confirmation.is_ack_satisfied() {
                "Enter run   Backspace edit   Esc cancel".to_string()
            } else {
                "Backspace edit   Esc cancel".to_string()
            });
        }
    }
    lines
}

fn render_resource_command_preview(frame: &mut Frame<'_>, session: &RuntimeSession) {
    let Some(preview) = session.app().root.resources.command_preview() else {
        return;
    };
    if !resource_command_preview_visible(frame.area(), preview) {
        render_resource_command_preview_small_terminal_message(frame);
        return;
    }
    let lines = resource_command_preview_lines(preview);
    let area = resource_command_preview_area(frame.area());
    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(modal_text(area, &lines)).block(modal_block("command preview")),
        area,
    );
}

pub fn resource_command_preview_visible(area: Rect, preview: &ResourceCommandPreview) -> bool {
    let lines = resource_command_preview_lines(preview);
    paragraph_fits(resource_command_preview_area(area), &lines)
}

fn resource_command_preview_area(area: Rect) -> Rect {
    centered_rect(
        CONFIRMATION_MODAL_PERCENT_X,
        CONFIRMATION_MODAL_PERCENT_Y,
        area,
    )
}

fn render_resource_command_preview_small_terminal_message(frame: &mut Frame<'_>) {
    let area = frame.area();
    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new("nuvem command preview needs a larger terminal")
            .block(modal_block("command preview")),
        area,
    );
}

/// The modal's lines: one `String` per rendered line, which is the contract
/// `paragraph_fits` sizes the box from and `Paragraph::new(modal_text(area, &lines))`
/// draws.
///
/// The heading is sanitized because a plugin's title is its `label` template
/// rendered against the selected row, and the label is deliberately exempt from
/// the argv refusals (`app::state::plugins::resolve`). A row value carrying a
/// newline would make this vector's length disagree with the number of lines
/// the paragraph draws: the box is sized for one line fewer than it renders,
/// `resource_command_preview_visible` still answers true so the keymap arms
/// `Enter`, and the `Enter copy   Esc cancel` footer falls outside the border
/// while the copy still fires — the operator confirms a command whose
/// confirmation they cannot see.
///
/// The command line is **not** sanitized. It carries its own display quoting
/// (`gcloud_command_line`, which escapes rather than strips), because the
/// operator is approving exactly the bytes the argv holds; stripping there
/// would show a line different from the one being copied.
fn resource_command_preview_lines(preview: &ResourceCommandPreview) -> Vec<String> {
    vec![
        sanitize_display_text(preview.title()),
        String::new(),
        format!("Command: {}", preview.command_preview()),
        String::new(),
        "Enter copy   Esc cancel".to_string(),
    ]
}

fn render_ssh_launch_preview(frame: &mut Frame<'_>, session: &RuntimeSession) {
    let Some(preview) = session.app().root.resources.ssh_launch_preview() else {
        return;
    };
    if !ssh_launch_preview_visible(frame.area(), preview) {
        render_ssh_launch_preview_small_terminal_message(frame);
        return;
    }
    let binary = session
        .config()
        .map(|config| config.gcloud_binary.to_string_lossy().into_owned())
        .unwrap_or_else(|| "gcloud".to_string());
    let lines = ssh_launch_preview_lines(&binary, preview);
    let area = resource_command_preview_area(frame.area());
    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(modal_text(area, &lines)).block(modal_block("launch SSH")),
        area,
    );
}

pub fn ssh_launch_preview_visible(area: Rect, preview: &SshLaunchPreview) -> bool {
    let lines = ssh_launch_preview_lines("gcloud", preview);
    paragraph_fits(resource_command_preview_area(area), &lines)
}

fn render_ssh_launch_preview_small_terminal_message(frame: &mut Frame<'_>) {
    let area = frame.area();
    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new("nuvem SSH launch preview needs a larger terminal")
            .block(modal_block("launch SSH")),
        area,
    );
}

fn ssh_launch_preview_lines(binary: &str, preview: &SshLaunchPreview) -> Vec<String> {
    let spec = crate::services::runtime::interactive::gce_ssh_spec(
        binary,
        preview.context(),
        preview.instance(),
    );
    let project = preview.context().project.as_deref();
    let mut lines = vec![
        "Launch SSH".to_string(),
        String::new(),
        format!("Executable: {}", spec.program),
        format!("Arguments: {}", spec.args.join(" ")),
        format!("Configuration: {}", preview.context().configuration),
        format!("Instance: {}", preview.instance().name),
        format!("Zone: {}", preview.instance().zone),
    ];
    if let Some(project) = project {
        lines.push(format!("Project: {project}"));
    }
    lines.push(String::new());
    lines.push("Enter launch   Esc cancel".to_string());
    lines
}

/// Shown in place of the local port in the previewed argv when the plan has no
/// preferred port: the allocator asks the OS for a free one at launch, so there
/// is no number yet and printing one would be a lie.
const AUTO_LOCAL_PORT: &str = "<auto>";

/// The port passed to the domain argv builders when the real one is not known
/// yet. It never reaches a child process — [`substitute_auto_local_port`]
/// rewrites the argument that carries it before the line is rendered.
const AUTO_LOCAL_PORT_SENTINEL: u16 = 0;

fn render_tunnel_launch_preview(frame: &mut Frame<'_>, session: &RuntimeSession) {
    let Some(preview) = session.app().root.resources.tunnel_launch_preview() else {
        return;
    };
    if !tunnel_launch_preview_visible(frame.area(), preview) {
        render_tunnel_launch_preview_small_terminal_message(frame);
        return;
    }
    let gcloud_binary = session
        .config()
        .map(|config| config.gcloud_binary.to_string_lossy().into_owned())
        .unwrap_or_else(|| "gcloud".to_string());
    let proxy_binary = session
        .config()
        .map(|config| config.cloud_sql_proxy_binary.to_string_lossy().into_owned())
        .unwrap_or_else(|| "cloud-sql-proxy".to_string());
    let lines = tunnel_launch_preview_lines(&gcloud_binary, &proxy_binary, preview);
    let area = resource_command_preview_area(frame.area());
    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(modal_text(area, &lines)).block(modal_block("start tunnel")),
        area,
    );
}

pub fn tunnel_launch_preview_visible(area: Rect, preview: &TunnelLaunchPreview) -> bool {
    let lines = tunnel_launch_preview_lines("gcloud", "cloud-sql-proxy", preview);
    paragraph_fits(resource_command_preview_area(area), &lines)
}

fn render_tunnel_launch_preview_small_terminal_message(frame: &mut Frame<'_>) {
    let area = frame.area();
    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new("nuvem tunnel launch preview needs a larger terminal")
            .block(modal_block("start tunnel")),
        area,
    );
}

/// Builds the argv the tunnel would spawn, from the same domain planners the
/// runtime uses, so what the user confirms is what runs.
fn tunnel_launch_preview_spec(
    gcloud_binary: &str,
    proxy_binary: &str,
    preview: &TunnelLaunchPreview,
) -> TunnelSpec {
    let plan = preview.plan();
    let configuration = preview.context().configuration.as_str();
    let local_port = plan
        .preferred_local_port
        .unwrap_or(AUTO_LOCAL_PORT_SENTINEL);
    match plan.kind {
        TunnelKind::IapTcp => iap_tunnel_spec(
            gcloud_binary,
            configuration,
            &plan.instance_name,
            &plan.zone,
            plan.remote_port,
            local_port,
        ),
        TunnelKind::SshLocalForward => ssh_local_forward_spec(
            gcloud_binary,
            configuration,
            &plan.instance_name,
            &plan.zone,
            local_port,
            plan.remote_port,
        ),
        TunnelKind::CloudSqlProxy => {
            cloud_sql_proxy_spec(proxy_binary, &plan.connection_name, local_port)
        }
    }
}

/// Replaces the sentinel local port with [`AUTO_LOCAL_PORT`] in the one
/// argument that carries it.
///
/// The argument is found *structurally* — by the flag that introduces it, per
/// tunnel kind — rather than by searching the joined argv for the number. An
/// instance name, zone, or connection name is free to contain a `0`, and
/// rewriting one of those would show the user an argv that is not the one that
/// will run.
fn substitute_auto_local_port(kind: TunnelKind, args: &mut [String]) {
    match kind {
        TunnelKind::IapTcp => {
            if let Some(arg) = args
                .iter_mut()
                .find(|arg| arg.starts_with("--local-host-port="))
            {
                *arg = format!("--local-host-port=localhost:{AUTO_LOCAL_PORT}");
            }
        }
        TunnelKind::SshLocalForward => {
            if let Some(index) = args.iter().position(|arg| arg == "-L")
                && let Some(forward) = args.get_mut(index + 1)
            {
                // `{local}:localhost:{remote}` — only the local half is
                // unknown, so the rest of the spec is kept verbatim.
                let remainder = forward
                    .split_once(':')
                    .map(|(_, rest)| rest.to_string())
                    .unwrap_or_default();
                *forward = format!("{AUTO_LOCAL_PORT}:{remainder}");
            }
        }
        TunnelKind::CloudSqlProxy => {
            if let Some(index) = args.iter().position(|arg| arg == "--port")
                && let Some(port) = args.get_mut(index + 1)
            {
                *port = AUTO_LOCAL_PORT.to_string();
            }
        }
    }
}

/// The modal's body. The argv is sanitized before it is rendered: it is built
/// from cloud-supplied resource names, and this is the last hop before they
/// reach the terminal.
fn tunnel_launch_preview_lines(
    gcloud_binary: &str,
    proxy_binary: &str,
    preview: &TunnelLaunchPreview,
) -> Vec<String> {
    let plan = preview.plan();
    let mut spec = tunnel_launch_preview_spec(gcloud_binary, proxy_binary, preview);
    if plan.preferred_local_port.is_none() {
        substitute_auto_local_port(plan.kind, &mut spec.args);
    }
    let local_port = match plan.preferred_local_port {
        Some(port) => format!("Local port: {port} (the next free port is used if it is taken)"),
        None => "Local port: the first free loopback port, chosen at launch".to_string(),
    };
    let mut lines = vec![
        "Start tunnel".to_string(),
        String::new(),
        format!("Kind: {}", plan.kind.label()),
        format!(
            "Target: {} -> {}",
            sanitize_display_text(&plan.target_label),
            sanitize_display_text(&plan.remote_label)
        ),
        format!("Configuration: {}", preview.context().configuration),
    ];
    if let Some(project) = preview.context().project.as_deref() {
        lines.push(format!("Project: {project}"));
    }
    lines.push(local_port);
    lines.push(String::new());
    lines.push(format!(
        "Executable: {}",
        sanitize_display_text(&spec.program)
    ));
    lines.push(format!(
        "Arguments: {}",
        sanitize_display_text(&spec.args.join(" "))
    ));
    lines.push(String::new());
    lines.push("Enter start   Esc cancel".to_string());
    lines
}

fn render_k9s_launch_confirmation(frame: &mut Frame<'_>, session: &RuntimeSession) {
    let Some(confirmation) = session.app().root.resources.k9s_launch_confirmation() else {
        return;
    };
    if !k9s_launch_confirmation_preview_visible(frame.area(), confirmation) {
        render_k9s_launch_confirmation_small_terminal_message(frame);
        return;
    }
    let binary = session
        .config()
        .map(|config| config.k9s_binary.to_string_lossy().into_owned())
        .unwrap_or_else(|| "k9s".to_string());
    let gcloud_binary = session
        .config()
        .map(|config| config.gcloud_binary.to_string_lossy().into_owned())
        .unwrap_or_else(|| "gcloud".to_string());
    let lines = k9s_launch_confirmation_lines(&gcloud_binary, &binary, confirmation);
    let area = resource_command_preview_area(frame.area());
    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(modal_text(area, &lines)).block(modal_block("open in k9s")),
        area,
    );
}

pub fn k9s_launch_confirmation_preview_visible(
    area: Rect,
    confirmation: &K9sLaunchConfirmation,
) -> bool {
    let lines = k9s_launch_confirmation_lines("gcloud", "k9s", confirmation);
    paragraph_fits(resource_command_preview_area(area), &lines)
}

fn render_k9s_launch_confirmation_small_terminal_message(frame: &mut Frame<'_>) {
    let area = frame.area();
    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new("nuvem k9s launch confirmation needs a larger terminal")
            .block(modal_block("open in k9s")),
        area,
    );
}

fn k9s_launch_confirmation_lines(
    gcloud_binary: &str,
    k9s_binary: &str,
    confirmation: &K9sLaunchConfirmation,
) -> Vec<String> {
    let target = confirmation.target();
    let run_get_credentials = confirmation.credential_step() == CredentialStep::RunGetCredentials;
    let mut lines = vec![
        "Open in k9s".to_string(),
        String::new(),
        format!("Cluster: {}", target.cluster_name),
        format!("Location: {}", target.location),
        format!("Project: {}", target.project),
        format!("Configuration: {}", target.configuration),
        format!("Context: {}", target.kube_context),
        String::new(),
    ];
    if run_get_credentials {
        lines.push("Run get-credentials first (updates local kubeconfig):".to_string());
        lines.push(format!(
            "  {} {}",
            gcloud_binary,
            target.credentials_argv().join(" ")
        ));
        lines.push("Note: get-credentials modifies your local kubeconfig.".to_string());
    } else {
        lines.push("Reuse existing kubeconfig context.".to_string());
    }
    lines.push(format!("  {} {}", k9s_binary, target.k9s_argv().join(" ")));
    lines.push(String::new());
    lines.push("Enter launch   Esc cancel".to_string());
    lines
}

fn resource_action_confirmation_lines(confirmation: &ResourceActionConfirmation) -> Vec<String> {
    let (heading, action, target_label, target, location_label, location, context, impact, call) =
        match confirmation {
            ResourceActionConfirmation::Compute(compute) => (
                "Confirm GCE action",
                compute.action().label(),
                "Instance",
                compute.instance().name.clone(),
                "Zone",
                compute.instance().zone.clone(),
                compute.context(),
                compute.impact_warning(),
                compute.call(),
            ),
            ResourceActionConfirmation::CloudSql(cloud_sql) => (
                "Confirm Cloud SQL action",
                cloud_sql.action().label(),
                "Instance",
                cloud_sql.instance().name.clone(),
                "Region",
                cloud_sql.instance().region.clone(),
                cloud_sql.context(),
                cloud_sql.impact_warning(),
                cloud_sql.call(),
            ),
        };

    let project = context.project.as_deref().unwrap_or("-");
    let mut lines = vec![
        heading.to_string(),
        String::new(),
        format!("Action: {action}"),
        format!("{target_label}: {target}"),
        format!("{location_label}: {location}"),
        format!("Configuration: {}", context.configuration),
        format!("Project: {project}"),
        format!("Impact: {impact}"),
        format!("Call: {}", call.preview()),
    ];
    lines.push("Enter confirm   Esc cancel".to_string());
    lines
}

/// The modal's lines, wrapped to its inner width.
///
/// Every modal that asks [`paragraph_fits`] whether it can draw must render
/// *these* lines, or the two disagree about how many rows the content takes and
/// the guard measures something the operator never sees.
fn modal_lines(area: Rect, lines: &[String]) -> Vec<String> {
    let inner_width = usize::from(area.width.saturating_sub(2));
    lines
        .iter()
        .flat_map(|line| wrap_exact(line, inner_width))
        .collect()
}

/// The modal's content as one string, wrapped to its inner width.
fn modal_text(area: Rect, lines: &[String]) -> String {
    modal_lines(area, lines).join("\n")
}

/// Whether `lines` can be shown whole in `area`.
///
/// The width test used to be per line, unwrapped: every line had to fit the
/// inner width on one row. The height was never what failed — a GCE stop
/// confirmation is ten short lines against room for twelve at 80x24 — but its
/// command line is ninety columns, so below a frame of about 115 the modal
/// refused to draw and the operator could neither confirm nor read what they
/// would have confirmed. The action was unavailable.
///
/// Refusing to confirm a command the operator cannot see is right and stays.
/// What changes is the trade: wrapping keeps the whole command visible *and*
/// lets the action happen, so the only refusals left are the ones where the
/// content genuinely does not fit.
fn paragraph_fits(area: Rect, lines: &[String]) -> bool {
    if area.width < 3 || area.height < 3 {
        return false;
    }

    modal_lines(area, lines).len() + 2 <= usize::from(area.height)
}

/// Whether every line fits the pane's inner width on one row.
///
/// The one gate that is not [`paragraph_fits`]; see
/// [`resource_batch_progress_visible`] for why that panel is measured unwrapped.
///
/// Width only. This began as a copy of `paragraph_fits`'s earlier body and
/// carried two more conditions, both of which the mutation gate found
/// unreachable for the single caller and neither of which any test could pin: a
/// `width < 3 || height < 3` guard, and `lines.len() + 2 <= area.height`.
///
/// The caller passes at most three lines of `resource_batch_progress_lines`
/// into a modal that is 60% of a frame at least twelve rows tall, so the height
/// test is true at every size nuvem draws on — while the header line is long
/// enough that the *width* test is what decides, at every size. Keeping
/// arithmetic that cannot change an answer only invites a later reader to trust
/// it. What the panel needs to know is whether its lines would be clipped
/// horizontally; its vertical overflow is what `MAX_BATCH_PROGRESS_TARGET_LINES`
/// and the `+N more` line already handle.
fn lines_fit_width(area: Rect, lines: &[String]) -> bool {
    let inner_width = usize::from(area.width.saturating_sub(2));
    lines.iter().all(|line| line.width() <= inner_width)
}

pub(crate) fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(area);
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(vertical[1])[1]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The Cloud SQL proxy arm of `substitute_auto_local_port`, which nothing
    /// reached: its flag is `--port` rather than the `-L` and
    /// `--local-host-port` the instance tunnels use, and its value is the
    /// *next* argv element. Both halves are asserted, because matching the
    /// wrong flag and rewriting the wrong index both produce a preview that
    /// shows an argv the child will not run.
    #[test]
    fn the_proxy_preview_replaces_the_port_value_and_nothing_else() {
        let mut args: Vec<String> = ["--address", "127.0.0.1", "--port", "5432", "proj:eu:db"]
            .iter()
            .map(|a| a.to_string())
            .collect();

        substitute_auto_local_port(TunnelKind::CloudSqlProxy, &mut args);

        assert_eq!(
            args,
            vec![
                "--address",
                "127.0.0.1",
                "--port",
                AUTO_LOCAL_PORT,
                "proj:eu:db"
            ],
            "only the value after --port may change"
        );
    }

    /// A connection name is free to contain the digits of a port, and the
    /// substitution must still key on the flag rather than on the text.
    #[test]
    fn the_proxy_preview_leaves_a_connection_name_holding_digits_alone() {
        let mut args: Vec<String> = ["--port", "5432", "proj:eu:db-5432"]
            .iter()
            .map(|a| a.to_string())
            .collect();

        substitute_auto_local_port(TunnelKind::CloudSqlProxy, &mut args);

        assert_eq!(
            args[2], "proj:eu:db-5432",
            "the positional must be untouched"
        );
    }

    /// With no `--port` present there is nothing to rewrite, and the argv has
    /// to come back exactly as it went in rather than gaining a value.
    #[test]
    fn the_proxy_preview_changes_nothing_when_the_flag_is_absent() {
        let mut args: Vec<String> = ["--address", "127.0.0.1"]
            .iter()
            .map(|a| a.to_string())
            .collect();
        let before = args.clone();

        substitute_auto_local_port(TunnelKind::CloudSqlProxy, &mut args);

        assert_eq!(args, before);
    }

    use crate::app::state::plugins::PluginId;
    use crate::app::state::resources::ResourcesState;
    use crate::config::plugins::{PluginInput, PluginInputKind};
    use nuvem_domain::gcp::configuration::{GcloudConfigurationName, GcloudContext};
    use nuvem_domain::gcp::resource::{ComputeInstance, ResourceStatus};

    fn context(project: Option<&str>) -> GcloudContext {
        GcloudContext {
            configuration: GcloudConfigurationName::new("dev"),
            account: None,
            project: project.map(|p| p.to_string()),
            region: None,
            zone: None,
        }
    }

    fn instance() -> ComputeInstance {
        ComputeInstance {
            id: "demo".to_string(),
            name: "demo".to_string(),
            zone: "us-central1-a".to_string(),
            status: ResourceStatus::Running,
            machine_type: "e2-medium".to_string(),
            internal_ip: Some("10.0.0.2".to_string()),
            external_ip: None,
        }
    }

    fn k9s_confirmation(location: &str, existing: Option<&[String]>) -> ResourcesState {
        use nuvem_domain::gcp::k9s::{resolve_credential_step, resolve_launch_target};
        use nuvem_domain::gcp::resource::{EndpointVisibility, GkeCluster, GkeClusterStatus};
        let cluster = GkeCluster {
            name: "platform-dev".to_string(),
            location: location.to_string(),
            status: GkeClusterStatus::Running,
            current_master_version: None,
            current_node_count: None,
            endpoint_visibility: EndpointVisibility::Public,
        };
        let target = resolve_launch_target(&context(Some("dev-project")), &cluster).unwrap();
        let step = resolve_credential_step(existing, &target);
        let mut resources = ResourcesState::default();
        resources.open_k9s_launch_confirmation(target, step);
        resources
    }

    /// The modal names the plugin the operator wrote, not a built-in caption.
    /// A heading hard-coded to the SSH wording would tell the operator they are
    /// about to copy an SSH command whatever the line below it says.
    #[test]
    fn the_preview_modal_shows_a_plugins_own_label_as_its_heading() {
        let preview = ResourceCommandPreview::new(
            "Describe web-01".to_string(),
            "gcloud --configuration my-proj compute instances describe web-01".to_string(),
            "command".to_string(),
        );

        let lines = resource_command_preview_lines(&preview);

        assert_eq!(lines[0], "Describe web-01");
        assert_eq!(
            lines[2],
            "Command: gcloud --configuration my-proj compute instances describe web-01"
        );
    }

    /// One compute-scoped plugin captioned by the row's name, whose argv never
    /// names it. The label is the only place `{name}` appears, so the offer
    /// resolves however hostile the name is — which is precisely the case
    /// `plugin_argv`'s refusals cannot see.
    fn plugin_captioned_by_the_row_name() -> crate::config::plugins::Plugins {
        use std::io::Write;

        let body = "version = 1\n\n[[plugins]]\nid = \"describe\"\n\
                    scope = \"GCE instances\"\nkey = \"d\"\n\
                    label = \"Describe {name}\"\n\
                    args = [\"compute\", \"instances\", \"list\", \"--zones={zone}\"]\n";
        let mut file = tempfile::NamedTempFile::new().expect("a temp plugin file");
        file.write_all(body.as_bytes()).expect("the fixture writes");

        let mut problems = crate::config::problems::ConfigProblems::default();
        let plugins = crate::config::plugins::Plugins::load(file.path(), &mut problems);
        assert!(
            problems.is_empty(),
            "fixture must be a valid plugin file: {:?}",
            problems.messages()
        );
        assert_eq!(plugins.len(), 1, "the fixture plugin must load");
        plugins
    }

    /// The open palette over one compute instance named `name`, with that
    /// plugin loaded.
    fn palette_over_instance_named(name: &str) -> RuntimeSession {
        use crate::app::reducer::root::reduce;
        use crate::app::{Action, AppState, CommandAction, EffectResult, WorkspaceAction};
        use nuvem_domain::gcp::configuration::GcloudConfiguration;

        let mut app = AppState::new();
        reduce(
            &mut app,
            &Action::EffectResult(EffectResult::ConfigurationsLoaded {
                configurations: vec![GcloudConfiguration {
                    name: GcloudConfigurationName::new("dev"),
                    is_active: true,
                    account: None,
                    project: Some("dev-project".to_string()),
                    region: None,
                    zone: Some("us-central1-a".to_string()),
                }],
            }),
        );
        let context = app
            .root
            .current_context()
            .expect("the fixture has a context");
        reduce(
            &mut app,
            &Action::EffectResult(EffectResult::ComputeInstancesLoaded {
                context,
                instances: vec![ComputeInstance {
                    name: name.to_string(),
                    ..instance()
                }],
            }),
        );
        reduce(&mut app, &Action::Workspace(WorkspaceAction::Resources));
        app.root.plugins = plugin_captioned_by_the_row_name();
        reduce(&mut app, &Action::Command(CommandAction::Open));
        assert!(
            app.root
                .command
                .visible_commands(&app.root)
                .iter()
                .any(|command| matches!(
                    command,
                    crate::app::state::command::CommandDescriptor::Plugin(_)
                )),
            "the fixture palette must list the plugin, or this asserts nothing"
        );
        RuntimeSession::new_for_test(app)
    }

    /// The key column is right-aligned and always separated from the caption:
    /// `Refresh (refresh, reload)r` would read as part of the caption rather
    /// than as a key, so a row with no room for the gap keeps the caption and
    /// drops the key.
    #[test]
    fn a_palette_rows_key_sits_at_the_right_edge_behind_a_gap() {
        assert_eq!(
            row_with_trailing_key("abc".to_string(), Some("r"), 10),
            "abc      r"
        );
        // Exactly enough for the caption, one blank column and the key.
        assert_eq!(
            row_with_trailing_key("abc".to_string(), Some("r"), 5),
            "abc r"
        );
        // One column short of that: the key goes, the caption stays whole.
        assert_eq!(
            row_with_trailing_key("abc".to_string(), Some("r"), 4),
            "abc"
        );
        // A command with no key of its own renders exactly as it was.
        assert_eq!(row_with_trailing_key("abc".to_string(), None, 10), "abc");
        // Multi-column keys are measured in columns, not bytes.
        assert_eq!(
            row_with_trailing_key("a".to_string(), Some("Ctrl-t"), 8),
            "a Ctrl-t"
        );
    }

    /// A palette row names the key that runs the same command in the view
    /// underneath, so the operator who looked "refresh" up once stops looking
    /// it up. Rows for palette-only commands carry no key column at all —
    /// see `binding::command_key` for which is which.
    #[test]
    fn a_palette_row_names_the_key_that_also_runs_it() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        use crate::app::reducer::root::reduce;
        use crate::app::{Action, AppState, CommandAction, WorkspaceAction};

        let mut app = AppState::new();
        reduce(&mut app, &Action::Workspace(WorkspaceAction::Resources));
        reduce(&mut app, &Action::Command(CommandAction::Open));
        for character in "refresh".chars() {
            reduce(
                &mut app,
                &Action::Command(CommandAction::AppendChar(character)),
            );
        }
        let session = RuntimeSession::new_for_test(app);
        let mut terminal = Terminal::new(TestBackend::new(100, 30)).expect("a test terminal");
        terminal
            .draw(|frame| render_command_palette(frame, &session))
            .expect("the palette draws");

        let buffer = terminal.backend().buffer();
        let rows: Vec<String> = (0..buffer.area.height)
            .map(|y| {
                (0..buffer.area.width)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect();

        let refresh = rows
            .iter()
            .find(|row| row.contains("Refresh"))
            .expect("the palette lists Refresh");
        assert!(
            refresh.contains('r'),
            "the Refresh row should name its key, got {refresh:?}"
        );
        assert!(
            refresh
                .trim_end()
                .trim_end_matches('\u{2502}')
                .trim_end()
                .ends_with('r'),
            "the key is the row's trailing column, got {refresh:?}"
        );
    }

    /// The palette row is captioned by the plugin's `label` rendered against
    /// the selected row, and a label is exempt from the argv refusals. A row
    /// value carrying a newline or an ESC would otherwise be written into a
    /// buffer cell verbatim and reach the terminal as a control byte, moving
    /// the cursor or starting a live escape sequence in the middle of a frame.
    #[test]
    fn a_palette_row_captioned_by_a_hostile_row_value_writes_no_control_character() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        for hostile in ["web\n01", "web\u{1b}[2J01", "web\u{7f}01"] {
            let session = palette_over_instance_named(hostile);
            let mut terminal = Terminal::new(TestBackend::new(100, 30)).expect("a test terminal");
            terminal
                .draw(|frame| render_command_palette(frame, &session))
                .expect("the palette draws");

            let buffer = terminal.backend().buffer();
            assert!(
                buffer
                    .content()
                    .iter()
                    .all(|cell| !cell.symbol().chars().any(char::is_control)),
                "a control character from {hostile:?} reached a buffer cell"
            );
            let rows: Vec<String> = (0..buffer.area.height)
                .map(|y| {
                    (0..buffer.area.width)
                        .map(|x| buffer[(x, y)].symbol())
                        .collect::<String>()
                })
                .collect();
            assert!(
                rows.iter().any(|row| row.contains("Describe web")),
                "the plugin row must still caption itself for {hostile:?}: {rows:?}"
            );
        }
    }

    /// The modal is sized from the *count* of these lines and drawn from the
    /// same lines joined by `\n`, so the two numbers have to agree whatever the
    /// heading holds. A plugin's heading is its `label` rendered against the
    /// row, and the label is exempt from the argv refusals: a plugin labelled
    /// `"Describe {name}"` whose argv never names `{name}` resolves fine over a
    /// row whose name carries a newline. The box is then sized for one line
    /// fewer than the paragraph draws, `resource_command_preview_visible` still
    /// answers true, and the footer is pushed outside the border while `Enter`
    /// stays armed and copies.
    ///
    /// The count, not the absence of a newline in the string, is what is
    /// asserted: the count is the mechanism, and it stays true of any heading
    /// this modal ever grows.
    #[test]
    fn the_preview_modal_renders_exactly_the_line_count_it_is_sized_for() {
        let preview = ResourceCommandPreview::new(
            "Describe web\n01".to_string(),
            "gcloud --configuration my-proj compute instances list".to_string(),
            "command".to_string(),
        );

        let lines = resource_command_preview_lines(&preview);
        let paragraph = lines.join("\n");

        assert_eq!(
            paragraph.lines().count(),
            lines.len(),
            "the paragraph draws {} lines in a box sized for {}: {lines:?}",
            paragraph.lines().count(),
            lines.len()
        );
        // Sized for the same five lines as any other preview, footer included.
        assert_eq!(lines.len(), 5);
        assert_eq!(lines[4], "Enter copy   Esc cancel");
        // The command line is the operator's own bytes and is left to
        // `gcloud_command_line`'s escaping; only the heading is sanitized.
        assert_eq!(
            lines[2],
            "Command: gcloud --configuration my-proj compute instances list"
        );
    }

    #[test]
    fn k9s_confirmation_lines_show_both_argvs_and_mutation_note_when_creating() {
        let resources = k9s_confirmation("us-central1-a", None);
        let confirmation = resources.k9s_launch_confirmation().unwrap();
        let lines = k9s_launch_confirmation_lines("gcloud", "k9s", confirmation);

        assert!(
            lines
                .iter()
                .any(|line| line.contains("gcloud") && line.contains("get-credentials"))
        );
        assert!(
            lines
                .iter()
                .any(|line| line.starts_with("  k9s ") && line.contains("--context"))
        );
        assert!(
            lines
                .iter()
                .any(|line| line.contains("modifies your local kubeconfig"))
        );
        assert!(lines.contains(&"Enter launch   Esc cancel".to_string()));
    }

    #[test]
    fn k9s_confirmation_lines_reuse_omit_get_credentials_and_note() {
        let existing = vec!["gke_dev-project_us-central1_platform-dev".to_string()];
        let resources = k9s_confirmation("us-central1", Some(&existing));
        let confirmation = resources.k9s_launch_confirmation().unwrap();
        let lines = k9s_launch_confirmation_lines("gcloud", "k9s", confirmation);

        assert!(lines.iter().any(|line| line.contains("Reuse existing")));
        assert!(!lines.iter().any(|line| line.contains("get-credentials")));
        assert!(
            !lines
                .iter()
                .any(|line| line.contains("modifies your local kubeconfig"))
        );
        assert!(
            lines
                .iter()
                .any(|line| line.starts_with("  k9s ") && line.contains("--context"))
        );
    }

    #[test]
    fn ssh_launch_preview_lines_include_program_arguments_and_project_when_set() {
        let mut resources = ResourcesState::default();
        resources.show_ssh_launch_preview(context(Some("proj")), instance());
        let preview = resources.ssh_launch_preview().expect("preview set");
        let lines = ssh_launch_preview_lines("gcloud", preview);

        assert!(lines.contains(&"Executable: gcloud".to_string()));
        assert!(lines.iter().any(|line| line.starts_with("Arguments: ")
            && line.contains("--configuration")
            && line.contains("compute ssh")));
        assert!(lines.contains(&"Project: proj".to_string()));
    }

    fn tunnel_plan(
        kind: nuvem_domain::gcp::tunnel::TunnelKind,
        preferred_local_port: Option<u16>,
    ) -> crate::app::TunnelLaunchPlan {
        crate::app::TunnelLaunchPlan {
            kind,
            target_label: "db-dev-1".to_string(),
            remote_label: "5432".to_string(),
            instance_name: "demo".to_string(),
            zone: "us-central1-a".to_string(),
            connection_name: "proj:us-central1:db-dev-1".to_string(),
            preferred_local_port,
            remote_port: 5432,
            claimed_ports: Vec::new(),
        }
    }

    fn tunnel_preview(
        kind: nuvem_domain::gcp::tunnel::TunnelKind,
        preferred_local_port: Option<u16>,
    ) -> ResourcesState {
        let mut resources = ResourcesState::default();
        resources.show_tunnel_launch_preview(
            context(Some("proj")),
            tunnel_plan(kind, preferred_local_port),
        );
        resources
    }

    /// An instance tunnel has no conventional port, so the argv shows the
    /// placeholder the allocator fills in rather than a number that would be
    /// a lie.
    #[test]
    fn tunnel_launch_preview_lines_show_the_iap_argv_with_an_automatic_local_port() {
        use nuvem_domain::gcp::tunnel::TunnelKind;
        let resources = tunnel_preview(TunnelKind::IapTcp, None);
        let preview = resources.tunnel_launch_preview().expect("preview set");
        let lines = tunnel_launch_preview_lines("gcloud", "cloud-sql-proxy", preview);

        assert!(lines.contains(&"Executable: gcloud".to_string()));
        assert!(
            lines.iter().any(|line| line.starts_with("Arguments: ")
                && line.contains("compute start-iap-tunnel")
                && line.contains("--local-host-port=localhost:<auto>")),
            "argv line missing or still shows a port the allocator has not picked: {lines:?}"
        );
        assert!(
            lines
                .iter()
                .any(|line| line.starts_with("Local port:") && line.contains("free")),
            "the preview must say where the local port comes from: {lines:?}"
        );
        assert!(lines.contains(&"Project: proj".to_string()));
        assert!(lines.contains(&"Enter start   Esc cancel".to_string()));
    }

    /// The SQL proxy does have a conventional port, so the preview states it
    /// *and* that a taken port falls through to the next free one.
    #[test]
    fn tunnel_launch_preview_lines_state_the_requested_port_and_the_fallback() {
        use nuvem_domain::gcp::tunnel::TunnelKind;
        let resources = tunnel_preview(TunnelKind::CloudSqlProxy, Some(5432));
        let preview = resources.tunnel_launch_preview().expect("preview set");
        let lines = tunnel_launch_preview_lines("gcloud", "cloud-sql-proxy", preview);

        assert!(lines.contains(&"Executable: cloud-sql-proxy".to_string()));
        assert!(
            lines.iter().any(|line| line.starts_with("Arguments: ")
                && line.contains("--port 5432")
                && line.contains("proj:us-central1:db-dev-1")),
            "argv line missing the requested port: {lines:?}"
        );
        assert!(
            lines
                .iter()
                .any(|line| line.starts_with("Local port: 5432") && line.contains("next free")),
            "the preview must state the requested port and the fallback: {lines:?}"
        );
    }

    /// The SSH forward's port lives in the `-L` argument, so the substitution
    /// has to find it there rather than in a `--flag=` prefix.
    #[test]
    fn tunnel_launch_preview_lines_substitute_the_ssh_forward_port() {
        use nuvem_domain::gcp::tunnel::TunnelKind;
        let resources = tunnel_preview(TunnelKind::SshLocalForward, None);
        let preview = resources.tunnel_launch_preview().expect("preview set");
        let lines = tunnel_launch_preview_lines("gcloud", "cloud-sql-proxy", preview);

        assert!(
            lines
                .iter()
                .any(|line| line.starts_with("Arguments: ")
                    && line.contains("-L <auto>:localhost:5432")),
            "argv line missing the substituted forward spec: {lines:?}"
        );
    }

    #[test]
    fn tunnel_launch_preview_is_not_visible_in_a_small_terminal() {
        use nuvem_domain::gcp::tunnel::TunnelKind;
        let resources = tunnel_preview(TunnelKind::CloudSqlProxy, Some(5432));
        let preview = resources.tunnel_launch_preview().expect("preview set");

        assert!(!tunnel_launch_preview_visible(
            Rect::new(0, 0, 40, 10),
            preview
        ));
        assert!(tunnel_launch_preview_visible(
            Rect::new(0, 0, 160, 40),
            preview
        ));
    }

    fn text_input(name: &str, default: Option<&str>) -> PluginInput {
        PluginInput {
            name: name.to_string(),
            kind: PluginInputKind::String,
            default: default.map(str::to_string),
        }
    }

    fn typed_input(name: &str, kind: PluginInputKind, default: Option<&str>) -> PluginInput {
        PluginInput {
            name: name.to_string(),
            kind,
            default: default.map(str::to_string),
        }
    }

    fn prompt_with(inputs: Vec<PluginInput>) -> PluginPrompt {
        PluginPrompt::new(
            PluginId(0),
            "Resize disk".to_string(),
            false,
            inputs,
            Vec::new(),
        )
    }

    fn prompt_with_fields(count: usize) -> PluginPrompt {
        prompt_with(
            (0..count)
                .map(|index| text_input(&format!("field_{index}"), Some("value")))
                .collect(),
        )
    }

    #[test]
    fn a_prompt_too_tall_for_the_terminal_is_not_visible() {
        let prompt = prompt_with_fields(12);
        assert!(!plugin_prompt_visible(Rect::new(0, 0, 80, 6), &prompt));
        assert!(plugin_prompt_visible(Rect::new(0, 0, 80, 40), &prompt));
    }

    /// Indexed by content rather than by line number: the assertion is that
    /// exactly the focused field carries the marker, which stays true however
    /// the heading and key lines around it are laid out.
    #[test]
    fn the_focused_field_is_marked() {
        let mut prompt = prompt_with(vec![
            text_input("disk_size", Some("100")),
            text_input("tier", Some("small")),
        ]);
        prompt.focus_next();
        let lines = plugin_prompt_lines(&prompt);

        let marked: Vec<&String> = lines.iter().filter(|line| line.starts_with('>')).collect();
        assert_eq!(
            marked.len(),
            1,
            "exactly one field is being edited: {lines:?}"
        );
        assert!(
            marked[0].contains("tier"),
            "the operator must be able to see which field they are editing: {lines:?}"
        );
    }

    #[test]
    fn an_empty_field_is_marked_as_needing_a_value() {
        let prompt = prompt_with(vec![text_input("disk_size", None)]);
        let lines = plugin_prompt_lines(&prompt);
        assert!(
            lines.iter().any(|line| line.contains("needs a value")),
            "Enter is inert, so the modal must say why: {lines:?}"
        );
        assert!(
            !lines.iter().any(|line| line.contains("Enter")),
            "an unsatisfied prompt must not name a key that does nothing: {lines:?}"
        );
    }

    /// The refusal has to keep the operator's own text on screen: correcting a
    /// mistyped value is the only way out of it, and a field blanked or
    /// rewritten to what it "should" say would be a value they did not enter.
    #[test]
    fn an_unparseable_numeric_field_shows_what_was_typed_and_what_is_wanted() {
        let mut prompt = prompt_with(vec![typed_input("size", PluginInputKind::Integer, None)]);
        for value in "1.5".chars() {
            prompt.append(value);
        }
        let lines = plugin_prompt_lines(&prompt);

        let field = lines
            .iter()
            .find(|line| line.contains("size:"))
            .unwrap_or_else(|| panic!("the field must still be drawn: {lines:?}"));
        assert!(
            field.contains("1.5"),
            "the operator corrects what they typed, so it must stay visible: {field:?}"
        );
        assert!(
            field.contains("needs a whole number"),
            "Enter is inert, so the modal must say what would satisfy it: {field:?}"
        );
        assert!(
            !lines.iter().any(|line| line.contains("Enter")),
            "an unsatisfiable prompt must not name a key that does nothing: {lines:?}"
        );
    }

    /// The other side of the marker: a field holding a value its rule accepts
    /// renders as that value and nothing else. Without this the modal could
    /// hang "needs a number" off a field that is already fine, which reads as a
    /// refusal of a prompt whose `Enter` is in fact offered.
    #[test]
    fn a_field_holding_an_acceptable_value_is_rendered_bare() {
        for (kind, typed) in [
            (PluginInputKind::String, "quarterly review"),
            (PluginInputKind::Number, "0.5"),
            (PluginInputKind::Integer, "200"),
        ] {
            let prompt = prompt_with(vec![typed_input("size", kind, Some(typed))]);
            let lines = plugin_prompt_lines(&prompt);

            let field = lines
                .iter()
                .find(|line| line.contains("size:"))
                .unwrap_or_else(|| panic!("the field must be drawn: {lines:?}"));
            assert_eq!(
                field.trim_start_matches(['>', ' ']),
                format!("size: {typed}"),
                "a value the rule accepts carries no marker"
            );
            assert!(
                lines.iter().any(|line| line.contains("Enter continue")),
                "and the prompt it satisfies must offer Enter: {lines:?}"
            );
        }
    }

    /// Each rule names what *it* wants. A `number` refusing `abc` with "needs a
    /// whole number" would send the operator to correct a precision that was
    /// never the problem.
    #[test]
    fn an_empty_numeric_field_asks_for_a_number_rather_than_for_any_value() {
        let lines = plugin_prompt_lines(&prompt_with(vec![typed_input(
            "share",
            PluginInputKind::Number,
            None,
        )]));
        assert!(
            lines.iter().any(|line| line.contains("needs a number")),
            "a number field wants a number, not any value: {lines:?}"
        );
    }

    #[test]
    fn a_satisfied_prompt_names_enter() {
        let prompt = prompt_with(vec![text_input("disk_size", Some("100"))]);
        let lines = plugin_prompt_lines(&prompt);
        assert!(
            lines.iter().any(|line| line.contains("Enter continue")),
            "a submittable prompt must say so: {lines:?}"
        );
    }

    /// `Tab` wraps within the fields, so on a one-field prompt it does nothing
    /// the operator can see — and the modal only names keys that act.
    #[test]
    fn tab_is_named_only_when_there_is_another_field_to_reach() {
        let one = plugin_prompt_lines(&prompt_with_fields(1));
        assert!(
            !one.iter().any(|line| line.contains("Tab")),
            "a single field has nowhere to tab to: {one:?}"
        );

        let two = plugin_prompt_lines(&prompt_with_fields(2));
        assert!(
            two.iter().any(|line| line.contains("Tab next field")),
            "a second field is only reachable by tabbing to it: {two:?}"
        );
    }

    /// A dropdown holds an index into its choices, so the modal shows the
    /// selected choice and the keys that move it — not a text field the
    /// operator would type into to no effect.
    #[test]
    fn a_dropdown_field_shows_its_current_choice_and_the_keys_that_move_it() {
        let prompt = prompt_with(vec![PluginInput {
            name: "tier".to_string(),
            kind: PluginInputKind::Dropdown {
                choices: vec!["db-f1-micro".to_string(), "db-g1-small".to_string()],
            },
            default: Some("db-g1-small".to_string()),
        }]);
        let lines = plugin_prompt_lines(&prompt);

        assert!(
            lines
                .iter()
                .any(|line| line.contains("tier") && line.contains("< db-g1-small >")),
            "a dropdown must render its choice, not free text: {lines:?}"
        );
        assert!(
            lines.iter().any(|line| line.contains("choice")),
            "the keys that change a focused dropdown must be named: {lines:?}"
        );
    }

    /// A bool renders the text it will send, not a friendlier `yes`/`no`: the
    /// prompt is one step from a confirmation whose whole job is showing the
    /// exact argv, and a field that reads `yes` while sending `true` would
    /// make the operator reconcile two spellings of their own answer.
    #[test]
    fn a_bool_field_shows_the_text_it_will_send_and_the_keys_that_change_it() {
        let prompt = prompt_with(vec![PluginInput {
            name: "async".to_string(),
            kind: PluginInputKind::Bool,
            default: Some("true".to_string()),
        }]);
        let lines = plugin_prompt_lines(&prompt);

        assert!(
            lines
                .iter()
                .any(|line| line.contains("async") && line.contains("< true >")),
            "a bool must render the argv text: {lines:?}"
        );
        assert!(
            lines.iter().any(|line| line.contains("Left/Right")),
            "the keys that change a focused bool must be named: {lines:?}"
        );

        // The state every bool with no declared `default` opens in, so a
        // rendering pinned to `true` would be wrong in the common case while
        // the assertion above still passed.
        let off = prompt_with(vec![PluginInput {
            name: "async".to_string(),
            kind: PluginInputKind::Bool,
            default: None,
        }]);
        assert!(
            plugin_prompt_lines(&off)
                .iter()
                .any(|line| line.contains("async") && line.contains("< false >")),
            "a false bool must render as false: {:?}",
            plugin_prompt_lines(&off)
        );
    }

    /// The predicate the keymap gates `Enter` on and the modal the operator
    /// reads are the same measurement: a prompt whose lines do not fit is the
    /// prompt the renderer refuses to draw.
    ///
    /// Swept across a height range that crosses the boundary, and asserted at
    /// every step, rather than at one size where both sides happen to agree: a
    /// hand-rolled height like `lines.len() + 4` agrees with the real
    /// measurement on a large terminal and disagrees near the boundary, so a
    /// single-point assertion would not tell the two apart.
    #[test]
    fn visibility_measures_the_lines_the_modal_draws() {
        let prompt = prompt_with_fields(12);
        let mut agreed = (0, 0);
        for height in 6..40 {
            let area = Rect::new(0, 0, 80, height);
            let measured = paragraph_fits(
                resource_command_preview_area(area),
                &plugin_prompt_lines(&prompt),
            );
            assert_eq!(
                plugin_prompt_visible(area, &prompt),
                measured,
                "the gate and the drawn modal disagree at height {height}"
            );
            if measured {
                agreed.0 += 1;
            } else {
                agreed.1 += 1;
            }
        }
        assert!(
            agreed.0 > 0 && agreed.1 > 0,
            "the sweep must cross the fits/does-not-fit boundary, \
             or it only ever compares one answer: {agreed:?}"
        );
    }

    #[test]
    fn ssh_launch_preview_lines_omit_project_when_unset() {
        let mut resources = ResourcesState::default();
        resources.show_ssh_launch_preview(context(None), instance());
        let preview = resources.ssh_launch_preview().expect("preview set");
        let lines = ssh_launch_preview_lines("gcloud", preview);

        assert!(!lines.iter().any(|line| line.starts_with("Project:")));
    }
}
