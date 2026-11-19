use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::text::Line;
use ratatui::widgets::{Clear, List, ListItem, ListState, Paragraph};

use crate::app::projection::detail_body::{DetailBody, project_detail_body};
use crate::app::projection::table::{NO_COLUMN, TableModel};
use crate::app::projection::{
    CloudRunDetailProjection, CloudRunRevisionProjection, CloudSqlDetailProjection,
    ComputeDetailProjection, GkeDetailProjection, LoadProgressProjection, ResourceDetailProjection,
    ResourcePreviewProjection, ResourceWorkspaceProjection, ResourceWorkspaceStatus,
    SecretDetailProjection, SortDirection, StorageBucketDetailProjection, StorageDetailProjection,
    StorageObjectDetailProjection, StorageProblemProjection, project_cloud_run_revision,
    project_resource_list, project_resource_preview, project_resources, project_service_navigator,
};
use crate::app::state::resources::{
    BigQueryView, CloudRunDetailTab, CloudRunRevisionTab, CloudSqlDetailTab, ComputeDetailTab,
    FilterScope, GkeDetailTab, ResourcesState, ResourcesViewMode, SecretDetailTab,
    StorageDetailTab, StorageView,
};
use crate::services::runtime::session::RuntimeSession;
use crate::ui::service_navigator::render_service_navigator;
use crate::ui::shared::components::{
    bordered_block, empty_state, error_state, error_state_item, fit_spans, fit_text, loading_line,
    loading_state, modal_block, stale_state, tab_line,
};
use crate::ui::shared::picker::viewport_window;
use crate::ui::shared::table::{
    MARK_COLUMN_RESERVED_WIDTH, ROW_SELECT_SYMBOL, display_width, plan_table, render_table,
};
use crate::ui::theme;
use nuvem_domain::gcp::diagnostics::GcloudErrorKind;
use nuvem_domain::gcp::resource::ResourceServiceKind;
use nuvem_domain::gcp::text::sanitize_display_text;

/// Terminal width, in columns, at or above which the resources view is
/// eligible to split into a left list pane and a right preview/detail pane.
pub const WIDE_LAYOUT_MIN_WIDTH: u16 = 100;
/// Minimum width of the resource content pane (i.e. the area left after the
/// service sidebar) required to actually engage the list/preview split. This
/// keeps the split from clipping panes when the sidebar consumes horizontal
/// room on a merely-100-column terminal, while still splitting on wider ones.
const WIDE_SPLIT_MIN_CONTENT_WIDTH: u16 = 88;
const SERVICE_SIDEBAR_WIDTH: u16 = 28;
const MIN_RESOURCE_CONTENT_WIDTH: u16 = 32;
const MIN_WIDE_LAYOUT_WIDTH: u16 = 80;

pub fn render(frame: &mut Frame<'_>, area: Rect, session: &RuntimeSession) {
    // The list+preview/detail split is driven by the total terminal width so
    // that it stays available whether or not the service sidebar is shown.
    let wide = area.width >= WIDE_LAYOUT_MIN_WIDTH;

    if !session.app().root.navigator.sidebar_visible() {
        render_resource_content(frame, area, session, false, wide);
        return;
    }

    if area.width > MIN_WIDE_LAYOUT_WIDTH {
        let areas = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Length(SERVICE_SIDEBAR_WIDTH),
                Constraint::Min(MIN_RESOURCE_CONTENT_WIDTH),
            ])
            .split(area);
        let projection = project_service_navigator(&session.app().root, areas[0]);
        render_service_navigator(frame, areas[0], &projection, "Service navigator");
        render_resource_content(frame, areas[1], session, false, wide);
        return;
    }

    let areas = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(0)])
        .split(area);
    let projection = project_service_navigator(&session.app().root, areas[0]);
    render_service_navigator(frame, areas[0], &projection, "Service navigator");
    render_resource_content(frame, areas[1], session, true, wide);
}

fn render_resource_content(
    frame: &mut Frame<'_>,
    area: Rect,
    session: &RuntimeSession,
    compact: bool,
    wide_terminal: bool,
) {
    let projection = project_resources(&session.app().root.resources);
    // Split only when the terminal is wide AND the content pane itself still
    // has room for two columns after the sidebar took its share.
    let wide = wide_terminal && area.width >= WIDE_SPLIT_MIN_CONTENT_WIDTH;

    match (wide, projection.view_mode) {
        (false, ResourcesViewMode::List) => {
            render_list_body(frame, area, session, &projection, compact)
        }
        (false, ResourcesViewMode::Detail) => {
            render_detail(frame, area, session, &projection, compact)
        }
        (true, ResourcesViewMode::List) => {
            // Built once and used twice. Both the split below and the pane it
            // sizes need the same preview, and each used to derive its own:
            // `project_resource_preview` reaches the selected row through
            // `visible_rows`, which filters AND sorts the whole list, so a
            // preview of a single row cost two full sorts of every row on
            // every frame — three in total, for a frame that needs one.
            let preview = project_resource_preview(&session.app().root.resources);
            let (left, right) = split_wide_for_preview(area, preview.as_ref());
            render_list_body(frame, left, session, &projection, compact);
            render_preview(frame, right, preview.as_ref());
        }
        (true, ResourcesViewMode::Detail) => {
            let (left, right) = split_wide(area);
            render_list_body(frame, left, session, &projection, compact);
            render_detail(frame, right, session, &projection, compact);
        }
    }
}

/// The most of the content pane either right-hand pane may take.
///
/// This was the *only* rule, applied as a flat `Percentage(45)`: the preview
/// took 45% whether or not it had anything to put there. At 132x40 on GCE
/// instances that left the table 57 columns and truncated almost every cell,
/// while the preview painted five short lines and thirty blank ones — so
/// widening the terminal from 100 (too narrow to split at all) to 132 made the
/// table *less* readable. It is a ceiling now, not an allocation.
const RIGHT_PANE_MAX_PERCENT: u16 = 45;
/// The narrowest the preview is allowed to become. Below this the pane stops
/// being worth its two border columns, and a preview holding one long value
/// would be truncated to nothing rather than merely to less than it wanted.
const PREVIEW_MIN_WIDTH: u16 = 24;

/// Splits the content pane, giving the right-hand pane `width` columns.
fn split_wide_at(area: Rect, width: u16) -> (Rect, Rect) {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Min(0), Constraint::Length(width)])
        .split(area);
    (chunks[0], chunks[1])
}

/// The detail pane keeps the ceiling as its share: unlike the preview it has
/// tabs, scrollable field lists and wrapped values, so what it "needs" is not
/// bounded by the lines it happens to be showing right now.
fn split_wide(area: Rect) -> (Rect, Rect) {
    split_wide_at(area, right_pane_ceiling(area))
}

fn right_pane_ceiling(area: Rect) -> u16 {
    area.width.saturating_mul(RIGHT_PANE_MAX_PERCENT) / 100
}

/// Splits for the preview: what its content actually needs, capped by the
/// ceiling and floored so it stays readable. Everything it does not need goes
/// to the table.
fn split_wide_for_preview(area: Rect, preview: Option<&ResourcePreviewProjection>) -> (Rect, Rect) {
    let ceiling = right_pane_ceiling(area);
    let wanted = preview_natural_width(preview).saturating_add(PREVIEW_CHROME_WIDTH);
    // The floor is itself capped by the ceiling: on a pane too narrow for
    // `PREVIEW_MIN_WIDTH` the ceiling is what there is, and `clamp` panics if
    // its lower bound exceeds its upper.
    split_wide_at(area, wanted.clamp(PREVIEW_MIN_WIDTH.min(ceiling), ceiling))
}

/// The preview block's two border columns.
const PREVIEW_CHROME_WIDTH: u16 = 2;

/// The widest line the preview would paint, before any fitting.
///
/// Measured from the same projection the renderer draws, so the width the pane
/// is given and the content it is given cannot disagree. A preview with no row
/// selected paints a fixed message and is measured from that.
fn preview_natural_width(preview: Option<&ResourcePreviewProjection>) -> u16 {
    match preview {
        Some(preview) => {
            let title = display_width(&preview.title);
            preview
                .lines
                .iter()
                .map(|(label, value)| display_width(&format!("{label}: {value}")))
                .chain(std::iter::once(title))
                .max()
                .unwrap_or(title)
        }
        None => display_width(PREVIEW_EMPTY_MESSAGE),
    }
}

/// Shown in the preview pane when no row is highlighted.
const PREVIEW_EMPTY_MESSAGE: &str = "No resource selected";

/// Renders the resource list pane. When `projection` was built in Detail
/// view mode (its `list` field is empty by contract — see
/// `project_resources`), a fresh list-only projection is derived from
/// session state so the left pane still shows rows in the wide layout.
fn render_list_body(
    frame: &mut Frame<'_>,
    area: Rect,
    session: &RuntimeSession,
    projection: &ResourceWorkspaceProjection<'_>,
    compact: bool,
) {
    let list_projection;
    let projection = match projection.view_mode {
        ResourcesViewMode::List => projection,
        ResourcesViewMode::Detail => {
            list_projection = project_resource_list(&session.app().root.resources);
            &list_projection
        }
    };

    match projection.service {
        ResourceServiceKind::ComputeInstances => {
            render_compute_list(frame, area, projection, compact)
        }
        ResourceServiceKind::GkeClusters => render_gke_list(frame, area, projection, compact),
        ResourceServiceKind::CloudRunServices => {
            render_cloud_run_list(frame, area, projection, compact)
        }
        ResourceServiceKind::CloudSql => render_cloudsql_list(frame, area, projection, compact),
        ResourceServiceKind::CloudStorage => {
            render_storage_list(frame, area, session, projection, compact)
        }
        ResourceServiceKind::SecretManagerSecrets => {
            render_secrets_list(frame, area, projection, compact)
        }
        ResourceServiceKind::CloudFunctions => {
            render_cloud_functions_list(frame, area, projection, compact)
        }
        ResourceServiceKind::PersistentDisks => {
            render_persistent_disks_list(frame, area, projection, compact)
        }
        ResourceServiceKind::PubSubTopics => {
            render_pubsub_topics_list(frame, area, projection, compact)
        }
        ResourceServiceKind::VpcNetworks => {
            render_vpc_networks_list(frame, area, projection, compact)
        }
        ResourceServiceKind::VpcSubnets => {
            render_vpc_subnets_list(frame, area, projection, compact)
        }
        ResourceServiceKind::FirewallRules => {
            render_firewall_rules_list(frame, area, projection, compact)
        }
        ResourceServiceKind::PubSubSubscriptions => {
            render_pubsub_subscriptions_list(frame, area, projection, compact)
        }
        ResourceServiceKind::BigQueryDatasets => {
            render_bigquery_list(frame, area, session, projection, compact)
        }
        ResourceServiceKind::IamServiceAccounts => {
            render_service_accounts_list(frame, area, projection, compact)
        }
        ResourceServiceKind::IamPolicy => render_iam_policy_list(frame, area, projection, compact),
    }
}

fn render_preview(frame: &mut Frame<'_>, area: Rect, preview: Option<&ResourcePreviewProjection>) {
    let block = bordered_block("Preview");
    match preview {
        Some(preview) => {
            let inner_width = usize::from(area.width.saturating_sub(2));
            let mut lines = vec![Line::from(fit_text(&preview.title, inner_width))];
            for (label, value) in &preview.lines {
                lines.push(Line::from(fit_text(
                    &format!("{label}: {value}"),
                    inner_width,
                )));
            }
            frame.render_widget(Paragraph::new(lines).block(block), area);
        }
        None => {
            frame.render_widget(Paragraph::new(PREVIEW_EMPTY_MESSAGE).block(block), area);
        }
    }
}

fn render_compute_list(
    frame: &mut Frame<'_>,
    area: Rect,
    projection: &ResourceWorkspaceProjection<'_>,
    compact: bool,
) {
    if let Some(model) = projection.table.as_ref()
        && !model.rows.is_empty()
    {
        render_resource_table(
            frame,
            area,
            projection,
            model,
            "Resources / GCE instances".to_string(),
        );
        return;
    }

    let items = if projection.status == ResourceWorkspaceStatus::Loading {
        loading_items(projection.load_progress.as_ref(), "GCE instances")
    } else if projection.status == ResourceWorkspaceStatus::Failed {
        failed_items(projection.load_error.as_deref(), "GCE instances")
    } else if projection.status == ResourceWorkspaceStatus::Stale {
        vec![stale_state("Resource rows are stale. Press r to refresh.")]
    } else if projection.total_rows == 0 && projection.status == ResourceWorkspaceStatus::Ready {
        let name = projection
            .context_label
            .as_deref()
            .unwrap_or("active configuration");
        vec![empty_state(format!("No GCE instances found for {name}"))]
    } else if projection.total_rows == 0 {
        vec![loading_state("Press r to load GCE instances.")]
    } else if projection.visible_rows == 0 {
        vec![empty_state(format!(
            "No GCE instances match filter \"{}\"",
            projection.filter.applied
        ))]
    } else {
        unreachable!("non-empty table rows should have been rendered above")
    };
    let list = List::new(items)
        .block(bordered_block(compact_list_title(
            "Resources / GCE instances",
            projection,
            compact,
        )))
        .highlight_symbol("");
    render_list_with_optional_header(
        frame,
        area,
        list,
        (!compact).then(|| list_tools_line(projection)).flatten(),
        projection.selected_visible,
    );
}

fn render_gke_list(
    frame: &mut Frame<'_>,
    area: Rect,
    projection: &ResourceWorkspaceProjection<'_>,
    compact: bool,
) {
    if let Some(model) = projection.table.as_ref()
        && !model.rows.is_empty()
    {
        render_resource_table(
            frame,
            area,
            projection,
            model,
            "Resources / GKE clusters".to_string(),
        );
        return;
    }

    let items = if projection.status == ResourceWorkspaceStatus::Loading {
        loading_items(projection.load_progress.as_ref(), "GKE clusters")
    } else if projection.status == ResourceWorkspaceStatus::Failed {
        failed_items(projection.load_error.as_deref(), "GKE clusters")
    } else if projection.status == ResourceWorkspaceStatus::Stale {
        vec![stale_state("Resource rows are stale. Press r to refresh.")]
    } else if projection.total_rows == 0 && projection.status == ResourceWorkspaceStatus::Ready {
        let name = projection
            .context_label
            .as_deref()
            .unwrap_or("active configuration");
        vec![empty_state(format!("No GKE clusters found for {name}"))]
    } else if projection.total_rows == 0 {
        vec![loading_state("Press r to load GKE clusters.")]
    } else if projection.visible_rows == 0 {
        vec![empty_state(format!(
            "No GKE clusters match filter \"{}\"",
            projection.filter.applied
        ))]
    } else {
        unreachable!("non-empty table rows should have been rendered above")
    };
    let list = List::new(items)
        .block(bordered_block(compact_list_title(
            "Resources / GKE clusters",
            projection,
            compact,
        )))
        .highlight_symbol("");
    render_list_with_optional_header(
        frame,
        area,
        list,
        (!compact).then(|| list_tools_line(projection)).flatten(),
        projection.selected_visible,
    );
}

fn render_storage_list(
    frame: &mut Frame<'_>,
    area: Rect,
    session: &RuntimeSession,
    projection: &ResourceWorkspaceProjection<'_>,
    compact: bool,
) {
    let storage = session.app().root.resources.storage();
    let bucket_list = matches!(
        storage.view(),
        StorageView::Buckets | StorageView::BucketDetail
    );
    let title = if bucket_list {
        "Resources / Cloud Storage".to_string()
    } else {
        let bucket = storage
            .browser_bucket()
            .map(|bucket| sanitize_display_text(&bucket.name))
            .unwrap_or_default();
        let prefix = sanitize_display_text(&storage.current_prefix());
        format!("Resources / gs://{bucket}/{prefix}")
    };

    if let Some(model) = projection.table.as_ref()
        && !model.rows.is_empty()
    {
        render_resource_table(frame, area, projection, model, title);
        return;
    }

    let noun = if bucket_list { "buckets" } else { "objects" };
    let items = if let Some(problem) = projection.storage_problem.as_ref() {
        vec![storage_problem_state(problem)]
    } else if projection.status == ResourceWorkspaceStatus::Loading {
        loading_items(projection.load_progress.as_ref(), noun)
    } else if projection.status == ResourceWorkspaceStatus::Failed {
        failed_items(projection.load_error.as_deref(), noun)
    } else if projection.status == ResourceWorkspaceStatus::Stale {
        vec![stale_state("Resource rows are stale. Press r to refresh.")]
    } else if projection.total_rows == 0 && projection.status == ResourceWorkspaceStatus::Ready {
        if bucket_list {
            let name = projection
                .context_label
                .as_deref()
                .unwrap_or("active configuration");
            vec![empty_state(format!("No buckets found for {name}"))]
        } else {
            vec![empty_state("This prefix is empty".to_string())]
        }
    } else if projection.total_rows == 0 {
        vec![loading_state(format!("Press r to load {noun}."))]
    } else if projection.visible_rows == 0 {
        vec![empty_state(format!(
            "No {noun} match filter \"{}\"",
            projection.filter.applied
        ))]
    } else {
        unreachable!("non-empty table rows should have been rendered above")
    };
    let list = List::new(items)
        .block(bordered_block(storage_compact_title(
            title, projection, compact,
        )))
        .highlight_symbol("");
    render_list_with_optional_header(
        frame,
        area,
        list,
        (!compact).then(|| list_tools_line(projection)).flatten(),
        projection.selected_visible,
    );
}

fn render_cloud_run_list(
    frame: &mut Frame<'_>,
    area: Rect,
    projection: &ResourceWorkspaceProjection<'_>,
    compact: bool,
) {
    if let Some(model) = projection.table.as_ref()
        && !model.rows.is_empty()
    {
        render_resource_table(frame, area, projection, model, "Resources / Cloud Run");
        return;
    }

    let items = if projection.status == ResourceWorkspaceStatus::Loading {
        loading_items(projection.load_progress.as_ref(), "Cloud Run services")
    } else if projection.status == ResourceWorkspaceStatus::Failed {
        failed_items(projection.load_error.as_deref(), "Cloud Run services")
    } else if projection.status == ResourceWorkspaceStatus::Stale {
        vec![stale_state("Resource rows are stale. Press r to refresh.")]
    } else if projection.total_rows == 0 && projection.status == ResourceWorkspaceStatus::Ready {
        let name = projection
            .context_label
            .as_deref()
            .unwrap_or("active configuration");
        vec![empty_state(format!(
            "No Cloud Run services found for {name}"
        ))]
    } else if projection.total_rows == 0 {
        vec![loading_state("Press r to load Cloud Run services.")]
    } else if projection.visible_rows == 0 {
        vec![empty_state(format!(
            "No Cloud Run services match filter \"{}\"",
            projection.filter.applied
        ))]
    } else {
        unreachable!("non-empty table rows should have been rendered above")
    };
    let list = List::new(items)
        .block(bordered_block(compact_list_title(
            "Resources / Cloud Run",
            projection,
            compact,
        )))
        .highlight_symbol("");
    render_list_with_optional_header(
        frame,
        area,
        list,
        (!compact).then(|| list_tools_line(projection)).flatten(),
        projection.selected_visible,
    );
}

fn render_cloud_functions_list(
    frame: &mut Frame<'_>,
    area: Rect,
    projection: &ResourceWorkspaceProjection<'_>,
    compact: bool,
) {
    if let Some(model) = projection.table.as_ref()
        && !model.rows.is_empty()
    {
        render_resource_table(
            frame,
            area,
            projection,
            model,
            "Resources / Cloud Functions",
        );
        return;
    }

    let items = if projection.status == ResourceWorkspaceStatus::Loading {
        loading_items(projection.load_progress.as_ref(), "Cloud Functions")
    } else if projection.status == ResourceWorkspaceStatus::Failed {
        failed_items(projection.load_error.as_deref(), "Cloud Functions")
    } else if projection.status == ResourceWorkspaceStatus::Stale {
        vec![stale_state("Resource rows are stale. Press r to refresh.")]
    } else if projection.total_rows == 0 && projection.status == ResourceWorkspaceStatus::Ready {
        let name = projection
            .context_label
            .as_deref()
            .unwrap_or("active configuration");
        vec![empty_state(format!("No Cloud Functions found for {name}"))]
    } else if projection.total_rows == 0 {
        vec![loading_state("Press r to load Cloud Functions.")]
    } else if projection.visible_rows == 0 {
        vec![empty_state(format!(
            "No Cloud Functions match filter \"{}\"",
            projection.filter.applied
        ))]
    } else {
        unreachable!("non-empty table rows should have been rendered above")
    };
    let list = List::new(items)
        .block(bordered_block(compact_list_title(
            "Resources / Cloud Functions",
            projection,
            compact,
        )))
        .highlight_symbol("");
    render_list_with_optional_header(
        frame,
        area,
        list,
        (!compact).then(|| list_tools_line(projection)).flatten(),
        projection.selected_visible,
    );
}

fn render_persistent_disks_list(
    frame: &mut Frame<'_>,
    area: Rect,
    projection: &ResourceWorkspaceProjection<'_>,
    compact: bool,
) {
    if let Some(model) = projection.table.as_ref()
        && !model.rows.is_empty()
    {
        render_resource_table(
            frame,
            area,
            projection,
            model,
            "Resources / Persistent disks",
        );
        return;
    }

    let items = if projection.status == ResourceWorkspaceStatus::Loading {
        loading_items(projection.load_progress.as_ref(), "persistent disks")
    } else if projection.status == ResourceWorkspaceStatus::Failed {
        failed_items(projection.load_error.as_deref(), "persistent disks")
    } else if projection.status == ResourceWorkspaceStatus::Stale {
        vec![stale_state("Resource rows are stale. Press r to refresh.")]
    } else if projection.total_rows == 0 && projection.status == ResourceWorkspaceStatus::Ready {
        let name = projection
            .context_label
            .as_deref()
            .unwrap_or("active configuration");
        vec![empty_state(format!("No persistent disks found for {name}"))]
    } else if projection.total_rows == 0 {
        vec![loading_state("Press r to load persistent disks.")]
    } else if projection.visible_rows == 0 {
        vec![empty_state(format!(
            "No persistent disks match filter \"{}\"",
            projection.filter.applied
        ))]
    } else {
        unreachable!("non-empty table rows should have been rendered above")
    };
    let list = List::new(items)
        .block(bordered_block(compact_list_title(
            "Resources / Persistent disks",
            projection,
            compact,
        )))
        .highlight_symbol("");
    render_list_with_optional_header(
        frame,
        area,
        list,
        (!compact).then(|| list_tools_line(projection)).flatten(),
        projection.selected_visible,
    );
}

fn render_secrets_list(
    frame: &mut Frame<'_>,
    area: Rect,
    projection: &ResourceWorkspaceProjection<'_>,
    compact: bool,
) {
    if let Some(model) = projection.table.as_ref()
        && !model.rows.is_empty()
    {
        render_resource_table(frame, area, projection, model, "Resources / Secret Manager");
        return;
    }

    let items = if projection.status == ResourceWorkspaceStatus::Loading {
        loading_items(projection.load_progress.as_ref(), "secrets")
    } else if projection.status == ResourceWorkspaceStatus::Failed {
        failed_items(projection.load_error.as_deref(), "secrets")
    } else if projection.status == ResourceWorkspaceStatus::Stale {
        vec![stale_state("Resource rows are stale. Press r to refresh.")]
    } else if projection.total_rows == 0 && projection.status == ResourceWorkspaceStatus::Ready {
        let name = projection
            .context_label
            .as_deref()
            .unwrap_or("active configuration");
        vec![empty_state(format!("No secrets found for {name}"))]
    } else if projection.total_rows == 0 {
        vec![loading_state("Press r to load secrets.")]
    } else if projection.visible_rows == 0 {
        vec![empty_state(format!(
            "No secrets match filter \"{}\"",
            projection.filter.applied
        ))]
    } else {
        unreachable!("non-empty table rows should have been rendered above")
    };
    let list = List::new(items)
        .block(bordered_block(compact_list_title(
            "Resources / Secret Manager",
            projection,
            compact,
        )))
        .highlight_symbol("");
    render_list_with_optional_header(
        frame,
        area,
        list,
        (!compact).then(|| list_tools_line(projection)).flatten(),
        projection.selected_visible,
    );
}

fn render_cloudsql_list(
    frame: &mut Frame<'_>,
    area: Rect,
    projection: &ResourceWorkspaceProjection<'_>,
    compact: bool,
) {
    if let Some(model) = projection.table.as_ref()
        && !model.rows.is_empty()
    {
        render_resource_table(frame, area, projection, model, "Resources / Cloud SQL");
        return;
    }

    let items = if projection.status == ResourceWorkspaceStatus::Loading {
        loading_items(projection.load_progress.as_ref(), "Cloud SQL instances")
    } else if projection.status == ResourceWorkspaceStatus::Failed {
        failed_items(projection.load_error.as_deref(), "Cloud SQL instances")
    } else if projection.status == ResourceWorkspaceStatus::Stale {
        vec![stale_state("Resource rows are stale. Press r to refresh.")]
    } else if projection.total_rows == 0 && projection.status == ResourceWorkspaceStatus::Ready {
        let name = projection
            .context_label
            .as_deref()
            .unwrap_or("active configuration");
        vec![empty_state(format!(
            "No Cloud SQL instances found for {name}"
        ))]
    } else if projection.total_rows == 0 {
        vec![loading_state("Press r to load Cloud SQL instances.")]
    } else if projection.visible_rows == 0 {
        vec![empty_state(format!(
            "No Cloud SQL instances match filter \"{}\"",
            projection.filter.applied
        ))]
    } else {
        unreachable!("non-empty table rows should have been rendered above")
    };
    let list = List::new(items)
        .block(bordered_block(compact_list_title(
            "Resources / Cloud SQL",
            projection,
            compact,
        )))
        .highlight_symbol("");
    render_list_with_optional_header(
        frame,
        area,
        list,
        (!compact).then(|| list_tools_line(projection)).flatten(),
        projection.selected_visible,
    );
}

/// Renders the in-flight fetch affordance shared by every per-service list
/// renderer: a spinner frame plus elapsed seconds, derived from
/// `projection.load_progress` (`Some` iff `status == Loading`). Falls back to
/// the initial frame/zero elapsed if `progress` is somehow `None` -- the
/// projection guarantees `Some` here, but the renderer must not panic.
fn loading_items(progress: Option<&LoadProgressProjection>, label: &str) -> Vec<ListItem<'static>> {
    let (frame, elapsed_secs) = progress
        .map(|progress| (progress.spinner_frame, progress.elapsed_secs))
        .unwrap_or(("⠋", 0));
    vec![loading_state(format!(
        "{frame} Loading {label}… ({elapsed_secs}s elapsed)"
    ))]
}

/// Renders the list-scoped load-failure affordance shared by every
/// per-service list renderer: the `load_error` message (`Some` iff
/// `status == Failed`) followed by a "Press r to retry." item. Falls back to
/// a generic message if `load_error` is somehow `None` -- the projection
/// guarantees `Some` here, but the renderer must not panic.
fn failed_items(load_error: Option<&str>, label: &str) -> Vec<ListItem<'static>> {
    let message = load_error
        .map(str::to_string)
        .unwrap_or_else(|| format!("Failed to load {label}."));
    vec![
        error_state_item(message),
        error_state_item("Press r to retry."),
    ]
}

/// Renders a Cloud Storage list-load failure as a distinct list item,
/// rather than the generic "stale" or "empty" states: requester-pays gets
/// its own message (a billing/access condition, not a permission failure),
/// permission-denied gets a permission-specific message, and every other
/// classified failure falls back to a generic "failed to load" message
/// carrying the underlying (sanitized) error text.
fn storage_problem_state(problem: &StorageProblemProjection) -> ListItem<'static> {
    if problem.requester_pays {
        return error_state_item(format!(
            "Requester pays: this bucket requires billing to a user project ({})",
            problem.message
        ));
    }
    match problem.kind {
        GcloudErrorKind::PermissionDenied => {
            error_state_item(format!("Permission denied: {}", problem.message))
        }
        _ => error_state_item(format!("Failed to load: {}", problem.message)),
    }
}

fn storage_compact_title(
    title: String,
    projection: &ResourceWorkspaceProjection<'_>,
    compact: bool,
) -> String {
    if compact && let Some(tools) = list_tools_line(projection) {
        return format!("{title}  {tools}");
    }
    title
}

fn compact_list_title(
    title: &'static str,
    projection: &ResourceWorkspaceProjection<'_>,
    compact: bool,
) -> String {
    if compact && let Some(tools) = list_tools_line(projection) {
        return format!("{title}  {tools}");
    }
    title.to_string()
}

fn render_resource_table(
    frame: &mut Frame<'_>,
    area: Rect,
    projection: &ResourceWorkspaceProjection<'_>,
    model: &TableModel,
    block_title: impl Into<String>,
) {
    let block_title = block_title.into();
    // A page-append (`NextPage`) failure never clears already-loaded rows;
    // it gets a non-destructive banner line above the table instead of
    // replacing the list with an error state (Cloud Storage object browser
    // only -- `partial_page_error` is always `None` for other services).
    let banner = projection.partial_page_error.as_deref();
    let constraints: Vec<Constraint> = if banner.is_some() {
        vec![
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Min(0),
        ]
    } else {
        vec![Constraint::Length(1), Constraint::Min(0)]
    };
    let areas = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(area);
    let table_area = areas[areas.len() - 1];
    let block = bordered_block(block_title);
    let inner = block.inner(table_area);
    let marker_w = display_width(ROW_SELECT_SYMBOL);
    let reserved = if model.show_mark_column {
        marker_w.saturating_add(MARK_COLUMN_RESERVED_WIDTH)
    } else {
        marker_w
    };
    let plan = plan_table(
        model,
        inner.width.saturating_sub(reserved),
        projection.wide_columns,
    );
    // Fitted, not handed to the widget whole: a bare `Paragraph` is clipped at
    // the pane edge with nothing to say it was, and runs straight into the
    // border of the pane beside it. See `fitted_table_tools_line`.
    let tools = fitted_table_tools_line(
        projection,
        model,
        plan.hidden_count,
        plan.overflow,
        usize::from(areas[0].width),
    );
    frame.render_widget(Paragraph::new(tools), areas[0]);
    if let Some(message) = banner {
        let banner_line = error_state(format!("Next page failed: {message} (press r to retry)"));
        frame.render_widget(Paragraph::new(banner_line), areas[1]);
    }
    frame.render_widget(block, table_area);
    render_table(frame, inner, model, &plan);
}

/// One part of the tools line, with how much it matters when the line has to
/// lose something. Lower is kept longer.
struct ToolsSegment {
    text: String,
    /// A key hint (`w wide`) rather than a fact about the table.
    hint: bool,
    priority: u8,
}

fn table_tools_segments(
    projection: &ResourceWorkspaceProjection<'_>,
    model: &TableModel,
    hidden_count: usize,
    overflow: bool,
) -> Vec<ToolsSegment> {
    let fact = |text: String, priority: u8| ToolsSegment {
        text,
        hint: false,
        priority,
    };
    let hint = |text: &str, priority: u8| ToolsSegment {
        text: text.to_string(),
        hint: true,
        priority,
    };
    if projection.filter.active {
        return vec![fact(
            format!(
                "Filter: {}_{}",
                projection.filter.draft,
                draft_filter_mode_suffix(projection)
            ),
            0,
        )];
    }
    let filter = filter_label(projection);
    let suffix = applied_filter_mode_suffix(projection);
    let arrow = match model.sort.direction {
        SortDirection::Ascending => '▲',
        SortDirection::Descending => '▼',
    };
    let rows = fact(
        format!("Rows: {}/{}", model.counts.filtered, model.counts.total),
        1,
    );
    // A table whose sort points at [`NO_COLUMN`] has neither a sorted column
    // nor a column cursor (the BigQuery tables drill-down is the one such
    // resource table today). It gets neither the sort segment nor the
    // ←/→ and `s` hints: the arrow would claim an ascending order over rows
    // that arrive in listing order, and the hints would advertise two controls
    // this table does not offer.
    let mut segments = if model.sort.column == NO_COLUMN {
        vec![
            fact(format!("Filter: {filter}{suffix}"), 0),
            rows,
            hint("w wide", 4),
        ]
    } else {
        vec![
            fact(format!("Filter: {filter}{suffix}"), 0),
            fact(format!("Sort: {} {arrow}", projection.sort_label), 3),
            rows,
            hint("←/→ col", 6),
            hint("s sort", 5),
            hint("w wide", 4),
        ]
    };
    if projection.wide_columns {
        segments.push(fact("[WIDE]".to_string(), 2));
    }
    if !projection.wide_columns && hidden_count > 0 {
        // This fact names `w` itself, and says what it would show, so the
        // bare `w wide` hint goes: the line taught the same key twice.
        segments.retain(|segment| segment.text != "w wide");
        segments.push(fact(format!("+{hidden_count} cols (w)"), 2));
    }
    if projection.wide_columns && overflow {
        segments.push(fact("▸ more".to_string(), 2));
    }
    if let Some(page_hint) = page_indicator(projection) {
        segments.push(fact(page_hint, 2));
    }
    segments
}

/// Joins segments the way the tools line spells them: facts three spaces
/// apart, and a run of key hints joined by ` · `.
fn join_tools_segments<'a>(segments: impl IntoIterator<Item = &'a ToolsSegment>) -> String {
    let mut line = String::new();
    let mut previous_was_hint = false;
    for segment in segments {
        if !line.is_empty() {
            line.push_str(if previous_was_hint && segment.hint {
                " · "
            } else {
                "   "
            });
        }
        line.push_str(&segment.text);
        previous_was_hint = segment.hint;
    }
    line
}

/// The whole tools line, as it reads with room for all of it.
#[cfg(test)]
fn table_tools_line(
    projection: &ResourceWorkspaceProjection<'_>,
    model: &TableModel,
    hidden_count: usize,
    overflow: bool,
) -> String {
    join_tools_segments(&table_tools_segments(
        projection,
        model,
        hidden_count,
        overflow,
    ))
}

/// The tools line fitted to `width` columns.
///
/// One column is kept clear at the right, so the line never runs into the
/// border title of a pane beside it (`w wide┌Preview`). Within that, whole
/// segments are dropped, least important first — the key hints before the
/// sort, the sort before the row count and the filter — so a hint is never
/// cut to `←/→ c...`. Only when the filter and the row count alone are too
/// wide is the line cut, and then it ends in the overflow marker so a cut line
/// cannot pass for a complete one.
fn fitted_table_tools_line(
    projection: &ResourceWorkspaceProjection<'_>,
    model: &TableModel,
    hidden_count: usize,
    overflow: bool,
    width: usize,
) -> String {
    let budget = width.saturating_sub(1);
    let segments = table_tools_segments(projection, model, hidden_count, overflow);
    let mut kept: Vec<bool> = vec![true; segments.len()];
    loop {
        let line = join_tools_segments(
            segments
                .iter()
                .zip(&kept)
                .filter(|(_, keep)| **keep)
                .map(|(segment, _)| segment),
        );
        if usize::from(display_width(&line)) <= budget {
            return line;
        }
        let victim = segments
            .iter()
            .enumerate()
            .filter(|(index, segment)| kept[*index] && segment.priority > 1)
            .max_by_key(|(index, segment)| (segment.priority, *index))
            .map(|(index, _)| index);
        match victim {
            Some(index) => kept[index] = false,
            None => return fit_text(&line, budget),
        }
    }
}

/// Renders the applied-filter label for the table tools line, appending a
/// local/remote scope tag for Cloud Storage — the only service where an
/// applied filter can also narrow the server-side listing via a safe
/// prefix (see `is_safe_remote_prefix`).
fn filter_label(projection: &ResourceWorkspaceProjection<'_>) -> String {
    if projection.filter.applied.is_empty() {
        return "none".to_string();
    }
    if projection.service != ResourceServiceKind::CloudStorage {
        return projection.filter.applied.clone();
    }
    match projection.filter_scope {
        FilterScope::Local => format!("{} (local)", projection.filter.applied),
        FilterScope::Remote => format!("{} (remote)", projection.filter.applied),
    }
}

/// The `  [<mode>]` chip, plus `  ⚠ <error>` when `with_error` and the filter
/// did not parse or compile, as rendered immediately after the filter text.
///
/// Empty for Cloud Storage: its filter is a deliberate plain substring match,
/// so a mode chip there would advertise a grammar the service does not have.
/// Both values are read from the projection — the filter text is never
/// reparsed here.
fn filter_mode_suffix(projection: &ResourceWorkspaceProjection<'_>, with_error: bool) -> String {
    if projection.service == ResourceServiceKind::CloudStorage {
        return String::new();
    }
    let mut suffix = format!("  [{}]", projection.filter.mode_label);
    if let (true, Some(error)) = (with_error, &projection.filter.error) {
        suffix.push_str("  ⚠ ");
        suffix.push_str(error);
    }
    suffix
}

/// [`filter_mode_suffix`] for the active-input tools line, where the chip
/// tracks the *draft* but `filter.error` is about the *applied* text.
///
/// The error is therefore emitted only while the draft still equals what is
/// applied: reopening `/` without editing still explains the failure, while
/// the first keystroke of a fix drops a message that no longer describes what
/// is on screen — and that, for a fuzzy or label draft, would claim a regex
/// failure those modes cannot produce.
///
/// "Equals" is measured on the trimmed text, because everything downstream
/// already trims: `ResourcesState::apply_filter` stores the trimmed draft, and
/// `parse_filter` trims again before dispatching. `a(b ` and `a(b` are the same
/// filter and fail identically, so a stray trailing space must not read as a
/// fix in progress.
///
/// Unlike [`applied_filter_mode_suffix`], this has no empty-filter guard: an
/// open input with nothing typed still renders `[regex]`. The asymmetry is
/// deliberate. An idle `Filter: none` should make no claim about a filter that
/// is not there, whereas an open input is a claim about the text you are about
/// to type, and naming the default mode before the first keystroke is what
/// makes the grammar discoverable at all.
fn draft_filter_mode_suffix(projection: &ResourceWorkspaceProjection<'_>) -> String {
    filter_mode_suffix(
        projection,
        projection.filter.draft.trim() == projection.filter.applied.trim(),
    )
}

/// [`filter_mode_suffix`] for the applied (input-inactive) tools line, where
/// an empty filter renders as a bare `Filter: none` with nothing to label.
fn applied_filter_mode_suffix(projection: &ResourceWorkspaceProjection<'_>) -> String {
    if projection.filter.applied.is_empty() {
        return String::new();
    }
    filter_mode_suffix(projection, true)
}

fn list_tools_line(projection: &ResourceWorkspaceProjection<'_>) -> Option<String> {
    if projection.total_rows == 0
        && projection.filter.applied.is_empty()
        && !projection.filter.active
    {
        return None;
    }
    if projection.filter.active {
        return Some(format!(
            "Filter: {}_{}",
            projection.filter.draft,
            draft_filter_mode_suffix(projection)
        ));
    }

    let filter = if projection.filter.applied.is_empty() {
        "none"
    } else {
        projection.filter.applied.as_str()
    };
    let suffix = applied_filter_mode_suffix(projection);
    let mut line = format!(
        "Filter: {filter}{suffix}   Sort: {}   Rows: {}/{}",
        projection.sort_label, projection.visible_rows, projection.total_rows
    );
    if let Some(page_hint) = page_indicator(projection) {
        line.push_str("   ");
        line.push_str(&page_hint);
    }
    Some(line)
}

/// Minimal "Page k / more available / end of listing" hint for bounded
/// on-demand pagination (currently only the Cloud Storage object browser).
/// Returns `None` when pagination doesn't apply, e.g. the first page hasn't
/// loaded yet and no continuation token has ever been seen.
fn page_indicator(projection: &ResourceWorkspaceProjection<'_>) -> Option<String> {
    if projection.page_index == 0 && !projection.has_more {
        return None;
    }
    let page = projection.page_index + 1;
    let status = if projection.has_more {
        "more available"
    } else {
        "end of listing"
    };
    Some(format!("Page {page} ({status})"))
}

fn render_detail(
    frame: &mut Frame<'_>,
    area: Rect,
    session: &RuntimeSession,
    projection: &ResourceWorkspaceProjection<'_>,
    compact: bool,
) {
    // The body is built once, here, and handed to whichever renderer draws it:
    // it is the same body the reducer copies from, so the highlighted line and
    // the yanked value cannot come from two independently resolved bodies —
    // and the pane no longer rebuilds it a second time per frame.
    let body = project_detail_body(&session.app().root.resources);
    let selected = body
        .as_ref()
        .and_then(|body| selected_detail_line(&session.app().root.resources, body));

    // The Cloud Run revision drill-down layers on top of the service detail:
    // when a revision view is active it takes over the detail pane entirely.
    // Only take this branch while Cloud Run is the active service; otherwise a
    // stale revision view (left over from a prior visit to Cloud Run) must not
    // bleed into another service's detail pane.
    if session.app().root.resources.active_service() == ResourceServiceKind::CloudRunServices
        && let Some(revision) = project_cloud_run_revision(&session.app().root.resources)
    {
        render_cloud_run_revision_detail(frame, area, &revision, compact, body.as_ref(), selected);
        return;
    }

    let body = body.as_ref();
    match projection.detail.as_ref() {
        Some(ResourceDetailProjection::Compute(detail)) => {
            render_compute_detail(frame, area, detail, compact, body, selected)
        }
        Some(ResourceDetailProjection::Gke(detail)) => {
            render_gke_detail(frame, area, detail, compact, body, selected)
        }
        Some(ResourceDetailProjection::CloudRun(detail)) => {
            render_cloud_run_detail(frame, area, detail, compact, body, selected)
        }
        Some(ResourceDetailProjection::Secrets(detail)) => {
            render_secret_detail(frame, area, detail, compact, body, selected)
        }
        Some(ResourceDetailProjection::CloudSql(detail)) => {
            render_cloudsql_detail(frame, area, detail, compact, body, selected)
        }
        Some(ResourceDetailProjection::Storage(detail)) => {
            render_storage_detail(frame, area, detail, compact, body, selected)
        }
        None => unreachable!("detail view must have a detail projection"),
    }
}

fn render_compute_detail(
    frame: &mut Frame<'_>,
    area: Rect,
    projection: &ComputeDetailProjection<'_>,
    compact: bool,
    body: Option<&DetailBody>,
    selected_line: Option<usize>,
) {
    let target_name = projection.target_name.as_str();
    let mut lines = detail_header_lines(
        format!("GCE instance / {target_name}"),
        tab_line(
            ComputeDetailTab::ALL
                .iter()
                .map(|tab| (*tab == projection.active_tab, tab.label())),
        ),
        area,
        compact,
    );
    let header_len = lines.len();
    // Only a rendered body can hold a selection; an error or loading pane has
    // no fields, so ensure-visible must not act on a leftover cursor.
    let mut selected = None;
    if let Some(error) = projection.error {
        lines.push(error_state(format!("Detail load failed for {target_name}")));
        lines.push(error_state(error));
        lines.push(error_state(
            "Press r to refresh the list or Esc/q to return to the list.",
        ));
    } else if let Some(body) = body {
        selected = selected_line;
        push_body_lines(&mut lines, body, selected, area);
    } else {
        lines.push(loading_line(fit_text(
            &format!("Loading detail for {target_name}..."),
            detail_width(area),
        )));
    }

    let scroll =
        detail_scroll_for_selection(projection.scroll, selected, header_len, lines.len(), area);
    let paragraph = Paragraph::new(lines)
        .block(bordered_block("GCE instance detail"))
        .scroll((scroll, 0));
    frame.render_widget(paragraph, area);
}

fn render_gke_detail(
    frame: &mut Frame<'_>,
    area: Rect,
    projection: &GkeDetailProjection<'_>,
    compact: bool,
    body: Option<&DetailBody>,
    selected_line: Option<usize>,
) {
    let target_name = projection.target_name.as_str();
    let mut lines = detail_header_lines(
        format!("GKE cluster / {target_name}"),
        tab_line(
            GkeDetailTab::ALL
                .iter()
                .map(|tab| (*tab == projection.active_tab, tab.label())),
        ),
        area,
        compact,
    );
    let header_len = lines.len();
    let mut selected = None;
    if let Some(error) = projection.error {
        lines.push(error_state(format!("Detail load failed for {target_name}")));
        lines.push(error_state(error));
        lines.push(error_state(
            "Press r to refresh the list or Esc/q to return to the list.",
        ));
    } else if let Some(body) = body {
        selected = selected_line;
        push_body_lines(&mut lines, body, selected, area);
    } else {
        lines.push(loading_line(fit_text(
            &format!("Loading detail for {target_name}..."),
            detail_width(area),
        )));
    }

    let scroll =
        detail_scroll_for_selection(projection.scroll, selected, header_len, lines.len(), area);
    let paragraph = Paragraph::new(lines)
        .block(bordered_block("GKE cluster detail"))
        .scroll((scroll, 0));
    frame.render_widget(paragraph, area);
}

fn render_storage_detail(
    frame: &mut Frame<'_>,
    area: Rect,
    projection: &StorageDetailProjection<'_>,
    compact: bool,
    body: Option<&DetailBody>,
    selected_line: Option<usize>,
) {
    match projection {
        StorageDetailProjection::Bucket(detail) => {
            render_storage_bucket_detail(frame, area, detail, compact, body, selected_line)
        }
        StorageDetailProjection::Object(detail) => {
            render_storage_object_detail(frame, area, detail, compact, body, selected_line)
        }
    }
}

fn render_storage_bucket_detail(
    frame: &mut Frame<'_>,
    area: Rect,
    projection: &StorageBucketDetailProjection<'_>,
    compact: bool,
    body: Option<&DetailBody>,
    selected_line: Option<usize>,
) {
    let target_name = projection.target_name.as_str();
    let mut lines = detail_header_lines(
        format!("Cloud Storage bucket / {target_name}"),
        tab_line(
            StorageDetailTab::ALL
                .iter()
                .map(|tab| (*tab == projection.active_tab, tab.label())),
        ),
        area,
        compact,
    );
    let header_len = lines.len();
    let mut selected = None;
    if let Some(error) = projection.error {
        lines.push(error_state(format!("Detail load failed for {target_name}")));
        lines.push(error_state(error));
        lines.push(error_state(
            "Press r to refresh the list or Esc/q to return to the list.",
        ));
    } else if let Some(body) = body {
        selected = selected_line;
        push_body_lines(&mut lines, body, selected, area);
    } else {
        lines.push(loading_line(fit_text(
            &format!("Loading detail for {target_name}..."),
            detail_width(area),
        )));
    }

    let scroll =
        detail_scroll_for_selection(projection.scroll, selected, header_len, lines.len(), area);
    let paragraph = Paragraph::new(lines)
        .block(bordered_block("Cloud Storage bucket detail"))
        .scroll((scroll, 0));
    frame.render_widget(paragraph, area);
}

fn render_storage_object_detail(
    frame: &mut Frame<'_>,
    area: Rect,
    projection: &StorageObjectDetailProjection<'_>,
    compact: bool,
    body: Option<&DetailBody>,
    selected_line: Option<usize>,
) {
    let target_name = projection.target_name.as_str();
    let mut lines = detail_header_lines(
        format!("Cloud Storage object / {target_name}"),
        tab_line(
            StorageDetailTab::ALL
                .iter()
                .map(|tab| (*tab == projection.active_tab, tab.label())),
        ),
        area,
        compact,
    );
    let header_len = lines.len();
    let mut selected = None;
    if let Some(error) = projection.error {
        lines.push(error_state(format!("Detail load failed for {target_name}")));
        lines.push(error_state(error));
        lines.push(error_state(
            "Press r to refresh the list or Esc/q to return to the list.",
        ));
    } else if let Some(body) = body {
        selected = selected_line;
        push_body_lines(&mut lines, body, selected, area);
    } else {
        lines.push(loading_line(fit_text(
            &format!("Loading detail for {target_name}..."),
            detail_width(area),
        )));
    }

    let scroll =
        detail_scroll_for_selection(projection.scroll, selected, header_len, lines.len(), area);
    let paragraph = Paragraph::new(lines)
        .block(bordered_block("Cloud Storage object detail"))
        .scroll((scroll, 0));
    frame.render_widget(paragraph, area);
}

fn render_cloud_run_detail(
    frame: &mut Frame<'_>,
    area: Rect,
    projection: &CloudRunDetailProjection<'_>,
    compact: bool,
    body: Option<&DetailBody>,
    selected_line: Option<usize>,
) {
    let target_name = projection.target_name.as_str();
    let mut lines = detail_header_lines(
        format!("Cloud Run / {target_name}"),
        tab_line(
            CloudRunDetailTab::ALL
                .iter()
                .map(|tab| (*tab == projection.active_tab, tab.label())),
        ),
        area,
        compact,
    );
    let header_len = lines.len();
    let mut selected = None;
    if let Some(error) = projection.error {
        lines.push(error_state(format!("Detail load failed for {target_name}")));
        lines.push(error_state(error));
        lines.push(error_state(
            "Press r to refresh the list or Esc/q to return to the list.",
        ));
    } else if let Some(body) = body {
        selected = selected_line;
        push_body_lines(&mut lines, body, selected, area);
    } else {
        lines.push(loading_line(fit_text(
            &format!("Loading detail for {target_name}..."),
            detail_width(area),
        )));
    }

    let scroll =
        detail_scroll_for_selection(projection.scroll, selected, header_len, lines.len(), area);
    let paragraph = Paragraph::new(lines)
        .block(bordered_block("Cloud Run service detail"))
        .scroll((scroll, 0));
    frame.render_widget(paragraph, area);
}

/// The secret detail pane. Metadata only — see
/// `tests/secrets_no_payload_access_test.rs` for why there is nothing else it
/// could show.
fn render_secret_detail(
    frame: &mut Frame<'_>,
    area: Rect,
    projection: &SecretDetailProjection<'_>,
    compact: bool,
    body: Option<&DetailBody>,
    selected_line: Option<usize>,
) {
    let target_name = projection.target_name.as_str();
    let mut lines = detail_header_lines(
        format!("Secret Manager / {target_name}"),
        tab_line(
            SecretDetailTab::ALL
                .iter()
                .map(|tab| (*tab == projection.active_tab, tab.label())),
        ),
        area,
        compact,
    );
    let header_len = lines.len();
    let mut selected = None;
    if let Some(error) = projection.error {
        lines.push(error_state(format!("Detail load failed for {target_name}")));
        lines.push(error_state(error));
        lines.push(error_state(
            "Press r to refresh the list or Esc/q to return to the list.",
        ));
    } else if let Some(body) = body {
        selected = selected_line;
        push_body_lines(&mut lines, body, selected, area);
    } else {
        lines.push(loading_line(fit_text(
            &format!("Loading detail for {target_name}..."),
            detail_width(area),
        )));
    }

    let scroll =
        detail_scroll_for_selection(projection.scroll, selected, header_len, lines.len(), area);
    let paragraph = Paragraph::new(lines)
        .block(bordered_block("Secret detail"))
        .scroll((scroll, 0));
    frame.render_widget(paragraph, area);
}

fn render_cloudsql_detail(
    frame: &mut Frame<'_>,
    area: Rect,
    projection: &CloudSqlDetailProjection<'_>,
    compact: bool,
    body: Option<&DetailBody>,
    selected_line: Option<usize>,
) {
    let target_name = projection.target_name.as_str();
    let mut lines = detail_header_lines(
        format!("Cloud SQL / {target_name}"),
        tab_line(
            CloudSqlDetailTab::ALL
                .iter()
                .map(|tab| (*tab == projection.active_tab, tab.label())),
        ),
        area,
        compact,
    );
    let header_len = lines.len();
    let mut selected = None;
    if let Some(error) = projection.error {
        lines.push(error_state(format!("Detail load failed for {target_name}")));
        lines.push(error_state(error));
        lines.push(error_state(
            "Press r to refresh the list or Esc/q to return to the list.",
        ));
    } else if let Some(body) = body {
        selected = selected_line;
        push_body_lines(&mut lines, body, selected, area);
    } else {
        lines.push(loading_line(fit_text(
            &format!("Loading detail for {target_name}..."),
            detail_width(area),
        )));
    }

    let scroll =
        detail_scroll_for_selection(projection.scroll, selected, header_len, lines.len(), area);
    let paragraph = Paragraph::new(lines)
        .block(bordered_block("Cloud SQL instance detail"))
        .scroll((scroll, 0));
    frame.render_widget(paragraph, area);
}

fn render_cloud_run_revision_detail(
    frame: &mut Frame<'_>,
    area: Rect,
    projection: &CloudRunRevisionProjection<'_>,
    compact: bool,
    body: Option<&DetailBody>,
    selected_line: Option<usize>,
) {
    let target_name = projection.target_name.as_str();
    let mut lines = detail_header_lines(
        format!("Cloud Run revision / {target_name}"),
        tab_line(
            CloudRunRevisionTab::ALL
                .iter()
                .map(|tab| (*tab == projection.active_tab, tab.label())),
        ),
        area,
        compact,
    );
    let header_len = lines.len();
    let mut selected = None;
    if let Some(error) = projection.error {
        lines.push(error_state(format!("Detail load failed for {target_name}")));
        lines.push(error_state(error));
        lines.push(error_state(
            "Press r to refresh or Esc/q to return to the service detail.",
        ));
    } else if let Some(body) = body {
        selected = selected_line;
        push_body_lines(&mut lines, body, selected, area);
    } else {
        lines.push(loading_line(fit_text(
            &format!("Loading detail for {target_name}..."),
            detail_width(area),
        )));
    }

    let scroll =
        detail_scroll_for_selection(projection.scroll, selected, header_len, lines.len(), area);
    let paragraph = Paragraph::new(lines)
        .block(bordered_block("Cloud Run revision detail"))
        .scroll((scroll, 0));
    frame.render_widget(paragraph, area);
}

fn detail_width(area: Rect) -> usize {
    usize::from(area.width.saturating_sub(2))
}

fn detail_scroll(requested: u16, line_count: usize, area: Rect) -> u16 {
    let visible_lines = usize::from(area.height.saturating_sub(2));
    requested.min(
        line_count
            .saturating_sub(visible_lines)
            .try_into()
            .unwrap_or(u16::MAX),
    )
}

/// The detail pane's key hint. The field keys sit before the Esc hint so they
/// survive first when a narrow pane clips the line.
const DETAIL_HEADER_HINT: &str = "[ and ] tabs   n/p field   y copy   Esc/q return to list";

/// The index, into the body's `lines`, of the field the cursor selects — the
/// line the pane highlights and the one `y` copies.
///
/// The cursor indexes the `fields()` *vector*, not `lines`, so it is resolved
/// through `fields()` here, against the very body being drawn. A stale cursor
/// falls back to the last field, which is exactly how
/// `reducer::resources::selected_detail_field` clamps it: the highlighted line
/// and the copied line must never disagree.
fn selected_detail_line(resources: &ResourcesState, body: &DetailBody) -> Option<usize> {
    let index = resources.detail_field_index()?;
    let fields = body.fields();
    let (line, _) = fields.get(index).or_else(|| fields.last())?;
    Some(*line)
}

/// The scroll offset to render with: the operator's own scroll, nudged just
/// far enough to keep the selected field on screen. `selected_line` indexes
/// the body, so it is offset by `header_len` to reach its rendered row. With
/// no selection this is the plain `detail_scroll` clamp.
///
/// Pure by design: ensure-visible is computed per frame and never written back
/// to `detail_scroll` in state, so scrolling away from a selection and back
/// still lands where the operator left it.
fn detail_scroll_for_selection(
    requested: u16,
    selected_line: Option<usize>,
    header_len: usize,
    line_count: usize,
    area: Rect,
) -> u16 {
    let clamped = detail_scroll(requested, line_count, area);
    let Some(selected_line) = selected_line else {
        return clamped;
    };
    let visible_lines = usize::from(area.height.saturating_sub(2));
    if visible_lines == 0 {
        return clamped;
    }

    let row = header_len.saturating_add(selected_line);
    let top = usize::from(clamped);
    if row < top {
        return row.try_into().unwrap_or(u16::MAX);
    }
    if row >= top.saturating_add(visible_lines) {
        return (row + 1 - visible_lines).try_into().unwrap_or(u16::MAX);
    }
    clamped
}

/// Appends a detail body's display lines, drawing the selected field's line in
/// the same selection style the resource tables use so the operator can see
/// which value `y` would copy.
fn push_body_lines(
    lines: &mut Vec<Line<'static>>,
    body: &DetailBody,
    selected_line: Option<usize>,
    area: Rect,
) {
    for (index, text) in body.display_lines().into_iter().enumerate() {
        let text = fit_text(&text, detail_width(area));
        if Some(index) == selected_line {
            lines.push(Line::styled(text, theme::selected_row()));
        } else {
            lines.push(Line::from(text));
        }
    }
}

fn detail_header_lines(
    title: String,
    tabs: ratatui::text::Line<'static>,
    area: Rect,
    compact: bool,
) -> Vec<ratatui::text::Line<'static>> {
    // Fitted, not handed to the widget whole. A bare `Line` is clipped at the
    // pane edge with nothing to say it was, so the operator read
    // `[Summary] | Traffic | Revisions | Configurati` — a fourth tab they
    // cannot name and a fifth they cannot see — and `Esc/q ret`. The
    // hint's own doc comment already ordered its parts so the useful half
    // "survives first when a narrow pane clips the line"; what was missing is
    // the marker saying the rest was cut.
    //
    // The tab bar goes through `fit_spans` rather than `fit_text` because the
    // active tab is bracketed *and* styled: flattening the bar to fit it would
    // paint every tab in one colour at exactly the widths where the bar runs
    // off the pane.
    let width = usize::from(area.width.saturating_sub(2));
    let tabs = fit_spans(tabs.spans, width);

    if (compact && area.height.saturating_sub(2) <= 5) || area.height.saturating_sub(2) <= 4 {
        return vec![tabs];
    }

    vec![
        ratatui::text::Line::from(fit_text(&title, width)),
        tabs,
        ratatui::text::Line::from(fit_text(DETAIL_HEADER_HINT, width)),
        ratatui::text::Line::from(String::new()),
    ]
}

fn render_list_with_optional_header(
    frame: &mut Frame<'_>,
    area: Rect,
    list: List<'_>,
    list_tools: Option<String>,
    selected: Option<usize>,
) {
    let mut state = ListState::default();
    state.select(selected);
    let mut header_lines = Vec::new();
    if let Some(list_tools) = list_tools {
        header_lines.push(list_tools);
    }

    if header_lines.is_empty() {
        frame.render_stateful_widget(list, area, &mut state);
    } else {
        let areas = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(header_lines.len() as u16),
                Constraint::Min(0),
            ])
            .split(area);
        // Fitted per line for the reason `render_resource_table` fits its own
        // tools row: this header carries the same line, and clipping
        // it silently is the same defect on the list-shaped services.
        let header = header_lines
            .iter()
            .map(|line| fit_text(line, usize::from(areas[0].width)))
            .collect::<Vec<_>>()
            .join("\n");
        frame.render_widget(Paragraph::new(header), areas[0]);
        frame.render_stateful_widget(list, areas[1], &mut state);
    }
}

/// Builds the display lines for a batch action confirmation.
///
/// SAFETY: this lists EVERY eligible target by name together with
/// the API call that will be made for it (`ResourceCall::preview`), then every
/// skipped target
/// with its reason. Marks can be hidden by the active filter, so the operator
/// must be shown the concrete full list — it is never summarised or truncated
/// here (the modal scrolls when it overflows).
pub fn resource_batch_confirmation_lines(
    confirmation: &crate::app::state::resources::BatchActionConfirmation,
) -> Vec<String> {
    use crate::app::state::resources::BatchAck;

    let context = confirmation.context();
    let configuration = context.configuration.to_string();
    let project = context.project.as_deref().unwrap_or("(none)");
    let eligible = confirmation.eligible();
    let skipped = confirmation.skipped();

    let mut lines = vec![
        confirmation.action().title(),
        String::new(),
        format!("Configuration: {configuration}"),
        format!("Project: {project}"),
        format!("Action: {}", confirmation.action().label()),
        format!("Targets: {} ({} skipped)", eligible.len(), skipped.len()),
        String::new(),
    ];

    for target in eligible {
        lines.push(format!("{}: {}", target.name, target.call.preview()));
    }

    if !skipped.is_empty() {
        lines.push(String::new());
        lines.push("Skipped:".to_string());
        for (target, reason) in skipped {
            lines.push(format!(
                "{} \u{2014} {}",
                target.name,
                skip_reason_label(reason)
            ));
        }
    }

    lines.push(String::new());
    lines.push(match confirmation.ack() {
        BatchAck::CountRequired { .. } => format!("Type {} to confirm", eligible.len()),
        BatchAck::Simple => "Enter to confirm".to_string(),
    });
    lines.push("Esc cancel".to_string());
    lines
}

/// Maximum number of per-target lines rendered by
/// `resource_batch_progress_lines` before the remainder is summarised as a
/// `+N more` line. Unlike the batch confirmation (which must list every
/// target so the operator can verify the exact set of calls before
/// committing), this is a live progress glance, so a long target list is
/// capped for readability instead of scrolled.
const MAX_BATCH_PROGRESS_TARGET_LINES: usize = 20;

/// Builds the display lines for an in-flight batch action's progress: an
/// aggregate header (`<ok>/<total> done · <fail> failed · <pending> pending`)
/// followed by one line per target with color-independent status text, so the
/// state is legible even without ANSI color support.
pub fn resource_batch_progress_lines(
    workflow: &crate::app::state::resources::BatchWorkflow,
) -> Vec<String> {
    use crate::app::state::resources::BatchTargetStatus;

    let targets = workflow.targets();
    let total = targets.len();
    let ok = workflow.ok_count();
    let fail = workflow.fail_count();
    let pending = workflow.pending_count();

    let mut lines = vec![
        format!(
            "Batch {} \u{2014} {ok}/{total} done \u{b7} {fail} failed \u{b7} {pending} pending",
            workflow.action().label()
        ),
        String::new(),
    ];

    for target in targets.iter().take(MAX_BATCH_PROGRESS_TARGET_LINES) {
        let status = match &target.status {
            BatchTargetStatus::Pending => "PENDING".to_string(),
            BatchTargetStatus::Succeeded => "OK".to_string(),
            BatchTargetStatus::Failed(reason) => format!("FAIL: {reason}"),
        };
        lines.push(format!("{}  {status}", target.name));
    }

    if total > MAX_BATCH_PROGRESS_TARGET_LINES {
        lines.push(format!("+{} more", total - MAX_BATCH_PROGRESS_TARGET_LINES));
    }

    lines
}

fn skip_reason_label(reason: &crate::app::state::resources::SkipReason) -> String {
    use crate::app::state::resources::SkipReason;
    match reason {
        SkipReason::Ineligible(status_label) => {
            format!("ineligible ({status_label})")
        }
        SkipReason::RequiresHighAvailability => {
            "ineligible (requires an HA (regional) instance)".to_string()
        }
    }
}

/// Footer of the jump-picker overlay: the modal's own keys, since none of them
/// are the ones the resource list behind it teaches.
const JUMP_PICKER_FOOTER: &str = "j/k select   Enter jump   Esc cancel";

/// Renders the related-resource jump picker (`J`).
///
/// The same shape as the configuration context picker, minus everything that
/// picker needs for loading over the network: these rows are derived from state
/// that is already on screen, so there is no loading, empty, degraded or
/// filtered state to render — only offers, a cursor, and the way out.
pub fn render_jump_picker_overlay(frame: &mut Frame<'_>, session: &RuntimeSession) {
    let Some(picker) = session.app().root.jump_picker.as_ref() else {
        return;
    };
    let area = jump_picker_area(frame.area(), picker.rows().len());
    let inner_width = usize::from(area.width.saturating_sub(2));
    let lines = if jump_picker_chrome_fits(area) {
        let window = viewport_window(
            picker.rows().len(),
            picker.selected_index(),
            jump_picker_row_capacity(area),
        );
        let start = window.start;
        let mut lines: Vec<String> = picker.rows()[window]
            .iter()
            .enumerate()
            .map(|(offset, row)| {
                let prefix = if start + offset == picker.selected_index() {
                    ">"
                } else {
                    " "
                };
                format!(
                    "{prefix} {}",
                    fit_text(
                        &sanitize_display_text(&row.label),
                        inner_width.saturating_sub(2)
                    )
                )
            })
            .collect();
        lines.push(String::new());
        lines.push(fit_text(JUMP_PICKER_FOOTER, inner_width));
        lines
    } else {
        vec![
            fit_text("Jump picker needs more room", inner_width),
            fit_text("Resize to choose a related resource", inner_width),
        ]
    };
    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(lines.join("\n")).block(modal_block("related jumps")),
        area,
    );
}

/// The overlay's rectangle: 60% of the width, but only as tall as the offers
/// need.
///
/// A percentage height would cover a third of the screen to hold two rows,
/// hiding the very list the jumps are about. The height is clamped to the frame
/// so a tiny terminal still yields a drawable (if too small) rect, which
/// [`jump_picker_chrome_fits`] then rejects in favour of the resize message.
/// When the clamp bites — more configured rules than the frame has rows — the
/// list scrolls inside the rect; see [`jump_picker_row_capacity`].
fn jump_picker_area(frame: Rect, rows: usize) -> Rect {
    // offers + blank + footer + both borders.
    let wanted = u16::try_from(rows).unwrap_or(u16::MAX).saturating_add(4);
    let height = wanted.min(frame.height);
    // Widened before the multiply: `width * 60` leaves `u16` past 1092 columns,
    // and an ultrawide terminal must not be the one input that panics the
    // renderer. The quotient is at most `frame.width`, so it fits again.
    let width = u16::try_from(u32::from(frame.width) * 60 / 100)
        .unwrap_or(frame.width)
        .max(1);
    Rect {
        x: frame.x + (frame.width.saturating_sub(width)) / 2,
        y: frame.y + (frame.height.saturating_sub(height)) / 2,
        width,
        height,
    }
}

/// Whether the overlay can show *an* offer plus its footer.
///
/// Deliberately no row count: the list scrolls, exactly as the configuration
/// context picker's does, so the number of configured jumps cannot decide
/// whether the modal is drawable. Jump rules are user-supplied and uncapped, and
/// a fit test coupled to their count turns a long `[[jumps]]` section into a
/// picker that can never open. The fallback message is for a terminal with no
/// room for a single row, not for a long list.
fn jump_picker_chrome_fits(area: Rect) -> bool {
    if area.width < 3 || area.height < 3 {
        return false;
    }
    let inner_width = usize::from(area.width - 2);
    // one row + a blank line + the footer.
    usize::from(area.height - 2) >= 3 && JUMP_PICKER_FOOTER.chars().count() <= inner_width
}

/// How many offers the overlay paints at once — the scrolling window's size.
///
/// The blank line and the footer come out of the inner height;
/// [`jump_picker_chrome_fits`] guarantees at least one row is left, so a
/// fitting picker is never an empty box.
fn jump_picker_row_capacity(area: Rect) -> usize {
    usize::from(area.height.saturating_sub(2)).saturating_sub(2)
}
fn render_pubsub_topics_list(
    frame: &mut Frame<'_>,
    area: Rect,
    projection: &ResourceWorkspaceProjection<'_>,
    compact: bool,
) {
    if let Some(model) = projection.table.as_ref()
        && !model.rows.is_empty()
    {
        render_resource_table(frame, area, projection, model, "Resources / Pub/Sub topics");
        return;
    }

    let items = if projection.status == ResourceWorkspaceStatus::Loading {
        loading_items(projection.load_progress.as_ref(), "topics")
    } else if projection.status == ResourceWorkspaceStatus::Failed {
        failed_items(projection.load_error.as_deref(), "topics")
    } else if projection.status == ResourceWorkspaceStatus::Stale {
        vec![stale_state("Resource rows are stale. Press r to refresh.")]
    } else if projection.total_rows == 0 && projection.status == ResourceWorkspaceStatus::Ready {
        let name = projection
            .context_label
            .as_deref()
            .unwrap_or("active configuration");
        vec![empty_state(format!("No Pub/Sub topics found for {name}"))]
    } else if projection.total_rows == 0 {
        vec![loading_state("Press r to load Pub/Sub topics.")]
    } else if projection.visible_rows == 0 {
        vec![empty_state(format!(
            "No topics match filter \"{}\"",
            projection.filter.applied
        ))]
    } else {
        unreachable!("non-empty table rows should have been rendered above")
    };
    let list = List::new(items)
        .block(bordered_block(compact_list_title(
            "Resources / Pub/Sub topics",
            projection,
            compact,
        )))
        .highlight_symbol("");
    render_list_with_optional_header(
        frame,
        area,
        list,
        (!compact).then(|| list_tools_line(projection)).flatten(),
        projection.selected_visible,
    );
}

fn render_vpc_networks_list(
    frame: &mut Frame<'_>,
    area: Rect,
    projection: &ResourceWorkspaceProjection<'_>,
    compact: bool,
) {
    if let Some(model) = projection.table.as_ref()
        && !model.rows.is_empty()
    {
        render_resource_table(frame, area, projection, model, "Resources / VPC networks");
        return;
    }

    let items = if projection.status == ResourceWorkspaceStatus::Loading {
        loading_items(projection.load_progress.as_ref(), "networks")
    } else if projection.status == ResourceWorkspaceStatus::Failed {
        failed_items(projection.load_error.as_deref(), "networks")
    } else if projection.status == ResourceWorkspaceStatus::Stale {
        vec![stale_state("Resource rows are stale. Press r to refresh.")]
    } else if projection.total_rows == 0 && projection.status == ResourceWorkspaceStatus::Ready {
        let name = projection
            .context_label
            .as_deref()
            .unwrap_or("active configuration");
        vec![empty_state(format!("No VPC networks found for {name}"))]
    } else if projection.total_rows == 0 {
        vec![loading_state("Press r to load VPC networks.")]
    } else if projection.visible_rows == 0 {
        vec![empty_state(format!(
            "No networks match filter \"{}\"",
            projection.filter.applied
        ))]
    } else {
        unreachable!("non-empty table rows should have been rendered above")
    };
    let list = List::new(items)
        .block(bordered_block(compact_list_title(
            "Resources / VPC networks",
            projection,
            compact,
        )))
        .highlight_symbol("");
    render_list_with_optional_header(
        frame,
        area,
        list,
        (!compact).then(|| list_tools_line(projection)).flatten(),
        projection.selected_visible,
    );
}

fn render_firewall_rules_list(
    frame: &mut Frame<'_>,
    area: Rect,
    projection: &ResourceWorkspaceProjection<'_>,
    compact: bool,
) {
    if let Some(model) = projection.table.as_ref()
        && !model.rows.is_empty()
    {
        render_resource_table(frame, area, projection, model, "Resources / Firewall rules");
        return;
    }

    let items = if projection.status == ResourceWorkspaceStatus::Loading {
        loading_items(projection.load_progress.as_ref(), "firewall rules")
    } else if projection.status == ResourceWorkspaceStatus::Failed {
        failed_items(projection.load_error.as_deref(), "firewall rules")
    } else if projection.status == ResourceWorkspaceStatus::Stale {
        vec![stale_state("Resource rows are stale. Press r to refresh.")]
    } else if projection.total_rows == 0 && projection.status == ResourceWorkspaceStatus::Ready {
        let name = projection
            .context_label
            .as_deref()
            .unwrap_or("active configuration");
        vec![empty_state(format!("No firewall rules found for {name}"))]
    } else if projection.total_rows == 0 {
        vec![loading_state("Press r to load firewall rules.")]
    } else if projection.visible_rows == 0 {
        vec![empty_state(format!(
            "No firewall rules match filter \"{}\"",
            projection.filter.applied
        ))]
    } else {
        unreachable!("non-empty table rows should have been rendered above")
    };
    let list = List::new(items)
        .block(bordered_block(compact_list_title(
            "Resources / Firewall rules",
            projection,
            compact,
        )))
        .highlight_symbol("");
    render_list_with_optional_header(
        frame,
        area,
        list,
        (!compact).then(|| list_tools_line(projection)).flatten(),
        projection.selected_visible,
    );
}

fn render_vpc_subnets_list(
    frame: &mut Frame<'_>,
    area: Rect,
    projection: &ResourceWorkspaceProjection<'_>,
    compact: bool,
) {
    if let Some(model) = projection.table.as_ref()
        && !model.rows.is_empty()
    {
        render_resource_table(frame, area, projection, model, "Resources / VPC subnets");
        return;
    }

    let items = if projection.status == ResourceWorkspaceStatus::Loading {
        loading_items(projection.load_progress.as_ref(), "subnets")
    } else if projection.status == ResourceWorkspaceStatus::Failed {
        failed_items(projection.load_error.as_deref(), "subnets")
    } else if projection.status == ResourceWorkspaceStatus::Stale {
        vec![stale_state("Resource rows are stale. Press r to refresh.")]
    } else if projection.total_rows == 0 && projection.status == ResourceWorkspaceStatus::Ready {
        let name = projection
            .context_label
            .as_deref()
            .unwrap_or("active configuration");
        vec![empty_state(format!("No VPC subnets found for {name}"))]
    } else if projection.total_rows == 0 {
        vec![loading_state("Press r to load VPC subnets.")]
    } else if projection.visible_rows == 0 {
        vec![empty_state(format!(
            "No subnets match filter \"{}\"",
            projection.filter.applied
        ))]
    } else {
        unreachable!("non-empty table rows should have been rendered above")
    };
    let list = List::new(items)
        .block(bordered_block(compact_list_title(
            "Resources / VPC subnets",
            projection,
            compact,
        )))
        .highlight_symbol("");
    render_list_with_optional_header(
        frame,
        area,
        list,
        (!compact).then(|| list_tools_line(projection)).flatten(),
        projection.selected_visible,
    );
}

fn render_pubsub_subscriptions_list(
    frame: &mut Frame<'_>,
    area: Rect,
    projection: &ResourceWorkspaceProjection<'_>,
    compact: bool,
) {
    if let Some(model) = projection.table.as_ref()
        && !model.rows.is_empty()
    {
        render_resource_table(
            frame,
            area,
            projection,
            model,
            "Resources / Pub/Sub subscriptions",
        );
        return;
    }

    let items = if projection.status == ResourceWorkspaceStatus::Loading {
        loading_items(projection.load_progress.as_ref(), "subscriptions")
    } else if projection.status == ResourceWorkspaceStatus::Failed {
        failed_items(projection.load_error.as_deref(), "subscriptions")
    } else if projection.status == ResourceWorkspaceStatus::Stale {
        vec![stale_state("Resource rows are stale. Press r to refresh.")]
    } else if projection.total_rows == 0 && projection.status == ResourceWorkspaceStatus::Ready {
        let name = projection
            .context_label
            .as_deref()
            .unwrap_or("active configuration");
        vec![empty_state(format!(
            "No Pub/Sub subscriptions found for {name}"
        ))]
    } else if projection.total_rows == 0 {
        vec![loading_state("Press r to load Pub/Sub subscriptions.")]
    } else if projection.visible_rows == 0 {
        vec![empty_state(format!(
            "No subscriptions match filter \"{}\"",
            projection.filter.applied
        ))]
    } else {
        unreachable!("non-empty table rows should have been rendered above")
    };
    let list = List::new(items)
        .block(bordered_block(compact_list_title(
            "Resources / Pub/Sub subscriptions",
            projection,
            compact,
        )))
        .highlight_symbol("");
    render_list_with_optional_header(
        frame,
        area,
        list,
        (!compact).then(|| list_tools_line(projection)).flatten(),
        projection.selected_visible,
    );
}

/// BigQuery paints its lists behind one service kind, and the view decides
/// which — the same routing `render_storage_list` does for the object browser,
/// and for the same reason: a drill-down is reachable only from a row of the
/// list above it, so it is a view rather than a service.
fn render_bigquery_list(
    frame: &mut Frame<'_>,
    area: Rect,
    session: &RuntimeSession,
    projection: &ResourceWorkspaceProjection<'_>,
    compact: bool,
) {
    let resources = &session.app().root.resources;
    match resources.bigquery_view() {
        BigQueryView::Datasets => render_bigquery_datasets_list(frame, area, projection, compact),
        BigQueryView::Tables => {
            render_bigquery_tables_list(frame, area, resources, projection, compact)
        }
        BigQueryView::Schema => {
            render_bigquery_schema_list(frame, area, resources, projection, compact)
        }
    }
}

/// The columns of the entered table.
///
/// Four empty branches, the tables list's four and for its reasons: entering a
/// table is what issues the `bq show`, and the drill scope is dropped when the
/// view is left, so there is neither a never-loaded state nor a stale one.
///
/// The zero-column branch is not an error. `a_schema_of_no_fields_is_an_empty_
/// level_not_a_failure` pins that a table whose `show` carried no schema
/// arrives here as an empty level: the call succeeded and the answer was that
/// there is nothing to show.
fn render_bigquery_schema_list(
    frame: &mut Frame<'_>,
    area: Rect,
    resources: &ResourcesState,
    projection: &ResourceWorkspaceProjection<'_>,
    compact: bool,
) {
    // The table the columns belong to, qualified by its dataset the way the
    // tables view names its dataset and the object browser names its bucket
    // and prefix. `bigquery_schema_table` is `Some` for as long as this view is
    // on screen (see `set_view`), so the fallback is unreachable rather than a
    // state with a rendering.
    let table_name = resources
        .bigquery_schema_table()
        .map(|table| {
            format!(
                "{}.{}",
                sanitize_display_text(&table.dataset_id),
                sanitize_display_text(&table.table_id)
            )
        })
        .unwrap_or_else(|| "table".to_string());
    let title = format!("Resources / BigQuery {table_name}");

    if let Some(model) = projection.table.as_ref()
        && !model.rows.is_empty()
    {
        render_resource_table(frame, area, projection, model, title);
        return;
    }

    let items = if let Some(problem) = resources.bigquery_schema_problem() {
        // `r` re-issues the `show` for the table the view is inside, so the
        // retry this offers is a retry of *this* call — see the
        // `BigQueryDatasets` arm of `ResourcesAction::Refresh`.
        failed_items(Some(problem.message.as_str()), "columns")
    } else if resources.bigquery_schema_loading() {
        // A plain line rather than `loading_items`, for the reason the tables
        // list uses one: `load_progress` ages the *datasets* collection's load
        // phase, and the schema read has no phase of its own, so the helper
        // would paint a spinner frozen on its first frame beside a counter
        // permanently reading `(0s elapsed)`.
        vec![loading_state("Loading columns…".to_string())]
    } else if projection.total_rows == 0 {
        vec![empty_state(format!("No columns in {table_name}"))]
    } else if projection.visible_rows == 0 {
        vec![empty_state(format!(
            "No columns match filter \"{}\"",
            projection.filter.applied
        ))]
    } else {
        unreachable!("non-empty table rows should have been rendered above")
    };
    let list = List::new(items)
        .block(bordered_block(storage_compact_title(
            title, projection, compact,
        )))
        .highlight_symbol("");
    render_list_with_optional_header(
        frame,
        area,
        list,
        (!compact).then(|| list_tools_line(projection)).flatten(),
        projection.selected_visible,
    );
}

/// The tables inside the entered dataset.
///
/// Four empty branches, not the datasets list's six. There is no *stale* state
/// — the drill scope is dropped when the view is left, so tables are never
/// held past the context that fetched them — and no *never loaded* state,
/// because entering a dataset is what issues the listing: the view cannot be
/// on screen without a call in flight, landed or failed.
fn render_bigquery_tables_list(
    frame: &mut Frame<'_>,
    area: Rect,
    resources: &ResourcesState,
    projection: &ResourceWorkspaceProjection<'_>,
    compact: bool,
) {
    // The dataset the drill is inside, named in the title the way the object
    // browser names its bucket and prefix. `bigquery_tables_dataset` is `Some`
    // for as long as this view is on screen (see `set_view`), so the fallback
    // is unreachable rather than a state with a rendering.
    let dataset = resources
        .bigquery_tables_dataset()
        .map(|dataset| sanitize_display_text(&dataset.dataset_id))
        .unwrap_or_else(|| "dataset".to_string());
    let title = format!("Resources / BigQuery {dataset}");

    if let Some(model) = projection.table.as_ref()
        && !model.rows.is_empty()
    {
        render_resource_table(frame, area, projection, model, title);
        return;
    }

    let items = if let Some(problem) = resources.bigquery_tables_problem() {
        // `r` re-issues the listing for the dataset the view is inside, so the
        // retry this offers is a retry of *this* call — see the
        // `BigQueryDatasets` arm of `ResourcesAction::Refresh`.
        failed_items(Some(problem.message.as_str()), "tables")
    } else if resources.bigquery_tables_loading() {
        // A plain line rather than `loading_items`, which every other list
        // uses: that helper renders a spinner frame and an elapsed count, and
        // both come from `load_progress`, which ages the *datasets*
        // collection's load phase. The tables listing has no phase of its own,
        // so the helper's `None` fallback would paint a spinner frozen on its
        // first frame beside a counter permanently reading `(0s elapsed)` — a
        // timer that never moves is worse than no timer.
        vec![loading_state("Loading tables…".to_string())]
    } else if projection.total_rows == 0 {
        vec![empty_state(format!("No tables in {dataset}"))]
    } else if projection.visible_rows == 0 {
        vec![empty_state(format!(
            "No tables match filter \"{}\"",
            projection.filter.applied
        ))]
    } else {
        unreachable!("non-empty table rows should have been rendered above")
    };
    let list = List::new(items)
        .block(bordered_block(storage_compact_title(
            title, projection, compact,
        )))
        .highlight_symbol("");
    render_list_with_optional_header(
        frame,
        area,
        list,
        (!compact).then(|| list_tools_line(projection)).flatten(),
        projection.selected_visible,
    );
}

fn render_bigquery_datasets_list(
    frame: &mut Frame<'_>,
    area: Rect,
    projection: &ResourceWorkspaceProjection<'_>,
    compact: bool,
) {
    if let Some(model) = projection.table.as_ref()
        && !model.rows.is_empty()
    {
        render_resource_table(
            frame,
            area,
            projection,
            model,
            "Resources / BigQuery datasets",
        );
        return;
    }

    let items = if projection.status == ResourceWorkspaceStatus::Loading {
        loading_items(projection.load_progress.as_ref(), "datasets")
    } else if projection.status == ResourceWorkspaceStatus::Failed {
        failed_items(projection.load_error.as_deref(), "datasets")
    } else if projection.status == ResourceWorkspaceStatus::Stale {
        vec![stale_state("Resource rows are stale. Press r to refresh.")]
    } else if projection.total_rows == 0 && projection.status == ResourceWorkspaceStatus::Ready {
        let name = projection
            .context_label
            .as_deref()
            .unwrap_or("active configuration");
        vec![empty_state(format!(
            "No BigQuery datasets found for {name}"
        ))]
    } else if projection.total_rows == 0 {
        vec![loading_state("Press r to load BigQuery datasets.")]
    } else if projection.visible_rows == 0 {
        vec![empty_state(format!(
            "No datasets match filter \"{}\"",
            projection.filter.applied
        ))]
    } else {
        unreachable!("non-empty table rows should have been rendered above")
    };
    let list = List::new(items)
        .block(bordered_block(compact_list_title(
            "Resources / BigQuery datasets",
            projection,
            compact,
        )))
        .highlight_symbol("");
    render_list_with_optional_header(
        frame,
        area,
        list,
        (!compact).then(|| list_tools_line(projection)).flatten(),
        projection.selected_visible,
    );
}

fn render_service_accounts_list(
    frame: &mut Frame<'_>,
    area: Rect,
    projection: &ResourceWorkspaceProjection<'_>,
    compact: bool,
) {
    if let Some(model) = projection.table.as_ref()
        && !model.rows.is_empty()
    {
        render_resource_table(
            frame,
            area,
            projection,
            model,
            "Resources / Service accounts",
        );
        return;
    }

    let items = if projection.status == ResourceWorkspaceStatus::Loading {
        loading_items(projection.load_progress.as_ref(), "service accounts")
    } else if projection.status == ResourceWorkspaceStatus::Failed {
        failed_items(projection.load_error.as_deref(), "service accounts")
    } else if projection.status == ResourceWorkspaceStatus::Stale {
        vec![stale_state("Resource rows are stale. Press r to refresh.")]
    } else if projection.total_rows == 0 && projection.status == ResourceWorkspaceStatus::Ready {
        let name = projection
            .context_label
            .as_deref()
            .unwrap_or("active configuration");
        vec![empty_state(format!("No service accounts found for {name}"))]
    } else if projection.total_rows == 0 {
        vec![loading_state("Press r to load service accounts.")]
    } else if projection.visible_rows == 0 {
        vec![empty_state(format!(
            "No service accounts match filter \"{}\"",
            projection.filter.applied
        ))]
    } else {
        unreachable!("non-empty table rows should have been rendered above")
    };
    let list = List::new(items)
        .block(bordered_block(compact_list_title(
            "Resources / Service accounts",
            projection,
            compact,
        )))
        .highlight_symbol("");
    render_list_with_optional_header(
        frame,
        area,
        list,
        (!compact).then(|| list_tools_line(projection)).flatten(),
        projection.selected_visible,
    );
}

fn render_iam_policy_list(
    frame: &mut Frame<'_>,
    area: Rect,
    projection: &ResourceWorkspaceProjection<'_>,
    compact: bool,
) {
    if let Some(model) = projection.table.as_ref()
        && !model.rows.is_empty()
    {
        render_resource_table(frame, area, projection, model, "Resources / IAM policy");
        return;
    }

    let items = if projection.status == ResourceWorkspaceStatus::Loading {
        loading_items(projection.load_progress.as_ref(), "IAM policy")
    } else if projection.status == ResourceWorkspaceStatus::Failed {
        failed_items(projection.load_error.as_deref(), "IAM policy")
    } else if projection.status == ResourceWorkspaceStatus::Stale {
        vec![stale_state("Resource rows are stale. Press r to refresh.")]
    } else if projection.total_rows == 0 && projection.status == ResourceWorkspaceStatus::Ready {
        let name = projection
            .context_label
            .as_deref()
            .unwrap_or("active configuration");
        vec![empty_state(format!("No policy bindings found for {name}"))]
    } else if projection.total_rows == 0 {
        vec![loading_state("Press r to load the IAM policy.")]
    } else if projection.visible_rows == 0 {
        vec![empty_state(format!(
            "No policy bindings match filter \"{}\"",
            projection.filter.applied
        ))]
    } else {
        unreachable!("non-empty table rows should have been rendered above")
    };
    let list = List::new(items)
        .block(bordered_block(compact_list_title(
            "Resources / IAM policy",
            projection,
            compact,
        )))
        .highlight_symbol("");
    render_list_with_optional_header(
        frame,
        area,
        list,
        (!compact).then(|| list_tools_line(projection)).flatten(),
        projection.selected_visible,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::projection::detail_body::{
        REVISION_SELECT_MARKER, REVISION_UNSELECTED_MARKER, project_detail_body,
    };
    use crate::app::reducer::root::reduce;
    use crate::app::{Action, AppState, Effect, EffectResult, ResourcesAction};
    use crate::ui::shared::table::plan_table;
    use nuvem_domain::gcp::configuration::{GcloudConfiguration, GcloudConfigurationName};
    use nuvem_domain::gcp::resource::{
        ComputeInstance, ComputeInstanceDetail, ComputeKeyValue, ResourceStatus,
    };
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::style::Style;

    fn app_with_loaded_gce_instance() -> AppState {
        let mut app = AppState::new();
        reduce(
            &mut app,
            &Action::EffectResult(EffectResult::ConfigurationsLoaded {
                configurations: vec![GcloudConfiguration {
                    name: GcloudConfigurationName::new("dev"),
                    is_active: true,
                    account: None,
                    project: Some("dev-project".to_string()),
                    region: None,
                    zone: None,
                }],
            }),
        );
        let context = app.root.current_context().unwrap().clone();
        reduce(
            &mut app,
            &Action::EffectResult(EffectResult::ComputeInstancesLoaded {
                context,
                instances: vec![ComputeInstance {
                    id: "api-dev-1-id".to_string(),
                    name: "api-dev-1".to_string(),
                    zone: "europe-west1-b".to_string(),
                    status: ResourceStatus::Running,
                    machine_type: "e2-medium".to_string(),
                    internal_ip: Some("10.0.0.2".to_string()),
                    external_ip: None,
                }],
            }),
        );
        app
    }

    /// The projection layer restates the revision row marker because it must
    /// not import `src/ui` (see ARCHITECTURE.md). This pins the copy to the
    /// renderer's own symbol so the two can never drift apart.
    #[test]
    fn revision_select_marker_matches_shared_row_select_symbol() {
        assert_eq!(REVISION_SELECT_MARKER, ROW_SELECT_SYMBOL);
        // The unselected rows are padded to the marker's width by hand. If the
        // symbol ever grows, the filler has to grow with it or every
        // unselected revision row silently misaligns.
        assert_eq!(
            display_width(REVISION_UNSELECTED_MARKER),
            display_width(ROW_SELECT_SYMBOL)
        );
    }

    #[test]
    fn table_tools_line_surfaces_hidden_column_hint_when_narrow() {
        let app = app_with_loaded_gce_instance();
        let projection = project_resources(&app.root.resources);
        let model = projection
            .table
            .as_ref()
            .expect("GCE table model should be present once instances are loaded");

        let plan = plan_table(model, 40, false);
        assert!(
            plan.hidden_count > 0,
            "expected a dropped column at width 40, plan: {plan:?}"
        );

        let line = table_tools_line(&projection, model, plan.hidden_count, plan.overflow);
        assert!(
            line.contains(&format!("+{} cols (w)", plan.hidden_count)),
            "expected hidden-column hint in tools line, got: {line}"
        );
        // And `w` is taught once: the fact names it, so the hint goes.
        assert_eq!(
            line.matches("(w)").count() + line.matches("w wide").count(),
            1,
            "the tools line names `w` more than once: {line}"
        );
    }

    #[test]
    fn table_tools_line_surfaces_overflow_hint_when_wide_and_overflowing() {
        let app = app_with_loaded_gce_instance();
        let mut projection = project_resources(&app.root.resources);
        projection.wide_columns = true;
        let model = projection
            .table
            .as_ref()
            .expect("GCE table model should be present once instances are loaded");

        let plan = plan_table(model, 40, true);
        assert!(
            plan.overflow,
            "expected overflow at width 40 in wide mode, plan: {plan:?}"
        );

        let line = table_tools_line(&projection, model, plan.hidden_count, plan.overflow);
        assert!(
            line.contains("▸ more"),
            "expected overflow hint in tools line, got: {line}"
        );
    }

    /// A 12-row pane: two rows go to the border, leaving a ten-line viewport.
    fn pane() -> Rect {
        Rect::new(0, 0, 40, 12)
    }

    #[test]
    fn detail_scroll_for_selection_without_a_selection_reproduces_the_plain_clamp() {
        let area = pane();
        assert_eq!(
            detail_scroll_for_selection(5, None, 4, 40, area),
            detail_scroll(5, 40, area)
        );
        // A request past the last page is clamped exactly as before: 12 lines
        // in a ten-line viewport can scroll by at most 2.
        assert_eq!(detail_scroll_for_selection(99, None, 4, 12, area), 2);
    }

    #[test]
    fn detail_scroll_for_selection_scrolls_up_to_a_selection_above_the_viewport() {
        // Body line 0 sits on rendered row 4 behind the four header lines,
        // well above a viewport that starts at row 20.
        assert_eq!(detail_scroll_for_selection(20, Some(0), 4, 40, pane()), 4);
    }

    #[test]
    fn detail_scroll_for_selection_scrolls_down_to_a_selection_below_the_viewport() {
        // Body line 30 sits on rendered row 34; the ten-line viewport has to
        // start at 25 for that row to be its last line.
        assert_eq!(detail_scroll_for_selection(0, Some(30), 4, 40, pane()), 25);
    }

    #[test]
    fn detail_scroll_for_selection_leaves_a_visible_selection_untouched() {
        // Rendered row 14, viewport rows 5..15: already on screen.
        assert_eq!(detail_scroll_for_selection(5, Some(10), 4, 40, pane()), 5);
    }

    fn compute_instance_detail() -> ComputeInstanceDetail {
        ComputeInstanceDetail {
            name: "api-dev-1".to_string(),
            zone: "europe-west1-b".to_string(),
            status: ResourceStatus::Running,
            machine_type: "e2-medium".to_string(),
            creation_timestamp: Some("2026-07-03T10:11:12.000-07:00".to_string()),
            network_interfaces: Vec::new(),
            tags: Vec::new(),
            service_accounts: Vec::new(),
            disks: Vec::new(),
            labels: Vec::new(),
            metadata: Vec::new(),
            raw_json: "{\"name\":\"api-dev-1\"}".to_string(),
        }
    }

    /// AppState with the GCE detail pane open on a loaded instance, so its
    /// Summary tab carries copyable fields for the cursor to land on.
    fn app_with_open_compute_detail() -> AppState {
        let mut app = app_with_loaded_gce_instance();
        reduce(&mut app, &Action::Resources(ResourcesAction::OpenSelected));
        let context = app.root.current_context().unwrap().clone();
        reduce(
            &mut app,
            &Action::EffectResult(EffectResult::ComputeInstanceDetailLoaded {
                context,
                detail: compute_instance_detail(),
            }),
        );
        app
    }

    /// The value `y` would put on the clipboard right now.
    fn copied_value(app: &mut AppState) -> String {
        let effects = reduce(app, &Action::Resources(ResourcesAction::CopyResourceUri));
        match effects.as_slice() {
            [Effect::CopyToClipboard { value, .. }] => value.clone(),
            other => panic!("expected a single clipboard effect, got: {other:?}"),
        }
    }

    #[test]
    fn the_highlighted_line_is_always_the_line_y_copies() {
        let mut app = app_with_open_compute_detail();
        // One walk past the end of the field list, so the wrap is covered too.
        for step in 0..7 {
            reduce(
                &mut app,
                &Action::Resources(ResourcesAction::SelectNextDetailField),
            );
            let body = project_detail_body(&app.root.resources).expect("detail body on screen");
            let line =
                selected_detail_line(&app.root.resources, &body).expect("a highlighted line");
            let copied = copied_value(&mut app);
            let highlighted = body.lines[line]
                .copy
                .as_ref()
                .expect("the highlighted line must carry a copy payload");
            assert_eq!(
                highlighted.value, copied,
                "step {step}: highlighted line {line} ({:?}) disagrees with what y copies",
                body.lines[line].text
            );
        }
    }

    #[test]
    fn a_stale_field_cursor_highlights_the_line_the_clamped_copy_takes() {
        let mut app = app_with_open_compute_detail();
        app.root.resources.set_detail_field_index(Some(99));

        let body = project_detail_body(&app.root.resources).expect("detail body on screen");
        let line = selected_detail_line(&app.root.resources, &body).expect("a highlighted line");
        let last = body.fields().last().expect("summary fields").0;
        assert_eq!(
            line, last,
            "a stale cursor must fall back to the last field"
        );
        assert_eq!(
            body.lines[line].copy.as_ref().expect("copy payload").value,
            copied_value(&mut app)
        );
    }

    #[test]
    fn no_cursor_highlights_nothing() {
        let app = app_with_open_compute_detail();
        let body = project_detail_body(&app.root.resources).expect("detail body on screen");
        assert_eq!(selected_detail_line(&app.root.resources, &body), None);
    }

    /// Whether a rendered cell carries every attribute of `expected`, which is
    /// how a themed style shows up once the terminal has filled in the rest.
    fn carries_style(cell: Style, expected: Style) -> bool {
        (expected.fg.is_none() || cell.fg == expected.fg)
            && (expected.bg.is_none() || cell.bg == expected.bg)
            && cell.add_modifier.contains(expected.add_modifier)
    }

    /// Renders the resources workspace and returns each row as `(text, whether
    /// its first cell carries the selection style)`.
    fn rendered_rows(session: &RuntimeSession) -> Vec<(String, bool)> {
        let area = Rect::new(0, 0, 80, 24);
        let backend = TestBackend::new(area.width, area.height);
        let mut terminal = Terminal::new(backend).expect("terminal should initialize");
        terminal
            .draw(|frame| render(frame, area, session))
            .expect("detail should render");
        let buffer = terminal.backend().buffer().clone();
        (0..area.height)
            .map(|y| {
                let text: String = (0..area.width)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect::<Vec<_>>()
                    .concat();
                // Column 1 is the first cell inside the pane border, so it
                // holds the line's first character. The highlight is
                // text-only — `fit_text` never pads — so only the cells the
                // text occupies carry the style.
                let selected = carries_style(buffer[(1, y)].style(), theme::selected_row());
                (text, selected)
            })
            .collect()
    }

    #[test]
    fn the_selected_field_line_is_drawn_in_the_selection_style() {
        let mut app = app_with_open_compute_detail();
        // Summary fields: Name, Status, Zone, ... — three moves lands on Zone.
        for _ in 0..3 {
            reduce(
                &mut app,
                &Action::Resources(ResourcesAction::SelectNextDetailField),
            );
        }
        let session = RuntimeSession::new_for_test(app);

        let rows = rendered_rows(&session);
        let zone = rows
            .iter()
            .find(|(text, _)| text.contains("Zone: europe-west1-b"))
            .expect("the Zone line should be on screen");
        assert!(zone.1, "expected the selected Zone line to be highlighted");
        let name = rows
            .iter()
            .find(|(text, _)| text.contains("Name: api-dev-1"))
            .expect("the Name line should be on screen");
        assert!(
            !name.1,
            "expected only the selected line to be highlighted, but Name was too"
        );
    }

    #[test]
    fn nothing_is_highlighted_while_no_field_is_selected() {
        let session = RuntimeSession::new_for_test(app_with_open_compute_detail());
        assert!(
            rendered_rows(&session)
                .iter()
                .all(|(_, selected)| !selected),
            "no line may be highlighted before the operator selects a field"
        );
    }

    /// AppState with the GCE detail pane open on the Metadata tab of an
    /// instance carrying far more labels than the pane can show at once.
    fn app_with_long_metadata_tab() -> AppState {
        let mut app = app_with_loaded_gce_instance();
        reduce(&mut app, &Action::Resources(ResourcesAction::OpenSelected));
        let context = app.root.current_context().unwrap().clone();
        let mut detail = compute_instance_detail();
        detail.labels = (0..40)
            .map(|index| ComputeKeyValue {
                key: format!("label-{index}"),
                value: format!("value-{index}"),
            })
            .collect();
        reduce(
            &mut app,
            &Action::EffectResult(EffectResult::ComputeInstanceDetailLoaded { context, detail }),
        );
        // Summary -> Network -> Disks -> Metadata.
        for _ in 0..3 {
            reduce(&mut app, &Action::Resources(ResourcesAction::SelectNextTab));
        }
        app
    }

    #[test]
    fn a_selection_below_a_short_pane_is_scrolled_into_view() {
        // A Metadata tab far longer than the pane, so the last field can only
        // be seen if the renderer scrolls to it. Stepping back one field from
        // no selection lands on that last label.
        let mut app = app_with_long_metadata_tab();
        reduce(
            &mut app,
            &Action::Resources(ResourcesAction::SelectPreviousDetailField),
        );
        let session = RuntimeSession::new_for_test(app);

        let rows = rendered_rows(&session);
        let last = rows
            .iter()
            .find(|(text, _)| text.contains("label-39: value-39"))
            .expect("the selected last label must be scrolled into view");
        assert!(last.1, "expected the selected label line to be highlighted");
    }

    #[test]
    fn paging_a_long_tab_moves_the_viewport_even_with_a_field_selected() {
        let mut app = app_with_long_metadata_tab();
        // The first label, at the top of the tab, is what ensure-visible would
        // otherwise pin the viewport to.
        reduce(
            &mut app,
            &Action::Resources(ResourcesAction::SelectNextDetailField),
        );
        let before = rendered_rows(&RuntimeSession::new_for_test(app.clone()));
        let first_label = |rows: &[(String, bool)]| {
            rows.iter()
                .any(|(text, _)| text.contains("label-0: value-0"))
        };
        assert!(
            first_label(&before),
            "the selected first label should start on screen"
        );

        // Two pages down: the selected line is now well above the viewport.
        for _ in 0..2 {
            reduce(
                &mut app,
                &Action::Resources(ResourcesAction::ScrollDetailDown),
            );
        }
        let after = rendered_rows(&RuntimeSession::new_for_test(app));

        assert!(
            !first_label(&after),
            "paging down must move the viewport past the previously selected \
             line, got: {:?}",
            after.iter().map(|(text, _)| text).collect::<Vec<_>>()
        );
        assert!(
            after.iter().all(|(_, selected)| !selected),
            "paging releases the field cursor, so nothing stays highlighted"
        );
    }

    #[test]
    fn the_detail_header_hint_names_the_field_keys() {
        let session = RuntimeSession::new_for_test(app_with_open_compute_detail());
        let rows = rendered_rows(&session);
        assert!(
            rows.iter()
                .any(|(text, _)| text.contains(DETAIL_HEADER_HINT)),
            "expected the hint line on screen, got: {:?}",
            rows.iter().map(|(text, _)| text).collect::<Vec<_>>()
        );
        assert!(DETAIL_HEADER_HINT.contains("n/p"));
        assert!(DETAIL_HEADER_HINT.contains('y'));
    }

    /// The overlay may not paint as an open-but-empty box. Whenever the chrome
    /// is declared to fit there has to be room for at least one offer, so the
    /// two are pinned together rather than one being inferred from the other.
    #[test]
    fn a_fitting_jump_picker_always_has_room_for_at_least_one_row() {
        for height in 0..40u16 {
            for width in [0u16, 2, 3, 37, 38, 40, 80] {
                let area = Rect {
                    x: 0,
                    y: 0,
                    width,
                    height,
                };
                if jump_picker_chrome_fits(area) {
                    assert!(
                        jump_picker_row_capacity(area) >= 1,
                        "a fitting {width}x{height} picker must hold a row"
                    );
                }
            }
        }
    }

    /// A terminal too short for one row plus the chrome must say so rather than
    /// paint a box with nothing in it.
    #[test]
    fn a_terminal_too_short_for_one_row_does_not_fit() {
        for height in 0..5u16 {
            assert!(
                !jump_picker_chrome_fits(Rect {
                    x: 0,
                    y: 0,
                    width: 80,
                    height,
                }),
                "a {height}-line overlay cannot hold a row, a blank line and the footer"
            );
        }
        assert!(jump_picker_chrome_fits(Rect {
            x: 0,
            y: 0,
            width: 80,
            height: 5,
        }));
    }

    /// The footer is the widest thing the overlay paints, so a box narrower
    /// than it is refused however tall it is.
    #[test]
    fn a_jump_picker_narrower_than_its_footer_does_not_fit() {
        let needed = u16::try_from(JUMP_PICKER_FOOTER.chars().count()).unwrap() + 2;
        let area = |width| Rect {
            x: 0,
            y: 0,
            width,
            height: 24,
        };
        assert!(!jump_picker_chrome_fits(area(needed - 1)));
        assert!(jump_picker_chrome_fits(area(needed)));
    }
}
