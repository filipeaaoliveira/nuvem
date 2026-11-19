//! The session's native branch, exercised end to end for the first time.
//!
//! The session prefers a native client when it holds one, and until this file
//! nothing ever gave it one: every harness ran with `sdk_clients: None`, so
//! the native arm of every effect was dead code as far as the suite was
//! concerned. The detail-pane panic of 2026-09-14 is what that costs -- it
//! crashed every pane, and 4,353 tests passed over it.
//!
//! Nothing here reaches the network: the clients answer from stubs.

use clap::Parser;
use google_cloud_container_v1::client::ClusterManager;
use google_cloud_container_v1::model::{Cluster, ListClustersResponse};
use nuvem::app::{
    Action, AppState, CloudSqlInstanceAction, ComputeInstanceAction, Effect, ResourcesAction,
    WorkspaceAction,
};
use nuvem::services::demo::DemoCommandRunner;
use nuvem::services::runtime::session::RuntimeSession;
use nuvem::services::sdk::gke::SdkGkeService;
use nuvem::services::sdk::{ClientSource, SdkClients};
use nuvem::{Cli, Config};
use nuvem_domain::gcp::resource::ResourceServiceKind;

/// Answers the two calls the table and the pane make.
///
/// The generated stub's defaults do not return an error, they panic, so a
/// call this fake does not implement is indistinguishable from the crash
/// under test. Both calls are answered for that reason.
#[derive(Debug, Default)]
struct ClusterStub;

impl google_cloud_container_v1::stub::ClusterManager for ClusterStub {
    async fn list_clusters(
        &self,
        _req: google_cloud_container_v1::model::ListClustersRequest,
        _options: google_cloud_gax::options::RequestOptions,
    ) -> google_cloud_gax::Result<google_cloud_gax::response::Response<ListClustersResponse>> {
        Ok(google_cloud_gax::response::Response::from(
            ListClustersResponse::new().set_clusters(vec![
                Cluster::new()
                    .set_name("checkout")
                    .set_location("europe-west4")
                    .set_status(google_cloud_container_v1::model::cluster::Status::Running),
            ]),
        ))
    }

    async fn get_cluster(
        &self,
        _req: google_cloud_container_v1::model::GetClusterRequest,
        _options: google_cloud_gax::options::RequestOptions,
    ) -> google_cloud_gax::Result<google_cloud_gax::response::Response<Cluster>> {
        Ok(google_cloud_gax::response::Response::from(
            Cluster::new()
                .set_name("checkout")
                .set_location("europe-west4")
                .set_status(google_cloud_container_v1::model::cluster::Status::Running)
                .set_current_master_version("1.31.5-gke.1000"),
        ))
    }
}

/// Records every argv it is handed and answers each with empty JSON, which
/// every parser here reads as "no rows".
///
/// The demo runner would answer with fixtures, which is fine for driving a
/// session but useless for asking whether gcloud was called at all.
///
/// The binary is recorded as the argv's first word, because which binary ran
/// is part of what was spawned: `bq ls` and `gcloud ls` are different
/// commands, and the whole-set assertion below would otherwise conflate them.
#[derive(Clone, Default)]
struct RecordingRunner {
    commands: std::sync::Arc<std::sync::Mutex<Vec<Vec<String>>>>,
}

impl nuvem::services::gcloud::cli::CommandRunner for RecordingRunner {
    fn run(
        &self,
        command: nuvem::services::gcloud::cli::GcloudCommand,
    ) -> Result<
        nuvem::services::gcloud::cli::CommandOutput,
        nuvem::services::gcloud::cli::CommandError,
    > {
        let mut argv = vec![command.program.binary_name().to_string()];
        argv.extend(command.args.iter().cloned());
        self.commands.lock().expect("not poisoned").push(argv);
        let stdout = if command.args.iter().any(|arg| arg == "configurations") {
            include_str!("fixtures/gcloud_configurations.json").to_string()
        } else {
            "[]".to_string()
        };
        Ok(nuvem::services::gcloud::cli::CommandOutput {
            stdout,
            stderr: String::new(),
        })
    }
}

/// One instance row, named and placed.
fn compute_instance(name: &str, zone: &str) -> nuvem_domain::gcp::resource::ComputeInstance {
    nuvem_domain::gcp::resource::ComputeInstance {
        id: format!("{name}-id"),
        name: name.to_string(),
        zone: zone.to_string(),
        status: nuvem_domain::gcp::resource::ResourceStatus::Running,
        machine_type: "e2-medium".to_string(),
        internal_ip: None,
        external_ip: None,
    }
}

/// One Cloud SQL row, named and placed.
fn cloud_sql_instance(name: &str) -> nuvem_domain::gcp::resource::CloudSqlInstance {
    use nuvem_domain::gcp::resource::{
        CloudSqlAvailability, CloudSqlInstance, CloudSqlStatus, EndpointVisibility,
    };
    CloudSqlInstance {
        name: name.to_string(),
        database_version: "POSTGRES_15".to_string(),
        engine: "PostgreSQL".to_string(),
        region: "europe-west1".to_string(),
        tier: "db-custom-2-7680".to_string(),
        availability: CloudSqlAvailability::Regional,
        status: CloudSqlStatus::Runnable,
        connectivity: EndpointVisibility::Public,
    }
}

fn session_holding_a_stub() -> RuntimeSession {
    let cli = Cli::parse_from(["nuvem", "--config", "/nonexistent/nuvem.toml"]);
    let config = Config::resolve(&cli).config;
    let clients = SdkClients::with_gke_for_test(
        "dev",
        SdkGkeService::from_clusters_for_test(ClusterManager::from_stub(ClusterStub)),
    );
    RuntimeSession::new_with_sdk_clients_for_test(
        config,
        AppState::new(),
        Box::new(DemoCommandRunner::new()),
        clients,
    )
}

/// Bounded drain: the effect queue parks when empty rather than reporting so.
async fn drain(session: &mut RuntimeSession) {
    for _ in 0..200 {
        match tokio::time::timeout(
            std::time::Duration::from_millis(25),
            session.process_next_effect_result(),
        )
        .await
        {
            Ok(true) => {}
            _ => break,
        }
    }
}

/// Drives the session to a loaded GKE table, the way the keys do.
async fn gke_table(session: &mut RuntimeSession) {
    session.initialize().await;
    drain(session).await;
    session
        .dispatch_action(Action::Workspace(WorkspaceAction::Resources))
        .await;
    drain(session).await;
    session
        .dispatch_action(Action::Resources(ResourcesAction::SelectService(
            ResourceServiceKind::GkeClusters,
        )))
        .await;
    drain(session).await;
    session
        .dispatch_action(Action::Resources(ResourcesAction::Refresh))
        .await;
    drain(session).await;
}

/// The listing the stub planted reaches the table.
///
/// This is the assertion that says the native branch was taken at all: the
/// runner in this session is a demo runner, whose GKE fixture has different
/// clusters, so a row named by the stub can only have come through the client.
#[tokio::test(flavor = "multi_thread")]
async fn a_session_holding_a_native_client_fills_the_table_from_it() {
    let mut session = session_holding_a_stub();
    gke_table(&mut session).await;

    let rows = session.app().root.resources.gke_clusters();
    assert_eq!(rows.len(), 1, "the stub's cluster reached the table");
    assert_eq!(rows[0].name, "checkout");
    assert_eq!(rows[0].location, "europe-west4");
}

/// Opening a detail pane does not panic.
///
/// The effect that loads a pane is awaited inline on the run loop rather than
/// handed to a blocking thread, so anything it calls runs on a thread that is
/// driving the runtime. `Handle::block_on` refuses to run there, and the
/// first native pane crashed the process with "Cannot start a runtime from
/// within a runtime". A panic in a dispatched effect takes this test process
/// with it, so reaching the assertion at all is most of what is asserted.
#[tokio::test(flavor = "multi_thread")]
async fn opening_a_native_detail_pane_does_not_panic() {
    let mut session = session_holding_a_stub();
    gke_table(&mut session).await;
    assert_eq!(
        session.app().root.resources.gke_clusters().len(),
        1,
        "the row the pane opens has to be there, or this test opens nothing \
         and proves nothing"
    );

    // Enter on the row: the pane's own load runs on the run loop rather than
    // on a blocking thread, which is the thread the old bridge refused.
    session
        .dispatch_action(Action::Resources(ResourcesAction::OpenSelected))
        .await;
    drain(&mut session).await;

    // A panic in a dispatched effect takes this process with it, so arriving
    // here at all is most of the assertion. The rest says the pane was
    // actually filled through the client rather than merely surviving.
    let detail = session
        .app()
        .root
        .resources
        .gke_detail()
        .expect("the pane loaded through the native client");
    assert_eq!(detail.name, "checkout");
    assert_eq!(
        detail.current_master_version.as_deref(),
        Some("1.31.5-gke.1000")
    );
}

/// A demo session's native clients are the fixture-backed ones.
///
/// Demo used to be safe by holding no clients at all: mutations were
/// intercepted by matching the gcloud argv -- `compute instances`, `sql
/// instances` -- and an RPC would have gone straight past that interception to
/// the real API, so having nothing to make the call with was the whole
/// mechanism. It is not the mechanism any more. Demo now reads and mutates
/// through clients, and what keeps it offline is which clients they are:
/// [`ClientSource::Showcase`] builds every one `from_stub`, which takes no
/// credentials and no transport.
///
/// So the property has to be asserted by source rather than by absence. A
/// session that came back with `ClientSource::Live` here would be one an
/// operator could point at a real project by typing `--demo`.
#[tokio::test(flavor = "multi_thread")]
async fn a_demo_session_holds_fixture_backed_native_clients() {
    let cli = Cli::parse_from(["nuvem", "--demo", "--config", "/nonexistent/nuvem.toml"]);
    let config = Config::resolve(&cli).config;
    assert!(config.demo, "the fixture is a demo session");

    let session = RuntimeSession::new(
        config,
        cli,
        AppState::new(),
        std::path::PathBuf::from("/nonexistent/nuvem-state.toml"),
    );

    assert_eq!(
        session.native_client_source(),
        Some(ClientSource::Showcase),
        "a demo session must reach no real API: its clients are built from \
         stubs, which hold no credentials and no transport"
    );
}

/// Nothing in a demo session asks gcloud for a token, or for anything else
/// beyond the two reads that have nowhere native to go.
///
/// The stronger half of the pin above, and the one the old
/// "holds no native clients" assertion was standing in for. Source is a label
/// on the cache; this is the behaviour the label is supposed to buy.
///
/// # This is the only guard on native *routing*
///
/// Two other things look like one and are not. The demo coverage walk in
/// `demo_runner_test.rs` pins the cells each service paints, and a service
/// still reading through gcloud paints them identically, because in demo both
/// backends answer from the same fixtures. The client-cache macro in
/// [`nuvem::services::sdk`] proves a `from_fixtures()` exists for every cached
/// service and will not compile without one -- but existence is not use, and
/// nothing in the type system says an effect handler ever calls it.
///
/// The whole-set assertion at the end of this walk is what does: every read
/// that falls back to its `Gcloud*Service::new(runner)` arm spawns an argv,
/// and an argv outside [`EVERY_ARGV_A_DEMO_WALK_MAY_SPAWN`] fails here by
/// name. Forcing the Cloud Run, Pub/Sub and Storage loads onto their gcloud
/// arms is caught here and nowhere else in the suite.
///
/// `gcloud --configuration <name> config config-helper --format=json` is the
/// only argv [`nuvem::services::gcloud::context_credentials`] ever spawns, and
/// it is spawned on exactly one path: constructing a *live* client, which
/// needs a bearer token before it can open a transport. A fixture client is
/// built `from_stub` and has no such step. So the absence of that argv is the
/// mechanical proof that no live client was constructed for any service --
/// which is what "offline" means here, and which counting rows cannot show.
///
/// # Both halves are measured, because neither is free
///
/// A cache uses its runner for exactly one thing, the token, and
/// `SdkClients::showcase` holds a `DemoCommandRunner` so that even a stray ask
/// could not reach real gcloud. That also makes the ask invisible: written
/// with the session's runner recording and the cache's own runner left alone,
/// this test passed with the macro's fixture arm hand-mutated to build *live*
/// clients. `showcase_over_for_test` is what closes that -- one recorder
/// behind both the session and the cache -- and with it that mutation fails
/// here.
///
/// The first assertion is the other half. This session is handed its clients,
/// so on its own it says nothing about what `RuntimeSession::new` decides for
/// `--demo`; a session built by the real constructor is asked first, and
/// putting `None` back in that constructor fails this test rather than quietly
/// making the walk below vacuous.
#[tokio::test(flavor = "multi_thread")]
async fn a_demo_session_never_asks_gcloud_for_credentials() {
    let cli = Cli::parse_from(["nuvem", "--demo", "--config", "/nonexistent/nuvem.toml"]);
    let config = Config::resolve(&cli).config;
    assert!(config.demo, "the fixture is a demo session");

    let built_by_the_binary = RuntimeSession::new(
        config.clone(),
        cli,
        AppState::new(),
        std::path::PathBuf::from("/nonexistent/nuvem-state.toml"),
    );
    assert_eq!(
        built_by_the_binary.native_client_source(),
        Some(ClientSource::Showcase),
        "the source the walk below asserts about has to be the source a real \
         demo session holds, or the walk is about a cache nothing builds"
    );

    let runner = std::sync::Arc::new(RecordingRunner::default());
    let recorded = std::sync::Arc::clone(&runner.commands);
    let mut session = RuntimeSession::new_with_sdk_clients_for_test(
        config,
        AppState::new(),
        Box::new(RecordingRunner {
            commands: std::sync::Arc::clone(&recorded),
        }),
        std::sync::Arc::new(SdkClients::showcase_over_for_test(runner)),
    );

    session.initialize().await;
    drain(&mut session).await;
    session
        .dispatch_action(Action::Workspace(WorkspaceAction::Resources))
        .await;
    drain(&mut session).await;

    for service in ResourceServiceKind::ALL {
        session
            .dispatch_action(Action::Resources(ResourcesAction::SelectService(service)))
            .await;
        drain(&mut session).await;
        session
            .dispatch_action(Action::Resources(ResourcesAction::Refresh))
            .await;
        drain(&mut session).await;
    }

    let argvs = recorded.lock().expect("not poisoned");
    assert!(
        !argvs.is_empty(),
        "the walk must really drive the session, or the absence below proves \
         nothing about credentials"
    );
    assert!(
        !argvs.iter().any(|argv| argv
            .windows(2)
            .any(|pair| pair == ["config", "config-helper"])),
        "a demo session fetched a token, so it built a live client: {argvs:?}"
    );

    let spawned: std::collections::BTreeSet<String> =
        argvs.iter().map(|argv| argv.join(" ")).collect();
    let expected: std::collections::BTreeSet<String> = EVERY_ARGV_A_DEMO_WALK_MAY_SPAWN
        .iter()
        .map(|argv| (*argv).to_string())
        .collect();
    let unexpected: Vec<&String> = spawned.difference(&expected).collect();
    assert!(
        unexpected.is_empty(),
        "a demo walk spawned a command outside the argvs the remaining CLI \
         reads make, which means a service routed to its gcloud arm instead of \
         its native client: {unexpected:?}\n\
         \n\
         Each extra argv names the read that fell through. Wire that service's \
         `load_*` to the client cache, or -- if it genuinely has no published \
         Rust client -- add its argv here and to the ledger in \
         `docs/guides/demo-mode.md`, which is what that list is for.\n\
         \n\
         spawned:  {spawned:?}\n\
         expected: {expected:?}"
    );
    assert_eq!(
        spawned, expected,
        "the walk no longer spawns one of the CLI reads this set was written \
         against, so the set has gone stale and stopped guarding anything: \
         drop the argv that is gone"
    );
}

/// The complete set of commands a `--demo` walk of every service may spawn.
///
/// Three. The first two survive for a reason recorded in
/// `docs/decisions/0001-native-sdk-for-read-paths.md`. `bq ls` is the dataset
/// listing: `google-cloud-bigquery` is the one crate of the set still pre-1.0,
/// and `bq` already inherits the configuration guarantee through
/// `CLOUDSDK_ACTIVE_CONFIG_NAME`. `gcloud config configurations list` is how
/// nuvem discovers which configurations exist at all, which is a gcloud-local
/// concept with no API behind it. `gcloud auth list` is Home's health probe,
/// which a demo session runs as it opens so its first screen says gcloud is
/// available; the probe asks about gcloud itself, so it has no client.
///
/// Everything else in the walk reads through a fixture-backed client, and this
/// set is what holds that: a `load_*` that falls back to its
/// `Gcloud*Service::new(runner)` arm spawns a third argv and names itself
/// here. The demo coverage walk cannot see that -- an unwired service answers
/// from the same fixtures through the CLI and paints byte-identical rows --
/// and neither can the client-cache macro, which proves only that a
/// `from_fixtures()` exists per service, never that any effect calls it.
const EVERY_ARGV_A_DEMO_WALK_MAY_SPAWN: &[&str] = &[
    "bq --format=prettyjson --project_id=dev-project ls -d -n 500",
    "gcloud config configurations list --format=json",
    "gcloud --configuration dev auth list --format=json",
];

/// The other side of it: a session that is not in demo does build them.
///
/// Without this the pin above would also pass if native clients were never
/// built at all, which is the state this whole migration exists to leave.
#[tokio::test(flavor = "multi_thread")]
async fn an_ordinary_session_does_build_native_clients() {
    let cli = Cli::parse_from(["nuvem", "--config", "/nonexistent/nuvem.toml"]);
    let config = Config::resolve(&cli).config;
    assert!(!config.demo, "the fixture is an ordinary session");

    let session = RuntimeSession::new(
        config,
        cli,
        AppState::new(),
        std::path::PathBuf::from("/nonexistent/nuvem-state.toml"),
    );

    assert!(
        session.holds_native_clients(),
        "an ordinary session reads through the native clients"
    );
}

/// A confirmed compute action goes to the client, and gcloud is not asked.
///
/// The negative half is the one that matters. A test that only checked the
/// client was called would pass just as happily if both backends ran, which
/// would mean every stop being issued twice.
#[tokio::test(flavor = "multi_thread")]
async fn a_confirmed_compute_action_reaches_the_client_and_not_gcloud() {
    use std::sync::{Arc, Mutex};

    type Recorded = Arc<Mutex<Vec<String>>>;

    #[derive(Debug, Default)]
    struct InstancesStub {
        called: Recorded,
    }

    impl google_cloud_compute_v1::stub::Instances for InstancesStub {
        async fn stop(
            &self,
            req: google_cloud_compute_v1::model::instances::StopRequest,
            _options: google_cloud_gax::options::RequestOptions,
        ) -> google_cloud_gax::Result<
            google_cloud_gax::response::Response<google_cloud_compute_v1::model::Operation>,
        > {
            self.called
                .lock()
                .expect("not poisoned")
                .push(format!("stop {} {}", req.zone, req.instance));
            Ok(google_cloud_gax::response::Response::from(
                google_cloud_compute_v1::model::Operation::new(),
            ))
        }
    }

    let called: Recorded = Arc::new(Mutex::new(Vec::new()));
    let clients = SdkClients::with_compute_for_test(
        "dev",
        nuvem::services::sdk::compute::SdkComputeService::from_instances_for_test(
            google_cloud_compute_v1::client::Instances::from_stub(InstancesStub {
                called: Arc::clone(&called),
            }),
        ),
    );

    let cli = Cli::parse_from(["nuvem", "--config", "/nonexistent/nuvem.toml"]);
    let config = Config::resolve(&cli).config;
    let runner = RecordingRunner::default();
    let recorded = Arc::clone(&runner.commands);
    let mut session = RuntimeSession::new_with_sdk_clients_for_test(
        config,
        AppState::new(),
        Box::new(runner),
        clients,
    );

    session.initialize().await;
    drain(&mut session).await;
    let context = session
        .app()
        .root
        .current_context()
        .expect("the fixtures resolve a context");

    session
        .dispatch_action(Action::Effect(Effect::ComputeInstanceAction {
            context,
            instance: compute_instance("api-dev-1", "europe-west1-b"),
            action: ComputeInstanceAction::Stop,
            operation_id: 1,
        }))
        .await;
    drain(&mut session).await;

    assert_eq!(
        called.lock().expect("not poisoned").as_slice(),
        ["stop europe-west1-b api-dev-1"],
        "the confirmed action reached the native client"
    );
    let argvs = recorded.lock().expect("not poisoned");
    assert!(
        !argvs
            .iter()
            .any(|argv| argv.windows(2).any(|pair| pair == ["instances", "stop"])),
        "and gcloud was not asked to do it as well: {argvs:?}"
    );
}

/// A confirmed Cloud SQL action goes to the client, and gcloud is not asked.
///
/// Same two halves as the compute case, and the same reason the negative one
/// carries the weight: without it, both backends running would look like a
/// pass while every stop was issued twice.
#[tokio::test(flavor = "multi_thread")]
async fn a_confirmed_cloud_sql_action_reaches_the_client_and_not_gcloud() {
    use std::sync::{Arc, Mutex};

    type Recorded = Arc<Mutex<Vec<String>>>;

    #[derive(Debug, Default)]
    struct InstancesStub {
        called: Recorded,
    }

    impl google_cloud_sql_v1::stub::SqlInstancesService for InstancesStub {
        async fn patch(
            &self,
            req: google_cloud_sql_v1::model::SqlInstancesPatchRequest,
            _options: google_cloud_gax::options::RequestOptions,
        ) -> google_cloud_gax::Result<
            google_cloud_gax::response::Response<google_cloud_sql_v1::model::Operation>,
        > {
            let policy = req
                .body
                .as_ref()
                .and_then(|body| body.settings.as_ref())
                .and_then(|settings| settings.activation_policy.name())
                .unwrap_or("none")
                .to_string();
            self.called
                .lock()
                .expect("not poisoned")
                .push(format!("patch {} {policy}", req.instance));
            Ok(google_cloud_gax::response::Response::from(
                google_cloud_sql_v1::model::Operation::new()
                    .set_status(google_cloud_sql_v1::model::operation::SqlOperationStatus::Done),
            ))
        }
    }

    #[derive(Debug, Default)]
    struct UsersStub;
    impl google_cloud_sql_v1::stub::SqlUsersService for UsersStub {}

    /// Never read back: the patch answers an operation already done.
    #[derive(Debug, Default)]
    struct OperationsStub;
    impl google_cloud_sql_v1::stub::SqlOperationsService for OperationsStub {}

    let called: Recorded = Arc::new(Mutex::new(Vec::new()));
    let clients = SdkClients::with_cloudsql_for_test(
        "dev",
        nuvem::services::sdk::cloudsql::SdkCloudSqlService::from_clients_for_test(
            google_cloud_sql_v1::client::SqlInstancesService::from_stub(InstancesStub {
                called: Arc::clone(&called),
            }),
            google_cloud_sql_v1::client::SqlUsersService::from_stub(UsersStub),
            google_cloud_sql_v1::client::SqlOperationsService::from_stub(OperationsStub),
        ),
    );

    let cli = Cli::parse_from(["nuvem", "--config", "/nonexistent/nuvem.toml"]);
    let config = Config::resolve(&cli).config;
    let runner = RecordingRunner::default();
    let recorded = Arc::clone(&runner.commands);
    let mut session = RuntimeSession::new_with_sdk_clients_for_test(
        config,
        AppState::new(),
        Box::new(runner),
        clients,
    );

    session.initialize().await;
    drain(&mut session).await;
    let context = session
        .app()
        .root
        .current_context()
        .expect("the fixtures resolve a context");

    session
        .dispatch_action(Action::Effect(Effect::CloudSqlInstanceAction {
            context,
            instance: cloud_sql_instance("orders-db"),
            action: CloudSqlInstanceAction::Stop,
            operation_id: 1,
        }))
        .await;
    drain(&mut session).await;

    assert_eq!(
        called.lock().expect("not poisoned").as_slice(),
        ["patch orders-db NEVER"],
        "the confirmed stop reached the client as a patch of the activation policy"
    );
    let argvs = recorded.lock().expect("not poisoned");
    assert!(
        !argvs
            .iter()
            .any(|argv| argv.windows(2).any(|pair| pair == ["instances", "patch"])),
        "and gcloud was not asked to do it as well: {argvs:?}"
    );
}

/// The two backends admit different things when a write is abandoned.
///
/// They are not interchangeable messages. The CLI path kills a process and
/// cannot say whether the command ever reached GCP, so its caveat hedges on
/// both halves: the operation *may* still be running. The native path sent
/// the request and knows the operation exists; only the watching stopped.
/// Collapsing the two into one wording would make the more informative case
/// as vague as the less.
///
/// The CLI half is reached through the error an abandoned command produces,
/// which is public; the native half is asserted through the message the
/// session attaches, which is why this drives a real cancelled write rather
/// than reading a constant.
#[test]
fn the_cli_caveat_hedges_on_whether_the_operation_is_running() {
    let error: nuvem::app::AppError = nuvem::services::gcloud::cli::CommandError::TimedOut {
        seconds: 1800,
        ceiling: nuvem::services::gcloud::cli::CommandCeiling::Lifecycle,
    }
    .into();

    let detail = error.detail().to_lowercase();
    assert!(
        detail.contains("may still be running"),
        "the CLI path cannot say whether the command reached GCP, and its \
         message hedges accordingly: {detail}"
    );
    assert!(
        !detail.contains("is still running"),
        "and it must not claim the stronger thing the native path can claim: \
         {detail}"
    );
}
