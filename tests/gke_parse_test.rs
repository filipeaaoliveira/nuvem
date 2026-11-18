use nuvem::services::resources::gke::{parse_cluster_detail_json, parse_clusters_json};
use nuvem_domain::gcp::resource::{EndpointVisibility, GkeClusterStatus};

#[test]
fn parses_gke_cluster_rows() {
    let rows = parse_clusters_json(include_str!("fixtures/gke_clusters.json"))
        .expect("cluster fixture should parse");

    assert_eq!(rows.len(), 3);
    assert_eq!(rows[0].name, "platform-dev");
    assert_eq!(rows[0].location, "europe-west1");
    assert_eq!(rows[0].status, GkeClusterStatus::Running);
    assert_eq!(
        rows[0].current_master_version.as_deref(),
        Some("1.31.5-gke.1023000")
    );
    assert_eq!(rows[0].current_node_count, Some(6));
    assert_eq!(rows[0].endpoint_visibility, EndpointVisibility::Public);
    assert_eq!(rows[1].location, "europe-west1-b");
    assert_eq!(rows[1].endpoint_visibility, EndpointVisibility::Private);
    assert_eq!(
        rows[2].status,
        GkeClusterStatus::Unknown("MAINTENANCE_WINDOW".to_string())
    );
    assert_eq!(rows[2].endpoint_visibility, EndpointVisibility::Unknown);
}

#[test]
fn parses_gke_cluster_detail_tabs() {
    let detail = parse_cluster_detail_json(include_str!("fixtures/gke_cluster_detail.json"))
        .expect("detail fixture should parse");

    assert_eq!(detail.name, "platform-dev");
    assert_eq!(detail.location, "europe-west1");
    assert_eq!(detail.status, GkeClusterStatus::Running);
    assert_eq!(
        detail.current_master_version.as_deref(),
        Some("1.31.5-gke.1023000")
    );
    assert_eq!(
        detail.current_node_version.as_deref(),
        Some("1.31.5-gke.1023000")
    );
    assert_eq!(detail.endpoint.as_deref(), Some("34.118.10.20"));
    assert!(!detail.private_cluster);
    assert_eq!(detail.network.as_deref(), Some("shared-vpc"));
    assert_eq!(detail.subnetwork.as_deref(), Some("gke-subnet"));
    assert_eq!(detail.node_pools.len(), 2);
    assert_eq!(detail.node_pools[0].name, "default-pool");
    assert_eq!(
        detail.node_pools[0].machine_type.as_deref(),
        Some("e2-standard-4")
    );
    assert_eq!(detail.node_pools[0].autoscaling.as_deref(), Some("1-6"));
    assert_eq!(detail.node_pools[1].autoscaling.as_deref(), Some("off"));
    assert!(
        detail
            .addons
            .iter()
            .any(|addon| addon.name == "HTTP load balancing" && addon.enabled)
    );
    assert!(
        detail
            .labels
            .iter()
            .any(|entry| entry.key == "team" && entry.value == "platform")
    );
    assert!(detail.raw_json.contains("\"name\": \"platform-dev\""));
}

#[test]
fn parses_gke_missing_optional_fields_without_failure() {
    let rows =
        parse_clusters_json(r#"[{"name":"minimal","location":"us-central1","status":"RUNNING"}]"#)
            .expect("minimal row should parse");

    assert_eq!(rows[0].name, "minimal");
    assert_eq!(rows[0].current_master_version, None);
    assert_eq!(rows[0].current_node_count, None);
    assert_eq!(rows[0].endpoint_visibility, EndpointVisibility::Unknown);
}
