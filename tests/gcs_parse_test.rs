//! Cloud Storage parsing, pinned against the shape `gcloud storage` actually
//! emits.
//!
//! The fixtures here are gcloud's own schema — a flat top-level array for a
//! listing, `snake_case` keys, and `size` as a number — not the Cloud Storage
//! JSON API schema. Modelling the API instead is what left the object browser
//! unable to open any bucket while every fixture-backed test stayed green, so
//! a fixture that drifts back to `{"items": [...]}` is a regression in itself.

use nuvem::services::resources::storage::{
    parse_bucket_detail_json, parse_buckets_json, parse_object_detail_json, parse_object_page_json,
};
use nuvem_domain::gcp::text::sanitize_display_text;

#[test]
fn sanitize_replaces_control_and_escape_sequences() {
    let hostile = "evil\r\n\x1b[2Jname\x07\u{7f}ok";
    let cleaned = sanitize_display_text(hostile);
    assert!(!cleaned.chars().any(|c| c.is_control()));
    assert!(!cleaned.contains('\u{1b}'));
    assert!(cleaned.contains("evil"));
    assert!(cleaned.contains("name"));
    assert!(cleaned.contains("ok"));
    // Non-control Unicode preserved
    assert_eq!(sanitize_display_text("café-☕-测试"), "café-☕-测试");
}

/// The defect this module exists to keep fixed: a listing arrives as a JSON
/// array, and the parser used to expect an object with an `items` field. The
/// failure was not a missing field but a type error two lines into the
/// payload — "invalid type: map, expected a sequence" — because a struct can
/// also be deserialized from a positional sequence.
#[test]
fn an_object_listing_is_a_top_level_array_not_an_items_envelope() {
    let listing = r#"[{"name":"a.txt","size":1},{"name":"b.txt","size":2}]"#;
    let page = parse_object_page_json(listing, "").expect("gcloud emits a bare array");
    assert_eq!(page.objects.len(), 2);

    // The API envelope is not what gcloud sends, and must not silently parse.
    let envelope = r#"{"items":[{"name":"a.txt"}],"prefixes":[]}"#;
    assert!(
        parse_object_page_json(envelope, "").is_err(),
        "the JSON API envelope must not be accepted as gcloud output"
    );
}

#[test]
fn parses_gcs_bucket_rows() {
    let rows = parse_buckets_json(include_str!("fixtures/gcs_buckets.json"))
        .expect("bucket fixture should parse");

    assert_eq!(rows.len(), 3);

    let assets = &rows[0];
    assert_eq!(assets.name, "alpha-bucket");
    assert_eq!(assets.location, "US");
    assert_eq!(assets.location_type.as_deref(), Some("multi-region"));
    assert_eq!(assets.storage_class, "STANDARD");
    assert!(assets.versioning_enabled);
    assert!(assets.uniform_bucket_level_access);
    assert_eq!(assets.public_access_prevention.as_deref(), Some("enforced"));
    assert_eq!(assets.time_created.as_deref(), Some("2024-01-02T03:04:05Z"));
    assert_eq!(assets.updated.as_deref(), Some("2024-02-03T04:05:06Z"));

    let logs = &rows[1];
    assert_eq!(logs.name, "beta-bucket");
    assert_eq!(logs.storage_class, "NEARLINE");
    assert!(!logs.versioning_enabled);
    assert!(!logs.uniform_bucket_level_access);
    assert_eq!(logs.public_access_prevention.as_deref(), Some("inherited"));

    // Every optional absent: gcloud omits a field rather than sending null.
    let archive = &rows[2];
    assert_eq!(archive.name, "gamma-bucket");
    assert_eq!(archive.location, "ASIA");
    assert_eq!(archive.location_type, None);
    assert_eq!(archive.storage_class, "COLDLINE");
    assert!(!archive.versioning_enabled);
    assert!(!archive.uniform_bucket_level_access);
    assert_eq!(archive.public_access_prevention, None);
    assert_eq!(archive.time_created, None);
    assert_eq!(archive.updated, None);
}

#[test]
fn parses_gcs_bucket_detail() {
    let detail = parse_bucket_detail_json(include_str!("fixtures/gcs_bucket_detail.json"))
        .expect("bucket detail fixture should parse");

    assert_eq!(detail.name, "alpha-bucket");
    assert_eq!(detail.location, "US");
    assert_eq!(detail.storage_class, "STANDARD");
    assert!(detail.versioning_enabled);
    assert!(detail.uniform_bucket_level_access);
    assert!(detail.requester_pays);
    // Nested policy dictionaries keep the API's camelCase inside gcloud's
    // snake_case resource, so this reads `retentionPeriod`.
    assert_eq!(detail.retention_period.as_deref(), Some("2592000"));
    assert!(detail.default_kms_key_present);
    assert!(
        detail
            .labels
            .iter()
            .any(|entry| entry.key == "team" && entry.value == "platform")
    );

    // No raw key material should ever leak into any struct field.
    assert!(!detail.raw_json.contains("cryptoKeys"));
    assert!(!format!("{detail:?}").contains("cryptoKeys"));
}

#[test]
fn parses_gcs_object_page() {
    let page = parse_object_page_json(include_str!("fixtures/gcs_objects.json"), "")
        .expect("object listing fixture should parse");

    // Folders are derived from the recursive listing, deduplicated and sorted.
    let folders: Vec<&str> = page.prefixes.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(folders, vec!["docs/", "photos/", "reports/"]);

    // Only the objects sitting at this level, never the nested ones.
    let keys: Vec<&str> = page.objects.iter().map(|o| o.key.as_str()).collect();
    assert_eq!(
        keys,
        vec![
            "top-level.txt",
            "name.bin",
            "temp-hold.txt",
            "event-hold.txt",
            "retained.txt",
        ]
    );

    // gcloud never sends a page token, so a listing is always the last page.
    assert_eq!(page.next_page_token, None);

    let top = &page.objects[0];
    assert_eq!(top.size_bytes, Some(128));
    assert_eq!(top.storage_class.as_deref(), Some("STANDARD"));
    assert_eq!(top.updated.as_deref(), Some("2024-03-01T10:00:00Z"));
    assert!(!top.kms_encrypted);
    assert!(!top.temporary_hold);

    assert!(page.objects[1].kms_encrypted);
    assert!(page.objects[2].temporary_hold);
    assert!(page.objects[3].event_based_hold);
    assert!(page.objects[4].has_retention);
}

/// Entering a folder lists it recursively and groups one level down again,
/// with names left absolute for the browser to strip.
#[test]
fn groups_one_level_below_a_nested_prefix() {
    let page = parse_object_page_json(include_str!("fixtures/gcs_objects.json"), "reports/")
        .expect("nested listing should parse");

    assert_eq!(page.prefixes.len(), 1);
    assert_eq!(page.prefixes[0].name, "reports/2024/");
    assert!(page.objects.is_empty());
}

/// A folder's zero-byte placeholder has the prefix as its whole name; it is a
/// directory marker, not a row.
#[test]
fn the_placeholder_object_for_a_folder_is_not_listed_as_a_row() {
    let listing = r#"[{"name":"docs/"},{"name":"docs/read.txt"}]"#;
    let page = parse_object_page_json(listing, "docs/").expect("listing should parse");

    assert!(page.prefixes.is_empty());
    let keys: Vec<&str> = page.objects.iter().map(|o| o.key.as_str()).collect();
    assert_eq!(keys, vec!["docs/read.txt"]);
}

#[test]
fn parses_gcs_object_detail() {
    let detail = parse_object_detail_json(include_str!("fixtures/gcs_object_detail.json"))
        .expect("object detail fixture should parse");

    assert_eq!(detail.key, "reports/2024/mañana-测试.txt");
    assert_eq!(detail.size_bytes, Some(1024));
    assert_eq!(detail.md5_hash.as_deref(), Some("XUFAKrxLKna5cZ2REBfFkg=="));
    assert_eq!(detail.crc32c.as_deref(), Some("AAAAAA=="));
    assert!(detail.kms_encrypted);
    assert_eq!(
        detail.retention_expiration.as_deref(),
        Some("2025-06-01T00:00:00Z")
    );

    // No raw key material should ever leak into any struct field.
    assert!(!detail.raw_json.contains("cryptoKeys"));
    assert!(!format!("{detail:?}").contains("cryptoKeys"));
}

#[test]
fn parses_gcs_missing_optional_fields_without_failure() {
    let rows = parse_buckets_json(r#"[{"name":"minimal","location":"us-central1"}]"#)
        .expect("minimal bucket row should parse");

    assert_eq!(rows[0].name, "minimal");
    assert_eq!(rows[0].location, "us-central1");
    assert_eq!(rows[0].location_type, None);
    assert_eq!(rows[0].storage_class, "");
    assert!(!rows[0].versioning_enabled);
    assert!(!rows[0].uniform_bucket_level_access);
    assert_eq!(rows[0].public_access_prevention, None);
    assert_eq!(rows[0].time_created, None);
    assert_eq!(rows[0].updated, None);

    let page =
        parse_object_page_json(r#"[{"name":"minimal.txt"}]"#, "").expect("minimal page parses");
    assert_eq!(page.prefixes.len(), 0);
    assert_eq!(page.objects.len(), 1);
    let obj = &page.objects[0];
    assert_eq!(obj.key, "minimal.txt");
    assert_eq!(obj.size_bytes, None);
    assert_eq!(obj.storage_class, None);
    assert_eq!(obj.generation, None);
    assert_eq!(obj.updated, None);
    assert_eq!(obj.content_type, None);
    assert!(!obj.kms_encrypted);
    assert!(!obj.temporary_hold);
    assert!(!obj.event_based_hold);
    assert!(!obj.has_retention);
    assert_eq!(page.next_page_token, None);
}

#[test]
fn parses_gcs_bucket_detail_missing_optional_fields() {
    let detail = parse_bucket_detail_json(
        r#"{"name":"minimal-bucket","location":"us-central1","default_storage_class":"STANDARD"}"#,
    )
    .expect("minimal bucket detail should parse");

    assert_eq!(detail.name, "minimal-bucket");
    assert_eq!(detail.location, "us-central1");
    assert_eq!(detail.storage_class, "STANDARD");
    assert!(!detail.requester_pays);
    assert_eq!(detail.retention_period, None);
    assert!(!detail.default_kms_key_present);
    assert!(!detail.versioning_enabled);
    assert!(!detail.uniform_bucket_level_access);
    assert!(!detail.raw_json.is_empty());
}

#[test]
fn parses_gcs_object_detail_missing_optional_fields() {
    let detail = parse_object_detail_json(r#"{"name":"solo.txt"}"#)
        .expect("minimal object detail should parse");

    assert_eq!(detail.key, "solo.txt");
    assert!(!detail.kms_encrypted);
    assert!(!detail.temporary_hold);
    assert!(!detail.event_based_hold);
    assert_eq!(detail.retention_expiration, None);
    assert_eq!(detail.md5_hash, None);
    assert_eq!(detail.crc32c, None);
    assert_eq!(detail.size_bytes, None);
}

/// Every spelling of the KMS key field redacts, including the one the
/// generated client emits.
///
/// The pass used to hold a list of literal names. gcloud's `default_kms_key`
/// and the API's `defaultKmsKeyName` were both on it; the client's
/// `defaultKmsKey` was not, so serializing a model would have put key material
/// straight into the JSON tab. This walks the spellings so the next backend is
/// covered by the assertion rather than by luck.
#[test]
fn kms_key_material_is_redacted_in_every_spelling() {
    for spelling in [
        "default_kms_key",
        "defaultKmsKey",
        "defaultKmsKeyName",
        "kms_key",
        "kmsKey",
        "kmsKeyName",
    ] {
        let json = format!(
            r#"{{"name":"alpha-bucket","location":"US","storage_class":"STANDARD",
                 "encryption":{{"{spelling}":"projects/p/locations/l/keyRings/r/cryptoKeys/k"}}}}"#
        );
        let detail = parse_bucket_detail_json(&json)
            .unwrap_or_else(|error| panic!("{spelling} fixture should parse: {error}"));
        assert!(
            !detail.raw_json.contains("cryptoKeys/k"),
            "{spelling}: key material survived into raw_json:\n{}",
            detail.raw_json
        );
        assert!(
            detail.raw_json.contains("<redacted>"),
            "{spelling}: nothing was redacted:\n{}",
            detail.raw_json
        );
    }
}
