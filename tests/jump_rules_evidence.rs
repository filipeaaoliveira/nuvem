//! Evidence generator — the `J` picker driven by `[[jumps]]` rules.
//!
//! The picker itself did not change with this ticket, so a screenshot would
//! show what the original picker already did. What changed is where its rows come from,
//! and that is only visible by asking the same picker four different questions:
//! what it offers with no config at all (which must still be the original set,
//! service by service), what it offers under a rule the user wrote, what a
//! rule the config layer refuses costs the rules beside it, and what the
//! status line says when a rule is valid but cannot be resolved for the row on
//! screen. A reviewer reading the four side by side can see that a rule is
//! data end to end and that neither an unloadable rule nor an unrenderable one
//! degrades into an empty picker.
//!
//! Every assertion runs unconditionally. When `NUVEM_EVIDENCE_DIR` is set the
//! same four answers are also written out as a transcript; the test writes
//! nothing at all when it is unset, and never outside that directory.

use std::fs;
use std::path::PathBuf;

use nuvem::app::reducer::root::reduce;
use nuvem::app::state::jumps::applicable_jumps;
use nuvem::app::{Action, AppState, EffectResult, JumpsAction, ResourcesAction, WorkspaceAction};
use nuvem::config::jumps::JumpRules;
use nuvem_domain::gcp::configuration::{GcloudConfiguration, GcloudConfigurationName};
use nuvem_domain::gcp::resource::{
    CloudRunService, CloudRunServiceStatus, CloudSqlAvailability, CloudSqlInstance, CloudSqlStatus,
    ComputeInstance, EndpointVisibility, GkeCluster, GkeClusterStatus, ResourceServiceKind,
    ResourceStatus, StorageBucket,
};

use crate::support::jump_rules;
use jump_rules::with_rules;

// ---------------------------------------------------------------------------
// fixtures
// ---------------------------------------------------------------------------

fn config(name: &str) -> GcloudConfiguration {
    GcloudConfiguration {
        name: GcloudConfigurationName::new(name),
        is_active: true,
        account: Some("dev@example.com".to_string()),
        project: Some("dev-project".to_string()),
        region: Some("europe-west1".to_string()),
        zone: Some("europe-west1-b".to_string()),
    }
}

fn app_with_context() -> AppState {
    let mut app = AppState::new();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ConfigurationsLoaded {
            configurations: vec![config("dev")],
        }),
    );
    reduce(&mut app, &Action::Workspace(WorkspaceAction::Resources));
    app
}

fn select_service(app: &mut AppState, service: ResourceServiceKind) {
    reduce(
        app,
        &Action::Resources(ResourcesAction::SelectService(service)),
    );
}

/// A GCE instance with no external IP — the value a rule can name and the row
/// can lack, which is the whole runtime-emptiness case below.
fn instance(name: &str) -> ComputeInstance {
    ComputeInstance {
        id: format!("{name}-id"),
        name: name.to_string(),
        zone: "europe-west1-b".to_string(),
        status: ResourceStatus::Running,
        machine_type: "e2-medium".to_string(),
        internal_ip: Some("10.0.0.2".to_string()),
        external_ip: None,
    }
}

fn gke_cluster(name: &str) -> GkeCluster {
    GkeCluster {
        name: name.to_string(),
        location: "europe-west1".to_string(),
        status: GkeClusterStatus::Running,
        current_master_version: Some("1.30".to_string()),
        current_node_count: Some(3),
        endpoint_visibility: EndpointVisibility::Public,
    }
}

fn cloud_sql_instance(name: &str) -> CloudSqlInstance {
    CloudSqlInstance {
        name: name.to_string(),
        database_version: "POSTGRES_15".to_string(),
        engine: "PostgreSQL".to_string(),
        region: "us-central1".to_string(),
        tier: "db-custom-2-7680".to_string(),
        availability: CloudSqlAvailability::Zonal,
        status: CloudSqlStatus::Runnable,
        connectivity: EndpointVisibility::Public,
    }
}

fn cloud_run_service(name: &str) -> CloudRunService {
    CloudRunService {
        name: name.to_string(),
        region: "us-central1".to_string(),
        url: None,
        latest_ready_revision: Some(format!("{name}-00001")),
        latest_created_revision: Some(format!("{name}-00001")),
        traffic_summary: "100% -> latest".to_string(),
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

/// One loaded row in each service, with `service` the one the cursor is on.
///
/// Every service is loaded every time so the rule set is the only thing that
/// differs between the sections below: a jump that is missing is missing
/// because no rule offers it, not because its service was never fetched.
fn app_on(service: ResourceServiceKind) -> AppState {
    let mut app = app_with_context();
    let context = app.root.current_context().expect("context exists");
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ComputeInstancesLoaded {
            context: context.clone(),
            instances: vec![instance("web-01")],
        }),
    );
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::GkeClustersLoaded {
            context: context.clone(),
            clusters: vec![gke_cluster("platform-dev")],
        }),
    );
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::CloudSqlInstancesLoaded {
            context: context.clone(),
            instances: vec![cloud_sql_instance("orders-db")],
        }),
    );
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::CloudRunServicesLoaded {
            context: context.clone(),
            services: vec![cloud_run_service("api")],
        }),
    );
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::StorageBucketsLoaded {
            context,
            buckets: vec![storage_bucket("assets-bucket")],
        }),
    );
    select_service(&mut app, service);
    app
}

/// The picker's rows as a user reads them: the destination tab's key and the
/// row's wording. Rendering the picker itself would add its borders and its
/// cursor without adding a fact — these two strings are what a jump *is*.
fn rows(app: &AppState) -> Vec<String> {
    applicable_jumps(&app.root)
        .iter()
        .map(|jump| format!("{} | {}", jump.target.config_key(), jump.label))
        .collect()
}

/// Opens the picker the way `J` does and returns the status line, which is
/// where a picker that refused to open explains itself.
fn open_picker(app: &mut AppState) -> Option<String> {
    reduce(app, &Action::Jumps(JumpsAction::Open));
    app.root.system.status_message.clone()
}

// ---------------------------------------------------------------------------
// evidence writing
// ---------------------------------------------------------------------------

/// The evidence directory, or `None` when `NUVEM_EVIDENCE_DIR` is unset.
///
/// There is deliberately no fallback location: with the variable unset this
/// test writes nothing, so it cannot leave files behind on a developer's
/// machine or collide with a concurrent run.
fn evidence_dir() -> Option<PathBuf> {
    let dir = std::env::var("NUVEM_EVIDENCE_DIR").ok()?;
    let path = PathBuf::from(dir);
    fs::create_dir_all(&path).ok()?;
    Some(path)
}

// ---------------------------------------------------------------------------
// the four answers
// ---------------------------------------------------------------------------

#[test]
fn the_jump_picker_is_driven_by_rules_end_to_end() {
    let mut transcript = String::from(
        "Where the `J` picker's rows come from.\n\
         Each block is a rule set and the picker it produces for the row under the\n\
         cursor, read as `<destination tab> | <row wording>`.\n",
    );
    let mut section = |title: &str, body: String| {
        transcript.push_str(&format!("\n{title}\n{}\n{body}", "-".repeat(title.len())));
    };

    // ---- 1. No config at all: the original picker, service by service ----
    let mut defaults = String::new();
    for (service, expected) in [
        (
            ResourceServiceKind::ComputeInstances,
            vec![
                "compute:network | Network of web-01".to_string(),
                "compute:disks | Disks of web-01".to_string(),
            ],
        ),
        (
            ResourceServiceKind::CloudSql,
            vec!["cloudsql:replicas | Replicas of orders-db".to_string()],
        ),
        (
            ResourceServiceKind::CloudRunServices,
            vec!["cloudrun:revisions | Revisions of api".to_string()],
        ),
        (ResourceServiceKind::GkeClusters, Vec::new()),
        (ResourceServiceKind::CloudStorage, Vec::new()),
    ] {
        let mut app = app_on(service);
        assert_eq!(
            rows(&app),
            expected,
            "the built-in rules must reproduce the original picker for {service:?}"
        );
        defaults.push_str(&format!("  {}:\n", service.label()));
        if expected.is_empty() {
            // A service with no rule must say so rather than opening an empty
            // modal, and it says "configured" — the sentence is about the
            // file, not about the service.
            let reason = open_picker(&mut app);
            assert!(app.root.jump_picker.is_none());
            defaults.push_str(&format!(
                "    (no rows) status: {}\n",
                reason.clone().unwrap_or_default()
            ));
            assert_eq!(
                reason.as_deref(),
                // One arm per service rather than a catch-all: a sixth service
                // with no default rules would otherwise be asserted against
                // Cloud Storage's noun and pass while saying something else.
                Some(match service {
                    ResourceServiceKind::GkeClusters =>
                        "No related-resource jumps configured for GKE clusters",
                    ResourceServiceKind::CloudStorage =>
                        "No related-resource jumps configured for Cloud Storage buckets",
                    other => unreachable!("{other:?} has default rules, so its picker opens"),
                })
            );
        }
        for row in expected {
            defaults.push_str(&format!("    {row}\n"));
        }
    }
    section(
        "1. No `[[jumps]]` in the file — the built-in rules",
        defaults,
    );

    // ---- 2. A rule the user wrote ----------------------------------------
    let configured = "[[jumps]]\n\
                      from = \"compute\"\n\
                      to = \"gke:node-pools\"\n\
                      name = \"{name}-cluster\"\n\
                      location = \"{zone}\"\n\
                      label = \"Node pools of the {name} cluster\"\n";
    let mut app = app_on(ResourceServiceKind::ComputeInstances);
    with_rules(&mut app, configured);
    let custom = rows(&app);
    assert_eq!(
        custom,
        ["gke:node-pools | Node pools of the web-01 cluster"],
        "a configured rule replaces the built-in set rather than adding to it, and its \
         own wording is what the picker shows"
    );
    section(
        "2. One `[[jumps]]` rule — a GCE row jumping into another service's tab",
        format!("{configured}\n  {}\n", custom.join("\n  ")),
    );

    // ---- 3. A rule the config layer refuses -------------------------------
    let rejected = "[[jumps]]\n\
                    from = \"compute\"\n\
                    to = \"compute:netwrok\"\n\
                    \n\
                    [[jumps]]\n\
                    from = \"compute\"\n\
                    to = \"compute:disks\"\n";
    #[derive(serde::Deserialize)]
    struct Doc {
        jumps: toml::Value,
    }
    let doc: Doc = toml::from_str(rejected).expect("the fixture is valid TOML");
    let mut warnings = Vec::new();
    let surviving = JumpRules::from_file(&doc.jumps, &mut warnings);
    assert_eq!(
        warnings,
        ["jumps[0]: unknown destination tab \"compute:netwrok\"; ignoring this rule"],
        "a rejected rule is named by its index and its problem"
    );
    assert_eq!(
        surviving.rules().len(),
        1,
        "the rule beside the rejected one must still be loaded"
    );
    app.root.jump_rules = surviving;
    let after_rejection = rows(&app);
    assert_eq!(
        after_rejection,
        ["compute:disks | Disks of web-01"],
        "and it must still be offered — a typo costs its own rule, not the picker"
    );
    section(
        "3. A rule the config layer refuses — per rule, not per file",
        format!(
            "{rejected}\n  warning: {}\n  picker:\n    {}\n",
            warnings[0],
            after_rejection.join("\n    ")
        ),
    );

    // ---- 4. A rule that loads but cannot be resolved for this row ---------
    let unrenderable = "[[jumps]]\n\
                        from = \"compute\"\n\
                        to = \"compute:network\"\n\
                        name = \"{external_ip}\"\n";
    let mut app = app_on(ResourceServiceKind::ComputeInstances);
    with_rules(&mut app, unrenderable);
    assert!(
        app.root
            .resources
            .current_compute_target()
            .expect("a row is selected")
            .external_ip
            .is_none(),
        "the row must lack the field for this section to mean anything"
    );
    assert!(
        rows(&app).is_empty(),
        "a jump that cannot be resolved is not applicable, so it is not offered"
    );
    let reason = open_picker(&mut app);
    assert!(
        app.root.jump_picker.is_none(),
        "and the picker stays closed rather than opening empty"
    );
    assert_eq!(
        reason.as_deref(),
        Some(
            "Jump \"compute:network\" needs external_ip, which this GCE instance has no value for"
        ),
        "the status line names the rule the user wrote and the field the row lacks"
    );
    section(
        "4. A valid rule with nothing to render for this row",
        format!(
            "{unrenderable}\n  web-01 has no external IP.\n  picker: (closed)\n  status: {}\n",
            reason.unwrap_or_default()
        ),
    );

    let Some(dir) = evidence_dir() else {
        return;
    };
    fs::write(dir.join("jump-rules-transcript.txt"), transcript)
        .expect("write jump rules transcript");
}
