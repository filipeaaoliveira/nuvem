//! The Secret Manager mappers, which a mutation sweep reported as untested:
//! `map_secret`, `map_version` and `map_secret_detail` in
//! `src/services/sdk/secrets.rs`. Every field is planted with a distinct
//! non-default value and asserted exactly, so a body replaced wholesale cannot
//! pass; every branch — replication kind, version state, rotation shape,
//! expiration oneof, the non-empty location filter — is exercised from both
//! sides with inputs that render differently.

use google_cloud_secretmanager_v1::model as api;
use google_cloud_wkt::{Duration, Timestamp};

use nuvem::services::sdk::secrets::{map_secret, map_secret_detail, map_version};
use nuvem_domain::gcp::resource::{ComputeKeyValue, Secret, SecretVersion, SecretVersionState};

/// `2024-01-02T03:04:05Z`.
const CREATED_AT: i64 = 1_704_164_645;
/// `2025-01-01T03:04:05Z`.
const NEXT_ROTATION_AT: i64 = 1_735_700_645;
/// `2026-01-01T03:04:05Z`.
const EXPIRES_AT: i64 = 1_767_236_645;

fn stamp(seconds: i64) -> Timestamp {
    Timestamp::new(seconds, 0).expect("a valid timestamp")
}

fn pair(key: &str, value: &str) -> ComputeKeyValue {
    ComputeKeyValue {
        key: key.to_string(),
        value: value.to_string(),
    }
}

fn replica(location: &str) -> api::replication::user_managed::Replica {
    api::replication::user_managed::Replica::new().set_location(location)
}

fn user_managed(locations: &[&str]) -> api::Replication {
    let replicas = locations.iter().copied().map(replica);
    api::Replication::new()
        .set_user_managed(api::replication::UserManaged::new().set_replicas(replicas))
}

fn automatic() -> api::Replication {
    api::Replication::new().set_automatic(api::replication::Automatic::new())
}

// ---------------------------------------------------------------------------
// map_version
// ---------------------------------------------------------------------------

#[test]
fn a_version_maps_its_short_id_enabled_state_and_create_time() {
    let version = api::SecretVersion::new()
        .set_name("projects/acme-prod/secrets/api-key/versions/7")
        .set_state(api::secret_version::State::Enabled)
        .set_create_time(stamp(CREATED_AT));

    assert_eq!(
        map_version(&version),
        SecretVersion {
            name: "7".to_string(),
            state: SecretVersionState::Enabled,
            create_time: Some("2024-01-02T03:04:05Z".to_string()),
        },
        "every field is carried: the trailing segment of the path, the \
         translated state, and the CLI's timestamp shape"
    );
}

#[test]
fn a_disabled_version_does_not_read_as_enabled() {
    let version = api::SecretVersion::new()
        .set_name("projects/acme-prod/secrets/api-key/versions/12")
        .set_state(api::secret_version::State::Disabled)
        .set_create_time(stamp(1_712_345_678));

    assert_eq!(
        map_version(&version),
        SecretVersion {
            name: "12".to_string(),
            state: SecretVersionState::Disabled,
            create_time: Some("2024-04-05T19:34:38Z".to_string()),
        }
    );
}

#[test]
fn a_destroyed_version_does_not_read_as_enabled() {
    let version = api::SecretVersion::new()
        .set_name("projects/acme-prod/secrets/api-key/versions/1")
        .set_state(api::secret_version::State::Destroyed)
        .set_create_time(stamp(1_600_000_000));

    assert_eq!(
        map_version(&version),
        SecretVersion {
            name: "1".to_string(),
            state: SecretVersionState::Destroyed,
            create_time: Some("2020-09-13T12:26:40Z".to_string()),
        }
    );
}

#[test]
fn an_unrecognised_state_name_is_carried_through_verbatim() {
    let version = api::SecretVersion::new()
        .set_name("projects/acme-prod/secrets/api-key/versions/9")
        .set_state(api::secret_version::State::from(
            "SCHEDULED_FOR_DESTRUCTION",
        ))
        .set_create_time(stamp(CREATED_AT));

    assert_eq!(
        map_version(&version),
        SecretVersion {
            name: "9".to_string(),
            state: SecretVersionState::Unknown("SCHEDULED_FOR_DESTRUCTION".to_string()),
            create_time: Some("2024-01-02T03:04:05Z".to_string()),
        },
        "a state nuvem does not understand must not collapse into a known one"
    );
}

#[test]
fn the_unspecified_state_is_unknown_under_its_own_name() {
    let version = api::SecretVersion::new()
        .set_name("projects/acme-prod/secrets/api-key/versions/3")
        .set_state(api::secret_version::State::Unspecified)
        .set_create_time(stamp(CREATED_AT));

    assert_eq!(
        map_version(&version).state,
        SecretVersionState::Unknown("STATE_UNSPECIFIED".to_string())
    );
}

#[test]
fn a_state_that_has_no_name_is_unknown_with_an_empty_label() {
    // An enum deserialized from an unrecognised *integer* has a value but no
    // name, which is the `None` arm of the state translation.
    let version = api::SecretVersion::new()
        .set_name("projects/acme-prod/secrets/api-key/versions/4")
        .set_state(api::secret_version::State::from(97_i32));

    assert_eq!(
        map_version(&version),
        SecretVersion {
            name: "4".to_string(),
            state: SecretVersionState::Unknown(String::new()),
            create_time: None,
        },
        "no name and no create time are both absences, not defaults of a \
         known state"
    );
}

#[test]
fn a_version_name_with_no_slash_passes_through_unchanged() {
    let version = api::SecretVersion::new()
        .set_name("42")
        .set_state(api::secret_version::State::Enabled);

    assert_eq!(map_version(&version).name, "42");
}

// ---------------------------------------------------------------------------
// map_secret
// ---------------------------------------------------------------------------

#[test]
fn an_automatic_secret_maps_every_field_and_reads_as_automatic() {
    let secret = api::Secret::new()
        .set_name("projects/acme-prod/secrets/api-key")
        .set_create_time(stamp(CREATED_AT))
        .set_labels([("team", "platform"), ("env", "prod")])
        .set_replication(automatic());

    assert_eq!(
        map_secret(&secret),
        Secret {
            name: "api-key".to_string(),
            create_time: Some("2024-01-02T03:04:05Z".to_string()),
            labels: vec![pair("env", "prod"), pair("team", "platform")],
            replication: "Automatic".to_string(),
        },
        "labels sort by key so a reload cannot reshuffle them, and an \
         automatic policy names no locations"
    );
}

#[test]
fn a_user_managed_secret_names_its_locations_in_order() {
    let secret = api::Secret::new()
        .set_name("projects/acme-prod/secrets/db-password")
        .set_create_time(stamp(NEXT_ROTATION_AT))
        .set_labels([("owner", "payments")])
        .set_replication(user_managed(&["europe-west1", "europe-west4"]));

    assert_eq!(
        map_secret(&secret),
        Secret {
            name: "db-password".to_string(),
            create_time: Some("2025-01-01T03:04:05Z".to_string()),
            labels: vec![pair("owner", "payments")],
            replication: "User-managed (europe-west1, europe-west4)".to_string(),
        },
        "the replica order the API returned is the order the cell shows"
    );
}

#[test]
fn a_replica_with_no_location_is_dropped_rather_than_rendered_blank() {
    let secret = api::Secret::new()
        .set_name("projects/acme-prod/secrets/db-password")
        .set_replication(user_managed(&["", "us-east1", ""]));

    assert_eq!(
        map_secret(&secret).replication,
        "User-managed (us-east1)",
        "an unset location contributes nothing, so the cell never reads \
         'User-managed (, us-east1, )'"
    );
}

#[test]
fn a_user_managed_policy_with_no_usable_locations_is_named_without_a_list() {
    let empty = api::Secret::new()
        .set_name("projects/acme-prod/secrets/no-replicas")
        .set_replication(user_managed(&[]));
    assert_eq!(
        map_secret(&empty).replication,
        "User-managed",
        "no replicas at all still says who chose the policy"
    );

    let blank = api::Secret::new()
        .set_name("projects/acme-prod/secrets/blank-replicas")
        .set_replication(user_managed(&["", ""]));
    assert_eq!(
        map_secret(&blank).replication,
        "User-managed",
        "replicas whose locations are all unset leave nothing to list"
    );
}

#[test]
fn a_secret_with_no_replication_field_reads_as_unknown() {
    let secret = api::Secret::new().set_name("projects/acme-prod/secrets/orphan");

    assert_eq!(
        map_secret(&secret),
        Secret {
            name: "orphan".to_string(),
            create_time: None,
            labels: Vec::new(),
            replication: "Unknown".to_string(),
        },
        "absent replication is neither automatic nor user-managed"
    );
}

#[test]
fn a_replication_message_with_no_policy_set_reads_as_unknown() {
    let secret = api::Secret::new()
        .set_name("projects/acme-prod/secrets/half-set")
        .set_replication(api::Replication::new());

    assert_eq!(
        map_secret(&secret).replication,
        "Unknown",
        "a replication message carrying neither branch is as unknown as none"
    );
}

// ---------------------------------------------------------------------------
// map_secret_detail
// ---------------------------------------------------------------------------

#[test]
fn a_fully_populated_detail_carries_every_field_distinctly() {
    let secret = api::Secret::new()
        .set_name("projects/acme-prod/secrets/api-key")
        .set_create_time(stamp(CREATED_AT))
        .set_labels([("team", "platform"), ("env", "prod")])
        .set_annotations([("owner", "payments"), ("audit", "2024")])
        .set_replication(user_managed(&["europe-west1", "europe-west4"]))
        .set_rotation(
            api::Rotation::new()
                .set_rotation_period(Duration::new(604_800, 0).expect("a valid duration"))
                .set_next_rotation_time(stamp(NEXT_ROTATION_AT)),
        )
        .set_expire_time(stamp(EXPIRES_AT));

    let detail = map_secret_detail(&secret);

    assert_eq!(detail.name, "api-key");
    assert_eq!(
        detail.create_time.as_deref(),
        Some("2024-01-02T03:04:05Z"),
        "the create time is the secret's own, not the rotation's"
    );
    assert_eq!(
        detail.labels,
        vec![pair("env", "prod"), pair("team", "platform")]
    );
    assert_eq!(
        detail.annotations,
        vec![pair("audit", "2024"), pair("owner", "payments")],
        "annotations are their own sorted list, not a copy of the labels"
    );
    assert_eq!(
        detail.replication,
        "User-managed (europe-west1, europe-west4)"
    );
    assert_eq!(
        detail.rotation.as_deref(),
        Some("every 604800s, next 2025-01-01T03:04:05Z"),
        "a policy with both halves reads as both"
    );
    assert_eq!(
        detail.expire_time.as_deref(),
        Some("2026-01-01T03:04:05Z"),
        "the absolute expiry is the secret's own, not the rotation time"
    );
    assert!(
        detail.versions.is_empty(),
        "versions are filled by the separate fetch, never by describe"
    );

    let raw: serde_json::Value =
        serde_json::from_str(&detail.raw_json).expect("the JSON tab holds parseable JSON");
    assert_eq!(raw["name"], "projects/acme-prod/secrets/api-key");
    assert_eq!(raw["labels"]["team"], "platform");
    assert_eq!(raw["annotations"]["owner"], "payments");
}

#[test]
fn a_rotation_with_only_a_period_says_only_how_often() {
    let secret = api::Secret::new()
        .set_name("projects/acme-prod/secrets/api-key")
        .set_rotation(
            api::Rotation::new()
                .set_rotation_period(Duration::new(3_600, 0).expect("a valid duration")),
        );

    assert_eq!(
        map_secret_detail(&secret).rotation.as_deref(),
        Some("every 3600s")
    );
}

#[test]
fn a_rotation_with_only_a_next_time_says_only_when() {
    let secret = api::Secret::new()
        .set_name("projects/acme-prod/secrets/api-key")
        .set_rotation(api::Rotation::new().set_next_rotation_time(stamp(NEXT_ROTATION_AT)));

    assert_eq!(
        map_secret_detail(&secret).rotation.as_deref(),
        Some("next 2025-01-01T03:04:05Z")
    );
}

#[test]
fn a_rotation_policy_carrying_neither_half_reads_as_absent() {
    let secret = api::Secret::new()
        .set_name("projects/acme-prod/secrets/api-key")
        .set_rotation(api::Rotation::new());

    assert_eq!(
        map_secret_detail(&secret).rotation,
        None,
        "an empty policy carries no information and must not render as \
         'every ' or 'next '"
    );
}

#[test]
fn a_secret_with_no_rotation_policy_has_no_rotation_cell() {
    let secret = api::Secret::new().set_name("projects/acme-prod/secrets/api-key");

    assert_eq!(map_secret_detail(&secret).rotation, None);
}

#[test]
fn a_ttl_expiration_reports_no_expire_time_rather_than_a_computed_one() {
    let secret = api::Secret::new()
        .set_name("projects/acme-prod/secrets/short-lived")
        .set_ttl(Duration::new(86_400, 0).expect("a valid duration"));

    assert_eq!(
        map_secret_detail(&secret).expire_time,
        None,
        "only the absolute branch of the expiration oneof is a timestamp the \
         pane can show"
    );
}

#[test]
fn an_absolute_expiration_is_rendered_and_an_absent_one_is_none() {
    let expiring = api::Secret::new()
        .set_name("projects/acme-prod/secrets/short-lived")
        .set_expire_time(stamp(EXPIRES_AT));
    assert_eq!(
        map_secret_detail(&expiring).expire_time.as_deref(),
        Some("2026-01-01T03:04:05Z")
    );

    let permanent = api::Secret::new().set_name("projects/acme-prod/secrets/permanent");
    assert_eq!(map_secret_detail(&permanent).expire_time, None);
}

#[test]
fn an_automatic_detail_reads_as_automatic_and_names_no_locations() {
    let secret = api::Secret::new()
        .set_name("projects/acme-prod/secrets/api-key")
        .set_replication(automatic());

    assert_eq!(map_secret_detail(&secret).replication, "Automatic");
}

#[test]
fn a_detail_with_no_replication_field_reads_as_unknown() {
    let secret = api::Secret::new().set_name("projects/acme-prod/secrets/api-key");

    assert_eq!(map_secret_detail(&secret).replication, "Unknown");
}

#[test]
fn a_detail_drops_replicas_with_no_location_as_the_list_row_does() {
    let secret = api::Secret::new()
        .set_name("projects/acme-prod/secrets/api-key")
        .set_replication(user_managed(&["", "asia-east1"]));

    assert_eq!(
        map_secret_detail(&secret).replication,
        "User-managed (asia-east1)",
        "the detail pane and the table cell are the same wording over the \
         same filter"
    );
}

#[test]
fn an_empty_secret_maps_to_absences_rather_than_to_placeholder_values() {
    let detail = map_secret_detail(&api::Secret::new());

    assert_eq!(detail.name, "");
    assert_eq!(detail.create_time, None);
    assert!(detail.labels.is_empty());
    assert!(detail.annotations.is_empty());
    assert_eq!(detail.replication, "Unknown");
    assert_eq!(detail.rotation, None);
    assert_eq!(detail.expire_time, None);
    assert!(detail.versions.is_empty());
    assert_eq!(
        detail.raw_json, "{}",
        "an all-unset secret renders as an empty object, not as an error string"
    );
}

#[test]
fn the_detail_and_the_list_row_agree_on_the_fields_they_share() {
    let secret = api::Secret::new()
        .set_name("projects/acme-prod/secrets/api-key")
        .set_create_time(stamp(CREATED_AT))
        .set_labels([("team", "platform"), ("env", "prod")])
        .set_replication(user_managed(&["europe-west1", "europe-west4"]));

    let row = map_secret(&secret);
    let detail = map_secret_detail(&secret);

    assert_eq!(row.name, detail.name);
    assert_eq!(row.create_time, detail.create_time);
    assert_eq!(row.labels, detail.labels);
    assert_eq!(
        row.replication, detail.replication,
        "one wording, so opening a row cannot change what the policy says"
    );
}
