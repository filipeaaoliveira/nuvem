use crate::config::favorites::{FavoriteKind, FavoriteScopes};
use crate::services::gcloud::configurations::{GcloudConfigurationDraft, GcloudContextEditDraft};
use nuvem_domain::gcp::configuration::{GcloudConfiguration, GcloudConfigurationName};
use nuvem_domain::gcp::location::{ComputeRegion, ComputeZone};
use nuvem_domain::gcp::project::GcloudProject;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigurationCreateField {
    Name,
    Account,
    Project,
    Region,
    Zone,
}

impl ConfigurationCreateField {
    pub const ALL: [Self; 5] = [
        Self::Name,
        Self::Account,
        Self::Project,
        Self::Region,
        Self::Zone,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Name => "Name",
            Self::Account => "Account",
            Self::Project => "Project",
            Self::Region => "Region",
            Self::Zone => "Zone",
        }
    }

    pub fn next(self) -> Self {
        match self {
            Self::Name => Self::Account,
            Self::Account => Self::Project,
            Self::Project => Self::Region,
            Self::Region => Self::Zone,
            Self::Zone => Self::Name,
        }
    }

    pub fn previous(self) -> Self {
        match self {
            Self::Name => Self::Zone,
            Self::Account => Self::Name,
            Self::Project => Self::Account,
            Self::Region => Self::Project,
            Self::Zone => Self::Region,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigurationCreateForm {
    draft: GcloudConfigurationDraft,
    focused_field: ConfigurationCreateField,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigurationContextEditField {
    Project,
    Region,
    Zone,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigurationContextPickerKind {
    Project,
    Region,
    Zone,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigurationContextPickerItem {
    Project(GcloudProject),
    Region(ComputeRegion),
    Zone(ComputeZone),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigurationContextPickerStatus {
    Loading,
    Ready,
    Empty,
    Degraded(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigurationContextPicker {
    target: GcloudConfigurationName,
    kind: ConfigurationContextPickerKind,
    rows: Vec<ConfigurationContextPickerItem>,
    filter_input_active: bool,
    filter_draft: String,
    filter_applied: String,
    selected_visible: usize,
    status: ConfigurationContextPickerStatus,
    initial_value: String,
    /// The current user's favorited values for this picker's own kind, in
    /// source order. Compared against [`ConfigurationContextPickerItem::value`]
    /// to place favorites ahead of the rest in [`Self::visible_items`].
    favorites: Vec<String>,
}

impl ConfigurationContextEditField {
    pub const ALL: [Self; 3] = [Self::Project, Self::Region, Self::Zone];

    pub fn label(self) -> &'static str {
        match self {
            Self::Project => "Project",
            Self::Region => "Region",
            Self::Zone => "Zone",
        }
    }

    pub fn next(self) -> Self {
        match self {
            Self::Project => Self::Region,
            Self::Region => Self::Zone,
            Self::Zone => Self::Project,
        }
    }

    pub fn previous(self) -> Self {
        match self {
            Self::Project => Self::Zone,
            Self::Region => Self::Project,
            Self::Zone => Self::Region,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigurationContextEditForm {
    target: GcloudConfigurationName,
    draft: GcloudContextEditDraft,
    focused_field: ConfigurationContextEditField,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigurationActivationConfirmation {
    target: GcloudConfiguration,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActivationConfirmationRequest {
    Started,
    AlreadyActive,
    NoSelection,
}

impl ConfigurationActivationConfirmation {
    pub fn new(target: GcloudConfiguration) -> Self {
        Self { target }
    }

    pub fn target(&self) -> &GcloudConfiguration {
        &self.target
    }

    pub fn command_preview(&self) -> String {
        format!(
            "gcloud config configurations activate {}",
            self.target.name.as_str()
        )
    }
}

impl Default for ConfigurationCreateForm {
    fn default() -> Self {
        Self {
            draft: GcloudConfigurationDraft {
                name: String::new(),
                account: None,
                project: None,
                region: None,
                zone: None,
            },
            focused_field: ConfigurationCreateField::Name,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ConfigurationsState {
    rows: Vec<GcloudConfiguration>,
    selected_visible: usize,
    filter_input_active: bool,
    filter_draft: String,
    filter_applied: String,
    create_form: Option<ConfigurationCreateForm>,
    context_edit_form: Option<ConfigurationContextEditForm>,
    activation_confirmation: Option<ConfigurationActivationConfirmation>,
    context_picker: Option<ConfigurationContextPicker>,
    /// The current user's favorited project ids, region names and zone
    /// names, from the config file's `[favorites]` table. Set at startup and
    /// on reload, beside `aliases` and `jump_rules`; empty when no config
    /// file (or no `[favorites]` table) is present.
    favorites: FavoriteScopes,
}

impl From<ConfigurationContextEditField> for ConfigurationContextPickerKind {
    fn from(field: ConfigurationContextEditField) -> Self {
        match field {
            ConfigurationContextEditField::Project => Self::Project,
            ConfigurationContextEditField::Region => Self::Region,
            ConfigurationContextEditField::Zone => Self::Zone,
        }
    }
}

impl From<ConfigurationContextPickerKind> for FavoriteKind {
    fn from(kind: ConfigurationContextPickerKind) -> Self {
        match kind {
            ConfigurationContextPickerKind::Project => Self::Project,
            ConfigurationContextPickerKind::Region => Self::Region,
            ConfigurationContextPickerKind::Zone => Self::Zone,
        }
    }
}

impl ConfigurationContextPickerItem {
    fn value(&self) -> &str {
        match self {
            Self::Project(project) => &project.project_id,
            Self::Region(region) => &region.name,
            Self::Zone(zone) => &zone.name,
        }
    }

    fn matches_filter(&self, filter: &str) -> bool {
        let fields: Vec<&str> = match self {
            Self::Project(project) => vec![
                &project.project_id,
                &project.name,
                project.lifecycle_state.as_deref().unwrap_or(""),
            ],
            Self::Region(region) => vec![&region.name, region.status.as_deref().unwrap_or("")],
            Self::Zone(zone) => vec![
                &zone.name,
                zone.region.as_deref().unwrap_or(""),
                zone.status.as_deref().unwrap_or(""),
            ],
        };
        fields
            .into_iter()
            .any(|value| value.to_lowercase().contains(filter))
    }
}

impl ConfigurationContextPicker {
    fn new(
        target: GcloudConfigurationName,
        kind: ConfigurationContextPickerKind,
        initial_value: String,
        favorites: Vec<String>,
    ) -> Self {
        Self {
            target,
            kind,
            rows: Vec::new(),
            filter_input_active: false,
            filter_draft: String::new(),
            filter_applied: String::new(),
            selected_visible: 0,
            status: ConfigurationContextPickerStatus::Loading,
            initial_value,
            favorites,
        }
    }

    pub fn kind(&self) -> ConfigurationContextPickerKind {
        self.kind
    }
    pub fn target(&self) -> &GcloudConfigurationName {
        &self.target
    }
    pub fn status(&self) -> &ConfigurationContextPickerStatus {
        &self.status
    }
    pub fn is_filter_input_active(&self) -> bool {
        self.filter_input_active
    }
    pub fn filter_draft(&self) -> &str {
        &self.filter_draft
    }

    pub fn visible_items(&self) -> Vec<&ConfigurationContextPickerItem> {
        let filter = self.filter_draft.trim().to_lowercase();
        let filtered = self
            .rows
            .iter()
            .filter(|item| filter.is_empty() || item.matches_filter(&filter));

        // Stable partition: favorites keep their source order, then the rest
        // keep theirs. A comparison sort would not do this — it would reorder
        // within a group whenever the comparator judged two items equal.
        let mut favorites = Vec::new();
        let mut rest = Vec::new();
        for item in filtered {
            if self.is_favorite(item) {
                favorites.push(item);
            } else {
                rest.push(item);
            }
        }
        favorites.extend(rest);
        favorites
    }

    /// Whether `item` is one of the current user's favorites for this
    /// picker's kind. Exposed so the overlay can mark favorite rows.
    pub fn is_favorite(&self, item: &ConfigurationContextPickerItem) -> bool {
        self.favorites.iter().any(|value| value == item.value())
    }

    pub fn selected_item(&self) -> Option<&ConfigurationContextPickerItem> {
        if self.status != ConfigurationContextPickerStatus::Ready {
            return None;
        }
        self.visible_items().get(self.selected_visible).copied()
    }

    pub fn start_filtering(&mut self) {
        self.filter_draft = self.filter_applied.clone();
        self.filter_input_active = true;
    }

    pub fn update_filter(&mut self, value: String) {
        if self.filter_input_active {
            let selected = self.selected_value();
            self.filter_draft = value;
            self.reselect(selected);
        }
    }

    pub fn apply_filter(&mut self) {
        if self.filter_input_active {
            self.filter_applied = self.filter_draft.trim().to_string();
            self.filter_draft = self.filter_applied.clone();
            self.filter_input_active = false;
            self.clamp_selected_visible();
        }
    }

    pub fn cancel_filter(&mut self) {
        let selected = self.selected_value();
        self.filter_draft = self.filter_applied.clone();
        self.filter_input_active = false;
        self.reselect(selected);
    }

    pub fn select_next(&mut self) {
        let len = self.visible_items().len();
        if len > 0 {
            self.selected_visible = (self.selected_visible + 1).min(len - 1);
        }
    }

    pub fn select_previous(&mut self) {
        self.selected_visible = self.selected_visible.saturating_sub(1);
    }

    pub fn set_loaded(&mut self, rows: Vec<ConfigurationContextPickerItem>) {
        self.rows = rows;
        self.status = if self.rows.is_empty() {
            ConfigurationContextPickerStatus::Empty
        } else {
            ConfigurationContextPickerStatus::Ready
        };
        self.selected_visible = self
            .visible_items()
            .iter()
            .position(|item| item.value() == self.initial_value)
            .unwrap_or(0);
        self.clamp_selected_visible();
    }

    pub fn set_degraded(&mut self, error: String) {
        self.status = ConfigurationContextPickerStatus::Degraded(error);
    }

    pub fn retry(&mut self) {
        self.selected_visible = 0;
        self.status = ConfigurationContextPickerStatus::Loading;
    }

    /// The value of the currently selected row, or `None` when nothing is
    /// visible. Taken *before* a filter change so [`Self::reselect`] can find
    /// the same item again afterwards.
    fn selected_value(&self) -> Option<String> {
        self.visible_items()
            .get(self.selected_visible)
            .map(|item| item.value().to_string())
    }

    /// Puts the cursor back on `value` in the freshly ordered visible view.
    ///
    /// The selection is stored as an index into that view, so any filter
    /// change that adds or removes a row above the cursor would otherwise
    /// leave an in-bounds index pointing at a *different* item than the
    /// operator selected. Resolving by value instead is the same thing
    /// [`Self::set_loaded`] does for `initial_value`.
    ///
    /// Falls back to clamping when `value` is no longer visible (including
    /// when it was the row the filter just removed): there is no identity left
    /// to follow, and the index still has to stay in bounds.
    fn reselect(&mut self, value: Option<String>) {
        let found = value.and_then(|value| {
            self.visible_items()
                .iter()
                .position(|item| item.value() == value)
        });
        match found {
            Some(index) => self.selected_visible = index,
            None => self.clamp_selected_visible(),
        }
    }

    fn clamp_selected_visible(&mut self) {
        self.selected_visible = self
            .selected_visible
            .min(self.visible_items().len().saturating_sub(1));
    }
}

impl ConfigurationCreateForm {
    pub fn draft(&self) -> &GcloudConfigurationDraft {
        &self.draft
    }

    pub fn focused_field(&self) -> ConfigurationCreateField {
        self.focused_field
    }

    pub fn field_value(&self, field: ConfigurationCreateField) -> &str {
        match field {
            ConfigurationCreateField::Name => &self.draft.name,
            ConfigurationCreateField::Account => self.draft.account.as_deref().unwrap_or(""),
            ConfigurationCreateField::Project => self.draft.project.as_deref().unwrap_or(""),
            ConfigurationCreateField::Region => self.draft.region.as_deref().unwrap_or(""),
            ConfigurationCreateField::Zone => self.draft.zone.as_deref().unwrap_or(""),
        }
    }

    fn append_char(&mut self, value: char) {
        if value.is_control() {
            return;
        }
        match self.focused_field {
            ConfigurationCreateField::Name => self.draft.name.push(value),
            ConfigurationCreateField::Account => append_optional(&mut self.draft.account, value),
            ConfigurationCreateField::Project => append_optional(&mut self.draft.project, value),
            ConfigurationCreateField::Region => append_optional(&mut self.draft.region, value),
            ConfigurationCreateField::Zone => append_optional(&mut self.draft.zone, value),
        }
    }

    fn backspace(&mut self) {
        match self.focused_field {
            ConfigurationCreateField::Name => {
                self.draft.name.pop();
            }
            ConfigurationCreateField::Account => backspace_optional(&mut self.draft.account),
            ConfigurationCreateField::Project => backspace_optional(&mut self.draft.project),
            ConfigurationCreateField::Region => backspace_optional(&mut self.draft.region),
            ConfigurationCreateField::Zone => backspace_optional(&mut self.draft.zone),
        }
    }

    fn focus_next(&mut self) {
        self.focused_field = self.focused_field.next();
    }

    fn focus_previous(&mut self) {
        self.focused_field = self.focused_field.previous();
    }

    fn focus_name(&mut self) {
        self.focused_field = ConfigurationCreateField::Name;
    }

    fn submit_draft(&self) -> Option<GcloudConfigurationDraft> {
        let name = self.draft.name.trim().to_string();
        if name.is_empty() {
            return None;
        }
        Some(GcloudConfigurationDraft {
            name,
            account: trimmed_optional(self.draft.account.as_deref()),
            project: trimmed_optional(self.draft.project.as_deref()),
            region: trimmed_optional(self.draft.region.as_deref()),
            zone: trimmed_optional(self.draft.zone.as_deref()),
        })
    }
}

impl ConfigurationContextEditForm {
    pub fn new(active: &GcloudConfiguration) -> Self {
        let target = active.name.clone();
        Self {
            target: target.clone(),
            draft: GcloudContextEditDraft {
                configuration: target,
                project: active.project.clone(),
                region: active.region.clone(),
                zone: active.zone.clone(),
            },
            focused_field: ConfigurationContextEditField::Project,
        }
    }

    pub fn target(&self) -> &GcloudConfigurationName {
        &self.target
    }

    pub fn draft(&self) -> &GcloudContextEditDraft {
        &self.draft
    }

    pub fn focused_field(&self) -> ConfigurationContextEditField {
        self.focused_field
    }

    fn focused_field_value(&self) -> &str {
        self.field_value(self.focused_field)
    }

    pub fn field_value(&self, field: ConfigurationContextEditField) -> &str {
        match field {
            ConfigurationContextEditField::Project => self.draft.project.as_deref().unwrap_or(""),
            ConfigurationContextEditField::Region => self.draft.region.as_deref().unwrap_or(""),
            ConfigurationContextEditField::Zone => self.draft.zone.as_deref().unwrap_or(""),
        }
    }

    fn append_char(&mut self, value: char) {
        if value.is_control() {
            return;
        }
        match self.focused_field {
            ConfigurationContextEditField::Project => {
                append_optional(&mut self.draft.project, value)
            }
            ConfigurationContextEditField::Region => append_optional(&mut self.draft.region, value),
            ConfigurationContextEditField::Zone => append_optional(&mut self.draft.zone, value),
        }
    }

    fn backspace(&mut self) {
        match self.focused_field {
            ConfigurationContextEditField::Project => backspace_optional(&mut self.draft.project),
            ConfigurationContextEditField::Region => backspace_optional(&mut self.draft.region),
            ConfigurationContextEditField::Zone => backspace_optional(&mut self.draft.zone),
        }
    }

    fn focus_next(&mut self) {
        self.focused_field = self.focused_field.next();
    }

    fn focus_previous(&mut self) {
        self.focused_field = self.focused_field.previous();
    }

    fn apply_picker_item(&mut self, item: ConfigurationContextPickerItem) -> bool {
        match (self.focused_field, item) {
            (
                ConfigurationContextEditField::Project,
                ConfigurationContextPickerItem::Project(project),
            ) => {
                self.draft.project = Some(project.project_id);
            }
            (
                ConfigurationContextEditField::Region,
                ConfigurationContextPickerItem::Region(region),
            ) => {
                self.draft.region = Some(region.name);
            }
            (ConfigurationContextEditField::Zone, ConfigurationContextPickerItem::Zone(zone)) => {
                self.draft.zone = Some(zone.name);
            }
            _ => return false,
        }
        true
    }

    fn submit_draft(&self) -> GcloudContextEditDraft {
        GcloudContextEditDraft {
            configuration: self.draft.configuration.clone(),
            project: trimmed_optional(self.draft.project.as_deref()),
            region: trimmed_optional(self.draft.region.as_deref()),
            zone: trimmed_optional(self.draft.zone.as_deref()),
        }
    }
}

fn append_optional(value: &mut Option<String>, ch: char) {
    value.get_or_insert_with(String::new).push(ch);
}

fn backspace_optional(value: &mut Option<String>) {
    if let Some(current) = value {
        current.pop();
        if current.is_empty() {
            *value = None;
        }
    }
}

fn trimmed_optional(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn configuration_matches_filter(configuration: &GcloudConfiguration, filter: &str) -> bool {
    configuration.name.as_str().to_lowercase().contains(filter)
        || configuration
            .account
            .as_deref()
            .is_some_and(|value| value.to_lowercase().contains(filter))
        || configuration
            .project
            .as_deref()
            .is_some_and(|value| value.to_lowercase().contains(filter))
        || configuration
            .region
            .as_deref()
            .is_some_and(|value| value.to_lowercase().contains(filter))
        || configuration
            .zone
            .as_deref()
            .is_some_and(|value| value.to_lowercase().contains(filter))
}

impl ConfigurationsState {
    pub fn rows(&self) -> &[GcloudConfiguration] {
        &self.rows
    }

    pub fn visible_rows(&self) -> Vec<&GcloudConfiguration> {
        let filter = self.filter_applied.trim().to_lowercase();
        self.rows
            .iter()
            .filter(|configuration| {
                filter.is_empty() || configuration_matches_filter(configuration, &filter)
            })
            .collect()
    }

    pub fn set_rows(&mut self, rows: Vec<GcloudConfiguration>) {
        self.rows = rows;
        self.activation_confirmation = None;
        self.clamp_selected_visible();
    }

    pub fn selected(&self) -> Option<&GcloudConfiguration> {
        self.visible_rows().get(self.selected_visible).copied()
    }

    pub fn selected_index(&self) -> Option<usize> {
        self.selected_visible_index()
    }

    pub fn selected_visible_index(&self) -> Option<usize> {
        (!self.visible_rows().is_empty()).then_some(self.selected_visible)
    }

    pub fn select_next(&mut self) {
        let len = self.visible_rows().len();
        if len > 0 {
            self.selected_visible = (self.selected_visible + 1).min(len - 1);
        }
    }

    pub fn select_previous(&mut self) {
        self.selected_visible = self.selected_visible.saturating_sub(1);
    }

    pub fn is_filter_input_active(&self) -> bool {
        self.filter_input_active
    }

    pub fn filter_draft(&self) -> &str {
        &self.filter_draft
    }

    pub fn filter_applied(&self) -> &str {
        &self.filter_applied
    }

    pub fn start_filtering(&mut self) {
        self.create_form = None;
        self.context_edit_form = None;
        self.activation_confirmation = None;
        self.context_picker = None;
        self.filter_input_active = true;
        self.filter_draft = self.filter_applied.clone();
    }

    pub fn update_filter(&mut self, value: String) {
        if self.filter_input_active {
            self.filter_draft = value;
        }
    }

    pub fn apply_filter(&mut self) {
        if self.filter_input_active {
            self.filter_applied = self.filter_draft.trim().to_string();
            self.filter_draft = self.filter_applied.clone();
            self.filter_input_active = false;
            self.clamp_selected_visible();
        }
    }

    pub fn cancel_filter(&mut self) {
        self.filter_draft = self.filter_applied.clone();
        self.filter_input_active = false;
    }

    pub fn clear_filter(&mut self) {
        self.filter_applied.clear();
        self.filter_draft.clear();
        self.filter_input_active = false;
        self.clamp_selected_visible();
    }

    fn clamp_selected_visible(&mut self) {
        self.selected_visible = self
            .selected_visible
            .min(self.visible_rows().len().saturating_sub(1));
    }

    pub fn active_name(&self) -> Option<&GcloudConfigurationName> {
        self.rows
            .iter()
            .find(|configuration| configuration.is_active)
            .map(|configuration| &configuration.name)
    }

    pub fn set_active(&mut self, active: GcloudConfiguration) {
        self.activation_confirmation = None;
        for row in &mut self.rows {
            row.is_active = row.name == active.name;
        }
        if let Some(existing) = self.rows.iter_mut().find(|row| row.name == active.name) {
            *existing = active;
        } else {
            self.rows.push(active);
        }
        self.clamp_selected_visible();
    }

    pub fn active(&self) -> Option<&GcloudConfiguration> {
        self.rows
            .iter()
            .find(|configuration| configuration.is_active)
    }

    pub fn is_create_form_open(&self) -> bool {
        self.create_form.is_some()
    }

    pub fn is_context_edit_form_open(&self) -> bool {
        self.context_edit_form.is_some()
    }

    pub fn context_picker(&self) -> Option<&ConfigurationContextPicker> {
        self.context_picker.as_ref()
    }

    pub fn create_form(&self) -> Option<&ConfigurationCreateForm> {
        self.create_form.as_ref()
    }

    pub fn context_edit_form(&self) -> Option<&ConfigurationContextEditForm> {
        self.context_edit_form.as_ref()
    }

    pub fn activation_confirmation(&self) -> Option<&ConfigurationActivationConfirmation> {
        self.activation_confirmation.as_ref()
    }

    pub fn start_activation_confirmation(&mut self) -> ActivationConfirmationRequest {
        let Some(selected) = self.selected().cloned() else {
            return ActivationConfirmationRequest::NoSelection;
        };
        if selected.is_active {
            return ActivationConfirmationRequest::AlreadyActive;
        }
        self.create_form = None;
        self.context_edit_form = None;
        self.activation_confirmation = Some(ConfigurationActivationConfirmation::new(selected));
        ActivationConfirmationRequest::Started
    }

    pub fn confirm_activation(&mut self) -> Option<GcloudConfigurationName> {
        self.activation_confirmation
            .take()
            .map(|confirmation| confirmation.target.name)
    }

    pub fn cancel_activation(&mut self) {
        self.activation_confirmation = None;
    }

    pub fn start_create(&mut self) {
        self.context_edit_form = None;
        self.activation_confirmation = None;
        self.context_picker = None;
        self.create_form = Some(ConfigurationCreateForm::default());
    }

    pub fn append_create_char(&mut self, value: char) {
        if let Some(form) = &mut self.create_form {
            form.append_char(value);
        }
    }

    pub fn backspace_create_field(&mut self) {
        if let Some(form) = &mut self.create_form {
            form.backspace();
        }
    }

    pub fn focus_next_create_field(&mut self) {
        if let Some(form) = &mut self.create_form {
            form.focus_next();
        }
    }

    pub fn focus_previous_create_field(&mut self) {
        if let Some(form) = &mut self.create_form {
            form.focus_previous();
        }
    }

    pub fn create_submission(&mut self) -> Option<GcloudConfigurationDraft> {
        let Some(form) = &mut self.create_form else {
            return None;
        };
        match form.submit_draft() {
            Some(draft) => {
                self.create_form = None;
                Some(draft)
            }
            None => {
                form.focus_name();
                None
            }
        }
    }

    pub fn cancel_create(&mut self) {
        self.create_form = None;
    }

    pub fn start_context_edit(&mut self) -> bool {
        let Some(active) = self.active().cloned() else {
            return false;
        };
        self.create_form = None;
        self.activation_confirmation = None;
        self.context_picker = None;
        self.context_edit_form = Some(ConfigurationContextEditForm::new(&active));
        true
    }

    pub fn append_context_edit_char(&mut self, value: char) {
        if let Some(form) = &mut self.context_edit_form {
            form.append_char(value);
        }
    }

    pub fn backspace_context_edit_field(&mut self) {
        if let Some(form) = &mut self.context_edit_form {
            form.backspace();
        }
    }

    pub fn focus_next_context_edit_field(&mut self) {
        if let Some(form) = &mut self.context_edit_form {
            form.focus_next();
        }
    }

    pub fn focus_previous_context_edit_field(&mut self) {
        if let Some(form) = &mut self.context_edit_form {
            form.focus_previous();
        }
    }

    pub fn context_edit_submission(&mut self) -> Option<GcloudContextEditDraft> {
        self.context_picker = None;
        self.context_edit_form
            .take()
            .map(|form| form.submit_draft())
    }

    pub fn cancel_context_edit(&mut self) {
        self.context_edit_form = None;
        self.context_picker = None;
    }

    pub fn open_context_picker(&mut self) -> Option<ConfigurationContextPickerKind> {
        let form = self.context_edit_form.as_ref()?;
        let kind = ConfigurationContextPickerKind::from(form.focused_field());
        let favorites = self.favorites.values(FavoriteKind::from(kind)).to_vec();
        self.context_picker = Some(ConfigurationContextPicker::new(
            form.target().clone(),
            kind,
            form.focused_field_value().to_string(),
            favorites,
        ));
        Some(kind)
    }

    /// Installs the current user's favorited scopes, read at startup and on
    /// every config reload beside `aliases` and `jump_rules`.
    ///
    /// Deliberately does not touch an already-open `context_picker`: its
    /// `favorites` were snapshotted from this value at [`Self::open_context_picker`]
    /// time, and stay put across a reload while the picker itself is open.
    /// The picker's `rows` are not re-fetched on reload either, so leaving
    /// `favorites` alone keeps the picker internally consistent; re-partitioning
    /// its already-rendered list out from under a live cursor mid-selection
    /// would be worse than showing a stale grouping until the picker is
    /// closed and reopened.
    pub fn set_favorites(&mut self, favorites: FavoriteScopes) {
        self.favorites = favorites;
    }

    pub fn start_context_picker_filter(&mut self) {
        if let Some(picker) = &mut self.context_picker {
            picker.start_filtering();
        }
    }

    pub fn update_context_picker_filter(&mut self, value: String) {
        if let Some(picker) = &mut self.context_picker {
            picker.update_filter(value);
        }
    }

    pub fn apply_context_picker_filter(&mut self) {
        if let Some(picker) = &mut self.context_picker {
            picker.apply_filter();
        }
    }

    pub fn cancel_context_picker_filter(&mut self) {
        if let Some(picker) = &mut self.context_picker {
            picker.cancel_filter();
        }
    }

    pub fn select_next_context_picker_item(&mut self) {
        if let Some(picker) = &mut self.context_picker {
            picker.select_next();
        }
    }

    pub fn select_previous_context_picker_item(&mut self) {
        if let Some(picker) = &mut self.context_picker {
            picker.select_previous();
        }
    }

    pub fn apply_context_picker_selection(&mut self) {
        let Some(item) = self
            .context_picker
            .as_ref()
            .and_then(ConfigurationContextPicker::selected_item)
            .cloned()
        else {
            return;
        };
        if self
            .context_edit_form
            .as_mut()
            .is_some_and(|form| form.apply_picker_item(item))
        {
            self.context_picker = None;
        }
    }

    pub fn retry_context_picker(&mut self) -> Option<ConfigurationContextPickerKind> {
        let picker = self.context_picker.as_mut()?;
        picker.retry();
        Some(picker.kind())
    }

    pub fn cancel_context_picker(&mut self) {
        self.context_picker = None;
    }

    pub fn set_context_picker_loaded(
        &mut self,
        configuration: &GcloudConfigurationName,
        kind: ConfigurationContextPickerKind,
        items: Vec<ConfigurationContextPickerItem>,
    ) {
        let matches_open_picker =
            self.context_edit_form
                .as_ref()
                .map(ConfigurationContextEditForm::target)
                == Some(configuration)
                && self.context_edit_form.as_ref().is_some_and(|form| {
                    ConfigurationContextPickerKind::from(form.focused_field()) == kind
                })
                && self.context_picker.as_ref().is_some_and(|picker| {
                    picker.target() == configuration && picker.kind() == kind
                });
        if matches_open_picker && let Some(picker) = &mut self.context_picker {
            picker.set_loaded(items);
        }
    }

    pub fn set_context_picker_degraded(
        &mut self,
        configuration: &GcloudConfigurationName,
        kind: ConfigurationContextPickerKind,
        error: String,
    ) {
        let matches_open_picker =
            self.context_edit_form
                .as_ref()
                .map(ConfigurationContextEditForm::target)
                == Some(configuration)
                && self.context_edit_form.as_ref().is_some_and(|form| {
                    ConfigurationContextPickerKind::from(form.focused_field()) == kind
                })
                && self.context_picker.as_ref().is_some_and(|picker| {
                    picker.target() == configuration && picker.kind() == kind
                });
        if matches_open_picker && let Some(picker) = &mut self.context_picker {
            picker.set_degraded(error);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn region(name: &str) -> ConfigurationContextPickerItem {
        ConfigurationContextPickerItem::Region(ComputeRegion {
            name: name.to_string(),
            status: None,
        })
    }

    fn picker(favorites: Vec<String>, initial_value: &str) -> ConfigurationContextPicker {
        ConfigurationContextPicker::new(
            GcloudConfigurationName::new("default"),
            ConfigurationContextPickerKind::Region,
            initial_value.to_string(),
            favorites,
        )
    }

    fn configuration(name: &str) -> GcloudConfiguration {
        GcloudConfiguration {
            name: GcloudConfigurationName::new(name),
            is_active: true,
            account: None,
            project: None,
            region: None,
            zone: None,
        }
    }

    /// Parses `body` as a `[favorites]`-table-shaped TOML fragment into a
    /// real `FavoriteScopes`, the same way the config loader does, so these
    /// tests exercise the same parser the rest of the crate does rather than
    /// hand-building the struct.
    fn favorite_scopes(body: &str) -> FavoriteScopes {
        let value: toml::Value = toml::from_str(body).expect("test fixture must be valid TOML");
        let mut problems = crate::config::problems::ConfigProblems::default();
        let scopes = FavoriteScopes::from_file(&value, &mut problems);
        assert!(
            problems.into_messages().is_empty(),
            "test fixture must be a warning-free [favorites] body"
        );
        scopes
    }

    #[test]
    fn visible_items_places_favorites_first_in_stable_source_order() {
        // 25 rows, favorites interleaved every third row rather than grouped
        // at the end: at this size, a comparison sort (e.g.
        // `sort_unstable_by_key`) stops falling back to an in-practice-stable
        // insertion sort and can reorder equal-key (same favorite/non-favorite
        // bucket) elements against each other. A 4-element fixture cannot
        // distinguish that from a real stable partition; this one does.
        let rows: Vec<ConfigurationContextPickerItem> =
            (0..25).map(|i| region(&format!("r-{i:02}"))).collect();
        let favorite_indices: Vec<usize> = (0..25).step_by(3).collect(); // 0,3,...,24
        let favorites: Vec<String> = favorite_indices
            .iter()
            .map(|i| format!("r-{i:02}"))
            .collect();

        let mut picker = picker(favorites, "r-00");
        picker.set_loaded(rows);

        let expected: Vec<String> = favorite_indices
            .iter()
            .map(|i| format!("r-{i:02}"))
            .chain(
                (0..25)
                    .filter(|i| !favorite_indices.contains(i))
                    .map(|i| format!("r-{i:02}")),
            )
            .collect();

        let ordered: Vec<&str> = picker
            .visible_items()
            .into_iter()
            .map(|item| item.value())
            .collect();
        assert_eq!(ordered, expected);
    }

    #[test]
    fn visible_items_with_no_favorites_returns_the_pre_existing_order() {
        let mut picker = picker(Vec::new(), "r-a");
        picker.set_loaded(vec![region("r-a"), region("r-b"), region("r-c")]);

        let ordered: Vec<&str> = picker
            .visible_items()
            .into_iter()
            .map(|item| item.value())
            .collect();
        assert_eq!(ordered, vec!["r-a", "r-b", "r-c"]);
    }

    #[test]
    fn a_filter_still_excludes_a_non_matching_favorite_and_includes_a_matching_non_favorite() {
        let mut picker = picker(vec!["r-prod".to_string()], "r-dev");
        picker.set_loaded(vec![region("r-dev"), region("r-prod")]);
        picker.start_filtering();
        picker.update_filter("dev".to_string());

        let ordered: Vec<&str> = picker
            .visible_items()
            .into_iter()
            .map(|item| item.value())
            .collect();
        assert_eq!(ordered, vec!["r-dev"]);
    }

    #[test]
    fn set_loaded_resolves_initial_value_against_the_ordered_view() {
        // "r-c" is the favorite and sits last in source order, so it moves to
        // the front of the ordered view. Selecting on the unordered `rows`
        // index would land on a different item than the one requested.
        let mut picker = picker(vec!["r-c".to_string()], "r-b");
        picker.set_loaded(vec![region("r-a"), region("r-b"), region("r-c")]);

        assert_eq!(picker.selected_item().map(|item| item.value()), Some("r-b"));
    }

    #[test]
    fn is_favorite_requires_an_exact_value_match_not_a_substring() {
        // "europe-west1" is a favorite; "europe-west1-b" is a different row
        // whose value merely contains it. Only the exact match may be
        // treated as a favorite, and only the exact match may lead the
        // ordered view.
        let mut picker = picker(vec!["europe-west1".to_string()], "europe-west1");
        picker.set_loaded(vec![region("europe-west1-b"), region("europe-west1")]);

        let visible = picker.visible_items();
        assert_eq!(
            visible.iter().map(|item| item.value()).collect::<Vec<_>>(),
            vec!["europe-west1", "europe-west1-b"]
        );
        assert!(picker.is_favorite(visible[0]));
        assert!(!picker.is_favorite(visible[1]));
    }

    #[test]
    fn open_context_picker_only_treats_same_kind_favorites_as_favorites() {
        // The value "shared-name" is favorited under `zones`, not `regions`.
        // Opening the Region picker must not hoist or star a region row that
        // happens to share that name — favorites are matched within their
        // own kind, never across kinds.
        let mut state = ConfigurationsState::default();
        state.set_rows(vec![configuration("default")]);
        state.set_favorites(favorite_scopes(r#"zones = ["shared-name"]"#));

        assert!(state.start_context_edit());
        state.focus_next_context_edit_field(); // Project -> Region
        let kind = state
            .open_context_picker()
            .expect("context edit form is open");
        assert_eq!(kind, ConfigurationContextPickerKind::Region);

        state.set_context_picker_loaded(
            &GcloudConfigurationName::new("default"),
            kind,
            vec![region("shared-name"), region("other-region")],
        );

        let picker = state.context_picker().expect("picker is open");
        let visible = picker.visible_items();
        assert_eq!(
            visible.iter().map(|item| item.value()).collect::<Vec<_>>(),
            vec!["shared-name", "other-region"],
            "no favorite means the pre-existing row order is kept"
        );
        assert!(
            !picker.is_favorite(visible[0]),
            "a zones favorite must not star a same-named region"
        );
    }

    #[test]
    fn open_picker_favorites_are_a_snapshot_unaffected_by_a_later_reload() {
        // The picker snapshots its favorites when it opens. A config reload
        // that arrives while the picker is still open must not re-partition
        // it out from under the user's live selection.
        let mut state = ConfigurationsState::default();
        state.set_rows(vec![configuration("default")]);
        state.set_favorites(favorite_scopes(r#"regions = ["r-a"]"#));

        assert!(state.start_context_edit());
        state.focus_next_context_edit_field(); // Project -> Region
        let kind = state
            .open_context_picker()
            .expect("context edit form is open");
        state.set_context_picker_loaded(
            &GcloudConfigurationName::new("default"),
            kind,
            vec![region("r-a"), region("r-b")],
        );

        // A reload arrives while the picker is open, favoriting "r-b" instead.
        state.set_favorites(favorite_scopes(r#"regions = ["r-b"]"#));

        let picker = state.context_picker().expect("picker is open");
        let visible = picker.visible_items();
        assert_eq!(
            visible.iter().map(|item| item.value()).collect::<Vec<_>>(),
            vec!["r-a", "r-b"],
            "the picker keeps ordering by the favorites it had at open time"
        );
        assert!(
            picker.is_favorite(visible[0]),
            "r-a was favorited when the picker opened and stays favorited"
        );
        assert!(
            !picker.is_favorite(visible[1]),
            "r-b only became a favorite after the picker opened"
        );
    }
}
