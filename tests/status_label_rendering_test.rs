//! Statuses a real API returns that this build has no variant for.
//!
//! The detail panes of the four services whose status enums carry an
//! `Unknown(raw)` arm used to format it with `{:?}`, so a cluster in
//! `PROVISIONING` read `Status: Unknown("PROVISIONING")`. Each capture here is
//! re-parsed with its status replaced by a value no build knows, and the pane
//! must paint it as a label.

use nuvem::app::projection::detail_body::DetailBody;
use nuvem::app::projection::detail_body::{
    cloud_run_revision_detail_body, cloud_sql_detail_body, compute_detail_body, gke_detail_body,
};
use nuvem::app::state::resources::{
    CloudRunRevisionTab, CloudSqlDetailTab, ComputeDetailTab, GkeDetailTab,
};
use nuvem::services::resources::{cloud_run, cloudsql, compute, gke};

const NEW_VALUE: &str = "MAINTENANCE_WINDOW";
const LABEL: &str = "Maintenance window";

fn texts(body: &DetailBody) -> Vec<String> {
    body.lines.iter().map(|line| line.text.clone()).collect()
}

fn assert_readable(what: &str, lines: Vec<String>, field: &str) {
    let expected = format!("{field}: {LABEL}");
    assert!(
        lines.iter().any(|line| line.trim() == expected),
        "{what}: expected `{expected}` among {lines:#?}"
    );
    assert!(
        !lines
            .iter()
            .any(|line| line.contains("Unknown(") || line.contains(NEW_VALUE)),
        "{what}: the Rust value or the raw constant leaked: {lines:#?}"
    );
}

#[test]
fn every_detail_pane_paints_an_unrecognised_status_as_a_label() {
    let compute = compute::parse_instance_detail_json(
        &include_str!("fixtures/gce_instance_detail.json")
            .replace("\"RUNNING\"", "\"MAINTENANCE_WINDOW\""),
    )
    .expect("parses");
    assert_readable(
        "GCE instance",
        texts(&compute_detail_body(&compute, ComputeDetailTab::Summary)),
        "Status",
    );

    let gke = gke::parse_cluster_detail_json(
        &include_str!("fixtures/gke_cluster_detail.json")
            .replace("\"RUNNING\"", "\"MAINTENANCE_WINDOW\""),
    )
    .expect("parses");
    assert_readable(
        "GKE cluster",
        texts(&gke_detail_body(&gke, GkeDetailTab::Summary)),
        "Status",
    );
    assert_readable(
        "GKE node pool",
        texts(&gke_detail_body(&gke, GkeDetailTab::NodePools)),
        "Status",
    );

    let sql = cloudsql::parse_instance_detail_json(
        &include_str!("fixtures/cloudsql_instance_detail.json")
            .replace("\"RUNNABLE\"", "\"MAINTENANCE_WINDOW\""),
    )
    .expect("parses");
    assert_readable(
        "Cloud SQL instance",
        texts(&cloud_sql_detail_body(&sql, CloudSqlDetailTab::Summary)),
        "Status",
    );

    let mut revision = cloud_run::parse_revision_detail_json(include_str!(
        "fixtures/cloud_run_revision_detail.json"
    ))
    .expect("parses");
    revision.ready = nuvem_domain::gcp::resource::CloudRunServiceStatus::Unknown(NEW_VALUE.into());
    assert_readable(
        "Cloud Run revision",
        texts(&cloud_run_revision_detail_body(
            &revision,
            CloudRunRevisionTab::Summary,
        )),
        "Ready",
    );
}
