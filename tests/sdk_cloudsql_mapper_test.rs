//! The Cloud SQL mappers on the native read path: `map_instance`,
//! `map_instance_detail` and `map_user`. A mutation sweep reported all three
//! as untested — nothing built a `DatabaseInstance` or a `User` and asserted
//! what came out, so a body replaced wholesale, a dropped `!`, a swapped `==`
//! or an inverted `||` all survived.
//!
//! Every field is therefore planted with a distinct non-default value and
//! asserted exactly, and every branch (private vs ipv4 vs primary address,
//! present vs absent settings, empty vs set proto strings, each status and
//! engine arm including the unrecognised one) is exercised from both sides
//! with inputs that produce different rows.

use google_cloud_sql_v1::model::database_instance::SqlInstanceState;
use google_cloud_sql_v1::model::settings::SqlActivationPolicy;
use google_cloud_sql_v1::model::{
    BackupConfiguration, DatabaseInstance, IpConfiguration, IpMapping, MaintenanceWindow, Settings,
    SqlAvailabilityType, SqlDatabaseVersion, SqlIpAddressType, SqlUpdateTrack, User,
};
use nuvem::services::sdk::cloudsql::{map_instance, map_instance_detail, map_user};
use nuvem_domain::gcp::resource::{
    CloudSqlAvailability, CloudSqlBackupConfig, CloudSqlInstance, CloudSqlMaintenanceWindow,
    CloudSqlStatus, CloudSqlUser, ComputeKeyValue, EndpointVisibility,
};

/// The epoch second `sdk_helpers_test` pins to `2024-01-02T03:04:05Z`.
const CREATED_AT_SECONDS: i64 = 1_704_164_645;

fn ip(kind: SqlIpAddressType, address: &str) -> IpMapping {
    IpMapping::new().set_type(kind).set_ip_address(address)
}

/// An instance with every field the detail mapper reads planted with a
/// distinct, non-default value, so a body replaced by `Default::default()`
/// or a closure replaced by `None` cannot produce this row.
fn fully_planted_instance() -> DatabaseInstance {
    DatabaseInstance::new()
        .set_name("planted-instance")
        .set_region("europe-west4")
        .set_state("MAINTENANCE")
        .set_database_version(SqlDatabaseVersion::Postgres15)
        .set_connection_name("planted-project:europe-west4:planted-instance")
        .set_maintenance_version("POSTGRES_15_4.R20240101.00_00")
        .set_master_instance_name("planted-project:europe-west4:planted-primary")
        .set_replica_names(["planted-replica-a", "planted-replica-b"])
        .set_create_time(
            google_cloud_wkt::Timestamp::new(CREATED_AT_SECONDS, 0).expect("a valid timestamp"),
        )
        .set_ip_addresses([
            ip(SqlIpAddressType::Primary, "34.90.1.2"),
            ip(SqlIpAddressType::Outgoing, "34.90.9.9"),
            ip(SqlIpAddressType::Private, "10.24.0.7"),
        ])
        .set_settings(
            Settings::new()
                .set_tier("db-custom-4-16384")
                .set_availability_type(SqlAvailabilityType::Regional)
                .set_activation_policy(SqlActivationPolicy::Always)
                .set_user_labels([("env", "prod"), ("team", "data-platform")])
                .set_ip_configuration(
                    IpConfiguration::new()
                        .set_ipv4_enabled(true)
                        .set_require_ssl(true)
                        .set_private_network(
                            "projects/planted-project/global/networks/planted-vpc",
                        ),
                )
                .set_maintenance_window(
                    MaintenanceWindow::new()
                        .set_day(3)
                        .set_hour(23)
                        .set_update_track(SqlUpdateTrack::Week5),
                )
                .set_backup_configuration(
                    BackupConfiguration::new()
                        .set_enabled(true)
                        .set_start_time("04:30")
                        .set_location("eu")
                        .set_point_in_time_recovery_enabled(true)
                        .set_transaction_log_retention_days(7),
                ),
        )
}

fn sorted_labels(labels: &[ComputeKeyValue]) -> Vec<(String, String)> {
    let mut pairs: Vec<(String, String)> = labels
        .iter()
        .map(|label| (label.key.clone(), label.value.clone()))
        .collect();
    pairs.sort();
    pairs
}

// ---------------------------------------------------------------------------
// map_instance
// ---------------------------------------------------------------------------

#[test]
fn every_row_field_comes_from_its_own_planted_value() {
    let instance =
        DatabaseInstance::new()
            .set_name("planted-instance")
            .set_region("europe-west4")
            .set_state("RUNNABLE")
            .set_database_version(SqlDatabaseVersion::Postgres15)
            .set_settings(
                Settings::new()
                    .set_tier("db-custom-4-16384")
                    .set_availability_type(SqlAvailabilityType::Regional)
                    .set_ip_configuration(IpConfiguration::new().set_private_network(
                        "projects/planted-project/global/networks/planted-vpc",
                    )),
            );

    assert_eq!(
        map_instance(&instance),
        CloudSqlInstance {
            name: "planted-instance".to_string(),
            database_version: "POSTGRES_15".to_string(),
            engine: "PostgreSQL".to_string(),
            region: "europe-west4".to_string(),
            tier: "db-custom-4-16384".to_string(),
            availability: CloudSqlAvailability::Regional,
            status: CloudSqlStatus::Runnable,
            connectivity: EndpointVisibility::Private,
        },
        "each row field is the planted value, not a default and not another field's"
    );
}

#[test]
fn a_non_empty_private_network_makes_the_instance_private_and_an_empty_one_does_not() {
    let with_network = DatabaseInstance::new().set_settings(
        Settings::new().set_ip_configuration(
            IpConfiguration::new()
                .set_private_network("projects/planted-project/global/networks/planted-vpc")
                .set_ipv4_enabled(false),
        ),
    );
    let without_network = DatabaseInstance::new().set_settings(
        Settings::new().set_ip_configuration(
            IpConfiguration::new()
                .set_private_network("")
                .set_ipv4_enabled(true),
        ),
    );

    assert_eq!(
        map_instance(&with_network).connectivity,
        EndpointVisibility::Private,
        "a private network string that is present means private connectivity"
    );
    assert_eq!(
        map_instance(&without_network).connectivity,
        EndpointVisibility::Public,
        "an empty private network string is no private network — the `!` on is_empty() decides it"
    );
}

#[test]
fn ipv4_enabled_alone_is_public_and_a_disabled_one_without_addresses_is_unknown() {
    let ipv4 = DatabaseInstance::new().set_settings(
        Settings::new().set_ip_configuration(IpConfiguration::new().set_ipv4_enabled(true)),
    );
    let no_ipv4 = DatabaseInstance::new().set_settings(
        Settings::new().set_ip_configuration(IpConfiguration::new().set_ipv4_enabled(false)),
    );

    assert_eq!(
        map_instance(&ipv4).connectivity,
        EndpointVisibility::Public,
        "ipv4_enabled is read through, not defaulted to false"
    );
    assert_eq!(
        map_instance(&no_ipv4).connectivity,
        EndpointVisibility::Unknown,
        "nothing public and nothing private leaves connectivity unknown"
    );
}

#[test]
fn a_primary_address_alone_is_public_and_a_non_primary_one_is_not() {
    let primary = DatabaseInstance::new()
        .set_ip_addresses([ip(SqlIpAddressType::Primary, "34.90.1.2")])
        .set_settings(
            Settings::new().set_ip_configuration(IpConfiguration::new().set_ipv4_enabled(false)),
        );
    let outgoing_only = DatabaseInstance::new()
        .set_ip_addresses([
            ip(SqlIpAddressType::Outgoing, "34.90.9.9"),
            ip(SqlIpAddressType::Private, "10.24.0.7"),
        ])
        .set_settings(
            Settings::new().set_ip_configuration(IpConfiguration::new().set_ipv4_enabled(false)),
        );

    assert_eq!(
        map_instance(&primary).connectivity,
        EndpointVisibility::Public,
        "a PRIMARY address makes the instance reachable even with ipv4_enabled false"
    );
    assert_eq!(
        map_instance(&outgoing_only).connectivity,
        EndpointVisibility::Unknown,
        "OUTGOING and PRIVATE addresses are matched by name, not by being present at all"
    );
}

#[test]
fn a_private_network_outranks_a_public_address() {
    let both = DatabaseInstance::new()
        .set_ip_addresses([ip(SqlIpAddressType::Primary, "34.90.1.2")])
        .set_settings(
            Settings::new().set_ip_configuration(
                IpConfiguration::new()
                    .set_ipv4_enabled(true)
                    .set_private_network("projects/planted-project/global/networks/planted-vpc"),
            ),
        );

    assert_eq!(
        map_instance(&both).connectivity,
        EndpointVisibility::Private,
        "the private branch is taken first; an ipv4 address does not override it"
    );
}

#[test]
fn an_instance_without_settings_keeps_the_fields_it_does_have() {
    let bare = DatabaseInstance::new()
        .set_name("bare-instance")
        .set_region("us-central1")
        .set_state("RUNNABLE")
        .set_database_version(SqlDatabaseVersion::Mysql80);

    assert_eq!(
        map_instance(&bare),
        CloudSqlInstance {
            name: "bare-instance".to_string(),
            database_version: "MYSQL_8_0".to_string(),
            engine: "MySQL".to_string(),
            region: "us-central1".to_string(),
            tier: String::new(),
            availability: CloudSqlAvailability::Unknown,
            status: CloudSqlStatus::Runnable,
            connectivity: EndpointVisibility::Unknown,
        },
        "absent settings mean an empty tier and unknown availability, not an empty row"
    );
}

#[test]
fn the_engine_is_normalized_from_the_raw_database_version_for_every_family() {
    let engine_of = |version: SqlDatabaseVersion| {
        let mapped = map_instance(&DatabaseInstance::new().set_database_version(version));
        (mapped.engine, mapped.database_version)
    };

    assert_eq!(
        engine_of(SqlDatabaseVersion::Postgres15),
        ("PostgreSQL".to_string(), "POSTGRES_15".to_string())
    );
    assert_eq!(
        engine_of(SqlDatabaseVersion::Mysql80),
        ("MySQL".to_string(), "MYSQL_8_0".to_string())
    );
    assert_eq!(
        engine_of(SqlDatabaseVersion::Sqlserver2017Standard),
        (
            "SQL Server".to_string(),
            "SQLSERVER_2017_STANDARD".to_string()
        )
    );
    assert_eq!(
        engine_of(SqlDatabaseVersion::from("SPANNERDB_1")),
        ("Unknown".to_string(), "SPANNERDB_1".to_string()),
        "an engine the client does not know keeps its raw version and reads as Unknown"
    );
}

#[test]
fn a_database_version_without_a_name_maps_to_an_empty_raw_version() {
    let numeric_only =
        DatabaseInstance::new().set_database_version(SqlDatabaseVersion::from(9_999));
    let mapped = map_instance(&numeric_only);

    assert_eq!(
        mapped.database_version,
        String::new(),
        "an enum carrying only an unknown integer has no name, and `unwrap_or_default` supplies \"\""
    );
    assert_eq!(mapped.engine, "Unknown".to_string());
}

#[test]
fn availability_is_translated_for_both_named_arms_and_falls_back_to_unknown() {
    let availability_of = |kind: SqlAvailabilityType| {
        map_instance(
            &DatabaseInstance::new().set_settings(Settings::new().set_availability_type(kind)),
        )
        .availability
    };

    assert_eq!(
        availability_of(SqlAvailabilityType::Regional),
        CloudSqlAvailability::Regional
    );
    assert_eq!(
        availability_of(SqlAvailabilityType::Zonal),
        CloudSqlAvailability::Zonal
    );
    assert_eq!(
        availability_of(SqlAvailabilityType::Unspecified),
        CloudSqlAvailability::Unknown,
        "the API's UNSPECIFIED is not a third availability, it is no answer"
    );
    assert_eq!(
        availability_of(SqlAvailabilityType::from("MULTI_REGIONAL")),
        CloudSqlAvailability::Unknown,
        "an availability the client does not know reads as unknown rather than as zonal"
    );
}

#[test]
fn every_status_arm_is_translated_including_the_unrecognised_one() {
    let status_of = |state: &str| map_instance(&DatabaseInstance::new().set_state(state)).status;

    assert_eq!(status_of("RUNNABLE"), CloudSqlStatus::Runnable);
    assert_eq!(status_of("STOPPED"), CloudSqlStatus::Stopped);
    assert_eq!(status_of("PENDING_CREATE"), CloudSqlStatus::PendingCreate);
    assert_eq!(status_of("MAINTENANCE"), CloudSqlStatus::Maintenance);
    assert_eq!(status_of("FAILED"), CloudSqlStatus::Failed);
    assert_eq!(
        status_of("UNKNOWN_STATE"),
        CloudSqlStatus::Failed,
        "UNKNOWN_STATE is a failure, not an unrecognised label"
    );
    assert_eq!(status_of("SUSPENDED"), CloudSqlStatus::Suspended);
    assert_eq!(
        status_of("REPAIRING"),
        CloudSqlStatus::Unknown("REPAIRING".to_string()),
        "a state with no arm is carried through verbatim so the column can still show it"
    );
}

#[test]
fn a_state_carrying_no_name_becomes_an_empty_unknown_status() {
    let numeric_only = DatabaseInstance::new().set_state(SqlInstanceState::from(4_242));

    assert_eq!(
        map_instance(&numeric_only).status,
        CloudSqlStatus::Unknown(String::new()),
        "an integer-only state has no name and defaults to the empty string"
    );
}

// ---------------------------------------------------------------------------
// map_instance_detail
// ---------------------------------------------------------------------------

#[test]
fn the_detail_carries_every_planted_scalar() {
    let detail = map_instance_detail(&fully_planted_instance());

    assert_eq!(detail.name, "planted-instance");
    assert_eq!(detail.region, "europe-west4");
    assert_eq!(detail.database_version, "POSTGRES_15");
    assert_eq!(detail.engine, "PostgreSQL");
    assert_eq!(detail.tier, "db-custom-4-16384");
    assert_eq!(detail.availability, CloudSqlAvailability::Regional);
    assert_eq!(detail.status, CloudSqlStatus::Maintenance);
    assert_eq!(detail.activation_policy.as_deref(), Some("ALWAYS"));
    assert_eq!(
        detail.creation_timestamp.as_deref(),
        Some("2024-01-02T03:04:05Z"),
        "the protobuf timestamp is rendered the way the CLI path renders it"
    );
    assert_eq!(
        detail.connection_name.as_deref(),
        Some("planted-project:europe-west4:planted-instance")
    );
    assert_eq!(
        detail.maintenance_version.as_deref(),
        Some("POSTGRES_15_4.R20240101.00_00")
    );
    assert!(detail.require_ssl, "require_ssl is read, not assumed");
    assert_eq!(
        detail.private_network.as_deref(),
        Some("projects/planted-project/global/networks/planted-vpc")
    );
    assert_eq!(
        detail.master_instance.as_deref(),
        Some("planted-project:europe-west4:planted-primary")
    );
}

#[test]
fn the_public_and_private_addresses_are_selected_by_address_type() {
    let detail = map_instance_detail(&fully_planted_instance());

    assert_eq!(
        detail.public_ip.as_deref(),
        Some("34.90.1.2"),
        "the public IP is the PRIMARY mapping, not merely the first one"
    );
    assert_eq!(
        detail.private_ip.as_deref(),
        Some("10.24.0.7"),
        "the private IP is the PRIVATE mapping, not the OUTGOING one that precedes it"
    );
}

#[test]
fn an_address_type_with_no_mapping_and_one_with_an_empty_string_are_both_absent() {
    let instance = DatabaseInstance::new().set_ip_addresses([
        ip(SqlIpAddressType::Outgoing, "34.90.9.9"),
        ip(SqlIpAddressType::Private, ""),
    ]);
    let detail = map_instance_detail(&instance);

    assert_eq!(
        detail.public_ip, None,
        "no PRIMARY mapping is no public IP, not the OUTGOING address"
    );
    assert_eq!(
        detail.private_ip, None,
        "a PRIVATE mapping with an empty address string is absent, not an empty string"
    );
}

#[test]
fn an_absent_require_ssl_reads_as_false_and_an_empty_private_network_as_absent() {
    let instance = DatabaseInstance::new().set_settings(
        Settings::new().set_ip_configuration(IpConfiguration::new().set_private_network("")),
    );
    let detail = map_instance_detail(&instance);

    assert!(
        !detail.require_ssl,
        "an unset require_ssl is not an enforced one"
    );
    assert_eq!(detail.private_network, None);
}

#[test]
fn the_maintenance_window_carries_day_hour_and_update_track() {
    let detail = map_instance_detail(&fully_planted_instance());

    assert_eq!(
        detail.maintenance_window,
        Some(CloudSqlMaintenanceWindow {
            day: Some(3),
            hour: Some(23),
            update_track: Some("week5".to_string()),
        }),
        "each window field is its own planted value"
    );
}

#[test]
fn a_window_with_an_out_of_range_day_and_no_hour_keeps_the_fields_it_can() {
    let instance = DatabaseInstance::new().set_settings(
        Settings::new().set_maintenance_window(
            MaintenanceWindow::new()
                .set_day(-1)
                .set_or_clear_hour(None::<i32>)
                .set_update_track(SqlUpdateTrack::Canary),
        ),
    );

    assert_eq!(
        map_instance_detail(&instance).maintenance_window,
        Some(CloudSqlMaintenanceWindow {
            day: None,
            hour: None,
            update_track: Some("canary".to_string()),
        }),
        "a negative day does not convert to u32 and drops out; an unset hour stays unset"
    );
}

#[test]
fn an_instance_without_a_maintenance_window_has_none() {
    let instance = DatabaseInstance::new().set_settings(Settings::new().set_tier("db-f1-micro"));

    assert_eq!(
        map_instance_detail(&instance).maintenance_window,
        None,
        "no window configured is no window, not a window of zeroes"
    );
}

#[test]
fn a_configured_backup_carries_every_planted_backup_field() {
    let detail = map_instance_detail(&fully_planted_instance());

    assert_eq!(
        detail.backup,
        CloudSqlBackupConfig {
            enabled: true,
            start_time: Some("04:30".to_string()),
            location: Some("eu".to_string()),
            point_in_time_recovery: true,
            transaction_log_retention_days: Some(7),
        }
    );
}

#[test]
fn an_absent_backup_configuration_falls_back_to_the_disabled_defaults() {
    let instance = DatabaseInstance::new().set_settings(Settings::new().set_tier("db-f1-micro"));

    assert_eq!(
        map_instance_detail(&instance).backup,
        CloudSqlBackupConfig {
            enabled: false,
            start_time: None,
            location: None,
            point_in_time_recovery: false,
            transaction_log_retention_days: None,
        },
        "no backup block means backups are reported off rather than unknown"
    );
}

#[test]
fn a_backup_block_with_unset_flags_and_empty_strings_reports_them_off_and_absent() {
    let instance = DatabaseInstance::new().set_settings(
        Settings::new().set_backup_configuration(
            BackupConfiguration::new()
                .set_start_time("")
                .set_location("")
                .set_transaction_log_retention_days(-3),
        ),
    );

    assert_eq!(
        map_instance_detail(&instance).backup,
        CloudSqlBackupConfig {
            enabled: false,
            start_time: None,
            location: None,
            point_in_time_recovery: false,
            transaction_log_retention_days: None,
        },
        "unset booleans are false, empty strings are absent, and a negative retention drops out"
    );
}

#[test]
fn replicas_keep_the_order_the_api_returned_and_an_empty_master_is_absent() {
    let detail = map_instance_detail(&fully_planted_instance());
    assert_eq!(
        detail.replicas,
        vec![
            "planted-replica-a".to_string(),
            "planted-replica-b".to_string()
        ],
        "every replica name is carried, in order"
    );

    let standalone = map_instance_detail(&DatabaseInstance::new().set_name("standalone"));
    assert!(
        standalone.replicas.is_empty(),
        "an instance with no replicas lists none"
    );
    assert_eq!(
        standalone.master_instance, None,
        "an empty master name means this instance is not a replica"
    );
}

#[test]
fn labels_are_carried_as_key_value_pairs_and_absent_settings_yield_none() {
    let detail = map_instance_detail(&fully_planted_instance());
    assert_eq!(
        sorted_labels(&detail.labels),
        vec![
            ("env".to_string(), "prod".to_string()),
            ("team".to_string(), "data-platform".to_string()),
        ],
        "each label keeps its own key and its own value"
    );

    let bare = map_instance_detail(&DatabaseInstance::new().set_name("bare"));
    assert!(
        bare.labels.is_empty(),
        "no settings means no labels rather than a fabricated pair"
    );
}

#[test]
fn an_activation_policy_without_a_name_is_absent_rather_than_unspecified() {
    let named = DatabaseInstance::new()
        .set_settings(Settings::new().set_activation_policy(SqlActivationPolicy::Never));
    let numeric_only = DatabaseInstance::new()
        .set_settings(Settings::new().set_activation_policy(SqlActivationPolicy::from(99)));

    assert_eq!(
        map_instance_detail(&named).activation_policy.as_deref(),
        Some("NEVER")
    );
    assert_eq!(
        map_instance_detail(&numeric_only).activation_policy,
        None,
        "a policy carrying only an unknown integer has no name to show"
    );
}

#[test]
fn a_detail_without_a_create_time_or_connection_name_leaves_them_absent() {
    let detail = map_instance_detail(&DatabaseInstance::new().set_name("bare"));

    assert_eq!(
        detail.creation_timestamp, None,
        "no create_time is no timestamp, not the epoch"
    );
    assert_eq!(detail.connection_name, None);
    assert_eq!(detail.maintenance_version, None);
    assert_eq!(detail.activation_policy, None);
    assert_eq!(detail.public_ip, None);
    assert_eq!(detail.private_ip, None);
}

#[test]
fn a_detail_without_settings_falls_back_on_every_field_settings_would_have_supplied() {
    let detail = map_instance_detail(
        &DatabaseInstance::new()
            .set_name("bare-detail")
            .set_region("us-east4")
            .set_state("STOPPED")
            .set_database_version(SqlDatabaseVersion::Sqlserver2017Standard),
    );

    assert_eq!(detail.name, "bare-detail");
    assert_eq!(detail.region, "us-east4");
    assert_eq!(detail.database_version, "SQLSERVER_2017_STANDARD");
    assert_eq!(detail.engine, "SQL Server");
    assert_eq!(detail.status, CloudSqlStatus::Stopped);
    assert_eq!(
        detail.tier,
        String::new(),
        "absent settings mean an empty tier, and the fields outside settings survive it"
    );
    assert_eq!(
        detail.availability,
        CloudSqlAvailability::Unknown,
        "no settings is no availability answer, not zonal"
    );
    assert!(
        !detail.require_ssl,
        "no ip configuration to read means SSL is not reported as enforced"
    );
    assert_eq!(detail.private_network, None);
    assert_eq!(detail.maintenance_window, None);
    assert!(detail.labels.is_empty());
    assert!(detail.replicas.is_empty());
}

#[test]
fn the_users_list_stays_empty_because_the_users_tab_fetches_it_separately() {
    let detail = map_instance_detail(&fully_planted_instance());

    assert!(
        detail.users.is_empty(),
        "the detail load must not imply a second call; the Users tab makes it"
    );
}

#[test]
fn the_raw_json_pane_holds_the_rendered_instance() {
    let detail = map_instance_detail(&fully_planted_instance());

    assert!(
        detail.raw_json.contains("\"name\": \"planted-instance\""),
        "the JSON tab renders this instance, pretty-printed:\n{}",
        detail.raw_json
    );
    assert!(
        detail.raw_json.contains("\"region\": \"europe-west4\""),
        "the whole model is rendered, not just its name:\n{}",
        detail.raw_json
    );
}

// ---------------------------------------------------------------------------
// map_user
// ---------------------------------------------------------------------------

#[test]
fn a_user_carries_its_name_host_and_type() {
    use google_cloud_sql_v1::model::user::SqlUserType;

    let user = User::new()
        .set_name("planted-user")
        .set_host("10.24.0.0/24")
        .set_type(SqlUserType::CloudIamServiceAccount);

    assert_eq!(
        map_user(&user),
        CloudSqlUser {
            name: "planted-user".to_string(),
            host: Some("10.24.0.0/24".to_string()),
            kind: Some("CLOUD_IAM_SERVICE_ACCOUNT".to_string()),
        },
        "each user field is its own planted value"
    );
}

/// The TYPE column is the user's type, not the model's `kind`.
///
/// `User::kind` is documented as the constant `sql#user`, and reading it
/// painted that constant into the TYPE column against the live API where the
/// CLI path painted the authentication type. A non-default type planted
/// alongside a `kind` that would be the wrong answer: a mapper reading `kind`
/// reads `sql#user` here, and one reading the type reads the planted word.
#[test]
fn the_type_column_is_the_user_type_and_never_the_models_kind_constant() {
    use google_cloud_sql_v1::model::user::SqlUserType;

    let user = User::new()
        .set_name("iam-user")
        .set_kind("sql#user")
        .set_type(SqlUserType::CloudIamUser);

    assert_eq!(
        map_user(&user).kind.as_deref(),
        Some("CLOUD_IAM_USER"),
        "the authentication type, in the same word the CLI path carries \
         through from the `type` key"
    );
}

#[test]
fn an_empty_host_is_absent_and_an_unstated_type_is_the_built_in_default() {
    let user = User::new().set_name("postgres").set_host("");

    assert_eq!(
        map_user(&user),
        CloudSqlUser {
            name: "postgres".to_string(),
            host: None,
            kind: Some("BUILT_IN".to_string()),
        },
        "a PostgreSQL user has no host, and \"\" is how the client spells \
         that; a user that states no type IS a built-in one -- proto3's \
         default for `SqlUserType` is `BUILT_IN`, and the field's own doc says \
         the default is the database's built-in user type"
    );
}

/// A type this build has no word for reads as a blank cell, not as a number.
#[test]
fn a_type_that_arrived_as_an_unknown_number_leaves_the_column_blank() {
    use google_cloud_sql_v1::model::user::SqlUserType;

    let user = User::new()
        .set_name("future")
        .set_type(SqlUserType::from(97));

    assert_eq!(
        map_user(&user).kind,
        None,
        "`name()` has no word for it, and `97` in a TYPE column would read as \
         a type called 97"
    );
}

#[test]
fn a_password_on_the_model_changes_nothing_about_the_mapped_user() {
    let bare = User::new().set_name("planted-user").set_host("%");
    let with_password = User::new()
        .set_name("planted-user")
        .set_host("%")
        .set_password("planted-secret");

    assert_eq!(
        map_user(&with_password),
        map_user(&bare),
        "nothing in the row is derived from a password"
    );
    let mapped = map_user(&with_password);
    assert!(
        !format!("{mapped:?}").contains("planted-secret"),
        "a password must not reach a row: {mapped:?}"
    );
}
