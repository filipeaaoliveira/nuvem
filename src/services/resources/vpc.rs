//! VPC networking: the flat, project-wide listings behind the networks table.
//!
//! # One module for three resources
//!
//! Networks, subnets and firewall rules are three `ResourceServiceKind`s and
//! one service type. They share the argv prefix (`gcloud compute ...`), the
//! row cap, and — the reason that matters — the URL-tail helper below: two of
//! the three receive their most important columns as full self-link URLs, and
//! a second copy of the shortening is how two tables end up disagreeing about
//! what a network is called. The networks half landed first; the other two
//! join it here rather than in modules of their own.
//!
//! # Flat, not nested
//!
//! Subnets look like children of a network and are not listed that way:
//! `gcloud compute networks subnets list` is a flat project-wide call, as
//! `firewall-rules list` is. Only the *display* would have been nested, and
//! nesting it would have cost all three the ability to be a jump target.
//!
//! # What this module does not do
//!
//! No describe, no detail pane, no lifecycle action. These are read-only
//! browsing tables, and a `compute networks describe` would return the same
//! fields the list call already returns.

use serde::Deserialize;

use nuvem_domain::gcp::configuration::GcloudContext;
use nuvem_domain::gcp::resource::{
    FirewallAction, FirewallDirection, FirewallRule, FirewallTargets, VpcNetwork, VpcRoutingMode,
    VpcSubnet, VpcSubnetMode,
};

use crate::services::gcloud::cli::{CommandError, CommandRunner, GcloudCommand};
use crate::services::resources::list_filter::RESOURCE_LIST_LIMIT;

#[derive(Debug, thiserror::Error)]
pub enum VpcParseError {
    #[error("invalid VPC JSON: {0}")]
    Json(#[from] serde_json::Error),
}

#[derive(Debug, thiserror::Error)]
pub enum VpcLoadError {
    #[error("{0}")]
    Command(#[from] CommandError),
    #[error("{0}")]
    Parse(#[from] VpcParseError),
}

pub struct GcloudVpcService<R> {
    runner: R,
}

impl<R> GcloudVpcService<R> {
    pub fn new(runner: R) -> Self {
        Self { runner }
    }

    pub fn runner(&self) -> &R {
        &self.runner
    }
}

impl<R: CommandRunner> GcloudVpcService<R> {
    /// Lists a project's VPC networks.
    ///
    /// # Nothing is pushed down, and the filter is taken anyway
    ///
    /// The applied filter is ignored rather than translated into a
    /// `--filter=` expression. The never-drop-a-row invariant is a claim about
    /// *display* values: each searchable field must be a verbatim substring of
    /// a raw field the server narrowed on. Three of this table's four columns
    /// fail that outright — `MODE` renders `AUTO` against a boolean,
    /// `ROUTING` against a nested string, and `SUBNETS` against the *length*
    /// of an array. Pushing any clause would drop rows an operator can see, so
    /// narrowing stays wholly client-side, which is the safe direction.
    ///
    /// The parameter stays in the signature so this call has the same shape as
    /// every other list load, which is what lets the registry, the background
    /// refresh and the reducer treat all of them alike.
    pub fn load_networks(
        &self,
        context: &GcloudContext,
        _filter: Option<&str>,
    ) -> Result<Vec<VpcNetwork>, VpcLoadError> {
        let mut args = base_args(context);
        args.extend([
            "compute".to_string(),
            "networks".to_string(),
            "list".to_string(),
            "--format=json".to_string(),
            format!("--limit={RESOURCE_LIST_LIMIT}"),
        ]);
        let output = self.runner.run(GcloudCommand::new(args))?;
        Ok(parse_networks_json(&output.stdout)?)
    }

    /// Lists every subnet in the project, across every region.
    ///
    /// One flat call despite subnets being regional: `networks subnets list`
    /// is project-wide, so there is no per-region fan-out and no `--regions`
    /// flag. That is the whole point of the table — an operator sees every
    /// region's subnets at once rather than paging through them.
    ///
    /// Nothing is pushed down, for the reason
    /// [`load_networks`](Self::load_networks) records: `REGION` and `NETWORK`
    /// are the *tails* of self-link URLs and `PRIVATE GOOGLE ACCESS` renders a
    /// bool as a word, so none of the three is a verbatim substring of a raw
    /// field and a pushed clause would drop rows an operator can read.
    pub fn load_subnets(
        &self,
        context: &GcloudContext,
        _filter: Option<&str>,
    ) -> Result<Vec<VpcSubnet>, VpcLoadError> {
        let mut args = base_args(context);
        args.extend([
            "compute".to_string(),
            "networks".to_string(),
            "subnets".to_string(),
            "list".to_string(),
            "--format=json".to_string(),
            format!("--limit={RESOURCE_LIST_LIMIT}"),
        ]);
        let output = self.runner.run(GcloudCommand::new(args))?;
        Ok(parse_subnets_json(&output.stdout)?)
    }

    /// Lists every firewall rule in the project.
    ///
    /// Flat and project-wide like the other two: a firewall rule is a global
    /// resource attached to a network, so there is nothing to iterate.
    ///
    /// Nothing is pushed down, for the reason the sibling loads record:
    /// `ACTION` is derived from *which key is present*, `DIRECTION` and
    /// `TARGETS` are rendered values, and `SOURCE/DEST` is whichever of two
    /// lists the direction selects. None is a verbatim substring of a raw
    /// field, so a pushed clause would drop rows an operator can read — and on
    /// this table a dropped row is a rule an operator concludes does not
    /// exist.
    pub fn load_firewall_rules(
        &self,
        context: &GcloudContext,
        _filter: Option<&str>,
    ) -> Result<Vec<FirewallRule>, VpcLoadError> {
        let mut args = base_args(context);
        args.extend([
            "compute".to_string(),
            "firewall-rules".to_string(),
            "list".to_string(),
            "--format=json".to_string(),
            format!("--limit={RESOURCE_LIST_LIMIT}"),
        ]);
        let output = self.runner.run(GcloudCommand::new(args))?;
        Ok(parse_firewall_rules_json(&output.stdout)?)
    }
}

/// The wire shape of a subnet.
///
/// `region` and `network` are `String` and not some URL type on purpose: they
/// arrive as full self-links and are reduced by [`resource_id`] on the way
/// into the domain struct, so nothing downstream ever holds the URL form and
/// no two cells can disagree about what a network is called.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawSubnet {
    #[serde(default)]
    name: String,
    #[serde(default)]
    region: String,
    #[serde(default)]
    network: String,
    #[serde(default)]
    ip_cidr_range: String,
    #[serde(default)]
    gateway_address: Option<String>,
    /// Defaulted, unlike a network's `autoCreateSubnetworks`: the API
    /// documents an absent flag as off, and this is a setting rather than a
    /// discriminator between two shapes of resource.
    #[serde(default)]
    private_ip_google_access: bool,
}

pub fn parse_subnets_json(stdout: &str) -> Result<Vec<VpcSubnet>, VpcParseError> {
    let raw: Vec<RawSubnet> = serde_json::from_str(stdout)?;
    Ok(raw.into_iter().map(subnet_from_raw).collect())
}

fn subnet_from_raw(raw: RawSubnet) -> VpcSubnet {
    VpcSubnet {
        name: raw.name,
        // Both self-links, reduced here and only here.
        region: resource_id(&raw.region),
        network: resource_id(&raw.network),
        ip_cidr_range: raw.ip_cidr_range,
        // An empty gateway is `None`, not `Some("")`: the two would paint the
        // same dash, and folding them here means one shape reaches the filter.
        gateway_address: raw.gateway_address.filter(|address| !address.is_empty()),
        private_google_access: raw.private_ip_google_access,
    }
}

/// The wire shape of a firewall rule.
///
/// `allowed` and `denied` are read for **presence only** — the protocol/port
/// lists inside them are not modelled, because no column paints them and a
/// field this struct carries that no cell reads is a cell waiting to disagree
/// with the filter. `serde_json::Value` rather than a typed list for the same
/// reason: the parser must not fail on a shape it does not render.
///
/// The two are separate here because that is how the JSON arrives; they are
/// collapsed to one [`FirewallAction`] on the way into the domain struct, so
/// nothing downstream can hold the four-state pair.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawFirewallRule {
    #[serde(default)]
    name: String,
    #[serde(default)]
    network: String,
    #[serde(default)]
    direction: Option<String>,
    #[serde(default)]
    priority: Option<u32>,
    #[serde(default)]
    disabled: bool,
    #[serde(default)]
    allowed: Option<serde_json::Value>,
    #[serde(default)]
    denied: Option<serde_json::Value>,
    #[serde(default)]
    source_ranges: Vec<String>,
    #[serde(default)]
    destination_ranges: Vec<String>,
    #[serde(default)]
    target_tags: Vec<String>,
    #[serde(default)]
    target_service_accounts: Vec<String>,
}

pub fn parse_firewall_rules_json(stdout: &str) -> Result<Vec<FirewallRule>, VpcParseError> {
    let raw: Vec<RawFirewallRule> = serde_json::from_str(stdout)?;
    Ok(raw.into_iter().map(firewall_rule_from_raw).collect())
}

fn firewall_rule_from_raw(raw: RawFirewallRule) -> FirewallRule {
    let direction = FirewallDirection::from_api(raw.direction.as_deref());
    // The rule carries the list matching its direction and not the other, so
    // the cell reads one of them rather than concatenating both. An
    // unrecognised or absent direction falls back to whichever list is
    // populated: the ranges are still true even when the direction word is
    // not one this build knows.
    let source_or_dest = match direction {
        FirewallDirection::Egress => raw.destination_ranges,
        FirewallDirection::Ingress => raw.source_ranges,
        _ if !raw.source_ranges.is_empty() => raw.source_ranges,
        _ => raw.destination_ranges,
    };

    FirewallRule {
        name: raw.name,
        network: resource_id(&raw.network),
        direction,
        priority: raw.priority,
        // Presence, not content: which key is there is the whole answer.
        action: FirewallAction::from_api(raw.allowed.is_some(), raw.denied.is_some()),
        // Tags and service accounts are alternative scopings, and which of
        // the two a rule uses is now kept rather than concatenated away:
        // matching an instance has to compare a tag against tags
        // and an email against service accounts, and a merged list cannot say
        // which is which. `from_api` also names the no-selector case, which is
        // "every instance in this network" and not "no instances".
        targets: FirewallTargets::from_api(raw.target_tags, raw.target_service_accounts),
        source_or_dest,
        disabled: raw.disabled,
    }
}

/// Shared leading args: `--configuration <cfg> [--project <p>]`.
fn base_args(context: &GcloudContext) -> Vec<String> {
    let mut args = vec![
        "--configuration".to_string(),
        context.configuration.as_str().to_string(),
    ];
    if let Some(project) = context.project.as_deref() {
        args.extend(["--project".to_string(), project.to_string()]);
    }
    args
}

/// The trailing segment of a self-link URL, which is the resource's id.
///
/// The compute API hands out references as full URLs
/// (`.../projects/p/regions/us-central1`), and every table cell that paints
/// one wants the last segment. Built in the networks slice, before either
/// caller that needs it exists, because the two that do — a subnet's `region`
/// and `network`, a firewall rule's `network` — would otherwise each grow a
/// copy of the same three lines, and two copies is how two tables come to
/// disagree about what one network is called.
///
/// A value with no slash is already an id and passes through unchanged, which
/// is also what leaves the empty string empty rather than turning it into a
/// panic or a dash. Deciding what an absent value *paints* belongs to the
/// column, not here.
pub fn resource_id(self_link: &str) -> String {
    self_link
        .rsplit('/')
        .next()
        .unwrap_or(self_link)
        .to_string()
}

/// The wire shape of a network.
///
/// Every field is optional, so a row missing any of them still parses; only a
/// document that is not the expected JSON fails the load, which is the bargain
/// every sibling service strikes.
///
/// `x_gcloud_subnet_mode` and `x_gcloud_bgp_routing_mode` are deliberately
/// absent from this struct. gcloud synthesises both into its JSON and they
/// carry exactly the strings the `MODE` and `ROUTING` columns paint, which is
/// what makes reading them tempting: they are gcloud's presentation layer, not
/// API fields, and the `x_` prefix is gcloud saying so. Both columns are
/// derived from the real fields below instead, and
/// `vpc_service_test::derives_mode_and_routing_from_the_api_fields_not_gclouds_synthesised_ones`
/// feeds this parser a row where the two disagree so the shortcut cannot be
/// taken back by a later edit without a test failing.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawNetwork {
    #[serde(default)]
    name: String,
    /// `Option`, not a defaulted `bool`: a legacy network omits this field
    /// entirely, and `false` would paint it `CUSTOM`. See
    /// [`VpcSubnetMode::from_api`].
    #[serde(default)]
    auto_create_subnetworks: Option<bool>,
    #[serde(default)]
    routing_config: Option<RawRoutingConfig>,
    /// Self-link URLs, one per subnet. Only the length reaches the table.
    #[serde(default)]
    subnetworks: Vec<String>,
    #[serde(default)]
    description: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawRoutingConfig {
    #[serde(default)]
    routing_mode: Option<String>,
}

pub fn parse_networks_json(stdout: &str) -> Result<Vec<VpcNetwork>, VpcParseError> {
    let raw: Vec<RawNetwork> = serde_json::from_str(stdout)?;
    Ok(raw.into_iter().map(network_from_raw).collect())
}

fn network_from_raw(raw: RawNetwork) -> VpcNetwork {
    VpcNetwork {
        name: raw.name,
        subnet_mode: VpcSubnetMode::from_api(raw.auto_create_subnetworks),
        routing_mode: VpcRoutingMode::from_api(
            raw.routing_config
                .as_ref()
                .and_then(|config| config.routing_mode.as_deref()),
        ),
        // The count, not the array: 43 self-link URLs is not a cell, and the
        // subnets are a table of their own.
        subnet_count: raw.subnetworks.len(),
        // An empty description is `None`, not `Some("")`: the two would paint
        // the same dash, and folding them here means one shape reaches the
        // filter's searchable set rather than two.
        description: raw.description.filter(|text| !text.is_empty()),
    }
}
