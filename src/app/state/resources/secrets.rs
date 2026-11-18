use super::*;

/// Marker type unifying the Secret Manager secrets service behind
/// [`ResourceService`]. Never instantiated.
pub enum Secrets {}

impl ResourceService for Secrets {
    type Row = Secret;
    type Detail = SecretDetail;
    type SortField = SecretSortField;
    type DetailTab = SecretDetailTab;

    const KIND: ResourceServiceKind = ResourceServiceKind::SecretManagerSecrets;

    fn collection(
        state: &ResourcesState,
    ) -> &ResourceCollectionState<Self::Row, Self::Detail, Self::SortField, Self::DetailTab> {
        &state.secrets
    }

    fn collection_mut(
        state: &mut ResourcesState,
    ) -> &mut ResourceCollectionState<Self::Row, Self::Detail, Self::SortField, Self::DetailTab>
    {
        &mut state.secrets
    }

    fn matches_filter(row: &Self::Row, filter: &CompiledFilter) -> bool {
        secret_matches_filter(row, filter)
    }

    /// The needle sits in the name, the identity column of the secret table —
    /// and the only field of a secret this crate ever renders as text, since
    /// a payload is never read.
    fn sample_row() -> Self::Row {
        Secret {
            name: SAMPLE_ROW_NEEDLE.to_string(),
            create_time: None,
            labels: Vec::new(),
            replication: "automatic".to_string(),
        }
    }

    fn compare(
        left: &Self::Row,
        right: &Self::Row,
        sort_field: Self::SortField,
        direction: SortDirection,
    ) -> std::cmp::Ordering {
        compare_secrets(left, right, sort_field, direction)
    }

    /// Secret Manager is metadata-browsing only — no lifecycle actions, so no
    /// marks. This exists to keep [`ResourceService`] total.
    ///
    /// A secret is a global resource with no location of its own, so the
    /// location half of the id is empty. The name alone is unique within a
    /// project, which is what row identity needs.
    fn row_id(row: &Self::Row) -> ResourceRowId {
        ResourceRowId {
            name: row.name.clone(),
            location: String::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecretSortField {
    Name,
    Created,
    Replication,
}

impl SecretSortField {
    pub fn label(self) -> &'static str {
        match self {
            Self::Name => "Name",
            Self::Created => "Created",
            Self::Replication => "Replication",
        }
    }

    pub const COLUMN_COUNT: usize = 4;

    pub fn from_column(index: usize) -> Option<Self> {
        match index {
            0 => Some(Self::Name),
            1 => Some(Self::Created),
            2 => Some(Self::Replication),
            _ => None,
        }
    }

    pub fn column(self) -> usize {
        match self {
            Self::Name => 0,
            Self::Created => 1,
            Self::Replication => 2,
        }
    }
}

/// The tabs of the secret detail pane.
///
/// `Versions` is where a secret's versions — and therefore its version count —
/// live. They are deliberately not on the list row: the Secret resource carries
/// no version count, so putting one in the table would cost an extra call per
/// secret on every list load. Here they are one on-demand fetch for the one
/// secret the operator actually opened.
///
/// There is no per-version detail tab and no version drill-down. A version has
/// three metadata fields and nothing to drill into — the only thing a deeper
/// view could add is the payload, which this binary structurally cannot fetch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecretDetailTab {
    Summary,
    Versions,
    Labels,
    Json,
}

impl SecretDetailTab {
    pub const ALL: [Self; 4] = [Self::Summary, Self::Versions, Self::Labels, Self::Json];

    pub fn label(self) -> &'static str {
        match self {
            Self::Summary => "Summary",
            Self::Versions => "Versions",
            Self::Labels => "Labels",
            Self::Json => "JSON",
        }
    }
}

impl ResourcesState {
    pub fn secrets(&self) -> &[Secret] {
        &self.secrets.rows
    }

    pub fn visible_secrets(&self) -> Vec<&Secret> {
        let filter = self.secrets.filter_applied.filter();
        let mut rows = self
            .secrets
            .rows
            .iter()
            .filter(|secret| secret_matches_filter(secret, filter))
            .collect::<Vec<_>>();
        rows.sort_by(|left, right| {
            compare_secrets(
                left,
                right,
                self.secrets.sort_field,
                self.secrets.sort_direction,
            )
        });
        rows
    }

    pub fn visible_secret_count(&self) -> usize {
        let filter = self.secrets.filter_applied.filter();
        self.secrets
            .rows
            .iter()
            .filter(|secret| secret_matches_filter(secret, filter))
            .count()
    }

    pub fn selected_secret(&self) -> Option<&Secret> {
        self.visible_secrets()
            .get(self.secrets.selected_visible)
            .copied()
    }

    pub fn set_secrets(&mut self, context: GcloudContext, secrets: Vec<Secret>) -> Option<String> {
        let prev_selected = self.selected_secret().map(|secret| secret.name.clone());
        let detail_open = self.secrets.view_mode == ResourcesViewMode::Detail;
        let detail_key = self
            .secrets
            .detail_target
            .as_ref()
            .map(|secret| secret.name.clone());

        self.secrets.context = Some(context);
        self.secrets.rows = secrets;
        self.secrets.stale = false;
        self.secrets.last_problem = None;
        self.secrets.last_diagnostic = None;
        self.secrets.clear_load_phase();

        let mut dropped_detail = None;
        if detail_open {
            let still_present = detail_key
                .as_ref()
                .map(|name| self.secrets.rows.iter().any(|secret| &secret.name == name))
                .unwrap_or(false);
            if !still_present {
                dropped_detail = detail_key;
                self.close_secret_detail();
            }
        } else {
            self.close_secret_detail();
        }

        self.secrets.selected_visible = prev_selected
            .as_ref()
            .and_then(|name| {
                self.visible_secrets()
                    .iter()
                    .position(|secret| &secret.name == name)
            })
            .unwrap_or_else(|| {
                self.secrets
                    .selected_visible
                    .min(self.visible_secrets().len().saturating_sub(1))
            });

        dropped_detail
    }

    pub fn mark_secrets_stale_and_clear(&mut self) {
        self.secrets.filter_draft = self.secrets.filter_applied.source().to_string();
        self.secrets.filter_input_active = false;
        self.secrets.rows.clear();
        self.secrets.context = None;
        self.secrets.selected_visible = 0;
        self.secrets.stale = true;
        self.close_secret_detail();
    }

    pub fn secrets_problem(&self) -> Option<&DiagnosticProblem> {
        self.secrets.last_problem.as_ref()
    }

    pub fn open_secret_detail(&mut self) -> Option<Secret> {
        let secret = self.selected_secret()?.clone();
        self.secrets.view_mode = ResourcesViewMode::Detail;
        self.secrets.active_detail_tab = SecretDetailTab::Summary;
        self.secrets.detail_scroll = 0;
        self.detail_field_index = None;
        self.secrets.detail_target = Some(secret.clone());
        self.secrets.detail_error_target_name = None;
        self.secrets.detail = None;
        self.secrets.detail_error = None;
        self.secret_versions_error = None;
        Some(secret)
    }

    pub fn active_secret_detail_tab(&self) -> SecretDetailTab {
        self.secrets.active_detail_tab
    }

    pub fn secret_detail(&self) -> Option<&SecretDetail> {
        self.secrets.detail.as_ref()
    }

    pub fn set_secret_detail(&mut self, detail: SecretDetail) {
        self.secrets.detail_error_target_name = None;
        self.secrets.detail_error = None;
        self.secrets.detail = Some(detail);
    }

    /// Installs a secret's versions onto the open detail.
    ///
    /// Versions arrive from their own fetch, after the describe that opened the
    /// pane, so this merges rather than replaces: dropping the rest of the
    /// detail to deliver versions would blank the Summary tab the operator is
    /// looking at.
    pub fn set_secret_versions(&mut self, versions: Vec<SecretVersion>) {
        self.secret_versions_error = None;
        if let Some(detail) = self.secrets.detail.as_mut() {
            detail.versions = versions;
        }
    }

    /// Records why the open secret's versions could not be listed.
    ///
    /// The describe that filled the pane and this list are separate calls
    /// behind separate IAM permissions, so the detail is left alone: the
    /// Summary tab keeps rendering what did load, and only the version count
    /// and the Versions tab report the failure.
    pub fn set_secret_versions_error(&mut self, error: String) {
        self.secret_versions_error = Some(error);
    }

    pub fn secret_versions_error(&self) -> Option<&str> {
        self.secret_versions_error.as_deref()
    }

    pub fn set_secret_detail_error(&mut self, secret_name: String, error: String) {
        self.secrets.view_mode = ResourcesViewMode::Detail;
        self.secrets.detail = None;
        self.secrets.detail_error_target_name = Some(secret_name);
        self.secrets.detail_error = Some(error);
    }

    pub fn secret_detail_target(&self) -> Option<&Secret> {
        self.secrets.detail_target.as_ref()
    }

    pub fn secret_detail_target_name(&self) -> Option<&str> {
        self.secrets
            .detail_target
            .as_ref()
            .map(|secret| secret.name.as_str())
            .or_else(|| {
                self.secrets
                    .detail
                    .as_ref()
                    .map(|detail| detail.name.as_str())
            })
            .or(self.secrets.detail_error_target_name.as_deref())
    }

    pub fn is_active_secret_detail_target(&self, secret_name: &str) -> bool {
        self.secrets
            .detail_target
            .as_ref()
            .map(|secret| secret.name == secret_name)
            .unwrap_or(false)
            || self.secrets.detail_error_target_name.as_deref() == Some(secret_name)
    }

    pub fn secret_detail_error(&self) -> Option<&str> {
        self.secrets.detail_error.as_deref()
    }

    pub(super) fn close_secret_detail(&mut self) {
        self.detail_field_index = None;
        self.secret_versions_error = None;
        self.secrets.view_mode = ResourcesViewMode::List;
        self.secrets.active_detail_tab = SecretDetailTab::Summary;
        self.secrets.detail_target = None;
        self.secrets.detail_error_target_name = None;
        self.secrets.detail = None;
        self.secrets.detail_error = None;
    }
}

/// The values a secrets filter searches.
///
/// Labels are included as `key=value` because that is how an operator thinks of
/// them and how the label filter grammar spells them, so a plain substring
/// search for `env=prod` finds the same rows the grammar would.
fn secret_searchable_fields(secret: &Secret) -> Vec<String> {
    let mut fields = vec![
        secret.name.clone(),
        secret.replication.clone(),
        secret.create_time.clone().unwrap_or_default(),
    ];
    fields.extend(
        secret
            .labels
            .iter()
            .map(|label| format!("{}={}", label.key, label.value)),
    );
    fields
}

fn secret_matches_filter(secret: &Secret, filter: &CompiledFilter) -> bool {
    filter.matches(&secret_searchable_fields(secret))
}

fn compare_secrets(
    left: &Secret,
    right: &Secret,
    sort_field: SecretSortField,
    direction: SortDirection,
) -> std::cmp::Ordering {
    let primary = match sort_field {
        SecretSortField::Name => left.name.cmp(&right.name),
        SecretSortField::Created => left
            .create_time
            .as_deref()
            .unwrap_or_default()
            .cmp(right.create_time.as_deref().unwrap_or_default()),
        SecretSortField::Replication => left.replication.cmp(&right.replication),
    };
    let ordering = primary.then_with(|| left.name.cmp(&right.name));
    match direction {
        SortDirection::Ascending => ordering,
        SortDirection::Descending => ordering.reverse(),
    }
}
