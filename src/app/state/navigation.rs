//! Reducer-owned navigation history: back/forward stacks of view locations.

use nuvem_domain::gcp::resource::ResourceServiceKind;

use super::resources::{
    CloudRunDetailTab, CloudRunRevisionTab, CloudSqlDetailTab, ComputeDetailTab, GkeDetailTab,
    RowIdentity, row_identity,
};
use super::root::{RootState, Workspace};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResourceKey {
    Compute { name: String, zone: String },
    Gke { name: String, location: String },
    CloudSql { name: String, region: String },
    CloudRun { name: String, region: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DetailTab {
    Compute(ComputeDetailTab),
    Gke(GkeDetailTab),
    CloudSql(CloudSqlDetailTab),
    CloudRun(CloudRunDetailTab),
}

/// Every tab of every service, flattened. Sized from the service tab enums so
/// adding a variant there is a compile error here rather than a silently short
/// list.
const DETAIL_TAB_COUNT: usize = ComputeDetailTab::ALL.len()
    + GkeDetailTab::ALL.len()
    + CloudSqlDetailTab::ALL.len()
    + CloudRunDetailTab::ALL.len();

impl DetailTab {
    /// Every detail tab there is, in service order. Built from the per-service
    /// `ALL` constants the tab bars already iterate, so the config vocabulary
    /// [`config_key`](Self::config_key) exposes cannot fall behind the tabs
    /// that actually exist.
    pub const ALL: &'static [Self] = &Self::ALL_TABS;

    const ALL_TABS: [Self; DETAIL_TAB_COUNT] = {
        let mut all = [Self::Compute(ComputeDetailTab::Summary); DETAIL_TAB_COUNT];
        let mut at = 0;
        let mut i = 0;
        while i < ComputeDetailTab::ALL.len() {
            all[at] = Self::Compute(ComputeDetailTab::ALL[i]);
            at += 1;
            i += 1;
        }
        i = 0;
        while i < GkeDetailTab::ALL.len() {
            all[at] = Self::Gke(GkeDetailTab::ALL[i]);
            at += 1;
            i += 1;
        }
        i = 0;
        while i < CloudSqlDetailTab::ALL.len() {
            all[at] = Self::CloudSql(CloudSqlDetailTab::ALL[i]);
            at += 1;
            i += 1;
        }
        i = 0;
        while i < CloudRunDetailTab::ALL.len() {
            all[at] = Self::CloudRun(CloudRunDetailTab::ALL[i]);
            at += 1;
            i += 1;
        }
        all
    };

    /// How the config file names this tab: `"<service>:<tab>"`, e.g.
    /// `"compute:network"` or `"gke:node-pools"`.
    ///
    /// Derived from [`label`](Self::label) rather than spelled out, so the
    /// name a user writes and the name the tab bar shows are the same string
    /// by construction.
    pub fn config_key(self) -> String {
        format!(
            "{}:{}",
            crate::config::config_service_id(self.service()),
            self.key_tab_half()
        )
    }

    /// The tab half of a config key. Shared with
    /// [`from_config_key`](Self::from_config_key) so the spelling that is
    /// emitted is by construction the spelling that is matched.
    fn key_tab_half(self) -> String {
        self.label().to_ascii_lowercase().replace(' ', "-")
    }

    /// The tab a config file's `"<service>:<tab>"` names, matched leniently
    /// (trimmed, case-insensitive) over [`ALL`](Self::ALL).
    ///
    /// The two halves are resolved by their own vocabularies, which is what
    /// keeps this parse-lenient and [`config_key`](Self::config_key)
    /// emit-canonical, the shape the rest of the config layer has. The service
    /// half goes through `crate::config::resolve_service_kind`, the same
    /// resolver a `[[jumps]]` rule's `from` uses, so `"gce:disks"` and
    /// `"compute:disks"` name one tab rather than one tab and one error. The
    /// tab half is matched against the labels the tab bar renders, so there is
    /// no second list of tab names to maintain and an unparseable key is one
    /// that names no tab.
    pub fn from_config_key(raw: &str) -> Option<Self> {
        let wanted = raw.trim().to_ascii_lowercase();
        let (service, tab) = wanted.split_once(':')?;
        let kind = crate::config::resolve_service_kind(service)?;
        Self::ALL
            .iter()
            .copied()
            .find(|candidate| candidate.service() == kind && candidate.key_tab_half() == tab)
    }

    /// The service whose pane this tab belongs to. A tab names exactly one
    /// service, so a holder of a tab can ask which pane it is about instead of
    /// having to carry the service beside it.
    pub fn service(self) -> ResourceServiceKind {
        match self {
            Self::Compute(_) => ResourceServiceKind::ComputeInstances,
            Self::Gke(_) => ResourceServiceKind::GkeClusters,
            Self::CloudSql(_) => ResourceServiceKind::CloudSql,
            Self::CloudRun(_) => ResourceServiceKind::CloudRunServices,
        }
    }

    /// The tab bar's own wording for this tab, delegated to the service tab
    /// enum that owns it — the single place any renaming happens.
    pub fn label(self) -> &'static str {
        match self {
            Self::Compute(tab) => tab.label(),
            Self::Gke(tab) => tab.label(),
            Self::CloudSql(tab) => tab.label(),
            Self::CloudRun(tab) => tab.label(),
        }
    }
}

/// Where a `[[jumps]]` rule lands.
///
/// Beside [`DetailTab`] rather than inside it, and that placement is the whole
/// design. A fifth `DetailTab` variant would have to be answered by
/// [`DetailTab::service`], [`DetailTab::label`], [`DetailTab::ALL`] and every
/// exhaustive match on a tab in the render and reducer layers — very nearly
/// the whole suite depends on this type, directly or through those matches.
/// A tab is a *pane* of a detail view; a service list is not one, and
/// saying so in a second enum leaves all 20 existing `DetailTab` sites
/// compiling unchanged.
///
/// What this does **not** buy: a target that lands on a list *filtered* by
/// something about the source row ("the firewall rules that target this
/// instance"). That is a third shape, and it needs domain matching this type
/// deliberately has no opinion about — a separate gap.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JumpTarget {
    /// A resource's detail pane, opened at this tab. The original shape.
    Detail(DetailTab),
    /// A service's list, with the cursor put on one row. Reaches the twelve
    /// services that have no `DetailTab` spelling and could never be named as
    /// a `to` before.
    List(ResourceServiceKind),
}

impl JumpTarget {
    /// The service whose view this target belongs to.
    pub fn service(self) -> ResourceServiceKind {
        match self {
            Self::Detail(tab) => tab.service(),
            Self::List(kind) => kind,
        }
    }

    /// How the config file names this target: a detail tab's
    /// `"<service>:<tab>"`, or a bare `"<service>"` for a list.
    ///
    /// The two vocabularies cannot collide, because a service id never
    /// contains a `:` and a detail key always does. That is what makes the
    /// grammar a strict *superset*: every `to` written earlier still
    /// parses to the detail target it always meant.
    pub fn config_key(self) -> String {
        match self {
            Self::Detail(tab) => tab.config_key(),
            Self::List(kind) => crate::config::config_service_id(kind).to_string(),
        }
    }

    /// The target a config file's `to` names, or `None` when it names nothing.
    ///
    /// The `:` decides which vocabulary is consulted, before either is asked,
    /// so the two are read as the separate grammars they are rather than as a
    /// fallback chain.
    ///
    /// It buys no behaviour on its own, and the honest version of that is worth
    /// stating: trying `DetailTab::from_config_key` first and falling through
    /// would answer identically for every input, because
    /// `resolve_service_kind` never matches a string containing a `:`. It is
    /// here so that the *reason* `"vpc-networks:zzz"` is nothing — a bad tab
    /// half, not an unknown service — is a property of the code rather than of
    /// that coincidence.
    pub fn from_config_key(raw: &str) -> Option<Self> {
        if raw.contains(':') {
            return DetailTab::from_config_key(raw).map(Self::Detail);
        }
        crate::config::resolve_service_kind(raw.trim()).map(Self::List)
    }

    /// What the destination *collection* is called: the tab bar's wording for
    /// a detail tab, the sidebar's wording for a service list. The name a
    /// document or a message uses for where a rule goes, as distinct from
    /// [`default_label`](Self::default_label), which names the single row.
    pub fn label(self) -> &'static str {
        match self {
            Self::Detail(tab) => tab.label(),
            Self::List(kind) => kind.label(),
        }
    }

    /// Whether landing on this target needs a location as well as a name.
    ///
    /// Read off [`crate::app::state::resources::row_identity`] — what the
    /// destination's own `row_id` builds — and deliberately *not* off
    /// `config::jumps::location_field`, which describes what a rule may read
    /// from the row it is offered *from*. The two are different questions and
    /// they disagree for two services: see [`RowIdentity`].
    pub fn keys_rows_by_location(self) -> bool {
        match self {
            // Every service with a detail pane is zonal or regional, and
            // `ResourceKey` carries that half in all four of its shapes.
            Self::Detail(_) => true,
            Self::List(kind) => {
                matches!(row_identity(kind), RowIdentity::NameAndLocation)
            }
        }
    }

    /// Whether a rule may land on this target at all.
    ///
    /// False only for a list whose row identity is [`RowIdentity::Opaque`]: no
    /// pair of templates can render the IAM policy table's role-and-condition
    /// half, so such a rule would parse, resolve, offer a row, and then never
    /// match anything. Refused at load instead, where the reason can be said.
    pub fn is_landable(self) -> bool {
        match self {
            Self::Detail(_) => true,
            Self::List(kind) => !matches!(row_identity(kind), RowIdentity::Opaque),
        }
    }

    /// The picker's built-in wording for a jump onto this target.
    ///
    /// A detail target reads `"Disks of web-01"` — the collection, then the
    /// resource it belongs to. A list target has no collection to name, so it
    /// reads `"VPC network default"`: the noun and the row, which is what the
    /// user is about to be looking at.
    pub fn default_label(self, name: &str) -> String {
        match self {
            Self::Detail(tab) => format!("{} of {name}", tab.label()),
            // Not "<table> of <row>": a list target does not land on a
            // collection *belonging to* the row, it lands on the row itself.
            // Wording it the other way would promise a filtered list, which is
            // the one thing this target deliberately is not.
            Self::List(kind) => format!("{} {name}", kind.selection_noun()),
        }
    }
}

/// A nested Cloud Run revision sub-location captured within a service detail.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CloudRunRevisionRef {
    pub name: String,
    pub region: String,
    pub tab: CloudRunRevisionTab,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceDetailLocation {
    pub key: ResourceKey,
    pub tab: DetailTab,
    /// Present when the captured view had drilled into a Cloud Run revision.
    pub revision: Option<CloudRunRevisionRef>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceLocation {
    pub service: ResourceServiceKind,
    pub detail: Option<ResourceDetailLocation>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NavLocation {
    Home,
    Configurations,
    Resources(ResourceLocation),
    Logging,
    /// The tunnel list. It carries no sub-location: the workspace has a single
    /// view, and the selected row is not worth restoring since the list is a
    /// live process table that may have changed underneath.
    Tunnels,
    /// The FinOps workspace. Carries no sub-location for the same reason
    /// [`NavLocation::Tunnels`] does not: the workspace has a single view, and
    /// its content is re-probed on entry rather than restored.
    FinOps,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NavigationState {
    back: Vec<NavLocation>,
    forward: Vec<NavLocation>,
}

impl NavigationState {
    pub fn record(&mut self, previous: NavLocation) {
        self.back.push(previous);
        self.forward.clear();
    }

    pub fn go_back(&mut self, current: NavLocation) -> Option<NavLocation> {
        let target = self.back.pop()?;
        self.forward.push(current);
        Some(target)
    }

    pub fn go_forward(&mut self, current: NavLocation) -> Option<NavLocation> {
        let target = self.forward.pop()?;
        self.back.push(current);
        Some(target)
    }

    pub fn can_go_back(&self) -> bool {
        !self.back.is_empty()
    }

    pub fn can_go_forward(&self) -> bool {
        !self.forward.is_empty()
    }

    pub fn reset(&mut self) {
        self.back.clear();
        self.forward.clear();
    }
}

impl NavLocation {
    pub fn capture(root: &RootState) -> NavLocation {
        match root.workspace {
            Workspace::Home => NavLocation::Home,
            Workspace::Configurations => NavLocation::Configurations,
            Workspace::Resources => {
                let service = root.resources.active_service();
                let detail = capture_detail(root, service);
                NavLocation::Resources(ResourceLocation { service, detail })
            }
            Workspace::Logging => NavLocation::Logging,
            Workspace::Tunnels => NavLocation::Tunnels,
            Workspace::FinOps => NavLocation::FinOps,
        }
    }
}

fn capture_detail(
    root: &RootState,
    service: ResourceServiceKind,
) -> Option<ResourceDetailLocation> {
    if !root.resources.is_showing_detail() {
        return None;
    }
    match service {
        ResourceServiceKind::ComputeInstances => {
            let instance = root.resources.detail_target()?;
            Some(ResourceDetailLocation {
                key: ResourceKey::Compute {
                    name: instance.name.clone(),
                    zone: instance.zone.clone(),
                },
                tab: DetailTab::Compute(root.resources.active_detail_tab()),
                revision: None,
            })
        }
        ResourceServiceKind::GkeClusters => {
            let cluster = root.resources.gke_detail_target()?;
            Some(ResourceDetailLocation {
                key: ResourceKey::Gke {
                    name: cluster.name.clone(),
                    location: cluster.location.clone(),
                },
                tab: DetailTab::Gke(root.resources.active_gke_detail_tab()),
                revision: None,
            })
        }
        ResourceServiceKind::CloudRunServices => {
            let service = root.resources.cloud_run_detail_target()?;
            let key = ResourceKey::CloudRun {
                name: service.name.clone(),
                region: service.region.clone(),
            };
            let revision = if root.resources.is_showing_cloud_run_revision() {
                let target = root.resources.cloud_run_revision_target()?;
                Some(CloudRunRevisionRef {
                    name: target.name.clone(),
                    region: root
                        .resources
                        .cloud_run_revision_region()
                        .unwrap_or(&service.region)
                        .to_string(),
                    tab: root.resources.active_cloud_run_revision_tab(),
                })
            } else {
                None
            };
            Some(ResourceDetailLocation {
                key,
                tab: DetailTab::CloudRun(root.resources.active_cloud_run_detail_tab()),
                revision,
            })
        }
        ResourceServiceKind::CloudSql => {
            let instance = root.resources.cloud_sql_detail_target()?;
            Some(ResourceDetailLocation {
                key: ResourceKey::CloudSql {
                    name: instance.name.clone(),
                    region: instance.region.clone(),
                },
                tab: DetailTab::CloudSql(root.resources.active_cloud_sql_detail_tab()),
                revision: None,
            })
        }
        // Cloud Storage detail views are bespoke and not yet restored through
        // navigation history; treat them as list-level for back/forward.
        // Cloud Storage and Secret Manager have bespoke detail views with no
        // `DetailTab`, so neither can be named as a jump destination.
        // Cloud Functions joins them for a different reason: it has no detail
        // view at all in this slice.
        // Pub/Sub topics join Cloud Storage and Secret Manager: the pane
        // exists and paints, but it has no `DetailTab` spelling, so it cannot
        // be named as a jump destination or restored by back/forward. That is
        // a gap in navigation history, not in the pane.
        ResourceServiceKind::CloudStorage
        | ResourceServiceKind::SecretManagerSecrets
        | ResourceServiceKind::CloudFunctions
        | ResourceServiceKind::PersistentDisks => None,
        ResourceServiceKind::PubSubTopics => None,
        ResourceServiceKind::VpcNetworks => None,
        ResourceServiceKind::VpcSubnets => None,
        ResourceServiceKind::FirewallRules => None,
        ResourceServiceKind::PubSubSubscriptions => None,
        // BigQuery datasets are list-only in this slice, so there is no pane
        // to capture at all.
        ResourceServiceKind::BigQueryDatasets => None,
        ResourceServiceKind::IamServiceAccounts => None,
        ResourceServiceKind::IamPolicy => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The service half of a key is a service name like any other, so it
    /// answers to the whole service vocabulary — the one a `[[jumps]]` rule's
    /// `from` already accepts. Matching the key whole against `config_key`
    /// made `"gce:disks"` name no tab while `from = "gce"` named a service,
    /// which is a vocabulary the document could not honestly describe.
    #[test]
    fn a_key_resolves_every_spelling_of_its_service() {
        let disks = DetailTab::from_config_key("compute:disks");
        assert!(disks.is_some(), "the canonical spelling must resolve");
        assert_eq!(DetailTab::from_config_key("gce:disks"), disks);
        assert_eq!(DetailTab::from_config_key("instances:disks"), disks);
        assert_eq!(
            DetailTab::from_config_key("run:revisions"),
            DetailTab::from_config_key("cloudrun:revisions")
        );
    }

    /// Leniency is on the service half only: a known service does not make an
    /// unknown tab resolve, or a typo would open whichever tab sorted first.
    #[test]
    fn a_lenient_service_does_not_make_an_unknown_tab_resolve() {
        assert_eq!(DetailTab::from_config_key("gce:zzz"), None);
        assert_eq!(DetailTab::from_config_key("gce"), None);
        // A compute tab named under GKE is still a typo: tabs belong to one
        // service each.
        assert_eq!(DetailTab::from_config_key("gke:disks"), None);
    }

    /// The config keys, spelled out.
    ///
    /// These strings are a user-facing, *persisted* vocabulary: they are what
    /// people write in `to = "..."` in their own TOML files. They are derived
    /// from [`DetailTab::label`], which is otherwise cosmetic tab-bar wording,
    /// so renaming a label is a breaking config change — renaming `NodePools`
    /// to "Node Groups" would turn `gke:node-pools` into `gke:node-groups` and
    /// every user carrying the old value would lose that jump to an "unknown
    /// destination tab" warning. Listing them here as literals is what makes
    /// that a failing test instead of a silent break in the field. Changing an
    /// entry below is allowed, but it is a deliberate migration, not a rename.
    #[test]
    fn every_tabs_config_key_is_the_one_users_have_written_down() {
        let keys: Vec<String> = DetailTab::ALL.iter().map(|tab| tab.config_key()).collect();
        assert_eq!(
            keys,
            vec![
                "compute:summary",
                "compute:network",
                "compute:disks",
                "compute:metadata",
                "compute:json",
                "gke:summary",
                "gke:networking",
                "gke:node-pools",
                "gke:addons",
                "gke:json",
                "cloudsql:summary",
                "cloudsql:connectivity",
                "cloudsql:maintenance",
                "cloudsql:backups",
                "cloudsql:replicas",
                "cloudsql:users",
                "cloudsql:json",
                "cloudrun:summary",
                "cloudrun:traffic",
                "cloudrun:revisions",
                "cloudrun:configuration",
                "cloudrun:conditions",
                "cloudrun:json",
            ]
        );
    }

    #[test]
    fn every_tab_round_trips_through_its_config_key() {
        for tab in DetailTab::ALL {
            let key = tab.config_key();
            assert_eq!(
                DetailTab::from_config_key(&key),
                Some(*tab),
                "{key} did not round-trip"
            );
        }
    }
}
