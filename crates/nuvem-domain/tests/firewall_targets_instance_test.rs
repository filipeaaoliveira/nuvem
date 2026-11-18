//! Whether a firewall rule targets a given instance.
//!
//! # What this is evidence about, and what it is not
//!
//! [`rule_targets_instance`] is a function of six values: the rule's network
//! and target selector, and the instance's network, tags and service accounts.
//! Every case below is **CONSTRUCTED** — built from the domain's own
//! vocabulary rather than from bytes an API sent — and that is the right kind
//! of evidence for this question. A constructed document is weak evidence
//! about *what the API sends*; it is exact evidence about *what this function
//! does with a given input*, because the input is a Rust value, not a
//! transcription of a wire format.
//!
//! The capture gap that does bite lives one layer down, in the parser that
//! decides which of these values a wire document produces. That is asserted in
//! `tests/vpc_firewall_service_test.rs` and `tests/compute_detail_parse_test.rs`
//! and labelled there.
//!
//! # The failure this exists to prevent
//!
//! A rule with no target selector applies to **every** instance in its
//! network. Six of the seven rules in the captured fixture are that shape. A
//! matcher that read "no selector" as "targets nothing" would return almost
//! nothing, look plausible, and silently omit every default rule in the
//! project — DENY rules included. That is worse than having no link at all,
//! because it answers a question the operator asked and answers it wrongly.

use nuvem_domain::gcp::resource::{
    ComputeInstanceDetail, ComputeNetworkInterface, FirewallAction, FirewallDirection,
    FirewallInstanceScope, FirewallRule, FirewallTargets, ResourceStatus, rule_targets_instance,
};

fn rule(network: &str, targets: FirewallTargets) -> FirewallRule {
    FirewallRule {
        name: "a-rule".to_string(),
        network: network.to_string(),
        direction: FirewallDirection::Ingress,
        priority: Some(1000),
        action: FirewallAction::Allow,
        targets,
        source_or_dest: vec!["0.0.0.0/0".to_string()],
        disabled: false,
    }
}

fn scope(network: &str, tags: &[&str], accounts: &[&str]) -> FirewallInstanceScope {
    FirewallInstanceScope {
        network: network.to_string(),
        tags: tags.iter().map(|t| (*t).to_string()).collect(),
        service_accounts: accounts.iter().map(|a| (*a).to_string()).collect(),
    }
}

fn tags(values: &[&str]) -> FirewallTargets {
    FirewallTargets::Tags(values.iter().map(|v| (*v).to_string()).collect())
}

fn accounts(values: &[&str]) -> FirewallTargets {
    FirewallTargets::ServiceAccounts(values.iter().map(|v| (*v).to_string()).collect())
}

// ---------------------------------------------------------------------------
// the dangerous case
// ---------------------------------------------------------------------------

/// A rule with no selector targets every instance in its network — including
/// an instance with no tags and no service accounts, which is what a plain
/// instance looks like and what the captured instance fixture actually is.
#[test]
fn a_rule_with_no_target_selector_targets_every_instance_in_its_network() {
    let untagged = scope("default", &[], &[]);
    let tagged = scope(
        "default",
        &["web-server"],
        &["sa@example.iam.gserviceaccount.com"],
    );

    assert!(rule_targets_instance(
        &rule("default", FirewallTargets::AllInstances),
        &untagged
    ));
    assert!(rule_targets_instance(
        &rule("default", FirewallTargets::AllInstances),
        &tagged
    ));
}

/// ...but only in *its* network. "Every instance" is scoped by the network the
/// rule lives in, and a rule cannot reach across networks whatever its
/// selector says. Reading `AllInstances` as unconditional would put every
/// default rule in the project on every instance's list.
#[test]
fn a_rule_with_no_target_selector_stops_at_its_own_network() {
    assert!(!rule_targets_instance(
        &rule("prod-vpc", FirewallTargets::AllInstances),
        &scope("default", &[], &[])
    ));
}

// ---------------------------------------------------------------------------
// the network gate
// ---------------------------------------------------------------------------

/// The network gate is absolute: a matching tag on the wrong network is not a
/// match. Tag names are not unique across networks, so a matcher that checked
/// the selector first and the network second — or not at all — would link an
/// instance to a rule that cannot reach it.
#[test]
fn a_matching_tag_on_another_network_is_not_a_match() {
    assert!(!rule_targets_instance(
        &rule("prod-vpc", tags(&["web-server"])),
        &scope("default", &["web-server"], &[])
    ));
}

// ---------------------------------------------------------------------------
// tags
// ---------------------------------------------------------------------------

/// Any one of the rule's tags being carried is enough; the instance does not
/// have to carry them all. `targetTags` is a disjunction on GCP's side, and
/// reading it as a conjunction would drop rules that do apply.
#[test]
fn one_matching_tag_is_enough_and_the_rest_need_not_match() {
    assert!(rule_targets_instance(
        &rule("default", tags(&["web-server", "bastion"])),
        &scope("default", &["bastion", "unrelated"], &[])
    ));
}

/// A tag-scoped rule does not reach an instance carrying none of its tags.
#[test]
fn a_tag_scoped_rule_misses_an_instance_with_none_of_its_tags() {
    assert!(!rule_targets_instance(
        &rule("default", tags(&["web-server"])),
        &scope("default", &["database"], &[])
    ));
    // And specifically not an untagged instance, which is the case that would
    // pass if an empty carried list were read as "matches anything".
    assert!(!rule_targets_instance(
        &rule("default", tags(&["web-server"])),
        &scope("default", &[], &[])
    ));
}

/// Comparison is exact. A tag that merely contains or resembles the selector
/// is a different tag, and claiming otherwise would put a rule on an instance
/// it does not govern.
#[test]
fn tag_matching_is_exact_rather_than_a_prefix_or_case_fold() {
    for carried in ["web-server-2", "web", "WEB-SERVER"] {
        assert!(
            !rule_targets_instance(
                &rule("default", tags(&["web-server"])),
                &scope("default", &[carried], &[])
            ),
            "{carried:?} must not match the tag web-server"
        );
    }
}

// ---------------------------------------------------------------------------
// service accounts
// ---------------------------------------------------------------------------

/// The service-account selector reads the instance's service accounts, not its
/// tags. The two lists are separate inputs and a matcher that searched both
/// for either selector would match an instance whose *tag* happened to equal
/// an account email.
#[test]
fn a_service_account_scoped_rule_reads_accounts_and_not_tags() {
    let email = "workload@nuvem-fixture-project.iam.gserviceaccount.com";

    assert!(rule_targets_instance(
        &rule("default", accounts(&[email])),
        &scope("default", &[], &[email])
    ));
    assert!(!rule_targets_instance(
        &rule("default", accounts(&[email])),
        &scope("default", &[email], &[])
    ));
}

/// And the mirror: a tag selector does not read the account list.
#[test]
fn a_tag_scoped_rule_does_not_read_the_service_account_list() {
    assert!(!rule_targets_instance(
        &rule("default", tags(&["web-server"])),
        &scope("default", &[], &["web-server"])
    ));
}

// ---------------------------------------------------------------------------
// the document the API does not send
// ---------------------------------------------------------------------------

/// SYNTHETIC — a rule carrying both selectors, which Google's reference says
/// cannot happen. It is representable because refusing to represent it would
/// mean the parser guessing which half to keep.
///
/// Either half matching is enough. That is the reading that cannot drop a rule
/// from a list an operator is reading for safety: the failure mode is a rule
/// shown that does not apply, rather than a DENY silently missing.
#[test]
fn a_rule_carrying_both_selectors_matches_on_either_of_them() {
    let email = "workload@example.iam.gserviceaccount.com";
    let both = FirewallTargets::Both {
        tags: vec!["web-server".to_string()],
        service_accounts: vec![email.to_string()],
    };

    assert!(rule_targets_instance(
        &rule("default", both.clone()),
        &scope("default", &["web-server"], &[])
    ));
    assert!(rule_targets_instance(
        &rule("default", both.clone()),
        &scope("default", &[], &[email])
    ));
    // Neither half matching is still not a match: "either" is a disjunction,
    // not "always".
    assert!(!rule_targets_instance(
        &rule("default", both),
        &scope("default", &["database"], &[])
    ));
}

// ---------------------------------------------------------------------------
// what the answer deliberately ignores
// ---------------------------------------------------------------------------

/// Targeting is not reachability. A disabled rule and a DENY rule both still
/// *target* the instance, and saying otherwise would hide them from the one
/// list an operator opens to find out what governs a workload.
///
/// The caller keeps rendering action, direction and disabled state; this
/// function has no opinion about them, and this test is what stops one being
/// smuggled in as a filter later.
#[test]
fn targeting_ignores_action_direction_and_whether_the_rule_is_switched_off() {
    let mut disabled_deny = rule("default", FirewallTargets::AllInstances);
    disabled_deny.disabled = true;
    disabled_deny.action = FirewallAction::Deny;
    disabled_deny.direction = FirewallDirection::Egress;

    assert!(rule_targets_instance(
        &disabled_deny,
        &scope("default", &[], &[])
    ));
}

// ---------------------------------------------------------------------------
// what the TARGETS cell and the filter read
// ---------------------------------------------------------------------------
//
// `values()` lives beside the matcher and is read by neither: the cell and the
// substring filter do not care which kind of selector scopes a rule, and the
// matcher cares about nothing else. It is tested here, in the domain crate,
// rather than through the rendering tests in the main crate, so that the
// domain crate's own test binary pins it: a `nuvem` test only reaches it
// through the rendering, and this function once went unpinned exactly that
// way.

/// The selectors a rule carries, whichever kind they are — and nothing for a
/// rule that carries none.
///
/// The empty answer is what makes the cell paint "all instances" instead of a
/// blank, so it is a real answer rather than a missing one.
#[test]
fn the_selector_values_are_the_ones_the_rule_carries() {
    assert!(FirewallTargets::AllInstances.values().is_empty());
    assert_eq!(
        tags(&["web-server", "bastion"]).values(),
        vec!["web-server", "bastion"]
    );
    assert_eq!(
        accounts(&["workload@example.iam.gserviceaccount.com"]).values(),
        vec!["workload@example.iam.gserviceaccount.com"]
    );
}

/// A rule carrying both selectors reports both halves, tags first.
///
/// Dropping either half would hide a selector from the one cell an operator
/// reads to see what a rule is scoped to — and this is the rule whose scope is
/// already unclear, so it is the last one that should be shown partially.
#[test]
fn a_rule_carrying_both_selectors_reports_every_selector_it_carries() {
    let both = FirewallTargets::Both {
        tags: vec!["web-server".to_string()],
        service_accounts: vec!["workload@example.iam.gserviceaccount.com".to_string()],
    };

    assert_eq!(
        both.values(),
        vec!["web-server", "workload@example.iam.gserviceaccount.com"]
    );
}

// ---------------------------------------------------------------------------
// Building the scope from an instance's detail
// ---------------------------------------------------------------------------

fn detail_with(
    interfaces: Vec<(&str, &str)>,
    tags: Vec<&str>,
    accounts: Vec<&str>,
) -> ComputeInstanceDetail {
    ComputeInstanceDetail {
        name: "web-01".to_string(),
        zone: "europe-west1-b".to_string(),
        status: ResourceStatus::Running,
        machine_type: "e2-medium".to_string(),
        creation_timestamp: None,
        network_interfaces: interfaces
            .into_iter()
            .map(|(network, subnetwork)| ComputeNetworkInterface {
                name: Some("nic".to_string()),
                network: Some(network.to_string()),
                subnetwork: Some(subnetwork.to_string()),
                subnetwork_region: Some("europe-west1".to_string()),
                internal_ip: None,
                external_ip: None,
            })
            .collect(),
        tags: tags.into_iter().map(str::to_string).collect(),
        service_accounts: accounts.into_iter().map(str::to_string).collect(),
        disks: Vec::new(),
        labels: Vec::new(),
        metadata: Vec::new(),
        raw_json: "{}".to_string(),
    }
}

/// The scope is read off the instance's *primary* interface, the same one
/// `{network}` and `{internal_ip}` already describe.
///
/// An instance with several interfaces sits on several networks, and a rule
/// list built from the wrong one would be about a network the operator is not
/// looking at. Naming the others needs a per-interface vocabulary nothing here
/// has, so the primary is the whole answer and this test says so.
#[test]
fn the_scope_of_an_instance_is_its_primary_interface_and_its_own_identity() {
    let detail = detail_with(
        vec![("prod-vpc", "prod-subnet"), ("mgmt-vpc", "mgmt-subnet")],
        vec!["web-server"],
        vec!["workload@example.iam.gserviceaccount.com"],
    );

    let scope = FirewallInstanceScope::from_detail(&detail).expect("a networked instance has one");
    assert_eq!(
        scope,
        FirewallInstanceScope {
            network: "prod-vpc".to_string(),
            tags: vec!["web-server".to_string()],
            service_accounts: vec!["workload@example.iam.gserviceaccount.com".to_string()],
        }
    );
}

/// An instance with no network interface has no scope, so no rule list can be
/// built for it — `None` rather than a scope on an empty network, which would
/// match every rule whose own network happened to be empty.
#[test]
fn an_instance_with_no_network_has_no_scope() {
    assert_eq!(
        FirewallInstanceScope::from_detail(&detail_with(Vec::new(), Vec::new(), Vec::new())),
        None
    );

    // ...and neither does an interface that carries no network id.
    let mut detail = detail_with(vec![("prod-vpc", "prod-subnet")], Vec::new(), Vec::new());
    detail.network_interfaces[0].network = None;
    assert_eq!(FirewallInstanceScope::from_detail(&detail), None);
}
