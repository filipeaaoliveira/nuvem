//! Evidence that a resource whose name would be read as an option never
//! reaches a child's argv.
//!
//! Three launches interpolate values nuvem did not author into a command line
//! — the managed tunnels, the GCE SSH session, and the k9s `get-credentials`
//! step — and none of them can escape a leading `-`, because gcloud spends
//! `--` on ssh passthrough and has nothing left to mean end-of-options (see
//! `nuvem_domain::gcp::argv`). Each therefore refuses at the point the launch
//! is *offered*, and this file pins that per seam: the unit tests next to the
//! predicate say what the rule is, and these say the rule is actually reached.
//!
//! Every case here is deliberately impossible against a well-behaved GCP —
//! resource names must begin with a letter — which is the point. The refusal
//! exists for the parse that went wrong and the `gcloud` that is not the one
//! we think it is, so nothing but a test will ever exercise it.

use nuvem::app::reducer::root::reduce;
use nuvem::app::state::command::CommandDescriptor;
use nuvem::app::{
    Action, AppState, CommandAction, Effect, EffectResult, ResourcesAction, WorkspaceAction,
};
use nuvem_domain::gcp::configuration::{
    GcloudConfiguration, GcloudConfigurationName, GcloudContext,
};
use nuvem_domain::gcp::k9s::resolve_launch_target;
use nuvem_domain::gcp::resource::{
    CloudSqlAvailability, CloudSqlInstance, CloudSqlStatus, ComputeInstance, EndpointVisibility,
    GkeCluster, GkeClusterStatus, ResourceServiceKind, ResourceStatus,
};

fn configuration(project: Option<&str>) -> GcloudConfiguration {
    GcloudConfiguration {
        name: GcloudConfigurationName::new("dev"),
        is_active: true,
        account: None,
        project: project.map(str::to_string),
        region: None,
        zone: None,
    }
}

fn instance(name: &str, zone: &str) -> ComputeInstance {
    ComputeInstance {
        id: "1001".to_string(),
        name: name.to_string(),
        zone: zone.to_string(),
        status: ResourceStatus::Running,
        machine_type: "e2-medium".to_string(),
        internal_ip: Some("10.0.0.2".to_string()),
        // No external IP, so the tunnel resolves to IAP — the branch that
        // carries the instance name positionally.
        external_ip: None,
    }
}

fn cloud_sql_instance(name: &str) -> CloudSqlInstance {
    CloudSqlInstance {
        name: name.to_string(),
        database_version: "POSTGRES_15".to_string(),
        engine: "POSTGRES".to_string(),
        region: "europe-west1".to_string(),
        tier: "db-custom-2-7680".to_string(),
        availability: CloudSqlAvailability::Zonal,
        status: CloudSqlStatus::Runnable,
        connectivity: EndpointVisibility::Public,
    }
}

fn app_with_configurations(project: Option<&str>) -> AppState {
    let mut app = AppState::new();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ConfigurationsLoaded {
            configurations: vec![configuration(project)],
        }),
    );
    app
}

fn context_of(app: &AppState) -> GcloudContext {
    app.root.current_context().expect("an active context")
}

fn app_with_compute(instances: Vec<ComputeInstance>) -> AppState {
    let mut app = app_with_configurations(Some("dev-project"));
    let context = context_of(&app);
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ComputeInstancesLoaded { context, instances }),
    );
    // The SSH palette entry is only offered from the Resources workspace.
    reduce(&mut app, &Action::Workspace(WorkspaceAction::Resources));
    app
}

fn app_with_cloud_sql(project: Option<&str>, instances: Vec<CloudSqlInstance>) -> AppState {
    let mut app = app_with_configurations(project);
    reduce(
        &mut app,
        &Action::Resources(ResourcesAction::SelectService(
            ResourceServiceKind::CloudSql,
        )),
    );
    let context = context_of(&app);
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::CloudSqlInstancesLoaded { context, instances }),
    );
    app
}

fn request_tunnel(app: &mut AppState) -> Vec<Effect> {
    reduce(app, &Action::Resources(ResourcesAction::RequestTunnel))
}

fn status_message(app: &AppState) -> String {
    app.root
        .system
        .status_message
        .clone()
        .expect("a refusal states its reason")
}

fn submit_from_palette(app: &mut AppState, descriptor: CommandDescriptor) -> Vec<Effect> {
    reduce(app, &Action::Command(CommandAction::Open));
    for ch in descriptor.label().chars() {
        reduce(app, &Action::Command(CommandAction::AppendChar(ch)));
    }
    assert_eq!(
        app.root.command.selected_visible_command(&app.root),
        Some(descriptor),
        "the palette must have the command under test selected"
    );
    reduce(app, &Action::Command(CommandAction::Submit))
}

// --- The managed tunnels ---

/// The instance name is the positional argument of `start-iap-tunnel`, so a
/// leading dash turns the target into a flag.
#[test]
fn a_tunnel_is_refused_when_the_instance_name_would_read_as_an_option() {
    let mut app = app_with_compute(vec![instance("--zone", "europe-west1-b")]);

    let effects = request_tunnel(&mut app);

    assert!(effects.is_empty(), "no launch effect may be emitted");
    assert!(
        app.root.resources.tunnel_launch_preview().is_none(),
        "no confirmation may be opened for an argv that cannot be built safely"
    );
    assert!(
        status_message(&app).contains("--zone"),
        "the refusal must name the offending value: {}",
        status_message(&app)
    );
}

/// The zone is `--zone`'s value rather than a positional, and is refused for
/// the same reason: gcloud reads the token after `--zone` as the next option
/// when it begins with a dash, not as the zone.
#[test]
fn a_tunnel_is_refused_when_the_zone_would_read_as_an_option() {
    let mut app = app_with_compute(vec![instance("web-1", "-b")]);

    let effects = request_tunnel(&mut app);

    assert!(effects.is_empty());
    assert!(app.root.resources.tunnel_launch_preview().is_none());
    assert!(status_message(&app).contains("-b"));
}

/// The Cloud SQL proxy takes the `project:region:instance` triple positionally,
/// and the project leads it — so a dashed project is what puts a dash at the
/// front of the composed value.
#[test]
fn a_cloud_sql_tunnel_is_refused_when_the_connection_name_would_read_as_an_option() {
    let mut app = app_with_cloud_sql(Some("-project"), vec![cloud_sql_instance("db-1")]);

    let effects = request_tunnel(&mut app);

    assert!(effects.is_empty());
    assert!(app.root.resources.tunnel_launch_preview().is_none());
    assert!(
        status_message(&app).contains("-project:europe-west1:db-1"),
        "the composed triple is the value that reaches argv: {}",
        status_message(&app)
    );
}

/// The guard must not cost the ordinary case its tunnel: interior dashes are
/// what almost every real GCP name looks like.
#[test]
fn an_ordinary_dashed_name_still_opens_its_tunnel_confirmation() {
    let mut app = app_with_compute(vec![instance("web-server-1", "europe-west1-b")]);

    let effects = request_tunnel(&mut app);

    assert!(effects.is_empty(), "requesting only opens the confirmation");
    assert!(
        app.root.resources.tunnel_launch_preview().is_some(),
        "a name with interior dashes is an ordinary name"
    );
}

// --- The GCE SSH session ---

/// Refused before the preview opens rather than at confirm: the preview exists
/// to show the argv that will run, and there is no honest argv to show here.
#[test]
fn the_ssh_preview_never_opens_for_a_name_that_would_read_as_an_option() {
    let mut app = app_with_compute(vec![instance("-i", "europe-west1-b")]);

    let effects = submit_from_palette(&mut app, CommandDescriptor::SshSelectedComputeInstance);

    assert!(effects.is_empty());
    assert!(
        app.root.resources.ssh_launch_preview().is_none(),
        "no SSH confirmation may be opened for an unsafe name"
    );
    assert!(status_message(&app).contains("-i"));
}

#[test]
fn an_ordinary_instance_still_opens_its_ssh_preview() {
    let mut app = app_with_compute(vec![instance("web-1", "europe-west1-b")]);

    submit_from_palette(&mut app, CommandDescriptor::SshSelectedComputeInstance);

    assert!(app.root.resources.ssh_launch_preview().is_some());
}

// --- The k9s credentials step ---

fn cluster(name: &str, location: &str) -> GkeCluster {
    GkeCluster {
        name: name.to_string(),
        location: location.to_string(),
        status: GkeClusterStatus::Running,
        current_master_version: Some("1.30.1".to_string()),
        current_node_count: Some(3),
        endpoint_visibility: EndpointVisibility::Public,
    }
}

fn context_with_project(project: &str) -> GcloudContext {
    GcloudContext {
        configuration: GcloudConfigurationName::new("dev"),
        account: None,
        project: Some(project.to_string()),
        region: None,
        zone: None,
    }
}

/// `resolve_launch_target` already refused empty components by returning
/// `None`; an option-like one is refused the same way, because the answer is
/// the same — there is no launch to offer.
#[test]
fn no_k9s_target_resolves_when_the_cluster_name_would_read_as_an_option() {
    assert!(
        resolve_launch_target(
            &context_with_project("dev-project"),
            &cluster("-x", "us-c1")
        )
        .is_none()
    );
}

#[test]
fn no_k9s_target_resolves_when_the_location_would_read_as_an_option() {
    assert!(
        resolve_launch_target(
            &context_with_project("dev-project"),
            &cluster("prod", "-us")
        )
        .is_none()
    );
}

#[test]
fn no_k9s_target_resolves_when_the_project_would_read_as_an_option() {
    assert!(
        resolve_launch_target(&context_with_project("-p"), &cluster("prod", "us-c1")).is_none()
    );
}

#[test]
fn an_ordinary_cluster_still_resolves_a_k9s_target() {
    let target = resolve_launch_target(
        &context_with_project("dev-project"),
        &cluster("prod-cluster", "europe-west1"),
    )
    .expect("an ordinary cluster resolves");

    assert_eq!(target.cluster_name, "prod-cluster");
}
