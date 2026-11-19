use crate::app::service_registry::{load_effect, service_id_for_resource};
use crate::app::state::navigation::{
    DetailTab, NavLocation, ResourceDetailLocation, ResourceKey, ResourceLocation,
};
use crate::app::state::resources::{ResourceLoadPhase, ResourceRowId};
use crate::app::state::root::{RootState, Workspace};
use crate::app::{Effect, NavAction};
use nuvem_domain::gcp::configuration::GcloudContext;
use nuvem_domain::gcp::resource::{
    CloudRunRevision, CloudRunServiceStatus, FirewallInstanceScope, ResourceServiceKind,
};

pub(super) fn reduce(root: &mut RootState, action: &NavAction) -> Vec<Effect> {
    if matches!(action, NavAction::Back) && leave_storage_view(root) {
        return Vec::new();
    }
    let current = NavLocation::capture(root);
    let target = match action {
        NavAction::Back => root.navigation.go_back(current),
        NavAction::Forward => root.navigation.go_forward(current),
    };
    match target {
        Some(location) => apply_location(root, location),
        None => {
            if matches!(action, NavAction::Back) && root.resources.is_showing_detail() {
                root.resources.close_detail();
            }
            Vec::new()
        }
    }
}

/// Back inside Cloud Storage steps out of the view it is in rather than
/// through history: an object's detail returns to the object browser, and the
/// browser or a bucket's metadata returns to the bucket list.
///
/// None of those views is a history entry, because entering them pushes none,
/// so `Back` from them used to leave Cloud Storage altogether, for whatever
/// came before it in history. BigQuery's drill answers the same need through
/// `LeaveDrillDown`. Returns whether it handled the key.
fn leave_storage_view(root: &mut RootState) -> bool {
    use crate::app::state::resources::StorageView;
    if root.workspace != Workspace::Resources
        || root.resources.active_service() != ResourceServiceKind::CloudStorage
    {
        return false;
    }
    match root.resources.storage().view() {
        StorageView::Buckets => false,
        StorageView::ObjectDetail | StorageView::BucketDetail => {
            root.resources.close_detail();
            true
        }
        StorageView::ObjectBrowser => {
            root.resources.clear_detail_field_index();
            root.resources.storage_mut().show_service_list();
            true
        }
    }
}

fn apply_location(root: &mut RootState, location: NavLocation) -> Vec<Effect> {
    match location {
        NavLocation::Home => {
            root.workspace = Workspace::Home;
            Vec::new()
        }
        NavLocation::Configurations => {
            root.workspace = Workspace::Configurations;
            Vec::new()
        }
        NavLocation::Resources(ResourceLocation { service, detail }) => {
            root.workspace = Workspace::Resources;
            match detail {
                None => {
                    root.resources.show_service_list(service);
                    Vec::new()
                }
                Some(detail) => apply_detail(root, service, detail),
            }
        }
        NavLocation::Logging => {
            root.workspace = Workspace::Logging;
            root.logging.status = crate::app::state::logging::LoggingStatus::Loading;
            super::logging::load_recent_or_settle(root, super::logging::UnloadedBuffer::Discard)
        }
        // Nothing to fetch: the tunnel list is maintained by the runtime's
        // supervision results, not by a load on entry.
        NavLocation::Tunnels => {
            root.workspace = Workspace::Tunnels;
            Vec::new()
        }
        // Re-probed rather than restored: recommendations refresh daily on
        // Google's side and the locations worth asking about come from rows
        // that may have been listed since.
        NavLocation::FinOps => {
            root.workspace = Workspace::FinOps;
            super::finops::reduce(root, &crate::app::action::FinOpsAction::Refresh)
        }
    }
}

/// Why a detail pane could not be opened, worded for which of the three things
/// actually happened.
///
/// "no longer available" asserts a history: the row was there and has since
/// gone. That is true for a back/forward restore and for a jump inside a
/// service the user has been in, and it is what a vanished row must keep
/// saying. It is *not* true of a configured jump into a service this session
/// never loaded — nothing was ever there to lose — and a message that claimed
/// otherwise would be the UI inventing a past the user did not have.
///
/// `arriving` splits the not-loaded half in two, because the two want opposite
/// advice. A list with a load in flight — one this call started, or one that
/// was already running — will have rows shortly, so the message says so and
/// says what to do about it: the jump is not carried across the load and
/// replayed, so the user is the one who repeats it. A list with nothing coming
/// — a failed load waiting on the runtime's retry backoff — has no such
/// follow-up, and telling the user to try again would be advice that does not
/// work.
fn unavailable(
    root: &RootState,
    service: ResourceServiceKind,
    name: &str,
    arriving: bool,
) -> String {
    if root.resources.has_loaded_list(service) {
        return format!("{name} is no longer available");
    }
    let noun = service.selection_noun();
    if arriving {
        return format!(
            "Loading {noun}s, so {name} cannot be opened yet — try again once they arrive"
        );
    }
    format!("No {noun} loaded yet, so {name} cannot be opened")
}

/// Opens a resource's detail pane at a named tab, syncing the list cursor to it
/// and emitting that service's detail fetches — reporting "no longer available"
/// when the row has since gone.
///
/// Shared with the related-resource jump reducer (`super::jumps`): a jump is
/// the same operation as restoring a detail location from history, so it goes
/// through this one path rather than hand-rolling an open that could skip a
/// fetch and leave the pane rendering stale data.
///
/// The contract is both halves: open the detail *and* ensure the list backing
/// it is being fetched. A rule may name a tab of a service this session has
/// never loaded, and `Alt-←`/`Alt-→` can restore into a list a failed refresh
/// emptied; in both cases the pane resolves to a list with no rows and — with
/// `poll_interval_secs = 0` — nothing that will ever put rows in it, so
/// [`backing_list_load`] emits that service's list load through the same
/// `service_registry::load_effect` seam `ResourcesAction::SelectService` uses.
/// It is scoped to lists that have never loaded for the current context *and*
/// have no fetch of their own already accounted for: a jump or restore into a
/// service the user has already seen, into one whose load is in flight, or into
/// one whose load failed *in this configuration* and is waiting on the
/// runtime's retry backoff, emits nothing extra. So this is not an
/// unconditional refetch, and — since this is also the `Alt-←`/`Alt-→` restore
/// path — repeating a restore cannot turn into one gcloud invocation per
/// keystroke.
pub(super) fn apply_detail(
    root: &mut RootState,
    service: ResourceServiceKind,
    detail: ResourceDetailLocation,
) -> Vec<Effect> {
    root.resources.select_service(service);
    let mut effects = backing_list_load(root, service);
    let arriving = !effects.is_empty()
        || matches!(
            root.resources.load_phase(service),
            ResourceLoadPhase::Loading { .. }
        );
    effects.extend(open_detail(root, service, detail, arriving));
    effects
}

/// Shows a service's list with the cursor on one named row, ensuring the list
/// backing it is being fetched — reporting why the row is not there when it is
/// not.
///
/// The list half of [`apply_detail`], and deliberately its sibling rather than
/// a branch inside it: both halves of that function's contract apply here too
/// (`backing_list_load` so a jump into a service this session never opened does
/// not land on a permanently empty table, and [`unavailable`] so a row that has
/// gone says so), while the detail half — open the pane, emit the describe —
/// has nothing to do on a list.
///
/// The list is shown whether or not the row is found. A jump the user asked for
/// has to leave them somewhere they can see the answer: landing on the table
/// with a message saying the row is filtered out or not loaded is information,
/// whereas staying put with a message would make `J` look broken.
pub(super) fn apply_list(
    root: &mut RootState,
    service: ResourceServiceKind,
    row: &ResourceRowId,
) -> Vec<Effect> {
    root.resources.show_service_list(service);
    let effects = backing_list_load(root, service);
    let arriving = !effects.is_empty()
        || matches!(
            root.resources.load_phase(service),
            ResourceLoadPhase::Loading { .. }
        );
    if !root.resources.select_visible_row(service, row) {
        let message = unavailable(root, service, &row.name, arriving);
        root.system.set_status_message(message);
    }
    effects
}

/// Shows a service's list narrowed to the rules that target `scope`, ensuring
/// the list backing it is being fetched.
///
/// The sibling of [`apply_list`], and it differs in the one way that matters:
/// there is no row to land on. "The rules that target this instance" is a set,
/// it may legitimately be empty, and an empty result is an *answer* — no rule
/// governs this instance — rather than a row that has gone. So this never
/// reports "no longer available"; it narrows, and the table says what it is
/// narrowed to.
///
/// The narrowing is set before the fetch is decided so that rows arriving later
/// are matched on arrival: the scope is stored, not the matching rule names, so
/// a rule created since the jump is matched like any other. Storing names would
/// go stale by *omitting* a rule from a list an operator is reading for safety.
pub(super) fn apply_narrowed_list(
    root: &mut RootState,
    service: ResourceServiceKind,
    scope: FirewallInstanceScope,
) -> Vec<Effect> {
    root.resources.show_service_list(service);
    root.resources.narrow_firewall_rules_to(scope);
    backing_list_load(root, service)
}

/// The list-load effect for a service whose list is not loaded and has nothing
/// else bringing it — the whole of the "and ensure the list is being fetched"
/// half of [`apply_detail`]'s contract.
fn backing_list_load(root: &RootState, service: ResourceServiceKind) -> Vec<Effect> {
    let Some(context) = root.current_context() else {
        return Vec::new();
    };
    if !needs_backing_list_load(root, service, &context) {
        return Vec::new();
    }
    let Some(id) = service_id_for_resource(service) else {
        return Vec::new();
    };
    // The service being loaded is the one whose filter gets pushed — the same
    // pairing `ResourcesAction::SelectService` makes.
    let filter = super::resources::active_filter_for(root, service);
    load_effect(id, context, filter).into_iter().collect()
}

/// Whether `service`'s list needs fetching *for `context`*.
///
/// Every question here is asked of the configuration the user is in now,
/// because both halves of the state outlive the configuration that produced
/// them. The rows carry the context they were loaded for; the load phase does
/// not, which is why a failure records its own.
fn needs_backing_list_load(
    root: &RootState,
    service: ResourceServiceKind,
    context: &GcloudContext,
) -> bool {
    match root.resources.loaded_list_context(service) {
        // Rows are already here for this configuration.
        Some(loaded) if loaded == context => false,
        // Rows belonging to a configuration the user has left. Unreachable
        // today — `switch_context` clears every service's stamp, and rows
        // restored from its cache are restored under the context being
        // switched *to* — but stated rather than folded into a neighbouring
        // arm, because the bug this predicate exists to prevent was exactly a
        // state being answered by an arm that did not mean it.
        Some(_) => true,
        None => match root.resources.load_phase(service) {
            // A fetch is already coming; a second buys nothing.
            ResourceLoadPhase::Loading { .. } => false,
            // A failure is the runtime's to retry, behind the
            // `should_auto_retry`/`gated_scope` backoff `poll_active_resource`
            // honours — but only the failure of *this* configuration's
            // backend. One inherited from a configuration the user has left
            // says nothing about the one they are in.
            ResourceLoadPhase::Failed { .. } => {
                !root.resources.list_load_failed_for(service, context)
            }
            ResourceLoadPhase::Idle => true,
        },
    }
}

fn open_detail(
    root: &mut RootState,
    service: ResourceServiceKind,
    detail: ResourceDetailLocation,
    arriving: bool,
) -> Vec<Effect> {
    let context = root.current_context();
    let revision = detail.revision;
    match (service, detail.key, detail.tab) {
        (
            ResourceServiceKind::ComputeInstances,
            ResourceKey::Compute { name, zone },
            DetailTab::Compute(tab),
        ) => match (
            context,
            root.resources.restore_compute_detail(&name, &zone, tab),
        ) {
            (Some(context), Some(instance)) => {
                vec![Effect::LoadComputeInstanceDetail { context, instance }]
            }
            _ => {
                root.resources.show_service_list(service);
                let message = unavailable(root, service, &name, arriving);
                root.system.set_status_message(message);
                Vec::new()
            }
        },
        (
            ResourceServiceKind::GkeClusters,
            ResourceKey::Gke { name, location },
            DetailTab::Gke(tab),
        ) => match (
            context,
            root.resources.restore_gke_detail(&name, &location, tab),
        ) {
            (Some(context), Some(cluster)) => {
                vec![Effect::LoadGkeClusterDetail { context, cluster }]
            }
            _ => {
                root.resources.show_service_list(service);
                let message = unavailable(root, service, &name, arriving);
                root.system.set_status_message(message);
                Vec::new()
            }
        },
        (
            ResourceServiceKind::CloudSql,
            ResourceKey::CloudSql { name, region },
            DetailTab::CloudSql(tab),
        ) => match (
            context,
            root.resources.restore_cloud_sql_detail(&name, &region, tab),
        ) {
            (Some(context), Some(instance)) => {
                vec![Effect::LoadCloudSqlInstanceDetail { context, instance }]
            }
            _ => {
                root.resources.show_service_list(service);
                let message = unavailable(root, service, &name, arriving);
                root.system.set_status_message(message);
                Vec::new()
            }
        },
        (
            ResourceServiceKind::CloudRunServices,
            ResourceKey::CloudRun { name, region },
            DetailTab::CloudRun(tab),
        ) => match (
            context,
            root.resources.restore_cloud_run_detail(&name, &region, tab),
        ) {
            (Some(context), Some(service)) => {
                let mut effects = vec![
                    Effect::LoadCloudRunServiceDetail {
                        context: context.clone(),
                        service: service.clone(),
                    },
                    Effect::LoadCloudRunRevisions {
                        context: context.clone(),
                        service,
                    },
                ];
                // Re-open the nested revision sub-location, if any. The detail's
                // authoritative revision list is not loaded yet (the effects above
                // fetch it), so reconstruct a minimal revision target by name; the
                // LoadCloudRunRevisionDetail effect fills in the rest.
                if let Some(rev) = revision {
                    let target = CloudRunRevision {
                        name: rev.name.clone(),
                        ready: CloudRunServiceStatus::Unknown(String::new()),
                        created: None,
                        image: None,
                        active_percent: None,
                    };
                    root.resources
                        .open_cloud_run_revision(target, rev.region.clone());
                    effects.push(Effect::LoadCloudRunRevisionDetail {
                        context,
                        region: rev.region,
                        revision: rev.name,
                    });
                }
                effects
            }
            _ => {
                root.resources.show_service_list(service);
                let message = unavailable(root, service, &name, arriving);
                root.system.set_status_message(message);
                Vec::new()
            }
        },
        // Mismatched service/key combinations (should not occur): resolve to the list.
        (service, _, _) => {
            root.resources.show_service_list(service);
            Vec::new()
        }
    }
}
