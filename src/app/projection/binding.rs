//! Binding catalog: the single source of key/label display metadata for the
//! footer and the scrollable help view.
//!
//! `context_bindings` mirrors the dispatch rules in `src/input/keymap.rs`
//! (input-agnostic here — no `crossterm` dependency) so the footer, the help
//! overlay, and parity tests all describe the same set of reachable keys per
//! context.

use crate::app::state::command::CommandDescriptor;
use crate::app::state::root::{RootState, Workspace};
use crate::config::hotkeys::{KeyOverrides, RebindableAction};
use nuvem_domain::gcp::resource::ResourceServiceKind;

/// Input-agnostic description of a key that can trigger a binding.
///
/// Mirrors the subset of `crossterm::event::KeyCode` (plus a couple of
/// synthesized cases) that `keymap.rs` dispatches on, without depending on
/// `crossterm` itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyTrigger {
    Char(char),
    CtrlChar(char),
    Enter,
    Esc,
    Tab,
    BackTab,
    Up,
    Down,
    Left,
    Right,
    AltLeft,
    AltRight,
    PageUp,
    PageDown,
    Backspace,
    PrintableText,
}

/// A single key binding, with the key(s) that trigger it and its display
/// metadata.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Binding {
    pub triggers: &'static [KeyTrigger],
    pub key: &'static str,
    pub label: &'static str,
    /// The rebindable action this binding represents, if any. Only bindings
    /// carrying `Some(action)` have their displayed key rewritten when the
    /// user overrides that action's hotkey.
    ///
    /// This identity is encoded intrinsically instead of being recovered by
    /// structural comparison, because some catalog bindings are *byte-for-byte
    /// twins* that nonetheless dispatch differently: `HELP_QUIT` is identical
    /// to `GLOBAL_QUIT` (`q`/"Quit"/`Char('q')`) but the Help overlay quits via
    /// a hardcoded `q` (see `keymap::help_key`) that ignores overrides, so it
    /// must never be relabeled. A structural match cannot tell the twins apart;
    /// an intrinsic tag can, and — unlike pointer identity — it survives the
    /// by-value copies these bindings undergo before reaching `display_binding`.
    pub rebindable: Option<RebindableAction>,
}

/// A binding whose displayed key/triggers reflect the active hotkey
/// overrides. Produced by [`context_bindings_with`] and [`help_model_with`]
/// so the footer and help view show *rebound* keys. Owns its `key` string
/// because a rebound key (e.g. `"H"`, `"Ctrl-x"`) is computed at runtime.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DisplayBinding {
    pub key: String,
    pub label: &'static str,
    pub triggers: Vec<KeyTrigger>,
}

// Navigation
pub const NAV_HOME: Binding = Binding {
    triggers: &[KeyTrigger::Char('h')],
    key: "h",
    label: "Home",
    rebindable: Some(RebindableAction::Home),
};
pub const NAV_CONFIGURATIONS: Binding = Binding {
    triggers: &[KeyTrigger::Char('c')],
    key: "c",
    label: "Configurations",
    rebindable: Some(RebindableAction::Configurations),
};
pub const NAV_RESOURCES: Binding = Binding {
    triggers: &[KeyTrigger::Char('g')],
    key: "g",
    label: "Resources",
    rebindable: Some(RebindableAction::Resources),
};
/// Lowercase `l` is the *workspace* jump. Its label spells that out because
/// `L` ([`RES_SELECTION_LOGS`]) means something narrower; two entries both
/// reading "Logs" would be mixed up on sight.
///
/// The two labels also name each other. They are not adjacent on screen — `l`
/// is a Navigation binding and `L` a current-view one, so the Global section
/// sits between them, and in the Logging workspace `L` is not listed at all.
/// A reader who pressed the wrong case has to be able to find the other key
/// from whichever entry they are looking at.
pub const NAV_LOGGING: Binding = Binding {
    triggers: &[KeyTrigger::Char('l')],
    key: "l",
    label: "Logging workspace: all project logs (L: selection only)",
    rebindable: None,
};
/// `Ctrl-t`, not a bare `t`: `t` is the Logging tail toggle and shadowing it
/// would be a regression (see `keymap::normal_key`).
pub const NAV_TUNNELS: Binding = Binding {
    triggers: &[KeyTrigger::CtrlChar('t')],
    key: "Ctrl-t",
    label: "Tunnels",
    rebindable: None,
};

/// `Ctrl-f`, not a bare `f`: `f` is the resource filter and shadowing it would
/// be a regression. Reserved in `hotkeys::reserved_trigger_reason` for the
/// same reason `Ctrl-t` is — the override table resolves first, so a binding
/// there would silently take this key.
pub const NAV_FINOPS: Binding = Binding {
    triggers: &[KeyTrigger::CtrlChar('f')],
    key: "Ctrl-f",
    label: "FinOps",
    rebindable: None,
};

// History navigation (browser-style back/forward, Alt+Left/Right, all
// workspaces).
pub const HIST_BACK: Binding = Binding {
    triggers: &[KeyTrigger::AltLeft],
    key: "Alt-←",
    label: "Back",
    rebindable: None,
};
pub const HIST_FORWARD: Binding = Binding {
    triggers: &[KeyTrigger::AltRight],
    key: "Alt-→",
    label: "Forward",
    rebindable: None,
};

// Global (available in every normal view)
pub const GLOBAL_REFRESH: Binding = Binding {
    triggers: &[KeyTrigger::Char('r')],
    key: "r",
    label: "Refresh",
    rebindable: Some(RebindableAction::Refresh),
};
pub const GLOBAL_HELP: Binding = Binding {
    triggers: &[KeyTrigger::Char('?')],
    key: "?",
    label: "Help",
    rebindable: Some(RebindableAction::Help),
};
pub const GLOBAL_QUIT: Binding = Binding {
    triggers: &[KeyTrigger::Char('q')],
    key: "q",
    label: "Quit",
    rebindable: Some(RebindableAction::Quit),
};
pub const GLOBAL_FORCE_QUIT: Binding = Binding {
    triggers: &[KeyTrigger::CtrlChar('c')],
    key: "Ctrl-C",
    label: "Quit",
    rebindable: None,
};
pub const GLOBAL_PALETTE: Binding = Binding {
    triggers: &[KeyTrigger::Char(':')],
    key: ":",
    label: "Command palette",
    rebindable: Some(RebindableAction::CommandPalette),
};
pub const GLOBAL_COPY_DIAGNOSTIC: Binding = Binding {
    triggers: &[KeyTrigger::Char('Y')],
    key: "Y",
    label: "Copy diagnostic",
    rebindable: None,
};
/// `Ctrl-x`, not a bare `x`: the key kills a child the operator confirmed, so
/// the modifier is the guard against a stray keystroke (see
/// `keymap::action_for_key`).
///
/// Listed in every context, beside `GLOBAL_FORCE_QUIT` and for the same reason:
/// both resolve ahead of the mode match, so both dispatch
/// wherever the operator is when a run needs killing.
pub const GLOBAL_CANCEL_PLUGIN: Binding = Binding {
    triggers: &[KeyTrigger::CtrlChar('x')],
    key: "Ctrl-x",
    label: "Cancel running plugin",
    rebindable: None,
};

// Resources list
pub const RES_TOGGLE_SIDEBAR: Binding = Binding {
    triggers: &[KeyTrigger::Char('v')],
    key: "v",
    label: "Toggle sidebar",
    rebindable: None,
};
pub const RES_TOGGLE_FOCUS: Binding = Binding {
    triggers: &[KeyTrigger::Tab],
    key: "Tab",
    label: "Focus sidebar",
    rebindable: None,
};
pub const RES_FILTER: Binding = Binding {
    triggers: &[KeyTrigger::Char('/')],
    key: "/",
    label: "Filter",
    rebindable: None,
};
pub const RES_SORT: Binding = Binding {
    triggers: &[KeyTrigger::Char('s')],
    key: "s",
    label: "Sort column",
    rebindable: None,
};
pub const RES_WIDE: Binding = Binding {
    triggers: &[KeyTrigger::Char('w')],
    key: "w",
    label: "Wide columns",
    rebindable: None,
};
pub const RES_COLUMN: Binding = Binding {
    triggers: &[KeyTrigger::Left, KeyTrigger::Right],
    key: "←/→",
    label: "Select column",
    rebindable: None,
};
pub const LIST_SELECT: Binding = Binding {
    triggers: &[
        KeyTrigger::Char('j'),
        KeyTrigger::Char('k'),
        KeyTrigger::Down,
        KeyTrigger::Up,
    ],
    key: "j/k",
    label: "Move selection",
    rebindable: None,
};
pub const RES_OPEN: Binding = Binding {
    triggers: &[KeyTrigger::Enter],
    key: "Enter",
    label: "Open / enter",
    rebindable: None,
};
pub const RES_OPEN_REVISION: Binding = Binding {
    triggers: &[KeyTrigger::Enter],
    key: "Enter",
    label: "Open revision",
    rebindable: None,
};
/// `Esc` and `Alt-←` both go back in history wherever this binding is listed,
/// so they are one entry. Listed as two, the help overlay and the footer read
/// `Esc Back  Alt-← Back`, which looks like two different things.
pub const RES_ESC_BACK: Binding = Binding {
    triggers: &[KeyTrigger::Esc, KeyTrigger::AltLeft],
    key: "Esc/Alt-←",
    label: "Back",
    rebindable: None,
};

// Batch selection (mark/range/clear/unmark, issue #35)
pub const MARK_TOGGLE: Binding = Binding {
    triggers: &[KeyTrigger::Char(' ')],
    key: "Space",
    label: "Mark",
    rebindable: None,
};
pub const MARK_RANGE: Binding = Binding {
    triggers: &[KeyTrigger::Char('V')],
    key: "V",
    label: "Range mark",
    rebindable: None,
};
pub const MARK_CLEAR: Binding = Binding {
    triggers: &[KeyTrigger::Char('\\')],
    key: "\\",
    label: "Clear marks",
    rebindable: None,
};
pub const MARK_UNMARK_VISIBLE: Binding = Binding {
    triggers: &[KeyTrigger::Char('U')],
    key: "U",
    label: "Unmark visible",
    rebindable: None,
};
pub const RES_BUCKET_METADATA: Binding = Binding {
    triggers: &[KeyTrigger::Char('i')],
    key: "i",
    label: "Bucket metadata",
    rebindable: None,
};
pub const RES_COPY_URI: Binding = Binding {
    triggers: &[KeyTrigger::Char('y')],
    key: "y",
    label: "Copy gs:// URI",
    rebindable: None,
};
/// The same `y` key as [`RES_COPY_URI`], described for the detail pane, where
/// it yanks the selected field's value — or the whole document on a JSON tab —
/// for every service, not just Cloud Storage.
pub const RES_COPY_FIELD: Binding = Binding {
    triggers: &[KeyTrigger::Char('y')],
    key: "y",
    label: "Copy field / document",
    rebindable: None,
};
pub const RES_NEXT_PAGE: Binding = Binding {
    triggers: &[KeyTrigger::Char('n')],
    key: "n",
    label: "Next page",
    rebindable: None,
};
pub const RES_START_TUNNEL: Binding = Binding {
    triggers: &[KeyTrigger::Char('t')],
    key: "t",
    label: "Start tunnel",
    rebindable: None,
};
/// Shifted `L`: Cloud Logging scoped to the resource under the cursor. The
/// label names what it acts on *and* names `l`, so a reader who landed in the
/// wrong place can see from this one line which key they wanted (see
/// [`NAV_LOGGING`]).
pub const RES_SELECTION_LOGS: Binding = Binding {
    triggers: &[KeyTrigger::Char('L')],
    key: "L",
    label: "Logs for the selected resource (l: all project logs)",
    rebindable: None,
};
/// Shifted `J`, next to [`RES_SELECTION_LOGS`]'s `L`: the picker of
/// related collections reachable from the resource under the cursor. Like `L`
/// the label names what it acts on, since the key itself says nothing.
pub const RES_RELATED_JUMPS: Binding = Binding {
    triggers: &[KeyTrigger::Char('J')],
    key: "J",
    label: "Related-resource jumps",
    rebindable: None,
};
/// `,` arms the plugin prefix. One entry rather than one per
/// plugin: which keys are declared is runtime data from the plugin file, and
/// the status line lists them while the prefix is armed.
pub const PLUGIN_PREFIX: Binding = Binding {
    triggers: &[KeyTrigger::Char(',')],
    key: ",",
    label: "Run plugin",
    rebindable: None,
};
/// The armed prefix's one statically knowable key.
pub const PLUGIN_PREFIX_CANCEL: Binding = Binding {
    triggers: &[KeyTrigger::Esc],
    key: "Esc",
    label: "Cancel",
    rebindable: None,
};
/// `Esc` inside a resource drill-down, where "back" is the list the operator
/// entered from rather than a history entry. BigQuery's tables view is the one
/// drill-down that answers it — Cloud Storage's object browser is a
/// multi-level prefix descent and keeps `Esc` for history, ascending one level
/// at a time with [`RES_ASCEND`] instead.
pub const RES_LEAVE_DRILLDOWN: Binding = Binding {
    triggers: &[KeyTrigger::Esc],
    key: "Esc",
    label: "Leave drill-down",
    rebindable: None,
};
pub const RES_ASCEND: Binding = Binding {
    triggers: &[KeyTrigger::Backspace, KeyTrigger::Char('-')],
    key: "Backspace/-",
    label: "Up one level",
    rebindable: None,
};

// Resource service navigator (sidebar)
pub const NAV_FILTER: Binding = Binding {
    triggers: &[KeyTrigger::Char('/')],
    key: "/",
    label: "Filter services",
    rebindable: None,
};
pub const NAV_SELECT: Binding = Binding {
    triggers: &[
        KeyTrigger::Char('j'),
        KeyTrigger::Char('k'),
        KeyTrigger::Down,
        KeyTrigger::Up,
    ],
    key: "j/k",
    label: "Move selection",
    rebindable: None,
};
pub const NAV_OPEN: Binding = Binding {
    triggers: &[KeyTrigger::Enter],
    key: "Enter",
    label: "Open service",
    rebindable: None,
};

// Resources detail
pub const RES_PREV_TAB: Binding = Binding {
    triggers: &[KeyTrigger::Char('[')],
    key: "[",
    label: "Previous tab",
    rebindable: None,
};
pub const RES_NEXT_TAB: Binding = Binding {
    triggers: &[KeyTrigger::Char(']')],
    key: "]",
    label: "Next tab",
    rebindable: None,
};
pub const RES_NEXT_FIELD: Binding = Binding {
    triggers: &[KeyTrigger::Char('n')],
    key: "n",
    label: "Next field",
    rebindable: None,
};
pub const RES_PREVIOUS_FIELD: Binding = Binding {
    triggers: &[KeyTrigger::Char('p')],
    key: "p",
    label: "Previous field",
    rebindable: None,
};
pub const RES_SCROLL_DETAIL: Binding = Binding {
    triggers: &[KeyTrigger::PageUp, KeyTrigger::PageDown],
    key: "PgUp/PgDn",
    label: "Scroll",
    rebindable: None,
};
/// The same PgUp/PgDn keys as [`RES_SCROLL_DETAIL`], described for the resource
/// detail pane, where paging also releases the field cursor — the only way to
/// deselect a field, so the help overlay has to say so. The plain label stays
/// for the surfaces with no field cursor (the batch confirmation and the
/// Logging detail).
pub const RES_SCROLL_DETAIL_FIELD: Binding = Binding {
    triggers: &[KeyTrigger::PageUp, KeyTrigger::PageDown],
    key: "PgUp/PgDn",
    label: "Scroll / release field",
    rebindable: None,
};
/// `q`, `Esc` and `Alt-←` all go back from a detail pane, so they are one
/// entry, for the reason [`RES_ESC_BACK`] gives.
pub const DETAIL_BACK: Binding = Binding {
    triggers: &[KeyTrigger::Char('q'), KeyTrigger::Esc, KeyTrigger::AltLeft],
    key: "q/Esc/Alt-←",
    label: "Back",
    // `q` here is the rebindable quit key: in resource detail, quit routes to
    // Back (see `RebindableAction::to_app_action`), so its displayed key must
    // track a quit override.
    rebindable: Some(RebindableAction::Quit),
};

// Logging list & detail
pub const LOG_TAIL: Binding = Binding {
    triggers: &[KeyTrigger::Char('t')],
    key: "t",
    label: "Pause/resume tail",
    rebindable: None,
};
pub const LOG_TIME_RANGE: Binding = Binding {
    triggers: &[KeyTrigger::Char('T')],
    key: "T",
    label: "Time range",
    rebindable: None,
};
pub const LOG_SEVERITY_UP: Binding = Binding {
    triggers: &[KeyTrigger::Char('s')],
    key: "s",
    label: "Raise severity floor",
    rebindable: None,
};
pub const LOG_SEVERITY_DOWN: Binding = Binding {
    triggers: &[KeyTrigger::Char('S')],
    key: "S",
    label: "Lower severity floor",
    rebindable: None,
};
/// One catalog entry for all seven since buckets: the digits are a single
/// contiguous control, so listing them one per line would say nothing extra
/// and would crowd out the rest of the view's help.
pub const LOG_SINCE_BUCKET: Binding = Binding {
    triggers: &[
        KeyTrigger::Char('0'),
        KeyTrigger::Char('1'),
        KeyTrigger::Char('2'),
        KeyTrigger::Char('3'),
        KeyTrigger::Char('4'),
        KeyTrigger::Char('5'),
        KeyTrigger::Char('6'),
    ],
    key: "0-6",
    label: "Since bucket",
    rebindable: None,
};
pub const LOG_SAVE: Binding = Binding {
    triggers: &[KeyTrigger::CtrlChar('s')],
    key: "Ctrl-S",
    label: "Save visible logs",
    rebindable: None,
};
pub const LOG_WRAP: Binding = Binding {
    triggers: &[KeyTrigger::Char('w')],
    key: "w",
    label: "Wrap",
    rebindable: None,
};
pub const LOG_FILTER: Binding = Binding {
    triggers: &[KeyTrigger::Char('/')],
    key: "/",
    label: "Filter",
    rebindable: None,
};
pub const LOG_COPY_ENTRY: Binding = Binding {
    triggers: &[KeyTrigger::Char('y')],
    key: "y",
    label: "Copy entry",
    rebindable: None,
};
pub const LOG_COPY_INSERT_ID: Binding = Binding {
    triggers: &[KeyTrigger::Char('i')],
    key: "i",
    label: "Copy insert ID",
    rebindable: None,
};
pub const LOG_OPEN: Binding = Binding {
    triggers: &[KeyTrigger::Enter],
    key: "Enter",
    label: "Open entry",
    rebindable: None,
};
pub const LOG_CLOSE: Binding = Binding {
    triggers: &[KeyTrigger::Enter, KeyTrigger::Esc, KeyTrigger::Char('q')],
    key: "Enter/Esc/q",
    label: "Close entry",
    // `q` here is the rebindable quit key: in log detail, quit routes to
    // CloseDetail (see `RebindableAction::to_app_action`), so its displayed key
    // must track a quit override.
    rebindable: Some(RebindableAction::Quit),
};
pub const LOG_SCROLL: Binding = Binding {
    triggers: &[KeyTrigger::PageUp, KeyTrigger::PageDown],
    key: "PgUp/PgDn",
    label: "Scroll",
    rebindable: None,
};

// Tunnels list
/// Shifted on purpose: a stray lowercase `k` moves the selection, so killing a
/// live child never sits one keystroke away from ordinary navigation.
pub const TUNNEL_KILL: Binding = Binding {
    triggers: &[KeyTrigger::Char('K')],
    key: "K",
    label: "Kill tunnel",
    rebindable: None,
};
pub const TUNNEL_DISMISS: Binding = Binding {
    triggers: &[KeyTrigger::Char('d')],
    key: "d",
    label: "Dismiss finished",
    rebindable: None,
};

// Configurations list
pub const CFG_CREATE: Binding = Binding {
    triggers: &[KeyTrigger::Char('a')],
    key: "a",
    label: "Create configuration",
    rebindable: None,
};
pub const CFG_EDIT: Binding = Binding {
    triggers: &[KeyTrigger::Char('e')],
    key: "e",
    label: "Edit active context",
    rebindable: None,
};
pub const CFG_CLEAR_FILTER: Binding = Binding {
    triggers: &[KeyTrigger::Char('u')],
    key: "u",
    label: "Clear filter",
    rebindable: None,
};
pub const CFG_ACTIVATE: Binding = Binding {
    triggers: &[KeyTrigger::Enter],
    key: "Enter",
    label: "Activate",
    rebindable: None,
};

// Text-entry & modal bindings (present for parity of non-normal modes; not
// shown in help view)
pub const TYPE_TEXT: Binding = Binding {
    triggers: &[KeyTrigger::PrintableText],
    key: "type",
    label: "Enter text",
    rebindable: None,
};
pub const EDIT_BACKSPACE: Binding = Binding {
    triggers: &[KeyTrigger::Backspace],
    key: "Backspace",
    label: "Delete char",
    rebindable: None,
};
pub const FORM_NEXT_FIELD: Binding = Binding {
    triggers: &[KeyTrigger::Tab, KeyTrigger::Down],
    key: "Tab",
    label: "Next field",
    rebindable: None,
};
pub const FORM_PREV_FIELD: Binding = Binding {
    triggers: &[KeyTrigger::BackTab, KeyTrigger::Up],
    key: "Shift-Tab",
    label: "Previous field",
    rebindable: None,
};
pub const FORM_SUBMIT: Binding = Binding {
    triggers: &[KeyTrigger::Enter],
    key: "Enter",
    label: "Submit",
    rebindable: None,
};
pub const MODAL_CANCEL: Binding = Binding {
    triggers: &[KeyTrigger::Esc],
    key: "Esc",
    label: "Cancel",
    rebindable: None,
};
pub const FILTER_APPLY: Binding = Binding {
    triggers: &[KeyTrigger::Enter],
    key: "Enter",
    label: "Apply",
    rebindable: None,
};
pub const PICKER_FILTER: Binding = Binding {
    triggers: &[KeyTrigger::Char('/')],
    key: "/",
    label: "Filter",
    rebindable: None,
};
pub const PICKER_RETRY: Binding = Binding {
    triggers: &[KeyTrigger::Char('r')],
    key: "r",
    label: "Retry discovery",
    rebindable: None,
};
pub const PICKER_SELECT: Binding = Binding {
    triggers: &[KeyTrigger::Enter],
    key: "Enter",
    label: "Select",
    rebindable: None,
};
/// [`PICKER_SELECT`]'s `Enter` in the related-resource jump picker, where the
/// verb is worth spelling out: the row names a destination, not a value.
pub const JUMP_PICKER_SELECT: Binding = Binding {
    triggers: &[KeyTrigger::Enter],
    key: "Enter",
    label: "Jump to selected",
    rebindable: None,
};
pub const CTX_PICK_VALUE: Binding = Binding {
    triggers: &[KeyTrigger::CtrlChar(' ')],
    key: "Ctrl-Space",
    label: "Pick value",
    rebindable: None,
};
pub const PALETTE_SELECT: Binding = Binding {
    // The command palette is a text-filter: bare printable keys (including
    // `j`/`k`) type into the query (see keymap `command_palette_key` and
    // README), so navigation uses the arrow keys.
    triggers: &[KeyTrigger::Down, KeyTrigger::Up],
    key: "↑/↓",
    label: "Select",
    rebindable: None,
};
pub const PALETTE_RUN: Binding = Binding {
    triggers: &[KeyTrigger::Enter],
    key: "Enter",
    label: "Run",
    rebindable: None,
};
pub const CONFIRM_ACCEPT: Binding = Binding {
    triggers: &[KeyTrigger::Enter],
    key: "Enter",
    label: "Confirm",
    rebindable: None,
};
pub const CONFIRM_ACCEPT_Y: Binding = Binding {
    triggers: &[KeyTrigger::Char('y')],
    key: "y",
    label: "Confirm",
    rebindable: None,
};
pub const CONFIRM_REJECT_N: Binding = Binding {
    triggers: &[KeyTrigger::Char('n')],
    key: "n",
    label: "Cancel",
    rebindable: None,
};
pub const PREVIEW_COPY: Binding = Binding {
    triggers: &[KeyTrigger::Enter],
    key: "Enter",
    label: "Copy",
    rebindable: None,
};
/// A plugin confirmation's `Enter`. It says "Run command", not
/// "Confirm", because that is what it does — and pointedly not "Copy", which
/// is what the same key does one modal over in [`PREVIEW_COPY`]. A reader who
/// cannot tell the two apart from the footer has no other way to tell them
/// apart before pressing the key.
pub const PLUGIN_RUN: Binding = Binding {
    triggers: &[KeyTrigger::Enter],
    key: "Enter",
    label: "Run command",
    rebindable: None,
};
/// The typed acknowledgement a `dangerous` plugin demands: `1`, one command
/// against one row.
///
/// All ten digits are listed because all ten dispatch — the keymap accepts any
/// digit and lets the acknowledgement fail to match, rather than silently
/// swallowing the wrong ones, so a footer naming only `1` would be claiming a
/// narrower keymap than the one that exists.
pub const PLUGIN_ACK_DIGIT: Binding = Binding {
    triggers: &[
        KeyTrigger::Char('0'),
        KeyTrigger::Char('1'),
        KeyTrigger::Char('2'),
        KeyTrigger::Char('3'),
        KeyTrigger::Char('4'),
        KeyTrigger::Char('5'),
        KeyTrigger::Char('6'),
        KeyTrigger::Char('7'),
        KeyTrigger::Char('8'),
        KeyTrigger::Char('9'),
    ],
    key: "0-9",
    label: "Type 1 to acknowledge a dangerous plugin",
    rebindable: None,
};
/// The typed-input prompt's `Enter`. It says "Continue", not "Run":
/// this key composes the argv and hands it to the confirmation, which is the
/// only place a plugin becomes a process. A footer promising a run here would
/// be describing [`PLUGIN_RUN`], one modal later.
pub const PLUGIN_PROMPT_CONTINUE: Binding = Binding {
    triggers: &[KeyTrigger::Enter],
    key: "Enter",
    label: "Continue to the confirmation",
    rebindable: None,
};
/// Field movement in the prompt. `Tab`/`Shift-Tab` alone, unlike
/// [`FORM_NEXT_FIELD`]: the prompt's arrows belong to the dropdown below, so
/// the vertical keys the configuration forms also accept are deliberately not
/// claimed here, and the catalog names exactly the keys the keymap dispatches.
pub const PLUGIN_PROMPT_NEXT_FIELD: Binding = Binding {
    triggers: &[KeyTrigger::Tab],
    key: "Tab",
    label: "Next field",
    rebindable: None,
};
pub const PLUGIN_PROMPT_PREV_FIELD: Binding = Binding {
    triggers: &[KeyTrigger::BackTab],
    key: "Shift-Tab",
    label: "Previous field",
    rebindable: None,
};
pub const PLUGIN_PROMPT_CHOICE: Binding = Binding {
    triggers: &[KeyTrigger::Left, KeyTrigger::Right],
    key: "←/→",
    label: "Change a dropdown's choice",
    rebindable: None,
};
pub const LAUNCH_CONFIRM: Binding = Binding {
    triggers: &[KeyTrigger::Enter],
    key: "Enter",
    label: "Launch",
    rebindable: None,
};

// Help view controls
pub const HELP_SCROLL: Binding = Binding {
    triggers: &[
        KeyTrigger::Char('j'),
        KeyTrigger::Char('k'),
        KeyTrigger::Down,
        KeyTrigger::Up,
    ],
    key: "j/k",
    label: "Scroll",
    rebindable: None,
};
pub const HELP_PAGE: Binding = Binding {
    triggers: &[KeyTrigger::PageUp, KeyTrigger::PageDown],
    key: "PgUp/PgDn",
    label: "Page",
    rebindable: None,
};
pub const HELP_JUMP: Binding = Binding {
    triggers: &[KeyTrigger::Char('g'), KeyTrigger::Char('G')],
    key: "g/G",
    label: "Top/Bottom",
    rebindable: None,
};
pub const HELP_CLOSE: Binding = Binding {
    triggers: &[KeyTrigger::Esc, KeyTrigger::Char('?')],
    key: "Esc/?",
    label: "Close",
    rebindable: None,
};
// Byte-for-byte identical to `GLOBAL_QUIT`, but the Help overlay quits via a
// hardcoded `q` in `keymap::help_key` that ignores hotkey overrides. It is
// therefore NOT rebindable: `rebindable: None` keeps its displayed key as `q`
// even when the user rebinds `quit` elsewhere.
pub const HELP_QUIT: Binding = Binding {
    triggers: &[KeyTrigger::Char('q')],
    key: "q",
    label: "Quit",
    rebindable: None,
};

/// Every context the footer/help view can be in. Mirrors the dispatch modes
/// in `src/input/keymap.rs::Mode`, plus the normal-mode sub-views that share
/// `Mode::Normal`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HelpContext {
    Home,
    ConfigList,
    ConfigFilter,
    ConfigCreate,
    ConfigContextEdit,
    ConfigContextPicker,
    ConfigContextPickerFilter,
    ResourceList,
    ResourceNavigator,
    ResourceDetail,
    ResourceFilter,
    ServiceFilter,
    LoggingList,
    LoggingDetail,
    LoggingFilter,
    TunnelsList,
    FinOps,
    CommandPalette,
    ConfirmResourceAction,
    ConfirmConfigurationActivation,
    ResourceCommandPreview,
    SshLaunchPreview,
    TunnelLaunchPreview,
    /// A plugin's argv, awaiting confirmation.
    PluginConfirmation,
    /// `,` is armed, awaiting a plugin key.
    PluginPrefix,
    /// The typed-input prompt a plugin's declared inputs are collected in,
    /// ahead of the confirmation.
    PluginPrompt,
    /// The related-resource jump picker (`J`).
    ResourceJumpPicker,
    /// A resource list reached by drilling into a row of the list above it,
    /// with another level below it — BigQuery's tables view. Every
    /// `ResourceList` key still dispatches; what differs is `Esc`, which
    /// leaves the drill instead of walking history back.
    ResourceDrilldown,
    /// The same, at the bottom of the hierarchy — BigQuery's schema view.
    /// Identical to [`Self::ResourceDrilldown`] except that `Enter`
    /// opens nothing here, so it is neither dispatched nor taught.
    ///
    /// A variant rather than a parameter on the one above, because
    /// `HelpContext` is `Copy`, is compared for equality all over the parity
    /// harness, and is enumerated by `ALL_HELP_CONTEXTS` — a payload would
    /// cost the first, silently change the meaning of the second, and make the
    /// third a question rather than a list.
    ResourceDrilldownLeaf,
    Help,
}

/// All `HelpContext` variants, for parity tests that must exercise every
/// context.
pub const ALL_HELP_CONTEXTS: &[HelpContext] = &[
    HelpContext::Home,
    HelpContext::ConfigList,
    HelpContext::ConfigFilter,
    HelpContext::ConfigCreate,
    HelpContext::ConfigContextEdit,
    HelpContext::ConfigContextPicker,
    HelpContext::ConfigContextPickerFilter,
    HelpContext::ResourceList,
    HelpContext::ResourceNavigator,
    HelpContext::ResourceDetail,
    HelpContext::ResourceFilter,
    HelpContext::ServiceFilter,
    HelpContext::LoggingList,
    HelpContext::LoggingDetail,
    HelpContext::LoggingFilter,
    HelpContext::TunnelsList,
    HelpContext::FinOps,
    HelpContext::CommandPalette,
    HelpContext::ConfirmResourceAction,
    HelpContext::ConfirmConfigurationActivation,
    HelpContext::ResourceCommandPreview,
    HelpContext::SshLaunchPreview,
    HelpContext::TunnelLaunchPreview,
    HelpContext::PluginConfirmation,
    HelpContext::PluginPrefix,
    HelpContext::PluginPrompt,
    HelpContext::ResourceJumpPicker,
    HelpContext::ResourceDrilldown,
    HelpContext::ResourceDrilldownLeaf,
    HelpContext::Help,
];

/// The keys pinned to the right of every normal view's footer, in render
/// order.
///
/// These two and no others, because these two are the ones that lead
/// everywhere else: `?` opens the overlay that lists the whole context, and
/// `:` opens the palette that runs any command by name. Every other global is
/// either situational (`Y` has a diagnostic to copy or it has not) or
/// destructive (`q`, `Ctrl-C`), and a permanent hint costs footer width in
/// every view at once.
///
/// Pinned rather than appended to the view's own hints because the footer is
/// one line that fills left-to-right and drops what does not fit: appended,
/// they would be dropped by exactly the narrow terminal whose operator has
/// least room to guess, and prepended they would push the view's own — more
/// informative — keys off the same edge. `render_footer` lays them out
/// right-aligned instead; `frame::fitted_pins` owns how much room they may
/// take when the two groups do not both fit.
pub const PINNED_BINDINGS: &[Binding] = &[GLOBAL_PALETTE, GLOBAL_HELP];

/// Bindings global to every context, shown in the "Global" help section and
/// excluded from a context's "Current view" bindings.
const GLOBAL_BINDINGS: &[Binding] = &[
    GLOBAL_REFRESH,
    GLOBAL_HELP,
    GLOBAL_QUIT,
    GLOBAL_FORCE_QUIT,
    GLOBAL_PALETTE,
    GLOBAL_COPY_DIAGNOSTIC,
    GLOBAL_CANCEL_PLUGIN,
];

/// Navigation bindings, shown in the "Navigation" help section and excluded
/// from a context's "Current view" bindings.
const NAV_BINDINGS: &[Binding] = &[
    NAV_HOME,
    NAV_CONFIGURATIONS,
    NAV_RESOURCES,
    NAV_LOGGING,
    NAV_TUNNELS,
];

/// Every binding reachable in a given context (used by parity tests and the
/// help view). Must mirror the dispatch rules in `src/input/keymap.rs`.
pub fn context_bindings(ctx: HelpContext) -> Vec<Binding> {
    match ctx {
        HelpContext::Home => vec![
            GLOBAL_PALETTE,
            GLOBAL_REFRESH,
            GLOBAL_COPY_DIAGNOSTIC,
            GLOBAL_CANCEL_PLUGIN,
            GLOBAL_HELP,
            GLOBAL_QUIT,
            GLOBAL_FORCE_QUIT,
            NAV_HOME,
            NAV_CONFIGURATIONS,
            NAV_RESOURCES,
            NAV_LOGGING,
            NAV_TUNNELS,
            NAV_FINOPS,
            HIST_BACK,
            HIST_FORWARD,
        ],
        HelpContext::ConfigList => vec![
            GLOBAL_PALETTE,
            RES_FILTER,
            CFG_CLEAR_FILTER,
            CFG_CREATE,
            CFG_EDIT,
            LIST_SELECT,
            CFG_ACTIVATE,
            GLOBAL_REFRESH,
            GLOBAL_COPY_DIAGNOSTIC,
            GLOBAL_CANCEL_PLUGIN,
            GLOBAL_HELP,
            GLOBAL_QUIT,
            GLOBAL_FORCE_QUIT,
            NAV_HOME,
            NAV_CONFIGURATIONS,
            NAV_RESOURCES,
            NAV_LOGGING,
            NAV_TUNNELS,
            NAV_FINOPS,
            HIST_BACK,
            HIST_FORWARD,
        ],
        HelpContext::ResourceList => vec![
            GLOBAL_PALETTE,
            RES_TOGGLE_SIDEBAR,
            RES_TOGGLE_FOCUS,
            RES_FILTER,
            RES_SORT,
            RES_WIDE,
            RES_COLUMN,
            LIST_SELECT,
            RES_OPEN,
            MARK_TOGGLE,
            MARK_RANGE,
            MARK_CLEAR,
            MARK_UNMARK_VISIBLE,
            RES_START_TUNNEL,
            RES_BUCKET_METADATA,
            RES_COPY_URI,
            RES_NEXT_PAGE,
            RES_SELECTION_LOGS,
            RES_RELATED_JUMPS,
            PLUGIN_PREFIX,
            RES_ASCEND,
            GLOBAL_REFRESH,
            GLOBAL_COPY_DIAGNOSTIC,
            GLOBAL_CANCEL_PLUGIN,
            GLOBAL_HELP,
            GLOBAL_QUIT,
            GLOBAL_FORCE_QUIT,
            NAV_HOME,
            NAV_CONFIGURATIONS,
            NAV_RESOURCES,
            NAV_LOGGING,
            NAV_TUNNELS,
            NAV_FINOPS,
            RES_ESC_BACK,
            HIST_FORWARD,
        ],
        // Derived from the list's rather than copied beside it: a copied list
        // would be two catalogues to keep in step, drifting one binding at a
        // time. Two edits, and both are edits the keymap makes too — this
        // catalogue is a claim about `resource_content_key` and
        // `normal_key`'s guarded `Esc` arm, not a second opinion beside them.
        //
        // Substituted: `Esc` leaves the drill instead of walking history back.
        //
        // Removed: `s` and `←`/`→`. They dispatch nothing in a drill-down,
        // because what they reached was the collection *behind* it — `s` and
        // the column cursor act on a table the operator cannot see. Teaching
        // them here would be the same false promise the frame footer was
        // making, on the more authoritative of the two surfaces.
        //
        // `RES_OPEN` is kept at the level that has something below it and
        // dropped at the level that does not — the one difference between the
        // two arms, and the reason there are two.
        //
        // Both levels resolve here because `is_showing_drilldown` counts both
        // of them; which one is decided by `drilldown_opens_another_level`,
        // carried onto the keymap `Context` as
        // `resources_drilldown_opens_another_level`. Until it was, the context
        // held one drill-down flag and no depth, so this arm served two levels
        // that differ in what `Enter` does and taught `Open / enter` on a
        // column row. The frame footer never had the problem: it reads live
        // state rather than a context.
        //
        // The catalogue could not be narrowed on its own. `Enter` dispatched
        // at both levels, and `keymap_help_parity_test` refuses a key that
        // dispatches with no catalogue entry — so the keymap arm had to stop
        // answering at the bottom level first. See `input::keymap`'s `Enter`
        // arm for why that is not an observable change.
        //
        // `RES_ASCEND` stays at both: `-`/`Backspace` also leave the drill,
        // and "up one level" is exactly what that is —
        // `leave_bigquery_drilldown` ascends one level per keypress.
        //
        // The leaf arm is written as a difference from the branch rather than
        // as a second list, so a binding added to the drill-down reaches both
        // levels instead of only the one someone remembered.
        HelpContext::ResourceDrilldown | HelpContext::ResourceDrilldownLeaf => {
            let leaf = ctx == HelpContext::ResourceDrilldownLeaf;
            context_bindings(HelpContext::ResourceList)
                .into_iter()
                .filter(|binding| ![RES_SORT, RES_COLUMN].contains(binding))
                .filter(|binding| !(leaf && *binding == RES_OPEN))
                // `Esc` leaves the drill-down here, so history keeps only
                // `Alt-←`.
                .flat_map(|binding| {
                    if binding == RES_ESC_BACK {
                        vec![RES_LEAVE_DRILLDOWN, HIST_BACK]
                    } else {
                        vec![binding]
                    }
                })
                .collect()
        }
        HelpContext::ResourceNavigator => vec![
            GLOBAL_PALETTE,
            RES_TOGGLE_SIDEBAR,
            RES_TOGGLE_FOCUS,
            NAV_FILTER,
            NAV_SELECT,
            NAV_OPEN,
            GLOBAL_REFRESH,
            GLOBAL_COPY_DIAGNOSTIC,
            GLOBAL_CANCEL_PLUGIN,
            GLOBAL_HELP,
            GLOBAL_QUIT,
            GLOBAL_FORCE_QUIT,
            NAV_HOME,
            NAV_CONFIGURATIONS,
            NAV_RESOURCES,
            NAV_LOGGING,
            NAV_TUNNELS,
            NAV_FINOPS,
            RES_ESC_BACK,
            HIST_FORWARD,
        ],
        HelpContext::ResourceDetail => vec![
            GLOBAL_PALETTE,
            RES_TOGGLE_SIDEBAR,
            RES_TOGGLE_FOCUS,
            LIST_SELECT,
            RES_PREV_TAB,
            RES_NEXT_TAB,
            RES_NEXT_FIELD,
            RES_PREVIOUS_FIELD,
            RES_SCROLL_DETAIL_FIELD,
            RES_OPEN_REVISION,
            RES_COPY_FIELD,
            RES_SELECTION_LOGS,
            RES_RELATED_JUMPS,
            PLUGIN_PREFIX,
            DETAIL_BACK,
            GLOBAL_REFRESH,
            GLOBAL_COPY_DIAGNOSTIC,
            GLOBAL_CANCEL_PLUGIN,
            GLOBAL_HELP,
            GLOBAL_FORCE_QUIT,
            NAV_HOME,
            NAV_CONFIGURATIONS,
            NAV_RESOURCES,
            NAV_LOGGING,
            NAV_TUNNELS,
            NAV_FINOPS,
            HIST_FORWARD,
        ],
        HelpContext::ResourceFilter => vec![
            TYPE_TEXT,
            EDIT_BACKSPACE,
            FILTER_APPLY,
            MODAL_CANCEL,
            GLOBAL_CANCEL_PLUGIN,
            GLOBAL_FORCE_QUIT,
        ],
        HelpContext::ServiceFilter => vec![
            TYPE_TEXT,
            EDIT_BACKSPACE,
            FILTER_APPLY,
            MODAL_CANCEL,
            GLOBAL_CANCEL_PLUGIN,
            GLOBAL_FORCE_QUIT,
        ],
        HelpContext::ConfigFilter => vec![
            TYPE_TEXT,
            EDIT_BACKSPACE,
            FILTER_APPLY,
            MODAL_CANCEL,
            GLOBAL_CANCEL_PLUGIN,
            GLOBAL_FORCE_QUIT,
        ],
        HelpContext::ConfigCreate => vec![
            TYPE_TEXT,
            EDIT_BACKSPACE,
            FORM_NEXT_FIELD,
            FORM_PREV_FIELD,
            FORM_SUBMIT,
            MODAL_CANCEL,
            GLOBAL_CANCEL_PLUGIN,
            GLOBAL_FORCE_QUIT,
        ],
        HelpContext::ConfigContextEdit => vec![
            CTX_PICK_VALUE,
            TYPE_TEXT,
            EDIT_BACKSPACE,
            FORM_NEXT_FIELD,
            FORM_PREV_FIELD,
            FORM_SUBMIT,
            MODAL_CANCEL,
            GLOBAL_CANCEL_PLUGIN,
            GLOBAL_FORCE_QUIT,
        ],
        HelpContext::ConfigContextPicker => vec![
            PICKER_FILTER,
            LIST_SELECT,
            PICKER_RETRY,
            PICKER_SELECT,
            MODAL_CANCEL,
            GLOBAL_CANCEL_PLUGIN,
            GLOBAL_FORCE_QUIT,
        ],
        HelpContext::ConfigContextPickerFilter => vec![
            TYPE_TEXT,
            EDIT_BACKSPACE,
            FILTER_APPLY,
            MODAL_CANCEL,
            GLOBAL_CANCEL_PLUGIN,
            GLOBAL_FORCE_QUIT,
        ],
        HelpContext::LoggingList => vec![
            GLOBAL_PALETTE,
            LIST_SELECT,
            LOG_OPEN,
            LOG_TAIL,
            LOG_TIME_RANGE,
            LOG_SINCE_BUCKET,
            LOG_SEVERITY_UP,
            LOG_SEVERITY_DOWN,
            LOG_WRAP,
            LOG_FILTER,
            LOG_COPY_ENTRY,
            LOG_COPY_INSERT_ID,
            LOG_SAVE,
            GLOBAL_REFRESH,
            GLOBAL_COPY_DIAGNOSTIC,
            GLOBAL_CANCEL_PLUGIN,
            GLOBAL_HELP,
            GLOBAL_QUIT,
            GLOBAL_FORCE_QUIT,
            NAV_HOME,
            NAV_CONFIGURATIONS,
            NAV_RESOURCES,
            NAV_LOGGING,
            NAV_TUNNELS,
            NAV_FINOPS,
            RES_ESC_BACK,
            HIST_FORWARD,
        ],
        HelpContext::LoggingDetail => vec![
            GLOBAL_PALETTE,
            LOG_CLOSE,
            LOG_SCROLL,
            LOG_WRAP,
            LOG_COPY_ENTRY,
            LOG_COPY_INSERT_ID,
            GLOBAL_REFRESH,
            GLOBAL_COPY_DIAGNOSTIC,
            GLOBAL_CANCEL_PLUGIN,
            GLOBAL_HELP,
            GLOBAL_FORCE_QUIT,
            NAV_HOME,
            NAV_CONFIGURATIONS,
            NAV_RESOURCES,
            NAV_LOGGING,
            NAV_TUNNELS,
            NAV_FINOPS,
            HIST_BACK,
            HIST_FORWARD,
        ],
        HelpContext::LoggingFilter => vec![
            TYPE_TEXT,
            EDIT_BACKSPACE,
            FILTER_APPLY,
            MODAL_CANCEL,
            GLOBAL_CANCEL_PLUGIN,
            GLOBAL_FORCE_QUIT,
        ],
        // `r` (Refresh) is listed because it still dispatches here — it
        // re-selects the workspace and changes nothing, since tunnel rows are
        // pushed by the supervisor rather than fetched. See
        // `RebindableAction::to_app_action`.
        HelpContext::TunnelsList => vec![
            GLOBAL_PALETTE,
            LIST_SELECT,
            TUNNEL_KILL,
            TUNNEL_DISMISS,
            GLOBAL_REFRESH,
            GLOBAL_COPY_DIAGNOSTIC,
            GLOBAL_CANCEL_PLUGIN,
            GLOBAL_HELP,
            GLOBAL_QUIT,
            GLOBAL_FORCE_QUIT,
            NAV_HOME,
            NAV_CONFIGURATIONS,
            NAV_RESOURCES,
            NAV_LOGGING,
            NAV_TUNNELS,
            NAV_FINOPS,
            RES_ESC_BACK,
            HIST_FORWARD,
        ],
        // No list keys: v1 is a read-only summary with nothing to select and
        // nothing to act on. `r` re-probes every cell, which is a real fetch.
        HelpContext::FinOps => vec![
            GLOBAL_PALETTE,
            GLOBAL_REFRESH,
            GLOBAL_COPY_DIAGNOSTIC,
            GLOBAL_CANCEL_PLUGIN,
            GLOBAL_HELP,
            GLOBAL_QUIT,
            GLOBAL_FORCE_QUIT,
            NAV_HOME,
            NAV_CONFIGURATIONS,
            NAV_RESOURCES,
            NAV_LOGGING,
            NAV_TUNNELS,
            NAV_FINOPS,
            RES_ESC_BACK,
            HIST_FORWARD,
        ],
        HelpContext::CommandPalette => vec![
            PALETTE_SELECT,
            TYPE_TEXT,
            EDIT_BACKSPACE,
            PALETTE_RUN,
            MODAL_CANCEL,
            GLOBAL_CANCEL_PLUGIN,
            GLOBAL_FORCE_QUIT,
        ],
        HelpContext::ConfirmResourceAction => {
            vec![
                CONFIRM_ACCEPT,
                MODAL_CANCEL,
                GLOBAL_CANCEL_PLUGIN,
                GLOBAL_FORCE_QUIT,
            ]
        }
        HelpContext::ConfirmConfigurationActivation => vec![
            CONFIRM_ACCEPT,
            CONFIRM_ACCEPT_Y,
            CONFIRM_REJECT_N,
            MODAL_CANCEL,
            GLOBAL_CANCEL_PLUGIN,
            GLOBAL_FORCE_QUIT,
        ],
        HelpContext::ResourceCommandPreview => {
            vec![
                PREVIEW_COPY,
                MODAL_CANCEL,
                GLOBAL_CANCEL_PLUGIN,
                GLOBAL_FORCE_QUIT,
            ]
        }
        // `Backspace` and the digits are listed for every plugin, not only a
        // `dangerous` one: the catalog is per-context and the keymap arm behind
        // them is ungated, so they do arrive here whatever the plugin is. The
        // labels say which plugin they are for.
        HelpContext::PluginPrompt => vec![
            TYPE_TEXT,
            EDIT_BACKSPACE,
            PLUGIN_PROMPT_NEXT_FIELD,
            PLUGIN_PROMPT_PREV_FIELD,
            PLUGIN_PROMPT_CHOICE,
            PLUGIN_PROMPT_CONTINUE,
            MODAL_CANCEL,
            GLOBAL_CANCEL_PLUGIN,
            GLOBAL_FORCE_QUIT,
        ],
        HelpContext::PluginConfirmation => vec![
            PLUGIN_RUN,
            PLUGIN_ACK_DIGIT,
            EDIT_BACKSPACE,
            MODAL_CANCEL,
            GLOBAL_CANCEL_PLUGIN,
            GLOBAL_FORCE_QUIT,
        ],
        // One entry and two globals, and the shortness is still the point: with
        // `,` armed every printable character dispatches, and which of them
        // names a plugin comes from the operator's file, so `Esc` is the only
        // key this context can describe statically. Both globals are listed
        // because `action_for_key` resolves them ahead of the mode match. That
        // is what changed for `Ctrl-x`: while it was an arm of
        // `normal_key` it dispatched nowhere here, and listing it would have
        // been the orphan `no_orphan_catalog_bindings` exists to reject. This is
        // the context the change was raised over — the others at least render a
        // modal that explains who owns the keyboard.
        HelpContext::PluginPrefix => vec![
            PLUGIN_PREFIX_CANCEL,
            GLOBAL_CANCEL_PLUGIN,
            GLOBAL_FORCE_QUIT,
        ],
        HelpContext::SshLaunchPreview | HelpContext::TunnelLaunchPreview => {
            vec![
                LAUNCH_CONFIRM,
                MODAL_CANCEL,
                GLOBAL_CANCEL_PLUGIN,
                GLOBAL_FORCE_QUIT,
            ]
        }
        // The smallest picker in the app: its rows come from state that is
        // already loaded, so there is nothing to filter and nothing to retry.
        HelpContext::ResourceJumpPicker => vec![
            LIST_SELECT,
            JUMP_PICKER_SELECT,
            MODAL_CANCEL,
            GLOBAL_CANCEL_PLUGIN,
            GLOBAL_FORCE_QUIT,
        ],
        HelpContext::Help => vec![
            HELP_SCROLL,
            HELP_PAGE,
            HELP_JUMP,
            HELP_CLOSE,
            HELP_QUIT,
            GLOBAL_CANCEL_PLUGIN,
            GLOBAL_FORCE_QUIT,
        ],
    }
}

/// The normal (non-modal) view underlying root state, used by `help_model`
/// to decide which view's bindings to teach — independent of whatever
/// transient overlay (help itself, a filter, a confirmation, ...) is
/// currently active. Never resolves to `ResourceNavigator`/`ServiceFilter`:
/// those `HelpContext` variants exist only so the parity harness can probe
/// the navigator-focused/filter-open dispatch branches; the footer/help
/// view always teaches the resource list/detail view as a whole.
fn normal_view_context(root: &RootState) -> HelpContext {
    match root.workspace {
        Workspace::Home => HelpContext::Home,
        Workspace::Configurations => HelpContext::ConfigList,
        Workspace::Resources => {
            if root.resources.is_showing_detail() {
                HelpContext::ResourceDetail
            } else if root.resources.is_showing_drilldown() {
                // A drill-down is a list, so this is not a `ResourceDetail`
                // case; what it needs its own context for is `Esc`, which
                // means something here that appears in no other resource
                // context's catalogue.
                //
                // Which of the two, by whether anything is below it. This is
                // the same question the frame footer asks of the same method,
                // and asking it here is what stopped the overlay teaching a
                // key that does nothing.
                if root.resources.drilldown_opens_another_level() {
                    HelpContext::ResourceDrilldown
                } else {
                    HelpContext::ResourceDrilldownLeaf
                }
            } else {
                HelpContext::ResourceList
            }
        }
        Workspace::Logging => {
            if root.logging.detail_open {
                HelpContext::LoggingDetail
            } else {
                HelpContext::LoggingList
            }
        }
        Workspace::Tunnels => HelpContext::TunnelsList,
        Workspace::FinOps => HelpContext::FinOps,
    }
}

fn view_title(ctx: HelpContext) -> &'static str {
    match ctx {
        HelpContext::Home => "Home",
        HelpContext::ConfigList => "Configurations",
        HelpContext::ResourceList => "Resources",
        HelpContext::ResourceDetail => "Resource detail",
        // One title for both drilled levels, deliberately: the operator is
        // looking at "a drill-down" either way, and the depth changes what
        // `Enter` does rather than what the view is. One arm rather than two
        // identical ones so that deleting it is one mutation rather than two,
        // each of which would need its own pin. The catch-all below answers
        // "Help", so a deleted arm is silent rather than a compile error;
        // `the_help_overlay_is_titled_for_the_drilldown_it_is_open_over` in
        // `tests/bigquery_drilldown_test.rs` asserts both levels for that
        // reason.
        HelpContext::ResourceDrilldown | HelpContext::ResourceDrilldownLeaf => {
            "Resource drill-down"
        }
        HelpContext::LoggingList => "Logging",
        HelpContext::LoggingDetail => "Log detail",
        HelpContext::LoggingFilter => "Filter logs",
        HelpContext::TunnelsList => "Tunnels",
        HelpContext::FinOps => "FinOps",
        _ => "Help",
    }
}

/// The rebindable action a catalog `Binding` represents, read from the
/// binding's intrinsic [`Binding::rebindable`] tag rather than recovered by
/// structural comparison. This is what keeps same-key-but-different-purpose
/// bindings apart — both the harmless case (`GLOBAL_REFRESH` vs `PICKER_RETRY`,
/// both `r`) and the byte-for-byte twin case (`GLOBAL_QUIT` vs `HELP_QUIT`,
/// which a structural match cannot distinguish at all). Only bindings tagged
/// `Some(action)` ever have their displayed key rewritten by an override.
fn rebindable_action_of(binding: &Binding) -> Option<RebindableAction> {
    binding.rebindable
}

/// Renders a single trigger to its footer/help display token.
fn trigger_token(trigger: KeyTrigger) -> String {
    match trigger {
        KeyTrigger::Char(c) => c.to_string(),
        KeyTrigger::CtrlChar(c) => format!("Ctrl-{c}"),
        KeyTrigger::Enter => "Enter".to_string(),
        KeyTrigger::Esc => "Esc".to_string(),
        KeyTrigger::Tab => "Tab".to_string(),
        KeyTrigger::BackTab => "Shift-Tab".to_string(),
        KeyTrigger::Backspace => "Backspace".to_string(),
        KeyTrigger::AltLeft => "Alt-←".to_string(),
        KeyTrigger::AltRight => "Alt-→".to_string(),
        other => format!("{other:?}"),
    }
}

/// Converts a catalog `Binding` into a `DisplayBinding`, rewriting the
/// displayed key/triggers when it is a rebindable binding and an override is
/// active. Non-rebindable bindings pass through unchanged.
pub fn display_binding(binding: &Binding, overrides: &KeyOverrides) -> DisplayBinding {
    match rebindable_action_of(binding) {
        None => DisplayBinding {
            key: binding.key.to_string(),
            label: binding.label,
            triggers: binding.triggers.to_vec(),
        },
        Some(action) => {
            let default = action.default_trigger();
            let effective = overrides.effective_trigger(action);
            let triggers: Vec<KeyTrigger> = binding
                .triggers
                .iter()
                .map(|trigger| {
                    if *trigger == default {
                        effective
                    } else {
                        *trigger
                    }
                })
                .collect();
            let key = triggers
                .iter()
                .map(|trigger| trigger_token(*trigger))
                .collect::<Vec<_>>()
                .join("/");
            DisplayBinding {
                key,
                label: binding.label,
                triggers,
            }
        }
    }
}

/// Override-aware variant of [`context_bindings`]: the same binding sets, but
/// with rebindable bindings' key/triggers reflecting the active overrides.
pub fn context_bindings_with(ctx: HelpContext, overrides: &KeyOverrides) -> Vec<DisplayBinding> {
    context_bindings(ctx)
        .iter()
        .map(|binding| display_binding(binding, overrides))
        .collect()
}

/// A titled group of bindings shown together in the help view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HelpSection {
    pub title: &'static str,
    pub bindings: Vec<DisplayBinding>,
}

/// The full help view model: a title for the underlying view and its
/// grouped bindings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HelpModel {
    pub view_title: String,
    /// Configuration problems recorded by the most recent load or reload,
    /// listed above the binding sections when non-empty.
    pub problems: Vec<String>,
    pub sections: Vec<HelpSection>,
}

/// The key that runs `command` from the view the palette is open over, or
/// `None` where there is no such key.
///
/// Two questions, and both have to be answered or the palette teaches a lie.
/// The first is whether the command has a key at all: most do not — the
/// lifecycle actions, the field copies and the context switch are palette-only
/// by design, and inventing a key for them would be worse than saying nothing.
/// The second is whether that key dispatches *here*: `L` opens the selection's
/// logs from a resource list or detail and dispatches nowhere on Home, so
/// Home's palette must not name it.
///
/// The second question is answered against [`context_bindings`] rather than
/// against a hand-kept list of "which commands are resource commands", because
/// `context_bindings` is the catalogue the parity harness already holds to
/// `keymap.rs`. A key taught here is therefore a key that harness has proved
/// dispatches in the view underneath.
pub fn command_key(
    command: CommandDescriptor,
    root: &RootState,
    overrides: &KeyOverrides,
) -> Option<String> {
    let binding = binding_for_command(command)?;
    context_bindings(normal_view_context(root))
        .contains(&binding)
        .then(|| display_binding(&binding, overrides).key)
}

/// The catalog binding that runs `command`, ignoring context.
///
/// Exhaustive with no catch-all, so a new descriptor cannot be added without
/// answering "does this have a key?". A `_ => None` arm would answer it
/// silently and wrongly for exactly the case that matters: a command that
/// gains a key and keeps a palette row that no longer teaches it.
fn binding_for_command(command: CommandDescriptor) -> Option<Binding> {
    match command {
        CommandDescriptor::GoHome => Some(NAV_HOME),
        CommandDescriptor::GoToConfigurations => Some(NAV_CONFIGURATIONS),
        CommandDescriptor::GoToTunnels => Some(NAV_TUNNELS),
        CommandDescriptor::GoToFinOps => Some(NAV_FINOPS),
        CommandDescriptor::OpenProjectLogs => Some(NAV_LOGGING),
        CommandDescriptor::RefreshCurrentView => Some(GLOBAL_REFRESH),
        CommandDescriptor::ShowHelp => Some(GLOBAL_HELP),
        CommandDescriptor::OpenLogsForSelected(_) => Some(RES_SELECTION_LOGS),
        // Both tunnel commands, because `t` is one key over the active
        // service's selection (`keymap::resource_content_key` dispatches
        // `RequestTunnel`, and the reducer picks the plan from the service).
        CommandDescriptor::StartTunnelForSelectedComputeInstance
        | CommandDescriptor::StartTunnelForSelectedCloudSqlInstance => Some(RES_START_TUNNEL),
        CommandDescriptor::ActivateSelectedConfiguration => Some(CFG_ACTIVATE),
        // `L` in a Cloud Run detail pane dispatches `OpenLogsForSelection`,
        // which the reducer scopes to the pane's target — not necessarily the
        // revision row this descriptor names. Close enough to be tempting and
        // not close enough to promise.
        CommandDescriptor::OpenLogsForSelectedCloudRunRevision => None,
        // `g` opens the Resources workspace, not a service within it: the
        // service switch is the navigator (`v`, then `Enter`) or this palette
        // row. Naming `g` here would promise a key that lands somewhere else.
        CommandDescriptor::GoToService(_) => None,
        // A plugin's key is a character from the operator's file behind the
        // `,` prefix, so the key that runs *this* plugin is runtime data the
        // catalogue cannot name. The status-line prompt lists them while the
        // prefix is armed.
        CommandDescriptor::Plugin(_) => None,
        // Palette-only, every one of them. `y` in a detail pane copies the
        // *selected field*, which is a different action from these named
        // field copies, and the lifecycle actions have no key by design: they
        // mutate, and the palette's confirmation is the point.
        CommandDescriptor::StartSelectedComputeInstance
        | CommandDescriptor::StopSelectedComputeInstance
        | CommandDescriptor::ResetSelectedComputeInstance
        | CommandDescriptor::SuspendSelectedComputeInstance
        | CommandDescriptor::ResumeSelectedComputeInstance
        | CommandDescriptor::ShowSelectedComputeInstanceSsh
        | CommandDescriptor::SshSelectedComputeInstance
        | CommandDescriptor::CopySelectedInstanceName
        | CommandDescriptor::CopySelectedInstanceZone
        | CommandDescriptor::CopySelectedInstanceInternalIp
        | CommandDescriptor::CopySelectedInstanceExternalIp
        | CommandDescriptor::CopySelectedInstanceSelfLink
        | CommandDescriptor::StartSelectedCloudSqlInstance
        | CommandDescriptor::StopSelectedCloudSqlInstance
        | CommandDescriptor::RestartSelectedCloudSqlInstance
        | CommandDescriptor::FailoverSelectedCloudSqlInstance
        | CommandDescriptor::CopySelectedCloudSqlConnectionName
        | CommandDescriptor::OpenSelectedGkeClusterInK9s
        | CommandDescriptor::ReloadConfig
        | CommandDescriptor::ShowPluginOutput
        | CommandDescriptor::SwitchProjectContext => None,
    }
}

/// Builds the grouped help model for the view currently underlying root
/// state (ignoring transient overlays such as the help toggle itself), using
/// the default (no-override) key catalog.
pub fn help_model(root: &RootState) -> HelpModel {
    help_model_with(root, &KeyOverrides::empty())
}

/// Override-aware variant of [`help_model`]: the same grouped model, but with
/// rebindable bindings' displayed keys reflecting the active overrides, so
/// the help view shows rebound keys.
/// Whether `service` answers `binding`, for the five bindings the catalogue
/// offers on every service and the reducer answers on two or three.
///
/// The catalogue is one flat list per [`HelpContext`], and `HelpContext` has no
/// service dimension, so this is where the service is folded in rather than
/// splitting the arm sixteen ways. Every other binding passes through: the
/// filter can only ever remove one of these five, which is what
/// `every_service_keeps_the_bindings_no_service_scopes` pins.
///
/// The predicates live in [`crate::app::service_capabilities`] beside the
/// argument for why they are exhaustive matches.
fn answered_by(binding: &Binding, service: ResourceServiceKind) -> bool {
    use crate::app::service_capabilities::{browses_objects, supports_marks, supports_tunnel};

    if *binding == MARK_TOGGLE
        || *binding == MARK_RANGE
        || *binding == MARK_CLEAR
        || *binding == MARK_UNMARK_VISIBLE
    {
        return supports_marks(service);
    }
    if *binding == RES_START_TUNNEL {
        return supports_tunnel(service);
    }
    if *binding == RES_BUCKET_METADATA || *binding == RES_COPY_URI || *binding == RES_NEXT_PAGE {
        return browses_objects(service);
    }
    true
}

pub fn help_model_with(root: &RootState, overrides: &KeyOverrides) -> HelpModel {
    let ctx = normal_view_context(root);
    let service = root.resources.active_service();
    let current_view_bindings: Vec<DisplayBinding> = context_bindings(ctx)
        .into_iter()
        .filter(|binding| !GLOBAL_BINDINGS.contains(binding) && !NAV_BINDINGS.contains(binding))
        .filter(|binding| answered_by(binding, service))
        .map(|binding| display_binding(&binding, overrides))
        .collect();

    let to_display = |bindings: &[Binding]| -> Vec<DisplayBinding> {
        bindings
            .iter()
            .map(|binding| display_binding(binding, overrides))
            .collect()
    };

    HelpModel {
        view_title: view_title(ctx).to_string(),
        problems: root.system.config_problems().to_vec(),
        sections: vec![
            HelpSection {
                title: "Current view",
                bindings: current_view_bindings,
            },
            HelpSection {
                title: "Global",
                bindings: to_display(&[
                    GLOBAL_PALETTE,
                    GLOBAL_REFRESH,
                    GLOBAL_COPY_DIAGNOSTIC,
                    GLOBAL_CANCEL_PLUGIN,
                    GLOBAL_HELP,
                    GLOBAL_QUIT,
                ]),
            },
            HelpSection {
                title: "Navigation",
                bindings: to_display(NAV_BINDINGS),
            },
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::state::root::{RootState, Workspace};

    #[test]
    fn resource_list_current_view_lists_resource_actions_only() {
        let root = RootState {
            workspace: Workspace::Resources,
            ..Default::default()
        };
        let model = help_model(&root);
        let current = model
            .sections
            .iter()
            .find(|s| s.title == "Current view")
            .unwrap();
        let keys: Vec<&str> = current.bindings.iter().map(|b| b.key.as_str()).collect();
        assert!(keys.contains(&"v")); // Toggle sidebar
        assert!(keys.contains(&"Tab")); // Focus sidebar
        assert!(keys.contains(&"/")); // Filter
        assert!(keys.contains(&"s")); // Sort column
        assert!(keys.contains(&"w")); // Wide columns
        assert!(keys.contains(&"←/→")); // Select column
        assert!(keys.contains(&"j/k")); // Move selection
        assert!(keys.contains(&"Enter")); // Open details
        // Esc and Alt-← do the same thing here, so they are one entry.
        assert!(keys.contains(&"Esc/Alt-←")); // Back
        assert!(!keys.contains(&"Alt-←")); // not listed twice
        assert!(keys.contains(&"Alt-→")); // History forward
        assert!(!current.bindings.iter().any(|b| b.key == "?")); // help is global, not current-view
        assert!(!current.bindings.iter().any(|b| b.key == "q")); // quit is global, not current-view
    }

    #[test]
    fn help_model_always_has_global_and_navigation_sections() {
        let model = help_model(&RootState::default());
        let titles: Vec<_> = model.sections.iter().map(|s| s.title).collect();
        assert!(titles.contains(&"Global"));
        assert!(titles.contains(&"Navigation"));
    }

    /// A global binding is filtered out of the "Current view" section, so the
    /// Global section is the overlay's only chance to teach it. `Ctrl-x` kills
    /// a child the operator started: a key with that consequence that no screen
    /// names is one the operator can only learn by accident.
    #[test]
    fn help_model_global_section_teaches_cancelling_a_running_plugin() {
        let model = help_model(&RootState::default());
        let global = model
            .sections
            .iter()
            .find(|section| section.title == "Global")
            .expect("the Global section is always present");
        assert!(
            global
                .bindings
                .iter()
                .any(|binding| binding.key == GLOBAL_CANCEL_PLUGIN.key
                    && binding.label == GLOBAL_CANCEL_PLUGIN.label),
            "the help overlay must name Ctrl-x: {:?}",
            global.bindings
        );
    }

    #[test]
    fn help_model_has_no_problems_when_state_recorded_none() {
        assert!(help_model(&RootState::default()).problems.is_empty());
    }

    #[test]
    fn help_model_carries_the_recorded_config_problems_in_order() {
        let mut root = RootState::default();
        root.system.set_config_problems(vec![
            "columns.run: unknown key".to_string(),
            "columns.sql: duplicate key".to_string(),
        ]);
        assert_eq!(
            help_model(&root).problems,
            vec![
                "columns.run: unknown key".to_string(),
                "columns.sql: duplicate key".to_string(),
            ]
        );
    }

    #[test]
    fn help_model_with_overrides_also_carries_config_problems() {
        let mut root = RootState::default();
        root.system
            .set_config_problems(vec!["config load failed".to_string()]);
        let model = help_model_with(&root, &KeyOverrides::empty());
        assert_eq!(model.problems, vec!["config load failed".to_string()]);
    }

    fn current_view_keys(root: &RootState) -> Vec<String> {
        help_model(root)
            .sections
            .iter()
            .find(|s| s.title == "Current view")
            .map(|s| s.bindings.iter().map(|b| b.key.clone()).collect())
            .unwrap_or_default()
    }

    #[test]
    fn help_model_current_view_is_mode_aware_across_workspaces() {
        let resources = current_view_keys(&RootState {
            workspace: Workspace::Resources,
            ..Default::default()
        });
        let configurations = current_view_keys(&RootState {
            workspace: Workspace::Configurations,
            ..Default::default()
        });

        // Each view teaches its own actions, not a shared static list.
        let has = |keys: &[String], k: &str| keys.iter().any(|key| key == k);
        assert!(has(&resources, "v")); // Toggle sidebar (resources only)
        assert!(has(&resources, "s")); // Sort column (resources only)
        assert!(has(&configurations, "a")); // Create configuration (configs only)
        assert!(has(&configurations, "e")); // Edit active context (configs only)
        assert!(!has(&configurations, "v"));
        assert!(!has(&resources, "a"));
        assert_ne!(resources, configurations);
    }

    #[test]
    fn help_model_home_view_teaches_no_view_specific_actions() {
        // Home has only global/navigation/history actions in its "Current
        // view" set; the history back/forward hints are the only non-global,
        // non-nav bindings there, so its set is not fully empty but must
        // still be distinct from the resource/configuration views.
        let home = current_view_keys(&RootState {
            workspace: Workspace::Home,
            ..Default::default()
        });
        let resources = current_view_keys(&RootState {
            workspace: Workspace::Resources,
            ..Default::default()
        });
        assert!(!home.iter().any(|k| k == "v"));
        assert!(!home.iter().any(|k| k == "a"));
        assert!(!resources.is_empty());
        assert_ne!(home, resources);
    }

    #[test]
    fn select_labels_are_mode_aware_between_palette_and_lists() {
        // The command palette is a text-filter, so bare j/k type and only the
        // arrow keys select — it must advertise "↑/↓ Select", never "j/k".
        assert_eq!(PALETTE_SELECT.key, "↑/↓");
        assert!(
            !PALETTE_SELECT
                .triggers
                .iter()
                .any(|t| matches!(t, KeyTrigger::Char('j') | KeyTrigger::Char('k')))
        );

        // Normal list/resource views keep vim-style j/k selection.
        assert_eq!(LIST_SELECT.key, "j/k");
        assert!(
            LIST_SELECT
                .triggers
                .iter()
                .any(|t| matches!(t, KeyTrigger::Char('j')))
        );

        // The palette context must not surface a j/k Select binding.
        assert!(
            !context_bindings(HelpContext::CommandPalette)
                .iter()
                .any(|b| b.key == "j/k")
        );
    }

    #[test]
    fn logging_list_and_detail_bindings_differ() {
        let list = context_bindings(HelpContext::LoggingList);
        let detail = context_bindings(HelpContext::LoggingDetail);
        assert_ne!(list, detail);

        // List: Enter opens the entry, j/k move selection, no close/scroll.
        assert!(list.contains(&LOG_OPEN));
        assert!(list.iter().any(|b| b.key == "j/k"));
        assert!(!list.contains(&LOG_CLOSE));
        assert!(!list.contains(&LOG_SCROLL));

        // Detail: Enter/Esc close the entry, PgUp/PgDn scroll, no list
        // selection or "Open entry" binding.
        assert!(detail.contains(&LOG_CLOSE));
        assert!(detail.contains(&LOG_SCROLL));
        assert!(!detail.contains(&LOG_OPEN));
        assert!(!detail.iter().any(|b| b.key == "j/k"));
    }

    #[test]
    fn logging_list_teaches_floor_bucket_and_save_keys() {
        let list = context_bindings(HelpContext::LoggingList);
        assert!(list.contains(&LOG_SEVERITY_UP));
        assert!(list.contains(&LOG_SEVERITY_DOWN));
        assert!(list.contains(&LOG_SINCE_BUCKET));
        assert!(list.contains(&LOG_SAVE));

        // The since buckets are one catalog entry covering every digit.
        assert_eq!(LOG_SINCE_BUCKET.key, "0-6");
        assert_eq!(LOG_SINCE_BUCKET.triggers.len(), 7);

        // The log detail view supports none of them.
        let detail = context_bindings(HelpContext::LoggingDetail);
        for binding in [
            LOG_SEVERITY_UP,
            LOG_SEVERITY_DOWN,
            LOG_SINCE_BUCKET,
            LOG_SAVE,
        ] {
            assert!(
                !detail.contains(&binding),
                "{:?} must not be taught in the log detail view",
                binding.label
            );
        }
    }

    #[test]
    fn help_quit_twin_is_not_treated_as_rebindable_quit() {
        // HELP_QUIT shares every display field with the rebindable GLOBAL_QUIT
        // (a structural match cannot tell them apart)...
        assert_eq!(GLOBAL_QUIT.key, HELP_QUIT.key);
        assert_eq!(GLOBAL_QUIT.label, HELP_QUIT.label);
        assert_eq!(GLOBAL_QUIT.triggers, HELP_QUIT.triggers);

        // ...but the intrinsic tag keeps them apart: only GLOBAL_QUIT is the
        // rebindable Quit; HELP_QUIT (the Help overlay's hardcoded `q`) is not.
        assert_eq!(
            rebindable_action_of(&GLOBAL_QUIT),
            Some(RebindableAction::Quit)
        );
        assert_eq!(rebindable_action_of(&HELP_QUIT), None);

        // End to end: with quit rebound to `x`, the Help context still shows
        // `q` for its quit control, matching keymap::help_key's hardcoded `q`.
        let mut raw = std::collections::BTreeMap::new();
        raw.insert("quit".to_string(), "x".to_string());
        let overrides = KeyOverrides::from_file(&raw).expect("quit override should be valid");
        let help = context_bindings_with(HelpContext::Help, &overrides);
        let quit = help
            .iter()
            .find(|binding| binding.label == "Quit")
            .expect("Help context lists a Quit control");
        assert_eq!(
            quit.key, "q",
            "Help overlay quit must stay `q` under a quit override"
        );
    }

    #[test]
    fn context_bindings_are_unique_per_context() {
        for ctx in ALL_HELP_CONTEXTS {
            let mut keys: Vec<_> = context_bindings(*ctx).iter().map(|b| b.key).collect();
            let before = keys.len();
            keys.sort_unstable();
            keys.dedup();
            assert_eq!(keys.len(), before, "duplicate key in {ctx:?}");
        }
    }

    /// No view teaches going back under two entries. The help overlay used to
    /// read `q/Esc Back` and then `Alt-← Back`, which reads as two different
    /// things for what is one history step; the keys share one entry now.
    #[test]
    fn no_view_lists_back_twice() {
        for ctx in ALL_HELP_CONTEXTS {
            let backs: Vec<_> = context_bindings(*ctx)
                .iter()
                .filter(|binding| binding.label == "Back")
                .map(|binding| binding.key)
                .collect();
            assert!(backs.len() <= 1, "{ctx:?} lists Back as {backs:?}");
        }
    }

    /// Every `view_title` arm, as one table.
    ///
    /// `view_title` ends in `_ => "Help"`, so a deleted arm is not a compile
    /// error — it silently retitles a view with the generic fallback. That has
    /// now been found twice by the mutation gate: the `ResourceDrilldown` arm
    /// during the BigQuery slice, and the `ResourceDetail` arm during the help
    /// work, which had
    /// never been pinned by anything at all. Two instances of one class is the
    /// point at which pinning the class beats pinning the instance.
    ///
    /// The assertion is the whole mapping rather than a list of "these must
    /// not say Help", because the catch-all is *correct* for most contexts —
    /// a confirmation modal or a filter prompt has no view of its own to name.
    /// A set-membership test would therefore have to enumerate the exceptions
    /// anyway, and would still pass on an arm that survived with the wrong
    /// string.
    ///
    /// Both directions fail deliberately. Delete an arm and its context drops
    /// to `"Help"` against the table. Add one and a context that used to fall
    /// through no longer does — which is a decision about what the overlay
    /// calls a view, and should be made by editing this table rather than by
    /// noticing later.
    #[test]
    fn every_view_title_arm_is_pinned() {
        use HelpContext::*;

        // The catch-all's own answer. Named once so the table below cannot
        // accidentally agree with it through a typo.
        const FALLBACK: &str = "Help";

        let titled: &[(HelpContext, &str)] = &[
            (Home, "Home"),
            (ConfigList, "Configurations"),
            (ResourceList, "Resources"),
            (ResourceDetail, "Resource detail"),
            (ResourceDrilldown, "Resource drill-down"),
            (ResourceDrilldownLeaf, "Resource drill-down"),
            (LoggingList, "Logging"),
            (LoggingDetail, "Log detail"),
            (LoggingFilter, "Filter logs"),
            (TunnelsList, "Tunnels"),
            (FinOps, "FinOps"),
        ];

        for (ctx, expected) in titled {
            let actual = view_title(*ctx);
            assert_eq!(
                actual,
                *expected,
                "{ctx:?} is titled {actual:?}; {}",
                if actual == FALLBACK {
                    "that is the catch-all, so its arm has been deleted"
                } else {
                    "the table says otherwise"
                }
            );
        }

        // Everything else falls through on purpose: a modal, a prompt or a
        // picker is drawn over a view rather than being one.
        for ctx in ALL_HELP_CONTEXTS {
            if titled.iter().any(|(titled, _)| titled == ctx) {
                continue;
            }
            assert_eq!(
                view_title(*ctx),
                FALLBACK,
                "{ctx:?} has grown a title of its own without being added to \
                 the table above, so nothing pins it"
            );
        }
    }
}
