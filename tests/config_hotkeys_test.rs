use std::collections::BTreeMap;

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
use nuvem::app::projection::binding::KeyTrigger;
use nuvem::app::{Action, WorkspaceAction};
use nuvem::config::hotkeys::{KeyOverrides, RebindableAction, parse_key_spec};
use nuvem::input::keymap::action_for_key;
use nuvem::input::{Context, Mode};

fn raw(p: &[(&str, &str)]) -> BTreeMap<String, String> {
    p.iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

#[test]
fn parses_simple_and_ctrl_specs() {
    assert_eq!(parse_key_spec("H").unwrap(), KeyTrigger::Char('H'));
    assert_eq!(parse_key_spec("ctrl-x").unwrap(), KeyTrigger::CtrlChar('x'));
    assert!(parse_key_spec("nope").is_err());
}

#[test]
fn valid_overrides_apply() {
    let o = KeyOverrides::from_file(&raw(&[("home", "H"), ("refresh", "R")])).unwrap();
    assert_eq!(
        o.effective_trigger(RebindableAction::Home),
        KeyTrigger::Char('H')
    );
    assert_eq!(
        o.action_for(KeyTrigger::Char('R')),
        Some(RebindableAction::Refresh)
    );
}

#[test]
fn collision_is_rejected() {
    let err = KeyOverrides::from_file(&raw(&[("home", "x"), ("refresh", "x")])).unwrap_err();
    assert!(
        err.iter()
            .any(|m| m.to_lowercase().contains("collision") || m.contains("x"))
    );
}

#[test]
fn reserved_key_is_rejected() {
    for spec in [
        "enter",
        "esc",
        "tab",
        "backtab",
        "backspace",
        "ctrl-c",
        "ctrl-f",
        "ctrl-s",
        "ctrl-t",
        "ctrl-x",
    ] {
        assert!(
            KeyOverrides::from_file(&raw(&[("home", spec)])).is_err(),
            "{spec} must be reserved"
        );
    }
}

/// The other side of `reserved_key_is_rejected`: a `ctrl-<char>` that is *not*
/// reserved must be genuinely accepted, and land in the effective table as the
/// `CtrlChar` trigger `override_trigger` looks up.
///
/// This is asserted on the `Ok` value rather than left to an `.expect()` in the
/// parity harness, because `parity_holds_under_overrides` now rebinds `refresh`
/// to `ctrl-r` in order to reach `override_trigger`'s Ctrl branch at all. If
/// `ctrl-r` were ever reserved, `from_file` would return `Err`, the harness's
/// sample would fall over on its `.expect`, and the failure would read as a
/// broken test rather than as "the sweep can no longer see the Ctrl branch".
/// Pinning acceptance here says which of the two it is.
#[test]
fn a_non_reserved_ctrl_char_is_accepted_as_an_override_target() {
    let overrides = match KeyOverrides::from_file(&raw(&[("refresh", "ctrl-r")])) {
        Ok(overrides) => overrides,
        Err(errors) => panic!("ctrl-r must be a valid override target, got errors: {errors:?}"),
    };
    assert_eq!(
        overrides.effective_trigger(RebindableAction::Refresh),
        KeyTrigger::CtrlChar('r'),
        "ctrl-r must land in the effective table as CtrlChar('r')"
    );
    assert_eq!(
        overrides.action_for(KeyTrigger::CtrlChar('r')),
        Some(RebindableAction::Refresh),
        "the reverse lookup override_trigger feeds must resolve CtrlChar('r') to Refresh"
    );
}

/// Reserving `ctrl-s` / `ctrl-t` breaks configs that previously "worked" by
/// silently shadowing the app's own dispatch, so the rejection has to explain
/// itself: which key, that it is reserved, and *why*.
#[test]
fn reserved_key_error_explains_why_the_key_is_reserved() {
    let message = |spec: &str| -> String {
        KeyOverrides::from_file(&raw(&[("home", spec)]))
            .unwrap_err()
            .into_iter()
            .find(|m| m.starts_with("hotkeys.home:"))
            .unwrap_or_else(|| panic!("{spec} should produce a hotkeys.home error"))
    };

    // `Ctrl-S` / `Ctrl-T`: the override table is resolved ahead of the app's
    // own arms, so the user's binding wins and steals the app key.
    let ctrl_s = message("ctrl-s");
    assert_eq!(
        ctrl_s,
        "hotkeys.home: ctrl-s is a reserved key — the override table is resolved before the \
         app's own keys, so a binding here would win and silently steal the Logging \
         save-to-file key — no error, no way to tell"
    );

    let ctrl_t = message("ctrl-t");
    assert_eq!(
        ctrl_t,
        "hotkeys.home: ctrl-t is a reserved key — the override table is resolved before the \
         app's own keys, so a binding here would win and silently steal the Tunnels workspace \
         key — no error, no way to tell"
    );

    let ctrl_f = message("ctrl-f");
    assert_eq!(
        ctrl_f,
        "hotkeys.home: ctrl-f is a reserved key — the override table is resolved before the \
         app's own keys, so a binding here would win and silently steal the FinOps workspace \
         key — no error, no way to tell"
    );

    // `Ctrl-C` is the opposite case: it force-quits ahead of every lookup, so a
    // binding onto it would never fire at all.
    assert_eq!(
        message("ctrl-c"),
        "hotkeys.home: ctrl-c is a reserved key — the app returns force-quit for it before \
         any mode or override lookup, so a binding here could never fire"
    );

    // `Ctrl-X` was in the `Ctrl-S` / `Ctrl-T` group until it was hoisted
    // beside `Ctrl-C` in `action_for_key`. It stays reserved, but for the other
    // reason: the override table never gets to see it now. The key it protects
    // is the same, and the reason names it, so a config that reaches for the
    // key is told what it would have cost rather than only that it is refused.
    assert_eq!(
        message("ctrl-x"),
        "hotkeys.home: ctrl-x is a reserved key — the app returns cancel-running-plugins for \
         it before any mode or override lookup, so a binding here could never fire; a plugin \
         that never returns has no other way to be stopped"
    );
    // Named keys are never looked up in the override table at all.
    assert_eq!(
        message("esc"),
        "hotkeys.home: Esc is a reserved key — only single-character keys can be looked up in \
         the override table, so a binding here could never fire; Esc stays the universal modal \
         and filter cancel"
    );
}

/// The required-action check renders the same reason, so a config that makes
/// `quit` unreachable via a reserved key is told which key and why.
#[test]
fn unreachable_required_action_error_explains_the_reserved_key() {
    let errors = KeyOverrides::from_file(&raw(&[("quit", "ctrl-s")])).unwrap_err();
    assert!(
        errors.iter().any(|m| m
            == "hotkeys.quit: required action is unreachable (reserved key ctrl-s — the \
                override table is resolved before the app's own keys, so a binding here would \
                win and silently steal the Logging save-to-file key — no error, no way to tell)"),
        "unexpected errors: {errors:?}"
    );
}

/// A Home / Normal-mode input context with the given overrides and all modal
/// flags cleared.
fn home_context(hotkeys: KeyOverrides) -> Context {
    Context {
        mode: Mode::Normal,
        workspace: nuvem::app::Workspace::Home,
        resources_detail_open: false,
        resources_drilldown_open: false,
        resources_drilldown_opens_another_level: false,
        service_filter_open: false,
        service_sidebar_visible: false,
        service_navigator_focused: false,
        resources_filter_open: false,
        configurations_filter_open: false,
        configurations_filter_draft: String::new(),
        logging_filter_open: false,
        logging_detail_open: false,
        logging_filter_draft: String::new(),
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
        hotkeys,
    }
}

fn press(code: KeyCode) -> KeyEvent {
    KeyEvent {
        code,
        modifiers: KeyModifiers::empty(),
        kind: KeyEventKind::Press,
        state: KeyEventState::empty(),
    }
}

#[test]
fn rebound_key_dispatches_and_default_is_freed() {
    let overrides = KeyOverrides::from_file(&raw(&[("home", "H")])).unwrap();
    let context = home_context(overrides);

    // The rebound key now dispatches Home.
    assert_eq!(
        action_for_key(press(KeyCode::Char('H')), &context),
        Some(Action::Workspace(WorkspaceAction::Home))
    );

    // The default key no longer maps to Home (it has been reassigned away).
    assert_ne!(
        action_for_key(press(KeyCode::Char('h')), &context),
        Some(Action::Workspace(WorkspaceAction::Home))
    );
}
