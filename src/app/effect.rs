use nuvem_domain::gcp::configuration::{
    GcloudConfiguration, GcloudConfigurationName, GcloudContext,
};
use nuvem_domain::gcp::diagnostics::{
    GcloudDiagnostic, GcloudDiagnosticsReport, GcloudErrorKind, classify_gcloud_stderr,
    label_naming_binary, stderr_indicates_requester_pays,
};
use nuvem_domain::gcp::logging::{LogCursor, LogEntry, LogQuery, LogSeverity, LogTimeRange};
use nuvem_domain::gcp::resource::{
    BigQueryDataset, BigQuerySchemaField, BigQueryTable, BigQueryTableStats, CloudFunction,
    CloudRunRevision, CloudRunRevisionDetail, CloudRunService, CloudRunServiceDetail,
    CloudSqlInstance, CloudSqlInstanceDetail, CloudSqlUser, ComputeInstance, ComputeInstanceDetail,
    FirewallRule, GkeCluster, GkeClusterDetail, IamPolicyMember, PersistentDisk,
    PubSubSubscription, PubSubTopic, Secret, SecretDetail, SecretVersion, ServiceAccount,
    StorageBucket, StorageBucketDetail, StorageObjectDetail, StorageObjectPage, VpcNetwork,
    VpcSubnet,
};

use crate::app::state::configurations::{
    ConfigurationContextPickerItem, ConfigurationContextPickerKind,
};
use nuvem_domain::gcp::k9s::{K9sLaunchPlan, K9sLaunchTarget};

use nuvem_domain::gcp::finops::{CostRecommendation, RecommenderId};

use crate::app::state::finops::FinOpsTicket;
use crate::app::state::resources::{
    BatchAction, BatchRowRef, BigQueryDrillGeneration, BigQueryListingEpoch,
    CloudSqlInstanceAction, ComputeInstanceAction,
};
use crate::services::gcloud::cli::{CommandError, failure_diagnostic};
use crate::services::gcloud::configurations::{GcloudConfigurationDraft, GcloudContextEditDraft};
use crate::services::resources::finops::Budget;
use crate::services::runtime::interactive::InteractiveProcessOutcome;
use crate::services::runtime::tunnel::TunnelId;
use nuvem_domain::gcp::tunnel::TunnelKind;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    LoadConfigurations,
    ActivateConfiguration {
        name: GcloudConfigurationName,
    },
    CreateConfiguration {
        draft: GcloudConfigurationDraft,
    },
    UpdateConfigurationContext {
        draft: GcloudContextEditDraft,
    },
    LoadContextPicker {
        kind: ConfigurationContextPickerKind,
        context: GcloudContext,
    },
    RefreshComputeInstances {
        context: GcloudContext,
        filter: Option<String>,
    },
    RefreshGkeClusters {
        context: GcloudContext,
        filter: Option<String>,
    },
    LoadComputeInstanceDetail {
        context: GcloudContext,
        instance: ComputeInstance,
    },
    LoadGkeClusterDetail {
        context: GcloudContext,
        cluster: GkeCluster,
    },
    RefreshCloudSqlInstances {
        context: GcloudContext,
        filter: Option<String>,
    },
    LoadCloudSqlInstanceDetail {
        context: GcloudContext,
        instance: CloudSqlInstance,
    },
    LoadCloudSqlUsers {
        context: GcloudContext,
        instance: CloudSqlInstance,
    },
    RefreshStorageBuckets {
        context: GcloudContext,
    },
    /// One `(recommender, location)` cost-recommendation probe.
    ///
    /// One effect per cell rather than one per card, so a denied zone cannot
    /// blank a card that other zones answered, and each answer renders as it
    /// lands.
    LoadCostRecommendations {
        context: GcloudContext,
        recommender: RecommenderId,
        location: String,
        ticket: FinOpsTicket,
    },
    /// Budgets for one billing account. Billing-account scoped rather than
    /// project scoped, which is why the budget card states whose money it is
    /// describing.
    LoadBudgets {
        context: GcloudContext,
        ticket: FinOpsTicket,
    },
    LoadStorageBucketDetail {
        context: GcloudContext,
        bucket: StorageBucket,
    },
    LoadStorageObjectPage {
        context: GcloudContext,
        bucket: String,
        prefix: String,
        page_token: Option<String>,
        page_size: u32,
    },
    LoadStorageObjectDetail {
        context: GcloudContext,
        bucket: String,
        object_key: String,
    },
    /// `operation_id` is the operations-log entry this action seeded. It is
    /// carried through to the result so the result settles exactly its own
    /// entry: the same instance can have another action in flight, and
    /// `(kind, target)` cannot tell the two apart.
    ComputeInstanceAction {
        context: GcloudContext,
        instance: ComputeInstance,
        action: ComputeInstanceAction,
        operation_id: u64,
    },
    CloudSqlInstanceAction {
        context: GcloudContext,
        instance: CloudSqlInstance,
        action: CloudSqlInstanceAction,
        operation_id: u64,
    },
    /// A user-declared plugin's argv, already confirmed by the operator.
    ///
    /// `argv` is carried whole rather than rebuilt from the plugin id: the
    /// operator confirmed *these* elements against the row that was selected
    /// then, and re-rendering here could produce a different command than the
    /// one that was read. It already carries its own `--configuration`, which
    /// `plugin_argv` injects, so `context` is never appended to the argv a
    /// second time: it travels through to the result, where it is what tells a
    /// run confirmed under the context the operator has since left from one
    /// belonging to the context they are looking at now. `operation_id` is the
    /// operations-log entry this run settles, for the same reason the lifecycle
    /// actions carry one.
    RunPlugin {
        context: GcloudContext,
        label: String,
        argv: Vec<String>,
        operation_id: u64,
    },
    /// Cancels every in-flight [`Effect::RunPlugin`] child and settles the
    /// operations-log entries they would otherwise strand `Pending`.
    ///
    /// Carries nothing. The reducer does not know which runs are in flight —
    /// the session's registry does — so this asks the service layer the same
    /// way [`Effect::StopTunnel`] asks it to signal a child.
    CancelRunningPlugins,
    /// A marked-row batch for any batchable service. The action names the
    /// service, and each target carries everything the runtime needs to run it
    /// (`name` + `location`), so no service-specific row type reaches the
    /// effect layer. `concurrency: 0` means "runtime substitutes its
    /// configured default".
    ///
    /// Each target is paired with the id of the operations-log entry seeded for
    /// it, the same pairing the batch-progress workflow takes, so a target's
    /// result can settle that target's own entry.
    RunResourceBatch {
        context: GcloudContext,
        action: BatchAction,
        targets: Vec<(BatchRowRef, u64)>,
        concurrency: usize,
    },
    ProbeGcloudDiagnostics {
        configuration: Option<GcloudConfigurationName>,
    },
    CopyToClipboard {
        value: String,
        description: String,
    },
    /// Persist the remembered-view map. Emitted only when the map actually
    /// changed; see [`crate::app::reducer::root::reduce`].
    SaveRememberedViews {
        views: crate::app::state::remembered_view::RememberedViews,
    },
    /// Write already-rendered, already-sanitized log lines to a dump file. The
    /// reducer performs no I/O and knows no paths: the runtime derives the
    /// destination directory from the resolved config.
    SaveLogEntries {
        lines: Vec<String>,
    },
    LaunchInteractiveProcess {
        command: InteractiveCommand,
    },
    /// `severity` is a server-side floor folded into the gcloud filter, not a
    /// display filter; `LogSeverity::Default` means no floor.
    LoadRecentLogs {
        context: GcloudContext,
        query: LogQuery,
        severity: LogSeverity,
        range: LogTimeRange,
    },
    TailLogs {
        context: GcloudContext,
        query: LogQuery,
        severity: LogSeverity,
        cursor: LogCursor,
    },
    /// Best-effort kubectl probe of existing kubeconfig contexts, so the
    /// confirmation can say whether it will reuse a context or run
    /// get-credentials. Never blocks the launch.
    PrepareK9sLaunch {
        target: K9sLaunchTarget,
    },
    /// Runs `gcloud container clusters get-credentials` when required, then (on
    /// success) triggers the interactive k9s launch. A failed credential step
    /// prevents the launch.
    RunK9sCredentials {
        plan: K9sLaunchPlan,
    },
    RefreshCloudRunServices {
        context: GcloudContext,
    },
    LoadCloudRunServiceDetail {
        context: GcloudContext,
        service: CloudRunService,
    },
    LoadCloudRunRevisions {
        context: GcloudContext,
        service: CloudRunService,
    },
    LoadCloudRunRevisionDetail {
        context: GcloudContext,
        region: String,
        revision: String,
    },
    RefreshSecrets {
        context: GcloudContext,
        filter: Option<String>,
    },
    RefreshCloudFunctions {
        context: GcloudContext,
        filter: Option<String>,
    },
    RefreshPersistentDisks {
        context: GcloudContext,
        filter: Option<String>,
    },
    RefreshPubSubTopics {
        context: GcloudContext,
        filter: Option<String>,
    },
    RefreshPubSubSubscriptions {
        context: GcloudContext,
        filter: Option<String>,
    },
    /// No `filter` field, for the reason [`Effect::RefreshBigQueryDatasets`]
    /// has none: there is nothing a filter could narrow the *request* with.
    /// Three of the four networks columns — MODE, ROUTING and SUBNETS — are
    /// derived from `autoCreateSubnetworks`, `routingConfig.routingMode` and
    /// the length of the `subnetworks` array, so none of them is a verbatim
    /// substring of any raw field and a pushed clause could only drop rows the
    /// operator can read on screen. The table narrows wholly client-side. See
    /// `crate::services::resources::vpc`.
    ///
    /// Carrying the field anyway was not free. It made the refresh *look*
    /// filterable, and a filter change accordingly re-issued it — spawning a
    /// gcloud process to re-download a document already in state, and putting
    /// the visible table at the mercy of that redundant call's failure.
    RefreshVpcNetworks {
        context: GcloudContext,
    },
    /// No `filter` field, for the reason [`Effect::RefreshVpcNetworks`] has
    /// none. `REGION` and `NETWORK` are the tails of self-link URLs and
    /// `PRIVATE GOOGLE ACCESS` renders a bool as a word, so none of the three
    /// is a verbatim substring of a raw field and a pushed clause could only
    /// drop rows the operator can read. The table narrows wholly client-side.
    RefreshVpcSubnets {
        context: GcloudContext,
    },
    /// The `filter` is carried for shape and never pushed down: `ACTION` is
    /// derived from which key is present, and none of the other cells is a
    /// verbatim raw field either.
    /// No `filter` field, for the reason [`Effect::RefreshVpcNetworks`] has
    /// none. `ACTION` is derived from which of `allowed`/`denied` arrived,
    /// `TARGETS` reads `all instances` where the rule scopes nothing, and the
    /// `DISABLED` badge is a word this application adds — none of the three
    /// appears in any raw field, so a pushed clause could only drop rows the
    /// operator can read. The table narrows wholly client-side.
    RefreshFirewallRules {
        context: GcloudContext,
    },
    /// No `filter` field, unlike every other list refresh: `bq ls` accepts no
    /// server-side filter expression, so there is nothing to narrow the
    /// request with and the datasets table narrows wholly client-side. See
    /// `GcloudBigQueryService::load_datasets`.
    RefreshBigQueryDatasets {
        context: GcloudContext,
    },
    /// Lists the tables inside one dataset, for the drill-down `Enter` opens.
    ///
    /// A per-target load rather than a service refresh: it is issued for the
    /// dataset the view just entered, and the drill generation comes back on
    /// the result so a listing the view is no longer waiting on can be dropped
    /// instead of landing under the wrong header. Filterless for the same
    /// reason [`Effect::RefreshBigQueryDatasets`] is.
    ///
    /// The identity and not the dataset id is what the result is checked
    /// against: re-entering the same dataset is a second drill wearing the
    /// first one's id, and `r` inside a drill is a second call wearing the
    /// first call's generation. So both counters ride out and back —
    /// `generation` for the drill, `listing` for the call.
    LoadBigQueryTables {
        context: GcloudContext,
        dataset_id: String,
        generation: BigQueryDrillGeneration,
        listing: BigQueryListingEpoch,
    },
    /// Asks `bq show` for one listed table's rows, bytes and last-modified
    /// time. One of these is issued per row of a landed listing — a dataset
    /// with 200 tables costs 200 of them, and nothing bounds that: a cap would
    /// leave the rows past it with the same empty cells a refused call leaves,
    /// which an operator could not tell apart.
    ///
    /// Best-effort, unlike [`Effect::LoadBigQueryTables`]. A refusal here
    /// leaves that one row's three stats unavailable and the table intact,
    /// because `bigquery.tables.list` granted with `bigquery.tables.get`
    /// denied is an ordinary read-only IAM shape. The two calls behind this
    /// table must never acquire one policy.
    ///
    /// It carries two identities, not one. `generation` says which drill, and
    /// `listing` says which listing call inside it. Neither covers the other:
    /// `r` re-lists without starting a new drill, so the generation alone
    /// would let a show issued before the retry fill a row the retry produced;
    /// and `Esc` bumps the generation without minting an epoch, so the epoch
    /// alone would let a show for a dataset the operator has left stay in
    /// scope.
    LoadBigQueryTableStats {
        context: GcloudContext,
        dataset_id: String,
        table_id: String,
        generation: BigQueryDrillGeneration,
        listing: BigQueryListingEpoch,
    },
    /// Reads one table's column schema, for the drill-down `Enter` opens on a
    /// table row.
    ///
    /// Exactly one of these is in flight per `Enter`, which is the whole
    /// difference from [`Effect::LoadBigQueryTableStats`] — that one is fanned
    /// out over every row of a landed listing and so is dispatched off the run
    /// loop behind a permit pool. A single call needs neither, and routing it
    /// through the enrichment pool would make one operator-initiated drill
    /// queue behind a wide dataset's outstanding shows.
    ///
    /// One identity rides out and back, not two. `generation` says which
    /// drill, and entering a table is a view transition that mints one, so a
    /// schema answered for a table the operator has left — or for a previous
    /// entry into the table they are back inside — fails the check. There is
    /// no second call under one drill for a listing epoch to separate, the way
    /// `r` makes one at the tables level; see
    /// `ResourcesState::matches_bigquery_schema_scope`.
    LoadBigQuerySchema {
        context: GcloudContext,
        dataset_id: String,
        table_id: String,
        generation: BigQueryDrillGeneration,
    },
    RefreshServiceAccounts {
        context: GcloudContext,
        filter: Option<String>,
    },
    /// No `filter` field: the policy read fetches one object and flattens it,
    /// so there is no list request to narrow — see
    /// `GcloudIamService::load_policy_members`.
    RefreshIamPolicy {
        context: GcloudContext,
    },
    LoadSecretDetail {
        context: GcloudContext,
        secret: Secret,
    },
    /// Lists one secret's versions: id, state, create time. Its sibling
    /// subcommand — the one returning payload material — has no effect variant
    /// here, and must never acquire one.
    LoadSecretVersions {
        context: GcloudContext,
        secret: Secret,
    },
    /// Starts a managed tunnel child under `id` and supervises it. The plan
    /// carries everything needed to allocate the port and build the argv, so
    /// the runtime never reaches back into app state.
    StartTunnel {
        id: TunnelId,
        context: GcloudContext,
        plan: TunnelLaunchPlan,
    },
    /// Stops the managed tunnel child registered under `id`. Idempotent: an id
    /// with no live child is a no-op in the runtime's registry.
    StopTunnel {
        id: TunnelId,
    },
    /// Re-resolve the config file at runtime and apply safe presentation /
    /// navigation fields live. Handled synchronously by the runtime session.
    ReloadConfig,
}

/// Everything the runtime needs to launch one tunnel, resolved by the app layer
/// before the effect is emitted.
///
/// It is a value rather than a reference into state on purpose: the launch runs
/// off the reducer, and by the time it does, the row it came from may have been
/// killed, dismissed, or replaced by a context switch. Carrying the plan means
/// the argv is built from what the user actually asked for.
///
/// `instance_name`/`zone` are used by the instance tunnels and `connection_name`
/// by the Cloud SQL proxy; the unused ones are empty for a given `kind`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TunnelLaunchPlan {
    pub kind: TunnelKind,
    /// What the tunnel points at, for display.
    pub target_label: String,
    /// The far side of the tunnel, for display.
    pub remote_label: String,
    pub instance_name: String,
    pub zone: String,
    /// The Cloud SQL `project:region:instance` triple.
    pub connection_name: String,
    /// The port to try first; `None` lets the allocator take an ephemeral one.
    pub preferred_local_port: Option<u16>,
    pub remote_port: u16,
    /// Ports already promised to live tunnels, so the allocator does not hand
    /// the same one out twice.
    pub claimed_ports: Vec<u16>,
}

/// A typed interactive-process launch request. The runtime resolves it to an
/// argv `InteractiveProcessSpec` using the configured gcloud binary.
// A short-lived, single-instance command value (never stored in bulk); the
// size gap between the GceSsh and K9s variants is not worth an extra box.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InteractiveCommand {
    GceSsh {
        context: GcloudContext,
        instance: ComputeInstance,
    },
    /// Launch k9s against an already-resolved kubeconfig context.
    K9s { kube_context: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EffectResult {
    ConfigurationsLoaded {
        configurations: Vec<GcloudConfiguration>,
    },
    CostRecommendationsLoaded {
        ticket: FinOpsTicket,
        recommender: RecommenderId,
        location: String,
        recommendations: Vec<CostRecommendation>,
    },
    CostRecommendationsFailed {
        ticket: FinOpsTicket,
        recommender: RecommenderId,
        location: String,
        error: AppError,
    },
    BudgetsLoaded {
        ticket: FinOpsTicket,
        budgets: Vec<Budget>,
    },
    BudgetsFailed {
        ticket: FinOpsTicket,
        error: AppError,
    },
    ConfigurationActivated {
        configuration: GcloudConfiguration,
    },
    ConfigurationActivationReloadFailed {
        configuration: GcloudConfiguration,
        error: AppError,
    },
    ConfigurationsReloadedWithWarning {
        configurations: Vec<GcloudConfiguration>,
        error: AppError,
    },
    ConfigurationCreatedReloadFailed {
        configuration: GcloudConfiguration,
        error: AppError,
    },
    ConfigurationContextUpdateReloadFailed {
        configuration: GcloudConfiguration,
        error: AppError,
    },
    ContextPickerLoaded {
        configuration: GcloudConfigurationName,
        kind: ConfigurationContextPickerKind,
        items: Vec<ConfigurationContextPickerItem>,
    },
    ContextPickerLoadFailed {
        configuration: GcloudConfigurationName,
        kind: ConfigurationContextPickerKind,
        error: AppError,
    },
    ComputeInstancesLoaded {
        context: GcloudContext,
        instances: Vec<ComputeInstance>,
    },
    ComputeInstancesLoadFailed {
        context: GcloudContext,
        error: AppError,
    },
    ComputeInstanceDetailLoaded {
        context: GcloudContext,
        detail: ComputeInstanceDetail,
    },
    GkeClustersLoaded {
        context: GcloudContext,
        clusters: Vec<GkeCluster>,
    },
    GkeClustersLoadFailed {
        context: GcloudContext,
        error: AppError,
    },
    GkeClusterDetailLoaded {
        context: GcloudContext,
        detail: GkeClusterDetail,
    },
    GkeClusterDetailLoadFailed {
        context: GcloudContext,
        cluster_name: String,
        cluster_location: String,
        error: AppError,
    },
    CloudSqlInstancesLoaded {
        context: GcloudContext,
        instances: Vec<CloudSqlInstance>,
    },
    CloudSqlInstancesLoadFailed {
        context: GcloudContext,
        error: AppError,
    },
    CloudSqlInstanceDetailLoaded {
        context: GcloudContext,
        // Boxed to keep `EffectResult`'s size down (this detail carries a
        // large describe payload); see clippy::large_enum_variant.
        detail: Box<CloudSqlInstanceDetail>,
    },
    CloudSqlInstanceDetailLoadFailed {
        context: GcloudContext,
        instance_name: String,
        instance_region: String,
        error: AppError,
    },
    CloudSqlUsersLoaded {
        context: GcloudContext,
        instance_name: String,
        users: Vec<CloudSqlUser>,
    },
    CloudSqlUsersLoadFailed {
        context: GcloudContext,
        instance_name: String,
        error: AppError,
    },
    ComputeInstanceDetailLoadFailed {
        context: GcloudContext,
        instance_name: String,
        instance_zone: String,
        error: AppError,
    },
    StorageBucketsLoaded {
        context: GcloudContext,
        buckets: Vec<StorageBucket>,
    },
    StorageBucketsLoadFailed {
        context: GcloudContext,
        error: AppError,
    },
    StorageBucketDetailLoaded {
        context: GcloudContext,
        detail: StorageBucketDetail,
    },
    StorageBucketDetailLoadFailed {
        context: GcloudContext,
        bucket_name: String,
        error: AppError,
    },
    StorageObjectPageLoaded {
        context: GcloudContext,
        bucket: String,
        prefix: String,
        append: bool,
        page: StorageObjectPage,
    },
    StorageObjectPageLoadFailed {
        context: GcloudContext,
        bucket: String,
        prefix: String,
        append: bool,
        error: AppError,
    },
    StorageObjectDetailLoaded {
        context: GcloudContext,
        detail: StorageObjectDetail,
    },
    StorageObjectDetailLoadFailed {
        context: GcloudContext,
        object_key: String,
        error: AppError,
    },
    /// `operation_id` names the operations-log entry this result settles. It is
    /// not an `Option`: both producers — the single-row action and each batch
    /// target — are handed an id by the reducer, and an optional id would
    /// reinstate a `(kind, target)` fallback that settles whichever entry for
    /// the same instance happens to be pending.
    ComputeInstanceActionSucceeded {
        context: GcloudContext,
        instance_name: String,
        instance_zone: String,
        action: ComputeInstanceAction,
        operation_id: u64,
    },
    ComputeInstanceActionFailed {
        context: GcloudContext,
        instance_name: String,
        instance_zone: String,
        action: ComputeInstanceAction,
        error: AppError,
        operation_id: u64,
    },
    CloudSqlInstanceActionSucceeded {
        context: GcloudContext,
        instance_name: String,
        action: CloudSqlInstanceAction,
        operation_id: u64,
    },
    CloudSqlInstanceActionFailed {
        context: GcloudContext,
        instance_name: String,
        action: CloudSqlInstanceAction,
        error: AppError,
        operation_id: u64,
    },
    /// A plugin run that exited zero. `output` is the child's stdout, carried
    /// so the operator can be shown what their own command printed; `label` is
    /// the plugin's, because it is the only name the operator ever gave this
    /// command. `operation_id` settles the entry the run seeded, for the same
    /// reason the lifecycle results carry one.
    ///
    /// `context` is the one the run was confirmed under. The run happens off
    /// the run loop, so the operator can switch configuration while the child
    /// is still alive; without it there would be nothing to tell a result
    /// belonging to the context they left from one belonging to the context
    /// they are looking at, and the first would complete the second's status
    /// operation.
    PluginRunSucceeded {
        context: GcloudContext,
        label: String,
        output: String,
        /// What the run wrote to stderr while still exiting 0 — a quota
        /// warning, a deprecation notice. Carried separately from `output`
        /// because the viewer marks the two apart, and dropped entirely
        /// before the plugin output work.
        stderr: String,
        operation_id: u64,
    },
    PluginRunFailed {
        context: GcloudContext,
        label: String,
        error: AppError,
        /// The failure's own stderr, unsanitized and unbounded.
        ///
        /// `error` already carries a sanitized 200-byte detail for the status
        /// line and the copy-diagnostic key; this is the same text kept whole
        /// so the viewer can show an error trailer longer than that bound,
        /// which a `gcloud` error routinely is.
        stderr: String,
        operation_id: u64,
    },
    /// A plugin run the operator cancelled.
    ///
    /// Distinct from [`EffectResult::PluginRunFailed`] for one reason:
    /// `PluginRunFailed` records a diagnostic unconditionally and by design —
    /// a run's failure belongs to the operator who started it even after they
    /// switch configuration — so routing a cancellation through it would leave
    /// a fabricated gcloud failure under the copy-diagnostic key. It settles no
    /// operations-log entry: the cancellation that produced it already settled
    /// every registered run by what its claim proved.
    PluginRunCancelled {
        context: GcloudContext,
        label: String,
        operation_id: u64,
    },
    ClipboardCopied {
        description: String,
    },
    ClipboardCopyFailed {
        description: String,
        error: AppError,
    },
    /// A remembered-view save succeeded. Carries nothing and produces no
    /// status message: the user pressed a service key, and confirming a
    /// background write every time they do would be noise. It exists so the
    /// dispatch arm has an honest success value instead of reusing a variant
    /// that means something else.
    RememberedViewsSaved,
    /// A remembered-view save failed. Reported once — the reducer latches on
    /// it — because saves are eager and an unwritable path would otherwise
    /// warn on every service keypress.
    RememberedViewsSaveFailed {
        error: String,
    },
    /// `path` is absolute, so the status line tells the user exactly where the
    /// dump landed.
    LogEntriesSaved {
        path: String,
        entries: usize,
    },
    LogEntriesSaveFailed {
        error: AppError,
    },
    InteractiveProcessCompleted {
        description: String,
        outcome: InteractiveProcessOutcome,
    },
    K9sLaunchPrepared {
        target: K9sLaunchTarget,
        existing_contexts: Option<Vec<String>>,
    },
    K9sCredentialsReady {
        kube_context: String,
        cluster_name: String,
    },
    K9sCredentialsFailed {
        cluster_name: String,
        error: AppError,
    },
    GcloudDiagnosticsProbed {
        report: GcloudDiagnosticsReport,
    },
    CloudRunServicesLoaded {
        context: GcloudContext,
        services: Vec<CloudRunService>,
    },
    CloudRunServicesLoadFailed {
        context: GcloudContext,
        error: AppError,
    },
    CloudRunServiceDetailLoaded {
        context: GcloudContext,
        detail: CloudRunServiceDetail,
    },
    CloudRunServiceDetailLoadFailed {
        context: GcloudContext,
        service_name: String,
        service_region: String,
        error: AppError,
    },
    CloudRunRevisionsLoaded {
        context: GcloudContext,
        service_name: String,
        service_region: String,
        revisions: Vec<CloudRunRevision>,
    },
    CloudRunRevisionsLoadFailed {
        context: GcloudContext,
        service_name: String,
        service_region: String,
        error: AppError,
    },
    CloudRunRevisionDetailLoaded {
        context: GcloudContext,
        detail: CloudRunRevisionDetail,
    },
    CloudRunRevisionDetailLoadFailed {
        context: GcloudContext,
        revision_name: String,
        region: String,
        error: AppError,
    },
    SecretsLoaded {
        context: GcloudContext,
        secrets: Vec<Secret>,
    },
    SecretsLoadFailed {
        context: GcloudContext,
        error: AppError,
    },
    CloudFunctionsLoaded {
        context: GcloudContext,
        functions: Vec<CloudFunction>,
    },
    CloudFunctionsLoadFailed {
        context: GcloudContext,
        error: AppError,
    },
    PersistentDisksLoaded {
        context: GcloudContext,
        disks: Vec<PersistentDisk>,
    },
    PersistentDisksLoadFailed {
        context: GcloudContext,
        error: AppError,
    },
    PubSubTopicsLoaded {
        context: GcloudContext,
        topics: Vec<PubSubTopic>,
    },
    PubSubTopicsLoadFailed {
        context: GcloudContext,
        error: AppError,
    },
    VpcNetworksLoaded {
        context: GcloudContext,
        networks: Vec<VpcNetwork>,
    },
    VpcSubnetsLoaded {
        context: GcloudContext,
        subnets: Vec<VpcSubnet>,
    },
    FirewallRulesLoaded {
        context: GcloudContext,
        rules: Vec<FirewallRule>,
    },
    FirewallRulesLoadFailed {
        context: GcloudContext,
        error: AppError,
    },
    VpcSubnetsLoadFailed {
        context: GcloudContext,
        error: AppError,
    },
    VpcNetworksLoadFailed {
        context: GcloudContext,
        error: AppError,
    },
    PubSubSubscriptionsLoaded {
        context: GcloudContext,
        subscriptions: Vec<PubSubSubscription>,
    },
    PubSubSubscriptionsLoadFailed {
        context: GcloudContext,
        error: AppError,
    },
    BigQueryDatasetsLoaded {
        context: GcloudContext,
        datasets: Vec<BigQueryDataset>,
    },
    BigQueryDatasetsLoadFailed {
        context: GcloudContext,
        error: AppError,
    },
    BigQueryTablesLoaded {
        context: GcloudContext,
        generation: BigQueryDrillGeneration,
        listing: BigQueryListingEpoch,
        tables: Vec<BigQueryTable>,
    },
    BigQueryTablesLoadFailed {
        context: GcloudContext,
        generation: BigQueryDrillGeneration,
        listing: BigQueryListingEpoch,
        error: AppError,
    },
    /// One table's `bq show` answered. Carries the table it is about, because
    /// the fan-out puts N of these in flight at once and the result is the
    /// only thing that says which row it belongs to — the effect that produced
    /// it is gone by then.
    BigQueryTableStatsLoaded {
        context: GcloudContext,
        table_id: String,
        generation: BigQueryDrillGeneration,
        listing: BigQueryListingEpoch,
        stats: BigQueryTableStats,
    },
    /// One table's `bq show` was refused, and that is the end of it: the row
    /// keeps the three unavailable stats it was listed with, the table keeps
    /// its rows, and no diagnostic is painted. See the arm in
    /// `src/app/reducer/root.rs` for why this reports nothing — the short
    /// version is that a fan-out of N calls would otherwise be a fan-out of N
    /// status messages over a table the operator can read perfectly well.
    ///
    /// It carries the same identity a success does, and is gated on it the
    /// same way: what the arm writes is three explicit unavailables onto that
    /// row, so an abandoned listing's refusal arriving late must not blank a
    /// row the live listing has already filled.
    BigQueryTableStatsLoadFailed {
        context: GcloudContext,
        table_id: String,
        generation: BigQueryDrillGeneration,
        listing: BigQueryListingEpoch,
        error: AppError,
    },
    /// One table's column schema answered, flattened into the rows the schema
    /// view paints.
    ///
    /// It carries the drill generation and no table id, unlike
    /// [`EffectResult::BigQueryTableStatsLoaded`]. That one needs an id
    /// because N shows are in flight at once and the result is the only thing
    /// saying which row it belongs to; here there is one call and one table,
    /// and the table it is about is the one the generation names. An id on
    /// this result could not tighten the guard either: re-entering the same
    /// table mints a fresh generation, so a stale answer for the live table's
    /// id is already out of scope, and no path could make an id conjunct false
    /// on its own.
    BigQuerySchemaLoaded {
        context: GcloudContext,
        generation: BigQueryDrillGeneration,
        fields: Vec<BigQuerySchemaField>,
    },
    /// The schema read was refused, and that fails the drill — the opposite of
    /// [`EffectResult::BigQueryTableStatsLoadFailed`], which leaves one row's
    /// cells unavailable and says nothing.
    ///
    /// The asymmetry is the same one `bq ls` and the per-row `bq show` have
    /// one level up. This call is the entire content of the view the operator
    /// just opened: there is no partial schema to leave standing, and an empty
    /// column list would read as "this table has no columns", which is not a
    /// table that exists. So the view keeps the table in its header and says
    /// what went wrong instead.
    BigQuerySchemaLoadFailed {
        context: GcloudContext,
        generation: BigQueryDrillGeneration,
        error: AppError,
    },
    ServiceAccountsLoaded {
        context: GcloudContext,
        accounts: Vec<ServiceAccount>,
    },
    ServiceAccountsLoadFailed {
        context: GcloudContext,
        error: AppError,
    },
    IamPolicyLoaded {
        context: GcloudContext,
        members: Vec<IamPolicyMember>,
    },
    IamPolicyLoadFailed {
        context: GcloudContext,
        error: AppError,
    },
    SecretDetailLoaded {
        context: GcloudContext,
        detail: SecretDetail,
    },
    SecretDetailLoadFailed {
        context: GcloudContext,
        secret_name: String,
        error: AppError,
    },
    SecretVersionsLoaded {
        context: GcloudContext,
        secret_name: String,
        versions: Vec<SecretVersion>,
    },
    SecretVersionsLoadFailed {
        context: GcloudContext,
        secret_name: String,
        error: AppError,
    },
    Failed {
        error: AppError,
    },
    /// Carries the `severity`/`range` window the page was fetched for, so the
    /// reducer can drop a page whose narrowing no longer matches the state it
    /// would be applied to. Without it, two loads racing after rapid `s`/`S`
    /// or bucket presses would land last-arrival-wins.
    RecentLogsLoaded {
        context: GcloudContext,
        query: LogQuery,
        severity: LogSeverity,
        range: LogTimeRange,
        entries: Vec<LogEntry>,
        dropped: usize,
    },
    RecentLogsLoadFailed {
        context: GcloudContext,
        query: LogQuery,
        error: AppError,
    },
    /// Carries the `severity` floor the poll was issued under, for the same
    /// reason as [`EffectResult::RecentLogsLoaded`]. There is no `range` to
    /// echo: the tail is cursor-driven and `read_since` sends no
    /// `--freshness`.
    LogsTailed {
        context: GcloudContext,
        query: LogQuery,
        severity: LogSeverity,
        entries: Vec<LogEntry>,
        dropped: usize,
    },
    LogsTailFailed {
        context: GcloudContext,
        query: LogQuery,
        error: AppError,
    },
    /// The runtime has chosen the loopback port this tunnel's child will be
    /// told to bind, and has not launched it yet.
    ///
    /// Reported as its own result rather than folded into readiness because the
    /// port has to reach `TunnelsState` *before* the child exists: a starting
    /// child has not `bind()`ed, so the OS still calls its port free, and
    /// `claimed_ports` is the only thing that stops a tunnel started moments
    /// later from being handed the same one.
    TunnelPortAllocated {
        id: TunnelId,
        local_port: u16,
    },
    /// The tunnel is serving traffic. `local_port` is the port the child
    /// actually reported, when it reported one; it wins over the requested one.
    TunnelReady {
        id: TunnelId,
        local_port: Option<u16>,
    },
    /// The tunnel will not serve traffic. `message` is already sanitized and
    /// bounded by the classifier — it comes from untrusted child output.
    TunnelFailed {
        id: TunnelId,
        message: String,
    },
    /// Something worth showing happened on a tunnel that is still working: an
    /// error-shaped line the supervisor deliberately did not act on, because
    /// once a tunnel is up such lines are usually a healthy child's chatter
    /// (`ssh` reporting one refused forwarded connection).
    ///
    /// A notice, not a verdict: it sets the row's MESSAGE and leaves the status
    /// exactly where it was. Without it the line would be invisible — a
    /// healthy-then-broken tunnel would end with no diagnostics at all.
    /// `message` is already sanitized and bounded; it comes from child output.
    TunnelNotice {
        id: TunnelId,
        message: String,
    },
    /// The child's output reached EOF and the process is gone. Whether that is
    /// a failure depends on what the tunnel was doing at the time, so the
    /// reducer decides; the runtime only reports the fact.
    TunnelExited {
        id: TunnelId,
    },
    /// The child was stopped as asked.
    TunnelStopped {
        id: TunnelId,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GcloudFailure {
    /// Set when the kind is known from the error origin (missing exec, cancelled).
    pub kind_hint: Option<GcloudErrorKind>,
    /// Raw message/stderr; sanitized only at display time.
    pub detail: String,
    pub exit_code: Option<i32>,
    pub command_summary: Option<String>,
    /// Which binary was missing, when `kind_hint` is `MissingExecutable`.
    ///
    /// Always a static literal from
    /// [`crate::services::gcloud::cli::CommandProgram::binary_name`], never a
    /// user-configured binary path: the diagnostic interpolates it into
    /// user-facing text without sanitizing it. `None` for every other kind,
    /// and for a missing-executable failure whose origin did not say — the
    /// rendering then stays binary-agnostic rather than guessing gcloud.
    pub missing_binary: Option<&'static str>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppError {
    Gcloud(GcloudFailure),
    Parse(String),
    MissingContext,
}

impl AppError {
    /// Plain gcloud error with no known origin/exit code (classified from text).
    pub fn gcloud(detail: impl Into<String>) -> Self {
        AppError::Gcloud(GcloudFailure {
            kind_hint: None,
            detail: detail.into(),
            exit_code: None,
            command_summary: None,
            missing_binary: None,
        })
    }

    /// Classifies this error into a coarse kind for degraded-service display.
    pub fn kind(&self) -> GcloudErrorKind {
        match self {
            AppError::Gcloud(f) => f
                .kind_hint
                .unwrap_or_else(|| classify_gcloud_stderr(&f.detail)),
            AppError::Parse(_) => GcloudErrorKind::Parse,
            AppError::MissingContext => GcloudErrorKind::MissingProject,
        }
    }

    /// True when this error's underlying gcloud stderr indicates a Cloud
    /// Storage requester-pays / `userProjectMissing` condition. Distinct
    /// from `kind()`'s coarse [`GcloudErrorKind`]; only the storage browser
    /// currently checks this, to render a message distinct from a generic
    /// permission-denied failure.
    pub fn indicates_requester_pays(&self) -> bool {
        match self {
            AppError::Gcloud(f) => stderr_indicates_requester_pays(&f.detail),
            AppError::Parse(_) | AppError::MissingContext => false,
        }
    }

    pub fn detail(&self) -> &str {
        match self {
            AppError::Gcloud(f) => &f.detail,
            AppError::Parse(m) => m,
            AppError::MissingContext => "no active gcloud context",
        }
    }

    pub fn exit_code(&self) -> Option<i32> {
        match self {
            AppError::Gcloud(f) => f.exit_code,
            _ => None,
        }
    }

    pub fn command_summary(&self) -> Option<&str> {
        match self {
            AppError::Gcloud(f) => f.command_summary.as_deref(),
            _ => None,
        }
    }

    /// The binary whose absence caused this failure, when it is known.
    ///
    /// `None` for every error that is not a missing executable, and for a
    /// missing executable whose origin did not name the program.
    pub fn missing_binary(&self) -> Option<&'static str> {
        match self {
            AppError::Gcloud(f) => f.missing_binary,
            AppError::Parse(_) | AppError::MissingContext => None,
        }
    }

    /// The failure category in words, naming the missing binary when this
    /// error knows which one it was — as opposed to `self.kind().label()`,
    /// which cannot. Prefer this at any site that has the error in hand.
    pub fn label(&self) -> std::borrow::Cow<'static, str> {
        label_naming_binary(self.kind(), self.missing_binary())
    }

    /// This error as the display-safe diagnostic the UI reads, attributed to
    /// `service`.
    ///
    /// The single funnel from an error to user-facing words. It lives here
    /// rather than in the reducer so that every path — and every test — gets
    /// the same sanitization *and* the same missing-binary attribution; a
    /// second hand-rolled `GcloudDiagnostic::new` call elsewhere is how the
    /// binary name gets dropped again.
    pub fn diagnostic(&self, service: &str) -> GcloudDiagnostic {
        GcloudDiagnostic::new(
            self.kind(),
            service,
            self.command_summary().map(str::to_string),
            self.exit_code(),
            self.detail(),
        )
        .naming_missing_binary(self.missing_binary())
    }

    /// Attaches a command summary (e.g. `"gcloud compute instances list"`) for
    /// display/copy in the resulting diagnostic. No-op for non-`Gcloud`
    /// variants.
    ///
    /// Stored verbatim: the redaction and the length bound are applied where
    /// the summary becomes user-facing, by `GcloudDiagnostic::new`. Prefer a
    /// static literal, and never pass raw stderr or a whole argv — a summary
    /// names the command, it does not reproduce it. The one operator-authored
    /// value on this path is a plugin's own label, which the operator wrote and
    /// is already reading in the confirmation modal.
    pub fn with_command_summary(mut self, summary: impl Into<String>) -> Self {
        if let AppError::Gcloud(failure) = &mut self {
            failure.command_summary = Some(summary.into());
        }
        self
    }
}

impl From<CommandError> for AppError {
    fn from(error: CommandError) -> Self {
        match error {
            // The program is carried through, not dropped. `CommandError`'s
            // own `Display` has named the right binary since slice 1, but this
            // conversion is where the user-facing diagnostic is built, and
            // dropping `program` here is what made a missing `bq` render as
            // "missing gcloud CLI — Install the gcloud CLI".
            CommandError::MissingExecutable { program, detail } => {
                AppError::Gcloud(GcloudFailure {
                    kind_hint: Some(GcloudErrorKind::MissingExecutable),
                    detail,
                    exit_code: None,
                    command_summary: None,
                    missing_binary: Some(program.binary_name()),
                })
            }
            CommandError::Io(message) => AppError::gcloud(message),
            // `detail` is the stream the child actually diagnosed on, not
            // stderr by name: `bq` writes its errors to stdout and leaves
            // stderr empty, so this arm used to build a failure whose whole
            // content was "" — and `AppError::kind` then classified that empty
            // string rather than the message.
            CommandError::Failed {
                stdout,
                stderr,
                exit_code,
            } => AppError::Gcloud(GcloudFailure {
                kind_hint: None,
                detail: failure_diagnostic(&stdout, &stderr).to_string(),
                exit_code,
                command_summary: None,
                missing_binary: None,
            }),
            // The caveat rides along for the same reason it does on `TimedOut`
            // below: killing the CLI ends nuvem's view of the call and nothing
            // else, so a lifecycle action abandoned at shutdown has to say the
            // operation may still be running rather than read as an undo.
            // It is empty for a read, which is every cancellation
            // that existed before — a cancelled listing started nothing.
            CommandError::Cancelled { ceiling } => AppError::Gcloud(GcloudFailure {
                kind_hint: Some(GcloudErrorKind::Cancelled),
                detail: format!(
                    "gcloud command was cancelled{}",
                    ceiling.abandoned_call_caveat()
                ),
                exit_code: None,
                command_summary: None,
                missing_binary: None,
            }),
            // `Unknown`, stated rather than left to be inferred. The kind is
            // the right one — a timed-out call is a call that has not answered
            // yet, not one that answered no, so Transient retryability and the
            // poll ticker reissuing it are what an operator would do by hand,
            // where `Cancelled` is Persistent because somebody decided to stop.
            //
            // Leaving `kind_hint: None` did not produce it. With no hint,
            // `AppError::kind` classifies the detail, and the detail says
            // "timed out" — one of `classify_gcloud_stderr`'s Network needles.
            // Network is Transient too, so the retry gate never noticed; what
            // the operator got was the network label and "Check your network
            // connection and retry" for a child nuvem killed at its own
            // ceiling.
            CommandError::TimedOut { seconds, ceiling } => AppError::Gcloud(GcloudFailure {
                kind_hint: Some(GcloudErrorKind::Unknown),
                // The caveat is part of the detail rather than an extra field:
                // it is the same sentence at every render site, and a lifecycle
                // timeout that reads as a clean refusal is the failure worth
                // preventing everywhere rather than in the one view that
                // remembered to ask.
                detail: format!(
                    "gcloud command timed out after {seconds}s{}",
                    ceiling.abandoned_call_caveat()
                ),
                exit_code: None,
                command_summary: None,
                missing_binary: None,
            }),
        }
    }
}

impl From<crate::services::resources::compute::ComputeLoadError> for AppError {
    fn from(error: crate::services::resources::compute::ComputeLoadError) -> Self {
        use crate::services::resources::compute::ComputeLoadError as E;
        match error {
            E::Command(c) => c.into(),
            E::Parse(p) => AppError::Parse(p.to_string()),
        }
    }
}

/// The conversion the Cloud SQL action path was missing.
///
/// Its caller built `AppError::gcloud(error.to_string())`, which flattens a
/// classified failure into a detail string and leaves `kind_hint` empty — so
/// the kind was re-derived by reading that string, and a `TimedOut` whose text
/// says "timed out" came back classified `Network`. Every other service reaches
/// `AppError` through a conversion like this one; Cloud SQL was the outlier.
impl From<crate::services::resources::cloudsql::CloudSqlLoadError> for AppError {
    fn from(error: crate::services::resources::cloudsql::CloudSqlLoadError) -> Self {
        use crate::services::resources::cloudsql::CloudSqlLoadError as E;
        match error {
            E::Command(c) => c.into(),
            E::Parse(p) => AppError::Parse(p.to_string()),
        }
    }
}

impl From<crate::services::resources::gke::GkeLoadError> for AppError {
    fn from(error: crate::services::resources::gke::GkeLoadError) -> Self {
        use crate::services::resources::gke::GkeLoadError as E;
        match error {
            E::Command(c) => c.into(),
            E::Parse(p) => AppError::Parse(p.to_string()),
        }
    }
}

impl From<crate::services::gcloud::configurations::GcloudConfigurationError> for AppError {
    fn from(error: crate::services::gcloud::configurations::GcloudConfigurationError) -> Self {
        use crate::services::gcloud::configurations::GcloudConfigurationError as E;
        // Preserve the full contextual message (which variant-specific fields like
        // configuration name are folded into) as the detail, while still deriving
        // kind_hint/exit_code from the underlying CommandError's origin where present.
        // Only the two contextual variants need the rendered message, so it is
        // computed from a reference before the value is consumed by the match below.
        let message = matches!(
            error,
            E::CreatedConfigurationSetupFailed { .. } | E::ContextUpdatePartiallyApplied { .. }
        )
        .then(|| error.to_string());
        match error {
            E::Command(c) => c.into(),
            E::CreatedConfigurationSetupFailed { source, .. } => {
                let mut inner: AppError = source.into();
                if let AppError::Gcloud(failure) = &mut inner {
                    failure.detail = message.expect("computed above for this variant");
                }
                inner
            }
            E::ContextUpdatePartiallyApplied { source, .. } => {
                // Unboxed first: the source is boxed for the enum's width, not
                // for its behaviour, and this conversion is the same one every
                // other variant makes.
                let mut inner: AppError = (*source).into();
                if let AppError::Gcloud(failure) = &mut inner {
                    failure.detail = message.expect("computed above for this variant");
                }
                inner
            }
            E::Parse(p) => AppError::Parse(p.to_string()),
        }
    }
}

impl std::fmt::Display for AppError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AppError::MissingContext => write!(f, "no gcloud context configured"),
            AppError::Gcloud(_) | AppError::Parse(_) => write!(f, "{}", self.detail()),
        }
    }
}

/// The resource a detail load asked about, kept beside the load so its answer
/// can be checked against it.
///
/// A detail pane opens on a target and shows "Loading detail for ..." until a
/// result for *that* target arrives. Results for any other target are dropped,
/// because they are usually stale: the operator moved on before the previous
/// load returned. That rule has one blind spot. When a load answers with a
/// document about a different resource than the one it asked for, the answer
/// is dropped as stale and no failure ever follows, so the pane waits forever.
/// [`DetailRequest::check`] turns that answer into the load's own failure,
/// naming both resources, so the pane always settles.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DetailRequest {
    ComputeInstance { name: String, zone: String },
    GkeCluster { name: String, location: String },
    CloudSqlInstance { name: String, region: String },
    StorageBucket { name: String },
    StorageObject { key: String },
    Secret { name: String },
    CloudRunService { name: String, region: String },
    CloudRunRevision { name: String, region: String },
}

impl DetailRequest {
    /// The request an effect makes, when it is a detail load.
    pub fn of(effect: &Effect) -> Option<Self> {
        Some(match effect {
            Effect::LoadComputeInstanceDetail { instance, .. } => Self::ComputeInstance {
                name: instance.name.clone(),
                zone: instance.zone.clone(),
            },
            Effect::LoadGkeClusterDetail { cluster, .. } => Self::GkeCluster {
                name: cluster.name.clone(),
                location: cluster.location.clone(),
            },
            Effect::LoadCloudSqlInstanceDetail { instance, .. } => Self::CloudSqlInstance {
                name: instance.name.clone(),
                region: instance.region.clone(),
            },
            Effect::LoadStorageBucketDetail { bucket, .. } => Self::StorageBucket {
                name: bucket.name.clone(),
            },
            Effect::LoadStorageObjectDetail { object_key, .. } => Self::StorageObject {
                key: object_key.clone(),
            },
            Effect::LoadSecretDetail { secret, .. } => Self::Secret {
                name: secret.name.clone(),
            },
            Effect::LoadCloudRunServiceDetail { service, .. } => Self::CloudRunService {
                name: service.name.clone(),
                region: service.region.clone(),
            },
            Effect::LoadCloudRunRevisionDetail {
                region, revision, ..
            } => Self::CloudRunRevision {
                name: revision.clone(),
                region: region.clone(),
            },
            _ => return None,
        })
    }

    /// Passes `result` through unless it is a detail answered for another
    /// resource, which becomes this request's load failure.
    ///
    /// Each comparison uses the fields the reducer gates the same result on,
    /// so a result this lets through is one the pane accepts, and one it
    /// rewrites is one the pane would have dropped.
    pub fn check(self, result: EffectResult) -> EffectResult {
        fn mismatch(asked: &str, answered: &str) -> AppError {
            AppError::Parse(format!(
                "asked for {asked}, but the response describes {answered}"
            ))
        }
        match (self, result) {
            (
                Self::ComputeInstance { name, zone },
                EffectResult::ComputeInstanceDetailLoaded { context, detail },
            ) if detail.name != name || detail.zone != zone => {
                EffectResult::ComputeInstanceDetailLoadFailed {
                    context,
                    error: mismatch(
                        &format!("{name} in {zone}"),
                        &format!("{} in {}", detail.name, detail.zone),
                    ),
                    instance_name: name,
                    instance_zone: zone,
                }
            }
            (
                Self::GkeCluster { name, location },
                EffectResult::GkeClusterDetailLoaded { context, detail },
            ) if detail.name != name || detail.location != location => {
                EffectResult::GkeClusterDetailLoadFailed {
                    context,
                    error: mismatch(
                        &format!("{name} in {location}"),
                        &format!("{} in {}", detail.name, detail.location),
                    ),
                    cluster_name: name,
                    cluster_location: location,
                }
            }
            (
                Self::CloudSqlInstance { name, region },
                EffectResult::CloudSqlInstanceDetailLoaded { context, detail },
            ) if detail.name != name || detail.region != region => {
                EffectResult::CloudSqlInstanceDetailLoadFailed {
                    context,
                    error: mismatch(
                        &format!("{name} in {region}"),
                        &format!("{} in {}", detail.name, detail.region),
                    ),
                    instance_name: name,
                    instance_region: region,
                }
            }
            (
                Self::StorageBucket { name },
                EffectResult::StorageBucketDetailLoaded { context, detail },
            ) if detail.name != name => EffectResult::StorageBucketDetailLoadFailed {
                context,
                error: mismatch(&name, &detail.name),
                bucket_name: name,
            },
            (
                Self::StorageObject { key },
                EffectResult::StorageObjectDetailLoaded { context, detail },
            ) if detail.key != key => EffectResult::StorageObjectDetailLoadFailed {
                context,
                error: mismatch(&key, &detail.key),
                object_key: key,
            },
            (Self::Secret { name }, EffectResult::SecretDetailLoaded { context, detail })
                if detail.name != name =>
            {
                EffectResult::SecretDetailLoadFailed {
                    context,
                    error: mismatch(&name, &detail.name),
                    secret_name: name,
                }
            }
            (
                Self::CloudRunService { name, region },
                EffectResult::CloudRunServiceDetailLoaded { context, detail },
            ) if detail.name != name || detail.region != region => {
                EffectResult::CloudRunServiceDetailLoadFailed {
                    context,
                    error: mismatch(
                        &format!("{name} in {region}"),
                        &format!("{} in {}", detail.name, detail.region),
                    ),
                    service_name: name,
                    service_region: region,
                }
            }
            (
                Self::CloudRunRevision { name, region },
                EffectResult::CloudRunRevisionDetailLoaded { context, detail },
            ) if detail.name != name => EffectResult::CloudRunRevisionDetailLoadFailed {
                context,
                error: mismatch(&name, &detail.name),
                revision_name: name,
                region,
            },
            (_, result) => result,
        }
    }
}
