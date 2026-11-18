use super::drill::DrillHierarchy;
use super::*;

/// Marker type unifying the Cloud Storage buckets service behind
/// [`ResourceService`]. Never instantiated.
pub enum Storage {}

impl ResourceService for Storage {
    type Row = StorageBucket;
    type Detail = StorageBucketDetail;
    type SortField = StorageSortField;
    type DetailTab = StorageDetailTab;

    const KIND: ResourceServiceKind = ResourceServiceKind::CloudStorage;

    fn collection(
        state: &ResourcesState,
    ) -> &ResourceCollectionState<Self::Row, Self::Detail, Self::SortField, Self::DetailTab> {
        &state.storage.collection
    }

    fn collection_mut(
        state: &mut ResourcesState,
    ) -> &mut ResourceCollectionState<Self::Row, Self::Detail, Self::SortField, Self::DetailTab>
    {
        &mut state.storage.collection
    }

    /// Satisfies the trait signature over the same searchable fields the
    /// bucket list uses. Cloud Storage's *own* narrowing does not go through
    /// here — [`StorageState::visible_buckets`] keeps the plain substring
    /// match of `bucket_matches_filter`, for the reason documented there.
    fn matches_filter(row: &Self::Row, filter: &CompiledFilter) -> bool {
        filter.matches(&owned_bucket_searchable_fields(row))
    }

    /// The needle sits in the name, the identity column of the bucket table.
    fn sample_row() -> Self::Row {
        StorageBucket {
            name: SAMPLE_ROW_NEEDLE.to_string(),
            location: "europe-west1".to_string(),
            location_type: None,
            storage_class: "STANDARD".to_string(),
            versioning_enabled: false,
            uniform_bucket_level_access: true,
            public_access_prevention: None,
            time_created: None,
            updated: None,
        }
    }

    fn compare(
        left: &Self::Row,
        right: &Self::Row,
        sort_field: Self::SortField,
        direction: SortDirection,
    ) -> std::cmp::Ordering {
        compare_buckets(left, right, sort_field, direction)
    }

    /// Buckets are never marked; this exists only to keep [`ResourceService`]
    /// total. A bucket's `location` is its multi-region/region string.
    fn row_id(row: &Self::Row) -> ResourceRowId {
        ResourceRowId {
            name: row.name.clone(),
            location: row.location.clone(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StorageView {
    Buckets,
    ObjectBrowser,
    BucketDetail,
    ObjectDetail,
}

/// A prefix is safe to also use as a server-side object-listing filter when
/// it is non-empty, contains none of the glob-ish characters `* ? [ ]`, does
/// not start with `/` (which would look like an absolute path rather than a
/// prefix relative to the current folder), and has no control characters.
pub fn is_safe_remote_prefix(candidate: &str) -> bool {
    !candidate.is_empty()
        && !candidate.starts_with('/')
        && !candidate
            .chars()
            .any(|ch| ch.is_control() || matches!(ch, '*' | '?' | '[' | ']'))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StorageSortField {
    Name,
    Location,
    StorageClass,
    Updated,
}

impl StorageSortField {
    pub fn label(self) -> &'static str {
        match self {
            Self::Name => "Name",
            Self::Location => "Location",
            Self::StorageClass => "Class",
            Self::Updated => "Updated",
        }
    }

    pub const COLUMN_COUNT: usize = 6;

    pub fn from_column(index: usize) -> Option<Self> {
        match index {
            0 => Some(Self::Name),
            1 => Some(Self::Location),
            2 => Some(Self::StorageClass),
            3 => Some(Self::Updated),
            _ => None,
        }
    }

    pub fn column(self) -> usize {
        match self {
            Self::Name => 0,
            Self::Location => 1,
            Self::StorageClass => 2,
            Self::Updated => 3,
        }
    }
}

/// The active tab within either Cloud Storage detail pane (bucket or
/// object). Both panes share the same Summary/Metadata/JSON structure, so a
/// single enum backs `StorageState::bucket_detail_tab` and
/// `StorageState::object_detail_tab`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StorageDetailTab {
    Summary,
    Metadata,
    Json,
}

impl StorageDetailTab {
    pub const ALL: [Self; 3] = [Self::Summary, Self::Metadata, Self::Json];

    pub fn label(self) -> &'static str {
        match self {
            Self::Summary => "Summary",
            Self::Metadata => "Metadata",
            Self::Json => "JSON",
        }
    }
}

/// One row in the object browser: either a synthetic folder (common prefix) or
/// a concrete object.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StorageBrowserRow<'a> {
    Prefix(&'a StoragePrefix),
    Object(&'a StorageObject),
}

/// Outcome of ascending one level in the object browser.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AscendOutcome {
    /// Reload the object page for the given bucket and (now-shorter) prefix.
    ReloadPage { bucket: String, prefix: String },
    /// The browser was at the root prefix; the view returned to the bucket list.
    ReturnedToBuckets,
}

/// Bespoke state for the Cloud Storage service: a flat bucket list plus an
/// object browser and per-resource detail panes. It mirrors the generic
/// list/nav/filter/sort surface of [`ResourceCollectionState`] but routes each
/// generic operation to whichever [`StorageView`] is active.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StorageState {
    view: StorageView,
    /// Generic list/nav/filter/sort/detail surface for the bucket list. The
    /// `view` field above stays the routing source of truth; the embedded
    /// collection's `view_mode` is kept in sync with it by
    /// [`DrillHierarchy::restore_invariant`], which the provided
    /// [`DrillHierarchy::set_view`] runs on every transition
    /// (`Detail` iff `view == StorageView::BucketDetail`). The collection's
    /// `marks` / `mark_anchor` stay empty for storage (marks are
    /// compute-only today).
    pub(super) collection: ResourceCollectionState<
        StorageBucket,
        StorageBucketDetail,
        StorageSortField,
        StorageDetailTab,
    >,
    /// True when the bucket-list `last_problem`'s stderr indicated a
    /// requester-pays / `userProjectMissing` condition. Meaningless when the
    /// collection's `last_problem` is `None`. See
    /// [`StorageBrowserState::requester_pays`] for the browser (object-page)
    /// equivalent.
    last_problem_requester_pays: bool,

    browser: StorageBrowserState,

    object_detail: Option<StorageObjectDetail>,
    object_detail_error: Option<String>,
    object_detail_target: Option<String>,
    object_detail_tab: StorageDetailTab,
    object_detail_scroll: u16,
}

impl Default for StorageState {
    fn default() -> Self {
        Self {
            view: StorageView::Buckets,
            collection: ResourceCollectionState::new(
                StorageSortField::Name,
                StorageDetailTab::Summary,
            ),
            last_problem_requester_pays: false,
            browser: StorageBrowserState::default(),
            object_detail: None,
            object_detail_error: None,
            object_detail_target: None,
            object_detail_tab: StorageDetailTab::Summary,
            object_detail_scroll: 0,
        }
    }
}

impl StorageState {
    pub fn view(&self) -> StorageView {
        self.stored_view()
    }

    fn is_browser_list(&self) -> bool {
        matches!(
            self.view,
            StorageView::ObjectBrowser | StorageView::ObjectDetail
        )
    }

    // ---- generic list/context surface (routed by view) ----

    pub fn has_loaded_context(&self) -> bool {
        self.collection.context.is_some()
    }

    pub fn active_context_label(&self) -> Option<String> {
        self.collection
            .context
            .as_ref()
            .map(|context| context.configuration.to_string())
    }

    pub fn is_stale(&self) -> bool {
        self.collection.stale
    }

    /// The current in-flight/failure state of the bucket-list fetch.
    pub fn load_phase(&self) -> &ResourceLoadPhase {
        self.collection.load_phase()
    }

    /// The identity of the bucket listing currently held. See
    /// [`ResourcesState::list_landed_sequence`].
    ///
    /// [`ResourcesState::list_landed_sequence`]: super::ResourcesState::list_landed_sequence
    pub(super) fn landed_sequence(&self) -> Option<u64> {
        self.collection.landed_sequence()
    }

    pub(super) fn clear_load_phase(&mut self) {
        self.collection.clear_load_phase();
    }

    pub(super) fn fail_load(&mut self, context: GcloudContext, message: String) {
        self.collection.fail_load(context, message);
    }

    /// The context a failed bucket-list fetch belongs to. See
    /// [`ResourcesState::list_load_failed_for`].
    pub(super) fn failed_context(&self) -> Option<&GcloudContext> {
        self.collection.failed_context()
    }

    /// Marks the bucket-list fetch identified by `sequence` as in flight.
    pub(super) fn begin_load(&mut self, sequence: u64, started_at: std::time::Instant) {
        self.collection.begin_load(sequence, started_at);
    }

    /// Resolves the bucket-list fetch identified by `sequence` back to `Idle`,
    /// if it still owns the load phase, returning whether it did. See
    /// [`ResourcesState::abandon_load`].
    ///
    /// [`ResourcesState::abandon_load`]: super::ResourcesState::abandon_load
    pub(super) fn abandon_load(&mut self, sequence: u64) -> bool {
        self.collection.abandon_load(sequence)
    }

    pub fn needs_refresh(&self, context: &GcloudContext) -> bool {
        self.collection.context.as_ref() != Some(context) || self.collection.stale
    }

    pub fn is_showing_detail(&self) -> bool {
        matches!(
            self.view,
            StorageView::BucketDetail | StorageView::ObjectDetail
        )
    }

    pub fn selected_visible_index(&self) -> Option<usize> {
        if self.is_browser_list() {
            (!self.visible_browser_rows().is_empty()).then_some(self.browser.selected)
        } else {
            (!self.visible_buckets().is_empty()).then_some(self.collection.selected_visible)
        }
    }

    pub fn selected_visible_index_for_visible_rows(&self, visible_rows: usize) -> Option<usize> {
        (visible_rows > 0).then_some(if self.is_browser_list() {
            self.browser.selected
        } else {
            self.collection.selected_visible
        })
    }

    pub fn is_filter_input_active(&self) -> bool {
        match self.view {
            StorageView::ObjectBrowser | StorageView::ObjectDetail => {
                self.browser.filter_input_active
            }
            _ => self.collection.filter_input_active,
        }
    }

    pub fn filter_draft(&self) -> &str {
        if self.is_browser_list() {
            &self.browser.filter_draft
        } else {
            &self.collection.filter_draft
        }
    }

    pub fn filter_applied(&self) -> &str {
        if self.is_browser_list() {
            &self.browser.filter_applied
        } else {
            self.collection.filter_applied.source()
        }
    }

    pub fn sort_label(&self) -> &'static str {
        if self.is_browser_list() {
            "Name"
        } else {
            self.collection.sort_field.label()
        }
    }

    pub fn start_filter(&mut self) {
        if self.is_browser_list() {
            self.browser.filter_input_active = true;
            self.browser.filter_draft = self.browser.filter_applied.clone();
        } else {
            self.collection.filter_input_active = true;
            self.collection.filter_draft = self.collection.filter_applied.source().to_string();
        }
    }

    pub fn append_filter_char(&mut self, value: char) {
        if value.is_control() {
            return;
        }
        if self.is_browser_list() {
            self.browser.filter_draft.push(value);
        } else {
            self.collection.filter_draft.push(value);
        }
    }

    pub fn backspace_filter(&mut self) {
        if self.is_browser_list() {
            self.browser.filter_draft.pop();
        } else {
            self.collection.filter_draft.pop();
        }
    }

    /// Applies the pending filter draft. Object-browser filtering always
    /// narrows the already-loaded rows locally; when the applied draft is
    /// also a [`is_safe_remote_prefix`], the browser additionally marks
    /// `remote_filter` and returns the `(bucket, prefix)` pair for a fresh
    /// first-page reload scoped to that prefix. Bucket-list filtering is
    /// always local only and never returns a reload request.
    pub fn apply_filter(&mut self) -> Option<(String, String)> {
        if self.is_browser_list() {
            self.browser.filter_applied = self.browser.filter_draft.trim().to_string();
            self.browser.filter_draft = self.browser.filter_applied.clone();
            self.browser.filter_input_active = false;

            let reload = if is_safe_remote_prefix(&self.browser.filter_applied) {
                self.browser.remote_filter = true;
                // Reuse `effective_prefix()` (now that `remote_filter` is set)
                // rather than re-deriving `current_prefix() + filter_applied`
                // here too.
                let prefix = self.effective_prefix();
                self.browser
                    .bucket
                    .as_ref()
                    .map(|bucket| (bucket.name.clone(), prefix))
            } else {
                self.browser.remote_filter = false;
                None
            };

            self.browser.selected = self
                .browser
                .selected
                .min(self.visible_browser_rows().len().saturating_sub(1));
            reload
        } else {
            self.collection.filter_applied =
                CompiledResourceFilter::new(self.collection.filter_draft.trim().to_string());
            self.collection.filter_draft = self.collection.filter_applied.source().to_string();
            self.collection.filter_input_active = false;
            self.collection.selected_visible = self
                .collection
                .selected_visible
                .min(self.visible_buckets().len().saturating_sub(1));
            None
        }
    }

    /// Whether the currently applied filter is local-only, or additionally
    /// narrowed the server-side listing (see [`Self::apply_filter`]).
    pub fn filter_scope(&self) -> FilterScope {
        if self.is_browser_list() && self.browser.remote_filter {
            FilterScope::Remote
        } else {
            FilterScope::Local
        }
    }

    pub fn cancel_filter(&mut self) {
        if self.is_browser_list() {
            self.browser.filter_draft = self.browser.filter_applied.clone();
            self.browser.filter_input_active = false;
        } else {
            self.collection.filter_draft = self.collection.filter_applied.source().to_string();
            self.collection.filter_input_active = false;
        }
    }

    pub fn selected_column(&self) -> usize {
        if self.is_browser_list() {
            0
        } else {
            self.collection.selected_column
        }
    }

    pub fn sort_direction(&self) -> SortDirection {
        if self.is_browser_list() {
            SortDirection::Ascending
        } else {
            self.collection.sort_direction
        }
    }

    /// The bucket list's sort arrow, as a display position in `layout`. The
    /// object browser has no sortable columns and pins it to the first one.
    pub fn sort_column_index(&self, layout: &[usize]) -> usize {
        if self.is_browser_list() {
            0
        } else {
            display_position(layout, self.collection.sort_field.column())
        }
    }

    /// Column navigation and sorting below apply to the bucket list only:
    /// `layout` is the bucket list's, and the object browser — a detail mode
    /// with fixed columns and no sorting — ignores every one of these keys.
    pub fn select_next_column(&mut self, layout: &[usize]) {
        if self.is_browser_list() {
            return;
        }
        self.collection.selected_column = next_sortable_column(
            self.collection.selected_column,
            layout,
            StorageSortField::from_column,
        );
    }

    pub fn select_previous_column(&mut self, layout: &[usize]) {
        if self.is_browser_list() {
            return;
        }
        self.collection.selected_column = previous_sortable_column(
            self.collection.selected_column,
            layout,
            StorageSortField::from_column,
        );
    }

    pub fn sort_selected_column(&mut self, layout: &[usize]) {
        if self.is_browser_list() {
            return;
        }
        let Some(field) = layout
            .get(self.collection.selected_column)
            .copied()
            .and_then(StorageSortField::from_column)
        else {
            return;
        };
        if self.collection.sort_field == field {
            self.collection.sort_direction = self.collection.sort_direction.toggled();
        } else {
            self.collection.sort_field = field;
            self.collection.sort_direction = SortDirection::Ascending;
        }
        self.collection.selected_visible = self
            .collection
            .selected_visible
            .min(self.visible_buckets().len().saturating_sub(1));
    }

    pub fn select_next(&mut self) {
        if self.is_browser_list() {
            let len = self.visible_browser_rows().len();
            if len > 0 {
                self.browser.selected = (self.browser.selected + 1).min(len - 1);
            }
        } else {
            let len = self.visible_buckets().len();
            if len > 0 {
                self.collection.selected_visible =
                    (self.collection.selected_visible + 1).min(len - 1);
            }
        }
    }

    pub fn select_previous(&mut self) {
        if self.is_browser_list() {
            self.browser.selected = self.browser.selected.saturating_sub(1);
        } else {
            self.collection.selected_visible = self.collection.selected_visible.saturating_sub(1);
        }
    }

    pub fn active_detail_tab_label(&self) -> &'static str {
        match self.view {
            StorageView::ObjectDetail => self.object_detail_tab.label(),
            _ => self.collection.active_detail_tab.label(),
        }
    }

    pub fn select_next_tab(&mut self) {
        match self.view {
            StorageView::ObjectDetail => {
                self.object_detail_scroll = 0;
                self.object_detail_tab = next_detail_tab(self.object_detail_tab);
            }
            _ => {
                self.collection.detail_scroll = 0;
                self.collection.active_detail_tab =
                    next_detail_tab(self.collection.active_detail_tab);
            }
        }
    }

    pub fn select_previous_tab(&mut self) {
        match self.view {
            StorageView::ObjectDetail => {
                self.object_detail_scroll = 0;
                self.object_detail_tab = previous_detail_tab(self.object_detail_tab);
            }
            _ => {
                self.collection.detail_scroll = 0;
                self.collection.active_detail_tab =
                    previous_detail_tab(self.collection.active_detail_tab);
            }
        }
    }

    pub fn scroll_detail_down(&mut self) {
        match self.view {
            StorageView::ObjectDetail => {
                self.object_detail_scroll =
                    self.object_detail_scroll.saturating_add(DETAIL_PAGE_SIZE);
            }
            _ => {
                self.collection.detail_scroll = self
                    .collection
                    .detail_scroll
                    .saturating_add(DETAIL_PAGE_SIZE);
            }
        }
    }

    pub fn scroll_detail_up(&mut self) {
        match self.view {
            StorageView::ObjectDetail => {
                self.object_detail_scroll =
                    self.object_detail_scroll.saturating_sub(DETAIL_PAGE_SIZE);
            }
            _ => {
                self.collection.detail_scroll = self
                    .collection
                    .detail_scroll
                    .saturating_sub(DETAIL_PAGE_SIZE);
            }
        }
    }

    pub fn detail_scroll(&self) -> u16 {
        match self.view {
            StorageView::ObjectDetail => self.object_detail_scroll,
            _ => self.collection.detail_scroll,
        }
    }

    pub fn close_detail(&mut self) {
        match self.view {
            StorageView::BucketDetail => {
                self.set_view(StorageView::Buckets);
                self.clear_bucket_detail();
            }
            StorageView::ObjectDetail => {
                self.set_view(StorageView::ObjectBrowser);
                self.clear_object_detail();
            }
            _ => {}
        }
    }

    pub fn show_service_list(&mut self) {
        self.set_view(StorageView::Buckets);
        self.clear_bucket_detail();
        self.clear_object_detail();
    }

    pub fn set_problem(&mut self, problem: DiagnosticProblem) {
        self.collection.last_problem = Some(problem);
    }

    pub fn problem(&self) -> Option<&DiagnosticProblem> {
        self.collection.last_problem.as_ref()
    }

    /// Records a sanitized diagnostic and keeps `last_problem` in sync via the
    /// `From<&GcloudDiagnostic>` bridge, mirroring
    /// [`ResourcesState::set_service_diagnostic`] for the bespoke storage state.
    pub fn set_diagnostic(&mut self, diagnostic: GcloudDiagnostic) {
        self.collection.last_problem = Some(DiagnosticProblem::from(&diagnostic));
        self.collection.last_diagnostic = Some(diagnostic);
    }

    pub fn diagnostic(&self) -> Option<&GcloudDiagnostic> {
        self.collection.last_diagnostic.as_ref()
    }

    /// Sets whether `last_problem`'s stderr indicated a requester-pays
    /// condition. Kept separate from `set_problem` because the generic
    /// [`ResourcesState::set_service_problem`] dispatch (shared with GCE/GKE)
    /// only carries a [`DiagnosticProblem`]; the storage-specific reducer arm
    /// calls this immediately afterward when relevant.
    pub fn set_problem_requester_pays(&mut self, requester_pays: bool) {
        self.last_problem_requester_pays = requester_pays;
    }

    pub fn problem_requester_pays(&self) -> bool {
        self.last_problem_requester_pays
    }

    pub fn mark_stale_and_clear(&mut self) {
        self.collection.filter_draft = self.collection.filter_applied.source().to_string();
        self.collection.filter_input_active = false;
        self.collection.rows.clear();
        self.collection.context = None;
        self.collection.selected_visible = 0;
        self.collection.stale = true;
        self.browser = StorageBrowserState::default();
        self.set_view(StorageView::Buckets);
        self.clear_bucket_detail();
        self.clear_object_detail();
    }

    fn clear_bucket_detail(&mut self) {
        self.collection.detail = None;
        self.collection.detail_error = None;
        self.collection.detail_target = None;
        self.collection.detail_error_target_name = None;
        self.collection.active_detail_tab = StorageDetailTab::Summary;
        self.collection.detail_scroll = 0;
    }

    fn clear_object_detail(&mut self) {
        self.object_detail = None;
        self.object_detail_error = None;
        self.object_detail_target = None;
        self.object_detail_tab = StorageDetailTab::Summary;
        self.object_detail_scroll = 0;
    }

    // ---- bucket list ----

    pub fn buckets(&self) -> &[StorageBucket] {
        &self.collection.rows
    }

    pub fn visible_buckets(&self) -> Vec<&StorageBucket> {
        let filter = self
            .collection
            .filter_applied
            .source()
            .trim()
            .to_lowercase();
        let mut rows = self
            .collection
            .rows
            .iter()
            .filter(|bucket| filter.is_empty() || bucket_matches_filter(bucket, &filter))
            .collect::<Vec<_>>();
        rows.sort_by(|left, right| {
            compare_buckets(
                left,
                right,
                self.collection.sort_field,
                self.collection.sort_direction,
            )
        });
        rows
    }

    pub fn visible_bucket_count(&self) -> usize {
        let filter = self
            .collection
            .filter_applied
            .source()
            .trim()
            .to_lowercase();
        self.collection
            .rows
            .iter()
            .filter(|bucket| filter.is_empty() || bucket_matches_filter(bucket, &filter))
            .count()
    }

    pub fn selected_bucket(&self) -> Option<&StorageBucket> {
        self.visible_buckets()
            .get(self.collection.selected_visible)
            .copied()
    }

    pub fn set_buckets(
        &mut self,
        context: GcloudContext,
        buckets: Vec<StorageBucket>,
    ) -> Option<String> {
        let prev_selected = self.selected_bucket().map(|bucket| bucket.name.clone());
        let detail_open = self.view == StorageView::BucketDetail;
        let detail_key = self
            .collection
            .detail_target
            .as_ref()
            .map(|bucket| bucket.name.clone());

        self.collection.context = Some(context);
        self.collection.rows = buckets;
        self.collection.stale = false;
        self.collection.last_problem = None;
        self.collection.last_diagnostic = None;
        self.last_problem_requester_pays = false;
        self.clear_load_phase();

        let mut dropped_detail = None;
        if detail_open {
            let still_present = detail_key
                .as_ref()
                .map(|name| {
                    self.collection
                        .rows
                        .iter()
                        .any(|bucket| &bucket.name == name)
                })
                .unwrap_or(false);
            if !still_present {
                dropped_detail = detail_key;
                self.set_view(StorageView::Buckets);
                self.clear_bucket_detail();
            }
        }

        self.collection.selected_visible = prev_selected
            .as_ref()
            .and_then(|name| {
                self.visible_buckets()
                    .iter()
                    .position(|bucket| &bucket.name == name)
            })
            .unwrap_or_else(|| {
                self.collection
                    .selected_visible
                    .min(self.visible_buckets().len().saturating_sub(1))
            });

        dropped_detail
    }

    pub fn open_bucket_metadata(&mut self) -> Option<StorageBucket> {
        let bucket = self.selected_bucket()?.clone();
        self.set_view(StorageView::BucketDetail);
        self.clear_bucket_detail();
        self.collection.detail_target = Some(bucket.clone());
        Some(bucket)
    }

    // ---- object browser ----

    pub fn open_object_browser(&mut self) -> Option<StorageBucket> {
        let bucket = self.selected_bucket()?.clone();
        self.browser = StorageBrowserState {
            bucket: Some(bucket.clone()),
            loading: true,
            ..StorageBrowserState::default()
        };
        self.set_view(StorageView::ObjectBrowser);
        Some(bucket)
    }

    pub fn browser_bucket(&self) -> Option<&StorageBucket> {
        self.browser.bucket.as_ref()
    }

    pub fn browser_prefix_path(&self) -> &[String] {
        &self.browser.prefix_path
    }

    /// Joins the current prefix segments into a gcloud-style prefix (each
    /// segment already carries its own trailing slash from the API), or an
    /// empty string at the bucket root.
    pub fn current_prefix(&self) -> String {
        self.browser.prefix_path.concat()
    }

    /// The prefix currently governing the object browser's server-side
    /// scope: the plain current prefix, or — while a safe-prefix remote
    /// filter is active — the current prefix with the applied filter
    /// appended.
    fn effective_prefix(&self) -> String {
        if self.browser.remote_filter {
            format!("{}{}", self.current_prefix(), self.browser.filter_applied)
        } else {
            self.current_prefix()
        }
    }

    pub fn browser_loading(&self) -> bool {
        self.browser.loading
    }

    pub fn browser_problem(&self) -> Option<&DiagnosticProblem> {
        self.browser.problem.as_ref()
    }

    pub fn browser_partial_page_error(&self) -> Option<&str> {
        self.browser.partial_page_error.as_deref()
    }

    pub fn browser_row_total(&self) -> usize {
        self.browser.prefixes.len() + self.browser.objects.len()
    }

    /// True when the API returned a continuation token for the current
    /// browser scope, i.e. another page can be fetched via `NextPage`.
    pub fn browser_has_more(&self) -> bool {
        self.browser.next_page_token.is_some()
    }

    /// Zero-based index of the most recently loaded page for the current
    /// browser scope (0 for the first page, incremented once per append).
    pub fn browser_page_index(&self) -> usize {
        self.browser.page_index
    }

    pub fn visible_browser_row_count(&self) -> usize {
        self.visible_browser_rows().len()
    }

    /// Returns the browser rows (prefixes first, then objects) matching the
    /// applied local filter. The filter is compared against each row's display
    /// name relative to the current prefix.
    pub fn visible_browser_rows(&self) -> Vec<StorageBrowserRow<'_>> {
        let prefix = self.current_prefix();
        let filter = self.browser.filter_applied.trim().to_lowercase();
        let matches = |name: &str| {
            filter.is_empty()
                || browser_display_name(name, &prefix)
                    .to_lowercase()
                    .contains(&filter)
        };
        self.browser
            .prefixes
            .iter()
            .filter(|item| matches(&item.name))
            .map(StorageBrowserRow::Prefix)
            .chain(
                self.browser
                    .objects
                    .iter()
                    .filter(|item| matches(&item.key))
                    .map(StorageBrowserRow::Object),
            )
            .collect()
    }

    /// True when `(bucket, prefix)` matches the object browser's current
    /// scope. Used to ignore both stale-scope page loads (already the case
    /// for `set_object_page`) and stale-scope page-load *failures*: a
    /// `StorageObjectPageLoadFailed` for a bucket/prefix the user has since
    /// navigated away from must not clobber the now-current view.
    pub fn matches_browser_scope(&self, bucket: &str, prefix: &str) -> bool {
        self.browser
            .bucket
            .as_ref()
            .map(|b| b.name == bucket)
            .unwrap_or(false)
            && self.effective_prefix() == prefix
    }

    pub fn set_object_page(
        &mut self,
        bucket: &str,
        prefix: &str,
        page: StorageObjectPage,
        append: bool,
    ) {
        if !self.matches_browser_scope(bucket, prefix) {
            return;
        }
        if append {
            self.browser.prefixes.extend(page.prefixes);
            self.browser.objects.extend(page.objects);
            self.browser.page_index = self.browser.page_index.saturating_add(1);
        } else {
            self.browser.prefixes = page.prefixes;
            self.browser.objects = page.objects;
            self.browser.selected = 0;
            self.browser.page_index = 0;
        }
        self.browser.next_page_token = page.next_page_token;
        self.browser.loading = false;
        self.browser.problem = None;
        self.browser.requester_pays = false;
        self.browser.partial_page_error = None;
        let len = self.visible_browser_rows().len();
        self.browser.selected = self.browser.selected.min(len.saturating_sub(1));
    }

    pub fn set_browser_problem(&mut self, problem: DiagnosticProblem, requester_pays: bool) {
        self.browser.loading = false;
        self.browser.problem = Some(problem);
        self.browser.requester_pays = requester_pays;
    }

    pub fn browser_requester_pays(&self) -> bool {
        self.browser.requester_pays
    }

    pub fn set_browser_partial_page_error(&mut self, message: String) {
        self.browser.loading = false;
        self.browser.partial_page_error = Some(message);
    }

    /// Descends into the selected common-prefix row. Returns the bucket name and
    /// the new (longer) prefix to load, or `None` when the selection is an
    /// object rather than a folder.
    pub fn descend_into_selected(&mut self) -> Option<(String, String)> {
        let bucket = self.browser.bucket.as_ref()?.name.clone();
        let new_prefix = match self.visible_browser_rows().get(self.browser.selected)? {
            StorageBrowserRow::Prefix(prefix) => prefix.name.clone(),
            StorageBrowserRow::Object(_) => return None,
        };
        // `prefix.name` is the full path from the bucket root; track only the
        // trailing segment relative to the current prefix so `current_prefix`
        // reconstructs the same full path.
        let current = self.current_prefix();
        let segment = new_prefix
            .strip_prefix(&current)
            .unwrap_or(&new_prefix)
            .to_string();
        self.browser.prefix_path.push(segment);
        self.browser.prefixes.clear();
        self.browser.objects.clear();
        self.browser.selected = 0;
        self.browser.next_page_token = None;
        self.browser.page_index = 0;
        self.browser.loading = true;
        self.reset_browser_filter();
        Some((bucket, self.current_prefix()))
    }

    /// Opens the object detail pane for the selected object row. Returns the
    /// bucket name and object key, or `None` when the selection is a folder.
    pub fn open_selected_object_detail(&mut self) -> Option<(String, String)> {
        let bucket = self.browser.bucket.as_ref()?.name.clone();
        let key = match self.visible_browser_rows().get(self.browser.selected)? {
            StorageBrowserRow::Object(object) => object.key.clone(),
            StorageBrowserRow::Prefix(_) => return None,
        };
        self.set_view(StorageView::ObjectDetail);
        self.clear_object_detail();
        self.object_detail_target = Some(key.clone());
        Some((bucket, key))
    }

    /// Ascends one level in the object browser, popping the deepest prefix
    /// segment or returning to the bucket list at the root.
    pub fn ascend(&mut self) -> Option<AscendOutcome> {
        if self.browser.prefix_path.pop().is_some() {
            self.browser.prefixes.clear();
            self.browser.objects.clear();
            self.browser.selected = 0;
            self.browser.next_page_token = None;
            self.browser.page_index = 0;
            self.browser.loading = true;
            self.reset_browser_filter();
            let bucket = self.browser.bucket.as_ref()?.name.clone();
            Some(AscendOutcome::ReloadPage {
                bucket,
                prefix: self.current_prefix(),
            })
        } else {
            self.set_view(StorageView::Buckets);
            self.browser.next_page_token = None;
            Some(AscendOutcome::ReturnedToBuckets)
        }
    }

    /// Clears the object-browser filter (draft, applied, active input, and
    /// remote-filter flag). Called when structural navigation (descend or
    /// ascend) changes the browser's scope, since a filter applied at one
    /// level should not silently keep narrowing a different level.
    fn reset_browser_filter(&mut self) {
        self.browser.filter_draft.clear();
        self.browser.filter_applied.clear();
        self.browser.filter_input_active = false;
        self.browser.remote_filter = false;
    }

    /// Returns a first-page load request for the current browser scope.
    pub fn browser_page_request(&self) -> Option<(String, String)> {
        let bucket = self.browser.bucket.as_ref()?.name.clone();
        Some((bucket, self.effective_prefix()))
    }

    /// Returns the next-page request `(bucket, prefix, token)` when a
    /// continuation token is available.
    pub fn next_page_request(&self) -> Option<(String, String, String)> {
        let bucket = self.browser.bucket.as_ref()?.name.clone();
        let token = self.browser.next_page_token.clone()?;
        Some((bucket, self.effective_prefix(), token))
    }

    /// Builds the `gs://` URI and a human description for the selected row (or
    /// bucket). The URI is returned verbatim for clipboard use and must not be
    /// sanitized.
    pub fn selected_uri(&self, _context: &GcloudContext) -> Option<(String, String)> {
        if self.is_browser_list() {
            let bucket = self.browser.bucket.as_ref()?;
            match self.visible_browser_rows().get(self.browser.selected)? {
                StorageBrowserRow::Prefix(prefix) => Some((
                    format!("gs://{}/{}", bucket.name, prefix.name),
                    "prefix URI".to_string(),
                )),
                StorageBrowserRow::Object(object) => Some((
                    format!("gs://{}/{}", bucket.name, object.key),
                    "object URI".to_string(),
                )),
            }
        } else {
            let bucket = self.selected_bucket()?;
            Some((format!("gs://{}", bucket.name), "bucket URI".to_string()))
        }
    }

    // ---- detail panes ----

    pub fn bucket_detail(&self) -> Option<&StorageBucketDetail> {
        self.collection.detail.as_ref()
    }

    pub fn bucket_detail_error(&self) -> Option<&str> {
        self.collection.detail_error.as_deref()
    }

    pub fn bucket_detail_tab(&self) -> StorageDetailTab {
        self.collection.active_detail_tab
    }

    pub fn object_detail(&self) -> Option<&StorageObjectDetail> {
        self.object_detail.as_ref()
    }

    pub fn object_detail_error(&self) -> Option<&str> {
        self.object_detail_error.as_deref()
    }

    pub fn object_detail_tab(&self) -> StorageDetailTab {
        self.object_detail_tab
    }

    // Does not refresh `collection.detail_target`: the reducer only routes a
    // `StorageBucketDetailLoaded` result here when
    // `is_active_bucket_detail_target` matches the loaded name (see the
    // guard in `src/app/reducer/root.rs`), so `detail_target` already names
    // this bucket.
    pub fn set_bucket_detail(&mut self, detail: StorageBucketDetail) {
        self.collection.detail_error_target_name = None;
        self.collection.detail_error = None;
        self.collection.detail = Some(detail);
    }

    pub fn set_bucket_detail_error(&mut self, bucket_name: String, error: String) {
        self.set_view(StorageView::BucketDetail);
        self.collection.detail = None;
        self.collection.detail_error_target_name = Some(bucket_name);
        self.collection.detail_error = Some(error);
    }

    pub fn set_object_detail(&mut self, detail: StorageObjectDetail) {
        self.object_detail_error = None;
        self.object_detail_target = Some(detail.key.clone());
        self.object_detail = Some(detail);
    }

    pub fn set_object_detail_error(&mut self, object_key: String, error: String) {
        self.set_view(StorageView::ObjectDetail);
        self.object_detail = None;
        self.object_detail_target = Some(object_key);
        self.object_detail_error = Some(error);
    }

    pub fn is_active_bucket_detail_target(&self, bucket_name: &str) -> bool {
        self.collection
            .detail_target
            .as_ref()
            .map(|bucket| bucket.name == bucket_name)
            .unwrap_or(false)
            || self.collection.detail_error_target_name.as_deref() == Some(bucket_name)
    }

    pub fn is_active_object_detail_target(&self, object_key: &str) -> bool {
        self.object_detail_target.as_deref() == Some(object_key)
    }

    /// The bucket the view currently on screen is about: the browsed bucket
    /// inside the object browser, the metadata pane's target in
    /// `BucketDetail`, and the list selection in the bucket list.
    ///
    /// Keyed off the view on purpose. Ascending out of the object browser
    /// leaves `browser.bucket` populated (see [`Self::ascend`]), so reading
    /// that field alone would name a bucket the user has already left; and
    /// with a metadata pane open the list cursor is still movable, so the list
    /// selection is not what is being shown either.
    pub fn view_bucket(&self) -> Option<&StorageBucket> {
        match self.view {
            StorageView::ObjectBrowser | StorageView::ObjectDetail => self.browser.bucket.as_ref(),
            StorageView::BucketDetail => self.collection.detail_target.as_ref(),
            StorageView::Buckets => self.selected_bucket(),
        }
    }

    /// Returns the detail-target display name for the active detail view.
    pub fn detail_target_name(&self) -> Option<&str> {
        match self.view {
            StorageView::ObjectDetail => self.object_detail_target.as_deref().or_else(|| {
                self.object_detail
                    .as_ref()
                    .map(|detail| detail.key.as_str())
            }),
            _ => self
                .collection
                .detail_target
                .as_ref()
                .map(|bucket| bucket.name.as_str())
                .or_else(|| {
                    self.collection
                        .detail
                        .as_ref()
                        .map(|detail| detail.name.as_str())
                })
                .or(self.collection.detail_error_target_name.as_deref()),
        }
    }
}

impl DrillHierarchy for StorageState {
    type View = StorageView;

    fn stored_view(&self) -> Self::View {
        self.view
    }

    fn store_view(&mut self, view: Self::View) {
        self.view = view;
    }

    /// The bucket-detail view is the one Cloud Storage view that is a pane
    /// rather than a list, so the embedded collection's `view_mode` has to
    /// follow the view (`Detail` iff `view == StorageView::BucketDetail`,
    /// `List` otherwise) or the shared list surface renders the wrong shape.
    ///
    /// It runs on every transition rather than only the ones that already
    /// touch `collection`, which is what covers the reducer-reachable
    /// transitions that otherwise leave the collection alone — entering the
    /// detail pane by way of `set_bucket_detail_error`, say.
    ///
    /// Unlike BigQuery's implementation this bumps no generation and clears
    /// no child collection: the object browser's page state is keyed by
    /// bucket and prefix rather than by a drill counter.
    fn restore_invariant(&mut self, view: Self::View) {
        self.collection.view_mode = if view == StorageView::BucketDetail {
            ResourcesViewMode::Detail
        } else {
            ResourcesViewMode::List
        };
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct StorageBrowserState {
    bucket: Option<StorageBucket>,
    prefix_path: Vec<String>,
    prefixes: Vec<StoragePrefix>,
    objects: Vec<StorageObject>,
    selected: usize,
    next_page_token: Option<String>,
    page_index: usize,
    filter_input_active: bool,
    filter_draft: String,
    filter_applied: String,
    remote_filter: bool,
    problem: Option<DiagnosticProblem>,
    /// True when `problem`'s stderr indicated a requester-pays /
    /// `userProjectMissing` condition, rendered with a distinct message
    /// from a generic permission-denied failure. Meaningless when
    /// `problem` is `None`.
    requester_pays: bool,
    partial_page_error: Option<String>,
    loading: bool,
}

/// Strips the current prefix from a raw browser row name and sanitizes the
/// result the same way the projection layer sanitizes for rendering (see
/// [`sanitize_display_text`]). This is used to match the local object-browser
/// filter, so a hostile key containing control characters filters
/// consistently with what's actually rendered on screen. Sanitizing is
/// idempotent, so callers that already sanitize (e.g. the projection layer)
/// double-sanitizing here is harmless.
fn browser_display_name(name: &str, current_prefix: &str) -> String {
    sanitize_display_text(name.strip_prefix(current_prefix).unwrap_or(name))
}

/// The tab that follows `current` in `StorageDetailTab::ALL`, wrapping
/// around after the last tab. Shared by the bucket- and object-detail tab
/// cycling in [`StorageState::select_next_tab`].
fn next_detail_tab(current: StorageDetailTab) -> StorageDetailTab {
    let index = StorageDetailTab::ALL
        .iter()
        .position(|tab| *tab == current)
        .unwrap_or(0);
    StorageDetailTab::ALL[(index + 1) % StorageDetailTab::ALL.len()]
}

/// The tab that precedes `current` in `StorageDetailTab::ALL`, wrapping
/// around before the first tab. Shared by the bucket- and object-detail tab
/// cycling in [`StorageState::select_previous_tab`].
fn previous_detail_tab(current: StorageDetailTab) -> StorageDetailTab {
    let index = StorageDetailTab::ALL
        .iter()
        .position(|tab| *tab == current)
        .unwrap_or(0);
    StorageDetailTab::ALL[(index + StorageDetailTab::ALL.len() - 1) % StorageDetailTab::ALL.len()]
}

/// The values Cloud Storage's bucket filter searches for one row.
///
/// Borrows rather than owns: this runs once per bucket per `visible_*` call,
/// i.e. every frame, and neither caller needs the fields to outlive `bucket`.
fn bucket_searchable_fields(bucket: &StorageBucket) -> [&str; 3] {
    [
        bucket.name.as_str(),
        bucket.location.as_str(),
        bucket.storage_class.as_str(),
    ]
}

/// Cloud Storage's own narrowing, deliberately left as a plain lowercased
/// substring match rather than routed through the prefix grammar.
///
/// The object browser reloads on `filter_applied` used verbatim as a remote
/// object prefix when [`is_safe_remote_prefix`] admits it — and it admits
/// `-f api` and `!api` as literal prefixes. A grammar-aware client predicate
/// here would narrow a server result that had been fetched for a *different*
/// (literal) prefix, dropping rows. Keeping both sides literal keeps them
/// consistent. `filter` is expected lowercased by the caller.
fn bucket_matches_filter(bucket: &StorageBucket, filter: &str) -> bool {
    bucket_searchable_fields(bucket)
        .iter()
        .any(|value| value.to_lowercase().contains(filter))
}

/// Same fields, owned, for [`CompiledFilter::matches`]'s `&[String]`. The
/// allocation is confined to the trait boundary that requires it — the
/// per-frame [`StorageState::visible_buckets`] path stays borrowing.
fn owned_bucket_searchable_fields(bucket: &StorageBucket) -> Vec<String> {
    bucket_searchable_fields(bucket)
        .into_iter()
        .map(str::to_string)
        .collect()
}

fn compare_buckets(
    left: &StorageBucket,
    right: &StorageBucket,
    sort_field: StorageSortField,
    direction: SortDirection,
) -> std::cmp::Ordering {
    let primary = match sort_field {
        StorageSortField::Name => left.name.cmp(&right.name),
        StorageSortField::Location => left.location.cmp(&right.location),
        StorageSortField::StorageClass => left.storage_class.cmp(&right.storage_class),
        StorageSortField::Updated => left
            .updated
            .as_deref()
            .unwrap_or_default()
            .cmp(right.updated.as_deref().unwrap_or_default()),
    };
    let ordering = primary.then_with(|| left.name.cmp(&right.name));
    match direction {
        SortDirection::Ascending => ordering,
        SortDirection::Descending => ordering.reverse(),
    }
}

#[cfg(test)]
mod set_view_tests {
    use super::*;

    // `ResourceCollectionState::view_mode` is module-private and, for
    // storage, is never read back through any public accessor (unlike
    // compute/GKE/CloudSql/CloudRun, `ResourcesState::is_showing_detail`
    // dispatches storage through `StorageState::is_showing_detail`, which
    // reads `self.view`, not `self.collection.view_mode`). That makes the
    // invariant unobservable from an integration test in `tests/`, so these
    // are unit tests inside this module that read `state.collection
    // .view_mode` directly, mirroring the inline `#[cfg(test)]` style used
    // by `context_resource_cache_tests` in `mod.rs`.

    fn bucket(name: &str) -> StorageBucket {
        StorageBucket {
            name: name.to_string(),
            location: "US".to_string(),
            location_type: None,
            storage_class: "STANDARD".to_string(),
            versioning_enabled: false,
            uniform_bucket_level_access: true,
            public_access_prevention: None,
            time_created: None,
            updated: None,
        }
    }

    /// Reproduces the reducer-reachable gap: `OpenSelected` while
    /// `StorageView::BucketDetail` is active calls
    /// `StorageState::open_object_browser` (see
    /// `open_selected_storage` in `src/app/reducer/resources.rs`), which
    /// must leave `collection.view_mode` in sync with the new
    /// (non-detail) `view`.
    #[test]
    fn open_object_browser_from_bucket_detail_syncs_collection_view_mode_to_list() {
        let mut state = StorageState::default();
        state.collection.rows.push(bucket("bucket-a"));

        state.open_bucket_metadata();
        assert_eq!(state.view(), StorageView::BucketDetail);
        assert_eq!(state.collection.view_mode, ResourcesViewMode::Detail);

        state.open_object_browser();

        assert_eq!(state.view(), StorageView::ObjectBrowser);
        assert_eq!(
            state.collection.view_mode,
            ResourcesViewMode::List,
            "collection.view_mode must leave Detail once the object browser \
             is opened from BucketDetail"
        );
    }

    /// Reproduces the second reducer-reachable gap: a bucket-detail load
    /// failure forces `view = StorageView::BucketDetail` (see
    /// `EffectResult::StorageBucketDetailLoadFailed` handling in
    /// `src/app/reducer/root.rs`) without syncing `collection.view_mode`.
    #[test]
    fn set_bucket_detail_error_syncs_collection_view_mode_to_detail() {
        let mut state = StorageState::default();
        state.collection.rows.push(bucket("bucket-a"));

        state.set_bucket_detail_error("bucket-a".to_string(), "boom".to_string());

        assert_eq!(state.view(), StorageView::BucketDetail);
        assert_eq!(
            state.collection.view_mode,
            ResourcesViewMode::Detail,
            "collection.view_mode must reflect BucketDetail after a \
             detail-load error forces the view there"
        );
    }
}
