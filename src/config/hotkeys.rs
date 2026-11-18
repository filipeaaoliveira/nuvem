//! User-rebindable normal-mode hotkeys, layered over the #25 binding catalog.
//!
//! Only a small, fixed set of normal-mode global/navigation actions are
//! rebindable (see [`RebindableAction`]). A [`KeyOverrides`] table maps each
//! rebindable action to an *effective* [`KeyTrigger`] (the user override, or
//! the built-in default). Keymap dispatch and the footer/help catalog both
//! resolve rebindable keys through this effective table, so a rebound key
//! stays consistent across input, footer, and help.

use std::collections::BTreeMap;

use crate::app::projection::binding::KeyTrigger;
use crate::app::{
    Action, CommandAction, Effect, FinOpsAction, LoggingAction, NavAction, ResourcesAction,
    SystemAction, Workspace, WorkspaceAction,
};
use crate::input::Context;

/// A normal-mode global/navigation action whose key can be rebound from the
/// `[hotkeys]` config table. Modal / text-entry keys are never rebindable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum RebindableAction {
    Home,
    Configurations,
    Resources,
    Refresh,
    Help,
    Quit,
    CommandPalette,
}

impl RebindableAction {
    /// Every rebindable action, in a stable order (used for deterministic
    /// reverse lookup and validation).
    pub const ALL: [RebindableAction; 7] = [
        RebindableAction::Home,
        RebindableAction::Configurations,
        RebindableAction::Resources,
        RebindableAction::Refresh,
        RebindableAction::Help,
        RebindableAction::Quit,
        RebindableAction::CommandPalette,
    ];

    /// The `[hotkeys]` config-table key that names this action.
    pub fn config_key(&self) -> &'static str {
        match self {
            RebindableAction::Home => "home",
            RebindableAction::Configurations => "configurations",
            RebindableAction::Resources => "resources",
            RebindableAction::Refresh => "refresh",
            RebindableAction::Help => "help",
            RebindableAction::Quit => "quit",
            RebindableAction::CommandPalette => "command_palette",
        }
    }

    /// Parse a `[hotkeys]` config-table key into its action.
    pub fn from_config_key(key: &str) -> Option<Self> {
        RebindableAction::ALL
            .into_iter()
            .find(|action| action.config_key() == key)
    }

    /// The built-in default trigger for this action.
    pub fn default_trigger(&self) -> KeyTrigger {
        match self {
            RebindableAction::Home => KeyTrigger::Char('h'),
            RebindableAction::Configurations => KeyTrigger::Char('c'),
            RebindableAction::Resources => KeyTrigger::Char('g'),
            RebindableAction::Refresh => KeyTrigger::Char('r'),
            RebindableAction::Help => KeyTrigger::Char('?'),
            RebindableAction::Quit => KeyTrigger::Char('q'),
            RebindableAction::CommandPalette => KeyTrigger::Char(':'),
        }
    }

    /// The concrete [`Action`] this rebindable action dispatches, given the
    /// current input [`Context`]. `Refresh` and `Quit` are context-dependent
    /// (mirroring the previous hardcoded keymap behavior).
    pub fn to_app_action(&self, context: &Context) -> Action {
        match self {
            RebindableAction::Home => Action::Workspace(WorkspaceAction::Home),
            RebindableAction::Configurations => Action::Workspace(WorkspaceAction::Configurations),
            RebindableAction::Resources => Action::Workspace(WorkspaceAction::Resources),
            RebindableAction::Help => Action::System(SystemAction::ToggleHelp),
            RebindableAction::CommandPalette => Action::Command(CommandAction::Open),
            RebindableAction::Refresh => match context.workspace {
                Workspace::Configurations => Action::Effect(Effect::LoadConfigurations),
                Workspace::Home => Action::Workspace(WorkspaceAction::Home),
                Workspace::Resources => Action::Resources(ResourcesAction::Refresh),
                Workspace::Logging => Action::Logging(LoggingAction::Refresh),
                // Tunnel rows are pushed by the supervisor as children change
                // state, so there is nothing to re-fetch. Re-selecting the
                // workspace keeps the key honest (it stays reachable and does
                // not fall through to another workspace's refresh) while
                // changing nothing.
                Workspace::Tunnels => Action::Workspace(WorkspaceAction::Tunnels),
                Workspace::FinOps => Action::FinOps(FinOpsAction::Refresh),
            },
            RebindableAction::Quit => {
                if context.workspace == Workspace::Resources && context.resources_detail_open {
                    Action::Navigation(NavAction::Back)
                } else if context.workspace == Workspace::Logging && context.logging_detail_open {
                    Action::Logging(LoggingAction::CloseDetail)
                } else {
                    Action::QuitRequested
                }
            }
        }
    }
}

/// The set of user hotkey overrides: `RebindableAction -> KeyTrigger`.
/// Absent actions fall back to their built-in default trigger.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct KeyOverrides {
    map: BTreeMap<RebindableAction, KeyTrigger>,
}

impl KeyOverrides {
    /// An override table with no overrides (all actions use their defaults).
    pub fn empty() -> Self {
        Self::default()
    }

    /// Whether any override is set.
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    /// The effective trigger for an action: the override if present, else the
    /// built-in default.
    pub fn effective_trigger(&self, action: RebindableAction) -> KeyTrigger {
        self.map
            .get(&action)
            .copied()
            .unwrap_or_else(|| action.default_trigger())
    }

    /// Reverse lookup over the *effective* table (defaults with overrides
    /// applied): the action a trigger currently dispatches, if any.
    pub fn action_for(&self, trigger: KeyTrigger) -> Option<RebindableAction> {
        RebindableAction::ALL
            .into_iter()
            .find(|action| self.effective_trigger(*action) == trigger)
    }

    /// Parse and validate the raw `[hotkeys]` config table. On *any* error
    /// (unknown action, bad key spec, reserved key, collision, or an
    /// unreachable required action) the whole set is rejected: `Err` collects
    /// every message and the caller falls back to defaults with a warning.
    pub fn from_file(raw: &BTreeMap<String, String>) -> Result<KeyOverrides, Vec<String>> {
        let mut errors = Vec::new();
        let mut map = BTreeMap::new();

        for (key, spec) in raw {
            let Some(action) = RebindableAction::from_config_key(key) else {
                errors.push(format!("hotkeys.{key}: unknown rebindable action"));
                continue;
            };
            match parse_key_spec(spec) {
                Ok(trigger) => {
                    map.insert(action, trigger);
                }
                Err(message) => errors.push(format!("hotkeys.{key}: {message}")),
            }
        }

        let effective = effective_table(&map);
        if let Err(mut validation_errors) = validate(&effective) {
            errors.append(&mut validation_errors);
        }

        if errors.is_empty() {
            Ok(KeyOverrides { map })
        } else {
            Err(errors)
        }
    }
}

/// The full effective table: every rebindable action mapped to its override
/// (if any) or built-in default.
fn effective_table(
    overrides: &BTreeMap<RebindableAction, KeyTrigger>,
) -> BTreeMap<RebindableAction, KeyTrigger> {
    RebindableAction::ALL
        .into_iter()
        .map(|action| {
            let trigger = overrides
                .get(&action)
                .copied()
                .unwrap_or_else(|| action.default_trigger());
            (action, trigger)
        })
        .collect()
}

/// Parse a single key spec into a [`KeyTrigger`].
///
/// - `"H"` -> `Char('H')` (case preserved), `"ctrl-r"` -> `CtrlChar('r')`.
/// - Named keys (`enter`, `esc`, `tab`, `backspace`, ...) parse to their
///   trigger (so validation can flag them as reserved).
/// - Multi-character or otherwise unrecognized specs are rejected.
pub fn parse_key_spec(spec: &str) -> Result<KeyTrigger, String> {
    let trimmed = spec.trim();
    if trimmed.is_empty() {
        return Err("empty key spec".to_string());
    }

    match trimmed.to_ascii_lowercase().as_str() {
        "enter" | "return" => return Ok(KeyTrigger::Enter),
        "esc" | "escape" => return Ok(KeyTrigger::Esc),
        "tab" => return Ok(KeyTrigger::Tab),
        "backtab" | "shift-tab" => return Ok(KeyTrigger::BackTab),
        "backspace" => return Ok(KeyTrigger::Backspace),
        _ => {}
    }

    if let Some(rest) = trimmed.to_ascii_lowercase().strip_prefix("ctrl-") {
        let mut chars = rest.chars();
        return match (chars.next(), chars.next()) {
            (Some(c), None) => Ok(KeyTrigger::CtrlChar(c)),
            _ => Err(format!("invalid ctrl key spec {spec:?}")),
        };
    }

    let mut chars = trimmed.chars();
    match (chars.next(), chars.next()) {
        (Some(c), None) => Ok(KeyTrigger::Char(c)),
        _ => Err(format!("unknown key spec {spec:?}")),
    }
}

/// Why a trigger may never be used as an override target, or `None` if it is
/// free to rebind.
///
/// Three distinct mechanisms, and the reasons must keep them straight:
///
/// - `Esc` / `Enter` / `Tab` / `BackTab` / `Backspace`: `override_trigger`
///   (`input/keymap.rs`) only ever yields a trigger for `KeyCode::Char`, so an
///   override onto a named key could never be looked up at all. The binding
///   would be inert, and the key keeps its universal meaning.
/// - `Ctrl-C` / `Ctrl-X`: `action_for_key` returns their action at the very
///   top, before any mode or override lookup, so a binding onto either could
///   never fire. `Ctrl-X` joined `Ctrl-C` up there later; it is reserved
///   for the same reason the named keys are, an inert binding, rather than for
///   the theft below.
/// - `Ctrl-S` / `Ctrl-T`: the opposite. `normal_key` resolves the override
///   table *before* the hardcoded `Ctrl-T` arm and before per-workspace
///   dispatch (where `logging_list_key` handles `Ctrl-S`), and
///   `override_trigger` maps `Ctrl-<char>` to `CtrlChar` — the same exact
///   chord those arms match, so the lookup lands on it first. A user binding
///   therefore *wins* and silently steals the app's own key. (The match is
///   exact on both sides: `Ctrl-Alt-S` and `Ctrl-Shift-S` reach neither the
///   override table nor the app's arm.)
///
/// Each reason must be supportable from the keymap; do not add a trigger here
/// without one.
fn reserved_reason(trigger: KeyTrigger) -> Option<&'static str> {
    match trigger {
        KeyTrigger::Esc => Some(
            "only single-character keys can be looked up in the override table, so a binding \
             here could never fire; Esc stays the universal modal and filter cancel",
        ),
        KeyTrigger::Enter => Some(
            "only single-character keys can be looked up in the override table, so a binding \
             here could never fire; Enter stays the universal modal and filter submit",
        ),
        KeyTrigger::Tab => Some(
            "only single-character keys can be looked up in the override table, so a binding \
             here could never fire; Tab stays the focus move in forms and the resource sidebar",
        ),
        KeyTrigger::BackTab => Some(
            "only single-character keys can be looked up in the override table, so a binding \
             here could never fire; Shift-Tab stays the reverse focus move in forms",
        ),
        KeyTrigger::Backspace => Some(
            "only single-character keys can be looked up in the override table, so a binding \
             here could never fire; Backspace stays the text-entry delete",
        ),
        KeyTrigger::CtrlChar('c') => Some(
            "the app returns force-quit for it before any mode or override lookup, so a \
             binding here could never fire",
        ),
        KeyTrigger::CtrlChar('s') => Some(
            "the override table is resolved before the app's own keys, so a binding here would \
             win and silently steal the Logging save-to-file key — no error, no way to tell",
        ),
        KeyTrigger::CtrlChar('f') => Some(
            "the override table is resolved before the app's own keys, so a binding here would \
             win and silently steal the FinOps workspace key — no error, no way to tell",
        ),
        KeyTrigger::CtrlChar('t') => Some(
            "the override table is resolved before the app's own keys, so a binding here would \
             win and silently steal the Tunnels workspace key — no error, no way to tell",
        ),
        // Grouped with `Ctrl-C` rather than with `Ctrl-S`/`Ctrl-T`
        // because the key resolves in `keymap::action_for_key` ahead of the
        // mode match, so the override table never sees it. Still reserved, and
        // the reason still names the key it protects — a config that reaches
        // for it learns what the binding would have cost, not just that it was
        // refused.
        KeyTrigger::CtrlChar('x') => Some(
            "the app returns cancel-running-plugins for it before any mode or override lookup, \
             so a binding here could never fire; a plugin that never returns has no other way \
             to be stopped",
        ),
        _ => None,
    }
}

/// A human-readable rendering of a trigger for error messages.
fn describe(trigger: KeyTrigger) -> String {
    match trigger {
        KeyTrigger::Char(c) => c.to_string(),
        KeyTrigger::CtrlChar(c) => format!("ctrl-{c}"),
        other => format!("{other:?}"),
    }
}

/// Validate an *effective* hotkey table. Detects:
/// - reserved trigger targets;
/// - collisions (two actions mapped to the same effective trigger);
/// - unreachable required actions (`quit` and `help` must each keep a
///   distinct, non-reserved trigger).
pub fn validate(effective: &BTreeMap<RebindableAction, KeyTrigger>) -> Result<(), Vec<String>> {
    let mut errors = Vec::new();

    for (action, trigger) in effective {
        if let Some(reason) = reserved_reason(*trigger) {
            errors.push(format!(
                "hotkeys.{}: {} is a reserved key — {reason}",
                action.config_key(),
                describe(*trigger)
            ));
        }
    }

    let actions: Vec<RebindableAction> = effective.keys().copied().collect();
    for (i, a) in actions.iter().enumerate() {
        for b in actions.iter().skip(i + 1) {
            if effective[a] == effective[b] {
                errors.push(format!(
                    "hotkeys: collision — {} and {} both map to {}",
                    a.config_key(),
                    b.config_key(),
                    describe(effective[a])
                ));
            }
        }
    }

    for required in [RebindableAction::Quit, RebindableAction::Help] {
        let trigger = effective[&required];
        if let Some(reason) = reserved_reason(trigger) {
            errors.push(format!(
                "hotkeys.{}: required action is unreachable (reserved key {} — {reason})",
                required.config_key(),
                describe(trigger)
            ));
        }
        let collides = actions
            .iter()
            .any(|other| *other != required && effective[other] == trigger);
        if collides {
            errors.push(format!(
                "hotkeys.{}: required action is unreachable (key {} shared with another action)",
                required.config_key(),
                describe(trigger)
            ));
        }
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}
