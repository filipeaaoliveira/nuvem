//! Tests for the service-agnostic batch confirmation/workflow state
//! (`BatchAction`, `BatchRowRef`, `BatchableService`). Written first (TDD RED)
//! before the types exist.
//!
//! Compute is the reference behaviour: the confirmation split and the rendered
//! modal lines must stay byte-for-byte what the GCE-only implementation
//! produced.

use nuvem::app::state::resources::{
    BatchAck, BatchAction, BatchRowRef, BatchTargetStatus, BatchableService, CloudSql,
    CloudSqlInstanceAction, Compute, ComputeInstanceAction, ResourcesState, SkipReason,
    cloud_sql_lifecycle_call, compute_lifecycle_call,
};
use nuvem::app::{AppState, Workspace};
use nuvem::services::runtime::session::RuntimeSession;
use nuvem::ui::resources::{resource_batch_confirmation_lines, resource_batch_progress_lines};
use nuvem_domain::gcp::configuration::{GcloudConfigurationName, GcloudContext};
use nuvem_domain::gcp::resource::{
    CloudSqlAvailability, CloudSqlInstance, CloudSqlStatus, ComputeInstance, EndpointVisibility,
    ResourceServiceKind, ResourceStatus,
};
use ratatui::{Terminal, backend::TestBackend};

/// Pairs targets with stand-in operations-log ids for tests that drive workflow
/// state directly, with no operations log behind it. The ids start well above
/// zero so they can never coincide with a target's index: a positional bug
/// would produce `0, 1, 2…`, and these helpers must be inert by construction
/// rather than by coincidence.
fn with_stand_in_ids(targets: Vec<BatchRowRef>) -> Vec<(BatchRowRef, u64)> {
    targets
        .into_iter()
        .enumerate()
        .map(|(index, target)| (target, 900 + index as u64))
        .collect()
}

fn ctx() -> GcloudContext {
    GcloudContext {
        configuration: GcloudConfigurationName::new("dev"),
        account: None,
        project: Some("dev-project".to_string()),
        region: None,
        zone: None,
    }
}

fn instance(name: &str, status: ResourceStatus) -> ComputeInstance {
    ComputeInstance {
        id: format!("{name}-id"),
        name: name.to_string(),
        zone: "europe-west1-b".to_string(),
        status,
        machine_type: "e2-medium".to_string(),
        internal_ip: None,
        external_ip: None,
    }
}

/// Loads `instances` under the dev context and marks every one of them.
fn state_with_all_marked(instances: Vec<ComputeInstance>) -> ResourcesState {
    let count = instances.len();
    let mut state = ResourcesState::default();
    state.set_instances(ctx(), instances);
    if count > 0 {
        state.toggle_mark();
        for _ in 1..count {
            state.select_next();
            state.toggle_mark();
        }
    }
    assert_eq!(state.marked_count(), count);
    state
}

fn sql_instance(
    name: &str,
    status: CloudSqlStatus,
    availability: CloudSqlAvailability,
) -> CloudSqlInstance {
    CloudSqlInstance {
        name: name.to_string(),
        database_version: "POSTGRES_15".to_string(),
        engine: "PostgreSQL".to_string(),
        region: "europe-west1".to_string(),
        tier: "db-custom-2-7680".to_string(),
        availability,
        status,
        connectivity: EndpointVisibility::Private,
    }
}

/// Loads `instances` under the dev context, selects Cloud SQL, and marks every
/// one of them.
fn cloud_sql_state_with_all_marked(instances: Vec<CloudSqlInstance>) -> ResourcesState {
    let count = instances.len();
    let mut state = ResourcesState::default();
    state.select_service(ResourceServiceKind::CloudSql);
    state.set_cloud_sql_instances(ctx(), instances);
    if count > 0 {
        state.toggle_mark();
        for _ in 1..count {
            state.select_next();
            state.toggle_mark();
        }
    }
    assert_eq!(state.marked_count(), count);
    state
}

#[test]
fn cloud_sql_stop_restart_and_failover_are_destructive_but_start_is_not() {
    assert!(CloudSqlInstanceAction::Stop.is_destructive());
    assert!(CloudSqlInstanceAction::Restart.is_destructive());
    assert!(CloudSqlInstanceAction::Failover.is_destructive());
    assert!(!CloudSqlInstanceAction::Start.is_destructive());
}

#[test]
fn batch_action_titles_name_the_service_they_target() {
    assert_eq!(
        BatchAction::Compute(ComputeInstanceAction::Stop).title(),
        "Confirm GCE batch action"
    );
    assert_eq!(
        BatchAction::CloudSql(CloudSqlInstanceAction::Stop).title(),
        "Confirm Cloud SQL batch action"
    );
}

#[test]
fn batch_action_delegates_label_destructiveness_and_service_kind() {
    let compute = BatchAction::Compute(ComputeInstanceAction::Stop);
    assert_eq!(compute.label(), ComputeInstanceAction::Stop.label());
    assert!(compute.is_destructive());
    assert_eq!(
        compute.service_kind(),
        ResourceServiceKind::ComputeInstances
    );

    let start = BatchAction::Compute(ComputeInstanceAction::Start);
    assert!(!start.is_destructive());

    let cloud_sql = BatchAction::CloudSql(CloudSqlInstanceAction::Restart);
    assert_eq!(cloud_sql.label(), CloudSqlInstanceAction::Restart.label());
    assert!(cloud_sql.is_destructive());
    assert_eq!(cloud_sql.service_kind(), ResourceServiceKind::CloudSql);
}

#[test]
fn compute_wraps_its_action_and_projects_a_row_ref() {
    let row = instance("api", ResourceStatus::Running);
    assert_eq!(
        Compute::wrap(ComputeInstanceAction::Stop),
        BatchAction::Compute(ComputeInstanceAction::Stop)
    );
    assert!(Compute::is_eligible(&row, ComputeInstanceAction::Stop));
    assert!(!Compute::is_eligible(&row, ComputeInstanceAction::Start));

    let row_ref = Compute::row_ref(&row, &ctx(), ComputeInstanceAction::Stop);
    assert_eq!(
        row_ref,
        BatchRowRef {
            name: "api".to_string(),
            location: "europe-west1-b".to_string(),
            status_label: "Running".to_string(),
            call: compute_lifecycle_call(
                &ctx(),
                "api",
                "europe-west1-b",
                ComputeInstanceAction::Stop
            ),
        }
    );
}

#[test]
fn compute_row_ref_status_label_renders_unknown_as_the_table_does() {
    let row = instance("api", ResourceStatus::Unknown("FOO".to_string()));
    let row_ref = Compute::row_ref(&row, &ctx(), ComputeInstanceAction::Stop);
    assert_eq!(row_ref.status_label, "Foo");
}

#[test]
fn compute_batch_confirmation_over_mixed_statuses_splits_eligible_and_skipped() {
    let mut state = state_with_all_marked(vec![
        instance("api", ResourceStatus::Running),
        instance("web", ResourceStatus::Running),
        instance("db", ResourceStatus::Stopped),
    ]);

    state.start_batch_confirmation::<Compute>(ctx(), ComputeInstanceAction::Stop);

    let confirmation = state.batch_confirmation().expect("batch confirmation open");
    assert_eq!(
        confirmation.action(),
        BatchAction::Compute(ComputeInstanceAction::Stop)
    );
    // Every marked row lands in exactly one list: two eligible, one skipped.
    let eligible: Vec<&str> = confirmation
        .eligible()
        .iter()
        .map(|target| target.name.as_str())
        .collect();
    assert_eq!(eligible, vec!["api", "web"]);
    assert_eq!(confirmation.skipped().len(), 1);
    assert_eq!(confirmation.skipped()[0].0.name, "db");
    assert_eq!(
        confirmation.skipped()[0].1,
        SkipReason::Ineligible("Stopped".to_string())
    );
    // Destructive action: typed-count acknowledgement, unsatisfied until the
    // eligible count is typed.
    assert_eq!(
        confirmation.ack(),
        &BatchAck::CountRequired {
            typed: String::new()
        }
    );
    assert!(
        !confirmation
            .ack()
            .is_satisfied(confirmation.eligible().len())
    );
}

#[test]
fn compute_batch_confirmation_lines_are_unchanged_for_a_mixed_marked_set() {
    let mut state = state_with_all_marked(vec![
        instance("api", ResourceStatus::Running),
        instance("web", ResourceStatus::Running),
        instance("db", ResourceStatus::Stopped),
    ]);
    state.start_batch_confirmation::<Compute>(ctx(), ComputeInstanceAction::Stop);
    let confirmation = state.batch_confirmation().expect("batch confirmation open");

    assert_eq!(
        resource_batch_confirmation_lines(confirmation),
        vec![
            "Confirm GCE batch action".to_string(),
            String::new(),
            "Configuration: dev".to_string(),
            "Project: dev-project".to_string(),
            "Action: Stop instance".to_string(),
            "Targets: 2 (1 skipped)".to_string(),
            String::new(),
            "api: compute.instances.stop zones/europe-west1-b/instances/api".to_string(),
            "web: compute.instances.stop zones/europe-west1-b/instances/web".to_string(),
            String::new(),
            "Skipped:".to_string(),
            "db \u{2014} ineligible (Stopped)".to_string(),
            String::new(),
            "Type 2 to confirm".to_string(),
            "Esc cancel".to_string(),
        ]
    );
}

#[test]
fn compute_batch_confirmation_is_not_opened_when_nothing_is_eligible() {
    let mut state = state_with_all_marked(vec![instance("db", ResourceStatus::Stopped)]);

    state.start_batch_confirmation::<Compute>(ctx(), ComputeInstanceAction::Stop);

    assert!(state.batch_confirmation().is_none());
}

#[test]
fn compute_batch_confirmation_enumerates_the_marked_rows_of_its_own_service() {
    // The split must enumerate `S`'s marked rows, never "whatever the active
    // service happens to be". Marks are made on Compute, then the active
    // service is switched away: a Compute batch still sees the Compute marks.
    let mut state = state_with_all_marked(vec![
        instance("api", ResourceStatus::Running),
        instance("web", ResourceStatus::Running),
    ]);
    state.select_service(ResourceServiceKind::CloudSql);
    assert_eq!(state.marked_count(), 0, "Cloud SQL has no marks");

    state.start_batch_confirmation::<Compute>(ctx(), ComputeInstanceAction::Stop);

    let confirmation = state.batch_confirmation().expect("batch confirmation open");
    let eligible: Vec<&str> = confirmation
        .eligible()
        .iter()
        .map(|target| target.name.as_str())
        .collect();
    assert_eq!(eligible, vec!["api", "web"]);
}

#[test]
fn batch_workflow_targets_carry_the_row_location_and_render_progress_lines() {
    let mut state = state_with_all_marked(vec![
        instance("api", ResourceStatus::Running),
        instance("web", ResourceStatus::Running),
    ]);
    state.start_batch_confirmation::<Compute>(ctx(), ComputeInstanceAction::Stop);
    let (action, context, targets) = state
        .batch_confirmation_submission()
        .expect("confirmation submits");
    assert_eq!(action, BatchAction::Compute(ComputeInstanceAction::Stop));
    let previews: Vec<String> = targets.iter().map(|target| target.call.preview()).collect();
    assert_eq!(
        previews,
        vec![
            "compute.instances.stop zones/europe-west1-b/instances/api".to_string(),
            "compute.instances.stop zones/europe-west1-b/instances/web".to_string(),
        ]
    );

    state.begin_batch_workflow(action, context, with_stand_in_ids(targets));
    let workflow = state.batch_workflow().expect("workflow present");
    assert_eq!(workflow.targets().len(), 2);
    assert_eq!(workflow.targets()[0].name, "api");
    assert_eq!(workflow.targets()[0].location, "europe-west1-b");
    assert_eq!(workflow.targets()[0].status, BatchTargetStatus::Pending);

    state.resolve_batch_target(
        ResourceServiceKind::ComputeInstances,
        "api",
        Some("europe-west1-b"),
        BatchTargetStatus::Succeeded,
    );
    let workflow = state.batch_workflow().expect("workflow present");
    assert_eq!(
        resource_batch_progress_lines(workflow),
        vec![
            "Batch Stop instance \u{2014} 1/2 done \u{b7} 0 failed \u{b7} 1 pending".to_string(),
            String::new(),
            "api  OK".to_string(),
            "web  PENDING".to_string(),
        ]
    );
}

/// Cancellation resolves a stranded operations-log entry by id, so each target
/// must carry the id of the entry seeded for it — and carry the *right* one.
/// The ids here are deliberately out of order and not index-shaped, so a
/// workflow that paired them positionally by accident would fail.
#[test]
fn batch_workflow_targets_carry_their_own_operations_log_id() {
    let mut state = state_with_all_marked(vec![
        instance("api", ResourceStatus::Running),
        instance("web", ResourceStatus::Running),
    ]);
    state.start_batch_confirmation::<Compute>(ctx(), ComputeInstanceAction::Stop);
    let (action, context, targets) = state
        .batch_confirmation_submission()
        .expect("confirmation submits");
    assert_eq!(
        targets
            .iter()
            .map(|target| target.name.as_str())
            .collect::<Vec<_>>(),
        vec!["api", "web"]
    );

    let paired: Vec<(BatchRowRef, u64)> = targets.into_iter().zip([41_u64, 7_u64]).collect();
    state.begin_batch_workflow(action, context, paired);

    let workflow = state.batch_workflow().expect("workflow present");
    assert_eq!(
        workflow
            .targets()
            .iter()
            .map(|target| (target.name.as_str(), target.operation_id))
            .collect::<Vec<_>>(),
        vec![("api", 41), ("web", 7)]
    );
}

/// `Some(location)` is the Compute identity: a result for the right name in the
/// wrong zone belongs to a different instance and must not resolve the target.
#[test]
fn resolving_a_batch_target_with_a_location_requires_both_name_and_location() {
    let mut state = state_with_all_marked(vec![instance("api", ResourceStatus::Running)]);
    state.start_batch_confirmation::<Compute>(ctx(), ComputeInstanceAction::Stop);
    let (action, context, targets) = state
        .batch_confirmation_submission()
        .expect("confirmation submits");
    state.begin_batch_workflow(action, context, with_stand_in_ids(targets));

    // Right name, wrong location: not this target.
    state.resolve_batch_target(
        ResourceServiceKind::ComputeInstances,
        "api",
        Some("europe-west4-a"),
        BatchTargetStatus::Succeeded,
    );
    assert_eq!(
        state.batch_workflow().expect("workflow present").targets()[0].status,
        BatchTargetStatus::Pending
    );

    state.resolve_batch_target(
        ResourceServiceKind::ComputeInstances,
        "api",
        Some("europe-west1-b"),
        BatchTargetStatus::Succeeded,
    );
    assert_eq!(
        state.batch_workflow().expect("workflow present").targets()[0].status,
        BatchTargetStatus::Succeeded
    );
}

/// A service whose action results carry no location (Cloud SQL reports only an
/// instance name) resolves by name alone; the target keeps its stored location
/// for rendering.
#[test]
fn resolving_a_batch_target_without_a_location_matches_by_name_alone() {
    let mut state = state_with_all_marked(vec![
        instance("api", ResourceStatus::Running),
        instance("web", ResourceStatus::Running),
    ]);
    state.start_batch_confirmation::<Compute>(ctx(), ComputeInstanceAction::Stop);
    let (action, context, targets) = state
        .batch_confirmation_submission()
        .expect("confirmation submits");
    state.begin_batch_workflow(action, context, with_stand_in_ids(targets));

    state.resolve_batch_target(
        ResourceServiceKind::ComputeInstances,
        "web",
        None,
        BatchTargetStatus::Succeeded,
    );

    let workflow = state.batch_workflow().expect("workflow present");
    assert_eq!(workflow.targets()[0].status, BatchTargetStatus::Pending);
    assert_eq!(workflow.targets()[1].status, BatchTargetStatus::Succeeded);
    assert_eq!(workflow.targets()[1].location, "europe-west1-b");
}

#[test]
fn cloud_sql_wraps_its_action_and_projects_a_row_ref() {
    let row = sql_instance(
        "orders",
        CloudSqlStatus::Runnable,
        CloudSqlAvailability::Regional,
    );
    assert_eq!(
        CloudSql::wrap(CloudSqlInstanceAction::Stop),
        BatchAction::CloudSql(CloudSqlInstanceAction::Stop)
    );
    assert!(CloudSql::is_eligible(&row, CloudSqlInstanceAction::Stop));
    assert!(!CloudSql::is_eligible(&row, CloudSqlInstanceAction::Start));

    let row_ref = CloudSql::row_ref(&row, &ctx(), CloudSqlInstanceAction::Stop);
    assert_eq!(
        row_ref,
        BatchRowRef {
            name: "orders".to_string(),
            // Cloud SQL's target location is its region.
            location: "europe-west1".to_string(),
            status_label: "Runnable".to_string(),
            call: cloud_sql_lifecycle_call(&ctx(), "orders", CloudSqlInstanceAction::Stop),
        }
    );
}

/// `is_eligible` must consult availability, not only status: `Failover` on a
/// Runnable but zonal (non-HA) instance is ineligible.
#[test]
fn cloud_sql_eligibility_consults_availability_for_failover() {
    let ha = sql_instance(
        "orders",
        CloudSqlStatus::Runnable,
        CloudSqlAvailability::Regional,
    );
    let zonal = sql_instance(
        "reports",
        CloudSqlStatus::Runnable,
        CloudSqlAvailability::Zonal,
    );
    assert!(CloudSql::is_eligible(&ha, CloudSqlInstanceAction::Failover));
    assert!(!CloudSql::is_eligible(
        &zonal,
        CloudSqlInstanceAction::Failover
    ));
}

/// The batch preview a target carries must be exactly the string the single-row
/// confirmation shows for the same action, so the two paths cannot drift.
#[test]
fn cloud_sql_row_ref_previews_match_the_single_row_confirmation() {
    for action in [
        CloudSqlInstanceAction::Start,
        CloudSqlInstanceAction::Stop,
        CloudSqlInstanceAction::Restart,
        CloudSqlInstanceAction::Failover,
    ] {
        let row = sql_instance(
            "orders",
            CloudSqlStatus::Runnable,
            CloudSqlAvailability::Regional,
        );
        let mut state = ResourcesState::default();
        state.start_cloud_sql_action_confirmation(ctx(), row.clone(), action);
        let single = state
            .action_confirmation()
            .expect("single-row confirmation open")
            .call();

        assert_eq!(
            CloudSql::row_ref(&row, &ctx(), action).call,
            single,
            "the batch target and the single-row confirmation must name the \
             same call for {action:?}, or the two modals promise different work"
        );
    }
}

#[test]
fn cloud_sql_row_ref_previews_are_the_documented_gcloud_invocations() {
    let row = sql_instance(
        "orders",
        CloudSqlStatus::Runnable,
        CloudSqlAvailability::Regional,
    );
    let preview = |action| CloudSql::row_ref(&row, &ctx(), action).call.preview();
    assert_eq!(
        preview(CloudSqlInstanceAction::Start),
        "sql.instances.patch instances/orders activationPolicy=ALWAYS"
    );
    assert_eq!(
        preview(CloudSqlInstanceAction::Stop),
        "sql.instances.patch instances/orders activationPolicy=NEVER"
    );
    assert_eq!(
        preview(CloudSqlInstanceAction::Restart),
        "sql.instances.restart instances/orders"
    );
    assert_eq!(
        preview(CloudSqlInstanceAction::Failover),
        "sql.instances.failover instances/orders"
    );
}

/// A marked Cloud SQL set plus a destructive action asks for a typed count, and
/// every marked row lands in exactly one of eligible/skipped — the ineligible
/// one carrying its reason rather than being silently dropped.
#[test]
fn cloud_sql_batch_confirmation_splits_eligible_and_skipped_and_requires_a_count() {
    let mut state = cloud_sql_state_with_all_marked(vec![
        sql_instance(
            "orders",
            CloudSqlStatus::Runnable,
            CloudSqlAvailability::Regional,
        ),
        sql_instance(
            "reports",
            CloudSqlStatus::Runnable,
            CloudSqlAvailability::Zonal,
        ),
        sql_instance(
            "archive",
            CloudSqlStatus::Stopped,
            CloudSqlAvailability::Regional,
        ),
    ]);

    state.start_batch_confirmation::<CloudSql>(ctx(), CloudSqlInstanceAction::Failover);

    let confirmation = state.batch_confirmation().expect("batch confirmation open");
    assert_eq!(
        confirmation.action(),
        BatchAction::CloudSql(CloudSqlInstanceAction::Failover)
    );
    let eligible: Vec<&str> = confirmation
        .eligible()
        .iter()
        .map(|target| target.name.as_str())
        .collect();
    assert_eq!(eligible, vec!["orders"]);
    // Both ineligible rows are reported, neither dropped.
    let skipped: Vec<(&str, &SkipReason)> = confirmation
        .skipped()
        .iter()
        .map(|(target, reason)| (target.name.as_str(), reason))
        .collect();
    assert_eq!(
        skipped,
        vec![
            ("archive", &SkipReason::Ineligible("Stopped".to_string())),
            // Runnable but zonal: ineligible for failover on availability, not
            // status, so the reason names the HA requirement instead of a
            // status that reads as eligible.
            ("reports", &SkipReason::RequiresHighAvailability),
        ]
    );
    assert_eq!(
        confirmation.eligible().len() + confirmation.skipped().len(),
        3,
        "every marked row lands in exactly one list"
    );

    // Destructive: typed-count acknowledgement, unsatisfied until typed.
    assert_eq!(
        confirmation.ack(),
        &BatchAck::CountRequired {
            typed: String::new()
        }
    );
    assert!(
        !confirmation
            .ack()
            .is_satisfied(confirmation.eligible().len())
    );
    state.batch_ack_append_digit('1');
    let confirmation = state.batch_confirmation().expect("batch confirmation open");
    assert!(
        confirmation
            .ack()
            .is_satisfied(confirmation.eligible().len())
    );
}

/// `Start` is not destructive, so a Cloud SQL start batch needs no typed count.
#[test]
fn cloud_sql_start_batch_needs_no_typed_count() {
    let mut state = cloud_sql_state_with_all_marked(vec![sql_instance(
        "archive",
        CloudSqlStatus::Stopped,
        CloudSqlAvailability::Zonal,
    )]);

    state.start_batch_confirmation::<CloudSql>(ctx(), CloudSqlInstanceAction::Start);

    let confirmation = state.batch_confirmation().expect("batch confirmation open");
    assert_eq!(confirmation.ack(), &BatchAck::Simple);
    assert!(
        confirmation
            .ack()
            .is_satisfied(confirmation.eligible().len())
    );
}

#[test]
fn cloud_sql_batch_confirmation_is_not_opened_when_nothing_is_eligible() {
    let mut state = cloud_sql_state_with_all_marked(vec![sql_instance(
        "archive",
        CloudSqlStatus::Stopped,
        CloudSqlAvailability::Zonal,
    )]);

    state.start_batch_confirmation::<CloudSql>(ctx(), CloudSqlInstanceAction::Stop);

    assert!(state.batch_confirmation().is_none());
}

/// A Cloud SQL batch workflow's targets keep their region and resolve from a
/// result that carries no region at all (matching by name alone).
#[test]
fn cloud_sql_batch_workflow_targets_resolve_by_name_alone() {
    let mut state = cloud_sql_state_with_all_marked(vec![
        sql_instance(
            "orders",
            CloudSqlStatus::Runnable,
            CloudSqlAvailability::Regional,
        ),
        sql_instance(
            "reports",
            CloudSqlStatus::Runnable,
            CloudSqlAvailability::Regional,
        ),
    ]);
    state.start_batch_confirmation::<CloudSql>(ctx(), CloudSqlInstanceAction::Restart);
    let (action, context, targets) = state
        .batch_confirmation_submission()
        .expect("confirmation submits");
    assert_eq!(
        action,
        BatchAction::CloudSql(CloudSqlInstanceAction::Restart)
    );
    let previews: Vec<String> = targets.iter().map(|target| target.call.preview()).collect();
    assert_eq!(
        previews,
        vec![
            "sql.instances.restart instances/orders".to_string(),
            "sql.instances.restart instances/reports".to_string(),
        ]
    );

    state.begin_batch_workflow(action, context, with_stand_in_ids(targets));
    state.resolve_batch_target(
        ResourceServiceKind::CloudSql,
        "reports",
        None,
        BatchTargetStatus::Succeeded,
    );

    let workflow = state.batch_workflow().expect("workflow present");
    assert_eq!(workflow.targets()[0].status, BatchTargetStatus::Pending);
    assert_eq!(workflow.targets()[1].status, BatchTargetStatus::Succeeded);
    assert_eq!(workflow.targets()[1].location, "europe-west1");
}

/// A Cloud SQL action result or load failure routes through
/// `mark_active_stale_and_clear(CloudSql)`, which empties the rows. An open
/// batch confirmation holds targets snapshotted from exactly those rows, so it
/// must not survive the teardown — Compute's `mark_stale_and_clear` already
/// closes it, and the Cloud SQL path has to match.
#[test]
fn cloud_sql_stale_and_clear_closes_an_open_batch_confirmation() {
    let mut state = cloud_sql_state_with_all_marked(vec![sql_instance(
        "orders",
        CloudSqlStatus::Runnable,
        CloudSqlAvailability::Regional,
    )]);
    state.start_batch_confirmation::<CloudSql>(ctx(), CloudSqlInstanceAction::Restart);
    assert!(state.batch_confirmation().is_some(), "precondition: open");

    state.mark_active_stale_and_clear(ResourceServiceKind::CloudSql);

    assert!(
        state.cloud_sql_instances().is_empty(),
        "precondition: the rows the targets were snapshotted from are gone"
    );
    assert!(
        state.batch_confirmation().is_none(),
        "an open Cloud SQL batch confirmation must not outlive its rows"
    );
}

/// The single-row Cloud SQL confirmation is torn down by the same path, again
/// mirroring Compute.
#[test]
fn cloud_sql_stale_and_clear_closes_an_open_single_row_confirmation() {
    let mut state = ResourcesState::default();
    state.select_service(ResourceServiceKind::CloudSql);
    let row = sql_instance(
        "orders",
        CloudSqlStatus::Runnable,
        CloudSqlAvailability::Regional,
    );
    state.set_cloud_sql_instances(ctx(), vec![row.clone()]);
    state.start_cloud_sql_action_confirmation(ctx(), row, CloudSqlInstanceAction::Stop);
    assert!(state.action_confirmation().is_some(), "precondition: open");

    state.mark_active_stale_and_clear(ResourceServiceKind::CloudSql);

    assert!(
        state.action_confirmation().is_none(),
        "an open Cloud SQL action confirmation must not outlive its rows"
    );
}

/// The batch path renders an unrecognised Cloud SQL status with its value,
/// spelled the way the table shows it, so a skip line can be matched back to
/// the row it came from. This is deliberately not the status-count bucketing
/// label, which collapses every `Unknown(_)` to a bare "Unknown".
#[test]
fn cloud_sql_row_ref_status_label_renders_unknown_as_the_table_does() {
    let row = sql_instance(
        "orders",
        CloudSqlStatus::Unknown("MAINTENANCE_PENDING".to_string()),
        CloudSqlAvailability::Regional,
    );
    let row_ref = CloudSql::row_ref(&row, &ctx(), CloudSqlInstanceAction::Stop);
    assert_eq!(row_ref.status_label, "Maintenance pending");
}

/// ...and the skip panel inherits that label, so `ineligible (...)` names
/// the same status the row displays.
#[test]
fn cloud_sql_skip_reason_keeps_the_raw_unknown_status() {
    let mut state = cloud_sql_state_with_all_marked(vec![
        sql_instance(
            "orders",
            CloudSqlStatus::Runnable,
            CloudSqlAvailability::Regional,
        ),
        sql_instance(
            "reports",
            CloudSqlStatus::Unknown("MAINTENANCE_PENDING".to_string()),
            CloudSqlAvailability::Regional,
        ),
    ]);

    state.start_batch_confirmation::<CloudSql>(ctx(), CloudSqlInstanceAction::Stop);

    let confirmation = state.batch_confirmation().expect("batch confirmation open");
    let skipped: Vec<(&str, &SkipReason)> = confirmation
        .skipped()
        .iter()
        .map(|(target, reason)| (target.name.as_str(), reason))
        .collect();
    assert_eq!(
        skipped,
        vec![(
            "reports",
            &SkipReason::Ineligible("Maintenance pending".to_string())
        )]
    );
}

/// Renders the real batch confirmation overlay at `width`x`height` and returns
/// its terminal buffer as trimmed lines, so a test can assert on the ratatui
/// border title and body exactly as an operator sees them.
fn rendered_batch_modal_at(resources: ResourcesState, width: u16, height: u16) -> Vec<String> {
    let mut app = AppState::new();
    app.root.workspace = Workspace::Resources;
    app.root.resources = resources;
    let session = RuntimeSession::new_for_test(app);
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).expect("terminal");
    terminal
        .draw(|frame| nuvem::ui::render(frame, &session))
        .expect("render");
    let buffer = terminal.backend().buffer();
    let width = buffer.area.width as usize;
    buffer
        .content()
        .chunks(width)
        .map(|line| {
            line.iter()
                .map(|cell| cell.symbol())
                .collect::<String>()
                .trim_end()
                .to_string()
        })
        .collect()
}

/// Renders the batch confirmation overlay at a size that comfortably fits it.
fn rendered_batch_modal(resources: ResourcesState) -> Vec<String> {
    rendered_batch_modal_at(resources, 140, 40)
}

/// Renders the batch confirmation overlay at the app's own minimum supported
/// terminal size (`ui::HELP_MIN_WIDTH`x`HELP_MIN_HEIGHT`), which is too small
/// for the modal preview and so takes the fallback path.
fn rendered_small_terminal_batch_modal(resources: ResourcesState) -> Vec<String> {
    rendered_batch_modal_at(resources, 60, 12)
}

/// The batch modal's border title names the service the batch targets. GCE
/// keeps exactly the title it has always rendered; a Cloud SQL batch says so
/// instead of claiming to be a GCE batch.
#[test]
fn batch_modal_border_title_names_the_service_the_batch_targets() {
    let mut compute = state_with_all_marked(vec![
        instance("api", ResourceStatus::Running),
        instance("web", ResourceStatus::Running),
    ]);
    compute.start_batch_confirmation::<Compute>(ctx(), ComputeInstanceAction::Stop);
    let compute_lines = rendered_batch_modal(compute).join("\n");
    assert!(
        compute_lines.contains("confirm GCE batch"),
        "GCE batch modal keeps its border title, got:\n{compute_lines}"
    );

    let mut cloud_sql = cloud_sql_state_with_all_marked(vec![
        sql_instance(
            "orders",
            CloudSqlStatus::Runnable,
            CloudSqlAvailability::Regional,
        ),
        sql_instance(
            "reports",
            CloudSqlStatus::Runnable,
            CloudSqlAvailability::Regional,
        ),
    ]);
    cloud_sql.start_batch_confirmation::<CloudSql>(ctx(), CloudSqlInstanceAction::Restart);
    let cloud_sql_lines = rendered_batch_modal(cloud_sql).join("\n");
    assert!(
        cloud_sql_lines.contains("confirm Cloud SQL batch"),
        "Cloud SQL batch modal names Cloud SQL, got:\n{cloud_sql_lines}"
    );
    assert!(
        !cloud_sql_lines.contains("confirm GCE batch"),
        "Cloud SQL batch modal must not claim to be a GCE batch, got:\n{cloud_sql_lines}"
    );
}

/// A zonal (non-HA) instance is skipped for `Failover` because of its
/// availability, not its status — reporting `ineligible (Runnable)` reads as if
/// an eligible row was dropped. The skip line must name the HA requirement,
/// while a row that really is status-ineligible still reports its status.
#[test]
fn zonal_cloud_sql_row_skipped_for_failover_explains_the_ha_requirement() {
    let mut state = cloud_sql_state_with_all_marked(vec![
        sql_instance(
            "orders",
            CloudSqlStatus::Runnable,
            CloudSqlAvailability::Regional,
        ),
        sql_instance(
            "reports",
            CloudSqlStatus::Runnable,
            CloudSqlAvailability::Zonal,
        ),
        sql_instance(
            "archive",
            CloudSqlStatus::Stopped,
            CloudSqlAvailability::Zonal,
        ),
    ]);

    state.start_batch_confirmation::<CloudSql>(ctx(), CloudSqlInstanceAction::Failover);

    let confirmation = state.batch_confirmation().expect("batch confirmation open");
    let lines = resource_batch_confirmation_lines(confirmation);
    let reports = lines
        .iter()
        .find(|line| line.starts_with("reports"))
        .expect("skipped zonal row is reported");
    assert!(
        reports.contains("HA"),
        "a zonal row skipped for failover names the HA requirement, got: {reports}"
    );
    assert!(
        !reports.contains("ineligible (Runnable)"),
        "a zonal row must not be reported by a status that reads as eligible, got: {reports}"
    );

    // A genuinely status-ineligible row still reports its status.
    let archive = lines
        .iter()
        .find(|line| line.starts_with("archive"))
        .expect("skipped stopped row is reported");
    assert!(
        archive.contains("ineligible (Stopped)"),
        "a status-ineligible row still reports its status, got: {archive}"
    );
}

/// When the terminal is too small for the batch modal, the fallback message
/// names the service the batch targets, so it agrees with the border title
/// drawn around it. GCE keeps exactly the message it has always rendered.
#[test]
fn small_terminal_batch_fallback_names_the_service_the_batch_targets() {
    let mut compute = state_with_all_marked(vec![
        instance("api", ResourceStatus::Running),
        instance("web", ResourceStatus::Running),
    ]);
    compute.start_batch_confirmation::<Compute>(ctx(), ComputeInstanceAction::Stop);
    let compute_lines = rendered_small_terminal_batch_modal(compute).join("\n");
    assert!(
        compute_lines.contains("nuvem GCE batch confirmation needs a larger terminal"),
        "GCE fallback keeps its message, got:\n{compute_lines}"
    );
    assert!(
        compute_lines.contains("confirm GCE batch"),
        "GCE fallback keeps its border title, got:\n{compute_lines}"
    );

    let mut cloud_sql = cloud_sql_state_with_all_marked(vec![
        sql_instance(
            "orders",
            CloudSqlStatus::Runnable,
            CloudSqlAvailability::Regional,
        ),
        sql_instance(
            "reports",
            CloudSqlStatus::Runnable,
            CloudSqlAvailability::Regional,
        ),
    ]);
    cloud_sql.start_batch_confirmation::<CloudSql>(ctx(), CloudSqlInstanceAction::Restart);
    let cloud_sql_lines = rendered_small_terminal_batch_modal(cloud_sql).join("\n");
    assert!(
        cloud_sql_lines.contains("nuvem Cloud SQL batch confirmation needs a larger terminal"),
        "Cloud SQL fallback names Cloud SQL, got:\n{cloud_sql_lines}"
    );
    assert!(
        !cloud_sql_lines.contains("nuvem GCE batch confirmation"),
        "Cloud SQL fallback must not claim to be a GCE batch, got:\n{cloud_sql_lines}"
    );
}

/// The batch workflow is shared across services, so a target is only resolved
/// by a result from the service that owns the workflow. A Cloud SQL result for
/// an identically-named instance must leave a Compute batch's target `Pending`
/// — matching on the name (and, for Cloud SQL, the name alone) is not enough.
#[test]
fn a_result_from_another_service_never_resolves_a_batch_target() {
    let mut state = state_with_all_marked(vec![instance("orders", ResourceStatus::Running)]);
    state.start_batch_confirmation::<Compute>(ctx(), ComputeInstanceAction::Stop);
    let (action, context, targets) = state
        .batch_confirmation_submission()
        .expect("confirmation submits");
    state.begin_batch_workflow(action, context, with_stand_in_ids(targets));

    // A Cloud SQL result naming the same instance: not this workflow's target.
    state.resolve_batch_target(
        ResourceServiceKind::CloudSql,
        "orders",
        None,
        BatchTargetStatus::Succeeded,
    );
    assert_eq!(
        state.batch_workflow().expect("workflow present").targets()[0].status,
        BatchTargetStatus::Pending,
        "a Cloud SQL result must not resolve a Compute batch target"
    );

    // The owning service's result still resolves it.
    state.resolve_batch_target(
        ResourceServiceKind::ComputeInstances,
        "orders",
        Some("europe-west1-b"),
        BatchTargetStatus::Succeeded,
    );
    assert_eq!(
        state.batch_workflow().expect("workflow present").targets()[0].status,
        BatchTargetStatus::Succeeded
    );
}

/// The confirmation fields are shared across services, so a Cloud SQL
/// stale/clear must only close a confirmation Cloud SQL opened: a Compute batch
/// confirmation snapshots Compute rows, which a Cloud SQL teardown leaves
/// untouched.
#[test]
fn cloud_sql_stale_and_clear_leaves_a_compute_confirmation_open() {
    let mut state = state_with_all_marked(vec![instance("orders", ResourceStatus::Running)]);
    state.start_batch_confirmation::<Compute>(ctx(), ComputeInstanceAction::Stop);
    assert!(state.batch_confirmation().is_some(), "precondition: open");

    state.mark_active_stale_and_clear(ResourceServiceKind::CloudSql);

    assert!(
        state.batch_confirmation().is_some(),
        "a Cloud SQL teardown must not close a Compute batch confirmation"
    );

    let row = instance("orders", ResourceStatus::Running);
    state.start_action_confirmation(ctx(), row, ComputeInstanceAction::Stop);
    assert!(state.action_confirmation().is_some(), "precondition: open");

    state.mark_active_stale_and_clear(ResourceServiceKind::CloudSql);

    assert!(
        state.action_confirmation().is_some(),
        "a Cloud SQL teardown must not close a Compute action confirmation"
    );
}

/// The mirror of the above: a Compute stale/clear must not tear down a Cloud
/// SQL confirmation, whose targets were snapshotted from Cloud SQL rows that
/// the Compute teardown does not touch.
#[test]
fn compute_stale_and_clear_leaves_a_cloud_sql_confirmation_open() {
    let mut state = cloud_sql_state_with_all_marked(vec![sql_instance(
        "orders",
        CloudSqlStatus::Runnable,
        CloudSqlAvailability::Regional,
    )]);
    state.start_batch_confirmation::<CloudSql>(ctx(), CloudSqlInstanceAction::Restart);
    assert!(state.batch_confirmation().is_some(), "precondition: open");

    state.mark_active_stale_and_clear(ResourceServiceKind::ComputeInstances);

    assert!(
        state.batch_confirmation().is_some(),
        "a Compute teardown must not close a Cloud SQL batch confirmation"
    );

    let row = sql_instance(
        "orders",
        CloudSqlStatus::Runnable,
        CloudSqlAvailability::Regional,
    );
    state.start_cloud_sql_action_confirmation(ctx(), row, CloudSqlInstanceAction::Stop);
    assert!(state.action_confirmation().is_some(), "precondition: open");

    state.mark_active_stale_and_clear(ResourceServiceKind::ComputeInstances);

    assert!(
        state.action_confirmation().is_some(),
        "a Compute teardown must not close a Cloud SQL action confirmation"
    );
}

/// Renders the batch progress panel at the app's own minimum supported
/// terminal size, which is too small for the panel and so takes the fallback
/// path. Shares the render helper with the modal tests: both assert on the real
/// `ui::render` output for a `ResourcesState`.
fn rendered_small_terminal_batch_progress(resources: ResourcesState) -> Vec<String> {
    rendered_batch_modal_at(resources, 60, 12)
}

/// Confirms `resources`' open batch confirmation, leaving a running workflow so
/// the progress panel is what the frame renders.
fn run_batch(resources: &mut ResourcesState) {
    let (action, context, targets) = resources
        .batch_confirmation_submission()
        .expect("confirmation submits");
    resources.begin_batch_workflow(action, context, with_stand_in_ids(targets));
    assert!(resources.batch_workflow().is_some(), "workflow is running");
}

/// The twin of the confirmation fallback: when the terminal is too small for
/// the *progress* panel, its message names the service whose batch is running,
/// rather than calling every batch a GCE one. GCE keeps exactly the message it
/// has always rendered.
#[test]
fn small_terminal_batch_progress_fallback_names_the_service_the_batch_targets() {
    let mut compute = state_with_all_marked(vec![
        instance("api", ResourceStatus::Running),
        instance("web", ResourceStatus::Running),
    ]);
    compute.start_batch_confirmation::<Compute>(ctx(), ComputeInstanceAction::Stop);
    run_batch(&mut compute);
    let compute_lines = rendered_small_terminal_batch_progress(compute).join("\n");
    assert!(
        compute_lines.contains("nuvem GCE batch progress needs a larger terminal"),
        "GCE progress fallback keeps its message, got:\n{compute_lines}"
    );

    let mut cloud_sql = cloud_sql_state_with_all_marked(vec![
        sql_instance(
            "orders",
            CloudSqlStatus::Runnable,
            CloudSqlAvailability::Regional,
        ),
        sql_instance(
            "reports",
            CloudSqlStatus::Runnable,
            CloudSqlAvailability::Regional,
        ),
    ]);
    cloud_sql.start_batch_confirmation::<CloudSql>(ctx(), CloudSqlInstanceAction::Restart);
    run_batch(&mut cloud_sql);
    let cloud_sql_lines = rendered_small_terminal_batch_progress(cloud_sql).join("\n");
    assert!(
        cloud_sql_lines.contains("nuvem Cloud SQL batch progress needs a larger terminal"),
        "Cloud SQL progress fallback names Cloud SQL, got:\n{cloud_sql_lines}"
    );
    assert!(
        !cloud_sql_lines.contains("nuvem GCE batch progress"),
        "Cloud SQL progress fallback must not claim to be a GCE batch, got:\n{cloud_sql_lines}"
    );
}

/// Every user-visible batch string that names a service derives from one
/// accessor, so the two services are worded consistently across the whole batch
/// surface and adding a third cannot leave one panel naming the wrong service.
#[test]
fn batch_chrome_names_its_service_consistently_across_every_surface() {
    let compute = BatchAction::Compute(ComputeInstanceAction::Stop);
    assert_eq!(compute.service_label(), "GCE");
    assert_eq!(compute.title(), "Confirm GCE batch action");
    assert_eq!(compute.modal_border_title(), "confirm GCE batch");
    assert_eq!(
        compute.small_terminal_message(),
        "nuvem GCE batch confirmation needs a larger terminal"
    );
    assert_eq!(
        compute.progress_small_terminal_message(),
        "nuvem GCE batch progress needs a larger terminal"
    );

    let cloud_sql = BatchAction::CloudSql(CloudSqlInstanceAction::Restart);
    assert_eq!(cloud_sql.service_label(), "Cloud SQL");
    assert_eq!(cloud_sql.title(), "Confirm Cloud SQL batch action");
    assert_eq!(cloud_sql.modal_border_title(), "confirm Cloud SQL batch");
    assert_eq!(
        cloud_sql.small_terminal_message(),
        "nuvem Cloud SQL batch confirmation needs a larger terminal"
    );
    assert_eq!(
        cloud_sql.progress_small_terminal_message(),
        "nuvem Cloud SQL batch progress needs a larger terminal"
    );
}

/// A row marked and then filtered out of view is still part of the batch.
///
/// This is the second loop in `marked_rows` — the one that walks the marks
/// themselves rather than the visible rows — and nothing reached it: every
/// other batch test leaves the list unfiltered, where the first loop already
/// found everything. The property matters because the two steps are separate
/// in time: an operator marks three rows, types a filter to find a fourth, and
/// the three they marked must still be what the action runs against.
#[test]
fn a_marked_row_hidden_by_a_filter_is_still_in_the_batch() {
    let mut state = state_with_all_marked(vec![
        instance("api", ResourceStatus::Running),
        instance("web", ResourceStatus::Running),
    ]);

    // Narrow the list to "web", so "api" is marked but no longer visible.
    state.start_filter();
    for character in "web".chars() {
        state.append_filter_char(character);
    }
    state.apply_filter();

    state.start_batch_confirmation::<Compute>(ctx(), ComputeInstanceAction::Stop);

    let confirmation = state.batch_confirmation().expect("batch confirmation open");
    let mut eligible: Vec<&str> = confirmation
        .eligible()
        .iter()
        .map(|target| target.name.as_str())
        .collect();
    eligible.sort_unstable();
    assert_eq!(
        eligible,
        vec!["api", "web"],
        "a mark made before the filter must survive it"
    );
}

/// And the hidden row is matched by identity, not by position: with the
/// visible row removed from the list entirely, the mark resolves against the
/// rows the collection still holds.
#[test]
fn a_hidden_marked_row_is_resolved_by_its_own_id() {
    let mut state = state_with_all_marked(vec![
        instance("api", ResourceStatus::Running),
        instance("web", ResourceStatus::Running),
    ]);

    state.start_filter();
    for character in "zzz-matches-nothing".chars() {
        state.append_filter_char(character);
    }
    state.apply_filter();

    state.start_batch_confirmation::<Compute>(ctx(), ComputeInstanceAction::Stop);

    let confirmation = state.batch_confirmation().expect("batch confirmation open");
    let mut eligible: Vec<&str> = confirmation
        .eligible()
        .iter()
        .map(|target| target.name.as_str())
        .collect();
    eligible.sort_unstable();
    assert_eq!(
        eligible,
        vec!["api", "web"],
        "with nothing visible, every marked row must come from the mark set"
    );
}

/// Every target in a batch names its own call.
///
/// The batch modal lists targets unsummarised because marks can be hidden by
/// an active filter, so the list is the only thing between an operator and
/// acting on rows they cannot see. That argument applies to what each line
/// says as much as to how many there are: a list of names without calls is a
/// count, not a confirmation.
#[test]
fn each_batch_target_carries_the_call_that_will_run_for_it() {
    let mut state = state_with_all_marked(vec![
        instance("checkout-api-3", ResourceStatus::Running),
        instance("orders-api-1", ResourceStatus::Running),
    ]);

    state.start_batch_confirmation::<Compute>(ctx(), ComputeInstanceAction::Stop);

    let confirmation = state.batch_confirmation().expect("batch confirmation open");
    let targets = confirmation.eligible();
    assert_eq!(
        targets.len(),
        2,
        "both rows are running, so both are eligible"
    );

    for (target, name) in targets.iter().zip(["checkout-api-3", "orders-api-1"]) {
        assert!(
            target
                .call
                .resource
                .ends_with(&format!("/instances/{name}")),
            "the call names this target's own instance: {:?}",
            target.call.resource
        );
        assert_eq!(target.call.method, "instances.stop");
    }
    assert_ne!(
        targets[0].call, targets[1].call,
        "two targets must not share a call: one of them would be confirming \
         the other's action"
    );
}
