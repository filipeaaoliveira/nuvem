//! The Cloud Storage mappers turn a generated client model into the rows and
//! detail structs the panes render, and a mutation sweep found every one of
//! them unpinned: a body replaced wholesale, a boolean flipped, or a fallback
//! branch deleted, and nothing went red.
//!
//! Each test plants a *distinct, non-default* value in every field it asserts,
//! so a mapper that answered `Default::default()` or a constant cannot pass,
//! and every conditional — `bucket_id` vs `name`, a versioning block present
//! but disabled, an empty checksum, generation zero — is exercised from both
//! sides with outputs that differ. Base64 and timestamp expectations are
//! written out literally rather than recomputed from the input, so a mapper
//! that stopped encoding is visible rather than tautologically agreed with.

use google_cloud_storage::model::bucket::iam_config::UniformBucketLevelAccess;
use google_cloud_storage::model::bucket::{
    Billing, Encryption, IamConfig, RetentionPolicy, Versioning,
};
use google_cloud_storage::model::{Bucket, Object, ObjectChecksums};
use google_cloud_wkt::{Duration, Timestamp};
use nuvem::services::sdk::storage::{map_bucket, map_bucket_detail, map_object, map_object_detail};
use nuvem_domain::gcp::resource::{
    ComputeKeyValue, StorageBucket, StorageBucketDetail, StorageObject, StorageObjectDetail,
};

/// 2023-11-14T22:13:20Z, written out so the formatter is pinned rather than
/// re-derived from the same seconds the mapper reads.
const CREATED_SECONDS: i64 = 1_700_000_000;
const CREATED_RENDERED: &str = "2023-11-14T22:13:20Z";
/// 2024-03-09T16:00:00Z.
const UPDATED_SECONDS: i64 = 1_710_000_000;
const UPDATED_RENDERED: &str = "2024-03-09T16:00:00Z";
/// 2024-07-03T09:46:40Z.
const RETAINED_SECONDS: i64 = 1_720_000_000;
const RETAINED_RENDERED: &str = "2024-07-03T09:46:40Z";

const KMS_KEY: &str = "projects/p/locations/l/keyRings/r/cryptoKeys/planted-key";

/// A 16-byte MD5 digest and its base64 spelling, both written by hand.
const MD5_DIGEST: [u8; 16] = [
    0x9e, 0x10, 0x7d, 0x9d, 0x37, 0x2b, 0xb6, 0x82, 0x6b, 0xd8, 0x1d, 0x35, 0x42, 0xa4, 0x19, 0xd6,
];
const MD5_BASE64: &str = "nhB9nTcrtoJr2B01QqQZ1g==";

fn timestamp(seconds: i64) -> Timestamp {
    Timestamp::new(seconds, 0).expect("timestamp seconds are in range")
}

// ---------------------------------------------------------------------------
// bucket_id: the row shows the id, and falls back through two spellings.
// ---------------------------------------------------------------------------

#[test]
fn a_bucket_row_names_itself_by_bucket_id_not_by_the_resource_name() {
    let bucket = Bucket::new()
        .set_name("projects/p/buckets/resource-name")
        .set_bucket_id("planted-id");

    assert_eq!(map_bucket(&bucket).name, "planted-id");
}

#[test]
fn a_bucket_without_a_bucket_id_is_named_by_the_last_segment_of_its_resource_name() {
    let bucket = Bucket::new().set_name("projects/p/buckets/derived-from-name");

    assert_eq!(map_bucket(&bucket).name, "derived-from-name");
}

#[test]
fn a_bucket_name_carrying_no_slash_is_used_whole() {
    let bucket = Bucket::new().set_name("bare-name");

    assert_eq!(map_bucket(&bucket).name, "bare-name");
}

// ---------------------------------------------------------------------------
// map_bucket
// ---------------------------------------------------------------------------

#[test]
fn map_bucket_carries_every_field_with_its_planted_value() {
    let bucket = Bucket::new()
        .set_name("projects/p/buckets/ignored")
        .set_bucket_id("planted-id")
        .set_location("europe-west1")
        .set_location_type("region")
        .set_storage_class("NEARLINE")
        .set_versioning(Versioning::new().set_enabled(true))
        .set_iam_config(
            IamConfig::new()
                .set_uniform_bucket_level_access(UniformBucketLevelAccess::new().set_enabled(true))
                .set_public_access_prevention("enforced"),
        )
        .set_create_time(timestamp(CREATED_SECONDS))
        .set_update_time(timestamp(UPDATED_SECONDS));

    assert_eq!(
        map_bucket(&bucket),
        StorageBucket {
            name: "planted-id".to_string(),
            location: "europe-west1".to_string(),
            location_type: Some("region".to_string()),
            storage_class: "NEARLINE".to_string(),
            versioning_enabled: true,
            uniform_bucket_level_access: true,
            public_access_prevention: Some("enforced".to_string()),
            time_created: Some(CREATED_RENDERED.to_string()),
            updated: Some(UPDATED_RENDERED.to_string()),
        }
    );
}

#[test]
fn map_bucket_reports_false_when_the_blocks_are_present_and_say_so() {
    let bucket = Bucket::new()
        .set_name("projects/p/buckets/quiet")
        .set_bucket_id("quiet")
        .set_location("us")
        .set_location_type("multi-region")
        .set_storage_class("STANDARD")
        .set_versioning(Versioning::new().set_enabled(false))
        .set_iam_config(
            IamConfig::new()
                .set_uniform_bucket_level_access(UniformBucketLevelAccess::new().set_enabled(false))
                .set_public_access_prevention(""),
        );

    assert_eq!(
        map_bucket(&bucket),
        StorageBucket {
            name: "quiet".to_string(),
            location: "us".to_string(),
            location_type: Some("multi-region".to_string()),
            storage_class: "STANDARD".to_string(),
            versioning_enabled: false,
            uniform_bucket_level_access: false,
            public_access_prevention: None,
            time_created: None,
            updated: None,
        }
    );
}

#[test]
fn map_bucket_reports_false_when_the_blocks_are_absent_entirely() {
    let bucket = Bucket::new()
        .set_name("projects/p/buckets/bare")
        .set_bucket_id("bare");

    assert_eq!(
        map_bucket(&bucket),
        StorageBucket {
            name: "bare".to_string(),
            location: String::new(),
            location_type: None,
            storage_class: String::new(),
            versioning_enabled: false,
            uniform_bucket_level_access: false,
            public_access_prevention: None,
            time_created: None,
            updated: None,
        }
    );
}

#[test]
fn a_bucket_with_an_iam_config_but_no_uniform_access_block_is_not_uniform() {
    let bucket = Bucket::new()
        .set_bucket_id("half-configured")
        .set_iam_config(IamConfig::new().set_public_access_prevention("inherited"));

    let mapped = map_bucket(&bucket);

    assert!(!mapped.uniform_bucket_level_access);
    assert_eq!(
        mapped.public_access_prevention,
        Some("inherited".to_string())
    );
}

// ---------------------------------------------------------------------------
// map_object
// ---------------------------------------------------------------------------

#[test]
fn map_object_carries_every_field_with_its_planted_value() {
    let object = Object::new()
        .set_name("folder/planted.txt")
        .set_size(4_096_i64)
        .set_storage_class("ARCHIVE")
        .set_generation(1_234_567_890_i64)
        .set_update_time(timestamp(UPDATED_SECONDS))
        .set_content_type("application/pdf")
        .set_kms_key(KMS_KEY)
        .set_temporary_hold(true)
        .set_event_based_hold(true)
        .set_retention_expire_time(timestamp(RETAINED_SECONDS));

    assert_eq!(
        map_object(&object),
        StorageObject {
            key: "folder/planted.txt".to_string(),
            size_bytes: Some(4096),
            storage_class: Some("ARCHIVE".to_string()),
            generation: Some("1234567890".to_string()),
            updated: Some(UPDATED_RENDERED.to_string()),
            content_type: Some("application/pdf".to_string()),
            kms_encrypted: true,
            temporary_hold: true,
            event_based_hold: true,
            has_retention: true,
        }
    );
}

#[test]
fn map_object_reports_every_flag_off_when_nothing_is_set() {
    let object = Object::new().set_name("plain.txt").set_size(0_i64);

    assert_eq!(
        map_object(&object),
        StorageObject {
            key: "plain.txt".to_string(),
            size_bytes: Some(0),
            storage_class: None,
            generation: None,
            updated: None,
            content_type: None,
            kms_encrypted: false,
            temporary_hold: false,
            event_based_hold: false,
            has_retention: false,
        }
    );
}

#[test]
fn an_object_generation_of_zero_is_no_generation_at_all() {
    let unversioned = Object::new().set_name("o").set_generation(0_i64);
    let versioned = Object::new().set_name("o").set_generation(7_i64);

    assert_eq!(map_object(&unversioned).generation, None);
    assert_eq!(map_object(&versioned).generation, Some("7".to_string()));
}

#[test]
fn an_object_is_kms_encrypted_exactly_when_it_carries_a_key_name() {
    let encrypted = Object::new().set_name("o").set_kms_key(KMS_KEY);
    let plain = Object::new().set_name("o").set_kms_key("");

    assert!(map_object(&encrypted).kms_encrypted);
    assert!(!map_object(&plain).kms_encrypted);
}

#[test]
fn an_event_based_hold_is_false_when_unset_and_when_explicitly_false() {
    let unset = Object::new().set_name("o");
    let explicit_false = Object::new().set_name("o").set_event_based_hold(false);
    let held = Object::new().set_name("o").set_event_based_hold(true);

    assert!(!map_object(&unset).event_based_hold);
    assert!(!map_object(&explicit_false).event_based_hold);
    assert!(map_object(&held).event_based_hold);
}

#[test]
fn an_object_has_retention_exactly_when_an_expiry_is_present() {
    let retained = Object::new()
        .set_name("o")
        .set_retention_expire_time(timestamp(RETAINED_SECONDS));
    let free = Object::new().set_name("o");

    assert!(map_object(&retained).has_retention);
    assert!(!map_object(&free).has_retention);
}

#[test]
fn a_negative_object_size_is_not_shown_rather_than_wrapped() {
    let object = Object::new().set_name("o").set_size(-1_i64);

    assert_eq!(map_object(&object).size_bytes, None);
}

// ---------------------------------------------------------------------------
// map_bucket_detail
// ---------------------------------------------------------------------------

#[test]
fn map_bucket_detail_carries_every_field_with_its_planted_value() {
    let bucket = Bucket::new()
        .set_name("projects/p/buckets/ignored")
        .set_bucket_id("planted-id")
        .set_location("europe-west1")
        .set_location_type("region")
        .set_storage_class("NEARLINE")
        .set_versioning(Versioning::new().set_enabled(true))
        .set_iam_config(
            IamConfig::new()
                .set_uniform_bucket_level_access(UniformBucketLevelAccess::new().set_enabled(true))
                .set_public_access_prevention("enforced"),
        )
        .set_billing(Billing::new().set_requester_pays(true))
        .set_retention_policy(
            RetentionPolicy::new()
                .set_retention_duration(Duration::new(86_400, 0).expect("duration is in range")),
        )
        .set_encryption(Encryption::new().set_default_kms_key(KMS_KEY))
        .set_labels([("env", "prod")])
        .set_create_time(timestamp(CREATED_SECONDS))
        .set_update_time(timestamp(UPDATED_SECONDS));

    let mapped = map_bucket_detail(&bucket);

    assert_eq!(
        mapped,
        StorageBucketDetail {
            name: "planted-id".to_string(),
            location: "europe-west1".to_string(),
            location_type: Some("region".to_string()),
            storage_class: "NEARLINE".to_string(),
            versioning_enabled: true,
            uniform_bucket_level_access: true,
            public_access_prevention: Some("enforced".to_string()),
            requester_pays: true,
            retention_period: Some("86400".to_string()),
            default_kms_key_present: true,
            labels: vec![ComputeKeyValue {
                key: "env".to_string(),
                value: "prod".to_string(),
            }],
            time_created: Some(CREATED_RENDERED.to_string()),
            updated: Some(UPDATED_RENDERED.to_string()),
            raw_json: mapped.raw_json.clone(),
        }
    );
}

#[test]
fn map_bucket_detail_reports_false_when_the_blocks_are_present_and_say_so() {
    let bucket = Bucket::new()
        .set_name("projects/p/buckets/quiet")
        .set_bucket_id("quiet")
        .set_location("us")
        .set_location_type("multi-region")
        .set_storage_class("STANDARD")
        .set_versioning(Versioning::new().set_enabled(false))
        .set_iam_config(
            IamConfig::new()
                .set_uniform_bucket_level_access(UniformBucketLevelAccess::new().set_enabled(false))
                .set_public_access_prevention(""),
        )
        .set_billing(Billing::new().set_requester_pays(false))
        // A policy with no duration: present, but nothing to render.
        .set_retention_policy(RetentionPolicy::new().set_is_locked(true))
        .set_encryption(Encryption::new().set_default_kms_key(""));

    let mapped = map_bucket_detail(&bucket);

    assert_eq!(
        mapped,
        StorageBucketDetail {
            name: "quiet".to_string(),
            location: "us".to_string(),
            location_type: Some("multi-region".to_string()),
            storage_class: "STANDARD".to_string(),
            versioning_enabled: false,
            uniform_bucket_level_access: false,
            public_access_prevention: None,
            requester_pays: false,
            retention_period: None,
            default_kms_key_present: false,
            labels: Vec::new(),
            time_created: None,
            updated: None,
            raw_json: mapped.raw_json.clone(),
        }
    );
}

#[test]
fn map_bucket_detail_reports_false_when_the_blocks_are_absent_entirely() {
    let bucket = Bucket::new()
        .set_name("projects/p/buckets/bare")
        .set_bucket_id("bare");

    let mapped = map_bucket_detail(&bucket);

    assert_eq!(
        mapped,
        StorageBucketDetail {
            name: "bare".to_string(),
            location: String::new(),
            location_type: None,
            storage_class: String::new(),
            versioning_enabled: false,
            uniform_bucket_level_access: false,
            public_access_prevention: None,
            requester_pays: false,
            retention_period: None,
            default_kms_key_present: false,
            labels: Vec::new(),
            time_created: None,
            updated: None,
            raw_json: mapped.raw_json.clone(),
        }
    );
}

#[test]
fn a_bucket_detail_renders_the_retention_duration_in_whole_seconds() {
    let bucket = Bucket::new()
        .set_bucket_id("retained")
        .set_retention_policy(RetentionPolicy::new().set_retention_duration(
            Duration::new(7_776_000, 500_000_000).expect("duration is in range"),
        ));

    assert_eq!(
        map_bucket_detail(&bucket).retention_period,
        Some("7776000".to_string()),
        "sub-second precision must not reach the pane"
    );
}

#[test]
fn a_bucket_detail_carries_every_label_pair() {
    let bucket = Bucket::new()
        .set_bucket_id("labelled")
        .set_labels([("env", "prod"), ("team", "platform")]);

    let mut pairs: Vec<(String, String)> = map_bucket_detail(&bucket)
        .labels
        .into_iter()
        .map(|label| (label.key, label.value))
        .collect();
    // The API hands labels back as a map, so only the set is meaningful.
    pairs.sort();

    assert_eq!(
        pairs,
        vec![
            ("env".to_string(), "prod".to_string()),
            ("team".to_string(), "platform".to_string()),
        ]
    );
}

#[test]
fn a_bucket_detail_reports_the_default_kms_key_as_present_without_naming_it() {
    let bucket = Bucket::new()
        .set_bucket_id("encrypted")
        .set_encryption(Encryption::new().set_default_kms_key(KMS_KEY));

    let mapped = map_bucket_detail(&bucket);

    assert!(mapped.default_kms_key_present);
    assert!(
        !mapped.raw_json.contains("planted-key"),
        "key material reached the pane:\n{}",
        mapped.raw_json
    );
    assert!(
        mapped
            .raw_json
            .contains("\"defaultKmsKey\": \"<redacted>\""),
        "the key was not redacted in place:\n{}",
        mapped.raw_json
    );
}

#[test]
fn a_bucket_detail_raw_json_shows_the_rest_of_the_model() {
    let bucket = Bucket::new()
        .set_bucket_id("documented")
        .set_location("europe-west1")
        .set_rpo("ASYNC_TURBO");

    let raw = map_bucket_detail(&bucket).raw_json;

    assert!(raw.contains("\"bucketId\": \"documented\""), "{raw}");
    assert!(raw.contains("\"rpo\": \"ASYNC_TURBO\""), "{raw}");
    assert!(
        !raw.contains("<redacted>"),
        "a clean bucket was redacted:\n{raw}"
    );
}

// ---------------------------------------------------------------------------
// map_object_detail
// ---------------------------------------------------------------------------

#[test]
fn map_object_detail_carries_every_field_with_its_planted_value() {
    let object = Object::new()
        .set_name("folder/planted.txt")
        .set_size(4_096_i64)
        .set_storage_class("ARCHIVE")
        .set_generation(1_234_567_890_i64)
        .set_update_time(timestamp(UPDATED_SECONDS))
        .set_content_type("application/pdf")
        .set_kms_key(KMS_KEY)
        .set_temporary_hold(true)
        .set_event_based_hold(true)
        .set_retention_expire_time(timestamp(RETAINED_SECONDS))
        .set_checksums(
            ObjectChecksums::new()
                .set_md5_hash(MD5_DIGEST.to_vec())
                .set_crc32c(0xDEAD_BEEF_u32),
        );

    let mapped = map_object_detail(&object);

    assert_eq!(
        mapped,
        StorageObjectDetail {
            key: "folder/planted.txt".to_string(),
            size_bytes: Some(4096),
            storage_class: Some("ARCHIVE".to_string()),
            generation: Some("1234567890".to_string()),
            updated: Some(UPDATED_RENDERED.to_string()),
            content_type: Some("application/pdf".to_string()),
            kms_encrypted: true,
            temporary_hold: true,
            event_based_hold: true,
            retention_expiration: Some(RETAINED_RENDERED.to_string()),
            md5_hash: Some(MD5_BASE64.to_string()),
            crc32c: Some("3q2+7w==".to_string()),
            raw_json: mapped.raw_json.clone(),
        }
    );
}

#[test]
fn map_object_detail_reports_every_flag_off_when_nothing_is_set() {
    let object = Object::new().set_name("plain.txt").set_size(0_i64);

    let mapped = map_object_detail(&object);

    assert_eq!(
        mapped,
        StorageObjectDetail {
            key: "plain.txt".to_string(),
            size_bytes: Some(0),
            storage_class: None,
            generation: None,
            updated: None,
            content_type: None,
            kms_encrypted: false,
            temporary_hold: false,
            event_based_hold: false,
            retention_expiration: None,
            md5_hash: None,
            crc32c: None,
            raw_json: mapped.raw_json.clone(),
        }
    );
}

#[test]
fn an_object_md5_is_the_base64_of_the_digest_bytes() {
    let object = Object::new()
        .set_name("o")
        .set_checksums(ObjectChecksums::new().set_md5_hash(MD5_DIGEST.to_vec()));

    assert_eq!(
        map_object_detail(&object).md5_hash,
        Some(MD5_BASE64.to_string())
    );
}

#[test]
fn an_empty_md5_digest_is_no_hash_rather_than_an_empty_encoding() {
    let object = Object::new().set_name("o").set_checksums(
        ObjectChecksums::new()
            .set_md5_hash(Vec::<u8>::new())
            .set_crc32c(1_u32),
    );

    let mapped = map_object_detail(&object);

    assert_eq!(mapped.md5_hash, None);
    // The crc32c beside it still maps, so this is the filter and not a
    // missing checksums block.
    assert_eq!(mapped.crc32c, Some("AAAAAQ==".to_string()));
}

#[test]
fn an_object_crc32c_is_the_base64_of_its_big_endian_bytes() {
    let object = Object::new()
        .set_name("o")
        .set_checksums(ObjectChecksums::new().set_crc32c(0x0A0B_0C0D_u32));

    assert_eq!(
        map_object_detail(&object).crc32c,
        Some("CgsMDQ==".to_string())
    );
}

#[test]
fn a_crc32c_of_zero_is_still_a_checksum() {
    let present = Object::new()
        .set_name("o")
        .set_checksums(ObjectChecksums::new().set_crc32c(0_u32));
    let absent = Object::new()
        .set_name("o")
        .set_checksums(ObjectChecksums::new().set_md5_hash(MD5_DIGEST.to_vec()));

    assert_eq!(
        map_object_detail(&present).crc32c,
        Some("AAAAAA==".to_string())
    );
    assert_eq!(map_object_detail(&absent).crc32c, None);
}

#[test]
fn an_object_without_a_checksums_block_has_neither_hash() {
    let object = Object::new().set_name("o");

    let mapped = map_object_detail(&object);

    assert_eq!(mapped.md5_hash, None);
    assert_eq!(mapped.crc32c, None);
}

#[test]
fn an_object_detail_renders_the_retention_expiry_the_way_the_cli_did() {
    let retained = Object::new()
        .set_name("o")
        .set_retention_expire_time(timestamp(RETAINED_SECONDS));
    let free = Object::new().set_name("o");

    assert_eq!(
        map_object_detail(&retained).retention_expiration,
        Some(RETAINED_RENDERED.to_string())
    );
    assert_eq!(map_object_detail(&free).retention_expiration, None);
}

#[test]
fn an_object_detail_redacts_its_kms_key_while_still_reporting_encryption() {
    let object = Object::new().set_name("o").set_kms_key(KMS_KEY);

    let mapped = map_object_detail(&object);

    assert!(mapped.kms_encrypted);
    assert!(
        !mapped.raw_json.contains("planted-key"),
        "key material reached the pane:\n{}",
        mapped.raw_json
    );
    assert!(
        mapped.raw_json.contains("\"kmsKey\": \"<redacted>\""),
        "the key was not redacted in place:\n{}",
        mapped.raw_json
    );
}
