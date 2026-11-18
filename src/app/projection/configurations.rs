//! Render-facing table model for the Configurations workspace.
//!
//! The workspace used to build each row with `format!` and a two-space join,
//! which left nothing aligned and no header. It was the only list in
//! the app that was not a column table, and it is the one screen whose whole
//! job is reading one field — account, project — down the rows.
//!
//! Shaped after [`super::tunnels`]: a hand-built [`TableModel`] rather than an
//! entry in the column catalog, because the workspace offers neither the column
//! cursor (`←`/`→`) nor `s` to sort by, so it has nothing for a configured
//! layout to configure.

use nuvem_domain::gcp::configuration::GcloudConfiguration;

use super::table::{
    Cell, CellAlign, NO_COLUMN, RowCounts, SortDirection, SortSpec, TableColumn, TableModel,
    TableRow,
};
use crate::app::state::configurations::ConfigurationsState;

/// Marks the active configuration, in the NAME cell rather than in a column of
/// its own.
///
/// A column would be droppable: `plan_table` protects only the single
/// lowest-priority column from being hidden, and that has to stay NAME, since
/// rows the operator cannot tell apart are worse than rows whose active one is
/// unmarked. Carried in the cell, the marker cannot be separated from the name
/// it qualifies at any width.
const ACTIVE_MARKER: &str = "* ";
/// What every inactive row carries in its place, so the names still start at
/// one column.
const INACTIVE_MARKER: &str = "  ";

/// Projects the visible configurations into the shared table model.
pub fn project_configurations_table(state: &ConfigurationsState) -> TableModel {
    let visible = state.visible_rows();
    let rows: Vec<TableRow> = visible
        .iter()
        .map(|configuration| configuration_row(configuration))
        .collect();

    TableModel {
        columns: configuration_columns(),
        rows,
        selected_row: state.selected_visible_index(),
        // Neither `←`/`→` nor `s` is bound in this workspace, so `NO_COLUMN`
        // points both fields at an index no column has: `render_table`
        // decorates no header with a selection bracket or a sort arrow, rather
        // than advertising a control the workspace does not offer. Same
        // reasoning as the tunnels list.
        selected_column: NO_COLUMN,
        sort: SortSpec {
            column: NO_COLUMN,
            direction: SortDirection::Ascending,
        },
        counts: RowCounts {
            filtered: visible.len(),
            total: state.rows().len(),
        },
        show_mark_column: false,
    }
}

fn configuration_row(configuration: &GcloudConfiguration) -> TableRow {
    let marker = if configuration.is_active {
        ACTIVE_MARKER
    } else {
        INACTIVE_MARKER
    };
    TableRow {
        cells: vec![
            Cell::plain(format!("{marker}{}", configuration.name.as_str())),
            Cell::optional(configuration.account.clone()),
            Cell::optional(configuration.project.clone()),
            Cell::optional(configuration.region.clone()),
            Cell::optional(configuration.zone.clone()),
        ],
        marked: false,
    }
}

/// Preferred widths are the real shapes these values take: a gcloud account is
/// an email, a project id is bounded at 30 characters, and a zone is a region
/// plus a suffix. NAME's preferred width carries [`ACTIVE_MARKER`] on top of
/// the name itself.
fn configuration_columns() -> Vec<TableColumn> {
    vec![
        TableColumn {
            key: "name",
            header: "NAME",
            min_width: 6,
            preferred_width: 26,
            // 0, so `plan_table` protects it: it is the only cell that says
            // which configuration a row is.
            priority: 0,
            align: CellAlign::Left,
            sortable: false,
        },
        TableColumn {
            key: "account",
            header: "ACCOUNT",
            min_width: 8,
            preferred_width: 30,
            priority: 1,
            align: CellAlign::Left,
            sortable: false,
        },
        TableColumn {
            key: "project",
            header: "PROJECT",
            min_width: 8,
            preferred_width: 30,
            priority: 2,
            align: CellAlign::Left,
            sortable: false,
        },
        TableColumn {
            key: "region",
            header: "REGION",
            min_width: 6,
            preferred_width: 16,
            priority: 3,
            align: CellAlign::Left,
            sortable: false,
        },
        TableColumn {
            key: "zone",
            header: "ZONE",
            min_width: 4,
            preferred_width: 18,
            priority: 4,
            align: CellAlign::Left,
            sortable: false,
        },
    ]
}
