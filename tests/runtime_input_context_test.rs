use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
use nuvem::app::reducer::root::reduce;
use nuvem::app::state::resources::{Compute, ComputeInstanceAction};
use nuvem::app::{
    Action, AppState, ConfigurationsAction, EffectResult, JumpsAction, NavigatorAction,
    ResourcesAction, Workspace, WorkspaceAction,
};
use nuvem::input::Mode;
use nuvem::input::keymap::action_for_key;
use nuvem::services::runtime::input_context_for_test;
use nuvem::services::runtime::session::RuntimeSession;
use nuvem_domain::gcp::configuration::{GcloudConfiguration, GcloudConfigurationName};
use nuvem_domain::gcp::resource::{ComputeInstance, ResourceServiceKind, ResourceStatus};
use ratatui::layout::Rect;

fn running_instance(name: &str) -> ComputeInstance {
    ComputeInstance {
        id: format!("{name}-id"),
        name: name.to_string(),
        zone: "europe-west1-b".to_string(),
        status: ResourceStatus::Running,
        machine_type: "e2-medium".to_string(),
        internal_ip: None,
        external_ip: None,
    }
}

fn stopped_instance(name: &str) -> ComputeInstance {
    ComputeInstance {
        id: format!("{name}-id"),
        name: name.to_string(),
        zone: "europe-west1-b".to_string(),
        status: ResourceStatus::Stopped,
        machine_type: "e2-medium".to_string(),
        internal_ip: None,
        external_ip: None,
    }
}

fn configuration(name: &str, is_active: bool) -> GcloudConfiguration {
    GcloudConfiguration {
        name: GcloudConfigurationName::new(name),
        is_active,
        account: None,
        project: None,
        region: None,
        zone: None,
    }
}

#[test]
fn pending_configuration_activation_confirmation_uses_confirmation_input_mode() {
    let mut app = AppState::new();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ConfigurationsLoaded {
            configurations: vec![configuration("dev", true), configuration("prod", false)],
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
    let session = RuntimeSession::new_for_test(app);

    let visible_context = input_context_for_test(&session, Rect::new(0, 0, 120, 30));
    assert_eq!(visible_context.mode, Mode::ConfirmConfigurationActivation);
    assert!(visible_context.configuration_activation_confirmation_preview_visible);

    let hidden_context = input_context_for_test(&session, Rect::new(0, 0, 60, 12));
    assert_eq!(hidden_context.mode, Mode::ConfirmConfigurationActivation);
    assert!(!hidden_context.configuration_activation_confirmation_preview_visible);
}

#[test]
fn home_navigator_filter_uses_service_filter_context() {
    let mut app = AppState::new();
    reduce(&mut app, &Action::Navigator(NavigatorAction::StartFilter));
    let session = RuntimeSession::new_for_test(app);

    let context = input_context_for_test(&session, Rect::new(0, 0, 120, 30));
    assert_eq!(context.mode, Mode::Filter);
    assert!(context.service_filter_open);
    assert!(!context.resources_filter_open);
}

#[test]
fn resources_sidebar_focus_uses_service_filter_context() {
    let mut app = AppState::new();
    reduce(&mut app, &Action::Workspace(WorkspaceAction::Resources));
    reduce(&mut app, &Action::Navigator(NavigatorAction::ToggleFocus));
    reduce(&mut app, &Action::Navigator(NavigatorAction::StartFilter));
    let session = RuntimeSession::new_for_test(app);

    let context = input_context_for_test(&session, Rect::new(0, 0, 120, 30));
    assert_eq!(context.mode, Mode::Filter);
    assert!(context.service_filter_open);
    assert!(context.service_sidebar_visible);
    assert!(context.service_navigator_focused);
    assert!(!context.resources_filter_open);
}

#[test]
fn resources_content_filter_remains_separate_from_service_filter_context() {
    let mut app = AppState::new();
    reduce(&mut app, &Action::Workspace(WorkspaceAction::Resources));
    reduce(&mut app, &Action::Resources(ResourcesAction::StartFilter));
    let session = RuntimeSession::new_for_test(app);

    let context = input_context_for_test(&session, Rect::new(0, 0, 120, 30));
    assert_eq!(context.mode, Mode::Filter);
    assert!(!context.service_filter_open);
    assert!(!context.service_navigator_focused);
    assert!(context.resources_filter_open);
}

#[test]
fn open_batch_confirmation_uses_confirmation_mode_and_reports_unsatisfied_ack() {
    let mut app = AppState::new();
    reduce(&mut app, &Action::Workspace(WorkspaceAction::Resources));
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ConfigurationsLoaded {
            configurations: vec![configuration("dev", true)],
        }),
    );
    let context = app.root.current_context().unwrap().clone();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ComputeInstancesLoaded {
            context: context.clone(),
            instances: vec![running_instance("api"), running_instance("web")],
        }),
    );
    // Mark both instances, then open a destructive (typed-count) batch.
    app.root.resources.toggle_mark();
    app.root.resources.select_next();
    app.root.resources.toggle_mark();
    app.root
        .resources
        .start_batch_confirmation::<Compute>(context, ComputeInstanceAction::Stop);
    assert!(app.root.resources.batch_confirmation().is_some());
    assert_eq!(app.root.workspace, Workspace::Resources);

    let session = RuntimeSession::new_for_test(app);
    let ctx = input_context_for_test(&session, Rect::new(0, 0, 120, 30));

    assert_eq!(ctx.mode, Mode::ConfirmResourceAction);
    assert!(ctx.resource_batch_confirmation_open);
    // A typed-count ack with nothing typed is not satisfied, so Enter is blocked.
    assert!(!ctx.resource_batch_ack_satisfied);
}

#[test]
fn batch_confirmation_on_small_terminal_blocks_enter_even_when_ack_is_satisfied() {
    let mut app = AppState::new();
    reduce(&mut app, &Action::Workspace(WorkspaceAction::Resources));
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ConfigurationsLoaded {
            configurations: vec![configuration("dev", true)],
        }),
    );
    let context = app.root.current_context().unwrap().clone();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ComputeInstancesLoaded {
            context: context.clone(),
            instances: vec![stopped_instance("api"), stopped_instance("web")],
        }),
    );
    // Mark both instances, then open a non-destructive (Start) batch: its
    // `Simple` ack is always satisfied, so the only guard left standing
    // between Enter and `ConfirmBatchAction` is modal visibility.
    app.root.resources.toggle_mark();
    app.root.resources.select_next();
    app.root.resources.toggle_mark();
    app.root
        .resources
        .start_batch_confirmation::<Compute>(context, ComputeInstanceAction::Start);
    assert!(app.root.resources.batch_confirmation().is_some());

    let session = RuntimeSession::new_for_test(app);

    let visible_ctx = input_context_for_test(&session, Rect::new(0, 0, 120, 30));
    assert_eq!(visible_ctx.mode, Mode::ConfirmResourceAction);
    assert!(visible_ctx.resource_batch_confirmation_open);
    assert!(visible_ctx.resource_batch_ack_satisfied);
    assert!(visible_ctx.resource_batch_confirmation_visible);
    assert_eq!(
        action_for_key(enter_key(), &visible_ctx),
        Some(Action::Resources(ResourcesAction::ConfirmBatchAction))
    );

    let hidden_ctx = input_context_for_test(&session, Rect::new(0, 0, 10, 5));
    assert_eq!(hidden_ctx.mode, Mode::ConfirmResourceAction);
    assert!(hidden_ctx.resource_batch_confirmation_open);
    assert!(hidden_ctx.resource_batch_ack_satisfied);
    assert!(!hidden_ctx.resource_batch_confirmation_visible);
    // The target list isn't on screen, so Enter must not confirm the batch.
    assert_eq!(action_for_key(enter_key(), &hidden_ctx), None);
}

/// The plugin confirmation's readability guard, taken from a real session at
/// both sizes. `plugin_confirmation_visible` answering `true` is pinned at a
/// wide terminal by
/// `plugin_execution_evidence::a_dangerous_plugin_cannot_be_confirmed_by_a_keypress_alone`,
/// and the renderer's own half — the fallback message — by that file's
/// `a_terminal_too_small_for_the_modal_falls_back_to_a_message`. Neither
/// produces a `false` from the projection: every other `false` in the suite is
/// written by hand into a `Context` literal (`keymap_test`). So a projection
/// simplified to `.is_some()` would leave the operator reading "needs a larger
/// terminal" — no argv at all — while `Enter` still ran the command, with the
/// suite green.
#[test]
fn plugin_confirmation_visibility_follows_the_terminal_the_session_is_drawn_into() {
    let mut app = AppState::new();
    app.root.resources.show_plugin_confirmation(
        "Describe web-01".to_string(),
        vec![
            "--configuration".to_string(),
            "dev".to_string(),
            "compute".to_string(),
            "instances".to_string(),
            "describe".to_string(),
            "web-01".to_string(),
            // Long enough that the argv still does not fit at 60x12. The
            // modal wraps rather than demanding each line fit one
            // row, so a short argv now draws at that size and the `false` this
            // test is about would never be produced.
            "--project=acme-production-platform".to_string(),
            "--format=json(name,status,machineType,networkInterfaces)".to_string(),
        ],
        // An ordinary plugin, whose `Simple` ack is satisfied from the start:
        // that leaves readability as the only thing between `Enter` and the
        // child, so the `None` below is attributable to this flag alone.
        false,
    );
    let session = RuntimeSession::new_for_test(app);

    let visible_ctx = input_context_for_test(&session, Rect::new(0, 0, 120, 30));
    assert_eq!(visible_ctx.mode, Mode::PluginConfirmation);
    assert!(visible_ctx.plugin_confirmation_open);
    assert!(visible_ctx.plugin_confirmation_ack_satisfied);
    assert!(visible_ctx.plugin_confirmation_visible);
    assert_eq!(
        action_for_key(enter_key(), &visible_ctx),
        Some(Action::Resources(ResourcesAction::ConfirmPlugin))
    );

    // 60x12 is the app's own minimum supported size, and the size at which the
    // modal draws its fallback message instead of the argv.
    let hidden_ctx = input_context_for_test(&session, Rect::new(0, 0, 60, 12));
    assert_eq!(hidden_ctx.mode, Mode::PluginConfirmation);
    assert!(hidden_ctx.plugin_confirmation_open);
    assert!(hidden_ctx.plugin_confirmation_ack_satisfied);
    assert!(
        !hidden_ctx.plugin_confirmation_visible,
        "the argv is not on screen at 60x12, and the projection has to say so"
    );
    // Nothing the operator could have read, so Enter must not run the command.
    assert_eq!(action_for_key(enter_key(), &hidden_ctx), None);
}

/// The prompt's readability guard, taken from a real session at both sizes —
/// the same evidence the confirmation carries above, one step earlier in the
/// chain. The keymap parity walk only probes the satisfied-and-visible state,
/// so without this the `visible = false` branch would rest on a hand-written
/// `Context` literal: a projection simplified to `.is_some()` would leave the
/// operator reading "needs a larger terminal" — no values at all — while
/// `Enter` still composed an argv out of them, with the suite green.
#[test]
fn plugin_prompt_visibility_follows_the_terminal_the_session_is_drawn_into() {
    use nuvem::app::state::plugins::PluginId;
    use nuvem::app::state::resources::PluginPrompt;
    use nuvem::config::plugins::{PluginInput, PluginInputKind};

    let mut app = AppState::new();
    // Every field defaulted, so the prompt is satisfied from the start and
    // readability is the only thing between `Enter` and the confirmation.
    let inputs = (0..4)
        .map(|index| PluginInput {
            name: format!("field_{index}"),
            kind: PluginInputKind::String,
            default: Some("100".to_string()),
        })
        .collect();
    app.root.resources.show_plugin_prompt(PluginPrompt::new(
        PluginId(0),
        "Resize web-01".to_string(),
        false,
        inputs,
        Vec::new(),
    ));
    let session = RuntimeSession::new_for_test(app);

    let visible_ctx = input_context_for_test(&session, Rect::new(0, 0, 120, 30));
    assert_eq!(visible_ctx.mode, Mode::PluginPrompt);
    assert!(visible_ctx.plugin_prompt_satisfied);
    assert!(visible_ctx.plugin_prompt_visible);
    assert_eq!(
        action_for_key(enter_key(), &visible_ctx),
        Some(Action::Resources(ResourcesAction::SubmitPluginPrompt))
    );

    // 60x12 is the app's own minimum supported size, and the size at which the
    // modal draws its fallback message instead of the fields.
    let hidden_ctx = input_context_for_test(&session, Rect::new(0, 0, 60, 12));
    assert_eq!(hidden_ctx.mode, Mode::PluginPrompt);
    assert!(hidden_ctx.plugin_prompt_satisfied);
    assert!(
        !hidden_ctx.plugin_prompt_visible,
        "the fields are not on screen at 60x12, and the projection has to say so"
    );
    // Nothing the operator could have read, so Enter must not compose the argv.
    assert_eq!(action_for_key(enter_key(), &hidden_ctx), None);
}

/// An armed prefix is a mode with no overlay of its own, so the only
/// evidence it is armed is that the next character reaches the reducer as a
/// plugin key. This pins the derivation the keymap's dispatch arm relies on:
/// `Mode::PluginPrefix` is assigned from `plugin_prefix_pending` and nowhere
/// else, which is why neither the arm nor `plugin_prefix_key` restates the flag.
#[test]
fn a_pending_plugin_prefix_reads_the_next_character_as_a_plugin_key() {
    let mut app = AppState::new();
    app.root.plugin_prefix_pending = Some(ResourceServiceKind::ComputeInstances);
    let session = RuntimeSession::new_for_test(app);

    let ctx = input_context_for_test(&session, Rect::new(0, 0, 120, 30));

    assert_eq!(ctx.mode, Mode::PluginPrefix);
    assert_eq!(
        action_for_key(
            KeyEvent {
                code: KeyCode::Char('d'),
                modifiers: KeyModifiers::empty(),
                kind: KeyEventKind::Press,
                state: KeyEventState::empty(),
            },
            &ctx
        ),
        Some(Action::Resources(ResourcesAction::PluginPrefixPress('d')))
    );
}

/// The prefix ranks last in the ladder, below every arm that stands for an
/// overlay. `,` binds only in the normal resource views, so this combination is
/// unreachable today — which is the reason to pin it: were a future path to arm
/// the prefix under an open modal, ranking the prefix higher would strand that
/// modal on screen with its own keys dead.
#[test]
fn an_open_confirmation_outranks_a_pending_prefix() {
    let mut app = AppState::new();
    app.root.plugin_prefix_pending = Some(ResourceServiceKind::ComputeInstances);
    app.root.resources.show_plugin_confirmation(
        "Describe web-01".to_string(),
        vec!["compute".to_string(), "instances".to_string()],
        false,
    );
    let session = RuntimeSession::new_for_test(app);

    let ctx = input_context_for_test(&session, Rect::new(0, 0, 120, 30));

    assert_eq!(ctx.mode, Mode::PluginConfirmation);
}

fn enter_key() -> KeyEvent {
    KeyEvent {
        code: KeyCode::Enter,
        modifiers: KeyModifiers::empty(),
        kind: KeyEventKind::Press,
        state: KeyEventState::empty(),
    }
}

/// End-to-end wiring for the related-resource jump picker: an open
/// picker must actually put the runtime into the mode whose keymap arm serves
/// it. Without this the reducer and the keymap can both be right while the
/// modal on screen is driven by the resource list's keys.
#[test]
fn an_open_jump_picker_uses_the_jump_picker_input_mode() {
    let mut app = AppState::new();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ConfigurationsLoaded {
            configurations: vec![configuration("dev", true)],
        }),
    );
    reduce(&mut app, &Action::Workspace(WorkspaceAction::Resources));
    let gcloud_context = app.root.current_context().expect("context exists");
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ComputeInstancesLoaded {
            context: gcloud_context,
            instances: vec![running_instance("web-01")],
        }),
    );
    reduce(&mut app, &Action::Jumps(JumpsAction::Open));
    assert!(app.root.jump_picker.is_some(), "picker should have opened");

    let session = RuntimeSession::new_for_test(app);
    let context = input_context_for_test(&session, Rect::new(0, 0, 120, 30));

    assert!(context.resource_jump_picker_open);
    assert_eq!(context.mode, Mode::ResourceJumpPicker);
    assert_eq!(
        action_for_key(
            KeyEvent {
                code: KeyCode::Enter,
                modifiers: KeyModifiers::empty(),
                kind: KeyEventKind::Press,
                state: KeyEventState::empty(),
            },
            &context
        ),
        Some(Action::Jumps(JumpsAction::Submit)),
        "Enter in the open picker must reach the jump, not the list's Open"
    );
}
