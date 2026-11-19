use nuvem::app::state::resources::ResourcesState;
use nuvem_domain::gcp::configuration::{GcloudConfiguration, GcloudConfigurationName};
use nuvem_domain::gcp::diagnostics::{DiagnosticProblem, GcloudErrorKind};
use nuvem_domain::gcp::resource::{ComputeInstance, ResourceServiceKind, ResourceStatus};

fn context() -> nuvem_domain::gcp::configuration::GcloudContext {
    GcloudConfiguration {
        name: GcloudConfigurationName::new("dev"),
        is_active: true,
        account: Some("dev@example.com".to_string()),
        project: Some("proj".to_string()),
        region: None,
        zone: None,
    }
    .context()
}

fn instance(name: &str, status: ResourceStatus) -> ComputeInstance {
    ComputeInstance {
        id: name.to_string(),
        name: name.to_string(),
        zone: "z".to_string(),
        status,
        machine_type: "e2".to_string(),
        internal_ip: None,
        external_ip: None,
    }
}

#[test]
fn compute_defaults_are_not_loaded() {
    let state = ResourcesState::default();
    assert!(!state.compute_loaded());
    assert!(!state.compute_stale());
    assert!(state.compute_status_counts().is_empty());
    assert!(state.compute_problem().is_none());
}

#[test]
fn compute_status_counts_group_by_normalized_status() {
    let mut state = ResourcesState::default();
    state.set_instances(
        context(),
        vec![
            instance("a", ResourceStatus::Running),
            instance("b", ResourceStatus::Running),
            instance("c", ResourceStatus::Stopped),
            instance("d", ResourceStatus::Unknown("PROVISIONING".to_string())),
        ],
    );
    assert!(state.compute_loaded());
    let counts = state.compute_status_counts();
    assert!(counts.contains(&("Running".to_string(), 2)));
    assert!(counts.contains(&("Stopped".to_string(), 1)));
    // An unrecognised status is counted under the label its row paints.
    assert!(counts.contains(&("Provisioning".to_string(), 1)));
}

#[test]
fn service_problem_is_stored_and_cleared_on_load() {
    let mut state = ResourcesState::default();
    state.set_service_problem(
        ResourceServiceKind::ComputeInstances,
        DiagnosticProblem::new(GcloudErrorKind::PermissionDenied, "denied"),
    );
    assert_eq!(
        state.compute_problem().unwrap().kind,
        GcloudErrorKind::PermissionDenied
    );
    state.set_instances(context(), vec![instance("a", ResourceStatus::Running)]);
    assert!(state.compute_problem().is_none());
}
