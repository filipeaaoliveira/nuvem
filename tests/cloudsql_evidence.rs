//! Evidence generator for issue #33 — Cloud SQL resource service.
//!
//! Renders the real end-user surfaces through `nuvem::ui::render` (the exact
//! terminal buffer an operator sees) and writes ASCII "screenshots" to the
//! evidence directory when `NUVEM_EVIDENCE_DIR` is set (assertions always run).
//! Demonstrates: read-only list + detail tabs, the shared
//! CONFIRMED action modal showing the exact gcloud argv for Stop
//! (activation-policy=NEVER) and Failover, failover eligibility gating in the
//! command palette (Regional/HA offers it, Zonal never does), and render-level
//! secret redaction across the Users and Raw JSON tabs.

use std::fs;
use std::path::PathBuf;

use nuvem::app::{Action, AppState, CommandAction, EffectResult, ResourcesAction, WorkspaceAction};
use nuvem::services::runtime::session::RuntimeSession;
use nuvem_domain::gcp::configuration::{GcloudConfiguration, GcloudConfigurationName};
use nuvem_domain::gcp::resource::{
    CloudSqlAvailability, CloudSqlInstance, CloudSqlStatus, EndpointVisibility, ResourceServiceKind,
};
use ratatui::{Terminal, backend::TestBackend};

fn evidence_dir() -> Option<PathBuf> {
    let dir = std::env::var("NUVEM_EVIDENCE_DIR").ok()?;
    let path = PathBuf::from(dir);
    fs::create_dir_all(&path).ok()?;
    Some(path)
}

fn write_evidence(name: &str, header: &str, lines: &[String]) {
    let Some(dir) = evidence_dir() else {
        return;
    };
    let mut body = String::new();
    body.push_str(header);
    body.push_str("\n\n");
    let width = lines.iter().map(|l| l.chars().count()).max().unwrap_or(0);
    body.push('+');
    body.push_str(&"-".repeat(width));
    body.push_str("+\n");
    for line in lines {
        body.push('|');
        body.push_str(line);
        body.push_str("|\n");
    }
    body.push('+');
    body.push_str(&"-".repeat(width));
    body.push_str("+\n");
    fs::write(dir.join(name), body).expect("write evidence");
    println!("wrote {name}");
}

fn render_lines(session: &RuntimeSession, width: u16, height: u16) -> Vec<String> {
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).expect("terminal");
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
                .map(|c| c.symbol())
                .collect::<String>()
                .trim_end()
                .to_string()
        })
        .collect()
}

async fn load_dev(session: &mut RuntimeSession) {
    session
        .apply_effect_result(EffectResult::ConfigurationsLoaded {
            configurations: vec![GcloudConfiguration {
                name: GcloudConfigurationName::new("dev"),
                is_active: true,
                account: Some("dev@example.com".to_string()),
                project: Some("dev-project".to_string()),
                region: Some("us-east1".to_string()),
                zone: None,
            }],
        })
        .await;
}

fn ha_instance() -> CloudSqlInstance {
    CloudSqlInstance {
        name: "mysql-ha".to_string(),
        database_version: "MYSQL_8_0".to_string(),
        engine: "MySQL".to_string(),
        region: "us-east1".to_string(),
        tier: "db-n1-standard-4".to_string(),
        availability: CloudSqlAvailability::Regional,
        status: CloudSqlStatus::Runnable,
        connectivity: EndpointVisibility::Private,
    }
}

async fn select_cloud_sql(session: &mut RuntimeSession, instances: Vec<CloudSqlInstance>) {
    session
        .dispatch_action(Action::Resources(ResourcesAction::SelectService(
            ResourceServiceKind::CloudSql,
        )))
        .await;
    let context = session.app().root.current_context().unwrap().clone();
    session
        .apply_effect_result(EffectResult::CloudSqlInstancesLoaded { context, instances })
        .await;
    session
        .dispatch_action(Action::Workspace(WorkspaceAction::Resources))
        .await;
}

async fn open_palette_and_type(session: &mut RuntimeSession, query: &str) {
    session
        .dispatch_action(Action::Command(CommandAction::Open))
        .await;
    for ch in query.chars() {
        session
            .dispatch_action(Action::Command(CommandAction::AppendChar(ch)))
            .await;
    }
}

/// List view + detail tabs (including redacted Users/Raw JSON), the CONFIRMED
/// Stop and Failover modals with exact gcloud argv, and the palette gating.
#[tokio::test]
async fn cloud_sql_surfaces_render_for_review() {
    // --- 1. Read-only list view -----------------------------------------
    let mut session = RuntimeSession::new_for_test(AppState::new());
    load_dev(&mut session).await;
    select_cloud_sql(&mut session, vec![ha_instance()]).await;
    write_evidence(
        "01_list_view.txt",
        "Cloud SQL read-only list view (g -> Cloud SQL): shows instance rows with engine/version/tier/availability/status.",
        &render_lines(&session, 120, 24),
    );

    // --- 2. Detail view, all tabs, with lazy-loaded users ---------------
    const DETAIL_JSON: &str = include_str!("fixtures/cloudsql_instance_detail.json");
    const USERS_JSON: &str = include_str!("fixtures/cloudsql_users.json");
    let detail = nuvem::services::resources::cloudsql::parse_instance_detail_json(DETAIL_JSON)
        .expect("detail fixture parses");
    let users =
        nuvem::services::resources::cloudsql::parse_users_json(USERS_JSON).expect("users parse");
    let context = session.app().root.current_context().unwrap().clone();

    session
        .dispatch_action(Action::Resources(ResourcesAction::OpenSelected))
        .await;
    session
        .apply_effect_result(EffectResult::CloudSqlInstanceDetailLoaded {
            context: context.clone(),
            detail: Box::new(detail),
        })
        .await;
    session
        .apply_effect_result(EffectResult::CloudSqlUsersLoaded {
            context,
            instance_name: "mysql-ha".to_string(),
            users,
        })
        .await;

    let tab_names = [
        "Summary",
        "Connectivity",
        "Maintenance",
        "Backups",
        "Replicas",
        "Users",
        "Json",
    ];
    for (i, tab) in tab_names.iter().enumerate() {
        let lines = render_lines(&session, 120, 30);
        // Redaction guard, proven against the actual rendered buffer.
        let joined = lines.join("\n");
        assert!(
            !joined.contains("SECRETPEM")
                && !joined.contains("ARRAYSECRETPEM")
                && !joined.contains("SUPERSECRET")
                && !joined.contains("BEGIN CERTIFICATE"),
            "secret leaked into rendered {tab} tab:\n{joined}"
        );
        write_evidence(
            &format!("02_detail_tab_{i}_{tab}.txt"),
            &format!(
                "Cloud SQL detail — {tab} tab. Built from a fixture containing a root password and cert PEM; \
                 redact_secrets strips them at the parse boundary so no secret (SUPERSECRET / SECRETPEM / BEGIN CERTIFICATE) \
                 reaches any tab, including Raw JSON and Users (which has no password field).",
            ),
            &lines,
        );
        session
            .dispatch_action(Action::Resources(ResourcesAction::SelectNextTab))
            .await;
    }

    // --- 3. CONFIRMED Stop modal (activation-policy=NEVER) --------------
    let mut stop_session = RuntimeSession::new_for_test(AppState::new());
    load_dev(&mut stop_session).await;
    select_cloud_sql(&mut stop_session, vec![ha_instance()]).await;
    open_palette_and_type(&mut stop_session, "stop selected cloud sql").await;
    stop_session
        .dispatch_action(Action::Command(CommandAction::Submit))
        .await;
    write_evidence(
        "03_confirm_stop_modal.txt",
        "Shared ConfirmResourceAction modal for Stop — shows exact argv \
         'gcloud --configuration dev sql instances patch mysql-ha --activation-policy=NEVER' \
         plus impact warning before execution (Cloud SQL has no stop verb, so Stop = patch activation policy).",
        &render_lines(&stop_session, 120, 22),
    );

    // --- 4. CONFIRMED Failover modal (direct verb) ---------------------
    let mut failover_session = RuntimeSession::new_for_test(AppState::new());
    load_dev(&mut failover_session).await;
    select_cloud_sql(&mut failover_session, vec![ha_instance()]).await;
    open_palette_and_type(&mut failover_session, "fail over selected cloud sql").await;
    failover_session
        .dispatch_action(Action::Command(CommandAction::Submit))
        .await;
    write_evidence(
        "04_confirm_failover_modal.txt",
        "Shared ConfirmResourceAction modal for Failover — direct verb, exact argv \
         'gcloud --configuration dev sql instances failover mysql-ha'. Eligible because the instance is Regional/HA and Runnable.",
        &render_lines(&failover_session, 120, 22),
    );

    // --- 5. Palette gating: Regional offers Failover ------------------
    let mut regional = RuntimeSession::new_for_test(AppState::new());
    load_dev(&mut regional).await;
    select_cloud_sql(&mut regional, vec![ha_instance()]).await;
    open_palette_and_type(&mut regional, "cloud sql").await;
    write_evidence(
        "05_palette_regional_offers_failover.txt",
        "Command palette filtered to 'cloud sql' with a Regional/HA Runnable instance selected: \
         Start / Stop / Restart / Failover all offered.",
        &render_lines(&regional, 120, 24),
    );

    // --- 6. Palette gating: Zonal hides Failover ----------------------
    let mut zonal_instance = ha_instance();
    zonal_instance.name = "mysql-zonal".to_string();
    zonal_instance.availability = CloudSqlAvailability::Zonal;
    let mut zonal = RuntimeSession::new_for_test(AppState::new());
    load_dev(&mut zonal).await;
    select_cloud_sql(&mut zonal, vec![zonal_instance]).await;
    open_palette_and_type(&mut zonal, "cloud sql").await;
    let zonal_lines = render_lines(&zonal, 120, 24);
    assert!(
        !zonal_lines.iter().any(|l| l.contains("Fail over")),
        "Failover must never appear for a Zonal instance:\n{}",
        zonal_lines.join("\n")
    );
    write_evidence(
        "06_palette_zonal_hides_failover.txt",
        "Command palette filtered to 'cloud sql' with a Zonal (non-HA) instance selected: \
         Start / Stop / Restart offered, Failover intentionally ABSENT (unsupported action/state combos never appear).",
        &zonal_lines,
    );
}
