use std::fs;

use tracing_subscriber::EnvFilter;
use tracing_subscriber::filter::LevelFilter;

use crate::Config;

/// Log targets held at `info` whatever `RUST_LOG` says.
///
/// The generated Google Cloud clients instrument every request at `debug`
/// and `trace`, which is where a payload — a describe response with its
/// certificate fields, a config-helper reply with its token — would be
/// written out verbatim. The log file is the one surface the diagnostics
/// sanitiser never sees, so the floor is enforced here rather than trusted to
/// whoever set the environment. `tests/logging_floor_test.rs` holds this list
/// to the client crates the manifest actually depends on.
pub const CLIENT_LOG_TARGETS: &[&str] = &[
    "google_cloud_auth",
    "google_cloud_gax",
    "google_cloud_gax_internal",
    "google_cloud_compute_v1",
    "google_cloud_container_v1",
    "google_cloud_functions_v2",
    "google_cloud_iam_admin_v1",
    "google_cloud_iam_v1",
    "google_cloud_logging_v2",
    "google_cloud_lro",
    "google_cloud_pubsub",
    "google_cloud_recommender_v1",
    "google_cloud_resourcemanager_v3",
    "google_cloud_run_v2",
    "google_cloud_secretmanager_v1",
    "google_cloud_sql_v1",
    "google_cloud_storage",
    "google_cloud_wkt",
    "reqwest",
    "hyper",
    "hyper_util",
    "h2",
    "rustls",
    "tower",
];

/// The filter the subscriber runs under: `RUST_LOG` on top of an `info`
/// default, with the client targets pinned at `info` last so an environment
/// directive cannot lower them.
pub fn log_filter() -> EnvFilter {
    let mut filter = EnvFilter::builder()
        .with_default_directive(LevelFilter::INFO.into())
        .from_env_lossy();
    for target in CLIENT_LOG_TARGETS {
        filter = filter.add_directive(
            format!("{target}=info")
                .parse()
                .expect("a static `target=info` directive parses"),
        );
    }
    filter
}

/// Opens the log file and installs the tracing subscriber.
///
/// Filesystem failures here carry [`crate::exit::STARTUP`]: nothing has run
/// yet, so a caller retrying is retrying a local problem rather than a cloud
/// one.
pub fn initialize_tracing(config: &Config) -> anyhow::Result<()> {
    let startup = |error: std::io::Error| {
        anyhow::Error::new(crate::exit::Categorised::with_source(
            crate::exit::STARTUP,
            format!("could not open the log file {}", config.log_file.display()),
            error,
        ))
    };
    if let Some(parent) = config.log_file.parent() {
        fs::create_dir_all(parent).map_err(startup)?;
    }
    let mut options = fs::OpenOptions::new();
    options.create(true).append(true);
    // Owner-only on creation: the file receives whatever the floor above lets
    // through, and a world-readable log is not a decision to make by default.
    // An existing file keeps the mode it was created with.
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = options.open(&config.log_file).map_err(startup)?;
    tracing_subscriber::fmt()
        .with_env_filter(log_filter())
        .with_writer(file)
        .with_ansi(false)
        .try_init()
        .ok();
    Ok(())
}
