//! Evidence: renders the actual Resources TUI a user sees while
//! hopping between gcloud configurations, proving that switching *back* to a
//! previously-visited configuration restores its resource rows as
//! stale-but-visible instead of dropping to an empty list that forces a full
//! re-fetch. Drives the real reducer (the same `ConfigurationActivated`
//! effect the app emits) and rasterizes each frame with ratatui's
//! `TestBackend`. Writes the transcript to NUVEM_EVIDENCE_DIR when set.

use nuvem::app::reducer::root::reduce;
use nuvem::app::{Action, AppState, EffectResult, Workspace};
use nuvem::services::runtime::session::RuntimeSession;
use nuvem_domain::gcp::configuration::{GcloudConfiguration, GcloudConfigurationName};
use nuvem_domain::gcp::resource::{ComputeInstance, ResourceStatus};
use ratatui::{Terminal, backend::TestBackend};

fn config(name: &str, active: bool, project: &str) -> GcloudConfiguration {
    GcloudConfiguration {
        name: GcloudConfigurationName::new(name),
        is_active: active,
        account: None,
        project: Some(project.to_string()),
        region: None,
        zone: None,
    }
}

fn instance(name: &str) -> ComputeInstance {
    ComputeInstance {
        id: name.to_string(),
        name: name.to_string(),
        zone: "us-central1-a".to_string(),
        status: ResourceStatus::Running,
        machine_type: "e2-medium".to_string(),
        internal_ip: Some("10.0.0.4".to_string()),
        external_ip: None,
    }
}

/// Renders the whole app frame and returns the visible text grid, one row per
/// line, exactly as painted to the terminal.
fn render_frame(app: &AppState, width: u16, height: u16) -> String {
    let session = RuntimeSession::new_for_test(app.clone());
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).expect("terminal should initialize");
    terminal
        .draw(|frame| nuvem::ui::render(frame, &session))
        .expect("frame should render");
    terminal
        .backend()
        .buffer()
        .content()
        .chunks(usize::from(width))
        .map(|row| {
            row.iter()
                .map(|cell| cell.symbol())
                .collect::<String>()
                .trim_end()
                .to_string()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn switching_back_to_a_configuration_shows_cached_rows_in_the_rendered_tui() {
    let width = 100u16;
    let height = 16u16;

    let mut app = AppState::new();
    app.root.workspace = Workspace::Resources;

    // Two configurations available; "dev" is active on entry.
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ConfigurationsLoaded {
            configurations: vec![
                config("dev", true, "dev-project"),
                config("prod", false, "prod-project"),
            ],
        }),
    );

    // dev's GCE instances finish loading -> rows are visible & fresh.
    let dev_context = app.root.current_context().unwrap().clone();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ComputeInstancesLoaded {
            context: dev_context,
            instances: vec![instance("api-dev-1"), instance("api-dev-2")],
        }),
    );
    let frame_dev_loaded = render_frame(&app, width, height);
    assert!(frame_dev_loaded.contains("api-dev-1"));
    assert!(frame_dev_loaded.contains("api-dev-2"));

    // User activates "prod": this used to clear everything; the list
    // is now empty for the freshly-entered context.
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ConfigurationActivated {
            configuration: config("prod", true, "prod-project"),
        }),
    );
    let frame_prod = render_frame(&app, width, height);
    assert!(app.root.resources.instances().is_empty());
    assert!(!frame_prod.contains("api-dev-1"));

    // User switches back to "dev": the cached rows are restored and rendered
    // immediately (stale-but-visible) rather than showing an empty/loading
    // list, and the freshness system flags them stale for background refresh.
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ConfigurationActivated {
            configuration: config("dev", true, "dev-project"),
        }),
    );
    let frame_dev_restored = render_frame(&app, width, height);

    assert_eq!(app.root.resources.instances().len(), 2);
    assert!(app.root.resources.is_stale());
    assert!(frame_dev_restored.contains("api-dev-1"));
    assert!(frame_dev_restored.contains("api-dev-2"));

    let report = format!(
        "Retain per-context resource caches across configuration switches\n\
         ================================================================================\n\n\
         STEP 1 — Configuration \"dev\" active, GCE instances loaded (fresh):\n\
         --------------------------------------------------------------------------------\n{frame_dev_loaded}\n\n\
         STEP 2 — Switched to configuration \"prod\": list is empty for the new context:\n\
         --------------------------------------------------------------------------------\n{frame_prod}\n\n\
         STEP 3 — Switched BACK to \"dev\": cached rows restored, visible & marked stale\n\
         (freshness system will refresh in the background — no empty list, no forced wait):\n\
         --------------------------------------------------------------------------------\n{frame_dev_restored}\n"
    );

    println!("\n{report}");

    if let Ok(dir) = std::env::var("NUVEM_EVIDENCE_DIR") {
        let path = std::path::Path::new(&dir).join("context_cache_switchback_render.txt");
        std::fs::write(&path, &report).expect("write evidence file");
    }
}
