use nuvem::app::projection::{ResourceFreshness, project_home_overview};
use nuvem::app::state::operations::OperationOutcome;
use nuvem::app::state::root::RootState;
use nuvem_domain::gcp::configuration::{GcloudConfiguration, GcloudConfigurationName};
use nuvem_domain::gcp::diagnostics::{
    DiagnosticProblem, GcloudDiagnostic, GcloudDiagnosticsReport, GcloudErrorKind,
};
use nuvem_domain::gcp::resource::{ComputeInstance, ResourceServiceKind, ResourceStatus, Secret};
use std::time::Instant;

fn active_dev(root: &mut RootState) {
    root.configurations.set_rows(vec![GcloudConfiguration {
        name: GcloudConfigurationName::new("dev"),
        is_active: true,
        account: Some("dev@example.com".to_string()),
        project: Some("proj".to_string()),
        region: Some("europe-west1".to_string()),
        zone: None,
    }]);
}

#[test]
fn no_project_and_not_loaded_are_distinguished_from_empty() {
    let root = RootState::default();
    let overview = project_home_overview(&root, Instant::now());
    assert!(overview.context.project.is_none());
    let gce = &overview.inventory[0];
    assert_eq!(gce.service_label, "GCE instances");
    assert!(matches!(gce.freshness, ResourceFreshness::NotLoaded));
}

#[test]
fn loaded_empty_service_is_healthy_empty() {
    let mut root = RootState::default();
    active_dev(&mut root);
    let context = root.current_context().unwrap();
    root.resources.set_instances(context, vec![]);
    let overview = project_home_overview(&root, Instant::now());
    assert!(matches!(
        overview.inventory[0].freshness,
        ResourceFreshness::HealthyEmpty
    ));
    assert_eq!(overview.inventory[0].total, 0);
}

#[test]
fn loaded_service_reports_status_counts() {
    let mut root = RootState::default();
    active_dev(&mut root);
    let context = root.current_context().unwrap();
    root.resources.set_instances(
        context,
        vec![ComputeInstance {
            id: "a".into(),
            name: "a".into(),
            zone: "z".into(),
            status: ResourceStatus::Running,
            machine_type: "e2".into(),
            internal_ip: None,
            external_ip: None,
        }],
    );
    let overview = project_home_overview(&root, Instant::now());
    assert!(matches!(
        overview.inventory[0].freshness,
        ResourceFreshness::Loaded
    ));
    assert!(
        overview.inventory[0]
            .status_counts
            .contains(&("Running".to_string(), 1))
    );
}

#[test]
fn degraded_service_shows_cause() {
    let mut root = RootState::default();
    active_dev(&mut root);
    root.resources.set_service_diagnostic(
        ResourceServiceKind::ComputeInstances,
        GcloudDiagnostic::new(
            GcloudErrorKind::ApiDisabled,
            "GCE instances",
            None,
            None,
            "Compute API disabled",
        ),
    );
    let overview = project_home_overview(&root, Instant::now());
    let card = &overview.inventory[0];
    let problem = card.problem.as_ref().unwrap();
    assert!(problem.contains("API disabled"));
    // AC #5: the headline is always "{label} — {user action}", never just
    // the raw category or message.
    assert_eq!(
        problem.as_str(),
        "API disabled — Enable the required API in the Cloud Console"
    );
    assert_eq!(
        card.user_action.as_deref(),
        Some("Enable the required API in the Cloud Console")
    );
    assert_eq!(card.detail_preview.as_deref(), Some("Compute API disabled"));
}

#[test]
fn healthy_service_has_no_user_action_or_detail_preview() {
    let mut root = RootState::default();
    active_dev(&mut root);
    let context = root.current_context().unwrap();
    root.resources.set_instances(context, vec![]);
    let overview = project_home_overview(&root, Instant::now());
    assert!(overview.inventory[0].user_action.is_none());
    assert!(overview.inventory[0].detail_preview.is_none());
}

#[test]
fn missing_gcloud_surfaces_in_health() {
    let mut root = RootState::default();
    root.system.diagnostics.complete(GcloudDiagnosticsReport {
        available: false,
        active_account: None,
        problem: Some(DiagnosticProblem::new(
            GcloudErrorKind::Unknown,
            "gcloud not found",
        )),
    });
    let overview = project_home_overview(&root, Instant::now());
    assert_eq!(overview.gcloud.available, Some(false));
    assert!(overview.gcloud.problem.is_some());
    // AC #5/#6: the health card exposes the same headline + user action +
    // bounded detail preview shape as the inventory cards.
    assert_eq!(
        overview.gcloud.problem.as_deref(),
        Some("unknown error — Retry, or copy the details to investigate")
    );
    assert_eq!(
        overview.gcloud.user_action.as_deref(),
        Some("Retry, or copy the details to investigate")
    );
    assert_eq!(
        overview.gcloud.detail_preview.as_deref(),
        Some("gcloud not found")
    );
}

#[test]
fn health_detail_preview_is_sanitized_even_from_raw_probe_text() {
    // AC #3 anti-leak: the gcloud auth-list probe stores raw stderr in
    // `DiagnosticProblem.message` (it predates the sanitized `GcloudDiagnostic`
    // carrier); the projection must still never leak a credential-shaped
    // token to the UI.
    let mut root = RootState::default();
    root.system.diagnostics.complete(GcloudDiagnosticsReport {
        available: true,
        active_account: None,
        problem: Some(DiagnosticProblem::new(
            GcloudErrorKind::PermissionDenied,
            "permission denied: Bearer ya29.SECRET-token-value",
        )),
    });
    let overview = project_home_overview(&root, Instant::now());
    let preview = overview.gcloud.detail_preview.unwrap();
    assert!(!preview.contains("ya29.SECRET-token-value"));
    assert!(preview.contains("[redacted]"));
}

/// The inventory is one entry per service, derived from the registry. A
/// hand-written list is what left three services with no Home entry at all,
/// and a length assertion alone would not catch a duplicated arm.
#[test]
fn inventory_has_one_entry_per_service_kind_in_registry_order() {
    let root = RootState::default();
    let overview = project_home_overview(&root, Instant::now());

    let kinds: Vec<ResourceServiceKind> = overview.inventory.iter().map(|card| card.kind).collect();

    assert_eq!(kinds, ResourceServiceKind::ALL.to_vec());

    for card in &overview.inventory {
        assert_eq!(
            card.service_label,
            card.kind.label(),
            "{:?} is labelled off-registry",
            card.kind
        );
    }
}

/// A service with no status breakdown still has to report the rows it holds.
/// Deriving `total` from summed status counts reported "loaded (empty)" for
/// every secret and bucket ever fetched.
#[test]
fn statusless_service_with_rows_is_loaded_not_healthy_empty() {
    let mut root = RootState::default();
    active_dev(&mut root);
    let context = root.current_context().unwrap();
    root.resources.set_secrets(
        context,
        vec![Secret {
            name: "api-signing-key".to_string(),
            create_time: None,
            labels: Vec::new(),
            replication: "Automatic".to_string(),
        }],
    );

    let card = root_card(&root, ResourceServiceKind::SecretManagerSecrets);
    assert_eq!(card.total, 1);
    assert!(matches!(card.freshness, ResourceFreshness::Loaded));
    assert!(card.status_counts.is_empty());
}

fn root_card(
    root: &RootState,
    kind: ResourceServiceKind,
) -> nuvem::app::projection::InventoryCardProjection {
    project_home_overview(root, Instant::now())
        .inventory
        .into_iter()
        .find(|card| card.kind == kind)
        .expect("every kind has an inventory entry")
}

#[test]
fn pending_operation_appears() {
    let mut root = RootState::default();
    root.system.operations.begin("Stop instance", "web-1");
    let overview = project_home_overview(&root, Instant::now());
    assert_eq!(overview.operations[0].label, "Stop instance web-1");
    assert_eq!(overview.operations[0].outcome, OperationOutcome::Pending);
}

/// Home's contract is "is my context healthy and what is in it". FinOps gets
/// one line, and only when there is a figure behind it — a
/// "Potential savings: USD 0.00" here would be the same false good news the
/// cost screen exists to avoid, and a zero total is indistinguishable from a
/// total nobody was allowed to compute.
#[test]
fn home_shows_no_savings_teaser_until_finops_has_a_non_zero_total() {
    use nuvem::app::state::finops::CellState;
    use nuvem_domain::gcp::configuration::GcloudContext;
    use nuvem_domain::gcp::finops::{CostRecommendation, RecommendationState};
    use nuvem_domain::gcp::money::Money;

    let context = GcloudContext {
        configuration: GcloudConfigurationName::new("dev"),
        account: None,
        project: Some("proj".to_string()),
        region: None,
        zone: None,
    };

    // Unprobed: nothing.
    assert!(
        project_home_overview(&RootState::default(), Instant::now())
            .savings_teaser
            .is_none()
    );

    let seeded = |nanos: i128| {
        let mut root = RootState::default();
        let probes = root
            .finops
            .begin_refresh(context.clone(), &["europe-west1-b".to_string()]);
        for (recommender, location, ticket) in probes {
            root.finops.install_cell(
                ticket,
                recommender,
                &location,
                CellState::Ready(vec![CostRecommendation {
                    name: "r".to_string(),
                    description: "d".to_string(),
                    state: RecommendationState::Active,
                    priority: None,
                    xor_group: None,
                    monthly: Some(Money {
                        currency: "USD".to_string(),
                        nanos,
                    }),
                    last_refresh: None,
                    target_resources: Vec::new(),
                }]),
            );
        }
        root
    };

    // Probed, and everything netted out to nothing: still no teaser, because
    // "we found no savings" and "we could not look" render identically as a
    // zero and only one of them is good news.
    assert!(
        project_home_overview(&seeded(0), Instant::now())
            .savings_teaser
            .is_none()
    );

    let teaser = project_home_overview(&seeded(2_500_000_000), Instant::now())
        .savings_teaser
        .expect("a real figure earns a teaser");
    assert!(teaser.contains("USD 10.00"), "four cards at 2.50: {teaser}");
    assert!(
        teaser.contains("Ctrl-f"),
        "the teaser names the key that opens the workspace: {teaser}"
    );
}
