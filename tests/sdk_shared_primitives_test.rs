//! The primitives both backends share, pinned against their mutants.
//!
//! The SDK sweep reported the small shared deciders — `cloudsql::connectivity`,
//! `gke::addons_from`, `gke::autoscaling_label`, `secrets::replication_label_from`
//! and the label/status translators next to them — as untested: the CLI parsers
//! exercise them only incidentally, so a flipped operator inside one survived.
//! Each test below drives both sides of one conditional with inputs that produce
//! different output, so the operator swaps (`||` -> `&&`, `==` -> `!=`), the `!`
//! deletions, the match-guard replacements and the whole-body `Default::default()`
//! substitutions all change an asserted value rather than passing unnoticed.

use nuvem::services::resources::cloudsql::{
    connectivity, normalize_engine, parse_availability, parse_instances_json, parse_status,
};
use nuvem::services::resources::gke::{
    GCE_PD_CSI_DRIVER, HORIZONTAL_POD_AUTOSCALING, HTTP_LOAD_BALANCING, addons_from,
    autoscaling_label, endpoint_visibility, parse_cluster_detail_json, parse_cluster_status,
    tail_segment,
};
use nuvem::services::resources::secrets::{
    key_values, replication_label_from, rotation_label_from, short_id, version_state,
};
use nuvem_domain::gcp::resource::{
    CloudSqlAvailability, CloudSqlInstance, CloudSqlStatus, ComputeKeyValue, EndpointVisibility,
    GkeAddon, GkeClusterStatus, GkeNodePool, SecretVersionState,
};

// ---------------------------------------------------------------------------
// cloudsql::connectivity
// ---------------------------------------------------------------------------

/// The private branch wins over both public signals, so `private` is not one
/// input among three that the public signals can outvote. Pins the `Private`
/// answer against every combination the other two arguments can take; no
/// operator swap inside the `||` can reach this row.
#[test]
fn connectivity_reports_private_whatever_the_public_signals_say() {
    assert_eq!(
        connectivity(true, false, false),
        EndpointVisibility::Private
    );
    assert_eq!(connectivity(true, true, false), EndpointVisibility::Private);
    assert_eq!(connectivity(true, false, true), EndpointVisibility::Private);
    assert_eq!(connectivity(true, true, true), EndpointVisibility::Private);
}

/// An ipv4-enabled instance with no primary address is public. Kills
/// `ipv4 || has_primary_ip` -> `ipv4 && has_primary_ip`, which would answer
/// `Unknown` here.
#[test]
fn connectivity_reports_public_from_ipv4_alone() {
    assert_eq!(connectivity(false, true, false), EndpointVisibility::Public);
}

/// A primary address with ipv4 off is public too — the other half of the `||`,
/// and the half the CLI path depends on.
#[test]
fn connectivity_reports_public_from_a_primary_address_alone() {
    assert_eq!(connectivity(false, false, true), EndpointVisibility::Public);
}

/// Both signals present stays public, so an `&&` mutant cannot hide behind this
/// case alone.
#[test]
fn connectivity_reports_public_when_both_signals_are_present() {
    assert_eq!(connectivity(false, true, true), EndpointVisibility::Public);
}

/// Neither signal is `Unknown`, not `Public`. This is the only case that
/// reaches the final arm, so it is what stops a mapper from answering `Public`
/// for an instance with no public signal at all — the direction that matters,
/// since the wrong answer here reads as reachable.
#[test]
fn connectivity_reports_unknown_without_any_public_signal() {
    assert_eq!(
        connectivity(false, false, false),
        EndpointVisibility::Unknown
    );
}

// ---------------------------------------------------------------------------
// cloudsql::parse_instances_json — the connectivity decision through the CLI
// ---------------------------------------------------------------------------

/// Every row field carries a distinct non-default value, so replacing the
/// mapper body with `Default::default()` changes all eight of them.
#[test]
fn parse_instances_json_maps_every_row_field_distinctly() {
    let json = r#"[
      {"name":"sql-primary-reachable","databaseVersion":"MYSQL_8_0","region":"europe-west4",
       "state":"MAINTENANCE",
       "settings":{"tier":"db-n1-standard-8","availabilityType":"REGIONAL",
                   "ipConfiguration":{"ipv4Enabled":false}},
       "ipAddresses":[{"type":"PRIVATE","ipAddress":"10.0.0.9"},
                      {"type":"PRIMARY","ipAddress":"34.0.0.9"}]}
    ]"#;

    let rows = parse_instances_json(json).expect("instance JSON should parse");

    assert_eq!(
        rows,
        vec![CloudSqlInstance {
            name: "sql-primary-reachable".to_string(),
            database_version: "MYSQL_8_0".to_string(),
            engine: "MySQL".to_string(),
            region: "europe-west4".to_string(),
            tier: "db-n1-standard-8".to_string(),
            availability: CloudSqlAvailability::Regional,
            status: CloudSqlStatus::Maintenance,
            connectivity: EndpointVisibility::Public,
        }]
    );
}

/// `ipv4Enabled` is false and no private network is attached, so the row is
/// public only because one address is typed `PRIMARY` — and that address is
/// listed second, so the `.any()` must scan past the private one.
#[test]
fn parse_instances_json_reads_public_from_a_primary_address_when_ipv4_is_off() {
    let json = r#"[
      {"name":"primary-only","databaseVersion":"POSTGRES_15","region":"us-central1",
       "state":"RUNNABLE",
       "settings":{"tier":"db-custom-2-7680","availabilityType":"ZONAL",
                   "ipConfiguration":{"ipv4Enabled":false}},
       "ipAddresses":[{"type":"PRIVATE","ipAddress":"10.0.0.1"},
                      {"type":"PRIMARY","ipAddress":"34.0.0.1"}]}
    ]"#;

    let rows = parse_instances_json(json).expect("instance JSON should parse");

    assert_eq!(rows[0].connectivity, EndpointVisibility::Public);
}

/// The same shape with the primary address removed answers `Unknown`. Kills
/// `ip.kind.as_deref() == Some("PRIMARY")` -> `!=`, which would match the
/// `PRIVATE` entry and report this instance publicly reachable.
#[test]
fn parse_instances_json_reads_unknown_when_only_a_private_address_exists() {
    let json = r#"[
      {"name":"private-address-only","databaseVersion":"POSTGRES_15","region":"us-central1",
       "state":"RUNNABLE",
       "settings":{"tier":"db-custom-2-7680","availabilityType":"ZONAL",
                   "ipConfiguration":{"ipv4Enabled":false}},
       "ipAddresses":[{"type":"PRIVATE","ipAddress":"10.0.0.1"}]}
    ]"#;

    let rows = parse_instances_json(json).expect("instance JSON should parse");

    assert_eq!(rows[0].connectivity, EndpointVisibility::Unknown);
}

/// An attached `privateNetwork` makes the row private even though both public
/// signals are set — the `is_some()` result must reach `connectivity`'s first
/// branch.
#[test]
fn parse_instances_json_reads_private_when_a_private_network_is_attached() {
    let json = r#"[
      {"name":"peered","databaseVersion":"POSTGRES_15","region":"us-central1",
       "state":"RUNNABLE",
       "settings":{"tier":"db-custom-2-7680","availabilityType":"ZONAL",
                   "ipConfiguration":{"ipv4Enabled":true,
                                      "privateNetwork":"projects/p/global/networks/default"}},
       "ipAddresses":[{"type":"PRIMARY","ipAddress":"34.0.0.2"}]}
    ]"#;

    let rows = parse_instances_json(json).expect("instance JSON should parse");

    assert_eq!(rows[0].connectivity, EndpointVisibility::Private);
}

/// No addresses at all, `ipv4Enabled` true: public by the other half of the
/// `||`, read through the `.map(...).unwrap_or(false)` on the ip config.
#[test]
fn parse_instances_json_reads_public_from_ipv4_with_no_addresses() {
    let json = r#"[
      {"name":"ipv4-only","databaseVersion":"SQLSERVER_2019_STANDARD","region":"us-west1",
       "state":"RUNNABLE",
       "settings":{"tier":"db-custom-4-16384","availabilityType":"ZONAL",
                   "ipConfiguration":{"ipv4Enabled":true}}}
    ]"#;

    let rows = parse_instances_json(json).expect("instance JSON should parse");

    assert_eq!(rows[0].connectivity, EndpointVisibility::Public);
    assert_eq!(rows[0].engine, "SQL Server");
}

/// A missing `ipConfiguration` block leaves ipv4 off rather than on: with a
/// primary address the row is still public, without one it is unknown. The
/// second row is the only case where the ipv4 reading is the sole signal, so it
/// is what separates `|c| c.ipv4_enabled` from a `true` replacement.
#[test]
fn parse_instances_json_defaults_a_missing_ip_config_to_no_ipv4() {
    let json = r#"[
      {"name":"no-config-with-primary","databaseVersion":"POSTGRES_15","region":"us-east1",
       "state":"RUNNABLE","settings":{"tier":"db-g1-small","availabilityType":"ZONAL"},
       "ipAddresses":[{"type":"PRIMARY","ipAddress":"34.0.0.3"}]},
      {"name":"no-config-no-address","databaseVersion":"POSTGRES_15","region":"us-east1",
       "state":"RUNNABLE","settings":{"tier":"db-g1-small","availabilityType":"ZONAL"}}
    ]"#;

    let rows = parse_instances_json(json).expect("instance JSON should parse");

    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].name, "no-config-with-primary");
    assert_eq!(rows[0].connectivity, EndpointVisibility::Public);
    assert_eq!(rows[1].name, "no-config-no-address");
    assert_eq!(rows[1].connectivity, EndpointVisibility::Unknown);
}

/// Absent optional scalars collapse to the empty string rather than being
/// dropped, and the row order is the array order.
#[test]
fn parse_instances_json_defaults_absent_scalars_and_keeps_array_order() {
    let json = r#"[
      {"name":"second-in-file","state":"STOPPED","settings":{}},
      {"name":"first-in-file","databaseVersion":"POSTGRES_15","region":"us-central1",
       "state":"RUNNABLE","settings":{"tier":"db-g1-small"}}
    ]"#;

    let rows = parse_instances_json(json).expect("instance JSON should parse");

    assert_eq!(rows[0].name, "second-in-file");
    assert_eq!(rows[0].database_version, "");
    assert_eq!(rows[0].engine, "Unknown");
    assert_eq!(rows[0].region, "");
    assert_eq!(rows[0].tier, "");
    assert_eq!(rows[0].availability, CloudSqlAvailability::Unknown);
    assert_eq!(rows[0].status, CloudSqlStatus::Stopped);
    assert_eq!(rows[1].name, "first-in-file");
    assert_eq!(rows[1].tier, "db-g1-small");
}

// ---------------------------------------------------------------------------
// cloudsql::normalize_engine / parse_status / parse_availability
// ---------------------------------------------------------------------------

/// Every prefix arm answers a different product name, and the fallback answers
/// none of them.
#[test]
fn normalize_engine_names_each_family_and_falls_back_for_the_rest() {
    assert_eq!(normalize_engine("POSTGRES_15"), "PostgreSQL");
    assert_eq!(normalize_engine("MYSQL_8_0"), "MySQL");
    assert_eq!(normalize_engine("SQLSERVER_2019_STANDARD"), "SQL Server");
    assert_eq!(normalize_engine("ORACLE_19C"), "Unknown");
    assert_eq!(normalize_engine(""), "Unknown");
}

/// The uppercase normalisation runs before the prefix tests, so a lowercase
/// version string still resolves. Kills dropping the `to_uppercase()` call.
#[test]
fn normalize_engine_is_case_insensitive() {
    assert_eq!(normalize_engine("postgres_15"), "PostgreSQL");
    assert_eq!(normalize_engine("mysql_8_0"), "MySQL");
    assert_eq!(normalize_engine("sqlserver_2019_standard"), "SQL Server");
}

/// Each state string maps to its own variant, both spellings of the failed
/// state included, and anything unrecognised is carried through verbatim.
#[test]
fn parse_status_covers_every_arm_including_the_verbatim_fallback() {
    assert_eq!(parse_status("RUNNABLE"), CloudSqlStatus::Runnable);
    assert_eq!(parse_status("STOPPED"), CloudSqlStatus::Stopped);
    assert_eq!(
        parse_status("PENDING_CREATE"),
        CloudSqlStatus::PendingCreate
    );
    assert_eq!(parse_status("MAINTENANCE"), CloudSqlStatus::Maintenance);
    assert_eq!(parse_status("FAILED"), CloudSqlStatus::Failed);
    assert_eq!(parse_status("UNKNOWN_STATE"), CloudSqlStatus::Failed);
    assert_eq!(parse_status("SUSPENDED"), CloudSqlStatus::Suspended);
    assert_eq!(
        parse_status("PENDING_DELETE"),
        CloudSqlStatus::Unknown("PENDING_DELETE".to_string())
    );
    assert_eq!(parse_status(""), CloudSqlStatus::Unknown(String::new()));
}

/// Both known availability strings differ from each other and from the
/// fallback, which also covers the absent case.
#[test]
fn parse_availability_separates_regional_zonal_and_the_unknown_fallback() {
    assert_eq!(
        parse_availability(Some("REGIONAL")),
        CloudSqlAvailability::Regional
    );
    assert_eq!(
        parse_availability(Some("ZONAL")),
        CloudSqlAvailability::Zonal
    );
    assert_eq!(
        parse_availability(Some("MULTI_ZONAL")),
        CloudSqlAvailability::Unknown
    );
    assert_eq!(parse_availability(None), CloudSqlAvailability::Unknown);
}

// ---------------------------------------------------------------------------
// gke::addons_from
// ---------------------------------------------------------------------------

/// An addon the cluster did not report at all yields no row — it is not the
/// same as a reported-off addon. Kills replacing the `filter_map` closure so
/// that `None` becomes a row.
#[test]
fn addons_from_omits_entries_the_cluster_did_not_report() {
    let rows = addons_from(&[("reported", Some(true)), ("absent", None)]);

    assert_eq!(
        rows,
        vec![GkeAddon {
            name: "reported".to_string(),
            enabled: true,
        }]
    );
}

/// `Some(true)` and `Some(false)` must reach the row's `enabled` field
/// unchanged, and the rows come back in the order the entries were handed in.
/// The alternating flags mean no single replacement of the mapped value
/// satisfies all three rows. The order is asserted by hand: cargo-mutants has
/// no reordering operator, so nothing but this equality pins it.
#[test]
fn addons_from_keeps_enabled_and_disabled_entries_apart_and_in_order() {
    let rows = addons_from(&[
        ("first-off", Some(false)),
        ("second-on", Some(true)),
        ("third-off", Some(false)),
    ]);

    assert_eq!(
        rows,
        vec![
            GkeAddon {
                name: "first-off".to_string(),
                enabled: false,
            },
            GkeAddon {
                name: "second-on".to_string(),
                enabled: true,
            },
            GkeAddon {
                name: "third-off".to_string(),
                enabled: false,
            },
        ]
    );
}

/// No entries and all-absent entries both yield no rows, so the empty result is
/// reachable and distinguishable from the populated one above.
#[test]
fn addons_from_yields_no_rows_when_nothing_was_reported() {
    assert_eq!(addons_from(&[]), Vec::<GkeAddon>::new());
    assert_eq!(
        addons_from(&[("a", None), ("b", None)]),
        Vec::<GkeAddon>::new()
    );
}

// ---------------------------------------------------------------------------
// gke::parse_cluster_detail_json — the addon inversion through the CLI
// ---------------------------------------------------------------------------

/// `httpLoadBalancing.disabled = true` must render as `enabled = false`, and
/// `disabled = false` as `enabled = true`. Kills the deletion of the `!` in
/// `map(|addon| !addon.disabled)` at both addons that carry the inverted flag,
/// while the CSI driver's already-positive `enabled` flag pins that it is *not*
/// inverted.
#[test]
fn cluster_detail_inverts_the_disabled_flag_and_passes_the_enabled_one_through() {
    let json = r#"{
      "name":"addon-mix","location":"europe-west1","status":"RUNNING",
      "addonsConfig":{
        "httpLoadBalancing":{"disabled":true},
        "horizontalPodAutoscaling":{"disabled":false},
        "gcePersistentDiskCsiDriverConfig":{"enabled":false}
      }
    }"#;

    let detail = parse_cluster_detail_json(json).expect("detail JSON should parse");

    assert_eq!(
        detail.addons,
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

/// The opposite flags on the same three addons, so no single replacement value
/// satisfies both this test and the one above.
#[test]
fn cluster_detail_reads_the_opposite_addon_flags_as_their_opposites() {
    let json = r#"{
      "name":"addon-mix-inverted","location":"europe-west1","status":"RUNNING",
      "addonsConfig":{
        "httpLoadBalancing":{"disabled":false},
        "horizontalPodAutoscaling":{"disabled":true},
        "gcePersistentDiskCsiDriverConfig":{"enabled":true}
      }
    }"#;

    let detail = parse_cluster_detail_json(json).expect("detail JSON should parse");

    assert_eq!(
        detail.addons,
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

/// An addon block that names only one addon yields exactly one row; a missing
/// `addonsConfig` yields none. The populated half kills the replacement of
/// `parse_addons`'s body with an empty `Vec`, and the absent half pins that the
/// empty answer is reachable rather than being that same mutant's output.
#[test]
fn cluster_detail_distinguishes_a_partial_addon_block_from_an_absent_one() {
    let partial = r#"{
      "name":"partial","location":"europe-west1","status":"RUNNING",
      "addonsConfig":{"horizontalPodAutoscaling":{"disabled":true}}
    }"#;
    let absent = r#"{"name":"absent","location":"europe-west1","status":"RUNNING"}"#;

    let partial = parse_cluster_detail_json(partial).expect("detail JSON should parse");
    let absent = parse_cluster_detail_json(absent).expect("detail JSON should parse");

    assert_eq!(
        partial.addons,
        vec![GkeAddon {
            name: HORIZONTAL_POD_AUTOSCALING.to_string(),
            enabled: false,
        }]
    );
    assert_eq!(absent.addons, Vec::<GkeAddon>::new());
}

/// Every field of the detail carries a distinct non-default value, so replacing
/// the parser body wholesale changes each of them. `network`/`subnetwork` are
/// tail-trimmed, `location` comes from `location` rather than `zone`, and the
/// two node pools differ in every column.
#[test]
fn cluster_detail_maps_every_field_distinctly() {
    let json = r#"{
      "name":"detail-full","location":"europe-west4","zone":"europe-west4-a",
      "status":"RECONCILING",
      "currentMasterVersion":"1.32.1-gke.100","currentNodeVersion":"1.31.9-gke.200",
      "endpoint":"35.1.2.3",
      "network":"https://www.googleapis.com/compute/v1/projects/p/global/networks/prod-vpc",
      "subnetwork":"https://www.googleapis.com/compute/v1/projects/p/regions/europe-west4/subnetworks/prod-subnet",
      "resourceLabels":{"team":"platform","env":"prod"},
      "privateClusterConfig":{"enablePrivateEndpoint":true},
      "nodePools":[
        {"name":"pool-a","status":"RUNNING","version":"1.31.9-gke.200","initialNodeCount":3,
         "config":{"machineType":"e2-standard-4"},
         "autoscaling":{"enabled":true,"minNodeCount":2,"maxNodeCount":7}},
        {"name":"pool-b","status":"ERROR","version":"1.30.4-gke.300","initialNodeCount":1,
         "config":{"machineType":"n2-standard-8"},
         "autoscaling":{"enabled":false,"minNodeCount":5,"maxNodeCount":9}}
      ]
    }"#;

    let detail = parse_cluster_detail_json(json).expect("detail JSON should parse");

    assert_eq!(detail.name, "detail-full");
    assert_eq!(detail.location, "europe-west4");
    assert_eq!(detail.status, GkeClusterStatus::Reconciling);
    assert_eq!(
        detail.current_master_version.as_deref(),
        Some("1.32.1-gke.100")
    );
    assert_eq!(
        detail.current_node_version.as_deref(),
        Some("1.31.9-gke.200")
    );
    assert_eq!(detail.endpoint.as_deref(), Some("35.1.2.3"));
    assert!(detail.private_cluster);
    assert_eq!(detail.network.as_deref(), Some("prod-vpc"));
    assert_eq!(detail.subnetwork.as_deref(), Some("prod-subnet"));
    assert_eq!(
        detail.labels,
        vec![
            ComputeKeyValue {
                key: "env".to_string(),
                value: "prod".to_string(),
            },
            ComputeKeyValue {
                key: "team".to_string(),
                value: "platform".to_string(),
            },
        ]
    );
    assert_eq!(
        detail.node_pools,
        vec![
            GkeNodePool {
                name: "pool-a".to_string(),
                status: GkeClusterStatus::Running,
                version: Some("1.31.9-gke.200".to_string()),
                machine_type: Some("e2-standard-4".to_string()),
                autoscaling: Some("2-7".to_string()),
                node_count: Some(3),
            },
            GkeNodePool {
                name: "pool-b".to_string(),
                status: GkeClusterStatus::Error,
                version: Some("1.30.4-gke.300".to_string()),
                machine_type: Some("n2-standard-8".to_string()),
                // Disabled autoscaling reads "off" even though the API still
                // reported a 5-9 range: the `!enabled` test decides, not the bounds.
                autoscaling: Some("off".to_string()),
                node_count: Some(1),
            },
        ]
    );
    assert!(detail.raw_json.contains("\"name\": \"detail-full\""));
}

/// With `location` absent the zone fills the cell; with both present location
/// wins. Kills swapping the two sides of the `.or()`.
#[test]
fn cluster_detail_falls_back_to_the_zone_only_when_location_is_absent() {
    let zonal = r#"{"name":"zonal","zone":"us-central1-c","status":"RUNNING"}"#;
    let neither = r#"{"name":"placeless","status":"RUNNING"}"#;

    assert_eq!(
        parse_cluster_detail_json(zonal)
            .expect("detail JSON should parse")
            .location,
        "us-central1-c"
    );
    assert_eq!(
        parse_cluster_detail_json(neither)
            .expect("detail JSON should parse")
            .location,
        ""
    );
}

/// A cluster that reported `enablePrivateEndpoint: false` is not private, and
/// neither is one that reported no `privateClusterConfig` at all. Kills
/// replacing the `|config| config.enable_private_endpoint` closure with `true`,
/// which the all-distinct detail above cannot reach on its own.
#[test]
fn cluster_detail_reads_a_reported_public_endpoint_as_not_private() {
    let reported = r#"{
      "name":"public-endpoint","location":"europe-west1","status":"RUNNING",
      "privateClusterConfig":{"enablePrivateEndpoint":false}
    }"#;
    let absent = r#"{"name":"no-config","location":"europe-west1","status":"RUNNING"}"#;

    assert!(
        !parse_cluster_detail_json(reported)
            .expect("detail JSON should parse")
            .private_cluster
    );
    assert!(
        !parse_cluster_detail_json(absent)
            .expect("detail JSON should parse")
            .private_cluster
    );
}

// ---------------------------------------------------------------------------
// gke::autoscaling_label / endpoint_visibility / parse_cluster_status / tail_segment
// ---------------------------------------------------------------------------

/// A disabled pool reads "off" even when the API reported bounds. Kills the
/// deletion of the `!` in `if !enabled`, which would render "3-9" here.
#[test]
fn autoscaling_label_reads_off_for_a_disabled_pool_with_bounds() {
    assert_eq!(autoscaling_label(false, Some(3), Some(9)), "off");
    assert_eq!(autoscaling_label(false, None, None), "off");
}

/// An enabled pool renders its bounds, and the two bounds are distinct so a
/// swap of `min` and `max` changes the string.
#[test]
fn autoscaling_label_renders_the_bounds_of_an_enabled_pool() {
    assert_eq!(autoscaling_label(true, Some(2), Some(7)), "2-7");
}

/// Absent bounds on an enabled pool default to zero rather than being omitted.
#[test]
fn autoscaling_label_defaults_absent_bounds_to_zero() {
    assert_eq!(autoscaling_label(true, None, None), "0-0");
    assert_eq!(autoscaling_label(true, Some(4), None), "4-0");
    assert_eq!(autoscaling_label(true, None, Some(8)), "0-8");
}

/// The private flag wins over a present endpoint; a present endpoint alone is
/// public; neither is unknown. Three distinct answers over the same function.
#[test]
fn endpoint_visibility_separates_private_public_and_unknown() {
    let endpoint = "35.1.2.3".to_string();

    assert_eq!(
        endpoint_visibility(Some(&endpoint), true),
        EndpointVisibility::Private
    );
    assert_eq!(endpoint_visibility(None, true), EndpointVisibility::Private);
    assert_eq!(
        endpoint_visibility(Some(&endpoint), false),
        EndpointVisibility::Public
    );
    assert_eq!(
        endpoint_visibility(None, false),
        EndpointVisibility::Unknown
    );
}

/// Every cluster status arm answers its own variant, and an unrecognised status
/// is carried through verbatim rather than collapsing into `Running`.
#[test]
fn parse_cluster_status_covers_every_arm_including_the_verbatim_fallback() {
    assert_eq!(parse_cluster_status("RUNNING"), GkeClusterStatus::Running);
    assert_eq!(
        parse_cluster_status("RECONCILING"),
        GkeClusterStatus::Reconciling
    );
    assert_eq!(parse_cluster_status("STOPPING"), GkeClusterStatus::Stopping);
    assert_eq!(parse_cluster_status("ERROR"), GkeClusterStatus::Error);
    assert_eq!(
        parse_cluster_status("DEGRADED"),
        GkeClusterStatus::Unknown("DEGRADED".to_string())
    );
    assert_eq!(
        parse_cluster_status(""),
        GkeClusterStatus::Unknown(String::new())
    );
}

/// The trailing segment of a resource path, and a value with no slash passing
/// through unchanged.
#[test]
fn tail_segment_keeps_only_the_last_path_segment() {
    assert_eq!(
        tail_segment("https://www.googleapis.com/compute/v1/projects/p/global/networks/prod-vpc"),
        "prod-vpc"
    );
    assert_eq!(tail_segment("projects/p/networks/shared"), "shared");
    assert_eq!(tail_segment("already-short"), "already-short");
    assert_eq!(tail_segment(""), "");
}

// ---------------------------------------------------------------------------
// secrets::replication_label_from
// ---------------------------------------------------------------------------

/// An automatic policy returns early, whatever locations were handed alongside
/// it. Kills replacing the `automatic` test with `false`, which would fall
/// through to the user-managed arms.
#[test]
fn replication_label_reads_automatic_whatever_locations_accompany_it() {
    assert_eq!(replication_label_from(true, None), "Automatic");
    assert_eq!(replication_label_from(true, Some(Vec::new())), "Automatic");
    assert_eq!(
        replication_label_from(true, Some(vec!["europe-west1".to_string()])),
        "Automatic"
    );
}

/// A user-managed policy with no replicas is "User-managed" with no
/// parenthesised list. Kills replacing the `locations.is_empty()` match guard
/// with `false`, which would render "User-managed ()".
#[test]
fn replication_label_omits_the_parenthesis_for_an_empty_location_list() {
    assert_eq!(
        replication_label_from(false, Some(Vec::new())),
        "User-managed"
    );
}

/// A user-managed policy names its replica locations, comma-separated, in the
/// order given. Kills replacing the same match guard with `true`, which would
/// drop the locations entirely.
#[test]
fn replication_label_names_the_replica_locations_in_order() {
    assert_eq!(
        replication_label_from(
            false,
            Some(vec!["europe-west1".to_string(), "us-east1".to_string()])
        ),
        "User-managed (europe-west1, us-east1)"
    );
    assert_eq!(
        replication_label_from(
            false,
            Some(vec!["us-east1".to_string(), "europe-west1".to_string()])
        ),
        "User-managed (us-east1, europe-west1)"
    );
    assert_eq!(
        replication_label_from(false, Some(vec!["asia-south1".to_string()])),
        "User-managed (asia-south1)"
    );
}

/// Neither policy shape reported: "Unknown", which must not be confused with an
/// empty user-managed list.
#[test]
fn replication_label_reads_unknown_when_no_policy_was_reported() {
    assert_eq!(replication_label_from(false, None), "Unknown");
}

// ---------------------------------------------------------------------------
// secrets::rotation_label_from / short_id / key_values / version_state
// ---------------------------------------------------------------------------

/// Each of the four (period, next) combinations renders its own wording, and
/// the all-absent case is `None` rather than an empty string.
#[test]
fn rotation_label_covers_every_combination_of_period_and_next_time() {
    assert_eq!(
        rotation_label_from(
            Some("2592000s".to_string()),
            Some("2026-01-01T00:00:00Z".to_string())
        ),
        Some("every 2592000s, next 2026-01-01T00:00:00Z".to_string())
    );
    assert_eq!(
        rotation_label_from(Some("2592000s".to_string()), None),
        Some("every 2592000s".to_string())
    );
    assert_eq!(
        rotation_label_from(None, Some("2026-01-01T00:00:00Z".to_string())),
        Some("next 2026-01-01T00:00:00Z".to_string())
    );
    assert_eq!(rotation_label_from(None, None), None);
}

/// The trailing segment of a secret or version path, and a bare name passing
/// through unchanged.
#[test]
fn short_id_keeps_only_the_last_path_segment() {
    assert_eq!(short_id("projects/p/secrets/api-key"), "api-key");
    assert_eq!(short_id("projects/p/secrets/api-key/versions/3"), "3");
    assert_eq!(short_id("bare-name"), "bare-name");
    assert_eq!(short_id(""), "");
}

/// Labels render as key/value pairs sorted by key, not in insertion order — so
/// a reload cannot reshuffle the pane.
#[test]
fn key_values_renders_pairs_sorted_by_key() {
    let mut map = std::collections::BTreeMap::new();
    map.insert("zone".to_string(), "europe-west1".to_string());
    map.insert("env".to_string(), "prod".to_string());
    map.insert("team".to_string(), "platform".to_string());

    assert_eq!(
        key_values(map),
        vec![
            ComputeKeyValue {
                key: "env".to_string(),
                value: "prod".to_string(),
            },
            ComputeKeyValue {
                key: "team".to_string(),
                value: "platform".to_string(),
            },
            ComputeKeyValue {
                key: "zone".to_string(),
                value: "europe-west1".to_string(),
            },
        ]
    );
    assert_eq!(
        key_values(std::collections::BTreeMap::new()),
        Vec::<ComputeKeyValue>::new()
    );
}

/// Every version state arm, including the two shapes of unknown: an
/// unrecognised name carried through verbatim, and an absent state as the empty
/// string. Neither may read as `Enabled`.
#[test]
fn version_state_covers_every_arm_and_never_defaults_to_enabled() {
    assert_eq!(version_state(Some("ENABLED")), SecretVersionState::Enabled);
    assert_eq!(
        version_state(Some("DISABLED")),
        SecretVersionState::Disabled
    );
    assert_eq!(
        version_state(Some("DESTROYED")),
        SecretVersionState::Destroyed
    );
    assert_eq!(
        version_state(Some("DESTROY_SCHEDULED")),
        SecretVersionState::Unknown("DESTROY_SCHEDULED".to_string())
    );
    assert_eq!(
        version_state(None),
        SecretVersionState::Unknown(String::new())
    );
}
