//! The related-resource jumps available for the *current
//! selection*, as a pure function of [`RootState`].
//!
//! Per constraint **G2** the picker's contents are derived from the selection
//! by `applicable_jumps`, never from a hardcoded per-service list inspected at
//! render time: a follow-up ticket turns these entries into user-supplied data,
//! and this signature is the seam it needs.
//!
//! The sharp edge here is the same one Task 2 hit: with a detail pane open the
//! list cursor still moves behind it, so resolving from the list accessor can
//! name a resource the user is not looking at. These tests pin the same
//! precedence rule `selection_log_query` established — the detail target wins.

use nuvem::app::reducer::root::reduce;
use nuvem::app::state::jumps::{JumpKey, ResourceJump, applicable_jumps, jump_diagnostics};
use nuvem::app::state::navigation::{DetailTab, JumpTarget, ResourceKey};
use nuvem::app::state::resources::{
    CloudRunDetailTab, CloudSqlDetailTab, ComputeDetailTab, GkeDetailTab,
};
use nuvem::app::{Action, AppState, EffectResult, ResourcesAction, WorkspaceAction};
use nuvem_domain::gcp::configuration::{GcloudConfiguration, GcloudConfigurationName};
use nuvem_domain::gcp::logging::{LogQuery, LogResourceKind};
use nuvem_domain::gcp::resource::{
    BigQueryDataset, CloudFunction, CloudFunctionEnvironment, CloudFunctionState,
    CloudRunCondition, CloudRunRevision, CloudRunService, CloudRunServiceDetail,
    CloudRunServiceStatus, CloudSqlAvailability, CloudSqlInstance, CloudSqlStatus, ComputeInstance,
    ComputeInstanceDetail, ComputeNetworkInterface, DiskStatus, EndpointVisibility, FirewallAction,
    FirewallDirection, FirewallRule, FirewallTargets, GkeCluster, GkeClusterStatus,
    IamPolicyMember, PersistentDisk, PubSubDelivery, PubSubSubscription, PubSubSubscriptionState,
    PubSubTopic, ResourceServiceKind, ResourceStatus, Secret, ServiceAccount, StorageBucket,
    SubscriptionCount, VpcNetwork, VpcRoutingMode, VpcSubnet, VpcSubnetMode,
};

// ---------------------------------------------------------------------------
// fixtures
// ---------------------------------------------------------------------------

use crate::support::jump_rules;
use jump_rules::with_rules;

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

fn revision(name: &str) -> CloudRunRevision {
    CloudRunRevision {
        name: name.to_string(),
        ready: CloudRunServiceStatus::Ready,
        created: None,
        image: Some("gcr.io/x/y:latest".to_string()),
        active_percent: Some(100),
    }
}

fn cloud_run_detail(name: &str, revisions: Vec<CloudRunRevision>) -> CloudRunServiceDetail {
    CloudRunServiceDetail {
        name: name.to_string(),
        region: "us-central1".to_string(),
        url: None,
        status: CloudRunServiceStatus::Ready,
        latest_ready_revision: Some(format!("{name}-00001")),
        latest_created_revision: Some(format!("{name}-00001")),
        traffic: Vec::new(),
        revisions,
        conditions: vec![CloudRunCondition {
            type_: "Ready".to_string(),
            status: "True".to_string(),
            reason: None,
            message: None,
            last_transition_time: None,
        }],
        image: None,
        cpu: None,
        memory: None,
        concurrency: None,
        service_account: None,
        ingress: None,
        min_instances: None,
        max_instances: None,
        labels: Vec::new(),
        raw_json: "{}".to_string(),
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

/// Flattens the jumps to `(target, label)` pairs so a test failure prints the
/// exact labels rather than a wall of struct debug output.
fn pairs(jumps: &[ResourceJump]) -> Vec<(DetailTab, &str)> {
    jumps
        .iter()
        .map(|jump| (detail_tab(jump.target), jump.label.as_str()))
        .collect()
}

/// The detail tab a target names, refusing a list target rather than rendering
/// it as something a `DetailTab` comparison could accept.
///
/// Every test that reaches for [`pairs`] or [`offers`] configures detail rules,
/// so a list target arriving here is a rule resolving to the wrong destination
/// — which is a failure to report, not a case to widen the helper for. The
/// list targets have their own file.
fn detail_tab(target: JumpTarget) -> DetailTab {
    match target {
        JumpTarget::Detail(tab) => tab,
        JumpTarget::List(kind) => panic!("expected a detail target, got the {kind:?} list"),
    }
}

/// The detail key a jump key carries, refusing a row id for the same reason.
fn detail_key(key: &JumpKey) -> ResourceKey {
    match key {
        JumpKey::Detail(key) => key.clone(),
        JumpKey::Row(row) => panic!("expected a detail key, got the row id {row:?}"),
        JumpKey::Narrowed(scope) => panic!("expected a detail key, got a narrowing of {scope:?}"),
    }
}

/// Every offer as `(target, key, label)` — the whole of a [`ResourceJump`].
/// This is what lets the built-in rules be pinned against the *old* hardcoded
/// picker rather than against their own output.
fn offers(jumps: &[ResourceJump]) -> Vec<(DetailTab, ResourceKey, String)> {
    jumps
        .iter()
        .map(|jump| {
            (
                detail_tab(jump.target),
                detail_key(&jump.key),
                jump.label.clone(),
            )
        })
        .collect()
}

/// The resource a key names, whichever service keys it and whichever of the
/// two shapes it is in.
fn key_name(key: &JumpKey) -> String {
    match key {
        JumpKey::Detail(
            ResourceKey::Compute { name, .. }
            | ResourceKey::Gke { name, .. }
            | ResourceKey::CloudSql { name, .. }
            | ResourceKey::CloudRun { name, .. },
        ) => name.clone(),
        JumpKey::Row(row) => row.name.clone(),
        JumpKey::Narrowed(scope) => panic!("expected a keyed jump, got a narrowing of {scope:?}"),
    }
}

fn select_service(app: &mut AppState, service: ResourceServiceKind) {
    reduce(
        app,
        &Action::Resources(ResourcesAction::SelectService(service)),
    );
}

// ---------------------------------------------------------------------------
// list selection
// ---------------------------------------------------------------------------

fn app_with_compute() -> AppState {
    let mut app = app_with_context();
    let context = app.root.current_context().expect("context exists");
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ComputeInstancesLoaded {
            context,
            instances: vec![instance("web-01"), instance("web-02")],
        }),
    );
    app
}

/// The order is the *tab bar's* order (Summary, Network, Disks, ...), not an
/// arbitrary one: the picker offers the jumps in the order the user will find
/// them once they land, so the two never disagree on screen.
#[test]
fn compute_row_offers_network_and_disks_in_tab_bar_order() {
    let app = app_with_compute();
    assert_eq!(
        pairs(&applicable_jumps(&app.root)),
        [
            (
                DetailTab::Compute(ComputeDetailTab::Network),
                "Network of web-01"
            ),
            (
                DetailTab::Compute(ComputeDetailTab::Disks),
                "Disks of web-01"
            ),
        ]
    );
}

fn app_with_cloud_sql() -> AppState {
    let mut app = app_with_context();
    let context = app.root.current_context().expect("context exists");
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::CloudSqlInstancesLoaded {
            context,
            // Cloud SQL rows render sorted by name, so `orders-db` is the row
            // under the cursor and `reporting-db` is the one below it.
            instances: vec![
                cloud_sql_instance("orders-db"),
                cloud_sql_instance("reporting-db"),
            ],
        }),
    );
    select_service(&mut app, ResourceServiceKind::CloudSql);
    app
}

#[test]
fn cloud_sql_row_offers_replicas_named_for_the_instance() {
    let app = app_with_cloud_sql();
    assert_eq!(
        pairs(&applicable_jumps(&app.root)),
        [(
            DetailTab::CloudSql(CloudSqlDetailTab::Replicas),
            "Replicas of orders-db"
        )]
    );
}

fn app_with_cloud_run() -> AppState {
    let mut app = app_with_context();
    let context = app.root.current_context().expect("context exists");
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::CloudRunServicesLoaded {
            context,
            services: vec![cloud_run_service("api"), cloud_run_service("worker")],
        }),
    );
    select_service(&mut app, ResourceServiceKind::CloudRunServices);
    app
}

#[test]
fn cloud_run_row_offers_revisions_named_for_the_service() {
    let app = app_with_cloud_run();
    assert_eq!(
        pairs(&applicable_jumps(&app.root)),
        [(
            DetailTab::CloudRun(CloudRunDetailTab::Revisions),
            "Revisions of api"
        )]
    );
}

// ---------------------------------------------------------------------------
// detail-pane precedence (the Task 2 rule: the detail target wins)
// ---------------------------------------------------------------------------

/// `j`/`k` still reach the collection behind an open detail pane, so a jump
/// resolved from the list cursor would name a resource the user cannot see.
#[test]
fn compute_detail_wins_over_a_list_cursor_that_moved_behind_it() {
    let mut app = app_with_compute();
    reduce(&mut app, &Action::Resources(ResourcesAction::OpenSelected));
    reduce(&mut app, &Action::Resources(ResourcesAction::SelectNext));

    assert!(app.root.resources.is_showing_detail());
    assert_eq!(
        app.root
            .resources
            .selected_visible_instance()
            .map(|instance| instance.name.as_str()),
        Some("web-02"),
        "the list cursor must have moved for this test to mean anything"
    );

    assert_eq!(
        pairs(&applicable_jumps(&app.root)),
        [
            (
                DetailTab::Compute(ComputeDetailTab::Network),
                "Network of web-01"
            ),
            (
                DetailTab::Compute(ComputeDetailTab::Disks),
                "Disks of web-01"
            ),
        ]
    );
}

#[test]
fn cloud_sql_detail_wins_over_a_list_cursor_that_moved_behind_it() {
    let mut app = app_with_cloud_sql();
    reduce(&mut app, &Action::Resources(ResourcesAction::OpenSelected));
    reduce(&mut app, &Action::Resources(ResourcesAction::SelectNext));

    assert!(app.root.resources.is_showing_detail());
    assert_eq!(
        app.root
            .resources
            .selected_cloud_sql_instance()
            .map(|instance| instance.name.as_str()),
        Some("reporting-db"),
        "the list cursor must have moved for this test to mean anything"
    );

    assert_eq!(
        pairs(&applicable_jumps(&app.root)),
        [(
            DetailTab::CloudSql(CloudSqlDetailTab::Replicas),
            "Replicas of orders-db"
        )]
    );
}

#[test]
fn cloud_run_detail_wins_over_a_list_cursor_that_moved_behind_it() {
    let mut app = app_with_cloud_run();
    reduce(&mut app, &Action::Resources(ResourcesAction::OpenSelected));
    reduce(&mut app, &Action::Resources(ResourcesAction::SelectNext));

    assert!(app.root.resources.is_showing_detail());
    assert_eq!(
        app.root
            .resources
            .selected_cloud_run_service()
            .map(|service| service.name.as_str()),
        Some("worker"),
        "the list cursor must have moved for this test to mean anything"
    );

    assert_eq!(
        pairs(&applicable_jumps(&app.root)),
        [(
            DetailTab::CloudRun(CloudRunDetailTab::Revisions),
            "Revisions of api"
        )]
    );
}

/// The nested revision view owns the pane, so *the revision* is the selection
/// there — and a revision has no related-resource jumps in this ticket. Same
/// rule `selection_log_query` follows when it scopes logs to the revision
/// rather than to the parent service.
#[test]
fn inside_the_cloud_run_revision_view_no_jumps_apply() {
    let mut app = app_with_cloud_run();
    reduce(&mut app, &Action::Resources(ResourcesAction::OpenSelected));
    let context = app.root.current_context().expect("context exists");
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::CloudRunServiceDetailLoaded {
            context,
            detail: cloud_run_detail("api", vec![revision("api-00001"), revision("api-00002")]),
        }),
    );
    // Summary -> Traffic -> Revisions, then open a revision.
    reduce(&mut app, &Action::Resources(ResourcesAction::SelectNextTab));
    reduce(&mut app, &Action::Resources(ResourcesAction::SelectNextTab));
    reduce(&mut app, &Action::Resources(ResourcesAction::OpenSelected));
    assert!(app.root.resources.is_showing_cloud_run_revision());

    assert_eq!(applicable_jumps(&app.root), Vec::new());
}

// ---------------------------------------------------------------------------
// services and states with no jumps
// ---------------------------------------------------------------------------

fn app_with_gke() -> AppState {
    let mut app = app_with_context();
    let context = app.root.current_context().expect("context exists");
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::GkeClustersLoaded {
            context,
            clusters: vec![gke_cluster("platform-dev")],
        }),
    );
    select_service(&mut app, ResourceServiceKind::GkeClusters);
    app
}

fn app_with_storage() -> AppState {
    let mut app = app_with_context();
    select_service(&mut app, ResourceServiceKind::CloudStorage);
    let context = app.root.current_context().expect("context exists");
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::StorageBucketsLoaded {
            context,
            // Two rows, so the list cursor has somewhere to drift to behind
            // an open metadata pane.
            buckets: vec![
                storage_bucket("assets-bucket"),
                storage_bucket("logs-bucket"),
            ],
        }),
    );
    app
}

fn app_with_cloud_functions() -> AppState {
    let mut app = app_with_context();
    select_service(&mut app, ResourceServiceKind::CloudFunctions);
    let context = app.root.current_context().expect("context exists");
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::CloudFunctionsLoaded {
            context,
            functions: vec![CloudFunction {
                name: "thumbnailer".to_string(),
                region: "europe-west1".to_string(),
                state: CloudFunctionState::Active,
                environment: CloudFunctionEnvironment::Gen2,
                url: Some("https://europe-west1-dev.cloudfunctions.net/thumbnailer".to_string()),
                update_time: Some("2026-03-01T10:00:00Z".to_string()),
                labels: Vec::new(),
            }],
        }),
    );
    app
}

fn app_with_persistent_disks() -> AppState {
    let mut app = app_with_context();
    select_service(&mut app, ResourceServiceKind::PersistentDisks);
    let context = app.root.current_context().expect("context exists");
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::PersistentDisksLoaded {
            context,
            disks: vec![PersistentDisk {
                name: "detached-legacy".to_string(),
                location: "europe-west1-b".to_string(),
                status: DiskStatus::Ready,
                size_gb: 20,
                disk_type: "pd-ssd".to_string(),
                users: Vec::new(),
                labels: Vec::new(),
            }],
        }),
    );
    app
}
fn app_with_pubsub_topics() -> AppState {
    let mut app = app_with_context();
    select_service(&mut app, ResourceServiceKind::PubSubTopics);
    let context = app.root.current_context().expect("context exists");
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::PubSubTopicsLoaded {
            context,
            topics: vec![PubSubTopic {
                name: "orders".to_string(),
                full_name: "projects/dev-project/topics/orders".to_string(),
                subscription_count: SubscriptionCount::Known(2),
                message_retention: Some("604800s".to_string()),
                kms_key: None,
                labels: Vec::new(),
            }],
        }),
    );
    app
}

/// A network row carrying a value for every field `fields_for` lists, which
/// for this service is `name` alone — a network is global, so there is no
/// location field to populate.
fn app_with_vpc_networks() -> AppState {
    let mut app = app_with_context();
    select_service(&mut app, ResourceServiceKind::VpcNetworks);
    let context = app.root.current_context().expect("context exists");
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::VpcNetworksLoaded {
            context,
            networks: vec![VpcNetwork {
                name: "default".to_string(),
                subnet_mode: VpcSubnetMode::Auto,
                routing_mode: VpcRoutingMode::Regional,
                subnet_count: 43,
                description: Some("Default network for the project".to_string()),
            }],
        }),
    );
    app
}

/// A subnet row carrying a value for every field `fields_for` lists, which for
/// this service is `name` *and* `location` — a subnet is regional, so unlike
/// the networks table beside it, an absent location would be a wiring gap
/// rather than a resource that has none.
/// Fully populated in the sense this walk needs: every field
/// `config::jumps::fields_for` may name for firewall rules has a value. A
/// firewall rule is a *global* resource, so the only field it offers is
/// `name`; it has no location, which is why its jumps can never be keyed.
fn app_with_firewall_rules() -> AppState {
    let mut app = app_with_context();
    select_service(&mut app, ResourceServiceKind::FirewallRules);
    let context = app.root.current_context().expect("context exists");
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::FirewallRulesLoaded {
            context,
            rules: vec![FirewallRule {
                name: "default-allow-icmp".to_string(),
                network: "default".to_string(),
                direction: FirewallDirection::Ingress,
                priority: Some(65534),
                action: FirewallAction::Allow,
                targets: FirewallTargets::AllInstances,
                source_or_dest: vec!["0.0.0.0/0".to_string()],
                disabled: false,
            }],
        }),
    );
    app
}

fn app_with_vpc_subnets() -> AppState {
    let mut app = app_with_context();
    select_service(&mut app, ResourceServiceKind::VpcSubnets);
    let context = app.root.current_context().expect("context exists");
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::VpcSubnetsLoaded {
            context,
            subnets: vec![VpcSubnet {
                name: "default".to_string(),
                region: "us-central1".to_string(),
                network: "default".to_string(),
                ip_cidr_range: "10.128.0.0/20".to_string(),
                gateway_address: Some("10.128.0.1".to_string()),
                private_google_access: false,
            }],
        }),
    );
    app
}

fn app_with_pubsub_subscriptions() -> AppState {
    let mut app = app_with_context();
    select_service(&mut app, ResourceServiceKind::PubSubSubscriptions);
    let context = app.root.current_context().expect("context exists");
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::PubSubSubscriptionsLoaded {
            context,
            subscriptions: vec![PubSubSubscription {
                name: "orders-worker".to_string(),
                full_name: "projects/dev-project/subscriptions/orders-worker".to_string(),
                topic: "projects/dev-project/topics/orders".to_string(),
                delivery: PubSubDelivery::Pull,
                ack_deadline_seconds: Some(60),
                message_retention: Some("604800s".to_string()),
                dead_letter: None,
                state: PubSubSubscriptionState::Active,
                labels: Vec::new(),
            }],
        }),
    );
    app
}

fn app_with_bigquery_datasets() -> AppState {
    let mut app = app_with_context();
    select_service(&mut app, ResourceServiceKind::BigQueryDatasets);
    let context = app.root.current_context().expect("context exists");
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::BigQueryDatasetsLoaded {
            context,
            datasets: vec![BigQueryDataset {
                dataset_id: "analytics_events".to_string(),
                project_id: "dev-project".to_string(),
                location: "europe-west1".to_string(),
                friendly_name: Some("Analytics events".to_string()),
                labels: Vec::new(),
            }],
        }),
    );
    app
}

fn app_with_service_accounts() -> AppState {
    let mut app = app_with_context();
    select_service(&mut app, ResourceServiceKind::IamServiceAccounts);
    let context = app.root.current_context().expect("context exists");
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ServiceAccountsLoaded {
            context,
            accounts: vec![ServiceAccount {
                email: "deployer@dev-project.iam.gserviceaccount.com".to_string(),
                display_name: Some("Deployer".to_string()),
                disabled: false,
                unique_id: "100000000000000000001".to_string(),
            }],
        }),
    );
    app
}

fn app_with_policy_bindings() -> AppState {
    let mut app = app_with_context();
    select_service(&mut app, ResourceServiceKind::IamPolicy);
    let context = app.root.current_context().expect("context exists");
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::IamPolicyLoaded {
            context,
            members: vec![IamPolicyMember::from_wire(
                "roles/artifactregistry.admin",
                "user:alice@example.com",
                None,
            )],
        }),
    );
    app
}

fn app_with_secrets() -> AppState {
    let mut app = app_with_context();
    select_service(&mut app, ResourceServiceKind::SecretManagerSecrets);
    let context = app.root.current_context().expect("context exists");
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::SecretsLoaded {
            context,
            secrets: vec![
                Secret {
                    name: "api-signing-key".to_string(),
                    create_time: Some("2026-02-11T09:14:22Z".to_string()),
                    labels: Vec::new(),
                    replication: "Automatic".to_string(),
                },
                Secret {
                    name: "db-password".to_string(),
                    create_time: Some("2026-03-02T17:41:08Z".to_string()),
                    labels: Vec::new(),
                    replication: "Automatic".to_string(),
                },
            ],
        }),
    );
    app
}

#[test]
fn gke_cluster_selection_offers_no_jumps() {
    let app = app_with_gke();

    assert!(app.root.resources.selected_gke_cluster().is_some());
    assert_eq!(applicable_jumps(&app.root), Vec::new());
}

#[test]
fn storage_bucket_selection_offers_no_jumps() {
    let app = app_with_storage();

    assert!(app.root.resources.storage().selected_bucket().is_some());
    assert_eq!(applicable_jumps(&app.root), Vec::new());
}

#[test]
fn an_empty_compute_list_offers_no_jumps() {
    let mut app = app_with_context();
    let context = app.root.current_context().expect("context exists");
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ComputeInstancesLoaded {
            context,
            instances: Vec::new(),
        }),
    );

    assert_eq!(applicable_jumps(&app.root), Vec::new());
}

#[test]
fn a_freshly_started_app_offers_no_jumps() {
    let app = AppState::new();
    assert_eq!(applicable_jumps(&app.root), Vec::new());
}

// ---------------------------------------------------------------------------
// the built-in rules are the original picker
// ---------------------------------------------------------------------------

/// The guarantee: a user who configures nothing sees exactly the
/// picker that shipped first — same destinations, same order, same keys, same
/// wording. Asserting the *whole* offer rather than a projection of it is the
/// point: a rule engine that got the label right and the key wrong would look
/// correct in every other test here and send `gcloud describe` at the wrong
/// resource.
///
/// All five services in one test because the property is the picker as a
/// whole; a per-service split would let a service quietly stop being covered.
#[test]
fn the_built_in_rules_reproduce_the_fixed_picker_for_every_service() {
    let compute_key = ResourceKey::Compute {
        name: "web-01".to_string(),
        zone: "europe-west1-b".to_string(),
    };
    assert_eq!(
        offers(&applicable_jumps(&app_with_compute().root)),
        vec![
            (
                DetailTab::Compute(ComputeDetailTab::Network),
                compute_key.clone(),
                "Network of web-01".to_string(),
            ),
            (
                DetailTab::Compute(ComputeDetailTab::Disks),
                compute_key,
                "Disks of web-01".to_string(),
            ),
        ]
    );

    assert_eq!(
        offers(&applicable_jumps(&app_with_cloud_sql().root)),
        vec![(
            DetailTab::CloudSql(CloudSqlDetailTab::Replicas),
            ResourceKey::CloudSql {
                name: "orders-db".to_string(),
                region: "us-central1".to_string(),
            },
            "Replicas of orders-db".to_string(),
        )]
    );

    assert_eq!(
        offers(&applicable_jumps(&app_with_cloud_run().root)),
        vec![(
            DetailTab::CloudRun(CloudRunDetailTab::Revisions),
            ResourceKey::CloudRun {
                name: "api".to_string(),
                region: "us-central1".to_string(),
            },
            "Revisions of api".to_string(),
        )]
    );

    assert_eq!(offers(&applicable_jumps(&app_with_gke().root)), Vec::new());
    assert_eq!(
        offers(&applicable_jumps(&app_with_storage().root)),
        Vec::new()
    );
}

// ---------------------------------------------------------------------------
// configured rules
// ---------------------------------------------------------------------------

#[test]
fn a_configured_rule_is_offered_with_its_own_wording() {
    let mut app = app_with_compute();
    with_rules(
        &mut app,
        r#"
[[jumps]]
from = "compute"
to = "compute:metadata"
label = "Metadata of {name} ({machine_type})"
"#,
    );

    assert_eq!(
        offers(&applicable_jumps(&app.root)),
        vec![(
            DetailTab::Compute(ComputeDetailTab::Metadata),
            ResourceKey::Compute {
                name: "web-01".to_string(),
                zone: "europe-west1-b".to_string(),
            },
            "Metadata of web-01 (e2-medium)".to_string(),
        )]
    );
}

/// The picker lists rules in the order the file declares them. The built-in set
/// leans on this to keep tab-bar order, and a user reordering their rules is
/// reordering their picker — so it cannot be sorted anywhere on the way.
#[test]
fn the_offers_are_in_the_configured_rule_order() {
    let mut app = app_with_compute();
    with_rules(
        &mut app,
        r#"
[[jumps]]
from = "compute"
to = "compute:disks"

[[jumps]]
from = "compute"
to = "compute:json"

[[jumps]]
from = "compute"
to = "compute:network"
"#,
    );

    assert_eq!(
        applicable_jumps(&app.root)
            .iter()
            .map(|jump| jump.target)
            .collect::<Vec<_>>(),
        vec![
            JumpTarget::Detail(DetailTab::Compute(ComputeDetailTab::Disks)),
            JumpTarget::Detail(DetailTab::Compute(ComputeDetailTab::Json)),
            JumpTarget::Detail(DetailTab::Compute(ComputeDetailTab::Network)),
        ]
    );
}

/// A rule may name a tab of a *different* service, which is the whole point of
/// making jumps data. The key is then keyed as that service keys its
/// resources — `apply_detail` describes a GKE cluster with a location, never a
/// zone — so the destination decides the key's shape, not the source row.
#[test]
fn a_rule_may_target_another_services_tab() {
    let mut app = app_with_compute();
    with_rules(
        &mut app,
        r#"
[[jumps]]
from = "compute"
to = "gke:node-pools"
name = "{name}-cluster"
"#,
    );

    assert_eq!(
        offers(&applicable_jumps(&app.root)),
        vec![(
            DetailTab::Gke(GkeDetailTab::NodePools),
            ResourceKey::Gke {
                name: "web-01-cluster".to_string(),
                location: "europe-west1-b".to_string(),
            },
            "Node Pools of web-01-cluster".to_string(),
        )]
    );
}

/// GKE has no built-in jumps, so a rule from it exercises the source lookup for
/// a service the fixed picker never resolved at all.
#[test]
fn a_rule_can_be_offered_from_a_service_with_no_built_in_jumps() {
    let mut app = app_with_gke();
    with_rules(
        &mut app,
        r#"
[[jumps]]
from = "gke"
to = "gke:node-pools"
"#,
    );

    assert_eq!(
        offers(&applicable_jumps(&app.root)),
        vec![(
            DetailTab::Gke(GkeDetailTab::NodePools),
            ResourceKey::Gke {
                name: "platform-dev".to_string(),
                location: "europe-west1".to_string(),
            },
            "Node Pools of platform-dev".to_string(),
        )]
    );
}

// ---------------------------------------------------------------------------
// a rule that cannot resolve for this row is not offered
// ---------------------------------------------------------------------------

/// Load-time validation cannot see that *this* instance has no external IP.
/// Offering the row anyway would open a pane describing nothing.
#[test]
fn a_rule_whose_field_is_empty_for_the_row_is_not_offered() {
    let mut app = app_with_compute();
    assert!(
        instance("web-01").external_ip.is_none(),
        "the fixture must lack the field for this test to mean anything"
    );
    with_rules(
        &mut app,
        r#"
[[jumps]]
from = "compute"
to = "compute:network"
name = "{external_ip}"
"#,
    );

    assert_eq!(applicable_jumps(&app.root), Vec::new());
}

/// A `label` is wording, not identity. Suppressing a jump whose key resolved
/// perfectly because its *caption* mentions a field this row has no value for
/// trades a working jump for a cosmetic one, so the built-in wording is used
/// and the row is still offered.
#[test]
fn a_rule_whose_label_alone_is_unrenderable_is_still_offered_with_the_built_in_wording() {
    let mut app = app_with_compute();
    assert!(
        instance("web-01").external_ip.is_none(),
        "the fixture must lack the field for this test to mean anything"
    );
    with_rules(
        &mut app,
        r#"
[[jumps]]
from = "compute"
to = "compute:disks"
label = "Disks of {external_ip}"
"#,
    );

    assert_eq!(
        offers(&applicable_jumps(&app.root)),
        vec![(
            DetailTab::Compute(ComputeDetailTab::Disks),
            ResourceKey::Compute {
                name: "web-01".to_string(),
                zone: "europe-west1-b".to_string(),
            },
            "Disks of web-01".to_string(),
        )]
    );
    // Nothing to explain: the jump resolved.
    assert_eq!(jump_diagnostics(&app.root), None);
}

/// The other half of the trade, pinned so the fallback cannot spread: `name`
/// and `location` *are* the key, so an unrenderable one still has to suppress
/// the offer rather than fall back to some other resource.
#[test]
fn an_unrenderable_name_or_location_still_suppresses_the_offer() {
    for key in ["name", "location"] {
        let mut app = app_with_compute();
        with_rules(
            &mut app,
            &format!(
                "[[jumps]]\nfrom = \"compute\"\nto = \"compute:disks\"\n{key} = \"{{external_ip}}\"\n"
            ),
        );

        assert_eq!(applicable_jumps(&app.root), Vec::new(), "{key}");
        assert_eq!(
            jump_diagnostics(&app.root),
            Some(
                "Jump \"compute:disks\" needs external_ip, which this GCE instance has no value \
                 for"
                .to_string()
            ),
            "{key}"
        );
    }
}

/// The rendered name reaches `navigation::apply_detail`, which builds
/// `gcloud describe` arguments from it. A value that is not a GCP resource name
/// is dropped here rather than argued with there.
#[test]
fn a_rendered_name_that_is_not_a_resource_name_is_not_offered() {
    // A path separator: `{name}/{zone}` reads like a resource path but is not a
    // name, and `gcloud` would take the slash as part of the identifier.
    let mut app = app_with_compute();
    with_rules(
        &mut app,
        "[[jumps]]\nfrom = \"compute\"\nto = \"compute:disks\"\nname = \"{name}/{zone}\"\n",
    );
    assert_eq!(applicable_jumps(&app.root), Vec::new());

    // A shell metacharacter, arriving from the row itself rather than from the
    // template — the case no amount of config validation could catch.
    let mut app = app_with_context();
    let context = app.root.current_context().expect("context exists");
    let mut hostile = instance("web-01");
    hostile.name = "web-01;$(id)".to_string();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ComputeInstancesLoaded {
            context,
            instances: vec![hostile],
        }),
    );
    assert_eq!(applicable_jumps(&app.root), Vec::new());
}

/// The location is the other half of the key `apply_detail` describes, so it is
/// held to the same standard as the name.
#[test]
fn a_rendered_location_that_is_not_a_resource_name_is_not_offered() {
    let mut app = app_with_compute();
    with_rules(
        &mut app,
        "[[jumps]]\nfrom = \"compute\"\nto = \"compute:disks\"\nlocation = \"{status} {zone}\"\n",
    );

    assert_eq!(applicable_jumps(&app.root), Vec::new());
    // Named, not just absent: an offer dropped for an unusable location and one
    // dropped for a missing field are the same empty vector.
    assert_eq!(
        jump_diagnostics(&app.root),
        Some(
            "Jump \"compute:disks\" renders an invalid location for this GCE instance".to_string()
        )
    );
}

/// `{status}` in a `name` is bad advice, not a rejected rule.
///
/// The name check is about the *characters* in a rendered value, and most
/// status labels are a single ordinary word, so `name = "{status}"` renders
/// "Running", passes, and is offered — it just names a resource that does not
/// exist, which only shows up when the jump is followed. The statuses worded
/// with a space are the ones the check refuses, and they are the minority.
/// Both halves are pinned here because the README's advice about `{status}`
/// has to be worded from the reason that is actually true.
#[test]
fn a_status_rendered_into_a_name_is_only_refused_when_the_status_has_a_space() {
    let rules = "[[jumps]]\nfrom = \"compute\"\nto = \"compute:disks\"\nname = \"{status}\"\n";

    // "Running": an offer, unhelpful but not refused.
    let mut app = app_with_compute();
    with_rules(&mut app, rules);
    assert_eq!(
        pairs(&applicable_jumps(&app.root)),
        [(
            DetailTab::Compute(ComputeDetailTab::Disks),
            "Disks of Running"
        )]
    );

    // "Unknown (provisioning)": refused, because of the space and the brackets.
    let mut app = app_with_context();
    let context = app.root.current_context().expect("context exists");
    let mut provisioning = instance("web-01");
    provisioning.status = ResourceStatus::Unknown("provisioning".to_string());
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ComputeInstancesLoaded {
            context,
            instances: vec![provisioning],
        }),
    );
    with_rules(&mut app, rules);
    assert_eq!(applicable_jumps(&app.root), Vec::new());
}

/// A rule that omits `location` keys the target by the source row's own
/// zone/region — the built-in rules spell theirs out, so nothing else covers
/// the omitted case.
#[test]
fn a_rule_without_a_location_template_keys_the_target_by_the_rows_own_location() {
    let mut app = app_with_cloud_sql();
    with_rules(
        &mut app,
        "[[jumps]]\nfrom = \"cloudsql\"\nto = \"cloudsql:replicas\"\n",
    );

    assert_eq!(
        applicable_jumps(&app.root)[0].key,
        JumpKey::Detail(ResourceKey::CloudSql {
            name: "orders-db".to_string(),
            region: "us-central1".to_string(),
        })
    );
}

/// A rule from a service that is not the one on screen is not this selection's
/// business, however many of them the file declares.
#[test]
fn rules_from_another_service_are_not_offered() {
    let mut app = app_with_compute();
    with_rules(
        &mut app,
        "[[jumps]]\nfrom = \"cloudsql\"\nto = \"cloudsql:replicas\"\n",
    );

    assert_eq!(applicable_jumps(&app.root), Vec::new());
}

// ---------------------------------------------------------------------------
// the picker's noun is the destination tab's own label
// ---------------------------------------------------------------------------

/// A jump offer reads `"<tab> of <resource>"`, and the `<tab>` half must *be*
/// the destination tab's label rather than a second spelling of it: a second
/// spelling drifts silently the day a tab is renamed, leaving the picker
/// offering "Network of web-01" while the tab bar says "Networking". The
/// `<resource>` half must be the resource the key names, or the row would
/// describe one resource and open another.
#[test]
fn an_offer_with_no_label_template_is_named_by_its_destination_tab_and_key() {
    for app in [
        app_with_compute(),
        app_with_cloud_sql(),
        app_with_cloud_run(),
    ] {
        let jumps = applicable_jumps(&app.root);
        assert!(!jumps.is_empty(), "expected the selection to offer a jump");
        for jump in jumps {
            assert_eq!(
                jump.label,
                format!("{} of {}", jump.target.label(), key_name(&jump.key))
            );
        }
    }
}

// ---------------------------------------------------------------------------
// `L` and `J` resolve the same resource
// ---------------------------------------------------------------------------

/// The resource `J` would jump to, read off the offers' keys. Panics rather
/// than returning an `Option` so a service that stopped offering jumps fails
/// loudly here instead of trivially "agreeing" with `L`.
fn jump_target_name(app: &AppState) -> String {
    let jumps = applicable_jumps(&app.root);
    assert!(!jumps.is_empty(), "expected the selection to offer a jump");
    let names: Vec<String> = jumps.iter().map(|jump| key_name(&jump.key)).collect();
    let first = names[0].clone();
    assert!(
        names.iter().all(|name| *name == first),
        "one selection cannot offer jumps to two resources: {names:?}"
    );
    first
}

/// The resource `L` scoped its log query to. Presses the key, so it moves the
/// app into the Logging workspace — call it after `jump_target_name`.
fn log_target_name(app: &mut AppState) -> String {
    reduce(
        app,
        &Action::Resources(ResourcesAction::OpenLogsForSelection),
    );
    match &app.root.logging.query {
        LogQuery::Resource(LogResourceKind::ComputeInstance { instance_name, .. })
        | LogQuery::Resource(LogResourceKind::CloudSqlInstance { instance_name, .. }) => {
            instance_name.clone()
        }
        LogQuery::Resource(LogResourceKind::CloudRunService { service_name, .. }) => {
            service_name.clone()
        }
        LogQuery::Resource(LogResourceKind::StorageBucket { bucket_name }) => bucket_name.clone(),
        other => panic!("expected a resource-scoped log query, got {other:?}"),
    }
}

/// The invariant itself: `L` and `J` are two features asking the same question
/// — "which resource is this view about" — and both read the answer from
/// `ResourcesState::current_*_target`. A second copy of that rule is how `L`
/// would scope logs to one instance while `J` offered the disks of another.
fn assert_l_and_j_agree(app: &mut AppState) {
    let jumped_to = jump_target_name(app);
    let logged_for = log_target_name(app);
    assert_eq!(
        jumped_to, logged_for,
        "`J` resolved {jumped_to} but `L` resolved {logged_for}"
    );
}

#[test]
fn l_and_j_resolve_the_same_resource_from_a_list() {
    for mut app in [
        app_with_compute(),
        app_with_cloud_sql(),
        app_with_cloud_run(),
    ] {
        assert_l_and_j_agree(&mut app);
    }
}

/// The context where the two could most easily disagree: the list cursor is
/// still movable behind an open pane, so one feature resolving from the list
/// accessor would name the resource the user is *not* looking at.
#[test]
fn l_and_j_resolve_the_same_resource_behind_an_open_detail() {
    for mut app in [
        app_with_compute(),
        app_with_cloud_sql(),
        app_with_cloud_run(),
    ] {
        reduce(&mut app, &Action::Resources(ResourcesAction::OpenSelected));
        reduce(&mut app, &Action::Resources(ResourcesAction::SelectNext));
        assert!(app.root.resources.is_showing_detail());

        assert_l_and_j_agree(&mut app);
    }
}

/// Cloud Storage is the newly reachable half of the same invariant: it had no
/// jumps to offer before this ticket, so a rule is what makes `J` ask the
/// question at all. Its pane is opened by its own action and its "what is on
/// screen" accessor is `StorageState::view_bucket`, but the rule is the rule —
/// the bucket the metadata pane is about beats the list cursor still movable
/// behind it.
#[test]
fn l_and_j_resolve_the_same_bucket_behind_an_open_metadata_pane() {
    let mut app = app_with_storage();
    with_rules(
        &mut app,
        "[[jumps]]\nfrom = \"storage\"\nto = \"gke:node-pools\"\n",
    );
    reduce(
        &mut app,
        &Action::Resources(ResourcesAction::OpenBucketMetadata),
    );
    reduce(&mut app, &Action::Resources(ResourcesAction::SelectNext));

    assert_eq!(
        app.root
            .resources
            .storage()
            .selected_bucket()
            .map(|bucket| bucket.name.as_str()),
        Some("logs-bucket"),
        "the list cursor must have moved for this test to mean anything"
    );

    assert_l_and_j_agree(&mut app);
}

// ---------------------------------------------------------------------------
// the two field tables must agree
// ---------------------------------------------------------------------------

/// A Compute row carrying a value for *every* field `fields_for` lists. The
/// shared fixture deliberately has no external IP, and an absent value is
/// exactly what the parity test below must not mistake for a wiring gap.
fn fully_populated_instance(name: &str) -> ComputeInstance {
    ComputeInstance {
        external_ip: Some("34.1.2.3".to_string()),
        ..instance(name)
    }
}

/// The detail behind [`fully_populated_instance`], carrying a value for every
/// detail-backed field `fields_for(ComputeInstances)` declares.
fn fully_populated_compute_detail(name: &str) -> ComputeInstanceDetail {
    ComputeInstanceDetail {
        name: name.to_string(),
        zone: "europe-west1-b".to_string(),
        status: ResourceStatus::Running,
        machine_type: "e2-medium".to_string(),
        creation_timestamp: None,
        network_interfaces: vec![ComputeNetworkInterface {
            name: Some("nic0".to_string()),
            network: Some("prod-vpc".to_string()),
            subnetwork: Some("prod-subnet".to_string()),
            subnetwork_region: Some("europe-west1".to_string()),
            internal_ip: Some("10.0.0.2".to_string()),
            external_ip: Some("34.1.2.3".to_string()),
        }],
        tags: Vec::new(),
        service_accounts: Vec::new(),
        disks: Vec::new(),
        labels: Vec::new(),
        metadata: Vec::new(),
        raw_json: "{}".to_string(),
    }
}

fn app_with_fully_populated_row(service: ResourceServiceKind) -> AppState {
    match service {
        ResourceServiceKind::ComputeInstances => {
            let mut app = app_with_context();
            let context = app.root.current_context().expect("context exists");
            reduce(
                &mut app,
                &Action::EffectResult(EffectResult::ComputeInstancesLoaded {
                    context: context.clone(),
                    instances: vec![fully_populated_instance("web-01")],
                }),
            );
            // "Fully populated" has to mean *every place a field is declared to
            // come from*, not just the row: `{network}` and `{subnetwork}` are
            // detail-backed (slice 2), so a fixture that stopped at the
            // row would report the wiring this walk hunts as working — the
            // field would be missing for the one reason the walk is not asking
            // about. So the pane is opened and its payload landed too.
            reduce(&mut app, &Action::Resources(ResourcesAction::OpenSelected));
            reduce(
                &mut app,
                &Action::EffectResult(EffectResult::ComputeInstanceDetailLoaded {
                    context,
                    detail: fully_populated_compute_detail("web-01"),
                }),
            );
            app
        }
        ResourceServiceKind::GkeClusters => app_with_gke(),
        ResourceServiceKind::CloudSql => app_with_cloud_sql(),
        ResourceServiceKind::CloudRunServices => app_with_cloud_run(),
        ResourceServiceKind::CloudStorage => app_with_storage(),
        ResourceServiceKind::SecretManagerSecrets => app_with_secrets(),
        ResourceServiceKind::CloudFunctions => app_with_cloud_functions(),
        ResourceServiceKind::PersistentDisks => app_with_persistent_disks(),
        ResourceServiceKind::PubSubTopics => app_with_pubsub_topics(),
        ResourceServiceKind::PubSubSubscriptions => app_with_pubsub_subscriptions(),
        ResourceServiceKind::BigQueryDatasets => app_with_bigquery_datasets(),
        ResourceServiceKind::IamServiceAccounts => app_with_service_accounts(),
        ResourceServiceKind::IamPolicy => app_with_policy_bindings(),
        ResourceServiceKind::VpcNetworks => app_with_vpc_networks(),
        ResourceServiceKind::VpcSubnets => app_with_vpc_subnets(),
        ResourceServiceKind::FirewallRules => app_with_firewall_rules(),
    }
}

/// How a rule's `from` names this service. Spelled out here because the
/// canonical emitter is crate-private; `config_service_id_round_trips` pins
/// that these are spellings the parser accepts.
fn rule_service_name(service: ResourceServiceKind) -> &'static str {
    match service {
        ResourceServiceKind::ComputeInstances => "compute",
        ResourceServiceKind::GkeClusters => "gke",
        ResourceServiceKind::CloudSql => "cloudsql",
        ResourceServiceKind::CloudRunServices => "cloudrun",
        ResourceServiceKind::CloudStorage => "storage",
        ResourceServiceKind::SecretManagerSecrets => "secrets",
        ResourceServiceKind::CloudFunctions => "functions",
        ResourceServiceKind::PersistentDisks => "disks",
        ResourceServiceKind::PubSubTopics => "pubsub-topics",
        ResourceServiceKind::PubSubSubscriptions => "pubsub-subscriptions",
        ResourceServiceKind::BigQueryDatasets => "bigquery-datasets",
        ResourceServiceKind::IamServiceAccounts => "service-accounts",
        ResourceServiceKind::IamPolicy => "iam-policy",
        ResourceServiceKind::VpcNetworks => "vpc-networks",
        ResourceServiceKind::VpcSubnets => "vpc-subnets",
        ResourceServiceKind::FirewallRules => "firewall-rules",
    }
}

/// `fields_for` (config) decides which fields a template may name; the state
/// layer's `selected_fields` decides which it can actually answer. They are two
/// hand-maintained `(service, field)` tables, and a pair present in the first
/// but missing from the second validates at load and then never renders — with
/// `jump_diagnostics` blaming the *row* ("which this GCE instance has no value
/// for") for what is a wiring gap. So every pair is exercised against a row
/// that has a value for everything: what is left is either an offer or, for a
/// value that is not a legal resource name, the "renders an invalid name"
/// verdict — both of which prove the field was found.
///
/// A third verdict proves it just as well: `cannot be keyed`, which a service
/// with no location field earns. `offer` renders the *name* template before it
/// ever consults the location, so a rule that got as far as failing on the
/// location necessarily resolved the field under test. Secret Manager is that
/// case — a secret is global, so no jump can be keyed from one — and reading
/// its refusal as a missing field would be reading a real, documented
/// limitation as the wiring gap this test hunts. The two are different: a
/// wiring gap is a field `fields_for` promises and `selected_fields` cannot
/// answer, and that is still what fails here.
#[test]
fn every_field_a_template_may_name_resolves_for_a_fully_populated_row() {
    for service in ResourceServiceKind::ALL {
        for (field, _) in nuvem::config::jumps::fields_for(service) {
            let mut app = app_with_fully_populated_row(service);
            with_rules(
                &mut app,
                &format!(
                    "[[jumps]]\nfrom = \"{}\"\nto = \"compute:network\"\nname = \"{{{field}}}\"\n",
                    rule_service_name(service)
                ),
            );

            let offered = !applicable_jumps(&app.root).is_empty();
            let diagnostic = jump_diagnostics(&app.root);
            assert!(
                offered
                    || diagnostic.as_deref().is_some_and(|why| {
                        why.contains("renders an invalid name") || why.contains("cannot be keyed")
                    }),
                "{service:?}'s {field:?} is offered by fields_for but selected_fields \
                 has no value for it: {diagnostic:?}"
            );
        }
    }
}
