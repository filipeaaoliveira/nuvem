//! A jump target that lands on a *list* positioned at a row,
//! rather than on a detail pane.
//!
//! Before this, `JumpRule.to` was a `DetailTab` and the four services with
//! detail tabs were the only legal destinations: twelve of the sixteen — every
//! VPC table, Secret Manager, Cloud Functions, Pub/Sub, BigQuery, IAM — could
//! be a jump *source* and never a jump *target*. What is pinned here is the
//! seam that changes that: the offer a list rule produces, the landing it
//! performs, and the two ways the landing can fail to find its row.
//!
//! The landing is asserted on the *cursor*, not on the workspace: a jump that
//! switched to the right table and left the cursor on row 0 would satisfy every
//! "did we navigate" assertion and be useless, since finding the row by hand is
//! the work the jump exists to save.

use nuvem::app::reducer::root::reduce;
use nuvem::app::state::jumps::{JumpKey, applicable_jumps};
use nuvem::app::state::navigation::JumpTarget;
use nuvem::app::state::resources::ResourceRowId;
use nuvem::app::{
    Action, AppState, Effect, EffectResult, JumpsAction, NavAction, ResourcesAction,
    WorkspaceAction,
};
use nuvem_domain::gcp::configuration::{GcloudConfiguration, GcloudConfigurationName};
use nuvem_domain::gcp::resource::{
    FirewallAction, FirewallDirection, FirewallRule, FirewallTargets, ResourceServiceKind,
    VpcNetwork, VpcRoutingMode, VpcSubnet, VpcSubnetMode,
};

use crate::support::jump_rules::with_rules;

// ---------------------------------------------------------------------------
// fixtures
// ---------------------------------------------------------------------------

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

fn app_with_context() -> AppState {
    let mut app = AppState::new();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ConfigurationsLoaded {
            configurations: vec![config()],
        }),
    );
    reduce(&mut app, &Action::Workspace(WorkspaceAction::Resources));
    app
}

fn network(name: &str) -> VpcNetwork {
    VpcNetwork {
        name: name.to_string(),
        subnet_mode: VpcSubnetMode::Auto,
        routing_mode: VpcRoutingMode::Regional,
        subnet_count: 43,
        description: None,
    }
}

fn subnet(name: &str, region: &str, network: &str) -> VpcSubnet {
    VpcSubnet {
        name: name.to_string(),
        region: region.to_string(),
        network: network.to_string(),
        ip_cidr_range: "10.128.0.0/20".to_string(),
        gateway_address: Some("10.128.0.1".to_string()),
        private_google_access: false,
    }
}

fn firewall_rule(name: &str, network: &str) -> FirewallRule {
    FirewallRule {
        name: name.to_string(),
        network: network.to_string(),
        direction: FirewallDirection::Ingress,
        priority: Some(65534),
        action: FirewallAction::Allow,
        targets: FirewallTargets::AllInstances,
        source_or_dest: vec!["0.0.0.0/0".to_string()],
        disabled: false,
    }
}

fn load_networks(app: &mut AppState, names: &[&str]) {
    let context = app.root.current_context().expect("context exists");
    reduce(
        app,
        &Action::EffectResult(EffectResult::VpcNetworksLoaded {
            context,
            networks: names.iter().copied().map(network).collect(),
        }),
    );
}

fn load_subnets(app: &mut AppState, subnets: Vec<VpcSubnet>) {
    let context = app.root.current_context().expect("context exists");
    reduce(
        app,
        &Action::EffectResult(EffectResult::VpcSubnetsLoaded { context, subnets }),
    );
}

fn load_firewall_rules(app: &mut AppState, rules: Vec<FirewallRule>) {
    let context = app.root.current_context().expect("context exists");
    reduce(
        app,
        &Action::EffectResult(EffectResult::FirewallRulesLoaded { context, rules }),
    );
}

fn select_service(app: &mut AppState, service: ResourceServiceKind) {
    reduce(
        app,
        &Action::Resources(ResourcesAction::SelectService(service)),
    );
}

fn status(app: &AppState) -> Option<&str> {
    app.root.system.status_message.as_deref()
}

/// The row the named service's cursor is on, by name — read through the
/// projection the table paints rather than off the raw index, so a cursor
/// pointing past a filtered list fails here instead of "agreeing".
fn selected_network(app: &AppState) -> Option<String> {
    let resources = &app.root.resources;
    assert_eq!(
        resources.active_service(),
        ResourceServiceKind::VpcNetworks,
        "the networks cursor is only meaningful with that table on screen"
    );
    let index = resources.selected_visible_index()?;
    resources
        .visible_vpc_networks()
        .get(index)
        .map(|row| row.name.clone())
}

/// An app sitting on the subnets table with the cursor on `subnet-b`, whose
/// network is `prod-vpc`, and with three networks loaded so landing on the
/// right one is a choice rather than the only option.
fn app_on_a_subnet() -> AppState {
    let mut app = app_with_context();
    load_networks(&mut app, &["default", "prod-vpc", "staging-vpc"]);
    load_subnets(
        &mut app,
        vec![
            subnet("subnet-a", "europe-west1", "default"),
            subnet("subnet-b", "europe-west1", "prod-vpc"),
        ],
    );
    select_service(&mut app, ResourceServiceKind::VpcSubnets);
    reduce(&mut app, &Action::Resources(ResourcesAction::SelectNext));
    app
}

// ---------------------------------------------------------------------------
// the offer
// ---------------------------------------------------------------------------

/// The built-in subnet rule resolves to the *network* the row names, keyed the
/// way the networks table identifies its own rows: name only, empty location.
///
/// A key carrying the subnet's region would look right in every debug dump and
/// match no network row at all, because a network is global.
#[test]
fn a_subnet_offers_a_list_jump_to_its_parent_network_keyed_by_name_alone() {
    let app = app_on_a_subnet();

    let jumps = applicable_jumps(&app.root);
    assert_eq!(jumps.len(), 1, "one built-in rule, got {jumps:?}");
    assert_eq!(
        jumps[0].target,
        JumpTarget::List(ResourceServiceKind::VpcNetworks)
    );
    assert_eq!(
        jumps[0].key,
        JumpKey::Row(ResourceRowId {
            name: "prod-vpc".to_string(),
            location: String::new(),
        })
    );
}

/// The picker row names the resource it lands *on*, not a collection belonging
/// to it. "VPC networks of prod-vpc" would promise a filtered table, which is
/// the one thing a list target is not.
#[test]
fn a_list_jump_reads_as_the_noun_and_the_row_it_lands_on() {
    let app = app_on_a_subnet();

    assert_eq!(applicable_jumps(&app.root)[0].label, "VPC network prod-vpc");
}

/// A firewall rule's network is a jump key even though it is not the rule's
/// identity — many rules share one network, so `{network}` names a different
/// resource than `{name}` does.
#[test]
fn a_firewall_rule_offers_a_list_jump_to_the_network_it_is_attached_to() {
    let mut app = app_with_context();
    load_networks(&mut app, &["default", "prod-vpc"]);
    load_firewall_rules(&mut app, vec![firewall_rule("allow-icmp", "prod-vpc")]);
    select_service(&mut app, ResourceServiceKind::FirewallRules);

    let jumps = applicable_jumps(&app.root);
    assert_eq!(jumps.len(), 1, "one built-in rule, got {jumps:?}");
    assert_eq!(
        jumps[0].key,
        JumpKey::Row(ResourceRowId {
            name: "prod-vpc".to_string(),
            location: String::new(),
        })
    );
}

// ---------------------------------------------------------------------------
// the landing
// ---------------------------------------------------------------------------

/// The whole point of the slice: `Enter` switches to the destination table
/// *and* puts the cursor on the named row.
///
/// `prod-vpc` is deliberately neither the first nor the last of the three
/// networks, so a landing that forgot to move the cursor, or that clamped it to
/// either end, is a different answer than the right one.
#[test]
fn submitting_a_list_jump_shows_the_table_with_the_cursor_on_the_named_row() {
    let mut app = app_on_a_subnet();

    reduce(&mut app, &Action::Jumps(JumpsAction::Open));
    let effects = reduce(&mut app, &Action::Jumps(JumpsAction::Submit));

    assert!(app.root.jump_picker.is_none(), "the picker must close");
    assert_eq!(
        app.root.resources.active_service(),
        ResourceServiceKind::VpcNetworks
    );
    assert_eq!(selected_network(&app).as_deref(), Some("prod-vpc"));
    // Nothing is fetched: the destination list is already loaded for this
    // configuration, so a list jump costs no gcloud invocation at all. The one
    // effect it does emit is the remembered-view save every service switch
    // makes, which is not a fetch.
    assert!(
        !effects
            .iter()
            .any(|effect| matches!(effect, Effect::RefreshVpcNetworks { .. })),
        "a loaded list must not be refetched: {effects:?}"
    );
    assert_eq!(status(&app), None, "a jump that landed says nothing");
}

/// A jump whose row has gone says so, and still shows the table. Landing
/// silently on row 0 would tell the user the network they came from is called
/// something it is not.
#[test]
fn submitting_a_list_jump_whose_row_is_gone_says_so_and_still_shows_the_table() {
    let mut app = app_with_context();
    // The networks list has loaded — for this configuration — and simply does
    // not contain the subnet's network any more.
    load_networks(&mut app, &["default"]);
    load_subnets(
        &mut app,
        vec![subnet("subnet-b", "europe-west1", "prod-vpc")],
    );
    select_service(&mut app, ResourceServiceKind::VpcSubnets);

    reduce(&mut app, &Action::Jumps(JumpsAction::Open));
    reduce(&mut app, &Action::Jumps(JumpsAction::Submit));

    assert_eq!(
        app.root.resources.active_service(),
        ResourceServiceKind::VpcNetworks
    );
    assert_eq!(status(&app), Some("prod-vpc is no longer available"));
}

/// A rule may name a service this session has never opened. The list is fetched
/// on the way past and the message says the row cannot be opened *yet* — the
/// same "try again once they arrive" contract `apply_detail` has, because a
/// jump is not replayed across the load.
#[test]
fn a_list_jump_into_a_never_loaded_service_starts_its_fetch_and_says_to_retry() {
    let mut app = app_with_context();
    load_subnets(
        &mut app,
        vec![subnet("subnet-b", "europe-west1", "prod-vpc")],
    );
    select_service(&mut app, ResourceServiceKind::VpcSubnets);

    reduce(&mut app, &Action::Jumps(JumpsAction::Open));
    let effects = reduce(&mut app, &Action::Jumps(JumpsAction::Submit));

    assert!(
        effects
            .iter()
            .any(|effect| matches!(effect, Effect::RefreshVpcNetworks { .. })),
        "the destination list must be fetched on the way past: {effects:?}"
    );
    assert_eq!(
        status(&app),
        Some("Loading VPC networks, so prod-vpc cannot be opened yet — try again once they arrive")
    );
}

/// A list jump is a navigation like any other, so `Alt-←` puts the user back on
/// the table they pressed `J` in. Without the history entry the jump would be a
/// one-way door.
#[test]
fn a_list_jump_is_traversable_back_to_where_it_was_pressed() {
    let mut app = app_on_a_subnet();

    reduce(&mut app, &Action::Jumps(JumpsAction::Open));
    reduce(&mut app, &Action::Jumps(JumpsAction::Submit));
    reduce(&mut app, &Action::Navigation(NavAction::Back));

    assert_eq!(
        app.root.resources.active_service(),
        ResourceServiceKind::VpcSubnets
    );
}

/// A configured rule can name any service's list, including the five the README
/// records as target-incapable. Secret Manager is the one it names by name, so
/// this fails the day the seam stops reaching beyond VPC.
#[test]
fn a_configured_rule_can_land_on_a_service_that_has_no_detail_tab_at_all() {
    let mut app = app_with_context();
    let context = app.root.current_context().expect("context exists");
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::SecretsLoaded {
            context,
            secrets: vec![
                nuvem_domain::gcp::resource::Secret {
                    name: "api-key".to_string(),
                    create_time: None,
                    replication: "automatic".to_string(),
                    labels: Vec::new(),
                },
                nuvem_domain::gcp::resource::Secret {
                    name: "prod-vpc".to_string(),
                    create_time: None,
                    replication: "automatic".to_string(),
                    labels: Vec::new(),
                },
            ],
        }),
    );
    load_subnets(
        &mut app,
        vec![subnet("subnet-b", "europe-west1", "prod-vpc")],
    );
    select_service(&mut app, ResourceServiceKind::VpcSubnets);
    with_rules(
        &mut app,
        "[[jumps]]\nfrom = \"vpc-subnets\"\nto = \"secrets\"\nname = \"{network}\"\n",
    );

    reduce(&mut app, &Action::Jumps(JumpsAction::Open));
    reduce(&mut app, &Action::Jumps(JumpsAction::Submit));

    assert_eq!(
        app.root.resources.active_service(),
        ResourceServiceKind::SecretManagerSecrets
    );
    assert_eq!(status(&app), None, "the secret named prod-vpc is loaded");
    assert_eq!(
        app.root.resources.selected_visible_index(),
        Some(1),
        "the cursor must be on the second secret, not left at the top"
    );
}
