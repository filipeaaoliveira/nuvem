use super::*;

/// Marker type unifying the persistent disks service behind [`ResourceService`].
/// Never instantiated.
pub enum PersistentDisks {}

impl ResourceService for PersistentDisks {
    type Row = PersistentDisk;
    /// No detail pane in this slice, so there is nothing for a describe to
    /// fill and no tab to land on. The unit types make that the type rather
    /// than a placeholder struct the renderer could be handed.
    ///
    /// This is also why no `[[jumps]]` rule can name a disk as its *target*:
    /// `JumpRule::to` is a `DetailTab`, and a service with no detail pane has
    /// no tab to be one. Jumps *from* a disk work, via `config::jumps::fields_for`.
    type Detail = ();
    type SortField = DiskSortField;
    type DetailTab = ();

    const KIND: ResourceServiceKind = ResourceServiceKind::PersistentDisks;

    fn collection(
        state: &ResourcesState,
    ) -> &ResourceCollectionState<Self::Row, Self::Detail, Self::SortField, Self::DetailTab> {
        &state.disks
    }

    fn collection_mut(
        state: &mut ResourcesState,
    ) -> &mut ResourceCollectionState<Self::Row, Self::Detail, Self::SortField, Self::DetailTab>
    {
        &mut state.disks
    }

    fn matches_filter(row: &Self::Row, filter: &CompiledFilter) -> bool {
        disk_matches_filter(row, filter)
    }

    /// The needle sits in the name, the identity column of the disk table.
    fn sample_row() -> Self::Row {
        PersistentDisk {
            name: SAMPLE_ROW_NEEDLE.to_string(),
            location: "europe-west1-b".to_string(),
            status: DiskStatus::Ready,
            size_gb: 10,
            disk_type: "pd-balanced".to_string(),
            users: Vec::new(),
            labels: Vec::new(),
        }
    }

    fn compare(
        left: &Self::Row,
        right: &Self::Row,
        sort_field: Self::SortField,
        direction: SortDirection,
    ) -> std::cmp::Ordering {
        compare_disks(left, right, sort_field, direction)
    }

    /// Persistent disks are read-only here — no lifecycle actions, so no marks.
    /// This exists to keep [`ResourceService`] total.
    ///
    /// A disk's name is unique within its zone (or, for a regional disk, its
    /// region), not within the project, so the location is part of the identity
    /// rather than decoration.
    fn row_id(row: &Self::Row) -> ResourceRowId {
        ResourceRowId {
            name: row.name.clone(),
            location: row.location.clone(),
        }
    }
}

/// The text the orphan column paints, and the term an operator types into `/`
/// to isolate the disks worth deleting.
///
/// One constant because the cell and the searchable field must be the same
/// string: a badge an operator can read but cannot filter by is the filter
/// lying about the rows it hid.
pub const ORPHAN_BADGE: &str = "ORPHAN";

/// The sortable columns of the disks table.
///
/// One variant per *sortable* catalogue column. The catalogue has seven columns
/// and six of them sort: `orphan` does not, for the reason recorded beside
/// `PERSISTENT_DISK_COLUMNS`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiskSortField {
    Name,
    Location,
    Status,
    Size,
    Type,
    Attached,
}

impl DiskSortField {
    pub fn label(self) -> &'static str {
        match self {
            Self::Name => "Name",
            Self::Location => "Location",
            Self::Status => "Status",
            Self::Size => "Size",
            Self::Type => "Type",
            Self::Attached => "Attached",
        }
    }

    pub const COLUMN_COUNT: usize = 7;

    pub fn from_column(index: usize) -> Option<Self> {
        match index {
            0 => Some(Self::Name),
            1 => Some(Self::Location),
            2 => Some(Self::Status),
            // 3 is `orphan`, the one catalogue column with no sort field.
            4 => Some(Self::Size),
            5 => Some(Self::Type),
            6 => Some(Self::Attached),
            _ => None,
        }
    }

    pub fn column(self) -> usize {
        match self {
            Self::Name => 0,
            Self::Location => 1,
            Self::Status => 2,
            Self::Size => 4,
            Self::Type => 5,
            Self::Attached => 6,
        }
    }
}

impl ResourcesState {
    pub fn persistent_disks(&self) -> &[PersistentDisk] {
        &self.disks.rows
    }

    /// Delegates to the generic helper rather than re-deriving filter-then-sort
    /// here.
    ///
    /// Re-deriving it is what the sibling read-only services do, and it leaves
    /// the `ResourceService` impl's `matches_filter` and `compare` with no
    /// caller — the mutation gate caught exactly that, surviving a mutant that
    /// replaced this service's `matches_filter` with both `true` and `false`
    /// while every filter test still passed, because the tests were reaching
    /// the free function and the trait impl was dead. Going through the trait
    /// makes the impl live, so those same tests now pin it.
    pub fn visible_persistent_disks(&self) -> Vec<&PersistentDisk> {
        self.visible_rows::<PersistentDisks>()
    }

    pub fn visible_persistent_disk_count(&self) -> usize {
        let filter = self.disks.filter_applied.filter();
        self.disks
            .rows
            .iter()
            .filter(|disk| disk_matches_filter(disk, filter))
            .count()
    }

    pub fn selected_persistent_disk(&self) -> Option<&PersistentDisk> {
        self.visible_persistent_disks()
            .get(self.disks.selected_visible)
            .copied()
    }

    /// Installs a freshly loaded list, keeping the cursor on the disk it was
    /// already on where that disk is still listed.
    pub fn set_persistent_disks(&mut self, context: GcloudContext, disks: Vec<PersistentDisk>) {
        let previously_selected = self
            .selected_persistent_disk()
            .map(|disk| (disk.name.clone(), disk.location.clone()));

        self.disks.context = Some(context);
        self.disks.rows = disks;
        self.disks.stale = false;
        self.disks.last_problem = None;
        self.disks.last_diagnostic = None;
        self.disks.clear_load_phase();

        self.disks.selected_visible = previously_selected
            .as_ref()
            .and_then(|(name, location)| {
                self.visible_persistent_disks()
                    .iter()
                    .position(|disk| &disk.name == name && &disk.location == location)
            })
            .unwrap_or_else(|| {
                self.disks
                    .selected_visible
                    .min(self.visible_persistent_disks().len().saturating_sub(1))
            });
    }

    pub fn mark_persistent_disks_stale_and_clear(&mut self) {
        self.disks.filter_draft = self.disks.filter_applied.source().to_string();
        self.disks.filter_input_active = false;
        self.disks.rows.clear();
        self.disks.context = None;
        self.disks.selected_visible = 0;
        self.disks.stale = true;
    }

    pub fn persistent_disks_problem(&self) -> Option<&DiagnosticProblem> {
        self.disks.last_problem.as_ref()
    }
}

/// The rendered size cell, e.g. `20 GB`.
///
/// One function because the cell and the searchable field must agree: an
/// operator who reads `20 GB` and types `20 GB` must match the row they are
/// looking at.
pub fn disk_size_label(size_gb: u64) -> String {
    format!("{size_gb} GB")
}

/// The rendered attached-instances cell: every instance using this disk.
///
/// Comma-separated rather than a count, because the count an operator can
/// already read off the orphan badge is the only part a number would carry,
/// and the names are what they need to go look at next.
pub fn disk_attached_label(users: &[String]) -> String {
    users.join(", ")
}

/// The values a disks filter searches.
///
/// Every cell the table paints, because `/` narrows what is on screen and a
/// term an operator can read in a column but cannot filter by is the filter
/// lying about the rows it hid.
///
/// Status contributes its *display label* (`Ready`, not `READY`) — the same
/// string the cell carries — and the orphan badge contributes [`ORPHAN_BADGE`]
/// for the badged rows only, so `/ORPHAN` isolates exactly the rows wearing it.
/// The never-drop-a-row premise holds trivially here because this service
/// pushes no regex clause down at all (`FILTER_FIELDS` in
/// `crate::services::resources::disks` is empty), so every one of these values
/// is matched client-side against a row that was never narrowed server-side.
///
/// Labels are included as `key=value` because that is how an operator thinks of
/// them and how the label filter grammar spells them.
fn disk_searchable_fields(disk: &PersistentDisk) -> Vec<String> {
    let mut fields = vec![
        disk.name.clone(),
        disk.location.clone(),
        disk.status.label().to_string(),
        disk_size_label(disk.size_gb),
        disk.disk_type.clone(),
        disk_attached_label(&disk.users),
    ];
    if disk.is_orphan() {
        fields.push(ORPHAN_BADGE.to_string());
    }
    fields.extend(
        disk.labels
            .iter()
            .map(|label| format!("{}={}", label.key, label.value)),
    );
    fields
}

fn disk_matches_filter(disk: &PersistentDisk, filter: &CompiledFilter) -> bool {
    filter.matches(&disk_searchable_fields(disk))
}

fn compare_disks(
    left: &PersistentDisk,
    right: &PersistentDisk,
    sort_field: DiskSortField,
    direction: SortDirection,
) -> std::cmp::Ordering {
    let primary = match sort_field {
        DiskSortField::Name => left.name.cmp(&right.name),
        DiskSortField::Location => left.location.cmp(&right.location),
        // By the label, which is what the column paints and what the sort arrow
        // above it claims to have ordered.
        DiskSortField::Status => left.status.label().cmp(&right.status.label()),
        // Numerically, on the integer rather than on the rendered cell: as text
        // `9 GB` sorts after `20 GB`, which is the opposite of what an operator
        // scanning for the biggest waste is asking the column for.
        DiskSortField::Size => left.size_gb.cmp(&right.size_gb),
        DiskSortField::Type => left.disk_type.cmp(&right.disk_type),
        // By the rendered cell, so the ordering matches what is on screen. An
        // unattached disk is the empty string, so orphans group together rather
        // than scattering through the ordering.
        DiskSortField::Attached => {
            disk_attached_label(&left.users).cmp(&disk_attached_label(&right.users))
        }
    };
    // Location breaks a name tie rather than the name breaking itself: two
    // disks can share a name across zones, so name alone is not a total order
    // over this row set.
    let ordering = primary
        .then_with(|| left.name.cmp(&right.name))
        .then_with(|| left.location.cmp(&right.location));
    match direction {
        SortDirection::Ascending => ordering,
        SortDirection::Descending => ordering.reverse(),
    }
}
