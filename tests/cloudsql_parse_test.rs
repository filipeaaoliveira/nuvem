use nuvem::services::resources::cloudsql::{parse_instances_json, parse_status};
use nuvem_domain::gcp::resource::{CloudSqlAvailability, CloudSqlStatus, EndpointVisibility};

const FIXTURE: &str = include_str!("fixtures/cloudsql_instances.json");

#[test]
fn parses_all_states_and_normalizes() {
    let rows = parse_instances_json(FIXTURE).unwrap();
    assert_eq!(rows.len(), 5);
    let ha = rows.iter().find(|r| r.name == "mysql-ha").unwrap();
    assert_eq!(ha.engine, "MySQL");
    assert_eq!(ha.availability, CloudSqlAvailability::Regional);
    assert_eq!(ha.status, CloudSqlStatus::Runnable);
    assert_eq!(ha.connectivity, EndpointVisibility::Private);
    let stopped = rows.iter().find(|r| r.name == "pg-stopped").unwrap();
    assert_eq!(stopped.status, CloudSqlStatus::Stopped);
    let maint = rows.iter().find(|r| r.name == "sqlserver-maint").unwrap();
    assert_eq!(maint.status, CloudSqlStatus::Maintenance);
    assert_eq!(maint.engine, "SQL Server");
    let standalone = rows.iter().find(|r| r.name == "pg-standalone").unwrap();
    assert_eq!(standalone.engine, "PostgreSQL");
    assert_eq!(standalone.connectivity, EndpointVisibility::Public);
    assert_eq!(standalone.tier, "db-custom-2-7680");
    assert_eq!(standalone.availability, CloudSqlAvailability::Zonal);
    // Replica fixture row (raw JSON carries masterInstanceName); confirms
    // replicas parse cleanly alongside standalone/HA/stopped/maintenance rows.
    let replica = rows.iter().find(|r| r.name == "pg-replica").unwrap();
    assert_eq!(replica.engine, "PostgreSQL");
    assert_eq!(replica.status, CloudSqlStatus::Runnable);
    assert_eq!(replica.availability, CloudSqlAvailability::Zonal);
}

#[test]
fn parse_status_normalizes_all_known_states() {
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
        parse_status("SOME_OTHER_STATE"),
        CloudSqlStatus::Unknown("SOME_OTHER_STATE".to_string())
    );
}
