//! Evidence — `number` and `integer` are collected at the
//! prompt.
//!
//! The prompt was built for `string` and `dropdown` first; a plugin naming a
//! numeric input was withheld whole. What this file pins is the one thing that
//! admission adds — a parse — and the two properties that parse must have: it
//! is the loader's rule and not `f64::from_str`'s, and it refuses without
//! taking anything the operator typed.

use nuvem::app::reducer::root::reduce;
use nuvem::app::state::resources::Draft;
use nuvem::app::{Action, AppState, ResourcesAction};
use nuvem::config::plugins::{PluginInputKind, Plugins, TextRule};
use nuvem::config::problems::ConfigProblems;
use nuvem_domain::gcp::resource::ResourceServiceKind;
use std::io::Write;
use tempfile::NamedTempFile;

use crate::plugin_prefix_evidence::{load, root_with_plugin};

const ROW: &str = "vm-1";
const KEY: char = 'r';

/// A Compute list with `ROW` selected and one plugin declared by `body`.
///
/// Built through the real parser for the reason the prompt fixtures are: an
/// input's kind and default are the config layer's decisions, and a hand-built
/// `Plugin` would let this file prompt for a declaration the operator could
/// never write — which is exactly the drift these tests exist to catch.
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

/// One input of `declared` type, named by `args_tail`, with no default — so the
/// prompt opens unsatisfied and everything typed is the whole value.
fn state_with_input_of_type(declared: &str, args_tail: &str) -> AppState {
    state_with(&format!(
        "version = 1\n\n\
         [[plugins]]\n\
         id = \"resize\"\n\
         scope = \"compute\"\n\
         key = \"r\"\n\
         label = \"Resize {{name}}\"\n\
         args = [\"compute\", \"instances\", \"resize\", \"{{name}}\", {args_tail}]\n\
         \n\
         [[plugins.inputs]]\n\
         name = \"size\"\n\
         type = \"{declared}\"\n"
    ))
}

fn press_the_plugin_key(app: &mut AppState) {
    reduce(app, &Action::Resources(ResourcesAction::ArmPluginPrefix));
    reduce(
        app,
        &Action::Resources(ResourcesAction::PluginPrefixPress(KEY)),
    );
}

fn type_into_the_prompt(app: &mut AppState, value: &str) {
    for typed in value.chars() {
        reduce(
            app,
            &Action::Resources(ResourcesAction::PluginPromptAppend(typed)),
        );
    }
}

/// What the loader says about the same value written as a declared default, or
/// `None` if it accepts it. The other half of every parity assertion below.
fn loader_verdict_on_default(declared: &str, default: &str) -> Option<String> {
    let mut file = NamedTempFile::new().expect("a temp plugin file");
    file.write_all(
        format!(
            "version = 1\n\n\
             [[plugins]]\n\
             id = \"resize\"\n\
             scope = \"compute\"\n\
             key = \"r\"\n\
             label = \"Resize {{name}}\"\n\
             args = [\"compute\", \"instances\", \"resize\", \"{{name}}\", \"{{size}}\"]\n\
             \n\
             [[plugins.inputs]]\n\
             name = \"size\"\n\
             type = \"{declared}\"\n\
             default = {default}\n"
        )
        .as_bytes(),
    )
    .expect("the fixture writes");

    let mut problems = ConfigProblems::default();
    let plugins = Plugins::load(file.path(), &mut problems);
    if plugins.len() == 1 {
        assert!(problems.is_empty(), "an accepted declaration warns nothing");
        return None;
    }
    Some(problems.messages().join("; "))
}

/// The whole point of the slice: a numeric input is asked for and its value
/// reaches the argv the operator approves.
#[test]
fn a_number_and_an_integer_are_each_collected_and_reach_the_argv() {
    for (declared, typed) in [("number", "0.5"), ("integer", "200")] {
        let mut app = state_with_input_of_type(declared, "\"--size\", \"{size}\"");

        press_the_plugin_key(&mut app);
        assert!(
            app.root.resources.plugin_prompt().is_some(),
            "a {declared} input must open the prompt rather than withhold the plugin"
        );

        type_into_the_prompt(&mut app, typed);
        let effects = reduce(
            &mut app,
            &Action::Resources(ResourcesAction::SubmitPluginPrompt),
        );

        assert!(effects.is_empty(), "submitting a prompt runs nothing");
        let confirmation = app
            .root
            .resources
            .plugin_confirmation()
            .expect("the confirmation opens");
        assert!(
            confirmation.argv().contains(&typed.to_string()),
            "the argv the operator confirms carries the collected {declared}: {:?}",
            confirmation.argv()
        );
    }
}

/// The prompt's parse is the loader's, not Rust's. `"inf".parse::<f64>()` and
/// `"NaN".parse::<f64>()` both succeed, and the text reaches gcloud verbatim,
/// so accepting either here would let the prompt send exactly what a
/// declaration of the same value is refused for — with no config review in
/// front of it.
///
/// Both halves are asserted together rather than the prompt's alone: a test
/// naming only the prompt would stay green if the loader's rule were the one
/// that moved.
#[test]
fn the_prompt_refuses_the_numeric_values_a_declared_default_is_refused_for() {
    for (declared, typed, written) in [
        ("number", "inf", "inf"),
        ("number", "-inf", "-inf"),
        ("number", "nan", "nan"),
        ("integer", "1.5", "1.5"),
    ] {
        let refusal = loader_verdict_on_default(declared, written).unwrap_or_else(|| {
            panic!("the loader must refuse {written} declared as a {declared} default")
        });
        assert!(
            refusal.contains("must be a finite number")
                || refusal.contains("must be a whole number"),
            "the loader's refusal is the rule being mirrored: {refusal}"
        );

        let mut app = state_with_input_of_type(declared, "\"--size\", \"{size}\"");
        press_the_plugin_key(&mut app);
        type_into_the_prompt(&mut app, typed);
        reduce(
            &mut app,
            &Action::Resources(ResourcesAction::SubmitPluginPrompt),
        );

        assert!(
            app.root.resources.plugin_confirmation().is_none(),
            "{typed:?} must not reach a confirmation as a {declared}"
        );
        let prompt = app
            .root
            .resources
            .plugin_prompt()
            .expect("the prompt stays open on a refusal");
        assert_eq!(
            prompt.values()[0].1,
            typed,
            "the operator's text is what they correct, so it must survive the refusal"
        );
    }
}

/// A refusal is a gate, not a discard. The prompt is taken only once nothing
/// can fail, so the fields the operator got right are still there to submit.
#[test]
fn correcting_one_field_keeps_what_was_typed_into_the_others() {
    let mut app = state_with(
        "version = 1\n\n\
         [[plugins]]\n\
         id = \"resize\"\n\
         scope = \"compute\"\n\
         key = \"r\"\n\
         label = \"Resize {name}\"\n\
         args = [\"compute\", \"instances\", \"resize\", \"{name}\", \"--reason\", \"{reason}\", \
         \"--size\", \"{size}\"]\n\
         \n\
         [[plugins.inputs]]\n\
         name = \"reason\"\n\
         type = \"string\"\n\
         \n\
         [[plugins.inputs]]\n\
         name = \"size\"\n\
         type = \"integer\"\n",
    );

    press_the_plugin_key(&mut app);
    type_into_the_prompt(&mut app, "capacity");
    reduce(
        &mut app,
        &Action::Resources(ResourcesAction::PluginPromptNextField),
    );
    type_into_the_prompt(&mut app, "1.5");
    reduce(
        &mut app,
        &Action::Resources(ResourcesAction::SubmitPluginPrompt),
    );
    assert!(
        app.root.resources.plugin_confirmation().is_none(),
        "an unparseable integer must not compose an argv"
    );

    reduce(
        &mut app,
        &Action::Resources(ResourcesAction::PluginPromptBackspace),
    );
    reduce(
        &mut app,
        &Action::Resources(ResourcesAction::PluginPromptBackspace),
    );
    reduce(
        &mut app,
        &Action::Resources(ResourcesAction::SubmitPluginPrompt),
    );

    let confirmation = app
        .root
        .resources
        .plugin_confirmation()
        .expect("the corrected prompt submits");
    assert!(
        confirmation.argv().contains(&"capacity".to_string()),
        "the field the operator got right must not have been dropped: {:?}",
        confirmation.argv()
    );
    assert!(confirmation.argv().contains(&"1".to_string()));
}

/// An `integer` accepts `-1`, and gcloud reads a leading `-` in a positional as
/// a flag. Admitting negatives is what makes this refusal reachable through a
/// typed value for the first time, so it is pinned end to end rather than
/// assumed from the row-field case.
#[test]
fn a_negative_value_in_a_leading_argv_position_is_still_refused() {
    let mut app = state_with_input_of_type("integer", "\"{size}\"");

    press_the_plugin_key(&mut app);
    type_into_the_prompt(&mut app, "-1");
    let effects = reduce(
        &mut app,
        &Action::Resources(ResourcesAction::SubmitPluginPrompt),
    );

    assert!(effects.is_empty(), "a refused submit runs nothing");
    assert!(
        app.root.resources.plugin_confirmation().is_none(),
        "gcloud would read the element as a flag, silently changing the command"
    );
    assert!(
        app.root
            .system
            .status_message
            .as_deref()
            .is_some_and(|message| message.contains("read as a flag")),
        "and the refusal must say why: {:?}",
        app.root.system.status_message
    );
    assert!(
        app.root.resources.plugin_prompt().is_some(),
        "the answer to a value gcloud would misread is to edit it"
    );
}

/// A typed value reaches the argv as typed. A declared default is stored
/// canonically — `0.50` becomes `0.5` — but re-rendering what the operator
/// entered would put characters on the confirmation they did not type, and that
/// screen is the invariant the prompt was built in front of.
#[test]
fn a_typed_numeric_value_is_not_canonicalised_on_its_way_to_the_confirmation() {
    let mut app = state_with_input_of_type("number", "\"--size\", \"{size}\"");

    press_the_plugin_key(&mut app);
    type_into_the_prompt(&mut app, "0.50");
    reduce(
        &mut app,
        &Action::Resources(ResourcesAction::SubmitPluginPrompt),
    );

    let confirmation = app
        .root
        .resources
        .plugin_confirmation()
        .expect("0.50 is a finite number");
    assert!(
        confirmation.argv().contains(&"0.50".to_string()),
        "the confirmation shows the characters that will run: {:?}",
        confirmation.argv()
    );
}

/// Every declarable kind, checked against what the prompt actually built for it.
///
/// The expected side is an exhaustive `match`, so a kind added to
/// `PluginInputKind` fails to compile here until someone decides what the prompt
/// does with it — and the assertion compares that decision against production
/// rather than against itself, so a kind wired to the wrong rule fails too. Both
/// halves are needed: exhaustiveness alone would let `integer` carry `Finite`,
/// and a value check alone would let a new kind slip in unexamined.
#[test]
fn every_declarable_kind_reaches_the_prompt_as_the_draft_its_declaration_implies() {
    for kind in [
        PluginInputKind::String,
        PluginInputKind::Number,
        PluginInputKind::Integer,
        PluginInputKind::Dropdown {
            choices: vec!["10".to_string(), "20".to_string()],
        },
        PluginInputKind::Bool,
    ] {
        let expected = match &kind {
            PluginInputKind::String => Some(Draft::Text {
                text: String::new(),
                rule: TextRule::Free,
            }),
            PluginInputKind::Number => Some(Draft::Text {
                text: String::new(),
                rule: TextRule::Finite,
            }),
            PluginInputKind::Integer => Some(Draft::Text {
                text: String::new(),
                rule: TextRule::Whole,
            }),
            // A dropdown holds an index, so it carries no text rule at all —
            // the thing that makes a numeric rule on one unrepresentable.
            PluginInputKind::Dropdown { choices } => Some(Draft::Choice {
                choices: choices.clone(),
                selected: 0,
            }),
            // Two states and no text, so no rule.
            PluginInputKind::Bool => Some(Draft::Toggle { on: false }),
        };
        let declared = match &kind {
            PluginInputKind::String => "type = \"string\"",
            PluginInputKind::Number => "type = \"number\"",
            PluginInputKind::Integer => "type = \"integer\"",
            PluginInputKind::Dropdown { .. } => "type = \"dropdown\"\nchoices = [\"10\", \"20\"]",
            PluginInputKind::Bool => "type = \"bool\"",
        };

        let mut app = state_with(&format!(
            "version = 1\n\n\
             [[plugins]]\n\
             id = \"resize\"\n\
             scope = \"compute\"\n\
             key = \"r\"\n\
             label = \"Resize {{name}}\"\n\
             args = [\"compute\", \"instances\", \"resize\", \"{{name}}\", \"--size\", \"{{size}}\"]\n\
             \n\
             [[plugins.inputs]]\n\
             name = \"size\"\n\
             {declared}\n"
        ));
        press_the_plugin_key(&mut app);

        let built = app
            .root
            .resources
            .plugin_prompt()
            .map(|prompt| prompt.fields()[0].draft.clone());
        assert_eq!(
            built, expected,
            "the prompt built the wrong draft for {kind:?}"
        );
    }
}

// `a_bool_input_is_still_withheld_rather_than_prompted_for` stood here until
// `bool` was admitted, pinning that it was refused whole rather than prompted for.
// It did the job it was written for: admitting the kind had to come through
// this test rather than past it, and it failed the moment `is_promptable`
// went away. What replaced the behaviour it guarded is pinned in
// `plugin_bool_prompt_evidence`, which is where a bool's prompt now
// lives; re-asserting the old refusal here would only assert that a removed
// code path is still removed.
