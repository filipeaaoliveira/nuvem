//! `map_network`, `map_subnet` and `map_firewall_rule` — the three
//! translations between the native Compute v1 models and the VPC rows, all
//! three reported untested by the mutation sweep.
//!
//! Every mapped field is planted with its own non-default value so a body
//! replaced wholesale cannot pass, and every decision the mappers make — the
//! `autoCreateSubnetworks` tri-state, the routing and direction enum ladders
//! including their unknown arms, the `is_empty` filters behind `description`
//! and `gatewayAddress`, the `unwrap_or(false)` flags, the negative-priority
//! `try_from`, the allowed/denied and tags/service-accounts presence pairs,
//! and the guarded fallback that picks a range list — is asserted on both
//! sides with outcomes that differ.

use google_cloud_compute_v1::model::Firewall;
use google_cloud_compute_v1::model::Network;
use google_cloud_compute_v1::model::NetworkRoutingConfig;
use google_cloud_compute_v1::model::Subnetwork;
use google_cloud_compute_v1::model::SubnetworkSecondaryRange;
use google_cloud_compute_v1::model::firewall::{Allowed, Denied, Direction};
use google_cloud_compute_v1::model::network_routing_config::RoutingMode;
use google_cloud_compute_v1::model::subnetwork::{Purpose, Role};

use nuvem::services::sdk::vpc::{map_firewall_rule, map_network, map_subnet};
use nuvem_domain::gcp::resource::{
    FirewallAction, FirewallDirection, FirewallRule, FirewallTargets, VpcNetwork, VpcRoutingMode,
    VpcSubnet, VpcSubnetMode,
};

/// A network self-link. The tail segment is a different word from every other
/// value in these tests, so a mapper that kept the whole URL — or read some
/// other field — cannot look right.
const NETWORK_LINK: &str =
    "https://www.googleapis.com/compute/v1/projects/acme-prod/global/networks/shared-vpc";
const REGION_LINK: &str =
    "https://www.googleapis.com/compute/v1/projects/acme-prod/regions/europe-west4";

fn routing_config(mode: RoutingMode) -> NetworkRoutingConfig {
    let mut config = NetworkRoutingConfig::new();
    config.routing_mode = Some(mode);
    config
}

/// A network with every mapped field set to its own non-default value.
fn populated_network() -> Network {
    let mut network = Network::new();
    network.name = Some("shared-vpc".to_string());
    network.auto_create_subnetworks = Some(true);
    network.routing_config = Some(routing_config(RoutingMode::Global));
    network.subnetworks = vec![
        format!("{NETWORK_LINK}/subnetworks/web-eu"),
        format!("{NETWORK_LINK}/subnetworks/web-us"),
        format!("{NETWORK_LINK}/subnetworks/db-eu"),
    ];
    network.description = Some("the production perimeter".to_string());
    network
}

/// A subnet with every mapped field set to its own non-default value.
fn populated_subnet() -> Subnetwork {
    let mut subnet = Subnetwork::new();
    subnet.name = Some("web-eu".to_string());
    subnet.region = Some(REGION_LINK.to_string());
    subnet.network = Some(NETWORK_LINK.to_string());
    subnet.ip_cidr_range = Some("10.132.0.0/20".to_string());
    subnet.gateway_address = Some("10.132.0.1".to_string());
    subnet.private_ip_google_access = Some(true);
    subnet
}

fn allowed(protocol: &str, ports: &[&str]) -> Allowed {
    let mut entry = Allowed::new();
    entry.ip_protocol = Some(protocol.to_string());
    entry.ports = ports.iter().map(|port| (*port).to_string()).collect();
    entry
}

fn denied(protocol: &str) -> Denied {
    let mut entry = Denied::new();
    entry.ip_protocol = Some(protocol.to_string());
    entry
}

/// A firewall rule with a name and a parent network, and nothing else — each
/// test plants the fields whose branch it is pinning.
fn rule(name: &str) -> Firewall {
    let mut rule = Firewall::new();
    rule.name = Some(name.to_string());
    rule.network = Some(NETWORK_LINK.to_string());
    rule
}

fn ranges(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_string()).collect()
}

// ---------------------------------------------------------------- map_network

/// Whole-struct equality against five distinct values, so a body replaced by a
/// default row, or any field crossed with another, fails.
#[test]
fn a_network_maps_every_field_from_its_own_source() {
    let mut network = populated_network();
    // Fields the row does not carry, populated too: a mapper that read one of
    // them into a row field would have to differ from this expectation.
    network.id = Some(7_744_113_355_990_011);
    network.mtu = Some(8896);
    network.gateway_i_pv_4 = Some("10.132.0.1".to_string());
    network.self_link = Some(NETWORK_LINK.to_string());
    network.creation_timestamp = Some("2024-03-01T09:15:00.000-08:00".to_string());

    assert_eq!(
        map_network(&network),
        VpcNetwork {
            name: "shared-vpc".to_string(),
            subnet_mode: VpcSubnetMode::Auto,
            routing_mode: VpcRoutingMode::Global,
            subnet_count: 3,
            description: Some("the production perimeter".to_string()),
        }
    );
}

/// The tri-state the `MODE` column rests on. An absent flag is `LEGACY` and
/// not the `CUSTOM` a boolean default would paint, so all three arms are
/// pinned against outcomes that differ from one another.
#[test]
fn auto_create_subnetworks_separates_auto_custom_and_legacy() {
    let cases = [
        (Some(true), VpcSubnetMode::Auto),
        (Some(false), VpcSubnetMode::Custom),
        (None, VpcSubnetMode::Legacy),
    ];
    for (flag, expected) in cases {
        let mut network = populated_network();
        network.auto_create_subnetworks = flag;
        assert_eq!(
            map_network(&network).subnet_mode,
            expected,
            "autoCreateSubnetworks {flag:?}"
        );
    }
}

/// Every arm of the routing ladder, the unrecognised one included, plus the
/// two shapes that reach it as `None`: no `routingConfig` at all, and a
/// `routingConfig` carrying no mode. Both must paint blank rather than the
/// API's `REGIONAL` creation default.
#[test]
fn routing_mode_covers_every_arm_including_an_unrecognised_name() {
    let mut absent_config = populated_network();
    absent_config.routing_config = None;
    assert_eq!(
        map_network(&absent_config).routing_mode,
        VpcRoutingMode::Unspecified
    );

    let mut empty_config = populated_network();
    empty_config.routing_config = Some(NetworkRoutingConfig::new());
    assert_eq!(
        map_network(&empty_config).routing_mode,
        VpcRoutingMode::Unspecified
    );

    let cases = [
        (RoutingMode::Regional, VpcRoutingMode::Regional),
        (RoutingMode::Global, VpcRoutingMode::Global),
        (
            RoutingMode::from("MULTI_REGIONAL"),
            VpcRoutingMode::Unrecognized("MULTI_REGIONAL".to_string()),
        ),
        // An enum that arrived as an unknown *integer* has no name at all, and
        // that is an unspecified mode rather than a word invented for it.
        (RoutingMode::from(42), VpcRoutingMode::Unspecified),
        // An unknown name that is the empty string is the same blank, and not
        // an `Unrecognized("")` that would paint a cell out of nothing.
        (RoutingMode::from(""), VpcRoutingMode::Unspecified),
    ];
    for (mode, expected) in cases {
        let mut network = populated_network();
        network.routing_config = Some(routing_config(mode.clone()));
        assert_eq!(
            map_network(&network).routing_mode,
            expected,
            "routingMode {mode:?}"
        );
    }
}

/// The count, not the array — and three distinct counts, so a body that
/// returned a constant length cannot pass.
#[test]
fn subnet_count_is_the_length_of_the_subnetworks_array() {
    let cases: [(Vec<String>, usize); 3] = [
        (Vec::new(), 0),
        (vec![format!("{NETWORK_LINK}/subnetworks/web-eu")], 1),
        (
            vec![
                format!("{NETWORK_LINK}/subnetworks/web-eu"),
                format!("{NETWORK_LINK}/subnetworks/web-us"),
            ],
            2,
        ),
    ];
    for (subnetworks, expected) in cases {
        let mut network = populated_network();
        network.subnetworks = subnetworks;
        assert_eq!(map_network(&network).subnet_count, expected);
    }
}

/// `non_empty` on the description, both ways round: an empty string is the
/// absence the preview pane renders nothing for, and text survives verbatim.
#[test]
fn an_empty_description_is_dropped_and_text_survives() {
    let cases = [
        (None, None),
        (Some(String::new()), None),
        (
            Some("the production perimeter".to_string()),
            Some("the production perimeter".to_string()),
        ),
    ];
    for (description, expected) in cases {
        let mut network = populated_network();
        network.description = description.clone();
        assert_eq!(
            map_network(&network).description,
            expected,
            "description {description:?}"
        );
    }
}

#[test]
fn a_network_without_a_name_maps_to_the_empty_string() {
    let mut network = populated_network();
    network.name = None;
    assert_eq!(map_network(&network).name, "");
}

// ----------------------------------------------------------------- map_subnet

/// Whole-struct equality against six distinct values, two of which arrive as
/// self-link URLs and must come back as their last path segment.
#[test]
fn a_subnet_maps_every_field_from_its_own_source() {
    let mut subnet = populated_subnet();
    // Fields the row does not carry.
    subnet.id = Some(4_412_907_733_119_004);
    subnet.description = Some("front-end tier".to_string());
    subnet.enable_flow_logs = Some(true);
    subnet.self_link = Some(format!("{REGION_LINK}/subnetworks/web-eu"));

    assert_eq!(
        map_subnet(&subnet),
        VpcSubnet {
            name: "web-eu".to_string(),
            region: "europe-west4".to_string(),
            network: "shared-vpc".to_string(),
            ip_cidr_range: "10.132.0.0/20".to_string(),
            gateway_address: Some("10.132.0.1".to_string()),
            private_google_access: true,
        }
    );
}

/// The URL tail, whichever shape the field arrives in. A value that is already
/// an id passes through unchanged, which is what stops the reduction from
/// being a substring hack.
#[test]
fn region_and_network_are_reduced_to_their_last_path_segment() {
    let mut from_urls = populated_subnet();
    from_urls.region = Some(REGION_LINK.to_string());
    from_urls.network = Some(NETWORK_LINK.to_string());
    let mapped = map_subnet(&from_urls);
    assert_eq!(mapped.region, "europe-west4");
    assert_eq!(mapped.network, "shared-vpc");

    let mut bare = populated_subnet();
    bare.region = Some("us-central1".to_string());
    bare.network = Some("legacy-net".to_string());
    let mapped = map_subnet(&bare);
    assert_eq!(mapped.region, "us-central1");
    assert_eq!(mapped.network, "legacy-net");
}

#[test]
fn a_subnet_missing_its_string_fields_maps_them_to_empty_strings() {
    let mut subnet = populated_subnet();
    subnet.name = None;
    subnet.region = None;
    subnet.network = None;
    subnet.ip_cidr_range = None;
    let mapped = map_subnet(&subnet);
    assert_eq!(mapped.name, "");
    assert_eq!(mapped.region, "");
    assert_eq!(mapped.network, "");
    assert_eq!(mapped.ip_cidr_range, "");
}

/// `non_empty` on the gateway, both ways round. An empty string reported by
/// the API is the same absence as no key at all.
#[test]
fn an_empty_gateway_address_is_dropped_and_an_address_survives() {
    let cases = [
        (None, None),
        (Some(String::new()), None),
        (
            Some("10.132.0.1".to_string()),
            Some("10.132.0.1".to_string()),
        ),
    ];
    for (address, expected) in cases {
        let mut subnet = populated_subnet();
        subnet.gateway_address = address.clone();
        assert_eq!(
            map_subnet(&subnet).gateway_address,
            expected,
            "gatewayAddress {address:?}"
        );
    }
}

/// A plain `bool`: an unreported flag is off, and the two reported values are
/// carried through rather than collapsed.
#[test]
fn private_google_access_is_off_unless_the_api_reported_it_on() {
    let cases = [(Some(true), true), (Some(false), false), (None, false)];
    for (flag, expected) in cases {
        let mut subnet = populated_subnet();
        subnet.private_ip_google_access = flag;
        assert_eq!(
            map_subnet(&subnet).private_google_access,
            expected,
            "privateIpGoogleAccess {flag:?}"
        );
    }
}

/// The row is six fields wide on purpose. Purpose, role and secondary ranges
/// are real subnet fields the table does not paint, and planting them must not
/// move a single cell — the pin that says this mapper stayed narrow.
#[test]
fn purpose_role_and_secondary_ranges_do_not_reach_the_row() {
    let plain = map_subnet(&populated_subnet());

    let mut secondary = SubnetworkSecondaryRange::new();
    secondary.range_name = Some("pods".to_string());
    secondary.ip_cidr_range = Some("10.4.0.0/14".to_string());
    let mut other_secondary = SubnetworkSecondaryRange::new();
    other_secondary.range_name = Some("services".to_string());
    other_secondary.ip_cidr_range = Some("10.8.0.0/20".to_string());

    let mut enriched = populated_subnet();
    enriched.purpose = Some(Purpose::PrivateRfc1918);
    enriched.role = Some(Role::Backup);
    enriched.secondary_ip_ranges = vec![secondary, other_secondary];

    assert_eq!(map_subnet(&enriched), plain);
}

// ---------------------------------------------------------- map_firewall_rule

/// Whole-struct equality for an INGRESS rule. `destinationRanges` is planted
/// with a range the expectation does not contain, so a mapper that read the
/// list not matching the direction fails here rather than on a fixture where
/// the two happen to agree.
#[test]
fn an_ingress_rule_maps_every_field_and_takes_its_source_ranges() {
    let mut ingress = rule("allow-https");
    ingress.direction = Some(Direction::Ingress);
    ingress.priority = Some(1200);
    ingress.allowed = vec![allowed("tcp", &["443", "8443"])];
    ingress.target_tags = ranges(&["web", "edge"]);
    ingress.source_ranges = ranges(&["10.0.0.0/8", "192.168.0.0/16"]);
    ingress.destination_ranges = ranges(&["203.0.113.0/24"]);
    ingress.disabled = Some(true);
    // Fields the row does not carry.
    ingress.description = Some("public HTTPS".to_string());
    ingress.source_tags = ranges(&["bastion"]);
    ingress.id = Some(9_003_112_447_556_001);

    assert_eq!(
        map_firewall_rule(&ingress),
        FirewallRule {
            name: "allow-https".to_string(),
            network: "shared-vpc".to_string(),
            direction: FirewallDirection::Ingress,
            priority: Some(1200),
            action: FirewallAction::Allow,
            targets: FirewallTargets::Tags(vec!["web".to_string(), "edge".to_string()]),
            source_or_dest: vec!["10.0.0.0/8".to_string(), "192.168.0.0/16".to_string()],
            disabled: true,
        }
    );
}

/// The mirror image: an EGRESS rule takes `destinationRanges` while a
/// populated `sourceRanges` sits next to it, and the two lists differ.
#[test]
fn an_egress_rule_maps_every_field_and_takes_its_destination_ranges() {
    let mut egress = rule("deny-egress-rfc1918");
    egress.direction = Some(Direction::Egress);
    egress.priority = Some(65000);
    egress.denied = vec![denied("all")];
    egress.target_service_accounts = ranges(&["batch@acme-prod.iam.gserviceaccount.com"]);
    egress.source_ranges = ranges(&["10.0.0.0/8"]);
    egress.destination_ranges = ranges(&["0.0.0.0/0", "172.16.0.0/12"]);
    egress.disabled = Some(false);

    assert_eq!(
        map_firewall_rule(&egress),
        FirewallRule {
            name: "deny-egress-rfc1918".to_string(),
            network: "shared-vpc".to_string(),
            direction: FirewallDirection::Egress,
            priority: Some(65000),
            action: FirewallAction::Deny,
            targets: FirewallTargets::ServiceAccounts(vec![
                "batch@acme-prod.iam.gserviceaccount.com".to_string()
            ]),
            source_or_dest: vec!["0.0.0.0/0".to_string(), "172.16.0.0/12".to_string()],
            disabled: false,
        }
    );
}

/// Every arm of the direction ladder, the unrecognised one included, and the
/// two shapes that reach it as `None`: no key, and an enum that arrived as an
/// unknown integer and so carries no name.
#[test]
fn direction_covers_every_arm_including_an_unrecognised_name() {
    let cases = [
        (Some(Direction::Ingress), FirewallDirection::Ingress),
        (Some(Direction::Egress), FirewallDirection::Egress),
        (None, FirewallDirection::Unspecified),
        (
            Some(Direction::from("SIDEWAYS")),
            FirewallDirection::Unrecognized("SIDEWAYS".to_string()),
        ),
        (Some(Direction::from(97)), FirewallDirection::Unspecified),
        (Some(Direction::from("")), FirewallDirection::Unspecified),
    ];
    for (direction, expected) in cases {
        let mut firewall = rule("directional");
        firewall.direction = direction.clone();
        assert_eq!(
            map_firewall_rule(&firewall).direction,
            expected,
            "direction {direction:?}"
        );
    }
}

/// The direction decides the list, and it decides it even when the other list
/// is the only populated one: an INGRESS rule with no `sourceRanges` paints an
/// empty cell rather than borrowing the destinations.
#[test]
fn a_known_direction_never_borrows_the_other_range_list() {
    let mut ingress = rule("ingress-no-sources");
    ingress.direction = Some(Direction::Ingress);
    ingress.source_ranges = Vec::new();
    ingress.destination_ranges = ranges(&["203.0.113.0/24"]);
    assert_eq!(
        map_firewall_rule(&ingress).source_or_dest,
        Vec::<String>::new()
    );

    let mut egress = rule("egress-no-destinations");
    egress.direction = Some(Direction::Egress);
    egress.source_ranges = ranges(&["10.0.0.0/8"]);
    egress.destination_ranges = Vec::new();
    assert_eq!(
        map_firewall_rule(&egress).source_or_dest,
        Vec::<String>::new()
    );
}

/// The guarded fallback, both sides. With the direction unreadable the rule
/// carries whichever list is populated; the two lists differ everywhere, so a
/// guard forced to either constant picks the wrong one in one of these cases.
#[test]
fn an_unreadable_direction_falls_back_to_whichever_list_is_populated() {
    let unreadable = [None, Some(Direction::from("SIDEWAYS"))];

    for direction in unreadable {
        let mut sources_only = rule("sources-only");
        sources_only.direction = direction.clone();
        sources_only.source_ranges = ranges(&["10.0.0.0/8", "10.1.0.0/16"]);
        sources_only.destination_ranges = Vec::new();
        assert_eq!(
            map_firewall_rule(&sources_only).source_or_dest,
            vec!["10.0.0.0/8".to_string(), "10.1.0.0/16".to_string()],
            "sources only, direction {direction:?}"
        );

        let mut destinations_only = rule("destinations-only");
        destinations_only.direction = direction.clone();
        destinations_only.source_ranges = Vec::new();
        destinations_only.destination_ranges = ranges(&["0.0.0.0/0", "198.51.100.0/24"]);
        assert_eq!(
            map_firewall_rule(&destinations_only).source_or_dest,
            vec!["0.0.0.0/0".to_string(), "198.51.100.0/24".to_string()],
            "destinations only, direction {direction:?}"
        );

        // Both populated: the guard fires first, so the sources win.
        let mut both = rule("both-lists");
        both.direction = direction.clone();
        both.source_ranges = ranges(&["10.2.0.0/16"]);
        both.destination_ranges = ranges(&["203.0.113.0/24"]);
        assert_eq!(
            map_firewall_rule(&both).source_or_dest,
            vec!["10.2.0.0/16".to_string()],
            "both lists, direction {direction:?}"
        );

        // Neither populated: an empty cell, not a panic and not a borrowed one.
        let mut neither = rule("neither-list");
        neither.direction = direction.clone();
        assert_eq!(
            map_firewall_rule(&neither).source_or_dest,
            Vec::<String>::new(),
            "neither list, direction {direction:?}"
        );
    }
}

/// Presence of the two rule lists, over the whole cross-product. Both the
/// impossible shapes — neither key and both keys — are `Unspecified` rather
/// than the `ALLOW` a default would paint on a rule nobody could read.
#[test]
fn action_is_allow_or_deny_only_when_exactly_one_list_is_populated() {
    let cases = [
        (false, false, FirewallAction::Unspecified),
        (true, false, FirewallAction::Allow),
        (false, true, FirewallAction::Deny),
        (true, true, FirewallAction::Unspecified),
    ];
    for (has_allowed, has_denied, expected) in cases {
        let mut firewall = rule("action-case");
        if has_allowed {
            firewall.allowed = vec![allowed("tcp", &["22"])];
        }
        if has_denied {
            firewall.denied = vec![denied("udp")];
        }
        assert_eq!(
            map_firewall_rule(&firewall).action,
            expected,
            "allowed {has_allowed}, denied {has_denied}"
        );
    }
}

/// The whole targets cross-product, with two values per selector so order is
/// pinned as well as membership. No selector at all is `AllInstances` — the
/// widest scope — and not an empty list.
#[test]
fn targets_read_tags_service_accounts_both_or_every_instance() {
    let tags = ["web", "edge"];
    let accounts = [
        "batch@acme-prod.iam.gserviceaccount.com",
        "api@acme-prod.iam.gserviceaccount.com",
    ];

    let mut none = rule("no-selectors");
    none.target_tags = Vec::new();
    none.target_service_accounts = Vec::new();
    assert_eq!(
        map_firewall_rule(&none).targets,
        FirewallTargets::AllInstances
    );

    let mut tagged = rule("tagged");
    tagged.target_tags = ranges(&tags);
    assert_eq!(
        map_firewall_rule(&tagged).targets,
        FirewallTargets::Tags(vec!["web".to_string(), "edge".to_string()])
    );

    let mut by_account = rule("by-account");
    by_account.target_service_accounts = ranges(&accounts);
    assert_eq!(
        map_firewall_rule(&by_account).targets,
        FirewallTargets::ServiceAccounts(vec![
            "batch@acme-prod.iam.gserviceaccount.com".to_string(),
            "api@acme-prod.iam.gserviceaccount.com".to_string(),
        ])
    );

    let mut both = rule("both-selectors");
    both.target_tags = ranges(&tags);
    both.target_service_accounts = ranges(&accounts);
    assert_eq!(
        map_firewall_rule(&both).targets,
        FirewallTargets::Both {
            tags: vec!["web".to_string(), "edge".to_string()],
            service_accounts: vec![
                "batch@acme-prod.iam.gserviceaccount.com".to_string(),
                "api@acme-prod.iam.gserviceaccount.com".to_string(),
            ],
        }
    );
}

/// The API reports priority as a signed integer and the row holds an unsigned
/// one. A value that does not fit is dropped rather than wrapped, and an
/// unreported priority is a dash rather than the `1000` creation default.
#[test]
fn a_priority_that_does_not_fit_is_dropped_and_a_real_one_is_kept() {
    let cases = [
        (None, None),
        (Some(0), Some(0)),
        (Some(1200), Some(1200)),
        (Some(65535), Some(65535)),
        (Some(i32::MAX), Some(i32::MAX as u32)),
        (Some(-1), None),
        (Some(i32::MIN), None),
    ];
    for (priority, expected) in cases {
        let mut firewall = rule("priority-case");
        firewall.priority = priority;
        assert_eq!(
            map_firewall_rule(&firewall).priority,
            expected,
            "priority {priority:?}"
        );
    }
}

/// The badge that keeps a disabled DENY rule from reading like a live one. An
/// unreported flag is enabled, which is what the API documents.
#[test]
fn a_rule_is_enabled_unless_the_api_reported_it_disabled() {
    let cases = [(Some(true), true), (Some(false), false), (None, false)];
    for (flag, expected) in cases {
        let mut firewall = rule("disabled-case");
        firewall.disabled = flag;
        assert_eq!(
            map_firewall_rule(&firewall).disabled,
            expected,
            "disabled {flag:?}"
        );
    }
}

/// The same URL-tail reduction the subnet row gets, plus the empty-string
/// fallbacks, so the two tables cannot disagree about what a network is called.
#[test]
fn a_rules_network_is_reduced_and_its_missing_fields_are_empty() {
    let mut linked = rule("linked");
    linked.network = Some(NETWORK_LINK.to_string());
    assert_eq!(map_firewall_rule(&linked).network, "shared-vpc");

    let mut bare = rule("bare");
    bare.network = Some("legacy-net".to_string());
    assert_eq!(map_firewall_rule(&bare).network, "legacy-net");

    let mut missing = rule("missing");
    missing.name = None;
    missing.network = None;
    let mapped = map_firewall_rule(&missing);
    assert_eq!(mapped.name, "");
    assert_eq!(mapped.network, "");
}
