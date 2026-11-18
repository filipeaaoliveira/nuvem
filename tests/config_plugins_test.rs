//! `plugins.toml` as it behaves through a real config file (slice 1):
//! where it is read from, what reaches `ResolvedConfig`, and what reaches the
//! warning list.
//!
//! The location tests are the safety property of the feature, not plumbing: a
//! `--config` that could drag plugins in would make a repository-supplied
//! config file executable, which is exactly what the separate file exists to
//! prevent.

use std::io::Write;

use clap::Parser;
use nuvem::{Cli, Config, ConfigOutcome};
use nuvem_domain::gcp::resource::ResourceServiceKind;

/// A config path that cannot exist, passed wherever a test is about the plugin
/// file alone. Without it `--config` falls back to the developer's real
/// `~/.config/nuvem/config.toml`, and every `problems.is_empty()` assertion
/// below would be reporting on that file rather than on the plugin file. A
/// missing config file is already silent, so this adds no warning of its own.
const NO_CONFIG: &str = "/nonexistent/nuvem-config-none.toml";

/// Resolve with an explicit `--config` path, returning the temp file so it
/// outlives the resolve call.
fn resolve_with_config(body: &str) -> (ConfigOutcome, tempfile::NamedTempFile) {
    let mut file = tempfile::NamedTempFile::new().unwrap();
    file.write_all(body.as_bytes()).unwrap();
    let cli = Cli::parse_from(["nuvem", "--config", file.path().to_str().unwrap()]);
    (Config::resolve(&cli), file)
}

#[test]
fn a_config_path_does_not_move_where_plugins_are_read_from() {
    // The whole point of the separate file: `nuvem --config ./repo.toml` is a
    // workflow the README advertises, and it must not be able to bring
    // executable declarations with it. If this ever passes by pointing at the
    // config file's directory, a checked-in repo config becomes executable.
    let (out, file) = resolve_with_config("version = 1\n");

    // Asserted as "not *in* that directory", not as "does not share a prefix
    // with it": the platform config dir can legitimately sit under the same
    // ancestor as the temp file (both land under /var/folders/... on macOS
    // when HOME is itself a temp dir), and a `starts_with` check calls that a
    // violation. The directory is what "follows --config" actually means.
    let config_dir = file.path().parent().unwrap();
    assert_ne!(
        out.config.plugins_path.parent(),
        Some(config_dir),
        "plugins path {:?} follows --config into {:?}",
        out.config.plugins_path,
        config_dir,
    );
    assert_eq!(
        out.config.plugins_path,
        nuvem::config::default_plugins_path()
    );
}

#[test]
fn an_explicit_plugins_flag_selects_the_file() {
    let plugins = tempfile::NamedTempFile::new().unwrap();
    let cli = Cli::parse_from(["nuvem", "--plugins", plugins.path().to_str().unwrap()]);

    let out = Config::resolve(&cli);

    assert_eq!(out.config.plugins_path, plugins.path());
}

#[test]
fn a_missing_plugins_file_is_not_a_problem() {
    // Absent is the overwhelmingly common case. It must be silent, or every
    // user without plugins reads a warning forever.
    let cli = Cli::parse_from([
        "nuvem",
        "--config",
        NO_CONFIG,
        "--plugins",
        "/nonexistent/nuvem-plugins-none.toml",
    ]);

    let out = Config::resolve(&cli);

    assert!(
        out.problems.is_empty(),
        "unexpected: {:?}",
        out.problems.messages()
    );
}

/// Resolve with an explicit `--plugins` path, returning the temp file so it
/// outlives the resolve call. `--config` is pinned at [`NO_CONFIG`] so these
/// tests are about the plugin file and nothing else.
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

#[test]
fn a_well_formed_plugin_loads_with_its_templates_parsed() {
    let (out, _file) = resolve_plugins(
        r#"
version = 1

[[plugins]]
id = "describe"
scope = "compute"
key = "d"
label = "Describe {name}"
args = ["compute", "instances", "describe", "{name}", "--zone", "{zone}"]
"#,
    );

    assert!(
        out.problems.is_empty(),
        "unexpected: {:?}",
        out.problems.messages()
    );

    // Filtered here rather than by a scope-filtered accessor on `Plugins`: the
    // set numbers its plugins in file order (`app::state::plugins::PluginId`),
    // and an accessor that hands out a scope-filtered view is the thing a
    // caller would number instead.
    let loaded: Vec<_> = out
        .config
        .plugins
        .all()
        .iter()
        .filter(|plugin| plugin.scope == ResourceServiceKind::ComputeInstances)
        .collect();
    assert_eq!(loaded.len(), 1);

    // Stated as well as counted: `len` and `is_empty` are two answers to the
    // same question, and a set that loaded a plugin must give both.
    assert!(!out.config.plugins.is_empty());
    assert_eq!(out.config.plugins.len(), 1);

    let plugin = loaded[0];
    assert_eq!(plugin.id, "describe");
    assert_eq!(plugin.key, 'd');
    // `dangerous` is absent from the file, so the default must be the safe
    // one. A default of `true` would be merely annoying; a default of `false`
    // is the one worth pinning, because it is the value a user gets without
    // asking for it.
    assert!(!plugin.dangerous);
    // Six templates, one per argv element: `args` is an array of strings and
    // each element becomes exactly one argv element. A single string that got
    // split would land here as some other count.
    assert_eq!(plugin.args.len(), 6);
}

/// The other half of `dangerous`, and the one no fixture in the repository was
/// exercising: a file that says `true` must produce a plugin that *is*
/// dangerous.
///
/// The default is pinned above, so between them the two tests cover both
/// values of the boolean arm. Without this one, an arm that ignored the
/// declared value and answered `false` would leave the whole suite green while
/// silently downgrading `sql instances delete` from "type 1 to confirm" to a
/// bare `Enter` — the acknowledgement is chosen from this field alone
/// (`PluginConfirmation::new`), so this parse is the first link in that chain.
#[test]
fn a_plugin_declared_dangerous_loads_as_dangerous() {
    let (out, _file) = resolve_plugins(
        r#"
version = 1

[[plugins]]
id = "delete-instance"
scope = "cloudsql"
key = "D"
label = "Delete instance {name}"
args = ["sql", "instances", "delete", "{name}"]
dangerous = true
"#,
    );

    // A warning here would mean the entry was dropped, and a dropped entry
    // cannot answer the question this test asks.
    assert!(
        out.problems.is_empty(),
        "unexpected: {:?}",
        out.problems.messages()
    );

    let loaded = out.config.plugins.all();
    assert_eq!(loaded.len(), 1);
    assert_eq!(loaded[0].id, "delete-instance");
    assert!(
        loaded[0].dangerous,
        "a plugin declaring dangerous = true must load as dangerous"
    );
}

#[test]
fn a_plugin_scoped_to_one_service_is_not_offered_to_another() {
    let (out, _file) = resolve_plugins(
        r#"
version = 1

[[plugins]]
id = "describe"
scope = "compute"
key = "d"
label = "Describe {name}"
args = ["compute", "instances", "describe", "{name}"]
"#,
    );

    assert_eq!(
        out.config
            .plugins
            .all()
            .iter()
            .filter(|plugin| plugin.scope == ResourceServiceKind::CloudSql)
            .count(),
        0
    );
}

/// The one message shape every per-entry rejection shares. Asserted through a
/// helper so a change to the wording is one edit, and so each test below is
/// about *which* entry was dropped rather than about formatting.
fn assert_dropped(problems: &[&str], index: usize, needle: &str) {
    let prefix = format!("plugins[{index}]: ");
    let message = problems
        .iter()
        .find(|m| m.starts_with(&prefix))
        .unwrap_or_else(|| panic!("no warning for plugins[{index}] in {problems:?}"));
    assert!(
        message.contains(needle),
        "warning {message:?} does not explain {needle:?}"
    );
    assert!(
        message.ends_with("ignoring this plugin"),
        "warning {message:?} does not say what it did about the problem"
    );
}

#[test]
fn a_broken_plugin_is_dropped_without_taking_its_siblings_with_it() {
    let (out, _file) = resolve_plugins(
        r#"
version = 1

[[plugins]]
id = "broken"
scope = "zzz"
key = "z"
label = "Nope"
args = ["version"]

[[plugins]]
id = "fine"
scope = "cloudsql"
key = "d"
label = "Describe {name}"
args = ["sql", "instances", "describe", "{name}"]
"#,
    );

    assert_dropped(&out.problems.messages(), 0, "zzz");
    assert_eq!(
        out.config.plugins.len(),
        1,
        "the valid sibling was dropped too"
    );
    assert_eq!(out.config.plugins.all()[0].id, "fine");
}

#[test]
fn a_template_naming_a_field_the_scope_lacks_is_refused_by_name() {
    // Cloud SQL rows carry `region`, not `zone`. This is the message
    // `Template::parse` already produces; the test pins that plugins get it
    // rather than accepting a template that could never render.
    let (out, _file) = resolve_plugins(
        r#"
version = 1

[[plugins]]
id = "wrong-field"
scope = "cloudsql"
key = "d"
label = "Describe {name}"
args = ["sql", "instances", "describe", "{name}", "--zone", "{zone}"]
"#,
    );

    assert_dropped(&out.problems.messages(), 0, "zone");
    assert!(out.config.plugins.is_empty());
}

#[test]
fn each_rejected_shape_names_the_setting_that_is_wrong() {
    // One file, one entry per failure class, so the indices are the test's
    // subject. Every entry is otherwise well-formed, so nothing here can pass
    // for the wrong reason.
    let (out, _file) = resolve_plugins(
        r#"
version = 1

[[plugins]]
id = "unknown-key"
scope = "compute"
key = "a"
label = "L"
args = ["version"]
colour = "red"

[[plugins]]
scope = "compute"
key = "b"
label = "L"
args = ["version"]

[[plugins]]
id = "bad-type"
scope = "compute"
key = "c"
label = "L"
args = "version"

[[plugins]]
id = "long-key"
scope = "compute"
key = "ctrl-x"
label = "L"
args = ["version"]

[[plugins]]
id = "unbalanced"
scope = "compute"
key = "e"
label = "Describe {name"
args = ["version"]

[[plugins]]
id = "empty-args"
scope = "compute"
key = "f"
label = "L"
args = []
"#,
    );

    let problems = out.problems.messages();
    assert_dropped(&problems, 0, "colour");
    assert_dropped(&problems, 1, "id");
    assert_dropped(&problems, 2, "args");
    assert_dropped(&problems, 3, "ctrl-x");
    assert_dropped(&problems, 4, "unbalanced");
    assert_dropped(&problems, 5, "args");
    assert!(out.config.plugins.is_empty(), "a malformed entry loaded");
}

#[test]
fn two_plugins_cannot_share_an_id_or_a_key_within_one_scope() {
    // Both are ambiguity the user cannot see: a duplicate id makes two
    // warnings name the same plugin, and a duplicate key makes one of the two
    // unreachable with nothing on screen to say which.
    let (out, _file) = resolve_plugins(
        r#"
version = 1

[[plugins]]
id = "describe"
scope = "compute"
key = "d"
label = "Describe {name}"
args = ["compute", "instances", "describe", "{name}"]

[[plugins]]
id = "describe"
scope = "compute"
key = "e"
label = "Again"
args = ["version"]

[[plugins]]
id = "other"
scope = "compute"
key = "d"
label = "Same key"
args = ["version"]
"#,
    );

    // The needles name the *setting* that clashed, not just the plugin it
    // clashed with. Both messages mention `describe` either way, so a needle
    // of `"describe"` alone would pass even if the warning told the user the
    // wrong thing about which of the two settings was the duplicate.
    let problems = out.problems.messages();
    assert_dropped(&problems, 1, "id \"describe\"");
    assert_dropped(&problems, 2, "key 'd'");
    assert_eq!(out.config.plugins.len(), 1, "a duplicate survived");
    assert_eq!(out.config.plugins.all()[0].id, "describe");
}

#[test]
fn the_same_key_in_two_different_scopes_is_fine() {
    // Scopes are disjoint keyspaces: a Compute plugin and a Cloud SQL plugin
    // are never offered at the same time. Rejecting this would make one
    // service's bindings constrain another's for no reason.
    let (out, _file) = resolve_plugins(
        r#"
version = 1

[[plugins]]
id = "describe"
scope = "compute"
key = "d"
label = "Describe {name}"
args = ["compute", "instances", "describe", "{name}"]

[[plugins]]
id = "describe"
scope = "cloudsql"
key = "d"
label = "Describe {name}"
args = ["sql", "instances", "describe", "{name}"]
"#,
    );

    assert!(
        out.problems.is_empty(),
        "unexpected: {:?}",
        out.problems.messages()
    );
    assert_eq!(out.config.plugins.len(), 2);
}

#[test]
fn a_single_bracket_plugins_section_is_one_warning_not_a_parse_failure() {
    // `[plugins]` written for `[[plugins]]` is the likeliest mistake in a
    // schema whose only section is an array of tables — the same reason
    // `[[jumps]]` takes its section as a raw value.
    let (out, _file) = resolve_plugins("version = 1\n[plugins]\nid = \"x\"\n");

    let problems = out.problems.messages();
    assert!(
        problems.iter().any(|m| m.contains("[[plugins]]")),
        "no warning naming the array syntax in {problems:?}"
    );
    assert!(out.config.plugins.is_empty());
}

#[test]
fn an_unparseable_plugin_file_reports_its_line_and_leaves_the_config_alone() {
    let mut plugins = tempfile::NamedTempFile::new().unwrap();
    plugins
        .write_all(b"version = 1\n[[plugins]]\nthis is : not = toml [[[\n")
        .unwrap();
    let mut config = tempfile::NamedTempFile::new().unwrap();
    config
        .write_all(b"version = 1\n[general]\ntheme = \"no-color\"\n")
        .unwrap();

    let cli = Cli::parse_from([
        "nuvem",
        "--config",
        config.path().to_str().unwrap(),
        "--plugins",
        plugins.path().to_str().unwrap(),
    ]);
    let out = Config::resolve(&cli);

    let problems = out.problems.messages();
    assert!(
        problems.iter().any(|m| m.contains(":3")),
        "no line number in {problems:?}"
    );
    // The plugin file failing must not cost the user their config file. This
    // is the whole reason the two are separate loads.
    assert_eq!(out.config.theme, nuvem::config::theme::ThemeChoice::NoColor);
    assert!(
        out.load_error.is_none(),
        "a plugin problem became a config load error"
    );
    assert!(out.config.plugins.is_empty());
}

#[test]
fn an_unsupported_plugin_file_version_loads_no_plugins_and_says_so() {
    let (out, _file) = resolve_plugins(
        r#"
version = 99

[[plugins]]
id = "describe"
scope = "compute"
key = "d"
label = "Describe {name}"
args = ["compute", "instances", "describe", "{name}"]
"#,
    );

    let problems = out.problems.messages();
    assert!(
        problems.iter().any(|m| m.contains("99")),
        "no warning naming the version in {problems:?}"
    );
    assert!(
        out.config.plugins.is_empty(),
        "plugins from a file this build does not understand were loaded"
    );
}

#[test]
fn info_reports_the_plugin_path_and_how_many_loaded() {
    let (out, file) = resolve_plugins(
        r#"
version = 1

[[plugins]]
id = "describe"
scope = "compute"
key = "d"
label = "Describe {name}"
args = ["compute", "instances", "describe", "{name}"]
"#,
    );

    let report = nuvem::config::info_report(&out, "loaded");

    assert!(
        report.contains(file.path().to_str().unwrap()),
        "info does not name the plugin file it read:\n{report}"
    );
    // The count is what tells a user whose plugin silently did not load that
    // it did not load. A path alone cannot say that.
    assert!(
        report
            .lines()
            .any(|line| line.starts_with("plugins:") && line.contains('1')),
        "info does not report how many plugins loaded:\n{report}"
    );
}

#[test]
fn info_reports_a_plugin_warning_as_a_warning_line() {
    let (out, _file) = resolve_plugins(
        "version = 1\n[[plugins]]\nid = \"x\"\nscope = \"zzz\"\nkey = \"z\"\nlabel = \"L\"\nargs = [\"version\"]\n",
    );

    let report = nuvem::config::info_report(&out, "loaded");

    assert!(
        report.contains("warning: plugins[0]"),
        "a plugin problem is not reported as a warning:\n{report}"
    );
}

#[test]
fn an_unreadable_plugin_path_warns_rather_than_passing_for_absent() {
    // A directory where a file should be is the reachable non-`NotFound` read
    // failure. Absent is silent by design, and this is the case that must not
    // be folded into it: an unreadable path means the user has plugins that
    // are not loading, and saying nothing would leave them wondering why the
    // commands they declared never appear.
    let directory = tempfile::tempdir().unwrap();
    let cli = Cli::parse_from([
        "nuvem",
        "--config",
        NO_CONFIG,
        "--plugins",
        directory.path().to_str().unwrap(),
    ]);

    let out = Config::resolve(&cli);

    let problems = out.problems.messages();
    assert!(
        problems
            .iter()
            .any(|m| m.contains(directory.path().to_str().unwrap())),
        "no warning naming the unreadable path in {problems:?}"
    );
    assert!(out.config.plugins.is_empty());
}

#[test]
fn a_plugins_array_in_the_config_file_is_named_rather_than_ignored() {
    // The one misplacement the two-file split makes likely, and the one its
    // security argument is about: a user who copies the schema into
    // `config.toml` gets a file nuvem accepts and a feature that never
    // appears. Serde ignores unknown top-level keys, so without this the
    // mistake is invisible.
    let (out, _file) = resolve_with_config(
        r#"
version = 1

[[plugins]]
id = "describe"
scope = "compute"
key = "d"
label = "Describe {name}"
args = ["compute", "instances", "describe", "{name}"]
"#,
    );

    let problems = out.problems.messages();
    assert!(
        problems
            .iter()
            .any(|m| m.contains("plugins.toml") && m.contains("config file")),
        "no warning saying where plugin declarations belong in {problems:?}"
    );
}

#[test]
fn a_misspelled_plugins_key_is_named_rather_than_silently_loading_nothing() {
    // `[[plugin]]` for `[[plugins]]`. The single-bracket typo next door
    // already warns; this one used to be indistinguishable from an absent
    // file, which the README documents as the normal, silent case.
    let (out, _file) = resolve_plugins(
        r#"
version = 1

[[plugin]]
id = "describe"
scope = "compute"
key = "d"
label = "Describe {name}"
args = ["compute", "instances", "describe", "{name}"]
"#,
    );

    let problems = out.problems.messages();
    assert!(
        problems.iter().any(|m| m.contains("plugin")),
        "no warning naming the unknown section in {problems:?}"
    );
    assert!(out.config.plugins.is_empty());
}

#[test]
fn a_plugin_file_that_declares_nothing_stays_silent() {
    // The guard above must not turn "I have a plugin file with no plugins in
    // it yet" into a warning; only a key that looks like a mistake earns one.
    let (out, _file) = resolve_plugins("version = 1\n");

    assert!(
        out.problems.is_empty(),
        "unexpected: {:?}",
        out.problems.messages()
    );
}

#[test]
fn a_version_key_inside_an_entry_says_it_is_in_the_wrong_place() {
    // `version` written below the tables is parsed by TOML as a key of the
    // last one, so the whole-file guard never sees it. Reported as the
    // misplacement it is, rather than as a generic unknown setting, because
    // the consequence is that every entry above it loaded under rules that
    // may not be theirs.
    let (out, _file) = resolve_plugins(
        r#"
[[plugins]]
id = "describe"
scope = "compute"
key = "d"
label = "Describe {name}"
args = ["compute", "instances", "describe", "{name}"]
version = 2
"#,
    );

    let problems = out.problems.messages();
    assert!(
        problems
            .iter()
            .any(|m| m.contains("version") && m.contains("top of the file")),
        "no warning naming the misplaced version line in {problems:?}"
    );
}

#[test]
fn a_key_that_is_whitespace_or_a_control_character_is_refused() {
    // A key the terminal cannot deliver, or one that collides with a reader's
    // own space/escape, is not a binding a user can ever press. Every other
    // malformed setting warns; this one used to load clean.
    for raw in ["\" \"", "\"\\t\""] {
        let (out, _file) = resolve_plugins(&format!(
            "version = 1\n[[plugins]]\nid = \"x\"\nscope = \"compute\"\nkey = {raw}\nlabel = \"L\"\nargs = [\"version\"]\n"
        ));

        assert_dropped(&out.problems.messages(), 0, "key");
        assert!(out.config.plugins.is_empty(), "{raw} loaded as a key");
    }
}

#[test]
fn info_distinguishes_a_plugin_file_that_is_absent_from_one_that_declares_nothing() {
    // `(0 loaded)` alone cannot tell a user that nuvem never opened a file —
    // which is exactly the report a macOS user gets after writing
    // ~/.config/nuvem/plugins.toml while nuvem reads Application Support.
    let absent = Cli::parse_from([
        "nuvem",
        "--config",
        NO_CONFIG,
        "--plugins",
        "/nonexistent/nuvem-plugins-none.toml",
    ]);
    let missing = nuvem::config::run_info(&absent);
    assert!(
        missing.contains("not found"),
        "info does not say the plugin file is absent:\n{missing}"
    );

    let (out, _file) = resolve_plugins("version = 1\n");
    let empty = nuvem::config::info_report(&out, "loaded");
    assert!(
        !empty.contains("not found"),
        "a plugin file that exists is reported as absent:\n{empty}"
    );
}
