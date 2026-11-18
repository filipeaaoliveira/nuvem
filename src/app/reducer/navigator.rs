use crate::app::service_registry::{descriptor, load_effect};
use crate::app::state::root::{RootState, Workspace};
use crate::app::{Effect, NavigatorAction};

pub fn reduce(state: &mut RootState, action: &NavigatorAction) -> Vec<Effect> {
    match action {
        NavigatorAction::SelectNext => state.navigator.select_next(),
        NavigatorAction::SelectPrevious => state.navigator.select_previous(),
        NavigatorAction::StartFilter => state.navigator.start_filter(),
        NavigatorAction::AppendFilterChar(value) => state.navigator.append_filter_char(*value),
        NavigatorAction::BackspaceFilter => state.navigator.backspace_filter(),
        NavigatorAction::ApplyFilter => state.navigator.apply_filter(),
        NavigatorAction::CancelFilter => state.navigator.cancel_filter(),
        NavigatorAction::OpenSelected => return open_selected(state),
        NavigatorAction::ToggleSidebar => state.navigator.toggle_sidebar(),
        NavigatorAction::ToggleFocus => state.navigator.toggle_focus(),
    }

    Vec::new()
}

fn open_selected(state: &mut RootState) -> Vec<Effect> {
    let id = state.navigator.selected_id();
    if !state.navigator.visible_ids().contains(&id) {
        return Vec::new();
    }

    let selected = *descriptor(id);

    if !selected.is_available() {
        if let Some(reason) = selected.unavailable_reason() {
            state.system.set_operation_status_message(
                "service navigator",
                format!("{} is unavailable: {reason}", selected.label),
            );
        }
        return Vec::new();
    }

    let Some(target) = selected.target else {
        return Vec::new();
    };
    let Some(service) = target.resource_kind() else {
        return Vec::new();
    };

    state.resources.select_service(service);
    state.workspace = Workspace::Resources;

    // The service being opened is the one whose filter gets pushed.
    let filter = super::resources::active_filter_for(state, service);
    state
        .current_context()
        .and_then(|context| load_effect(id, context, filter))
        .into_iter()
        .collect()
}
