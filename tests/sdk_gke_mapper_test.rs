//! The native GKE mappers, pinned field by field.
//!
//! The mutation sweep reported `map_cluster` and `map_cluster_detail` in
//! `src/services/sdk/gke.rs` as untested: nothing constructed a
//! `container_v1::model::Cluster` and asserted what came out the other side,
//! so every mutant of those bodies survived. Each test here plants a distinct,
//! non-default value for the field it names and asserts the exact mapped
//! output, and each conditional — the preferred/deprecated endpoint pair, the
//! empty-string folds, the addon presence/enablement split, the `try_from`
//! narrowings — is exercised from both sides with inputs that map to
//! *different* results. A body replaced by a constant, a dropped `!`, or a
//! match guard forced to `true`/`false` therefore changes an assertion.

#![allow(deprecated)]

use google_cloud_container_v1::model::control_plane_endpoints_config::IPEndpointsConfig;
use google_cloud_container_v1::model::{
    AddonsConfig, Cluster, ControlPlaneEndpointsConfig, GcePersistentDiskCsiDriverConfig,
    HorizontalPodAutoscaling, HttpLoadBalancing, NodeConfig, NodePool, NodePoolAutoscaling,
    PrivateClusterConfig, cluster, node_pool,
};
use nuvem::services::sdk::gke::{map_cluster, map_cluster_detail};
use nuvem_domain::gcp::resource::{
    ComputeKeyValue, EndpointVisibility, GkeAddon, GkeCluster, GkeClusterDetail, GkeClusterStatus,
    GkeNodePool,
};

const HTTP_LOAD_BALANCING: &str = "HTTP load balancing";
const HORIZONTAL_POD_AUTOSCALING: &str = "Horizontal Pod Autoscaling";
const GCE_PD_CSI_DRIVER: &str = "GCE Persistent Disk CSI Driver";

/// A cluster with a distinct, non-default value in every field either mapper
/// reads. Tests that care about one field override that field and nothing
/// else, so a mutant that drops an unrelated field still fails here.
fn planted_cluster() -> Cluster {
    Cluster::new()
        .set_name("planted-cluster")
        .set_location("europe-west4-a")
        .set_status(cluster::Status::Running)
        .set_current_master_version("1.29.3-gke.1093000")
        .set_current_node_version("1.28.9-gke.1069000")
        .set_endpoint("203.0.113.7")
        .set_current_node_count(7_i32)
        .set_network("projects/planted/global/networks/planted-network")
        .set_subnetwork("projects/planted/regions/europe-west4/subnetworks/planted-subnet")
        .set_resource_labels([("team", "platform")])
}

/// The control-plane config the API now reports, with the public endpoint set
/// either way. Its sense is the reverse of the deprecated flag.
fn ip_endpoints(public: bool) -> ControlPlaneEndpointsConfig {
    ControlPlaneEndpointsConfig::new()
        .set_ip_endpoints_config(IPEndpointsConfig::new().set_enable_public_endpoint(public))
}

fn private_cluster_config(private_endpoint: bool) -> PrivateClusterConfig {
    PrivateClusterConfig::new().set_enable_private_endpoint(private_endpoint)
}

/// The detail struct with its JSON blanked, so a whole-struct comparison can
/// state every other field at once.
fn without_json(detail: GkeClusterDetail) -> GkeClusterDetail {
    GkeClusterDetail {
        raw_json: String::new(),
        ..detail
    }
}

// ---------------------------------------------------------------------------
// map_cluster
// ---------------------------------------------------------------------------

#[test]
fn a_cluster_row_carries_every_planted_field() {
    let cluster = planted_cluster().set_control_plane_endpoints_config(ip_endpoints(true));

    assert_eq!(
        map_cluster(&cluster),
        GkeCluster {
            name: "planted-cluster".to_string(),
            location: "europe-west4-a".to_string(),
            status: GkeClusterStatus::Running,
            current_master_version: Some("1.29.3-gke.1093000".to_string()),
            current_node_count: Some(7),
            endpoint_visibility: EndpointVisibility::Public,
        }
    );
}

#[test]
fn a_blank_master_version_is_not_a_version() {
    let cluster = planted_cluster()
        .set_current_master_version("")
        .set_control_plane_endpoints_config(ip_endpoints(true));

    assert_eq!(map_cluster(&cluster).current_master_version, None);
    // And the populated case maps through unchanged, so the `!` in front of
    // `is_empty()` cannot be dropped without one of the two failing.
    assert_eq!(
        map_cluster(&planted_cluster()).current_master_version,
        Some("1.29.3-gke.1093000".to_string())
    );
}

#[test]
fn every_named_cluster_status_maps_to_its_own_variant() {
    for (planted, expected) in [
        (cluster::Status::Running, GkeClusterStatus::Running),
        (cluster::Status::Reconciling, GkeClusterStatus::Reconciling),
        (cluster::Status::Stopping, GkeClusterStatus::Stopping),
        (cluster::Status::Error, GkeClusterStatus::Error),
        (
            cluster::Status::Provisioning,
            GkeClusterStatus::Unknown("PROVISIONING".to_string()),
        ),
        (
            cluster::Status::Degraded,
            GkeClusterStatus::Unknown("DEGRADED".to_string()),
        ),
    ] {
        let cluster = planted_cluster().set_status(planted.clone());
        assert_eq!(
            map_cluster(&cluster).status,
            expected,
            "status {planted:?} mapped wrong"
        );
    }
}

#[test]
fn a_status_the_client_cannot_name_becomes_the_empty_unknown() {
    // An enum deserialised from an unrecognised *integer* has no name at all,
    // which is the `unwrap_or_default()` arm: the empty string, not a panic
    // and not the previous cluster's status.
    let cluster = planted_cluster().set_status(cluster::Status::from(4242_i32));

    assert_eq!(
        map_cluster(&cluster).status,
        GkeClusterStatus::Unknown(String::new())
    );
}

#[test]
fn a_node_count_the_api_reports_negative_is_not_a_node_count() {
    let cluster = planted_cluster().set_current_node_count(-3_i32);

    assert_eq!(map_cluster(&cluster).current_node_count, None);
    assert_eq!(map_cluster(&planted_cluster()).current_node_count, Some(7));
}

#[test]
fn a_cluster_reporting_zero_nodes_reports_nothing_rather_than_zero() {
    // proto3 has no absent-vs-zero for a scalar: an omitted
    // `currentNodeCount` and a stated zero deserialize to the same 0, and the
    // CLI reads the same wire document into `None`. Passing the zero through
    // painted "0 nodes" in the table where the CLI painted a dash -- visible
    // on `legacy-unknown` once `--demo` moved to this backend. The fold is
    // what the field comment beside it always claimed, and now what the
    // conversion does.
    let cluster = planted_cluster().set_current_node_count(0_i32);

    assert_eq!(map_cluster(&cluster).current_node_count, None);
    assert_eq!(
        map_cluster(&planted_cluster()).current_node_count,
        Some(7),
        "and a stated non-zero count still reaches the cell"
    );
}

#[test]
fn a_cluster_that_states_only_the_deprecated_zone_is_still_placed() {
    // `location` supersedes `zone`, but a zonal cluster created before the
    // change answers `zone` and leaves `location` empty. Reading `location`
    // alone painted a blank REGION cell for such a cluster natively while the
    // CLI painted the zone, because `parse_clusters_json` has always fallen
    // back to it.
    #[allow(deprecated)]
    let cluster = planted_cluster()
        .set_location("")
        .set_zone("europe-west1-b");

    assert_eq!(map_cluster(&cluster).location, "europe-west1-b");
    assert_eq!(map_cluster_detail(&cluster).location, "europe-west1-b");
}

#[test]
fn a_cluster_that_states_both_places_prefers_the_current_field() {
    // `zone` is deprecated, so where both are stated the newer one wins --
    // otherwise a regional cluster that still echoes a zone would read as
    // zonal.
    #[allow(deprecated)]
    let cluster = planted_cluster()
        .set_location("europe-west1")
        .set_zone("europe-west1-b");

    assert_eq!(map_cluster(&cluster).location, "europe-west1");
    assert_eq!(map_cluster_detail(&cluster).location, "europe-west1");
}

#[test]
fn the_current_control_plane_config_wins_over_the_deprecated_private_flag() {
    // Both reported, and they disagree: the new config says public, the old
    // one says private. Preferring the deprecated field would flip this.
    let cluster = planted_cluster()
        .set_control_plane_endpoints_config(ip_endpoints(true))
        .set_private_cluster_config(private_cluster_config(true));

    assert_eq!(
        map_cluster(&cluster).endpoint_visibility,
        EndpointVisibility::Public
    );
}

#[test]
fn a_public_endpoint_reported_off_reads_private_even_when_the_old_flag_says_otherwise() {
    let cluster = planted_cluster()
        .set_control_plane_endpoints_config(ip_endpoints(false))
        .set_private_cluster_config(private_cluster_config(false));

    assert_eq!(
        map_cluster(&cluster).endpoint_visibility,
        EndpointVisibility::Private
    );
}

#[test]
fn the_deprecated_flag_answers_when_the_new_config_is_absent() {
    let private = planted_cluster().set_private_cluster_config(private_cluster_config(true));
    let public = planted_cluster().set_private_cluster_config(private_cluster_config(false));

    assert_eq!(
        map_cluster(&private).endpoint_visibility,
        EndpointVisibility::Private
    );
    assert_eq!(
        map_cluster(&public).endpoint_visibility,
        EndpointVisibility::Public
    );
}

#[test]
fn an_empty_control_plane_config_still_falls_back_to_the_deprecated_flag() {
    // Two shapes of "the new config told us nothing": the block is there but
    // carries no IP endpoints, and the IP endpoints are there but carry no
    // public-endpoint answer. Both must reach the fallback.
    let no_ip_endpoints = planted_cluster()
        .set_control_plane_endpoints_config(ControlPlaneEndpointsConfig::new())
        .set_private_cluster_config(private_cluster_config(true));
    let no_public_flag = planted_cluster()
        .set_control_plane_endpoints_config(
            ControlPlaneEndpointsConfig::new().set_ip_endpoints_config(IPEndpointsConfig::new()),
        )
        .set_private_cluster_config(private_cluster_config(true));

    assert_eq!(
        map_cluster(&no_ip_endpoints).endpoint_visibility,
        EndpointVisibility::Private
    );
    assert_eq!(
        map_cluster(&no_public_flag).endpoint_visibility,
        EndpointVisibility::Private
    );
}

#[test]
fn a_cluster_reporting_neither_config_is_treated_as_not_private() {
    let cluster = planted_cluster();

    assert_eq!(
        map_cluster(&cluster).endpoint_visibility,
        EndpointVisibility::Public
    );
}

#[test]
fn a_blank_endpoint_on_a_non_private_cluster_reads_unknown_not_public() {
    let cluster = planted_cluster().set_endpoint("");

    assert_eq!(
        map_cluster(&cluster).endpoint_visibility,
        EndpointVisibility::Unknown
    );
    // A private cluster with no endpoint is still private, so the blank
    // endpoint is not what decides the private case.
    let private = planted_cluster()
        .set_endpoint("")
        .set_private_cluster_config(private_cluster_config(true));
    assert_eq!(
        map_cluster(&private).endpoint_visibility,
        EndpointVisibility::Private
    );
}

// ---------------------------------------------------------------------------
// map_cluster_detail
// ---------------------------------------------------------------------------

#[test]
fn a_cluster_detail_carries_every_planted_field() {
    let cluster = planted_cluster()
        .set_control_plane_endpoints_config(ip_endpoints(true))
        .set_node_pools([NodePool::new()
            .set_name("pool-alpha")
            .set_status(node_pool::Status::Running)
            .set_version("1.28.9-gke.1069000")
            .set_config(NodeConfig::new().set_machine_type("e2-standard-4"))
            .set_autoscaling(
                NodePoolAutoscaling::new()
                    .set_enabled(true)
                    .set_min_node_count(2_i32)
                    .set_max_node_count(9_i32),
            )
            .set_initial_node_count(3_i32)])
        .set_addons_config(
            AddonsConfig::new()
                .set_http_load_balancing(HttpLoadBalancing::new().set_disabled(false))
                .set_horizontal_pod_autoscaling(HorizontalPodAutoscaling::new().set_disabled(false))
                .set_gce_persistent_disk_csi_driver_config(
                    GcePersistentDiskCsiDriverConfig::new().set_enabled(true),
                ),
        );

    let mapped = map_cluster_detail(&cluster);

    assert_eq!(
        without_json(mapped),
        GkeClusterDetail {
            name: "planted-cluster".to_string(),
            location: "europe-west4-a".to_string(),
            status: GkeClusterStatus::Running,
            current_master_version: Some("1.29.3-gke.1093000".to_string()),
            current_node_version: Some("1.28.9-gke.1069000".to_string()),
            endpoint: Some("203.0.113.7".to_string()),
            private_cluster: false,
            network: Some("planted-network".to_string()),
            subnetwork: Some("planted-subnet".to_string()),
            node_pools: vec![GkeNodePool {
                name: "pool-alpha".to_string(),
                status: GkeClusterStatus::Running,
                version: Some("1.28.9-gke.1069000".to_string()),
                machine_type: Some("e2-standard-4".to_string()),
                autoscaling: Some("2-9".to_string()),
                node_count: Some(3),
            }],
            addons: vec![
                GkeAddon {
                    name: HTTP_LOAD_BALANCING.to_string(),
                    enabled: true,
                },
                GkeAddon {
                    name: HORIZONTAL_POD_AUTOSCALING.to_string(),
                    enabled: true,
                },
                GkeAddon {
                    name: GCE_PD_CSI_DRIVER.to_string(),
                    enabled: true,
                },
            ],
            labels: vec![ComputeKeyValue {
                key: "team".to_string(),
                value: "platform".to_string(),
            }],
            raw_json: String::new(),
        }
    );
}

#[test]
fn the_detail_pane_folds_the_same_blanks_the_row_does() {
    let cluster = planted_cluster()
        .set_current_master_version("")
        .set_current_node_version("")
        .set_endpoint("")
        .set_network("")
        .set_subnetwork("");

    let mapped = map_cluster_detail(&cluster);

    assert_eq!(mapped.current_master_version, None);
    assert_eq!(mapped.current_node_version, None);
    assert_eq!(mapped.endpoint, None);
    assert_eq!(mapped.network, None);
    assert_eq!(mapped.subnetwork, None);
}

#[test]
fn the_network_and_subnetwork_are_the_last_url_segment() {
    let cluster = planted_cluster()
        .set_network("projects/other/global/networks/vpc-seven")
        .set_subnetwork("projects/other/regions/europe-west4/subnetworks/subnet-nine");

    let mapped = map_cluster_detail(&cluster);

    assert_eq!(mapped.network, Some("vpc-seven".to_string()));
    assert_eq!(mapped.subnetwork, Some("subnet-nine".to_string()));
}

#[test]
fn the_detail_pane_reads_private_from_the_same_pair_the_row_uses() {
    let new_config_says_public = planted_cluster()
        .set_control_plane_endpoints_config(ip_endpoints(true))
        .set_private_cluster_config(private_cluster_config(true));
    let new_config_says_private = planted_cluster()
        .set_control_plane_endpoints_config(ip_endpoints(false))
        .set_private_cluster_config(private_cluster_config(false));

    assert!(!map_cluster_detail(&new_config_says_public).private_cluster);
    assert!(map_cluster_detail(&new_config_says_private).private_cluster);
}

#[test]
fn the_detail_pane_falls_back_to_the_deprecated_private_flag() {
    let private = planted_cluster().set_private_cluster_config(private_cluster_config(true));
    let public = planted_cluster().set_private_cluster_config(private_cluster_config(false));
    let unreported = planted_cluster();

    assert!(map_cluster_detail(&private).private_cluster);
    assert!(!map_cluster_detail(&public).private_cluster);
    assert!(!map_cluster_detail(&unreported).private_cluster);
}

#[test]
fn node_pools_keep_their_reported_order_and_their_own_values() {
    let cluster = planted_cluster().set_node_pools([
        NodePool::new()
            .set_name("pool-alpha")
            .set_status(node_pool::Status::Running)
            .set_version("1.28.9-gke.1069000")
            .set_config(NodeConfig::new().set_machine_type("e2-standard-4"))
            .set_initial_node_count(3_i32),
        NodePool::new()
            .set_name("pool-beta")
            .set_status(node_pool::Status::Reconciling)
            .set_version("1.27.8-gke.1067000")
            .set_config(NodeConfig::new().set_machine_type("n2-highmem-8"))
            .set_initial_node_count(11_i32),
    ]);

    let pools = map_cluster_detail(&cluster).node_pools;

    assert_eq!(
        pools,
        vec![
            GkeNodePool {
                name: "pool-alpha".to_string(),
                status: GkeClusterStatus::Running,
                version: Some("1.28.9-gke.1069000".to_string()),
                machine_type: Some("e2-standard-4".to_string()),
                autoscaling: None,
                node_count: Some(3),
            },
            GkeNodePool {
                name: "pool-beta".to_string(),
                status: GkeClusterStatus::Reconciling,
                version: Some("1.27.8-gke.1067000".to_string()),
                machine_type: Some("n2-highmem-8".to_string()),
                autoscaling: None,
                node_count: Some(11),
            },
        ]
    );
}

#[test]
fn a_node_pools_autoscaling_cell_distinguishes_absent_off_and_a_range() {
    let cluster = planted_cluster().set_node_pools([
        NodePool::new().set_name("unreported"),
        NodePool::new()
            .set_name("off")
            .set_autoscaling(NodePoolAutoscaling::new().set_enabled(false)),
        NodePool::new().set_name("on").set_autoscaling(
            NodePoolAutoscaling::new()
                .set_enabled(true)
                .set_min_node_count(2_i32)
                .set_max_node_count(9_i32),
        ),
    ]);

    let cells: Vec<Option<String>> = map_cluster_detail(&cluster)
        .node_pools
        .into_iter()
        .map(|pool| pool.autoscaling)
        .collect();

    assert_eq!(
        cells,
        vec![None, Some("off".to_string()), Some("2-9".to_string())]
    );
}

#[test]
fn autoscaling_bounds_the_client_reports_negative_fall_back_to_zero() {
    let cluster =
        planted_cluster().set_node_pools([NodePool::new().set_name("pool").set_autoscaling(
            NodePoolAutoscaling::new()
                .set_enabled(true)
                .set_min_node_count(-1_i32)
                .set_max_node_count(-4_i32),
        )]);

    assert_eq!(
        map_cluster_detail(&cluster).node_pools[0].autoscaling,
        Some("0-0".to_string())
    );
}

#[test]
fn a_node_pool_machine_type_is_absent_when_unreported_or_blank() {
    let cluster = planted_cluster().set_node_pools([
        NodePool::new().set_name("no-config"),
        NodePool::new()
            .set_name("blank-machine")
            .set_config(NodeConfig::new().set_machine_type("")),
        NodePool::new()
            .set_name("named-machine")
            .set_config(NodeConfig::new().set_machine_type("c3-standard-22")),
    ]);

    let machines: Vec<Option<String>> = map_cluster_detail(&cluster)
        .node_pools
        .into_iter()
        .map(|pool| pool.machine_type)
        .collect();

    assert_eq!(
        machines,
        vec![None, None, Some("c3-standard-22".to_string())]
    );
}

#[test]
fn a_node_pool_version_is_absent_when_blank_and_present_otherwise() {
    let cluster = planted_cluster().set_node_pools([
        NodePool::new().set_name("blank").set_version(""),
        NodePool::new()
            .set_name("versioned")
            .set_version("1.26.15-gke.1390000"),
    ]);

    let versions: Vec<Option<String>> = map_cluster_detail(&cluster)
        .node_pools
        .into_iter()
        .map(|pool| pool.version)
        .collect();

    assert_eq!(
        versions,
        vec![None, Some("1.26.15-gke.1390000".to_string())]
    );
}

#[test]
fn a_node_pool_count_the_api_reports_negative_is_not_a_count() {
    let cluster = planted_cluster().set_node_pools([
        NodePool::new()
            .set_name("negative")
            .set_initial_node_count(-2_i32),
        NodePool::new()
            .set_name("counted")
            .set_initial_node_count(5_i32),
    ]);

    let counts: Vec<Option<u32>> = map_cluster_detail(&cluster)
        .node_pools
        .into_iter()
        .map(|pool| pool.node_count)
        .collect();

    assert_eq!(counts, vec![None, Some(5)]);
}

#[test]
fn a_node_pool_status_the_client_cannot_name_becomes_the_empty_unknown() {
    let cluster = planted_cluster().set_node_pools([
        NodePool::new()
            .set_name("unnamed-status")
            .set_status(node_pool::Status::from(777_i32)),
        NodePool::new()
            .set_name("degraded-ish")
            .set_status(node_pool::Status::RunningWithError),
    ]);

    let statuses: Vec<GkeClusterStatus> = map_cluster_detail(&cluster)
        .node_pools
        .into_iter()
        .map(|pool| pool.status)
        .collect();

    assert_eq!(
        statuses,
        vec![
            GkeClusterStatus::Unknown(String::new()),
            GkeClusterStatus::Unknown("RUNNING_WITH_ERROR".to_string()),
        ]
    );
}

#[test]
fn a_cluster_with_no_addons_block_has_no_addon_rows() {
    let cluster = planted_cluster();

    assert_eq!(map_cluster_detail(&cluster).addons, Vec::<GkeAddon>::new());
}

#[test]
fn an_addon_the_cluster_did_not_report_yields_no_row_at_all() {
    // Only HTTP load balancing is reported, so the other two are absent rather
    // than present-and-off: "not reported" and "reported off" are different
    // answers and the pane must not invent the second.
    let cluster = planted_cluster().set_addons_config(
        AddonsConfig::new().set_http_load_balancing(HttpLoadBalancing::new().set_disabled(false)),
    );

    assert_eq!(
        map_cluster_detail(&cluster).addons,
        vec![GkeAddon {
            name: HTTP_LOAD_BALANCING.to_string(),
            enabled: true,
        }]
    );
}

#[test]
fn reported_addons_carry_their_own_enablement_in_pane_order() {
    // HTTP load balancing off between two that are on: a constant substituted
    // for the first entry's enablement shows up against its neighbours rather
    // than agreeing with them.
    let cluster = planted_cluster().set_addons_config(
        AddonsConfig::new()
            .set_http_load_balancing(HttpLoadBalancing::new().set_disabled(true))
            .set_horizontal_pod_autoscaling(HorizontalPodAutoscaling::new().set_disabled(false))
            .set_gce_persistent_disk_csi_driver_config(
                GcePersistentDiskCsiDriverConfig::new().set_enabled(false),
            ),
    );

    assert_eq!(
        map_cluster_detail(&cluster).addons,
        vec![
            GkeAddon {
                name: HTTP_LOAD_BALANCING.to_string(),
                enabled: false,
            },
            GkeAddon {
                name: HORIZONTAL_POD_AUTOSCALING.to_string(),
                enabled: true,
            },
            GkeAddon {
                name: GCE_PD_CSI_DRIVER.to_string(),
                enabled: false,
            },
        ]
    );
}

#[test]
fn horizontal_pod_autoscaling_reported_off_reads_off() {
    // The middle addon on its own false side, which no other case gives it:
    // without this, its `!addon.disabled` could answer a constant `true` and
    // every other addon assertion would still agree.
    let cluster = planted_cluster().set_addons_config(
        AddonsConfig::new()
            .set_http_load_balancing(HttpLoadBalancing::new().set_disabled(false))
            .set_horizontal_pod_autoscaling(HorizontalPodAutoscaling::new().set_disabled(true))
            .set_gce_persistent_disk_csi_driver_config(
                GcePersistentDiskCsiDriverConfig::new().set_enabled(true),
            ),
    );

    assert_eq!(
        map_cluster_detail(&cluster).addons,
        vec![
            GkeAddon {
                name: HTTP_LOAD_BALANCING.to_string(),
                enabled: true,
            },
            GkeAddon {
                name: HORIZONTAL_POD_AUTOSCALING.to_string(),
                enabled: false,
            },
            GkeAddon {
                name: GCE_PD_CSI_DRIVER.to_string(),
                enabled: true,
            },
        ]
    );
}

#[test]
fn a_disabled_http_load_balancer_and_an_enabled_csi_driver_read_opposite_ways() {
    // The two addons whose reported field has the opposite sense, checked
    // against each other: `disabled` is negated, `enabled` is not.
    let cluster = planted_cluster().set_addons_config(
        AddonsConfig::new()
            .set_http_load_balancing(HttpLoadBalancing::new().set_disabled(false))
            .set_gce_persistent_disk_csi_driver_config(
                GcePersistentDiskCsiDriverConfig::new().set_enabled(false),
            ),
    );

    assert_eq!(
        map_cluster_detail(&cluster).addons,
        vec![
            GkeAddon {
                name: HTTP_LOAD_BALANCING.to_string(),
                enabled: true,
            },
            GkeAddon {
                name: GCE_PD_CSI_DRIVER.to_string(),
                enabled: false,
            },
        ]
    );
}

#[test]
fn every_resource_label_reaches_the_pane_with_its_own_value() {
    // `resource_labels` is a HashMap, so the mapper's output order is the
    // map's; the set of pairs is what is asserted.
    let cluster =
        planted_cluster().set_resource_labels([("team", "platform"), ("env", "production")]);

    let mut labels = map_cluster_detail(&cluster).labels;
    labels.sort_by_key(|label| label.key.clone());

    assert_eq!(
        labels,
        vec![
            ComputeKeyValue {
                key: "env".to_string(),
                value: "production".to_string(),
            },
            ComputeKeyValue {
                key: "team".to_string(),
                value: "platform".to_string(),
            },
        ]
    );
}

#[test]
fn a_cluster_with_no_labels_has_no_label_rows() {
    let cluster = planted_cluster().set_resource_labels(Vec::<(String, String)>::new());

    assert_eq!(
        map_cluster_detail(&cluster).labels,
        Vec::<ComputeKeyValue>::new()
    );
}

#[test]
fn the_json_tab_renders_the_cluster_the_pane_was_built_from() {
    let cluster = planted_cluster().set_node_pools([NodePool::new().set_name("pool-alpha")]);

    let raw = map_cluster_detail(&cluster).raw_json;
    let parsed: serde_json::Value =
        serde_json::from_str(&raw).expect("the JSON tab must hold parseable JSON");

    assert_eq!(parsed["name"], serde_json::json!("planted-cluster"));
    assert_eq!(parsed["location"], serde_json::json!("europe-west4-a"));
    assert_eq!(
        parsed["nodePools"][0]["name"],
        serde_json::json!("pool-alpha")
    );
}
