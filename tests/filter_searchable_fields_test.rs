//! Pins the searchable-field premise the server-side filter pushdown depends
//! on: every client-side searchable display value is either a verbatim
//! substring of a pushed raw field or literally enumerated in that service's
//! `OPAQUE_LABELS`. Rendering a status with `{:?}` broke that premise — the
//! `Unknown("REPAIRING")` Debug shell appears in no raw gcloud field and is
//! modelled by no opaque label, so a filter matching the shell but neither
//! `unknown` nor a raw field would be pushed down and drop the row.
//!
//! Written first (TDD RED) against the `{:?}` rendering.

use nuvem::app::state::resources::{CloudSql, Compute, Gke, ResourceService};
use nuvem_domain::gcp::filter::{CompiledFilter, compile_filter};
use nuvem_domain::gcp::resource::{
    CloudSqlAvailability, CloudSqlInstance, CloudSqlStatus, ComputeInstance, EndpointVisibility,
    GkeCluster, GkeClusterStatus, ResourceStatus,
};

/// Compiles a needle the way the `visible_*` accessors do. `CompiledFilter`
/// owns case-insensitivity, so the needles below need no lowercasing; the
/// Debug-shell fragments escape their parentheses so they are tested as the
/// literal text they are rather than as regex grouping.
fn compiled(input: &str) -> CompiledFilter {
    compile_filter(input).expect("test filter input must compile")
}

fn compute_row(status: ResourceStatus) -> ComputeInstance {
    ComputeInstance {
        id: "api-dev-1-id".into(),
        name: "api-dev-1".into(),
        zone: "europe-west1-b".into(),
        status,
        machine_type: "e2-medium".into(),
        internal_ip: Some("10.0.0.4".into()),
        external_ip: None,
    }
}

fn gke_row(status: GkeClusterStatus) -> GkeCluster {
    GkeCluster {
        name: "prod-cluster".into(),
        location: "europe-west1".into(),
        status,
        current_master_version: Some("1.30".into()),
        current_node_count: Some(3),
        endpoint_visibility: EndpointVisibility::Public,
    }
}

fn cloud_sql_row(status: CloudSqlStatus) -> CloudSqlInstance {
    CloudSqlInstance {
        name: "orders-db".into(),
        database_version: "POSTGRES_15".into(),
        engine: "PostgreSQL".into(),
        region: "europe-west1".into(),
        tier: "db-custom-2-7680".into(),
        availability: CloudSqlAvailability::Zonal,
        status,
        connectivity: EndpointVisibility::Private,
    }
}

#[test]
fn compute_unknown_status_exposes_no_debug_shell() {
    let row = compute_row(ResourceStatus::Unknown("REPAIRING".into()));
    for shell in [
        r"unknown\(",
        r#"own\(""#,
        r#""\)"#,
        r#"unknown\("repairing"\)"#,
    ] {
        assert!(
            !Compute::matches_filter(&row, &compiled(shell)),
            "searchable fields must not carry the Debug shell fragment {shell:?}"
        );
    }
}

#[test]
fn compute_unknown_status_still_matches_label_and_raw_value() {
    let row = compute_row(ResourceStatus::Unknown("REPAIRING".into()));
    assert!(
        Compute::matches_filter(&row, &compiled("^repairing$")),
        "the display label, `Repairing`, must stay searchable as painted"
    );
    assert!(
        Compute::matches_filter(&row, &compiled("repairing")),
        "the raw status carried by Unknown must stay searchable"
    );
}

#[test]
fn compute_known_status_matches_its_display_label() {
    let row = compute_row(ResourceStatus::Running);
    assert!(Compute::matches_filter(&row, &compiled("running")));
    assert!(!Compute::matches_filter(&row, &compiled("stopped")));
}

#[test]
fn gke_unknown_status_exposes_no_debug_shell() {
    let row = gke_row(GkeClusterStatus::Unknown("DEGRADED".into()));
    for shell in [
        r"unknown\(",
        r#"own\(""#,
        r#""\)"#,
        r#"unknown\("degraded"\)"#,
    ] {
        assert!(
            !Gke::matches_filter(&row, &compiled(shell)),
            "searchable fields must not carry the Debug shell fragment {shell:?}"
        );
    }
}

#[test]
fn gke_unknown_status_still_matches_label_and_raw_value() {
    let row = gke_row(GkeClusterStatus::Unknown("DEGRADED".into()));
    assert!(
        Gke::matches_filter(&row, &compiled("^degraded$")),
        "the display label, `Degraded`, must stay searchable as painted"
    );
    assert!(
        Gke::matches_filter(&row, &compiled("degraded")),
        "the raw status carried by Unknown must stay searchable"
    );
}

#[test]
fn gke_known_status_matches_its_display_label() {
    let row = gke_row(GkeClusterStatus::Reconciling);
    assert!(Gke::matches_filter(&row, &compiled("reconciling")));
    assert!(!Gke::matches_filter(&row, &compiled("error")));
}

#[test]
fn cloud_sql_unknown_status_exposes_no_debug_shell() {
    let row = cloud_sql_row(CloudSqlStatus::Unknown("SUSPENDED_BY_BILLING".into()));
    for shell in [
        r"unknown\(",
        r#"own\(""#,
        r#""\)"#,
        r#"unknown\("suspended_by_billing"\)"#,
    ] {
        assert!(
            !CloudSql::matches_filter(&row, &compiled(shell)),
            "searchable fields must not carry the Debug shell fragment {shell:?}"
        );
    }
}

#[test]
fn cloud_sql_unknown_status_still_matches_label_and_raw_value() {
    let row = cloud_sql_row(CloudSqlStatus::Unknown("SUSPENDED_BY_BILLING".into()));
    assert!(
        CloudSql::matches_filter(&row, &compiled("by billing")),
        "the display label, `Suspended by billing`, must stay searchable as painted"
    );
    assert!(
        CloudSql::matches_filter(&row, &compiled("suspended_by_billing")),
        "the raw status carried by Unknown must stay searchable"
    );
}

#[test]
fn cloud_sql_known_status_matches_its_display_label() {
    let row = cloud_sql_row(CloudSqlStatus::Runnable);
    assert!(CloudSql::matches_filter(&row, &compiled("runnable")));
    assert!(!CloudSql::matches_filter(&row, &compiled("stopped")));
}
