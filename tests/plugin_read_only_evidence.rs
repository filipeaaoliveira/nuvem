//! Evidence: `--read-only` admits a plugin nuvem can prove reads.
//!
//! The gate is one predicate consulted from three places — the palette's
//! submit, the keyed press, and the palette's own dimming — so this file
//! presses each door rather than testing the predicate again.

use nuvem::app::reducer::root::reduce;
use nuvem::app::state::command::CommandDescriptor;
use nuvem::app::state::plugins::PluginId;
use nuvem::app::{Action, AppState, CommandAction, ResourcesAction};
use nuvem::services::runtime::session::RuntimeSession;
use nuvem_domain::gcp::resource::ResourceServiceKind;
use ratatui::Terminal;
use ratatui::backend::TestBackend;

use crate::plugin_prefix_evidence::{load, root_with_plugin};

const ROW: &str = "vm-1";
const KEY: char = 'r';

/// The plugin's `label` rendered against the selected row, which is how its
/// palette row is found on the frame.
const PLUGIN_ROW: &str = "Probe vm-1";

/// The tail of `src/ui/mod.rs`'s own `READ_ONLY_MARKER`, restated rather than
/// imported: the constant is private, and a test that read the renderer's value
/// would agree with it however it changed. Only the tail, because the row is
/// searched with `contains` and the constant's leading padding is layout the
/// marker's meaning does not depend on.
const READ_ONLY_MARKER: &str = "\u{2014} read-only";

fn state_with(args: &str, read_only: bool) -> AppState {
    // The `args` handed to `root_with_plugin` are discarded a line below: this
    // needs the world it builds — an active context, a loaded row, a selection —
    // and the plugin set each case declares for itself, so the templates here
    // are a placeholder and nothing reads them.
    let mut app = root_with_plugin(
        ResourceServiceKind::ComputeInstances,
        KEY,
        &["placeholder"],
        ROW,
    );
    app.root.plugins = load(&format!(
        "version = 1\n\n\
         [[plugins]]\n\
         id = \"probe\"\n\
         scope = \"compute\"\n\
         key = \"r\"\n\
         label = \"Probe {{name}}\"\n\
         args = {args}\n"
    ));
    app.root.read_only = read_only;
    app
}

fn press_the_plugin_key(app: &mut AppState) {
    reduce(app, &Action::Resources(ResourcesAction::ArmPluginPrefix));
    reduce(
        app,
        &Action::Resources(ResourcesAction::PluginPrefixPress(KEY)),
    );
}

const READING: &str = "[\"compute\", \"instances\", \"describe\", \"{name}\"]";
const MUTATING: &str = "[\"compute\", \"instances\", \"delete\", \"{name}\"]";

#[test]
fn a_reading_plugin_runs_under_read_only_through_the_keyed_door() {
    let mut app = state_with(READING, true);

    press_the_plugin_key(&mut app);

    assert!(
        app.root.resources.plugin_confirmation().is_some(),
        "a plugin nuvem can prove reads must reach its confirmation"
    );
}

#[test]
fn a_mutating_plugin_is_still_refused_under_read_only() {
    let mut app = state_with(MUTATING, true);

    press_the_plugin_key(&mut app);

    assert!(
        app.root.resources.plugin_confirmation().is_none(),
        "the flag's promise is that this cannot run"
    );
    assert!(
        app.root
            .system
            .status_message
            .as_deref()
            .is_some_and(|message| message.contains("read-only")),
        "and the refusal must say why: {:?}",
        app.root.system.status_message
    );
}

/// The rendered palette, because the dimming site is a renderer: a test that
/// asks the predicate itself would pass with `src/ui/mod.rs` reverted to
/// `read_only && is_mutating()`, which is exactly the disagreement between what
/// the palette shows and what the reducer does that one shared predicate exists
/// to prevent. Read off the frame an operator sees, through the marker — the
/// load-bearing half of the explanation, since DIM is invisible on some
/// terminals.
///
/// Both directions in one walk: the reading plugin's row must be present and
/// unmarked, and the mutating plugin's row must be present and marked. Either
/// half alone passes vacuously when no plugin row is rendered at all.
fn palette_row_for_the_plugin(args: &str) -> String {
    let mut app = state_with(args, true);
    reduce(&mut app, &Action::Command(CommandAction::Open));

    let session = RuntimeSession::new_for_test(app);
    let mut terminal = Terminal::new(TestBackend::new(120, 30)).expect("terminal");
    terminal
        .draw(|frame| nuvem::ui::render(frame, &session))
        .expect("render");
    let buffer = terminal.backend().buffer().clone();
    let width = buffer.area.width as usize;

    buffer
        .content()
        .chunks(width)
        .map(|line| line.iter().map(|cell| cell.symbol()).collect::<String>())
        .find(|line| line.contains(PLUGIN_ROW))
        .unwrap_or_else(|| panic!("the plugin's palette row {PLUGIN_ROW:?} is not on screen"))
}

#[test]
fn a_reading_plugin_is_no_longer_marked_in_the_rendered_palette() {
    let row = palette_row_for_the_plugin(READING);

    assert!(
        !row.contains(READ_ONLY_MARKER),
        "a plugin nuvem can prove reads is runnable, so its row must carry no \
         refusal marker: {row:?}"
    );
}

#[test]
fn a_mutating_plugin_is_still_marked_in_the_rendered_palette() {
    let row = palette_row_for_the_plugin(MUTATING);

    assert!(
        row.contains(READ_ONLY_MARKER),
        "a plugin the mode refuses must say so on its own row: {row:?}"
    );
}

/// The refusal stays ahead of the prompt: a read-only operator must
/// not be asked for every value a plugin declares and refused afterwards. Uses
/// a plugin that NEEDS values, since a `Ready` offer is refused either way.
#[test]
fn a_refused_plugin_that_needs_values_is_refused_before_its_prompt_opens() {
    let mut app = root_with_plugin(
        ResourceServiceKind::ComputeInstances,
        KEY,
        &["compute", "instances", "describe", "{name}"],
        ROW,
    );
    app.root.plugins = load(
        "version = 1\n\n\
         [[plugins]]\n\
         id = \"probe\"\n\
         scope = \"compute\"\n\
         key = \"r\"\n\
         label = \"Probe {name}\"\n\
         args = [\"compute\", \"instances\", \"delete\", \"{name}\", \"--size\", \"{size}\"]\n\
         \n\
         [[plugins.inputs]]\n\
         name = \"size\"\n\
         type = \"integer\"\n",
    );
    app.root.read_only = true;

    press_the_plugin_key(&mut app);

    assert!(
        app.root.resources.plugin_prompt().is_none(),
        "read-only must refuse before the operator types a single value"
    );
}

/// The palette's own door, walked the way an operator walks it: open the
/// palette, land on the plugin row, submit. The reducer's `Submit` arm carries
/// its own copy of the refusal, and without a test that reaches it the arm can
/// be reverted to `read_only && is_mutating()` — which refuses a plugin nuvem
/// can prove reads — with the rest of the suite green, because every other test
/// here presses the keyed door instead.
fn submit_the_plugin_from_the_palette(app: &mut AppState) {
    reduce(app, &Action::Command(CommandAction::Open));
    let rows = app.root.command.visible_commands(&app.root).len();
    for _ in 0..rows {
        if matches!(
            app.root.command.selected_visible_command(&app.root),
            Some(CommandDescriptor::Plugin(_))
        ) {
            reduce(app, &Action::Command(CommandAction::Submit));
            return;
        }
        reduce(app, &Action::Command(CommandAction::SelectNext));
    }
    panic!(
        "the open palette offered no plugin row: {:?}",
        app.root.command.visible_commands(&app.root)
    );
}

#[test]
fn a_reading_plugin_runs_under_read_only_through_the_palettes_door() {
    let mut app = state_with(READING, true);

    submit_the_plugin_from_the_palette(&mut app);

    assert!(
        app.root.resources.plugin_confirmation().is_some(),
        "the palette's arm must admit what the predicate admits: {:?}",
        app.root.system.status_message
    );
}

#[test]
fn a_mutating_plugin_is_still_refused_through_the_palettes_door() {
    let mut app = state_with(MUTATING, true);

    submit_the_plugin_from_the_palette(&mut app);

    assert!(
        app.root.resources.plugin_confirmation().is_none(),
        "the flag's promise is that this cannot run from the palette either"
    );
    assert!(
        app.root
            .system
            .status_message
            .as_deref()
            .is_some_and(|message| message.contains("read-only")),
        "and the refusal must say why: {:?}",
        app.root.system.status_message
    );
}

/// The refusal has to survive the status line, not just be produced.
///
/// `render_status` fits the whole line into the pane's width and `fit_text`
/// truncates from the RIGHT, so a sentence one word too long spends its width
/// on the label and drops "cannot prove" — the one clause that separates "nuvem
/// could not vouch for this" from "this mutates", which is the entire reason
/// this sentence was split off the mutation one. 80 columns is the width to pin
/// it at: it is the terminal default, and the label is the branch's own
/// fixture label rather than a short one chosen to make the assertion pass.
///
/// Asserting the operative clause alone would pass on a line whose tail was cut
/// after it, so the whole sentence is asserted and the line is checked for the
/// ellipsis `fit_text` appends.
#[test]
fn a_plugin_refusal_fits_an_eighty_column_status_line() {
    let mut app = root_with_plugin(
        ResourceServiceKind::ComputeInstances,
        KEY,
        &["compute", "instances", "delete", "{name}"],
        "web-01",
    );
    app.root.read_only = true;

    press_the_plugin_key(&mut app);

    let refusal = app
        .root
        .system
        .status_message
        .clone()
        .expect("the mode refuses a plugin it cannot prove reads");
    assert_eq!(
        refusal,
        "Describe web-01 is unavailable: read-only mode cannot prove this only reads"
    );

    let session = RuntimeSession::new_for_test(app);
    let mut terminal = Terminal::new(TestBackend::new(80, 30)).expect("terminal");
    terminal
        .draw(|frame| nuvem::ui::render(frame, &session))
        .expect("render");
    let buffer = terminal.backend().buffer().clone();
    let width = buffer.area.width as usize;
    let lines: Vec<String> = buffer
        .content()
        .chunks(width)
        .map(|line| line.iter().map(|cell| cell.symbol()).collect::<String>())
        .collect();

    let status = lines
        .iter()
        .find(|line| line.contains("is unavailable:"))
        .unwrap_or_else(|| panic!("the refusal is not on the 80-column frame: {lines:?}"));
    assert!(
        status.contains(&refusal),
        "the refusal is truncated at 80 columns: {status:?}"
    );
    assert!(
        !status.trim_end().ends_with("..."),
        "the status line was cut, so some later reword will lose the clause \
         that matters: {status:?}"
    );
}

/// The fail-closed half of the predicate, which no door can reach: the set is
/// loaded once at startup, so an id past its end is unreachable through the UI
/// and only a direct call can pin the answer. Without this, `is_none_or`
/// weakening to `is_some_and` — "an id that does not resolve is fine to run" —
/// changes no test.
#[test]
fn an_id_that_resolves_to_no_plugin_is_refused_under_read_only() {
    let app = state_with(READING, true);
    let past_the_end = PluginId(app.root.plugins.all().len());

    assert!(
        nuvem::app::state::plugins::plugin_blocked_by_read_only(&app.root, past_the_end),
        "an id the plugin set cannot resolve must refuse, not run"
    );
}

/// `dangerous` and `Reads` in one plugin, a combination nothing could reach
/// before this slice: read-only refused every plugin, so a dangerous one never
/// got as far as a confirmation under the flag. The two guards answer different
/// questions — the mode asks what the declaration proves, the typed
/// acknowledgement asks whether the operator read the line — so admitting a
/// plugin must not also excuse it from typing.
#[test]
fn a_dangerous_reading_plugin_still_demands_its_typed_acknowledgement() {
    let mut app = state_with_dangerous_reading_plugin();

    press_the_plugin_key(&mut app);

    let confirmation = app
        .root
        .resources
        .plugin_confirmation()
        .expect("a dangerous plugin nuvem can prove reads still reaches its confirmation");
    assert!(
        !confirmation.is_ack_satisfied(),
        "the mode admitting it does not spend the acknowledgement: {:?}",
        confirmation.ack()
    );
}

fn state_with_dangerous_reading_plugin() -> AppState {
    let mut app = state_with(READING, true);
    app.root.plugins = load(
        "version = 1\n\n\
         [[plugins]]\n\
         id = \"probe\"\n\
         scope = \"compute\"\n\
         key = \"r\"\n\
         label = \"Probe {name}\"\n\
         dangerous = true\n\
         args = [\"compute\", \"instances\", \"describe\", \"{name}\"]\n",
    );
    app
}
