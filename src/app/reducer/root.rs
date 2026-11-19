use crate::app::state::logging::LoggingStatus;
use crate::app::state::navigation::NavLocation;
use crate::app::state::operations::OperationOutcome;
use crate::app::state::plugin_output::PLUGIN_OUTPUT_PAGE_STEP;
use crate::app::state::resources::BatchTargetStatus;
use crate::app::state::root::{AppState, RootState, Workspace};
use crate::app::{Action, AppError, Effect, EffectResult, ResourcesAction, WorkspaceAction};
use nuvem_domain::gcp::configuration::GcloudConfigurationName;
use nuvem_domain::gcp::diagnostics::{DiagnosticProblem, GcloudDiagnostic, sanitize_detail};
use nuvem_domain::gcp::plugin_output::PluginOutput;
use nuvem_domain::gcp::resource::{ResourceServiceKind, TableStat};

/// The display form of an error: its `Debug` rendering through the one
/// sanitiser every other display surface uses. A `Debug` string is the
/// shape most likely to carry a child's raw stderr, which is exactly what the
/// chokepoint exists to strip.
fn error_message(error: &impl std::fmt::Debug) -> String {
    sanitize_detail(&format!("{error:?}"))
}

const HELP_PAGE_STEP: u16 = 5;

/// Builds a sanitized, display-safe diagnostic from an `AppError` for the given
/// service label. This is the single chokepoint through which every user-facing
/// failure surface must pass — never render `{error:?}` directly, since the raw
/// Debug output can leak unbounded stderr and credential-shaped tokens.
fn build_diagnostic(service: &str, error: &AppError) -> GcloudDiagnostic {
    error.diagnostic(service)
}

/// The operations-log detail for a plugin run's stdout, or `None` when the
/// command printed nothing worth showing.
///
/// Every other detail written to the log is a literal this repo wrote
/// ("Start accepted") or an already-sanitized diagnostic. This one is the raw
/// stdout of a command the *operator* declared, so it is unbounded, arbitrarily
/// many lines, and free to contain whatever they asked gcloud to print —
/// including a token, from `gcloud auth print-access-token`. `sanitize_detail`
/// is the app's single chokepoint for exactly that shape: it strips control
/// characters, collapses the whitespace a multi-line output would otherwise
/// smuggle into a one-line record, redacts credential-shaped values and bounds
/// the length to `DETAIL_MAX_LEN`.
///
/// So the record keeps the head of the output rather than the whole of it, and
/// that is the intended trade: the "Recent operations" card renders `detail` as
/// one `fit_text` line and holds ten entries. Showing a plugin's full output is
/// a terminal's job, not a ten-entry log's — what belongs here is enough to
/// tell the operator which run this was and roughly what came back.
///
/// `None` rather than `Some("")` for a command that printed nothing: an empty
/// detail would draw a blank second line under the entry, and `resolve_by_id`
/// reads `None` as "nothing new to report", which is precisely the case.
///
/// The failure arm does not use this — its detail comes from the diagnostic —
/// but it holds itself to the same bound, by composing the classification with
/// the sanitized preview and passing the pair through `sanitize_detail` again.
/// So every plugin detail in the log, success or failure, is one line of at
/// most `DETAIL_MAX_LEN`.
/// Attaches a run's captured output, unless it printed nothing at all.
///
/// A run can settle with both streams empty — `MissingExecutable`, `Io`, and
/// the task-failure path all do — and attaching that empty capture would make
/// `record.output.is_some()` mean "this run settled" rather than "this run has
/// something to show". Everything downstream reads it the second way: the
/// palette offers the viewer on it, and the viewer opens on it. So the empty
/// case is refused here, at the one point that knows it is empty, rather than
/// re-checked at each place that would otherwise be wrong.
fn attach_plugin_output(app: &mut AppState, operation_id: u64, stdout: &str, stderr: &str) {
    let output = PluginOutput::capture(stdout, stderr);
    if output.lines().is_empty() {
        return;
    }
    // The bool says whether a record was still there to attach to. A run
    // whose entry was evicted mid-flight has nowhere to put its output and
    // nothing on screen to put it beside, so there is nothing to do with the
    // answer beyond not pretending it was checked.
    let _evicted_mid_run = app
        .root
        .system
        .operations
        .attach_output(operation_id, output);
}

fn plugin_run_detail(output: &str) -> Option<String> {
    let detail = sanitize_detail(output);
    (!detail.is_empty()).then_some(detail)
}

/// What the remembered-view sync compares across one reduction.
///
/// Captured before and after the reduction so both halves of the feature —
/// applying a configuration's remembered service and learning a new one — sit
/// in one place. `select_service` has ten reducer call sites — four direct
/// (`command.rs`, `resources.rs`, `navigator.rs`, `navigation.rs`) and six more
/// through `ResourcesState::show_service_list` in `navigation.rs` — and
/// `switch_context` five; wiring either at each site would mean the next site
/// added later compiles cleanly and is silently never remembered.
struct RememberedViewProbe {
    configuration: Option<GcloudConfigurationName>,
    service: ResourceServiceKind,
}

impl RememberedViewProbe {
    fn capture(root: &RootState) -> Self {
        Self {
            configuration: root.configurations.active().map(|c| c.name.clone()),
            service: root.resources.active_service(),
        }
    }
}

pub fn reduce(app: &mut AppState, action: &Action) -> Vec<Effect> {
    // Captured for *every* action, including `Action::Navigation`. The
    // `NavLocation` capture below deliberately skips Navigation — going back
    // must not record new history — but a back-navigation that lands on a
    // different service is a service change like any other and must be
    // remembered. The two guards are separate for that reason.
    let before_view = RememberedViewProbe::capture(&app.root);

    let mut effects = if let Action::Navigation(nav) = action {
        super::navigation::reduce(&mut app.root, nav)
    } else {
        let record = is_forward_nav_action(action);
        let before = record.then(|| NavLocation::capture(&app.root));
        let effects = reduce_inner(app, action);
        if let Some(before) = before {
            let after = NavLocation::capture(&app.root);
            if before != after {
                app.root.navigation.record(before);
            }
        }
        effects
    };

    effects.extend(sync_remembered_view(app, before_view));
    effects
}

/// Applies or learns the remembered view for one reduction, whichever this
/// reduction calls for. The two are exclusive: landing on a configuration reads
/// its memory, so it must not immediately write that value back.
fn sync_remembered_view(app: &mut AppState, before: RememberedViewProbe) -> Option<Effect> {
    // No active configuration means no key: nothing to apply, nothing to learn.
    let after_configuration = app.root.configurations.active().map(|c| c.name.clone())?;

    if before.configuration.as_ref() != Some(&after_configuration) {
        // Landed on a different configuration: apply, learn nothing.
        let service = app
            .root
            .remembered_views
            .get(&after_configuration)
            .unwrap_or(app.root.default_service);
        app.root.resources.select_service(service);
        return None;
    }

    let service = app.root.resources.active_service();
    if service == before.service {
        return None;
    }

    app.root.remembered_views.set(after_configuration, service);

    // The latch: a failed save, or a startup load this build could not read,
    // silences writes for the rest of the session. The value is still recorded
    // in memory above, so the session behaves normally and only the durability
    // is lost. See `RootState::remembered_view_saves_suppressed`.
    if app.root.remembered_view_saves_suppressed {
        return None;
    }
    Some(Effect::SaveRememberedViews {
        views: app.root.remembered_views.clone(),
    })
}

fn is_forward_nav_action(action: &Action) -> bool {
    matches!(
        action,
        Action::Workspace(_)
            | Action::Resources(crate::app::ResourcesAction::OpenSelected)
            | Action::Resources(crate::app::ResourcesAction::SelectService(_))
            | Action::Resources(crate::app::ResourcesAction::OpenLogsForSelection)
            | Action::Navigator(crate::app::NavigatorAction::OpenSelected)
            // Palette submits are candidates, not guaranteed navigations: the
            // `before != after` guard in `reduce` keeps copy/refresh/tunnel
            // commands from recording anything, while the ones that do move
            // (log jumps, service jumps, workspace jumps) finally get history.
            | Action::Command(crate::app::CommandAction::Submit)
            // A related-resource jump is a navigation like any other. It is a
            // candidate rather than a certainty for the same reason palette
            // submits are: jumping to the tab already on screen moves nothing,
            // and the `before != after` guard drops it.
            | Action::Jumps(crate::app::JumpsAction::Submit)
    )
}

fn reduce_inner(app: &mut AppState, action: &Action) -> Vec<Effect> {
    match action {
        Action::Workspace(action) => {
            app.root.workspace = match action {
                WorkspaceAction::Home => Workspace::Home,
                WorkspaceAction::Configurations => Workspace::Configurations,
                WorkspaceAction::Resources => Workspace::Resources,
                WorkspaceAction::Logging => Workspace::Logging,
                WorkspaceAction::Tunnels => Workspace::Tunnels,
                WorkspaceAction::FinOps => Workspace::FinOps,
            };
            if *action == WorkspaceAction::Home && !app.root.system.diagnostics.is_in_flight() {
                app.root.system.diagnostics.begin();
                return vec![Effect::ProbeGcloudDiagnostics {
                    configuration: app.root.configurations.active_name().cloned(),
                }];
            }
            // Entering FinOps probes, like Logging and unlike Tunnels: the
            // grid is empty until something asks, and an empty grid renders
            // "not probed" rather than a total.
            if *action == WorkspaceAction::FinOps && app.root.current_context().is_some() {
                return super::finops::reduce(
                    &mut app.root,
                    &crate::app::action::FinOpsAction::Refresh,
                );
            }
            if *action == WorkspaceAction::Logging && app.root.current_context().is_some() {
                app.root.logging.status = LoggingStatus::Loading;
                return super::logging::load_recent_or_settle(
                    &mut app.root,
                    super::logging::UnloadedBuffer::Discard,
                );
            }
            Vec::new()
        }
        Action::Logging(action) => super::logging::reduce(&mut app.root, action),
        Action::Tunnels(action) => super::tunnels::reduce(&mut app.root, action),
        Action::FinOps(action) => super::finops::reduce(&mut app.root, action),
        Action::Command(action) => super::command::reduce(&mut app.root, action),
        Action::Configurations(action) => super::configurations::reduce(&mut app.root, action),
        Action::Navigator(action) => super::navigator::reduce(&mut app.root, action),
        Action::Resources(action) => super::resources::reduce(&mut app.root, action),
        Action::Jumps(action) => super::jumps::reduce(&mut app.root, action),
        Action::System(action) => {
            match action {
                crate::app::SystemAction::ToggleHelp => {
                    app.root.system.show_help = !app.root.system.show_help;
                    app.root.system.help_scroll = 0;
                }
                crate::app::SystemAction::ClearOverlay => {
                    app.root.system.show_help = false;
                    app.root.system.help_scroll = 0;
                }
                crate::app::SystemAction::ScrollHelpDown => {
                    app.root.system.help_scroll = app.root.system.help_scroll.saturating_add(1);
                }
                crate::app::SystemAction::ScrollHelpUp => {
                    app.root.system.help_scroll = app.root.system.help_scroll.saturating_sub(1);
                }
                crate::app::SystemAction::ScrollHelpPageDown => {
                    app.root.system.help_scroll =
                        app.root.system.help_scroll.saturating_add(HELP_PAGE_STEP);
                }
                crate::app::SystemAction::ScrollHelpPageUp => {
                    app.root.system.help_scroll =
                        app.root.system.help_scroll.saturating_sub(HELP_PAGE_STEP);
                }
                crate::app::SystemAction::ScrollHelpTop => {
                    app.root.system.help_scroll = 0;
                }
                crate::app::SystemAction::ScrollHelpBottom => {
                    app.root.system.help_scroll = u16::MAX;
                }
                crate::app::SystemAction::CopyDiagnostic => {
                    return match app.root.system.last_diagnostic() {
                        Some(diagnostic) => vec![Effect::CopyToClipboard {
                            value: diagnostic.copy_text(),
                            description: "diagnostic".to_string(),
                        }],
                        None => {
                            app.root.system.set_status_message("No diagnostic to copy");
                            Vec::new()
                        }
                    };
                }
                crate::app::SystemAction::ClosePluginOutput => {
                    app.root.system.close_plugin_output();
                }
                crate::app::SystemAction::PluginOutputOlder => {
                    if let Some((viewer, operations)) = app.root.system.plugin_output_and_log() {
                        viewer.older(operations);
                    }
                }
                crate::app::SystemAction::PluginOutputNewer => {
                    if let Some((viewer, operations)) = app.root.system.plugin_output_and_log() {
                        viewer.newer(operations);
                    }
                }
                crate::app::SystemAction::ScrollPluginOutputDown => {
                    if let Some((viewer, _)) = app.root.system.plugin_output_and_log() {
                        viewer.scroll_down(1);
                    }
                }
                crate::app::SystemAction::ScrollPluginOutputUp => {
                    if let Some((viewer, _)) = app.root.system.plugin_output_and_log() {
                        viewer.scroll_up(1);
                    }
                }
                crate::app::SystemAction::ScrollPluginOutputPageDown => {
                    if let Some((viewer, _)) = app.root.system.plugin_output_and_log() {
                        viewer.scroll_down(PLUGIN_OUTPUT_PAGE_STEP);
                    }
                }
                crate::app::SystemAction::ScrollPluginOutputPageUp => {
                    if let Some((viewer, _)) = app.root.system.plugin_output_and_log() {
                        viewer.scroll_up(PLUGIN_OUTPUT_PAGE_STEP);
                    }
                }
                crate::app::SystemAction::CopyPluginOutput => {
                    return match app.root.system.plugin_output() {
                        Some(viewer) => vec![Effect::CopyToClipboard {
                            value: viewer.to_clipboard_text(),
                            description: "plugin output".to_string(),
                        }],
                        None => Vec::new(),
                    };
                }
                crate::app::SystemAction::CancelRunningPlugins => {
                    // Emitted unconditionally. Whether anything is in flight is
                    // the session's knowledge, not the reducer's: the log's
                    // pending `"plugin"` entries are a projection of that
                    // registry and can outlive it (the log is bounded and
                    // evicts from the back), so deciding here would be deciding
                    // on the wrong evidence.
                    return vec![Effect::CancelRunningPlugins];
                }
            }
            Vec::new()
        }
        Action::Effect(effect) => vec![effect.clone()],
        Action::EffectResult(result) => apply_effect_result(app, result),
        Action::Navigation(_) => unreachable!("Action::Navigation is handled in reduce"),
        Action::QuitRequested => Vec::new(),
    }
}

fn apply_effect_result(app: &mut AppState, result: &EffectResult) -> Vec<Effect> {
    let follow_up_refresh = match result {
        EffectResult::ComputeInstanceActionSucceeded { context, .. }
            if app.root.current_context().as_ref() == Some(context) =>
        {
            Some(FollowUpRefresh::Compute(context.clone()))
        }
        EffectResult::CloudSqlInstanceActionSucceeded { context, .. }
            if app.root.current_context().as_ref() == Some(context) =>
        {
            Some(FollowUpRefresh::CloudSql(context.clone()))
        }
        // A plugin ran gcloud, so what is on screen may no longer match — true
        // of a read-only plugin too, whose output the operator may well want
        // reconciled against the list. Deliberately does NOT consult
        // `is_mutating`: it answers `true` for every plugin unconditionally
        // (`src/app/state/command.rs:251`), because a plugin's argv is runtime
        // data, so consulting it here would make this either a no-op or
        // unconditional anyway. Narrowing by verb was decided against here
        // rather than left open: the reload is in-place, so marks,
        // cursor and an open detail pane survive it, and reconciling the screen
        // after any gcloud run is worth one list call even when the run only
        // read. `--read-only` does consult a command-path classifier
        // (`declared_effect`), because being wrong there breaks a promise
        // rather than costing a request.
        //
        // In-place, not `mark_active_stale_and_clear` like the lifecycle arms
        // above: that clears rows, drops marks, resets the cursor and closes the
        // detail pane, which is the right price for a known mutation and the
        // wrong one for a `describe`.
        EffectResult::PluginRunSucceeded { context, .. }
        | EffectResult::PluginRunFailed { context, .. }
        | EffectResult::PluginRunCancelled { context, .. }
            if app.root.workspace == Workspace::Resources
                && app.root.current_context().as_ref() == Some(context) =>
        {
            Some(FollowUpRefresh::ActiveService)
        }
        _ => None,
    };

    // Credentials resolved -> launch k9s interactively through the shared seam.
    //
    // Read-only is re-checked HERE and not only at submit/confirm: those two
    // gates ran before `get-credentials` was dispatched, the palette stays
    // usable while it runs, and a config reload inside that window turns the
    // mode on with the launch already in flight. Without this check the
    // interactive session would open on a live cluster under a mode that
    // promises to refuse exactly that.
    if let EffectResult::K9sCredentialsReady { kube_context, .. } = result {
        apply_effect_result_inner(app, result);
        if app.root.read_only {
            super::resources::refuse_under_read_only(
                &mut app.root,
                crate::app::state::command::CommandDescriptor::OpenSelectedGkeClusterInK9s
                    .label()
                    .as_ref(),
            );
            return Vec::new();
        }
        return vec![Effect::LaunchInteractiveProcess {
            command: crate::app::effect::InteractiveCommand::K9s {
                kube_context: kube_context.clone(),
            },
        }];
    }

    apply_effect_result_inner(app, result);

    // A post-action follow-up refresh carries the *refreshed* service's own
    // applied filter — never the active service's, and never `None` while that
    // service has one applied. The lifecycle action may have targeted a service
    // the user has since switched away from; pushing the active service's term
    // into that other service's reload would leak a cross-service filter and
    // narrow the wrong stored list, but `None` is not the safe default it looks
    // like either. The refresh *replaces* that service's stored rows, and in
    // label mode there is no client-side narrowing left to re-apply
    // (`CompiledFilter::Label` matches every row, because list rows carry no
    // labels), so an unfiltered reload turns the whole project into that
    // service's row set while the tools line still asserts `-l k=v` is in
    // force — and navigating back triggers no reload to correct it.
    // `active_filter_for` encodes both halves off per-service state.
    let mut effects = match follow_up_refresh {
        Some(FollowUpRefresh::Compute(context)) => {
            let filter = super::resources::active_filter_for(
                &app.root,
                ResourceServiceKind::ComputeInstances,
            );
            vec![Effect::RefreshComputeInstances { context, filter }]
        }
        Some(FollowUpRefresh::CloudSql(context)) => {
            let filter =
                super::resources::active_filter_for(&app.root, ResourceServiceKind::CloudSql);
            vec![Effect::RefreshCloudSqlInstances { context, filter }]
        }
        Some(FollowUpRefresh::ActiveService) => {
            // Delegated rather than hand-built: this is the one definition of
            // "reload the active service", and it is what covers every
            // `ResourceServiceKind`, reloads an open detail pane, and carries
            // that service's own applied filter rather than widening it.
            let mut effects = super::resources::reduce(&mut app.root, &ResourcesAction::Refresh);
            // ...with one subtraction. `Refresh` re-requests the Cloud Storage
            // object page with `page_token: None`, and a first page that is not
            // an append replaces the accumulated pages wholesale and resets the
            // cursor and page index (`set_object_page`,
            // `src/app/state/resources/storage.rs:933`). That is the only
            // state-destroying work `Refresh` carries, and it is a fair price
            // for a deliberate `r` — the operator asked. It is not a fair price
            // for a reload nobody pressed a key for: an operator who has paged
            // twice into a bucket would lose those pages and their place the
            // moment some unrelated plugin finished. The bucket list refresh
            // stays; the background poll never emitted this either
            // (`schedule_active_refresh`, `src/services/runtime/session.rs:3720`).
            effects.retain(|effect| !matches!(effect, Effect::LoadStorageObjectPage { .. }));
            effects
        }
        None => Vec::new(),
    };

    if let EffectResult::CloudSqlInstanceDetailLoaded { .. } = result {
        effects.extend(super::resources::cloud_sql_users_lazy_load(&mut app.root));
    }

    // The enrichment fan-out: one `bq show` per row the listing just put on
    // screen. It is emitted here, from the result, rather than from the key
    // that opened the drill, because the rows to ask about are not known until
    // `bq ls` answers.
    //
    // Gated by exactly the condition the `BigQueryTablesLoaded` arm lands
    // under, through the same function, so the two cannot drift into fanning
    // out over rows that were not installed — or installing rows nothing then
    // enriches.
    if let EffectResult::BigQueryTablesLoaded {
        context,
        generation,
        listing,
        ..
    } = result
        && bigquery_tables_result_lands(app, context, *generation, *listing)
    {
        effects.extend(super::resources::bigquery_table_stats_fan_out(
            &app.root,
            context,
            *generation,
            *listing,
        ));
    }

    effects
}

/// Whether a tables listing result belongs to the view that is on screen.
///
/// The context check and the drill check, in one place because three callers
/// need the same answer: the arm that installs a listing, the arm that paints
/// its failure, and the fan-out that follows the first. A listing accepted by
/// one and refused by another would either strand the rows unenriched or fan
/// out over rows the operator has left behind.
fn bigquery_tables_result_lands(
    app: &AppState,
    context: &nuvem_domain::gcp::configuration::GcloudContext,
    generation: crate::app::state::resources::BigQueryDrillGeneration,
    listing: crate::app::state::resources::BigQueryListingEpoch,
) -> bool {
    app.root.current_context().as_ref() == Some(context)
        && app
            .root
            .resources
            .matches_bigquery_tables_scope(generation, listing)
}

/// Which service to refresh after a successful, context-matching lifecycle
/// action result.
enum FollowUpRefresh {
    Compute(nuvem_domain::gcp::configuration::GcloudContext),
    CloudSql(nuvem_domain::gcp::configuration::GcloudContext),
    /// The active service, whatever it is, reloaded in place. Carries no
    /// context: the guard that produces it has already established that the
    /// result's context is the current one.
    ActiveService,
}

fn apply_effect_result_inner(app: &mut AppState, result: &EffectResult) {
    match result {
        EffectResult::CostRecommendationsLoaded { .. }
        | EffectResult::CostRecommendationsFailed { .. }
        | EffectResult::BudgetsLoaded { .. }
        | EffectResult::BudgetsFailed { .. } => {
            super::finops::apply_result(&mut app.root, result.clone());
        }
        EffectResult::ConfigurationsLoaded { configurations } => {
            apply_configurations(app, configurations);
            app.root.system.clear_status_message();
        }
        EffectResult::ConfigurationsReloadedWithWarning {
            configurations,
            error,
        } => {
            apply_configurations(app, configurations);
            let diag = build_diagnostic("gcloud", error);
            app.root.system.set_error_status_message(diag.headline());
            app.root.system.set_last_diagnostic(diag);
        }
        EffectResult::ConfigurationActivated { configuration } => {
            app.root.configurations.set_active(configuration.clone());
            let restore = app.root.current_context();
            app.root.resources.switch_context(restore.as_ref());
            app.root.navigation.reset();
            app.root.system.clear_status_message();
        }
        EffectResult::ConfigurationActivationReloadFailed {
            configuration,
            error,
        } => {
            app.root.configurations.set_active(configuration.clone());
            let restore = app.root.current_context();
            app.root.resources.switch_context(restore.as_ref());
            app.root.navigation.reset();
            let diag = build_diagnostic("gcloud", error);
            app.root.system.set_error_status_message(diag.headline());
            app.root.system.set_last_diagnostic(diag);
        }
        EffectResult::ConfigurationCreatedReloadFailed {
            configuration,
            error,
        } => {
            app.root.configurations.set_active(configuration.clone());
            let restore = app.root.current_context();
            app.root.resources.switch_context(restore.as_ref());
            app.root.navigation.reset();
            let diag = build_diagnostic("gcloud", error);
            app.root.system.set_error_status_message(diag.headline());
            app.root.system.set_last_diagnostic(diag);
        }
        EffectResult::ConfigurationContextUpdateReloadFailed {
            configuration,
            error,
        } => {
            app.root.configurations.set_active(configuration.clone());
            let restore = app.root.current_context();
            app.root.resources.switch_context(restore.as_ref());
            app.root.navigation.reset();
            let diag = build_diagnostic("gcloud", error);
            app.root.system.set_error_status_message(diag.headline());
            app.root.system.set_last_diagnostic(diag);
        }
        EffectResult::ContextPickerLoaded {
            configuration,
            kind,
            items,
        } => {
            app.root
                .configurations
                .set_context_picker_loaded(configuration, *kind, items.clone());
        }
        EffectResult::ContextPickerLoadFailed {
            configuration,
            kind,
            error,
        } => {
            let diag = build_diagnostic("gcloud configuration", error);
            app.root.configurations.set_context_picker_degraded(
                configuration,
                *kind,
                diag.headline(),
            );
            app.root.system.set_last_diagnostic(diag);
        }
        EffectResult::ComputeInstancesLoaded { context, instances } => {
            if app.root.current_context().as_ref() == Some(context) {
                let dropped = app
                    .root
                    .resources
                    .set_instances(context.clone(), instances.clone());
                match dropped {
                    Some(name) => app
                        .root
                        .system
                        .set_status_message(format!("{name} is no longer available")),
                    None => app.root.system.clear_status_message(),
                }
                // A completed batch workflow is dismissed once this refresh has
                // applied fresh rows; while targets are still pending (a
                // concurrent refresh mid-batch), it stays visible. Only the
                // service that owns the workflow may dismiss it.
                app.root
                    .resources
                    .clear_completed_batch_workflow_owned_by(ResourceServiceKind::ComputeInstances);
            }
        }
        EffectResult::GkeClustersLoaded { context, clusters } => {
            if app.root.current_context().as_ref() == Some(context) {
                let dropped = app
                    .root
                    .resources
                    .set_gke_clusters(context.clone(), clusters.clone());
                match dropped {
                    Some(name) => app
                        .root
                        .system
                        .set_status_message(format!("{name} is no longer available")),
                    None => app.root.system.clear_status_message(),
                }
            }
        }
        EffectResult::ComputeInstancesLoadFailed { context, error } => {
            if app.root.current_context().as_ref() == Some(context) {
                app.root
                    .resources
                    .mark_active_stale_and_clear(ResourceServiceKind::ComputeInstances);
                let diag = build_diagnostic("GCE instances", error);
                app.root
                    .system
                    .set_operation_status_message("GCE instances", diag.headline());
                app.root
                    .resources
                    .set_service_diagnostic(ResourceServiceKind::ComputeInstances, diag.clone());
                app.root.resources.fail_load(
                    ResourceServiceKind::ComputeInstances,
                    context.clone(),
                    diag.headline(),
                );
                app.root.system.set_last_diagnostic(diag);
                app.root
                    .resources
                    .clear_completed_batch_workflow_owned_by(ResourceServiceKind::ComputeInstances);
            }
        }
        EffectResult::GkeClustersLoadFailed { context, error } => {
            if app.root.current_context().as_ref() == Some(context) {
                app.root
                    .resources
                    .mark_active_stale_and_clear(ResourceServiceKind::GkeClusters);
                let diag = build_diagnostic("GKE clusters", error);
                app.root
                    .system
                    .set_operation_status_message("GKE clusters", diag.headline());
                app.root
                    .resources
                    .set_service_diagnostic(ResourceServiceKind::GkeClusters, diag.clone());
                app.root.resources.fail_load(
                    ResourceServiceKind::GkeClusters,
                    context.clone(),
                    diag.headline(),
                );
                app.root.system.set_last_diagnostic(diag);
            }
        }
        EffectResult::ComputeInstanceDetailLoaded { context, detail } => {
            if app.root.current_context().as_ref() == Some(context)
                && app
                    .root
                    .resources
                    .is_active_detail_target(&detail.name, &detail.zone)
            {
                app.root.resources.set_detail(detail.clone());
                app.root.system.clear_status_message();
            }
        }
        EffectResult::ComputeInstanceDetailLoadFailed {
            context,
            instance_name,
            instance_zone,
            error,
        } => {
            if app.root.current_context().as_ref() == Some(context)
                && app
                    .root
                    .resources
                    .is_active_detail_target(instance_name, instance_zone)
            {
                let diag = build_diagnostic("GCE instances", error);
                let headline = diag.headline();
                let detail_message = if diag.detail_preview().is_empty() {
                    headline.clone()
                } else {
                    format!("{headline} ({})", diag.detail_preview())
                };
                app.root
                    .resources
                    .set_detail_error(instance_name.clone(), detail_message);
                app.root
                    .system
                    .set_operation_status_message("GCE detail", headline);
                app.root.system.set_last_diagnostic(diag);
            }
        }
        EffectResult::GkeClusterDetailLoaded { context, detail } => {
            if app.root.current_context().as_ref() == Some(context)
                && app
                    .root
                    .resources
                    .is_active_gke_detail_target(&detail.name, &detail.location)
            {
                app.root.resources.set_gke_detail(detail.clone());
                app.root.system.clear_status_message();
            }
        }
        EffectResult::GkeClusterDetailLoadFailed {
            context,
            cluster_name,
            cluster_location,
            error,
        } => {
            if app.root.current_context().as_ref() == Some(context)
                && app
                    .root
                    .resources
                    .is_active_gke_detail_target(cluster_name, cluster_location)
            {
                let diag = build_diagnostic("GKE clusters", error);
                let headline = diag.headline();
                let detail_message = if diag.detail_preview().is_empty() {
                    headline.clone()
                } else {
                    format!("{headline} ({})", diag.detail_preview())
                };
                app.root
                    .resources
                    .set_gke_detail_error(cluster_name.clone(), detail_message);
                app.root
                    .system
                    .set_operation_status_message("GKE detail", headline);
                app.root.system.set_last_diagnostic(diag);
            }
        }
        EffectResult::CloudSqlInstancesLoaded { context, instances } => {
            if app.root.current_context().as_ref() == Some(context) {
                let dropped = app
                    .root
                    .resources
                    .set_cloud_sql_instances(context.clone(), instances.clone());
                match dropped {
                    Some(name) => app
                        .root
                        .system
                        .set_status_message(format!("{name} is no longer available")),
                    None => app.root.system.clear_status_message(),
                }
                // A completed batch workflow is dismissed once this refresh has
                // applied fresh rows; while targets are still pending (a
                // concurrent refresh mid-batch), it stays visible. Only the
                // service that owns the workflow may dismiss it.
                app.root
                    .resources
                    .clear_completed_batch_workflow_owned_by(ResourceServiceKind::CloudSql);
            }
        }
        EffectResult::CloudSqlInstancesLoadFailed { context, error } => {
            if app.root.current_context().as_ref() == Some(context) {
                app.root
                    .resources
                    .mark_active_stale_and_clear(ResourceServiceKind::CloudSql);
                let diag = build_diagnostic("Cloud SQL instances", error);
                app.root
                    .system
                    .set_operation_status_message("Cloud SQL instances", diag.headline());
                app.root
                    .resources
                    .set_service_diagnostic(ResourceServiceKind::CloudSql, diag.clone());
                app.root.resources.fail_load(
                    ResourceServiceKind::CloudSql,
                    context.clone(),
                    diag.headline(),
                );
                app.root.system.set_last_diagnostic(diag);
                // A failed follow-up refresh must still dismiss a completed
                // batch-progress panel, or it strands over the list forever.
                app.root
                    .resources
                    .clear_completed_batch_workflow_owned_by(ResourceServiceKind::CloudSql);
            }
        }
        EffectResult::CloudSqlInstanceDetailLoaded { context, detail } => {
            if app.root.current_context().as_ref() == Some(context)
                && app
                    .root
                    .resources
                    .is_active_cloud_sql_detail_target(&detail.name, &detail.region)
            {
                app.root
                    .resources
                    .set_cloud_sql_detail(detail.as_ref().clone());
                app.root.system.clear_status_message();
            }
        }
        EffectResult::CloudSqlInstanceDetailLoadFailed {
            context,
            instance_name,
            instance_region,
            error,
        } => {
            if app.root.current_context().as_ref() == Some(context)
                && app
                    .root
                    .resources
                    .is_active_cloud_sql_detail_target(instance_name, instance_region)
            {
                let message = error_message(&error);
                app.root
                    .resources
                    .set_cloud_sql_detail_error(instance_name.clone(), message.clone());
                app.root
                    .system
                    .set_operation_status_message("Cloud SQL detail", message);
            }
        }
        EffectResult::CloudSqlUsersLoaded {
            context,
            instance_name,
            users,
        } => {
            if app.root.current_context().as_ref() == Some(context)
                && app
                    .root
                    .resources
                    .is_active_cloud_sql_detail_name(instance_name)
            {
                app.root.resources.set_cloud_sql_users(users.clone());
                app.root.system.clear_status_message();
            }
        }
        EffectResult::CloudSqlUsersLoadFailed {
            context,
            instance_name,
            error,
        } => {
            if app.root.current_context().as_ref() == Some(context)
                && app
                    .root
                    .resources
                    .is_active_cloud_sql_detail_name(instance_name)
            {
                app.root
                    .system
                    .set_operation_status_message("Cloud SQL users", error_message(&error));
            }
        }
        EffectResult::StorageBucketsLoaded { context, buckets } => {
            if app.root.current_context().as_ref() == Some(context) {
                let dropped = app
                    .root
                    .resources
                    .storage_mut()
                    .set_buckets(context.clone(), buckets.clone());
                match dropped {
                    Some(name) => app
                        .root
                        .system
                        .set_status_message(format!("{name} is no longer available")),
                    None => app.root.system.clear_status_message(),
                }
            }
        }
        EffectResult::StorageBucketsLoadFailed { context, error } => {
            if app.root.current_context().as_ref() == Some(context) {
                app.root
                    .resources
                    .mark_active_stale_and_clear(ResourceServiceKind::CloudStorage);
                let diag = build_diagnostic("Cloud Storage", error);
                app.root
                    .system
                    .set_operation_status_message("Cloud Storage", diag.headline());
                app.root
                    .resources
                    .set_service_diagnostic(ResourceServiceKind::CloudStorage, diag.clone());
                app.root.resources.fail_load(
                    ResourceServiceKind::CloudStorage,
                    context.clone(),
                    diag.headline(),
                );
                app.root.system.set_last_diagnostic(diag);
                app.root
                    .resources
                    .storage_mut()
                    .set_problem_requester_pays(error.indicates_requester_pays());
            }
        }
        EffectResult::StorageBucketDetailLoaded { context, detail } => {
            if app.root.current_context().as_ref() == Some(context)
                && app
                    .root
                    .resources
                    .storage()
                    .is_active_bucket_detail_target(&detail.name)
            {
                app.root
                    .resources
                    .storage_mut()
                    .set_bucket_detail(detail.clone());
                app.root.system.clear_status_message();
            }
        }
        EffectResult::StorageBucketDetailLoadFailed {
            context,
            bucket_name,
            error,
        } => {
            if app.root.current_context().as_ref() == Some(context)
                && app
                    .root
                    .resources
                    .storage()
                    .is_active_bucket_detail_target(bucket_name)
            {
                let message = error_message(&error);
                app.root
                    .resources
                    .storage_mut()
                    .set_bucket_detail_error(bucket_name.clone(), message.clone());
                app.root
                    .system
                    .set_operation_status_message("Cloud Storage detail", message);
            }
        }
        EffectResult::StorageObjectPageLoaded {
            context,
            bucket,
            prefix,
            append,
            page,
        } => {
            if app.root.current_context().as_ref() == Some(context) {
                app.root.resources.storage_mut().set_object_page(
                    bucket,
                    prefix,
                    page.clone(),
                    *append,
                );
                app.root.system.clear_status_message();
            }
        }
        EffectResult::StorageObjectPageLoadFailed {
            context,
            bucket,
            prefix,
            append,
            error,
        } => {
            if app.root.current_context().as_ref() == Some(context)
                && app
                    .root
                    .resources
                    .storage()
                    .matches_browser_scope(bucket, prefix)
            {
                let message = sanitize_detail(error.detail());
                if *append {
                    app.root
                        .resources
                        .storage_mut()
                        .set_browser_partial_page_error(message.clone());
                } else {
                    app.root.resources.storage_mut().set_browser_problem(
                        DiagnosticProblem::new(error.kind(), message.clone())
                            .naming_missing_binary(error.missing_binary()),
                        error.indicates_requester_pays(),
                    );
                }
                app.root
                    .system
                    .set_operation_status_message("Cloud Storage objects", message);
            }
        }
        EffectResult::StorageObjectDetailLoaded { context, detail } => {
            if app.root.current_context().as_ref() == Some(context)
                && app
                    .root
                    .resources
                    .storage()
                    .is_active_object_detail_target(&detail.key)
            {
                app.root
                    .resources
                    .storage_mut()
                    .set_object_detail(detail.clone());
                app.root.system.clear_status_message();
            }
        }
        EffectResult::StorageObjectDetailLoadFailed {
            context,
            object_key,
            error,
        } => {
            if app.root.current_context().as_ref() == Some(context)
                && app
                    .root
                    .resources
                    .storage()
                    .is_active_object_detail_target(object_key)
            {
                let message = error_message(&error);
                app.root
                    .resources
                    .storage_mut()
                    .set_object_detail_error(object_key.clone(), message.clone());
                app.root
                    .system
                    .set_operation_status_message("Cloud Storage object detail", message);
            }
        }
        EffectResult::ComputeInstanceActionSucceeded {
            context,
            instance_name,
            instance_zone,
            action,
            operation_id,
        } => {
            let action_name = match action {
                crate::app::state::resources::ComputeInstanceAction::Start => "Start",
                crate::app::state::resources::ComputeInstanceAction::Stop => "Stop",
                crate::app::state::resources::ComputeInstanceAction::Reset => "Reset",
                crate::app::state::resources::ComputeInstanceAction::Suspend => "Suspend",
                crate::app::state::resources::ComputeInstanceAction::Resume => "Resume",
            };
            // The pending operations-log entry must always resolve once its
            // result arrives, even if the active context has since moved
            // on, otherwise it is left stuck at Pending forever. Settling it by
            // id settles this action's own entry: a `(kind, target)` lookup
            // would settle whichever entry for the same instance is pending,
            // which is the wrong one whenever a batch and a single-row action
            // overlap.
            app.root.system.operations.resolve_by_id(
                *operation_id,
                OperationOutcome::Succeeded,
                Some(format!("{action_name} accepted")),
            );
            // Resolve the matching batch-progress target (no-op for a
            // single-row action, which has no active workflow).
            app.root.resources.resolve_batch_target(
                ResourceServiceKind::ComputeInstances,
                instance_name,
                Some(instance_zone),
                BatchTargetStatus::Succeeded,
            );
            if app.root.current_context().as_ref() == Some(context) {
                app.root
                    .resources
                    .mark_active_stale_and_clear(ResourceServiceKind::ComputeInstances);
                app.root.system.set_operation_status_message(
                    "GCE action",
                    format!("{action_name} accepted for {instance_name}; refreshing GCE instances"),
                );
            }
        }
        EffectResult::ComputeInstanceActionFailed {
            context,
            instance_name,
            instance_zone,
            action: _,
            error,
            operation_id,
        } => {
            // Resolved here, before the context check below, because the
            // pending operations-log entry must always settle once its result
            // arrives, even if the active context has since moved on;
            // otherwise it is left stuck at Pending forever. By id for the same
            // reason as the success arm: only the id names this action's own
            // entry.
            let diag = build_diagnostic("GCE action", error);
            app.root.system.operations.resolve_by_id(
                *operation_id,
                OperationOutcome::Failed,
                Some(diag.label().to_string()),
            );
            // Resolve the matching batch-progress target (no-op for a
            // single-row action, which has no active workflow).
            app.root.resources.resolve_batch_target(
                ResourceServiceKind::ComputeInstances,
                instance_name,
                Some(instance_zone),
                BatchTargetStatus::Failed(error.label().to_string()),
            );
            if app.root.current_context().as_ref() == Some(context) {
                app.root
                    .resources
                    .mark_active_stale_and_clear(ResourceServiceKind::ComputeInstances);
                app.root
                    .system
                    .set_operation_status_message("GCE action", diag.headline());
            }
            app.root.system.set_last_diagnostic(diag);
        }
        EffectResult::PluginRunSucceeded {
            context,
            label,
            output,
            stderr,
            operation_id,
        } => {
            // Settled before any context check, like the lifecycle results: a
            // pending entry must always settle once its result arrives, or it
            // is stuck at Pending forever. The run is on a blocking thread and
            // outlives the context it was confirmed under, so "the operator has
            // moved on" is the normal case here, not an edge one.
            app.root.system.operations.resolve_by_id(
                *operation_id,
                OperationOutcome::Succeeded,
                plugin_run_detail(output),
            );
            // The whole of what the command printed, kept as lines. The detail
            // above stays what it was — one sanitized line for the log — and
            // this is the long form the viewer reads.
            attach_plugin_output(app, *operation_id, output, stderr);
            // The status line, unlike the log, belongs to the context on
            // screen: a run confirmed under a configuration the operator has
            // left must not narrate itself over what they are looking at now.
            if app.root.current_context().as_ref() == Some(context) {
                app.root
                    .system
                    .set_operation_status_message("plugin", format!("{label} finished"));
            }
        }
        EffectResult::PluginRunFailed {
            context,
            label,
            error,
            stderr,
            operation_id,
        } => {
            // `build_diagnostic` is the chokepoint that classifies and
            // sanitizes; the record carries both halves because neither is
            // enough on its own. The classification alone ("permission denied")
            // cannot tell two failures of the same kind apart, and gcloud's own
            // words alone leave the operator to classify stderr themselves.
            //
            // Composed, then re-sanitized, so `DETAIL_MAX_LEN` bounds the whole
            // string and not each half: `detail_preview` is already at that
            // bound in the worst case, so concatenating anything onto it would
            // make this the one log detail longer than every other. Sanitizing
            // twice is not redundant here — the second pass is what applies the
            // bound to the *sum*, at the cost of the classification eating a few
            // characters of the preview, which is the right way round.
            let diag = build_diagnostic("plugin", error);
            let detail = if diag.detail_preview().is_empty() {
                diag.label().to_string()
            } else {
                sanitize_detail(&format!("{}: {}", diag.label(), diag.detail_preview()))
            };
            app.root.system.operations.resolve_by_id(
                *operation_id,
                OperationOutcome::Failed,
                Some(detail),
            );
            // A failure's stderr is the case with the most to read and the
            // least room to read it in: `detail` above is bounded at
            // `DETAIL_MAX_LEN`, and a gcloud error trailer is routinely
            // longer than that.
            attach_plugin_output(app, *operation_id, "", stderr);
            if app.root.current_context().as_ref() == Some(context) {
                app.root.system.set_operation_status_message(
                    "plugin",
                    format!("{label}: {}", diag.headline()),
                );
            }
            // Recorded unconditionally, so the "copy diagnostic" key can hand
            // over the failure of a run the operator started even if they have
            // since switched configuration; it is their own command's failure
            // either way.
            app.root.system.set_last_diagnostic(diag);
        }
        EffectResult::PluginRunCancelled {
            context,
            label,
            operation_id: _,
        } => {
            // Settles nothing. `cancel_running_plugins` already settled this
            // entry by what the run's claim proved — issued or not — and
            // `resolve_by_id` is pending-guarded, so a settle here could only
            // ever be a no-op or a second, weaker answer to a question already
            // answered. No diagnostic either: the operator stopped this
            // command, and a `Y` press must not hand them gcloud's words about
            // it as though something went wrong.
            if app.root.current_context().as_ref() == Some(context) {
                app.root
                    .system
                    .set_operation_status_message("plugin", format!("{label} cancelled"));
            }
        }
        EffectResult::CloudSqlInstanceActionSucceeded {
            context,
            instance_name,
            action,
            operation_id,
        } => {
            let action_name = match action {
                crate::app::state::resources::CloudSqlInstanceAction::Start => "Start",
                crate::app::state::resources::CloudSqlInstanceAction::Stop => "Stop",
                crate::app::state::resources::CloudSqlInstanceAction::Restart => "Restart",
                crate::app::state::resources::CloudSqlInstanceAction::Failover => "Failover",
            };
            // The pending operations-log entry must always resolve once its
            // result arrives, even if the active context has since moved
            // on, otherwise it is left stuck at Pending forever. The id names
            // this action's own entry; `(kind, target)` would not distinguish
            // it from another action pending on the same instance.
            app.root.system.operations.resolve_by_id(
                *operation_id,
                OperationOutcome::Succeeded,
                Some(format!("{action_name} accepted")),
            );
            // Resolve the matching batch-progress target (no-op for a
            // single-row action, which has no active workflow). Cloud SQL
            // results carry no region and instance IDs are unique per project,
            // so the target matches by name alone.
            app.root.resources.resolve_batch_target(
                ResourceServiceKind::CloudSql,
                instance_name,
                None,
                BatchTargetStatus::Succeeded,
            );
            if app.root.current_context().as_ref() == Some(context) {
                app.root
                    .resources
                    .mark_active_stale_and_clear(ResourceServiceKind::CloudSql);
                app.root.system.set_operation_status_message(
                    "Cloud SQL action",
                    format!(
                        "{action_name} accepted for {instance_name}; refreshing Cloud SQL instances"
                    ),
                );
            }
        }
        EffectResult::CloudSqlInstanceActionFailed {
            context,
            instance_name,
            action: _,
            error,
            operation_id,
        } => {
            // Resolved here, before the context check below, because the
            // pending operations-log entry must always settle once its result
            // arrives, even if the active context has since moved on;
            // otherwise it is left stuck at Pending forever. By id for the same
            // reason as the success arm: only the id names this action's own
            // entry.
            app.root.system.operations.resolve_by_id(
                *operation_id,
                OperationOutcome::Failed,
                Some(error.label().to_string()),
            );
            // Resolve the matching batch-progress target (no-op for a
            // single-row action, which has no active workflow). Cloud SQL
            // results carry no region and instance IDs are unique per project,
            // so the target matches by name alone.
            app.root.resources.resolve_batch_target(
                ResourceServiceKind::CloudSql,
                instance_name,
                None,
                BatchTargetStatus::Failed(error.label().to_string()),
            );
            if app.root.current_context().as_ref() == Some(context) {
                app.root
                    .resources
                    .mark_active_stale_and_clear(ResourceServiceKind::CloudSql);
                app.root
                    .system
                    .set_operation_status_message("Cloud SQL action", error_message(&error));
            }
        }
        EffectResult::ClipboardCopied { description } => {
            app.root
                .system
                .set_status_message(format!("Copied {description} to clipboard"));
        }
        EffectResult::ClipboardCopyFailed {
            description: _,
            error,
        } => {
            let diag = build_diagnostic("gcloud", error);
            app.root.system.set_error_status_message(diag.headline());
            app.root.system.set_last_diagnostic(diag);
        }
        // Deliberately silent: see the variant's doc comment. Matched
        // explicitly rather than swept into a catch-all so the exhaustive
        // match keeps doing its job.
        EffectResult::RememberedViewsSaved => {}
        EffectResult::RememberedViewsSaveFailed { error } => {
            // Latched, not repeated: see `RootState::remembered_view_saves_suppressed`.
            app.root.remembered_view_saves_suppressed = true;
            app.root.system.set_error_status_message(format!(
                "Could not save the remembered view: {error}; \
                 nuvem will not try again this session"
            ));
        }
        EffectResult::LogEntriesSaved { path, entries } => {
            let noun = if *entries == 1 { "entry" } else { "entries" };
            app.root
                .system
                .set_status_message(format!("Saved {entries} log {noun} to {path}"));
        }
        EffectResult::LogEntriesSaveFailed { error } => {
            let diag = build_diagnostic("log export", error);
            // `headline()` drops the service, so on its own it reads as an
            // unattributed error right after the user pressed a save key. Name
            // the operation in the status while still recording the diagnostic
            // so `Y` can copy the full details.
            app.root.system.set_error_status_message(format!(
                "Could not save log entries: {}",
                diag.headline()
            ));
            app.root.system.set_last_diagnostic(diag);
        }
        EffectResult::GcloudDiagnosticsProbed { report } => {
            app.root.system.diagnostics.complete(report.clone());
            // AC #6: make the health probe's problem copyable via `Y`, the
            // same as resource-load failures. Leave `last_diagnostic`
            // untouched when the probe reports no problem.
            if let Some(problem) = &report.problem {
                let diag = GcloudDiagnostic::new(
                    problem.kind,
                    "gcloud",
                    Some("gcloud auth list".to_string()),
                    None,
                    &problem.message,
                )
                .naming_missing_binary(problem.missing_binary);
                app.root.system.set_last_diagnostic(diag);
            }
        }
        EffectResult::RecentLogsLoaded {
            context,
            query,
            severity,
            range,
            entries,
            dropped,
        } => {
            // Last-request-wins by identity: a page is only applied when the
            // whole narrowing window it was fetched for -- context, query,
            // severity floor and time bucket -- still matches current state.
            // A slower load for a stale floor is dropped even when it arrives
            // last.
            if app.root.current_context().as_ref() == Some(context)
                && app.root.logging.query == *query
                && app.root.logging.severity_floor == *severity
                && app.root.logging.time_range == *range
            {
                app.root.logging.apply_recent(entries.clone(), *dropped);
            }
        }
        EffectResult::LogsTailed {
            context,
            query,
            severity,
            entries,
            dropped,
        } => {
            // Same last-request-wins-by-identity rule as the recent-page arm,
            // and it matters more here: `apply_tail` appends and rederives the
            // cursor from the newest entry, so a page fetched under an
            // abandoned floor would leave sub-floor rows on screen and regress
            // the cursor until the next window change.
            if app.root.current_context().as_ref() == Some(context)
                && app.root.logging.query == *query
                && app.root.logging.severity_floor == *severity
            {
                app.root.logging.apply_tail(entries.clone(), *dropped);
            }
        }
        EffectResult::RecentLogsLoadFailed {
            context,
            query,
            error,
        }
        | EffectResult::LogsTailFailed {
            context,
            query,
            error,
        } => {
            if app.root.current_context().as_ref() == Some(context)
                && app.root.logging.query == *query
            {
                app.root.logging.record_error(error.kind());
            }
        }
        EffectResult::CloudRunServicesLoaded { context, services } => {
            if app.root.current_context().as_ref() == Some(context) {
                let dropped = app
                    .root
                    .resources
                    .set_cloud_run_services(context.clone(), services.clone());
                match dropped {
                    Some(name) => app
                        .root
                        .system
                        .set_status_message(format!("{name} is no longer available")),
                    None => app.root.system.clear_status_message(),
                }
            }
        }
        EffectResult::CloudRunServicesLoadFailed { context, error } => {
            if app.root.current_context().as_ref() == Some(context) {
                app.root
                    .resources
                    .mark_active_stale_and_clear(ResourceServiceKind::CloudRunServices);
                let diag = build_diagnostic("Cloud Run services", error);
                app.root
                    .system
                    .set_operation_status_message("Cloud Run services", diag.headline());
                app.root
                    .resources
                    .set_service_diagnostic(ResourceServiceKind::CloudRunServices, diag.clone());
                app.root.resources.fail_load(
                    ResourceServiceKind::CloudRunServices,
                    context.clone(),
                    diag.headline(),
                );
                app.root.system.set_last_diagnostic(diag);
            }
        }
        EffectResult::CloudRunServiceDetailLoaded { context, detail } => {
            if app.root.current_context().as_ref() == Some(context)
                && app
                    .root
                    .resources
                    .is_active_cloud_run_detail_target(&detail.name, &detail.region)
            {
                app.root.resources.set_cloud_run_detail(detail.clone());
                app.root.system.clear_status_message();
            }
        }
        EffectResult::CloudRunServiceDetailLoadFailed {
            context,
            service_name,
            service_region,
            error,
        } => {
            if app.root.current_context().as_ref() == Some(context)
                && app
                    .root
                    .resources
                    .is_active_cloud_run_detail_target(service_name, service_region)
            {
                let message = error_message(&error);
                app.root
                    .resources
                    .set_cloud_run_detail_error(service_name.clone(), message.clone());
                app.root
                    .system
                    .set_operation_status_message("Cloud Run detail", message);
            }
        }
        EffectResult::SecretsLoaded { context, secrets } => {
            if app.root.current_context().as_ref() == Some(context) {
                let dropped = app
                    .root
                    .resources
                    .set_secrets(context.clone(), secrets.clone());
                match dropped {
                    Some(name) => app
                        .root
                        .system
                        .set_status_message(format!("{name} is no longer available")),
                    None => app.root.system.clear_status_message(),
                }
            }
        }
        EffectResult::CloudFunctionsLoaded { context, functions } => {
            if app.root.current_context().as_ref() == Some(context) {
                app.root
                    .resources
                    .set_cloud_functions(context.clone(), functions.clone());
                app.root.system.clear_status_message();
            }
        }
        EffectResult::PersistentDisksLoaded { context, disks } => {
            if app.root.current_context().as_ref() == Some(context) {
                app.root
                    .resources
                    .set_persistent_disks(context.clone(), disks.clone());
                app.root.system.clear_status_message();
            }
        }
        EffectResult::CloudFunctionsLoadFailed { context, error } => {
            if app.root.current_context().as_ref() == Some(context) {
                app.root
                    .resources
                    .mark_active_stale_and_clear(ResourceServiceKind::CloudFunctions);
                let diag = build_diagnostic("Cloud Functions", error);
                app.root
                    .system
                    .set_operation_status_message("Cloud Functions", diag.headline());
                app.root
                    .resources
                    .set_service_diagnostic(ResourceServiceKind::CloudFunctions, diag.clone());
                app.root.resources.fail_load(
                    ResourceServiceKind::CloudFunctions,
                    context.clone(),
                    diag.headline(),
                );
                app.root.system.set_last_diagnostic(diag);
            }
        }
        EffectResult::PersistentDisksLoadFailed { context, error } => {
            if app.root.current_context().as_ref() == Some(context) {
                app.root
                    .resources
                    .mark_active_stale_and_clear(ResourceServiceKind::PersistentDisks);
                let diag = build_diagnostic("persistent disks", error);
                app.root
                    .system
                    .set_operation_status_message("persistent disks", diag.headline());
                app.root
                    .resources
                    .set_service_diagnostic(ResourceServiceKind::PersistentDisks, diag.clone());
                app.root.resources.fail_load(
                    ResourceServiceKind::PersistentDisks,
                    context.clone(),
                    diag.headline(),
                );
                app.root.system.set_last_diagnostic(diag);
            }
        }
        EffectResult::PubSubTopicsLoaded { context, topics } => {
            if app.root.current_context().as_ref() == Some(context) {
                app.root
                    .resources
                    .set_pubsub_topics(context.clone(), topics.clone());
                app.root.system.clear_status_message();
            }
        }
        EffectResult::PubSubTopicsLoadFailed { context, error } => {
            if app.root.current_context().as_ref() == Some(context) {
                app.root
                    .resources
                    .mark_active_stale_and_clear(ResourceServiceKind::PubSubTopics);
                let diag = build_diagnostic("Pub/Sub topics", error);
                app.root
                    .system
                    .set_operation_status_message("Pub/Sub topics", diag.headline());
                app.root
                    .resources
                    .set_service_diagnostic(ResourceServiceKind::PubSubTopics, diag.clone());
                app.root.resources.fail_load(
                    ResourceServiceKind::PubSubTopics,
                    context.clone(),
                    diag.headline(),
                );
                app.root.system.set_last_diagnostic(diag);
            }
        }
        EffectResult::VpcNetworksLoaded { context, networks } => {
            if app.root.current_context().as_ref() == Some(context) {
                app.root
                    .resources
                    .set_vpc_networks(context.clone(), networks.clone());
                app.root.system.clear_status_message();
            }
        }
        EffectResult::VpcNetworksLoadFailed { context, error } => {
            if app.root.current_context().as_ref() == Some(context) {
                app.root
                    .resources
                    .mark_active_stale_and_clear(ResourceServiceKind::VpcNetworks);
                let diag = build_diagnostic("VPC networks", error);
                app.root
                    .system
                    .set_operation_status_message("VPC networks", diag.headline());
                app.root
                    .resources
                    .set_service_diagnostic(ResourceServiceKind::VpcNetworks, diag.clone());
                app.root.resources.fail_load(
                    ResourceServiceKind::VpcNetworks,
                    context.clone(),
                    diag.headline(),
                );
                app.root.system.set_last_diagnostic(diag);
            }
        }
        EffectResult::FirewallRulesLoaded { context, rules } => {
            if app.root.current_context().as_ref() == Some(context) {
                app.root
                    .resources
                    .set_firewall_rules(context.clone(), rules.clone());
                app.root.system.clear_status_message();
            }
        }
        EffectResult::FirewallRulesLoadFailed { context, error } => {
            if app.root.current_context().as_ref() == Some(context) {
                app.root
                    .resources
                    .mark_active_stale_and_clear(ResourceServiceKind::FirewallRules);
                let diag = build_diagnostic("firewall rules", error);
                app.root
                    .system
                    .set_operation_status_message("firewall rules", diag.headline());
                app.root
                    .resources
                    .set_service_diagnostic(ResourceServiceKind::FirewallRules, diag.clone());
                app.root.resources.fail_load(
                    ResourceServiceKind::FirewallRules,
                    context.clone(),
                    diag.headline(),
                );
                app.root.system.set_last_diagnostic(diag);
            }
        }
        EffectResult::VpcSubnetsLoaded { context, subnets } => {
            if app.root.current_context().as_ref() == Some(context) {
                app.root
                    .resources
                    .set_vpc_subnets(context.clone(), subnets.clone());
                app.root.system.clear_status_message();
            }
        }
        EffectResult::VpcSubnetsLoadFailed { context, error } => {
            if app.root.current_context().as_ref() == Some(context) {
                app.root
                    .resources
                    .mark_active_stale_and_clear(ResourceServiceKind::VpcSubnets);
                let diag = build_diagnostic("VPC subnets", error);
                app.root
                    .system
                    .set_operation_status_message("VPC subnets", diag.headline());
                app.root
                    .resources
                    .set_service_diagnostic(ResourceServiceKind::VpcSubnets, diag.clone());
                app.root.resources.fail_load(
                    ResourceServiceKind::VpcSubnets,
                    context.clone(),
                    diag.headline(),
                );
                app.root.system.set_last_diagnostic(diag);
            }
        }
        EffectResult::PubSubSubscriptionsLoaded {
            context,
            subscriptions,
        } => {
            if app.root.current_context().as_ref() == Some(context) {
                app.root
                    .resources
                    .set_pubsub_subscriptions(context.clone(), subscriptions.clone());
                app.root.system.clear_status_message();
            }
        }
        EffectResult::PubSubSubscriptionsLoadFailed { context, error } => {
            if app.root.current_context().as_ref() == Some(context) {
                app.root
                    .resources
                    .mark_active_stale_and_clear(ResourceServiceKind::PubSubSubscriptions);
                let diag = build_diagnostic("Pub/Sub subscriptions", error);
                app.root
                    .system
                    .set_operation_status_message("Pub/Sub subscriptions", diag.headline());
                app.root
                    .resources
                    .set_service_diagnostic(ResourceServiceKind::PubSubSubscriptions, diag.clone());
                app.root.resources.fail_load(
                    ResourceServiceKind::PubSubSubscriptions,
                    context.clone(),
                    diag.headline(),
                );
                app.root.system.set_last_diagnostic(diag);
            }
        }
        EffectResult::BigQueryDatasetsLoaded { context, datasets } => {
            if app.root.current_context().as_ref() == Some(context) {
                app.root
                    .resources
                    .set_bigquery_datasets(context.clone(), datasets.clone());
                app.root.system.clear_status_message();
            }
        }
        EffectResult::BigQueryDatasetsLoadFailed { context, error } => {
            if app.root.current_context().as_ref() == Some(context) {
                app.root
                    .resources
                    .mark_active_stale_and_clear(ResourceServiceKind::BigQueryDatasets);
                let diag = build_diagnostic("BigQuery datasets", error);
                app.root
                    .system
                    .set_operation_status_message("BigQuery datasets", diag.headline());
                app.root
                    .resources
                    .set_service_diagnostic(ResourceServiceKind::BigQueryDatasets, diag.clone());
                app.root.resources.fail_load(
                    ResourceServiceKind::BigQueryDatasets,
                    context.clone(),
                    diag.headline(),
                );
                app.root.system.set_last_diagnostic(diag);
            }
        }
        EffectResult::BigQueryTablesLoaded {
            context,
            generation,
            listing,
            tables,
        } => {
            if bigquery_tables_result_lands(app, context, *generation, *listing) {
                app.root.resources.set_bigquery_tables(tables.clone());
                app.root.system.clear_status_message();
            }
        }
        EffectResult::BigQueryTablesLoadFailed {
            context,
            generation,
            listing,
            error,
        } => {
            if bigquery_tables_result_lands(app, context, *generation, *listing) {
                let diag = build_diagnostic("BigQuery tables", error);
                app.root.resources.set_bigquery_tables_problem(
                    DiagnosticProblem::new(error.kind(), diag.headline())
                        .naming_missing_binary(error.missing_binary()),
                );
                app.root
                    .system
                    .set_operation_status_message("BigQuery tables", diag.headline());
                app.root.system.set_last_diagnostic(diag);
            }
        }
        // One table's stats answered. Merged onto that row alone: the setter
        // writes no cursor, no filter, no problem and no loading flag, so an
        // answer arriving while the operator is reading the table changes
        // three cells and nothing they are doing.
        //
        // Both identities are checked. The generation drops an answer for a
        // drill that has been left — `Esc` moves it and does not mint an epoch,
        // so the epoch alone would still be the live one. The listing epoch
        // drops an answer for a listing that `r` replaced, which the generation
        // cannot see because a retry is not a new drill.
        EffectResult::BigQueryTableStatsLoaded {
            context,
            table_id,
            generation,
            listing,
            stats,
        } => {
            if app.root.current_context().as_ref() == Some(context)
                && app
                    .root
                    .resources
                    .matches_bigquery_enrichment_scope(*generation, *listing)
            {
                let stats = *stats;
                app.root
                    .resources
                    .merge_bigquery_table_enrichment(table_id, |row| {
                        row.rows = stats.rows;
                        row.bytes = stats.bytes;
                        row.last_modified = stats.last_modified;
                    });
            }
        }
        // A refused `show`, which writes nothing at all — deliberately, and
        // this is the half of the two-call policy that is easiest to lose.
        //
        // The row was listed with all three stats unavailable and keeps them,
        // which is already the truthful rendering: the `-` in those cells says
        // "not answered", never "zero". What must not happen here is anything
        // the *listing's* failure does. `set_bigquery_tables_problem` clears
        // the rows and paints an error over the table, which is right when
        // `bq ls` fails — an empty table would claim the dataset has no tables
        // — and catastrophic when one `bq show` out of two hundred is denied,
        // because `bigquery.tables.list` granted with `bigquery.tables.get`
        // denied is an ordinary read-only IAM shape. Nor a status message: N
        // calls would mean N messages over a table that is perfectly readable.
        //
        // What it does write is the refusal itself, onto that one row: three
        // explicit unavailables, through the same setter the success path
        // uses. Stated honestly, that body is a no-op in every state it is
        // reachable in — deleting the three writes kills no test, because a
        // row only ever holds numbers from a successful show of the live
        // listing, and the live listing does not also refuse the same table.
        // It is kept for two reasons, neither of them "it is not a no-op". It
        // asserts "the cells show what *this* listing's call answered" against
        // the day a row can arrive already filled — a listing that preserved
        // rows, a cache — and it is what gives the scope check on this arm
        // something to protect: with the writes gone, dropping the scope check
        // would be unobservable too, and with them
        // `a_refused_show_from_an_earlier_listing_is_dropped_too` fails when
        // the check goes, because an abandoned listing's refusal would blank a
        // row the live listing has filled.
        //
        // The inconsistency is worth naming: by the standard that deleted the
        // generation conjunct from the enrichment scope check — hand-mutate,
        // kill nothing, remove — this body would have gone the same way. That
        // standard was wrong there; the conjunct was load-bearing and is back.
        // Here it is a judgement that three cheap writes are worth keeping,
        // not a claim that a test would catch their removal.
        EffectResult::BigQueryTableStatsLoadFailed {
            context,
            table_id,
            generation,
            listing,
            error: _,
        } => {
            if app.root.current_context().as_ref() == Some(context)
                && app
                    .root
                    .resources
                    .matches_bigquery_enrichment_scope(*generation, *listing)
            {
                app.root
                    .resources
                    .merge_bigquery_table_enrichment(table_id, |row| {
                        row.rows = TableStat::Unavailable;
                        row.bytes = TableStat::Unavailable;
                        row.last_modified = TableStat::Unavailable;
                    });
            }
        }
        // The schema for the table the operator drilled into.
        //
        // Two guards, one conjunct shorter than the enrichment arms above. The
        // context drops an answer for a project the operator has switched
        // away from; the generation drops one for a table they have left, or
        // for a previous entry into the table they are back inside. There is
        // no listing epoch here because there is no second call under one
        // drill for it to separate — see
        // `ResourcesState::matches_bigquery_schema_scope`.
        //
        // A schema of no fields lands exactly like a schema of forty: an
        // answered call is an answered call, and `set_bigquery_schema` clears
        // the problem either way. `bq show` returning a table whose schema
        // flattens to nothing is not the same fact as `bq show` being refused,
        // and painting the refusal's diagnostic over it would say the read
        // failed when it succeeded.
        EffectResult::BigQuerySchemaLoaded {
            context,
            generation,
            fields,
        } => {
            if app.root.current_context().as_ref() == Some(context)
                && app
                    .root
                    .resources
                    .matches_bigquery_schema_scope(*generation)
            {
                app.root.resources.set_bigquery_schema(fields.clone());
                app.root.system.clear_status_message();
            }
        }
        // A refused schema read fails the drill, the way a refused `bq ls`
        // fails the tables listing and unlike the per-row `bq show`, which
        // leaves one row's cells unavailable and reports nothing. There is no
        // partial schema to keep: this one call is the view's whole content,
        // and an empty column list in place of a diagnostic would read as a
        // table with no columns.
        EffectResult::BigQuerySchemaLoadFailed {
            context,
            generation,
            error,
        } => {
            if app.root.current_context().as_ref() == Some(context)
                && app
                    .root
                    .resources
                    .matches_bigquery_schema_scope(*generation)
            {
                let diag = build_diagnostic("BigQuery schema", error);
                app.root.resources.set_bigquery_schema_problem(
                    DiagnosticProblem::new(error.kind(), diag.headline())
                        .naming_missing_binary(error.missing_binary()),
                );
                app.root
                    .system
                    .set_operation_status_message("BigQuery schema", diag.headline());
                app.root.system.set_last_diagnostic(diag);
            }
        }
        EffectResult::ServiceAccountsLoaded { context, accounts } => {
            if app.root.current_context().as_ref() == Some(context) {
                app.root
                    .resources
                    .set_service_accounts(context.clone(), accounts.clone());
                app.root.system.clear_status_message();
            }
        }
        EffectResult::IamPolicyLoaded { context, members } => {
            if app.root.current_context().as_ref() == Some(context) {
                app.root
                    .resources
                    .set_policy_members(context.clone(), members.clone());
                app.root.system.clear_status_message();
            }
        }
        EffectResult::IamPolicyLoadFailed { context, error } => {
            if app.root.current_context().as_ref() == Some(context) {
                app.root
                    .resources
                    .mark_active_stale_and_clear(ResourceServiceKind::IamPolicy);
                let diag = build_diagnostic("IAM policy", error);
                app.root
                    .system
                    .set_operation_status_message("IAM policy", diag.headline());
                app.root
                    .resources
                    .set_service_diagnostic(ResourceServiceKind::IamPolicy, diag.clone());
                app.root.resources.fail_load(
                    ResourceServiceKind::IamPolicy,
                    context.clone(),
                    diag.headline(),
                );
                app.root.system.set_last_diagnostic(diag);
            }
        }
        EffectResult::ServiceAccountsLoadFailed { context, error } => {
            if app.root.current_context().as_ref() == Some(context) {
                app.root
                    .resources
                    .mark_active_stale_and_clear(ResourceServiceKind::IamServiceAccounts);
                let diag = build_diagnostic("service accounts", error);
                app.root
                    .system
                    .set_operation_status_message("service accounts", diag.headline());
                app.root
                    .resources
                    .set_service_diagnostic(ResourceServiceKind::IamServiceAccounts, diag.clone());
                app.root.resources.fail_load(
                    ResourceServiceKind::IamServiceAccounts,
                    context.clone(),
                    diag.headline(),
                );
                app.root.system.set_last_diagnostic(diag);
            }
        }
        EffectResult::SecretsLoadFailed { context, error } => {
            if app.root.current_context().as_ref() == Some(context) {
                app.root
                    .resources
                    .mark_active_stale_and_clear(ResourceServiceKind::SecretManagerSecrets);
                let diag = build_diagnostic("secrets", error);
                app.root
                    .system
                    .set_operation_status_message("secrets", diag.headline());
                app.root.resources.set_service_diagnostic(
                    ResourceServiceKind::SecretManagerSecrets,
                    diag.clone(),
                );
                app.root.resources.fail_load(
                    ResourceServiceKind::SecretManagerSecrets,
                    context.clone(),
                    diag.headline(),
                );
                app.root.system.set_last_diagnostic(diag);
            }
        }
        EffectResult::SecretDetailLoaded { context, detail } => {
            if app.root.current_context().as_ref() == Some(context)
                && app
                    .root
                    .resources
                    .is_active_secret_detail_target(&detail.name)
            {
                app.root.resources.set_secret_detail(detail.clone());
                app.root.system.clear_status_message();
            }
        }
        EffectResult::SecretDetailLoadFailed {
            context,
            secret_name,
            error,
        } => {
            if app.root.current_context().as_ref() == Some(context)
                && app
                    .root
                    .resources
                    .is_active_secret_detail_target(secret_name)
            {
                let message = error_message(&error);
                app.root
                    .resources
                    .set_secret_detail_error(secret_name.clone(), message.clone());
                app.root
                    .system
                    .set_operation_status_message("secret detail", message);
            }
        }
        EffectResult::SecretVersionsLoaded {
            context,
            secret_name,
            versions,
        } => {
            if app.root.current_context().as_ref() == Some(context)
                && app
                    .root
                    .resources
                    .is_active_secret_detail_target(secret_name)
            {
                app.root.resources.set_secret_versions(versions.clone());
            }
        }
        EffectResult::SecretVersionsLoadFailed {
            context,
            secret_name,
            error,
        } => {
            if app.root.current_context().as_ref() == Some(context)
                && app
                    .root
                    .resources
                    .is_active_secret_detail_target(secret_name)
            {
                let message = error_message(&error);
                app.root
                    .resources
                    .set_secret_versions_error(message.clone());
                app.root
                    .system
                    .set_operation_status_message("secret versions", message);
            }
        }
        EffectResult::CloudRunRevisionsLoaded {
            context,
            service_name,
            service_region,
            revisions,
        } => {
            if app.root.current_context().as_ref() == Some(context)
                && app
                    .root
                    .resources
                    .is_active_cloud_run_detail_target(service_name, service_region)
                && let Some(mut detail) = app.root.resources.cloud_run_detail().cloned()
            {
                detail.revisions = revisions.clone();
                app.root.resources.set_cloud_run_detail(detail);
            }
        }
        EffectResult::CloudRunRevisionsLoadFailed {
            context,
            service_name,
            service_region,
            error,
        } => {
            if app.root.current_context().as_ref() == Some(context)
                && app
                    .root
                    .resources
                    .is_active_cloud_run_detail_target(service_name, service_region)
            {
                app.root
                    .system
                    .set_operation_status_message("Cloud Run revisions", error_message(&error));
            }
        }
        EffectResult::CloudRunRevisionDetailLoaded { context, detail } => {
            if app.root.current_context().as_ref() == Some(context)
                && app
                    .root
                    .resources
                    .cloud_run_revision_target()
                    .map(|r| r.name.as_str())
                    == Some(detail.name.as_str())
            {
                app.root
                    .resources
                    .set_cloud_run_revision_detail(detail.clone());
                app.root.system.clear_status_message();
            }
        }
        EffectResult::CloudRunRevisionDetailLoadFailed {
            context,
            revision_name,
            region: _,
            error,
        } => {
            if app.root.current_context().as_ref() == Some(context)
                && app
                    .root
                    .resources
                    .cloud_run_revision_target()
                    .map(|r| r.name.as_str())
                    == Some(revision_name.as_str())
            {
                let message = error_message(&error);
                app.root
                    .resources
                    .set_cloud_run_revision_detail_error(message.clone());
                app.root
                    .system
                    .set_operation_status_message("Cloud Run revision detail", message);
            }
        }
        EffectResult::Failed { error } => {
            let diag = build_diagnostic("gcloud", error);
            app.root.system.set_error_status_message(diag.headline());
            app.root.system.set_last_diagnostic(diag);
        }
        EffectResult::InteractiveProcessCompleted {
            description,
            outcome,
        } => {
            app.root
                .system
                .set_status_message(interactive_status_line(description, outcome));
        }
        EffectResult::K9sLaunchPrepared {
            target,
            existing_contexts,
        } => {
            let step = nuvem_domain::gcp::k9s::resolve_credential_step(
                existing_contexts.as_deref(),
                target,
            );
            app.root
                .resources
                .open_k9s_launch_confirmation(target.clone(), step);
        }
        EffectResult::K9sCredentialsReady { .. } => {
            // The interactive launch is dispatched as a follow-up effect in
            // `apply_effect_result`; nothing to mutate here.
        }
        // Every tunnel result for an id that is no longer in the list is a
        // silent no-op: a late result from a dismissed row must not resurrect
        // it, and ids are never reused, so it cannot land on a different one.
        EffectResult::TunnelPortAllocated { id, local_port } => {
            app.root.tunnels.record_allocated_port(*id, *local_port);
        }
        EffectResult::TunnelReady { id, local_port } => {
            app.root.tunnels.mark_active(*id, *local_port);
        }
        EffectResult::TunnelFailed { id, message } => {
            app.root.tunnels.mark_failed(*id, message.clone());
        }
        EffectResult::TunnelNotice { id, message } => {
            app.root.tunnels.set_notice(*id, message.clone());
        }
        EffectResult::TunnelExited { id } => {
            super::tunnels::apply_exited(&mut app.root, *id);
        }
        EffectResult::TunnelStopped { id } => {
            app.root.tunnels.mark_stopped(*id);
        }
        EffectResult::K9sCredentialsFailed {
            cluster_name,
            error,
        } => {
            app.root.system.set_error_status_message(format!(
                "k9s: get-credentials for {cluster_name} failed: {error:?}"
            ));
        }
    }
}

fn interactive_status_line(
    description: &str,
    outcome: &crate::services::runtime::interactive::InteractiveProcessOutcome,
) -> String {
    use crate::services::runtime::interactive::InteractiveProcessOutcome as O;
    match outcome {
        O::Succeeded => format!("{description} finished"),
        O::ExitedNonZero { code } => format!("{description} exited with status {code}"),
        O::TerminatedBySignal { signal } => {
            format!("{description} terminated by signal {signal}")
        }
        O::MissingBinary => format!("{description} failed: binary not found"),
        O::LaunchFailed { message } => format!("{description} failed: {message}"),
        O::NotStartedInDemo => {
            format!("Demo mode does not open sessions: {description} was not started")
        }
    }
}

fn apply_configurations(
    app: &mut AppState,
    configurations: &[nuvem_domain::gcp::configuration::GcloudConfiguration],
) {
    let previous_context = app.root.current_context();
    app.root.configurations.set_rows(configurations.to_vec());
    let new_context = app.root.current_context();
    if previous_context.is_some() && previous_context != new_context {
        app.root.resources.switch_context(new_context.as_ref());
        app.root.navigation.reset();
    }
}

#[cfg(test)]
mod remembered_view_tests {
    use super::*;
    use crate::app::state::system::OperationPhase;
    use crate::app::{NavAction, ResourcesAction};
    use nuvem_domain::gcp::configuration::{GcloudConfiguration, GcloudConfigurationName};
    use nuvem_domain::gcp::resource::ResourceServiceKind;

    fn configuration(name: &str, is_active: bool) -> GcloudConfiguration {
        GcloudConfiguration {
            name: GcloudConfigurationName::new(name),
            is_active,
            account: None,
            project: Some(format!("{name}-project")),
            region: None,
            zone: None,
        }
    }

    /// A state whose active configuration is `active`, built through the same
    /// `set_rows` the `ConfigurationsLoaded` result uses.
    fn state_with_active(active: &str) -> AppState {
        let mut app = AppState::new();
        app.root
            .configurations
            .set_rows(vec![configuration(active, true)]);
        app
    }

    /// Switches the active configuration through a real reducer path — the
    /// `ConfigurationActivated` result, one of the five `switch_context` call
    /// sites — rather than poking `active_service` or the row list. A test that
    /// bypassed `reduce` would pass against an implementation that had wired
    /// the logic into each call site by hand, and so would prove nothing.
    fn activate_configuration_for_test(app: &mut AppState, name: &str) -> Vec<Effect> {
        reduce(
            app,
            &Action::EffectResult(EffectResult::ConfigurationActivated {
                configuration: configuration(name, true),
            }),
        )
    }

    #[test]
    fn selecting_a_different_service_records_it_against_the_active_configuration() {
        let mut app = state_with_active("dev");

        let effects = reduce(
            &mut app,
            &Action::Resources(ResourcesAction::SelectService(
                ResourceServiceKind::CloudSql,
            )),
        );

        assert_eq!(
            app.root
                .remembered_views
                .get(&GcloudConfigurationName::new("dev")),
            Some(ResourceServiceKind::CloudSql)
        );
        assert_eq!(
            effects
                .iter()
                .filter(|e| matches!(e, Effect::SaveRememberedViews { .. }))
                .count(),
            1,
            "exactly one save effect per learned change, got {effects:?}"
        );
    }

    #[test]
    fn reselecting_the_active_service_emits_no_save() {
        let mut app = state_with_active("dev");
        reduce(
            &mut app,
            &Action::Resources(ResourcesAction::SelectService(
                ResourceServiceKind::CloudSql,
            )),
        );

        let effects = reduce(
            &mut app,
            &Action::Resources(ResourcesAction::SelectService(
                ResourceServiceKind::CloudSql,
            )),
        );

        assert!(
            !effects
                .iter()
                .any(|e| matches!(e, Effect::SaveRememberedViews { .. })),
            "an unchanged value must not be written, got {effects:?}"
        );
    }

    /// The bypass this task exists to close. `reduce` early-returns for
    /// `Action::Navigation` before the `NavLocation` capture, and
    /// `navigation.rs` calls `select_service` — so a back-navigation landing on
    /// a different service must still be recorded.
    ///
    /// The scenario is built so the back demonstrably moves services: entering
    /// the resources workspace records Home, selecting Cloud SQL records the
    /// Compute list, so `Back` lands on Compute with Cloud SQL active.
    #[test]
    fn a_navigation_action_that_changes_the_service_is_still_recorded() {
        let mut app = state_with_active("dev");
        reduce(&mut app, &Action::Workspace(WorkspaceAction::Resources));
        assert_eq!(
            app.root.resources.active_service(),
            ResourceServiceKind::ComputeInstances,
            "fixture: the resources workspace opens on Compute"
        );
        reduce(
            &mut app,
            &Action::Resources(ResourcesAction::SelectService(
                ResourceServiceKind::CloudSql,
            )),
        );
        assert_eq!(
            app.root.resources.active_service(),
            ResourceServiceKind::CloudSql,
            "fixture: the select must have moved the active service"
        );

        let effects = reduce(&mut app, &Action::Navigation(NavAction::Back));

        assert_eq!(
            app.root.resources.active_service(),
            ResourceServiceKind::ComputeInstances,
            "fixture: the back must land on the Compute list"
        );
        assert_eq!(
            app.root
                .remembered_views
                .get(&GcloudConfigurationName::new("dev")),
            Some(ResourceServiceKind::ComputeInstances),
            "a Navigation action that changes the service must be recorded, \
             or the early return at the top of `reduce` is still a bypass"
        );
        assert!(
            effects
                .iter()
                .any(|e| matches!(e, Effect::SaveRememberedViews { .. })),
            "and it must be persisted, got {effects:?}"
        );
    }

    /// The startup restore, and the transition every other test here skips: at
    /// launch nothing is active yet, so the first `ConfigurationsLoaded` is a
    /// `None -> Some` landing rather than a switch between two configurations.
    /// A choke point that only reacted to a change *between* configurations
    /// would leave the remembered view unapplied on exactly the path the
    /// feature exists for.
    #[test]
    fn the_first_configuration_to_land_applies_its_remembered_service() {
        let mut app = AppState::new();
        assert!(
            app.root.configurations.active().is_none(),
            "fixture: nothing is active before configurations load"
        );
        app.root.default_service = ResourceServiceKind::ComputeInstances;
        app.root.remembered_views.set(
            GcloudConfigurationName::new("prod"),
            ResourceServiceKind::GkeClusters,
        );

        reduce(
            &mut app,
            &Action::EffectResult(EffectResult::ConfigurationsLoaded {
                configurations: vec![configuration("prod", true)],
            }),
        );

        assert_eq!(
            app.root.resources.active_service(),
            ResourceServiceKind::GkeClusters,
            "the first configuration to land must apply its remembered view, \
             not the default service"
        );
    }

    #[test]
    fn landing_on_a_configuration_applies_its_remembered_service() {
        let mut app = state_with_active("dev");
        app.root.remembered_views.set(
            GcloudConfigurationName::new("prod"),
            ResourceServiceKind::GkeClusters,
        );

        activate_configuration_for_test(&mut app, "prod");

        assert_eq!(
            app.root.resources.active_service(),
            ResourceServiceKind::GkeClusters
        );
    }

    #[test]
    fn landing_on_a_configuration_with_no_memory_applies_default_service() {
        let mut app = state_with_active("dev");
        app.root.default_service = ResourceServiceKind::CloudRunServices;

        activate_configuration_for_test(&mut app, "prod");

        assert_eq!(
            app.root.resources.active_service(),
            ResourceServiceKind::CloudRunServices
        );
    }

    /// Landing reads memory; it must not write it back.
    #[test]
    fn landing_on_a_configuration_emits_no_save() {
        let mut app = state_with_active("dev");
        app.root.remembered_views.set(
            GcloudConfigurationName::new("prod"),
            ResourceServiceKind::GkeClusters,
        );

        let effects = activate_configuration_for_test(&mut app, "prod");

        assert!(
            !effects
                .iter()
                .any(|e| matches!(e, Effect::SaveRememberedViews { .. })),
            "landing learned nothing and must write nothing, got {effects:?}"
        );
    }

    #[test]
    fn no_save_is_emitted_once_a_save_has_failed() {
        let mut app = state_with_active("dev");
        app.root.remembered_view_saves_suppressed = true;

        let effects = reduce(
            &mut app,
            &Action::Resources(ResourcesAction::SelectService(
                ResourceServiceKind::CloudSql,
            )),
        );

        assert!(
            !effects
                .iter()
                .any(|e| matches!(e, Effect::SaveRememberedViews { .. })),
            "the latch must stop further writes, got {effects:?}"
        );
    }

    /// The latch's setter, driven through the reducer's result-handling path:
    /// a failed save both latches and says so once in the status line.
    #[test]
    fn a_failed_save_latches_and_reports_once() {
        let mut app = state_with_active("dev");

        reduce(
            &mut app,
            &Action::EffectResult(EffectResult::RememberedViewsSaveFailed {
                error: "permission denied".to_string(),
            }),
        );

        assert!(
            app.root.remembered_view_saves_suppressed,
            "the failed save must latch"
        );
        let message = app
            .root
            .system
            .status_message
            .as_deref()
            .expect("a failed save must be reported in the status line");
        assert!(
            message.contains("permission denied"),
            "the status message must name the failure, got {message:?}"
        );
        assert_eq!(
            app.root.system.status_phase,
            Some(OperationPhase::Failed),
            "and it must be an error status, not a healthy one"
        );
    }
}
