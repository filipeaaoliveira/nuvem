use crate::support::remembered_view::remembered;
use nuvem::app::reducer::root::reduce;
use nuvem::app::service_registry::{ServiceId, descriptor, service_id_for_resource};
use nuvem::app::state::command::{CommandDescriptor, palette_display_label};
use nuvem::app::state::plugins::PluginId;
use nuvem::app::state::resources::{
    BatchAction, CloudSqlInstanceAction, ComputeInstanceAction, ComputeInstanceField,
};
use nuvem::app::state::root::Workspace;
use nuvem::app::state::system::OperationPhase;
use nuvem::app::{
    Action, AppState, CommandAction, Effect, EffectResult, ResourcesAction, WorkspaceAction,
};
use nuvem::config::aliases::AliasTable;
use nuvem::config::plugins::Plugins;
use nuvem::config::problems::ConfigProblems;
use nuvem_domain::gcp::configuration::{
    GcloudConfiguration, GcloudConfigurationName, GcloudContext,
};
use nuvem_domain::gcp::logging::{LogQuery, LogResourceKind};
use nuvem_domain::gcp::resource::{
    BigQueryDataset, CloudFunction, CloudFunctionEnvironment, CloudFunctionState, CloudRunService,
    CloudRunServiceStatus, CloudSqlAvailability, CloudSqlInstance, CloudSqlStatus, ComputeInstance,
    DiskStatus, EndpointVisibility, FirewallAction, FirewallDirection, FirewallRule,
    FirewallTargets, GkeCluster, GkeClusterStatus, IamPolicyMember, PersistentDisk, PubSubDelivery,
    PubSubSubscription, PubSubSubscriptionState, PubSubTopic, ResourceServiceKind, ResourceStatus,
    Secret, ServiceAccount, StorageBucket, SubscriptionCount, VpcNetwork, VpcRoutingMode,
    VpcSubnet, VpcSubnetMode,
};
use nuvem_domain::gcp::tunnel::TunnelKind;
use std::collections::BTreeMap;
use std::io::Write;
use tempfile::NamedTempFile;

fn config(name: &str, active: bool, project: &str) -> GcloudConfiguration {
    GcloudConfiguration {
        name: GcloudConfigurationName::new(name),
        is_active: active,
        account: Some("dev@example.com".to_string()),
        project: Some(project.to_string()),
        region: Some("europe-west1".to_string()),
        zone: Some("europe-west1-b".to_string()),
    }
}

fn instance(name: &str) -> ComputeInstance {
    ComputeInstance {
        id: format!("{name}-id"),
        name: name.to_string(),
        zone: "europe-west1-b".to_string(),
        status: ResourceStatus::Running,
        machine_type: "e2-medium".to_string(),
        internal_ip: Some("10.0.0.2".to_string()),
        external_ip: None,
    }
}

fn instance_with_status(name: &str, status: ResourceStatus) -> ComputeInstance {
    ComputeInstance {
        status,
        ..instance(name)
    }
}

fn app_with_selected_instance() -> AppState {
    let mut app = AppState::new();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ConfigurationsLoaded {
            configurations: vec![config("dev", true, "dev-project")],
        }),
    );
    let context = app.root.current_context().expect("context exists");
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ComputeInstancesLoaded {
            context,
            instances: vec![instance("api-dev-1"), instance("worker-dev-1")],
        }),
    );
    reduce(&mut app, &Action::Workspace(WorkspaceAction::Resources));
    app
}

fn app_with_selected_instance_status(status: ResourceStatus) -> AppState {
    let mut app = app_with_selected_instance();
    let context = app.root.current_context().expect("context exists");
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ComputeInstancesLoaded {
            context,
            instances: vec![instance_with_status("api-dev-1", status)],
        }),
    );
    app
}

fn selected_instance(app: &AppState) -> ComputeInstance {
    app.root
        .resources
        .selected_visible_instance()
        .expect("selected instance exists")
        .clone()
}

fn select_command_from_open_palette(
    app: &mut AppState,
    descriptor: CommandDescriptor,
) -> Vec<Effect> {
    reduce(app, &Action::Command(CommandAction::Open));
    for ch in descriptor.label().chars() {
        reduce(app, &Action::Command(CommandAction::AppendChar(ch)));
    }
    assert_eq!(
        app.root.command.selected_visible_command(&app.root),
        Some(descriptor)
    );
    let effects = reduce(app, &Action::Command(CommandAction::Submit));
    assert!(!app.root.command.is_open());
    effects
}

fn config_without_project() -> GcloudConfiguration {
    GcloudConfiguration {
        name: GcloudConfigurationName::new("dev"),
        is_active: true,
        account: Some("dev@example.com".to_string()),
        project: None,
        region: Some("europe-west1".to_string()),
        zone: Some("europe-west1-b".to_string()),
    }
}

fn app_with_selected_inactive_configuration() -> AppState {
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
        &Action::Workspace(WorkspaceAction::Configurations),
    );
    reduce(
        &mut app,
        &Action::Configurations(nuvem::app::ConfigurationsAction::SelectNext),
    );
    app
}

#[test]
fn configuration_activation_command_requires_configurations_workspace_and_inactive_selection() {
    let mut app = app_with_selected_inactive_configuration();
    reduce(&mut app, &Action::Workspace(WorkspaceAction::Home));
    reduce(&mut app, &Action::Command(CommandAction::Open));

    let home_commands = app.root.command.visible_commands(&app.root);
    assert!(!home_commands.contains(&CommandDescriptor::ActivateSelectedConfiguration));
    assert!(home_commands.contains(&CommandDescriptor::GoHome));

    reduce(
        &mut app,
        &Action::Workspace(WorkspaceAction::Configurations),
    );
    reduce(
        &mut app,
        &Action::Configurations(nuvem::app::ConfigurationsAction::SelectPrevious),
    );

    let active_selected_commands = app.root.command.visible_commands(&app.root);
    assert!(!active_selected_commands.contains(&CommandDescriptor::ActivateSelectedConfiguration));
    assert!(active_selected_commands.contains(&CommandDescriptor::GoHome));

    reduce(
        &mut app,
        &Action::Configurations(nuvem::app::ConfigurationsAction::SelectNext),
    );

    let inactive_selected_commands = app.root.command.visible_commands(&app.root);
    assert_eq!(
        inactive_selected_commands.first(),
        Some(&CommandDescriptor::ActivateSelectedConfiguration)
    );
    assert!(inactive_selected_commands.contains(&CommandDescriptor::GoHome));
    assert_eq!(
        CommandDescriptor::ActivateSelectedConfiguration.label(),
        "Activate selected gcloud configuration"
    );
}

#[test]
fn submitting_configuration_activation_command_opens_activation_confirmation() {
    let mut app = app_with_selected_inactive_configuration();
    reduce(&mut app, &Action::Command(CommandAction::Open));

    let effects = reduce(&mut app, &Action::Command(CommandAction::Submit));

    assert!(effects.is_empty());
    assert!(!app.root.command.is_open());
    let confirmation = app
        .root
        .configurations
        .activation_confirmation()
        .expect("activation confirmation should open");
    assert_eq!(confirmation.target().name.as_str(), "prod");
}

#[test]
fn command_palette_does_not_open_during_configuration_activation_confirmation() {
    let mut app = app_with_selected_inactive_configuration();
    reduce(
        &mut app,
        &Action::Configurations(nuvem::app::ConfigurationsAction::RequestActivateSelected),
    );

    let effects = reduce(&mut app, &Action::Command(CommandAction::Open));

    assert!(effects.is_empty());
    assert!(app.root.configurations.activation_confirmation().is_some());
    assert!(!app.root.command.is_open());
}

#[test]
fn cancelling_configuration_confirmation_does_not_reveal_hidden_command_palette() {
    let mut app = app_with_selected_inactive_configuration();
    reduce(
        &mut app,
        &Action::Configurations(nuvem::app::ConfigurationsAction::RequestActivateSelected),
    );
    reduce(&mut app, &Action::Command(CommandAction::Open));

    reduce(
        &mut app,
        &Action::Configurations(nuvem::app::ConfigurationsAction::CancelActivation),
    );

    assert!(app.root.configurations.activation_confirmation().is_none());
    assert!(!app.root.command.is_open());
}

#[test]
fn opening_command_palette_with_selected_instance_exposes_start_and_stop() {
    let mut app = app_with_selected_instance_status(ResourceStatus::Stopped);

    let effects = reduce(&mut app, &Action::Command(CommandAction::Open));

    assert!(effects.is_empty());
    assert!(app.root.command.is_open());
    let commands = app.root.command.visible_commands(&app.root);
    assert!(commands.contains(&CommandDescriptor::StartSelectedComputeInstance));
    assert!(!commands.contains(&CommandDescriptor::StopSelectedComputeInstance));
    assert_eq!(
        app.root.command.selected_visible_command(&app.root),
        Some(CommandDescriptor::StartSelectedComputeInstance)
    );
}

#[test]
fn running_instance_exposes_stop_reset_suspend_and_copy_commands() {
    let mut app = app_with_selected_instance_status(ResourceStatus::Running);
    reduce(&mut app, &Action::Command(CommandAction::Open));

    let commands = app.root.command.visible_commands(&app.root);

    assert!(commands.contains(&CommandDescriptor::StopSelectedComputeInstance));
    assert!(commands.contains(&CommandDescriptor::ResetSelectedComputeInstance));
    assert!(commands.contains(&CommandDescriptor::SuspendSelectedComputeInstance));
    assert!(!commands.contains(&CommandDescriptor::StartSelectedComputeInstance));
    assert!(!commands.contains(&CommandDescriptor::ResumeSelectedComputeInstance));
    assert!(commands.contains(&CommandDescriptor::ShowSelectedComputeInstanceSsh));
    assert!(commands.contains(&CommandDescriptor::CopySelectedInstanceSelfLink));
}

#[test]
fn stopped_and_suspended_instances_expose_only_eligible_mutations() {
    let mut stopped = app_with_selected_instance_status(ResourceStatus::Stopped);
    reduce(&mut stopped, &Action::Command(CommandAction::Open));
    let stopped_commands = stopped.root.command.visible_commands(&stopped.root);
    assert!(stopped_commands.contains(&CommandDescriptor::StartSelectedComputeInstance));
    assert!(!stopped_commands.contains(&CommandDescriptor::StopSelectedComputeInstance));
    assert!(!stopped_commands.contains(&CommandDescriptor::ResetSelectedComputeInstance));

    let mut suspended = app_with_selected_instance_status(ResourceStatus::Suspended);
    reduce(&mut suspended, &Action::Command(CommandAction::Open));
    let suspended_commands = suspended.root.command.visible_commands(&suspended.root);
    assert!(suspended_commands.contains(&CommandDescriptor::ResumeSelectedComputeInstance));
    assert!(!suspended_commands.contains(&CommandDescriptor::SuspendSelectedComputeInstance));
}

#[test]
fn copy_commands_hide_missing_ips_and_self_link_without_project() {
    let mut app = app_with_selected_instance_status(ResourceStatus::Running);
    app.root
        .configurations
        .set_rows(vec![config_without_project()]);
    let context = app.root.current_context().expect("context exists");
    let mut missing_ips = instance_with_status("api-dev-1", ResourceStatus::Running);
    missing_ips.internal_ip = None;
    app.root.resources.set_instances(context, vec![missing_ips]);
    reduce(&mut app, &Action::Command(CommandAction::Open));

    let commands = app.root.command.visible_commands(&app.root);
    assert!(!commands.contains(&CommandDescriptor::CopySelectedInstanceInternalIp));
    assert!(!commands.contains(&CommandDescriptor::CopySelectedInstanceExternalIp));
    assert!(!commands.contains(&CommandDescriptor::CopySelectedInstanceSelfLink));
}

#[test]
fn typed_instance_actions_and_fields_expose_exact_metadata_and_values() {
    let app = app_with_selected_instance_status(ResourceStatus::Running);
    let context = app.root.current_context().expect("context exists");
    let selected = app
        .root
        .resources
        .selected_visible_instance()
        .expect("selected instance exists");

    assert_eq!(ComputeInstanceAction::Reset.command_verb(), "reset");
    assert!(!ComputeInstanceAction::Start.is_eligible(&selected.status));
    assert!(ComputeInstanceAction::Reset.is_eligible(&selected.status));
    assert_eq!(
        ComputeInstanceField::Name.value(&context, selected),
        Some("api-dev-1".to_string())
    );
    assert_eq!(
        ComputeInstanceField::Zone.value(&context, selected),
        Some("europe-west1-b".to_string())
    );
    assert_eq!(
        ComputeInstanceField::SelfLink.value(&context, selected),
        Some("https://www.googleapis.com/compute/v1/projects/dev-project/zones/europe-west1-b/instances/api-dev-1".to_string())
    );
}

#[test]
fn opening_command_palette_without_selected_instance_has_only_navigation_commands() {
    let mut app = AppState::new();
    reduce(&mut app, &Action::Workspace(WorkspaceAction::Resources));

    reduce(&mut app, &Action::Command(CommandAction::Open));

    assert!(app.root.command.is_open());
    let commands = app.root.command.visible_commands(&app.root);
    assert!(commands.contains(&CommandDescriptor::GoHome));
    assert!(!commands.contains(&CommandDescriptor::StartSelectedComputeInstance));
    assert_eq!(
        app.root.command.selected_visible_command(&app.root),
        Some(CommandDescriptor::GoHome)
    );
}

#[test]
fn command_palette_filtering_is_case_insensitive_and_clamps_selection() {
    let mut app = app_with_selected_instance_status(ResourceStatus::Stopped);
    reduce(&mut app, &Action::Command(CommandAction::Open));
    reduce(&mut app, &Action::Command(CommandAction::SelectNext));
    for ch in "START".chars() {
        reduce(&mut app, &Action::Command(CommandAction::AppendChar(ch)));
    }

    assert_eq!(app.root.command.filter(), "START");
    assert_eq!(
        app.root.command.visible_commands(&app.root),
        vec![
            CommandDescriptor::StartSelectedComputeInstance,
            CommandDescriptor::StartTunnelForSelectedComputeInstance,
        ]
    );
    assert_eq!(
        app.root.command.selected_visible_command(&app.root),
        Some(CommandDescriptor::StartSelectedComputeInstance)
    );
}

#[test]
fn cancel_command_palette_closes_and_discards_filter() {
    let mut app = app_with_selected_instance();
    reduce(&mut app, &Action::Command(CommandAction::Open));
    reduce(&mut app, &Action::Command(CommandAction::AppendChar('s')));

    let effects = reduce(&mut app, &Action::Command(CommandAction::Cancel));

    assert!(effects.is_empty());
    assert!(!app.root.command.is_open());
    assert_eq!(app.root.command.filter(), "");
}

#[test]
fn submitting_selected_command_opens_resource_action_confirmation() {
    let mut app = app_with_selected_instance_status(ResourceStatus::Stopped);
    reduce(&mut app, &Action::Command(CommandAction::Open));

    let effects = reduce(&mut app, &Action::Command(CommandAction::Submit));

    assert!(effects.is_empty());
    assert!(!app.root.command.is_open());
    let confirmation = app
        .root
        .resources
        .action_confirmation()
        .expect("confirmation should open");
    assert_eq!(
        confirmation.action(),
        nuvem::app::state::resources::ComputeInstanceAction::Start
    );
    assert_eq!(confirmation.context().configuration.as_str(), "dev");
    assert_eq!(confirmation.instance().name, "api-dev-1");
}

#[test]
fn reset_suspend_and_resume_commands_open_confirmations_with_typed_effects() {
    for (status, descriptor, action) in [
        (
            ResourceStatus::Running,
            CommandDescriptor::ResetSelectedComputeInstance,
            ComputeInstanceAction::Reset,
        ),
        (
            ResourceStatus::Running,
            CommandDescriptor::SuspendSelectedComputeInstance,
            ComputeInstanceAction::Suspend,
        ),
        (
            ResourceStatus::Suspended,
            CommandDescriptor::ResumeSelectedComputeInstance,
            ComputeInstanceAction::Resume,
        ),
    ] {
        let mut app = app_with_selected_instance_status(status);
        let effects = select_command_from_open_palette(&mut app, descriptor);

        assert_eq!(
            app.root.resources.action_confirmation().unwrap().action(),
            action
        );
        assert!(effects.is_empty());

        let context = app.root.current_context().unwrap();
        let instance = selected_instance(&app);
        let effects = reduce(
            &mut app,
            &Action::Resources(ResourcesAction::ConfirmPendingAction),
        );
        // The effect carries the id of the entry this confirmation seeded.
        let operation_id = app.root.system.operations.records().front().unwrap().id;
        assert_eq!(
            effects,
            vec![Effect::ComputeInstanceAction {
                context,
                instance,
                action,
                operation_id,
            }]
        );
    }
}

/// The SSH preview's heading, command line and clipboard description must
/// survive the preview being generalised to carry a plugin's command. It is
/// the same modal, the same keymap arm and the same copy action; only what
/// fills it is now carried rather than formatted from a compute instance.
#[test]
fn the_ssh_preview_reads_exactly_as_it_did_before_the_preview_was_generalised() {
    let mut app = app_with_selected_instance_status(ResourceStatus::Running);
    select_command_from_open_palette(&mut app, CommandDescriptor::ShowSelectedComputeInstanceSsh);

    let preview = app
        .root
        .resources
        .command_preview()
        .expect("the modal is open");
    assert_eq!(preview.title(), "Show SSH command");
    assert_eq!(
        preview.command_preview(),
        "gcloud --configuration dev compute ssh api-dev-1 --zone europe-west1-b"
    );

    // The clipboard description is the toast's wording ("Copied SSH command to
    // clipboard"), so it is part of what must not drift when the preview stops
    // knowing it holds an SSH command.
    let effects = reduce(
        &mut app,
        &Action::Resources(ResourcesAction::CopyPendingCommand),
    );
    assert_eq!(
        effects,
        vec![Effect::CopyToClipboard {
            value: "gcloud --configuration dev compute ssh api-dev-1 --zone europe-west1-b"
                .to_string(),
            description: "SSH command".to_string(),
        }]
    );
}

#[test]
fn ssh_preview_enter_emits_clipboard_effect_and_escape_cancels() {
    let mut app = app_with_selected_instance_status(ResourceStatus::Running);
    select_command_from_open_palette(&mut app, CommandDescriptor::ShowSelectedComputeInstanceSsh);

    assert_eq!(
        app.root
            .resources
            .command_preview()
            .unwrap()
            .command_preview(),
        "gcloud --configuration dev compute ssh api-dev-1 --zone europe-west1-b"
    );

    let effects = reduce(
        &mut app,
        &Action::Resources(ResourcesAction::CopyPendingCommand),
    );
    assert_eq!(
        effects,
        vec![Effect::CopyToClipboard {
            value: "gcloud --configuration dev compute ssh api-dev-1 --zone europe-west1-b"
                .to_string(),
            description: "SSH command".to_string(),
        }]
    );
    assert!(app.root.resources.command_preview().is_none());

    select_command_from_open_palette(&mut app, CommandDescriptor::ShowSelectedComputeInstanceSsh);
    reduce(
        &mut app,
        &Action::Resources(ResourcesAction::CancelPendingCommandPreview),
    );
    assert!(app.root.resources.command_preview().is_none());
}

#[test]
fn ssh_command_opens_launch_preview() {
    let mut app = app_with_selected_instance_status(ResourceStatus::Running);
    let instance = selected_instance(&app);

    let effects =
        select_command_from_open_palette(&mut app, CommandDescriptor::SshSelectedComputeInstance);

    assert!(effects.is_empty());
    let preview = app
        .root
        .resources
        .ssh_launch_preview()
        .expect("launch preview opened");
    assert_eq!(preview.instance(), &instance);
}

#[test]
fn copy_field_command_emits_the_selected_value() {
    let mut app = app_with_selected_instance_status(ResourceStatus::Running);
    let effects =
        select_command_from_open_palette(&mut app, CommandDescriptor::CopySelectedInstanceName);

    assert_eq!(
        effects,
        vec![Effect::CopyToClipboard {
            value: "api-dev-1".to_string(),
            description: "instance name".to_string(),
        }]
    );
}

#[test]
fn submitting_command_while_palette_is_closed_does_not_open_confirmation() {
    let mut app = app_with_selected_instance();

    let effects = reduce(&mut app, &Action::Command(CommandAction::Submit));

    assert!(effects.is_empty());
    assert!(!app.root.command.is_open());
    assert!(app.root.resources.action_confirmation().is_none());
}

fn cloud_sql_instance(
    name: &str,
    availability: CloudSqlAvailability,
    status: CloudSqlStatus,
) -> CloudSqlInstance {
    CloudSqlInstance {
        name: name.to_string(),
        database_version: "POSTGRES_15".to_string(),
        engine: "PostgreSQL".to_string(),
        region: "us-central1".to_string(),
        tier: "db-custom-2-7680".to_string(),
        availability,
        status,
        connectivity: EndpointVisibility::Public,
    }
}

fn app_with_selected_cloud_sql(
    availability: CloudSqlAvailability,
    status: CloudSqlStatus,
) -> AppState {
    let mut app = AppState::new();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ConfigurationsLoaded {
            configurations: vec![config("dev", true, "dev-project")],
        }),
    );
    reduce(
        &mut app,
        &Action::Resources(ResourcesAction::SelectService(
            ResourceServiceKind::CloudSql,
        )),
    );
    let context = app.root.current_context().expect("context exists");
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::CloudSqlInstancesLoaded {
            context,
            instances: vec![cloud_sql_instance("db-dev-1", availability, status)],
        }),
    );
    reduce(&mut app, &Action::Workspace(WorkspaceAction::Resources));
    app
}

fn app_with_instances(instances: Vec<ComputeInstance>) -> AppState {
    let mut app = AppState::new();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ConfigurationsLoaded {
            configurations: vec![config("dev", true, "dev-project")],
        }),
    );
    let context = app.root.current_context().expect("context exists");
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ComputeInstancesLoaded { context, instances }),
    );
    reduce(&mut app, &Action::Workspace(WorkspaceAction::Resources));
    app
}

#[test]
fn failover_command_is_absent_for_zonal_and_present_for_regional() {
    // Zonal (non-HA), running: Failover must never be offered.
    let zonal = app_with_selected_cloud_sql(CloudSqlAvailability::Zonal, CloudSqlStatus::Runnable);
    let zonal_commands = zonal.root.command.visible_commands(&zonal.root);
    assert!(
        !zonal_commands.contains(&CommandDescriptor::FailoverSelectedCloudSqlInstance),
        "Failover must be hidden for a Zonal instance: {zonal_commands:?}"
    );
    // Other actions still appear for a running instance.
    assert!(zonal_commands.contains(&CommandDescriptor::StopSelectedCloudSqlInstance));

    // Regional (HA), running: Failover is offered.
    let regional =
        app_with_selected_cloud_sql(CloudSqlAvailability::Regional, CloudSqlStatus::Runnable);
    let regional_commands = regional.root.command.visible_commands(&regional.root);
    assert!(
        regional_commands.contains(&CommandDescriptor::FailoverSelectedCloudSqlInstance),
        "Failover must be offered for a Regional instance: {regional_commands:?}"
    );
}

/// Marks the two instances at visible indices 0 and 1 (assumes ascending
/// name sort, the default), leaving the selection on the second instance.
fn mark_first_two_visible(app: &mut AppState) {
    reduce(app, &Action::Resources(ResourcesAction::ToggleMark));
    reduce(app, &Action::Resources(ResourcesAction::SelectNext));
    reduce(app, &Action::Resources(ResourcesAction::ToggleMark));
}

#[test]
fn palette_label_reflects_marked_count_for_batch_actions() {
    use nuvem::app::state::command::palette_display_label;
    let mut app = app_with_instances(vec![
        instance_with_status("api-dev-1", ResourceStatus::Running),
        instance_with_status("worker-dev-1", ResourceStatus::Running),
    ]);

    // With zero marks the palette keeps the static single-row wording.
    assert_eq!(
        palette_display_label(CommandDescriptor::StopSelectedComputeInstance, &app.root),
        "Stop selected GCE instance"
    );

    mark_first_two_visible(&mut app);
    assert_eq!(app.root.resources.marked_count(), 2);

    // With marks, the five compute mutation commands read as the marked set.
    assert_eq!(
        palette_display_label(CommandDescriptor::StopSelectedComputeInstance, &app.root),
        "Stop 2 marked GCE instances"
    );
    assert_eq!(
        palette_display_label(CommandDescriptor::StartSelectedComputeInstance, &app.root),
        "Start 2 marked GCE instances"
    );

    // Non-mutation commands are unaffected by marks.
    assert_eq!(
        palette_display_label(CommandDescriptor::GoHome, &app.root),
        CommandDescriptor::GoHome.label()
    );
}

#[test]
fn submitting_stop_with_two_marked_running_instances_opens_batch_confirmation_requiring_count() {
    let mut app = app_with_instances(vec![
        instance_with_status("api-dev-1", ResourceStatus::Running),
        instance_with_status("worker-dev-1", ResourceStatus::Running),
    ]);
    mark_first_two_visible(&mut app);
    assert_eq!(app.root.resources.marked_count(), 2);

    let effects =
        select_command_from_open_palette(&mut app, CommandDescriptor::StopSelectedComputeInstance);

    assert!(effects.is_empty());
    let confirmation = app
        .root
        .resources
        .batch_confirmation()
        .expect("batch confirmation should open");
    assert_eq!(
        confirmation.action(),
        BatchAction::Compute(ComputeInstanceAction::Stop)
    );
    assert_eq!(confirmation.eligible().len(), 2);
    assert!(confirmation.skipped().is_empty());
    assert!(matches!(
        confirmation.ack(),
        nuvem::app::state::resources::BatchAck::CountRequired { typed } if typed.is_empty()
    ));
    assert!(app.root.resources.action_confirmation().is_none());
}

#[test]
fn submitting_stop_with_one_running_and_one_stopped_marked_skips_the_ineligible_instance() {
    let mut app = app_with_instances(vec![
        instance_with_status("api-dev-1", ResourceStatus::Running),
        instance_with_status("worker-dev-1", ResourceStatus::Stopped),
    ]);
    mark_first_two_visible(&mut app);
    assert_eq!(app.root.resources.marked_count(), 2);

    select_command_from_open_palette(&mut app, CommandDescriptor::StopSelectedComputeInstance);

    let confirmation = app
        .root
        .resources
        .batch_confirmation()
        .expect("batch confirmation should open");
    assert_eq!(confirmation.eligible().len(), 1);
    assert_eq!(confirmation.eligible()[0].name, "api-dev-1");
    assert_eq!(confirmation.skipped().len(), 1);
    assert_eq!(confirmation.skipped()[0].0.name, "worker-dev-1");
    assert_eq!(
        confirmation.skipped()[0].1,
        nuvem::app::state::resources::SkipReason::Ineligible("Stopped".to_string())
    );
}

#[test]
fn submitting_start_with_marked_stopped_instances_requires_only_simple_ack() {
    let mut app = app_with_instances(vec![
        instance_with_status("api-dev-1", ResourceStatus::Stopped),
        instance_with_status("worker-dev-1", ResourceStatus::Stopped),
    ]);
    mark_first_two_visible(&mut app);

    select_command_from_open_palette(&mut app, CommandDescriptor::StartSelectedComputeInstance);

    let confirmation = app
        .root
        .resources
        .batch_confirmation()
        .expect("batch confirmation should open");
    assert_eq!(confirmation.eligible().len(), 2);
    assert_eq!(
        confirmation.ack(),
        &nuvem::app::state::resources::BatchAck::Simple
    );
}

#[test]
fn submitting_stop_with_zero_marks_still_uses_the_single_row_confirmation() {
    let mut app = app_with_selected_instance_status(ResourceStatus::Running);
    assert_eq!(app.root.resources.marked_count(), 0);

    select_command_from_open_palette(&mut app, CommandDescriptor::StopSelectedComputeInstance);

    assert!(app.root.resources.batch_confirmation().is_none());
    let confirmation = app
        .root
        .resources
        .action_confirmation()
        .expect("single-row confirmation should open");
    assert_eq!(confirmation.action(), ComputeInstanceAction::Stop);
}

fn app_with_cloud_sql_instances(instances: Vec<CloudSqlInstance>) -> AppState {
    let mut app = AppState::new();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ConfigurationsLoaded {
            configurations: vec![config("dev", true, "dev-project")],
        }),
    );
    reduce(
        &mut app,
        &Action::Resources(ResourcesAction::SelectService(
            ResourceServiceKind::CloudSql,
        )),
    );
    let context = app.root.current_context().expect("context exists");
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::CloudSqlInstancesLoaded { context, instances }),
    );
    reduce(&mut app, &Action::Workspace(WorkspaceAction::Resources));
    app
}

/// Marks the first visible row and leaves the selection on the second, so the
/// selected row and the marked set disagree.
fn mark_first_then_select_second(app: &mut AppState) {
    reduce(app, &Action::Resources(ResourcesAction::ToggleMark));
    reduce(app, &Action::Resources(ResourcesAction::SelectNext));
}

#[test]
fn cloud_sql_command_is_offered_when_a_marked_row_is_eligible_but_the_selected_row_is_not() {
    let mut app = app_with_cloud_sql_instances(vec![
        cloud_sql_instance(
            "db-a",
            CloudSqlAvailability::Zonal,
            CloudSqlStatus::Runnable,
        ),
        cloud_sql_instance("db-b", CloudSqlAvailability::Zonal, CloudSqlStatus::Stopped),
    ]);
    mark_first_then_select_second(&mut app);
    assert_eq!(app.root.resources.marked_count(), 1);
    assert_eq!(
        app.root
            .resources
            .selected_cloud_sql_instance()
            .expect("selection exists")
            .name,
        "db-b"
    );

    let commands = app.root.command.visible_commands(&app.root);

    // Stop is ineligible for the selected (Stopped) row, but the marked row is
    // Runnable, so the batch-capable descriptor must still be offered.
    assert!(
        commands.contains(&CommandDescriptor::StopSelectedCloudSqlInstance),
        "Stop must be offered because a marked row is eligible: {commands:?}"
    );
    // Start is eligible for the selected row only; with marks present the
    // marked set alone decides, so it must not be offered.
    assert!(
        !commands.contains(&CommandDescriptor::StartSelectedCloudSqlInstance),
        "Start must not be offered when no marked row is eligible: {commands:?}"
    );
}

#[test]
fn cloud_sql_failover_gating_over_marks_still_requires_high_availability() {
    let mut app = app_with_cloud_sql_instances(vec![
        cloud_sql_instance(
            "db-a",
            CloudSqlAvailability::Zonal,
            CloudSqlStatus::Runnable,
        ),
        cloud_sql_instance(
            "db-b",
            CloudSqlAvailability::Regional,
            CloudSqlStatus::Runnable,
        ),
    ]);
    // Mark the Zonal row, select the Regional one.
    mark_first_then_select_second(&mut app);

    let commands = app.root.command.visible_commands(&app.root);

    assert!(
        !commands.contains(&CommandDescriptor::FailoverSelectedCloudSqlInstance),
        "Failover must stay hidden when the only marked row is Zonal: {commands:?}"
    );
    assert!(commands.contains(&CommandDescriptor::StopSelectedCloudSqlInstance));
}

#[test]
fn submitting_stop_with_marked_cloud_sql_instances_opens_batch_confirmation_requiring_count() {
    let mut app = app_with_cloud_sql_instances(vec![
        cloud_sql_instance(
            "db-a",
            CloudSqlAvailability::Zonal,
            CloudSqlStatus::Runnable,
        ),
        cloud_sql_instance(
            "db-b",
            CloudSqlAvailability::Zonal,
            CloudSqlStatus::Runnable,
        ),
    ]);
    mark_first_two_visible(&mut app);
    assert_eq!(app.root.resources.marked_count(), 2);

    let effects =
        select_command_from_open_palette(&mut app, CommandDescriptor::StopSelectedCloudSqlInstance);

    assert!(effects.is_empty());
    let confirmation = app
        .root
        .resources
        .batch_confirmation()
        .expect("batch confirmation should open");
    assert_eq!(
        confirmation.action(),
        BatchAction::CloudSql(CloudSqlInstanceAction::Stop)
    );
    assert_eq!(confirmation.eligible().len(), 2);
    assert!(confirmation.skipped().is_empty());
    assert!(matches!(
        confirmation.ack(),
        nuvem::app::state::resources::BatchAck::CountRequired { typed } if typed.is_empty()
    ));
    assert!(app.root.resources.action_confirmation().is_none());
}

#[test]
fn submitting_stop_with_one_runnable_and_one_stopped_marked_cloud_sql_row_reports_the_skip() {
    let mut app = app_with_cloud_sql_instances(vec![
        cloud_sql_instance(
            "db-a",
            CloudSqlAvailability::Zonal,
            CloudSqlStatus::Runnable,
        ),
        cloud_sql_instance("db-b", CloudSqlAvailability::Zonal, CloudSqlStatus::Stopped),
    ]);
    mark_first_two_visible(&mut app);
    assert_eq!(app.root.resources.marked_count(), 2);

    select_command_from_open_palette(&mut app, CommandDescriptor::StopSelectedCloudSqlInstance);

    let confirmation = app
        .root
        .resources
        .batch_confirmation()
        .expect("batch confirmation should open");
    assert_eq!(confirmation.eligible().len(), 1);
    assert_eq!(confirmation.eligible()[0].name, "db-a");
    assert_eq!(confirmation.skipped().len(), 1);
    assert_eq!(confirmation.skipped()[0].0.name, "db-b");
    assert_eq!(
        confirmation.skipped()[0].1,
        nuvem::app::state::resources::SkipReason::Ineligible("Stopped".to_string())
    );
}

#[test]
fn submitting_a_cloud_sql_action_with_zero_marks_still_uses_the_single_row_confirmation() {
    let mut app =
        app_with_selected_cloud_sql(CloudSqlAvailability::Zonal, CloudSqlStatus::Runnable);
    assert_eq!(app.root.resources.marked_count(), 0);

    select_command_from_open_palette(&mut app, CommandDescriptor::StopSelectedCloudSqlInstance);

    assert!(app.root.resources.batch_confirmation().is_none());
    let confirmation = app
        .root
        .resources
        .action_confirmation()
        .expect("single-row confirmation should open")
        .cloud_sql()
        .expect("Cloud SQL confirmation");
    assert_eq!(confirmation.action(), CloudSqlInstanceAction::Stop);
    assert_eq!(confirmation.instance().name, "db-dev-1");
}

#[test]
fn palette_label_reflects_marked_count_for_cloud_sql_batch_actions() {
    use nuvem::app::state::command::palette_display_label;
    let mut app = app_with_cloud_sql_instances(vec![
        cloud_sql_instance(
            "db-a",
            CloudSqlAvailability::Regional,
            CloudSqlStatus::Runnable,
        ),
        cloud_sql_instance(
            "db-b",
            CloudSqlAvailability::Regional,
            CloudSqlStatus::Runnable,
        ),
    ]);

    // With zero marks the palette keeps the static single-row wording.
    assert_eq!(
        palette_display_label(CommandDescriptor::StopSelectedCloudSqlInstance, &app.root),
        "Stop selected Cloud SQL instance"
    );

    mark_first_two_visible(&mut app);
    assert_eq!(app.root.resources.marked_count(), 2);

    assert_eq!(
        palette_display_label(CommandDescriptor::StopSelectedCloudSqlInstance, &app.root),
        "Stop 2 marked Cloud SQL instances"
    );
    assert_eq!(
        palette_display_label(CommandDescriptor::StartSelectedCloudSqlInstance, &app.root),
        "Start 2 marked Cloud SQL instances"
    );
    assert_eq!(
        palette_display_label(
            CommandDescriptor::RestartSelectedCloudSqlInstance,
            &app.root
        ),
        "Restart 2 marked Cloud SQL instances"
    );
    assert_eq!(
        palette_display_label(
            CommandDescriptor::FailoverSelectedCloudSqlInstance,
            &app.root
        ),
        "Fail over 2 marked Cloud SQL instances"
    );

    // Non-batch Cloud SQL commands keep their static wording.
    assert_eq!(
        palette_display_label(
            CommandDescriptor::CopySelectedCloudSqlConnectionName,
            &app.root
        ),
        CommandDescriptor::CopySelectedCloudSqlConnectionName.label()
    );
}

fn app_home_with_context() -> AppState {
    let mut app = AppState::new();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ConfigurationsLoaded {
            configurations: vec![config("dev", true, "dev-project")],
        }),
    );
    reduce(&mut app, &Action::Workspace(WorkspaceAction::Home));
    app
}

#[test]
fn palette_is_never_empty_from_home_and_lists_navigation() {
    let app = app_home_with_context();
    let commands = app.root.command.visible_commands(&app.root);
    assert!(commands.contains(&CommandDescriptor::GoHome));
    assert!(commands.contains(&CommandDescriptor::GoToConfigurations));
    assert!(commands.contains(&CommandDescriptor::GoToService(
        ResourceServiceKind::ComputeInstances
    )));
    assert!(commands.contains(&CommandDescriptor::GoToService(
        ResourceServiceKind::GkeClusters
    )));
    assert!(commands.contains(&CommandDescriptor::GoToService(
        ResourceServiceKind::CloudRunServices
    )));
    assert!(commands.contains(&CommandDescriptor::ShowHelp));
    assert!(commands.contains(&CommandDescriptor::SwitchProjectContext));
    // Home has nothing to refresh.
    assert!(!commands.contains(&CommandDescriptor::RefreshCurrentView));
}

#[test]
fn palette_shows_cloud_run_now_available_in_service_registry() {
    // navigation_commands() consults the service registry's ServiceAvailability;
    // Cloud Run is now Available (Task 10), so it must surface as a navigation
    // command in the palette.
    let app = app_home_with_context();
    let commands = app.root.command.visible_commands(&app.root);
    assert!(
        commands.contains(&CommandDescriptor::GoToService(
            ResourceServiceKind::CloudRunServices
        )),
        "Cloud Run must appear in the command palette once its service-registry \
         descriptor is Available"
    );
}

#[test]
fn refresh_is_available_in_resources_workspace() {
    let app = app_with_selected_instance();
    let commands = app.root.command.visible_commands(&app.root);
    assert!(commands.contains(&CommandDescriptor::RefreshCurrentView));
}

#[test]
fn switch_project_context_is_hidden_without_a_context() {
    let app = AppState::new(); // no configurations loaded -> no context
    let commands = app.root.command.visible_commands(&app.root);
    assert!(!commands.contains(&CommandDescriptor::SwitchProjectContext));
    // But the palette is still non-empty.
    assert!(!commands.is_empty());
}

#[test]
fn context_actions_are_ordered_before_navigation() {
    let app = app_with_selected_instance();
    let commands = app.root.command.visible_commands(&app.root);
    let first_nav = commands
        .iter()
        .position(|command| *command == CommandDescriptor::GoHome)
        .expect("navigation present");
    let last_context = commands
        .iter()
        .position(|command| *command == CommandDescriptor::StopSelectedComputeInstance)
        .expect("context present");
    assert!(
        last_context < first_nav,
        "context must precede navigation: {commands:?}"
    );
}

#[test]
fn navigation_descriptors_expose_labels_and_aliases() {
    assert_eq!(CommandDescriptor::GoHome.label(), "Home");
    assert_eq!(
        CommandDescriptor::GoToConfigurations.label(),
        "Configurations"
    );
    assert_eq!(
        CommandDescriptor::GoToService(ResourceServiceKind::ComputeInstances).label(),
        "GCE instances"
    );
    assert_eq!(
        CommandDescriptor::GoToService(ResourceServiceKind::GkeClusters).label(),
        "GKE clusters"
    );
    assert_eq!(
        CommandDescriptor::GoToService(ResourceServiceKind::CloudRunServices).label(),
        "Cloud Run"
    );
    assert_eq!(CommandDescriptor::RefreshCurrentView.label(), "Refresh");
    assert_eq!(CommandDescriptor::ShowHelp.label(), "Help");
    assert_eq!(
        CommandDescriptor::SwitchProjectContext.label(),
        "Switch project / context"
    );

    assert_eq!(
        CommandDescriptor::GoToService(ResourceServiceKind::ComputeInstances).aliases(),
        &["gce", "compute", "instances"]
    );
    assert_eq!(
        CommandDescriptor::GoToService(ResourceServiceKind::GkeClusters).aliases(),
        &["gke", "clusters"]
    );
    assert_eq!(
        CommandDescriptor::GoToService(ResourceServiceKind::CloudRunServices).aliases(),
        &["run", "serverless", "cloud run"]
    );
    assert_eq!(
        CommandDescriptor::GoToConfigurations.aliases(),
        &["configurations", "configs", "config"]
    );
    assert!(
        CommandDescriptor::StartSelectedComputeInstance
            .aliases()
            .is_empty()
    );
}

/// What each log command answers to in the palette.
///
/// Walks `ALL` rather than naming services, for the same reason the palette's
/// entries are now generated from it: the interesting content is the two
/// exceptions, and a per-service list would go stale beside them.
///
/// Both exceptions were surviving mutants before this test existed — deleting
/// either arm left the `_` catch-all or the generic `OpenLogsForSelected(_)`
/// arm to answer, and nothing objected. "logging" is a word users reach for
/// when they mean logs, so losing it costs a search that should have hit.
#[test]
fn log_commands_answer_to_the_words_users_search_for() {
    assert_eq!(
        CommandDescriptor::OpenProjectLogs.aliases(),
        &["logs", "logging"],
        "project logs must stay findable under both words"
    );

    // Cloud Run is the one service whose per-selection log command carries
    // "logging" too, matching the project-wide entry.
    assert_eq!(
        CommandDescriptor::OpenLogsForSelected(ResourceServiceKind::CloudRunServices).aliases(),
        &["logs", "logging"]
    );

    for kind in ResourceServiceKind::ALL {
        if kind == ResourceServiceKind::CloudRunServices {
            continue;
        }
        assert_eq!(
            CommandDescriptor::OpenLogsForSelected(kind).aliases(),
            &["logs"],
            "{kind:?} should answer to the default log alias"
        );
    }

    assert_eq!(
        CommandDescriptor::OpenLogsForSelectedCloudRunRevision.aliases(),
        &["logs"],
        "the revision entry is not generated from ALL and needs its own pin"
    );
}

fn filter_palette(app: &mut AppState, query: &str) -> Vec<CommandDescriptor> {
    reduce(app, &Action::Command(CommandAction::Open));
    for ch in query.chars() {
        reduce(app, &Action::Command(CommandAction::AppendChar(ch)));
    }
    let visible = app.root.command.visible_commands(&app.root);
    reduce(app, &Action::Command(CommandAction::Cancel));
    visible
}

#[test]
fn exact_alias_ranks_before_prefix_and_substring() {
    let mut app = app_home_with_context();
    // "gke" is an exact alias of GoToService(GkeClusters).
    let visible = filter_palette(&mut app, "gke");
    assert_eq!(
        visible.first().copied(),
        Some(CommandDescriptor::GoToService(
            ResourceServiceKind::GkeClusters
        ))
    );
}

#[test]
fn alias_search_is_case_insensitive_and_matches_compute() {
    let mut app = app_home_with_context();
    let visible = filter_palette(&mut app, "COMPUTE");
    assert!(visible.contains(&CommandDescriptor::GoToService(
        ResourceServiceKind::ComputeInstances
    )));
}

#[test]
fn prefix_matches_rank_above_substring_matches() {
    // "conf" is a prefix of "Configurations"/"config" (tier 1) and a substring
    // of nothing else that outranks it; GoToConfigurations should appear first.
    let mut app = app_home_with_context();
    let visible = filter_palette(&mut app, "conf");
    assert_eq!(
        visible.first().copied(),
        Some(CommandDescriptor::GoToConfigurations)
    );
}

#[test]
fn nonmatching_filter_yields_empty_visible_list() {
    let mut app = app_home_with_context();
    let visible = filter_palette(&mut app, "zzzzz");
    assert!(visible.is_empty());
}

#[test]
fn user_alias_surfaces_the_matching_go_to_service_command() {
    let mut app = app_home_with_context();
    let raw: BTreeMap<String, String> = [("vms".to_string(), "gce".to_string())]
        .into_iter()
        .collect();
    let mut problems = ConfigProblems::default();
    app.root.aliases = AliasTable::from_file(&raw, &mut problems);
    assert!(problems.is_empty());

    let visible = filter_palette(&mut app, "vms");
    assert!(visible.contains(&CommandDescriptor::GoToService(
        ResourceServiceKind::ComputeInstances
    )));
}

#[test]
fn navigating_home_updates_workspace() {
    let mut app = app_with_selected_instance();
    let effects = select_command_from_open_palette(&mut app, CommandDescriptor::GoHome);
    assert!(effects.is_empty());
    assert_eq!(app.root.workspace, Workspace::Home);
    assert!(!app.root.command.is_open());
}

#[test]
fn navigating_to_configurations_updates_workspace() {
    let mut app = app_home_with_context();
    let effects = select_command_from_open_palette(&mut app, CommandDescriptor::GoToConfigurations);
    assert!(effects.is_empty());
    assert_eq!(app.root.workspace, Workspace::Configurations);
}

#[test]
fn navigating_to_a_service_from_detail_returns_to_the_list() {
    let mut app = app_with_selected_instance();
    reduce(&mut app, &Action::Resources(ResourcesAction::OpenSelected));
    assert!(app.root.resources.is_showing_detail());
    let effects = select_command_from_open_palette(
        &mut app,
        CommandDescriptor::GoToService(ResourceServiceKind::ComputeInstances),
    );
    assert!(effects.is_empty());
    assert_eq!(app.root.workspace, Workspace::Resources);
    assert!(!app.root.resources.is_showing_detail());
}

#[test]
fn navigating_to_a_service_syncs_navigator_selection_with_active_service() {
    let mut app = app_with_selected_instance();
    assert_eq!(
        app.root.navigator.selected_id(),
        ServiceId::ComputeInstances
    );

    let effects = select_command_from_open_palette(
        &mut app,
        CommandDescriptor::GoToService(ResourceServiceKind::GkeClusters),
    );

    // The palette jump changes the active service, so it also emits the
    // remembered-view save. GKE's list is not fetched here — this
    // fixture has no rows for it and `GoToService` does not load — so the save
    // is the whole of the effect list.
    assert_eq!(
        effects,
        vec![Effect::SaveRememberedViews {
            views: remembered("dev", ResourceServiceKind::GkeClusters),
        }]
    );
    assert_eq!(app.root.workspace, Workspace::Resources);
    assert_eq!(
        app.root.resources.active_service(),
        ResourceServiceKind::GkeClusters
    );
    assert_eq!(app.root.navigator.selected_id(), ServiceId::GkeClusters);
}

#[test]
fn refresh_command_reloads_the_active_service() {
    let mut app = app_with_selected_instance();
    let context = app.root.current_context().expect("context exists");
    let effects = select_command_from_open_palette(&mut app, CommandDescriptor::RefreshCurrentView);
    assert_eq!(
        effects,
        vec![Effect::RefreshComputeInstances {
            context,
            filter: None
        }]
    );
}

#[test]
fn refresh_command_reloads_configurations() {
    let mut app = app_with_selected_inactive_configuration();
    let effects = select_command_from_open_palette(&mut app, CommandDescriptor::RefreshCurrentView);
    assert_eq!(effects, vec![Effect::LoadConfigurations]);
}

#[test]
fn reload_config_command_is_available_and_emits_reload_effect() {
    let mut app = app_home_with_context();
    let commands = {
        reduce(&mut app, &Action::Command(CommandAction::Open));
        let visible = app.root.command.visible_commands(&app.root);
        reduce(&mut app, &Action::Command(CommandAction::Cancel));
        visible
    };
    assert!(commands.contains(&CommandDescriptor::ReloadConfig));
    assert_eq!(
        CommandDescriptor::ReloadConfig.label(),
        "Reload configuration"
    );
    assert_eq!(
        CommandDescriptor::ReloadConfig.aliases(),
        &["reload config", "config reload"]
    );

    let effects = select_command_from_open_palette(&mut app, CommandDescriptor::ReloadConfig);
    assert_eq!(effects, vec![Effect::ReloadConfig]);
    assert!(!app.root.command.is_open());
}

#[test]
fn help_command_opens_help_overlay() {
    let mut app = app_with_selected_instance();
    let effects = select_command_from_open_palette(&mut app, CommandDescriptor::ShowHelp);
    assert!(effects.is_empty());
    assert!(app.root.system.show_help);
}

#[test]
fn switch_project_context_opens_active_context_edit_form() {
    let mut app = app_with_selected_instance();
    let effects =
        select_command_from_open_palette(&mut app, CommandDescriptor::SwitchProjectContext);
    assert!(effects.is_empty());
    assert_eq!(app.root.workspace, Workspace::Configurations);
    assert!(app.root.configurations.is_context_edit_form_open());
}

fn gke_cluster(name: &str) -> GkeCluster {
    GkeCluster {
        name: name.to_string(),
        location: "europe-west1".to_string(),
        status: GkeClusterStatus::Running,
        current_master_version: Some("1.30".to_string()),
        current_node_count: Some(3),
        endpoint_visibility: EndpointVisibility::Public,
    }
}

fn app_with_selected_gke_cluster() -> AppState {
    let mut app = AppState::new();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ConfigurationsLoaded {
            configurations: vec![config("dev", true, "dev-project")],
        }),
    );
    let context = app.root.current_context().expect("context exists");
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::GkeClustersLoaded {
            context,
            clusters: vec![gke_cluster("primary")],
        }),
    );
    reduce(&mut app, &Action::Workspace(WorkspaceAction::Resources));
    reduce(
        &mut app,
        &Action::Resources(ResourcesAction::SelectService(
            ResourceServiceKind::GkeClusters,
        )),
    );
    app
}

#[test]
fn open_project_logs_command_switches_to_logging_and_loads() {
    let mut app = app_with_selected_instance();
    let context = app.root.current_context().expect("context exists");
    let effects = select_command_from_open_palette(&mut app, CommandDescriptor::OpenProjectLogs);

    assert_eq!(app.root.workspace, Workspace::Logging);
    assert_eq!(
        effects,
        vec![Effect::LoadRecentLogs {
            context,
            query: LogQuery::ProjectLogs,
            severity: app.root.logging.severity_floor,
            range: app.root.logging.time_range,
        }]
    );
}

#[test]
fn open_logs_for_selected_instance_uses_concrete_identity() {
    let mut app = app_with_selected_instance();
    let context = app.root.current_context().expect("context exists");
    let instance = selected_instance(&app);
    let effects = select_command_from_open_palette(
        &mut app,
        CommandDescriptor::OpenLogsForSelected(ResourceServiceKind::ComputeInstances),
    );

    assert_eq!(app.root.workspace, Workspace::Logging);
    assert_eq!(
        effects,
        vec![Effect::LoadRecentLogs {
            context,
            query: LogQuery::Resource(LogResourceKind::ComputeInstance {
                instance_id: instance.id.clone(),
                instance_name: instance.name.clone(),
                zone: instance.zone.clone(),
            }),
            severity: app.root.logging.severity_floor,
            range: app.root.logging.time_range,
        }]
    );
}

fn cloud_run_service(name: &str, region: &str) -> CloudRunService {
    CloudRunService {
        name: name.to_string(),
        region: region.to_string(),
        url: None,
        latest_ready_revision: None,
        latest_created_revision: None,
        traffic_summary: "-".to_string(),
        status: CloudRunServiceStatus::Ready,
        ready_message: None,
    }
}

fn app_with_selected_cloud_run_service() -> AppState {
    let mut app = AppState::new();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ConfigurationsLoaded {
            configurations: vec![config("dev", true, "dev-project")],
        }),
    );
    let context = app.root.current_context().expect("context exists");
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::CloudRunServicesLoaded {
            context,
            services: vec![cloud_run_service("api", "us-central1")],
        }),
    );
    reduce(&mut app, &Action::Workspace(WorkspaceAction::Resources));
    reduce(
        &mut app,
        &Action::Resources(ResourcesAction::SelectService(
            ResourceServiceKind::CloudRunServices,
        )),
    );
    app
}

#[test]
fn open_logs_for_selected_cloud_run_service_uses_concrete_identity() {
    let mut app = app_with_selected_cloud_run_service();
    let context = app.root.current_context().expect("context exists");
    let service = app
        .root
        .resources
        .selected_cloud_run_service()
        .expect("selected service exists")
        .clone();
    let effects = select_command_from_open_palette(
        &mut app,
        CommandDescriptor::OpenLogsForSelected(ResourceServiceKind::CloudRunServices),
    );

    assert_eq!(app.root.workspace, Workspace::Logging);
    assert_eq!(
        effects,
        vec![Effect::LoadRecentLogs {
            context: context.clone(),
            query: LogQuery::Resource(LogResourceKind::CloudRunService {
                service_name: service.name.clone(),
                region: service.region.clone(),
                project: context.project.clone().unwrap_or_default(),
            }),
            severity: app.root.logging.severity_floor,
            range: app.root.logging.time_range,
        }]
    );
}

#[test]
fn open_logs_for_selected_gke_cluster_uses_concrete_identity() {
    let mut app = app_with_selected_gke_cluster();
    let context = app.root.current_context().expect("context exists");
    let cluster = app
        .root
        .resources
        .selected_gke_cluster()
        .expect("selected cluster exists")
        .clone();
    let effects = select_command_from_open_palette(
        &mut app,
        CommandDescriptor::OpenLogsForSelected(ResourceServiceKind::GkeClusters),
    );

    assert_eq!(app.root.workspace, Workspace::Logging);
    assert_eq!(
        effects,
        vec![Effect::LoadRecentLogs {
            context: context.clone(),
            query: LogQuery::Resource(LogResourceKind::GkeCluster {
                cluster_name: cluster.name.clone(),
                location: cluster.location.clone(),
                project: context.project.clone().unwrap_or_default(),
            }),
            severity: app.root.logging.severity_floor,
            range: app.root.logging.time_range,
        }]
    );
}

// ---------------------------------------------------------------------------
// Tunnels workspace navigation command
// ---------------------------------------------------------------------------

/// Runs the palette against `query` and returns the command it would submit.
fn palette_match(app: &mut AppState, query: &str) -> Option<CommandDescriptor> {
    reduce(app, &Action::Command(CommandAction::Open));
    for ch in query.chars() {
        reduce(app, &Action::Command(CommandAction::AppendChar(ch)));
    }
    let selected = app.root.command.selected_visible_command(&app.root);
    reduce(app, &Action::Command(CommandAction::Cancel));
    selected
}

#[test]
fn the_tunnels_command_is_always_offered_and_labelled_tunnels() {
    let app = AppState::new();
    assert!(
        app.root
            .command
            .visible_commands(&app.root)
            .contains(&CommandDescriptor::GoToTunnels)
    );
    assert_eq!(CommandDescriptor::GoToTunnels.label(), "Tunnels");
}

#[test]
fn every_documented_tunnels_alias_resolves_to_the_tunnels_command() {
    for alias in ["tunnels", "pf", "forward", "port forward"] {
        let mut app = AppState::new();
        assert_eq!(
            palette_match(&mut app, alias),
            Some(CommandDescriptor::GoToTunnels),
            "alias {alias:?} should select the Tunnels command"
        );
    }
}

#[test]
fn submitting_the_tunnels_command_opens_the_tunnels_workspace() {
    let mut app = AppState::new();
    reduce(&mut app, &Action::Command(CommandAction::Open));
    for ch in "tunnels".chars() {
        reduce(&mut app, &Action::Command(CommandAction::AppendChar(ch)));
    }

    let effects = reduce(&mut app, &Action::Command(CommandAction::Submit));

    assert!(effects.is_empty(), "opening Tunnels fetches nothing");
    assert!(!app.root.command.is_open());
    assert_eq!(app.root.workspace, Workspace::Tunnels);
}

/// There is nothing to re-fetch in the Tunnels workspace, so the palette must
/// not offer a refresh that would do nothing (or, worse, refresh another
/// workspace's data).
#[test]
fn the_palette_offers_no_refresh_command_in_the_tunnels_workspace() {
    let mut app = AppState::new();
    reduce(&mut app, &Action::Workspace(WorkspaceAction::Tunnels));
    assert!(
        !app.root
            .command
            .visible_commands(&app.root)
            .contains(&CommandDescriptor::RefreshCurrentView)
    );
}

#[test]
fn start_tunnel_command_for_a_gce_instance_opens_the_launch_preview() {
    let mut app = app_with_selected_instance_status(ResourceStatus::Running);

    let effects = select_command_from_open_palette(
        &mut app,
        CommandDescriptor::StartTunnelForSelectedComputeInstance,
    );

    assert!(effects.is_empty(), "the preview is state, not an effect");
    let preview = app
        .root
        .resources
        .tunnel_launch_preview()
        .expect("tunnel launch preview opened");
    assert_eq!(preview.plan().instance_name, "api-dev-1");
    assert_eq!(preview.plan().kind, TunnelKind::IapTcp);
}

#[test]
fn start_tunnel_command_for_a_cloud_sql_instance_opens_the_launch_preview() {
    let mut app =
        app_with_selected_cloud_sql(CloudSqlAvailability::Zonal, CloudSqlStatus::Runnable);

    let effects = select_command_from_open_palette(
        &mut app,
        CommandDescriptor::StartTunnelForSelectedCloudSqlInstance,
    );

    assert!(effects.is_empty());
    let preview = app
        .root
        .resources
        .tunnel_launch_preview()
        .expect("tunnel launch preview opened");
    assert_eq!(preview.plan().kind, TunnelKind::CloudSqlProxy);
    assert_eq!(
        preview.plan().connection_name,
        "dev-project:us-central1:db-dev-1"
    );
}

/// Both entries share the tunnel aliases, so typing `pf` finds whichever one
/// the active service offers.
#[test]
fn tunnel_aliases_match_the_start_tunnel_commands() {
    for alias in ["tunnel", "pf", "forward"] {
        let mut app = app_with_selected_instance_status(ResourceStatus::Running);
        reduce(&mut app, &Action::Command(CommandAction::Open));
        for ch in alias.chars() {
            reduce(&mut app, &Action::Command(CommandAction::AppendChar(ch)));
        }
        let visible = app.root.command.visible_commands(&app.root);
        assert!(
            visible.contains(&CommandDescriptor::StartTunnelForSelectedComputeInstance),
            "alias {alias:?} should surface the GCE tunnel command, got {visible:?}"
        );
    }
}

/// The read-only gate lives in the reducer's `Submit` arm, and it is gated on
/// `CommandDescriptor::is_mutating` rather than on the destructiveness axis
/// used by the confirmation modals. All three cases below exist to pin that
/// choice: `Stop` proves a gate exists at all, `Start` proves the gate is on
/// the *mutation* axis, and the copy case proves the gate is not simply
/// refusing everything.
#[test]
fn read_only_refuses_mutating_commands_but_not_the_rest() {
    // Destructive-and-mutating: the obvious case.
    let mut app = app_with_selected_instance_status(ResourceStatus::Running);
    app.root.read_only = true;
    let effects =
        select_command_from_open_palette(&mut app, CommandDescriptor::StopSelectedComputeInstance);
    assert!(
        app.root.resources.action_confirmation().is_none(),
        "read-only must refuse Stop: it mutates the cloud instance's lifecycle state"
    );
    assert!(effects.is_empty());
    assert_eq!(
        app.root.system.status_message.as_deref(),
        Some(
            "Stop selected GCE instance is unavailable: read-only mode refuses cloud-resource mutations"
        ),
        "a blocked submit must explain itself instead of feeling like a dead key"
    );

    // Mutating but NOT destructive: the discriminator. A gate written against
    // `is_destructive` passes the Stop case above and FAILS here, because
    // `ComputeInstanceAction::Start::is_destructive()` is false. That is the
    // entire point of `is_mutating` being a separate axis — Start boots the
    // instance and resumes billing, which is a cloud-resource mutation.
    let mut app = app_with_selected_instance_status(ResourceStatus::Stopped);
    app.root.read_only = true;
    let effects =
        select_command_from_open_palette(&mut app, CommandDescriptor::StartSelectedComputeInstance);
    assert!(
        app.root.resources.action_confirmation().is_none(),
        "read-only must refuse Start: it is not destructive, but it does mutate cloud state \
         (boots the instance, billing resumes) — a gate built on is_destructive would let it through"
    );
    assert!(effects.is_empty());

    // Non-mutating: read-only refuses cloud-resource mutations only, so a
    // clipboard read of an already-fetched field must still work. A gate that
    // blocks everything fails here.
    let mut app = app_with_selected_instance_status(ResourceStatus::Running);
    app.root.read_only = true;
    let effects =
        select_command_from_open_palette(&mut app, CommandDescriptor::CopySelectedInstanceName);
    assert_eq!(
        effects,
        vec![Effect::CopyToClipboard {
            value: "api-dev-1".to_string(),
            description: "instance name".to_string(),
        }],
        "read-only must not block non-mutating commands: copying a name changes nothing in the cloud"
    );
}

/// With rows marked, the palette row the operator submitted read "Stop 2
/// marked GCE instances" (see `palette_display_label`), so the refusal must
/// name that same command — "Stop selected GCE instance" would misstate the
/// blast radius of what was actually refused.
#[test]
fn read_only_refusal_names_the_batch_label_the_palette_row_showed() {
    let mut app = app_with_instances(vec![
        instance_with_status("api-dev-1", ResourceStatus::Running),
        instance_with_status("worker-dev-1", ResourceStatus::Running),
    ]);
    mark_first_two_visible(&mut app);
    assert_eq!(app.root.resources.marked_count(), 2);
    app.root.read_only = true;

    let effects =
        select_command_from_open_palette(&mut app, CommandDescriptor::StopSelectedComputeInstance);

    assert!(effects.is_empty());
    assert!(
        app.root.resources.batch_confirmation().is_none(),
        "read-only must refuse the batch, not open its confirmation"
    );
    assert_eq!(
        app.root.system.status_message.as_deref(),
        Some(
            "Stop 2 marked GCE instances is unavailable: read-only mode refuses \
             cloud-resource mutations"
        ),
        "the refusal must use the batch-aware label the palette row showed"
    );
}

/// k9s is not a lifecycle action, which is exactly why it was missed: it is a
/// full read-write Kubernetes UI, and the launch effect resolves a kube context
/// (running `get-credentials` when absent) before spawning it. Read-only has to
/// refuse the launch, not just the API-call commands.
#[test]
fn read_only_refuses_opening_a_cluster_in_k9s() {
    let mut app = app_with_selected_gke_cluster();
    app.root.read_only = true;

    let effects =
        select_command_from_open_palette(&mut app, CommandDescriptor::OpenSelectedGkeClusterInK9s);

    assert!(
        !effects
            .iter()
            .any(|effect| matches!(effect, Effect::PrepareK9sLaunch { .. })),
        "read-only must not spawn a read-write cluster UI: {effects:?}"
    );
    assert_eq!(
        app.root.system.status_message.as_deref(),
        Some(
            "Open selected GKE cluster in k9s is unavailable: read-only mode refuses \
             cloud-resource mutations"
        )
    );
}

/// The submit gate cannot see a confirmation that was already open when the
/// mode came on — a config reload can turn read-only ON mid-session, and the
/// modal sitting on screen is still one keypress from a cloud mutation. The
/// gate therefore has to exist at CONFIRM as well.
#[test]
fn read_only_turned_on_after_a_confirmation_opened_refuses_the_confirm() {
    let mut app = app_with_selected_instance_status(ResourceStatus::Running);
    select_command_from_open_palette(&mut app, CommandDescriptor::StopSelectedComputeInstance);
    assert!(
        app.root.resources.action_confirmation().is_some(),
        "the confirmation opens while the mode is still off"
    );

    // A config reload turns the mode on with the modal already live.
    app.root.read_only = true;

    let effects = reduce(
        &mut app,
        &Action::Resources(ResourcesAction::ConfirmPendingAction),
    );

    assert!(
        effects.is_empty(),
        "confirming an already-open mutation must not reach the cloud: {effects:?}"
    );
    assert!(
        app.root.resources.action_confirmation().is_none(),
        "the refused confirmation must close, not sit there re-refusing"
    );
    assert!(
        app.root.system.status_message.as_deref().is_some_and(
            |message| message.contains("read-only mode refuses cloud-resource mutations")
        )
    );
    assert_eq!(
        app.root.system.status_phase,
        Some(OperationPhase::Healthy),
        "the refusal is a healthy status line, not a failure — and not \
         Degraded, Loading, or absent either, which an assert_ne! against \
         Failed would have accepted"
    );
}

/// Same hole on the batch path: the batch confirmation is a separate modal
/// with its own confirm action.
#[test]
fn read_only_turned_on_after_a_batch_confirmation_opened_refuses_the_confirm() {
    let mut app = app_with_instances(vec![
        instance_with_status("api-dev-1", ResourceStatus::Running),
        instance_with_status("worker-dev-1", ResourceStatus::Running),
    ]);
    mark_first_two_visible(&mut app);
    select_command_from_open_palette(&mut app, CommandDescriptor::StopSelectedComputeInstance);
    assert!(app.root.resources.batch_confirmation().is_some());

    app.root.read_only = true;

    let effects = reduce(
        &mut app,
        &Action::Resources(ResourcesAction::ConfirmBatchAction),
    );

    assert!(effects.is_empty(), "no batch may run: {effects:?}");
    assert!(app.root.resources.batch_confirmation().is_none());
    assert!(app.root.system.operations.records().is_empty());
}

/// And on the SSH launch preview, whose confirm spawns an interactive shell on
/// the instance.
#[test]
fn read_only_turned_on_after_the_ssh_preview_opened_refuses_the_launch() {
    let mut app = app_with_selected_instance_status(ResourceStatus::Running);
    select_command_from_open_palette(&mut app, CommandDescriptor::SshSelectedComputeInstance);
    assert!(app.root.resources.ssh_launch_preview().is_some());

    app.root.read_only = true;

    let effects = reduce(
        &mut app,
        &Action::Resources(ResourcesAction::ConfirmSshLaunch),
    );

    assert!(
        effects.is_empty(),
        "no interactive session may be spawned: {effects:?}"
    );
    assert!(app.root.resources.ssh_launch_preview().is_none());
}

/// The mode working as configured is not a failed operation. Rendering a
/// refusal through the error style teaches operators to discount the error
/// style, which is what makes a real failure easy to miss.
#[test]
fn a_read_only_refusal_is_not_reported_as_a_failure() {
    let mut app = app_with_selected_instance_status(ResourceStatus::Running);
    app.root.read_only = true;

    select_command_from_open_palette(&mut app, CommandDescriptor::StopSelectedComputeInstance);

    assert_eq!(
        app.root.system.status_phase,
        Some(OperationPhase::Healthy),
        "a refusal is the mode doing its job, not an operation that failed — \
         pinned to Healthy so that a status left Degraded, Loading, or unset \
         fails here too"
    );
    assert!(app.root.system.status_message.is_some());
}

// --- Secret Manager's palette surface --------------------------------------
//
// The secret-scoped log jump is offered from two different states — the list
// with a row under the cursor, and an open secret's detail pane — and from
// neither of them without a context to resolve the project. Each of those
// conditions is asserted from both sides below, because a guard that is too
// wide offers a jump that resolves to nothing, and one that is too narrow
// silently removes the only way to reach a secret's audit trail.

fn secret(name: &str) -> Secret {
    Secret {
        name: name.to_string(),
        create_time: Some("2026-02-11T09:14:22Z".to_string()),
        labels: Vec::new(),
        replication: "Automatic".to_string(),
    }
}

fn secret_rows() -> Vec<Secret> {
    vec![secret("api-signing-key"), secret("db-password")]
}

fn app_showing_persistent_disks() -> AppState {
    let mut app = AppState::new();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ConfigurationsLoaded {
            configurations: vec![config("dev", true, "dev-project")],
        }),
    );
    let context = app.root.current_context().expect("context exists");
    reduce(&mut app, &Action::Workspace(WorkspaceAction::Resources));
    reduce(
        &mut app,
        &Action::Resources(ResourcesAction::SelectService(
            ResourceServiceKind::PersistentDisks,
        )),
    );
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::PersistentDisksLoaded {
            context,
            disks: vec![PersistentDisk {
                name: "detached-legacy".to_string(),
                location: "europe-west1-b".to_string(),
                status: DiskStatus::Ready,
                size_gb: 20,
                disk_type: "pd-ssd".to_string(),
                users: Vec::new(),
                labels: Vec::new(),
            }],
        }),
    );
    app
}

fn app_showing_bigquery_datasets() -> AppState {
    let mut app = AppState::new();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ConfigurationsLoaded {
            configurations: vec![config("dev", true, "dev-project")],
        }),
    );
    let context = app.root.current_context().expect("context exists");
    reduce(&mut app, &Action::Workspace(WorkspaceAction::Resources));
    reduce(
        &mut app,
        &Action::Resources(ResourcesAction::SelectService(
            ResourceServiceKind::BigQueryDatasets,
        )),
    );
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::BigQueryDatasetsLoaded {
            context,
            datasets: vec![BigQueryDataset {
                dataset_id: "analytics_events".to_string(),
                project_id: "dev-project".to_string(),
                location: "europe-west1".to_string(),
                friendly_name: None,
                labels: Vec::new(),
            }],
        }),
    );
    app
}

fn app_showing_pubsub_subscriptions() -> AppState {
    let mut app = AppState::new();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ConfigurationsLoaded {
            configurations: vec![config("dev", true, "dev-project")],
        }),
    );
    let context = app.root.current_context().expect("context exists");
    reduce(&mut app, &Action::Workspace(WorkspaceAction::Resources));
    reduce(
        &mut app,
        &Action::Resources(ResourcesAction::SelectService(
            ResourceServiceKind::PubSubSubscriptions,
        )),
    );
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::PubSubSubscriptionsLoaded {
            context,
            subscriptions: vec![PubSubSubscription {
                name: "orders-worker".to_string(),
                full_name: "projects/dev-project/subscriptions/orders-worker".to_string(),
                topic: "projects/dev-project/topics/orders".to_string(),
                delivery: PubSubDelivery::Pull,
                ack_deadline_seconds: Some(60),
                message_retention: Some("604800s".to_string()),
                dead_letter: None,
                state: PubSubSubscriptionState::Active,
                labels: Vec::new(),
            }],
        }),
    );
    app
}

fn app_showing_pubsub_topics() -> AppState {
    let mut app = AppState::new();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ConfigurationsLoaded {
            configurations: vec![config("dev", true, "dev-project")],
        }),
    );
    let context = app.root.current_context().expect("context exists");
    reduce(&mut app, &Action::Workspace(WorkspaceAction::Resources));
    reduce(
        &mut app,
        &Action::Resources(ResourcesAction::SelectService(
            ResourceServiceKind::PubSubTopics,
        )),
    );
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::PubSubTopicsLoaded {
            context,
            topics: vec![PubSubTopic {
                name: "orders".to_string(),
                full_name: "projects/dev-project/topics/orders".to_string(),
                subscription_count: SubscriptionCount::Known(2),
                message_retention: Some("604800s".to_string()),
                kms_key: None,
                labels: Vec::new(),
            }],
        }),
    );
    app
}

fn app_showing_vpc_networks() -> AppState {
    let mut app = AppState::new();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ConfigurationsLoaded {
            configurations: vec![config("dev", true, "dev-project")],
        }),
    );
    let context = app.root.current_context().expect("context exists");
    reduce(&mut app, &Action::Workspace(WorkspaceAction::Resources));
    reduce(
        &mut app,
        &Action::Resources(ResourcesAction::SelectService(
            ResourceServiceKind::VpcNetworks,
        )),
    );
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::VpcNetworksLoaded {
            context,
            networks: vec![VpcNetwork {
                name: "default".to_string(),
                subnet_mode: VpcSubnetMode::Auto,
                routing_mode: VpcRoutingMode::Regional,
                subnet_count: 43,
                description: None,
            }],
        }),
    );
    app
}

/// One INGRESS ALLOW rule, which is the shape every rule in the fixture
/// project has. `disabled: false` matters: a disabled rule's identity cell
/// carries the `DISABLED` badge, so a disabled row here would change what the
/// palette walk is matching against.
fn app_showing_firewall_rules() -> AppState {
    let mut app = AppState::new();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ConfigurationsLoaded {
            configurations: vec![config("dev", true, "dev-project")],
        }),
    );
    let context = app.root.current_context().expect("context exists");
    reduce(&mut app, &Action::Workspace(WorkspaceAction::Resources));
    reduce(
        &mut app,
        &Action::Resources(ResourcesAction::SelectService(
            ResourceServiceKind::FirewallRules,
        )),
    );
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::FirewallRulesLoaded {
            context,
            rules: vec![FirewallRule {
                name: "default-allow-icmp".to_string(),
                network: "default".to_string(),
                direction: FirewallDirection::Ingress,
                priority: Some(65534),
                action: FirewallAction::Allow,
                targets: FirewallTargets::AllInstances,
                source_or_dest: vec!["0.0.0.0/0".to_string()],
                disabled: false,
            }],
        }),
    );
    app
}

fn app_showing_vpc_subnets() -> AppState {
    let mut app = AppState::new();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ConfigurationsLoaded {
            configurations: vec![config("dev", true, "dev-project")],
        }),
    );
    let context = app.root.current_context().expect("context exists");
    reduce(&mut app, &Action::Workspace(WorkspaceAction::Resources));
    reduce(
        &mut app,
        &Action::Resources(ResourcesAction::SelectService(
            ResourceServiceKind::VpcSubnets,
        )),
    );
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::VpcSubnetsLoaded {
            context,
            subnets: vec![VpcSubnet {
                name: "default".to_string(),
                region: "us-central1".to_string(),
                network: "default".to_string(),
                ip_cidr_range: "10.128.0.0/20".to_string(),
                gateway_address: Some("10.128.0.1".to_string()),
                private_google_access: false,
            }],
        }),
    );
    app
}

fn app_showing_service_accounts() -> AppState {
    let mut app = AppState::new();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ConfigurationsLoaded {
            configurations: vec![config("dev", true, "dev-project")],
        }),
    );
    let context = app.root.current_context().expect("context exists");
    reduce(&mut app, &Action::Workspace(WorkspaceAction::Resources));
    reduce(
        &mut app,
        &Action::Resources(ResourcesAction::SelectService(
            ResourceServiceKind::IamServiceAccounts,
        )),
    );
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ServiceAccountsLoaded {
            context,
            accounts: vec![ServiceAccount {
                email: "deployer@dev-project.iam.gserviceaccount.com".to_string(),
                display_name: Some("Deployer".to_string()),
                disabled: false,
                unique_id: "100000000000000000001".to_string(),
            }],
        }),
    );
    app
}

fn app_showing_policy_bindings() -> AppState {
    let mut app = AppState::new();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ConfigurationsLoaded {
            configurations: vec![config("dev", true, "dev-project")],
        }),
    );
    let context = app.root.current_context().expect("context exists");
    reduce(&mut app, &Action::Workspace(WorkspaceAction::Resources));
    reduce(
        &mut app,
        &Action::Resources(ResourcesAction::SelectService(
            ResourceServiceKind::IamPolicy,
        )),
    );
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::IamPolicyLoaded {
            context,
            members: vec![IamPolicyMember::from_wire(
                "roles/artifactregistry.admin",
                "user:alice@example.com",
                None,
            )],
        }),
    );
    app
}

fn app_showing_cloud_functions() -> AppState {
    let mut app = AppState::new();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ConfigurationsLoaded {
            configurations: vec![config("dev", true, "dev-project")],
        }),
    );
    let context = app.root.current_context().expect("context exists");
    reduce(&mut app, &Action::Workspace(WorkspaceAction::Resources));
    reduce(
        &mut app,
        &Action::Resources(ResourcesAction::SelectService(
            ResourceServiceKind::CloudFunctions,
        )),
    );
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::CloudFunctionsLoaded {
            context,
            functions: vec![CloudFunction {
                name: "thumbnailer".to_string(),
                region: "europe-west1".to_string(),
                state: CloudFunctionState::Active,
                environment: CloudFunctionEnvironment::Gen2,
                url: Some("https://europe-west1-dev.cloudfunctions.net/thumbnailer".to_string()),
                update_time: Some("2026-03-01T10:00:00Z".to_string()),
                labels: Vec::new(),
            }],
        }),
    );
    app
}

/// With no function under the cursor the palette must not offer the jump.
///
/// The guard is a conjunction, and the mutation gate found that loosening it
/// changed nothing any test asserted: an entry offered with nothing selected
/// resolves to the "No Cloud Function selected to open logs for" refusal, so
/// the palette would advertise a command that can only fail.
#[test]
fn the_function_log_jump_is_not_offered_with_an_empty_functions_list() {
    let mut app = AppState::new();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ConfigurationsLoaded {
            configurations: vec![config("dev", true, "dev-project")],
        }),
    );
    reduce(&mut app, &Action::Workspace(WorkspaceAction::Resources));
    reduce(
        &mut app,
        &Action::Resources(ResourcesAction::SelectService(
            ResourceServiceKind::CloudFunctions,
        )),
    );
    assert!(app.root.current_context().is_some());
    assert!(
        app.root.resources.selected_cloud_function().is_none(),
        "the list must genuinely be empty, or this asserts nothing"
    );

    reduce(&mut app, &Action::Command(CommandAction::Open));

    assert!(
        !app.root.command.visible_commands(&app.root).contains(
            &CommandDescriptor::OpenLogsForSelected(ResourceServiceKind::CloudFunctions)
        ),
        "the palette offers a log jump with no function selected"
    );
}

/// The jump belongs to the service on screen, not to whichever service last
/// loaded rows.
///
/// Also a mutation finding: loosening the arm's first conjunction let the entry
/// be offered from another service's list entirely, where `L` names the
/// selection of the service the operator is *not* looking at.
#[test]
fn the_function_log_jump_is_not_offered_from_another_services_list() {
    let mut app = app_showing_cloud_functions();
    assert!(app.root.resources.selected_cloud_function().is_some());
    reduce(
        &mut app,
        &Action::Resources(ResourcesAction::SelectService(
            ResourceServiceKind::GkeClusters,
        )),
    );
    assert!(
        app.root.resources.selected_gke_cluster().is_none(),
        "GKE must have no selection of its own, or its own arm answers first"
    );

    reduce(&mut app, &Action::Command(CommandAction::Open));

    assert!(
        !app.root.command.visible_commands(&app.root).contains(
            &CommandDescriptor::OpenLogsForSelected(ResourceServiceKind::CloudFunctions)
        ),
        "the palette offers the Cloud Functions jump while GKE is on screen"
    );
}

fn app_showing_secrets() -> AppState {
    let mut app = AppState::new();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ConfigurationsLoaded {
            configurations: vec![config("dev", true, "dev-project")],
        }),
    );
    let context = app.root.current_context().expect("context exists");
    reduce(&mut app, &Action::Workspace(WorkspaceAction::Resources));
    reduce(
        &mut app,
        &Action::Resources(ResourcesAction::SelectService(
            ResourceServiceKind::SecretManagerSecrets,
        )),
    );
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::SecretsLoaded {
            context,
            secrets: secret_rows(),
        }),
    );
    app
}

fn storage_bucket(name: &str) -> StorageBucket {
    StorageBucket {
        name: name.to_string(),
        location: "EU".to_string(),
        location_type: Some("multi-region".to_string()),
        storage_class: "STANDARD".to_string(),
        versioning_enabled: false,
        uniform_bucket_level_access: true,
        public_access_prevention: Some("enforced".to_string()),
        time_created: None,
        updated: Some("2026-01-01T00:00:00Z".to_string()),
    }
}

fn app_with_selected_storage_bucket() -> AppState {
    let mut app = AppState::new();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ConfigurationsLoaded {
            configurations: vec![config("dev", true, "dev-project")],
        }),
    );
    reduce(&mut app, &Action::Workspace(WorkspaceAction::Resources));
    reduce(
        &mut app,
        &Action::Resources(ResourcesAction::SelectService(
            ResourceServiceKind::CloudStorage,
        )),
    );
    let context = app.root.current_context().expect("context exists");
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::StorageBucketsLoaded {
            context,
            buckets: vec![storage_bucket("assets-bucket")],
        }),
    );
    app
}

/// The state each service is in when its log jump should be on offer: the
/// service active in the Resources workspace, a context resolved, and a row
/// under the cursor.
///
/// The match is exhaustive on purpose. A service added to
/// `ResourceServiceKind::ALL` cannot compile here until someone says what
/// "this service on screen with a row selected" means for it, and the walk
/// below then holds the palette to offering the log entry from that state.
fn app_showing_service(kind: ResourceServiceKind) -> AppState {
    match kind {
        ResourceServiceKind::ComputeInstances => app_with_selected_instance(),
        ResourceServiceKind::GkeClusters => app_with_selected_gke_cluster(),
        ResourceServiceKind::CloudSql => {
            app_with_selected_cloud_sql(CloudSqlAvailability::Zonal, CloudSqlStatus::Runnable)
        }
        ResourceServiceKind::CloudRunServices => app_with_selected_cloud_run_service(),
        ResourceServiceKind::CloudStorage => app_with_selected_storage_bucket(),
        ResourceServiceKind::SecretManagerSecrets => app_showing_secrets(),
        ResourceServiceKind::CloudFunctions => app_showing_cloud_functions(),
        ResourceServiceKind::PersistentDisks => app_showing_persistent_disks(),
        ResourceServiceKind::PubSubTopics => app_showing_pubsub_topics(),
        ResourceServiceKind::PubSubSubscriptions => app_showing_pubsub_subscriptions(),
        ResourceServiceKind::BigQueryDatasets => app_showing_bigquery_datasets(),
        ResourceServiceKind::IamServiceAccounts => app_showing_service_accounts(),
        ResourceServiceKind::IamPolicy => app_showing_policy_bindings(),
        ResourceServiceKind::VpcNetworks => app_showing_vpc_networks(),
        ResourceServiceKind::VpcSubnets => app_showing_vpc_subnets(),
        ResourceServiceKind::FirewallRules => app_showing_firewall_rules(),
    }
}

/// The services that have logs to jump to.
///
/// A persistent disk is not one: nothing writes logs *as* a disk, so a
/// resource-scoped query for one could only ever return zero rows — which on
/// screen reads as "this disk has no logs" rather than as "disks do not have
/// logs". The walk below asserts the *absence* of the entry for these, so a
/// service wired to an empty query fails rather than passing by omission.
fn has_logs(kind: ResourceServiceKind) -> bool {
    match kind {
        ResourceServiceKind::ComputeInstances
        | ResourceServiceKind::GkeClusters
        | ResourceServiceKind::CloudSql
        | ResourceServiceKind::CloudRunServices
        | ResourceServiceKind::CloudStorage
        | ResourceServiceKind::SecretManagerSecrets
        | ResourceServiceKind::CloudFunctions
        | ResourceServiceKind::PubSubTopics
        | ResourceServiceKind::PubSubSubscriptions
        | ResourceServiceKind::BigQueryDatasets
        // The one VPC table with logs: flow logs and firewall hit logs are
        // both attributed to `gce_subnetwork`.
        | ResourceServiceKind::VpcSubnets => true,
        // A service account is an identity that appears *in* log entries as
        // the caller, not a resource with a log stream of its own — the same
        // refusal as disks, for the same "zero rows would read as an answer"
        // reason.
        // A policy binding is a grant rather than a resource, so nothing
        // writes log entries as one — the same refusal.
        // A VPC network is the same refusal one step further: flow logs and
        // firewall hit logs exist, and Cloud Logging attributes both to the
        // *subnetwork*, so a network-scoped query returns zero rows while
        // looking like it should return many.
        // A firewall rule is the same refusal again, and the one where the
        // empty answer would be most misread: a rule's hits *are* logged, but
        // against the subnetwork the traffic crossed, with the rule named in
        // the payload. Zero rows on a DENY rule reads as "nothing was
        // blocked".
        ResourceServiceKind::PersistentDisks
        | ResourceServiceKind::IamServiceAccounts
        | ResourceServiceKind::IamPolicy
        | ResourceServiceKind::VpcNetworks
        | ResourceServiceKind::FirewallRules => false,
    }
}

/// Every service offers its log jump from the palette an operator opens.
///
/// The descriptor existing is not the same property: `CommandDescriptor::all`
/// is generated from `ResourceServiceKind::ALL`, while what the palette lists
/// comes from `context_commands`, a hand-written per-service match ending in a
/// catch-all that yields nothing. A service can therefore have its descriptor
/// generated and still show nothing when it is the active service — which is
/// the known symptom (the `L` keybinding worked, the palette had no entry)
/// one layer down. This drives the real palette so the omission fails here.
#[test]
fn every_service_offers_its_log_jump_from_the_open_palette() {
    for kind in ResourceServiceKind::ALL {
        let mut app = app_showing_service(kind);
        assert_eq!(
            app.root.resources.active_service(),
            kind,
            "{kind:?} must be the active service for this walk to say anything"
        );
        assert_eq!(app.root.workspace, Workspace::Resources);
        assert!(app.root.current_context().is_some());

        reduce(&mut app, &Action::Command(CommandAction::Open));
        let offered = app
            .root
            .command
            .visible_commands(&app.root)
            .contains(&CommandDescriptor::OpenLogsForSelected(kind));
        if has_logs(kind) {
            assert!(
                offered,
                "the palette offers no log jump for {kind:?} with a row selected, \
                 leaving the L keybinding as the only route to its logs"
            );
        } else {
            assert!(
                !offered,
                "{kind:?} produces no logs, but the palette offers a log jump \
                 for it — the query behind it can only ever return zero rows, \
                 which reads as an empty log rather than as a missing feature"
            );
        }
    }
}

/// The BigQuery arm's guard is a conjunction, and each half is load-bearing.
///
/// Both `&&` in it survived being replaced by `||` with the whole suite green:
/// the walk above only ever drives the state where all three conjuncts hold,
/// so a disjunction reproduces it exactly. What a widened guard costs is a
/// palette entry offered with nothing to open — `OpenLogsForSelected` resolves
/// against the selected dataset, so an entry with no selection, or one shown
/// while another service is on screen, is a row that either does nothing or
/// queries the logs of a resource the operator is not looking at.
///
/// Two tests rather than one, so each half is measured on its own: a single
/// test aborts on its first assertion, which would leave the second mutant
/// killed by the first case and the service half unpinned again.
#[test]
fn the_bigquery_log_jump_needs_a_selected_dataset() {
    let jump = CommandDescriptor::OpenLogsForSelected(ResourceServiceKind::BigQueryDatasets);

    // No listing has arrived, so there is no dataset to scope a query to.
    let mut empty = AppState::new();
    reduce(
        &mut empty,
        &Action::EffectResult(EffectResult::ConfigurationsLoaded {
            configurations: vec![config("dev", true, "dev-project")],
        }),
    );
    reduce(&mut empty, &Action::Workspace(WorkspaceAction::Resources));
    reduce(
        &mut empty,
        &Action::Resources(ResourcesAction::SelectService(
            ResourceServiceKind::BigQueryDatasets,
        )),
    );
    assert!(
        empty.root.current_context().is_some(),
        "the context half of the guard must hold, or this case says nothing \
         about the selection half"
    );
    assert!(
        empty.root.resources.selected_bigquery_dataset().is_none(),
        "this case is about an empty listing; a selected dataset would make it \
         the same state the walk above already drives"
    );
    reduce(&mut empty, &Action::Command(CommandAction::Open));
    assert!(
        !empty
            .root
            .command
            .visible_commands(&empty.root)
            .contains(&jump),
        "the palette offers the dataset log jump with no dataset selected: the \
         entry resolves against a selection that is not there"
    );
}

/// The other half of the same guard: see
/// [`the_bigquery_log_jump_needs_a_selected_dataset`].
#[test]
fn the_bigquery_log_jump_is_not_offered_from_another_service() {
    let jump = CommandDescriptor::OpenLogsForSelected(ResourceServiceKind::BigQueryDatasets);

    // A dataset is selected, but the operator has moved to another service —
    // one with no log jump of its own, so anything offered here came from the
    // BigQuery arm.
    let mut elsewhere = app_showing_bigquery_datasets();
    reduce(
        &mut elsewhere,
        &Action::Resources(ResourcesAction::SelectService(
            ResourceServiceKind::PersistentDisks,
        )),
    );
    assert!(
        elsewhere
            .root
            .resources
            .selected_bigquery_dataset()
            .is_some(),
        "the selection half of the guard must still hold, or this case says \
         nothing about the service half"
    );
    assert_eq!(
        elsewhere.root.resources.active_service(),
        ResourceServiceKind::PersistentDisks
    );
    reduce(&mut elsewhere, &Action::Command(CommandAction::Open));
    assert!(
        !elsewhere
            .root
            .command
            .visible_commands(&elsewhere.root)
            .contains(&jump),
        "the palette offers the BigQuery log jump while another service is on \
         screen: the entry would query the logs of a resource the operator is \
         not looking at"
    );
}

/// One `{name}`-only plugin scoped to `kind`, loaded through the real config
/// parser over a real file — the only way a `Plugin` is ever built.
///
/// The scope is spelled with the service registry's own label rather than a
/// literal per kind, so a service added to `ResourceServiceKind::ALL` needs no
/// second table here for the walk below to cover it. The assertions are what
/// make the walk non-vacuous: a spelling the parser does not accept, or a file
/// this build rejects, would otherwise leave every service asserting about an
/// empty plugin set.
fn plugin_scoped_to(kind: ResourceServiceKind) -> Plugins {
    let scope = descriptor(
        service_id_for_resource(kind).expect("every resource service has a registry entry"),
    )
    .label;
    let body = format!(
        "version = 1\n\n[[plugins]]\nid = \"describe\"\nscope = {scope:?}\n\
         key = \"d\"\nlabel = \"Describe {{name}}\"\nargs = [\"describe\", \"{{name}}\"]\n"
    );
    let mut file = NamedTempFile::new().expect("a temp plugin file");
    file.write_all(body.as_bytes()).expect("the fixture writes");

    let mut problems = ConfigProblems::default();
    let plugins = Plugins::load(file.path(), &mut problems);
    assert!(
        problems.is_empty(),
        "{kind:?}: fixture must be a valid plugin file: {:?}",
        problems.messages()
    );
    assert_eq!(plugins.len(), 1, "{kind:?}: the fixture plugin must load");
    plugins
}

/// Every service offers its plugins from the palette an operator opens, not
/// merely from `context_commands`.
///
/// `context_commands` returns early per workspace and per service, and several
/// services have no arm of their own at all, so "the plugin resolves" and "the
/// palette lists it" are different properties. This drives the real palette,
/// which is the layer where a per-service guard can swallow the entry — a
/// plugin scoped to a service that never appears is the failure this whole
/// feature is for.
#[test]
fn every_service_offers_its_plugin_from_the_open_palette() {
    for kind in ResourceServiceKind::ALL {
        let mut app = app_showing_service(kind);
        assert_eq!(
            app.root.resources.active_service(),
            kind,
            "{kind:?} must be the active service for this walk to say anything"
        );
        app.root.plugins = plugin_scoped_to(kind);

        reduce(&mut app, &Action::Command(CommandAction::Open));
        let visible = app.root.command.visible_commands(&app.root);

        let offered = visible
            .iter()
            .find(|command| matches!(command, CommandDescriptor::Plugin(_)))
            .unwrap_or_else(|| {
                panic!("{kind:?} offered no plugin from the open palette: {visible:?}")
            });
        // The row's text, not just its presence: a row that read "Plugin
        // command" would be an entry the operator cannot tell from any other
        // plugin's.
        let label = palette_display_label(*offered, &app.root);
        assert!(
            label.starts_with("Describe "),
            "{kind:?} rendered its plugin row as {label:?}"
        );
    }
}

/// One compute-scoped plugin with the given `args`, loaded through the real
/// config parser over a real file — the only way a `Plugin` is ever built, so
/// the argv these tests preview is one the config layer would actually hand
/// over.
fn compute_plugin_with_args(args: &[&str]) -> Plugins {
    compute_plugin_named("describe", "Describe {name}", args)
}

/// The same fixture with the declaration's own naming spelled out, for the
/// tests whose subject is what the argv *is*: `--read-only` now decides from
/// the command path, so a plugin that must be refused has to declare a verb
/// nuvem never vouched for — and a row labelled "Describe" that deletes would
/// leave the refusal sentence describing a plugin the fixture does not hold.
fn compute_plugin_named(id: &str, label: &str, args: &[&str]) -> Plugins {
    let scope = descriptor(
        service_id_for_resource(ResourceServiceKind::ComputeInstances)
            .expect("Compute has a registry entry"),
    )
    .label;
    let body = format!(
        "version = 1\n\n[[plugins]]\nid = {id:?}\nscope = {scope:?}\n\
         key = \"d\"\nlabel = {label:?}\nargs = {args:?}\n"
    );
    let mut file = NamedTempFile::new().expect("a temp plugin file");
    file.write_all(body.as_bytes()).expect("the fixture writes");

    let mut problems = ConfigProblems::default();
    let plugins = Plugins::load(file.path(), &mut problems);
    assert!(
        problems.is_empty(),
        "fixture must be a valid plugin file: {:?}",
        problems.messages()
    );
    assert_eq!(plugins.len(), 1, "the fixture plugin must load");
    plugins
}

/// Two compute-scoped plugins, behind a Cloud SQL one, in file order and
/// loaded through the real config parser over a real file.
///
/// Each compute entry carries its own label and its own argv, so an assertion
/// about the second cannot pass by reading the first. The Cloud SQL entry is
/// what makes the file indices (1 and 2) differ from the palette's own row
/// numbering of the plugins it lists (0 and 1) — the confusion
/// `app::state::plugins::PluginId` documents.
fn two_compute_plugins_behind_a_cloud_sql_one() -> Plugins {
    let compute = descriptor(
        service_id_for_resource(ResourceServiceKind::ComputeInstances)
            .expect("Compute has a registry entry"),
    )
    .label;
    let cloud_sql = descriptor(
        service_id_for_resource(ResourceServiceKind::CloudSql)
            .expect("Cloud SQL has a registry entry"),
    )
    .label;
    let body = format!(
        "version = 1\n\n\
         [[plugins]]\nid = \"sql-describe\"\nscope = {cloud_sql:?}\nkey = \"s\"\n\
         label = \"Describe {{name}}\"\n\
         args = [\"sql\", \"instances\", \"describe\", \"{{name}}\"]\n\n\
         [[plugins]]\nid = \"describe\"\nscope = {compute:?}\nkey = \"d\"\n\
         label = \"Describe {{name}}\"\n\
         args = [\"compute\", \"instances\", \"describe\", \"{{name}}\"]\n\n\
         [[plugins]]\nid = \"disks\"\nscope = {compute:?}\nkey = \"k\"\n\
         label = \"List disks of {{name}}\"\n\
         args = [\"compute\", \"disks\", \"list\", \"--filter=users:{{name}}\"]\n"
    );
    let mut file = NamedTempFile::new().expect("a temp plugin file");
    file.write_all(body.as_bytes()).expect("the fixture writes");

    let mut problems = ConfigProblems::default();
    let plugins = Plugins::load(file.path(), &mut problems);
    assert!(
        problems.is_empty(),
        "fixture must be a valid plugin file: {:?}",
        problems.messages()
    );
    assert_eq!(plugins.len(), 3, "every fixture plugin must load");
    plugins
}

/// The palette row carries the plugin's position in the *file*, so submitting
/// the second compute row confirms the second compute plugin's command.
///
/// Every other end-to-end plugin test here stops at the first plugin row, so
/// all of them assert about `PluginId(0)` — the one index a descriptor keyed
/// off palette row order would also get right. This one reaches the row after
/// it, over a file whose Cloud SQL entry makes the two orders disagree: keyed
/// off the row, the descriptor would name the compute *describe* plugin and
/// offer its argv against a row the operator asked to list disks for.
#[test]
fn submitting_the_second_plugin_row_confirms_the_second_plugins_command() {
    let mut app = app_with_one_instance("web-01", "us-central1-a");
    app.root.plugins = two_compute_plugins_behind_a_cloud_sql_one();
    reduce(&mut app, &Action::Command(CommandAction::Open));
    select_the_nth_plugin_row(&mut app, 1);

    // The row the operator is about to submit already names the second plugin.
    let selected = app
        .root
        .command
        .selected_visible_command(&app.root)
        .expect("a row is selected");
    assert_eq!(
        selected,
        CommandDescriptor::Plugin(PluginId(2)),
        "the row must carry the plugin's file index, not its position among \
         the rows the palette lists"
    );
    assert_eq!(
        palette_display_label(selected, &app.root),
        "List disks of web-01"
    );

    reduce(&mut app, &Action::Command(CommandAction::Submit));

    let confirmation = app
        .root
        .resources
        .plugin_confirmation()
        .expect("the modal opened");
    assert_eq!(confirmation.label(), "List disks of web-01");
    assert_eq!(
        confirmation.command_preview(),
        "gcloud --configuration dev compute disks list --filter=users:web-01"
    );
}

/// The Compute list on screen with exactly one instance under the cursor.
///
/// Named and zoned by the caller, because the argv a plugin composes is built
/// from those two values and one of these tests needs a name that has to be
/// quoted.
fn app_with_one_instance(name: &str, zone: &str) -> AppState {
    let mut app = AppState::new();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ConfigurationsLoaded {
            configurations: vec![config("dev", true, "dev-project")],
        }),
    );
    let context = app.root.current_context().expect("context exists");
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ComputeInstancesLoaded {
            context,
            instances: vec![ComputeInstance {
                zone: zone.to_string(),
                ..instance(name)
            }],
        }),
    );
    reduce(&mut app, &Action::Workspace(WorkspaceAction::Resources));
    assert!(
        app.root.resources.selected_visible_instance().is_some(),
        "the fixture must have a row under the cursor for a plugin to resolve against"
    );
    app
}

/// Moves the palette cursor onto the plugin row, whatever position the built-in
/// commands leave it in. Selecting by index would silently start asserting
/// about a different command the next time a built-in is added.
fn select_the_plugin_row(app: &mut AppState) {
    select_the_nth_plugin_row(app, 0);
}

/// Moves the palette cursor onto the `nth` plugin row (0-based), counted in the
/// order the palette lists them.
///
/// Every other test here stops at the first plugin row, so every end-to-end
/// assertion about a plugin runs against `PluginId(0)` — the one index that is
/// right whether the descriptor is keyed off file order or off palette row
/// order. Reaching a later row is what tells the two apart.
fn select_the_nth_plugin_row(app: &mut AppState, nth: usize) {
    let rows = app.root.command.visible_commands(&app.root).len();
    let mut seen = 0;
    for _ in 0..rows {
        if matches!(
            app.root.command.selected_visible_command(&app.root),
            Some(CommandDescriptor::Plugin(_))
        ) {
            if seen == nth {
                return;
            }
            seen += 1;
        }
        reduce(app, &Action::Command(CommandAction::SelectNext));
    }
    panic!(
        "the open palette offered fewer than {} plugin rows: {:?}",
        nth + 1,
        app.root.command.visible_commands(&app.root)
    );
}

/// Submitting a plugin opens its confirmation with the exact argv, rendered by
/// the one renderer every other command line goes through.
#[test]
fn submitting_a_plugin_opens_a_confirmation_of_its_exact_argv() {
    let mut app = app_with_one_instance("web-01", "us-central1-a");
    app.root.plugins = compute_plugin_with_args(&[
        "compute",
        "instances",
        "describe",
        "{name}",
        "--zone",
        "{zone}",
    ]);
    reduce(&mut app, &Action::Command(CommandAction::Open));
    select_the_plugin_row(&mut app);

    let effects = reduce(&mut app, &Action::Command(CommandAction::Submit));

    let confirmation = app
        .root
        .resources
        .plugin_confirmation()
        .expect("the modal opened");
    assert_eq!(confirmation.label(), "Describe web-01");
    assert_eq!(
        confirmation.command_preview(),
        "gcloud --configuration dev compute instances describe web-01 --zone us-central1-a"
    );
    assert!(
        app.root.resources.command_preview().is_none(),
        "a plugin must not also open the copy-only preview"
    );
    assert!(!app.root.command.is_open(), "the palette closes on submit");
    // Submitting is the question, not the answer: opening the confirmation
    // emits no effect at all, so nothing runs until the operator answers it.
    assert!(effects.is_empty(), "submitting a plugin ran {effects:?}");
}

/// A value that would need quoting to be read correctly is quoted, because the
/// line the operator reads before answering is the entire safety argument for
/// this feature. The shared renderer was made faithful; this is the first
/// caller that can produce such a value.
#[test]
fn a_plugin_confirmation_quotes_a_value_containing_a_space() {
    let mut app = app_with_one_instance("web 01", "us-central1-a");
    app.root.plugins = compute_plugin_with_args(&["compute", "instances", "describe", "{name}"]);
    reduce(&mut app, &Action::Command(CommandAction::Open));
    select_the_plugin_row(&mut app);

    reduce(&mut app, &Action::Command(CommandAction::Submit));

    assert_eq!(
        app.root
            .resources
            .plugin_confirmation()
            .expect("the modal opened")
            .command_preview(),
        "gcloud --configuration dev compute instances describe 'web 01'"
    );
}

/// Read-only refuses a plugin nuvem cannot prove only reads, before any modal
/// opens. The argv is a `delete` rather than the file's usual `describe`: this
/// path used to refuse every plugin unconditionally, and a `describe` fixture
/// now runs here — which is the point of the read-only rule, and would make this test pass
/// for no reason at all.
#[test]
fn read_only_refuses_a_plugin_before_its_confirmation_opens() {
    let mut app = app_with_one_instance("web-01", "us-central1-a");
    app.root.read_only = true;
    app.root.plugins = compute_plugin_named(
        "delete",
        "Delete {name}",
        &["compute", "instances", "delete", "{name}"],
    );
    reduce(&mut app, &Action::Command(CommandAction::Open));
    select_the_plugin_row(&mut app);

    reduce(&mut app, &Action::Command(CommandAction::Submit));

    assert!(app.root.resources.plugin_confirmation().is_none());
    assert!(
        app.root.resources.command_preview().is_none(),
        "the refusal must not fall back to the copy-only preview either"
    );
    assert_eq!(
        app.root.system.status_message.as_deref(),
        Some("Delete web-01 is unavailable: read-only mode cannot prove this only reads"),
        "the refusal must name the plugin the way its palette row did"
    );
}

/// The clipboard path has nothing of a plugin's left to copy.
///
/// Submitting a plugin used to open the shared `ResourceCommandPreview`, whose
/// `Enter` is `CopyPendingCommand`, and that copy was the whole of what a
/// plugin did. It opens its own confirmation now, so both halves of that move
/// are pinned here: the clipboard action finds nothing to write, and it does
/// not reach into the confirmation the operator has not answered yet — the two
/// modals are separate slots, and a copy that emptied the pending one would
/// discard the question.
#[test]
fn the_clipboard_path_has_nothing_of_a_plugins_to_copy() {
    let mut app = app_with_one_instance("web-01", "us-central1-a");
    app.root.plugins = compute_plugin_with_args(&["compute", "instances", "describe", "{name}"]);
    reduce(&mut app, &Action::Command(CommandAction::Open));
    select_the_plugin_row(&mut app);
    reduce(&mut app, &Action::Command(CommandAction::Submit));

    let effects = reduce(
        &mut app,
        &Action::Resources(ResourcesAction::CopyPendingCommand),
    );

    assert!(
        effects.is_empty(),
        "a plugin reached the clipboard: {effects:?}"
    );
    assert!(
        app.root.resources.plugin_confirmation().is_some(),
        "the copy action consumed the confirmation the operator still owes an \
         answer to"
    );
}

/// Typing `logs` with a secret under the cursor selects the secret jump.
///
/// The end-to-end half of the alias assertion: rank is computed over aliases,
/// so an aliasless command cannot be first for this query.
#[test]
fn typing_logs_over_a_secret_selects_the_secret_log_jump() {
    let mut app = app_showing_secrets();
    reduce(&mut app, &Action::Command(CommandAction::Open));
    for ch in "logs".chars() {
        reduce(&mut app, &Action::Command(CommandAction::AppendChar(ch)));
    }

    assert_eq!(
        app.root.command.selected_visible_command(&app.root),
        Some(CommandDescriptor::OpenLogsForSelected(
            ResourceServiceKind::SecretManagerSecrets
        ))
    );
}

/// Both states that carry a secret offer the jump — the list and the detail.
///
/// In the list there is no detail target, and in the detail pane the target is
/// what the jump resolves against; the resolver accepts either, so the command
/// must be offered from either. A guard that demanded both would hide it in the
/// list, where operators reach for it most.
#[test]
fn secret_log_jump_is_offered_from_the_secrets_list_and_from_an_open_secret() {
    let mut app = app_showing_secrets();
    assert!(
        app.root.resources.secret_detail_target().is_none(),
        "the list state must genuinely have no detail target, or this asserts \
         nothing about the list half of the guard"
    );
    assert!(app.root.resources.selected_secret().is_some());

    reduce(&mut app, &Action::Command(CommandAction::Open));
    assert!(
        app.root.command.visible_commands(&app.root).contains(
            &CommandDescriptor::OpenLogsForSelected(ResourceServiceKind::SecretManagerSecrets)
        ),
        "the selected row alone must offer the jump"
    );
    reduce(&mut app, &Action::Command(CommandAction::Cancel));

    reduce(&mut app, &Action::Resources(ResourcesAction::OpenSelected));
    assert!(
        app.root.resources.secret_detail_target().is_some(),
        "opening a secret must set the detail target this half depends on"
    );
    reduce(&mut app, &Action::Command(CommandAction::Open));
    assert!(
        app.root.command.visible_commands(&app.root).contains(
            &CommandDescriptor::OpenLogsForSelected(ResourceServiceKind::SecretManagerSecrets)
        ),
        "the open secret must offer the jump too"
    );
}

/// Without a context the jump is not offered, even with a secret selected.
///
/// The audit-log query is scoped by project, which only the active context
/// supplies. Offering the command here would open a log view that can never
/// resolve, and the operator would read the empty result as "nothing touched
/// this secret".
#[test]
fn secret_log_jump_is_withheld_when_no_context_resolves() {
    let mut app = AppState::new();
    reduce(&mut app, &Action::Workspace(WorkspaceAction::Resources));
    reduce(
        &mut app,
        &Action::Resources(ResourcesAction::SelectService(
            ResourceServiceKind::SecretManagerSecrets,
        )),
    );
    // Rows that outlived the context that fetched them: installed directly,
    // because no reachable action loads rows while `current_context()` is None.
    app.root.resources.set_secrets(
        GcloudContext {
            configuration: GcloudConfigurationName::new("dev"),
            account: None,
            project: Some("dev-project".to_string()),
            region: None,
            zone: None,
        },
        secret_rows(),
    );
    assert!(app.root.current_context().is_none());
    assert!(
        app.root.resources.selected_secret().is_some(),
        "the missing context must be the only reason the jump is withheld"
    );

    reduce(&mut app, &Action::Command(CommandAction::Open));

    assert!(
        !app.root.command.visible_commands(&app.root).contains(
            &CommandDescriptor::OpenLogsForSelected(ResourceServiceKind::SecretManagerSecrets)
        ),
        "a jump that cannot name a project must not be offered"
    );
}

/// Another service's list does not offer the secret jump.
///
/// The secrets rows survive a service switch, so "a secret is selected" stays
/// true while the operator is looking at Cloud Run. The active service is what
/// decides whose row the palette is acting on.
#[test]
fn secret_log_jump_is_withheld_while_another_service_is_active() {
    let mut app = app_showing_secrets();
    reduce(
        &mut app,
        &Action::Resources(ResourcesAction::SelectService(
            ResourceServiceKind::CloudRunServices,
        )),
    );
    assert_eq!(
        app.root.resources.active_service(),
        ResourceServiceKind::CloudRunServices
    );
    assert!(
        app.root.resources.selected_secret().is_some(),
        "the retained secrets selection is exactly what a service-blind guard \
         would act on"
    );

    reduce(&mut app, &Action::Command(CommandAction::Open));

    assert!(
        !app.root.command.visible_commands(&app.root).contains(
            &CommandDescriptor::OpenLogsForSelected(ResourceServiceKind::SecretManagerSecrets)
        ),
        "the Cloud Run list must not offer a jump scoped to a secret"
    );
}
