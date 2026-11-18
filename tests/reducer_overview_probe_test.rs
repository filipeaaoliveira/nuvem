use nuvem::app::reducer::root::reduce;
use nuvem::app::state::root::Workspace;
use nuvem::app::{Action, AppState, Effect, WorkspaceAction};

#[test]
fn entering_home_emits_a_single_probe() {
    let mut app = AppState::new();
    app.root.workspace = Workspace::Resources;
    let effects = reduce(&mut app, &Action::Workspace(WorkspaceAction::Home));
    assert_eq!(app.root.workspace, Workspace::Home);
    assert!(matches!(
        effects.as_slice(),
        [Effect::ProbeGcloudDiagnostics { .. }]
    ));
    assert!(app.root.system.diagnostics.is_in_flight());
}

#[test]
fn re_entering_home_while_in_flight_emits_nothing() {
    let mut app = AppState::new();
    let _ = reduce(&mut app, &Action::Workspace(WorkspaceAction::Home));
    let effects = reduce(&mut app, &Action::Workspace(WorkspaceAction::Home));
    assert!(effects.is_empty());
}
