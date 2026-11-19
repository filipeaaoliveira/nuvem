//! Render-facing metadata for the shared application frame.

use chrono::{DateTime, Local};

use crate::app::projection::binding::{
    Binding, CFG_ACTIVATE, CFG_CREATE, CFG_EDIT, CONFIRM_ACCEPT, CONFIRM_ACCEPT_Y, CTX_PICK_VALUE,
    DETAIL_BACK, EDIT_BACKSPACE, FILTER_APPLY, FORM_NEXT_FIELD, FORM_SUBMIT,
    GLOBAL_COPY_DIAGNOSTIC, GLOBAL_QUIT, GLOBAL_REFRESH, HELP_CLOSE, HIST_BACK, HIST_FORWARD,
    LIST_SELECT, LOG_CLOSE, LOG_COPY_ENTRY, LOG_COPY_INSERT_ID, LOG_FILTER, LOG_OPEN, LOG_TAIL,
    LOG_TIME_RANGE, LOG_WRAP, MODAL_CANCEL, NAV_CONFIGURATIONS, NAV_FILTER, NAV_OPEN,
    NAV_RESOURCES, PALETTE_RUN, PALETTE_SELECT, PICKER_FILTER, PICKER_RETRY, PICKER_SELECT,
    PINNED_BINDINGS, PLUGIN_PREFIX_CANCEL, PREVIEW_COPY, RES_COLUMN, RES_FILTER,
    RES_LEAVE_DRILLDOWN, RES_NEXT_TAB, RES_OPEN, RES_PREV_TAB, RES_SCROLL_DETAIL, RES_SORT,
    RES_TOGGLE_FOCUS, RES_TOGGLE_SIDEBAR, RES_WIDE, TUNNEL_DISMISS, TUNNEL_KILL, display_binding,
};
use crate::app::state::configurations::{
    ConfigurationContextPicker, ConfigurationContextPickerKind,
};
use crate::app::state::navigator::NavigatorFocus;
use crate::app::state::plugins;
use crate::app::state::resources::{BigQueryView, ResourcesState, StorageView};
use crate::app::state::root::{RootState, Workspace};
use crate::app::state::system::OperationPhase;
use crate::config::hotkeys::KeyOverrides;
use nuvem_domain::gcp::resource::ResourceServiceKind;
use nuvem_domain::gcp::text::sanitize_display_text;

#[derive(Debug, Clone, PartialEq, Eq)]
/// Active gcloud context prepared for shared frame rendering.
pub struct FrameContextProjection {
    /// Active native gcloud configuration name, when available.
    pub configuration: Option<String>,
    /// Account associated with the active context, when available.
    pub account: Option<String>,
    /// Project associated with the active context, when available.
    pub project: Option<String>,
    /// Region associated with the active context, when available.
    pub region: Option<String>,
    /// Zone associated with the active context, when available.
    pub zone: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Operation status prepared for the shared frame.
pub struct FrameOperationProjection {
    /// Semantic phase displayed in the shared status area.
    pub phase: OperationPhase,
    /// Default operation label shown when no detailed message is active.
    pub label: String,
    /// Detailed status text, when available.
    pub message: Option<String>,
    /// Operation label associated with the detailed status text.
    pub message_label: Option<String>,
    /// Timestamp of the last completed refresh, if any.
    pub last_refresh_at: Option<DateTime<Local>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// A compact key/action pair for the shared frame footer.
pub struct FrameHint {
    /// Key or key combination to display (owned, because rebound keys are
    /// computed at runtime).
    pub key: String,
    /// Action label paired with the key.
    pub label: &'static str,
}

impl From<Binding> for FrameHint {
    fn from(binding: Binding) -> Self {
        FrameHint {
            key: binding.key.to_string(),
            label: binding.label,
        }
    }
}

/// Projects a slice of catalog `Binding`s (see `binding.rs`) into footer
/// hints using the default (no-override) keys.
fn from_bindings(bindings: &[Binding]) -> Vec<FrameHint> {
    from_bindings_with(bindings, &KeyOverrides::empty())
}

/// Override-aware variant of [`from_bindings`]: rebindable bindings' keys
/// reflect the active overrides, so the footer shows rebound keys.
fn from_bindings_with(bindings: &[Binding], overrides: &KeyOverrides) -> Vec<FrameHint> {
    bindings
        .iter()
        .map(|binding| {
            let display = display_binding(binding, overrides);
            FrameHint {
                key: display.key,
                label: display.label,
            }
        })
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Render-facing metadata shared by every normal application workspace.
pub struct AppFrameProjection {
    /// Application title shown in the frame header.
    pub app_title: &'static str,
    /// Current view or transient mode title.
    pub view_title: String,
    /// Navigation path shown below the header.
    pub breadcrumbs: Vec<String>,
    /// Active gcloud context metadata.
    pub context: FrameContextProjection,
    /// Active resource service, when the resources workspace is selected.
    pub service_label: Option<&'static str>,
    /// Operation, status, and refresh metadata for the frame.
    pub operation: FrameOperationProjection,
    /// Mode-aware actions rendered in the footer.
    pub footer_hints: Vec<FrameHint>,
    /// The always-available keys ([`PINNED_BINDINGS`]) rendered right-aligned
    /// in the footer, with their width reserved ahead of `footer_hints`.
    ///
    /// Empty while a mode owns the keyboard: in a filter, a form, or under any
    /// overlay, `?` and `:` are characters or do nothing, and a hint is a
    /// promise about a key.
    pub footer_pinned_hints: Vec<FrameHint>,
}

/// Projects the active gcloud context into shared frame context metadata.
pub(crate) fn project_frame_context(root: &RootState) -> FrameContextProjection {
    let context = root.current_context();
    FrameContextProjection {
        configuration: context
            .as_ref()
            .map(|value| value.configuration.to_string()),
        account: context.as_ref().and_then(|value| value.account.clone()),
        project: context.as_ref().and_then(|value| value.project.clone()),
        region: context.as_ref().and_then(|value| value.region.clone()),
        zone: context.and_then(|value| value.zone),
    }
}

/// Projects root application state into the metadata rendered by the shared
/// frame, using the default (no-override) footer keys.
pub fn project_app_frame(root: &RootState) -> AppFrameProjection {
    project_app_frame_with(root, &KeyOverrides::empty())
}

/// Override-aware variant of [`project_app_frame`]: the footer hints for
/// rebindable actions reflect the active hotkey overrides.
pub fn project_app_frame_with(root: &RootState, overrides: &KeyOverrides) -> AppFrameProjection {
    let (view_title, breadcrumbs, service_label, footer_hints) =
        workspace_metadata(root, overrides);

    AppFrameProjection {
        app_title: "nuvem",
        view_title,
        breadcrumbs,
        context: project_frame_context(root),
        service_label,
        operation: frame_operation(root),
        footer_hints,
        footer_pinned_hints: pinned_hints(root, overrides),
    }
}

/// The pinned footer keys, or none while a mode owns the keyboard.
///
/// The predicate mirrors the overlay ladder in `ui::render_active_overlay`
/// and the text-entry sub-views each `*_metadata` builder early-returns for.
/// Every overlay is covered by `transient_footer_hints`, which gives each its
/// own footer; the text-entry sub-views are listed beside it.
fn pinned_hints(root: &RootState, overrides: &KeyOverrides) -> Vec<FrameHint> {
    if a_mode_owns_the_keyboard(root) {
        return Vec::new();
    }
    from_bindings_with(PINNED_BINDINGS, overrides)
}

/// Whether something other than the normal view is reading the keyboard.
///
/// One predicate rather than a flag threaded out of each builder, because the
/// builders answer "what does this view show" and this asks "does `?` still
/// open help" — the same question `keymap::action_for_key` answers from its
/// `Mode`, which the app layer cannot see (it is resolved in
/// `services::runtime::input_context`, and needs the terminal area). The
/// mirror is the cost; the tests in `tests/ui_key_discoverability_test.rs`
/// walk every arm so drift shows up as a red test rather than as a footer
/// advertising a swallowed key.
fn a_mode_owns_the_keyboard(root: &RootState) -> bool {
    transient_footer_hints(root).is_some()
        // Text entry: every one of these reads a printable `?` as a character.
        || root.configurations.context_picker().is_some()
        || root.configurations.is_filter_input_active()
        || root.configurations.create_form().is_some()
        || root.configurations.context_edit_form().is_some()
        || root.resources.is_filter_input_active()
        || root.navigator.is_filter_input_active()
        || root.logging.filter_active
}

/// What a settled status line calls the view on screen: the service, the
/// list drilled into below it, or a detail, in Resources, and the workspace
/// anywhere else.
fn view_status_label(root: &RootState) -> String {
    match root.workspace {
        Workspace::Home => "Home".to_string(),
        Workspace::Configurations => "Configurations".to_string(),
        Workspace::Logging => "Logging".to_string(),
        Workspace::Tunnels => "Tunnels".to_string(),
        Workspace::FinOps => "FinOps".to_string(),
        Workspace::Resources => {
            let service = root.resources.active_service().label();
            if root.resources.is_showing_detail() {
                format!("{service} detail")
            } else {
                drilled_list_name(&root.resources).unwrap_or_else(|| service.to_string())
            }
        }
    }
}

/// The name of the list on screen when it is one a row was opened into
/// rather than the service's own list: a Cloud Storage bucket or folder, a
/// BigQuery dataset's tables or a table's columns. `None` on a service's own
/// list.
///
/// The list pane's title is `Resources / ` and this name, and the settled
/// status line is this name, so the two cannot disagree about which list is
/// open. Before, the line kept saying `BigQuery datasets` over a table's
/// columns.
pub fn drilled_list_name(resources: &ResourcesState) -> Option<String> {
    match resources.active_service() {
        ResourceServiceKind::CloudStorage => match resources.storage().view() {
            StorageView::Buckets | StorageView::BucketDetail => None,
            StorageView::ObjectBrowser | StorageView::ObjectDetail => {
                let storage = resources.storage();
                let bucket = storage
                    .browser_bucket()
                    .map(|bucket| sanitize_display_text(&bucket.name))
                    .unwrap_or_default();
                let prefix = sanitize_display_text(&storage.current_prefix());
                Some(format!("gs://{bucket}/{prefix}"))
            }
        },
        // `bigquery_tables_dataset` and `bigquery_schema_table` are `Some` for
        // as long as their view is on screen (see `set_view`), so the
        // fallbacks are unreachable rather than states with a rendering.
        ResourceServiceKind::BigQueryDatasets => match resources.bigquery_view() {
            BigQueryView::Datasets => None,
            BigQueryView::Tables => {
                let dataset = resources
                    .bigquery_tables_dataset()
                    .map(|dataset| sanitize_display_text(&dataset.dataset_id))
                    .unwrap_or_else(|| "dataset".to_string());
                Some(format!("BigQuery {dataset}"))
            }
            BigQueryView::Schema => {
                let table = resources
                    .bigquery_schema_table()
                    .map(|table| {
                        format!(
                            "{}.{}",
                            sanitize_display_text(&table.dataset_id),
                            sanitize_display_text(&table.table_id)
                        )
                    })
                    .unwrap_or_else(|| "table".to_string());
                Some(format!("BigQuery {table}"))
            }
        },
        _ => None,
    }
}

/// Projects the status area's phase and text, which is the operation's own
/// status except while a transient prompt owns the line.
fn frame_operation(root: &RootState) -> FrameOperationProjection {
    let Some(prompt) = plugin_prefix_prompt(root) else {
        let phase = root
            .system
            .status_phase
            .unwrap_or(root.system.operation.phase);
        // A settled, healthy line names the view on screen rather than the
        // last operation that settled, which is often a background load of
        // a view the operator has left (`[OK] Cloud Run services` over the
        // GCE list). Loading, a failure and a status message still speak
        // for themselves: each is news about something that was asked for.
        let label = if phase == OperationPhase::Healthy && root.system.status_message.is_none() {
            view_status_label(root)
        } else {
            root.system.operation.label.clone()
        };
        return FrameOperationProjection {
            phase,
            label,
            message: root.system.status_message.clone(),
            message_label: root.system.status_operation_label.clone(),
            last_refresh_at: root.system.last_refresh_at,
        };
    };
    // A prompt displaces the status message, so it displaces everything that
    // described it. `status_phase` and `status_operation_label` both belong to
    // the message that is no longer on screen, and `render_status` styles the
    // whole line from the phase: kept, a refused press (`status_phase =
    // Failed`) or a failed load (`operation.phase = Failed`, which
    // `runtime::session` completes for a GCE/GKE/Cloud SQL/Cloud Storage/Cloud
    // Run detail and a GCE action) would paint `[ERR] Plugin key: a` — nuvem
    // reporting a fault at the moment it is waiting on a keystroke.
    //
    // The distinction is between a phase that is a verdict on work that is
    // *over* — and whose message the prompt has just displaced — and one that
    // is still a true statement about what is on screen. `Failed` and `Healthy`
    // are the former: they described the message that is no longer there.
    // `Loading` and `Refreshing` describe work still running underneath, which
    // the operator wants to know about while they pick a key. So does
    // `Degraded`, which is not a verdict on a finished job at all but a standing
    // claim about the freshness of the table currently displayed — neutralising
    // it would repaint a `[STALE]` line as `[OK]` for exactly as long as the
    // prefix stays armed.
    FrameOperationProjection {
        phase: match root.system.operation.phase {
            phase @ (OperationPhase::Loading
            | OperationPhase::Refreshing
            | OperationPhase::Degraded) => phase,
            OperationPhase::Idle | OperationPhase::Healthy | OperationPhase::Failed => {
                OperationPhase::Idle
            }
        },
        // Deliberately empty rather than the operation's label: `render_status`
        // reads `message_label` whenever `message` is `Some`, and this branch
        // always sets one — so the operation's label cannot reach the screen
        // here, and cloning it reads as though it still did.
        label: String::new(),
        message: Some(prompt),
        message_label: None,
        last_refresh_at: root.system.last_refresh_at,
    }
}

/// The prompt shown while `,` has armed the plugin prefix: which keys this
/// scope declares, and the one way out.
///
/// Derived rather than written to the status line by the reducer, so it cannot
/// outlive the flag it describes — and it has to be derived, because *which*
/// keys exist is runtime data from the operator's plugin file that no static
/// footer or help entry can name.
///
/// The key list is never empty, and that is now true by construction: `,` only
/// arms where the scope declares a plugin (`plugins::any_scoped`), which is the
/// same question `bound_keys` answers with the keys themselves — and the flag
/// carries the scope it armed over, so a scope changed underneath it by an
/// effect result (a configuration activation landing its remembered service)
/// leaves no prompt at all rather than one advertising nothing.
fn plugin_prefix_prompt(root: &RootState) -> Option<String> {
    let armed = root.plugin_prefix_pending?;
    if armed != root.resources.active_service() {
        return None;
    }
    let keys = plugins::bound_keys(&root.plugins, armed)
        .iter()
        .map(char::to_string)
        .collect::<Vec<_>>()
        .join(", ");
    Some(format!(
        "Plugin key: {keys} ({} to cancel)",
        PLUGIN_PREFIX_CANCEL.key
    ))
}

fn workspace_metadata(
    root: &RootState,
    overrides: &KeyOverrides,
) -> (String, Vec<String>, Option<&'static str>, Vec<FrameHint>) {
    let (view_title, breadcrumbs, service_label, default_hints) = match root.workspace {
        Workspace::Home => (
            "Home".to_string(),
            breadcrumbs(&["Home"]),
            None,
            with_copy_diagnostic_hint(
                // No `GLOBAL_HELP` here: `?` is pinned to the right of every
                // normal footer now, and listing it twice would cost Home the
                // width that keeps `q Quit` on screen at the 60-column
                // minimum frame size.
                from_bindings_with(
                    &[
                        GLOBAL_REFRESH,
                        NAV_RESOURCES,
                        NAV_CONFIGURATIONS,
                        GLOBAL_QUIT,
                    ],
                    overrides,
                ),
                root,
            ),
        ),
        Workspace::Configurations => configuration_metadata(root, overrides),
        Workspace::Resources => resource_metadata(root, overrides),
        Workspace::Logging => logging_metadata(root, overrides),
        Workspace::Tunnels => tunnels_metadata(root),
        Workspace::FinOps => finops_metadata(root),
    };
    let footer_hints = transient_footer_hints(root).unwrap_or(default_hints);

    (view_title, breadcrumbs, service_label, footer_hints)
}

/// Appends the copy-diagnostic hint, when there's a diagnostic to copy, to
/// the *end* of a base workspace view's footer hints. Only called from the
/// three main (non-modal) view builders — Home, the Configurations list, and
/// the Resources list/detail — never from filter/form/picker/confirmation
/// sub-views, which keep their own curated, narrow footers. Appending last
/// means this is the first hint `render_footer`'s width-budgeted loop drops
/// on a narrow terminal, so it never displaces a higher-priority hint like
/// history Back/Forward.
fn with_copy_diagnostic_hint(mut hints: Vec<FrameHint>, root: &RootState) -> Vec<FrameHint> {
    if root.system.last_diagnostic().is_some() {
        hints.push(FrameHint::from(GLOBAL_COPY_DIAGNOSTIC));
    }
    hints
}

fn transient_footer_hints(root: &RootState) -> Option<Vec<FrameHint>> {
    use crate::app::projection::binding::{
        JUMP_PICKER_SELECT, LAUNCH_CONFIRM, LIST_SELECT, OUTPUT_CLOSE, OUTPUT_COPY, OUTPUT_NEWER,
        OUTPUT_OLDER, OUTPUT_SCROLL, PLUGIN_ACK_DIGIT, PLUGIN_PROMPT_CONTINUE,
        PLUGIN_PROMPT_NEXT_FIELD, PLUGIN_RUN,
    };

    // In the order `services::runtime::input_context` ranks the modes, so the
    // footer names the keys of the overlay that owns them. Every modal owns
    // the footer: a list's `Enter Open` or `/ Filter` left under a dialog
    // names keys the dialog swallows.
    if root.system.show_help {
        return Some(from_bindings(&[HELP_CLOSE]));
    }

    if root.system.plugin_output().is_some() {
        return Some(from_bindings(&[
            OUTPUT_SCROLL,
            OUTPUT_CLOSE,
            OUTPUT_OLDER,
            OUTPUT_NEWER,
            OUTPUT_COPY,
        ]));
    }

    if root.resources.action_confirmation().is_some() {
        return Some(from_bindings(&[CONFIRM_ACCEPT, MODAL_CANCEL]));
    }

    if root.resources.batch_confirmation().is_some() {
        return Some(from_bindings(&[
            CONFIRM_ACCEPT,
            MODAL_CANCEL,
            RES_SCROLL_DETAIL,
        ]));
    }

    if root.configurations.activation_confirmation().is_some() {
        return Some(from_bindings(&[
            CONFIRM_ACCEPT,
            CONFIRM_ACCEPT_Y,
            MODAL_CANCEL,
        ]));
    }

    // `Enter` is offered where the keymap dispatches it: a prompt with a
    // required field still empty, or a dangerous plugin not yet acknowledged,
    // does not continue.
    if let Some(prompt) = root.resources.plugin_prompt() {
        let mut hints = Vec::new();
        if prompt.is_satisfied() {
            hints.push(PLUGIN_PROMPT_CONTINUE);
        }
        hints.extend([PLUGIN_PROMPT_NEXT_FIELD, MODAL_CANCEL]);
        return Some(from_bindings(&hints));
    }

    if let Some(confirmation) = root.resources.plugin_confirmation() {
        return Some(from_bindings(if confirmation.is_ack_satisfied() {
            &[PLUGIN_RUN, MODAL_CANCEL]
        } else {
            &[PLUGIN_ACK_DIGIT, MODAL_CANCEL]
        }));
    }

    if root.resources.command_preview().is_some() {
        return Some(from_bindings(&[PREVIEW_COPY, MODAL_CANCEL]));
    }

    if root.resources.ssh_launch_preview().is_some()
        || root.resources.k9s_launch_confirmation().is_some()
        || root.resources.tunnel_launch_preview().is_some()
    {
        return Some(from_bindings(&[LAUNCH_CONFIRM, MODAL_CANCEL]));
    }

    if root.jump_picker.is_some() {
        return Some(from_bindings(&[
            JUMP_PICKER_SELECT,
            LIST_SELECT,
            MODAL_CANCEL,
        ]));
    }

    if root.command.is_open() {
        return Some(from_bindings(&[PALETTE_SELECT, PALETTE_RUN, MODAL_CANCEL]));
    }

    // Last, matching the mode ladder in `input_context`: every arm above stands
    // for an overlay that renders, and the armed prefix renders none. It owns
    // the keyboard bar the two globals `action_for_key` resolves ahead of the
    // mode match (`Ctrl-C`, and `Ctrl-x`), so the view's own
    // footer — `Enter Open`, `/ Filter`, `J Jumps` — is advertising keys this
    // state swallows. `Esc` is the one *view* key here that a static footer can
    // name; which characters mean anything is the operator's plugin file, and
    // the status-line prompt lists those.
    //
    // Deliberately on `is_some()` rather than on the scope still matching: this
    // says who owns the keyboard, and the prefix owns it either way. The prompt
    // is the part that must be right about *which* keys, and it goes quiet on a
    // mismatch.
    if root.plugin_prefix_pending.is_some() {
        return Some(from_bindings(&[PLUGIN_PREFIX_CANCEL]));
    }

    None
}

fn configuration_metadata(
    root: &RootState,
    overrides: &KeyOverrides,
) -> (String, Vec<String>, Option<&'static str>, Vec<FrameHint>) {
    let configurations = &root.configurations;

    if let Some(picker) = configurations.context_picker() {
        let kind = picker_kind_label(picker.kind());
        if picker.is_filter_input_active() {
            return (
                format!("Filter {kind}"),
                breadcrumbs(&["Home", "Configurations", "Edit context", kind, "Filter"]),
                None,
                from_bindings(&[EDIT_BACKSPACE, FILTER_APPLY, MODAL_CANCEL]),
            );
        }
        return (
            format!("Select {kind}"),
            breadcrumbs(&["Home", "Configurations", "Edit context", kind]),
            None,
            context_picker_hints(picker),
        );
    }

    if configurations.is_filter_input_active() {
        return (
            "Filter configurations".to_string(),
            breadcrumbs(&["Home", "Configurations", "Filter"]),
            None,
            from_bindings(&[FILTER_APPLY, EDIT_BACKSPACE, MODAL_CANCEL]),
        );
    }

    if configurations.create_form().is_some() {
        return (
            "Create configuration".to_string(),
            breadcrumbs(&["Home", "Configurations", "Create"]),
            None,
            from_bindings(&[FORM_SUBMIT, MODAL_CANCEL, FORM_NEXT_FIELD]),
        );
    }

    if configurations.context_edit_form().is_some() {
        return (
            "Edit configuration context".to_string(),
            breadcrumbs(&["Home", "Configurations", "Edit context"]),
            None,
            // Priority order: the footer drops hints whole from the end, and
            // the way out of a form is worth more than its field keys.
            from_bindings(&[FORM_SUBMIT, MODAL_CANCEL, FORM_NEXT_FIELD, CTX_PICK_VALUE]),
        );
    }

    (
        "Configurations".to_string(),
        breadcrumbs(&["Home", "Configurations"]),
        None,
        with_copy_diagnostic_hint(
            from_bindings_with(
                &[
                    GLOBAL_REFRESH,
                    CFG_CREATE,
                    CFG_EDIT,
                    RES_FILTER,
                    CFG_ACTIVATE,
                ],
                overrides,
            ),
            root,
        ),
    )
}

fn context_picker_hints(picker: &ConfigurationContextPicker) -> Vec<FrameHint> {
    let mut picker_hints = Vec::new();
    if picker.selected_item().is_some() {
        picker_hints.extend(from_bindings(&[PICKER_SELECT]));
    }
    picker_hints.extend(from_bindings(&[PICKER_FILTER, PICKER_RETRY, MODAL_CANCEL]));
    picker_hints
}

fn resource_metadata(
    root: &RootState,
    overrides: &KeyOverrides,
) -> (String, Vec<String>, Option<&'static str>, Vec<FrameHint>) {
    let resources = &root.resources;
    let service_label = resources.active_service_label();
    let mut breadcrumbs = breadcrumbs(&["Home", "Resources", service_label]);

    if resources.is_filter_input_active() {
        breadcrumbs.push("Filter".to_string());
        return (
            format!("Filter {service_label}"),
            breadcrumbs,
            Some(service_label),
            from_bindings(&[FILTER_APPLY, EDIT_BACKSPACE, MODAL_CANCEL]),
        );
    }

    // A list opened from a row extends the path with what was opened, so the
    // breadcrumb names the list on screen. An object's detail keeps the
    // bucket and folders it was opened from, as a table's columns keep the
    // dataset.
    let storage_view = resources.storage().view();
    if resources.active_service() == ResourceServiceKind::CloudStorage
        && matches!(
            storage_view,
            StorageView::ObjectBrowser | StorageView::ObjectDetail
        )
    {
        if let Some(bucket) = resources.storage().browser_bucket() {
            breadcrumbs.push(sanitize_display_text(&bucket.name));
        }
        for segment in resources.storage().browser_prefix_path() {
            breadcrumbs.push(sanitize_display_text(segment.trim_end_matches('/')));
        }
    }
    if resources.active_service() == ResourceServiceKind::BigQueryDatasets {
        let view = resources.bigquery_view();
        if view != BigQueryView::Datasets
            && let Some(dataset) = resources.bigquery_tables_dataset()
        {
            breadcrumbs.push(sanitize_display_text(&dataset.dataset_id));
        }
        if view == BigQueryView::Schema
            && let Some(table) = resources.bigquery_schema_table()
        {
            breadcrumbs.push(sanitize_display_text(&table.table_id));
        }
    }

    if resources.is_showing_detail() {
        let detail_name = match resources.active_service() {
            ResourceServiceKind::ComputeInstances => {
                resources.detail_target_name().map(str::to_string)
            }
            ResourceServiceKind::GkeClusters => {
                resources.gke_detail_target_name().map(str::to_string)
            }
            ResourceServiceKind::CloudSql => {
                resources.cloud_sql_detail_target_name().map(str::to_string)
            }
            ResourceServiceKind::CloudRunServices => {
                resources.cloud_run_detail_target_name().map(str::to_string)
            }
            // An object is named below the folders the path already shows.
            ResourceServiceKind::CloudStorage => {
                let prefix = resources.storage().current_prefix();
                resources.storage().detail_target_name().map(|name| {
                    let name = match storage_view {
                        StorageView::ObjectDetail => name.strip_prefix(&prefix).unwrap_or(name),
                        _ => name,
                    };
                    sanitize_display_text(name)
                })
            }
            ResourceServiceKind::SecretManagerSecrets => {
                resources.secret_detail_target_name().map(str::to_string)
            }
            // No detail pane, so no detail breadcrumb to add.
            ResourceServiceKind::CloudFunctions
            | ResourceServiceKind::PersistentDisks
            | ResourceServiceKind::PubSubTopics
            | ResourceServiceKind::VpcNetworks
            | ResourceServiceKind::VpcSubnets
            | ResourceServiceKind::FirewallRules
            | ResourceServiceKind::PubSubSubscriptions
            | ResourceServiceKind::BigQueryDatasets
            | ResourceServiceKind::IamServiceAccounts
            | ResourceServiceKind::IamPolicy => None,
        };
        if let Some(name) = detail_name {
            breadcrumbs.push(name);
        }
        breadcrumbs.push(resources.active_detail_tab_label().to_string());
        let mut detail_hints = from_bindings_with(
            &[
                RES_PREV_TAB,
                RES_NEXT_TAB,
                RES_SCROLL_DETAIL,
                GLOBAL_REFRESH,
                DETAIL_BACK,
            ],
            overrides,
        );
        // `Back` is already `DETAIL_BACK`, which carries `Alt-←` too.
        detail_hints.extend(history_forward_hint(root));
        detail_hints = with_copy_diagnostic_hint(detail_hints, root);
        return (
            format!("{service_label} detail"),
            breadcrumbs,
            Some(service_label),
            detail_hints,
        );
    }

    let mut footer_hints = if root.navigator.sidebar_visible()
        && root.navigator.focus() == NavigatorFocus::Navigator
    {
        from_bindings_with(
            &[NAV_OPEN, NAV_FILTER, RES_TOGGLE_FOCUS, RES_TOGGLE_SIDEBAR],
            overrides,
        )
    } else if resources.is_showing_drilldown() {
        // A drill-down list is a list, so most of the hints below still hold —
        // but two of them do not, and a footer hint is a promise about the key
        // it names. `RES_COLUMN` and `RES_SORT` name controls this table does
        // not have: neither `BIGQUERY_TABLE_COLUMNS` nor
        // `BIGQUERY_SCHEMA_COLUMNS` carries a sort field, and the
        // two keys act on the datasets collection behind the drill, where
        // nothing on screen would move. `RES_LEAVE_DRILLDOWN` takes their
        // place: it is the one key this view has that the list above it does
        // not, and the footer is where an operator finds it without opening
        // the help overlay.
        //
        // `RES_OPEN` was a third omission and is back — conditionally.
        // `Enter` on a table row opens that table's column schema, so the hint
        // promises something the key delivers; `Enter` on a *column* row opens
        // nothing, because a column is the bottom of this hierarchy. So the
        // hint is offered only where the row on screen has a level under it.
        //
        // The order is the list's priority order: the footer drops hints from
        // the end, so opening and leaving come first and the sidebar keys last.
        let mut bindings = Vec::new();
        if resources.drilldown_opens_another_level() {
            bindings.push(RES_OPEN);
        }
        bindings.extend([
            RES_LEAVE_DRILLDOWN,
            RES_FILTER,
            RES_WIDE,
            GLOBAL_REFRESH,
            RES_TOGGLE_SIDEBAR,
            RES_TOGGLE_FOCUS,
        ]);
        from_bindings_with(&bindings, overrides)
    } else {
        // In priority order: the footer drops hints whole from the end when
        // the row is too narrow for all of them, so what a list is *for* —
        // opening, filtering, sorting and widening its rows — comes first,
        // and the sidebar keys, which the sidebar itself shows, come last.
        // `Enter` is offered only where it opens something.
        let mut bindings = Vec::new();
        if crate::app::service_capabilities::opens_on_enter(resources.active_service()) {
            bindings.push(RES_OPEN);
        }
        bindings.extend([
            RES_FILTER,
            RES_SORT,
            RES_WIDE,
            RES_COLUMN,
            GLOBAL_REFRESH,
            RES_TOGGLE_SIDEBAR,
            RES_TOGGLE_FOCUS,
        ]);
        from_bindings_with(&bindings, overrides)
    };
    footer_hints.extend(history_hints(root));
    footer_hints = with_copy_diagnostic_hint(footer_hints, root);

    let view_title = resource_list_title(resources);

    (view_title, breadcrumbs, Some(service_label), footer_hints)
}

/// Builds the resource list view title, appending a `Marked: {n}` segment
/// (and a hidden-marks count) when the active service supports marks and at
/// least one of its rows is marked. GKE, Cloud Run and Cloud Storage have no
/// mark concept, so they never grow the segment.
fn resource_list_title(resources: &crate::app::state::resources::ResourcesState) -> String {
    use nuvem_domain::gcp::resource::ResourceServiceKind;

    let base = "Resources";
    // Defence in depth only: `marked_count()` is the real guard — it returns 0
    // for every service without a mark concept, so the `marked == 0` return
    // below already suppresses the segment. This match just states the intent
    // locally; it is not the source of truth for which services can be marked.
    if !matches!(
        resources.active_service(),
        ResourceServiceKind::ComputeInstances | ResourceServiceKind::CloudSql
    ) {
        return base.to_string();
    }
    let marked = resources.marked_count();
    if marked == 0 {
        return base.to_string();
    }
    let hidden = marked - resources.visible_marked_count();
    if hidden > 0 {
        format!("{base} \u{b7} Marked: {marked} ({hidden} hidden)")
    } else {
        format!("{base} \u{b7} Marked: {marked}")
    }
}

fn logging_metadata(
    root: &RootState,
    overrides: &KeyOverrides,
) -> (String, Vec<String>, Option<&'static str>, Vec<FrameHint>) {
    let logging = &root.logging;
    let summary = logging.query.summary();
    let breadcrumbs = breadcrumbs(&["Home", "Logging", summary.as_str()]);

    if logging.filter_active {
        return (
            "Filter logs".to_string(),
            breadcrumbs,
            None,
            from_bindings(&[EDIT_BACKSPACE, FILTER_APPLY, MODAL_CANCEL]),
        );
    }

    if logging.detail_open {
        let mut detail_hints = from_bindings(&[
            RES_SCROLL_DETAIL,
            LOG_COPY_ENTRY,
            LOG_COPY_INSERT_ID,
            LOG_CLOSE,
        ]);
        detail_hints.extend(history_hints(root));
        return ("Log detail".to_string(), breadcrumbs, None, detail_hints);
    }

    let mut footer_hints = from_bindings_with(
        &[
            GLOBAL_REFRESH,
            LOG_TAIL,
            LOG_TIME_RANGE,
            LOG_WRAP,
            LOG_FILTER,
            LOG_OPEN,
        ],
        overrides,
    );
    footer_hints.extend(history_hints(root));

    ("Logging".to_string(), breadcrumbs, None, footer_hints)
}

/// Frame metadata for the FinOps workspace.
///
/// `r` is in the footer here, unlike Tunnels: a refresh re-probes every cell,
/// which is a real fetch with a visible result.
fn finops_metadata(
    root: &RootState,
) -> (String, Vec<String>, Option<&'static str>, Vec<FrameHint>) {
    let mut footer_hints = from_bindings(&[GLOBAL_REFRESH]);
    footer_hints.extend(history_hints(root));
    let footer_hints = with_copy_diagnostic_hint(footer_hints, root);

    (
        "FinOps".to_string(),
        breadcrumbs(&["Home", "FinOps"]),
        None,
        footer_hints,
    )
}

/// Frame metadata for the Tunnels workspace.
///
/// The footer lists only keys the workspace actually handles. `r` is
/// deliberately absent: it still dispatches (globally, via the rebindable
/// table) but does nothing here, since tunnel rows are pushed by the
/// supervisor rather than fetched, and advertising a no-op refresh would be
/// dishonest. It stays in the help catalog, where its reachability is the
/// point.
fn tunnels_metadata(
    root: &RootState,
) -> (String, Vec<String>, Option<&'static str>, Vec<FrameHint>) {
    let mut footer_hints = from_bindings(&[LIST_SELECT, TUNNEL_KILL, TUNNEL_DISMISS]);
    footer_hints.extend(history_hints(root));
    let footer_hints = with_copy_diagnostic_hint(footer_hints, root);

    (
        "Tunnels".to_string(),
        breadcrumbs(&["Home", "Tunnels"]),
        None,
        footer_hints,
    )
}

fn history_hints(root: &RootState) -> Vec<FrameHint> {
    let mut extra = Vec::new();
    if root.navigation.can_go_back() {
        extra.extend(from_bindings(&[HIST_BACK]));
    }
    if root.navigation.can_go_forward() {
        extra.extend(from_bindings(&[HIST_FORWARD]));
    }
    extra
}

/// The forward half of [`history_hints`], for a view whose own back key
/// already goes back in history.
fn history_forward_hint(root: &RootState) -> Vec<FrameHint> {
    if root.navigation.can_go_forward() {
        from_bindings(&[HIST_FORWARD])
    } else {
        Vec::new()
    }
}

fn breadcrumbs(segments: &[&str]) -> Vec<String> {
    segments
        .iter()
        .map(|segment| (*segment).to_string())
        .collect()
}

fn picker_kind_label(kind: ConfigurationContextPickerKind) -> &'static str {
    match kind {
        ConfigurationContextPickerKind::Project => "Project",
        ConfigurationContextPickerKind::Region => "Region",
        ConfigurationContextPickerKind::Zone => "Zone",
    }
}

/// Formats the age of the last successful refresh relative to `now`.
pub fn format_last_refresh(now: DateTime<Local>, last: Option<DateTime<Local>>) -> String {
    let Some(last) = last else {
        return "Never".to_string();
    };
    let seconds = (now - last).num_seconds().max(0);
    if seconds < 5 {
        "just now".to_string()
    } else if seconds < 60 {
        format!("{seconds}s ago")
    } else if seconds < 3600 {
        format!("{}m ago", seconds / 60)
    } else {
        format!("{}h ago", seconds / 3600)
    }
}
