//! Property test for the operations-log settlement invariant.
//!
//! The lineage here is five example-shaped fixes for the
//! same underlying contract, each pinning one specific stranding path after it
//! shipped. This test states the contract once and lets proptest hunt for the
//! next path: for *any* interleaving of single-row actions, batch
//! confirmations, configuration switches, and effect results — in any order,
//! with any success/failure mix — every operations-log entry the reducer mints
//! is settled once the result carrying its id has been processed. No entry
//! reads `Pending` after the outstanding results drain.
//!
//! The reducer side of the contract has two halves this exercises directly:
//! an entry's id travels on exactly one effect, and the result arms settle by
//! that id *before* any current-context check (the class of bug is a
//! settle guarded by a context the session has since left). Runtime-side
//! settlement of cancelled batches is pinned separately in
//! `batch_supersession_evidence.rs` and `batch_cancel_evidence.rs`.

use proptest::prelude::*;

use nuvem::app::reducer::root::reduce;
use nuvem::app::state::operations::OperationOutcome;
use nuvem::app::state::resources::{
    BatchAction, CloudSqlInstanceAction, Compute, ComputeInstanceAction,
};
use nuvem::app::{Action, AppError, AppState, Effect, EffectResult, ResourcesAction};
use nuvem_domain::gcp::configuration::{GcloudConfiguration, GcloudConfigurationName};
use nuvem_domain::gcp::resource::{
    CloudSqlAvailability, CloudSqlInstance, CloudSqlStatus, ComputeInstance, EndpointVisibility,
    ResourceStatus,
};

fn configuration(name: &str, project: &str) -> GcloudConfiguration {
    GcloudConfiguration {
        name: GcloudConfigurationName::new(name),
        is_active: true,
        account: None,
        project: Some(project.to_string()),
        region: None,
        zone: None,
    }
}

/// The configurations a generated session hops between. Distinct projects so a
/// switch really changes the context results are checked against.
const CONFIGURATIONS: [(&str, &str); 3] = [
    ("dev", "dev-project"),
    ("staging", "staging-project"),
    ("prod", "prod-project"),
];

fn compute_instance(name: &str, status: ResourceStatus) -> ComputeInstance {
    ComputeInstance {
        id: name.to_string(),
        name: name.to_string(),
        zone: "europe-west1-b".to_string(),
        status,
        machine_type: "e2-medium".to_string(),
        internal_ip: None,
        external_ip: None,
    }
}

fn cloud_sql_instance(name: &str) -> CloudSqlInstance {
    CloudSqlInstance {
        name: name.to_string(),
        database_version: "POSTGRES_15".to_string(),
        engine: "PostgreSQL".to_string(),
        region: "europe-west1".to_string(),
        tier: "db-custom-2-7680".to_string(),
        availability: CloudSqlAvailability::Zonal,
        status: CloudSqlStatus::Runnable,
        connectivity: EndpointVisibility::Public,
    }
}

/// The widest fleet a batch step lands, and the number of `SelectPrevious`
/// steps it takes to drive the cursor back to the first row from anywhere in
/// that fleet (selection clamps, it does not wrap).
const MAX_FLEET: usize = 4;

/// One step of a generated session.
#[derive(Debug, Clone)]
enum Step {
    /// Confirm a single-row Compute `Stop` on one of the fixture instances.
    ConfirmSingleCompute { instance: usize },
    /// Confirm a single-row Cloud SQL `Restart` on one of the fixture instances.
    ConfirmSingleSql { instance: usize },
    /// Mark every row of a freshly landed Stopped fleet and confirm a `Start`
    /// batch over it. `Start` is non-destructive, so the confirmation needs no
    /// typed acknowledgement and `Confirm` submits directly.
    ConfirmComputeBatch { size: usize },
    /// Activate a different gcloud configuration mid-flight. Outstanding
    /// results then arrive for a context the session has left — the exact
    /// window those bugs lived in.
    SwitchConfiguration { config: usize },
    /// Deliver one outstanding effect result, chosen by `pick`, succeeding or
    /// failing per `fail`.
    DeliverResult { pick: usize, fail: bool },
}

fn step_strategy() -> impl Strategy<Value = Step> {
    prop_oneof![
        (0usize..3).prop_map(|instance| Step::ConfirmSingleCompute { instance }),
        (0usize..3).prop_map(|instance| Step::ConfirmSingleSql { instance }),
        (1usize..4).prop_map(|size| Step::ConfirmComputeBatch { size }),
        (0usize..CONFIGURATIONS.len()).prop_map(|config| Step::SwitchConfiguration { config }),
        ((0usize..16), any::<bool>()).prop_map(|(pick, fail)| Step::DeliverResult { pick, fail }),
    ]
}

/// An effect result the session still owes the reducer: everything needed to
/// synthesize the success or failure arm for the entry `operation_id` names.
#[derive(Debug, Clone)]
enum Outstanding {
    Compute {
        context: nuvem_domain::gcp::configuration::GcloudContext,
        name: String,
        zone: String,
        action: ComputeInstanceAction,
        operation_id: u64,
    },
    Sql {
        context: nuvem_domain::gcp::configuration::GcloudContext,
        name: String,
        action: CloudSqlInstanceAction,
        operation_id: u64,
    },
}

impl Outstanding {
    fn operation_id(&self) -> u64 {
        match self {
            Self::Compute { operation_id, .. } | Self::Sql { operation_id, .. } => *operation_id,
        }
    }
}

/// Harvests the outstanding results owed for `effects`.
///
/// The match is exhaustive with an explicit ignore list rather than a catch-all
/// arm: a new operation-carrying `Effect` variant then fails to compile here
/// instead of being silently skipped, which would leave its entries stranded
/// `Pending` with nothing to notice it by.
fn collect_outstanding(
    effects: &[Effect],
    outstanding: &mut Vec<Outstanding>,
    minted: &mut Vec<u64>,
) {
    let before = outstanding.len();
    for effect in effects {
        match effect {
            Effect::ComputeInstanceAction {
                context,
                instance,
                action,
                operation_id,
            } => outstanding.push(Outstanding::Compute {
                context: context.clone(),
                name: instance.name.clone(),
                zone: instance.zone.clone(),
                action: *action,
                operation_id: *operation_id,
            }),
            Effect::CloudSqlInstanceAction {
                context,
                instance,
                action,
                operation_id,
            } => outstanding.push(Outstanding::Sql {
                context: context.clone(),
                name: instance.name.clone(),
                action: *action,
                operation_id: *operation_id,
            }),
            Effect::RunResourceBatch {
                context,
                action,
                targets,
                ..
            } => {
                for (target, operation_id) in targets {
                    match action {
                        BatchAction::Compute(action) => outstanding.push(Outstanding::Compute {
                            context: context.clone(),
                            name: target.name.clone(),
                            zone: target.location.clone(),
                            action: *action,
                            operation_id: *operation_id,
                        }),
                        BatchAction::CloudSql(action) => outstanding.push(Outstanding::Sql {
                            context: context.clone(),
                            name: target.name.clone(),
                            action: *action,
                            operation_id: *operation_id,
                        }),
                    }
                }
            }
            // A plugin run does carry an `operation_id`, so it does not belong
            // in the ignore list below — but no step in this property confirms
            // a plugin, and the reducer emits this effect for nothing else, so
            // reaching it means the generator grew a step whose entries this
            // harvester silently stopped settling.
            Effect::RunPlugin { .. } => {
                unreachable!("no step in this property confirms a plugin")
            }
            // Everything below carries no `operation_id`, so it owes this test
            // no result. Listing them keeps the compiler as the gate.
            Effect::LoadConfigurations
            | Effect::ActivateConfiguration { .. }
            | Effect::CreateConfiguration { .. }
            | Effect::UpdateConfigurationContext { .. }
            | Effect::LoadContextPicker { .. }
            | Effect::RefreshComputeInstances { .. }
            | Effect::RefreshGkeClusters { .. }
            | Effect::LoadComputeInstanceDetail { .. }
            | Effect::LoadGkeClusterDetail { .. }
            | Effect::RefreshCloudSqlInstances { .. }
            | Effect::LoadCloudSqlInstanceDetail { .. }
            | Effect::LoadCloudSqlUsers { .. }
            | Effect::RefreshStorageBuckets { .. }
            | Effect::LoadStorageBucketDetail { .. }
            | Effect::LoadStorageObjectPage { .. }
            | Effect::LoadStorageObjectDetail { .. }
            | Effect::ProbeGcloudDiagnostics { .. }
            | Effect::CopyToClipboard { .. }
            | Effect::SaveRememberedViews { .. }
            | Effect::SaveLogEntries { .. }
            | Effect::LaunchInteractiveProcess { .. }
            | Effect::LoadRecentLogs { .. }
            | Effect::TailLogs { .. }
            | Effect::PrepareK9sLaunch { .. }
            | Effect::RunK9sCredentials { .. }
            | Effect::RefreshCloudRunServices { .. }
            | Effect::LoadCloudRunServiceDetail { .. }
            | Effect::LoadCloudRunRevisions { .. }
            | Effect::LoadCloudRunRevisionDetail { .. }
            | Effect::RefreshSecrets { .. }
            | Effect::RefreshCloudFunctions { .. }
            | Effect::RefreshPersistentDisks { .. }
            | Effect::RefreshPubSubTopics { .. }
            | Effect::RefreshPubSubSubscriptions { .. }
            | Effect::RefreshVpcNetworks { .. }
            | Effect::RefreshVpcSubnets { .. }
            | Effect::RefreshFirewallRules { .. }
            | Effect::RefreshBigQueryDatasets { .. }
            | Effect::LoadBigQueryTables { .. }
            | Effect::LoadBigQueryTableStats { .. }
            | Effect::LoadBigQuerySchema { .. }
            | Effect::RefreshServiceAccounts { .. }
            | Effect::RefreshIamPolicy { .. }
            | Effect::LoadSecretDetail { .. }
            | Effect::LoadSecretVersions { .. }
            | Effect::StartTunnel { .. }
            | Effect::StopTunnel { .. }
            | Effect::CancelRunningPlugins
            // FinOps probes mint no operation id: the grid's own ticket
            // sequence decides supersession, so there is nothing here to
            // leave stranded Pending.
            | Effect::LoadCostRecommendations { .. }
            | Effect::LoadBudgets { .. }
            | Effect::ReloadConfig => {}
        }
    }
    minted.extend(outstanding[before..].iter().map(Outstanding::operation_id));
}

fn deliver(app: &mut AppState, owed: Outstanding, fail: bool) {
    let result = match owed {
        Outstanding::Compute {
            context,
            name,
            zone,
            action,
            operation_id,
        } => {
            if fail {
                EffectResult::ComputeInstanceActionFailed {
                    context,
                    instance_name: name,
                    instance_zone: zone,
                    action,
                    error: AppError::gcloud("generated failure"),
                    operation_id,
                }
            } else {
                EffectResult::ComputeInstanceActionSucceeded {
                    context,
                    instance_name: name,
                    instance_zone: zone,
                    action,
                    operation_id,
                }
            }
        }
        Outstanding::Sql {
            context,
            name,
            action,
            operation_id,
        } => {
            if fail {
                EffectResult::CloudSqlInstanceActionFailed {
                    context,
                    instance_name: name,
                    action,
                    error: AppError::gcloud("generated failure"),
                    operation_id,
                }
            } else {
                EffectResult::CloudSqlInstanceActionSucceeded {
                    context,
                    instance_name: name,
                    action,
                    operation_id,
                }
            }
        }
    };
    let effects = reduce(app, &Action::EffectResult(result));
    // Result arms settle state; they must never mint new operations.
    assert!(
        !effects.iter().any(|effect| matches!(
            effect,
            Effect::ComputeInstanceAction { .. }
                | Effect::CloudSqlInstanceAction { .. }
                | Effect::RunResourceBatch { .. }
        )),
        "an action-result arm emitted a new operation effect"
    );
}

/// True when minting `mints` more operations would push the bounded log past
/// its capacity and start evicting records. While under capacity the log never
/// drops anything, so its length is exactly the number of operations minted so
/// far — including any the reducer minted without handing this test an effect.
///
/// The capacity is read off the log rather than mirrored as a literal here: an
/// under-count would let the log evict a still-`Pending` record and fail the
/// property spuriously, an over-count would silently shrink what it covers.
fn would_evict(app: &AppState, mints: usize) -> bool {
    app.root.system.operations.records().len() + mints > app.root.system.operations.capacity()
}

fn apply_step(
    app: &mut AppState,
    step: &Step,
    outstanding: &mut Vec<Outstanding>,
    minted: &mut Vec<u64>,
) {
    match step {
        Step::ConfirmSingleCompute { instance } => {
            if would_evict(app, 1) {
                return;
            }
            let context = app.root.current_context().expect("configurations seeded");
            let names = ["api-1", "api-2", "worker-1"];
            app.root.resources.start_action_confirmation(
                context,
                compute_instance(names[*instance], ResourceStatus::Running),
                ComputeInstanceAction::Stop,
            );
            let effects = reduce(
                app,
                &Action::Resources(ResourcesAction::ConfirmPendingAction),
            );
            collect_outstanding(&effects, outstanding, minted);
        }
        Step::ConfirmSingleSql { instance } => {
            if would_evict(app, 1) {
                return;
            }
            let context = app.root.current_context().expect("configurations seeded");
            let names = ["pg-main", "pg-replica", "pg-scratch"];
            app.root.resources.start_cloud_sql_action_confirmation(
                context,
                cloud_sql_instance(names[*instance]),
                CloudSqlInstanceAction::Restart,
            );
            let effects = reduce(
                app,
                &Action::Resources(ResourcesAction::ConfirmPendingAction),
            );
            collect_outstanding(&effects, outstanding, minted);
        }
        Step::ConfirmComputeBatch { size } => {
            if would_evict(app, *size) {
                return;
            }
            let context = app.root.current_context().expect("configurations seeded");
            let fleet: Vec<ComputeInstance> = (0..*size)
                .map(|index| compute_instance(&format!("batch-{index}"), ResourceStatus::Stopped))
                .collect();
            reduce(
                app,
                &Action::EffectResult(EffectResult::ComputeInstancesLoaded {
                    context: context.clone(),
                    instances: fleet,
                }),
            );
            // Marks survive a reload, and the cursor may sit anywhere in the
            // previous fleet, so start from a known state: no marks, cursor on
            // the first row. Without this a `ToggleMark` can *unmark* a row a
            // previous batch step left marked, the batch confirmation finds
            // nothing eligible and returns early, and the step degenerates into
            // a no-op that contributes no coverage and nothing to notice it by.
            reduce(app, &Action::Resources(ResourcesAction::ClearMarks));
            for _ in 0..MAX_FLEET {
                reduce(app, &Action::Resources(ResourcesAction::SelectPrevious));
            }
            reduce(app, &Action::Resources(ResourcesAction::ToggleMark));
            for _ in 1..*size {
                reduce(app, &Action::Resources(ResourcesAction::SelectNext));
                reduce(app, &Action::Resources(ResourcesAction::ToggleMark));
            }
            app.root
                .resources
                .start_batch_confirmation::<Compute>(context, ComputeInstanceAction::Start);
            let effects = reduce(app, &Action::Resources(ResourcesAction::ConfirmBatchAction));
            let before = outstanding.len();
            collect_outstanding(&effects, outstanding, minted);
            // A size-N batch owes N results. Anything less means the batch
            // silently dropped targets, which would quietly shrink what the
            // property covers instead of failing.
            assert_eq!(
                outstanding.len() - before,
                *size,
                "a batch of {size} eligible rows yielded {} outstanding results",
                outstanding.len() - before
            );
        }
        Step::SwitchConfiguration { config } => {
            let (name, project) = CONFIGURATIONS[*config];
            reduce(
                app,
                &Action::EffectResult(EffectResult::ConfigurationActivated {
                    configuration: configuration(name, project),
                }),
            );
        }
        Step::DeliverResult { pick, fail } => {
            if outstanding.is_empty() {
                return;
            }
            let owed = outstanding.remove(pick % outstanding.len());
            deliver(app, owed, *fail);
        }
    }
}

proptest! {
    #[test]
    fn every_minted_operation_settles_once_its_result_is_processed(
        steps in proptest::collection::vec(step_strategy(), 0..40),
    ) {
        let mut app = AppState::new();
        reduce(
            &mut app,
            &Action::EffectResult(EffectResult::ConfigurationsLoaded {
                configurations: vec![configuration("dev", "dev-project")],
            }),
        );

        let mut outstanding = Vec::new();
        let mut minted: Vec<u64> = Vec::new();
        for step in &steps {
            apply_step(&mut app, step, &mut outstanding, &mut minted);
        }

        // Drain: every effect the session issued gets its result, alternating
        // outcomes so both arms keep being exercised.
        for (index, owed) in outstanding.drain(..).enumerate() {
            deliver(&mut app, owed, index % 2 == 1);
        }

        // The invariant all five stranding tickets violated somewhere: with no
        // result left outstanding, nothing may still read Pending.
        prop_assert_eq!(
            app.root.system.operations.pending_count(),
            0,
            "operations log strands entries Pending after all results settled: {:?}",
            app.root.system.operations.records()
        );

        // Stated over the ids actually minted, not just over whatever survived:
        // a `pending_count` of zero would also hold for an entry the bounded log
        // evicted while it was still Pending. Sessions are capped at the log's
        // capacity precisely so every minted id is still there to check.
        for id in &minted {
            let record = app
                .root
                .system
                .operations
                .records()
                .iter()
                .find(|record| record.id == *id);
            let Some(record) = record else {
                return Err(TestCaseError::fail(format!(
                    "operation {id} left the log before it settled: {:?}",
                    app.root.system.operations.records()
                )));
            };
            prop_assert_ne!(
                record.outcome,
                OperationOutcome::Pending,
                "operation {} is still Pending after its result was processed: {:?}",
                id,
                record
            );
        }
    }
}
