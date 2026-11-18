//! Visual evidence — the same `J` picker, painted by the real
//! `nuvem::ui::render` path, under three different rule sets.
//!
//! `tests/jump_rules_evidence.rs` proves in text where the picker's rows come
//! from. This binary answers the reviewer's other question: what does a user
//! actually see. Each frame below is a full terminal screen, so the rows the
//! config file produced can be read on the surface that renders them, and the
//! frame after `Enter` shows that a user-written rule really lands on the tab
//! it named.
//!
//! Assertions run unconditionally; frames are written only when
//! `NUVEM_EVIDENCE_DIR` is set, and only under that directory.

use std::fs;
use std::path::PathBuf;

use nuvem::app::reducer::root::reduce;
use nuvem::app::{Action, AppState, EffectResult, JumpsAction, ResourcesAction, WorkspaceAction};
use nuvem::services::runtime::session::RuntimeSession;
use nuvem_domain::gcp::configuration::{GcloudConfiguration, GcloudConfigurationName};
use nuvem_domain::gcp::resource::{
    ComputeInstance, EndpointVisibility, GkeCluster, GkeClusterStatus, ResourceServiceKind,
    ResourceStatus,
};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;

use crate::support::frame_svg;
use crate::support::jump_rules;

use frame_svg::buffer_to_svg;
use jump_rules::with_rules;

const WIDTH: u16 = 120;
const HEIGHT: u16 = 30;

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

/// No external IP: the field a rule may name and this row cannot render.
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

/// The cluster the rule below names: `{name}-cluster` for the `web-01` row.
/// It has to be loaded for the jump to land rather than defer, which is the
/// point of frame 3.
fn gke_cluster(name: &str) -> GkeCluster {
    GkeCluster {
        name: name.to_string(),
        location: "europe-west1-b".to_string(),
        status: GkeClusterStatus::Running,
        current_master_version: Some("1.30".to_string()),
        current_node_count: Some(3),
        endpoint_visibility: EndpointVisibility::Public,
    }
}

fn app_on_compute() -> AppState {
    let mut app = AppState::new();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ConfigurationsLoaded {
            configurations: vec![config("dev")],
        }),
    );
    reduce(&mut app, &Action::Workspace(WorkspaceAction::Resources));
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
            context,
            clusters: vec![gke_cluster("web-01-cluster")],
        }),
    );
    reduce(
        &mut app,
        &Action::Resources(ResourcesAction::SelectService(
            ResourceServiceKind::ComputeInstances,
        )),
    );
    app
}

fn draw(app: AppState) -> Buffer {
    let session = RuntimeSession::new_for_test(app);
    let mut terminal = Terminal::new(TestBackend::new(WIDTH, HEIGHT)).expect("terminal init");
    terminal
        .draw(|frame| nuvem::ui::render(frame, &session))
        .expect("render");
    terminal.backend().buffer().clone()
}

fn text(buffer: &Buffer) -> String {
    (0..buffer.area.height)
        .map(|y| {
            (0..buffer.area.width)
                .map(|x| buffer[(x, y)].symbol().to_string())
                .collect::<String>()
                .trim_end()
                .to_string()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn evidence_dir() -> Option<PathBuf> {
    let dir = std::env::var("NUVEM_EVIDENCE_DIR").ok()?;
    let path = PathBuf::from(dir);
    fs::create_dir_all(&path).ok()?;
    Some(path)
}

#[test]
fn the_rendered_picker_shows_the_configured_rules() {
    let mut frames: Vec<(&str, String, String, Buffer)> = Vec::new();

    // ---- 1. No `[[jumps]]` in the file: the two built-in offers, unchanged ----
    let mut app = app_on_compute();
    reduce(&mut app, &Action::Jumps(JumpsAction::Open));
    let default_frame = draw(app);
    let rendered = text(&default_frame);
    assert!(
        rendered.contains("> Network of web-01") && rendered.contains("Disks of web-01"),
        "with no config the picker must still paint the built-in offers:\n{rendered}"
    );
    frames.push((
        "01_picker_built_in_rules.svg",
        "1. No [[jumps]] in the config file".to_string(),
        "The built-in rules, re-expressed as rules — byte-identical to the original picker."
            .to_string(),
        default_frame,
    ));

    // ---- 2. A user-written rule, into another service's tab ---------------
    let configured = "[[jumps]]\n\
                      from = \"compute\"\n\
                      to = \"gke:node-pools\"\n\
                      name = \"{name}-cluster\"\n\
                      location = \"{zone}\"\n\
                      label = \"Node pools of the {name} cluster\"\n";
    let mut app = app_on_compute();
    with_rules(&mut app, configured);
    reduce(&mut app, &Action::Jumps(JumpsAction::Open));
    let configured_frame = draw(app.clone());
    let rendered = text(&configured_frame);
    assert!(
        rendered.contains("> Node pools of the web-01 cluster"),
        "the user's own wording must reach the screen:\n{rendered}"
    );
    assert!(
        !rendered.contains("Network of web-01"),
        "a configured set replaces the built-ins rather than merging:\n{rendered}"
    );
    frames.push((
        "02_picker_user_written_rule.svg",
        "2. One [[jumps]] rule the user wrote".to_string(),
        "to = \"gke:node-pools\", name = \"{name}-cluster\", label = \"Node pools of the {name} cluster\""
            .to_string(),
        configured_frame,
    ));

    // ---- 3. Enter on that rule lands on the tab the rule named ------------
    reduce(&mut app, &Action::Jumps(JumpsAction::Submit));
    let landed_frame = draw(app);
    let rendered = text(&landed_frame);
    assert!(
        rendered.contains("web-01-cluster"),
        "the jump must open the identity the template built:\n{rendered}"
    );
    assert!(
        rendered.contains("Node Pools"),
        "and it must be on the destination tab the rule named:\n{rendered}"
    );
    frames.push((
        "03_after_enter_lands_on_rule_target.svg",
        "3. Enter on that rule".to_string(),
        "The GCE row lands on the GKE cluster \"web-01-cluster\", Node Pools tab — the template built the identity."
            .to_string(),
        landed_frame,
    ));

    // ---- 4. A valid rule with nothing to render for this row --------------
    let unrenderable = "[[jumps]]\n\
                        from = \"compute\"\n\
                        to = \"compute:network\"\n\
                        name = \"{external_ip}\"\n";
    let mut app = app_on_compute();
    with_rules(&mut app, unrenderable);
    reduce(&mut app, &Action::Jumps(JumpsAction::Open));
    let diagnostic_frame = draw(app);
    let rendered = text(&diagnostic_frame);
    assert!(
        rendered.contains("needs external_ip"),
        "the status line must say why there is nothing to offer:\n{rendered}"
    );
    frames.push((
        "04_status_line_says_why_no_offer.svg",
        "4. A rule that cannot render for this row".to_string(),
        "name = \"{external_ip}\" and web-01 has none: no picker opens, and the status line says which field is missing."
            .to_string(),
        diagnostic_frame,
    ));

    let Some(dir) = evidence_dir() else {
        return;
    };
    for (name, title, caption, buffer) in &frames {
        fs::write(dir.join(name), buffer_to_svg(buffer, title, caption)).expect("write svg frame");
    }
}
