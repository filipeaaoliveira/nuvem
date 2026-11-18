use nuvem::services::resources::cloud_run::{
    parse_revision_detail_json, parse_revisions_json, parse_service_detail_json,
    parse_services_json,
};
use nuvem_domain::gcp::resource::CloudRunServiceStatus;

#[test]
fn parses_cloud_run_service_rows() {
    let rows = parse_services_json(include_str!("fixtures/cloud_run_services.json"))
        .expect("services fixture should parse");

    assert_eq!(rows.len(), 5);

    let orders = &rows[0];
    assert_eq!(orders.name, "orders-api");
    assert_eq!(orders.region, "europe-west1");
    assert_eq!(
        orders.url.as_deref(),
        Some("https://orders-api-abcde-ew.a.run.app")
    );
    assert_eq!(orders.status, CloudRunServiceStatus::Ready);
    assert_eq!(
        orders.latest_ready_revision.as_deref(),
        Some("orders-api-00007-tabc")
    );
    assert_eq!(orders.traffic_summary, "orders-api-00007-tabc 100%");
}

#[test]
fn normalizes_mixed_health_conditions() {
    let rows = parse_services_json(include_str!("fixtures/cloud_run_services.json"))
        .expect("services fixture should parse");

    let billing = rows.iter().find(|s| s.name == "billing-worker").unwrap();
    assert_eq!(billing.status, CloudRunServiceStatus::NotReady);
    assert_eq!(
        billing.ready_message.as_deref(),
        Some("Revision billing-worker-00004-zxcv is not ready.")
    );

    let checkout = rows.iter().find(|s| s.name == "checkout-api").unwrap();
    assert_eq!(checkout.status, CloudRunServiceStatus::Failed);

    let payments = rows.iter().find(|s| s.name == "payments-api").unwrap();
    assert_eq!(payments.status, CloudRunServiceStatus::Reconciling);
    // Highest-percent traffic target wins the summary.
    assert_eq!(payments.traffic_summary, "payments-api-00019-old0 80%");
}

#[test]
fn tolerates_missing_optional_fields_and_unknown_status() {
    let rows = parse_services_json(include_str!("fixtures/cloud_run_services.json"))
        .expect("services fixture should parse");

    let legacy = rows.iter().find(|s| s.name == "legacy-frontend").unwrap();
    assert_eq!(legacy.region, "asia-northeast1");
    assert_eq!(legacy.url, None);
    assert_eq!(legacy.latest_ready_revision, None);
    assert_eq!(legacy.traffic_summary, "-");
    assert!(matches!(legacy.status, CloudRunServiceStatus::Unknown(_)));
}

#[test]
fn tolerates_service_without_location_label() {
    let rows = parse_services_json(
        r#"[{"metadata":{"name":"orphan"},"status":{"conditions":[{"type":"Ready","status":"True"}]}}]"#,
    )
    .expect("minimal service should parse");

    assert_eq!(rows[0].name, "orphan");
    assert_eq!(rows[0].region, "");
    assert_eq!(rows[0].status, CloudRunServiceStatus::Ready);
    assert_eq!(rows[0].traffic_summary, "-");
}

#[test]
fn parses_cloud_run_service_detail() {
    let detail = parse_service_detail_json(include_str!("fixtures/cloud_run_service_detail.json"))
        .expect("detail fixture should parse");

    assert_eq!(detail.name, "orders-api");
    assert_eq!(detail.region, "europe-west1");
    assert_eq!(detail.status, CloudRunServiceStatus::Ready);
    assert_eq!(
        detail.url.as_deref(),
        Some("https://orders-api-abcde-ew.a.run.app")
    );
    assert_eq!(
        detail.image.as_deref(),
        Some("europe-docker.pkg.dev/example/apps/orders-api:1.4.2")
    );
    assert_eq!(detail.cpu.as_deref(), Some("1000m"));
    assert_eq!(detail.memory.as_deref(), Some("512Mi"));
    assert_eq!(detail.concurrency, Some(80));
    assert_eq!(
        detail.service_account.as_deref(),
        Some("orders-api@example.iam.gserviceaccount.com")
    );
    assert_eq!(detail.ingress.as_deref(), Some("all"));
    assert_eq!(detail.min_instances, Some(2));
    assert_eq!(detail.max_instances, Some(20));
    assert_eq!(detail.traffic.len(), 2);
    assert_eq!(detail.conditions.len(), 3);
    assert!(
        detail
            .labels
            .iter()
            .any(|kv| kv.key == "team" && kv.value == "commerce")
    );
    assert!(detail.raw_json.contains("\"name\": \"orders-api\""));
}

#[test]
fn parses_cloud_run_revisions_with_per_revision_status() {
    let revisions = parse_revisions_json(include_str!("fixtures/cloud_run_revisions.json"))
        .expect("revisions fixture should parse");

    assert_eq!(revisions.len(), 2);
    assert_eq!(revisions[0].name, "orders-api-00007-tabc");
    assert_eq!(revisions[0].ready, CloudRunServiceStatus::Ready);
    assert_eq!(
        revisions[0].image.as_deref(),
        Some("europe-docker.pkg.dev/example/apps/orders-api:1.4.2")
    );
    assert_eq!(
        revisions[0].created.as_deref(),
        Some("2026-07-18T09:11:58Z")
    );
    assert_eq!(revisions[1].name, "orders-api-00006-sxyz");
    assert_eq!(revisions[1].ready, CloudRunServiceStatus::Failed);
}

#[test]
fn parses_cloud_run_revision_detail() {
    let detail =
        parse_revision_detail_json(include_str!("fixtures/cloud_run_revision_detail.json"))
            .expect("revision detail fixture should parse");

    assert_eq!(detail.name, "orders-api-00007-tabc");
    assert_eq!(detail.service.as_deref(), Some("orders-api"));
    assert_eq!(detail.region, "europe-west1");
    assert_eq!(detail.ready, CloudRunServiceStatus::Ready);
    assert_eq!(
        detail.image.as_deref(),
        Some("europe-docker.pkg.dev/example/apps/orders-api:1.4.2")
    );
    assert_eq!(detail.cpu.as_deref(), Some("1000m"));
    assert_eq!(detail.memory.as_deref(), Some("512Mi"));
    assert_eq!(detail.concurrency, Some(80));
    assert_eq!(detail.min_instances, Some(2));
    assert_eq!(detail.max_instances, Some(20));
    assert_eq!(
        detail.service_account.as_deref(),
        Some("orders-api@example.iam.gserviceaccount.com")
    );
    assert_eq!(detail.created.as_deref(), Some("2026-07-18T09:11:58Z"));
    assert_eq!(detail.conditions.len(), 4);
    assert!(!detail.raw_json.is_empty());
}

#[test]
fn surfaces_parse_errors_on_invalid_json() {
    let error = parse_services_json("{not-json").expect_err("invalid json should error");
    assert!(error.to_string().contains("invalid Cloud Run"));
}
