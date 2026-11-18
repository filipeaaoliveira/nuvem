use nuvem::services::gcloud::configurations::parse_configurations_json;
use nuvem_domain::gcp::configuration::GcloudConfigurationName;

#[test]
fn parses_native_gcloud_configurations() {
    let json = include_str!("fixtures/gcloud_configurations.json");
    let configs = parse_configurations_json(json).expect("fixture should parse");

    assert_eq!(configs.len(), 2);
    assert_eq!(configs[0].name, GcloudConfigurationName::new("dev"));
    assert!(configs[0].is_active);
    assert_eq!(configs[0].account.as_deref(), Some("dev@example.com"));
    assert_eq!(configs[0].project.as_deref(), Some("dev-project"));
    assert_eq!(configs[0].region.as_deref(), Some("europe-west1"));
    assert_eq!(configs[0].zone.as_deref(), Some("europe-west1-b"));
    assert_eq!(configs[1].project.as_deref(), Some("prod-project"));
}

#[test]
fn parses_flat_account_and_project_fields() {
    let json = r#"
[
  {
    "name": "dev",
    "is_active": true,
    "account": "dev@example.com",
    "project": "dev-project"
  }
]
"#;

    let configs = parse_configurations_json(json).expect("fixture should parse");

    assert_eq!(configs.len(), 1);
    assert_eq!(configs[0].name, GcloudConfigurationName::new("dev"));
    assert!(configs[0].is_active);
    assert_eq!(configs[0].account.as_deref(), Some("dev@example.com"));
    assert_eq!(configs[0].project.as_deref(), Some("dev-project"));
}
