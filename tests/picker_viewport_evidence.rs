//! Visual evidence — both picker overlays, painted by the real
//! `nuvem::ui::render` path, with their scrolling window now coming from
//! `ui::shared::picker::viewport_window`.
//!
//! `tests/picker_viewport_test.rs` pins what the helper computes and
//! `tests/picker_viewport_drift_test.rs` pins that both pickers still route
//! through it. Neither shows a reviewer what the window looks like. Each frame
//! below is a full terminal screen of a 30-row list too long to fit, at the top
//! of the list, in the middle and at the bottom, so the window can be watched
//! sliding and staying full on the surface a user actually sees.
//!
//! Assertions run unconditionally; frames are written only when
//! `NUVEM_EVIDENCE_DIR` is set, and only under that directory.

use std::fs;
use std::path::PathBuf;

use nuvem::app::reducer::root::reduce;
use nuvem::app::state::configurations::{
    ConfigurationContextPickerItem, ConfigurationContextPickerKind,
};
use nuvem::app::{
    Action, AppState, ConfigurationsAction, EffectResult, JumpsAction, WorkspaceAction,
};
use nuvem::services::runtime::session::RuntimeSession;
use nuvem::ui::shared::picker::viewport_window;
use nuvem_domain::gcp::configuration::{GcloudConfiguration, GcloudConfigurationName};
use nuvem_domain::gcp::project::GcloudProject;
use nuvem_domain::gcp::resource::{ComputeInstance, ResourceStatus};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;

use crate::support::frame_svg;

use frame_svg::buffer_to_svg;

/// Longer than either terminal below can paint, so every frame is a scrolling
/// one.
const ROWS: usize = 30;
const WIDTH: u16 = 120;
const HEIGHT: u16 = 24;

fn dev_configuration() -> GcloudConfiguration {
    GcloudConfiguration {
        name: GcloudConfigurationName::new("dev"),
        is_active: true,
        account: Some("dev@example.com".to_string()),
        project: Some("seed-project".to_string()),
        region: None,
        zone: None,
    }
}

fn draw(app: AppState) -> Buffer {
    let session = RuntimeSession::new_for_test(app);
    let mut terminal = Terminal::new(TestBackend::new(WIDTH, HEIGHT)).expect("terminal init");
    terminal
        .draw(|frame| nuvem::ui::render(frame, &session))
        .expect("render");
    terminal.backend().buffer().clone()
}

fn lines(buffer: &Buffer) -> Vec<String> {
    (0..buffer.area.height)
        .map(|y| {
            (0..buffer.area.width)
                .map(|x| buffer[(x, y)].symbol().to_string())
                .collect::<String>()
        })
        .collect()
}

/// The indices of `labels` that reached the screen, in paint order. The labels
/// are unique per index, so this names the exact slice the renderer chose.
fn painted_indices(buffer: &Buffer, labels: &[String]) -> Vec<usize> {
    let mut found = Vec::new();
    for line in lines(buffer) {
        for (index, label) in labels.iter().enumerate() {
            if line.contains(label.as_str()) {
                found.push(index);
            }
        }
    }
    found
}

/// Asserts the painted slice is the one the shared helper names for the
/// capacity that slice implies, and returns it for the frame's caption.
fn assert_shared_window(buffer: &Buffer, labels: &[String], selected: usize, what: &str) -> String {
    let painted = painted_indices(buffer, labels);
    assert!(!painted.is_empty(), "{what}: no rows painted");
    assert!(
        painted.windows(2).all(|pair| pair[1] == pair[0] + 1),
        "{what}: painted rows {painted:?} are not contiguous"
    );
    let start = painted[0];
    let expected = viewport_window(ROWS, selected, painted.len());
    assert_eq!(
        start..start + painted.len(),
        expected,
        "{what}: painted {start}..{} but viewport_window says {expected:?}",
        start + painted.len()
    );
    format!(
        "rows {}\u{2013}{} of {ROWS} painted, selection {selected} \u{2014} \
         viewport_window({ROWS}, {selected}, {}) = {:?}",
        expected.start,
        expected.end - 1,
        painted.len(),
        expected
    )
}

fn evidence_dir() -> Option<PathBuf> {
    let dir = std::env::var("NUVEM_EVIDENCE_DIR").ok()?;
    let path = PathBuf::from(dir);
    fs::create_dir_all(&path).ok()?;
    Some(path)
}

// ---------------------------------------------------------------------------
// the related-resource jump picker
// ---------------------------------------------------------------------------

fn jump_label(index: usize) -> String {
    format!("Jump rule {index:02} of web-01")
}

fn app_with_jump_rules() -> AppState {
    let toml_source: String = (0..ROWS)
        .map(|index| {
            format!(
                "[[jumps]]\nfrom = \"compute\"\nto = \"compute:disks\"\n\
                 label = \"Jump rule {index:02} of {{name}}\"\n\n"
            )
        })
        .collect();
    // `toml::from_str`, not `parse()`: since toml 1.0 the `FromStr` impl on
    // `Value` reads a single value rather than a document.
    let document: toml::Value = toml::from_str(&toml_source).expect("fixture toml parses");
    let mut warnings = Vec::new();
    let rules = nuvem::config::jumps::JumpRules::from_file(
        document.get("jumps").expect("fixture has [[jumps]]"),
        &mut warnings,
    );
    assert!(warnings.is_empty(), "fixture rules are valid: {warnings:?}");

    let mut app = AppState::new();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ConfigurationsLoaded {
            configurations: vec![dev_configuration()],
        }),
    );
    reduce(&mut app, &Action::Workspace(WorkspaceAction::Resources));
    let context = app.root.current_context().expect("context exists");
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ComputeInstancesLoaded {
            context,
            instances: vec![ComputeInstance {
                id: "web-01-id".to_string(),
                name: "web-01".to_string(),
                zone: "europe-west1-b".to_string(),
                status: ResourceStatus::Running,
                machine_type: "e2-medium".to_string(),
                internal_ip: Some("10.0.0.2".to_string()),
                external_ip: None,
            }],
        }),
    );
    app.root.jump_rules = rules;
    reduce(&mut app, &Action::Jumps(JumpsAction::Open));
    app
}

// ---------------------------------------------------------------------------
// the configuration context picker
// ---------------------------------------------------------------------------

fn context_label(index: usize) -> String {
    format!("ctx-project-{index:02}")
}

fn app_with_context_picker() -> AppState {
    let mut app = AppState::new();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ConfigurationsLoaded {
            configurations: vec![dev_configuration()],
        }),
    );
    reduce(
        &mut app,
        &Action::Configurations(ConfigurationsAction::StartEditActiveContext),
    );
    reduce(
        &mut app,
        &Action::Configurations(ConfigurationsAction::OpenContextPicker),
    );
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ContextPickerLoaded {
            configuration: GcloudConfigurationName::new("dev"),
            kind: ConfigurationContextPickerKind::Project,
            items: (0..ROWS)
                .map(|index| {
                    ConfigurationContextPickerItem::Project(GcloudProject {
                        project_id: context_label(index),
                        name: context_label(index),
                        lifecycle_state: Some("ACTIVE".to_string()),
                    })
                })
                .collect(),
        }),
    );
    app
}

#[test]
fn both_pickers_paint_the_shared_helpers_window() {
    let stops = [0usize, 15, ROWS - 1];
    let mut frames: Vec<(String, String, String, Buffer)> = Vec::new();

    let jump_labels: Vec<String> = (0..ROWS).map(jump_label).collect();
    let mut app = app_with_jump_rules();
    let mut at = 0usize;
    for (step, selected) in stops.iter().copied().enumerate() {
        while at < selected {
            reduce(&mut app, &Action::Jumps(JumpsAction::SelectNext));
            at += 1;
        }
        let buffer = draw(app.clone());
        let caption = assert_shared_window(
            &buffer,
            &jump_labels,
            selected,
            &format!("jump picker, selection {selected}"),
        );
        frames.push((
            format!("{}_jump_picker_selection_{selected:02}.svg", step + 1),
            format!("Jump picker (J) \u{2014} {ROWS} rules, selection {selected}"),
            caption,
            buffer,
        ));
    }

    let context_labels: Vec<String> = (0..ROWS).map(context_label).collect();
    let mut app = app_with_context_picker();
    let mut at = 0usize;
    for (step, selected) in stops.iter().copied().enumerate() {
        while at < selected {
            reduce(
                &mut app,
                &Action::Configurations(ConfigurationsAction::SelectNextContextPickerItem),
            );
            at += 1;
        }
        let buffer = draw(app.clone());
        let caption = assert_shared_window(
            &buffer,
            &context_labels,
            selected,
            &format!("context picker, selection {selected}"),
        );
        frames.push((
            format!("{}_context_picker_selection_{selected:02}.svg", step + 4),
            format!("Configuration context picker \u{2014} {ROWS} projects, selection {selected}"),
            caption,
            buffer,
        ));
    }

    let Some(dir) = evidence_dir() else {
        return;
    };
    for (name, title, caption, buffer) in &frames {
        fs::write(dir.join(name), buffer_to_svg(buffer, title, caption)).expect("write svg frame");
    }
}
