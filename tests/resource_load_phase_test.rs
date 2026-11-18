//! Tests for `ResourceLoadPhase`, the per-collection in-flight/failure
//! channel that lets the UI distinguish "loading" from "not loaded" without
//! touching the existing (unrelated) error channels such as `detail_error`
//! or `last_diagnostic`. Written first (TDD RED) before the enum, field, and
//! `ResourcesState` helpers exist. Follows the fixture/style conventions of
//! `tests/resource_service_trait_test.rs`.

use nuvem::app::state::resources::{ResourceLoadPhase, ResourceLoadTicket, ResourcesState};
use nuvem_domain::gcp::configuration::{GcloudConfigurationName, GcloudContext};
use nuvem_domain::gcp::resource::{
    CloudRunService, CloudRunServiceStatus, CloudSqlAvailability, CloudSqlInstance, CloudSqlStatus,
    ComputeInstance, EndpointVisibility, GkeCluster, GkeClusterStatus, ResourceServiceKind,
    ResourceStatus, StorageBucket,
};
use std::time::Instant;

fn ctx() -> GcloudContext {
    GcloudContext {
        configuration: GcloudConfigurationName::new("dev"),
        account: None,
        project: Some("dev-project".into()),
        region: None,
        zone: None,
    }
}

fn other_ctx() -> GcloudContext {
    GcloudContext {
        configuration: GcloudConfigurationName::new("prod"),
        account: None,
        project: Some("prod-project".into()),
        region: None,
        zone: None,
    }
}

fn instance(name: &str) -> ComputeInstance {
    ComputeInstance {
        id: format!("{name}-id"),
        name: name.into(),
        zone: "europe-west1-b".into(),
        status: ResourceStatus::Running,
        machine_type: "e2-medium".into(),
        internal_ip: None,
        external_ip: None,
    }
}

fn cluster(name: &str) -> GkeCluster {
    GkeCluster {
        name: name.into(),
        location: "europe-west1".into(),
        status: GkeClusterStatus::Running,
        current_master_version: Some("1.30".into()),
        current_node_count: Some(3),
        endpoint_visibility: EndpointVisibility::Public,
    }
}

fn sql_instance(name: &str) -> CloudSqlInstance {
    CloudSqlInstance {
        name: name.into(),
        database_version: "POSTGRES_15".into(),
        engine: "PostgreSQL".into(),
        region: "europe-west1".into(),
        tier: "db-custom-1-3840".into(),
        availability: CloudSqlAvailability::Zonal,
        status: CloudSqlStatus::Runnable,
        connectivity: EndpointVisibility::Public,
    }
}

fn run_service(name: &str) -> CloudRunService {
    CloudRunService {
        name: name.into(),
        region: "europe-west1".into(),
        url: Some(format!("https://{name}.example.com")),
        latest_ready_revision: Some(format!("{name}-00001")),
        latest_created_revision: Some(format!("{name}-00001")),
        traffic_summary: "100% latest".into(),
        status: CloudRunServiceStatus::Ready,
        ready_message: None,
    }
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

#[test]
fn default_load_phase_is_idle_for_every_service_kind() {
    for kind in ResourceServiceKind::ALL {
        let mut state = ResourcesState::default();
        state.select_service(kind);
        assert_eq!(state.active_load_phase(), &ResourceLoadPhase::Idle);
    }
}

#[test]
fn begin_load_sets_loading_phase_for_the_given_kind_only() {
    let mut state = ResourcesState::default();
    let started = Instant::now();

    state.select_service(ResourceServiceKind::ComputeInstances);
    state.begin_load(ResourceServiceKind::ComputeInstances, started);
    assert_eq!(
        state.active_load_phase(),
        &ResourceLoadPhase::Loading {
            started_at: started
        }
    );

    // Every other kind is untouched and stays Idle.
    for other in [
        ResourceServiceKind::GkeClusters,
        ResourceServiceKind::CloudSql,
        ResourceServiceKind::CloudRunServices,
        ResourceServiceKind::CloudStorage,
    ] {
        state.select_service(other);
        assert_eq!(
            state.active_load_phase(),
            &ResourceLoadPhase::Idle,
            "expected {other:?} to remain Idle"
        );
    }
}

#[test]
fn fail_load_sets_failed_phase_with_the_message() {
    let mut state = ResourcesState::default();
    state.select_service(ResourceServiceKind::CloudSql);
    state.fail_load(ResourceServiceKind::CloudSql, ctx(), "boom".to_string());
    assert_eq!(
        state.active_load_phase(),
        &ResourceLoadPhase::Failed {
            message: "boom".to_string(),
            context: ctx(),
        }
    );
}

/// The context a failure belongs to is carried *inside* the `Failed`
/// variant rather than in a second field beside `load_phase`, so the phase and
/// the context it was recorded under cannot drift apart.
#[test]
fn fail_load_carries_the_failing_context_inside_the_failed_variant() {
    let mut state = ResourcesState::default();
    state.select_service(ResourceServiceKind::CloudSql);

    state.fail_load(ResourceServiceKind::CloudSql, ctx(), "boom".to_string());

    assert_eq!(
        state.active_load_phase(),
        &ResourceLoadPhase::Failed {
            message: "boom".to_string(),
            context: ctx(),
        }
    );
}

/// Folding the context into the variant makes it part of the variant's
/// identity: two failures carrying the same message but recorded under
/// different configurations are different phases. Nothing may rely on them
/// comparing equal.
#[test]
fn failed_phases_with_the_same_message_differ_across_contexts() {
    let mut here = ResourcesState::default();
    here.select_service(ResourceServiceKind::CloudSql);
    here.fail_load(ResourceServiceKind::CloudSql, ctx(), "boom".to_string());

    let mut there = ResourcesState::default();
    there.select_service(ResourceServiceKind::CloudSql);
    there.fail_load(
        ResourceServiceKind::CloudSql,
        other_ctx(),
        "boom".to_string(),
    );

    assert_ne!(here.active_load_phase(), there.active_load_phase());
    assert!(here.list_load_failed_for(ResourceServiceKind::CloudSql, &ctx()));
    assert!(!here.list_load_failed_for(ResourceServiceKind::CloudSql, &other_ctx()));
}

#[test]
fn successful_compute_load_resets_phase_to_idle() {
    let mut state = ResourcesState::default();
    state.select_service(ResourceServiceKind::ComputeInstances);
    state.begin_load(ResourceServiceKind::ComputeInstances, Instant::now());

    state.set_instances(ctx(), vec![instance("a")]);

    assert_eq!(state.active_load_phase(), &ResourceLoadPhase::Idle);
}

#[test]
fn successful_gke_load_resets_phase_to_idle() {
    let mut state = ResourcesState::default();
    state.select_service(ResourceServiceKind::GkeClusters);
    state.begin_load(ResourceServiceKind::GkeClusters, Instant::now());

    state.set_gke_clusters(ctx(), vec![cluster("a")]);

    assert_eq!(state.active_load_phase(), &ResourceLoadPhase::Idle);
}

#[test]
fn successful_cloud_sql_load_resets_phase_to_idle() {
    let mut state = ResourcesState::default();
    state.select_service(ResourceServiceKind::CloudSql);
    state.begin_load(ResourceServiceKind::CloudSql, Instant::now());

    state.set_cloud_sql_instances(ctx(), vec![sql_instance("a")]);

    assert_eq!(state.active_load_phase(), &ResourceLoadPhase::Idle);
}

#[test]
fn successful_cloud_run_load_resets_phase_to_idle() {
    let mut state = ResourcesState::default();
    state.select_service(ResourceServiceKind::CloudRunServices);
    state.begin_load(ResourceServiceKind::CloudRunServices, Instant::now());

    state.set_cloud_run_services(ctx(), vec![run_service("a")]);

    assert_eq!(state.active_load_phase(), &ResourceLoadPhase::Idle);
}

#[test]
fn storage_begin_load_sets_loading_phase() {
    let mut state = ResourcesState::default();
    let started = Instant::now();
    state.select_service(ResourceServiceKind::CloudStorage);

    state.begin_load(ResourceServiceKind::CloudStorage, started);

    assert_eq!(
        state.active_load_phase(),
        &ResourceLoadPhase::Loading {
            started_at: started
        }
    );
}

#[test]
fn storage_fail_load_sets_failed_phase_with_the_message() {
    let mut state = ResourcesState::default();
    state.select_service(ResourceServiceKind::CloudStorage);

    state.fail_load(
        ResourceServiceKind::CloudStorage,
        ctx(),
        "bucket list failed".to_string(),
    );

    assert_eq!(
        state.active_load_phase(),
        &ResourceLoadPhase::Failed {
            message: "bucket list failed".to_string(),
            context: ctx(),
        }
    );
}

#[test]
fn storage_successful_bucket_load_resets_phase_to_idle() {
    let mut state = ResourcesState::default();
    state.select_service(ResourceServiceKind::CloudStorage);
    state.begin_load(ResourceServiceKind::CloudStorage, Instant::now());

    state
        .storage_mut()
        .set_buckets(ctx(), vec![storage_bucket("a")]);

    assert_eq!(state.active_load_phase(), &ResourceLoadPhase::Idle);
}

// ---------------------------------------------------------------------------
// `abandon_load`: resolving the load phase of a detached in-flight
// fetch whose result the runtime's generation guard dropped. Identity is the
// ticket minted by `begin_load`, so a late abandon can never clobber a newer
// load.
// ---------------------------------------------------------------------------

/// A dropped fetch's own ticket resolves its still-current `Loading` phase,
/// so the spinner stops instead of ticking forever.
#[test]
fn abandon_load_with_its_own_ticket_resets_phase_to_idle() {
    for kind in ResourceServiceKind::ALL {
        let mut state = ResourcesState::default();
        state.select_service(kind);
        let ticket = state.begin_load(kind, Instant::now());

        state.abandon_load(ticket);

        assert_eq!(
            state.active_load_phase(),
            &ResourceLoadPhase::Idle,
            "expected {kind:?} to be resolved to Idle by its own ticket"
        );
    }
}

/// The ordering guarantee: a newer `begin_load` for the same service takes the
/// phase over, and the older (stale) ticket must not clear it.
#[test]
fn abandon_load_with_a_superseded_ticket_leaves_the_newer_loading_phase() {
    let mut state = ResourcesState::default();
    state.select_service(ResourceServiceKind::ComputeInstances);
    let stale_ticket = state.begin_load(ResourceServiceKind::ComputeInstances, Instant::now());
    let newer_started = Instant::now();
    let _newer_ticket = state.begin_load(ResourceServiceKind::ComputeInstances, newer_started);

    state.abandon_load(stale_ticket);

    assert_eq!(
        state.active_load_phase(),
        &ResourceLoadPhase::Loading {
            started_at: newer_started
        }
    );
}

/// A real, still-valid error must survive a late abandon.
#[test]
fn abandon_load_does_not_clear_a_failed_phase() {
    let mut state = ResourcesState::default();
    state.select_service(ResourceServiceKind::GkeClusters);
    let ticket = state.begin_load(ResourceServiceKind::GkeClusters, Instant::now());
    state.fail_load(ResourceServiceKind::GkeClusters, ctx(), "boom".to_string());

    state.abandon_load(ticket);

    assert_eq!(
        state.active_load_phase(),
        &ResourceLoadPhase::Failed {
            message: "boom".to_string(),
            context: ctx(),
        }
    );
}

/// The detached-load hazard in sequence: a load resolves (rows land, phase back to
/// `Idle`), a fresh fetch starts, and only then does the first ticket come back
/// — from a late drop or a double resolution. It must not clear the fresh
/// fetch's `Loading` phase.
#[test]
fn abandon_load_after_its_result_landed_leaves_a_fresh_load_alone() {
    let mut state = ResourcesState::default();
    state.select_service(ResourceServiceKind::CloudSql);
    let ticket = state.begin_load(ResourceServiceKind::CloudSql, Instant::now());
    state.set_cloud_sql_instances(ctx(), vec![sql_instance("a")]);
    assert_eq!(state.active_load_phase(), &ResourceLoadPhase::Idle);
    let fresh_started = Instant::now();
    state.begin_load(ResourceServiceKind::CloudSql, fresh_started);

    state.abandon_load(ticket);

    assert_eq!(
        state.active_load_phase(),
        &ResourceLoadPhase::Loading {
            started_at: fresh_started
        }
    );
}

/// A ticket resolves only the service it was minted for; concurrent loads on
/// other services keep spinning.
#[test]
fn abandon_load_only_affects_its_own_service_kind() {
    let mut state = ResourcesState::default();
    let compute_ticket = state.begin_load(ResourceServiceKind::ComputeInstances, Instant::now());
    let others = [
        ResourceServiceKind::GkeClusters,
        ResourceServiceKind::CloudSql,
        ResourceServiceKind::CloudRunServices,
        ResourceServiceKind::CloudStorage,
    ];
    let started = Instant::now();
    for other in others {
        state.begin_load(other, started);
    }

    state.abandon_load(compute_ticket);

    assert_eq!(
        state.load_phase(ResourceServiceKind::ComputeInstances),
        &ResourceLoadPhase::Idle
    );
    for other in others {
        assert_eq!(
            state.load_phase(other),
            &ResourceLoadPhase::Loading {
                started_at: started
            },
            "expected {other:?} to stay Loading"
        );
    }
}

/// Identity is a monotonic counter, not the `Instant`: two loads minted inside
/// the clock's resolution must still be distinguishable, and tickets never
/// repeat across services either.
#[test]
fn load_tickets_are_distinct_across_services_and_successive_loads() {
    let mut state = ResourcesState::default();
    let started = Instant::now();
    let mut tickets = Vec::new();
    for kind in ResourceServiceKind::ALL {
        tickets.push(state.begin_load(kind, started));
        tickets.push(state.begin_load(kind, started));
    }

    for (index, ticket) in tickets.iter().enumerate() {
        for other in &tickets[index + 1..] {
            assert_ne!(ticket, other, "tickets must be unique: {ticket:?}");
        }
    }
}

/// Task 2 moves a ticket into a spawned task and sends it back over a channel,
/// so it must be plain `Copy` data with no borrows. Compile-time assertion: no
/// task is spawned here, only the `Send + 'static` bound such a task needs.
#[test]
fn load_ticket_is_copy_and_send_static() {
    fn assert_send_static<T: Send + 'static>(_value: T) {}

    let mut state = ResourcesState::default();
    let ticket: ResourceLoadTicket =
        state.begin_load(ResourceServiceKind::CloudRunServices, Instant::now());
    let copied = ticket;
    assert_eq!(copied, ticket);
    assert_send_static(ticket);
}
