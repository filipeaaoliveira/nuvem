use super::*;

/// Marker type unifying the GKE clusters service behind
/// [`ResourceService`]. Never instantiated.
pub enum Gke {}

impl ResourceService for Gke {
    type Row = GkeCluster;
    type Detail = GkeClusterDetail;
    type SortField = GkeSortField;
    type DetailTab = GkeDetailTab;

    const KIND: ResourceServiceKind = ResourceServiceKind::GkeClusters;

    fn collection(
        state: &ResourcesState,
    ) -> &ResourceCollectionState<Self::Row, Self::Detail, Self::SortField, Self::DetailTab> {
        &state.gke
    }

    fn collection_mut(
        state: &mut ResourcesState,
    ) -> &mut ResourceCollectionState<Self::Row, Self::Detail, Self::SortField, Self::DetailTab>
    {
        &mut state.gke
    }

    fn matches_filter(row: &Self::Row, filter: &CompiledFilter) -> bool {
        gke_cluster_matches_filter(row, filter)
    }

    /// The needle sits in the name, the identity column of the cluster table.
    fn sample_row() -> Self::Row {
        GkeCluster {
            name: SAMPLE_ROW_NEEDLE.to_string(),
            location: "europe-west1".to_string(),
            status: GkeClusterStatus::Running,
            current_master_version: Some("1.30".to_string()),
            current_node_count: Some(3),
            endpoint_visibility: EndpointVisibility::Public,
        }
    }

    fn compare(
        left: &Self::Row,
        right: &Self::Row,
        sort_field: Self::SortField,
        direction: SortDirection,
    ) -> std::cmp::Ordering {
        compare_gke_clusters(left, right, sort_field, direction)
    }

    /// GKE has no lifecycle actions and so no marks; this exists only to keep
    /// [`ResourceService`] total.
    fn row_id(row: &Self::Row) -> ResourceRowId {
        ResourceRowId {
            name: row.name.clone(),
            location: row.location.clone(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GkeSortField {
    Name,
    Status,
    Location,
    Version,
    Nodes,
}

impl GkeSortField {
    pub fn label(self) -> &'static str {
        match self {
            Self::Name => "Name",
            Self::Status => "Status",
            Self::Location => "Location",
            Self::Version => "Version",
            Self::Nodes => "Nodes",
        }
    }

    pub const COLUMN_COUNT: usize = 6;

    pub fn from_column(index: usize) -> Option<Self> {
        match index {
            0 => Some(Self::Name),
            1 => Some(Self::Status),
            2 => Some(Self::Location),
            3 => Some(Self::Version),
            4 => Some(Self::Nodes),
            _ => None,
        }
    }

    pub fn column(self) -> usize {
        match self {
            Self::Name => 0,
            Self::Status => 1,
            Self::Location => 2,
            Self::Version => 3,
            Self::Nodes => 4,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GkeDetailTab {
    Summary,
    Networking,
    NodePools,
    Addons,
    Json,
}

impl GkeDetailTab {
    pub const ALL: [Self; 5] = [
        Self::Summary,
        Self::Networking,
        Self::NodePools,
        Self::Addons,
        Self::Json,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Summary => "Summary",
            Self::Networking => "Networking",
            Self::NodePools => "Node Pools",
            Self::Addons => "Addons",
            Self::Json => "JSON",
        }
    }
}

/// Confirmation modal for the "Open in k9s" workflow. Carries the resolved
/// launch target and whether the kubeconfig context will be reused or created
/// via get-credentials, so the preview can spell out both argv sequences.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct K9sLaunchConfirmation {
    target: K9sLaunchTarget,
    credential_step: CredentialStep,
}

impl K9sLaunchConfirmation {
    pub fn target(&self) -> &K9sLaunchTarget {
        &self.target
    }
    pub fn credential_step(&self) -> CredentialStep {
        self.credential_step
    }
}

impl ResourcesState {
    pub fn gke_clusters(&self) -> &[GkeCluster] {
        &self.gke.rows
    }

    pub fn visible_gke_clusters(&self) -> Vec<&GkeCluster> {
        let filter = self.gke.filter_applied.filter();
        let mut rows = self
            .gke
            .rows
            .iter()
            .filter(|cluster| gke_cluster_matches_filter(cluster, filter))
            .collect::<Vec<_>>();
        rows.sort_by(|left, right| {
            compare_gke_clusters(left, right, self.gke.sort_field, self.gke.sort_direction)
        });
        rows
    }

    pub fn visible_gke_cluster_count(&self) -> usize {
        let filter = self.gke.filter_applied.filter();
        self.gke
            .rows
            .iter()
            .filter(|cluster| gke_cluster_matches_filter(cluster, filter))
            .count()
    }

    pub fn selected_gke_cluster(&self) -> Option<&GkeCluster> {
        self.visible_gke_clusters()
            .get(self.gke.selected_visible)
            .copied()
    }

    pub fn set_gke_clusters(
        &mut self,
        context: GcloudContext,
        clusters: Vec<GkeCluster>,
    ) -> Option<String> {
        if self.gke.context.as_ref() != Some(&context) {
            self.command_preview = None;
            self.ssh_launch_preview = None;
            self.tunnel_launch_preview = None;
            self.k9s_launch_confirmation = None;
        }
        let prev_selected = self
            .selected_gke_cluster()
            .map(|c| (c.name.clone(), c.location.clone()));
        let detail_open = self.gke.view_mode == ResourcesViewMode::Detail;
        let detail_key = self
            .gke
            .detail_target
            .as_ref()
            .map(|c| (c.name.clone(), c.location.clone()));

        self.gke.context = Some(context);
        self.gke.rows = clusters;
        self.gke.stale = false;
        self.gke.last_problem = None;
        self.gke.last_diagnostic = None;
        self.gke.clear_load_phase();

        let mut dropped_detail = None;
        if detail_open {
            let still_present = detail_key
                .as_ref()
                .map(|(n, l)| {
                    self.gke
                        .rows
                        .iter()
                        .any(|c| &c.name == n && &c.location == l)
                })
                .unwrap_or(false);
            if !still_present {
                dropped_detail = detail_key.map(|(name, _)| name);
                self.close_gke_detail();
            }
        } else {
            self.close_gke_detail();
        }

        self.gke.selected_visible = prev_selected
            .as_ref()
            .and_then(|(n, l)| {
                self.visible_gke_clusters()
                    .iter()
                    .position(|c| &c.name == n && &c.location == l)
            })
            .unwrap_or_else(|| {
                self.gke
                    .selected_visible
                    .min(self.visible_gke_clusters().len().saturating_sub(1))
            });

        dropped_detail
    }

    pub fn mark_gke_stale_and_clear(&mut self) {
        self.gke.filter_draft = self.gke.filter_applied.source().to_string();
        self.gke.filter_input_active = false;
        self.gke.rows.clear();
        self.gke.context = None;
        self.gke.selected_visible = 0;
        self.gke.stale = true;
        self.command_preview = None;
        self.ssh_launch_preview = None;
        self.tunnel_launch_preview = None;
        self.k9s_launch_confirmation = None;
        self.close_gke_detail();
    }

    pub fn gke_loaded(&self) -> bool {
        self.gke.context.is_some()
    }

    pub fn gke_stale(&self) -> bool {
        self.gke.stale
    }

    pub fn gke_problem(&self) -> Option<&DiagnosticProblem> {
        self.gke.last_problem.as_ref()
    }

    pub fn gke_status_counts(&self) -> Vec<(String, usize)> {
        count_labels(self.gke.rows.iter().map(|c| gke_status_label(&c.status)))
    }

    pub fn k9s_launch_confirmation(&self) -> Option<&K9sLaunchConfirmation> {
        self.k9s_launch_confirmation.as_ref()
    }

    pub fn open_k9s_launch_confirmation(
        &mut self,
        target: K9sLaunchTarget,
        credential_step: CredentialStep,
    ) {
        self.close_other_modals();
        self.k9s_launch_confirmation = Some(K9sLaunchConfirmation {
            target,
            credential_step,
        });
    }

    pub fn cancel_k9s_launch_confirmation(&mut self) {
        self.k9s_launch_confirmation = None;
    }

    pub fn k9s_launch_submission(&mut self) -> Option<K9sLaunchPlan> {
        self.k9s_launch_confirmation
            .take()
            .map(|confirmation| K9sLaunchPlan {
                target: confirmation.target,
                credential_step: confirmation.credential_step,
            })
    }

    pub fn open_gke_detail(&mut self) -> Option<GkeCluster> {
        let cluster = self.selected_gke_cluster()?.clone();
        self.command_preview = None;
        self.ssh_launch_preview = None;
        self.tunnel_launch_preview = None;
        self.k9s_launch_confirmation = None;
        self.gke.view_mode = ResourcesViewMode::Detail;
        self.gke.active_detail_tab = GkeDetailTab::Summary;
        self.gke.detail_scroll = 0;
        self.detail_field_index = None;
        self.gke.detail_target = Some(cluster.clone());
        self.gke.detail_error_target_name = None;
        self.gke.detail = None;
        self.gke.detail_error = None;
        Some(cluster)
    }

    /// Returns the active GKE detail tab without consulting compute state.
    pub fn active_gke_detail_tab(&self) -> GkeDetailTab {
        self.gke.active_detail_tab
    }

    pub fn gke_detail(&self) -> Option<&GkeClusterDetail> {
        self.gke.detail.as_ref()
    }

    pub fn set_gke_detail(&mut self, detail: GkeClusterDetail) {
        self.gke.detail_error_target_name = None;
        self.gke.detail_error = None;
        self.gke.detail = Some(detail);
    }

    pub fn set_gke_detail_error(&mut self, cluster_name: String, error: String) {
        self.gke.view_mode = ResourcesViewMode::Detail;
        self.gke.detail = None;
        self.gke.detail_error_target_name = Some(cluster_name);
        self.gke.detail_error = Some(error);
    }

    pub fn gke_detail_target_name(&self) -> Option<&str> {
        self.gke
            .detail_target
            .as_ref()
            .map(|cluster| cluster.name.as_str())
            .or_else(|| self.gke.detail.as_ref().map(|detail| detail.name.as_str()))
            .or(self.gke.detail_error_target_name.as_deref())
    }

    pub fn is_active_gke_detail_target(&self, cluster_name: &str, cluster_location: &str) -> bool {
        self.gke
            .detail_target
            .as_ref()
            .map(|cluster| cluster.name == cluster_name && cluster.location == cluster_location)
            .unwrap_or(false)
    }

    pub fn gke_detail_error(&self) -> Option<&str> {
        self.gke.detail_error.as_deref()
    }

    pub fn gke_detail_target(&self) -> Option<&GkeCluster> {
        self.gke.detail_target.as_ref()
    }

    pub fn restore_gke_detail(
        &mut self,
        name: &str,
        location: &str,
        tab: GkeDetailTab,
    ) -> Option<GkeCluster> {
        let cluster = self
            .gke
            .rows
            .iter()
            .find(|c| c.name == name && c.location == location)
            .cloned()?;
        if let Some(index) = self
            .visible_gke_clusters()
            .iter()
            .position(|c| c.name == name && c.location == location)
        {
            self.gke.selected_visible = index;
        }
        self.gke.view_mode = ResourcesViewMode::Detail;
        self.gke.active_detail_tab = tab;
        self.gke.detail_scroll = 0;
        self.detail_field_index = None;
        self.gke.detail_target = Some(cluster.clone());
        self.gke.detail_error_target_name = None;
        self.gke.detail = None;
        self.gke.detail_error = None;
        Some(cluster)
    }

    pub(super) fn close_gke_detail(&mut self) {
        self.detail_field_index = None;
        self.gke.view_mode = ResourcesViewMode::List;
        self.gke.active_detail_tab = GkeDetailTab::Summary;
        self.gke.detail_target = None;
        self.gke.detail_error_target_name = None;
        self.gke.detail = None;
        self.gke.detail_error = None;
    }
}

fn gke_status_label(status: &GkeClusterStatus) -> String {
    match status {
        GkeClusterStatus::Running => "Running".to_string(),
        GkeClusterStatus::Reconciling => "Reconciling".to_string(),
        GkeClusterStatus::Stopping => "Stopping".to_string(),
        GkeClusterStatus::Error => "Error".to_string(),
        GkeClusterStatus::Unknown(_) => "Unknown".to_string(),
    }
}

fn gke_cluster_matches_filter(cluster: &GkeCluster, filter: &CompiledFilter) -> bool {
    filter.matches(&gke_searchable_fields(cluster))
}

/// The values the `/` filter searches for one row.
///
/// Status contributes its *display label* plus, for
/// [`GkeClusterStatus::Unknown`], the raw status string it carries — never the
/// `{:?}` rendering. The Debug shell (`Unknown("DEGRADED")`) appears in no raw
/// gcloud field and in no entry of the GKE service's `OPAQUE_LABELS`, so a
/// filter matching the shell alone would slip through the server-side pushdown
/// gate and drop the row. The label is enumerated in `OPAQUE_LABELS`, and the
/// raw string is stored verbatim from the JSON `status` field (see
/// `parse_cluster_status` in `crate::services::resources::gke`) which is itself
/// pushed, so both satisfy the never-drop-a-row premise documented in
/// `crate::services::resources::list_filter`.
fn gke_searchable_fields(cluster: &GkeCluster) -> Vec<String> {
    let mut fields = vec![
        cluster.name.clone(),
        cluster.location.clone(),
        gke_status_label(&cluster.status),
        cluster.current_master_version.clone().unwrap_or_default(),
        cluster
            .current_node_count
            .map(|count| count.to_string())
            .unwrap_or_default(),
        match cluster.endpoint_visibility {
            EndpointVisibility::Public => "Public".to_string(),
            EndpointVisibility::Private => "Private".to_string(),
            EndpointVisibility::Unknown => "Unknown".to_string(),
        },
    ];
    if let GkeClusterStatus::Unknown(raw) = &cluster.status {
        fields.push(raw.clone());
    }
    fields
}

fn compare_gke_clusters(
    left: &GkeCluster,
    right: &GkeCluster,
    sort_field: GkeSortField,
    direction: SortDirection,
) -> std::cmp::Ordering {
    let primary = match sort_field {
        GkeSortField::Name => left.name.cmp(&right.name),
        GkeSortField::Status => left.status.label().cmp(&right.status.label()),
        GkeSortField::Location => left.location.cmp(&right.location),
        GkeSortField::Version => left
            .current_master_version
            .as_deref()
            .unwrap_or_default()
            .cmp(right.current_master_version.as_deref().unwrap_or_default()),
        GkeSortField::Nodes => left.current_node_count.cmp(&right.current_node_count),
    };
    let ordering = primary
        .then_with(|| left.name.cmp(&right.name))
        .then_with(|| left.location.cmp(&right.location));
    match direction {
        SortDirection::Ascending => ordering,
        SortDirection::Descending => ordering.reverse(),
    }
}
