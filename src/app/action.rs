// EffectResult carries payloads for every resource kind (compute, GKE, Cloud
// Run); boxing it to shrink Action would ripple through every call/match
// site for marginal benefit, since Action is not on a hot per-frame path.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    QuitRequested,
    Workspace(WorkspaceAction),
    Command(CommandAction),
    Configurations(ConfigurationsAction),
    Navigator(NavigatorAction),
    Resources(ResourcesAction),
    Jumps(JumpsAction),
    Logging(LoggingAction),
    Tunnels(TunnelsAction),
    FinOps(FinOpsAction),
    System(SystemAction),
    Navigation(NavAction),
    Effect(super::effect::Effect),
    EffectResult(super::effect::EffectResult),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NavAction {
    Back,
    Forward,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkspaceAction {
    Home,
    Configurations,
    Resources,
    Logging,
    Tunnels,
    FinOps,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandAction {
    Open,
    AppendChar(char),
    Backspace,
    SelectNext,
    SelectPrevious,
    Submit,
    Cancel,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigurationsAction {
    SelectNext,
    SelectPrevious,
    RequestActivateSelected,
    ConfirmActivation,
    CancelActivation,
    StartFiltering,
    UpdateFilter(String),
    ApplyFilter,
    CancelFilter,
    ClearFilter,
    StartCreate,
    AppendCreateChar(char),
    BackspaceCreateField,
    FocusNextCreateField,
    FocusPreviousCreateField,
    SubmitCreate,
    CancelCreate,
    StartEditActiveContext,
    AppendContextEditChar(char),
    BackspaceContextEditField,
    FocusNextContextEditField,
    FocusPreviousContextEditField,
    SubmitContextEdit,
    CancelContextEdit,
    OpenContextPicker,
    StartContextPickerFilter,
    UpdateContextPickerFilter(String),
    ApplyContextPickerFilter,
    CancelContextPickerFilter,
    SelectNextContextPickerItem,
    SelectPreviousContextPickerItem,
    ApplyContextPickerSelection,
    RetryContextPicker,
    CancelContextPicker,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NavigatorAction {
    SelectNext,
    SelectPrevious,
    StartFilter,
    AppendFilterChar(char),
    BackspaceFilter,
    ApplyFilter,
    CancelFilter,
    OpenSelected,
    ToggleSidebar,
    ToggleFocus,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResourcesAction {
    SelectService(nuvem_domain::gcp::resource::ResourceServiceKind),
    SelectNext,
    SelectPrevious,
    Refresh,
    OpenSelected,
    CloseDetail,
    ScrollDetailDown,
    ScrollDetailUp,
    SelectNextTab,
    SelectPreviousTab,
    SelectNextDetailField,
    SelectPreviousDetailField,
    StartFilter,
    AppendFilterChar(char),
    BackspaceFilter,
    ApplyFilter,
    CancelFilter,
    SelectPreviousColumn,
    SelectNextColumn,
    SortBySelectedColumn,
    ToggleWideColumns,
    ConfirmPendingAction,
    CancelPendingAction,
    CopyPendingCommand,
    CancelPendingCommandPreview,
    ConfirmSshLaunch,
    CancelSshLaunch,
    /// Resolves a managed-tunnel plan from the selected row and opens the
    /// launch confirmation. A no-op for services with no tunnel plan.
    RequestTunnel,
    /// Starts the previewed tunnel.
    ///
    /// `started_at` is stamped by the caller because reducers must never read
    /// the clock: it is the instant the launch was requested, and it becomes
    /// the entry's `started_at` (which the tunnel list ages against).
    ConfirmTunnelLaunch {
        started_at: std::time::Instant,
    },
    CancelTunnelLaunch,
    ConfirmK9sLaunch,
    CancelK9sLaunch,
    ToggleMark,
    RangeMark,
    ClearMarks,
    UnmarkVisible,
    BatchAckAppendDigit(char),
    BatchAckBackspace,
    ConfirmBatchAction,
    CancelBatchAction,
    ScrollBatchConfirmDown,
    ScrollBatchConfirmUp,
    /// Runs the plugin whose argv the confirmation is showing.
    ///
    /// Unlike [`ResourcesAction::CopyPendingCommand`], which the copy-only
    /// preview's `Enter` dispatches, this one executes.
    ConfirmPlugin,
    PluginAckAppendDigit(char),
    PluginAckBackspace,
    CancelPluginConfirmation,
    /// Appends a character to the focused plugin-prompt field.
    PluginPromptAppend(char),
    PluginPromptBackspace,
    PluginPromptNextField,
    PluginPromptPreviousField,
    /// Moves a dropdown field's selection; `true` is forward.
    PluginPromptCycleChoice(bool),
    /// Composes the argv from the collected values and opens the confirmation.
    /// Never runs anything: [`ResourcesAction::ConfirmPlugin`] is still the only
    /// arm that does.
    SubmitPluginPrompt,
    CancelPluginPrompt,
    /// Arms the plugin prefix (`,`). Dispatched unconditionally from
    /// the resource list and detail; the reducer refuses in a scope with no
    /// plugin, which is where the plugin set is in scope to ask.
    ArmPluginPrefix,
    /// The character pressed after the prefix. Every character, not only the
    /// declared ones: the reducer is what can tell "bound but unresolvable"
    /// from "never declared", and those are different sentences.
    PluginPrefixPress(char),
    CancelPluginPrefix,
    AscendPrefix,
    /// Leaves a drill-down view for the list it was entered from, without
    /// leaving the resources workspace.
    ///
    /// Dispatched from `Esc`, and only while a drill-down is open — the
    /// keymap gates it on that, because `Esc` otherwise means "go back" and
    /// a drill-down the operator is inside is not a history entry to leave.
    /// BigQuery's tables view is the only drill-down that claims it today.
    /// It also answers `-`/`Backspace` through [`Self::AscendPrefix`], so the
    /// two keys agree; Cloud Storage's object browser answers only those,
    /// because its descent is many levels deep and `Esc` has no
    /// level-at-a-time meaning to carry there.
    LeaveDrillDown,
    OpenBucketMetadata,
    NextPage,
    CopyResourceUri,
    /// Opens Cloud Logging scoped to whatever resource the current view has
    /// under the cursor. Resolution honours the view that is on
    /// screen — a Cloud Run revision beats its parent service, a browsed
    /// Cloud Storage bucket beats the bucket-list selection — and reports a
    /// status message rather than no-oping when nothing can be resolved.
    OpenLogsForSelection,
}

/// The related-resource jump picker (`J`).
///
/// `Open` is a request, not a guarantee: when the current selection has no
/// applicable jump the picker stays closed and a status message names why
/// (G4 — no silent no-ops).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JumpsAction {
    Open,
    SelectNext,
    SelectPrevious,
    /// Jumps to the selected offer's detail tab and closes the picker.
    Submit,
    Cancel,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoggingAction {
    SelectNext,
    SelectPrevious,
    OpenDetail,
    CloseDetail,
    ScrollDetailUp,
    ScrollDetailDown,
    ToggleTail,
    CycleTimeRange,
    /// Jump straight to a time bucket, bypassing the `CycleTimeRange` walk.
    SetTimeRange(nuvem_domain::gcp::logging::LogTimeRange),
    /// Step the server-side severity floor one rank up/down.
    RaiseSeverityFloor,
    LowerSeverityFloor,
    ToggleWrap,
    StartFilter,
    AppendFilterChar(char),
    BackspaceFilter,
    ApplyFilter,
    CancelFilter,
    CopySelectedEntry,
    CopySelectedInsertId,
    /// Dump the currently visible entries to a file beside the app's own log
    /// file, in a `log-dumps` sibling directory.
    SaveToFile,
    Refresh,
}

/// What the user can do to a managed tunnel. There is no confirmation step for
/// `KillSelected`: stopping a tunnel touches no cloud state, only a local child
/// process, and the tunnel can simply be started again.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TunnelsAction {
    SelectNext,
    SelectPrevious,
    /// Stops the selected tunnel's child process.
    KillSelected,
    /// Drops the selected row from the list. Only a tunnel that has already
    /// stopped or failed is dismissible.
    DismissSelected,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SystemAction {
    ToggleHelp,
    ClearOverlay,
    ScrollHelpDown,
    ScrollHelpUp,
    ScrollHelpPageDown,
    ScrollHelpPageUp,
    ScrollHelpTop,
    ScrollHelpBottom,
    /// Copies the last recorded gcloud diagnostic's sanitized, bounded
    /// technical detail to the clipboard (issue #31 AC #6).
    CopyDiagnostic,
    /// Closes the plugin output viewer.
    ///
    /// There is deliberately no `Open` sibling. The viewer is opened from the
    /// command palette, which reaches `SystemState::open_plugin_output`
    /// directly through its own descriptor arm — an action variant nothing
    /// dispatches would be a second entry point that only tests could take,
    /// and the tests would then be evidence about a path the app never
    /// follows.
    ClosePluginOutput,
    /// Moves to the next older retained run, and to the next newer one.
    PluginOutputOlder,
    PluginOutputNewer,
    ScrollPluginOutputDown,
    ScrollPluginOutputUp,
    ScrollPluginOutputPageDown,
    ScrollPluginOutputPageUp,
    /// Copies the run on screen to the clipboard — the escape hatch for
    /// output the retention bound cut.
    CopyPluginOutput,
    /// Cancels every plugin run currently in flight.
    ///
    /// Every run, not a selected one: a plugin's operations-log entry is
    /// rendered on Home's read-only overview card, which has no selection, and
    /// two plugins in flight at once is already the rare case. The alternative
    /// — adding selection to that card — is a larger change than the defect
    /// justifies.
    CancelRunningPlugins,
}

/// What the FinOps workspace can be asked to do.
///
/// One variant in v1: the workspace issues no mutations, so there is nothing
/// to act on beyond re-probing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FinOpsAction {
    /// Re-probe every (recommender, location) cell, deriving the locations
    /// from the rows the Resources workspace has listed.
    Refresh,
}
