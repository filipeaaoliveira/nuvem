//! The related-resource jump picker (`J`).
//!
//! The *contents* of the picker are the pure
//! [`applicable_jumps`], derived
//! from the user's `[[jumps]]` rules; this module owns the user-facing half —
//! opening the modal, moving in it, and performing the jump.

use crate::app::state::jumps::{JumpKey, JumpPickerState, applicable_jumps, jump_diagnostics};
use crate::app::state::navigation::{
    DetailTab, JumpTarget, NavLocation, ResourceDetailLocation, ResourceKey,
};
use crate::app::state::resources::ResourceRowId;
use crate::app::state::root::RootState;
use crate::app::{Effect, JumpsAction};
use nuvem_domain::gcp::resource::FirewallInstanceScope;
use nuvem_domain::gcp::resource::ResourceServiceKind;

pub fn reduce(root: &mut RootState, action: &JumpsAction) -> Vec<Effect> {
    match action {
        JumpsAction::Open => open(root),
        JumpsAction::SelectNext => {
            if let Some(picker) = root.jump_picker.as_mut() {
                picker.select_next();
            }
            Vec::new()
        }
        JumpsAction::SelectPrevious => {
            if let Some(picker) = root.jump_picker.as_mut() {
                picker.select_previous();
            }
            Vec::new()
        }
        JumpsAction::Submit => submit(root),
        // Closing is the whole of it: the picker never touched the view it was
        // opened over, so there is nothing to put back.
        JumpsAction::Cancel => {
            root.jump_picker = None;
            Vec::new()
        }
    }
}

fn open(root: &mut RootState) -> Vec<Effect> {
    match JumpPickerState::new(applicable_jumps(root)) {
        Some(picker) => {
            root.jump_picker = Some(picker);
        }
        // G4: a jump that does not apply must say so. The empty vector alone
        // cannot tell "this kind of resource has no related tab" from "nothing
        // is selected", and the two want opposite advice, so the reason is
        // re-read from state here rather than guessed.
        None => {
            let message = no_jumps_reason(root);
            root.system.set_error_status_message(message);
        }
    }
    Vec::new()
}

/// Performs the selected jump — opening a detail pane at the tab the offer
/// named, or showing a service list positioned at the row it named — and closes
/// the picker.
///
/// A list target is the simple case and is handed straight to
/// [`super::navigation::apply_list`]: there is no pane that might already be
/// open and no tab to switch to, so none of the three detail cases below
/// applies. Landing on a list the user is already looking at moves the cursor
/// and records no history entry, because [`NavLocation::capture`] does not
/// carry the cursor — which is right: there is nowhere to go back to.
///
/// A jump to a *different* resource is expressed as a [`ResourceDetailLocation`]
/// and handed to [`super::navigation::apply_detail`] — the same path
/// `Alt-←`/`Alt-→` use to restore a detail location. That is deliberate: it
/// already opens the pane at a named tab *and* emits the service's detail
/// fetches (both of Cloud Run's), so a jump cannot land on a pane that renders
/// stale or empty.
///
/// A jump to another tab of the pane *already* open is the opposite case, and
/// the common one: the detail is fetched, so `apply_detail` would blank the
/// pane and re-describe the resource the user is looking at. That jump is a tab
/// change and is performed as one, exactly like `[`/`]`. It still counts as a
/// navigation — the tab is part of the captured location, so the `before !=
/// after` guard in [`super::root::reduce`] records the history entry.
///
/// A jump to the tab *already on screen* moves nowhere, and is a true no-op
/// beyond closing the picker. `select_detail_tab` is not idempotent — a tab
/// change means a new field list, a fresh scroll and, for Cloud Run, a reset
/// revision cursor — so running it here would leave "lose your place in the
/// pane" as the only visible effect of the jump.
fn submit(root: &mut RootState) -> Vec<Effect> {
    let Some(jump) = root
        .jump_picker
        .as_ref()
        .and_then(|p| p.selected().cloned())
    else {
        return Vec::new();
    };

    // Mapped before the picker is closed: closing first would turn an
    // unmappable pair into the silent no-op G4 forbids.
    let Some(destination) = destination(jump.target, &jump.key) else {
        root.jump_picker = None;
        root.system
            .set_error_status_message("That jump has no destination".to_string());
        return Vec::new();
    };
    root.jump_picker = None;

    let (service, tab, key) = match destination {
        // A list target has no pane to already be open and no tab to switch
        // to, so none of the three cases below apply: it is always the
        // "navigate there" path.
        Destination::List { service, row } => {
            return super::navigation::apply_list(root, service, &row);
        }
        Destination::Narrowed { service, scope } => {
            return super::navigation::apply_narrowed_list(root, service, scope);
        }
        Destination::Detail { service, tab, key } => (service, tab, key),
    };

    if let Some(active_tab) = open_detail_tab(root, service, &key) {
        if active_tab == tab {
            return Vec::new();
        }
        root.resources.select_detail_tab(tab);
        // A tab switch and this lazy load are paired everywhere else the tab
        // can change — `[`/`]` do exactly this. Now that a rule may name any
        // detail tab, a user's `to = "cloudsql:users"` reaches it, and without
        // it that jump would land on a permanently empty tab.
        return super::resources::cloud_sql_users_lazy_load(root);
    }

    super::navigation::apply_detail(
        root,
        service,
        ResourceDetailLocation {
            key,
            tab,
            // A jump always lands on a service detail pane; the nested Cloud
            // Run revision view is never a jump destination (and, per
            // `applicable_jumps`, never a jump *origin* either).
            revision: None,
        },
    )
}

/// The tab on screen when the detail pane already shows this exact resource —
/// the only case in which a jump is a tab change rather than an open, and
/// `None` otherwise.
///
/// The question is asked of [`NavLocation::capture`] rather than of each
/// service's detail target in turn, so "which resource is the pane showing, on
/// which tab" keeps exactly one answer, the one navigation history records.
/// Returning the tab as well as the verdict is what lets the caller tell a tab
/// change from a jump that is already where it is going. A pane drilled into a
/// Cloud Run revision is deliberately *not* a match: leaving that view is a
/// re-open, not a tab switch.
fn open_detail_tab(
    root: &RootState,
    service: ResourceServiceKind,
    key: &ResourceKey,
) -> Option<DetailTab> {
    match NavLocation::capture(root) {
        NavLocation::Resources(location) if location.service == service => location
            .detail
            .filter(|detail| detail.key == *key && detail.revision.is_none())
            .map(|detail| detail.tab),
        _ => None,
    }
}

/// Where a checked jump goes, in the shape the view that performs it needs.
enum Destination {
    Detail {
        service: ResourceServiceKind,
        tab: DetailTab,
        key: ResourceKey,
    },
    List {
        service: ResourceServiceKind,
        row: ResourceRowId,
    },
    /// A list narrowed to a computed set, with no row to land on.
    Narrowed {
        service: ResourceServiceKind,
        scope: FirewallInstanceScope,
    },
}

/// The view a jump lands on, once the target and the key are known to agree.
///
/// The target *is* the destination, so all this decides is that the key is the
/// shape that view identifies a resource by — a pairing `apply_detail` would
/// otherwise discover by describing the wrong resource, and `apply_list` by
/// searching a list of row ids for a value that could never be one.
///
/// `None` is unreachable today: `offer` (in `crate::app::state::jumps`) builds
/// the `JumpKey` variant by matching on the very `rule.to` that becomes
/// `target` here, so the two can never disagree. It is kept as defence in
/// depth — the check is what keeps that construction rule *local* to `offer`
/// rather than an invariant every future producer of a `ResourceJump` has to
/// know about — and the caller reports it instead of silently no-oping, per G4.
fn destination(target: JumpTarget, key: &JumpKey) -> Option<Destination> {
    let service = target.service();
    match (target, key) {
        (JumpTarget::Detail(tab), JumpKey::Detail(key)) => {
            let keyed = match key {
                ResourceKey::Compute { .. } => ResourceServiceKind::ComputeInstances,
                ResourceKey::Gke { .. } => ResourceServiceKind::GkeClusters,
                ResourceKey::CloudSql { .. } => ResourceServiceKind::CloudSql,
                ResourceKey::CloudRun { .. } => ResourceServiceKind::CloudRunServices,
            };
            (service == keyed).then(|| Destination::Detail {
                service,
                tab,
                key: key.clone(),
            })
        }
        // A row id carries no service of its own — it is `(name, location)`
        // for every one of the sixteen — so there is nothing to cross-check
        // here beyond the shapes matching. The pairing that matters for a list
        // target is instead enforced at load: `keys_rows_by_location` decides
        // whether the id may carry a location at all.
        (JumpTarget::List(service), JumpKey::Row(row)) => Some(Destination::List {
            service,
            row: row.clone(),
        }),
        // A narrowing lands on a list, and only on the service its computation
        // is defined for — which `config::jumps::validate` has already checked,
        // so this is the same defence-in-depth the arms above are.
        (JumpTarget::List(service), JumpKey::Narrowed(scope)) => Some(Destination::Narrowed {
            service,
            scope: scope.clone(),
        }),
        (JumpTarget::Detail(_), JumpKey::Row(_))
        | (JumpTarget::List(_), JumpKey::Detail(_))
        | (JumpTarget::Detail(_), JumpKey::Narrowed(_)) => None,
    }
}

/// Why no jump applies, worded for the situation the user is actually in.
///
/// The order of the checks is load-bearing. A rule that cannot resolve for this
/// row has to report *itself*: it is the only reason the user can act on, and
/// every check below would otherwise describe it as something it is not. A list
/// with nothing selected is likewise not a "no jumps here" situation, and a
/// service no rule is written for is not a "select something first" one —
/// selecting a row would change nothing.
fn no_jumps_reason(root: &RootState) -> String {
    if root.resources.is_showing_cloud_run_revision() {
        return "No related-resource jumps from a Cloud Run revision".to_string();
    }
    if let Some(reason) = jump_diagnostics(root) {
        return reason;
    }
    let service = root.resources.active_service();
    // Phrased from the rule set rather than from a list of services, because
    // which services offer jumps is now the user's to decide: a configured GKE
    // rule must not be answered with "GKE has no jumps". Every noun in
    // `ResourceServiceKind::selection_noun` pluralises with a bare "s".
    if root.jump_rules.for_service(service).next().is_none() {
        return format!(
            "No related-resource jumps configured for {}s",
            service.selection_noun()
        );
    }
    // The service has rules and none of them refused, so the only way to get
    // here is an empty selection. Same noun table and shape as the `L` log
    // jump's "nothing selected" message, so the two read as siblings.
    format!("No {} selected to jump from", service.selection_noun())
}
