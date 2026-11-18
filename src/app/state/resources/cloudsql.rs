use super::*;

/// Marker type unifying the Cloud SQL service behind [`ResourceService`].
/// Never instantiated.
pub enum CloudSql {}

impl ResourceService for CloudSql {
    type Row = CloudSqlInstance;
    type Detail = CloudSqlInstanceDetail;
    type SortField = CloudSqlInstanceSortField;
    type DetailTab = CloudSqlDetailTab;

    const KIND: ResourceServiceKind = ResourceServiceKind::CloudSql;

    fn collection(
        state: &ResourcesState,
    ) -> &ResourceCollectionState<Self::Row, Self::Detail, Self::SortField, Self::DetailTab> {
        &state.cloud_sql
    }

    fn collection_mut(
        state: &mut ResourcesState,
    ) -> &mut ResourceCollectionState<Self::Row, Self::Detail, Self::SortField, Self::DetailTab>
    {
        &mut state.cloud_sql
    }

    fn matches_filter(row: &Self::Row, filter: &CompiledFilter) -> bool {
        cloud_sql_matches_filter(row, filter)
    }

    /// The needle sits in the name, the identity column of the instance table.
    fn sample_row() -> Self::Row {
        CloudSqlInstance {
            name: SAMPLE_ROW_NEEDLE.to_string(),
            database_version: "POSTGRES_15".to_string(),
            engine: "PostgreSQL".to_string(),
            region: "europe-west1".to_string(),
            tier: "db-custom-1-3840".to_string(),
            availability: CloudSqlAvailability::Zonal,
            status: CloudSqlStatus::Runnable,
            connectivity: EndpointVisibility::Private,
        }
    }

    fn compare(
        left: &Self::Row,
        right: &Self::Row,
        sort_field: Self::SortField,
        direction: SortDirection,
    ) -> std::cmp::Ordering {
        compare_cloud_sql_instances(left, right, sort_field, direction)
    }

    fn row_id(row: &Self::Row) -> ResourceRowId {
        ResourceRowId {
            name: row.name.clone(),
            location: row.region.clone(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloudSqlInstanceSortField {
    Name,
    Engine,
    Region,
    Tier,
    Availability,
    Status,
}

impl CloudSqlInstanceSortField {
    pub fn label(self) -> &'static str {
        match self {
            Self::Name => "Name",
            Self::Engine => "Engine",
            Self::Region => "Region",
            Self::Tier => "Tier",
            Self::Availability => "Availability",
            Self::Status => "Status",
        }
    }

    pub const COLUMN_COUNT: usize = 7;

    pub fn from_column(index: usize) -> Option<Self> {
        match index {
            0 => Some(Self::Name),
            1 => Some(Self::Engine),
            2 => Some(Self::Region),
            3 => Some(Self::Tier),
            4 => Some(Self::Availability),
            6 => Some(Self::Status),
            _ => None,
        }
    }

    pub fn column(self) -> usize {
        match self {
            Self::Name => 0,
            Self::Engine => 1,
            Self::Region => 2,
            Self::Tier => 3,
            Self::Availability => 4,
            Self::Status => 6,
        }
    }
}

lifecycle_actions! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum CloudSqlInstanceAction {
        Start,
        Stop,
        Restart,
        Failover,
    }
}

impl CloudSqlInstanceAction {
    pub fn label(self) -> &'static str {
        match self {
            Self::Start => "Start instance",
            Self::Stop => "Stop instance",
            Self::Restart => "Restart instance",
            Self::Failover => "Fail over instance",
        }
    }

    pub fn impact_warning(self) -> &'static str {
        match self {
            Self::Start => "Activates the instance; billing resumes.",
            Self::Stop => "Databases become unavailable and billing pauses.",
            Self::Restart => "Brief downtime while the instance restarts.",
            Self::Failover => "Promotes the standby; short outage during failover.",
        }
    }

    /// Returns true for actions whose effect is destructive enough to warrant
    /// an explicit typed-count acknowledgement in a batch confirmation.
    pub fn is_destructive(self) -> bool {
        matches!(self, Self::Stop | Self::Restart | Self::Failover)
    }

    pub fn is_eligible(self, status: &CloudSqlStatus, availability: CloudSqlAvailability) -> bool {
        match self {
            Self::Start => matches!(status, CloudSqlStatus::Stopped),
            Self::Stop => matches!(status, CloudSqlStatus::Runnable),
            Self::Restart => matches!(status, CloudSqlStatus::Runnable),
            Self::Failover => matches!(status, CloudSqlStatus::Runnable) && availability.is_ha(),
        }
    }

    pub fn requested_status(self, name: &str) -> String {
        let verb = match self {
            Self::Start => "Start",
            Self::Stop => "Stop",
            Self::Restart => "Restart",
            Self::Failover => "Failover",
        };
        format!("{verb} requested for {name}")
    }
}

/// The call a Cloud SQL lifecycle action makes, named once.
///
/// Cloud SQL has no `start` or `stop` method: both are a patch of the
/// activation policy, which is why this carries a field where the compute
/// equivalent does not. Without it the two would render identically and a
/// confirmation would not say which one it was for.
pub fn cloud_sql_lifecycle_call(
    context: &GcloudContext,
    name: &str,
    action: CloudSqlInstanceAction,
) -> ResourceCall {
    let project = context.project.as_deref().unwrap_or_default();
    let resource = format!("projects/{project}/instances/{name}");
    match action {
        CloudSqlInstanceAction::Start => ResourceCall::new("sql", "instances.patch", resource)
            .with_field("settings.activationPolicy", "ALWAYS"),
        CloudSqlInstanceAction::Stop => ResourceCall::new("sql", "instances.patch", resource)
            .with_field("settings.activationPolicy", "NEVER"),
        CloudSqlInstanceAction::Restart => ResourceCall::new("sql", "instances.restart", resource),
        CloudSqlInstanceAction::Failover => {
            ResourceCall::new("sql", "instances.failover", resource)
        }
    }
}

/// The one argv both the runtime executes and every preview renders for a
/// Cloud SQL lifecycle action, so the executed command and the
/// operator-facing preview cannot drift apart while every caller goes
/// through it.
pub(crate) fn cloud_sql_lifecycle_argv(
    context: &GcloudContext,
    name: &str,
    action: CloudSqlInstanceAction,
) -> Vec<String> {
    let configuration = context.configuration.as_str().to_string();
    match action {
        CloudSqlInstanceAction::Start => vec![
            "--configuration".to_string(),
            configuration,
            "sql".to_string(),
            "instances".to_string(),
            "patch".to_string(),
            name.to_string(),
            "--activation-policy=ALWAYS".to_string(),
        ],
        CloudSqlInstanceAction::Stop => vec![
            "--configuration".to_string(),
            configuration,
            "sql".to_string(),
            "instances".to_string(),
            "patch".to_string(),
            name.to_string(),
            "--activation-policy=NEVER".to_string(),
        ],
        CloudSqlInstanceAction::Restart => vec![
            "--configuration".to_string(),
            configuration,
            "sql".to_string(),
            "instances".to_string(),
            "restart".to_string(),
            name.to_string(),
        ],
        CloudSqlInstanceAction::Failover => vec![
            "--configuration".to_string(),
            configuration,
            "sql".to_string(),
            "instances".to_string(),
            "failover".to_string(),
            name.to_string(),
        ],
    }
}

/// A pending, confirmed-before-execution lifecycle action against a Cloud SQL
/// instance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CloudSqlActionConfirmation {
    pub(super) action: CloudSqlInstanceAction,
    pub(super) context: GcloudContext,
    pub(super) instance: CloudSqlInstance,
}

impl CloudSqlActionConfirmation {
    pub fn action(&self) -> CloudSqlInstanceAction {
        self.action
    }

    pub fn context(&self) -> &GcloudContext {
        &self.context
    }

    pub fn instance(&self) -> &CloudSqlInstance {
        &self.instance
    }

    /// The call this confirmation is for. See the compute equivalent.
    pub fn call(&self) -> ResourceCall {
        cloud_sql_lifecycle_call(&self.context, &self.instance.name, self.action)
    }

    pub fn impact_warning(&self) -> &'static str {
        self.action.impact_warning()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloudSqlDetailTab {
    Summary,
    Connectivity,
    Maintenance,
    Backups,
    Replicas,
    Users,
    Json,
}

impl CloudSqlDetailTab {
    pub const ALL: [Self; 7] = [
        Self::Summary,
        Self::Connectivity,
        Self::Maintenance,
        Self::Backups,
        Self::Replicas,
        Self::Users,
        Self::Json,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Summary => "Summary",
            Self::Connectivity => "Connectivity",
            Self::Maintenance => "Maintenance",
            Self::Backups => "Backups",
            Self::Replicas => "Replicas",
            Self::Users => "Users",
            Self::Json => "JSON",
        }
    }
}

impl ResourcesState {
    pub fn cloud_sql_instances(&self) -> &[CloudSqlInstance] {
        &self.cloud_sql.rows
    }

    pub fn visible_cloud_sql_instances(&self) -> Vec<&CloudSqlInstance> {
        self.visible_rows::<CloudSql>()
    }

    /// Returns the number of Cloud SQL instances matching the applied filter.
    pub fn visible_cloud_sql_instance_count(&self) -> usize {
        let filter = self.cloud_sql.filter_applied.filter();
        self.cloud_sql
            .rows
            .iter()
            .filter(|instance| cloud_sql_matches_filter(instance, filter))
            .count()
    }

    pub fn selected_cloud_sql_instance(&self) -> Option<&CloudSqlInstance> {
        self.visible_cloud_sql_instances()
            .get(self.cloud_sql.selected_visible)
            .copied()
    }

    pub fn set_cloud_sql_instances(
        &mut self,
        context: GcloudContext,
        instances: Vec<CloudSqlInstance>,
    ) -> Option<String> {
        if self.cloud_sql.context.as_ref() != Some(&context) {
            self.command_preview = None;
            // A Cloud SQL tunnel preview names a row from the *previous*
            // context; the SSH preview is compute-only so it never needed
            // clearing here, but this one does.
            self.tunnel_launch_preview = None;
        }
        let prev_selected = self
            .selected_cloud_sql_instance()
            .map(|i| (i.name.clone(), i.region.clone()));
        let detail_open = self.cloud_sql.view_mode == ResourcesViewMode::Detail;
        let detail_key = self
            .cloud_sql
            .detail_target
            .as_ref()
            .map(|i| (i.name.clone(), i.region.clone()));

        self.cloud_sql.context = Some(context);
        self.cloud_sql.rows = instances;
        self.cloud_sql.stale = false;
        self.cloud_sql.last_problem = None;
        self.cloud_sql.last_diagnostic = None;
        self.cloud_sql.clear_load_phase();

        self.prune_marks::<CloudSql>();

        let mut dropped_detail = None;
        if detail_open {
            let still_present = detail_key
                .as_ref()
                .map(|(n, r)| {
                    self.cloud_sql
                        .rows
                        .iter()
                        .any(|i| &i.name == n && &i.region == r)
                })
                .unwrap_or(false);
            if !still_present {
                dropped_detail = detail_key.map(|(name, _)| name);
                self.close_cloud_sql_detail();
            }
        } else {
            self.close_cloud_sql_detail();
        }

        self.cloud_sql.selected_visible = prev_selected
            .as_ref()
            .and_then(|(n, r)| {
                self.visible_cloud_sql_instances()
                    .iter()
                    .position(|i| &i.name == n && &i.region == r)
            })
            .unwrap_or_else(|| {
                self.cloud_sql
                    .selected_visible
                    .min(self.visible_cloud_sql_instances().len().saturating_sub(1))
            });

        dropped_detail
    }

    pub fn mark_cloud_sql_stale_and_clear(&mut self) {
        self.cloud_sql.filter_draft = self.cloud_sql.filter_applied.source().to_string();
        self.cloud_sql.filter_input_active = false;
        self.cloud_sql.rows.clear();
        self.cloud_sql.context = None;
        self.cloud_sql.selected_visible = 0;
        self.cloud_sql.stale = true;
        self.clear_marks_for::<CloudSql>();
        // A Cloud SQL confirmation snapshots the rows just discarded, so it may
        // not outlive them — mirrors `mark_stale_and_clear` on the compute
        // side, which likewise leaves another service's confirmation alone.
        self.close_confirmations_owned_by(ResourceServiceKind::CloudSql);
        self.command_preview = None;
        // Same reason as in `set_cloud_sql_instances`: the previewed row is
        // gone, so the plan built from it must go with it.
        self.tunnel_launch_preview = None;
        self.close_cloud_sql_detail();
    }

    /// Returns marked Cloud SQL instances in visible sorted order, followed by
    /// any hidden (filtered-out) marked instances in stable order — the Cloud
    /// SQL twin of [`ResourcesState::marked_instances`].
    pub fn marked_cloud_sql_instances(&self) -> Vec<CloudSqlInstance> {
        self.marked_rows::<CloudSql>()
    }

    /// Returns true when the instance identified by `(name, region)` is marked.
    pub fn is_cloud_sql_instance_marked(&self, name: &str, region: &str) -> bool {
        self.cloud_sql.marks.contains(&ResourceRowId {
            name: name.to_string(),
            location: region.to_string(),
        })
    }

    pub fn cloud_sql_loaded(&self) -> bool {
        self.cloud_sql.context.is_some()
    }

    pub fn cloud_sql_stale(&self) -> bool {
        self.cloud_sql.stale
    }

    pub fn cloud_sql_problem(&self) -> Option<&DiagnosticProblem> {
        self.cloud_sql.last_problem.as_ref()
    }

    pub fn cloud_sql_status_counts(&self) -> Vec<(String, usize)> {
        count_labels(
            self.cloud_sql
                .rows
                .iter()
                .map(|i| cloud_sql_status_count_label(&i.status)),
        )
    }

    pub fn start_cloud_sql_action_confirmation(
        &mut self,
        context: GcloudContext,
        instance: CloudSqlInstance,
        action: CloudSqlInstanceAction,
    ) {
        self.close_other_modals();
        self.action_confirmation = Some(ResourceActionConfirmation::CloudSql(
            CloudSqlActionConfirmation {
                action,
                context,
                instance,
            },
        ));
    }

    /// Takes the pending confirmation and turns it into a launch plan, clearing
    /// the modal.
    pub fn open_cloud_sql_detail(&mut self) -> Option<CloudSqlInstance> {
        let instance = self.selected_cloud_sql_instance()?.clone();
        self.command_preview = None;
        self.tunnel_launch_preview = None;
        self.cloud_sql.view_mode = ResourcesViewMode::Detail;
        self.cloud_sql.active_detail_tab = CloudSqlDetailTab::Summary;
        self.cloud_sql.detail_scroll = 0;
        self.detail_field_index = None;
        self.cloud_sql.detail_target = Some(instance.clone());
        self.cloud_sql.detail_error_target_name = None;
        self.cloud_sql.detail = None;
        self.cloud_sql.detail_error = None;
        Some(instance)
    }

    /// Returns the active Cloud SQL detail tab without consulting other state.
    pub fn active_cloud_sql_detail_tab(&self) -> CloudSqlDetailTab {
        self.cloud_sql.active_detail_tab
    }

    pub fn cloud_sql_detail(&self) -> Option<&CloudSqlInstanceDetail> {
        self.cloud_sql.detail.as_ref()
    }

    pub fn set_cloud_sql_detail(&mut self, detail: CloudSqlInstanceDetail) {
        self.cloud_sql.detail_error_target_name = None;
        self.cloud_sql.detail_error = None;
        self.cloud_sql.detail = Some(detail);
    }

    /// Stores lazily-loaded users on the active Cloud SQL detail, if any.
    pub fn set_cloud_sql_users(&mut self, users: Vec<nuvem_domain::gcp::resource::CloudSqlUser>) {
        if let Some(detail) = self.cloud_sql.detail.as_mut() {
            detail.users = users;
        }
    }

    pub fn set_cloud_sql_detail_error(&mut self, instance_name: String, error: String) {
        self.cloud_sql.view_mode = ResourcesViewMode::Detail;
        self.cloud_sql.detail = None;
        self.cloud_sql.detail_error_target_name = Some(instance_name);
        self.cloud_sql.detail_error = Some(error);
    }

    pub fn cloud_sql_detail_target_name(&self) -> Option<&str> {
        self.cloud_sql
            .detail_target
            .as_ref()
            .map(|instance| instance.name.as_str())
            .or_else(|| {
                self.cloud_sql
                    .detail
                    .as_ref()
                    .map(|detail| detail.name.as_str())
            })
            .or(self.cloud_sql.detail_error_target_name.as_deref())
    }

    pub fn is_active_cloud_sql_detail_target(
        &self,
        instance_name: &str,
        instance_region: &str,
    ) -> bool {
        self.cloud_sql
            .detail_target
            .as_ref()
            .map(|instance| instance.name == instance_name && instance.region == instance_region)
            .unwrap_or(false)
    }

    /// True when the active Cloud SQL detail target matches `instance_name`,
    /// regardless of region. Used for lazy user loads keyed only by name.
    pub fn is_active_cloud_sql_detail_name(&self, instance_name: &str) -> bool {
        self.cloud_sql
            .detail_target
            .as_ref()
            .map(|instance| instance.name == instance_name)
            .unwrap_or(false)
    }

    pub fn cloud_sql_detail_error(&self) -> Option<&str> {
        self.cloud_sql.detail_error.as_deref()
    }

    pub fn cloud_sql_detail_target(&self) -> Option<&CloudSqlInstance> {
        self.cloud_sql.detail_target.as_ref()
    }

    /// Returns the active Cloud SQL detail's loaded users, if the detail and
    /// its lazily-loaded user list are present.
    pub fn cloud_sql_detail_users_loaded(&self) -> bool {
        self.cloud_sql
            .detail
            .as_ref()
            .map(|detail| !detail.users.is_empty())
            .unwrap_or(false)
    }

    pub fn restore_cloud_sql_detail(
        &mut self,
        name: &str,
        region: &str,
        tab: CloudSqlDetailTab,
    ) -> Option<CloudSqlInstance> {
        let instance = self
            .cloud_sql
            .rows
            .iter()
            .find(|i| i.name == name && i.region == region)
            .cloned()?;
        if let Some(index) = self
            .visible_cloud_sql_instances()
            .iter()
            .position(|i| i.name == name && i.region == region)
        {
            self.cloud_sql.selected_visible = index;
        }
        self.cloud_sql.view_mode = ResourcesViewMode::Detail;
        self.cloud_sql.active_detail_tab = tab;
        self.cloud_sql.detail_scroll = 0;
        self.detail_field_index = None;
        self.cloud_sql.detail_target = Some(instance.clone());
        self.cloud_sql.detail_error_target_name = None;
        self.cloud_sql.detail = None;
        self.cloud_sql.detail_error = None;
        Some(instance)
    }

    pub(super) fn close_cloud_sql_detail(&mut self) {
        self.detail_field_index = None;
        self.cloud_sql.view_mode = ResourcesViewMode::List;
        self.cloud_sql.active_detail_tab = CloudSqlDetailTab::Summary;
        self.cloud_sql.detail_target = None;
        self.cloud_sql.detail_error_target_name = None;
        self.cloud_sql.detail = None;
        self.cloud_sql.detail_error = None;
    }
}

fn cloud_sql_matches_filter(instance: &CloudSqlInstance, filter: &CompiledFilter) -> bool {
    filter.matches(&cloud_sql_searchable_fields(instance))
}

/// The values the `/` filter searches for one row.
///
/// Status contributes its *display label* plus, for
/// [`CloudSqlStatus::Unknown`], the raw status string it carries — never the
/// `{:?}` rendering. The Debug shell (`Unknown("SUSPENDED_BY_BILLING")`)
/// appears in no raw gcloud field and in no entry of the Cloud SQL service's
/// `OPAQUE_LABELS`, so a filter matching the shell alone would slip through
/// the server-side pushdown gate and drop the row. The label is enumerated in
/// `OPAQUE_LABELS`, and the raw string is stored verbatim from the JSON
/// `state` field (see `parse_status` in `crate::services::resources::cloudsql`)
/// which is itself pushed, so both satisfy the never-drop-a-row premise
/// documented in `crate::services::resources::list_filter`.
fn cloud_sql_searchable_fields(instance: &CloudSqlInstance) -> Vec<String> {
    let mut fields = vec![
        instance.name.clone(),
        instance.engine.clone(),
        instance.database_version.clone(),
        instance.region.clone(),
        instance.tier.clone(),
        instance.availability.label().to_string(),
        cloud_sql_status_count_label(&instance.status),
        match instance.connectivity {
            EndpointVisibility::Public => "Public".to_string(),
            EndpointVisibility::Private => "Private".to_string(),
            EndpointVisibility::Unknown => "Unknown".to_string(),
        },
    ];
    if let CloudSqlStatus::Unknown(raw) = &instance.status {
        fields.push(raw.clone());
    }
    fields
}

fn compare_cloud_sql_instances(
    left: &CloudSqlInstance,
    right: &CloudSqlInstance,
    sort_field: CloudSqlInstanceSortField,
    direction: SortDirection,
) -> std::cmp::Ordering {
    let primary = match sort_field {
        CloudSqlInstanceSortField::Name => left.name.cmp(&right.name),
        CloudSqlInstanceSortField::Engine => left.engine.cmp(&right.engine),
        CloudSqlInstanceSortField::Region => left.region.cmp(&right.region),
        CloudSqlInstanceSortField::Tier => left.tier.cmp(&right.tier),
        CloudSqlInstanceSortField::Availability => {
            format!("{:?}", left.availability).cmp(&format!("{:?}", right.availability))
        }
        CloudSqlInstanceSortField::Status => left.status.label().cmp(&right.status.label()),
    };
    let ordering = primary
        .then_with(|| left.name.cmp(&right.name))
        .then_with(|| left.region.cmp(&right.region));
    match direction {
        SortDirection::Ascending => ordering,
        SortDirection::Descending => ordering.reverse(),
    }
}

/// Status label used for the Cloud SQL status-count summary (and the row
/// filter's searchable text): `Unknown` collapses to a bare label so every
/// unrecognised status groups into one bucket. Kept distinct from
/// `batch::cloud_sql_status_label`, which preserves the raw value for per-row
/// display.
fn cloud_sql_status_count_label(status: &CloudSqlStatus) -> String {
    match status {
        CloudSqlStatus::Runnable => "Runnable".to_string(),
        CloudSqlStatus::Stopped => "Stopped".to_string(),
        CloudSqlStatus::PendingCreate => "Pending create".to_string(),
        CloudSqlStatus::Maintenance => "Maintenance".to_string(),
        CloudSqlStatus::Failed => "Failed".to_string(),
        CloudSqlStatus::Suspended => "Suspended".to_string(),
        CloudSqlStatus::Unknown(_) => "Unknown".to_string(),
    }
}
