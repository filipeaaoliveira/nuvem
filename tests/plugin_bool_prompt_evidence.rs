//! Evidence — `bool` is collected at the prompt, and is
//! the last kind that was declared, validated and then reached nothing.
//!
//! An earlier slice admitted `number` and `integer`; `bool` stayed out on an argument
//! about meaning rather than about parsing — a bool is a flag's *presence*, so
//! admitting one looked like a change to what an argv element may be. This
//! slice settles that the other way: a bool contributes the **text** `true` or
//! `false`, exactly like every other kind, which is the representation the
//! loader already normalises a declared default into
//! (`config::plugins::default_for`). Flag presence is a separate question and
//! is not foreclosed here.
//!
//! What this file pins is therefore not a parse — there is nothing to parse —
//! but the three things admission adds: the plugin is offered at all, the
//! field is two-state and cannot be anything else, and the text it yields is
//! the text the loader would have accepted as a default.

use nuvem::app::reducer::root::reduce;
use nuvem::app::state::resources::Draft;
use nuvem::app::{Action, AppState, ResourcesAction};

use crate::plugin_prefix_evidence::{load, root_with_plugin};
use nuvem_domain::gcp::resource::ResourceServiceKind;

const ROW: &str = "vm-1";
const KEY: char = 'r';

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

/// One `bool` input named by the args, optionally with a declared default.
fn state_with_bool(default: Option<bool>) -> AppState {
    let default_line = match default {
        Some(value) => format!("default = {value}\n"),
        None => String::new(),
    };
    state_with(&format!(
        "version = 1\n\n\
         [[plugins]]\n\
         id = \"resize\"\n\
         scope = \"compute\"\n\
         key = \"r\"\n\
         label = \"Resize {{name}}\"\n\
         args = [\"compute\", \"instances\", \"resize\", \"{{name}}\", \"--async={{async}}\"]\n\
         \n\
         [[plugins.inputs]]\n\
         name = \"async\"\n\
         type = \"bool\"\n\
         {default_line}"
    ))
}

fn press_the_plugin_key(app: &mut AppState) {
    reduce(app, &Action::Resources(ResourcesAction::ArmPluginPrefix));
    reduce(
        app,
        &Action::Resources(ResourcesAction::PluginPrefixPress(KEY)),
    );
}

/// The one this slice exists for: before it, this press withheld the plugin
/// whole and said it "needs input async".
#[test]
fn a_plugin_naming_a_bool_is_offered_and_prompts_for_it() {
    let mut app = state_with_bool(None);

    press_the_plugin_key(&mut app);

    let prompt = app
        .root
        .resources
        .plugin_prompt()
        .expect("a bool no longer withholds its plugin");
    assert_eq!(prompt.fields().len(), 1);
    assert_eq!(prompt.fields()[0].name, "async");
}

/// An unanswered bool starts at the answer that adds nothing, so tabbing past
/// a field cannot opt an operator into something they never said yes to.
#[test]
fn an_undeclared_bool_starts_false() {
    let mut app = state_with_bool(None);

    press_the_plugin_key(&mut app);

    assert_eq!(
        app.root.resources.plugin_prompt().unwrap().fields()[0].draft,
        Draft::Toggle { on: false }
    );
}

#[test]
fn a_declared_default_decides_where_the_toggle_starts() {
    for declared in [true, false] {
        let mut app = state_with_bool(Some(declared));

        press_the_plugin_key(&mut app);

        assert_eq!(
            app.root.resources.plugin_prompt().unwrap().fields()[0].draft,
            Draft::Toggle { on: declared },
            "a declared default of {declared} must be where the field opens"
        );
    }
}

/// Two states and no third. Cycling in either direction from either state
/// lands on the other one — the property that makes a bool a bool rather than
/// a two-item dropdown that could grow a third item.
#[test]
fn cycling_a_toggle_moves_between_exactly_two_states() {
    for forward in [true, false] {
        let mut app = state_with_bool(Some(false));
        press_the_plugin_key(&mut app);

        for expected in [true, false, true] {
            reduce(
                &mut app,
                &Action::Resources(ResourcesAction::PluginPromptCycleChoice(forward)),
            );
            assert_eq!(
                app.root.resources.plugin_prompt().unwrap().fields()[0].draft,
                Draft::Toggle { on: expected },
                "cycling forward={forward} must alternate"
            );
        }
    }
}

/// A bool is always submittable: there is no text to be empty or to misparse,
/// so the prompt is satisfied the moment it opens.
#[test]
fn a_bool_field_is_satisfied_without_being_touched() {
    let mut app = state_with_bool(None);

    press_the_plugin_key(&mut app);

    assert!(
        app.root.resources.plugin_prompt().unwrap().is_satisfied(),
        "a two-state field has nothing to complete"
    );
}

/// The load-bearing one, and the one this slice exists for: the answer has to
/// reach the argv the operator confirms. Everything above pins where the field
/// starts and how it moves; none of it would notice a toggle that submitted
/// the wrong text, or no text at all.
#[test]
fn the_answer_reaches_the_argv_the_operator_confirms() {
    for (cycles, expected) in [(0, "--async=false"), (1, "--async=true")] {
        let mut app = state_with_bool(None);
        press_the_plugin_key(&mut app);

        for _ in 0..cycles {
            reduce(
                &mut app,
                &Action::Resources(ResourcesAction::PluginPromptCycleChoice(true)),
            );
        }
        let effects = reduce(
            &mut app,
            &Action::Resources(ResourcesAction::SubmitPluginPrompt),
        );

        assert!(
            effects.is_empty(),
            "submitting a prompt must not run anything: {effects:?}"
        );
        let argv = app
            .root
            .resources
            .plugin_confirmation()
            .expect("the confirmation opens")
            .argv()
            .to_vec();
        assert!(
            argv.contains(&expected.to_string()),
            "after {cycles} cycle(s) the argv must carry {expected:?}: {argv:?}"
        );
    }
}

/// A declared default reaches the argv too, so the two ways a bool arrives at
/// its value — declared and answered — are both pinned rather than only the
/// one the operator touches.
#[test]
fn a_declared_default_reaches_the_argv_untouched() {
    let mut app = state_with_bool(Some(true));
    press_the_plugin_key(&mut app);

    reduce(
        &mut app,
        &Action::Resources(ResourcesAction::SubmitPluginPrompt),
    );

    let argv = app
        .root
        .resources
        .plugin_confirmation()
        .expect("the confirmation opens")
        .argv()
        .to_vec();
    assert!(
        argv.contains(&"--async=true".to_string()),
        "a declared default must reach the argv: {argv:?}"
    );
}
