use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
use nuvem::app::{
    Action, CommandAction, ConfigurationsAction, Effect, JumpsAction, LoggingAction, NavAction,
    NavigatorAction, ResourcesAction, SystemAction, TunnelsAction, Workspace, WorkspaceAction,
};
use nuvem::input::keymap::action_for_key;
use nuvem::input::{Context, Mode};
use nuvem_domain::gcp::logging::LogTimeRange;

// Every `Context` in this file starts from the shared helper and names only the
// fields the case actually varies, so a new `Context` field is named once there
// rather than once per test. `service_sidebar_visible: true` is stated
// explicitly wherever it applies: it gates real dispatch arms, so it is a thing
// a test says, not a thing it inherits.
use crate::support::keymap_context::base_context;

fn normal_resources_list_context() -> Context {
    Context {
        workspace: Workspace::Resources,
        service_sidebar_visible: true,
        ..base_context()
    }
}

fn normal_resources_detail_context() -> Context {
    Context {
        hotkeys: nuvem::config::hotkeys::KeyOverrides::empty(),
        resources_detail_open: true,
        ..normal_resources_list_context()
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

fn modified_press(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
    KeyEvent {
        code,
        modifiers,
        kind: KeyEventKind::Press,
        state: KeyEventState::empty(),
    }
}

fn ctrl_c() -> KeyEvent {
    KeyEvent {
        code: KeyCode::Char('c'),
        modifiers: KeyModifiers::CONTROL,
        kind: KeyEventKind::Press,
        state: KeyEventState::empty(),
    }
}

fn normal_context(workspace: Workspace, detail_open: bool) -> Context {
    Context {
        workspace,
        resources_detail_open: detail_open,
        service_sidebar_visible: true,
        ..base_context()
    }
}

fn context(workspace: Workspace, mode: Mode) -> Context {
    Context {
        mode,
        workspace,
        service_sidebar_visible: true,
        ..base_context()
    }
}

#[test]
fn alt_left_maps_to_navigation_back_in_normal_mode() {
    let ctx = normal_resources_list_context();
    let action = action_for_key(modified_press(KeyCode::Left, KeyModifiers::ALT), &ctx);
    assert_eq!(action, Some(Action::Navigation(NavAction::Back)));
}

#[test]
fn alt_right_maps_to_navigation_forward_in_normal_mode() {
    let ctx = normal_resources_list_context();
    let action = action_for_key(modified_press(KeyCode::Right, KeyModifiers::ALT), &ctx);
    assert_eq!(action, Some(Action::Navigation(NavAction::Forward)));
}

#[test]
fn esc_in_resource_list_maps_to_navigation_back() {
    let ctx = normal_resources_list_context();
    let action = action_for_key(press(KeyCode::Esc), &ctx);
    assert_eq!(action, Some(Action::Navigation(NavAction::Back)));
}

#[test]
fn esc_in_resource_detail_maps_to_navigation_back() {
    let ctx = normal_resources_detail_context();
    let action = action_for_key(press(KeyCode::Esc), &ctx);
    assert_eq!(action, Some(Action::Navigation(NavAction::Back)));
}

#[test]
fn q_in_resource_detail_maps_to_navigation_back() {
    let ctx = normal_resources_detail_context();
    let action = action_for_key(press(KeyCode::Char('q')), &ctx);
    assert_eq!(action, Some(Action::Navigation(NavAction::Back)));
}

#[test]
fn l_in_resource_list_opens_logging_workspace() {
    let ctx = normal_resources_list_context();
    let action = action_for_key(press(KeyCode::Char('l')), &ctx);
    assert_eq!(action, Some(Action::Workspace(WorkspaceAction::Logging)));
}

#[test]
fn l_in_resource_detail_opens_logging_workspace() {
    let ctx = normal_resources_detail_context();
    let action = action_for_key(press(KeyCode::Char('l')), &ctx);
    assert_eq!(action, Some(Action::Workspace(WorkspaceAction::Logging)));
}

#[test]
fn enter_in_resource_detail_maps_to_open_selected() {
    let ctx = normal_resources_detail_context();
    let action = action_for_key(press(KeyCode::Enter), &ctx);
    assert_eq!(
        action,
        Some(Action::Resources(ResourcesAction::OpenSelected))
    );
}

#[test]
fn home_dashboard_keys_do_not_drive_the_hidden_navigator() {
    // Home renders the read-only overview dashboard (issue #27). The service
    // navigator lives in the Resources sidebar now, not on Home, so Home input
    // must NOT emit NavigatorActions (which would drive an invisible list).
    let context = context(Workspace::Home, Mode::Normal);
    assert_eq!(action_for_key(press(KeyCode::Char('/')), &context), None);
    assert_eq!(action_for_key(press(KeyCode::Char('j')), &context), None);
    assert_eq!(action_for_key(press(KeyCode::Char('k')), &context), None);
    assert_eq!(action_for_key(press(KeyCode::Enter), &context), None);
    // The command palette entry point remains available from Home.
    assert_eq!(
        action_for_key(press(KeyCode::Char(':')), &context),
        Some(Action::Command(CommandAction::Open))
    );
    // Workspace navigation is handled globally and still works from Home.
    assert_eq!(
        action_for_key(press(KeyCode::Char('g')), &context),
        Some(Action::Workspace(WorkspaceAction::Resources))
    );
}

#[test]
fn resources_sidebar_keys_are_only_active_when_sidebar_has_focus() {
    let mut context = context(Workspace::Resources, Mode::Normal);
    context.service_sidebar_visible = true;
    context.service_navigator_focused = true;
    assert_eq!(
        action_for_key(press(KeyCode::Char('/')), &context),
        Some(Action::Navigator(NavigatorAction::StartFilter))
    );
    assert_eq!(
        action_for_key(press(KeyCode::Enter), &context),
        Some(Action::Navigator(NavigatorAction::OpenSelected))
    );

    context.service_navigator_focused = false;
    assert_eq!(
        action_for_key(press(KeyCode::Char('/')), &context),
        Some(Action::Resources(ResourcesAction::StartFilter))
    );
}

#[test]
fn resources_v_and_tab_toggle_sidebar_and_focus() {
    let context = context(Workspace::Resources, Mode::Normal);
    assert_eq!(
        action_for_key(press(KeyCode::Char('v')), &context),
        Some(Action::Navigator(NavigatorAction::ToggleSidebar))
    );
    assert_eq!(
        action_for_key(press(KeyCode::Tab), &context),
        Some(Action::Navigator(NavigatorAction::ToggleFocus))
    );
}

#[test]
fn c_opens_configurations_workspace_in_normal_mode() {
    let action = action_for_key(
        press(KeyCode::Char('c')),
        &Context {
            service_sidebar_visible: true,
            ..base_context()
        },
    );
    assert_eq!(
        action,
        Some(Action::Workspace(WorkspaceAction::Configurations))
    );
}

#[test]
fn enter_in_config_activation_confirm_confirms_activation() {
    let action = action_for_key(
        press(KeyCode::Enter),
        &Context {
            mode: Mode::ConfirmConfigurationActivation,
            workspace: Workspace::Configurations,
            service_sidebar_visible: true,
            configuration_activation_confirmation_open: true,
            configuration_activation_confirmation_preview_visible: true,
            ..base_context()
        },
    );
    assert_eq!(
        action,
        Some(Action::Configurations(
            ConfigurationsAction::ConfirmActivation
        ))
    );
}

#[test]
fn y_in_config_activation_confirm_confirms_activation() {
    let action = action_for_key(
        press(KeyCode::Char('y')),
        &Context {
            mode: Mode::ConfirmConfigurationActivation,
            workspace: Workspace::Configurations,
            service_sidebar_visible: true,
            configuration_activation_confirmation_open: true,
            configuration_activation_confirmation_preview_visible: true,
            ..base_context()
        },
    );

    assert_eq!(
        action,
        Some(Action::Configurations(
            ConfigurationsAction::ConfirmActivation
        ))
    );
}

#[test]
fn esc_in_config_activation_confirm_cancels_activation() {
    let action = action_for_key(
        press(KeyCode::Esc),
        &Context {
            mode: Mode::ConfirmConfigurationActivation,
            workspace: Workspace::Configurations,
            service_sidebar_visible: true,
            configuration_activation_confirmation_open: true,
            ..base_context()
        },
    );

    assert_eq!(
        action,
        Some(Action::Configurations(
            ConfigurationsAction::CancelActivation
        ))
    );
}

#[test]
fn n_in_config_activation_confirm_cancels_activation() {
    let action = action_for_key(
        press(KeyCode::Char('n')),
        &Context {
            mode: Mode::ConfirmConfigurationActivation,
            workspace: Workspace::Configurations,
            service_sidebar_visible: true,
            configuration_activation_confirmation_open: true,
            ..base_context()
        },
    );

    assert_eq!(
        action,
        Some(Action::Configurations(
            ConfigurationsAction::CancelActivation
        ))
    );
}

#[test]
fn configuration_activation_confirmation_does_not_confirm_when_preview_is_hidden() {
    let context = Context {
        mode: Mode::ConfirmConfigurationActivation,
        workspace: Workspace::Configurations,
        service_sidebar_visible: true,
        configuration_activation_confirmation_open: true,
        ..base_context()
    };

    assert_eq!(action_for_key(press(KeyCode::Enter), &context), None);
    assert_eq!(action_for_key(press(KeyCode::Char('y')), &context), None);
    assert_eq!(
        action_for_key(press(KeyCode::Esc), &context),
        Some(Action::Configurations(
            ConfigurationsAction::CancelActivation
        ))
    );
    assert_eq!(
        action_for_key(press(KeyCode::Char('n')), &context),
        Some(Action::Configurations(
            ConfigurationsAction::CancelActivation
        ))
    );
}

#[test]
fn key_release_is_ignored() {
    let mut key = press(KeyCode::Char('c'));
    key.kind = KeyEventKind::Release;
    assert_eq!(
        action_for_key(
            key,
            &Context {
                service_sidebar_visible: true,
                ..base_context()
            }
        ),
        None
    );
}

#[test]
fn shift_y_copies_the_diagnostic_from_any_normal_mode_workspace() {
    // Global, like `r`/`?`: reachable regardless of active workspace.
    for workspace in [
        Workspace::Home,
        Workspace::Configurations,
        Workspace::Resources,
    ] {
        let action = action_for_key(
            press(KeyCode::Char('Y')),
            &Context {
                workspace,
                ..normal_resources_list_context()
            },
        );
        assert_eq!(
            action,
            Some(Action::System(SystemAction::CopyDiagnostic)),
            "workspace {workspace:?} must dispatch CopyDiagnostic on Y"
        );
    }
}

#[test]
fn r_refreshes_configurations_workspace() {
    let action = action_for_key(
        press(KeyCode::Char('r')),
        &Context {
            workspace: Workspace::Configurations,
            service_sidebar_visible: true,
            ..base_context()
        },
    );

    assert_eq!(action, Some(Action::Effect(Effect::LoadConfigurations)));
}

#[test]
fn r_refreshes_resources_workspace() {
    let action = action_for_key(
        press(KeyCode::Char('r')),
        &Context {
            workspace: Workspace::Resources,
            service_sidebar_visible: true,
            ..base_context()
        },
    );

    assert_eq!(action, Some(Action::Resources(ResourcesAction::Refresh)));
}

#[test]
fn resource_list_keys_select_and_open_instances() {
    let context = Context {
        workspace: Workspace::Resources,
        service_sidebar_visible: true,
        ..base_context()
    };

    assert_eq!(
        action_for_key(press(KeyCode::Char('j')), &context),
        Some(Action::Resources(ResourcesAction::SelectNext))
    );
    assert_eq!(
        action_for_key(press(KeyCode::Down), &context),
        Some(Action::Resources(ResourcesAction::SelectNext))
    );
    assert_eq!(
        action_for_key(press(KeyCode::Char('k')), &context),
        Some(Action::Resources(ResourcesAction::SelectPrevious))
    );
    assert_eq!(
        action_for_key(press(KeyCode::Up), &context),
        Some(Action::Resources(ResourcesAction::SelectPrevious))
    );
    assert_eq!(
        action_for_key(press(KeyCode::Enter), &context),
        Some(Action::Resources(ResourcesAction::OpenSelected))
    );
}

#[test]
fn v_toggles_resource_service_sidebar() {
    let context = Context {
        workspace: Workspace::Resources,
        service_sidebar_visible: true,
        ..base_context()
    };

    let action = action_for_key(press(KeyCode::Char('v')), &context);

    assert_eq!(
        action,
        Some(Action::Navigator(
            nuvem::app::NavigatorAction::ToggleSidebar
        ))
    );
}

#[test]
fn resource_detail_keys_cycle_tabs_and_close_detail() {
    let context = Context {
        workspace: Workspace::Resources,
        resources_detail_open: true,
        service_sidebar_visible: true,
        ..base_context()
    };

    for key in [KeyCode::Char('j'), KeyCode::Down] {
        assert_eq!(
            action_for_key(press(key), &context),
            Some(Action::Resources(ResourcesAction::SelectNext))
        );
    }
    for key in [KeyCode::Char('k'), KeyCode::Up] {
        assert_eq!(
            action_for_key(press(key), &context),
            Some(Action::Resources(ResourcesAction::SelectPrevious))
        );
    }
    assert_eq!(
        action_for_key(press(KeyCode::Char(']')), &context),
        Some(Action::Resources(ResourcesAction::SelectNextTab))
    );
    assert_eq!(
        action_for_key(press(KeyCode::Char('[')), &context),
        Some(Action::Resources(ResourcesAction::SelectPreviousTab))
    );
    assert_eq!(
        action_for_key(press(KeyCode::PageDown), &context),
        Some(Action::Resources(ResourcesAction::ScrollDetailDown))
    );
    assert_eq!(
        action_for_key(press(KeyCode::PageUp), &context),
        Some(Action::Resources(ResourcesAction::ScrollDetailUp))
    );
    assert_eq!(
        action_for_key(press(KeyCode::Char('v')), &context),
        Some(Action::Navigator(NavigatorAction::ToggleSidebar))
    );
    assert_eq!(
        action_for_key(press(KeyCode::Tab), &context),
        Some(Action::Navigator(NavigatorAction::ToggleFocus))
    );
    assert_eq!(
        action_for_key(press(KeyCode::Esc), &context),
        Some(Action::Navigation(NavAction::Back))
    );
    assert_eq!(
        action_for_key(press(KeyCode::Char('q')), &context),
        Some(Action::Navigation(NavAction::Back))
    );
}

#[test]
fn focused_resource_sidebar_remains_navigable_while_detail_is_open() {
    let mut context = context(Workspace::Resources, Mode::Normal);
    context.resources_detail_open = true;
    context.service_navigator_focused = true;

    for key in [KeyCode::Char('j'), KeyCode::Down] {
        assert_eq!(
            action_for_key(press(key), &context),
            Some(Action::Navigator(NavigatorAction::SelectNext))
        );
    }
    for key in [KeyCode::Char('k'), KeyCode::Up] {
        assert_eq!(
            action_for_key(press(key), &context),
            Some(Action::Navigator(NavigatorAction::SelectPrevious))
        );
    }
    assert_eq!(
        action_for_key(press(KeyCode::Enter), &context),
        Some(Action::Navigator(NavigatorAction::OpenSelected))
    );

    assert_eq!(
        action_for_key(press(KeyCode::Char('v')), &context),
        Some(Action::Navigator(NavigatorAction::ToggleSidebar))
    );
    assert_eq!(
        action_for_key(press(KeyCode::Tab), &context),
        Some(Action::Navigator(NavigatorAction::ToggleFocus))
    );

    context.service_navigator_focused = false;
    assert_eq!(
        action_for_key(press(KeyCode::Char(']')), &context),
        Some(Action::Resources(ResourcesAction::SelectNextTab))
    );
    assert_eq!(
        action_for_key(press(KeyCode::Char('[')), &context),
        Some(Action::Resources(ResourcesAction::SelectPreviousTab))
    );
}

#[test]
fn resource_detail_mode_does_not_trigger_hidden_list_actions() {
    let context = Context {
        workspace: Workspace::Resources,
        resources_detail_open: true,
        service_sidebar_visible: true,
        ..base_context()
    };

    // In detail view j/k/Up/Down move the revision cursor (SelectNext/
    // SelectPrevious) so the Cloud Run revision drill-down can reach every
    // revision; the state routes these to the revision cursor only while on
    // the Revisions tab and otherwise to the underlying row cursor.
    for key in [KeyCode::Char('j'), KeyCode::Down] {
        assert_eq!(
            action_for_key(press(key), &context),
            Some(Action::Resources(ResourcesAction::SelectNext))
        );
    }
    for key in [KeyCode::Char('k'), KeyCode::Up] {
        assert_eq!(
            action_for_key(press(key), &context),
            Some(Action::Resources(ResourcesAction::SelectPrevious))
        );
    }
    // Enter in detail drives the revision drill-down (OpenSelected).
    assert_eq!(
        action_for_key(press(KeyCode::Enter), &context),
        Some(Action::Resources(ResourcesAction::OpenSelected))
    );
}

#[test]
fn resource_list_mode_does_not_trigger_detail_actions() {
    let context = Context {
        workspace: Workspace::Resources,
        service_sidebar_visible: true,
        ..base_context()
    };

    assert_eq!(action_for_key(press(KeyCode::Char('[')), &context), None);
    assert_eq!(action_for_key(press(KeyCode::Char(']')), &context), None);
    assert_eq!(
        action_for_key(press(KeyCode::Esc), &context),
        Some(Action::Navigation(NavAction::Back))
    );
}

#[test]
fn q_quits_from_resource_list_but_closes_resource_detail() {
    assert_eq!(
        action_for_key(
            press(KeyCode::Char('q')),
            &Context {
                workspace: Workspace::Resources,
                service_sidebar_visible: true,
                ..base_context()
            },
        ),
        Some(Action::QuitRequested)
    );

    assert_eq!(
        action_for_key(
            press(KeyCode::Char('q')),
            &Context {
                workspace: Workspace::Resources,
                resources_detail_open: true,
                service_sidebar_visible: true,
                ..base_context()
            },
        ),
        Some(Action::Navigation(NavAction::Back))
    );
}

#[test]
fn resource_list_keys_filter_sort_and_column_nav() {
    let context = Context {
        workspace: Workspace::Resources,
        service_sidebar_visible: true,
        ..base_context()
    };

    assert_eq!(
        action_for_key(press(KeyCode::Char('/')), &context),
        Some(Action::Resources(ResourcesAction::StartFilter))
    );
    assert_eq!(
        action_for_key(press(KeyCode::Char('s')), &context),
        Some(Action::Resources(ResourcesAction::SortBySelectedColumn))
    );
    assert_eq!(
        action_for_key(press(KeyCode::Char('w')), &context),
        Some(Action::Resources(ResourcesAction::ToggleWideColumns))
    );
    assert_eq!(
        action_for_key(press(KeyCode::Left), &context),
        Some(Action::Resources(ResourcesAction::SelectPreviousColumn))
    );
    assert_eq!(
        action_for_key(press(KeyCode::Right), &context),
        Some(Action::Resources(ResourcesAction::SelectNextColumn))
    );
}

#[test]
fn resource_detail_mode_ignores_filter_and_sort_keys() {
    let context = Context {
        workspace: Workspace::Resources,
        resources_detail_open: true,
        service_sidebar_visible: true,
        ..base_context()
    };

    assert_eq!(action_for_key(press(KeyCode::Char('/')), &context), None);
    assert_eq!(action_for_key(press(KeyCode::Char('s')), &context), None);
}

#[test]
fn resource_filter_mode_edits_applies_cancels_and_preserves_ctrl_c_quit() {
    let context = Context {
        mode: Mode::Filter,
        workspace: Workspace::Resources,
        service_sidebar_visible: true,
        resources_filter_open: true,
        ..base_context()
    };

    assert_eq!(
        action_for_key(press(KeyCode::Char('a')), &context),
        Some(Action::Resources(ResourcesAction::AppendFilterChar('a')))
    );
    assert_eq!(
        action_for_key(press(KeyCode::Backspace), &context),
        Some(Action::Resources(ResourcesAction::BackspaceFilter))
    );
    assert_eq!(
        action_for_key(press(KeyCode::Enter), &context),
        Some(Action::Resources(ResourcesAction::ApplyFilter))
    );
    assert_eq!(
        action_for_key(press(KeyCode::Esc), &context),
        Some(Action::Resources(ResourcesAction::CancelFilter))
    );
    assert_eq!(
        action_for_key(ctrl_c(), &context),
        Some(Action::QuitRequested)
    );
}

#[test]
fn configurations_list_key_starts_create_form() {
    let context = Context {
        workspace: Workspace::Configurations,
        service_sidebar_visible: true,
        ..base_context()
    };

    assert_eq!(
        action_for_key(press(KeyCode::Char('a')), &context),
        Some(Action::Configurations(ConfigurationsAction::StartCreate))
    );
}

#[test]
fn configurations_list_key_starts_context_edit_form() {
    let context = Context {
        workspace: Workspace::Configurations,
        service_sidebar_visible: true,
        ..base_context()
    };

    assert_eq!(
        action_for_key(press(KeyCode::Char('e')), &context),
        Some(Action::Configurations(
            ConfigurationsAction::StartEditActiveContext
        ))
    );
}

#[test]
fn configurations_list_keys_start_and_clear_filter() {
    let context = Context {
        workspace: Workspace::Configurations,
        service_sidebar_visible: true,
        ..base_context()
    };

    assert_eq!(
        action_for_key(press(KeyCode::Char('/')), &context),
        Some(Action::Configurations(ConfigurationsAction::StartFiltering))
    );
    assert_eq!(
        action_for_key(press(KeyCode::Char('u')), &context),
        Some(Action::Configurations(ConfigurationsAction::ClearFilter))
    );
}

#[test]
fn configuration_filter_mode_updates_applies_cancels_and_preserves_ctrl_c_quit() {
    let context = Context {
        mode: Mode::Filter,
        workspace: Workspace::Configurations,
        service_sidebar_visible: true,
        configurations_filter_open: true,
        configurations_filter_draft: "st".to_string(),
        ..base_context()
    };

    assert_eq!(
        action_for_key(press(KeyCode::Char('a')), &context),
        Some(Action::Configurations(ConfigurationsAction::UpdateFilter(
            "sta".to_string(),
        )))
    );
    assert_eq!(
        action_for_key(press(KeyCode::Backspace), &context),
        Some(Action::Configurations(ConfigurationsAction::UpdateFilter(
            "s".to_string(),
        )))
    );
    assert_eq!(
        action_for_key(press(KeyCode::Enter), &context),
        Some(Action::Configurations(ConfigurationsAction::ApplyFilter))
    );
    assert_eq!(
        action_for_key(press(KeyCode::Esc), &context),
        Some(Action::Configurations(ConfigurationsAction::CancelFilter))
    );
    assert_eq!(
        action_for_key(ctrl_c(), &context),
        Some(Action::QuitRequested)
    );
}

#[test]
fn configurations_create_mode_edits_submits_cancels_and_preserves_ctrl_c_quit() {
    let context = Context {
        mode: Mode::EditConfiguration,
        workspace: Workspace::Configurations,
        service_sidebar_visible: true,
        configuration_create_open: true,
        ..base_context()
    };

    assert_eq!(
        action_for_key(press(KeyCode::Char('d')), &context),
        Some(Action::Configurations(
            ConfigurationsAction::AppendCreateChar('d')
        ))
    );
    assert_eq!(
        action_for_key(
            modified_press(KeyCode::Char('D'), KeyModifiers::SHIFT),
            &context
        ),
        Some(Action::Configurations(
            ConfigurationsAction::AppendCreateChar('D')
        ))
    );
    assert_eq!(
        action_for_key(
            modified_press(KeyCode::Char('d'), KeyModifiers::ALT),
            &context
        ),
        None
    );
    assert_eq!(
        action_for_key(press(KeyCode::Backspace), &context),
        Some(Action::Configurations(
            ConfigurationsAction::BackspaceCreateField
        ))
    );
    assert_eq!(
        action_for_key(press(KeyCode::Tab), &context),
        Some(Action::Configurations(
            ConfigurationsAction::FocusNextCreateField
        ))
    );
    assert_eq!(
        action_for_key(press(KeyCode::Down), &context),
        Some(Action::Configurations(
            ConfigurationsAction::FocusNextCreateField
        ))
    );
    assert_eq!(
        action_for_key(press(KeyCode::BackTab), &context),
        Some(Action::Configurations(
            ConfigurationsAction::FocusPreviousCreateField
        ))
    );
    assert_eq!(
        action_for_key(press(KeyCode::Up), &context),
        Some(Action::Configurations(
            ConfigurationsAction::FocusPreviousCreateField
        ))
    );
    assert_eq!(
        action_for_key(press(KeyCode::Enter), &context),
        Some(Action::Configurations(ConfigurationsAction::SubmitCreate))
    );
    assert_eq!(
        action_for_key(press(KeyCode::Esc), &context),
        Some(Action::Configurations(ConfigurationsAction::CancelCreate))
    );
    assert_eq!(
        action_for_key(ctrl_c(), &context),
        Some(Action::QuitRequested)
    );
}

#[test]
fn configurations_context_edit_mode_edits_submits_cancels_and_preserves_ctrl_c_quit() {
    let context = Context {
        mode: Mode::EditConfiguration,
        workspace: Workspace::Configurations,
        service_sidebar_visible: true,
        configuration_context_edit_open: true,
        ..base_context()
    };

    assert_eq!(
        action_for_key(press(KeyCode::Char('p')), &context),
        Some(Action::Configurations(
            ConfigurationsAction::AppendContextEditChar('p')
        ))
    );
    assert_eq!(
        action_for_key(
            modified_press(KeyCode::Char('P'), KeyModifiers::SHIFT),
            &context
        ),
        Some(Action::Configurations(
            ConfigurationsAction::AppendContextEditChar('P')
        ))
    );
    assert_eq!(
        action_for_key(
            modified_press(KeyCode::Char('p'), KeyModifiers::ALT),
            &context
        ),
        None
    );
    assert_eq!(
        action_for_key(press(KeyCode::Backspace), &context),
        Some(Action::Configurations(
            ConfigurationsAction::BackspaceContextEditField
        ))
    );
    assert_eq!(
        action_for_key(press(KeyCode::Tab), &context),
        Some(Action::Configurations(
            ConfigurationsAction::FocusNextContextEditField
        ))
    );
    assert_eq!(
        action_for_key(press(KeyCode::Down), &context),
        Some(Action::Configurations(
            ConfigurationsAction::FocusNextContextEditField
        ))
    );
    assert_eq!(
        action_for_key(press(KeyCode::BackTab), &context),
        Some(Action::Configurations(
            ConfigurationsAction::FocusPreviousContextEditField
        ))
    );
    assert_eq!(
        action_for_key(press(KeyCode::Up), &context),
        Some(Action::Configurations(
            ConfigurationsAction::FocusPreviousContextEditField
        ))
    );
    assert_eq!(
        action_for_key(press(KeyCode::Enter), &context),
        Some(Action::Configurations(
            ConfigurationsAction::SubmitContextEdit
        ))
    );
    assert_eq!(
        action_for_key(press(KeyCode::Esc), &context),
        Some(Action::Configurations(
            ConfigurationsAction::CancelContextEdit
        ))
    );
    assert_eq!(
        action_for_key(ctrl_c(), &context),
        Some(Action::QuitRequested)
    );
}

#[test]
fn colon_opens_command_palette_from_resource_list() {
    let context = Context {
        workspace: Workspace::Resources,
        service_sidebar_visible: true,
        ..base_context()
    };

    assert_eq!(
        action_for_key(press(KeyCode::Char(':')), &context),
        Some(Action::Command(CommandAction::Open))
    );
}

#[test]
fn colon_opens_command_palette_from_configurations_list() {
    let context = Context {
        workspace: Workspace::Configurations,
        service_sidebar_visible: true,
        ..base_context()
    };

    assert_eq!(
        action_for_key(press(KeyCode::Char(':')), &context),
        Some(Action::Command(CommandAction::Open))
    );
}

#[test]
fn command_palette_keys_filter_navigate_submit_cancel_and_preserve_ctrl_c_quit() {
    let context = Context {
        mode: Mode::CommandPalette,
        workspace: Workspace::Resources,
        service_sidebar_visible: true,
        command_palette_open: true,
        ..base_context()
    };

    assert_eq!(
        action_for_key(press(KeyCode::Char('s')), &context),
        Some(Action::Command(CommandAction::AppendChar('s')))
    );
    assert_eq!(
        action_for_key(
            modified_press(KeyCode::Char('S'), KeyModifiers::SHIFT),
            &context
        ),
        Some(Action::Command(CommandAction::AppendChar('S')))
    );
    assert_eq!(
        action_for_key(
            modified_press(KeyCode::Char('s'), KeyModifiers::ALT),
            &context
        ),
        None
    );
    assert_eq!(
        action_for_key(press(KeyCode::Backspace), &context),
        Some(Action::Command(CommandAction::Backspace))
    );
    assert_eq!(
        action_for_key(press(KeyCode::Down), &context),
        Some(Action::Command(CommandAction::SelectNext))
    );
    assert_eq!(
        action_for_key(press(KeyCode::Char('j')), &context),
        Some(Action::Command(CommandAction::AppendChar('j')))
    );
    assert_eq!(
        action_for_key(press(KeyCode::Up), &context),
        Some(Action::Command(CommandAction::SelectPrevious))
    );
    assert_eq!(
        action_for_key(press(KeyCode::Char('k')), &context),
        Some(Action::Command(CommandAction::AppendChar('k')))
    );
    assert_eq!(
        action_for_key(press(KeyCode::Enter), &context),
        Some(Action::Command(CommandAction::Submit))
    );
    assert_eq!(
        action_for_key(press(KeyCode::Esc), &context),
        Some(Action::Command(CommandAction::Cancel))
    );
    assert_eq!(
        action_for_key(ctrl_c(), &context),
        Some(Action::QuitRequested)
    );
}

#[test]
fn resource_action_confirmation_keys_only_confirm_when_preview_is_visible() {
    let mut context = Context {
        mode: Mode::ConfirmResourceAction,
        workspace: Workspace::Resources,
        service_sidebar_visible: true,
        resource_action_confirmation_open: true,
        resource_action_confirmation_preview_visible: true,
        ..base_context()
    };

    assert_eq!(
        action_for_key(press(KeyCode::Enter), &context),
        Some(Action::Resources(ResourcesAction::ConfirmPendingAction))
    );
    assert_eq!(
        action_for_key(press(KeyCode::Esc), &context),
        Some(Action::Resources(ResourcesAction::CancelPendingAction))
    );

    context.resource_action_confirmation_preview_visible = false;

    assert_eq!(action_for_key(press(KeyCode::Enter), &context), None);
    assert_eq!(
        action_for_key(press(KeyCode::Esc), &context),
        Some(Action::Resources(ResourcesAction::CancelPendingAction))
    );
}

#[test]
fn ssh_command_preview_maps_enter_to_copy_and_escape_to_cancel() {
    let mut context = Context {
        mode: Mode::ResourceCommandPreview,
        workspace: Workspace::Resources,
        service_sidebar_visible: true,
        resource_command_preview_open: true,
        resource_command_preview_visible: true,
        ..base_context()
    };

    assert_eq!(
        action_for_key(press(KeyCode::Enter), &context),
        Some(Action::Resources(ResourcesAction::CopyPendingCommand))
    );
    assert_eq!(
        action_for_key(press(KeyCode::Esc), &context),
        Some(Action::Resources(
            ResourcesAction::CancelPendingCommandPreview
        ))
    );

    context.resource_command_preview_visible = false;
    assert_eq!(action_for_key(press(KeyCode::Enter), &context), None);
    assert_eq!(
        action_for_key(press(KeyCode::Esc), &context),
        Some(Action::Resources(
            ResourcesAction::CancelPendingCommandPreview
        ))
    );
}

#[test]
fn configurations_create_keys_do_not_trigger_from_resources_workspace() {
    let context = Context {
        workspace: Workspace::Resources,
        service_sidebar_visible: true,
        ..base_context()
    };

    assert_eq!(action_for_key(press(KeyCode::Char('a')), &context), None);
}

#[test]
fn configurations_context_edit_keys_do_not_trigger_from_resources_workspace() {
    let context = Context {
        workspace: Workspace::Resources,
        service_sidebar_visible: true,
        ..base_context()
    };

    assert_eq!(action_for_key(press(KeyCode::Char('e')), &context), None);
}

#[test]
fn ctrl_space_opens_picker_from_context_edit_form() {
    let context = Context {
        mode: Mode::EditConfiguration,
        workspace: Workspace::Configurations,
        service_sidebar_visible: true,
        configuration_context_edit_open: true,
        ..base_context()
    };
    let ctrl_space = KeyEvent {
        code: KeyCode::Char(' '),
        modifiers: KeyModifiers::CONTROL,
        kind: KeyEventKind::Press,
        state: KeyEventState::empty(),
    };

    assert_eq!(
        action_for_key(ctrl_space, &context),
        Some(Action::Configurations(
            ConfigurationsAction::OpenContextPicker
        ))
    );
}

#[test]
fn ctrl_space_is_ignored_outside_context_edit_form() {
    let context = Context {
        mode: Mode::EditConfiguration,
        workspace: Workspace::Configurations,
        service_sidebar_visible: true,
        configuration_create_open: true,
        ..base_context()
    };
    let ctrl_space = KeyEvent {
        code: KeyCode::Char(' '),
        modifiers: KeyModifiers::CONTROL,
        kind: KeyEventKind::Press,
        state: KeyEventState::empty(),
    };

    assert_eq!(action_for_key(ctrl_space, &context), None);

    let context = Context {
        hotkeys: nuvem::config::hotkeys::KeyOverrides::empty(),
        configuration_context_edit_open: true,
        configuration_context_picker_open: true,
        ..context
    };
    let ctrl_space = KeyEvent {
        code: KeyCode::Char(' '),
        modifiers: KeyModifiers::CONTROL,
        kind: KeyEventKind::Press,
        state: KeyEventState::empty(),
    };

    assert_eq!(action_for_key(ctrl_space, &context), None);
}

#[test]
fn context_picker_keys_filter_navigate_select_retry_and_cancel() {
    let context = Context {
        mode: Mode::ConfigurationContextPicker,
        workspace: Workspace::Configurations,
        service_sidebar_visible: true,
        configuration_context_edit_open: true,
        configuration_context_picker_open: true,
        ..base_context()
    };

    assert_eq!(
        action_for_key(press(KeyCode::Char('/')), &context),
        Some(Action::Configurations(
            ConfigurationsAction::StartContextPickerFilter
        ))
    );
    assert_eq!(
        action_for_key(press(KeyCode::Char('j')), &context),
        Some(Action::Configurations(
            ConfigurationsAction::SelectNextContextPickerItem
        ))
    );
    assert_eq!(
        action_for_key(press(KeyCode::Down), &context),
        Some(Action::Configurations(
            ConfigurationsAction::SelectNextContextPickerItem
        ))
    );
    assert_eq!(
        action_for_key(press(KeyCode::Char('k')), &context),
        Some(Action::Configurations(
            ConfigurationsAction::SelectPreviousContextPickerItem
        ))
    );
    assert_eq!(
        action_for_key(press(KeyCode::Up), &context),
        Some(Action::Configurations(
            ConfigurationsAction::SelectPreviousContextPickerItem
        ))
    );
    assert_eq!(
        action_for_key(press(KeyCode::Char('r')), &context),
        Some(Action::Configurations(
            ConfigurationsAction::RetryContextPicker
        ))
    );
    assert_eq!(
        action_for_key(press(KeyCode::Enter), &context),
        Some(Action::Configurations(
            ConfigurationsAction::ApplyContextPickerSelection
        ))
    );
    assert_eq!(
        action_for_key(press(KeyCode::Esc), &context),
        Some(Action::Configurations(
            ConfigurationsAction::CancelContextPicker
        ))
    );
}

#[test]
fn context_picker_filter_escape_only_cancels_filter_first() {
    let context = Context {
        mode: Mode::ConfigurationContextPicker,
        workspace: Workspace::Configurations,
        service_sidebar_visible: true,
        configuration_context_edit_open: true,
        configuration_context_picker_open: true,
        configuration_context_picker_filter_open: true,
        configuration_context_picker_filter_draft: "pro".to_string(),
        ..base_context()
    };

    assert_eq!(
        action_for_key(press(KeyCode::Esc), &context),
        Some(Action::Configurations(
            ConfigurationsAction::CancelContextPickerFilter
        ))
    );
}

#[test]
fn context_picker_ctrl_c_still_quits() {
    let context = Context {
        mode: Mode::ConfigurationContextPicker,
        workspace: Workspace::Configurations,
        service_sidebar_visible: true,
        configuration_context_edit_open: true,
        configuration_context_picker_open: true,
        ..base_context()
    };

    assert_eq!(
        action_for_key(ctrl_c(), &context),
        Some(Action::QuitRequested)
    );
}

#[test]
fn modified_picker_text_keys_do_not_append_unexpectedly() {
    let context = Context {
        mode: Mode::ConfigurationContextPicker,
        workspace: Workspace::Configurations,
        service_sidebar_visible: true,
        configuration_context_edit_open: true,
        configuration_context_picker_open: true,
        configuration_context_picker_filter_open: true,
        configuration_context_picker_filter_draft: "pr".to_string(),
        ..base_context()
    };

    assert_eq!(
        action_for_key(press(KeyCode::Char('o')), &context),
        Some(Action::Configurations(
            ConfigurationsAction::UpdateContextPickerFilter("pro".to_string())
        ))
    );
    assert_eq!(
        action_for_key(
            modified_press(KeyCode::Char('O'), KeyModifiers::SHIFT),
            &context
        ),
        Some(Action::Configurations(
            ConfigurationsAction::UpdateContextPickerFilter("prO".to_string())
        ))
    );
    assert_eq!(
        action_for_key(
            modified_press(KeyCode::Char('o'), KeyModifiers::ALT),
            &context
        ),
        None
    );
    assert_eq!(
        action_for_key(press(KeyCode::Backspace), &context),
        Some(Action::Configurations(
            ConfigurationsAction::UpdateContextPickerFilter("p".to_string())
        ))
    );
    assert_eq!(
        action_for_key(press(KeyCode::Enter), &context),
        Some(Action::Configurations(
            ConfigurationsAction::ApplyContextPickerFilter
        ))
    );
}

#[test]
fn colon_opens_command_palette_from_home() {
    let action = action_for_key(
        press(KeyCode::Char(':')),
        &normal_context(Workspace::Home, false),
    );
    assert_eq!(action, Some(Action::Command(CommandAction::Open)));
}

#[test]
fn colon_opens_command_palette_from_resource_detail() {
    let action = action_for_key(
        press(KeyCode::Char(':')),
        &normal_context(Workspace::Resources, true),
    );
    assert_eq!(action, Some(Action::Command(CommandAction::Open)));
}

// Cloud Storage key bindings (issue #34): `i` bucket metadata, `y` copy
// gs:// URI, `n` next page, `Backspace`/`-` ascend a prefix level. These
// dispatch unconditionally from the generic resource list/detail contexts;
// the reducer itself gates on the active service being Cloud Storage.
#[test]
fn i_opens_bucket_metadata_from_resource_list() {
    let context = normal_resources_list_context();
    assert_eq!(
        action_for_key(press(KeyCode::Char('i')), &context),
        Some(Action::Resources(ResourcesAction::OpenBucketMetadata))
    );
}

#[test]
fn y_copies_resource_uri_from_resource_list() {
    let context = normal_resources_list_context();
    assert_eq!(
        action_for_key(press(KeyCode::Char('y')), &context),
        Some(Action::Resources(ResourcesAction::CopyResourceUri))
    );
}

#[test]
fn y_copies_resource_uri_from_resource_detail() {
    let context = normal_resources_detail_context();
    assert_eq!(
        action_for_key(press(KeyCode::Char('y')), &context),
        Some(Action::Resources(ResourcesAction::CopyResourceUri))
    );
}

#[test]
fn n_loads_next_page_from_resource_list() {
    let context = normal_resources_list_context();
    assert_eq!(
        action_for_key(press(KeyCode::Char('n')), &context),
        Some(Action::Resources(ResourcesAction::NextPage))
    );
}

#[test]
fn backspace_and_dash_ascend_prefix_from_resource_list() {
    let context = normal_resources_list_context();
    assert_eq!(
        action_for_key(press(KeyCode::Backspace), &context),
        Some(Action::Resources(ResourcesAction::AscendPrefix))
    );
    assert_eq!(
        action_for_key(press(KeyCode::Char('-')), &context),
        Some(Action::Resources(ResourcesAction::AscendPrefix))
    );
}

#[test]
fn storage_keys_are_not_reachable_from_resource_detail_except_copy_uri() {
    // `i` (bucket metadata) and `-`/Backspace (ascend) are list-only actions;
    // only `y` (copy URI) is meaningful from a detail pane, since the reducer
    // resolves the URI from current selection regardless of view. `n` is bound
    // in both contexts, to different actions (next page in the list, next
    // detail field in the pane) -- see the detail field-cursor tests below.
    let context = normal_resources_detail_context();
    assert_eq!(action_for_key(press(KeyCode::Char('i')), &context), None);
    assert_eq!(action_for_key(press(KeyCode::Char('-')), &context), None);
    assert_eq!(action_for_key(press(KeyCode::Backspace), &context), None);
}

// Detail-pane field cursor: `n`/`p` move over the copyable fields of
// the pane on screen. They are detail-only; the list context keeps `n` on
// Cloud Storage pagination (see `n_loads_next_page_from_resource_list`) and
// leaves `p` unbound.
#[test]
fn n_selects_the_next_detail_field_from_resource_detail() {
    let context = normal_resources_detail_context();
    assert_eq!(
        action_for_key(press(KeyCode::Char('n')), &context),
        Some(Action::Resources(ResourcesAction::SelectNextDetailField))
    );
}

#[test]
fn p_selects_the_previous_detail_field_from_resource_detail() {
    let context = normal_resources_detail_context();
    assert_eq!(
        action_for_key(press(KeyCode::Char('p')), &context),
        Some(Action::Resources(
            ResourcesAction::SelectPreviousDetailField
        ))
    );
}

#[test]
fn detail_field_keys_are_not_reachable_from_the_resource_list() {
    let context = normal_resources_list_context();
    assert_eq!(
        action_for_key(press(KeyCode::Char('n')), &context),
        Some(Action::Resources(ResourcesAction::NextPage))
    );
    assert_eq!(action_for_key(press(KeyCode::Char('p')), &context), None);
}

fn logging_context(mode: Mode) -> Context {
    Context {
        workspace: Workspace::Logging,
        ..context(Workspace::Logging, mode)
    }
}

fn k9s_confirmation_context(preview_visible: bool) -> Context {
    Context {
        mode: Mode::ConfirmK9sLaunch,
        k9s_launch_confirmation_open: true,
        k9s_launch_confirmation_preview_visible: preview_visible,
        resource_jump_picker_open: false,
        ..normal_resources_list_context()
    }
}

#[test]
fn l_opens_logging_workspace_from_home() {
    let action = action_for_key(
        press(KeyCode::Char('l')),
        &normal_context(Workspace::Home, false),
    );
    assert_eq!(action, Some(Action::Workspace(WorkspaceAction::Logging)));
}

#[test]
fn t_toggles_tail_in_logging_normal() {
    let action = action_for_key(press(KeyCode::Char('t')), &logging_context(Mode::Normal));
    assert_eq!(action, Some(Action::Logging(LoggingAction::ToggleTail)));
}

#[test]
fn slash_starts_filter_in_logging_normal() {
    let action = action_for_key(press(KeyCode::Char('/')), &logging_context(Mode::Normal));
    assert_eq!(action, Some(Action::Logging(LoggingAction::StartFilter)));
}

#[test]
fn r_refreshes_logging() {
    let action = action_for_key(press(KeyCode::Char('r')), &logging_context(Mode::Normal));
    assert_eq!(action, Some(Action::Logging(LoggingAction::Refresh)));
}

fn logging_detail_context() -> Context {
    Context {
        logging_detail_open: true,
        ..logging_context(Mode::Normal)
    }
}

fn logging_list_context() -> Context {
    Context {
        logging_detail_open: false,
        ..logging_context(Mode::Normal)
    }
}

#[test]
fn esc_closes_detail_when_logging_detail_open() {
    let action = action_for_key(press(KeyCode::Esc), &logging_detail_context());
    assert_eq!(action, Some(Action::Logging(LoggingAction::CloseDetail)));
}

#[test]
fn enter_closes_detail_when_logging_detail_open() {
    let action = action_for_key(press(KeyCode::Enter), &logging_detail_context());
    assert_eq!(action, Some(Action::Logging(LoggingAction::CloseDetail)));
}

#[test]
fn page_down_scrolls_detail_when_logging_detail_open() {
    let action = action_for_key(press(KeyCode::PageDown), &logging_detail_context());
    assert_eq!(
        action,
        Some(Action::Logging(LoggingAction::ScrollDetailDown))
    );
}

#[test]
fn k9s_confirmation_enter_confirms_when_preview_visible() {
    let action = action_for_key(press(KeyCode::Enter), &k9s_confirmation_context(true));
    assert_eq!(
        action,
        Some(Action::Resources(ResourcesAction::ConfirmK9sLaunch))
    );
}

#[test]
fn page_up_scrolls_detail_when_logging_detail_open() {
    let action = action_for_key(press(KeyCode::PageUp), &logging_detail_context());
    assert_eq!(action, Some(Action::Logging(LoggingAction::ScrollDetailUp)));
}

#[test]
fn q_closes_detail_when_logging_detail_open() {
    let action = action_for_key(press(KeyCode::Char('q')), &logging_detail_context());
    assert_eq!(action, Some(Action::Logging(LoggingAction::CloseDetail)));
}

#[test]
fn j_k_do_not_navigate_list_when_logging_detail_open() {
    assert_eq!(
        action_for_key(press(KeyCode::Char('j')), &logging_detail_context()),
        None
    );
    assert_eq!(
        action_for_key(press(KeyCode::Char('k')), &logging_detail_context()),
        None
    );
}

#[test]
fn space_toggles_mark_in_resource_list() {
    let ctx = normal_resources_list_context();
    assert_eq!(
        action_for_key(press(KeyCode::Char(' ')), &ctx),
        Some(Action::Resources(ResourcesAction::ToggleMark))
    );
    assert_eq!(
        action_for_key(
            modified_press(KeyCode::Char('V'), KeyModifiers::SHIFT),
            &ctx
        ),
        Some(Action::Resources(ResourcesAction::RangeMark))
    );
    assert_eq!(
        action_for_key(press(KeyCode::Char('\\')), &ctx),
        Some(Action::Resources(ResourcesAction::ClearMarks))
    );
    assert_eq!(
        action_for_key(
            modified_press(KeyCode::Char('U'), KeyModifiers::SHIFT),
            &ctx
        ),
        Some(Action::Resources(ResourcesAction::UnmarkVisible))
    );
}

#[test]
fn enter_opens_detail_when_logging_list_active() {
    let action = action_for_key(press(KeyCode::Enter), &logging_list_context());
    assert_eq!(action, Some(Action::Logging(LoggingAction::OpenDetail)));
}

#[test]
fn esc_navigates_back_when_logging_list_active() {
    let action = action_for_key(press(KeyCode::Esc), &logging_list_context());
    assert_eq!(action, Some(Action::Navigation(NavAction::Back)));
}

#[test]
fn q_quits_when_logging_list_active() {
    let action = action_for_key(press(KeyCode::Char('q')), &logging_list_context());
    assert_eq!(action, Some(Action::QuitRequested));
}

#[test]
fn k9s_confirmation_enter_ignored_when_preview_hidden() {
    let action = action_for_key(press(KeyCode::Enter), &k9s_confirmation_context(false));
    assert_eq!(action, None);
}

#[test]
fn k9s_confirmation_esc_cancels() {
    let action = action_for_key(press(KeyCode::Esc), &k9s_confirmation_context(true));
    assert_eq!(
        action,
        Some(Action::Resources(ResourcesAction::CancelK9sLaunch))
    );
}

#[test]
fn lowercase_v_still_toggles_sidebar_in_resource_list() {
    let ctx = normal_resources_list_context();
    assert_eq!(
        action_for_key(press(KeyCode::Char('v')), &ctx),
        Some(Action::Navigator(NavigatorAction::ToggleSidebar))
    );
}

fn batch_confirmation_context(ack_satisfied: bool) -> Context {
    Context {
        mode: Mode::ConfirmResourceAction,
        resource_batch_confirmation_open: true,
        resource_batch_confirmation_visible: true,
        resource_batch_ack_satisfied: ack_satisfied,
        ..context(Workspace::Resources, Mode::ConfirmResourceAction)
    }
}

#[test]
fn batch_confirmation_enter_confirms_when_ack_satisfied() {
    let ctx = batch_confirmation_context(true);
    assert_eq!(
        action_for_key(press(KeyCode::Enter), &ctx),
        Some(Action::Resources(ResourcesAction::ConfirmBatchAction))
    );
}

#[test]
fn batch_confirmation_enter_ignored_when_ack_unsatisfied() {
    let ctx = batch_confirmation_context(false);
    assert_eq!(action_for_key(press(KeyCode::Enter), &ctx), None);
}

#[test]
fn batch_confirmation_enter_ignored_when_modal_not_visible() {
    // Mirrors the single-row `resource_action_confirmation_preview_visible`
    // gate: even when the ack is satisfied, Enter must not confirm a batch
    // action if the terminal is too small to render the target list.
    let ctx = Context {
        resource_batch_confirmation_visible: false,
        ..batch_confirmation_context(true)
    };
    assert_eq!(action_for_key(press(KeyCode::Enter), &ctx), None);
}

#[test]
fn batch_confirmation_digit_appends_to_ack() {
    let ctx = batch_confirmation_context(false);
    assert_eq!(
        action_for_key(press(KeyCode::Char('2')), &ctx),
        Some(Action::Resources(ResourcesAction::BatchAckAppendDigit('2')))
    );
}

#[test]
fn batch_confirmation_backspace_and_scroll_and_esc() {
    let ctx = batch_confirmation_context(false);
    assert_eq!(
        action_for_key(press(KeyCode::Backspace), &ctx),
        Some(Action::Resources(ResourcesAction::BatchAckBackspace))
    );
    assert_eq!(
        action_for_key(press(KeyCode::PageDown), &ctx),
        Some(Action::Resources(ResourcesAction::ScrollBatchConfirmDown))
    );
    assert_eq!(
        action_for_key(press(KeyCode::PageUp), &ctx),
        Some(Action::Resources(ResourcesAction::ScrollBatchConfirmUp))
    );
    assert_eq!(
        action_for_key(press(KeyCode::Esc), &ctx),
        Some(Action::Resources(ResourcesAction::CancelBatchAction))
    );
}

fn logging_filter_context() -> Context {
    Context {
        logging_filter_open: true,
        ..logging_context(Mode::Filter)
    }
}

#[test]
fn s_and_shift_s_step_the_severity_floor_in_logging_list() {
    let ctx = logging_list_context();
    assert_eq!(
        action_for_key(press(KeyCode::Char('s')), &ctx),
        Some(Action::Logging(LoggingAction::RaiseSeverityFloor))
    );
    assert_eq!(
        action_for_key(
            modified_press(KeyCode::Char('S'), KeyModifiers::SHIFT),
            &ctx
        ),
        Some(Action::Logging(LoggingAction::LowerSeverityFloor))
    );
}

#[test]
fn digits_select_since_buckets_in_logging_list() {
    let ctx = logging_list_context();
    let expected = [
        ('0', LogTimeRange::OneMinute),
        ('1', LogTimeRange::FiveMinutes),
        ('2', LogTimeRange::FifteenMinutes),
        ('3', LogTimeRange::ThirtyMinutes),
        ('4', LogTimeRange::OneHour),
        ('5', LogTimeRange::SixHours),
        ('6', LogTimeRange::TwentyFourHours),
    ];
    for (digit, range) in expected {
        assert_eq!(
            action_for_key(press(KeyCode::Char(digit)), &ctx),
            Some(Action::Logging(LoggingAction::SetTimeRange(range))),
            "digit {digit} should select its bucket"
        );
    }
    // Only seven buckets exist, so the remaining digits stay unbound.
    for digit in ['7', '8', '9'] {
        assert_eq!(
            action_for_key(press(KeyCode::Char(digit)), &ctx),
            None,
            "digit {digit} is not a bucket"
        );
    }
}

#[test]
fn ctrl_s_saves_logs_and_is_not_read_as_the_severity_key_in_logging_list() {
    let action = action_for_key(
        modified_press(KeyCode::Char('s'), KeyModifiers::CONTROL),
        &logging_list_context(),
    );
    assert_eq!(action, Some(Action::Logging(LoggingAction::SaveToFile)));
}

#[test]
fn severity_and_bucket_keys_type_into_the_logging_filter_draft() {
    // Filter entry keeps consuming plain characters as text: neither the
    // severity keys nor the bucket digits may be stolen from a typed pattern.
    let ctx = logging_filter_context();
    for ch in ['s', 'S', '0', '6'] {
        assert_eq!(
            action_for_key(press(KeyCode::Char(ch)), &ctx),
            Some(Action::Logging(LoggingAction::AppendFilterChar(ch))),
            "{ch} should be filter text while the filter is open"
        );
    }
    // Save is not reachable from filter entry either.
    assert_eq!(
        action_for_key(
            modified_press(KeyCode::Char('s'), KeyModifiers::CONTROL),
            &ctx
        ),
        None
    );
}

#[test]
fn severity_bucket_and_save_keys_are_unbound_in_logging_detail() {
    let ctx = logging_detail_context();
    for ch in ['s', 'S', '0', '6'] {
        assert_eq!(
            action_for_key(press(KeyCode::Char(ch)), &ctx),
            None,
            "{ch} must not act in the log detail view"
        );
    }
    assert_eq!(
        action_for_key(
            modified_press(KeyCode::Char('s'), KeyModifiers::CONTROL),
            &ctx
        ),
        None
    );
}

// ---------------------------------------------------------------------------
// Tunnels workspace
// ---------------------------------------------------------------------------

fn ctrl(code: char) -> KeyEvent {
    modified_press(KeyCode::Char(code), KeyModifiers::CONTROL)
}

fn tunnels_context() -> Context {
    Context {
        workspace: Workspace::Tunnels,
        ..normal_resources_list_context()
    }
}

/// `Ctrl-t` has the same reach as `l`: it opens the Tunnels workspace from
/// every normal-mode workspace, including from inside Tunnels itself.
#[test]
fn ctrl_t_opens_the_tunnels_workspace_from_every_workspace() {
    for workspace in [
        Workspace::Home,
        Workspace::Configurations,
        Workspace::Resources,
        Workspace::Logging,
        Workspace::Tunnels,
    ] {
        assert_eq!(
            action_for_key(
                ctrl('t'),
                &Context {
                    workspace,
                    ..normal_resources_list_context()
                }
            ),
            Some(Action::Workspace(WorkspaceAction::Tunnels)),
            "workspace {workspace:?} must open Tunnels on Ctrl-t"
        );
    }
}

#[test]
fn ctrl_x_cancels_running_plugins_from_every_normal_view() {
    // One per normal-mode surface, built from this file's own context helpers
    // (`tests/keymap_test.rs:10`, `:53`, `:2847`, `:2840`, `:3154`). The claim
    // is that the key has the same reach as `l` and `Ctrl-t`, and a single
    // context could not distinguish that from a Resources-only binding.
    let contexts = [
        normal_resources_list_context(),
        normal_resources_detail_context(),
        logging_list_context(),
        logging_detail_context(),
        tunnels_context(),
    ];
    for context in contexts {
        assert_eq!(
            action_for_key(
                modified_press(KeyCode::Char('x'), KeyModifiers::CONTROL),
                &context
            ),
            Some(Action::System(SystemAction::CancelRunningPlugins)),
            "Ctrl-x must cancel a running plugin in {:?}",
            context.workspace
        );
    }
}

/// Every `Mode`, not just every normal-mode surface.
///
/// A plugin outlives the view it was confirmed in, so a modal that opens after
/// the run started must not be able to strand it. `Ctrl-x` used to be
/// an arm of `normal_key`, so every other mode answered `None` — including
/// `PluginPrefix`, which unlike its siblings renders no modal to explain who
/// owns the keyboard.
///
/// The walk is exhaustive rather than a sample because a mode added later
/// inherits whatever this key does in it, and a sample cannot notice.
#[test]
fn ctrl_x_cancels_running_plugins_from_every_mode() {
    for mode in Mode::ALL {
        let mut context = base_context();
        context.mode = mode;

        assert_eq!(
            action_for_key(
                modified_press(KeyCode::Char('x'), KeyModifiers::CONTROL),
                &context
            ),
            Some(Action::System(SystemAction::CancelRunningPlugins)),
            "Ctrl-x must cancel a running plugin in {mode:?}"
        );
    }
}

/// The chords next to `Ctrl-x`, over every mode for the same reason the walk
/// above is: the exactness of the modifier match is a global
/// property, so a single context could no longer stand for it.
#[test]
fn ctrl_x_chords_do_not_cancel_running_plugins() {
    for mode in Mode::ALL {
        let mut context = base_context();
        context.mode = mode;

        // Exactly CONTROL, like Ctrl-t and Ctrl-C: a `.contains` guard would
        // also cancel on chords the help catalog never claims.
        for modifiers in [
            KeyModifiers::CONTROL | KeyModifiers::ALT,
            KeyModifiers::CONTROL | KeyModifiers::SHIFT,
        ] {
            assert_eq!(
                action_for_key(modified_press(KeyCode::Char('x'), modifiers), &context),
                None,
                "{modifiers:?} must not cancel a running plugin in {mode:?}"
            );
        }
    }

    // A bare `x` is unbound in the normal-mode views and must stay that way: it
    // is what makes `Ctrl-x` safe to hand a destructive meaning. Not swept over
    // `Mode::ALL` — `PluginPrefix` reads every plain character as a plugin key,
    // so a bare `x` dispatching there is the point of that mode, not a leak.
    let context = normal_resources_list_context();
    assert_eq!(action_for_key(press(KeyCode::Char('x')), &context), None);
}

/// `Ctrl-t` must also reach the Tunnels workspace from a resource detail view
/// and an open log entry, the two normal-mode sub-views with their own dispatch
/// tables.
#[test]
fn ctrl_t_reaches_tunnels_from_detail_sub_views() {
    let resource_detail = Context {
        workspace: Workspace::Resources,
        resources_detail_open: true,
        ..normal_resources_list_context()
    };
    let logging_detail = Context {
        workspace: Workspace::Logging,
        logging_detail_open: true,
        ..normal_resources_list_context()
    };
    for context in [resource_detail, logging_detail] {
        assert_eq!(
            action_for_key(ctrl('t'), &context),
            Some(Action::Workspace(WorkspaceAction::Tunnels))
        );
    }
}

/// The regression this key was chosen to avoid: a bare `t` must stay the
/// Logging tail toggle, and must never open the Tunnels workspace.
#[test]
fn ctrl_t_does_not_shadow_the_logging_tail_toggle() {
    let logging = Context {
        workspace: Workspace::Logging,
        ..normal_resources_list_context()
    };
    assert_eq!(
        action_for_key(press(KeyCode::Char('t')), &logging),
        Some(Action::Logging(LoggingAction::ToggleTail)),
        "a bare `t` in Logging must still toggle the tail"
    );
    assert_eq!(
        action_for_key(ctrl('t'), &logging),
        Some(Action::Workspace(WorkspaceAction::Tunnels)),
        "Ctrl-t must open Tunnels even where a bare `t` is bound"
    );
}

/// Conversely, a bare `t` outside Logging must not open Tunnels either: the
/// key belongs to Task 7's start-a-tunnel action on a resource row.
#[test]
fn a_bare_t_does_not_open_the_tunnels_workspace() {
    for workspace in [
        Workspace::Home,
        Workspace::Configurations,
        Workspace::Resources,
        Workspace::Tunnels,
    ] {
        assert_ne!(
            action_for_key(
                press(KeyCode::Char('t')),
                &Context {
                    workspace,
                    ..normal_resources_list_context()
                }
            ),
            Some(Action::Workspace(WorkspaceAction::Tunnels)),
            "a bare `t` must not open Tunnels from {workspace:?}"
        );
    }
}

#[test]
fn tunnels_j_k_and_arrows_move_the_selection() {
    let context = tunnels_context();
    for code in [KeyCode::Char('j'), KeyCode::Down] {
        assert_eq!(
            action_for_key(press(code), &context),
            Some(Action::Tunnels(TunnelsAction::SelectNext))
        );
    }
    for code in [KeyCode::Char('k'), KeyCode::Up] {
        assert_eq!(
            action_for_key(press(code), &context),
            Some(Action::Tunnels(TunnelsAction::SelectPrevious))
        );
    }
}

#[test]
fn tunnels_shift_k_kills_the_selected_tunnel() {
    assert_eq!(
        action_for_key(press(KeyCode::Char('K')), &tunnels_context()),
        Some(Action::Tunnels(TunnelsAction::KillSelected))
    );
}

#[test]
fn tunnels_d_dismisses_the_selected_row() {
    assert_eq!(
        action_for_key(press(KeyCode::Char('d')), &tunnels_context()),
        Some(Action::Tunnels(TunnelsAction::DismissSelected))
    );
}

#[test]
fn tunnels_esc_goes_back_through_navigation_history() {
    assert_eq!(
        action_for_key(press(KeyCode::Esc), &tunnels_context()),
        Some(Action::Navigation(NavAction::Back))
    );
}

#[test]
fn tunnels_colon_opens_the_command_palette() {
    assert_eq!(
        action_for_key(press(KeyCode::Char(':')), &tunnels_context()),
        Some(Action::Command(CommandAction::Open))
    );
}

/// `r` still resolves in Tunnels (it is a global rebindable key), but there is
/// nothing to fetch, so it re-selects the workspace rather than falling
/// through to another workspace's refresh.
#[test]
fn r_in_tunnels_is_a_workspace_reselect_not_another_workspaces_refresh() {
    assert_eq!(
        action_for_key(press(KeyCode::Char('r')), &tunnels_context()),
        Some(Action::Workspace(WorkspaceAction::Tunnels))
    );
}

/// The two other non-rebindable global keys must keep working from Tunnels.
#[test]
fn tunnels_keeps_the_global_logging_and_copy_diagnostic_keys() {
    let context = tunnels_context();
    assert_eq!(
        action_for_key(press(KeyCode::Char('l')), &context),
        Some(Action::Workspace(WorkspaceAction::Logging))
    );
    assert_eq!(
        action_for_key(press(KeyCode::Char('Y')), &context),
        Some(Action::System(SystemAction::CopyDiagnostic))
    );
}

/// `t` on a resource row asks for a managed tunnel. It is dispatched
/// unconditionally from the generic resource list — the reducer no-ops it for
/// services with no tunnel plan, the same shape as the storage-only keys.
#[test]
fn t_on_a_resource_row_requests_a_tunnel() {
    assert_eq!(
        action_for_key(press(KeyCode::Char('t')), &normal_resources_list_context()),
        Some(Action::Resources(ResourcesAction::RequestTunnel))
    );
}

/// A detail pane shows one resource, not a row to tunnel to, so `t` stays
/// unbound there rather than silently acting on the list selection behind it.
#[test]
fn t_in_a_resource_detail_pane_is_unbound() {
    assert_eq!(
        action_for_key(
            press(KeyCode::Char('t')),
            &normal_resources_detail_context()
        ),
        None
    );
}

/// `t` must not shadow the navigator when the sidebar has focus.
#[test]
fn t_with_the_service_navigator_focused_is_unbound() {
    let mut context = normal_resources_list_context();
    context.service_navigator_focused = true;
    assert_eq!(action_for_key(press(KeyCode::Char('t')), &context), None);
}

fn tunnel_launch_preview_context(visible: bool) -> Context {
    let mut context = normal_resources_list_context();
    context.mode = Mode::TunnelLaunchPreview;
    context.tunnel_launch_preview_open = true;
    context.tunnel_launch_preview_visible = visible;
    context
}

#[test]
fn enter_in_a_visible_tunnel_launch_preview_confirms_the_launch() {
    let action = action_for_key(press(KeyCode::Enter), &tunnel_launch_preview_context(true));
    assert!(
        matches!(
            action,
            Some(Action::Resources(
                ResourcesAction::ConfirmTunnelLaunch { .. }
            ))
        ),
        "expected a ConfirmTunnelLaunch, got {action:?}"
    );
}

/// The same guard the other launch modals use: a preview the terminal is too
/// small to render must not be confirmable, since the user cannot read the
/// argv they would be accepting.
#[test]
fn enter_in_a_clipped_tunnel_launch_preview_is_ignored() {
    assert_eq!(
        action_for_key(press(KeyCode::Enter), &tunnel_launch_preview_context(false)),
        None
    );
}

#[test]
fn esc_cancels_the_tunnel_launch_preview_even_when_clipped() {
    for visible in [true, false] {
        assert_eq!(
            action_for_key(press(KeyCode::Esc), &tunnel_launch_preview_context(visible)),
            Some(Action::Resources(ResourcesAction::CancelTunnelLaunch))
        );
    }
}

/// `L` opens Cloud Logging scoped to the current selection. It is
/// bound in both the resource list and the resource detail pane, since both
/// have a resource under the cursor.
///
/// Both spellings of the keypress are asserted, and the shifted one is the
/// load-bearing half: a real terminal reports `L` as `Char('L')` *with*
/// [`KeyModifiers::SHIFT`], never with empty modifiers. Asserting only the bare
/// form would leave the key free to go dead in every terminal while the suite
/// stayed green — the same convention `V` and `U` follow above.
#[test]
fn shift_l_opens_logs_for_the_selection_in_list_and_detail() {
    for context in [
        normal_resources_list_context(),
        normal_resources_detail_context(),
    ] {
        assert_eq!(
            action_for_key(press(KeyCode::Char('L')), &context),
            Some(Action::Resources(ResourcesAction::OpenLogsForSelection))
        );
        assert_eq!(
            action_for_key(
                modified_press(KeyCode::Char('L'), KeyModifiers::SHIFT),
                &context
            ),
            Some(Action::Resources(ResourcesAction::OpenLogsForSelection))
        );
    }
}

/// `L` must be a plain key: `Ctrl-L` is a different chord and must not fall
/// through to the selection-logs jump.
#[test]
fn ctrl_l_is_not_the_selection_logs_key() {
    assert_ne!(
        action_for_key(
            modified_press(KeyCode::Char('L'), KeyModifiers::CONTROL),
            &normal_resources_list_context()
        ),
        Some(Action::Resources(ResourcesAction::OpenLogsForSelection))
    );
}

/// Lowercase `l` still means "go to the Logging workspace" — the two keys are
/// deliberately different actions (see the help catalog wording).
#[test]
fn lowercase_l_still_opens_the_logging_workspace_from_a_resource_list() {
    assert_eq!(
        action_for_key(press(KeyCode::Char('l')), &normal_resources_list_context()),
        Some(Action::Workspace(WorkspaceAction::Logging))
    );
}

// ---------------------------------------------------------------------------
// `J`: the related-resource jump picker
// ---------------------------------------------------------------------------

fn jump_picker_context() -> Context {
    Context {
        mode: Mode::ResourceJumpPicker,
        resource_jump_picker_open: true,
        ..normal_resources_list_context()
    }
}

/// `J` reaches the picker from both resource contexts, mirroring `L`: a detail
/// pane has a resource on screen too.
///
/// As with `L`, the shifted spelling is the one a real terminal actually sends,
/// so it is asserted next to the bare one.
#[test]
fn shift_j_opens_the_jump_picker_in_list_and_detail() {
    for context in [
        normal_resources_list_context(),
        normal_resources_detail_context(),
    ] {
        assert_eq!(
            action_for_key(press(KeyCode::Char('J')), &context),
            Some(Action::Jumps(JumpsAction::Open))
        );
        assert_eq!(
            action_for_key(
                modified_press(KeyCode::Char('J'), KeyModifiers::SHIFT),
                &context
            ),
            Some(Action::Jumps(JumpsAction::Open))
        );
    }
}

/// The picker is a *resource* key. Offering it in the Logging or Tunnels
/// workspace would open a modal about a selection those views do not have.
#[test]
fn shift_j_is_unbound_outside_the_resource_workspace() {
    for context in [
        normal_context(Workspace::Home, false),
        logging_context(Mode::Normal),
        tunnels_context(),
    ] {
        assert_eq!(action_for_key(press(KeyCode::Char('J')), &context), None);
    }
}

#[test]
fn the_jump_picker_moves_with_j_k_and_the_arrow_keys() {
    let context = jump_picker_context();
    for code in [KeyCode::Char('j'), KeyCode::Down] {
        assert_eq!(
            action_for_key(press(code), &context),
            Some(Action::Jumps(JumpsAction::SelectNext))
        );
    }
    for code in [KeyCode::Char('k'), KeyCode::Up] {
        assert_eq!(
            action_for_key(press(code), &context),
            Some(Action::Jumps(JumpsAction::SelectPrevious))
        );
    }
}

#[test]
fn the_jump_picker_submits_on_enter_and_cancels_on_esc() {
    let context = jump_picker_context();
    assert_eq!(
        action_for_key(press(KeyCode::Enter), &context),
        Some(Action::Jumps(JumpsAction::Submit))
    );
    assert_eq!(
        action_for_key(press(KeyCode::Esc), &context),
        Some(Action::Jumps(JumpsAction::Cancel))
    );
}

/// The modal owns the keyboard while it is up: keys that would otherwise move
/// the list or open another view behind it must not reach the normal-mode arms.
#[test]
fn the_jump_picker_swallows_normal_mode_keys() {
    let context = jump_picker_context();
    for code in [KeyCode::Char('l'), KeyCode::Char('/'), KeyCode::Char('L')] {
        assert_eq!(action_for_key(press(code), &context), None);
    }
}

/// `Ctrl-S` is the Logging save-to-file key, and the resource list
/// used to steal it — `resource_content_key` matched on `key.code` alone, so
/// `Ctrl-S` in a table sorted the selected column. The modifier now decides:
/// plain `s` still sorts, `Ctrl-S` is unbound outside Logging, and inside
/// Logging it still saves.
#[test]
fn ctrl_s_does_not_sort_the_resource_list_and_still_saves_in_logging() {
    let list = normal_resources_list_context();
    let ctrl_s = modified_press(KeyCode::Char('s'), KeyModifiers::CONTROL);

    assert_eq!(
        action_for_key(ctrl_s, &list),
        None,
        "Ctrl-S must be unbound in the resource list, not a column sort"
    );
    assert_eq!(
        action_for_key(press(KeyCode::Char('s')), &list),
        Some(Action::Resources(ResourcesAction::SortBySelectedColumn)),
        "plain s must still sort the resource list"
    );

    let logging = normal_context(Workspace::Logging, false);
    assert_eq!(
        action_for_key(ctrl_s, &logging),
        Some(Action::Logging(LoggingAction::SaveToFile))
    );
    assert_eq!(
        action_for_key(press(KeyCode::Char('s')), &logging),
        Some(Action::Logging(LoggingAction::RaiseSeverityFloor)),
        "plain s must still raise the Logging severity floor"
    );
}

/// The same class of bug, one letter over: every plain-letter arm the app never
/// meant to bind under Ctrl is now unbound, while the four real Ctrl bindings
/// (`Ctrl-C` quit, `Ctrl-T` Tunnels, `Ctrl-S` Logging save, `Ctrl-Space` in the
/// context-edit form) keep dispatching.
#[test]
fn ctrl_letters_are_unbound_where_only_the_plain_letter_is_a_binding() {
    let list = normal_resources_list_context();
    for ch in ['i', 'j', 'k', 'l', 'n', 'v', 'w', 'y'] {
        assert_eq!(
            action_for_key(
                modified_press(KeyCode::Char(ch), KeyModifiers::CONTROL),
                &list
            ),
            None,
            "Ctrl-{ch} must be unbound in the resource list"
        );
    }

    // The plain and shifted forms are untouched.
    assert_eq!(
        action_for_key(press(KeyCode::Char('l')), &list),
        Some(Action::Workspace(WorkspaceAction::Logging))
    );
    assert_eq!(
        action_for_key(press(KeyCode::Down), &list),
        Some(Action::Resources(ResourcesAction::SelectNext)),
        "the bare Down arrow carries no modifiers and must keep working"
    );
    assert_eq!(
        action_for_key(
            modified_press(KeyCode::Char('L'), KeyModifiers::SHIFT),
            &list
        ),
        Some(Action::Resources(ResourcesAction::OpenLogsForSelection))
    );

    // The real Ctrl bindings survive.
    assert_eq!(
        action_for_key(
            modified_press(KeyCode::Char('t'), KeyModifiers::CONTROL),
            &list
        ),
        Some(Action::Workspace(WorkspaceAction::Tunnels))
    );
    assert_eq!(action_for_key(ctrl_c(), &list), Some(Action::QuitRequested));
}

/// The plugin confirmation's `Enter` is gated on the acknowledgement, and the
/// editing keys are not.
///
/// The parity walk only ever probes the satisfied state — an ordinary plugin —
/// so the unsatisfied half of the guard has no coverage there: dropping
/// `&& context.plugin_confirmation_ack_satisfied` from the keymap arm would let
/// a `dangerous` plugin run on a bare `Enter` with nothing typed, and every
/// other test in the suite would still pass. `Backspace` and the digits are
/// asserted in that same unsatisfied state because that is the only state a
/// user can type themselves out of.
#[test]
fn plugin_confirmation_enter_waits_for_the_acknowledgement() {
    let mut confirmation = context(Workspace::Resources, Mode::PluginConfirmation);
    confirmation.plugin_confirmation_open = true;
    confirmation.plugin_confirmation_ack_satisfied = false;
    // Held true throughout: this test is about the acknowledgement guard, and
    // the visibility guard has its own test below.
    confirmation.plugin_confirmation_visible = true;

    assert_eq!(
        action_for_key(press(KeyCode::Enter), &confirmation),
        None,
        "a dangerous plugin must not run before its acknowledgement is typed"
    );
    assert_eq!(
        action_for_key(press(KeyCode::Char('1')), &confirmation),
        Some(Action::Resources(ResourcesAction::PluginAckAppendDigit(
            '1'
        )))
    );
    assert_eq!(
        action_for_key(press(KeyCode::Backspace), &confirmation),
        Some(Action::Resources(ResourcesAction::PluginAckBackspace))
    );
    assert_eq!(
        action_for_key(press(KeyCode::Esc), &confirmation),
        Some(Action::Resources(ResourcesAction::CancelPluginConfirmation))
    );

    confirmation.plugin_confirmation_ack_satisfied = true;

    assert_eq!(
        action_for_key(press(KeyCode::Enter), &confirmation),
        Some(Action::Resources(ResourcesAction::ConfirmPlugin))
    );
    assert_eq!(
        action_for_key(ctrl_c(), &confirmation),
        Some(Action::QuitRequested)
    );
}

/// A plugin cannot be run from a modal the terminal is too small to draw.
///
/// At that size `ui::render_plugin_confirmation` draws "nuvem plugin
/// confirmation needs a larger terminal" and no argv, so an armed `Enter` would
/// execute a command the operator was never shown — the same guard
/// `resource_command_preview_key` and `ssh_launch_preview_key` carry, and the
/// one this modal needs most, since its `Enter` runs rather than copies.
///
/// The other three keys stay armed on purpose, and are asserted here so a guard
/// widened to cover the whole arm fails: `Esc` has to close a modal that cannot
/// be read, and typing into an unreadable acknowledgement changes nothing on
/// its own.
#[test]
fn plugin_confirmation_enter_waits_for_a_readable_modal() {
    let mut confirmation = context(Workspace::Resources, Mode::PluginConfirmation);
    confirmation.plugin_confirmation_open = true;
    confirmation.plugin_confirmation_ack_satisfied = true;
    confirmation.plugin_confirmation_visible = false;

    assert_eq!(
        action_for_key(press(KeyCode::Enter), &confirmation),
        None,
        "a plugin must not run from a modal whose argv the terminal cannot show"
    );
    assert_eq!(
        action_for_key(press(KeyCode::Esc), &confirmation),
        Some(Action::Resources(ResourcesAction::CancelPluginConfirmation)),
        "cancelling has to work at any terminal size"
    );
    assert_eq!(
        action_for_key(press(KeyCode::Char('1')), &confirmation),
        Some(Action::Resources(ResourcesAction::PluginAckAppendDigit(
            '1'
        )))
    );
    assert_eq!(
        action_for_key(press(KeyCode::Backspace), &confirmation),
        Some(Action::Resources(ResourcesAction::PluginAckBackspace))
    );

    confirmation.plugin_confirmation_visible = true;

    assert_eq!(
        action_for_key(press(KeyCode::Enter), &confirmation),
        Some(Action::Resources(ResourcesAction::ConfirmPlugin)),
        "and it runs once the modal is readable"
    );
}

/// `,` arms from the resource list and from a detail pane, the two sites that
/// bind `J`: both have a resource on screen for the gesture to be about.
#[test]
fn comma_arms_the_plugin_prefix_from_the_list_and_the_detail() {
    for detail_open in [false, true] {
        let mut context = base_context();
        context.workspace = Workspace::Resources;
        context.resources_detail_open = detail_open;

        assert_eq!(
            action_for_key(press(KeyCode::Char(',')), &context),
            Some(Action::Resources(ResourcesAction::ArmPluginPrefix)),
            "detail_open = {detail_open}"
        );
    }
}

/// `,` is not bound outside the resource views, where there is no selection
/// for a plugin to be about — the same rule `J` follows.
#[test]
fn comma_is_unbound_outside_the_resource_views() {
    for workspace in [
        Workspace::Home,
        Workspace::Configurations,
        Workspace::Logging,
        Workspace::Tunnels,
    ] {
        let mut context = base_context();
        context.workspace = workspace;

        assert_eq!(
            action_for_key(press(KeyCode::Char(',')), &context),
            None,
            "{workspace:?}"
        );
    }
}

/// The focused service navigator is inside the Resources workspace and still
/// does not arm: its cursor is over a *service*, not a row, so there is no
/// resource for a plugin to be about — exactly why `J` is unbound there too.
/// Pinned because the comments on the reducer's `PluginPrefixPress` arm rest on
/// where `,` binds, and "the resource views" is narrower than "the workspace".
#[test]
fn comma_is_unbound_while_the_service_navigator_is_focused() {
    for detail_open in [false, true] {
        let mut context = base_context();
        context.workspace = Workspace::Resources;
        context.resources_detail_open = detail_open;
        context.service_sidebar_visible = true;
        context.service_navigator_focused = true;

        assert_eq!(
            action_for_key(press(KeyCode::Char(',')), &context),
            None,
            "detail_open = {detail_open}"
        );
    }
}

/// Every character reaches the reducer, because only the reducer can tell a
/// bound-but-unresolvable key from one that was never declared, and those are
/// different sentences.
#[test]
fn an_armed_prefix_sends_any_character_to_the_reducer() {
    let mut context = base_context();
    context.mode = Mode::PluginPrefix;
    context.workspace = Workspace::Resources;

    for ch in ['a', 'D', 'z', '7'] {
        assert_eq!(
            action_for_key(press(KeyCode::Char(ch)), &context),
            Some(Action::Resources(ResourcesAction::PluginPrefixPress(ch))),
            "{ch:?}"
        );
    }
}

/// Esc cancels.
#[test]
fn an_armed_prefix_is_cancelled_by_esc() {
    let mut context = base_context();
    context.mode = Mode::PluginPrefix;
    context.workspace = Workspace::Resources;

    assert_eq!(
        action_for_key(press(KeyCode::Esc), &context),
        Some(Action::Resources(ResourcesAction::CancelPluginPrefix))
    );
}

/// A non-character key is swallowed rather than falling through to the list
/// bindings underneath: an armed prefix that let `j` move the cursor would
/// leave the operator's next character resolving as a plugin key.
#[test]
fn an_armed_prefix_swallows_a_non_character_key() {
    let mut context = base_context();
    context.mode = Mode::PluginPrefix;
    context.workspace = Workspace::Resources;

    for code in [
        KeyCode::Down,
        KeyCode::Enter,
        KeyCode::Backspace,
        KeyCode::Tab,
    ] {
        assert_eq!(action_for_key(press(code), &context), None, "{code:?}");
    }
}

/// A chord is not a plugin key: `Ctrl-x` with a prefix armed must not resolve
/// as `PluginPrefixPress('x')` and run whatever plugin declares `x`. That is
/// this test's claim, and the hoist did not touch it — `plugin_prefix_key` still
/// takes only `is_plain_text_key` characters.
///
/// What changed is where the chord lands instead. It used to be `None`,
/// because `Ctrl-x` was an arm of `normal_key`; it is now
/// `CancelRunningPlugins`, because the arm moved up beside `Ctrl-C` in
/// `action_for_key`. An armed prefix is the sharpest case for that move: it
/// renders no modal, so a dead cancel key here had nothing on screen to explain
/// it.
#[test]
fn an_armed_prefix_reads_a_chord_as_the_cancel_key_not_a_plugin_key() {
    let mut context = base_context();
    context.mode = Mode::PluginPrefix;
    context.workspace = Workspace::Resources;

    assert_eq!(
        action_for_key(
            modified_press(KeyCode::Char('x'), KeyModifiers::CONTROL),
            &context
        ),
        Some(Action::System(SystemAction::CancelRunningPlugins))
    );
}

/// The typed-input prompt's context: open, every field filled, and
/// the modal small enough to draw — the state the catalog's `Enter` entry
/// claims to work in.
fn plugin_prompt_context() -> Context {
    Context {
        mode: Mode::PluginPrompt,
        workspace: Workspace::Resources,
        plugin_prompt_open: true,
        plugin_prompt_satisfied: true,
        plugin_prompt_visible: true,
        ..base_context()
    }
}

/// The prompt's editing keys: typing reaches the focused field, `Tab` and its
/// shifted face move between fields, the arrows cycle a dropdown, and `Esc`
/// abandons the whole gesture.
#[test]
fn the_plugin_prompt_edits_moves_and_cancels() {
    let prompt = plugin_prompt_context();

    assert_eq!(
        action_for_key(press(KeyCode::Char('5')), &prompt),
        Some(Action::Resources(ResourcesAction::PluginPromptAppend('5')))
    );
    assert_eq!(
        action_for_key(press(KeyCode::Backspace), &prompt),
        Some(Action::Resources(ResourcesAction::PluginPromptBackspace))
    );
    assert_eq!(
        action_for_key(press(KeyCode::Tab), &prompt),
        Some(Action::Resources(ResourcesAction::PluginPromptNextField))
    );
    assert_eq!(
        action_for_key(press(KeyCode::BackTab), &prompt),
        Some(Action::Resources(
            ResourcesAction::PluginPromptPreviousField
        ))
    );
    assert_eq!(
        action_for_key(press(KeyCode::Right), &prompt),
        Some(Action::Resources(ResourcesAction::PluginPromptCycleChoice(
            true
        )))
    );
    assert_eq!(
        action_for_key(press(KeyCode::Left), &prompt),
        Some(Action::Resources(ResourcesAction::PluginPromptCycleChoice(
            false
        )))
    );
    assert_eq!(
        action_for_key(press(KeyCode::Esc), &prompt),
        Some(Action::Resources(ResourcesAction::CancelPluginPrompt))
    );
    assert_eq!(
        action_for_key(press(KeyCode::Enter), &prompt),
        Some(Action::Resources(ResourcesAction::SubmitPluginPrompt))
    );
}

/// `Enter` waits for every field to hold a value.
///
/// The parity walk only ever probes the satisfied state, so the unsatisfied
/// half of this guard has no coverage there: dropping
/// `&& context.plugin_prompt_satisfied` from the keymap arm would compose an
/// argv from a blank the operator never filled in, and every other test in the
/// suite would still pass. The editing keys are asserted in that same state,
/// because typing is the only way out of it.
#[test]
fn the_plugin_prompt_enter_waits_for_every_field() {
    let mut prompt = plugin_prompt_context();
    prompt.plugin_prompt_satisfied = false;

    assert_eq!(
        action_for_key(press(KeyCode::Enter), &prompt),
        None,
        "an argv must not be composed from a field the operator left blank"
    );
    assert_eq!(
        action_for_key(press(KeyCode::Char('a')), &prompt),
        Some(Action::Resources(ResourcesAction::PluginPromptAppend('a')))
    );
    assert_eq!(
        action_for_key(press(KeyCode::Esc), &prompt),
        Some(Action::Resources(ResourcesAction::CancelPluginPrompt))
    );

    prompt.plugin_prompt_satisfied = true;

    assert_eq!(
        action_for_key(press(KeyCode::Enter), &prompt),
        Some(Action::Resources(ResourcesAction::SubmitPluginPrompt))
    );
}

/// `Enter` also waits for a modal the terminal can draw, the guard the
/// confirmation behind it carries for the same reason: a submit composes the
/// argv the operator is about to approve, and the values it is composed from
/// are the ones on a screen too small to show them.
///
/// The other keys stay armed on purpose, and are asserted here so a guard
/// widened to cover the whole arm fails: `Esc` has to close a modal that cannot
/// be read.
#[test]
fn the_plugin_prompt_enter_waits_for_a_readable_modal() {
    let mut prompt = plugin_prompt_context();
    prompt.plugin_prompt_visible = false;

    assert_eq!(
        action_for_key(press(KeyCode::Enter), &prompt),
        None,
        "a prompt the terminal cannot show must not compose an argv"
    );
    assert_eq!(
        action_for_key(press(KeyCode::Esc), &prompt),
        Some(Action::Resources(ResourcesAction::CancelPluginPrompt)),
        "cancelling has to work at any terminal size"
    );
    assert_eq!(
        action_for_key(press(KeyCode::Char('a')), &prompt),
        Some(Action::Resources(ResourcesAction::PluginPromptAppend('a')))
    );

    prompt.plugin_prompt_visible = true;

    assert_eq!(
        action_for_key(press(KeyCode::Enter), &prompt),
        Some(Action::Resources(ResourcesAction::SubmitPluginPrompt)),
        "and it submits once the modal is readable"
    );
}

/// The prompt takes every printable character, so the cancel chord has to keep
/// resolving ahead of it: a run started before the prompt opened
/// outlives it, and `is_plain_text_key` is what keeps `Ctrl-x` from being typed
/// into a field as a literal `x`.
#[test]
fn ctrl_x_cancels_running_plugins_from_inside_the_plugin_prompt() {
    assert_eq!(
        action_for_key(
            modified_press(KeyCode::Char('x'), KeyModifiers::CONTROL),
            &plugin_prompt_context()
        ),
        Some(Action::System(SystemAction::CancelRunningPlugins))
    );
    assert_eq!(
        action_for_key(ctrl_c(), &plugin_prompt_context()),
        Some(Action::QuitRequested)
    );
}
