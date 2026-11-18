//! `[[jumps]]` as it behaves through a real config file (task 1):
//! what reaches `ResolvedConfig`, what reaches the warning list, and what a
//! broken rule is and is not allowed to take down with it.

use std::io::Write;

use clap::Parser;
use nuvem::app::state::navigation::JumpTarget;
use nuvem::config::jumps::JumpRules;
use nuvem::config::jumps::JumpSelect;
use nuvem::config::theme::ThemeChoice;
use nuvem::{Cli, Config, ConfigOutcome};
use nuvem_domain::gcp::resource::ResourceServiceKind;

const COMPUTE: ResourceServiceKind = ResourceServiceKind::ComputeInstances;

fn resolve(body: &str) -> (ConfigOutcome, tempfile::NamedTempFile) {
    let mut file = tempfile::NamedTempFile::new().unwrap();
    file.write_all(body.as_bytes()).unwrap();
    let cli = Cli::parse_from([
        "nuvem",
        "--plugins",
        "/nonexistent/nuvem-plugins-test.toml",
        "--config",
        file.path().to_str().unwrap(),
    ]);
    // The temp file is returned so it outlives the resolve call's path.
    (Config::resolve(&cli), file)
}

#[test]
fn a_file_without_a_jumps_array_keeps_the_built_in_rules() {
    let (out, _file) = resolve("version = 1\n[general]\npoll_interval_secs = 12\n");

    assert!(
        out.problems.is_empty(),
        "unexpected: {:?}",
        out.problems.messages()
    );
    assert_eq!(out.config.jumps, JumpRules::defaults());
}

#[test]
fn a_jumps_array_replaces_the_built_in_rules_rather_than_adding_to_them() {
    // Merging would make removing a built-in jump impossible.
    let (out, _file) =
        resolve("version = 1\n[[jumps]]\nfrom = \"compute\"\nto = \"compute:metadata\"\n");

    assert!(
        out.problems.is_empty(),
        "unexpected: {:?}",
        out.problems.messages()
    );
    let keys: Vec<String> = out
        .config
        .jumps
        .for_service(COMPUTE)
        .map(|rule| rule.to.config_key())
        .collect();
    assert_eq!(keys, vec!["compute:metadata".to_string()]);
}

#[test]
fn a_broken_rule_warns_without_failing_the_load_or_the_rest_of_the_file() {
    let (out, _file) = resolve(
        r#"
version = 1
[general]
theme = "no-color"

[[jumps]]
from = "zzz"
to = "compute:disks"

[[jumps]]
from = "cloudsql"
to = "cloudsql:replicas"
"#,
    );

    assert_eq!(out.load_error, None, "a bad rule is never fatal");
    assert_eq!(
        out.problems.messages(),
        vec!["jumps[0]: unknown from service \"zzz\"; ignoring this rule"]
    );
    // The rest of the file still applied.
    assert_eq!(out.config.theme, ThemeChoice::NoColor);
    assert_eq!(out.config.jumps.rules().len(), 1);
    assert_eq!(
        out.config.jumps.rules()[0].to.config_key(),
        "cloudsql:replicas"
    );
}

#[test]
fn a_rule_of_the_wrong_shape_does_not_discard_the_whole_file() {
    // The reason `[[jumps]]` arrives as raw TOML: a type slip inside one rule
    // is a per-rule warning, not a deserialization failure that would take
    // theme, aliases and hotkeys down with it.
    let (out, _file) = resolve(
        "version = 1\n[general]\ntheme = \"no-color\"\n\n[[jumps]]\nfrom = 7\nto = \"compute:disks\"\n",
    );

    assert_eq!(out.load_error, None);
    assert_eq!(out.config.theme, ThemeChoice::NoColor);
    assert_eq!(
        out.problems.messages(),
        vec!["jumps[0]: from must be a string, found integer; ignoring this rule"]
    );
    assert!(out.config.jumps.rules().is_empty());
}

#[test]
fn a_single_bracket_jumps_table_warns_without_discarding_the_rest_of_the_file() {
    // `[[jumps]]` is the file's first array-of-tables, so writing `[jumps]`
    // is the likeliest mistake in the whole schema. It must cost the user
    // their jumps, not their theme, aliases, hotkeys and columns.
    let (out, _file) = resolve(
        "version = 1\n[general]\ntheme = \"no-color\"\n\n[jumps]\nfrom = \"compute\"\nto = \"compute:disks\"\n",
    );

    assert_eq!(out.load_error, None, "a shape slip is never fatal");
    assert_eq!(out.config.theme, ThemeChoice::NoColor);
    assert_eq!(
        out.problems.messages(),
        vec![
            "jumps: expected an array of jump rules ([[jumps]]), found table; \
             ignoring every jump rule"
        ]
    );
    assert!(out.config.jumps.rules().is_empty());
}

#[test]
fn a_to_with_no_tab_half_names_a_service_list_rather_than_a_detail_tab() {
    // The grammar's superset. `"vpc-networks"` is a service id with
    // no `:` after it, which no detail key can ever be, so the two spellings
    // cannot collide and every file written before this ticket still parses.
    let (out, _file) = resolve(
        "version = 1\n[[jumps]]\nfrom = \"firewall-rules\"\nto = \"vpc-networks\"\nname = \"{network}\"\n",
    );

    assert!(
        out.problems.is_empty(),
        "unexpected: {:?}",
        out.problems.messages()
    );
    let rule = &out.config.jumps.rules()[0];
    assert_eq!(rule.to, JumpTarget::List(ResourceServiceKind::VpcNetworks));
    // Emitted back in the spelling the user wrote, as detail keys are.
    assert_eq!(rule.to.config_key(), "vpc-networks");
}

#[test]
fn a_to_with_a_tab_half_is_still_a_detail_target() {
    // The other half of the superset: the four built-in destinations keep
    // their meaning, so `to` is not silently re-read as a service list.
    let (out, _file) =
        resolve("version = 1\n[[jumps]]\nfrom = \"compute\"\nto = \"compute:network\"\n");

    assert!(
        out.problems.is_empty(),
        "unexpected: {:?}",
        out.problems.messages()
    );
    assert!(matches!(
        out.config.jumps.rules()[0].to,
        JumpTarget::Detail(_)
    ));
}

#[test]
fn a_location_template_on_a_list_target_that_keys_by_name_alone_is_rejected() {
    // A VPC network's row identity is its name and an empty location, so a
    // rendered location could only ever fail to match. Refused at load, where
    // the warning can name the key, rather than at `J`, where it would look
    // like the row had gone.
    let (out, _file) = resolve(
        "version = 1\n[[jumps]]\nfrom = \"vpc-subnets\"\nto = \"vpc-networks\"\nname = \"{network}\"\nlocation = \"{location}\"\n",
    );

    assert_eq!(out.load_error, None);
    assert_eq!(
        out.problems.messages(),
        vec![
            "jumps[0]: location template on a jump to vpc-networks, which keys its rows \
             by name alone; ignoring this rule"
        ]
    );
    assert!(out.config.jumps.rules().is_empty());
}

#[test]
fn a_bigquery_list_destination_keys_rows_by_name_alone_despite_having_a_location_field() {
    // The case the first version of this got wrong. `bigquery-datasets`
    // declares a `{location}` field a rule may *read*, and identifies its own
    // rows by the dataset id alone, so a `location` on a rule landing there
    // could never match. The two questions are different and this is the
    // service where deriving one from the other breaks.
    let (out, _file) = resolve(
        "version = 1\n[[jumps]]\nfrom = \"bigquery-datasets\"\nto = \"bigquery-datasets\"\nlocation = \"{location}\"\n",
    );

    assert_eq!(out.load_error, None);
    assert_eq!(
        out.problems.messages(),
        vec![
            "jumps[0]: location template on a jump to bigquery-datasets, which keys its rows \
             by name alone; ignoring this rule"
        ]
    );
}

#[test]
fn the_iam_policy_table_is_not_a_list_destination_at_all() {
    // A policy binding is identified by member *and* role — two bindings can
    // share a member — and a rule has no template that renders a role. Refused
    // at load rather than offered as a row that lands nowhere.
    let (out, _file) = resolve("version = 1\n[[jumps]]\nfrom = \"compute\"\nto = \"iam-policy\"\n");

    assert_eq!(out.load_error, None);
    assert_eq!(
        out.problems.messages(),
        vec![
            "jumps[0]: IAM policy identifies a row by more than a name and a location, \
             so to \"iam-policy\" is not a destination; ignoring this rule"
        ]
    );
    assert!(out.config.jumps.rules().is_empty());
}

#[test]
fn an_unknown_service_without_a_tab_half_is_still_an_unknown_destination() {
    // The superset must not turn every typo into a service list.
    let (out, _file) = resolve("version = 1\n[[jumps]]\nfrom = \"compute\"\nto = \"zzz\"\n");

    assert_eq!(out.load_error, None);
    assert_eq!(
        out.problems.messages(),
        vec!["jumps[0]: unknown destination tab \"zzz\"; ignoring this rule"]
    );
}

#[test]
fn a_select_names_a_computation_and_the_pair_it_is_defined_for() {
    // The narrowing is a rule property rather than a third
    // `JumpTarget`: the destination really is the firewall rules list, and what
    // is new is that the landing arrives narrowed.
    let (out, _file) = resolve(
        "version = 1\n[[jumps]]\nfrom = \"compute\"\nto = \"firewall-rules\"\nselect = \"targets-this-instance\"\n",
    );

    assert!(
        out.problems.is_empty(),
        "unexpected: {:?}",
        out.problems.messages()
    );
    assert_eq!(
        out.config.jumps.rules()[0].select,
        Some(JumpSelect::FirewallRulesTargetingInstance)
    );
}

#[test]
fn a_select_on_a_pair_it_is_not_defined_for_is_rejected() {
    // The computation needs an instance to compute from and a firewall rules
    // table to narrow. Pointed at anything else it would resolve, offer a row,
    // and then narrow nothing — an empty table that reads as "no rules apply"
    // rather than as a mistake in the file.
    let (out, _file) = resolve(
        "version = 1\n[[jumps]]\nfrom = \"gke\"\nto = \"firewall-rules\"\nselect = \"targets-this-instance\"\n",
    );

    assert_eq!(out.load_error, None);
    assert_eq!(
        out.problems.messages(),
        vec![
            "jumps[0]: select \"targets-this-instance\" is only defined from compute to \
             firewall-rules; ignoring this rule"
        ]
    );
    assert!(out.config.jumps.rules().is_empty());
}

#[test]
fn a_select_pointed_at_the_wrong_destination_is_rejected_too() {
    // Both halves are checked, not just the source.
    let (out, _file) = resolve(
        "version = 1\n[[jumps]]\nfrom = \"compute\"\nto = \"vpc-networks\"\nname = \"{network}\"\nselect = \"targets-this-instance\"\n",
    );

    assert_eq!(out.load_error, None);
    assert!(
        out.problems.messages()[0].contains("is only defined from compute to firewall-rules"),
        "unexpected: {:?}",
        out.problems.messages()
    );
}

#[test]
fn an_unknown_select_is_rejected_by_name() {
    let (out, _file) = resolve(
        "version = 1\n[[jumps]]\nfrom = \"compute\"\nto = \"firewall-rules\"\nselect = \"zzz\"\n",
    );

    assert_eq!(out.load_error, None);
    assert_eq!(
        out.problems.messages(),
        vec!["jumps[0]: unknown select \"zzz\"; ignoring this rule"]
    );
}

#[test]
fn an_empty_jumps_array_configures_no_jumps() {
    // Written out in full, this is how a user turns every jump off.
    let (out, _file) = resolve("version = 1\njumps = []\n");

    assert!(
        out.problems.is_empty(),
        "unexpected: {:?}",
        out.problems.messages()
    );
    assert!(out.config.jumps.rules().is_empty());
}
