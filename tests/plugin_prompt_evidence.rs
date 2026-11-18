//! Evidence — the values a plugin declares are collected
//! before its confirmation exists.
//!
//! Slice 7 parsed `[[plugins.inputs]]` and resolved a plugin naming
//! one to `Command::NeedsValues`; nothing collected them, so the press said so
//! and stopped. What this file pins is the shape that replaces that stop: one
//! entry both doors go through, a prompt in front of the confirmation, and a
//! submit that composes an argv without running anything.

use nuvem::app::reducer::root::reduce;
use nuvem::app::state::command::CommandDescriptor;
use nuvem::app::state::configurations::ConfigurationsState;
use nuvem::app::{Action, AppState, CommandAction, ResourcesAction};
use nuvem_domain::gcp::resource::ResourceServiceKind;

use crate::plugin_prefix_evidence::{load, root_with_plugin};

/// The row every fixture here selects. Named for the label assertion: the
/// heading the operator reads has to name the row *and* the typed value, so
/// the two have to be distinguishable in it.
const ROW: &str = "vm-1";

/// The plugin key the tests press.
const KEY: char = 'r';

/// A Compute list with `ROW` selected and one plugin bound to `KEY`, declared
/// by `body`.
///
/// Built through the real parser (`load`) rather than by constructing a
/// `Plugin`: an input's default, kind and admissibility are all the config
/// layer's decisions, and a hand-built plugin would let this file prompt for a
/// declaration the operator could never write.
fn state_with(body: &str) -> AppState {
    let mut app = root_with_plugin(
        ResourceServiceKind::ComputeInstances,
        KEY,
        &["compute", "instances", "describe", "{name}"],
        ROW,
    );
    app.root.plugins = load(body);
    app
}

/// One string input, no default — so the prompt opens unsatisfied and every
/// character the test types is the whole value.
fn state_with_plugin_declaring_a_string_input() -> AppState {
    state_with(
        "version = 1\n\n\
         [[plugins]]\n\
         id = \"resize\"\n\
         scope = \"compute\"\n\
         key = \"r\"\n\
         label = \"Resize {name}\"\n\
         args = [\"compute\", \"instances\", \"resize\", \"{name}\", \"--size\", \"{disk_size}\"]\n\
         \n\
         [[plugins.inputs]]\n\
         name = \"disk_size\"\n\
         type = \"string\"\n",
    )
}

/// The same plugin with the input named in its `label` too, which is what
/// gives the confirmation heading something only the submit path can render.
fn state_with_plugin_whose_label_names_an_input() -> AppState {
    state_with(
        "version = 1\n\n\
         [[plugins]]\n\
         id = \"resize\"\n\
         scope = \"compute\"\n\
         key = \"r\"\n\
         label = \"Resize {name} to {disk_size}GB\"\n\
         args = [\"compute\", \"instances\", \"resize\", \"{name}\", \"--size\", \"{disk_size}\"]\n\
         \n\
         [[plugins.inputs]]\n\
         name = \"disk_size\"\n\
         type = \"string\"\n",
    )
}

/// A plugin naming no input at all: the shape every plugin written before
/// `[[plugins.inputs]]` existed has.
fn state_with_plugin_declaring_no_inputs() -> AppState {
    state_with(
        "version = 1\n\n\
         [[plugins]]\n\
         id = \"describe\"\n\
         scope = \"compute\"\n\
         key = \"r\"\n\
         label = \"Describe {name}\"\n\
         args = [\"compute\", \"instances\", \"describe\", \"{name}\"]\n",
    )
}

/// The keyed door: `,` then the plugin's key.
fn press_the_plugin_key(app: &mut AppState) {
    reduce(app, &Action::Resources(ResourcesAction::ArmPluginPrefix));
    reduce(
        app,
        &Action::Resources(ResourcesAction::PluginPrefixPress(KEY)),
    );
}

/// The other door: the palette, submitted on the plugin's own row.
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

/// The refusal has to come before the first keystroke, and the offer it refuses
/// has to be one that *needs values* — a `Ready` offer would be refused by the
/// confirmation opener even if the check had drifted below the match, so it
/// cannot tell the two placements apart. This is the only check on the keyed
/// door's path.
#[test]
fn a_read_only_session_is_refused_at_prompt_open() {
    let mut app = state_with_plugin_declaring_a_string_input();
    app.root.read_only = true;

    press_the_plugin_key(&mut app);

    assert!(
        app.root.resources.plugin_prompt().is_none(),
        "read-only must refuse before the operator types a single value"
    );
    assert!(
        app.root
            .system
            .status_message
            .as_deref()
            .is_some_and(|message| message.contains("read-only")),
        "and must say why: {:?}",
        app.root.system.status_message
    );
}

/// The palette submits the same offer through the same entry, so neither door
/// can come to disagree about read-only.
#[test]
fn a_read_only_session_is_refused_at_the_palette_door_too() {
    let mut app = state_with_plugin_declaring_a_string_input();
    app.root.read_only = true;

    submit_the_plugin_from_the_palette(&mut app);

    assert!(
        app.root.resources.plugin_prompt().is_none(),
        "read-only must refuse the palette's door before any value is collected"
    );
    assert!(
        app.root.resources.plugin_confirmation().is_none(),
        "and must leave nothing to confirm"
    );
}

#[test]
fn a_plugin_with_inputs_opens_the_prompt_not_the_confirmation() {
    let mut app = state_with_plugin_declaring_a_string_input();

    press_the_plugin_key(&mut app);

    assert!(app.root.resources.plugin_prompt().is_some());
    assert!(
        app.root.resources.plugin_confirmation().is_none(),
        "the confirmation must not be reachable before values are collected"
    );
}

/// The palette's door reaches the prompt as well: a plugin needing values was
/// offered there before anything could collect them.
#[test]
fn the_palette_door_opens_the_prompt_too() {
    let mut app = state_with_plugin_declaring_a_string_input();

    submit_the_plugin_from_the_palette(&mut app);

    assert!(
        app.root.resources.plugin_prompt().is_some(),
        "both doors must route through the one entry"
    );
    assert!(app.root.resources.plugin_confirmation().is_none());
}

#[test]
fn a_plugin_without_inputs_still_opens_the_confirmation_directly() {
    let mut app = state_with_plugin_declaring_no_inputs();

    press_the_plugin_key(&mut app);

    assert!(
        app.root.resources.plugin_prompt().is_none(),
        "a plugin asking for nothing must not open a prompt with no fields"
    );
    assert!(app.root.resources.plugin_confirmation().is_some());
}

#[test]
fn submitting_the_prompt_opens_the_confirmation_with_the_value_substituted() {
    let mut app = state_with_plugin_declaring_a_string_input();
    press_the_plugin_key(&mut app);

    reduce(
        &mut app,
        &Action::Resources(ResourcesAction::PluginPromptAppend('2')),
    );
    reduce(
        &mut app,
        &Action::Resources(ResourcesAction::PluginPromptAppend('0')),
    );
    let effects = reduce(
        &mut app,
        &Action::Resources(ResourcesAction::SubmitPluginPrompt),
    );

    assert!(
        effects.is_empty(),
        "submitting a prompt must not run anything; only ConfirmPlugin does: {effects:?}"
    );
    let confirmation = app
        .root
        .resources
        .plugin_confirmation()
        .expect("the confirmation opens");
    assert!(
        confirmation.argv().contains(&"20".to_string()),
        "the argv the operator confirms carries the collected value: {:?}",
        confirmation.argv()
    );
    assert!(
        app.root.resources.plugin_prompt().is_none(),
        "the prompt closes"
    );
}

/// The keymap gates `Enter` on `is_satisfied`, and the reducer asks again: the
/// keymap's answer is about a keypress, this one about the state, so anything
/// else that ever dispatches the action is gated too.
#[test]
fn submitting_an_unsatisfied_prompt_opens_no_confirmation() {
    let mut app = state_with_plugin_declaring_a_string_input();
    press_the_plugin_key(&mut app);

    let effects = reduce(
        &mut app,
        &Action::Resources(ResourcesAction::SubmitPluginPrompt),
    );

    assert!(effects.is_empty());
    assert!(
        app.root.resources.plugin_confirmation().is_none(),
        "the keymap gates this, and the reducer must not undo that gate"
    );
    assert!(
        app.root.resources.plugin_prompt().is_some(),
        "the prompt stays open"
    );
}

#[test]
fn cancelling_the_prompt_runs_nothing_and_logs_nothing() {
    let mut app = state_with_plugin_declaring_a_string_input();
    let before = app.root.system.operations.records().len();
    press_the_plugin_key(&mut app);

    let effects = reduce(
        &mut app,
        &Action::Resources(ResourcesAction::CancelPluginPrompt),
    );

    assert!(effects.is_empty());
    assert!(app.root.resources.plugin_prompt().is_none());
    assert!(app.root.resources.plugin_confirmation().is_none());
    assert_eq!(
        app.root.system.operations.records().len(),
        before,
        "Esc at the prompt leaves no operations-log entry"
    );
}

/// A submit that cannot compose an argv keeps the operator's typing.
///
/// `{disk_size}` is a whole argv element, so a value starting with `-` would
/// reach gcloud as a flag — `Unoffered::LeadingDash`, the same rule that
/// withholds a row value shaped that way. The answer to such a value is to
/// edit it, which is only possible if the modal is still there holding it: a
/// submit that took the prompt before composing would have closed it and lost
/// the value with nothing to retype it from.
#[test]
fn a_value_that_cannot_compose_an_argv_keeps_the_prompt_and_its_value() {
    let mut app = state_with_plugin_declaring_a_string_input();
    press_the_plugin_key(&mut app);

    for typed in ['-', 'x'] {
        reduce(
            &mut app,
            &Action::Resources(ResourcesAction::PluginPromptAppend(typed)),
        );
    }
    let effects = reduce(
        &mut app,
        &Action::Resources(ResourcesAction::SubmitPluginPrompt),
    );

    assert!(effects.is_empty(), "a refused submit runs nothing");
    assert!(
        app.root.resources.plugin_confirmation().is_none(),
        "the confirmation may only ever hold an argv that passed every rule"
    );
    let prompt = app
        .root
        .resources
        .plugin_prompt()
        .expect("the prompt stays open so the value can be edited");
    assert_eq!(
        prompt.values(),
        vec![("disk_size".to_string(), "-x".to_string())],
        "and it stays open on what the operator typed, not on an empty field"
    );
    assert_eq!(
        app.root.system.status_message.as_deref(),
        Some(
            "Plugin \"resize\" args[5] renders a value beginning with \"-\" for this \
             GCE instance, which gcloud would read as a flag"
        ),
        "and says why, in the wording the palette and the keyed press already use"
    );
}

/// A plugin whose templates mix a row field and an input, on a row that has no
/// value for the field.
///
/// `external_ip` is `None` on the fixture row, so this plugin is offered — the
/// argv is not composed until submit — and refused only once the operator has
/// typed. The sentence they get then decides whether they can act on it.
fn state_with_plugin_mixing_a_missing_field_and_an_input() -> AppState {
    state_with(
        "version = 1\n\n\
         [[plugins]]\n\
         id = \"probe\"\n\
         scope = \"compute\"\n\
         key = \"r\"\n\
         label = \"Probe {name}\"\n\
         args = [\"compute\", \"probe\", \"{external_ip}\", \"--size={disk_size}\"]\n\
         \n\
         [[plugins.inputs]]\n\
         name = \"disk_size\"\n\
         type = \"string\"\n",
    )
}

/// The refusal names the row field nothing typed can supply, not the values.
///
/// Blaming the values would put the operator in a loop: they would edit a
/// perfectly good `disk_size` forever, because the missing `external_ip` is not
/// something any value they type can answer. Naming the field tells them the
/// answer is another row.
#[test]
fn a_submit_refused_for_a_missing_row_field_names_the_field_not_the_values() {
    let mut app = state_with_plugin_mixing_a_missing_field_and_an_input();
    press_the_plugin_key(&mut app);
    assert!(
        app.root.resources.plugin_prompt().is_some(),
        "the plugin is offered: its argv is not composed until submit"
    );

    reduce(
        &mut app,
        &Action::Resources(ResourcesAction::PluginPromptAppend('9')),
    );
    let effects = reduce(
        &mut app,
        &Action::Resources(ResourcesAction::SubmitPluginPrompt),
    );

    assert!(effects.is_empty(), "a refused submit runs nothing");
    assert!(app.root.resources.plugin_confirmation().is_none());
    assert_eq!(
        app.root.system.status_message.as_deref(),
        Some("Plugin \"probe\" needs external_ip, which this GCE instance has no value for"),
        "the operator must be told about the field, not about what they typed"
    );
    assert!(
        app.root.resources.plugin_prompt().is_some(),
        "the prompt stays open, holding what was typed"
    );
}

/// The other fallible step on the submit path, held to the same rule.
///
/// Unreachable in practice — no context means no rows, and no row means no
/// offer to have opened this prompt — which is exactly why it must not fail
/// silently if it ever becomes reachable, the argument the `ConfirmPlugin` arm
/// already makes for its own no-context case.
#[test]
fn a_submit_with_no_active_context_keeps_the_prompt_and_says_so() {
    let mut app = state_with_plugin_declaring_a_string_input();
    press_the_plugin_key(&mut app);
    reduce(
        &mut app,
        &Action::Resources(ResourcesAction::PluginPromptAppend('9')),
    );
    app.root.configurations = ConfigurationsState::default();

    reduce(
        &mut app,
        &Action::Resources(ResourcesAction::SubmitPluginPrompt),
    );

    assert!(app.root.resources.plugin_confirmation().is_none());
    assert!(
        app.root.resources.plugin_prompt().is_some(),
        "an Enter that composed nothing must not swallow the values too"
    );
    assert!(
        app.root
            .system
            .status_message
            .as_deref()
            .is_some_and(|message| message.contains("no active gcloud configuration")),
        "and must say why: {:?}",
        app.root.system.status_message
    );
}

/// The heading is re-rendered on submit, which is the only moment a label
/// naming an input can render at all: the palette showed the plugin's id
/// because no value existed then.
#[test]
fn the_confirmation_heading_shows_the_label_with_the_value_substituted() {
    let mut app = state_with_plugin_whose_label_names_an_input();
    press_the_plugin_key(&mut app);

    reduce(
        &mut app,
        &Action::Resources(ResourcesAction::PluginPromptAppend('9')),
    );
    reduce(
        &mut app,
        &Action::Resources(ResourcesAction::SubmitPluginPrompt),
    );

    assert_eq!(
        app.root
            .resources
            .plugin_confirmation()
            .expect("opens")
            .label(),
        "Resize vm-1 to 9GB"
    );
}

/// A `dangerous` plugin with inputs: the flag is read off the prompt on submit,
/// and it is the prompt that carries it there — the offer that opened the
/// prompt is gone by then.
fn state_with_dangerous_plugin_declaring_a_string_input() -> AppState {
    state_with(
        "version = 1\n\n\
         [[plugins]]\n\
         id = \"resize\"\n\
         scope = \"compute\"\n\
         key = \"r\"\n\
         label = \"Resize {name}\"\n\
         dangerous = true\n\
         args = [\"compute\", \"instances\", \"resize\", \"{name}\", \"--size\", \"{disk_size}\"]\n\
         \n\
         [[plugins.inputs]]\n\
         name = \"disk_size\"\n\
         type = \"string\"\n",
    )
}

/// Three fields, because two cannot tell stepping back from stepping forward
/// apart: on a two-field prompt both land on the other field.
fn state_with_plugin_declaring_three_string_inputs() -> AppState {
    state_with(
        "version = 1\n\n\
         [[plugins]]\n\
         id = \"resize\"\n\
         scope = \"compute\"\n\
         key = \"r\"\n\
         label = \"Resize {name}\"\n\
         args = [\"compute\", \"instances\", \"resize\", \"{name}\", \
         \"--size\", \"{disk_size}\", \"--tier\", \"{tier}\", \"--note\", \"{note}\"]\n\
         \n\
         [[plugins.inputs]]\n\
         name = \"disk_size\"\n\
         type = \"string\"\n\
         \n\
         [[plugins.inputs]]\n\
         name = \"tier\"\n\
         type = \"string\"\n\
         \n\
         [[plugins.inputs]]\n\
         name = \"note\"\n\
         type = \"string\"\n",
    )
}

/// Three choices, for the reason the three-field fixture exists: with two, a
/// backwards step and a forwards step select the same one.
fn state_with_plugin_declaring_a_dropdown_of_three_choices() -> AppState {
    state_with(
        "version = 1\n\n\
         [[plugins]]\n\
         id = \"resize\"\n\
         scope = \"compute\"\n\
         key = \"r\"\n\
         label = \"Resize {name}\"\n\
         args = [\"compute\", \"instances\", \"resize\", \"{name}\", \"--tier\", \"{tier}\"]\n\
         \n\
         [[plugins.inputs]]\n\
         name = \"tier\"\n\
         type = \"dropdown\"\n\
         choices = [\"small\", \"medium\", \"large\"]\n",
    )
}

/// The typed acknowledgement a `dangerous` plugin demands has to reach the
/// confirmation the prompt composes. Both directions are asserted because the
/// flag is read, not assumed: a submit that always demanded the `1` would train
/// the operator to type it, and one that never did would run a destructive
/// command on a bare `Enter`.
#[test]
fn the_prompt_carries_the_dangerous_flag_into_the_confirmation_it_composes() {
    let mut dangerous = state_with_dangerous_plugin_declaring_a_string_input();
    press_the_plugin_key(&mut dangerous);
    reduce(
        &mut dangerous,
        &Action::Resources(ResourcesAction::PluginPromptAppend('9')),
    );
    reduce(
        &mut dangerous,
        &Action::Resources(ResourcesAction::SubmitPluginPrompt),
    );
    assert!(
        !dangerous
            .root
            .resources
            .plugin_confirmation()
            .expect("the confirmation opens")
            .is_ack_satisfied(),
        "a dangerous plugin must still demand the typed acknowledgement"
    );

    let mut ordinary = state_with_plugin_declaring_a_string_input();
    press_the_plugin_key(&mut ordinary);
    reduce(
        &mut ordinary,
        &Action::Resources(ResourcesAction::PluginPromptAppend('9')),
    );
    reduce(
        &mut ordinary,
        &Action::Resources(ResourcesAction::SubmitPluginPrompt),
    );
    assert!(
        ordinary
            .root
            .resources
            .plugin_confirmation()
            .expect("the confirmation opens")
            .is_ack_satisfied(),
        "an ordinary plugin must be confirmable by the keypress alone"
    );
}

/// `Shift-Tab` from the first field reaches the *last*, not the second.
#[test]
fn stepping_back_from_the_first_field_wraps_to_the_last() {
    let mut app = state_with_plugin_declaring_three_string_inputs();
    press_the_plugin_key(&mut app);
    let focus = |app: &AppState| {
        app.root
            .resources
            .plugin_prompt()
            .expect("the prompt is open")
            .focus()
    };
    assert_eq!(focus(&app), 0, "the prompt opens on its first field");

    reduce(
        &mut app,
        &Action::Resources(ResourcesAction::PluginPromptPreviousField),
    );
    assert_eq!(
        focus(&app),
        2,
        "stepping back from the first field wraps to the last"
    );

    reduce(
        &mut app,
        &Action::Resources(ResourcesAction::PluginPromptNextField),
    );
    assert_eq!(focus(&app), 0, "and stepping forward from the last wraps");
}

/// `Left` on a dropdown selects the *previous* choice, wrapping to the last.
#[test]
fn cycling_a_dropdown_backwards_reaches_the_last_choice() {
    let mut app = state_with_plugin_declaring_a_dropdown_of_three_choices();
    press_the_plugin_key(&mut app);
    let value = |app: &AppState| {
        app.root
            .resources
            .plugin_prompt()
            .expect("the prompt is open")
            .values()
            .into_iter()
            .map(|(_, value)| value)
            .next()
            .expect("the dropdown is the prompt's one field")
    };
    assert_eq!(value(&app), "small", "a dropdown opens on its first choice");

    reduce(
        &mut app,
        &Action::Resources(ResourcesAction::PluginPromptCycleChoice(false)),
    );
    assert_eq!(
        value(&app),
        "large",
        "cycling backwards from the first choice wraps to the last"
    );

    reduce(
        &mut app,
        &Action::Resources(ResourcesAction::PluginPromptCycleChoice(true)),
    );
    assert_eq!(value(&app), "small", "and cycling forwards wraps back");
}

/// The submit *takes* the prompt rather than reading it and leaving it: a
/// second submit must find nothing rather than compose the same argv twice.
/// Asserted on the state directly, because the confirmation the submit opens
/// clears every other modal on its way in — which would make a borrow-and-leave
/// look identical from the outside until something opened no confirmation.
#[test]
fn taking_the_prompt_yields_it_once() {
    let mut app = state_with_plugin_declaring_a_string_input();
    press_the_plugin_key(&mut app);

    let taken = app
        .root
        .resources
        .take_plugin_prompt()
        .expect("the prompt is open");

    assert_eq!(taken.label(), "Resize vm-1");
    assert!(
        app.root.resources.take_plugin_prompt().is_none(),
        "the prompt is gone once it has been taken"
    );
}
