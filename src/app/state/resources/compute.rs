use super::*;

/// Marker type unifying the compute (GCE instances) service behind
/// [`ResourceService`]. Never instantiated.
pub enum Compute {}

impl ResourceService for Compute {
    type Row = ComputeInstance;
    type Detail = ComputeInstanceDetail;
    type SortField = ComputeInstanceSortField;
    type DetailTab = ComputeDetailTab;

    const KIND: ResourceServiceKind = ResourceServiceKind::ComputeInstances;

    fn collection(
        state: &ResourcesState,
    ) -> &ResourceCollectionState<Self::Row, Self::Detail, Self::SortField, Self::DetailTab> {
        &state.compute
    }

    fn collection_mut(
        state: &mut ResourcesState,
    ) -> &mut ResourceCollectionState<Self::Row, Self::Detail, Self::SortField, Self::DetailTab>
    {
        &mut state.compute
    }

    fn matches_filter(row: &Self::Row, filter: &CompiledFilter) -> bool {
        instance_matches_filter(row, filter)
    }

    /// The needle sits in the name: it is the identity column, and the field
    /// an operator filters an instance list by first.
    fn sample_row() -> Self::Row {
        ComputeInstance {
            id: format!("{SAMPLE_ROW_NEEDLE}-id"),
            name: SAMPLE_ROW_NEEDLE.to_string(),
            zone: "europe-west1-b".to_string(),
            status: ResourceStatus::Running,
            machine_type: "e2-medium".to_string(),
            internal_ip: None,
            external_ip: None,
        }
    }

    fn compare(
        left: &Self::Row,
        right: &Self::Row,
        sort_field: Self::SortField,
        direction: SortDirection,
    ) -> std::cmp::Ordering {
        compare_instances(left, right, sort_field, direction)
    }

    fn row_id(row: &Self::Row) -> ResourceRowId {
        ResourceRowId {
            name: row.name.clone(),
            location: row.zone.clone(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComputeInstanceSortField {
    Name,
    Status,
    Zone,
    MachineType,
}

impl ComputeInstanceSortField {
    pub fn label(self) -> &'static str {
        match self {
            Self::Name => "Name",
            Self::Status => "Status",
            Self::Zone => "Zone",
            Self::MachineType => "Machine Type",
        }
    }

    pub const COLUMN_COUNT: usize = 6;

    pub fn from_column(index: usize) -> Option<Self> {
        match index {
            0 => Some(Self::Name),
            1 => Some(Self::Status),
            2 => Some(Self::Zone),
            3 => Some(Self::MachineType),
            _ => None,
        }
    }

    pub fn column(self) -> usize {
        match self {
            Self::Name => 0,
            Self::Status => 1,
            Self::Zone => 2,
            Self::MachineType => 3,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComputeDetailTab {
    Summary,
    Network,
    Disks,
    Metadata,
    Json,
}

impl ComputeDetailTab {
    pub const ALL: [Self; 5] = [
        Self::Summary,
        Self::Network,
        Self::Disks,
        Self::Metadata,
        Self::Json,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Summary => "Summary",
            Self::Network => "Network",
            Self::Disks => "Disks",
            Self::Metadata => "Metadata",
            Self::Json => "JSON",
        }
    }
}

lifecycle_actions! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum ComputeInstanceAction {
        Start,
        Stop,
        Reset,
        Suspend,
        Resume,
    }
}

impl ComputeInstanceAction {
    pub fn label(self) -> &'static str {
        match self {
            Self::Start => "Start instance",
            Self::Stop => "Stop instance",
            Self::Reset => "Reset instance",
            Self::Suspend => "Suspend instance",
            Self::Resume => "Resume instance",
        }
    }

    pub fn command_verb(self) -> &'static str {
        match self {
            Self::Start => "start",
            Self::Stop => "stop",
            Self::Reset => "reset",
            Self::Suspend => "suspend",
            Self::Resume => "resume",
        }
    }

    /// The API method this action calls, as the service names it.
    ///
    /// Kept beside `command_verb` for the reason `LogTimeRange::minutes` is
    /// kept beside `freshness_arg`: the two describe the same action through
    /// different backends, and they are only safe while they cannot be edited
    /// apart.
    pub fn call_method(self) -> &'static str {
        match self {
            Self::Start => "instances.start",
            Self::Stop => "instances.stop",
            Self::Reset => "instances.reset",
            Self::Suspend => "instances.suspend",
            Self::Resume => "instances.resume",
        }
    }

    pub fn is_eligible(self, status: &ResourceStatus) -> bool {
        matches!(
            (self, status),
            (Self::Start, ResourceStatus::Stopped)
                | (Self::Stop, ResourceStatus::Running)
                | (Self::Reset, ResourceStatus::Running)
                | (Self::Suspend, ResourceStatus::Running)
                | (Self::Resume, ResourceStatus::Suspended)
        )
    }

    pub fn impact_warning(self) -> &'static str {
        match self {
            Self::Start => "Boots the instance; billing resumes.",
            Self::Stop => "Guest OS halts and the instance stops; billing pauses.",
            Self::Reset => "Hard reset; the guest OS reboots without a clean shutdown.",
            Self::Suspend => "Suspends the instance to disk; resume later to restore.",
            Self::Resume => "Resumes a suspended instance; billing resumes.",
        }
    }

    /// Returns true for actions whose effect is destructive enough to warrant
    /// an explicit typed-count acknowledgement in a batch confirmation.
    pub fn is_destructive(self) -> bool {
        matches!(self, Self::Stop | Self::Reset | Self::Suspend)
    }

    pub fn requested_status(self, instance_name: &str) -> String {
        let verb = match self {
            Self::Start => "Start",
            Self::Stop => "Stop",
            Self::Reset => "Reset",
            Self::Suspend => "Suspend",
            Self::Resume => "Resume",
        };
        format!("{verb} requested for {instance_name}")
    }
}

/// The call a GCE lifecycle action makes, named once.
///
/// The argv below is a rendering of this rather than the other way round, so
/// the preview and the execution have one source between them -- the property
/// `compute_lifecycle_argv` used to hold alone, moved somewhere a transport
/// without an argv can still hold it.
pub fn compute_lifecycle_call(
    context: &GcloudContext,
    name: &str,
    zone: &str,
    action: ComputeInstanceAction,
) -> ResourceCall {
    let project = context.project.as_deref().unwrap_or_default();
    ResourceCall::new(
        "compute",
        action.call_method(),
        format!("projects/{project}/zones/{zone}/instances/{name}"),
    )
}

/// The one argv both the runtime executes and every preview renders for a
/// GCE lifecycle action, so the executed command and the operator-facing
/// preview cannot drift apart while every caller goes through it.
///
/// A rendering of [`compute_lifecycle_call`]: the call is the source, and
/// this is what it looks like sent as a command line.
pub fn compute_lifecycle_argv(
    context: &GcloudContext,
    name: &str,
    zone: &str,
    action: ComputeInstanceAction,
) -> Vec<String> {
    vec![
        "--configuration".to_string(),
        context.configuration.as_str().to_string(),
        "compute".to_string(),
        "instances".to_string(),
        action.command_verb().to_string(),
        name.to_string(),
        "--zone".to_string(),
        zone.to_string(),
    ]
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComputeInstanceField {
    Name,
    Zone,
    InternalIp,
    ExternalIp,
    SelfLink,
}

impl ComputeInstanceField {
    pub fn label(self) -> &'static str {
        match self {
            Self::Name => "Copy selected instance name",
            Self::Zone => "Copy selected instance zone",
            Self::InternalIp => "Copy selected instance internal IP",
            Self::ExternalIp => "Copy selected instance external IP",
            Self::SelfLink => "Copy selected instance self link",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Self::Name => "instance name",
            Self::Zone => "instance zone",
            Self::InternalIp => "instance internal IP",
            Self::ExternalIp => "instance external IP",
            Self::SelfLink => "instance self link",
        }
    }

    pub fn value(self, context: &GcloudContext, instance: &ComputeInstance) -> Option<String> {
        match self {
            Self::Name => Some(instance.name.clone()),
            Self::Zone => Some(instance.zone.clone()),
            Self::InternalIp => instance.internal_ip.clone(),
            Self::ExternalIp => instance.external_ip.clone(),
            Self::SelfLink => context.project.as_ref().map(|project| {
                format!(
                    "https://www.googleapis.com/compute/v1/projects/{project}/zones/{}/instances/{}",
                    instance.zone, instance.name
                )
            }),
        }
    }
}

/// A pending, confirmed-before-execution lifecycle action against a GCE
/// instance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComputeActionConfirmation {
    pub(super) action: ComputeInstanceAction,
    pub(super) context: GcloudContext,
    pub(super) instance: ComputeInstance,
}

impl ComputeActionConfirmation {
    pub fn action(&self) -> ComputeInstanceAction {
        self.action
    }

    pub fn context(&self) -> &GcloudContext {
        &self.context
    }

    pub fn instance(&self) -> &ComputeInstance {
        &self.instance
    }

    /// The call this confirmation is for.
    ///
    /// What the modal renders. The runtime runs the argv derived from the
    /// same builder, so the two cannot disagree about what is about to
    /// happen.
    pub fn call(&self) -> ResourceCall {
        compute_lifecycle_call(
            &self.context,
            &self.instance.name,
            &self.instance.zone,
            self.action,
        )
    }

    pub fn impact_warning(&self) -> &'static str {
        self.action.impact_warning()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SshLaunchPreview {
    context: GcloudContext,
    instance: ComputeInstance,
}

impl SshLaunchPreview {
    pub fn context(&self) -> &GcloudContext {
        &self.context
    }
    pub fn instance(&self) -> &ComputeInstance {
        &self.instance
    }
}

impl ResourcesState {
    pub fn compute(
        &self,
    ) -> &ResourceCollectionState<
        ComputeInstance,
        ComputeInstanceDetail,
        ComputeInstanceSortField,
        ComputeDetailTab,
    > {
        &self.compute
    }

    pub fn compute_mut(
        &mut self,
    ) -> &mut ResourceCollectionState<
        ComputeInstance,
        ComputeInstanceDetail,
        ComputeInstanceSortField,
        ComputeDetailTab,
    > {
        &mut self.compute
    }

    pub fn instances(&self) -> &[ComputeInstance] {
        &self.compute.rows
    }

    pub fn visible_instances(&self) -> Vec<&ComputeInstance> {
        self.visible_rows::<Compute>()
    }

    /// Returns the number of instances matching the applied filter.
    pub fn visible_instance_count(&self) -> usize {
        let filter = self.compute.filter_applied.filter();
        self.compute
            .rows
            .iter()
            .filter(|instance| instance_matches_filter(instance, filter))
            .count()
    }

    pub fn selected_instance(&self) -> Option<&ComputeInstance> {
        self.selected_visible_instance()
    }

    pub fn selected_visible_instance(&self) -> Option<&ComputeInstance> {
        self.visible_instances()
            .get(self.compute.selected_visible)
            .copied()
    }

    pub fn set_instances(
        &mut self,
        context: GcloudContext,
        instances: Vec<ComputeInstance>,
    ) -> Option<String> {
        if self.compute.context.as_ref() != Some(&context) {
            self.command_preview = None;
            self.ssh_launch_preview = None;
            self.tunnel_launch_preview = None;
            self.k9s_launch_confirmation = None;
        }
        let prev_selected = self
            .selected_visible_instance()
            .map(|i| (i.name.clone(), i.zone.clone()));
        let detail_open = self.compute.view_mode == ResourcesViewMode::Detail;
        let detail_key = self
            .compute
            .detail_target
            .as_ref()
            .map(|i| (i.name.clone(), i.zone.clone()));

        self.compute.context = Some(context);
        self.compute.rows = instances;
        self.compute.stale = false;
        self.compute.last_problem = None;
        self.compute.last_diagnostic = None;
        self.compute.clear_load_phase();

        self.prune_marks::<Compute>();

        let mut dropped_detail = None;
        if detail_open {
            let still_present = detail_key
                .as_ref()
                .map(|(n, z)| {
                    self.compute
                        .rows
                        .iter()
                        .any(|i| &i.name == n && &i.zone == z)
                })
                .unwrap_or(false);
            if !still_present {
                dropped_detail = detail_key.map(|(name, _)| name);
                self.close_compute_detail();
            }
        } else {
            self.close_compute_detail();
        }

        self.compute.selected_visible = prev_selected
            .as_ref()
            .and_then(|(n, z)| {
                self.visible_instances()
                    .iter()
                    .position(|i| &i.name == n && &i.zone == z)
            })
            .unwrap_or_else(|| {
                self.compute
                    .selected_visible
                    .min(self.visible_instances().len().saturating_sub(1))
            });

        dropped_detail
    }

    pub fn mark_stale_and_clear(&mut self) {
        self.compute.filter_draft = self.compute.filter_applied.source().to_string();
        self.compute.filter_input_active = false;
        self.compute.rows.clear();
        self.compute.context = None;
        self.compute.selected_visible = 0;
        self.compute.stale = true;
        self.clear_marks_for::<Compute>();
        // Only a Compute confirmation snapshots the rows just discarded; one
        // opened by another service is left standing.
        self.close_confirmations_owned_by(ResourceServiceKind::ComputeInstances);
        self.command_preview = None;
        self.ssh_launch_preview = None;
        self.tunnel_launch_preview = None;
        self.k9s_launch_confirmation = None;
        self.close_compute_detail();
    }

    /// Returns marked instances in visible sorted order, followed by any
    /// hidden (filtered-out) marked instances in stable order.
    pub fn marked_instances(&self) -> Vec<ComputeInstance> {
        self.marked_rows::<Compute>()
    }

    /// Returns true when the instance identified by `(name, zone)` is marked.
    pub fn is_instance_marked(&self, name: &str, zone: &str) -> bool {
        self.compute.marks.contains(&ResourceRowId {
            name: name.to_string(),
            location: zone.to_string(),
        })
    }

    pub fn compute_loaded(&self) -> bool {
        self.compute.context.is_some()
    }

    pub fn compute_stale(&self) -> bool {
        self.compute.stale
    }

    pub fn compute_problem(&self) -> Option<&DiagnosticProblem> {
        self.compute.last_problem.as_ref()
    }

    pub fn compute_status_counts(&self) -> Vec<(String, usize)> {
        count_labels(
            self.compute
                .rows
                .iter()
                .map(|i| status_count_label(&i.status)),
        )
    }

    pub fn start_action_confirmation(
        &mut self,
        context: GcloudContext,
        instance: ComputeInstance,
        action: ComputeInstanceAction,
    ) {
        self.close_other_modals();
        self.action_confirmation = Some(ResourceActionConfirmation::Compute(
            ComputeActionConfirmation {
                action,
                context,
                instance,
            },
        ));
    }

    pub fn show_ssh_command_preview(&mut self, context: GcloudContext, instance: ComputeInstance) {
        self.show_command_preview(
            "Show SSH command".to_string(),
            format!(
                "gcloud --configuration {} compute ssh {} --zone {}",
                context.configuration, instance.name, instance.zone
            ),
            "SSH command".to_string(),
        );
    }

    pub fn ssh_launch_preview(&self) -> Option<&SshLaunchPreview> {
        self.ssh_launch_preview.as_ref()
    }

    pub fn show_ssh_launch_preview(&mut self, context: GcloudContext, instance: ComputeInstance) {
        self.close_other_modals();
        self.ssh_launch_preview = Some(SshLaunchPreview { context, instance });
    }

    pub fn cancel_ssh_launch_preview(&mut self) {
        self.ssh_launch_preview = None;
        self.tunnel_launch_preview = None;
        self.k9s_launch_confirmation = None;
    }

    /// The launch request for the currently-previewed instance, if any.
    pub fn ssh_launch_submission(&self) -> Option<(GcloudContext, ComputeInstance)> {
        self.ssh_launch_preview
            .as_ref()
            .map(|preview| (preview.context.clone(), preview.instance.clone()))
    }

    pub fn open_detail(&mut self) -> Option<ComputeInstance> {
        let instance = self.selected_visible_instance()?.clone();
        self.command_preview = None;
        self.ssh_launch_preview = None;
        self.tunnel_launch_preview = None;
        self.k9s_launch_confirmation = None;
        self.compute.view_mode = ResourcesViewMode::Detail;
        self.compute.active_detail_tab = ComputeDetailTab::Summary;
        self.compute.detail_scroll = 0;
        self.detail_field_index = None;
        self.compute.detail_target = Some(instance.clone());
        self.compute.detail_error_target_name = None;
        self.compute.detail = None;
        self.compute.detail_error = None;
        Some(instance)
    }

    pub fn detail(&self) -> Option<&ComputeInstanceDetail> {
        self.compute.detail.as_ref()
    }

    pub fn set_detail(&mut self, detail: ComputeInstanceDetail) {
        self.compute.detail_error_target_name = None;
        self.compute.detail_error = None;
        self.compute.detail = Some(detail);
    }

    pub fn set_detail_error(&mut self, instance_name: String, error: String) {
        self.compute.view_mode = ResourcesViewMode::Detail;
        self.compute.detail = None;
        self.compute.detail_error_target_name = Some(instance_name);
        self.compute.detail_error = Some(error);
    }

    pub fn detail_target(&self) -> Option<&ComputeInstance> {
        self.compute.detail_target.as_ref()
    }

    pub fn detail_target_name(&self) -> Option<&str> {
        self.compute
            .detail_target
            .as_ref()
            .map(|instance| instance.name.as_str())
            .or_else(|| {
                self.compute
                    .detail
                    .as_ref()
                    .map(|detail| detail.name.as_str())
            })
            .or(self.compute.detail_error_target_name.as_deref())
    }

    pub fn is_active_detail_target(&self, instance_name: &str, instance_zone: &str) -> bool {
        self.compute
            .detail_target
            .as_ref()
            .map(|instance| instance.name == instance_name && instance.zone == instance_zone)
            .unwrap_or(false)
    }

    pub fn detail_error(&self) -> Option<&str> {
        self.compute.detail_error.as_deref()
    }

    pub fn active_detail_tab(&self) -> ComputeDetailTab {
        self.compute.active_detail_tab
    }

    pub fn restore_compute_detail(
        &mut self,
        name: &str,
        zone: &str,
        tab: ComputeDetailTab,
    ) -> Option<ComputeInstance> {
        let instance = self
            .compute
            .rows
            .iter()
            .find(|i| i.name == name && i.zone == zone)
            .cloned()?;
        if let Some(index) = self
            .visible_instances()
            .iter()
            .position(|i| i.name == name && i.zone == zone)
        {
            self.compute.selected_visible = index;
        }
        self.compute.view_mode = ResourcesViewMode::Detail;
        self.compute.active_detail_tab = tab;
        self.compute.detail_scroll = 0;
        self.detail_field_index = None;
        self.compute.detail_target = Some(instance.clone());
        self.compute.detail_error_target_name = None;
        self.compute.detail = None;
        self.compute.detail_error = None;
        Some(instance)
    }

    pub(super) fn close_compute_detail(&mut self) {
        // Sole closer for this pane (close, service list, stale/context teardown,
        // and a refresh that drops the open target), so the field cursor dies
        // with the pane on every one of those paths.
        self.detail_field_index = None;
        self.compute.view_mode = ResourcesViewMode::List;
        self.compute.active_detail_tab = ComputeDetailTab::Summary;
        self.compute.detail_target = None;
        self.compute.detail_error_target_name = None;
        self.compute.detail = None;
        self.compute.detail_error = None;
    }
}

fn instance_matches_filter(instance: &ComputeInstance, filter: &CompiledFilter) -> bool {
    filter.matches(&searchable_fields(instance))
}

/// The values the `/` filter searches for one row.
///
/// Status contributes its *display label* plus, for
/// [`ResourceStatus::Unknown`], the raw status string it carries — never the
/// `{:?}` rendering. The Debug shell (`Unknown("REPAIRING")`) appears in no
/// raw gcloud field and in no entry of the compute service's `OPAQUE_LABELS`,
/// so a filter matching the shell alone would slip through the server-side
/// pushdown gate and drop the row. The label is enumerated in
/// `OPAQUE_LABELS`, and the raw string is stored verbatim from the JSON
/// `status` field (see `parse_status` in `crate::services::resources::compute`)
/// which is itself pushed, so both satisfy the never-drop-a-row premise
/// documented in `crate::services::resources::list_filter`.
fn searchable_fields(instance: &ComputeInstance) -> Vec<String> {
    let mut fields = vec![
        instance.name.clone(),
        instance.zone.clone(),
        resource_status_label(&instance.status),
        instance.machine_type.clone(),
        instance.internal_ip.clone().unwrap_or_default(),
        instance.external_ip.clone().unwrap_or_default(),
    ];
    if let ResourceStatus::Unknown(raw) = &instance.status {
        fields.push(raw.clone());
    }
    fields
}

fn compare_instances(
    left: &ComputeInstance,
    right: &ComputeInstance,
    sort_field: ComputeInstanceSortField,
    direction: SortDirection,
) -> std::cmp::Ordering {
    let primary = match sort_field {
        ComputeInstanceSortField::Name => left.name.cmp(&right.name),
        ComputeInstanceSortField::Status => {
            format!("{:?}", left.status).cmp(&format!("{:?}", right.status))
        }
        ComputeInstanceSortField::Zone => left.zone.cmp(&right.zone),
        ComputeInstanceSortField::MachineType => left.machine_type.cmp(&right.machine_type),
    };
    let ordering = primary
        .then_with(|| left.name.cmp(&right.name))
        .then_with(|| left.zone.cmp(&right.zone));
    match direction {
        SortDirection::Ascending => ordering,
        SortDirection::Descending => ordering.reverse(),
    }
}

/// Status label used for the compute status-count summary: `Unknown` collapses
/// to a bare label so every unrecognised status groups into one bucket. Kept
/// distinct from `batch::resource_status_label`, which preserves the raw value
/// for per-row display.
fn status_count_label(status: &ResourceStatus) -> String {
    match status {
        ResourceStatus::Running => "Running".to_string(),
        ResourceStatus::Stopped => "Stopped".to_string(),
        ResourceStatus::Suspended => "Suspended".to_string(),
        ResourceStatus::Pending => "Pending".to_string(),
        ResourceStatus::Unknown(_) => "Unknown".to_string(),
    }
}
