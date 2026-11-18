//! Native Google Cloud clients for the read paths.
//!
//! See `docs/decisions/0001-native-sdk-for-read-paths.md`. Listings and detail
//! loads come through here; configuration, authentication, tunnels, plugins,
//! BigQuery and label-filtered listings stay on the `gcloud` and `bq` CLIs,
//! and the modules under `services/resources/` keep their CLI implementations
//! as the fallback backend for those.
//!
//! A live cache's clients are authenticated by
//! [`crate::services::gcloud::context_credentials`], so a reading is aimed by
//! the configuration the operator selected rather than by machine-wide
//! Application Default Credentials. A [`ClientSource::Fixtures`] cache --
//! which is what `--demo` builds -- fetches no token at all, because its
//! clients are constructed `from_stub` rather than connected.

pub mod cloud_run;
pub mod cloudsql;
pub mod compute;
pub mod discovery;
pub mod disks;
pub mod finops;
pub mod functions;
pub mod gke;
pub mod iam;
pub mod logging;
pub mod pubsub;
pub mod secrets;
pub mod storage;
pub mod vpc;

/// A failure reported by a native client rather than by a spawned command.
///
/// Kept distinct from `CommandError` because nothing was spawned: there is no
/// exit status, no stderr, and no argv to show an operator. The message is the
/// service's own.
#[derive(Debug, Clone, thiserror::Error)]
#[error("{0}")]
pub struct ApiError(String);

impl ApiError {
    pub fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl From<google_cloud_gax::error::Error> for ApiError {
    fn from(error: google_cloud_gax::error::Error) -> Self {
        Self(error.to_string())
    }
}

/// Seconds since the epoch rendered the way the CLI renders a timestamp, so a
/// row does not change shape depending on which backend filled it.
///
/// The CLI answers `2024-01-02T03:04:05Z`; the clients answer a protobuf
/// `Timestamp`. Projections compare and sort these as strings, so they have to
/// agree.
pub fn timestamp_to_string(value: Option<&google_cloud_wkt::Timestamp>) -> Option<String> {
    value.map(|stamp| {
        chrono::DateTime::from_timestamp(stamp.seconds(), stamp.nanos() as u32)
            .map(|at| at.format("%Y-%m-%dT%H:%M:%SZ").to_string())
            .unwrap_or_default()
    })
}

/// `""` is how the generated models spell "unset" for a string field, because
/// proto3 scalars have no null. A projection that shows an empty string where
/// the CLI showed nothing is a rendering difference, so collapse it here.
pub fn non_empty(value: &str) -> Option<String> {
    (!value.is_empty()).then(|| value.to_string())
}

/// Where a cached client's answers come from.
///
/// Live and fixture-backed clients are the same types holding the same
/// caches; only their construction differs, so the difference is carried
/// here rather than by a second cache type that every caller would have to
/// choose between.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClientSource {
    /// Built from the configuration's credentials, reaching the real API.
    Live,
    /// Built from `from_stub`, answering the demo fixtures. Such a client
    /// holds a stub where a transport would be, so it is offline by
    /// construction rather than by a flag anyone can forget to check.
    Fixtures,
}

/// The native clients a session uses, built on demand and kept per gcloud
/// configuration.
///
/// A client is bound to the credentials it was constructed with, and those
/// credentials are bound to one configuration name, so switching context must
/// not reuse a client built for the configuration just left. Keying the cache
/// by name is what makes that impossible rather than merely unlikely.
///
/// Construction is deferred because it costs a TLS setup and, on first use,
/// one `gcloud` call for the token. A session that never opens a migrated
/// workspace never pays either.
///
/// # Why the fixture arm lives in the macro
///
/// A [`ClientSource::Fixtures`] cache serves every service through
/// `<Service>::from_fixtures()`, and the macro expands that call once per
/// entry in the invocation list below. That is the point of putting it here:
/// a fifteenth service cannot be added to that list without writing a
/// `from_fixtures` for it, because the expansion would not compile. The
/// guard is the compiler, on every build, naming the service it is missing.
///
/// This is precisely what the demo coverage walk cannot give. That test drives
/// a `--demo` session per service and asserts the cells it paints, which is
/// a strong assertion about *rendering* and says nothing about *backend*: a
/// demo session keeps its command runner either way, so a service nobody
/// wired to the fixture clients renders the same fixtures through the CLI
/// path and the walk stays green.
///
/// # What this macro does not guard
///
/// Existence, not use. The expansion proves every cached service *has* a
/// `from_fixtures()`; it cannot prove any effect handler calls it. A `load_*`
/// left on its `Gcloud*Service::new(runner)` arm compiles perfectly well
/// beside a stub nothing reaches.
///
/// Routing is guarded one layer out, by
/// `runtime_native_effect_test::a_demo_session_never_asks_gcloud_for_credentials`:
/// it walks every `ResourceServiceKind` through a real `--demo` session behind
/// a recording runner and asserts the complete set of argvs spawned, which is
/// two. A read that falls through to the CLI spawns a third and fails there by
/// name.
macro_rules! sdk_client_cache {
    ($($field:ident : $service:path => $method:ident / $seeder:ident),+ $(,)?) => {
        pub struct SdkClients {
            runner: std::sync::Arc<
                dyn crate::services::gcloud::cli::CommandRunner + Send + Sync,
            >,
            source: ClientSource,
            $($field: std::sync::Mutex<
                std::collections::HashMap<String, std::sync::Arc<$service>>,
            >,)+
        }

        impl std::fmt::Debug for SdkClients {
            fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.debug_struct("SdkClients").finish_non_exhaustive()
            }
        }

        impl SdkClients {
            /// A cache whose clients are built from real credentials.
            ///
            /// The runner is what fetches the token for a configuration, so a
            /// live cache needs one; see [`ClientSource`].
            pub fn new(
                runner: std::sync::Arc<
                    dyn crate::services::gcloud::cli::CommandRunner + Send + Sync,
                >,
            ) -> Self {
                Self {
                    runner,
                    source: ClientSource::Live,
                    $($field: std::sync::Mutex::new(std::collections::HashMap::new()),)+
                }
            }

            /// A cache whose clients answer the demo fixtures.
            ///
            /// Built over a `DemoCommandRunner` so that the one thing a cache
            /// uses its runner for -- fetching a token -- could not reach a
            /// real `gcloud` even if a fixture client ever asked for
            /// credentials, which none of them does.
            pub fn fixtures() -> Self {
                Self {
                    source: ClientSource::Fixtures,
                    ..Self::new(std::sync::Arc::new(
                        crate::services::demo::DemoCommandRunner::new(),
                    ))
                }
            }

            /// The same fixture cache, over a runner the caller supplies.
            ///
            /// The one thing a cache uses its runner for is fetching a
            /// token, and `fixtures` deliberately hides that behind a
            /// `DemoCommandRunner` -- which also hides it from a test. A
            /// fixture client is supposed never to ask, and the only way to
            /// assert "never asked" is to hold the runner it would have
            /// asked. Nothing in the application calls this; the binary takes
            /// `fixtures`.
            pub fn fixtures_over_for_test(
                runner: std::sync::Arc<
                    dyn crate::services::gcloud::cli::CommandRunner + Send + Sync,
                >,
            ) -> Self {
                Self {
                    source: ClientSource::Fixtures,
                    ..Self::new(runner)
                }
            }

            /// Where this cache's clients come from.
            pub fn source(&self) -> ClientSource {
                self.source
            }

            /// Drops every client and every cached token except the ones
            /// built for `keep`.
            ///
            /// Called on a configuration switch. A client is cheap to hold
            /// but its credentials carry a bearer token, and a session that
            /// walks through many configurations should not keep a token for
            /// each of them alive until it exits. Rebuilding one on return
            /// costs a TLS setup and one gcloud call, which a switch already
            /// pays.
            pub fn evict_except(&self, keep: &str) {
                $(
                    self.$field
                        .lock()
                        .expect("sdk client cache is not poisoned")
                        .retain(|configuration, _| configuration == keep);
                )+
            }

            /// Every configuration name that currently holds a client, in
            /// any cache. For tests and diagnostics; the runtime never lists
            /// them.
            pub fn cached_configurations(&self) -> Vec<String> {
                let mut names = std::collections::BTreeSet::new();
                $(
                    names.extend(
                        self.$field
                            .lock()
                            .expect("sdk client cache is not poisoned")
                            .keys()
                            .cloned(),
                    );
                )+
                names.into_iter().collect()
            }

            $(
                /// Puts an already-built service in the cache for one
                /// configuration, so a test can hold clients with no
                /// credentials and no network.
                ///
                /// Nothing in the application calls this: the cache is filled
                /// by the getter below, on first ask. It exists because every
                /// session harness ran with no clients at all, which left the
                /// native branch of every effect unreachable from the suite.
                pub fn $seeder(
                    configuration: &str,
                    service: $service,
                ) -> std::sync::Arc<Self> {
                    let clients = Self::new(std::sync::Arc::new(
                        crate::services::demo::DemoCommandRunner::new(),
                    ));
                    clients
                        .$field
                        .lock()
                        .expect("sdk client cache is not poisoned")
                        .insert(configuration.to_string(), std::sync::Arc::new(service));
                    std::sync::Arc::new(clients)
                }

                /// The client for one configuration, constructed on first ask.
                ///
                /// Called from the blocking effect threads, so it drives the
                /// client's own async construction to completion here rather
                /// than making every caller async: the thread is already one
                /// the runtime set aside for blocking. A fixture-backed cache
                /// has no such construction to drive, and still caches per
                /// configuration so that the two sources behave identically
                /// everywhere except in what they build.
                pub fn $method(
                    &self,
                    configuration: &str,
                ) -> Result<std::sync::Arc<$service>, ApiError> {
                    if let Some(existing) = self
                        .$field
                        .lock()
                        .expect("sdk client cache is not poisoned")
                        .get(configuration)
                    {
                        return Ok(std::sync::Arc::clone(existing));
                    }
                    let built = match self.source {
                        ClientSource::Live => {
                            let credentials =
                                crate::services::gcloud::context_credentials::credentials_for(
                                    std::sync::Arc::clone(&self.runner),
                                    configuration,
                                );
                            // Through `block_on` rather than
                            // `Handle::block_on` directly: a client is
                            // constructed on first ask, and the first ask can
                            // come from a detail pane dispatched on the run
                            // loop, where the bare call panics. See
                            // `block_on`.
                            std::sync::Arc::new(
                                crate::services::sdk::block_on(
                                    <$service>::connect(credentials),
                                )?,
                            )
                        }
                        // No credentials to fetch, no transport to open, and
                        // so nothing to block on or to fail: the whole
                        // asynchronous construction above exists for the live
                        // path's TLS setup and token.
                        ClientSource::Fixtures => {
                            std::sync::Arc::new(<$service>::from_fixtures())
                        }
                    };
                    self.$field
                        .lock()
                        .expect("sdk client cache is not poisoned")
                        .insert(configuration.to_string(), std::sync::Arc::clone(&built));
                    Ok(built)
                }
            )+
        }
    };
}

sdk_client_cache! {
    storage_clients: crate::services::sdk::storage::SdkStorageService => storage / with_storage_for_test,
    compute_clients: crate::services::sdk::compute::SdkComputeService => compute / with_compute_for_test,
    vpc_clients: crate::services::sdk::vpc::SdkVpcService => vpc / with_vpc_for_test,
    disks_clients: crate::services::sdk::disks::SdkDisksService => disks / with_disks_for_test,
    gke_clients: crate::services::sdk::gke::SdkGkeService => gke / with_gke_for_test,
    cloudsql_clients: crate::services::sdk::cloudsql::SdkCloudSqlService => cloudsql / with_cloudsql_for_test,
    secrets_clients: crate::services::sdk::secrets::SdkSecretsService => secrets / with_secrets_for_test,
    pubsub_clients: crate::services::sdk::pubsub::SdkPubSubService => pubsub / with_pubsub_for_test,
    iam_clients: crate::services::sdk::iam::SdkIamService => iam / with_iam_for_test,
    cloud_run_clients: crate::services::sdk::cloud_run::SdkCloudRunService => cloud_run / with_cloud_run_for_test,
    functions_clients: crate::services::sdk::functions::SdkFunctionsService => functions / with_functions_for_test,
    logging_clients: crate::services::sdk::logging::SdkLoggingService => logging / with_logging_for_test,
    finops_clients: crate::services::sdk::finops::SdkFinOpsService => finops / with_finops_for_test,
    discovery_clients: crate::services::sdk::discovery::SdkDiscoveryService => discovery / with_discovery_for_test,
}

/// The response rendered as the JSON tab shows it.
///
/// Every redaction pass the CLI paths use runs here, on every pane, rather
/// than each pane choosing: KMS key material by normalised field name, and
/// Cloud SQL's certificate and private-key keys plus anything carrying PEM
/// markers. A pane that needed one and did not call it would leak silently,
/// and the passes are cheap and idempotent on payloads that carry none of it.
pub fn detail_json<T: serde::Serialize>(value: &T) -> String {
    let mut json = match serde_json::to_value(value) {
        Ok(json) => json,
        Err(error) => return format!("<could not render JSON: {error}>"),
    };
    crate::services::resources::storage::redact_kms_key_material(&mut json);
    crate::services::resources::cloudsql::redact_secrets(&mut json);
    serde_json::to_string_pretty(&json)
        .unwrap_or_else(|error| format!("<could not render: {error}>"))
}

/// Runs a client call from a blocking effect thread, abandoning it if the
/// operator navigates away.
///
/// `None` means cancelled. Dropping the future *is* the cancellation: the
/// in-flight request goes with it, where the CLI path had to kill a process
/// group to achieve the same thing.
pub fn block_on_cancellable<F: std::future::Future>(
    future: F,
    cancellation: &tokio_util::sync::CancellationToken,
) -> Option<F::Output> {
    // Through the same bridge as `block_on`, and for the same reason: this is
    // reachable from a runtime worker as well as from a blocking thread.
    block_on(async {
        tokio::select! {
            output = future => Some(output),
            _ = cancellation.cancelled() => None,
        }
    })
}

/// Runs a client call to completion from whichever thread the caller is on.
///
/// # Why this is not just `Handle::block_on`
///
/// It was, and it panicked. `Handle::block_on` refuses to run on a thread that
/// is currently driving the runtime -- "Cannot start a runtime from within a
/// runtime" -- and two kinds of caller reach here. Effects that go through
/// `dispatch_background_refresh` run on a `spawn_blocking` thread, where the
/// bare call is fine, and that is every listing. Effects that fall to the run
/// loop's catch-all are awaited inline on a runtime worker, and that is every
/// detail pane: opening one crashed the process.
///
/// `block_in_place` is what makes one bridge serve both. On a worker it hands
/// this thread's queued work to a sibling before blocking, so the runtime keeps
/// running; on a blocking thread there is nothing to hand over and it is a
/// passthrough. Both paths are exercised by `sdk_bridge_test.rs`.
///
/// It requires a multi-threaded runtime, which is what the application and
/// every harness that reaches a client use. On a current-thread runtime there
/// is no sibling to hand the work to, so the bare call is the only thing
/// available -- correct from a blocking thread, and a panic from the worker,
/// which is the same architectural limit stated more honestly than a hang.
pub fn block_on<F: std::future::Future>(future: F) -> F::Output {
    let handle = tokio::runtime::Handle::current();
    match handle.runtime_flavor() {
        tokio::runtime::RuntimeFlavor::CurrentThread => handle.block_on(future),
        _ => tokio::task::block_in_place(|| handle.block_on(future)),
    }
}
