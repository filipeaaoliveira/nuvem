//! The firewall rules table narrowed to one instance.
//!
//! The narrowing is a *second* predicate over the same list, intersected with
//! the text filter rather than replacing it. What is pinned here is that every
//! consumer of "the visible list" agrees — the painted rows, the cursor, and
//! the row the cursor resolves to — because a narrowing applied to the table
//! and not to the cursor would put the selection on a row nobody can see.
//!
//! The hazard this feature carries is a short list that looks like a complete
//! one. So the state says plainly that it is narrowed, and clearing it is a
//! single call rather than something the operator has to reconstruct.

use nuvem::app::state::resources::ResourcesState;
use nuvem_domain::gcp::configuration::{GcloudConfigurationName, GcloudContext};
use nuvem_domain::gcp::resource::{
    FirewallAction, FirewallDirection, FirewallInstanceScope, FirewallRule, FirewallTargets,
    ResourceServiceKind,
};

fn context() -> GcloudContext {
    GcloudContext {
        configuration: GcloudConfigurationName::new("dev"),
        account: Some("dev@example.com".to_string()),
        project: Some("dev-project".to_string()),
        region: Some("europe-west1".to_string()),
        zone: Some("europe-west1-b".to_string()),
    }
}

fn rule(
    name: &str,
    network: &str,
    targets: FirewallTargets,
    action: FirewallAction,
) -> FirewallRule {
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

/// Four rules: two that target a `web-server`-tagged instance on `default`,
/// one scoped to a tag it does not carry, and one on another network.
fn seeded() -> ResourcesState {
    let mut state = ResourcesState::default();
    state.select_service(ResourceServiceKind::FirewallRules);
    state.set_firewall_rules(
        context(),
        vec![
            rule(
                "allow-all-internal",
                "default",
                FirewallTargets::AllInstances,
                FirewallAction::Allow,
            ),
            rule(
                "deny-web-egress",
                "default",
                FirewallTargets::Tags(vec!["web-server".to_string()]),
                FirewallAction::Deny,
            ),
            rule(
                "allow-db-only",
                "default",
                FirewallTargets::Tags(vec!["database".to_string()]),
                FirewallAction::Allow,
            ),
            rule(
                "other-network-all",
                "prod-vpc",
                FirewallTargets::AllInstances,
                FirewallAction::Allow,
            ),
        ],
    );
    state
}

fn scope() -> FirewallInstanceScope {
    FirewallInstanceScope {
        network: "default".to_string(),
        tags: vec!["web-server".to_string()],
        service_accounts: Vec::new(),
    }
}

fn visible(state: &ResourcesState) -> Vec<String> {
    state
        .visible_firewall_rules()
        .iter()
        .map(|rule| rule.name.clone())
        .collect()
}

/// Narrowing keeps exactly the rules that target the instance — the
/// no-selector rule on its network included, and the other network's excluded.
#[test]
fn narrowing_keeps_the_rules_that_target_the_instance() {
    let mut state = seeded();
    assert_eq!(visible(&state).len(), 4, "all four before narrowing");

    state.narrow_firewall_rules_to(scope());

    assert_eq!(
        visible(&state),
        vec!["allow-all-internal", "deny-web-egress"]
    );
}

/// A DENY rule that targets the instance stays in the narrowed list.
///
/// This is the whole safety point of the feature: the list an operator opens
/// to find out what governs a workload must not quietly drop the rules that
/// block traffic. `deny-web-egress` is in the assertion above by name, and
/// this states why it has to be.
#[test]
fn a_deny_rule_that_targets_the_instance_is_never_narrowed_away() {
    let mut state = seeded();
    state.narrow_firewall_rules_to(scope());

    assert!(
        state
            .visible_firewall_rules()
            .iter()
            .any(|rule| rule.action == FirewallAction::Deny),
        "a DENY rule targeting this instance must survive the narrowing"
    );
}

/// The narrowing is reported, not silent. A short table that looked like a
/// complete one is the failure this feature would otherwise introduce.
#[test]
fn the_state_says_which_instance_the_table_is_narrowed_to() {
    let mut state = seeded();
    assert_eq!(state.firewall_narrowed_to(), None);

    state.narrow_firewall_rules_to(scope());
    assert_eq!(state.firewall_narrowed_to(), Some(&scope()));

    state.clear_firewall_narrowing();
    assert_eq!(state.firewall_narrowed_to(), None);
    assert_eq!(
        visible(&state).len(),
        4,
        "clearing restores the whole table"
    );
}

/// The cursor and the painted list agree. A narrowing that moved the rows but
/// not the cursor would leave the selection on a row the table no longer
/// shows, and every action keyed on the selection would act on it.
#[test]
fn the_cursor_lands_inside_the_narrowed_list() {
    let mut state = seeded();
    // Put the cursor on the last row of the full table.
    for _ in 0..3 {
        state.select_next();
    }
    assert_eq!(state.selected_visible_index(), Some(3));

    state.narrow_firewall_rules_to(scope());

    let index = state
        .selected_visible_index()
        .expect("a narrowed list with rows has a selection");
    assert!(
        index < visible(&state).len(),
        "cursor {index} is outside the {} narrowed rows",
        visible(&state).len()
    );
    assert_eq!(
        state.selected_firewall_rule().map(|rule| rule.name.clone()),
        Some("allow-all-internal".to_string()),
        "the cursor goes to the top of what is left, not to an arbitrary row"
    );
}

/// The narrowing recomputes against whatever rows the table currently holds,
/// so a rule that arrives in a later load is matched like any other.
///
/// Storing the matching rule *names* at jump time would pass every test above
/// and fail this one — by omitting a rule, which is the direction that matters.
#[test]
fn a_rule_that_arrives_after_the_narrowing_is_still_matched() {
    let mut state = seeded();
    state.narrow_firewall_rules_to(scope());
    assert_eq!(visible(&state).len(), 2);

    // The list reloads, and the project has grown a new DENY rule since.
    let mut rules: Vec<FirewallRule> = state.firewall_rules().to_vec();
    rules.push(rule(
        "deny-added-later",
        "default",
        FirewallTargets::AllInstances,
        FirewallAction::Deny,
    ));
    state.set_firewall_rules(context(), rules);

    assert!(
        visible(&state).contains(&"deny-added-later".to_string()),
        "a rule added since the jump must appear: {:?}",
        visible(&state)
    );
}
