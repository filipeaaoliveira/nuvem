//! Render-agnostic table model shared by resource list projections.
//!
//! These types live in the app layer so projections can build a `TableModel`
//! without depending on the `ui` layer. The width-fitting algorithm and the
//! ratatui renderer that consume this model live in `src/ui/shared/table.rs`.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CellAlign {
    Left,
    Right,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableColumn {
    pub key: &'static str,
    pub header: &'static str,
    pub min_width: u16,
    pub preferred_width: u16,
    pub priority: u8,
    pub align: CellAlign,
    pub sortable: bool,
    /// How the column gives way when the table is too narrow for it.
    pub fit: ColumnFit,
}

/// How a column gives way when its table is too narrow for it.
///
/// Decided per column by what its values are, not by how long the demo's
/// happen to be: `db-custom-4-16384` and `SQL Server 2019 Standard` are a
/// tier and an engine, and a cut one (`db-custom-4-163…`) names a different
/// tier or none, while a URL or a list of regions is long by nature and its
/// head still says what it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColumnFit {
    /// A short identifier — a tier, an engine, a region, a status, a
    /// timestamp. Shown whole or dropped, never cut.
    Whole,
    /// A value that is long by nature — a URL, a list, free text. It may be
    /// cut, always with the `…` marker, and only once the table has no free
    /// width left to give it.
    Cut,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatusSemantic {
    Healthy,
    Busy,
    Warning,
    Degraded,
    Neutral,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CellKind {
    Plain,
    Missing,
    Status(StatusSemantic),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cell {
    pub text: String,
    pub kind: CellKind,
}

/// Makes one cell's text safe to put on a buffer line.
///
/// Every cell goes through here, which is the point: cell text is resource
/// data straight out of `gcloud`'s JSON, and until now nothing between the
/// parse and the terminal touched it. What kept that from being an escape
/// injection was ratatui dropping graphemes it considers unprintable when it
/// writes to its buffer — an undocumented detail of a dependency, not a
/// property of this code, and one that says nothing about the reordering
/// characters or about any sink that is not a ratatui buffer.
///
/// Doing it in the constructor rather than at the 131 call sites is what makes
/// it a property instead of a habit: a projection added later cannot forget,
/// because there is no way to build a cell that skips this.
///
/// The clean case keeps its `String` rather than building a new one. A list
/// can carry a few thousand cells per frame and essentially all of them are
/// ordinary GCP names, so the common path is a scan and no allocation.
fn cell_text(text: impl Into<String>) -> String {
    let text = text.into();
    if nuvem_domain::gcp::text::rendered_text_is_clean(&text) {
        return text;
    }
    nuvem_domain::gcp::text::sanitize_rendered_text(&text)
}

impl Cell {
    pub fn plain(text: impl Into<String>) -> Self {
        Self {
            text: cell_text(text),
            kind: CellKind::Plain,
        }
    }

    pub fn missing() -> Self {
        Self {
            text: "-".to_string(),
            kind: CellKind::Missing,
        }
    }

    pub fn status(text: impl Into<String>, semantic: StatusSemantic) -> Self {
        Self {
            text: cell_text(text),
            kind: CellKind::Status(semantic),
        }
    }

    /// Builds a plain cell from a value, or a `Missing` cell when absent.
    pub fn optional(value: Option<impl Into<String>>) -> Self {
        match value {
            Some(value) => Self::plain(value),
            None => Self::missing(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableRow {
    pub cells: Vec<Cell>,
    /// Whether this row's underlying resource is currently marked for a
    /// batch operation. Rendered as a dedicated leading glyph column,
    /// independent of the cursor's `> ` highlight. GKE rows never carry
    /// marks since batch selection only applies to compute instances.
    pub marked: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortDirection {
    Ascending,
    Descending,
}

impl SortDirection {
    pub fn toggled(self) -> Self {
        match self {
            Self::Ascending => Self::Descending,
            Self::Descending => Self::Ascending,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SortSpec {
    pub column: usize,
    pub direction: SortDirection,
}

/// A column index no column can have, used to mean "this table has no selected
/// column and no sorted column".
///
/// [`crate::ui::shared::table::render_table`] decorates the header at
/// `selected_column` with a `[...]` bracket and the one at `sort.column` with
/// an arrow; pointing both at an index no column has is how a table that
/// offers neither `←`/`→` nor `s` declines to advertise them. The tunnels list
/// and the BigQuery tables drill-down are the two tables in that position.
pub const NO_COLUMN: usize = usize::MAX;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RowCounts {
    pub filtered: usize,
    pub total: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableModel {
    pub columns: Vec<TableColumn>,
    pub rows: Vec<TableRow>,
    pub selected_row: Option<usize>,
    pub selected_column: usize,
    pub sort: SortSpec,
    pub counts: RowCounts,
    /// Whether to render the leading mark-glyph column. Only the compute
    /// instance table supports batch marking; every other table (GKE, Cloud
    /// SQL, logging) leaves this false so no blank mark column is drawn and no
    /// mark-column width is reserved.
    pub show_mark_column: bool,
}

use nuvem_domain::gcp::resource::{
    CloudFunctionState, CloudRunServiceStatus, CloudSqlStatus, DiskStatus, GkeClusterStatus,
    ResourceStatus,
};

impl StatusSemantic {
    pub fn from_cloud_sql_status(status: &CloudSqlStatus) -> Self {
        match status {
            CloudSqlStatus::Runnable => Self::Healthy,
            CloudSqlStatus::PendingCreate => Self::Busy,
            CloudSqlStatus::Maintenance => Self::Warning,
            CloudSqlStatus::Suspended => Self::Warning,
            CloudSqlStatus::Failed => Self::Degraded,
            CloudSqlStatus::Stopped => Self::Neutral,
            CloudSqlStatus::Unknown(_) => Self::Neutral,
        }
    }

    pub fn from_resource_status(status: &ResourceStatus) -> Self {
        match status {
            ResourceStatus::Running => Self::Healthy,
            ResourceStatus::Pending => Self::Busy,
            ResourceStatus::Suspended => Self::Warning,
            ResourceStatus::Stopped => Self::Neutral,
            ResourceStatus::Unknown(_) => Self::Neutral,
        }
    }

    pub fn from_gke_status(status: &GkeClusterStatus) -> Self {
        match status {
            GkeClusterStatus::Running => Self::Healthy,
            GkeClusterStatus::Reconciling => Self::Busy,
            GkeClusterStatus::Stopping => Self::Warning,
            GkeClusterStatus::Error => Self::Degraded,
            GkeClusterStatus::Unknown(_) => Self::Neutral,
        }
    }

    pub fn from_cloud_run_status(status: &CloudRunServiceStatus) -> Self {
        match status {
            CloudRunServiceStatus::Ready => Self::Healthy,
            CloudRunServiceStatus::Reconciling => Self::Busy,
            CloudRunServiceStatus::NotReady => Self::Warning,
            CloudRunServiceStatus::Failed => Self::Degraded,
            CloudRunServiceStatus::Unknown(_) => Self::Neutral,
        }
    }

    /// The colour behind a Cloud Function's STATE cell.
    ///
    /// `DEPLOYING` and `DELETING` are [`Self::Busy`]: work is in flight and the
    /// row will change on its own, which is not the same as something being
    /// wrong. `DETACHING` is [`Self::Warning`] rather than busy — a function
    /// being detached from its environment is not a failure, but it is not a
    /// steady state either. `STATE_UNSPECIFIED`, the API's own `UNKNOWN`, and a
    /// value nuvem does not recognise are all [`Self::Neutral`]: nuvem does not
    /// know whether the function is well, and painting it healthy or degraded
    /// would be an invention.
    pub fn from_cloud_function_state(state: &CloudFunctionState) -> Self {
        match state {
            CloudFunctionState::Active => Self::Healthy,
            CloudFunctionState::Deploying | CloudFunctionState::Deleting => Self::Busy,
            CloudFunctionState::Detaching => Self::Warning,
            CloudFunctionState::Failed | CloudFunctionState::DetachFailed => Self::Degraded,
            CloudFunctionState::Unspecified
            | CloudFunctionState::Unknown
            | CloudFunctionState::Unrecognized(_) => Self::Neutral,
        }
    }

    /// A disk's status, coloured by what an operator can do about it.
    ///
    /// `Ready` is healthy even when the disk is an orphan: the status column
    /// answers "is this disk usable", and the orphan column beside it answers
    /// "is anything using it". Colouring a healthy READY disk as a warning here
    /// would make the two columns disagree, and it is the badge — not the
    /// status — that carries the cost claim.
    pub fn from_disk_status(status: &DiskStatus) -> Self {
        match status {
            DiskStatus::Ready => Self::Healthy,
            DiskStatus::Creating | DiskStatus::Restoring | DiskStatus::Deleting => Self::Busy,
            DiskStatus::Failed => Self::Degraded,
            DiskStatus::Unavailable => Self::Warning,
            DiskStatus::Unrecognized(_) => Self::Neutral,
        }
    }
}
