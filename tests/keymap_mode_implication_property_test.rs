//! Property test for the keymap's mode-implies-its-flag invariant.
//!
//! `action_for_key` dispatches on `context.mode`, and its arms used to restate
//! conditions the mode already implies — `Mode::CommandPalette if
//! context.command_palette_open`, or a `workspace ==` beside a flag that is
//! itself defined as `workspace == W && ...`. Every such expression is dead,
//! because `services::runtime::input_context` assigns each mode in exactly one
//! place and builds each scoped flag with its workspace already folded in.
//! The first one was found the hard way: the mutation gate flagged
//! `Mode::PluginConfirmation`'s guard as an equivalent mutant, no test could
//! kill it, and the only route to green was deleting the expression. A later
//! sweep took the rest — eight whole guards, and six redundant conjuncts inside the
//! two guards that stayed.
//!
//! Deleting them is safe only while those implications hold, and nothing in the
//! type system says they do — they are properties of one if/else ladder, which
//! any later branch can break by assigning a mode a second time or by dropping
//! a workspace conjunct from a flag. So this test states them once and lets
//! proptest hunt for a session that violates either. A change to that ladder
//! fails here rather than silently making the unguarded arms wrong.
//!
//! ## What it covers, and what it does not
//!
//! The generated steps drive the real reducer and the real resource-state
//! openers — no mocks — and both invariants are checked after *every* step, not
//! only at quiescence, because a mode is read on every keystroke rather than at
//! rest. Steps are a sequence rather than a set on purpose: the openers are not
//! independent (`close_other_modals` clears its neighbours, a workspace switch
//! changes which filter flag a `Mode::Filter` can come from), so order is part
//! of the state space.
//!
//! Several openers call the resource-state methods directly rather than going
//! through `reduce`, because the reducer paths to them need effect results this
//! test has no runtime to deliver. The walk therefore explores a *superset* of
//! production-reachable states, which is the conservative direction — but it
//! means a reader cannot cite a pass here as evidence that reducer-enforced
//! modal exclusivity was exercised. `reducer_resources_test`'s
//! `opening_any_modal_over_any_other_leaves_exactly_one_open` is what pins that.
//!
//! It does not prove the ladder correct for states no step can reach. A branch
//! guarded on a condition no generated step produces is invisible here, the
//! same way it is invisible to the mutation gate. What it does buy is that
//! every arm that sweep touched is exercised in combination with the other
//! overlays, which is the interaction that would break them.

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
use proptest::prelude::*;

use crate::support::keymap_context::base_context;

use nuvem::app::reducer::root::reduce;
use nuvem::app::state::plugins::PluginId;
use nuvem::app::state::resources::{Compute, ComputeInstanceAction, PluginPrompt};
use nuvem::app::{
    Action, AppState, CommandAction, ConfigurationsAction, EffectResult, JumpsAction,
    LoggingAction, NavigatorAction, ResourcesAction, SystemAction, TunnelLaunchPlan, Workspace,
    WorkspaceAction,
};
use nuvem::config::plugins::{PluginInput, PluginInputKind};
use nuvem::input::keymap::action_for_key;
use nuvem::input::{Context, Mode};
use nuvem::services::runtime::input_context_for_test;
use nuvem::services::runtime::session::RuntimeSession;
use nuvem_domain::gcp::configuration::{GcloudConfiguration, GcloudConfigurationName};
use nuvem_domain::gcp::k9s::{CredentialStep, resolve_launch_target};
use nuvem_domain::gcp::resource::{
    ComputeInstance, EndpointVisibility, GkeCluster, GkeClusterStatus, ResourceStatus,
};
use nuvem_domain::gcp::tunnel::TunnelKind;
use ratatui::layout::Rect;

/// Large enough that every overlay renders. A terminal too small to draw a
/// modal changes the `_visible` flags, never the `_open` ones this invariant is
/// about, so the size is held constant rather than generated.
const TERMINAL: Rect = Rect {
    x: 0,
    y: 0,
    width: 160,
    height: 48,
};

fn configuration(name: &str, is_active: bool) -> GcloudConfiguration {
    GcloudConfiguration {
        name: GcloudConfigurationName::new(name),
        is_active,
        account: None,
        project: Some(format!("{name}-project")),
        region: None,
        zone: None,
    }
}

fn instance(name: &str) -> ComputeInstance {
    ComputeInstance {
        id: format!("{name}-id"),
        name: name.to_string(),
        zone: "europe-west1-b".to_string(),
        status: ResourceStatus::Running,
        machine_type: "e2-medium".to_string(),
        internal_ip: Some("10.0.0.4".to_string()),
        external_ip: None,
    }
}

fn gke_cluster(name: &str) -> GkeCluster {
    GkeCluster {
        name: name.to_string(),
        location: "europe-west1-b".to_string(),
        status: GkeClusterStatus::Running,
        current_master_version: Some("1.31.5-gke.1023000".to_string()),
        current_node_count: Some(3),
        endpoint_visibility: EndpointVisibility::Public,
    }
}

/// A session with everything the openers need already loaded: two
/// configurations so one is inactive and can be activated, a compute fleet so a
/// row is selected, and a GKE cluster so the k9s target resolves.
fn seeded_app() -> AppState {
    let mut app = AppState::new();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ConfigurationsLoaded {
            configurations: vec![configuration("dev", true), configuration("prod", false)],
        }),
    );
    reduce(&mut app, &Action::Workspace(WorkspaceAction::Resources));
    let context = app.root.current_context().expect("seeded context").clone();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ComputeInstancesLoaded {
            context,
            instances: vec![instance("api-dev-1"), instance("worker-dev-1")],
        }),
    );
    app
}

/// One step of a generated session. Every variant either opens or closes
/// something the `input_context` mode ladder reads, so a sequence of them walks
/// the ladder's reachable states.
#[derive(Debug, Clone, Copy)]
enum Step {
    OpenActionConfirmation,
    OpenBatchConfirmation,
    OpenCommandPreview,
    OpenSshLaunchPreview,
    OpenTunnelLaunchPreview,
    OpenK9sConfirmation,
    OpenPluginConfirmation,
    OpenPluginPrompt,
    OpenJumpPicker,
    CancelJumpPicker,
    OpenCommandPalette,
    CancelCommandPalette,
    ToggleHelp,
    ArmPluginPrefix,
    CancelPluginPrefix,
    StartServiceFilter,
    CancelServiceFilter,
    StartResourcesFilter,
    CancelResourcesFilter,
    StartConfigurationsFilter,
    CancelConfigurationsFilter,
    StartLoggingFilter,
    CancelLoggingFilter,
    StartConfigurationCreate,
    CancelConfigurationCreate,
    StartContextEdit,
    CancelContextEdit,
    OpenContextPicker,
    CancelContextPicker,
    RequestActivation,
    CancelActivation,
    OpenLoggingDetail,
    CloseLoggingDetail,
    ClearOverlay,
    SwitchWorkspace(u8),
}

fn step_strategy() -> impl Strategy<Value = Step> {
    prop_oneof![
        Just(Step::OpenActionConfirmation),
        Just(Step::OpenBatchConfirmation),
        Just(Step::OpenCommandPreview),
        Just(Step::OpenSshLaunchPreview),
        Just(Step::OpenTunnelLaunchPreview),
        Just(Step::OpenK9sConfirmation),
        Just(Step::OpenPluginConfirmation),
        Just(Step::OpenPluginPrompt),
        Just(Step::OpenJumpPicker),
        Just(Step::CancelJumpPicker),
        Just(Step::OpenCommandPalette),
        Just(Step::CancelCommandPalette),
        Just(Step::ToggleHelp),
        Just(Step::ArmPluginPrefix),
        Just(Step::CancelPluginPrefix),
        Just(Step::StartServiceFilter),
        Just(Step::CancelServiceFilter),
        Just(Step::StartResourcesFilter),
        Just(Step::CancelResourcesFilter),
        Just(Step::StartConfigurationsFilter),
        Just(Step::CancelConfigurationsFilter),
        Just(Step::StartLoggingFilter),
        Just(Step::CancelLoggingFilter),
        Just(Step::StartConfigurationCreate),
        Just(Step::CancelConfigurationCreate),
        Just(Step::StartContextEdit),
        Just(Step::CancelContextEdit),
        Just(Step::OpenContextPicker),
        Just(Step::CancelContextPicker),
        Just(Step::RequestActivation),
        Just(Step::CancelActivation),
        Just(Step::OpenLoggingDetail),
        Just(Step::CloseLoggingDetail),
        Just(Step::ClearOverlay),
        (0u8..5).prop_map(Step::SwitchWorkspace),
    ]
}

/// `reduce` returns the effects an action queued; every step here is driven for
/// its state change alone, and no runtime is present to run them.
fn dispatch(app: &mut AppState, action: Action) {
    let _ = reduce(app, &action);
}

/// Puts the session in the workspace a step's action is only accepted from.
///
/// The workspace-scoped openers switch first rather than no-oping when the
/// generated sequence happens to be somewhere else — which is what an operator
/// does, since the configuration forms cannot be opened from the resource list
/// at all. Without it those steps were reachable only when a `SwitchWorkspace`
/// landed immediately before them, and the states this property most needs to
/// see were the ones it almost never generated.
///
/// The `SwitchWorkspace` steps remain in the set and still fire afterwards,
/// which is what produces the combination the invariant is about: a flag opened
/// in its own workspace, then the workspace changed underneath it.
fn enter(app: &mut AppState, workspace: WorkspaceAction) {
    dispatch(app, Action::Workspace(workspace));
}

fn apply(step: Step, app: &mut AppState) {
    match step {
        Step::OpenActionConfirmation => {
            let context = app.root.current_context().expect("context").clone();
            let Some(selected) = app.root.resources.selected_instance().cloned() else {
                return;
            };
            app.root.resources.start_action_confirmation(
                context,
                selected,
                ComputeInstanceAction::Stop,
            );
        }
        Step::OpenBatchConfirmation => {
            let context = app.root.current_context().expect("context").clone();
            app.root.resources.toggle_mark();
            app.root
                .resources
                .start_batch_confirmation::<Compute>(context, ComputeInstanceAction::Stop);
        }
        Step::OpenCommandPreview => app.root.resources.show_command_preview(
            "Show command".to_string(),
            "gcloud --configuration dev compute instances describe api-dev-1".to_string(),
            "command".to_string(),
        ),
        Step::OpenSshLaunchPreview => {
            let context = app.root.current_context().expect("context").clone();
            let Some(selected) = app.root.resources.selected_instance().cloned() else {
                return;
            };
            app.root
                .resources
                .show_ssh_launch_preview(context, selected);
        }
        Step::OpenTunnelLaunchPreview => {
            let context = app.root.current_context().expect("context").clone();
            let Some(selected) = app.root.resources.selected_instance().cloned() else {
                return;
            };
            app.root.resources.show_tunnel_launch_preview(
                context,
                TunnelLaunchPlan {
                    kind: TunnelKind::IapTcp,
                    target_label: selected.name.clone(),
                    remote_label: "22".to_string(),
                    instance_name: selected.name.clone(),
                    zone: selected.zone.clone(),
                    connection_name: String::new(),
                    preferred_local_port: Some(8022),
                    remote_port: 22,
                    claimed_ports: Vec::new(),
                },
            );
        }
        Step::OpenK9sConfirmation => {
            let context = app.root.current_context().expect("context").clone();
            let Some(target) = resolve_launch_target(&context, &gke_cluster("platform-dev")) else {
                return;
            };
            app.root
                .resources
                .open_k9s_launch_confirmation(target, CredentialStep::ReuseExisting);
        }
        Step::OpenPluginConfirmation => app.root.resources.show_plugin_confirmation(
            "Describe api-dev-1".to_string(),
            vec!["compute".to_string(), "instances".to_string()],
            false,
        ),
        // A non-empty input list, because `PluginPrompt`'s focus movement
        // documents a non-empty invariant its callers uphold — a prompt seeded
        // with no fields is the one shape the type is not built for.
        Step::OpenPluginPrompt => app.root.resources.show_plugin_prompt(PluginPrompt::new(
            PluginId(0),
            "Resize api-dev-1".to_string(),
            false,
            vec![PluginInput {
                name: "disk_size".to_string(),
                kind: PluginInputKind::String,
                default: Some("100".to_string()),
            }],
            Vec::new(),
        )),
        Step::OpenJumpPicker => dispatch(app, Action::Jumps(JumpsAction::Open)),
        Step::CancelJumpPicker => dispatch(app, Action::Jumps(JumpsAction::Cancel)),
        Step::OpenCommandPalette => dispatch(app, Action::Command(CommandAction::Open)),
        Step::CancelCommandPalette => dispatch(app, Action::Command(CommandAction::Cancel)),
        Step::ToggleHelp => dispatch(app, Action::System(SystemAction::ToggleHelp)),
        Step::ArmPluginPrefix => dispatch(app, Action::Resources(ResourcesAction::ArmPluginPrefix)),
        Step::CancelPluginPrefix => {
            dispatch(app, Action::Resources(ResourcesAction::CancelPluginPrefix))
        }
        Step::StartServiceFilter => {
            enter(app, WorkspaceAction::Home);
            dispatch(app, Action::Navigator(NavigatorAction::StartFilter));
        }
        Step::CancelServiceFilter => {
            dispatch(app, Action::Navigator(NavigatorAction::CancelFilter))
        }
        Step::StartResourcesFilter => {
            enter(app, WorkspaceAction::Resources);
            dispatch(app, Action::Resources(ResourcesAction::StartFilter));
        }
        Step::CancelResourcesFilter => {
            dispatch(app, Action::Resources(ResourcesAction::CancelFilter))
        }
        Step::StartConfigurationsFilter => {
            enter(app, WorkspaceAction::Configurations);
            dispatch(
                app,
                Action::Configurations(ConfigurationsAction::StartFiltering),
            );
        }
        Step::CancelConfigurationsFilter => dispatch(
            app,
            Action::Configurations(ConfigurationsAction::CancelFilter),
        ),
        Step::StartLoggingFilter => {
            enter(app, WorkspaceAction::Logging);
            dispatch(app, Action::Logging(LoggingAction::StartFilter));
        }
        Step::CancelLoggingFilter => dispatch(app, Action::Logging(LoggingAction::CancelFilter)),
        Step::StartConfigurationCreate => {
            enter(app, WorkspaceAction::Configurations);
            dispatch(
                app,
                Action::Configurations(ConfigurationsAction::StartCreate),
            );
        }
        Step::CancelConfigurationCreate => dispatch(
            app,
            Action::Configurations(ConfigurationsAction::CancelCreate),
        ),
        Step::StartContextEdit => {
            enter(app, WorkspaceAction::Configurations);
            dispatch(
                app,
                Action::Configurations(ConfigurationsAction::StartEditActiveContext),
            );
        }
        Step::CancelContextEdit => dispatch(
            app,
            Action::Configurations(ConfigurationsAction::CancelContextEdit),
        ),
        // The picker opens *over* the context-edit form and reads its focused
        // field, so there is no path to it that does not leave
        // `configuration_context_edit_open` true underneath. That overlap is
        // the whole reason the ladder ranks the picker first, and the reason
        // the context-edit dispatch arm used to carry a
        // `!configuration_context_picker_open` conjunct.
        Step::OpenContextPicker => {
            apply(Step::StartContextEdit, app);
            dispatch(
                app,
                Action::Configurations(ConfigurationsAction::OpenContextPicker),
            );
        }
        Step::CancelContextPicker => dispatch(
            app,
            Action::Configurations(ConfigurationsAction::CancelContextPicker),
        ),
        Step::RequestActivation => {
            dispatch(
                app,
                Action::Configurations(ConfigurationsAction::SelectNext),
            );
            dispatch(
                app,
                Action::Configurations(ConfigurationsAction::RequestActivateSelected),
            );
        }
        Step::CancelActivation => dispatch(
            app,
            Action::Configurations(ConfigurationsAction::CancelActivation),
        ),
        // Present because it is a `Context` flag the ladder's neighbourhood
        // reads and nothing else here sets. A second mode assignment hidden
        // behind a flag no step produces is invisible to this property, so the
        // set of flags the steps can move is the property's real reach — and
        // that gap was found by probe, on this exact flag.
        //
        // Set directly rather than through `LoggingAction::OpenDetail`, which
        // needs a selected entry and so needs a loaded log page. Going through
        // the action instead was tried first and no-oped silently, which is the
        // failure mode `the_generated_steps_reach_the_logging_detail_flag`
        // exists to catch: a step that widens nothing while looking like it
        // widened something.
        Step::OpenLoggingDetail => {
            enter(app, WorkspaceAction::Logging);
            app.root.logging.detail_open = true;
        }
        Step::CloseLoggingDetail => dispatch(app, Action::Logging(LoggingAction::CloseDetail)),
        Step::ClearOverlay => dispatch(app, Action::System(SystemAction::ClearOverlay)),
        Step::SwitchWorkspace(which) => {
            let workspace = match which {
                0 => WorkspaceAction::Home,
                1 => WorkspaceAction::Resources,
                2 => WorkspaceAction::Configurations,
                3 => WorkspaceAction::Logging,
                _ => WorkspaceAction::Tunnels,
            };
            dispatch(app, Action::Workspace(workspace));
        }
    }
}

/// The first invariant: every mode the ladder can assign implies the condition
/// it is assigned under.
///
/// Matched exhaustively rather than with a lookup table, so a mode added to
/// `Mode` fails to compile here until someone decides which side it is on.
/// `Normal` is the ladder's fallthrough, and `Help` and `PluginPrefix` are
/// derived from state `Context` does not carry — those three have no flag to
/// imply, which is also why their dispatch arms are unguarded.
fn assert_mode_implies_its_flag(context: &Context, trace: &[Step]) {
    let holds = match context.mode {
        Mode::CommandPalette => context.command_palette_open,
        Mode::ResourceCommandPreview => context.resource_command_preview_open,
        Mode::SshLaunchPreview => context.ssh_launch_preview_open,
        Mode::TunnelLaunchPreview => context.tunnel_launch_preview_open,
        Mode::ConfirmK9sLaunch => context.k9s_launch_confirmation_open,
        Mode::ResourceJumpPicker => context.resource_jump_picker_open,
        Mode::PluginConfirmation => context.plugin_confirmation_open,
        Mode::PluginPrompt => context.plugin_prompt_open,
        Mode::PluginOutput => context.plugin_output_open,
        Mode::ConfirmResourceAction => {
            context.resource_action_confirmation_open || context.resource_batch_confirmation_open
        }
        Mode::ConfigurationContextPicker => context.configuration_context_picker_open,
        // The picker branch sits *above* the create/edit branch in the ladder,
        // so this mode also implies the picker is shut. The context-edit arm
        // used to restate that as a `!configuration_context_picker_open`
        // conjunct.
        Mode::EditConfiguration => {
            (context.configuration_create_open || context.configuration_context_edit_open)
                && !context.configuration_context_picker_open
        }
        Mode::Filter => {
            context.service_filter_open
                || context.resources_filter_open
                || context.configurations_filter_open
                || context.logging_filter_open
        }
        Mode::ConfirmConfigurationActivation => context.configuration_activation_confirmation_open,
        Mode::Normal | Mode::Help | Mode::PluginPrefix => true,
    };

    assert!(
        holds,
        "input_context assigned {:?} without the condition it is assigned under; \
         the keymap arm for it no longer restates that condition, so it would \
         dispatch to a handler whose modal is closed. Steps: {trace:?}",
        context.mode
    );
}

/// The second half of the contract: a workspace-scoped `_open` flag implies its
/// own workspace.
///
/// `input_context` builds each of these as `workspace == W && ...`, so a guard
/// pairing the flag with a `workspace ==` conjunct restates the flag's own
/// definition. Six such conjuncts sat on the `Mode::Filter` and
/// `Mode::EditConfiguration` arms — arms whose *other* conjunct is load-bearing,
/// which is why the guards stayed while the conjuncts went.
///
/// `service_filter_open` is deliberately absent: it is open on Home *or*
/// Resources, so it implies no single workspace, and its dispatch arm never
/// claimed one.
fn assert_flag_implies_its_workspace(context: &Context, trace: &[Step]) {
    let scoped = [
        (
            context.resources_filter_open,
            Workspace::Resources,
            "resources_filter_open",
        ),
        (
            context.logging_filter_open,
            Workspace::Logging,
            "logging_filter_open",
        ),
        (
            context.configurations_filter_open,
            Workspace::Configurations,
            "configurations_filter_open",
        ),
        (
            context.configuration_create_open,
            Workspace::Configurations,
            "configuration_create_open",
        ),
        (
            context.configuration_context_edit_open,
            Workspace::Configurations,
            "configuration_context_edit_open",
        ),
        (
            context.configuration_context_picker_open,
            Workspace::Configurations,
            "configuration_context_picker_open",
        ),
    ];

    for (open, workspace, name) in scoped {
        assert!(
            !open || context.workspace == workspace,
            "input_context reported {name} outside {workspace:?} (workspace is {:?}); \
             the keymap arm reading that flag no longer restates the workspace, \
             so it would dispatch the wrong view's keys. Steps: {trace:?}",
            context.workspace
        );
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    #[test]
    fn every_mode_the_ladder_assigns_implies_its_own_flag(
        steps in prop::collection::vec(step_strategy(), 1..12)
    ) {
        let mut session = RuntimeSession::new_for_test(seeded_app());
        let mut trace: Vec<Step> = Vec::new();

        for step in steps {
            apply(step, session.app_mut());
            trace.push(step);

            let context = input_context_for_test(&session, TERMINAL);
            assert_mode_implies_its_flag(&context, &trace);
            assert_flag_implies_its_workspace(&context, &trace);
        }
    }
}

/// Every mode the two assertions above constrain is actually reached by the
/// generated steps.
///
/// Without this the property could pass by never entering them — both
/// assertions are vacuous for a mode no session produces, so an opener that
/// silently no-ops (a batch confirmation with nothing marked, a k9s target that
/// will not resolve, a configuration form opened from the wrong workspace)
/// would quietly hollow out the property rather than fail it.
///
/// The four `Mode::Filter` rows are separate because they are separate dispatch
/// arms with separate flags, and three of the four are what
/// `assert_flag_implies_its_workspace` is about. `Mode::Normal`, `Mode::Help`
/// and `Mode::PluginPrefix` are absent for the reason their assertion is
/// `true`: there is no flag to be vacuous about.
#[test]
fn the_generated_steps_reach_every_mode_the_invariant_constrains() {
    let expected: [(&[Step], Mode); 17] = [
        (&[Step::OpenCommandPalette], Mode::CommandPalette),
        (&[Step::OpenCommandPreview], Mode::ResourceCommandPreview),
        (&[Step::OpenSshLaunchPreview], Mode::SshLaunchPreview),
        (&[Step::OpenTunnelLaunchPreview], Mode::TunnelLaunchPreview),
        (&[Step::OpenK9sConfirmation], Mode::ConfirmK9sLaunch),
        (&[Step::OpenJumpPicker], Mode::ResourceJumpPicker),
        (&[Step::OpenActionConfirmation], Mode::ConfirmResourceAction),
        (&[Step::OpenBatchConfirmation], Mode::ConfirmResourceAction),
        (&[Step::OpenPluginConfirmation], Mode::PluginConfirmation),
        (&[Step::OpenPluginPrompt], Mode::PluginPrompt),
        (
            &[Step::RequestActivation],
            Mode::ConfirmConfigurationActivation,
        ),
        (&[Step::OpenContextPicker], Mode::ConfigurationContextPicker),
        (&[Step::StartConfigurationCreate], Mode::EditConfiguration),
        (&[Step::StartContextEdit], Mode::EditConfiguration),
        (&[Step::StartServiceFilter], Mode::Filter),
        (&[Step::StartConfigurationsFilter], Mode::Filter),
        (&[Step::StartLoggingFilter], Mode::Filter),
    ];

    for (steps, mode) in expected {
        let mut app = seeded_app();
        for step in steps {
            apply(*step, &mut app);
        }
        let session = RuntimeSession::new_for_test(app);
        let context = input_context_for_test(&session, TERMINAL);
        assert_eq!(
            context.mode, mode,
            "{steps:?} did not put the session into {mode:?}, so the property test \
             never exercises that mode's dispatch arm"
        );
    }
}

/// `Step::OpenLoggingDetail` really sets the flag it is named for.
///
/// It is the one step present purely to widen the property's reach rather than
/// to reach a mode, so nothing else would notice it going inert — and its first
/// implementation did exactly that.
#[test]
fn the_generated_steps_reach_the_logging_detail_flag() {
    let mut app = seeded_app();
    apply(Step::OpenLoggingDetail, &mut app);

    let session = RuntimeSession::new_for_test(app);
    let context = input_context_for_test(&session, TERMINAL);
    assert!(
        context.logging_detail_open,
        "no generated step opens the logging detail, so a ladder branch \
         conditioned on it would be invisible to the property"
    );
}

/// The resources filter is the one `Mode::Filter` flavour the walk above cannot
/// pin by mode alone: with the sidebar focused the service filter outranks it,
/// so reaching `resources_filter_open` is what has to be asserted rather than
/// the mode it produces.
#[test]
fn the_generated_steps_reach_the_resources_filter_flag() {
    let mut app = seeded_app();
    apply(Step::StartResourcesFilter, &mut app);

    let session = RuntimeSession::new_for_test(app);
    let context = input_context_for_test(&session, TERMINAL);
    assert!(
        context.resources_filter_open,
        "no generated step opens the resources filter, so \
         assert_flag_implies_its_workspace never checks it"
    );
}

/// One row of the filter-arm walk: the flavour's name, what opening it looks
/// like on a `Context`, and the action its own handler answers `Esc` with.
type FilterCase = (&'static str, fn(&mut Context), Action);

/// Each `Mode::Filter` arm dispatches to its *own* handler.
///
/// The arms are ordered, and the last one carries no guard, so a guard that
/// wrongly answers false does not decline the key — it hands it to the next
/// arm, and eventually to the logging filter, which is a different view's keys
/// on the same keystroke. The diff-scoped mutation gate found exactly that:
/// `replace match guard context.service_filter_open with false` survived,
/// because nothing distinguished the service filter's handler from its
/// neighbours.
///
/// `Esc` is the probe because it is the one key all four bind to visibly
/// different actions. Each case sets one flag only — the state
/// `assert_flag_implies_its_workspace` says the runtime produces — so a leak to
/// a neighbouring arm shows up as that neighbour's action.
#[test]
fn each_filter_arm_dispatches_to_its_own_handler() {
    let esc = KeyEvent {
        code: KeyCode::Esc,
        modifiers: KeyModifiers::empty(),
        kind: KeyEventKind::Press,
        state: KeyEventState::empty(),
    };

    let cases: [FilterCase; 4] = [
        (
            "service",
            |context| {
                context.workspace = Workspace::Home;
                context.service_filter_open = true;
            },
            Action::Navigator(NavigatorAction::CancelFilter),
        ),
        (
            "resources",
            |context| {
                context.workspace = Workspace::Resources;
                context.resources_filter_open = true;
            },
            Action::Resources(ResourcesAction::CancelFilter),
        ),
        (
            "configurations",
            |context| {
                context.workspace = Workspace::Configurations;
                context.configurations_filter_open = true;
            },
            Action::Configurations(ConfigurationsAction::CancelFilter),
        ),
        (
            "logging",
            |context| {
                context.workspace = Workspace::Logging;
                context.logging_filter_open = true;
            },
            Action::Logging(LoggingAction::CancelFilter),
        ),
    ];

    for (name, open, expected) in cases {
        let mut context = base_context();
        context.mode = Mode::Filter;
        open(&mut context);

        assert_eq!(
            action_for_key(esc, &context),
            Some(expected),
            "Esc with only the {name} filter open did not reach the {name} filter's \
             handler; the dispatch arm handed the key to a neighbouring filter"
        );
    }

    // The four cases above are each reachable with one flag set, so between
    // them they pin every arm against a *wrongly declining* neighbour — but not
    // against a reorder, because three of the four are mutually exclusive by
    // workspace and swapping them changes nothing.
    //
    // The service and resources filters are the exception: both can be open at
    // once on `Workspace::Resources`, since `service_filter_open` is the
    // navigator's own filter and the sidebar sits beside the list. So their
    // relative order is the one ordering in this group that is load-bearing,
    // and cargo-mutants has no operator that reorders match arms — nothing but
    // this assertion would notice.
    let mut both = base_context();
    both.mode = Mode::Filter;
    both.workspace = Workspace::Resources;
    both.service_filter_open = true;
    both.resources_filter_open = true;
    assert_eq!(
        action_for_key(esc, &both),
        Some(Action::Navigator(NavigatorAction::CancelFilter)),
        "with both the navigator and the resource filter open, Esc must reach the \
         navigator's — its arm is first, and the focused sidebar is what the \
         operator is typing into"
    );
}

/// The same, for the two `Mode::EditConfiguration` arms.
///
/// Its second arm is unguarded for the same reason the logging one is, so a
/// create-form guard answering false would hand the key to the context-edit
/// form rather than declining it.
#[test]
fn each_edit_configuration_arm_dispatches_to_its_own_handler() {
    let esc = KeyEvent {
        code: KeyCode::Esc,
        modifiers: KeyModifiers::empty(),
        kind: KeyEventKind::Press,
        state: KeyEventState::empty(),
    };

    let mut create = base_context();
    create.mode = Mode::EditConfiguration;
    create.workspace = Workspace::Configurations;
    create.configuration_create_open = true;
    assert_eq!(
        action_for_key(esc, &create),
        Some(Action::Configurations(ConfigurationsAction::CancelCreate)),
        "Esc in the create form did not reach the create form's handler"
    );

    let mut edit = base_context();
    edit.mode = Mode::EditConfiguration;
    edit.workspace = Workspace::Configurations;
    edit.configuration_context_edit_open = true;
    assert_eq!(
        action_for_key(esc, &edit),
        Some(Action::Configurations(
            ConfigurationsAction::CancelContextEdit
        )),
        "Esc in the context-edit form did not reach the context-edit form's handler"
    );
}
