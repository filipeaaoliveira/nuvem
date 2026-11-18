//! BigQuery in application state: the datasets table, the tables listed inside
//! whichever dataset was entered, and the column schema of whichever table was
//! entered under that.
//!
//! # Three views behind one service kind
//!
//! `Enter` on a dataset row shows that dataset's tables; `Enter` on a table
//! row shows that table's columns; `Esc` comes back up one level at a time.
//! All three are lists, and all three live under the single
//! [`ResourceServiceKind::BigQueryDatasets`] — there is no second service
//! kind for tables and no third for schemas. That is the shape Cloud Storage
//! set with its object browser — `ResourceServiceKind` in
//! `crates/nuvem-domain/src/gcp/resource.rs` has one `CloudStorage` variant
//! and none for the browser — and the reason is the same: a tables view is
//! reachable only from a dataset and a schema view only from a table, so a
//! kind of its own would put it in the navigator, the palette and the jump
//! picker as a destination that can be selected with nothing chosen above it.
//!
//! [`BigQueryView`] says which list is on screen and
//! [`BigQueryDrilldownState`] holds it beside the two drill scopes, the same
//! way `StorageBrowserState` sits beside `StorageState`'s bucket collection.
//!
//! # Where the rows come from
//!
//! The datasets arrive through [`ResourcesState::set_bigquery_datasets`], fed
//! by one `bq ls -d` per load. The tables arrive through
//! [`ResourcesState::set_bigquery_tables`], fed by one `bq ls <dataset>` per
//! entered dataset. The columns arrive through
//! [`ResourcesState::set_bigquery_schema`], fed by one `bq show <table>` per
//! entered table — see `crate::services::resources::bigquery`. None of those
//! calls takes a server-side filter expression, so `/` narrows the loaded rows
//! client-side in all three views and a filter change issues no call at all.
//!
//! # Still list-only
//!
//! No view here has a detail pane: `Detail` and `DetailTab` are `()` for the
//! datasets collection, and `Enter` on a *schema* row opens nothing, because
//! a column is the bottom of this hierarchy.
use super::drill::DrillHierarchy;
use super::*;
use nuvem_domain::gcp::resource::BigQuerySchemaField;

/// Marker type unifying the BigQuery datasets service behind
/// [`ResourceService`]. Never instantiated.
pub enum BigQuery {}

impl ResourceService for BigQuery {
    type Row = BigQueryDataset;
    type Detail = ();
    type SortField = BigQuerySortField;
    type DetailTab = ();

    const KIND: ResourceServiceKind = ResourceServiceKind::BigQueryDatasets;

    fn collection(
        state: &ResourcesState,
    ) -> &ResourceCollectionState<Self::Row, Self::Detail, Self::SortField, Self::DetailTab> {
        &state.bigquery_datasets
    }

    fn collection_mut(
        state: &mut ResourcesState,
    ) -> &mut ResourceCollectionState<Self::Row, Self::Detail, Self::SortField, Self::DetailTab>
    {
        &mut state.bigquery_datasets
    }

    fn matches_filter(row: &Self::Row, filter: &CompiledFilter) -> bool {
        dataset_matches_filter(row, filter)
    }

    /// The needle sits in the dataset id, the identity column of the dataset
    /// table. Legal there because the constant is letters only: a dataset id
    /// admits no hyphen, which is why the needle carries none.
    fn sample_row() -> Self::Row {
        BigQueryDataset {
            dataset_id: SAMPLE_ROW_NEEDLE.to_string(),
            project_id: "sample-project".to_string(),
            location: "EU".to_string(),
            friendly_name: None,
            labels: Vec::new(),
        }
    }

    fn compare(
        left: &Self::Row,
        right: &Self::Row,
        sort_field: Self::SortField,
        direction: SortDirection,
    ) -> std::cmp::Ordering {
        compare_datasets(left, right, sort_field, direction)
    }

    /// A dataset id is unique within its project and the table lists one
    /// project at a time, so the id alone is the identity. The location is
    /// deliberately *not* folded in: it is fixed for the life of a dataset,
    /// so it can add nothing to an identity the id already settles, and a
    /// row whose location came back empty would otherwise get a second
    /// identity from the same dataset.
    fn row_id(row: &Self::Row) -> ResourceRowId {
        ResourceRowId {
            name: row.dataset_id.clone(),
            location: String::new(),
        }
    }
}

/// Which of the three BigQuery lists is on screen.
///
/// All three are lists, so none maps to [`ResourcesViewMode::Detail`] — see
/// `BigQueryDrilldownState::restore_invariant`.
///
/// The variants are the levels in descent order, and the order is the
/// hierarchy: `Schema` is reachable only from `Tables`, which is reachable
/// only from `Datasets`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum BigQueryView {
    #[default]
    Datasets,
    Tables,
    Schema,
}

/// Which drill a tables listing or a schema load belongs to.
///
/// The dataset id cannot answer that question: re-entering the same dataset
/// is a second drill with the same id, and a scope check keyed on the id
/// accepts the first drill's answer over the second drill's state. The same
/// holds a level down for a table id. A counter bumped on every view
/// transition gives each drill an identity nothing else can collide with, so
/// a result is either this drill's or it is dropped.
///
/// Opaque on purpose. `BigQueryDrilldownState::set_view` advances the
/// counter; the three functions that hand a value out are
/// [`ResourcesState::open_selected_bigquery_dataset`],
/// [`ResourcesState::reload_bigquery_tables`] and
/// [`ResourcesState::open_selected_bigquery_table`], each returning the live
/// one on a request. From there it is carried out on the effect and handed
/// back on the result.
///
/// Nothing outside this module needs to read the number, and nothing anywhere
/// may invent one: the field is private, there is no constructor, and there
/// is deliberately no `Default` — the only ways to hold a generation are to
/// receive one from a request or a result, or to have read the live one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BigQueryDrillGeneration(u64);

impl BigQueryDrillGeneration {
    fn next(self) -> Self {
        Self(self.0.wrapping_add(1))
    }
}

/// Which *listing call* a per-table enrichment belongs to.
///
/// [`BigQueryDrillGeneration`] identifies a drill and deliberately survives
/// `r`: [`ResourcesState::reload_bigquery_tables`] re-lists the dataset the
/// operator is already inside without bumping it. That is right for the
/// listing and not enough for the fan-out over its rows. A `bq show` issued
/// from the first listing still matches the generation when it returns, and
/// the id match in [`ResourcesState::merge_bigquery_table_enrichment`] does
/// not separate them either: a table present in both listings has the same id
/// in both, so the discarded call's numbers land on the retry's row and the
/// operator reads pre-retry data as the answer to the refresh they asked for.
///
/// So each listing *request* — entry and retry alike — mints an epoch, and a
/// fan-out carries the epoch of the listing it was fanned out from. A merge
/// happens only while that epoch *and* the drill it was minted inside are both
/// still live: `Esc` moves the generation and mints no epoch, so the epoch on
/// its own would still be the live one after the drill is gone.
///
/// Opaque and `Default`-less for the same reasons the generation is: the only
/// ways to hold one are to receive it on a request or a result, or to read the
/// live one, so nothing can invent a value that collides with a live listing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BigQueryListingEpoch(u64);

impl BigQueryListingEpoch {
    fn next(self) -> Self {
        Self(self.0.wrapping_add(1))
    }
}

/// What a tables listing has to be issued for: the dataset to ask `bq ls`
/// about, and the identity the answer will be checked against.
///
/// All three travel together because they are minted together — a dataset id
/// resolved without the generation and epoch that were live when it was
/// resolved is exactly the stale-scope bug this type exists to prevent.
///
/// The epoch was once left off, on the argument that nothing reads it until
/// the fan-out is being built and so it could be read from the live counter
/// then. That argument produced a bug. `r` pressed while the first `bq ls` is
/// still in flight mints a second epoch, and with no epoch on the result the
/// first listing still landed — installing the discarded call's rows and
/// fanning out over them under the retry's epoch, so both fan-outs matched,
/// the abandoned one's shows merged into the retry's rows, and the dataset
/// paid 2N calls. See [`ResourcesState::matches_bigquery_tables_scope`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BigQueryTablesRequest {
    pub dataset_id: String,
    pub generation: BigQueryDrillGeneration,
    pub listing: BigQueryListingEpoch,
}

/// What one per-table `bq show` has to be issued for: the table to ask about,
/// and the identity its answer will be checked against.
///
/// Both halves of that identity are here because they answer different
/// questions. The generation says *which drill* — a show for a dataset the
/// operator has left must not land, and nothing else here can say that,
/// because `BigQueryDrilldownState::set_view` moves the generation and
/// leaves the epoch alone. The epoch says *which listing inside that drill* —
/// a show from before an `r` retry must not land either, and the generation
/// cannot see that, because a retry is not a new drill. See
/// [`ResourcesState::matches_bigquery_enrichment_scope`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BigQueryTableStatsRequest {
    pub dataset_id: String,
    pub table_id: String,
    pub generation: BigQueryDrillGeneration,
    pub listing: BigQueryListingEpoch,
}

/// What one schema load has to be issued for: the table to ask `bq show`
/// about, and the identity its answer will be checked against.
///
/// The generation and nothing else. That is the whole difference from
/// [`BigQueryTableStatsRequest`], which carries a listing epoch too, and the
/// asymmetry is deliberate: the epoch separates two *listings of one dataset*,
/// which is a question the tables level asks because `r` re-lists without
/// leaving the drill. Entering a table is a view transition, so it mints a
/// generation of its own; `r` inside the schema view would do the same by
/// re-entering rather than by minting a second epoch under one drill. There
/// is nothing here for an epoch to distinguish, so there is no epoch. See
/// [`ResourcesState::matches_bigquery_schema_scope`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BigQuerySchemaRequest {
    pub dataset_id: String,
    pub table_id: String,
    pub generation: BigQueryDrillGeneration,
}

/// The cursor and the applied filter of one BigQuery list.
///
/// Split out of [`DrillLevel`] rather than left inline because it is the part
/// of a level that is *not* parameterised by what the level lists. Thirteen of
/// the fourteen routing points in the "generic list surface" section below
/// read or write exactly these four fields, so a single non-generic
/// [`BigQueryDrilldownState::drilled_cursor_mut`] can hand back whichever
/// level is on screen — which is what stops those thirteen needing a `Tables`
/// arm and a byte-identical `Schema` arm beside it.
///
/// A generic `&mut DrillLevel<..>` could not do that job: the two levels are
/// two distinct types, so one function cannot return a reference to either.
/// The cursor is the widest common part that stays one type.
///
/// # What this seam deliberately does not cover
///
/// Stated as a list rather than as "every routing point", because an absolute
/// here is what a later reader would rely on when deciding whether a new
/// accessor may skip the cursor — and two things in that section legitimately
/// reach past it. Needing either is not a sign the accessor is written wrong:
///
/// - **A count of rows.** [`ResourcesState::bigquery_visible_len`] answers
///   that by naming all three views, because the count depends on the row type
///   and on which `*_matches_filter` predicate applies to it, and this struct
///   holds neither. Three of the thirteen — `bigquery_select_next`,
///   `bigquery_selected_visible_index` and `bigquery_apply_filter` — use the
///   cursor *and* that count, which is also why [`Self::apply_filter`] does
///   not clamp `selected_visible` itself.
/// - **The sort field.** [`ResourcesState::bigquery_sort_label`] is the
///   fourteenth routing point and the only one that touches no cursor at all.
///   It reads the datasets collection's sort field, and its `Tables` and
///   `Schema` arms are two different string literals rather than one shared
///   body — so it was never part of the duplication this seam removed, and
///   collapsing it would mean losing an answer, not a copy.
///
/// Both of those carry the mirror of this note, so neither side can go stale
/// on its own.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct DrillCursor {
    selected_visible: usize,
    filter_input_active: bool,
    filter_draft: String,
    filter_applied: CompiledResourceFilter,
}

impl DrillCursor {
    /// Opens the filter prompt over what is currently applied, so the operator
    /// edits the live expression rather than starting from an empty line.
    fn start_filter(&mut self) {
        self.filter_input_active = true;
        self.filter_draft = self.filter_applied.source().to_string();
    }

    /// Compiles the draft and closes the prompt.
    ///
    /// The draft is then read *back* out of the compiled filter rather than
    /// left as the operator typed it, which is what makes the trim visible on
    /// the next `/`: the applied expression and the draft shown for editing
    /// are the same string. The cursor is deliberately not clamped here —
    /// clamping needs the visible count, which only the caller can compute,
    /// because it depends on the row type this cursor knows nothing about.
    fn apply_filter(&mut self) {
        self.filter_applied = CompiledResourceFilter::new(self.filter_draft.trim().to_string());
        self.filter_draft = self.filter_applied.source().to_string();
        self.filter_input_active = false;
    }

    /// Abandons the draft and closes the prompt, leaving the applied filter —
    /// and so the rows on screen — exactly as they were.
    fn cancel_filter(&mut self) {
        self.filter_draft = self.filter_applied.source().to_string();
        self.filter_input_active = false;
    }
}

/// One drilled level: the row it was entered from, the rows that were loaded
/// under it, and this level's own cursor, filter and failure.
///
/// The cursor and the filter are this level's alone, deliberately: each level
/// above keeps its own behind the drill, so `Esc` lands the operator back on
/// the row they entered from rather than on whatever row this level's cursor
/// happened to reach, and a filter typed here does not survive into the list
/// behind it.
///
/// # Why this is generic, and why only here
///
/// The tables level and the schema level held two structs with six identical
/// fields, a `rows` differing only in element type, and a parent handle
/// differing only in name — with the consequence that thirteen routing
/// functions carried a `Schema` arm that was the `Tables` arm with one field
/// path changed. Both were collapsed into this one type.
///
/// It is scoped to this module on purpose and **Cloud Storage must not be
/// moved onto it.** That was measured during the BigQuery slice and rejected on the
/// evidence: `StorageBrowserState` splits its rows into `prefixes` and
/// `objects` unified only at read time by a borrowed `StorageBrowserRow<'a>`,
/// its applied filter is a `String` beside a `remote_filter` pushdown flag
/// rather than a `CompiledResourceFilter` alone, and it paginates
/// (`next_page_token`, `page_index`). So of the three fields that make this
/// type worth having — `rows`, `cursor.filter_applied`, `parent` — it shares
/// only the last. That measurement answered a *cross-service* question and this type
/// answers a *within-BigQuery* one; they are not the same question, and
/// re-litigating the first one is not a reason to reopen the second.
#[derive(Debug, Clone, PartialEq, Eq)]
struct DrillLevel<Parent, Row> {
    /// The row one level up that was entered to reach this level: the dataset
    /// for the tables list, the table for the schema list. `None` above the
    /// level — [`BigQueryDrilldownState::restore_invariant`] keeps those two
    /// the same question.
    parent: Option<Parent>,
    rows: Vec<Row>,
    cursor: DrillCursor,
    problem: Option<DiagnosticProblem>,
    loading: bool,
}

/// Hand-written rather than derived because `#[derive(Default)]` on a generic
/// struct demands `Parent: Default` and `Row: Default`, and neither
/// [`BigQueryDataset`] nor [`BigQueryTable`] has a `Default` — nor should
/// acquire one to satisfy a derive, since "the default dataset" is not a
/// thing. Nothing here needs those bounds: an absent parent is `None` and an
/// unloaded level has no rows.
impl<Parent, Row> Default for DrillLevel<Parent, Row> {
    fn default() -> Self {
        Self {
            parent: None,
            rows: Vec::new(),
            cursor: DrillCursor::default(),
            problem: None,
            loading: false,
        }
    }
}

/// The drill scope: the dataset that was entered and the tables
/// `bq ls <dataset>` returned for it.
type BigQueryTablesState = DrillLevel<BigQueryDataset, BigQueryTable>;

/// The schema scope: the table that was entered and the columns `bq show`
/// flattened out of it.
type BigQuerySchemaState = DrillLevel<BigQueryTable, BigQuerySchemaField>;

/// The BigQuery view and the drill scope it selects between, held beside the
/// datasets collection on [`ResourcesState`] the way `StorageBrowserState` is
/// held beside Cloud Storage's bucket collection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct BigQueryDrilldownState {
    view: BigQueryView,
    tables: BigQueryTablesState,
    schema: BigQuerySchemaState,
    /// The drill currently on screen. Lives out here rather than on
    /// `tables` because `set_view` resets `tables` wholesale when the view
    /// returns to the datasets list, and a generation that reset with it
    /// would hand the next drill an identity a stale result already carries.
    generation: BigQueryDrillGeneration,
    /// The listing call currently on screen, out here for the same reason the
    /// generation is. Bumped by every listing *request* rather than by every
    /// view transition, which is the one thing that distinguishes it from the
    /// generation — see [`BigQueryListingEpoch`].
    listing: BigQueryListingEpoch,
}

/// Hand-written rather than derived because [`BigQueryDrillGeneration`] has
/// no `Default`: this is the one place that names a starting generation, and
/// keeping it the only one is what makes "nothing anywhere may invent one"
/// true. The value it starts at is arbitrary — the counter is compared for
/// equality and never read — but it is reached only through this struct, so
/// no caller can construct a generation that collides with a live drill's.
impl Default for BigQueryDrilldownState {
    fn default() -> Self {
        Self {
            view: BigQueryView::default(),
            tables: BigQueryTablesState::default(),
            schema: BigQuerySchemaState::default(),
            generation: BigQueryDrillGeneration(0),
            listing: BigQueryListingEpoch(0),
        }
    }
}

impl BigQueryDrilldownState {
    /// Records that a listing call is being issued, and returns the epoch it
    /// is being issued under.
    ///
    /// Called by both functions that mint a [`BigQueryTablesRequest`] and by
    /// nothing else, which is what makes "one listing, one epoch" true. It is
    /// deliberately *not* folded into `set_view`: entering bumps both counters
    /// and `r` bumps only this one, and that difference is the whole point —
    /// see [`BigQueryListingEpoch`].
    fn begin_listing(&mut self) -> BigQueryListingEpoch {
        self.listing = self.listing.next();
        self.listing
    }

    /// The cursor and filter of the drilled level on screen, and `None` on the
    /// datasets list — whose cursor and filter live on the shared
    /// [`ResourceCollectionState`] every service has, not on a [`DrillLevel`].
    ///
    /// This and [`Self::drilled_cursor_mut`] are the two places that route by
    /// view for the whole "generic list surface" section below. Before the collapse
    /// each of those thirteen functions matched all three views itself, with a
    /// `Schema` arm that was the `Tables` arm with one field path changed;
    /// now they ask this one question and get one answer for both drilled
    /// levels.
    ///
    /// The `None` for `Datasets` is what keeps the two-arm callers honest
    /// about a fourth level: the exhaustive match is *here*, so a variant
    /// added to [`BigQueryView`] fails to compile in this function and its
    /// mutable twin — where the answer for the new level is decided once —
    /// rather than in thirteen places that would each need the same edit.
    fn drilled_cursor(&self) -> Option<&DrillCursor> {
        match self.view {
            BigQueryView::Datasets => None,
            BigQueryView::Tables => Some(&self.tables.cursor),
            BigQueryView::Schema => Some(&self.schema.cursor),
        }
    }

    /// The mutable half of [`Self::drilled_cursor`]; see its note for why the
    /// exhaustive match lives here and not in the callers.
    fn drilled_cursor_mut(&mut self) -> Option<&mut DrillCursor> {
        match self.view {
            BigQueryView::Datasets => None,
            BigQueryView::Tables => Some(&mut self.tables.cursor),
            BigQueryView::Schema => Some(&mut self.schema.cursor),
        }
    }
}

impl DrillHierarchy for BigQueryDrilldownState {
    type View = BigQueryView;

    fn stored_view(&self) -> Self::View {
        self.view
    }

    fn store_view(&mut self, view: Self::View) {
        self.view = view;
    }

    /// Two things, both load-bearing.
    ///
    /// The generation bump gives each drill an identity: a result minted
    /// under an earlier one is dropped rather than merged, which is what
    /// stops a departed drill's `bq show` landing on the rows now on screen.
    /// It happens on *every* transition rather than only on entry into the
    /// tables view, which is what lets the scope check be a single equality.
    /// Bumping only on entry would leave the generation a drill was issued
    /// under still live after `Esc`, so a listing that arrived after leaving
    /// would match, and the check would need a second conjunct on the view to
    /// say what one counter already says.
    ///
    /// The resets enforce *"a level's loaded rows exist only while that level
    /// or one below it is on screen"*. Without the tables reset, `Esc` then
    /// `Enter` on a different dataset would paint the previous dataset's
    /// tables under the new dataset's header for as long as the fresh `bq ls`
    /// takes to return. Without the schema reset, `Esc` then `Enter` on a
    /// different table would paint the previous table's columns under the new
    /// table's header for as long as the fresh `bq show` takes to return.
    ///
    /// They are written as an ascent rather than as one reset per level:
    /// arriving at `Tables` clears what lives below it, and arriving at
    /// `Datasets` clears everything below it. Arriving at `Schema` clears
    /// nothing, because the tables list behind it is exactly what `Esc` has to
    /// come back to still populated.
    ///
    /// Unlike Cloud Storage's implementation this syncs no `view_mode`:
    /// BigQuery has no detail pane, so the datasets collection's `view_mode`
    /// is never assigned anywhere and stays `List` under all three views. The
    /// unit test below pins that, so a detail view added later cannot leave
    /// the assumption silently false.
    fn restore_invariant(&mut self, view: Self::View) {
        self.generation = self.generation.next();
        if view == BigQueryView::Datasets || view == BigQueryView::Tables {
            self.schema = BigQuerySchemaState::default();
        }
        if view == BigQueryView::Datasets {
            self.tables = BigQueryTablesState::default();
        }
    }
}

/// The sortable columns of the datasets table.
///
/// One variant per *sortable* catalogue column. The catalogue has three columns
/// and two of them sort: `labels` does not, for the reason recorded beside
/// `BIGQUERY_DATASET_COLUMNS`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BigQuerySortField {
    DatasetId,
    Location,
}

impl BigQuerySortField {
    pub fn label(self) -> &'static str {
        match self {
            Self::DatasetId => "Dataset",
            Self::Location => "Location",
        }
    }

    pub const COLUMN_COUNT: usize = 3;

    pub fn from_column(index: usize) -> Option<Self> {
        match index {
            0 => Some(Self::DatasetId),
            1 => Some(Self::Location),
            // 2 is `labels`, the one catalogue column with no sort field.
            _ => None,
        }
    }

    pub fn column(self) -> usize {
        match self {
            Self::DatasetId => 0,
            Self::Location => 1,
        }
    }
}

impl ResourcesState {
    pub fn bigquery_datasets(&self) -> &[BigQueryDataset] {
        &self.bigquery_datasets.rows
    }

    pub fn visible_bigquery_datasets(&self) -> Vec<&BigQueryDataset> {
        let filter = self.bigquery_datasets.filter_applied.filter();
        let mut rows = self
            .bigquery_datasets
            .rows
            .iter()
            .filter(|dataset| dataset_matches_filter(dataset, filter))
            .collect::<Vec<_>>();
        rows.sort_by(|left, right| {
            compare_datasets(
                left,
                right,
                self.bigquery_datasets.sort_field,
                self.bigquery_datasets.sort_direction,
            )
        });
        rows
    }

    pub fn visible_bigquery_dataset_count(&self) -> usize {
        let filter = self.bigquery_datasets.filter_applied.filter();
        self.bigquery_datasets
            .rows
            .iter()
            .filter(|dataset| dataset_matches_filter(dataset, filter))
            .count()
    }

    pub fn selected_bigquery_dataset(&self) -> Option<&BigQueryDataset> {
        self.visible_bigquery_datasets()
            .get(self.bigquery_datasets.selected_visible)
            .copied()
    }

    /// Installs a freshly loaded list, keeping the cursor on the dataset it
    /// was already on where that dataset is still listed.
    pub fn set_bigquery_datasets(
        &mut self,
        context: GcloudContext,
        datasets: Vec<BigQueryDataset>,
    ) {
        let previously_selected = self
            .selected_bigquery_dataset()
            .map(|dataset| dataset.dataset_id.clone());

        self.bigquery_datasets.context = Some(context);
        self.bigquery_datasets.rows = datasets;
        self.bigquery_datasets.stale = false;
        self.bigquery_datasets.last_problem = None;
        self.bigquery_datasets.last_diagnostic = None;
        self.bigquery_datasets.clear_load_phase();

        self.bigquery_datasets.selected_visible = previously_selected
            .as_ref()
            .and_then(|dataset_id| {
                self.visible_bigquery_datasets()
                    .iter()
                    .position(|dataset| &dataset.dataset_id == dataset_id)
            })
            .unwrap_or_else(|| {
                self.bigquery_datasets
                    .selected_visible
                    .min(self.visible_bigquery_datasets().len().saturating_sub(1))
            });
    }

    pub fn mark_bigquery_datasets_stale_and_clear(&mut self) {
        // The drill scope is a dataset of the project being left, so it goes
        // with the rows. Through `set_view`, which is what also drops the
        // tables listed inside it.
        self.bigquery.set_view(BigQueryView::Datasets);
        self.bigquery_datasets.filter_draft =
            self.bigquery_datasets.filter_applied.source().to_string();
        self.bigquery_datasets.filter_input_active = false;
        self.bigquery_datasets.rows.clear();
        self.bigquery_datasets.context = None;
        self.bigquery_datasets.selected_visible = 0;
        self.bigquery_datasets.stale = true;
    }

    pub fn bigquery_datasets_problem(&self) -> Option<&DiagnosticProblem> {
        self.bigquery_datasets.last_problem.as_ref()
    }

    // ---- the tables drill-down ----

    pub fn bigquery_view(&self) -> BigQueryView {
        self.bigquery.stored_view()
    }

    /// The dataset the tables view is inside, and `None` on the datasets
    /// list — `BigQueryDrilldownState::set_view` keeps those two the same
    /// question.
    pub fn bigquery_tables_dataset(&self) -> Option<&BigQueryDataset> {
        self.bigquery.tables.parent.as_ref()
    }

    pub fn bigquery_tables(&self) -> &[BigQueryTable] {
        &self.bigquery.tables.rows
    }

    /// The loaded tables the filter admits, in the order `bq ls` listed them.
    ///
    /// Unsorted, unlike [`Self::visible_bigquery_datasets`]. The tables table
    /// has columns now, but none of them sorts: `BIGQUERY_TABLE_COLUMNS`
    /// carries no sort field at all (its type is uninhabited), the tools line
    /// over the view advertises neither `s` nor the column cursor, and the
    /// rows are painted in the order `bq ls` returned them.
    pub fn visible_bigquery_tables(&self) -> Vec<&BigQueryTable> {
        let filter = self.bigquery.tables.cursor.filter_applied.filter();
        self.bigquery
            .tables
            .rows
            .iter()
            .filter(|table| table_matches_filter(table, filter))
            .collect()
    }

    pub fn visible_bigquery_table_count(&self) -> usize {
        self.visible_bigquery_tables().len()
    }

    pub fn selected_bigquery_table(&self) -> Option<&BigQueryTable> {
        self.visible_bigquery_tables()
            .get(self.bigquery.tables.cursor.selected_visible)
            .copied()
    }

    pub fn bigquery_tables_loading(&self) -> bool {
        self.bigquery.tables.loading
    }

    pub fn bigquery_tables_problem(&self) -> Option<&DiagnosticProblem> {
        self.bigquery.tables.problem.as_ref()
    }

    /// Enters the selected dataset, returning the request the caller must
    /// issue: the dataset the view now claims to be in, and the drill the
    /// answer belongs to.
    ///
    /// `None` — and no transition — when there is nothing selected (an empty
    /// or wholly filtered-out list), or when this is not the datasets list:
    /// `Enter` a level down means something else, and
    /// [`Self::open_selected_bigquery_table`] is what it means.
    pub fn open_selected_bigquery_dataset(&mut self) -> Option<BigQueryTablesRequest> {
        if self.bigquery.view != BigQueryView::Datasets {
            return None;
        }
        let dataset = self.selected_bigquery_dataset()?.clone();
        let dataset_id = dataset.dataset_id.clone();
        self.bigquery.set_view(BigQueryView::Tables);
        self.bigquery.tables.parent = Some(dataset);
        self.bigquery.tables.loading = true;
        let listing = self.bigquery.begin_listing();
        Some(BigQueryTablesRequest {
            dataset_id,
            generation: self.bigquery.generation,
            listing,
        })
    }

    /// Ascends one level of the drill: the schema view leaves for the tables
    /// list it was entered from, the tables list for the datasets list.
    ///
    /// One level at a time, and written as a match so that a level added
    /// below this hierarchy has to say where it ascends to rather than
    /// inheriting a guard that happens to compile. Dropping straight to the
    /// datasets list from the schema view would take the operator two levels
    /// for one keypress, and drop the tables listing they were reading on the
    /// way — the same collapse the Cloud Storage note on
    /// [`ResourcesState::is_showing_drilldown`] refuses.
    ///
    /// A no-op on the datasets list, and that path is genuinely reachable:
    /// `Esc` is gated on `resources_drilldown_open` and so cannot arrive
    /// here without a drill, but `-`/`Backspace` are dispatched for the whole
    /// resources list, so pressing `-` on the datasets list with nothing
    /// drilled into lands here with `view == Datasets`.
    ///
    /// It still returns nothing. A "there was nothing to leave" flag would
    /// have no reader: both reducer arms that call this issue no effect
    /// either way, and the two keys that reach it mean "go up" — a key
    /// pressed at the top has already done what it says.
    pub fn leave_bigquery_drilldown(&mut self) {
        match self.bigquery.view {
            BigQueryView::Datasets => {}
            BigQueryView::Tables => self.bigquery.set_view(BigQueryView::Datasets),
            BigQueryView::Schema => self.leave_bigquery_schema(),
        }
    }

    /// Re-issues the listing for the dataset the view is inside, returning the
    /// request so the caller can build the effect for exactly that dataset,
    /// under the drill that is on screen.
    ///
    /// The generation is not bumped: a retry is another call for the drill the
    /// operator is already inside, not a new drill, and bumping here would
    /// invalidate the very listing this retries alongside its own.
    ///
    /// `None` — and no state change — anywhere but the tables view, which is
    /// what makes `r` a *retry of this call* only while that view is on
    /// screen. The failure state paints "Press r to retry." and this is what
    /// makes that sentence true; without it `r` would refresh the datasets
    /// list behind the drill and leave the failed listing on screen.
    ///
    /// The guard names `Tables` rather than refusing `Datasets`, because the
    /// schema view is neither: a listing issued from there lands *while the
    /// columns are on screen*, and — since this deliberately does not bump the
    /// generation — [`Self::matches_bigquery_tables_scope`] would accept it.
    /// `r` two levels down is [`Self::reload_bigquery_schema`].
    pub fn reload_bigquery_tables(&mut self) -> Option<BigQueryTablesRequest> {
        if self.bigquery.view != BigQueryView::Tables {
            return None;
        }
        let dataset_id = self.bigquery.tables.parent.as_ref()?.dataset_id.clone();
        // The rows are left alone: a refresh over a populated list keeps it
        // painted until the fresh one lands, the way every other service's
        // does. The problem is cleared because it is about the call being
        // retried.
        self.bigquery.tables.problem = None;
        self.bigquery.tables.loading = true;
        // The generation is untouched (above); the listing epoch is not. This
        // is a second call for the same drill, and the enrichments fanned out
        // from the call it replaces are answers about a listing the operator
        // asked to discard.
        let listing = self.bigquery.begin_listing();
        Some(BigQueryTablesRequest {
            dataset_id,
            generation: self.bigquery.generation,
            listing,
        })
    }

    /// Whether a listing minted under `generation` and `listing` is still the
    /// one this view is waiting on.
    ///
    /// The generation is what a slow listing for a dataset the operator has
    /// since left fails, the same guard Cloud Storage's object pages carry —
    /// and what a listing from an earlier drill into the *same* dataset fails,
    /// which is why this is a generation check and not a dataset-id one.
    ///
    /// The epoch is what a listing the operator replaced with `r` fails, and
    /// the generation cannot stand in for it: `reload_bigquery_tables`
    /// deliberately does not bump the generation, so `r` pressed while the
    /// first `bq ls` is still in flight leaves both listings matching it.
    /// Both would then install their rows, and — since the fan-out over each
    /// carries the identity that listing was accepted under — the discarded
    /// call's shows would merge into the retry's rows.
    pub fn matches_bigquery_tables_scope(
        &self,
        generation: BigQueryDrillGeneration,
        listing: BigQueryListingEpoch,
    ) -> bool {
        self.bigquery.generation == generation && self.bigquery.listing == listing
    }

    /// Whether an enrichment minted under `generation` and `listing` is still
    /// one this view wants.
    ///
    /// The same two conjuncts as [`Self::matches_bigquery_tables_scope`], and
    /// as of the fix that restored the generation here, the same body. They
    /// stay two functions because they answer two questions — one about a
    /// listing this view would install, one about an enrichment it would
    /// merge — and each is killed by its own test; a single shared function
    /// would be killed by either, which is weaker. The cost is that the two
    /// bodies are now byte-identical, so an anchored edit or a hand-mutation
    /// can land on the wrong one and still look right: pin such an edit to a
    /// line, and read which test fails rather than the diffstat.
    ///
    /// Both conjuncts are load-bearing, though only under the current bump
    /// policy: `set_view` moves one counter and `begin_listing` the other, so
    /// neither implies the other. They are read only ever as a pair, and one
    /// counter bumped on both events would be an equivalent predicate — the
    /// pair is kept because it gives each conjunct its own killable mutant,
    /// and it survives a future path that bumps one without the other.
    ///
    /// The generation drops a show for a drill that has been left.
    /// `BigQueryDrilldownState::set_view` bumps the generation and never
    /// touches the epoch — `begin_listing` is the only thing that moves the
    /// epoch, and `Esc` issues no listing — so after `Esc` the abandoned
    /// drill's epoch is still the live one. Without this conjunct that drill's
    /// shows are still in scope.
    ///
    /// The epoch drops a show for a listing that has been re-issued inside the
    /// drill that is still open: `r` mints a new epoch without bumping the
    /// generation, so the generation cannot tell a show issued before the
    /// retry from one issued after it.
    ///
    /// The generation conjunct was deleted once, on the grounds that
    /// hand-mutating it away killed no test. That much was true; the reason
    /// given for it was not. It killed nothing because `set_view` *also*
    /// empties `tables.rows`, so a post-`Esc` enrichment that passes this
    /// guard then finds no row to merge into and writes nothing — the safety
    /// was another function's clearing behaviour, not this check. Anything
    /// that preserved rows across the transition (a re-entry cache, keeping
    /// the last dataset painted) would have turned that into a silent
    /// cross-drill merge. `an_enrichment_for_a_drill_that_was_left_is_out_of_scope`
    /// in `tests/bigquery_enrichment_test.rs` asks this function directly and
    /// is what kills the conjunct now.
    pub fn matches_bigquery_enrichment_scope(
        &self,
        generation: BigQueryDrillGeneration,
        listing: BigQueryListingEpoch,
    ) -> bool {
        self.bigquery.generation == generation && self.bigquery.listing == listing
    }

    /// One `bq show` request per loaded table, under the identity of the
    /// listing that produced them.
    ///
    /// `generation` and `listing` are the accepted result's, passed in rather
    /// than read from the live counters, so a request says which listing it
    /// was fanned out from and not merely which listing was live when it was
    /// built. The caller has already checked the two against the live ones
    /// through [`Self::matches_bigquery_tables_scope`] — but a fan-out that
    /// read the live counters instead would go on being stamped "live"
    /// whatever that check later became, which is how the discarded listing's
    /// shows once ended up carrying the retry's epoch.
    ///
    /// The *rows* are read live, and that is the other half deliberately: the
    /// caller has just installed them, so the set asked about and the set on
    /// screen are one read of one field and cannot drift.
    ///
    /// The fan-out is *exactly* the listing: every loaded row, once, in the
    /// order `bq ls` returned them. Not the visible rows — a row the filter is
    /// hiding is still a listed row, and it must not need a filter change to
    /// acquire its numbers. Nothing bounds the count: a dataset with 200
    /// tables mints 200 requests, because a cap would leave the rows past it
    /// unavailable in a way no operator could tell apart from a refusal.
    pub fn bigquery_table_stat_requests(
        &self,
        generation: BigQueryDrillGeneration,
        listing: BigQueryListingEpoch,
    ) -> Vec<BigQueryTableStatsRequest> {
        self.bigquery
            .tables
            .rows
            .iter()
            .map(|row| BigQueryTableStatsRequest {
                dataset_id: row.dataset_id.clone(),
                table_id: row.table_id.clone(),
                generation,
                listing,
            })
            .collect()
    }

    /// Installs a freshly listed set of tables, resetting the cursor: the rows
    /// are a different dataset's or a re-listing of the same one, and there is
    /// no per-row identity to hold a cursor on that a `bq show` would confirm.
    pub fn set_bigquery_tables(&mut self, tables: Vec<BigQueryTable>) {
        self.bigquery.tables.rows = tables;
        self.bigquery.tables.cursor.selected_visible = 0;
        self.bigquery.tables.problem = None;
        self.bigquery.tables.loading = false;
    }

    /// Merges an enrichment into the listed row whose table id is `table_id`,
    /// and touches nothing else.
    ///
    /// This is deliberately not [`Self::set_bigquery_tables`] widened. That
    /// one resets the cursor to 0, clears the problem and clears the loading
    /// flag, which is right for a *listing* — the rows are a different set —
    /// and wrong for an answer about one row: it would yank the operator's
    /// cursor back to the top and erase a problem the listing reported. The
    /// only assignment this makes is to the matched row, so the cursor, both
    /// filters, the problem and the loading flag are exactly as they were.
    ///
    /// The match runs over the loaded rows, not the visible ones: a row the
    /// filter is hiding is still a listed row, and an enrichment issued for
    /// it before the filter was typed has to land on it.
    ///
    /// # What this assumes about identity
    ///
    /// Stated for the fan-out that will call it:
    ///
    /// - **The table id identifies the row.** `bq ls <dataset>` lists one
    ///   dataset and a table id is unique within a dataset, so at most one
    ///   loaded row can match. The dataset is not re-checked here because the
    ///   rows are one dataset's by construction —
    ///   `BigQueryDrilldownState::set_view` drops them on the way out of
    ///   the drill.
    /// - **An id that matches no loaded row is dropped, never appended.** The
    ///   rows come from the listing alone; an enrichment is an answer *about*
    ///   a row and never a reason for one to exist. A `bq show` for a table
    ///   the listing has since stopped returning would otherwise resurrect it
    ///   as a row with no kind, no created time and no labels.
    /// - **The scope is the caller's check, not this one's.** The caller must
    ///   have accepted the result through
    ///   [`Self::matches_bigquery_enrichment_scope`] first; nothing here can
    ///   tell whose enrichment it is being handed.
    /// - **At this level a generation identifies a drill, not a call**, which
    ///   is why that check takes a second argument.
    ///   [`Self::reload_bigquery_tables`] does not bump the generation, so a
    ///   show issued before an `r` retry and one issued after are
    ///   indistinguishable by generation, and by table id too — the same table
    ///   has the same id in both listings. The listing epoch is what separates
    ///   them; see [`BigQueryListingEpoch`]. The claim is scoped to the tables
    ///   level deliberately: one level down,
    ///   [`Self::reload_bigquery_schema`] answers `r` by re-entering, so there
    ///   a generation does identify a call — which is exactly why that level
    ///   needs no epoch beside it.
    ///
    /// `enrich` is handed the whole row rather than the three stat fields,
    /// which keeps this setter indifferent to what an enrichment carries. The
    /// cost is that nothing here stops a closure writing a field it was not
    /// given the row for: rewriting `table_id` breaks the match for every
    /// later enrichment of that row, and rewriting `kind` or `labels` — which
    /// `table_searchable_fields` reads — changes which rows an applied
    /// filter shows, moving the cursor without touching the field this
    /// docstring promises to leave alone. The stat fields are not searchable,
    /// so the fan-out's own use is clear of that.
    pub fn merge_bigquery_table_enrichment(
        &mut self,
        table_id: &str,
        enrich: impl FnOnce(&mut BigQueryTable),
    ) {
        let Some(row) = self
            .bigquery
            .tables
            .rows
            .iter_mut()
            .find(|row| row.table_id == table_id)
        else {
            return;
        };
        enrich(row);
    }

    /// Records why the listing failed, leaving the view inside the dataset:
    /// the header still names the dataset the operator asked for, and the
    /// pane says what went wrong instead of showing an empty list that reads
    /// as "this dataset has no tables".
    pub fn set_bigquery_tables_problem(&mut self, problem: DiagnosticProblem) {
        self.bigquery.tables.rows.clear();
        self.bigquery.tables.cursor.selected_visible = 0;
        self.bigquery.tables.problem = Some(problem);
        self.bigquery.tables.loading = false;
    }

    // ---- the schema drill-down ----

    /// The table the schema view is inside, and `None` anywhere above it —
    /// `BigQueryDrilldownState::restore_invariant` keeps those two the same
    /// question.
    pub fn bigquery_schema_table(&self) -> Option<&BigQueryTable> {
        self.bigquery.schema.parent.as_ref()
    }

    pub fn bigquery_schema_fields(&self) -> &[BigQuerySchemaField] {
        &self.bigquery.schema.rows
    }

    /// The loaded columns the filter admits, in the order the schema walk
    /// flattened them.
    ///
    /// Unsorted, for a stronger reason than the tables list has: the order is
    /// the schema's own. A `RECORD` is immediately followed by the fields
    /// nested inside it, so the sequence is what says which columns belong to
    /// which parent, and any reordering destroys a fact the rows carry no
    /// other way.
    pub fn visible_bigquery_schema_fields(&self) -> Vec<&BigQuerySchemaField> {
        let filter = self.bigquery.schema.cursor.filter_applied.filter();
        self.bigquery
            .schema
            .rows
            .iter()
            .filter(|field| schema_field_matches_filter(field, filter))
            .collect()
    }

    pub fn visible_bigquery_schema_field_count(&self) -> usize {
        self.visible_bigquery_schema_fields().len()
    }

    pub fn selected_bigquery_schema_field(&self) -> Option<&BigQuerySchemaField> {
        self.visible_bigquery_schema_fields()
            .get(self.bigquery.schema.cursor.selected_visible)
            .copied()
    }

    pub fn bigquery_schema_loading(&self) -> bool {
        self.bigquery.schema.loading
    }

    pub fn bigquery_schema_problem(&self) -> Option<&DiagnosticProblem> {
        self.bigquery.schema.problem.as_ref()
    }

    /// Enters the selected table, returning the request the caller must issue:
    /// the table the view now claims to be in, and the drill the answer
    /// belongs to.
    ///
    /// `None` — and no transition — when there is nothing selected (an empty
    /// or wholly filtered-out tables list), or when the tables list is not the
    /// view on screen. That second guard is what stops `Enter` on a *dataset*
    /// row descending two levels at once, and what makes `Enter` on a *column*
    /// row open nothing: a column is the bottom of this hierarchy.
    ///
    /// The dataset id comes off the selected row rather than off the tables
    /// level's `parent`, because the row is what the request is about. The two
    /// agree — `bq ls <dataset>` lists one dataset — and reading the row is
    /// what keeps them agreeing if that ever stops being true.
    pub fn open_selected_bigquery_table(&mut self) -> Option<BigQuerySchemaRequest> {
        if self.bigquery.view != BigQueryView::Tables {
            return None;
        }
        let table = self.selected_bigquery_table()?.clone();
        let dataset_id = table.dataset_id.clone();
        let table_id = table.table_id.clone();
        self.bigquery.set_view(BigQueryView::Schema);
        self.bigquery.schema.parent = Some(table);
        self.bigquery.schema.loading = true;
        Some(BigQuerySchemaRequest {
            dataset_id,
            table_id,
            generation: self.bigquery.generation,
        })
    }

    /// Leaves the schema view for the tables list it was entered from.
    ///
    /// A no-op anywhere else, which is what keeps it safe to call from a key
    /// dispatched for the whole resources list.
    /// [`Self::leave_bigquery_drilldown`] is the caller that reaches it from
    /// `Esc` and `-`, by asking which level is on screen and ascending exactly
    /// one. It returns nothing, for the reason
    /// `leave_bigquery_drilldown` returns nothing: the keys that reach it mean
    /// "go up", and a key pressed at the top has already done what it says.
    ///
    /// The tables listed behind the schema are still loaded afterwards, so no
    /// listing is re-issued: `restore_invariant` clears what is *below* the
    /// view arrived at and never what is at it.
    pub fn leave_bigquery_schema(&mut self) {
        if self.bigquery.view != BigQueryView::Schema {
            return;
        }
        self.bigquery.set_view(BigQueryView::Tables);
    }

    /// Re-issues the `bq show` for the table the view is inside, returning the
    /// request so the caller can build the effect for exactly that table.
    ///
    /// This is what `r` means two levels down. The alternative — `r` doing
    /// nothing here — was rejected because the schema view has a failure state
    /// of its own, and a view that says "Press r to retry." while `r` retries
    /// the level above it is worse than one that says nothing at all.
    ///
    /// Unlike [`Self::reload_bigquery_tables`] this *does* mint a new
    /// generation, by re-entering the level it is already at: a retry here is
    /// the same call a re-entry would issue, and
    /// [`Self::matches_bigquery_schema_scope`] is a generation equality with
    /// no listing epoch beside it. Without the bump the call this replaces
    /// would still be in scope and would install its fields over the retry's.
    /// Re-entering `Schema` clears nothing — see
    /// `BigQueryDrilldownState::restore_invariant` — so the columns already
    /// painted stay painted until the fresh ones land, the way the tables
    /// list's rows do.
    ///
    /// `None` — and no state change — anywhere but the schema view.
    pub fn reload_bigquery_schema(&mut self) -> Option<BigQuerySchemaRequest> {
        if self.bigquery.view != BigQueryView::Schema {
            return None;
        }
        let table = self.bigquery.schema.parent.as_ref()?;
        let dataset_id = table.dataset_id.clone();
        let table_id = table.table_id.clone();
        // The transition comes first and the mutations after, the order
        // `open_selected_bigquery_table` uses. Re-entering `Schema` clears
        // nothing today, so both orders behave alike — but `restore_invariant`
        // is the place a future edit would make arrival at a level clear that
        // level, and under that edit mutating first would have the transition
        // reset `loading` and `table` while the request went out.
        self.bigquery.set_view(BigQueryView::Schema);
        // The problem is cleared because it is about the call being retried;
        // the rows are left alone for the reason `reload_bigquery_tables`
        // leaves its own.
        self.bigquery.schema.problem = None;
        self.bigquery.schema.loading = true;
        Some(BigQuerySchemaRequest {
            dataset_id,
            table_id,
            generation: self.bigquery.generation,
        })
    }

    /// Whether a schema load minted under `generation` is still the one this
    /// view is waiting on.
    ///
    /// Modelled on [`Self::matches_bigquery_enrichment_scope`] and
    /// deliberately one conjunct shorter. The generation alone answers it:
    /// `BigQueryDrilldownState::set_view` bumps it on every transition, so
    /// entering a table mints one, leaving the table moves past it, and
    /// entering the same table again mints a third — a load for a table the
    /// operator has left, or for the previous entry into the table they are
    /// back inside, fails this check.
    ///
    /// A listing epoch would add nothing here. The epoch exists to separate
    /// two `bq ls` calls made *without* a view transition between them, which
    /// is what `r` does at the tables level; there is no equivalent at this
    /// level, because [`Self::reload_bigquery_schema`] answers `r` here by
    /// re-entering the level, and a re-entry is a transition that mints a
    /// generation. Carrying an epoch anyway would mean a second conjunct that
    /// no path can make false on its own.
    pub fn matches_bigquery_schema_scope(&self, generation: BigQueryDrillGeneration) -> bool {
        self.bigquery.generation == generation
    }

    /// Installs a freshly loaded schema, resetting the cursor: the rows are a
    /// different table's, and a cursor held over them would name a column at
    /// the same index rather than the column it was on.
    pub fn set_bigquery_schema(&mut self, fields: Vec<BigQuerySchemaField>) {
        self.bigquery.schema.rows = fields;
        self.bigquery.schema.cursor.selected_visible = 0;
        self.bigquery.schema.problem = None;
        self.bigquery.schema.loading = false;
    }

    /// Records why the schema load failed, leaving the view inside the table:
    /// the header still names the table the operator asked for, and the pane
    /// says what went wrong instead of showing an empty list that reads as
    /// "this table has no columns".
    ///
    /// The same policy [`Self::set_bigquery_tables_problem`] applies one level
    /// up, and for the same reason: an empty list and a failed load are
    /// different facts, and a table with genuinely no columns does not exist.
    pub fn set_bigquery_schema_problem(&mut self, problem: DiagnosticProblem) {
        self.bigquery.schema.rows.clear();
        self.bigquery.schema.cursor.selected_visible = 0;
        self.bigquery.schema.problem = Some(problem);
        self.bigquery.schema.loading = false;
    }

    // ---- generic list surface, routed by view ----
    //
    // The datasets list, the tables list and the schema list are three lists
    // behind one service kind, so the generic actions `mod.rs` dispatches per
    // service have to reach whichever one is on screen. These are the routing
    // points; every other generic operation (sort, columns, marks, logs, URIs)
    // stays on the datasets collection. Both drilled-in tables paint columns,
    // but none of those columns sorts and there is no column cursor over them,
    // so neither view has anything for those operations to be about — which is
    // why their tools lines offer neither `s` nor `←`/`→`.

    /// How many rows the list on screen shows under its own filter.
    ///
    /// The one routing point in this section that still has to name all three
    /// views, and the reason it does is not tidiness: the count depends on the
    /// row type and on which `*_matches_filter` predicate applies to it, and
    /// those are precisely the two things a [`DrillCursor`] does not know.
    /// Three of the functions below need it — `bigquery_select_next`,
    /// `bigquery_selected_visible_index` and `bigquery_apply_filter` — and
    /// each takes it from here rather than matching for itself.
    ///
    /// This is the first of the two documented exceptions to the cursor seam;
    /// [`DrillCursor`] carries the other half of the pair, and the two are
    /// written to be read together.
    fn bigquery_visible_len(&self) -> usize {
        match self.bigquery.view {
            BigQueryView::Datasets => self.visible_bigquery_dataset_count(),
            BigQueryView::Tables => self.visible_bigquery_table_count(),
            BigQueryView::Schema => self.visible_bigquery_schema_field_count(),
        }
    }

    /// Where the cursor of the list on screen sits among its visible rows.
    ///
    /// Deliberately unclamped, which is what the per-view arms this replaced
    /// did: the index is whatever was last written, and read on its own it can
    /// name a row the filter no longer admits. Nothing here can fix that,
    /// because clamping needs a count and a count needs the row type — see
    /// [`Self::bigquery_visible_len`]. It is kept honest on the write side
    /// instead: `bigquery_select_next` and `bigquery_apply_filter` clamp
    /// against a count taken after the change, and `bigquery_select_previous`
    /// can only move the index down. Each of the two readers below gates on a
    /// count before returning it — `bigquery_selected_visible_index` on the
    /// live one, and `..._for_visible_rows` on the size of the projected row
    /// list its caller passes in.
    fn bigquery_selected_visible(&self) -> usize {
        match self.bigquery.drilled_cursor() {
            Some(cursor) => cursor.selected_visible,
            None => self.bigquery_datasets.selected_visible,
        }
    }

    /// The cursor index of the list on screen, to move.
    ///
    /// Paired with [`Self::bigquery_selected_visible`] rather than folded into
    /// it because the datasets index lives on the shared collection and the
    /// two drilled ones on a [`DrillCursor`]; this is the seam where that
    /// difference stops mattering to the callers.
    fn bigquery_selected_visible_mut(&mut self) -> &mut usize {
        match self.bigquery.drilled_cursor_mut() {
            Some(cursor) => &mut cursor.selected_visible,
            None => &mut self.bigquery_datasets.selected_visible,
        }
    }

    pub(super) fn bigquery_select_next(&mut self) {
        let len = self.bigquery_visible_len();
        if len > 0 {
            let selected = self.bigquery_selected_visible_mut();
            *selected = (*selected + 1).min(len - 1);
        }
    }

    pub(super) fn bigquery_select_previous(&mut self) {
        let selected = self.bigquery_selected_visible_mut();
        *selected = selected.saturating_sub(1);
    }

    pub(super) fn bigquery_selected_visible_index(&self) -> Option<usize> {
        (self.bigquery_visible_len() > 0).then(|| self.bigquery_selected_visible())
    }

    pub(super) fn bigquery_selected_visible_index_for_visible_rows(
        &self,
        visible_rows: usize,
    ) -> Option<usize> {
        (visible_rows > 0).then(|| self.bigquery_selected_visible())
    }

    /// The label the tools line puts after `Sort:`.
    ///
    /// The one accessor in this section whose three arms are three different
    /// answers, which is why it keeps its own match where the rest gave theirs
    /// up: the tables list is not sorted at all — `visible_bigquery_tables`
    /// keeps `bq ls` order — so it says so rather than borrowing the datasets
    /// list's sort field, which is what the shared accessor would otherwise
    /// report while the drill is open, and the schema list says something
    /// different again. Cloud Storage's object browser answers the same
    /// question the same way (`StorageState::sort_label`).
    ///
    /// This is the second of the two documented exceptions to the cursor seam
    /// — the one routing point here that reads no [`DrillCursor`] at all. That
    /// type carries the other half of the pair, and the two are written to be
    /// read together.
    pub(super) fn bigquery_sort_label(&self) -> &'static str {
        match self.bigquery.view {
            BigQueryView::Datasets => self.bigquery_datasets.sort_field.label(),
            BigQueryView::Tables => "Listing order",
            // The schema's own order, and the one thing that says which fields
            // are nested inside which record. See
            // `visible_bigquery_schema_fields`.
            BigQueryView::Schema => "Schema order",
        }
    }

    pub(super) fn bigquery_filter_input_active(&self) -> bool {
        match self.bigquery.drilled_cursor() {
            Some(cursor) => cursor.filter_input_active,
            None => self.bigquery_datasets.filter_input_active,
        }
    }

    pub(super) fn bigquery_filter_draft(&self) -> &str {
        match self.bigquery.drilled_cursor() {
            Some(cursor) => &cursor.filter_draft,
            None => &self.bigquery_datasets.filter_draft,
        }
    }

    pub(super) fn bigquery_filter_applied(&self) -> &str {
        match self.bigquery.drilled_cursor() {
            Some(cursor) => cursor.filter_applied.source(),
            None => self.bigquery_datasets.filter_applied.source(),
        }
    }

    pub(super) fn bigquery_applied_filter_error(&self) -> Option<String> {
        match self.bigquery.drilled_cursor() {
            Some(cursor) => cursor.filter_applied.error_text(),
            None => self.bigquery_datasets.filter_applied.error_text(),
        }
    }

    pub(super) fn bigquery_start_filter(&mut self) {
        match self.bigquery.drilled_cursor_mut() {
            Some(cursor) => cursor.start_filter(),
            None => {
                self.bigquery_datasets.filter_input_active = true;
                self.bigquery_datasets.filter_draft =
                    self.bigquery_datasets.filter_applied.source().to_string();
            }
        }
    }

    pub(super) fn bigquery_append_filter_char(&mut self, value: char) {
        match self.bigquery.drilled_cursor_mut() {
            Some(cursor) => cursor.filter_draft.push(value),
            None => self.bigquery_datasets.filter_draft.push(value),
        }
    }

    pub(super) fn bigquery_backspace_filter(&mut self) {
        match self.bigquery.drilled_cursor_mut() {
            Some(cursor) => {
                cursor.filter_draft.pop();
            }
            None => {
                self.bigquery_datasets.filter_draft.pop();
            }
        }
    }

    pub(super) fn bigquery_apply_filter(&mut self) {
        match self.bigquery.drilled_cursor_mut() {
            Some(cursor) => cursor.apply_filter(),
            None => {
                self.bigquery_datasets.filter_applied = CompiledResourceFilter::new(
                    self.bigquery_datasets.filter_draft.trim().to_string(),
                );
                self.bigquery_datasets.filter_draft =
                    self.bigquery_datasets.filter_applied.source().to_string();
                self.bigquery_datasets.filter_input_active = false;
            }
        }
        // Clamping is the one part of applying a filter that cannot live on
        // the cursor: it needs the count of rows the *new* filter admits, and
        // that is a question about the row type. So it happens out here, once,
        // for whichever list is on screen — and after the assignment above,
        // because a count taken before it would clamp against the old filter.
        let len = self.bigquery_visible_len();
        let selected = self.bigquery_selected_visible_mut();
        *selected = (*selected).min(len.saturating_sub(1));
    }

    pub(super) fn bigquery_cancel_filter(&mut self) {
        match self.bigquery.drilled_cursor_mut() {
            Some(cursor) => cursor.cancel_filter(),
            None => {
                self.bigquery_datasets.filter_draft =
                    self.bigquery_datasets.filter_applied.source().to_string();
                self.bigquery_datasets.filter_input_active = false;
            }
        }
    }
}

/// The values a datasets filter searches.
///
/// Every cell the table paints, because `/` narrows what is on screen and a
/// term an operator can read in a column but cannot filter by is the filter
/// lying about the rows it hid.
///
/// The friendly name and the labels are searchable without a column of their
/// own. That is the safe direction of the never-drop-a-row invariant — it can
/// only widen what matches, never hide a row that is on screen — and the
/// friendly name is in particular what an operator who knows a dataset by its
/// display name will type.
///
/// The never-drop-a-row premise holds trivially here because this service
/// pushes no clause down at all: `bq` takes no server-side filter expression,
/// so every value here is matched client-side against a row that was never
/// narrowed server-side.
fn dataset_searchable_fields(dataset: &BigQueryDataset) -> Vec<String> {
    let mut fields = vec![
        dataset.dataset_id.clone(),
        dataset.location.clone(),
        dataset.project_id.clone(),
        dataset.friendly_name.clone().unwrap_or_default(),
    ];
    fields.extend(
        dataset
            .labels
            .iter()
            .map(|label| format!("{}={}", label.key, label.value)),
    );
    fields
}

fn dataset_matches_filter(dataset: &BigQueryDataset, filter: &CompiledFilter) -> bool {
    filter.matches(&dataset_searchable_fields(dataset))
}

fn compare_datasets(
    left: &BigQueryDataset,
    right: &BigQueryDataset,
    sort_field: BigQuerySortField,
    direction: SortDirection,
) -> std::cmp::Ordering {
    let primary = match sort_field {
        BigQuerySortField::DatasetId => left.dataset_id.cmp(&right.dataset_id),
        // Multi-regions (`US`) and regions (`europe-west1`) sort together as
        // the strings the column paints. A dataset whose location came back
        // empty sorts as the empty string, grouping those rows rather than
        // scattering them through the ordering.
        BigQuerySortField::Location => left.location.cmp(&right.location),
    };
    // The dataset id breaks every tie and is unique within the project, so
    // this is a total order over the row set.
    let ordering = primary.then_with(|| left.dataset_id.cmp(&right.dataset_id));
    match direction {
        SortDirection::Ascending => ordering,
        SortDirection::Descending => ordering.reverse(),
    }
}

/// The values a tables filter searches.
///
/// The table id, its dataset, its kind and its labels — every *word* the row
/// carries, and in particular every word the table paints. The kind is matched
/// on the word the `TYPE` cell shows (`BigQueryTableKind::label`, so
/// `Materialized view` and not `MATERIALIZED_VIEW`): a term an operator can
/// read in a column but cannot filter by is the filter lying about the rows it
/// hid.
///
/// The three enrichment columns are the deliberate exception, and they are
/// numbers rather than words. `ROWS`, `BYTES` and `LAST MODIFIED` are painted
/// from `TableStat`s the projection formats — a humanised `14.9 KiB`, a
/// rendered instant, and the dash an unavailable stat paints — so searching
/// them would mean either matching text no field here holds, or matching the
/// dash, which every unenriched row carries and which would make `/-` select
/// exactly the rows nothing is known about. A substring of a formatted number
/// is not a term an operator means.
///
/// The labels are searchable without a column of their own, the safe direction
/// of the never-drop-a-row rule — it can only widen what matches — and the
/// reason they have none is recorded beside `BIGQUERY_TABLE_COLUMNS`.
///
/// The never-drop-a-row premise holds trivially, as it does for datasets: `bq
/// ls` pushes no clause down, so every row here was fetched unnarrowed.
fn table_searchable_fields(table: &BigQueryTable) -> Vec<String> {
    let mut fields = vec![
        table.table_id.clone(),
        table.dataset_id.clone(),
        table.kind.label().to_string(),
    ];
    fields.extend(
        table
            .labels
            .iter()
            .map(|label| format!("{}={}", label.key, label.value)),
    );
    fields
}

fn table_matches_filter(table: &BigQueryTable, filter: &CompiledFilter) -> bool {
    filter.matches(&table_searchable_fields(table))
}

/// The values a schema filter searches.
///
/// Every *word* the table paints, and the table paints four columns: `FIELD`
/// is the dotted path, `TYPE` the type, `MODE` the mode, and `DESCRIPTION` the
/// description where there is one. A term an operator can read in a column but
/// cannot filter by is the filter lying about the rows it hid.
///
/// The one thing on screen that is not searchable is the dash `DESCRIPTION`
/// paints for a field carrying none, and that is the same exclusion the tables
/// level makes for `ROWS` and `BYTES`: a dash is not a word the row holds, and
/// matching it would make `/-` select exactly the fields nobody documented.
///
/// The path is matched whole rather than as its leaf name, which is what makes
/// `/event_params` select a record together with everything nested inside it —
/// the children carry the parent's path as their prefix. That is the search an
/// operator staring at a nested schema means.
///
/// A field with no description contributes nothing rather than an empty
/// string, so a filter cannot match on the absence. The never-drop-a-row
/// premise holds trivially, as it does for the two levels above: `bq show`
/// pushes no clause down, so every row here was fetched unnarrowed.
fn schema_field_searchable_fields(field: &BigQuerySchemaField) -> Vec<String> {
    let mut fields = vec![
        field.path.clone(),
        field.field_type.clone(),
        field.mode.clone(),
    ];
    fields.extend(field.description.clone());
    fields
}

fn schema_field_matches_filter(field: &BigQuerySchemaField, filter: &CompiledFilter) -> bool {
    filter.matches(&schema_field_searchable_fields(field))
}

#[cfg(test)]
mod tests {
    use super::*;
    use nuvem_domain::gcp::resource::{BigQueryTableKind, TableStat};

    fn dataset(dataset_id: &str) -> BigQueryDataset {
        BigQueryDataset {
            dataset_id: dataset_id.to_string(),
            project_id: "dev-project".to_string(),
            location: "US".to_string(),
            friendly_name: None,
            labels: Vec::new(),
        }
    }

    fn table(dataset_id: &str, table_id: &str) -> BigQueryTable {
        BigQueryTable {
            dataset_id: dataset_id.to_string(),
            table_id: table_id.to_string(),
            kind: BigQueryTableKind::Table,
            created: None,
            labels: Vec::new(),
            rows: TableStat::Unavailable,
            bytes: TableStat::Unavailable,
            last_modified: TableStat::Unavailable,
        }
    }

    /// Pins what [`BigQueryDrilldownState::restore_invariant`] claims it does
    /// *not* have to do: keep a `view_mode` in sync.
    ///
    /// The claim is that all three BigQuery views are lists, so the datasets
    /// collection's `view_mode` is `List` under every one of them — nothing
    /// assigns it, unlike the storage collection's, which `StorageState`'s own
    /// `restore_invariant` moves to `Detail` for the bucket-detail view. This
    /// test is what stops that claim from going quietly false the day a
    /// BigQuery detail pane lands: the pane would set `view_mode`, and the
    /// invariant would then have a second thing to keep consistent.
    ///
    /// It walks the full descent and the full ascent rather than each view in
    /// isolation, because `view_mode` is a field a *transition* would assign:
    /// checking `Schema` only after entering it directly would miss an
    /// assignment made on the way back up.
    #[test]
    fn all_three_views_leave_the_datasets_collection_in_list_mode() {
        let mut state = ResourcesState::default();
        state.select_service(ResourceServiceKind::BigQueryDatasets);
        state.set_bigquery_datasets(
            GcloudContext {
                configuration: nuvem_domain::gcp::configuration::GcloudConfigurationName::new(
                    "dev",
                ),
                account: None,
                project: Some("dev-project".to_string()),
                region: None,
                zone: None,
            },
            vec![dataset("sales")],
        );
        assert_eq!(state.bigquery_datasets.view_mode, ResourcesViewMode::List);

        state.open_selected_bigquery_dataset();

        assert_eq!(state.bigquery_view(), BigQueryView::Tables);
        assert_eq!(
            state.bigquery_datasets.view_mode,
            ResourcesViewMode::List,
            "the tables view is a list, not a pane over the datasets list"
        );

        state.set_bigquery_tables(vec![table("sales", "orders")]);
        state.open_selected_bigquery_table();

        assert_eq!(state.bigquery_view(), BigQueryView::Schema);
        assert_eq!(
            state.bigquery_datasets.view_mode,
            ResourcesViewMode::List,
            "and the schema view is a list too, not a pane over either list \
             above it"
        );

        state.leave_bigquery_schema();

        assert_eq!(state.bigquery_view(), BigQueryView::Tables);
        assert_eq!(state.bigquery_datasets.view_mode, ResourcesViewMode::List);

        state.leave_bigquery_drilldown();

        assert_eq!(state.bigquery_view(), BigQueryView::Datasets);
        assert_eq!(state.bigquery_datasets.view_mode, ResourcesViewMode::List);
    }
}
