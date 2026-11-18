use nuvem::services::resources::cloudsql::{parse_instance_detail_json, parse_users_json};

const DETAIL: &str = include_str!("fixtures/cloudsql_instance_detail.json");
const USERS: &str = include_str!("fixtures/cloudsql_users.json");

#[test]
fn parses_detail_fields() {
    let d = parse_instance_detail_json(DETAIL).unwrap();
    assert_eq!(d.name, "mysql-ha");
    assert!(d.backup.enabled);
    assert_eq!(d.backup.transaction_log_retention_days, Some(7));
    assert_eq!(d.maintenance_window.as_ref().unwrap().day, Some(7));
    assert_eq!(d.replicas, vec!["mysql-ha-replica".to_string()]);
    assert!(d.users.is_empty()); // lazy-loaded separately
}

#[test]
fn raw_json_redacts_cert_and_password() {
    let d = parse_instance_detail_json(DETAIL).unwrap();
    assert!(
        !d.raw_json.contains("SECRETPEM"),
        "cert PEM leaked into raw_json"
    );
    assert!(!d.raw_json.contains("BEGIN CERTIFICATE"), "cert PEM leaked");
    assert!(
        !d.raw_json.contains("SUPERSECRET"),
        "password leaked into raw_json"
    );
    assert!(d.raw_json.contains("<redacted>"));
}

#[test]
fn raw_json_redacts_secrets_nested_in_arrays() {
    let d = parse_instance_detail_json(DETAIL).unwrap();
    assert!(
        !d.raw_json.contains("ARRAYSECRETPEM"),
        "cert PEM nested in array leaked into raw_json"
    );
    assert!(
        !d.raw_json.contains("ARRAYSECRETKEY"),
        "clientKey nested in array leaked into raw_json"
    );
}

#[test]
fn parses_users_without_secrets() {
    let users = parse_users_json(USERS).unwrap();
    assert_eq!(users.len(), 2);
    assert_eq!(users[0].name, "postgres");
    // struct has no password field at all — compile-time guarantee
}
