//! The subnets half of the VPC service layer (slice 2).
//!
//! # The parsing hazard this slice exists around
//!
//! A subnet's `region` and `network` both arrive as **full self-link URLs**
//! (`https://.../projects/p/regions/us-central1`), not as ids. Every cell that
//! paints one takes the last path segment, through the `resource_id` helper
//! slice 1 built for exactly this. A table that painted the raw URLs would be
//! unreadable at any terminal width, and one that took the *first* segment or
//! split on the wrong separator would paint `https:` on every row.
//!
//! # Names are not unique here, and that is the other divergence
//!
//! The capture holds subnets **all named `default`**, one per region — which
//! is the normal shape of an auto-mode network, not an edge case. A subnet is
//! a regional resource, so its identity is (name, region); the networks table
//! one slice earlier could use the name alone because a network is global.
//! Anything keyed on the name alone here collapses every row into one.
//!
//! The fixture is the capture plus one constructed row, built to the same
//! shape and carrying `privateIpGoogleAccess` on — see [`SUBNETS_JSON`] for
//! why. Where a count below is four rather than three, that row is the
//! difference; the collision itself is the capture's and needs no help.

use nuvem::services::gcloud::cli::{CommandError, CommandOutput, CommandRunner, GcloudCommand};
use nuvem::services::resources::vpc::{GcloudVpcService, parse_subnets_json};
use nuvem_domain::gcp::configuration::{GcloudConfigurationName, GcloudContext};
use std::cell::RefCell;

/// A live capture: `gcloud compute networks subnets list --format=json`
/// against a real project on 2026-08-25, scrubbed only by replacing the source
/// project id with `nuvem-fixture-project`. Field set and key spellings are the
/// API's own.
///
/// Trimmed to three of the 43 rows the project returned — the full set is 43
/// copies of one shape, and three regions is what it takes to show that the
/// name does not identify a row. The trim removes rows; it changes no field of
/// the rows it keeps.
///
/// The fourth row, `us-west1`, is CONSTRUCTED rather than captured: the
/// captured project runs `privateIpGoogleAccess` off on all 43 of its subnets,
/// and `false` is also what an absent flag parses to, so against the capture
/// alone every assertion on that column holds just as well for a mapping that
/// dropped the field. The constructed row carries the flag on, which is the
/// only way a test here can tell the two apart.
const SUBNETS_JSON: &str = include_str!("fixtures/vpc_subnets.json");

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

/// One flat call, listing every region at once.
///
/// `networks subnets list` is project-wide despite subnets being regional, so
/// there is no per-region fan-out here and no `--regions` flag: the whole
/// point of the flat listing is that an operator sees every region's subnets
/// in one table.
#[test]
fn a_subnets_load_issues_one_flat_project_wide_call() {
    let runner = ScriptedRunner::ok(SUBNETS_JSON);
    let service = GcloudVpcService::new(&runner);

    service.load_subnets(&ctx(), None).expect("subnets load");

    assert_eq!(
        runner.calls(),
        vec![vec![
            "--configuration".to_string(),
            "dev".to_string(),
            "--project".to_string(),
            "dev-project".to_string(),
            "compute".to_string(),
            "networks".to_string(),
            "subnets".to_string(),
            "list".to_string(),
            "--format=json".to_string(),
            "--limit=500".to_string(),
        ]],
        "one project-wide call, not one per region"
    );
}

/// The fixture's rows, cell by cell.
#[test]
fn parses_the_captured_subnets() {
    let subnets = parse_subnets_json(SUBNETS_JSON).expect("the capture parses");

    assert_eq!(subnets.len(), 4);
    let first = &subnets[0];
    assert_eq!(first.name, "default");
    assert_eq!(first.region, "us-central1");
    assert_eq!(first.network, "default");
    assert_eq!(first.ip_cidr_range, "10.128.0.0/20");
    assert_eq!(first.gateway_address.as_deref(), Some("10.128.0.1"));
    assert!(!first.private_google_access);

    // The constructed row, and the reason it is in the fixture: the flag is
    // read off the document rather than defaulted, which no row with it off
    // can show.
    let last = &subnets[3];
    assert_eq!(last.region, "us-west1");
    assert_eq!(last.ip_cidr_range, "10.138.0.0/20");
    assert_eq!(last.gateway_address.as_deref(), Some("10.138.0.1"));
    assert!(last.private_google_access);

    assert_eq!(
        subnets
            .iter()
            .map(|subnet| subnet.private_google_access)
            .collect::<Vec<_>>(),
        vec![false, false, false, true],
        "the column has to carry both values, or it is pinned only at the one \
         an absent flag would produce anyway"
    );
}

/// The self-link hazard, on both fields, against the captured URLs.
///
/// A parser that carried the raw values through would put a 90-character URL
/// in a cell headed `REGION`; one that split on the wrong separator, or took
/// the head rather than the tail, would paint `https:` on every row of both
/// columns.
#[test]
fn the_region_and_network_urls_are_reduced_to_their_last_path_segment() {
    let subnets = parse_subnets_json(SUBNETS_JSON).expect("the capture parses");

    let regions: Vec<&str> = subnets.iter().map(|s| s.region.as_str()).collect();
    assert_eq!(
        regions,
        vec!["us-central1", "europe-west1", "asia-east1", "us-west1"]
    );

    for subnet in &subnets {
        assert!(
            !subnet.region.contains('/') && !subnet.region.starts_with("http"),
            "the REGION cell must be an id, not a self-link: {:?}",
            subnet.region
        );
        assert!(
            !subnet.network.contains('/') && !subnet.network.starts_with("http"),
            "the NETWORK cell must be an id, not a self-link: {:?}",
            subnet.network
        );
        assert_eq!(
            subnet.network, "default",
            "every row in the fixture belongs to the default network"
        );
    }
}

/// The identity fact this slice turns on: every row shares one name.
///
/// The collision is the *capture's* and not a constructed pair, which is the
/// point — this is the ordinary shape of an auto-mode network, and the three
/// captured rows below carry it on their own. The constructed fourth row is
/// built to the same shape, so it neither creates the collision nor dilutes
/// it; it does raise the region count from three to four, which is why the
/// captured rows are counted separately here rather than left implied by a
/// number that no longer distinguishes them.
#[test]
fn the_captured_subnets_share_one_name_across_their_regions() {
    let subnets = parse_subnets_json(SUBNETS_JSON).expect("the capture parses");

    let names: std::collections::BTreeSet<&str> = subnets.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(
        names.len(),
        1,
        "the fixture's rows all share one name, which is why the name alone \
         cannot identify a subnet"
    );

    // The capture is rows 0..3; row 3 is the constructed one (see
    // `SUBNETS_JSON`). Sliced rather than counted whole so the claim in the
    // name — that the *captured* rows collide — is the claim asserted.
    let captured_regions: std::collections::BTreeSet<&str> =
        subnets[..3].iter().map(|s| s.region.as_str()).collect();
    assert_eq!(
        captured_regions.len(),
        3,
        "the three captured rows are told apart by region alone"
    );

    let regions: std::collections::BTreeSet<&str> =
        subnets.iter().map(|s| s.region.as_str()).collect();
    assert_eq!(
        regions.len(),
        4,
        "and so is the constructed row, which shares the name too"
    );
}

/// `privateIpGoogleAccess` is a bool the API may omit, and absent is `false`.
///
/// Unlike `autoCreateSubnetworks` one slice earlier, defaulting is correct
/// here: the field is not a discriminator between two shapes of resource, it
/// is a flag whose absence the API documents as off. Reaching the screen as
/// `Off` rather than as a dash is what lets an operator scan the column.
#[test]
fn an_absent_private_google_access_flag_is_off() {
    let json = r#"[
      { "name": "bare", "ipCidrRange": "10.0.0.0/24",
        "region": "https://www.googleapis.com/compute/v1/projects/p/regions/us-west1",
        "network": "https://www.googleapis.com/compute/v1/projects/p/global/networks/vpc" }
    ]"#;

    let subnets = parse_subnets_json(json).expect("the row parses");

    assert!(!subnets[0].private_google_access);
    assert_eq!(
        subnets[0].gateway_address, None,
        "an absent gateway is absent, not an empty string the cell would paint"
    );
}

/// A row missing every optional field still parses; a document that is not the
/// expected JSON does not.
#[test]
fn a_row_missing_every_optional_field_still_parses() {
    let subnets = parse_subnets_json(r#"[{ "name": "bare" }]"#).expect("a bare row parses");

    assert_eq!(subnets[0].name, "bare");
    assert_eq!(subnets[0].region, "");
    assert_eq!(subnets[0].network, "");
    assert_eq!(subnets[0].ip_cidr_range, "");

    parse_subnets_json("not json").expect_err("a non-JSON document fails the load");
}
