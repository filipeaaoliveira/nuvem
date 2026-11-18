use nuvem_domain::gcp::filter::{
    CompiledFilter, FilterMode, FilterQuery, LabelSelector, compile_filter, filter_mode,
    label_filter_field, parse_filter,
};
use nuvem_domain::gcp::resource::ResourceServiceKind;

fn fields(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_string()).collect()
}

fn selector(key: &str, value: &str) -> LabelSelector {
    LabelSelector {
        key: key.to_string(),
        value: value.to_string(),
    }
}

// --- mode dispatch -------------------------------------------------------

#[test]
fn plain_text_is_regex_mode() {
    assert_eq!(filter_mode("api"), FilterMode::Regex);
    assert_eq!(parse_filter("api"), Ok(FilterQuery::Regex("api".into())));
}

#[test]
fn empty_input_is_regex_mode_so_the_chip_has_a_label_while_typing() {
    assert_eq!(filter_mode(""), FilterMode::Regex);
    assert_eq!(filter_mode("   "), FilterMode::Regex);
}

#[test]
fn fuzzy_prefix_dispatches_with_and_without_a_space() {
    assert_eq!(filter_mode("-f api"), FilterMode::Fuzzy);
    assert_eq!(filter_mode("-fapi"), FilterMode::Fuzzy);
    assert_eq!(parse_filter("-f api"), Ok(FilterQuery::Fuzzy("api".into())));
    assert_eq!(parse_filter("-fapi"), Ok(FilterQuery::Fuzzy("api".into())));
}

#[test]
fn label_prefix_dispatches_with_and_without_a_space() {
    assert_eq!(filter_mode("-l env=prod"), FilterMode::Label);
    assert_eq!(filter_mode("-lenv=prod"), FilterMode::Label);
    let expected = Ok(FilterQuery::Label(vec![selector("env", "prod")]));
    assert_eq!(parse_filter("-l env=prod"), expected);
    assert_eq!(parse_filter("-lenv=prod"), expected);
}

#[test]
fn inverse_prefix_dispatches() {
    assert_eq!(filter_mode("!api"), FilterMode::Inverse);
    assert_eq!(filter_mode("! api"), FilterMode::Inverse);
    assert_eq!(parse_filter("!api"), Ok(FilterQuery::Inverse("api".into())));
    assert_eq!(
        parse_filter("! api"),
        Ok(FilterQuery::Inverse("api".into()))
    );
}

#[test]
fn surrounding_whitespace_is_trimmed_before_dispatch() {
    assert_eq!(filter_mode("  -f api  "), FilterMode::Fuzzy);
    assert_eq!(
        parse_filter("  -f api  "),
        Ok(FilterQuery::Fuzzy("api".into()))
    );
    assert_eq!(
        parse_filter("  api  "),
        Ok(FilterQuery::Regex("api".into()))
    );
    assert_eq!(
        parse_filter("  !api  "),
        Ok(FilterQuery::Inverse("api".into()))
    );
    assert_eq!(
        parse_filter("  -l env=prod  "),
        Ok(FilterQuery::Label(vec![selector("env", "prod")]))
    );
}

#[test]
fn mode_labels_are_the_lowercase_chip_names() {
    assert_eq!(FilterMode::Regex.label(), "regex");
    assert_eq!(FilterMode::Fuzzy.label(), "fuzzy");
    assert_eq!(FilterMode::Label.label(), "label");
    assert_eq!(FilterMode::Inverse.label(), "inverse");
}

// --- empty remainders ----------------------------------------------------

#[test]
fn empty_input_and_empty_remainders_mean_no_filter_without_an_error() {
    for input in ["", "   ", "-f", "-l", "!", "  -f  ", "  -l  ", "  !  "] {
        assert_eq!(
            parse_filter(input),
            Ok(FilterQuery::None),
            "input {input:?} should parse to no filter"
        );
        assert!(
            matches!(compile_filter(input), Ok(CompiledFilter::None)),
            "input {input:?} should compile to no filter"
        );
    }
}

#[test]
fn no_filter_keeps_every_row() {
    let filter = compile_filter("").expect("empty input compiles");
    assert!(filter.matches(&fields(&["anything", "at", "all"])));
    assert!(filter.matches(&[]));
}

// --- label parsing -------------------------------------------------------

#[test]
fn label_without_equals_reports_the_expected_shape() {
    let error = parse_filter("-l env").expect_err("missing = is an error");
    assert_eq!(
        error.message,
        "malformed label selector: expected key=value"
    );
    assert_eq!(
        error.to_string(),
        "malformed label selector: expected key=value"
    );
}

#[test]
fn label_with_an_empty_key_is_rejected() {
    let error = parse_filter("-l =prod").expect_err("empty key is an error");
    assert_eq!(error.message, "malformed label selector: empty key");
}

#[test]
fn label_with_an_empty_value_is_rejected() {
    let error = parse_filter("-l env=").expect_err("empty value is an error");
    assert_eq!(error.message, "malformed label selector: empty value");
}

#[test]
fn label_value_keeps_any_further_equals_signs() {
    assert_eq!(
        parse_filter("-l env=a=b"),
        Ok(FilterQuery::Label(vec![selector("env", "a=b")]))
    );
}

#[test]
fn label_splits_multiple_pairs_on_commas() {
    assert_eq!(
        parse_filter("-l env=prod,team=core"),
        Ok(FilterQuery::Label(vec![
            selector("env", "prod"),
            selector("team", "core"),
        ]))
    );
}

#[test]
fn label_pairs_are_trimmed_around_commas() {
    assert_eq!(
        parse_filter("-l env = prod , team = core"),
        Ok(FilterQuery::Label(vec![
            selector("env", "prod"),
            selector("team", "core"),
        ]))
    );
}

#[test]
fn label_never_narrows_client_side_because_it_is_server_authoritative() {
    let filter = compile_filter("-l env=prod").expect("label compiles");
    assert!(matches!(filter, CompiledFilter::Label));
    assert!(filter.matches(&fields(&["web-1", "RUNNING"])));
    assert!(filter.matches(&fields(&["totally-unrelated"])));
    assert!(filter.matches(&[]));
}

// --- regex compilation ---------------------------------------------------

#[test]
fn invalid_regex_reports_a_single_line_message() {
    let error = compile_filter("a(b").expect_err("unbalanced paren is an error");
    assert!(
        error.message.starts_with("invalid regex: "),
        "unexpected message: {}",
        error.message
    );
    assert!(
        !error.message.contains('\n'),
        "message must fit one UI line: {:?}",
        error.message
    );
    // The engine's own first line is the bare header "regex parse error:";
    // the message must carry the cause instead.
    assert!(
        error.message.contains("unclosed group"),
        "message must name the cause: {}",
        error.message
    );
}

#[test]
fn invalid_inverse_regex_reports_the_same_message_shape() {
    let error = compile_filter("!a(b").expect_err("unbalanced paren is an error");
    assert!(
        error.message.starts_with("invalid regex: "),
        "unexpected message: {}",
        error.message
    );
    assert!(
        !error.message.contains('\n'),
        "message must fit one UI line: {:?}",
        error.message
    );
    assert!(
        error.message.contains("unclosed group"),
        "message must name the cause: {}",
        error.message
    );
}

// --- matching ------------------------------------------------------------

#[test]
fn regex_matches_case_insensitively_and_partially() {
    let filter = compile_filter("API").expect("literal compiles");
    assert!(filter.matches(&fields(&["api-dev-1"])));
    assert!(filter.matches(&fields(&["us-central1-a", "API-PROD"])));
    assert!(!filter.matches(&fields(&["db-dev-1"])));
}

#[test]
fn regex_supports_metacharacters() {
    let filter = compile_filter("^api-.*-1$").expect("pattern compiles");
    assert!(filter.matches(&fields(&["api-dev-1"])));
    assert!(!filter.matches(&fields(&["prefix-api-dev-1"])));
}

#[test]
fn inverse_is_exactly_the_negation_of_regex_over_the_same_fields() {
    let regex = compile_filter("api").expect("regex compiles");
    let inverse = compile_filter("!api").expect("inverse compiles");
    for row in [
        fields(&["api-dev-1", "RUNNING"]),
        fields(&["db-dev-1", "RUNNING"]),
        fields(&[]),
        fields(&["", "API"]),
    ] {
        assert_eq!(
            inverse.matches(&row),
            !regex.matches(&row),
            "inverse must negate regex for {row:?}"
        );
    }
}

#[test]
fn fuzzy_matches_a_non_adjacent_subsequence() {
    let filter = compile_filter("-f apd").expect("fuzzy compiles");
    assert!(filter.matches(&fields(&["api-dev-1"])));
}

#[test]
fn fuzzy_is_case_insensitive_in_both_directions() {
    assert!(
        compile_filter("-f APD")
            .expect("fuzzy compiles")
            .matches(&fields(&["api-dev-1"]))
    );
    assert!(
        compile_filter("-f apd")
            .expect("fuzzy compiles")
            .matches(&fields(&["API-DEV-1"]))
    );
}

#[test]
fn fuzzy_rejects_an_out_of_order_needle() {
    let filter = compile_filter("-f dpa").expect("fuzzy compiles");
    assert!(!filter.matches(&fields(&["api-dev-1"])));
}

#[test]
fn fuzzy_matches_when_any_single_field_contains_the_subsequence() {
    let filter = compile_filter("-f apd").expect("fuzzy compiles");
    // Split across two fields the characters are not a subsequence of either.
    assert!(!filter.matches(&fields(&["ap", "dev"])));
    assert!(filter.matches(&fields(&["db-1", "api-dev-1"])));
}

// --- characterisation: prefix edges --------------------------------------

#[test]
fn a_prefix_inside_the_pattern_does_not_dispatch() {
    // Only a *leading* sigil selects a mode; the same characters further in
    // are ordinary pattern text.
    assert_eq!(filter_mode("api-f"), FilterMode::Regex);
    assert_eq!(
        parse_filter("api-f"),
        Ok(FilterQuery::Regex("api-f".into()))
    );
    assert_eq!(filter_mode("a!b"), FilterMode::Regex);
    assert_eq!(parse_filter("a!b"), Ok(FilterQuery::Regex("a!b".into())));
    assert_eq!(filter_mode("api-l"), FilterMode::Regex);
    assert_eq!(
        parse_filter("api-l"),
        Ok(FilterQuery::Regex("api-l".into()))
    );
    // ...and a sigil inside the *remainder* of a prefixed input stays literal.
    assert_eq!(
        parse_filter("-f api-f"),
        Ok(FilterQuery::Fuzzy("api-f".into()))
    );
    assert_eq!(parse_filter("!a!b"), Ok(FilterQuery::Inverse("a!b".into())));
}

#[test]
fn a_pattern_that_merely_starts_with_a_prefix_is_still_dispatched() {
    // The prefix is not required to be followed by a separator, so a pattern
    // that happens to begin with `-f` or `-l` cannot be searched literally.
    assert_eq!(filter_mode("-flag"), FilterMode::Fuzzy);
    assert_eq!(parse_filter("-flag"), Ok(FilterQuery::Fuzzy("lag".into())));
    assert_eq!(filter_mode("-latency"), FilterMode::Label);
    let error = parse_filter("-latency").expect_err("label needs key=value");
    assert_eq!(
        error.message,
        "malformed label selector: expected key=value"
    );
}

#[test]
fn a_prefix_followed_by_only_whitespace_is_no_filter() {
    for input in ["-f\t", "-f \t ", "-l\t", "!\t", "!  \n", "  !\t  "] {
        assert_eq!(
            parse_filter(input),
            Ok(FilterQuery::None),
            "input {input:?} should parse to no filter"
        );
        assert!(
            matches!(compile_filter(input), Ok(CompiledFilter::None)),
            "input {input:?} should compile to no filter"
        );
    }
}

// --- characterisation: fuzzy ---------------------------------------------

#[test]
fn a_fuzzy_needle_is_never_empty_because_an_empty_remainder_is_no_filter() {
    // Nothing can build `CompiledFilter::Fuzzy("")`: the empty remainder is
    // caught before compilation, so the subsequence test never sees an empty
    // needle.
    for input in ["-f", "-f ", "-f   "] {
        assert!(
            matches!(compile_filter(input), Ok(CompiledFilter::None)),
            "input {input:?} should compile to no filter"
        );
    }
}

#[test]
fn a_fuzzy_needle_with_a_repeated_char_needs_the_char_twice() {
    let filter = compile_filter("-f aa").expect("fuzzy compiles");
    assert!(!filter.matches(&fields(&["api"])));
    assert!(filter.matches(&fields(&["aabb"])));
    assert!(filter.matches(&fields(&["a-b-a"])));
}

#[test]
fn fuzzy_matches_non_ascii_needles_case_insensitively() {
    let filter = compile_filter("-f CFÉ").expect("fuzzy compiles");
    assert!(filter.matches(&fields(&["le café"])));
    assert!(
        compile_filter("-f cfé")
            .expect("fuzzy compiles")
            .matches(&fields(&["LE CAFÉ"]))
    );
}

#[test]
fn a_needle_char_that_lowercases_to_two_chars_needs_both_in_the_haystack() {
    // `İ` lowercases to `i` plus a combining dot, so the one char the user
    // typed becomes two the haystack must contain in order.
    let filter = compile_filter("-f İ").expect("fuzzy compiles");
    assert!(filter.matches(&fields(&["İstanbul"])));
    assert!(!filter.matches(&fields(&["Istanbul"])));
}

// --- characterisation: regex ---------------------------------------------

#[test]
fn an_inline_flag_in_the_pattern_overrides_the_implicit_case_insensitivity() {
    // The compiler sets the regex crate's case-insensitive builder flag, and an
    // inline `(?-i)` in the user's own pattern overrides it, so the search
    // becomes case-sensitive.
    let filter = compile_filter("(?-i)API").expect("pattern compiles");
    assert!(filter.matches(&fields(&["API-PROD"])));
    assert!(!filter.matches(&fields(&["api-dev-1"])));
}

#[test]
fn regex_is_case_insensitive_in_both_directions() {
    assert!(
        compile_filter("api")
            .expect("pattern compiles")
            .matches(&fields(&["API-PROD"]))
    );
    assert!(
        compile_filter("API")
            .expect("pattern compiles")
            .matches(&fields(&["api-dev-1"]))
    );
    assert!(
        compile_filter("!api")
            .expect("pattern compiles")
            .matches(&fields(&["DB-PROD"]))
    );
    assert!(
        !compile_filter("!API")
            .expect("pattern compiles")
            .matches(&fields(&["api-dev-1"]))
    );
}

#[test]
fn regex_matches_an_empty_field_string_when_the_pattern_can_match_empty() {
    let empty_capable = compile_filter("a*").expect("pattern compiles");
    assert!(empty_capable.matches(&fields(&[""])));
    let anchored = compile_filter("^$").expect("pattern compiles");
    assert!(anchored.matches(&fields(&[""])));
    let needs_a_char = compile_filter("api").expect("pattern compiles");
    assert!(!needs_a_char.matches(&fields(&[""])));
    // An empty field alongside a matching one still matches.
    assert!(needs_a_char.matches(&fields(&["", "api-dev-1"])));
}

#[test]
fn inverse_rejects_a_row_when_only_one_of_its_fields_matches() {
    let filter = compile_filter("!api").expect("inverse compiles");
    assert!(!filter.matches(&fields(&["api-dev-1", "db-prod"])));
    assert!(!filter.matches(&fields(&["db-prod", "api-dev-1"])));
    assert!(filter.matches(&fields(&["db-prod", "web-1"])));
}

// --- characterisation: label ---------------------------------------------

#[test]
fn label_selectors_keep_the_case_the_user_typed() {
    assert_eq!(
        parse_filter("-lENV=Prod"),
        Ok(FilterQuery::Label(vec![selector("ENV", "Prod")]))
    );
}

// --- label pushdown field ------------------------------------------------

/// Driven off [`ResourceServiceKind::ALL`] rather than a hand-written list of
/// kinds, so adding a service cannot slip past this test the way it once did:
/// the expectation table below is matched exhaustively, so a new kind fails to
/// compile until someone writes down its answer.
///
/// The answer is not free-form. `Some(field)` is a claim that the service
/// narrows `-l k=v` on the server, and `CompiledFilter::Label` keeps every row
/// client-side, so a `Some` whose pushdown is never issued means the filter
/// silently matches everything — with no error, because the state layer only
/// reports one when the field is `None`.
#[test]
fn label_filter_field_maps_every_service_kind() {
    for kind in ResourceServiceKind::ALL {
        let expected = match kind {
            ResourceServiceKind::ComputeInstances => Some("labels"),
            ResourceServiceKind::GkeClusters => Some("resourceLabels"),
            ResourceServiceKind::CloudSql => Some("settings.userLabels"),
            ResourceServiceKind::SecretManagerSecrets => Some("labels"),
            ResourceServiceKind::CloudFunctions => Some("labels"),
            ResourceServiceKind::PersistentDisks => Some("labels"),
            ResourceServiceKind::PubSubTopics => Some("labels"),
            ResourceServiceKind::PubSubSubscriptions => Some("labels"),
            // Neither a service account nor a policy binding carries user
            // labels at all — the IAM API has no field for them — so `-l k=v`
            // is unsupported here rather than silently matching every row.
            // A BigQuery dataset has labels, but `bq` accepts no server-side
            // filter expression, so there is nothing to push down.
            // Neither a VPC network nor a subnet has a `labels` field in the
            // `compute/v1` resource at all, unlike their instances and disks
            // siblings.
            ResourceServiceKind::CloudRunServices
            | ResourceServiceKind::CloudStorage
            | ResourceServiceKind::BigQueryDatasets
            | ResourceServiceKind::IamServiceAccounts
            | ResourceServiceKind::IamPolicy
            | ResourceServiceKind::VpcNetworks
            | ResourceServiceKind::VpcSubnets
            | ResourceServiceKind::FirewallRules => None,
        };
        assert_eq!(
            label_filter_field(kind),
            expected,
            "{kind:?} maps to the wrong label pushdown field"
        );
    }
}
