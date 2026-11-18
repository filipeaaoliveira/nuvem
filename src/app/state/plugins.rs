//! The argv a user-declared plugin composes against the selected row.
//!
//! [`crate::config::plugins`] validated the declaration; this module renders it.
//! A [`Plugin`] in hand is well-formed by construction, so nothing here
//! re-validates the file — what is left to decide is whether *this row* can
//! fill *these* templates, and whether the result is something the operator can
//! be shown truthfully.
//!
//! Nothing here executes anything: the argv is composed so the confirmation
//! can show it, and only `ConfirmPlugin` turns it into a process.
//!
//! There is deliberately **no length bound** on a rendered element. The modal
//! already refuses to render a command line that does not fit
//! (`plugin_confirmation_visible`, `src/ui/mod.rs`), and the keymap gates
//! `Enter` on that same answer, so an over-long element degrades to the
//! small-terminal path rather than to a truncated line the operator confirms
//! having misread. A constant chosen here would be arbitrary and would refuse
//! at load a command the terminal in front of the operator can show.

use std::cell::Cell;

use nuvem_domain::gcp::configuration::GcloudContext;
use nuvem_domain::gcp::resource::ResourceServiceKind;

use crate::app::RootState;
use crate::app::state::jumps::{field_name, lookup};
use crate::config::jumps::{JumpField, Template};
use crate::config::plugins::{Plugin, PluginInput, Plugins};

/// Why a plugin composes no argv for the selected row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unoffered {
    /// A template asked for a field this row has no value for.
    Missing(JumpField),
    /// A substituted value would be read by gcloud as a flag. Carries the
    /// `args` index so the diagnostic can name the element.
    LeadingDash(usize),
    /// A rendered element carries a character the confirmation line cannot
    /// show faithfully. Carries the `args` index.
    Control(usize),
}

// `NeedsInput` was the fourth variant here once. It meant "a template
// names an input of a kind nothing can ask for", which was a `bool` and only
// ever a `bool`; now that the prompt collects every declared kind, no input
// withholds a plugin, so the variant had nothing left that could construct it.
// A refusal no code path can reach is a sentence no operator can read and a
// mutant no test can kill.

/// The argv a plugin composes against the selected row and the values the
/// operator supplied, or the reason it composes none.
///
/// `values` is empty for the plugins that ask for nothing, which is every
/// plugin written before `[[plugins.inputs]]` existed; a plugin that does ask
/// composes no argv until the prompt hands the pairs over, which is why
/// [`Command`] and not an `Option` is what an [`Offer`] carries.
///
/// `--configuration` is injected rather than declared: every built-in argv
/// carries it, the confirmation has to show which context the command names,
/// and a plugin that omitted it would run against whatever configuration is
/// active in the ambient gcloud state rather than the one nuvem is showing.
pub fn plugin_argv(
    plugin: &Plugin,
    context: &GcloudContext,
    fields: &[(JumpField, String)],
    values: &[(String, String)],
) -> Result<Vec<String>, Unoffered> {
    let mut argv = vec![
        "--configuration".to_string(),
        context.configuration.to_string(),
    ];
    for (index, template) in plugin.args.iter().enumerate() {
        argv.push(render_element(template, fields, values, index)?);
    }
    Ok(argv)
}

/// One `args` element rendered against the row and the collected values.
///
/// The leading-dash and control rules police a collected value exactly as they
/// police a row value: text the operator typed can be read by gcloud as a flag,
/// and can carry an ESC into the line they are about to confirm, the same way a
/// row value can.
///
/// [`Template::render`] stops at the first field it cannot resolve, so the
/// field recorded last is that field — the same observation
/// [`crate::app::state::jumps`] makes, and for the same reason: a template's
/// segments belong to the config layer, so watching the lookup is how the
/// diagnostic learns which field to name.
fn render_element(
    template: &Template,
    fields: &[(JumpField, String)],
    values: &[(String, String)],
    index: usize,
) -> Result<String, Unoffered> {
    let missing = Cell::new(None);
    let rendered = template.render(
        |field| {
            let value = lookup(fields, field);
            if value.is_none() {
                missing.set(Some(field));
            }
            value
        },
        |name| {
            values
                .iter()
                .find(|(key, _)| key == name)
                .map(|(_, value)| value.clone())
        },
    );
    // `lookup` never answers with an empty string, so `render` can only have
    // refused after recording a field; the fallback keeps this total. An input
    // cannot be what refused: an argv is composed only for a plugin whose
    // templates name no input, or from a `values` holding every one of them.
    let rendered =
        rendered.ok_or_else(|| Unoffered::Missing(missing.get().unwrap_or(JumpField::Name)))?;

    // Before the leading-dash rule: a control character is the more
    // fundamental refusal, and checking it first keeps the reported reason
    // stable for a value that is both.
    if rendered.chars().any(char::is_control) {
        return Err(Unoffered::Control(index));
    }

    if template.starts_with_field() && rendered.starts_with('-') {
        return Err(Unoffered::LeadingDash(index));
    }

    Ok(rendered)
}

/// What a resolved plugin will run, or what it is still waiting for.
///
/// Two variants rather than an `Option<Vec<String>>` so that "no argv exists
/// yet" cannot be confused with "the argv is empty", and so no half-built argv
/// is constructible: an `Offer` carrying `Ready` is confirmable by
/// construction, which is what keeps the confirmation the only door to a
/// process without a test having to say so.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    /// Nothing to collect; this argv is what runs.
    Ready(Vec<String>),
    /// The plugin's templates name inputs. The prompt collects them and
    /// composes the argv on submit; there is no argv before it does.
    NeedsValues { inputs: Vec<PluginInput> },
}

/// Whether nuvem can prove, from the declaration alone, that running this
/// plugin only reads cloud state.
///
/// Two variants rather than a third `Mutates`, deliberately. The table is an
/// allowlist of *reading* commands, so an unmatched path means "nuvem never
/// vouched for this" — never "nuvem read it and it changes things". Claiming
/// the latter would assert something that was never established; proving it
/// would need a second, symmetric table of mutating commands, doubling a
/// maintenance surface that already rots silently, to buy one nuance in one
/// refusal sentence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeclaredEffect {
    /// Matched a command path nuvem owns, all of which only read.
    Reads,
    /// Not matched: the path is templated, flag-led, or simply not one nuvem
    /// names. `--read-only` refuses it.
    Unproven,
}

/// The gcloud command paths nuvem will vouch for as reading nothing.
///
/// Full anchored paths rather than bare verbs, and that is the whole design.
/// `["compute", "instances", "delete", "list"]` names an instance `list`, so a
/// scan for a reading verb anywhere — or one that walks the leading literal run
/// and takes its last element — classifies a delete as a read. Matching a fixed
/// prefix positionally cannot be fooled that way, because a positional is never
/// at the index a verb is read from.
///
/// Every entry must name a terminal gcloud command, never a command group.
/// The match vouches for whatever follows the matched prefix, which is sound
/// only while nothing that follows can extend the path into a different
/// command: `["compute", "instances"]` in this table would vouch for every verb
/// beneath that group, so `["compute", "instances", "delete", "{name}"]` would
/// classify as reading. A group entry is the one edit that turns this table
/// from an allowlist into a hole, and `every_table_entry_ends_in_a_reading_verb`
/// is what stops it landing.
///
/// Narrow on purpose. Everything absent is `Unproven` and refused, so the cost
/// of an omission is a plugin the operator has to run without `--read-only`,
/// never a mutation let through. This list rots silently as gcloud grows
/// commands; `every_table_entry_is_a_nonempty_path_that_classifies_as_reading`
/// is what makes each entry deliberate.
const READING_COMMANDS: [&[&str]; 21] = [
    &["compute", "instances", "describe"],
    &["compute", "instances", "list"],
    &["compute", "disks", "describe"],
    &["compute", "disks", "list"],
    &["sql", "instances", "describe"],
    &["sql", "instances", "list"],
    &["run", "services", "describe"],
    &["run", "services", "list"],
    &["run", "revisions", "describe"],
    &["run", "revisions", "list"],
    &["functions", "describe"],
    &["functions", "list"],
    &["container", "clusters", "describe"],
    &["container", "clusters", "list"],
    &["pubsub", "topics", "describe"],
    &["pubsub", "topics", "list"],
    &["pubsub", "subscriptions", "describe"],
    &["pubsub", "subscriptions", "list"],
    &["secrets", "describe"],
    &["secrets", "list"],
    &["iam", "service-accounts", "list"],
];

/// Whether `--read-only` may run this plugin, decided from its declaration.
///
/// Reads the `args` **templates** rather than a composed argv, for two reasons
/// that pull the same way. A templated element has no literal text, so a plugin
/// whose verb the operator supplies cannot match the table — which is the only
/// thing that makes matching safe now that an operator can type a verb
/// at the prompt. And the refusal has to fire where the prompt design put it, ahead of
/// the input prompt, where no argv exists yet for a plugin that needs values.
///
/// Substitution cannot defeat the match: it is anchored at element 0 against
/// literals, so a substituted value lands strictly after the matched prefix,
/// where it is a positional or a flag value and cannot change which gcloud
/// command runs. A substituted value that would read as a flag is refused
/// separately by [`Unoffered::LeadingDash`].
pub fn declared_effect(plugin: &Plugin) -> DeclaredEffect {
    // `--configuration <name>` is prepended by the runtime and is not part of
    // the declaration, so element 0 here is the operator's own first element.
    let declared: Vec<Option<&str>> = plugin
        .args
        .iter()
        .map(|template| template.literal())
        .collect();

    let matched = READING_COMMANDS.iter().any(|path| {
        path.len() <= declared.len()
            && path
                .iter()
                .zip(&declared)
                .all(|(expected, actual)| *actual == Some(*expected))
    });
    if matched {
        DeclaredEffect::Reads
    } else {
        DeclaredEffect::Unproven
    }
}

/// Whether `--read-only` refuses this plugin.
///
/// The one place the mode's rule about a plugin lives, called by the palette's
/// submit, the keyed press, and the palette's dimming. Three inlined copies of
/// `read_only && declared_effect(..) != Reads` is how the palette would come to
/// dim a row the reducer runs, or run a row it dims.
///
/// A plugin whose id does not resolve is refused: the set is loaded once at
/// startup so this is unreachable, and the safe answer for an unresolvable
/// plugin is the one that runs nothing.
pub fn plugin_blocked_by_read_only(state: &RootState, id: PluginId) -> bool {
    if !state.read_only {
        return false;
    }
    let PluginId(index) = id;
    state
        .plugins
        .all()
        .get(index)
        .is_none_or(|plugin| declared_effect(plugin) != DeclaredEffect::Reads)
}

/// The inputs this plugin's own templates name, in declaration order.
///
/// Read off the templates rather than off `plugin.inputs` for the reason
/// `refusal` does: an input declared but never named withholds nothing, and
/// prompting for one would ask the operator for a value that reaches no argv.
///
/// [`Template::names_input`] rather than a first-input answer: one template can
/// hold several input segments, and prompting for only the first would compose
/// an argv the operator was never asked to complete.
pub fn prompted_inputs(plugin: &Plugin) -> Vec<PluginInput> {
    let named = |input: &PluginInput| {
        plugin.label.names_input(&input.name)
            || plugin
                .args
                .iter()
                .any(|template| template.names_input(&input.name))
    };
    plugin
        .inputs
        .iter()
        .filter(|input| named(input))
        .cloned()
        .collect()
}

/// One plugin resolved against the selected row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Offer {
    /// Index into the plugin set, so the palette descriptor can stay `Copy`.
    pub id: PluginId,
    /// The plugin's `label` template rendered against this row.
    pub label: String,
    /// The argv, `--configuration` first, or the values still to be collected.
    pub command: Command,
    /// Mirrored from the declaration so the confirmation can demand a typed
    /// acknowledgement without re-reading the plugin set: an `Offer` is what
    /// crosses into the reducer, and a second lookup there could disagree
    /// with the offer the operator was shown.
    pub dangerous: bool,
}

/// A plugin's position in [`Plugins::all`], which is file order.
///
/// An index rather than the plugin's own `id`: the palette carries a `Copy`
/// descriptor per entry, and a `String` there would make every descriptor own
/// an allocation for a name the plugin set already holds.
///
/// An index is a sound identity only because the plugin set is loaded once at
/// startup and never swapped under a running session — a config reload reports
/// that a restart is required rather than replacing it. Making reload live
/// would silently repoint every `PluginId` a modal or palette is already
/// holding at whatever plugin now occupies that slot, so that change has to
/// re-key this type, not just the reload path.
///
/// It is emphatically *not* a position within the plugins of one scope. That
/// is why `resolve` and [`keyed`] both number the whole set with `enumerate`
/// *before* filtering by scope, and why no scope-filtered iterator over
/// [`Plugins`] is offered for a caller to number instead: the two orders agree
/// only when every plugin in the file shares one scope, which is exactly why a
/// set of `[cloudsql-…, compute-…]` is what catches the confusion. Filtering
/// before numbering would answer `0` for the Compute plugin and run the Cloud
/// SQL one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PluginId(pub usize);

/// Every plugin scoped to `service`, resolved into either its offer or the
/// reason it has none. Both public readers go through this one list, so the
/// palette and its explanation can never disagree about which plugins
/// resolved.
fn resolve<'a>(
    plugins: &'a Plugins,
    service: ResourceServiceKind,
    context: &GcloudContext,
    fields: &[(JumpField, String)],
) -> Vec<Result<Offer, (&'a Plugin, Unoffered)>> {
    plugins
        .all()
        .iter()
        .enumerate()
        .filter(|(_, plugin)| plugin.scope == service)
        .map(|(index, plugin)| offer_for(index, plugin, context, fields))
        .collect()
}

/// One plugin, at its position in the whole set, resolved against the row.
///
/// Shared by [`resolve`] and [`keyed`] so the two doors into a plugin compose
/// the same `Offer` from the same declaration. A second inline construction is
/// how the keyed press and the palette would come to disagree about a
/// plugin's `dangerous` flag or its label fallback.
fn offer_for<'a>(
    index: usize,
    plugin: &'a Plugin,
    context: &GcloudContext,
    fields: &[(JumpField, String)],
) -> Result<Offer, (&'a Plugin, Unoffered)> {
    // Every declared kind is promptable now, so no input withholds a
    // plugin any more. What used to guard this point was a check that all of
    // them were: a plugin naming a `bool` was refused whole rather than
    // prompting for its other values, since an operator who typed two of three
    // and was then refused would have typed for nothing.
    let inputs = prompted_inputs(plugin);
    let command = if inputs.is_empty() {
        Command::Ready(plugin_argv(plugin, context, fields, &[]).map_err(|why| (plugin, why))?)
    } else {
        // The argv is deliberately not composed here, not even speculatively:
        // it does not exist until the operator supplies the values, and a
        // provisional one held in state is the thing the confirmation exists to
        // rule out. Nor are the *field* templates rendered on their own to
        // withhold a plugin this row cannot complete: `render_element` cannot
        // tell an unresolved input from an unresolved field, so a field-only
        // pass would report every input-bearing plugin as missing a row field.
        // The row's own refusal therefore lands on submit, where `refusal`
        // names it truthfully (`app::reducer::resources`).
        Command::NeedsValues { inputs }
    };
    // A label that will not render costs a caption, not the offer: unlike an
    // argv element it is not part of what runs, so the fallback is the
    // plugin's own id — the name the operator wrote. For a label naming an
    // input the fallback lasts until the prompt collects the value — the
    // confirmation heading, drawn after collection, shows the real label.
    let label = render_label(&plugin.label, fields).unwrap_or_else(|| plugin.id.clone());
    Ok(Offer {
        id: PluginId(index),
        label,
        command,
        dangerous: plugin.dangerous,
    })
}

/// The sentence explaining why one plugin composed no argv for this row.
///
/// Extracted so [`diagnostics`] (which asks the scope-level question),
/// [`keyed`] (which asks about the one plugin the operator pressed) and the
/// prompt's submit (which asks whether the collected values compose an argv)
/// share a vocabulary. The three questions are different — see [`keyed`] — but
/// the wording of a refusal must not be.
///
/// Reachable from the reducer because a plugin can be offered on a row that
/// cannot fill its *field* templates: the argv is not composed until submit, so
/// submit is where such a row is first refused, and a sentence written there
/// would blame the typed values for a field no typing can supply.
pub(crate) fn refusal(plugin: &Plugin, why: Unoffered) -> String {
    let id = &plugin.id;
    let noun = plugin.scope.selection_noun();
    match why {
        Unoffered::Missing(field) => format!(
            "Plugin {id:?} needs {}, which this {noun} has no value for",
            field_name(plugin.scope, field)
        ),
        Unoffered::LeadingDash(index) => format!(
            "Plugin {id:?} args[{index}] renders a value beginning with \"-\" for this {noun}, \
             which gcloud would read as a flag"
        ),
        Unoffered::Control(index) => {
            format!("Plugin {id:?} args[{index}] renders an unprintable value for this {noun}")
        }
    }
}

/// The plugins this row can actually run, in file order — which is the order
/// the palette lists them in.
///
/// A plugin that cannot resolve produces no entry at all rather than one that
/// would compose an incomplete command; [`diagnostics`] composes the sentence
/// saying why.
///
/// **Nothing renders that sentence yet.** [`diagnostics`] has no caller outside
/// its own tests, and the palette is deliberately not that caller — it is a
/// global list where a missing entry is invisible and never "empty", so a
/// plugin-scoped explanation hung off it would fire at operators who simply
/// have no plugin scoped to the service in view. The keybinding slice is where
/// the operator is told, on the press that produces nothing, exactly as
/// `no_jumps_reason` (`app::reducer::jumps`) reaches
/// [`crate::app::state::jumps::jump_diagnostics`] when `J` would open an empty
/// picker. A keybinding shipped without that wiring re-creates the silence this
/// function exists to end.
pub fn applicable(
    plugins: &Plugins,
    service: ResourceServiceKind,
    context: &GcloudContext,
    fields: &[(JumpField, String)],
) -> Vec<Offer> {
    resolve(plugins, service, context, fields)
        .into_iter()
        .filter_map(Result::ok)
        .collect()
}

/// Why no plugin resolved even though this service *has* one — the sentence an
/// operator is owed once there is a gesture that produces nothing. `None` when
/// at least one resolved, so the caller keeps its own service-level and
/// selection-level wording.
///
/// There is no such caller today: this composes the wording and the keybinding
/// slice is what shows it (see [`applicable`]). Read the tense here as
/// "would be told", not "is told".
///
/// Reporting the *first* unresolvable plugin rather than all of them keeps the
/// message to something a status line can hold; a user with two broken plugins
/// fixes them one at a time either way. This mirrors
/// [`crate::app::state::jumps::jump_diagnostics`], because it answers the same
/// question about the same selection.
pub fn diagnostics(
    plugins: &Plugins,
    service: ResourceServiceKind,
    context: &GcloudContext,
    fields: &[(JumpField, String)],
) -> Option<String> {
    let resolved = resolve(plugins, service, context, fields);
    if resolved.iter().any(Result::is_ok) {
        return None;
    }
    let (plugin, why) = resolved.into_iter().find_map(Result::err)?;
    Some(refusal(plugin, why))
}

/// What a `,`-then-key press resolves to for the service in view.
///
/// Three answers rather than an `Option`, because the two failures are
/// different sentences to a different reader: `Refused` means fix the file or
/// the selection, `Unbound` means that key was never declared here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Keyed {
    /// The plugin bound to this key composed an argv for this row.
    Offer(Offer),
    /// A plugin is bound to this key and could not compose one. Carries the
    /// sentence `refusal` composed for *that* plugin.
    Refused(String),
    /// No plugin in this scope declares this key.
    Unbound,
}

/// The plugin bound to `key` within `service`, resolved against the row.
///
/// **Not [`diagnostics`].** That function answers the scope-level question —
/// did *anything* resolve, and if not why — and returns `None` as soon as any
/// sibling resolves. A keyed press asks about one plugin, so reusing
/// `diagnostics` here would report a sibling's problem under the key the
/// operator pressed, and would go silent in exactly the case that matters:
/// one plugin broken, another working.
///
/// `find` rather than a filter-and-check: [`crate::config::plugins`] drops a
/// plugin whose `key` is already used within its scope, so at most one entry
/// can match and the first is it.
///
/// The scope filter is part of the `find` predicate and the index comes from
/// `enumerate` *before* it, for the reason [`PluginId`] documents at length:
/// numbering within a scope would answer `PluginId(0)` for a file whose first
/// entry belongs to another service.
pub fn keyed(
    plugins: &Plugins,
    service: ResourceServiceKind,
    context: &GcloudContext,
    fields: &[(JumpField, String)],
    key: char,
) -> Keyed {
    let found = plugins
        .all()
        .iter()
        .enumerate()
        .find(|(_, plugin)| plugin.scope == service && plugin.key == key);
    let Some((index, plugin)) = found else {
        return Keyed::Unbound;
    };
    match offer_for(index, plugin, context, fields) {
        Ok(offer) => Keyed::Offer(offer),
        Err((plugin, why)) => Keyed::Refused(refusal(plugin, why)),
    }
}

/// Whether this service has any plugin at all, which is what `,` reads to
/// decide whether to arm.
///
/// A question about the file, not about the row: a scope whose only plugin no
/// row can resolve still arms the prefix, and the press then explains. Arming
/// on the row's answer instead would make `,` silently inert on the rows that
/// most need the diagnostic.
pub fn any_scoped(plugins: &Plugins, service: ResourceServiceKind) -> bool {
    plugins.all().iter().any(|plugin| plugin.scope == service)
}

/// This scope's declared keys in file order, for the hint the status line
/// shows while the prefix is armed. File order because that is the order the
/// palette lists plugins in.
pub fn bound_keys(plugins: &Plugins, service: ResourceServiceKind) -> Vec<char> {
    plugins
        .all()
        .iter()
        .filter(|plugin| plugin.scope == service)
        .map(|plugin| plugin.key)
        .collect()
}

/// The label rendered against the row, or `None` when it names a field this row
/// cannot fill.
fn render_label(template: &Template, fields: &[(JumpField, String)]) -> Option<String> {
    template.render(|field| lookup(fields, field), |_| None)
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use tempfile::NamedTempFile;

    use super::*;
    use crate::config::config_service_id;
    use crate::config::plugins::PluginInputKind;
    use crate::config::problems::ConfigProblems;
    use nuvem_domain::gcp::configuration::{GcloudConfigurationName, GcloudContext};

    /// One fixture entry: a whole `[[plugins]]` table, scope included, so a
    /// test can build a file whose entries do *not* all share one scope.
    struct Entry<'a> {
        scope: ResourceServiceKind,
        id: &'a str,
        label: &'a str,
        args: &'a [&'a str],
        /// The plugin's key, when the test is *about* the key. `None` takes the
        /// derivation in [`plugins_from`], which is what almost every fixture
        /// here wants.
        ///
        /// Spelling it is the only way to build the case that matters most to
        /// [`keyed`]: two scopes sharing one character. That file is legal —
        /// key uniqueness is per scope — but the derivation cannot produce it,
        /// so without this a `keyed` that found by key and *then* rejected on
        /// scope would pass every test in this module while making a plugin
        /// permanently unreachable from its own key.
        key: Option<char>,
        /// Written into the file only when set, so every other fixture here
        /// still exercises the omitted-key default the config layer applies.
        dangerous: bool,
    }

    /// A plugin set built the only way a plugin is ever built: through
    /// [`Plugins::load`] over a real file. A fixture that took a shortcut past
    /// validation would be testing a `Plugin` the config layer would never hand
    /// over.
    ///
    /// Each scope is spelled by [`config_service_id`] rather than by a literal,
    /// so the walk below takes its vocabulary from the config layer instead of
    /// from a second list that could drift from it.
    fn plugins_from(entries: &[Entry<'_>]) -> Plugins {
        let mut body = "version = 1\n".to_string();
        for (index, entry) in entries.iter().enumerate() {
            let scope = config_service_id(entry.scope);
            let (id, label, args) = (entry.id, entry.label, entry.args);
            // A distinct key per entry unless the test spelled one: `Plugins::load`
            // drops a plugin whose key is already used within its scope, which
            // would leave the fixture quietly one plugin short. Deriving from the
            // whole-file index rather than a per-scope counter keeps that true
            // across scopes too, at the cost of never producing the shared
            // character `Entry::key` exists to spell.
            let key = entry.key.unwrap_or_else(|| {
                char::from(b'a' + u8::try_from(index).expect("a small fixture"))
            });
            body.push_str(&format!(
                "\n[[plugins]]\nid = {id:?}\nscope = {scope:?}\nkey = {key:?}\n\
                 label = {label:?}\nargs = {args:?}\n"
            ));
            if entry.dangerous {
                body.push_str("dangerous = true\n");
            }
        }
        let mut file = NamedTempFile::new().expect("a temp plugin file");
        file.write_all(body.as_bytes()).expect("the fixture writes");

        let mut problems = ConfigProblems::default();
        let plugins = Plugins::load(file.path(), &mut problems);

        // Without these a malformed fixture would load *no* plugins and the
        // tests below would be asserting about an empty set.
        assert!(
            problems.is_empty(),
            "fixture must be a valid plugin file: {:?}",
            problems.messages()
        );
        assert_eq!(plugins.len(), entries.len(), "every entry must load");
        plugins
    }

    /// A plugin set loaded from a whole file body, for the fixtures [`Entry`]
    /// cannot spell: `[[plugins.inputs]]` is a sub-table, and a builder that
    /// took a list of inputs would be a second, partial encoder of the same
    /// schema. Loading through [`Plugins::load`] for the reason
    /// [`plugins_from`] does — a `Plugin` built any other way is one the config
    /// layer would never hand over.
    fn plugins_from_toml(body: &str) -> Plugins {
        let mut file = NamedTempFile::new().expect("a temp plugin file");
        file.write_all(body.as_bytes()).expect("the fixture writes");

        let mut problems = ConfigProblems::default();
        let plugins = Plugins::load(file.path(), &mut problems);

        assert!(
            problems.is_empty(),
            "fixture must be a valid plugin file: {:?}",
            problems.messages()
        );
        assert!(!plugins.is_empty(), "the fixture must load a plugin");
        plugins
    }

    /// The common shape: every entry on one scope, labelled `"Describe {name}"`.
    fn plugins_for(kind: ResourceServiceKind, entries: &[(&str, &[&str])]) -> Plugins {
        let entries: Vec<Entry<'_>> = entries
            .iter()
            .map(|(id, args)| Entry {
                scope: kind,
                id,
                label: "Describe {name}",
                args,
                key: None,
                dangerous: false,
            })
            .collect();
        plugins_from(&entries)
    }

    /// The argv of an offer that is ready to run.
    ///
    /// Panics rather than answering an empty slice for an offer still waiting
    /// for values: a test asserting about an argv would otherwise pass against
    /// an offer that has none.
    fn ready_argv(offer: &Offer) -> &[String] {
        match &offer.command {
            Command::Ready(argv) => argv,
            Command::NeedsValues { inputs } => {
                panic!("expected a ready argv, the offer is waiting for {inputs:?}")
            }
        }
    }

    /// One compute-scoped plugin with the given `args`.
    fn compute_plugin(args: &[&str]) -> Plugin {
        plugins_for(ResourceServiceKind::ComputeInstances, &[("p", args)])
            .all()
            .first()
            .cloned()
            .expect("one plugin loaded")
    }

    /// `GcloudContext` has no `Default` — every field is written out here
    /// rather than adding one, because a context is normally derived from a
    /// real configuration and a default would invite fixtures elsewhere.
    fn context(configuration: &str) -> GcloudContext {
        GcloudContext {
            configuration: GcloudConfigurationName::new(configuration),
            account: None,
            project: None,
            region: None,
            zone: None,
        }
    }

    /// The injected `--configuration` is the whole reason a plugin cannot
    /// silently target a different project than the one nuvem is displaying.
    #[test]
    fn an_argv_is_the_active_configuration_then_the_rendered_elements() {
        let plugin = compute_plugin(&[
            "compute",
            "instances",
            "describe",
            "{name}",
            "--zone",
            "{zone}",
        ]);
        let fields = vec![
            (JumpField::Name, "web-01".to_string()),
            (JumpField::Zone, "us-central1-a".to_string()),
        ];

        let argv =
            plugin_argv(&plugin, &context("my-proj"), &fields, &[]).expect("every field resolves");

        assert_eq!(
            argv,
            vec![
                "--configuration",
                "my-proj",
                "compute",
                "instances",
                "describe",
                "web-01",
                "--zone",
                "us-central1-a",
            ]
        );
    }

    /// A plugin composes an argv whole or not at all: an element that names a
    /// field the row cannot fill would otherwise become a `gcloud` invocation
    /// missing an argument. The refusal names *which* field, because the next
    /// slice's diagnostic can only be specific if this is.
    #[test]
    fn an_element_naming_a_field_the_row_cannot_fill_composes_nothing() {
        let plugin = compute_plugin(&["compute", "instances", "describe", "{name}", "{zone}"]);
        // A blank rather than an absent pair: that is how `selected_fields`
        // reports a field the resource has no value for, and a key built from
        // a blank is not a key.
        let fields = vec![
            (JumpField::Name, "web-01".to_string()),
            (JumpField::Zone, String::new()),
        ];

        let refusal = plugin_argv(&plugin, &context("my-proj"), &fields, &[])
            .expect_err("a blank is no zone");

        assert_eq!(refusal, Unoffered::Missing(JumpField::Zone));
    }

    /// Every declarable kind is offered with its value still to collect, and
    /// none withholds its plugin.
    ///
    /// This walk had a second side once — an unpromptable kind, which
    /// was `bool` and only ever `bool`, withheld its plugin and named the input
    /// in a refusal. Admitting the last kind emptied that side, and a branch no
    /// row can reach is one no assertion in it can fail.
    ///
    /// A walk over the kinds rather than a test per kind, because the set is
    /// what widens: a kind admitted to the schema without being taught to the
    /// prompt leaves an offer whose prompt has no field for it, and that is
    /// what this fails on. Its list is checked against
    /// `PluginInputKind::SPELLINGS` so a kind added there cannot quietly go
    /// unwalked here.
    #[test]
    fn every_input_kind_is_prompted_for_and_none_withholds_its_plugin() {
        assert_eq!(
            PluginInputKind::SPELLINGS.len(),
            5,
            "a kind was added to the schema; walk it here too"
        );
        for kind in PluginInputKind::SPELLINGS {
            let choices = if kind == "dropdown" {
                "choices = [\"10\", \"20\"]\n"
            } else {
                ""
            };
            let plugins = plugins_from_toml(&format!(
                "version = 1\n\n\
                 [[plugins]]\n\
                 id = \"grow\"\n\
                 scope = \"compute\"\n\
                 key = \"G\"\n\
                 label = \"Grow {{name}}\"\n\
                 args = [\"compute\", \"disks\", \"resize\", \"{{name}}\", \"--size\", \
                 \"{{disk_size}}\"]\n\
                 \n\
                 [[plugins.inputs]]\n\
                 name = \"disk_size\"\n\
                 type = {kind:?}\n\
                 {choices}"
            ));
            let fields = vec![(JumpField::Name, "web-01".to_string())];
            let ctx = context("my-proj");
            let offers = applicable(
                &plugins,
                ResourceServiceKind::ComputeInstances,
                &ctx,
                &fields,
            );
            let explanation = diagnostics(
                &plugins,
                ResourceServiceKind::ComputeInstances,
                &ctx,
                &fields,
            );

            assert_eq!(offers.len(), 1, "a {kind} input must not cost the offer");
            let Command::NeedsValues { inputs } = &offers[0].command else {
                panic!("a {kind} input must be collected, not composed into an argv");
            };
            assert_eq!(
                inputs
                    .iter()
                    .map(|input| input.name.as_str())
                    .collect::<Vec<_>>(),
                vec!["disk_size"],
                "the prompt must have a field for the {kind} it was offered on"
            );
            assert_eq!(
                explanation, None,
                "nothing is withheld for a {kind}, so there is nothing to explain"
            );
        }
    }

    /// A row value that begins with `-` is not a shell problem — there is no
    /// shell — it is gcloud's own flag parser reading a positional as an
    /// option. No GCP resource name begins with `-`, so refusing costs nothing
    /// real.
    #[test]
    fn a_substituted_value_that_would_read_as_a_flag_is_refused() {
        let plugin = compute_plugin(&["compute", "instances", "describe", "{name}"]);
        let fields = vec![(JumpField::Name, "--project=other".to_string())];

        let refusal =
            plugin_argv(&plugin, &context("my-proj"), &fields, &[]).expect_err("must refuse");

        assert_eq!(refusal, Unoffered::LeadingDash(3));
    }

    /// The refusal is about *substituted* values, not about the operator's own
    /// literals. A plugin that declares a flag is the normal case and must
    /// survive.
    #[test]
    fn a_literal_flag_the_operator_wrote_is_not_refused() {
        let plugin = compute_plugin(&["compute", "--zone={zone}", "--quiet"]);
        let fields = vec![(JumpField::Zone, "us-central1-a".to_string())];

        let argv =
            plugin_argv(&plugin, &context("my-proj"), &fields, &[]).expect("literals are fine");

        assert_eq!(
            argv,
            vec![
                "--configuration",
                "my-proj",
                "compute",
                "--zone=us-central1-a",
                "--quiet"
            ]
        );
    }

    /// A newline makes one argv element read as two lines in the modal, so the
    /// operator confirms a command line that is not the one being composed.
    /// This is the same reason `is_resource_name` refuses control characters —
    /// the preview is the whole safety argument.
    #[test]
    fn a_rendered_element_carrying_a_control_character_is_refused() {
        let plugin = compute_plugin(&["compute", "instances", "describe", "{name}"]);
        for hostile in ["web\n01", "web\u{0}01", "web\u{1b}[2J01"] {
            let fields = vec![(JumpField::Name, hostile.to_string())];

            let refusal = plugin_argv(&plugin, &context("my-proj"), &fields, &[])
                .expect_err("a control character must not reach the preview");

            assert_eq!(refusal, Unoffered::Control(3), "for {hostile:?}");
        }

        // Hostile both ways. Which refusal is reported is a decision, not an
        // accident: the operator is told the element cannot be *shown*, which
        // is the problem that survives fixing the leading dash.
        let fields = vec![(JumpField::Name, "-web\n01".to_string())];

        let refusal = plugin_argv(&plugin, &context("my-proj"), &fields, &[])
            .expect_err("a control character must not reach the preview");

        assert_eq!(refusal, Unoffered::Control(3));
    }

    /// The offers and the explanation of an empty offer list read the same
    /// resolution, exactly as `applicable_jumps`/`jump_diagnostics` do, so the
    /// picker and its explanation can never disagree about which plugins
    /// resolved.
    #[test]
    fn a_plugin_whose_field_this_row_lacks_is_not_offered_and_says_why() {
        let plugins = plugins_for(
            ResourceServiceKind::ComputeInstances,
            &[(
                "has-zone",
                &[
                    "compute",
                    "instances",
                    "describe",
                    "{name}",
                    "--zone",
                    "{zone}",
                ],
            )],
        );
        let fields = vec![(JumpField::Name, "web-01".to_string())]; // no zone value

        let ctx = context("my-proj");
        assert!(
            applicable(
                &plugins,
                ResourceServiceKind::ComputeInstances,
                &ctx,
                &fields
            )
            .is_empty()
        );
        assert_eq!(
            diagnostics(
                &plugins,
                ResourceServiceKind::ComputeInstances,
                &ctx,
                &fields
            ),
            Some(
                "Plugin \"has-zone\" needs zone, which this GCE instance has no value for"
                    .to_string()
            )
        );
    }

    /// One resolvable plugin means there is nothing to explain — the same rule
    /// `jump_diagnostics` follows, so a working plugin does not put a warning
    /// on the status line next to itself.
    #[test]
    fn a_resolvable_plugin_silences_the_diagnostic_for_its_broken_sibling() {
        let plugins = plugins_for(
            ResourceServiceKind::ComputeInstances,
            &[
                (
                    "broken",
                    &[
                        "compute",
                        "instances",
                        "describe",
                        "{name}",
                        "--zone",
                        "{zone}",
                    ],
                ),
                ("fine", &["compute", "instances", "describe", "{name}"]),
            ],
        );
        let fields = vec![(JumpField::Name, "web-01".to_string())];

        let ctx = context("my-proj");
        let offers = applicable(
            &plugins,
            ResourceServiceKind::ComputeInstances,
            &ctx,
            &fields,
        );

        assert_eq!(offers.len(), 1);
        assert_eq!(offers[0].label, "Describe web-01");
        assert_eq!(
            diagnostics(
                &plugins,
                ResourceServiceKind::ComputeInstances,
                &ctx,
                &fields
            ),
            None
        );
    }

    /// A `PluginId` numbers the whole plugin set, not the plugins of the scope
    /// in view. The two agree in every single-scope file, so the fixture holds
    /// a Cloud SQL plugin *before* the Compute one: numbering after the scope
    /// filter would answer `PluginId(0)` here, and the palette would then run
    /// the Cloud SQL command against the selected instance.
    #[test]
    fn a_plugin_is_numbered_within_the_whole_set_not_within_its_scope() {
        let plugins = plugins_from(&[
            Entry {
                scope: ResourceServiceKind::CloudSql,
                id: "cloudsql-describe",
                label: "Describe {name}",
                args: &["sql", "instances", "describe", "{name}"],
                key: None,
                dangerous: false,
            },
            Entry {
                scope: ResourceServiceKind::ComputeInstances,
                id: "compute-describe",
                label: "Describe {name}",
                args: &["compute", "instances", "describe", "{name}"],
                key: None,
                dangerous: false,
            },
        ]);
        let fields = vec![(JumpField::Name, "web-01".to_string())];

        let offers = applicable(
            &plugins,
            ResourceServiceKind::ComputeInstances,
            &context("my-proj"),
            &fields,
        );

        assert_eq!(offers.len(), 1);
        assert_eq!(offers[0].id, PluginId(1));
        // The argv is asserted beside the id so a wrong id cannot be read as a
        // cosmetic off-by-one: index 1 is the plugin that composed *this*.
        assert_eq!(
            ready_argv(&offers[0]),
            vec![
                "--configuration",
                "my-proj",
                "compute",
                "instances",
                "describe",
                "web-01"
            ]
        );
    }

    /// The two refusals that are about a *rendered* value name the `args`
    /// element they refused, because the index is the operator's only clue to
    /// which of a dozen elements is at fault.
    #[test]
    fn a_refused_element_is_named_by_its_args_index_in_the_diagnostic() {
        let plugins = plugins_for(
            ResourceServiceKind::ComputeInstances,
            &[("describe", &["compute", "instances", "describe", "{name}"])],
        );
        let ctx = context("my-proj");

        let dashed = vec![(JumpField::Name, "--project=other".to_string())];
        assert_eq!(
            diagnostics(
                &plugins,
                ResourceServiceKind::ComputeInstances,
                &ctx,
                &dashed
            ),
            Some(
                "Plugin \"describe\" args[3] renders a value beginning with \"-\" for this \
                 GCE instance, which gcloud would read as a flag"
                    .to_string()
            )
        );

        let unprintable = vec![(JumpField::Name, "web\n01".to_string())];
        assert_eq!(
            diagnostics(
                &plugins,
                ResourceServiceKind::ComputeInstances,
                &ctx,
                &unprintable
            ),
            Some(
                "Plugin \"describe\" args[3] renders an unprintable value for this GCE instance"
                    .to_string()
            )
        );
    }

    /// A label is a caption, not part of what runs, so a label naming a field
    /// this row lacks costs the operator the caption and not the command. The
    /// fallback is the plugin's own id — the name they wrote in the file, and
    /// the name every warning about this plugin already uses.
    #[test]
    fn a_label_that_will_not_render_falls_back_to_the_plugin_id_and_keeps_the_offer() {
        let plugins = plugins_from(&[Entry {
            scope: ResourceServiceKind::ComputeInstances,
            id: "restart",
            // Every arg resolves; only the caption names the absent field.
            label: "Restart {zone}",
            args: &["compute", "instances", "describe", "{name}"],
            key: None,
            dangerous: false,
        }]);
        let fields = vec![(JumpField::Name, "web-01".to_string())]; // no zone value

        let ctx = context("my-proj");
        let offers = applicable(
            &plugins,
            ResourceServiceKind::ComputeInstances,
            &ctx,
            &fields,
        );

        assert_eq!(
            offers.len(),
            1,
            "an unrenderable label must not cost the offer"
        );
        assert_eq!(offers[0].label, "restart");
        assert_eq!(
            ready_argv(&offers[0]),
            vec![
                "--configuration",
                "my-proj",
                "compute",
                "instances",
                "describe",
                "web-01"
            ]
        );
        // And nothing to explain: the plugin resolved.
        assert_eq!(
            diagnostics(
                &plugins,
                ResourceServiceKind::ComputeInstances,
                &ctx,
                &fields
            ),
            None
        );
    }

    /// The declaration's `dangerous` has to survive the hop into the `Offer`,
    /// because the `Offer` is what crosses into the reducer and the modal's
    /// typed acknowledgement is chosen from *its* flag and not from the plugin
    /// set. Both ends of the hop are pinned elsewhere: `config::plugins` loads
    /// the key
    /// (`config_plugins_test::a_plugin_declared_dangerous_loads_as_dangerous`)
    /// and the confirmation demands the count for an `Offer` that carries it
    /// (`state::resources::tests::a_dangerous_plugin_asks_for_a_typed_acknowledgement_and_an_ordinary_one_does_not`).
    /// But every test on the far end builds its `Offer` by hand, so this is the
    /// only place the two meet. A `resolve` that dropped the flag would
    /// downgrade a delete to a bare `Enter` with the suite green.
    #[test]
    fn a_plugin_declared_dangerous_is_offered_as_dangerous() {
        let plugins = plugins_from(&[Entry {
            scope: ResourceServiceKind::CloudSql,
            id: "delete",
            label: "Delete {name}",
            args: &["sql", "instances", "delete", "{name}"],
            key: None,
            dangerous: true,
        }]);
        let fields = vec![(JumpField::Name, "prod-db".to_string())];

        let offers = applicable(
            &plugins,
            ResourceServiceKind::CloudSql,
            &context("my-proj"),
            &fields,
        );

        assert_eq!(offers.len(), 1);
        assert!(
            offers[0].dangerous,
            "a plugin declared dangerous must be offered as dangerous"
        );
    }

    /// The other half: nothing makes an ordinary plugin dangerous on the way
    /// through, which would cost the operator a typed acknowledgement for a
    /// plugin that only describes.
    #[test]
    fn a_plugin_that_declares_nothing_is_offered_as_ordinary() {
        let plugins = plugins_for(
            ResourceServiceKind::ComputeInstances,
            &[("describe", &["compute", "instances", "describe", "{name}"])],
        );
        let fields = vec![(JumpField::Name, "web-01".to_string())];

        let offers = applicable(
            &plugins,
            ResourceServiceKind::ComputeInstances,
            &context("my-proj"),
            &fields,
        );

        assert_eq!(offers.len(), 1);
        assert!(!offers[0].dangerous);
    }

    /// Every service can carry a plugin, and every service's plugins resolve
    /// through the same field table `Template::parse` validated them against.
    ///
    /// Walks `ResourceServiceKind::ALL` rather than naming services: naming
    /// them is exactly how a new service ships with a validated plugin that
    /// then resolves against nothing.
    #[test]
    fn every_service_resolves_a_name_only_plugin_against_its_own_row() {
        for kind in ResourceServiceKind::ALL {
            let plugins = plugins_for(kind, &[("describe", &["describe", "{name}"])]);
            let fields = vec![(JumpField::Name, "row-01".to_string())];

            let offers = applicable(&plugins, kind, &context("my-proj"), &fields);

            assert_eq!(
                offers.len(),
                1,
                "{kind:?} offered no plugin for a row that has a name"
            );
            assert_eq!(
                ready_argv(&offers[0]),
                vec!["--configuration", "my-proj", "describe", "row-01"],
                "{kind:?}"
            );
        }
    }

    /// The keyed press resolves the plugin bound to *that* key, and the argv is
    /// asserted beside the id so a wrong `PluginId` cannot read as a cosmetic
    /// off-by-one — the same reason
    /// `a_plugin_is_numbered_within_the_whole_set_not_within_its_scope` asserts
    /// both.
    #[test]
    fn a_bound_key_resolves_that_plugins_offer() {
        let plugins = plugins_from(&[
            Entry {
                scope: ResourceServiceKind::ComputeInstances,
                id: "describe",
                label: "Describe {name}",
                args: &["compute", "instances", "describe", "{name}"],
                key: None,
                dangerous: false,
            },
            Entry {
                scope: ResourceServiceKind::ComputeInstances,
                id: "delete",
                label: "Delete {name}",
                args: &["compute", "instances", "delete", "{name}"],
                key: None,
                dangerous: true,
            },
        ]);
        let fields = vec![(JumpField::Name, "web-01".to_string())];

        // `plugins_from` keys entries `a`, `b`, … in file order, so `b` is
        // the delete.
        let resolved = keyed(
            &plugins,
            ResourceServiceKind::ComputeInstances,
            &context("my-proj"),
            &fields,
            'b',
        );

        let Keyed::Offer(offer) = resolved else {
            panic!("a bound, resolvable key must offer: {resolved:?}");
        };
        assert_eq!(offer.id, PluginId(1));
        assert_eq!(
            ready_argv(&offer),
            vec![
                "--configuration",
                "my-proj",
                "compute",
                "instances",
                "delete",
                "web-01"
            ]
        );
        assert!(offer.dangerous, "the declaration's flag must survive");
    }

    /// A key nobody declared in this scope is a different answer from a key whose
    /// plugin refused: the operator needs to know whether to fix their file or
    /// their selection.
    #[test]
    fn an_undeclared_key_is_unbound() {
        let plugins = plugins_for(
            ResourceServiceKind::ComputeInstances,
            &[("describe", &["compute", "instances", "describe", "{name}"])],
        );
        let fields = vec![(JumpField::Name, "web-01".to_string())];

        assert_eq!(
            keyed(
                &plugins,
                ResourceServiceKind::ComputeInstances,
                &context("my-proj"),
                &fields,
                'z',
            ),
            Keyed::Unbound
        );
    }

    /// A plugin scoped elsewhere does not answer this scope's key. `plugins_from`
    /// keys by position in the whole file, so the Cloud SQL entry owns `a` and
    /// the Compute one owns `b`: pressing `a` with an instance selected must be
    /// `Unbound`, because a `keyed` that did not filter by scope would run the
    /// Cloud SQL *delete* against the selected instance. The second half pins
    /// that the filter refuses only the wrong scope, not every key.
    #[test]
    fn a_key_declared_in_another_scope_is_unbound_here() {
        let plugins = plugins_from(&[
            Entry {
                scope: ResourceServiceKind::CloudSql,
                id: "cloudsql-delete",
                label: "Delete {name}",
                args: &["sql", "instances", "delete", "{name}"],
                key: None,
                dangerous: true,
            },
            Entry {
                scope: ResourceServiceKind::ComputeInstances,
                id: "compute-describe",
                label: "Describe {name}",
                args: &["compute", "instances", "describe", "{name}"],
                key: None,
                dangerous: false,
            },
        ]);
        let fields = vec![(JumpField::Name, "web-01".to_string())];
        let ctx = context("my-proj");

        // `a` is the Cloud SQL delete's key, declared in another scope.
        assert_eq!(
            keyed(
                &plugins,
                ResourceServiceKind::ComputeInstances,
                &ctx,
                &fields,
                'a',
            ),
            Keyed::Unbound,
            "another scope's key must not answer, let alone with its delete"
        );

        let Keyed::Offer(offer) = keyed(
            &plugins,
            ResourceServiceKind::ComputeInstances,
            &ctx,
            &fields,
            'b',
        ) else {
            panic!("the compute plugin is bound to `b` in its own scope");
        };
        assert_eq!(offer.id, PluginId(1));
        assert_eq!(
            ready_argv(&offer),
            vec![
                "--configuration",
                "my-proj",
                "compute",
                "instances",
                "describe",
                "web-01"
            ]
        );
    }

    /// Two scopes sharing one character: each answers with *its own* plugin.
    ///
    /// `key` uniqueness is per scope, so this file is legal and a real one will
    /// look like it — an operator who likes `d` for "describe" writes it on every
    /// scope. The scope test therefore has to be part of the `find` predicate. A
    /// `keyed` that found by key first and rejected the scope afterwards passes
    /// every other test in this module and answers `Unbound` here for whichever
    /// scope lost the race, telling the operator their key was never declared
    /// while the plugin sits in the file, permanently unreachable from it.
    ///
    /// The keys are spelled rather than derived because the derivation gives
    /// every entry a distinct character and so cannot build this file at all.
    /// Each argv is asserted beside its `PluginId` so answering with the wrong
    /// plugin cannot read as a cosmetic off-by-one.
    #[test]
    fn one_character_keyed_in_two_scopes_answers_with_each_scopes_own_plugin() {
        let plugins = plugins_from(&[
            Entry {
                scope: ResourceServiceKind::ComputeInstances,
                id: "compute-describe",
                label: "Describe {name}",
                args: &["compute", "instances", "describe", "{name}"],
                key: Some('d'),
                dangerous: false,
            },
            Entry {
                scope: ResourceServiceKind::CloudSql,
                id: "cloudsql-describe",
                label: "Describe {name}",
                args: &["sql", "instances", "describe", "{name}"],
                key: Some('d'),
                dangerous: false,
            },
        ]);
        let fields = vec![(JumpField::Name, "web-01".to_string())];
        let ctx = context("my-proj");

        let Keyed::Offer(compute) = keyed(
            &plugins,
            ResourceServiceKind::ComputeInstances,
            &ctx,
            &fields,
            'd',
        ) else {
            panic!("the compute plugin declares `d` in the compute scope");
        };
        assert_eq!(compute.id, PluginId(0));
        assert_eq!(
            ready_argv(&compute),
            vec![
                "--configuration",
                "my-proj",
                "compute",
                "instances",
                "describe",
                "web-01"
            ]
        );

        // The entry `find` reaches second, and the one a find-then-reject
        // implementation would answer `Unbound` for.
        let Keyed::Offer(sql) = keyed(&plugins, ResourceServiceKind::CloudSql, &ctx, &fields, 'd')
        else {
            panic!("the Cloud SQL plugin declares `d` in the Cloud SQL scope");
        };
        assert_eq!(sql.id, PluginId(1));
        assert_eq!(
            ready_argv(&sql),
            vec![
                "--configuration",
                "my-proj",
                "sql",
                "instances",
                "describe",
                "web-01"
            ]
        );
    }

    /// The refusal names the plugin the operator pressed, and reuses the wording
    /// `diagnostics` composes — one vocabulary, so the palette's sentence and the
    /// keyed press's sentence cannot drift.
    #[test]
    fn a_bound_key_whose_plugin_cannot_resolve_refuses_in_the_diagnostic_wording() {
        let plugins = plugins_for(
            ResourceServiceKind::ComputeInstances,
            &[(
                "describe",
                &[
                    "compute",
                    "instances",
                    "describe",
                    "{name}",
                    "--zone",
                    "{zone}",
                ],
            )],
        );
        let fields = vec![(JumpField::Name, "web-01".to_string())]; // no zone value

        assert_eq!(
            keyed(
                &plugins,
                ResourceServiceKind::ComputeInstances,
                &context("my-proj"),
                &fields,
                'a',
            ),
            Keyed::Refused(
                "Plugin \"describe\" needs zone, which this GCE instance has no value for"
                    .to_string()
            )
        );
    }

    /// The other half of the vocabulary claim: a *resolvable* sibling silences
    /// `diagnostics` but must not silence the keyed refusal. This is the case
    /// reusing `diagnostics` for the keyed press would have got wrong — it returns
    /// `None` as soon as any sibling resolves.
    #[test]
    fn a_resolvable_sibling_does_not_silence_the_keyed_refusal() {
        let plugins = plugins_from(&[
            Entry {
                scope: ResourceServiceKind::ComputeInstances,
                id: "broken",
                label: "Describe {name}",
                args: &[
                    "compute",
                    "instances",
                    "describe",
                    "{name}",
                    "--zone",
                    "{zone}",
                ],
                key: None,
                dangerous: false,
            },
            Entry {
                scope: ResourceServiceKind::ComputeInstances,
                id: "fine",
                label: "Describe {name}",
                args: &["compute", "instances", "describe", "{name}"],
                key: None,
                dangerous: false,
            },
        ]);
        let fields = vec![(JumpField::Name, "web-01".to_string())];
        let ctx = context("my-proj");

        assert_eq!(
            diagnostics(
                &plugins,
                ResourceServiceKind::ComputeInstances,
                &ctx,
                &fields
            ),
            None,
            "a resolvable sibling silences the scope-level sentence"
        );
        assert_eq!(
            keyed(
                &plugins,
                ResourceServiceKind::ComputeInstances,
                &ctx,
                &fields,
                'a',
            ),
            Keyed::Refused(
                "Plugin \"broken\" needs zone, which this GCE instance has no value for"
                    .to_string()
            ),
            "but the plugin the operator pressed still has to say why"
        );
    }

    /// `,` reads this to decide whether to arm at all, and it is a question about
    /// the file rather than about the row: a scope with a plugin that no row can
    /// resolve still arms, and the press then explains.
    #[test]
    fn a_scope_with_a_plugin_is_reported_as_having_one_and_others_are_not() {
        let plugins = plugins_for(
            ResourceServiceKind::ComputeInstances,
            &[("describe", &["compute", "instances", "describe", "{name}"])],
        );

        assert!(any_scoped(&plugins, ResourceServiceKind::ComputeInstances));
        assert!(!any_scoped(&plugins, ResourceServiceKind::CloudSql));
        assert!(!any_scoped(
            &Plugins::empty(),
            ResourceServiceKind::ComputeInstances
        ));
    }

    /// The hint the status line shows while the prefix is armed, in file order —
    /// the order the palette lists plugins in, so the two agree.
    #[test]
    fn bound_keys_are_this_scopes_keys_in_file_order() {
        let plugins = plugins_from(&[
            Entry {
                scope: ResourceServiceKind::CloudSql,
                id: "sql",
                label: "Describe {name}",
                args: &["sql", "instances", "describe", "{name}"],
                key: None,
                dangerous: false,
            },
            Entry {
                scope: ResourceServiceKind::ComputeInstances,
                id: "describe",
                label: "Describe {name}",
                args: &["compute", "instances", "describe", "{name}"],
                key: None,
                dangerous: false,
            },
            Entry {
                scope: ResourceServiceKind::ComputeInstances,
                id: "delete",
                label: "Delete {name}",
                args: &["compute", "instances", "delete", "{name}"],
                key: None,
                dangerous: true,
            },
        ]);

        assert_eq!(
            bound_keys(&plugins, ResourceServiceKind::ComputeInstances),
            vec!['b', 'c']
        );
    }

    /// A `dropdown` is promptable too, and it is the half of that answer no
    /// `string` fixture can pin: the operator picks from the declared choices
    /// rather than typing, so nothing about the prompt has to parse.
    #[test]
    fn a_plugin_naming_a_dropdown_input_offers_needs_values() {
        let plugins = plugins_from_toml(
            r#"version = 1

[[plugins]]
id = "resize"
scope = "compute"
key = "r"
label = "Resize {name}"
args = ["compute", "instances", "resize", "{name}", "--tier", "{tier}"]

[[plugins.inputs]]
name = "tier"
type = "dropdown"
choices = ["standard", "premium"]
"#,
        );
        let fields = vec![(JumpField::Name, "web-01".to_string())];

        let offers = applicable(
            &plugins,
            ResourceServiceKind::ComputeInstances,
            &context("my-proj"),
            &fields,
        );

        assert_eq!(offers.len(), 1, "a dropdown must not withhold the plugin");
        assert_eq!(
            offers[0].command,
            Command::NeedsValues {
                inputs: vec![PluginInput {
                    name: "tier".to_string(),
                    kind: PluginInputKind::Dropdown {
                        choices: vec!["standard".to_string(), "premium".to_string()],
                    },
                    default: None,
                }],
            },
            "the choices must survive into the offer, or the prompt has nothing to show"
        );
    }

    /// A plugin naming a `string` input is no longer withheld: it resolves to
    /// the state that says what is still missing, carrying the inputs the
    /// prompt will ask for. Withholding it was the previous slice's honest
    /// answer while nothing could ask; keeping it would mean the prompt slice
    /// has nothing to open.
    #[test]
    fn a_plugin_naming_a_string_input_offers_needs_values() {
        let plugins = plugins_from_toml(
            r#"version = 1

[[plugins]]
id = "resize"
scope = "compute"
key = "r"
label = "Resize {name}"
args = ["compute", "instances", "resize", "{name}", "--size", "{disk_size}"]

[[plugins.inputs]]
name = "disk_size"
type = "string"
default = "100"
"#,
        );
        let fields = vec![(JumpField::Name, "web-01".to_string())];

        let offers = applicable(
            &plugins,
            ResourceServiceKind::ComputeInstances,
            &context("my-proj"),
            &fields,
        );

        assert_eq!(offers.len(), 1, "an input no longer withholds the plugin");
        match &offers[0].command {
            Command::NeedsValues { inputs } => {
                assert_eq!(inputs.len(), 1);
                assert_eq!(inputs[0].name, "disk_size");
            }
            Command::Ready(argv) => panic!("expected NeedsValues, got {argv:?}"),
        }
    }

    /// A label naming an input costs the caption, not the offer — the same
    /// trade a label naming an absent row field makes. The fallback is not
    /// permanent any more: the confirmation heading is drawn after the prompt
    /// collects the value, so the operator's own caption comes back there.
    #[test]
    fn a_plugin_naming_an_input_in_its_label_falls_back_to_its_id() {
        let plugins = plugins_from_toml(
            r#"version = 1

[[plugins]]
id = "resize"
scope = "compute"
key = "r"
label = "Resize {name} to {disk_size}GB"
args = ["compute", "instances", "resize", "{name}", "--size", "{disk_size}"]

[[plugins.inputs]]
name = "disk_size"
type = "string"
"#,
        );
        let fields = vec![(JumpField::Name, "web-01".to_string())];

        let offers = applicable(
            &plugins,
            ResourceServiceKind::ComputeInstances,
            &context("my-proj"),
            &fields,
        );

        assert_eq!(offers.len(), 1, "a label input no longer costs the offer");
        assert_eq!(offers[0].label, "resize", "the palette shows the id");
    }

    /// The inverse of the refusal this test used to pin. There used to be a
    /// plugin mixing a promptable `tier` with a `bool` `background` was
    /// withheld whole, and the assertion here was that the refusal sentence
    /// named `background` rather than `tier`. There is no refusal left to
    /// word, so what it pins now is that the mixture is offered and that
    /// *both* inputs are collected — the property the old withholding rule
    /// existed to protect, since asking for one of two values and then
    /// refusing would have had the operator type for nothing.
    #[test]
    fn a_plugin_mixing_a_bool_with_another_kind_is_offered_and_collects_both() {
        let plugins = plugins_from_toml(
            r#"version = 1

[[plugins]]
id = "resize"
scope = "compute"
key = "r"
label = "Resize {name}"
args = ["compute", "instances", "resize", "{name}", "--tier", "{tier}", "--async", "{background}"]

[[plugins.inputs]]
name = "tier"
type = "string"

[[plugins.inputs]]
name = "background"
type = "bool"
"#,
        );
        let fields = vec![(JumpField::Name, "web-01".to_string())];
        let ctx = context("my-proj");

        let offers = applicable(
            &plugins,
            ResourceServiceKind::ComputeInstances,
            &ctx,
            &fields,
        );
        assert_eq!(offers.len(), 1, "a bool no longer costs the offer");
        let collected: Vec<&str> = match &offers[0].command {
            Command::NeedsValues { inputs } => {
                inputs.iter().map(|input| input.name.as_str()).collect()
            }
            other => panic!("expected a prompt, got {other:?}"),
        };
        assert_eq!(
            collected,
            vec!["tier", "background"],
            "both inputs are collected, in the order they are declared"
        );
        assert_eq!(
            diagnostics(
                &plugins,
                ResourceServiceKind::ComputeInstances,
                &ctx,
                &fields
            ),
            None,
            "and nothing is refused, so there is no sentence to compose"
        );
    }

    /// The collected value reaches the argv through the same walk a row field
    /// does. Without this the prompt could collect a value that never lands in
    /// the command the operator confirms.
    #[test]
    fn plugin_argv_substitutes_a_collected_value() {
        let plugins = plugins_from_toml(
            r#"version = 1

[[plugins]]
id = "resize"
scope = "compute"
key = "r"
label = "Resize {name}"
args = ["compute", "instances", "resize", "{name}", "--size", "{disk_size}"]

[[plugins.inputs]]
name = "disk_size"
type = "string"
"#,
        );
        let fields = vec![(JumpField::Name, "web-01".to_string())];
        let values = vec![("disk_size".to_string(), "200".to_string())];

        let argv = plugin_argv(&plugins.all()[0], &context("my-proj"), &fields, &values)
            .expect("collected values complete the argv");

        assert_eq!(
            argv,
            vec![
                "--configuration",
                "my-proj",
                "compute",
                "instances",
                "resize",
                "web-01",
                "--size",
                "200",
            ]
        );
    }

    /// One template can name two inputs, so the fields to prompt for are read
    /// with `names_input` rather than off the first input each template names.
    /// Prompting for only the first would compose an argv the operator was
    /// never asked to complete.
    #[test]
    fn prompted_inputs_keeps_both_inputs_of_one_template() {
        let plugins = plugins_from_toml(
            r#"version = 1

[[plugins]]
id = "resize"
scope = "compute"
key = "r"
label = "Resize {name}"
args = ["compute", "instances", "resize", "{name}", "--size={disk_size},tier={tier}"]

[[plugins.inputs]]
name = "disk_size"
type = "string"

[[plugins.inputs]]
name = "tier"
type = "string"
"#,
        );

        let inputs = prompted_inputs(&plugins.all()[0]);

        assert_eq!(
            inputs.len(),
            2,
            "one template can name two inputs; prompting for only the first would \
             compose an argv the operator never completed"
        );
    }

    /// An input the plugin declares but no template names withholds nothing,
    /// so it is not a prompt field either — asking for it would demand a value
    /// that reaches no argv.
    #[test]
    fn prompted_inputs_omits_a_declaration_no_template_names() {
        let plugins = plugins_from_toml(
            r#"version = 1

[[plugins]]
id = "resize"
scope = "compute"
key = "r"
label = "Resize {name}"
args = ["compute", "instances", "resize", "{name}", "--size", "{disk_size}"]

[[plugins.inputs]]
name = "disk_size"
type = "string"

[[plugins.inputs]]
name = "unused"
type = "string"
"#,
        );

        let inputs = prompted_inputs(&plugins.all()[0]);

        assert_eq!(
            inputs.len(),
            1,
            "an unused declaration is not a prompt field"
        );
        assert_eq!(inputs[0].name, "disk_size");
    }

    /// A declaration that stops short of a table entry's verb never named that
    /// verb, so nuvem has nothing to vouch for. Matching pairs elementwise and
    /// elementwise comparison runs out at the shorter side, so without the
    /// length guard a two-element declaration would take a three-element
    /// entry's word for what it does.
    #[test]
    fn a_path_that_stops_short_of_the_verb_is_unproven() {
        let plugins = plugins_from_toml(
            "version = 1\n\n\
             [[plugins]]\n\
             id = \"short\"\n\
             scope = \"compute\"\n\
             key = \"h\"\n\
             label = \"Short\"\n\
             args = [\"compute\", \"instances\"]\n",
        );
        assert_eq!(declared_effect(&plugins.all()[0]), DeclaredEffect::Unproven);
    }

    /// The unsound case that rules out a bare verb list. `list` here is an
    /// instance *name*, not a verb: any rule that scans the argv for a reading
    /// verb, or that walks the leading literal run and takes its last element,
    /// reads this delete as a read.
    #[test]
    fn a_reading_verb_in_a_positional_does_not_make_a_delete_read() {
        let plugins = plugins_from_toml(
            "version = 1\n\n\
             [[plugins]]\n\
             id = \"wipe\"\n\
             scope = \"compute\"\n\
             key = \"w\"\n\
             label = \"Wipe\"\n\
             args = [\"compute\", \"instances\", \"delete\", \"list\"]\n",
        );
        assert_eq!(declared_effect(&plugins.all()[0]), DeclaredEffect::Unproven);
    }

    /// A verb the operator supplies is runtime data, and they can type
    /// it. It must classify `Unproven` — refused — rather than matching on the
    /// literal elements around it.
    #[test]
    fn a_templated_verb_is_unknown_rather_than_read_from_its_neighbours() {
        let plugins = plugins_from_toml(
            "version = 1\n\n\
             [[plugins]]\n\
             id = \"sneaky\"\n\
             scope = \"compute\"\n\
             key = \"s\"\n\
             label = \"Do {verb}\"\n\
             args = [\"compute\", \"instances\", \"{verb}\", \"{name}\"]\n\
             \n\
             [[plugins.inputs]]\n\
             name = \"verb\"\n\
             type = \"string\"\n",
        );
        assert_eq!(declared_effect(&plugins.all()[0]), DeclaredEffect::Unproven);
    }

    #[test]
    fn a_declared_reading_command_reads_whatever_follows_it() {
        let plugins = plugins_from_toml(
            "version = 1\n\n\
             [[plugins]]\n\
             id = \"show\"\n\
             scope = \"compute\"\n\
             key = \"d\"\n\
             label = \"Describe {name}\"\n\
             args = [\"compute\", \"instances\", \"describe\", \"{name}\", \"--zone\", \"{zone}\"]\n",
        );
        assert_eq!(
            declared_effect(&plugins.all()[0]),
            DeclaredEffect::Reads,
            "substituted values land after the matched prefix, where they are \
             positionals and flag values"
        );
    }

    /// A reading command nuvem does not name is refused like any other. The
    /// table is an allowlist, so absence is not evidence of anything except
    /// that nuvem never vouched for it.
    #[test]
    fn a_reading_command_outside_the_table_is_still_unproven() {
        let plugins = plugins_from_toml(
            "version = 1\n\n\
             [[plugins]]\n\
             id = \"beta\"\n\
             scope = \"compute\"\n\
             key = \"b\"\n\
             label = \"Beta describe\"\n\
             args = [\"beta\", \"compute\", \"instances\", \"describe\", \"{name}\"]\n",
        );
        assert_eq!(declared_effect(&plugins.all()[0]), DeclaredEffect::Unproven);
    }

    /// A global flag before the command path defeats the anchor, and must fail
    /// closed rather than be skipped over: skipping leading flags is how a
    /// declaration whose real command path starts elsewhere would be matched
    /// against this table at all.
    ///
    /// What the anchor buys is that the *matched prefix* names the command —
    /// nothing before element 0 can move it. It buys nothing about the flags
    /// that follow: `["compute", "instances", "list", "--project", "prod"]`
    /// matches and classifies as `Reads`, so `--read-only` does not confine a
    /// plugin to the project nuvem is showing. It confines it to reading, which
    /// is the whole of the flag's promise.
    #[test]
    fn a_leading_flag_defeats_the_anchor_and_is_unproven() {
        let plugins = plugins_from_toml(
            "version = 1\n\n\
             [[plugins]]\n\
             id = \"cross\"\n\
             scope = \"compute\"\n\
             key = \"x\"\n\
             label = \"Cross-project\"\n\
             args = [\"--project\", \"other\", \"compute\", \"instances\", \"list\"]\n",
        );
        assert_eq!(declared_effect(&plugins.all()[0]), DeclaredEffect::Unproven);
    }

    /// The invariant that makes "reads whatever follows it" true: an entry must
    /// name a command, never a group. A group entry would vouch for every verb
    /// beneath it, so `["compute", "instances"]` would classify
    /// `["compute", "instances", "delete", "{name}"]` as reading.
    ///
    /// The final element being `describe` or `list` is a proxy for "terminal",
    /// not the property itself — deciding terminality needs a gcloud grammar
    /// nuvem does not have and would have to keep in step with a moving CLI.
    /// The proxy is the right one because it is the shape the hazard takes: a
    /// gcloud group is a noun (`instances`, `topics`), never a reading verb, so
    /// an entry that ends in one cannot be a group, and an entry that ends
    /// anywhere else has to be argued for here first.
    #[test]
    fn every_table_entry_ends_in_a_reading_verb() {
        for path in READING_COMMANDS {
            let verb = path.last().expect("a table entry names a command");
            assert!(
                matches!(*verb, "describe" | "list"),
                "table entry {path:?} ends in {verb:?}: a group entry would make \
                 every verb beneath it, deletes included, classify as reading"
            );
        }
    }

    /// The table is a maintenance surface: it goes stale when gcloud grows a
    /// command and nothing announces it. Walking it here makes every entry a
    /// deliberate one and makes a malformed entry fail loudly.
    #[test]
    fn every_table_entry_is_a_nonempty_path_that_classifies_as_reading() {
        for path in READING_COMMANDS {
            assert!(!path.is_empty(), "an empty path would match every plugin");
            let args = path
                .iter()
                .map(|element| format!("{element:?}"))
                .collect::<Vec<_>>()
                .join(", ");
            let plugins = plugins_from_toml(&format!(
                "version = 1\n\n\
                 [[plugins]]\n\
                 id = \"probe\"\n\
                 scope = \"compute\"\n\
                 key = \"p\"\n\
                 label = \"Probe\"\n\
                 args = [{args}]\n"
            ));
            assert_eq!(
                declared_effect(&plugins.all()[0]),
                DeclaredEffect::Reads,
                "table entry {path:?} must classify as reading"
            );
        }
    }
}
