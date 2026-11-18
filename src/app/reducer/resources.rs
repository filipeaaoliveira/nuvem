use crate::app::projection::detail_body::{CopyableField, DetailBody, project_detail_body};
use crate::app::service_registry::{load_effect, service_id_for_resource};
use crate::app::state::command::CommandDescriptor;
use crate::app::state::plugins::{Command, Keyed, Offer, PluginId};
use crate::app::state::resources::{
    ActionConfirmationSubmission, AscendOutcome, BigQueryDrillGeneration, BigQueryListingEpoch,
    CloudRunDetailTab, CloudSqlDetailTab, PluginPrompt,
};
use crate::app::state::root::{RootState, Workspace};
use crate::app::state::tunnels::{TunnelEntry, TunnelStatus};
use crate::app::{Effect, ResourcesAction, TunnelLaunchPlan};
use nuvem_domain::gcp::configuration::GcloudContext;
use nuvem_domain::gcp::logging::{LogQuery, LogResourceKind};
use nuvem_domain::gcp::resource::ResourceServiceKind;
use nuvem_domain::gcp::text::{sanitize_clipboard_text, sanitize_display_text};
use nuvem_domain::gcp::tunnel::{
    TunnelKind, cloud_sql_connection_name, compute_tunnel_kind, default_local_port,
    default_remote_port,
};

/// Refuses a pending cloud-resource mutation because read-only mode is on,
/// recording the same explanation the palette's submit gate gives.
///
/// Submit alone is not enough: read-only can come on mid-session (a config
/// reload folds it in monotonically), and a confirmation opened a moment
/// earlier is still one keypress away from the cloud. The same reasoning runs
/// one step further for k9s, whose confirm only starts `get-credentials` — the
/// launch happens later, when the result lands, so the reducer's
/// `K9sCredentialsReady` arm in [`super::root`] calls this too. Submit,
/// confirm, and launch are the three points where a mutation can still be
/// ahead of the mode.
///
/// This is the non-error status path on purpose — the mode refusing a mutation
/// is the mode working, not an operation that failed.
pub(super) fn refuse_under_read_only(state: &mut RootState, what: &str) {
    state.system.set_status_message(format!(
        "{what} is unavailable: read-only mode refuses cloud-resource mutations"
    ));
}

/// The refusal shown when a value bound for a child's argv would be read as an
/// option instead of as data. See [`nuvem_domain::gcp::argv`] for why refusing
/// is the only move available.
///
/// The offending value is quoted into the message because the operator has to
/// go and look at it: nothing they did produced it, so the useful part is
/// which row is malformed, not that nuvem declined.
pub(super) fn refuse_option_like(offender: &str) -> String {
    format!("refusing to launch: \"{offender}\" would be read as an option, not a name")
}

/// Refuses a plugin because read-only mode cannot prove it only reads.
///
/// A separate sentence from [`refuse_under_read_only`] because the mode
/// no longer refuses plugins as a class. `Unproven` means the
/// declaration matched no path in the reading table, and the common way to
/// land there is a plugin that reads perfectly well while naming a command the
/// table does not list. Telling that operator the mode "refuses
/// cloud-resource mutations" states something about their plugin that was
/// never established, and points them at rewriting a declaration that has
/// nothing wrong with it.
///
/// Shorter than the mutation sentence, not merely the same length class. The
/// status line truncates rather than wraps
/// ([`crate::ui::shared::components::fit_text`]),
/// and it truncates from the right — so a longer sentence spends the width on
/// the label and loses "cannot prove", the one clause this split exists to
/// deliver. `a_plugin_refusal_fits_an_eighty_column_status_line` pins the fit
/// so the next reword cannot quietly reintroduce it.
pub(super) fn refuse_plugin_under_read_only(state: &mut RootState, what: &str) {
    state.system.set_status_message(format!(
        "{what} is unavailable: read-only mode cannot prove this only reads"
    ));
}

/// The one entry into running a plugin, from either door: the palette's
/// `Submit` arm and the keyed press.
///
/// The palette's guard (`reducer::command`) fires before its call, and the
/// two overlap for a plugin: a plugin submitted from the palette is
/// asked about twice, once by the `Submit` arm and once here. That is defence
/// in depth rather than a division of labour — the `Submit` arm is bound to
/// `CommandAction::Submit` and also answers for every non-plugin command, and
/// this is the one entry a keyed press can reach. Both ask the same predicate
/// ([`crate::app::state::plugins::plugin_blocked_by_read_only`]), so what they
/// cannot do is disagree about a plugin.
///
/// Each has its own tests, in `tests/plugin_read_only_evidence.rs`: the
/// palette pair (`..._through_the_palettes_door`) kills the `Submit` arm
/// reverting to `read_only && is_mutating()`, which refuses a plugin nuvem can
/// prove reads, and the keyed pair (`..._through_the_keyed_door`,
/// `a_mutating_plugin_is_still_refused_under_read_only`) kills the same
/// mutation here. Deleting the palette's check outright is *not* observable —
/// this refuses the same plugin with the same sentence, which is the overlap
/// working — so the palette's copy is justified by the disagreement it
/// forecloses, not by a behaviour only it produces.
///
/// The refusal comes first, ahead of the match, because the prompt now sits in
/// front of the confirmation: below the match a read-only operator would be
/// asked for every value the plugin declares and told the mode refuses it only
/// on the last keystroke. The test that pins the placement presses a plugin
/// that *needs values*, since a `Ready` offer is refused either way round.
pub(super) fn open_plugin(state: &mut RootState, offer: Offer) {
    if crate::app::state::plugins::plugin_blocked_by_read_only(state, offer.id) {
        // The label the operator was shown, so the refusal names what they
        // pressed rather than a static description of the plugin.
        let label = offer.label.clone();
        refuse_plugin_under_read_only(state, &label);
        return;
    }
    match offer.command {
        Command::Ready(argv) => {
            state
                .resources
                .show_plugin_confirmation(offer.label, argv, offer.dangerous);
        }
        Command::NeedsValues { inputs } => {
            // The row's fields are snapshotted into the prompt rather than
            // re-read on submit: the modal owns the keyboard, so reading them
            // here is what makes the argv the operator approves and the argv
            // that runs the same object even if a refresh lands underneath.
            let service = state.resources.active_service();
            let Some(fields) = crate::app::state::jumps::selected_fields(&state.resources, service)
            else {
                // Silent, unlike every other refusal on this path: both doors
                // resolved an offer against a selection a moment ago, so
                // reaching here means the selection vanished between the two
                // reads, and there is no gesture left to explain.
                return;
            };
            state.resources.show_plugin_prompt(PluginPrompt::new(
                offer.id,
                offer.label,
                offer.dangerous,
                inputs,
                fields,
            ));
        }
    }
}

/// Objects requested per Cloud Storage object-listing page.
const STORAGE_PAGE_SIZE: u32 = 100;

/// Reported when `L` (or a palette log jump) fires with no resolved gcloud
/// context: without one there is no project to scope logs to, and
/// the recent-logs load would emit nothing at all.
const NO_CONTEXT_LOGS_STATUS: &str = "No active gcloud context to open logs for";

/// Reported when the active context resolves but carries no project. Cloud SQL
/// log entries are keyed by a composite `database_id` of the form
/// `<project>:<instance>`, so an empty project yields a filter that matches
/// zero rows — indistinguishable, on screen, from "this instance has no logs".
/// Refusing loudly is the only honest option.
const NO_CLOUD_SQL_PROJECT_LOGS_STATUS: &str =
    "Active gcloud context has no project; cannot scope Cloud SQL logs";

/// The Secret Manager counterpart of [`NO_CLOUD_SQL_PROJECT_LOGS_STATUS`]: a
/// secret's audit entries are matched by the full resource name
/// `projects/<project>/secrets/<name>`, so an empty project builds a filter
/// that cannot match anything and reads on screen as "this secret has no audit
/// trail".
const NO_SECRET_PROJECT_LOGS_STATUS: &str =
    "Active gcloud context has no project; cannot scope Secret Manager logs";

fn nothing_selected(noun: &str) -> String {
    format!("No {noun} selected to open logs for")
}

/// Resolves the resource the current view actually has on screen to a
/// [`LogQuery`], or an error message naming what was missing.
///
/// Two rules drive the resolution, and both exist so the query can never
/// describe something the user is not looking at:
///
/// * the nested Cloud Run revision view owns the pane, so its revision — not
///   the parent service — is "the selection" there;
/// * with a detail pane open the list cursor is still movable (`j`/`k` reach
///   the collection behind the pane), so the *detail target* is authoritative,
///   not the list selection.
///
/// The second rule is answered once, by `ResourcesState::current_*_target`
/// (and `StorageState::view_bucket`), rather than re-derived here: the `J`
/// related-resource jump asks the same question, and two copies of the rule
/// would let `L` and `J` name different resources.
fn selection_log_query(state: &RootState, context: &GcloudContext) -> Result<LogQuery, String> {
    let resources = &state.resources;
    let project = context.project.clone();

    if resources.is_showing_cloud_run_revision() {
        let revision = resources
            .cloud_run_revision_target()
            .ok_or_else(|| nothing_selected("Cloud Run revision"))?;
        let service = resources
            .cloud_run_detail_target()
            .ok_or_else(|| nothing_selected("Cloud Run service"))?;
        let region = resources
            .cloud_run_revision_region()
            .unwrap_or(service.region.as_str())
            .to_string();
        return Ok(LogQuery::Resource(LogResourceKind::CloudRunRevision {
            service_name: service.name.clone(),
            revision_name: revision.name.clone(),
            region,
            project: project.unwrap_or_default(),
        }));
    }

    let service = resources.active_service();
    let noun = service.selection_noun();
    match service {
        ResourceServiceKind::ComputeInstances => {
            let instance = resources
                .current_compute_target()
                .ok_or_else(|| nothing_selected(noun))?;
            Ok(LogQuery::Resource(LogResourceKind::ComputeInstance {
                instance_id: instance.id.clone(),
                instance_name: instance.name.clone(),
                zone: instance.zone.clone(),
            }))
        }
        ResourceServiceKind::GkeClusters => {
            let cluster = resources
                .current_gke_target()
                .ok_or_else(|| nothing_selected(noun))?;
            Ok(LogQuery::Resource(LogResourceKind::GkeCluster {
                cluster_name: cluster.name.clone(),
                location: cluster.location.clone(),
                project: project.unwrap_or_default(),
            }))
        }
        ResourceServiceKind::CloudSql => {
            let instance = resources
                .current_cloud_sql_target()
                .ok_or_else(|| nothing_selected(noun))?;
            // The project is load-bearing here, not decorative: see
            // `NO_CLOUD_SQL_PROJECT_LOGS_STATUS`.
            let project = project.ok_or(NO_CLOUD_SQL_PROJECT_LOGS_STATUS.to_string())?;
            Ok(LogQuery::Resource(LogResourceKind::CloudSqlInstance {
                instance_name: instance.name.clone(),
                project,
            }))
        }
        ResourceServiceKind::CloudRunServices => {
            let run_service = resources
                .current_cloud_run_target()
                .ok_or_else(|| nothing_selected(noun))?;
            Ok(LogQuery::Resource(LogResourceKind::CloudRunService {
                service_name: run_service.name.clone(),
                region: run_service.region.clone(),
                project: project.unwrap_or_default(),
            }))
        }
        ResourceServiceKind::CloudStorage => {
            let bucket = resources
                .storage()
                .view_bucket()
                .ok_or_else(|| nothing_selected(noun))?;
            Ok(LogQuery::Resource(LogResourceKind::StorageBucket {
                bucket_name: bucket.name.clone(),
            }))
        }
        // Both generations answer through one query — see
        // `LogResourceKind::CloudFunction` for why the environment field is
        // not consulted. No detail pane yet, so the list cursor is the whole
        // selection rule.
        ResourceServiceKind::CloudFunctions => {
            let function = resources
                .selected_cloud_function()
                .ok_or_else(|| nothing_selected(noun))?;
            Ok(LogQuery::Resource(LogResourceKind::CloudFunction {
                function_name: function.name.clone(),
                region: function.region.clone(),
            }))
        }
        // A persistent disk is not a log-producing resource: nothing writes
        // logs *as* a disk. Its lifecycle shows up in the audit log of the
        // instance or the operation that touched it, and pointing `L` at a
        // resource-scoped query that can only ever return zero rows would read
        // on screen as "this disk has no logs" rather than as "disks do not
        // have logs".
        ResourceServiceKind::PersistentDisks => {
            Err("Persistent disks do not produce logs".to_string())
        }
        // No detail pane to be the authoritative target in this slice, so the
        // list cursor is the whole rule here.
        ResourceServiceKind::PubSubTopics => {
            let topic = resources
                .selected_pubsub_topic()
                .ok_or_else(|| nothing_selected(noun))?;
            Ok(LogQuery::Resource(LogResourceKind::PubSubTopic {
                topic_name: topic.name.clone(),
            }))
        }
        // A VPC network is not a log-producing resource, for the reason a
        // persistent disk is not: nothing writes logs *as* a network. The
        // entries an operator is actually after here are VPC flow logs and
        // firewall hit logs, and Cloud Logging attributes both to the
        // *subnetwork* — `resource.type="gce_subnetwork"` — never to the
        // network. Scoping `L` to the network would issue a query that can
        // only return zero rows and read on screen as "this network has no
        // traffic", which is the most misleading answer this view could give.
        ResourceServiceKind::VpcNetworks => Err("VPC networks do not produce logs".to_string()),
        // The one VPC resource that *does* produce logs, and the reason the
        // networks table refuses: flow logs and firewall hit logs are both
        // attributed to `gce_subnetwork`. `L` here is the query an operator
        // pressing it on a network was actually reaching for.
        ResourceServiceKind::VpcSubnets => {
            let subnet = resources
                .selected_vpc_subnet()
                .ok_or_else(|| nothing_selected(noun))?;
            Ok(LogQuery::Resource(LogResourceKind::VpcSubnet {
                subnet_name: subnet.name.clone(),
            }))
        }
        // A firewall rule is not a log-producing resource, and this is the
        // third VPC table to answer the question differently. Cloud Logging
        // has no `gce_firewall_rule` stream: a rule's *hits* are logged
        // against the subnetwork the traffic crossed, and its edits appear in
        // the project's audit log as operations on `compute.firewalls`. So the
        // entries an operator wants after reading this table are one table
        // over, which is exactly where `L` does work.
        ResourceServiceKind::FirewallRules => Err(
            "Firewall rules do not produce logs; a rule's hits are logged \
                 against the subnetwork the traffic crossed"
                .to_string(),
        ),
        ResourceServiceKind::PubSubSubscriptions => {
            let subscription = resources
                .selected_pubsub_subscription()
                .ok_or_else(|| nothing_selected(noun))?;
            Ok(LogQuery::Resource(LogResourceKind::PubSubSubscription {
                subscription_name: subscription.name.clone(),
            }))
        }
        // List-only in this slice, so the list cursor is the whole selection
        // rule. What the query reaches is the dataset's audit trail — see
        // `LogResourceKind::BigQueryDataset` for why query jobs are not in it.
        ResourceServiceKind::BigQueryDatasets => {
            let dataset = resources
                .selected_bigquery_dataset()
                .ok_or_else(|| nothing_selected(noun))?;
            Ok(LogQuery::Resource(LogResourceKind::BigQueryDataset {
                dataset_id: dataset.dataset_id.clone(),
            }))
        }
        // A service account is not a log-producing resource: it is an
        // identity that appears *in* log entries as the caller, not a
        // resource with a log stream of its own. A resource-scoped query
        // keyed on it could only ever return zero rows, which would read on
        // screen as "this account has no logs" rather than as "accounts do
        // not have logs".
        ResourceServiceKind::IamServiceAccounts => {
            Err("Service accounts do not produce logs".to_string())
        }
        ResourceServiceKind::IamPolicy => Err("Policy bindings do not produce logs".to_string()),
        // A secret's logs are its audit trail — metadata access, never payload
        // material, which nothing here fetches in the first place.
        ResourceServiceKind::SecretManagerSecrets => {
            let secret = resources
                .secret_detail_target()
                .or(resources.selected_secret())
                .ok_or_else(|| nothing_selected(noun))?;
            // The project is interpolated into the matched resource name, not
            // merely carried alongside it: see `NO_SECRET_PROJECT_LOGS_STATUS`.
            let project = project.ok_or(NO_SECRET_PROJECT_LOGS_STATUS.to_string())?;
            Ok(LogQuery::Resource(LogResourceKind::Secret {
                secret_name: secret.name.clone(),
                project,
            }))
        }
    }
}

/// Opens the Logging workspace scoped to the current selection.
///
/// The single resolver behind both the `L` key and every palette
/// `OpenLogsForSelected*` command, so the two can never disagree about what
/// "the selection" is. Follows the log viewer's load shape: narrowing identity
/// rides on `Effect::LoadRecentLogs` and is validated where the result is
/// applied, so there is deliberately no keypress-site in-flight guard here
/// (that shape was rejected earlier).
pub(super) fn open_logs_for_selection(state: &mut RootState) -> Vec<Effect> {
    let Some(context) = state.current_context() else {
        state
            .system
            .set_error_status_message(NO_CONTEXT_LOGS_STATUS);
        return Vec::new();
    };
    match selection_log_query(state, &context) {
        Ok(query) => {
            state.workspace = Workspace::Logging;
            state.logging.open_query(query);
            super::logging::load_recent_or_settle(state, super::logging::UnloadedBuffer::Discard)
        }
        Err(message) => {
            state.system.set_error_status_message(message);
            Vec::new()
        }
    }
}

/// Moves the detail-pane field cursor by one, wrapping at both ends. From no
/// selection, `forward` picks the first field and backward the last. A pane
/// with no copyable fields (none open, payload not arrived, or a JSON tab)
/// leaves the cursor untouched.
fn move_detail_field(state: &mut RootState, forward: bool) -> Vec<Effect> {
    let Some(count) = project_detail_body(&state.resources)
        .map(|body| body.fields().len())
        .filter(|count| *count > 0)
    else {
        return Vec::new();
    };
    // The stored index is clamped first: a body can shrink under it when a
    // detail payload reloads, and a wrap must still land inside the pane.
    let next = match (state.resources.detail_field_index(), forward) {
        (None, true) => 0,
        (None, false) => count - 1,
        (Some(current), true) => (current.min(count - 1) + 1) % count,
        (Some(current), false) => (current.min(count - 1) + count - 1) % count,
    };
    state.resources.set_detail_field_index(Some(next));
    Vec::new()
}

/// The copyable field under the cursor of `body`, clamped against the field
/// list actually on screen so a stale index can never resolve to the wrong
/// field (or past the end).
fn selected_detail_field(state: &RootState, body: &DetailBody) -> Option<CopyableField> {
    let index = state.resources.detail_field_index()?;
    let fields = body.fields();
    let (_, field) = fields.get(index).or_else(|| fields.last())?;
    Some((*field).clone())
}

/// The far side of an instance tunnel is always sshd.
const DEFAULT_SSH_REMOTE_PORT: u16 = 22;

/// Reads the active service's applied filter as an owned option, trimmed and
/// collapsed to `None` when empty (so an empty draft clears any server-side
/// `--filter` and falls back to a full, unfiltered reload).
///
/// A convenience over [`active_filter_for`] for the sites that reload the
/// active service *by construction* — the `Refresh` and `ApplyFilter` arms
/// below, which have already matched on `active_service()`, and the runtime's
/// background poll (`services::runtime::session`). Public because that poll
/// lives outside the reducer and the trim/empty-collapse rule must have
/// exactly one definition.
pub fn active_filter(state: &RootState) -> Option<String> {
    active_filter_for(state, state.resources.active_service())
}

/// The applied filter to push when reloading `service`: that service's *own*
/// applied filter, trimmed and collapsed to `None` when empty — whether or not
/// `service` happens to be the active one.
///
/// Both halves matter. Passing some *other* service's filter would leak a
/// cross-service term and narrow the wrong stored list against a filter the
/// user never applied to it. Passing `None` for a service that does have a
/// filter applied is just as wrong: the reload *replaces* that service's
/// stored rows, and in label mode there is no client-side narrowing left to
/// re-apply (`CompiledFilter::Label` keeps every row), so the downloaded set
/// simply becomes the visible set while the tools line still asserts the
/// filter is in force. That holds for a reload of a service the user has since
/// switched away from too: nothing reloads it on the way back (`needs_refresh`
/// is false once the context matches and `stale` is clear), so the widened
/// list would persist until the next poll tick — and forever under
/// `--poll-interval 0`.
///
/// `filter_applied` is per-service state, so the right value is always
/// available; see `ResourcesState::filter_applied_for`.
pub fn active_filter_for(state: &RootState, service: ResourceServiceKind) -> Option<String> {
    let f = state.resources.filter_applied_for(service).trim();
    (!f.is_empty()).then(|| f.to_string())
}

/// When the active Cloud SQL detail switches to the Users tab and its users
/// have not been loaded yet, emit a lazy load for the current instance. This
/// is the sole users fetch trigger; the Users tab is never eager-loaded.
pub(super) fn cloud_sql_users_lazy_load(state: &mut RootState) -> Vec<Effect> {
    if state.resources.active_service() != ResourceServiceKind::CloudSql {
        return Vec::new();
    }
    if state.resources.active_cloud_sql_detail_tab() != CloudSqlDetailTab::Users {
        return Vec::new();
    }
    if state.resources.cloud_sql_detail_users_loaded() {
        return Vec::new();
    }
    match (
        state.current_context(),
        state.resources.cloud_sql_detail_target().cloned(),
    ) {
        (Some(context), Some(instance)) => vec![Effect::LoadCloudSqlUsers { context, instance }],
        _ => Vec::new(),
    }
}

/// One `bq show` per table the landed listing put on screen.
///
/// Called from the `BigQueryTablesLoaded` arm, after that arm has accepted the
/// listing — the rows this reads are the ones it just installed. A dataset
/// with 200 tables produces 200 effects here and nothing caps that; see
/// `ResourcesState::bigquery_table_stat_requests` for why a cap was refused.
///
/// `context`, `generation` and `listing` are the accepted result's, all three
/// checked against the live ones by the caller: the shows go out for the
/// project the rows came from rather than for whatever the picker moved to in
/// between, and each one names the listing it was fanned out from.
pub(super) fn bigquery_table_stats_fan_out(
    state: &RootState,
    context: &GcloudContext,
    generation: BigQueryDrillGeneration,
    listing: BigQueryListingEpoch,
) -> Vec<Effect> {
    state
        .resources
        .bigquery_table_stat_requests(generation, listing)
        .into_iter()
        .map(|request| Effect::LoadBigQueryTableStats {
            context: context.clone(),
            dataset_id: request.dataset_id,
            table_id: request.table_id,
            generation: request.generation,
            listing: request.listing,
        })
        .collect()
}

pub fn reduce(state: &mut RootState, action: &ResourcesAction) -> Vec<Effect> {
    match action {
        ResourcesAction::SelectService(service) => {
            let changed = state.resources.active_service() != *service;
            state.resources.select_service(*service);
            if !changed {
                return Vec::new();
            }

            let Some(id) = service_id_for_resource(*service) else {
                return Vec::new();
            };
            // The service being loaded is the one whose filter gets pushed.
            let filter = active_filter_for(state, *service);
            state
                .current_context()
                .and_then(|context| load_effect(id, context, filter))
                .into_iter()
                .collect()
        }
        ResourcesAction::Refresh => {
            match (state.resources.active_service(), state.current_context()) {
                (ResourceServiceKind::ComputeInstances, Some(context)) => {
                    let mut effects = vec![Effect::RefreshComputeInstances {
                        context: context.clone(),
                        filter: active_filter(state),
                    }];
                    if state.resources.is_showing_detail()
                        && let Some(instance) = state.resources.detail_target().cloned()
                    {
                        effects.push(Effect::LoadComputeInstanceDetail { context, instance });
                    }
                    effects
                }
                (ResourceServiceKind::GkeClusters, Some(context)) => {
                    let mut effects = vec![Effect::RefreshGkeClusters {
                        context: context.clone(),
                        filter: active_filter(state),
                    }];
                    if state.resources.is_showing_detail()
                        && let Some(cluster) = state.resources.gke_detail_target().cloned()
                    {
                        effects.push(Effect::LoadGkeClusterDetail { context, cluster });
                    }
                    effects
                }
                (ResourceServiceKind::CloudSql, Some(context)) => {
                    let mut effects = vec![Effect::RefreshCloudSqlInstances {
                        context: context.clone(),
                        filter: active_filter(state),
                    }];
                    if state.resources.is_showing_detail()
                        && let Some(instance) = state.resources.cloud_sql_detail_target().cloned()
                    {
                        effects.push(Effect::LoadCloudSqlInstanceDetail { context, instance });
                    }
                    effects
                }
                (ResourceServiceKind::CloudRunServices, Some(context)) => {
                    let mut effects = vec![Effect::RefreshCloudRunServices {
                        context: context.clone(),
                    }];
                    if state.resources.is_showing_detail()
                        && let Some(service) = state.resources.cloud_run_detail_target().cloned()
                    {
                        effects.push(Effect::LoadCloudRunServiceDetail {
                            context: context.clone(),
                            service: service.clone(),
                        });
                        effects.push(Effect::LoadCloudRunRevisions {
                            context: context.clone(),
                            service,
                        });
                    }
                    if state.resources.is_showing_cloud_run_revision()
                        && let (Some(revision), Some(region)) = (
                            state.resources.cloud_run_revision_target().cloned(),
                            state
                                .resources
                                .cloud_run_revision_region()
                                .map(str::to_string),
                        )
                    {
                        effects.push(Effect::LoadCloudRunRevisionDetail {
                            context,
                            region,
                            revision: revision.name,
                        });
                    }
                    effects
                }
                // No detail pane to reload alongside the list.
                (ResourceServiceKind::PersistentDisks, Some(context)) => {
                    vec![Effect::RefreshPersistentDisks {
                        context,
                        filter: active_filter(state),
                    }]
                }
                (ResourceServiceKind::PubSubTopics, Some(context)) => {
                    vec![Effect::RefreshPubSubTopics {
                        context,
                        filter: active_filter(state),
                    }]
                }
                (ResourceServiceKind::VpcNetworks, Some(context)) => {
                    vec![Effect::RefreshVpcNetworks { context }]
                }
                (ResourceServiceKind::VpcSubnets, Some(context)) => {
                    vec![Effect::RefreshVpcSubnets { context }]
                }
                (ResourceServiceKind::FirewallRules, Some(context)) => {
                    vec![Effect::RefreshFirewallRules { context }]
                }
                (ResourceServiceKind::PubSubSubscriptions, Some(context)) => {
                    vec![Effect::RefreshPubSubSubscriptions {
                        context,
                        filter: active_filter(state),
                    }]
                }
                // With a drill open, `r` retries the call *that* view is
                // showing as well — the same shape the detail-pane services
                // above have, where the refresh reloads the pane on screen and
                // not only the list behind it. The failure state's "Press r to
                // retry." is a claim about this arm.
                //
                // At most one of the two retries fires: each returns `None`
                // unless its own level is the view on screen, so which call is
                // retried is asked of the state rather than decided by a view
                // test written here. Two levels down that is the `bq show` for
                // the table entered — a `bq ls` issued from there would land a
                // tables listing while the operator is reading columns.
                (ResourceServiceKind::BigQueryDatasets, Some(context)) => {
                    let mut effects = vec![Effect::RefreshBigQueryDatasets {
                        context: context.clone(),
                    }];
                    if let Some(request) = state.resources.reload_bigquery_tables() {
                        effects.push(Effect::LoadBigQueryTables {
                            context: context.clone(),
                            dataset_id: request.dataset_id,
                            generation: request.generation,
                            listing: request.listing,
                        });
                    }
                    if let Some(request) = state.resources.reload_bigquery_schema() {
                        effects.push(Effect::LoadBigQuerySchema {
                            context,
                            dataset_id: request.dataset_id,
                            table_id: request.table_id,
                            generation: request.generation,
                        });
                    }
                    effects
                }
                (ResourceServiceKind::IamServiceAccounts, Some(context)) => {
                    vec![Effect::RefreshServiceAccounts {
                        context,
                        filter: active_filter(state),
                    }]
                }
                (ResourceServiceKind::IamPolicy, Some(context)) => {
                    vec![Effect::RefreshIamPolicy { context }]
                }
                // No detail pane to reload alongside the list.
                (ResourceServiceKind::CloudFunctions, Some(context)) => {
                    vec![Effect::RefreshCloudFunctions {
                        context,
                        filter: active_filter(state),
                    }]
                }
                (ResourceServiceKind::SecretManagerSecrets, Some(context)) => {
                    let mut effects = vec![Effect::RefreshSecrets {
                        context: context.clone(),
                        filter: active_filter(state),
                    }];
                    if state.resources.is_showing_detail()
                        && let Some(secret) = state.resources.secret_detail_target().cloned()
                    {
                        effects.push(Effect::LoadSecretDetail {
                            context: context.clone(),
                            secret: secret.clone(),
                        });
                        effects.push(Effect::LoadSecretVersions { context, secret });
                    }
                    effects
                }
                (ResourceServiceKind::CloudStorage, Some(context)) => {
                    use crate::app::state::resources::StorageView;
                    let mut effects = vec![Effect::RefreshStorageBuckets {
                        context: context.clone(),
                    }];
                    if matches!(
                        state.resources.storage().view(),
                        StorageView::ObjectBrowser | StorageView::ObjectDetail
                    ) && let Some((bucket, prefix)) =
                        state.resources.storage().browser_page_request()
                    {
                        effects.push(Effect::LoadStorageObjectPage {
                            context,
                            bucket,
                            prefix,
                            page_token: None,
                            page_size: STORAGE_PAGE_SIZE,
                        });
                    }
                    effects
                }
                (_, None) => Vec::new(),
            }
        }
        ResourcesAction::SelectNext => {
            state.resources.select_next();
            Vec::new()
        }
        ResourcesAction::SelectPrevious => {
            state.resources.select_previous();
            Vec::new()
        }
        ResourcesAction::OpenSelected => match state.resources.active_service() {
            ResourceServiceKind::CloudRunServices => {
                if !state.resources.is_showing_detail() {
                    match (
                        state.current_context(),
                        state.resources.open_cloud_run_detail(),
                    ) {
                        (Some(context), Some(service)) => vec![
                            Effect::LoadCloudRunServiceDetail {
                                context: context.clone(),
                                service: service.clone(),
                            },
                            Effect::LoadCloudRunRevisions { context, service },
                        ],
                        _ => Vec::new(),
                    }
                } else if state.resources.active_cloud_run_detail_tab()
                    == CloudRunDetailTab::Revisions
                {
                    let context = state.current_context();
                    let revision = state.resources.selected_cloud_run_revision().cloned();
                    let region = state
                        .resources
                        .cloud_run_detail_target()
                        .map(|service| service.region.clone());
                    match (context, revision, region) {
                        (Some(context), Some(revision), Some(region)) => {
                            state
                                .resources
                                .open_cloud_run_revision(revision.clone(), region.clone());
                            vec![Effect::LoadCloudRunRevisionDetail {
                                context,
                                region,
                                revision: revision.name,
                            }]
                        }
                        _ => Vec::new(),
                    }
                } else {
                    Vec::new()
                }
            }
            ResourceServiceKind::ComputeInstances => {
                if state.resources.is_showing_detail() {
                    Vec::new()
                } else {
                    match (state.current_context(), state.resources.open_detail()) {
                        (Some(context), Some(instance)) => {
                            vec![Effect::LoadComputeInstanceDetail { context, instance }]
                        }
                        _ => Vec::new(),
                    }
                }
            }
            ResourceServiceKind::GkeClusters => {
                if state.resources.is_showing_detail() {
                    Vec::new()
                } else {
                    match (state.current_context(), state.resources.open_gke_detail()) {
                        (Some(context), Some(cluster)) => {
                            vec![Effect::LoadGkeClusterDetail { context, cluster }]
                        }
                        _ => Vec::new(),
                    }
                }
            }
            ResourceServiceKind::CloudSql => {
                match (
                    state.current_context(),
                    state.resources.open_cloud_sql_detail(),
                ) {
                    (Some(context), Some(instance)) => {
                        vec![Effect::LoadCloudSqlInstanceDetail { context, instance }]
                    }
                    _ => Vec::new(),
                }
            }
            ResourceServiceKind::CloudStorage => open_selected_storage(state),
            // `Enter` on a dataset drills into its tables, and `Enter` on a
            // table drills into its columns. Which of the two this is comes
            // from the two openers rather than from a view test written here:
            // each returns `None` — and transitions nothing — unless its own
            // level is the one on screen, so calling them in order asks the
            // state which level it is in exactly once. That is also what makes
            // `Enter` on a *column* row a no-op: neither opener claims it, and
            // a column is the bottom of this hierarchy.
            ResourceServiceKind::BigQueryDatasets => {
                // The context is resolved before either transition, not after:
                // entering a view is what promises the call that fills it, and
                // without a context there is no call to make. Drilling in
                // anyway would leave the view loading forever.
                let Some(context) = state.current_context() else {
                    return Vec::new();
                };
                if let Some(request) = state.resources.open_selected_bigquery_dataset() {
                    return vec![Effect::LoadBigQueryTables {
                        context,
                        dataset_id: request.dataset_id,
                        generation: request.generation,
                        listing: request.listing,
                    }];
                }
                match state.resources.open_selected_bigquery_table() {
                    Some(request) => vec![Effect::LoadBigQuerySchema {
                        context,
                        dataset_id: request.dataset_id,
                        table_id: request.table_id,
                        generation: request.generation,
                    }],
                    None => Vec::new(),
                }
            }
            // Nothing to open: none of Cloud Functions, persistent disks, the
            // two Pub/Sub tables or service accounts has a detail pane yet, so
            // Enter on a row is a quiet no-op rather than an empty pane.
            ResourceServiceKind::CloudFunctions
            | ResourceServiceKind::PersistentDisks
            | ResourceServiceKind::PubSubTopics
            | ResourceServiceKind::VpcNetworks
            | ResourceServiceKind::VpcSubnets
            | ResourceServiceKind::FirewallRules
            | ResourceServiceKind::PubSubSubscriptions
            | ResourceServiceKind::IamServiceAccounts
            | ResourceServiceKind::IamPolicy => Vec::new(),
            // Opening a secret loads its metadata and, in the same step, its
            // version list — the Versions tab is a click away and the count on
            // the Summary tab comes from it.
            ResourceServiceKind::SecretManagerSecrets => {
                if state.resources.is_showing_detail() {
                    Vec::new()
                } else {
                    match (
                        state.current_context(),
                        state.resources.open_secret_detail(),
                    ) {
                        (Some(context), Some(secret)) => vec![
                            Effect::LoadSecretDetail {
                                context: context.clone(),
                                secret: secret.clone(),
                            },
                            Effect::LoadSecretVersions { context, secret },
                        ],
                        _ => Vec::new(),
                    }
                }
            }
        },
        ResourcesAction::CloseDetail => {
            state.resources.close_detail();
            Vec::new()
        }
        ResourcesAction::ScrollDetailDown => {
            state.resources.scroll_detail_down();
            Vec::new()
        }
        ResourcesAction::ScrollDetailUp => {
            state.resources.scroll_detail_up();
            Vec::new()
        }
        ResourcesAction::SelectNextTab => {
            state.resources.select_next_tab();
            cloud_sql_users_lazy_load(state)
        }
        ResourcesAction::SelectPreviousTab => {
            state.resources.select_previous_tab();
            cloud_sql_users_lazy_load(state)
        }
        ResourcesAction::SelectNextDetailField => move_detail_field(state, true),
        ResourcesAction::SelectPreviousDetailField => move_detail_field(state, false),
        ResourcesAction::StartFilter => {
            state.resources.start_filter();
            Vec::new()
        }
        ResourcesAction::AppendFilterChar(value) => {
            state.resources.append_filter_char(*value);
            Vec::new()
        }
        ResourcesAction::BackspaceFilter => {
            state.resources.backspace_filter();
            Vec::new()
        }
        ResourcesAction::ApplyFilter => {
            let storage_reload = state.resources.apply_filter();
            match (storage_reload, state.current_context()) {
                (Some((bucket, prefix)), Some(context)) => vec![Effect::LoadStorageObjectPage {
                    context,
                    bucket,
                    prefix,
                    page_token: None,
                    page_size: STORAGE_PAGE_SIZE,
                }],
                (None, Some(context)) => match state.resources.active_service() {
                    ResourceServiceKind::ComputeInstances => {
                        vec![Effect::RefreshComputeInstances {
                            context,
                            filter: active_filter(state),
                        }]
                    }
                    ResourceServiceKind::GkeClusters => vec![Effect::RefreshGkeClusters {
                        context,
                        filter: active_filter(state),
                    }],
                    ResourceServiceKind::CloudSql => vec![Effect::RefreshCloudSqlInstances {
                        context,
                        filter: active_filter(state),
                    }],
                    ResourceServiceKind::CloudRunServices => {
                        vec![Effect::RefreshCloudRunServices { context }]
                    }
                    ResourceServiceKind::SecretManagerSecrets => {
                        vec![Effect::RefreshSecrets {
                            context,
                            filter: active_filter(state),
                        }]
                    }
                    ResourceServiceKind::PubSubTopics => {
                        vec![Effect::RefreshPubSubTopics {
                            context,
                            filter: active_filter(state),
                        }]
                    }
                    // No re-fetch on a filter change, for the reason the
                    // BigQuery arm below records: `compute networks list` has
                    // nothing this filter could narrow the request with, since
                    // MODE, ROUTING and SUBNETS are all derived and appear
                    // verbatim in no raw field. The rows already in state are
                    // simply re-narrowed client-side.
                    //
                    // Refetching here would be worse than wasteful. A failed
                    // refresh runs `mark_active_stale_and_clear`, which drops
                    // the rows and the context — so a transient error on a call
                    // that was never needed would blank a table the operator is
                    // reading, in response to them typing a filter.
                    ResourceServiceKind::VpcNetworks => Vec::new(),
                    // No re-fetch on a filter change, for the reason the
                    // networks arm above records: nothing in this filter can
                    // narrow `compute networks subnets list`, and a failed
                    // refresh clears the table — so a redundant call turns a
                    // transient error into a blank screen the operator caused
                    // by typing.
                    ResourceServiceKind::VpcSubnets => Vec::new(),
                    // No re-fetch on a filter change, for the reason the two
                    // VPC arms above record — and the sharpest instance of it.
                    // `/DENY` matches a word no raw field contains, so a
                    // pushed clause would return nothing; and a failed refresh
                    // clears the table, so a redundant call would blank an
                    // operator's view of what a project blocks at the moment
                    // they asked what it blocks.
                    ResourceServiceKind::FirewallRules => Vec::new(),
                    ResourceServiceKind::PubSubSubscriptions => {
                        vec![Effect::RefreshPubSubSubscriptions {
                            context,
                            filter: active_filter(state),
                        }]
                    }
                    // No re-fetch on a filter change: `bq ls` accepts no
                    // server-side filter expression, so there is nothing a new
                    // filter could narrow the request with and the same rows
                    // are simply re-narrowed client-side.
                    ResourceServiceKind::BigQueryDatasets => Vec::new(),
                    ResourceServiceKind::IamServiceAccounts => {
                        vec![Effect::RefreshServiceAccounts {
                            context,
                            filter: active_filter(state),
                        }]
                    }
                    // No filter argument: the policy read fetches one object
                    // and flattens it, so there is nothing to push down.
                    ResourceServiceKind::IamPolicy => {
                        vec![Effect::RefreshIamPolicy { context }]
                    }
                    ResourceServiceKind::CloudFunctions => {
                        vec![Effect::RefreshCloudFunctions {
                            context,
                            filter: active_filter(state),
                        }]
                    }
                    ResourceServiceKind::PersistentDisks => {
                        vec![Effect::RefreshPersistentDisks {
                            context,
                            filter: active_filter(state),
                        }]
                    }
                    ResourceServiceKind::CloudStorage => Vec::new(),
                },
                _ => Vec::new(),
            }
        }
        ResourcesAction::CancelFilter => {
            state.resources.cancel_filter();
            Vec::new()
        }
        ResourcesAction::SelectPreviousColumn => {
            state.resources.select_previous_column();
            Vec::new()
        }
        ResourcesAction::SelectNextColumn => {
            state.resources.select_next_column();
            Vec::new()
        }
        ResourcesAction::SortBySelectedColumn => {
            state.resources.sort_selected_column();
            Vec::new()
        }
        ResourcesAction::ToggleWideColumns => {
            state.resources.toggle_wide_columns();
            Vec::new()
        }
        ResourcesAction::ConfirmPendingAction if state.read_only => {
            state.resources.cancel_action_confirmation();
            refuse_under_read_only(state, "This action");
            Vec::new()
        }
        ResourcesAction::ConfirmPendingAction => state
            .resources
            .action_confirmation_submission()
            .map(|submission| match submission {
                ActionConfirmationSubmission::Compute {
                    action,
                    context,
                    instance,
                } => {
                    // The entry's id travels with the effect: its result is what
                    // settles this entry, and nothing else identifies it —
                    // another action on the same instance can be pending too.
                    let operation_id = state
                        .system
                        .operations
                        .begin(action.label(), instance.name.clone());
                    vec![Effect::ComputeInstanceAction {
                        context,
                        instance,
                        action,
                        operation_id,
                    }]
                }
                ActionConfirmationSubmission::CloudSql {
                    action,
                    context,
                    instance,
                } => {
                    let operation_id = state
                        .system
                        .operations
                        .begin(action.label(), instance.name.clone());
                    vec![Effect::CloudSqlInstanceAction {
                        context,
                        instance,
                        action,
                        operation_id,
                    }]
                }
            })
            .unwrap_or_default(),
        ResourcesAction::CancelPendingAction => {
            state.resources.cancel_action_confirmation();
            Vec::new()
        }
        ResourcesAction::CopyPendingCommand => {
            let preview = state.resources.command_preview().cloned();
            state.resources.cancel_command_preview();
            preview
                .map(|preview| {
                    vec![Effect::CopyToClipboard {
                        value: preview.command_preview(),
                        description: preview.copy_description().to_string(),
                    }]
                })
                .unwrap_or_default()
        }
        ResourcesAction::CancelPendingCommandPreview => {
            state.resources.cancel_command_preview();
            Vec::new()
        }
        ResourcesAction::ConfirmSshLaunch if state.read_only => {
            state.resources.cancel_ssh_launch_preview();
            refuse_under_read_only(
                state,
                CommandDescriptor::SshSelectedComputeInstance
                    .label()
                    .as_ref(),
            );
            Vec::new()
        }
        ResourcesAction::ConfirmSshLaunch => state
            .resources
            .ssh_launch_submission()
            .map(|(context, instance)| {
                state.resources.cancel_ssh_launch_preview();
                vec![Effect::LaunchInteractiveProcess {
                    command: crate::app::effect::InteractiveCommand::GceSsh { context, instance },
                }]
            })
            .unwrap_or_default(),
        ResourcesAction::CancelSshLaunch => {
            state.resources.cancel_ssh_launch_preview();
            state
                .system
                .set_status_message("SSH launch cancelled".to_string());
            Vec::new()
        }
        ResourcesAction::RequestTunnel => {
            request_tunnel(state);
            Vec::new()
        }
        ResourcesAction::ConfirmTunnelLaunch { started_at } => {
            confirm_tunnel_launch(state, *started_at)
        }
        ResourcesAction::CancelTunnelLaunch => {
            state.resources.cancel_tunnel_launch_preview();
            state
                .system
                .set_status_message("tunnel launch cancelled".to_string());
            Vec::new()
        }
        ResourcesAction::ConfirmK9sLaunch if state.read_only => {
            state.resources.cancel_k9s_launch_confirmation();
            refuse_under_read_only(
                state,
                CommandDescriptor::OpenSelectedGkeClusterInK9s
                    .label()
                    .as_ref(),
            );
            Vec::new()
        }
        ResourcesAction::ConfirmK9sLaunch => state
            .resources
            .k9s_launch_submission()
            .map(|plan| vec![Effect::RunK9sCredentials { plan }])
            .unwrap_or_default(),
        ResourcesAction::CancelK9sLaunch => {
            state.resources.cancel_k9s_launch_confirmation();
            state
                .system
                .set_status_message("k9s launch cancelled".to_string());
            Vec::new()
        }
        ResourcesAction::ToggleMark => {
            state.resources.toggle_mark();
            Vec::new()
        }
        ResourcesAction::RangeMark => {
            state.resources.range_mark();
            Vec::new()
        }
        ResourcesAction::ClearMarks => {
            state.resources.clear_marks();
            Vec::new()
        }
        ResourcesAction::UnmarkVisible => {
            state.resources.unmark_visible();
            Vec::new()
        }
        ResourcesAction::BatchAckAppendDigit(value) => {
            state.resources.batch_ack_append_digit(*value);
            Vec::new()
        }
        ResourcesAction::BatchAckBackspace => {
            state.resources.batch_ack_backspace();
            Vec::new()
        }
        ResourcesAction::ScrollBatchConfirmDown => {
            state.resources.scroll_batch_confirm_down();
            Vec::new()
        }
        ResourcesAction::ScrollBatchConfirmUp => {
            state.resources.scroll_batch_confirm_up();
            Vec::new()
        }
        ResourcesAction::ConfirmBatchAction if state.read_only => {
            state.resources.cancel_batch_confirmation();
            refuse_under_read_only(state, "This batch");
            Vec::new()
        }
        ResourcesAction::ConfirmBatchAction => state
            .resources
            .batch_confirmation_submission()
            .map(|(action, context, targets)| {
                // Each entry's id travels on with its own target: a target's
                // result settles exactly the entry seeded for it, and
                // cancelling this batch has to resolve exactly the entries
                // seeded here. The id is what makes "exactly these"
                // expressible.
                let seeded: Vec<_> = targets
                    .into_iter()
                    .map(|target| {
                        let id = state
                            .system
                            .operations
                            .begin(action.label(), target.name.clone());
                        (target, id)
                    })
                    .collect();
                // One effect for every service: the action names the service
                // and the targets are already projected, so the reducer never
                // matches on the action here — the runtime is the single place
                // that fans a batch out per service.
                //
                // The effect and the workflow are handed the same seeded list
                // rather than each building their own, so the two cannot
                // disagree about which id belongs to which target.
                let effect = Effect::RunResourceBatch {
                    context: context.clone(),
                    action,
                    targets: seeded.clone(),
                    // 0 = "runtime uses its configured default"; the runtime
                    // substitutes it (see Task 6).
                    concurrency: 0,
                };
                state
                    .resources
                    .begin_batch_workflow(action, context, seeded);
                vec![effect]
            })
            .unwrap_or_default(),
        // No `if state.read_only` arm beside this one, unlike the lifecycle
        // confirmations. `--read-only` refuses a plugin at whichever door opened
        // the confirmation — the palette's `Submit` and the keyed press both go
        // through [`open_plugin`], so the refusal has already
        // happened before anything is ever confirmed — and the load-bearing fact
        // that makes one refusal enough is narrower than the policy: while this
        // confirmation is open the mode resolver routes *every* key to
        // `plugin_confirmation_key`, so neither door can be reached again and
        // `ReloadConfig` cannot fire — the `read_only` flag cannot change
        // between the refusal and this arm.
        //
        // That is the invariant to check before adding any other way to reload
        // configuration. A file watcher, or a reload from anywhere that is not a
        // key press, would flip the flag under an open confirmation and this arm
        // would run a plugin `--read-only` had already refused. It would then
        // need the guard the lifecycle arms have.
        ResourcesAction::ConfirmPlugin => state
            .resources
            .plugin_confirmation_submission()
            .map(|(label, argv)| {
                // Sanitized once, here, because this is where the label stops
                // being something a render site draws and starts being text
                // that is *stored*: the operations-log entry below keeps it
                // until the log evicts it, and it travels with the effect into
                // the status line its result writes. A plugin's label is its
                // template rendered against the selected row and is
                // deliberately exempt from the argv refusals
                // (`app::state::plugins::resolve`), so a row value carrying a
                // newline or an ESC reaches it verbatim — the same hazard the
                // palette and the modal each sanitize at their own render site.
                // The argv is untouched: those are the bytes the operator
                // approved and the bytes the child must receive.
                let label = sanitize_display_text(&label);
                let Some(context) = state.current_context() else {
                    // Nothing to run under: the argv carries a
                    // `--configuration` that is resolved from the active
                    // context, and there is none. The confirmation has already
                    // been taken by the call above, so the modal closes either
                    // way; saying so is what keeps a confirmed Enter from
                    // looking like a dropped keypress. Unreachable in practice
                    // — the palette cannot offer a row-scoped plugin with no
                    // context, because there are no rows — which is exactly why
                    // it must not fail silently if it ever becomes reachable.
                    state.system.set_error_status_message(format!(
                        "{label} not run: no active gcloud configuration"
                    ));
                    return Vec::new();
                };
                // The entry's id travels with the effect, for the same reason
                // the lifecycle actions' does: nothing else identifies which
                // pending entry a result settles.
                let operation_id = state.system.operations.begin("plugin", label.clone());
                vec![Effect::RunPlugin {
                    context,
                    label,
                    argv,
                    operation_id,
                }]
            })
            .unwrap_or_default(),
        ResourcesAction::CancelPluginConfirmation => {
            state.resources.cancel_plugin_confirmation();
            Vec::new()
        }
        ResourcesAction::PluginAckAppendDigit(value) => {
            state.resources.plugin_ack_append_digit(*value);
            Vec::new()
        }
        ResourcesAction::PluginAckBackspace => {
            state.resources.plugin_ack_backspace();
            Vec::new()
        }
        ResourcesAction::PluginPromptAppend(value) => {
            if let Some(prompt) = state.resources.plugin_prompt_mut() {
                prompt.append(*value);
            }
            Vec::new()
        }
        ResourcesAction::PluginPromptBackspace => {
            if let Some(prompt) = state.resources.plugin_prompt_mut() {
                prompt.backspace();
            }
            Vec::new()
        }
        ResourcesAction::PluginPromptNextField => {
            if let Some(prompt) = state.resources.plugin_prompt_mut() {
                prompt.focus_next();
            }
            Vec::new()
        }
        ResourcesAction::PluginPromptPreviousField => {
            if let Some(prompt) = state.resources.plugin_prompt_mut() {
                prompt.focus_previous();
            }
            Vec::new()
        }
        ResourcesAction::PluginPromptCycleChoice(forward) => {
            if let Some(prompt) = state.resources.plugin_prompt_mut() {
                prompt.cycle_choice(*forward);
            }
            Vec::new()
        }
        // Emits no effect, like the arm that opens a confirmation from the
        // palette and unlike `ConfirmPlugin`: this composes the argv the
        // operator is about to read, and the confirmation is still the only
        // place a plugin becomes a process.
        ResourcesAction::SubmitPluginPrompt => {
            // The prompt is *read* here and taken only at the bottom, once
            // every step that can fail is behind us. Taking it first would
            // close the modal and drop what the operator typed on a path that
            // then returns without composing anything — the values would be
            // gone with nothing to retype them from. Every refusal below
            // therefore leaves the prompt open, with its values, and says why.
            let Some(prompt) = state.resources.plugin_prompt() else {
                return Vec::new();
            };
            // Re-asked here rather than trusted from the keymap: the keymap
            // gate is what stops the keystroke, and this is what stops anything
            // else that ever dispatches the action. Neither is redundant — the
            // keymap's answer is about a keypress, this one about the state.
            // It is also what `render_element`'s `Missing(JumpField::Name)`
            // fallback rests on: an argv composed with a prompted input absent
            // from `values` would be diagnosed as a missing row field.
            if !prompt.is_satisfied() {
                return Vec::new();
            }
            let PluginId(index) = prompt.plugin_id();
            let values = prompt.values();
            let Some(context) = state.current_context() else {
                // Unreachable in practice for the reason the `ConfirmPlugin`
                // arm gives — no context means no rows, and no row means no
                // offer to have opened this prompt — and spoken anyway for the
                // same reason: an Enter that came to nothing must not read as a
                // dropped keypress.
                let label = prompt.label().to_string();
                state.system.set_error_status_message(format!(
                    "{label} not run: no active gcloud configuration"
                ));
                return Vec::new();
            };
            // Indexing is sound while the prompt is open: see [`PluginId`] —
            // the set is loaded once at startup and a reload reports that a
            // restart is required rather than swapping it, so nothing can
            // repoint this index under the modal.
            let Some(plugin) = state.plugins.all().get(index).cloned() else {
                let label = prompt.label().to_string();
                state
                    .system
                    .set_error_status_message(format!("{label} not run: its plugin is not loaded"));
                return Vec::new();
            };
            let argv = match crate::app::state::plugins::plugin_argv(
                &plugin,
                &context,
                prompt.row_fields(),
                &values,
            ) {
                Ok(argv) => argv,
                // A collected value that renders as a flag or as an unprintable
                // is refused here rather than shown: the confirmation may only
                // ever hold an argv that passed every rule an offered one
                // passes. The prompt stays open on its values, because the
                // answer to a value gcloud would misread is to edit it.
                //
                // The reason is carried into the sentence rather than assumed
                // to be about the values: a plugin whose templates mix a row
                // field and an input is offered without its fields ever being
                // rendered, so this is also where a row that cannot fill them
                // is first refused. `refusal` is what the palette and the keyed
                // press say, so the operator reads one vocabulary whichever
                // door refused.
                Err(why) => {
                    state
                        .system
                        .set_error_status_message(crate::app::state::plugins::refusal(
                            &plugin, why,
                        ));
                    return Vec::new();
                }
            };
            // The heading is re-rendered with the values in hand, which is what
            // gives a label naming an input its real caption — the palette
            // showed the plugin's id because no value existed then.
            let label = plugin
                .label
                .render(
                    |field| crate::app::state::jumps::lookup(prompt.row_fields(), field),
                    |name| {
                        values
                            .iter()
                            .find(|(key, _)| key == name)
                            .map(|(_, value)| value.clone())
                    },
                )
                .unwrap_or_else(|| plugin.id.clone());
            let dangerous = prompt.dangerous();
            // Taken, not borrowed-and-cleared, for the reason
            // `plugin_confirmation_submission` is: a second submit must find
            // nothing rather than compose the same argv twice.
            state.resources.take_plugin_prompt();
            state
                .resources
                .show_plugin_confirmation(label, argv, dangerous);
            Vec::new()
        }
        ResourcesAction::CancelPluginPrompt => {
            state.resources.cancel_plugin_prompt();
            Vec::new()
        }
        ResourcesAction::ArmPluginPrefix => {
            let service = state.resources.active_service();
            if !crate::app::state::plugins::any_scoped(&state.plugins, service) {
                state.system.set_error_status_message(format!(
                    "No plugins are declared for {}s",
                    service.selection_noun()
                ));
                return Vec::new();
            }
            // The scope travels with the flag: see `RootState`'s field for why a
            // bare "armed" cannot be trusted by the time the key arrives.
            state.plugin_prefix_pending = Some(service);
            Vec::new()
        }
        ResourcesAction::PluginPrefixPress(key) => {
            // Taken first and unconditionally: every path out of this arm
            // consumes the prefix, and an early return that forgot to clear it
            // would leave the next keystroke resolving as a plugin key.
            let armed = state.plugin_prefix_pending.take();
            let service = state.resources.active_service();
            match armed {
                Some(armed) if armed == service => {}
                // The scope changed under the armed prefix, which no keypress
                // did — `sync_remembered_view` runs on every action, so a
                // configuration activation or the startup load landing its
                // remembered service is enough. Spoken, not silent: the
                // keystroke has been consumed by a state the operator did not
                // arm, and every other way a keyed press comes to nothing says
                // so for the same reason. Naming both scopes is what tells them
                // the view moved rather than that their key was wrong.
                Some(armed) => {
                    state.system.set_error_status_message(format!(
                        "Plugin not run: the view changed from {}s to {}s",
                        armed.selection_noun(),
                        service.selection_noun()
                    ));
                    return Vec::new();
                }
                // Unreachable through the keymap: `Mode::PluginPrefix` is
                // derived from this very field, so nothing dispatches this
                // action unarmed. Silent because there is no gesture to report
                // on — the operator never pressed `,`.
                None => return Vec::new(),
            }
            // Bound before the match, because `current_context` and
            // `selected_fields` borrow `state` immutably while every arm below
            // needs it mutably. `keyed` answers with owned values so that this
            // scope can end here.
            let resolved = {
                // Both refusals below are reachable, and neither may be
                // silent. `,` arms on `any_scoped`, which asks about the plugin
                // *file*, and the keymap binds it in both resource views (list
                // and detail) with no selection predicate — the focused service
                // navigator is the one Resources surface it stays unbound in,
                // for the reason `J` does. So a scope whose list is empty (a
                // project with no instances, or a first load still in flight)
                // reaches here with a declared plugin and nothing to run it on.
                // Two swallowed keys and no message is a dead key, which
                // is what `no_jumps_reason` (`super::jumps`) and the
                // `ConfirmPlugin` arm above both refuse to be; the wording is a
                // sibling of theirs on purpose.
                let Some(context) = state.current_context() else {
                    state
                        .system
                        .set_error_status_message("Plugin not run: no active gcloud configuration");
                    return Vec::new();
                };
                let Some(fields) =
                    crate::app::state::jumps::selected_fields(&state.resources, service)
                else {
                    state.system.set_error_status_message(format!(
                        "No {} selected to run a plugin on",
                        service.selection_noun()
                    ));
                    return Vec::new();
                };
                crate::app::state::plugins::keyed(&state.plugins, service, &context, &fields, *key)
            };
            match resolved {
                Keyed::Offer(offer) => open_plugin(state, offer),
                Keyed::Refused(why) => state.system.set_error_status_message(why),
                // Quoted the way `refusal` quotes the plugin id: `,` then a
                // stray space is a real press, and unquoted it renders as
                // "No plugin bound to   for GCE instances".
                Keyed::Unbound => state.system.set_error_status_message(format!(
                    "No plugin bound to {key:?} for {}s",
                    service.selection_noun()
                )),
            }
            Vec::new()
        }
        ResourcesAction::CancelPluginPrefix => {
            state.plugin_prefix_pending = None;
            Vec::new()
        }
        ResourcesAction::CancelBatchAction => {
            state.resources.cancel_batch_confirmation();
            Vec::new()
        }
        ResourcesAction::LeaveDrillDown => {
            state.resources.leave_bigquery_drilldown();
            Vec::new()
        }
        ResourcesAction::AscendPrefix => {
            // BigQuery's drill answers `-`/`Backspace` as well as `Esc`. The
            // hierarchy is two levels deep — tables inside a dataset, columns
            // inside a table — and both keys ascend exactly one of them, so
            // this is the same leave `Esc` performs, reachable by the key an
            // operator carrying the Cloud Storage habit will press. Without it
            // that key does nothing at all, with no movement and no message to
            // say why.
            if state.resources.active_service() == ResourceServiceKind::BigQueryDatasets {
                state.resources.leave_bigquery_drilldown();
                return Vec::new();
            }
            if state.resources.active_service() != ResourceServiceKind::CloudStorage {
                return Vec::new();
            }
            // Cloud Storage view transitions live on `StorageState`, which
            // cannot reach the `ResourcesState`-level field cursor, so the
            // three reducer arms that drive them clear it here instead.
            state.resources.clear_detail_field_index();
            let outcome = state.resources.storage_mut().ascend();
            match (outcome, state.current_context()) {
                (Some(AscendOutcome::ReloadPage { bucket, prefix }), Some(context)) => {
                    vec![storage_page_load(context, bucket, prefix)]
                }
                _ => Vec::new(),
            }
        }
        ResourcesAction::OpenBucketMetadata => {
            if state.resources.active_service() != ResourceServiceKind::CloudStorage {
                return Vec::new();
            }
            state.resources.clear_detail_field_index();
            match (
                state.current_context(),
                state.resources.storage_mut().open_bucket_metadata(),
            ) {
                (Some(context), Some(bucket)) => {
                    vec![Effect::LoadStorageBucketDetail { context, bucket }]
                }
                _ => Vec::new(),
            }
        }
        ResourcesAction::NextPage => {
            if state.resources.active_service() != ResourceServiceKind::CloudStorage {
                return Vec::new();
            }
            use crate::app::state::resources::StorageView;
            if state.resources.storage().view() != StorageView::ObjectBrowser {
                // A stale continuation token from a previous browser session
                // must not fire a page load once we're back on the bucket
                // list (or viewing object detail); only the live browser
                // table supports on-demand pagination.
                return Vec::new();
            }
            match (
                state.current_context(),
                state.resources.storage().next_page_request(),
            ) {
                (Some(context), Some((bucket, prefix, token))) => {
                    vec![Effect::LoadStorageObjectPage {
                        context,
                        bucket,
                        prefix,
                        page_token: Some(token),
                        page_size: STORAGE_PAGE_SIZE,
                    }]
                }
                _ => Vec::new(),
            }
        }
        ResourcesAction::CopyResourceUri => {
            // In a detail pane, `y` yanks the selected field (or the whole
            // document on a JSON tab, where nothing is individually
            // selectable). Everywhere else it keeps its original meaning: the
            // Cloud Storage URI of the current selection.
            if let Some(body) = project_detail_body(&state.resources) {
                if let Some(field) = selected_detail_field(state, &body) {
                    return vec![Effect::CopyToClipboard {
                        value: field.value,
                        description: format!("field \"{}\"", field.label),
                    }];
                }
                if let Some(raw_json) = body.raw_json {
                    return vec![Effect::CopyToClipboard {
                        value: raw_json,
                        description: "JSON document".to_string(),
                    }];
                }
            }
            if state.resources.active_service() != ResourceServiceKind::CloudStorage {
                return Vec::new();
            }
            let Some(context) = state.current_context() else {
                return Vec::new();
            };
            state
                .resources
                .storage()
                .selected_uri(&context)
                .map(|(value, description)| {
                    vec![Effect::CopyToClipboard {
                        value: sanitize_clipboard_text(&value),
                        description,
                    }]
                })
                .unwrap_or_default()
        }
        ResourcesAction::OpenLogsForSelection => open_logs_for_selection(state),
    }
}

/// Handles `OpenSelected` while the Cloud Storage service is active: entering a
/// bucket's object browser, descending into a folder, or opening an object's
/// detail pane depending on the active view and selected row.
fn open_selected_storage(state: &mut RootState) -> Vec<Effect> {
    use crate::app::state::resources::StorageView;
    let Some(context) = state.current_context() else {
        return Vec::new();
    };
    // Entering a browser, descending, and opening an object's detail all
    // replace whatever pane was on screen (see the `AscendPrefix` arm).
    state.resources.clear_detail_field_index();
    match state.resources.storage().view() {
        StorageView::Buckets | StorageView::BucketDetail => {
            match state.resources.storage_mut().open_object_browser() {
                Some(bucket) => vec![Effect::LoadStorageObjectPage {
                    context,
                    bucket: bucket.name,
                    prefix: String::new(),
                    page_token: None,
                    page_size: STORAGE_PAGE_SIZE,
                }],
                None => Vec::new(),
            }
        }
        StorageView::ObjectBrowser | StorageView::ObjectDetail => {
            if let Some((bucket, prefix)) = state.resources.storage_mut().descend_into_selected() {
                vec![storage_page_load(context, bucket, prefix)]
            } else if let Some((bucket, object_key)) =
                state.resources.storage_mut().open_selected_object_detail()
            {
                vec![Effect::LoadStorageObjectDetail {
                    context,
                    bucket,
                    object_key,
                }]
            } else {
                Vec::new()
            }
        }
    }
}

fn storage_page_load(
    context: nuvem_domain::gcp::configuration::GcloudContext,
    bucket: String,
    prefix: String,
) -> Effect {
    Effect::LoadStorageObjectPage {
        context,
        bucket,
        prefix,
        page_token: None,
        page_size: STORAGE_PAGE_SIZE,
    }
}

/// Resolves a managed-tunnel plan from the selected row and opens the launch
/// confirmation.
///
/// Only GCE and Cloud SQL rows have a tunnel plan; every other service is a
/// silent no-op, the same shape as the Cloud Storage-only keys above, because
/// `t` is dispatched unconditionally from the generic resource list.
fn request_tunnel(state: &mut RootState) {
    let Some(context) = state.current_context() else {
        return;
    };
    let plan = match state.resources.active_service() {
        ResourceServiceKind::ComputeInstances => {
            let Some(instance) = state.resources.selected_visible_instance() else {
                return;
            };
            // Both reach the tunnel argv — the name positionally, the zone as
            // `--zone`'s value — so either one beginning with `-` would be read
            // by gcloud as an option. Refused here rather than in the spec
            // builder: the builder also runs on the render path to draw the
            // preview, and this is where the operator can be told why.
            if let Some(offender) = nuvem_domain::gcp::argv::option_like_value([
                instance.name.as_str(),
                instance.zone.as_str(),
            ]) {
                state
                    .system
                    .set_status_message(refuse_option_like(offender));
                return;
            }
            // tgcp's IAP auto-detection: no external IP means the instance is
            // only reachable through IAP.
            let kind = compute_tunnel_kind(instance);
            TunnelLaunchPlan {
                kind,
                target_label: instance.name.clone(),
                remote_label: DEFAULT_SSH_REMOTE_PORT.to_string(),
                instance_name: instance.name.clone(),
                zone: instance.zone.clone(),
                connection_name: String::new(),
                // No conventional local port for an instance tunnel: the
                // allocator asks the OS for a free one.
                preferred_local_port: default_local_port(kind, None),
                remote_port: default_remote_port(kind, None),
                claimed_ports: Vec::new(),
            }
        }
        ResourceServiceKind::CloudSql => {
            let Some(instance) = state.resources.selected_cloud_sql_instance() else {
                return;
            };
            let Some(project) = context.project.as_deref() else {
                // Without a project there is no `project:region:instance`
                // triple, so there is no plan to preview — say so rather than
                // opening a half-built one.
                let message = "Cloud SQL tunnel needs an active project".to_string();
                state.system.set_status_message(message);
                return;
            };
            let kind = TunnelKind::CloudSqlProxy;
            let engine = Some(instance.engine.as_str());
            let remote_port = default_remote_port(kind, engine);
            // The proxy takes the triple as a positional argument, so the
            // composed value is the one that has to be data — and it leads with
            // the project, which is the only component that can put a `-` at
            // the front of it.
            let connection_name =
                cloud_sql_connection_name(project, &instance.region, &instance.name);
            if let Some(offender) =
                nuvem_domain::gcp::argv::option_like_value([connection_name.as_str()])
            {
                state
                    .system
                    .set_status_message(refuse_option_like(offender));
                return;
            }
            TunnelLaunchPlan {
                kind,
                target_label: instance.name.clone(),
                remote_label: remote_port.to_string(),
                instance_name: instance.name.clone(),
                zone: String::new(),
                connection_name,
                preferred_local_port: default_local_port(kind, engine),
                remote_port,
                claimed_ports: Vec::new(),
            }
        }
        ResourceServiceKind::GkeClusters
        | ResourceServiceKind::CloudRunServices
        | ResourceServiceKind::CloudStorage
        | ResourceServiceKind::SecretManagerSecrets
        | ResourceServiceKind::CloudFunctions
        | ResourceServiceKind::PersistentDisks => return,
        ResourceServiceKind::PubSubTopics => return,
        ResourceServiceKind::VpcNetworks => return,
        ResourceServiceKind::VpcSubnets => return,
        ResourceServiceKind::FirewallRules => return,
        ResourceServiceKind::PubSubSubscriptions => return,
        ResourceServiceKind::BigQueryDatasets => return,
        ResourceServiceKind::IamServiceAccounts => return,
        ResourceServiceKind::IamPolicy => return,
    };
    state.resources.show_tunnel_launch_preview(context, plan);
}

/// Starts the previewed tunnel: one freshly minted id shared by the `Starting`
/// row and the effect, and the live tunnels' ports snapshotted now (not at
/// preview time) so the allocator never hands out one that is already promised.
fn confirm_tunnel_launch(state: &mut RootState, started_at: std::time::Instant) -> Vec<Effect> {
    let Some((context, mut plan)) = state.resources.tunnel_launch_submission() else {
        return Vec::new();
    };
    plan.claimed_ports = state.tunnels.claimed_ports();
    let id = state.tunnels.mint_id();
    state.tunnels.insert_starting(TunnelEntry {
        id,
        kind: plan.kind,
        target_label: plan.target_label.clone(),
        // The requested port, replaced by the port the child actually reports
        // once it is up. `0` means "the allocator will pick one".
        local_port: plan.preferred_local_port.unwrap_or(0),
        remote_label: plan.remote_label.clone(),
        status: TunnelStatus::Starting,
        started_at,
        context: context.clone(),
        message: None,
    });
    vec![Effect::StartTunnel { id, context, plan }]
}
