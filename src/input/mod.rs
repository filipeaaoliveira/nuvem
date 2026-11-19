//! Translation of raw terminal input into semantic actions.
//!
//! This module defines the context and helper types — chiefly [`Mode`] —
//! and `keymap.rs` maps `crossterm` key events to [`crate::app::Action`]
//! values against them.
//!
//! Mapping answers "what did the user ask to do?" and stops there; "how should
//! state change?" is the reducer's answer. Every mode maps keys explicitly
//! rather than falling through to normal mode, because a mode that borrows
//! another's bindings is how a filter input swallows a global key, or fails
//! to.

pub mod keymap;

use crate::app::Workspace;
use crate::config::hotkeys::KeyOverrides;

/// Declares [`Mode`] and derives its `ALL` from the same variant list, so a
/// caller that must cover every mode walks the enum instead of re-listing it.
///
/// The generated `ALL` is the only item this produces. Every other use of
/// `Mode` is an exhaustive `match`, which the compiler already forces open
/// when a variant appears — but a hand-written array is not a `match`, and a
/// walk built from one goes on passing while it silently stops covering the
/// new variant. This is the same shape, and the same reason, as
/// `lifecycle_actions!` in `app::state::resources`.
macro_rules! modes {
    (
        $(#[$enum_meta:meta])*
        pub enum $name:ident {
            $($(#[$variant_meta:meta])* $variant:ident),+ $(,)?
        }
    ) => {
        $(#[$enum_meta])*
        pub enum $name {
            $($(#[$variant_meta])* $variant,)+
        }

        impl $name {
            /// Every mode, in declaration order, generated from the enum's own
            /// variant list.
            pub const ALL: [Self; [$(stringify!($variant)),+].len()] =
                [$(Self::$variant),+];
        }
    };
}

modes! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum Mode {
        Normal,
        Help,
        Filter,
        CommandPalette,
        EditConfiguration,
        ConfigurationContextPicker,
        ConfirmConfigurationActivation,
        ConfirmResourceAction,
        ResourceCommandPreview,
        SshLaunchPreview,
        TunnelLaunchPreview,
        ConfirmK9sLaunch,
        /// A plugin's argv, awaiting confirmation. Distinct from
        /// `ResourceCommandPreview`, whose `Enter` copies: here it runs.
        PluginConfirmation,
        /// `,` is armed and the next character is read as a plugin key.
        PluginPrefix,
        /// The typed-input prompt: one field per input a plugin's
        /// templates name, collected before the confirmation is composed.
        PluginPrompt,
        /// The related-resource jump picker (`J`).
        ResourceJumpPicker,
        /// The plugin output viewer: a scrollable read of what a
        /// run printed, paging between the runs the log still holds.
        PluginOutput,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Context {
    pub mode: Mode,
    pub workspace: Workspace,
    pub resources_detail_open: bool,
    /// A drill-down list is on screen inside the resources workspace — today
    /// only BigQuery's tables view. It routes `Esc` to
    /// [`crate::app::action::ResourcesAction::LeaveDrillDown`] instead of to
    /// history navigation,
    /// so leaving a dataset lands on the datasets list rather than wherever
    /// the operator was before the workspace.
    pub resources_drilldown_open: bool,
    /// Whether the drill-down on screen has another level *below* it — true in
    /// BigQuery's tables view, where `Enter` opens the selected table's column
    /// schema, and false in the schema view, where a column is the bottom of
    /// the hierarchy.
    ///
    /// Meaningless unless `resources_drilldown_open` is true, and false when it
    /// is false, for the reason `plugin_confirmation_ack_satisfied` gives: the
    /// arms that read this require the flag above first, so a default of true
    /// would make the pair's meaning depend on which is read first.
    ///
    /// It exists because `HelpContext` is derived from this struct, and one
    /// drill-down flag with no depth is why the help overlay taught
    /// `Open` on a column row for three slices. The frame
    /// footer never had the problem: it reads live state through
    /// `drilldown_opens_another_level` rather than a context.
    pub resources_drilldown_opens_another_level: bool,
    pub service_filter_open: bool,
    pub service_sidebar_visible: bool,
    pub service_navigator_focused: bool,
    pub resources_filter_open: bool,
    pub configurations_filter_open: bool,
    pub configurations_filter_draft: String,
    pub configuration_create_open: bool,
    pub configuration_context_edit_open: bool,
    pub configuration_activation_confirmation_open: bool,
    pub configuration_activation_confirmation_preview_visible: bool,
    pub configuration_context_picker_open: bool,
    pub configuration_context_picker_filter_open: bool,
    pub configuration_context_picker_filter_draft: String,
    pub command_palette_open: bool,
    pub resource_action_confirmation_open: bool,
    pub resource_action_confirmation_preview_visible: bool,
    pub resource_batch_confirmation_open: bool,
    pub resource_batch_confirmation_visible: bool,
    pub resource_batch_ack_satisfied: bool,
    pub resource_command_preview_open: bool,
    pub resource_command_preview_visible: bool,
    pub ssh_launch_preview_open: bool,
    pub ssh_launch_preview_visible: bool,
    pub tunnel_launch_preview_open: bool,
    pub tunnel_launch_preview_visible: bool,
    pub logging_filter_open: bool,
    pub logging_detail_open: bool,
    pub logging_filter_draft: String,
    pub k9s_launch_confirmation_open: bool,
    pub k9s_launch_confirmation_preview_visible: bool,
    pub resource_jump_picker_open: bool,
    pub plugin_confirmation_open: bool,
    /// Whether the typed acknowledgement a `dangerous` plugin demands is
    /// satisfied. Always true for an ordinary plugin, and false when no
    /// confirmation is open at all: `plugin_confirmation_open` is what the
    /// dispatch arm requires before this flag is ever read, so a default of
    /// true would make the pair's meaning depend on which of the two is read
    /// first — and this flag is read on its own, one arm deeper.
    pub plugin_confirmation_ack_satisfied: bool,
    /// Whether the confirmation's own modal fits the terminal, i.e. whether the
    /// operator can actually read the argv. False when it does not fit, where
    /// `ui::render_plugin_confirmation` draws the "needs a larger terminal"
    /// message instead — and a plugin's `Enter` runs the command, so it is
    /// gated on this the way the previews above are.
    pub plugin_confirmation_visible: bool,
    pub plugin_prompt_open: bool,
    /// Whether every field holds a value, i.e. whether `Submit` may be
    /// dispatched. False when no prompt is open at all, for the reason
    /// `plugin_confirmation_ack_satisfied` is: the dispatch arm requires
    /// `plugin_prompt_open` first, and this flag is read one arm deeper.
    pub plugin_prompt_satisfied: bool,
    /// Whether the prompt's modal fits the terminal. `Enter` composes an argv
    /// the operator is about to be shown, and composes it from values they can
    /// only check on a screen large enough to draw them, so it is gated on this
    /// the way the confirmation's `Enter` is.
    pub plugin_prompt_visible: bool,
    /// Whether the plugin output viewer is open.
    pub plugin_output_open: bool,
    /// Effective user hotkey overrides for rebindable normal-mode actions.
    pub hotkeys: KeyOverrides,
}
