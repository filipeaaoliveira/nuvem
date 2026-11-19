//! Completeness: every user-facing option the code has is in the documentation.
//!
//! The documentation pins in `tests/documentation_test.rs` check that what the
//! documents say is true. These check the other direction: that nothing the
//! code offers is missing from them. Each test walks one kind of option from
//! the code's own enumeration — clap's `Command` for flags, the config
//! structs and their `SETTINGS` lists for keys, the binding catalog for keys,
//! the registry for services — and fails naming every item the reference page
//! does not document. Where the page is a table, the walk also runs the other
//! way, so a row naming an option that no longer exists fails too.
//!
//! None of these expected sets is written down here. A list typed into a test
//! is a copy of what was documented when the test was written, and it goes on
//! passing after the code grows a new option.

use std::collections::{BTreeMap, BTreeSet};

use crate::support::user_docs;

/// The body of one page, with its frontmatter title as a `# ` line.
fn page(path: &str) -> String {
    user_docs::page(path)
}

/// The lines after `anchor` (a heading or a marker line), up to the next
/// heading at the same or a shallower level when `anchor` is a heading.
fn section<'a>(text: &'a str, anchor: &str) -> &'a str {
    let start = text
        .lines()
        .scan(0usize, |offset, line| {
            let at = *offset;
            *offset += line.len() + 1;
            Some((at, line))
        })
        .find(|(_, line)| *line == anchor)
        .map(|(at, line)| at + line.len() + 1)
        .unwrap_or_else(|| panic!("the page has a {anchor:?} line"));
    let rest = &text[start.min(text.len())..];
    let level = anchor.chars().take_while(|c| *c == '#').count();
    if level == 0 {
        return rest;
    }
    let mut offset = 0;
    let mut fenced = false;
    for line in rest.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") {
            fenced = !fenced;
        } else if !fenced {
            let depth = line.chars().take_while(|c| *c == '#').count();
            if depth > 0 && depth <= level && line[depth..].starts_with(' ') {
                return &rest[..offset];
            }
        }
        offset += line.len() + 1;
    }
    rest
}

/// The rows of the first table in `text`, as trimmed cells, without the
/// header and separator rows.
fn first_table(text: &str) -> Vec<Vec<String>> {
    let mut lines = text.lines().skip_while(|line| !line.starts_with('|'));
    assert!(lines.next().is_some(), "a table follows");
    lines
        .skip(1)
        .take_while(|line| line.starts_with('|'))
        .map(|line| {
            line.trim()
                .trim_start_matches('|')
                .trim_end_matches('|')
                .split('|')
                .map(|cell| cell.trim().to_string())
                .collect()
        })
        .collect()
}

/// Every backticked span in a cell, in order.
fn ticked(cell: &str) -> Vec<String> {
    cell.split('`')
        .skip(1)
        .step_by(2)
        .map(str::to_string)
        .collect()
}

/// The first backticked span in a cell, or a panic naming the cell.
fn first_ticked(cell: &str) -> String {
    ticked(cell)
        .into_iter()
        .next()
        .unwrap_or_else(|| panic!("the cell {cell:?} backticks its value"))
}

fn assert_no_missing(what: &str, missing: &[String]) {
    assert!(
        missing.is_empty(),
        "undocumented {what}: {missing:?}. Document each one, or the reader \
         cannot find it without reading the source"
    );
}

fn assert_no_extra(what: &str, extra: &[String]) {
    assert!(
        extra.is_empty(),
        "documented {what} that the code does not have: {extra:?}"
    );
}

// ---------------------------------------------------------------------------
// Command line
// ---------------------------------------------------------------------------

const CLI_PAGE: &str = "docs/reference/cli.md";

/// Every argument of a command and of every subcommand under it, recursively.
fn every_argument(command: &clap::Command) -> Vec<clap::Arg> {
    let mut arguments: Vec<clap::Arg> = command.get_arguments().cloned().collect();
    for subcommand in command.get_subcommands() {
        arguments.extend(every_argument(subcommand));
    }
    arguments
}

/// Every flag clap accepts, on the root command and on every subcommand, the
/// generated `--help` and `--version` included, has a row in the flags table,
/// spelled with its value placeholder when it takes one. Every row names a
/// real flag.
#[test]
fn cli_reference_documents_every_flag() {
    use clap::CommandFactory;

    let mut command = nuvem::Cli::command();
    command.build();

    let text = page(CLI_PAGE);
    let rows = first_table(section(&text, "## Flags"));
    // Each spelling in the first column, as written: `--config <PATH>`, `-h`.
    let documented: Vec<String> = rows.iter().flat_map(|row| ticked(&row[0])).collect();
    let documented_flag = |spelling: &str| -> Option<&String> {
        documented
            .iter()
            .find(|token| token.as_str() == spelling || token.starts_with(&format!("{spelling} ")))
    };

    let mut missing = Vec::new();
    let mut real = BTreeSet::new();
    for argument in every_argument(&command) {
        let takes_value = argument.get_action().takes_values();
        let mut spellings = Vec::new();
        if let Some(long) = argument.get_long() {
            spellings.push(format!("--{long}"));
        }
        if let Some(short) = argument.get_short() {
            spellings.push(format!("-{short}"));
        }
        for spelling in spellings {
            real.insert(spelling.clone());
            match documented_flag(&spelling) {
                None => missing.push(spelling),
                Some(token) if takes_value && !token.contains(" <") => missing.push(format!(
                    "{spelling} (takes a value, but the row shows no `<VALUE>` placeholder)"
                )),
                Some(_) => {}
            }
        }
    }
    assert_no_missing("command-line flags", &missing);

    let extra: Vec<String> = documented
        .iter()
        .map(|token| token.split(' ').next().unwrap_or_default().to_string())
        .filter(|flag| !real.contains(flag))
        .collect();
    assert_no_extra("command-line flags", &extra);
}

/// Every subcommand, clap's generated `help` included, has a row in the
/// commands table, and every row is a real subcommand.
#[test]
fn cli_reference_documents_every_subcommand() {
    use clap::CommandFactory;

    let mut command = nuvem::Cli::command();
    command.build();
    let real: BTreeSet<String> = command
        .get_subcommands()
        .map(|subcommand| subcommand.get_name().to_string())
        .collect();

    let text = page(CLI_PAGE);
    let documented: BTreeSet<String> = first_table(section(&text, "## Commands"))
        .iter()
        .map(|row| first_ticked(&row[0]))
        .collect();

    assert_no_missing(
        "subcommands",
        &real.difference(&documented).cloned().collect::<Vec<_>>(),
    );
    assert_no_extra(
        "subcommands",
        &documented.difference(&real).cloned().collect::<Vec<_>>(),
    );
}

/// The exit codes `src/exit.rs` defines, plus `0`, are exactly the codes the
/// command-line reference and the README tabulate.
///
/// The codes are read from the source rather than from the constants by name,
/// so a code added to `exit.rs` is expected here the moment it exists. The
/// named constants are compared too, so a parse that found nothing fails
/// rather than passing on an empty set.
#[test]
fn exit_codes_are_all_documented() {
    let source = std::fs::read_to_string("src/exit.rs").expect("src/exit.rs is readable");
    let mut real: BTreeSet<String> = source
        .lines()
        .filter_map(|line| line.strip_prefix("pub const "))
        .filter_map(|rest| rest.split_once(": u8 = "))
        .map(|(_, value)| value.trim_end_matches(';').trim().to_string())
        .collect();
    for code in [
        nuvem::exit::RUNTIME,
        nuvem::exit::USAGE,
        nuvem::exit::NO_TERMINAL,
        nuvem::exit::STARTUP,
    ] {
        assert!(
            real.contains(&code.to_string()),
            "the scan of src/exit.rs found {real:?}, which misses the exit code {code}"
        );
    }
    real.insert("0".to_string());

    let cli = page(CLI_PAGE);
    let readme = std::fs::read_to_string("README.md").expect("README.md is readable");
    for (name, table) in [
        (CLI_PAGE, first_table(section(&cli, "## Exit codes"))),
        ("README.md", first_table(section(&readme, "## Exit status"))),
    ] {
        let documented: BTreeSet<String> = table.iter().map(|row| first_ticked(&row[0])).collect();
        assert_eq!(
            documented, real,
            "{name} tabulates every exit code nuvem can return"
        );
    }
}

// ---------------------------------------------------------------------------
// Config file
// ---------------------------------------------------------------------------

const CONFIG_PAGE: &str = "docs/configuration/config-file.md";

/// The fields of `struct <name>` in `source` as `(name, type)`, minus the
/// `#[serde(flatten)]` catch-all every config struct carries for unknown keys.
fn struct_fields(source: &str, name: &str) -> Vec<(String, String)> {
    let start = source
        .find(&format!("pub struct {name} {{"))
        .unwrap_or_else(|| panic!("src/config/file.rs declares {name}"));
    let body = &source[start..];
    let body = &body[body.find('{').expect("the struct opens") + 1..];
    let body = &body[..body.find("\n}").expect("the struct closes")];
    body.lines()
        .filter_map(|line| line.trim().strip_prefix("pub "))
        .filter_map(|rest| rest.split_once(':'))
        .map(|(field, ty)| {
            (
                field.trim().to_string(),
                ty.trim().trim_end_matches(',').to_string(),
            )
        })
        .filter(|(field, _)| field != "unknown")
        .collect()
}

/// The struct an `Option<...>` field holds, when that struct is declared in
/// the same file: a nested table the deserializer types field by field.
fn nested_struct<'a>(source: &str, ty: &'a str) -> Option<&'a str> {
    let inner = ty.strip_prefix("Option<")?.strip_suffix('>')?;
    source
        .contains(&format!("pub struct {inner} {{"))
        .then_some(inner)
}

/// A documented key, or a family of documented keys, the code accepts.
#[derive(Debug)]
enum KeyPattern {
    /// One key, spelled exactly.
    Exact(String),
    /// A table keyed by names the user chooses: any row under the prefix.
    Under(String),
}

impl KeyPattern {
    fn matches(&self, key: &str) -> bool {
        match self {
            Self::Exact(exact) => key == exact,
            Self::Under(prefix) => key.starts_with(prefix.as_str()),
        }
    }
}

/// Every key the config file accepts, as the patterns the key table must
/// cover.
///
/// The walk starts at `FileConfig`. A field holding a struct declared in
/// `src/config/file.rs` is a typed table and is walked field by field, at any
/// depth; every field found is first confirmed against the real deserializer,
/// so a field renamed by a serde attribute or misread by the scan fails here.
/// A field the deserializer keeps as raw TOML is validated later by a module
/// of its own, and the keys inside it come from that module's own list. A raw
/// table with no list here fails the walk: its keys would otherwise go
/// unchecked.
fn config_key_patterns() -> Vec<KeyPattern> {
    use nuvem::config::file::FileConfig;
    use nuvem::config::hotkeys::RebindableAction;
    use nuvem::config::{columns, favorites, jumps, theme};

    let source = std::fs::read_to_string("src/config/file.rs").expect("src/config/file.rs");

    // Whether `key`, written at `path`, reaches the struct rather than the
    // unknown-key catch-all. A known key with a value of the wrong type is a
    // type error, which also proves it is known.
    let known = |path: &[String], key: &str| -> bool {
        let mut document = String::new();
        if !path.is_empty() {
            document.push_str(&format!("[{}]\n", path.join(".")));
        }
        document.push_str(&format!("{key} = 1\n"));
        match toml::from_str::<FileConfig>(&document) {
            Err(_) => true,
            Ok(config) => match path.first().map(String::as_str) {
                None => !config.unknown.contains_key(key),
                Some("general") => config
                    .general
                    .is_some_and(|general| !general.unknown.contains_key(key)),
                Some(other) => panic!("teach `known` the unknown-key map of [{other}]"),
            },
        }
    };
    assert!(
        !known(&[], "not_a_config_key"),
        "the deserializer oracle reports unknown keys as unknown"
    );

    let mut patterns = Vec::new();
    let mut stack: Vec<(Vec<String>, String)> = vec![(Vec::new(), "FileConfig".to_string())];
    while let Some((path, name)) = stack.pop() {
        let fields = struct_fields(&source, &name);
        assert!(!fields.is_empty(), "the scan found no fields in {name}");
        for (field, ty) in fields {
            assert!(
                known(&path, &field),
                "{field:?} is a {name} field the deserializer does not accept"
            );
            let mut full = path.clone();
            full.push(field.clone());
            let dotted = full.join(".");
            if let Some(inner) = nested_struct(&source, &ty) {
                stack.push((full, inner.to_string()));
                continue;
            }
            let scalar = ["Option<u64>", "Option<bool>", "Option<String>"].contains(&ty.as_str());
            if scalar {
                patterns.push(KeyPattern::Exact(dotted));
                continue;
            }
            match dotted.as_str() {
                "columns" => patterns.extend(
                    columns::SETTINGS
                        .iter()
                        .map(|key| KeyPattern::Exact(format!("columns.<service>.{key}"))),
                ),
                "jumps" => patterns.extend(
                    jumps::SETTINGS
                        .iter()
                        .map(|key| KeyPattern::Exact(format!("jumps[].{key}"))),
                ),
                "favorites" => patterns.extend(
                    favorites::KEYS
                        .iter()
                        .map(|(key, _)| KeyPattern::Exact(format!("favorites.{key}"))),
                ),
                "theme" => patterns.extend(
                    theme::THEME_KEYS
                        .iter()
                        .map(|key| KeyPattern::Under(format!("theme.{key}.<"))),
                ),
                // Keyed by the user's own names: one placeholder row each. The
                // hotkey actions themselves are held to the hotkeys page.
                "aliases" => patterns.push(KeyPattern::Under("aliases.<".to_string())),
                "hotkeys" => {
                    assert!(!RebindableAction::ALL.is_empty());
                    patterns.push(KeyPattern::Under("hotkeys.<".to_string()));
                }
                // Read only to be refused: plugins belong in their own file.
                "plugins" => patterns.push(KeyPattern::Exact(dotted)),
                other => panic!(
                    "the config file accepts a raw table `{other}` ({ty}) whose keys this \
                     walk does not know: list them here from the module that validates it"
                ),
            }
        }
    }
    patterns
}

/// Every key the config file accepts has a row in the key table on the config
/// file page, and every row is a key the code accepts.
#[test]
fn config_reference_documents_every_key() {
    let patterns = config_key_patterns();
    let text = page(CONFIG_PAGE);
    let documented: Vec<String> = first_table(section(&text, "## Keys"))
        .iter()
        .map(|row| first_ticked(&row[0]))
        .collect();

    let missing: Vec<String> = patterns
        .iter()
        .filter(|pattern| !documented.iter().any(|key| pattern.matches(key)))
        .map(|pattern| format!("{pattern:?}"))
        .collect();
    assert_no_missing("config-file keys", &missing);

    let extra: Vec<String> = documented
        .iter()
        .filter(|key| !patterns.iter().any(|pattern| pattern.matches(key)))
        .cloned()
        .collect();
    assert_no_extra("config-file keys", &extra);
}

/// Every rebindable action is in the hotkeys table under its config key, with
/// its real default key, and nothing else is.
#[test]
fn hotkeys_page_documents_every_rebindable_action() {
    use nuvem::app::projection::binding::KeyTrigger;
    use nuvem::config::hotkeys::RebindableAction;

    let spell = |trigger: KeyTrigger| -> String {
        match trigger {
            KeyTrigger::Char(c) => c.to_string(),
            KeyTrigger::CtrlChar(c) => format!("ctrl-{c}"),
            other => panic!("a default hotkey is a character, not {other:?}"),
        }
    };
    let real: BTreeMap<String, String> = RebindableAction::ALL
        .iter()
        .map(|action| {
            (
                action.config_key().to_string(),
                spell(action.default_trigger()),
            )
        })
        .collect();

    let text = page("docs/configuration/hotkeys.md");
    let documented: BTreeMap<String, String> = first_table(section(&text, "# Hotkeys"))
        .iter()
        .map(|row| (first_ticked(&row[0]), first_ticked(&row[1])))
        .collect();

    assert_eq!(
        documented, real,
        "the hotkeys table lists every rebindable action with its default key"
    );
}

/// Every built-in theme is in the themes table under its canonical name, and
/// every name in the table selects that theme.
#[test]
fn themes_page_documents_every_theme() {
    use nuvem::config::theme::ThemeChoice;

    // The exhaustive match is the enumeration: a theme added to the enum stops
    // this compiling until it is listed here, and so cannot be left out.
    let every = [
        ThemeChoice::Default,
        ThemeChoice::NoColor,
        ThemeChoice::HighContrast,
    ];
    for theme in every {
        match theme {
            ThemeChoice::Default | ThemeChoice::NoColor | ThemeChoice::HighContrast => {}
        }
    }

    let text = page("docs/configuration/themes.md");
    let documented: Vec<String> = first_table(section(&text, "# Themes and frame colors"))
        .iter()
        .map(|row| first_ticked(&row[0]))
        .collect();
    for name in &documented {
        assert!(
            ThemeChoice::from_str_lenient(name).is_some_and(|theme| theme.name() == name),
            "the themes table names {name:?}, which is not a theme's canonical name"
        );
    }
    let missing: Vec<String> = every
        .iter()
        .map(|theme| theme.name().to_string())
        .filter(|name| !documented.contains(name))
        .collect();
    assert_no_missing("themes", &missing);
}

// ---------------------------------------------------------------------------
// Keys
// ---------------------------------------------------------------------------

/// Every key binding of every view is in the key reference, with the label
/// the footer shows for it.
///
/// Each table on the page is preceded by a `<!-- help-context: ... -->` marker
/// naming the help contexts it documents; the marker `global` names the table
/// of keys every normal view shares. For each context of `ALL_HELP_CONTEXTS`,
/// every binding `context_bindings` returns must be a row of that context's
/// table or of the global table, and every row of the context's table must be
/// one of its bindings. The global table must hold only bindings Home offers,
/// since Home is the view that has nothing but the global keys.
#[test]
fn keys_page_documents_every_binding_of_every_view() {
    use nuvem::app::projection::binding::{ALL_HELP_CONTEXTS, HelpContext, context_bindings};

    const MARKER: &str = "<!-- help-context: ";
    let text = page("docs/reference/keys.md");

    let mut tables: BTreeMap<String, BTreeSet<(String, String)>> = BTreeMap::new();
    let lines: Vec<&str> = text.lines().collect();
    for (index, line) in lines.iter().enumerate() {
        let Some(names) = line
            .strip_prefix(MARKER)
            .and_then(|rest| rest.strip_suffix(" -->"))
        else {
            continue;
        };
        // The table directly under the marker, if the view has one.
        let after: Vec<&str> = lines[index + 1..]
            .iter()
            .skip_while(|line| line.trim().is_empty())
            .take_while(|line| line.starts_with('|'))
            .copied()
            .collect();
        let rows: BTreeSet<(String, String)> = if after.is_empty() {
            BTreeSet::new()
        } else {
            first_table(&after.join("\n"))
                .iter()
                .map(|row| (first_ticked(&row[0]), row[1].clone()))
                .collect()
        };
        for name in names.split_whitespace() {
            assert!(
                tables.insert(name.to_string(), rows.clone()).is_none(),
                "the key reference documents {name} twice"
            );
        }
    }

    let global = tables
        .remove("global")
        .expect("the key reference has a global table");
    let pairs = |context: HelpContext| -> BTreeSet<(String, String)> {
        context_bindings(context)
            .iter()
            .map(|binding| (binding.key.to_string(), binding.label.to_string()))
            .collect()
    };

    let home = pairs(HelpContext::Home);
    let invented: Vec<String> = global
        .difference(&home)
        .map(|(key, label)| format!("{key} ({label})"))
        .collect();
    assert_no_extra("global keys", &invented);

    let mut missing = Vec::new();
    let mut extra = Vec::new();
    for context in ALL_HELP_CONTEXTS {
        let name = format!("{context:?}");
        let Some(table) = tables.remove(&name) else {
            missing.push(format!("{name}: the whole view"));
            continue;
        };
        let real = pairs(*context);
        for (key, label) in &real {
            let pair = (key.clone(), label.clone());
            if !table.contains(&pair) && !global.contains(&pair) {
                missing.push(format!("{name}: {key} ({label})"));
            }
        }
        for (key, label) in table.difference(&real) {
            extra.push(format!("{name}: {key} ({label})"));
        }
    }
    assert_no_missing("key bindings", &missing);
    assert_no_extra("key bindings", &extra);

    let unknown: Vec<String> = tables.into_keys().collect();
    assert_no_extra("help contexts", &unknown);
}

/// The services whose selection `L` can scope Cloud Logging to.
///
/// Asked of the reducer itself rather than listed: for each service, an app
/// with an active configuration and an empty list is sent `L`. A service that
/// has a log scope refuses only because nothing is selected; a service without
/// one refuses whatever is selected, with a message of its own. Only the first
/// kind can ever offer an `Open logs for selected ...` command.
fn services_with_selection_logs() -> Vec<nuvem_domain::gcp::resource::ResourceServiceKind> {
    use nuvem::app::reducer::root::reduce;
    use nuvem::app::{Action, AppState, EffectResult, ResourcesAction, Workspace};
    use nuvem_domain::gcp::configuration::{GcloudConfiguration, GcloudConfigurationName};
    use nuvem_domain::gcp::resource::ResourceServiceKind;

    ResourceServiceKind::ALL
        .into_iter()
        .filter(|kind| {
            let mut app = AppState::new();
            reduce(
                &mut app,
                &Action::EffectResult(EffectResult::ConfigurationsLoaded {
                    configurations: vec![GcloudConfiguration {
                        name: GcloudConfigurationName::new("dev"),
                        is_active: true,
                        account: Some("dev@example.com".to_string()),
                        project: Some("dev-project".to_string()),
                        region: Some("europe-west1".to_string()),
                        zone: Some("europe-west1-b".to_string()),
                    }],
                }),
            );
            app.root.workspace = Workspace::Resources;
            reduce(
                &mut app,
                &Action::Resources(ResourcesAction::SelectService(*kind)),
            );
            reduce(
                &mut app,
                &Action::Resources(ResourcesAction::OpenLogsForSelection),
            );
            let message = app.root.system.status_message.clone().unwrap_or_default();
            message == format!("No {} selected to open logs for", kind.selection_noun())
        })
        .collect()
}

/// Every palette command some view can offer, every service's and every
/// logs-for-selection one included, is a row of the palette reference under its
/// exact label, with exactly the aliases it answers to. A logs-for-selection
/// command for a service with no log scope can never appear, so it must not be
/// documented either.
#[test]
fn palette_reference_documents_every_command() {
    use nuvem::app::state::command::CommandDescriptor;

    let logged = services_with_selection_logs();
    assert!(
        !logged.is_empty(),
        "the reducer probe found no service with a log scope, so it is probing wrongly"
    );
    // `CommandDescriptor::all` generates a logs-for-selection command for
    // exactly the services the reducer can scope logs to, so nothing is
    // filtered out here: a command that can only refuse does not exist.
    let generated: Vec<_> = CommandDescriptor::all()
        .into_iter()
        .filter_map(|command| match command {
            CommandDescriptor::OpenLogsForSelected(kind) => Some(kind),
            _ => None,
        })
        .collect();
    assert_eq!(
        generated, logged,
        "the logs-for-selection commands must be exactly the services `L` \
         can scope logs to"
    );
    let real: BTreeMap<String, BTreeSet<String>> = CommandDescriptor::all()
        .into_iter()
        .map(|command| {
            (
                command.label().into_owned(),
                command
                    .aliases()
                    .iter()
                    .map(|alias| (*alias).to_string())
                    .collect(),
            )
        })
        .collect();

    let text = page("docs/reference/commands.md");
    let mut documented: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut rest = text.as_str();
    while let Some(at) = rest.find("\n| Command |") {
        rest = &rest[at + 1..];
        for row in first_table(rest) {
            let aliases = ticked(&row[1]).into_iter().collect();
            assert!(
                documented.insert(row[0].clone(), aliases).is_none(),
                "the palette reference lists {:?} twice",
                row[0]
            );
        }
        rest = &rest[1..];
    }

    let missing: Vec<String> = real
        .keys()
        .filter(|label| !documented.contains_key(*label))
        .cloned()
        .collect();
    assert_no_missing("palette commands", &missing);
    let extra: Vec<String> = documented
        .keys()
        .filter(|label| !real.contains_key(*label))
        .cloned()
        .collect();
    assert_no_extra("palette commands", &extra);
    for (label, aliases) in &real {
        assert_eq!(
            &documented[label], aliases,
            "the palette reference lists the aliases of {label:?}"
        );
    }
}

// ---------------------------------------------------------------------------
// Services
// ---------------------------------------------------------------------------

const SERVICES_PAGE: &str = "docs/reference/services/overview.md";

/// Resolves a documented service id the way a config file would, through the
/// `[columns]` parser, so the id the page publishes is one a user may write.
fn resolve_service(id: &str) -> Option<nuvem_domain::gcp::resource::ResourceServiceKind> {
    use nuvem::config::columns::ColumnSettings;
    use nuvem::config::problems::ConfigProblems;
    use nuvem_domain::gcp::resource::ResourceServiceKind;

    let mut raw = BTreeMap::new();
    raw.insert(id.to_string(), toml::Value::Table(toml::map::Map::new()));
    let mut problems = ConfigProblems::default();
    let settings = ColumnSettings::from_file(&raw, &mut problems, &ColumnSettings::empty());
    ResourceServiceKind::ALL
        .into_iter()
        .find(|kind| settings.get(*kind).is_some())
}

/// Every registered service is a row of the services table: its label, an id
/// the config file resolves to it, every alias the registry gives it, and its
/// category. Each row links to a page whose title is the service's label.
#[test]
fn services_reference_documents_every_service() {
    use nuvem::app::service_registry::{ServiceTarget, service_descriptors};

    let text = page(SERVICES_PAGE);
    let rows = first_table(section(&text, "# All services"));

    let mut documented: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for row in &rows {
        let link = &row[0];
        let label = link
            .strip_prefix('[')
            .and_then(|rest| rest.split_once("]("))
            .map(|(label, _)| label.to_string())
            .unwrap_or_else(|| panic!("the services row {row:?} links its service's page"));
        let target = link
            .split_once("](")
            .and_then(|(_, rest)| rest.strip_suffix(')'))
            .expect("the link closes");
        let path = format!("docs/reference/services/{target}");
        let (frontmatter, _) = user_docs::split(
            &path,
            &std::fs::read_to_string(&path).unwrap_or_else(|_| panic!("{path} exists")),
        );
        assert_eq!(
            frontmatter.title, label,
            "{path} is titled with its service's label"
        );
        documented.insert(label, row.clone());
    }

    let mut missing = Vec::new();
    for descriptor in service_descriptors() {
        let Some(row) = documented.remove(descriptor.label) else {
            missing.push(descriptor.label.to_string());
            continue;
        };
        let id = first_ticked(&row[1]);
        let kind = descriptor.target.and_then(ServiceTarget::resource_kind);
        assert_eq!(
            resolve_service(&id),
            kind,
            "the services table gives {:?} the id {id:?}, which the config file does not \
             resolve to it",
            descriptor.label
        );
        let aliases: BTreeSet<String> = ticked(&row[2]).into_iter().collect();
        let real: BTreeSet<String> = descriptor
            .aliases
            .iter()
            .map(|alias| (*alias).to_string())
            .collect();
        assert_eq!(aliases, real, "the aliases of {:?}", descriptor.label);
        assert_eq!(
            row[3], descriptor.category,
            "the category of {:?}",
            descriptor.label
        );
    }
    assert_no_missing("services", &missing);
    assert_no_extra("services", &documented.into_keys().collect::<Vec<_>>());
}

/// The tab lists a capability-table cell documents: the text after each
/// `detail: ` up to the next `;` or the end of the cell.
fn documented_tabs(cell: &str) -> Vec<Vec<String>> {
    cell.split("detail: ")
        .skip(1)
        .map(|rest| {
            rest.split(';')
                .next()
                .unwrap_or_default()
                .split(',')
                .map(|tab| tab.trim().to_string())
                .collect()
        })
        .collect()
}

/// The capability table has a row per service, and four of its columns say
/// what the code says: the detail tabs `Enter` opens (from each service's tab
/// enum), marks and tunnels (from `service_capabilities`), and `L` (from the
/// reducer, through `services_with_selection_logs`).
///
/// The tab enums have no common trait to walk, so which enum belongs to which
/// service is written down below; the tabs themselves come from each enum's
/// own `ALL`. A service with no tab enum must document no `detail:` list.
#[test]
fn services_reference_documents_every_service_capability() {
    use nuvem::app::service_capabilities::{supports_marks, supports_tunnel};
    use nuvem::app::state::resources::{
        CloudRunDetailTab, CloudRunRevisionTab, CloudSqlDetailTab, ComputeDetailTab, GkeDetailTab,
        SecretDetailTab, StorageDetailTab,
    };
    use nuvem_domain::gcp::resource::ResourceServiceKind;

    fn labels<T: Copy>(all: &[T], label: fn(T) -> &'static str) -> Vec<String> {
        all.iter().map(|tab| label(*tab).to_string()).collect()
    }
    let tabs = |kind: ResourceServiceKind| -> Vec<Vec<String>> {
        match kind {
            ResourceServiceKind::ComputeInstances => {
                vec![labels(&ComputeDetailTab::ALL, ComputeDetailTab::label)]
            }
            ResourceServiceKind::GkeClusters => {
                vec![labels(&GkeDetailTab::ALL, GkeDetailTab::label)]
            }
            ResourceServiceKind::CloudSql => {
                vec![labels(&CloudSqlDetailTab::ALL, CloudSqlDetailTab::label)]
            }
            ResourceServiceKind::CloudRunServices => vec![
                labels(&CloudRunDetailTab::ALL, CloudRunDetailTab::label),
                labels(&CloudRunRevisionTab::ALL, CloudRunRevisionTab::label),
            ],
            ResourceServiceKind::SecretManagerSecrets => {
                vec![labels(&SecretDetailTab::ALL, SecretDetailTab::label)]
            }
            ResourceServiceKind::CloudStorage => {
                vec![labels(&StorageDetailTab::ALL, StorageDetailTab::label)]
            }
            ResourceServiceKind::CloudFunctions
            | ResourceServiceKind::PersistentDisks
            | ResourceServiceKind::PubSubTopics
            | ResourceServiceKind::PubSubSubscriptions
            | ResourceServiceKind::BigQueryDatasets
            | ResourceServiceKind::IamServiceAccounts
            | ResourceServiceKind::IamPolicy
            | ResourceServiceKind::VpcNetworks
            | ResourceServiceKind::VpcSubnets
            | ResourceServiceKind::FirewallRules => Vec::new(),
        }
    };

    let text = page(SERVICES_PAGE);
    let rows = first_table(section(&text, "## What each service supports"));
    let mut documented: BTreeMap<String, Vec<String>> =
        rows.into_iter().map(|row| (row[0].clone(), row)).collect();
    let logged = services_with_selection_logs();

    let yes = |cell: &str| match cell {
        "yes" => true,
        "no" => false,
        other => panic!("a capability cell reads yes or no, not {other:?}"),
    };
    let mut missing = Vec::new();
    for kind in ResourceServiceKind::ALL {
        let Some(row) = documented.remove(kind.label()) else {
            missing.push(kind.label().to_string());
            continue;
        };
        assert_eq!(
            documented_tabs(&row[1]),
            tabs(kind),
            "detail tabs of {}",
            kind.label()
        );
        assert_eq!(
            yes(&row[2]),
            supports_marks(kind),
            "marks on {}",
            kind.label()
        );
        assert_eq!(
            yes(&row[3]),
            supports_tunnel(kind),
            "tunnels on {}",
            kind.label()
        );
        assert_eq!(
            yes(&row[4]),
            logged.contains(&kind),
            "`L` on {}",
            kind.label()
        );
    }
    assert_no_missing("services in the capability table", &missing);
    assert_no_extra(
        "services in the capability table",
        &documented.into_keys().collect::<Vec<_>>(),
    );
}

/// Every environment variable nuvem's own code reads is a row of the
/// environment table: each `std::env::var` call site under `src/`, and the
/// variable the log filter is built from.
///
/// Only one direction is checked. `XDG_CONFIG_HOME` and `XDG_CACHE_HOME` are
/// read by the `dirs` crate, which this walk cannot see.
#[test]
fn cli_reference_documents_every_environment_variable() {
    fn sources(dir: &std::path::Path, out: &mut Vec<String>) {
        for entry in std::fs::read_dir(dir).expect("a readable source directory") {
            let path = entry.expect("a directory entry").path();
            if path.is_dir() {
                sources(&path, out);
            } else if path.extension().is_some_and(|ext| ext == "rs") {
                out.push(std::fs::read_to_string(&path).expect("a readable source file"));
            }
        }
    }
    let mut files = Vec::new();
    sources(std::path::Path::new("src"), &mut files);

    let mut real = BTreeSet::new();
    for file in &files {
        for call in ["env::var(\"", "env::var_os(\""] {
            for (at, _) in file.match_indices(call) {
                let rest = &file[at + call.len()..];
                real.insert(rest[..rest.find('"').expect("the name closes")].to_string());
            }
        }
    }
    real.insert(tracing_subscriber::EnvFilter::DEFAULT_ENV.to_string());

    let text = page(CLI_PAGE);
    let documented: BTreeSet<String> = first_table(section(&text, "## Environment"))
        .iter()
        .flat_map(|row| ticked(&row[0]))
        .collect();
    assert_no_missing(
        "environment variables",
        &real.difference(&documented).cloned().collect::<Vec<_>>(),
    );
}

/// The files table names every file nuvem reads or writes in its own
/// directories, with the file names the path functions really produce.
///
/// Only one direction is checked: the log-dump directory comes from a private
/// helper with nothing public to compare against.
#[test]
fn cli_reference_documents_every_file() {
    use nuvem::config::state::state_path_for;
    use nuvem::config::{Config, default_config_path, default_plugins_path};

    let name = |path: std::path::PathBuf| -> String {
        path.file_name()
            .expect("a file path")
            .to_string_lossy()
            .into_owned()
    };
    let real: BTreeSet<String> = [
        name(default_config_path()),
        name(default_plugins_path()),
        name(state_path_for(&default_config_path())),
        name(Config::default().log_file),
    ]
    .into_iter()
    .collect();

    let text = page(CLI_PAGE);
    let documented: BTreeSet<String> = first_table(section(&text, "## Files"))
        .iter()
        .map(|row| first_ticked(&row[0]))
        .collect();
    assert_no_missing(
        "files",
        &real.difference(&documented).cloned().collect::<Vec<_>>(),
    );
}

/// The label-filter table says, for every service, the field `-l` is pushed
/// as, or that label filtering is not supported there.
#[test]
fn filter_guide_documents_the_label_field_of_every_service() {
    use nuvem_domain::gcp::filter::label_filter_field;
    use nuvem_domain::gcp::resource::ResourceServiceKind;

    let text = page("docs/guides/filter-a-list.md");
    let rows = first_table(section(&text, "<!-- label-filter-fields -->"));
    let mut documented: BTreeMap<String, String> = rows
        .into_iter()
        .map(|row| (row[0].clone(), row[1].clone()))
        .collect();

    let mut missing = Vec::new();
    for kind in ResourceServiceKind::ALL {
        let Some(cell) = documented.remove(kind.label()) else {
            missing.push(kind.label().to_string());
            continue;
        };
        match label_filter_field(kind) {
            Some(field) => assert_eq!(
                first_ticked(&cell),
                field,
                "the label field of {}",
                kind.label()
            ),
            None => assert!(
                cell.starts_with("not supported"),
                "{} has no label field, so its row says label filtering is not supported, \
                 not {cell:?}",
                kind.label()
            ),
        }
    }
    assert_no_missing("services in the label-filter table", &missing);
    assert_no_extra(
        "services in the label-filter table",
        &documented.into_keys().collect::<Vec<_>>(),
    );
}

// ---------------------------------------------------------------------------
// Plugins
// ---------------------------------------------------------------------------

/// Every key the plugin file accepts — at the top level, in a `[[plugins]]`
/// entry and in a `[[plugins.inputs]]` table — and every input type is a row
/// of its table in the plugin file reference, and every row is real.
#[test]
fn plugin_reference_documents_every_field_and_input_type() {
    use nuvem::config::plugins::{INPUT_SETTINGS, PluginInputKind, SECTIONS, SETTINGS};

    let text = page("docs/plugins/plugin-file-reference.md");
    for (heading, real) in [
        ("## File layout", &SECTIONS[..]),
        ("## Plugin fields", &SETTINGS[..]),
        ("## Input fields", &INPUT_SETTINGS[..]),
        ("## Input types", &PluginInputKind::SPELLINGS[..]),
    ] {
        let documented: BTreeSet<String> = first_table(section(&text, heading))
            .iter()
            .map(|row| first_ticked(&row[0]))
            .collect();
        let real: BTreeSet<String> = real.iter().map(|key| (*key).to_string()).collect();
        assert_no_missing(
            &format!("plugin-file entries under {heading:?}"),
            &real.difference(&documented).cloned().collect::<Vec<_>>(),
        );
        assert_no_extra(
            &format!("plugin-file entries under {heading:?}"),
            &documented.difference(&real).cloned().collect::<Vec<_>>(),
        );
    }
}

/// Every user page carries the frontmatter the documentation site needs,
/// parsed as YAML the way the site parses it: a title, which the site renders
/// as the page heading, a description for search results and link previews,
/// and an integer `sidebar.order` that places the page in its group.
#[test]
fn every_docs_page_has_a_title_and_description() {
    for path in user_docs::paths() {
        if path == "README.md" {
            continue;
        }
        let text = std::fs::read_to_string(&path).expect("a listed page is readable");
        // `split` panics on invalid YAML, a missing or empty title or
        // description, and a `sidebar.order` that is not an integer.
        let (frontmatter, _) = user_docs::split(&path, &text);
        assert!(
            frontmatter.order >= 0,
            "{path} has a non-negative sidebar order"
        );
        // `page` also refuses a body with a level-1 heading of its own.
        assert!(page(&path).starts_with(&format!("# {}\n", frontmatter.title)));
        assert!(
            !frontmatter.description.trim().is_empty(),
            "{path} has a description"
        );
    }
}
