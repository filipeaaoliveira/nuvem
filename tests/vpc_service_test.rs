//! VPC service layer: the argv nuvem issues for a networks listing, and what
//! it makes of the JSON that comes back (slice 1).
//!
//! # The two fields this parser must not read
//!
//! `gcloud compute networks list --format=json` returns a document that is
//! *not* the `compute/v1/networks` resource. gcloud synthesises two extra keys
//! into every row — `x_gcloud_subnet_mode` and `x_gcloud_bgp_routing_mode` —
//! which carry exactly the strings its own table paints. They are the shortest
//! path to the `MODE` and `ROUTING` columns and they are the wrong one:
//! nothing commits gcloud to keeping a presentation field, and the `x_`
//! prefix is gcloud saying so. `MODE` is derived from `autoCreateSubnetworks`
//! and `ROUTING` from `routingConfig.routingMode`, both real API fields, and
//! `derives_mode_and_routing_from_the_api_fields_not_gclouds_synthesised_ones`
//! below feeds the parser a row where the two disagree so that a later edit
//! reaching for the easy field fails rather than passing by coincidence.

use nuvem::services::gcloud::cli::{CommandError, CommandOutput, CommandRunner, GcloudCommand};
use nuvem::services::resources::vpc::{GcloudVpcService, parse_networks_json, resource_id};
use nuvem_domain::gcp::configuration::{GcloudConfigurationName, GcloudContext};
use nuvem_domain::gcp::resource::{VpcRoutingMode, VpcSubnetMode};
use std::cell::RefCell;

/// A live capture: `gcloud compute networks list --format=json` against a real
/// project on 2026-08-25, scrubbed only by replacing the source project id
/// with `nuvem-fixture-project`. The field set, the key spellings and the
/// 43-entry `subnetworks` array are the API's own, not a transcription of the
/// reference — which is what makes an assertion about a *missing* field here
/// mean something.
const NETWORKS_JSON: &str = include_str!("fixtures/vpc_networks.json");

fn ctx() -> GcloudContext {
    GcloudContext {
        configuration: GcloudConfigurationName::new("dev"),
        account: None,
        project: Some("dev-project".into()),
        region: None,
        zone: None,
    }
}

/// Records the argv it is handed and answers with one canned document.
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

/// The argv, in full: the configuration and project the context names, the
/// three subcommand words, JSON output and the shared row cap.
#[test]
fn a_networks_load_issues_one_compute_networks_list() {
    let runner = ScriptedRunner::ok(NETWORKS_JSON);
    let service = GcloudVpcService::new(&runner);

    service.load_networks(&ctx(), None).expect("networks load");

    assert_eq!(
        runner.calls(),
        vec![vec![
            "--configuration".to_string(),
            "dev".to_string(),
            "--project".to_string(),
            "dev-project".to_string(),
            "compute".to_string(),
            "networks".to_string(),
            "list".to_string(),
            "--format=json".to_string(),
            "--limit=500".to_string(),
        ]],
        "a networks listing is one flat project-wide call"
    );
}

/// The captured row, cell by cell.
///
/// The project's one network is the auto-mode `default`, so this pins the
/// `AUTO`/`REGIONAL` half against real bytes; the constructed row in the next
/// test pins the half no project this account can reach has.
#[test]
fn parses_the_captured_default_network() {
    let networks = parse_networks_json(NETWORKS_JSON).expect("the capture parses");

    assert_eq!(networks.len(), 1, "the capture holds one network");
    let network = &networks[0];
    assert_eq!(network.name, "default");
    assert_eq!(network.subnet_mode, VpcSubnetMode::Auto);
    assert_eq!(network.routing_mode, VpcRoutingMode::Regional);
    assert_eq!(
        network.subnet_count, 43,
        "SUBNETS is the length of the `subnetworks` array"
    );
    assert_eq!(
        network.description.as_deref(),
        Some("Default network for the project")
    );
}

/// The pin that stops a later edit reading gcloud's presentation fields.
///
/// CONSTRUCTED, and deliberately impossible: this row says `AUTO`/`GLOBAL` in
/// the two `x_gcloud_*` keys while its API fields say custom and regional. The
/// real API never contradicts itself this way — that is the point. A parser
/// reading `x_gcloud_subnet_mode` passes every other test in this file and
/// fails only here.
#[test]
fn derives_mode_and_routing_from_the_api_fields_not_gclouds_synthesised_ones() {
    let json = r#"[
      {
        "name": "contradiction",
        "autoCreateSubnetworks": false,
        "routingConfig": { "routingMode": "REGIONAL" },
        "subnetworks": [],
        "x_gcloud_subnet_mode": "AUTO",
        "x_gcloud_bgp_routing_mode": "GLOBAL"
      }
    ]"#;

    let networks = parse_networks_json(json).expect("the row parses");

    assert_eq!(
        networks[0].subnet_mode,
        VpcSubnetMode::Custom,
        "MODE comes from autoCreateSubnetworks, not from x_gcloud_subnet_mode"
    );
    assert_eq!(
        networks[0].routing_mode,
        VpcRoutingMode::Regional,
        "ROUTING comes from routingConfig.routingMode, not from x_gcloud_bgp_routing_mode"
    );
}

/// An absent `autoCreateSubnetworks` is a legacy network, not a custom one.
///
/// CONSTRUCTED from the published `compute/v1/networks` reference: a legacy
/// network carries `IPv4Range` and omits `autoCreateSubnetworks` entirely.
/// This account can reach no legacy network, so the arm has never been seen
/// from the real API. Defaulting the absent field to `false` would paint such
/// a network `CUSTOM` — a network an operator would then look for subnets in
/// and find none, because a legacy network cannot have any.
#[test]
fn a_network_with_no_auto_create_subnetworks_field_is_legacy() {
    let json = r#"[
      { "name": "legacy-net", "IPv4Range": "10.240.0.0/16" }
    ]"#;

    let networks = parse_networks_json(json).expect("the row parses");

    assert_eq!(networks[0].subnet_mode, VpcSubnetMode::Legacy);
    assert_eq!(
        networks[0].routing_mode,
        VpcRoutingMode::Unspecified,
        "no routingConfig is a fact the API did not report, not a routing mode"
    );
    assert_eq!(networks[0].subnet_count, 0);
}

/// An unknown routing mode is carried through rather than collapsed.
#[test]
fn an_unrecognised_routing_mode_is_carried_verbatim() {
    let json = r#"[
      { "name": "n", "autoCreateSubnetworks": true,
        "routingConfig": { "routingMode": "INTERPLANETARY" } }
    ]"#;

    let networks = parse_networks_json(json).expect("the row parses");

    assert_eq!(
        networks[0].routing_mode,
        VpcRoutingMode::Unrecognized("INTERPLANETARY".to_string())
    );
}

/// A document that is not the expected JSON fails the load; a row missing
/// every optional field does not.
#[test]
fn a_row_missing_every_optional_field_still_parses() {
    let networks = parse_networks_json(r#"[{ "name": "bare" }]"#).expect("a bare row parses");

    assert_eq!(networks[0].name, "bare");
    assert_eq!(networks[0].description, None);

    parse_networks_json("not json").expect_err("a non-JSON document fails the load");
}

/// The URL-tail helper the next two slices reuse.
///
/// Subnets and firewall rules receive `region` and `network` as full self-link
/// URLs; a table cell wants the last path segment. Built here, in the slice
/// that first needs a resource id out of a path, so the other two do not each
/// grow a copy.
#[test]
fn resource_id_takes_the_last_path_segment() {
    assert_eq!(
        resource_id("https://www.googleapis.com/compute/v1/projects/p/regions/us-central1"),
        "us-central1"
    );
    assert_eq!(
        resource_id("https://www.googleapis.com/compute/v1/projects/p/global/networks/default"),
        "default"
    );
    assert_eq!(
        resource_id("already-short"),
        "already-short",
        "a value with no slash is already an id and passes through unchanged"
    );
    assert_eq!(
        resource_id(""),
        "",
        "an empty value stays empty rather than becoming a panic or a dash"
    );
}
