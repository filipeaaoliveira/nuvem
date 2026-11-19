use crate::app::projection::columns::identity_layout;
use crate::app::projection::table::SortDirection;
use crate::app::state::plugins::PluginId;
use crate::config::columns::ColumnSettings;
use crate::config::jumps::JumpField;
use crate::config::plugins::{PluginInput, PluginInputKind, TextRule};
use crate::services::resources::list_filter::is_expressible_label_selector;
use nuvem_domain::gcp::configuration::GcloudContext;
use nuvem_domain::gcp::diagnostics::{DiagnosticProblem, GcloudDiagnostic};
use nuvem_domain::gcp::filter::{
    CompiledFilter, FilterMode, FilterQuery, LabelSelector, filter_mode as filter_mode_of,
    label_filter_field, parse_filter,
};
use nuvem_domain::gcp::k9s::{CredentialStep, K9sLaunchPlan, K9sLaunchTarget};
use nuvem_domain::gcp::resource::{
    BigQueryDataset, BigQueryTable, CloudFunction, CloudFunctionEnvironment, CloudFunctionState,
    CloudRunRevision, CloudRunRevisionDetail, CloudRunService, CloudRunServiceDetail,
    CloudRunServiceStatus, CloudSqlAvailability, CloudSqlInstance, CloudSqlInstanceDetail,
    CloudSqlStatus, ComputeInstance, ComputeInstanceDetail, DiskStatus, EndpointVisibility,
    FirewallAction, FirewallDirection, FirewallInstanceScope, FirewallRule, FirewallTargets,
    GkeCluster, GkeClusterDetail, GkeClusterStatus, IamMemberKind, IamPolicyMember, PersistentDisk,
    PubSubDelivery, PubSubSubscription, PubSubSubscriptionState, PubSubTopic, ResourceServiceKind,
    ResourceStatus, Secret, SecretDetail, SecretVersion, ServiceAccount, StorageBucket,
    StorageBucketDetail, StorageObject, StorageObjectDetail, StorageObjectPage, StoragePrefix,
    SubscriptionCount, VpcNetwork, VpcRoutingMode, VpcSubnet, VpcSubnetMode, rule_targets_instance,
};
use nuvem_domain::gcp::resource_call::ResourceCall;
use nuvem_domain::gcp::text::sanitize_display_text;
use std::collections::BTreeSet;

/// Declares a lifecycle-action enum and, from that same variant list, its
/// `ALL` constant.
///
/// `ALL` is what the per-service parity tests walk to prove that the argv the
/// runtime executes and the "Command:" line an operator confirms agree for
/// every action. A hand-maintained `ALL` cannot carry that weight: adding a
/// variant to the enum and not to the constant compiles, and the parity test
/// then silently stops covering the new action while still reporting green.
/// Generating both from one list removes the place the omission could happen
/// — there is nowhere to declare a variant except inside an invocation, and
/// the invocation regenerates `ALL`.
///
/// Asserting `ALL.len() == N` would not achieve this. It is the same
/// remembered invariant with an extra step, because the author who forgot to
/// extend `ALL` also forgot to bump the number.
///
/// The generated `ALL` is deliberately the only item this produces: every
/// other method on these enums is an exhaustive `match`, which the compiler
/// already forces open when a variant appears.
macro_rules! lifecycle_actions {
    (
        $(#[$enum_meta:meta])*
        pub enum $name:ident {
            $($variant:ident),+ $(,)?
        }
    ) => {
        $(#[$enum_meta])*
        pub enum $name {
            $($variant,)+
        }

        impl $name {
            /// Every lifecycle action, in declaration order, generated from
            /// the enum's own variant list by `lifecycle_actions!` so a
            /// caller that must cover the whole set walks the enum instead of
            /// re-listing it.
            pub const ALL: [Self; [$(stringify!($variant)),+].len()] = [$(Self::$variant),+];
        }
    };
}

mod storage;
pub use self::storage::*;

mod batch;
mod bigquery;
mod cloudrun;
mod cloudsql;
mod compute;
mod disks;
mod drill;
mod filter_cache;
mod firewall;
mod functions;
mod gke;
mod iam;
mod pubsub;
mod secrets;
mod vpc;
mod vpc_subnets;
pub use self::batch::*;
pub use self::bigquery::*;
pub use self::cloudrun::*;
pub use self::cloudsql::*;
pub use self::compute::*;
pub use self::disks::*;
pub use self::firewall::*;
pub use self::functions::*;
pub use self::gke::*;
pub use self::iam::*;
pub use self::pubsub::*;
pub use self::secrets::*;
pub use self::vpc::*;
pub use self::vpc_subnets::*;

use self::filter_cache::CompiledResourceFilter;
use std::borrow::Cow;
use std::fmt::Write as _;
use unicode_general_category::{GeneralCategory, get_general_category};

/// Renders an argv into the exact "Command:" string the app shows an operator:
/// the line confirmed before a destructive action runs, the line a read-only
/// preview offers to copy, and the argv a plugin's confirmation shows. This is
/// the single rendering every argv-built modal goes through, so two of them
/// cannot drift apart in formatting — and a plugin, whose argv is the only one
/// a user wrote, is read in the same shape as the app's own.
///
/// Arguments outside [`quote_for_display`]'s safe set are quoted so that an
/// argument containing a space renders as one argument rather than two. The
/// quoting is **display only**: the argv is passed to the subprocess unsplit
/// and unquoted, and this string is never handed to a shell. It is not shell
/// escaping and must not be treated as though it were.
pub(crate) fn gcloud_command_line(argv: &[String]) -> String {
    let rendered: Vec<Cow<'_, str>> = argv.iter().map(|arg| quote_for_display(arg)).collect();
    format!("gcloud {}", rendered.join(" "))
}

/// True for the bytes that need no quoting to render faithfully: an argument
/// made only of these cannot be mistaken for two arguments, so it is shown
/// bare. The set is deliberately narrow — every literal and interpolated value
/// the app's argv builders emit today falls inside it, so widening it buys
/// nothing and each addition is another character whose rendering has to be
/// argued about.
fn is_display_safe(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'/' | b'=' | b':' | b'@' | b'-')
}

/// Renders one argv element unambiguously.
///
/// Safe arguments are borrowed untouched. Everything else is POSIX
/// single-quoted, which keeps the printable cases — a space, a `$`, a `;` —
/// both readable and paste-able. That paste-ability claim covers the printable
/// characters only: an escaped `\n` is a literal backslash-n inside POSIX
/// single quotes, so a rendering that escaped a character is readable but no
/// longer paste-equivalent to the argument.
///
/// Two Unicode general categories are escaped rather than passed through,
/// because a confirmation preview must not display in a different order, or
/// with different visible content, than the argv that will be passed:
///
/// - **Cc**, the control characters, for two reasons about the terminal rather
///   than the shell: the confirmation modal is sized by `paragraph_fits` from
///   a line count and a per-line width (`src/ui/mod.rs`), so an embedded
///   newline would clip the modal it is invisible to; and an ESC byte would
///   reach the terminal as a live escape sequence.
/// - **Cf**, the format characters, which draw as nothing while changing what
///   the operator reads. A bidi override (U+202E) makes `delete<RLO>etaerc`
///   draw as `deletecreate`, so the confirmed line reads in the opposite order
///   from the argv; a zero-width space or BOM (U+200B, U+FEFF) hides inside a
///   name that then looks identical to a different resource's.
///
/// The test is the character's general category, not a list of code points, so
/// a format character nobody enumerated is still escaped. Printable non-ASCII
/// is quoted but never escaped — `ção` renders as typed — so the rule is a
/// category test rather than an is-ASCII one. The escaped form is chosen by
/// code point rather than by category: below U+0100 the two-digit `\xNN` form,
/// anything above `\u{NNNN}`, either of which names the code point
/// unambiguously. Every Cc is below U+0100, but so are some Cf — U+00AD SOFT
/// HYPHEN renders `\xAD`, not `\u{00AD}`.
///
/// This is deliberately not the policy of [`nuvem_domain::gcp::text`], whose
/// sanitizers remove the offending bytes instead. The three bound for a
/// surface — `sanitize_rendered_text`, `sanitize_rendered_paragraph` and
/// `sanitize_clipboard_text` — strip whole escape sequences and then replace
/// or collapse the remaining control characters; `sanitize_display_text`,
/// which the others build on, replaces control characters only and leaves a
/// sequence's readable residue behind. Those sinks — buffer lines, rendered
/// paragraphs, the clipboard — carry text incidentally, so bytes that would
/// corrupt the surface are simply removed. A confirmation preview is the
/// opposite: the operator is approving exactly the bytes that will be passed
/// to the subprocess, so every byte has to stay visible. Hence escaping rather
/// than stripping, and hence `\x1B[31m` rendering with its `[31m` residue
/// intact where `sanitize_rendered_text` would remove the whole sequence.
fn quote_for_display(arg: &str) -> Cow<'_, str> {
    if !arg.is_empty() && arg.bytes().all(is_display_safe) {
        return Cow::Borrowed(arg);
    }

    let mut quoted = String::with_capacity(arg.len() + 2);
    quoted.push('\'');
    for character in arg.chars() {
        match character {
            '\'' => quoted.push_str("'\\''"),
            '\n' => quoted.push_str("\\n"),
            '\t' => quoted.push_str("\\t"),
            '\r' => quoted.push_str("\\r"),
            invisible
                if matches!(
                    get_general_category(invisible),
                    GeneralCategory::Control | GeneralCategory::Format
                ) =>
            {
                let code_point = invisible as u32;
                if code_point <= 0xFF {
                    write!(quoted, "\\x{code_point:02X}")
                } else {
                    write!(quoted, "\\u{{{code_point:04X}}}")
                }
                .expect("writing into a String cannot fail");
            }
            printable => quoted.push(printable),
        }
    }
    quoted.push('\'');
    Cow::Owned(quoted)
}

/// Identity of a resource row for mark-tracking purposes, independent of row
/// position or sort/filter state. `location` is the zone for GCE and the
/// region for Cloud SQL.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct ResourceRowId {
    pub name: String,
    pub location: String,
}

const DETAIL_PAGE_SIZE: u16 = 4;

/// Advances the cursor to the next displayed sortable column, or stays put
/// when none exists to the right.
///
/// `current` and the result are **display positions**; `layout` translates
/// each of them into the **catalog index** `is_sortable` answers about. The
/// two are the same number only for an unconfigured service.
fn next_sortable_column<T>(
    current: usize,
    layout: &[usize],
    is_sortable: fn(usize) -> Option<T>,
) -> usize {
    ((current + 1)..layout.len())
        .find(|&position| is_sortable(layout[position]).is_some())
        .unwrap_or(current)
}

/// Moves the cursor to the nearest lower displayed sortable column, or stays
/// put when none exists to the left. Positions and indices as in
/// [`next_sortable_column`]; the walk is clamped to the layout so a cursor
/// left beyond its right-hand end by a reload can still walk back into it.
fn previous_sortable_column<T>(
    current: usize,
    layout: &[usize],
    is_sortable: fn(usize) -> Option<T>,
) -> usize {
    (0..current.min(layout.len()))
        .rev()
        .find(|&position| is_sortable(layout[position]).is_some())
        .unwrap_or(current)
}

/// A service's effective layout, as a free function so the storage arms can
/// read it while holding a mutable borrow of [`ResourcesState::storage`].
/// [`ResourcesState::column_layout`] is the accessor everyone else uses.
fn service_layout(settings: &ColumnSettings, kind: ResourceServiceKind) -> &[usize] {
    settings
        .get(kind)
        .and_then(|service| service.columns.as_deref())
        .unwrap_or_else(|| identity_layout(kind))
}

/// The display position of a catalog index in `layout`.
///
/// Falls back to the first column when the layout does not show it, so the
/// sort arrow always lands on a rendered column rather than off the end of
/// the table. Every way a collection acquires a sort field puts it inside the
/// layout: the built-in default is `Name`, which every configured list must
/// include ([`crate::config::columns`]); a configured `sort` is rejected
/// unless it is among the configured columns; and `s` can only reach a
/// displayed column.
fn display_position(layout: &[usize], catalog_index: usize) -> usize {
    layout
        .iter()
        .position(|&index| index == catalog_index)
        .unwrap_or(0)
}

/// A service's configured default sort, but only on the load that actually
/// changes that service's block — `None` when `settings` says exactly what
/// `running` already said.
///
/// The configured sort is a *default*: it decides what the list opens on and
/// the user's `s` presses override it for the session. A reload the user
/// triggered for some other reason — another service's block, an unrelated
/// `[general]` field — says nothing about this service, so re-installing its
/// sort there would silently reorder a list the user had sorted by hand. At
/// startup nothing is running yet, so every configured service installs.
fn newly_configured_sort(
    settings: &ColumnSettings,
    running: &ColumnSettings,
    kind: ResourceServiceKind,
) -> Option<(usize, SortDirection)> {
    let configured = settings.get(kind);
    if configured == running.get(kind) {
        return None;
    }
    configured.and_then(|service| service.sort)
}

/// Installs `configured_sort` — the service's configured *default* sort, when
/// this load is the one that brings it in ([`newly_configured_sort`]) — and
/// anchors the sort and the column cursor inside `layout`, the layout the same
/// settings are about to install. Rows, filters, selection and load phase are
/// none of this function's business.
///
/// A validated setting always resolves (`sort` is rejected unless it names a
/// sortable column), so an index no `SortField` claims leaves the built-in
/// default in place rather than panicking.
///
/// The two anchors exist because a reload can install a layout that no longer
/// shows where the collection is currently pointing:
///
/// - a sort field outside the layout would order the rows by an invisible
///   column while the arrow ([`display_position`]) sat on a visible one, so it
///   falls back to `default_sort` — `Name`, which every configured list must
///   contain;
/// - a cursor on a non-sortable column (a valid list may open on one) would
///   highlight a header `s` silently does nothing for, so it moves to the
///   first sortable display position.
///
/// Both are no-ops for an unconfigured service: its layout is the whole
/// catalog, whose first column is the sortable `name`.
fn install_service_settings<Row, Detail, SortField: Copy, DetailTab>(
    collection: &mut ResourceCollectionState<Row, Detail, SortField, DetailTab>,
    configured_sort: Option<(usize, SortDirection)>,
    layout: &[usize],
    from_column: fn(usize) -> Option<SortField>,
    column: fn(SortField) -> usize,
    default_sort: SortField,
) {
    if let Some((catalog_index, direction)) = configured_sort
        && let Some(field) = from_column(catalog_index)
    {
        collection.sort_field = field;
        collection.sort_direction = direction;
    }

    if !layout.contains(&column(collection.sort_field)) {
        collection.sort_field = default_sort;
        collection.sort_direction = SortDirection::Ascending;
    }

    let is_sortable = |position: usize| {
        layout
            .get(position)
            .is_some_and(|&catalog_index| from_column(catalog_index).is_some())
    };
    if !is_sortable(collection.selected_column) {
        collection.selected_column = (0..layout.len())
            .find(|&position| is_sortable(position))
            .unwrap_or(0);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResourcesViewMode {
    List,
    Detail,
}

/// The shared confirmation modal payload, dispatched per resource kind so both
/// GCE and Cloud SQL flow through the same preview + confirm path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResourceActionConfirmation {
    Compute(ComputeActionConfirmation),
    CloudSql(CloudSqlActionConfirmation),
}

impl ResourceActionConfirmation {
    pub fn context(&self) -> &GcloudContext {
        match self {
            Self::Compute(confirmation) => confirmation.context(),
            Self::CloudSql(confirmation) => confirmation.context(),
        }
    }

    /// The call this confirmation is for, whichever service it belongs to.
    pub fn call(&self) -> ResourceCall {
        match self {
            Self::Compute(confirmation) => confirmation.call(),
            Self::CloudSql(confirmation) => confirmation.call(),
        }
    }

    pub fn impact_warning(&self) -> &'static str {
        match self {
            Self::Compute(confirmation) => confirmation.impact_warning(),
            Self::CloudSql(confirmation) => confirmation.impact_warning(),
        }
    }

    /// Returns the GCE confirmation payload, if this is a Compute action.
    pub fn compute(&self) -> Option<&ComputeActionConfirmation> {
        match self {
            Self::Compute(confirmation) => Some(confirmation),
            Self::CloudSql(_) => None,
        }
    }

    /// The service this confirmation belongs to. The confirmation slot is
    /// shared across services, so a per-service teardown uses this to close
    /// only the confirmation it owns.
    pub fn service_kind(&self) -> ResourceServiceKind {
        match self {
            Self::Compute(_) => ResourceServiceKind::ComputeInstances,
            Self::CloudSql(_) => ResourceServiceKind::CloudSql,
        }
    }

    /// Returns the Cloud SQL confirmation payload, if this is a Cloud SQL action.
    pub fn cloud_sql(&self) -> Option<&CloudSqlActionConfirmation> {
        match self {
            Self::CloudSql(confirmation) => Some(confirmation),
            Self::Compute(_) => None,
        }
    }

    /// GCE-only accessor retained for the compute confirmation flow.
    ///
    /// Panics if called on a Cloud SQL confirmation.
    pub fn action(&self) -> ComputeInstanceAction {
        self.compute()
            .expect("action() called on a non-compute confirmation")
            .action()
    }

    /// GCE-only accessor retained for the compute confirmation flow.
    ///
    /// Panics if called on a Cloud SQL confirmation.
    pub fn instance(&self) -> &ComputeInstance {
        self.compute()
            .expect("instance() called on a non-compute confirmation")
            .instance()
    }

    /// Short resource-kind label for UI chrome (modal titles, fallback text):
    /// "GCE" for Compute, "Cloud SQL" for CloudSql.
    pub fn resource_kind_label(&self) -> &'static str {
        match self {
            Self::Compute(_) => "GCE",
            Self::CloudSql(_) => "Cloud SQL",
        }
    }
}

/// The payload a confirmed action produces so the reducer can emit the correct
/// effect per resource kind.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActionConfirmationSubmission {
    Compute {
        action: ComputeInstanceAction,
        context: GcloudContext,
        instance: ComputeInstance,
    },
    CloudSql {
        action: CloudSqlInstanceAction,
        context: GcloudContext,
        instance: CloudSqlInstance,
    },
}

/// A command line waiting to be read and copied.
///
/// It carries the rendered line rather than the resource it came from: the row
/// may be gone by the time the operator reads it, and re-deriving the line at
/// render time would show a command built from a different row than the one the
/// modal was opened for. This is the same reason [`TunnelLaunchPreview`] carries
/// its resolved plan.
///
/// Generalised from "the SSH command for a compute instance", when a
/// plugin was a command to copy and reused this modal rather than adding a
/// mechanism of its own. A plugin is answered rather than copied now, so it
/// opens [`PluginConfirmation`] and this modal is the SSH command line again —
/// still generalised, but with one caller.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceCommandPreview {
    title: String,
    command: String,
    copy_description: String,
}

impl ResourceCommandPreview {
    pub fn new(title: String, command: String, copy_description: String) -> Self {
        Self {
            title,
            command,
            copy_description,
        }
    }

    /// The modal's heading — what this command is, in the app's own words.
    pub fn title(&self) -> &str {
        &self.title
    }

    pub fn command_preview(&self) -> String {
        self.command.clone()
    }

    /// What the clipboard toast calls the copied line ("Copied {…} to
    /// clipboard"). Separate from the title because the title is a heading and
    /// the toast is a sentence fragment: folding them would make the SSH copy
    /// report "Copied Show SSH command to clipboard".
    pub fn copy_description(&self) -> &str {
        &self.copy_description
    }
}

/// A resolved managed-tunnel launch waiting for the user's confirmation.
///
/// The plan is resolved once, when the user asks for the tunnel, and carried
/// verbatim into `Effect::StartTunnel` — the row it came from may be gone by
/// then, so re-reading the selection at confirmation time would launch a
/// different tunnel than the one that was previewed. The context is kept for
/// the same reason: it is the configuration whose argv the preview showed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TunnelLaunchPreview {
    context: GcloudContext,
    plan: crate::app::TunnelLaunchPlan,
}

impl TunnelLaunchPreview {
    pub fn context(&self) -> &GcloudContext {
        &self.context
    }

    pub fn plan(&self) -> &crate::app::TunnelLaunchPlan {
        &self.plan
    }
}

/// Explains why a marked row was excluded from a batch action. `Ineligible`
/// carries the row's pre-rendered status label (see
/// [`resource_status_label`]), so the reason survives without the UI needing
/// the service's own status type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SkipReason {
    Ineligible(String),
    /// The row's status allows the action but its availability does not: a
    /// zonal (non-HA) Cloud SQL instance targeted by `Failover`. Reporting the
    /// status here would name a state that reads as eligible, so the skip
    /// needs its own reason.
    RequiresHighAvailability,
}

/// The acknowledgement required before a batch action can be confirmed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BatchAck {
    Simple,
    CountRequired { typed: String },
}

impl BatchAck {
    /// Returns true when the acknowledgement is satisfied for the given number
    /// of eligible targets. `Simple` is always satisfied; `CountRequired`
    /// requires the typed number to exactly match the eligible count (and at
    /// least one eligible target to exist).
    pub fn is_satisfied(&self, eligible_count: usize) -> bool {
        match self {
            Self::Simple => true,
            Self::CountRequired { typed } => {
                typed.parse::<usize>() == Ok(eligible_count) && eligible_count > 0
            }
        }
    }

    /// Appends an ascii digit to a `CountRequired` acknowledgement; a no-op for
    /// `Simple` or non-digit input.
    fn append_digit(&mut self, value: char) {
        if let Self::CountRequired { typed } = self
            && value.is_ascii_digit()
        {
            typed.push(value);
        }
    }

    /// Removes the last typed character from a `CountRequired` acknowledgement;
    /// a no-op for `Simple`.
    fn backspace(&mut self) {
        if let Self::CountRequired { typed } = self {
            typed.pop();
        }
    }
}

/// Pending confirmation for an action targeting the full marked set of a
/// service's rows, split into rows eligible for the action and those skipped
/// (with a reason) because they were not.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BatchActionConfirmation {
    action: BatchAction,
    context: GcloudContext,
    eligible: Vec<BatchRowRef>,
    skipped: Vec<(BatchRowRef, SkipReason)>,
    ack: BatchAck,
    batch_confirm_scroll: u16,
}

impl BatchActionConfirmation {
    pub fn action(&self) -> BatchAction {
        self.action
    }

    pub fn context(&self) -> &GcloudContext {
        &self.context
    }

    pub fn eligible(&self) -> &[BatchRowRef] {
        &self.eligible
    }

    pub fn skipped(&self) -> &[(BatchRowRef, SkipReason)] {
        &self.skipped
    }

    pub fn ack(&self) -> &BatchAck {
        &self.ack
    }

    /// Vertical scroll offset applied when rendering the (potentially
    /// overflowing) full target list.
    pub fn batch_confirm_scroll(&self) -> u16 {
        self.batch_confirm_scroll
    }

    fn scroll_down(&mut self) {
        self.batch_confirm_scroll = self.batch_confirm_scroll.saturating_add(DETAIL_PAGE_SIZE);
    }

    fn scroll_up(&mut self) {
        self.batch_confirm_scroll = self.batch_confirm_scroll.saturating_sub(DETAIL_PAGE_SIZE);
    }
}

/// A plugin's argv, waiting to be read and confirmed.
///
/// It carries the **argv**, not a rendered line — unlike [`ResourceCommandPreview`],
/// which carries a rendered line precisely because it never executes. This one
/// does, so it must hold the thing that runs and render from it, keeping what
/// the operator reads and what reaches the process the same object.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginConfirmation {
    label: String,
    argv: Vec<String>,
    ack: BatchAck,
}

impl PluginConfirmation {
    /// The three things confirming needs, rather than the whole [`Offer`](crate::app::state::plugins::Offer): an
    /// offer no longer necessarily carries an argv, and the argv a plugin with
    /// inputs runs is composed from the values the prompt collected rather than
    /// read off the offer. Taking the pieces keeps this type unable to be built
    /// from an offer that has nothing to run.
    pub fn new(label: String, argv: Vec<String>, dangerous: bool) -> Self {
        // `CountRequired` with a target of 1: a dangerous plugin is one command
        // against one row, so the digit the operator types is `1`. Reusing the
        // batch shape keeps one "type something to prove you read it"
        // interaction in the app rather than two.
        let ack = if dangerous {
            BatchAck::CountRequired {
                typed: String::new(),
            }
        } else {
            BatchAck::Simple
        };
        Self { label, argv, ack }
    }

    pub fn label(&self) -> &str {
        &self.label
    }

    pub fn argv(&self) -> &[String] {
        &self.argv
    }

    /// The operator-facing line, rendered from the argv that will actually run.
    pub fn command_preview(&self) -> String {
        gcloud_command_line(&self.argv)
    }

    pub fn ack(&self) -> &BatchAck {
        &self.ack
    }

    /// A dangerous plugin needs the typed `1`; an ordinary one is satisfied by
    /// the keypress alone.
    ///
    /// It delegates to [`BatchAck::is_satisfied`] with an eligible count of 1,
    /// which parses what was typed rather than comparing characters — so `"01"`
    /// and `"001"` satisfy it too. That is the batch modal's behaviour for a
    /// one-row batch, and this reuses it deliberately rather than adding a
    /// second, stricter rule for the same gesture.
    pub fn is_ack_satisfied(&self) -> bool {
        self.ack.is_satisfied(1)
    }

    pub(super) fn ack_append_digit(&mut self, value: char) {
        self.ack.append_digit(value);
    }

    pub(super) fn ack_backspace(&mut self) {
        self.ack.backspace();
    }
}

/// One field of the plugin prompt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptField {
    pub name: String,
    pub draft: Draft,
}

/// What the operator is editing.
///
/// A dropdown holds an index into its declared choices rather than free text,
/// so a value the plugin never offered is unrepresentable — the same move
/// [`PluginInputKind::Dropdown`] makes at load, kept rather than flattened back
/// into a string the moment the operator touches it.
///
/// A text draft carries its own [`TextRule`] rather than the prompt keeping the
/// input's kind beside it, so a dropdown with a numeric rule is unrepresentable
/// for the same reason: the two pieces cannot be paired wrongly if only one of
/// them exists. It is also what makes the rule re-askable — the keymap gate,
/// the reducer's re-check and the modal's key line each ask
/// [`PluginPrompt::is_satisfied`] independently, and a rule derived once at
/// construction and thrown away could not answer all three.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Draft {
    Text {
        text: String,
        rule: TextRule,
    },
    Choice {
        choices: Vec<String>,
        selected: usize,
    },
    /// A `bool`, held as the two states it has rather than as a two-item
    /// `Choice`. A `Choice` of `["true", "false"]` would render and
    /// cycle identically and cost nothing to write, but it makes a bool with a
    /// third choice representable — the state the dropdown design went out of
    /// its way to make impossible, one kind over.
    Toggle {
        on: bool,
    },
}

impl Draft {
    fn value(&self) -> String {
        match self {
            Self::Text { text, .. } => text.clone(),
            // Total by construction: `choices` is non-empty (validated at
            // load) and `selected` is only ever moved modulo its length.
            Self::Choice { choices, selected } => choices[*selected].clone(),
            // The text a declared default of the same value is normalised to
            // (`config::plugins::default_for`), so what the prompt sends and
            // what a declaration sends cannot come to differ.
            Self::Toggle { on } => on.to_string(),
        }
    }

    /// Whether this draft may be submitted as it stands.
    ///
    /// A dropdown always may: it holds an index into a non-empty list, so it is
    /// never empty and there is no text to misparse.
    fn is_acceptable(&self) -> bool {
        match self {
            Self::Text { text, rule } => rule.accepts(text),
            // Both total for the same reason: neither holds text, so there is
            // nothing to be empty and nothing to misparse.
            Self::Choice { .. } | Self::Toggle { .. } => true,
        }
    }
}

/// The values a plugin's templates ask the operator for, collected before the
/// confirmation is composed.
///
/// Holds the row's fields rather than re-reading the selection on submit: the
/// modal owns the keyboard, so the selection cannot move under it, and
/// snapshotting is what makes "the argv the operator approves is the argv that
/// runs" exact rather than incidental.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginPrompt {
    plugin_id: PluginId,
    label: String,
    dangerous: bool,
    fields: Vec<PromptField>,
    focus: usize,
    row_fields: Vec<(JumpField, String)>,
}

impl PluginPrompt {
    pub fn new(
        plugin_id: PluginId,
        label: String,
        dangerous: bool,
        inputs: Vec<PluginInput>,
        row_fields: Vec<(JumpField, String)>,
    ) -> Self {
        let fields = inputs
            .into_iter()
            .map(|input| {
                // Every kind has its own arm rather than a wildcard: the rule
                // a field carries is the whole of what this slice adds, and a
                // wildcard is what would silently give a kind admitted later
                // the rule of the kind admitted before it.
                let draft = match input.kind {
                    PluginInputKind::Dropdown { choices } => {
                        // A declared default is one of the choices (validated
                        // at load); the fallback is the first, so a dropdown is
                        // never empty and never unsatisfied.
                        let selected = input
                            .default
                            .as_ref()
                            .and_then(|value| choices.iter().position(|c| c == value))
                            .unwrap_or(0);
                        Draft::Choice { choices, selected }
                    }
                    // `Bool` now has the arm this comment used to reserve
                    // for it. It contributes the *text* `true`/`false` like
                    // every other kind rather than a flag's presence: presence
                    // would make argv length depend on an answer, which the
                    // confirmation's exact-argv promise would have to account
                    // for, and it is not what a declared default is already
                    // normalised into.
                    //
                    // No default means `false` — the answer that adds nothing,
                    // so a field tabbed past cannot opt anyone into anything.
                    // Read from the loader's text rather than a parse: `true`
                    // is the only spelling `default_for` produces.
                    PluginInputKind::Bool => Draft::Toggle {
                        on: input.default.as_deref() == Some("true"),
                    },
                    PluginInputKind::String => Draft::Text {
                        text: input.default.unwrap_or_default(),
                        rule: TextRule::Free,
                    },
                    PluginInputKind::Number => Draft::Text {
                        text: input.default.unwrap_or_default(),
                        rule: TextRule::Finite,
                    },
                    PluginInputKind::Integer => Draft::Text {
                        text: input.default.unwrap_or_default(),
                        rule: TextRule::Whole,
                    },
                };
                PromptField {
                    name: input.name,
                    draft,
                }
            })
            .collect();
        Self {
            plugin_id,
            label,
            dangerous,
            fields,
            focus: 0,
            row_fields,
        }
    }

    pub fn plugin_id(&self) -> PluginId {
        self.plugin_id
    }

    pub fn label(&self) -> &str {
        &self.label
    }

    pub fn dangerous(&self) -> bool {
        self.dangerous
    }

    pub fn fields(&self) -> &[PromptField] {
        &self.fields
    }

    pub fn focus(&self) -> usize {
        self.focus
    }

    pub fn row_fields(&self) -> &[(JumpField, String)] {
        &self.row_fields
    }

    /// Every field's value, in declaration order — what `plugin_argv` renders
    /// against.
    pub fn values(&self) -> Vec<(String, String)> {
        self.fields
            .iter()
            .map(|field| (field.name.clone(), field.draft.value()))
            .collect()
    }

    /// Whether `Submit` may be dispatched at all.
    ///
    /// An empty field refuses for the reason an empty row field does in
    /// `Template::render`: an argv element built from a blank is not the
    /// command the operator meant. A numeric field refuses text its declared
    /// default would have been refused for — see [`TextRule`], which states
    /// both rules once.
    ///
    /// A refusal here is a gate and not a discard: `SubmitPluginPrompt` takes
    /// the prompt only once nothing can fail, so an operator who mistypes one
    /// field corrects it with the rest of their typing intact.
    pub fn is_satisfied(&self) -> bool {
        self.fields.iter().all(|field| field.draft.is_acceptable())
    }

    fn focused(&mut self) -> &mut Draft {
        // Total: `new` is only ever called with a non-empty `inputs` and
        // `focus` is only ever moved modulo the field count, so there is always
        // a focused field.
        &mut self.fields[self.focus].draft
    }

    /// Moves focus, wrapping.
    ///
    /// `% self.fields.len()` is safe without an emptiness guard because a
    /// prompt is only ever built from a non-empty `inputs`: `Command::
    /// NeedsValues` is constructed only when `prompted_inputs` returned at
    /// least one, and that arm is the only caller of `show_plugin_prompt`. A
    /// runtime guard here would be dead code that hid the invariant instead of
    /// stating it.
    pub fn focus_next(&mut self) {
        self.focus = (self.focus + 1) % self.fields.len();
    }

    pub fn focus_previous(&mut self) {
        self.focus = (self.focus + self.fields.len() - 1) % self.fields.len();
    }

    pub fn append(&mut self, value: char) {
        if let Draft::Text { text, .. } = self.focused() {
            text.push(value);
        }
    }

    pub fn backspace(&mut self) {
        if let Draft::Text { text, .. } = self.focused() {
            text.pop();
        }
    }

    pub fn cycle_choice(&mut self, forward: bool) {
        match self.focused() {
            Draft::Choice { choices, selected } => {
                let len = choices.len();
                *selected = if forward {
                    (*selected + 1) % len
                } else {
                    (*selected + len - 1) % len
                };
            }
            // Direction is meaningless with two states, but the key that moves
            // a dropdown has to move a toggle too: a field the operator cannot
            // change with the key the modal advertises reads as broken.
            Draft::Toggle { on } => *on = !*on,
            Draft::Text { .. } => {}
        }
    }
}

/// Per-target outcome of an in-flight batch action.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BatchTargetStatus {
    Pending,
    Succeeded,
    Failed(String),
}

/// One row targeted by a running batch action, tracked by identity
/// (`name` + `location`) so results can be attributed independently of row
/// order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BatchTarget {
    pub name: String,
    pub location: String,
    pub status: BatchTargetStatus,
    /// Id of the operations-log entry seeded for this target when the batch was
    /// confirmed. A cancelled batch has to resolve the entries it stranded
    /// `Pending`, and it must resolve *those* entries: a `(kind, target)` lookup
    /// would happily settle an unrelated single-row action's pending entry for
    /// the same resource. The id is the only handle that cannot mistake one
    /// operation for another.
    pub operation_id: u64,
}

/// Progress state for a batch action currently being applied across a set of
/// rows. Seeded when the batch is confirmed and torn down on a context change;
/// per-target status is resolved as each action result lands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BatchWorkflow {
    action: BatchAction,
    context: GcloudContext,
    targets: Vec<BatchTarget>,
}

impl BatchWorkflow {
    pub fn action(&self) -> BatchAction {
        self.action
    }

    pub fn context(&self) -> &GcloudContext {
        &self.context
    }

    pub fn targets(&self) -> &[BatchTarget] {
        &self.targets
    }

    pub fn ok_count(&self) -> usize {
        self.targets
            .iter()
            .filter(|target| target.status == BatchTargetStatus::Succeeded)
            .count()
    }

    pub fn fail_count(&self) -> usize {
        self.targets
            .iter()
            .filter(|target| matches!(target.status, BatchTargetStatus::Failed(_)))
            .count()
    }

    pub fn pending_count(&self) -> usize {
        self.targets
            .iter()
            .filter(|target| target.status == BatchTargetStatus::Pending)
            .count()
    }

    /// Returns true once no target remains `Pending`.
    pub fn is_complete(&self) -> bool {
        self.pending_count() == 0
    }
}

/// Distinguishes whether the currently applied Cloud Storage filter is
/// evaluated purely against already-loaded rows (`Local`) or additionally
/// narrowed the server-side object listing via a safe-prefix reload
/// (`Remote`). Bucket-list filtering is always `Local`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilterScope {
    Local,
    Remote,
}

impl FilterScope {
    pub fn label(self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::Remote => "remote",
        }
    }
}

/// Small fixed cap on how many contexts' resource rows we retain when
/// switching away from them. Bounds memory usage while comfortably covering
/// realistic multi-project/region hopping within a session.
const CONTEXT_CACHE_CAP: usize = 8;

/// Row lists captured for a single [`GcloudContext`] when switching away
/// from it. Only row lists are cached — not detail panes, filters, sort, or
/// marks, all of which are cheap to rebuild and context-specific.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct ContextSnapshot {
    compute: Option<Vec<ComputeInstance>>,
    gke: Option<Vec<GkeCluster>>,
    cloud_sql: Option<Vec<CloudSqlInstance>>,
    cloud_run: Option<Vec<CloudRunService>>,
    storage_buckets: Option<Vec<StorageBucket>>,
    secrets: Option<Vec<Secret>>,
    functions: Option<Vec<CloudFunction>>,
    disks: Option<Vec<PersistentDisk>>,
    pubsub_topics: Option<Vec<PubSubTopic>>,
    pubsub_subscriptions: Option<Vec<PubSubSubscription>>,
    vpc_networks: Option<Vec<VpcNetwork>>,
    vpc_subnets: Option<Vec<VpcSubnet>>,
    firewall_rules: Option<Vec<FirewallRule>>,
    bigquery_datasets: Option<Vec<BigQueryDataset>>,
    service_accounts: Option<Vec<ServiceAccount>>,
    policy_members: Option<Vec<IamPolicyMember>>,
}

impl ContextSnapshot {
    fn is_empty(&self) -> bool {
        self.compute.is_none()
            && self.gke.is_none()
            && self.cloud_sql.is_none()
            && self.cloud_run.is_none()
            && self.storage_buckets.is_none()
            && self.secrets.is_none()
            && self.functions.is_none()
            && self.disks.is_none()
            && self.pubsub_topics.is_none()
            && self.pubsub_subscriptions.is_none()
            && self.vpc_networks.is_none()
            && self.vpc_subnets.is_none()
            && self.firewall_rules.is_none()
            && self.bigquery_datasets.is_none()
            && self.service_accounts.is_none()
            && self.policy_members.is_none()
    }
}

/// Bounded least-recently-used cache of per-context resource row snapshots,
/// keyed by [`GcloudContext`] (which has no `Hash` impl, so this uses a
/// `Vec` with linear lookup rather than a hash map). `entries` is ordered
/// least-recently-used first: index `0` is the next eviction target.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct ContextResourceCache {
    entries: Vec<(GcloudContext, ContextSnapshot)>,
}

impl ContextResourceCache {
    /// Returns a mutable reference to the snapshot for `context`, moving it
    /// to the most-recently-used position. Inserts a fresh default snapshot
    /// if none exists yet, evicting the least-recently-used entry if that
    /// insertion pushes the cache past [`CONTEXT_CACHE_CAP`].
    fn entry_mut(&mut self, context: &GcloudContext) -> &mut ContextSnapshot {
        if let Some(pos) = self.entries.iter().position(|(ctx, _)| ctx == context) {
            let entry = self.entries.remove(pos);
            self.entries.push(entry);
        } else {
            self.entries
                .push((context.clone(), ContextSnapshot::default()));
            while self.entries.len() > CONTEXT_CACHE_CAP {
                self.entries.remove(0);
            }
        }
        &mut self
            .entries
            .last_mut()
            .expect("just inserted or moved an entry")
            .1
    }

    /// Removes and returns the snapshot for `context`, if cached.
    fn take(&mut self, context: &GcloudContext) -> Option<ContextSnapshot> {
        let pos = self.entries.iter().position(|(ctx, _)| ctx == context)?;
        Some(self.entries.remove(pos).1)
    }
}

/// In-flight / failure channel for a collection's list fetch, mirroring the
/// richness of `LoggingStatus` (src/app/state/logging.rs). This is separate
/// from the existing per-collection error channels (`detail_error`,
/// `last_problem`, `last_diagnostic`, ...): those describe detail-pane and
/// diagnostic-banner state, while `ResourceLoadPhase` describes only the
/// list-fetch's own in-flight/failure state.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum ResourceLoadPhase {
    #[default]
    Idle,
    Loading {
        started_at: std::time::Instant,
    },
    Failed {
        message: String,
        /// The context this failure belongs to.
        ///
        /// A failure clears the collection's `context`, so the phase alone
        /// cannot say whether the backend that failed is the one the user is
        /// looking at now: a configuration switch leaves the `Failed` stamp
        /// standing (deliberately, see `mark_stale_and_clear`) and it would
        /// otherwise read as a failure of the new configuration, which has not
        /// been tried.
        ///
        /// Carried inside the variant rather than in a field beside
        /// `load_phase` so the two cannot drift apart: a phase-mutating method
        /// cannot install `Failed` without naming the context it failed under,
        /// and every phase that is not `Failed` has no context to go stale.
        context: GcloudContext,
    },
}

/// Identity of one in-flight list fetch: the service it targets plus the
/// sequence number minted for it by [`ResourcesState::begin_load`].
///
/// Plain `Copy` data with no borrows, so the runtime can move it into a
/// spawned task and hand it back — see [`ResourcesState::abandon_load`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResourceLoadTicket {
    kind: ResourceServiceKind,
    sequence: u64,
}

impl ResourceLoadTicket {
    /// The service this fetch targets. Exposed so the runtime's drop site can
    /// tell whether its `auto_loaded_scope` record belongs to the abandoned
    /// fetch's service; `sequence` stays private, as identity is only ever
    /// compared inside [`ResourcesState`].
    pub fn kind(&self) -> ResourceServiceKind {
        self.kind
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceCollectionState<Row, Detail, SortField, DetailTab> {
    rows: Vec<Row>,
    selected_visible: usize,
    context: Option<GcloudContext>,
    stale: bool,
    load_phase: ResourceLoadPhase,
    /// Sequence number of the load that produced the current `load_phase`,
    /// so a late resolution can tell "still mine" from "superseded".
    load_sequence: u64,
    /// Sequence number of the last load whose *result* was applied, and `None`
    /// until one has been.
    ///
    /// Distinct from `load_sequence`, which is stamped when a fetch *starts*.
    /// The two diverge on every path that resolves a load without rows: a fetch
    /// in flight, a failure, and — the case that motivated this field — a load
    /// the runtime's generation guard abandoned, which returns the phase to
    /// `Idle` under a sequence nothing ever landed. A consumer keyed on
    /// `load_sequence` reads that as a fresh result and re-records the previous
    /// rows under the present moment.
    landed_sequence: Option<u64>,
    filter_input_active: bool,
    filter_draft: String,
    /// The applied pattern and the filter it compiled to.
    ///
    /// A cache, and the only thing that keeps it coherent is that no caller
    /// can write the source without going through
    /// [`CompiledResourceFilter::new`], which recompiles. Private on purpose —
    /// and private here means private to this module *and* its descendants,
    /// which is where the per-service modules write it from. Read it with
    /// [`ResourcesState::filter_applied_for`].
    filter_applied: CompiledResourceFilter,
    sort_field: SortField,
    selected_column: usize,
    sort_direction: SortDirection,
    view_mode: ResourcesViewMode,
    active_detail_tab: DetailTab,
    detail_scroll: u16,
    detail_target: Option<Row>,
    detail_error_target_name: Option<String>,
    detail: Option<Detail>,
    detail_error: Option<String>,
    last_problem: Option<DiagnosticProblem>,
    last_diagnostic: Option<GcloudDiagnostic>,
    marks: BTreeSet<ResourceRowId>,
    mark_anchor: Option<ResourceRowId>,
}

impl<Row, Detail, SortField, DetailTab> ResourceCollectionState<Row, Detail, SortField, DetailTab> {
    /// The loaded rows for this collection, in load order (unfiltered,
    /// unsorted). Exposed so callers reaching a collection generically
    /// through [`ResourceService::collection`] can inspect its rows without
    /// requiring a service-specific accessor on [`ResourcesState`].
    pub fn rows(&self) -> &[Row] {
        &self.rows
    }

    /// The current in-flight/failure state of this collection's list fetch.
    pub fn load_phase(&self) -> &ResourceLoadPhase {
        &self.load_phase
    }

    /// The sequence number of the last load whose result was applied, and
    /// `None` while none has been.
    ///
    /// Read outside the load machinery by the health sampler, which
    /// uses it as the identity of the *result* currently held: the sampler runs
    /// on every reduction, so it needs to tell "a listing I have already
    /// recorded" from "a fresh one", and the row count cannot do that — a
    /// re-list returning the same count is a real second sample.
    fn landed_sequence(&self) -> Option<u64> {
        self.landed_sequence
    }

    /// Resolves the load phase back to `Idle` unconditionally, for the paths
    /// that know the load is over (a completed fetch, a context clear).
    ///
    /// Together with [`Self::fail_load`] this replaces a general
    /// `set_load_phase(ResourceLoadPhase)`: neither can express `Loading`, so
    /// [`Self::begin_load`] stays the only way to install it and a `Loading`
    /// phase can never carry a stale `load_sequence`.
    fn clear_load_phase(&mut self) {
        self.load_phase = ResourceLoadPhase::Idle;
        // The one place a *result* resolves the phase, which is what makes it
        // the one place `landed_sequence` may advance. `abandon_load` sets the
        // phase directly and deliberately does not come through here.
        self.landed_sequence = Some(self.load_sequence);
    }

    /// Resolves the load phase to `Failed`, recording the context the failure
    /// belongs to inside the variant. See [`Self::clear_load_phase`] for why
    /// this is not a general setter.
    fn fail_load(&mut self, context: GcloudContext, message: String) {
        self.load_phase = ResourceLoadPhase::Failed { message, context };
    }

    /// The context a `Failed` load phase belongs to, and `None` for any other
    /// phase. It cannot disagree with the phase: there is nowhere else for it
    /// to live.
    fn failed_context(&self) -> Option<&GcloudContext> {
        match &self.load_phase {
            ResourceLoadPhase::Failed { context, .. } => Some(context),
            _ => None,
        }
    }

    /// Takes the load phase over for the load identified by `sequence`.
    fn begin_load(&mut self, sequence: u64, started_at: std::time::Instant) {
        self.load_sequence = sequence;
        self.load_phase = ResourceLoadPhase::Loading { started_at };
    }

    /// Resolves the load phase back to `Idle`, but only while it is still the
    /// `Loading` phase that the load identified by `sequence` installed.
    /// Returns whether that load still owned the phase and was resolved.
    fn abandon_load(&mut self, sequence: u64) -> bool {
        if self.load_sequence == sequence
            && matches!(self.load_phase, ResourceLoadPhase::Loading { .. })
        {
            self.load_phase = ResourceLoadPhase::Idle;
            return true;
        }
        false
    }

    fn new(sort_field: SortField, detail_tab: DetailTab) -> Self {
        Self {
            rows: Vec::new(),
            selected_visible: 0,
            context: None,
            stale: false,
            load_phase: ResourceLoadPhase::default(),
            load_sequence: 0,
            landed_sequence: None,
            filter_input_active: false,
            filter_draft: String::new(),
            filter_applied: CompiledResourceFilter::default(),
            sort_field,
            selected_column: 0,
            sort_direction: SortDirection::Ascending,
            view_mode: ResourcesViewMode::List,
            active_detail_tab: detail_tab,
            detail_scroll: 0,
            detail_target: None,
            detail_error_target_name: None,
            detail: None,
            detail_error: None,
            last_problem: None,
            last_diagnostic: None,
            marks: BTreeSet::new(),
            mark_anchor: None,
        }
    }
}

/// The text every [`ResourceService::sample_row`] carries in a field that
/// service filters by, and the needle a generic assertion filters that row
/// with.
///
/// One shared constant rather than a per-service one: the walk that uses it
/// (`tests/matches_filter_walk_test.rs`) is a single loop over services, and
/// a needle each service chose for itself would let a service pick a needle
/// its own rows always match. The alphabet is deliberately letters only —
/// it has to be embeddable verbatim in a Compute instance name, a BigQuery
/// dataset id and an IAM member string, whose legal characters do not
/// overlap beyond that, and it must carry no regex meaning under the filter
/// grammar's unprefixed regex default.
pub const SAMPLE_ROW_NEEDLE: &str = "nuvemsampleneedle";

/// Unifies the generic per-service resource collections (Compute, GKE,
/// Cloud SQL, Cloud Run, and the Cloud Storage bucket list) behind a single
/// interface, so generic UI/reducer code can operate on "the active service"
/// without a per-service match.
/// Implemented by zero-variant marker types ([`Compute`], [`Gke`],
/// [`CloudSql`], [`CloudRun`], [`Storage`]) rather than instantiated.
pub trait ResourceService {
    type Row;
    type Detail;
    type SortField: Copy;
    type DetailTab: Copy;
    const KIND: ResourceServiceKind;
    fn collection(
        state: &ResourcesState,
    ) -> &ResourceCollectionState<Self::Row, Self::Detail, Self::SortField, Self::DetailTab>;
    fn collection_mut(
        state: &mut ResourcesState,
    ) -> &mut ResourceCollectionState<Self::Row, Self::Detail, Self::SortField, Self::DetailTab>;
    /// Whether `row` survives an already-compiled filter. Compiling is the
    /// caller's job so a `visible_*` accessor pays for it once per call rather
    /// than once per row.
    fn matches_filter(row: &Self::Row, filter: &CompiledFilter) -> bool;
    /// Whether `row` survives a *narrowing* the jump layer applied to this
    /// list — a set computed from another resource, rather than text the user
    /// typed.
    ///
    /// Intersected with [`matches_filter`](Self::matches_filter) rather than
    /// replacing it, so a narrowed table still answers `/`. The two are
    /// different kinds of thing and the user can see and clear each: the
    /// filter is their words, the narrowing is the jump they followed.
    ///
    /// Defaulted to "no narrowing", which is the one place in this trait a
    /// default is right. `sample_row` has none because every service must
    /// answer it and a missing answer is a wrong one; here a service with no
    /// narrowing genuinely has nothing to say, and inheriting `true` is
    /// correct rather than silent.
    fn matches_narrowing(_row: &Self::Row, _state: &ResourcesState) -> bool {
        true
    }
    /// One row of this service, carrying [`SAMPLE_ROW_NEEDLE`] in a field
    /// [`Self::matches_filter`] searches and carrying no other row's text.
    ///
    /// This exists for one caller, and it is a test:
    /// `tests/matches_filter_walk_test.rs` asserts every service's
    /// `matches_filter` against a row it can build generically. Without it
    /// there is no generic way to reach `matches_filter` at all — its
    /// argument is `Self::Row`, a different type per service — which is what
    /// let VPC networks' implementation ship with no caller and survive
    /// mutation as both `true` and `false`.
    ///
    /// It is not behind `cfg(test)` because it could not be: the walk is an
    /// integration test, which compiles against this crate as a dependency
    /// with `cfg(test)` off, and a `test-support` feature would compile the
    /// library a second time under a different cfg — doubling what the
    /// mutation gate builds to hide sixteen struct literals. The cost of
    /// carrying them in the binary is a few hundred bytes of `.rodata` that
    /// nothing calls; the cost of not carrying them is the class of defect
    /// this trait already shipped once.
    ///
    /// The row is a fixture, not a capture: it needs to be filterable and
    /// distinguishable, not to resemble any real resource. What it must not
    /// be is empty of the needle, or a carrier of another service's, and the
    /// walk fails loudly on either.
    fn sample_row() -> Self::Row;
    fn compare(
        left: &Self::Row,
        right: &Self::Row,
        sort_field: Self::SortField,
        direction: SortDirection,
    ) -> std::cmp::Ordering;
    /// The row's mark identity: stable across refreshes, sorting and
    /// filtering, and unique within the service's row list.
    fn row_id(row: &Self::Row) -> ResourceRowId;
}

/// The shape of a service's [`ResourceRowId`], as a fact a caller with no row
/// in hand can ask about.
///
/// A jump rule is validated when the config file loads, long before any list
/// has rows, so "can a rule name a row of this service, and does it need a
/// location to do it" has to be answerable from the kind alone. Deriving it
/// from `config::jumps::location_field` was the first attempt and was wrong
/// twice — BigQuery declares a `{location}` field and deliberately keys rows by
/// dataset id alone, and the IAM policy table keys rows by a *role*, which is
/// not a location at all. Both would have shipped as jumps that resolve, land,
/// and never match a row.
///
/// [`row_identity`] is therefore its own table, and
/// `every_service_keys_rows_by_location_exactly_when_it_has_one` walks all
/// sixteen services comparing it against what `row_id` actually builds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowIdentity {
    /// The name alone; the location half of the id is always empty. A global
    /// resource — a secret, a topic, a VPC network.
    Name,
    /// Name plus a real zone/region/location, both halves renderable from a
    /// rule's templates.
    NameAndLocation,
    /// Name plus a second half that is *not* a location and that no template
    /// could render — the IAM policy table's role-and-condition. Such a
    /// service is a legal jump `from` and never a legal list `to`.
    Opaque,
}

/// How `kind` identifies one of its rows. See [`RowIdentity`].
pub fn row_identity(kind: ResourceServiceKind) -> RowIdentity {
    match kind {
        ResourceServiceKind::ComputeInstances
        | ResourceServiceKind::GkeClusters
        | ResourceServiceKind::CloudSql
        | ResourceServiceKind::CloudRunServices
        | ResourceServiceKind::CloudStorage
        | ResourceServiceKind::CloudFunctions
        | ResourceServiceKind::PersistentDisks
        | ResourceServiceKind::VpcSubnets => RowIdentity::NameAndLocation,
        ResourceServiceKind::SecretManagerSecrets
        | ResourceServiceKind::PubSubTopics
        | ResourceServiceKind::PubSubSubscriptions
        | ResourceServiceKind::IamServiceAccounts
        | ResourceServiceKind::VpcNetworks
        | ResourceServiceKind::FirewallRules
        // A dataset id is unique within the project the table lists, so
        // `BigQuery::row_id` folds no location in even though a dataset has
        // one and a rule may read it as a `{location}` field.
        | ResourceServiceKind::BigQueryDatasets => RowIdentity::Name,
        ResourceServiceKind::IamPolicy => RowIdentity::Opaque,
    }
}

/// Why a parsed label filter cannot narrow anything on `kind`, if it cannot.
///
/// Mirrors the two gates the pushdown applies to the same selectors
/// (`crate::services::resources::list_filter`): a service with no label field,
/// and a selector that cannot be expressed as a gcloud clause. Both make the
/// pushdown return `None`, leaving the table unfiltered, so the user is told
/// why instead of reading it as "every row has that label".
fn label_filter_error(kind: ResourceServiceKind, selectors: &[LabelSelector]) -> Option<String> {
    if label_filter_field(kind).is_none() {
        return Some(format!(
            "label filtering is not supported for {}",
            kind.label()
        ));
    }
    (!selectors.iter().all(is_expressible_label_selector))
        .then(|| "label selector is not expressible as a gcloud filter".to_string())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourcesState {
    active_service: ResourceServiceKind,
    /// The instance the firewall rules table has been narrowed to, if a jump
    /// narrowed it.
    ///
    /// The instance's *scope* rather than the matching rule names, so the
    /// answer is recomputed from whatever rows the table currently holds. A
    /// stored set of names would go stale the moment the list reloaded, and it
    /// would go stale by *omitting* rules — a rule created since the jump
    /// would be missing from a list an operator is reading to find out what
    /// governs a workload. Recomputing cannot omit.
    firewall_narrowed_to: Option<FirewallInstanceScope>,
    compute: ResourceCollectionState<
        ComputeInstance,
        ComputeInstanceDetail,
        ComputeInstanceSortField,
        ComputeDetailTab,
    >,
    gke: ResourceCollectionState<GkeCluster, GkeClusterDetail, GkeSortField, GkeDetailTab>,
    cloud_sql: ResourceCollectionState<
        CloudSqlInstance,
        CloudSqlInstanceDetail,
        CloudSqlInstanceSortField,
        CloudSqlDetailTab,
    >,
    cloud_run: ResourceCollectionState<
        CloudRunService,
        CloudRunServiceDetail,
        CloudRunSortField,
        CloudRunDetailTab,
    >,
    cloud_run_revision: Option<CloudRunRevisionView>,
    selected_revision: usize,
    secrets: ResourceCollectionState<Secret, SecretDetail, SecretSortField, SecretDetailTab>,
    /// Cloud Functions is list-only in this slice: no describe, no detail
    /// pane, hence the unit `Detail` and `DetailTab` (see
    /// [`CloudFunctions`]).
    functions: ResourceCollectionState<CloudFunction, (), CloudFunctionSortField, ()>,
    /// Persistent disks is list-only in this slice, for the same reason
    /// Cloud Functions is: no describe, no detail pane, hence the unit
    /// `Detail` and `DetailTab` (see [`PersistentDisks`]).
    disks: ResourceCollectionState<PersistentDisk, (), DiskSortField, ()>,
    /// Pub/Sub topics are list-only: no describe, no detail pane, hence the
    /// unit `Detail` and `DetailTab` (see [`PubSubTopics`]).
    pubsub_topics: ResourceCollectionState<PubSubTopic, (), PubSubTopicSortField, ()>,
    /// Pub/Sub subscriptions are a sibling top-level table, not a drill-down
    /// under `pubsub_topics`: one list call returns every subscription in the
    /// project, so this collection is loaded independently and holds rows
    /// whose parent topic may not be in `pubsub_topics` at all — including
    /// subscriptions whose topic has been deleted. List-only for the same
    /// reason topics are (see [`PubSubSubscriptions`]).
    pubsub_subscriptions:
        ResourceCollectionState<PubSubSubscription, (), PubSubSubscriptionSortField, ()>,
    /// VPC networks are list-only, for the same reason Pub/Sub topics are: no
    /// describe worth making, no detail pane, hence the unit `Detail` and
    /// `DetailTab` (see [`VpcNetworks`]). A network's subnets are *not* held
    /// here — they are a flat table of their own, because the API lists them
    /// project-wide rather than per network.
    vpc_networks: ResourceCollectionState<VpcNetwork, (), VpcNetworkSortField, ()>,
    /// VPC subnets are a flat sibling of `vpc_networks`, not a drill-down
    /// under it: one project-wide call returns every region's subnets, so this
    /// collection is loaded independently and holds rows whose parent network
    /// may not be in `vpc_networks` at all. List-only for the same reason
    /// networks are (see [`VpcSubnets`]).
    vpc_subnets: ResourceCollectionState<VpcSubnet, (), VpcSubnetSortField, ()>,
    /// Firewall rules are a flat sibling of the other two VPC tables, not a
    /// drill-down under a network: `firewall-rules list` is project-wide, and
    /// a rule is global to its project. List-only for the same reason (see
    /// [`FirewallRules`]).
    firewall_rules: ResourceCollectionState<FirewallRule, (), FirewallSortField, ()>,
    /// BigQuery datasets are a flat top-level table, list-only: no describe,
    /// no detail pane, hence the unit `Detail` and `DetailTab` (see
    /// [`BigQuery`]). A dataset's tables and their schemas are separate
    /// resources with their own calls and are not held here.
    bigquery_datasets: ResourceCollectionState<BigQueryDataset, (), BigQuerySortField, ()>,
    /// Which BigQuery list is on screen, and the drill scope the tables view
    /// is inside. Held beside `bigquery_datasets` rather than inside it, the
    /// way `StorageState` holds its object browser beside its bucket
    /// collection: the tables list is a second list under the same service
    /// kind, not a detail pane over the first.
    bigquery: BigQueryDrilldownState,
    /// Service accounts are list-only in this slice: no describe, no detail
    /// pane, hence the unit `Detail` and `DetailTab` (see [`IamServiceAccounts`]).
    service_accounts: ResourceCollectionState<ServiceAccount, (), ServiceAccountSortField, ()>,
    /// The project's IAM policy, one row per (role, member) pair. List-only:
    /// no describe, no detail pane, hence the unit `Detail` and `DetailTab`
    /// (see [`IamPolicy`]).
    policy_members: ResourceCollectionState<IamPolicyMember, (), IamPolicySortField, ()>,
    /// Why the open secret's versions could not be listed, when that fetch
    /// failed. It lives beside the detail rather than on it because the
    /// versions fetch is its own call: `secrets versions list` needs an IAM
    /// permission distinct from `secrets list`, so it can fail while the
    /// describe that filled the pane succeeded. Without this, an empty
    /// `detail.versions` renders identically whether the secret has no
    /// versions or the operator was not allowed to see them.
    secret_versions_error: Option<String>,
    /// Cursor over the copyable fields of the detail pane currently on screen,
    /// as an index into `DetailBody::fields()` (not into `lines`, and not a
    /// display row). `None` means no field is selected. It is cleared at every
    /// site that changes which fields are on screen, so it can never point at a
    /// field the operator is no longer looking at.
    detail_field_index: Option<usize>,
    storage: StorageState,
    context_cache: ContextResourceCache,
    action_confirmation: Option<ResourceActionConfirmation>,
    batch_confirmation: Option<BatchActionConfirmation>,
    batch_workflow: Option<BatchWorkflow>,
    command_preview: Option<ResourceCommandPreview>,
    ssh_launch_preview: Option<SshLaunchPreview>,
    tunnel_launch_preview: Option<TunnelLaunchPreview>,
    k9s_launch_confirmation: Option<K9sLaunchConfirmation>,
    plugin_confirmation: Option<PluginConfirmation>,
    plugin_prompt: Option<PluginPrompt>,
    wide_columns: bool,
    /// Validated `[columns.<service>]` settings. Empty until configuration is
    /// installed, which is what keeps every default table byte-for-byte what
    /// it was before this feature existed.
    column_settings: ColumnSettings,
    /// Source of [`ResourceLoadTicket`] sequence numbers, shared across every
    /// service so no two in-flight loads share an identity. It increments by
    /// one per load and wraps at `u64::MAX`, so it is monotonic across any
    /// realistic run.
    next_load_sequence: u64,
}

impl Default for ResourcesState {
    fn default() -> Self {
        Self {
            active_service: ResourceServiceKind::ComputeInstances,
            firewall_narrowed_to: None,
            compute: ResourceCollectionState::new(
                ComputeInstanceSortField::Name,
                ComputeDetailTab::Summary,
            ),
            gke: ResourceCollectionState::new(GkeSortField::Name, GkeDetailTab::Summary),
            cloud_sql: ResourceCollectionState::new(
                CloudSqlInstanceSortField::Name,
                CloudSqlDetailTab::Summary,
            ),
            cloud_run: ResourceCollectionState::new(
                CloudRunSortField::Name,
                CloudRunDetailTab::Summary,
            ),
            cloud_run_revision: None,
            selected_revision: 0,
            secrets: ResourceCollectionState::new(SecretSortField::Name, SecretDetailTab::Summary),
            functions: ResourceCollectionState::new(CloudFunctionSortField::Name, ()),
            disks: ResourceCollectionState::new(DiskSortField::Name, ()),
            pubsub_topics: ResourceCollectionState::new(PubSubTopicSortField::Name, ()),
            pubsub_subscriptions: ResourceCollectionState::new(
                PubSubSubscriptionSortField::Name,
                (),
            ),
            vpc_networks: ResourceCollectionState::new(VpcNetworkSortField::Name, ()),
            vpc_subnets: ResourceCollectionState::new(VpcSubnetSortField::Name, ()),
            firewall_rules: ResourceCollectionState::new(FirewallSortField::Name, ()),
            bigquery_datasets: ResourceCollectionState::new(BigQuerySortField::DatasetId, ()),
            bigquery: BigQueryDrilldownState::default(),
            service_accounts: ResourceCollectionState::new(ServiceAccountSortField::Email, ()),
            policy_members: ResourceCollectionState::new(IamPolicySortField::Member, ()),
            secret_versions_error: None,
            detail_field_index: None,
            storage: StorageState::default(),
            context_cache: ContextResourceCache::default(),
            action_confirmation: None,
            batch_confirmation: None,
            batch_workflow: None,
            command_preview: None,
            ssh_launch_preview: None,
            tunnel_launch_preview: None,
            k9s_launch_confirmation: None,
            plugin_confirmation: None,
            plugin_prompt: None,
            wide_columns: false,
            column_settings: ColumnSettings::empty(),
            next_load_sequence: 0,
        }
    }
}

impl ResourcesState {
    pub fn active_service(&self) -> ResourceServiceKind {
        self.active_service
    }

    /// Whether this service's list has ever been loaded for a context — the
    /// context stamp its rows carry, which a load that returned *no* rows sets
    /// just the same.
    ///
    /// Row emptiness deliberately does not answer this: a list that loaded and
    /// came back empty is a list the user has seen, and a resource that was in
    /// it and is not any more really is no longer available. What this
    /// separates is the case where nothing was ever fetched at all.
    pub fn has_loaded_list(&self, kind: ResourceServiceKind) -> bool {
        self.loaded_list_context(kind).is_some()
    }

    /// The context `kind`'s currently held rows were loaded for, and `None`
    /// when nothing has been fetched for them at all.
    ///
    /// The stamp rather than the bare "has it loaded" answer, because a caller
    /// deciding whether to fetch has to know *which* configuration the rows
    /// belong to: rows loaded for a configuration the user has since left are
    /// not rows for the one they are in.
    pub fn loaded_list_context(&self, kind: ResourceServiceKind) -> Option<&GcloudContext> {
        match kind {
            ResourceServiceKind::ComputeInstances => self.compute.context.as_ref(),
            ResourceServiceKind::GkeClusters => self.gke.context.as_ref(),
            ResourceServiceKind::CloudSql => self.cloud_sql.context.as_ref(),
            ResourceServiceKind::CloudRunServices => self.cloud_run.context.as_ref(),
            ResourceServiceKind::SecretManagerSecrets => self.secrets.context.as_ref(),
            ResourceServiceKind::CloudFunctions => self.functions.context.as_ref(),
            ResourceServiceKind::PersistentDisks => self.disks.context.as_ref(),
            ResourceServiceKind::PubSubTopics => self.pubsub_topics.context.as_ref(),
            ResourceServiceKind::VpcNetworks => self.vpc_networks.context.as_ref(),
            ResourceServiceKind::VpcSubnets => self.vpc_subnets.context.as_ref(),
            ResourceServiceKind::FirewallRules => self.firewall_rules.context.as_ref(),
            ResourceServiceKind::PubSubSubscriptions => self.pubsub_subscriptions.context.as_ref(),
            ResourceServiceKind::BigQueryDatasets => self.bigquery_datasets.context.as_ref(),
            ResourceServiceKind::IamServiceAccounts => self.service_accounts.context.as_ref(),
            ResourceServiceKind::IamPolicy => self.policy_members.context.as_ref(),
            ResourceServiceKind::CloudStorage => self.storage.collection.context.as_ref(),
        }
    }

    /// Whether `kind`'s last list fetch failed *for `context`*.
    ///
    /// A `Failed` phase outlives the configuration it was recorded under, so
    /// the phase on its own would report a backend as broken in a
    /// configuration that has never called it.
    pub fn list_load_failed_for(&self, kind: ResourceServiceKind, context: &GcloudContext) -> bool {
        let failed = match kind {
            ResourceServiceKind::ComputeInstances => self.compute.failed_context(),
            ResourceServiceKind::GkeClusters => self.gke.failed_context(),
            ResourceServiceKind::CloudSql => self.cloud_sql.failed_context(),
            ResourceServiceKind::CloudRunServices => self.cloud_run.failed_context(),
            ResourceServiceKind::SecretManagerSecrets => self.secrets.failed_context(),
            ResourceServiceKind::CloudFunctions => self.functions.failed_context(),
            ResourceServiceKind::PersistentDisks => self.disks.failed_context(),
            ResourceServiceKind::PubSubTopics => self.pubsub_topics.failed_context(),
            ResourceServiceKind::VpcNetworks => self.vpc_networks.failed_context(),
            ResourceServiceKind::VpcSubnets => self.vpc_subnets.failed_context(),
            ResourceServiceKind::FirewallRules => self.firewall_rules.failed_context(),
            ResourceServiceKind::PubSubSubscriptions => self.pubsub_subscriptions.failed_context(),
            ResourceServiceKind::BigQueryDatasets => self.bigquery_datasets.failed_context(),
            ResourceServiceKind::IamServiceAccounts => self.service_accounts.failed_context(),
            ResourceServiceKind::IamPolicy => self.policy_members.failed_context(),
            ResourceServiceKind::CloudStorage => self.storage.failed_context(),
        };
        failed == Some(context)
    }

    pub fn active_service_label(&self) -> &'static str {
        self.active_service.label()
    }

    /// Closes every modal slot.
    ///
    /// The modals are mutually exclusive: two open at once means the renderer
    /// shows whichever one [`crate::ui::render_active_overlay`] reaches first in
    /// its priority chain while the keymap dispatches for the other, so the
    /// second takes keys the operator cannot see it taking.
    ///
    /// Every opener calls this and then sets its own slot, so the set is
    /// enumerated **once**. It used to be seven hand-maintained lists across
    /// four files, no two of them agreeing on which neighbours to clear — a
    /// shape where adding an eighth modal means finding all seven, and where
    /// omitting one from a single list is invisible until an operator hits the
    /// pair. Anything that must survive an opener has to live outside this
    /// function, not be quietly left out of one caller.
    fn close_other_modals(&mut self) {
        self.action_confirmation = None;
        self.batch_confirmation = None;
        self.command_preview = None;
        self.ssh_launch_preview = None;
        self.tunnel_launch_preview = None;
        self.k9s_launch_confirmation = None;
        self.plugin_confirmation = None;
        self.plugin_prompt = None;
    }

    pub fn select_service(&mut self, service: ResourceServiceKind) {
        if self.active_service != service {
            self.close_other_modals();
            self.cloud_run_revision = None;
            self.selected_revision = 0;
            self.detail_field_index = None;
            self.active_service = service;
        }
    }

    /// The copyable-field cursor for the detail pane on screen, as an index
    /// into `DetailBody::fields()`. Callers must clamp it against the field
    /// count of the body they are reading: the body can shrink under a stale
    /// cursor when a detail payload reloads.
    pub fn detail_field_index(&self) -> Option<usize> {
        self.detail_field_index
    }

    pub fn set_detail_field_index(&mut self, index: Option<usize>) {
        self.detail_field_index = index;
    }

    /// Drops the field cursor. Called from every site that changes which
    /// fields the pane shows, so `y` can never copy a field the operator is no
    /// longer looking at.
    pub fn clear_detail_field_index(&mut self) {
        self.detail_field_index = None;
    }

    pub fn storage(&self) -> &StorageState {
        &self.storage
    }

    pub fn storage_mut(&mut self) -> &mut StorageState {
        &mut self.storage
    }

    pub fn has_loaded_context(&self) -> bool {
        match self.active_service {
            ResourceServiceKind::ComputeInstances => self.compute.context.is_some(),
            ResourceServiceKind::GkeClusters => self.gke.context.is_some(),
            ResourceServiceKind::CloudSql => self.cloud_sql.context.is_some(),
            ResourceServiceKind::CloudRunServices => self.cloud_run.context.is_some(),
            ResourceServiceKind::SecretManagerSecrets => self.secrets.context.is_some(),
            ResourceServiceKind::CloudFunctions => self.functions.context.is_some(),
            ResourceServiceKind::PersistentDisks => self.disks.context.is_some(),
            ResourceServiceKind::PubSubTopics => self.pubsub_topics.context.is_some(),
            ResourceServiceKind::VpcNetworks => self.vpc_networks.context.is_some(),
            ResourceServiceKind::VpcSubnets => self.vpc_subnets.context.is_some(),
            ResourceServiceKind::FirewallRules => self.firewall_rules.context.is_some(),
            ResourceServiceKind::PubSubSubscriptions => self.pubsub_subscriptions.context.is_some(),
            ResourceServiceKind::BigQueryDatasets => self.bigquery_datasets.context.is_some(),
            ResourceServiceKind::IamServiceAccounts => self.service_accounts.context.is_some(),
            ResourceServiceKind::IamPolicy => self.policy_members.context.is_some(),
            ResourceServiceKind::CloudStorage => self.storage.has_loaded_context(),
        }
    }

    /// Returns `kind`'s current list-fetch load phase.
    pub fn load_phase(&self, kind: ResourceServiceKind) -> &ResourceLoadPhase {
        match kind {
            ResourceServiceKind::ComputeInstances => self.compute.load_phase(),
            ResourceServiceKind::GkeClusters => self.gke.load_phase(),
            ResourceServiceKind::CloudSql => self.cloud_sql.load_phase(),
            ResourceServiceKind::CloudRunServices => self.cloud_run.load_phase(),
            ResourceServiceKind::SecretManagerSecrets => self.secrets.load_phase(),
            ResourceServiceKind::CloudFunctions => self.functions.load_phase(),
            ResourceServiceKind::PersistentDisks => self.disks.load_phase(),
            ResourceServiceKind::PubSubTopics => self.pubsub_topics.load_phase(),
            ResourceServiceKind::VpcNetworks => self.vpc_networks.load_phase(),
            ResourceServiceKind::VpcSubnets => self.vpc_subnets.load_phase(),
            ResourceServiceKind::FirewallRules => self.firewall_rules.load_phase(),
            ResourceServiceKind::PubSubSubscriptions => self.pubsub_subscriptions.load_phase(),
            ResourceServiceKind::BigQueryDatasets => self.bigquery_datasets.load_phase(),
            ResourceServiceKind::IamServiceAccounts => self.service_accounts.load_phase(),
            ResourceServiceKind::IamPolicy => self.policy_members.load_phase(),
            ResourceServiceKind::CloudStorage => self.storage.load_phase(),
        }
    }

    /// The identity of the listing result `kind` currently holds, and `None`
    /// while it holds none.
    ///
    /// For the health sampler, which needs to tell a result it has
    /// already recorded from a fresh one — something the row count cannot do,
    /// since a re-list returning the same number is a real second sample.
    /// Exhaustive for the reason [`Self::service_stale`] is: a new service must
    /// answer here before it compiles, so it cannot ship with a history that
    /// never advances.
    ///
    /// The identity of the *result*, not of the load: `begin_load` stamps a
    /// sequence when a fetch starts, and a fetch that is dropped rather than
    /// applied would otherwise present the rows already on hand under a
    /// sequence nobody has seen.
    pub fn list_landed_sequence(&self, kind: ResourceServiceKind) -> Option<u64> {
        match kind {
            ResourceServiceKind::ComputeInstances => self.compute.landed_sequence(),
            ResourceServiceKind::GkeClusters => self.gke.landed_sequence(),
            ResourceServiceKind::CloudSql => self.cloud_sql.landed_sequence(),
            ResourceServiceKind::CloudRunServices => self.cloud_run.landed_sequence(),
            ResourceServiceKind::SecretManagerSecrets => self.secrets.landed_sequence(),
            ResourceServiceKind::CloudFunctions => self.functions.landed_sequence(),
            ResourceServiceKind::PersistentDisks => self.disks.landed_sequence(),
            ResourceServiceKind::PubSubTopics => self.pubsub_topics.landed_sequence(),
            ResourceServiceKind::VpcNetworks => self.vpc_networks.landed_sequence(),
            ResourceServiceKind::VpcSubnets => self.vpc_subnets.landed_sequence(),
            ResourceServiceKind::FirewallRules => self.firewall_rules.landed_sequence(),
            ResourceServiceKind::PubSubSubscriptions => self.pubsub_subscriptions.landed_sequence(),
            ResourceServiceKind::BigQueryDatasets => self.bigquery_datasets.landed_sequence(),
            ResourceServiceKind::IamServiceAccounts => self.service_accounts.landed_sequence(),
            ResourceServiceKind::IamPolicy => self.policy_members.landed_sequence(),
            ResourceServiceKind::CloudStorage => self.storage.landed_sequence(),
        }
    }

    /// Returns the active service's current list-fetch load phase.
    pub fn active_load_phase(&self) -> &ResourceLoadPhase {
        self.load_phase(self.active_service)
    }

    /// Marks `kind`'s list fetch as in flight, started at `started_at`, and
    /// returns the [`ResourceLoadTicket`] identifying it. Callers that only
    /// want the phase change may drop the ticket; the runtime keeps it so it
    /// can later resolve this exact load via [`Self::abandon_load`].
    pub fn begin_load(
        &mut self,
        kind: ResourceServiceKind,
        started_at: std::time::Instant,
    ) -> ResourceLoadTicket {
        self.next_load_sequence = self.next_load_sequence.wrapping_add(1);
        let sequence = self.next_load_sequence;
        match kind {
            ResourceServiceKind::ComputeInstances => self.compute.begin_load(sequence, started_at),
            ResourceServiceKind::GkeClusters => self.gke.begin_load(sequence, started_at),
            ResourceServiceKind::CloudSql => self.cloud_sql.begin_load(sequence, started_at),
            ResourceServiceKind::CloudRunServices => {
                self.cloud_run.begin_load(sequence, started_at)
            }
            ResourceServiceKind::SecretManagerSecrets => {
                self.secrets.begin_load(sequence, started_at)
            }
            ResourceServiceKind::CloudFunctions => self.functions.begin_load(sequence, started_at),
            ResourceServiceKind::PersistentDisks => self.disks.begin_load(sequence, started_at),
            ResourceServiceKind::PubSubTopics => {
                self.pubsub_topics.begin_load(sequence, started_at)
            }
            ResourceServiceKind::VpcNetworks => self.vpc_networks.begin_load(sequence, started_at),
            ResourceServiceKind::VpcSubnets => self.vpc_subnets.begin_load(sequence, started_at),
            ResourceServiceKind::FirewallRules => {
                self.firewall_rules.begin_load(sequence, started_at)
            }
            ResourceServiceKind::PubSubSubscriptions => {
                self.pubsub_subscriptions.begin_load(sequence, started_at)
            }
            ResourceServiceKind::BigQueryDatasets => {
                self.bigquery_datasets.begin_load(sequence, started_at)
            }
            ResourceServiceKind::IamServiceAccounts => {
                self.service_accounts.begin_load(sequence, started_at)
            }
            ResourceServiceKind::IamPolicy => self.policy_members.begin_load(sequence, started_at),
            ResourceServiceKind::CloudStorage => self.storage.begin_load(sequence, started_at),
        }
        ResourceLoadTicket { kind, sequence }
    }

    /// Resolves the load started by `ticket` back to [`ResourceLoadPhase::Idle`],
    /// but only while that load still owns its service's phase. It is a no-op
    /// once a newer `begin_load` has taken the phase over, when the phase is
    /// `Failed { .. }` (a real, still-valid error), and when it is already
    /// `Idle`.
    ///
    /// Identity is the ticket's sequence number rather than the
    /// `Loading { started_at }` `Instant`: two loads for the same service minted
    /// inside the monotonic clock's resolution would compare equal and let a
    /// stale abandon clear a live load. The counter makes uniqueness structural
    /// instead of probabilistic.
    ///
    /// This reset belongs at the drop site — where the runtime's generation
    /// guard discards a detached in-flight result — and deliberately *not* in
    /// the per-service context-clear helpers (`mark_stale_and_clear`,
    /// `mark_gke_stale_and_clear`, ...). At context-switch time the fetch is
    /// still genuinely in flight, so `Loading` is the honest state; a blanket
    /// reset there would suppress the Loading affordance that must appear
    /// across a context change (pinned by the
    /// `resources_view_shows_loading_state_*` UI projection tests).
    ///
    /// Returns `true` when this ticket's load still owned a `Loading` phase and
    /// it was resolved to `Idle` — i.e. this call is the one that learned the
    /// fetch is over. `false` for every no-op case above. The runtime uses that
    /// answer to decide whether the detached fetch is worth rescheduling;
    /// a no-op means some newer load already owns the phase and is
    /// responsible for it.
    pub fn abandon_load(&mut self, ticket: ResourceLoadTicket) -> bool {
        match ticket.kind {
            ResourceServiceKind::ComputeInstances => self.compute.abandon_load(ticket.sequence),
            ResourceServiceKind::GkeClusters => self.gke.abandon_load(ticket.sequence),
            ResourceServiceKind::CloudSql => self.cloud_sql.abandon_load(ticket.sequence),
            ResourceServiceKind::CloudRunServices => self.cloud_run.abandon_load(ticket.sequence),
            ResourceServiceKind::SecretManagerSecrets => self.secrets.abandon_load(ticket.sequence),
            ResourceServiceKind::CloudFunctions => self.functions.abandon_load(ticket.sequence),
            ResourceServiceKind::PersistentDisks => self.disks.abandon_load(ticket.sequence),
            ResourceServiceKind::PubSubTopics => self.pubsub_topics.abandon_load(ticket.sequence),
            ResourceServiceKind::VpcNetworks => self.vpc_networks.abandon_load(ticket.sequence),
            ResourceServiceKind::VpcSubnets => self.vpc_subnets.abandon_load(ticket.sequence),
            ResourceServiceKind::FirewallRules => self.firewall_rules.abandon_load(ticket.sequence),
            ResourceServiceKind::PubSubSubscriptions => {
                self.pubsub_subscriptions.abandon_load(ticket.sequence)
            }
            ResourceServiceKind::BigQueryDatasets => {
                self.bigquery_datasets.abandon_load(ticket.sequence)
            }
            ResourceServiceKind::IamServiceAccounts => {
                self.service_accounts.abandon_load(ticket.sequence)
            }
            ResourceServiceKind::IamPolicy => self.policy_members.abandon_load(ticket.sequence),
            ResourceServiceKind::CloudStorage => self.storage.abandon_load(ticket.sequence),
        }
    }

    /// Marks `kind`'s list fetch as failed with `message`.
    pub fn fail_load(
        &mut self,
        kind: ResourceServiceKind,
        context: GcloudContext,
        message: String,
    ) {
        match kind {
            ResourceServiceKind::ComputeInstances => self.compute.fail_load(context, message),
            ResourceServiceKind::GkeClusters => self.gke.fail_load(context, message),
            ResourceServiceKind::CloudSql => self.cloud_sql.fail_load(context, message),
            ResourceServiceKind::CloudRunServices => self.cloud_run.fail_load(context, message),
            ResourceServiceKind::SecretManagerSecrets => self.secrets.fail_load(context, message),
            ResourceServiceKind::CloudFunctions => self.functions.fail_load(context, message),
            ResourceServiceKind::PersistentDisks => self.disks.fail_load(context, message),
            ResourceServiceKind::PubSubTopics => self.pubsub_topics.fail_load(context, message),
            ResourceServiceKind::VpcNetworks => self.vpc_networks.fail_load(context, message),
            ResourceServiceKind::VpcSubnets => self.vpc_subnets.fail_load(context, message),
            ResourceServiceKind::FirewallRules => self.firewall_rules.fail_load(context, message),
            ResourceServiceKind::PubSubSubscriptions => {
                self.pubsub_subscriptions.fail_load(context, message)
            }
            ResourceServiceKind::BigQueryDatasets => {
                self.bigquery_datasets.fail_load(context, message)
            }
            ResourceServiceKind::IamServiceAccounts => {
                self.service_accounts.fail_load(context, message)
            }
            ResourceServiceKind::IamPolicy => self.policy_members.fail_load(context, message),
            ResourceServiceKind::CloudStorage => self.storage.fail_load(context, message),
        }
    }

    /// Returns the loaded configuration name for the active service.
    /// Every location this workspace has listed a row in: instance zones and
    /// disk locations, in list order, deduplicated.
    ///
    /// The FinOps fanout is derived from these rather than from an enumeration
    /// of every Google zone. A disk location may be a region rather than a
    /// zone — a regional disk carries its region here — so callers must
    /// discriminate rather than assume; `gcp::finops::query_locations` does.
    pub fn listed_locations(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for location in self.instances().iter().map(|row| row.zone.clone()).chain(
            self.persistent_disks()
                .iter()
                .map(|row| row.location.clone()),
        ) {
            if !location.is_empty() && !out.contains(&location) {
                out.push(location);
            }
        }
        out
    }

    pub fn active_context_label(&self) -> Option<String> {
        match self.active_service {
            ResourceServiceKind::ComputeInstances => self
                .compute
                .context
                .as_ref()
                .map(|context| context.configuration.to_string()),
            ResourceServiceKind::GkeClusters => self
                .gke
                .context
                .as_ref()
                .map(|context| context.configuration.to_string()),
            ResourceServiceKind::CloudSql => self
                .cloud_sql
                .context
                .as_ref()
                .map(|context| context.configuration.to_string()),
            ResourceServiceKind::CloudRunServices => self
                .cloud_run
                .context
                .as_ref()
                .map(|context| context.configuration.to_string()),
            ResourceServiceKind::SecretManagerSecrets => self
                .secrets
                .context
                .as_ref()
                .map(|context| context.configuration.to_string()),
            ResourceServiceKind::CloudFunctions => self
                .functions
                .context
                .as_ref()
                .map(|context| context.configuration.to_string()),
            ResourceServiceKind::PersistentDisks => self
                .disks
                .context
                .as_ref()
                .map(|context| context.configuration.to_string()),
            ResourceServiceKind::PubSubTopics => self
                .pubsub_topics
                .context
                .as_ref()
                .map(|context| context.configuration.to_string()),
            ResourceServiceKind::VpcNetworks => self
                .vpc_networks
                .context
                .as_ref()
                .map(|context| context.configuration.to_string()),
            ResourceServiceKind::VpcSubnets => self
                .vpc_subnets
                .context
                .as_ref()
                .map(|context| context.configuration.to_string()),
            ResourceServiceKind::FirewallRules => self
                .firewall_rules
                .context
                .as_ref()
                .map(|context| context.configuration.to_string()),
            ResourceServiceKind::PubSubSubscriptions => self
                .pubsub_subscriptions
                .context
                .as_ref()
                .map(|context| context.configuration.to_string()),
            ResourceServiceKind::BigQueryDatasets => self
                .bigquery_datasets
                .context
                .as_ref()
                .map(|context| context.configuration.to_string()),
            ResourceServiceKind::IamServiceAccounts => self
                .service_accounts
                .context
                .as_ref()
                .map(|context| context.configuration.to_string()),
            ResourceServiceKind::IamPolicy => self
                .policy_members
                .context
                .as_ref()
                .map(|context| context.configuration.to_string()),
            ResourceServiceKind::CloudStorage => self.storage.active_context_label(),
        }
    }

    pub fn is_stale(&self) -> bool {
        match self.active_service {
            ResourceServiceKind::ComputeInstances => self.compute.stale,
            ResourceServiceKind::GkeClusters => self.gke.stale,
            ResourceServiceKind::CloudSql => self.cloud_sql.stale,
            ResourceServiceKind::CloudRunServices => self.cloud_run.stale,
            ResourceServiceKind::SecretManagerSecrets => self.secrets.stale,
            ResourceServiceKind::CloudFunctions => self.functions.stale,
            ResourceServiceKind::PersistentDisks => self.disks.stale,
            ResourceServiceKind::PubSubTopics => self.pubsub_topics.stale,
            ResourceServiceKind::VpcNetworks => self.vpc_networks.stale,
            ResourceServiceKind::VpcSubnets => self.vpc_subnets.stale,
            ResourceServiceKind::FirewallRules => self.firewall_rules.stale,
            ResourceServiceKind::PubSubSubscriptions => self.pubsub_subscriptions.stale,
            ResourceServiceKind::BigQueryDatasets => self.bigquery_datasets.stale,
            ResourceServiceKind::IamServiceAccounts => self.service_accounts.stale,
            ResourceServiceKind::IamPolicy => self.policy_members.stale,
            ResourceServiceKind::CloudStorage => self.storage.is_stale(),
        }
    }

    /// Returns true when the active service lacks fresh rows for `context`.
    pub fn needs_refresh(&self, context: &GcloudContext) -> bool {
        match self.active_service {
            ResourceServiceKind::ComputeInstances => {
                self.compute.context.as_ref() != Some(context) || self.compute.stale
            }
            ResourceServiceKind::GkeClusters => {
                self.gke.context.as_ref() != Some(context) || self.gke.stale
            }
            ResourceServiceKind::CloudSql => {
                self.cloud_sql.context.as_ref() != Some(context) || self.cloud_sql.stale
            }
            ResourceServiceKind::CloudRunServices => {
                self.cloud_run.context.as_ref() != Some(context) || self.cloud_run.stale
            }
            ResourceServiceKind::SecretManagerSecrets => {
                self.secrets.context.as_ref() != Some(context) || self.secrets.stale
            }
            ResourceServiceKind::CloudFunctions => {
                self.functions.context.as_ref() != Some(context) || self.functions.stale
            }
            ResourceServiceKind::PersistentDisks => {
                self.disks.context.as_ref() != Some(context) || self.disks.stale
            }
            ResourceServiceKind::PubSubTopics => {
                self.pubsub_topics.context.as_ref() != Some(context) || self.pubsub_topics.stale
            }
            ResourceServiceKind::VpcNetworks => {
                self.vpc_networks.context.as_ref() != Some(context) || self.vpc_networks.stale
            }
            ResourceServiceKind::VpcSubnets => {
                self.vpc_subnets.context.as_ref() != Some(context) || self.vpc_subnets.stale
            }
            ResourceServiceKind::FirewallRules => {
                self.firewall_rules.context.as_ref() != Some(context) || self.firewall_rules.stale
            }
            ResourceServiceKind::PubSubSubscriptions => {
                self.pubsub_subscriptions.context.as_ref() != Some(context)
                    || self.pubsub_subscriptions.stale
            }
            ResourceServiceKind::BigQueryDatasets => {
                self.bigquery_datasets.context.as_ref() != Some(context)
                    || self.bigquery_datasets.stale
            }
            ResourceServiceKind::IamServiceAccounts => {
                self.service_accounts.context.as_ref() != Some(context)
                    || self.service_accounts.stale
            }
            ResourceServiceKind::IamPolicy => {
                self.policy_members.context.as_ref() != Some(context) || self.policy_members.stale
            }
            ResourceServiceKind::CloudStorage => self.storage.needs_refresh(context),
        }
    }

    pub fn selected_index(&self) -> Option<usize> {
        self.selected_visible_index()
    }

    pub fn selected_visible_index(&self) -> Option<usize> {
        match self.active_service {
            ResourceServiceKind::ComputeInstances => {
                (!self.visible_instances().is_empty()).then_some(self.compute.selected_visible)
            }
            ResourceServiceKind::GkeClusters => {
                (!self.visible_gke_clusters().is_empty()).then_some(self.gke.selected_visible)
            }
            ResourceServiceKind::CloudSql => (!self.visible_cloud_sql_instances().is_empty())
                .then_some(self.cloud_sql.selected_visible),
            ResourceServiceKind::CloudRunServices => {
                (!self.visible_cloud_run_services().is_empty())
                    .then_some(self.cloud_run.selected_visible)
            }
            ResourceServiceKind::SecretManagerSecrets => {
                (!self.visible_secrets().is_empty()).then_some(self.secrets.selected_visible)
            }
            ResourceServiceKind::CloudFunctions => (!self.visible_cloud_functions().is_empty())
                .then_some(self.functions.selected_visible),
            ResourceServiceKind::PersistentDisks => {
                (!self.visible_persistent_disks().is_empty()).then_some(self.disks.selected_visible)
            }
            ResourceServiceKind::PubSubTopics => (!self.visible_pubsub_topics().is_empty())
                .then_some(self.pubsub_topics.selected_visible),
            ResourceServiceKind::VpcNetworks => (!self.visible_vpc_networks().is_empty())
                .then_some(self.vpc_networks.selected_visible),
            ResourceServiceKind::VpcSubnets => (!self.visible_vpc_subnets().is_empty())
                .then_some(self.vpc_subnets.selected_visible),
            ResourceServiceKind::FirewallRules => (!self.visible_firewall_rules().is_empty())
                .then_some(self.firewall_rules.selected_visible),
            ResourceServiceKind::PubSubSubscriptions => {
                (!self.visible_pubsub_subscriptions().is_empty())
                    .then_some(self.pubsub_subscriptions.selected_visible)
            }
            ResourceServiceKind::BigQueryDatasets => self.bigquery_selected_visible_index(),
            ResourceServiceKind::IamServiceAccounts => {
                (!self.visible_service_accounts().is_empty())
                    .then_some(self.service_accounts.selected_visible)
            }
            ResourceServiceKind::IamPolicy => (!self.visible_policy_members().is_empty())
                .then_some(self.policy_members.selected_visible),
            ResourceServiceKind::CloudStorage => self.storage.selected_visible_index(),
        }
    }

    /// Returns the selected index only when the projected row list is non-empty.
    pub fn selected_visible_index_for_visible_rows(&self, visible_rows: usize) -> Option<usize> {
        match self.active_service {
            ResourceServiceKind::ComputeInstances => {
                (visible_rows > 0).then_some(self.compute.selected_visible)
            }
            ResourceServiceKind::GkeClusters => {
                (visible_rows > 0).then_some(self.gke.selected_visible)
            }
            ResourceServiceKind::CloudSql => {
                (visible_rows > 0).then_some(self.cloud_sql.selected_visible)
            }
            ResourceServiceKind::CloudRunServices => {
                (visible_rows > 0).then_some(self.cloud_run.selected_visible)
            }
            ResourceServiceKind::SecretManagerSecrets => {
                (visible_rows > 0).then_some(self.secrets.selected_visible)
            }
            ResourceServiceKind::CloudFunctions => {
                (visible_rows > 0).then_some(self.functions.selected_visible)
            }
            ResourceServiceKind::PersistentDisks => {
                (visible_rows > 0).then_some(self.disks.selected_visible)
            }
            ResourceServiceKind::PubSubTopics => {
                (visible_rows > 0).then_some(self.pubsub_topics.selected_visible)
            }
            ResourceServiceKind::VpcNetworks => {
                (visible_rows > 0).then_some(self.vpc_networks.selected_visible)
            }
            ResourceServiceKind::VpcSubnets => {
                (visible_rows > 0).then_some(self.vpc_subnets.selected_visible)
            }
            ResourceServiceKind::FirewallRules => {
                (visible_rows > 0).then_some(self.firewall_rules.selected_visible)
            }
            ResourceServiceKind::PubSubSubscriptions => {
                (visible_rows > 0).then_some(self.pubsub_subscriptions.selected_visible)
            }
            ResourceServiceKind::BigQueryDatasets => {
                self.bigquery_selected_visible_index_for_visible_rows(visible_rows)
            }
            ResourceServiceKind::IamServiceAccounts => {
                (visible_rows > 0).then_some(self.service_accounts.selected_visible)
            }
            ResourceServiceKind::IamPolicy => {
                (visible_rows > 0).then_some(self.policy_members.selected_visible)
            }
            ResourceServiceKind::CloudStorage => self
                .storage
                .selected_visible_index_for_visible_rows(visible_rows),
        }
    }

    pub fn is_filter_input_active(&self) -> bool {
        match self.active_service {
            ResourceServiceKind::ComputeInstances => self.compute.filter_input_active,
            ResourceServiceKind::GkeClusters => self.gke.filter_input_active,
            ResourceServiceKind::CloudSql => self.cloud_sql.filter_input_active,
            ResourceServiceKind::CloudRunServices => self.cloud_run.filter_input_active,
            ResourceServiceKind::SecretManagerSecrets => self.secrets.filter_input_active,
            ResourceServiceKind::CloudFunctions => self.functions.filter_input_active,
            ResourceServiceKind::PersistentDisks => self.disks.filter_input_active,
            ResourceServiceKind::PubSubTopics => self.pubsub_topics.filter_input_active,
            ResourceServiceKind::VpcNetworks => self.vpc_networks.filter_input_active,
            ResourceServiceKind::VpcSubnets => self.vpc_subnets.filter_input_active,
            ResourceServiceKind::FirewallRules => self.firewall_rules.filter_input_active,
            ResourceServiceKind::PubSubSubscriptions => {
                self.pubsub_subscriptions.filter_input_active
            }
            ResourceServiceKind::BigQueryDatasets => self.bigquery_filter_input_active(),
            ResourceServiceKind::IamServiceAccounts => self.service_accounts.filter_input_active,
            ResourceServiceKind::IamPolicy => self.policy_members.filter_input_active,
            ResourceServiceKind::CloudStorage => self.storage.is_filter_input_active(),
        }
    }

    pub fn filter_draft(&self) -> &str {
        match self.active_service {
            ResourceServiceKind::ComputeInstances => &self.compute.filter_draft,
            ResourceServiceKind::GkeClusters => &self.gke.filter_draft,
            ResourceServiceKind::CloudSql => &self.cloud_sql.filter_draft,
            ResourceServiceKind::CloudRunServices => &self.cloud_run.filter_draft,
            ResourceServiceKind::SecretManagerSecrets => &self.secrets.filter_draft,
            ResourceServiceKind::CloudFunctions => &self.functions.filter_draft,
            ResourceServiceKind::PersistentDisks => &self.disks.filter_draft,
            ResourceServiceKind::PubSubTopics => &self.pubsub_topics.filter_draft,
            ResourceServiceKind::VpcNetworks => &self.vpc_networks.filter_draft,
            ResourceServiceKind::VpcSubnets => &self.vpc_subnets.filter_draft,
            ResourceServiceKind::FirewallRules => &self.firewall_rules.filter_draft,
            ResourceServiceKind::PubSubSubscriptions => &self.pubsub_subscriptions.filter_draft,
            ResourceServiceKind::BigQueryDatasets => self.bigquery_filter_draft(),
            ResourceServiceKind::IamServiceAccounts => &self.service_accounts.filter_draft,
            ResourceServiceKind::IamPolicy => &self.policy_members.filter_draft,
            ResourceServiceKind::CloudStorage => self.storage.filter_draft(),
        }
    }

    pub fn filter_applied(&self) -> &str {
        self.filter_applied_for(self.active_service)
    }

    /// The applied filter of `kind`, whether or not it is the active service.
    ///
    /// Each service owns its `filter_applied` field and only ever edits it
    /// while it is active, so the stored text stays meaningful after the user
    /// switches away — which is what lets a reload of a *non-active* service
    /// (a post-action follow-up, an end-of-batch refresh) carry that service's
    /// own filter instead of dropping it.
    pub fn filter_applied_for(&self, kind: ResourceServiceKind) -> &str {
        match kind {
            ResourceServiceKind::ComputeInstances => self.compute.filter_applied.source(),
            ResourceServiceKind::GkeClusters => self.gke.filter_applied.source(),
            ResourceServiceKind::CloudSql => self.cloud_sql.filter_applied.source(),
            ResourceServiceKind::CloudRunServices => self.cloud_run.filter_applied.source(),
            ResourceServiceKind::SecretManagerSecrets => self.secrets.filter_applied.source(),
            ResourceServiceKind::CloudFunctions => self.functions.filter_applied.source(),
            ResourceServiceKind::PersistentDisks => self.disks.filter_applied.source(),
            ResourceServiceKind::PubSubTopics => self.pubsub_topics.filter_applied.source(),
            ResourceServiceKind::VpcNetworks => self.vpc_networks.filter_applied.source(),
            ResourceServiceKind::VpcSubnets => self.vpc_subnets.filter_applied.source(),
            ResourceServiceKind::FirewallRules => self.firewall_rules.filter_applied.source(),
            ResourceServiceKind::PubSubSubscriptions => {
                self.pubsub_subscriptions.filter_applied.source()
            }
            ResourceServiceKind::BigQueryDatasets => self.bigquery_filter_applied(),
            ResourceServiceKind::IamServiceAccounts => {
                self.service_accounts.filter_applied.source()
            }
            ResourceServiceKind::IamPolicy => self.policy_members.filter_applied.source(),
            ResourceServiceKind::CloudStorage => self.storage.filter_applied(),
        }
    }

    /// The mode of the draft while the filter input is active, otherwise of
    /// the applied text. Always [`FilterMode::Regex`] for Cloud Storage, which
    /// keeps the plain substring filter and renders no mode chip.
    pub fn filter_mode(&self) -> FilterMode {
        if self.active_service == ResourceServiceKind::CloudStorage {
            return FilterMode::Regex;
        }
        // Imported under a distinct name so this reads as the domain
        // classifier it is, not as recursion into the method it sits in.
        filter_mode_of(if self.is_filter_input_active() {
            self.filter_draft()
        } else {
            self.filter_applied()
        })
    }

    /// The non-fatal inline filter error for the active service's *applied*
    /// filter, if any. Cloud Storage never reports one: its filter is a plain
    /// substring with nothing to misparse.
    pub fn filter_error(&self) -> Option<String> {
        if self.active_service == ResourceServiceKind::CloudStorage {
            return None;
        }
        let applied = self.filter_applied();
        match parse_filter(applied) {
            Err(error) => Some(error.message),
            Ok(FilterQuery::Label(selectors)) => {
                label_filter_error(self.active_service, &selectors)
            }
            // Everything that parsed and is not a label selector can only
            // still fail by not compiling. The message is the one the cache
            // swallowed when it fell back to no narrowing -- read off the
            // cache rather than recompiled, which is the point of the cache.
            Ok(_) => self.applied_filter_error(),
        }
    }

    /// The compile error of the active service's applied pattern, asked of the
    /// cache. Never copied into a field beside the filter: a second copy is a
    /// second thing that can disagree with the filter the rows were actually
    /// matched against. Cloud Storage compiles nothing, so it has none.
    fn applied_filter_error(&self) -> Option<String> {
        match self.active_service {
            ResourceServiceKind::ComputeInstances => self.compute.filter_applied.error_text(),
            ResourceServiceKind::GkeClusters => self.gke.filter_applied.error_text(),
            ResourceServiceKind::CloudSql => self.cloud_sql.filter_applied.error_text(),
            ResourceServiceKind::CloudRunServices => self.cloud_run.filter_applied.error_text(),
            ResourceServiceKind::SecretManagerSecrets => self.secrets.filter_applied.error_text(),
            ResourceServiceKind::CloudFunctions => self.functions.filter_applied.error_text(),
            ResourceServiceKind::PersistentDisks => self.disks.filter_applied.error_text(),
            ResourceServiceKind::PubSubTopics => self.pubsub_topics.filter_applied.error_text(),
            ResourceServiceKind::VpcNetworks => self.vpc_networks.filter_applied.error_text(),
            ResourceServiceKind::VpcSubnets => self.vpc_subnets.filter_applied.error_text(),
            ResourceServiceKind::FirewallRules => self.firewall_rules.filter_applied.error_text(),
            ResourceServiceKind::PubSubSubscriptions => {
                self.pubsub_subscriptions.filter_applied.error_text()
            }
            ResourceServiceKind::BigQueryDatasets => self.bigquery_applied_filter_error(),
            ResourceServiceKind::IamServiceAccounts => {
                self.service_accounts.filter_applied.error_text()
            }
            ResourceServiceKind::IamPolicy => self.policy_members.filter_applied.error_text(),
            ResourceServiceKind::CloudStorage => None,
        }
    }

    pub fn action_confirmation(&self) -> Option<&ResourceActionConfirmation> {
        self.action_confirmation.as_ref()
    }

    pub fn batch_confirmation(&self) -> Option<&BatchActionConfirmation> {
        self.batch_confirmation.as_ref()
    }

    pub fn command_preview(&self) -> Option<&ResourceCommandPreview> {
        self.command_preview.as_ref()
    }

    /// Open the preview for an already-rendered command line, closing every
    /// other modal through `close_other_modals`.
    ///
    /// `show_ssh_command_preview` delegates here rather than opening its own
    /// preview, so the two cannot drift on the title and copy wording either.
    pub fn show_command_preview(
        &mut self,
        title: String,
        command: String,
        copy_description: String,
    ) {
        self.close_other_modals();
        self.command_preview = Some(ResourceCommandPreview::new(
            title,
            command,
            copy_description,
        ));
    }

    pub fn sort_label(&self) -> &'static str {
        match self.active_service {
            ResourceServiceKind::ComputeInstances => self.compute.sort_field.label(),
            ResourceServiceKind::GkeClusters => self.gke.sort_field.label(),
            ResourceServiceKind::CloudSql => self.cloud_sql.sort_field.label(),
            ResourceServiceKind::CloudRunServices => self.cloud_run.sort_field.label(),
            ResourceServiceKind::SecretManagerSecrets => self.secrets.sort_field.label(),
            ResourceServiceKind::CloudFunctions => self.functions.sort_field.label(),
            ResourceServiceKind::PersistentDisks => self.disks.sort_field.label(),
            ResourceServiceKind::PubSubTopics => self.pubsub_topics.sort_field.label(),
            ResourceServiceKind::VpcNetworks => self.vpc_networks.sort_field.label(),
            ResourceServiceKind::VpcSubnets => self.vpc_subnets.sort_field.label(),
            ResourceServiceKind::FirewallRules => self.firewall_rules.sort_field.label(),
            ResourceServiceKind::PubSubSubscriptions => {
                self.pubsub_subscriptions.sort_field.label()
            }
            ResourceServiceKind::BigQueryDatasets => self.bigquery_sort_label(),
            ResourceServiceKind::IamServiceAccounts => self.service_accounts.sort_field.label(),
            ResourceServiceKind::IamPolicy => self.policy_members.sort_field.label(),
            ResourceServiceKind::CloudStorage => self.storage.sort_label(),
        }
    }

    pub fn start_filter(&mut self) {
        match self.active_service {
            ResourceServiceKind::ComputeInstances => {
                self.compute.filter_input_active = true;
                self.compute.filter_draft = self.compute.filter_applied.source().to_string();
            }
            ResourceServiceKind::GkeClusters => {
                self.gke.filter_input_active = true;
                self.gke.filter_draft = self.gke.filter_applied.source().to_string();
            }
            ResourceServiceKind::CloudSql => {
                self.cloud_sql.filter_input_active = true;
                self.cloud_sql.filter_draft = self.cloud_sql.filter_applied.source().to_string();
            }
            ResourceServiceKind::CloudRunServices => {
                self.cloud_run.filter_input_active = true;
                self.cloud_run.filter_draft = self.cloud_run.filter_applied.source().to_string();
            }
            ResourceServiceKind::SecretManagerSecrets => {
                self.secrets.filter_input_active = true;
                self.secrets.filter_draft = self.secrets.filter_applied.source().to_string();
            }
            ResourceServiceKind::CloudFunctions => {
                self.functions.filter_input_active = true;
                self.functions.filter_draft = self.functions.filter_applied.source().to_string();
            }
            ResourceServiceKind::PersistentDisks => {
                self.disks.filter_input_active = true;
                self.disks.filter_draft = self.disks.filter_applied.source().to_string();
            }
            ResourceServiceKind::PubSubTopics => {
                self.pubsub_topics.filter_input_active = true;
                self.pubsub_topics.filter_draft =
                    self.pubsub_topics.filter_applied.source().to_string();
            }
            ResourceServiceKind::VpcNetworks => {
                self.vpc_networks.filter_input_active = true;
                self.vpc_networks.filter_draft =
                    self.vpc_networks.filter_applied.source().to_string();
            }
            ResourceServiceKind::VpcSubnets => {
                self.vpc_subnets.filter_input_active = true;
                self.vpc_subnets.filter_draft =
                    self.vpc_subnets.filter_applied.source().to_string();
            }
            ResourceServiceKind::FirewallRules => {
                self.firewall_rules.filter_input_active = true;
                self.firewall_rules.filter_draft =
                    self.firewall_rules.filter_applied.source().to_string();
            }
            ResourceServiceKind::PubSubSubscriptions => {
                self.pubsub_subscriptions.filter_input_active = true;
                self.pubsub_subscriptions.filter_draft = self
                    .pubsub_subscriptions
                    .filter_applied
                    .source()
                    .to_string();
            }
            ResourceServiceKind::BigQueryDatasets => self.bigquery_start_filter(),
            ResourceServiceKind::IamServiceAccounts => {
                self.service_accounts.filter_input_active = true;
                self.service_accounts.filter_draft =
                    self.service_accounts.filter_applied.source().to_string();
            }
            ResourceServiceKind::IamPolicy => {
                self.policy_members.filter_input_active = true;
                self.policy_members.filter_draft =
                    self.policy_members.filter_applied.source().to_string();
            }
            ResourceServiceKind::CloudStorage => self.storage.start_filter(),
        }
    }

    pub fn append_filter_char(&mut self, value: char) {
        if !value.is_control() {
            match self.active_service {
                ResourceServiceKind::ComputeInstances => self.compute.filter_draft.push(value),
                ResourceServiceKind::GkeClusters => self.gke.filter_draft.push(value),
                ResourceServiceKind::CloudSql => self.cloud_sql.filter_draft.push(value),
                ResourceServiceKind::CloudRunServices => self.cloud_run.filter_draft.push(value),
                ResourceServiceKind::SecretManagerSecrets => self.secrets.filter_draft.push(value),
                ResourceServiceKind::CloudFunctions => self.functions.filter_draft.push(value),
                ResourceServiceKind::PersistentDisks => self.disks.filter_draft.push(value),
                ResourceServiceKind::PubSubTopics => self.pubsub_topics.filter_draft.push(value),
                ResourceServiceKind::VpcNetworks => self.vpc_networks.filter_draft.push(value),
                ResourceServiceKind::VpcSubnets => self.vpc_subnets.filter_draft.push(value),
                ResourceServiceKind::FirewallRules => self.firewall_rules.filter_draft.push(value),
                ResourceServiceKind::PubSubSubscriptions => {
                    self.pubsub_subscriptions.filter_draft.push(value)
                }
                ResourceServiceKind::BigQueryDatasets => self.bigquery_append_filter_char(value),
                ResourceServiceKind::IamServiceAccounts => {
                    self.service_accounts.filter_draft.push(value)
                }
                ResourceServiceKind::IamPolicy => self.policy_members.filter_draft.push(value),
                ResourceServiceKind::CloudStorage => self.storage.append_filter_char(value),
            }
        }
    }

    pub fn backspace_filter(&mut self) {
        match self.active_service {
            ResourceServiceKind::ComputeInstances => {
                self.compute.filter_draft.pop();
            }
            ResourceServiceKind::GkeClusters => {
                self.gke.filter_draft.pop();
            }
            ResourceServiceKind::CloudSql => {
                self.cloud_sql.filter_draft.pop();
            }
            ResourceServiceKind::CloudRunServices => {
                self.cloud_run.filter_draft.pop();
            }
            ResourceServiceKind::SecretManagerSecrets => {
                self.secrets.filter_draft.pop();
            }
            ResourceServiceKind::CloudFunctions => {
                self.functions.filter_draft.pop();
            }
            ResourceServiceKind::PersistentDisks => {
                self.disks.filter_draft.pop();
            }
            ResourceServiceKind::PubSubTopics => {
                self.pubsub_topics.filter_draft.pop();
            }
            ResourceServiceKind::VpcNetworks => {
                self.vpc_networks.filter_draft.pop();
            }
            ResourceServiceKind::VpcSubnets => {
                self.vpc_subnets.filter_draft.pop();
            }
            ResourceServiceKind::FirewallRules => {
                self.firewall_rules.filter_draft.pop();
            }
            ResourceServiceKind::PubSubSubscriptions => {
                self.pubsub_subscriptions.filter_draft.pop();
            }
            ResourceServiceKind::BigQueryDatasets => self.bigquery_backspace_filter(),
            ResourceServiceKind::IamServiceAccounts => {
                self.service_accounts.filter_draft.pop();
            }
            ResourceServiceKind::IamPolicy => {
                self.policy_members.filter_draft.pop();
            }
            ResourceServiceKind::CloudStorage => self.storage.backspace_filter(),
        }
    }

    /// Applies the pending filter draft for the active service. Only Cloud
    /// Storage's object browser can return a reload request (see
    /// [`StorageState::apply_filter`]); every other service/view is local
    /// only and always returns `None`.
    pub fn apply_filter(&mut self) -> Option<(String, String)> {
        match self.active_service {
            ResourceServiceKind::ComputeInstances => {
                self.compute.filter_applied =
                    CompiledResourceFilter::new(self.compute.filter_draft.trim().to_string());
                self.compute.filter_draft = self.compute.filter_applied.source().to_string();
                self.compute.filter_input_active = false;
                self.compute.selected_visible = self
                    .compute
                    .selected_visible
                    .min(self.visible_instances().len().saturating_sub(1));
                None
            }
            ResourceServiceKind::GkeClusters => {
                self.gke.filter_applied =
                    CompiledResourceFilter::new(self.gke.filter_draft.trim().to_string());
                self.gke.filter_draft = self.gke.filter_applied.source().to_string();
                self.gke.filter_input_active = false;
                self.gke.selected_visible = self
                    .gke
                    .selected_visible
                    .min(self.visible_gke_clusters().len().saturating_sub(1));
                None
            }
            ResourceServiceKind::CloudSql => {
                self.cloud_sql.filter_applied =
                    CompiledResourceFilter::new(self.cloud_sql.filter_draft.trim().to_string());
                self.cloud_sql.filter_draft = self.cloud_sql.filter_applied.source().to_string();
                self.cloud_sql.filter_input_active = false;
                self.cloud_sql.selected_visible = self
                    .cloud_sql
                    .selected_visible
                    .min(self.visible_cloud_sql_instances().len().saturating_sub(1));
                None
            }
            ResourceServiceKind::CloudRunServices => {
                self.cloud_run.filter_applied =
                    CompiledResourceFilter::new(self.cloud_run.filter_draft.trim().to_string());
                self.cloud_run.filter_draft = self.cloud_run.filter_applied.source().to_string();
                self.cloud_run.filter_input_active = false;
                self.cloud_run.selected_visible = self
                    .cloud_run
                    .selected_visible
                    .min(self.visible_cloud_run_services().len().saturating_sub(1));
                None
            }
            ResourceServiceKind::SecretManagerSecrets => {
                self.secrets.filter_applied =
                    CompiledResourceFilter::new(self.secrets.filter_draft.trim().to_string());
                self.secrets.filter_draft = self.secrets.filter_applied.source().to_string();
                self.secrets.filter_input_active = false;
                self.secrets.selected_visible = self
                    .secrets
                    .selected_visible
                    .min(self.visible_secrets().len().saturating_sub(1));
                None
            }
            ResourceServiceKind::CloudFunctions => {
                self.functions.filter_applied =
                    CompiledResourceFilter::new(self.functions.filter_draft.trim().to_string());
                self.functions.filter_draft = self.functions.filter_applied.source().to_string();
                self.functions.filter_input_active = false;
                self.functions.selected_visible = self
                    .functions
                    .selected_visible
                    .min(self.visible_cloud_functions().len().saturating_sub(1));
                None
            }
            ResourceServiceKind::PersistentDisks => {
                self.disks.filter_applied =
                    CompiledResourceFilter::new(self.disks.filter_draft.trim().to_string());
                self.disks.filter_draft = self.disks.filter_applied.source().to_string();
                self.disks.filter_input_active = false;
                self.disks.selected_visible = self
                    .disks
                    .selected_visible
                    .min(self.visible_persistent_disks().len().saturating_sub(1));
                None
            }
            ResourceServiceKind::PubSubTopics => {
                self.pubsub_topics.filter_applied =
                    CompiledResourceFilter::new(self.pubsub_topics.filter_draft.trim().to_string());
                self.pubsub_topics.filter_draft =
                    self.pubsub_topics.filter_applied.source().to_string();
                self.pubsub_topics.filter_input_active = false;
                self.pubsub_topics.selected_visible = self
                    .pubsub_topics
                    .selected_visible
                    .min(self.visible_pubsub_topics().len().saturating_sub(1));
                None
            }
            ResourceServiceKind::VpcNetworks => {
                self.vpc_networks.filter_applied =
                    CompiledResourceFilter::new(self.vpc_networks.filter_draft.trim().to_string());
                self.vpc_networks.filter_draft =
                    self.vpc_networks.filter_applied.source().to_string();
                self.vpc_networks.filter_input_active = false;
                self.vpc_networks.selected_visible = self
                    .vpc_networks
                    .selected_visible
                    .min(self.visible_vpc_networks().len().saturating_sub(1));
                None
            }
            ResourceServiceKind::VpcSubnets => {
                self.vpc_subnets.filter_applied =
                    CompiledResourceFilter::new(self.vpc_subnets.filter_draft.trim().to_string());
                self.vpc_subnets.filter_draft =
                    self.vpc_subnets.filter_applied.source().to_string();
                self.vpc_subnets.filter_input_active = false;
                self.vpc_subnets.selected_visible = self
                    .vpc_subnets
                    .selected_visible
                    .min(self.visible_vpc_subnets().len().saturating_sub(1));
                None
            }
            ResourceServiceKind::FirewallRules => {
                self.firewall_rules.filter_applied = CompiledResourceFilter::new(
                    self.firewall_rules.filter_draft.trim().to_string(),
                );
                self.firewall_rules.filter_draft =
                    self.firewall_rules.filter_applied.source().to_string();
                self.firewall_rules.filter_input_active = false;
                self.firewall_rules.selected_visible = self
                    .firewall_rules
                    .selected_visible
                    .min(self.visible_firewall_rules().len().saturating_sub(1));
                None
            }
            ResourceServiceKind::PubSubSubscriptions => {
                self.pubsub_subscriptions.filter_applied = CompiledResourceFilter::new(
                    self.pubsub_subscriptions.filter_draft.trim().to_string(),
                );
                self.pubsub_subscriptions.filter_draft = self
                    .pubsub_subscriptions
                    .filter_applied
                    .source()
                    .to_string();
                self.pubsub_subscriptions.filter_input_active = false;
                self.pubsub_subscriptions.selected_visible = self
                    .pubsub_subscriptions
                    .selected_visible
                    .min(self.visible_pubsub_subscriptions().len().saturating_sub(1));
                None
            }
            ResourceServiceKind::BigQueryDatasets => {
                self.bigquery_apply_filter();
                None
            }
            ResourceServiceKind::IamServiceAccounts => {
                self.service_accounts.filter_applied = CompiledResourceFilter::new(
                    self.service_accounts.filter_draft.trim().to_string(),
                );
                self.service_accounts.filter_draft =
                    self.service_accounts.filter_applied.source().to_string();
                self.service_accounts.filter_input_active = false;
                self.service_accounts.selected_visible = self
                    .service_accounts
                    .selected_visible
                    .min(self.visible_service_accounts().len().saturating_sub(1));
                None
            }
            ResourceServiceKind::IamPolicy => {
                self.policy_members.filter_applied = CompiledResourceFilter::new(
                    self.policy_members.filter_draft.trim().to_string(),
                );
                self.policy_members.filter_draft =
                    self.policy_members.filter_applied.source().to_string();
                self.policy_members.filter_input_active = false;
                self.policy_members.selected_visible = self
                    .policy_members
                    .selected_visible
                    .min(self.visible_policy_members().len().saturating_sub(1));
                None
            }
            ResourceServiceKind::CloudStorage => self.storage.apply_filter(),
        }
    }

    /// Whether the active service's applied filter is local-only, or (Cloud
    /// Storage object browser only) also narrowed the server-side listing.
    pub fn filter_scope(&self) -> FilterScope {
        match self.active_service {
            ResourceServiceKind::ComputeInstances
            | ResourceServiceKind::GkeClusters
            | ResourceServiceKind::CloudSql
            | ResourceServiceKind::CloudRunServices
            | ResourceServiceKind::SecretManagerSecrets
            | ResourceServiceKind::PubSubTopics
            | ResourceServiceKind::VpcNetworks
            | ResourceServiceKind::VpcSubnets
            | ResourceServiceKind::FirewallRules
            | ResourceServiceKind::PubSubSubscriptions
            | ResourceServiceKind::BigQueryDatasets
            | ResourceServiceKind::IamServiceAccounts
            | ResourceServiceKind::IamPolicy
            | ResourceServiceKind::CloudFunctions => FilterScope::Local,
            ResourceServiceKind::PersistentDisks => FilterScope::Local,
            ResourceServiceKind::CloudStorage => self.storage.filter_scope(),
        }
    }

    pub fn cancel_filter(&mut self) {
        match self.active_service {
            ResourceServiceKind::ComputeInstances => {
                self.compute.filter_draft = self.compute.filter_applied.source().to_string();
                self.compute.filter_input_active = false;
            }
            ResourceServiceKind::GkeClusters => {
                self.gke.filter_draft = self.gke.filter_applied.source().to_string();
                self.gke.filter_input_active = false;
            }
            ResourceServiceKind::CloudSql => {
                self.cloud_sql.filter_draft = self.cloud_sql.filter_applied.source().to_string();
                self.cloud_sql.filter_input_active = false;
            }
            ResourceServiceKind::CloudRunServices => {
                self.cloud_run.filter_draft = self.cloud_run.filter_applied.source().to_string();
                self.cloud_run.filter_input_active = false;
            }
            ResourceServiceKind::SecretManagerSecrets => {
                self.secrets.filter_draft = self.secrets.filter_applied.source().to_string();
                self.secrets.filter_input_active = false;
            }
            ResourceServiceKind::CloudFunctions => {
                self.functions.filter_draft = self.functions.filter_applied.source().to_string();
                self.functions.filter_input_active = false;
            }
            ResourceServiceKind::PersistentDisks => {
                self.disks.filter_draft = self.disks.filter_applied.source().to_string();
                self.disks.filter_input_active = false;
            }
            ResourceServiceKind::PubSubTopics => {
                self.pubsub_topics.filter_draft =
                    self.pubsub_topics.filter_applied.source().to_string();
                self.pubsub_topics.filter_input_active = false;
            }
            ResourceServiceKind::VpcNetworks => {
                self.vpc_networks.filter_draft =
                    self.vpc_networks.filter_applied.source().to_string();
                self.vpc_networks.filter_input_active = false;
            }
            ResourceServiceKind::VpcSubnets => {
                self.vpc_subnets.filter_draft =
                    self.vpc_subnets.filter_applied.source().to_string();
                self.vpc_subnets.filter_input_active = false;
            }
            ResourceServiceKind::FirewallRules => {
                self.firewall_rules.filter_draft =
                    self.firewall_rules.filter_applied.source().to_string();
                self.firewall_rules.filter_input_active = false;
            }
            ResourceServiceKind::PubSubSubscriptions => {
                self.pubsub_subscriptions.filter_draft = self
                    .pubsub_subscriptions
                    .filter_applied
                    .source()
                    .to_string();
                self.pubsub_subscriptions.filter_input_active = false;
            }
            ResourceServiceKind::BigQueryDatasets => self.bigquery_cancel_filter(),
            ResourceServiceKind::IamServiceAccounts => {
                self.service_accounts.filter_draft =
                    self.service_accounts.filter_applied.source().to_string();
                self.service_accounts.filter_input_active = false;
            }
            ResourceServiceKind::IamPolicy => {
                self.policy_members.filter_draft =
                    self.policy_members.filter_applied.source().to_string();
                self.policy_members.filter_input_active = false;
            }
            ResourceServiceKind::CloudStorage => self.storage.cancel_filter(),
        }
    }

    /// Installs validated column settings, with the sort and the column cursor
    /// anchored inside each service's new layout (`install_service_settings`).
    /// A service's configured *default* sort is installed only on the load that
    /// changes that service's block (`newly_configured_sort`), so a reload
    /// about something else leaves the sort the user reached with `s` alone.
    ///
    /// Nothing else moves: rows, filters, selection and load phase are none of
    /// this call's business, so applying configuration (at startup or on
    /// reload) can never disturb a list that is loading or already loaded.
    pub fn set_column_settings(&mut self, settings: ColumnSettings) {
        let running = &self.column_settings;
        install_service_settings(
            &mut self.compute,
            newly_configured_sort(&settings, running, ResourceServiceKind::ComputeInstances),
            service_layout(&settings, ResourceServiceKind::ComputeInstances),
            ComputeInstanceSortField::from_column,
            ComputeInstanceSortField::column,
            ComputeInstanceSortField::Name,
        );
        install_service_settings(
            &mut self.gke,
            newly_configured_sort(&settings, running, ResourceServiceKind::GkeClusters),
            service_layout(&settings, ResourceServiceKind::GkeClusters),
            GkeSortField::from_column,
            GkeSortField::column,
            GkeSortField::Name,
        );
        install_service_settings(
            &mut self.cloud_sql,
            newly_configured_sort(&settings, running, ResourceServiceKind::CloudSql),
            service_layout(&settings, ResourceServiceKind::CloudSql),
            CloudSqlInstanceSortField::from_column,
            CloudSqlInstanceSortField::column,
            CloudSqlInstanceSortField::Name,
        );
        install_service_settings(
            &mut self.cloud_run,
            newly_configured_sort(&settings, running, ResourceServiceKind::CloudRunServices),
            service_layout(&settings, ResourceServiceKind::CloudRunServices),
            CloudRunSortField::from_column,
            CloudRunSortField::column,
            CloudRunSortField::Name,
        );
        install_service_settings(
            &mut self.secrets,
            newly_configured_sort(
                &settings,
                running,
                ResourceServiceKind::SecretManagerSecrets,
            ),
            service_layout(&settings, ResourceServiceKind::SecretManagerSecrets),
            SecretSortField::from_column,
            SecretSortField::column,
            SecretSortField::Name,
        );
        install_service_settings(
            &mut self.functions,
            newly_configured_sort(&settings, running, ResourceServiceKind::CloudFunctions),
            service_layout(&settings, ResourceServiceKind::CloudFunctions),
            CloudFunctionSortField::from_column,
            CloudFunctionSortField::column,
            CloudFunctionSortField::Name,
        );
        install_service_settings(
            &mut self.disks,
            newly_configured_sort(&settings, running, ResourceServiceKind::PersistentDisks),
            service_layout(&settings, ResourceServiceKind::PersistentDisks),
            DiskSortField::from_column,
            DiskSortField::column,
            DiskSortField::Name,
        );
        install_service_settings(
            &mut self.pubsub_topics,
            newly_configured_sort(&settings, running, ResourceServiceKind::PubSubTopics),
            service_layout(&settings, ResourceServiceKind::PubSubTopics),
            PubSubTopicSortField::from_column,
            PubSubTopicSortField::column,
            PubSubTopicSortField::Name,
        );
        install_service_settings(
            &mut self.pubsub_subscriptions,
            newly_configured_sort(&settings, running, ResourceServiceKind::PubSubSubscriptions),
            service_layout(&settings, ResourceServiceKind::PubSubSubscriptions),
            PubSubSubscriptionSortField::from_column,
            PubSubSubscriptionSortField::column,
            PubSubSubscriptionSortField::Name,
        );
        install_service_settings(
            &mut self.vpc_networks,
            newly_configured_sort(&settings, running, ResourceServiceKind::VpcNetworks),
            service_layout(&settings, ResourceServiceKind::VpcNetworks),
            VpcNetworkSortField::from_column,
            VpcNetworkSortField::column,
            VpcNetworkSortField::Name,
        );
        install_service_settings(
            &mut self.vpc_subnets,
            newly_configured_sort(&settings, running, ResourceServiceKind::VpcSubnets),
            service_layout(&settings, ResourceServiceKind::VpcSubnets),
            VpcSubnetSortField::from_column,
            VpcSubnetSortField::column,
            VpcSubnetSortField::Name,
        );
        install_service_settings(
            &mut self.firewall_rules,
            newly_configured_sort(&settings, running, ResourceServiceKind::FirewallRules),
            service_layout(&settings, ResourceServiceKind::FirewallRules),
            FirewallSortField::from_column,
            FirewallSortField::column,
            FirewallSortField::Name,
        );
        install_service_settings(
            &mut self.bigquery_datasets,
            newly_configured_sort(&settings, running, ResourceServiceKind::BigQueryDatasets),
            service_layout(&settings, ResourceServiceKind::BigQueryDatasets),
            BigQuerySortField::from_column,
            BigQuerySortField::column,
            BigQuerySortField::DatasetId,
        );
        install_service_settings(
            &mut self.service_accounts,
            newly_configured_sort(&settings, running, ResourceServiceKind::IamServiceAccounts),
            service_layout(&settings, ResourceServiceKind::IamServiceAccounts),
            ServiceAccountSortField::from_column,
            ServiceAccountSortField::column,
            ServiceAccountSortField::Email,
        );
        install_service_settings(
            &mut self.policy_members,
            newly_configured_sort(&settings, running, ResourceServiceKind::IamPolicy),
            service_layout(&settings, ResourceServiceKind::IamPolicy),
            IamPolicySortField::from_column,
            IamPolicySortField::column,
            IamPolicySortField::Member,
        );
        install_service_settings(
            &mut self.storage.collection,
            newly_configured_sort(&settings, running, ResourceServiceKind::CloudStorage),
            service_layout(&settings, ResourceServiceKind::CloudStorage),
            StorageSortField::from_column,
            StorageSortField::column,
            StorageSortField::Name,
        );
        self.column_settings = settings;
    }

    /// A service's effective column layout: catalog indices in display order.
    /// An unconfigured service gets the identity layout — the whole catalog,
    /// in catalog order — which is today's table.
    pub fn column_layout(&self, kind: ResourceServiceKind) -> &[usize] {
        service_layout(&self.column_settings, kind)
    }

    /// Whether this service's column *list* came from the user's config.
    /// Only then are the emitted priorities positional (design §6): a block
    /// that sets nothing but `sort` leaves the catalog's own hand-tuned
    /// priorities, which is what an unchanged layout deserves.
    pub fn has_configured_columns(&self, kind: ResourceServiceKind) -> bool {
        self.column_settings
            .get(kind)
            .is_some_and(|service| service.columns.is_some())
    }

    pub fn selected_column(&self) -> usize {
        match self.active_service {
            ResourceServiceKind::ComputeInstances => self.compute.selected_column,
            ResourceServiceKind::GkeClusters => self.gke.selected_column,
            ResourceServiceKind::CloudSql => self.cloud_sql.selected_column,
            ResourceServiceKind::CloudRunServices => self.cloud_run.selected_column,
            ResourceServiceKind::SecretManagerSecrets => self.secrets.selected_column,
            ResourceServiceKind::CloudFunctions => self.functions.selected_column,
            ResourceServiceKind::PersistentDisks => self.disks.selected_column,
            ResourceServiceKind::PubSubTopics => self.pubsub_topics.selected_column,
            ResourceServiceKind::VpcNetworks => self.vpc_networks.selected_column,
            ResourceServiceKind::VpcSubnets => self.vpc_subnets.selected_column,
            ResourceServiceKind::FirewallRules => self.firewall_rules.selected_column,
            ResourceServiceKind::PubSubSubscriptions => self.pubsub_subscriptions.selected_column,
            ResourceServiceKind::BigQueryDatasets => self.bigquery_datasets.selected_column,
            ResourceServiceKind::IamServiceAccounts => self.service_accounts.selected_column,
            ResourceServiceKind::IamPolicy => self.policy_members.selected_column,
            ResourceServiceKind::CloudStorage => self.storage.selected_column(),
        }
    }

    pub fn sort_direction(&self) -> SortDirection {
        match self.active_service {
            ResourceServiceKind::ComputeInstances => self.compute.sort_direction,
            ResourceServiceKind::GkeClusters => self.gke.sort_direction,
            ResourceServiceKind::CloudSql => self.cloud_sql.sort_direction,
            ResourceServiceKind::CloudRunServices => self.cloud_run.sort_direction,
            ResourceServiceKind::SecretManagerSecrets => self.secrets.sort_direction,
            ResourceServiceKind::CloudFunctions => self.functions.sort_direction,
            ResourceServiceKind::PersistentDisks => self.disks.sort_direction,
            ResourceServiceKind::PubSubTopics => self.pubsub_topics.sort_direction,
            ResourceServiceKind::VpcNetworks => self.vpc_networks.sort_direction,
            ResourceServiceKind::VpcSubnets => self.vpc_subnets.sort_direction,
            ResourceServiceKind::FirewallRules => self.firewall_rules.sort_direction,
            ResourceServiceKind::PubSubSubscriptions => self.pubsub_subscriptions.sort_direction,
            ResourceServiceKind::BigQueryDatasets => self.bigquery_datasets.sort_direction,
            ResourceServiceKind::IamServiceAccounts => self.service_accounts.sort_direction,
            ResourceServiceKind::IamPolicy => self.policy_members.sort_direction,
            ResourceServiceKind::CloudStorage => self.storage.sort_direction(),
        }
    }

    /// The **display position** of the column the active collection is sorted
    /// by — where the sort arrow goes. `SortField::column()` names a catalog
    /// index, so the layout translates it.
    pub fn sort_column_index(&self) -> usize {
        let layout = self.column_layout(self.active_service);
        match self.active_service {
            ResourceServiceKind::ComputeInstances => {
                display_position(layout, self.compute.sort_field.column())
            }
            ResourceServiceKind::GkeClusters => {
                display_position(layout, self.gke.sort_field.column())
            }
            ResourceServiceKind::CloudSql => {
                display_position(layout, self.cloud_sql.sort_field.column())
            }
            ResourceServiceKind::CloudRunServices => {
                display_position(layout, self.cloud_run.sort_field.column())
            }
            ResourceServiceKind::SecretManagerSecrets => {
                display_position(layout, self.secrets.sort_field.column())
            }
            ResourceServiceKind::CloudFunctions => {
                display_position(layout, self.functions.sort_field.column())
            }
            ResourceServiceKind::PersistentDisks => {
                display_position(layout, self.disks.sort_field.column())
            }
            ResourceServiceKind::PubSubTopics => {
                display_position(layout, self.pubsub_topics.sort_field.column())
            }
            ResourceServiceKind::VpcNetworks => {
                display_position(layout, self.vpc_networks.sort_field.column())
            }
            ResourceServiceKind::VpcSubnets => {
                display_position(layout, self.vpc_subnets.sort_field.column())
            }
            ResourceServiceKind::FirewallRules => {
                display_position(layout, self.firewall_rules.sort_field.column())
            }
            ResourceServiceKind::PubSubSubscriptions => {
                display_position(layout, self.pubsub_subscriptions.sort_field.column())
            }
            ResourceServiceKind::BigQueryDatasets => {
                display_position(layout, self.bigquery_datasets.sort_field.column())
            }
            ResourceServiceKind::IamServiceAccounts => {
                display_position(layout, self.service_accounts.sort_field.column())
            }
            ResourceServiceKind::IamPolicy => {
                display_position(layout, self.policy_members.sort_field.column())
            }
            ResourceServiceKind::CloudStorage => self.storage.sort_column_index(layout),
        }
    }

    pub fn is_wide_columns(&self) -> bool {
        self.wide_columns
    }

    pub fn toggle_wide_columns(&mut self) {
        self.wide_columns = !self.wide_columns;
    }

    pub fn select_next_column(&mut self) {
        let layout = service_layout(&self.column_settings, self.active_service);
        match self.active_service {
            ResourceServiceKind::ComputeInstances => {
                self.compute.selected_column = next_sortable_column(
                    self.compute.selected_column,
                    layout,
                    ComputeInstanceSortField::from_column,
                );
            }
            ResourceServiceKind::GkeClusters => {
                self.gke.selected_column = next_sortable_column(
                    self.gke.selected_column,
                    layout,
                    GkeSortField::from_column,
                );
            }
            ResourceServiceKind::CloudSql => {
                self.cloud_sql.selected_column = next_sortable_column(
                    self.cloud_sql.selected_column,
                    layout,
                    CloudSqlInstanceSortField::from_column,
                );
            }
            ResourceServiceKind::CloudRunServices => {
                self.cloud_run.selected_column = next_sortable_column(
                    self.cloud_run.selected_column,
                    layout,
                    CloudRunSortField::from_column,
                );
            }
            ResourceServiceKind::SecretManagerSecrets => {
                self.secrets.selected_column = next_sortable_column(
                    self.secrets.selected_column,
                    layout,
                    SecretSortField::from_column,
                );
            }
            ResourceServiceKind::CloudFunctions => {
                self.functions.selected_column = next_sortable_column(
                    self.functions.selected_column,
                    layout,
                    CloudFunctionSortField::from_column,
                );
            }
            ResourceServiceKind::PersistentDisks => {
                self.disks.selected_column = next_sortable_column(
                    self.disks.selected_column,
                    layout,
                    DiskSortField::from_column,
                );
            }
            ResourceServiceKind::PubSubTopics => {
                self.pubsub_topics.selected_column = next_sortable_column(
                    self.pubsub_topics.selected_column,
                    layout,
                    PubSubTopicSortField::from_column,
                );
            }
            ResourceServiceKind::VpcNetworks => {
                self.vpc_networks.selected_column = next_sortable_column(
                    self.vpc_networks.selected_column,
                    layout,
                    VpcNetworkSortField::from_column,
                );
            }
            ResourceServiceKind::VpcSubnets => {
                self.vpc_subnets.selected_column = next_sortable_column(
                    self.vpc_subnets.selected_column,
                    layout,
                    VpcSubnetSortField::from_column,
                );
            }
            ResourceServiceKind::FirewallRules => {
                self.firewall_rules.selected_column = next_sortable_column(
                    self.firewall_rules.selected_column,
                    layout,
                    FirewallSortField::from_column,
                );
            }
            ResourceServiceKind::PubSubSubscriptions => {
                self.pubsub_subscriptions.selected_column = next_sortable_column(
                    self.pubsub_subscriptions.selected_column,
                    layout,
                    PubSubSubscriptionSortField::from_column,
                );
            }
            ResourceServiceKind::BigQueryDatasets => {
                self.bigquery_datasets.selected_column = next_sortable_column(
                    self.bigquery_datasets.selected_column,
                    layout,
                    BigQuerySortField::from_column,
                );
            }
            ResourceServiceKind::IamServiceAccounts => {
                self.service_accounts.selected_column = next_sortable_column(
                    self.service_accounts.selected_column,
                    layout,
                    ServiceAccountSortField::from_column,
                );
            }
            ResourceServiceKind::IamPolicy => {
                self.policy_members.selected_column = next_sortable_column(
                    self.policy_members.selected_column,
                    layout,
                    IamPolicySortField::from_column,
                );
            }
            ResourceServiceKind::CloudStorage => self.storage.select_next_column(layout),
        }
    }

    pub fn select_previous_column(&mut self) {
        let layout = service_layout(&self.column_settings, self.active_service);
        match self.active_service {
            ResourceServiceKind::ComputeInstances => {
                self.compute.selected_column = previous_sortable_column(
                    self.compute.selected_column,
                    layout,
                    ComputeInstanceSortField::from_column,
                );
            }
            ResourceServiceKind::GkeClusters => {
                self.gke.selected_column = previous_sortable_column(
                    self.gke.selected_column,
                    layout,
                    GkeSortField::from_column,
                );
            }
            ResourceServiceKind::CloudSql => {
                self.cloud_sql.selected_column = previous_sortable_column(
                    self.cloud_sql.selected_column,
                    layout,
                    CloudSqlInstanceSortField::from_column,
                );
            }
            ResourceServiceKind::CloudRunServices => {
                self.cloud_run.selected_column = previous_sortable_column(
                    self.cloud_run.selected_column,
                    layout,
                    CloudRunSortField::from_column,
                );
            }
            ResourceServiceKind::SecretManagerSecrets => {
                self.secrets.selected_column = previous_sortable_column(
                    self.secrets.selected_column,
                    layout,
                    SecretSortField::from_column,
                );
            }
            ResourceServiceKind::CloudFunctions => {
                self.functions.selected_column = previous_sortable_column(
                    self.functions.selected_column,
                    layout,
                    CloudFunctionSortField::from_column,
                );
            }
            ResourceServiceKind::PersistentDisks => {
                self.disks.selected_column = previous_sortable_column(
                    self.disks.selected_column,
                    layout,
                    DiskSortField::from_column,
                );
            }
            ResourceServiceKind::PubSubTopics => {
                self.pubsub_topics.selected_column = previous_sortable_column(
                    self.pubsub_topics.selected_column,
                    layout,
                    PubSubTopicSortField::from_column,
                );
            }
            ResourceServiceKind::VpcNetworks => {
                self.vpc_networks.selected_column = previous_sortable_column(
                    self.vpc_networks.selected_column,
                    layout,
                    VpcNetworkSortField::from_column,
                );
            }
            ResourceServiceKind::VpcSubnets => {
                self.vpc_subnets.selected_column = previous_sortable_column(
                    self.vpc_subnets.selected_column,
                    layout,
                    VpcSubnetSortField::from_column,
                );
            }
            ResourceServiceKind::FirewallRules => {
                self.firewall_rules.selected_column = previous_sortable_column(
                    self.firewall_rules.selected_column,
                    layout,
                    FirewallSortField::from_column,
                );
            }
            ResourceServiceKind::PubSubSubscriptions => {
                self.pubsub_subscriptions.selected_column = previous_sortable_column(
                    self.pubsub_subscriptions.selected_column,
                    layout,
                    PubSubSubscriptionSortField::from_column,
                );
            }
            ResourceServiceKind::BigQueryDatasets => {
                self.bigquery_datasets.selected_column = previous_sortable_column(
                    self.bigquery_datasets.selected_column,
                    layout,
                    BigQuerySortField::from_column,
                );
            }
            ResourceServiceKind::IamServiceAccounts => {
                self.service_accounts.selected_column = previous_sortable_column(
                    self.service_accounts.selected_column,
                    layout,
                    ServiceAccountSortField::from_column,
                );
            }
            ResourceServiceKind::IamPolicy => {
                self.policy_members.selected_column = previous_sortable_column(
                    self.policy_members.selected_column,
                    layout,
                    IamPolicySortField::from_column,
                );
            }
            ResourceServiceKind::CloudStorage => self.storage.select_previous_column(layout),
        }
    }

    /// Sorts by the column the cursor is on. `selected_column` is a display
    /// position, so it is translated through the layout before any
    /// `SortField`, which speaks catalog indices, is consulted.
    pub fn sort_selected_column(&mut self) {
        let layout = service_layout(&self.column_settings, self.active_service);
        let catalog_index = layout.get(self.selected_column()).copied();
        match self.active_service {
            ResourceServiceKind::ComputeInstances => {
                let Some(field) = catalog_index.and_then(ComputeInstanceSortField::from_column)
                else {
                    return;
                };
                if self.compute.sort_field == field {
                    self.compute.sort_direction = self.compute.sort_direction.toggled();
                } else {
                    self.compute.sort_field = field;
                    self.compute.sort_direction = SortDirection::Ascending;
                }
                self.compute.selected_visible = self
                    .compute
                    .selected_visible
                    .min(self.visible_instances().len().saturating_sub(1));
            }
            ResourceServiceKind::GkeClusters => {
                let Some(field) = catalog_index.and_then(GkeSortField::from_column) else {
                    return;
                };
                if self.gke.sort_field == field {
                    self.gke.sort_direction = self.gke.sort_direction.toggled();
                } else {
                    self.gke.sort_field = field;
                    self.gke.sort_direction = SortDirection::Ascending;
                }
                self.gke.selected_visible = self
                    .gke
                    .selected_visible
                    .min(self.visible_gke_clusters().len().saturating_sub(1));
            }
            ResourceServiceKind::CloudSql => {
                let Some(field) = catalog_index.and_then(CloudSqlInstanceSortField::from_column)
                else {
                    return;
                };
                if self.cloud_sql.sort_field == field {
                    self.cloud_sql.sort_direction = self.cloud_sql.sort_direction.toggled();
                } else {
                    self.cloud_sql.sort_field = field;
                    self.cloud_sql.sort_direction = SortDirection::Ascending;
                }
                self.cloud_sql.selected_visible = self
                    .cloud_sql
                    .selected_visible
                    .min(self.visible_cloud_sql_instances().len().saturating_sub(1));
            }
            ResourceServiceKind::CloudRunServices => {
                let Some(field) = catalog_index.and_then(CloudRunSortField::from_column) else {
                    return;
                };
                if self.cloud_run.sort_field == field {
                    self.cloud_run.sort_direction = self.cloud_run.sort_direction.toggled();
                } else {
                    self.cloud_run.sort_field = field;
                    self.cloud_run.sort_direction = SortDirection::Ascending;
                }
                self.cloud_run.selected_visible = self
                    .cloud_run
                    .selected_visible
                    .min(self.visible_cloud_run_services().len().saturating_sub(1));
            }
            ResourceServiceKind::SecretManagerSecrets => {
                let Some(field) = catalog_index.and_then(SecretSortField::from_column) else {
                    return;
                };
                if self.secrets.sort_field == field {
                    self.secrets.sort_direction = self.secrets.sort_direction.toggled();
                } else {
                    self.secrets.sort_field = field;
                    self.secrets.sort_direction = SortDirection::Ascending;
                }
                self.secrets.selected_visible = self
                    .secrets
                    .selected_visible
                    .min(self.visible_secrets().len().saturating_sub(1));
            }
            ResourceServiceKind::CloudFunctions => {
                let Some(field) = catalog_index.and_then(CloudFunctionSortField::from_column)
                else {
                    return;
                };
                if self.functions.sort_field == field {
                    self.functions.sort_direction = self.functions.sort_direction.toggled();
                } else {
                    self.functions.sort_field = field;
                    self.functions.sort_direction = SortDirection::Ascending;
                }
                self.functions.selected_visible = self
                    .functions
                    .selected_visible
                    .min(self.visible_cloud_functions().len().saturating_sub(1));
            }
            ResourceServiceKind::PersistentDisks => {
                let Some(field) = catalog_index.and_then(DiskSortField::from_column) else {
                    return;
                };
                if self.disks.sort_field == field {
                    self.disks.sort_direction = self.disks.sort_direction.toggled();
                } else {
                    self.disks.sort_field = field;
                    self.disks.sort_direction = SortDirection::Ascending;
                }
                self.disks.selected_visible = self
                    .disks
                    .selected_visible
                    .min(self.visible_persistent_disks().len().saturating_sub(1));
            }
            ResourceServiceKind::PubSubTopics => {
                let Some(field) = catalog_index.and_then(PubSubTopicSortField::from_column) else {
                    return;
                };
                if self.pubsub_topics.sort_field == field {
                    self.pubsub_topics.sort_direction = self.pubsub_topics.sort_direction.toggled();
                } else {
                    self.pubsub_topics.sort_field = field;
                    self.pubsub_topics.sort_direction = SortDirection::Ascending;
                }
                self.pubsub_topics.selected_visible = self
                    .pubsub_topics
                    .selected_visible
                    .min(self.visible_pubsub_topics().len().saturating_sub(1));
            }
            ResourceServiceKind::VpcNetworks => {
                let Some(field) = catalog_index.and_then(VpcNetworkSortField::from_column) else {
                    return;
                };
                if self.vpc_networks.sort_field == field {
                    self.vpc_networks.sort_direction = self.vpc_networks.sort_direction.toggled();
                } else {
                    self.vpc_networks.sort_field = field;
                    self.vpc_networks.sort_direction = SortDirection::Ascending;
                }
                self.vpc_networks.selected_visible = self
                    .vpc_networks
                    .selected_visible
                    .min(self.visible_vpc_networks().len().saturating_sub(1));
            }
            ResourceServiceKind::VpcSubnets => {
                let Some(field) = catalog_index.and_then(VpcSubnetSortField::from_column) else {
                    return;
                };
                if self.vpc_subnets.sort_field == field {
                    self.vpc_subnets.sort_direction = self.vpc_subnets.sort_direction.toggled();
                } else {
                    self.vpc_subnets.sort_field = field;
                    self.vpc_subnets.sort_direction = SortDirection::Ascending;
                }
                self.vpc_subnets.selected_visible = self
                    .vpc_subnets
                    .selected_visible
                    .min(self.visible_vpc_subnets().len().saturating_sub(1));
            }
            ResourceServiceKind::FirewallRules => {
                let Some(field) = catalog_index.and_then(FirewallSortField::from_column) else {
                    return;
                };
                if self.firewall_rules.sort_field == field {
                    self.firewall_rules.sort_direction =
                        self.firewall_rules.sort_direction.toggled();
                } else {
                    self.firewall_rules.sort_field = field;
                    self.firewall_rules.sort_direction = SortDirection::Ascending;
                }
                self.firewall_rules.selected_visible = self
                    .firewall_rules
                    .selected_visible
                    .min(self.visible_firewall_rules().len().saturating_sub(1));
            }
            ResourceServiceKind::PubSubSubscriptions => {
                let Some(field) = catalog_index.and_then(PubSubSubscriptionSortField::from_column)
                else {
                    return;
                };
                if self.pubsub_subscriptions.sort_field == field {
                    self.pubsub_subscriptions.sort_direction =
                        self.pubsub_subscriptions.sort_direction.toggled();
                } else {
                    self.pubsub_subscriptions.sort_field = field;
                    self.pubsub_subscriptions.sort_direction = SortDirection::Ascending;
                }
                self.pubsub_subscriptions.selected_visible = self
                    .pubsub_subscriptions
                    .selected_visible
                    .min(self.visible_pubsub_subscriptions().len().saturating_sub(1));
            }
            ResourceServiceKind::BigQueryDatasets => {
                let Some(field) = catalog_index.and_then(BigQuerySortField::from_column) else {
                    return;
                };
                if self.bigquery_datasets.sort_field == field {
                    self.bigquery_datasets.sort_direction =
                        self.bigquery_datasets.sort_direction.toggled();
                } else {
                    self.bigquery_datasets.sort_field = field;
                    self.bigquery_datasets.sort_direction = SortDirection::Ascending;
                }
                self.bigquery_datasets.selected_visible = self
                    .bigquery_datasets
                    .selected_visible
                    .min(self.visible_bigquery_datasets().len().saturating_sub(1));
            }
            ResourceServiceKind::IamServiceAccounts => {
                let Some(field) = catalog_index.and_then(ServiceAccountSortField::from_column)
                else {
                    return;
                };
                if self.service_accounts.sort_field == field {
                    self.service_accounts.sort_direction =
                        self.service_accounts.sort_direction.toggled();
                } else {
                    self.service_accounts.sort_field = field;
                    self.service_accounts.sort_direction = SortDirection::Ascending;
                }
                self.service_accounts.selected_visible = self
                    .service_accounts
                    .selected_visible
                    .min(self.visible_service_accounts().len().saturating_sub(1));
            }
            ResourceServiceKind::IamPolicy => {
                let Some(field) = catalog_index.and_then(IamPolicySortField::from_column) else {
                    return;
                };
                if self.policy_members.sort_field == field {
                    self.policy_members.sort_direction =
                        self.policy_members.sort_direction.toggled();
                } else {
                    self.policy_members.sort_field = field;
                    self.policy_members.sort_direction = SortDirection::Ascending;
                }
                self.policy_members.selected_visible = self
                    .policy_members
                    .selected_visible
                    .min(self.visible_policy_members().len().saturating_sub(1));
            }
            ResourceServiceKind::CloudStorage => self.storage.sort_selected_column(layout),
        }
    }

    pub fn select_next(&mut self) {
        match self.active_service {
            ResourceServiceKind::ComputeInstances => {
                let len = self.visible_instances().len();
                if len > 0 {
                    self.compute.selected_visible =
                        (self.compute.selected_visible + 1).min(len - 1);
                }
            }
            ResourceServiceKind::GkeClusters => {
                let len = self.visible_gke_clusters().len();
                if len > 0 {
                    self.gke.selected_visible = (self.gke.selected_visible + 1).min(len - 1);
                }
            }
            ResourceServiceKind::CloudSql => {
                let len = self.visible_cloud_sql_instances().len();
                if len > 0 {
                    self.cloud_sql.selected_visible =
                        (self.cloud_sql.selected_visible + 1).min(len - 1);
                }
            }
            ResourceServiceKind::CloudRunServices => {
                if self.is_on_cloud_run_revisions_tab() {
                    let len = self.cloud_run_revision_count();
                    if len > 0 {
                        self.selected_revision = (self.selected_revision + 1).min(len - 1);
                    }
                } else {
                    let len = self.visible_cloud_run_services().len();
                    if len > 0 {
                        self.cloud_run.selected_visible =
                            (self.cloud_run.selected_visible + 1).min(len - 1);
                    }
                }
            }
            // No revision-style cursor: the Versions tab is a list to read,
            // not a level to descend into, so `j`/`k` always move the secrets
            // list itself.
            ResourceServiceKind::SecretManagerSecrets => {
                let len = self.visible_secrets().len();
                if len > 0 {
                    self.secrets.selected_visible =
                        (self.secrets.selected_visible + 1).min(len - 1);
                }
            }
            ResourceServiceKind::CloudFunctions => {
                let len = self.visible_cloud_functions().len();
                if len > 0 {
                    self.functions.selected_visible =
                        (self.functions.selected_visible + 1).min(len - 1);
                }
            }
            ResourceServiceKind::PersistentDisks => {
                let len = self.visible_persistent_disks().len();
                if len > 0 {
                    self.disks.selected_visible = (self.disks.selected_visible + 1).min(len - 1);
                }
            }
            ResourceServiceKind::PubSubTopics => {
                let len = self.visible_pubsub_topics().len();
                if len > 0 {
                    self.pubsub_topics.selected_visible =
                        (self.pubsub_topics.selected_visible + 1).min(len - 1);
                }
            }
            ResourceServiceKind::VpcNetworks => {
                let len = self.visible_vpc_networks().len();
                if len > 0 {
                    self.vpc_networks.selected_visible =
                        (self.vpc_networks.selected_visible + 1).min(len - 1);
                }
            }
            ResourceServiceKind::VpcSubnets => {
                let len = self.visible_vpc_subnets().len();
                if len > 0 {
                    self.vpc_subnets.selected_visible =
                        (self.vpc_subnets.selected_visible + 1).min(len - 1);
                }
            }
            ResourceServiceKind::FirewallRules => {
                let len = self.visible_firewall_rules().len();
                if len > 0 {
                    self.firewall_rules.selected_visible =
                        (self.firewall_rules.selected_visible + 1).min(len - 1);
                }
            }
            ResourceServiceKind::PubSubSubscriptions => {
                let len = self.visible_pubsub_subscriptions().len();
                if len > 0 {
                    self.pubsub_subscriptions.selected_visible =
                        (self.pubsub_subscriptions.selected_visible + 1).min(len - 1);
                }
            }
            ResourceServiceKind::BigQueryDatasets => {
                self.bigquery_select_next();
            }
            ResourceServiceKind::IamServiceAccounts => {
                let len = self.visible_service_accounts().len();
                if len > 0 {
                    self.service_accounts.selected_visible =
                        (self.service_accounts.selected_visible + 1).min(len - 1);
                }
            }
            ResourceServiceKind::IamPolicy => {
                let len = self.visible_policy_members().len();
                if len > 0 {
                    self.policy_members.selected_visible =
                        (self.policy_members.selected_visible + 1).min(len - 1);
                }
            }
            ResourceServiceKind::CloudStorage => self.storage.select_next(),
        }
    }

    pub fn select_previous(&mut self) {
        match self.active_service {
            ResourceServiceKind::ComputeInstances => {
                self.compute.selected_visible = self.compute.selected_visible.saturating_sub(1);
            }
            ResourceServiceKind::GkeClusters => {
                self.gke.selected_visible = self.gke.selected_visible.saturating_sub(1);
            }
            ResourceServiceKind::CloudSql => {
                self.cloud_sql.selected_visible = self.cloud_sql.selected_visible.saturating_sub(1);
            }
            ResourceServiceKind::CloudRunServices => {
                if self.is_on_cloud_run_revisions_tab() {
                    self.selected_revision = self.selected_revision.saturating_sub(1);
                } else {
                    self.cloud_run.selected_visible =
                        self.cloud_run.selected_visible.saturating_sub(1);
                }
            }
            ResourceServiceKind::SecretManagerSecrets => {
                self.secrets.selected_visible = self.secrets.selected_visible.saturating_sub(1);
            }
            ResourceServiceKind::CloudFunctions => {
                self.functions.selected_visible = self.functions.selected_visible.saturating_sub(1);
            }
            ResourceServiceKind::PersistentDisks => {
                self.disks.selected_visible = self.disks.selected_visible.saturating_sub(1);
            }
            ResourceServiceKind::PubSubTopics => {
                self.pubsub_topics.selected_visible =
                    self.pubsub_topics.selected_visible.saturating_sub(1);
            }
            ResourceServiceKind::VpcNetworks => {
                self.vpc_networks.selected_visible =
                    self.vpc_networks.selected_visible.saturating_sub(1);
            }
            ResourceServiceKind::VpcSubnets => {
                self.vpc_subnets.selected_visible =
                    self.vpc_subnets.selected_visible.saturating_sub(1);
            }
            ResourceServiceKind::FirewallRules => {
                self.firewall_rules.selected_visible =
                    self.firewall_rules.selected_visible.saturating_sub(1);
            }
            ResourceServiceKind::PubSubSubscriptions => {
                self.pubsub_subscriptions.selected_visible =
                    self.pubsub_subscriptions.selected_visible.saturating_sub(1);
            }
            ResourceServiceKind::BigQueryDatasets => self.bigquery_select_previous(),
            ResourceServiceKind::IamServiceAccounts => {
                self.service_accounts.selected_visible =
                    self.service_accounts.selected_visible.saturating_sub(1);
            }
            ResourceServiceKind::IamPolicy => {
                self.policy_members.selected_visible =
                    self.policy_members.selected_visible.saturating_sub(1);
            }
            ResourceServiceKind::CloudStorage => self.storage.select_previous(),
        }
    }

    /// The rows of `S` in visible order: the applied filter, then the
    /// collection's sort field and direction.
    fn visible_rows<'a, S: ResourceService>(&'a self) -> Vec<&'a S::Row>
    where
        S::Row: 'a,
        S::Detail: 'a,
        S::SortField: 'a,
        S::DetailTab: 'a,
    {
        let collection = S::collection(self);
        let filter = collection.filter_applied.filter();
        let mut rows = collection
            .rows
            .iter()
            // Both, and in this order because the filter is the cheap one.
            // Inside `visible_rows` rather than in the per-service accessors
            // so that every consumer of "the visible list" — the painted
            // table, the cursor, the mark ids, `select_visible_row` — sees the
            // same list. A narrowing applied in one of them and not the others
            // would put the cursor on a row the table does not show.
            .filter(|row| S::matches_filter(row, filter) && S::matches_narrowing(row, self))
            .collect::<Vec<_>>();
        rows.sort_by(|left, right| {
            S::compare(
                left,
                right,
                collection.sort_field,
                collection.sort_direction,
            )
        });
        rows
    }

    /// The mark identities of `S`'s visible rows, in visible order.
    fn visible_row_ids<S: ResourceService>(&self) -> Vec<ResourceRowId> {
        self.visible_rows::<S>()
            .iter()
            .map(|row| S::row_id(row))
            .collect()
    }

    /// Toggles the mark on `S`'s selected visible row and makes it the new
    /// mark anchor.
    fn toggle_mark_for<S: ResourceService>(&mut self) {
        let selected_index = S::collection(self).selected_visible;
        let Some(id) = self
            .visible_rows::<S>()
            .get(selected_index)
            .map(|row| S::row_id(row))
        else {
            return;
        };
        let collection = S::collection_mut(self);
        if collection.marks.contains(&id) {
            collection.marks.remove(&id);
        } else {
            collection.marks.insert(id.clone());
        }
        collection.mark_anchor = Some(id);
    }

    /// Marks every one of `S`'s visible rows between the mark anchor and the
    /// current selection, inclusive. Falls back to [`Self::toggle_mark_for`]
    /// when there is no valid anchor in the visible list.
    fn range_mark_for<S: ResourceService>(&mut self) {
        let visible_ids = self.visible_row_ids::<S>();
        let selected_index = S::collection(self).selected_visible;
        if selected_index >= visible_ids.len() {
            return;
        }
        let anchor_index = S::collection(self)
            .mark_anchor
            .as_ref()
            .and_then(|anchor| visible_ids.iter().position(|id| id == anchor));
        let Some(anchor_index) = anchor_index else {
            self.toggle_mark_for::<S>();
            return;
        };
        let (start, end) = if anchor_index <= selected_index {
            (anchor_index, selected_index)
        } else {
            (selected_index, anchor_index)
        };
        let collection = S::collection_mut(self);
        for id in &visible_ids[start..=end] {
            collection.marks.insert(id.clone());
        }
    }

    /// Clears all of `S`'s marks and its mark anchor.
    fn clear_marks_for<S: ResourceService>(&mut self) {
        let collection = S::collection_mut(self);
        collection.marks.clear();
        collection.mark_anchor = None;
    }

    /// Removes marks from `S`'s currently visible rows only, leaving marks on
    /// filtered-out (hidden) rows untouched.
    fn unmark_visible_for<S: ResourceService>(&mut self) {
        let visible_ids = self.visible_row_ids::<S>();
        let collection = S::collection_mut(self);
        for id in &visible_ids {
            collection.marks.remove(id);
        }
    }

    /// The number of `S`'s marked rows that are currently visible.
    fn visible_marked_count_for<S: ResourceService>(&self) -> usize {
        let marks = &S::collection(self).marks;
        self.visible_rows::<S>()
            .iter()
            .filter(|row| marks.contains(&S::row_id(row)))
            .count()
    }

    /// `S`'s marked rows in visible sorted order, followed by any hidden
    /// (filtered-out) marked rows in stable order.
    fn marked_rows<S: ResourceService>(&self) -> Vec<S::Row>
    where
        S::Row: Clone,
    {
        let collection = S::collection(self);
        let mut result = Vec::new();
        let mut seen: BTreeSet<ResourceRowId> = BTreeSet::new();
        for row in self.visible_rows::<S>() {
            let id = S::row_id(row);
            if collection.marks.contains(&id) {
                result.push(row.clone());
                seen.insert(id);
            }
        }
        for id in &collection.marks {
            if !seen.contains(id)
                && let Some(row) = collection.rows.iter().find(|row| S::row_id(row) == *id)
            {
                result.push(row.clone());
            }
        }
        result
    }

    /// Drops marks (and the anchor) whose identity no longer matches any row
    /// in `S`'s freshly loaded list.
    fn prune_marks<S: ResourceService>(&mut self) {
        let collection = S::collection_mut(self);
        let rows = &collection.rows;
        let live: BTreeSet<ResourceRowId> = rows.iter().map(|row| S::row_id(row)).collect();
        collection.marks.retain(|id| live.contains(id));
        if !collection
            .mark_anchor
            .as_ref()
            .is_some_and(|anchor| live.contains(anchor))
        {
            collection.mark_anchor = None;
        }
    }

    /// Puts `S`'s list cursor on the visible row whose identity is `id`,
    /// reporting whether there was one.
    ///
    /// Over the *visible* rows rather than the raw ones, because the cursor is
    /// an index into exactly that list: positioning against the unfiltered
    /// order would land the user on a different row whenever a filter or a
    /// non-default sort is applied, which is the state a list is usually in
    /// when someone reaches for a jump. `false` means the row is filtered out
    /// or not loaded — two situations the caller words differently, so the
    /// verdict comes back rather than a status message being set here.
    fn select_visible_row_for<S: ResourceService>(&mut self, id: &ResourceRowId) -> bool {
        let Some(index) = self
            .visible_rows::<S>()
            .iter()
            .position(|row| S::row_id(row) == *id)
        else {
            return false;
        };
        S::collection_mut(self).selected_visible = index;
        true
    }

    /// Puts `kind`'s list cursor on the visible row whose identity is `id`,
    /// reporting whether there was one.
    ///
    /// The jump reducer's landing step for a `JumpTarget::List`. One arm per
    /// service and no fallthrough arm: a service added without a landing is
    /// then a compile error here rather than a jump that silently opens the
    /// right list at the wrong row.
    pub fn select_visible_row(&mut self, kind: ResourceServiceKind, id: &ResourceRowId) -> bool {
        match kind {
            ResourceServiceKind::ComputeInstances => self.select_visible_row_for::<Compute>(id),
            ResourceServiceKind::GkeClusters => self.select_visible_row_for::<Gke>(id),
            ResourceServiceKind::CloudSql => self.select_visible_row_for::<CloudSql>(id),
            ResourceServiceKind::CloudRunServices => self.select_visible_row_for::<CloudRun>(id),
            ResourceServiceKind::CloudStorage => self.select_visible_row_for::<Storage>(id),
            ResourceServiceKind::SecretManagerSecrets => self.select_visible_row_for::<Secrets>(id),
            ResourceServiceKind::CloudFunctions => {
                self.select_visible_row_for::<CloudFunctions>(id)
            }
            ResourceServiceKind::PersistentDisks => {
                self.select_visible_row_for::<PersistentDisks>(id)
            }
            ResourceServiceKind::PubSubTopics => self.select_visible_row_for::<PubSubTopics>(id),
            ResourceServiceKind::PubSubSubscriptions => {
                self.select_visible_row_for::<PubSubSubscriptions>(id)
            }
            ResourceServiceKind::BigQueryDatasets => self.select_visible_row_for::<BigQuery>(id),
            ResourceServiceKind::IamServiceAccounts => {
                self.select_visible_row_for::<IamServiceAccounts>(id)
            }
            ResourceServiceKind::IamPolicy => self.select_visible_row_for::<IamPolicy>(id),
            ResourceServiceKind::VpcNetworks => self.select_visible_row_for::<VpcNetworks>(id),
            ResourceServiceKind::VpcSubnets => self.select_visible_row_for::<VpcSubnets>(id),
            ResourceServiceKind::FirewallRules => self.select_visible_row_for::<FirewallRules>(id),
        }
    }

    /// Toggles the mark on the currently selected visible row of the active
    /// service and sets it as the new mark anchor.
    pub fn toggle_mark(&mut self) {
        match self.active_service {
            ResourceServiceKind::ComputeInstances => self.toggle_mark_for::<Compute>(),
            ResourceServiceKind::CloudSql => self.toggle_mark_for::<CloudSql>(),
            ResourceServiceKind::GkeClusters
            | ResourceServiceKind::CloudRunServices
            | ResourceServiceKind::SecretManagerSecrets
            | ResourceServiceKind::CloudFunctions
            | ResourceServiceKind::PersistentDisks
            | ResourceServiceKind::PubSubTopics
            | ResourceServiceKind::VpcNetworks
            | ResourceServiceKind::VpcSubnets
            | ResourceServiceKind::FirewallRules
            | ResourceServiceKind::PubSubSubscriptions
            | ResourceServiceKind::BigQueryDatasets
            | ResourceServiceKind::IamServiceAccounts
            | ResourceServiceKind::IamPolicy
            | ResourceServiceKind::CloudStorage => {}
        }
    }

    /// Marks every row in the active service's visible (filtered+sorted) list
    /// between the mark anchor and the current selection, inclusive. Falls
    /// back to `toggle_mark` when there is no valid anchor in the visible list.
    pub fn range_mark(&mut self) {
        match self.active_service {
            ResourceServiceKind::ComputeInstances => self.range_mark_for::<Compute>(),
            ResourceServiceKind::CloudSql => self.range_mark_for::<CloudSql>(),
            ResourceServiceKind::GkeClusters
            | ResourceServiceKind::CloudRunServices
            | ResourceServiceKind::SecretManagerSecrets
            | ResourceServiceKind::CloudFunctions
            | ResourceServiceKind::PersistentDisks
            | ResourceServiceKind::PubSubTopics
            | ResourceServiceKind::VpcNetworks
            | ResourceServiceKind::VpcSubnets
            | ResourceServiceKind::FirewallRules
            | ResourceServiceKind::PubSubSubscriptions
            | ResourceServiceKind::BigQueryDatasets
            | ResourceServiceKind::IamServiceAccounts
            | ResourceServiceKind::IamPolicy
            | ResourceServiceKind::CloudStorage => {}
        }
    }

    /// Clears all marks and the mark anchor for the active service.
    pub fn clear_marks(&mut self) {
        match self.active_service {
            ResourceServiceKind::ComputeInstances => self.clear_marks_for::<Compute>(),
            ResourceServiceKind::CloudSql => self.clear_marks_for::<CloudSql>(),
            ResourceServiceKind::GkeClusters
            | ResourceServiceKind::CloudRunServices
            | ResourceServiceKind::SecretManagerSecrets
            | ResourceServiceKind::CloudFunctions
            | ResourceServiceKind::PersistentDisks
            | ResourceServiceKind::PubSubTopics
            | ResourceServiceKind::VpcNetworks
            | ResourceServiceKind::VpcSubnets
            | ResourceServiceKind::FirewallRules
            | ResourceServiceKind::PubSubSubscriptions
            | ResourceServiceKind::BigQueryDatasets
            | ResourceServiceKind::IamServiceAccounts
            | ResourceServiceKind::IamPolicy
            | ResourceServiceKind::CloudStorage => {}
        }
    }

    /// Removes marks from the active service's currently visible rows only,
    /// leaving marks on filtered-out (hidden) rows untouched.
    pub fn unmark_visible(&mut self) {
        match self.active_service {
            ResourceServiceKind::ComputeInstances => self.unmark_visible_for::<Compute>(),
            ResourceServiceKind::CloudSql => self.unmark_visible_for::<CloudSql>(),
            ResourceServiceKind::GkeClusters
            | ResourceServiceKind::CloudRunServices
            | ResourceServiceKind::SecretManagerSecrets
            | ResourceServiceKind::CloudFunctions
            | ResourceServiceKind::PersistentDisks
            | ResourceServiceKind::PubSubTopics
            | ResourceServiceKind::VpcNetworks
            | ResourceServiceKind::VpcSubnets
            | ResourceServiceKind::FirewallRules
            | ResourceServiceKind::PubSubSubscriptions
            | ResourceServiceKind::BigQueryDatasets
            | ResourceServiceKind::IamServiceAccounts
            | ResourceServiceKind::IamPolicy
            | ResourceServiceKind::CloudStorage => {}
        }
    }

    /// Returns the total number of marked rows for the active service,
    /// including those hidden by the current filter.
    pub fn marked_count(&self) -> usize {
        match self.active_service {
            ResourceServiceKind::ComputeInstances => Compute::collection(self).marks.len(),
            ResourceServiceKind::CloudSql => CloudSql::collection(self).marks.len(),
            ResourceServiceKind::GkeClusters
            | ResourceServiceKind::CloudRunServices
            | ResourceServiceKind::SecretManagerSecrets
            | ResourceServiceKind::CloudFunctions
            | ResourceServiceKind::PersistentDisks
            | ResourceServiceKind::PubSubTopics
            | ResourceServiceKind::VpcNetworks
            | ResourceServiceKind::VpcSubnets
            | ResourceServiceKind::FirewallRules
            | ResourceServiceKind::PubSubSubscriptions
            | ResourceServiceKind::BigQueryDatasets
            | ResourceServiceKind::IamServiceAccounts
            | ResourceServiceKind::IamPolicy
            | ResourceServiceKind::CloudStorage => 0,
        }
    }

    /// Returns the number of marked rows of the active service that are
    /// currently visible.
    pub fn visible_marked_count(&self) -> usize {
        match self.active_service {
            ResourceServiceKind::ComputeInstances => self.visible_marked_count_for::<Compute>(),
            ResourceServiceKind::CloudSql => self.visible_marked_count_for::<CloudSql>(),
            ResourceServiceKind::GkeClusters
            | ResourceServiceKind::CloudRunServices
            | ResourceServiceKind::SecretManagerSecrets
            | ResourceServiceKind::CloudFunctions
            | ResourceServiceKind::PersistentDisks
            | ResourceServiceKind::PubSubTopics
            | ResourceServiceKind::VpcNetworks
            | ResourceServiceKind::VpcSubnets
            | ResourceServiceKind::FirewallRules
            | ResourceServiceKind::PubSubSubscriptions
            | ResourceServiceKind::BigQueryDatasets
            | ResourceServiceKind::IamServiceAccounts
            | ResourceServiceKind::IamPolicy
            | ResourceServiceKind::CloudStorage => 0,
        }
    }

    pub fn mark_all_stale_and_clear(&mut self) {
        self.mark_stale_and_clear();
        self.mark_gke_stale_and_clear();
        self.mark_cloud_sql_stale_and_clear();
        self.mark_cloud_run_stale_and_clear();
        self.mark_secrets_stale_and_clear();
        self.mark_cloud_functions_stale_and_clear();
        self.mark_persistent_disks_stale_and_clear();
        self.mark_pubsub_topics_stale_and_clear();
        self.mark_pubsub_subscriptions_stale_and_clear();
        self.mark_vpc_networks_stale_and_clear();
        self.mark_vpc_subnets_stale_and_clear();
        self.mark_firewall_rules_stale_and_clear();
        self.mark_bigquery_datasets_stale_and_clear();
        self.mark_service_accounts_stale_and_clear();
        self.mark_policy_members_stale_and_clear();
        self.storage.mark_stale_and_clear();
        self.close_other_modals();
        // A context/configuration switch detaches any batch-progress workflow;
        // this is the context-change teardown, unlike per-action stale marking.
        // The workflow is a progress panel rather than a modal, so it is not
        // part of the set `close_other_modals` owns.
        self.batch_workflow = None;
    }

    /// Closes the single-row and the batch confirmation, but only when
    /// `service` is the one that opened them. Both slots are shared across
    /// services while each service's stale/clear discards only its own rows,
    /// so an unconditional teardown would close a modal whose snapshotted rows
    /// are still loaded — a Cloud SQL action result tearing down an open
    /// Compute confirmation, or the reverse.
    fn close_confirmations_owned_by(&mut self, service: ResourceServiceKind) {
        if self
            .action_confirmation
            .as_ref()
            .is_some_and(|confirmation| confirmation.service_kind() == service)
        {
            self.action_confirmation = None;
        }
        if self
            .batch_confirmation
            .as_ref()
            .is_some_and(|confirmation| confirmation.action.service_kind() == service)
        {
            self.batch_confirmation = None;
        }
    }

    pub fn mark_active_stale_and_clear(&mut self, service: ResourceServiceKind) {
        match service {
            ResourceServiceKind::ComputeInstances => self.mark_stale_and_clear(),
            ResourceServiceKind::GkeClusters => self.mark_gke_stale_and_clear(),
            ResourceServiceKind::CloudSql => self.mark_cloud_sql_stale_and_clear(),
            ResourceServiceKind::CloudRunServices => self.mark_cloud_run_stale_and_clear(),
            ResourceServiceKind::SecretManagerSecrets => self.mark_secrets_stale_and_clear(),
            ResourceServiceKind::CloudFunctions => self.mark_cloud_functions_stale_and_clear(),
            ResourceServiceKind::PersistentDisks => self.mark_persistent_disks_stale_and_clear(),
            ResourceServiceKind::PubSubTopics => self.mark_pubsub_topics_stale_and_clear(),
            ResourceServiceKind::VpcNetworks => self.mark_vpc_networks_stale_and_clear(),
            ResourceServiceKind::VpcSubnets => self.mark_vpc_subnets_stale_and_clear(),
            ResourceServiceKind::FirewallRules => self.mark_firewall_rules_stale_and_clear(),
            ResourceServiceKind::PubSubSubscriptions => {
                self.mark_pubsub_subscriptions_stale_and_clear()
            }
            ResourceServiceKind::BigQueryDatasets => self.mark_bigquery_datasets_stale_and_clear(),
            ResourceServiceKind::IamServiceAccounts => self.mark_service_accounts_stale_and_clear(),
            ResourceServiceKind::IamPolicy => self.mark_policy_members_stale_and_clear(),
            ResourceServiceKind::CloudStorage => self.storage.mark_stale_and_clear(),
        }
    }

    /// Handles a configuration/context switch: caches each service's
    /// currently-loaded rows under its own context (bounded LRU, see
    /// `ContextResourceCache`), clears and marks everything stale via
    /// [`Self::mark_all_stale_and_clear`], then — if `restore` names a
    /// context previously cached — restores that context's rows as
    /// stale-but-visible so the existing freshness system drives a
    /// background refresh instead of showing an empty list.
    pub fn switch_context(&mut self, restore: Option<&GcloudContext>) {
        let restored = restore.and_then(|ctx| self.context_cache.take(ctx));

        if let Some(context) = self.compute.context.clone()
            && !self.compute.rows.is_empty()
        {
            self.context_cache.entry_mut(&context).compute = Some(self.compute.rows.clone());
        }
        if let Some(context) = self.gke.context.clone()
            && !self.gke.rows.is_empty()
        {
            self.context_cache.entry_mut(&context).gke = Some(self.gke.rows.clone());
        }
        if let Some(context) = self.cloud_sql.context.clone()
            && !self.cloud_sql.rows.is_empty()
        {
            self.context_cache.entry_mut(&context).cloud_sql = Some(self.cloud_sql.rows.clone());
        }
        if let Some(context) = self.cloud_run.context.clone()
            && !self.cloud_run.rows.is_empty()
        {
            self.context_cache.entry_mut(&context).cloud_run = Some(self.cloud_run.rows.clone());
        }
        if let Some(context) = self.storage.collection.context.clone()
            && !self.storage.collection.rows.is_empty()
        {
            self.context_cache.entry_mut(&context).storage_buckets =
                Some(self.storage.collection.rows.clone());
        }
        if let Some(context) = self.secrets.context.clone()
            && !self.secrets.rows.is_empty()
        {
            self.context_cache.entry_mut(&context).secrets = Some(self.secrets.rows.clone());
        }
        if let Some(context) = self.functions.context.clone()
            && !self.functions.rows.is_empty()
        {
            self.context_cache.entry_mut(&context).functions = Some(self.functions.rows.clone());
        }
        if let Some(context) = self.disks.context.clone()
            && !self.disks.rows.is_empty()
        {
            self.context_cache.entry_mut(&context).disks = Some(self.disks.rows.clone());
        }

        if let Some(context) = self.pubsub_topics.context.clone()
            && !self.pubsub_topics.rows.is_empty()
        {
            self.context_cache.entry_mut(&context).pubsub_topics =
                Some(self.pubsub_topics.rows.clone());
        }
        if let Some(context) = self.pubsub_subscriptions.context.clone()
            && !self.pubsub_subscriptions.rows.is_empty()
        {
            self.context_cache.entry_mut(&context).pubsub_subscriptions =
                Some(self.pubsub_subscriptions.rows.clone());
        }
        if let Some(context) = self.vpc_networks.context.clone()
            && !self.vpc_networks.rows.is_empty()
        {
            self.context_cache.entry_mut(&context).vpc_networks =
                Some(self.vpc_networks.rows.clone());
        }
        if let Some(context) = self.vpc_subnets.context.clone()
            && !self.vpc_subnets.rows.is_empty()
        {
            self.context_cache.entry_mut(&context).vpc_subnets =
                Some(self.vpc_subnets.rows.clone());
        }
        if let Some(context) = self.firewall_rules.context.clone()
            && !self.firewall_rules.rows.is_empty()
        {
            self.context_cache.entry_mut(&context).firewall_rules =
                Some(self.firewall_rules.rows.clone());
        }

        if let Some(context) = self.bigquery_datasets.context.clone()
            && !self.bigquery_datasets.rows.is_empty()
        {
            self.context_cache.entry_mut(&context).bigquery_datasets =
                Some(self.bigquery_datasets.rows.clone());
        }

        if let Some(context) = self.service_accounts.context.clone()
            && !self.service_accounts.rows.is_empty()
        {
            self.context_cache.entry_mut(&context).service_accounts =
                Some(self.service_accounts.rows.clone());
        }

        if let Some(context) = self.policy_members.context.clone()
            && !self.policy_members.rows.is_empty()
        {
            self.context_cache.entry_mut(&context).policy_members =
                Some(self.policy_members.rows.clone());
        }

        self.mark_all_stale_and_clear();

        let Some(context) = restore else {
            return;
        };
        let Some(snapshot) = restored else {
            return;
        };
        if snapshot.is_empty() {
            return;
        }

        // Destructured rather than field-accessed so a service added to
        // `ContextSnapshot` cannot be saved and then silently never restored:
        // a new field has to be named here before this compiles.
        let ContextSnapshot {
            compute,
            gke,
            cloud_sql,
            cloud_run,
            storage_buckets,
            secrets,
            functions,
            disks,
            pubsub_topics,
            pubsub_subscriptions,
            vpc_networks,
            vpc_subnets,
            firewall_rules,
            bigquery_datasets,
            service_accounts,
            policy_members,
        } = snapshot;

        if let Some(rows) = compute {
            self.compute.rows = rows;
            self.compute.context = Some(context.clone());
            self.compute.stale = true;
            self.compute.selected_visible = 0;
        }
        if let Some(rows) = gke {
            self.gke.rows = rows;
            self.gke.context = Some(context.clone());
            self.gke.stale = true;
            self.gke.selected_visible = 0;
        }
        if let Some(rows) = cloud_sql {
            self.cloud_sql.rows = rows;
            self.cloud_sql.context = Some(context.clone());
            self.cloud_sql.stale = true;
            self.cloud_sql.selected_visible = 0;
        }
        if let Some(rows) = cloud_run {
            self.cloud_run.rows = rows;
            self.cloud_run.context = Some(context.clone());
            self.cloud_run.stale = true;
            self.cloud_run.selected_visible = 0;
        }
        if let Some(buckets) = storage_buckets {
            self.storage.collection.rows = buckets;
            self.storage.collection.context = Some(context.clone());
            self.storage.collection.stale = true;
            self.storage.collection.selected_visible = 0;
        }
        if let Some(rows) = secrets {
            self.secrets.rows = rows;
            self.secrets.context = Some(context.clone());
            self.secrets.stale = true;
            self.secrets.selected_visible = 0;
        }
        if let Some(rows) = functions {
            self.functions.rows = rows;
            self.functions.context = Some(context.clone());
            self.functions.stale = true;
            self.functions.selected_visible = 0;
        }
        if let Some(rows) = disks {
            self.disks.rows = rows;
            self.disks.context = Some(context.clone());
            self.disks.stale = true;
            self.disks.selected_visible = 0;
        }
        if let Some(rows) = pubsub_topics {
            self.pubsub_topics.rows = rows;
            self.pubsub_topics.context = Some(context.clone());
            self.pubsub_topics.stale = true;
            self.pubsub_topics.selected_visible = 0;
        }
        if let Some(rows) = pubsub_subscriptions {
            self.pubsub_subscriptions.rows = rows;
            self.pubsub_subscriptions.context = Some(context.clone());
            self.pubsub_subscriptions.stale = true;
            self.pubsub_subscriptions.selected_visible = 0;
        }
        if let Some(rows) = vpc_networks {
            self.vpc_networks.rows = rows;
            self.vpc_networks.context = Some(context.clone());
            self.vpc_networks.stale = true;
            self.vpc_networks.selected_visible = 0;
        }
        if let Some(rows) = vpc_subnets {
            self.vpc_subnets.rows = rows;
            self.vpc_subnets.context = Some(context.clone());
            self.vpc_subnets.stale = true;
            self.vpc_subnets.selected_visible = 0;
        }
        if let Some(rows) = firewall_rules {
            self.firewall_rules.rows = rows;
            self.firewall_rules.context = Some(context.clone());
            self.firewall_rules.stale = true;
            self.firewall_rules.selected_visible = 0;
        }
        if let Some(rows) = bigquery_datasets {
            self.bigquery_datasets.rows = rows;
            self.bigquery_datasets.context = Some(context.clone());
            self.bigquery_datasets.stale = true;
            self.bigquery_datasets.selected_visible = 0;
        }
        if let Some(rows) = service_accounts {
            self.service_accounts.rows = rows;
            self.service_accounts.context = Some(context.clone());
            self.service_accounts.stale = true;
            self.service_accounts.selected_visible = 0;
        }
        if let Some(rows) = policy_members {
            self.policy_members.rows = rows;
            self.policy_members.context = Some(context.clone());
            self.policy_members.stale = true;
            self.policy_members.selected_visible = 0;
        }
    }

    pub fn set_service_problem(
        &mut self,
        service: ResourceServiceKind,
        problem: DiagnosticProblem,
    ) {
        match service {
            ResourceServiceKind::ComputeInstances => self.compute.last_problem = Some(problem),
            ResourceServiceKind::GkeClusters => self.gke.last_problem = Some(problem),
            ResourceServiceKind::CloudSql => self.cloud_sql.last_problem = Some(problem),
            ResourceServiceKind::CloudRunServices => self.cloud_run.last_problem = Some(problem),
            ResourceServiceKind::SecretManagerSecrets => self.secrets.last_problem = Some(problem),
            ResourceServiceKind::CloudFunctions => self.functions.last_problem = Some(problem),
            ResourceServiceKind::PersistentDisks => self.disks.last_problem = Some(problem),
            ResourceServiceKind::PubSubTopics => self.pubsub_topics.last_problem = Some(problem),
            ResourceServiceKind::VpcNetworks => self.vpc_networks.last_problem = Some(problem),
            ResourceServiceKind::VpcSubnets => self.vpc_subnets.last_problem = Some(problem),
            ResourceServiceKind::FirewallRules => self.firewall_rules.last_problem = Some(problem),
            ResourceServiceKind::PubSubSubscriptions => {
                self.pubsub_subscriptions.last_problem = Some(problem)
            }
            ResourceServiceKind::BigQueryDatasets => {
                self.bigquery_datasets.last_problem = Some(problem)
            }
            ResourceServiceKind::IamServiceAccounts => {
                self.service_accounts.last_problem = Some(problem)
            }
            ResourceServiceKind::IamPolicy => self.policy_members.last_problem = Some(problem),
            ResourceServiceKind::CloudStorage => self.storage.set_problem(problem),
        }
    }

    /// Returns the sanitized diagnostic recorded for `service`, when the most
    /// recent operation against it failed.
    pub fn service_diagnostic(&self, service: ResourceServiceKind) -> Option<&GcloudDiagnostic> {
        match service {
            ResourceServiceKind::ComputeInstances => self.compute.last_diagnostic.as_ref(),
            ResourceServiceKind::GkeClusters => self.gke.last_diagnostic.as_ref(),
            ResourceServiceKind::CloudSql => self.cloud_sql.last_diagnostic.as_ref(),
            ResourceServiceKind::CloudRunServices => self.cloud_run.last_diagnostic.as_ref(),
            ResourceServiceKind::SecretManagerSecrets => self.secrets.last_diagnostic.as_ref(),
            ResourceServiceKind::CloudFunctions => self.functions.last_diagnostic.as_ref(),
            ResourceServiceKind::PersistentDisks => self.disks.last_diagnostic.as_ref(),
            ResourceServiceKind::PubSubTopics => self.pubsub_topics.last_diagnostic.as_ref(),
            ResourceServiceKind::VpcNetworks => self.vpc_networks.last_diagnostic.as_ref(),
            ResourceServiceKind::VpcSubnets => self.vpc_subnets.last_diagnostic.as_ref(),
            ResourceServiceKind::FirewallRules => self.firewall_rules.last_diagnostic.as_ref(),
            ResourceServiceKind::PubSubSubscriptions => {
                self.pubsub_subscriptions.last_diagnostic.as_ref()
            }
            ResourceServiceKind::BigQueryDatasets => {
                self.bigquery_datasets.last_diagnostic.as_ref()
            }
            ResourceServiceKind::IamServiceAccounts => {
                self.service_accounts.last_diagnostic.as_ref()
            }
            ResourceServiceKind::IamPolicy => self.policy_members.last_diagnostic.as_ref(),
            ResourceServiceKind::CloudStorage => self.storage.diagnostic(),
        }
    }

    /// Whether `kind`'s held rows are stale, for any service rather than only
    /// the active one.
    ///
    /// [`ResourcesState::is_stale`] answers the same question about
    /// `active_service` and takes no kind, so it cannot serve the Home
    /// overview, which describes every service at once. Exhaustive on purpose:
    /// a new service must answer here before it compiles.
    pub fn service_stale(&self, kind: ResourceServiceKind) -> bool {
        match kind {
            ResourceServiceKind::ComputeInstances => self.compute.stale,
            ResourceServiceKind::GkeClusters => self.gke.stale,
            ResourceServiceKind::CloudSql => self.cloud_sql.stale,
            ResourceServiceKind::CloudRunServices => self.cloud_run.stale,
            ResourceServiceKind::SecretManagerSecrets => self.secrets.stale,
            ResourceServiceKind::CloudFunctions => self.functions.stale,
            ResourceServiceKind::PersistentDisks => self.disks.stale,
            ResourceServiceKind::PubSubTopics => self.pubsub_topics.stale,
            ResourceServiceKind::VpcNetworks => self.vpc_networks.stale,
            ResourceServiceKind::VpcSubnets => self.vpc_subnets.stale,
            ResourceServiceKind::FirewallRules => self.firewall_rules.stale,
            ResourceServiceKind::PubSubSubscriptions => self.pubsub_subscriptions.stale,
            ResourceServiceKind::BigQueryDatasets => self.bigquery_datasets.stale,
            ResourceServiceKind::IamServiceAccounts => self.service_accounts.stale,
            ResourceServiceKind::IamPolicy => self.policy_members.stale,
            ResourceServiceKind::CloudStorage => self.storage.is_stale(),
        }
    }

    /// How many rows `kind` currently holds, unfiltered.
    ///
    /// The Home overview's count comes from here rather than from summing
    /// [`service_status_counts`](Self::service_status_counts): a service with
    /// no status breakdown sums to zero however many rows it holds, which the
    /// freshness rule would then read as "loaded (empty)".
    pub fn service_row_count(&self, kind: ResourceServiceKind) -> usize {
        match kind {
            ResourceServiceKind::ComputeInstances => self.instances().len(),
            ResourceServiceKind::GkeClusters => self.gke_clusters().len(),
            ResourceServiceKind::CloudSql => self.cloud_sql_instances().len(),
            ResourceServiceKind::CloudRunServices => self.cloud_run_services().len(),
            ResourceServiceKind::SecretManagerSecrets => self.secrets().len(),
            ResourceServiceKind::CloudFunctions => self.cloud_functions().len(),
            ResourceServiceKind::PersistentDisks => self.persistent_disks().len(),
            ResourceServiceKind::PubSubTopics => self.pubsub_topics().len(),
            ResourceServiceKind::VpcNetworks => self.vpc_networks().len(),
            ResourceServiceKind::VpcSubnets => self.vpc_subnets().len(),
            ResourceServiceKind::FirewallRules => self.firewall_rules().len(),
            ResourceServiceKind::PubSubSubscriptions => self.pubsub_subscriptions().len(),
            ResourceServiceKind::BigQueryDatasets => self.bigquery_datasets().len(),
            ResourceServiceKind::IamServiceAccounts => self.service_accounts().len(),
            ResourceServiceKind::IamPolicy => self.policy_members().len(),
            ResourceServiceKind::CloudStorage => self.storage.buckets().len(),
        }
    }

    /// `kind`'s rows grouped by their status label, or empty for a service
    /// with no lifecycle-status shape.
    ///
    /// Empty is a real answer, not a gap to fill later: a Cloud Storage bucket
    /// and a secret have no status to break down, and inventing one would put
    /// a made-up word on the first screen an operator sees.
    pub fn service_status_counts(&self, kind: ResourceServiceKind) -> Vec<(String, usize)> {
        match kind {
            ResourceServiceKind::ComputeInstances => self.compute_status_counts(),
            ResourceServiceKind::GkeClusters => self.gke_status_counts(),
            ResourceServiceKind::CloudSql => self.cloud_sql_status_counts(),
            ResourceServiceKind::CloudRunServices => self.cloud_run_status_counts(),
            ResourceServiceKind::CloudFunctions => count_labels(
                self.functions
                    .rows
                    .iter()
                    .map(|function| function.state.label().to_string()),
            ),
            ResourceServiceKind::PersistentDisks => count_labels(
                self.disks
                    .rows
                    .iter()
                    .map(|disk| disk.status.label().to_string()),
            ),
            // A subscription does carry a lifecycle state, and it is the one
            // worth counting on the first screen: `RESOURCE_ERROR` means
            // Pub/Sub cannot deliver to it — a deleted push endpoint's service
            // account, most often — and that is invisible in every other
            // column. This is where subscriptions part company with topics.
            ResourceServiceKind::PubSubSubscriptions => count_labels(
                self.pubsub_subscriptions
                    .rows
                    .iter()
                    .map(|subscription| subscription.state.label().to_string()),
            ),
            // A topic has no lifecycle state to count: it exists or it does
            // not. Nor does a BigQuery dataset. Grouped with the other
            // services whose rows carry no status rather than given a
            // synthetic one.
            ResourceServiceKind::SecretManagerSecrets
            | ResourceServiceKind::PubSubTopics
            | ResourceServiceKind::VpcNetworks
            | ResourceServiceKind::VpcSubnets
            | ResourceServiceKind::FirewallRules
            | ResourceServiceKind::BigQueryDatasets
            | ResourceServiceKind::IamServiceAccounts
            | ResourceServiceKind::IamPolicy
            | ResourceServiceKind::CloudStorage => Vec::new(),
        }
    }

    /// Records a sanitized diagnostic for `service`, keeping the legacy
    /// `DiagnosticProblem` accessors (`compute_problem`/`gke_problem`, used by the
    /// Home overview) in sync via the `From<&GcloudDiagnostic>` bridge.
    pub fn set_service_diagnostic(
        &mut self,
        service: ResourceServiceKind,
        diagnostic: GcloudDiagnostic,
    ) {
        let problem = DiagnosticProblem::from(&diagnostic);
        match service {
            ResourceServiceKind::ComputeInstances => {
                self.compute.last_diagnostic = Some(diagnostic);
                self.compute.last_problem = Some(problem);
            }
            ResourceServiceKind::GkeClusters => {
                self.gke.last_diagnostic = Some(diagnostic);
                self.gke.last_problem = Some(problem);
            }
            ResourceServiceKind::CloudSql => {
                self.cloud_sql.last_diagnostic = Some(diagnostic);
                self.cloud_sql.last_problem = Some(problem);
            }
            ResourceServiceKind::CloudRunServices => {
                self.cloud_run.last_diagnostic = Some(diagnostic);
                self.cloud_run.last_problem = Some(problem);
            }
            ResourceServiceKind::SecretManagerSecrets => {
                self.secrets.last_diagnostic = Some(diagnostic);
                self.secrets.last_problem = Some(problem);
            }
            ResourceServiceKind::CloudFunctions => {
                self.functions.last_diagnostic = Some(diagnostic);
                self.functions.last_problem = Some(problem);
            }
            ResourceServiceKind::PersistentDisks => {
                self.disks.last_diagnostic = Some(diagnostic);
                self.disks.last_problem = Some(problem);
            }
            ResourceServiceKind::PubSubTopics => {
                self.pubsub_topics.last_diagnostic = Some(diagnostic);
                self.pubsub_topics.last_problem = Some(problem);
            }
            ResourceServiceKind::VpcNetworks => {
                self.vpc_networks.last_diagnostic = Some(diagnostic);
                self.vpc_networks.last_problem = Some(problem);
            }
            ResourceServiceKind::VpcSubnets => {
                self.vpc_subnets.last_diagnostic = Some(diagnostic);
                self.vpc_subnets.last_problem = Some(problem);
            }
            ResourceServiceKind::FirewallRules => {
                self.firewall_rules.last_diagnostic = Some(diagnostic);
                self.firewall_rules.last_problem = Some(problem);
            }
            ResourceServiceKind::PubSubSubscriptions => {
                self.pubsub_subscriptions.last_diagnostic = Some(diagnostic);
                self.pubsub_subscriptions.last_problem = Some(problem);
            }
            ResourceServiceKind::BigQueryDatasets => {
                self.bigquery_datasets.last_diagnostic = Some(diagnostic);
                self.bigquery_datasets.last_problem = Some(problem);
            }
            ResourceServiceKind::IamServiceAccounts => {
                self.service_accounts.last_diagnostic = Some(diagnostic);
                self.service_accounts.last_problem = Some(problem);
            }
            ResourceServiceKind::IamPolicy => {
                self.policy_members.last_diagnostic = Some(diagnostic);
                self.policy_members.last_problem = Some(problem);
            }
            ResourceServiceKind::CloudStorage => {
                self.storage.set_diagnostic(diagnostic);
            }
        }
    }

    pub fn action_confirmation_submission(&mut self) -> Option<ActionConfirmationSubmission> {
        let confirmation = self.action_confirmation.take()?;
        self.command_preview = None;
        self.ssh_launch_preview = None;
        self.tunnel_launch_preview = None;
        self.k9s_launch_confirmation = None;
        self.plugin_confirmation = None;
        self.plugin_prompt = None;
        Some(match confirmation {
            ResourceActionConfirmation::Compute(confirmation) => {
                ActionConfirmationSubmission::Compute {
                    action: confirmation.action,
                    context: confirmation.context,
                    instance: confirmation.instance,
                }
            }
            ResourceActionConfirmation::CloudSql(confirmation) => {
                ActionConfirmationSubmission::CloudSql {
                    action: confirmation.action,
                    context: confirmation.context,
                    instance: confirmation.instance,
                }
            }
        })
    }

    pub fn cancel_action_confirmation(&mut self) {
        self.action_confirmation = None;
    }

    pub fn cancel_batch_confirmation(&mut self) {
        self.batch_confirmation = None;
    }

    /// Starts a batch confirmation targeting `S`'s currently marked rows,
    /// splitting them into those eligible for `action` and those skipped
    /// (with a reason). Does nothing if no marked row is eligible.
    ///
    /// The split enumerates `S`'s own marked rows, never the active service's:
    /// a batch is always about the service whose action was chosen, so no
    /// service can count one set of marks and act on another's rows.
    pub fn start_batch_confirmation<S: BatchableService>(
        &mut self,
        context: GcloudContext,
        action: S::Action,
    ) where
        S::Row: Clone,
    {
        let mut eligible = Vec::new();
        let mut skipped = Vec::new();
        for row in self.marked_rows::<S>() {
            let row_ref = S::row_ref(&row, &context, action);
            if S::is_eligible(&row, action) {
                eligible.push(row_ref);
            } else {
                let reason = S::skip_reason(&row, &row_ref, action);
                skipped.push((row_ref, reason));
            }
        }
        if eligible.is_empty() {
            return;
        }
        let action = S::wrap(action);
        let ack = if action.is_destructive() {
            BatchAck::CountRequired {
                typed: String::new(),
            }
        } else {
            BatchAck::Simple
        };
        self.close_other_modals();
        self.batch_confirmation = Some(BatchActionConfirmation {
            action,
            context,
            eligible,
            skipped,
            ack,
            batch_confirm_scroll: 0,
        });
    }

    /// Appends a typed digit to the batch confirmation's acknowledgement.
    /// No-op when there is no batch confirmation or the ack is `Simple`.
    pub fn batch_ack_append_digit(&mut self, value: char) {
        if let Some(confirmation) = self.batch_confirmation.as_mut() {
            confirmation.ack.append_digit(value);
        }
    }

    /// Removes the last typed character from the batch confirmation's
    /// acknowledgement. No-op when there is no batch confirmation or the ack is
    /// `Simple`.
    pub fn batch_ack_backspace(&mut self) {
        if let Some(confirmation) = self.batch_confirmation.as_mut() {
            confirmation.ack.backspace();
        }
    }

    pub fn scroll_batch_confirm_down(&mut self) {
        if let Some(confirmation) = self.batch_confirmation.as_mut() {
            confirmation.scroll_down();
        }
    }

    pub fn scroll_batch_confirm_up(&mut self) {
        if let Some(confirmation) = self.batch_confirmation.as_mut() {
            confirmation.scroll_up();
        }
    }

    /// Takes the pending batch confirmation, returning the action, context and
    /// eligible targets, and clears the confirmation.
    pub fn batch_confirmation_submission(
        &mut self,
    ) -> Option<(BatchAction, GcloudContext, Vec<BatchRowRef>)> {
        let confirmation = self.batch_confirmation.take()?;
        self.command_preview = None;
        Some((
            confirmation.action,
            confirmation.context,
            confirmation.eligible,
        ))
    }

    /// Seeds a batch-progress workflow with every target `Pending`. Called when
    /// a batch action is confirmed, alongside the operations-log entries.
    ///
    /// Each target arrives paired with the id of the operations-log entry
    /// seeded for it. The pairing is a tuple rather than a second
    /// `Vec<u64>` argument because two parallel vectors make mis-pairing a
    /// silent, type-checked-clean mistake: any future edit that filters,
    /// reorders or extends one list without the other would attach a target's
    /// id to its neighbour, and the only symptom would be a cancelled batch
    /// settling the wrong operations-log entry. Carrying the id inside the
    /// tuple makes that class of edit impossible to write by accident.
    pub fn begin_batch_workflow(
        &mut self,
        action: BatchAction,
        context: GcloudContext,
        targets: Vec<(BatchRowRef, u64)>,
    ) {
        let targets = targets
            .into_iter()
            .map(|(target, operation_id)| BatchTarget {
                name: target.name,
                location: target.location,
                status: BatchTargetStatus::Pending,
                operation_id,
            })
            .collect();
        self.batch_workflow = Some(BatchWorkflow {
            action,
            context,
            targets,
        });
    }

    /// Resolves the matching batch target to `status`. A no-op when there is no
    /// active workflow or no target matches, which keeps the single-action
    /// result path unaffected.
    ///
    /// `service` is the service whose action result arrived. The workflow is a
    /// single shared slot, so a result only ever resolves a target of a
    /// workflow its own service owns — otherwise a Cloud SQL result, which
    /// identifies its instance by name alone, could resolve an identically
    /// named target of a Compute batch.
    ///
    /// `location` is `Some` for services whose target identity is
    /// `(name, location)` and whose action results carry that location back
    /// (Compute: name + zone); it is `None` for services whose action results
    /// identify a target by name alone, which then matches on the name only.
    pub fn resolve_batch_target(
        &mut self,
        service: ResourceServiceKind,
        name: &str,
        location: Option<&str>,
        status: BatchTargetStatus,
    ) {
        if let Some(workflow) = self.batch_workflow.as_mut() {
            if workflow.action.service_kind() != service {
                return;
            }
            if let Some(target) = workflow.targets.iter_mut().find(|target| {
                target.name == name && location.is_none_or(|location| target.location == location)
            }) {
                target.status = status;
            }
        }
    }

    pub fn batch_workflow(&self) -> Option<&BatchWorkflow> {
        self.batch_workflow.as_ref()
    }

    pub fn clear_batch_workflow(&mut self) {
        self.batch_workflow = None;
    }

    /// Dismisses a completed batch workflow, but only if `service` is the one
    /// that owns it. The workflow slot is shared across services, so a load
    /// result for one service must not tear down the other service's
    /// still-awaited progress panel — the same ownership rule
    /// `ResourcesState::close_confirmations_owned_by` applies to the shared
    /// confirmation slots.
    pub fn clear_completed_batch_workflow_owned_by(&mut self, service: ResourceServiceKind) {
        if self.batch_workflow.as_ref().is_some_and(|workflow| {
            workflow.is_complete() && workflow.action().service_kind() == service
        }) {
            self.batch_workflow = None;
        }
    }

    pub fn cancel_command_preview(&mut self) {
        self.command_preview = None;
    }

    /// Opens the confirmation for an argv that is ready to run, closing every
    /// other modal through `close_other_modals`.
    pub fn show_plugin_confirmation(&mut self, label: String, argv: Vec<String>, dangerous: bool) {
        self.close_other_modals();
        self.plugin_confirmation = Some(PluginConfirmation::new(label, argv, dangerous));
    }

    pub fn plugin_confirmation(&self) -> Option<&PluginConfirmation> {
        self.plugin_confirmation.as_ref()
    }

    pub fn cancel_plugin_confirmation(&mut self) {
        self.plugin_confirmation = None;
    }

    /// Takes the confirmation and returns what the effect needs. Taking rather
    /// than borrowing is what makes double-submission unrepresentable: the
    /// second `Enter` finds nothing.
    pub fn plugin_confirmation_submission(&mut self) -> Option<(String, Vec<String>)> {
        let confirmation = self.plugin_confirmation.take()?;
        Some((confirmation.label, confirmation.argv))
    }

    /// Appends a typed digit to the plugin confirmation's acknowledgement.
    /// No-op when there is no plugin confirmation or the ack is `Simple`.
    pub fn plugin_ack_append_digit(&mut self, value: char) {
        if let Some(confirmation) = self.plugin_confirmation.as_mut() {
            confirmation.ack_append_digit(value);
        }
    }

    /// Removes the last typed character from the plugin confirmation's
    /// acknowledgement. No-op when there is no plugin confirmation or the ack is
    /// `Simple`.
    pub fn plugin_ack_backspace(&mut self) {
        if let Some(confirmation) = self.plugin_confirmation.as_mut() {
            confirmation.ack_backspace();
        }
    }

    /// Opens the prompt that collects a plugin's declared inputs, closing every
    /// other modal through `close_other_modals`.
    pub fn show_plugin_prompt(&mut self, prompt: PluginPrompt) {
        self.close_other_modals();
        self.plugin_prompt = Some(prompt);
    }

    pub fn plugin_prompt(&self) -> Option<&PluginPrompt> {
        self.plugin_prompt.as_ref()
    }

    pub fn plugin_prompt_mut(&mut self) -> Option<&mut PluginPrompt> {
        self.plugin_prompt.as_mut()
    }

    pub fn cancel_plugin_prompt(&mut self) {
        self.plugin_prompt = None;
    }

    /// Takes the prompt, for the reason `plugin_confirmation_submission` takes
    /// the confirmation: a second submit must find nothing.
    pub fn take_plugin_prompt(&mut self) -> Option<PluginPrompt> {
        self.plugin_prompt.take()
    }

    /// The pending "start this tunnel?" confirmation, if one is open.
    pub fn tunnel_launch_preview(&self) -> Option<&TunnelLaunchPreview> {
        self.tunnel_launch_preview.as_ref()
    }

    /// Opens the confirmation for an already-resolved plan, closing every other
    /// modal through `close_other_modals` so exactly one is ever open.
    pub fn show_tunnel_launch_preview(
        &mut self,
        context: GcloudContext,
        plan: crate::app::TunnelLaunchPlan,
    ) {
        self.close_other_modals();
        self.tunnel_launch_preview = Some(TunnelLaunchPreview { context, plan });
    }

    pub fn cancel_tunnel_launch_preview(&mut self) {
        self.tunnel_launch_preview = None;
    }

    /// Takes the pending preview and turns it into the launch inputs, closing
    /// the modal. The plan's `claimed_ports` is still empty here — it is a
    /// snapshot of live tunnels that is only correct at confirmation time, so
    /// the reducer fills it in when it emits the effect.
    pub fn tunnel_launch_submission(
        &mut self,
    ) -> Option<(GcloudContext, crate::app::TunnelLaunchPlan)> {
        self.tunnel_launch_preview
            .take()
            .map(|preview| (preview.context, preview.plan))
    }

    pub fn close_detail(&mut self) {
        self.command_preview = None;
        self.ssh_launch_preview = None;
        self.tunnel_launch_preview = None;
        self.k9s_launch_confirmation = None;
        self.plugin_confirmation = None;
        // The per-service closers below clear the field cursor themselves; the
        // Cloud Storage one lives on `StorageState`, which cannot reach it.
        self.detail_field_index = None;
        match self.active_service {
            ResourceServiceKind::ComputeInstances => self.close_compute_detail(),
            ResourceServiceKind::GkeClusters => self.close_gke_detail(),
            ResourceServiceKind::CloudSql => self.close_cloud_sql_detail(),
            ResourceServiceKind::CloudRunServices => self.close_cloud_run_detail(),
            ResourceServiceKind::SecretManagerSecrets => self.close_secret_detail(),
            // No detail pane exists for Cloud Functions, persistent disks,
            // Pub/Sub or BigQuery yet, so there is nothing to close (see
            // [`CloudFunctions`]).
            ResourceServiceKind::CloudFunctions
            | ResourceServiceKind::PersistentDisks
            | ResourceServiceKind::PubSubTopics
            | ResourceServiceKind::VpcNetworks
            | ResourceServiceKind::VpcSubnets
            | ResourceServiceKind::FirewallRules
            | ResourceServiceKind::PubSubSubscriptions
            | ResourceServiceKind::BigQueryDatasets
            | ResourceServiceKind::IamServiceAccounts
            | ResourceServiceKind::IamPolicy => {}
            ResourceServiceKind::CloudStorage => self.storage.close_detail(),
        }
    }

    /// The instance the firewall rules table is narrowed to, if any.
    ///
    /// Public so the render layer can say *why* the table is short. A narrowed
    /// table that looked like a complete one would be the whole hazard of this
    /// feature: an operator reading "these are the rules" when they are
    /// reading "these are some of the rules".
    pub fn firewall_narrowed_to(&self) -> Option<&FirewallInstanceScope> {
        self.firewall_narrowed_to.as_ref()
    }

    /// Narrows the firewall rules table to the rules targeting `scope`, and
    /// puts the cursor at the top of what is left.
    ///
    /// The cursor moves because the row it pointed at is a row of the *old*
    /// list; leaving it would land the operator on an arbitrary rule of the
    /// new one.
    pub fn narrow_firewall_rules_to(&mut self, scope: FirewallInstanceScope) {
        self.firewall_narrowed_to = Some(scope);
        self.firewall_rules.selected_visible = 0;
    }

    /// Clears the narrowing, restoring the whole table.
    pub fn clear_firewall_narrowing(&mut self) {
        if self.firewall_narrowed_to.take().is_some() {
            self.firewall_rules.selected_visible = 0;
        }
    }

    pub fn is_showing_detail(&self) -> bool {
        match self.active_service {
            ResourceServiceKind::ComputeInstances => {
                self.compute.view_mode == ResourcesViewMode::Detail
            }
            ResourceServiceKind::GkeClusters => self.gke.view_mode == ResourcesViewMode::Detail,
            ResourceServiceKind::CloudSql => self.cloud_sql.view_mode == ResourcesViewMode::Detail,
            ResourceServiceKind::CloudRunServices => {
                self.cloud_run.view_mode == ResourcesViewMode::Detail
            }
            ResourceServiceKind::SecretManagerSecrets => {
                self.secrets.view_mode == ResourcesViewMode::Detail
            }
            // Cloud Functions has no detail pane to be showing.
            ResourceServiceKind::CloudFunctions => false,
            ResourceServiceKind::PersistentDisks => false,
            ResourceServiceKind::PubSubTopics => false,
            ResourceServiceKind::VpcNetworks => false,
            ResourceServiceKind::VpcSubnets => false,
            ResourceServiceKind::FirewallRules => false,
            ResourceServiceKind::PubSubSubscriptions => false,
            ResourceServiceKind::BigQueryDatasets => false,
            ResourceServiceKind::IamServiceAccounts => false,
            ResourceServiceKind::IamPolicy => false,
            ResourceServiceKind::CloudStorage => self.storage.is_showing_detail(),
        }
    }

    /// Whether the active service is inside a drill-down: a second list
    /// reached from a row of the first, which `Esc` leaves for that first
    /// list rather than leaving the workspace.
    ///
    /// BigQuery's two lower views are the only ones — the tables inside a
    /// dataset and the columns inside a table. The question is asked of the
    /// view being anything but the datasets list rather than of it being
    /// `Tables`, because both lower views are lists reached from a row of the
    /// list above them, and `Esc` in each leaves for that list:
    /// [`ResourcesState::leave_bigquery_drilldown`] ascends one level at a
    /// time, so the multi-level collapse ruled out below does not happen here.
    ///
    /// Cloud Storage's object browser is deliberately not counted: it ascends
    /// one prefix level at a time with `-`/`Backspace`, and routing `Esc` into
    /// it would collapse a multi-level descent back to the bucket list in a
    /// single keypress.
    pub fn is_showing_drilldown(&self) -> bool {
        self.active_service == ResourceServiceKind::BigQueryDatasets
            && self.bigquery_view() != BigQueryView::Datasets
    }

    /// Whether `Enter` on the selected row of the drill on screen opens
    /// another level below it.
    ///
    /// False on BigQuery's schema view, which is the bottom of that hierarchy:
    /// `Enter` on a column dispatches, finds neither opener claiming it, and
    /// does nothing. The frame footer asks this before hinting `Open`,
    /// because a hint is a promise about the key it names.
    ///
    /// The help catalogue cannot ask it. `HelpContext` is derived from the
    /// keymap `Context`, which carries one drill-down flag and no notion of
    /// depth — and the keymap is right not to: `Enter` dispatches at every
    /// level, and it is the reducer that finds nothing to open.
    pub fn drilldown_opens_another_level(&self) -> bool {
        self.is_showing_drilldown() && self.bigquery_view() != BigQueryView::Schema
    }

    /// The GCE instance the view currently has on screen: the detail pane's
    /// target when a pane is open, the list selection otherwise.
    ///
    /// The rule is an invariant of *what is being shown*, not of any one
    /// feature, so it lives here and every caller reads it from here — with a
    /// detail pane open the list cursor is still movable (`j`/`k` reach the
    /// collection behind the pane), and a feature resolving from the list
    /// accessor would name a resource the user is not looking at. The `L` log
    /// jump and the `J` related-resource jump both ask this question; asking it
    /// twice is how they drift into disagreeing.
    pub fn current_compute_target(&self) -> Option<&ComputeInstance> {
        if self.is_showing_detail() {
            self.detail_target()
        } else {
            self.selected_visible_instance()
        }
    }

    /// The GKE cluster on screen, by the same rule as
    /// [`current_compute_target`](Self::current_compute_target).
    pub fn current_gke_target(&self) -> Option<&GkeCluster> {
        if self.is_showing_detail() {
            self.gke_detail_target()
        } else {
            self.selected_gke_cluster()
        }
    }

    /// The Cloud SQL instance on screen, by the same rule as
    /// [`current_compute_target`](Self::current_compute_target).
    pub fn current_cloud_sql_target(&self) -> Option<&CloudSqlInstance> {
        if self.is_showing_detail() {
            self.cloud_sql_detail_target()
        } else {
            self.selected_cloud_sql_instance()
        }
    }

    /// The Cloud Run service on screen, by the same rule as
    /// [`current_compute_target`](Self::current_compute_target). Note this
    /// answers "which *service*": inside the nested revision view the pane
    /// belongs to the revision, and callers that care check
    /// [`is_showing_cloud_run_revision`](Self::is_showing_cloud_run_revision)
    /// first.
    pub fn current_cloud_run_target(&self) -> Option<&CloudRunService> {
        if self.is_showing_detail() {
            self.cloud_run_detail_target()
        } else {
            self.selected_cloud_run_service()
        }
    }

    pub fn active_detail_tab_label(&self) -> &'static str {
        match self.active_service {
            ResourceServiceKind::ComputeInstances => self.compute.active_detail_tab.label(),
            ResourceServiceKind::GkeClusters => self.gke.active_detail_tab.label(),
            ResourceServiceKind::CloudSql => self.cloud_sql.active_detail_tab.label(),
            ResourceServiceKind::CloudRunServices => self.cloud_run.active_detail_tab.label(),
            ResourceServiceKind::SecretManagerSecrets => self.secrets.active_detail_tab.label(),
            // Unreachable in practice rather than by construction: the label
            // is only ever read for a service showing a detail pane, and
            // `is_showing_detail` is `false` here for the same reason this is
            // empty — there are no tabs to name.
            ResourceServiceKind::CloudFunctions => "",
            ResourceServiceKind::PersistentDisks => "",
            ResourceServiceKind::PubSubTopics => "",
            ResourceServiceKind::VpcNetworks => "",
            ResourceServiceKind::VpcSubnets => "",
            ResourceServiceKind::FirewallRules => "",
            ResourceServiceKind::PubSubSubscriptions => "",
            ResourceServiceKind::BigQueryDatasets => "",
            ResourceServiceKind::IamServiceAccounts => "",
            ResourceServiceKind::IamPolicy => "",
            ResourceServiceKind::CloudStorage => self.storage.active_detail_tab_label(),
        }
    }

    pub fn select_next_tab(&mut self) {
        // A new tab means a new field list, for every service including the
        // storage panes routed through `StorageState`.
        self.detail_field_index = None;
        match self.active_service {
            ResourceServiceKind::ComputeInstances => {
                self.compute.detail_scroll = 0;
                let index = ComputeDetailTab::ALL
                    .iter()
                    .position(|tab| *tab == self.compute.active_detail_tab)
                    .unwrap_or(0);
                self.compute.active_detail_tab =
                    ComputeDetailTab::ALL[(index + 1) % ComputeDetailTab::ALL.len()];
            }
            ResourceServiceKind::GkeClusters => {
                self.gke.detail_scroll = 0;
                let index = GkeDetailTab::ALL
                    .iter()
                    .position(|tab| *tab == self.gke.active_detail_tab)
                    .unwrap_or(0);
                self.gke.active_detail_tab =
                    GkeDetailTab::ALL[(index + 1) % GkeDetailTab::ALL.len()];
            }
            ResourceServiceKind::CloudSql => {
                self.cloud_sql.detail_scroll = 0;
                let index = CloudSqlDetailTab::ALL
                    .iter()
                    .position(|tab| *tab == self.cloud_sql.active_detail_tab)
                    .unwrap_or(0);
                self.cloud_sql.active_detail_tab =
                    CloudSqlDetailTab::ALL[(index + 1) % CloudSqlDetailTab::ALL.len()];
            }
            ResourceServiceKind::CloudRunServices => {
                self.cloud_run.detail_scroll = 0;
                self.selected_revision = 0;
                let index = CloudRunDetailTab::ALL
                    .iter()
                    .position(|tab| *tab == self.cloud_run.active_detail_tab)
                    .unwrap_or(0);
                self.cloud_run.active_detail_tab =
                    CloudRunDetailTab::ALL[(index + 1) % CloudRunDetailTab::ALL.len()];
            }
            ResourceServiceKind::SecretManagerSecrets => {
                self.secrets.detail_scroll = 0;
                let index = SecretDetailTab::ALL
                    .iter()
                    .position(|tab| *tab == self.secrets.active_detail_tab)
                    .unwrap_or(0);
                self.secrets.active_detail_tab =
                    SecretDetailTab::ALL[(index + 1) % SecretDetailTab::ALL.len()];
            }
            // No tabs to cycle through.
            ResourceServiceKind::CloudFunctions => {}
            ResourceServiceKind::PersistentDisks => {}
            ResourceServiceKind::PubSubTopics => {}
            ResourceServiceKind::VpcNetworks => {}
            ResourceServiceKind::VpcSubnets => {}
            ResourceServiceKind::FirewallRules => {}
            ResourceServiceKind::PubSubSubscriptions => {}
            ResourceServiceKind::BigQueryDatasets => {}
            ResourceServiceKind::IamServiceAccounts => {}
            ResourceServiceKind::IamPolicy => {}
            ResourceServiceKind::CloudStorage => self.storage.select_next_tab(),
        }
    }

    /// Switches the open detail pane straight to `tab`, doing the same
    /// bookkeeping the `[`/`]` keys do: a new tab means a new field list, a
    /// fresh scroll, and — for Cloud Run — a reset revision cursor.
    ///
    /// This is what a related-resource jump uses when it lands on the pane that
    /// is already open: re-opening the pane would drop the fetched detail and
    /// re-describe the very resource on screen.
    ///
    /// A tab belonging to some *other* service is ignored. Unlike
    /// [`select_next_tab`](Self::select_next_tab) this method takes the pane to
    /// write from its argument rather than from `active_service`, so without
    /// the check a mismatched pair would move an off-screen service's tab —
    /// invisible until that service is next opened, on a tab nobody chose.
    pub fn select_detail_tab(&mut self, tab: crate::app::state::navigation::DetailTab) {
        use crate::app::state::navigation::DetailTab;

        if tab.service() != self.active_service {
            return;
        }

        self.detail_field_index = None;
        match tab {
            DetailTab::Compute(tab) => {
                self.compute.detail_scroll = 0;
                self.compute.active_detail_tab = tab;
            }
            DetailTab::Gke(tab) => {
                self.gke.detail_scroll = 0;
                self.gke.active_detail_tab = tab;
            }
            DetailTab::CloudSql(tab) => {
                self.cloud_sql.detail_scroll = 0;
                self.cloud_sql.active_detail_tab = tab;
            }
            DetailTab::CloudRun(tab) => {
                self.cloud_run.detail_scroll = 0;
                self.selected_revision = 0;
                self.cloud_run.active_detail_tab = tab;
            }
        }
    }

    pub fn select_previous_tab(&mut self) {
        self.detail_field_index = None;
        match self.active_service {
            ResourceServiceKind::ComputeInstances => {
                self.compute.detail_scroll = 0;
                let index = ComputeDetailTab::ALL
                    .iter()
                    .position(|tab| *tab == self.compute.active_detail_tab)
                    .unwrap_or(0);
                self.compute.active_detail_tab = ComputeDetailTab::ALL
                    [(index + ComputeDetailTab::ALL.len() - 1) % ComputeDetailTab::ALL.len()];
            }
            ResourceServiceKind::GkeClusters => {
                self.gke.detail_scroll = 0;
                let index = GkeDetailTab::ALL
                    .iter()
                    .position(|tab| *tab == self.gke.active_detail_tab)
                    .unwrap_or(0);
                self.gke.active_detail_tab = GkeDetailTab::ALL
                    [(index + GkeDetailTab::ALL.len() - 1) % GkeDetailTab::ALL.len()];
            }
            ResourceServiceKind::CloudSql => {
                self.cloud_sql.detail_scroll = 0;
                let index = CloudSqlDetailTab::ALL
                    .iter()
                    .position(|tab| *tab == self.cloud_sql.active_detail_tab)
                    .unwrap_or(0);
                self.cloud_sql.active_detail_tab = CloudSqlDetailTab::ALL
                    [(index + CloudSqlDetailTab::ALL.len() - 1) % CloudSqlDetailTab::ALL.len()];
            }
            ResourceServiceKind::CloudRunServices => {
                self.cloud_run.detail_scroll = 0;
                self.selected_revision = 0;
                let index = CloudRunDetailTab::ALL
                    .iter()
                    .position(|tab| *tab == self.cloud_run.active_detail_tab)
                    .unwrap_or(0);
                self.cloud_run.active_detail_tab = CloudRunDetailTab::ALL
                    [(index + CloudRunDetailTab::ALL.len() - 1) % CloudRunDetailTab::ALL.len()];
            }
            ResourceServiceKind::SecretManagerSecrets => {
                self.secrets.detail_scroll = 0;
                let index = SecretDetailTab::ALL
                    .iter()
                    .position(|tab| *tab == self.secrets.active_detail_tab)
                    .unwrap_or(0);
                self.secrets.active_detail_tab = SecretDetailTab::ALL
                    [(index + SecretDetailTab::ALL.len() - 1) % SecretDetailTab::ALL.len()];
            }
            ResourceServiceKind::CloudFunctions => {}
            ResourceServiceKind::PersistentDisks => {}
            ResourceServiceKind::PubSubTopics => {}
            ResourceServiceKind::VpcNetworks => {}
            ResourceServiceKind::VpcSubnets => {}
            ResourceServiceKind::FirewallRules => {}
            ResourceServiceKind::PubSubSubscriptions => {}
            ResourceServiceKind::BigQueryDatasets => {}
            ResourceServiceKind::IamServiceAccounts => {}
            ResourceServiceKind::IamPolicy => {}
            ResourceServiceKind::CloudStorage => self.storage.select_previous_tab(),
        }
    }

    /// Pages the detail pane down. Paging drops the field cursor: the
    /// selection pins the viewport (render-time ensure-visible would snap it
    /// straight back), so an explicit scroll is also how the operator lets a
    /// selected field go.
    pub fn scroll_detail_down(&mut self) {
        self.detail_field_index = None;
        match self.active_service {
            ResourceServiceKind::ComputeInstances => {
                self.compute.detail_scroll =
                    self.compute.detail_scroll.saturating_add(DETAIL_PAGE_SIZE);
            }
            ResourceServiceKind::GkeClusters => {
                self.gke.detail_scroll = self.gke.detail_scroll.saturating_add(DETAIL_PAGE_SIZE);
            }
            ResourceServiceKind::CloudSql => {
                self.cloud_sql.detail_scroll = self
                    .cloud_sql
                    .detail_scroll
                    .saturating_add(DETAIL_PAGE_SIZE);
            }
            ResourceServiceKind::CloudRunServices => {
                self.cloud_run.detail_scroll = self
                    .cloud_run
                    .detail_scroll
                    .saturating_add(DETAIL_PAGE_SIZE);
            }
            ResourceServiceKind::SecretManagerSecrets => {
                self.secrets.detail_scroll =
                    self.secrets.detail_scroll.saturating_add(DETAIL_PAGE_SIZE);
            }
            // Nothing to scroll: no detail pane.
            ResourceServiceKind::CloudFunctions => {}
            ResourceServiceKind::PersistentDisks => {}
            ResourceServiceKind::PubSubTopics => {}
            ResourceServiceKind::VpcNetworks => {}
            ResourceServiceKind::VpcSubnets => {}
            ResourceServiceKind::FirewallRules => {}
            ResourceServiceKind::PubSubSubscriptions => {}
            ResourceServiceKind::BigQueryDatasets => {}
            ResourceServiceKind::IamServiceAccounts => {}
            ResourceServiceKind::IamPolicy => {}
            ResourceServiceKind::CloudStorage => self.storage.scroll_detail_down(),
        }
    }

    /// Pages the detail pane up, dropping the field cursor exactly as
    /// [`Self::scroll_detail_down`] does.
    pub fn scroll_detail_up(&mut self) {
        self.detail_field_index = None;
        match self.active_service {
            ResourceServiceKind::ComputeInstances => {
                self.compute.detail_scroll =
                    self.compute.detail_scroll.saturating_sub(DETAIL_PAGE_SIZE);
            }
            ResourceServiceKind::GkeClusters => {
                self.gke.detail_scroll = self.gke.detail_scroll.saturating_sub(DETAIL_PAGE_SIZE);
            }
            ResourceServiceKind::CloudSql => {
                self.cloud_sql.detail_scroll = self
                    .cloud_sql
                    .detail_scroll
                    .saturating_sub(DETAIL_PAGE_SIZE);
            }
            ResourceServiceKind::CloudRunServices => {
                self.cloud_run.detail_scroll = self
                    .cloud_run
                    .detail_scroll
                    .saturating_sub(DETAIL_PAGE_SIZE);
            }
            ResourceServiceKind::SecretManagerSecrets => {
                self.secrets.detail_scroll =
                    self.secrets.detail_scroll.saturating_sub(DETAIL_PAGE_SIZE);
            }
            // Nothing to scroll: no detail pane.
            ResourceServiceKind::CloudFunctions => {}
            ResourceServiceKind::PersistentDisks => {}
            ResourceServiceKind::PubSubTopics => {}
            ResourceServiceKind::VpcNetworks => {}
            ResourceServiceKind::VpcSubnets => {}
            ResourceServiceKind::FirewallRules => {}
            ResourceServiceKind::PubSubSubscriptions => {}
            ResourceServiceKind::BigQueryDatasets => {}
            ResourceServiceKind::IamServiceAccounts => {}
            ResourceServiceKind::IamPolicy => {}
            ResourceServiceKind::CloudStorage => self.storage.scroll_detail_up(),
        }
    }

    pub fn detail_scroll(&self) -> u16 {
        match self.active_service {
            ResourceServiceKind::ComputeInstances => self.compute.detail_scroll,
            ResourceServiceKind::GkeClusters => self.gke.detail_scroll,
            ResourceServiceKind::CloudSql => self.cloud_sql.detail_scroll,
            ResourceServiceKind::CloudRunServices => self.cloud_run.detail_scroll,
            ResourceServiceKind::SecretManagerSecrets => self.secrets.detail_scroll,
            // No detail pane to scroll (see [`CloudFunctions`]).
            ResourceServiceKind::CloudFunctions => 0,
            ResourceServiceKind::PersistentDisks => 0,
            ResourceServiceKind::PubSubTopics => 0,
            ResourceServiceKind::VpcNetworks => 0,
            ResourceServiceKind::VpcSubnets => 0,
            ResourceServiceKind::FirewallRules => 0,
            ResourceServiceKind::PubSubSubscriptions => 0,
            ResourceServiceKind::BigQueryDatasets => 0,
            ResourceServiceKind::IamServiceAccounts => 0,
            ResourceServiceKind::IamPolicy => 0,
            ResourceServiceKind::CloudStorage => self.storage.detail_scroll(),
        }
    }

    pub fn show_service_list(&mut self, service: ResourceServiceKind) {
        self.select_service(service);
        // As in `close_detail`: covers the Cloud Storage branch, whose closer
        // cannot reach the field cursor.
        self.detail_field_index = None;
        match service {
            ResourceServiceKind::ComputeInstances => self.close_compute_detail(),
            ResourceServiceKind::GkeClusters => self.close_gke_detail(),
            ResourceServiceKind::CloudSql => self.close_cloud_sql_detail(),
            ResourceServiceKind::CloudRunServices => self.close_cloud_run_detail(),
            ResourceServiceKind::SecretManagerSecrets => self.close_secret_detail(),
            ResourceServiceKind::CloudFunctions
            | ResourceServiceKind::PersistentDisks
            | ResourceServiceKind::PubSubTopics
            | ResourceServiceKind::VpcNetworks
            | ResourceServiceKind::VpcSubnets
            | ResourceServiceKind::FirewallRules
            | ResourceServiceKind::PubSubSubscriptions
            | ResourceServiceKind::BigQueryDatasets
            | ResourceServiceKind::IamServiceAccounts
            | ResourceServiceKind::IamPolicy => {}
            ResourceServiceKind::CloudStorage => self.storage.show_service_list(),
        }
    }
}

fn count_labels(labels: impl Iterator<Item = String>) -> Vec<(String, usize)> {
    let mut counts: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
    for label in labels {
        *counts.entry(label).or_insert(0) += 1;
    }
    counts.into_iter().collect()
}

#[cfg(test)]
mod context_resource_cache_tests {
    use super::*;
    use nuvem_domain::gcp::configuration::GcloudConfigurationName;

    fn context(name: &str) -> GcloudContext {
        GcloudContext {
            configuration: GcloudConfigurationName::new(name),
            account: None,
            project: Some(format!("{name}-project")),
            region: None,
            zone: None,
        }
    }

    #[test]
    fn eviction_beyond_cap_drops_the_least_recently_used_entry() {
        let mut cache = ContextResourceCache::default();
        for i in 0..CONTEXT_CACHE_CAP {
            cache.entry_mut(&context(&format!("ctx-{i}"))).compute = Some(Vec::new());
        }

        // Cache is now at capacity; inserting one more entry evicts ctx-0,
        // the least-recently-touched entry.
        cache.entry_mut(&context("ctx-new")).compute = Some(Vec::new());

        assert!(cache.take(&context("ctx-0")).is_none());
        assert!(cache.take(&context("ctx-new")).is_some());
    }

    #[test]
    fn touching_an_entry_refreshes_its_recency_so_it_survives_eviction() {
        let mut cache = ContextResourceCache::default();
        for i in 0..CONTEXT_CACHE_CAP {
            cache.entry_mut(&context(&format!("ctx-{i}"))).compute = Some(Vec::new());
        }

        // Touch ctx-0 again so it becomes most-recently-used instead of
        // least-recently-used.
        cache.entry_mut(&context("ctx-0"));

        // Inserting one more entry should now evict ctx-1 (the new LRU)
        // instead of ctx-0.
        cache.entry_mut(&context("ctx-new")).compute = Some(Vec::new());

        assert!(cache.take(&context("ctx-0")).is_some());
        assert!(cache.take(&context("ctx-1")).is_none());
    }

    #[test]
    fn take_consumes_the_entry() {
        let mut cache = ContextResourceCache::default();
        let ctx = context("dev");
        cache.entry_mut(&ctx).compute = Some(Vec::new());

        assert!(cache.take(&ctx).is_some());
        assert!(cache.take(&ctx).is_none());
    }
}

#[cfg(test)]
mod switch_context_tests {
    use super::*;
    use nuvem_domain::gcp::configuration::GcloudConfigurationName;
    use nuvem_domain::gcp::resource::{
        CloudFunctionEnvironment, CloudFunctionState, FirewallAction, FirewallDirection,
        PubSubDelivery, PubSubSubscriptionState, SubscriptionCount, VpcRoutingMode, VpcSubnetMode,
    };

    fn context(name: &str) -> GcloudContext {
        GcloudContext {
            configuration: GcloudConfigurationName::new(name),
            account: None,
            project: Some(format!("{name}-project")),
            region: None,
            zone: None,
        }
    }

    fn instance(name: &str) -> ComputeInstance {
        ComputeInstance {
            id: name.to_string(),
            name: name.to_string(),
            zone: "us-central1-a".to_string(),
            status: ResourceStatus::Running,
            machine_type: "e2-small".to_string(),
            // Populated, unlike the rest of this module's fixtures, because
            // `every_detail_backed_field_is_declared_and_absent_without_its_detail`
            // reads this row to decide whether a field resolves *from the row*.
            // Left as `None` they would look exactly like a field wired to a
            // detail and not declared as one, and the walk would need an
            // exception list — which is the hand-maintained table the walk
            // exists to replace.
            internal_ip: Some("10.0.0.2".to_string()),
            external_ip: Some("34.1.2.3".to_string()),
        }
    }

    #[test]
    fn restore_target_survives_outgoing_capture_when_cache_is_full() {
        let mut state = ResourcesState::default();
        let target = context("target");

        // Seed the restore target first so it sits at the least-recently-used
        // front, then fill the cache to capacity with other contexts.
        state.context_cache.entry_mut(&target).compute = Some(vec![instance("target-vm")]);
        for i in 0..CONTEXT_CACHE_CAP - 1 {
            state
                .context_cache
                .entry_mut(&context(&format!("filler-{i}")))
                .compute = Some(vec![instance(&format!("filler-vm-{i}"))]);
        }

        // Active/outgoing context is not cached, so capturing it inserts a
        // fresh entry that would evict the LRU front (the restore target).
        let outgoing = context("outgoing");
        state.compute.context = Some(outgoing);
        state.compute.rows = vec![instance("outgoing-vm")];

        state.switch_context(Some(&target));

        assert_eq!(state.compute.context.as_ref(), Some(&target));
        assert_eq!(state.compute.rows, vec![instance("target-vm")]);
        assert!(state.compute.stale);
    }

    fn secret(name: &str) -> Secret {
        Secret {
            name: name.to_string(),
            create_time: None,
            labels: Vec::new(),
            replication: "Automatic".to_string(),
        }
    }

    fn cluster(name: &str) -> GkeCluster {
        GkeCluster {
            name: name.to_string(),
            location: "europe-west1".to_string(),
            status: GkeClusterStatus::Running,
            current_master_version: None,
            current_node_count: None,
            endpoint_visibility: EndpointVisibility::Public,
        }
    }

    fn sql_instance(name: &str) -> CloudSqlInstance {
        CloudSqlInstance {
            name: name.to_string(),
            database_version: "POSTGRES_15".to_string(),
            engine: "PostgreSQL".to_string(),
            region: "europe-west1".to_string(),
            tier: "db-f1-micro".to_string(),
            availability: CloudSqlAvailability::Zonal,
            status: CloudSqlStatus::Runnable,
            connectivity: EndpointVisibility::Public,
        }
    }

    fn run_service(name: &str) -> CloudRunService {
        CloudRunService {
            name: name.to_string(),
            region: "europe-west1".to_string(),
            url: None,
            latest_ready_revision: None,
            latest_created_revision: None,
            traffic_summary: "100% latest".to_string(),
            status: CloudRunServiceStatus::Ready,
            ready_message: None,
        }
    }

    fn bucket(name: &str) -> StorageBucket {
        StorageBucket {
            name: name.to_string(),
            location: "EU".to_string(),
            location_type: Some("multi-region".to_string()),
            storage_class: "STANDARD".to_string(),
            versioning_enabled: false,
            uniform_bucket_level_access: true,
            public_access_prevention: None,
            time_created: None,
            updated: None,
        }
    }

    fn cloud_function(name: &str) -> CloudFunction {
        CloudFunction {
            name: name.to_string(),
            region: "europe-west1".to_string(),
            state: CloudFunctionState::Active,
            environment: CloudFunctionEnvironment::Gen2,
            url: None,
            update_time: None,
            labels: Vec::new(),
        }
    }

    use nuvem_domain::gcp::resource::DiskStatus;

    fn persistent_disk(name: &str) -> PersistentDisk {
        PersistentDisk {
            name: name.to_string(),
            location: "europe-west1-b".to_string(),
            status: DiskStatus::Ready,
            size_gb: 20,
            disk_type: "pd-ssd".to_string(),
            users: Vec::new(),
            labels: Vec::new(),
        }
    }
    fn pubsub_topic(name: &str) -> PubSubTopic {
        PubSubTopic {
            name: name.to_string(),
            full_name: format!("projects/demo/topics/{name}"),
            subscription_count: SubscriptionCount::Known(0),
            message_retention: None,
            kms_key: None,
            labels: Vec::new(),
        }
    }
    fn firewall_rule(name: &str) -> FirewallRule {
        FirewallRule {
            name: name.to_string(),
            network: "default".to_string(),
            direction: FirewallDirection::Ingress,
            priority: Some(1000),
            action: FirewallAction::Allow,
            targets: FirewallTargets::AllInstances,
            source_or_dest: vec!["0.0.0.0/0".to_string()],
            disabled: false,
        }
    }
    fn vpc_subnet(name: &str) -> VpcSubnet {
        VpcSubnet {
            name: name.to_string(),
            region: "us-central1".to_string(),
            network: "default".to_string(),
            ip_cidr_range: "10.128.0.0/20".to_string(),
            gateway_address: Some("10.128.0.1".to_string()),
            private_google_access: false,
        }
    }
    fn vpc_network(name: &str) -> VpcNetwork {
        VpcNetwork {
            name: name.to_string(),
            subnet_mode: VpcSubnetMode::Auto,
            routing_mode: VpcRoutingMode::Regional,
            subnet_count: 0,
            description: None,
        }
    }
    fn pubsub_subscription(name: &str) -> PubSubSubscription {
        PubSubSubscription {
            name: name.to_string(),
            full_name: format!("projects/demo/subscriptions/{name}"),
            topic: format!("projects/demo/topics/{name}"),
            delivery: PubSubDelivery::Pull,
            ack_deadline_seconds: Some(10),
            message_retention: None,
            dead_letter: None,
            state: PubSubSubscriptionState::Active,
            labels: Vec::new(),
        }
    }
    fn bigquery_dataset(dataset_id: &str) -> BigQueryDataset {
        BigQueryDataset {
            dataset_id: dataset_id.to_string(),
            project_id: "demo".to_string(),
            location: "US".to_string(),
            friendly_name: None,
            labels: Vec::new(),
        }
    }
    fn policy_member(name: &str) -> IamPolicyMember {
        IamPolicyMember::from_wire("roles/viewer", &format!("user:{name}@example.com"), None)
    }
    fn service_account(name: &str) -> ServiceAccount {
        ServiceAccount {
            email: format!("{name}@demo.iam.gserviceaccount.com"),
            display_name: None,
            disabled: false,
            unique_id: "100000000000000000001".to_string(),
        }
    }

    /// Loads one row of `kind` under `context`, straight onto the collection.
    ///
    /// Exhaustive on the kind on purpose. Seeding is the half of a context
    /// test that walking `ResourceServiceKind::ALL` cannot check by itself: a
    /// walk that reads every service but seeds only some passes for a service
    /// whose rows were never there to be dropped, which is precisely how a
    /// service missing from `switch_context`'s capture loop or from
    /// `ContextSnapshot::is_empty` would survive one.
    fn seed_one_row(
        state: &mut ResourcesState,
        kind: ResourceServiceKind,
        context: &GcloudContext,
    ) {
        match kind {
            ResourceServiceKind::ComputeInstances => {
                state.compute.context = Some(context.clone());
                state.compute.rows = vec![instance("vm")];
            }
            ResourceServiceKind::GkeClusters => {
                state.gke.context = Some(context.clone());
                state.gke.rows = vec![cluster("cluster")];
            }
            ResourceServiceKind::CloudSql => {
                state.cloud_sql.context = Some(context.clone());
                state.cloud_sql.rows = vec![sql_instance("db")];
            }
            ResourceServiceKind::CloudRunServices => {
                state.cloud_run.context = Some(context.clone());
                state.cloud_run.rows = vec![run_service("svc")];
            }
            ResourceServiceKind::SecretManagerSecrets => {
                state.secrets.context = Some(context.clone());
                state.secrets.rows = vec![secret("api-signing-key")];
            }
            ResourceServiceKind::CloudFunctions => {
                state.functions.context = Some(context.clone());
                state.functions.rows = vec![cloud_function("thumbnailer")];
            }
            ResourceServiceKind::PersistentDisks => {
                state.disks.context = Some(context.clone());
                state.disks.rows = vec![persistent_disk("detached-legacy")];
            }
            ResourceServiceKind::PubSubTopics => {
                state.pubsub_topics.context = Some(context.clone());
                state.pubsub_topics.rows = vec![pubsub_topic("orders")];
            }
            ResourceServiceKind::VpcNetworks => {
                state.vpc_networks.context = Some(context.clone());
                state.vpc_networks.rows = vec![vpc_network("orders")];
            }
            ResourceServiceKind::VpcSubnets => {
                state.vpc_subnets.context = Some(context.clone());
                state.vpc_subnets.rows = vec![vpc_subnet("orders")];
            }
            ResourceServiceKind::FirewallRules => {
                state.firewall_rules.context = Some(context.clone());
                state.firewall_rules.rows = vec![firewall_rule("orders")];
            }
            ResourceServiceKind::PubSubSubscriptions => {
                state.pubsub_subscriptions.context = Some(context.clone());
                state.pubsub_subscriptions.rows = vec![pubsub_subscription("orders")];
            }
            ResourceServiceKind::BigQueryDatasets => {
                state.bigquery_datasets.context = Some(context.clone());
                state.bigquery_datasets.rows = vec![bigquery_dataset("orders")];
            }
            ResourceServiceKind::IamServiceAccounts => {
                state.service_accounts.context = Some(context.clone());
                state.service_accounts.rows = vec![service_account("orders")];
            }
            ResourceServiceKind::IamPolicy => {
                state.policy_members.context = Some(context.clone());
                state.policy_members.rows = vec![policy_member("orders")];
            }
            ResourceServiceKind::CloudStorage => {
                state.storage.collection.context = Some(context.clone());
                state.storage.collection.rows = vec![bucket("assets")];
            }
        }
    }

    /// Seeds `count` rows for `kind`, so a walk can move a cursor rather than
    /// only observe one.
    ///
    /// Exhaustive on the kind for the same reason `seed_one_row` is: a walk
    /// that reads every service but seeds only some passes vacuously for the
    /// services it never filled.
    fn seed_rows(
        state: &mut ResourcesState,
        kind: ResourceServiceKind,
        context: &GcloudContext,
        count: usize,
    ) {
        let names: Vec<String> = (0..count).map(|index| format!("row-{index}")).collect();
        let names = || names.iter().map(String::as_str);
        match kind {
            ResourceServiceKind::ComputeInstances => {
                state.compute.context = Some(context.clone());
                state.compute.rows = names().map(instance).collect();
            }
            ResourceServiceKind::GkeClusters => {
                state.gke.context = Some(context.clone());
                state.gke.rows = names().map(cluster).collect();
            }
            ResourceServiceKind::CloudSql => {
                state.cloud_sql.context = Some(context.clone());
                state.cloud_sql.rows = names().map(sql_instance).collect();
            }
            ResourceServiceKind::CloudRunServices => {
                state.cloud_run.context = Some(context.clone());
                state.cloud_run.rows = names().map(run_service).collect();
            }
            ResourceServiceKind::SecretManagerSecrets => {
                state.secrets.context = Some(context.clone());
                state.secrets.rows = names().map(secret).collect();
            }
            ResourceServiceKind::CloudFunctions => {
                state.functions.context = Some(context.clone());
                state.functions.rows = names().map(cloud_function).collect();
            }
            ResourceServiceKind::PersistentDisks => {
                state.disks.context = Some(context.clone());
                state.disks.rows = names().map(persistent_disk).collect();
            }
            ResourceServiceKind::PubSubTopics => {
                state.pubsub_topics.context = Some(context.clone());
                state.pubsub_topics.rows = names().map(pubsub_topic).collect();
            }
            ResourceServiceKind::VpcNetworks => {
                state.vpc_networks.context = Some(context.clone());
                state.vpc_networks.rows = names().map(vpc_network).collect();
            }
            ResourceServiceKind::VpcSubnets => {
                state.vpc_subnets.context = Some(context.clone());
                state.vpc_subnets.rows = names().map(vpc_subnet).collect();
            }
            ResourceServiceKind::FirewallRules => {
                state.firewall_rules.context = Some(context.clone());
                state.firewall_rules.rows = names().map(firewall_rule).collect();
            }
            ResourceServiceKind::PubSubSubscriptions => {
                state.pubsub_subscriptions.context = Some(context.clone());
                state.pubsub_subscriptions.rows = names().map(pubsub_subscription).collect();
            }
            ResourceServiceKind::BigQueryDatasets => {
                state.bigquery_datasets.context = Some(context.clone());
                state.bigquery_datasets.rows = names().map(bigquery_dataset).collect();
            }
            ResourceServiceKind::IamServiceAccounts => {
                state.service_accounts.context = Some(context.clone());
                state.service_accounts.rows = names().map(service_account).collect();
            }
            ResourceServiceKind::IamPolicy => {
                state.policy_members.context = Some(context.clone());
                state.policy_members.rows = names().map(policy_member).collect();
            }
            ResourceServiceKind::CloudStorage => {
                state.storage.collection.context = Some(context.clone());
                state.storage.collection.rows = names().map(bucket).collect();
            }
        }
    }

    /// A state with `count` rows of `kind` loaded against `context`, with that
    /// service on screen — the shape every cursor walk below starts from.
    fn showing(kind: ResourceServiceKind, context: &GcloudContext, count: usize) -> ResourcesState {
        let mut state = ResourcesState::default();
        state.select_service(kind);
        seed_rows(&mut state, kind, context, count);
        state
    }

    /// The raw cursor `select_next` writes, read without going through
    /// `selected_visible_index`.
    ///
    /// The indirection matters: reading the cursor through
    /// `selected_visible_index` would make the `select_next` walk fail
    /// whenever *that* function is wrong, so a defect in one arm would be
    /// reported against the other. Each walk asserts on the state its own
    /// function owns.
    fn cursor(state: &ResourcesState, kind: ResourceServiceKind) -> usize {
        match kind {
            ResourceServiceKind::ComputeInstances => state.compute.selected_visible,
            ResourceServiceKind::GkeClusters => state.gke.selected_visible,
            ResourceServiceKind::CloudSql => state.cloud_sql.selected_visible,
            ResourceServiceKind::CloudRunServices => state.cloud_run.selected_visible,
            ResourceServiceKind::SecretManagerSecrets => state.secrets.selected_visible,
            ResourceServiceKind::CloudFunctions => state.functions.selected_visible,
            ResourceServiceKind::PersistentDisks => state.disks.selected_visible,
            ResourceServiceKind::PubSubTopics => state.pubsub_topics.selected_visible,
            ResourceServiceKind::VpcNetworks => state.vpc_networks.selected_visible,
            ResourceServiceKind::VpcSubnets => state.vpc_subnets.selected_visible,
            ResourceServiceKind::FirewallRules => state.firewall_rules.selected_visible,
            ResourceServiceKind::PubSubSubscriptions => state.pubsub_subscriptions.selected_visible,
            ResourceServiceKind::BigQueryDatasets => state.bigquery_datasets.selected_visible,
            ResourceServiceKind::IamServiceAccounts => state.service_accounts.selected_visible,
            ResourceServiceKind::IamPolicy => state.policy_members.selected_visible,
            ResourceServiceKind::CloudStorage => state.storage.collection.selected_visible,
        }
    }

    /// The identities of `kind`'s visible rows, in visible order.
    ///
    /// A hand-written dispatch like [`cursor`] above, and for the same reason:
    /// it must not be `select_visible_row`'s own arm table, or a walk over it
    /// would be comparing that function with itself and a service wired to the
    /// wrong collection would "agree".
    fn visible_row_ids(state: &ResourcesState, kind: ResourceServiceKind) -> Vec<ResourceRowId> {
        match kind {
            ResourceServiceKind::ComputeInstances => state.visible_row_ids::<Compute>(),
            ResourceServiceKind::GkeClusters => state.visible_row_ids::<Gke>(),
            ResourceServiceKind::CloudSql => state.visible_row_ids::<CloudSql>(),
            ResourceServiceKind::CloudRunServices => state.visible_row_ids::<CloudRun>(),
            ResourceServiceKind::CloudStorage => state.visible_row_ids::<Storage>(),
            ResourceServiceKind::SecretManagerSecrets => state.visible_row_ids::<Secrets>(),
            ResourceServiceKind::CloudFunctions => state.visible_row_ids::<CloudFunctions>(),
            ResourceServiceKind::PersistentDisks => state.visible_row_ids::<PersistentDisks>(),
            ResourceServiceKind::PubSubTopics => state.visible_row_ids::<PubSubTopics>(),
            ResourceServiceKind::PubSubSubscriptions => {
                state.visible_row_ids::<PubSubSubscriptions>()
            }
            ResourceServiceKind::BigQueryDatasets => state.visible_row_ids::<BigQuery>(),
            ResourceServiceKind::IamServiceAccounts => {
                state.visible_row_ids::<IamServiceAccounts>()
            }
            ResourceServiceKind::IamPolicy => state.visible_row_ids::<IamPolicy>(),
            ResourceServiceKind::VpcNetworks => state.visible_row_ids::<VpcNetworks>(),
            ResourceServiceKind::VpcSubnets => state.visible_row_ids::<VpcSubnets>(),
            ResourceServiceKind::FirewallRules => state.visible_row_ids::<FirewallRules>(),
        }
    }

    /// Every service lands a jump on the row it names, and says so when there
    /// is no such row.
    ///
    /// An ALL-walk rather than a VPC example, because `select_visible_row` is
    /// sixteen hand-written arms and the failure it exists to prevent is one
    /// arm wired to a neighbouring service's collection — which a
    /// three-service test cannot see. Each row is visited from a *different*
    /// cursor position than the one it should end on, so a landing that never
    /// moved the cursor cannot pass by starting in the right place.
    #[test]
    fn every_service_lands_a_jump_on_the_row_it_names() {
        let context = context("dev");
        for kind in ResourceServiceKind::ALL {
            let mut state = showing(kind, &context, 3);
            let ids = visible_row_ids(&state, kind);
            assert_eq!(ids.len(), 3, "{kind:?} seeded no rows to land on");

            // Backwards, so every step is a move and the last step is not the
            // index the cursor already sat on.
            for (index, id) in ids.iter().enumerate().rev() {
                assert!(
                    state.select_visible_row(kind, id),
                    "{kind:?} cannot find its own row {id:?}"
                );
                assert_eq!(
                    cursor(&state, kind),
                    index,
                    "{kind:?} landed on the wrong row for {id:?}"
                );
            }

            let absent = ResourceRowId {
                name: "no-such-row".to_string(),
                location: "no-such-place".to_string(),
            };
            assert!(
                !state.select_visible_row(kind, &absent),
                "{kind:?} claims to have landed on a row it does not have"
            );
            assert_eq!(
                cursor(&state, kind),
                0,
                "{kind:?} moved its cursor for a row it did not find"
            );
        }
    }

    /// [`row_identity`] against what `row_id` actually builds, for all sixteen
    /// services.
    ///
    /// `row_identity` is a hand-written table consulted at config-load time,
    /// when no list has rows; this is the only thing keeping it in step with
    /// the sixteen `row_id` implementations it describes. It is not decoration:
    /// the first version of that predicate was derived from
    /// `config::jumps::location_field` and this walk is what caught it being
    /// wrong for BigQuery datasets (a declared `{location}` field, a row
    /// identity that is the dataset id alone) and for the IAM policy table (no
    /// location field, a row identity whose second half is a role).
    ///
    /// The three cases are told apart by two observable facts rather than by
    /// restating the table: whether the built id has a location half at all,
    /// and whether the service declares a location *field* a template could
    /// render into it.
    #[test]
    fn every_service_keys_rows_by_location_exactly_when_it_has_one() {
        let context = context("dev");
        for kind in ResourceServiceKind::ALL {
            let state = showing(kind, &context, 1);
            let id = visible_row_ids(&state, kind)
                .into_iter()
                .next()
                .unwrap_or_else(|| panic!("{kind:?} seeded no row"));

            let expected = if id.location.is_empty() {
                RowIdentity::Name
            } else if crate::config::jumps::location_field(kind).is_some() {
                RowIdentity::NameAndLocation
            } else {
                RowIdentity::Opaque
            };
            assert_eq!(
                row_identity(kind),
                expected,
                "{kind:?} builds the row identity {id:?}"
            );
        }
    }

    /// Every detail-backed field is declared, is absent without its detail, and
    /// belongs to a service that has a detail at all (slice 2).
    ///
    /// The sibling of `every_service_keys_rows_by_location_exactly_when_it_has_one`,
    /// guarding the same class of mistake one level down. Slice 1's was a
    /// predicate about a *destination* that did not match what `row_id` builds;
    /// this one is about a *source* field that does not come from where the
    /// vocabulary implies. `config::jumps::detail_backed_fields` is a
    /// hand-written table read while classifying a diagnostic, and nothing else
    /// keeps it honest against the sixteen `selected_fields` arms.
    ///
    /// Every state here has rows and no detail anywhere, which is the state `J`
    /// is usually pressed in — so a field wired to the row while declared
    /// detail-backed shows up as a value that is present when it must not be.
    #[test]
    fn every_detail_backed_field_is_declared_and_absent_without_its_detail() {
        use crate::app::state::jumps::{lookup, selected_fields};
        use crate::app::state::navigation::DetailTab;
        use crate::config::jumps::{detail_backed_fields, fields_for};

        let context = context("dev");
        let mut saw_one = false;
        for kind in ResourceServiceKind::ALL {
            let mut state = showing(kind, &context, 1);
            state.select_service(kind);
            let fields = selected_fields(&state, kind)
                .unwrap_or_else(|| panic!("{kind:?} resolved no fields for a seeded row"));

            // The other direction, and the one that catches a field wired to a
            // detail while declared row-backed: with no detail loaded, every
            // field the vocabulary offers and the table does *not* call
            // detail-backed has to resolve from the row alone. A field that
            // does not is either detail-backed and undeclared, or a promise
            // `selected_fields` cannot keep — and both are the wiring gap.
            for (name, field) in fields_for(kind) {
                if detail_backed_fields(kind).contains(field) {
                    continue;
                }
                assert!(
                    lookup(&fields, *field).is_some(),
                    "{kind:?}'s {name:?} is not declared detail-backed but has no \
                     value on a fully populated row"
                );
            }

            for field in detail_backed_fields(kind) {
                saw_one = true;
                assert!(
                    fields_for(kind).iter().any(|(_, known)| known == field),
                    "{kind:?} declares {field:?} detail-backed but offers no such field"
                );
                assert!(
                    DetailTab::ALL.iter().any(|tab| tab.service() == kind),
                    "{kind:?} has no detail pane, so {field:?} cannot be read off one"
                );
                assert_eq!(
                    lookup(&fields, *field),
                    None,
                    "{kind:?}'s {field:?} resolved from the row, so it is not detail-backed"
                );
            }
        }
        assert!(
            saw_one,
            "no service declares a detail-backed field, so this test asserts nothing"
        );
    }

    /// Every service asks for a refresh exactly when what is on screen is not
    /// this context's fresh rows.
    ///
    /// `needs_refresh` is a per-service arm of `!=` and `||`, and three of the
    /// seven arms had nothing pinning either operator: flipping `!=` to `==`
    /// (never refresh the context you are looking at, always refresh the one
    /// you are not) and `||` to `&&` (a stale list never refreshes while you
    /// sit on it) both survived. The three cases below are one per operand, so
    /// a wrong answer names which.
    #[test]
    fn every_service_refreshes_exactly_when_its_rows_are_wrong() {
        let dev = context("dev");
        let prod = context("prod");

        for kind in ResourceServiceKind::ALL {
            let mut state = showing(kind, &dev, 1);

            assert!(
                !state.needs_refresh(&dev),
                "{kind:?} wants to refetch rows it already holds for this context"
            );
            assert!(
                state.needs_refresh(&prod),
                "{kind:?} would show another configuration's rows under this one's header"
            );

            // Back through the context cache, which is how a list becomes
            // visible-but-stale: same context, rows still on screen, refresh
            // still owed.
            state.switch_context(Some(&prod));
            state.switch_context(Some(&dev));
            assert_eq!(
                rows_and_staleness(&state, kind),
                (1, true),
                "{kind:?} did not come back from the context cache stale, so the \
                 next assertion would not be about staleness at all"
            );
            assert!(
                state.needs_refresh(&dev),
                "{kind:?} sits on a stale list without refreshing it"
            );
        }
    }

    /// Every service reports a selected row exactly when it has rows.
    ///
    /// The arm is `(!visible.is_empty()).then_some(..)`, and deleting the `!`
    /// inverts it: a selection out of an empty list, and none out of a full
    /// one. Both halves are asserted because either alone leaves the other
    /// direction free.
    #[test]
    fn every_service_reports_a_selection_exactly_when_it_has_rows() {
        let dev = context("dev");

        for kind in ResourceServiceKind::ALL {
            let mut state = ResourcesState::default();
            state.select_service(kind);
            assert_eq!(
                state.selected_visible_index(),
                None,
                "{kind:?} reported a selected row with nothing on screen"
            );

            seed_rows(&mut state, kind, &dev, 3);
            assert_eq!(
                state.selected_visible_index(),
                Some(0),
                "{kind:?} reported no selection with three rows on screen"
            );
        }
    }

    /// Every service's projected selection follows the projected row count.
    ///
    /// `(visible_rows > 0).then_some(..)` is the arm, and the comparison is
    /// the whole of it: `>=` mints a selection into an empty projection, `==`
    /// and `<` withhold one from a full projection. Zero and non-zero are both
    /// asserted because no single case rules out all three.
    #[test]
    fn every_service_projects_a_selection_exactly_when_rows_are_projected() {
        let dev = context("dev");

        for kind in ResourceServiceKind::ALL {
            let state = showing(kind, &dev, 3);

            assert_eq!(
                state.selected_visible_index_for_visible_rows(0),
                None,
                "{kind:?} selected a row in a projection that has none"
            );
            assert_eq!(
                state.selected_visible_index_for_visible_rows(1),
                Some(0),
                "{kind:?} withheld the selection from a projection that has a row"
            );
            assert_eq!(
                state.selected_visible_index_for_visible_rows(3),
                Some(0),
                "{kind:?} withheld the selection from a full projection"
            );
        }
    }

    /// Every service's cursor advances one row and stops on the last.
    ///
    /// The arm is `if len > 0 { selected = (selected + 1).min(len - 1) }`, and
    /// each piece fails differently: `>` as `>=` reaches `len - 1` with
    /// `len == 0` and underflows, and `-` as `+` or `/` raises the clamp above
    /// the last index, so the cursor walks off the end of the list. The empty
    /// list and the last row are therefore both walked, per service.
    #[test]
    fn every_service_advances_one_row_and_stops_on_the_last() {
        let dev = context("dev");

        for kind in ResourceServiceKind::ALL {
            // An empty list: there is no last index to clamp to, so asking the
            // cursor to move must leave it alone rather than compute one.
            let mut empty = ResourcesState::default();
            empty.select_service(kind);
            empty.select_next();
            assert_eq!(
                cursor(&empty, kind),
                0,
                "{kind:?} moved its cursor on an empty list"
            );

            let mut state = showing(kind, &dev, 3);
            assert_eq!(cursor(&state, kind), 0, "{kind:?} did not start on row 0");

            state.select_next();
            assert_eq!(
                cursor(&state, kind),
                1,
                "{kind:?} did not advance from row 0 to row 1"
            );

            state.select_next();
            assert_eq!(
                cursor(&state, kind),
                2,
                "{kind:?} did not advance from row 1 to its last row"
            );

            state.select_next();
            assert_eq!(
                cursor(&state, kind),
                2,
                "{kind:?} moved its cursor past its last row, where no row is"
            );
        }
    }

    /// Every service's cursor steps back one row and stops on the first.
    ///
    /// The counterpart to the walk above, and the third case named.
    /// `select_previous` clamps with `saturating_sub`, so the failure it
    /// guards against is not an underflow but a cursor that either refuses to
    /// move or wraps to the end of the list.
    #[test]
    fn every_service_steps_back_one_row_and_stops_on_the_first() {
        let dev = context("dev");

        for kind in ResourceServiceKind::ALL {
            let mut empty = ResourcesState::default();
            empty.select_service(kind);
            empty.select_previous();
            assert_eq!(
                cursor(&empty, kind),
                0,
                "{kind:?} moved its cursor on an empty list"
            );

            let mut state = showing(kind, &dev, 3);
            state.select_next();
            state.select_next();
            assert_eq!(
                cursor(&state, kind),
                2,
                "{kind:?} did not reach its last row, so the walk back asserts nothing"
            );

            state.select_previous();
            assert_eq!(
                cursor(&state, kind),
                1,
                "{kind:?} did not step back from its last row to row 1"
            );

            state.select_previous();
            assert_eq!(
                cursor(&state, kind),
                0,
                "{kind:?} did not step back from row 1 to its first row"
            );

            state.select_previous();
            assert_eq!(
                cursor(&state, kind),
                0,
                "{kind:?} moved its cursor off the top of its list"
            );
        }
    }

    /// How many rows `kind` currently holds, and whether they are stale.
    fn rows_and_staleness(state: &ResourcesState, kind: ResourceServiceKind) -> (usize, bool) {
        match kind {
            ResourceServiceKind::ComputeInstances => {
                (state.compute.rows.len(), state.compute.stale)
            }
            ResourceServiceKind::GkeClusters => (state.gke.rows.len(), state.gke.stale),
            ResourceServiceKind::CloudSql => (state.cloud_sql.rows.len(), state.cloud_sql.stale),
            ResourceServiceKind::CloudRunServices => {
                (state.cloud_run.rows.len(), state.cloud_run.stale)
            }
            ResourceServiceKind::SecretManagerSecrets => {
                (state.secrets.rows.len(), state.secrets.stale)
            }
            ResourceServiceKind::CloudFunctions => {
                (state.functions.rows.len(), state.functions.stale)
            }
            ResourceServiceKind::PersistentDisks => (state.disks.rows.len(), state.disks.stale),
            ResourceServiceKind::PubSubTopics => {
                (state.pubsub_topics.rows.len(), state.pubsub_topics.stale)
            }
            ResourceServiceKind::VpcNetworks => {
                (state.vpc_networks.rows.len(), state.vpc_networks.stale)
            }
            ResourceServiceKind::VpcSubnets => {
                (state.vpc_subnets.rows.len(), state.vpc_subnets.stale)
            }
            ResourceServiceKind::FirewallRules => {
                (state.firewall_rules.rows.len(), state.firewall_rules.stale)
            }
            ResourceServiceKind::PubSubSubscriptions => (
                state.pubsub_subscriptions.rows.len(),
                state.pubsub_subscriptions.stale,
            ),
            ResourceServiceKind::BigQueryDatasets => (
                state.bigquery_datasets.rows.len(),
                state.bigquery_datasets.stale,
            ),
            ResourceServiceKind::IamServiceAccounts => (
                state.service_accounts.rows.len(),
                state.service_accounts.stale,
            ),
            ResourceServiceKind::IamPolicy => {
                (state.policy_members.rows.len(), state.policy_members.stale)
            }
            ResourceServiceKind::CloudStorage => (
                state.storage.collection.rows.len(),
                state.storage.collection.stale,
            ),
        }
    }

    /// Every service's rows survive a round trip through the context cache.
    ///
    /// This is the invariant broken twice in one feature, and the
    /// compiler caught neither time, because neither half is a match on the
    /// enum: `switch_context`'s capture loop is a sequence of `if let` blocks,
    /// `ContextSnapshot::is_empty` is a hand-written conjunction, and
    /// `mark_all_stale_and_clear` is a sequence of calls. Only the *restore*
    /// half is compiler-checked, by the exhaustive destructure — so a service
    /// can be saved and never restored, or restored and never saved, and
    /// everything still builds.
    ///
    /// Driven off `ResourceServiceKind::ALL` with both the seeding and the
    /// reading exhaustive on the kind, so all four places have to know about a
    /// new service before this compiles *and* before it passes.
    #[test]
    fn every_service_survives_a_round_trip_through_the_context_cache() {
        for kind in ResourceServiceKind::ALL {
            let mut state = ResourcesState::default();
            let dev = context("dev");
            let prod = context("prod");
            seed_one_row(&mut state, kind, &dev);
            assert_eq!(
                rows_and_staleness(&state, kind).0,
                1,
                "{kind:?} was not seeded, so this iteration would assert nothing"
            );

            state.switch_context(Some(&prod));
            assert_eq!(
                rows_and_staleness(&state, kind).0,
                0,
                "{kind:?} kept the previous configuration's rows on screen"
            );

            state.switch_context(Some(&dev));
            let (rows, stale) = rows_and_staleness(&state, kind);
            assert_eq!(
                rows, 1,
                "{kind:?} was not restored from the context cache, so switching \
                 back to a project shows an empty table until the refresh lands"
            );
            assert!(
                stale,
                "{kind:?} was restored as fresh, so no background refresh will run"
            );
            assert_eq!(
                state.loaded_list_context(kind),
                Some(&dev),
                "{kind:?} was restored without the context stamp it belongs to"
            );
        }
    }

    /// A context switch must discard every service's rows, not most of them.
    ///
    /// Driven off [`ResourceServiceKind::ALL`] rather than a hand-written list,
    /// because a service left out of `mark_all_stale_and_clear` keeps the
    /// previous configuration's rows on screen — another project's resources,
    /// under the new project's header — until the background refresh lands.
    #[test]
    fn switching_context_clears_the_rows_of_every_service() {
        let mut state = ResourcesState::default();
        let dev = context("dev");

        for kind in ResourceServiceKind::ALL {
            seed_one_row(&mut state, kind, &dev);
        }

        state.switch_context(Some(&context("prod")));

        for kind in ResourceServiceKind::ALL {
            let rows = match kind {
                ResourceServiceKind::ComputeInstances => state.compute.rows.len(),
                ResourceServiceKind::GkeClusters => state.gke.rows.len(),
                ResourceServiceKind::CloudSql => state.cloud_sql.rows.len(),
                ResourceServiceKind::CloudRunServices => state.cloud_run.rows.len(),
                ResourceServiceKind::SecretManagerSecrets => state.secrets.rows.len(),
                ResourceServiceKind::CloudFunctions => state.functions.rows.len(),
                ResourceServiceKind::PersistentDisks => state.disks.rows.len(),
                ResourceServiceKind::PubSubTopics => state.pubsub_topics.rows.len(),
                ResourceServiceKind::VpcNetworks => state.vpc_networks.rows.len(),
                ResourceServiceKind::VpcSubnets => state.vpc_subnets.rows.len(),
                ResourceServiceKind::FirewallRules => state.firewall_rules.rows.len(),
                ResourceServiceKind::PubSubSubscriptions => state.pubsub_subscriptions.rows.len(),
                ResourceServiceKind::BigQueryDatasets => state.bigquery_datasets.rows.len(),
                ResourceServiceKind::IamServiceAccounts => state.service_accounts.rows.len(),
                ResourceServiceKind::IamPolicy => state.policy_members.rows.len(),
                ResourceServiceKind::CloudStorage => state.storage.collection.rows.len(),
            };
            assert_eq!(rows, 0, "{kind:?} kept its rows across a context switch");
            assert!(
                state.loaded_list_context(kind).is_none(),
                "{kind:?} kept its loaded-context stamp across a context switch"
            );
        }
    }

    /// Secrets are cached across a context switch like every other service's
    /// rows: switching away captures them, and switching back repaints them as
    /// stale-but-visible rather than showing the empty placeholder while the
    /// background refresh runs.
    #[test]
    fn secrets_survive_a_round_trip_through_the_context_cache() {
        let mut state = ResourcesState::default();
        let dev = context("dev");
        let prod = context("prod");

        state.secrets.context = Some(dev.clone());
        state.secrets.rows = vec![secret("api-signing-key")];

        state.switch_context(Some(&prod));
        assert!(
            state.secrets.rows.is_empty(),
            "switching away clears the outgoing context's rows"
        );

        state.switch_context(Some(&dev));

        assert_eq!(state.secrets.context.as_ref(), Some(&dev));
        assert_eq!(state.secrets.rows, vec![secret("api-signing-key")]);
        assert!(
            state.secrets.stale,
            "restored rows are visible but stale, so a refresh still fires"
        );
    }

    /// Every service must be able to answer the three questions a Home
    /// inventory line asks. Driven off `ResourceServiceKind::ALL`: a service
    /// added without an arm cannot compile, and one wired to the wrong
    /// collection reports zero rows here while its own table shows one.
    #[test]
    fn every_service_reports_its_own_row_count_and_staleness() {
        let dev = context("dev");

        for kind in ResourceServiceKind::ALL {
            let mut state = ResourcesState::default();
            assert_eq!(
                state.service_row_count(kind),
                0,
                "{kind:?} reported rows before anything was seeded"
            );
            assert!(
                !state.service_stale(kind),
                "{kind:?} was stale before anything was loaded"
            );

            seed_one_row(&mut state, kind, &dev);
            assert_eq!(
                state.service_row_count(kind),
                1,
                "{kind:?} does not count the row it was just seeded with"
            );

            state.switch_context(Some(&context("prod")));
            state.switch_context(Some(&dev));
            assert!(
                state.service_stale(kind),
                "{kind:?} came back from the context cache reporting fresh"
            );
        }
    }

    /// Status counts are optional per service, but they must describe the rows
    /// that service actually holds. A service wired to another's collection
    /// would pass a "non-empty" assertion and fail this one.
    #[test]
    fn status_counts_describe_the_services_own_rows() {
        let dev = context("dev");

        for kind in ResourceServiceKind::ALL {
            let mut state = ResourcesState::default();
            seed_one_row(&mut state, kind, &dev);
            let counts = state.service_status_counts(kind);
            let total: usize = counts.iter().map(|(_, count)| count).sum();
            assert!(
                counts.is_empty() || total == 1,
                "{kind:?} counted {total} statuses for a single seeded row: {counts:?}"
            );
        }
    }

    /// Cloud Functions has a state field and a State column, so its line shows
    /// a breakdown rather than a bare count.
    #[test]
    fn cloud_functions_status_counts_are_by_state() {
        let mut state = ResourcesState::default();
        seed_one_row(
            &mut state,
            ResourceServiceKind::CloudFunctions,
            &context("dev"),
        );

        assert_eq!(
            state.service_status_counts(ResourceServiceKind::CloudFunctions),
            vec![("Active".to_string(), 1)]
        );
    }
}

#[cfg(test)]
mod compiled_filter_cache_tests {
    use super::*;

    /// The compiled filter is a cache, so reading it must hand back the *same*
    /// value every time rather than compiling a fresh one per call — that is
    /// the whole point of the filter cache, and `visible_rows` runs on every render.
    ///
    /// Asserted on identity rather than on elapsed time: a timing assertion
    /// would be a flake, and pointer identity is exactly the property "no
    /// recompile happened" means here.
    #[test]
    fn reading_the_compiled_filter_twice_returns_the_same_cached_value() {
        let mut state = ResourcesState::default();
        state.compute.filter_applied = CompiledResourceFilter::new("^api-".to_string());

        let first = state.compute.filter_applied.filter();
        let second = state.compute.filter_applied.filter();

        assert!(std::ptr::eq(first, second));
    }

    /// The same holds for a pattern that does not compile: the fallback to
    /// "narrows nothing" must also be a cached value, not one built per read.
    #[test]
    fn reading_an_uncompilable_filter_twice_returns_the_same_fallback() {
        let mut state = ResourcesState::default();
        state.compute.filter_applied = CompiledResourceFilter::new("a(b".to_string());

        let first = state.compute.filter_applied.filter();
        let second = state.compute.filter_applied.filter();

        assert!(matches!(first, CompiledFilter::None));
        assert!(std::ptr::eq(first, second));
    }
}

#[cfg(test)]
mod command_line_tests {
    use super::*;
    use nuvem_domain::gcp::configuration::{GcloudConfigurationName, GcloudContext};

    fn context() -> GcloudContext {
        GcloudContext {
            configuration: GcloudConfigurationName::new("dev"),
            account: None,
            project: Some("dev-project".to_string()),
            region: None,
            zone: None,
        }
    }

    /// Every argv the app builds today renders byte-for-byte as it did before
    /// display quoting existed. This is the evidence it is safe to land
    /// ahead of its consumer: the safe set covers every literal the builders
    /// emit and every value they interpolate (a GCP resource name, a zone or
    /// region, a configuration name), so quoting never engages on this tree.
    ///
    /// The cases walk `ALL` rather than a hand-written variant list so a
    /// lifecycle action added later is covered without editing this test.
    #[test]
    fn every_builtin_argv_renders_unquoted() {
        for action in ComputeInstanceAction::ALL {
            let argv = compute_lifecycle_argv(&context(), "web-01", "us-central1-a", action);
            let rendered = gcloud_command_line(&argv);

            assert_eq!(
                rendered,
                format!("gcloud {}", argv.join(" ")),
                "compute argv for {action:?} gained quoting"
            );
            assert!(
                !rendered.contains('\''),
                "compute argv for {action:?} rendered a quote: {rendered}"
            );
        }

        for action in CloudSqlInstanceAction::ALL {
            let argv = cloud_sql_lifecycle_argv(&context(), "primary-db", action);
            let rendered = gcloud_command_line(&argv);

            assert_eq!(
                rendered,
                format!("gcloud {}", argv.join(" ")),
                "cloud sql argv for {action:?} gained quoting"
            );
            assert!(
                !rendered.contains('\''),
                "cloud sql argv for {action:?} rendered a quote: {rendered}"
            );
        }
    }

    /// One fully spelled-out expectation, so the test above cannot pass by
    /// comparing a broken renderer against itself.
    #[test]
    fn a_compute_stop_renders_the_expected_line() {
        let argv = compute_lifecycle_argv(
            &context(),
            "web-01",
            "us-central1-a",
            ComputeInstanceAction::Stop,
        );

        assert_eq!(
            gcloud_command_line(&argv),
            "gcloud --configuration dev compute instances stop web-01 --zone us-central1-a"
        );
    }

    #[test]
    fn an_argument_containing_a_space_renders_as_one_argument() {
        let argv = vec!["compute".to_string(), "my vm".to_string()];

        assert_eq!(gcloud_command_line(&argv), "gcloud compute 'my vm'");
    }

    #[test]
    fn an_empty_argument_is_visible() {
        let argv = vec!["a".to_string(), String::new(), "b".to_string()];

        assert_eq!(gcloud_command_line(&argv), "gcloud a '' b");
    }

    #[test]
    fn quoting_covers_each_class_of_argument() {
        let cases = [
            // (argument, rendering)
            ("plain", "plain"),
            ("a.b_c-d/e=f:g@h", "a.b_c-d/e=f:g@h"),
            ("", "''"),
            ("my vm", "'my vm'"),
            ("it's", "'it'\\''s'"),
            ("say \"hi\"", "'say \"hi\"'"),
            ("a$b", "'a$b'"),
            ("a;b", "'a;b'"),
            ("a`b`", "'a`b`'"),
            ("a\nb", "'a\\nb'"),
            ("a\tb", "'a\\tb'"),
            ("a\rb", "'a\\rb'"),
            ("\u{1b}[31m", "'\\x1B[31m'"),
            ("a\u{7f}b", "'a\\x7Fb'"),
            ("delete\u{202e}etaerc", "'delete\\u{202E}etaerc'"),
            ("web\u{200b}01", "'web\\u{200B}01'"),
            ("\u{feff}web-01", "'\\u{FEFF}web-01'"),
            ("soft\u{ad}hyphen", "'soft\\xADhyphen'"),
            ("ção", "'ção'"),
            ("日本", "'日本'"),
        ];

        for (argument, expected) in cases {
            assert_eq!(
                quote_for_display(argument),
                expected,
                "rendering of {argument:?}"
            );
        }
    }

    /// The quoting is display-only. Nothing in the app splits the rendered
    /// string back apart or hands it to a shell, and this test states the
    /// direction of the guarantee: the argv is the truth, the string is a view.
    #[test]
    fn quoting_a_safe_argument_borrows_it() {
        assert!(matches!(quote_for_display("web-01"), Cow::Borrowed(_)));
        assert!(matches!(quote_for_display("my vm"), Cow::Owned(_)));
    }

    /// Splits a rendered command line back into its top-level fields, using
    /// the same rules `quote_for_display` writes with: a `'` toggles quoting,
    /// a backslash outside quotes escapes the next character (that is the
    /// `'\''` seam), and only an unquoted space separates fields.
    ///
    /// This exists to state the invariant, not to be used by the app. Nothing
    /// in nuvem parses a rendered command line — the argv is the truth.
    fn split_display_fields(rendered: &str) -> Vec<String> {
        let body = rendered
            .strip_prefix("gcloud ")
            .expect("every rendering is prefixed with the program name");

        let mut fields = Vec::new();
        let mut current = String::new();
        let mut in_quotes = false;
        let mut characters = body.chars();

        while let Some(character) = characters.next() {
            match character {
                '\'' => in_quotes = !in_quotes,
                '\\' if !in_quotes => {
                    if let Some(escaped) = characters.next() {
                        current.push(escaped);
                    }
                }
                ' ' if !in_quotes => {
                    fields.push(std::mem::take(&mut current));
                }
                other => current.push(other),
            }
        }
        fields.push(current);
        fields
    }

    proptest::proptest! {
        /// For any argv at all, the operator sees exactly as many arguments as
        /// will be passed. The join-on-spaces this replaced broke precisely
        /// this: an argument containing a space showed as two.
        #[test]
        fn a_rendering_shows_exactly_the_arguments_it_was_given(
            argv in proptest::collection::vec("(?s).*", 1..6usize)
        ) {
            let rendered = gcloud_command_line(&argv);

            proptest::prop_assert_eq!(
                split_display_fields(&rendered).len(),
                argv.len(),
                "rendered as {:?} from {:?}",
                rendered,
                argv
            );
        }

        /// A rendering is always one line, so the confirmation modal's
        /// `paragraph_fits` height count cannot disagree with what is drawn.
        #[test]
        fn a_rendering_is_always_a_single_line(
            argv in proptest::collection::vec("(?s).*", 1..6usize)
        ) {
            let rendered = gcloud_command_line(&argv);

            proptest::prop_assert!(
                !rendered.chars().any(|character| character.is_control()),
                "rendered a control character: {:?}",
                rendered
            );
        }

        /// No format character reaches a rendering either, so the line the
        /// operator reads cannot draw in a different order, or with different
        /// visible content, than the argv it was built from. The broad
        /// generator above almost never draws one, so this alphabet mixes
        /// plain letters with bidi overrides, isolates and zero-width
        /// characters.
        #[test]
        fn a_rendering_never_carries_an_invisible_character(
            argv in proptest::collection::vec(
                proptest::string::string_regex(
                    "[a-z\u{ad}\u{200b}\u{200e}\u{202a}-\u{202e}\u{2066}-\u{2069}\u{feff}]{0,6}",
                ).unwrap(),
                1..6usize,
            )
        ) {
            let rendered = gcloud_command_line(&argv);

            proptest::prop_assert!(
                !rendered.chars().any(|character| matches!(
                    get_general_category(character),
                    GeneralCategory::Control | GeneralCategory::Format
                )),
                "rendered an invisible character: {:?}",
                rendered
            );
        }

        /// The same field-count invariant as
        /// `a_rendering_shows_exactly_the_arguments_it_was_given`, over an
        /// alphabet built for it rather than a broad one.
        ///
        /// This exists because the broad `(?s).*` generator almost never
        /// samples an ASCII space: re-introducing the original defect (adding
        /// `b' '` to `is_display_safe`) leaves that property passing at the
        /// default case count, and it takes thousands of cases to fail. A space
        /// is the very character this rendering was changed for, so this
        /// property draws from a small alphabet dense in the characters
        /// quoting is about — space, quote, backslash, `$`, `;`, `=` — and
        /// catches the defect immediately. The broad property is kept for the
        /// opposite reason: it hunts unusual code points this one never emits.
        #[test]
        fn a_rendering_over_quoting_relevant_characters_shows_exactly_its_arguments(
            argv in proptest::collection::vec(
                proptest::string::string_regex("[a-z '\"\\\\\\n\\t $;=]{0,6}").unwrap(),
                1..6usize,
            )
        ) {
            let rendered = gcloud_command_line(&argv);

            proptest::prop_assert_eq!(
                split_display_fields(&rendered).len(),
                argv.len(),
                "rendered as {:?} from {:?}",
                rendered,
                argv
            );
        }
    }
}

#[cfg(test)]
mod plugin_confirmation_tests {
    use super::*;

    /// The confirmation carries neither a plugin identity nor an
    /// offer, so the fixture is the three things it is built from.
    fn confirmation(dangerous: bool) -> PluginConfirmation {
        PluginConfirmation::new(
            "Describe web-01".to_string(),
            vec!["--configuration".to_string(), "dev".to_string()],
            dangerous,
        )
    }

    /// The declaration's `dangerous` is the only thing that decides which
    /// acknowledgement the confirmation asks for, and it is the only branch in
    /// this type. Backwards, it would run a plugin the operator marked
    /// dangerous on a single keypress.
    ///
    /// The mutual exclusion these modals live under is pinned over every
    /// opener in `reducer_resources_test`, where the fixtures to open the other
    /// six exist.
    #[test]
    fn a_dangerous_plugin_asks_for_a_typed_acknowledgement_and_an_ordinary_one_does_not() {
        assert_eq!(
            confirmation(true).ack(),
            &BatchAck::CountRequired {
                typed: String::new()
            }
        );
        assert_eq!(confirmation(false).ack(), &BatchAck::Simple);
    }

    /// The count a plugin's acknowledgement is measured against is 1 — one
    /// command against one row — so `1` satisfies it and any other number does
    /// not. Nothing else in the app supplies that 1, so an off-by-one here
    /// would leave a dangerous plugin's confirm key permanently dead.
    #[test]
    fn a_dangerous_confirmation_is_satisfied_by_a_typed_one_and_by_nothing_else() {
        let mut dangerous = confirmation(true);
        assert!(!dangerous.is_ack_satisfied(), "nothing typed yet");

        dangerous.ack_append_digit('2');
        assert!(!dangerous.is_ack_satisfied(), "2 is not the count");

        dangerous.ack_backspace();
        dangerous.ack_append_digit('1');
        assert!(dangerous.is_ack_satisfied());

        assert!(
            confirmation(false).is_ack_satisfied(),
            "an ordinary plugin needs no typing"
        );
    }
}

#[cfg(test)]
mod plugin_prompt_tests {
    use super::*;
    use crate::config::plugins::{PluginInput, PluginInputKind};

    fn text_input(name: &str, default: Option<&str>) -> PluginInput {
        PluginInput {
            name: name.to_string(),
            kind: PluginInputKind::String,
            default: default.map(str::to_string),
        }
    }

    fn choice_input(name: &str, choices: &[&str], default: Option<&str>) -> PluginInput {
        PluginInput {
            name: name.to_string(),
            kind: PluginInputKind::Dropdown {
                choices: choices.iter().map(|c| c.to_string()).collect(),
            },
            default: default.map(str::to_string),
        }
    }

    fn prompt(inputs: Vec<PluginInput>) -> PluginPrompt {
        PluginPrompt::new(PluginId(0), "Resize".to_string(), false, inputs, Vec::new())
    }

    fn typed_input(name: &str, kind: PluginInputKind, default: Option<&str>) -> PluginInput {
        PluginInput {
            name: name.to_string(),
            kind,
            default: default.map(str::to_string),
        }
    }

    #[test]
    fn a_text_field_is_prefilled_from_its_default() {
        let prompt = prompt(vec![text_input("disk_size", Some("100"))]);
        assert_eq!(
            prompt.values(),
            vec![("disk_size".to_string(), "100".to_string())]
        );
        assert!(prompt.is_satisfied());
    }

    #[test]
    fn a_field_with_no_default_starts_empty_and_unsatisfied() {
        let mut prompt = prompt(vec![text_input("disk_size", None)]);
        assert!(!prompt.is_satisfied(), "an empty field must not submit");
        prompt.append('5');
        assert!(prompt.is_satisfied());
        prompt.backspace();
        assert!(!prompt.is_satisfied());
    }

    #[test]
    fn a_dropdown_selects_its_declared_default() {
        let prompt = prompt(vec![choice_input(
            "tier",
            &["db-f1-micro", "db-g1-small"],
            Some("db-g1-small"),
        )]);
        assert_eq!(
            prompt.values(),
            vec![("tier".to_string(), "db-g1-small".to_string())]
        );
    }

    #[test]
    fn a_dropdown_with_no_default_selects_the_first_choice() {
        let prompt = prompt(vec![choice_input(
            "tier",
            &["db-f1-micro", "db-g1-small"],
            None,
        )]);
        assert_eq!(
            prompt.values(),
            vec![("tier".to_string(), "db-f1-micro".to_string())],
            "a dropdown is never empty, so it is always satisfied"
        );
        assert!(prompt.is_satisfied());
    }

    #[test]
    fn cycling_a_dropdown_wraps_in_both_directions() {
        let mut prompt = prompt(vec![choice_input("tier", &["a", "b"], None)]);
        prompt.cycle_choice(true);
        assert_eq!(prompt.values()[0].1, "b");
        prompt.cycle_choice(true);
        assert_eq!(prompt.values()[0].1, "a", "forward wraps");
        prompt.cycle_choice(false);
        assert_eq!(prompt.values()[0].1, "b", "backward wraps");
    }

    #[test]
    fn typing_reaches_only_the_focused_field() {
        let mut prompt = prompt(vec![text_input("a", None), text_input("b", None)]);
        prompt.append('1');
        prompt.focus_next();
        prompt.append('2');
        assert_eq!(
            prompt.values(),
            vec![
                ("a".to_string(), "1".to_string()),
                ("b".to_string(), "2".to_string())
            ]
        );
    }

    #[test]
    fn focus_wraps_in_both_directions() {
        let mut prompt = prompt(vec![text_input("a", None), text_input("b", None)]);
        assert_eq!(prompt.focus(), 0);
        prompt.focus_next();
        prompt.focus_next();
        assert_eq!(prompt.focus(), 0, "next wraps past the last field");
        prompt.focus_previous();
        assert_eq!(prompt.focus(), 1, "previous wraps past the first field");
    }

    #[test]
    fn typing_into_a_dropdown_does_nothing() {
        let mut prompt = prompt(vec![choice_input("tier", &["a", "b"], None)]);
        prompt.append('z');
        prompt.backspace();
        assert_eq!(
            prompt.values()[0].1,
            "a",
            "a dropdown holds an index, so an unlisted choice is unrepresentable"
        );
    }

    /// The prompt's numeric gate is the loader's, one step later. `default_for`
    /// refuses a non-finite `number` because the text reaches gcloud verbatim
    /// and `--share inf` is not a number; `"inf".parse::<f64>()` succeeds, so a
    /// bare parse here would accept at the prompt exactly what a declaration of
    /// the same value is refused for.
    #[test]
    fn a_number_field_refuses_the_non_finite_values_its_declared_default_is_refused_for() {
        for hostile in ["inf", "-inf", "NaN", "infinity"] {
            let mut prompt = prompt(vec![typed_input("share", PluginInputKind::Number, None)]);
            for value in hostile.chars() {
                prompt.append(value);
            }
            assert!(
                !prompt.is_satisfied(),
                "{hostile:?} parses as an f64 but is not a number gcloud can take"
            );
        }
    }

    #[test]
    fn a_number_field_takes_a_finite_decimal() {
        let mut prompt = prompt(vec![typed_input("share", PluginInputKind::Number, None)]);
        for value in "0.5".chars() {
            prompt.append(value);
        }
        assert!(prompt.is_satisfied());
        assert_eq!(
            prompt.values(),
            vec![("share".to_string(), "0.5".to_string())]
        );
    }

    /// The counterpart of the loader's "default 1.5 must be a whole number": an
    /// `integer` reaches an argv verbatim, so the prompt must not send what the
    /// declaration could not.
    #[test]
    fn an_integer_field_refuses_a_float_the_way_its_declared_default_is_refused() {
        for hostile in ["1.5", "1.0", "1e3", "0x10", " 1", "1 "] {
            let mut prompt = prompt(vec![typed_input("size", PluginInputKind::Integer, None)]);
            for value in hostile.chars() {
                prompt.append(value);
            }
            assert!(!prompt.is_satisfied(), "{hostile:?} is not a whole number");
        }
    }

    #[test]
    fn an_integer_field_takes_a_whole_number_including_a_negative_one() {
        for accepted in ["200", "-1", "0"] {
            let mut prompt = prompt(vec![typed_input("size", PluginInputKind::Integer, None)]);
            for value in accepted.chars() {
                prompt.append(value);
            }
            assert!(prompt.is_satisfied(), "{accepted:?} is a whole number");
            assert_eq!(prompt.values()[0].1, accepted);
        }
    }

    /// A typed value reaches the argv as typed. A declared default is stored
    /// canonically (`0.50` becomes `0.5`), but re-rendering what the operator
    /// entered would put characters on the confirmation they did not type — and
    /// that screen is the one thing the prompt was built in front of.
    #[test]
    fn a_typed_numeric_value_is_carried_verbatim() {
        let mut prompt = prompt(vec![typed_input("share", PluginInputKind::Number, None)]);
        for value in "0.50".chars() {
            prompt.append(value);
        }
        assert_eq!(
            prompt.values()[0].1,
            "0.50",
            "the confirmation shows the characters that will run"
        );
    }

    #[test]
    fn a_numeric_field_is_prefilled_from_its_declared_default() {
        let prompt = prompt(vec![typed_input(
            "size",
            PluginInputKind::Integer,
            Some("200"),
        )]);
        assert_eq!(prompt.values()[0].1, "200");
        assert!(prompt.is_satisfied());
    }

    /// The refusal is a gate, not a discard: `SubmitPluginPrompt` takes the
    /// prompt only once nothing can fail, so an operator who mistypes one field
    /// corrects it with the rest of their typing intact.
    #[test]
    fn correcting_an_unparseable_field_leaves_the_other_fields_untouched() {
        let mut prompt = prompt(vec![
            text_input("reason", Some("capacity review")),
            typed_input("size", PluginInputKind::Integer, None),
        ]);
        prompt.focus_next();
        for value in "1.5".chars() {
            prompt.append(value);
        }
        assert!(!prompt.is_satisfied());

        prompt.backspace();
        prompt.backspace();
        assert!(prompt.is_satisfied());
        assert_eq!(
            prompt.values(),
            vec![
                ("reason".to_string(), "capacity review".to_string()),
                ("size".to_string(), "1".to_string())
            ]
        );
    }

    /// An unparseable field and an empty one are both unsatisfied, and a string
    /// field is unaffected by either rule: it takes any non-empty text.
    #[test]
    fn a_string_field_takes_text_no_numeric_field_would() {
        let mut prompt = prompt(vec![text_input("reason", None)]);
        for value in "1.5 GB".chars() {
            prompt.append(value);
        }
        assert!(prompt.is_satisfied());
    }

    #[test]
    fn every_field_must_be_filled_before_the_prompt_is_satisfied() {
        let mut prompt = prompt(vec![text_input("a", Some("x")), text_input("b", None)]);
        assert!(!prompt.is_satisfied());
        prompt.focus_next();
        prompt.append('y');
        assert!(prompt.is_satisfied());
    }
}
