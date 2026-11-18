//! `{network}` and `{subnetwork}` for a GCE instance, read
//! from the *detail* payload rather than from the list row.
//!
//! Every other jump field is a column of the row `J` was pressed on. These two
//! are not: `ComputeInstance` carries no network at all, and the values live
//! one level down on `ComputeInstanceDetail.network_interfaces`. That makes
//! them the first fields in the vocabulary that can be *legitimately* absent
//! while the resource plainly has one — the pane simply has not been fetched
//! yet — and the point of this slice is that the two absences are told apart.
//! Blaming the row for an unfetched pane sends the user to look at the wrong
//! thing.
//!
//! The sharpest case here is the stale one: with the pane closed, the list
//! cursor is the target while `compute.detail` still holds the *previous*
//! instance's payload. A field wired to "whatever detail is loaded" would
//! happily hand one instance's network to another.

use nuvem::app::reducer::root::reduce;
use nuvem::app::state::jumps::{JumpKey, ResourceJump, applicable_jumps, jump_diagnostics};
use nuvem::app::state::navigation::JumpTarget;
use nuvem::app::state::resources::ResourceRowId;
use nuvem::app::{Action, AppState, EffectResult, JumpsAction, ResourcesAction, WorkspaceAction};
use nuvem_domain::gcp::configuration::{GcloudConfiguration, GcloudConfigurationName};
use nuvem_domain::gcp::resource::{
    ComputeInstance, ComputeInstanceDetail, ComputeNetworkInterface, FirewallAction,
    FirewallDirection, FirewallRule, FirewallTargets, ResourceServiceKind, ResourceStatus,
    VpcNetwork, VpcRoutingMode, VpcSubnet, VpcSubnetMode,
};

use crate::support::jump_rules::with_rules;

/// A rule set whose only compute rule needs `{network}`, so the picker's
/// emptiness and its explanation are both about that one field.
const NETWORK_RULE: &str =
    "[[jumps]]\nfrom = \"compute\"\nto = \"vpc-networks\"\nname = \"{network}\"\n";

fn config() -> GcloudConfiguration {
    GcloudConfiguration {
        name: GcloudConfigurationName::new("dev"),
        is_active: true,
        account: Some("dev@example.com".to_string()),
        project: Some("dev-project".to_string()),
        region: Some("europe-west1".to_string()),
        zone: Some("europe-west1-b".to_string()),
    }
}

fn instance(name: &str) -> ComputeInstance {
    ComputeInstance {
        id: format!("{name}-id"),
        name: name.to_string(),
        zone: "europe-west1-b".to_string(),
        status: ResourceStatus::Running,
        machine_type: "e2-medium".to_string(),
        internal_ip: Some("10.0.0.2".to_string()),
        external_ip: None,
    }
}

fn interface(network: &str, subnetwork: &str) -> ComputeNetworkInterface {
    ComputeNetworkInterface {
        name: Some("nic0".to_string()),
        network: Some(network.to_string()),
        subnetwork: Some(subnetwork.to_string()),
        subnetwork_region: Some("europe-west1".to_string()),
        internal_ip: Some("10.0.0.2".to_string()),
        external_ip: None,
    }
}

fn detail(name: &str, interfaces: Vec<ComputeNetworkInterface>) -> ComputeInstanceDetail {
    ComputeInstanceDetail {
        name: name.to_string(),
        zone: "europe-west1-b".to_string(),
        status: ResourceStatus::Running,
        machine_type: "e2-medium".to_string(),
        creation_timestamp: None,
        network_interfaces: interfaces,
        tags: Vec::new(),
        service_accounts: Vec::new(),
        disks: Vec::new(),
        labels: Vec::new(),
        metadata: Vec::new(),
        raw_json: "{}".to_string(),
    }
}

fn network(name: &str) -> VpcNetwork {
    VpcNetwork {
        name: name.to_string(),
        subnet_mode: VpcSubnetMode::Custom,
        routing_mode: VpcRoutingMode::Regional,
        subnet_count: 2,
        description: None,
    }
}

/// Two instances loaded, the networks table loaded, compute on screen.
fn app_with_compute() -> AppState {
    let mut app = AppState::new();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ConfigurationsLoaded {
            configurations: vec![config()],
        }),
    );
    reduce(&mut app, &Action::Workspace(WorkspaceAction::Resources));
    let context = app.root.current_context().expect("context exists");
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ComputeInstancesLoaded {
            context: context.clone(),
            instances: vec![instance("web-01"), instance("web-02")],
        }),
    );
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::VpcNetworksLoaded {
            context,
            networks: vec![network("default"), network("prod-vpc")],
        }),
    );
    app
}

/// Opens the selected instance's pane and lands `payload` in it.
fn open_detail_with(app: &mut AppState, payload: ComputeInstanceDetail) {
    reduce(app, &Action::Resources(ResourcesAction::OpenSelected));
    let context = app.root.current_context().expect("context exists");
    reduce(
        app,
        &Action::EffectResult(EffectResult::ComputeInstanceDetailLoaded {
            context,
            detail: payload,
        }),
    );
}

// ---------------------------------------------------------------------------

/// Moves the picker's cursor onto the row `wanted` matches, failing if there
/// is no such row.
///
/// A bounded walk rather than `while cursor is not the one { select_next }`.
/// That loop spins for ever when the row is absent — `select_next` clamps at
/// the end — so a missing offer showed up as a ten-second timeout instead of a
/// failure. A hang is not a red test: it says nothing about what went wrong,
/// and a sweep counting `FAIL` lines does not count it at all.
fn select_offer(app: &mut AppState, wanted: impl Fn(&ResourceJump) -> bool) {
    let index = app
        .root
        .jump_picker
        .as_ref()
        .expect("the picker must be open")
        .rows()
        .iter()
        .position(&wanted)
        .expect("no offer in the picker matched");
    for _ in 0..index {
        reduce(app, &Action::Jumps(JumpsAction::SelectNext));
    }
}

/// The value comes off the fetched pane, and keys the networks table the way
/// that table keys its own rows — name alone, since a network is global.
#[test]
fn an_instance_network_field_resolves_from_the_loaded_detail() {
    let mut app = app_with_compute();
    with_rules(&mut app, NETWORK_RULE);
    open_detail_with(
        &mut app,
        detail("web-01", vec![interface("prod-vpc", "prod-subnet")]),
    );

    let jumps = applicable_jumps(&app.root);
    assert_eq!(
        jumps.len(),
        1,
        "expected the one configured rule: {jumps:?}"
    );
    assert_eq!(
        jumps[0].key,
        JumpKey::Row(ResourceRowId {
            name: "prod-vpc".to_string(),
            location: String::new(),
        })
    );
}

/// Pressing `J` in the list — where it is usually pressed — offers nothing for
/// a rule that needs the detail, because the row genuinely does not carry it.
#[test]
fn an_instance_network_field_is_absent_while_no_detail_is_fetched() {
    let mut app = app_with_compute();
    with_rules(&mut app, NETWORK_RULE);

    assert_eq!(applicable_jumps(&app.root), Vec::new());
}

/// The slice's point. An unfetched pane is not the row's fault, and the advice
/// that follows from it is the opposite: open the pane, rather than go and look
/// at a resource that supposedly has no network.
#[test]
fn a_field_that_needs_the_detail_says_so_instead_of_blaming_the_row() {
    let mut app = app_with_compute();
    with_rules(&mut app, NETWORK_RULE);
    // The pane is open, so the fetch is on its way; the payload has not landed.
    reduce(&mut app, &Action::Resources(ResourcesAction::OpenSelected));

    assert_eq!(
        jump_diagnostics(&app.root).as_deref(),
        Some(
            "Jump \"vpc-networks\" needs network, which is on this GCE instance's \
             detail — open it and try again"
        )
    );
}

/// The other half of the same fork: with the pane fetched and the instance
/// really having no network interface, the row *is* the answer, and the
/// original wording is what must come back.
#[test]
fn a_field_the_fetched_detail_has_no_value_for_still_blames_the_row() {
    let mut app = app_with_compute();
    with_rules(&mut app, NETWORK_RULE);
    open_detail_with(&mut app, detail("web-01", Vec::new()));

    assert_eq!(
        jump_diagnostics(&app.root).as_deref(),
        Some("Jump \"vpc-networks\" needs network, which this GCE instance has no value for")
    );
}

/// An instance may have several interfaces. Both fields must describe the
/// *same* one, and it must be the same one `{internal_ip}` already describes —
/// the parser takes that from `network_interfaces.first()`, so a `{network}`
/// read off a different interface would put two fields of one picker row on two
/// different networks.
#[test]
fn both_network_fields_read_the_interface_the_row_ip_already_came_from() {
    let mut app = app_with_compute();
    with_rules(
        &mut app,
        "[[jumps]]\nfrom = \"compute\"\nto = \"vpc-networks\"\nname = \"{network}\"\n\
         label = \"{network} {subnetwork} {internal_ip}\"\n",
    );
    open_detail_with(
        &mut app,
        detail(
            "web-01",
            vec![
                interface("prod-vpc", "prod-subnet"),
                interface("mgmt-vpc", "mgmt-subnet"),
            ],
        ),
    );

    assert_eq!(
        applicable_jumps(&app.root)[0].label,
        "prod-vpc prod-subnet 10.0.0.2"
    );
}

/// A payload that describes a *different* instance than the one selected never
/// supplies the field.
///
/// The state is built by handing `set_detail` a payload directly, because no
/// reducer path reaches it: `close_compute_detail` clears `compute.detail`, and
/// `set_detail` is itself guarded by `is_active_detail_target`, so the two
/// invariants between them keep target and payload in step. The identity check
/// in `compute_detail_for` is therefore defence in depth — and this is the only
/// test that can fail when it is removed, which is the whole reason to write it
/// this way. Going through `CloseDetail` instead (as the test below does) is
/// protected twice over and pins neither mechanism on its own.
///
/// What it defends is worth the line: handing `web-01`'s network to `web-02`
/// would be a jump that resolves, offers a picker row and lands on the wrong
/// network, silently.
#[test]
fn a_detail_describing_another_instance_never_supplies_the_field() {
    let mut app = app_with_compute();
    with_rules(&mut app, NETWORK_RULE);
    // web-02 is the selection; the loaded pane payload is web-01's.
    reduce(&mut app, &Action::Resources(ResourcesAction::SelectNext));
    app.root
        .resources
        .set_detail(detail("web-01", vec![interface("prod-vpc", "prod-subnet")]));

    let jumps = applicable_jumps(&app.root);
    assert!(
        jumps.is_empty(),
        "web-02 must not inherit web-01's network: {jumps:?}"
    );
}

/// Closing the pane takes the field with it: back in the list, `{network}` is
/// absent again rather than lingering from the pane that was open.
///
/// Two independent mechanisms enforce this — `close_compute_detail` clears the
/// payload, and the identity check above would reject it anyway — so this
/// passes with either one broken. It is kept as the behavioural statement a
/// reader wants ("the value does not outlive the pane"); the mechanism-level
/// pin is the test above.
#[test]
fn the_field_does_not_outlive_the_pane_it_was_read_from() {
    let mut app = app_with_compute();
    with_rules(&mut app, NETWORK_RULE);
    open_detail_with(
        &mut app,
        detail("web-01", vec![interface("prod-vpc", "prod-subnet")]),
    );
    // Back to the list, and on to the *other* instance.
    reduce(&mut app, &Action::Resources(ResourcesAction::CloseDetail));
    reduce(&mut app, &Action::Resources(ResourcesAction::SelectNext));

    let jumps = applicable_jumps(&app.root);
    assert!(
        jumps.is_empty(),
        "web-02 must not inherit web-01's network: {jumps:?}"
    );
}

// ---------------------------------------------------------------------------
// The subnet jump, which needs the region as well as the name
// ---------------------------------------------------------------------------

fn subnet(name: &str, region: &str) -> VpcSubnet {
    VpcSubnet {
        name: name.to_string(),
        region: region.to_string(),
        network: "default".to_string(),
        ip_cidr_range: "10.128.0.0/20".to_string(),
        gateway_address: None,
        private_google_access: false,
    }
}

/// The built-in instance-to-subnet jump lands on the subnet in the instance's
/// *own* region, not on the first row that happens to share its name.
///
/// This is the captured case the subnet jump exists for: an auto-mode
/// network names every one of its subnets `default`, one per region, so the
/// three rows here are what a real project looks like. A rule keyed on
/// `{subnetwork}` alone matches all three and lands on whichever sorts first —
/// which is why the region is carried through the parser at all.
#[test]
fn the_subnet_jump_lands_on_the_subnet_in_the_instances_own_region() {
    let mut app = app_with_compute();
    let context = app.root.current_context().expect("context exists");
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::VpcSubnetsLoaded {
            context,
            subnets: vec![
                subnet("default", "europe-north1"),
                subnet("default", "europe-west1"),
                subnet("default", "europe-west4"),
            ],
        }),
    );
    open_detail_with(
        &mut app,
        detail("web-01", vec![interface("default", "default")]),
    );

    // Only the subnet rule, so the assertion is about that rule's key.
    let subnet_jump = applicable_jumps(&app.root)
        .into_iter()
        .find(|jump| jump.target == JumpTarget::List(ResourceServiceKind::VpcSubnets))
        .expect("the built-in instance-to-subnet jump must be offered");

    assert_eq!(
        subnet_jump.key,
        JumpKey::Row(ResourceRowId {
            name: "default".to_string(),
            // The interface's self-link region, not the instance's zone.
            location: "europe-west1".to_string(),
        })
    );

    reduce(&mut app, &Action::Jumps(JumpsAction::Open));
    select_offer(&mut app, |row| {
        row.target == JumpTarget::List(ResourceServiceKind::VpcSubnets)
    });
    reduce(&mut app, &Action::Jumps(JumpsAction::Submit));

    assert_eq!(
        app.root.resources.active_service(),
        ResourceServiceKind::VpcSubnets
    );
    assert_eq!(
        app.root.resources.selected_visible_index(),
        Some(1),
        "the cursor must be on the europe-west1 row, not the first `default`"
    );
    assert_eq!(
        app.root.system.status_message.as_deref(),
        None,
        "the row was found, so nothing is reported"
    );
}

/// Without the region the same jump matches nothing at all, rather than landing
/// on a same-named subnet in another region.
///
/// The interface here carries a subnetwork link with no `regions` segment, so
/// `{subnetwork_region}` is absent and the rule refuses to render. That is the
/// safe half of the parser's `None`: an unrenderable rule offers no row, where
/// a guessed region would offer one that lands somewhere plausible and wrong.
#[test]
fn a_subnet_with_no_region_offers_no_jump_rather_than_a_wrong_one() {
    let mut app = app_with_compute();
    let context = app.root.current_context().expect("context exists");
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::VpcSubnetsLoaded {
            context,
            subnets: vec![subnet("default", "europe-north1")],
        }),
    );
    let mut payload = detail("web-01", vec![interface("default", "default")]);
    payload.network_interfaces[0].subnetwork_region = None;
    open_detail_with(&mut app, payload);

    assert!(
        !applicable_jumps(&app.root)
            .iter()
            .any(|jump| jump.target == JumpTarget::List(ResourceServiceKind::VpcSubnets)),
        "a subnet jump with no region must not be offered"
    );
}

// ---------------------------------------------------------------------------
// The firewall rules that target this instance
// ---------------------------------------------------------------------------

fn fw(name: &str, network: &str, targets: FirewallTargets, action: FirewallAction) -> FirewallRule {
    FirewallRule {
        name: name.to_string(),
        network: network.to_string(),
        direction: FirewallDirection::Ingress,
        priority: Some(1000),
        action,
        targets,
        source_or_dest: vec!["0.0.0.0/0".to_string()],
        disabled: false,
    }
}

fn load_firewall_rules(app: &mut AppState, rules: Vec<FirewallRule>) {
    let context = app.root.current_context().expect("context exists");
    reduce(
        app,
        &Action::EffectResult(EffectResult::FirewallRulesLoaded { context, rules }),
    );
}

/// A tagged instance whose pane is fetched, and four rules of which two apply.
fn app_on_a_tagged_instance() -> AppState {
    let mut app = app_with_compute();
    load_firewall_rules(
        &mut app,
        vec![
            fw(
                "allow-all-internal",
                "prod-vpc",
                FirewallTargets::AllInstances,
                FirewallAction::Allow,
            ),
            fw(
                "deny-web-egress",
                "prod-vpc",
                FirewallTargets::Tags(vec!["web-server".to_string()]),
                FirewallAction::Deny,
            ),
            fw(
                "allow-db-only",
                "prod-vpc",
                FirewallTargets::Tags(vec!["database".to_string()]),
                FirewallAction::Allow,
            ),
            fw(
                "other-network",
                "default",
                FirewallTargets::AllInstances,
                FirewallAction::Allow,
            ),
        ],
    );
    let mut payload = detail("web-01", vec![interface("prod-vpc", "prod-subnet")]);
    payload.tags = vec!["web-server".to_string()];
    open_detail_with(&mut app, payload);
    app
}

/// The built-in jump lands on the firewall rules table narrowed to the rules
/// that target this instance — the network-wide one included, the other
/// network's and the other tag's excluded.
#[test]
fn the_firewall_jump_narrows_the_table_to_the_rules_that_target_this_instance() {
    let mut app = app_on_a_tagged_instance();

    reduce(&mut app, &Action::Jumps(JumpsAction::Open));
    select_offer(&mut app, |row| {
        row.label.starts_with("Firewall rules targeting")
    });
    reduce(&mut app, &Action::Jumps(JumpsAction::Submit));

    assert_eq!(
        app.root.resources.active_service(),
        ResourceServiceKind::FirewallRules
    );
    let visible: Vec<String> = app
        .root
        .resources
        .visible_firewall_rules()
        .iter()
        .map(|rule| rule.name.clone())
        .collect();
    assert_eq!(visible, vec!["allow-all-internal", "deny-web-egress"]);
    assert!(
        app.root.resources.firewall_narrowed_to().is_some(),
        "the table must say it is narrowed rather than look complete"
    );
}

/// The jump is offered only once the pane is fetched, because the tags and
/// service accounts it computes from live on the detail — the same rule
/// `{network}` follows.
#[test]
fn the_firewall_jump_is_not_offered_from_the_list() {
    let mut app = app_with_compute();
    load_firewall_rules(&mut app, Vec::new());

    assert!(
        !applicable_jumps(&app.root)
            .iter()
            .any(|jump| jump.label.starts_with("Firewall rules targeting")),
        "a narrowing jump needs the detail, so the list offers none"
    );
}
