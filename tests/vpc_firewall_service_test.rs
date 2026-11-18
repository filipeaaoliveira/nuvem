//! The firewall-rules half of the VPC service layer (slice 3).
//!
//! # The discrimination this slice exists around
//!
//! A firewall rule carries `allowed` **or** `denied` — never both, never
//! neither. That is a fact about the API, and the shape a naive
//! `Option<Vec<_>>` pair gets wrong: two independent options can represent
//! four states, three of which the API never sends, and code reading them has
//! to decide what "both" and "neither" mean every time it touches them. One
//! field that can hold one answer cannot be read wrong.
//!
//! Getting it wrong is not a cosmetic defect. A `DENY` rule painted `ALLOW` —
//! or a rule with neither key defaulted to `ALLOW` — tells an operator that
//! traffic is permitted when it is blocked, or the reverse, on the one table
//! in this application whose whole job is to say what reaches a workload.
//!
//! # Fixture provenance — read before trusting a case below
//!
//! `tests/fixtures/vpc_firewall_rules.json` is **mixed**, and it is the only
//! mixed fixture in this epic:
//!
//! * the four `default-allow-*` rules are **captured** from live gcloud on
//!   2026-08-25, scrubbed only by replacing the project id;
//! * `allow-web-to-tagged`, `deny-all-egress` and `disabled-sa-scoped-ssh` are
//!   **constructed** from Google's published `compute/v1/firewalls`
//!   reference.
//!
//! The account this repo captures from can reach no EGRESS rule, no `denied`
//! rule, no tag-scoped rule and no service-account-scoped rule. So the
//! captured half exercises exactly one shape — `INGRESS`/`allowed`/enabled/
//! source-ranges — and **every other arm this slice adds rests on the
//! reference**. Each test below says which it is standing on.
//!
//! # A third label, and why it is not a weaker kind of capture
//!
//! Some tests below are marked `SYNTHETIC` rather than `CONSTRUCTED`. The
//! distinction is not a shade of the same thing. A `CONSTRUCTED` row stands in
//! for a capture this account cannot take — it is a transcription, and if a
//! reachable project ever grew an egress rule it would be replaced by the real
//! bytes. A `SYNTHETIC` document is one no real project can ever produce, and
//! it is here because a capture *cannot do its job*.
//!
//! The direction-to-ranges match is the worked example. Every real rule
//! populates exactly the list its direction calls for: INGRESS carries
//! `sourceRanges`, EGRESS carries `destinationRanges`, neither carries the
//! other. So a parser that ignored the direction entirely and took whichever
//! list was non-empty agrees with a capture on every single row. The two
//! readings — "the direction chooses the list" and "take whatever is
//! populated" — are indistinguishable on real input and part company only on
//! documents the API does not send. Slice 3's first mutation sweep found
//! exactly that: five survivors in that one match, none of them reachable from
//! the fixture.
//!
//! Generalised: a capture contains only states the real system produces, so
//! where two code paths agree on every real input and differ only on
//! impossible ones, no capture discriminates them — and the more faithful the
//! capture, the more certainly it cannot. The discriminating document has to
//! be built by hand. That is not a shortcut around getting a real one; it is
//! the only thing that can tell the two implementations apart at all.

use nuvem::services::gcloud::cli::{CommandError, CommandOutput, CommandRunner, GcloudCommand};
use nuvem::services::resources::vpc::{GcloudVpcService, parse_firewall_rules_json};
use nuvem_domain::gcp::configuration::{GcloudConfigurationName, GcloudContext};
use nuvem_domain::gcp::resource::{FirewallAction, FirewallDirection, FirewallTargets};
use std::cell::RefCell;

const FIREWALL_JSON: &str = include_str!("fixtures/vpc_firewall_rules.json");

fn ctx() -> GcloudContext {
    GcloudContext {
        configuration: GcloudConfigurationName::new("dev"),
        account: None,
        project: Some("dev-project".into()),
        region: None,
        zone: None,
    }
}

struct ScriptedRunner {
    stdout: String,
    seen: RefCell<Vec<Vec<String>>>,
}

impl ScriptedRunner {
    fn ok(stdout: &str) -> Self {
        Self {
            stdout: stdout.to_string(),
            seen: RefCell::new(Vec::new()),
        }
    }

    fn calls(&self) -> Vec<Vec<String>> {
        self.seen.borrow().clone()
    }
}

impl CommandRunner for ScriptedRunner {
    fn run(&self, command: GcloudCommand) -> Result<CommandOutput, CommandError> {
        self.seen.borrow_mut().push(command.args.clone());
        Ok(CommandOutput {
            stdout: self.stdout.clone(),
            stderr: String::new(),
        })
    }
}

#[test]
fn a_firewall_load_issues_one_flat_project_wide_call() {
    let runner = ScriptedRunner::ok(FIREWALL_JSON);
    let service = GcloudVpcService::new(&runner);

    service
        .load_firewall_rules(&ctx(), None)
        .expect("firewall load");

    assert_eq!(
        runner.calls(),
        vec![vec![
            "--configuration".to_string(),
            "dev".to_string(),
            "--project".to_string(),
            "dev-project".to_string(),
            "compute".to_string(),
            "firewall-rules".to_string(),
            "list".to_string(),
            "--format=json".to_string(),
            "--limit=500".to_string(),
        ]],
        "firewall rules are a flat project-wide listing"
    );
}

/// The captured rules, which are all one shape.
///
/// CAPTURED. Four `default-allow-*` INGRESS rules with `allowed`, enabled,
/// scoped by source range, at the default priority. Everything this asserts
/// came off the wire.
#[test]
fn parses_the_captured_default_rules() {
    let rules = parse_firewall_rules_json(FIREWALL_JSON).expect("the fixture parses");

    let ssh = rules
        .iter()
        .find(|rule| rule.name == "default-allow-ssh")
        .expect("the capture holds default-allow-ssh");

    assert_eq!(ssh.network, "default", "the self-link is reduced to its id");
    assert_eq!(ssh.direction, FirewallDirection::Ingress);
    assert_eq!(ssh.action, FirewallAction::Allow);
    assert_eq!(ssh.priority, Some(65534));
    assert!(!ssh.disabled);
    assert_eq!(ssh.source_or_dest, vec!["0.0.0.0/0".to_string()]);
    assert_eq!(
        ssh.targets,
        FirewallTargets::AllInstances,
        "a rule with no tag or service-account scoping targets every instance \
         in its network, which the type names rather than leaving as an empty \
         list for each reader to interpret"
    );
}

/// ALLOW and DENY are *which key is present*, and the two are exclusive.
///
/// CONSTRUCTED for the DENY half: `deny-all-egress` is modelled from the
/// published reference, because no project this account can reach has a
/// `denied` rule at all.
#[test]
fn the_action_is_whichever_of_allowed_or_denied_the_rule_carries() {
    let rules = parse_firewall_rules_json(FIREWALL_JSON).expect("the fixture parses");

    let deny = rules
        .iter()
        .find(|rule| rule.name == "deny-all-egress")
        .expect("the fixture holds the constructed deny rule");
    assert_eq!(deny.action, FirewallAction::Deny);
    assert_eq!(deny.direction, FirewallDirection::Egress);

    let allow_count = rules
        .iter()
        .filter(|rule| rule.action == FirewallAction::Allow)
        .count();
    assert_eq!(allow_count, 6, "every other rule in the fixture allows");
}

/// A rule carrying **neither** key is unspecified, not allow.
///
/// CONSTRUCTED, and deliberately a document the API does not send. Defaulting
/// to `Allow` is the tempting reading — `allowed` is the common case — and it
/// is the one that puts "traffic permitted" on screen for a rule that says no
/// such thing. On this table that is the most dangerous possible wrong answer,
/// so an unreadable rule reads as unreadable.
#[test]
fn a_rule_carrying_neither_key_is_unspecified_rather_than_allow() {
    let json = r#"[
      { "name": "neither", "direction": "INGRESS", "priority": 1000 }
    ]"#;

    let rules = parse_firewall_rules_json(json).expect("the row parses");

    assert_eq!(rules[0].action, FirewallAction::Unspecified);
    assert_ne!(
        rules[0].action,
        FirewallAction::Allow,
        "a rule that says neither must never read as permitting traffic"
    );
}

/// A rule carrying **both** keys is unspecified too.
///
/// CONSTRUCTED, and impossible: the API never sends both. Which is exactly why
/// it is here — a parser that read `allowed` first and stopped would call this
/// rule ALLOW, and a document we cannot read is not a document that permits
/// traffic. Reporting it as unspecified is refusing to guess, on the one table
/// where guessing wrong is a security misread.
#[test]
fn a_rule_carrying_both_keys_refuses_to_guess() {
    let json = r#"[
      { "name": "contradiction", "direction": "INGRESS",
        "allowed": [{"IPProtocol": "tcp"}],
        "denied": [{"IPProtocol": "tcp"}] }
    ]"#;

    let rules = parse_firewall_rules_json(json).expect("the row parses");

    assert_eq!(rules[0].action, FirewallAction::Unspecified);
}

/// `disabled` reaches the domain as a bool, and the captured rules are all
/// enabled while the constructed one is not.
///
/// CONSTRUCTED for the disabled case. This is the flag whose absence from the
/// screen would be the worst outcome of this slice: a disabled DENY rule reads
/// exactly like a live DENY rule, so an operator would believe traffic is
/// blocked that is in fact flowing.
#[test]
fn the_disabled_flag_survives_the_parse() {
    let rules = parse_firewall_rules_json(FIREWALL_JSON).expect("the fixture parses");

    let disabled: Vec<&str> = rules
        .iter()
        .filter(|rule| rule.disabled)
        .map(|rule| rule.name.as_str())
        .collect();

    assert_eq!(
        disabled,
        vec!["disabled-sa-scoped-ssh"],
        "exactly the constructed disabled rule is disabled"
    );
}

/// INGRESS reads its source ranges; EGRESS reads its destination ranges.
///
/// One column shows one of the two, because a rule only ever has the one that
/// matches its direction — and a table with both columns would be half empty
/// on every row.
#[test]
fn the_source_or_dest_column_follows_the_direction() {
    let rules = parse_firewall_rules_json(FIREWALL_JSON).expect("the fixture parses");

    let ingress = rules
        .iter()
        .find(|rule| rule.name == "default-allow-internal")
        .expect("captured ingress rule");
    assert_eq!(ingress.source_or_dest, vec!["10.128.0.0/9".to_string()]);

    let egress = rules
        .iter()
        .find(|rule| rule.name == "deny-all-egress")
        .expect("constructed egress rule");
    assert_eq!(
        egress.source_or_dest,
        vec!["0.0.0.0/0".to_string()],
        "an EGRESS rule's cell is its destinationRanges"
    );
}

/// The direction chooses the list, and it chooses it even when the *other*
/// list is the populated one.
///
/// SYNTHETIC, and deliberately so. The fixture cannot discriminate here: every
/// real rule populates exactly the list its direction calls for, so a parser
/// that ignored the direction entirely and simply took whichever list was
/// non-empty would agree with the fixture on every row. These two documents
/// are the ones where the two readings disagree, which is the only place the
/// rule "the direction chooses" is observable.
///
/// The right answer is the empty cell. An INGRESS rule that reported only
/// destination ranges has told us nothing about its sources, and painting its
/// destinations under a cell an operator reads as "where this traffic comes
/// from" would be worse than a dash — it would name the wrong end of the
/// connection on a security table.
#[test]
fn the_direction_chooses_the_list_even_when_the_other_one_is_the_populated_one() {
    let ingress_with_only_destinations = parse_firewall_rules_json(
        r#"[{
            "name": "ingress-reporting-destinations",
            "direction": "INGRESS",
            "destinationRanges": ["10.0.0.0/8"]
        }]"#,
    )
    .expect("the row parses");
    assert!(
        ingress_with_only_destinations[0].source_or_dest.is_empty(),
        "an INGRESS rule's cell is its sourceRanges; its destinations are not \
         a stand-in for them: {:?}",
        ingress_with_only_destinations[0].source_or_dest
    );

    let egress_with_only_sources = parse_firewall_rules_json(
        r#"[{
            "name": "egress-reporting-sources",
            "direction": "EGRESS",
            "sourceRanges": ["10.0.0.0/8"]
        }]"#,
    )
    .expect("the row parses");
    assert!(
        egress_with_only_sources[0].source_or_dest.is_empty(),
        "and an EGRESS rule's cell is its destinationRanges: {:?}",
        egress_with_only_sources[0].source_or_dest
    );
}

/// A rule whose direction this build cannot read still shows its ranges.
///
/// SYNTHETIC. The two arms above are keyed on a direction word; a rule whose
/// direction is absent or is a spelling Google added after this build matches
/// neither, and the ranges it reported are still true. So the fallback takes
/// whichever list is populated — sources first, because an unreadable
/// direction is far more likely to be an ingress rule (they outnumber egress
/// rules in every project) and because a wrong *label* on a correct range is
/// recoverable where a blank cell is not.
///
/// Both halves are needed: the sources case alone passes over a fallback that
/// always takes sources, and the destinations case alone passes over one that
/// always takes destinations.
#[test]
fn an_unreadable_direction_still_shows_whichever_ranges_the_rule_reported() {
    let with_sources = parse_firewall_rules_json(
        r#"[{
            "name": "sideways-with-sources",
            "direction": "SIDEWAYS",
            "sourceRanges": ["10.0.0.0/8"]
        }]"#,
    )
    .expect("the row parses");
    assert_eq!(
        with_sources[0].source_or_dest,
        vec!["10.0.0.0/8".to_string()],
        "the direction word is unknown; the ranges are not"
    );

    let with_destinations = parse_firewall_rules_json(
        r#"[{
            "name": "no-direction-with-destinations",
            "destinationRanges": ["192.168.0.0/16"]
        }]"#,
    )
    .expect("the row parses");
    assert_eq!(
        with_destinations[0].source_or_dest,
        vec!["192.168.0.0/16".to_string()],
        "a rule reporting no direction at all falls back to the list it did \
         populate, rather than to the empty one"
    );
}

/// Targets come from tags or service accounts, whichever the rule scopes by.
///
/// CONSTRUCTED, both cases: no reachable project has either.
#[test]
fn targets_keep_which_kind_of_selector_scopes_the_rule() {
    let rules = parse_firewall_rules_json(FIREWALL_JSON).expect("the fixture parses");

    let tagged = rules
        .iter()
        .find(|rule| rule.name == "allow-web-to-tagged")
        .expect("constructed tag-scoped rule");
    assert_eq!(
        tagged.targets,
        FirewallTargets::Tags(vec!["web-server".to_string()])
    );

    let scoped = rules
        .iter()
        .find(|rule| rule.name == "disabled-sa-scoped-ssh")
        .expect("constructed service-account-scoped rule");
    assert_eq!(
        scoped.targets,
        FirewallTargets::ServiceAccounts(vec![
            "workload@nuvem-fixture-project.iam.gserviceaccount.com".to_string()
        ]),
        "a service-account-scoped rule targets that account, and the parser \
         keeps that it is an account rather than a tag"
    );
}

/// SYNTHETIC — both selectors on one rule, which Google's `compute/v1/firewalls`
/// reference documents as impossible ("targetServiceAccounts and targetTags
/// cannot be used at the same time").
///
/// It is represented rather than resolved for the reason `FirewallAction`
/// names its own impossible pair: picking one half would be a guess about a
/// rule whose scope is genuinely unclear, and the guess would be invisible.
#[test]
fn a_rule_carrying_both_selectors_is_named_rather_than_resolved() {
    let json = r#"[
        {
            "name": "impossible",
            "network": "https://www.googleapis.com/compute/v1/projects/p/global/networks/default",
            "direction": "INGRESS",
            "allowed": [{"IPProtocol": "tcp"}],
            "targetTags": ["web-server"],
            "targetServiceAccounts": ["workload@example.iam.gserviceaccount.com"]
        }
    ]"#;

    let rules = parse_firewall_rules_json(json).expect("the synthetic document parses");
    assert_eq!(
        rules[0].targets,
        FirewallTargets::Both {
            tags: vec!["web-server".to_string()],
            service_accounts: vec!["workload@example.iam.gserviceaccount.com".to_string()],
        }
    );
}

/// SYNTHETIC — an explicitly empty selector array, which no captured rule
/// carries: every rule in the fixture either omits the key or lists at least
/// one entry.
///
/// It reads as `AllInstances`, the same as an absent key. That is an
/// assumption resting on the reference's "if neither is specified, the rule
/// applies to all instances", not on bytes the API sent, and this test is
/// where it is written down so that changing it is a deliberate act.
#[test]
fn an_empty_selector_array_reads_the_same_as_an_absent_one() {
    let json = r#"[
        {
            "name": "empty-selectors",
            "network": "https://www.googleapis.com/compute/v1/projects/p/global/networks/default",
            "direction": "INGRESS",
            "allowed": [{"IPProtocol": "tcp"}],
            "targetTags": [],
            "targetServiceAccounts": []
        }
    ]"#;

    let rules = parse_firewall_rules_json(json).expect("the synthetic document parses");
    assert_eq!(rules[0].targets, FirewallTargets::AllInstances);
}

/// An unrecognised direction is carried rather than collapsed to ingress.
#[test]
fn an_unrecognised_direction_is_carried_verbatim() {
    let json = r#"[{ "name": "n", "direction": "SIDEWAYS" }]"#;

    let rules = parse_firewall_rules_json(json).expect("the row parses");

    assert_eq!(
        rules[0].direction,
        FirewallDirection::Unrecognized("SIDEWAYS".to_string())
    );
}

/// A row missing every optional field still parses; a non-JSON document does
/// not.
#[test]
fn a_row_missing_every_optional_field_still_parses() {
    let rules = parse_firewall_rules_json(r#"[{ "name": "bare" }]"#).expect("a bare row parses");

    assert_eq!(rules[0].name, "bare");
    assert_eq!(rules[0].network, "");
    assert_eq!(rules[0].priority, None);
    assert!(!rules[0].disabled);
    assert_eq!(rules[0].action, FirewallAction::Unspecified);

    parse_firewall_rules_json("not json").expect_err("a non-JSON document fails the load");
}
