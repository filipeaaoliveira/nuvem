use crate::app::service_registry::service_id_for_resource;
use crate::app::state::command::{CommandDescriptor, cloud_sql_connection_name};
use crate::app::state::resources::{
    CloudSql, CloudSqlInstanceAction, Compute, ComputeInstanceAction, ComputeInstanceField,
};
use crate::app::state::root::{RootState, Workspace};
use crate::app::{CommandAction, ConfigurationsAction, Effect, ResourcesAction};
use nuvem_domain::gcp::logging::LogQuery;

pub fn reduce(state: &mut RootState, action: &CommandAction) -> Vec<Effect> {
    match action {
        CommandAction::Open => {
            if state.resources.action_confirmation().is_none()
                && state.resources.batch_confirmation().is_none()
                && state.configurations.activation_confirmation().is_none()
            {
                state.command.open();
            }
        }
        CommandAction::AppendChar(value) => {
            if state.command.is_open() {
                state.command.append_char(*value);
            }
        }
        CommandAction::Backspace => {
            if state.command.is_open() {
                state.command.backspace();
            }
        }
        CommandAction::SelectNext => {
            if state.command.is_open() {
                let snapshot = state.clone();
                state.command.select_next(&snapshot);
            }
        }
        CommandAction::SelectPrevious => {
            if state.command.is_open() {
                state.command.select_previous();
            }
        }
        CommandAction::Submit => {
            if !state.command.is_open() {
                return Vec::new();
            }
            if let Some(command) = state.command.selected_visible_command(state) {
                // The reducer is the single enforcement point for read-only:
                // blocked commands stay in the palette (greyed, not hidden), so
                // this arm is what actually refuses them. The axis is
                // `is_mutating`, not `is_destructive` — `Start` is not
                // destructive but does change cloud state.
                // `is_mutating` answers the compile-time question about the
                // descriptor enum and keeps answering `true` for a plugin,
                // whose argv is runtime data. A plugin's own answer comes from
                // its declaration (`declared_effect`) — the two are not in
                // competition: one classifies variants, the other classifies
                // config. `command_blocked_by_read_only` owns that split, so
                // this arm and the palette's dimming cannot come to disagree
                // about which rows the mode refuses.
                if crate::app::state::command::command_blocked_by_read_only(command, state) {
                    // Name the command the way the palette row the operator
                    // just submitted named it: with rows marked, the row read
                    // "Stop 2 marked GCE instances", and a refusal that says
                    // "Stop selected GCE instance" misstates the blast radius
                    // the same way the static label would on the row itself.
                    let label = crate::app::state::command::palette_display_label(command, state);
                    state.command.close();
                    // Two sentences, because the mode refuses these two for
                    // different reasons: a plugin is refused for being
                    // unproven, which is not a claim that it mutates.
                    match command {
                        CommandDescriptor::Plugin(_) => {
                            super::resources::refuse_plugin_under_read_only(state, &label);
                        }
                        _ => super::resources::refuse_under_read_only(state, &label),
                    }
                    return Vec::new();
                }
                state.command.close();
                match command {
                    CommandDescriptor::ActivateSelectedConfiguration => {
                        return super::configurations::reduce(
                            state,
                            &ConfigurationsAction::RequestActivateSelected,
                        );
                    }
                    CommandDescriptor::StartSelectedComputeInstance
                    | CommandDescriptor::StopSelectedComputeInstance
                    | CommandDescriptor::ResetSelectedComputeInstance
                    | CommandDescriptor::SuspendSelectedComputeInstance
                    | CommandDescriptor::ResumeSelectedComputeInstance => {
                        if let Some(action) = mutation_action(command) {
                            if state.resources.marked_count() > 0 {
                                if let Some(context) = state.current_context() {
                                    state
                                        .resources
                                        .start_batch_confirmation::<Compute>(context, action);
                                }
                            } else {
                                let context = state.current_context();
                                let instance = state.resources.selected_visible_instance().cloned();
                                if let (Some(context), Some(instance)) = (context, instance)
                                    && action.is_eligible(&instance.status)
                                {
                                    state
                                        .resources
                                        .start_action_confirmation(context, instance, action);
                                }
                            }
                        }
                    }
                    CommandDescriptor::StartSelectedCloudSqlInstance
                    | CommandDescriptor::StopSelectedCloudSqlInstance
                    | CommandDescriptor::RestartSelectedCloudSqlInstance
                    | CommandDescriptor::FailoverSelectedCloudSqlInstance => {
                        if let Some(action) = cloud_sql_mutation_action(command) {
                            if state.resources.marked_count() > 0 {
                                if let Some(context) = state.current_context() {
                                    state
                                        .resources
                                        .start_batch_confirmation::<CloudSql>(context, action);
                                }
                            } else {
                                let context = state.current_context();
                                let instance =
                                    state.resources.selected_cloud_sql_instance().cloned();
                                if let (Some(context), Some(instance)) = (context, instance)
                                    && action.is_eligible(&instance.status, instance.availability)
                                {
                                    state.resources.start_cloud_sql_action_confirmation(
                                        context, instance, action,
                                    );
                                }
                            }
                        }
                    }
                    CommandDescriptor::CopySelectedCloudSqlConnectionName => {
                        let context = state.current_context();
                        let instance = state.resources.selected_cloud_sql_instance();
                        if let (Some(context), Some(instance)) = (context, instance)
                            && let Some(value) = cloud_sql_connection_name(&context, instance)
                        {
                            return vec![Effect::CopyToClipboard {
                                value,
                                description: "Cloud SQL connection name".to_string(),
                            }];
                        }
                    }
                    CommandDescriptor::ShowSelectedComputeInstanceSsh => {
                        if let (Some(context), Some(instance)) = (
                            state.current_context(),
                            state.resources.selected_visible_instance().cloned(),
                        ) {
                            state.resources.show_ssh_command_preview(context, instance);
                        }
                    }
                    CommandDescriptor::SshSelectedComputeInstance => {
                        if let (Some(context), Some(instance)) = (
                            state.current_context(),
                            state.resources.selected_visible_instance().cloned(),
                        ) {
                            // Refused before the preview opens, not at confirm:
                            // the preview's whole job is to show the argv that
                            // will run, and there is no honest one to show for
                            // a name gcloud would read as an option.
                            if let Some(offender) = nuvem_domain::gcp::argv::option_like_value([
                                instance.name.as_str(),
                                instance.zone.as_str(),
                            ]) {
                                let message = super::resources::refuse_option_like(offender);
                                state.system.set_status_message(message);
                                return Vec::new();
                            }
                            state.resources.show_ssh_launch_preview(context, instance);
                        }
                    }
                    // Both share the reducer's plan resolution, which reads
                    // the active service's own selection — so one arm is
                    // enough, and it can never build a plan from the wrong
                    // service's row.
                    CommandDescriptor::StartTunnelForSelectedComputeInstance
                    | CommandDescriptor::StartTunnelForSelectedCloudSqlInstance => {
                        return super::resources::reduce(state, &ResourcesAction::RequestTunnel);
                    }
                    CommandDescriptor::CopySelectedInstanceName
                    | CommandDescriptor::CopySelectedInstanceZone
                    | CommandDescriptor::CopySelectedInstanceInternalIp
                    | CommandDescriptor::CopySelectedInstanceExternalIp
                    | CommandDescriptor::CopySelectedInstanceSelfLink => {
                        let context = state.current_context();
                        let instance = state.resources.selected_visible_instance();
                        if let (Some(context), Some(instance)) = (context, instance)
                            && let Some(field) = copy_field(command)
                            && let Some(value) = field.value(&context, instance)
                        {
                            return vec![Effect::CopyToClipboard {
                                value,
                                description: field.description().to_string(),
                            }];
                        }
                    }
                    CommandDescriptor::OpenProjectLogs => {
                        state.workspace = Workspace::Logging;
                        state.logging.open_query(LogQuery::ProjectLogs);
                        return super::logging::load_recent_or_settle(
                            state,
                            super::logging::UnloadedBuffer::Discard,
                        );
                    }
                    // Every resource-scoped log jump resolves through the one
                    // reducer-side resolver, which reads the view actually on
                    // screen. Duplicating the query construction per command
                    // is how the palette and the `L` key would drift into
                    // disagreeing about what "the selection" is.
                    CommandDescriptor::OpenLogsForSelected(_)
                    | CommandDescriptor::OpenLogsForSelectedCloudRunRevision => {
                        return super::resources::reduce(
                            state,
                            &ResourcesAction::OpenLogsForSelection,
                        );
                    }
                    CommandDescriptor::GoHome => {
                        state.workspace = Workspace::Home;
                    }
                    CommandDescriptor::GoToConfigurations => {
                        state.workspace = Workspace::Configurations;
                    }
                    CommandDescriptor::GoToTunnels => {
                        state.workspace = Workspace::Tunnels;
                    }
                    CommandDescriptor::GoToFinOps => {
                        state.workspace = Workspace::FinOps;
                        return super::finops::reduce(
                            state,
                            &crate::app::action::FinOpsAction::Refresh,
                        );
                    }
                    CommandDescriptor::GoToService(kind) => {
                        state.workspace = Workspace::Resources;
                        state.resources.select_service(kind);
                        if let Some(id) = service_id_for_resource(kind) {
                            state.navigator.select(id);
                        }
                        state.resources.close_detail();
                    }
                    CommandDescriptor::RefreshCurrentView => match state.workspace {
                        Workspace::Configurations => return vec![Effect::LoadConfigurations],
                        Workspace::Resources => {
                            return super::resources::reduce(state, &ResourcesAction::Refresh);
                        }
                        Workspace::Logging => {
                            return super::logging::reduce(
                                state,
                                &crate::app::LoggingAction::Refresh,
                            );
                        }
                        Workspace::Home => {}
                        // Tunnel rows are pushed by the supervisor, so there
                        // is nothing to re-fetch. `navigation_commands` does
                        // not offer Refresh here; this arm exists so the
                        // command can never silently refresh a different
                        // workspace's data.
                        Workspace::Tunnels => {}
                        Workspace::FinOps => {
                            return super::finops::reduce(
                                state,
                                &crate::app::action::FinOpsAction::Refresh,
                            );
                        }
                    },
                    CommandDescriptor::OpenSelectedGkeClusterInK9s => {
                        if let (Some(context), Some(cluster)) = (
                            state.current_context(),
                            state.resources.selected_gke_cluster().cloned(),
                        ) && let Some(target) =
                            nuvem_domain::gcp::k9s::resolve_launch_target(&context, &cluster)
                        {
                            return vec![Effect::PrepareK9sLaunch { target }];
                        }
                    }
                    CommandDescriptor::ShowPluginOutput => {
                        // Unconditional: this arm is only reached while the
                        // entry is offered, and it is offered only while some
                        // record still holds output.
                        state.system.open_plugin_output();
                    }
                    CommandDescriptor::ReloadConfig => {
                        return vec![Effect::ReloadConfig];
                    }
                    CommandDescriptor::ShowHelp => {
                        state.system.show_help = true;
                    }
                    // Submitting a plugin opens its confirmation — or, when
                    // the plugin declares values, the prompt that collects
                    // them — and stops there: the palette decides *what* is
                    // about to run, the modal's own Enter decides *whether* it
                    // does, which is why this arm emits no effect. A
                    // `read_only` session reaches it only for a plugin
                    // `plugin_blocked_by_read_only` admits, and a second guard
                    // here would be a second thing to keep in step with the
                    // refusal above.
                    //
                    // Both are opened through the shared entry
                    // (`super::resources::open_plugin`) so this door and the
                    // keyed press cannot come to disagree about
                    // read-only, or about which of the two modals a plugin
                    // opens.
                    CommandDescriptor::Plugin(id) => {
                        // Composed here rather than carried from offer time:
                        // the palette may have been open while the list
                        // refreshed, so the argv the operator is about to read
                        // is built from the row that is selected *now*.
                        if let Some(offer) = crate::app::state::command::plugin_offer(state, id) {
                            super::resources::open_plugin(state, offer);
                        }
                    }
                    CommandDescriptor::SwitchProjectContext => {
                        state.workspace = Workspace::Configurations;
                        return super::configurations::reduce(
                            state,
                            &ConfigurationsAction::StartEditActiveContext,
                        );
                    }
                }
            }
        }
        CommandAction::Cancel => state.command.close(),
    }

    Vec::new()
}

fn mutation_action(command: CommandDescriptor) -> Option<ComputeInstanceAction> {
    match command {
        CommandDescriptor::StartSelectedComputeInstance => Some(ComputeInstanceAction::Start),
        CommandDescriptor::StopSelectedComputeInstance => Some(ComputeInstanceAction::Stop),
        CommandDescriptor::ResetSelectedComputeInstance => Some(ComputeInstanceAction::Reset),
        CommandDescriptor::SuspendSelectedComputeInstance => Some(ComputeInstanceAction::Suspend),
        CommandDescriptor::ResumeSelectedComputeInstance => Some(ComputeInstanceAction::Resume),
        _ => None,
    }
}

fn cloud_sql_mutation_action(command: CommandDescriptor) -> Option<CloudSqlInstanceAction> {
    match command {
        CommandDescriptor::StartSelectedCloudSqlInstance => Some(CloudSqlInstanceAction::Start),
        CommandDescriptor::StopSelectedCloudSqlInstance => Some(CloudSqlInstanceAction::Stop),
        CommandDescriptor::RestartSelectedCloudSqlInstance => Some(CloudSqlInstanceAction::Restart),
        CommandDescriptor::FailoverSelectedCloudSqlInstance => {
            Some(CloudSqlInstanceAction::Failover)
        }
        _ => None,
    }
}

fn copy_field(command: CommandDescriptor) -> Option<ComputeInstanceField> {
    match command {
        CommandDescriptor::CopySelectedInstanceName => Some(ComputeInstanceField::Name),
        CommandDescriptor::CopySelectedInstanceZone => Some(ComputeInstanceField::Zone),
        CommandDescriptor::CopySelectedInstanceInternalIp => Some(ComputeInstanceField::InternalIp),
        CommandDescriptor::CopySelectedInstanceExternalIp => Some(ComputeInstanceField::ExternalIp),
        CommandDescriptor::CopySelectedInstanceSelfLink => Some(ComputeInstanceField::SelfLink),
        _ => None,
    }
}
