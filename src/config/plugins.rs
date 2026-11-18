//! User-declared, row-scoped `gcloud` commands: the `[[plugins]]` array of the
//! separate plugin file.
//!
//! A plugin says "when this service's list is in view, offer this labelled
//! command against the selected row, built from these templated argv
//! elements". It is the one part of nuvem's configuration that causes a
//! process to run, which is why it is a **separate file** resolved from the
//! platform config directory and never from `--config`'s directory — see
//! [`crate::config::ResolvedConfig::plugins_path`].
//!
//! Nothing in this module executes anything. It parses and validates and stops
//! at "this declaration is well-formed". Composing a row's argv from a
//! validated plugin belongs to [`crate::app::state::plugins`]; running it
//! belongs to the reducer's `ConfirmPlugin` arm; and the prefix key that would
//! reach a plugin without going through the palette is a later slice.
//!
//! Templates are not reimplemented here. [`crate::config::jumps::Template`]
//! already parses `{field}` against a service's own field vocabulary and
//! renders single-pass by construction, so a substituted value containing `{`
//! is copied out literally rather than re-expanded. A second templater would
//! be a second set of rules to keep in step.
//!
//! Validation granularity follows `[[jumps]]`, not `[hotkeys]`. A hotkey set
//! is one coherent table, so a problem rejects the whole of it; plugins are
//! independent items, and dropping one does not corrupt the others. Each
//! rejected plugin gets its own warning naming its index.

use std::path::Path;

use nuvem_domain::gcp::resource::ResourceServiceKind;

use crate::config::jumps::{Template, fields_for};
use crate::config::problems::{ConfigProblemSource, ConfigProblems};
use crate::config::{config_service_id, resolve_service_kind};

/// The tail every plugin-invalidating warning ends with.
const FALLBACK: &str = "ignoring this plugin";

/// Every key a `[[plugins]]` entry may carry; anything else is a typo and
/// rejects the entry.
const SETTINGS: [&str; 7] = ["id", "scope", "key", "label", "args", "dangerous", "inputs"];

/// Every key a `[[plugins.inputs]]` entry may carry.
const INPUT_SETTINGS: [&str; 4] = ["name", "type", "choices", "default"];

/// Every key the plugin file may carry at the top level. Anything else is a
/// typo — `[[plugin]]` for `[[plugins]]` above all.
const SECTIONS: [&str; 2] = ["version", "plugins"];

/// What an input asks the operator for, and therefore what its value may be.
///
/// `integer` is a type of its own rather than a flag on `number`, because the
/// answer reaches an argv verbatim: one key says everything about what may be
/// sent, with no default to guess and no setting that is meaningless on the
/// other kinds.
///
/// A dropdown's `choices` live *inside* the variant rather than beside it, so
/// "a dropdown with no choices" and "choices on something that is not a
/// dropdown" are both unrepresentable once validation has run, instead of being
/// re-checked wherever an input is used.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PluginInputKind {
    String,
    /// Any number, integral or not.
    Number,
    /// Whole numbers only.
    Integer,
    Bool,
    Dropdown {
        /// Non-empty, in the order the operator wrote them.
        choices: Vec<String>,
    },
}

impl PluginInputKind {
    /// Every spelling `type` accepts. Named in the warning a wrong one earns,
    /// so the list a user is offered cannot drift from the one that parses.
    ///
    /// `pub(crate)` for a second reader with the same stake: the kind walk in
    /// `app::state::plugins` asserts that every kind reaches the prompt, and a
    /// walk over a hand-copied list would silently stop covering a kind added
    /// here.
    pub(crate) const SPELLINGS: [&str; 5] = ["string", "number", "integer", "bool", "dropdown"];
}

/// What a typed field will accept as text.
///
/// Declared here, beside `default_for`, because the two decide the same thing
/// one step apart: `default_for` decides whether a *declared* value may reach an
/// argv, and this decides whether a *typed* one may. The text is what gcloud
/// receives in both cases, so a rule stated twice is a rule that can come to
/// differ — and the direction it would differ in is the dangerous one, since the
/// prompt is the door with no config review behind it.
///
/// The parity is over *values*, not over spellings, and the difference is worth
/// stating because the two sides cannot be made identical. TOML has one spelling
/// per number, so `default_for` never sees `1e3`, `.5`, `+5` or `007`; a typed
/// field does, and accepts each — every one of them denotes a value a
/// declaration of the same number is accepted for, so nothing reaches an argv
/// here that could not be declared. What differs is the text: a declared `1e3`
/// is canonicalised to `1000` before it is stored, and a typed one reaches
/// gcloud as `1e3`, because a typed value is sent as typed (`PluginPrompt`).
///
/// Rust's own parses are not the rule. `"inf".parse::<f64>()` and
/// `"NaN".parse::<f64>()` both succeed, and `--share inf` is not a number, so
/// [`Self::Finite`] carries the finiteness check `default_for` refuses a
/// non-finite float for. [`Self::Whole`] is `i64`, which rejects `1.0` and
/// `1e3` exactly as `default_for` rejects a float default under `integer`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextRule {
    /// Any text at all, as long as there is some.
    Free,
    /// A finite number — `number`.
    Finite,
    /// A whole number — `integer`.
    Whole,
}

impl TextRule {
    /// Whether this text may be sent as the value of a field carrying this rule.
    ///
    /// Emptiness is refused by every rule rather than checked separately: an
    /// argv element built from a blank is not the command the operator meant,
    /// and the numeric parses refuse `""` on their own, so one predicate
    /// answers for every kind of field.
    pub fn accepts(self, text: &str) -> bool {
        match self {
            Self::Free => !text.is_empty(),
            Self::Finite => text.parse::<f64>().is_ok_and(f64::is_finite),
            Self::Whole => text.parse::<i64>().is_ok(),
        }
    }

    /// What a field carrying this rule is still waiting for, as the prompt says
    /// it. Reached only where [`Self::accepts`] just said no, so it names what
    /// would satisfy the field rather than what is wrong with what is there —
    /// the operator can see their own text on the same line.
    pub fn wanted(self) -> &'static str {
        match self {
            Self::Free => "needs a value",
            Self::Finite => "needs a number",
            Self::Whole => "needs a whole number",
        }
    }
}

/// One value the operator supplies for a plugin, rather than nuvem reading it
/// off the selected row.
///
/// An input's name is admitted to its plugin's template vocabulary
/// ([`Template::parse_with_inputs`]), and a template naming one renders only
/// once the prompt has collected a value for it. Every declared kind is asked
/// for, so no kind withholds a plugin: a `number` and an `integer`
/// as text carrying the [`TextRule`] their declared defaults are validated
/// against, a `dropdown` as an index into its choices, and a `bool` as the two
/// states it has, sending the same `true`/`false` text `default_for`
/// normalises a declared bool default into.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginInput {
    /// The `{field}` this input answers for, unique within its plugin and
    /// distinct from every field its scope already offers.
    pub name: String,
    pub kind: PluginInputKind,
    /// The declared default, as the text it would contribute to an argv.
    ///
    /// One representation rather than a typed value: this module's job ends at
    /// "the declaration is well-formed", and the text is what reaches an argv, so
    /// deciding it here means there is one numeric rendering rather than one at
    /// load and another wherever a value is sent. It is the canonical rendering
    /// of the value the file declared and not the file's own spelling of it —
    /// `0.50` is stored as `"0.5"` and `1.0` as `"1"`, which are the same number
    /// written the one way. Whether the value was legal for [`Self::kind`] at all
    /// is decided here too, at load.
    ///
    /// `None` is a well-formed input: a default is optional.
    pub default: Option<String>,
}

/// One user-declared command offered against a row of `scope`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plugin {
    /// Unique within a scope. Names the plugin in warnings; the palette row
    /// shows the rendered [`Plugin::label`], never this.
    pub id: String,
    pub scope: ResourceServiceKind,
    /// The character pressed after the plugin prefix key.
    pub key: char,
    /// The palette entry and the confirmation heading, templated so the
    /// operator is told which row the command names.
    pub label: Template,
    /// One template per argv element. An array, never one string: splitting a
    /// string would be a shell parser under another name, and there is no
    /// shell anywhere on this path.
    pub args: Vec<Template>,
    /// Adds a typed acknowledgement to the confirmation, rather than a
    /// keypress.
    pub dangerous: bool,
    /// The values the operator supplies, in the order they were declared —
    /// which is the order the prompt slice will ask for them.
    ///
    /// Each one's `name` is admitted to this plugin's own [`Plugin::label`] and
    /// [`Plugin::args`] templates and to no others'. Empty for a plugin that
    /// asks for nothing, which is every plugin written before this schema
    /// existed.
    pub inputs: Vec<PluginInput>,
}

/// The validated plugin set.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Plugins {
    plugins: Vec<Plugin>,
}

impl Plugins {
    /// No plugins — the default when the file is absent.
    pub fn empty() -> Self {
        Self::default()
    }

    pub fn is_empty(&self) -> bool {
        self.plugins.is_empty()
    }

    pub fn len(&self) -> usize {
        self.plugins.len()
    }

    /// Every plugin, in file order.
    pub fn all(&self) -> &[Plugin] {
        &self.plugins
    }

    /// Read and validate the plugin file at `path`.
    ///
    /// A missing file is not a problem: it is the overwhelmingly common case,
    /// and warning about it would put a line in front of every user who has no
    /// plugins.
    pub fn load(path: &Path, problems: &mut ConfigProblems) -> Self {
        // Stricter than the other two config reads, and this is the file that
        // earns it: a plugin declaration is the only configuration that causes
        // a process to run, so a `plugins.toml` any local account can append to
        // is a command any local account can get offered — and confirmed by an
        // operator who has no reason to suspect the row. Refused rather than
        // warned about, because a warning leaves the entries loaded.
        if crate::config::read::is_world_writable(path) {
            problems.push(
                ConfigProblemSource::Plugins,
                format!(
                    "{}: world-writable, so any local account could add a command here; \
                     loading no plugins (chmod o-w to restore it)",
                    path.display()
                ),
            );
            return Self::empty();
        }

        let contents = match crate::config::read::read_config_file(path) {
            Ok(Some(contents)) => contents,
            Ok(None) => return Self::empty(),
            Err(message) => {
                problems.push(
                    ConfigProblemSource::Plugins,
                    format!("{}: {message}; loading no plugins", path.display()),
                );
                return Self::empty();
            }
        };

        // `toml::from_str`, not `contents.parse()`: since toml 1.0 the
        // `FromStr` impl on `Value` parses a single VALUE rather than a
        // document, so a plugin file went from parsing to failing at its first
        // key with "unexpected content, expected nothing" -- and this function
        // answers a parse failure by loading no plugins at all, silently.
        let document = match toml::from_str::<toml::Value>(&contents) {
            Ok(document) => document,
            Err(err) => {
                let line = err
                    .span()
                    .map(|span| contents[..span.start].matches('\n').count() + 1);
                let where_ = match line {
                    Some(line) => format!("{}:{line}", path.display()),
                    None => path.display().to_string(),
                };
                problems.push(
                    ConfigProblemSource::Plugins,
                    format!("{where_}: {}; loading no plugins", err.message()),
                );
                return Self::empty();
            }
        };

        // Rejected whole, unlike a bad entry. A version this build does not
        // understand means the *meaning* of every entry is unknown — an entry
        // that parses under v1's rules may say something different under v2's,
        // and running a command that means something else is the failure this
        // whole file is careful about.
        match document.get("version") {
            None | Some(toml::Value::Integer(1)) => {}
            Some(other) => {
                problems.push(
                    ConfigProblemSource::Plugins,
                    format!(
                        "{}: unsupported plugin file version {other}; loading no plugins",
                        path.display()
                    ),
                );
                return Self::empty();
            }
        }

        Self::from_document(&document, problems)
    }

    fn from_document(document: &toml::Value, problems: &mut ConfigProblems) -> Self {
        // A top-level key that is neither of the two this schema has is a
        // typo, and `[[plugin]]` for `[[plugins]]` is the one that costs the
        // user everything: without this the file loads nothing and says
        // nothing, which is indistinguishable from the absent file the README
        // documents as the normal, silent case.
        if let Some(table) = document.as_table() {
            let unknown: Vec<&str> = table
                .keys()
                .map(String::as_str)
                .filter(|key| !SECTIONS.contains(key))
                .collect();
            if !unknown.is_empty() {
                problems.push(
                    ConfigProblemSource::Plugins,
                    format!(
                        "unknown top-level {} {}; plugins are declared as [[plugins]] entries",
                        if unknown.len() == 1 { "key" } else { "keys" },
                        unknown
                            .iter()
                            .map(|key| format!("{key:?}"))
                            .collect::<Vec<_>>()
                            .join(", "),
                    ),
                );
            }
        }

        let Some(raw) = document.get("plugins") else {
            return Self::empty();
        };

        let Some(entries) = raw.as_array() else {
            problems.push(
                ConfigProblemSource::Plugins,
                format!(
                    "plugins: expected an array of plugins ([[plugins]]), found {}; \
                     loading no plugins",
                    raw.type_str()
                ),
            );
            return Self::empty();
        };

        let mut plugins: Vec<Plugin> = Vec::new();
        for (index, entry) in entries.iter().enumerate() {
            let plugin = match validate(entry) {
                Ok(plugin) => plugin,
                Err(problem) => {
                    problems.push(
                        ConfigProblemSource::Plugins,
                        format!("plugins[{index}]: {problem}; {FALLBACK}"),
                    );
                    continue;
                }
            };

            // Uniqueness is per scope, not global: two scopes are never
            // offered at once, so a Compute plugin's key cannot shadow a
            // Cloud SQL one. Checked here rather than in `validate` because
            // it is the only rule about an entry's *neighbours*.
            let clash = plugins.iter().find(|other| {
                other.scope == plugin.scope && (other.id == plugin.id || other.key == plugin.key)
            });
            if let Some(clash) = clash {
                let what = if clash.id == plugin.id {
                    format!("id {:?}", plugin.id)
                } else {
                    format!("key {:?}", plugin.key)
                };
                problems.push(
                    ConfigProblemSource::Plugins,
                    format!(
                        "plugins[{index}]: {what} is already used by plugin {:?} on {}; {FALLBACK}",
                        clash.id,
                        config_service_id(plugin.scope),
                    ),
                );
                continue;
            }

            plugins.push(plugin);
        }
        Self { plugins }
    }
}

/// Validate one raw entry. `Err` carries the problem clause of the warning;
/// the first problem found wins, since the entry is dropped whole either way.
fn validate(entry: &toml::Value) -> Result<Plugin, String> {
    let table = entry.as_table().ok_or_else(|| {
        format!(
            "expected a table of plugin settings, found {}",
            entry.type_str()
        )
    })?;

    if let Some(key) = table.keys().find(|key| !SETTINGS.contains(&key.as_str())) {
        // `version` is the one unknown setting worth naming specially. Written
        // below the tables it is parsed as a key of the last one rather than
        // as a document key, so the whole-file guard never sees it and every
        // entry above has already loaded under rules that may not be theirs.
        // Reported as a misplacement, because "unknown setting" would send the
        // user looking for a typo in a line that is spelled correctly.
        if key == "version" {
            return Err(
                "version must be at the top of the file, above the [[plugins]] tables".to_string(),
            );
        }
        return Err(format!("unknown setting {key:?}"));
    }

    // Shape first, then meaning: every key is read as the type it must have
    // before any of them is interpreted.
    let id = setting_str("id", table.get("id"))?.ok_or("missing id")?;
    let raw_scope = setting_str("scope", table.get("scope"))?.ok_or("missing scope")?;
    let raw_key = setting_str("key", table.get("key"))?.ok_or("missing key")?;
    let raw_label = setting_str("label", table.get("label"))?.ok_or("missing label")?;
    let dangerous = match table.get("dangerous") {
        None => false,
        Some(toml::Value::Boolean(value)) => *value,
        Some(other) => {
            return Err(format!(
                "dangerous must be a boolean, found {}",
                other.type_str()
            ));
        }
    };

    if id.is_empty() {
        return Err("id is empty".to_string());
    }

    let scope = resolve_service_kind(raw_scope)
        .ok_or_else(|| format!("unknown scope service {raw_scope:?}"))?;

    // Exactly one character. A multi-character spec would have to be a chord
    // grammar, and this slice has no dispatch to give one meaning to.
    let mut chars = raw_key.chars();
    let key = match (chars.next(), chars.next()) {
        (Some(c), None) => c,
        _ => return Err(format!("key {raw_key:?} must be a single character")),
    };
    // A space, a tab or a bare escape is one character and still not a binding
    // anyone can press: the terminal either cannot deliver it or the reader
    // has already spent it. Refused here so the user is told, rather than
    // shown `(1 loaded)` for a plugin that can never be reached.
    if key.is_whitespace() || key.is_control() {
        return Err(format!(
            "key {raw_key:?} must be a printable, non-space character"
        ));
    }

    // The inputs widen the template vocabulary, so they are validated before
    // any template is parsed — and after `scope`, because an input may not
    // shadow one of the scope's own fields.
    let inputs = inputs(table.get("inputs"), scope)?;
    let input_names: Vec<&str> = inputs.iter().map(|input| input.name.as_str()).collect();

    // The scope's field table plus this plugin's inputs is the template
    // vocabulary, so `scope` has to be resolved before any template can be
    // parsed.
    let template = |what: &str, raw: &str| {
        if raw.is_empty() {
            return Err(format!("{what} is empty"));
        }
        Template::parse_with_inputs(raw, scope, &input_names)
            .map_err(|problem| format!("{what} {raw:?}: {problem}"))
    };

    let label = template("label", raw_label)?;

    let raw_args = table.get("args").ok_or("missing args")?;
    let raw_args = raw_args.as_array().ok_or_else(|| {
        format!(
            "args must be an array of strings, found {}",
            raw_args.type_str()
        )
    })?;
    if raw_args.is_empty() {
        return Err("args is empty".to_string());
    }
    let mut args = Vec::with_capacity(raw_args.len());
    for (index, value) in raw_args.iter().enumerate() {
        let raw = value
            .as_str()
            .ok_or_else(|| format!("args[{index}] must be a string, found {}", value.type_str()))?;
        args.push(template(&format!("args[{index}]"), raw)?);
    }

    Ok(Plugin {
        id: id.to_string(),
        scope,
        key,
        label,
        args,
        dangerous,
        inputs,
    })
}

/// Validate a plugin's whole `inputs` array against the fields `scope` already
/// offers. Absent is an empty array: a plugin that asks for nothing.
///
/// `Err` carries the problem clause, naming the input by index — and by name
/// once one has been read, because an operator reads their file by name and
/// counts tables only when they have to.
fn inputs(
    raw: Option<&toml::Value>,
    scope: ResourceServiceKind,
) -> Result<Vec<PluginInput>, String> {
    let Some(raw) = raw else {
        return Ok(Vec::new());
    };
    let entries = raw.as_array().ok_or_else(|| {
        format!(
            "inputs must be an array of input tables ([[plugins.inputs]]), found {}",
            raw.type_str()
        )
    })?;

    let mut inputs: Vec<PluginInput> = Vec::with_capacity(entries.len());
    for (index, entry) in entries.iter().enumerate() {
        // Named by index *and* by name, because an operator reads their file by
        // name: counting `[[plugins.inputs]]` tables to find the one at fault is
        // work the warning can do for them. Read straight off the raw table
        // rather than from the parsed input, so a table that failed to parse for
        // some other reason still says which one it was — and falls back to the
        // index alone when the name is what is wrong.
        let named = entry
            .as_table()
            .and_then(|table| table.get("name"))
            .and_then(toml::Value::as_str)
            .map(|name| format!(" {name:?}"))
            .unwrap_or_default();
        let parsed =
            input(entry, scope).map_err(|problem| format!("inputs[{index}]{named}: {problem}"))?;
        // Within one plugin, because that is the scope of the vocabulary an
        // input joins. Two plugins may each declare a `size`; one plugin
        // declaring it twice makes `{size}` ambiguous with nothing on screen to
        // say which table won.
        if let Some(first) = inputs.iter().position(|other| other.name == parsed.name) {
            return Err(format!(
                "inputs[{index}] {:?}: name is already used by inputs[{first}]",
                parsed.name
            ));
        }
        inputs.push(parsed);
    }
    Ok(inputs)
}

/// Validate one `[[plugins.inputs]]` table.
fn input(entry: &toml::Value, scope: ResourceServiceKind) -> Result<PluginInput, String> {
    let table = entry.as_table().ok_or_else(|| {
        format!(
            "expected a table of input settings, found {}",
            entry.type_str()
        )
    })?;

    if let Some(key) = table
        .keys()
        .find(|key| !INPUT_SETTINGS.contains(&key.as_str()))
    {
        return Err(format!("unknown setting {key:?}"));
    }

    let name = setting_str("name", table.get("name"))?.ok_or("missing name")?;
    // The name becomes a `{field}` in a template, so a space, a brace or a dash
    // names something no template can spell — and the row's own fields are all
    // written this way, so this is the vocabulary's existing shape rather than a
    // new rule.
    let identifier = !name.is_empty()
        && name.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_')
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    if !identifier {
        return Err(format!(
            "name {name:?} must be a bare identifier: a letter or \"_\", then letters, digits or \"_\""
        ));
    }

    // The collision that has to be decided rather than discovered. Admitting
    // the input would give the row's value to a template the operator wrote for
    // their own, or the other way round, and no warning would say which.
    if fields_for(scope).iter().any(|(field, _)| *field == name) {
        return Err(format!(
            "name {name:?} is already a field of {}",
            config_service_id(scope)
        ));
    }

    let raw_type = setting_str("type", table.get("type"))?.ok_or("missing type")?;
    if !PluginInputKind::SPELLINGS.contains(&raw_type) {
        return Err(format!(
            "unknown type {raw_type:?}; expected one of {}",
            PluginInputKind::SPELLINGS.join(", ")
        ));
    }

    let raw_choices = table.get("choices");
    let kind = if raw_type == "dropdown" {
        let raw_choices = raw_choices.ok_or("a dropdown needs a non-empty choices array")?;
        let listed = raw_choices.as_array().ok_or_else(|| {
            format!(
                "choices must be an array of strings, found {}",
                raw_choices.type_str()
            )
        })?;
        // A dropdown of nothing is a prompt the operator cannot answer, so it
        // would make the plugin unrunnable rather than merely odd.
        if listed.is_empty() {
            return Err("a dropdown needs a non-empty choices array".to_string());
        }
        let mut choices = Vec::with_capacity(listed.len());
        for (index, value) in listed.iter().enumerate() {
            let choice = value.as_str().ok_or_else(|| {
                format!(
                    "choices[{index}] must be a string, found {}",
                    value.type_str()
                )
            })?;
            // Emptiness is refused everywhere else in this file — `id`, a
            // template, an `args` element — and an empty choice is the worse
            // case: a blank row field can be filled by another row, and a choice
            // can never be anything but what it is, so picking it would compose
            // an argv element the confirmation cannot show as anything.
            if choice.is_empty() {
                return Err(format!("choices[{index}] is empty"));
            }
            choices.push(choice.to_string());
        }
        PluginInputKind::Dropdown { choices }
    } else {
        // Accepted silently, the operator would be typing freely into an input
        // they wrote a fixed list for.
        if raw_choices.is_some() {
            return Err(format!(
                "choices is only meaningful on a dropdown, and this input is a {raw_type}"
            ));
        }
        match raw_type {
            "string" => PluginInputKind::String,
            "number" => PluginInputKind::Number,
            "integer" => PluginInputKind::Integer,
            // Total by the spelling check above: `SPELLINGS` is the only list
            // of types, and `dropdown` took the other branch.
            _ => PluginInputKind::Bool,
        }
    };

    let default = default_for(&kind, table.get("default"))?;

    Ok(PluginInput {
        name: name.to_string(),
        kind,
        default,
    })
}

/// The declared default as the text it would put in an argv, or the problem
/// clause saying why it is not one this input can take. `None` for an input that
/// declares no default, which is well-formed.
fn default_for(
    kind: &PluginInputKind,
    raw: Option<&toml::Value>,
) -> Result<Option<String>, String> {
    let Some(raw) = raw else {
        return Ok(None);
    };

    let text = match (kind, raw) {
        (PluginInputKind::String, toml::Value::String(text)) => text.clone(),
        (PluginInputKind::Bool, toml::Value::Boolean(value)) => value.to_string(),
        (PluginInputKind::Number, toml::Value::Integer(value)) => value.to_string(),
        // TOML has `inf` and `nan`, and this text is what would reach gcloud:
        // `--share inf` is not a number, so a declaration that says it sends one
        // must not be able to say this.
        (PluginInputKind::Number, toml::Value::Float(value)) if !value.is_finite() => {
            return Err(format!("default {value} must be a finite number"));
        }
        (PluginInputKind::Number, toml::Value::Float(value)) => value.to_string(),
        (PluginInputKind::Integer, toml::Value::Integer(value)) => value.to_string(),
        // Named separately from the type mismatch below: `1.5` under `integer`
        // is the right *kind* of thing written to the wrong precision, and the
        // value reaches an argv verbatim, so the declaration would send
        // something it says it will not.
        (PluginInputKind::Integer, toml::Value::Float(value)) => {
            return Err(format!("default {value} must be a whole number"));
        }
        (PluginInputKind::Dropdown { choices }, toml::Value::String(text)) => {
            if !choices.iter().any(|choice| choice == text) {
                return Err(format!("default {text:?} is not one of its choices"));
            }
            text.clone()
        }
        (kind, other) => {
            return Err(format!(
                "default must be {}, found {}",
                match kind {
                    PluginInputKind::String | PluginInputKind::Dropdown { .. } => "a string",
                    PluginInputKind::Number => "a number",
                    PluginInputKind::Integer => "an integer",
                    PluginInputKind::Bool => "a boolean",
                },
                other.type_str()
            ));
        }
    };
    Ok(Some(text))
}

/// Read one setting as a string, or say what type it actually had.
fn setting_str<'a>(key: &str, value: Option<&'a toml::Value>) -> Result<Option<&'a str>, String> {
    match value {
        None => Ok(None),
        Some(toml::Value::String(text)) => Ok(Some(text.as_str())),
        Some(other) => Err(format!(
            "{key} must be a string, found {}",
            other.type_str()
        )),
    }
}
