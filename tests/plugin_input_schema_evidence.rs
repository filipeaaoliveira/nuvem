//! The typed-input schema of `[[plugins.inputs]]` (slice 7): what a
//! well-formed declaration parses into, and what each malformed one costs.
//!
//! Nothing in this slice prompts and nothing reaches an argv. The evidence for
//! that half is here too, at the bottom: a plugin whose argv names an input is
//! *not* offered, so the palette cannot show a command built from a value the
//! operator was never asked for. Slice 8 is what supplies the value.
//!
//! Granularity is the module's own rule (`src/config/plugins.rs:23`): a bad
//! input rejects its own plugin with a warning naming it, and leaves the
//! siblings loaded. One test per malformed case, because a batched assertion
//! passes as soon as *some* entry warns.

use std::io::Write;

use clap::Parser;
use nuvem::app::state::plugins::{Command, applicable, diagnostics};
use nuvem::config::jumps::JumpField;
use nuvem::config::plugins::{PluginInputKind, Plugins};
use nuvem::config::problems::ConfigProblems;
use nuvem::{Cli, Config, ConfigOutcome};
use nuvem_domain::gcp::configuration::{GcloudConfigurationName, GcloudContext};
use nuvem_domain::gcp::resource::ResourceServiceKind;

/// A config path that cannot exist, so `problems` reports on the plugin file
/// and not on the developer's own `~/.config/nuvem/config.toml`.
const NO_CONFIG: &str = "/nonexistent/nuvem-config-none.toml";

/// Resolve with an explicit `--plugins` path, returning the temp file so it
/// outlives the resolve call.
fn resolve_plugins(body: &str) -> (ConfigOutcome, tempfile::NamedTempFile) {
    let mut file = tempfile::NamedTempFile::new().unwrap();
    file.write_all(body.as_bytes()).unwrap();
    let cli = Cli::parse_from([
        "nuvem",
        "--config",
        NO_CONFIG,
        "--plugins",
        file.path().to_str().unwrap(),
    ]);
    (Config::resolve(&cli), file)
}

/// One Compute-scoped plugin whose `args` name only row fields, with `inputs`
/// appended verbatim. Every malformed case below varies the inputs alone, so
/// nothing can be dropped for a reason the test is not about.
fn one_plugin_with(inputs: &str) -> String {
    format!(
        "version = 1\n\n\
         [[plugins]]\n\
         id    = \"resize\"\n\
         scope = \"compute\"\n\
         key   = \"R\"\n\
         label = \"Resize {{name}}\"\n\
         args  = [\"compute\", \"instances\", \"describe\", \"{{name}}\"]\n\
         {inputs}"
    )
}

/// Assert that the lone plugin was dropped, that the warning explains `needle`,
/// and that it says what it did about the problem.
fn assert_only_plugin_dropped(inputs: &str, needle: &str) {
    let (out, _file) = resolve_plugins(&one_plugin_with(inputs));

    let problems = out.problems.messages();
    let message = problems
        .iter()
        .find(|m| m.starts_with("plugins[0]: "))
        .unwrap_or_else(|| panic!("no warning for plugins[0] in {problems:?}"));
    assert!(
        message.contains(needle),
        "warning {message:?} does not explain {needle:?}"
    );
    assert!(
        message.ends_with("ignoring this plugin"),
        "warning {message:?} does not say what it did about the problem"
    );
    assert!(
        out.config.plugins.is_empty(),
        "a plugin with a malformed input loaded"
    );
}

/// The README's example, with `machine_type` renamed: on `compute-instances`
/// that name *is* a row field, and an input may not shadow one.
const RESIZE: &str = "version = 1\n\n\
     [[plugins]]\n\
     id    = \"resize\"\n\
     scope = \"compute\"\n\
     key   = \"R\"\n\
     label = \"Resize {name}\"\n\
     args  = [\"compute\", \"instances\", \"set-machine-type\", \"{name}\", \"--zone\", \"{zone}\", \"--machine-type\", \"{target_type}\", \"--disk-size\", \"{disk_size}\", \"--share\", \"{cpu_share}\", \"--async\", \"{run_async}\", \"--note\", \"{note}\"]\n\
     \n\
     [[plugins.inputs]]\n\
     name    = \"target_type\"\n\
     type    = \"dropdown\"\n\
     choices = [\"e2-standard-2\", \"e2-standard-4\"]\n\
     default = \"e2-standard-2\"\n\
     \n\
     [[plugins.inputs]]\n\
     name    = \"disk_size\"\n\
     type    = \"integer\"\n\
     default = 100\n\
     \n\
     [[plugins.inputs]]\n\
     name    = \"cpu_share\"\n\
     type    = \"number\"\n\
     default = 0.5\n\
     \n\
     [[plugins.inputs]]\n\
     name    = \"run_async\"\n\
     type    = \"bool\"\n\
     default = true\n\
     \n\
     [[plugins.inputs]]\n\
     name = \"note\"\n\
     type = \"string\"\n";

#[test]
fn a_plugin_declaring_well_formed_inputs_loads_with_them_parsed() {
    let (out, _file) = resolve_plugins(RESIZE);

    assert!(
        out.problems.is_empty(),
        "unexpected: {:?}",
        out.problems.messages()
    );
    assert_eq!(out.config.plugins.len(), 1);

    let plugin = &out.config.plugins.all()[0];
    let names: Vec<&str> = plugin
        .inputs
        .iter()
        .map(|input| input.name.as_str())
        .collect();
    // File order, because the prompt slice asks for them in the order the
    // operator wrote them.
    assert_eq!(
        names,
        vec!["target_type", "disk_size", "cpu_share", "run_async", "note"]
    );

    let kinds: Vec<&PluginInputKind> = plugin.inputs.iter().map(|input| &input.kind).collect();
    assert_eq!(
        kinds,
        vec![
            &PluginInputKind::Dropdown {
                choices: vec!["e2-standard-2".to_string(), "e2-standard-4".to_string()],
            },
            &PluginInputKind::Integer,
            &PluginInputKind::Number,
            &PluginInputKind::Bool,
            &PluginInputKind::String,
        ]
    );

    // Kept as the text it contributes to an argv: the declaration's job ends at
    // "well-formed", the value lands in an argv verbatim, and one
    // representation means there is no second numeric formatting rule between
    // load and the command line. `note` declares none, so `default` is
    // optional.
    let defaults: Vec<Option<&str>> = plugin
        .inputs
        .iter()
        .map(|input| input.default.as_deref())
        .collect();
    assert_eq!(
        defaults,
        vec![
            Some("e2-standard-2"),
            Some("100"),
            Some("0.5"),
            Some("true"),
            None
        ]
    );
}

/// An input's name is admitted to the template vocabulary alongside the row's
/// own fields. Without admission this file is `unknown field "disk_size" for
/// compute-instances` and the whole plugin drops — which is what an operator
/// gets today, and the reason this slice cannot be added incrementally by one.
#[test]
fn an_argv_may_name_an_input_alongside_the_rows_own_fields() {
    let (out, _file) = resolve_plugins(
        "version = 1\n\n\
         [[plugins]]\n\
         id    = \"grow\"\n\
         scope = \"compute\"\n\
         key   = \"G\"\n\
         label = \"Grow {name} to {disk_size}\"\n\
         args  = [\"compute\", \"disks\", \"resize\", \"{name}\", \"--size\", \"{disk_size}\"]\n\
         \n\
         [[plugins.inputs]]\n\
         name = \"disk_size\"\n\
         type = \"integer\"\n",
    );

    assert!(
        out.problems.is_empty(),
        "unexpected: {:?}",
        out.problems.messages()
    );
    assert_eq!(out.config.plugins.len(), 1);
    assert_eq!(out.config.plugins.all()[0].inputs.len(), 1);
}

#[test]
fn an_input_name_that_collides_with_a_row_field_is_refused() {
    // The collision the ticket names: the row's `{zone}` and an input called
    // `zone` cannot both win, and the loser cannot be decided silently — a
    // template would render one of the two and no warning would say which.
    assert_only_plugin_dropped(
        "[[plugins.inputs]]\nname = \"zone\"\ntype = \"string\"\n",
        "zone",
    );
}

#[test]
fn an_unknown_key_inside_an_input_is_refused() {
    assert_only_plugin_dropped(
        "[[plugins.inputs]]\nname = \"size\"\ntype = \"string\"\ncolour = \"red\"\n",
        "colour",
    );
}

#[test]
fn an_input_with_no_name_is_refused() {
    assert_only_plugin_dropped("[[plugins.inputs]]\ntype = \"string\"\n", "name");
}

#[test]
fn an_input_name_that_is_not_a_bare_identifier_is_refused() {
    // The name becomes a `{field}` in a template, so a space or a brace in it
    // names something no template can ever spell. A leading digit is the case
    // the *first* character's rule is about on its own: `9size` is made only of
    // legal identifier characters, and is still not an identifier.
    for name in ["disk size", "disk-size", "{size}", "9size", ""] {
        assert_only_plugin_dropped(
            &format!("[[plugins.inputs]]\nname = {name:?}\ntype = \"string\"\n"),
            "name",
        );
    }
}

#[test]
fn an_input_name_may_begin_with_an_underscore() {
    // The other half of the leading-character rule: `_` is not a letter, so a
    // check that admitted letters alone would refuse a name the row's own field
    // vocabulary would allow.
    let (out, _file) = resolve_plugins(&one_plugin_with(
        "[[plugins.inputs]]\nname = \"_size\"\ntype = \"integer\"\n",
    ));

    assert!(
        out.problems.is_empty(),
        "unexpected: {:?}",
        out.problems.messages()
    );
    assert_eq!(out.config.plugins.all()[0].inputs[0].name, "_size");
}

#[test]
fn an_input_with_an_unknown_type_is_refused() {
    assert_only_plugin_dropped(
        "[[plugins.inputs]]\nname = \"size\"\ntype = \"date\"\n",
        "date",
    );
}

#[test]
fn an_input_with_no_type_is_refused() {
    // No default type: the type decides what the prompt is and what reaches an
    // argv, and guessing `string` for a number would put an unvalidated value
    // on a command line.
    assert_only_plugin_dropped("[[plugins.inputs]]\nname = \"size\"\n", "type");
}

#[test]
fn a_dropdown_without_choices_is_refused() {
    assert_only_plugin_dropped(
        "[[plugins.inputs]]\nname = \"size\"\ntype = \"dropdown\"\n",
        "choices",
    );
}

#[test]
fn a_dropdown_with_empty_choices_is_refused() {
    // A dropdown of nothing is a prompt the operator cannot answer.
    assert_only_plugin_dropped(
        "[[plugins.inputs]]\nname = \"size\"\ntype = \"dropdown\"\nchoices = []\n",
        "choices",
    );
}

#[test]
fn choices_on_an_input_that_is_not_a_dropdown_is_refused() {
    // Accepted silently, the operator would be typing freely into an input they
    // wrote a fixed list for.
    assert_only_plugin_dropped(
        "[[plugins.inputs]]\nname = \"size\"\ntype = \"string\"\nchoices = [\"a\"]\n",
        "choices",
    );
}

#[test]
fn a_default_whose_type_contradicts_the_input_is_refused() {
    for (kind, default) in [
        ("integer", "\"big\""),
        ("number", "\"big\""),
        ("bool", "\"yes\""),
        ("string", "3"),
    ] {
        assert_only_plugin_dropped(
            &format!("[[plugins.inputs]]\nname = \"size\"\ntype = {kind:?}\ndefault = {default}\n"),
            "default",
        );
    }
}

#[test]
fn a_fractional_default_on_an_integer_input_is_refused() {
    // The value reaches an argv verbatim, so `1.5` under `integer` would hand
    // gcloud something the declaration says it will not send.
    assert_only_plugin_dropped(
        "[[plugins.inputs]]\nname = \"size\"\ntype = \"integer\"\ndefault = 1.5\n",
        "default",
    );
}

#[test]
fn a_dropdown_default_outside_its_own_choices_is_refused() {
    assert_only_plugin_dropped(
        "[[plugins.inputs]]\nname = \"size\"\ntype = \"dropdown\"\n\
         choices = [\"a\", \"b\"]\ndefault = \"c\"\n",
        "not one of its choices",
    );
}

#[test]
fn two_inputs_in_one_plugin_cannot_share_a_name() {
    // A duplicate name makes `{size}` ambiguous with nothing on screen to say
    // which declaration won.
    assert_only_plugin_dropped(
        "[[plugins.inputs]]\nname = \"size\"\ntype = \"string\"\n\
         \n[[plugins.inputs]]\nname = \"size\"\ntype = \"integer\"\n",
        "already used",
    );
}

#[test]
fn an_inputs_key_that_is_not_an_array_of_tables_is_refused() {
    assert_only_plugin_dropped("inputs = \"size\"\n", "inputs");
    assert_only_plugin_dropped("inputs = [\"size\"]\n", "inputs[0]");
}

#[test]
fn a_bad_input_does_not_take_its_siblings_with_it() {
    // The module's granularity rule. Without it an operator's typo in one
    // input's type costs them every plugin in the file.
    let (out, _file) = resolve_plugins(
        "version = 1\n\n\
         [[plugins]]\n\
         id    = \"broken\"\n\
         scope = \"compute\"\n\
         key   = \"b\"\n\
         label = \"Broken\"\n\
         args  = [\"version\"]\n\
         \n\
         [[plugins.inputs]]\n\
         name = \"size\"\n\
         type = \"date\"\n\
         \n\
         [[plugins]]\n\
         id    = \"fine\"\n\
         scope = \"compute\"\n\
         key   = \"f\"\n\
         label = \"Describe {name}\"\n\
         args  = [\"compute\", \"instances\", \"describe\", \"{name}\"]\n",
    );

    let problems = out.problems.messages();
    assert!(
        problems.iter().any(|m| m.starts_with("plugins[0]: ")),
        "no warning for the broken entry in {problems:?}"
    );
    assert_eq!(out.config.plugins.len(), 1, "the valid sibling was dropped");
    assert_eq!(out.config.plugins.all()[0].id, "fine");
}

/// The Compute row every offer test below resolves against.
fn compute_row() -> Vec<(JumpField, String)> {
    vec![
        (JumpField::Name, "web-01".to_string()),
        (JumpField::Zone, "europe-west1-b".to_string()),
    ]
}

fn context() -> GcloudContext {
    GcloudContext {
        configuration: GcloudConfigurationName::new("dev"),
        account: None,
        project: None,
        region: None,
        zone: None,
    }
}

/// A plugin set loaded from `body` through the real config parser, asserted
/// clean — a fixture that warned would be testing a plugin that never loaded.
fn loaded(body: &str) -> Plugins {
    let mut file = tempfile::NamedTempFile::new().unwrap();
    file.write_all(body.as_bytes()).unwrap();
    let mut problems = ConfigProblems::default();
    let plugins = Plugins::load(file.path(), &mut problems);
    assert!(
        problems.is_empty(),
        "fixture must be a valid plugin file: {:?}",
        problems.messages()
    );
    plugins
}

#[test]
fn a_plugin_whose_label_names_a_bool_input_is_offered_and_collects_it() {
    // The argv is complete here — only the *caption* needs a value. A label
    // naming any input the prompt collects is offered: the prompt takes the
    // value and the confirmation heading shows the caption the operator wrote.
    // A `bool` used to be the exception, and the id fallback would have
    // been permanent rather than row-dependent, so it was withheld with a
    // reason. A bool is now collected like any other kind, so the caption
    // resolves the same way.
    let plugins = loaded(
        "version = 1\n\n\
         [[plugins]]\n\
         id    = \"grow\"\n\
         scope = \"compute\"\n\
         key   = \"G\"\n\
         label = \"Grow {name} in the {background}\"\n\
         args  = [\"compute\", \"instances\", \"describe\", \"{name}\"]\n\
         \n\
         [[plugins.inputs]]\n\
         name = \"background\"\n\
         type = \"bool\"\n",
    );
    assert_eq!(plugins.len(), 1, "the fixture plugin must load");

    let offers = applicable(
        &plugins,
        ResourceServiceKind::ComputeInstances,
        &context(),
        &compute_row(),
    );

    assert_eq!(
        offers.len(),
        1,
        "a caption naming a bool withheld its plugin: {offers:?}"
    );
    let Command::NeedsValues { inputs } = &offers[0].command else {
        panic!("the caption's value must be collected: {:?}", offers[0]);
    };
    assert_eq!(
        inputs
            .iter()
            .map(|input| input.name.as_str())
            .collect::<Vec<_>>(),
        vec!["background"]
    );
    assert_eq!(
        diagnostics(
            &plugins,
            ResourceServiceKind::ComputeInstances,
            &context(),
            &compute_row(),
        ),
        None,
        "nothing is withheld, so nothing is explained"
    );
}

#[test]
fn an_input_warning_names_the_input_as_well_as_its_index() {
    // An operator reads their file by name. Without the name, a file with five
    // inputs makes them count `[[plugins.inputs]]` tables to find the one at
    // fault.
    let (out, _file) = resolve_plugins(&one_plugin_with(
        "[[plugins.inputs]]\nname = \"note\"\ntype = \"string\"\n\
         \n[[plugins.inputs]]\nname = \"widget\"\ntype = \"strng\"\n",
    ));

    let problems = out.problems.messages();
    let message = problems
        .iter()
        .find(|m| m.starts_with("plugins[0]: "))
        .unwrap_or_else(|| panic!("no warning for plugins[0] in {problems:?}"));
    assert!(
        message.contains("widget"),
        "the warning does not name the input: {message:?}"
    );
    assert!(
        message.contains("inputs[1]"),
        "the warning does not name the input's index: {message:?}"
    );
}

#[test]
fn a_number_default_that_is_not_finite_is_refused() {
    // TOML has `inf` and `nan`, and this text is what would reach gcloud
    // verbatim: `--share inf` is not a number the declaration can claim to be
    // sending.
    for default in ["inf", "-inf", "nan"] {
        assert_only_plugin_dropped(
            &format!(
                "[[plugins.inputs]]\nname = \"share\"\ntype = \"number\"\ndefault = {default}\n"
            ),
            "default",
        );
    }
}

#[test]
fn a_dropdown_choice_may_not_be_empty() {
    // Everywhere else this file refuses emptiness — `id`, `label`, an `args`
    // element — and `Template::render` refuses a row field that resolves to a
    // blank. An empty choice is worse than a blank field: it can never be
    // anything else, so choosing it would compose an argv with an empty element
    // that the confirmation cannot show as anything.
    assert_only_plugin_dropped(
        "[[plugins.inputs]]\nname = \"size\"\ntype = \"dropdown\"\nchoices = [\"\", \"a\"]\n",
        "choices",
    );
}

#[test]
fn a_plugin_whose_argv_names_a_bool_input_is_offered_and_collects_it() {
    // `{background}` is a `bool`. Nothing used to prompt for one, so
    // this plugin loaded and was absent from the palette rather than being
    // offered with a command line built from a value the operator was never
    // shown. The prompt now collects every declared kind, so the value is
    // taken before the confirmation composes the argv — the same path an
    // input of any other kind takes.
    let plugins = loaded(
        "version = 1\n\n\
         [[plugins]]\n\
         id    = \"grow\"\n\
         scope = \"compute\"\n\
         key   = \"G\"\n\
         label = \"Grow {name}\"\n\
         args  = [\"compute\", \"disks\", \"resize\", \"{name}\", \"--async\", \"{background}\"]\n\
         \n\
         [[plugins.inputs]]\n\
         name = \"background\"\n\
         type = \"bool\"\n",
    );
    assert_eq!(plugins.len(), 1, "the fixture plugin must load");

    let offers = applicable(
        &plugins,
        ResourceServiceKind::ComputeInstances,
        &context(),
        &compute_row(),
    );

    assert_eq!(
        offers.len(),
        1,
        "a bool input withheld its plugin: {offers:?}"
    );
    let Command::NeedsValues { inputs } = &offers[0].command else {
        panic!("a bool must be collected, not composed: {:?}", offers[0]);
    };
    assert_eq!(
        inputs
            .iter()
            .map(|input| input.name.as_str())
            .collect::<Vec<_>>(),
        vec!["background"]
    );
    assert_eq!(
        diagnostics(
            &plugins,
            ResourceServiceKind::ComputeInstances,
            &context(),
            &compute_row(),
        ),
        None,
        "nothing is withheld, so nothing is explained"
    );
}

#[test]
fn an_input_no_argv_element_names_does_not_withhold_the_plugin() {
    // The refusal is about a template that cannot render, not about the mere
    // presence of an `inputs` table: this plugin's argv is complete, so it is
    // offered exactly as it was before this slice.
    let plugins = loaded(
        "version = 1\n\n\
         [[plugins]]\n\
         id    = \"describe\"\n\
         scope = \"compute\"\n\
         key   = \"d\"\n\
         label = \"Describe {name}\"\n\
         args  = [\"compute\", \"instances\", \"describe\", \"{name}\"]\n\
         \n\
         [[plugins.inputs]]\n\
         name = \"note\"\n\
         type = \"string\"\n",
    );

    let offers = applicable(
        &plugins,
        ResourceServiceKind::ComputeInstances,
        &context(),
        &compute_row(),
    );

    assert_eq!(offers.len(), 1, "a complete argv was withheld");
    let Command::Ready(argv) = &offers[0].command else {
        panic!(
            "a plugin no template asks a value of is ready: {:?}",
            offers[0]
        );
    };
    assert_eq!(
        argv,
        &vec![
            "--configuration",
            "dev",
            "compute",
            "instances",
            "describe",
            "web-01",
        ]
    );
}
