use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::widgets::{Clear, Paragraph};

use crate::app::projection::project_configurations_table;
use crate::app::state::configurations::{
    ConfigurationActivationConfirmation, ConfigurationContextEditField,
    ConfigurationContextEditForm, ConfigurationContextPicker, ConfigurationContextPickerItem,
    ConfigurationContextPickerKind, ConfigurationContextPickerStatus, ConfigurationCreateField,
    ConfigurationCreateForm,
};
use crate::services::runtime::session::RuntimeSession;
use crate::ui::centered_rect;
use crate::ui::shared::components::{bordered_block, fit_text, modal_block};
use crate::ui::shared::picker::viewport_window;
use crate::ui::shared::table::{ROW_SELECT_SYMBOL, display_width, plan_table, render_table};

pub fn render(frame: &mut Frame<'_>, area: Rect, session: &RuntimeSession) {
    if let Some(form) = session.app().root.configurations.create_form() {
        render_create_form(frame, area, form);
        return;
    }

    if let Some(form) = session.app().root.configurations.context_edit_form() {
        render_context_edit_form(frame, area, form);
        return;
    }

    let configurations = &session.app().root.configurations;

    let areas = match list_tools_line(configurations) {
        Some(tools_line) => {
            let areas = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Length(1), Constraint::Min(0)])
                .split(area);
            frame.render_widget(
                Paragraph::new(fit_text(&tools_line, usize::from(areas[0].width))),
                areas[0],
            );
            areas[1]
        }
        None => area,
    };

    let block = bordered_block("gcloud configurations");
    let inner = block.inner(areas);
    frame.render_widget(block, areas);

    // The empty states stay paragraphs: a header over no rows says nothing the
    // message does not, and the message is the whole content of the pane.
    if let Some(message) = empty_state_message(configurations) {
        frame.render_widget(
            Paragraph::new(fit_text(&message, usize::from(inner.width))),
            inner,
        );
        return;
    }

    let model = project_configurations_table(configurations);
    // The cursor lives in ratatui's highlight gutter, exactly as it does in
    // every resource table, so its width comes out of the budget before the
    // columns are planned rather than out of the first column afterwards.
    let plan = plan_table(
        &model,
        inner.width.saturating_sub(display_width(ROW_SELECT_SYMBOL)),
        false,
    );
    render_table(frame, inner, &model, &plan);
}

/// The message that stands in for the table when there is none to draw, or
/// `None` when there are rows.
fn empty_state_message(
    configurations: &crate::app::state::configurations::ConfigurationsState,
) -> Option<String> {
    if configurations.rows().is_empty() {
        return Some("No configurations loaded. Press r to refresh.".to_string());
    }
    if configurations.visible_rows().is_empty() {
        return Some(format!(
            "No configurations match filter \"{}\"",
            configurations.filter_applied()
        ));
    }
    None
}

fn list_tools_line(
    configurations: &crate::app::state::configurations::ConfigurationsState,
) -> Option<String> {
    if configurations.rows().is_empty()
        && configurations.filter_applied().is_empty()
        && !configurations.is_filter_input_active()
    {
        return None;
    }

    if configurations.is_filter_input_active() {
        return Some(format!("Filter: {}_", configurations.filter_draft()));
    }

    let filter = if configurations.filter_applied().is_empty() {
        "none"
    } else {
        configurations.filter_applied()
    };
    Some(format!(
        "Filter: {filter}   Rows: {}/{}",
        configurations.visible_rows().len(),
        configurations.rows().len()
    ))
}

fn render_create_form(frame: &mut Frame<'_>, area: Rect, form: &ConfigurationCreateForm) {
    let lines = create_form_lines(form)
        .into_iter()
        .map(|line| fit_text(&line, usize::from(area.width.saturating_sub(2))))
        .collect::<Vec<_>>();
    let focused_line = 2 + ConfigurationCreateField::ALL
        .iter()
        .position(|field| *field == form.focused_field())
        .unwrap_or_default();
    let paragraph = Paragraph::new(lines.join("\n"))
        .block(bordered_block("create configuration"))
        .scroll((form_scroll(&lines, area, focused_line), 0));

    frame.render_widget(paragraph, area);
}

/// Marks where typing lands in the focused field of a form.
///
/// The same glyph every other input in the app uses — the resource filter, the
/// configurations filter, the service navigator, the context picker, the
/// command palette's query row — so it means here what it means there.
///
/// The forms carried none, which cost two things: an empty focused
/// field was indistinguishable from a label, and a trailing space was
/// invisible. These values become a gcloud configuration name and a project id,
/// and the form is the last place a stray one can be seen before it is
/// submitted.
const FIELD_CURSOR: &str = "_";

/// One form row: focus marker, label, value, and the cursor on the focused
/// field only. An unfocused field is not being typed into and must not claim
/// to be.
fn form_field_line(focused: bool, label: &str, value: &str) -> String {
    let prefix = if focused { "> " } else { "  " };
    let cursor = if focused { FIELD_CURSOR } else { "" };
    format!("{prefix}{label}: {value}{cursor}")
}

fn create_form_lines(form: &ConfigurationCreateForm) -> Vec<String> {
    let mut lines = vec!["Create gcloud configuration".to_string(), String::new()];
    lines.extend(ConfigurationCreateField::ALL.iter().map(|field| {
        form_field_line(
            *field == form.focused_field(),
            field.label(),
            form.field_value(*field),
        )
    }));
    lines.push(String::new());
    lines.push("Enter create   Esc cancel   Tab next field".to_string());
    lines
}

fn render_context_edit_form(
    frame: &mut Frame<'_>,
    area: Rect,
    form: &ConfigurationContextEditForm,
) {
    let lines = context_edit_form_lines(form)
        .into_iter()
        .map(|line| fit_text(&line, usize::from(area.width.saturating_sub(2))))
        .collect::<Vec<_>>();
    let focused_line = 4 + ConfigurationContextEditField::ALL
        .iter()
        .position(|field| *field == form.focused_field())
        .unwrap_or_default();
    let paragraph = Paragraph::new(lines.join("\n"))
        .block(bordered_block("edit active context"))
        .scroll((form_scroll(&lines, area, focused_line), 0));

    frame.render_widget(paragraph, area);
}

fn form_scroll(lines: &[String], area: Rect, focused_line: usize) -> u16 {
    let content_height = usize::from(area.height.saturating_sub(2));
    if content_height == 0 || lines.len() <= content_height {
        return 0;
    }
    focused_line
        .saturating_sub(content_height.saturating_sub(1))
        .try_into()
        .unwrap_or(u16::MAX)
}

fn context_edit_form_lines(form: &ConfigurationContextEditForm) -> Vec<String> {
    let mut lines = vec![
        "Edit active gcloud context".to_string(),
        String::new(),
        format!("Configuration: {}", form.target()),
        String::new(),
    ];
    lines.extend(ConfigurationContextEditField::ALL.iter().map(|field| {
        form_field_line(
            *field == form.focused_field(),
            field.label(),
            form.field_value(*field),
        )
    }));
    lines.push(String::new());
    lines.push("Enter save   Esc cancel   Tab next field   Ctrl-Space pick".to_string());
    lines
}

pub fn activation_confirmation_lines(
    confirmation: &ConfigurationActivationConfirmation,
) -> Vec<String> {
    let target = confirmation.target();
    vec![
        "Confirm configuration activation".to_string(),
        String::new(),
        format!("Target: {}", target.name),
        format!("Account: {}", target.account.as_deref().unwrap_or("-")),
        format!("Project: {}", target.project.as_deref().unwrap_or("-")),
        format!("Region: {}", target.region.as_deref().unwrap_or("-")),
        format!("Zone: {}", target.zone.as_deref().unwrap_or("-")),
        format!("Command: {}", confirmation.command_preview()),
        String::new(),
        "Enter/y confirm   Esc/n cancel".to_string(),
    ]
}

pub fn render_context_picker_overlay(frame: &mut Frame<'_>, session: &RuntimeSession) {
    let Some(picker) = session.app().root.configurations.context_picker() else {
        return;
    };
    let area = centered_rect(80, 70, frame.area());
    let lines = if picker_chrome_fits(area, picker) {
        picker_lines(picker, area)
    } else {
        picker_fallback_lines(picker, area.width)
    };
    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(lines.join("\n")).block(modal_block(format!(
            "pick {}",
            picker_kind_name(picker.kind())
        ))),
        area,
    );
}

fn picker_lines(picker: &ConfigurationContextPicker, area: Rect) -> Vec<String> {
    let plural = picker_kind_plural(picker.kind());
    let inner_width = usize::from(area.width.saturating_sub(2));
    let footer = picker_footer_lines(picker)
        .into_iter()
        .map(|line| fit_text(&line, inner_width))
        .collect::<Vec<_>>();
    match picker.status() {
        ConfigurationContextPickerStatus::Loading => {
            let mut lines = vec![format!("Loading {plural}...")];
            lines.extend(footer);
            lines
        }
        ConfigurationContextPickerStatus::Empty => {
            let mut lines = vec![format!("No {plural} found")];
            lines.extend(footer);
            lines
        }
        ConfigurationContextPickerStatus::Degraded(error) => {
            let mut lines = vec![format!("Unable to load {plural}: {error}")];
            lines.extend(footer);
            lines
        }
        ConfigurationContextPickerStatus::Ready => {
            let visible_items = picker.visible_items();
            if visible_items.is_empty() {
                let mut lines = vec![
                    format!("No {plural} match filter \"{}\"", picker.filter_draft()),
                    format!("Filter: {}", picker.filter_draft()),
                ];
                lines.extend(footer);
                return lines;
            }

            let selected_index = picker
                .selected_item()
                .and_then(|selected| visible_items.iter().position(|item| *item == selected))
                .unwrap_or(0);
            let row_capacity =
                usize::from(area.height.saturating_sub(2)).saturating_sub(1 + footer.len());
            let window = viewport_window(visible_items.len(), selected_index, row_capacity);
            let mut lines = visible_items[window]
                .iter()
                .map(|item| {
                    let prefix = if picker.selected_item() == Some(*item) {
                        ">"
                    } else {
                        " "
                    };
                    let favorite_marker = if picker.is_favorite(item) { "*" } else { " " };
                    let gutter = format!("{prefix} {favorite_marker}");
                    format!(
                        "{gutter} {}",
                        fit_text(
                            &picker_item_line(item),
                            inner_width.saturating_sub(gutter.chars().count() + 1)
                        )
                    )
                })
                .collect::<Vec<_>>();
            let filter = if picker.is_filter_input_active() {
                format!("{}_", picker.filter_draft())
            } else if picker.filter_draft().is_empty() {
                "none".to_string()
            } else {
                picker.filter_draft().to_string()
            };
            lines.push(format!("Filter: {filter}"));
            lines.extend(footer);
            lines
        }
    }
}

fn picker_footer_lines(picker: &ConfigurationContextPicker) -> Vec<String> {
    if picker.is_filter_input_active() {
        return vec![
            "Backspace clear   Enter apply".to_string(),
            "Esc cancel filter".to_string(),
        ];
    }

    if picker.status() == &ConfigurationContextPickerStatus::Ready
        && !picker.visible_items().is_empty()
    {
        return vec![
            "/ filter   j/k select   Enter choose".to_string(),
            "r retry   Esc back".to_string(),
        ];
    }

    if picker.status() == &ConfigurationContextPickerStatus::Ready
        && !picker.filter_draft().is_empty()
    {
        return vec![
            "/ filter   type/backspace".to_string(),
            "Enter apply   Esc cancel   r retry".to_string(),
        ];
    }

    vec!["/ filter   r retry".to_string(), "Esc back".to_string()]
}

fn picker_chrome_fits(area: Rect, picker: &ConfigurationContextPicker) -> bool {
    if area.width < 3 || area.height < 3 {
        return false;
    }

    let inner_width = usize::from(area.width - 2);
    let footer = picker_footer_lines(picker);
    let content_lines = match picker.status() {
        ConfigurationContextPickerStatus::Ready if picker.visible_items().is_empty() => 2,
        ConfigurationContextPickerStatus::Ready => 2,
        _ => 1,
    } + footer.len();

    usize::from(area.height - 2) >= content_lines
        && footer
            .iter()
            .all(|line| line.chars().count() <= inner_width)
}

fn picker_fallback_lines(picker: &ConfigurationContextPicker, width: u16) -> Vec<String> {
    let inner_width = usize::from(width.saturating_sub(2));
    vec![
        fit_text("Picker needs more room", inner_width),
        fit_text(
            &format!("Resize to select a {}", picker_kind_name(picker.kind())),
            inner_width,
        ),
    ]
}

fn picker_kind_name(kind: ConfigurationContextPickerKind) -> &'static str {
    match kind {
        ConfigurationContextPickerKind::Project => "project",
        ConfigurationContextPickerKind::Region => "region",
        ConfigurationContextPickerKind::Zone => "zone",
    }
}

fn picker_kind_plural(kind: ConfigurationContextPickerKind) -> &'static str {
    match kind {
        ConfigurationContextPickerKind::Project => "projects",
        ConfigurationContextPickerKind::Region => "regions",
        ConfigurationContextPickerKind::Zone => "zones",
    }
}

fn picker_item_line(item: &ConfigurationContextPickerItem) -> String {
    match item {
        ConfigurationContextPickerItem::Project(project) => format!(
            "{}  {}  {}",
            project.project_id,
            project.name,
            project.lifecycle_state.as_deref().unwrap_or("-")
        ),
        ConfigurationContextPickerItem::Region(region) => {
            format!(
                "{}  {}",
                region.name,
                region.status.as_deref().unwrap_or("-")
            )
        }
        ConfigurationContextPickerItem::Zone(zone) => format!(
            "{}  {}  {}",
            zone.name,
            zone.region.as_deref().unwrap_or("-"),
            zone.status.as_deref().unwrap_or("-")
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::reducer::root::reduce;
    use crate::app::state::configurations::ConfigurationContextPickerItem;
    use crate::app::{Action, AppState, ConfigurationsAction, EffectResult, WorkspaceAction};
    use nuvem_domain::gcp::configuration::{GcloudConfiguration, GcloudConfigurationName};
    use nuvem_domain::gcp::project::GcloudProject;
    use ratatui::layout::Rect;

    /// Row text in the context-picker overlay is produced by `fit_text(text,
    /// inner_width.saturating_sub(gutter.chars().count() + 1))` — the
    /// subtraction closes the gutter's own width and its trailing separator
    /// space, both fixed at three and one characters. No picker fixture
    /// elsewhere in the suite is long enough to exercise the shrink path (every
    /// one renders unchanged), so an off-by-one there would pass silently. This
    /// pins the shrink itself: a deliberately over-long project id must
    /// truncate to exactly the width that subtraction produces, not one
    /// character more or fewer.
    #[test]
    fn picker_row_text_truncates_to_exactly_the_available_width() {
        let mut app = AppState::new();
        reduce(
            &mut app,
            &Action::EffectResult(EffectResult::ConfigurationsLoaded {
                configurations: vec![GcloudConfiguration {
                    name: GcloudConfigurationName::new("dev"),
                    is_active: true,
                    account: Some("dev@example.com".to_string()),
                    project: Some("dev-project".to_string()),
                    region: Some("europe-west1".to_string()),
                    zone: Some("europe-west1-b".to_string()),
                }],
            }),
        );
        reduce(
            &mut app,
            &Action::Workspace(WorkspaceAction::Configurations),
        );
        reduce(
            &mut app,
            &Action::Configurations(ConfigurationsAction::StartEditActiveContext),
        );
        reduce(
            &mut app,
            &Action::Configurations(ConfigurationsAction::OpenContextPicker),
        );
        reduce(
            &mut app,
            &Action::EffectResult(EffectResult::ContextPickerLoaded {
                configuration: GcloudConfigurationName::new("dev"),
                kind: ConfigurationContextPickerKind::Project,
                items: vec![ConfigurationContextPickerItem::Project(GcloudProject {
                    project_id: "x".repeat(200),
                    name: "irrelevant".to_string(),
                    lifecycle_state: None,
                })],
            }),
        );

        let picker = app
            .root
            .configurations
            .context_picker()
            .expect("picker is open and loaded");
        let area = Rect {
            x: 0,
            y: 0,
            width: 50,
            height: 20,
        };
        let lines = picker_lines(picker, area);

        // Mirrors the subtraction under test rather than hardcoding its
        // result, so the expectation moves in lockstep with `inner_width`
        // (here, `area.width - 2`) if that ever changes.
        let inner_width = usize::from(area.width.saturating_sub(2));
        let gutter_len = 3; // "{prefix} {favorite_marker}": 1 + 1 + 1 chars.
        let expected_width = inner_width.saturating_sub(gutter_len + 1);

        let row = lines
            .iter()
            .find(|line| line.contains('x'))
            .expect("a row rendering the over-long project id");
        let text = &row[row.find('x').expect("row contains the truncated text")..];

        assert_eq!(text.chars().count(), expected_width);
        assert_eq!(
            text,
            format!("{}...", "x".repeat(expected_width - 3)),
            "row text must be exactly `inner_width`-fitted, not off by one"
        );
    }
}
