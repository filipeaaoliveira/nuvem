//! Render-facing projection for the Cloud Logging workspace.
//!
//! Every string exposed here is passed through
//! `nuvem_domain::gcp::logging::sanitize_log_text` so untrusted log text can
//! never carry control characters or escape sequences into the renderer.

use chrono::Local;

use nuvem_domain::gcp::logging::{LogEntry, LogPayload, LogSeverity, sanitize_log_text};
use nuvem_domain::gcp::text_filter::TextFilterSyntax;

use crate::app::projection::format_last_refresh;
use crate::app::projection::table::{
    Cell, CellAlign, RowCounts, SortDirection, SortSpec, StatusSemantic, TableColumn, TableModel,
    TableRow,
};
use crate::app::state::logging::{LoggingState, LoggingStatus, TailState};
use crate::app::state::root::RootState;

/// Header metadata shown above the log table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoggingHeaderProjection {
    pub project: String,
    pub query: String,
    /// The exact generated `gcloud logging` filter string (or a placeholder
    /// for project-wide reads that carry no filter clause).
    pub filter: String,
    pub time_range: String,
    /// The `0`..=`6` digit that selects `time_range`, shown beside the label so
    /// the direct-addressing keys are discoverable from the header itself.
    pub time_range_bucket: char,
    /// The severity floor pushed into the gcloud filter, as a label. The
    /// `LogSeverity::Default` no-floor sentinel is displayed as `all`, since
    /// "DEFAULT" would read as a severity rather than as "not narrowed".
    pub severity_floor: String,
    pub tail: String,
    pub freshness: String,
    pub status: String,
    pub filter_active: bool,
    pub filter_draft: String,
    pub applied_filter: String,
    /// Which search mode the applied pattern selected (`substring`, `fuzzy`,
    /// `regex`, each optionally `not `-prefixed), so a typed sigil is confirmed
    /// rather than guessed at. `None` when no pattern is applied, or when the
    /// applied one does not parse.
    pub search_mode: Option<String>,
    /// The applied pattern's parse error, when it has one.
    pub search_error: Option<String>,
}

/// A single key/value pair from a log entry's label maps.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogLabelProjection {
    pub key: String,
    pub value: String,
}

/// Render-facing model for the expanded single-entry detail view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogDetailView {
    pub severity: String,
    pub timestamp: String,
    pub resource_type: String,
    pub log_name: String,
    pub message: String,
    pub payload_lines: Vec<String>,
    pub resource_labels: Vec<LogLabelProjection>,
    pub labels: Vec<LogLabelProjection>,
    pub scroll: u16,
    /// When true, the renderer wraps long message/payload lines instead of
    /// truncating them (toggled with `w`).
    pub wrap: bool,
}

/// Distinct body states so the renderer can show tailored empty/loading/
/// degraded text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoggingBodyState {
    Loading,
    /// Nothing has been fetched and nothing is in flight -- the body counterpart
    /// of the header's "Not loaded". Distinct from `Empty`, which means a load
    /// ran and matched nothing.
    NotLoaded,
    Empty,
    Rows,
    Degraded {
        message: String,
    },
}

/// The full render-facing projection for `Workspace::Logging`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoggingWorkspaceProjection {
    pub header: LoggingHeaderProjection,
    pub table: TableModel,
    pub detail: Option<LogDetailView>,
    pub body_state: LoggingBodyState,
}

/// Projects typed logging state into the shared render-facing workspace model.
pub fn project_logging(root: &RootState) -> LoggingWorkspaceProjection {
    let logging = &root.logging;
    let project = root
        .current_context()
        .and_then(|context| context.project)
        .map(|project| sanitize_log_text(&project))
        .unwrap_or_else(|| "-".to_string());

    let rendered_filter = logging.query.render_filter();
    let filter = if rendered_filter.is_empty() {
        "(all recent project logs)".to_string()
    } else {
        sanitize_log_text(&rendered_filter)
    };

    let header = LoggingHeaderProjection {
        project,
        query: sanitize_log_text(&logging.query.summary()),
        filter,
        time_range: logging.time_range.label().to_string(),
        time_range_bucket: logging.time_range.bucket_digit(),
        severity_floor: severity_floor_label(logging.severity_floor),
        tail: tail_label(logging.tail).to_string(),
        freshness: format_last_refresh(Local::now(), logging.freshness),
        status: status_text(&logging.status),
        filter_active: logging.filter_active,
        filter_draft: sanitize_log_text(&logging.filter_draft),
        applied_filter: sanitize_log_text(logging.applied_filter()),
        search_mode: search_mode(logging),
        search_error: logging
            .applied_filter_error()
            .as_deref()
            .map(sanitize_log_text),
    };

    let visible = logging.visible_entries();
    let table = log_table_model(logging, &visible);
    let detail = if logging.detail_open {
        logging
            .selected
            .and_then(|index| visible.get(index).copied())
            .map(|entry| log_detail_view(entry, logging.detail_scroll, logging.wrap))
    } else {
        None
    };

    let body_state = body_state(&logging.status, visible.is_empty());

    LoggingWorkspaceProjection {
        header,
        table,
        detail,
        body_state,
    }
}

/// The floor's display label. `LogSeverity::Default` is the "no floor"
/// sentinel, so it reads as `all` rather than as its own severity name.
fn severity_floor_label(floor: LogSeverity) -> String {
    match floor {
        LogSeverity::Default => "all".to_string(),
        other => other.label().to_string(),
    }
}

/// The mode label of the applied search pattern.
///
/// Read from the pure-syntax view rather than a compiled filter: this runs on
/// every frame, and naming the mode never needed the compile. The syntax view
/// and [`nuvem_domain::gcp::text_filter::TextFilter::parse`] consume the same
/// grammar decision, so the label can never describe a different pattern than
/// the one the rows were matched against.
///
/// The one thing syntax cannot see is whether a regex compiles, so an
/// unparseable pattern is suppressed here via
/// [`LoggingState::applied_filter_error`], which asks the compiled cache — it has
/// no mode to report, and the header shows `search_error` instead.
///
/// A pattern that matches everything (whitespace only, `//`, a bare `~`) also
/// reports no mode: it narrows nothing, so naming a mode would advertise an
/// active filter the rows do not reflect.
fn search_mode(logging: &LoggingState) -> Option<String> {
    if logging.applied_filter_error().is_some() {
        return None;
    }
    let applied = logging.applied_filter();
    if applied.is_empty() {
        return None;
    }
    let syntax = TextFilterSyntax::of(applied);
    if syntax.is_match_all() {
        return None;
    }
    Some(syntax.describe())
}

fn tail_label(tail: TailState) -> &'static str {
    match tail {
        TailState::Running => "Tailing",
        TailState::Paused => "Paused",
    }
}

fn status_text(status: &LoggingStatus) -> String {
    match status {
        LoggingStatus::Initial => "Not loaded".to_string(),
        LoggingStatus::Loading => "Loading...".to_string(),
        LoggingStatus::Loaded => "Loaded".to_string(),
        LoggingStatus::HealthyEmpty => "No matching logs".to_string(),
        LoggingStatus::Degraded { kind } => format!("Degraded: {}", kind.label()),
        LoggingStatus::PartialParse { dropped } => {
            format!("Loaded ({dropped} unparsed dropped)")
        }
    }
}

fn body_state(status: &LoggingStatus, is_empty: bool) -> LoggingBodyState {
    match status {
        LoggingStatus::Loading => LoggingBodyState::Loading,
        // `Initial` means no fetch was ever dispatched, so a spinner here would
        // be waiting for a result nobody asked for -- the same lie the header
        // stopped telling. Every path that actually dispatches a first load
        // installs `Loading` before the next render, so this arm never hides an
        // in-flight fetch.
        LoggingStatus::Initial => LoggingBodyState::NotLoaded,
        // A transient failure must not blank an already-populated tail: keep
        // rendering the last-known rows and surface the degraded state via the
        // header `status_text`. Only show the degraded body when empty.
        LoggingStatus::Degraded { kind } if is_empty => LoggingBodyState::Degraded {
            message: format!("Logging degraded: {}", kind.label()),
        },
        LoggingStatus::HealthyEmpty => LoggingBodyState::Empty,
        _ if is_empty => LoggingBodyState::Empty,
        _ => LoggingBodyState::Rows,
    }
}

fn severity_semantic(severity: LogSeverity) -> StatusSemantic {
    match severity {
        LogSeverity::Emergency
        | LogSeverity::Alert
        | LogSeverity::Critical
        | LogSeverity::Error => StatusSemantic::Degraded,
        LogSeverity::Warning => StatusSemantic::Warning,
        LogSeverity::Notice | LogSeverity::Info => StatusSemantic::Healthy,
        LogSeverity::Debug | LogSeverity::Default => StatusSemantic::Neutral,
    }
}

fn log_columns() -> Vec<TableColumn> {
    vec![
        TableColumn {
            key: "timestamp",
            header: "TIMESTAMP",
            min_width: 10,
            preferred_width: 24,
            priority: 0,
            align: CellAlign::Left,
            sortable: false,
        },
        TableColumn {
            key: "severity",
            header: "SEVERITY",
            min_width: 7,
            preferred_width: 10,
            priority: 1,
            align: CellAlign::Left,
            sortable: false,
        },
        TableColumn {
            key: "resource",
            header: "RESOURCE",
            min_width: 8,
            preferred_width: 20,
            priority: 2,
            align: CellAlign::Left,
            sortable: false,
        },
        TableColumn {
            key: "message",
            header: "MESSAGE",
            min_width: 10,
            preferred_width: 60,
            priority: 3,
            align: CellAlign::Left,
            sortable: false,
        },
    ]
}

fn log_table_model(logging: &LoggingState, visible: &[&LogEntry]) -> TableModel {
    let rows = visible
        .iter()
        .map(|entry| TableRow {
            cells: vec![
                Cell::plain(sanitize_log_text(&entry.timestamp)),
                Cell::status(
                    sanitize_log_text(entry.severity.label()),
                    severity_semantic(entry.severity),
                ),
                Cell::plain(sanitize_log_text(&entry.resource_type)),
                Cell::plain(sanitize_log_text(&entry.message)),
            ],
            marked: false,
        })
        .collect();
    TableModel {
        columns: log_columns(),
        rows,
        selected_row: logging.selected,
        selected_column: 0,
        sort: SortSpec {
            column: 0,
            direction: SortDirection::Descending,
        },
        counts: RowCounts {
            filtered: visible.len(),
            total: logging.entries.len(),
        },
        show_mark_column: false,
    }
}

fn log_detail_view(entry: &LogEntry, scroll: u16, wrap: bool) -> LogDetailView {
    let payload_lines = match &entry.payload {
        LogPayload::Text(text) | LogPayload::Json(text) => {
            text.lines().map(sanitize_log_text).collect::<Vec<_>>()
        }
        LogPayload::Empty => Vec::new(),
    };
    LogDetailView {
        severity: sanitize_log_text(entry.severity.label()),
        timestamp: sanitize_log_text(&entry.timestamp),
        resource_type: sanitize_log_text(&entry.resource_type),
        log_name: sanitize_log_text(&entry.log_name),
        message: sanitize_log_text(&entry.message),
        payload_lines,
        resource_labels: label_projections(&entry.resource_labels),
        labels: label_projections(&entry.labels),
        scroll,
        wrap,
    }
}

fn label_projections(
    labels: &std::collections::BTreeMap<String, String>,
) -> Vec<LogLabelProjection> {
    labels
        .iter()
        .map(|(key, value)| LogLabelProjection {
            key: sanitize_log_text(key),
            value: sanitize_log_text(value),
        })
        .collect()
}
