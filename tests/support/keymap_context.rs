//! Shared support for building the keymap `Context` a `HelpContext` stands for.
//!
//! `Context` has no `Default` and no public constructor, so any test that wants
//! to probe `keymap::action_for_key` in a particular context has to name every
//! field. `base_context` does that once, and `context_for` flips only the flags
//! a given `HelpContext` needs, so a probe lands on the intended branch.
//!
//! It sets the flag the mode is *derived* from rather than mirroring whatever
//! the dispatch arm happens to guard on, and those stopped being the same thing
//! after the guard sweep: most arms now carry no guard at all, because
//! `services::runtime::input_context` assigns each of those modes only under
//! that flag. Setting the flag anyway is what keeps a hand-built `Context` a
//! state the runtime could actually produce — which is the whole point of the
//! probe — so these stay put, and `Mode::PluginPrefix` below documents the case
//! where there is no flag to set.
//!
//! When a *guarded* arm changes — `Mode::Filter` and `Mode::EditConfiguration`
//! are the two left — `context_for` is the one place that has to follow it.
//!
//! `tests/keymap_help_parity_test.rs` uses these to prove the help catalog and
//! the keymap agree across every context;
//! `tests/logviewer_debt_evidence.rs` uses them to probe the
//! `Ctrl-<letter>` sweep; `tests/keymap_modifier_evidence.rs` uses them
//! to probe the chord leaks that sweep's widening found. All three pull this
//! file in with
//! `#[path = "support/keymap_context.rs"]`, so it lives in a subdirectory: a
//! plain `tests/*.rs` file would become a test target of its own.
//!
//! Each binary uses a different subset of what is here — today both reach it
//! through `context_for`, so `base_context` would be reported as dead code in
//! both — hence the crate-level allow rather than a narrower one.
#![allow(dead_code)]

use nuvem::app::Workspace;
use nuvem::app::projection::binding::HelpContext;
use nuvem::config::hotkeys::KeyOverrides;
use nuvem::input::{Context, Mode};

/// Baseline context with every flag false / mode Normal / workspace Home.
/// `context_for` starts from this and flips only the flags the target
/// `HelpContext` needs, mirroring the guards in `src/input/keymap.rs`.
pub(crate) fn base_context() -> Context {
    Context {
        mode: Mode::Normal,
        workspace: Workspace::Home,
        resources_detail_open: false,
        resources_drilldown_open: false,
        resources_drilldown_opens_another_level: false,
        service_filter_open: false,
        service_sidebar_visible: false,
        service_navigator_focused: false,
        resources_filter_open: false,
        configurations_filter_open: false,
        configurations_filter_draft: String::new(),
        configuration_create_open: false,
        configuration_context_edit_open: false,
        configuration_activation_confirmation_open: false,
        configuration_activation_confirmation_preview_visible: false,
        configuration_context_picker_open: false,
        configuration_context_picker_filter_open: false,
        configuration_context_picker_filter_draft: String::new(),
        command_palette_open: false,
        resource_action_confirmation_open: false,
        resource_action_confirmation_preview_visible: false,
        resource_command_preview_open: false,
        resource_command_preview_visible: false,
        ssh_launch_preview_open: false,
        ssh_launch_preview_visible: false,
        tunnel_launch_preview_open: false,
        tunnel_launch_preview_visible: false,
        logging_filter_open: false,
        logging_detail_open: false,
        logging_filter_draft: String::new(),
        k9s_launch_confirmation_open: false,
        k9s_launch_confirmation_preview_visible: false,
        resource_jump_picker_open: false,
        resource_batch_confirmation_open: false,
        resource_batch_confirmation_visible: false,
        resource_batch_ack_satisfied: false,
        plugin_confirmation_open: false,
        plugin_confirmation_ack_satisfied: false,
        plugin_confirmation_visible: false,
        plugin_prompt_open: false,
        plugin_prompt_satisfied: false,
        plugin_prompt_visible: false,
        plugin_output_open: false,
        hotkeys: KeyOverrides::empty(),
    }
}

/// Builds the keymap `Context` that corresponds to a given `HelpContext`,
/// mirroring the dispatch guards in `src/input/keymap.rs::action_for_key`
/// exactly, so probing hits the intended branch.
///
/// Some arms restate a value `base_context` already set — `Mode::Normal`,
/// `resources_detail_open = false`, and the "picker/filter is *not* open"
/// flags. Those are kept deliberately: they are the guard the matching arm in
/// `keymap.rs` actually tests, and writing them out is what makes an arm here
/// readable against the arm there without holding the baseline in your head.
pub(crate) fn context_for(ctx: HelpContext) -> Context {
    let mut c = base_context();
    match ctx {
        HelpContext::Home => {
            c.mode = Mode::Normal;
            c.workspace = Workspace::Home;
        }
        HelpContext::ConfigList => {
            c.mode = Mode::Normal;
            c.workspace = Workspace::Configurations;
        }
        HelpContext::ConfigFilter => {
            c.mode = Mode::Filter;
            c.workspace = Workspace::Configurations;
            c.configurations_filter_open = true;
        }
        HelpContext::ConfigCreate => {
            c.mode = Mode::EditConfiguration;
            c.workspace = Workspace::Configurations;
            c.configuration_create_open = true;
        }
        HelpContext::ConfigContextEdit => {
            c.mode = Mode::EditConfiguration;
            c.workspace = Workspace::Configurations;
            c.configuration_context_edit_open = true;
            c.configuration_context_picker_open = false;
        }
        HelpContext::ConfigContextPicker => {
            c.mode = Mode::ConfigurationContextPicker;
            c.workspace = Workspace::Configurations;
            c.configuration_context_picker_open = true;
            c.configuration_context_picker_filter_open = false;
        }
        HelpContext::ConfigContextPickerFilter => {
            c.mode = Mode::ConfigurationContextPicker;
            c.workspace = Workspace::Configurations;
            c.configuration_context_picker_open = true;
            c.configuration_context_picker_filter_open = true;
        }
        HelpContext::ResourceList => {
            c.mode = Mode::Normal;
            c.workspace = Workspace::Resources;
            c.resources_detail_open = false;
        }
        HelpContext::ResourceDetail => {
            c.mode = Mode::Normal;
            c.workspace = Workspace::Resources;
            c.resources_detail_open = true;
        }
        // The flag the mode is derived from: `input_context` sets
        // `resources_drilldown_open` from `is_showing_drilldown`, and the
        // keymap's `Esc` arm is one of the few that still reads its guard
        // directly.
        HelpContext::ResourceDrilldown => {
            c.mode = Mode::Normal;
            c.workspace = Workspace::Resources;
            c.resources_detail_open = false;
            c.resources_drilldown_open = true;
            c.resources_drilldown_opens_another_level = true;
        }
        // The bottom of the drill. The pair of flags is what the two arms of
        // the catalogue are keyed on, and the parity harness has to be able to
        // reach both or the leaf arm is enumerated by `ALL_HELP_CONTEXTS` and
        // probed against a context that is not it.
        HelpContext::ResourceDrilldownLeaf => {
            c.mode = Mode::Normal;
            c.workspace = Workspace::Resources;
            c.resources_detail_open = false;
            c.resources_drilldown_open = true;
            c.resources_drilldown_opens_another_level = false;
        }
        HelpContext::ResourceNavigator => {
            c.mode = Mode::Normal;
            c.workspace = Workspace::Resources;
            c.resources_detail_open = false;
            c.service_sidebar_visible = true;
            c.service_navigator_focused = true;
        }
        HelpContext::ResourceFilter => {
            c.mode = Mode::Filter;
            c.workspace = Workspace::Resources;
            c.resources_filter_open = true;
        }
        HelpContext::ServiceFilter => {
            c.mode = Mode::Filter;
            c.workspace = Workspace::Resources;
            c.service_filter_open = true;
        }
        HelpContext::LoggingList => {
            c.mode = Mode::Normal;
            c.workspace = Workspace::Logging;
            c.logging_detail_open = false;
        }
        HelpContext::LoggingDetail => {
            c.mode = Mode::Normal;
            c.workspace = Workspace::Logging;
            c.logging_detail_open = true;
        }
        HelpContext::LoggingFilter => {
            c.mode = Mode::Filter;
            c.workspace = Workspace::Logging;
            c.logging_filter_open = true;
        }
        HelpContext::TunnelsList => {
            c.mode = Mode::Normal;
            c.workspace = Workspace::Tunnels;
        }
        HelpContext::FinOps => {
            c.mode = Mode::Normal;
            c.workspace = Workspace::FinOps;
        }
        HelpContext::CommandPalette => {
            c.mode = Mode::CommandPalette;
            c.command_palette_open = true;
        }
        HelpContext::ConfirmResourceAction => {
            c.mode = Mode::ConfirmResourceAction;
            c.resource_action_confirmation_open = true;
            c.resource_action_confirmation_preview_visible = true;
        }
        HelpContext::ConfirmConfigurationActivation => {
            c.mode = Mode::ConfirmConfigurationActivation;
            c.configuration_activation_confirmation_open = true;
            c.configuration_activation_confirmation_preview_visible = true;
        }
        HelpContext::ResourceCommandPreview => {
            c.mode = Mode::ResourceCommandPreview;
            c.resource_command_preview_open = true;
            c.resource_command_preview_visible = true;
        }
        HelpContext::SshLaunchPreview => {
            c.mode = Mode::SshLaunchPreview;
            c.ssh_launch_preview_open = true;
            c.ssh_launch_preview_visible = true;
        }
        HelpContext::TunnelLaunchPreview => {
            c.mode = Mode::TunnelLaunchPreview;
            c.tunnel_launch_preview_open = true;
            c.tunnel_launch_preview_visible = true;
        }
        // The ordinary (non-`dangerous`) plugin: open, with its acknowledgement
        // already satisfied, which is what `is_ack_satisfied` reports when no
        // typing is demanded. That is the state the catalog's `Enter` entry
        // claims to work in, so it is the state the probe has to be in.
        HelpContext::PluginConfirmation => {
            c.mode = Mode::PluginConfirmation;
            c.workspace = Workspace::Resources;
            c.plugin_confirmation_open = true;
            c.plugin_confirmation_ack_satisfied = true;
            // The catalog's `Enter` entry claims a modal the operator can read,
            // and the keymap gates it on exactly that.
            c.plugin_confirmation_visible = true;
        }
        // Open, every field filled, and the modal drawable — the state the
        // catalog's `Enter` entry claims to work in, and the state the keymap
        // gates it on.
        HelpContext::PluginPrompt => {
            c.mode = Mode::PluginPrompt;
            c.workspace = Workspace::Resources;
            c.plugin_prompt_open = true;
            c.plugin_prompt_satisfied = true;
            c.plugin_prompt_visible = true;
        }
        // No flag to set: the keymap's arm for this mode matches on the mode
        // alone, deliberately, so there is no `Context` field for it
        // to read and none to mirror here.
        HelpContext::PluginPrefix => {
            c.mode = Mode::PluginPrefix;
            c.workspace = Workspace::Resources;
        }
        HelpContext::ResourceJumpPicker => {
            c.mode = Mode::ResourceJumpPicker;
            c.workspace = Workspace::Resources;
            c.resource_jump_picker_open = true;
        }
        HelpContext::Help => {
            c.mode = Mode::Help;
        }
    }
    c
}
