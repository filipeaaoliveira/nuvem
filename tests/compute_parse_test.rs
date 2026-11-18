use nuvem::services::resources::compute::parse_instances_json;
use nuvem_domain::gcp::resource::ResourceStatus;

#[test]
fn parses_gce_instance_rows() {
    let json = include_str!("fixtures/gce_instances.json");
    let rows = parse_instances_json(json).expect("fixture should parse");

    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].name, "api-dev-1");
    assert_eq!(rows[0].zone, "europe-west1-b");
    assert_eq!(rows[0].machine_type, "e2-medium");
    assert_eq!(rows[0].internal_ip.as_deref(), Some("10.0.0.4"));
    assert_eq!(rows[0].external_ip.as_deref(), Some("34.10.10.10"));
    assert_eq!(rows[0].status, ResourceStatus::Running);
    assert_eq!(rows[1].status, ResourceStatus::Stopped);
}

#[test]
fn parse_instances_recognizes_suspended_instances() {
    let rows = parse_instances_json(
        r#"[{"id":"1","name":"paused","zone":"zones/europe-west1-b","status":"SUSPENDED","machineType":"zones/europe-west1-b/machineTypes/e2-medium"}]"#,
    )
    .expect("suspended instance should parse");

    assert_eq!(rows[0].status, ResourceStatus::Suspended);
}

#[test]
fn parse_instances_keeps_suspend_and_resume_transitions_pending() {
    for status in ["SUSPENDING", "RESUMING"] {
        let rows = parse_instances_json(&format!(
            r#"[{{"id":"1","name":"transitioning","zone":"zones/europe-west1-b","status":"{status}","machineType":"zones/europe-west1-b/machineTypes/e2-medium"}}]"#
        ))
        .expect("transitioning instance should parse");

        assert_eq!(rows[0].status, ResourceStatus::Pending, "status={status}");
    }
}
