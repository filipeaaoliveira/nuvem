//! The firewall rules slice of `ResourcesState` (slice 3).
//!
//! The three thin seams pinned up front here are the ones slice 1's sweep
//! found unguarded and slice 2 pre-empted: the `ResourceService` impl's
//! `matches_filter` (no production caller reaches it — `visible_firewall_rules`
//! calls the free function, and the generic `visible_rows::<S>` path is
//! entered only by the marks machinery), the `visible_*_count` accessor (no
//! generic caller at all), and the cursor comparison in `set_*`, which needs a
//! reload whose ordering changed before it means anything.
//!
//! The `column`/`from_column` round trip is **not** here: it is covered for
//! every service, including this one, by `sort_column_invariant_test`.
//!
//! ## Fixture provenance
//!
//! The four `default-allow-*` rules are the shape a live capture produced
//! (2026-08-25). Everything interesting below is CONSTRUCTED from Google's
//! published `compute/v1/firewalls` reference, because no project this account
//! can reach has one: EGRESS, `denied`, target tags, service-account scoping,
//! and `disabled: true`. Two of those are the ones a wrong render endangers —
//! a DENY read as an ALLOW, and a disabled rule read as a live one — so they
//! are asserted here rather than left to the parser tests alone.

use nuvem::app::state::resources::{
    DISABLED_BADGE, FirewallRules, FirewallSortField, ResourceService, ResourcesState,
};
use nuvem_domain::gcp::configuration::{GcloudConfigurationName, GcloudContext};
use nuvem_domain::gcp::diagnostics::{GcloudDiagnostic, GcloudErrorKind};
use nuvem_domain::gcp::filter::compile_filter;
use nuvem_domain::gcp::resource::{
    FirewallAction, FirewallDirection, FirewallRule, FirewallTargets, ResourceServiceKind,
};

fn ctx() -> GcloudContext {
    GcloudContext {
        configuration: GcloudConfigurationName::new("dev"),
        account: None,
        project: Some("dev-project".into()),
        region: None,
        zone: None,
    }
}

/// An INGRESS ALLOW rule scoped by source range: the captured shape.
fn allow(name: &str, priority: u32) -> FirewallRule {
    FirewallRule {
        name: name.to_string(),
        network: "default".to_string(),
        direction: FirewallDirection::Ingress,
        priority: Some(priority),
        action: FirewallAction::Allow,
        targets: FirewallTargets::AllInstances,
        source_or_dest: vec!["0.0.0.0/0".to_string()],
        disabled: false,
    }
}

/// CONSTRUCTED: an EGRESS DENY rule scoped by destination range.
fn deny_egress(name: &str, priority: u32) -> FirewallRule {
    FirewallRule {
        name: name.to_string(),
        network: "default".to_string(),
        direction: FirewallDirection::Egress,
        priority: Some(priority),
        action: FirewallAction::Deny,
        targets: FirewallTargets::AllInstances,
        source_or_dest: vec!["0.0.0.0/0".to_string()],
        disabled: false,
    }
}

/// Moves the column cursor to `column` and sorts by it.
///
/// `sort_selected_column` acts on wherever the column cursor already is, and
/// `select_next_column` clamps rather than wrapping, so stepping right from
/// zero is the only way to land on a chosen column deterministically.
fn sort_by_column(state: &mut ResourcesState, column: usize) {
    for _ in 0..column {
        state.select_next_column();
    }
    state.sort_selected_column();
}

fn state_with(rules: Vec<FirewallRule>) -> ResourcesState {
    let mut state = ResourcesState::default();
    state.select_service(ResourceServiceKind::FirewallRules);
    state.set_firewall_rules(ctx(), rules);
    state
}

/// One of each shape the table has to tell apart.
fn mixed_rules() -> Vec<FirewallRule> {
    vec![
        allow("default-allow-icmp", 65534),
        deny_egress("deny-all-egress", 65535),
        // CONSTRUCTED: disabled, and scoped to a service account.
        FirewallRule {
            name: "disabled-sa-scoped-ssh".to_string(),
            network: "default".to_string(),
            direction: FirewallDirection::Ingress,
            priority: Some(900),
            action: FirewallAction::Allow,
            targets: FirewallTargets::Tags(vec![
                "bastion@nuvem-fixture-project.iam.gserviceaccount.com".to_string(),
            ]),
            source_or_dest: Vec::new(),
            disabled: true,
        },
    ]
}

// --- Identity ---

/// A firewall rule is global, so its identity is the name alone.
///
/// This is the networks shape rather than the subnets one, and the difference
/// is load-bearing: a rule name is unique across the whole project, even
/// across networks, so qualifying it with the network would invent a
/// distinction the API does not make.
#[test]
fn a_rules_identity_is_its_name_with_no_location_to_qualify_it() {
    let id = FirewallRules::row_id(&allow("default-allow-icmp", 65534));

    assert_eq!(id.name, "default-allow-icmp");
    assert!(
        id.location.is_empty(),
        "a firewall rule has no location; inventing one would key marks and \
         remembered views on a field the API never reports"
    );
}

// --- The trait seam ---

/// `FirewallRules::matches_filter` through the trait, not the free function.
///
/// Nothing production-side reaches this impl. It is a trait obligation with no
/// caller, which is the kind a later refactor gets wrong the first time a
/// generic helper starts dispatching by kind. Asserting a match *and* a
/// non-match on one row excludes both constant answers.
#[test]
fn the_resource_service_impl_filters_rather_than_answering_a_constant() {
    let row = allow("default-allow-icmp", 65534);
    let matching = compile_filter("icmp").expect("a literal needle compiles");
    let missing = compile_filter("zzz-nowhere").expect("a literal needle compiles");

    assert!(FirewallRules::matches_filter(&row, &matching));
    assert!(!FirewallRules::matches_filter(&row, &missing));
}

/// `/DENY` finds every blocking rule, which nothing in the raw row supports.
///
/// The API reports a `denied` *key*, not the word `DENY`; the word exists only
/// because the searchable set contributes the action's label. Without it an
/// operator auditing what a project blocks would type the one word the screen
/// shows them and get nothing.
#[test]
fn the_filter_narrows_by_the_action_word_that_exists_only_on_screen() {
    let mut state = state_with(mixed_rules());

    state.start_filter();
    for character in "DENY".chars() {
        state.append_filter_char(character);
    }
    state.apply_filter();

    let names: Vec<&str> = state
        .visible_firewall_rules()
        .into_iter()
        .map(|rule| rule.name.as_str())
        .collect();
    assert_eq!(names, vec!["deny-all-egress"]);
}

/// The `DISABLED` badge is searchable, for the reason `PUBLIC` is on the IAM
/// table: an operator hunting rules that are not in force types the word they
/// can see, and it appears in no field of the row.
#[test]
fn the_filter_narrows_by_the_disabled_badge() {
    let mut state = state_with(mixed_rules());

    state.start_filter();
    for character in DISABLED_BADGE.chars() {
        state.append_filter_char(character);
    }
    state.apply_filter();

    let names: Vec<&str> = state
        .visible_firewall_rules()
        .into_iter()
        .map(|rule| rule.name.as_str())
        .collect();
    assert_eq!(names, vec!["disabled-sa-scoped-ssh"]);
}

/// `all instances` is searchable, which is how an operator finds the widest
/// rules in the project.
///
/// An empty target list means the rule applies to every instance in the
/// network. The words are the cell's, not the row's, so a searchable set built
/// from raw fields alone would leave the broadest rules unfindable.
#[test]
fn the_filter_narrows_by_the_words_the_unscoped_targets_cell_paints() {
    let mut state = state_with(mixed_rules());

    state.start_filter();
    for character in "all instances".chars() {
        state.append_filter_char(character);
    }
    state.apply_filter();

    let names: Vec<&str> = state
        .visible_firewall_rules()
        .into_iter()
        .map(|rule| rule.name.as_str())
        .collect();
    assert_eq!(
        names,
        vec!["default-allow-icmp", "deny-all-egress"],
        "the two unscoped rules, and not the service-account-scoped one"
    );
}

// --- The visible count ---

/// The `filtered`/`total` the row counter paints, which nothing else reads.
#[test]
fn the_visible_count_counts_the_rows_the_filter_leaves() {
    let mut state = state_with(mixed_rules());

    assert_eq!(state.visible_firewall_rule_count(), 3);

    state.start_filter();
    for character in "egress".chars() {
        state.append_filter_char(character);
    }
    state.apply_filter();

    assert_eq!(state.visible_firewall_rule_count(), 1);
    assert_eq!(
        state.visible_firewall_rule_count(),
        state.visible_firewall_rules().len(),
        "the count and the list are two readings of one filter"
    );
}

// --- The cursor across a reload ---

/// A reload keeps the cursor on the same rule rather than on the same row
/// index.
///
/// Pinned with a reload that inserts a rule sorting ahead of the selected one,
/// because a `set_*` that ignored the remembered name would look correct
/// against any reload that returned the same sequence.
#[test]
fn a_reload_keeps_the_cursor_on_the_same_rule() {
    let mut state = state_with(mixed_rules());

    // Default ordering is by name: default-allow-icmp, deny-all-egress,
    // disabled-sa-scoped-ssh. One step reaches `deny-all-egress`.
    state.select_next();
    assert_eq!(
        state.selected_firewall_rule().map(|r| r.name.as_str()),
        Some("deny-all-egress"),
        "precondition: the cursor is on the row this test follows"
    );

    let mut reloaded = mixed_rules();
    reloaded.push(allow("aaa-first-alphabetically", 1000));
    state.set_firewall_rules(ctx(), reloaded);

    assert_eq!(
        state.selected_firewall_rule().map(|r| r.name.as_str()),
        Some("deny-all-egress"),
        "the cursor follows the rule, not the index it happened to sit at"
    );
}

/// A rule that is gone after the reload leaves the cursor in range.
#[test]
fn a_reload_that_drops_the_selected_rule_clamps_the_cursor() {
    let mut state = state_with(mixed_rules());
    state.select_next();
    state.select_next();

    state.set_firewall_rules(ctx(), vec![allow("only", 1000)]);

    assert_eq!(
        state.selected_firewall_rule().map(|r| r.name.as_str()),
        Some("only")
    );
}

// --- Sorting ---

/// `PRIORITY` sorts numerically, which is the order traffic meets the rules in.
///
/// Lower priority wins, so ascending by this column is evaluation order — the
/// reason an operator sorts by it at all. A text sort would put `1000` ahead
/// of `900` and misrepresent which rule fires first, and the mistake is
/// invisible on any project whose priorities happen to share a digit count.
#[test]
fn priority_sorts_numerically_so_the_order_is_evaluation_order() {
    let mut state = state_with(vec![
        allow("nine-hundred", 900),
        allow("one-thousand", 1000),
        allow("sixty-five-thousand", 65534),
    ]);

    sort_by_column(&mut state, FirewallSortField::Priority.column());

    let priorities: Vec<Option<u32>> = state
        .visible_firewall_rules()
        .into_iter()
        .map(|rule| rule.priority)
        .collect();

    assert_eq!(priorities, vec![Some(900), Some(1000), Some(65534)]);
}

/// A rule with no reported priority sorts as absent, not as zero.
///
/// Zero would put it ahead of every real rule and claim it is evaluated first,
/// which is a statement about a value the listing never reported.
#[test]
fn a_rule_with_no_priority_does_not_claim_to_be_evaluated_first() {
    let unreported = FirewallRule {
        priority: None,
        ..allow("no-priority-reported", 0)
    };
    let mut state = state_with(vec![allow("has-priority", 900), unreported]);

    sort_by_column(&mut state, FirewallSortField::Priority.column());

    let names: Vec<&str> = state
        .visible_firewall_rules()
        .into_iter()
        .map(|rule| rule.name.as_str())
        .collect();
    assert_eq!(
        names,
        vec!["no-priority-reported", "has-priority"],
        "None sorts before Some, which is `absent`, not `zero`: the row says \
         nothing was reported rather than that 0 was"
    );
}

// --- The problem accessor ---

/// A refusal for the active context is reportable.
///
/// The positive half of the pair: a negative assertion about this accessor is
/// worth nothing unless something proves the field can be filled at all.
#[test]
fn a_failure_for_this_context_becomes_the_tables_problem() {
    let mut state = state_with(mixed_rules());
    assert!(state.firewall_rules_problem().is_none());

    state.set_service_diagnostic(
        ResourceServiceKind::FirewallRules,
        GcloudDiagnostic::new(
            GcloudErrorKind::PermissionDenied,
            "Firewall rules",
            None,
            None,
            "compute.firewalls.list denied on dev-project",
        ),
    );

    assert_eq!(
        state
            .firewall_rules_problem()
            .expect("a diagnostic for this service is reportable")
            .kind,
        GcloudErrorKind::PermissionDenied
    );
}

// --- The sort vocabulary's labels ---

/// Each field's label, which is what the sort indicator paints.
#[test]
fn every_sort_field_has_its_own_label() {
    let labels = [
        FirewallSortField::Name.label(),
        FirewallSortField::Network.label(),
        FirewallSortField::Direction.label(),
        FirewallSortField::Priority.label(),
        FirewallSortField::Action.label(),
        FirewallSortField::Targets.label(),
        FirewallSortField::SourceOrDest.label(),
    ];

    assert_eq!(
        labels,
        [
            "Name",
            "Network",
            "Direction",
            "Priority",
            "Action",
            "Targets",
            "Source/dest"
        ]
    );
}
