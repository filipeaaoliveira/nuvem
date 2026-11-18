use crate::app::effect::InteractiveCommand;
use crate::app::reducer::root::reduce;
use crate::app::state::configurations::{
    ConfigurationContextPickerItem, ConfigurationContextPickerKind,
};
use crate::app::state::logging::LOG_PAGE_LIMIT;
use crate::app::state::operations::OperationOutcome;
use crate::app::state::remembered_view::RememberedViews;
use crate::app::state::resources::{
    BatchAction, BatchRowRef, BigQueryDrillGeneration, BigQueryListingEpoch,
    CloudSqlInstanceAction, ComputeInstanceAction, ResourceLoadTicket,
};
use crate::app::state::root::Workspace;
use crate::app::state::system::OperationPhase;
use crate::app::{Action, AppError, AppState, Effect, EffectResult};
use crate::config::theme::Theme;
use crate::services::demo::DemoCommandRunner;
use crate::services::gcloud::cli::{
    CommandError, CommandRunner, GcloudCommand, ProcessCommandRunner,
};
use crate::services::gcloud::configurations::{
    GcloudConfigurationError, GcloudConfigurationService, GcloudContextAppliedProperties,
};
use crate::services::gcloud::discovery::{GcloudDiscoveryError, GcloudDiscoveryService};
use crate::services::resources::bigquery::{BigQueryLoadError, GcloudBigQueryService};
use crate::services::resources::cloud_run::{CloudRunLoadError, GcloudCloudRunService};
use crate::services::resources::cloudsql::GcloudCloudSqlService;
use crate::services::resources::compute::GcloudComputeService;
use crate::services::resources::disks::{DisksLoadError, GcloudDisksService};
use crate::services::resources::finops::GcloudFinOpsService;
use crate::services::resources::functions::{CloudFunctionsLoadError, GcloudFunctionsService};
use crate::services::resources::gke::GcloudGkeService;
use crate::services::resources::iam::{GcloudIamService, IamLoadError};
use crate::services::resources::logging::{GcloudLoggingService, LogLoadError};
use crate::services::resources::pubsub::{GcloudPubSubService, PubSubLoadError};
use crate::services::resources::secrets::{GcloudSecretsService, SecretsLoadError};
use crate::services::resources::storage::{GcloudStorageService, StorageLoadError};
use crate::services::resources::vpc::{GcloudVpcService, VpcLoadError};
use crate::services::runtime::clipboard::{ClipboardWriter, PlatformClipboardWriter};
use crate::services::runtime::log_export;
use crate::services::runtime::remembered_view_store::{
    FileRememberedViewStore, RememberedViewStore,
};
use crate::services::runtime::tunnel::port::{BindPortProbe, PortProbe, allocate_local_port};
use crate::services::runtime::tunnel::{
    DemoTunnelLauncher, LaunchedTunnel, ProcessTunnelLauncher, TunnelControl, TunnelId,
    TunnelLaunchError, TunnelLauncher, TunnelOutput, TunnelRegistry, TunnelShutdown,
};
use crate::{Cli, Config};
use chrono::{DateTime, Local};
use nuvem_domain::gcp::configuration::{
    GcloudConfiguration, GcloudConfigurationName, GcloudContext,
};
use nuvem_domain::gcp::k9s::CredentialStep;
use nuvem_domain::gcp::logging::{LogCursor, LogQuery, LogSeverity, LogTimeRange};
use nuvem_domain::gcp::resource::ResourceServiceKind;
use nuvem_domain::gcp::text::sanitize_display_text;
use nuvem_domain::gcp::tunnel::{
    TunnelKind, TunnelReadiness, TunnelSignal, TunnelSpec, classify_tunnel_line,
    cloud_sql_proxy_spec, iap_tunnel_spec, is_always_fatal_tunnel_line, readiness,
    ssh_local_forward_spec,
};
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tokio_util::sync::CancellationToken;

/// How long a managed tunnel may stay `Starting` before the supervisor gives up
/// on it. 20s is chosen against the slowest thing being waited for: a cold
/// `gcloud compute start-iap-tunnel` has to refresh credentials and open the
/// relay before it prints its banner, which is seconds on a warm cache and
/// noticeably longer on a cold one. Shorter would fail healthy tunnels; longer
/// would leave a wedged one spinning with no explanation.
const TUNNEL_READINESS_TIMEOUT: Duration = Duration::from_secs(20);

/// Grace every tunnel stop gives the child to exit on `SIGTERM` before it is
/// killed. Long enough for `gcloud`/`ssh` to tear a relay down cleanly, short
/// enough that the quit path (which spends this per still-running tunnel) does
/// not feel like a hang.
const TUNNEL_STOP_GRACE: Duration = Duration::from_secs(2);

/// How many unremarkable output lines the supervisor keeps for diagnostics. A
/// tunnel that times out never said anything the classifier recognised, so the
/// last few lines it *did* print are the only clue about what it was doing;
/// five is enough to be useful and small enough to stay bounded.
const TUNNEL_TAIL_LINES: usize = 5;

/// Longest tunnel failure message that reaches app state, in characters. Child
/// output is untrusted and unbounded, and the status line only has room for a
/// sentence. (The classifier bounds its own messages; this bounds the ones the
/// supervisor assembles.)
const MAX_TUNNEL_MESSAGE_CHARS: usize = 300;

/// The background writer behind [`Effect::SaveRememberedViews`], and the whole
/// of that fix.
///
/// `config::state::save` creates a temp file, fsyncs it, renames it and fsyncs
/// the directory. That is a fine price for a command the user asked for by
/// name — the arm's other writers, `SaveLogEntries` and `CopyToClipboard`, are
/// exactly that — but this effect fires on ordinary navigation: a service key,
/// a navigator open, a palette jump, an Alt-Left back. On a networked or
/// synced `$HOME` that put an fsync pair in the navigation path for something
/// the user never asked for, and gave them no way to attribute the stall.
///
/// Ordering is the constraint, and the shape is chosen for it. Each save
/// carries the *whole* map, so two saves landing out of order is a lost
/// update, not a merge — the loss mode `crate::config::state` bounds to
/// *across* processes would reappear inside one. A `tokio::spawn` per save
/// would do exactly that. Instead there is one channel and one consumer:
/// `UnboundedSender` preserves send order, and the consumer runs each save to
/// completion before taking the next, so writes reach the disk in dispatch
/// order by construction and the last write still wins. The results come back
/// on the session's existing result channel in that same order, which is what
/// keeps the `remembered_view_saves_suppressed` latch a single ordered
/// transition rather than a race between two failures.
///
/// It is a `spawn_blocking` thread rather than a `tokio::spawn` task: the work
/// is a blocking fsync, and it must never run on a reactor thread.
struct RememberedViewWriter {
    sender: mpsc::UnboundedSender<RememberedViews>,
    /// Awaited by [`RuntimeSession::flush_remembered_view_saves`] once the
    /// sender is dropped, so a save still in flight lands before the process
    /// exits. Without it, moving the write off the run loop would quietly
    /// downgrade the eager saving to "saved unless you quit".
    handle: tokio::task::JoinHandle<()>,
}

enum QueuedEffectResult {
    ContextPicker {
        generation: u64,
        result: EffectResult,
    },
    Background {
        context_generation: u64,
        operation_generation: u64,
        scope_generation: u64,
        status_generation: u64,
        /// The resource load this result belongs to, for list refreshes that
        /// took a service's load phase over. `None` for background log
        /// fetches, which have no resource load phase. Carried so the drop
        /// site can resolve the phase a detached fetch left behind — see
        /// [`RuntimeSession::process_next_effect_result`].
        load_ticket: Option<ResourceLoadTicket>,
        result: EffectResult,
    },
    Diagnostics {
        context_generation: u64,
        result: EffectResult,
    },
    Batch {
        context_generation: u64,
        scope_generation: u64,
        batch_generation: u64,
        result: EffectResult,
    },
    /// A plugin run's outcome.
    ///
    /// Unlike every other queued result, the generation it carries does not
    /// decide **delivery**: this result is the only thing that can settle the
    /// operations-log entry the run seeded, so dropping it would leave that
    /// entry Pending forever — the failure the lifecycle results' `operation_id`
    /// exists to avoid. It is always reduced, and the guards decide only
    /// whether it may take the status line over, in
    /// [`RuntimeSession::apply_plugin_effect_result`].
    ///
    /// Two guards, because a plugin run can be outlived by two different
    /// things. The context it carries answers "the operator switched
    /// configuration"; `operation_generation` answers "the operator started
    /// something else in the same context" — a refresh, a detail load, another
    /// action — which is the cross-batch supersession the generation rule pinned.
    /// A plugin both opens an operation and bumps that counter, so without this
    /// a slow plugin's result would stamp `Failed`/`"plugin"` over the
    /// in-flight refresh the operator is actually waiting on. The
    /// generation-less queued results (a remembered-view save, a tunnel) need
    /// neither guard for a reason that does not extend to a plugin: they return
    /// `None` from `effect_result_operation` and so never touch the status line
    /// at all.
    Plugin {
        operation_generation: u64,
        result: EffectResult,
    },
    /// One single-row lifecycle action's outcome.
    ///
    /// Delivered exactly like a plugin's and for the same reasons: it is the
    /// only thing that can settle the operations-log entry the action seeded,
    /// so it is always reduced, and `operation_generation` decides only whether
    /// it may take the status line over. A lifecycle action both opens an
    /// operation and bumps that counter, so without the guard a slow `stop`
    /// would stamp its outcome over the refresh the operator started after it.
    ///
    /// It exists as its own variant rather than reusing `Plugin` because the
    /// plugin arm also retires a handle from the plugin registry, and a
    /// lifecycle action has none.
    InstanceAction {
        operation_generation: u64,
        result: EffectResult,
    },
    /// A remembered-view save's outcome, carrying **no generation** for the
    /// same reason a tunnel's does not: the save is not scoped to a view. It
    /// records where the user *was*, so a result arriving after they have
    /// navigated on is still the truth about the write that happened, and
    /// dropping it would drop the failure latch with it.
    RememberedViewSave { result: EffectResult },
    /// One table's `bq show`, carrying **no generation** — it carries its own.
    ///
    /// Every enrichment result is stamped with the listing epoch it was fanned
    /// out from, and the reducer drops it unless that listing is still the one
    /// on screen. That is a finer guard than any counter here: the epoch moves
    /// when `r` re-lists inside the drill, which `operation_generation` and
    /// `scope_generation` cannot see.
    ///
    /// Passing these through the `Background` queue instead would drop them on
    /// the wrong signal. A fan-out over a large dataset is in flight for as
    /// long as its calls take, and *any* later operation — a poll tick, a
    /// detail load — bumps `operation_generation` and would silently discard
    /// every answer still outstanding, leaving those rows blank with nothing
    /// in flight to fill them. The status line is not at risk from the
    /// alternative: `effect_result_operation` answers `None` for both
    /// outcomes, so an enrichment can no more take the status line over than a
    /// remembered-view save can.
    BigQueryEnrichment { result: EffectResult },
    /// A managed tunnel's result, deliberately carrying **no generation**.
    ///
    /// Every other queued result is scoped to a view and is dropped when the
    /// view moves on. A tunnel is not a view: it is a child process that keeps
    /// running across context, workspace, and service switches (see
    /// `TunnelsState`), and it is addressed by an id that is never reused. A
    /// guard here could only drop the one message that would have corrected the
    /// row, stranding a live tunnel in `Starting` with no way to ever resolve
    /// it — so tunnel results are applied unconditionally.
    Tunnel { result: EffectResult },
}

/// How many per-table `bq show` calls may be in flight at once, across every
/// BigQuery enrichment fan-out in the session.
///
/// Four, the same default the marked-row batch runner uses, and chosen the
/// same way: enough that a listing of a few dozen tables fills in while the
/// operator is still reading it, few enough that entering a dataset does not
/// become a burst of processes against the machine and the API. A call past
/// the limit waits for a permit rather than being refused a slot outright --
/// but waiting is not a promise that it runs: a newer listing supersedes the
/// queue, and [`Session::dispatch_bigquery_table_stats`] drops what it finds
/// cancelled on either side of the permit.
///
/// The number is judgement rather than measurement. No fan-out has been run
/// against a live `bq`, so what four costs on a wide dataset is unmeasured.
const BIGQUERY_ENRICHMENT_CONCURRENCY: usize = 4;

/// Who got to one batch target first: nobody yet, its worker, or the
/// cancellation. The three states of [`BatchTargetClaim`].
const CLAIM_IDLE: u8 = 0;
const CLAIM_STARTED: u8 = 1;
const CLAIM_CANCELLED: u8 = 2;

/// The single decision of whether one batch target's gcloud command was ever
/// issued, claimed by whichever of the two racers gets there first.
///
/// The worker task and `detach_and_settle_batch` run on different threads of a
/// multi-thread runtime, so a flag the worker *sets* and the cancellation
/// *reads* would be two observations of a moving value: the worker can be
/// descheduled between clearing its cancellation check and setting the flag,
/// during which the cancellation sees an unset flag, reports `Cancelled` — "the
/// command did not run" — and the worker then wakes up and runs it. A few
/// instructions wide, but a thread can lose a whole timeslice inside it.
///
/// A compare-exchange removes the window by construction rather than by
/// timing: exactly one of the two claims can succeed, and the loser learns it
/// lost. The worker that loses must not issue its command; the cancellation
/// that loses must report `Unknown` rather than `Cancelled`. Nothing here leans
/// on the cancellation token's own ordering.
#[derive(Debug)]
struct BatchTargetClaim(AtomicU8);

impl BatchTargetClaim {
    fn new() -> Self {
        Self(AtomicU8::new(CLAIM_IDLE))
    }

    /// Claims this target for its worker. `false` means cancellation claimed it
    /// first and its entry already reads `Cancelled`, so the caller must issue
    /// no command — the only way to keep that entry true.
    fn claim_start(&self) -> bool {
        self.0
            .compare_exchange(
                CLAIM_IDLE,
                CLAIM_STARTED,
                Ordering::SeqCst,
                Ordering::SeqCst,
            )
            .is_ok()
    }

    /// Claims this target for the cancellation. `true` means no command was
    /// ever issued (`Cancelled`); `false` means the worker got there first and
    /// its command is on its way to GCP (`Unknown`).
    fn claim_cancel(&self) -> bool {
        self.0
            .compare_exchange(
                CLAIM_IDLE,
                CLAIM_CANCELLED,
                Ordering::SeqCst,
                Ordering::SeqCst,
            )
            .is_ok()
    }
}

/// One in-flight plugin run: the log entry it settles, and the single decision
/// of whether its command was ever issued.
///
/// The claim is [`BatchTargetClaim`] rather than a plugin-specific twin because
/// the race is identical — a worker on a `spawn_blocking` thread against a
/// cancellation on the reactor — and its compare-exchange argument is what
/// makes the answer binding rather than timing-dependent.
struct PluginRunHandle {
    operation_id: u64,
    claim: Arc<BatchTargetClaim>,
}

/// Why the in-flight batch is being detached, which decides only the wording of
/// the settled entries' details — never their outcomes.
///
/// The two outcomes are a function of what is provable about each target
/// (`Cancelled` for one that never issued its command, `Unknown` for one already
/// in flight) and are therefore identical on both paths. The agent is not: a
/// context or scope switch really is the operator cancelling the batch, while a
/// second confirmation cancels nothing from their point of view — they confirmed
/// a replacement, and the outgoing batch merely lost every way it had to report.
/// Telling them their batch "was cancelled" there names the wrong agent.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
enum BatchDetachReason {
    /// A context or scope switch took the batch away with nothing replacing it.
    Cancelled,
    /// A newer batch was confirmed over this one.
    Superseded,
}

impl BatchDetachReason {
    /// Detail for a target that never issued its gcloud command.
    fn unissued_detail(self) -> &'static str {
        match self {
            Self::Cancelled => "the batch was cancelled before this command ran",
            Self::Superseded => "the batch was superseded by a newer one before this command ran",
        }
    }

    /// Detail for a target whose gcloud command was already in flight.
    ///
    /// Both wordings name no key: they point at a refresh, and the refresh
    /// binding is rebindable (`RebindableAction::Refresh`), so a hardcoded key
    /// would be a second lie stacked on the uncertainty.
    fn issued_detail(self) -> &'static str {
        match self {
            Self::Cancelled => {
                "the batch was cancelled after this command was issued; refresh this service to see the resource's actual state"
            }
            Self::Superseded => {
                "the batch was superseded by a newer one after this command was issued; refresh this service to see the resource's actual state"
            }
        }
    }
}

/// One target of the in-flight batch, the operations-log entry it seeded, and
/// the claim that decides, once, whether its gcloud command was issued.
///
/// Per-target and lock-free on purpose: a shared set behind a mutex would be a
/// guard that could be held across an await point in the batch task.
///
/// The `operation_id` is held here rather than read back off the batch-progress
/// workflow because the workflow is not guaranteed to outlive the reducer: a
/// configuration switch tears it down (`mark_all_stale_and_clear`) whether or
/// not the context actually changed, so a cancellation that had to look the ids
/// up there could find nothing left and strand every entry `Pending` forever.
struct InFlightBatchTarget {
    operation_id: u64,
    claim: Arc<BatchTargetClaim>,
}

/// The gate every batch target passes before its gcloud command is issued:
/// cancellation check, permit, cancellation check again, claim. `None` means
/// this target must issue nothing; `Some` hands back the permit that has to be
/// held for as long as the command runs.
///
/// A named function rather than the opening lines of the spawned task so a unit
/// test can drive the ordering directly — the interleaving that matters cannot
/// be produced through the effect loop, see
/// `a_batch_target_cancelled_while_waiting_for_a_permit_never_claims`.
///
/// The claim's position is load-bearing: after the post-permit cancellation
/// check and before the command is issued. Earlier would claim a target that
/// then bails at the check and never runs anything, reporting `Unknown` for
/// work provably not done. Later — after `spawn_blocking`, or once the command
/// returns — leaves a window in which the command is in flight but unclaimed,
/// so a cancellation landing in that window would report `Cancelled` ("the
/// command did not run") while GCP is carrying it out.
///
/// The claim is also the second, and stricter, reason to bail: the check above
/// it can only see a cancellation that already happened, while losing the
/// compare-exchange means one landed in the instant since. Its entry then reads
/// `Cancelled`, and issuing the command anyway would make that a lie.
///
/// Every direction the claim could move is pinned. Too late is caught by
/// `in_flight_batch_target_is_never_reported_cancelled` in
/// tests/runtime_batch_effect_test.rs; before `acquire_owned()` by both
/// `cancelled_batch_*` tests and
/// `batch_entries_settle_when_the_workflow_was_torn_down_before_cancellation`,
/// which reject the `Unknown` a target parked at the permit then reads. The
/// narrow window between `acquire_owned()` returning and the check below is
/// pinned by the unit tests on this function, and only by them: the state that
/// distinguishes it — permit acquired, claim not yet reached — spans no await
/// point, so `detach_and_settle_batch`'s single synchronous read of the claim
/// cannot land inside it from outside the task. On the current-thread runtime
/// the two can only interleave at an await point, so through the effect loop
/// that interleaving has probability exactly zero, not merely small; on the
/// multi-thread runtime it is nanosecond-scale (200 racing iterations, 0
/// detections, measured). Driving this function under a hand-rolled poll is
/// what makes it deterministic instead.
/// The same shape for a fan-out's `bq show`, with no claim to win: nothing
/// reports a per-table show anywhere, so the only question is whether to spend
/// a permit on it.
///
/// Cancelled before the permit, and cancelled again after it. The second check
/// is the one that matters for the case this exists for: a wide dataset's
/// queue is hundreds of calls long, and every one of them was waiting here
/// when the operator left. Without it they would each take a permit and run,
/// putting the drill the operator is now looking at behind all of them.
async fn claim_bigquery_enrichment_slot(
    cancellation: &CancellationToken,
    semaphore: Arc<Semaphore>,
) -> Option<OwnedSemaphorePermit> {
    if cancellation.is_cancelled() {
        return None;
    }
    let permit = semaphore.acquire_owned().await.ok()?;
    if cancellation.is_cancelled() {
        return None;
    }
    Some(permit)
}

async fn claim_batch_target_slot(
    cancellation: CancellationToken,
    semaphore: Arc<Semaphore>,
    claim: &BatchTargetClaim,
) -> Option<OwnedSemaphorePermit> {
    // A batch detached by a context/service switch must not start new work;
    // not-yet-acquired targets bail out here.
    if cancellation.is_cancelled() {
        return None;
    }
    let permit = semaphore.acquire_owned().await.ok()?;
    if cancellation.is_cancelled() {
        return None;
    }
    if !claim.claim_start() {
        return None;
    }
    Some(permit)
}

pub struct RuntimeSession {
    app: AppState,
    config: Option<Config>,
    /// The parsed CLI, retained so a runtime config reload can re-resolve with
    /// CLI-flag precedence preserved. Absent for test sessions.
    cli: Option<Cli>,
    /// Set when a reload changed the poll interval; the run loop consumes it to
    /// rebuild its poll timer. See [`RuntimeSession::take_poll_interval_dirty`].
    poll_interval_dirty: bool,
    gcloud_runner: Option<Arc<dyn CommandRunner + Send + Sync>>,
    /// The native clients the migrated read paths use.
    ///
    /// `None` in demo mode, which is the whole reason the CLI implementations
    /// under `services/resources/` are still there: demo serves fixtures
    /// through the command runner, so the paths that read them cannot be
    /// deleted while `--demo` exists.
    sdk_clients: Option<Arc<crate::services::sdk::SdkClients>>,
    clipboard_writer: Option<Arc<dyn ClipboardWriter + Send + Sync>>,
    /// Where [`Effect::SaveRememberedViews`] writes. Absent in sessions with
    /// nothing to persist to (test harnesses), which is not an error — see the
    /// dispatch arm.
    remembered_view_store: Option<Arc<dyn RememberedViewStore + Send + Sync>>,
    /// The single background writer that owns every remembered-view save, and
    /// the reason they are ordered. Started on the first save rather than in
    /// the constructor, because a session can be built outside a Tokio runtime
    /// and `spawn_blocking` panics there; dispatch is always inside one.
    remembered_view_writer: Option<RememberedViewWriter>,
    effect_result_sender: mpsc::UnboundedSender<QueuedEffectResult>,
    effect_result_receiver: mpsc::UnboundedReceiver<QueuedEffectResult>,
    context_picker_generation: u64,
    context_picker_cancellation: CancellationToken,
    context_picker_slots: Arc<Semaphore>,
    diagnostics_slots: Arc<Semaphore>,
    context_generation: u64,
    operation_generation: u64,
    scope_generation: u64,
    batch_generation: u64,
    batch_cancellation: CancellationToken,
    /// Releases every in-flight lifecycle action on the way out of the process.
    ///
    /// Distinct from `batch_cancellation`, and deliberately never replaced. That
    /// one is scoped to *a* batch and is cancelled whenever one is detached — a
    /// context switch, a supersession — where an already-issued command is
    /// deliberately left to run and reported `Unknown`. This one is
    /// flipped exactly once, by [`RuntimeSession::cancel_lifecycle_actions_for_shutdown`],
    /// and it covers the single-row action path as well as the batch: both sit
    /// on a `spawn_blocking` thread that `Runtime::drop` waits for, and both
    /// carry `CommandCeiling::Lifecycle`, which is thirty minutes.
    lifecycle_cancellation: CancellationToken,
    /// The in-flight batch's targets and their claims. Replaced wholesale by
    /// every dispatch and taken on cancellation, so a previous batch's claims
    /// can never be consulted for the current one.
    batch_targets: Vec<InFlightBatchTarget>,
    /// Cancels every in-flight plugin run. Replaced on each cancellation, so a
    /// run started afterwards is not born cancelled.
    plugin_cancellation: CancellationToken,
    /// One handle per in-flight plugin run, taken by `cancel_running_plugins`.
    ///
    /// The operations-log ids alone would not do: the log is bounded and its
    /// entries can be evicted, and only the claim can answer whether a run's
    /// command was ever issued.
    plugin_runs: Vec<PluginRunHandle>,
    batch_concurrency: usize,
    /// How many per-table `bq show` calls may be running at once, shared by
    /// every enrichment task of every fan-out.
    ///
    /// Session-wide rather than per fan-out, which is the point: a listing can
    /// put up to `RESOURCE_LIST_LIMIT` rows on screen, and one spawned process
    /// per row would be that many `bq` invocations racing at once. The queue
    /// behind these permits is what turns a wide fan-out into a slow one
    /// instead of a stampede, and
    /// [`RuntimeSession::supersede_bigquery_enrichments`] is what keeps the
    /// queue from being several fan-outs deep.
    bigquery_enrichment_slots: Arc<Semaphore>,
    /// Cancels the fan-out of the listing a newer listing has replaced.
    ///
    /// Replaced whenever `Effect::LoadBigQueryTables` is dispatched, so it
    /// covers both ways a fan-out is superseded: entering another dataset, and
    /// `r`. The shows queued behind the permits bail instead of running, which
    /// is what stops a wide abandoned dataset from holding the live drill's
    /// rows blank behind up to `RESOURCE_LIST_LIMIT` calls it no longer wants.
    ///
    /// It cannot stop the calls already inside `spawn_blocking` — a spawned
    /// `bq` is not interruptible from here — so at most
    /// `BIGQUERY_ENRICHMENT_CONCURRENCY` of them still run to completion. Their
    /// answers are dropped by the reducer's scope check, as they always were:
    /// this is about the machine and the queue, not about correctness.
    bigquery_enrichment_cancellation: CancellationToken,
    pending_background_refreshes: usize,
    auto_loaded_scope: Option<(ResourceServiceKind, GcloudContext)>,
    pending_interactive_command: Option<InteractiveCommand>,
    kubectl_runner: Option<Arc<dyn CommandRunner + Send + Sync>>,
    k9s_binary: String,
    /// The active-service scope whose most recent load failure was
    /// `Retryability::Persistent`, if any. Consulted by `poll_active_resource`
    /// to stop auto-hammering gcloud on auth/permission/config errors. Cleared
    /// on manual refresh (`ResourcesAction::Refresh`) and on context/scope
    /// change, both of which always proceed regardless of this gate.
    gated_scope: Option<(ResourceServiceKind, GcloudContext)>,
    /// Every live tunnel's control handle. Held as a field so the registry's
    /// own destructor is reached when the session goes away — the last backstop
    /// against leaking a tunnel child (see the `tunnel` module).
    tunnels: TunnelRegistry,
    tunnel_launcher: Box<dyn TunnelLauncher>,
    port_probe: Box<dyn PortProbe>,
    /// How long a tunnel may stay `Starting`. A field rather than the constant
    /// so tests can pin the timeout path in milliseconds instead of seconds.
    readiness_timeout: Duration,
    /// The config problems from the most recent resolve, before the deferred
    /// `[theme.overrides]` check adds to them.
    ///
    /// Retained because `SystemState::set_config_problems` replaces the list
    /// wholesale: the deferred check re-runs whenever the known configurations
    /// change, so it has to rebuild `base + deferred` rather than append, or
    /// repeated loads would stack duplicate warnings.
    base_config_problems: Vec<String>,
}

impl RuntimeSession {
    /// `state_path` is derived once, by
    /// [`crate::services::runtime::load_remembered_views`], and handed here
    /// rather than derived again: the session must write back to the very file
    /// startup read, and a second derivation is a place the two can drift.
    pub fn new(config: Config, cli: Cli, app: AppState, state_path: PathBuf) -> Self {
        // Demo mode swaps the live gcloud runner for one that serves
        // embedded JSON fixtures, so every command renders offline. That swap,
        // the fixture-backed client cache below it, and the withheld
        // remembered-view store below that are the three demo-aware seams in
        // the session, and `load_remembered_views` withholds the matching
        // read; services, reducers, and views are unchanged.
        let runner: Arc<dyn CommandRunner + Send + Sync> = if config.demo {
            Arc::new(DemoCommandRunner::new())
        } else {
            Arc::new(ProcessCommandRunner::with_binaries(
                config.gcloud_binary.clone(),
                config.bq_binary.clone(),
            ))
        };
        // Demo mode reads through the native clients too, built from stubs
        // that serve the same embedded fixtures: a demo session and a live one
        // now differ in where the bytes come from, not in which code path
        // reads them. A fixture client holds a stub where a transport would
        // be, so it is offline by construction rather than by a flag.
        //
        // The runner above stays demo's runner all the same. Everything with
        // no native client behind it still travels as argv: BigQuery (`bq`,
        // because BigQuery has no GA gcloud surface), FinOps budgets (Cloud
        // Billing publishes no Rust client; the recommender calls beside them
        // do read natively, and take the arm above), gcloud's own
        // configuration and auth state, and every label-filtered listing,
        // which the native filter cannot express and which falls back to the
        // pushed gcloud filter.
        let sdk_clients = Some(Arc::new(if config.demo {
            crate::services::sdk::SdkClients::fixtures()
        } else {
            crate::services::sdk::SdkClients::new(Arc::clone(&runner))
        }));
        // Demo mode is the one session that must not persist: its
        // configurations come from the fixtures, which are named `dev` and
        // `prod` like real ones, so saving would write invented entries over
        // the user's actual remembered views. No store is the whole mechanism —
        // the dispatch arm already treats an absent store as nothing to do.
        let remembered_view_store: Option<Arc<dyn RememberedViewStore + Send + Sync>> =
            if config.demo {
                None
            } else {
                Some(Arc::new(FileRememberedViewStore::new(state_path)))
            };
        let mut session = Self::with_runner(
            app,
            Some(config),
            Some(cli),
            Some(runner),
            Some(Arc::new(PlatformClipboardWriter)),
            remembered_view_store,
        );
        // Set here rather than threaded through `with_runner`: every other
        // constructor is a test harness driving fixtures through a fake
        // runner, and none of them should reach a real service.
        session.sdk_clients = sdk_clients;
        session
    }

    pub fn new_for_test(app: AppState) -> Self {
        Self::with_runner(app, None, None, None, None, None)
    }

    pub fn new_with_runner_for_test(
        app: AppState,
        runner: Box<dyn CommandRunner + Send + Sync>,
    ) -> Self {
        Self::with_runner(app, None, None, Some(Arc::from(runner)), None, None)
    }

    pub fn new_with_runner_and_clipboard_for_test(
        app: AppState,
        runner: Box<dyn CommandRunner + Send + Sync>,
        clipboard: Box<dyn ClipboardWriter + Send + Sync>,
    ) -> Self {
        Self::with_runner(
            app,
            None,
            None,
            Some(Arc::from(runner)),
            Some(Arc::from(clipboard)),
            None,
        )
    }

    /// Test constructor that injects the remembered-view store, so the
    /// reducer-to-runtime save path can be exercised without a filesystem.
    pub fn new_with_remembered_view_store_for_test(
        config: Config,
        app: AppState,
        store: Arc<dyn RememberedViewStore + Send + Sync>,
    ) -> Self {
        Self::with_runner(app, Some(config), None, None, None, Some(store))
    }

    /// Test constructor that stores the resolved config but no runner, for
    /// tests that drive `apply_effect_result` directly and never dispatch an
    /// effect (the theme-override tests).
    pub fn new_with_config_for_test(config: Config, app: AppState) -> Self {
        Self::with_runner(app, Some(config), None, None, None, None)
    }

    /// The same session, holding native clients.
    ///
    /// `new_with_config_and_runner_for_test` leaves `sdk_clients` as `None`,
    /// which is what made every native branch invisible to the suite. A test
    /// that wants the native path takes this and hands it clients built from
    /// stubs.
    pub fn new_with_sdk_clients_for_test(
        config: Config,
        app: AppState,
        runner: Box<dyn CommandRunner + Send + Sync>,
        clients: Arc<crate::services::sdk::SdkClients>,
    ) -> Self {
        let mut session =
            Self::with_runner(app, Some(config), None, Some(Arc::from(runner)), None, None);
        session.sdk_clients = Some(clients);
        session
    }

    /// Whether this session reads through native clients at all.
    ///
    /// Exposed for the pin on an ordinary session: that it builds clients is a
    /// property of how `new` is written, and a property nothing asserts is one
    /// a refactor can drop without anything going red. It no longer separates
    /// demo from live — both hold clients now — so ask
    /// [`Self::native_client_source`] for that.
    pub fn holds_native_clients(&self) -> bool {
        self.sdk_clients.is_some()
    }

    /// Where this session's native clients get their answers, or `None` when
    /// it holds none (the test harnesses).
    ///
    /// [`crate::services::sdk::ClientSource::Fixtures`] is the whole of demo
    /// mode's offline guarantee at this level: such a cache builds every
    /// client `from_stub`, which takes no credentials and no transport.
    pub fn native_client_source(&self) -> Option<crate::services::sdk::ClientSource> {
        self.sdk_clients.as_deref().map(|clients| clients.source())
    }

    pub fn new_with_config_and_runner_for_test(
        config: Config,
        app: AppState,
        runner: Box<dyn CommandRunner + Send + Sync>,
    ) -> Self {
        Self::with_runner(app, Some(config), None, Some(Arc::from(runner)), None, None)
    }

    /// Test constructor that also stores the parsed [`Cli`], enabling
    /// [`Effect::ReloadConfig`] to re-resolve the config from a temp file.
    pub fn new_with_cli_and_config_for_test(cli: Cli, config: Config, app: AppState) -> Self {
        Self::with_runner(app, Some(config), Some(cli), None, None, None)
    }

    fn with_runner(
        app: AppState,
        config: Option<Config>,
        cli: Option<Cli>,
        runner: Option<Arc<dyn CommandRunner + Send + Sync>>,
        clipboard_writer: Option<Arc<dyn ClipboardWriter + Send + Sync>>,
        remembered_view_store: Option<Arc<dyn RememberedViewStore + Send + Sync>>,
    ) -> Self {
        let (effect_result_sender, effect_result_receiver) = mpsc::unbounded_channel();
        let kubectl_runner: Option<Arc<dyn CommandRunner + Send + Sync>> = config
            .as_ref()
            // kubectl reuses the same runner type, so the `gcloud` slot holds
            // the kubectl path and the `bq` slot keeps its `bq` default. Only
            // kubectl commands are ever submitted to this instance and they all
            // carry `CommandProgram::Gcloud`, so the `bq` slot is unused; a
            // future kubectl command must not select `CommandProgram::Bq`,
            // which would spawn the real `bq` from this runner.
            .map(|config| Arc::new(ProcessCommandRunner::new(config.kubectl_binary.clone())) as _);
        let k9s_binary = config
            .as_ref()
            .map(|config| config.k9s_binary.to_string_lossy().into_owned())
            .unwrap_or_else(|| "k9s".to_string());
        let batch_concurrency = config
            .as_ref()
            .map(|config| config.batch_concurrency)
            .unwrap_or(4)
            .max(1);
        // Demo mode swaps the tunnel launcher the same way it swaps the gcloud
        // runner: the demo tunnel behaves like a healthy child without
        // spawning one, so the workspace renders offline.
        let tunnel_launcher: Box<dyn TunnelLauncher> =
            if config.as_ref().is_some_and(|config| config.demo) {
                Box::new(DemoTunnelLauncher)
            } else {
                Box::new(ProcessTunnelLauncher)
            };
        Self {
            app,
            config,
            cli,
            poll_interval_dirty: false,
            gcloud_runner: runner,
            sdk_clients: None,
            clipboard_writer,
            remembered_view_store,
            remembered_view_writer: None,
            effect_result_sender,
            effect_result_receiver,
            context_picker_generation: 0,
            context_picker_cancellation: CancellationToken::new(),
            context_picker_slots: Arc::new(Semaphore::new(1)),
            diagnostics_slots: Arc::new(Semaphore::new(1)),
            context_generation: 0,
            operation_generation: 0,
            scope_generation: 0,
            batch_generation: 0,
            batch_cancellation: CancellationToken::new(),
            lifecycle_cancellation: CancellationToken::new(),
            batch_targets: Vec::new(),
            plugin_cancellation: CancellationToken::new(),
            plugin_runs: Vec::new(),
            batch_concurrency,
            bigquery_enrichment_slots: Arc::new(Semaphore::new(BIGQUERY_ENRICHMENT_CONCURRENCY)),
            bigquery_enrichment_cancellation: CancellationToken::new(),
            pending_background_refreshes: 0,
            auto_loaded_scope: None,
            pending_interactive_command: None,
            kubectl_runner,
            k9s_binary,
            gated_scope: None,
            tunnels: TunnelRegistry::new(),
            tunnel_launcher,
            port_probe: Box::new(BindPortProbe),
            readiness_timeout: TUNNEL_READINESS_TIMEOUT,
            base_config_problems: Vec::new(),
        }
    }

    /// Install the config problems from the most recent resolve. Re-runs the
    /// deferred `[theme.overrides]` check so the list on display is always
    /// `base + deferred`.
    pub fn set_base_config_problems(&mut self, problems: Vec<String>) {
        self.base_config_problems = problems;
        self.refresh_config_problems();
    }

    /// Apply the active configuration's frame override, if it has one, over
    /// the configured base theme.
    fn refresh_theme(&mut self) {
        let Some(config) = self.config.as_ref() else {
            return;
        };
        let base = Theme::for_choice(config.theme);
        // The whole active configuration rather than only its name:
        // the project it points at is the second key an override may name,
        // and it is the one that is the same string for everyone who can
        // reach it.
        let themed = match self.app.root.configurations.active() {
            Some(active) => {
                let color = config
                    .theme_overrides
                    .border_color_for(active.name.as_str(), active.project.as_deref());
                match color {
                    Some(color) => base.with_border_color(color),
                    None => base,
                }
            }
            None => base,
        };
        crate::config::theme::set_active(themed);
    }

    /// Rebuild the displayed config problems as `base + deferred`.
    ///
    /// The deferred part is the check `ThemeOverrides::from_file` structurally
    /// cannot make: whether an override names a gcloud configuration that
    /// actually exists. Silence here would be the feature failing exactly as
    /// it is meant to prevent — the user wrote `prod = "red"` so production
    /// would be unmistakable, and a typo would leave them looking at an
    /// ordinary frame believing they were guarded.
    ///
    /// Only runs once the configurations are known: with none loaded, every
    /// override would look unknown and warn spuriously at startup.
    fn refresh_config_problems(&mut self) {
        let mut problems = self.base_config_problems.clone();

        let known: Vec<&str> = self
            .app
            .root
            .configurations
            .rows()
            .iter()
            .map(|row| row.name.as_str())
            .collect();

        // The projects those configurations point at, for the project-override half of
        // the check. A configuration with no project set contributes nothing
        // rather than an empty id.
        let targeted: Vec<&str> = self
            .app
            .root
            .configurations
            .rows()
            .iter()
            .filter_map(|row| row.project.as_deref())
            .collect();

        if let Some(config) = self.config.as_ref()
            && !known.is_empty()
        {
            for name in config.theme_overrides.configurations() {
                if !known.contains(&name) {
                    problems.push(format!(
                        "theme.overrides.{name}: no gcloud configuration named {name:?}; \
                             this override will never apply"
                    ));
                }
            }
            // Same silent no-op, one key over: an id no configuration
            // reaches leaves the frame ordinary while the operator
            // believes it is guarded. Guarded by `known` rather than by
            // `targeted`, which is legitimately empty when no
            // configuration has a project set.
            for project in config.theme_overrides.projects() {
                if !targeted.contains(&project) {
                    problems.push(format!(
                        "theme.project_overrides.{project}: no gcloud configuration targets \
                             project {project:?}; this override will never apply"
                    ));
                }
            }
        }

        self.app.root.system.set_config_problems(problems);
    }

    /// Test constructor injecting the managed-tunnel seams: a fake launcher, a
    /// fake port probe, and a readiness deadline short enough to pin the
    /// timeout path without spending the production 20 seconds.
    pub fn new_with_tunnel_runtime_for_test(
        app: AppState,
        launcher: Box<dyn TunnelLauncher>,
        probe: Box<dyn PortProbe>,
        readiness_timeout: Duration,
    ) -> Self {
        let mut session = Self::with_runner(app, None, None, None, None, None);
        session.tunnel_launcher = launcher;
        session.port_probe = probe;
        session.readiness_timeout = readiness_timeout;
        session
    }

    /// Test constructor injecting fake gcloud + kubectl runners for the k9s
    /// prepare/credentials flow.
    pub fn new_with_k9s_runners_for_test(
        config: Config,
        app: AppState,
        gcloud: Box<dyn CommandRunner + Send + Sync>,
        kubectl: Box<dyn CommandRunner + Send + Sync>,
    ) -> Self {
        let mut session =
            Self::with_runner(app, Some(config), None, Some(Arc::from(gcloud)), None, None);
        session.kubectl_runner = Some(Arc::from(kubectl));
        session
    }

    pub fn app(&self) -> &AppState {
        &self.app
    }

    /// Mutable access to the app state, used by the runtime's startup
    /// wiring to surface config warnings in the status line once
    /// `initialize()` has settled (its `ConfigurationsLoaded` /
    /// `ConfigurationActivated` effect results clear any status message set
    /// beforehand).
    /// How many spawned effect tasks could still deliver a result.
    ///
    /// Every task that will send on the effect-result channel holds a clone of
    /// the sender for as long as it runs, and nothing else holds one, so this
    /// is the number of tasks alive: zero means nothing more can arrive and a
    /// drain of the channel is complete. A test harness reads this to know
    /// when a session is settled rather than guessing from a quiet interval,
    /// which under load guesses wrong.
    pub fn effect_tasks_in_flight(&self) -> usize {
        self.effect_result_sender.strong_count().saturating_sub(1)
    }

    pub fn app_mut(&mut self) -> &mut AppState {
        &mut self.app
    }

    pub async fn initialize(&mut self) {
        let effect = match self.config.as_ref().and_then(|config| {
            config
                .starting_configuration
                .as_ref()
                .map(|name| GcloudConfigurationName::new(name.clone()))
        }) {
            Some(name) => Effect::ActivateConfiguration { name },
            None => Effect::LoadConfigurations,
        };
        self.dispatch_action(Action::Effect(effect)).await;
    }

    pub async fn dispatch_action(&mut self, action: Action) -> bool {
        if matches!(action, Action::QuitRequested) {
            self.cancel_context_picker();
            // A tunnel child outlives the command that started it, so quit is
            // the last moment the app can still stop one. The registry's own
            // `Drop` is a backstop, not the plan: doing it here means the
            // shutdown runs off the reactor with a bounded grace, and the
            // process does not exit while children are still being signalled.
            self.shutdown_all_tunnels().await;
            // The last navigation's save may still be with the writer. Quit is
            // the last moment it can still be waited for, and eager saving is
            // the property chosen here; see [`RememberedViewWriter`].
            self.flush_remembered_view_saves().await;
            self.cancel_plugins_for_shutdown();
            return false;
        }
        if matches!(
            action,
            Action::Configurations(
                crate::app::ConfigurationsAction::CancelContextPicker
                    | crate::app::ConfigurationsAction::RetryContextPicker
            )
        ) {
            self.cancel_context_picker();
        }
        if matches!(
            action,
            Action::Resources(crate::app::ResourcesAction::Refresh)
        ) {
            // Manual refresh always proceeds and clears the retry gate.
            self.gated_scope = None;
        }
        let context_picker_was_open = self.app.root.configurations.context_picker().is_some();
        let effects = self.reduce(&action);
        if context_picker_was_open && self.app.root.configurations.context_picker().is_none() {
            self.cancel_context_picker();
        }
        self.dispatch_effects(effects, false).await;
        self.ensure_active_resource_loaded().await;
        true
    }

    pub async fn process_next_effect_result(&mut self) -> bool {
        let Some(queued_result) = self.effect_result_receiver.recv().await else {
            return false;
        };
        match queued_result {
            QueuedEffectResult::ContextPicker { generation, result } => {
                if generation == self.context_picker_generation {
                    self.apply_effect_result(result).await;
                }
            }
            QueuedEffectResult::Background {
                context_generation,
                operation_generation,
                scope_generation,
                status_generation,
                load_ticket,
                result,
            } => {
                self.pending_background_refreshes =
                    self.pending_background_refreshes.saturating_sub(1);
                if context_generation == self.context_generation
                    && operation_generation == self.operation_generation
                    && scope_generation == self.scope_generation
                {
                    self.apply_background_effect_result(result, status_generation)
                        .await;
                } else if let Some(load_ticket) = load_ticket {
                    // The result lost the guard above, so it is dropped. Any of
                    // the three dimensions can do that: `context_generation` (a
                    // context switch), `scope_generation` (a workspace or
                    // active-service switch), and `operation_generation` (a
                    // later batch carrying an effect that opens a status-bar
                    // operation — list refreshes, detail loads, log fetches,
                    // instance/SQL actions and batches all included).
                    // `operation_generation` is stamped once per
                    // `dispatch_effects` batch, so this fetch's own batch can
                    // never drop it. This is the only place that learns the
                    // fetch is over, so it must also resolve the `Loading`
                    // phase the fetch installed — otherwise nothing does:
                    // `ensure_active_resource_loaded` (below) bails outside the
                    // Resources workspace and leaves `auto_loaded_scope`
                    // untouched, so no refresh is rescheduled either and the
                    // spinner ticks forever.
                    //
                    // Deliberately here and NOT in the per-service context-clear
                    // helpers (`mark_stale_and_clear`, `mark_gke_stale_and_clear`,
                    // ...) nor in `reduce`'s context/scope-change block: at
                    // switch time the fetch is still genuinely in flight, so
                    // `Loading` is the honest state. A blanket reset there
                    // suppresses the Loading affordance a fresh fetch must show
                    // right after a context change (pinned by the
                    // `resources_view_shows_loading_state_*` UI projection tests).
                    //
                    // Keyed by the ticket, so a late drop can only clear the load
                    // it started: any newer `begin_load` stamps a new sequence and
                    // makes this a no-op. `ensure_active_resource_loaded()` runs
                    // after this, so a warranted reschedule still wins.
                    //
                    // Resolving the spinner is only half of it. The fetch the
                    // user was owed never landed, so the reschedule has to be
                    // re-armed too: `auto_loaded_scope` still records
                    // the scope this fetch was started for, and returning to
                    // the Resources workspace would early-return on its
                    // `== scope` check and schedule nothing, leaving empty rows
                    // and the press-r prompt where a refetch belongs. Forget
                    // that record so `ensure_active_resource_loaded()` — which
                    // runs right after this — re-evaluates.
                    //
                    // Only when `abandon_load` reports that this ticket still
                    // owned the phase: otherwise a newer load already owns it
                    // and is responsible for its own record.
                    //
                    // The service match is a conservative guard, not an active
                    // protection: today it cannot actually fail. The only write
                    // to `auto_loaded_scope` is in `ensure_active_resource_loaded`,
                    // past its `pending_background_refreshes != 0` early return,
                    // and that counter stays non-zero for as long as this ticket
                    // is outstanding — so the record cannot be rewritten between
                    // this fetch being scheduled and its result landing here. It
                    // is kept because it is one comparison and it keeps the clear
                    // scoped to the fetch that earned it if that invariant ever
                    // loosens.
                    //
                    // Service-only rather than the full `(service, context)`
                    // scope because a full match would need new plumbing —
                    // `ResourceLoadTicket` carries no context — for no
                    // behavioural difference: a record for the same service but
                    // a different context is already unequal to the current
                    // scope, so `ensure_active_resource_loaded` reschedules
                    // whether or not we cleared it. Clearing is redundant there,
                    // never harmful, and `needs_refresh` still decides.
                    //
                    // This cannot loop: `ensure_active_resource_loaded` still
                    // requires `needs_refresh`, still defers while
                    // `pending_background_refreshes != 0` *without* recording
                    // the scope, and a dropped result never reaches the apply
                    // site, so the persistent-failure path (which is drop-free)
                    // keeps its `gated_scope` and its recorded scope.
                    let ticket_kind = load_ticket.kind();
                    if self.app.root.resources.abandon_load(load_ticket)
                        && self
                            .auto_loaded_scope
                            .as_ref()
                            .is_some_and(|(service, _)| *service == ticket_kind)
                    {
                        self.auto_loaded_scope = None;
                    }
                }
            }
            QueuedEffectResult::Diagnostics {
                context_generation,
                result,
            } => {
                if context_generation == self.context_generation {
                    let effects = self.reduce(&Action::EffectResult(result));
                    self.dispatch_effects(effects, true).await;
                } else {
                    // The probe result belongs to a superseded context; it
                    // must be dropped, but the in-flight guard still needs
                    // to be released so the next probe can run.
                    self.app.root.system.diagnostics.abandon();
                }
            }
            QueuedEffectResult::Batch {
                context_generation,
                scope_generation,
                batch_generation,
                result,
            } => {
                // Drop results from a superseded batch or a context/service
                // switch. `batch_generation` is bumped both when a new batch
                // starts and whenever the context or scope changes (see
                // `reduce`), so a stale target never mutates the new context.
                if context_generation == self.context_generation
                    && scope_generation == self.scope_generation
                    && batch_generation == self.batch_generation
                {
                    self.apply_batch_effect_result(result).await;
                }
            }
            // Applied unconditionally: a tunnel is addressed by id, not by the
            // active view, so none of the generation guards apply to it. See
            // `QueuedEffectResult::Tunnel`.
            QueuedEffectResult::Tunnel { result } => {
                self.apply_effect_result(result).await;
            }
            // Delivered unconditionally; the guards decide only whether it may
            // touch the status line. See `QueuedEffectResult::Plugin`.
            QueuedEffectResult::Plugin {
                operation_generation,
                result,
            } => {
                self.apply_plugin_effect_result(result, operation_generation)
                    .await;
            }
            // Delivered unconditionally, like a plugin's, and for the same
            // reason: nothing else can settle the operations-log entry this
            // action seeded. See `QueuedEffectResult::InstanceAction`.
            QueuedEffectResult::InstanceAction {
                operation_generation,
                result,
            } => {
                self.apply_operation_effect_result(result, operation_generation)
                    .await;
            }
            // Applied unconditionally, like a tunnel's: the save is not scoped
            // to a view, so no generation guard applies. They arrive in write
            // order and are applied one at a time, which is what keeps the
            // failure latch a single ordered transition.
            QueuedEffectResult::RememberedViewSave { result } => {
                self.apply_effect_result(result).await;
            }
            // Applied unconditionally: the result carries the listing epoch it
            // belongs to and the reducer checks it. See
            // `QueuedEffectResult::BigQueryEnrichment`.
            QueuedEffectResult::BigQueryEnrichment { result } => {
                self.apply_effect_result(result).await;
            }
        }
        self.ensure_active_resource_loaded().await;
        true
    }

    async fn dispatch_effects(&mut self, effects: Vec<Effect>, preserve_status: bool) {
        let mut pending = effects
            .into_iter()
            .map(|effect| (effect, preserve_status))
            .collect::<VecDeque<_>>();
        let mut operation_generation_bumped = false;
        while let Some((effect, preserve_status)) = pending.pop_front() {
            if let Some((phase, label)) = effect_operation(&effect) {
                // At most one bump per `dispatch_effects` call, so every effect
                // in this batch — follow-ups `push_front`ed below included —
                // captures the same generation and no batch can invalidate its
                // own later effects. `ResourcesAction::Refresh` with a detail
                // view open emits a list refresh *and* a detail load; with a
                // bump per effect the detail load superseded the list fetch its
                // own batch had just started, so those rows were always dropped.
                // Cross-batch supersession is what this counter
                // exists for, and it survives: the next `dispatch_effects` call
                // bumps again, so a newer list refresh still drops an older
                // in-flight one. Bumped lazily on the first effect that opens an
                // operation rather than unconditionally up front, so a batch
                // that opens none still leaves in-flight fetches alone. The
                // operation itself still begins per effect — only the
                // supersession stamp is batch-wide. That laziness is
                // load-bearing and pinned: hoisting this to the top of
                // `dispatch_effects` turns every clipboard copy, log export or
                // tunnel start into a supersession event that drops the list
                // fetch the user was owed, which
                // `a_batch_that_opens_no_operation_does_not_supersede_an_in_flight_fetch`
                // in `tests/generation_test.rs` fails on.
                //
                // This is the only supersession stamp a background result
                // carries besides context and scope. A second counter,
                // `resource_generation`, used to be bumped here for a
                // hand-maintained list of resource effects; every effect on that
                // list also opens an operation, so it could only ever drop
                // results this stamp had already dropped. Whether that
                // stays true is a question about `effect_operation`: a new
                // `Effect` variant cannot be added without answering it, because
                // that match lists every variant explicitly with no `_` arm and
                // fails to compile until the new one is placed. Nothing checks
                // that the *answer* is right, so a variant whose result must
                // supersede in-flight background fetches has to be given an
                // operation there — not a counter of its own.
                if !operation_generation_bumped {
                    self.operation_generation = self.operation_generation.wrapping_add(1);
                    operation_generation_bumped = true;
                }
                if preserve_status {
                    self.app.root.system.begin_follow_up_operation(phase, label);
                } else {
                    self.app.root.system.begin_operation(phase, label);
                }
            }
            // A new tables listing supersedes the fan-out of the one it
            // replaces, before the listing itself is dispatched. `Esc` is not
            // a listing and so does not reach here: leaving a dataset without
            // entering another leaves that fan-out queued, which costs the
            // machine but nothing the operator is looking at. Entering the
            // next dataset — the case where the queue is in the way — cancels
            // it.
            if matches!(effect, Effect::LoadBigQueryTables { .. }) {
                self.supersede_bigquery_enrichments();
            }
            match effect {
                Effect::LoadContextPicker { kind, context } => {
                    self.dispatch_context_picker(kind, context);
                }
                Effect::RefreshComputeInstances { context, filter } => {
                    self.dispatch_background_refresh(
                        ResourceServiceKind::ComputeInstances,
                        context,
                        filter,
                    );
                }
                Effect::RefreshGkeClusters { context, filter } => {
                    self.dispatch_background_refresh(
                        ResourceServiceKind::GkeClusters,
                        context,
                        filter,
                    );
                }
                Effect::RefreshCloudRunServices { context } => {
                    self.dispatch_background_refresh(
                        ResourceServiceKind::CloudRunServices,
                        context,
                        None,
                    );
                }
                Effect::RefreshCloudSqlInstances { context, filter } => {
                    self.dispatch_background_refresh(
                        ResourceServiceKind::CloudSql,
                        context,
                        filter,
                    );
                }
                Effect::RefreshStorageBuckets { context } => {
                    self.dispatch_background_refresh(
                        ResourceServiceKind::CloudStorage,
                        context,
                        None,
                    );
                }
                Effect::RefreshSecrets { context, filter } => {
                    self.dispatch_background_refresh(
                        ResourceServiceKind::SecretManagerSecrets,
                        context,
                        filter,
                    );
                }
                Effect::RefreshCloudFunctions { context, filter } => {
                    self.dispatch_background_refresh(
                        ResourceServiceKind::CloudFunctions,
                        context,
                        filter,
                    );
                }
                Effect::RefreshPersistentDisks { context, filter } => {
                    self.dispatch_background_refresh(
                        ResourceServiceKind::PersistentDisks,
                        context,
                        filter,
                    );
                }
                Effect::RefreshPubSubTopics { context, filter } => {
                    self.dispatch_background_refresh(
                        ResourceServiceKind::PubSubTopics,
                        context,
                        filter,
                    );
                }
                Effect::RefreshVpcNetworks { context } => {
                    self.dispatch_background_refresh(
                        ResourceServiceKind::VpcNetworks,
                        context,
                        None,
                    );
                }
                Effect::RefreshFirewallRules { context } => {
                    self.dispatch_background_refresh(
                        ResourceServiceKind::FirewallRules,
                        context,
                        None,
                    );
                }
                Effect::RefreshVpcSubnets { context } => {
                    self.dispatch_background_refresh(
                        ResourceServiceKind::VpcSubnets,
                        context,
                        None,
                    );
                }
                Effect::RefreshPubSubSubscriptions { context, filter } => {
                    self.dispatch_background_refresh(
                        ResourceServiceKind::PubSubSubscriptions,
                        context,
                        filter,
                    );
                }
                // No filter to forward: `bq ls` accepts no server-side
                // filter expression, so the datasets table narrows wholly
                // client-side.
                Effect::RefreshBigQueryDatasets { context } => {
                    self.dispatch_background_refresh(
                        ResourceServiceKind::BigQueryDatasets,
                        context,
                        None,
                    );
                }
                Effect::RefreshServiceAccounts { context, filter } => {
                    self.dispatch_background_refresh(
                        ResourceServiceKind::IamServiceAccounts,
                        context,
                        filter,
                    );
                }
                Effect::RefreshIamPolicy { context } => {
                    self.dispatch_background_refresh(ResourceServiceKind::IamPolicy, context, None);
                }
                Effect::ProbeGcloudDiagnostics { configuration } => {
                    self.dispatch_diagnostics_probe(configuration);
                }
                Effect::LaunchInteractiveProcess { command } => {
                    // The run loop owns the terminal; stash the request and let
                    // it perform the suspend/launch/restore. Any queued
                    // background results stay in the channel and are drained
                    // afterward (generation guards drop stale ones).
                    self.pending_interactive_command = Some(command);
                }
                Effect::LoadRecentLogs { .. } | Effect::TailLogs { .. } => {
                    self.dispatch_background_logs(effect);
                }
                // Off the run loop, unlike the listing it follows. One landed
                // listing issues one of these per row, and the run loop awaits
                // the arm below one effect at a time — so on this path a wide
                // dataset would freeze the UI, keypresses included, for as
                // long as every one of its `bq show` calls took in sequence.
                Effect::LoadBigQueryTableStats { .. } => {
                    self.dispatch_bigquery_table_stats(effect);
                }
                Effect::RunResourceBatch {
                    context,
                    action,
                    targets,
                    concurrency,
                } => {
                    self.dispatch_resource_batch(context, action, targets, concurrency);
                }
                Effect::RunPlugin {
                    context,
                    label,
                    argv,
                    operation_id,
                } => {
                    self.dispatch_run_plugin(context, label, argv, operation_id);
                }
                Effect::CancelRunningPlugins => {
                    self.cancel_running_plugins();
                }
                Effect::StartTunnel { id, context, plan } => {
                    self.dispatch_start_tunnel(id, context, plan);
                }
                Effect::StopTunnel { id } => {
                    self.dispatch_stop_tunnel(id);
                }
                Effect::ReloadConfig => {
                    self.reload_config();
                }
                // Off the run loop, unlike the writers below it in this match:
                // see [`RememberedViewWriter`]. The outcome comes back on the
                // result channel, so the reducer still sees it — just not
                // before the next frame is drawn.
                Effect::SaveRememberedViews { views } => {
                    self.dispatch_remembered_view_save(views);
                }
                // Off the run loop, like the plugin dispatch below it and for
                // the same reason. These two effects used to fall to
                // the catch-all, where `dispatch_effect` calls the blocking
                // `runner.run` inline — awaited inside the run loop's
                // `select!`, so nothing was drawn, no key was read, no poll
                // ticked and no termination arm was reached for as long as the
                // child lived. That was survivable at a two-minute ceiling and
                // is not at a lifecycle one: `gcloud sql instances failover`
                // blocks until GCP finishes the operation, and freezing the TUI
                // for the duration — with Ctrl-C arriving as a byte, not a
                // signal, because the terminal is in raw mode — is not a thing
                // a confirmation dialog may do.
                Effect::ComputeInstanceAction { .. } | Effect::CloudSqlInstanceAction { .. } => {
                    self.dispatch_instance_action(effect);
                }
                effect => {
                    let result = self.dispatch_effect(effect).await;
                    let result_was_accepted = self.effect_result_is_accepted(&result);
                    let follow_up_effects = self.reduce(&Action::EffectResult(result.clone()));
                    if result_was_accepted {
                        self.complete_effect_operation(&result, true);
                    }
                    for follow_up in follow_up_effects.into_iter().rev() {
                        pending.push_front((follow_up, true));
                    }
                }
            }
        }
    }

    /// Hands a save to the writer and returns. A session with no store — a
    /// demo run, or a test harness with nothing to persist to — has nothing to
    /// hand it to, which is not an error; see [`RuntimeSession::new`].
    fn dispatch_remembered_view_save(&mut self, views: RememberedViews) {
        let Some(store) = self.remembered_view_store.clone() else {
            return;
        };
        let writer = self.remembered_view_writer.get_or_insert_with(|| {
            let (sender, mut receiver) = mpsc::unbounded_channel::<RememberedViews>();
            let result_sender = self.effect_result_sender.clone();
            let handle = tokio::task::spawn_blocking(move || {
                // One consumer, each save run to completion before the next is
                // taken: this loop *is* the ordering guarantee.
                while let Some(views) = receiver.blocking_recv() {
                    let result = match store.save(&views) {
                        Ok(()) => EffectResult::RememberedViewsSaved,
                        Err(error) => EffectResult::RememberedViewsSaveFailed { error },
                    };
                    // A closed result channel means the session is gone and
                    // there is nothing left to report to. Deliberately not a
                    // `break`: the saves queued behind this one still have to
                    // reach the disk.
                    let _ = result_sender.send(QueuedEffectResult::RememberedViewSave { result });
                }
            });
            RememberedViewWriter { sender, handle }
        });
        // A closed channel means the writer thread is gone, which only happens
        // after a flush. Dropping the save is right then: the session is on its
        // way out and the map it would write is the one already on disk.
        let _ = writer.sender.send(views);
    }

    /// Waits for every queued save to reach the disk. Called on the way out —
    /// once from the quit action, once more after the run loop returns, so the
    /// exits that never dispatch `QuitRequested` (a closed event stream, a
    /// closed result channel) are covered too. Idempotent: the writer is taken,
    /// so a second call has nothing to wait for.
    pub async fn flush_remembered_view_saves(&mut self) {
        let Some(writer) = self.remembered_view_writer.take() else {
            return;
        };
        // Dropping the sender is what ends the writer's loop; without it the
        // join below would wait forever for a channel the session still holds.
        drop(writer.sender);
        if let Err(error) = writer.handle.await {
            tracing::warn!(%error, "the remembered-view writer failed on shutdown");
        }
    }

    pub async fn apply_effect_result(&mut self, result: EffectResult) {
        self.update_retry_gate(&result);
        let result_was_accepted = self.effect_result_is_accepted(&result);
        let effects = self.reduce(&Action::EffectResult(result.clone()));
        if result_was_accepted {
            self.complete_effect_operation(&result, true);
        }
        self.dispatch_effects(effects, true).await;
        self.ensure_active_resource_loaded().await;
    }

    /// [`RuntimeSession::apply_effect_result`] with one extra condition on the
    /// status line: the operation this run opened must still be the one the
    /// status line is reporting. See [`QueuedEffectResult::Plugin`] for why the
    /// condition guards the completion rather than the delivery.
    ///
    /// The delivery being unconditional is what makes the save/restore below
    /// necessary rather than belt-and-braces. The reducer's own arms settle the
    /// operations-log entry — which must happen however stale the result is —
    /// and in the same pass write `"{label} finished"` to the status line, which
    /// must not. The reducer cannot tell the difference: it sees the context,
    /// not the generation. So the status fields are taken before the reduce and
    /// put back after it whenever this result is not the one the status line is
    /// reporting, exactly as `apply_background_effect_result` does for a
    /// superseded background load. Everything else the reduce wrote — the
    /// settled entry, the recorded diagnostic — stands.
    async fn apply_plugin_effect_result(
        &mut self,
        result: EffectResult,
        operation_generation: u64,
    ) {
        // The run this result belongs to is over, whatever it reported, so its
        // handle leaves the registry here — unconditionally, before the
        // generation guard, because a run that finished under a superseded
        // generation is just as finished. A handle left behind would have the
        // next cancellation claim a run that already ended: the settle itself is
        // a no-op (`resolve_by_id` is pending-guarded and the worker already won
        // that claim), but the status line would report a cancellation that did
        // not happen and hide the "No plugin is running" answer behind it. The
        // registry would also grow for the life of the process.
        if let EffectResult::PluginRunSucceeded { operation_id, .. }
        | EffectResult::PluginRunFailed { operation_id, .. }
        | EffectResult::PluginRunCancelled { operation_id, .. } = &result
        {
            let finished = *operation_id;
            self.plugin_runs
                .retain(|handle| handle.operation_id != finished);
        }
        self.apply_operation_effect_result(result, operation_generation)
            .await;
    }

    /// Applies a result that owns an operations-log entry and arrived off the
    /// run loop: a plugin run, or a single-row lifecycle action.
    ///
    /// Shared rather than duplicated because the two need exactly the same
    /// handling — unconditional delivery so the entry settles, a generation
    /// guard on the status line only, and the reducer's follow-up effects
    /// dispatched, which for a lifecycle action is the list refresh its success
    /// arm asks for. `apply_plugin_effect_result` is this plus the registry
    /// retirement that only a plugin has.
    async fn apply_operation_effect_result(
        &mut self,
        result: EffectResult,
        operation_generation: u64,
    ) {
        self.update_retry_gate(&result);
        let result_was_accepted = operation_generation == self.operation_generation
            && self.effect_result_is_accepted(&result);
        let status_message = self.app.root.system.status_message.clone();
        let status_operation_label = self.app.root.system.status_operation_label.clone();
        let status_phase = self.app.root.system.status_phase;
        let status_generation = self.app.root.system.status_generation;
        let effects = self.reduce(&Action::EffectResult(result.clone()));
        if result_was_accepted {
            self.complete_effect_operation(&result, true);
        } else {
            self.app.root.system.status_message = status_message;
            self.app.root.system.status_operation_label = status_operation_label;
            self.app.root.system.status_phase = status_phase;
            self.app.root.system.status_generation = status_generation;
        }
        self.dispatch_effects(effects, true).await;
        self.ensure_active_resource_loaded().await;
    }

    pub fn take_pending_interactive_command(&mut self) -> Option<InteractiveCommand> {
        self.pending_interactive_command.take()
    }

    pub fn resolve_interactive_spec(
        &self,
        command: &InteractiveCommand,
    ) -> crate::services::runtime::interactive::InteractiveProcessSpec {
        match command {
            InteractiveCommand::GceSsh { context, instance } => {
                let binary = self
                    .config
                    .as_ref()
                    .map(|config| config.gcloud_binary.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "gcloud".to_string());
                crate::services::runtime::interactive::gce_ssh_spec(&binary, context, instance)
            }
            InteractiveCommand::K9s { kube_context } => {
                crate::services::runtime::k9s::k9s_spec(&self.k9s_binary, kube_context)
            }
        }
    }

    pub fn tmux_detected(&self) -> bool {
        crate::services::runtime::interactive::tmux_detected_in(
            std::env::var("TMUX").ok().as_deref(),
        )
    }

    pub async fn apply_interactive_outcome(
        &mut self,
        description: String,
        outcome: crate::services::runtime::interactive::InteractiveProcessOutcome,
    ) {
        self.apply_effect_result(EffectResult::InteractiveProcessCompleted {
            description,
            outcome,
        })
        .await;
    }

    /// Records the `Retryability` of a resource-load result against the
    /// poll-tick retry gate: a `Persistent` failure for a service gates that
    /// service's scope until manual refresh or a context/scope switch clears
    /// it (see `gated_scope`); a fresh success clears any stale gate.
    fn update_retry_gate(&mut self, result: &EffectResult) {
        use nuvem_domain::gcp::diagnostics::Retryability;
        match result {
            EffectResult::ComputeInstancesLoadFailed { context, error } => {
                if error.kind().retryability() == Retryability::Persistent {
                    self.gated_scope =
                        Some((ResourceServiceKind::ComputeInstances, context.clone()));
                }
            }
            EffectResult::GkeClustersLoadFailed { context, error } => {
                if error.kind().retryability() == Retryability::Persistent {
                    self.gated_scope = Some((ResourceServiceKind::GkeClusters, context.clone()));
                }
            }
            EffectResult::ComputeInstancesLoaded { .. }
            | EffectResult::GkeClustersLoaded { .. } => {
                self.gated_scope = None;
            }
            _ => {}
        }
    }

    async fn apply_background_effect_result(
        &mut self,
        result: EffectResult,
        status_generation: u64,
    ) {
        self.update_retry_gate(&result);
        let preserve_status = self.app.root.system.status_generation != status_generation;
        let status_message = self.app.root.system.status_message.clone();
        let status_operation_label = self.app.root.system.status_operation_label.clone();
        let status_phase = self.app.root.system.status_phase;
        let result_was_accepted = self.effect_result_is_accepted(&result);
        let effects = self.reduce(&Action::EffectResult(result.clone()));
        if preserve_status {
            self.app.root.system.status_message = status_message;
            self.app.root.system.status_operation_label = status_operation_label;
            self.app.root.system.status_phase = status_phase;
        }
        if result_was_accepted {
            self.complete_effect_operation(&result, !preserve_status);
        }
        self.dispatch_effects(effects, true).await;
    }

    fn effect_result_is_accepted(&self, result: &EffectResult) -> bool {
        match result {
            // Acceptance is decided by the FinOps ticket rather than by the
            // context: the grid mints a sequence per refresh and drops a
            // superseded result itself, so a second check here would be a
            // second place for the rule to drift.
            EffectResult::CostRecommendationsLoaded { .. }
            | EffectResult::CostRecommendationsFailed { .. }
            | EffectResult::BudgetsLoaded { .. }
            | EffectResult::BudgetsFailed { .. } => true,
            EffectResult::ConfigurationsLoaded { .. }
            | EffectResult::ConfigurationActivated { .. }
            | EffectResult::ConfigurationActivationReloadFailed { .. }
            | EffectResult::ConfigurationsReloadedWithWarning { .. }
            | EffectResult::ConfigurationCreatedReloadFailed { .. }
            | EffectResult::ConfigurationContextUpdateReloadFailed { .. } => true,
            EffectResult::ComputeInstancesLoaded { context, .. }
            | EffectResult::ComputeInstancesLoadFailed { context, .. }
            | EffectResult::GkeClustersLoaded { context, .. }
            | EffectResult::GkeClustersLoadFailed { context, .. }
            | EffectResult::CloudSqlInstancesLoaded { context, .. }
            | EffectResult::CloudSqlInstancesLoadFailed { context, .. }
            | EffectResult::ComputeInstanceActionSucceeded { context, .. }
            | EffectResult::ComputeInstanceActionFailed { context, .. }
            | EffectResult::CloudSqlInstanceActionSucceeded { context, .. }
            | EffectResult::CloudSqlInstanceActionFailed { context, .. }
            | EffectResult::StorageBucketsLoaded { context, .. }
            | EffectResult::StorageBucketsLoadFailed { context, .. }
            | EffectResult::StorageObjectPageLoaded { context, .. }
            | EffectResult::StorageObjectPageLoadFailed { context, .. }
            | EffectResult::PubSubTopicsLoaded { context, .. }
            | EffectResult::PubSubTopicsLoadFailed { context, .. }
            | EffectResult::VpcNetworksLoaded { context, .. }
            | EffectResult::VpcNetworksLoadFailed { context, .. }
            | EffectResult::VpcSubnetsLoaded { context, .. }
            | EffectResult::VpcSubnetsLoadFailed { context, .. }
            | EffectResult::FirewallRulesLoaded { context, .. }
            | EffectResult::FirewallRulesLoadFailed { context, .. }
            | EffectResult::PubSubSubscriptionsLoaded { context, .. }
            | EffectResult::PubSubSubscriptionsLoadFailed { context, .. }
            | EffectResult::BigQueryDatasetsLoaded { context, .. }
            | EffectResult::BigQueryDatasetsLoadFailed { context, .. }
            | EffectResult::BigQueryTablesLoaded { context, .. }
            | EffectResult::BigQueryTablesLoadFailed { context, .. }
            | EffectResult::BigQueryTableStatsLoaded { context, .. }
            | EffectResult::BigQueryTableStatsLoadFailed { context, .. }
            | EffectResult::BigQuerySchemaLoaded { context, .. }
            | EffectResult::BigQuerySchemaLoadFailed { context, .. }
            | EffectResult::ServiceAccountsLoaded { context, .. }
            | EffectResult::ServiceAccountsLoadFailed { context, .. }
            | EffectResult::IamPolicyLoaded { context, .. }
            | EffectResult::IamPolicyLoadFailed { context, .. }
            | EffectResult::SecretsLoaded { context, .. }
            | EffectResult::SecretsLoadFailed { context, .. }
            | EffectResult::SecretDetailLoaded { context, .. }
            | EffectResult::SecretDetailLoadFailed { context, .. }
            | EffectResult::SecretVersionsLoaded { context, .. }
            | EffectResult::SecretVersionsLoadFailed { context, .. }
            | EffectResult::CloudFunctionsLoaded { context, .. }
            | EffectResult::CloudFunctionsLoadFailed { context, .. }
            | EffectResult::PersistentDisksLoaded { context, .. }
            | EffectResult::PersistentDisksLoadFailed { context, .. } => {
                self.app.root.current_context().as_ref() == Some(context)
            }
            EffectResult::CloudSqlInstanceDetailLoaded { context, detail } => {
                self.app.root.current_context().as_ref() == Some(context)
                    && self
                        .app
                        .root
                        .resources
                        .is_active_cloud_sql_detail_target(&detail.name, &detail.region)
            }
            EffectResult::CloudSqlInstanceDetailLoadFailed {
                context,
                instance_name,
                instance_region,
                ..
            } => {
                self.app.root.current_context().as_ref() == Some(context)
                    && self
                        .app
                        .root
                        .resources
                        .is_active_cloud_sql_detail_target(instance_name, instance_region)
            }
            EffectResult::CloudSqlUsersLoaded {
                context,
                instance_name,
                ..
            }
            | EffectResult::CloudSqlUsersLoadFailed {
                context,
                instance_name,
                ..
            } => {
                self.app.root.current_context().as_ref() == Some(context)
                    && self
                        .app
                        .root
                        .resources
                        .is_active_cloud_sql_detail_name(instance_name)
            }
            EffectResult::StorageBucketDetailLoaded { context, detail } => {
                self.app.root.current_context().as_ref() == Some(context)
                    && self
                        .app
                        .root
                        .resources
                        .storage()
                        .is_active_bucket_detail_target(&detail.name)
            }
            EffectResult::StorageBucketDetailLoadFailed {
                context,
                bucket_name,
                ..
            } => {
                self.app.root.current_context().as_ref() == Some(context)
                    && self
                        .app
                        .root
                        .resources
                        .storage()
                        .is_active_bucket_detail_target(bucket_name)
            }
            EffectResult::StorageObjectDetailLoaded { context, detail } => {
                self.app.root.current_context().as_ref() == Some(context)
                    && self
                        .app
                        .root
                        .resources
                        .storage()
                        .is_active_object_detail_target(&detail.key)
            }
            EffectResult::StorageObjectDetailLoadFailed {
                context,
                object_key,
                ..
            } => {
                self.app.root.current_context().as_ref() == Some(context)
                    && self
                        .app
                        .root
                        .resources
                        .storage()
                        .is_active_object_detail_target(object_key)
            }
            EffectResult::ComputeInstanceDetailLoaded { context, detail } => {
                self.app.root.current_context().as_ref() == Some(context)
                    && self
                        .app
                        .root
                        .resources
                        .is_active_detail_target(&detail.name, &detail.zone)
            }
            EffectResult::ComputeInstanceDetailLoadFailed {
                context,
                instance_name,
                instance_zone,
                ..
            } => {
                self.app.root.current_context().as_ref() == Some(context)
                    && self
                        .app
                        .root
                        .resources
                        .is_active_detail_target(instance_name, instance_zone)
            }
            EffectResult::GkeClusterDetailLoaded { context, detail } => {
                self.app.root.current_context().as_ref() == Some(context)
                    && self
                        .app
                        .root
                        .resources
                        .is_active_gke_detail_target(&detail.name, &detail.location)
            }
            EffectResult::GkeClusterDetailLoadFailed {
                context,
                cluster_name,
                cluster_location,
                ..
            } => {
                self.app.root.current_context().as_ref() == Some(context)
                    && self
                        .app
                        .root
                        .resources
                        .is_active_gke_detail_target(cluster_name, cluster_location)
            }
            EffectResult::RecentLogsLoaded { context, .. }
            | EffectResult::RecentLogsLoadFailed { context, .. }
            | EffectResult::LogsTailed { context, .. }
            | EffectResult::LogsTailFailed { context, .. } => {
                self.app.root.current_context().as_ref() == Some(context)
            }
            // Gated on the context like the lifecycle actions, and for the same
            // reason: the run happens on a blocking thread, so the operator can
            // switch configuration while the child is still alive, and a result
            // from the context they left must not complete the operation of the
            // context they are looking at. The result is still delivered to the
            // reducer either way — only this gate is context-scoped — so the
            // operations-log entry it settles is settled regardless.
            EffectResult::PluginRunSucceeded { context, .. }
            | EffectResult::PluginRunFailed { context, .. }
            | EffectResult::PluginRunCancelled { context, .. } => {
                self.app.root.current_context().as_ref() == Some(context)
            }
            EffectResult::Failed { .. } => {
                is_configuration_operation(self.app.root.system.operation.label.as_str())
            }
            EffectResult::ContextPickerLoaded { .. }
            | EffectResult::ContextPickerLoadFailed { .. }
            | EffectResult::ClipboardCopied { .. }
            | EffectResult::ClipboardCopyFailed { .. }
            | EffectResult::LogEntriesSaved { .. }
            | EffectResult::LogEntriesSaveFailed { .. }
            // A remembered-view save belongs to no context and to no tracked
            // operation, exactly like a clipboard copy: it must not complete
            // whatever operation the status line is currently reporting.
            | EffectResult::RememberedViewsSaved
            | EffectResult::RememberedViewsSaveFailed { .. }
            | EffectResult::GcloudDiagnosticsProbed { .. } => false,
            EffectResult::InteractiveProcessCompleted { .. } => false,
            EffectResult::K9sLaunchPrepared { .. }
            | EffectResult::K9sCredentialsReady { .. }
            | EffectResult::K9sCredentialsFailed { .. } => false,
            // Tunnel results are addressed by tunnel id, not by the active
            // view, so they are never gated on the current context: a tunnel
            // outlives the context it was started under.
            EffectResult::TunnelPortAllocated { .. }
            | EffectResult::TunnelReady { .. }
            | EffectResult::TunnelFailed { .. }
            | EffectResult::TunnelNotice { .. }
            | EffectResult::TunnelExited { .. }
            | EffectResult::TunnelStopped { .. } => false,
            EffectResult::CloudRunServicesLoaded { context, .. }
            | EffectResult::CloudRunServicesLoadFailed { context, .. } => {
                self.app.root.current_context().as_ref() == Some(context)
            }
            EffectResult::CloudRunServiceDetailLoaded { context, detail } => {
                self.app.root.current_context().as_ref() == Some(context)
                    && self
                        .app
                        .root
                        .resources
                        .is_active_cloud_run_detail_target(&detail.name, &detail.region)
            }
            EffectResult::CloudRunServiceDetailLoadFailed {
                context,
                service_name,
                service_region,
                ..
            }
            | EffectResult::CloudRunRevisionsLoaded {
                context,
                service_name,
                service_region,
                ..
            }
            | EffectResult::CloudRunRevisionsLoadFailed {
                context,
                service_name,
                service_region,
                ..
            } => {
                self.app.root.current_context().as_ref() == Some(context)
                    && self
                        .app
                        .root
                        .resources
                        .is_active_cloud_run_detail_target(service_name, service_region)
            }
            EffectResult::CloudRunRevisionDetailLoaded { context, detail } => {
                self.app.root.current_context().as_ref() == Some(context)
                    && self
                        .app
                        .root
                        .resources
                        .cloud_run_revision_target()
                        .map(|revision| revision.name.as_str())
                        == Some(detail.name.as_str())
            }
            EffectResult::CloudRunRevisionDetailLoadFailed {
                context,
                revision_name,
                ..
            } => {
                self.app.root.current_context().as_ref() == Some(context)
                    && self
                        .app
                        .root
                        .resources
                        .cloud_run_revision_target()
                        .map(|revision| revision.name.as_str())
                        == Some(revision_name.as_str())
            }
        }
    }

    fn complete_effect_operation(&mut self, result: &EffectResult, update_status: bool) {
        let operation_label = self.app.root.system.operation.label.as_str();
        let Some((phase, label)) = effect_result_operation(result, operation_label) else {
            return;
        };
        if is_refresh_result(result, operation_label) {
            self.app
                .root
                .system
                .complete_refresh_operation(phase, label, Local::now());
        } else {
            self.app.root.system.complete_operation(phase, label);
        }
        if update_status {
            self.app
                .root
                .system
                .associate_status_message_with_operation(label);
        }
    }

    fn reduce(&mut self, action: &Action) -> Vec<Effect> {
        let previous_context = self.app.root.current_context();
        let previous_scope = self.view_scope();
        let previous_frame_context = self.active_frame_context();
        let previous_known = self.known_configuration_scopes();
        let effects = reduce(&mut self.app, action);
        let context_changed = previous_context != self.app.root.current_context();
        let scope_changed = previous_scope != self.view_scope();

        // The health sample. Here rather than in the reducer for the
        // same two reasons the frame override below is: the clock read is a
        // runtime-layer concern, and the reducer stays a pure state transition.
        //
        // Keyed on what the resource collections now hold rather than on
        // specific `EffectResult` variants, so a future list result — or a
        // service added to the registry — cannot silently skip being sampled.
        // That is the known failure mode: three services shipped with no Home
        // entry and nothing failed.
        self.app
            .root
            .sample_service_health(std::time::Instant::now());

        // The frame override. Recomputed here rather than
        // in the reducer so the process-wide theme global stays a
        // runtime-layer concern, beside the other two `set_active` calls, and
        // the reducer stays a pure state transition.
        //
        // Keyed on the whole pair the colour is looked up by, not on the name
        // alone: a configuration's project changes under a stable name (an
        // edit through the context form, or a `gcloud config set project`
        // picked up by a reload), and a name-only guard would leave the
        // previous project's frame painted over the new one.
        if previous_frame_context != self.active_frame_context() {
            self.refresh_theme();
        }
        // The deferred "this override names nothing" check. Keyed on the known
        // configurations rather than on specific `EffectResult` variants, so a
        // future result that introduces configurations cannot silently skip
        // it — and on their projects as well as their names, because
        // the check reads both. A project repointed under a stable name
        // changes the answer while every name stays put, and a name-only key
        // would leave the previous answer on display: a warning claiming an
        // override will never apply after it started applying, or worse, no
        // warning after it stopped.
        if previous_known != self.known_configuration_scopes() {
            self.refresh_config_problems();
        }
        if context_changed {
            self.context_generation = self.context_generation.wrapping_add(1);
            // The native clients and the tokens behind them are keyed by
            // configuration; the ones for the configuration just left are
            // dropped rather than kept alive for the session's lifetime.
            if let (Some(clients), Some((active, _))) =
                (&self.sdk_clients, self.active_frame_context())
            {
                clients.evict_except(active.as_str());
            }
        }
        if scope_changed {
            self.scope_generation = self.scope_generation.wrapping_add(1);
        }
        // A context/service/workspace switch detaches any in-flight batch:
        // bumping the generation drops late results and cancelling the token
        // stops targets that have not yet started running.
        if context_changed || scope_changed {
            self.cancel_batch();
        }
        if context_changed || scope_changed {
            // A context or active-service switch always gets a fresh attempt;
            // the retry gate only applies within the scope that failed.
            self.gated_scope = None;
        }
        effects
    }

    /// Everything the frame colour is looked up by: the active
    /// configuration's name and the project it points at. The diff that
    /// decides whether the theme needs recomputing reads this rather than the
    /// name alone.
    fn active_frame_context(&self) -> Option<(GcloudConfigurationName, Option<String>)> {
        self.app
            .root
            .configurations
            .active()
            .map(|active| (active.name.clone(), active.project.clone()))
    }

    /// The configurations nuvem currently knows about, each with the project
    /// it points at — everything the deferred check reads. Small (a handful of
    /// gcloud configurations), so cloning it per action to diff against is
    /// cheaper than the plumbing needed to avoid it.
    fn known_configuration_scopes(&self) -> Vec<(GcloudConfigurationName, Option<String>)> {
        self.app
            .root
            .configurations
            .rows()
            .iter()
            .map(|row| (row.name.clone(), row.project.clone()))
            .collect()
    }

    fn view_scope(&self) -> (Workspace, ResourceServiceKind) {
        (
            self.app.root.workspace,
            self.app.root.resources.active_service(),
        )
    }

    /// Writes a log dump beside the app's own log file, in a `log-dumps`
    /// sibling directory. Without a resolved config -- or with a log file that
    /// names no directory to sit beside -- there is no cache dir to write
    /// into, which is reported as a failure rather than silently dropping the
    /// user's export or writing it into the working directory.
    fn save_log_entries(&self, lines: &[String]) -> EffectResult {
        let Some(directory) = self.log_dump_directory() else {
            return EffectResult::LogEntriesSaveFailed {
                error: AppError::gcloud(
                    "no log directory is resolved, so there is nowhere to save log entries",
                ),
            };
        };
        let timestamp = format_log_dump_timestamp(Local::now());
        match log_export::write_log_dump(&directory, &timestamp, lines) {
            Ok(path) => EffectResult::LogEntriesSaved {
                path: path.display().to_string(),
                entries: lines.len(),
            },
            Err(error) => EffectResult::LogEntriesSaveFailed {
                error: AppError::gcloud(format!("could not write the log dump: {error}")),
            },
        }
    }

    /// The `log-dumps` directory beside the app's log file, or `None` when the
    /// log file has no directory to sit beside. A bare `--log-file nuvem.log`
    /// yields `Some("")` from `parent()`, which would join into a RELATIVE
    /// `log-dumps` written under whatever directory the TUI happens to have
    /// been started in — ruled out for this feature — so an empty parent is
    /// treated as no parent and reported as a save failure.
    fn log_dump_directory(&self) -> Option<PathBuf> {
        let config = self.config.as_ref()?;
        let parent = config
            .log_file
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())?;
        Some(parent.join("log-dumps"))
    }

    async fn dispatch_effect(&self, effect: Effect) -> EffectResult {
        match effect {
            Effect::LoadCostRecommendations {
                context,
                recommender,
                location,
                ticket,
            } => {
                let Some(runner) = &self.gcloud_runner else {
                    return EffectResult::CostRecommendationsFailed {
                        ticket,
                        recommender,
                        location,
                        error: crate::app::AppError::gcloud(
                            "no gcloud command runner configured".to_string(),
                        ),
                    };
                };
                if let Some(clients) = self.sdk_clients.as_deref() {
                    let project = context.project.clone().unwrap_or_default();
                    let loaded =
                        clients
                            .finops(context.configuration.as_str())
                            .and_then(|client| {
                                crate::services::sdk::block_on(client.load_recommendations(
                                    &project,
                                    recommender,
                                    &location,
                                ))
                            });
                    return match loaded {
                        Ok(recommendations) => EffectResult::CostRecommendationsLoaded {
                            ticket,
                            recommender,
                            location,
                            recommendations,
                        },
                        // Same safety property as the CLI arm below: anything
                        // short of success lands here rather than as an empty
                        // list, so a refusal cannot read as "nothing to save".
                        Err(error) => EffectResult::CostRecommendationsFailed {
                            ticket,
                            recommender,
                            location,
                            error: crate::app::AppError::gcloud(error.to_string()),
                        },
                    };
                }
                let service = GcloudFinOpsService::new(runner.as_ref());
                match service.load_recommendations(&context, recommender, &location) {
                    Ok(recommendations) => EffectResult::CostRecommendationsLoaded {
                        ticket,
                        recommender,
                        location,
                        recommendations,
                    },
                    // The error path is the safety property: every refusal
                    // writes `[]` to stdout, so anything short of a zero exit
                    // must land here rather than as an empty list.
                    Err(error) => EffectResult::CostRecommendationsFailed {
                        ticket,
                        recommender,
                        location,
                        error: crate::app::AppError::gcloud(error.to_string()),
                    },
                }
            }
            Effect::LoadBudgets { context, ticket } => {
                let Some(runner) = &self.gcloud_runner else {
                    return EffectResult::BudgetsFailed {
                        ticket,
                        error: crate::app::AppError::gcloud(
                            "no gcloud command runner configured".to_string(),
                        ),
                    };
                };
                let service = GcloudFinOpsService::new(runner.as_ref());
                match service.load_budgets(&context) {
                    Ok(budgets) => EffectResult::BudgetsLoaded { ticket, budgets },
                    Err(error) => EffectResult::BudgetsFailed {
                        ticket,
                        error: crate::app::AppError::gcloud(error.to_string()),
                    },
                }
            }
            Effect::LoadConfigurations => {
                let Some(runner) = &self.gcloud_runner else {
                    return EffectResult::Failed {
                        error: crate::app::AppError::gcloud("no gcloud command runner configured"),
                    };
                };
                let service = GcloudConfigurationService::new(runner.as_ref());
                match service.load_configurations() {
                    Ok(configurations) => EffectResult::ConfigurationsLoaded { configurations },
                    Err(error) => EffectResult::Failed {
                        error: error.into(),
                    },
                }
            }
            Effect::ActivateConfiguration { name } => {
                let Some(runner) = &self.gcloud_runner else {
                    return EffectResult::Failed {
                        error: crate::app::AppError::gcloud("no gcloud command runner configured"),
                    };
                };
                let known_configuration = self
                    .app
                    .root
                    .configurations
                    .rows()
                    .iter()
                    .find(|configuration| configuration.name == name)
                    .cloned()
                    .map(|mut configuration| {
                        configuration.is_active = true;
                        configuration
                    });
                let service = GcloudConfigurationService::new(runner.as_ref());
                match service.activate_configuration(&name) {
                    Ok(()) => match service.load_configurations() {
                        Ok(configurations) => {
                            let mut active_configurations = configurations
                                .iter()
                                .filter(|configuration| configuration.is_active);
                            let requested_is_uniquely_active = active_configurations
                                .next()
                                .is_some_and(|configuration| configuration.name == name)
                                && active_configurations.next().is_none();
                            if requested_is_uniquely_active {
                                EffectResult::ConfigurationsLoaded { configurations }
                            } else {
                                EffectResult::ConfigurationActivationReloadFailed {
                                    configuration: known_configuration.clone().unwrap_or_else(
                                        || active_configuration_shell(name.as_str().to_string()),
                                    ),
                                    error: crate::app::AppError::gcloud(format!(
                                        "activated configuration {name} was not uniquely active after reload"
                                    )),
                                }
                            }
                        }
                        Err(error) => EffectResult::ConfigurationActivationReloadFailed {
                            configuration: known_configuration.unwrap_or_else(|| {
                                active_configuration_shell(name.as_str().to_string())
                            }),
                            error: error.into(),
                        },
                    },
                    Err(error) => EffectResult::Failed {
                        error: error.into(),
                    },
                }
            }
            Effect::CreateConfiguration { draft } => {
                let Some(runner) = &self.gcloud_runner else {
                    return EffectResult::Failed {
                        error: crate::app::AppError::gcloud("no gcloud command runner configured"),
                    };
                };
                let name = draft.name.clone();
                let fallback_configuration = GcloudConfiguration {
                    name: GcloudConfigurationName::new(name.clone()),
                    is_active: true,
                    account: draft.account.clone(),
                    project: draft.project.clone(),
                    region: draft.region.clone(),
                    zone: draft.zone.clone(),
                };
                let service = GcloudConfigurationService::new(runner.as_ref());
                match service.create_configuration(draft) {
                    Ok(()) => match service.load_configurations() {
                        Ok(configurations) => {
                            if configurations
                                .iter()
                                .any(|configuration| configuration.name.as_str() == name)
                            {
                                EffectResult::ConfigurationsLoaded { configurations }
                            } else {
                                EffectResult::ConfigurationCreatedReloadFailed {
                                    configuration: fallback_configuration,
                                    error: crate::app::AppError::gcloud(format!(
                                        "created configuration {name} was not returned by gcloud"
                                    )),
                                }
                            }
                        }
                        Err(error) => EffectResult::ConfigurationCreatedReloadFailed {
                            configuration: fallback_configuration,
                            error: error.into(),
                        },
                    },
                    Err(GcloudConfigurationError::CreatedConfigurationSetupFailed {
                        name,
                        source,
                    }) => {
                        let setup_error: AppError =
                            GcloudConfigurationError::CreatedConfigurationSetupFailed {
                                name: name.clone(),
                                source,
                            }
                            .into();
                        match service.load_configurations() {
                            Ok(configurations) => {
                                if configurations
                                    .iter()
                                    .any(|configuration| configuration.name.as_str() == name)
                                {
                                    EffectResult::ConfigurationsReloadedWithWarning {
                                        configurations,
                                        error: setup_error,
                                    }
                                } else {
                                    EffectResult::ConfigurationCreatedReloadFailed {
                                        configuration: active_configuration_shell(name),
                                        error: setup_error,
                                    }
                                }
                            }
                            Err(reload_error) => EffectResult::ConfigurationCreatedReloadFailed {
                                configuration: active_configuration_shell(name),
                                error: AppError::gcloud(format!(
                                    "{setup_error:?}; failed to reload configurations: {reload_error}"
                                )),
                            },
                        }
                    }
                    Err(error) => EffectResult::Failed {
                        error: error.into(),
                    },
                }
            }
            Effect::UpdateConfigurationContext { draft } => {
                let Some(runner) = &self.gcloud_runner else {
                    return EffectResult::Failed {
                        error: AppError::gcloud("no gcloud command runner configured"),
                    };
                };
                let configuration_name = draft.configuration.clone();
                let known_configuration = self.app.root.configurations.active().cloned();
                let service = GcloudConfigurationService::new(runner.as_ref());
                match service.update_context(draft) {
                    Ok(applied) => {
                        let fallback_configuration = context_update_fallback_configuration(
                            configuration_name,
                            known_configuration,
                            applied,
                        );
                        match service.load_configurations() {
                            Ok(configurations) => {
                                EffectResult::ConfigurationsLoaded { configurations }
                            }
                            Err(error) => EffectResult::ConfigurationContextUpdateReloadFailed {
                                configuration: fallback_configuration,
                                error: error.into(),
                            },
                        }
                    }
                    Err(GcloudConfigurationError::ContextUpdatePartiallyApplied {
                        applied,
                        source,
                    }) => {
                        let fallback_configuration = context_update_fallback_configuration(
                            configuration_name,
                            known_configuration,
                            applied.clone(),
                        );
                        let update_error: AppError =
                            GcloudConfigurationError::ContextUpdatePartiallyApplied {
                                applied,
                                source,
                            }
                            .into();
                        match service.load_configurations() {
                            Ok(configurations) => EffectResult::ConfigurationsReloadedWithWarning {
                                configurations,
                                error: update_error,
                            },
                            Err(reload_error) => {
                                EffectResult::ConfigurationContextUpdateReloadFailed {
                                    configuration: fallback_configuration,
                                    error: AppError::gcloud(format!(
                                        "{update_error:?}; failed to reload configurations: {reload_error}"
                                    )),
                                }
                            }
                        }
                    }
                    Err(error) => EffectResult::Failed {
                        error: error.into(),
                    },
                }
            }
            Effect::LoadContextPicker { kind, context } => self.load_context_picker(kind, context),
            Effect::RefreshComputeInstances { context, filter } => {
                let Some(runner) = &self.gcloud_runner else {
                    return EffectResult::ComputeInstancesLoadFailed {
                        context,
                        error: crate::app::AppError::gcloud("no gcloud command runner configured"),
                    };
                };
                load_compute_instances(
                    runner.as_ref(),
                    self.sdk_clients.as_deref(),
                    context,
                    filter,
                )
            }
            Effect::LoadComputeInstanceDetail { context, instance } => {
                let instance_name = instance.name.clone();
                let instance_zone = instance.zone.clone();
                let Some(runner) = &self.gcloud_runner else {
                    return EffectResult::ComputeInstanceDetailLoadFailed {
                        context,
                        instance_name,
                        instance_zone,
                        error: crate::app::AppError::gcloud("no gcloud command runner configured"),
                    };
                };
                if let Some(clients) = self.sdk_clients.as_deref() {
                    let project = context.project.clone().unwrap_or_default();
                    let loaded =
                        clients
                            .compute(context.configuration.as_str())
                            .and_then(|client| {
                                crate::services::sdk::block_on(client.load_instance_detail(
                                    &project,
                                    &instance.zone,
                                    &instance.name,
                                ))
                            });
                    return match loaded {
                        Ok(detail) => EffectResult::ComputeInstanceDetailLoaded { context, detail },
                        Err(error) => EffectResult::ComputeInstanceDetailLoadFailed {
                            context,
                            instance_name,
                            instance_zone,
                            error: crate::app::AppError::gcloud(error.to_string()),
                        },
                    };
                }
                let service =
                    crate::services::resources::compute::GcloudComputeService::new(runner.as_ref());
                match service.load_instance_detail(&context, &instance) {
                    Ok(detail) => EffectResult::ComputeInstanceDetailLoaded { context, detail },
                    Err(crate::services::resources::compute::ComputeLoadError::Command(error)) => {
                        let error: AppError = error.into();
                        EffectResult::ComputeInstanceDetailLoadFailed {
                            context,
                            instance_name,
                            instance_zone,
                            error: error.with_command_summary("gcloud compute instances describe"),
                        }
                    }
                    Err(crate::services::resources::compute::ComputeLoadError::Parse(error)) => {
                        EffectResult::ComputeInstanceDetailLoadFailed {
                            context,
                            instance_name,
                            instance_zone,
                            error: crate::app::AppError::Parse(error.to_string()),
                        }
                    }
                }
            }
            Effect::RefreshGkeClusters { context, filter } => {
                let Some(runner) = &self.gcloud_runner else {
                    return EffectResult::GkeClustersLoadFailed {
                        context,
                        error: crate::app::AppError::gcloud("no gcloud command runner configured"),
                    };
                };
                load_gke_clusters(
                    runner.as_ref(),
                    self.sdk_clients.as_deref(),
                    context,
                    filter,
                )
            }
            Effect::LoadGkeClusterDetail { context, cluster } => {
                let cluster_name = cluster.name.clone();
                let cluster_location = cluster.location.clone();
                let Some(runner) = &self.gcloud_runner else {
                    return EffectResult::GkeClusterDetailLoadFailed {
                        context,
                        cluster_name,
                        cluster_location,
                        error: crate::app::AppError::gcloud("no gcloud command runner configured"),
                    };
                };
                if let Some(clients) = self.sdk_clients.as_deref() {
                    let project = context.project.clone().unwrap_or_default();
                    let loaded = clients
                        .gke(context.configuration.as_str())
                        .and_then(|client| {
                            crate::services::sdk::block_on(client.load_cluster_detail(
                                &project,
                                &cluster.location,
                                &cluster.name,
                            ))
                        });
                    return match loaded {
                        Ok(detail) => EffectResult::GkeClusterDetailLoaded { context, detail },
                        Err(error) => EffectResult::GkeClusterDetailLoadFailed {
                            context,
                            cluster_name,
                            cluster_location,
                            error: crate::app::AppError::gcloud(error.to_string()),
                        },
                    };
                }
                let service = GcloudGkeService::new(runner.as_ref());
                match service.load_cluster_detail(&context, &cluster) {
                    Ok(detail) => EffectResult::GkeClusterDetailLoaded { context, detail },
                    Err(crate::services::resources::gke::GkeLoadError::Command(error)) => {
                        let error: AppError = error.into();
                        EffectResult::GkeClusterDetailLoadFailed {
                            context,
                            cluster_name,
                            cluster_location,
                            error: error.with_command_summary("gcloud container clusters describe"),
                        }
                    }
                    Err(crate::services::resources::gke::GkeLoadError::Parse(error)) => {
                        EffectResult::GkeClusterDetailLoadFailed {
                            context,
                            cluster_name,
                            cluster_location,
                            error: crate::app::AppError::Parse(error.to_string()),
                        }
                    }
                }
            }
            Effect::RefreshCloudSqlInstances { context, filter } => {
                let Some(runner) = &self.gcloud_runner else {
                    return EffectResult::CloudSqlInstancesLoadFailed {
                        context,
                        error: crate::app::AppError::gcloud(
                            "no gcloud command runner configured".to_string(),
                        ),
                    };
                };
                load_cloud_sql_instances(
                    runner.as_ref(),
                    self.sdk_clients.as_deref(),
                    context,
                    filter,
                )
            }
            Effect::LoadCloudSqlInstanceDetail { context, instance } => {
                let instance_name = instance.name.clone();
                let instance_region = instance.region.clone();
                let Some(runner) = &self.gcloud_runner else {
                    return EffectResult::CloudSqlInstanceDetailLoadFailed {
                        context,
                        instance_name,
                        instance_region,
                        error: crate::app::AppError::gcloud(
                            "no gcloud command runner configured".to_string(),
                        ),
                    };
                };
                if let Some(clients) = self.sdk_clients.as_deref() {
                    let project = context.project.clone().unwrap_or_default();
                    let loaded =
                        clients
                            .cloudsql(context.configuration.as_str())
                            .and_then(|client| {
                                crate::services::sdk::block_on(
                                    client.load_instance_detail(&project, &instance.name),
                                )
                            });
                    return match loaded {
                        Ok(detail) => EffectResult::CloudSqlInstanceDetailLoaded {
                            context,
                            detail: Box::new(detail),
                        },
                        Err(error) => EffectResult::CloudSqlInstanceDetailLoadFailed {
                            context,
                            instance_name,
                            instance_region,
                            error: crate::app::AppError::gcloud(error.to_string()),
                        },
                    };
                }
                let service = GcloudCloudSqlService::new(runner.as_ref());
                match service.load_instance_detail(&context, &instance) {
                    Ok(detail) => EffectResult::CloudSqlInstanceDetailLoaded {
                        context,
                        detail: Box::new(detail),
                    },
                    Err(crate::services::resources::cloudsql::CloudSqlLoadError::Command(
                        error,
                    )) => EffectResult::CloudSqlInstanceDetailLoadFailed {
                        context,
                        instance_name,
                        instance_region,
                        error: crate::app::AppError::gcloud(error.to_string()),
                    },
                    Err(crate::services::resources::cloudsql::CloudSqlLoadError::Parse(error)) => {
                        EffectResult::CloudSqlInstanceDetailLoadFailed {
                            context,
                            instance_name,
                            instance_region,
                            error: crate::app::AppError::Parse(error.to_string()),
                        }
                    }
                }
            }
            Effect::LoadCloudSqlUsers { context, instance } => {
                let instance_name = instance.name.clone();
                let Some(runner) = &self.gcloud_runner else {
                    return EffectResult::CloudSqlUsersLoadFailed {
                        context,
                        instance_name,
                        error: crate::app::AppError::gcloud(
                            "no gcloud command runner configured".to_string(),
                        ),
                    };
                };
                if let Some(clients) = self.sdk_clients.as_deref() {
                    let project = context.project.clone().unwrap_or_default();
                    let loaded =
                        clients
                            .cloudsql(context.configuration.as_str())
                            .and_then(|client| {
                                crate::services::sdk::block_on(
                                    client.load_users(&project, &instance_name),
                                )
                            });
                    return match loaded {
                        Ok(users) => EffectResult::CloudSqlUsersLoaded {
                            context,
                            instance_name,
                            users,
                        },
                        Err(error) => EffectResult::CloudSqlUsersLoadFailed {
                            context,
                            instance_name,
                            error: crate::app::AppError::gcloud(error.to_string()),
                        },
                    };
                }
                let service = GcloudCloudSqlService::new(runner.as_ref());
                match service.load_users(&context, &instance_name) {
                    Ok(users) => EffectResult::CloudSqlUsersLoaded {
                        context,
                        instance_name,
                        users,
                    },
                    Err(crate::services::resources::cloudsql::CloudSqlLoadError::Command(
                        error,
                    )) => EffectResult::CloudSqlUsersLoadFailed {
                        context,
                        instance_name,
                        error: crate::app::AppError::gcloud(error.to_string()),
                    },
                    Err(crate::services::resources::cloudsql::CloudSqlLoadError::Parse(error)) => {
                        EffectResult::CloudSqlUsersLoadFailed {
                            context,
                            instance_name,
                            error: crate::app::AppError::Parse(error.to_string()),
                        }
                    }
                }
            }
            Effect::RefreshStorageBuckets { context } => {
                let Some(runner) = &self.gcloud_runner else {
                    return EffectResult::StorageBucketsLoadFailed {
                        context,
                        error: crate::app::AppError::gcloud(
                            "no gcloud command runner configured".to_string(),
                        ),
                    };
                };
                load_storage_buckets(runner.as_ref(), self.sdk_clients.as_deref(), context)
            }
            Effect::LoadStorageBucketDetail { context, bucket } => {
                let bucket_name = bucket.name.clone();
                let Some(runner) = &self.gcloud_runner else {
                    return EffectResult::StorageBucketDetailLoadFailed {
                        context,
                        bucket_name,
                        error: crate::app::AppError::gcloud(
                            "no gcloud command runner configured".to_string(),
                        ),
                    };
                };
                if let Some(clients) = self.sdk_clients.as_deref() {
                    let loaded =
                        clients
                            .storage(context.configuration.as_str())
                            .and_then(|client| {
                                crate::services::sdk::block_on(
                                    client.load_bucket_detail(&bucket_name),
                                )
                            });
                    return match loaded {
                        Ok(detail) => EffectResult::StorageBucketDetailLoaded { context, detail },
                        Err(error) => EffectResult::StorageBucketDetailLoadFailed {
                            context,
                            bucket_name,
                            error: crate::app::AppError::gcloud(error.to_string()),
                        },
                    };
                }
                let service = GcloudStorageService::new(runner.as_ref());
                match service.load_bucket_detail(&context, &bucket_name) {
                    Ok(detail) => EffectResult::StorageBucketDetailLoaded { context, detail },
                    Err(StorageLoadError::Command(error)) => {
                        EffectResult::StorageBucketDetailLoadFailed {
                            context,
                            bucket_name,
                            error: crate::app::AppError::gcloud(error.to_string()),
                        }
                    }
                    Err(StorageLoadError::Parse(error)) => {
                        EffectResult::StorageBucketDetailLoadFailed {
                            context,
                            bucket_name,
                            error: crate::app::AppError::Parse(error.to_string()),
                        }
                    }
                }
            }
            Effect::LoadStorageObjectPage {
                context,
                bucket,
                prefix,
                page_token,
                page_size,
            } => load_storage_object_page(
                self.gcloud_runner.as_deref(),
                self.sdk_clients.as_deref(),
                context,
                bucket,
                prefix,
                page_token,
                page_size,
            ),
            Effect::LoadStorageObjectDetail {
                context,
                bucket,
                object_key,
            } => {
                let Some(runner) = &self.gcloud_runner else {
                    return EffectResult::StorageObjectDetailLoadFailed {
                        context,
                        object_key,
                        error: crate::app::AppError::gcloud(
                            "no gcloud command runner configured".to_string(),
                        ),
                    };
                };
                let service = GcloudStorageService::new(runner.as_ref());
                if let Some(clients) = self.sdk_clients.as_deref() {
                    let loaded =
                        clients
                            .storage(context.configuration.as_str())
                            .and_then(|client| {
                                crate::services::sdk::block_on(
                                    client.load_object_detail(&bucket, &object_key),
                                )
                            });
                    return match loaded {
                        Ok(detail) => EffectResult::StorageObjectDetailLoaded { context, detail },
                        Err(error) => EffectResult::StorageObjectDetailLoadFailed {
                            context,
                            object_key,
                            error: crate::app::AppError::gcloud(error.to_string()),
                        },
                    };
                }
                match service.load_object_detail(&context, &bucket, &object_key) {
                    Ok(detail) => EffectResult::StorageObjectDetailLoaded { context, detail },
                    Err(StorageLoadError::Command(error)) => {
                        EffectResult::StorageObjectDetailLoadFailed {
                            context,
                            object_key,
                            error: crate::app::AppError::gcloud(error.to_string()),
                        }
                    }
                    Err(StorageLoadError::Parse(error)) => {
                        EffectResult::StorageObjectDetailLoadFailed {
                            context,
                            object_key,
                            error: crate::app::AppError::Parse(error.to_string()),
                        }
                    }
                }
            }
            Effect::ComputeInstanceAction { .. } | Effect::CloudSqlInstanceAction { .. } => {
                unreachable!(
                    "a lifecycle action is dispatched to a blocking thread in dispatch_effects"
                )
            }
            Effect::RunPlugin { .. } => {
                unreachable!("a plugin run is dispatched to a blocking thread in dispatch_effects")
            }
            Effect::CancelRunningPlugins => {
                unreachable!("cancellation is applied to session state in dispatch_effects")
            }
            Effect::CopyToClipboard { value, description } => match &self.clipboard_writer {
                Some(writer) => match writer.set_text(&value) {
                    Ok(()) => EffectResult::ClipboardCopied { description },
                    Err(error) => EffectResult::ClipboardCopyFailed {
                        description,
                        error: AppError::gcloud(format!("clipboard copy failed: {error}")),
                    },
                },
                None => EffectResult::ClipboardCopyFailed {
                    description,
                    error: AppError::gcloud("no clipboard writer configured"),
                },
            },
            Effect::SaveRememberedViews { .. } => {
                unreachable!("a remembered-view save is dispatched to the background writer")
            }
            Effect::SaveLogEntries { lines } => self.save_log_entries(&lines),
            Effect::ProbeGcloudDiagnostics { .. } => {
                unreachable!("diagnostics probe is dispatched in the background")
            }
            Effect::LaunchInteractiveProcess { .. } => {
                unreachable!("interactive launch is routed through the run loop")
            }
            Effect::LoadRecentLogs { .. } => {
                unreachable!("log loading is dispatched in the background")
            }
            Effect::TailLogs { .. } => {
                unreachable!("log tailing is dispatched in the background")
            }
            Effect::PrepareK9sLaunch { target } => {
                let existing_contexts = self.kubectl_runner.as_ref().and_then(|runner| {
                    crate::services::runtime::k9s::resolve_existing_contexts(runner.as_ref())
                });
                EffectResult::K9sLaunchPrepared {
                    target,
                    existing_contexts,
                }
            }
            Effect::RunK9sCredentials { plan } => self.dispatch_k9s_credentials(plan),
            Effect::RefreshSecrets { context, filter } => {
                let Some(runner) = &self.gcloud_runner else {
                    return EffectResult::SecretsLoadFailed {
                        context,
                        error: crate::app::AppError::gcloud(
                            "no gcloud command runner configured".to_string(),
                        ),
                    };
                };
                load_secrets(
                    runner.as_ref(),
                    self.sdk_clients.as_deref(),
                    context,
                    filter,
                )
            }
            Effect::RefreshCloudFunctions { context, filter } => {
                let Some(runner) = &self.gcloud_runner else {
                    return EffectResult::CloudFunctionsLoadFailed {
                        context,
                        error: crate::app::AppError::gcloud(
                            "no gcloud command runner configured".to_string(),
                        ),
                    };
                };
                load_cloud_functions(
                    runner.as_ref(),
                    self.sdk_clients.as_deref(),
                    context,
                    filter,
                )
            }
            Effect::RefreshPersistentDisks { context, filter } => {
                let Some(runner) = &self.gcloud_runner else {
                    return EffectResult::PersistentDisksLoadFailed {
                        context,
                        error: crate::app::AppError::gcloud(
                            "no gcloud command runner configured".to_string(),
                        ),
                    };
                };
                load_persistent_disks(
                    runner.as_ref(),
                    self.sdk_clients.as_deref(),
                    context,
                    filter,
                )
            }
            Effect::RefreshPubSubTopics { context, filter } => {
                let Some(runner) = &self.gcloud_runner else {
                    return EffectResult::PubSubTopicsLoadFailed {
                        context,
                        error: crate::app::AppError::gcloud(
                            "no gcloud command runner configured".to_string(),
                        ),
                    };
                };
                load_pubsub_topics(
                    runner.as_ref(),
                    self.sdk_clients.as_deref(),
                    context,
                    filter,
                )
            }
            Effect::RefreshVpcNetworks { context } => {
                let Some(runner) = &self.gcloud_runner else {
                    return EffectResult::VpcNetworksLoadFailed {
                        context,
                        error: crate::app::AppError::gcloud(
                            "no gcloud command runner configured".to_string(),
                        ),
                    };
                };
                load_vpc_networks(runner.as_ref(), self.sdk_clients.as_deref(), context)
            }
            Effect::RefreshVpcSubnets { context } => {
                let Some(runner) = &self.gcloud_runner else {
                    return EffectResult::VpcSubnetsLoadFailed {
                        context,
                        error: crate::app::AppError::gcloud(
                            "no gcloud command runner configured".to_string(),
                        ),
                    };
                };
                load_vpc_subnets(runner.as_ref(), self.sdk_clients.as_deref(), context)
            }
            Effect::RefreshFirewallRules { context } => {
                let Some(runner) = &self.gcloud_runner else {
                    return EffectResult::FirewallRulesLoadFailed {
                        context,
                        error: crate::app::AppError::gcloud(
                            "no gcloud command runner configured".to_string(),
                        ),
                    };
                };
                load_firewall_rules(runner.as_ref(), self.sdk_clients.as_deref(), context)
            }
            Effect::RefreshPubSubSubscriptions { context, filter } => {
                let Some(runner) = &self.gcloud_runner else {
                    return EffectResult::PubSubSubscriptionsLoadFailed {
                        context,
                        error: crate::app::AppError::gcloud(
                            "no gcloud command runner configured".to_string(),
                        ),
                    };
                };
                load_pubsub_subscriptions(
                    runner.as_ref(),
                    self.sdk_clients.as_deref(),
                    context,
                    filter,
                )
            }
            Effect::RefreshBigQueryDatasets { context } => {
                let Some(runner) = &self.gcloud_runner else {
                    return EffectResult::BigQueryDatasetsLoadFailed {
                        context,
                        error: crate::app::AppError::gcloud(
                            "no gcloud command runner configured".to_string(),
                        ),
                    };
                };
                load_bigquery_datasets(runner.as_ref(), context)
            }
            Effect::LoadBigQueryTables {
                context,
                dataset_id,
                generation,
                listing,
            } => {
                let Some(runner) = &self.gcloud_runner else {
                    return EffectResult::BigQueryTablesLoadFailed {
                        context,
                        generation,
                        listing,
                        error: crate::app::AppError::gcloud(
                            "no gcloud command runner configured".to_string(),
                        ),
                    };
                };
                load_bigquery_tables(runner.as_ref(), context, dataset_id, generation, listing)
            }
            Effect::LoadBigQuerySchema {
                context,
                dataset_id,
                table_id,
                generation,
            } => {
                let Some(runner) = &self.gcloud_runner else {
                    return EffectResult::BigQuerySchemaLoadFailed {
                        context,
                        generation,
                        error: crate::app::AppError::gcloud(
                            "no gcloud command runner configured".to_string(),
                        ),
                    };
                };
                load_bigquery_schema(runner.as_ref(), context, &dataset_id, &table_id, generation)
            }
            // Not reached today: `dispatch_effects` routes this variant to
            // `dispatch_bigquery_table_stats` before the catch-all arm that
            // calls this function. A refusal rather than an `unreachable!`
            // because the consequence of that routing being reordered should
            // be one row's three cells staying unavailable — which is what a
            // denied `bq show` already looks like — and not a panicked run
            // loop taking the session down with it.
            Effect::LoadBigQueryTableStats {
                context,
                table_id,
                generation,
                listing,
                ..
            } => EffectResult::BigQueryTableStatsLoadFailed {
                context,
                table_id,
                generation,
                listing,
                error: crate::app::AppError::gcloud(
                    "table stats are dispatched in the background".to_string(),
                ),
            },
            Effect::RefreshServiceAccounts { context, filter } => {
                let Some(runner) = &self.gcloud_runner else {
                    return EffectResult::ServiceAccountsLoadFailed {
                        context,
                        error: crate::app::AppError::gcloud(
                            "no gcloud command runner configured".to_string(),
                        ),
                    };
                };
                load_service_accounts(
                    runner.as_ref(),
                    self.sdk_clients.as_deref(),
                    context,
                    filter,
                )
            }
            Effect::RefreshIamPolicy { context } => {
                let Some(runner) = &self.gcloud_runner else {
                    return EffectResult::IamPolicyLoadFailed {
                        context,
                        error: crate::app::AppError::gcloud(
                            "no gcloud command runner configured".to_string(),
                        ),
                    };
                };
                load_policy_members(runner.as_ref(), self.sdk_clients.as_deref(), context)
            }
            Effect::LoadSecretDetail { context, secret } => {
                let secret_name = secret.name.clone();
                let Some(runner) = &self.gcloud_runner else {
                    return EffectResult::SecretDetailLoadFailed {
                        context,
                        secret_name,
                        error: crate::app::AppError::gcloud(
                            "no gcloud command runner configured".to_string(),
                        ),
                    };
                };
                if let Some(clients) = self.sdk_clients.as_deref() {
                    let project = context.project.clone().unwrap_or_default();
                    let loaded =
                        clients
                            .secrets(context.configuration.as_str())
                            .and_then(|client| {
                                crate::services::sdk::block_on(
                                    client.load_secret_detail(&project, &secret.name),
                                )
                            });
                    return match loaded {
                        Ok(detail) => EffectResult::SecretDetailLoaded { context, detail },
                        Err(error) => EffectResult::SecretDetailLoadFailed {
                            context,
                            secret_name,
                            error: crate::app::AppError::gcloud(error.to_string()),
                        },
                    };
                }
                let client = GcloudSecretsService::new(runner.as_ref());
                match client.load_secret_detail(&context, &secret.name) {
                    Ok(detail) => EffectResult::SecretDetailLoaded { context, detail },
                    Err(SecretsLoadError::Command(error)) => EffectResult::SecretDetailLoadFailed {
                        context,
                        secret_name,
                        error: crate::app::AppError::gcloud(error.to_string()),
                    },
                    Err(SecretsLoadError::Parse(error)) => EffectResult::SecretDetailLoadFailed {
                        context,
                        secret_name,
                        error: crate::app::AppError::Parse(error.to_string()),
                    },
                }
            }
            Effect::LoadSecretVersions { context, secret } => {
                let secret_name = secret.name.clone();
                let Some(runner) = &self.gcloud_runner else {
                    return EffectResult::SecretVersionsLoadFailed {
                        context,
                        secret_name,
                        error: crate::app::AppError::gcloud(
                            "no gcloud command runner configured".to_string(),
                        ),
                    };
                };
                if let Some(clients) = self.sdk_clients.as_deref() {
                    let project = context.project.clone().unwrap_or_default();
                    let loaded =
                        clients
                            .secrets(context.configuration.as_str())
                            .and_then(|client| {
                                crate::services::sdk::block_on(
                                    client.load_secret_versions(&project, &secret.name),
                                )
                            });
                    return match loaded {
                        Ok(versions) => EffectResult::SecretVersionsLoaded {
                            context,
                            secret_name,
                            versions,
                        },
                        Err(error) => EffectResult::SecretVersionsLoadFailed {
                            context,
                            secret_name,
                            error: crate::app::AppError::gcloud(error.to_string()),
                        },
                    };
                }
                let client = GcloudSecretsService::new(runner.as_ref());
                match client.load_secret_versions(&context, &secret.name) {
                    Ok(versions) => EffectResult::SecretVersionsLoaded {
                        context,
                        secret_name,
                        versions,
                    },
                    Err(SecretsLoadError::Command(error)) => {
                        EffectResult::SecretVersionsLoadFailed {
                            context,
                            secret_name,
                            error: crate::app::AppError::gcloud(error.to_string()),
                        }
                    }
                    Err(SecretsLoadError::Parse(error)) => EffectResult::SecretVersionsLoadFailed {
                        context,
                        secret_name,
                        error: crate::app::AppError::Parse(error.to_string()),
                    },
                }
            }
            Effect::RefreshCloudRunServices { context } => {
                let Some(runner) = &self.gcloud_runner else {
                    return EffectResult::CloudRunServicesLoadFailed {
                        context,
                        error: crate::app::AppError::gcloud(
                            "no gcloud command runner configured".to_string(),
                        ),
                    };
                };
                load_cloud_run_services(runner.as_ref(), self.sdk_clients.as_deref(), context)
            }
            Effect::LoadCloudRunServiceDetail { context, service } => {
                let service_name = service.name.clone();
                let service_region = service.region.clone();
                let Some(runner) = &self.gcloud_runner else {
                    return EffectResult::CloudRunServiceDetailLoadFailed {
                        context,
                        service_name,
                        service_region,
                        error: crate::app::AppError::gcloud(
                            "no gcloud command runner configured".to_string(),
                        ),
                    };
                };
                if let Some(clients) = self.sdk_clients.as_deref() {
                    let project = context.project.clone().unwrap_or_default();
                    let loaded =
                        clients
                            .cloud_run(context.configuration.as_str())
                            .and_then(|client| {
                                crate::services::sdk::block_on(client.load_service_detail(
                                    &project,
                                    &service.region,
                                    &service.name,
                                ))
                            });
                    return match loaded {
                        Ok(detail) => EffectResult::CloudRunServiceDetailLoaded { context, detail },
                        Err(error) => EffectResult::CloudRunServiceDetailLoadFailed {
                            context,
                            service_name,
                            service_region,
                            error: crate::app::AppError::gcloud(error.to_string()),
                        },
                    };
                }
                let service_client = GcloudCloudRunService::new(runner.as_ref());
                match service_client.load_service_detail(&context, &service) {
                    Ok(detail) => EffectResult::CloudRunServiceDetailLoaded { context, detail },
                    Err(CloudRunLoadError::Command(error)) => {
                        EffectResult::CloudRunServiceDetailLoadFailed {
                            context,
                            service_name,
                            service_region,
                            error: crate::app::AppError::gcloud(error.to_string()),
                        }
                    }
                    Err(CloudRunLoadError::Parse(error)) => {
                        EffectResult::CloudRunServiceDetailLoadFailed {
                            context,
                            service_name,
                            service_region,
                            error: crate::app::AppError::Parse(error.to_string()),
                        }
                    }
                }
            }
            Effect::LoadCloudRunRevisions { context, service } => {
                let service_name = service.name.clone();
                let service_region = service.region.clone();
                let Some(runner) = &self.gcloud_runner else {
                    return EffectResult::CloudRunRevisionsLoadFailed {
                        context,
                        service_name,
                        service_region,
                        error: crate::app::AppError::gcloud(
                            "no gcloud command runner configured".to_string(),
                        ),
                    };
                };
                if let Some(clients) = self.sdk_clients.as_deref() {
                    let project = context.project.clone().unwrap_or_default();
                    let loaded =
                        clients
                            .cloud_run(context.configuration.as_str())
                            .and_then(|client| {
                                crate::services::sdk::block_on(client.load_revisions(
                                    &project,
                                    &service.region,
                                    &service.name,
                                ))
                            });
                    return match loaded {
                        Ok(revisions) => EffectResult::CloudRunRevisionsLoaded {
                            context,
                            service_name,
                            service_region,
                            revisions,
                        },
                        Err(error) => EffectResult::CloudRunRevisionsLoadFailed {
                            context,
                            service_name,
                            service_region,
                            error: crate::app::AppError::gcloud(error.to_string()),
                        },
                    };
                }
                let service_client = GcloudCloudRunService::new(runner.as_ref());
                match service_client.load_revisions(&context, &service) {
                    Ok(revisions) => EffectResult::CloudRunRevisionsLoaded {
                        context,
                        service_name,
                        service_region,
                        revisions,
                    },
                    Err(CloudRunLoadError::Command(error)) => {
                        EffectResult::CloudRunRevisionsLoadFailed {
                            context,
                            service_name,
                            service_region,
                            error: crate::app::AppError::gcloud(error.to_string()),
                        }
                    }
                    Err(CloudRunLoadError::Parse(error)) => {
                        EffectResult::CloudRunRevisionsLoadFailed {
                            context,
                            service_name,
                            service_region,
                            error: crate::app::AppError::Parse(error.to_string()),
                        }
                    }
                }
            }
            Effect::LoadCloudRunRevisionDetail {
                context,
                region,
                revision,
            } => {
                let Some(runner) = &self.gcloud_runner else {
                    return EffectResult::CloudRunRevisionDetailLoadFailed {
                        context,
                        revision_name: revision,
                        region,
                        error: crate::app::AppError::gcloud(
                            "no gcloud command runner configured".to_string(),
                        ),
                    };
                };
                let service_client = GcloudCloudRunService::new(runner.as_ref());
                if let Some(clients) = self.sdk_clients.as_deref() {
                    let project = context.project.clone().unwrap_or_default();
                    let loaded =
                        clients
                            .cloud_run(context.configuration.as_str())
                            .and_then(|client| {
                                crate::services::sdk::block_on(
                                    client.load_revision_detail(&project, &region, &revision),
                                )
                            });
                    return match loaded {
                        Ok(detail) => {
                            EffectResult::CloudRunRevisionDetailLoaded { context, detail }
                        }
                        Err(error) => EffectResult::CloudRunRevisionDetailLoadFailed {
                            context,
                            revision_name: revision,
                            region,
                            error: crate::app::AppError::gcloud(error.to_string()),
                        },
                    };
                }
                match service_client.load_revision_detail(&context, &region, &revision) {
                    Ok(detail) => EffectResult::CloudRunRevisionDetailLoaded { context, detail },
                    Err(CloudRunLoadError::Command(error)) => {
                        EffectResult::CloudRunRevisionDetailLoadFailed {
                            context,
                            revision_name: revision,
                            region,
                            error: crate::app::AppError::gcloud(error.to_string()),
                        }
                    }
                    Err(CloudRunLoadError::Parse(error)) => {
                        EffectResult::CloudRunRevisionDetailLoadFailed {
                            context,
                            revision_name: revision,
                            region,
                            error: crate::app::AppError::Parse(error.to_string()),
                        }
                    }
                }
            }
            Effect::RunResourceBatch { .. } => {
                unreachable!("resource batch is dispatched in dispatch_effects")
            }
            Effect::StartTunnel { .. } | Effect::StopTunnel { .. } => {
                unreachable!("tunnel effects are handled in dispatch_effects (Task 5)")
            }
            Effect::ReloadConfig => {
                unreachable!("config reload is handled synchronously in dispatch_effects")
            }
        }
    }

    /// Runs `get-credentials` when required (captured, non-interactive). Success
    /// -> `K9sCredentialsReady` (the reducer then dispatches the interactive
    /// launch); failure -> `K9sCredentialsFailed`, which prevents the launch.
    fn dispatch_k9s_credentials(
        &self,
        plan: nuvem_domain::gcp::k9s::K9sLaunchPlan,
    ) -> EffectResult {
        if plan.credential_step == CredentialStep::RunGetCredentials {
            let Some(runner) = &self.gcloud_runner else {
                return EffectResult::K9sCredentialsFailed {
                    cluster_name: plan.target.cluster_name.clone(),
                    error: AppError::gcloud("no gcloud command runner configured"),
                };
            };
            if let Err(error) = runner.run(GcloudCommand::new(plan.target.credentials_argv())) {
                return EffectResult::K9sCredentialsFailed {
                    cluster_name: plan.target.cluster_name.clone(),
                    error: AppError::from(error),
                };
            }
        }
        EffectResult::K9sCredentialsReady {
            kube_context: plan.target.kube_context.clone(),
            cluster_name: plan.target.cluster_name.clone(),
        }
    }

    /// Spawns a bounded-concurrency, generation-guarded background runner for a
    /// batch of resource actions. Each target acquires a permit from a shared
    /// semaphore, runs its action off the async runtime, and reports a
    /// [`QueuedEffectResult::Batch`] tagged with the generations captured here.
    ///
    /// The service is resolved exactly once, before the loop: matching
    /// [`BatchAction`] selects the per-target runner and the join-error
    /// fallback, and everything else — the generation bump, the semaphore, the
    /// cancellation checks, the `spawn_blocking` hop — is shared by every
    /// service.
    fn dispatch_resource_batch(
        &mut self,
        context: GcloudContext,
        action: BatchAction,
        targets: Vec<(BatchRowRef, u64)>,
        concurrency: usize,
    ) {
        let target_runner = BatchTargetRunner::for_action(action);
        // A batch still running when this one is confirmed is superseded here,
        // and superseding it is what strands it: the batch generation moves on,
        // which drops every result the outgoing batch has yet to report, and
        // `batch_targets` is replaced wholesale, so afterwards nothing is left
        // that could settle its operations-log entries. They are settled now,
        // while the claims that say what is provable about each target are
        // still in hand.
        //
        // Deliberately not `cancel_batch`: that would also clear the
        // batch-progress workflow, which by now belongs to the batch being
        // confirmed here, not the one being superseded. See
        // [`Self::detach_and_settle_batch`].
        //
        // `batch_generation` is therefore bumped twice on this path — once
        // inside the settling half, once on the line below — and that is
        // harmless rather than an oversight to tidy away. It is compared in
        // exactly one place, the equality guard in `process_next_effect_result`
        // — the only other read is the stamp taken below, after both bumps — so
        // any non-zero number of bumps between two batches is the same guard. Each
        // half keeps its own bump so that neither depends on the other having
        // run: detaching invalidates the outgoing batch wherever it is called
        // from, and this dispatch stamps the generation its own targets carry.
        self.detach_and_settle_batch(BatchDetachReason::Superseded);
        self.batch_generation = self.batch_generation.wrapping_add(1);
        let batch_generation = self.batch_generation;
        let context_generation = self.context_generation;
        let scope_generation = self.scope_generation;
        let limit = if concurrency == 0 {
            self.batch_concurrency
        } else {
            concurrency
        }
        .max(1);
        let semaphore = Arc::new(Semaphore::new(limit));
        // Replaced wholesale, never appended to: a claim left over from an
        // earlier batch would be read as "this target's command was issued" and
        // turn a plainly cancelled entry into a spurious `Unknown`.
        self.batch_targets = Vec::with_capacity(targets.len());
        for (target, operation_id) in targets {
            let claim = Arc::new(BatchTargetClaim::new());
            self.batch_targets.push(InFlightBatchTarget {
                operation_id,
                claim: claim.clone(),
            });
            let runner = self.gcloud_runner.clone();
            // Threaded per target, like the runner: the same native-when-held
            // rule as the single-row path and as every read.
            let sdk = self.sdk_clients.clone();
            let sender = self.effect_result_sender.clone();
            let context = context.clone();
            let semaphore = semaphore.clone();
            let cancellation = self.batch_cancellation.clone();
            // A second, unrelated token. `batch_cancellation` says "this batch
            // has been taken away" and is replaced on every detach; this one
            // says "the process is leaving" and is never replaced. Merging them
            // would make a context switch kill children mid-command, which
            // this deliberately does not do.
            let shutdown = self.lifecycle_cancellation.clone();
            let target_runner = target_runner.clone();
            tokio::spawn(async move {
                let Some(permit) = claim_batch_target_slot(cancellation, semaphore, &claim).await
                else {
                    return;
                };
                let BatchTargetRunner { run, join_error } = target_runner;
                let join_error_context = context.clone();
                let join_error_target = target.clone();
                let result = tokio::task::spawn_blocking(move || {
                    let _permit = permit;
                    run(
                        Backends { runner, sdk },
                        context,
                        target,
                        operation_id,
                        shutdown,
                    )
                })
                .await
                // A panicking action task must still resolve this target: with
                // no result sent, the target's batch-progress entry and its
                // operations-log entry would stay `Pending` forever, and the
                // workflow would never reach `is_complete()` to fire the
                // follow-up refresh. Report it as a failed action instead, the
                // same way `dispatch_background_refresh` handles a join error.
                .unwrap_or_else(|error| {
                    join_error(
                        join_error_context,
                        join_error_target,
                        AppError::gcloud(format!("batch action task failed: {error}")),
                        operation_id,
                    )
                });
                let _ = sender.send(QueuedEffectResult::Batch {
                    context_generation,
                    scope_generation,
                    batch_generation,
                    result,
                });
            });
        }
    }

    /// Applies one batch target result: it resolves the operations-log entry
    /// and the batch-progress target through the reducer, but the per-result
    /// follow-up refresh is intentionally dropped so exactly one refresh is
    /// issued once the whole batch completes.
    async fn apply_batch_effect_result(&mut self, result: EffectResult) {
        let result_was_accepted = self.effect_result_is_accepted(&result);
        // Discarding the reducer's effects is safe only because of what a batch
        // result can do: it resolves an operations-log entry and a
        // batch-progress target, and neither changes the active configuration
        // nor the active service. The remembered-view choke point in `reduce`
        // emits `Effect::SaveRememberedViews` exactly when one of those two
        // changes, so today it emits nothing here and nothing is lost.
        //
        // A batch result that *did* change either — an action that reactivates
        // a configuration, or one that lands the user on another service —
        // would have its save silently dropped on the floor here, which is the
        // one hazard the single-choke-point design exists to prevent. Route the
        // effects out of this function before adding such a result.
        let _per_result_effects = self.reduce(&Action::EffectResult(result.clone()));
        if result_was_accepted {
            self.complete_effect_operation(&result, true);
        }
        // Read the action off the still-live workflow: the refresh that
        // follows tears it down, and by then the action is gone. Matching
        // `BatchAction` rather than its `service_kind()` keeps this match
        // exhaustive without a catch-all, so a new batchable service gets a
        // compile error here instead of a runtime panic in the run loop.
        let completed_action = self
            .app
            .root
            .resources
            .batch_workflow()
            .filter(|workflow| workflow.is_complete())
            .map(|workflow| workflow.action());
        if let Some(action) = completed_action
            && let Some(context) = self.app.root.current_context()
        {
            // Same rule as every other reload: carry the batched
            // service's own applied filter, whether or not it is still the
            // active service. A batch runs over marked rows the user
            // narrowed with `/`, and in label mode the narrowing lives
            // entirely in the pushed `--filter`, so an unfiltered
            // end-of-batch reload would silently widen that service's
            // stored rows back to the whole project.
            let filter = crate::app::reducer::resources::active_filter_for(
                &self.app.root,
                action.service_kind(),
            );
            let refresh = match action {
                BatchAction::Compute(_) => Effect::RefreshComputeInstances { context, filter },
                BatchAction::CloudSql(_) => Effect::RefreshCloudSqlInstances { context, filter },
            };
            self.dispatch_effects(vec![refresh], true).await;
        }
        self.ensure_active_resource_loaded().await;
    }

    /// Launches one managed tunnel child and puts a supervisor behind it.
    ///
    /// Everything that can fail before there is a child to supervise is
    /// reported here as `TunnelFailed`, because the reducer has already put a
    /// `Starting` row on screen: a silent return would leave it starting
    /// forever.
    fn dispatch_start_tunnel(
        &mut self,
        id: TunnelId,
        context: GcloudContext,
        plan: crate::app::TunnelLaunchPlan,
    ) {
        let local_port = match allocate_local_port(
            plan.preferred_local_port,
            &plan.claimed_ports,
            self.port_probe.as_ref(),
        ) {
            Ok(local_port) => local_port,
            Err(error) => {
                self.send_tunnel_result(EffectResult::TunnelFailed {
                    id,
                    message: error.to_string(),
                });
                return;
            }
        };

        // Published before the child is launched, not after it reports ready:
        // `claimed_ports` is what stops a tunnel started during this one's
        // multi-second startup from being handed the same port, and until the
        // allocation is in state the row still carries the placeholder the
        // reducer inserted (`0` for the kinds with no preferred port).
        self.send_tunnel_result(EffectResult::TunnelPortAllocated { id, local_port });

        let spec = self.tunnel_spec(&context, &plan, local_port);
        let launched = match self.tunnel_launcher.launch(&spec) {
            Ok(launched) => launched,
            Err(TunnelLaunchError::MissingBinary) => {
                // Distinguished from every other launch failure so the row can
                // say "install it" instead of showing an errno.
                self.send_tunnel_result(EffectResult::TunnelFailed {
                    id,
                    // Config-sourced rather than child output, so the risk is
                    // lower — but every tunnel message reaching app state goes
                    // through the same bound, with no exceptions to audit.
                    message: bounded_tunnel_message(&format!("{}: binary not found", spec.program)),
                });
                return;
            }
            Err(TunnelLaunchError::LaunchFailed { message }) => {
                self.send_tunnel_result(EffectResult::TunnelFailed {
                    id,
                    message: bounded_tunnel_message(&message),
                });
                return;
            }
        };

        let LaunchedTunnel { output, control } = launched;
        self.tunnels.register(id, Arc::clone(&control));
        if readiness(plan.kind) == TunnelReadiness::OnSpawn {
            // `ssh -N` prints nothing at all on success, so a healthy spawn is
            // the only readiness signal there will ever be — and the readiness
            // deadline below is never armed for it.
            self.send_tunnel_result(EffectResult::TunnelReady {
                id,
                local_port: Some(local_port),
            });
        }
        self.spawn_tunnel_supervisor(id, plan.kind, local_port, output, control);
    }

    /// Builds the argv for one tunnel from the domain planners. The port that
    /// reaches the child is the *allocated* one, never the requested one.
    fn tunnel_spec(
        &self,
        context: &GcloudContext,
        plan: &crate::app::TunnelLaunchPlan,
        local_port: u16,
    ) -> TunnelSpec {
        let configuration = context.configuration.as_str();
        match plan.kind {
            TunnelKind::IapTcp => iap_tunnel_spec(
                &self.gcloud_binary(),
                configuration,
                &plan.instance_name,
                &plan.zone,
                plan.remote_port,
                local_port,
            ),
            TunnelKind::SshLocalForward => ssh_local_forward_spec(
                &self.gcloud_binary(),
                configuration,
                &plan.instance_name,
                &plan.zone,
                local_port,
                plan.remote_port,
            ),
            TunnelKind::CloudSqlProxy => cloud_sql_proxy_spec(
                &self.cloud_sql_proxy_binary(),
                &plan.connection_name,
                local_port,
            ),
        }
    }

    fn gcloud_binary(&self) -> String {
        self.config
            .as_ref()
            .map(|config| config.gcloud_binary.to_string_lossy().into_owned())
            .unwrap_or_else(|| "gcloud".to_string())
    }

    fn cloud_sql_proxy_binary(&self) -> String {
        self.config
            .as_ref()
            .map(|config| config.cloud_sql_proxy_binary.to_string_lossy().into_owned())
            .unwrap_or_else(|| "cloud-sql-proxy".to_string())
    }

    /// Supervises one live tunnel: a blocking pump over its output and an async
    /// task that classifies each line against a bounded readiness deadline.
    ///
    /// The split is forced by the seam: `TunnelOutput::next_line` blocks, so it
    /// belongs on a `spawn_blocking` thread and never on the reactor, while the
    /// deadline needs a timer only the async side has.
    fn spawn_tunnel_supervisor(
        &self,
        id: TunnelId,
        kind: TunnelKind,
        local_port: u16,
        output: Box<dyn TunnelOutput>,
        control: Arc<dyn TunnelControl>,
    ) {
        let sender = self.effect_result_sender.clone();
        let readiness_timeout = self.readiness_timeout;
        let (line_sender, mut lines) = mpsc::unbounded_channel::<String>();

        // The pump owns the ONLY sender: EOF reaches the supervisor as
        // `recv() == None`, which happens exactly when this sender is dropped.
        // A clone kept anywhere else would mean the supervisor waits forever on
        // a child that is already gone.
        tokio::task::spawn_blocking(move || {
            let mut output = output;
            while let Some(line) = output.next_line() {
                if line_sender.send(line).is_err() {
                    // The supervisor is gone (the tunnel already failed or was
                    // stopped), so there is nobody left to read for.
                    break;
                }
            }
        });

        tokio::spawn(async move {
            // `OnSpawn` kinds were reported ready at launch, so their deadline
            // starts disabled and is never armed.
            let mut is_ready = readiness(kind) == TunnelReadiness::OnSpawn;
            let mut tail: VecDeque<String> = VecDeque::new();
            // One deadline for the whole startup, created once: re-creating it
            // per line would let a chatty child that never comes up push the
            // deadline out forever.
            let readiness_deadline = tokio::time::sleep(readiness_timeout);
            tokio::pin!(readiness_deadline);

            loop {
                let line = tokio::select! {
                    line = lines.recv() => line,
                    _ = &mut readiness_deadline, if !is_ready => {
                        // Report first, then stop the child: a tunnel must never
                        // be left stuck in `Starting`, and the shutdown below
                        // can take up to the stop grace.
                        let _ = sender.send(QueuedEffectResult::Tunnel {
                            result: EffectResult::TunnelFailed {
                                id,
                                message: readiness_timeout_message(readiness_timeout, &tail),
                            },
                        });
                        shutdown_supervised_tunnel(&control, "the readiness deadline expired").await;
                        return;
                    }
                };

                let Some(line) = line else {
                    // Both pipes closed: the child is gone. Whether that is a
                    // failure depends on what the tunnel was doing, which only
                    // the reducer knows.
                    let _ = sender.send(QueuedEffectResult::Tunnel {
                        result: EffectResult::TunnelExited { id },
                    });
                    return;
                };

                match classify_tunnel_line(kind, &line) {
                    TunnelSignal::Ready {
                        local_port: reported,
                    } => {
                        is_ready = true;
                        let _ = sender.send(QueuedEffectResult::Tunnel {
                            result: EffectResult::TunnelReady {
                                id,
                                // The child's own port wins; the allocated one
                                // is the fallback when it did not print one.
                                local_port: reported.or(Some(local_port)),
                            },
                        });
                    }
                    // Two tiers, because one rule cannot serve both halves of a
                    // tunnel's life.
                    //
                    // *Before readiness* every error line is fatal. The
                    // classifier is deliberately liberal there — a bare "error"
                    // or "failed" anywhere in a line is enough — because that
                    // is what stops a drifting startup banner from hanging the
                    // tunnel in `Starting` forever.
                    //
                    // *After readiness* that liberality would kill working
                    // tunnels: the same lines are ordinary chatter from a
                    // healthy child (`ssh` reporting one refused forwarded
                    // connection, `gcloud`'s `...failed.` spinner text). So
                    // only the narrow always-fatal tier still fails a ready
                    // tunnel — lines that mean it cannot carry traffic at all
                    // (a bind failure, a permission/IAP denial, a host-key or
                    // authentication failure). That tier is what gives
                    // `OnSpawn` kinds any error detection whatsoever: `ssh -L`
                    // is ready from the instant it spawns, so `!is_ready` is
                    // never true for it and a forward that could not bind
                    // would otherwise sit `Active` forever.
                    //
                    // The classifier stays a pure "what does this line look
                    // like?"; the policy of what to do about it lives here.
                    TunnelSignal::Failed { message }
                        if !is_ready || is_always_fatal_tunnel_line(&line) =>
                    {
                        // Already sanitized and bounded by the classifier.
                        let _ = sender.send(QueuedEffectResult::Tunnel {
                            result: EffectResult::TunnelFailed { id, message },
                        });
                        shutdown_supervised_tunnel(&control, "the child reported a failure").await;
                        return;
                    }
                    // A broad-marker line on a tunnel that is already up. It is
                    // deliberately not acted on — but it must not vanish
                    // either: `tail` is read only by the readiness-deadline
                    // arm, which is unreachable once `is_ready`, so a line kept
                    // only there would be invisible to the user and a
                    // healthy-then-broken tunnel would end with no diagnostics
                    // at all. It goes to the row's MESSAGE as a *notice*
                    // instead: the status is left exactly where it was.
                    TunnelSignal::Failed { message } => {
                        tracing::debug!(
                            tunnel_id = %id,
                            %message,
                            "a ready tunnel reported an error-shaped line"
                        );
                        // Already sanitized and bounded by the classifier.
                        let _ = sender.send(QueuedEffectResult::Tunnel {
                            result: EffectResult::TunnelNotice { id, message },
                        });
                        push_tunnel_tail_line(&mut tail, line);
                    }
                    TunnelSignal::Ignored => {
                        push_tunnel_tail_line(&mut tail, line);
                    }
                }
            }
        });
    }

    /// Stops one managed tunnel, off the run loop.
    ///
    /// `TunnelControl::shutdown` blocks for the whole terminate-poll-kill
    /// sequence — up to the grace plus a reap window for a child that ignores
    /// `SIGTERM`. Awaiting that here would freeze `dispatch_action`, and with it
    /// the frame the run loop draws next, for seconds after a single keypress.
    /// So the stop is *dispatched*, not awaited: the reducer has already marked
    /// the row `Stopping`, and the outcome lands later through the same
    /// effect-result channel every other tunnel result uses.
    ///
    /// The control is cloned out of the registry and deliberately **left
    /// registered** for the duration. Deregistering it up front would hand the
    /// blocking task the only remaining handle to that child, so a quit or a
    /// `Drop` in the window before the stop finished would find nothing to reap
    /// — the one way this subsystem's no-leak invariant fails silently. Two
    /// callers reaching the same control is safe by contract: `shutdown` is
    /// idempotent, and the second one gets `AlreadyExited`.
    fn dispatch_stop_tunnel(&self, id: TunnelId) {
        let Some(control) = self.tunnels.control(id) else {
            // Nothing is registered under that id — the child is already gone
            // (it exited on its own, or a previous stop reaped it). The user
            // asked for the tunnel to be gone, and it is.
            tracing::debug!(tunnel_id = %id, "no live tunnel to stop");
            self.send_tunnel_result(EffectResult::TunnelStopped { id });
            return;
        };
        let sender = self.effect_result_sender.clone();
        // `spawn_blocking` rather than `tokio::spawn`: the work is blocking, and
        // this way it starts on a blocking thread without the reactor having to
        // poll anything first. It *returns* the result rather than sending it,
        // so the send below is the single place a stop is reported from and the
        // join error has somewhere to be handled — see the awaiting task.
        let shutdown = tokio::task::spawn_blocking(move || {
            match control.shutdown(TUNNEL_STOP_GRACE) {
                // The child may still be running, so the row must not claim a
                // clean stop. `Failed` is not chosen for being *less* terminal
                // than `Stopped` — both are terminal, so either way the port is
                // released and `d` dismisses the row. What it buys is that the
                // shutdown's own reason reaches the MESSAGE column instead of a
                // stop the user would believe; the control stays registered
                // either way, so quit and the `Drop` backstop can still reach a
                // child that is probably still alive.
                //
                // The residual, stated plainly: between this failed stop and the
                // backstop that finally reaps the child, the released port can
                // be handed to a new tunnel while the old child may still hold
                // it. The new tunnel then fails to bind and says so, which is a
                // worse outcome than a clean stop and a better one than a row
                // that lies.
                TunnelShutdown::Failed { message } => {
                    tracing::warn!(tunnel_id = %id, %message, "the managed tunnel stop failed");
                    EffectResult::TunnelFailed {
                        id,
                        message: bounded_tunnel_message(&message),
                    }
                }
                // Every other outcome ends the same way, `AlreadyExited`
                // included: the user asked for the tunnel to be gone, and it is.
                outcome => {
                    tracing::debug!(tunnel_id = %id, ?outcome, "managed tunnel stopped");
                    EffectResult::TunnelStopped { id }
                }
            }
        });
        // The join is awaited rather than dropped, and that is the whole point:
        // the row is `Stopping` and nothing else in the app will ever move it,
        // so if the blocking task unwinds and no result is sent, the tunnel is
        // stuck for the rest of the session — `d` will not dismiss a non-
        // terminal row, its port stays claimed, and a second `K` is refused.
        // Today's controls do not panic; this is what keeps that from being a
        // load-bearing assumption when a future one does.
        //
        // `tokio::spawn`, not an `.await` here: the blocking sequence must stay
        // off the run loop. This task only parks on the join.
        tokio::spawn(async move {
            let result = shutdown.await.unwrap_or_else(|error| {
                // A panicked or cancelled stop means the child's fate is
                // genuinely unknown — quite unlike the `Ok` path, where the
                // control reported what happened. `Failed` says exactly that
                // and carries the reason into the MESSAGE column, where
                // `Stopped` would assert a clean stop nobody observed. Both are
                // terminal, so either way the row can be dismissed and the port
                // is released; the control also stays registered either way, so
                // quit and the `Drop` backstop still reach the child.
                tracing::warn!(tunnel_id = %id, %error, "the managed tunnel stop task failed");
                EffectResult::TunnelFailed {
                    id,
                    // The join error carries the panic payload, which came from
                    // a control that may have folded child output into it, so
                    // it goes through the same sanitizing bound as every other
                    // message that reaches a row.
                    message: bounded_tunnel_message(&format!("the tunnel stop failed: {error}")),
                }
            });
            let _ = sender.send(QueuedEffectResult::Tunnel { result });
        });
    }

    /// Shuts every live tunnel down, off the reactor and with a bounded grace.
    async fn shutdown_all_tunnels(&mut self) {
        if self.tunnels.is_empty() {
            return;
        }
        let registry = std::mem::take(&mut self.tunnels);
        let outcomes = tokio::task::spawn_blocking(move || {
            let mut registry = registry;
            registry.shutdown_all(TUNNEL_STOP_GRACE)
        })
        .await
        .unwrap_or_else(|error| {
            tracing::warn!(%error, "the tunnel shutdown task failed on quit");
            Vec::new()
        });
        for (id, outcome) in outcomes {
            tracing::debug!(tunnel_id = %id, ?outcome, "tunnel shut down on quit");
        }
    }

    /// Queues a tunnel result on the existing effect-result channel. There is
    /// deliberately no second event path: the run loop already drains this one.
    fn send_tunnel_result(&self, result: EffectResult) {
        let _ = self
            .effect_result_sender
            .send(QueuedEffectResult::Tunnel { result });
    }

    /// Cancels every in-flight plugin run and settles the operations-log
    /// entries they would otherwise strand `Pending` for the life of the
    /// process.
    ///
    /// The shape is [`Self::detach_and_settle_batch`]'s, for the same reason:
    /// once the token is cancelled nothing will report these entries' outcomes,
    /// so this is the last chance to say something true about them, and each
    /// run's [`BatchTargetClaim`] is the whole of what we can prove. Claiming
    /// here is what makes the answer binding — the worker can no longer issue a
    /// command behind an entry that reads `Cancelled`.
    fn cancel_running_plugins(&mut self) {
        // Taken, not borrowed: these handles describe the runs being cancelled
        // and must not outlive them.
        let in_flight = std::mem::take(&mut self.plugin_runs);
        if in_flight.is_empty() {
            self.app
                .root
                .system
                .set_status_message("No plugin is running");
            return;
        }
        self.plugin_cancellation.cancel();
        // A fresh token, so a plugin confirmed after this cancellation is not
        // born cancelled.
        self.plugin_cancellation = CancellationToken::new();

        let cancelled = in_flight.len();
        for handle in in_flight {
            let issued = !handle.claim.claim_cancel();
            let (outcome, detail) = if issued {
                (
                    OperationOutcome::Unknown,
                    "the plugin was cancelled after its command was issued; it may have already changed something",
                )
            } else {
                (
                    OperationOutcome::Cancelled,
                    "the plugin was cancelled before its command ran",
                )
            };
            // The return value is deliberately ignored, exactly as the batch
            // ignores it: the log is bounded and evicts from the back, so an id
            // can outlive its record, and a missing or already-settled record
            // is a no-op rather than an error.
            self.app.root.system.operations.resolve_by_id(
                handle.operation_id,
                outcome,
                Some(detail.to_string()),
            );
        }
        self.app.root.system.set_status_message(if cancelled == 1 {
            "Cancelled the running plugin".to_string()
        } else {
            format!("Cancelled {cancelled} running plugins")
        });
    }

    /// Cancels every in-flight plugin run on the way out of the process.
    ///
    /// Not `Self::cancel_running_plugins`: that one is the operator's key, and
    /// everything it does beyond flipping the token — settling the
    /// operations-log entries, replacing the token so a later run is not born
    /// cancelled, writing a status line — assumes a session that keeps running.
    /// On shutdown there is no reader left for any of it, and settling would be
    /// inventing outcomes nobody will see.
    ///
    /// Flipping the token is the part that has to happen here, and it has to
    /// happen before the runtime is dropped: a plugin's worker sits on a
    /// `spawn_blocking` thread, `Runtime::drop` blocks on every started one, and
    /// a user-declared argv can wait forever. Cancelling lets
    /// `run_with_cancellation` kill the child's process group and return, so the
    /// operator who pressed `q` rather than Ctrl-x gets a process that exits.
    pub fn cancel_plugins_for_shutdown(&mut self) {
        self.plugin_cancellation.cancel();
    }

    /// Stops waiting on every in-flight lifecycle action on the way out of the
    /// process — a batch of VM stops, or a single row's `stop`/failover.
    ///
    /// The same argument as [`Self::cancel_plugins_for_shutdown`], for the same
    /// reason: the worker sits on a `spawn_blocking` thread and `Runtime::drop`
    /// blocks on every started one. A lifecycle command carries a thirty-minute
    /// ceiling, so before this the operator's `q` restored the
    /// terminal, returned from `run()`, and then hung with nothing on screen to
    /// explain it — and with `SIGTERM` already swallowed by the tokio handler
    /// `TerminationSignals::install` registered, only `kill -9` got out.
    ///
    /// Nothing is settled here, exactly as on the plugin path: the results have
    /// no reader left, and inventing outcomes nobody will see is not honesty.
    ///
    /// What this does NOT do, and what the code it releases must not claim: it
    /// does not cancel the operation. A `stop` gcloud has already handed to GCP
    /// is carried out there whatever happens to the CLI; all that is abandoned
    /// is nuvem's wait for the answer. `CommandError::Cancelled` carries the
    /// command's ceiling so the message says so.
    pub fn cancel_lifecycle_actions_for_shutdown(&mut self) {
        self.lifecycle_cancellation.cancel();
    }

    /// Detaches the in-flight batch and settles every operations-log entry it
    /// would otherwise strand `Pending`, leaving the batch-progress workflow
    /// untouched.
    ///
    /// The workflow is deliberately not cleared here, and that is the whole
    /// reason this half exists separately from [`Self::cancel_batch`]. A batch
    /// is also detached when a *new* batch supersedes it, and on that path the
    /// reducer has already installed the incoming batch's workflow by the time
    /// the dispatch runs (`begin_batch_workflow` happens before
    /// `Effect::RunResourceBatch` is emitted). Clearing here would therefore
    /// destroy the workflow of the batch the operator just confirmed: no
    /// progress overlay for it, and no end-of-batch refresh, because
    /// `apply_batch_effect_result` reads the workflow to fire that refresh.
    /// Merging this back into `cancel_batch` recreates exactly that defect.
    ///
    /// Each target carries its own operations-log id, so nothing here reads the
    /// batch-progress workflow. That is the point: the workflow does not
    /// reliably survive the reducer. Re-activating the already-active
    /// configuration resolves to the same context, so the reducer takes no
    /// cancelling branch, yet its teardown still nulls the workflow — after
    /// which a later scope switch cancelled a batch whose ids had gone with it,
    /// stranding every entry `Pending` for the life of the process.
    ///
    /// Two things guarantee no result ever arrives for these targets: the
    /// cancellation token stops unstarted ones from issuing a command at all,
    /// and the generation bump drops the results of the ones already running.
    /// Whatever this leaves `Pending` therefore stays `Pending` for the life of
    /// the process, so detachment is the last chance to say something true
    /// about it — and each target's [`BatchTargetClaim`] is the whole of what
    /// we can prove. Claiming a target here is what makes the answer binding:
    /// its worker can no longer issue a command behind a `Cancelled` entry.
    /// `reason` selects the detail wording only; both paths settle at the same
    /// two outcomes, because the outcome is what the claim proves and the reason
    /// cannot change that.
    fn detach_and_settle_batch(&mut self, reason: BatchDetachReason) {
        self.batch_generation = self.batch_generation.wrapping_add(1);
        self.batch_cancellation.cancel();
        self.batch_cancellation = CancellationToken::new();

        // Taken, not borrowed: these claims describe the batch being detached
        // and must not outlive it.
        let in_flight = std::mem::take(&mut self.batch_targets);
        let resolutions: Vec<(u64, OperationOutcome, &'static str)> = in_flight
            .into_iter()
            .map(|target| {
                // Every target is swept, including the ones that already
                // finished. Filtering to the unresolved ones would mean asking
                // the batch-progress workflow which those are, and the workflow
                // may already be gone; `resolve_by_id` settles a record only
                // while it is still `Pending`, so a target that reported its own
                // outcome is left exactly as it reported it.
                let issued = !target.claim.claim_cancel();
                if issued {
                    (
                        target.operation_id,
                        OperationOutcome::Unknown,
                        reason.issued_detail(),
                    )
                } else {
                    (
                        target.operation_id,
                        OperationOutcome::Cancelled,
                        reason.unissued_detail(),
                    )
                }
            })
            .collect();
        for (id, outcome, detail) in resolutions {
            // The return value is deliberately ignored. The log is bounded and
            // evicts from the back, so an id can outlive its record; a missing
            // or already-resolved record is a no-op, not an error.
            self.app
                .root
                .system
                .operations
                .resolve_by_id(id, outcome, Some(detail.to_string()));
        }
    }

    /// Detaches and settles the in-flight batch, then tears its batch-progress
    /// workflow down.
    ///
    /// This is the path for the batch going away with nothing taking its place:
    /// a context or scope switch. Nothing will install a replacement workflow,
    /// so leaving the old one standing would keep the progress overlay on
    /// screen over a batch that no longer exists and can never complete. The
    /// clear must stay *out* of [`Self::detach_and_settle_batch`], which is
    /// also used when a new batch supersedes this one and whose workflow is
    /// already the incoming batch's by then.
    fn cancel_batch(&mut self) {
        self.detach_and_settle_batch(BatchDetachReason::Cancelled);
        self.app.root.resources.clear_batch_workflow();
    }

    pub fn config(&self) -> Option<&Config> {
        self.config.as_ref()
    }

    /// Returns whether a reload changed the poll interval since the last call,
    /// clearing the flag. The run loop uses this to rebuild its poll timer.
    pub fn take_poll_interval_dirty(&mut self) -> bool {
        std::mem::take(&mut self.poll_interval_dirty)
    }

    /// Re-resolve the config from the stored [`Cli`] and apply the safe
    /// presentation / navigation fields live. Restart-required fields
    /// (`default_service`, starting configuration) are reported but not
    /// applied. On a load error or any validation warning the running config
    /// is left untouched and the problem is surfaced in the status line. Cloud
    /// state is never touched — only presentation / navigation fields change.
    fn reload_config(&mut self) {
        let Some(cli) = self.cli.as_ref() else {
            self.app
                .root
                .system
                .set_error_status_message("config reload unavailable in this session");
            return;
        };

        // Computed before resolving so the validator can tell an invalidated
        // service's warning what its reload actually does: keep this, or
        // fall back to the built-in columns. The later
        // `adopt_over` call below performs that same adoption; both read the
        // same `running_columns`, so the message and the outcome cannot
        // disagree.
        let running_columns = self
            .config
            .as_ref()
            .map(|running| running.columns.clone())
            .unwrap_or_default();
        let outcome = Config::resolve_over(cli, &running_columns);

        // Only a load-level failure (bad TOML / unsupported version) bails
        // and keeps the entire prior config. Field-level `problems` (a bad
        // alias, unknown theme, ...) already got a safe default substituted
        // by `resolve` and must not block adopting the rest of the reload —
        // this matches startup behavior, which applies the resolved config
        // and only surfaces such problems.
        // The problems list is replaced on every reload, this early return
        // included: leaving the previous file's problems on display would make
        // the help overlay describe a config that is no longer being read.
        self.set_base_config_problems(outcome.problems.clone().into_messages());

        if let Some(error) = &outcome.load_error {
            self.app
                .root
                .system
                .set_error_status_message(format!("config reload failed: {error}"));
            return;
        }

        let new = outcome.config;

        // Safe fields applied live. The theme is not set here: it depends on
        // the active configuration's override as well as on `new.theme`, so
        // it is recomputed by `refresh_theme` once `applied` is installed
        // below and both halves are in place.
        self.app.root.aliases = new.aliases.clone();
        self.app.root.jump_rules = new.jumps.clone();
        // Monotonic, exactly as `resolve` folds the flag over the file: once
        // read-only is on it stays on for the life of the process. A plain
        // assignment would make "edit the file, then Reload configuration" a
        // per-session toggle — and Reload is a permitted command under the
        // mode, so the guard could be switched off from inside the session it
        // is guarding.
        if new.read_only {
            self.app.root.read_only = true;
        }
        self.app
            .root
            .configurations
            .set_favorites(new.favorites.clone());
        self.app
            .root
            .navigator
            .set_sidebar_visible(new.sidebar_visible);
        // Column settings are adopted per service rather than wholesale: a
        // service whose block no longer validates keeps the settings it is
        // already running, so one broken block can neither half-apply nor cost
        // another service its valid configuration (design §5).
        let columns = new.columns.adopt_over(&running_columns);
        self.app.root.resources.set_column_settings(columns.clone());

        // Detect restart-required and poll-interval changes against the
        // currently running config.
        let mut restart_required: Vec<&str> = Vec::new();
        let mut poll_changed = true;
        if let Some(current) = self.config.as_ref() {
            if new.default_service != current.default_service {
                restart_required.push("default_service");
            }
            if new.starting_configuration != current.starting_configuration {
                restart_required.push("starting configuration");
            }
            // Plugins are restart-required by design, not by omission.
            // Swapping executable declarations under a running session opens a
            // window between the preview the operator read and the file on
            // disk, and repoints every `PluginId` a palette is holding. Saying
            // nothing would be worse than either: the problems list *is*
            // replaced from the fresh resolve, so an operator who edits the
            // file to fix a rejected entry watches the warning vanish and
            // reads that as the fix having taken effect.
            //
            // The comparison is over the *validated* sets, so an edit that
            // leaves a rejected entry still rejected changes nothing here and
            // says nothing — which is correct: the running set really is
            // unchanged, and its warning is still on the problems list.
            if new.plugins != current.plugins {
                restart_required.push("plugins");
            }
            poll_changed = new.poll_interval_secs != current.poll_interval_secs;
        }

        // Build the stored config by taking the prior config and overwriting
        // only the safe fields. Restart-required and immutable fields
        // (default_service, starting_configuration, gcloud_binary, bq_binary,
        // log_file, config_path) are left at their prior, still-running values so
        // `config()` / `nuvem info` never report a value the app has not
        // actually adopted.
        //
        // `plugins` is absent from the assignment list below on purpose, and
        // this sentence is the difference between that and an omission: the
        // clone already carries the running plugin set, which is the one
        // `RootState::plugins` is still offering and the one `nuvem info` must
        // keep reporting. Adding `applied.plugins = new.plugins` here would
        // make executable declarations live-swappable through the back door.
        let mut applied = self.config.clone().unwrap_or_default();
        applied.theme = new.theme;
        applied.aliases = new.aliases;
        applied.jumps = new.jumps;
        // Same monotonic fold as the live state above, so `config()` and
        // `nuvem info` never report a mode weaker than the one in force.
        applied.read_only = applied.read_only || new.read_only;
        applied.favorites = new.favorites;
        applied.theme_overrides = new.theme_overrides;
        applied.sidebar_visible = new.sidebar_visible;
        applied.poll_interval_secs = new.poll_interval_secs;
        applied.hotkeys = new.hotkeys;
        applied.columns = columns;

        // The pointer has to name the help key *this* reload adopted, so it is
        // taken off the value being installed rather than off the running
        // config — there is no absent-config case to write a fallback for, and
        // none to assert away either.
        let hotkeys = applied.hotkeys.clone();

        self.config = Some(applied);

        // Both depend on the config just installed: the frame override for the
        // active configuration, and the deferred check of the reloaded
        // override names against the configurations already known.
        self.refresh_theme();
        self.refresh_config_problems();

        // Composed after the deferred check, from the merged `base + deferred`
        // list rather than from `outcome.problems`: a reload whose only problem
        // is an override naming no existing configuration must say so on the
        // status line, which is the only surface the user sees without opening
        // help.
        let warnings = self.app.root.system.config_problems();
        let mut message = match warnings.len() {
            0 => "configuration reloaded".to_string(),
            1 => format!("config reloaded (warning: {})", warnings[0]),
            // More than one: the count plus a pointer at the help overlay,
            // which lists them all, via the same helper startup uses.
            count => format!(
                "config reloaded ({count} problems; {})",
                crate::services::runtime::help_pointer(&hotkeys)
            ),
        };

        if poll_changed {
            self.poll_interval_dirty = true;
        }

        if !restart_required.is_empty() {
            message.push_str(&format!(
                "; restart required for: {}",
                restart_required.join(", ")
            ));
        }
        self.app.root.system.set_status_message(message);
    }

    fn dispatch_context_picker(
        &mut self,
        kind: ConfigurationContextPickerKind,
        context: GcloudContext,
    ) {
        self.cancel_context_picker();
        self.context_picker_generation = self.context_picker_generation.wrapping_add(1);
        let generation = self.context_picker_generation;
        let cancellation = self.context_picker_cancellation.clone();
        let runner = self.gcloud_runner.clone();
        let sdk_for_picker = self.sdk_clients.clone();
        let sender = self.effect_result_sender.clone();
        let configuration = context.configuration.clone();
        let failed_configuration = configuration.clone();
        let cancellation_result = EffectResult::ContextPickerLoadFailed {
            configuration: failed_configuration.clone(),
            kind,
            error: AppError::gcloud("gcloud discovery was cancelled"),
        };
        let Ok(permit) = self.context_picker_slots.clone().try_acquire_owned() else {
            let _ = sender.send(QueuedEffectResult::ContextPicker {
                generation,
                result: EffectResult::ContextPickerLoadFailed {
                    configuration: failed_configuration,
                    kind,
                    error: AppError::gcloud("previous gcloud discovery is still stopping"),
                },
            });
            return;
        };
        tokio::spawn(async move {
            let result = tokio::task::spawn_blocking(move || {
                let _permit = permit;
                match runner {
                    Some(runner) => load_context_picker(
                        runner.as_ref(),
                        sdk_for_picker.as_deref(),
                        kind,
                        context,
                        &cancellation,
                    ),
                    None => Some(EffectResult::ContextPickerLoadFailed {
                        configuration,
                        kind,
                        error: AppError::gcloud("no gcloud command runner configured"),
                    }),
                }
            })
            .await
            .map(|result| result.unwrap_or(cancellation_result.clone()))
            .unwrap_or(cancellation_result);
            let _ = sender.send(QueuedEffectResult::ContextPicker { generation, result });
        });
    }

    fn dispatch_background_refresh(
        &mut self,
        service: ResourceServiceKind,
        context: GcloudContext,
        filter: Option<String>,
    ) {
        self.pending_background_refreshes = self.pending_background_refreshes.saturating_add(1);
        // Minted here, where the fetch is spawned, so the load phase and the
        // task that owns it cannot drift apart: every list refresh gets exactly
        // one ticket, and it rides the result back to the drop site.
        let load_ticket = self
            .app
            .root
            .resources
            .begin_load(service, std::time::Instant::now());
        let runner = self.gcloud_runner.clone();
        let sdk_for_task = self.sdk_clients.clone();
        let sender = self.effect_result_sender.clone();
        let task_context = context.clone();
        let failed_context = context;
        let join_error_context = failed_context.clone();
        let context_generation = self.context_generation;
        let operation_generation = self.operation_generation;
        let scope_generation = self.scope_generation;
        let status_generation = self.app.root.system.status_generation;

        tokio::spawn(async move {
            let result = tokio::task::spawn_blocking(move || match runner {
                Some(runner) => match service {
                    ResourceServiceKind::ComputeInstances => load_compute_instances(
                        runner.as_ref(),
                        sdk_for_task.as_deref(),
                        task_context,
                        filter,
                    ),
                    ResourceServiceKind::GkeClusters => load_gke_clusters(
                        runner.as_ref(),
                        sdk_for_task.as_deref(),
                        task_context,
                        filter,
                    ),
                    ResourceServiceKind::CloudRunServices => load_cloud_run_services(
                        runner.as_ref(),
                        sdk_for_task.as_deref(),
                        task_context,
                    ),
                    ResourceServiceKind::CloudSql => load_cloud_sql_instances(
                        runner.as_ref(),
                        sdk_for_task.as_deref(),
                        task_context,
                        filter,
                    ),
                    ResourceServiceKind::CloudStorage => {
                        load_storage_buckets(runner.as_ref(), sdk_for_task.as_deref(), task_context)
                    }
                    ResourceServiceKind::SecretManagerSecrets => load_secrets(
                        runner.as_ref(),
                        sdk_for_task.as_deref(),
                        task_context,
                        filter,
                    ),
                    ResourceServiceKind::PersistentDisks => load_persistent_disks(
                        runner.as_ref(),
                        sdk_for_task.as_deref(),
                        task_context,
                        filter,
                    ),
                    ResourceServiceKind::CloudFunctions => load_cloud_functions(
                        runner.as_ref(),
                        sdk_for_task.as_deref(),
                        task_context,
                        filter,
                    ),
                    ResourceServiceKind::PubSubTopics => load_pubsub_topics(
                        runner.as_ref(),
                        sdk_for_task.as_deref(),
                        task_context,
                        filter,
                    ),
                    // No filter: three of the four networks columns are
                    // derived and appear verbatim in no raw field, so `filter`
                    // has nothing to be pushed into.
                    ResourceServiceKind::VpcNetworks => {
                        load_vpc_networks(runner.as_ref(), sdk_for_task.as_deref(), task_context)
                    }
                    // No filter, for the reason the networks arm above
                    // records: none of this table's columns is a verbatim
                    // substring of a raw field.
                    ResourceServiceKind::VpcSubnets => {
                        load_vpc_subnets(runner.as_ref(), sdk_for_task.as_deref(), task_context)
                    }
                    // No filter, for the reason the two VPC arms above
                    // record: ACTION, TARGETS and the DISABLED badge are all
                    // words this application adds.
                    ResourceServiceKind::FirewallRules => {
                        load_firewall_rules(runner.as_ref(), sdk_for_task.as_deref(), task_context)
                    }
                    ResourceServiceKind::PubSubSubscriptions => load_pubsub_subscriptions(
                        runner.as_ref(),
                        sdk_for_task.as_deref(),
                        task_context,
                        filter,
                    ),
                    ResourceServiceKind::IamServiceAccounts => load_service_accounts(
                        runner.as_ref(),
                        sdk_for_task.as_deref(),
                        task_context,
                        filter,
                    ),
                    // The policy read takes no filter: it fetches one object
                    // and flattens it, so there is nothing to push down.
                    ResourceServiceKind::IamPolicy => {
                        load_policy_members(runner.as_ref(), sdk_for_task.as_deref(), task_context)
                    }
                    // No filter: `bq ls` accepts no server-side filter
                    // expression, so `filter` has nothing to be pushed into.
                    ResourceServiceKind::BigQueryDatasets => {
                        load_bigquery_datasets(runner.as_ref(), task_context)
                    }
                },
                None => background_refresh_runner_missing(service, failed_context.clone()),
            })
            .await
            .unwrap_or_else(|error| {
                background_refresh_join_failed(service, join_error_context, error)
            });

            let _ = sender.send(QueuedEffectResult::Background {
                context_generation,
                operation_generation,
                scope_generation,
                status_generation,
                load_ticket: Some(load_ticket),
                result,
            });
        });
    }

    /// Runs one table's `bq show` off the run loop, behind the shared permit
    /// pool.
    ///
    /// The permit is taken inside the spawned task and awaited rather than
    /// tried: every one of these calls is wanted, so the excess queues instead
    /// of being refused. A fan-out over a wide dataset therefore proceeds a
    /// few calls at a time, rows filling in as answers land, with the UI
    /// responsive throughout.
    ///
    /// A call still queued for a listing a newer listing has replaced does not
    /// run: [`Self::supersede_bigquery_enrichments`] flips the token these
    /// tasks check on both sides of the permit. One already inside
    /// `spawn_blocking` does run to completion — a spawned `bq` cannot be
    /// interrupted from here — and its answer is dropped at the reducer, where
    /// the scope lives.
    ///
    /// `pending_background_refreshes` is deliberately not incremented: that
    /// counter is decremented only on the `Background` queue's arm, and it
    /// gates the auto-load of the active service. A fan-out holding it up for
    /// minutes would defer the very list refresh the operator is waiting on.
    fn dispatch_bigquery_table_stats(&mut self, effect: Effect) {
        let Effect::LoadBigQueryTableStats {
            context,
            dataset_id,
            table_id,
            generation,
            listing,
        } = effect
        else {
            unreachable!("dispatch_bigquery_table_stats called with unsupported effect: {effect:?}")
        };
        let runner = self.gcloud_runner.clone();
        let sender = self.effect_result_sender.clone();
        let slots = self.bigquery_enrichment_slots.clone();
        let cancellation = self.bigquery_enrichment_cancellation.clone();
        let join_error_context = context.clone();
        let join_error_table = table_id.clone();

        tokio::spawn(async move {
            // No permit means no call: the listing this was fanned out from
            // has been replaced, or the semaphore is closed on the way out.
            // Nothing is sent either way — the reducer would drop the answer
            // anyway, and a result for a call that never went out has nothing
            // to report.
            let Some(permit) = claim_bigquery_enrichment_slot(&cancellation, slots).await else {
                return;
            };
            let result = tokio::task::spawn_blocking(move || {
                let _permit = permit;
                match runner {
                    Some(runner) => load_bigquery_table_stats(
                        runner.as_ref(),
                        context,
                        &dataset_id,
                        table_id,
                        generation,
                        listing,
                    ),
                    None => EffectResult::BigQueryTableStatsLoadFailed {
                        context,
                        table_id,
                        generation,
                        listing,
                        error: AppError::gcloud("no gcloud command runner configured"),
                    },
                }
            })
            .await
            .unwrap_or_else(|error| EffectResult::BigQueryTableStatsLoadFailed {
                context: join_error_context,
                table_id: join_error_table,
                generation,
                listing,
                error: AppError::gcloud(format!("background table stats task failed: {error}")),
            });

            let _ = sender.send(QueuedEffectResult::BigQueryEnrichment { result });
        });
    }

    /// Cancels the fan-out of the listing being replaced and arms a fresh
    /// token for the one being dispatched.
    ///
    /// A fresh token rather than a reused one for the same reason
    /// `cancel_running_plugins` mints one: the shows about to be fanned out
    /// from the new listing must not be born cancelled. They are spawned from
    /// this session on a later pass of the effect loop — the fan-out is
    /// emitted by the listing's *result* — so they read whatever token is on
    /// the session then.
    fn supersede_bigquery_enrichments(&mut self) {
        self.bigquery_enrichment_cancellation.cancel();
        self.bigquery_enrichment_cancellation = CancellationToken::new();
    }

    fn dispatch_background_logs(&mut self, effect: Effect) {
        self.pending_background_refreshes = self.pending_background_refreshes.saturating_add(1);
        let runner = self.gcloud_runner.clone();
        let sdk_for_logs = self.sdk_clients.clone();
        let sender = self.effect_result_sender.clone();
        let context_generation = self.context_generation;
        let operation_generation = self.operation_generation;
        let scope_generation = self.scope_generation;
        let status_generation = self.app.root.system.status_generation;
        let is_tail = matches!(effect, Effect::TailLogs { .. });
        let (join_error_context, join_error_query) = match &effect {
            Effect::LoadRecentLogs { context, query, .. }
            | Effect::TailLogs { context, query, .. } => (context.clone(), query.clone()),
            other => {
                unreachable!("dispatch_background_logs called with unsupported effect: {other:?}")
            }
        };

        tokio::spawn(async move {
            let result = tokio::task::spawn_blocking(move || match (runner, effect) {
                (
                    Some(runner),
                    Effect::LoadRecentLogs {
                        context,
                        query,
                        severity,
                        range,
                    },
                ) => load_recent_logs(
                    runner.as_ref(),
                    sdk_for_logs.as_deref(),
                    context,
                    query,
                    severity,
                    range,
                ),
                (
                    Some(runner),
                    Effect::TailLogs {
                        context,
                        query,
                        severity,
                        cursor,
                    },
                ) => tail_logs(
                    runner.as_ref(),
                    sdk_for_logs.as_deref(),
                    context,
                    query,
                    severity,
                    cursor,
                ),
                (None, Effect::LoadRecentLogs { context, query, .. }) => {
                    EffectResult::RecentLogsLoadFailed {
                        context,
                        query,
                        error: AppError::gcloud("no gcloud command runner configured"),
                    }
                }
                (None, Effect::TailLogs { context, query, .. }) => EffectResult::LogsTailFailed {
                    context,
                    query,
                    error: AppError::gcloud("no gcloud command runner configured"),
                },
                (_, other) => unreachable!(
                    "dispatch_background_logs called with unsupported effect: {other:?}"
                ),
            })
            .await
            .unwrap_or_else(|error| {
                let message = format!("background log task failed: {error}");
                if is_tail {
                    EffectResult::LogsTailFailed {
                        context: join_error_context,
                        query: join_error_query,
                        error: AppError::gcloud(message),
                    }
                } else {
                    EffectResult::RecentLogsLoadFailed {
                        context: join_error_context,
                        query: join_error_query,
                        error: AppError::gcloud(message),
                    }
                }
            });

            let _ = sender.send(QueuedEffectResult::Background {
                context_generation,
                operation_generation,
                scope_generation,
                status_generation,
                // Cloud Logging has no resource load phase to resolve.
                load_ticket: None,
                result,
            });
        });
    }

    /// Refreshes the active service, carrying its applied filter.
    ///
    /// This path refreshes the active service *by construction*, so there is no
    /// cross-service term to leak and the filter is always the right one to
    /// push. Passing `None` here would be a silent disarm: the reload replaces
    /// the visible rows, and label mode narrows on the server only
    /// (`CompiledFilter::Label` keeps every row), so an unfiltered poll would
    /// swap a `-l k=v` table for the whole project while the tools line still
    /// shows the filter.
    async fn schedule_active_refresh(&mut self) {
        let Some(context) = self.app.root.current_context() else {
            return;
        };
        let first_load = !self.app.root.resources.has_loaded_context();
        let filter = crate::app::reducer::resources::active_filter(&self.app.root);
        let effect = match self.app.root.resources.active_service() {
            ResourceServiceKind::ComputeInstances => {
                Effect::RefreshComputeInstances { context, filter }
            }
            ResourceServiceKind::GkeClusters => Effect::RefreshGkeClusters { context, filter },
            ResourceServiceKind::CloudRunServices => Effect::RefreshCloudRunServices { context },
            ResourceServiceKind::CloudSql => Effect::RefreshCloudSqlInstances { context, filter },
            ResourceServiceKind::CloudStorage => Effect::RefreshStorageBuckets { context },
            ResourceServiceKind::SecretManagerSecrets => Effect::RefreshSecrets { context, filter },
            ResourceServiceKind::CloudFunctions => {
                Effect::RefreshCloudFunctions { context, filter }
            }
            ResourceServiceKind::PersistentDisks => {
                Effect::RefreshPersistentDisks { context, filter }
            }
            ResourceServiceKind::PubSubTopics => Effect::RefreshPubSubTopics { context, filter },
            ResourceServiceKind::VpcNetworks => Effect::RefreshVpcNetworks { context },
            ResourceServiceKind::VpcSubnets => Effect::RefreshVpcSubnets { context },
            ResourceServiceKind::FirewallRules => Effect::RefreshFirewallRules { context },
            ResourceServiceKind::PubSubSubscriptions => {
                Effect::RefreshPubSubSubscriptions { context, filter }
            }
            ResourceServiceKind::IamServiceAccounts => {
                Effect::RefreshServiceAccounts { context, filter }
            }
            ResourceServiceKind::IamPolicy => Effect::RefreshIamPolicy { context },
            ResourceServiceKind::BigQueryDatasets => Effect::RefreshBigQueryDatasets { context },
        };
        self.dispatch_effects(vec![effect], false).await;
        if first_load {
            self.app.root.system.operation.phase = OperationPhase::Loading;
        }
    }

    /// Schedules a background refresh of the active service when polling applies.
    ///
    /// Skips scheduling when the active (service, context) scope's last
    /// failure was `Retryability::Persistent` (see `gated_scope`), so the
    /// poll ticker does not keep hammering gcloud on auth/permission/config
    /// errors. Manual refresh (`ResourcesAction::Refresh`) always proceeds
    /// and clears the gate; this method is only reached by the timer tick.
    pub async fn poll_active_resource(&mut self) {
        if self.app.root.workspace != Workspace::Resources {
            return;
        }
        if self.pending_background_refreshes != 0 {
            return;
        }
        let Some(context) = self.app.root.current_context() else {
            return;
        };
        let scope = (self.app.root.resources.active_service(), context);
        let is_gated = self.gated_scope.as_ref() == Some(&scope);
        let last_retryability =
            is_gated.then_some(nuvem_domain::gcp::diagnostics::Retryability::Persistent);
        if !should_auto_retry(last_retryability) {
            return;
        }
        self.schedule_active_refresh().await;
    }

    /// Schedules one bounded Cloud Logging tail request per poll tick, when
    /// the Logging workspace is active and the tail is running. Computes the
    /// decision from the pure `should_tail` on the CURRENT (pre-decrement)
    /// backoff value, then decrements the quota backoff counter exactly once
    /// per call (draining the skip counter every tick, even on `Skip`). This
    /// ordering matters: a backoff of N must produce N fully-skipped ticks
    /// before tailing resumes on tick N+1, so the decision has to observe
    /// the backoff before it is drained for this tick.
    pub async fn poll_active_tail(&mut self) {
        let decision = crate::services::runtime::tail::should_tail(
            &self.app.root,
            self.pending_background_refreshes,
        );
        self.app.root.logging.decrement_backoff();
        let Some(context) = self.app.root.current_context() else {
            return;
        };
        let effect = match decision {
            crate::services::runtime::tail::TailDecision::Skip => return,
            crate::services::runtime::tail::TailDecision::LoadFirstPage => {
                // The only path that starts a first page without a reducer
                // having marked the buffer `Loading` first. Mark it here, or the
                // viewer keeps rendering "Not loaded" over a fetch that is
                // running -- e.g. a context that resolved after the workspace
                // was entered without one.
                self.app.root.logging.begin_first_load();
                Effect::LoadRecentLogs {
                    context,
                    query: self.app.root.logging.query.clone(),
                    severity: self.app.root.logging.severity_floor,
                    range: self.app.root.logging.time_range,
                }
            }
            crate::services::runtime::tail::TailDecision::Tail => Effect::TailLogs {
                context,
                query: self.app.root.logging.query.clone(),
                severity: self.app.root.logging.severity_floor,
                cursor: self
                    .app
                    .root
                    .logging
                    .cursor
                    .clone()
                    .expect("Tail decision implies cursor present"),
            },
        };
        self.dispatch_effects(vec![effect], false).await;
    }

    async fn ensure_active_resource_loaded(&mut self) {
        if self.app.root.workspace != Workspace::Resources {
            return;
        }
        let Some(context) = self.app.root.current_context() else {
            return;
        };
        // Defer while a refresh is in flight without recording the scope: the in-flight
        // completion re-runs this and re-evaluates the (possibly switched) active scope.
        if self.pending_background_refreshes != 0 {
            return;
        }
        let scope = (self.app.root.resources.active_service(), context.clone());
        if self.auto_loaded_scope.as_ref() == Some(&scope) {
            return;
        }
        self.auto_loaded_scope = Some(scope);
        if self.app.root.resources.needs_refresh(&context) {
            self.schedule_active_refresh().await;
        }
    }

    fn dispatch_diagnostics_probe(&self, configuration: Option<GcloudConfigurationName>) {
        let runner = self.gcloud_runner.clone();
        let sender = self.effect_result_sender.clone();
        let context_generation = self.context_generation;
        let Ok(permit) = self.diagnostics_slots.clone().try_acquire_owned() else {
            return;
        };
        tokio::spawn(async move {
            let report = tokio::task::spawn_blocking(move || {
                let _permit = permit;
                match runner {
                    Some(runner) => {
                        crate::services::gcloud::diagnostics::GcloudDiagnosticsService::new(
                            runner.as_ref(),
                        )
                        .probe(configuration.as_ref())
                    }
                    None => nuvem_domain::gcp::diagnostics::GcloudDiagnosticsReport {
                        available: false,
                        active_account: None,
                        problem: Some(nuvem_domain::gcp::diagnostics::DiagnosticProblem::new(
                            nuvem_domain::gcp::diagnostics::GcloudErrorKind::Unknown,
                            "no gcloud command runner configured",
                        )),
                    },
                }
            })
            .await
            .unwrap_or_else(|error| {
                nuvem_domain::gcp::diagnostics::GcloudDiagnosticsReport {
                    available: false,
                    active_account: None,
                    problem: Some(nuvem_domain::gcp::diagnostics::DiagnosticProblem::new(
                        nuvem_domain::gcp::diagnostics::GcloudErrorKind::Unknown,
                        format!("diagnostics task failed: {error}"),
                    )),
                }
            });
            let _ = sender.send(QueuedEffectResult::Diagnostics {
                context_generation,
                result: EffectResult::GcloudDiagnosticsProbed { report },
            });
        });
    }

    fn cancel_context_picker(&mut self) {
        self.context_picker_generation = self.context_picker_generation.wrapping_add(1);
        self.context_picker_cancellation.cancel();
        self.context_picker_cancellation = CancellationToken::new();
    }

    fn load_context_picker(
        &self,
        kind: ConfigurationContextPickerKind,
        context: GcloudContext,
    ) -> EffectResult {
        let Some(runner) = &self.gcloud_runner else {
            return EffectResult::ContextPickerLoadFailed {
                configuration: context.configuration,
                kind,
                error: AppError::gcloud("no gcloud command runner configured"),
            };
        };
        let configuration = context.configuration.clone();
        load_context_picker(
            runner.as_ref(),
            self.sdk_clients.as_deref(),
            kind,
            context,
            &CancellationToken::new(),
        )
        .unwrap_or_else(|| EffectResult::ContextPickerLoadFailed {
            configuration,
            kind,
            error: AppError::gcloud("gcloud discovery was cancelled"),
        })
    }

    /// Hands one confirmed single-row lifecycle action to a blocking thread and
    /// returns.
    ///
    /// Off the reactor for the reason the plugin dispatch below gives, which
    /// applies to these two effects with more force since they were given a
    /// ceiling measured in tens of minutes: their argv carries no `--async`, so
    /// gcloud blocks until GCP finishes the operation, and running that inline
    /// froze the whole TUI — no draw, no key, no poll tick, no termination arm
    /// — until it did.
    ///
    /// Both effects go through one function so the Compute and Cloud SQL paths
    /// cannot drift into different threading, which is exactly how the Cloud
    /// SQL path came to classify its own errors differently from Compute's.
    /// The two arms differ only in which result-building pair they hand over.
    fn dispatch_instance_action(&mut self, effect: Effect) {
        type ActionRun = Box<dyn FnOnce() -> EffectResult + Send + 'static>;
        type ActionJoinError = Box<dyn FnOnce(AppError) -> EffectResult + Send + 'static>;

        let runner = self.gcloud_runner.clone();
        // The same rule every read follows: native when this session holds
        // clients, gcloud otherwise. A demo session holds none, which is what
        // keeps its mutations from reaching a real API -- demo intercepts by
        // matching the argv, and a request would go straight past that.
        let sdk = self.sdk_clients.clone();
        let sender = self.effect_result_sender.clone();
        // Read here, where the action is spawned, so the guard the result is
        // held to is the generation of the batch that opened this action's
        // operation and not of whatever the operator has started since.
        let operation_generation = self.operation_generation;
        // A single row's `stop` is the same shape as a batch of them — one
        // blocking thread, one thirty-minute ceiling, one `Runtime::drop`
        // waiting on it — so it holds the same shutdown token. Only
        // one of the two arms below moves it, so it is cloned once here rather
        // than per arm.
        let shutdown = self.lifecycle_cancellation.clone();

        let (run, join_error): (ActionRun, ActionJoinError) = match effect {
            Effect::ComputeInstanceAction {
                context,
                instance,
                action,
                operation_id,
            } => {
                // Kept for the join-error arm, which cannot reach the values the
                // blocking closure took ownership of.
                let abandoned_context = context.clone();
                let abandoned_name = instance.name.clone();
                let abandoned_zone = instance.zone.clone();
                (
                    Box::new(move || {
                        run_instance_action_result(
                            Backends { runner, sdk },
                            context,
                            instance.name,
                            instance.zone,
                            action,
                            operation_id,
                            shutdown,
                        )
                    }),
                    Box::new(move |error| EffectResult::ComputeInstanceActionFailed {
                        context: abandoned_context,
                        instance_name: abandoned_name,
                        instance_zone: abandoned_zone,
                        action,
                        error,
                        operation_id,
                    }),
                )
            }
            Effect::CloudSqlInstanceAction {
                context,
                instance,
                action,
                operation_id,
            } => {
                let abandoned_context = context.clone();
                let abandoned_name = instance.name.clone();
                (
                    Box::new(move || {
                        run_cloud_sql_action_result(
                            Backends { runner, sdk },
                            context,
                            instance.name,
                            action,
                            operation_id,
                            shutdown,
                        )
                    }),
                    Box::new(move |error| EffectResult::CloudSqlInstanceActionFailed {
                        context: abandoned_context,
                        instance_name: abandoned_name,
                        action,
                        error,
                        operation_id,
                    }),
                )
            }
            other => {
                unreachable!("dispatch_instance_action called with unsupported effect: {other:?}")
            }
        };

        tokio::spawn(async move {
            let result = tokio::task::spawn_blocking(run)
                .await
                // A panicking action task must still resolve its
                // operations-log entry, the same way a batch target's does:
                // with no result sent, that entry stays `Pending` for the life
                // of the process and the row it names never refreshes.
                .unwrap_or_else(|error| {
                    join_error(AppError::gcloud(format!(
                        "the instance action task failed: {error}"
                    )))
                });
            let _ = sender.send(QueuedEffectResult::InstanceAction {
                operation_generation,
                result,
            });
        });
    }

    /// Hands a confirmed plugin's argv to a blocking thread and returns.
    ///
    /// Off the reactor, like every other child this file starts: a plugin is an
    /// arbitrary user-declared gcloud invocation with no cancellation and no
    /// timeout, so running it inline would let one command that never returns
    /// freeze the TUI for good.
    fn dispatch_run_plugin(
        &mut self,
        context: GcloudContext,
        label: String,
        argv: Vec<String>,
        operation_id: u64,
    ) {
        let runner = self.gcloud_runner.clone();
        let sender = self.effect_result_sender.clone();
        // Read here, where the run is spawned, so it is the generation of the
        // batch that opened the "plugin" operation and not of whatever the
        // operator has started by the time the child exits.
        let operation_generation = self.operation_generation;
        // Kept for the join-error arm below, which cannot reach the values the
        // blocking closure took ownership of.
        let abandoned_context = context.clone();
        let abandoned_label = label.clone();
        // Registered before the spawn, not inside it: a cancellation landing
        // between the confirmation and the worker's first instruction must
        // still find this run, and a registry the worker populates could not
        // promise that.
        let claim = Arc::new(BatchTargetClaim::new());
        self.plugin_runs.push(PluginRunHandle {
            operation_id,
            claim: Arc::clone(&claim),
        });
        let cancellation = self.plugin_cancellation.clone();
        tokio::spawn(async move {
            let result = tokio::task::spawn_blocking(move || {
                run_plugin_result(
                    runner,
                    context,
                    label,
                    argv,
                    operation_id,
                    cancellation,
                    claim,
                )
            })
            .await
            .unwrap_or_else(|error| EffectResult::PluginRunFailed {
                context: abandoned_context,
                label: abandoned_label,
                error: AppError::gcloud(format!("the plugin task failed: {error}")),
                // No child ran, so there is no stderr to keep.
                stderr: String::new(),
                operation_id,
            });
            let _ = sender.send(QueuedEffectResult::Plugin {
                operation_generation,
                result,
            });
        });
    }
}

/// Runs a single compute instance action and maps the outcome onto the shared
/// `ComputeInstanceActionSucceeded`/`Failed` result variants. Used by both the
/// single-row action path and each background batch target, so the two share
/// identical result handling downstream.
///
/// `operation_id` is the operations-log entry this run settles, carried
/// through unchanged so every outcome — including the "no runner configured"
/// early return — resolves it.
/// What a native write says when the session stops waiting for it.
///
/// Deliberately not `CommandCeiling::Lifecycle`'s caveat, which the CLI path
/// uses. There, the process was killed and it is unknown whether the command
/// reached GCP at all, so the caveat hedges on both. Here the request was
/// sent, the operation it started exists, and only the watching stopped --
/// which is a narrower thing to admit and a more useful one to read.
const ABANDONED_WRITE: &str =
    "the wait was abandoned; the operation it started is still running in GCP";

/// The two ways this session can reach Google Cloud.
///
/// They travel together because the rule that picks between them is one rule:
/// native when the session holds clients, `gcloud` otherwise. Passing them as
/// two arguments invited a call site that had one and not the other, which is
/// exactly the state a demo session is in and exactly the state that must not
/// reach a real API.
#[derive(Clone, Default)]
struct Backends {
    runner: Option<Arc<dyn CommandRunner + Send + Sync>>,
    sdk: Option<Arc<crate::services::sdk::SdkClients>>,
}

fn run_instance_action_result(
    backends: Backends,
    context: GcloudContext,
    instance_name: String,
    instance_zone: String,
    action: ComputeInstanceAction,
    operation_id: u64,
    cancellation: CancellationToken,
) -> EffectResult {
    let Backends { runner, sdk } = backends;
    let Some(runner) = runner else {
        return EffectResult::ComputeInstanceActionFailed {
            context,
            instance_name,
            instance_zone,
            action,
            error: AppError::gcloud("no gcloud command runner configured"),
            operation_id,
        };
    };
    if let Some(clients) = sdk.as_deref() {
        let project = context.project.clone().unwrap_or_default();
        let accepted = clients
            .compute(context.configuration.as_str())
            .and_then(|client| {
                crate::services::sdk::block_on_cancellable(
                    client.run_action(&project, &instance_zone, &instance_name, action),
                    &cancellation,
                )
                // Abandoned, not undone. What is known here is narrower than
                // on the CLI path and the message says so rather than
                // borrowing its words: there, a killed process leaves it
                // unknown whether the command was even issued, while here the
                // request was sent and the operation it started is the thing
                // still running.
                .unwrap_or_else(|| Err(crate::services::sdk::ApiError::new(ABANDONED_WRITE)))
            });
        return match accepted {
            Ok(()) => EffectResult::ComputeInstanceActionSucceeded {
                context,
                instance_name,
                instance_zone,
                action,
                operation_id,
            },
            Err(error) => EffectResult::ComputeInstanceActionFailed {
                context,
                instance_name,
                instance_zone,
                action,
                error: AppError::gcloud(error.to_string()),
                operation_id,
            },
        };
    }

    let service = GcloudComputeService::new(runner.as_ref());
    match service.run_action_by_id_with_cancellation(
        &context,
        &instance_name,
        &instance_zone,
        action,
        &cancellation,
    ) {
        Ok(()) => EffectResult::ComputeInstanceActionSucceeded {
            context,
            instance_name,
            instance_zone,
            action,
            operation_id,
        },
        Err(error) => {
            let error: AppError = error.into();
            EffectResult::ComputeInstanceActionFailed {
                context,
                instance_name,
                instance_zone,
                action,
                error: error.with_command_summary(format!(
                    "gcloud compute instances {}",
                    action.command_verb()
                )),
                operation_id,
            }
        }
    }
}

/// Runs one confirmed plugin and maps the outcome onto the `PluginRun*` result
/// variants. Blocking, and called only from a blocking thread.
///
/// `GcloudCommand::new(argv)` and nothing else: there is no program field and
/// no shell, so the elements the operator confirmed are the elements the child
/// receives. `context` is not read on the way in — a plugin's argv already
/// carries the `--configuration` that pins it, injected by `plugin_argv`, and
/// consulting the context here could only append it a second time — but it is
/// carried out on every arm, including the no-runner one, because that is what
/// makes a result attributable to the context it was confirmed under.
fn run_plugin_result(
    runner: Option<Arc<dyn CommandRunner + Send + Sync>>,
    context: GcloudContext,
    label: String,
    argv: Vec<String>,
    operation_id: u64,
    cancellation: CancellationToken,
    claim: Arc<BatchTargetClaim>,
) -> EffectResult {
    let Some(runner) = runner else {
        return EffectResult::PluginRunFailed {
            context,
            label,
            error: AppError::gcloud("no gcloud command runner configured"),
            // Nothing ran, so there is no stderr to keep.
            stderr: String::new(),
            operation_id,
        };
    };
    // Losing means a cancellation got here first and its entry already reads
    // `Cancelled` — "the command was never issued". Issuing one now is the one
    // thing that could make that entry false.
    if !claim.claim_start() {
        return EffectResult::PluginRunCancelled {
            context,
            label,
            operation_id,
        };
    }
    match runner.run_with_cancellation(GcloudCommand::new(argv), &cancellation) {
        Ok(output) => EffectResult::PluginRunSucceeded {
            context,
            label,
            output: output.stdout,
            stderr: output.stderr,
            operation_id,
        },
        // A plugin's argv carries the read ceiling, and this arm reports the
        // operator's own cancel key rather than the error's text, so the
        // ceiling has no reader here.
        Err(CommandError::Cancelled { .. }) => EffectResult::PluginRunCancelled {
            context,
            label,
            operation_id,
        },
        Err(error) => {
            // Taken before the conversion: `AppError` sanitizes and bounds
            // what it carries, which is what a status line needs and what a
            // viewer must not be limited to.
            let stderr = match &error {
                // Literally stderr, and deliberately not
                // `failure_diagnostic(stdout, stderr)` — the one reader of a
                // `Failed` that keeps the distinction the others were
                // taught to resolve. What the viewer wants here is the
                // unbounded error trailer, named as the stream it came from;
                // the failure's *reason* is on the `AppError` built two lines
                // below, and that one does resolve, so a plugin that diagnoses
                // on stdout still gets a status line that says why.
                //
                // A plugin's argv is operator-declared, so any of them may
                // behave like `bq`. Carrying its stdout through this arm as
                // well is a change to `PluginRunFailed`'s shape and to what
                // the viewer draws, which is not this ticket's.
                CommandError::Failed { stderr, .. } => stderr.clone(),
                // Nothing ran, or it was killed: no stderr was produced.
                //
                // `TimedOut` sits here rather than with `Failed` even though
                // the child may well have written some: whatever it wrote is
                // a partial transcript of a command that never reached a
                // verdict, and putting it where a reader expects the reason
                // for a failure would present it as one.
                CommandError::MissingExecutable { .. }
                | CommandError::Io(_)
                | CommandError::Cancelled { .. }
                | CommandError::TimedOut { .. } => String::new(),
            };
            let error: AppError = error.into();
            EffectResult::PluginRunFailed {
                context,
                // The plugin's own label, not its argv: it is the name the
                // operator gave this command, and the argv can carry values
                // they would not want repeated into a status line.
                error: error.with_command_summary(label.clone()),
                label,
                stderr,
                operation_id,
            }
        }
    }
}

/// Runs a single Cloud SQL instance action and maps the outcome onto the shared
/// `CloudSqlInstanceActionSucceeded`/`Failed` result variants. Used by both the
/// single-row action path and each background batch target, so the two share
/// identical result handling downstream.
///
/// The instance is identified by name alone: that is all the Cloud SQL argv
/// takes, and all its result variants carry back.
fn run_cloud_sql_action_result(
    backends: Backends,
    context: GcloudContext,
    instance_name: String,
    action: CloudSqlInstanceAction,
    operation_id: u64,
    cancellation: CancellationToken,
) -> EffectResult {
    let Backends { runner, sdk } = backends;
    let Some(runner) = runner else {
        return EffectResult::CloudSqlInstanceActionFailed {
            context,
            instance_name,
            action,
            error: AppError::gcloud("no gcloud command runner configured".to_string()),
            operation_id,
        };
    };
    if let Some(clients) = sdk.as_deref() {
        let project = context.project.clone().unwrap_or_default();
        let accepted = clients
            .cloudsql(context.configuration.as_str())
            .and_then(|client| {
                crate::services::sdk::block_on_cancellable(
                    client.run_action(&project, &instance_name, action),
                    &cancellation,
                )
                // Abandoned, not undone -- see the compute twin.
                .unwrap_or_else(|| Err(crate::services::sdk::ApiError::new(ABANDONED_WRITE)))
            });
        return match accepted {
            Ok(()) => EffectResult::CloudSqlInstanceActionSucceeded {
                context,
                instance_name,
                action,
                operation_id,
            },
            Err(error) => EffectResult::CloudSqlInstanceActionFailed {
                context,
                instance_name,
                action,
                error: AppError::gcloud(error.to_string()),
                operation_id,
            },
        };
    }

    let service = GcloudCloudSqlService::new(runner.as_ref());
    match service.run_action_by_id_with_cancellation(
        &context,
        &instance_name,
        action,
        &cancellation,
    ) {
        Ok(()) => EffectResult::CloudSqlInstanceActionSucceeded {
            context,
            instance_name,
            action,
            operation_id,
        },
        // `into()` rather than `AppError::gcloud(error.to_string())`, which is
        // what this arm used to do. Flattening the error into a string dropped
        // its classification, and `AppError::kind` then re-derived one by
        // reading the text: a `TimedOut`, whose detail necessarily says "timed
        // out", came back classified `Network`, so a failover nuvem had killed
        // at its own ceiling was reported to the operator as a connectivity
        // problem. The Compute arm above has always converted.
        Err(error) => EffectResult::CloudSqlInstanceActionFailed {
            context,
            instance_name,
            action,
            error: AppError::from(error),
            operation_id,
        },
    }
}

/// The service-specific half of a batch: how a single target's action runs, and
/// how a target whose blocking task panicked (so it produced no result of its
/// own) is reported as failed. Selected once per batch by matching
/// [`BatchAction`], so the bounded semaphore, the cancellation checks, the
/// `spawn_blocking` hop and the join-error fallback in
/// [`RuntimeSession::dispatch_resource_batch`] are written once and shared by
/// every batchable service.
#[derive(Clone)]
struct BatchTargetRunner {
    run: BatchTargetRun,
    join_error: BatchTargetJoinError,
}

/// Runs one batch target's action on a `spawn_blocking` thread and maps the
/// outcome onto its service's action-result variants. The `u64` is the target's
/// own operations-log entry id, which the result carries back so it settles
/// that entry and no other.
type BatchTargetRun = Arc<
    dyn Fn(Backends, GcloudContext, BatchRowRef, u64, CancellationToken) -> EffectResult
        + Send
        + Sync,
>;

/// Reports a target whose blocking task panicked as a failed action, so it is
/// never stranded `Pending`.
type BatchTargetJoinError =
    Arc<dyn Fn(GcloudContext, BatchRowRef, AppError, u64) -> EffectResult + Send + Sync>;

impl BatchTargetRunner {
    fn for_action(action: BatchAction) -> Self {
        match action {
            BatchAction::Compute(action) => Self {
                run: Arc::new(
                    move |backends, context, target, operation_id, cancellation| {
                        run_instance_action_result(
                            backends,
                            context,
                            target.name,
                            target.location,
                            action,
                            operation_id,
                            cancellation,
                        )
                    },
                ),
                join_error: Arc::new(move |context, target, error, operation_id| {
                    EffectResult::ComputeInstanceActionFailed {
                        context,
                        instance_name: target.name,
                        instance_zone: target.location,
                        action,
                        error,
                        operation_id,
                    }
                }),
            },
            BatchAction::CloudSql(action) => Self {
                run: Arc::new(
                    move |backends, context, target, operation_id, cancellation| {
                        // Cloud SQL identifies an instance by name alone, so
                        // the target's `location` (its region) is carried for
                        // display only and never reaches the command.
                        run_cloud_sql_action_result(
                            backends,
                            context,
                            target.name,
                            action,
                            operation_id,
                            cancellation,
                        )
                    },
                ),
                join_error: Arc::new(move |context, target, error, operation_id| {
                    EffectResult::CloudSqlInstanceActionFailed {
                        context,
                        instance_name: target.name,
                        action,
                        error,
                        operation_id,
                    }
                }),
            },
        }
    }
}

impl Drop for RuntimeSession {
    fn drop(&mut self) {
        self.context_picker_cancellation.cancel();
        // `tunnels` is a field, so its own destructor runs from here and stops
        // anything the quit path did not reach (a panic, an early return).
    }
}

/// Formats the filename component of a log dump as `YYYYMMDD-HHMMSS.mmm`.
/// Seconds alone are too coarse: two saves inside the same second would resolve
/// to one filename and the truncating write would drop the earlier dump while
/// the success status still named it. The millisecond suffix keeps the name
/// readable and the `logs-<timestamp>.log` shape intact. Taking the instant as
/// an argument keeps the writer (and this formatting) clock-free and testable.
pub fn format_log_dump_timestamp(now: DateTime<Local>) -> String {
    now.format("%Y%m%d-%H%M%S%.3f").to_string()
}

/// Stops a supervised child from a `spawn_blocking` thread.
/// [`TunnelControl::shutdown`] signals, polls, and kills — all blocking — so
/// calling it on the reactor would stall every other task for the grace.
async fn shutdown_supervised_tunnel(control: &Arc<dyn TunnelControl>, reason: &str) {
    let control = Arc::clone(control);
    match tokio::task::spawn_blocking(move || control.shutdown(TUNNEL_STOP_GRACE)).await {
        // The control stays registered: a later `StopTunnel` or the quit path
        // will signal it again, which the trait requires to be idempotent, and
        // that is cheaper than teaching the supervisor to reach back into the
        // session's registry from a detached task.
        Ok(outcome) => tracing::debug!(?outcome, reason, "supervised tunnel child shut down"),
        Err(error) => tracing::warn!(%error, reason, "the tunnel shutdown task failed"),
    }
}

/// Keeps a child's output line in the supervisor's diagnostic tail, evicting
/// the oldest once it is full. Every line class that is kept goes through here,
/// so the retention bound and the eviction policy exist in exactly one place
/// rather than being restated per arm and drifting apart.
fn push_tunnel_tail_line(tail: &mut VecDeque<String>, line: String) {
    if tail.len() == TUNNEL_TAIL_LINES {
        tail.pop_front();
    }
    tail.push_back(line);
}

/// The message a tunnel that never reported ready is failed with: the timeout
/// itself plus whatever the child last printed, which is the only clue about
/// what it was doing. The tail is untrusted child output, so every line is
/// sanitized and the whole message is length-bounded.
fn readiness_timeout_message(timeout: Duration, tail: &VecDeque<String>) -> String {
    let mut message = format!("the tunnel did not become ready within {timeout:?}");
    if !tail.is_empty() {
        let captured: Vec<String> = tail
            .iter()
            .map(|line| sanitize_display_text(line))
            .collect();
        message.push_str("; last output: ");
        message.push_str(&captured.join(" | "));
    }
    bounded_tunnel_message(&message)
}

/// Sanitizes and length-bounds one tunnel message on its way into app state.
fn bounded_tunnel_message(message: &str) -> String {
    sanitize_display_text(message)
        .chars()
        .take(MAX_TUNNEL_MESSAGE_CHARS)
        .collect()
}

fn effect_operation(effect: &Effect) -> Option<(OperationPhase, &'static str)> {
    match effect {
        Effect::LoadCostRecommendations { .. } => {
            Some((OperationPhase::Refreshing, "cost recommendations"))
        }
        Effect::LoadBudgets { .. } => Some((OperationPhase::Refreshing, "budgets")),
        Effect::LoadConfigurations => Some((OperationPhase::Loading, "configurations")),
        Effect::ActivateConfiguration { .. }
        | Effect::CreateConfiguration { .. }
        | Effect::UpdateConfigurationContext { .. } => {
            Some((OperationPhase::Refreshing, "gcloud configuration"))
        }
        Effect::RefreshComputeInstances { .. } => {
            Some((OperationPhase::Refreshing, "GCE instances"))
        }
        Effect::RefreshGkeClusters { .. } => Some((OperationPhase::Refreshing, "GKE clusters")),
        Effect::RefreshSecrets { .. } => Some((OperationPhase::Refreshing, "secrets")),
        Effect::RefreshCloudFunctions { .. } => {
            Some((OperationPhase::Refreshing, "Cloud Functions"))
        }
        Effect::RefreshPersistentDisks { .. } => {
            Some((OperationPhase::Refreshing, "persistent disks"))
        }
        Effect::RefreshPubSubTopics { .. } => {
            Some((OperationPhase::Refreshing, "Pub/Sub topics"))
        }
        Effect::RefreshVpcNetworks { .. } => Some((OperationPhase::Refreshing, "VPC networks")),
        Effect::RefreshVpcSubnets { .. } => Some((OperationPhase::Refreshing, "VPC subnets")),
        Effect::RefreshFirewallRules { .. } => {
            Some((OperationPhase::Refreshing, "firewall rules"))
        }
        Effect::RefreshPubSubSubscriptions { .. } => {
            Some((OperationPhase::Refreshing, "Pub/Sub subscriptions"))
        }
        Effect::RefreshBigQueryDatasets { .. } => {
            Some((OperationPhase::Refreshing, "BigQuery datasets"))
        }
        Effect::LoadBigQueryTables { .. } => Some((OperationPhase::Loading, "BigQuery tables")),
        // No operation, and so no `operation_generation` bump either. One
        // landed listing issues one of these per row, so an operation apiece
        // would be N status-line takeovers and N operation records for a call
        // the operator never asked for by name — and under the generation rule an
        // effect that opens no operation cannot supersede the fetch they did
        // ask for. The listing already reported for this view.
        Effect::LoadBigQueryTableStats { .. } => None,
        Effect::LoadBigQuerySchema { .. } => Some((OperationPhase::Loading, "BigQuery schema")),
        Effect::RefreshServiceAccounts { .. } => {
            Some((OperationPhase::Refreshing, "service accounts"))
        }
        Effect::RefreshIamPolicy { .. } => Some((OperationPhase::Refreshing, "IAM policy")),
        Effect::LoadSecretDetail { .. } => Some((OperationPhase::Loading, "secret detail")),
        Effect::LoadSecretVersions { .. } => Some((OperationPhase::Loading, "secret versions")),
        Effect::RefreshCloudSqlInstances { .. } => {
            Some((OperationPhase::Refreshing, "Cloud SQL instances"))
        }
        Effect::LoadComputeInstanceDetail { .. } => Some((OperationPhase::Loading, "GCE detail")),
        Effect::LoadGkeClusterDetail { .. } => Some((OperationPhase::Loading, "GKE detail")),
        Effect::LoadCloudSqlInstanceDetail { .. } => {
            Some((OperationPhase::Loading, "Cloud SQL detail"))
        }
        Effect::LoadCloudSqlUsers { .. } => Some((OperationPhase::Loading, "Cloud SQL users")),
        Effect::RefreshStorageBuckets { .. } => Some((OperationPhase::Refreshing, "Cloud Storage")),
        Effect::LoadStorageBucketDetail { .. } => {
            Some((OperationPhase::Loading, "Cloud Storage detail"))
        }
        Effect::LoadStorageObjectPage { .. } => {
            Some((OperationPhase::Loading, "Cloud Storage objects"))
        }
        Effect::LoadStorageObjectDetail { .. } => {
            Some((OperationPhase::Loading, "Cloud Storage object detail"))
        }
        Effect::ComputeInstanceAction { .. } => Some((OperationPhase::Refreshing, "GCE action")),
        Effect::RefreshCloudRunServices { .. } => {
            Some((OperationPhase::Refreshing, "Cloud Run services"))
        }
        Effect::LoadCloudRunServiceDetail { .. } => {
            Some((OperationPhase::Loading, "Cloud Run detail"))
        }
        Effect::LoadCloudRunRevisions { .. } => {
            Some((OperationPhase::Loading, "Cloud Run revisions"))
        }
        Effect::LoadCloudRunRevisionDetail { .. } => {
            Some((OperationPhase::Loading, "Cloud Run revision detail"))
        }
        Effect::CloudSqlInstanceAction { .. } => {
            Some((OperationPhase::Refreshing, "Cloud SQL action"))
        }
        Effect::RunPlugin { .. } => Some((OperationPhase::Refreshing, "plugin")),
        // No operation, and therefore no `operation_generation` bump. Under the
        // generation rule an effect that opens no operation must not supersede an
        // in-flight fetch, and cancelling a plugin is not the operator starting
        // something the status line should switch to reporting.
        Effect::CancelRunningPlugins => None,
        Effect::LoadRecentLogs { .. } => Some((OperationPhase::Loading, "logs")),
        Effect::TailLogs { .. } => Some((OperationPhase::Refreshing, "logs")),
        Effect::RunResourceBatch { action, .. } => Some((
            OperationPhase::Refreshing,
            match action {
                BatchAction::Compute(_) => "GCE batch",
                BatchAction::CloudSql(_) => "Cloud SQL batch",
            },
        )),
        Effect::LoadContextPicker { .. }
        | Effect::CopyToClipboard { .. }
        // No operation label: the save is a background write the user never
        // asked for by name, so it must not take over the status line's
        // operation from whatever they were actually doing.
        | Effect::SaveRememberedViews { .. }
        | Effect::SaveLogEntries { .. }
        | Effect::ProbeGcloudDiagnostics { .. }
        | Effect::LaunchInteractiveProcess { .. }
        | Effect::PrepareK9sLaunch { .. }
        | Effect::RunK9sCredentials { .. }
        | Effect::StartTunnel { .. }
        | Effect::StopTunnel { .. }
        | Effect::ReloadConfig => None,
    }
}

fn effect_result_operation(
    result: &EffectResult,
    operation_label: &str,
) -> Option<(OperationPhase, &'static str)> {
    match result {
        EffectResult::CostRecommendationsLoaded { .. } => {
            Some((OperationPhase::Healthy, "cost recommendations"))
        }
        EffectResult::CostRecommendationsFailed { .. } => {
            Some((OperationPhase::Degraded, "cost recommendations"))
        }
        EffectResult::BudgetsLoaded { .. } => Some((OperationPhase::Healthy, "budgets")),
        EffectResult::BudgetsFailed { .. } => Some((OperationPhase::Degraded, "budgets")),
        EffectResult::ConfigurationsLoaded { .. } => Some((
            OperationPhase::Healthy,
            configuration_operation_label(operation_label),
        )),
        EffectResult::ConfigurationActivated { .. } => {
            Some((OperationPhase::Healthy, "gcloud configuration"))
        }
        EffectResult::ConfigurationActivationReloadFailed { .. }
        | EffectResult::ConfigurationsReloadedWithWarning { .. }
        | EffectResult::ConfigurationCreatedReloadFailed { .. }
        | EffectResult::ConfigurationContextUpdateReloadFailed { .. } => {
            Some((OperationPhase::Degraded, "gcloud configuration"))
        }
        EffectResult::Failed { .. } if is_configuration_operation(operation_label) => Some((
            OperationPhase::Degraded,
            configuration_operation_label(operation_label),
        )),
        EffectResult::ComputeInstancesLoaded { .. } => {
            Some((OperationPhase::Healthy, "GCE instances"))
        }
        EffectResult::ComputeInstancesLoadFailed { .. } => {
            Some((OperationPhase::Degraded, "GCE instances"))
        }
        EffectResult::GkeClustersLoaded { .. } => Some((OperationPhase::Healthy, "GKE clusters")),
        EffectResult::GkeClustersLoadFailed { .. } => {
            Some((OperationPhase::Degraded, "GKE clusters"))
        }
        EffectResult::SecretsLoaded { .. } => Some((OperationPhase::Healthy, "secrets")),
        EffectResult::CloudFunctionsLoaded { .. } => {
            Some((OperationPhase::Healthy, "Cloud Functions"))
        }
        EffectResult::PubSubTopicsLoadFailed { .. } => {
            Some((OperationPhase::Degraded, "Pub/Sub topics"))
        }
        EffectResult::VpcNetworksLoadFailed { .. } => {
            Some((OperationPhase::Degraded, "VPC networks"))
        }
        EffectResult::VpcSubnetsLoadFailed { .. } => {
            Some((OperationPhase::Degraded, "VPC subnets"))
        }
        EffectResult::FirewallRulesLoadFailed { .. } => {
            Some((OperationPhase::Degraded, "firewall rules"))
        }
        EffectResult::PubSubSubscriptionsLoadFailed { .. } => {
            Some((OperationPhase::Degraded, "Pub/Sub subscriptions"))
        }
        EffectResult::SecretsLoadFailed { .. } => Some((OperationPhase::Degraded, "secrets")),
        EffectResult::CloudFunctionsLoadFailed { .. } => {
            Some((OperationPhase::Degraded, "Cloud Functions"))
        }
        EffectResult::PersistentDisksLoaded { .. } => {
            Some((OperationPhase::Healthy, "persistent disks"))
        }
        EffectResult::PersistentDisksLoadFailed { .. } => {
            Some((OperationPhase::Degraded, "persistent disks"))
        }
        EffectResult::PubSubTopicsLoaded { .. } => {
            Some((OperationPhase::Healthy, "Pub/Sub topics"))
        }
        EffectResult::VpcNetworksLoaded { .. } => Some((OperationPhase::Healthy, "VPC networks")),
        EffectResult::VpcSubnetsLoaded { .. } => Some((OperationPhase::Healthy, "VPC subnets")),
        EffectResult::FirewallRulesLoaded { .. } => {
            Some((OperationPhase::Healthy, "firewall rules"))
        }
        EffectResult::PubSubSubscriptionsLoaded { .. } => {
            Some((OperationPhase::Healthy, "Pub/Sub subscriptions"))
        }
        EffectResult::BigQueryDatasetsLoaded { .. } => {
            Some((OperationPhase::Healthy, "BigQuery datasets"))
        }
        EffectResult::BigQueryDatasetsLoadFailed { .. } => {
            Some((OperationPhase::Degraded, "BigQuery datasets"))
        }
        EffectResult::BigQueryTablesLoaded { .. } => {
            Some((OperationPhase::Healthy, "BigQuery tables"))
        }
        EffectResult::BigQueryTablesLoadFailed { .. } => {
            Some((OperationPhase::Degraded, "BigQuery tables"))
        }
        // Silent in both outcomes, matching the effect: a per-row `show` opens
        // no operation, so there is none for its answer to settle. A refusal
        // in particular must not report `Degraded` — one denied
        // `bigquery.tables.get` out of two hundred is not a degraded table,
        // and the row's own empty cells are where it is visible.
        EffectResult::BigQueryTableStatsLoaded { .. }
        | EffectResult::BigQueryTableStatsLoadFailed { .. } => None,
        EffectResult::BigQuerySchemaLoaded { .. } => {
            Some((OperationPhase::Healthy, "BigQuery schema"))
        }
        EffectResult::BigQuerySchemaLoadFailed { .. } => {
            Some((OperationPhase::Degraded, "BigQuery schema"))
        }
        EffectResult::ServiceAccountsLoaded { .. } => {
            Some((OperationPhase::Healthy, "service accounts"))
        }
        EffectResult::ServiceAccountsLoadFailed { .. } => {
            Some((OperationPhase::Degraded, "service accounts"))
        }
        EffectResult::IamPolicyLoaded { .. } => Some((OperationPhase::Healthy, "IAM policy")),
        EffectResult::IamPolicyLoadFailed { .. } => Some((OperationPhase::Degraded, "IAM policy")),
        EffectResult::SecretDetailLoaded { .. } => Some((OperationPhase::Healthy, "secret detail")),
        EffectResult::SecretDetailLoadFailed { .. } => {
            Some((OperationPhase::Degraded, "secret detail"))
        }
        EffectResult::SecretVersionsLoaded { .. } => {
            Some((OperationPhase::Healthy, "secret versions"))
        }
        EffectResult::SecretVersionsLoadFailed { .. } => {
            Some((OperationPhase::Degraded, "secret versions"))
        }
        EffectResult::ComputeInstanceDetailLoaded { .. } => {
            Some((OperationPhase::Healthy, "GCE detail"))
        }
        EffectResult::ComputeInstanceDetailLoadFailed { .. } => {
            Some((OperationPhase::Failed, "GCE detail"))
        }
        EffectResult::GkeClusterDetailLoaded { .. } => {
            Some((OperationPhase::Healthy, "GKE detail"))
        }
        EffectResult::GkeClusterDetailLoadFailed { .. } => {
            Some((OperationPhase::Failed, "GKE detail"))
        }
        EffectResult::CloudSqlInstancesLoaded { .. } => {
            Some((OperationPhase::Healthy, "Cloud SQL instances"))
        }
        EffectResult::CloudSqlInstancesLoadFailed { .. } => {
            Some((OperationPhase::Degraded, "Cloud SQL instances"))
        }
        EffectResult::CloudSqlInstanceDetailLoaded { .. } => {
            Some((OperationPhase::Healthy, "Cloud SQL detail"))
        }
        EffectResult::CloudSqlInstanceDetailLoadFailed { .. } => {
            Some((OperationPhase::Failed, "Cloud SQL detail"))
        }
        EffectResult::CloudSqlUsersLoaded { .. } => {
            Some((OperationPhase::Healthy, "Cloud SQL users"))
        }
        EffectResult::CloudSqlUsersLoadFailed { .. } => {
            Some((OperationPhase::Failed, "Cloud SQL users"))
        }
        EffectResult::StorageBucketsLoaded { .. } => {
            Some((OperationPhase::Healthy, "Cloud Storage"))
        }
        EffectResult::StorageBucketsLoadFailed { .. } => {
            Some((OperationPhase::Degraded, "Cloud Storage"))
        }
        EffectResult::StorageBucketDetailLoaded { .. } => {
            Some((OperationPhase::Healthy, "Cloud Storage detail"))
        }
        EffectResult::StorageBucketDetailLoadFailed { .. } => {
            Some((OperationPhase::Failed, "Cloud Storage detail"))
        }
        EffectResult::StorageObjectPageLoaded { .. } => {
            Some((OperationPhase::Healthy, "Cloud Storage objects"))
        }
        EffectResult::StorageObjectPageLoadFailed { .. } => {
            Some((OperationPhase::Failed, "Cloud Storage objects"))
        }
        EffectResult::StorageObjectDetailLoaded { .. } => {
            Some((OperationPhase::Healthy, "Cloud Storage object detail"))
        }
        EffectResult::StorageObjectDetailLoadFailed { .. } => {
            Some((OperationPhase::Failed, "Cloud Storage object detail"))
        }
        EffectResult::ComputeInstanceActionSucceeded { .. } => {
            Some((OperationPhase::Healthy, "GCE action"))
        }
        EffectResult::ComputeInstanceActionFailed { .. } => {
            Some((OperationPhase::Failed, "GCE action"))
        }
        EffectResult::CloudRunServicesLoaded { .. } => {
            Some((OperationPhase::Healthy, "Cloud Run services"))
        }
        EffectResult::CloudRunServicesLoadFailed { .. } => {
            Some((OperationPhase::Degraded, "Cloud Run services"))
        }
        EffectResult::CloudRunServiceDetailLoaded { .. } => {
            Some((OperationPhase::Healthy, "Cloud Run detail"))
        }
        EffectResult::CloudRunServiceDetailLoadFailed { .. } => {
            Some((OperationPhase::Failed, "Cloud Run detail"))
        }
        EffectResult::CloudRunRevisionsLoaded { .. } => {
            Some((OperationPhase::Healthy, "Cloud Run revisions"))
        }
        EffectResult::CloudRunRevisionsLoadFailed { .. } => {
            Some((OperationPhase::Degraded, "Cloud Run revisions"))
        }
        EffectResult::CloudRunRevisionDetailLoaded { .. } => {
            Some((OperationPhase::Healthy, "Cloud Run revision detail"))
        }
        EffectResult::CloudRunRevisionDetailLoadFailed { .. } => {
            Some((OperationPhase::Failed, "Cloud Run revision detail"))
        }
        EffectResult::PluginRunSucceeded { .. } => Some((OperationPhase::Healthy, "plugin")),
        EffectResult::PluginRunFailed { .. } => Some((OperationPhase::Failed, "plugin")),
        // Healthy, not Failed: the run ended the way the operator asked it to.
        EffectResult::PluginRunCancelled { .. } => Some((OperationPhase::Healthy, "plugin")),
        EffectResult::CloudSqlInstanceActionSucceeded { .. } => {
            Some((OperationPhase::Healthy, "Cloud SQL action"))
        }
        EffectResult::CloudSqlInstanceActionFailed { .. } => {
            Some((OperationPhase::Failed, "Cloud SQL action"))
        }
        EffectResult::RecentLogsLoaded { .. } | EffectResult::LogsTailed { .. } => {
            Some((OperationPhase::Healthy, "logs"))
        }
        EffectResult::RecentLogsLoadFailed { .. } | EffectResult::LogsTailFailed { .. } => {
            Some((OperationPhase::Degraded, "logs"))
        }
        EffectResult::ContextPickerLoaded { .. }
        | EffectResult::ContextPickerLoadFailed { .. }
        | EffectResult::ClipboardCopied { .. }
        | EffectResult::ClipboardCopyFailed { .. }
        | EffectResult::LogEntriesSaved { .. }
        | EffectResult::LogEntriesSaveFailed { .. }
        // Matching the effect side: the save carries no operation, so neither
        // outcome may set an operation phase on the status line.
        | EffectResult::RememberedViewsSaved
        | EffectResult::RememberedViewsSaveFailed { .. }
        | EffectResult::GcloudDiagnosticsProbed { .. }
        | EffectResult::InteractiveProcessCompleted { .. }
        | EffectResult::K9sLaunchPrepared { .. }
        | EffectResult::K9sCredentialsReady { .. }
        | EffectResult::K9sCredentialsFailed { .. }
        | EffectResult::TunnelPortAllocated { .. }
        | EffectResult::TunnelReady { .. }
        | EffectResult::TunnelFailed { .. }
        | EffectResult::TunnelNotice { .. }
        | EffectResult::TunnelExited { .. }
        | EffectResult::TunnelStopped { .. }
        | EffectResult::Failed { .. } => None,
    }
}

fn is_refresh_result(result: &EffectResult, operation_label: &str) -> bool {
    match result {
        EffectResult::ConfigurationsLoaded { .. }
        | EffectResult::ConfigurationActivationReloadFailed { .. }
        | EffectResult::ConfigurationsReloadedWithWarning { .. }
        | EffectResult::ConfigurationCreatedReloadFailed { .. }
        | EffectResult::ConfigurationContextUpdateReloadFailed { .. }
        | EffectResult::ComputeInstancesLoaded { .. }
        | EffectResult::ComputeInstancesLoadFailed { .. }
        | EffectResult::GkeClustersLoaded { .. }
        | EffectResult::GkeClustersLoadFailed { .. }
        | EffectResult::CloudRunServicesLoaded { .. }
        | EffectResult::CloudRunServicesLoadFailed { .. }
        | EffectResult::CloudSqlInstancesLoaded { .. }
        | EffectResult::CloudSqlInstancesLoadFailed { .. }
        | EffectResult::RecentLogsLoaded { .. }
        | EffectResult::RecentLogsLoadFailed { .. }
        | EffectResult::LogsTailed { .. }
        | EffectResult::LogsTailFailed { .. }
        | EffectResult::StorageBucketsLoaded { .. }
        | EffectResult::StorageBucketsLoadFailed { .. } => true,
        EffectResult::Failed { .. } => matches!(
            operation_label,
            "configurations"
                | "GCE instances"
                | "GKE clusters"
                | "Cloud Run services"
                | "Cloud SQL instances"
                | "logs"
                | "Cloud Storage"
        ),
        _ => false,
    }
}

fn is_configuration_operation(label: &str) -> bool {
    matches!(label, "configurations" | "gcloud configuration")
}

fn configuration_operation_label(label: &str) -> &'static str {
    if label == "gcloud configuration" {
        "gcloud configuration"
    } else {
        "configurations"
    }
}

fn load_context_picker(
    runner: &dyn CommandRunner,
    sdk: Option<&crate::services::sdk::SdkClients>,
    kind: ConfigurationContextPickerKind,
    context: GcloudContext,
    cancellation: &CancellationToken,
) -> Option<EffectResult> {
    if let Some(clients) = sdk {
        return load_context_picker_natively(clients, kind, context, cancellation);
    }
    let configuration = context.configuration;
    let service = GcloudDiscoveryService::new(runner);
    let items = match kind {
        ConfigurationContextPickerKind::Project => service
            .load_projects_with_cancellation(&configuration, cancellation)
            .map(|rows| {
                rows.into_iter()
                    .map(ConfigurationContextPickerItem::Project)
                    .collect()
            }),
        ConfigurationContextPickerKind::Region => service
            .load_regions_with_cancellation(&configuration, cancellation)
            .map(|rows| {
                rows.into_iter()
                    .map(ConfigurationContextPickerItem::Region)
                    .collect()
            }),
        ConfigurationContextPickerKind::Zone => service
            .load_zones_with_cancellation(&configuration, cancellation)
            .map(|rows| {
                rows.into_iter()
                    .map(ConfigurationContextPickerItem::Zone)
                    .collect()
            }),
    };
    if cancellation.is_cancelled() {
        return None;
    }
    match items {
        Ok(items) => Some(EffectResult::ContextPickerLoaded {
            configuration,
            kind,
            items,
        }),
        Err(GcloudDiscoveryError::Command(error)) => Some(EffectResult::ContextPickerLoadFailed {
            configuration,
            kind,
            error: error.into(),
        }),
        Err(GcloudDiscoveryError::Parse(error)) => Some(EffectResult::ContextPickerLoadFailed {
            configuration,
            kind,
            error: AppError::Parse(error.to_string()),
        }),
    }
}

/// The picker over the native clients.
///
/// `None` is returned for a cancelled load exactly as on the CLI path, and
/// means the same thing: the operator moved on, so there is no result to
/// deliver. Here the request is dropped rather than a process killed.
///
/// Regions and zones are project-scoped where projects are not, so an unset
/// project reaches the API as an empty one and is refused there -- which is
/// what `gcloud compute regions list` does with no project configured too.
fn load_context_picker_natively(
    clients: &crate::services::sdk::SdkClients,
    kind: ConfigurationContextPickerKind,
    context: GcloudContext,
    cancellation: &CancellationToken,
) -> Option<EffectResult> {
    let project = context.project.clone().unwrap_or_default();
    let configuration = context.configuration;
    let client = match clients.discovery(configuration.as_str()) {
        Ok(client) => client,
        Err(error) => {
            return Some(EffectResult::ContextPickerLoadFailed {
                configuration,
                kind,
                error: AppError::gcloud(error.to_string()),
            });
        }
    };

    let loaded = match kind {
        ConfigurationContextPickerKind::Project => {
            crate::services::sdk::block_on_cancellable(client.load_projects(), cancellation).map(
                |result| {
                    result.map(|rows| {
                        rows.into_iter()
                            .map(ConfigurationContextPickerItem::Project)
                            .collect::<Vec<_>>()
                    })
                },
            )
        }
        ConfigurationContextPickerKind::Region => {
            crate::services::sdk::block_on_cancellable(client.load_regions(&project), cancellation)
                .map(|result| {
                    result.map(|rows| {
                        rows.into_iter()
                            .map(ConfigurationContextPickerItem::Region)
                            .collect::<Vec<_>>()
                    })
                })
        }
        ConfigurationContextPickerKind::Zone => {
            crate::services::sdk::block_on_cancellable(client.load_zones(&project), cancellation)
                .map(|result| {
                    result.map(|rows| {
                        rows.into_iter()
                            .map(ConfigurationContextPickerItem::Zone)
                            .collect::<Vec<_>>()
                    })
                })
        }
    };

    // `None` from the helper is cancellation; so is a token cancelled while the
    // rows were being mapped, which is the check the CLI path also makes.
    let loaded = loaded?;
    if cancellation.is_cancelled() {
        return None;
    }
    match loaded {
        Ok(items) => Some(EffectResult::ContextPickerLoaded {
            configuration,
            kind,
            items,
        }),
        Err(error) => Some(EffectResult::ContextPickerLoadFailed {
            configuration,
            kind,
            error: AppError::gcloud(error.to_string()),
        }),
    }
}

fn load_compute_instances(
    runner: &(dyn CommandRunner + Send + Sync),
    sdk: Option<&crate::services::sdk::SdkClients>,
    context: GcloudContext,
    filter: Option<String>,
) -> EffectResult {
    // Label mode is honoured only by the server-side expression, which the
    // native clients do not carry, so such a read goes through gcloud.
    if let Some(clients) =
        crate::services::resources::list_filter::native_clients_for(sdk, filter.as_deref())
    {
        let project = context.project.clone().unwrap_or_default();
        let loaded = clients
            .compute(context.configuration.as_str())
            .and_then(|client| crate::services::sdk::block_on(client.load_instances(&project)));
        return match loaded {
            Ok(instances) => EffectResult::ComputeInstancesLoaded { context, instances },
            Err(error) => EffectResult::ComputeInstancesLoadFailed {
                context,
                error: AppError::gcloud(error.to_string()),
            },
        };
    }
    let service = GcloudComputeService::new(runner);
    match service.load_instances(&context, filter.as_deref()) {
        Ok(instances) => EffectResult::ComputeInstancesLoaded { context, instances },
        Err(crate::services::resources::compute::ComputeLoadError::Command(error)) => {
            let error: AppError = error.into();
            EffectResult::ComputeInstancesLoadFailed {
                context,
                error: error.with_command_summary("gcloud compute instances list"),
            }
        }
        Err(crate::services::resources::compute::ComputeLoadError::Parse(error)) => {
            EffectResult::ComputeInstancesLoadFailed {
                context,
                error: AppError::Parse(error.to_string()),
            }
        }
    }
}

fn load_gke_clusters(
    runner: &(dyn CommandRunner + Send + Sync),
    sdk: Option<&crate::services::sdk::SdkClients>,
    context: GcloudContext,
    filter: Option<String>,
) -> EffectResult {
    // Label mode is honoured only by the server-side expression, which the
    // native clients do not carry, so such a read goes through gcloud.
    if let Some(clients) =
        crate::services::resources::list_filter::native_clients_for(sdk, filter.as_deref())
    {
        let project = context.project.clone().unwrap_or_default();
        let loaded = clients
            .gke(context.configuration.as_str())
            .and_then(|client| crate::services::sdk::block_on(client.load_clusters(&project)));
        return match loaded {
            Ok(clusters) => EffectResult::GkeClustersLoaded { context, clusters },
            Err(error) => EffectResult::GkeClustersLoadFailed {
                context,
                error: AppError::gcloud(error.to_string()),
            },
        };
    }
    let service = GcloudGkeService::new(runner);
    match service.load_clusters(&context, filter.as_deref()) {
        Ok(clusters) => EffectResult::GkeClustersLoaded { context, clusters },
        Err(crate::services::resources::gke::GkeLoadError::Command(error)) => {
            let error: AppError = error.into();
            EffectResult::GkeClustersLoadFailed {
                context,
                error: error.with_command_summary("gcloud container clusters list"),
            }
        }
        Err(crate::services::resources::gke::GkeLoadError::Parse(error)) => {
            EffectResult::GkeClustersLoadFailed {
                context,
                error: AppError::Parse(error.to_string()),
            }
        }
    }
}

fn load_cloud_run_services(
    runner: &(dyn CommandRunner + Send + Sync),
    sdk: Option<&crate::services::sdk::SdkClients>,
    context: GcloudContext,
) -> EffectResult {
    if let Some(clients) = sdk {
        let project = context.project.clone().unwrap_or_default();
        let loaded = clients
            .cloud_run(context.configuration.as_str())
            .and_then(|client| crate::services::sdk::block_on(client.load_services(&project)));
        return match loaded {
            Ok(value) => EffectResult::CloudRunServicesLoaded {
                context,
                services: value,
            },
            Err(error) => EffectResult::CloudRunServicesLoadFailed {
                context,
                error: AppError::gcloud(error.to_string()),
            },
        };
    }
    let service = GcloudCloudRunService::new(runner);
    match service.load_services(&context) {
        Ok(services) => EffectResult::CloudRunServicesLoaded { context, services },
        Err(CloudRunLoadError::Command(error)) => EffectResult::CloudRunServicesLoadFailed {
            context,
            error: AppError::gcloud(error.to_string()),
        },
        Err(CloudRunLoadError::Parse(error)) => EffectResult::CloudRunServicesLoadFailed {
            context,
            error: AppError::Parse(error.to_string()),
        },
    }
}

/// Loads the Cloud Functions list.
fn load_cloud_functions(
    runner: &(dyn CommandRunner + Send + Sync),
    sdk: Option<&crate::services::sdk::SdkClients>,
    context: GcloudContext,
    filter: Option<String>,
) -> EffectResult {
    // Label mode is honoured only by the server-side expression, which the
    // native clients do not carry, so such a read goes through gcloud.
    if let Some(clients) =
        crate::services::resources::list_filter::native_clients_for(sdk, filter.as_deref())
    {
        let project = context.project.clone().unwrap_or_default();
        let loaded = clients
            .functions(context.configuration.as_str())
            .and_then(|client| crate::services::sdk::block_on(client.load_functions(&project)));
        return match loaded {
            Ok(value) => EffectResult::CloudFunctionsLoaded {
                context,
                functions: value,
            },
            Err(error) => EffectResult::CloudFunctionsLoadFailed {
                context,
                error: AppError::gcloud(error.to_string()),
            },
        };
    }
    let service = GcloudFunctionsService::new(runner);
    match service.load_functions(&context, filter.as_deref()) {
        Ok(functions) => EffectResult::CloudFunctionsLoaded { context, functions },
        Err(CloudFunctionsLoadError::Command(error)) => EffectResult::CloudFunctionsLoadFailed {
            context,
            error: AppError::gcloud(error.to_string()),
        },
        Err(CloudFunctionsLoadError::Parse(error)) => EffectResult::CloudFunctionsLoadFailed {
            context,
            error: AppError::Parse(error.to_string()),
        },
    }
}

fn load_persistent_disks(
    runner: &(dyn CommandRunner + Send + Sync),
    sdk: Option<&crate::services::sdk::SdkClients>,
    context: GcloudContext,
    filter: Option<String>,
) -> EffectResult {
    // Label mode is honoured only by the server-side expression, which the
    // native clients do not carry, so such a read goes through gcloud.
    if let Some(clients) =
        crate::services::resources::list_filter::native_clients_for(sdk, filter.as_deref())
    {
        let project = context.project.clone().unwrap_or_default();
        let loaded = clients
            .disks(context.configuration.as_str())
            .and_then(|client| crate::services::sdk::block_on(client.load_disks(&project)));
        return match loaded {
            Ok(value) => EffectResult::PersistentDisksLoaded {
                context,
                disks: value,
            },
            Err(error) => EffectResult::PersistentDisksLoadFailed {
                context,
                error: AppError::gcloud(error.to_string()),
            },
        };
    }
    let service = GcloudDisksService::new(runner);
    match service.load_disks(&context, filter.as_deref()) {
        Ok(disks) => EffectResult::PersistentDisksLoaded { context, disks },
        Err(DisksLoadError::Command(error)) => EffectResult::PersistentDisksLoadFailed {
            context,
            error: AppError::gcloud(error.to_string()),
        },
        Err(DisksLoadError::Parse(error)) => EffectResult::PersistentDisksLoadFailed {
            context,
            error: AppError::Parse(error.to_string()),
        },
    }
}

/// Loads the secrets list. Metadata only — this is `secrets list`, and the
/// payload-returning subcommand has no loader here or anywhere else.
fn load_secrets(
    runner: &(dyn CommandRunner + Send + Sync),
    sdk: Option<&crate::services::sdk::SdkClients>,
    context: GcloudContext,
    filter: Option<String>,
) -> EffectResult {
    // Label mode is honoured only by the server-side expression, which the
    // native clients do not carry, so such a read goes through gcloud.
    if let Some(clients) =
        crate::services::resources::list_filter::native_clients_for(sdk, filter.as_deref())
    {
        let project = context.project.clone().unwrap_or_default();
        let loaded = clients
            .secrets(context.configuration.as_str())
            .and_then(|client| crate::services::sdk::block_on(client.load_secrets(&project)));
        return match loaded {
            Ok(value) => EffectResult::SecretsLoaded {
                context,
                secrets: value,
            },
            Err(error) => EffectResult::SecretsLoadFailed {
                context,
                error: AppError::gcloud(error.to_string()),
            },
        };
    }
    let service = GcloudSecretsService::new(runner);
    match service.load_secrets(&context, filter.as_deref()) {
        Ok(secrets) => EffectResult::SecretsLoaded { context, secrets },
        Err(SecretsLoadError::Command(error)) => EffectResult::SecretsLoadFailed {
            context,
            error: AppError::gcloud(error.to_string()),
        },
        Err(SecretsLoadError::Parse(error)) => EffectResult::SecretsLoadFailed {
            context,
            error: AppError::Parse(error.to_string()),
        },
    }
}

/// The flat subscriptions load.
///
/// Unlike the topics load, which swallows a subscriptions-call failure to keep
/// its other columns, this surfaces every failure: there is nothing left to
/// render, and an empty table would be the claim that the project has no
/// subscriptions.
fn load_pubsub_subscriptions(
    runner: &(dyn CommandRunner + Send + Sync),
    sdk: Option<&crate::services::sdk::SdkClients>,
    context: GcloudContext,
    filter: Option<String>,
) -> EffectResult {
    // Label mode is honoured only by the server-side expression, which the
    // native clients do not carry, so such a read goes through gcloud.
    if let Some(clients) =
        crate::services::resources::list_filter::native_clients_for(sdk, filter.as_deref())
    {
        let project = context.project.clone().unwrap_or_default();
        let loaded = clients
            .pubsub(context.configuration.as_str())
            .and_then(|client| crate::services::sdk::block_on(client.load_subscriptions(&project)));
        return match loaded {
            Ok(value) => EffectResult::PubSubSubscriptionsLoaded {
                context,
                subscriptions: value,
            },
            Err(error) => EffectResult::PubSubSubscriptionsLoadFailed {
                context,
                error: AppError::gcloud(error.to_string()),
            },
        };
    }
    let service = GcloudPubSubService::new(runner);
    match service.load_subscriptions(&context, filter.as_deref()) {
        Ok(subscriptions) => EffectResult::PubSubSubscriptionsLoaded {
            context,
            subscriptions,
        },
        Err(PubSubLoadError::Command(error)) => EffectResult::PubSubSubscriptionsLoadFailed {
            context,
            error: AppError::gcloud(error.to_string()),
        },
        Err(PubSubLoadError::Parse(error)) => EffectResult::PubSubSubscriptionsLoadFailed {
            context,
            error: AppError::Parse(error.to_string()),
        },
    }
}

/// The datasets load.
///
/// Takes no filter, unlike every sibling loader here: `bq ls` has no
/// `--filter`, so there is nothing to push down and the table narrows wholly
/// client-side (see `GcloudBigQueryService::load_datasets`).
fn load_bigquery_datasets(
    runner: &(dyn CommandRunner + Send + Sync),
    context: GcloudContext,
) -> EffectResult {
    let service = GcloudBigQueryService::new(runner);
    match service.load_datasets(&context) {
        Ok(datasets) => EffectResult::BigQueryDatasetsLoaded { context, datasets },
        Err(BigQueryLoadError::Command(error)) => EffectResult::BigQueryDatasetsLoadFailed {
            context,
            error: AppError::gcloud(error.to_string()),
        },
        Err(BigQueryLoadError::Parse(error)) => EffectResult::BigQueryDatasetsLoadFailed {
            context,
            error: AppError::Parse(error.to_string()),
        },
    }
}

/// The tables load for one dataset.
///
/// The drill generation and the listing epoch ride along on both results: they
/// are the scope the reducer checks the answer against before letting it land.
/// The dataset id names what to ask `bq ls` for and stops there.
fn load_bigquery_tables(
    runner: &(dyn CommandRunner + Send + Sync),
    context: GcloudContext,
    dataset_id: String,
    generation: BigQueryDrillGeneration,
    listing: BigQueryListingEpoch,
) -> EffectResult {
    let service = GcloudBigQueryService::new(runner);
    match service.load_tables(&context, &dataset_id) {
        Ok(tables) => EffectResult::BigQueryTablesLoaded {
            context,
            generation,
            listing,
            tables,
        },
        Err(BigQueryLoadError::Command(error)) => EffectResult::BigQueryTablesLoadFailed {
            context,
            generation,
            listing,
            error: AppError::gcloud(error.to_string()),
        },
        Err(BigQueryLoadError::Parse(error)) => EffectResult::BigQueryTablesLoadFailed {
            context,
            generation,
            listing,
            error: AppError::Parse(error.to_string()),
        },
    }
}

/// One table's column schema, for the drill an `Enter` on a table row opened.
///
/// Every failure fails the drill, which is the opposite of
/// [`load_bigquery_table_stats`]'s policy and matches
/// [`load_bigquery_tables`]'s: this call is the whole content of the view the
/// operator just opened, so there is nothing partial left to paint beside a
/// refusal.
///
/// A schema that flattens to no fields is not a failure and does not come
/// through here — `Ok(vec![])` is an answered call, and the arm in
/// `src/app/reducer/root.rs` installs it as an empty level.
///
/// The drill generation rides through untouched, because the answer is checked
/// against the view as it will be when it arrives, not as it was when the call
/// went out.
fn load_bigquery_schema(
    runner: &(dyn CommandRunner + Send + Sync),
    context: GcloudContext,
    dataset_id: &str,
    table_id: &str,
    generation: BigQueryDrillGeneration,
) -> EffectResult {
    let service = GcloudBigQueryService::new(runner);
    match service.load_table_schema(&context, dataset_id, table_id) {
        Ok(fields) => EffectResult::BigQuerySchemaLoaded {
            context,
            generation,
            fields,
        },
        Err(BigQueryLoadError::Command(error)) => EffectResult::BigQuerySchemaLoadFailed {
            context,
            generation,
            error: AppError::gcloud(error.to_string()),
        },
        Err(BigQueryLoadError::Parse(error)) => EffectResult::BigQuerySchemaLoadFailed {
            context,
            generation,
            error: AppError::Parse(error.to_string()),
        },
    }
}

/// One table's stats, for the fan-out over a landed listing.
///
/// Best-effort by construction: every failure becomes
/// [`EffectResult::BigQueryTableStatsLoadFailed`] for this table alone, which
/// the reducer resolves into three unavailable cells on one row. Nothing here
/// can fail the table — that is `bq ls`'s privilege, and keeping the two
/// apart is the point.
///
/// The drill generation and the listing epoch ride through untouched, because
/// the answer is checked against the view as it will be when it arrives, not
/// as it was when the call went out.
fn load_bigquery_table_stats(
    runner: &(dyn CommandRunner + Send + Sync),
    context: GcloudContext,
    dataset_id: &str,
    table_id: String,
    generation: BigQueryDrillGeneration,
    listing: BigQueryListingEpoch,
) -> EffectResult {
    let service = GcloudBigQueryService::new(runner);
    match service.load_table_stats(&context, dataset_id, &table_id) {
        Ok(stats) => EffectResult::BigQueryTableStatsLoaded {
            context,
            table_id,
            generation,
            listing,
            stats,
        },
        Err(BigQueryLoadError::Command(error)) => EffectResult::BigQueryTableStatsLoadFailed {
            context,
            table_id,
            generation,
            listing,
            error: AppError::gcloud(error.to_string()),
        },
        Err(BigQueryLoadError::Parse(error)) => EffectResult::BigQueryTableStatsLoadFailed {
            context,
            table_id,
            generation,
            listing,
            error: AppError::Parse(error.to_string()),
        },
    }
}

/// The networks load.
///
/// Every failure surfaces: there is one call behind this table and no
/// enrichment to degrade to, so a refusal leaves nothing to render, and an
/// empty table would be the claim that the project has no VPC networks —
/// which a denied `compute.networks.list` is in no position to make.
fn load_vpc_networks(
    runner: &(dyn CommandRunner + Send + Sync),
    sdk: Option<&crate::services::sdk::SdkClients>,
    context: GcloudContext,
) -> EffectResult {
    if let Some(clients) = sdk {
        let project = context.project.clone().unwrap_or_default();
        let loaded = clients
            .vpc(context.configuration.as_str())
            .and_then(|client| crate::services::sdk::block_on(client.load_networks(&project)));
        return match loaded {
            Ok(value) => EffectResult::VpcNetworksLoaded {
                context,
                networks: value,
            },
            Err(error) => EffectResult::VpcNetworksLoadFailed {
                context,
                error: AppError::gcloud(error.to_string()),
            },
        };
    }
    let service = GcloudVpcService::new(runner);
    match service.load_networks(&context, None) {
        Ok(networks) => EffectResult::VpcNetworksLoaded { context, networks },
        Err(VpcLoadError::Command(error)) => EffectResult::VpcNetworksLoadFailed {
            context,
            error: AppError::gcloud(error.to_string()),
        },
        Err(VpcLoadError::Parse(error)) => EffectResult::VpcNetworksLoadFailed {
            context,
            error: AppError::Parse(error.to_string()),
        },
    }
}

/// The subnets load. Every failure surfaces, for the reason
/// [`load_vpc_networks`] records: one call, no enrichment to degrade to, and
/// an empty table would claim the project has no subnets.
/// The firewall-rules load. Every failure surfaces: one call, no enrichment
/// to degrade to, and an empty table would claim the project has no firewall
/// rules — which on this table reads as "nothing is blocked".
fn load_firewall_rules(
    runner: &(dyn CommandRunner + Send + Sync),
    sdk: Option<&crate::services::sdk::SdkClients>,
    context: GcloudContext,
) -> EffectResult {
    if let Some(clients) = sdk {
        let project = context.project.clone().unwrap_or_default();
        let loaded = clients
            .vpc(context.configuration.as_str())
            .and_then(|client| {
                crate::services::sdk::block_on(client.load_firewall_rules(&project))
            });
        return match loaded {
            Ok(value) => EffectResult::FirewallRulesLoaded {
                context,
                rules: value,
            },
            Err(error) => EffectResult::FirewallRulesLoadFailed {
                context,
                error: AppError::gcloud(error.to_string()),
            },
        };
    }
    let service = GcloudVpcService::new(runner);
    match service.load_firewall_rules(&context, None) {
        Ok(rules) => EffectResult::FirewallRulesLoaded { context, rules },
        Err(VpcLoadError::Command(error)) => EffectResult::FirewallRulesLoadFailed {
            context,
            error: AppError::gcloud(error.to_string()),
        },
        Err(VpcLoadError::Parse(error)) => EffectResult::FirewallRulesLoadFailed {
            context,
            error: AppError::Parse(error.to_string()),
        },
    }
}

fn load_vpc_subnets(
    runner: &(dyn CommandRunner + Send + Sync),
    sdk: Option<&crate::services::sdk::SdkClients>,
    context: GcloudContext,
) -> EffectResult {
    if let Some(clients) = sdk {
        let project = context.project.clone().unwrap_or_default();
        let loaded = clients
            .vpc(context.configuration.as_str())
            .and_then(|client| crate::services::sdk::block_on(client.load_subnets(&project)));
        return match loaded {
            Ok(value) => EffectResult::VpcSubnetsLoaded {
                context,
                subnets: value,
            },
            Err(error) => EffectResult::VpcSubnetsLoadFailed {
                context,
                error: AppError::gcloud(error.to_string()),
            },
        };
    }
    let service = GcloudVpcService::new(runner);
    match service.load_subnets(&context, None) {
        Ok(subnets) => EffectResult::VpcSubnetsLoaded { context, subnets },
        Err(VpcLoadError::Command(error)) => EffectResult::VpcSubnetsLoadFailed {
            context,
            error: AppError::gcloud(error.to_string()),
        },
        Err(VpcLoadError::Parse(error)) => EffectResult::VpcSubnetsLoadFailed {
            context,
            error: AppError::Parse(error.to_string()),
        },
    }
}

fn load_pubsub_topics(
    runner: &(dyn CommandRunner + Send + Sync),
    sdk: Option<&crate::services::sdk::SdkClients>,
    context: GcloudContext,
    filter: Option<String>,
) -> EffectResult {
    // Label mode is honoured only by the server-side expression, which the
    // native clients do not carry, so such a read goes through gcloud.
    if let Some(clients) =
        crate::services::resources::list_filter::native_clients_for(sdk, filter.as_deref())
    {
        let project = context.project.clone().unwrap_or_default();
        let loaded = clients
            .pubsub(context.configuration.as_str())
            .and_then(|client| crate::services::sdk::block_on(client.load_topics(&project)));
        return match loaded {
            Ok(value) => EffectResult::PubSubTopicsLoaded {
                context,
                topics: value,
            },
            Err(error) => EffectResult::PubSubTopicsLoadFailed {
                context,
                error: AppError::gcloud(error.to_string()),
            },
        };
    }
    let service = GcloudPubSubService::new(runner);
    match service.load_topics(&context, filter.as_deref()) {
        Ok(topics) => EffectResult::PubSubTopicsLoaded { context, topics },
        Err(PubSubLoadError::Command(error)) => EffectResult::PubSubTopicsLoadFailed {
            context,
            error: AppError::gcloud(error.to_string()),
        },
        Err(PubSubLoadError::Parse(error)) => EffectResult::PubSubTopicsLoadFailed {
            context,
            error: AppError::Parse(error.to_string()),
        },
    }
}

fn load_service_accounts(
    runner: &(dyn CommandRunner + Send + Sync),
    sdk: Option<&crate::services::sdk::SdkClients>,
    context: GcloudContext,
    filter: Option<String>,
) -> EffectResult {
    // Label mode is honoured only by the server-side expression, which the
    // native clients do not carry, so such a read goes through gcloud.
    if let Some(clients) =
        crate::services::resources::list_filter::native_clients_for(sdk, filter.as_deref())
    {
        let project = context.project.clone().unwrap_or_default();
        let loaded = clients
            .iam(context.configuration.as_str())
            .and_then(|client| {
                crate::services::sdk::block_on(client.load_service_accounts(&project))
            });
        return match loaded {
            Ok(value) => EffectResult::ServiceAccountsLoaded {
                context,
                accounts: value,
            },
            Err(error) => EffectResult::ServiceAccountsLoadFailed {
                context,
                error: AppError::gcloud(error.to_string()),
            },
        };
    }
    let service = GcloudIamService::new(runner);
    match service.load_service_accounts(&context, filter.as_deref()) {
        Ok(accounts) => EffectResult::ServiceAccountsLoaded { context, accounts },
        Err(IamLoadError::Command(error)) => EffectResult::ServiceAccountsLoadFailed {
            context,
            error: AppError::gcloud(error.to_string()),
        },
        Err(IamLoadError::Parse(error)) => EffectResult::ServiceAccountsLoadFailed {
            context,
            error: AppError::Parse(error.to_string()),
        },
        // The service-accounts list never needs a project of its own, but the
        // error type is shared with the policy read, so the arm exists; keep
        // the load error's own words rather than substituting a context error
        // it did not report.
        Err(error @ IamLoadError::MissingProject) => EffectResult::ServiceAccountsLoadFailed {
            context,
            error: AppError::gcloud(error.to_string()),
        },
    }
}

/// The policy read takes no `filter`: it fetches one policy object and
/// flattens it, so there is no list request a clause could narrow.
fn load_policy_members(
    runner: &(dyn CommandRunner + Send + Sync),
    sdk: Option<&crate::services::sdk::SdkClients>,
    context: GcloudContext,
) -> EffectResult {
    if let Some(clients) = sdk {
        let project = context.project.clone().unwrap_or_default();
        let loaded = clients
            .iam(context.configuration.as_str())
            .and_then(|client| crate::services::sdk::block_on(client.load_policy(&project)));
        return match loaded {
            Ok(value) => EffectResult::IamPolicyLoaded {
                context,
                members: value,
            },
            Err(error) => EffectResult::IamPolicyLoadFailed {
                context,
                error: AppError::gcloud(error.to_string()),
            },
        };
    }
    let service = GcloudIamService::new(runner);
    match service.load_policy_members(&context) {
        Ok(members) => EffectResult::IamPolicyLoaded { context, members },
        Err(IamLoadError::Command(error)) => EffectResult::IamPolicyLoadFailed {
            context,
            error: AppError::gcloud(error.to_string()),
        },
        Err(IamLoadError::Parse(error)) => EffectResult::IamPolicyLoadFailed {
            context,
            error: AppError::Parse(error.to_string()),
        },
        // `get-iam-policy` names the project positionally, so a context with no
        // project cannot ask the question at all.
        Err(error @ IamLoadError::MissingProject) => EffectResult::IamPolicyLoadFailed {
            context,
            error: AppError::gcloud(error.to_string()),
        },
    }
}

fn load_cloud_sql_instances(
    runner: &(dyn CommandRunner + Send + Sync),
    sdk: Option<&crate::services::sdk::SdkClients>,
    context: GcloudContext,
    filter: Option<String>,
) -> EffectResult {
    // Label mode is honoured only by the server-side expression, which the
    // native clients do not carry, so such a read goes through gcloud.
    if let Some(clients) =
        crate::services::resources::list_filter::native_clients_for(sdk, filter.as_deref())
    {
        let project = context.project.clone().unwrap_or_default();
        let loaded = clients
            .cloudsql(context.configuration.as_str())
            .and_then(|client| crate::services::sdk::block_on(client.load_instances(&project)));
        return match loaded {
            Ok(value) => EffectResult::CloudSqlInstancesLoaded {
                context,
                instances: value,
            },
            Err(error) => EffectResult::CloudSqlInstancesLoadFailed {
                context,
                error: AppError::gcloud(error.to_string()),
            },
        };
    }
    let service = GcloudCloudSqlService::new(runner);
    match service.load_instances(&context, filter.as_deref()) {
        Ok(instances) => EffectResult::CloudSqlInstancesLoaded { context, instances },
        Err(crate::services::resources::cloudsql::CloudSqlLoadError::Command(error)) => {
            EffectResult::CloudSqlInstancesLoadFailed {
                context,
                error: AppError::gcloud(error.to_string()),
            }
        }
        Err(crate::services::resources::cloudsql::CloudSqlLoadError::Parse(error)) => {
            EffectResult::CloudSqlInstancesLoadFailed {
                context,
                error: AppError::Parse(error.to_string()),
            }
        }
    }
}

pub fn load_recent_logs(
    runner: &(dyn CommandRunner + Send + Sync),
    sdk: Option<&crate::services::sdk::SdkClients>,
    context: GcloudContext,
    query: LogQuery,
    severity: LogSeverity,
    range: LogTimeRange,
) -> EffectResult {
    if let Some(clients) = sdk {
        let project = context.project.clone().unwrap_or_default();
        let loaded = clients
            .logging(context.configuration.as_str())
            .and_then(|client| {
                crate::services::sdk::block_on(client.read_recent(
                    &project,
                    &query,
                    severity,
                    range,
                    LOG_PAGE_LIMIT,
                ))
            });
        return match loaded {
            Ok(outcome) => EffectResult::RecentLogsLoaded {
                context,
                query,
                severity,
                range,
                entries: outcome.entries,
                dropped: outcome.dropped,
            },
            Err(error) => EffectResult::RecentLogsLoadFailed {
                context,
                query,
                error: AppError::gcloud(error.to_string()),
            },
        };
    }
    let service = GcloudLoggingService::new(runner);
    match service.read_recent(&context, &query, severity, range, LOG_PAGE_LIMIT) {
        Ok(outcome) => EffectResult::RecentLogsLoaded {
            context,
            query,
            severity,
            range,
            entries: outcome.entries,
            dropped: outcome.dropped,
        },
        Err(LogLoadError::Command(error)) => EffectResult::RecentLogsLoadFailed {
            context,
            query,
            error: error.into(),
        },
        Err(LogLoadError::Parse(error)) => EffectResult::RecentLogsLoadFailed {
            context,
            query,
            error: AppError::Parse(error.to_string()),
        },
    }
}

pub fn tail_logs(
    runner: &(dyn CommandRunner + Send + Sync),
    sdk: Option<&crate::services::sdk::SdkClients>,
    context: GcloudContext,
    query: LogQuery,
    severity: LogSeverity,
    cursor: LogCursor,
) -> EffectResult {
    if let Some(clients) = sdk {
        let project = context.project.clone().unwrap_or_default();
        let loaded = clients
            .logging(context.configuration.as_str())
            .and_then(|client| {
                crate::services::sdk::block_on(client.read_since(
                    &project,
                    &query,
                    severity,
                    &cursor,
                    LOG_PAGE_LIMIT,
                ))
            });
        return match loaded {
            Ok(outcome) => EffectResult::LogsTailed {
                context,
                query,
                severity,
                entries: outcome.entries,
                dropped: outcome.dropped,
            },
            Err(error) => EffectResult::LogsTailFailed {
                context,
                query,
                error: AppError::gcloud(error.to_string()),
            },
        };
    }
    let service = GcloudLoggingService::new(runner);
    match service.read_since(&context, &query, severity, &cursor, LOG_PAGE_LIMIT) {
        Ok(outcome) => EffectResult::LogsTailed {
            context,
            query,
            severity,
            entries: outcome.entries,
            dropped: outcome.dropped,
        },
        Err(LogLoadError::Command(error)) => EffectResult::LogsTailFailed {
            context,
            query,
            error: error.into(),
        },
        Err(LogLoadError::Parse(error)) => EffectResult::LogsTailFailed {
            context,
            query,
            error: AppError::Parse(error.to_string()),
        },
    }
}

fn load_storage_buckets(
    runner: &(dyn CommandRunner + Send + Sync),
    sdk: Option<&crate::services::sdk::SdkClients>,
    context: GcloudContext,
) -> EffectResult {
    // The native path when there is one, which is every session the binary
    // builds -- demo included, where the cache is fixture-backed. `sdk` is
    // `None` only in the test harnesses that pass no cache, and those serve
    // fixtures through the runner below.
    if let Some(clients) = sdk {
        let project = context.project.clone().unwrap_or_default();
        let loaded = clients
            .storage(context.configuration.as_str())
            .and_then(|client| crate::services::sdk::block_on(client.load_buckets(&project)));
        return match loaded {
            Ok(buckets) => EffectResult::StorageBucketsLoaded { context, buckets },
            Err(error) => EffectResult::StorageBucketsLoadFailed {
                context,
                error: AppError::gcloud(error.to_string()),
            },
        };
    }
    let service = GcloudStorageService::new(runner);
    match service.load_buckets(&context) {
        Ok(buckets) => EffectResult::StorageBucketsLoaded { context, buckets },
        Err(StorageLoadError::Command(error)) => EffectResult::StorageBucketsLoadFailed {
            context,
            error: AppError::gcloud(error.to_string()),
        },
        Err(StorageLoadError::Parse(error)) => EffectResult::StorageBucketsLoadFailed {
            context,
            error: AppError::Parse(error.to_string()),
        },
    }
}

fn load_storage_object_page(
    runner: Option<&(dyn CommandRunner + Send + Sync)>,
    sdk: Option<&crate::services::sdk::SdkClients>,
    context: GcloudContext,
    bucket: String,
    prefix: String,
    page_token: Option<String>,
    page_size: u32,
) -> EffectResult {
    let append = page_token.is_some();
    let Some(runner) = runner else {
        return EffectResult::StorageObjectPageLoadFailed {
            context,
            bucket,
            prefix,
            append,
            error: AppError::gcloud("no gcloud command runner configured".to_string()),
        };
    };
    if let Some(clients) = sdk {
        let loaded = clients
            .storage(context.configuration.as_str())
            .and_then(|client| {
                crate::services::sdk::block_on(client.load_object_page(
                    &bucket,
                    &prefix,
                    page_token.as_deref(),
                    page_size,
                ))
            });
        return match loaded {
            Ok(page) => EffectResult::StorageObjectPageLoaded {
                context,
                bucket,
                prefix,
                append,
                page,
            },
            Err(error) => EffectResult::StorageObjectPageLoadFailed {
                context,
                bucket,
                prefix,
                append,
                error: AppError::gcloud(error.to_string()),
            },
        };
    }
    let service = GcloudStorageService::new(runner);
    match service.load_object_page(&context, &bucket, &prefix, page_token.as_deref(), page_size) {
        Ok(page) => EffectResult::StorageObjectPageLoaded {
            context,
            bucket,
            prefix,
            append,
            page,
        },
        Err(StorageLoadError::Command(error)) => EffectResult::StorageObjectPageLoadFailed {
            context,
            bucket,
            prefix,
            append,
            error: AppError::gcloud(error.to_string()),
        },
        Err(StorageLoadError::Parse(error)) => EffectResult::StorageObjectPageLoadFailed {
            context,
            bucket,
            prefix,
            append,
            error: AppError::Parse(error.to_string()),
        },
    }
}

/// Produces the appropriate load-failure result for a background refresh when
/// no gcloud runner is configured, keyed to the active service so storage
/// failures are not misreported as compute failures.
fn background_refresh_runner_missing(
    service: ResourceServiceKind,
    context: GcloudContext,
) -> EffectResult {
    let error = AppError::gcloud("no gcloud command runner configured".to_string());
    match service {
        ResourceServiceKind::ComputeInstances => {
            EffectResult::ComputeInstancesLoadFailed { context, error }
        }
        ResourceServiceKind::GkeClusters => EffectResult::GkeClustersLoadFailed { context, error },
        ResourceServiceKind::CloudSql => {
            EffectResult::CloudSqlInstancesLoadFailed { context, error }
        }
        ResourceServiceKind::CloudRunServices => {
            EffectResult::CloudRunServicesLoadFailed { context, error }
        }
        ResourceServiceKind::CloudStorage => {
            EffectResult::StorageBucketsLoadFailed { context, error }
        }
        ResourceServiceKind::SecretManagerSecrets => {
            EffectResult::SecretsLoadFailed { context, error }
        }
        ResourceServiceKind::CloudFunctions => {
            EffectResult::CloudFunctionsLoadFailed { context, error }
        }
        ResourceServiceKind::PersistentDisks => {
            EffectResult::PersistentDisksLoadFailed { context, error }
        }
        ResourceServiceKind::PubSubTopics => {
            EffectResult::PubSubTopicsLoadFailed { context, error }
        }
        ResourceServiceKind::VpcNetworks => EffectResult::VpcNetworksLoadFailed { context, error },
        ResourceServiceKind::VpcSubnets => EffectResult::VpcSubnetsLoadFailed { context, error },
        ResourceServiceKind::FirewallRules => {
            EffectResult::FirewallRulesLoadFailed { context, error }
        }
        ResourceServiceKind::PubSubSubscriptions => {
            EffectResult::PubSubSubscriptionsLoadFailed { context, error }
        }
        ResourceServiceKind::IamServiceAccounts => {
            EffectResult::ServiceAccountsLoadFailed { context, error }
        }
        ResourceServiceKind::IamPolicy => EffectResult::IamPolicyLoadFailed { context, error },
        ResourceServiceKind::BigQueryDatasets => {
            EffectResult::BigQueryDatasetsLoadFailed { context, error }
        }
    }
}

/// Produces the appropriate load-failure result when a background refresh task
/// fails to join, keyed to the active service.
fn background_refresh_join_failed(
    service: ResourceServiceKind,
    context: GcloudContext,
    error: tokio::task::JoinError,
) -> EffectResult {
    let error = AppError::gcloud(format!("background refresh task failed: {error}"));
    match service {
        ResourceServiceKind::ComputeInstances => {
            EffectResult::ComputeInstancesLoadFailed { context, error }
        }
        ResourceServiceKind::GkeClusters => EffectResult::GkeClustersLoadFailed { context, error },
        ResourceServiceKind::CloudSql => {
            EffectResult::CloudSqlInstancesLoadFailed { context, error }
        }
        ResourceServiceKind::CloudRunServices => {
            EffectResult::CloudRunServicesLoadFailed { context, error }
        }
        ResourceServiceKind::CloudStorage => {
            EffectResult::StorageBucketsLoadFailed { context, error }
        }
        ResourceServiceKind::SecretManagerSecrets => {
            EffectResult::SecretsLoadFailed { context, error }
        }
        ResourceServiceKind::CloudFunctions => {
            EffectResult::CloudFunctionsLoadFailed { context, error }
        }
        ResourceServiceKind::PersistentDisks => {
            EffectResult::PersistentDisksLoadFailed { context, error }
        }
        ResourceServiceKind::PubSubTopics => {
            EffectResult::PubSubTopicsLoadFailed { context, error }
        }
        ResourceServiceKind::VpcNetworks => EffectResult::VpcNetworksLoadFailed { context, error },
        ResourceServiceKind::VpcSubnets => EffectResult::VpcSubnetsLoadFailed { context, error },
        ResourceServiceKind::FirewallRules => {
            EffectResult::FirewallRulesLoadFailed { context, error }
        }
        ResourceServiceKind::PubSubSubscriptions => {
            EffectResult::PubSubSubscriptionsLoadFailed { context, error }
        }
        ResourceServiceKind::IamServiceAccounts => {
            EffectResult::ServiceAccountsLoadFailed { context, error }
        }
        ResourceServiceKind::IamPolicy => EffectResult::IamPolicyLoadFailed { context, error },
        ResourceServiceKind::BigQueryDatasets => {
            EffectResult::BigQueryDatasetsLoadFailed { context, error }
        }
    }
}

fn context_update_fallback_configuration(
    configuration_name: GcloudConfigurationName,
    known_configuration: Option<GcloudConfiguration>,
    applied: GcloudContextAppliedProperties,
) -> GcloudConfiguration {
    let mut configuration = known_configuration.unwrap_or_else(|| GcloudConfiguration {
        name: configuration_name.clone(),
        is_active: true,
        account: None,
        project: None,
        region: None,
        zone: None,
    });
    configuration.name = configuration_name;
    configuration.is_active = true;
    if let Some(project) = applied.project {
        configuration.project = project;
    }
    if let Some(region) = applied.region {
        configuration.region = region;
    }
    if let Some(zone) = applied.zone {
        configuration.zone = zone;
    }
    configuration
}

fn active_configuration_shell(name: String) -> GcloudConfiguration {
    GcloudConfiguration {
        name: GcloudConfigurationName::new(name),
        is_active: true,
        account: None,
        project: None,
        region: None,
        zone: None,
    }
}

/// Whether the poll ticker should auto-retry the active scope, given the
/// `Retryability` of its last failure (`None` when it has never failed, or
/// its last result was a success). `Persistent` failures (auth/permission/
/// config/parse errors) are not auto-retried — hammering gcloud will not fix
/// them. `Transient` failures (network/quota/unknown) retry on the next tick
/// exactly as before this gate existed.
fn should_auto_retry(last: Option<nuvem_domain::gcp::diagnostics::Retryability>) -> bool {
    !matches!(
        last,
        Some(nuvem_domain::gcp::diagnostics::Retryability::Persistent)
    )
}

#[cfg(test)]
mod tests {
    use super::{
        BIGQUERY_ENRICHMENT_CONCURRENCY, BatchTargetClaim, RuntimeSession, claim_batch_target_slot,
        claim_bigquery_enrichment_slot, should_auto_retry,
    };
    use crate::app::state::remembered_view::RememberedViews;
    use crate::app::{Action, AppState, ConfigurationsAction, Effect, EffectResult};
    use crate::services::runtime::remembered_view_store::RememberedViewStore;
    use crate::{Cli, Config};
    use clap::Parser;
    use nuvem_domain::gcp::configuration::{
        GcloudConfiguration, GcloudConfigurationName, GcloudContext,
    };
    use nuvem_domain::gcp::diagnostics::Retryability;
    use nuvem_domain::gcp::resource::ResourceServiceKind;
    use std::future::Future;
    use std::pin::pin;
    use std::sync::Arc;
    use std::task::{Context, Poll, Waker};
    use std::time::Duration;
    use tokio::sync::Semaphore;
    use tokio_util::sync::CancellationToken;

    /// A config reload re-resolves from the stored [`Cli`], so `read_only`
    /// must travel the same live-apply path `aliases`/`jump_rules` already
    /// do (see `reload_config`), landing in `RootState.read_only` without a
    /// restart.
    #[test]
    fn reload_config_carries_read_only_into_root_state() {
        let mut file = tempfile::NamedTempFile::new().unwrap();
        std::io::Write::write_all(&mut file, b"version = 1\n[general]\nread_only = true\n")
            .unwrap();
        let cli = Cli::parse_from(["nuvem", "--config", file.path().to_str().unwrap()]);

        let mut session = RuntimeSession::new_with_cli_and_config_for_test(
            cli,
            Config::default(),
            AppState::new(),
        );
        assert!(!session.app.root.read_only, "starts false in this fixture");

        session.reload_config();

        assert!(session.app.root.read_only);
    }

    /// A session whose `--plugins` file holds one entry per line of `ids`,
    /// started the way a real run starts: resolve the config from the `Cli`,
    /// then `apply_startup_config` to install the set. The config file is a
    /// temp file too, so the reload re-resolves against the fixture and never
    /// against the developer's own `~/.config/nuvem`.
    ///
    /// Both temp files are returned: dropping a `NamedTempFile` deletes it,
    /// and a plugin file deleted before the reload would resolve to the empty
    /// set for the wrong reason.
    fn session_with_plugins(
        ids: &[&str],
    ) -> (
        RuntimeSession,
        tempfile::NamedTempFile,
        tempfile::NamedTempFile,
    ) {
        let mut config_file = tempfile::NamedTempFile::new().unwrap();
        std::io::Write::write_all(&mut config_file, b"version = 1\n").unwrap();
        let plugin_file = write_plugin_file(ids);

        let cli = Cli::parse_from([
            "nuvem",
            "--config",
            config_file.path().to_str().unwrap(),
            "--plugins",
            plugin_file.path().to_str().unwrap(),
        ]);
        let outcome = Config::resolve(&cli);
        assert!(
            outcome.problems.is_empty(),
            "fixture must resolve cleanly: {:?}",
            outcome.problems.messages()
        );
        let mut app = AppState::new();
        crate::services::runtime::apply_startup_config(&mut app, &outcome.config);

        let session = RuntimeSession::new_with_cli_and_config_for_test(cli, outcome.config, app);
        (session, config_file, plugin_file)
    }

    /// One valid `[[plugins]]` entry per id, each with its own key: two
    /// entries sharing a key within a scope would leave the fixture quietly
    /// one plugin short.
    fn write_plugin_file(ids: &[&str]) -> tempfile::NamedTempFile {
        let mut body = "version = 1\n".to_string();
        for (index, id) in ids.iter().enumerate() {
            let key = char::from(b'a' + u8::try_from(index).expect("a small fixture"));
            body.push_str(&format!(
                "\n[[plugins]]\nid = {id:?}\nscope = \"compute\"\nkey = {key:?}\n\
                 label = \"Describe {{name}}\"\nargs = [\"compute\", \"instances\", \
                 \"describe\", \"{{name}}\"]\n"
            ));
        }
        let mut file = tempfile::NamedTempFile::new().unwrap();
        std::io::Write::write_all(&mut file, body.as_bytes()).unwrap();
        file
    }

    /// Editing the plugin file and reloading must say that the change needs a
    /// restart. Without this the warning from the old file disappears (the
    /// problems list is replaced from the fresh resolve) while the running
    /// plugin set is unchanged, so the operator is told the opposite of what
    /// happened.
    #[test]
    fn reload_reports_a_changed_plugin_file_as_restart_required() {
        let (mut session, _config_file, plugin_file) = session_with_plugins(&["a"]);
        assert_eq!(
            session.app.root.plugins.len(),
            1,
            "starts with the file's plugin"
        );

        let rewritten = write_plugin_file(&["a", "b"]);
        std::fs::copy(rewritten.path(), plugin_file.path()).unwrap();
        session.reload_config();

        assert_eq!(
            session.app.root.plugins.len(),
            1,
            "the running set is deliberately not swapped"
        );
        assert_eq!(
            session.config.as_ref().unwrap().plugins.len(),
            1,
            "and the stored config has to agree, or `nuvem info` reports a set \
             the session is not actually offering"
        );
        let message = session
            .app
            .root
            .system
            .status_message
            .clone()
            .expect("reload reports");
        assert!(
            message.contains("restart required for: plugins"),
            "reload said {message:?}"
        );
    }

    /// And an unchanged file must not claim a restart is needed, or the
    /// message becomes noise every operator learns to ignore — which costs the
    /// notice its meaning on the reload where it is true.
    #[test]
    fn reload_is_silent_about_plugins_when_the_file_did_not_change() {
        let (mut session, _config_file, _plugin_file) = session_with_plugins(&["a"]);

        session.reload_config();

        let message = session
            .app
            .root
            .system
            .status_message
            .clone()
            .expect("reload reports");
        assert!(!message.contains("plugins"), "reload said {message:?}");
    }

    /// Read-only is sticky for the life of the process. There is deliberately
    /// no per-session toggle, and "edit the config file, then run the
    /// palette's Reload configuration" would be one — the reload is itself a
    /// permitted command under the mode, so an unconditional fold would let
    /// the guard be switched off live from inside the guarded session.
    #[test]
    fn reload_config_cannot_turn_read_only_off() {
        let mut file = tempfile::NamedTempFile::new().unwrap();
        std::io::Write::write_all(&mut file, b"version = 1\n[general]\nread_only = false\n")
            .unwrap();
        let cli = Cli::parse_from(["nuvem", "--config", file.path().to_str().unwrap()]);

        let mut session = RuntimeSession::new_with_cli_and_config_for_test(
            cli,
            Config {
                read_only: true,
                ..Config::default()
            },
            AppState::new(),
        );
        session.app.root.read_only = true;

        session.reload_config();

        assert!(
            session.app.root.read_only,
            "a reload must never clear read-only: the fold is monotonic, and \
             RootState.read_only documents that it can only be turned on"
        );
        assert!(
            session.config.as_ref().unwrap().read_only,
            "the stored config has to agree with the live guard, or `nuvem info` \
             reports a mode the app is not actually in"
        );
    }

    /// The health history is filled from the funnel, not from the thirteen
    /// list-result arms, and this pins that the funnel actually samples.
    /// A funnel that reduced but never sampled would leave every
    /// sparkline empty forever, and an empty sparkline is indistinguishable
    /// from a service that has only just been listed.
    #[test]
    fn the_action_funnel_samples_a_landed_listing_into_the_health_history() {
        let mut session =
            RuntimeSession::new_with_config_for_test(Config::default(), AppState::new());
        session.reduce(&Action::EffectResult(EffectResult::ConfigurationsLoaded {
            configurations: vec![GcloudConfiguration {
                name: GcloudConfigurationName::new("dev"),
                is_active: true,
                account: None,
                project: Some("dev-project".to_string()),
                region: None,
                zone: None,
            }],
        }));
        let context = session
            .app
            .root
            .current_context()
            .expect("the loaded configuration is active");

        session.reduce(&Action::EffectResult(
            EffectResult::ComputeInstancesLoaded {
                context: context.clone(),
                instances: Vec::new(),
            },
        ));

        assert_eq!(
            session
                .app
                .root
                .health
                .series_in(ResourceServiceKind::ComputeInstances, &context),
            vec![0],
            "the landed listing must have become a sample"
        );
    }

    /// The action funnel does two things, and both have to hold: the pure
    /// reducer's state change lands on the session's own `AppState`, and the
    /// effects that reducer produced are handed back for dispatch.
    ///
    /// Pinned in-crate, and deliberately not only through the runtime tests.
    /// A funnel that applied nothing and returned no effects leaves the
    /// session inert, and the integration suites notice that by parking
    /// forever on an effect result the runtime will now never produce —
    /// a hang, not a failure. Here it is a fast assertion instead.
    #[test]
    fn reduce_applies_the_state_change_and_returns_the_reducers_effects() {
        let mut session =
            RuntimeSession::new_with_config_for_test(Config::default(), AppState::new());

        session.reduce(&Action::Configurations(ConfigurationsAction::StartCreate));
        assert!(
            session.app.root.configurations.is_create_form_open(),
            "the funnel must apply the reducer's state change to its own AppState"
        );

        session.reduce(&Action::Configurations(
            ConfigurationsAction::AppendCreateChar('p'),
        ));
        let effects = session.reduce(&Action::Configurations(ConfigurationsAction::SubmitCreate));

        assert!(
            matches!(effects.as_slice(), [Effect::CreateConfiguration { .. }]),
            "the funnel must hand back the reducer's effects, got {effects:?}"
        );
    }

    #[test]
    fn poll_gate_skips_persistent_failures() {
        assert!(!should_auto_retry(Some(Retryability::Persistent)));
        assert!(should_auto_retry(Some(Retryability::Transient)));
        assert!(should_auto_retry(None));
    }

    /// The worker and the cancellation both reach for the same claim from
    /// different threads, so "did this target issue its command?" must be
    /// decided once, by whoever gets there first — not observed twice. Here
    /// cancellation wins: it reports `Cancelled` ("the command did not run"),
    /// so the worker must be refused and issue nothing. A plain load/store pair
    /// cannot refuse it — the worker would go on to run gcloud against an
    /// instance the log has already sworn was left alone.
    #[test]
    fn a_target_claimed_by_cancellation_is_refused_the_command() {
        let claim = BatchTargetClaim::new();

        assert!(
            claim.claim_cancel(),
            "cancellation reaching an untouched target reports Cancelled"
        );
        assert!(
            !claim.claim_start(),
            "the worker must not issue a command for a target already reported Cancelled"
        );
    }

    /// The other order, and the other half of the exclusion: once the worker
    /// holds the claim the command is on its way to GCP, so cancellation may no
    /// longer call it `Cancelled`. It loses the claim and reports `Unknown`.
    #[test]
    fn a_target_claimed_by_the_worker_is_never_reported_cancelled() {
        let claim = BatchTargetClaim::new();

        assert!(claim.claim_start(), "an untouched target starts");
        assert!(
            !claim.claim_cancel(),
            "a started target must be reported Unknown, not Cancelled"
        );
    }

    /// Neither side may claim twice: a second claim is not a fresh decision.
    #[test]
    fn a_claim_is_decided_once() {
        let started = BatchTargetClaim::new();
        assert!(started.claim_start());
        assert!(!started.claim_start());

        let cancelled = BatchTargetClaim::new();
        assert!(cancelled.claim_cancel());
        assert!(!cancelled.claim_cancel());
    }

    /// The one window the batch tests cannot reach: a target that has acquired
    /// its permit but not yet passed the post-permit cancellation check. It must
    /// claim nothing, so cancellation can still report `Cancelled` for a target
    /// that provably issued no command — moving the claim up to just after
    /// `acquire_owned()` would over-claim it as `Unknown` instead.
    ///
    /// Driven by hand rather than through the effect loop on purpose. The
    /// distinguishing state spans no await point, so from outside the task the
    /// interleaving is unreachable; here the first `poll` is what proves the
    /// target is parked on the permit and past the first cancellation check,
    /// and only then does the cancellation land.
    #[tokio::test]
    async fn a_batch_target_cancelled_while_waiting_for_a_permit_never_claims() {
        let semaphore = Arc::new(Semaphore::new(1));
        let held = semaphore.clone().acquire_owned().await.unwrap();
        let cancellation = CancellationToken::new();
        let claim = BatchTargetClaim::new();

        let mut slot = pin!(claim_batch_target_slot(
            cancellation.clone(),
            semaphore.clone(),
            &claim
        ));
        assert!(
            slot.as_mut()
                .poll(&mut Context::from_waker(Waker::noop()))
                .is_pending(),
            "the target must be parked on the permit, past the first cancellation check"
        );

        cancellation.cancel();
        drop(held);

        assert!(
            slot.await.is_none(),
            "a cancelled batch must issue no command for this target"
        );
        assert!(
            claim.claim_cancel(),
            "the target issued nothing, so cancellation must still report Cancelled"
        );
        assert_eq!(
            semaphore.available_permits(),
            1,
            "the refused target must not strand the permit it acquired"
        );
    }

    /// The other side of the same gate: a target that has not acquired anything
    /// yet bails before touching the semaphore, so a detached batch cannot make
    /// its still-queued targets wait on permits nobody will use.
    #[tokio::test]
    async fn a_batch_target_cancelled_before_it_starts_takes_no_permit() {
        let semaphore = Arc::new(Semaphore::new(1));
        let held = semaphore.clone().acquire_owned().await.unwrap();
        let cancellation = CancellationToken::new();
        cancellation.cancel();
        let claim = BatchTargetClaim::new();

        let mut slot = pin!(claim_batch_target_slot(
            cancellation,
            semaphore.clone(),
            &claim
        ));
        assert!(
            matches!(
                slot.as_mut().poll(&mut Context::from_waker(Waker::noop())),
                Poll::Ready(None)
            ),
            "an already-cancelled target must bail without waiting for a permit"
        );

        drop(held);
        assert_eq!(semaphore.available_permits(), 1);
        assert!(
            claim.claim_cancel(),
            "a target that never started reports Cancelled"
        );
    }

    /// The path that runs a command: the claim is taken, and the permit comes
    /// back with it so the concurrency limit still covers the command itself.
    #[tokio::test]
    async fn an_uncancelled_batch_target_claims_and_holds_its_permit() {
        let semaphore = Arc::new(Semaphore::new(1));
        let claim = BatchTargetClaim::new();

        let permit =
            claim_batch_target_slot(CancellationToken::new(), semaphore.clone(), &claim).await;

        assert!(permit.is_some(), "an uncancelled target issues its command");
        assert_eq!(
            semaphore.available_permits(),
            0,
            "the permit is held for as long as the command runs"
        );
        assert!(
            !claim.claim_cancel(),
            "a claimed target must be reported Unknown, not Cancelled"
        );

        drop(permit);
        assert_eq!(semaphore.available_permits(), 1);
    }

    /// Losing the claim is the stricter bail: the cancellation landed in the
    /// instant since the check above it, its entry already reads `Cancelled`,
    /// and issuing the command would make that a lie. The permit goes back
    /// either way.
    #[tokio::test]
    async fn a_batch_target_claimed_by_cancellation_is_refused_its_permit() {
        let semaphore = Arc::new(Semaphore::new(1));
        let claim = BatchTargetClaim::new();
        assert!(claim.claim_cancel());

        assert!(
            claim_batch_target_slot(CancellationToken::new(), semaphore.clone(), &claim)
                .await
                .is_none(),
            "a target already reported Cancelled issues nothing"
        );
        assert_eq!(semaphore.available_permits(), 1);
    }

    /// A closed semaphore is the runtime shutting the batch down; the target
    /// resolves to "issued nothing" rather than unwrapping an error.
    #[tokio::test]
    async fn a_batch_target_on_a_closed_semaphore_never_claims() {
        let semaphore = Arc::new(Semaphore::new(1));
        semaphore.close();
        let claim = BatchTargetClaim::new();

        assert!(
            claim_batch_target_slot(CancellationToken::new(), semaphore, &claim)
                .await
                .is_none()
        );
        assert!(
            claim.claim_cancel(),
            "nothing was issued, so the entry may still read Cancelled"
        );
    }

    /// The fan-out's own slot claim. A show still queued for a listing that
    /// has been replaced must not spend a permit: the permits are what the
    /// live drill's shows are waiting for.
    #[tokio::test]
    async fn a_superseded_enrichment_waiting_for_a_permit_never_takes_one() {
        let semaphore = Arc::new(Semaphore::new(1));
        let held = semaphore.clone().acquire_owned().await.unwrap();
        let cancellation = CancellationToken::new();
        cancellation.cancel();

        let mut slot = pin!(claim_bigquery_enrichment_slot(
            &cancellation,
            semaphore.clone()
        ));
        assert!(
            matches!(
                slot.as_mut().poll(&mut Context::from_waker(Waker::noop())),
                Poll::Ready(None)
            ),
            "an already-superseded show must bail without waiting for a permit"
        );

        drop(held);
        assert_eq!(semaphore.available_permits(), 1);
    }

    /// The check on the far side of the permit, which is the one the queue
    /// needs: every show of an abandoned wide dataset was parked *inside*
    /// `acquire_owned` when the operator left, and each wakes up holding a
    /// permit the live drill wants.
    #[tokio::test]
    async fn an_enrichment_superseded_while_it_waited_gives_the_permit_back() {
        let semaphore = Arc::new(Semaphore::new(1));
        let held = semaphore.clone().acquire_owned().await.unwrap();
        let cancellation = CancellationToken::new();

        let mut slot = pin!(claim_bigquery_enrichment_slot(
            &cancellation,
            semaphore.clone()
        ));
        assert!(
            slot.as_mut()
                .poll(&mut Context::from_waker(Waker::noop()))
                .is_pending(),
            "it is waiting for the permit, which is the state this is about"
        );

        cancellation.cancel();
        drop(held);

        assert!(
            slot.await.is_none(),
            "the listing it was fanned out from is gone, so it issues nothing"
        );
        assert_eq!(
            semaphore.available_permits(),
            1,
            "and the permit goes back to the fan-out that is still wanted"
        );
    }

    /// An uncancelled show takes its permit and holds it for the call.
    #[tokio::test]
    async fn a_live_enrichment_claims_and_holds_its_permit() {
        let semaphore = Arc::new(Semaphore::new(1));
        let cancellation = CancellationToken::new();

        let permit = claim_bigquery_enrichment_slot(&cancellation, semaphore.clone()).await;

        assert!(permit.is_some(), "a live show issues its command");
        assert_eq!(semaphore.available_permits(), 0);
        drop(permit);
        assert_eq!(semaphore.available_permits(), 1);
    }

    /// The wiring: dispatching a tables listing is what supersedes the fan-out
    /// of the listing it replaces. Without it the token never flips and the
    /// checks above are unreachable in the running app.
    #[tokio::test]
    async fn dispatching_a_tables_listing_supersedes_the_previous_fan_out() {
        let context = GcloudContext {
            configuration: GcloudConfigurationName::new("dev"),
            account: None,
            project: Some("dev-project".to_string()),
            region: None,
            zone: None,
        };
        let mut app = AppState::new();
        app.root
            .resources
            .select_service(ResourceServiceKind::BigQueryDatasets);
        app.root.resources.set_bigquery_datasets(
            context.clone(),
            vec![nuvem_domain::gcp::resource::BigQueryDataset {
                dataset_id: "austin_bikeshare".to_string(),
                project_id: "dev-project".to_string(),
                location: "US".to_string(),
                friendly_name: None,
                labels: Vec::new(),
            }],
        );
        let request = app
            .root
            .resources
            .open_selected_bigquery_dataset()
            .expect("the dataset is selectable");
        let listing = Effect::LoadBigQueryTables {
            context,
            dataset_id: request.dataset_id,
            generation: request.generation,
            listing: request.listing,
        };

        let mut session = RuntimeSession::new_for_test(app);
        let superseded = session.bigquery_enrichment_cancellation.clone();

        session.dispatch_effects(vec![listing], false).await;

        assert!(
            superseded.is_cancelled(),
            "the shows queued for the listing this one replaces must stop \
             waiting for permits"
        );
        assert!(
            !session.bigquery_enrichment_cancellation.is_cancelled(),
            "and the fan-out this listing will produce must not be born \
             cancelled"
        );
    }

    /// A store that records what it was asked to save and can be told to fail.
    #[derive(Default)]
    struct FakeRememberedViewStore {
        saved: std::sync::Mutex<Vec<RememberedViews>>,
        fail: bool,
    }

    impl RememberedViewStore for FakeRememberedViewStore {
        fn save(&self, views: &RememberedViews) -> Result<(), String> {
            self.saved.lock().unwrap().push(views.clone());
            if self.fail {
                Err("permission denied".to_string())
            } else {
                Ok(())
            }
        }
    }

    /// A store that blocks inside `save` until the test releases it, and
    /// announces when it got there. The two channels are what make the
    /// run-loop test deterministic rather than a race against a sleep: the
    /// test can prove the write is *still in flight* at the moment it checks
    /// whether dispatch returned.
    struct GatedRememberedViewStore {
        entered: std::sync::mpsc::Sender<()>,
        release: std::sync::Mutex<std::sync::mpsc::Receiver<()>>,
    }

    impl RememberedViewStore for GatedRememberedViewStore {
        fn save(&self, _views: &RememberedViews) -> Result<(), String> {
            let _ = self.entered.send(());
            let _ = self.release.lock().unwrap().recv();
            Ok(())
        }
    }

    /// A store that makes the *first* save slow and records the order saves
    /// actually reached the disk in. A writer that ran saves concurrently
    /// would let the second overtake the first here.
    #[derive(Default)]
    struct OrderRecordingRememberedViewStore {
        saved: std::sync::Mutex<Vec<RememberedViews>>,
        calls: std::sync::atomic::AtomicUsize,
    }

    impl RememberedViewStore for OrderRecordingRememberedViewStore {
        fn save(&self, views: &RememberedViews) -> Result<(), String> {
            if self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0 {
                std::thread::sleep(Duration::from_millis(300));
            }
            self.saved.lock().unwrap().push(views.clone());
            Ok(())
        }
    }

    fn views_for(service: ResourceServiceKind) -> RememberedViews {
        let mut views = RememberedViews::default();
        views.set(GcloudConfigurationName::new("dev"), service);
        views
    }

    /// The point of this writer. `config::state::save` creates a temp file, fsyncs
    /// it, renames it and fsyncs the directory — and this effect fires on
    /// ordinary navigation, not on a command the user asked for by name. So
    /// dispatching it must hand the write to a background writer and return,
    /// leaving the run loop free to draw the frame the navigation earned.
    ///
    /// The dispatch runs in its own task so that an implementation which
    /// blocks can be observed as a timeout rather than hanging the test: a
    /// blocking `save` parks the worker thread it is polled on, and a
    /// `timeout` wrapped around it in *this* task could never be polled again.
    #[tokio::test(flavor = "multi_thread")]
    async fn dispatching_a_remembered_view_save_does_not_hold_the_run_loop() {
        let (entered_tx, entered_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let store = Arc::new(GatedRememberedViewStore {
            entered: entered_tx,
            release: std::sync::Mutex::new(release_rx),
        });
        let mut session = RuntimeSession::new_with_remembered_view_store_for_test(
            Config::default(),
            AppState::new(),
            store,
        );

        let mut dispatch = tokio::spawn(async move {
            session
                .dispatch_effects(
                    vec![Effect::SaveRememberedViews {
                        views: views_for(ResourceServiceKind::CloudSql),
                    }],
                    false,
                )
                .await;
            session
        });

        // The write reached the store and is parked there, so whatever the
        // dispatch task did, it did not do it by finishing the write.
        assert!(
            entered_rx.recv_timeout(Duration::from_secs(10)).is_ok(),
            "the save must still reach the store"
        );
        let settled = tokio::time::timeout(Duration::from_secs(2), &mut dispatch).await;
        // Released before asserting, so a red run unblocks the parked thread
        // instead of leaving it wedged for the rest of the binary.
        let _ = release_tx.send(());

        let joined = settled.expect(
            "dispatching the save must return while the write is still in \
             flight; it waited for the fsync on the run loop instead",
        );
        joined.expect("the dispatch task itself must not panic");
    }

    /// The constraint the ticket puts on any fix. Sequential dispatch is what
    /// currently guarantees write ordering, and a `tokio::spawn` per save
    /// would drop it: two navigations in quick succession would race, and the
    /// older map could be published over the newer one. Each save carries the
    /// *whole* map, so an out-of-order landing is a lost update, not a merge.
    ///
    /// The two dispatches here are separate calls because that is how
    /// navigation produces them — one per reduced action, not one batch.
    #[tokio::test(flavor = "multi_thread")]
    async fn remembered_view_saves_land_in_dispatch_order() {
        let store = Arc::new(OrderRecordingRememberedViewStore::default());
        let mut session = RuntimeSession::new_with_remembered_view_store_for_test(
            Config::default(),
            AppState::new(),
            store.clone(),
        );

        let first = views_for(ResourceServiceKind::CloudSql);
        let second = views_for(ResourceServiceKind::GkeClusters);
        session
            .dispatch_effects(
                vec![Effect::SaveRememberedViews {
                    views: first.clone(),
                }],
                false,
            )
            .await;
        session
            .dispatch_effects(
                vec![Effect::SaveRememberedViews {
                    views: second.clone(),
                }],
                false,
            )
            .await;
        session.flush_remembered_view_saves().await;

        assert_eq!(
            store.saved.lock().unwrap().as_slice(),
            &[first, second],
            "the newer map must reach the disk last, so the last write still wins"
        );
    }

    /// Eager saving was chosen for crash durability, so moving the write
    /// off the run loop must not turn "saved" into "saved unless you quit".
    /// Quit is the app's own exit, and it already blocks on a bounded tunnel
    /// shutdown; the pending save waits in the same place.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_pending_remembered_view_save_is_flushed_before_quit_returns() {
        let store = Arc::new(OrderRecordingRememberedViewStore::default());
        let mut session = RuntimeSession::new_with_remembered_view_store_for_test(
            Config::default(),
            AppState::new(),
            store.clone(),
        );

        let views = views_for(ResourceServiceKind::CloudSql);
        session
            .dispatch_effects(
                vec![Effect::SaveRememberedViews {
                    views: views.clone(),
                }],
                false,
            )
            .await;

        assert!(
            !session.dispatch_action(Action::QuitRequested).await,
            "QuitRequested ends the run loop"
        );

        assert_eq!(
            store.saved.lock().unwrap().as_slice(),
            &[views],
            "a save still in flight when the user quits must land before the \
             process exits, or eager saving silently loses the last navigation"
        );
    }

    /// The production wiring, not a fake, and composed the way [`super::run`]
    /// composes it: the path the startup load derived is what the session
    /// writes back to, so a `--config <path>` run keeps its state beside that
    /// file. Nothing else pins this — deriving from `default_state_path()` at
    /// either end would compile, pass every other test, and quietly write into
    /// whatever `dirs::config_dir()` resolves to on the host platform
    /// (`~/Library/Application Support/nuvem` on macOS, `~/.config/nuvem` on
    /// Linux) instead of beside the config file in use.
    #[tokio::test]
    async fn the_production_store_writes_beside_the_config_file_in_use() {
        let dir = tempfile::tempdir().expect("temp dir");
        let config_path = dir.path().join("config.toml");
        let config = Config {
            config_path: config_path.clone(),
            ..Config::default()
        };
        let startup = crate::services::runtime::load_remembered_views(
            &config,
            &mut crate::config::problems::ConfigProblems::default(),
        );
        let mut session = RuntimeSession::new(
            config,
            Cli::parse_from(["nuvem", "--config", config_path.to_str().unwrap()]),
            AppState::new(),
            startup.state_path,
        );

        let views = views_for(ResourceServiceKind::CloudSql);
        session
            .dispatch_effects(
                vec![Effect::SaveRememberedViews {
                    views: views.clone(),
                }],
                false,
            )
            .await;
        session.flush_remembered_view_saves().await;

        let reloaded: Option<RememberedViews> =
            crate::config::state::load(&dir.path().join("state.toml")).expect("load");
        assert_eq!(
            reloaded,
            Some(views),
            "state must land beside the config file in use, not beside the \
             default config this run never read"
        );
    }

    /// A stand-in binary that names itself when it runs, so a test can tell
    /// *which* binary a runner spawned rather than only that one did.
    #[cfg(unix)]
    fn named_echo_binary(directory: &std::path::Path, name: &str) -> std::path::PathBuf {
        use std::os::unix::fs::PermissionsExt;

        let path = directory.join(name);
        std::fs::write(&path, format!("#!/bin/sh\nprintf 'ran {name}'\n"))
            .expect("write named echo executable");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
            .expect("mark named echo executable");
        path
    }

    /// `--bq-binary`'s entire effect is one line of `RuntimeSession::new`.
    /// Building the runner with `ProcessCommandRunner::new(gcloud_binary)`
    /// instead still compiles and still passes every config test — the flag
    /// would resolve into `Config` exactly as asserted there and then be
    /// dropped on the floor, leaving `bq` to a bare `PATH` lookup. So this
    /// runs a `bq` command on the session's own runner and reads back which
    /// binary was spawned.
    #[cfg(unix)]
    #[tokio::test]
    async fn the_session_runner_spawns_the_configured_bq_binary() {
        let dir = tempfile::tempdir().expect("temp dir");
        let config_path = dir.path().join("config.toml");
        let config = Config {
            config_path: config_path.clone(),
            bq_binary: named_echo_binary(dir.path(), "fake-bq"),
            ..Config::default()
        };
        let session = RuntimeSession::new(
            config,
            Cli::parse_from(["nuvem", "--config", config_path.to_str().unwrap()]),
            AppState::new(),
            dir.path().join("state.toml"),
        );

        let runner = session
            .gcloud_runner
            .as_ref()
            .expect("a production session always wires a runner");
        let output = runner
            .run(crate::services::gcloud::cli::GcloudCommand::bq(["ls"]))
            .expect("the configured bq stand-in must be spawnable");

        assert_eq!(
            output.stdout, "ran fake-bq",
            "the session must hand the configured bq path to its runner, not \
             leave bq to whatever `PATH` resolves"
        );
    }

    /// The demo fixtures name their configurations `dev` and `prod`, which are
    /// ordinary real gcloud configuration names, and `--demo` leaves
    /// `config_path` pointing at the user's real config. A demo run must
    /// therefore touch the real state file from neither end: persisting would
    /// write invented entries over the views a real session learned, and
    /// loading would open the demo on whatever service the user last had open
    /// for real. Both halves are asserted here because they are one property.
    #[tokio::test]
    async fn a_demo_session_neither_reads_nor_writes_the_state_file() {
        let dir = tempfile::tempdir().expect("temp dir");
        let config_path = dir.path().join("config.toml");
        let state_path = crate::config::state::state_path_for(&config_path);
        // A real previous run's state, sitting where a demo would find it.
        let mut previously_remembered = RememberedViews::default();
        previously_remembered.set(
            GcloudConfigurationName::new("dev"),
            ResourceServiceKind::GkeClusters,
        );
        crate::config::state::save(&state_path, &previously_remembered).expect("seed real state");
        let written_by_the_real_run =
            std::fs::read_to_string(&state_path).expect("read the seeded state");

        let config = Config {
            config_path: config_path.clone(),
            demo: true,
            ..Config::default()
        };
        let mut problems = crate::config::problems::ConfigProblems::default();
        let startup = crate::services::runtime::load_remembered_views(&config, &mut problems);

        assert_eq!(
            startup.views,
            RememberedViews::default(),
            "a demo run must not adopt the user's real remembered views"
        );
        assert!(
            !startup.saves_suppressed,
            "the demo carve-out is not a load failure; nothing is suppressed by it"
        );
        assert!(
            problems.messages().is_empty(),
            "a demo run must not report on a file it never read, got {:?}",
            problems.messages()
        );

        let mut session = RuntimeSession::new(
            config,
            Cli::parse_from(["nuvem", "--demo", "--config", config_path.to_str().unwrap()]),
            AppState::new(),
            startup.state_path,
        );

        session
            .dispatch_effects(
                vec![Effect::SaveRememberedViews {
                    views: views_for(ResourceServiceKind::CloudSql),
                }],
                false,
            )
            .await;
        // A demo session never starts a writer, so there is nothing to flush;
        // calling it anyway is what proves the no-store path is inert rather
        // than merely slow to land.
        session.flush_remembered_view_saves().await;

        assert_eq!(
            std::fs::read_to_string(&state_path).expect("read back"),
            written_by_the_real_run,
            "the real run's state file must be byte-for-byte untouched by a demo"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn the_save_effect_reaches_the_store_exactly_once() {
        let store = Arc::new(FakeRememberedViewStore::default());
        let mut session = RuntimeSession::new_with_remembered_view_store_for_test(
            Config::default(),
            AppState::new(),
            store.clone(),
        );

        let views = views_for(ResourceServiceKind::CloudSql);
        session
            .dispatch_effects(
                vec![Effect::SaveRememberedViews {
                    views: views.clone(),
                }],
                false,
            )
            .await;
        session.flush_remembered_view_saves().await;

        assert_eq!(store.saved.lock().unwrap().as_slice(), &[views]);
    }

    /// The latch still gets set, and it gets set through the result channel now
    /// rather than inline. This is the half of the ordering constraint that is
    /// not about bytes on disk: the failure has to arrive as one ordered
    /// transition into `remembered_view_saves_suppressed`, or the reducer's
    /// "report once per session" promise is a race.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_failing_store_latches_saves_off_through_the_result_channel() {
        let store = Arc::new(FakeRememberedViewStore {
            fail: true,
            ..Default::default()
        });
        let mut session = RuntimeSession::new_with_remembered_view_store_for_test(
            Config::default(),
            AppState::new(),
            store,
        );

        session
            .dispatch_effects(
                vec![Effect::SaveRememberedViews {
                    views: RememberedViews::default(),
                }],
                false,
            )
            .await;
        session.flush_remembered_view_saves().await;

        assert!(
            !session.app.root.remembered_view_saves_suppressed,
            "the write failing is not itself the latch; the result has to be \
             applied for that"
        );
        assert!(
            session.process_next_effect_result().await,
            "the failure must be queued as a result the run loop drains"
        );
        assert!(
            session.app.root.remembered_view_saves_suppressed,
            "a store error must still latch saves off for the session"
        );
    }

    /// The width the BigQuery prose puts on the enrichment fan-out, in both
    /// documents, against the semaphore that really bounds it.
    ///
    /// This pin lives here rather than beside the other BigQuery documentation
    /// pins in `tests/documentation_test.rs` for one reason:
    /// [`BIGQUERY_ENRICHMENT_CONCURRENCY`] is private to this module, and an
    /// integration test under `tests/` is a separate crate. `pub(crate)` would
    /// not reach it either -- only `pub` would, which turns an internal tuning
    /// number into the library's public API in order to satisfy a
    /// documentation test. A unit test inside the module reads the constant
    /// with no visibility change at all, so this is both the strongest
    /// available binding and the cheapest. `tests/documentation_test.rs`
    /// carries a comment pointing here, so the pin is findable from the file a
    /// reader looks in first.
    ///
    /// ARCHITECTURE.md makes the claim twice in the README's own idiom -- the
    /// calls "run behind `BIGQUERY_ENRICHMENT_CONCURRENCY` permits, four at a
    /// time", and the uncancelled remainder keeps "running four at a time".
    /// Pinning only the README would red one document on a retune and leave
    /// the other silently false, which is the failure this pin exists to
    /// prevent rather than to move one document over. Both sections are
    /// therefore parsed the same way. Both spell the width as a word today;
    /// the parse reads words only, and says so in its own failure message, so
    /// a sentence that switches to `4` fails as an unreadable claim rather
    /// than as a wrong one.
    ///
    /// The prose spells the width as a word and says it in several places, so
    /// asserting a section contains `"four"` would pass on the schema view's
    /// unrelated "four columns" and would survive the constant moving to 8.
    /// Instead every occurrence of the idiom the claim is made in -- `N at a
    /// time`, `the N already in flight` -- is found, the number word near it is
    /// taken, and each one is compared with the constant spelled out. An edit
    /// that retunes the semaphore and updates five of the six sentences fails
    /// on the sixth.
    ///
    /// What "every occurrence is compared" does **not** buy is a count. No
    /// expected number of claim sites is asserted, so deleting one of the
    /// documentation's four or one of ARCHITECTURE's two leaves the survivors
    /// agreeing with the constant and this test green. That is a deliberate
    /// trade: a pinned count fails on every honest rewrite that merges two
    /// sentences or moves one between documents, and deletion of the prose
    /// wholesale is what the `presence` checks in
    /// `tests/documentation_test.rs` are for. Read this as "no documented
    /// width disagrees with the semaphore", not as "the widths are all still
    /// written down".
    #[test]
    fn the_docs_document_the_real_bigquery_enrichment_concurrency() {
        const FEATURES: &str = include_str!("../../../docs/features.md");
        const ARCHITECTURE: &str = include_str!("../../../ARCHITECTURE.md");
        const SECTIONS: [(&str, &str, &str); 2] = [
            ("docs/features.md", FEATURES, "### BigQuery"),
            (
                "ARCHITECTURE.md",
                ARCHITECTURE,
                "### BigQuery: two drill-down levels, no detail pane",
            ),
        ];
        /// The number words the prose could plausibly be spelling, in the
        /// order a `usize` indexes them: `NUMBER_WORDS[4]` is `"four"`.
        const NUMBER_WORDS: [&str; 11] = [
            "zero", "one", "two", "three", "four", "five", "six", "seven", "eight", "nine", "ten",
        ];
        /// How far back from the idiom the width is allowed to sit. `four of
        /// them run at a time` puts three words between them, leaving the
        /// width fourth from the idiom; anything wider than this is a rewrite
        /// that should be re-read rather than silently matched.
        const LOOKBEHIND: usize = 6;

        let expected = *NUMBER_WORDS
            .get(BIGQUERY_ENRICHMENT_CONCURRENCY)
            .unwrap_or_else(|| {
                panic!(
                    "the prose spells the fan-out width as a word; teach this test how to \
                     spell {BIGQUERY_ENRICHMENT_CONCURRENCY} before setting the concurrency \
                     to it"
                )
            });

        let bare = |word: &str| -> String {
            word.chars()
                .filter(|character| character.is_ascii_alphabetic())
                .collect::<String>()
                .to_ascii_lowercase()
        };

        for (name, document, heading) in SECTIONS {
            let section = bigquery_section(name, document, heading);

            let mut documented: Vec<String> = Vec::new();
            for idiom in ["at a time", "already in flight"] {
                let mut rest = section.as_str();
                while let Some(at) = rest.find(idiom) {
                    let preceding: Vec<String> = rest[..at]
                        .split_whitespace()
                        .rev()
                        .take(LOOKBEHIND)
                        .map(bare)
                        .collect();
                    let widths: Vec<&String> = preceding
                        .iter()
                        .filter(|word| NUMBER_WORDS.contains(&word.as_str()))
                        .collect();
                    assert_eq!(
                        widths.len(),
                        1,
                        "{name}: the fan-out claim ending {idiom:?} must name exactly one \
                         width, spelled as a word, in the {LOOKBEHIND} words before it -- \
                         found {widths:?} among {preceding:?}. A digit (`4`) is not a width \
                         this test can read: spell it out, or teach the test to read digits. \
                         A claim this test cannot read is a claim it cannot check"
                    );
                    documented.push(widths[0].clone());
                    rest = &rest[at + idiom.len()..];
                }
            }

            assert!(
                !documented.is_empty(),
                "{name}: the BigQuery section no longer says how many enrichment calls run \
                 at a time, which is the only thing bounding what a wide dataset costs the \
                 machine"
            );
            for (index, width) in documented.iter().enumerate() {
                assert_eq!(
                    width,
                    expected,
                    "{name}: the BigQuery section's fan-out width #{} of {} documents a \
                     concurrency the semaphore does not grant",
                    index + 1,
                    documented.len()
                );
            }
        }
    }

    /// `bigquery_section` against a literal fixture, so editing
    /// docs/features.md or ARCHITECTURE.md can never make these pass or fail.
    ///
    /// The helper is a near-verbatim copy of `Doc::section_body` in
    /// `tests/documentation_test.rs`. Sharing one would need that helper made
    /// public across crates, which is not worth doing for a private tuning
    /// number — but of the two copies only that one carried a fixture
    /// (`section_body_is_scoped_body_only_and_fence_aware`), so this was the
    /// copy free to drift while the other stayed pinned. The four
    /// properties below are the four that one asserts, checked here against
    /// this copy.
    #[test]
    fn bigquery_section_is_scoped_body_only_and_fence_aware() {
        const FIXTURE: &str = "\
# Title

## Alpha

Alpha says four at a time.

## Beta

### Why the width is four

The claim is stated
across two lines here.

### Sibling

Sibling says nothing.

## Layout

Prose before the block.

```text
## a heading-shaped comment inside a fence
```

Prose after the block.

## Betamax

Betamax is a longer heading that starts with Beta.
";

        // 1. Scoped: a claim stated in another section is not in this one.
        assert!(
            !bigquery_section("FIXTURE.md", FIXTURE, "### Sibling").contains("four at a time"),
            "a section body must not carry a neighbouring section's prose"
        );

        // 2. Body-only. The heading here restates its body's words, so a
        //    haystack that kept the heading line would satisfy a claim from the
        //    heading alone — and the fan-out pin's whole job is counting claim
        //    sites.
        let why = bigquery_section("FIXTURE.md", FIXTURE, "### Why the width is four");
        assert!(
            !why.contains("Why the width"),
            "the heading line must not be part of the body it heads"
        );
        assert!(
            why.contains("The claim is stated across two lines here."),
            "the body itself must be in the body, reflowed to one line"
        );

        // 3. Fence-aware: a heading-shaped line inside a fenced block must not
        //    terminate the section. This is the case the BigQuery probe
        //    demonstrated — `## list the datasets` inside a pasteable
        //    `bq` block left the earlier scan seeing an empty haystack, so the
        //    pin would have gone green having checked nothing.
        let layout = bigquery_section("FIXTURE.md", FIXTURE, "## Layout");
        assert!(
            layout.contains("Prose after the block"),
            "a heading-shaped line inside a fenced block must not terminate the section"
        );

        // 4. Anchored on the whole heading line: a heading that is a prefix of
        //    a longer one must not select the longer one's span.
        let beta = bigquery_section("FIXTURE.md", FIXTURE, "## Beta");
        assert!(
            !beta.contains("Betamax is a longer heading"),
            "a prefix heading must not select the section it is a prefix of"
        );
        assert!(
            beta.contains("Sibling says nothing"),
            "a section body includes its own subsections"
        );
    }

    /// A markdown section body, reflowed to one line so a claim that wraps
    /// across a line break is one string to match.
    ///
    /// Fence-aware and level-aware, for the same reasons `Doc::section_body`
    /// in `tests/documentation_test.rs` is. The scan this replaces terminated
    /// on `"## "` and `"### "` only, which got it wrong in both directions: a
    /// `#### ` sub-heading inserted into either BigQuery section matched
    /// neither prefix and was walked straight past, while a `## ` or `### `
    /// line inside a fenced block -- a heading-shaped comment in a pasteable
    /// `bq` block is the obvious one -- terminated the scan above the section's
    /// own prose. The second case is the one the probe for this change
    /// demonstrated: `## list the datasets` added inside the first `bq` fence
    /// left the old scan seeing none of the four width claims, so it would
    /// have gone green on an empty haystack. Neither document carries such a
    /// line today; docs/features.md has no fenced `#` line at all, and the
    /// three ARCHITECTURE.md carries are all single-hash and all far outside
    /// its BigQuery section, so none of them terminated the old scan either. Both cases end the same way: claim sites silently leave
    /// the haystack and the pin quietly stops checking them.
    fn bigquery_section(name: &str, document: &str, heading: &str) -> String {
        let level = heading
            .chars()
            .take_while(|character| *character == '#')
            .count();
        assert!(level > 0, "{heading:?} is not a markdown heading");
        let is_fence = |line: &str| {
            let trimmed = line.trim_start();
            trimmed.starts_with("```") || trimmed.starts_with("~~~")
        };

        let mut fenced = false;
        let mut lines = document.lines();
        // The whole heading line has to match, so a heading that is a prefix
        // of a longer one cannot select the longer one's span.
        let found = lines.by_ref().any(|line| {
            if is_fence(line) {
                fenced = !fenced;
                return false;
            }
            !fenced && line == heading
        });
        assert!(
            found,
            "{name} documents BigQuery under a {heading:?} heading"
        );

        let mut body: Vec<&str> = Vec::new();
        for line in lines {
            if is_fence(line) {
                fenced = !fenced;
            } else if !fenced {
                let depth = line
                    .chars()
                    .take_while(|character| *character == '#')
                    .count();
                if depth > 0 && depth <= level && line[depth..].starts_with(' ') {
                    break;
                }
            }
            body.push(line);
        }
        let section = body.join(" ");
        let section = section.split_whitespace().collect::<Vec<_>>().join(" ");
        assert!(
            !section.is_empty(),
            "{name}'s {heading:?} section has a body"
        );
        section
    }
}
