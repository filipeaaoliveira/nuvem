//! Starting a managed tunnel from a selected resource row.
//!
//! This is the only path that emits [`Effect::StartTunnel`], so the plan
//! resolution per service (including the IAP auto-detection), the confirmation
//! preview's lifetime, and the `Starting` row the reducer inserts are all pinned
//! here rather than in the per-module unit tests.

use std::time::Instant;

use nuvem::app::reducer::root::reduce;
use nuvem::app::state::tunnels::TunnelStatus;
use nuvem::app::{Action, AppState, Effect, EffectResult, ResourcesAction};
use nuvem::input::Mode;
use nuvem::services::runtime::input_context_for_test;
use nuvem::services::runtime::session::RuntimeSession;
use nuvem_domain::gcp::configuration::{
    GcloudConfiguration, GcloudConfigurationName, GcloudContext,
};
use nuvem_domain::gcp::resource::{
    CloudRunService, CloudRunServiceStatus, CloudSqlAvailability, CloudSqlInstance, CloudSqlStatus,
    ComputeInstance, EndpointVisibility, GkeCluster, GkeClusterStatus, ResourceServiceKind,
    ResourceStatus, StorageBucket,
};
use nuvem_domain::gcp::tunnel::TunnelKind;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::layout::Rect;

fn configuration(name: &str, active: bool, project: Option<&str>) -> GcloudConfiguration {
    GcloudConfiguration {
        name: GcloudConfigurationName::new(name),
        is_active: active,
        account: None,
        project: project.map(str::to_string),
        region: None,
        zone: None,
    }
}

fn instance(name: &str, external_ip: Option<&str>) -> ComputeInstance {
    ComputeInstance {
        id: name.to_string(),
        name: name.to_string(),
        zone: "europe-west1-b".to_string(),
        status: ResourceStatus::Running,
        machine_type: "e2-medium".to_string(),
        internal_ip: Some("10.0.0.2".to_string()),
        external_ip: external_ip.map(str::to_string),
    }
}

fn cloud_sql_instance(name: &str, database_version: &str, engine: &str) -> CloudSqlInstance {
    CloudSqlInstance {
        name: name.to_string(),
        database_version: database_version.to_string(),
        engine: engine.to_string(),
        region: "europe-west1".to_string(),
        tier: "db-custom-2-7680".to_string(),
        availability: CloudSqlAvailability::Zonal,
        status: CloudSqlStatus::Runnable,
        connectivity: EndpointVisibility::Public,
    }
}

fn app_with_configurations(configurations: Vec<GcloudConfiguration>) -> AppState {
    let mut app = AppState::new();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ConfigurationsLoaded { configurations }),
    );
    app
}

fn context_of(app: &AppState) -> GcloudContext {
    app.root.current_context().expect("an active context")
}

fn app_with_compute(instances: Vec<ComputeInstance>) -> AppState {
    let mut app = app_with_configurations(vec![configuration("dev", true, Some("dev-project"))]);
    let context = context_of(&app);
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ComputeInstancesLoaded { context, instances }),
    );
    app
}

fn app_with_cloud_sql(project: Option<&str>, instances: Vec<CloudSqlInstance>) -> AppState {
    let mut app = app_with_configurations(vec![configuration("dev", true, project)]);
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

fn confirm_tunnel(app: &mut AppState, started_at: Instant) -> Vec<Effect> {
    reduce(
        app,
        &Action::Resources(ResourcesAction::ConfirmTunnelLaunch { started_at }),
    )
}

#[test]
fn request_tunnel_on_a_gce_row_without_an_external_ip_plans_an_iap_tunnel() {
    let mut app = app_with_compute(vec![instance("api-dev-1", None)]);

    let effects = request_tunnel(&mut app);

    assert!(effects.is_empty(), "the preview is state, not an effect");
    let preview = app
        .root
        .resources
        .tunnel_launch_preview()
        .expect("a tunnel preview is open");
    let plan = preview.plan();
    assert_eq!(plan.kind, TunnelKind::IapTcp);
    assert_eq!(plan.instance_name, "api-dev-1");
    assert_eq!(plan.zone, "europe-west1-b");
    assert_eq!(plan.remote_port, 22);
    assert_eq!(plan.preferred_local_port, None);
    assert_eq!(plan.target_label, "api-dev-1");
    assert_eq!(plan.remote_label, "22");
    assert_eq!(preview.context().configuration.as_str(), "dev");
}

#[test]
fn request_tunnel_on_a_gce_row_with_an_external_ip_plans_an_ssh_local_forward() {
    let mut app = app_with_compute(vec![instance("api-dev-1", Some("34.1.2.3"))]);

    request_tunnel(&mut app);

    let plan = app
        .root
        .resources
        .tunnel_launch_preview()
        .expect("a tunnel preview is open")
        .plan()
        .clone();
    assert_eq!(plan.kind, TunnelKind::SshLocalForward);
    assert_eq!(plan.remote_port, 22);
    assert_eq!(plan.preferred_local_port, None);
}

#[test]
fn request_tunnel_on_a_cloud_sql_row_plans_a_proxy_with_the_engine_ports() {
    let mut app = app_with_cloud_sql(
        Some("dev-project"),
        vec![cloud_sql_instance("db-dev-1", "POSTGRES_15", "PostgreSQL")],
    );

    request_tunnel(&mut app);

    let plan = app
        .root
        .resources
        .tunnel_launch_preview()
        .expect("a tunnel preview is open")
        .plan()
        .clone();
    assert_eq!(plan.kind, TunnelKind::CloudSqlProxy);
    assert_eq!(plan.connection_name, "dev-project:europe-west1:db-dev-1");
    assert_eq!(plan.preferred_local_port, Some(5432));
    assert_eq!(plan.remote_port, 5432);
    assert_eq!(plan.target_label, "db-dev-1");
}

#[test]
fn request_tunnel_on_a_mysql_row_plans_the_mysql_port() {
    let mut app = app_with_cloud_sql(
        Some("dev-project"),
        vec![cloud_sql_instance("db-dev-1", "MYSQL_8_0", "MySQL")],
    );

    request_tunnel(&mut app);

    let plan = app
        .root
        .resources
        .tunnel_launch_preview()
        .expect("a tunnel preview is open")
        .plan()
        .clone();
    assert_eq!(plan.preferred_local_port, Some(3306));
    assert_eq!(plan.remote_port, 3306);
}

#[test]
fn request_tunnel_on_a_cloud_sql_row_without_a_project_says_so_and_opens_nothing() {
    let mut app = app_with_cloud_sql(
        None,
        vec![cloud_sql_instance("db-dev-1", "POSTGRES_15", "PostgreSQL")],
    );

    let effects = request_tunnel(&mut app);

    assert!(effects.is_empty());
    assert!(
        app.root.resources.tunnel_launch_preview().is_none(),
        "a connection name cannot be built without a project"
    );
    assert!(
        app.root
            .system
            .status_message
            .as_ref()
            .is_some_and(|message| message.to_lowercase().contains("project")),
        "the user must be told why nothing happened, got {:?}",
        app.root.system.status_message
    );
}

#[test]
fn request_tunnel_is_a_silent_no_op_for_services_with_no_tunnel_plan() {
    for (service, load) in [
        (ResourceServiceKind::GkeClusters, "gke"),
        (ResourceServiceKind::CloudRunServices, "run"),
        (ResourceServiceKind::CloudStorage, "storage"),
    ] {
        let mut app =
            app_with_configurations(vec![configuration("dev", true, Some("dev-project"))]);
        reduce(
            &mut app,
            &Action::Resources(ResourcesAction::SelectService(service)),
        );
        let context = context_of(&app);
        match load {
            "gke" => {
                reduce(
                    &mut app,
                    &Action::EffectResult(EffectResult::GkeClustersLoaded {
                        context,
                        clusters: vec![GkeCluster {
                            name: "platform-dev".to_string(),
                            location: "europe-west1".to_string(),
                            status: GkeClusterStatus::Running,
                            current_master_version: None,
                            current_node_count: None,
                            endpoint_visibility: EndpointVisibility::Public,
                        }],
                    }),
                );
            }
            "run" => {
                reduce(
                    &mut app,
                    &Action::EffectResult(EffectResult::CloudRunServicesLoaded {
                        context,
                        services: vec![CloudRunService {
                            name: "checkout".to_string(),
                            region: "europe-west1".to_string(),
                            url: None,
                            latest_ready_revision: None,
                            latest_created_revision: None,
                            traffic_summary: String::new(),
                            status: CloudRunServiceStatus::Ready,
                            ready_message: None,
                        }],
                    }),
                );
            }
            _ => {
                reduce(
                    &mut app,
                    &Action::EffectResult(EffectResult::StorageBucketsLoaded {
                        context,
                        buckets: vec![StorageBucket {
                            name: "artifacts".to_string(),
                            location: "EU".to_string(),
                            location_type: None,
                            storage_class: "STANDARD".to_string(),
                            versioning_enabled: false,
                            uniform_bucket_level_access: true,
                            public_access_prevention: None,
                            time_created: None,
                            updated: None,
                        }],
                    }),
                );
            }
        }
        app.root.system.clear_status_message();

        let effects = request_tunnel(&mut app);

        assert!(effects.is_empty(), "{service:?} must not emit an effect");
        assert!(
            app.root.resources.tunnel_launch_preview().is_none(),
            "{service:?} must not open a tunnel preview"
        );
        assert_eq!(
            app.root.system.status_message, None,
            "{service:?} must be a silent no-op, not a status message"
        );
    }
}

#[test]
fn confirming_inserts_a_starting_entry_and_emits_start_tunnel_with_the_same_id() {
    let mut app = app_with_compute(vec![instance("api-dev-1", None)]);
    request_tunnel(&mut app);
    let started_at = Instant::now();

    let effects = confirm_tunnel(&mut app, started_at);

    let entries = app.root.tunnels.entries();
    assert_eq!(entries.len(), 1);
    let entry = &entries[0];
    assert_eq!(entry.status, TunnelStatus::Starting);
    assert_eq!(entry.kind, TunnelKind::IapTcp);
    assert_eq!(entry.target_label, "api-dev-1");
    assert_eq!(entry.remote_label, "22");
    assert_eq!(entry.started_at, started_at);
    assert_eq!(entry.context.configuration.as_str(), "dev");

    let [Effect::StartTunnel { id, context, plan }] = effects.as_slice() else {
        panic!("expected exactly one StartTunnel effect, got {effects:?}");
    };
    assert_eq!(*id, entry.id, "the row and the effect share one id");
    assert_eq!(context.configuration.as_str(), "dev");
    assert_eq!(plan.kind, TunnelKind::IapTcp);
    assert!(
        app.root.resources.tunnel_launch_preview().is_none(),
        "confirming closes the preview"
    );
}

#[test]
fn confirming_carries_the_ports_already_claimed_by_live_tunnels() {
    let mut app = app_with_cloud_sql(
        Some("dev-project"),
        vec![cloud_sql_instance("db-dev-1", "POSTGRES_15", "PostgreSQL")],
    );
    let at = Instant::now();
    request_tunnel(&mut app);
    confirm_tunnel(&mut app, at);
    request_tunnel(&mut app);

    let effects = confirm_tunnel(&mut app, at);

    let [Effect::StartTunnel { plan, .. }] = effects.as_slice() else {
        panic!("expected exactly one StartTunnel effect, got {effects:?}");
    };
    assert_eq!(
        plan.claimed_ports,
        vec![5432],
        "the first tunnel's port is already promised"
    );
}

#[test]
fn confirming_without_an_open_preview_does_nothing() {
    let mut app = app_with_compute(vec![instance("api-dev-1", None)]);

    let effects = confirm_tunnel(&mut app, Instant::now());

    assert!(effects.is_empty());
    assert!(app.root.tunnels.is_empty());
}

#[test]
fn cancelling_clears_the_preview_and_says_so() {
    let mut app = app_with_compute(vec![instance("api-dev-1", None)]);
    request_tunnel(&mut app);

    let effects = reduce(
        &mut app,
        &Action::Resources(ResourcesAction::CancelTunnelLaunch),
    );

    assert!(effects.is_empty());
    assert!(app.root.resources.tunnel_launch_preview().is_none());
    assert_eq!(
        app.root.system.status_message.as_deref(),
        Some("tunnel launch cancelled")
    );
    assert!(app.root.tunnels.is_empty(), "cancelling starts nothing");
}

#[test]
fn a_service_switch_clears_the_preview() {
    let mut app = app_with_compute(vec![instance("api-dev-1", None)]);
    request_tunnel(&mut app);
    assert!(app.root.resources.tunnel_launch_preview().is_some());

    reduce(
        &mut app,
        &Action::Resources(ResourcesAction::SelectService(
            ResourceServiceKind::GkeClusters,
        )),
    );

    assert!(app.root.resources.tunnel_launch_preview().is_none());
}

#[test]
fn a_context_switch_clears_the_preview() {
    let mut app = AppState::new();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ConfigurationsLoaded {
            configurations: vec![
                configuration("dev", true, Some("dev-project")),
                configuration("prod", false, Some("prod-project")),
            ],
        }),
    );
    let context = context_of(&app);
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ComputeInstancesLoaded {
            context,
            instances: vec![instance("api-dev-1", None)],
        }),
    );
    request_tunnel(&mut app);
    assert!(app.root.resources.tunnel_launch_preview().is_some());

    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ConfigurationActivated {
            configuration: configuration("prod", true, Some("prod-project")),
        }),
    );

    assert!(app.root.resources.tunnel_launch_preview().is_none());
}

#[test]
fn opening_and_closing_a_detail_pane_clears_the_preview() {
    let mut app = app_with_compute(vec![instance("api-dev-1", None)]);
    request_tunnel(&mut app);

    reduce(&mut app, &Action::Resources(ResourcesAction::OpenSelected));
    assert!(
        app.root.resources.tunnel_launch_preview().is_none(),
        "opening a detail pane clears the preview"
    );

    reduce(&mut app, &Action::Resources(ResourcesAction::CloseDetail));
    request_tunnel(&mut app);
    // The list view is back, so a preview can be opened again; closing the
    // detail must clear it too.
    assert!(app.root.resources.tunnel_launch_preview().is_some());
    reduce(&mut app, &Action::Resources(ResourcesAction::OpenSelected));
    reduce(&mut app, &Action::Resources(ResourcesAction::CloseDetail));
    assert!(app.root.resources.tunnel_launch_preview().is_none());
}

/// The keyboard cannot reach this today (the open preview owns input, and only
/// Enter/Esc are handled), but the Cloud SQL detail opener must clear the
/// preview like its Compute and GKE siblings do: a preview surviving into a
/// detail pane would render over a view it does not belong to.
#[test]
fn opening_a_cloud_sql_detail_pane_clears_the_preview() {
    let mut app = app_with_cloud_sql(
        Some("dev-project"),
        vec![cloud_sql_instance("db-dev-1", "POSTGRES_15", "PostgreSQL")],
    );
    request_tunnel(&mut app);
    assert!(app.root.resources.tunnel_launch_preview().is_some());

    app.root
        .resources
        .open_cloud_sql_detail()
        .expect("the selected instance opens a detail pane");

    assert!(
        app.root.resources.tunnel_launch_preview().is_none(),
        "opening a Cloud SQL detail pane leaves a stale tunnel preview"
    );
}

#[test]
fn replacing_cloud_sql_rows_for_a_new_context_clears_the_preview() {
    let mut app = app_with_cloud_sql(
        Some("dev-project"),
        vec![cloud_sql_instance("db-dev-1", "POSTGRES_15", "PostgreSQL")],
    );
    request_tunnel(&mut app);
    assert!(app.root.resources.tunnel_launch_preview().is_some());

    app.root
        .resources
        .mark_active_stale_and_clear(ResourceServiceKind::CloudSql);

    assert!(app.root.resources.tunnel_launch_preview().is_none());
}

/// Renders through the real `nuvem::ui::render` entrypoint and returns the
/// trimmed screen lines.
fn render_lines(session: &RuntimeSession, width: u16, height: u16) -> Vec<String> {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("terminal");
    terminal
        .draw(|frame| nuvem::ui::render(frame, session))
        .expect("render");
    let buffer = terminal.backend().buffer();
    let w = buffer.area.width as usize;
    buffer
        .content()
        .chunks(w)
        .map(|line| {
            line.iter()
                .map(|cell| cell.symbol())
                .collect::<String>()
                .trim_end()
                .to_string()
        })
        .collect()
}

fn session_with_open_preview() -> RuntimeSession {
    let mut app = app_with_compute(vec![instance("api-dev-1", None)]);
    request_tunnel(&mut app);
    RuntimeSession::new_for_test(app)
}

#[test]
fn the_open_preview_renders_the_argv_and_owns_the_input_mode() {
    let session = session_with_open_preview();
    let area = Rect::new(0, 0, 160, 40);

    let screen = render_lines(&session, area.width, area.height).join("\n");
    assert!(
        screen.contains("start tunnel"),
        "the modal chrome is missing:\n{screen}"
    );
    assert!(
        screen.contains("start-iap-tunnel"),
        "the argv is missing:\n{screen}"
    );

    let context = input_context_for_test(&session, area);
    assert_eq!(context.mode, Mode::TunnelLaunchPreview);
    assert!(context.tunnel_launch_preview_open);
    assert!(context.tunnel_launch_preview_visible);
}

/// The same small-terminal fallback the other launch modals use: the argv is
/// replaced by a "needs a larger terminal" notice, and the confirm key is
/// withheld because the user cannot read what they would be accepting.
#[test]
fn a_terminal_too_small_for_the_preview_gets_the_fallback_and_no_confirm_key() {
    let session = session_with_open_preview();
    let area = Rect::new(0, 0, 70, 14);

    let screen = render_lines(&session, area.width, area.height).join("\n");
    assert!(
        screen.contains("needs a larger terminal"),
        "expected the small-terminal fallback:\n{screen}"
    );

    let context = input_context_for_test(&session, area);
    assert_eq!(context.mode, Mode::TunnelLaunchPreview);
    assert!(context.tunnel_launch_preview_open);
    assert!(!context.tunnel_launch_preview_visible);
}
