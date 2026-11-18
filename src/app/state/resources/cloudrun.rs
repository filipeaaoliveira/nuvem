use super::*;

/// Marker type unifying the Cloud Run services service behind
/// [`ResourceService`]. Never instantiated.
pub enum CloudRun {}

impl ResourceService for CloudRun {
    type Row = CloudRunService;
    type Detail = CloudRunServiceDetail;
    type SortField = CloudRunSortField;
    type DetailTab = CloudRunDetailTab;

    const KIND: ResourceServiceKind = ResourceServiceKind::CloudRunServices;

    fn collection(
        state: &ResourcesState,
    ) -> &ResourceCollectionState<Self::Row, Self::Detail, Self::SortField, Self::DetailTab> {
        &state.cloud_run
    }

    fn collection_mut(
        state: &mut ResourcesState,
    ) -> &mut ResourceCollectionState<Self::Row, Self::Detail, Self::SortField, Self::DetailTab>
    {
        &mut state.cloud_run
    }

    fn matches_filter(row: &Self::Row, filter: &CompiledFilter) -> bool {
        cloud_run_service_matches_filter(row, filter)
    }

    /// The needle sits in the name, the identity column of the service table.
    fn sample_row() -> Self::Row {
        CloudRunService {
            name: SAMPLE_ROW_NEEDLE.to_string(),
            region: "europe-west1".to_string(),
            url: Some(format!("https://{SAMPLE_ROW_NEEDLE}.example.com")),
            latest_ready_revision: Some(format!("{SAMPLE_ROW_NEEDLE}-00001")),
            latest_created_revision: Some(format!("{SAMPLE_ROW_NEEDLE}-00001")),
            traffic_summary: "100% latest".to_string(),
            status: CloudRunServiceStatus::Ready,
            ready_message: None,
        }
    }

    fn compare(
        left: &Self::Row,
        right: &Self::Row,
        sort_field: Self::SortField,
        direction: SortDirection,
    ) -> std::cmp::Ordering {
        compare_cloud_run_services(left, right, sort_field, direction)
    }

    /// Cloud Run has no lifecycle actions and so no marks; this exists only to
    /// keep [`ResourceService`] total.
    fn row_id(row: &Self::Row) -> ResourceRowId {
        ResourceRowId {
            name: row.name.clone(),
            location: row.region.clone(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloudRunSortField {
    Name,
    Status,
    Region,
    Url,
    LatestRevision,
}

impl CloudRunSortField {
    pub fn label(self) -> &'static str {
        match self {
            Self::Name => "Name",
            Self::Status => "Status",
            Self::Region => "Region",
            Self::Url => "URL",
            Self::LatestRevision => "Latest Revision",
        }
    }

    pub const COLUMN_COUNT: usize = 6;

    pub fn from_column(index: usize) -> Option<Self> {
        match index {
            0 => Some(Self::Name),
            1 => Some(Self::Status),
            2 => Some(Self::Region),
            3 => Some(Self::Url),
            4 => Some(Self::LatestRevision),
            _ => None,
        }
    }

    pub fn column(self) -> usize {
        match self {
            Self::Name => 0,
            Self::Status => 1,
            Self::Region => 2,
            Self::Url => 3,
            Self::LatestRevision => 4,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloudRunDetailTab {
    Summary,
    Traffic,
    Revisions,
    Configuration,
    Conditions,
    Json,
}

impl CloudRunDetailTab {
    pub const ALL: [Self; 6] = [
        Self::Summary,
        Self::Traffic,
        Self::Revisions,
        Self::Configuration,
        Self::Conditions,
        Self::Json,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Summary => "Summary",
            Self::Traffic => "Traffic",
            Self::Revisions => "Revisions",
            Self::Configuration => "Configuration",
            Self::Conditions => "Conditions",
            Self::Json => "JSON",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloudRunRevisionTab {
    Summary,
    Conditions,
    Json,
}

impl CloudRunRevisionTab {
    pub const ALL: [Self; 3] = [Self::Summary, Self::Conditions, Self::Json];

    pub fn label(self) -> &'static str {
        match self {
            Self::Summary => "Summary",
            Self::Conditions => "Conditions",
            Self::Json => "JSON",
        }
    }
}

/// Nested view state for drilling into a single Cloud Run revision from a
/// service's Revisions tab.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CloudRunRevisionView {
    target: CloudRunRevision,
    region: String,
    active_tab: CloudRunRevisionTab,
    scroll: u16,
    detail: Option<CloudRunRevisionDetail>,
    detail_error: Option<String>,
}

impl ResourcesState {
    pub fn cloud_run_services(&self) -> &[CloudRunService] {
        &self.cloud_run.rows
    }

    pub fn visible_cloud_run_services(&self) -> Vec<&CloudRunService> {
        let filter = self.cloud_run.filter_applied.filter();
        let mut rows = self
            .cloud_run
            .rows
            .iter()
            .filter(|service| cloud_run_service_matches_filter(service, filter))
            .collect::<Vec<_>>();
        rows.sort_by(|left, right| {
            compare_cloud_run_services(
                left,
                right,
                self.cloud_run.sort_field,
                self.cloud_run.sort_direction,
            )
        });
        rows
    }

    /// Returns the number of Cloud Run services matching the applied filter.
    pub fn visible_cloud_run_service_count(&self) -> usize {
        let filter = self.cloud_run.filter_applied.filter();
        self.cloud_run
            .rows
            .iter()
            .filter(|service| cloud_run_service_matches_filter(service, filter))
            .count()
    }

    pub fn selected_cloud_run_service(&self) -> Option<&CloudRunService> {
        self.visible_cloud_run_services()
            .get(self.cloud_run.selected_visible)
            .copied()
    }

    pub fn set_cloud_run_services(
        &mut self,
        context: GcloudContext,
        services: Vec<CloudRunService>,
    ) -> Option<String> {
        if self.cloud_run.context.as_ref() != Some(&context) {
            self.command_preview = None;
        }
        let prev_selected = self
            .selected_cloud_run_service()
            .map(|s| (s.name.clone(), s.region.clone()));
        let detail_open = self.cloud_run.view_mode == ResourcesViewMode::Detail;
        let detail_key = self
            .cloud_run
            .detail_target
            .as_ref()
            .map(|s| (s.name.clone(), s.region.clone()));

        self.cloud_run.context = Some(context);
        self.cloud_run.rows = services;
        self.cloud_run.stale = false;
        self.cloud_run.last_problem = None;
        self.cloud_run.clear_load_phase();

        let mut dropped_detail = None;
        if detail_open {
            let still_present = detail_key
                .as_ref()
                .map(|(n, r)| {
                    self.cloud_run
                        .rows
                        .iter()
                        .any(|s| &s.name == n && &s.region == r)
                })
                .unwrap_or(false);
            if !still_present {
                dropped_detail = detail_key.map(|(name, _)| name);
                self.close_cloud_run_detail();
            }
        } else {
            self.close_cloud_run_detail();
        }

        self.cloud_run.selected_visible = prev_selected
            .as_ref()
            .and_then(|(n, r)| {
                self.visible_cloud_run_services()
                    .iter()
                    .position(|s| &s.name == n && &s.region == r)
            })
            .unwrap_or_else(|| {
                self.cloud_run
                    .selected_visible
                    .min(self.visible_cloud_run_services().len().saturating_sub(1))
            });

        dropped_detail
    }

    pub fn mark_cloud_run_stale_and_clear(&mut self) {
        self.cloud_run.filter_draft = self.cloud_run.filter_applied.source().to_string();
        self.cloud_run.filter_input_active = false;
        self.cloud_run.rows.clear();
        self.cloud_run.context = None;
        self.cloud_run.selected_visible = 0;
        self.cloud_run.stale = true;
        self.command_preview = None;
        self.close_cloud_run_detail();
    }

    pub fn cloud_run_loaded(&self) -> bool {
        self.cloud_run.context.is_some()
    }

    pub fn cloud_run_stale(&self) -> bool {
        self.cloud_run.stale
    }

    pub fn cloud_run_problem(&self) -> Option<&DiagnosticProblem> {
        self.cloud_run.last_problem.as_ref()
    }

    pub fn cloud_run_status_counts(&self) -> Vec<(String, usize)> {
        count_labels(
            self.cloud_run
                .rows
                .iter()
                .map(|s| cloud_run_status_label(&s.status)),
        )
    }

    pub fn open_cloud_run_detail(&mut self) -> Option<CloudRunService> {
        let service = self.selected_cloud_run_service()?.clone();
        self.command_preview = None;
        self.cloud_run_revision = None;
        self.selected_revision = 0;
        self.cloud_run.view_mode = ResourcesViewMode::Detail;
        self.cloud_run.active_detail_tab = CloudRunDetailTab::Summary;
        self.cloud_run.detail_scroll = 0;
        self.detail_field_index = None;
        self.cloud_run.detail_target = Some(service.clone());
        self.cloud_run.detail_error_target_name = None;
        self.cloud_run.detail = None;
        self.cloud_run.detail_error = None;
        Some(service)
    }

    /// Returns the active Cloud Run detail tab without consulting other state.
    pub fn active_cloud_run_detail_tab(&self) -> CloudRunDetailTab {
        self.cloud_run.active_detail_tab
    }

    pub fn cloud_run_detail(&self) -> Option<&CloudRunServiceDetail> {
        self.cloud_run.detail.as_ref()
    }

    pub fn set_cloud_run_detail(&mut self, detail: CloudRunServiceDetail) {
        self.cloud_run.detail_error_target_name = None;
        self.cloud_run.detail_error = None;
        self.cloud_run.detail = Some(detail);
    }

    pub fn set_cloud_run_detail_error(&mut self, service_name: String, error: String) {
        self.cloud_run.view_mode = ResourcesViewMode::Detail;
        self.cloud_run.detail = None;
        self.cloud_run.detail_error_target_name = Some(service_name);
        self.cloud_run.detail_error = Some(error);
    }

    pub fn cloud_run_detail_target(&self) -> Option<&CloudRunService> {
        self.cloud_run.detail_target.as_ref()
    }

    pub fn cloud_run_detail_target_name(&self) -> Option<&str> {
        self.cloud_run
            .detail_target
            .as_ref()
            .map(|service| service.name.as_str())
            .or_else(|| {
                self.cloud_run
                    .detail
                    .as_ref()
                    .map(|detail| detail.name.as_str())
            })
            .or(self.cloud_run.detail_error_target_name.as_deref())
    }

    pub fn is_active_cloud_run_detail_target(
        &self,
        service_name: &str,
        service_region: &str,
    ) -> bool {
        self.cloud_run
            .detail_target
            .as_ref()
            .map(|service| service.name == service_name && service.region == service_region)
            .unwrap_or(false)
    }

    pub fn cloud_run_detail_error(&self) -> Option<&str> {
        self.cloud_run.detail_error.as_deref()
    }

    pub fn restore_cloud_run_detail(
        &mut self,
        name: &str,
        region: &str,
        tab: CloudRunDetailTab,
    ) -> Option<CloudRunService> {
        let service = self
            .cloud_run
            .rows
            .iter()
            .find(|s| s.name == name && s.region == region)
            .cloned()?;
        if let Some(index) = self
            .visible_cloud_run_services()
            .iter()
            .position(|s| s.name == name && s.region == region)
        {
            self.cloud_run.selected_visible = index;
        }
        self.cloud_run_revision = None;
        self.selected_revision = 0;
        self.cloud_run.view_mode = ResourcesViewMode::Detail;
        self.cloud_run.active_detail_tab = tab;
        self.cloud_run.detail_scroll = 0;
        self.detail_field_index = None;
        self.cloud_run.detail_target = Some(service.clone());
        self.cloud_run.detail_error_target_name = None;
        self.cloud_run.detail = None;
        self.cloud_run.detail_error = None;
        Some(service)
    }

    // --- Nested Cloud Run revision view ---

    pub(super) fn is_on_cloud_run_revisions_tab(&self) -> bool {
        self.cloud_run.view_mode == ResourcesViewMode::Detail
            && self.cloud_run.active_detail_tab == CloudRunDetailTab::Revisions
    }

    pub(super) fn cloud_run_revision_count(&self) -> usize {
        self.cloud_run
            .detail
            .as_ref()
            .map(|detail| detail.revisions.len())
            .unwrap_or(0)
    }

    /// Returns the revision under the Revisions-tab cursor, if any.
    pub fn selected_cloud_run_revision(&self) -> Option<&CloudRunRevision> {
        self.cloud_run
            .detail
            .as_ref()
            .and_then(|detail| detail.revisions.get(self.selected_revision))
    }

    pub fn selected_cloud_run_revision_index(&self) -> usize {
        self.selected_revision
    }

    pub fn open_cloud_run_revision(&mut self, revision: CloudRunRevision, region: String) {
        // The revision view takes the pane over entirely, so the service
        // detail's field cursor no longer describes anything on screen.
        self.detail_field_index = None;
        self.cloud_run_revision = Some(CloudRunRevisionView {
            target: revision,
            region,
            active_tab: CloudRunRevisionTab::Summary,
            scroll: 0,
            detail: None,
            detail_error: None,
        });
    }

    pub fn close_cloud_run_revision(&mut self) {
        self.detail_field_index = None;
        self.cloud_run_revision = None;
    }

    pub fn is_showing_cloud_run_revision(&self) -> bool {
        self.cloud_run_revision.is_some()
    }

    pub fn cloud_run_revision_target(&self) -> Option<&CloudRunRevision> {
        self.cloud_run_revision.as_ref().map(|view| &view.target)
    }

    pub fn cloud_run_revision_region(&self) -> Option<&str> {
        self.cloud_run_revision
            .as_ref()
            .map(|view| view.region.as_str())
    }

    pub fn cloud_run_revision_detail(&self) -> Option<&CloudRunRevisionDetail> {
        self.cloud_run_revision
            .as_ref()
            .and_then(|view| view.detail.as_ref())
    }

    pub fn cloud_run_revision_detail_error(&self) -> Option<&str> {
        self.cloud_run_revision
            .as_ref()
            .and_then(|view| view.detail_error.as_deref())
    }

    pub fn active_cloud_run_revision_tab(&self) -> CloudRunRevisionTab {
        self.cloud_run_revision
            .as_ref()
            .map(|view| view.active_tab)
            .unwrap_or(CloudRunRevisionTab::Summary)
    }

    pub fn cloud_run_revision_scroll(&self) -> u16 {
        self.cloud_run_revision
            .as_ref()
            .map(|view| view.scroll)
            .unwrap_or(0)
    }

    pub fn set_cloud_run_revision_detail(&mut self, detail: CloudRunRevisionDetail) {
        if let Some(view) = self.cloud_run_revision.as_mut() {
            view.detail_error = None;
            view.detail = Some(detail);
        }
    }

    pub fn set_cloud_run_revision_detail_error(&mut self, error: String) {
        if let Some(view) = self.cloud_run_revision.as_mut() {
            view.detail = None;
            view.detail_error = Some(error);
        }
    }

    pub fn select_next_cloud_run_revision_tab(&mut self) {
        if let Some(view) = self.cloud_run_revision.as_mut() {
            view.scroll = 0;
            let index = CloudRunRevisionTab::ALL
                .iter()
                .position(|tab| *tab == view.active_tab)
                .unwrap_or(0);
            view.active_tab =
                CloudRunRevisionTab::ALL[(index + 1) % CloudRunRevisionTab::ALL.len()];
            // A new tab means a new field list, as on the service detail.
            self.detail_field_index = None;
        }
    }

    pub fn select_previous_cloud_run_revision_tab(&mut self) {
        if let Some(view) = self.cloud_run_revision.as_mut() {
            view.scroll = 0;
            let index = CloudRunRevisionTab::ALL
                .iter()
                .position(|tab| *tab == view.active_tab)
                .unwrap_or(0);
            view.active_tab = CloudRunRevisionTab::ALL
                [(index + CloudRunRevisionTab::ALL.len() - 1) % CloudRunRevisionTab::ALL.len()];
            self.detail_field_index = None;
        }
    }

    pub fn scroll_cloud_run_revision_down(&mut self) {
        if let Some(view) = self.cloud_run_revision.as_mut() {
            view.scroll = view.scroll.saturating_add(DETAIL_PAGE_SIZE);
        }
    }

    pub fn scroll_cloud_run_revision_up(&mut self) {
        if let Some(view) = self.cloud_run_revision.as_mut() {
            view.scroll = view.scroll.saturating_sub(DETAIL_PAGE_SIZE);
        }
    }

    pub(super) fn close_cloud_run_detail(&mut self) {
        self.detail_field_index = None;
        self.cloud_run.view_mode = ResourcesViewMode::List;
        self.cloud_run.active_detail_tab = CloudRunDetailTab::Summary;
        self.cloud_run.detail_target = None;
        self.cloud_run.detail_error_target_name = None;
        self.cloud_run.detail = None;
        self.cloud_run.detail_error = None;
        self.cloud_run_revision = None;
        self.selected_revision = 0;
    }
}

fn cloud_run_status_label(status: &CloudRunServiceStatus) -> String {
    match status {
        CloudRunServiceStatus::Ready => "Ready".to_string(),
        CloudRunServiceStatus::NotReady => "Not ready".to_string(),
        CloudRunServiceStatus::Reconciling => "Reconciling".to_string(),
        CloudRunServiceStatus::Failed => "Failed".to_string(),
        CloudRunServiceStatus::Unknown(_) => "Unknown".to_string(),
    }
}

fn cloud_run_service_matches_filter(service: &CloudRunService, filter: &CompiledFilter) -> bool {
    filter.matches(&cloud_run_searchable_fields(service))
}

fn cloud_run_searchable_fields(service: &CloudRunService) -> Vec<String> {
    vec![
        service.name.clone(),
        service.region.clone(),
        service.status.label().into_owned(),
        service.url.clone().unwrap_or_default(),
        service.latest_ready_revision.clone().unwrap_or_default(),
        service.latest_created_revision.clone().unwrap_or_default(),
        service.traffic_summary.clone(),
    ]
}

fn compare_cloud_run_services(
    left: &CloudRunService,
    right: &CloudRunService,
    sort_field: CloudRunSortField,
    direction: SortDirection,
) -> std::cmp::Ordering {
    let primary = match sort_field {
        CloudRunSortField::Name => left.name.cmp(&right.name),
        CloudRunSortField::Status => left.status.label().cmp(&right.status.label()),
        CloudRunSortField::Region => left.region.cmp(&right.region),
        CloudRunSortField::Url => left
            .url
            .as_deref()
            .unwrap_or_default()
            .cmp(right.url.as_deref().unwrap_or_default()),
        CloudRunSortField::LatestRevision => left
            .latest_ready_revision
            .as_deref()
            .unwrap_or_default()
            .cmp(right.latest_ready_revision.as_deref().unwrap_or_default()),
    };
    let ordering = primary
        .then_with(|| left.name.cmp(&right.name))
        .then_with(|| left.region.cmp(&right.region));
    match direction {
        SortDirection::Ascending => ordering,
        SortDirection::Descending => ordering.reverse(),
    }
}
