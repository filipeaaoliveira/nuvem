//! The `J` related-jump picker: opening it, moving inside it,
//! and the jump `Enter` actually performs.
//!
//! The picker's rows come from `applicable_jumps` (Task 3, constraint **G2**);
//! what is pinned here is the *user-facing half*: that a jump that does not
//! apply says so instead of no-oping (**G4**), that `Submit` lands on the right
//! detail tab through the same open-detail path navigation history uses (so the
//! pane fetches rather than rendering stale), and that the jump is traversable
//! with `Alt-←` (**G5**).
use crate::support::remembered_view::remembered;

use nuvem::app::reducer::root::reduce;
use nuvem::app::state::jumps::{JumpKey, JumpPickerState, ResourceJump};
use nuvem::app::state::navigation::{DetailTab, JumpTarget, ResourceKey};
use nuvem::app::state::resources::{CloudRunDetailTab, CloudSqlDetailTab, ComputeDetailTab};
use nuvem::app::{
    Action, AppState, Effect, EffectResult, JumpsAction, NavAction, ResourcesAction,
    WorkspaceAction,
};
use nuvem_domain::gcp::configuration::{GcloudConfiguration, GcloudConfigurationName};
use nuvem_domain::gcp::resource::{
    CloudRunCondition, CloudRunRevision, CloudRunService, CloudRunServiceDetail,
    CloudRunServiceStatus, CloudSqlAvailability, CloudSqlInstance, CloudSqlStatus, ComputeInstance,
    ComputeInstanceDetail, EndpointVisibility, GkeCluster, GkeClusterStatus, ResourceServiceKind,
    ResourceStatus, StorageBucket,
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

/// Activates another gcloud configuration, the way the runtime's result does:
/// every service's rows and context stamp go, the load phases stay.
fn activate(app: &mut AppState, name: &str) {
    reduce(
        app,
        &Action::EffectResult(EffectResult::ConfigurationActivated {
            configuration: GcloudConfiguration {
                name: GcloudConfigurationName::new(name),
                project: Some(format!("{name}-project")),
                ..config(name)
            },
        }),
    );
}

fn load_compute(app: &mut AppState) {
    let context = app.root.current_context().expect("context exists");
    reduce(
        app,
        &Action::EffectResult(EffectResult::ComputeInstancesLoaded {
            context,
            instances: vec![instance("web-01"), instance("web-02")],
        }),
    );
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

fn compute_detail(name: &str) -> ComputeInstanceDetail {
    ComputeInstanceDetail {
        name: name.to_string(),
        zone: "europe-west1-b".to_string(),
        status: ResourceStatus::Running,
        machine_type: "e2-medium".to_string(),
        creation_timestamp: None,
        network_interfaces: Vec::new(),
        tags: Vec::new(),
        service_accounts: Vec::new(),
        disks: Vec::new(),
        labels: Vec::new(),
        metadata: Vec::new(),
        raw_json: "{}".to_string(),
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

fn select_service(app: &mut AppState, service: ResourceServiceKind) {
    reduce(
        app,
        &Action::Resources(ResourcesAction::SelectService(service)),
    );
}

fn open_picker(app: &mut AppState) -> Vec<Effect> {
    reduce(app, &Action::Jumps(JumpsAction::Open))
}

fn status(app: &AppState) -> Option<&str> {
    app.root.system.status_message.as_deref()
}

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

fn app_with_cloud_sql() -> AppState {
    let mut app = app_with_context();
    let context = app.root.current_context().expect("context exists");
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::CloudSqlInstancesLoaded {
            context,
            instances: vec![
                cloud_sql_instance("orders-db"),
                cloud_sql_instance("reporting-db"),
            ],
        }),
    );
    select_service(&mut app, ResourceServiceKind::CloudSql);
    app
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

// ---------------------------------------------------------------------------
// G4: opening where no jump applies never silently no-ops
// ---------------------------------------------------------------------------

/// No rule is written for GKE, so the message names the service rather than a
/// selection: saying "no cluster selected" while a cluster is plainly selected
/// would be the UI contradicting reality. It says *configured*, because whether
/// GKE offers jumps is now the user's decision and not a fact about GKE.
#[test]
fn opening_on_gke_names_the_service_and_leaves_the_picker_closed() {
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
    assert!(app.root.resources.selected_gke_cluster().is_some());

    let effects = open_picker(&mut app);

    assert!(effects.is_empty());
    assert!(app.root.jump_picker.is_none());
    assert_eq!(
        status(&app),
        Some("No related-resource jumps configured for GKE clusters")
    );
}

#[test]
fn opening_on_cloud_storage_names_the_service() {
    let mut app = app_with_context();
    select_service(&mut app, ResourceServiceKind::CloudStorage);
    let context = app.root.current_context().expect("context exists");
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::StorageBucketsLoaded {
            context,
            buckets: vec![storage_bucket("assets-bucket")],
        }),
    );

    open_picker(&mut app);

    assert!(app.root.jump_picker.is_none());
    assert_eq!(
        status(&app),
        Some("No related-resource jumps configured for Cloud Storage buckets")
    );
}

/// The other empty case: a service that *does* have jumps, but with nothing
/// under the cursor to name. This one has to read differently from the message
/// above, since here selecting a row is exactly what would help.
#[test]
fn opening_on_an_empty_compute_list_says_nothing_is_selected() {
    let mut app = app_with_context();
    let context = app.root.current_context().expect("context exists");
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ComputeInstancesLoaded {
            context,
            instances: Vec::new(),
        }),
    );

    open_picker(&mut app);

    assert!(app.root.jump_picker.is_none());
    assert_eq!(status(&app), Some("No GCE instance selected to jump from"));
}

/// The mirror of the GKE message: once a rule *is* written for GKE, an empty
/// cluster list is a "select something" situation and must not be answered with
/// "no jumps configured for GKE clusters". Which services offer jumps is read
/// from the rule set, never from a list of services baked into the message.
#[test]
fn opening_on_a_configured_service_with_nothing_selected_says_nothing_is_selected() {
    let mut app = app_with_context();
    select_service(&mut app, ResourceServiceKind::GkeClusters);
    with_rules(
        &mut app,
        "[[jumps]]\nfrom = \"gke\"\nto = \"gke:node-pools\"\n",
    );
    assert!(app.root.resources.selected_gke_cluster().is_none());

    open_picker(&mut app);

    assert!(app.root.jump_picker.is_none());
    assert_eq!(status(&app), Some("No GKE cluster selected to jump from"));
}

/// The case only the rule engine can produce: the service has a rule, a row is
/// selected, and the rule still cannot name a target. "No GCE instance selected
/// to jump from" would be false in every clause, so the rule reports itself —
/// naming the rule by the `to` the user wrote and the field it needs.
#[test]
fn opening_where_a_rule_needs_a_field_the_row_lacks_names_the_rule_and_the_field() {
    let mut app = app_with_compute();
    with_rules(
        &mut app,
        "[[jumps]]\nfrom = \"compute\"\nto = \"compute:network\"\nname = \"{external_ip}\"\n",
    );
    assert!(
        app.root
            .resources
            .current_compute_target()
            .expect("a row is selected")
            .external_ip
            .is_none(),
        "the row must lack the field for this test to mean anything"
    );

    open_picker(&mut app);

    assert!(app.root.jump_picker.is_none());
    assert_eq!(
        status(&app),
        Some(
            "Jump \"compute:network\" needs external_ip, which this GCE instance has no value for"
        )
    );
}

/// The other unresolvable case: every field has a value, but what they render
/// to is not a name `gcloud describe` may be handed. The reason has to be
/// distinguishable from the missing-field one, since the fix is different —
/// there is nothing to wait for, the template itself is wrong.
#[test]
fn opening_where_a_rule_renders_an_unusable_name_says_so() {
    let mut app = app_with_compute();
    with_rules(
        &mut app,
        "[[jumps]]\nfrom = \"compute\"\nto = \"compute:disks\"\nname = \"{name}/{zone}\"\n",
    );

    open_picker(&mut app);

    assert!(app.root.jump_picker.is_none());
    assert_eq!(
        status(&app),
        Some("Jump \"compute:disks\" renders an invalid name for this GCE instance")
    );
}

/// The location half of the same check, and the only way its wording is
/// pinned: an offer dropped for an unusable *location* is indistinguishable
/// from one dropped for a missing field if only the empty picker is asserted.
#[test]
fn opening_where_a_rule_renders_an_unusable_location_says_which_half_failed() {
    let mut app = app_with_compute();
    with_rules(
        &mut app,
        "[[jumps]]\nfrom = \"compute\"\nto = \"compute:disks\"\nlocation = \"{status} {zone}\"\n",
    );

    open_picker(&mut app);

    assert!(app.root.jump_picker.is_none());
    assert_eq!(
        status(&app),
        Some("Jump \"compute:disks\" renders an invalid location for this GCE instance")
    );
}

/// One broken rule does not silence the rules beside it: the picker opens with
/// what resolved, and says nothing about what did not. An error status over a
/// usable picker would be the modal and the status line disagreeing.
#[test]
fn a_rule_that_cannot_resolve_does_not_stop_the_rules_that_can() {
    let mut app = app_with_compute();
    with_rules(
        &mut app,
        "[[jumps]]\nfrom = \"compute\"\nto = \"compute:network\"\nname = \"{external_ip}\"\n\
         \n[[jumps]]\nfrom = \"compute\"\nto = \"compute:disks\"\n",
    );

    open_picker(&mut app);

    assert_eq!(picker_labels(&app), ["Disks of web-01"]);
    assert_eq!(status(&app), None);
}

/// A configured rule is a real jump, not just a row: it opens the pane at the
/// tab it named. `compute:metadata` is a tab the fixed picker could never
/// reach, so this exercises the destination mapping rather than re-running the
/// built-in one.
#[test]
fn a_jump_from_a_configured_rule_lands_on_the_tab_the_rule_named() {
    let mut app = app_with_compute();
    with_rules(
        &mut app,
        "[[jumps]]\nfrom = \"compute\"\nto = \"compute:metadata\"\n",
    );

    open_picker(&mut app);
    submit(&mut app);

    assert_eq!(
        app.root
            .resources
            .detail_target()
            .map(|instance| instance.name.as_str()),
        Some("web-01")
    );
    assert_eq!(
        app.root.resources.active_detail_tab(),
        ComputeDetailTab::Metadata
    );
}

// ---------------------------------------------------------------------------
// opening, moving, cancelling
// ---------------------------------------------------------------------------

fn picker_labels(app: &AppState) -> Vec<&str> {
    app.root
        .jump_picker
        .as_ref()
        .expect("picker is open")
        .rows()
        .iter()
        .map(|row| row.label.as_str())
        .collect()
}

fn selected_index(app: &AppState) -> usize {
    app.root
        .jump_picker
        .as_ref()
        .expect("picker is open")
        .selected_index()
}

#[test]
fn opening_on_a_compute_row_offers_exactly_the_two_jumps_for_that_instance() {
    let mut app = app_with_compute();

    let effects = open_picker(&mut app);

    assert!(effects.is_empty(), "opening the picker fetches nothing");
    assert_eq!(
        picker_labels(&app),
        ["Network of web-01", "Disks of web-01"]
    );
    assert_eq!(selected_index(&app), 0);
    assert_eq!(
        status(&app),
        None,
        "opening a usable picker is not an error"
    );
}

#[test]
fn the_cursor_moves_down_and_back_up_without_leaving_the_rows() {
    let mut app = app_with_compute();
    open_picker(&mut app);

    reduce(&mut app, &Action::Jumps(JumpsAction::SelectNext));
    assert_eq!(selected_index(&app), 1);

    // Clamped at both ends, like the configuration context picker.
    reduce(&mut app, &Action::Jumps(JumpsAction::SelectNext));
    assert_eq!(selected_index(&app), 1);

    reduce(&mut app, &Action::Jumps(JumpsAction::SelectPrevious));
    assert_eq!(selected_index(&app), 0);
    reduce(&mut app, &Action::Jumps(JumpsAction::SelectPrevious));
    assert_eq!(selected_index(&app), 0);
}

#[test]
fn cancelling_closes_the_picker_and_leaves_the_view_untouched() {
    let mut app = app_with_compute();
    let before = app.root.clone();
    open_picker(&mut app);
    reduce(&mut app, &Action::Jumps(JumpsAction::SelectNext));

    let effects = reduce(&mut app, &Action::Jumps(JumpsAction::Cancel));

    assert!(effects.is_empty());
    assert!(app.root.jump_picker.is_none());
    assert_eq!(
        app.root, before,
        "cancelling must restore the state opening the picker started from"
    );
}

#[test]
fn opening_inside_a_cloud_run_revision_names_the_revision_view() {
    let mut app = app_with_cloud_run();
    reduce(&mut app, &Action::Resources(ResourcesAction::OpenSelected));
    let context = app.root.current_context().expect("context exists");
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::CloudRunServiceDetailLoaded {
            context,
            detail: cloud_run_detail("api", vec![revision("api-00001")]),
        }),
    );
    // Summary -> Traffic -> Revisions, then drill into a revision.
    reduce(&mut app, &Action::Resources(ResourcesAction::SelectNextTab));
    reduce(&mut app, &Action::Resources(ResourcesAction::SelectNextTab));
    reduce(&mut app, &Action::Resources(ResourcesAction::OpenSelected));
    assert!(app.root.resources.is_showing_cloud_run_revision());

    open_picker(&mut app);

    assert!(app.root.jump_picker.is_none());
    assert_eq!(
        status(&app),
        Some("No related-resource jumps from a Cloud Run revision")
    );
}

// ---------------------------------------------------------------------------
// Submit: the jump itself
// ---------------------------------------------------------------------------

fn submit(app: &mut AppState) -> Vec<Effect> {
    reduce(app, &Action::Jumps(JumpsAction::Submit))
}

/// Selects the offer whose label matches, so a Submit test can never quietly
/// jump to the wrong row after an ordering change.
fn select_labelled(app: &mut AppState, label: &str) {
    let index = app
        .root
        .jump_picker
        .as_ref()
        .expect("picker is open")
        .rows()
        .iter()
        .position(|row| row.label == label)
        .unwrap_or_else(|| panic!("picker should offer {label:?}"));
    for _ in 0..index {
        reduce(app, &Action::Jumps(JumpsAction::SelectNext));
    }
}

#[test]
fn submitting_the_network_offer_opens_the_instance_detail_on_the_network_tab() {
    let mut app = app_with_compute();
    let context = app.root.current_context().expect("context exists");
    open_picker(&mut app);
    select_labelled(&mut app, "Network of web-01");

    let effects = submit(&mut app);

    assert!(app.root.jump_picker.is_none(), "the picker closes on jump");
    assert!(app.root.resources.is_showing_detail());
    assert_eq!(
        app.root
            .resources
            .detail_target()
            .map(|instance| instance.name.as_str()),
        Some("web-01")
    );
    assert_eq!(
        app.root.resources.active_detail_tab(),
        ComputeDetailTab::Network
    );
    // The pane must fetch, not render whatever was left behind it.
    assert_eq!(
        effects,
        vec![Effect::LoadComputeInstanceDetail {
            context,
            instance: instance("web-01"),
        }]
    );
}

#[test]
fn submitting_the_disks_offer_opens_the_instance_detail_on_the_disks_tab() {
    let mut app = app_with_compute();
    open_picker(&mut app);
    select_labelled(&mut app, "Disks of web-01");

    let effects = submit(&mut app);

    assert_eq!(
        app.root.resources.active_detail_tab(),
        ComputeDetailTab::Disks
    );
    assert_eq!(effects.len(), 1, "the detail fetch is still emitted");
}

#[test]
fn submitting_replicas_opens_the_cloud_sql_detail_on_the_replicas_tab() {
    let mut app = app_with_cloud_sql();
    let context = app.root.current_context().expect("context exists");
    open_picker(&mut app);
    select_labelled(&mut app, "Replicas of orders-db");

    let effects = submit(&mut app);

    assert!(app.root.resources.is_showing_detail());
    assert_eq!(
        app.root
            .resources
            .cloud_sql_detail_target()
            .map(|instance| instance.name.as_str()),
        Some("orders-db")
    );
    assert_eq!(
        app.root.resources.active_cloud_sql_detail_tab(),
        CloudSqlDetailTab::Replicas
    );
    assert_eq!(
        effects,
        vec![Effect::LoadCloudSqlInstanceDetail {
            context,
            instance: cloud_sql_instance("orders-db"),
        }]
    );
}

/// Cloud Run's detail needs *two* fetches (the service and its revisions), and
/// the revisions one is exactly what the Revisions tab renders — a jump that
/// dropped it would land on a permanently empty tab.
#[test]
fn submitting_revisions_opens_the_cloud_run_detail_and_fetches_the_revisions() {
    let mut app = app_with_cloud_run();
    let context = app.root.current_context().expect("context exists");
    open_picker(&mut app);
    select_labelled(&mut app, "Revisions of api");

    let effects = submit(&mut app);

    assert!(app.root.resources.is_showing_detail());
    assert_eq!(
        app.root.resources.active_cloud_run_detail_tab(),
        CloudRunDetailTab::Revisions
    );
    assert_eq!(
        effects,
        vec![
            Effect::LoadCloudRunServiceDetail {
                context: context.clone(),
                service: cloud_run_service("api"),
            },
            Effect::LoadCloudRunRevisions {
                context,
                service: cloud_run_service("api"),
            },
        ]
    );
}

/// The most common jump of all: the user is already on web-01's pane and wants
/// its Network tab. That is the same resource and the same already-fetched
/// detail, so it must behave like `]` — an instant tab change — rather than
/// blanking the pane and waiting out a `gcloud ... describe` round trip.
#[test]
fn a_jump_to_another_tab_of_the_open_pane_keeps_the_detail_and_fetches_nothing() {
    let mut app = app_with_compute();
    reduce(&mut app, &Action::Resources(ResourcesAction::OpenSelected));
    let context = app.root.current_context().expect("context exists");
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ComputeInstanceDetailLoaded {
            context,
            detail: compute_detail("web-01"),
        }),
    );
    assert!(
        app.root.resources.detail().is_some(),
        "the pane must be showing a fetched detail for this test to mean anything"
    );

    open_picker(&mut app);
    select_labelled(&mut app, "Network of web-01");
    let effects = submit(&mut app);

    assert!(app.root.jump_picker.is_none());
    assert_eq!(
        app.root.resources.active_detail_tab(),
        ComputeDetailTab::Network
    );
    assert!(
        effects.is_empty(),
        "the detail is already loaded; re-fetching it is what blanks the pane"
    );
    assert_eq!(
        app.root.resources.detail(),
        Some(&compute_detail("web-01")),
        "the fetched detail must survive a tab jump within the same pane"
    );
}

/// Cloud Run's pane costs two fetches, so re-opening it for a tab change is the
/// most expensive version of the same mistake.
#[test]
fn a_jump_to_the_revisions_tab_of_the_open_service_fetches_nothing() {
    let mut app = app_with_cloud_run();
    reduce(&mut app, &Action::Resources(ResourcesAction::OpenSelected));
    let context = app.root.current_context().expect("context exists");
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::CloudRunServiceDetailLoaded {
            context,
            detail: cloud_run_detail("api", vec![revision("api-00001")]),
        }),
    );

    open_picker(&mut app);
    select_labelled(&mut app, "Revisions of api");
    let effects = submit(&mut app);

    assert_eq!(
        app.root.resources.active_cloud_run_detail_tab(),
        CloudRunDetailTab::Revisions
    );
    assert!(
        effects.is_empty(),
        "same service, same pane: nothing to load"
    );
    assert!(
        app.root.resources.cloud_run_detail().is_some(),
        "the fetched service detail must survive the tab jump"
    );
}

/// The precedence rule Task 3 pinned, carried through to the jump: with a
/// detail pane open the list cursor still moves behind it, and the jump must
/// land on the instance the label named, not on whatever the cursor drifted to.
#[test]
fn a_jump_from_an_open_pane_lands_on_the_named_instance_not_the_drifted_cursor() {
    let mut app = app_with_compute();
    reduce(&mut app, &Action::Resources(ResourcesAction::OpenSelected));
    reduce(&mut app, &Action::Resources(ResourcesAction::SelectNext));
    assert_eq!(
        app.root
            .resources
            .selected_visible_instance()
            .map(|instance| instance.name.as_str()),
        Some("web-02"),
        "the list cursor must have moved for this test to mean anything"
    );

    open_picker(&mut app);
    select_labelled(&mut app, "Disks of web-01");
    submit(&mut app);

    assert_eq!(
        app.root
            .resources
            .detail_target()
            .map(|instance| instance.name.as_str()),
        Some("web-01")
    );
    assert_eq!(
        app.root.resources.active_detail_tab(),
        ComputeDetailTab::Disks
    );
}

#[test]
fn submitting_with_no_picker_open_changes_nothing() {
    let mut app = app_with_compute();
    let before = app.root.clone();

    let effects = submit(&mut app);

    assert!(effects.is_empty());
    assert_eq!(app.root, before);
}

/// G4 has no exceptions, including for the paths that "cannot happen". A row
/// whose target and key disagree has no destination, and closing the picker
/// while saying nothing is precisely the silent no-op the constraint forbids —
/// the user would see the modal vanish and the view unchanged, with no way to
/// tell a jump happened at all.
#[test]
fn submitting_a_row_with_no_destination_says_so_instead_of_vanishing() {
    let mut app = app_with_compute();
    app.root.jump_picker = JumpPickerState::new(vec![ResourceJump {
        target: JumpTarget::Detail(DetailTab::CloudSql(CloudSqlDetailTab::Replicas)),
        key: JumpKey::Detail(ResourceKey::Compute {
            name: "web-01".to_string(),
            zone: "europe-west1-b".to_string(),
        }),
        label: "Replicas of web-01".to_string(),
    }]);

    let effects = submit(&mut app);

    assert!(effects.is_empty());
    assert!(app.root.jump_picker.is_none());
    assert!(!app.root.resources.is_showing_detail());
    assert!(
        status(&app).is_some(),
        "a jump that goes nowhere has to say so"
    );
}

// ---------------------------------------------------------------------------
// G5: jumps are part of navigation history
// ---------------------------------------------------------------------------

fn back(app: &mut AppState) -> Vec<Effect> {
    reduce(app, &Action::Navigation(NavAction::Back))
}

#[test]
fn alt_left_after_a_jump_from_the_list_returns_to_the_list() {
    let mut app = app_with_compute();
    open_picker(&mut app);
    select_labelled(&mut app, "Disks of web-01");
    submit(&mut app);
    assert!(app.root.resources.is_showing_detail());

    back(&mut app);

    assert!(
        !app.root.resources.is_showing_detail(),
        "Back should return to the list the jump was made from"
    );
}

/// The sharper half of G5: jumping between two tabs of the *same* pane is still
/// a move, so Back has to land on the tab the user left, not just close the
/// pane.
#[test]
fn alt_left_after_a_tab_jump_returns_to_the_originating_tab() {
    let mut app = app_with_compute();
    reduce(&mut app, &Action::Resources(ResourcesAction::OpenSelected));
    assert_eq!(
        app.root.resources.active_detail_tab(),
        ComputeDetailTab::Summary
    );

    open_picker(&mut app);
    select_labelled(&mut app, "Disks of web-01");
    submit(&mut app);
    assert_eq!(
        app.root.resources.active_detail_tab(),
        ComputeDetailTab::Disks
    );

    back(&mut app);

    assert!(app.root.resources.is_showing_detail());
    assert_eq!(
        app.root.resources.active_detail_tab(),
        ComputeDetailTab::Summary
    );
    assert_eq!(
        app.root
            .resources
            .detail_target()
            .map(|instance| instance.name.as_str()),
        Some("web-01")
    );
}

/// The other half of "moves nothing": it must not *change* anything either.
///
/// `select_detail_tab` is not idempotent — a tab change clears the field
/// cursor, resets the scroll, and (for Cloud Run) sends the revision cursor
/// back to the top — so running it for a jump that lands where the pane
/// already is would leave "you lost your place" as the jump's only visible
/// effect. The field cursor in particular belongs to a different feature,
/// so nothing about this pane's own behaviour would flag the loss.
#[test]
fn a_jump_to_the_tab_already_on_screen_leaves_the_pane_untouched() {
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

    // Land on the Revisions tab through a jump that really does move.
    open_picker(&mut app);
    select_labelled(&mut app, "Revisions of api");
    submit(&mut app);
    assert_eq!(
        app.root.resources.active_cloud_run_detail_tab(),
        CloudRunDetailTab::Revisions
    );

    // The operator then makes themselves at home in the pane.
    reduce(&mut app, &Action::Resources(ResourcesAction::SelectNext));
    reduce(
        &mut app,
        &Action::Resources(ResourcesAction::ScrollDetailDown),
    );
    app.root.resources.set_detail_field_index(Some(1));
    let scroll = app.root.resources.detail_scroll();
    assert_eq!(app.root.resources.selected_cloud_run_revision_index(), 1);
    assert!(
        scroll > 0,
        "the pane must be scrolled for this to mean anything"
    );

    open_picker(&mut app);
    select_labelled(&mut app, "Revisions of api");
    let effects = submit(&mut app);

    assert!(effects.is_empty());
    assert!(app.root.jump_picker.is_none(), "the modal still closes");
    assert_eq!(
        app.root.resources.active_cloud_run_detail_tab(),
        CloudRunDetailTab::Revisions
    );
    assert_eq!(
        app.root.resources.selected_cloud_run_revision_index(),
        1,
        "the revision cursor must survive a jump to where it already is"
    );
    assert_eq!(app.root.resources.detail_scroll(), scroll);
    assert_eq!(app.root.resources.detail_field_index(), Some(1));
}

/// The `before != after` guard in `reduce` is what keeps this honest: a jump to
/// the tab that is already on screen moves nothing, so it must not push a
/// history entry that Back would then consume as a no-op step.
#[test]
fn a_jump_to_the_tab_already_on_screen_records_no_history() {
    let mut app = app_with_compute();
    reduce(&mut app, &Action::Resources(ResourcesAction::OpenSelected));
    reduce(&mut app, &Action::Resources(ResourcesAction::SelectNextTab));
    assert_eq!(
        app.root.resources.active_detail_tab(),
        ComputeDetailTab::Network
    );
    let history_before = app.root.navigation.clone();

    open_picker(&mut app);
    select_labelled(&mut app, "Network of web-01");
    submit(&mut app);

    assert_eq!(app.root.navigation, history_before);
}

/// Characterization guard for the reused open-detail path (not TDD-driven —
/// `apply_detail` already had this behaviour; the point is that routing jumps
/// through it inherits the behaviour instead of re-inventing it).
///
/// The picker holds the keyboard but not the effect queue: a list reload can
/// land while it is up. When the offered instance is gone by the time Enter is
/// pressed, the jump must say so and stay on the list rather than opening a
/// pane for a resource that no longer exists.
#[test]
fn a_jump_whose_resource_vanished_reports_it_instead_of_opening_an_empty_pane() {
    let mut app = app_with_compute();
    open_picker(&mut app);
    select_labelled(&mut app, "Disks of web-01");

    // A refresh lands while the picker is on screen and drops web-01.
    let context = app.root.current_context().expect("context exists");
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ComputeInstancesLoaded {
            context,
            instances: vec![instance("web-02")],
        }),
    );

    let effects = submit(&mut app);

    assert!(effects.is_empty(), "nothing to fetch for a vanished row");
    assert!(!app.root.resources.is_showing_detail());
    assert_eq!(status(&app), Some("web-01 is no longer available"));
}

/// A configured rule can name a service this session has never loaded, which
/// the fixed picker could never do. "web-01-cluster is no longer available"
/// asserts a history that never happened — nothing was ever there to lose — so
/// that path has to say what is actually true.
///
/// It also has to *make* it true: the destination's list load is emitted, or
/// the message describes a dead end as though it were a transient state. With
/// `poll_interval_secs = 0` nothing is in flight and nothing ever would be.
#[test]
fn a_jump_into_a_never_loaded_service_says_so_and_fetches_that_services_list() {
    let mut app = app_with_compute();
    let context = app.root.current_context().expect("context exists");
    with_rules(
        &mut app,
        "[[jumps]]\nfrom = \"compute\"\nto = \"gke:node-pools\"\nname = \"{name}-cluster\"\n",
    );

    open_picker(&mut app);
    let effects = submit(&mut app);

    // The save is the jump's own service change being remembered against "dev":
    // a jump selects the destination service, so the choke point in
    // `reduce` learns it like any other switch.
    assert_eq!(
        effects,
        vec![
            Effect::RefreshGkeClusters {
                context,
                filter: None
            },
            Effect::SaveRememberedViews {
                views: remembered("dev", ResourceServiceKind::GkeClusters),
            }
        ],
        "the destination list must be fetched, not left permanently empty"
    );
    assert!(!app.root.resources.is_showing_detail());
    // The jump is not carried across the load and replayed, so the message has
    // to hand the next step back to the user rather than describe a dead end.
    assert_eq!(
        status(&app),
        Some(
            "Loading GKE clusters, so web-01-cluster cannot be opened yet — try again once they arrive"
        )
    );
}

/// The repeat case, on the path that really does re-enter `apply_detail` as
/// often as the user asks: the jump itself. A destination whose last list load
/// *failed* is one the runtime is holding behind its retry backoff, so
/// repeating the jump must not walk around that gate — a per-attempt "does not
/// fetch" check would pass while attempt two onwards hammered an already-broken
/// backend, so the total across every attempt is what is counted.
#[test]
fn repeating_a_jump_into_a_service_whose_list_failed_never_re_fetches_it() {
    let mut app = app_with_compute();
    let context = app.root.current_context().expect("context exists");
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::GkeClustersLoadFailed {
            context,
            error: nuvem::app::AppError::gcloud("container API disabled"),
        }),
    );
    with_rules(
        &mut app,
        "[[jumps]]\nfrom = \"compute\"\nto = \"gke:node-pools\"\nname = \"{name}-cluster\"\n",
    );

    let mut fetches = 0;
    for _ in 0..20 {
        // A failed jump leaves the user on the destination's list, so getting
        // back to the instance is part of repeating the jump.
        select_service(&mut app, ResourceServiceKind::ComputeInstances);
        open_picker(&mut app);
        fetches += submit(&mut app)
            .iter()
            .filter(|effect| matches!(effect, Effect::RefreshGkeClusters { .. }))
            .count();
    }

    assert_eq!(
        fetches, 0,
        "the retry backoff owns a failed list, not the jump path"
    );
    assert_eq!(
        status(&app),
        Some("No GKE cluster loaded yet, so web-01-cluster cannot be opened"),
        "every attempt must really have reached the open, or the count means nothing"
    );
}

/// The `Alt-←`/`Alt-→` half of the test below.
///
/// Both paths reach the same `backing_list_load`, which is exactly why this is
/// pinned separately: sharing a helper today is not a guarantee that an edit to
/// the jump path tomorrow carries the restore path with it.
///
/// The inherited-failure state is assembled rather than acted out, because a
/// configuration switch resets navigation history — so there is no sequence of
/// actions that both leaves history holding a GKE detail *and* leaves GKE
/// carrying another configuration's failure. What is set up is precisely the
/// state the switch produces for that service: rows and context stamp gone,
/// `Failed` still standing and belonging to `dev`.
#[test]
fn a_history_restore_is_not_suppressed_by_a_failure_inherited_from_another_configuration() {
    let mut app = app_with_compute();
    let dev = app.root.current_context().expect("context exists");
    activate(&mut app, "prod");
    let prod = app.root.current_context().expect("context exists");
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::GkeClustersLoaded {
            context: prod.clone(),
            clusters: vec![gke_cluster("platform-dev")],
        }),
    );
    select_service(&mut app, ResourceServiceKind::GkeClusters);
    reduce(&mut app, &Action::Resources(ResourcesAction::OpenSelected));
    reduce(&mut app, &Action::Navigation(NavAction::Back));
    assert!(
        app.root.navigation.can_go_forward(),
        "the restore this test performs must be in history"
    );

    app.root
        .resources
        .mark_active_stale_and_clear(ResourceServiceKind::GkeClusters);
    app.root.resources.fail_load(
        ResourceServiceKind::GkeClusters,
        dev,
        "container API disabled".to_string(),
    );

    let effects = reduce(&mut app, &Action::Navigation(NavAction::Forward));

    assert_eq!(
        effects,
        vec![Effect::RefreshGkeClusters {
            context: prod,
            filter: None
        }],
        "prod's GKE backend has never been called, so the restore must call it"
    );
}

/// A failure belonging to a configuration the user has left must not speak for
/// the one they are in.
///
/// The `Failed` load phase survives a configuration switch on purpose (the
/// fetch behind a `Loading` phase really is still in flight at that moment), so
/// consulting it without asking *which* configuration it failed in would put
/// the empty-list-with-nothing-in-flight dead end back, this time for a backend
/// the new configuration has never called.
#[test]
fn a_jump_is_not_suppressed_by_a_failure_inherited_from_another_configuration() {
    let mut app = app_with_compute();
    let dev = app.root.current_context().expect("context exists");
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::GkeClustersLoadFailed {
            context: dev,
            error: nuvem::app::AppError::gcloud("container API disabled"),
        }),
    );
    activate(&mut app, "prod");
    load_compute(&mut app);
    let prod = app.root.current_context().expect("context exists");
    with_rules(
        &mut app,
        "[[jumps]]\nfrom = \"compute\"\nto = \"gke:node-pools\"\nname = \"{name}-cluster\"\n",
    );

    open_picker(&mut app);
    let effects = submit(&mut app);

    // Plus the jump's remembered-view save, here against "prod".
    assert_eq!(
        effects,
        vec![
            Effect::RefreshGkeClusters {
                context: prod,
                filter: None
            },
            Effect::SaveRememberedViews {
                views: remembered("prod", ResourceServiceKind::GkeClusters),
            }
        ],
        "prod's GKE backend has never been called, so it must be"
    );
}

/// The other half of the same fix: making an inherited failure non-suppressing
/// must not make failure non-suppressing *everywhere*, or the anti-hammering
/// guarantee is quietly undone. A failure recorded in the configuration the
/// user is actually in still gates every repeat.
#[test]
fn repeating_a_jump_after_a_failure_in_the_current_configuration_never_re_fetches() {
    let mut app = app_with_compute();
    activate(&mut app, "prod");
    load_compute(&mut app);
    let prod = app.root.current_context().expect("context exists");
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::GkeClustersLoadFailed {
            context: prod,
            error: nuvem::app::AppError::gcloud("container API disabled"),
        }),
    );
    with_rules(
        &mut app,
        "[[jumps]]\nfrom = \"compute\"\nto = \"gke:node-pools\"\nname = \"{name}-cluster\"\n",
    );

    let mut fetches = 0;
    for _ in 0..20 {
        select_service(&mut app, ResourceServiceKind::ComputeInstances);
        open_picker(&mut app);
        fetches += submit(&mut app)
            .iter()
            .filter(|effect| matches!(effect, Effect::RefreshGkeClusters { .. }))
            .count();
    }

    assert_eq!(
        fetches, 0,
        "a failure in the current configuration still belongs to the retry backoff"
    );
    assert_eq!(
        status(&app),
        Some("No GKE cluster loaded yet, so web-01-cluster cannot be opened"),
        "every attempt must really have reached the open, or the count means nothing"
    );
}

/// The same jump while that service's list is *already* in flight. A second
/// fetch would buy nothing — the rows one load short of arriving are the rows
/// the other load would return — so no effect is emitted, while the message
/// stays the one for rows that are coming: what the user does next is the same
/// either way.
#[test]
fn a_jump_into_a_service_whose_list_is_already_loading_does_not_fetch_it_again() {
    let mut app = app_with_compute();
    app.root
        .resources
        .begin_load(ResourceServiceKind::GkeClusters, std::time::Instant::now());
    with_rules(
        &mut app,
        "[[jumps]]\nfrom = \"compute\"\nto = \"gke:node-pools\"\nname = \"{name}-cluster\"\n",
    );

    open_picker(&mut app);
    let effects = submit(&mut app);

    // The jump still changes the active service, so it still emits the
    // remembered-view save. What this test pins is that no *list
    // load* is emitted.
    assert_eq!(
        effects,
        vec![Effect::SaveRememberedViews {
            views: remembered("dev", ResourceServiceKind::GkeClusters),
        }],
        "a load is already in flight for that list"
    );
    assert_eq!(
        status(&app),
        Some(
            "Loading GKE clusters, so web-01-cluster cannot be opened yet — try again once they arrive"
        )
    );
}

/// The other direction, and the one that keeps the fetch above from becoming
/// an unconditional refetch: a cross-service jump into a list the user has
/// already loaded emits that service's *detail* fetch and nothing else.
#[test]
fn a_jump_into_an_already_loaded_service_emits_no_extra_list_load() {
    let mut app = app_with_compute();
    let context = app.root.current_context().expect("context exists");
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::GkeClustersLoaded {
            context: context.clone(),
            clusters: vec![gke_cluster("web-01-cluster")],
        }),
    );
    select_service(&mut app, ResourceServiceKind::ComputeInstances);
    with_rules(
        &mut app,
        "[[jumps]]\nfrom = \"compute\"\nto = \"gke:node-pools\"\nname = \"{name}-cluster\"\nlocation = \"europe-west1\"\n",
    );

    open_picker(&mut app);
    let effects = submit(&mut app);

    assert!(app.root.resources.is_showing_detail());
    // Plus the jump's remembered-view save; no extra *list* load.
    assert_eq!(
        effects,
        vec![
            Effect::LoadGkeClusterDetail {
                context,
                cluster: gke_cluster("web-01-cluster"),
            },
            Effect::SaveRememberedViews {
                views: remembered("dev", ResourceServiceKind::GkeClusters),
            }
        ]
    );
}

// ---------------------------------------------------------------------------
// `select_detail_tab` only ever writes the service on screen
// ---------------------------------------------------------------------------

/// The tab-change half of a jump writes whichever service sub-state the
/// [`DetailTab`] variant names. Its one caller pre-checks that the pane on
/// screen is that service's, but the method is public and cheap to call: a
/// mismatched pair must be a no-op rather than a silent write to a pane the
/// user is not looking at, which would surface later as a detail opening on a
/// tab nobody chose.
#[test]
fn selecting_a_detail_tab_of_another_service_leaves_both_services_alone() {
    let mut app = app_with_compute();
    assert_eq!(
        app.root.resources.active_service(),
        ResourceServiceKind::ComputeInstances
    );
    let compute_before = app.root.resources.active_detail_tab();
    let cloud_sql_before = app.root.resources.active_cloud_sql_detail_tab();
    assert_ne!(cloud_sql_before, CloudSqlDetailTab::Replicas);

    app.root
        .resources
        .select_detail_tab(DetailTab::CloudSql(CloudSqlDetailTab::Replicas));

    assert_eq!(
        app.root.resources.active_cloud_sql_detail_tab(),
        cloud_sql_before,
        "the off-screen Cloud SQL pane must not have moved"
    );
    assert_eq!(
        app.root.resources.active_detail_tab(),
        compute_before,
        "and the service on screen must not have moved either"
    );
}

/// The matching case still works, so the guard above cannot be satisfied by
/// making the method inert.
#[test]
fn selecting_a_detail_tab_of_the_active_service_still_switches_it() {
    let mut app = app_with_compute();

    app.root
        .resources
        .select_detail_tab(DetailTab::Compute(ComputeDetailTab::Disks));

    assert_eq!(
        app.root.resources.active_detail_tab(),
        ComputeDetailTab::Disks
    );
}
