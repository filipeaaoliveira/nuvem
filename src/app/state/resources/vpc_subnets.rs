//! VPC subnets in application state (slice 2).
//!
//! # Regional, and that is the whole difference from the networks table
//!
//! A subnet's identity is **(name, region)**, not the name alone. An auto-mode
//! network gets one subnet per region and calls every one of them `default`,
//! so the captured project returns 43 rows sharing a single name. Keying on
//! the name would collapse them into one row for the cursor, for marks, and
//! for the remembered view.
//!
//! That is why this file exists rather than a second marker type inside
//! `vpc.rs`: the networks table one slice earlier is global and takes the
//! Pub/Sub topics shape, and this one takes the Cloud Functions shape, where
//! the location qualifies the name.
//!
//! # Flat across every region
//!
//! `gcloud compute networks subnets list` is one project-wide call, so this
//! table shows every region at once and sorts across them. `REGION` is a
//! column an operator sorts by rather than a scope they enter, which is the
//! reading the flat listing supports and a per-region drill-down would not.
//!
//! # List-only
//!
//! No detail pane: `Detail` and `DetailTab` are `()`. Everything a subnet
//! describe would return is already on the row, and the two fields with no
//! column of their own — the purpose and the stack type — are in the preview.

use super::*;

/// Marker type unifying the VPC subnets service behind [`ResourceService`].
/// Never instantiated.
pub enum VpcSubnets {}

impl ResourceService for VpcSubnets {
    type Row = VpcSubnet;
    type Detail = ();
    type SortField = VpcSubnetSortField;
    type DetailTab = ();

    const KIND: ResourceServiceKind = ResourceServiceKind::VpcSubnets;

    fn collection(
        state: &ResourcesState,
    ) -> &ResourceCollectionState<Self::Row, Self::Detail, Self::SortField, Self::DetailTab> {
        &state.vpc_subnets
    }

    fn collection_mut(
        state: &mut ResourcesState,
    ) -> &mut ResourceCollectionState<Self::Row, Self::Detail, Self::SortField, Self::DetailTab>
    {
        &mut state.vpc_subnets
    }

    fn matches_filter(row: &Self::Row, filter: &CompiledFilter) -> bool {
        subnet_matches_filter(row, filter)
    }

    /// The needle sits in the name. The parent network deliberately does not
    /// carry it: a subnet matched through its network's name would leave a
    /// `matches_filter` reading the wrong field looking correct here.
    fn sample_row() -> Self::Row {
        VpcSubnet {
            name: SAMPLE_ROW_NEEDLE.to_string(),
            region: "europe-west1".to_string(),
            network: "default".to_string(),
            ip_cidr_range: "10.0.0.0/24".to_string(),
            gateway_address: Some("10.0.0.1".to_string()),
            private_google_access: true,
        }
    }

    fn compare(
        left: &Self::Row,
        right: &Self::Row,
        sort_field: Self::SortField,
        direction: SortDirection,
    ) -> std::cmp::Ordering {
        compare_subnets(left, right, sort_field, direction)
    }

    /// The region qualifies the name, because the name does not identify a
    /// subnet. Three rows called `default` in three regions are three
    /// resources, and an identity that could not tell them apart would move
    /// the cursor to the wrong one on every reload.
    fn row_id(row: &Self::Row) -> ResourceRowId {
        ResourceRowId {
            name: row.name.clone(),
            location: row.region.clone(),
        }
    }
}

/// The sortable columns of the subnets table.
///
/// One variant per catalogue column, and every column sorts: like the networks
/// table, this one carries no labels cell, because the `compute/v1`
/// subnetworks resource has no `labels` field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VpcSubnetSortField {
    Name,
    Region,
    Network,
    Range,
    Gateway,
    PrivateGoogleAccess,
}

impl VpcSubnetSortField {
    pub fn label(self) -> &'static str {
        match self {
            Self::Name => "Name",
            Self::Region => "Region",
            Self::Network => "Network",
            Self::Range => "Range",
            Self::Gateway => "Gateway",
            Self::PrivateGoogleAccess => "Private Google access",
        }
    }

    pub const COLUMN_COUNT: usize = 6;

    pub fn from_column(index: usize) -> Option<Self> {
        match index {
            0 => Some(Self::Name),
            1 => Some(Self::Region),
            2 => Some(Self::Network),
            3 => Some(Self::Range),
            4 => Some(Self::Gateway),
            5 => Some(Self::PrivateGoogleAccess),
            _ => None,
        }
    }

    pub fn column(self) -> usize {
        match self {
            Self::Name => 0,
            Self::Region => 1,
            Self::Network => 2,
            Self::Range => 3,
            Self::Gateway => 4,
            Self::PrivateGoogleAccess => 5,
        }
    }
}

/// What the `PRIVATE GOOGLE ACCESS` cell paints.
///
/// Words rather than a bool's `true`/`false`, and beside the state rather than
/// in the projection, so the cell and the filter cannot disagree about the
/// spelling an operator types.
pub fn private_google_access_label(subnet: &VpcSubnet) -> &'static str {
    if subnet.private_google_access {
        "On"
    } else {
        "Off"
    }
}

impl ResourcesState {
    pub fn vpc_subnets(&self) -> &[VpcSubnet] {
        &self.vpc_subnets.rows
    }

    pub fn visible_vpc_subnets(&self) -> Vec<&VpcSubnet> {
        let filter = self.vpc_subnets.filter_applied.filter();
        let mut rows = self
            .vpc_subnets
            .rows
            .iter()
            .filter(|subnet| subnet_matches_filter(subnet, filter))
            .collect::<Vec<_>>();
        rows.sort_by(|left, right| {
            compare_subnets(
                left,
                right,
                self.vpc_subnets.sort_field,
                self.vpc_subnets.sort_direction,
            )
        });
        rows
    }

    pub fn visible_vpc_subnet_count(&self) -> usize {
        let filter = self.vpc_subnets.filter_applied.filter();
        self.vpc_subnets
            .rows
            .iter()
            .filter(|subnet| subnet_matches_filter(subnet, filter))
            .count()
    }

    pub fn selected_vpc_subnet(&self) -> Option<&VpcSubnet> {
        self.visible_vpc_subnets()
            .get(self.vpc_subnets.selected_visible)
            .copied()
    }

    /// Installs a freshly loaded list, keeping the cursor on the subnet it was
    /// already on where that subnet is still listed.
    ///
    /// The match is on **(name, region)**, not on the name: with 43 subnets
    /// called `default`, a name match would put the cursor on whichever region
    /// happened to sort first.
    pub fn set_vpc_subnets(&mut self, context: GcloudContext, subnets: Vec<VpcSubnet>) {
        let previously_selected = self
            .selected_vpc_subnet()
            .map(|subnet| (subnet.name.clone(), subnet.region.clone()));

        self.vpc_subnets.context = Some(context);
        self.vpc_subnets.rows = subnets;
        self.vpc_subnets.stale = false;
        self.vpc_subnets.last_problem = None;
        self.vpc_subnets.last_diagnostic = None;
        self.vpc_subnets.clear_load_phase();

        self.vpc_subnets.selected_visible = previously_selected
            .as_ref()
            .and_then(|(name, region)| {
                self.visible_vpc_subnets()
                    .iter()
                    .position(|subnet| &subnet.name == name && &subnet.region == region)
            })
            .unwrap_or_else(|| {
                self.vpc_subnets
                    .selected_visible
                    .min(self.visible_vpc_subnets().len().saturating_sub(1))
            });
    }

    pub fn mark_vpc_subnets_stale_and_clear(&mut self) {
        self.vpc_subnets.filter_draft = self.vpc_subnets.filter_applied.source().to_string();
        self.vpc_subnets.filter_input_active = false;
        self.vpc_subnets.rows.clear();
        self.vpc_subnets.context = None;
        self.vpc_subnets.selected_visible = 0;
        self.vpc_subnets.stale = true;
    }

    pub fn vpc_subnets_problem(&self) -> Option<&DiagnosticProblem> {
        self.vpc_subnets.last_problem.as_ref()
    }
}

/// The values a subnets filter searches.
///
/// Every cell the table paints, because `/` narrows what is on screen and a
/// term an operator can read in a column but cannot filter by is the filter
/// lying about the rows it hid.
///
/// `REGION` and `NETWORK` contribute their *ids*, which is what the cells show
/// — searching the self-link URLs they arrived as would match every row of the
/// table on `projects` or `compute`. `PRIVATE GOOGLE ACCESS` contributes its
/// rendered word, because `true` appears in no cell.
///
/// The premise holds trivially in any case: this service pushes no clause down
/// at all, so every value here is matched client-side against a row the server
/// never narrowed.
fn subnet_searchable_fields(subnet: &VpcSubnet) -> Vec<String> {
    vec![
        subnet.name.clone(),
        subnet.region.clone(),
        subnet.network.clone(),
        subnet.ip_cidr_range.clone(),
        subnet.gateway_address.clone().unwrap_or_default(),
        private_google_access_label(subnet).to_string(),
    ]
}

fn subnet_matches_filter(subnet: &VpcSubnet, filter: &CompiledFilter) -> bool {
    filter.matches(&subnet_searchable_fields(subnet))
}

fn compare_subnets(
    left: &VpcSubnet,
    right: &VpcSubnet,
    sort_field: VpcSubnetSortField,
    direction: SortDirection,
) -> std::cmp::Ordering {
    let primary = match sort_field {
        VpcSubnetSortField::Name => left.name.cmp(&right.name),
        VpcSubnetSortField::Region => left.region.cmp(&right.region),
        VpcSubnetSortField::Network => left.network.cmp(&right.network),
        // Lexicographically on the CIDR text, which is what the cell shows.
        // Numeric ordering of address space would be a different and arguably
        // better answer, and it is deliberately not attempted here: it would
        // sort by a value no column paints, so an operator could not predict
        // the order from the screen.
        VpcSubnetSortField::Range => left.ip_cidr_range.cmp(&right.ip_cidr_range),
        // An absent gateway sorts as the empty string, grouping the rows that
        // have none rather than interleaving them.
        VpcSubnetSortField::Gateway => left
            .gateway_address
            .as_deref()
            .unwrap_or_default()
            .cmp(right.gateway_address.as_deref().unwrap_or_default()),
        VpcSubnetSortField::PrivateGoogleAccess => {
            private_google_access_label(left).cmp(private_google_access_label(right))
        }
    };
    // Name *and region* break every tie, because the name alone does not: 43
    // subnets called `default` would otherwise have no defined order among
    // them, and the cursor would move under a reload.
    let ordering = primary
        .then_with(|| left.name.cmp(&right.name))
        .then_with(|| left.region.cmp(&right.region));
    match direction {
        SortDirection::Ascending => ordering,
        SortDirection::Descending => ordering.reverse(),
    }
}
