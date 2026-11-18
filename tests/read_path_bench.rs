//! Per-call wall time for every read path nuvem has, measured on both
//! backends against the live project the active gcloud configuration names.
//!
//! `docs/decisions/0001-native-sdk-for-read-paths.md` moved every listing and
//! every detail load off the `gcloud` subprocess and onto the native client
//! libraries, on the strength of two captures: `gcloud storage buckets list
//! --format=json` at 1.32s – 1.56s against the same data over HTTPS with a
//! cached token at 0.31s – 0.47s. That record measured one path on one
//! machine. This example measures all of them, on the machine in front of you,
//! through the application's own code rather than through a hand-built call —
//! so the numbers it prints are directly comparable to the ones in the record,
//! and a reader can check whether the decision still holds here.
//!
//! What is measured, for every path, `READ_PATH_BENCH_ITERATIONS` times
//! (default 5):
//!
//! - **native** — through [`nuvem::services::sdk::SdkClients`], the same cache
//!   the session holds, authenticated by the same `config-helper` bridge.
//! - **cli** — through the matching `Gcloud*Service` over a
//!   [`ProcessCommandRunner`], the implementations `--demo` still runs on.
//!
//! Each iteration is a whole call: request, response, and deserialisation into
//! the domain row type. The row count is reported beside the timings, because a
//! backend that returns fewer rows is not measuring the same work.
//!
//! Three costs are measured on their own, because they are not per-path:
//! client construction on a cold cache, the first request made through that
//! freshly built client, and `gcloud --version` — the interpreter floor every
//! CLI call pays before any data moves. The `gcloud config config-helper`
//! subprocess that mints the token falls inside one of the first two: the
//! credential provider spawns it from `headers()`, which the client libraries
//! call when a request is made rather than when a client is built. Those two
//! are printed as separate numbers so a reader can see which one carries it on
//! this machine rather than take this file's word for it.
//!
//! # Fairness
//!
//! - Both backends are aimed at the same configuration, the same project and
//!   the same parameters. Where a path takes a page size, a window, a limit, a
//!   recommender or a location, both are handed the same one.
//! - Both are warmed by one untimed call before any timing starts, so neither
//!   pays a connection setup or a page cache miss inside a sample.
//! - Each native service holds its own token cache, so the first request
//!   through each one spawns its own `config-helper`. That request is the
//!   untimed warm call, which is why no timed native sample contains a
//!   subprocess. Drop the warm call and the native column stops measuring what
//!   the poll repeats and starts measuring thirteen interpreter starts.
//! - Iterations interleave — native, cli, native, cli — rather than running all
//!   of one backend and then all of the other, so a machine that gets busier
//!   partway through slows both equally.
//! - A failure is reported per path and per backend and never ends the run. The
//!   other backend keeps going, and so does every later path.
//! - A detail pane is loaded for the first row its own listing returned, and is
//!   skipped when that listing came back empty.
//!
//! # What this prints
//!
//! Numbers and path names. No project id, bucket name, account, instance name,
//! configuration name, region, zone, or any other resource identifier is ever
//! written to stdout — not in the table, and not in an error. Error text is
//! masked against every identifier this run touched, passed through the
//! application's own credential redaction, and then cut to 60 characters. The
//! output is safe to paste into an issue.
//!
//! # Running it
//!
//! ```text
//! READ_PATH_BENCH_ITERATIONS=5 cargo nextest run --release \
//!     -E 'test(read_path_bench)' --run-ignored all --no-capture
//! ```
//!
//! It lives under `tests/` rather than in `examples/` or `benches/` for the
//! reason `render_bench.rs` states: mounted in the one suite binary, it is
//! built, linted and formatted with the rest of the tests. It is `#[ignore]`d
//! and asserts nothing: it reaches the network and needs a live project, so it
//! runs when someone asks for it and never as a gate.
//!
//! It reads. It creates nothing, changes nothing, and deletes nothing, and the
//! one Secret Manager call it makes is `secrets.list` — never `access`.

use std::cell::RefCell;
use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;
use std::time::Instant;

use nuvem::services::gcloud::cli::{CommandRunner, GcloudCommand, ProcessCommandRunner};
use nuvem::services::gcloud::configurations::GcloudConfigurationService;
use nuvem::services::gcloud::discovery::GcloudDiscoveryService;
use nuvem::services::resources::cloud_run::GcloudCloudRunService;
use nuvem::services::resources::cloudsql::GcloudCloudSqlService;
use nuvem::services::resources::compute::GcloudComputeService;
use nuvem::services::resources::disks::GcloudDisksService;
use nuvem::services::resources::finops::GcloudFinOpsService;
use nuvem::services::resources::functions::GcloudFunctionsService;
use nuvem::services::resources::gke::GcloudGkeService;
use nuvem::services::resources::iam::GcloudIamService;
use nuvem::services::resources::logging::GcloudLoggingService;
use nuvem::services::resources::pubsub::GcloudPubSubService;
use nuvem::services::resources::secrets::GcloudSecretsService;
use nuvem::services::resources::storage::GcloudStorageService;
use nuvem::services::sdk::{SdkClients, block_on};
use nuvem_domain::gcp::configuration::GcloudContext;
use nuvem_domain::gcp::diagnostics::sanitize_detail;
use nuvem_domain::gcp::finops::RecommenderId;
use nuvem_domain::gcp::logging::{LogQuery, LogSeverity, LogTimeRange};
use nuvem_domain::gcp::resource::{
    CloudRunService, CloudSqlInstance, ComputeInstance, GkeCluster, Secret, StorageBucket,
};

/// Iterations per backend per path when `READ_PATH_BENCH_ITERATIONS` says
/// nothing.
const DEFAULT_ITERATIONS: usize = 5;

/// The page size the object browser asks for.
///
/// `STORAGE_PAGE_SIZE` in `src/app/reducer/resources.rs` is private to that
/// module, so this restates its value rather than importing it. If that
/// constant moves, this one has to move with it: the two backends would still
/// be fair to each other, but they would no longer measure what the app does.
const STORAGE_PAGE_SIZE: u32 = 100;

/// The logging window both backends are handed. Small on purpose: this is a
/// latency measurement, not a log search.
const LOG_RANGE: LogTimeRange = LogTimeRange::FiveMinutes;

/// The entry cap both backends are handed.
const LOG_LIMIT: usize = 50;

/// How much of a failure's text survives to stdout, after masking and
/// redaction. Short enough that an identifier the mask missed is unlikely to
/// come out whole.
const ERROR_PREVIEW_CHARS: usize = 60;

/// What a detail pane reports when its listing came back with nothing to open.
const EMPTY_LISTING: &str = "the listing returned no rows";

/// A boxed call under measurement. It answers how many rows it deserialised,
/// or why it failed.
type Call<'a> = Box<dyn FnMut() -> Result<usize, String> + 'a>;

/// Boxes one measurable call.
///
/// The bound on `body` is what gives each closure below its return type, so
/// the `?` inside them resolves without an annotation at every call site.
fn call<'a>(body: impl FnMut() -> Result<usize, String> + 'a) -> Result<Call<'a>, String> {
    Ok(Box::new(body))
}

/// What one backend did on one path.
enum Outcome {
    Measured {
        millis: Vec<f64>,
        rows: usize,
    },
    /// The path has no twin on this backend, or a prerequisite was missing.
    Skipped(String),
    /// Unsanitised. Nothing renders this without [`mask`] first.
    Failed(String),
}

impl Outcome {
    fn median(&self) -> Option<f64> {
        match self {
            Self::Measured { millis, .. } => stats(millis).map(|(_, median, _)| median),
            Self::Skipped(_) | Self::Failed(_) => None,
        }
    }

    /// `min / median / max`, or why there is nothing to show.
    fn timing_cell(&self) -> String {
        match self {
            Self::Measured { millis, .. } => match stats(millis) {
                Some((min, median, max)) => format!("{min:.0} / {median:.0} / {max:.0}"),
                None => "no samples".to_string(),
            },
            Self::Skipped(_) => "skipped".to_string(),
            Self::Failed(_) => "failed".to_string(),
        }
    }

    fn rows_cell(&self) -> String {
        match self {
            Self::Measured { rows, .. } => rows.to_string(),
            Self::Skipped(_) | Self::Failed(_) => "-".to_string(),
        }
    }
}

/// One read path, measured on both backends.
struct PathRow {
    label: &'static str,
    native: Outcome,
    cli: Outcome,
}

/// One backend's side of a path while it is being measured.
struct Lane<'a> {
    call: Option<Call<'a>>,
    skipped: Option<String>,
    failed: Option<String>,
    millis: Vec<f64>,
    rows: usize,
}

impl<'a> Lane<'a> {
    fn new(call: Result<Call<'a>, String>) -> Self {
        let (call, skipped) = match call {
            Ok(call) => (Some(call), None),
            Err(reason) => (None, Some(reason)),
        };
        Self {
            call,
            skipped,
            failed: None,
            millis: Vec::new(),
            rows: 0,
        }
    }

    /// One untimed call, so neither backend pays its connection setup inside a
    /// sample. A failure here retires the lane exactly as one during timing
    /// does: there is nothing to learn from timing a call that does not work.
    fn warm(&mut self) {
        let Some(call) = self.call.as_mut() else {
            return;
        };
        if let Err(error) = call() {
            self.failed = Some(error);
            self.call = None;
        }
    }

    fn tick(&mut self) {
        let Some(call) = self.call.as_mut() else {
            return;
        };
        let started = Instant::now();
        match call() {
            Ok(rows) => {
                self.millis.push(elapsed_millis(started));
                self.rows = rows;
            }
            Err(error) => {
                self.failed = Some(error);
                self.call = None;
            }
        }
    }

    fn finish(self) -> Outcome {
        if let Some(reason) = self.skipped {
            return Outcome::Skipped(reason);
        }
        if let Some(error) = self.failed {
            return Outcome::Failed(error);
        }
        Outcome::Measured {
            millis: self.millis,
            rows: self.rows,
        }
    }
}

#[test]
#[ignore = "reaches the live project the active gcloud configuration names"]
fn read_path_bench() {
    if let Err(error) = run() {
        // Printed rather than asserted: this is a measurement, and a machine
        // with no reachable project is not a failing repository.
        println!("read path benchmark did not run: {error}");
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let iterations = iterations_from_env();
    // A multi-thread runtime because that is what the session runs on, and the
    // body goes on a blocking thread because the `SdkClients` getters and
    // `services::sdk::block_on` both call `Handle::current().block_on`, which
    // is legal from the blocking pool and a panic from a runtime worker.
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    let report = runtime.block_on(async move {
        tokio::task::spawn_blocking(move || benchmark(iterations)).await
    })?;
    report.map_err(Into::into)
}

fn benchmark(iterations: usize) -> Result<(), String> {
    println!("# nuvem read-path benchmark");
    println!();
    println!("iterations per backend per path: {iterations}");
    println!("1-minute load average at start:  {}", load_average());
    println!("this run prints timings and row counts only, never an identifier");
    println!();

    let runner: Arc<dyn CommandRunner + Send + Sync> =
        Arc::new(ProcessCommandRunner::new(PathBuf::from("gcloud")));
    let cli_runner: &(dyn CommandRunner + Send + Sync) = runner.as_ref();

    // The context both backends are aimed by. Resolved once, untimed, and
    // never printed.
    let context = active_context(cli_runner)?;
    let configuration = context.configuration.as_str().to_string();
    let project = context
        .project
        .clone()
        .ok_or_else(|| "the active gcloud configuration names no project".to_string())?;

    println!("## fixed costs");
    println!();
    interpreter_floor(cli_runner, iterations);
    cold_start(&runner, &configuration, &project, iterations);
    poll_tick_cost(&runner, cli_runner, &configuration, &project, iterations);
    println!();

    let clients = SdkClients::new(Arc::clone(&runner));
    let log_query = LogQuery::ProjectLogs;

    // The CLI services, built once. Each borrows the runner and none of them
    // owns state, so a fresh service per path would measure the same thing.
    let storage_cli = GcloudStorageService::new(cli_runner);
    let compute_cli = GcloudComputeService::new(cli_runner);
    let disks_cli = GcloudDisksService::new(cli_runner);
    let gke_cli = GcloudGkeService::new(cli_runner);
    let cloudsql_cli = GcloudCloudSqlService::new(cli_runner);
    let secrets_cli = GcloudSecretsService::new(cli_runner);
    let pubsub_cli = GcloudPubSubService::new(cli_runner);
    let iam_cli = GcloudIamService::new(cli_runner);
    let cloud_run_cli = GcloudCloudRunService::new(cli_runner);
    let functions_cli = GcloudFunctionsService::new(cli_runner);
    let logging_cli = GcloudLoggingService::new(cli_runner);
    let finops_cli = GcloudFinOpsService::new(cli_runner);
    let discovery_cli = GcloudDiscoveryService::new(cli_runner);

    println!("## listings");
    println!();
    println!(
        "  {:<26} {:>26}   {:>26}",
        "", "native min / med / max ms", "cli min / med / max ms"
    );

    let mut rows: Vec<PathRow> = Vec::new();

    // -- storage buckets ---------------------------------------------------
    let bucket_slot: RefCell<Option<StorageBucket>> = RefCell::new(None);
    rows.push(measure_pair(
        "storage buckets",
        iterations,
        call(|| {
            let client = clients.storage(&configuration).map_err(display)?;
            let loaded = block_on(client.load_buckets(&project)).map_err(display)?;
            remember(&bucket_slot, loaded.first());
            Ok(loaded.len())
        }),
        call(|| {
            let loaded = storage_cli.load_buckets(&context).map_err(display)?;
            remember(&bucket_slot, loaded.first());
            Ok(loaded.len())
        }),
    ));
    let first_bucket = bucket_slot.into_inner();

    // -- storage object page -----------------------------------------------
    rows.push(match first_bucket.as_ref() {
        Some(bucket) => measure_pair(
            "storage object page",
            iterations,
            call(|| {
                let client = clients.storage(&configuration).map_err(display)?;
                let page =
                    block_on(client.load_object_page(&bucket.name, "", None, STORAGE_PAGE_SIZE))
                        .map_err(display)?;
                Ok(page.prefixes.len() + page.objects.len())
            }),
            call(|| {
                let page = storage_cli
                    .load_object_page(&context, &bucket.name, "", None, STORAGE_PAGE_SIZE)
                    .map_err(display)?;
                Ok(page.prefixes.len() + page.objects.len())
            }),
        ),
        None => skipped_pair("storage object page", EMPTY_LISTING),
    });

    // -- compute instances -------------------------------------------------
    let instance_slot: RefCell<Option<ComputeInstance>> = RefCell::new(None);
    rows.push(measure_pair(
        "compute instances",
        iterations,
        call(|| {
            let client = clients.compute(&configuration).map_err(display)?;
            let loaded = block_on(client.load_instances(&project)).map_err(display)?;
            remember(&instance_slot, loaded.first());
            Ok(loaded.len())
        }),
        call(|| {
            let loaded = compute_cli
                .load_instances(&context, None)
                .map_err(display)?;
            remember(&instance_slot, loaded.first());
            Ok(loaded.len())
        }),
    ));
    let first_instance = instance_slot.into_inner();

    // -- persistent disks --------------------------------------------------
    rows.push(measure_pair(
        "persistent disks",
        iterations,
        call(|| {
            let client = clients.disks(&configuration).map_err(display)?;
            let loaded = block_on(client.load_disks(&project)).map_err(display)?;
            Ok(loaded.len())
        }),
        call(|| {
            let loaded = disks_cli.load_disks(&context, None).map_err(display)?;
            Ok(loaded.len())
        }),
    ));

    // -- gke clusters ------------------------------------------------------
    let cluster_slot: RefCell<Option<GkeCluster>> = RefCell::new(None);
    rows.push(measure_pair(
        "gke clusters",
        iterations,
        call(|| {
            let client = clients.gke(&configuration).map_err(display)?;
            let loaded = block_on(client.load_clusters(&project)).map_err(display)?;
            remember(&cluster_slot, loaded.first());
            Ok(loaded.len())
        }),
        call(|| {
            let loaded = gke_cli.load_clusters(&context, None).map_err(display)?;
            remember(&cluster_slot, loaded.first());
            Ok(loaded.len())
        }),
    ));
    let first_cluster = cluster_slot.into_inner();

    // -- cloudsql instances ------------------------------------------------
    let sql_slot: RefCell<Option<CloudSqlInstance>> = RefCell::new(None);
    rows.push(measure_pair(
        "cloudsql instances",
        iterations,
        call(|| {
            let client = clients.cloudsql(&configuration).map_err(display)?;
            let loaded = block_on(client.load_instances(&project)).map_err(display)?;
            remember(&sql_slot, loaded.first());
            Ok(loaded.len())
        }),
        call(|| {
            let loaded = cloudsql_cli
                .load_instances(&context, None)
                .map_err(display)?;
            remember(&sql_slot, loaded.first());
            Ok(loaded.len())
        }),
    ));
    let first_sql = sql_slot.into_inner();

    // -- secrets -----------------------------------------------------------
    let secret_slot: RefCell<Option<Secret>> = RefCell::new(None);
    rows.push(measure_pair(
        "secrets",
        iterations,
        call(|| {
            let client = clients.secrets(&configuration).map_err(display)?;
            let loaded = block_on(client.load_secrets(&project)).map_err(display)?;
            remember(&secret_slot, loaded.first());
            Ok(loaded.len())
        }),
        call(|| {
            let loaded = secrets_cli.load_secrets(&context, None).map_err(display)?;
            remember(&secret_slot, loaded.first());
            Ok(loaded.len())
        }),
    ));
    let first_secret = secret_slot.into_inner();

    // -- pubsub ------------------------------------------------------------
    rows.push(measure_pair(
        "pubsub topics",
        iterations,
        call(|| {
            let client = clients.pubsub(&configuration).map_err(display)?;
            let loaded = block_on(client.load_topics(&project)).map_err(display)?;
            Ok(loaded.len())
        }),
        call(|| {
            let loaded = pubsub_cli.load_topics(&context, None).map_err(display)?;
            Ok(loaded.len())
        }),
    ));
    rows.push(measure_pair(
        "pubsub subscriptions",
        iterations,
        call(|| {
            let client = clients.pubsub(&configuration).map_err(display)?;
            let loaded = block_on(client.load_subscriptions(&project)).map_err(display)?;
            Ok(loaded.len())
        }),
        call(|| {
            let loaded = pubsub_cli
                .load_subscriptions(&context, None)
                .map_err(display)?;
            Ok(loaded.len())
        }),
    ));

    // -- iam ---------------------------------------------------------------
    rows.push(measure_pair(
        "iam service accounts",
        iterations,
        call(|| {
            let client = clients.iam(&configuration).map_err(display)?;
            let loaded = block_on(client.load_service_accounts(&project)).map_err(display)?;
            Ok(loaded.len())
        }),
        call(|| {
            let loaded = iam_cli
                .load_service_accounts(&context, None)
                .map_err(display)?;
            Ok(loaded.len())
        }),
    ));
    rows.push(measure_pair(
        "iam policy members",
        iterations,
        call(|| {
            let client = clients.iam(&configuration).map_err(display)?;
            let loaded = block_on(client.load_policy(&project)).map_err(display)?;
            Ok(loaded.len())
        }),
        call(|| {
            let loaded = iam_cli.load_policy_members(&context).map_err(display)?;
            Ok(loaded.len())
        }),
    ));

    // -- cloud run and cloud functions -------------------------------------
    let service_slot: RefCell<Option<CloudRunService>> = RefCell::new(None);
    rows.push(measure_pair(
        "cloud run services",
        iterations,
        call(|| {
            let client = clients.cloud_run(&configuration).map_err(display)?;
            let loaded = block_on(client.load_services(&project)).map_err(display)?;
            remember(&service_slot, loaded.first());
            Ok(loaded.len())
        }),
        call(|| {
            let loaded = cloud_run_cli.load_services(&context).map_err(display)?;
            remember(&service_slot, loaded.first());
            Ok(loaded.len())
        }),
    ));
    let first_service = service_slot.into_inner();

    rows.push(measure_pair(
        "cloud functions",
        iterations,
        call(|| {
            let client = clients.functions(&configuration).map_err(display)?;
            let loaded = block_on(client.load_functions(&project)).map_err(display)?;
            Ok(loaded.len())
        }),
        call(|| {
            let loaded = functions_cli
                .load_functions(&context, None)
                .map_err(display)?;
            Ok(loaded.len())
        }),
    ));

    // -- logging -----------------------------------------------------------
    rows.push(measure_pair(
        "logging read_recent",
        iterations,
        call(|| {
            let client = clients.logging(&configuration).map_err(display)?;
            let outcome = block_on(client.read_recent(
                &project,
                &log_query,
                LogSeverity::Default,
                LOG_RANGE,
                LOG_LIMIT,
            ))
            .map_err(display)?;
            Ok(outcome.entries.len())
        }),
        call(|| {
            let outcome = logging_cli
                .read_recent(
                    &context,
                    &log_query,
                    LogSeverity::Default,
                    LOG_RANGE,
                    LOG_LIMIT,
                )
                .map_err(display)?;
            Ok(outcome.entries.len())
        }),
    ));

    // -- finops ------------------------------------------------------------
    // A zonal recommender needs a zone and a regional one needs a region;
    // handing either the wrong kind returns an empty list that reads as
    // "nothing to save". Whichever the configuration can answer is used, and
    // both backends are handed the same pair.
    let recommender_target = context
        .zone
        .clone()
        .map(|zone| (RecommenderId::IdleVm, zone))
        .or_else(|| {
            context
                .region
                .clone()
                .map(|region| (RecommenderId::UnusedAddress, region))
        });
    rows.push(match recommender_target.as_ref() {
        Some((recommender, location)) => {
            let recommender = *recommender;
            measure_pair(
                "finops recommendations",
                iterations,
                call(|| {
                    let client = clients.finops(&configuration).map_err(display)?;
                    let loaded =
                        block_on(client.load_recommendations(&project, recommender, location))
                            .map_err(display)?;
                    Ok(loaded.len())
                }),
                call(|| {
                    let loaded = finops_cli
                        .load_recommendations(&context, recommender, location)
                        .map_err(display)?;
                    Ok(loaded.len())
                }),
            )
        }
        None => skipped_pair(
            "finops recommendations",
            "the active configuration sets no compute zone or region",
        ),
    });

    // -- discovery ---------------------------------------------------------
    rows.push(measure_pair(
        "discovery projects",
        iterations,
        call(|| {
            let client = clients.discovery(&configuration).map_err(display)?;
            let loaded = block_on(client.load_projects()).map_err(display)?;
            Ok(loaded.len())
        }),
        call(|| {
            let loaded = discovery_cli
                .load_projects(&context.configuration)
                .map_err(display)?;
            Ok(loaded.len())
        }),
    ));
    rows.push(measure_pair(
        "discovery regions",
        iterations,
        call(|| {
            let client = clients.discovery(&configuration).map_err(display)?;
            let loaded = block_on(client.load_regions(&project)).map_err(display)?;
            Ok(loaded.len())
        }),
        call(|| {
            let loaded = discovery_cli
                .load_regions(&context.configuration)
                .map_err(display)?;
            Ok(loaded.len())
        }),
    ));
    rows.push(measure_pair(
        "discovery zones",
        iterations,
        call(|| {
            let client = clients.discovery(&configuration).map_err(display)?;
            let loaded = block_on(client.load_zones(&project)).map_err(display)?;
            Ok(loaded.len())
        }),
        call(|| {
            let loaded = discovery_cli
                .load_zones(&context.configuration)
                .map_err(display)?;
            Ok(loaded.len())
        }),
    ));

    println!();
    println!("## detail panes");
    println!();

    // -- detail: compute instance ------------------------------------------
    rows.push(match first_instance.as_ref() {
        Some(instance) => measure_pair(
            "detail: compute instance",
            iterations,
            call(|| {
                let client = clients.compute(&configuration).map_err(display)?;
                block_on(client.load_instance_detail(&project, &instance.zone, &instance.name))
                    .map_err(display)?;
                Ok(1)
            }),
            call(|| {
                compute_cli
                    .load_instance_detail(&context, instance)
                    .map_err(display)?;
                Ok(1)
            }),
        ),
        None => skipped_pair("detail: compute instance", EMPTY_LISTING),
    });

    // -- detail: gke cluster -----------------------------------------------
    rows.push(match first_cluster.as_ref() {
        Some(cluster) => measure_pair(
            "detail: gke cluster",
            iterations,
            call(|| {
                let client = clients.gke(&configuration).map_err(display)?;
                block_on(client.load_cluster_detail(&project, &cluster.location, &cluster.name))
                    .map_err(display)?;
                Ok(1)
            }),
            call(|| {
                gke_cli
                    .load_cluster_detail(&context, cluster)
                    .map_err(display)?;
                Ok(1)
            }),
        ),
        None => skipped_pair("detail: gke cluster", EMPTY_LISTING),
    });

    // -- detail: cloudsql instance -----------------------------------------
    rows.push(match first_sql.as_ref() {
        Some(instance) => measure_pair(
            "detail: cloudsql instance",
            iterations,
            call(|| {
                let client = clients.cloudsql(&configuration).map_err(display)?;
                block_on(client.load_instance_detail(&project, &instance.name)).map_err(display)?;
                Ok(1)
            }),
            call(|| {
                cloudsql_cli
                    .load_instance_detail(&context, instance)
                    .map_err(display)?;
                Ok(1)
            }),
        ),
        None => skipped_pair("detail: cloudsql instance", EMPTY_LISTING),
    });

    // -- detail: secret ----------------------------------------------------
    rows.push(match first_secret.as_ref() {
        Some(secret) => measure_pair(
            "detail: secret",
            iterations,
            call(|| {
                let client = clients.secrets(&configuration).map_err(display)?;
                block_on(client.load_secret_detail(&project, &secret.name)).map_err(display)?;
                Ok(1)
            }),
            call(|| {
                secrets_cli
                    .load_secret_detail(&context, &secret.name)
                    .map_err(display)?;
                Ok(1)
            }),
        ),
        None => skipped_pair("detail: secret", EMPTY_LISTING),
    });

    // -- detail: storage bucket --------------------------------------------
    rows.push(match first_bucket.as_ref() {
        Some(bucket) => measure_pair(
            "detail: storage bucket",
            iterations,
            call(|| {
                let client = clients.storage(&configuration).map_err(display)?;
                block_on(client.load_bucket_detail(&bucket.name)).map_err(display)?;
                Ok(1)
            }),
            call(|| {
                storage_cli
                    .load_bucket_detail(&context, &bucket.name)
                    .map_err(display)?;
                Ok(1)
            }),
        ),
        None => skipped_pair("detail: storage bucket", EMPTY_LISTING),
    });

    // -- detail: cloud run service -----------------------------------------
    rows.push(match first_service.as_ref() {
        Some(service) => measure_pair(
            "detail: cloud run service",
            iterations,
            call(|| {
                let client = clients.cloud_run(&configuration).map_err(display)?;
                block_on(client.load_service_detail(&project, &service.region, &service.name))
                    .map_err(display)?;
                Ok(1)
            }),
            call(|| {
                cloud_run_cli
                    .load_service_detail(&context, service)
                    .map_err(display)?;
                Ok(1)
            }),
        ),
        None => skipped_pair("detail: cloud run service", EMPTY_LISTING),
    });

    // Every identifier this run touched, longest first so a short one cannot
    // pre-empt a longer one it is a substring of. Nothing below prints an
    // error without going through this.
    let masks = masks_for(
        &context,
        first_bucket.as_ref(),
        first_instance.as_ref(),
        first_cluster.as_ref(),
        first_sql.as_ref(),
        first_secret.as_ref(),
        first_service.as_ref(),
    );

    println!();
    print_table(&rows, &masks);
    println!();
    print_summary(&rows, iterations);
    println!();
    println!("1-minute load average at end: {}", load_average());
    Ok(())
}

/// The configuration gcloud would use if nothing named one, resolved to the
/// account, project, region and zone a command running under it would see.
fn active_context(runner: &(dyn CommandRunner + Send + Sync)) -> Result<GcloudContext, String> {
    // The failure text is withheld rather than shown, for the reason
    // `interpreter_floor` withholds its own: this runs before the identifier
    // masks exist, and there is nothing to build them from until it succeeds.
    // `CommandError::Failed` carries the child's stderr verbatim, and gcloud's
    // stderr names the account and the config directory — which is to say the
    // operator — whenever it complains about either. This error reaches the
    // terminal through `main`, where no mask can reach it.
    let configurations = GcloudConfigurationService::new(runner)
        .load_configurations()
        .map_err(|_| {
            "gcloud could not list its configurations; its message is withheld because it \
             precedes the redaction this example prints errors through"
                .to_string()
        })?;
    configurations
        .into_iter()
        .find(|configuration| configuration.is_active)
        .map(|configuration| configuration.context())
        .ok_or_else(|| "gcloud reports no active configuration".to_string())
}

/// What one poll tick costs the machine on each backend.
///
/// The decision record left an experiment outstanding: raising
/// `poll_interval_secs` from 5 to 30 would cut listing volume six-fold, and if
/// most of the cost is the cadence then that is cheaper than migrating a
/// backend. The question it asks is how much work one tick is, and wall time
/// does not answer it -- a poll runs off-screen, so what it spends is the
/// machine's, not the operator's.
///
/// So this measures processor time rather than elapsed time: `getrusage`
/// before and after N listings of the same table on each backend, children
/// included, divided by N. On the CLI backend each tick is a Python
/// interpreter start; on the native backend it is one request on a runtime
/// that is already running.
fn poll_tick_cost(
    runner: &Arc<dyn CommandRunner + Send + Sync>,
    cli_runner: &(dyn CommandRunner + Send + Sync),
    configuration: &str,
    project: &str,
    iterations: usize,
) {
    const NATIVE: &str = "native cpu per poll tick";
    const CLI: &str = "cli cpu per poll tick";

    let clients = SdkClients::new(Arc::clone(runner));
    let context = GcloudContext {
        configuration: nuvem_domain::gcp::configuration::GcloudConfigurationName::new(
            configuration,
        ),
        account: None,
        project: Some(project.to_string()),
        region: None,
        zone: None,
    };
    let cli = GcloudStorageService::new(cli_runner);

    // The bucket listing, because it is the table the record named and the
    // cheapest listing on both backends -- the tick's floor rather than a
    // favourable case. One untimed call per backend first, as everywhere else.
    let native_once = || {
        clients
            .storage(configuration)
            .and_then(|client| block_on(client.load_buckets(project)))
            .is_ok()
    };
    let cli_once = || cli.load_buckets(&context).is_ok();

    if native_once() {
        let before = cpu_seconds();
        let mut healthy = true;
        for _ in 0..iterations {
            healthy &= native_once();
        }
        report_cpu(NATIVE, cpu_seconds() - before, iterations, healthy);
    } else {
        println!("  {NATIVE:<44} failed");
    }

    if cli_once() {
        let before = cpu_seconds();
        let mut healthy = true;
        for _ in 0..iterations {
            healthy &= cli_once();
        }
        report_cpu(CLI, cpu_seconds() - before, iterations, healthy);
    } else {
        println!("  {CLI:<44} failed");
    }
}

fn report_cpu(label: &str, spent: f64, iterations: usize, healthy: bool) {
    if !healthy || iterations == 0 {
        println!("  {label:<44} failed");
        return;
    }
    // Two decimals, because this is the number the poll-cadence question
    // turns on and a native tick is expected to land under a millisecond:
    // rounding it to `0 ms` would hide the size of the gap rather than report
    // it.
    let per_tick = spent / iterations as f64;
    println!("  {label:<44} {:.2} ms", per_tick * 1000.0);
}

/// User plus system processor time for this process and every child it has
/// reaped, in seconds.
///
/// `RUSAGE_CHILDREN` is the half that matters for the CLI backend: the work is
/// done in a subprocess, and a measurement that counted only this process would
/// report a gcloud listing as nearly free.
fn cpu_seconds() -> f64 {
    fn total(who: i32) -> f64 {
        // SAFETY: `getrusage` writes the `rusage` it is handed and reads
        // nothing else; the struct is zeroed first, so every field it does not
        // set is defined.
        let mut usage: libc::rusage = unsafe { std::mem::zeroed() };
        // SAFETY: the pointer is to the zeroed local above, which outlives
        // the call, and `getrusage` only writes through it.
        if unsafe { libc::getrusage(who, &mut usage) } != 0 {
            return 0.0;
        }
        let seconds = |time: libc::timeval| time.tv_sec as f64 + time.tv_usec as f64 / 1_000_000.0;
        seconds(usage.ru_utime) + seconds(usage.ru_stime)
    }
    total(libc::RUSAGE_SELF) + total(libc::RUSAGE_CHILDREN)
}

/// `gcloud --version`, which does nothing but start the interpreter.
///
/// Every CLI row below pays this before a single byte of its own moves, so it
/// is the part of the gap between the two backends that no amount of
/// server-side speed can close.
fn interpreter_floor(runner: &(dyn CommandRunner + Send + Sync), iterations: usize) {
    const LABEL: &str = "gcloud --version (interpreter floor)";

    let mut millis = Vec::new();
    // One untimed call first, matching every other lane in this file.
    let mut healthy = runner.run(GcloudCommand::new(["--version"])).is_ok();
    if healthy {
        for _ in 0..iterations {
            let started = Instant::now();
            if runner.run(GcloudCommand::new(["--version"])).is_err() {
                healthy = false;
                break;
            }
            millis.push(elapsed_millis(started));
        }
    }
    // The failure text is dropped rather than shown: the identifier masks are
    // not built this early, and an unmasked message is the one thing this
    // example must never print.
    match stats(&millis).filter(|_| healthy) {
        Some((min, median, max)) => {
            println!("  {LABEL:<44} {min:.0} / {median:.0} / {max:.0} ms");
        }
        None => println!("  {LABEL:<44} failed"),
    }
}

/// What a cold cache costs, and what it stops costing once it is warm.
///
/// Three numbers, because they are three different things and the decision
/// record only ever quoted the third. Construction is `clients.storage` on a
/// cache that holds nothing: it builds the client and hands it a credential
/// provider without yet asking that provider for anything. The first request
/// is where the provider is asked, so it pays the `gcloud config config-helper`
/// subprocess on top of the request itself. The warm requests reuse the cached
/// token, and are what the poll actually repeats.
///
/// The `SdkClients` is built here rather than shared with the run below,
/// because a cache that has already served someone is not cold and a token
/// already minted is not acquired.
fn cold_start(
    runner: &Arc<dyn CommandRunner + Send + Sync>,
    configuration: &str,
    project: &str,
    iterations: usize,
) {
    const CONSTRUCTION: &str = "native client construction (cold cache)";
    const FIRST: &str = "first native request after construction";
    const WARM: &str = "warm native requests on the same client";

    let clients = SdkClients::new(Arc::clone(runner));

    let started = Instant::now();
    let Ok(client) = clients.storage(configuration) else {
        println!("  {CONSTRUCTION:<44} failed");
        return;
    };
    println!("  {CONSTRUCTION:<44} {:.0} ms", elapsed_millis(started));

    let started = Instant::now();
    if block_on(client.load_buckets(project)).is_err() {
        println!("  {FIRST:<44} failed");
        return;
    }
    println!("  {FIRST:<44} {:.0} ms", elapsed_millis(started));

    let mut millis = Vec::new();
    for _ in 0..iterations {
        let started = Instant::now();
        if block_on(client.load_buckets(project)).is_err() {
            break;
        }
        millis.push(elapsed_millis(started));
    }
    match stats(&millis) {
        Some((min, median, max)) => {
            println!("  {WARM:<44} {min:.0} / {median:.0} / {max:.0} ms");
        }
        None => println!("  {WARM:<44} failed"),
    }
}

/// Warms both backends, then interleaves their iterations so that a machine
/// which gets busier partway through slows the two equally.
fn measure_pair(
    label: &'static str,
    iterations: usize,
    native: Result<Call<'_>, String>,
    cli: Result<Call<'_>, String>,
) -> PathRow {
    let mut native_lane = Lane::new(native);
    let mut cli_lane = Lane::new(cli);
    native_lane.warm();
    cli_lane.warm();
    for _ in 0..iterations {
        native_lane.tick();
        cli_lane.tick();
    }
    let row = PathRow {
        label,
        native: native_lane.finish(),
        cli: cli_lane.finish(),
    };
    println!(
        "  {:<26} {:>26}   {:>26}",
        row.label,
        row.native.timing_cell(),
        row.cli.timing_cell()
    );
    row
}

/// A path neither backend could be asked for, because a prerequisite the two
/// share is missing.
fn skipped_pair(label: &'static str, reason: &str) -> PathRow {
    measure_pair(label, 0, Err(reason.to_string()), Err(reason.to_string()))
}

fn print_table(rows: &[PathRow], masks: &[String]) {
    println!("## results");
    println!();
    println!("| path | native ms | native rows | cli ms | cli rows | cli / native |");
    println!("| --- | ---: | ---: | ---: | ---: | ---: |");
    for row in rows {
        let ratio = match (row.native.median(), row.cli.median()) {
            (Some(native), Some(cli)) if native > 0.0 => format!("{:.1}x", cli / native),
            _ => "-".to_string(),
        };
        println!(
            "| {} | {} | {} | {} | {} | {} |",
            row.label,
            row.native.timing_cell(),
            row.native.rows_cell(),
            row.cli.timing_cell(),
            row.cli.rows_cell(),
            ratio
        );
    }
    println!();
    println!("Timings are min / median / max wall milliseconds per call.");

    let mut notes: Vec<String> = Vec::new();
    for row in rows {
        for (backend, outcome) in [("native", &row.native), ("cli", &row.cli)] {
            match outcome {
                // Every skip reason in this file is a literal, so masking one
                // is a no-op today. It goes through the mask anyway: the rule
                // this example has to keep is that nothing reaches stdout
                // unmasked, and a rule that holds only because of what the
                // current call sites happen to pass is not a rule.
                Outcome::Skipped(reason) => {
                    let detail = mask(reason, masks);
                    notes.push(format!("- {} / {backend}: skipped, {detail}", row.label));
                }
                Outcome::Failed(error) => {
                    let detail = mask(error, masks);
                    notes.push(format!("- {} / {backend}: failed, {detail}", row.label));
                }
                Outcome::Measured { .. } => {}
            }
        }
    }
    if !notes.is_empty() {
        println!();
        println!("### skipped and failed");
        println!();
        for note in &notes {
            println!("{note}");
        }
    }
}

fn print_summary(rows: &[PathRow], iterations: usize) {
    let paired: Vec<(f64, f64)> = rows
        .iter()
        .filter_map(|row| Some((row.native.median()?, row.cli.median()?)))
        .collect();

    println!("## summary");
    println!();
    if paired.is_empty() {
        println!(
            "No path completed on both backends, so there is nothing to compare here. \
             The skipped-and-failed list above says why, one line per path and backend."
        );
        return;
    }

    let native_total: f64 = paired.iter().map(|(native, _)| native).sum();
    let cli_total: f64 = paired.iter().map(|(_, cli)| cli).sum();
    let widest = paired
        .iter()
        .filter(|(native, _)| *native > 0.0)
        .map(|(native, cli)| cli / native)
        .fold(0.0_f64, f64::max);
    let ratio = if native_total > 0.0 {
        cli_total / native_total
    } else {
        0.0
    };
    let measured = paired.len();
    let total = rows.len();

    println!(
        "Over {iterations} iterations per backend, {measured} of {total} read paths completed \
         on both. Adding up one median call per path, the native backend spends \
         {native_total:.0} ms where the CLI backend spends {cli_total:.0} ms, a factor of \
         {ratio:.1}; the widest single-path gap is {widest:.1}x. The decision record measured \
         one of these paths — the bucket listing — at 1.32s-1.56s on gcloud against \
         0.31s-0.47s over HTTPS, and the rows above are what that comparison looks like on \
         this machine for every path rather than for one. Read them against the interpreter \
         floor printed at the top: `gcloud --version` moves no data at all, so whatever it \
         costs is what every CLI row pays before its own request begins, and the load \
         averages at the start and end say whether the machine stayed still while it was \
         measured."
    );
}

/// Records the first row a listing returned, so both backends open the same
/// detail pane. First writer wins, and the native lane warms first.
fn remember<T: Clone>(slot: &RefCell<Option<T>>, row: Option<&T>) {
    let mut held = slot.borrow_mut();
    if held.is_none() {
        *held = row.cloned();
    }
}

fn display<E: std::fmt::Display>(error: E) -> String {
    error.to_string()
}

fn elapsed_millis(started: Instant) -> f64 {
    started.elapsed().as_secs_f64() * 1000.0
}

fn stats(millis: &[f64]) -> Option<(f64, f64, f64)> {
    if millis.is_empty() {
        return None;
    }
    let mut sorted = millis.to_vec();
    sorted.sort_by(f64::total_cmp);
    let middle = sorted.len() / 2;
    let median = if sorted.len().is_multiple_of(2) {
        (sorted[middle - 1] + sorted[middle]) / 2.0
    } else {
        sorted[middle]
    };
    Some((sorted[0], median, sorted[sorted.len() - 1]))
}

fn iterations_from_env() -> usize {
    std::env::var("READ_PATH_BENCH_ITERATIONS")
        .ok()
        .and_then(|raw| raw.trim().parse::<usize>().ok())
        .filter(|count| *count > 0)
        .unwrap_or(DEFAULT_ITERATIONS)
}

/// Every string this run learned that could identify a project, a person or a
/// resource, longest first.
///
/// Built after the run rather than before it because a listing's first row is
/// only known once the listing has happened, and that row's name is exactly
/// what a detail-pane failure quotes back.
fn masks_for(
    context: &GcloudContext,
    bucket: Option<&StorageBucket>,
    instance: Option<&ComputeInstance>,
    cluster: Option<&GkeCluster>,
    sql: Option<&CloudSqlInstance>,
    secret: Option<&Secret>,
    service: Option<&CloudRunService>,
) -> Vec<String> {
    let mut masks: Vec<String> = vec![context.configuration.as_str().to_string()];
    let resolved = [
        context.project.clone(),
        context.account.clone(),
        context.region.clone(),
        context.zone.clone(),
    ];
    masks.extend(resolved.into_iter().flatten());
    // An account is `local@domain`, and a message is as likely to quote one
    // half as the whole.
    if let Some(account) = context.account.as_deref()
        && let Some((local, domain)) = account.split_once('@')
    {
        masks.push(local.to_string());
        masks.push(domain.to_string());
    }
    if let Some(bucket) = bucket {
        masks.push(bucket.name.clone());
        masks.push(bucket.location.clone());
    }
    if let Some(instance) = instance {
        masks.push(instance.name.clone());
        masks.push(instance.zone.clone());
        masks.push(instance.id.clone());
    }
    if let Some(cluster) = cluster {
        masks.push(cluster.name.clone());
        masks.push(cluster.location.clone());
    }
    if let Some(sql) = sql {
        masks.push(sql.name.clone());
        masks.push(sql.region.clone());
    }
    if let Some(secret) = secret {
        masks.push(secret.name.clone());
    }
    if let Some(service) = service {
        masks.push(service.name.clone());
        masks.push(service.region.clone());
    }

    // Two characters is not an identifier, it is a fragment that would turn a
    // message into noise without hiding anything.
    masks.retain(|value| value.chars().count() >= 3);
    masks.sort();
    masks.dedup();
    masks.sort_by_key(|value| std::cmp::Reverse(value.len()));
    masks
}

/// A failure rendered so it cannot carry an identifier out of the process.
///
/// Three passes, narrowest last. Every string this run learned is replaced;
/// then the application's own redaction strips credential-shaped tokens and
/// control characters and collapses whitespace; then the result is cut to
/// [`ERROR_PREVIEW_CHARS`], which is the backstop for an identifier the first
/// pass never saw.
fn mask(error: &str, masks: &[String]) -> String {
    let mut text = error.to_string();
    for value in masks {
        text = text.replace(value.as_str(), "<id>");
    }
    let sanitized = sanitize_detail(&text);
    let preview: String = sanitized.chars().take(ERROR_PREVIEW_CHARS).collect();
    if sanitized.chars().count() > ERROR_PREVIEW_CHARS {
        format!("{preview}...")
    } else {
        preview
    }
}

/// The machine's 1-minute load average, so a reader can tell a slow path from
/// a busy machine.
fn load_average() -> String {
    if let Ok(text) = std::fs::read_to_string("/proc/loadavg")
        && let Some(first) = text.split_whitespace().next()
    {
        return first.to_string();
    }
    Command::new("uptime")
        .output()
        .ok()
        .and_then(|output| parse_uptime_load(&String::from_utf8_lossy(&output.stdout)))
        .unwrap_or_else(|| "unknown".to_string())
}

/// The first number after `uptime`'s `load average:` / `load averages:`,
/// whichever spelling and whichever separator this platform uses.
fn parse_uptime_load(text: &str) -> Option<String> {
    let tail = text.rsplit_once("average")?.1;
    let tail = tail.trim_start_matches(['s', ':', ' ']);
    let first = tail
        .split([',', ' '])
        .map(str::trim)
        .find(|token| !token.is_empty())?;
    Some(first.to_string())
}
