//! Pins the shape of [`BigQuerySchemaField`] — the one row of a table's
//! schema drill-down.
//!
//! This lives in the domain crate rather than in the root crate's suite for
//! the same reason `bigquery_table_stat_test.rs` does: a `nuvem-domain` type
//! is pinned by `nuvem-domain`'s own test binary, so the crate can be checked
//! on its own. What the *parse* makes of a real `bq show` document is pinned separately, in the
//! root crate's `tests/bigquery_service_test.rs`.

use nuvem_domain::gcp::resource::BigQuerySchemaField;

/// A nested field's `path` is its dotted ancestry, not its leaf name.
///
/// The type carries one flat row per field, parents included, so the leaf name
/// alone would not identify a row: `value` appears under `event_params` and
/// could appear again under any other record in the same table. The dotted
/// path is what makes two rows with the same leaf name distinguishable, and it
/// is also the spelling BigQuery itself accepts in a query.
#[test]
fn a_nested_fields_path_carries_its_whole_ancestry() {
    let leaf = BigQuerySchemaField {
        path: "event_params.value.string_value".to_string(),
        field_type: "STRING".to_string(),
        mode: "NULLABLE".to_string(),
        description: None,
    };

    assert_eq!(leaf.path, "event_params.value.string_value");
    assert_eq!(
        leaf.path.split('.').count(),
        3,
        "three levels of ancestry survive in the path, not just the last one"
    );
    assert_eq!(
        leaf.path.split('.').next_back(),
        Some("string_value"),
        "the leaf name is the last segment, so a renderer can recover it \
         without a second field holding a copy of it"
    );
}

/// A top-level field's path is just its name — no leading separator, and
/// nothing that would make it read as a child of an unnamed root.
#[test]
fn a_top_level_fields_path_is_its_bare_name() {
    let field = BigQuerySchemaField {
        path: "event_date".to_string(),
        field_type: "STRING".to_string(),
        mode: "NULLABLE".to_string(),
        description: Some("The date on which the event was logged.".to_string()),
    };

    assert_eq!(field.path, "event_date");
    assert!(
        !field.path.starts_with('.'),
        "a top-level field is not a child of anything: {}",
        field.path
    );
    assert_eq!(field.path.split('.').count(), 1);
}

/// `description` is an `Option`, and the empty string is not the absence.
///
/// BigQuery lets a field carry `"description": ""`, and that is a different
/// document from one omitting the key. Folding the two together would make the
/// type unable to say which it was handed — so the distinction is kept here,
/// and any decision to render them the same belongs to the renderer.
#[test]
fn an_absent_description_and_an_empty_one_are_different_values() {
    let absent = BigQuerySchemaField {
        path: "event_name".to_string(),
        field_type: "STRING".to_string(),
        mode: "NULLABLE".to_string(),
        description: None,
    };
    let empty = BigQuerySchemaField {
        description: Some(String::new()),
        ..absent.clone()
    };

    assert_ne!(
        absent, empty,
        "an omitted description and an empty one must stay distinguishable"
    );
    assert_eq!(absent.description, None);
    assert_eq!(empty.description.as_deref(), Some(""));
}

/// A record parent and a leaf are the same type, and the parent is a row of
/// its own rather than a container that disappears into its children.
///
/// The drill-down paints one line per field, so `event_params` has to be a
/// value in its own right — carrying its own `RECORD` type and its `REPEATED`
/// mode, which are facts no child of it holds.
#[test]
fn a_record_parent_is_a_row_of_its_own_beside_its_children() {
    let parent = BigQuerySchemaField {
        path: "event_params".to_string(),
        field_type: "RECORD".to_string(),
        mode: "REPEATED".to_string(),
        description: Some("A repeated record of the parameters.".to_string()),
    };
    let child = BigQuerySchemaField {
        path: "event_params.key".to_string(),
        field_type: "STRING".to_string(),
        mode: "NULLABLE".to_string(),
        description: None,
    };

    assert_eq!(parent.field_type, "RECORD");
    assert_eq!(
        parent.mode, "REPEATED",
        "the repetition is the parent's, and none of its children records it"
    );
    assert_eq!(child.mode, "NULLABLE");
    assert!(
        child.path.starts_with(&format!("{}.", parent.path)),
        "the child's path is the parent's path plus a segment: {} under {}",
        child.path,
        parent.path
    );
    assert_ne!(parent, child);
}
