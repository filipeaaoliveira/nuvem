use nuvem::app::state::operations::OperationOutcome;
use nuvem::app::state::system::SystemState;

#[test]
fn system_state_default_has_empty_operations_and_not_run_diagnostics() {
    let system = SystemState::default();
    assert!(system.operations.is_empty());
    assert!(!system.diagnostics.is_in_flight());
}

#[test]
fn system_state_tracks_operations() {
    let mut system = SystemState::default();
    let id = system.operations.begin("Stop instance", "db-1");
    system
        .operations
        .resolve_by_id(id, OperationOutcome::Failed, Some("boom".to_string()));
    assert_eq!(
        system.operations.records().front().unwrap().outcome,
        OperationOutcome::Failed
    );
}
