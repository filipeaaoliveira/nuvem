use crate::app::state::configurations::ActivationConfirmationRequest;
use crate::app::state::root::RootState;
use crate::app::{ConfigurationsAction, Effect};

const REQUIRED_NAME_STATUS: &str = "Configuration name is required";
const NO_ACTIVE_CONFIGURATION_STATUS: &str = "No active gcloud configuration";
const ALREADY_ACTIVE_CONFIGURATION_STATUS: &str = "Configuration is already active";

pub fn reduce(state: &mut RootState, action: &ConfigurationsAction) -> Vec<Effect> {
    match action {
        ConfigurationsAction::SelectNext => state.configurations.select_next(),
        ConfigurationsAction::SelectPrevious => state.configurations.select_previous(),
        ConfigurationsAction::RequestActivateSelected => {
            match state.configurations.start_activation_confirmation() {
                ActivationConfirmationRequest::Started => {
                    clear_status_if(state, &[ALREADY_ACTIVE_CONFIGURATION_STATUS]);
                }
                ActivationConfirmationRequest::AlreadyActive => {
                    state
                        .system
                        .set_status_message(ALREADY_ACTIVE_CONFIGURATION_STATUS);
                }
                ActivationConfirmationRequest::NoSelection => {}
            }
        }
        ConfigurationsAction::ConfirmActivation => {
            if let Some(name) = state.configurations.confirm_activation() {
                return vec![Effect::ActivateConfiguration { name }];
            }
        }
        ConfigurationsAction::CancelActivation => state.configurations.cancel_activation(),
        ConfigurationsAction::StartCreate => {
            state.configurations.start_create();
            clear_status_if(
                state,
                &[NO_ACTIVE_CONFIGURATION_STATUS, REQUIRED_NAME_STATUS],
            );
        }
        ConfigurationsAction::AppendCreateChar(value) => {
            state.configurations.append_create_char(*value);
        }
        ConfigurationsAction::BackspaceCreateField => {
            state.configurations.backspace_create_field();
        }
        ConfigurationsAction::FocusNextCreateField => {
            state.configurations.focus_next_create_field();
        }
        ConfigurationsAction::FocusPreviousCreateField => {
            state.configurations.focus_previous_create_field();
        }
        ConfigurationsAction::SubmitCreate => {
            if let Some(draft) = state.configurations.create_submission() {
                state.system.clear_status_message();
                return vec![Effect::CreateConfiguration { draft }];
            }
            if state.configurations.is_create_form_open() {
                state.system.set_error_status_message(REQUIRED_NAME_STATUS);
            }
        }
        ConfigurationsAction::CancelCreate => {
            state.configurations.cancel_create();
            clear_status_if(state, &[REQUIRED_NAME_STATUS]);
        }
        ConfigurationsAction::StartEditActiveContext => {
            if state.configurations.start_context_edit() {
                clear_status_if(
                    state,
                    &[NO_ACTIVE_CONFIGURATION_STATUS, REQUIRED_NAME_STATUS],
                );
            } else {
                state
                    .system
                    .set_status_message(NO_ACTIVE_CONFIGURATION_STATUS);
            }
        }
        ConfigurationsAction::AppendContextEditChar(value) => {
            state.configurations.append_context_edit_char(*value);
        }
        ConfigurationsAction::BackspaceContextEditField => {
            state.configurations.backspace_context_edit_field();
        }
        ConfigurationsAction::FocusNextContextEditField => {
            state.configurations.focus_next_context_edit_field();
        }
        ConfigurationsAction::FocusPreviousContextEditField => {
            state.configurations.focus_previous_context_edit_field();
        }
        ConfigurationsAction::SubmitContextEdit => {
            if let Some(draft) = state.configurations.context_edit_submission() {
                clear_status_if(state, &[NO_ACTIVE_CONFIGURATION_STATUS]);
                return vec![Effect::UpdateConfigurationContext { draft }];
            }
        }
        ConfigurationsAction::CancelContextEdit => {
            state.configurations.cancel_context_edit();
            clear_status_if(state, &[NO_ACTIVE_CONFIGURATION_STATUS]);
        }
        ConfigurationsAction::OpenContextPicker => {
            if let Some(context) = state.current_context()
                && let Some(kind) = state.configurations.open_context_picker()
            {
                return vec![Effect::LoadContextPicker { kind, context }];
            }
        }
        ConfigurationsAction::StartContextPickerFilter => {
            state.configurations.start_context_picker_filter();
        }
        ConfigurationsAction::UpdateContextPickerFilter(value) => {
            state
                .configurations
                .update_context_picker_filter(value.clone());
        }
        ConfigurationsAction::ApplyContextPickerFilter => {
            state.configurations.apply_context_picker_filter();
        }
        ConfigurationsAction::CancelContextPickerFilter => {
            state.configurations.cancel_context_picker_filter();
        }
        ConfigurationsAction::SelectNextContextPickerItem => {
            state.configurations.select_next_context_picker_item();
        }
        ConfigurationsAction::SelectPreviousContextPickerItem => {
            state.configurations.select_previous_context_picker_item();
        }
        ConfigurationsAction::ApplyContextPickerSelection => {
            state.configurations.apply_context_picker_selection();
        }
        ConfigurationsAction::RetryContextPicker => {
            if let Some(context) = state.current_context()
                && let Some(kind) = state.configurations.retry_context_picker()
            {
                return vec![Effect::LoadContextPicker { kind, context }];
            }
        }
        ConfigurationsAction::CancelContextPicker => {
            state.configurations.cancel_context_picker();
        }
        ConfigurationsAction::StartFiltering => {
            state.configurations.start_filtering();
        }
        ConfigurationsAction::UpdateFilter(value) => {
            state.configurations.update_filter(value.clone());
        }
        ConfigurationsAction::ApplyFilter => {
            state.configurations.apply_filter();
        }
        ConfigurationsAction::CancelFilter => {
            state.configurations.cancel_filter();
        }
        ConfigurationsAction::ClearFilter => {
            state.configurations.clear_filter();
        }
    }
    Vec::new()
}

fn clear_status_if(state: &mut RootState, statuses: &[&str]) {
    if state
        .system
        .status_message
        .as_deref()
        .is_some_and(|message| statuses.contains(&message))
    {
        state.system.clear_status_message();
    }
}
