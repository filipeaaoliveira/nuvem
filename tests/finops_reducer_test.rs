use nuvem::app::action::FinOpsAction;
use nuvem::app::effect::{AppError, Effect, EffectResult};
use nuvem::app::reducer::finops;
use nuvem::app::state::finops::CardState;
use nuvem::app::state::root::RootState;
use nuvem_domain::gcp::configuration::{
    GcloudConfiguration, GcloudConfigurationName, GcloudContext,
};
use nuvem_domain::gcp::finops::RecommenderId;
use nuvem_domain::gcp::resource::{ComputeInstance, ResourceStatus};

fn context() -> GcloudContext {
    GcloudContext {
        configuration: GcloudConfigurationName::new("default"),
        account: None,
        project: Some("proj".to_string()),
        region: None,
        zone: None,
    }
}

fn with_active_configuration() -> RootState {
    let mut root = RootState::default();
    root.configurations.set_active(GcloudConfiguration {
        name: GcloudConfigurationName::new("default"),
        is_active: true,
        account: None,
        project: Some("proj".to_string()),
        region: None,
        zone: None,
    });
    root
}

/// A root whose Resources workspace has already listed one instance, which is
/// where the FinOps fanout comes from.
fn root_with_a_listed_zone(zone: &str) -> RootState {
    let mut root = with_active_configuration();
    root.resources.set_instances(
        context(),
        vec![ComputeInstance {
            id: "1".to_string(),
            name: "vm-1".to_string(),
            zone: zone.to_string(),
            status: ResourceStatus::Running,
            machine_type: "e2-medium".to_string(),
            internal_ip: None,
            external_ip: None,
        }],
    );
    root
}

fn probes(effects: &[Effect]) -> Vec<(RecommenderId, String)> {
    effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::LoadCostRecommendations {
                recommender,
                location,
                ..
            } => Some((*recommender, location.clone())),
            _ => None,
        })
        .collect()
}

/// A refresh dispatches one effect per probe and leaves every card Loading.
#[test]
fn a_refresh_dispatches_one_effect_per_probe() {
    let mut root = root_with_a_listed_zone("europe-west1-b");
    let effects = finops::reduce(&mut root, &FinOpsAction::Refresh);

    assert_eq!(
        probes(&effects).len(),
        4,
        "three zonal recommenders plus one regional"
    );
    assert_eq!(
        root.finops.card_state(RecommenderId::IdleVm),
        CardState::Loading
    );
}

/// The regional recommender is asked about the region, never the zone: a zone
/// passed to it returns an empty array that reads as "nothing to save".
#[test]
fn the_regional_recommender_is_asked_about_the_region() {
    let mut root = root_with_a_listed_zone("europe-west1-b");
    let effects = finops::reduce(&mut root, &FinOpsAction::Refresh);

    let regional: Vec<String> = probes(&effects)
        .into_iter()
        .filter(|(recommender, _)| *recommender == RecommenderId::UnusedAddress)
        .map(|(_, location)| location)
        .collect();
    assert_eq!(regional, vec!["europe-west1".to_string()]);
}

/// A failure carries its classified kind onto the card rather than a generic
/// error, so "enable the API" and "check IAM" stay distinguishable. The
/// classifier is what makes this stderr classify as ApiDisabled at all.
#[test]
fn a_failed_probe_lands_as_an_unavailable_cell_carrying_its_kind() {
    let mut root = root_with_a_listed_zone("europe-west1-b");
    let effects = finops::reduce(&mut root, &FinOpsAction::Refresh);
    let Some(Effect::LoadCostRecommendations {
        recommender,
        location,
        ticket,
        ..
    }) = effects
        .iter()
        .find(|effect| matches!(effect, Effect::LoadCostRecommendations { .. }))
        .cloned()
    else {
        panic!("expected a probe effect");
    };

    assert!(finops::apply_result(
        &mut root,
        EffectResult::CostRecommendationsFailed {
            ticket,
            recommender,
            location,
            error: AppError::gcloud(
                "PERMISSION_DENIED: Recommender API has not been used in project p \
                 before or it is disabled.\n  reason: SERVICE_DISABLED",
            ),
        },
    ));

    let coverage = root.finops.coverage(recommender);
    assert_eq!(coverage.refused.len(), 1);
    assert!(coverage.answered.is_empty());
    assert_eq!(
        coverage.refused[0].1,
        nuvem_domain::gcp::diagnostics::GcloudErrorKind::ApiDisabled,
        "a disabled API must not be reported as a permissions problem"
    );
}

/// A zero-row success is Empty, not Unavailable, and it counts as coverage.
/// This is the reducer half of the exit-status rule: the service turns a
/// non-zero exit into Failed, so this arm only ever sees a real answer.
#[test]
fn a_successful_empty_probe_is_recorded_as_answered() {
    let mut root = root_with_a_listed_zone("europe-west1-b");
    let effects = finops::reduce(&mut root, &FinOpsAction::Refresh);
    let Some(Effect::LoadCostRecommendations {
        recommender,
        location,
        ticket,
        ..
    }) = effects
        .iter()
        .find(|effect| matches!(effect, Effect::LoadCostRecommendations { .. }))
        .cloned()
    else {
        panic!("expected a probe effect");
    };

    assert!(finops::apply_result(
        &mut root,
        EffectResult::CostRecommendationsLoaded {
            ticket,
            recommender,
            location: location.clone(),
            recommendations: Vec::new(),
        },
    ));

    assert_eq!(root.finops.coverage(recommender).answered, vec![location]);
    assert!(root.finops.coverage(recommender).refused.is_empty());
}

/// With nothing listed there is nothing to probe, and the reducer says so by
/// dispatching nothing rather than by querying a guessed zone.
#[test]
fn a_refresh_with_nothing_listed_dispatches_nothing() {
    let mut root = with_active_configuration();
    let effects = finops::reduce(&mut root, &FinOpsAction::Refresh);
    assert!(probes(&effects).is_empty());
    assert_eq!(
        root.finops.card_state(RecommenderId::IdleVm),
        CardState::NotProbed
    );
}

/// With no active configuration there is no project to ask about, so the
/// refresh is a no-op rather than a call against whatever gcloud defaults to.
#[test]
fn a_refresh_without_an_active_configuration_dispatches_nothing() {
    let mut root = RootState::default();
    let effects = finops::reduce(&mut root, &FinOpsAction::Refresh);
    assert!(effects.is_empty());
}

/// Every dispatched effect carries the active context, so a result arriving
/// after a configuration change can be told apart from a current one.
#[test]
fn every_probe_carries_the_active_context() {
    let mut root = root_with_a_listed_zone("europe-west1-b");
    let effects = finops::reduce(&mut root, &FinOpsAction::Refresh);

    for effect in &effects {
        if let Effect::LoadCostRecommendations { context, .. } = effect {
            assert_eq!(context.project.as_deref(), Some("proj"));
        }
    }
}

/// Disk locations feed the fanout as well as instance zones, and a regional
/// disk's location reaches only the regional recommender.
#[test]
fn listed_locations_come_from_instances_and_disks_alike() {
    let mut root = root_with_a_listed_zone("europe-west1-b");
    root.resources.set_persistent_disks(
        context(),
        vec![nuvem_domain::gcp::resource::PersistentDisk {
            name: "d-1".to_string(),
            location: "us-central1-a".to_string(),
            status: nuvem_domain::gcp::resource::DiskStatus::Ready,
            size_gb: 10,
            disk_type: "pd-standard".to_string(),
            users: Vec::new(),
            labels: Vec::new(),
        }],
    );

    let effects = finops::reduce(&mut root, &FinOpsAction::Refresh);
    let zonal: Vec<String> = probes(&effects)
        .into_iter()
        .filter(|(recommender, _)| *recommender == RecommenderId::IdleVm)
        .map(|(_, location)| location)
        .collect();
    assert_eq!(
        zonal,
        vec!["europe-west1-b".to_string(), "us-central1-a".to_string()]
    );
}

/// An unrelated result changes nothing, so the reducer's fold cannot be the
/// place a foreign effect result silently lands.
#[test]
fn an_unrelated_effect_result_changes_nothing() {
    let mut root = root_with_a_listed_zone("europe-west1-b");
    finops::reduce(&mut root, &FinOpsAction::Refresh);
    let before = root.finops.clone();

    assert!(!finops::apply_result(
        &mut root,
        EffectResult::ConfigurationsLoaded {
            configurations: Vec::new(),
        },
    ));
    assert_eq!(root.finops, before);
}

/// A row whose location came back blank contributes nothing, and a disk in a
/// zone an instance already named does not double the fanout. Both matter for
/// the same reason: each entry here becomes one gcloud subprocess, and
/// `--location=` with an empty value is a call that can only fail.
#[test]
fn a_blank_location_is_skipped_and_a_repeated_one_is_not_queried_twice() {
    let mut root = root_with_a_listed_zone("europe-west1-b");
    root.resources.set_persistent_disks(
        context(),
        vec![
            nuvem_domain::gcp::resource::PersistentDisk {
                name: "same-zone".to_string(),
                location: "europe-west1-b".to_string(),
                status: nuvem_domain::gcp::resource::DiskStatus::Ready,
                size_gb: 10,
                disk_type: "pd-standard".to_string(),
                users: Vec::new(),
                labels: Vec::new(),
            },
            nuvem_domain::gcp::resource::PersistentDisk {
                name: "no-location".to_string(),
                location: String::new(),
                status: nuvem_domain::gcp::resource::DiskStatus::Ready,
                size_gb: 10,
                disk_type: "pd-standard".to_string(),
                users: Vec::new(),
                labels: Vec::new(),
            },
        ],
    );

    assert_eq!(
        root.resources.listed_locations(),
        vec!["europe-west1-b".to_string()]
    );

    let effects = finops::reduce(&mut root, &FinOpsAction::Refresh);
    let zonal: Vec<String> = probes(&effects)
        .into_iter()
        .filter(|(recommender, _)| *recommender == RecommenderId::IdleVm)
        .map(|(_, location)| location)
        .collect();
    assert_eq!(zonal, vec!["europe-west1-b".to_string()]);
}

/// The budgets call goes out even when nothing has been listed: budgets are
/// billing-account scoped, so they have nothing to do with which zones hold
/// rows, and a project with no listed resources can still have a budget.
#[test]
fn a_refresh_asks_for_budgets_even_with_nothing_listed() {
    let mut root = with_active_configuration();
    let effects = finops::reduce(&mut root, &FinOpsAction::Refresh);

    assert!(probes(&effects).is_empty());
    assert_eq!(
        effects
            .iter()
            .filter(|effect| matches!(effect, Effect::LoadBudgets { .. }))
            .count(),
        1
    );
}

/// "No billing account" is a fact about the project, not a failure to read
/// one, so it gets its own state rather than an error inviting a retry that
/// cannot succeed.
#[test]
fn a_project_with_no_billing_account_is_not_reported_as_an_error() {
    use nuvem::app::state::finops::BudgetsState;

    let mut root = with_active_configuration();
    let effects = finops::reduce(&mut root, &FinOpsAction::Refresh);
    let Some(Effect::LoadBudgets { ticket, .. }) = effects
        .iter()
        .find(|effect| matches!(effect, Effect::LoadBudgets { .. }))
        .cloned()
    else {
        panic!("expected a budgets effect");
    };

    assert!(finops::apply_result(
        &mut root,
        EffectResult::BudgetsFailed {
            ticket,
            error: AppError::gcloud("no billing account is linked to this project"),
        },
    ));
    assert_eq!(*root.finops.budgets(), BudgetsState::NoBillingAccount);
}

/// Every other budgets failure keeps its classified kind, so a denied call and
/// a disabled API stay distinguishable here too.
#[test]
fn a_denied_budgets_call_keeps_its_kind() {
    use nuvem::app::state::finops::BudgetsState;
    use nuvem_domain::gcp::diagnostics::GcloudErrorKind;

    let mut root = with_active_configuration();
    let effects = finops::reduce(&mut root, &FinOpsAction::Refresh);
    let Some(Effect::LoadBudgets { ticket, .. }) = effects
        .iter()
        .find(|effect| matches!(effect, Effect::LoadBudgets { .. }))
        .cloned()
    else {
        panic!("expected a budgets effect");
    };

    finops::apply_result(
        &mut root,
        EffectResult::BudgetsFailed {
            ticket,
            error: AppError::gcloud("PERMISSION_DENIED: the caller does not have permission"),
        },
    );

    assert!(matches!(
        root.finops.budgets(),
        BudgetsState::Unavailable {
            kind: GcloudErrorKind::PermissionDenied,
            ..
        }
    ));
}

/// A budgets result from a superseded refresh is dropped, under the same
/// ticket discipline as a cell.
#[test]
fn a_superseded_budgets_result_is_dropped() {
    use nuvem::app::state::finops::BudgetsState;

    let mut root = with_active_configuration();
    let effects = finops::reduce(&mut root, &FinOpsAction::Refresh);
    let Some(Effect::LoadBudgets { ticket, .. }) = effects
        .iter()
        .find(|effect| matches!(effect, Effect::LoadBudgets { .. }))
        .cloned()
    else {
        panic!("expected a budgets effect");
    };

    finops::reduce(&mut root, &FinOpsAction::Refresh);

    assert!(!finops::apply_result(
        &mut root,
        EffectResult::BudgetsLoaded {
            ticket,
            budgets: Vec::new(),
        },
    ));
    assert_eq!(*root.finops.budgets(), BudgetsState::Loading);
}

/// The positive path: a budgets result under the current ticket lands, with
/// its rows intact.
#[test]
fn a_budgets_result_lands_with_its_rows() {
    use nuvem::app::state::finops::BudgetsState;
    use nuvem::services::resources::finops::Budget;

    let mut root = with_active_configuration();
    let effects = finops::reduce(&mut root, &FinOpsAction::Refresh);
    let Some(Effect::LoadBudgets { ticket, .. }) = effects
        .iter()
        .find(|effect| matches!(effect, Effect::LoadBudgets { .. }))
        .cloned()
    else {
        panic!("expected a budgets effect");
    };

    let budget = Budget {
        display_name: "Monthly cap".to_string(),
        amount: None,
        matches_last_period: true,
        thresholds: vec![100],
        covers_whole_account: false,
    };
    assert!(finops::apply_result(
        &mut root,
        EffectResult::BudgetsLoaded {
            ticket,
            budgets: vec![budget.clone()],
        },
    ));
    assert_eq!(*root.finops.budgets(), BudgetsState::Ready(vec![budget]));
}
