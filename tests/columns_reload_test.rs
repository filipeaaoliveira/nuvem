//! Configured columns from startup through a live `:` reload (task 4).
//!
//! The property under test is atomicity *per service*: a reload whose block for
//! one service no longer validates leaves that service's previously running
//! settings in force — never defaults, never a half-applied list — while every
//! other service adopts whatever the new file says. The reload is also required
//! to be loud about it, and to leave rows, selection and filters alone.

use std::io::Write;

use clap::Parser;
use nuvem::app::projection::binding::help_model_with;
use nuvem::app::projection::columns::catalog_meta;
use nuvem::app::projection::project_resources;
use nuvem::app::projection::table::{SortDirection, TableModel};
use nuvem::app::reducer::root::reduce;
use nuvem::app::{Action, AppState, EffectResult, ResourcesAction};
use nuvem::config::hotkeys::KeyOverrides;
use nuvem::services::runtime::apply_startup_config;
use nuvem::services::runtime::session::RuntimeSession;
use nuvem::{Cli, Config};
use nuvem_domain::gcp::configuration::{GcloudConfiguration, GcloudConfigurationName};
use nuvem_domain::gcp::resource::{ComputeInstance, ResourceServiceKind, ResourceStatus};

const COMPUTE: ResourceServiceKind = ResourceServiceKind::ComputeInstances;
const GKE: ResourceServiceKind = ResourceServiceKind::GkeClusters;
const STORAGE: ResourceServiceKind = ResourceServiceKind::CloudStorage;

fn write_config(body: &str) -> tempfile::NamedTempFile {
    let mut file = tempfile::NamedTempFile::new().expect("create temp config");
    file.write_all(body.as_bytes()).expect("write temp config");
    file.flush().expect("flush temp config");
    file
}

fn cli_for(path: &std::path::Path) -> Cli {
    Cli::parse_from([
        "nuvem",
        "--plugins",
        "/nonexistent/nuvem-plugins-test.toml",
        "--config",
        path.to_str().expect("utf-8 path"),
    ])
}

fn configuration() -> GcloudConfiguration {
    GcloudConfiguration {
        name: GcloudConfigurationName::new("dev"),
        is_active: true,
        account: Some("dev@example.com".to_string()),
        project: Some("dev-project".to_string()),
        region: Some("europe-west1".to_string()),
        zone: Some("europe-west1-b".to_string()),
    }
}

fn instance(name: &str, status: ResourceStatus, zone: &str) -> ComputeInstance {
    ComputeInstance {
        id: name.to_string(),
        name: name.to_string(),
        zone: zone.to_string(),
        status,
        machine_type: "e2-medium".to_string(),
        internal_ip: Some("10.0.0.4".to_string()),
        external_ip: None,
    }
}

/// The three fixture rows, whose name order and status order are exact
/// reverses of each other: any assertion about which column the table is
/// sorted by can tell the two apart.
fn instances() -> Vec<ComputeInstance> {
    vec![
        instance("api", ResourceStatus::Suspended, "europe-west1-a"),
        instance("db", ResourceStatus::Stopped, "europe-west1-b"),
        instance("web", ResourceStatus::Pending, "europe-west1-c"),
    ]
}

/// A session on the compute list, wired the way `run()` wires the real one:
/// the resolved config goes through `apply_startup_config` before any rows
/// load, and the same file backs the `:` reload.
fn session_with_compute_rows(path: &std::path::Path) -> RuntimeSession {
    let cli = cli_for(path);
    let outcome = Config::resolve(&cli);
    assert!(
        outcome.problems.is_empty(),
        "the starting config must validate cleanly: {:?}",
        outcome.problems.messages()
    );
    let config = outcome.config;

    let mut app = AppState::new();
    apply_startup_config(&mut app, &config);
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ConfigurationsLoaded {
            configurations: vec![configuration()],
        }),
    );
    reduce(
        &mut app,
        &Action::Resources(ResourcesAction::SelectService(COMPUTE)),
    );
    let context = app.root.current_context().expect("active context").clone();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ComputeInstancesLoaded {
            context,
            instances: instances(),
        }),
    );

    RuntimeSession::new_with_cli_and_config_for_test(cli, config, app)
}

async fn reload(session: &mut RuntimeSession, path: &std::path::Path, body: &str) {
    std::fs::write(path, body).expect("rewrite temp config");
    session
        .dispatch_action(Action::Effect(nuvem::app::Effect::ReloadConfig))
        .await;
}

fn table(session: &RuntimeSession) -> TableModel {
    project_resources(&session.app().root.resources)
        .table
        .expect("list mode builds a table model")
}

fn header_keys(session: &RuntimeSession) -> Vec<String> {
    table(session)
        .columns
        .iter()
        .map(|column| column.key.to_string())
        .collect()
}

/// The keys of a service's effective layout, so a test reads the columns the
/// way the user wrote them instead of as catalog indices.
fn layout_keys(session: &RuntimeSession, kind: ResourceServiceKind) -> Vec<String> {
    let meta = catalog_meta(kind);
    session
        .app()
        .root
        .resources
        .column_layout(kind)
        .iter()
        .map(|&index| meta[index].key.to_string())
        .collect()
}

fn status_message(session: &RuntimeSession) -> String {
    session
        .app()
        .root
        .system
        .status_message
        .clone()
        .unwrap_or_default()
}

const REORDERED_COMPUTE: &str =
    "version = 1\n\n[columns.compute]\ncolumns = [\"zone\", \"name\", \"status\"]\n";

#[test]
fn startup_installs_the_configured_columns() {
    let file = write_config(REORDERED_COMPUTE);
    let session = session_with_compute_rows(file.path());

    assert_eq!(header_keys(&session), ["zone", "name", "status"]);
}

#[test]
fn a_file_without_a_columns_table_leaves_the_default_columns_at_startup() {
    let file = write_config("version = 1\n[general]\npoll_interval_secs = 12\n");
    let session = session_with_compute_rows(file.path());

    let expected: Vec<String> = catalog_meta(COMPUTE)
        .iter()
        .map(|column| column.key.to_string())
        .collect();
    assert_eq!(header_keys(&session), expected);
}

#[tokio::test]
async fn a_newly_invalid_block_keeps_the_running_columns_and_says_why() {
    let file = write_config(REORDERED_COMPUTE);
    let mut session = session_with_compute_rows(file.path());

    reload(
        &mut session,
        file.path(),
        "version = 1\n\n[columns.compute]\ncolumns = [\"name\", \"zzz\"]\n",
    )
    .await;

    // The previously running list, not the catalog defaults and not a
    // half-applied `["name"]`.
    assert_eq!(header_keys(&session), ["zone", "name", "status"]);
    assert_eq!(table(&session).rows.len(), 3, "the table is never emptied");

    let status = status_message(&session);
    assert!(
        status.contains("unknown column \"zzz\""),
        "the reload must say why nothing changed, got: {status:?}"
    );
}

#[tokio::test]
async fn an_invalid_block_does_not_hold_back_another_services_valid_one() {
    let file = write_config(
        r#"version = 1

[columns.compute]
columns = ["zone", "name", "status"]

[columns.gke]
columns = ["name", "location"]
"#,
    );
    let mut session = session_with_compute_rows(file.path());

    reload(
        &mut session,
        file.path(),
        r#"version = 1

[columns.compute]
columns = ["name", "zzz"]

[columns.gke]
columns = ["name", "version", "location"]
"#,
    )
    .await;

    assert_eq!(layout_keys(&session, COMPUTE), ["zone", "name", "status"]);
    assert_eq!(
        layout_keys(&session, GKE),
        ["name", "version", "location"],
        "a valid block must be adopted even while another service's is broken"
    );
}

#[tokio::test]
async fn removing_a_block_restores_that_services_default_columns() {
    let file = write_config(REORDERED_COMPUTE);
    let mut session = session_with_compute_rows(file.path());
    assert_eq!(header_keys(&session), ["zone", "name", "status"]);

    reload(&mut session, file.path(), "version = 1\n").await;

    let expected: Vec<String> = catalog_meta(COMPUTE)
        .iter()
        .map(|column| column.key.to_string())
        .collect();
    assert_eq!(header_keys(&session), expected);
    let status = status_message(&session);
    assert!(
        status.starts_with("configuration reloaded"),
        "a clean reload must not warn, got: {status:?}"
    );
}

#[tokio::test]
async fn the_reloaded_settings_reach_the_stored_config() {
    // `nuvem info` reads the stored config, so it must never report a column
    // list the app has not actually installed — nor keep reporting one it has
    // stopped using.
    let file = write_config(REORDERED_COMPUTE);
    let mut session = session_with_compute_rows(file.path());

    reload(
        &mut session,
        file.path(),
        "version = 1\n\n[columns.compute]\ncolumns = [\"name\", \"status\"]\n",
    )
    .await;

    let stored = session.config().expect("session carries a config");
    let installed = session.app().root.resources.column_layout(COMPUTE);
    assert_eq!(
        stored
            .columns
            .get(COMPUTE)
            .and_then(|compute| compute.columns.clone()),
        Some(installed.to_vec())
    );
}

#[tokio::test]
async fn a_reload_leaves_rows_selection_and_filters_alone() {
    let file = write_config(REORDERED_COMPUTE);
    let mut session = session_with_compute_rows(file.path());

    // A user mid-session: a filter applied and a row other than the first
    // selected.
    for action in [
        ResourcesAction::StartFilter,
        ResourcesAction::AppendFilterChar('b'),
        ResourcesAction::ApplyFilter,
        ResourcesAction::SelectNext,
    ] {
        session.dispatch_action(Action::Resources(action)).await;
    }
    let before = table(&session);
    assert_eq!(before.rows.len(), 2, "the filter narrows to api and db");
    assert_eq!(before.selected_row, Some(1));
    let load_phase = session.app().root.resources.active_load_phase().clone();

    reload(
        &mut session,
        file.path(),
        "version = 1\n\n[columns.compute]\ncolumns = [\"name\", \"status\", \"zone\"]\n",
    )
    .await;

    let after = table(&session);
    assert_eq!(after.rows.len(), before.rows.len());
    assert_eq!(after.selected_row, before.selected_row);
    assert_eq!(
        session.app().root.resources.filter_applied(),
        "b",
        "the applied filter survives a reload"
    );
    assert_eq!(
        session.app().root.resources.active_load_phase(),
        &load_phase,
        "a reload must not touch the list's load phase"
    );
}

#[tokio::test]
async fn a_reload_that_drops_the_sorted_column_re_anchors_the_sort() {
    // The reachable sequence: the user sorts by STATUS, then reloads a config
    // whose list no longer shows it. Leaving `sort_field` on Status would order
    // the rows by an invisible column while the arrow sat on a visible one.
    let file = write_config(REORDERED_COMPUTE);
    let mut session = session_with_compute_rows(file.path());

    // Cursor: zone (display 0) -> name -> status.
    session
        .dispatch_action(Action::Resources(ResourcesAction::SelectNextColumn))
        .await;
    session
        .dispatch_action(Action::Resources(ResourcesAction::SelectNextColumn))
        .await;
    session
        .dispatch_action(Action::Resources(ResourcesAction::SortBySelectedColumn))
        .await;
    assert_eq!(session.app().root.resources.sort_label(), "Status");

    reload(
        &mut session,
        file.path(),
        "version = 1\n\n[columns.compute]\ncolumns = [\"zone\", \"name\"]\n",
    )
    .await;

    let table = table(&session);
    let sorted_by = table.columns[table.sort.column].key;
    assert_eq!(sorted_by, "name", "the arrow lands on a displayed column");
    assert_eq!(session.app().root.resources.sort_label(), "Name");
    assert_eq!(table.sort.direction, SortDirection::Ascending);

    let names: Vec<&str> = table
        .rows
        .iter()
        .map(|row| row.cells[table.sort.column].text.as_str())
        .collect();
    assert_eq!(
        names,
        ["api", "db", "web"],
        "the rows are ordered by the column the arrow points at"
    );
}

#[tokio::test]
async fn a_reload_installs_a_newly_configured_default_sort() {
    // The mirror of the case above: a reload that *adds* a sort applies it,
    // overriding the sort the user reached with `s`.
    let file = write_config(REORDERED_COMPUTE);
    let mut session = session_with_compute_rows(file.path());

    reload(
        &mut session,
        file.path(),
        "version = 1\n\n[columns.compute]\ncolumns = [\"zone\", \"name\", \"status\"]\nsort = \"status\"\nsort_direction = \"desc\"\n",
    )
    .await;

    let table = table(&session);
    assert_eq!(table.columns[table.sort.column].key, "status");
    assert_eq!(table.sort.direction, SortDirection::Descending);
    assert_eq!(session.app().root.resources.sort_label(), "Status");
}

/// Compute opening on STATUS, with a second service configured so a reload can
/// change that service's block and nothing else.
const SORTED_COMPUTE_AND_GKE: &str = r#"version = 1

[columns.compute]
columns = ["zone", "name", "status"]
sort = "status"

[columns.gke]
columns = ["name", "location"]
"#;

/// The name cells in display order, which tell the fixture's orderings apart:
/// by name (or zone) the rows read api, db, web, and by status they read the
/// exact reverse.
fn sorted_names(session: &RuntimeSession) -> Vec<String> {
    let table = table(session);
    let name_position = table
        .columns
        .iter()
        .position(|column| column.key == "name")
        .expect("every configured list shows the name column");
    table
        .rows
        .iter()
        .map(|row| row.cells[name_position].text.clone())
        .collect()
}

/// Puts the user's own sort on compute: `s` on ZONE, which the configured
/// `sort = "status"` opened away from.
async fn sort_compute_by_zone(session: &mut RuntimeSession) {
    session
        .dispatch_action(Action::Resources(ResourcesAction::SortBySelectedColumn))
        .await;
    assert_eq!(
        session.app().root.resources.sort_label(),
        "Zone",
        "the cursor starts on zone, so `s` sorts by it"
    );
}

#[tokio::test]
async fn a_reload_about_another_service_leaves_the_users_own_sort_alone() {
    // The configured `sort` is a *default*: it says what the list opens on,
    // not what it must go back to whenever some unrelated block is edited.
    let file = write_config(SORTED_COMPUTE_AND_GKE);
    let mut session = session_with_compute_rows(file.path());
    assert_eq!(session.app().root.resources.sort_label(), "Status");

    sort_compute_by_zone(&mut session).await;

    reload(
        &mut session,
        file.path(),
        r#"version = 1

[columns.compute]
columns = ["zone", "name", "status"]
sort = "status"

[columns.gke]
columns = ["name", "version", "location"]
"#,
    )
    .await;

    assert_eq!(
        layout_keys(&session, GKE),
        ["name", "version", "location"],
        "the reload really did adopt the block that changed"
    );

    let table = table(&session);
    assert_eq!(session.app().root.resources.sort_label(), "Zone");
    assert_eq!(table.columns[table.sort.column].key, "zone");
    assert_eq!(table.sort.direction, SortDirection::Ascending);
    assert_eq!(
        sorted_names(&session),
        ["api", "db", "web"],
        "the rows keep the order the user sorted them into"
    );
}

#[tokio::test]
async fn a_reload_of_the_services_own_block_reinstalls_its_configured_sort() {
    // The other side of the same rule: when this service's block is what
    // changed, its configured default sort is what the user just asked for.
    let file = write_config(SORTED_COMPUTE_AND_GKE);
    let mut session = session_with_compute_rows(file.path());

    sort_compute_by_zone(&mut session).await;

    reload(
        &mut session,
        file.path(),
        r#"version = 1

[columns.compute]
columns = ["name", "zone", "status"]
sort = "status"

[columns.gke]
columns = ["name", "location"]
"#,
    )
    .await;

    let table = table(&session);
    assert_eq!(session.app().root.resources.sort_label(), "Status");
    assert_eq!(table.columns[table.sort.column].key, "status");
    assert_eq!(table.sort.direction, SortDirection::Ascending);
    assert_eq!(
        sorted_names(&session),
        ["web", "db", "api"],
        "the configured sort, not the user's, decides the order"
    );
}

#[tokio::test]
async fn a_load_error_leaves_every_services_columns_running() {
    // The existing contract: a file-level failure keeps the entire prior
    // config, so no service's table moves. Both services are configured away
    // from their defaults, so a reload that wiped the settings — or adopted
    // half of them — would show up in either assertion.
    let file = write_config(
        r#"version = 1

[columns.compute]
columns = ["zone", "name", "status"]

[columns.storage]
columns = ["name", "class"]
"#,
    );
    let mut session = session_with_compute_rows(file.path());

    reload(&mut session, file.path(), "this is not valid toml = = =\n").await;

    assert_eq!(layout_keys(&session, COMPUTE), ["zone", "name", "status"]);
    assert_eq!(layout_keys(&session, STORAGE), ["name", "class"]);
    assert!(
        status_message(&session).contains("config reload failed"),
        "a load error is still reported"
    );
}

/// The problems recorded by the most recent reload, as the help overlay would
/// list them.
fn problems(session: &RuntimeSession) -> Vec<String> {
    session.app().root.system.config_problems().to_vec()
}

/// A single reload warning keeps the earlier wording, byte for byte.
#[tokio::test]
async fn a_single_reload_problem_keeps_the_original_wording() {
    let file = write_config(REORDERED_COMPUTE);
    let mut session = session_with_compute_rows(file.path());

    reload(
        &mut session,
        file.path(),
        "version = 1\n[general]\ntheme = \"bogus\"\n",
    )
    .await;

    assert_eq!(
        status_message(&session),
        "config reloaded (warning: unknown general.theme \"bogus\"; using default)"
    );
    assert_eq!(
        problems(&session),
        ["unknown general.theme \"bogus\"; using default"]
    );
}

/// Two problems: the status line counts them and points at help, which lists
/// both instead of hiding everything after the first.
#[tokio::test]
async fn several_reload_problems_are_counted_and_all_recorded() {
    let file = write_config(REORDERED_COMPUTE);
    let mut session = session_with_compute_rows(file.path());

    reload(
        &mut session,
        file.path(),
        "version = 1\n[general]\ndefault_service = \"nope\"\ntheme = \"bogus\"\n",
    )
    .await;

    assert_eq!(
        status_message(&session),
        "config reloaded (2 problems; press ? for details)"
    );
    assert_eq!(
        problems(&session),
        [
            "unknown general.default_service \"nope\"; using default",
            "unknown general.theme \"bogus\"; using default",
        ]
    );
    assert_eq!(
        help_model_with(&session.app().root, &KeyOverrides::empty()).problems,
        problems(&session),
        "the help overlay lists every problem"
    );
}

/// The reload pointer names the help key the reloaded config just installed,
/// never a hardcoded `?`.
#[tokio::test]
async fn the_reload_pointer_names_the_rebound_help_key() {
    let file = write_config(REORDERED_COMPUTE);
    let mut session = session_with_compute_rows(file.path());

    reload(
        &mut session,
        file.path(),
        "version = 1\n[general]\ndefault_service = \"nope\"\ntheme = \"bogus\"\n\n[hotkeys]\nhelp = \"H\"\n",
    )
    .await;

    let status = status_message(&session);
    assert_eq!(status, "config reloaded (2 problems; press H for details)");
    assert!(
        !status.contains('?'),
        "the default key must not be named once help is rebound: {status:?}"
    );
}

/// The defect this project keeps ruling against: a reload that *fixes* the
/// config must not leave the old problems on display.
#[tokio::test]
async fn a_clean_reload_clears_the_previous_problems() {
    let file = write_config(REORDERED_COMPUTE);
    let mut session = session_with_compute_rows(file.path());

    reload(
        &mut session,
        file.path(),
        "version = 1\n[general]\ndefault_service = \"nope\"\ntheme = \"bogus\"\n",
    )
    .await;
    assert_eq!(problems(&session).len(), 2, "problems are present first");

    reload(&mut session, file.path(), REORDERED_COMPUTE).await;

    assert_eq!(status_message(&session), "configuration reloaded");
    assert!(
        problems(&session).is_empty(),
        "a fixed config must leave nothing behind: {:?}",
        problems(&session)
    );
    assert!(
        help_model_with(&session.app().root, &KeyOverrides::empty())
            .problems
            .is_empty(),
        "the help section is gone once the config is clean"
    );
}

/// The tail differs *per service*, not per reload, because it is
/// derived from what was already running when that service's block broke —
/// never from a single "is this a reload" flag shared by the whole file.
/// Compute enters this reload with a running list of its own; gke never had
/// one, so the same reload must still tell them apart.
#[tokio::test]
async fn a_reload_with_two_invalid_blocks_ties_each_tail_to_its_own_history() {
    let file = write_config(
        r#"version = 1

[columns.compute]
columns = ["zone", "name", "status"]
"#,
    );
    let mut session = session_with_compute_rows(file.path());

    reload(
        &mut session,
        file.path(),
        r#"version = 1

[columns.compute]
columns = ["name", "zzz"]

[columns.gke]
columns = ["zzz"]
"#,
    )
    .await;

    assert_eq!(
        problems(&session),
        [
            "columns.compute: unknown column \"zzz\"; ignoring this block, keeping the column settings already in use",
            "columns.gke: unknown column \"zzz\"; ignoring this service's column settings",
        ]
    );

    // The sentences are tied to what the user can actually see: compute kept
    // its configured list, gke fell back to its catalog default.
    assert_eq!(header_keys(&session), ["zone", "name", "status"]);
    let gke_default: Vec<String> = catalog_meta(GKE)
        .iter()
        .map(|column| column.key.to_string())
        .collect();
    assert_eq!(layout_keys(&session, GKE), gke_default);

    // The standing help overlay is what was made durable; both sentences
    // must reach it, not just the status line.
    assert_eq!(
        help_model_with(&session.app().root, &KeyOverrides::empty()).problems,
        problems(&session),
        "the help overlay lists both per-service sentences"
    );
}

/// The fresh-load counterpart of the test above: nothing is running yet, so
/// even a service that would otherwise qualify for `KEPT` on a reload gets
/// the `FALLBACK` sentence here. Pins the fresh-load path so it cannot drift
/// onto the reload wording.
#[test]
fn a_fresh_load_with_an_invalid_block_falls_back_and_says_so() {
    let file = write_config("version = 1\n\n[columns.compute]\ncolumns = [\"name\", \"zzz\"]\n");
    let cli = cli_for(file.path());

    let outcome = Config::resolve(&cli);

    assert_eq!(
        outcome.problems.messages(),
        ["columns.compute: unknown column \"zzz\"; ignoring this service's column settings"]
    );
}

/// A load-level reload failure still replaces the problems list, so the help
/// overlay describes the file that just failed rather than the previous one.
#[tokio::test]
async fn a_failed_reload_replaces_the_recorded_problems() {
    let file = write_config(REORDERED_COMPUTE);
    let mut session = session_with_compute_rows(file.path());

    reload(
        &mut session,
        file.path(),
        "version = 1\n[general]\ndefault_service = \"nope\"\ntheme = \"bogus\"\n",
    )
    .await;
    assert_eq!(problems(&session).len(), 2);

    reload(&mut session, file.path(), "this is not valid toml = = =\n").await;

    assert!(
        status_message(&session).contains("config reload failed"),
        "a load error is still reported"
    );
    let recorded = problems(&session);
    assert_eq!(
        recorded.len(),
        1,
        "the stale field-level problems are gone: {recorded:?}"
    );
    assert!(
        !recorded[0].contains("general.theme"),
        "the recorded problem describes the failed load: {recorded:?}"
    );
}
