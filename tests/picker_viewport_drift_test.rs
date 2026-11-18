//! Both picker overlays route their scrolling window through one helper.
//!
//! `tests/picker_viewport_test.rs` pins what `viewport_window` computes. This
//! file pins the other half, and it is the half the ticket was filed about: that
//! the related-resource jump picker and the configuration context picker still
//! *use* it. The two carried private copies of the same arithmetic before this
//! change, and the failure mode being guarded is not a wrong window — it is one
//! picker quietly growing its own again.
//!
//! So neither test below asserts a fixed window. Each renders the picker, reads
//! the painted rows back out of the terminal buffer, recovers the capacity that
//! renderer actually used from how many rows it painted, and asserts the window
//! on screen is the one `viewport_window` returns for that capacity. A caller
//! that stops calling the helper passes only for as long as its copy agrees;
//! the moment it drifts, the picker it belongs to goes red. Recovering the
//! capacity from the output rather than hard-coding it is deliberate: the
//! private chrome arithmetic each picker uses to size its viewport is free to
//! change, and this file has no opinion about it.

use nuvem::app::reducer::root::reduce;
use nuvem::app::state::configurations::{
    ConfigurationContextPickerItem, ConfigurationContextPickerKind,
};
use nuvem::app::state::root::Workspace;
use nuvem::app::{
    Action, AppState, ConfigurationsAction, EffectResult, JumpsAction, WorkspaceAction,
};
use nuvem::services::runtime::session::RuntimeSession;
use nuvem::ui::shared::picker::viewport_window;
use nuvem_domain::gcp::configuration::{GcloudConfiguration, GcloudConfigurationName};
use nuvem_domain::gcp::project::GcloudProject;
use nuvem_domain::gcp::resource::{ComputeInstance, ResourceStatus};
use ratatui::{Terminal, backend::TestBackend};

/// More rows than any terminal height under test can paint, so every case here
/// is a scrolling one. A fixture that fits on screen would make both tests pass
/// against a renderer with no window arithmetic at all.
const ROWS: usize = 30;

fn render_lines(app: AppState, width: u16, height: u16) -> Vec<String> {
    let session = RuntimeSession::new_for_test(app);
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal
        .draw(|frame| nuvem::ui::render(frame, &session))
        .unwrap();
    let buffer = terminal.backend().buffer();
    (0..buffer.area.height)
        .map(|y| {
            (0..buffer.area.width)
                .map(|x| buffer.cell((x, y)).unwrap().symbol().to_string())
                .collect::<String>()
        })
        .collect()
}

/// The indices of `labels` that appear in the painted frame, in paint order.
///
/// This is what makes the assertion a window assertion rather than a "something
/// was drawn" assertion: the labels are unique per index, so the set that
/// reached the screen names the exact slice the renderer chose.
fn painted_indices(frame: &[String], labels: &[String]) -> Vec<usize> {
    let mut found = Vec::new();
    for line in frame {
        for (index, label) in labels.iter().enumerate() {
            if line.contains(label.as_str()) {
                found.push(index);
            }
        }
    }
    found
}

/// The shared assertion: whatever slice reached the screen must be the one
/// `viewport_window` names for the capacity that slice implies.
///
/// Contiguity is checked first because the capacity recovery depends on it — a
/// renderer painting a scattered set of rows would otherwise yield a
/// meaningless capacity and a vacuously satisfied window.
fn assert_window_matches_helper(painted: &[usize], selected: usize, total: usize, context: &str) {
    assert!(
        !painted.is_empty(),
        "{context}: the picker painted no rows at all"
    );
    assert!(
        painted.windows(2).all(|pair| pair[1] == pair[0] + 1),
        "{context}: painted rows {painted:?} are not a contiguous window"
    );

    let capacity = painted.len();
    let expected = viewport_window(total, selected, capacity);
    let start = painted[0];

    assert_eq!(
        start..start + painted.len(),
        expected,
        "{context}: the picker painted rows {start}..{} but viewport_window says {expected:?} \
         (total={total} selected={selected} capacity={capacity}) — this picker has stopped \
         agreeing with the shared helper",
        start + painted.len()
    );
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
            configurations: vec![GcloudConfiguration {
                name: GcloudConfigurationName::new("dev"),
                is_active: true,
                account: Some("dev@example.com".to_string()),
                project: Some("seed-project".to_string()),
                region: None,
                zone: None,
            }],
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
    assert_eq!(app.root.workspace, Workspace::Resources);
    app.root.jump_rules = rules;
    reduce(&mut app, &Action::Jumps(JumpsAction::Open));
    assert_eq!(
        app.root
            .jump_picker
            .as_ref()
            .expect("picker opens")
            .rows()
            .len(),
        ROWS,
        "the fixture must produce one row per rule"
    );
    app
}

/// Swept across selections and heights rather than checked at one point: a
/// divergent copy of the arithmetic agrees with the helper over part of the
/// range — typically the top of the list, where both clamp to zero — and a
/// single-point test would land there and pass.
#[test]
fn the_jump_picker_window_is_the_shared_helpers_window() {
    let labels: Vec<String> = (0..ROWS).map(jump_label).collect();

    for height in [12u16, 16, 24] {
        let mut app = app_with_jump_rules();
        for selected in 0..ROWS {
            if selected > 0 {
                reduce(&mut app, &Action::Jumps(JumpsAction::SelectNext));
            }
            let frame = render_lines(app.clone(), 80, height);
            let painted = painted_indices(&frame, &labels);
            assert_window_matches_helper(
                &painted,
                selected,
                ROWS,
                &format!("jump picker at height {height}, selection {selected}"),
            );
        }
    }
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
            configurations: vec![GcloudConfiguration {
                name: GcloudConfigurationName::new("dev"),
                is_active: true,
                account: Some("dev@example.com".to_string()),
                project: Some("seed-project".to_string()),
                region: None,
                zone: None,
            }],
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

/// The same sweep against the other picker. No favorites are configured, so
/// `visible_items` keeps the fixture's own order and row *index* means the same
/// thing here as it does in the helper.
#[test]
fn the_context_picker_window_is_the_shared_helpers_window() {
    let labels: Vec<String> = (0..ROWS).map(context_label).collect();

    for height in [16u16, 20, 24] {
        let mut app = app_with_context_picker();
        for selected in 0..ROWS {
            if selected > 0 {
                reduce(
                    &mut app,
                    &Action::Configurations(ConfigurationsAction::SelectNextContextPickerItem),
                );
            }
            let frame = render_lines(app.clone(), 100, height);
            let painted = painted_indices(&frame, &labels);
            assert_window_matches_helper(
                &painted,
                selected,
                ROWS,
                &format!("context picker at height {height}, selection {selected}"),
            );
        }
    }
}
