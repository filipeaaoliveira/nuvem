use nuvem_domain::gcp::logging::{LogQuery, LogResourceKind};

#[test]
fn project_logs_render_empty_filter() {
    assert_eq!(LogQuery::ProjectLogs.render_filter(), "");
    assert_eq!(LogQuery::ProjectLogs.summary(), "project logs");
}

#[test]
fn compute_instance_filter_targets_concrete_identity() {
    let query = LogQuery::Resource(LogResourceKind::ComputeInstance {
        instance_id: "1001".to_string(),
        instance_name: "api-dev-1".to_string(),
        zone: "europe-west1-b".to_string(),
    });
    assert_eq!(
        query.render_filter(),
        r#"resource.type="gce_instance" AND resource.labels.instance_id="1001""#
    );
    assert!(query.summary().contains("api-dev-1"));
}

#[test]
fn gke_cluster_filter_targets_concrete_identity() {
    let query = LogQuery::Resource(LogResourceKind::GkeCluster {
        cluster_name: "prod".to_string(),
        location: "us-central1".to_string(),
        project: "dev-project".to_string(),
    });
    assert_eq!(
        query.render_filter(),
        r#"resource.type="k8s_cluster" AND resource.labels.cluster_name="prod" AND resource.labels.location="us-central1""#
    );
}

#[test]
fn cloud_sql_instance_filter_targets_concrete_identity() {
    let query = LogQuery::Resource(LogResourceKind::CloudSqlInstance {
        instance_name: "orders-db".to_string(),
        project: "dev-project".to_string(),
    });
    assert_eq!(
        query.render_filter(),
        r#"resource.type="cloudsql_database" AND resource.labels.database_id="dev-project:orders-db""#
    );
}

#[test]
fn storage_bucket_filter_targets_concrete_identity() {
    let query = LogQuery::Resource(LogResourceKind::StorageBucket {
        bucket_name: "assets-dev".to_string(),
    });
    assert_eq!(
        query.render_filter(),
        r#"resource.type="gcs_bucket" AND resource.labels.bucket_name="assets-dev""#
    );
}

#[test]
fn cloud_run_revision_filter_targets_concrete_identity() {
    let query = LogQuery::Resource(LogResourceKind::CloudRunRevision {
        service_name: "orders-api".to_string(),
        revision_name: "orders-api-00007-abc".to_string(),
        region: "europe-west1".to_string(),
        project: "dev-project".to_string(),
    });
    assert_eq!(
        query.render_filter(),
        r#"resource.type="cloud_run_revision" AND resource.labels.service_name="orders-api" AND resource.labels.revision_name="orders-api-00007-abc" AND resource.labels.location="europe-west1""#
    );
}

#[test]
fn cloud_sql_instance_filter_escapes_quotes_and_backslashes() {
    let query = LogQuery::Resource(LogResourceKind::CloudSqlInstance {
        instance_name: r#"a"b\c"#.to_string(),
        project: r#"p"1\2"#.to_string(),
    });
    assert_eq!(
        query.render_filter(),
        r#"resource.type="cloudsql_database" AND resource.labels.database_id="p\"1\\2:a\"b\\c""#
    );
}

#[test]
fn storage_bucket_filter_escapes_quotes_and_backslashes() {
    let query = LogQuery::Resource(LogResourceKind::StorageBucket {
        bucket_name: r#"a"b\c"#.to_string(),
    });
    assert_eq!(
        query.render_filter(),
        r#"resource.type="gcs_bucket" AND resource.labels.bucket_name="a\"b\\c""#
    );
}

#[test]
fn cloud_run_revision_filter_escapes_quotes_and_backslashes() {
    let query = LogQuery::Resource(LogResourceKind::CloudRunRevision {
        service_name: r#"s"1"#.to_string(),
        revision_name: r#"r\2"#.to_string(),
        region: r#"z"3\4"#.to_string(),
        project: "dev-project".to_string(),
    });
    assert_eq!(
        query.render_filter(),
        r#"resource.type="cloud_run_revision" AND resource.labels.service_name="s\"1" AND resource.labels.revision_name="r\\2" AND resource.labels.location="z\"3\\4""#
    );
}

#[test]
fn filter_escapes_quotes_and_backslashes() {
    let query = LogQuery::Resource(LogResourceKind::ComputeInstance {
        instance_id: r#"a"b\c"#.to_string(),
        instance_name: "n".to_string(),
        zone: "z".to_string(),
    });
    assert_eq!(
        query.render_filter(),
        r#"resource.type="gce_instance" AND resource.labels.instance_id="a\"b\\c""#
    );
}
