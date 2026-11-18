use nuvem::app::reducer::root::reduce;
use nuvem::app::state::configurations::{
    ConfigurationContextEditField, ConfigurationContextPickerItem, ConfigurationContextPickerKind,
    ConfigurationContextPickerStatus, ConfigurationCreateField,
};
use nuvem::app::{Action, AppError, AppState, ConfigurationsAction, Effect, EffectResult};
use nuvem::services::gcloud::configurations::{GcloudConfigurationDraft, GcloudContextEditDraft};
use nuvem_domain::gcp::configuration::{GcloudConfiguration, GcloudConfigurationName};
use nuvem_domain::gcp::location::ComputeRegion;
use nuvem_domain::gcp::project::GcloudProject;

fn config(name: &str, active: bool, project: &str) -> GcloudConfiguration {
    GcloudConfiguration {
        name: GcloudConfigurationName::new(name),
        is_active: active,
        account: Some(format!("{name}@example.com")),
        project: Some(project.to_string()),
        region: None,
        zone: None,
    }
}

fn config_with_context(
    name: &str,
    active: bool,
    project: Option<&str>,
    region: Option<&str>,
    zone: Option<&str>,
) -> GcloudConfiguration {
    GcloudConfiguration {
        name: GcloudConfigurationName::new(name),
        is_active: active,
        account: Some(format!("{name}@example.com")),
        project: project.map(str::to_string),
        region: region.map(str::to_string),
        zone: zone.map(str::to_string),
    }
}

fn app_with_context_edit() -> AppState {
    let mut app = AppState::new();
    app.root.configurations.set_rows(vec![config_with_context(
        "dev",
        true,
        Some("dev-project"),
        Some("europe-west1"),
        Some("europe-west1-b"),
    )]);
    reduce(
        &mut app,
        &Action::Configurations(ConfigurationsAction::StartEditActiveContext),
    );
    app
}

fn project_item(project_id: &str, name: &str) -> ConfigurationContextPickerItem {
    ConfigurationContextPickerItem::Project(GcloudProject {
        project_id: project_id.to_string(),
        name: name.to_string(),
        lifecycle_state: Some("ACTIVE".to_string()),
    })
}

#[test]
fn loading_configurations_sets_active_context() {
    let mut app = AppState::new();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ConfigurationsLoaded {
            configurations: vec![
                config("dev", true, "dev-project"),
                config("prod", false, "prod-project"),
            ],
        }),
    );

    assert_eq!(
        app.root.configurations.active_name().unwrap().as_str(),
        "dev"
    );
    assert_eq!(
        app.root.current_context().unwrap().project.as_deref(),
        Some("dev-project")
    );
}

#[test]
fn configuration_activation_request_opens_confirmation_without_effect() {
    let mut app = AppState::new();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ConfigurationsLoaded {
            configurations: vec![
                config("dev", true, "dev-project"),
                config("prod", false, "prod-project"),
            ],
        }),
    );
    reduce(
        &mut app,
        &Action::Configurations(ConfigurationsAction::SelectNext),
    );

    let effects = reduce(
        &mut app,
        &Action::Configurations(ConfigurationsAction::RequestActivateSelected),
    );

    assert!(effects.is_empty());
    let confirmation = app
        .root
        .configurations
        .activation_confirmation()
        .expect("activation confirmation is open");
    assert_eq!(confirmation.target().name.as_str(), "prod");
    assert_eq!(
        confirmation.target().account.as_deref(),
        Some("prod@example.com")
    );
    assert_eq!(
        confirmation.target().project.as_deref(),
        Some("prod-project")
    );
    assert_eq!(
        confirmation.command_preview(),
        "gcloud config configurations activate prod"
    );
}

#[test]
fn configuration_activation_confirmation_returns_activate_effect() {
    let mut app = AppState::new();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ConfigurationsLoaded {
            configurations: vec![
                config("dev", true, "dev-project"),
                config("prod", false, "prod-project"),
            ],
        }),
    );
    reduce(
        &mut app,
        &Action::Configurations(ConfigurationsAction::SelectNext),
    );
    reduce(
        &mut app,
        &Action::Configurations(ConfigurationsAction::RequestActivateSelected),
    );

    let effects = reduce(
        &mut app,
        &Action::Configurations(ConfigurationsAction::ConfirmActivation),
    );

    assert!(matches!(
        effects.as_slice(),
        [Effect::ActivateConfiguration { name }] if name.as_str() == "prod"
    ));
    assert!(app.root.configurations.activation_confirmation().is_none());
}

#[test]
fn configuration_activation_cancellation_closes_confirmation_without_effect() {
    let mut app = AppState::new();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ConfigurationsLoaded {
            configurations: vec![
                config("dev", true, "dev-project"),
                config("prod", false, "prod-project"),
            ],
        }),
    );
    reduce(
        &mut app,
        &Action::Configurations(ConfigurationsAction::SelectNext),
    );
    reduce(
        &mut app,
        &Action::Configurations(ConfigurationsAction::RequestActivateSelected),
    );

    let effects = reduce(
        &mut app,
        &Action::Configurations(ConfigurationsAction::CancelActivation),
    );

    assert!(effects.is_empty());
    assert!(app.root.configurations.activation_confirmation().is_none());
}

#[test]
fn starting_other_configuration_modes_closes_activation_confirmation() {
    let mut app = AppState::new();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ConfigurationsLoaded {
            configurations: vec![
                config("dev", true, "dev-project"),
                config("prod", false, "prod-project"),
            ],
        }),
    );
    reduce(
        &mut app,
        &Action::Configurations(ConfigurationsAction::SelectNext),
    );

    for action in [
        ConfigurationsAction::StartCreate,
        ConfigurationsAction::StartEditActiveContext,
        ConfigurationsAction::StartFiltering,
    ] {
        reduce(
            &mut app,
            &Action::Configurations(ConfigurationsAction::RequestActivateSelected),
        );
        assert!(app.root.configurations.activation_confirmation().is_some());

        let effects = reduce(&mut app, &Action::Configurations(action));

        assert!(effects.is_empty());
        assert!(app.root.configurations.activation_confirmation().is_none());
    }
}

#[test]
fn configuration_activation_request_for_active_row_is_a_no_op() {
    let mut app = AppState::new();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ConfigurationsLoaded {
            configurations: vec![config("dev", true, "dev-project")],
        }),
    );

    let effects = reduce(
        &mut app,
        &Action::Configurations(ConfigurationsAction::RequestActivateSelected),
    );

    assert!(effects.is_empty());
    assert!(app.root.configurations.activation_confirmation().is_none());
    assert_eq!(
        app.root.system.status_message.as_deref(),
        Some("Configuration is already active")
    );
}

#[test]
fn successful_configuration_load_clears_stale_status() {
    let mut app = AppState::new();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::Failed {
            error: AppError::gcloud("list failed"),
        }),
    );

    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ConfigurationsLoaded {
            configurations: vec![config("dev", true, "dev-project")],
        }),
    );

    assert_eq!(app.root.system.status_message, None);
}

#[test]
fn start_create_opens_empty_form_focused_on_name() {
    let mut app = AppState::new();

    let effects = reduce(
        &mut app,
        &Action::Configurations(ConfigurationsAction::StartCreate),
    );

    assert!(effects.is_empty());
    let form = app.root.configurations.create_form().expect("form is open");
    assert_eq!(form.focused_field(), ConfigurationCreateField::Name);
    assert_eq!(form.draft().name, "");
    assert_eq!(form.draft().account, None);
    assert_eq!(form.draft().project, None);
    assert_eq!(form.draft().region, None);
    assert_eq!(form.draft().zone, None);
}

#[test]
fn create_form_typing_backspace_and_optional_fields_update_focused_field() {
    let mut app = AppState::new();
    reduce(
        &mut app,
        &Action::Configurations(ConfigurationsAction::StartCreate),
    );
    for ch in "dev".chars() {
        reduce(
            &mut app,
            &Action::Configurations(ConfigurationsAction::AppendCreateChar(ch)),
        );
    }
    reduce(
        &mut app,
        &Action::Configurations(ConfigurationsAction::BackspaceCreateField),
    );
    reduce(
        &mut app,
        &Action::Configurations(ConfigurationsAction::FocusNextCreateField),
    );
    for ch in "dev@example.com".chars() {
        reduce(
            &mut app,
            &Action::Configurations(ConfigurationsAction::AppendCreateChar(ch)),
        );
    }
    reduce(
        &mut app,
        &Action::Configurations(ConfigurationsAction::BackspaceCreateField),
    );

    let form = app.root.configurations.create_form().expect("form is open");
    assert_eq!(form.draft().name, "de");
    assert_eq!(form.draft().account.as_deref(), Some("dev@example.co"));
    assert_eq!(form.field_value(ConfigurationCreateField::Name), "de");
    assert_eq!(
        form.field_value(ConfigurationCreateField::Account),
        "dev@example.co"
    );
}

#[test]
fn create_form_focus_cycles_forward_and_backward() {
    let mut app = AppState::new();
    reduce(
        &mut app,
        &Action::Configurations(ConfigurationsAction::StartCreate),
    );

    for expected in [
        ConfigurationCreateField::Account,
        ConfigurationCreateField::Project,
        ConfigurationCreateField::Region,
        ConfigurationCreateField::Zone,
        ConfigurationCreateField::Name,
    ] {
        reduce(
            &mut app,
            &Action::Configurations(ConfigurationsAction::FocusNextCreateField),
        );
        assert_eq!(
            app.root
                .configurations
                .create_form()
                .expect("form is open")
                .focused_field(),
            expected
        );
    }

    for expected in [
        ConfigurationCreateField::Zone,
        ConfigurationCreateField::Region,
        ConfigurationCreateField::Project,
        ConfigurationCreateField::Account,
        ConfigurationCreateField::Name,
    ] {
        reduce(
            &mut app,
            &Action::Configurations(ConfigurationsAction::FocusPreviousCreateField),
        );
        assert_eq!(
            app.root
                .configurations
                .create_form()
                .expect("form is open")
                .focused_field(),
            expected
        );
    }
}

#[test]
fn submitting_empty_create_form_sets_status_and_emits_no_effect() {
    let mut app = AppState::new();
    app.root.system.begin_operation(
        nuvem::app::state::system::OperationPhase::Healthy,
        "GCE instances",
    );
    reduce(
        &mut app,
        &Action::Configurations(ConfigurationsAction::StartCreate),
    );

    let effects = reduce(
        &mut app,
        &Action::Configurations(ConfigurationsAction::SubmitCreate),
    );

    assert!(effects.is_empty());
    assert!(app.root.configurations.is_create_form_open());
    assert_eq!(
        app.root
            .configurations
            .create_form()
            .expect("form remains open")
            .focused_field(),
        ConfigurationCreateField::Name
    );
    assert_eq!(
        app.root.system.status_message.as_deref(),
        Some("Configuration name is required")
    );
    assert_eq!(app.root.system.status_operation_label, None);
    assert_eq!(
        app.root.system.status_phase,
        Some(nuvem::app::state::system::OperationPhase::Failed)
    );
}

#[test]
fn cancel_after_empty_create_validation_clears_local_status() {
    let mut app = AppState::new();
    reduce(
        &mut app,
        &Action::Configurations(ConfigurationsAction::StartCreate),
    );
    reduce(
        &mut app,
        &Action::Configurations(ConfigurationsAction::SubmitCreate),
    );

    let effects = reduce(
        &mut app,
        &Action::Configurations(ConfigurationsAction::CancelCreate),
    );

    assert!(effects.is_empty());
    assert!(!app.root.configurations.is_create_form_open());
    assert_eq!(app.root.system.status_message, None);
}

#[test]
fn submitting_valid_create_form_trims_values_closes_form_and_emits_create_effect() {
    let mut app = AppState::new();
    reduce(
        &mut app,
        &Action::Configurations(ConfigurationsAction::StartCreate),
    );
    for ch in " dev ".chars() {
        reduce(
            &mut app,
            &Action::Configurations(ConfigurationsAction::AppendCreateChar(ch)),
        );
    }
    reduce(
        &mut app,
        &Action::Configurations(ConfigurationsAction::FocusNextCreateField),
    );
    for ch in "  ".chars() {
        reduce(
            &mut app,
            &Action::Configurations(ConfigurationsAction::AppendCreateChar(ch)),
        );
    }
    reduce(
        &mut app,
        &Action::Configurations(ConfigurationsAction::FocusNextCreateField),
    );
    for ch in " dev-project ".chars() {
        reduce(
            &mut app,
            &Action::Configurations(ConfigurationsAction::AppendCreateChar(ch)),
        );
    }
    reduce(
        &mut app,
        &Action::Configurations(ConfigurationsAction::FocusNextCreateField),
    );
    for ch in " europe-west1 ".chars() {
        reduce(
            &mut app,
            &Action::Configurations(ConfigurationsAction::AppendCreateChar(ch)),
        );
    }
    reduce(
        &mut app,
        &Action::Configurations(ConfigurationsAction::FocusNextCreateField),
    );
    for ch in " europe-west1-b ".chars() {
        reduce(
            &mut app,
            &Action::Configurations(ConfigurationsAction::AppendCreateChar(ch)),
        );
    }

    let effects = reduce(
        &mut app,
        &Action::Configurations(ConfigurationsAction::SubmitCreate),
    );

    assert!(!app.root.configurations.is_create_form_open());
    assert_eq!(
        effects,
        vec![Effect::CreateConfiguration {
            draft: GcloudConfigurationDraft {
                name: "dev".to_string(),
                account: None,
                project: Some("dev-project".to_string()),
                region: Some("europe-west1".to_string()),
                zone: Some("europe-west1-b".to_string()),
            },
        }]
    );
}

#[test]
fn cancel_create_form_discards_draft_and_emits_no_effect() {
    let mut app = AppState::new();
    reduce(
        &mut app,
        &Action::Configurations(ConfigurationsAction::StartCreate),
    );
    for ch in "dev".chars() {
        reduce(
            &mut app,
            &Action::Configurations(ConfigurationsAction::AppendCreateChar(ch)),
        );
    }

    let effects = reduce(
        &mut app,
        &Action::Configurations(ConfigurationsAction::CancelCreate),
    );

    assert!(effects.is_empty());
    assert!(!app.root.configurations.is_create_form_open());

    reduce(
        &mut app,
        &Action::Configurations(ConfigurationsAction::StartCreate),
    );
    assert_eq!(
        app.root
            .configurations
            .create_form()
            .expect("new form is open")
            .draft()
            .name,
        ""
    );
}

#[test]
fn start_edit_active_context_prefills_active_project_region_and_zone() {
    let mut app = AppState::new();
    app.root.configurations.set_rows(vec![config_with_context(
        "dev",
        true,
        Some("dev-project"),
        Some("europe-west1"),
        Some("europe-west1-b"),
    )]);

    let effects = reduce(
        &mut app,
        &Action::Configurations(ConfigurationsAction::StartEditActiveContext),
    );

    assert!(effects.is_empty());
    let form = app
        .root
        .configurations
        .context_edit_form()
        .expect("form is open");
    assert_eq!(form.target().as_str(), "dev");
    assert_eq!(form.draft().configuration.as_str(), "dev");
    assert_eq!(form.draft().project.as_deref(), Some("dev-project"));
    assert_eq!(form.draft().region.as_deref(), Some("europe-west1"));
    assert_eq!(form.draft().zone.as_deref(), Some("europe-west1-b"));
    assert_eq!(form.focused_field(), ConfigurationContextEditField::Project);
}

#[test]
fn start_edit_active_context_without_active_configuration_sets_status() {
    let mut app = AppState::new();
    app.root.configurations.set_rows(vec![config_with_context(
        "dev",
        false,
        Some("dev-project"),
        None,
        None,
    )]);

    let effects = reduce(
        &mut app,
        &Action::Configurations(ConfigurationsAction::StartEditActiveContext),
    );

    assert!(effects.is_empty());
    assert!(!app.root.configurations.is_context_edit_form_open());
    assert_eq!(
        app.root.system.status_message.as_deref(),
        Some("No active gcloud configuration")
    );
}

#[test]
fn start_create_after_failed_context_edit_clears_no_active_status() {
    let mut app = AppState::new();
    app.root.configurations.set_rows(vec![config_with_context(
        "dev",
        false,
        Some("dev-project"),
        None,
        None,
    )]);
    reduce(
        &mut app,
        &Action::Configurations(ConfigurationsAction::StartEditActiveContext),
    );

    let effects = reduce(
        &mut app,
        &Action::Configurations(ConfigurationsAction::StartCreate),
    );

    assert!(effects.is_empty());
    assert!(app.root.configurations.is_create_form_open());
    assert_eq!(app.root.system.status_message, None);
}

#[test]
fn start_context_edit_after_invalid_create_clears_required_name_status() {
    let mut app = AppState::new();
    app.root.configurations.set_rows(vec![config_with_context(
        "dev",
        true,
        Some("dev-project"),
        Some("europe-west1"),
        Some("europe-west1-b"),
    )]);
    reduce(
        &mut app,
        &Action::Configurations(ConfigurationsAction::StartCreate),
    );
    reduce(
        &mut app,
        &Action::Configurations(ConfigurationsAction::SubmitCreate),
    );

    let effects = reduce(
        &mut app,
        &Action::Configurations(ConfigurationsAction::StartEditActiveContext),
    );

    assert!(effects.is_empty());
    assert!(app.root.configurations.is_context_edit_form_open());
    assert_eq!(app.root.system.status_message, None);
}

#[test]
fn context_edit_typing_backspace_and_optional_emptying_update_focused_field() {
    let mut app = AppState::new();
    app.root.configurations.set_rows(vec![config_with_context(
        "dev",
        true,
        Some("dev-project"),
        None,
        None,
    )]);
    reduce(
        &mut app,
        &Action::Configurations(ConfigurationsAction::StartEditActiveContext),
    );
    for _ in "dev-project".chars() {
        reduce(
            &mut app,
            &Action::Configurations(ConfigurationsAction::BackspaceContextEditField),
        );
    }
    for ch in "new-project".chars() {
        reduce(
            &mut app,
            &Action::Configurations(ConfigurationsAction::AppendContextEditChar(ch)),
        );
    }
    reduce(
        &mut app,
        &Action::Configurations(ConfigurationsAction::FocusNextContextEditField),
    );
    reduce(
        &mut app,
        &Action::Configurations(ConfigurationsAction::AppendContextEditChar('x')),
    );
    reduce(
        &mut app,
        &Action::Configurations(ConfigurationsAction::BackspaceContextEditField),
    );

    let form = app
        .root
        .configurations
        .context_edit_form()
        .expect("form is open");
    assert_eq!(form.draft().project.as_deref(), Some("new-project"));
    assert_eq!(form.draft().region, None);
    assert_eq!(
        form.field_value(ConfigurationContextEditField::Project),
        "new-project"
    );
    assert_eq!(form.field_value(ConfigurationContextEditField::Region), "");
}

#[test]
fn context_edit_focus_cycles_forward_and_backward() {
    let mut app = AppState::new();
    app.root.configurations.set_rows(vec![config_with_context(
        "dev",
        true,
        Some("dev-project"),
        Some("europe-west1"),
        Some("europe-west1-b"),
    )]);
    reduce(
        &mut app,
        &Action::Configurations(ConfigurationsAction::StartEditActiveContext),
    );

    for expected in [
        ConfigurationContextEditField::Region,
        ConfigurationContextEditField::Zone,
        ConfigurationContextEditField::Project,
    ] {
        reduce(
            &mut app,
            &Action::Configurations(ConfigurationsAction::FocusNextContextEditField),
        );
        assert_eq!(
            app.root
                .configurations
                .context_edit_form()
                .expect("form is open")
                .focused_field(),
            expected
        );
    }

    for expected in [
        ConfigurationContextEditField::Zone,
        ConfigurationContextEditField::Region,
        ConfigurationContextEditField::Project,
    ] {
        reduce(
            &mut app,
            &Action::Configurations(ConfigurationsAction::FocusPreviousContextEditField),
        );
        assert_eq!(
            app.root
                .configurations
                .context_edit_form()
                .expect("form is open")
                .focused_field(),
            expected
        );
    }
}

#[test]
fn submitting_context_edit_trims_values_closes_form_and_emits_update_effect() {
    let mut app = AppState::new();
    app.root.configurations.set_rows(vec![config_with_context(
        "dev",
        true,
        Some("dev-project"),
        Some("europe-west1"),
        None,
    )]);
    reduce(
        &mut app,
        &Action::Configurations(ConfigurationsAction::StartEditActiveContext),
    );
    for _ in "dev-project".chars() {
        reduce(
            &mut app,
            &Action::Configurations(ConfigurationsAction::BackspaceContextEditField),
        );
    }
    for ch in " new-project ".chars() {
        reduce(
            &mut app,
            &Action::Configurations(ConfigurationsAction::AppendContextEditChar(ch)),
        );
    }
    reduce(
        &mut app,
        &Action::Configurations(ConfigurationsAction::FocusNextContextEditField),
    );
    for _ in "europe-west1".chars() {
        reduce(
            &mut app,
            &Action::Configurations(ConfigurationsAction::BackspaceContextEditField),
        );
    }
    for ch in "  ".chars() {
        reduce(
            &mut app,
            &Action::Configurations(ConfigurationsAction::AppendContextEditChar(ch)),
        );
    }
    reduce(
        &mut app,
        &Action::Configurations(ConfigurationsAction::FocusNextContextEditField),
    );
    for ch in " europe-west1-b ".chars() {
        reduce(
            &mut app,
            &Action::Configurations(ConfigurationsAction::AppendContextEditChar(ch)),
        );
    }

    let effects = reduce(
        &mut app,
        &Action::Configurations(ConfigurationsAction::SubmitContextEdit),
    );

    assert!(!app.root.configurations.is_context_edit_form_open());
    assert_eq!(
        effects,
        vec![Effect::UpdateConfigurationContext {
            draft: GcloudContextEditDraft {
                configuration: GcloudConfigurationName::new("dev"),
                project: Some("new-project".to_string()),
                region: None,
                zone: Some("europe-west1-b".to_string()),
            },
        }]
    );
}

#[test]
fn cancel_context_edit_discards_draft_and_emits_no_effect() {
    let mut app = AppState::new();
    app.root.configurations.set_rows(vec![config_with_context(
        "dev",
        true,
        Some("dev-project"),
        Some("europe-west1"),
        Some("europe-west1-b"),
    )]);
    reduce(
        &mut app,
        &Action::Configurations(ConfigurationsAction::StartEditActiveContext),
    );
    reduce(
        &mut app,
        &Action::Configurations(ConfigurationsAction::AppendContextEditChar('x')),
    );

    let effects = reduce(
        &mut app,
        &Action::Configurations(ConfigurationsAction::CancelContextEdit),
    );

    assert!(effects.is_empty());
    assert!(!app.root.configurations.is_context_edit_form_open());

    reduce(
        &mut app,
        &Action::Configurations(ConfigurationsAction::StartEditActiveContext),
    );
    assert_eq!(
        app.root
            .configurations
            .context_edit_form()
            .expect("new form is open")
            .draft()
            .project
            .as_deref(),
        Some("dev-project")
    );
}

#[test]
fn configuration_filter_draft_applies_cancels_and_clears() {
    let mut app = AppState::new();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ConfigurationsLoaded {
            configurations: vec![
                config_with_context(
                    "dev",
                    true,
                    Some("dev-project"),
                    Some("europe-west1"),
                    Some("europe-west1-b"),
                ),
                config_with_context(
                    "prod",
                    false,
                    Some("prod-project"),
                    Some("us-central1"),
                    Some("us-central1-a"),
                ),
            ],
        }),
    );

    reduce(
        &mut app,
        &Action::Configurations(ConfigurationsAction::StartFiltering),
    );
    assert!(app.root.configurations.is_filter_input_active());
    assert_eq!(app.root.configurations.filter_draft(), "");

    reduce(
        &mut app,
        &Action::Configurations(ConfigurationsAction::UpdateFilter("PROD".to_string())),
    );
    assert_eq!(app.root.configurations.filter_draft(), "PROD");
    assert_eq!(app.root.configurations.filter_applied(), "");
    assert_eq!(app.root.configurations.visible_rows().len(), 2);

    reduce(
        &mut app,
        &Action::Configurations(ConfigurationsAction::ApplyFilter),
    );
    assert!(!app.root.configurations.is_filter_input_active());
    assert_eq!(app.root.configurations.filter_applied(), "PROD");
    assert_eq!(
        app.root.configurations.selected().unwrap().name.as_str(),
        "prod"
    );
    assert_eq!(
        app.root
            .configurations
            .visible_rows()
            .into_iter()
            .map(|configuration| configuration.name.as_str())
            .collect::<Vec<_>>(),
        vec!["prod"]
    );

    reduce(
        &mut app,
        &Action::Configurations(ConfigurationsAction::StartFiltering),
    );
    reduce(
        &mut app,
        &Action::Configurations(ConfigurationsAction::UpdateFilter("dev".to_string())),
    );
    reduce(
        &mut app,
        &Action::Configurations(ConfigurationsAction::CancelFilter),
    );
    assert!(!app.root.configurations.is_filter_input_active());
    assert_eq!(app.root.configurations.filter_applied(), "PROD");
    assert_eq!(app.root.configurations.filter_draft(), "PROD");

    reduce(
        &mut app,
        &Action::Configurations(ConfigurationsAction::ClearFilter),
    );
    assert_eq!(app.root.configurations.filter_applied(), "");
    assert_eq!(app.root.configurations.filter_draft(), "");
    assert_eq!(app.root.configurations.visible_rows().len(), 2);
}

#[test]
fn open_project_picker_emits_project_load_effect() {
    let mut app = app_with_context_edit();
    let context = app.root.current_context().expect("active context");

    let effects = reduce(
        &mut app,
        &Action::Configurations(ConfigurationsAction::OpenContextPicker),
    );

    assert_eq!(
        effects,
        vec![Effect::LoadContextPicker {
            kind: ConfigurationContextPickerKind::Project,
            context,
        }]
    );
    assert_eq!(
        app.root
            .configurations
            .context_picker()
            .expect("picker is open")
            .status(),
        &ConfigurationContextPickerStatus::Loading
    );
}

#[test]
fn open_region_picker_emits_region_load_effect() {
    let mut app = app_with_context_edit();
    reduce(
        &mut app,
        &Action::Configurations(ConfigurationsAction::FocusNextContextEditField),
    );
    let context = app.root.current_context().expect("active context");

    let effects = reduce(
        &mut app,
        &Action::Configurations(ConfigurationsAction::OpenContextPicker),
    );

    assert!(matches!(
        effects.as_slice(),
        [Effect::LoadContextPicker {
            kind: ConfigurationContextPickerKind::Region,
            context: effect_context,
        }] if effect_context == &context
    ));
}

#[test]
fn open_zone_picker_emits_zone_load_effect() {
    let mut app = app_with_context_edit();
    for _ in 0..2 {
        reduce(
            &mut app,
            &Action::Configurations(ConfigurationsAction::FocusNextContextEditField),
        );
    }
    let context = app.root.current_context().expect("active context");

    let effects = reduce(
        &mut app,
        &Action::Configurations(ConfigurationsAction::OpenContextPicker),
    );

    assert!(matches!(
        effects.as_slice(),
        [Effect::LoadContextPicker {
            kind: ConfigurationContextPickerKind::Zone,
            context: effect_context,
        }] if effect_context == &context
    ));
}

#[test]
fn picker_filtering_is_case_insensitive_and_clamps_selection() {
    let mut app = app_with_context_edit();
    reduce(
        &mut app,
        &Action::Configurations(ConfigurationsAction::OpenContextPicker),
    );
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ContextPickerLoaded {
            configuration: GcloudConfigurationName::new("dev"),
            kind: ConfigurationContextPickerKind::Project,
            items: vec![
                project_item("dev-project", "Development"),
                project_item("prod-project", "Production"),
            ],
        }),
    );
    reduce(
        &mut app,
        &Action::Configurations(ConfigurationsAction::SelectNextContextPickerItem),
    );
    reduce(
        &mut app,
        &Action::Configurations(ConfigurationsAction::StartContextPickerFilter),
    );
    reduce(
        &mut app,
        &Action::Configurations(ConfigurationsAction::UpdateContextPickerFilter(
            "DEV".to_string(),
        )),
    );

    let picker = app
        .root
        .configurations
        .context_picker()
        .expect("picker is open");
    assert_eq!(picker.visible_items().len(), 1);
    assert_eq!(
        picker.selected_item(),
        Some(&project_item("dev-project", "Development"))
    );
}

#[test]
fn picker_filter_cancel_restores_the_applied_filter() {
    let mut app = app_with_context_edit();
    reduce(
        &mut app,
        &Action::Configurations(ConfigurationsAction::OpenContextPicker),
    );
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ContextPickerLoaded {
            configuration: GcloudConfigurationName::new("dev"),
            kind: ConfigurationContextPickerKind::Project,
            items: vec![
                project_item("dev-project", "Development"),
                project_item("prod-project", "Production"),
            ],
        }),
    );
    reduce(
        &mut app,
        &Action::Configurations(ConfigurationsAction::StartContextPickerFilter),
    );
    reduce(
        &mut app,
        &Action::Configurations(ConfigurationsAction::UpdateContextPickerFilter(
            "prod".to_string(),
        )),
    );
    reduce(
        &mut app,
        &Action::Configurations(ConfigurationsAction::ApplyContextPickerFilter),
    );
    reduce(
        &mut app,
        &Action::Configurations(ConfigurationsAction::StartContextPickerFilter),
    );
    reduce(
        &mut app,
        &Action::Configurations(ConfigurationsAction::UpdateContextPickerFilter(
            "dev".to_string(),
        )),
    );
    reduce(
        &mut app,
        &Action::Configurations(ConfigurationsAction::CancelContextPickerFilter),
    );

    let picker = app
        .root
        .configurations
        .context_picker()
        .expect("picker is open");
    assert!(!picker.is_filter_input_active());
    assert_eq!(picker.filter_draft(), "prod");
    assert_eq!(picker.visible_items().len(), 1);
    assert_eq!(
        picker.selected_item(),
        Some(&project_item("prod-project", "Production"))
    );
}

#[test]
fn picker_selection_updates_only_the_context_edit_draft() {
    let mut app = app_with_context_edit();
    reduce(
        &mut app,
        &Action::Configurations(ConfigurationsAction::OpenContextPicker),
    );
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ContextPickerLoaded {
            configuration: GcloudConfigurationName::new("dev"),
            kind: ConfigurationContextPickerKind::Project,
            items: vec![
                project_item("dev-project", "Development"),
                project_item("prod-project", "Production"),
            ],
        }),
    );
    reduce(
        &mut app,
        &Action::Configurations(ConfigurationsAction::SelectNextContextPickerItem),
    );

    let effects = reduce(
        &mut app,
        &Action::Configurations(ConfigurationsAction::ApplyContextPickerSelection),
    );

    assert!(effects.is_empty());
    assert!(app.root.configurations.context_picker().is_none());
    assert_eq!(
        app.root
            .configurations
            .context_edit_form()
            .expect("form is open")
            .draft()
            .project
            .as_deref(),
        Some("prod-project")
    );
    assert_eq!(
        app.root.current_context().expect("active context").project,
        Some("dev-project".to_string())
    );
}

#[test]
fn picker_cancel_preserves_context_edit_draft() {
    let mut app = app_with_context_edit();
    reduce(
        &mut app,
        &Action::Configurations(ConfigurationsAction::OpenContextPicker),
    );
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ContextPickerLoaded {
            configuration: GcloudConfigurationName::new("dev"),
            kind: ConfigurationContextPickerKind::Project,
            items: vec![
                project_item("dev-project", "Development"),
                project_item("prod-project", "Production"),
            ],
        }),
    );
    reduce(
        &mut app,
        &Action::Configurations(ConfigurationsAction::SelectNextContextPickerItem),
    );
    reduce(
        &mut app,
        &Action::Configurations(ConfigurationsAction::CancelContextPicker),
    );

    assert!(app.root.configurations.context_picker().is_none());
    assert_eq!(
        app.root
            .configurations
            .context_edit_form()
            .expect("form is open")
            .draft()
            .project
            .as_deref(),
        Some("dev-project")
    );
}

#[test]
fn retrying_picker_preserves_filter_and_emits_load_effect() {
    let mut app = app_with_context_edit();
    reduce(
        &mut app,
        &Action::Configurations(ConfigurationsAction::OpenContextPicker),
    );
    reduce(
        &mut app,
        &Action::Configurations(ConfigurationsAction::StartContextPickerFilter),
    );
    reduce(
        &mut app,
        &Action::Configurations(ConfigurationsAction::UpdateContextPickerFilter(
            "prod".to_string(),
        )),
    );
    let context = app.root.current_context().expect("active context");

    let effects = reduce(
        &mut app,
        &Action::Configurations(ConfigurationsAction::RetryContextPicker),
    );

    assert_eq!(
        effects,
        vec![Effect::LoadContextPicker {
            kind: ConfigurationContextPickerKind::Project,
            context,
        }]
    );
    let picker = app
        .root
        .configurations
        .context_picker()
        .expect("picker is open");
    assert_eq!(picker.filter_draft(), "prod");
    assert_eq!(picker.status(), &ConfigurationContextPickerStatus::Loading);
}

#[test]
fn picker_empty_and_degraded_states_do_not_emit_selection_effect() {
    let mut app = app_with_context_edit();
    reduce(
        &mut app,
        &Action::Configurations(ConfigurationsAction::OpenContextPicker),
    );
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ContextPickerLoaded {
            configuration: GcloudConfigurationName::new("dev"),
            kind: ConfigurationContextPickerKind::Project,
            items: Vec::new(),
        }),
    );
    let effects = reduce(
        &mut app,
        &Action::Configurations(ConfigurationsAction::ApplyContextPickerSelection),
    );
    assert!(effects.is_empty());
    assert_eq!(
        app.root
            .configurations
            .context_picker()
            .expect("empty picker remains open")
            .status(),
        &ConfigurationContextPickerStatus::Empty
    );

    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ContextPickerLoadFailed {
            configuration: GcloudConfigurationName::new("dev"),
            kind: ConfigurationContextPickerKind::Project,
            error: AppError::gcloud("permission denied"),
        }),
    );
    let effects = reduce(
        &mut app,
        &Action::Configurations(ConfigurationsAction::ApplyContextPickerSelection),
    );
    assert!(effects.is_empty());
    assert!(matches!(
        app.root
            .configurations
            .context_picker()
            .expect("degraded picker remains open")
            .status(),
        ConfigurationContextPickerStatus::Degraded(message) if message.contains("permission denied")
    ));
}

#[test]
fn picker_loaded_result_after_focus_change_is_ignored() {
    let mut loaded_app = app_with_context_edit();
    reduce(
        &mut loaded_app,
        &Action::Configurations(ConfigurationsAction::OpenContextPicker),
    );
    reduce(
        &mut loaded_app,
        &Action::Configurations(ConfigurationsAction::FocusNextContextEditField),
    );
    reduce(
        &mut loaded_app,
        &Action::EffectResult(EffectResult::ContextPickerLoaded {
            configuration: GcloudConfigurationName::new("dev"),
            kind: ConfigurationContextPickerKind::Project,
            items: vec![project_item("new-project", "New Project")],
        }),
    );
    assert_eq!(
        loaded_app
            .root
            .configurations
            .context_picker()
            .expect("project picker remains open")
            .status(),
        &ConfigurationContextPickerStatus::Loading
    );
}

#[test]
fn picker_failed_result_after_focus_change_is_ignored() {
    let mut failed_app = app_with_context_edit();
    reduce(
        &mut failed_app,
        &Action::Configurations(ConfigurationsAction::OpenContextPicker),
    );
    reduce(
        &mut failed_app,
        &Action::Configurations(ConfigurationsAction::FocusNextContextEditField),
    );
    reduce(
        &mut failed_app,
        &Action::EffectResult(EffectResult::ContextPickerLoadFailed {
            configuration: GcloudConfigurationName::new("dev"),
            kind: ConfigurationContextPickerKind::Project,
            error: AppError::gcloud("permission denied"),
        }),
    );
    assert_eq!(
        failed_app
            .root
            .configurations
            .context_picker()
            .expect("project picker remains open")
            .status(),
        &ConfigurationContextPickerStatus::Loading
    );
}

#[test]
fn picker_loaded_result_for_mismatched_picker_kind_is_ignored() {
    let mut app = app_with_context_edit();
    reduce(
        &mut app,
        &Action::Configurations(ConfigurationsAction::OpenContextPicker),
    );

    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ContextPickerLoaded {
            configuration: GcloudConfigurationName::new("dev"),
            kind: ConfigurationContextPickerKind::Region,
            items: vec![ConfigurationContextPickerItem::Region(ComputeRegion {
                name: "europe-west1".to_string(),
                status: Some("UP".to_string()),
            })],
        }),
    );

    assert_eq!(
        app.root
            .configurations
            .context_picker()
            .expect("project picker remains open")
            .status(),
        &ConfigurationContextPickerStatus::Loading
    );
}

#[test]
fn picker_failed_result_for_mismatched_picker_kind_is_ignored() {
    let mut app = app_with_context_edit();
    reduce(
        &mut app,
        &Action::Configurations(ConfigurationsAction::OpenContextPicker),
    );

    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ContextPickerLoadFailed {
            configuration: GcloudConfigurationName::new("dev"),
            kind: ConfigurationContextPickerKind::Region,
            error: AppError::gcloud("permission denied"),
        }),
    );

    assert_eq!(
        app.root
            .configurations
            .context_picker()
            .expect("project picker remains open")
            .status(),
        &ConfigurationContextPickerStatus::Loading
    );
}

#[test]
fn picker_result_for_closed_form_is_ignored() {
    let mut app = app_with_context_edit();
    reduce(
        &mut app,
        &Action::Configurations(ConfigurationsAction::OpenContextPicker),
    );
    reduce(
        &mut app,
        &Action::Configurations(ConfigurationsAction::CancelContextEdit),
    );

    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ContextPickerLoaded {
            configuration: GcloudConfigurationName::new("dev"),
            kind: ConfigurationContextPickerKind::Project,
            items: vec![project_item("new-project", "New Project")],
        }),
    );
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ContextPickerLoadFailed {
            configuration: GcloudConfigurationName::new("dev"),
            kind: ConfigurationContextPickerKind::Project,
            error: AppError::gcloud("permission denied"),
        }),
    );

    assert!(app.root.configurations.context_edit_form().is_none());
    assert!(app.root.configurations.context_picker().is_none());
    assert_eq!(app.root.system.status_message, None);
}

#[test]
fn configuration_filter_matches_context_fields_case_insensitively() {
    let mut app = AppState::new();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ConfigurationsLoaded {
            configurations: vec![
                config_with_context(
                    "dev",
                    true,
                    Some("dev-project"),
                    Some("europe-west1"),
                    Some("europe-west1-b"),
                ),
                GcloudConfiguration {
                    name: GcloudConfigurationName::new("ops"),
                    is_active: false,
                    account: Some("ops@example.com".to_string()),
                    project: None,
                    region: Some("asia-south1".to_string()),
                    zone: None,
                },
            ],
        }),
    );

    for (filter, expected) in [
        ("DEV@EXAMPLE", "dev"),
        ("dev-project", "dev"),
        ("WEST1-B", "dev"),
        ("asia-south", "ops"),
        ("ops@example", "ops"),
    ] {
        reduce(
            &mut app,
            &Action::Configurations(ConfigurationsAction::StartFiltering),
        );
        reduce(
            &mut app,
            &Action::Configurations(ConfigurationsAction::UpdateFilter(filter.to_string())),
        );
        reduce(
            &mut app,
            &Action::Configurations(ConfigurationsAction::ApplyFilter),
        );

        assert_eq!(
            app.root
                .configurations
                .visible_rows()
                .into_iter()
                .map(|configuration| configuration.name.as_str())
                .collect::<Vec<_>>(),
            vec![expected]
        );
    }
}

#[test]
fn configuration_selection_clamps_to_visible_rows_and_activation_uses_visible_selection() {
    let mut app = AppState::new();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ConfigurationsLoaded {
            configurations: vec![
                config("dev", true, "dev-project"),
                config("stage", false, "stage-project"),
                config("prod", false, "prod-project"),
            ],
        }),
    );
    reduce(
        &mut app,
        &Action::Configurations(ConfigurationsAction::SelectNext),
    );
    reduce(
        &mut app,
        &Action::Configurations(ConfigurationsAction::SelectNext),
    );
    assert_eq!(
        app.root.configurations.selected().unwrap().name.as_str(),
        "prod"
    );

    reduce(
        &mut app,
        &Action::Configurations(ConfigurationsAction::StartFiltering),
    );
    reduce(
        &mut app,
        &Action::Configurations(ConfigurationsAction::UpdateFilter("dev".to_string())),
    );
    reduce(
        &mut app,
        &Action::Configurations(ConfigurationsAction::ApplyFilter),
    );
    assert_eq!(app.root.configurations.selected_visible_index(), Some(0));
    assert_eq!(
        app.root.configurations.selected().unwrap().name.as_str(),
        "dev"
    );

    reduce(
        &mut app,
        &Action::Configurations(ConfigurationsAction::ClearFilter),
    );
    reduce(
        &mut app,
        &Action::Configurations(ConfigurationsAction::SelectNext),
    );
    let request_effects = reduce(
        &mut app,
        &Action::Configurations(ConfigurationsAction::RequestActivateSelected),
    );
    assert!(request_effects.is_empty());
    let effects = reduce(
        &mut app,
        &Action::Configurations(ConfigurationsAction::ConfirmActivation),
    );

    assert!(matches!(
        effects.as_slice(),
        [Effect::ActivateConfiguration { name }] if name.as_str() == "stage"
    ));
}

#[test]
fn configuration_activation_clamps_selection_after_visible_context_changes() {
    let mut app = AppState::new();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ConfigurationsLoaded {
            configurations: vec![
                config("dev", true, "target-project"),
                config("prod", false, "target-project"),
            ],
        }),
    );
    reduce(
        &mut app,
        &Action::Configurations(ConfigurationsAction::StartFiltering),
    );
    reduce(
        &mut app,
        &Action::Configurations(ConfigurationsAction::UpdateFilter(
            "target-project".to_string(),
        )),
    );
    reduce(
        &mut app,
        &Action::Configurations(ConfigurationsAction::ApplyFilter),
    );
    reduce(
        &mut app,
        &Action::Configurations(ConfigurationsAction::SelectNext),
    );
    assert_eq!(
        app.root.configurations.selected().unwrap().name.as_str(),
        "prod"
    );

    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ConfigurationActivationReloadFailed {
            configuration: config("prod", true, "other-project"),
            error: AppError::gcloud("reload failed"),
        }),
    );

    assert_eq!(app.root.configurations.selected_visible_index(), Some(0));
    assert_eq!(
        app.root.configurations.selected().unwrap().name.as_str(),
        "dev"
    );
    let request_effects = reduce(
        &mut app,
        &Action::Configurations(ConfigurationsAction::RequestActivateSelected),
    );
    assert!(request_effects.is_empty());
    let effects = reduce(
        &mut app,
        &Action::Configurations(ConfigurationsAction::ConfirmActivation),
    );
    assert!(matches!(
        effects.as_slice(),
        [Effect::ActivateConfiguration { name }] if name.as_str() == "dev"
    ));
}
