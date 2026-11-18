//! Drives `DemoCommandRunner` through the real `Gcloud*Service` types to prove
//! the embedded fixtures parse successfully via the actual service layer, not
//! just via string comparisons against fixture bytes.

use std::time::{Duration, Instant};

use clap::Parser; // for Cli::parse_from
use nuvem::app::projection::project_resource_list;
use nuvem::app::projection::table::{Cell, CellKind};
use nuvem::app::state::resources::BigQueryView;
use nuvem::app::{Action, AppState, EffectResult, ResourcesAction, WorkspaceAction};
use nuvem::services::demo::DemoCommandRunner;
use nuvem::services::gcloud::cli::{CommandError, CommandRunner, GcloudCommand};
use nuvem::services::gcloud::configurations::GcloudConfigurationService;
use nuvem::services::gcloud::diagnostics::GcloudDiagnosticsService;
use nuvem::services::gcloud::discovery::GcloudDiscoveryService;
use nuvem::services::resources::bigquery::GcloudBigQueryService;
use nuvem::services::resources::cloud_run::GcloudCloudRunService;
use nuvem::services::resources::cloudsql::GcloudCloudSqlService;
use nuvem::services::resources::compute::GcloudComputeService;
use nuvem::services::resources::finops::GcloudFinOpsService;
use nuvem::services::resources::gke::GcloudGkeService;
use nuvem::services::resources::logging::GcloudLoggingService;
use nuvem::services::resources::storage::GcloudStorageService;
use nuvem::services::runtime::session::RuntimeSession;
use nuvem_domain::gcp::configuration::{
    GcloudConfiguration, GcloudConfigurationName, GcloudContext,
};
use nuvem_domain::gcp::finops::RecommenderId;
use nuvem_domain::gcp::logging::{LogQuery, LogSeverity, LogTimeRange};
use nuvem_domain::gcp::resource::{
    ComputeInstance, ResourceServiceKind, ResourceStatus, TableStat,
};

fn context() -> GcloudContext {
    GcloudContext {
        configuration: GcloudConfigurationName::new("dev"),
        account: Some("dev@example.com".to_string()),
        project: Some("dev-project".to_string()),
        region: None,
        zone: None,
    }
}

#[test]
fn configurations_load_through_demo_runner() {
    let service = GcloudConfigurationService::new(DemoCommandRunner::new());

    let configurations = service
        .load_configurations()
        .expect("demo configurations load");

    assert_eq!(configurations.len(), 2);
    assert_eq!(
        configurations.iter().filter(|c| c.is_active).count(),
        1,
        "exactly one configuration should be active"
    );
}

#[test]
fn compute_instances_load_detail_and_action_through_demo_runner() {
    let service = GcloudComputeService::new(DemoCommandRunner::new());

    let instances = service
        .load_instances(&context(), None)
        .expect("demo instances load");
    assert_eq!(instances.len(), 2);

    let detail = service
        .load_instance_detail(&context(), &instances[0])
        .expect("demo instance detail load");
    assert!(!detail.name.is_empty());

    service
        .stop_instance(&context(), &instances[0])
        .expect("demo stop_instance should succeed as a no-op");
}

#[test]
fn gke_clusters_load_and_detail_through_demo_runner() {
    let service = GcloudGkeService::new(DemoCommandRunner::new());

    let clusters = service
        .load_clusters(&context(), None)
        .expect("demo gke list");
    assert_eq!(clusters.len(), 3);

    let detail = service
        .load_cluster_detail(&context(), &clusters[0])
        .expect("demo gke detail");
    assert!(!detail.name.is_empty());
}

#[test]
fn cloud_run_services_revisions_and_detail_through_demo_runner() {
    let service = GcloudCloudRunService::new(DemoCommandRunner::new());

    let services = service
        .load_services(&context())
        .expect("demo cloud run services");
    assert_eq!(services.len(), 5);

    let detail = service
        .load_service_detail(&context(), &services[0])
        .expect("demo cloud run service detail");
    assert!(!detail.name.is_empty());

    let revisions = service
        .load_revisions(&context(), &services[0])
        .expect("demo cloud run revisions");
    assert_eq!(revisions.len(), 2);

    let revision_detail = service
        .load_revision_detail(&context(), &services[0].region, &revisions[0].name)
        .expect("demo cloud run revision detail");
    assert!(!revision_detail.name.is_empty());
}

#[test]
fn cloud_sql_instances_detail_and_users_through_demo_runner() {
    let service = GcloudCloudSqlService::new(DemoCommandRunner::new());

    let instances = service
        .load_instances(&context(), None)
        .expect("demo cloud sql instances");
    assert_eq!(instances.len(), 5);

    let detail = service
        .load_instance_detail(&context(), &instances[0])
        .expect("demo cloud sql instance detail");
    assert!(!detail.name.is_empty());

    let users = service
        .load_users(&context(), instances[0].name.as_str())
        .expect("demo cloud sql users");
    assert_eq!(users.len(), 2);
}

#[test]
fn storage_buckets_detail_and_object_pages_through_demo_runner() {
    let service = GcloudStorageService::new(DemoCommandRunner::new());

    let buckets = service.load_buckets(&context()).expect("demo buckets");
    assert_eq!(buckets.len(), 3);

    let detail = service
        .load_bucket_detail(&context(), buckets[0].name.as_str())
        .expect("demo bucket detail");
    assert!(!detail.name.is_empty());

    let first_page = service
        .load_object_page(&context(), buckets[0].name.as_str(), "", None, 50)
        .expect("demo object first page");
    assert!(!first_page.objects.is_empty());
    assert!(
        !first_page.prefixes.is_empty(),
        "folders are derived from keys"
    );
    // gcloud emits no page token, so demo mode serves one page and the browser
    // never offers to fetch a second.
    assert_eq!(first_page.next_page_token, None);

    let nested = service
        .load_object_page(&context(), buckets[0].name.as_str(), "photos/", None, 50)
        .expect("demo nested page");
    assert!(
        !nested.objects.is_empty(),
        "entering a folder lists its objects"
    );

    let object_detail = service
        .load_object_detail(&context(), buckets[0].name.as_str(), "some/object.txt")
        .expect("demo object detail");
    assert!(!object_detail.key.is_empty());
}

#[test]
fn discovery_projects_regions_and_zones_through_demo_runner() {
    let service = GcloudDiscoveryService::new(DemoCommandRunner::new());
    let configuration = GcloudConfigurationName::new("dev");

    let projects = service
        .load_projects(&configuration)
        .expect("demo projects");
    assert_eq!(projects.len(), 2);

    let regions = service.load_regions(&configuration).expect("demo regions");
    assert_eq!(regions.len(), 2);

    let zones = service.load_zones(&configuration).expect("demo zones");
    assert_eq!(zones.len(), 2);
}

#[test]
fn logging_read_recent_through_demo_runner() {
    let service = GcloudLoggingService::new(DemoCommandRunner::new());

    let outcome = service
        .read_recent(
            &context(),
            &LogQuery::ProjectLogs,
            LogSeverity::Default,
            LogTimeRange::OneHour,
            200,
        )
        .expect("demo log read");

    assert!(!outcome.entries.is_empty());
}

#[test]
fn compute_action_wrappers_succeed_as_no_ops_through_demo_runner() {
    let service = GcloudComputeService::new(DemoCommandRunner::new());
    let instance = ComputeInstance {
        id: "1001".to_string(),
        name: "api-dev-1".to_string(),
        zone: "europe-west1-b".to_string(),
        status: ResourceStatus::Running,
        machine_type: "e2-medium".to_string(),
        internal_ip: None,
        external_ip: None,
    };

    service
        .start_instance(&context(), &instance)
        .expect("demo start_instance should succeed as a no-op");
    service
        .reset_instance(&context(), &instance)
        .expect("demo reset_instance should succeed as a no-op");
}

/// Each of the three `bq` argvs must reach *its own* fixture, not merely some
/// fixture.
///
/// The two listings differ by one flag and one positional — `ls -d -n 500`
/// against `ls -n 500 <dataset>` — so "each argv returns something" and "each
/// argv returns the right thing" are different claims. Served the wrong
/// fixture, neither parse fails: every field of both raw shapes is
/// `#[serde(default)]` and each takes its identity from a nested object the
/// other does not carry, so `parse_datasets_json` reads the tables document as
/// two datasets with empty ids and `parse_tables_json` reads the datasets
/// document as four tables with empty ids. A count-based test passes either
/// way; pinning the ids is what tells the two apart, and it is checked through
/// the real service so the argv under test is the argv the app sends.
///
/// The `show` argv is here for the same reason and pins its three numbers: a
/// `show` served either listing would leave all three stats unavailable or
/// fail the parse, and both of those are indistinguishable from the refusal
/// path the fan-out is built to tolerate. `is_bq_table_show` is what routes
/// it; the exclusivity of the three predicates is checked as a table in
/// `each_bq_argv_satisfies_exactly_one_arm_predicate`.
#[test]
fn each_bigquery_argv_reaches_its_own_fixture() {
    let service = GcloudBigQueryService::new(DemoCommandRunner::new());

    let datasets = service
        .load_datasets(&context())
        .expect("demo dataset list");
    let tables = service
        .load_tables(&context(), "austin_bikeshare")
        .expect("demo table list");

    assert_eq!(
        datasets
            .iter()
            .map(|dataset| dataset.dataset_id.as_str())
            .collect::<Vec<_>>(),
        vec![
            "agent_analytic_sample_dataset",
            "america_health_rankings",
            "austin_311",
            "austin_bikeshare",
        ],
        "the datasets argv must reach the datasets fixture"
    );
    assert_eq!(
        tables
            .iter()
            .map(|table| table.table_id.as_str())
            .collect::<Vec<_>>(),
        vec!["bikeshare_stations", "bikeshare_trips"],
        "the tables argv must reach the tables fixture"
    );

    let stats = service
        .load_table_stats(&context(), "austin_bikeshare", "bikeshare_stations")
        .expect("demo table show");
    assert_eq!(
        (stats.rows, stats.bytes, stats.last_modified),
        (
            TableStat::Known(4238),
            TableStat::Known(13_208_066),
            TableStat::Known(1_787_306_613_694),
        ),
        "the show argv must reach the table-show fixture and paint its numbers"
    );
}

/// Drilling into a dataset in demo mode must actually paint tables.
///
/// The arm parsing in isolation is not the same as the drill working: the
/// `Enter` has to resolve a context, issue `Effect::LoadBigQueryTables`, and
/// have the result land inside the dataset it named. This drives all of that
/// through the session with only the demo runner behind it — the exact
/// wiring `--demo` runs.
#[tokio::test]
async fn entering_a_dataset_lists_its_tables_through_the_demo_runner() {
    let mut session = RuntimeSession::new_with_runner_for_test(
        AppState::new(),
        Box::new(DemoCommandRunner::new()),
    );
    session
        .apply_effect_result(EffectResult::ConfigurationsLoaded {
            configurations: vec![GcloudConfiguration {
                name: GcloudConfigurationName::new("dev"),
                is_active: true,
                account: Some("dev@example.com".to_string()),
                project: Some("dev-project".to_string()),
                region: None,
                zone: None,
            }],
        })
        .await;
    session
        .dispatch_action(Action::Resources(ResourcesAction::SelectService(
            ResourceServiceKind::BigQueryDatasets,
        )))
        .await;
    // `SelectService` is the only load dispatched here, deliberately: a
    // `Refresh` on top of it would queue a second listing and bump the
    // operation generation, which discards the first result — leaving the
    // assertion below reading whichever of the two happened to be processed.
    //
    // Timed out rather than awaited bare: `process_next_effect_result` is an
    // unbounded `recv` on a channel the session itself holds a sender for, so
    // it never returns and a reclassified effect turns this line into a silent
    // ten-second hang. With the bound it is a named failure.
    let applied =
        tokio::time::timeout(Duration::from_secs(5), session.process_next_effect_result())
            .await
            .expect("the datasets listing should produce an effect result");
    assert!(applied, "effect-result channel closed");
    assert_eq!(session.app().root.resources.bigquery_datasets().len(), 4);

    session
        .dispatch_action(Action::Resources(ResourcesAction::OpenSelected))
        .await;
    // No second `process_next_effect_result` here: the tables listing runs as
    // a foreground effect, applied inside `dispatch_action` itself, so there
    // is no queued result left to drain and awaiting one would hang.

    assert_eq!(
        session.app().root.resources.bigquery_view(),
        BigQueryView::Tables
    );
    assert_eq!(
        session
            .app()
            .root
            .resources
            .bigquery_tables()
            .iter()
            .map(|table| table.table_id.as_str())
            .collect::<Vec<_>>(),
        vec!["bikeshare_stations", "bikeshare_trips"],
        "demo mode must paint the tables fixture inside the entered dataset"
    );

    // The listing fans out into one `bq show` per row, dispatched off the run
    // loop, so the two answers arrive on the result channel. Bounded for the
    // same reason as the wait above: a slice that queued one fewer would hang
    // here rather than fail by name.
    for index in 0..2 {
        let next =
            tokio::time::timeout(Duration::from_secs(5), session.process_next_effect_result())
                .await
                .unwrap_or_else(|_| {
                    panic!("no queued show result {index} arrived: two listed tables are two shows")
                });
        assert!(next, "effect-result channel closed");
    }
    let rows = session.app().root.resources.bigquery_tables().to_vec();
    assert!(
        rows.iter()
            .all(|table| (table.rows, table.bytes, table.last_modified)
                == (
                    TableStat::Known(4238),
                    TableStat::Known(13_208_066),
                    TableStat::Known(1_787_306_613_694),
                )),
        "demo mode must paint the show fixture's numbers, not the dash a refused \
         call leaves — both rows land the one captured table resource: {rows:?}"
    );
}

/// And one more `Enter` opens that table's columns, nested ones included.
///
/// This is the claim the `show` arm's fixture choice exists to make. Both
/// callers of `bq show` send the same argv, so demo mode cannot serve the
/// stats fan-out and the schema drill different documents — and the capture
/// that answers both is chosen for the half only this view reads. A flat
/// schema would paint scalars, which is exactly the shape a flattener that
/// dropped the record's children, or kept only their leaf names, would also
/// produce.
///
/// So the assertions are about nesting and about the absent description: the
/// two-level dotted path, the `REPEATED` record above it, and the field the
/// capture documents with nothing.
#[tokio::test]
async fn entering_a_table_lists_its_columns_through_the_demo_runner() {
    let mut session = RuntimeSession::new_with_runner_for_test(
        AppState::new(),
        Box::new(DemoCommandRunner::new()),
    );
    session
        .apply_effect_result(EffectResult::ConfigurationsLoaded {
            configurations: vec![GcloudConfiguration {
                name: GcloudConfigurationName::new("dev"),
                is_active: true,
                account: Some("dev@example.com".to_string()),
                project: Some("dev-project".to_string()),
                region: None,
                zone: None,
            }],
        })
        .await;
    session
        .dispatch_action(Action::Resources(ResourcesAction::SelectService(
            ResourceServiceKind::BigQueryDatasets,
        )))
        .await;
    let applied =
        tokio::time::timeout(Duration::from_secs(5), session.process_next_effect_result())
            .await
            .expect("the datasets listing should produce an effect result");
    assert!(applied, "effect-result channel closed");

    // Into the dataset, then into the table the cursor lands on.
    session
        .dispatch_action(Action::Resources(ResourcesAction::OpenSelected))
        .await;
    session
        .dispatch_action(Action::Resources(ResourcesAction::OpenSelected))
        .await;

    assert_eq!(
        session.app().root.resources.bigquery_view(),
        BigQueryView::Schema,
        "the second Enter must descend a second level rather than no-op"
    );
    let fields = session
        .app()
        .root
        .resources
        .bigquery_schema_fields()
        .to_vec();
    let paths = fields
        .iter()
        .map(|field| field.path.as_str())
        .collect::<Vec<_>>();
    assert!(
        paths.contains(&"event_params.value.string_value"),
        "demo mode must show a record flattened to its dotted path two levels \
         down, which is what a flat capture could not show: {paths:?}"
    );
    assert!(
        paths.contains(&"event_params"),
        "and the parent record must be kept as a row of its own, not replaced \
         by its children: {paths:?}"
    );
    assert!(
        fields
            .iter()
            .any(|field| field.path == "event_params" && field.mode == "REPEATED"),
        "the repeated record must carry its own mode: {fields:?}"
    );
    assert!(
        fields.iter().any(|field| field.description.is_none()),
        "and at least one field must be undescribed, so demo mode paints the \
         dash rather than only the documented half of the column: {fields:?}"
    );
}

/// How long a demo session is given to settle before the drain below calls it
/// hung.
///
/// A backstop, not a pacing knob: a session that settles leaves the loop on the
/// condition and never waits this long. It sits under the suite's own ten-second
/// per-test ceiling (`.config/nextest.toml`) so a session that never settles
/// fails here — naming the count of tasks still able to send — rather than being
/// killed by the harness with nothing said about which service hung.
const SETTLE_TIMEOUT: Duration = Duration::from_secs(5);

/// How long one `recv` waits before the loop below re-reads the in-flight
/// count.
///
/// A pacing knob that costs real wall-clock time. The comment this replaces
/// called it free — "making this longer or shorter changes when the question is
/// asked and never what the answer is" — and that was measurably false:
/// [`drain`] ends on a poll that finds the channel empty, and finding a channel
/// empty takes the whole interval. Every `drain` call pays at least one — the
/// last poll it makes — and pays another for any earlier poll that times out
/// while work is still in flight. The walk below drains four times per service
/// across sixteen services, so it spends sixty-four intervals doing nothing in
/// the settled case, which is the case it is normally in.
/// Measured: at 300ms the walk takes 19.42s under `cargo test`, well past
/// nextest's ten-second per-test ceiling, which kills it; at 5ms the probe is
/// ~0.32s of a 0.50s run.
///
/// What it genuinely does *not* decide is settledness. It is the ceiling on a
/// `recv` that is otherwise unbounded, not a window of silence the exit
/// consults: no value of it can make an unsettled session look settled, because
/// that question is answered by [`RuntimeSession::effect_tasks_in_flight`]
/// before the poll rather than by the poll. Shortening it is the only direction
/// that is free; lengthening it buys nothing and spends the suite's budget.
const SETTLE_PROBE: Duration = Duration::from_millis(5);

/// Drives the session until nothing more can arrive.
///
/// The exit condition is the effects being *over* — every spawned task has
/// delivered its result and dropped the sender clone it held — rather than a
/// window of silence. Silence is not settledness: one fixture call slower than
/// the window returns from here with the rows still empty, and the whole-vector
/// assertions below then fail on a row count, which is a flake wearing the exact
/// shape of the defect this gate exists to catch.
/// `RuntimeSession::effect_tasks_in_flight` is the session's own settled-or-not
/// signal — it counts the tasks that could still send — so zero, followed by a
/// poll that finds nothing, is the fact this waits for. In that order, and the
/// body says why. [`SETTLE_TIMEOUT`] is only a backstop against a wait that
/// would otherwise hang.
async fn drain(session: &mut RuntimeSession) {
    let deadline = Instant::now() + SETTLE_TIMEOUT;
    loop {
        // Count first, poll second, and that order is the whole property.
        //
        // A zero count means no clone of the result sender is alive, so from
        // this instant nothing further can be *sent*; the poll that follows is
        // then conclusive about what is already buffered, and an empty poll
        // means the session is over. The reverse order — sample quiet, then
        // count — is what this replaces, and on a `multi_thread` runtime it is
        // a race: a task can send its result and drop its sender inside the gap
        // between the two reads, so the poll saw silence and the count then saw
        // zero while that result sat unread in the channel, and `drain`
        // returned with a table still missing rows. The two other users of this
        // idiom in the tree are `current_thread` sessions, where no task can run
        // during the gap at all — which is why copying it here dropped the
        // property instead of carrying it over.
        let in_flight = session.effect_tasks_in_flight();
        let progressed = matches!(
            tokio::time::timeout(SETTLE_PROBE, session.process_next_effect_result()).await,
            Ok(true)
        );
        if !progressed && in_flight == 0 {
            return;
        }
        if !progressed {
            // Only an iteration that made no progress can be a hang, so only
            // those are held to the deadline. The unconditional assert this
            // replaces fired on iterations that had just applied a result, and
            // reported `0 effect task(s) still hold the result sender` — a
            // message that reads as the opposite of a hang and names the one
            // count that cannot be the cause of one. Reaching here means
            // `in_flight` was non-zero, so the number below is always the tasks
            // actually being waited on.
            //
            // A session that keeps producing results forever is deliberately
            // not this loop's failure to name: it is making progress, and the
            // suite's own per-test ceiling is what bounds it.
            assert!(
                Instant::now() < deadline,
                "the demo session did not settle within {SETTLE_TIMEOUT:?}: {in_flight} effect \
                 task(s) still hold the result sender"
            );
        }
    }
}

/// A `--demo` session driven to one service's loaded table, the way the keys
/// drive it: into the resources workspace, onto the service, then refresh.
///
/// A whole session per service rather than one session walked across all
/// sixteen, so a service cannot pass on rows another service's load left in
/// state.
async fn demo_session_showing(kind: ResourceServiceKind) -> RuntimeSession {
    let cli = nuvem::Cli::parse_from(["nuvem", "--demo", "--config", "/nonexistent/nuvem.toml"]);
    let config = nuvem::Config::resolve(&cli).config;
    assert!(config.demo, "the fixture has to be a demo session");
    let state_path = nuvem::config::state::state_path_for(&config.config_path);
    let mut session = RuntimeSession::new(config, cli, AppState::new(), state_path);

    session.initialize().await;
    drain(&mut session).await;
    session
        .dispatch_action(Action::Workspace(WorkspaceAction::Resources))
        .await;
    drain(&mut session).await;
    session
        .dispatch_action(Action::Resources(ResourcesAction::SelectService(kind)))
        .await;
    drain(&mut session).await;
    session
        .dispatch_action(Action::Resources(ResourcesAction::Refresh))
        .await;
    drain(&mut session).await;
    session
}

/// One painted cell, spelled so the assertion carries the styling as well as
/// the text.
///
/// A [`Cell`] is text *and* a kind, and the kind is not decoration. `Missing`
/// is the dimmed dash a table paints where a value is absent; a `Plain` cell
/// whose text happens to be `-` is a dash the parser itself produced, which an
/// operator reads as data; `Status` is the colour a health column is scanned
/// by. Three different cells can all spell `-`, and a text-only assertion could
/// not tell them apart. So a `Plain` cell is written bare — the common case,
/// and the literal below is then exactly the string on screen — and every other
/// kind is prefixed with its own name, lowercased.
fn painted(cell: &Cell) -> String {
    match &cell.kind {
        CellKind::Plain => cell.text.clone(),
        CellKind::Missing => format!("missing|{}", cell.text),
        CellKind::Status(semantic) => {
            let semantic = format!("{semantic:?}").to_lowercase();
            format!("{semantic}|{}", cell.text)
        }
    }
}

/// Asserts one service's whole painted table: the headers it shows, then every
/// row it paints, cell by cell, against the strings that reach the screen.
///
/// The cells come off the session's own [`TableModel`] — the one
/// `project_resource_list` hands the renderer — rather than being re-derived
/// from the domain rows here. That is what this function exists for. A test
/// that walks the model and formats it the way it believes the table does is
/// asserting its own beliefs, and the projection can stop painting a column
/// outright without the walk noticing, because the walk never reads the column.
/// Not hypothetical: seven label helpers were mutated to return `""`, seven
/// columns across seven of these sixteen tables went blank, and the
/// model-derived version of this walk stayed green through all seven. It also
/// asserted three strings no cell has ever painted, and a dash in three columns
/// where the table paints nothing at all.
///
/// One comparison rather than a walk of named cells, and that is the other half
/// of the point. A walk asserts the rows it names and says nothing about the
/// rest, so a fixture that deserializes into blank rows passes it as long as
/// the blanks land on rows nobody wrote an `assert_eq!` for. Whole-vector
/// equality has no such rows: every row the table paints is compared, and every
/// cell of every row.
///
/// `headers` is asserted too and shares `N` with each expected row, so a column
/// added, dropped or reordered fails here by name — and a row of the wrong
/// width does not compile.
///
/// [`TableModel`]: nuvem::app::projection::table::TableModel
fn assert_table<const N: usize>(
    kind: ResourceServiceKind,
    session: &RuntimeSession,
    headers: [&str; N],
    expected: &[[&str; N]],
) {
    let projection = project_resource_list(&session.app().root.resources);
    assert_eq!(
        projection.service, kind,
        "the session was driven to the wrong service"
    );
    let table = projection
        .table
        .unwrap_or_else(|| panic!("{kind:?} paints no table in list mode"));
    assert_eq!(
        table
            .columns
            .iter()
            .map(|column| column.header)
            .collect::<Vec<_>>(),
        headers.as_slice(),
        "{kind:?} columns"
    );
    let actual = table
        .rows
        .iter()
        .map(|row| row.cells.iter().map(painted).collect::<Vec<_>>())
        .collect::<Vec<_>>();
    let expected = expected
        .iter()
        .map(|row| {
            row.iter()
                .map(|cell| (*cell).to_string())
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    assert_eq!(
        actual,
        expected,
        "{kind:?} rows, cell by cell: {}",
        headers.join(" / ")
    );
}

/// `--demo` must be able to reach every service the app can navigate to, and
/// every column those services paint must arrive carrying the fixture's value.
///
/// Two guards in one walk, because they fail together.
///
/// The first is the one this test has always made. `DemoCommandRunner` routes
/// fixtures by matching argv against a hand-written list of command shapes,
/// which no compiler relates to `ResourceServiceKind::ALL`: a service added
/// without a route falls through to the unknown-command error and its tab
/// renders a gcloud failure in a mode that is supposed to have no backend at
/// all. The match below is exhaustive — no `_` arm — so a new service has to
/// name its rows here before this file compiles. That exhaustiveness is the
/// whole reason this test exists, and it is kept.
///
/// The second is what the rows are compared against, and it is why every table
/// is asserted whole, as [`assert_table`] paints it.
///
/// `count > 0` is not a claim about a table an operator can read, and this is
/// not theoretical: wrapping an untouched Knative v1 document in a `services`
/// envelope produced five all-default Cloud Run rows, and `assert_eq!(rows
/// .len(), 5)` passed — it failed one line later, on the names being
/// `["", "", "", "", ""]`. The likelier shape is worse than that one, because
/// it is not visibly broken: proto3 JSON accepts a field under both its
/// lowerCamelCase name and its original snake_case proto name, so on a
/// half-renamed mapping some keys land for real and the rest are absorbed into
/// a private unknown-fields map with nothing raised — a row that is populated
/// enough to look right and wrong in the columns nobody asserted.
///
/// # Rows appear here in the order the table paints them
///
/// Which is not the order the fixtures list them in: each service's
/// `visible_*` accessor sorts, and every table here is under its default sort.
/// The expected vectors are therefore the screen's own order, and a sort
/// comparator that changed would fail here — correctly, since the order rows
/// appear in is something an operator reads.
///
/// # What this does and does not guard about the backend
///
/// What it pins is the *values* `--demo` renders, whichever backend produced
/// them. Reading them off a real `RuntimeSession` rather than off a
/// `Gcloud*Service` built over a bare runner is what makes them the app's own
/// rows — loaded by the effect the keys dispatch, through the session's own
/// routing, and projected by the app's own projection — rather than a second
/// path assembled by the test.
///
/// It is **not** a guard against a service that quietly keeps using the gcloud
/// arm after the migration to native clients, and must not be read as one.
/// `--demo` keeps `DemoCommandRunner` as the session's runner, and a `load_*`
/// with no client wired falls back to its `Gcloud*Service::new(runner)` arm —
/// so an unwired service answers from these same fixtures and produces rows
/// byte-identical to a wired one. Nothing observable here separates the two.
///
/// The guard against that is
/// `runtime_native_effect_test::a_demo_session_never_asks_gcloud_for_credentials`,
/// which walks the same services behind a recording runner and asserts the
/// complete set of argvs a demo walk spawns. A read that falls back to its
/// gcloud arm spawns an argv outside that set and fails there by name.
///
/// The client-cache macro is a weaker, separate guard, and must not be read as
/// this one: it proves a `from_fixtures()` *exists* for every cached service,
/// so a service with no stub fails to build. It says nothing about whether any
/// effect handler routes to that stub.
#[tokio::test(flavor = "multi_thread")]
async fn every_service_fills_its_painted_columns_through_a_demo_session() {
    for kind in ResourceServiceKind::ALL {
        let session = demo_session_showing(kind).await;

        match kind {
            // Row 2 is the instance with no public address, and its EXTERNAL-IP is
            // the dimmed dash of an absent value rather than a `-` some parser
            // chose to write.
            ResourceServiceKind::ComputeInstances => assert_table(
                kind,
                &session,
                [
                    "NAME",
                    "STATUS",
                    "ZONE",
                    "MACHINE-TYPE",
                    "INTERNAL-IP",
                    "EXTERNAL-IP",
                ],
                &[
                    [
                        "api-dev-1",
                        "healthy|Running",
                        "europe-west1-b",
                        "e2-medium",
                        "10.0.0.4",
                        "34.10.10.10",
                    ],
                    [
                        "worker-dev-1",
                        "neutral|Stopped",
                        "europe-west1-c",
                        "e2-small",
                        "10.0.0.5",
                        "missing|-",
                    ],
                ],
            ),

            // Row 1 is the unreadable cluster: a status this client does not model,
            // no master version, no node count, and an endpoint visibility the
            // parser could not decide. Two of its six cells are the dash of an
            // absent value; the third is the word `Unknown`, which the parser
            // picked and the table paints plain — an answer, not an absence.
            ResourceServiceKind::GkeClusters => assert_table(
                kind,
                &session,
                ["NAME", "STATUS", "LOCATION", "VERSION", "NODES", "ENDPOINT"],
                &[
                    [
                        "legacy-unknown",
                        "neutral|Unknown(\"MAINTENANCE_WINDOW\")",
                        "us-central1",
                        "missing|-",
                        "missing|-",
                        "Unknown",
                    ],
                    [
                        "platform-dev",
                        "healthy|Running",
                        "europe-west1",
                        "1.31.5-gke.1023000",
                        "6",
                        "Public",
                    ],
                    [
                        "private-prod",
                        "busy|Reconciling",
                        "europe-west1-b",
                        "1.30.9-gke.1127000",
                        "3",
                        "Private",
                    ],
                ],
            ),

            // Two cells here are the projection's own spelling and are readable
            // nowhere else: ENGINE joins the engine to the raw version in one cell,
            // and AVAILABILITY paints `HA (regional)` where the model carries
            // `Regional`.
            ResourceServiceKind::CloudSql => assert_table(
                kind,
                &session,
                [
                    "NAME",
                    "ENGINE",
                    "REGION",
                    "TIER",
                    "AVAILABILITY",
                    "CONNECTIVITY",
                    "STATUS",
                ],
                &[
                    [
                        "mysql-ha",
                        "MySQL (MYSQL_8_0)",
                        "us-east1",
                        "db-n1-standard-4",
                        "HA (regional)",
                        "Private",
                        "healthy|Runnable",
                    ],
                    [
                        "pg-replica",
                        "PostgreSQL (POSTGRES_15)",
                        "us-central1",
                        "db-custom-2-7680",
                        "Zonal",
                        "Public",
                        "healthy|Runnable",
                    ],
                    [
                        "pg-standalone",
                        "PostgreSQL (POSTGRES_15)",
                        "us-central1",
                        "db-custom-2-7680",
                        "Zonal",
                        "Public",
                        "healthy|Runnable",
                    ],
                    [
                        "pg-stopped",
                        "PostgreSQL (POSTGRES_14)",
                        "europe-west1",
                        "db-g1-small",
                        "Zonal",
                        "Public",
                        "neutral|Stopped",
                    ],
                    [
                        "sqlserver-maint",
                        "SQL Server (SQLSERVER_2019_STANDARD)",
                        "us-west1",
                        "db-custom-4-16384",
                        "HA (regional)",
                        "Public",
                        "warning|Maintenance",
                    ],
                ],
            ),

            // The table the `services` envelope silently emptied. `legacy-frontend`
            // is the row with nothing to show, and its three empty-looking cells
            // are two different things: two dashes for absent values, and a TRAFFIC
            // dash the parser itself produced, painted plain.
            ResourceServiceKind::CloudRunServices => assert_table(
                kind,
                &session,
                [
                    "NAME",
                    "STATUS",
                    "REGION",
                    "URL",
                    "LATEST-REVISION",
                    "TRAFFIC",
                ],
                &[
                    [
                        "billing-worker",
                        "warning|NotReady",
                        "us-central1",
                        "https://billing-worker-fghij-uc.a.run.app",
                        "billing-worker-00003-qwer",
                        "billing-worker-00003-qwer 100%",
                    ],
                    [
                        "checkout-api",
                        "degraded|Failed",
                        "europe-west1",
                        "https://checkout-api-klmno-ew.a.run.app",
                        "checkout-api-00011-aaaa",
                        "checkout-api-00011-aaaa 100%",
                    ],
                    [
                        "legacy-frontend",
                        "neutral|Unknown(\"Unknown\")",
                        "asia-northeast1",
                        "missing|-",
                        "missing|-",
                        "-",
                    ],
                    [
                        "orders-api",
                        "healthy|Ready",
                        "europe-west1",
                        "https://orders-api-abcde-ew.a.run.app",
                        "orders-api-00007-tabc",
                        "orders-api-00007-tabc 100%",
                    ],
                    [
                        "payments-api",
                        "busy|Reconciling",
                        "europe-west1",
                        "https://payments-api-pqrst-ew.a.run.app",
                        "payments-api-00019-old0",
                        "payments-api-00019-old0 80%",
                    ],
                ],
            ),

            // UPDATED is one of the three columns this walk corrected on its own.
            // Two of the three buckets report no update time, and
            // `storage_bucket_row` paints them `Cell::plain(String::new())` — an
            // EMPTY cell, where the model-derived expectation this replaces
            // asserted `-`. `public_access` on the same builder dashes the same
            // absence (`gamma-bucket`, last cell). The inconsistency is real and is
            // deliberately left alone: which of the two a bucket table should paint
            // is a rendering question, and this gate's job is to say what it paints
            // today.
            ResourceServiceKind::CloudStorage => assert_table(
                kind,
                &session,
                [
                    "NAME",
                    "LOCATION",
                    "CLASS",
                    "UPDATED",
                    "VERSIONING",
                    "PUBLIC-ACCESS",
                ],
                &[
                    [
                        "alpha-bucket",
                        "US",
                        "STANDARD",
                        "2024-02-03T04:05:06Z",
                        "on",
                        "enforced",
                    ],
                    [
                        "beta-bucket",
                        "EUROPE-WEST1",
                        "NEARLINE",
                        "",
                        "off",
                        "inherited",
                    ],
                    ["gamma-bucket", "ASIA", "COLDLINE", "", "off", "-"],
                ],
            ),

            // Row 2 is the user-managed policy, whose REPLICATION cell names the
            // regions — the whole difference between the two policies, and the only
            // thing separating it from the one word every other secret prints.
            ResourceServiceKind::SecretManagerSecrets => assert_table(
                kind,
                &session,
                ["NAME", "CREATED", "REPLICATION", "LABELS"],
                &[
                    [
                        "api-signing-key",
                        "2026-02-11T09:14:22Z",
                        "Automatic",
                        "env=prod, owner=platform",
                    ],
                    [
                        "db-password",
                        "2026-03-02T17:41:08Z",
                        "User-managed (europe-west1, europe-west4)",
                        "env=staging",
                    ],
                    ["unlabelled-token", "2026-05-20T06:00:00Z", "Automatic", "-"],
                ],
            ),

            // STATE and GENERATION are display labels rather than model words:
            // `2nd gen`, `Detach failed`, and — for the values the API added after
            // this client — the raw `SUSPENDED` and `GEN_3` painted verbatim, not
            // wrapped in the `Unrecognized(...)` spelling the model carries them
            // in. `bare-function` is the row that reported neither an update time
            // nor a URL.
            ResourceServiceKind::CloudFunctions => assert_table(
                kind,
                &session,
                ["NAME", "REGION", "STATE", "GENERATION", "UPDATED", "URL"],
                &[
                    [
                        "api-gateway",
                        "europe-west1",
                        "healthy|Active",
                        "2nd gen",
                        "2026-07-14T08:31:05Z",
                        "https://europe-west1-dev-project.cloudfunctions.net/api-gateway",
                    ],
                    [
                        "bare-function",
                        "us-west2",
                        "neutral|Unspecified",
                        "unspecified",
                        "missing|-",
                        "missing|-",
                    ],
                    [
                        "detached-worker",
                        "asia-northeast1",
                        "degraded|Detach failed",
                        "unspecified",
                        "2026-06-21T04:12:00Z",
                        "missing|-",
                    ],
                    [
                        "drifting-webhook",
                        "us-east1",
                        "neutral|Unknown",
                        "2nd gen",
                        "2026-05-09T22:47:31Z",
                        "missing|-",
                    ],
                    [
                        "image-resizer",
                        "europe-west4",
                        "busy|Deploying",
                        "2nd gen",
                        "2026-08-01T11:59:12Z",
                        "missing|-",
                    ],
                    [
                        "legacy-thumbnailer",
                        "us-central1",
                        "healthy|Active",
                        "1st gen",
                        "2025-11-02T16:04:44Z",
                        "https://us-central1-dev-project.cloudfunctions.net/legacy-thumbnailer",
                    ],
                    [
                        "quarantined-job",
                        "us-east1",
                        "neutral|SUSPENDED",
                        "GEN_3",
                        "2026-07-30T13:20:00Z",
                        "missing|-",
                    ],
                ],
            ),

            // Two more of the three corrected columns, both in this table. ORPHAN
            // paints `Cell::plain(String::new())` for the five rows that are not
            // orphans and ATTACHED paints the empty label for the four with no
            // users, so nine cells here are EMPTY where the model-derived
            // expectation asserted `-`. ORPHAN is also the column that shows
            // `badge()`'s old claim was wrong: it is a column of its own, keyed
            // `orphan` at priority 1, not a badge riding the identity cell.
            //
            // `detached-legacy` is the one true orphan — unattached *and* Ready —
            // and the pair with `being-built` is what separates waste from a disk
            // still being made. `regional-share` is regional, which is what keeps
            // LOCATION from meaning "zone". `bare-row` is the document that carried
            // no location, no type and no recognisable status: its STATUS cell is
            // empty text painted neutral, not the `Unrecognized("")` the model
            // holds.
            ResourceServiceKind::PersistentDisks => assert_table(
                kind,
                &session,
                [
                    "NAME", "LOCATION", "STATUS", "ORPHAN", "SIZE", "TYPE", "ATTACHED",
                ],
                &[
                    ["bare-row", "", "neutral|", "", "0 GB", "", ""],
                    [
                        "being-built",
                        "europe-west1-c",
                        "busy|Creating",
                        "",
                        "50 GB",
                        "pd-ssd",
                        "",
                    ],
                    [
                        "broken-disk",
                        "europe-west1-c",
                        "degraded|Failed",
                        "",
                        "9 GB",
                        "pd-standard",
                        "",
                    ],
                    [
                        "detached-legacy",
                        "europe-west1-b",
                        "healthy|Ready",
                        "warning|ORPHAN",
                        "20 GB",
                        "pd-ssd",
                        "",
                    ],
                    [
                        "gke-node-a",
                        "europe-west1-b",
                        "healthy|Ready",
                        "",
                        "100 GB",
                        "pd-balanced",
                        "gke-node-a",
                    ],
                    [
                        "regional-share",
                        "europe-west1",
                        "healthy|Ready",
                        "",
                        "200 GB",
                        "pd-balanced",
                        "db-primary, db-standby",
                    ],
                ],
            ),

            // SUBS paints the number alone — `0` for the topic with no subscribers
            // — where the model carries `Known(0)`; what it has to be told apart
            // from is the dash an unavailable count leaves. The two dashes beside
            // it differ in kind on purpose: KMS KEY is absent, LABELS is the
            // empty-list dash the projection writes itself.
            ResourceServiceKind::PubSubTopics => assert_table(
                kind,
                &session,
                ["NAME", "SUBS", "RETENTION", "KMS KEY", "LABELS"],
                &[
                    ["audit-log", "0", "missing|-", "missing|-", "-"],
                    ["back-orders", "1", "86400s", "missing|-", "-"],
                    [
                        "orders",
                        "2",
                        "604800s",
                        "projects/dev-project/locations/europe-west1/keyRings/nuvem/cryptoKeys/pubsub",
                        "env=prod, team=platform",
                    ],
                ],
            ),

            // The table that carried all three of the asserted strings no cell
            // paints. TOPIC paints `short_id(topic)` — `orders`, not
            // `projects/dev-project/topics/orders`. DEAD LETTER paints
            // `short_id(...)` too — `dead-letters`, with no `after 5`: the policy's
            // `max_delivery_attempts` reaches no list column at all, so the old
            // expectation named a number this table has never shown. DELIVERY
            // paints the word alone; a push subscription's endpoint is detail-view
            // content.
            //
            // `orphaned`'s topic is the sentinel the API returns for a subscription
            // whose topic was deleted — a value to print, not a blank.
            ResourceServiceKind::PubSubSubscriptions => assert_table(
                kind,
                &session,
                [
                    "NAME",
                    "TOPIC",
                    "DELIVERY",
                    "STATE",
                    "ACK(S)",
                    "RETENTION",
                    "DEAD LETTER",
                    "LABELS",
                ],
                &[
                    [
                        "back-orders-worker",
                        "back-orders",
                        "Pull",
                        "neutral|Unspecified",
                        "20",
                        "missing|-",
                        "missing|-",
                        "-",
                    ],
                    [
                        "orders-audit",
                        "orders",
                        "Push",
                        "degraded|Resource error",
                        "10",
                        "missing|-",
                        "dead-letters",
                        "-",
                    ],
                    [
                        "orders-worker",
                        "orders",
                        "Pull",
                        "healthy|Active",
                        "60",
                        "604800s",
                        "missing|-",
                        "team=platform",
                    ],
                    [
                        "orphaned",
                        "_deleted-topic_",
                        "Pull",
                        "healthy|Active",
                        "10",
                        "missing|-",
                        "missing|-",
                        "-",
                    ],
                ],
            ),

            // Three columns, four rows, and LABELS is the only cell that tells
            // them apart — DATASET aside, all four sit in US and all four are
            // tagged `dg_data_confidentiality=public`. Three carry exactly that
            // trio of `dg_` labels; the first carries those plus three
            // `dataplex-` ones, so it is the row that pins the flattening of a
            // label map wider than one line.
            ResourceServiceKind::BigQueryDatasets => assert_table(
                kind,
                &session,
                ["DATASET", "LOCATION", "LABELS"],
                &[
                    [
                        "agent_analytic_sample_dataset",
                        "US",
                        "dataplex-data-documentation-published-location=us-central1, dataplex-data-documentation-published-project=bigquery-public-data, dataplex-data-documentation-published-scan=crs-6979059d-0000-2d5a-b323-f4f5e802fd9c, dg_data_confidentiality=public, dg_data_context=metadata, dg_data_source=public",
                    ],
                    [
                        "america_health_rankings",
                        "US",
                        "dg_data_confidentiality=public, dg_data_context=metadata, dg_data_source=public",
                    ],
                    [
                        "austin_311",
                        "US",
                        "dg_data_confidentiality=public, dg_data_context=metadata, dg_data_source=public",
                    ],
                    [
                        "austin_bikeshare",
                        "US",
                        "dg_data_confidentiality=public, dg_data_context=metadata, dg_data_source=public",
                    ],
                ],
            ),

            // Row 1 is the account the listing carried with no email and no id at
            // all: two empty cells, which are values this table paints and not rows
            // it drops — and the empty EMAIL is why it sorts first. Rows 2 and 3
            // are the pair an empty display name turns on, since it dashes exactly
            // as an absent one does. STATE separates an identity that cannot
            // authenticate from one that can, and is not derivable from the policy
            // table.
            ResourceServiceKind::IamServiceAccounts => assert_table(
                kind,
                &session,
                ["EMAIL", "DISPLAY NAME", "STATE", "UNIQUE ID"],
                &[
                    ["", "missing|-", "Enabled", ""],
                    [
                        "empty-display-name@dev-project.iam.gserviceaccount.com",
                        "missing|-",
                        "Enabled",
                        "100000000000000000001",
                    ],
                    [
                        "no-display-name@dev-project.iam.gserviceaccount.com",
                        "missing|-",
                        "Enabled",
                        "117303179448968546820",
                    ],
                    [
                        "old-deployer@dev-project.iam.gserviceaccount.com",
                        "Retired deploy account",
                        "Disabled",
                        "104405669981102690742",
                    ],
                    [
                        "sa-vault@dev-project.iam.gserviceaccount.com",
                        "SA used by the vault node pool",
                        "Enabled",
                        "110727432291788759739",
                    ],
                ],
            ),

            // Twelve flattened bindings, one row per (role, member) pair, in four
            // columns: PUBLIC and DELETED have no columns of their own. Asserting
            // cells does not lose them — `iam_policy_member_cell` writes the badge
            // words into the MEMBER text and paints the cell `Warning`, so both
            // halves are in the assertion. The public ones are what an operator
            // opens this table for; the deleted ones are grants that outlived the
            // identity they were made to.
            //
            // `user:dave@example.com` is the binding whose role the document left
            // empty: the member still reaches the table, with nothing in the cell
            // that says what it can do.
            ResourceServiceKind::IamPolicy => assert_table(
                kind,
                &session,
                ["MEMBER", "ROLE", "TYPE", "CONDITION"],
                &[
                    [
                        "warning|PUBLIC allAuthenticatedUsers",
                        "roles/viewer",
                        "All authenticated users",
                        "missing|-",
                    ],
                    [
                        "warning|PUBLIC allUsers",
                        "roles/storage.objectViewer",
                        "All users",
                        "missing|-",
                    ],
                    [
                        "warning|DELETED deleted:serviceAccount:gone@dev-project.iam.gserviceaccount.com?uid=104405669981102690742",
                        "roles/editor",
                        "Service account",
                        "missing|-",
                    ],
                    [
                        "warning|DELETED deleted:user:bob@example.com?uid=9",
                        "roles/editor",
                        "User",
                        "missing|-",
                    ],
                    ["domain:example.com", "roles/viewer", "Domain", "missing|-"],
                    [
                        "group:platform@example.com",
                        "roles/artifactregistry.admin",
                        "Group",
                        "missing|-",
                    ],
                    [
                        "principal://iam.googleapis.com/locations/global/workforcePools/wf/subject/user:erin@example.com",
                        "projects/dev-project/roles/artifactregistry.admin",
                        "Principal",
                        "missing|-",
                    ],
                    [
                        "principalSet://iam.googleapis.com/projects/1/locations/global/workloadIdentityPools/gh/*",
                        "projects/dev-project/roles/artifactregistry.admin",
                        "Principal set",
                        "missing|-",
                    ],
                    [
                        "serviceAccount:sa-gke@dev-project.iam.gserviceaccount.com",
                        "roles/artifactregistry.admin",
                        "Service account",
                        "missing|-",
                    ],
                    [
                        "user:alice@example.com",
                        "roles/artifactregistry.admin",
                        "User",
                        "missing|-",
                    ],
                    [
                        "user:carol@example.com",
                        "roles/compute.admin",
                        "User",
                        "expires-2027",
                    ],
                    ["user:dave@example.com", "", "User", "missing|-"],
                ],
            ),

            // One row, and it fills all four cells. SUBNETS is read straight off
            // the network resource — `subnet_count: raw.subnetworks.len()` in
            // `src/services/resources/vpc.rs` — and `load_vpc_networks` makes no
            // second call, so 43 here is the length of this document's own
            // `subnetworks` array and not a count of the subnets table below.
            ResourceServiceKind::VpcNetworks => assert_table(
                kind,
                &session,
                ["NAME", "MODE", "ROUTING", "SUBNETS"],
                &[["default", "AUTO", "REGIONAL", "43"]],
            ),

            // Every row shares one name, which is why they are asserted together: a
            // subnet's identity is (name, region), and a row keyed on the name
            // alone would collapse them. The order on screen is that fact made
            // visible — `compare_subnets` breaks the name tie on region, so under
            // the default NAME sort these four come out region-ordered.
            //
            // `us-west1` is the fixture's constructed row and the only subnet with
            // PGA on. The captured project has the flag off on every one of its
            // subnets, and `false` is also what an absent flag parses to, so
            // without that row this column would be pinned at the value a mapping
            // that dropped the field entirely would produce anyway.
            ResourceServiceKind::VpcSubnets => assert_table(
                kind,
                &session,
                ["NAME", "REGION", "NETWORK", "RANGE", "GATEWAY", "PGA"],
                &[
                    [
                        "default",
                        "asia-east1",
                        "default",
                        "10.140.0.0/20",
                        "10.140.0.1",
                        "Off",
                    ],
                    [
                        "default",
                        "europe-west1",
                        "default",
                        "10.132.0.0/20",
                        "10.132.0.1",
                        "Off",
                    ],
                    [
                        "default",
                        "us-central1",
                        "default",
                        "10.128.0.0/20",
                        "10.128.0.1",
                        "Off",
                    ],
                    [
                        "default",
                        "us-west1",
                        "default",
                        "10.138.0.0/20",
                        "10.138.0.1",
                        "On",
                    ],
                ],
            ),

            // The security table, and the cells an operator reads to decide what
            // reaches a workload. TARGETS is the third of the asserted strings no
            // cell paints: it joins the selector values and deliberately loses
            // whether they are tags or service accounts, so `allow-web-to-tagged`
            // paints `web-server` and not `tags web-server`.
            //
            // DISABLED has no column, and the one disabled rule is pinned anyway:
            // `firewall_rule_row` leads the NAME cell with the badge word and the
            // cell is painted degraded, so the badge is in the string and its
            // styling is in the kind beside it. That row is also the only one whose
            // SOURCE/DEST is absent — it scopes by something that column does not
            // carry.
            ResourceServiceKind::FirewallRules => assert_table(
                kind,
                &session,
                [
                    "NAME",
                    "NETWORK",
                    "DIRECTION",
                    "PRIORITY",
                    "ACTION",
                    "TARGETS",
                    "SOURCE/DEST",
                ],
                &[
                    [
                        "allow-web-to-tagged",
                        "default",
                        "INGRESS",
                        "1000",
                        "ALLOW",
                        "web-server",
                        "10.0.0.0/8",
                    ],
                    [
                        "default-allow-icmp",
                        "default",
                        "INGRESS",
                        "65534",
                        "ALLOW",
                        "all instances",
                        "0.0.0.0/0",
                    ],
                    [
                        "default-allow-internal",
                        "default",
                        "INGRESS",
                        "65534",
                        "ALLOW",
                        "all instances",
                        "10.128.0.0/9",
                    ],
                    [
                        "default-allow-rdp",
                        "default",
                        "INGRESS",
                        "65534",
                        "ALLOW",
                        "all instances",
                        "0.0.0.0/0",
                    ],
                    [
                        "default-allow-ssh",
                        "default",
                        "INGRESS",
                        "65534",
                        "ALLOW",
                        "all instances",
                        "0.0.0.0/0",
                    ],
                    [
                        "deny-all-egress",
                        "default",
                        "EGRESS",
                        "65535",
                        "DENY",
                        "all instances",
                        "0.0.0.0/0",
                    ],
                    [
                        "degraded|DISABLED disabled-sa-scoped-ssh",
                        "default",
                        "INGRESS",
                        "900",
                        "ALLOW",
                        "workload@nuvem-fixture-project.iam.gserviceaccount.com",
                        "missing|-",
                    ],
                ],
            ),
        }
    }
}
#[test]
fn unknown_command_fails_with_demo_marker_in_stderr() {
    let runner = DemoCommandRunner::new();

    let error = runner
        .run(GcloudCommand::new(["totally", "unknown", "command"]))
        .expect_err("unknown command should fail");

    match error {
        CommandError::Failed { stderr, .. } => {
            assert!(
                stderr.contains("demo"),
                "expected stderr to mention demo mode, got: {stderr}"
            );
        }
        other => panic!("expected CommandError::Failed, got {other:?}"),
    }
}

#[test]
fn serves_diagnostics_probe_as_healthy_through_the_demo_runner() {
    let service = GcloudDiagnosticsService::new(DemoCommandRunner::new());
    let configuration = GcloudConfigurationName::new("dev");

    let report = service.probe(Some(&configuration));

    assert!(report.available);
    assert_eq!(report.active_account.as_deref(), Some("dev@example.com"));
    assert!(report.problem.is_none());
}

#[tokio::test]
async fn demo_session_loads_configurations_from_fixtures_end_to_end() {
    let cli = nuvem::Cli::parse_from(["nuvem", "--demo", "--config", "/nonexistent/nuvem.toml"]);
    let config = nuvem::Config::resolve(&cli).config;
    let app = nuvem::app::AppState::new();
    let state_path = nuvem::config::state::state_path_for(&config.config_path);
    let mut session =
        nuvem::services::runtime::session::RuntimeSession::new(config, cli, app, state_path);
    session.initialize().await; // dispatches LoadConfigurations synchronously
    let rows = session.app().root.configurations.rows();
    assert_eq!(rows.len(), 2);
    assert!(rows.iter().any(|row| row.is_active));
}

/// Every FinOps call the workspace makes, through the real service layer.
///
/// Demo mode used to route none of them, so the workspace the README
/// sells demo mode for — "UI iteration, demo recordings, docs screenshots" — was
/// the one that could not be demoed: four "not probed" cards and a Budgets card
/// reading "unavailable: unknown error", which is what an unrouted argv becomes
/// once `no_fixture` is classified.
#[test]
fn finops_recommendations_and_budgets_through_demo_runner() {
    let service = GcloudFinOpsService::new(DemoCommandRunner::new());

    // One recommendation per recommender, at the location `query_locations`
    // derives for it from the demo compute and disk fixtures.
    for (recommender, location, saving) in [
        (RecommenderId::IdleVm, "europe-west1-b", "24.50"),
        (RecommenderId::MachineType, "europe-west1-b", "8.25"),
        (RecommenderId::IdleDisk, "europe-west1-b", "3.40"),
        (RecommenderId::UnusedAddress, "europe-west1", "7.30"),
    ] {
        let rows = service
            .load_recommendations(&context(), recommender, location)
            .unwrap_or_else(|error| panic!("demo {recommender:?} recommendations: {error}"));
        assert_eq!(
            rows.len(),
            1,
            "{recommender:?} at {location} should list exactly one recommendation"
        );
        let monthly = rows[0]
            .monthly
            .as_ref()
            .unwrap_or_else(|| panic!("{recommender:?} has no monthly saving"));
        // The API states a saving as a negative cost over the projection's own
        // duration; both corrections have to have happened for this to read as
        // a positive monthly figure.
        assert_eq!(
            monthly.render(),
            format!("USD {saving}"),
            "{recommender:?} saving did not survive sign correction and scaling"
        );
    }

    // Any other location answers an empty list rather than the same rows again.
    // A fixture served per location would list one recommendation once per zone
    // and the savings total would count it that many times.
    let elsewhere = service
        .load_recommendations(&context(), RecommenderId::IdleVm, "europe-west1-c")
        .expect("demo recommendations elsewhere");
    assert!(
        elsewhere.is_empty(),
        "a location with nothing to save must answer an empty list, got {elsewhere:?}"
    );

    // Budgets take two calls: which billing account the project is billed to,
    // then that account's budgets. Both have to be routed for either to work.
    let budgets = service.load_budgets(&context()).expect("demo budgets");
    assert_eq!(budgets.len(), 2);
    assert!(
        !budgets[0].covers_whole_account,
        "the first budget filters on this project and must say so"
    );
    assert!(
        budgets[1].covers_whole_account && budgets[1].matches_last_period,
        "the second budget is account-wide and tracks the last period"
    );
}
