use std::borrow::Cow;

use super::plugins::{Offer, PluginId};
use super::resources::{CloudSqlInstanceAction, ComputeInstanceAction, ComputeInstanceField};
use super::root::{RootState, Workspace};
use crate::app::service_registry::{self, descriptor, service_id_for_resource};
use nuvem_domain::gcp::resource::ResourceServiceKind;

/// Declares `CommandDescriptor` and, from the same list, the enumeration of
/// its unit variants.
///
/// The list is written once. A test that has to reason about "every
/// descriptor" — the `is_mutating` partition below is the one that matters —
/// cannot then be given a total that drifts from the enum: the enum and
/// [`CommandDescriptor::UNIT_ALL`] are the same list. A hand-maintained
/// constant would reintroduce exactly the gap it is there to close, because a
/// new variant omitted from it changes no count anything checks.
macro_rules! command_descriptors {
    ($($variant:ident),+ $(,)?) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub enum CommandDescriptor {
            $($variant,)+
            /// Payload-carrying variant: one descriptor per service kind
            /// rather than one descriptor, kept outside the generated list
            /// because [`CommandDescriptor::all`] expands it over
            /// `ResourceServiceKind::ALL`.
            GoToService(ResourceServiceKind),
            /// The other payload-carrying variant, expanded the same way.
            ///
            /// These were six hand-written unit variants once. That
            /// shape enumerated "one entry per service" without the compiler
            /// knowing that is what it was doing: adding a service forces 71
            /// match sites across 13 files, and this list was in none of them,
            /// so Secret Manager shipped with a working `L` keybinding and no
            /// palette entry. Parameterising it moves the list into the half
            /// `all` derives, where an omission stops being expressible.
            ///
            /// `OpenLogsForSelectedCloudRunRevision` stays a unit variant: a
            /// revision is a sub-view of a service, not a
            /// `ResourceServiceKind`, so it has no member of `ALL` to come
            /// from.
            OpenLogsForSelected(ResourceServiceKind),
            /// A user-declared command, identified by its index in
            /// [`RootState::plugins`].
            ///
            /// Deliberately outside `UNIT_ALL` **and** outside
            /// [`CommandDescriptor::all`]: plugins are runtime data read from a
            /// file, so there is no compile-time set to expand over the way
            /// `ResourceServiceKind::ALL` is expanded. That is why
            /// `is_mutating` gets an explicit test of its own — the partition
            /// test walks `all()` and cannot see this variant.
            Plugin(PluginId),
        }

        impl CommandDescriptor {
            /// Every unit variant, in declaration order.
            pub const UNIT_ALL: [Self; [$(stringify!($variant)),+].len()] =
                [$(Self::$variant),+];
        }
    };
}

command_descriptors! {
    ActivateSelectedConfiguration,
    StartSelectedComputeInstance,
    StopSelectedComputeInstance,
    ResetSelectedComputeInstance,
    SuspendSelectedComputeInstance,
    ResumeSelectedComputeInstance,
    ShowSelectedComputeInstanceSsh,
    SshSelectedComputeInstance,
    StartTunnelForSelectedComputeInstance,
    CopySelectedInstanceName,
    CopySelectedInstanceZone,
    CopySelectedInstanceInternalIp,
    CopySelectedInstanceExternalIp,
    CopySelectedInstanceSelfLink,
    StartSelectedCloudSqlInstance,
    StopSelectedCloudSqlInstance,
    RestartSelectedCloudSqlInstance,
    FailoverSelectedCloudSqlInstance,
    CopySelectedCloudSqlConnectionName,
    StartTunnelForSelectedCloudSqlInstance,
    OpenProjectLogs,
    OpenLogsForSelectedCloudRunRevision,
    GoHome,
    GoToConfigurations,
    GoToTunnels,
    GoToFinOps,
    OpenSelectedGkeClusterInK9s,
    RefreshCurrentView,
    ReloadConfig,
    ShowPluginOutput,
    ShowHelp,
    SwitchProjectContext,
}

impl CommandDescriptor {
    /// Every constructible descriptor: the unit variants the enum declaration
    /// generates, plus one `GoToService` per service kind.
    ///
    /// Derived from the enum on both halves, so it cannot fall behind it —
    /// which is what lets a test say "every descriptor" and mean it.
    pub fn all() -> Vec<Self> {
        let mut all = Self::UNIT_ALL.to_vec();
        all.extend(ResourceServiceKind::ALL.map(Self::GoToService));
        all.extend(ResourceServiceKind::ALL.map(Self::OpenLogsForSelected));
        all
    }

    /// The palette's display string.
    ///
    /// `Cow` rather than `String`: `match_rank` calls this for every
    /// descriptor on every keystroke, so making ~30 static labels allocate to
    /// serve the two derived ones would be a real cost for no gain.
    pub fn label(self) -> Cow<'static, str> {
        match self {
            Self::ActivateSelectedConfiguration => {
                Cow::Borrowed("Activate selected gcloud configuration")
            }
            Self::StartSelectedComputeInstance => Cow::Borrowed("Start selected GCE instance"),
            Self::StopSelectedComputeInstance => Cow::Borrowed("Stop selected GCE instance"),
            Self::ResetSelectedComputeInstance => Cow::Borrowed("Reset selected GCE instance"),
            Self::SuspendSelectedComputeInstance => Cow::Borrowed("Suspend selected GCE instance"),
            Self::ResumeSelectedComputeInstance => Cow::Borrowed("Resume selected GCE instance"),
            Self::ShowSelectedComputeInstanceSsh => {
                Cow::Borrowed("Show SSH command for selected GCE instance")
            }
            Self::SshSelectedComputeInstance => Cow::Borrowed("SSH to selected GCE instance"),
            Self::StartTunnelForSelectedComputeInstance => {
                Cow::Borrowed("Start tunnel for selected GCE instance")
            }
            Self::CopySelectedInstanceName => Cow::Borrowed(ComputeInstanceField::Name.label()),
            Self::CopySelectedInstanceZone => Cow::Borrowed(ComputeInstanceField::Zone.label()),
            Self::CopySelectedInstanceInternalIp => {
                Cow::Borrowed(ComputeInstanceField::InternalIp.label())
            }
            Self::CopySelectedInstanceExternalIp => {
                Cow::Borrowed(ComputeInstanceField::ExternalIp.label())
            }
            Self::CopySelectedInstanceSelfLink => {
                Cow::Borrowed(ComputeInstanceField::SelfLink.label())
            }
            Self::StartSelectedCloudSqlInstance => {
                Cow::Borrowed("Start selected Cloud SQL instance")
            }
            Self::StopSelectedCloudSqlInstance => Cow::Borrowed("Stop selected Cloud SQL instance"),
            Self::RestartSelectedCloudSqlInstance => {
                Cow::Borrowed("Restart selected Cloud SQL instance")
            }
            Self::FailoverSelectedCloudSqlInstance => {
                Cow::Borrowed("Fail over selected Cloud SQL instance")
            }
            Self::CopySelectedCloudSqlConnectionName => {
                Cow::Borrowed("Copy selected Cloud SQL connection name")
            }
            Self::StartTunnelForSelectedCloudSqlInstance => {
                Cow::Borrowed("Start tunnel for selected Cloud SQL instance")
            }
            Self::OpenProjectLogs => Cow::Borrowed("Open project logs"),
            Self::OpenLogsForSelectedCloudRunRevision => {
                Cow::Borrowed("Open logs for selected Cloud Run revision")
            }
            Self::GoHome => Cow::Borrowed("Home"),
            Self::GoToConfigurations => Cow::Borrowed("Configurations"),
            Self::GoToTunnels => Cow::Borrowed("Tunnels"),
            Self::GoToFinOps => Cow::Borrowed("FinOps"),
            Self::GoToService(kind) => Cow::Borrowed(kind.label()),
            // Derived from the same noun the rest of the app uses for a
            // selection, so these six strings have one source rather than a
            // copy the palette maintains.
            Self::OpenLogsForSelected(kind) => {
                Cow::Owned(format!("Open logs for selected {}", kind.selection_noun()))
            }
            Self::OpenSelectedGkeClusterInK9s => Cow::Borrowed("Open selected GKE cluster in k9s"),
            Self::RefreshCurrentView => Cow::Borrowed("Refresh"),
            Self::ReloadConfig => Cow::Borrowed("Reload configuration"),
            // Offered only when some run has output to show (see
            // `available_commands`), so choosing it never opens an empty
            // overlay.
            Self::ShowPluginOutput => Cow::Borrowed("Plugin output"),
            Self::ShowHelp => Cow::Borrowed("Help"),
            Self::SwitchProjectContext => Cow::Borrowed("Switch project / context"),
            // Never displayed: `palette_display_label` and `match_rank` both
            // resolve a plugin's label from `RootState`, because the text
            // lives in a file rather than in this enum. Pinned by
            // `no_palette_row_shows_the_plugin_placeholder`.
            Self::Plugin(_) => Cow::Borrowed("Plugin command"),
        }
    }

    /// Answers one question about a command, and it is the question — not the
    /// list below — that decides a new descriptor's classification:
    ///
    /// > Does invoking this change cloud state, or hand the operator an
    /// > interactive session that can?
    ///
    /// Both halves are load-bearing. The first covers the direct case: an API
    /// call that alters a resource, such as an instance's lifecycle state. The
    /// second covers the delegated case, where this app performs no mutation
    /// itself but drops the operator into something that will — an SSH shell
    /// on an instance, or a k9s session against a live cluster. A command that
    /// only *reads* cloud state, or that only changes something local (a
    /// clipboard write, a gcloud configuration, a tunnel process on this
    /// machine), answers no.
    ///
    /// Classifying by enumeration is how the k9s launcher spent a release on
    /// the wrong side of this predicate: it was not one of the lifecycle
    /// actions anyone had in mind, so it was never matched against them.
    /// Classify against the question.
    ///
    /// This is a DIFFERENT axis from [`super::resources::BatchAction::is_destructive`]
    /// (and the per-action `is_destructive` it delegates to). `is_destructive`
    /// asks "how loudly should we ask for confirmation", and on that axis
    /// `Start`/`Resume` are excluded: nothing is torn down by booting an
    /// instance. `is_mutating` asks a strictly different question — "does
    /// this change cloud state" — and `Start` answers yes:
    /// `ComputeInstanceAction::Start::impact_warning()` says "Boots the
    /// instance; billing resumes." A read-only gate built on
    /// `is_destructive` alone would let `Start` straight through.
    ///
    /// This match is deliberately exhaustive with no wildcard arm: adding a
    /// new `CommandDescriptor` variant is a compile error until its mutation
    /// status is declared here, which is what makes `is_mutating` a
    /// guarantee rather than a best effort.
    pub fn is_mutating(self) -> bool {
        match self {
            Self::StartSelectedComputeInstance
            | Self::StopSelectedComputeInstance
            | Self::ResetSelectedComputeInstance
            | Self::SuspendSelectedComputeInstance
            | Self::ResumeSelectedComputeInstance
            | Self::StartSelectedCloudSqlInstance
            | Self::StopSelectedCloudSqlInstance
            | Self::RestartSelectedCloudSqlInstance
            | Self::FailoverSelectedCloudSqlInstance
            // Interactive sessions: this app issues no mutation itself, but
            // both drop the operator somewhere that can. `PrepareK9sLaunch`
            // additionally runs `get-credentials` when no kube context is
            // present, and k9s is a full read-write Kubernetes UI.
            | Self::SshSelectedComputeInstance
            | Self::OpenSelectedGkeClusterInK9s => true,
            // A plugin's argv is runtime data, so this compile-time question
            // about the descriptor enum cannot be answered from the variant:
            // `true` is the safe direction to be wrong in, and it is what a
            // caller that has no better source must act on.
            //
            // `--read-only` no longer is such a caller. It asks
            // `plugins::plugin_blocked_by_read_only`, which reads the plugin's
            // own declaration, so a plugin nuvem can prove only reads
            // runs under the flag while this still answers `true`. The two do
            // not compete: this classifies variants, that classifies config.
            Self::Plugin(_) => true,
            Self::ActivateSelectedConfiguration
            | Self::ShowSelectedComputeInstanceSsh
            | Self::StartTunnelForSelectedComputeInstance
            | Self::CopySelectedInstanceName
            | Self::CopySelectedInstanceZone
            | Self::CopySelectedInstanceInternalIp
            | Self::CopySelectedInstanceExternalIp
            | Self::CopySelectedInstanceSelfLink
            | Self::CopySelectedCloudSqlConnectionName
            | Self::StartTunnelForSelectedCloudSqlInstance
            | Self::OpenProjectLogs
            | Self::OpenLogsForSelected(_)
            | Self::OpenLogsForSelectedCloudRunRevision
            | Self::GoHome
            | Self::GoToConfigurations
            | Self::GoToTunnels
            | Self::GoToFinOps
            | Self::GoToService(_)
            | Self::RefreshCurrentView
            | Self::ReloadConfig
            | Self::ShowPluginOutput
            | Self::ShowHelp
            | Self::SwitchProjectContext => false,
        }
    }

    pub fn aliases(self) -> &'static [&'static str] {
        match self {
            Self::OpenProjectLogs => &["logs", "logging"],
            // Cloud Run is the one service whose logs are also findable under
            // "logging"; every other service's log command answers to "logs"
            // alone, and a service added later gets that default.
            Self::OpenLogsForSelected(ResourceServiceKind::CloudRunServices) => {
                &["logs", "logging"]
            }
            Self::OpenLogsForSelected(_) | Self::OpenLogsForSelectedCloudRunRevision => &["logs"],
            Self::GoHome => &["home"],
            Self::GoToConfigurations => &["configurations", "configs", "config"],
            Self::GoToTunnels => &["tunnels", "pf", "forward", "port forward"],
            // "budget" and "spend" reach it too: the workspace answers a
            // money question, and the operator is unlikely to arrive at it
            // by typing the word "recommender".
            Self::GoToFinOps => &["finops", "cost", "costs", "savings", "budget", "spend"],
            Self::GoToService(ResourceServiceKind::ComputeInstances) => {
                &["gce", "compute", "instances"]
            }
            Self::GoToService(ResourceServiceKind::GkeClusters) => &["gke", "clusters"],
            Self::GoToService(ResourceServiceKind::CloudSql) => {
                &["cloud sql", "sql", "database", "postgres", "mysql"]
            }
            Self::GoToService(ResourceServiceKind::CloudRunServices) => {
                &["run", "serverless", "cloud run"]
            }
            Self::GoToService(ResourceServiceKind::CloudStorage) => {
                &["gcs", "storage", "bucket", "buckets"]
            }
            Self::RefreshCurrentView => &["refresh", "reload"],
            Self::ReloadConfig => &["reload config", "config reload"],
            Self::ShowPluginOutput => &["plugin output", "output"],
            Self::ShowHelp => &["help"],
            Self::SwitchProjectContext => &["project", "context", "switch"],
            Self::SshSelectedComputeInstance => &["ssh"],
            // Shared with `GoToTunnels`: `pf`/`forward` should find both "take
            // me to the tunnel list" and "start one from this row", and the
            // palette ranks by label match, so neither hides the other.
            Self::StartTunnelForSelectedComputeInstance
            | Self::StartTunnelForSelectedCloudSqlInstance => &["tunnel", "pf", "forward"],
            Self::OpenSelectedGkeClusterInK9s => &["k9s", "open"],
            _ => &[],
        }
    }
}

/// Whether `--read-only` refuses this command.
///
/// The whole condition, not half of it. `is_mutating` answers the compile-time
/// question about the descriptor enum and keeps answering `true` for a plugin,
/// whose argv is runtime data; a plugin's own answer comes from its declaration
/// ([`super::plugins::plugin_blocked_by_read_only`]). The two do not compete —
/// one classifies variants, the other classifies config — but a caller that
/// wires them together itself is a caller that can wire them differently, and
/// the palette dimming a row the reducer runs is exactly the drift the single
/// plugin predicate exists to close. Both arms live here so there is one
/// answer for the palette's `Submit`, the palette's dimming, and any door
/// added after them.
pub fn command_blocked_by_read_only(command: CommandDescriptor, root: &RootState) -> bool {
    match command {
        CommandDescriptor::Plugin(id) => super::plugins::plugin_blocked_by_read_only(root, id),
        other => root.read_only && other.is_mutating(),
    }
}

/// The label shown for a command in the palette list.
///
/// When resources are marked, the batch-capable mutation commands operate on
/// the whole marked set (k9s-style retargeting), so their displayed label
/// reflects the marked count instead of the static "selected" wording. Every
/// command that the submit path can turn into a batch is covered — the five
/// compute mutations and the four Cloud SQL ones — because a command that runs
/// against N marked rows while reading "selected instance" misstates its blast
/// radius before the confirmation modal is ever seen.
///
/// This is a display-only transformation: `CommandDescriptor::label` (used for
/// palette fuzzy-matching) is unchanged.
pub fn palette_display_label(command: CommandDescriptor, root: &RootState) -> String {
    // Before the marked-count transformation below: a plugin is never a batch
    // command, and reading the marked count first would answer with wording
    // about marked instances for a command that acts on the selected row.
    if let CommandDescriptor::Plugin(id) = command
        && let Some(offer) = plugin_offer(root, id)
    {
        return offer.label;
    }
    let marked = root.resources.marked_count();
    if marked == 0 {
        return command.label().into_owned();
    }
    let (verb, resource_kind) = match command {
        CommandDescriptor::StartSelectedComputeInstance => ("Start", "GCE"),
        CommandDescriptor::StopSelectedComputeInstance => ("Stop", "GCE"),
        CommandDescriptor::ResetSelectedComputeInstance => ("Reset", "GCE"),
        CommandDescriptor::SuspendSelectedComputeInstance => ("Suspend", "GCE"),
        CommandDescriptor::ResumeSelectedComputeInstance => ("Resume", "GCE"),
        CommandDescriptor::StartSelectedCloudSqlInstance => ("Start", "Cloud SQL"),
        CommandDescriptor::StopSelectedCloudSqlInstance => ("Stop", "Cloud SQL"),
        CommandDescriptor::RestartSelectedCloudSqlInstance => ("Restart", "Cloud SQL"),
        CommandDescriptor::FailoverSelectedCloudSqlInstance => ("Fail over", "Cloud SQL"),
        _ => return command.label().into_owned(),
    };
    let noun = if marked == 1 { "instance" } else { "instances" };
    format!("{verb} {marked} marked {resource_kind} {noun}")
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CommandPaletteState {
    is_open: bool,
    filter: String,
    selected_visible: usize,
}

impl CommandPaletteState {
    pub fn is_open(&self) -> bool {
        self.is_open
    }

    pub fn filter(&self) -> &str {
        &self.filter
    }

    pub fn open(&mut self) {
        self.is_open = true;
        self.filter.clear();
        self.selected_visible = 0;
    }

    pub fn close(&mut self) {
        self.is_open = false;
        self.filter.clear();
        self.selected_visible = 0;
    }

    pub fn append_char(&mut self, value: char) {
        if !value.is_control() {
            self.filter.push(value);
            self.selected_visible = 0;
        }
    }

    pub fn backspace(&mut self) {
        self.filter.pop();
        self.selected_visible = 0;
    }

    pub fn select_next(&mut self, root: &RootState) {
        let len = self.visible_commands(root).len();
        if len > 0 {
            self.selected_visible = (self.selected_visible + 1).min(len - 1);
        }
    }

    pub fn select_previous(&mut self) {
        self.selected_visible = self.selected_visible.saturating_sub(1);
    }

    pub fn visible_commands(&self, root: &RootState) -> Vec<CommandDescriptor> {
        let commands = available_commands(root);
        let filter = self.filter.trim().to_lowercase();
        if filter.is_empty() {
            return commands;
        }
        let mut ranked: Vec<(u8, usize, CommandDescriptor)> = commands
            .into_iter()
            .enumerate()
            .filter_map(|(index, command)| {
                match_rank(command, &filter, root).map(|rank| (rank, index, command))
            })
            .collect();
        ranked.sort_by_key(|(rank, index, _)| (*rank, *index));
        ranked.into_iter().map(|(_, _, command)| command).collect()
    }

    pub fn selected_visible_command(&self, root: &RootState) -> Option<CommandDescriptor> {
        self.visible_commands(root)
            .get(self.selected_visible)
            .copied()
    }
}

fn available_commands(root: &RootState) -> Vec<CommandDescriptor> {
    let mut commands = context_commands(root);
    commands.extend(navigation_commands(root));
    commands
}

/// The rank a command matches `query` at, or `None` when it does not match.
///
/// Takes the whole [`RootState`] rather than just its alias table because a
/// plugin's search text is its rendered label, which lives in the plugin file:
/// matching a plugin against `CommandDescriptor::label` would search the
/// placeholder, so a plugin the operator labelled "Describe web-01" would be
/// unreachable by typing `describe` and reachable by typing `plugin`.
fn match_rank(command: CommandDescriptor, query: &str, root: &RootState) -> Option<u8> {
    let mut best: Option<u8> = None;
    let mut consider = |text: &str| {
        let text = text.to_lowercase();
        let rank = if text == query {
            Some(0)
        } else if text.starts_with(query) {
            Some(1)
        } else if text.contains(query) {
            Some(2)
        } else {
            None
        };
        if let Some(rank) = rank {
            best = Some(best.map_or(rank, |current| current.min(rank)));
        }
    };
    match command {
        CommandDescriptor::Plugin(_) => consider(&palette_display_label(command, root)),
        _ => consider(command.label().as_ref()),
    }
    for alias in command.aliases() {
        consider(alias);
    }
    if let CommandDescriptor::GoToService(kind) = command
        && let Some(service_id) = service_registry::service_id_for_resource(kind)
    {
        for (alias, id) in root.aliases.iter() {
            if id == service_id {
                consider(alias);
            }
        }
    }
    best
}

fn context_commands(root: &RootState) -> Vec<CommandDescriptor> {
    let mut commands = builtin_context_commands(root);
    // Appended after the built-in context entries, which is not the end of the
    // palette: `available_commands` puts the navigation entries after these,
    // so six plugins push Home / Go to… / Refresh / Help down six rows in an
    // unfiltered palette. That is intended — a plugin belongs with the entries
    // for the row in view — and it is written down because the ordering reads
    // like an accident from here.
    //
    // Appended *here* rather than inside a service's arm below: those arms
    // return early per service, and several services have no arm at all, so a
    // plugin scoped to one of them would never be offered — which is the whole
    // failure this feature exists to prevent.
    commands.extend(plugin_commands(root));
    commands
}

/// The entries the app itself offers for the current selection, unchanged
/// since before plugins existed. Split out from [`context_commands`] so the
/// per-service arms keep returning early without that early return deciding
/// whether plugins are offered.
fn builtin_context_commands(root: &RootState) -> Vec<CommandDescriptor> {
    match root.workspace {
        Workspace::Configurations => root
            .configurations
            .selected()
            .filter(|configuration| !configuration.is_active)
            .map(|_| vec![CommandDescriptor::ActivateSelectedConfiguration])
            .unwrap_or_default(),
        Workspace::Resources
            if root.resources.active_service() == ResourceServiceKind::ComputeInstances
                && !root.resources.is_showing_detail()
                && root.current_context().is_some()
                && root.resources.selected_visible_instance().is_some() =>
        {
            let context = root.current_context().expect("guard checks context");
            let instance = root
                .resources
                .selected_visible_instance()
                .expect("guard checks selected instance");
            let marked = root.resources.marked_instances();
            let mut commands = Vec::new();

            for (action, descriptor) in [
                (
                    ComputeInstanceAction::Start,
                    CommandDescriptor::StartSelectedComputeInstance,
                ),
                (
                    ComputeInstanceAction::Stop,
                    CommandDescriptor::StopSelectedComputeInstance,
                ),
                (
                    ComputeInstanceAction::Reset,
                    CommandDescriptor::ResetSelectedComputeInstance,
                ),
                (
                    ComputeInstanceAction::Suspend,
                    CommandDescriptor::SuspendSelectedComputeInstance,
                ),
                (
                    ComputeInstanceAction::Resume,
                    CommandDescriptor::ResumeSelectedComputeInstance,
                ),
            ] {
                let is_eligible = if marked.is_empty() {
                    action.is_eligible(&instance.status)
                } else {
                    marked
                        .iter()
                        .any(|marked| action.is_eligible(&marked.status))
                };
                if is_eligible {
                    commands.push(descriptor);
                }
            }

            commands.push(CommandDescriptor::ShowSelectedComputeInstanceSsh);
            commands.push(CommandDescriptor::SshSelectedComputeInstance);
            commands.push(CommandDescriptor::StartTunnelForSelectedComputeInstance);

            for (field, descriptor) in [
                (
                    ComputeInstanceField::Name,
                    CommandDescriptor::CopySelectedInstanceName,
                ),
                (
                    ComputeInstanceField::Zone,
                    CommandDescriptor::CopySelectedInstanceZone,
                ),
                (
                    ComputeInstanceField::InternalIp,
                    CommandDescriptor::CopySelectedInstanceInternalIp,
                ),
                (
                    ComputeInstanceField::ExternalIp,
                    CommandDescriptor::CopySelectedInstanceExternalIp,
                ),
                (
                    ComputeInstanceField::SelfLink,
                    CommandDescriptor::CopySelectedInstanceSelfLink,
                ),
            ] {
                if field.value(&context, instance).is_some() {
                    commands.push(descriptor);
                }
            }

            commands.push(CommandDescriptor::OpenLogsForSelected(
                ResourceServiceKind::ComputeInstances,
            ));

            commands
        }
        Workspace::Resources
            if root.resources.active_service() == ResourceServiceKind::GkeClusters
                && root.current_context().is_some()
                && root.resources.selected_gke_cluster().is_some() =>
        {
            let context = root.current_context().expect("guard checks context");
            let cluster = root
                .resources
                .selected_gke_cluster()
                .expect("guard checks selected cluster");
            let mut commands = Vec::new();
            if !root.resources.is_showing_detail() {
                commands.push(CommandDescriptor::OpenLogsForSelected(
                    ResourceServiceKind::GkeClusters,
                ));
            }
            if nuvem_domain::gcp::k9s::resolve_launch_target(&context, cluster).is_some() {
                commands.push(CommandDescriptor::OpenSelectedGkeClusterInK9s);
            }
            commands
        }
        Workspace::Resources
            if root.resources.active_service() == ResourceServiceKind::CloudSql
                && !root.resources.is_showing_detail()
                && root.current_context().is_some()
                && root.resources.selected_cloud_sql_instance().is_some() =>
        {
            let context = root.current_context().expect("guard checks context");
            let instance = root
                .resources
                .selected_cloud_sql_instance()
                .expect("guard checks selected instance");
            let marked = root.resources.marked_cloud_sql_instances();
            let mut commands = Vec::new();

            for (action, descriptor) in [
                (
                    CloudSqlInstanceAction::Start,
                    CommandDescriptor::StartSelectedCloudSqlInstance,
                ),
                (
                    CloudSqlInstanceAction::Stop,
                    CommandDescriptor::StopSelectedCloudSqlInstance,
                ),
                (
                    CloudSqlInstanceAction::Restart,
                    CommandDescriptor::RestartSelectedCloudSqlInstance,
                ),
                (
                    CloudSqlInstanceAction::Failover,
                    CommandDescriptor::FailoverSelectedCloudSqlInstance,
                ),
            ] {
                let is_eligible = if marked.is_empty() {
                    action.is_eligible(&instance.status, instance.availability)
                } else {
                    marked
                        .iter()
                        .any(|marked| action.is_eligible(&marked.status, marked.availability))
                };
                if is_eligible {
                    commands.push(descriptor);
                }
            }

            if cloud_sql_connection_name(&context, instance).is_some() {
                commands.push(CommandDescriptor::CopySelectedCloudSqlConnectionName);
                // The tunnel needs the same `project:region:instance` triple,
                // so it is offered under exactly the same condition.
                commands.push(CommandDescriptor::StartTunnelForSelectedCloudSqlInstance);
                // Cloud SQL log entries are keyed by a composite `database_id`
                // of the form `<project>:<instance>`, so the jump needs the
                // very same project the connection name does — without one the
                // reducer refuses, and an unconditional entry here would offer
                // a command that can only fail.
                commands.push(CommandDescriptor::OpenLogsForSelected(
                    ResourceServiceKind::CloudSql,
                ));
            }

            commands
        }
        // Ordered before the Cloud Run service arm below: the nested revision
        // view owns the pane, so its logs command is the one that applies.
        Workspace::Resources
            if root.resources.active_service() == ResourceServiceKind::CloudRunServices
                && root.resources.is_showing_cloud_run_revision()
                && root.current_context().is_some()
                && root.resources.cloud_run_revision_target().is_some() =>
        {
            vec![CommandDescriptor::OpenLogsForSelectedCloudRunRevision]
        }
        Workspace::Resources
            if root.resources.active_service() == ResourceServiceKind::CloudRunServices
                && !root.resources.is_showing_detail()
                && root.current_context().is_some()
                && root.resources.selected_cloud_run_service().is_some() =>
        {
            vec![CommandDescriptor::OpenLogsForSelected(
                ResourceServiceKind::CloudRunServices,
            )]
        }
        // Cloud Storage offers the bucket-scoped log jump from every view that
        // has a bucket on screen — the bucket list, a metadata pane, and the
        // object browser (where the browsed bucket is the target).
        Workspace::Resources
            if root.resources.active_service() == ResourceServiceKind::CloudStorage
                && root.current_context().is_some()
                && root.resources.storage().view_bucket().is_some() =>
        {
            vec![CommandDescriptor::OpenLogsForSelected(
                ResourceServiceKind::CloudStorage,
            )]
        }
        // Secret Manager offers the secret-scoped jump from the list and the
        // detail pane alike, matching the resolver, which accepts either. The
        // target is the secret's audit trail — who read or changed it — which
        // is metadata, never material.
        Workspace::Resources
            if root.resources.active_service() == ResourceServiceKind::SecretManagerSecrets
                && root.current_context().is_some()
                && (root.resources.secret_detail_target().is_some()
                    || root.resources.selected_secret().is_some()) =>
        {
            vec![CommandDescriptor::OpenLogsForSelected(
                ResourceServiceKind::SecretManagerSecrets,
            )]
        }
        // Cloud Functions offers the function-scoped jump from the list, which
        // is the only view it has: no detail pane exists for it yet, so the
        // list cursor is the whole selection rule the resolver applies.
        //
        // Added with the service rather than left to the slice that finishes
        // the palette work, because the descriptor `CommandDescriptor::all`
        // generates from `ResourceServiceKind::ALL` is not what an operator
        // sees — `context_commands` is. A generated descriptor with
        // no arm here is exactly the known symptom: `L` works, the palette
        // shows nothing.
        Workspace::Resources
            if root.resources.active_service() == ResourceServiceKind::CloudFunctions
                && root.current_context().is_some()
                && root.resources.selected_cloud_function().is_some() =>
        {
            vec![CommandDescriptor::OpenLogsForSelected(
                ResourceServiceKind::CloudFunctions,
            )]
        }
        // Pub/Sub topics offer the topic-scoped jump from the list, which is
        // the only view they have in this slice.
        //
        // Present for the same reason the Cloud Functions arm above is: a
        // descriptor generated from `ResourceServiceKind::ALL` with no arm here
        // is the same symptom — `L` works and the palette shows nothing.
        Workspace::Resources
            if root.resources.active_service() == ResourceServiceKind::PubSubTopics
                && root.current_context().is_some()
                && root.resources.selected_pubsub_topic().is_some() =>
        {
            vec![CommandDescriptor::OpenLogsForSelected(
                ResourceServiceKind::PubSubTopics,
            )]
        }
        // Subnets are the one VPC table that offers this: flow logs and
        // firewall hit logs are attributed to `gce_subnetwork`, so `L` here
        // reaches real entries where the networks table would reach none.
        Workspace::Resources
            if root.resources.active_service() == ResourceServiceKind::VpcSubnets
                && root.current_context().is_some()
                && root.resources.selected_vpc_subnet().is_some() =>
        {
            vec![CommandDescriptor::OpenLogsForSelected(
                ResourceServiceKind::VpcSubnets,
            )]
        }
        // Subscriptions offer their own subscription-scoped jump, not the
        // parent topic's: a subscription's delivery failures are logged
        // against the subscription, and the topic's entries do not include
        // them. Same arm shape, same reason.
        Workspace::Resources
            if root.resources.active_service() == ResourceServiceKind::PubSubSubscriptions
                && root.current_context().is_some()
                && root.resources.selected_pubsub_subscription().is_some() =>
        {
            vec![CommandDescriptor::OpenLogsForSelected(
                ResourceServiceKind::PubSubSubscriptions,
            )]
        }
        // BigQuery datasets offer the dataset-scoped jump from the list, the
        // only view they have in this slice. Same arm shape and same
        // reason as the two Pub/Sub arms above: a descriptor generated from
        // `ResourceServiceKind::ALL` with no arm here means `L` works and the
        // palette shows nothing.
        Workspace::Resources
            if root.resources.active_service() == ResourceServiceKind::BigQueryDatasets
                && root.current_context().is_some()
                && root.resources.selected_bigquery_dataset().is_some() =>
        {
            vec![CommandDescriptor::OpenLogsForSelected(
                ResourceServiceKind::BigQueryDatasets,
            )]
        }
        _ => Vec::new(),
    }
}

/// The plugin entries offered for whatever service is in view, in file order.
///
/// A plugin is a row-scoped command, so it needs the same three things a jump
/// does — a context to name a configuration, a service in view, and a row
/// under the cursor — and it is offered from no other workspace: the resources
/// selection survives a workspace change, so a guard that read the selection
/// alone would offer a Compute plugin from the tunnel list.
fn plugin_commands(root: &RootState) -> Vec<CommandDescriptor> {
    if root.workspace != Workspace::Resources
        || root.plugins.is_empty()
        || root.resources.is_showing_detail()
    {
        return Vec::new();
    }
    let Some(context) = root.current_context() else {
        return Vec::new();
    };
    let service = root.resources.active_service();
    let Some(fields) = crate::app::state::jumps::selected_fields(&root.resources, service) else {
        return Vec::new();
    };
    crate::app::state::plugins::applicable(&root.plugins, service, &context, &fields)
        .into_iter()
        .map(|offer| CommandDescriptor::Plugin(offer.id))
        .collect()
}

/// The offer a plugin descriptor names, resolved against the current
/// selection. `None` once the row it was offered against is gone.
///
/// This answers "does this id still resolve", **not** "should this plugin be
/// offered" — which is why it carries neither the `Workspace::Resources` guard
/// nor the `!is_showing_detail()` guard that `plugin_commands` has. Both
/// callers hold a `CommandDescriptor::Plugin` that only `plugin_commands`
/// could have produced, so the offering question is already settled by the time
/// either asks: `palette_display_label` renders a row of the list that guard
/// built, and the reducer's `Submit` arm acts on the row the operator selected
/// from it. Duplicating the guards here would buy nothing and would put the
/// same question in two places to keep in step; the failure to watch for is a
/// *third* caller that reaches this without going through the palette, which
/// would need its own guard rather than a widened one here.
pub fn plugin_offer(root: &RootState, id: PluginId) -> Option<Offer> {
    let service = root.resources.active_service();
    let context = root.current_context()?;
    let fields = crate::app::state::jumps::selected_fields(&root.resources, service)?;
    crate::app::state::plugins::applicable(&root.plugins, service, &context, &fields)
        .into_iter()
        .find(|offer| offer.id == id)
}

/// Builds the `project:region:instance` connection name for a Cloud SQL row,
/// when the active context exposes a project.
pub fn cloud_sql_connection_name(
    context: &nuvem_domain::gcp::configuration::GcloudContext,
    instance: &nuvem_domain::gcp::resource::CloudSqlInstance,
) -> Option<String> {
    context
        .project
        .as_ref()
        .map(|project| format!("{project}:{}:{}", instance.region, instance.name))
}

fn navigation_commands(root: &RootState) -> Vec<CommandDescriptor> {
    let mut commands = vec![
        CommandDescriptor::GoHome,
        CommandDescriptor::GoToConfigurations,
        CommandDescriptor::GoToTunnels,
        CommandDescriptor::GoToFinOps,
    ];
    for kind in ResourceServiceKind::ALL {
        let is_available = service_id_for_resource(kind)
            .map(|id| descriptor(id).is_available())
            .unwrap_or(false);
        if is_available {
            commands.push(CommandDescriptor::GoToService(kind));
        }
    }
    if matches!(
        root.workspace,
        Workspace::Configurations | Workspace::Resources | Workspace::Logging
    ) {
        commands.push(CommandDescriptor::RefreshCurrentView);
    }
    // Offered on the evidence of the log rather than on the plugin file:
    // whether a plugin is declared says nothing about whether one has run.
    if root
        .system
        .operations
        .records()
        .iter()
        .any(|record| record.output.is_some())
    {
        commands.push(CommandDescriptor::ShowPluginOutput);
    }
    commands.push(CommandDescriptor::ReloadConfig);
    commands.push(CommandDescriptor::ShowHelp);
    if root.current_context().is_some() {
        commands.push(CommandDescriptor::SwitchProjectContext);
        commands.push(CommandDescriptor::OpenProjectLogs);
    }
    commands
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use tempfile::NamedTempFile;

    use super::*;
    use crate::app::state::plugins::PluginId;
    use crate::app::state::resources::ComputeInstanceAction;
    use crate::config::config_service_id;
    use crate::config::plugins::Plugins;
    use crate::config::problems::ConfigProblems;
    use nuvem_domain::gcp::configuration::{GcloudConfiguration, GcloudConfigurationName};
    use nuvem_domain::gcp::resource::{ComputeInstance, ResourceStatus};

    /// A plugin set built the only way a plugin is ever built: through
    /// [`Plugins::load`] over a real file. A fixture that hand-built `Plugin`
    /// values would be testing something the config layer never hands over,
    /// and the palette reads whatever that layer produced.
    ///
    /// The scope is spelled by [`config_service_id`] rather than by a literal,
    /// so the fixture takes its vocabulary from the config layer instead of
    /// from a second list that could drift from it.
    fn plugins_scoped(scope: ResourceServiceKind, entries: &[(&str, &[&str])]) -> Plugins {
        let mut body = "version = 1\n".to_string();
        for (index, (id, args)) in entries.iter().enumerate() {
            let scope = config_service_id(scope);
            // A distinct key per entry: `Plugins::load` drops a plugin whose
            // key is already used within its scope, which would leave the
            // fixture quietly one plugin short.
            let key = char::from(b'a' + u8::try_from(index).expect("a small fixture"));
            body.push_str(&format!(
                "\n[[plugins]]\nid = {id:?}\nscope = {scope:?}\nkey = {key:?}\n\
                 label = \"Describe {{name}}\"\nargs = {args:?}\n"
            ));
        }
        let mut file = NamedTempFile::new().expect("a temp plugin file");
        file.write_all(body.as_bytes()).expect("the fixture writes");

        let mut problems = ConfigProblems::default();
        let plugins = Plugins::load(file.path(), &mut problems);

        // Without these a malformed fixture would load *no* plugins and every
        // assertion below would hold vacuously over an empty set.
        assert!(
            problems.is_empty(),
            "fixture must be a valid plugin file: {:?}",
            problems.messages()
        );
        assert_eq!(plugins.len(), entries.len(), "every entry must load");
        plugins
    }

    fn compute_plugins(entries: &[(&str, &[&str])]) -> Plugins {
        plugins_scoped(ResourceServiceKind::ComputeInstances, entries)
    }

    fn cloudsql_plugins(entries: &[(&str, &[&str])]) -> Plugins {
        plugins_scoped(ResourceServiceKind::CloudSql, entries)
    }

    /// The Compute list on screen, one instance under the cursor, and a
    /// context resolved — the state in which a compute-scoped plugin is
    /// supposed to be on offer.
    fn root_with_one_compute_instance_selected(name: &str) -> RootState {
        let mut root = RootState {
            workspace: Workspace::Resources,
            ..RootState::default()
        };
        root.configurations.set_rows(vec![GcloudConfiguration {
            name: GcloudConfigurationName::new("dev"),
            is_active: true,
            account: Some("dev@example.com".to_string()),
            project: Some("dev-project".to_string()),
            region: Some("europe-west1".to_string()),
            zone: Some("europe-west1-b".to_string()),
        }]);
        let context = root.current_context().expect("the active configuration");
        root.resources
            .select_service(ResourceServiceKind::ComputeInstances);
        root.resources.set_instances(
            context,
            vec![ComputeInstance {
                id: format!("{name}-id"),
                name: name.to_string(),
                zone: "europe-west1-b".to_string(),
                status: ResourceStatus::Running,
                machine_type: "e2-medium".to_string(),
                internal_ip: Some("10.0.0.2".to_string()),
                external_ip: None,
            }],
        );

        // The fixture's whole job is to be a state a plugin resolves against;
        // a silent failure here would make every assertion below vacuous.
        assert!(root.resources.selected_visible_instance().is_some());
        root
    }

    /// A plugin scoped to the service in view is offered against the selected
    /// row, labelled the way its template names that row.
    #[test]
    fn a_scoped_plugin_is_offered_against_the_selected_row() {
        let mut root = root_with_one_compute_instance_selected("web-01");
        root.plugins =
            compute_plugins(&[("describe", &["compute", "instances", "describe", "{name}"])]);

        let commands = available_commands(&root);

        assert!(
            commands.contains(&CommandDescriptor::Plugin(PluginId(0))),
            "the palette listed {commands:?}"
        );
        assert_eq!(
            palette_display_label(CommandDescriptor::Plugin(PluginId(0)), &root),
            "Describe web-01"
        );
    }

    /// A plugin scoped elsewhere is not offered here — the scope is the whole
    /// of what makes a plugin row-scoped.
    #[test]
    fn a_plugin_scoped_to_another_service_is_not_offered() {
        let mut root = root_with_one_compute_instance_selected("web-01");
        root.plugins =
            cloudsql_plugins(&[("describe", &["sql", "instances", "describe", "{name}"])]);

        assert!(
            !available_commands(&root)
                .iter()
                .any(|command| matches!(command, CommandDescriptor::Plugin(_)))
        );
    }

    /// The `label()` placeholder must never reach a palette row. It exists
    /// only because the enum's method cannot see the state the real label
    /// lives in.
    #[test]
    fn no_palette_row_shows_the_plugin_placeholder() {
        let mut root = root_with_one_compute_instance_selected("web-01");
        root.plugins =
            compute_plugins(&[("describe", &["compute", "instances", "describe", "{name}"])]);

        for command in available_commands(&root) {
            assert_ne!(
                palette_display_label(command, &root),
                "Plugin command",
                "{command:?} rendered the placeholder"
            );
        }
    }

    /// A plugin is found by what the operator called it, not by the
    /// placeholder `label()` answers with. Typing the first word of the
    /// rendered label has to reach it, or a palette with thirty built-in
    /// entries hides every plugin behind them.
    #[test]
    fn a_plugin_is_found_by_typing_a_word_of_its_rendered_label() {
        let mut root = root_with_one_compute_instance_selected("web-01");
        root.plugins =
            compute_plugins(&[("describe", &["compute", "instances", "describe", "{name}"])]);
        root.command.open();
        for value in "describe".chars() {
            root.command.append_char(value);
        }

        let visible = root.command.visible_commands(&root);

        assert!(
            visible.contains(&CommandDescriptor::Plugin(PluginId(0))),
            "typing \"describe\" left {visible:?}"
        );
    }

    /// The resources selection survives leaving the workspace, so "an instance
    /// is selected" is still true from the tunnel list. The service in view is
    /// what a plugin is scoped to, and there is no service in view here.
    #[test]
    fn a_plugin_is_not_offered_from_another_workspace() {
        let mut root = root_with_one_compute_instance_selected("web-01");
        root.plugins =
            compute_plugins(&[("describe", &["compute", "instances", "describe", "{name}"])]);
        root.workspace = Workspace::Tunnels;
        assert!(
            root.resources.selected_visible_instance().is_some(),
            "the retained selection is exactly what a workspace-blind guard \
             would act on"
        );

        assert!(
            !available_commands(&root)
                .iter()
                .any(|command| matches!(command, CommandDescriptor::Plugin(_)))
        );
    }

    /// An open detail pane withholds a plugin, the conservative direction: the
    /// pane is a different set of affordances, and a plugin is offered where it
    /// was designed to be read, from the list.
    ///
    /// The built-ins are *not* uniform about this, so this is not "the same
    /// rule as the commands beside it": Compute, Cloud SQL and Cloud Run's
    /// *service* arm guard on `!is_showing_detail()`, and GKE guards only its
    /// log jump — its k9s command is offered from the detail pane too.
    ///
    /// The genuine asymmetry is four arms, because these are the ones a detail
    /// pane can be open over: Secret Manager (offered from the list and the
    /// pane alike), Cloud Storage, GKE's k9s command, and the Cloud Run
    /// *revision* log jump — which is reachable only from the detail pane's
    /// Revisions tab, so it is offered exactly where `is_showing_detail()` is
    /// true. With a secret's detail pane open the palette still offers "Open
    /// logs for selected secret" while a secrets-scoped plugin has vanished,
    /// and with a revision open the same is true of a `cloudrun`-scoped one.
    ///
    /// The remaining unguarded arms cannot diverge at all, and are named here
    /// so the next reader does not sweep them: Cloud Functions, Persistent
    /// Disks, Pub/Sub, IAM service accounts and the IAM policy all answer
    /// `is_showing_detail()` with a hard-coded `false` (`resources::mod`), so
    /// their missing guard tests a condition that never holds — and Persistent
    /// Disks and both IAM services have no `context_commands` arm to guard in
    /// the first place. The four-arm inconsistency is the built-ins', and
    /// settling it means deciding one rule for all eleven services — out of
    /// scope for this slice, which is why it is recorded here rather than fixed
    /// by widening the plugin guard's claim.
    #[test]
    fn a_plugin_is_not_offered_while_a_detail_pane_is_open() {
        let mut root = root_with_one_compute_instance_selected("web-01");
        root.plugins =
            compute_plugins(&[("describe", &["compute", "instances", "describe", "{name}"])]);
        root.resources.open_detail().expect("a row to open");
        assert!(root.resources.is_showing_detail());

        assert!(
            !available_commands(&root)
                .iter()
                .any(|command| matches!(command, CommandDescriptor::Plugin(_)))
        );
    }

    /// `CommandDescriptor::all()` cannot enumerate plugins, so the partition
    /// test that keeps every other descriptor honest is blind to this one.
    /// Pinned separately rather than left to that test's silence.
    #[test]
    fn every_plugin_descriptor_is_mutating_whatever_its_index() {
        for index in [0, 1, 7, usize::MAX] {
            assert!(
                CommandDescriptor::Plugin(PluginId(index)).is_mutating(),
                "every plugin descriptor is mutating as far as the enum can \
                 tell, including index {index}"
            );
        }
    }

    #[test]
    fn stop_selected_compute_instance_is_mutating() {
        assert!(CommandDescriptor::StopSelectedComputeInstance.is_mutating());
    }

    #[test]
    fn start_selected_compute_instance_is_mutating_even_though_not_destructive() {
        assert!(
            CommandDescriptor::StartSelectedComputeInstance.is_mutating(),
            "Start is not is_destructive (billing resumes but nothing is torn down), \
             yet it does mutate cloud state by booting the instance — a gate built on \
             is_destructive alone would let it through"
        );
    }

    #[test]
    fn non_mutating_commands_are_not_mutating() {
        for command in [
            CommandDescriptor::CopySelectedInstanceName,
            CommandDescriptor::OpenProjectLogs,
            CommandDescriptor::ShowSelectedComputeInstanceSsh,
            CommandDescriptor::StartTunnelForSelectedComputeInstance,
            CommandDescriptor::ActivateSelectedConfiguration,
        ] {
            assert!(!command.is_mutating(), "{command:?} must not be mutating");
        }
    }

    #[test]
    fn is_mutating_and_is_destructive_disagree_on_start() {
        assert!(CommandDescriptor::StartSelectedComputeInstance.is_mutating());
        assert!(!ComputeInstanceAction::Start.is_destructive());
    }

    #[test]
    fn opening_a_cluster_in_k9s_is_mutating() {
        assert!(
            CommandDescriptor::OpenSelectedGkeClusterInK9s.is_mutating(),
            "k9s is a read-write Kubernetes UI on a live cluster, and the launch \
             path runs `get-credentials` when no context exists — handing the \
             operator an interactive session that can change cloud state is the \
             same category as SSH, which is already refused"
        );
    }

    /// The full partition of `CommandDescriptor` across `is_mutating`.
    ///
    /// Exhaustiveness in `is_mutating` forces a NEW variant to receive a
    /// decision; it does nothing to keep an EXISTING decision correct. Moving
    /// any variant between the two arms leaves every other test in the suite
    /// green, so this is the test that has to name both sets in full.
    ///
    /// "Every descriptor" here means every descriptor `all()` can build, which
    /// excludes `Plugin`: `all()` is derived from the enum's compile-time
    /// sets, and a plugin comes from a file, so there is no set of `PluginId`s
    /// to expand over. That variant is pinned by
    /// `every_plugin_descriptor_is_mutating_whatever_its_index` instead.
    #[test]
    fn is_mutating_partitions_every_descriptor_exactly_as_declared() {
        let mutating = [
            CommandDescriptor::StartSelectedComputeInstance,
            CommandDescriptor::StopSelectedComputeInstance,
            CommandDescriptor::ResetSelectedComputeInstance,
            CommandDescriptor::SuspendSelectedComputeInstance,
            CommandDescriptor::ResumeSelectedComputeInstance,
            CommandDescriptor::StartSelectedCloudSqlInstance,
            CommandDescriptor::StopSelectedCloudSqlInstance,
            CommandDescriptor::RestartSelectedCloudSqlInstance,
            CommandDescriptor::FailoverSelectedCloudSqlInstance,
            CommandDescriptor::SshSelectedComputeInstance,
            CommandDescriptor::OpenSelectedGkeClusterInK9s,
        ];
        let non_mutating = [
            CommandDescriptor::ActivateSelectedConfiguration,
            CommandDescriptor::ShowSelectedComputeInstanceSsh,
            CommandDescriptor::StartTunnelForSelectedComputeInstance,
            CommandDescriptor::CopySelectedInstanceName,
            CommandDescriptor::CopySelectedInstanceZone,
            CommandDescriptor::CopySelectedInstanceInternalIp,
            CommandDescriptor::CopySelectedInstanceExternalIp,
            CommandDescriptor::CopySelectedInstanceSelfLink,
            CommandDescriptor::CopySelectedCloudSqlConnectionName,
            CommandDescriptor::StartTunnelForSelectedCloudSqlInstance,
            CommandDescriptor::OpenProjectLogs,
            CommandDescriptor::OpenLogsForSelected(ResourceServiceKind::ComputeInstances),
            CommandDescriptor::OpenLogsForSelected(ResourceServiceKind::GkeClusters),
            CommandDescriptor::OpenLogsForSelected(ResourceServiceKind::CloudRunServices),
            CommandDescriptor::OpenLogsForSelected(ResourceServiceKind::CloudSql),
            CommandDescriptor::OpenLogsForSelected(ResourceServiceKind::CloudStorage),
            CommandDescriptor::OpenLogsForSelectedCloudRunRevision,
            CommandDescriptor::OpenLogsForSelected(ResourceServiceKind::SecretManagerSecrets),
            CommandDescriptor::OpenLogsForSelected(ResourceServiceKind::CloudFunctions),
            // Generated for every kind, and read-only like the rest. Persistent
            // disks produce no logs, so `context_commands` never offers this
            // one — see the negative assertion in `reducer_command_test`.
            CommandDescriptor::OpenLogsForSelected(ResourceServiceKind::PersistentDisks),
            CommandDescriptor::OpenLogsForSelected(ResourceServiceKind::PubSubTopics),
            CommandDescriptor::OpenLogsForSelected(ResourceServiceKind::PubSubSubscriptions),
            CommandDescriptor::OpenLogsForSelected(ResourceServiceKind::BigQueryDatasets),
            // Generated like the disks one, and equally never offered:
            // service accounts do not produce logs.
            CommandDescriptor::OpenLogsForSelected(ResourceServiceKind::IamServiceAccounts),
            // Generated alike and equally never offered: a policy binding is a
            // grant, not a log source.
            CommandDescriptor::OpenLogsForSelected(ResourceServiceKind::IamPolicy),
            // Generated alike and equally never offered: flow logs and
            // firewall hit logs are attributed to the subnetwork, never to the
            // network, so a network-scoped query could only return zero rows.
            CommandDescriptor::OpenLogsForSelected(ResourceServiceKind::VpcNetworks),
            // Offered, unlike the networks one: this is where flow logs and
            // firewall hit logs actually live.
            CommandDescriptor::OpenLogsForSelected(ResourceServiceKind::VpcSubnets),
            // Generated alike and never offered, for the networks reason one
            // step further: a rule's hits are logged against the subnetwork
            // the traffic crossed, with the rule named in the payload, so a
            // rule-scoped query returns zero rows — which on a DENY rule reads
            // as "nothing was blocked".
            CommandDescriptor::OpenLogsForSelected(ResourceServiceKind::FirewallRules),
            CommandDescriptor::GoHome,
            CommandDescriptor::GoToConfigurations,
            CommandDescriptor::GoToTunnels,
            CommandDescriptor::GoToFinOps,
            CommandDescriptor::GoToService(ResourceServiceKind::ComputeInstances),
            CommandDescriptor::GoToService(ResourceServiceKind::GkeClusters),
            CommandDescriptor::GoToService(ResourceServiceKind::CloudSql),
            CommandDescriptor::GoToService(ResourceServiceKind::CloudRunServices),
            CommandDescriptor::GoToService(ResourceServiceKind::CloudStorage),
            CommandDescriptor::GoToService(ResourceServiceKind::SecretManagerSecrets),
            CommandDescriptor::GoToService(ResourceServiceKind::CloudFunctions),
            CommandDescriptor::GoToService(ResourceServiceKind::PersistentDisks),
            CommandDescriptor::GoToService(ResourceServiceKind::PubSubTopics),
            CommandDescriptor::GoToService(ResourceServiceKind::PubSubSubscriptions),
            CommandDescriptor::GoToService(ResourceServiceKind::BigQueryDatasets),
            CommandDescriptor::GoToService(ResourceServiceKind::IamServiceAccounts),
            CommandDescriptor::GoToService(ResourceServiceKind::IamPolicy),
            CommandDescriptor::GoToService(ResourceServiceKind::VpcNetworks),
            CommandDescriptor::GoToService(ResourceServiceKind::VpcSubnets),
            CommandDescriptor::GoToService(ResourceServiceKind::FirewallRules),
            CommandDescriptor::RefreshCurrentView,
            CommandDescriptor::ReloadConfig,
            // Reads what a run already printed. It starts no command and
            // reaches no cloud resource, so `--read-only` leaves it alone —
            // the run it shows was already refused or already allowed.
            CommandDescriptor::ShowPluginOutput,
            CommandDescriptor::ShowHelp,
            CommandDescriptor::SwitchProjectContext,
        ];

        for command in mutating {
            assert!(
                command.is_mutating(),
                "{command:?} is listed as mutating but is_mutating() said false"
            );
        }
        for command in non_mutating {
            assert!(
                !command.is_mutating(),
                "{command:?} is listed as non-mutating but is_mutating() said true"
            );
        }

        // The two lists together must account for every constructible
        // descriptor. The total comes from the enum itself
        // (`CommandDescriptor::all`, generated from the variant list), never
        // from a literal: a hardcoded count is independent of the enum, so a
        // NEW variant left out of both lists would leave this green — the k9s
        // misclassification restated, and the exact gap this test exists to
        // close.
        let listed: Vec<CommandDescriptor> = mutating
            .iter()
            .chain(non_mutating.iter())
            .copied()
            .collect();
        for command in CommandDescriptor::all() {
            assert!(
                listed.contains(&command),
                "{command:?} is in neither list — its classification is pinned by nothing"
            );
        }
        assert_eq!(
            listed.len(),
            CommandDescriptor::all().len(),
            "the two lists must partition the descriptors exactly once each: \
             {listed:?}"
        );
    }

    /// Every service reachable from `ResourceServiceKind::ALL` has a log
    /// descriptor — that the variant exists, not that the palette offers it.
    ///
    /// Whether the palette actually lists the entry is a property of
    /// `context_commands`, whose per-service guards this cannot see; the
    /// end-to-end walk over the open palette lives in
    /// `every_service_offers_its_log_jump_from_the_open_palette`
    /// (tests/reducer_command_test.rs).
    ///
    /// This walks `ALL` rather than naming services, because naming them is
    /// the defect this pins against. Adding a service forces 71 match sites
    /// across 13 files and `command.rs` is in none of them, so when a slice
    /// added Secret Manager the `L` keybinding gained its arm — that is a
    /// match the compiler checks — and the hand-written descriptor list gained
    /// nothing.
    ///
    /// It passes at the commit that introduced it: the list was complete by
    /// then. It is a regression pin, not a bug detector.
    #[test]
    fn every_service_has_a_log_descriptor() {
        let all = CommandDescriptor::all();
        for kind in ResourceServiceKind::ALL {
            assert!(
                all.contains(&CommandDescriptor::OpenLogsForSelected(kind)),
                "{kind:?} has no log descriptor, so no palette entry can exist \
                 for it and the only way to reach its logs is the L keybinding"
            );
        }
    }

    /// Every generated log entry reads the way the hand-written one did.
    ///
    /// The nouns come from `selection_noun`, which the rest of the app already
    /// uses for a selection, so this pins that the two agree rather than that
    /// the palette owns a second copy of them.
    #[test]
    fn generated_log_labels_name_the_selection_the_way_the_app_does() {
        for kind in ResourceServiceKind::ALL {
            assert_eq!(
                CommandDescriptor::OpenLogsForSelected(kind).label(),
                format!("Open logs for selected {}", kind.selection_noun()),
            );
        }
        assert_eq!(
            CommandDescriptor::OpenLogsForSelected(ResourceServiceKind::SecretManagerSecrets)
                .label(),
            "Open logs for selected secret",
            "the label added by hand must survive being generated"
        );
    }

    /// The Cloud Run revision entry is deliberately NOT generated from `ALL`.
    ///
    /// A revision is a sub-view of a Cloud Run service rather than a
    /// `ResourceServiceKind`, so it has no member of `ALL` to come from and
    /// stays a unit variant. Pinned so a later tidy-up does not "finish" the
    /// collapse by deleting it.
    #[test]
    fn the_cloud_run_revision_log_entry_survives_outside_the_generated_set() {
        assert!(
            CommandDescriptor::all()
                .contains(&CommandDescriptor::OpenLogsForSelectedCloudRunRevision),
            "the revision entry is not derived from ALL and must be declared on its own"
        );
    }
}
