//! Related-resource jumps available for the current selection.
//!
//! A "jump" moves from the resource the user has in view to a *related*
//! collection that already exists as a detail tab — a Compute instance's disks
//! or network interfaces, a Cloud SQL instance's replicas, a Cloud Run
//! service's revisions. Nothing here fetches data; the destinations are tabs
//! the detail views already render.
//!
//! [`applicable_jumps`] is deliberately a function of [`RootState`] rather than
//! a per-service list the render layer inspects: the entries are user-supplied
//! data (`[[jumps]]`, see [`crate::config::jumps`]), and this signature is the
//! seam the original jumps left for them.

use std::cell::Cell;

use crate::app::state::navigation::DetailTab;
use crate::app::state::navigation::{JumpTarget, ResourceKey};
use crate::app::state::resources::{
    ResourceRowId, ResourcesState, cloud_sql_status_label, resource_status_label,
};
use crate::app::state::root::RootState;
use crate::config::jumps::{
    JumpField, JumpRule, JumpSelect, Template, detail_backed_fields, fields_for, location_field,
};
use nuvem_domain::gcp::resource::{
    ComputeInstance, ComputeInstanceDetail, ComputeNetworkInterface, FirewallInstanceScope,
    ResourceServiceKind,
};

/// The longest a rendered name or location may be. GCP's own limit for the
/// resource names these keys carry; see [`is_resource_name`].
const MAX_NAME_LEN: usize = 253;

/// One offer in the jump picker: where it goes, which resource it goes to, and
/// how it reads on screen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceJump {
    /// Where this jump lands: a detail tab, or a service list.
    /// [`JumpTarget::service`] answers which view that is either way.
    pub target: JumpTarget,
    /// The resource the jump lands on. Resolved once, when the offer is built,
    /// so that the row the user picks is the resource the label named even if
    /// the list underneath reloads while the picker is open — and so that
    /// performing the jump is a lookup rather than a second, possibly
    /// divergent, resolution of "what is selected".
    pub key: JumpKey,
    /// [`JumpTarget::default_label`] rendered for this row — `"Disks of
    /// web-01"`, `"VPC network default"` — unless the rule carries a `label`
    /// template that renders. Naming the concrete resource is what makes the
    /// picker self-explanatory when the same key opens it from five different
    /// services.
    pub label: String,
}

/// How a jump's landing resource is identified, in the shape the view it lands
/// on already uses.
///
/// Two shapes rather than one, because the two views identify a resource
/// differently and neither spelling is convertible into the other without
/// knowing the service: a detail pane is opened by the [`ResourceKey`]
/// navigation history keys it with, and a list cursor is positioned by the
/// [`ResourceRowId`] the marks already use. Collapsing them would mean one of
/// the two consumers translating on every call, which is the seam where "which
/// half of the key is the zone" gets answered twice and differently.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JumpKey {
    /// Keyed as navigation history keys it: name plus zone/region.
    Detail(ResourceKey),
    /// Keyed as the target service's own row identity.
    Row(ResourceRowId),
    /// Not a row at all: the list arrives narrowed to a computed set, and the
    /// cursor goes to the top of what is left.
    ///
    /// A third shape rather than a `Row` with a clever id, because there is no
    /// single row to name — "the rules that target this instance" is a set,
    /// and it may legitimately be empty.
    Narrowed(FirewallInstanceScope),
}

/// The open jump picker: the offers derived from the selection when `J` was
/// pressed, plus the cursor over them.
///
/// The rows are captured at open time rather than re-derived per frame because
/// the modal owns the keyboard but not the effect queue: a list reload can land
/// while it is on screen, and re-deriving would silently change the meaning of
/// the row the user is looking at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JumpPickerState {
    rows: Vec<ResourceJump>,
    selected: usize,
}

impl JumpPickerState {
    /// Opens a picker over `rows`, or `None` when there is nothing to offer —
    /// an empty modal would be a dead end (see G4: the caller reports why).
    pub fn new(rows: Vec<ResourceJump>) -> Option<Self> {
        if rows.is_empty() {
            return None;
        }
        Some(Self { rows, selected: 0 })
    }

    pub fn rows(&self) -> &[ResourceJump] {
        &self.rows
    }

    pub fn selected_index(&self) -> usize {
        self.selected
    }

    pub fn selected(&self) -> Option<&ResourceJump> {
        self.rows.get(self.selected)
    }

    /// Clamping, not wrapping — the same movement the configuration context
    /// picker has, so the two modals do not behave differently under `j`/`k`.
    pub fn select_next(&mut self) {
        // `new` refuses an empty vec and nothing here mutates `rows`, so there
        // is always a last index to clamp to.
        self.selected = (self.selected + 1).min(self.rows.len() - 1);
    }

    pub fn select_previous(&mut self) {
        self.selected = self.selected.saturating_sub(1);
    }
}

/// Returns the jumps valid for what the user has selected *right now*.
///
/// Two rules decide what "selected" means, and both exist so a jump can never
/// name a resource the user is not looking at:
///
/// * the nested Cloud Run revision view owns the pane, so the revision — not
///   the parent service — is the selection there, and a revision has no
///   related collection of its own;
/// * with a detail pane open the list cursor is still movable (`j`/`k` reach
///   the collection behind the pane), so the *detail target* is authoritative,
///   not the list selection.
///
/// The second rule is not this feature's to own — it is an invariant of what
/// the view is showing — so it is read from `ResourcesState::current_*_target`
/// (and `StorageState::view_bucket`, which answers the same question for the
/// service that has no detail *pane*), the same accessors `selection_log_query`
/// in `crate::app::reducer::resources` resolves log jumps through. `L` and `J`
/// therefore cannot name two different resources.
///
/// The rules are the configured ones ([`RootState::jump_rules`]), rendered
/// against that selection in file order — which is the picker's row order. A
/// rule that cannot resolve for this row produces no offer at all rather than a
/// row that would open a pane describing nothing; [`jump_diagnostics`] is where
/// the user is told why.
pub fn applicable_jumps(state: &RootState) -> Vec<ResourceJump> {
    resolve(state).into_iter().filter_map(Result::ok).collect()
}

/// Why the picker would be empty even though the active service *has* rules —
/// `None` when it would not be, so the caller can fall back to its own
/// service-level and selection-level wording.
///
/// This exists because the empty vector [`applicable_jumps`] returns cannot
/// tell "nothing is selected" from "the rule you wrote needs a field this row
/// has no value for", and only the second is a config problem the user can act
/// on. Reporting the *first* unresolvable rule rather than all of them keeps
/// the message a status line can hold; a user with two broken rules fixes them
/// one at a time either way.
pub fn jump_diagnostics(state: &RootState) -> Option<String> {
    let resolved = resolve(state);
    if resolved.iter().any(Result::is_ok) {
        return None;
    }
    let (rule, why) = resolved.into_iter().find_map(Result::err)?;
    let noun = rule.from.selection_noun();
    let destination = rule.to.config_key();
    Some(match why {
        // The fork this exists for. A detail-backed field that is
        // missing *because the pane is not fetched* is not the row's fault, and
        // the two cases send the user to opposite places: open the pane, or go
        // and look at a resource that really has no network.
        Unresolved::Missing(field) if awaiting_detail(&state.resources, rule.from, field) => {
            format!(
                "Jump {destination:?} needs {}, which is on this {noun}'s detail \
                 — open it and try again",
                field_name(rule.from, field)
            )
        }
        Unresolved::Missing(field) => format!(
            "Jump {destination:?} needs {}, which this {noun} has no value for",
            field_name(rule.from, field)
        ),
        Unresolved::Unsafe(part) => {
            format!("Jump {destination:?} renders an invalid {part} for this {noun}")
        }
        // Unreachable: `resolve` routes a narrowing rule to `narrowed_offer`,
        // which never returns this. Worded rather than `unreachable!` so a
        // future third path that forgets the routing reports a sentence instead
        // of panicking in front of the operator.
        Unresolved::NeedsSelection => {
            format!("Jump {destination:?} needs a selection this {noun} cannot provide")
        }
        Unresolved::NoSourceLocation => {
            format!("Jump {destination:?} cannot be keyed: a {noun} has no location")
        }
    })
}

/// Every rule for the current selection, resolved into either its offer or the
/// reason it has none. Both public functions read this one list, so the picker
/// and its explanation can never disagree about which rules resolved.
fn resolve(state: &RootState) -> Vec<Result<ResourceJump, (&JumpRule, Unresolved)>> {
    let resources = &state.resources;
    if resources.is_showing_cloud_run_revision() {
        return Vec::new();
    }
    let service = resources.active_service();
    let Some(fields) = selected_fields(resources, service) else {
        return Vec::new();
    };
    state
        .jump_rules
        .for_service(service)
        .map(|rule| {
            // A narrowing rule is computed from the selection rather
            // than rendered from it, so it takes the other path. Routed here,
            // where both paths get the same `&JumpRule` and the same error
            // shape, so the picker cannot tell them apart afterwards.
            match rule.select {
                Some(select) => narrowed_offer(rule, select, resources),
                None => offer(rule, &fields),
            }
            .map_err(|why| (rule, why))
        })
        .collect()
}

/// One narrowing rule resolved against the selection.
///
/// The only computation today is "the firewall rules that target this
/// instance", and it needs the instance's network, tags and service accounts —
/// all of which live on the *detail* payload, so the offer appears once the
/// pane is fetched and not from the list. That is the same rule `{network}`
/// follows, and the diagnostic says so through the same
/// [`Unresolved::Missing`] path.
fn narrowed_offer(
    rule: &JumpRule,
    select: JumpSelect,
    resources: &ResourcesState,
) -> Result<ResourceJump, Unresolved> {
    match select {
        JumpSelect::FirewallRulesTargetingInstance => {
            let target = resources
                .current_compute_target()
                .ok_or(Unresolved::Missing(JumpField::Name))?;
            let detail = compute_detail_for(resources, target)
                .ok_or(Unresolved::Missing(JumpField::Network))?;
            // No network, no scope — and no offer, rather than an offer that
            // narrows to every rule whose own network happens to be empty.
            let scope = FirewallInstanceScope::from_detail(detail)
                .ok_or(Unresolved::Missing(JumpField::Network))?;

            let label = rule
                .label
                .as_ref()
                .and_then(|template| {
                    let fields = selected_fields(resources, rule.from)?;
                    render(template, &fields).ok()
                })
                .unwrap_or_else(|| format!("Firewall rules targeting {}", target.name));

            Ok(ResourceJump {
                target: rule.to,
                key: JumpKey::Narrowed(scope),
                label,
            })
        }
    }
}

/// Why a rule produced no offer for the selected row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Unresolved {
    /// A template asked for a field this row has no value for.
    Missing(JumpField),
    /// A rendered `"name"` or `"location"` is not usable as a resource name.
    Unsafe(&'static str),
    /// A narrowing rule ([`JumpRule::select`]) reached `offer`, which renders
    /// templated keys and cannot build a computed one. Never surfaced: `resolve`
    /// routes those rules to `narrowed_offer` before `offer` is reached, and
    /// this exists so that routing is a compile-checked total match rather than
    /// an `unreachable!`.
    NeedsSelection,
    /// The rule left `location` to the source row, and the source service has
    /// no location field to take it from. Unreachable while
    /// `config::jumps`'s `every_service_has_the_name_and_location_fields_the_rules_assume`
    /// holds, and kept separate anyway: reporting it as a bad *template* would
    /// point the user at a line they did not write.
    NoSourceLocation,
}

/// One rule rendered against the selected row.
fn offer(rule: &JumpRule, fields: &[(JumpField, String)]) -> Result<ResourceJump, Unresolved> {
    // A narrowing rule does not render a key: the set it lands on is
    // computed from the source resource, so `name` and `location` have nothing
    // to name and are not consulted. It is handled before them rather than
    // after, so an unrenderable `{name}` cannot suppress a jump whose key was
    // never going to come from a template.
    if rule.select.is_some() {
        return Err(Unresolved::NeedsSelection);
    }
    let name = render(&rule.name, fields)?;
    if !is_resource_name(&name) {
        return Err(Unresolved::Unsafe("name"));
    }

    // A destination that keys its rows by name alone gets no location at all,
    // rather than an empty one computed from the source: asking the source for
    // a location it may not have would turn a perfectly resolvable jump into a
    // `NoSourceLocation` diagnostic about a value the destination never wanted.
    let location = if rule.to.keys_rows_by_location() {
        let rendered = match &rule.location {
            Some(template) => render(template, fields)?,
            // No `location` template means the source row's own location,
            // which is the field the config layer already guarantees every
            // service that needs one has.
            None => {
                let field = location_field(rule.from).ok_or(Unresolved::NoSourceLocation)?;
                lookup(fields, field).ok_or(Unresolved::Missing(field))?
            }
        };
        if !is_resource_name(&rendered) {
            return Err(Unresolved::Unsafe("location"));
        }
        rendered
    } else {
        String::new()
    };

    // The *destination* decides how the key is shaped: `apply_detail` describes
    // a GKE cluster by location and a Cloud SQL instance by region, whatever
    // service the jump was offered from — and a list target is keyed the way
    // that service's own rows are.
    let key = match rule.to {
        JumpTarget::Detail(DetailTab::Compute(_)) => JumpKey::Detail(ResourceKey::Compute {
            name: name.clone(),
            zone: location,
        }),
        JumpTarget::Detail(DetailTab::Gke(_)) => JumpKey::Detail(ResourceKey::Gke {
            name: name.clone(),
            location,
        }),
        JumpTarget::Detail(DetailTab::CloudSql(_)) => JumpKey::Detail(ResourceKey::CloudSql {
            name: name.clone(),
            region: location,
        }),
        JumpTarget::Detail(DetailTab::CloudRun(_)) => JumpKey::Detail(ResourceKey::CloudRun {
            name: name.clone(),
            region: location,
        }),
        JumpTarget::List(_) => JumpKey::Row(ResourceRowId {
            name: name.clone(),
            location,
        }),
    };

    // The built-in wording names the *target*, so a rule that renames the
    // resource it lands on says where it actually lands. It is also the
    // fallback when a `label` template will not render: unlike `name` and
    // `location` above, a label is not part of the key, so an unrenderable one
    // costs the user a caption rather than the whole jump.
    let label = rule
        .label
        .as_ref()
        .and_then(|template| render(template, fields).ok())
        .unwrap_or_else(|| rule.to.default_label(&name));

    Ok(ResourceJump {
        target: rule.to,
        key,
        label,
    })
}

/// Renders a template, reporting *which* field made it unrenderable.
///
/// [`Template::render`] stops at the first field it cannot resolve, so the
/// field recorded last is that field. A template's segments are private to the
/// config layer on purpose — the grammar is not this module's to walk — so
/// observing the lookup is how the diagnostic learns which field to name.
fn render(template: &Template, fields: &[(JumpField, String)]) -> Result<String, Unresolved> {
    let missing = Cell::new(None);
    let rendered = template.render(
        |field| {
            let value = lookup(fields, field);
            if value.is_none() {
                missing.set(Some(field));
            }
            value
        },
        |_| None,
    );
    match rendered {
        Some(value) => Ok(value),
        // `lookup` never answers with an empty string, so `render` can only
        // have refused after recording a field; the fallback keeps this total.
        None => Err(Unresolved::Missing(
            missing.get().unwrap_or(JumpField::Name),
        )),
    }
}

/// The row's value for `field`, absent when the row has none *or* has a blank
/// one — a key built from a blank is not a key, and neither is an argv element
/// that renders to nothing.
///
/// Shared with [`crate::app::state::plugins`] rather than copied into it: it
/// reads the same table [`selected_fields`] builds, where an absent value is
/// carried as an empty string, so two copies could only ever differ by one of
/// them forgetting that — and then `J` and a plugin would disagree about
/// whether this row has a zone.
pub(crate) fn lookup(fields: &[(JumpField, String)], field: JumpField) -> Option<String> {
    fields
        .iter()
        .find(|(candidate, _)| *candidate == field)
        .map(|(_, value)| value.clone())
        .filter(|value| !value.is_empty())
}

/// The network interface a GCE instance's `{network}` and `{subnetwork}`
/// describe, or `None` when the detail pane has not been fetched *for this
/// instance*.
///
/// The identity check in [`compute_detail_for`] is defence in depth, and saying
/// so is the honest version: no reducer path reaches a loaded payload that
/// describes a different instance than the target, because
/// `close_compute_detail` clears `compute.detail` and `set_detail` is guarded by
/// `is_active_detail_target`. The check is here so that this function is
/// correct on its own terms rather than by appeal to two invariants living in
/// another module — and because what it prevents is silent: one instance's
/// network handed to another, as a jump that resolves, offers a picker row and
/// lands on the wrong network.
///
/// `first()` rather than a search: an instance may have several interfaces, and
/// `ComputeInstance`'s own `internal_ip` and `external_ip` are already taken
/// from `network_interfaces.first()` by the parser. Picking a different one
/// here would put `{internal_ip}` and `{network}` of a single picker row on two
/// different networks. What this does NOT offer is the *other* interfaces;
/// naming them needs a per-interface vocabulary the template grammar has no
/// spelling for.
fn compute_primary_interface<'a>(
    resources: &'a ResourcesState,
    target: &ComputeInstance,
) -> Option<&'a ComputeNetworkInterface> {
    compute_detail_for(resources, target)?
        .network_interfaces
        .first()
}

/// The fetched detail pane, but only when it describes `target`.
///
/// Split out from [`compute_primary_interface`] rather than folded into it
/// because the two `None`s mean opposite things and one caller has to tell them
/// apart: "the pane is not fetched" is [`awaiting_detail`], and "the pane is
/// fetched and this instance has no interface" is a genuinely absent value. A
/// single predicate over the interface answered both with the same `None`, and
/// reported an instance with no network interfaces as one whose pane had not
/// loaded.
fn compute_detail_for<'a>(
    resources: &'a ResourcesState,
    target: &ComputeInstance,
) -> Option<&'a ComputeInstanceDetail> {
    resources
        .detail()
        .filter(|detail| detail.name == target.name && detail.zone == target.zone)
}

/// Whether `field` of `service` is one this selection cannot answer *yet*
/// because the detail carrying it has not been fetched — as distinct from one
/// the resource genuinely has no value for (slice 2).
///
/// The two are the same empty string in [`selected_fields`], and they want
/// opposite advice: "open the pane" against "this resource has no network".
/// Deciding it here, from the same accessors the field was read through, is
/// what keeps the diagnostic from re-deriving a second, divergent answer.
fn awaiting_detail(
    resources: &ResourcesState,
    service: ResourceServiceKind,
    field: JumpField,
) -> bool {
    if !detail_backed_fields(service).contains(&field) {
        return false;
    }
    match service {
        ResourceServiceKind::ComputeInstances => resources
            .current_compute_target()
            .is_some_and(|target| compute_detail_for(resources, target).is_none()),
        // No other service declares a detail-backed field, so nothing else can
        // reach here; `detail_backed_fields` is the table that decides.
        _ => false,
    }
}

/// The name a user writes between braces for `field`, read back out of the
/// config layer's own table so a warning cannot name a field by a spelling the
/// grammar does not accept.
///
/// Shared with [`crate::app::state::plugins`], whose diagnostic names a field
/// for the same reason and to the same reader.
pub(crate) fn field_name(kind: ResourceServiceKind, field: JumpField) -> &'static str {
    fields_for(kind)
        .iter()
        .find_map(|(name, candidate)| (*candidate == field).then_some(*name))
        .unwrap_or("that field")
}

/// Whether a rendered value may be used as a GCP resource name.
///
/// Defence in depth, not the last gate: `apply_detail` will only open a pane
/// for a key that matches an already-loaded row exactly, so a value reaches
/// `gcloud describe` only if GCP itself returned it as a resource name. This
/// check is what keeps a *templated* value — a status, an IP, two fields
/// spliced with a separator — from being offered as an identity in the first
/// place, which is why it is on the rendered result rather than on the
/// template.
///
/// The alphabet is deliberately the permissive union GCP's own name rules are
/// carved out of: it admits a leading `-` and a bare `..`, neither of which
/// any GCP service accepts. Tightening it here would encode one service's
/// naming rules for all five; a row that could not exist is refused by the
/// match against loaded rows anyway.
fn is_resource_name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_NAME_LEN
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

/// The selected resource's values for every field its service offers, or `None`
/// when nothing is selected.
///
/// The pairs answer exactly the `(service, field)` table
/// [`crate::config::jumps::fields_for`] validates templates against, so a
/// template that parsed can always be looked up here. A field the row has no
/// value for is carried as an empty string and refused by [`lookup`].
///
/// Two callers read this table now, not one: [`crate::app::state::plugins`]
/// renders a plugin's argv against the same row, and it has to be the same
/// answer — `J` and a plugin naming different resources for one selection is
/// the failure this shared function prevents.
pub(crate) fn selected_fields(
    resources: &ResourcesState,
    service: ResourceServiceKind,
) -> Option<Vec<(JumpField, String)>> {
    Some(match service {
        ResourceServiceKind::ComputeInstances => {
            let instance = resources.current_compute_target()?;
            // The two detail-backed fields (slice 2). Absent — an
            // empty string, which `lookup` refuses — whenever the pane is not
            // fetched for *this* instance.
            let interface = compute_primary_interface(resources, instance);
            vec![
                (JumpField::Name, instance.name.clone()),
                (JumpField::Zone, instance.zone.clone()),
                (JumpField::MachineType, instance.machine_type.clone()),
                (JumpField::Status, resource_status_label(&instance.status)),
                (
                    JumpField::InternalIp,
                    instance.internal_ip.clone().unwrap_or_default(),
                ),
                (
                    JumpField::ExternalIp,
                    instance.external_ip.clone().unwrap_or_default(),
                ),
                (
                    JumpField::Network,
                    interface
                        .and_then(|nic| nic.network.clone())
                        .unwrap_or_default(),
                ),
                (
                    JumpField::Subnetwork,
                    interface
                        .and_then(|nic| nic.subnetwork.clone())
                        .unwrap_or_default(),
                ),
                (
                    JumpField::SubnetworkRegion,
                    interface
                        .and_then(|nic| nic.subnetwork_region.clone())
                        .unwrap_or_default(),
                ),
            ]
        }
        ResourceServiceKind::GkeClusters => {
            let cluster = resources.current_gke_target()?;
            vec![
                (JumpField::Name, cluster.name.clone()),
                (JumpField::Location, cluster.location.clone()),
            ]
        }
        ResourceServiceKind::CloudSql => {
            let instance = resources.current_cloud_sql_target()?;
            vec![
                (JumpField::Name, instance.name.clone()),
                (JumpField::Region, instance.region.clone()),
                (JumpField::Engine, instance.engine.clone()),
                (JumpField::Tier, instance.tier.clone()),
                (JumpField::Status, cloud_sql_status_label(&instance.status)),
            ]
        }
        ResourceServiceKind::CloudRunServices => {
            let service = resources.current_cloud_run_target()?;
            vec![
                (JumpField::Name, service.name.clone()),
                (JumpField::Region, service.region.clone()),
            ]
        }
        // `view_bucket` is storage's `current_*_target`: it answers from the
        // view (the browsed bucket, then the metadata pane's target, then the
        // list cursor), which is the same precedence rule and the same
        // accessor `selection_log_query` resolves `L` through.
        ResourceServiceKind::CloudStorage => {
            let bucket = resources.storage().view_bucket()?;
            vec![
                (JumpField::Name, bucket.name.clone()),
                (JumpField::Location, bucket.location.clone()),
            ]
        }
        // A secret is global, so `name` is all there is to key on — which is
        // why `location_field` is `None` for this service.
        ResourceServiceKind::SecretManagerSecrets => {
            let secret = resources
                .secret_detail_target()
                .or(resources.selected_secret())?;
            vec![(JumpField::Name, secret.name.clone())]
        }
        // No detail pane to be the authoritative target yet, so the list
        // cursor is the whole rule here.
        ResourceServiceKind::CloudFunctions => {
            let function = resources.selected_cloud_function()?;
            vec![
                (JumpField::Name, function.name.clone()),
                (JumpField::Region, function.region.clone()),
            ]
        }
        // Same as Cloud Functions: no detail pane to be the authoritative
        // target, so the list cursor is the whole rule.
        ResourceServiceKind::PersistentDisks => {
            let disk = resources.selected_persistent_disk()?;
            vec![
                (JumpField::Name, disk.name.clone()),
                (JumpField::Location, disk.location.clone()),
            ]
        }
        // A topic is global, so `name` is all there is to key on — which is
        // why `location_field` is `None` for this service too. No detail pane
        // in this slice, so the list cursor is the whole rule.
        ResourceServiceKind::PubSubTopics => {
            let topic = resources.selected_pubsub_topic()?;
            vec![(JumpField::Name, topic.name.clone())]
        }
        // The network's own name is the whole rule: it is global, so there is
        // no location to offer, and neither `MODE` nor `ROUTING` is an
        // identity a jump could be keyed on — both are shared by every network
        // configured the same way.
        ResourceServiceKind::VpcNetworks => {
            let network = resources.selected_vpc_network()?;
            vec![(JumpField::Name, network.name.clone())]
        }
        // Name and location both, because a subnet's name does not identify
        // it: an auto-mode network calls every one of its subnets `default`,
        // so a rule that rendered `{name}` alone would name 43 resources and
        // land on whichever sorted first. `network` is a third field and not
        // part of that identity — it names the resource a subnet *points at*.
        ResourceServiceKind::VpcSubnets => {
            let subnet = resources.selected_vpc_subnet()?;
            vec![
                (JumpField::Name, subnet.name.clone()),
                (JumpField::Location, subnet.region.clone()),
                // The parent network's id, already shortened from its
                // self-link by the parser, so it is the name the networks
                // table paints and a list jump can match a row on.
                (JumpField::Network, subnet.network.clone()),
            ]
        }
        // A firewall rule is global to its project, so there is no location to
        // offer, and its name is the whole of its identity. `network` is here
        // as a jump key rather than as an identity: it is shared by many rules
        // and names a different resource, which is exactly the thing the
        // list target exists to reach.
        ResourceServiceKind::FirewallRules => {
            let rule = resources.selected_firewall_rule()?;
            vec![
                (JumpField::Name, rule.name.clone()),
                // Not an identity — many rules share one network — but the
                // network *is* a resource this row points at, which is exactly
                // what a jump key is for.
                (JumpField::Network, rule.network.clone()),
            ]
        }
        // The parent topic is deliberately not a jump field. `JumpField` has
        // no variant for it, and a rule keyed on the topic would fire on the
        // topics table's `name` too — two different resources answering one
        // rule. The subscription's own name is the identity, as it is for
        // topics and secrets.
        ResourceServiceKind::PubSubSubscriptions => {
            let subscription = resources.selected_pubsub_subscription()?;
            vec![(JumpField::Name, subscription.name.clone())]
        }
        // A dataset's id is its identity within the project and its location
        // is a real field of the row. No detail pane in this slice, so the
        // list cursor is the whole rule.
        ResourceServiceKind::BigQueryDatasets => {
            let dataset = resources.selected_bigquery_dataset()?;
            vec![
                (JumpField::Name, dataset.dataset_id.clone()),
                (JumpField::Location, dataset.location.clone()),
            ]
        }
        // An account is global and the email is the identity, so `name`
        // carries the email and `location_field` stays `None`. No detail pane
        // in this slice, so the list cursor is the whole rule.
        ResourceServiceKind::IamServiceAccounts => {
            let account = resources.selected_service_account()?;
            vec![(JumpField::Name, account.email.clone())]
        }
        // A binding is global and the member is the identity, so `name`
        // carries the member verbatim and `location_field` stays `None`.
        ResourceServiceKind::IamPolicy => {
            let member = resources.selected_policy_member()?;
            vec![(JumpField::Name, member.raw_member.clone())]
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The bound and the alphabet are what stand between a rendered template
    /// and `gcloud describe`, so both edges are pinned rather than assumed: a
    /// check that accepted a space or a slash would let a rule splice a shell
    /// word into an argument list.
    #[test]
    fn a_resource_name_is_bounded_and_limited_to_name_characters() {
        assert!(is_resource_name("web-01"));
        assert!(is_resource_name("web.01_alpha-2"));
        // Spelled out rather than read from `MAX_NAME_LEN`: a bound compared
        // against itself is a bound no test can move.
        assert!(is_resource_name(&"a".repeat(253)));

        assert!(!is_resource_name(""));
        assert!(!is_resource_name(&"a".repeat(254)));
        assert!(!is_resource_name("web 01"));
        assert!(!is_resource_name("web/01"));
        assert!(!is_resource_name("web-01;id"));
        assert!(!is_resource_name("$(id)"));
        // A value that happens to look like a template is a *value*: rendering
        // never rescans it, and it is refused here as the name it is not.
        assert!(!is_resource_name("{zone}"));
        // Both of these are refused only because the alphabet is *ASCII*
        // alphanumeric. Relaxing it to `char::is_alphanumeric` would admit
        // unicode homoglyphs of a name the user believes they typed, and a
        // newline would let one rendered value read as two lines in a message.
        assert!(!is_resource_name("wéb"));
        assert!(!is_resource_name("web-01\nlogs"));
    }
}
