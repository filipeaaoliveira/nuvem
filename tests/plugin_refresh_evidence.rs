//! Evidence: a plugin run that finishes reloads the resource table, so
//! the table and the operations log stop disagreeing about what happened.
//!
//! The reload is deliberately an in-place `ResourcesAction::Refresh` rather
//! than the lifecycle arms' `mark_active_stale_and_clear`: a plugin's argv is
//! runtime data, so this path cannot tell `describe` from `delete`, and paying
//! for that uncertainty with the operator's marks, detail pane and cursor
//! position is the wrong trade. `marks_and_detail_and_cursor_survive_the_reload`
//! below is what holds that line.

use nuvem::app::reducer::root::reduce;
use nuvem::app::state::resources::StorageView;
use nuvem::app::state::root::Workspace;
use nuvem::app::{Action, AppError, AppState, Effect, EffectResult, ResourcesAction};
use nuvem_domain::gcp::configuration::{
    GcloudConfiguration, GcloudConfigurationName, GcloudContext,
};
use nuvem_domain::gcp::resource::{
    ComputeInstance, ResourceServiceKind, ResourceStatus, StorageBucket, StorageObject,
    StorageObjectPage, StoragePrefix,
};

/// The operation id these tests hand to a result. Nothing here asserts on the
/// operations log — that is the execution evidence — so any id that is not pending
/// settles nothing and leaves the log out of the way.
const UNTRACKED: u64 = 9_001;

/// Copied from `tests/plugin_execution_evidence.rs:659`. Note
/// `GcloudConfigurationName::new` returns the name directly, not a `Result`.
fn configuration_fixture() -> GcloudConfiguration {
    GcloudConfiguration {
        name: GcloudConfigurationName::new("dev"),
        is_active: true,
        account: None,
        project: Some("dev-project".to_string()),
        region: Some("europe-west1".to_string()),
        zone: Some("europe-west1-b".to_string()),
    }
}

/// An app whose active context resolves and whose operator is looking at the
/// Resources workspace — the state a plugin result normally lands in.
///
/// The workspace is set explicitly because `RootState` defaults to
/// `Workspace::Home` (`src/app/state/root.rs:121`), which is also what makes
/// the negative test below a real one rather than an accident of the fixture.
pub(crate) fn app_at_a_loaded_context() -> (AppState, GcloudContext) {
    let mut app = AppState::new();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ConfigurationsLoaded {
            configurations: vec![configuration_fixture()],
        }),
    );
    let context = app
        .root
        .current_context()
        .expect("the loaded configuration is the active context")
        .clone();
    app.root.workspace = Workspace::Resources;
    (app, context)
}

fn succeeded(context: GcloudContext) -> Action {
    Action::EffectResult(EffectResult::PluginRunSucceeded {
        context,
        label: "Describe web 01".to_string(),
        output: "status: RUNNING".to_string(),
        stderr: String::new(),
        operation_id: UNTRACKED,
    })
}

fn failed(context: GcloudContext) -> Action {
    Action::EffectResult(EffectResult::PluginRunFailed {
        context,
        label: "Describe web 01".to_string(),
        // The convenience constructor (`src/app/effect.rs:781`): no known
        // origin or exit code, classified from the text. Nothing here asserts
        // on the diagnostic, so the cheapest well-formed failure is the right
        // one.
        error: AppError::gcloud("permission denied"),
        stderr: String::new(),
        operation_id: UNTRACKED,
    })
}

fn cancelled(context: GcloudContext) -> Action {
    Action::EffectResult(EffectResult::PluginRunCancelled {
        context,
        label: "Describe web 01".to_string(),
        operation_id: UNTRACKED,
    })
}

/// Every assertion in this file that wants "the active service was reloaded"
/// goes through this, so no test can be satisfied by an unrelated effect.
///
/// The `context` clause is a shape check, not evidence of context routing: the
/// delegated refresh builds its effect from `state.current_context()`, which the
/// guard has already established equals the result's context, so the two are the
/// same value by construction and this clause cannot fail today. It stays as a
/// tripwire for a future hand-built effect that would route elsewhere. Routing
/// itself is pinned by `results_for_a_context_the_operator_has_left_reload_nothing`.
fn assert_reloads_compute(effects: &[Effect], expected_context: &GcloudContext) {
    let refreshed = effects.iter().any(|effect| {
        matches!(
            effect,
            Effect::RefreshComputeInstances { context, .. } if context == expected_context
        )
    });
    assert!(
        refreshed,
        "the active service must be reloaded, got: {effects:?}"
    );
}

#[test]
fn a_succeeded_plugin_reloads_the_active_service() {
    let (mut app, context) = app_at_a_loaded_context();

    let effects = reduce(&mut app, &succeeded(context.clone()));

    assert_reloads_compute(&effects, &context);
}

#[test]
fn a_failed_plugin_reloads_the_active_service() {
    // A plugin that errored partway is exactly when the table is most likely to
    // be wrong: a `delete` that failed after removing two of five rows leaves
    // the log saying "failed" and the table showing all five.
    let (mut app, context) = app_at_a_loaded_context();

    let effects = reduce(&mut app, &failed(context.clone()));

    assert_reloads_compute(&effects, &context);
}

#[test]
fn a_cancelled_plugin_reloads_the_active_service() {
    // `Ctrl-x` kills a run mid-flight, so a cancelled plugin is the outcome
    // most likely to have half-applied a change.
    let (mut app, context) = app_at_a_loaded_context();

    let effects = reduce(&mut app, &cancelled(context.clone()));

    assert_reloads_compute(&effects, &context);
}

/// The three outcome constructors, each labelled, so a negative test covers all
/// three arms of the guard rather than only `succeeded`. The arm is written as
/// one three-variant pattern today; the refactor worth guarding against is a
/// split into three arms that drops a condition from one of them.
type LabelledOutcome = (&'static str, fn(GcloudContext) -> Action);

const OUTCOMES: [LabelledOutcome; 3] = [
    ("succeeded", succeeded),
    ("failed", failed),
    ("cancelled", cancelled),
];

#[test]
fn results_for_a_context_the_operator_has_left_reload_nothing() {
    // The run is on a blocking thread and outlives the context it was confirmed
    // under, so this is the normal case, not an edge one. Reloading here would
    // fetch one context's rows into another context's table.
    let (mut app, _context) = app_at_a_loaded_context();
    // `project` is `Option<String>` on `GcloudContext`
    // (`crates/nuvem-domain/src/gcp/configuration.rs:36`), so the differing
    // field is a `Some`, not a bare string.
    let elsewhere = GcloudContext {
        project: Some("some-other-project".to_string()),
        ..app
            .root
            .current_context()
            .expect("the fixture resolves a context")
            .clone()
    };

    for (label, outcome) in OUTCOMES {
        let effects = reduce(&mut app, &outcome(elsewhere.clone()));

        assert!(
            effects.is_empty(),
            "a {label} result from a context that is no longer current must reload nothing, got: {effects:?}"
        );
    }
}

#[test]
fn results_that_land_outside_the_resources_workspace_reload_nothing() {
    // Running a plugin from the palette while reading logs is ordinary. Issuing
    // a gcloud list for a table nobody is looking at buys nothing, which is the
    // same reason `poll_active_resource` returns early
    // (`src/services/runtime/session.rs:3748`).
    let (mut app, context) = app_at_a_loaded_context();
    app.root.workspace = Workspace::Logging;

    for (label, outcome) in OUTCOMES {
        let effects = reduce(&mut app, &outcome(context.clone()));

        assert!(
            effects.is_empty(),
            "no reload is due for a workspace the operator is not in, got a {label} result's {effects:?}"
        );
    }
}

/// Copied from `tests/runtime_compute_action_effect_test.rs:350`, the repo's
/// existing compute-row fixture.
fn instance(name: &str) -> ComputeInstance {
    ComputeInstance {
        id: format!("{name}-id"),
        name: name.to_string(),
        zone: "europe-west1-b".to_string(),
        status: ResourceStatus::Running,
        machine_type: "e2-medium".to_string(),
        internal_ip: None,
        external_ip: None,
    }
}

/// Loads two rows, marks one, opens the detail pane, and moves the cursor off
/// row zero — every piece of operator state `mark_active_stale_and_clear`
/// (`src/app/state/resources/mod.rs:3105`) would destroy.
fn app_with_operator_state() -> (AppState, GcloudContext) {
    let (mut app, context) = app_at_a_loaded_context();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ComputeInstancesLoaded {
            context: context.clone(),
            instances: vec![instance("api-dev-1"), instance("api-dev-2")],
        }),
    );
    reduce(&mut app, &Action::Resources(ResourcesAction::SelectNext));
    reduce(&mut app, &Action::Resources(ResourcesAction::ToggleMark));
    reduce(&mut app, &Action::Resources(ResourcesAction::OpenSelected));
    assert_eq!(
        app.root.resources.marked_count(),
        1,
        "fixture precondition: a row is marked before the plugin result lands"
    );
    assert!(
        app.root.resources.is_showing_detail(),
        "fixture precondition: the detail pane is open before the plugin result lands"
    );
    // Without this the cursor assertion downstream is vacuous: if `SelectNext`
    // ever stops moving on a two-row table, or these rows fail to land, the
    // cursor is already at row zero and `selected == cursor_before` degrades to
    // `0 == 0`, passing against the very reset-to-row-zero defect it exists to
    // catch.
    assert_eq!(
        app.root.resources.selected_visible_index(),
        Some(1),
        "fixture precondition: the cursor is off row zero before the plugin result lands"
    );
    (app, context)
}

#[test]
fn marks_and_detail_and_cursor_survive_the_reload() {
    // The whole reason this arm delegates to `ResourcesAction::Refresh` rather
    // than copying the lifecycle arms' `mark_active_stale_and_clear`. Asserting
    // the reload happened as well as what survived is what stops this test
    // passing vacuously if the reload is ever removed.
    let (mut app, context) = app_with_operator_state();
    let cursor_before = app.root.resources.selected_visible_index();

    let effects = reduce(&mut app, &succeeded(context.clone()));

    assert_reloads_compute(&effects, &context);
    assert_eq!(
        app.root.resources.marked_count(),
        1,
        "a reload must not drop the operator's marks"
    );
    assert!(
        app.root.resources.is_showing_detail(),
        "a reload must not close the detail pane the operator is reading"
    );
    assert_eq!(
        app.root.resources.selected_visible_index(),
        cursor_before,
        "a reload must not move the cursor back to row zero"
    );
    assert_eq!(
        app.root.resources.instances().len(),
        2,
        "the table must not blank while the reload is in flight"
    );
}

#[test]
fn the_reload_carries_the_active_services_own_filter() {
    // A reload *replaces* the stored rows, so an unfiltered one turns the whole
    // project into the service's row set while the tools line still asserts the
    // filter is in force. Delegating to `ResourcesAction::Refresh` is what buys
    // this; a hand-built effect is where the `None` would creep back in.
    let (mut app, context) = app_at_a_loaded_context();
    reduce(&mut app, &Action::Resources(ResourcesAction::StartFilter));
    for character in "api".chars() {
        reduce(
            &mut app,
            &Action::Resources(ResourcesAction::AppendFilterChar(character)),
        );
    }
    reduce(&mut app, &Action::Resources(ResourcesAction::ApplyFilter));

    let effects = reduce(&mut app, &succeeded(context.clone()));

    let Some(Effect::RefreshComputeInstances { filter, .. }) = effects
        .iter()
        .find(|effect| matches!(effect, Effect::RefreshComputeInstances { .. }))
    else {
        panic!("the reload must emit a compute refresh, got: {effects:?}");
    };
    assert_eq!(
        filter.as_deref(),
        Some("api"),
        "the reload must carry the applied filter, not None"
    );
}

#[test]
fn a_service_other_than_the_default_is_reloaded_too() {
    // Compute is the default active service, so every test above would pass on
    // an implementation hard-coded to it. This is the one that would not.
    let (mut app, context) = app_at_a_loaded_context();
    reduce(
        &mut app,
        &Action::Resources(ResourcesAction::SelectService(
            ResourceServiceKind::SecretManagerSecrets,
        )),
    );

    let effects = reduce(&mut app, &succeeded(context.clone()));

    let refreshed = effects.iter().any(|effect| {
        matches!(
            effect,
            Effect::RefreshSecrets { context: c, .. } if *c == context
        )
    });
    assert!(
        refreshed,
        "the reload must follow the active service, not a hard-coded one, got: {effects:?}"
    );
    assert!(
        !effects
            .iter()
            .any(|effect| matches!(effect, Effect::RefreshComputeInstances { .. })),
        "reloading compute while secrets is active would fetch the wrong table: {effects:?}"
    );
}

/// Copied from `tests/reducer_resources_test.rs:3524`, the repo's existing
/// storage fixtures, so the browser is set up the way the rest of the suite
/// sets it up rather than by poking state directly.
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

fn storage_prefix(name: &str) -> StoragePrefix {
    StoragePrefix {
        name: name.to_string(),
    }
}

fn storage_object(key: &str) -> StorageObject {
    StorageObject {
        key: key.to_string(),
        size_bytes: Some(12),
        storage_class: Some("STANDARD".to_string()),
        generation: None,
        updated: Some("2026-01-01T00:00:00Z".to_string()),
        content_type: Some("text/plain".to_string()),
        kms_encrypted: false,
        temporary_hold: false,
        event_based_hold: false,
        has_retention: false,
    }
}

/// Opens a bucket's object browser and pages forward once, so the accumulated
/// rows and the cursor are both things a wholesale page replacement would
/// destroy.
fn app_browsing_a_paged_bucket() -> (AppState, GcloudContext) {
    let (mut app, context) = app_at_a_loaded_context();
    reduce(
        &mut app,
        &Action::Resources(ResourcesAction::SelectService(
            ResourceServiceKind::CloudStorage,
        )),
    );
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::StorageBucketsLoaded {
            context: context.clone(),
            buckets: vec![storage_bucket("assets-bucket")],
        }),
    );
    reduce(&mut app, &Action::Resources(ResourcesAction::OpenSelected));
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::StorageObjectPageLoaded {
            context: context.clone(),
            bucket: "assets-bucket".to_string(),
            prefix: String::new(),
            append: false,
            page: StorageObjectPage {
                prefixes: vec![storage_prefix("images/")],
                objects: vec![storage_object("a.txt")],
                next_page_token: Some("TOKEN2".to_string()),
            },
        }),
    );
    reduce(&mut app, &Action::Resources(ResourcesAction::NextPage));
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::StorageObjectPageLoaded {
            context: context.clone(),
            bucket: "assets-bucket".to_string(),
            prefix: String::new(),
            append: true,
            page: StorageObjectPage {
                prefixes: Vec::new(),
                objects: vec![storage_object("b.txt"), storage_object("c.txt")],
                next_page_token: None,
            },
        }),
    );
    assert_eq!(
        app.root.resources.storage().view(),
        StorageView::ObjectBrowser,
        "fixture precondition: the object browser is open when the plugin result lands"
    );
    assert_eq!(
        app.root.resources.storage().visible_browser_rows().len(),
        4,
        "fixture precondition: two pages have accumulated before the plugin result lands"
    );
    (app, context)
}

#[test]
fn a_plugin_result_does_not_collapse_a_paged_object_browse() {
    // Cloud Storage is the one service whose `ResourcesAction::Refresh` carries
    // state-destroying work: it re-requests the object page with
    // `page_token: None`, which `set_object_page` treats as a replacement, so
    // the accumulated pages, the cursor and the page index all go back to the
    // first page. That is the right price for a deliberate `r` and the wrong
    // one for a reload the operator did not ask for — the same in-place promise
    // `marks_and_detail_and_cursor_survive_the_reload` holds for every other
    // service. The bucket list still refreshes: that costs nothing.
    let (mut app, context) = app_browsing_a_paged_bucket();

    let effects = reduce(&mut app, &succeeded(context.clone()));

    assert!(
        effects.iter().any(|effect| matches!(
            effect,
            Effect::RefreshStorageBuckets { context: c } if *c == context
        )),
        "the bucket list must still be reloaded, got: {effects:?}"
    );
    assert!(
        !effects
            .iter()
            .any(|effect| matches!(effect, Effect::LoadStorageObjectPage { .. })),
        "an involuntary reload must not re-request the first object page, got: {effects:?}"
    );
    assert_eq!(
        app.root.resources.storage().visible_browser_rows().len(),
        4,
        "the accumulated pages must survive a reload nobody pressed a key for"
    );
}

// ---------------------------------------------------------------------------
// End-to-end evidence: the surface the operator actually reads.
//
// Everything above works through the reducer's public seam and asserts on
// emitted effects. That answers "is the reload emitted"; it does not answer
// "does the table on screen stop disagreeing with the log", which is the whole
// of the reload as the operator experiences it. This section removes every double
// in the chain: a real child process runs the plugin's argv, the reload that
// follows runs a real `compute instances list` against the same child, and the
// two frames written out are full `nuvem::ui::render` frames.
//
// The fixture `gcloud` is a shell script whose inventory really changes when
// the plugin's `delete` runs, so a table still showing the deleted row after
// the run is the reported defect reproduced rather than a fixture artefact.
// ---------------------------------------------------------------------------

use std::os::unix::fs::PermissionsExt;
use std::time::Duration;

use nuvem::app::WorkspaceAction;
use nuvem::app::state::operations::OperationOutcome;
use nuvem::services::gcloud::cli::ProcessCommandRunner;
use nuvem::services::runtime::session::RuntimeSession;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;

use crate::support::frame_svg::buffer_to_svg;

const SCREEN: (u16, u16) = (120, 34);

/// The two rows the fixture project starts with, in the shape `gcloud compute
/// instances list --format=json` really emits (zone and machineType as
/// self-links), so the production parser is exercised rather than bypassed.
const INVENTORY_BEFORE: &str = r#"[
  {
    "id": "1001",
    "name": "api-dev-1",
    "zone": "https://www.googleapis.com/compute/v1/projects/dev-project/zones/europe-west1-b",
    "status": "RUNNING",
    "machineType": "https://www.googleapis.com/compute/v1/projects/dev-project/zones/europe-west1-b/machineTypes/e2-medium",
    "networkInterfaces": [{ "networkIP": "10.0.0.4" }]
  },
  {
    "id": "1002",
    "name": "web-01",
    "zone": "https://www.googleapis.com/compute/v1/projects/dev-project/zones/europe-west1-b",
    "status": "RUNNING",
    "machineType": "https://www.googleapis.com/compute/v1/projects/dev-project/zones/europe-west1-b/machineTypes/e2-medium",
    "networkInterfaces": [{ "networkIP": "10.0.0.5" }]
  }
]
"#;

/// The same project after the plugin's `delete` has really taken `web-01` away.
const INVENTORY_AFTER: &str = r#"[
  {
    "id": "1001",
    "name": "api-dev-1",
    "zone": "https://www.googleapis.com/compute/v1/projects/dev-project/zones/europe-west1-b",
    "status": "RUNNING",
    "machineType": "https://www.googleapis.com/compute/v1/projects/dev-project/zones/europe-west1-b/machineTypes/e2-medium",
    "networkInterfaces": [{ "networkIP": "10.0.0.4" }]
  }
]
"#;

/// A stand-in `gcloud` with state: `list` answers from whichever inventory the
/// marker file selects, and `delete` removes the marker, so the plugin's run
/// really changes what a later `list` returns.
fn fixture_gcloud(dir: &std::path::Path) -> std::path::PathBuf {
    let script = dir.join("gcloud");
    std::fs::write(dir.join("before.json"), INVENTORY_BEFORE).expect("write the before inventory");
    std::fs::write(dir.join("after.json"), INVENTORY_AFTER).expect("write the after inventory");
    std::fs::write(dir.join("web-01.present"), "").expect("seed the marker");
    let dir = dir.display();
    std::fs::write(
        &script,
        format!(
            "#!/bin/sh\n\
             verb=\"\"\n\
             for arg in \"$@\"; do\n\
               case \"$arg\" in list) verb=list ;; delete) verb=delete ;; esac\n\
             done\n\
             printf '%s\\n' \"$*\" >> {dir}/argv.log\n\
             if [ \"$verb\" = delete ]; then rm -f {dir}/web-01.present; printf 'Deleted [web-01].\\n'; exit 0; fi\n\
             if [ \"$verb\" = list ]; then\n\
               if [ -f {dir}/web-01.present ]; then cat {dir}/before.json; else cat {dir}/after.json; fi\n\
               exit 0\n\
             fi\n\
             printf '[]\\n'\n"
        ),
    )
    .expect("write the fixture gcloud");
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755))
        .expect("the fixture gcloud is executable");
    script
}

fn render(session: &RuntimeSession) -> Buffer {
    let mut terminal =
        Terminal::new(TestBackend::new(SCREEN.0, SCREEN.1)).expect("terminal should initialize");
    terminal
        .draw(|frame| nuvem::ui::render(frame, session))
        .expect("render should succeed");
    terminal.backend().buffer().clone()
}

fn buffer_lines(buffer: &Buffer) -> Vec<String> {
    (0..buffer.area.height)
        .map(|y| {
            (0..buffer.area.width)
                .map(|x| buffer[(x, y)].symbol().to_string())
                .collect::<String>()
                .trim_end()
                .to_string()
        })
        .collect()
}

/// Written only under `NUVEM_EVIDENCE_DIR`, never to a fixed machine-global path.
/// The assertions below run either way.
fn write_frame(name: &str, title: &str, caption: &str, buffer: &Buffer) {
    let Ok(dir) = std::env::var("NUVEM_EVIDENCE_DIR") else {
        eprintln!("NUVEM_EVIDENCE_DIR not set; skipping evidence render");
        return;
    };
    let dir = std::path::PathBuf::from(dir);
    std::fs::create_dir_all(&dir).expect("the evidence directory");
    std::fs::write(
        dir.join(format!("{name}.svg")),
        buffer_to_svg(buffer, title, caption),
    )
    .expect("write svg frame");
    std::fs::write(
        dir.join(format!("{name}.txt")),
        format!(
            "{title}\n{caption}\n\n{}\n",
            buffer_lines(buffer).join("\n")
        ),
    )
    .expect("write ascii frame");
}

/// Drains queued effect results until `done` holds, with a bounded number of
/// results and a deadline on each: a run that never reaches the state under
/// test must fail here rather than park the test forever.
async fn drain_until(
    session: &mut RuntimeSession,
    label: &str,
    mut done: impl FnMut(&RuntimeSession) -> bool,
) {
    for _ in 0..12 {
        if done(session) {
            return;
        }
        let processed = tokio::time::timeout(
            Duration::from_secs(30),
            session.process_next_effect_result(),
        )
        .await
        .unwrap_or_else(|_| panic!("timed out waiting for {label}"));
        assert!(processed, "the result channel closed waiting for {label}");
    }
    assert!(done(session), "{label} never happened");
}

/// The label, argv and `dangerous` flag of the delete the operator confirms —
/// the three things the confirmation is opened from.
fn delete_confirmation() -> (String, Vec<String>, bool) {
    (
        "Delete web-01".to_string(),
        vec![
            "--configuration".to_string(),
            "dev".to_string(),
            "compute".to_string(),
            "instances".to_string(),
            "delete".to_string(),
            "web-01".to_string(),
            "--zone".to_string(),
            "europe-west1-b".to_string(),
            "--quiet".to_string(),
        ],
        true,
    )
}

fn instance_names(app: &AppState) -> Vec<String> {
    app.root
        .resources
        .instances()
        .iter()
        .map(|instance| instance.name.clone())
        .collect()
}

/// The defect reproduced and fixed on the operator's own screen: a plugin that
/// really deletes a row leaves the table showing the row it deleted, while the
/// operations log says the run finished. After this change the same run leaves
/// the table agreeing with the log.
#[tokio::test]
async fn a_mutating_plugin_leaves_the_table_agreeing_with_the_operations_log() {
    let dir = tempfile::tempdir().expect("a temp dir for the fixture gcloud");
    let gcloud = fixture_gcloud(dir.path());

    let mut session = RuntimeSession::new_with_runner_for_test(
        AppState::new(),
        Box::new(ProcessCommandRunner::new(gcloud)),
    );
    session
        .apply_effect_result(EffectResult::ConfigurationsLoaded {
            configurations: vec![configuration_fixture()],
        })
        .await;
    session
        .dispatch_action(Action::Workspace(WorkspaceAction::Resources))
        .await;
    // The first table the operator sees is the real child's `list` output, not
    // a hand-injected `ComputeInstancesLoaded`.
    session
        .dispatch_action(Action::Resources(ResourcesAction::Refresh))
        .await;
    drain_until(&mut session, "the first compute list", |session| {
        instance_names(session.app()).len() == 2
    })
    .await;

    let before = render(&session);
    write_frame(
        "01-before-the-run",
        "1. The GCE list before the plugin runs",
        "Two instances, `web-01` among them, listed by a real `gcloud compute instances list` \
         child. This is the state the reported defect left on screen after the delete.",
        &before,
    );
    assert!(
        buffer_lines(&before)
            .iter()
            .any(|line| line.contains("web-01")),
        "the table must show the row the plugin is about to delete: {:#?}",
        buffer_lines(&before)
    );

    // The operator confirms the plugin, and Enter runs it — a real child that
    // really removes `web-01` from the fixture project.
    let (label, argv, dangerous) = delete_confirmation();
    session
        .app_mut()
        .root
        .resources
        .show_plugin_confirmation(label, argv, dangerous);
    session
        .dispatch_action(Action::Resources(ResourcesAction::ConfirmPlugin))
        .await;
    drain_until(&mut session, "the plugin run to settle", |session| {
        session.app().root.system.operations.pending_count() == 0
    })
    .await;

    let record = session
        .app()
        .root
        .system
        .operations
        .records()
        .iter()
        .find(|record| record.kind == "plugin")
        .expect("the confirmation seeded an operations-log entry")
        .clone();
    assert_eq!(
        record.outcome,
        OperationOutcome::Succeeded,
        "the log must say the run finished, detail: {:?}",
        record.detail
    );

    // The reload the log's "finished" now implies.
    drain_until(&mut session, "the reload after the run", |session| {
        instance_names(session.app()) == vec!["api-dev-1".to_string()]
    })
    .await;

    let after = render(&session);
    write_frame(
        "02-after-the-run",
        "2. The same list the moment the plugin run finishes",
        "No keypress in between: the finished run reloaded the active service, so `web-01` is \
         gone from the table exactly as it is gone from the project. Before this change the \
         table still showed it until the next poll or an `r` press.",
        &after,
    );
    let after_lines = buffer_lines(&after);
    assert!(
        !after_lines.iter().any(|line| line.contains("web-01")),
        "the deleted row must be gone from the table with no keypress: {after_lines:#?}"
    );
    assert!(
        after_lines.iter().any(|line| line.contains("api-dev-1")),
        "the surviving row must still be listed: {after_lines:#?}"
    );

    // The log the table now agrees with, on the surface that renders it.
    session
        .dispatch_action(Action::Workspace(WorkspaceAction::Home))
        .await;
    let log = render(&session);
    write_frame(
        "03-the-log-that-agrees",
        "3. Recent operations, same session",
        "The entry the table was contradicting: the plugin run finished, and the list the \
         operator just looked at is the state that run left behind.",
        &log,
    );

    if let Ok(evidence) = std::env::var("NUVEM_EVIDENCE_DIR") {
        let argv = std::fs::read_to_string(dir.path().join("argv.log")).expect("the child ran");
        std::fs::write(
            std::path::PathBuf::from(evidence).join("child-transcript.txt"),
            format!(
                "gcloud invocations the app made, in order, as recorded by the child itself:\n{argv}\n\
                 operations log entry: kind={} outcome={:?} detail={:?}\n\
                 table rows after the run: {:?}\n",
                record.kind,
                record.outcome,
                record.detail,
                instance_names(session.app()),
            ),
        )
        .expect("write the transcript");
    }
}
