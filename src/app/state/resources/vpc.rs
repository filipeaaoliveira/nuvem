//! VPC networking in application state.
//!
//! # A flat table, not a tree
//!
//! A network *looks* like the parent of its subnets, and is not modelled as
//! one. `gcloud compute networks subnets list` is a flat project-wide call, so
//! nesting subnets under networks would be a display choice with no call
//! behind it — and it would cost both of them the thing the ticket actually
//! asks for, which is that an instance's detail can jump *into* its network
//! and *into* its subnet. A drilled level is not addressable as a jump target.
//!
//! The `SUBNETS` column is therefore a count and not a list: the captured
//! `default` network carries 43 subnet self-links, which is not a cell, and
//! the subnets themselves are a table of their own (slice 2).
//!
//! # List-only
//!
//! No detail pane: `Detail` and `DetailTab` are `()`, so `Enter` on a row does
//! nothing at all. That is the shape Pub/Sub topics shipped in and stayed in,
//! and it is deliberate rather than unfinished — `compute networks describe`
//! returns the same fields the list call already returns, so a pane would
//! repeat the row it was opened from. What a network's description says is in
//! the preview pane, which is where the one unbounded field belongs.
//!
//! # No lifecycle actions, and no status counts
//!
//! Nothing here starts, stops or deletes: this is read-only browsing, and a
//! network has no lifecycle state to count on Home either. `MODE` is a
//! configuration fact, not a health one, so the inventory card reports a row
//! count and no status breakdown — the topics shape, not the instances one.

use super::*;

/// Marker type unifying the VPC networks service behind [`ResourceService`].
/// Never instantiated.
pub enum VpcNetworks {}

impl ResourceService for VpcNetworks {
    type Row = VpcNetwork;
    type Detail = ();
    type SortField = VpcNetworkSortField;
    type DetailTab = ();

    const KIND: ResourceServiceKind = ResourceServiceKind::VpcNetworks;

    fn collection(
        state: &ResourcesState,
    ) -> &ResourceCollectionState<Self::Row, Self::Detail, Self::SortField, Self::DetailTab> {
        &state.vpc_networks
    }

    fn collection_mut(
        state: &mut ResourcesState,
    ) -> &mut ResourceCollectionState<Self::Row, Self::Detail, Self::SortField, Self::DetailTab>
    {
        &mut state.vpc_networks
    }

    fn matches_filter(row: &Self::Row, filter: &CompiledFilter) -> bool {
        network_matches_filter(row, filter)
    }

    /// The needle sits in the name, the identity column of the network table.
    /// This is the service whose `matches_filter` was caught surviving
    /// mutation as both `true` and `false`, for want of exactly this
    /// constructor.
    fn sample_row() -> Self::Row {
        VpcNetwork {
            name: SAMPLE_ROW_NEEDLE.to_string(),
            subnet_mode: VpcSubnetMode::Custom,
            routing_mode: VpcRoutingMode::Regional,
            subnet_count: 2,
            description: None,
        }
    }

    fn compare(
        left: &Self::Row,
        right: &Self::Row,
        sort_field: Self::SortField,
        direction: SortDirection,
    ) -> std::cmp::Ordering {
        compare_networks(left, right, sort_field, direction)
    }

    /// A VPC network is a *global* resource — that is what distinguishes it
    /// from its own subnets, which are regional — so the name alone is the
    /// identity and there is no location to qualify it with. The Pub/Sub
    /// topics shape, not the Cloud Functions one.
    fn row_id(row: &Self::Row) -> ResourceRowId {
        ResourceRowId {
            name: row.name.clone(),
            location: String::new(),
        }
    }
}

/// The sortable columns of the networks table.
///
/// One variant per catalogue column, and here every column sorts: this table
/// carries no labels cell, because the `compute/v1` networks resource has no
/// `labels` field at all. That is the one place it differs from the topics
/// table it is otherwise modelled on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VpcNetworkSortField {
    Name,
    Mode,
    Routing,
    Subnets,
}

impl VpcNetworkSortField {
    pub fn label(self) -> &'static str {
        match self {
            Self::Name => "Name",
            Self::Mode => "Mode",
            Self::Routing => "Routing",
            Self::Subnets => "Subnets",
        }
    }

    pub const COLUMN_COUNT: usize = 4;

    pub fn from_column(index: usize) -> Option<Self> {
        match index {
            0 => Some(Self::Name),
            1 => Some(Self::Mode),
            2 => Some(Self::Routing),
            3 => Some(Self::Subnets),
            _ => None,
        }
    }

    pub fn column(self) -> usize {
        match self {
            Self::Name => 0,
            Self::Mode => 1,
            Self::Routing => 2,
            Self::Subnets => 3,
        }
    }
}

impl ResourcesState {
    pub fn vpc_networks(&self) -> &[VpcNetwork] {
        &self.vpc_networks.rows
    }

    pub fn visible_vpc_networks(&self) -> Vec<&VpcNetwork> {
        let filter = self.vpc_networks.filter_applied.filter();
        let mut rows = self
            .vpc_networks
            .rows
            .iter()
            .filter(|network| network_matches_filter(network, filter))
            .collect::<Vec<_>>();
        rows.sort_by(|left, right| {
            compare_networks(
                left,
                right,
                self.vpc_networks.sort_field,
                self.vpc_networks.sort_direction,
            )
        });
        rows
    }

    pub fn visible_vpc_network_count(&self) -> usize {
        let filter = self.vpc_networks.filter_applied.filter();
        self.vpc_networks
            .rows
            .iter()
            .filter(|network| network_matches_filter(network, filter))
            .count()
    }

    pub fn selected_vpc_network(&self) -> Option<&VpcNetwork> {
        self.visible_vpc_networks()
            .get(self.vpc_networks.selected_visible)
            .copied()
    }

    /// Installs a freshly loaded list, keeping the cursor on the network it
    /// was already on where that network is still listed.
    pub fn set_vpc_networks(&mut self, context: GcloudContext, networks: Vec<VpcNetwork>) {
        let previously_selected = self
            .selected_vpc_network()
            .map(|network| network.name.clone());

        self.vpc_networks.context = Some(context);
        self.vpc_networks.rows = networks;
        self.vpc_networks.stale = false;
        self.vpc_networks.last_problem = None;
        self.vpc_networks.last_diagnostic = None;
        self.vpc_networks.clear_load_phase();

        self.vpc_networks.selected_visible = previously_selected
            .as_ref()
            .and_then(|name| {
                self.visible_vpc_networks()
                    .iter()
                    .position(|network| &network.name == name)
            })
            .unwrap_or_else(|| {
                self.vpc_networks
                    .selected_visible
                    .min(self.visible_vpc_networks().len().saturating_sub(1))
            });
    }

    pub fn mark_vpc_networks_stale_and_clear(&mut self) {
        self.vpc_networks.filter_draft = self.vpc_networks.filter_applied.source().to_string();
        self.vpc_networks.filter_input_active = false;
        self.vpc_networks.rows.clear();
        self.vpc_networks.context = None;
        self.vpc_networks.selected_visible = 0;
        self.vpc_networks.stale = true;
    }

    pub fn vpc_networks_problem(&self) -> Option<&DiagnosticProblem> {
        self.vpc_networks.last_problem.as_ref()
    }
}

/// The values a networks filter searches.
///
/// Every cell the table paints, because `/` narrows what is on screen and a
/// term an operator can read in a column but cannot filter by is the filter
/// lying about the rows it hid. `MODE` and `ROUTING` contribute their rendered
/// labels — `AUTO`, `REGIONAL` — because those are the words in the cells; the
/// booleans and nested strings they are derived from appear nowhere on screen.
///
/// The subnet count contributes its digits, so `/^0$` finds the networks with
/// no subnets at all.
///
/// The description is searchable and has no column of its own: it is the one
/// field that says *why* a network exists, and it is unbounded free text that
/// would push the four facts a network is identified by off a narrow terminal.
/// Searchable-without-a-column is the safe direction of the never-drop-a-row
/// invariant — it can only widen what matches, never hide a row that is on
/// screen.
///
/// That premise holds trivially here in any case, because this service pushes
/// no clause down at all (see `crate::services::resources::vpc`), so every one
/// of these values is matched client-side against a row the server never
/// narrowed.
fn network_searchable_fields(network: &VpcNetwork) -> Vec<String> {
    vec![
        network.name.clone(),
        network.subnet_mode.label().to_string(),
        network.routing_mode.label().to_string(),
        network.subnet_count.to_string(),
        network.description.clone().unwrap_or_default(),
    ]
}

fn network_matches_filter(network: &VpcNetwork, filter: &CompiledFilter) -> bool {
    filter.matches(&network_searchable_fields(network))
}

fn compare_networks(
    left: &VpcNetwork,
    right: &VpcNetwork,
    sort_field: VpcNetworkSortField,
    direction: SortDirection,
) -> std::cmp::Ordering {
    let primary = match sort_field {
        VpcNetworkSortField::Name => left.name.cmp(&right.name),
        // On the rendered labels, which is what the cells say and what an
        // operator sorting this column is grouping by.
        VpcNetworkSortField::Mode => left.subnet_mode.label().cmp(right.subnet_mode.label()),
        VpcNetworkSortField::Routing => left.routing_mode.label().cmp(&right.routing_mode.label()),
        // Numerically, not by the rendered digits: `10` sorts after `9` in the
        // column an operator sorted to find the widest network, which text
        // ordering gets backwards.
        VpcNetworkSortField::Subnets => left.subnet_count.cmp(&right.subnet_count),
    };
    // The name breaks every tie and is unique within the project, so this is
    // a total order over the row set.
    let ordering = primary.then_with(|| left.name.cmp(&right.name));
    match direction {
        SortDirection::Ascending => ordering,
        SortDirection::Descending => ordering.reverse(),
    }
}
