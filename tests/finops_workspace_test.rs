//! The FinOps workspace shell: how it is reached, what reaching it does, and
//! what the navigation history restores.

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
use nuvem::app::projection::binding::help_model;
use nuvem::app::projection::project_app_frame;
use nuvem::app::reducer::reduce;
use nuvem::app::state::command::CommandDescriptor;
use nuvem::app::state::root::Workspace;
use nuvem::app::{
    Action, AppState, CommandAction, Effect, FinOpsAction, NavAction, WorkspaceAction,
};
use nuvem::input::keymap::action_for_key;
use nuvem::input::{Context, Mode};
use nuvem_domain::gcp::configuration::{GcloudConfiguration, GcloudConfigurationName};
use nuvem_domain::gcp::resource::{ComputeInstance, ResourceStatus};

use crate::support::keymap_context::base_context;

fn config() -> GcloudConfiguration {
    GcloudConfiguration {
        name: GcloudConfigurationName::new("dev"),
        is_active: true,
        account: None,
        project: Some("proj".to_string()),
        region: None,
        zone: None,
    }
}

fn key(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
    KeyEvent {
        code,
        modifiers,
        kind: KeyEventKind::Press,
        state: KeyEventState::NONE,
    }
}

/// An app with an active configuration and one listed instance, so entering
/// FinOps has locations to derive a fanout from.
fn app_with_a_listed_zone() -> AppState {
    let mut app = AppState::new();
    app.root.configurations.set_active(config());
    let context = app.root.current_context().expect("an active context");
    app.root.resources.set_instances(
        context,
        vec![ComputeInstance {
            id: "1".to_string(),
            name: "vm-1".to_string(),
            zone: "europe-west1-b".to_string(),
            status: ResourceStatus::Running,
            machine_type: "e2-medium".to_string(),
            internal_ip: None,
            external_ip: None,
        }],
    );
    app
}

fn probe_count(effects: &[Effect]) -> usize {
    effects
        .iter()
        .filter(|effect| matches!(effect, Effect::LoadCostRecommendations { .. }))
        .count()
}

/// `Ctrl-f`, not a bare `f`: `f` is the resource filter, and this arm sits
/// ahead of the per-workspace dispatch, so a modifier-blind version would
/// shadow it from every workspace.
#[test]
fn ctrl_f_opens_finops_and_a_bare_f_does_not() {
    let context = Context {
        workspace: Workspace::Home,
        ..base_context()
    };
    assert_eq!(
        action_for_key(key(KeyCode::Char('f'), KeyModifiers::CONTROL), &context),
        Some(Action::Workspace(WorkspaceAction::FinOps))
    );
    assert_ne!(
        action_for_key(key(KeyCode::Char('f'), KeyModifiers::NONE), &context),
        Some(Action::Workspace(WorkspaceAction::FinOps))
    );
}

/// The chord is exact. `.contains(CONTROL)` would hand Ctrl-Alt-F and
/// Ctrl-Shift-F a workspace the catalog never claims for them.
#[test]
fn a_wider_ctrl_f_chord_is_not_claimed() {
    let context = Context {
        workspace: Workspace::Home,
        ..base_context()
    };
    for modifiers in [
        KeyModifiers::CONTROL | KeyModifiers::ALT,
        KeyModifiers::CONTROL | KeyModifiers::SHIFT,
    ] {
        assert_ne!(
            action_for_key(key(KeyCode::Char('f'), modifiers), &context),
            Some(Action::Workspace(WorkspaceAction::FinOps)),
            "{modifiers:?} must not open FinOps"
        );
    }
}

/// Entering the workspace probes. The grid is empty until something asks, and
/// an empty grid renders "not probed" — so an entry that dispatched nothing
/// would leave the workspace permanently blank.
#[test]
fn entering_the_workspace_probes_every_cell() {
    let mut app = app_with_a_listed_zone();
    let effects = reduce(&mut app, &Action::Workspace(WorkspaceAction::FinOps));

    assert_eq!(app.root.workspace, Workspace::FinOps);
    assert_eq!(probe_count(&effects), 4);
}

/// Entering with no active configuration still lands on the workspace, and
/// dispatches nothing rather than querying whatever gcloud defaults to.
#[test]
fn entering_without_a_configuration_lands_but_probes_nothing() {
    let mut app = AppState::new();
    let effects = reduce(&mut app, &Action::Workspace(WorkspaceAction::FinOps));

    assert_eq!(app.root.workspace, Workspace::FinOps);
    assert_eq!(probe_count(&effects), 0);
}

/// The palette reaches it, and reaching it that way probes too — otherwise the
/// same destination would behave differently depending on how it was opened.
#[test]
fn the_command_palette_opens_finops_and_probes() {
    let mut app = app_with_a_listed_zone();
    reduce(&mut app, &Action::Command(CommandAction::Open));
    for character in "finops".chars() {
        reduce(
            &mut app,
            &Action::Command(CommandAction::AppendChar(character)),
        );
    }
    let effects = reduce(&mut app, &Action::Command(CommandAction::Submit));

    assert_eq!(app.root.workspace, Workspace::FinOps);
    assert_eq!(probe_count(&effects), 4);
}

/// "cost" and "savings" find it too. An operator looking for money is unlikely
/// to type the word "recommender".
#[test]
fn the_palette_finds_finops_by_the_words_an_operator_would_type() {
    for query in ["cost", "savings", "budget", "spend"] {
        let mut app = app_with_a_listed_zone();
        reduce(&mut app, &Action::Command(CommandAction::Open));
        for character in query.chars() {
            reduce(
                &mut app,
                &Action::Command(CommandAction::AppendChar(character)),
            );
        }
        assert!(
            app.root
                .command
                .visible_commands(&app.root)
                .contains(&CommandDescriptor::GoToFinOps),
            "{query} does not find FinOps"
        );
    }
}

/// Going back to FinOps re-probes rather than restoring a stale grid:
/// recommendations refresh daily on Google's side, and the locations worth
/// asking about may have been listed since.
#[test]
fn navigating_back_to_finops_reprobes() {
    let mut app = app_with_a_listed_zone();
    reduce(&mut app, &Action::Workspace(WorkspaceAction::FinOps));
    reduce(&mut app, &Action::Workspace(WorkspaceAction::Home));

    let effects = reduce(&mut app, &Action::Navigation(NavAction::Back));
    assert_eq!(app.root.workspace, Workspace::FinOps);
    assert_eq!(probe_count(&effects), 4);
}

/// The workspace has its own help context, so its key list is not another
/// workspace's and the help view names where the operator actually is.
#[test]
fn the_workspace_has_its_own_help_context() {
    let mut app = app_with_a_listed_zone();
    reduce(&mut app, &Action::Workspace(WorkspaceAction::FinOps));
    assert_eq!(help_model(&app.root).view_title, "FinOps");
}

/// Escape leaves through the navigation history, like every other workspace
/// with a single view.
#[test]
fn escape_leaves_the_workspace() {
    let context = Context {
        workspace: Workspace::FinOps,
        mode: Mode::Normal,
        ..base_context()
    };
    assert_eq!(
        action_for_key(key(KeyCode::Esc, KeyModifiers::NONE), &context),
        Some(Action::Navigation(NavAction::Back))
    );
}

/// The frame names the workspace itself, asserted on the projection rather
/// than on rendered pixels: "FinOps" also reaches the screen as the Ctrl-f nav
/// hint in the footer, so a screen-scraping assertion passes even when the
/// view title is wrong.
#[test]
fn the_frame_titles_and_breadcrumbs_name_the_workspace() {
    let mut app = app_with_a_listed_zone();
    reduce(&mut app, &Action::Workspace(WorkspaceAction::FinOps));

    let frame = project_app_frame(&app.root);
    assert_eq!(frame.view_title, "FinOps");
    assert_eq!(
        frame.breadcrumbs,
        vec!["Home".to_string(), "FinOps".to_string()]
    );
    assert_eq!(
        frame.service_label, None,
        "FinOps is not a resource service"
    );
}

/// `r` re-probes here, unlike Tunnels where it is deliberately a no-op, so the
/// footer advertises it.
#[test]
fn the_footer_offers_refresh() {
    let mut app = app_with_a_listed_zone();
    reduce(&mut app, &Action::Workspace(WorkspaceAction::FinOps));

    let frame = project_app_frame(&app.root);
    assert!(
        frame.footer_hints.iter().any(|hint| hint.key == "r"),
        "{:?}",
        frame.footer_hints
    );
}

/// The palette opens from FinOps, and it does so through the rebindable
/// override table rather than through a per-workspace arm — which is why
/// `finops_key` deliberately has no `:` case for a rebind to be shadowed by.
#[test]
fn the_palette_opens_from_finops_through_the_rebindable_table() {
    let context = Context {
        workspace: Workspace::FinOps,
        mode: Mode::Normal,
        ..base_context()
    };
    assert_eq!(
        action_for_key(key(KeyCode::Char(':'), KeyModifiers::NONE), &context),
        Some(Action::Command(CommandAction::Open))
    );
}

/// `r` re-probes from inside the workspace, also through the rebindable table.
#[test]
fn r_reprobes_from_inside_the_workspace() {
    let context = Context {
        workspace: Workspace::FinOps,
        mode: Mode::Normal,
        ..base_context()
    };
    assert_eq!(
        action_for_key(key(KeyCode::Char('r'), KeyModifiers::NONE), &context),
        Some(Action::FinOps(FinOpsAction::Refresh))
    );
}
