use super::command::CommandPaletteState;
use nuvem_domain::gcp::configuration::GcloudContext;
use nuvem_domain::gcp::resource::ResourceServiceKind;

use super::configurations::ConfigurationsState;
use super::finops::FinOpsState;
use super::health::{ServiceHealthHistory, observe_services};
use super::jumps::JumpPickerState;
use super::logging::LoggingState;
use super::navigation::NavigationState;
use super::navigator::ServiceNavigatorState;
use super::remembered_view::RememberedViews;
use super::resources::ResourcesState;
use super::system::SystemState;
use super::tunnels::TunnelsState;
use crate::config::aliases::AliasTable;
use crate::config::jumps::JumpRules;
use crate::config::plugins::Plugins;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Workspace {
    Home,
    Configurations,
    Resources,
    Logging,
    /// The managed-tunnel list. Unlike every other workspace this one is not a
    /// view of the active context: its rows are child processes that outlive a
    /// configuration change (see [`super::tunnels`]).
    Tunnels,
    /// Cost recommendations, budgets and the global inventory.
    ///
    /// Its own workspace rather than a `ResourceServiceKind`: the unit is
    /// money rather than rows, the key is a recommender rather than a service,
    /// and the axis crosses services — the disk recommender is about the same
    /// disks `PersistentDisks` already lists. Forcing it through the resource
    /// shape would mean stub arms in the exhaustiveness walks, which teaches a
    /// gate that catches real omissions to accept placeholders. Carries no
    /// sub-location, exactly like [`Workspace::Tunnels`].
    FinOps,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RootState {
    pub workspace: Workspace,
    pub command: CommandPaletteState,
    pub configurations: ConfigurationsState,
    pub navigator: ServiceNavigatorState,
    pub resources: ResourcesState,
    pub system: SystemState,
    pub navigation: NavigationState,
    pub logging: LoggingState,
    /// Managed tunnel children. Unlike every other list here this one is *not*
    /// scoped to the active context: a tunnel keeps running across a
    /// configuration change, so see [`super::tunnels`] for why it survives one.
    pub tunnels: TunnelsState,
    /// The FinOps grid: one cell per (recommender, location) probe. Money
    /// rather than rows, which is why it is its own workspace and not a
    /// ResourceServiceKind — see the design spec.
    pub finops: FinOpsState,
    /// Each service's bounded, context-stamped count history — the
    /// series Home's sparklines draw. Filled by
    /// [`RootState::sample_service_health`] from the runtime's action funnel
    /// rather than by the thirteen list-result reducer arms; see
    /// [`super::health`] for why the sampler observes instead of being
    /// notified.
    pub health: ServiceHealthHistory,
    /// The related-resource jump picker (`J`), while it is on screen. `None`
    /// is the closed state; the modal is never opened empty (see
    /// [`super::jumps::JumpPickerState::new`]).
    pub jump_picker: Option<JumpPickerState>,
    /// User-defined service aliases (from the config file's `[aliases]`
    /// table), merged into command-palette and service-navigator matching.
    /// Set at startup from the resolved config; empty when no config file
    /// (or no `[aliases]` table) is present.
    pub aliases: AliasTable,
    /// The `[[jumps]]` rules the `J` picker's offers are derived from. Set at
    /// startup from the resolved config, and the built-in set when the file
    /// configures none — an unconfigured user's picker is this engine's
    /// output, not a bypass of it.
    pub jump_rules: JumpRules,
    /// The validated `[[plugins]]` entries the palette offers against the
    /// selected row. Set at startup from the resolved config, and empty when
    /// the plugin file is absent — which is the overwhelmingly common case.
    ///
    /// Unlike `jump_rules` this is **not** replaced on reload: a plugin
    /// declaration is the one part of the configuration that can cause a
    /// process to run, and re-reading it while running opens a window between
    /// the preview the operator read and the file on disk. It would also
    /// repoint every [`super::plugins::PluginId`] a palette is already holding
    /// at whatever plugin now occupies that index. Reload reports the change
    /// instead (see `RuntimeSession::reload_config`).
    pub plugins: Plugins,
    /// Whether cloud-resource mutations are currently blocked. Set at
    /// startup from the resolved config (`--read-only` flag OR `[general]
    /// read_only` in the config file — either one is enough to turn it on)
    /// and re-folded across a config reload.
    ///
    /// The fold is monotonic: a reload can only turn this ON, never off. There
    /// is deliberately no per-session toggle, and "edit the config file, then
    /// run Reload configuration" would be one — Reload is itself permitted
    /// under the mode. Once on, read-only holds for the life of the process.
    ///
    /// This is a promise about CLOUD RESOURCE mutations only: it does not
    /// claim the app makes no local changes. With `read_only` set the app
    /// still activates gcloud configurations, opens tunnels, and writes
    /// clipboard content — none of those touch a cloud resource.
    pub read_only: bool,
    /// `Some(scope)` when `,` is armed and the next character will be read as a
    /// plugin key — carrying the service it was armed over.
    ///
    /// Root-level rather than inside the resources workspace because
    /// `input_context` derives `Mode::PluginPrefix` from it, and every other
    /// mode in that ladder reads a root-level fact. Cleared by every action
    /// the armed mode dispatches, so it cannot outlive its press.
    ///
    /// It carries the scope rather than a bare `bool` because clearing on the
    /// keyboard is not enough. `reduce` runs `sync_remembered_view` on *every*
    /// action, and `EffectResult::ConfigurationsLoaded` /
    /// `ConfigurationActivated` land the remembered service for the new
    /// configuration — so the scope can change under an armed prefix with no
    /// key pressed at all. Storing what it was armed over lets the press refuse
    /// a scope the operator never armed, instead of answering with that scope's
    /// plugin (one character may legally key a plugin in two scopes), and makes
    /// the prompt's "the key list is never empty" invariant true by
    /// construction rather than by assertion.
    pub plugin_prefix_pending: Option<ResourceServiceKind>,
    /// Which resources service the user last had open in each gcloud
    /// configuration. Loaded once at startup from the state file and kept in
    /// sync by the one choke point in [`crate::app::reducer::root::reduce`] —
    /// see [`super::remembered_view`] for why it is keyed by configuration
    /// name and stores the service as a raw string.
    pub remembered_views: RememberedViews,
    /// The service to open on for a configuration with no remembered view.
    /// Resolved from `general.default_service` and stored here because the
    /// fallback is applied inside the reducer, which has no access to the
    /// resolved `Config`.
    pub default_service: ResourceServiceKind,
    /// Set when this session must stop writing the state file, and never
    /// cleared for the life of the process. Two things set it, for two
    /// different reasons:
    ///
    /// **A save failed.** The latch is the point. Saves are eager — one per
    /// service change — so on a read-only filesystem an unlatched writer would
    /// attempt the create/fsync/rename/fsync dance on every service keypress
    /// and put an error on the status line every time, turning a lost
    /// convenience into a visible malfunction. One warning, then silence.
    ///
    /// **The startup load failed.** A state file that is corrupt, or that a
    /// newer nuvem wrote, is read as an empty map — but saving over it would
    /// then replace it with this build's `version = 1`, which is exactly what
    /// decision 4 in [`crate::config::state`] exists to prevent. We can't read
    /// it, so we don't get to destroy it. A *missing* file is not this case:
    /// there is nothing to clobber, and a first run must save normally.
    ///
    /// Either way the in-memory map still learns, so the session behaves
    /// normally and only durability is lost.
    pub remembered_view_saves_suppressed: bool,
}

impl Default for RootState {
    fn default() -> Self {
        Self {
            workspace: Workspace::Home,
            command: CommandPaletteState::default(),
            configurations: ConfigurationsState::default(),
            navigator: ServiceNavigatorState::default(),
            resources: ResourcesState::default(),
            system: SystemState::default(),
            navigation: NavigationState::default(),
            logging: LoggingState::default(),
            tunnels: TunnelsState::default(),
            finops: FinOpsState::default(),
            health: ServiceHealthHistory::default(),
            jump_picker: None,
            aliases: AliasTable::empty(),
            jump_rules: JumpRules::defaults(),
            plugins: Plugins::empty(),
            read_only: false,
            plugin_prefix_pending: None,
            remembered_views: RememberedViews::default(),
            // Matches `Config::default()` in `src/config/mod.rs`, so a
            // state built without a resolved config falls back the same way a
            // config-less run does.
            default_service: ResourceServiceKind::ComputeInstances,
            remembered_view_saves_suppressed: false,
        }
    }
}

impl RootState {
    /// Folds every settled listing result into [`Self::health`], stamping any
    /// new one at `at`.
    ///
    /// `at` is a parameter rather than an `Instant::now()` in here for the
    /// reason [`super::tunnels`] takes one: it is what makes the sample's age
    /// — and so slice 3's delta decay — assertable without sleeping. The
    /// clock read belongs to the runtime layer, which is also what keeps the
    /// reducer a pure state transition.
    ///
    /// Called on every reduction. That is not free — `observe_services` walks
    /// the registry and clones the context stamp of every settled service, and
    /// `record` then drops most of what it built as already seen. It is bought
    /// deliberately: the alternative is notifying the sampler from each of the
    /// thirteen list-result arms, which is the hand-written service list
    /// that has bitten before. nuvem reduces on operator input and on landed effect
    /// results, not on a tick, so the rate is bounded by what a person and a
    /// poll interval produce rather than by a frame clock.
    pub fn sample_service_health(&mut self, at: std::time::Instant) {
        for observation in observe_services(&self.resources) {
            self.health.record(observation, at);
        }
    }

    pub fn current_context(&self) -> Option<GcloudContext> {
        self.configurations
            .active()
            .map(|configuration| configuration.context())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AppState {
    pub root: RootState,
}

impl AppState {
    pub fn new() -> Self {
        Self::default()
    }
}
