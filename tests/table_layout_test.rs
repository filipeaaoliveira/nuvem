use nuvem::app::projection::table::{
    Cell, CellAlign, CellKind, RowCounts, SortDirection, SortSpec, StatusSemantic, TableColumn,
    TableModel, TableRow,
};

fn col(key: &'static str, header: &'static str, min: u16, pref: u16, prio: u8) -> TableColumn {
    TableColumn {
        key,
        header,
        min_width: min,
        preferred_width: pref,
        priority: prio,
        align: CellAlign::Left,
        sortable: true,
    }
}

#[test]
fn cell_constructors_carry_kind() {
    assert!(matches!(Cell::plain("api").kind, CellKind::Plain));
    assert!(matches!(Cell::missing().kind, CellKind::Missing));
    assert!(matches!(
        Cell::status("Running", StatusSemantic::Healthy).kind,
        CellKind::Status(StatusSemantic::Healthy)
    ));
    assert_eq!(Cell::missing().text, "-");
}

#[test]
fn sort_direction_toggles() {
    assert_eq!(
        SortDirection::Ascending.toggled(),
        SortDirection::Descending
    );
    assert_eq!(
        SortDirection::Descending.toggled(),
        SortDirection::Ascending
    );
}

#[test]
fn table_model_holds_columns_and_rows() {
    let model = TableModel {
        columns: vec![col("name", "NAME", 4, 20, 0)],
        rows: vec![TableRow {
            cells: vec![Cell::plain("api")],
            marked: false,
        }],
        selected_row: Some(0),
        selected_column: 0,
        sort: SortSpec {
            column: 0,
            direction: SortDirection::Ascending,
        },
        counts: RowCounts {
            filtered: 1,
            total: 1,
        },
        show_mark_column: true,
    };
    assert_eq!(model.rows[0].cells[0].text, "api");
}

use nuvem::ui::shared::table::{TablePlan, plan_table};

fn gce_columns() -> Vec<TableColumn> {
    vec![
        TableColumn {
            key: "name",
            header: "NAME",
            min_width: 6,
            preferred_width: 28,
            priority: 0,
            align: CellAlign::Left,
            sortable: true,
        },
        TableColumn {
            key: "status",
            header: "STATUS",
            min_width: 6,
            preferred_width: 10,
            priority: 1,
            align: CellAlign::Left,
            sortable: true,
        },
        TableColumn {
            key: "zone",
            header: "ZONE",
            min_width: 6,
            preferred_width: 18,
            priority: 2,
            align: CellAlign::Left,
            sortable: true,
        },
        TableColumn {
            key: "machine-type",
            header: "MACHINE-TYPE",
            min_width: 8,
            preferred_width: 18,
            priority: 3,
            align: CellAlign::Left,
            sortable: true,
        },
        TableColumn {
            key: "internal-ip",
            header: "INTERNAL-IP",
            min_width: 8,
            preferred_width: 15,
            priority: 4,
            align: CellAlign::Left,
            sortable: false,
        },
        TableColumn {
            key: "external-ip",
            header: "EXTERNAL-IP",
            min_width: 8,
            preferred_width: 15,
            priority: 5,
            align: CellAlign::Left,
            sortable: false,
        },
    ]
}

fn model_with(names: &[&str], columns: Vec<TableColumn>) -> TableModel {
    let rows = names
        .iter()
        .map(|name| TableRow {
            cells: vec![
                Cell::plain(*name),
                Cell::plain("Running"),
                Cell::plain("europe-west1-b"),
                Cell::plain("e2-medium"),
                Cell::plain("10.0.0.4"),
                Cell::plain("34.10.10.10"),
            ],
            marked: false,
        })
        .collect();
    TableModel {
        columns,
        rows,
        selected_row: Some(0),
        selected_column: 0,
        sort: SortSpec {
            column: 0,
            direction: SortDirection::Ascending,
        },
        counts: RowCounts {
            filtered: names.len(),
            total: names.len(),
        },
        show_mark_column: true,
    }
}

fn line_width(plan: &TablePlan) -> u16 {
    let visible: Vec<u16> = plan
        .columns
        .iter()
        .filter(|c| c.visible)
        .map(|c| c.width)
        .collect();
    let sum: u16 = visible.iter().sum();
    sum + 2 * (visible.len().saturating_sub(1)) as u16
}

#[test]
fn narrow_layout_hides_low_priority_columns_without_overflow() {
    let model = model_with(&["api-dev-1"], gce_columns());
    for width in [60u16, 80, 120, 160] {
        let plan = plan_table(&model, width, false);
        assert!(line_width(&plan) <= width, "width {width} overflowed");
        assert!(plan.columns[0].visible, "NAME must stay visible at {width}");
    }
}

#[test]
fn wider_layouts_reveal_more_columns() {
    let model = model_with(&["api-dev-1"], gce_columns());
    let narrow = plan_table(&model, 60, false);
    let wide = plan_table(&model, 160, false);
    let narrow_visible = narrow.columns.iter().filter(|c| c.visible).count();
    let wide_visible = wide.columns.iter().filter(|c| c.visible).count();
    assert!(wide_visible >= narrow_visible);
    assert_eq!(wide_visible, 6, "all columns fit at 160");
}

#[test]
fn top_priority_column_survives_extreme_narrowness() {
    let model = model_with(&["api-dev-1"], gce_columns());
    let plan = plan_table(&model, 20, false);
    assert!(plan.columns[0].visible);
    assert!(line_width(&plan) <= 20);
    assert!(plan.hidden_count >= 1);
}

#[test]
fn long_name_truncates_within_its_column_width() {
    let long = "a-very-long-instance-name-that-exceeds-preferred-width-000";
    let model = model_with(&[long], gce_columns());
    let plan = plan_table(&model, 80, false);
    let name = &plan.columns[0];
    assert!(name.visible);
    assert!(name.width <= 28, "name capped at preferred width");
}

#[test]
fn unicode_width_names_are_measured_by_display_width() {
    // Each CJK character is two display columns wide.
    let model = model_with(&["实例名称实例名称"], gce_columns());
    let plan = plan_table(&model, 120, false);
    assert!(line_width(&plan) <= 120);
    assert!(plan.columns[0].width >= 6);
}

#[test]
fn wide_mode_keeps_every_column_and_flags_overflow() {
    let model = model_with(&["api-dev-1"], gce_columns());
    let plan = plan_table(&model, 40, true);
    assert!(plan.columns.iter().all(|c| c.visible));
    assert_eq!(plan.hidden_count, 0);
    assert!(plan.overflow, "40 columns is too narrow for all six");
}

use nuvem::ui::shared::table::status_marker;
use nuvem_domain::gcp::resource::{CloudRunServiceStatus, GkeClusterStatus, ResourceStatus};

#[test]
fn resource_status_maps_to_semantic() {
    assert_eq!(
        StatusSemantic::from_resource_status(&ResourceStatus::Running),
        StatusSemantic::Healthy
    );
    assert_eq!(
        StatusSemantic::from_resource_status(&ResourceStatus::Pending),
        StatusSemantic::Busy
    );
    assert_eq!(
        StatusSemantic::from_resource_status(&ResourceStatus::Suspended),
        StatusSemantic::Warning
    );
    assert_eq!(
        StatusSemantic::from_resource_status(&ResourceStatus::Stopped),
        StatusSemantic::Neutral
    );
    assert_eq!(
        StatusSemantic::from_resource_status(&ResourceStatus::Unknown("x".into())),
        StatusSemantic::Neutral
    );
}

#[test]
fn gke_status_maps_to_semantic() {
    assert_eq!(
        StatusSemantic::from_gke_status(&GkeClusterStatus::Running),
        StatusSemantic::Healthy
    );
    assert_eq!(
        StatusSemantic::from_gke_status(&GkeClusterStatus::Reconciling),
        StatusSemantic::Busy
    );
    assert_eq!(
        StatusSemantic::from_gke_status(&GkeClusterStatus::Stopping),
        StatusSemantic::Warning
    );
    assert_eq!(
        StatusSemantic::from_gke_status(&GkeClusterStatus::Error),
        StatusSemantic::Degraded
    );
}

#[test]
fn cloud_run_status_maps_to_semantic() {
    assert_eq!(
        StatusSemantic::from_cloud_run_status(&CloudRunServiceStatus::Ready),
        StatusSemantic::Healthy
    );
    assert_eq!(
        StatusSemantic::from_cloud_run_status(&CloudRunServiceStatus::Reconciling),
        StatusSemantic::Busy
    );
    assert_eq!(
        StatusSemantic::from_cloud_run_status(&CloudRunServiceStatus::NotReady),
        StatusSemantic::Warning
    );
    assert_eq!(
        StatusSemantic::from_cloud_run_status(&CloudRunServiceStatus::Failed),
        StatusSemantic::Degraded
    );
    assert_eq!(
        StatusSemantic::from_cloud_run_status(&CloudRunServiceStatus::Unknown("x".into())),
        StatusSemantic::Neutral
    );
}

#[test]
fn status_markers_are_shared_vocabulary() {
    assert_eq!(status_marker(StatusSemantic::Healthy), "OK");
    assert_eq!(status_marker(StatusSemantic::Degraded), "ERR");
}

use nuvem::ui::shared::table::render_table;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::layout::Rect;

fn buffer_text(terminal: &Terminal<TestBackend>) -> String {
    let buffer = terminal.backend().buffer();
    let area = *buffer.area();
    let mut out = String::new();
    for y in 0..area.height {
        for x in 0..area.width {
            out.push_str(buffer[(x, y)].symbol());
        }
        out.push('\n');
    }
    out
}

#[test]
fn render_table_draws_headers_and_sort_indicator() {
    let mut model = model_with(&["api-dev-1", "worker-1"], gce_columns());
    model.sort = SortSpec {
        column: 0,
        direction: SortDirection::Ascending,
    };
    model.selected_column = 0;
    let plan = plan_table(&model, 120, false);

    let backend = TestBackend::new(120, 6);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|frame| render_table(frame, Rect::new(0, 0, 120, 6), &model, &plan))
        .unwrap();

    let text = buffer_text(&terminal);
    assert!(text.contains("NAME"), "header row missing NAME");
    assert!(text.contains("STATUS"));
    assert!(text.contains('▲'), "ascending sort indicator missing");
    assert!(text.contains("api-dev-1"));
}

#[test]
fn render_table_shows_mark_glyph_independent_of_cursor_highlight() {
    let mut model = model_with(&["api-dev-1", "worker-1"], gce_columns());
    model.rows[0].marked = true;
    model.selected_row = Some(1);
    let plan = plan_table(&model, 80, false);

    let backend = TestBackend::new(80, 6);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|frame| render_table(frame, Rect::new(0, 0, 80, 6), &model, &plan))
        .unwrap();

    let text = buffer_text(&terminal);
    let lines: Vec<&str> = text.lines().collect();
    // Row 0 (api-dev-1) is marked but not the cursor row: the diamond glyph
    // shows without the "> " cursor symbol.
    assert!(lines[1].contains('◆'), "marked row missing mark glyph");
    assert!(
        !lines[1].starts_with("> "),
        "unselected marked row must not show the cursor symbol"
    );
    // Row 1 (worker-1) is the cursor row but unmarked: "> " shows without
    // the mark glyph, proving the two indicators are independent.
    assert!(
        lines[2].starts_with("> "),
        "cursor row missing highlight symbol"
    );
    assert!(
        !lines[2].contains('◆'),
        "unmarked cursor row must not show the mark glyph"
    );
}

#[test]
fn render_table_shows_cursor_and_mark_together_on_same_row() {
    let mut model = model_with(&["api-dev-1", "worker-1"], gce_columns());
    model.rows[0].marked = true;
    model.selected_row = Some(0);
    let plan = plan_table(&model, 80, false);

    let backend = TestBackend::new(80, 6);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|frame| render_table(frame, Rect::new(0, 0, 80, 6), &model, &plan))
        .unwrap();

    let text = buffer_text(&terminal);
    let lines: Vec<&str> = text.lines().collect();
    // Row 0 (api-dev-1) is both the cursor row and marked: the cursor
    // highlight symbol and the mark glyph must both appear on the same row.
    assert!(
        lines[1].contains("> ◆"),
        "row that is both selected and marked must show cursor symbol and mark glyph together"
    );
}

#[test]
fn render_table_without_mark_column_draws_no_glyph_and_no_clip() {
    // Tables that opt out of the mark column (GKE, Cloud SQL, logging) must
    // render no leading glyph column and must not surrender width to one, so
    // the rightmost content column is never clipped — even when a row happens
    // to carry `marked: true`.
    let model = TableModel {
        columns: gce_columns(),
        rows: vec![TableRow {
            cells: vec![
                Cell::plain("api-dev-1"),
                Cell::plain("Running"),
                Cell::plain("europe-west1-b"),
                Cell::plain("e2-medium"),
                Cell::plain("10.0.0.4"),
                // Exactly fills the 15-wide external-ip column, so any lost
                // width would clip visible characters.
                Cell::plain("255.255.255.255"),
            ],
            marked: true,
        }],
        selected_row: Some(0),
        selected_column: 0,
        sort: SortSpec {
            column: 0,
            direction: SortDirection::Ascending,
        },
        counts: RowCounts {
            filtered: 1,
            total: 1,
        },
        show_mark_column: false,
    };
    // All six columns reach their preferred widths and exactly fill 114 cells;
    // the render area adds 2 for the cursor highlight gutter.
    let plan = plan_table(&model, 114, false);
    assert!(plan.columns.iter().all(|c| c.visible));

    let backend = TestBackend::new(116, 3);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|frame| render_table(frame, Rect::new(0, 0, 116, 3), &model, &plan))
        .unwrap();

    let text = buffer_text(&terminal);
    assert!(
        !text.contains('◆'),
        "table with show_mark_column=false must not draw the mark glyph"
    );
    assert!(
        text.contains("255.255.255.255"),
        "rightmost column must render in full when no mark column is reserved"
    );
}
