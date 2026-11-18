use nuvem::services::resources::list_filter::{
    RESOURCE_LIST_LIMIT, is_server_expressible, label_mode_needs_gcloud, native_clients_for,
    pushdown_expression, server_filter_expression,
};

/// Stand-in for a service's derived display labels, mirroring the compute
/// service's `OPAQUE_LABELS`.
const OPAQUE: &[&str] = &["running", "stopped", "unknown"];

#[test]
fn accepts_ascii_alphanumeric_dot_underscore_dash_terms() {
    assert!(is_server_expressible("api"));
    assert!(is_server_expressible("api-dev-1"));
    assert!(is_server_expressible("us_central1"));
    assert!(is_server_expressible("e2.medium"));
}

#[test]
fn rejects_empty_or_whitespace_only_terms() {
    assert!(!is_server_expressible(""));
    assert!(!is_server_expressible("   "));
}

#[test]
fn rejects_terms_with_whitespace_inside() {
    assert!(!is_server_expressible("api dev"));
}

#[test]
fn rejects_terms_with_path_separators() {
    assert!(!is_server_expressible("a/b"));
}

#[test]
fn rejects_terms_with_glob_or_regex_metacharacters() {
    assert!(!is_server_expressible("na*me"));
    assert!(!is_server_expressible("na?me"));
    assert!(!is_server_expressible("[abc]"));
}

#[test]
fn rejects_terms_with_quotes() {
    assert!(!is_server_expressible("na\"me"));
}

#[test]
fn rejects_non_ascii_terms() {
    assert!(!is_server_expressible("café"));
}

#[test]
fn builds_one_or_clause_per_field() {
    let expr = server_filter_expression("api", &["name", "zone", "status"], &[]);
    assert_eq!(
        expr,
        Some(r#"name ~ "(?i)api" OR zone ~ "(?i)api" OR status ~ "(?i)api""#.to_string())
    );
}

#[test]
fn trims_term_before_building_expression() {
    let expr = server_filter_expression("  api  ", &["name"], &[]);
    assert_eq!(expr, Some(r#"name ~ "(?i)api""#.to_string()));
}

#[test]
fn returns_none_for_non_expressible_term() {
    assert_eq!(server_filter_expression("na*me", &["name"], &[]), None);
}

#[test]
fn returns_none_for_empty_term() {
    assert_eq!(server_filter_expression("", &["name"], &[]), None);
}

#[test]
fn returns_none_for_empty_fields_slice() {
    assert_eq!(server_filter_expression("api", &[], &[]), None);
}

#[test]
fn gates_term_that_exactly_matches_an_opaque_label() {
    let opaque = &["running", "stopped", "unknown"];
    assert_eq!(server_filter_expression("stopped", &["name"], opaque), None);
}

#[test]
fn gates_term_that_is_a_substring_of_an_opaque_label() {
    let opaque = &["running", "stopped", "unknown"];
    assert_eq!(server_filter_expression("run", &["name"], opaque), None);
}

#[test]
fn does_not_gate_term_that_is_not_a_substring_of_any_opaque_label() {
    let opaque = &["running", "stopped", "unknown"];
    assert_eq!(
        server_filter_expression("api", &["name"], opaque),
        Some(r#"name ~ "(?i)api""#.to_string())
    );
}

#[test]
fn gates_term_that_is_a_substring_of_a_multi_word_opaque_label() {
    assert_eq!(
        server_filter_expression("regional", &["name"], &["ha (regional)"]),
        None
    );
}

#[test]
#[allow(clippy::assertions_on_constants)]
fn resource_list_limit_is_positive() {
    assert!(RESOURCE_LIST_LIMIT > 0);
}

#[test]
fn pushes_down_a_single_label_selector() {
    assert_eq!(
        pushdown_expression("-l env=prod", &["name"], OPAQUE, Some("labels")),
        Some(r#"labels.env = "prod""#.to_string())
    );
}

#[test]
fn joins_multiple_label_selectors_with_and() {
    assert_eq!(
        pushdown_expression("-l env=prod,team=core", &["name"], OPAQUE, Some("labels")),
        Some(r#"labels.env = "prod" AND labels.team = "core""#.to_string())
    );
}

#[test]
fn prefixes_label_selectors_with_the_gke_label_field() {
    assert_eq!(
        pushdown_expression("-l env=prod", &["name"], OPAQUE, Some("resourceLabels")),
        Some(r#"resourceLabels.env = "prod""#.to_string())
    );
}

#[test]
fn prefixes_label_selectors_with_the_cloud_sql_label_field() {
    assert_eq!(
        pushdown_expression(
            "-l env=prod",
            &["name"],
            OPAQUE,
            Some("settings.userLabels")
        ),
        Some(r#"settings.userLabels.env = "prod""#.to_string())
    );
}

#[test]
fn returns_none_for_label_filter_on_a_service_without_a_label_field() {
    assert_eq!(
        pushdown_expression("-l env=prod", &["name"], OPAQUE, None),
        None
    );
}

#[test]
fn returns_none_for_label_key_with_an_unexpressible_character() {
    assert_eq!(
        pushdown_expression("-l en v=prod", &["name"], OPAQUE, Some("labels")),
        None
    );
}

#[test]
fn returns_none_for_label_key_containing_a_dot() {
    // GCP label keys cannot contain `.`, and gcloud reads `labels.a.b` as a
    // nested field path, so the pushed expression would be a nonsense query.
    assert_eq!(
        pushdown_expression("-l a.b=c", &["name"], OPAQUE, Some("labels")),
        None
    );
}

#[test]
fn keeps_pushing_down_a_label_value_containing_a_dot() {
    // Only keys reject `.`; inside the quoted operand it is harmless.
    assert_eq!(
        pushdown_expression("-l env=pr.od", &["name"], OPAQUE, Some("labels")),
        Some(r#"labels.env = "pr.od""#.to_string())
    );
}

#[test]
fn returns_none_for_label_value_with_an_unexpressible_character() {
    assert_eq!(
        pushdown_expression("-l env=pr\"od", &["name"], OPAQUE, Some("labels")),
        None
    );
}

#[test]
fn returns_none_for_fuzzy_filter() {
    assert_eq!(
        pushdown_expression("-f api", &["name"], OPAQUE, Some("labels")),
        None
    );
}

#[test]
fn returns_none_for_inverse_filter() {
    assert_eq!(
        pushdown_expression("!api", &["name"], OPAQUE, Some("labels")),
        None
    );
}

#[test]
fn returns_none_for_empty_filter() {
    assert_eq!(
        pushdown_expression("", &["name"], OPAQUE, Some("labels")),
        None
    );
    assert_eq!(
        pushdown_expression("  ", &["name"], OPAQUE, Some("labels")),
        None
    );
}

#[test]
fn returns_none_for_malformed_label_filter() {
    assert_eq!(
        pushdown_expression("-l env", &["name"], OPAQUE, Some("labels")),
        None
    );
}

#[test]
fn pushes_down_a_plain_term_as_the_phase_one_expression() {
    assert_eq!(
        pushdown_expression("api", &["name", "zone"], OPAQUE, Some("labels")),
        Some(r#"name ~ "(?i)api" OR zone ~ "(?i)api""#.to_string())
    );
}

#[test]
fn gates_a_pattern_that_matches_an_opaque_label_without_being_a_substring() {
    // `r.n` is not a substring of `running`, so Phase 1's substring gate let
    // it through — yet the client-side regex matches the `Running` status
    // cell, so pushing it down would drop that row.
    assert_eq!(
        pushdown_expression("r.n", &["name"], OPAQUE, Some("labels")),
        None
    );
}

#[test]
fn pushes_down_a_pattern_that_matches_no_opaque_label() {
    assert_eq!(
        pushdown_expression("a.i", &["name"], OPAQUE, Some("labels")),
        Some(r#"name ~ "(?i)a.i""#.to_string())
    );
}

#[test]
fn returns_none_for_an_invalid_regex_pattern() {
    assert_eq!(
        pushdown_expression("a(b", &["name"], OPAQUE, Some("labels")),
        None
    );
}

/// Label mode is the one filter the native read path cannot honour: nothing
/// client-side narrows on labels, so the read has to go where the pushdown is.
#[test]
fn label_mode_routes_the_read_through_gcloud() {
    assert!(label_mode_needs_gcloud(Some("-l env=prod")));
    assert!(label_mode_needs_gcloud(Some("-l env=prod,team=core")));
}

/// Every other mode narrows on the client, so the backend that filled the rows
/// does not matter; an empty or malformed selector is not label mode either.
#[test]
fn every_other_mode_stays_on_the_native_path() {
    for applied in [
        None,
        Some(""),
        Some("api"),
        Some("-f api"),
        Some("!api"),
        Some("-l"),
        Some("-l env"),
    ] {
        assert!(!label_mode_needs_gcloud(applied), "{applied:?}");
    }
}

/// The one place the routing decision is made: native clients for every mode
/// but label, and never when there are no clients to begin with.
#[test]
fn native_clients_are_used_for_every_mode_but_label() {
    let clients = 7_u8;
    assert_eq!(native_clients_for(Some(&clients), None), Some(&clients));
    assert_eq!(
        native_clients_for(Some(&clients), Some("api")),
        Some(&clients)
    );
    assert_eq!(
        native_clients_for(Some(&clients), Some("-f api")),
        Some(&clients)
    );
    assert_eq!(
        native_clients_for(Some(&clients), Some("!api")),
        Some(&clients)
    );
    assert_eq!(
        native_clients_for(Some(&clients), Some("-l env=prod")),
        None
    );
    assert_eq!(native_clients_for::<u8>(None, Some("api")), None);
}
