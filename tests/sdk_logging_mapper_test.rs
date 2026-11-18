//! The native Cloud Logging mapper, which a mutation sweep reported as
//! untested: nothing pinned `map_entry`, so a body replaced wholesale with
//! `None` — or a timestamp/label helper replaced with its default — survived.
//!
//! Every field is planted with a distinct non-default value and asserted
//! exactly, so a `Default::default()` body cannot pass; every branch (missing
//! `insertId`, the three payload shapes, present/absent resource, present/absent
//! timestamp, each severity name plus both unknown spellings) is covered from
//! both sides with inputs that produce different output. The filter clauses the
//! two read paths compose are pinned beside them.

use std::collections::BTreeMap;

use google_cloud_logging_v2::model::LogEntry as ApiLogEntry;
use nuvem::services::resources::logging::{
    LogReadOutcome, compose_filter, non_empty, payload_and_message,
};
use nuvem::services::sdk::logging::{filter_to_send, map_entry, take_entry, window_start};
use nuvem_domain::gcp::logging::{
    LogEntry, LogPayload, LogQuery, LogResourceKind, LogSeverity, LogTimeRange, quote,
};

/// The monitored resource is `google_cloud_api::model::MonitoredResource`, a
/// type this crate does not depend on directly. Its default is reached through
/// the field it lives in, so the test never has to name the crate.
fn with_resource(mut entry: ApiLogEntry, kind: &str, labels: &[(&str, &str)]) -> ApiLogEntry {
    let mut resource = entry.resource.take().unwrap_or_default();
    resource.r#type = kind.to_string();
    resource.labels = labels
        .iter()
        .map(|(key, value)| ((*key).to_string(), (*value).to_string()))
        .collect();
    entry.resource = Some(resource);
    entry
}

fn map(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(key, value)| ((*key).to_string(), (*value).to_string()))
        .collect()
}

fn json_fields(pairs: &[(&str, serde_json::Value)]) -> google_cloud_wkt::Struct {
    pairs
        .iter()
        .map(|(key, value)| ((*key).to_string(), value.clone()))
        .collect()
}

fn timestamp(seconds: i64, nanos: i32) -> google_cloud_wkt::Timestamp {
    google_cloud_wkt::Timestamp::new(seconds, nanos).expect("a valid timestamp")
}

/// A populated entry whose every mapped field differs from that field's
/// default, so the whole-body `Default::default()` mutants have nowhere to
/// hide.
fn populated_entry() -> ApiLogEntry {
    with_resource(
        ApiLogEntry::new()
            .set_insert_id("insert-77a")
            .set_log_name("projects/lab-prod/logs/syslog")
            .set_timestamp(timestamp(1_704_164_645, 500_000_000))
            .set_severity("WARNING")
            .set_text_payload("boot finished")
            .set_labels([("tier", "gold"), ("team", "platform")]),
        "gce_instance",
        &[("zone", "europe-west1-b"), ("instance_id", "8123")],
    )
}

#[test]
fn a_populated_entry_maps_every_field_to_its_planted_value() {
    assert_eq!(
        map_entry(&populated_entry()),
        Some(LogEntry {
            insert_id: "insert-77a".into(),
            timestamp: "2024-01-02T03:04:05.500000000Z".into(),
            severity: LogSeverity::Warning,
            resource_type: "gce_instance".into(),
            resource_labels: map(&[("instance_id", "8123"), ("zone", "europe-west1-b")]),
            log_name: "projects/lab-prod/logs/syslog".into(),
            message: "boot finished".into(),
            payload: LogPayload::Text("boot finished".into()),
            labels: map(&[("team", "platform"), ("tier", "gold")]),
        }),
        "every field carries its own planted value, none of them a default"
    );
}

#[test]
fn an_entry_without_an_insert_id_is_dropped_rather_than_mapped() {
    let entry = populated_entry().set_insert_id("");
    assert_eq!(
        map_entry(&entry),
        None,
        "a tail cannot deduplicate an entry with no insertId, so it is dropped \
         even when every other field is present"
    );
    assert!(
        map_entry(&populated_entry()).is_some(),
        "the same entry with an insertId maps, so the drop is about that field alone"
    );
}

#[test]
fn a_single_character_insert_id_is_enough_to_keep_an_entry() {
    let mapped = map_entry(&populated_entry().set_insert_id("x")).expect("a mapped entry");
    assert_eq!(
        mapped.insert_id, "x",
        "only the empty insertId is the drop sentinel"
    );
}

#[test]
fn a_text_payload_becomes_the_text_body_and_its_own_message() {
    let mapped = map_entry(&populated_entry().set_text_payload("connection refused"))
        .expect("a mapped entry");
    assert_eq!(
        mapped.payload,
        LogPayload::Text("connection refused".into())
    );
    assert_eq!(
        mapped.message, "connection refused",
        "the one-line message is the text itself, not the log name"
    );
}

#[test]
fn a_text_payload_message_drops_control_characters_and_widens_tabs() {
    let mapped =
        map_entry(&populated_entry().set_text_payload("a\tb\nc\u{1b}d")).expect("a mapped entry");
    assert_eq!(
        mapped.message, "a bcd",
        "a tab becomes a space and newlines and escapes are dropped, so one \
         entry cannot paint over the rows beneath it"
    );
    assert_eq!(
        mapped.payload,
        LogPayload::Text("a\tb\nc\u{1b}d".into()),
        "the full body is kept verbatim; only the one-line message is sanitized"
    );
}

#[test]
fn a_json_payload_prefers_its_message_field_and_keeps_the_pretty_body() {
    let fields = json_fields(&[
        ("code", serde_json::json!(7)),
        ("message", serde_json::json!("disk full")),
    ]);
    let mapped = map_entry(&populated_entry().set_json_payload(fields)).expect("a mapped entry");
    assert_eq!(
        mapped.message, "disk full",
        "a structured payload's `message` field is the one-line form"
    );
    assert_eq!(
        mapped.payload,
        LogPayload::Json("{\n  \"code\": 7,\n  \"message\": \"disk full\"\n}".into()),
        "the body is the pretty-printed object, not the compact one"
    );
}

#[test]
fn a_json_payload_without_a_message_field_falls_back_to_its_whole_body() {
    let fields = json_fields(&[
        ("code", serde_json::json!(7)),
        ("detail", serde_json::json!("no space")),
    ]);
    let mapped = map_entry(&populated_entry().set_json_payload(fields)).expect("a mapped entry");
    assert_eq!(
        mapped.message, "{\"code\":7,\"detail\":\"no space\"}",
        "with no `message` field the compact body is the one-line form — not \
         the log name, which is only the last resort"
    );
    assert_eq!(
        mapped.payload,
        LogPayload::Json("{\n  \"code\": 7,\n  \"detail\": \"no space\"\n}".into())
    );
}

#[test]
fn a_json_payload_whose_message_is_not_a_string_falls_back_to_the_body() {
    let fields = json_fields(&[("message", serde_json::json!(404))]);
    let mapped = map_entry(&populated_entry().set_json_payload(fields)).expect("a mapped entry");
    assert_eq!(
        mapped.message, "{\"message\":404}",
        "a non-string `message` is not a one-line message"
    );
}

#[test]
fn a_proto_payload_is_left_empty_and_the_message_falls_back_to_the_log_name() {
    let entry = populated_entry()
        .set_log_name("projects/lab-prod/logs/cloudaudit.googleapis.com%2Factivity")
        .set_proto_payload(google_cloud_wkt::Any::default());
    let mapped = map_entry(&entry).expect("a mapped entry");
    assert_eq!(
        mapped.payload,
        LogPayload::Empty,
        "a proto payload is not rendered — the CLI path never showed one either"
    );
    assert_eq!(
        mapped.message, "projects/lab-prod/logs/cloudaudit.googleapis.com%2Factivity",
        "with no renderable body the log name is the last-resort message"
    );
}

#[test]
fn an_entry_with_no_payload_at_all_falls_back_to_the_log_name() {
    let mut entry = populated_entry().set_log_name("projects/lab-prod/logs/stderr");
    entry.payload = None;
    let mapped = map_entry(&entry).expect("a mapped entry");
    assert_eq!(mapped.payload, LogPayload::Empty);
    assert_eq!(mapped.message, "projects/lab-prod/logs/stderr");
}

#[test]
fn the_log_name_is_carried_through_whole_and_unshortened() {
    let mapped = map_entry(
        &populated_entry().set_log_name("projects/lab-prod/logs/run.googleapis.com%2Fstdout"),
    )
    .expect("a mapped entry");
    assert_eq!(
        mapped.log_name, "projects/lab-prod/logs/run.googleapis.com%2Fstdout",
        "the resume filter and the CLI path both spell the log name in full, \
         so the mapper must not trim the `projects/<id>/logs/` prefix"
    );
}

#[test]
fn every_api_severity_name_maps_to_its_own_domain_severity() {
    let cases = [
        ("DEFAULT", LogSeverity::Default),
        ("DEBUG", LogSeverity::Debug),
        ("INFO", LogSeverity::Info),
        ("NOTICE", LogSeverity::Notice),
        ("WARNING", LogSeverity::Warning),
        ("ERROR", LogSeverity::Error),
        ("CRITICAL", LogSeverity::Critical),
        ("ALERT", LogSeverity::Alert),
        ("EMERGENCY", LogSeverity::Emergency),
    ];
    for (name, expected) in cases {
        let mapped = map_entry(&populated_entry().set_severity(name)).expect("a mapped entry");
        assert_eq!(
            mapped.severity, expected,
            "{name} is the API's own spelling of {expected:?}"
        );
    }
}

#[test]
fn an_unrecognised_severity_name_maps_to_default_rather_than_failing() {
    let mapped = map_entry(&populated_entry().set_severity("PANIC")).expect("a mapped entry");
    assert_eq!(
        mapped.severity,
        LogSeverity::Default,
        "a name this client does not know is the unassigned level, and the \
         entry still maps"
    );
}

#[test]
fn a_severity_with_no_api_name_maps_to_default() {
    // An enum deserialized from an unknown *number* has a value but no name,
    // so `name()` is `None` and the mapper falls back to the empty string.
    let mapped = map_entry(&populated_entry().set_severity(451_i32)).expect("a mapped entry");
    assert_eq!(
        mapped.severity,
        LogSeverity::Default,
        "a nameless severity is the unassigned level, not a panic"
    );
}

#[test]
fn a_lowercase_severity_name_still_maps_to_its_level() {
    let mapped = map_entry(&populated_entry().set_severity("warning")).expect("a mapped entry");
    assert_eq!(
        mapped.severity,
        LogSeverity::Warning,
        "the translation upper-cases before matching, so case is not severity"
    );
}

#[test]
fn a_timestamp_is_rendered_rfc3339_with_nanosecond_precision() {
    let mapped = map_entry(&populated_entry().set_timestamp(timestamp(1_704_164_645, 123_456_789)))
        .expect("a mapped entry");
    assert_eq!(
        mapped.timestamp, "2024-01-02T03:04:05.123456789Z",
        "the tail feeds this string back into a `timestamp>=` clause, so the \
         nanoseconds and the `Z` are load-bearing rather than cosmetic"
    );
}

#[test]
fn a_whole_second_timestamp_still_carries_its_nine_fraction_digits() {
    let mapped = map_entry(&populated_entry().set_timestamp(timestamp(1_704_164_645, 0)))
        .expect("a mapped entry");
    assert_eq!(mapped.timestamp, "2024-01-02T03:04:05.000000000Z");
}

#[test]
fn an_entry_without_a_timestamp_renders_an_empty_string() {
    let mut entry = populated_entry();
    entry.timestamp = None;
    let mapped = map_entry(&entry).expect("a mapped entry");
    assert_eq!(
        mapped.timestamp, "",
        "no timestamp is the empty string, which is distinguishable from any \
         rendered one"
    );
}

#[test]
fn a_missing_resource_leaves_the_type_and_its_labels_empty() {
    let mut entry = populated_entry();
    entry.resource = None;
    let mapped = map_entry(&entry).expect("a mapped entry");
    assert_eq!(mapped.resource_type, "");
    assert!(
        mapped.resource_labels.is_empty(),
        "an absent resource contributes no labels"
    );
    assert_eq!(
        mapped.labels,
        map(&[("team", "platform"), ("tier", "gold")]),
        "the entry's own labels are a different field and survive"
    );
}

#[test]
fn resource_labels_keep_every_pair_and_come_back_in_key_order() {
    let entry = with_resource(
        populated_entry(),
        "k8s_container",
        &[
            ("pod_name", "api-7f9"),
            ("cluster_name", "prod-eu"),
            ("namespace_name", "orders"),
        ],
    );
    let mapped = map_entry(&entry).expect("a mapped entry");
    assert_eq!(mapped.resource_type, "k8s_container");
    assert_eq!(
        mapped
            .resource_labels
            .into_iter()
            .collect::<Vec<(String, String)>>(),
        vec![
            ("cluster_name".to_string(), "prod-eu".to_string()),
            ("namespace_name".to_string(), "orders".to_string()),
            ("pod_name".to_string(), "api-7f9".to_string()),
        ],
        "the API hands back an unordered map; the domain row is sorted by key \
         so a detail pane paints the same order every read"
    );
}

#[test]
fn entry_labels_and_resource_labels_do_not_borrow_from_each_other() {
    let entry = with_resource(
        populated_entry().set_labels([("owner", "payments"), ("build", "4821")]),
        "cloud_run_revision",
        &[("service_name", "orders-api"), ("location", "europe-west1")],
    );
    let mapped = map_entry(&entry).expect("a mapped entry");
    assert_eq!(
        mapped.labels,
        map(&[("build", "4821"), ("owner", "payments")]),
        "the entry's labels are the entry's"
    );
    assert_eq!(
        mapped.resource_labels,
        map(&[("location", "europe-west1"), ("service_name", "orders-api")]),
        "and the resource's labels are the resource's"
    );
}

#[test]
fn an_entry_with_no_labels_of_either_kind_maps_to_empty_maps() {
    let entry = with_resource(
        populated_entry().set_labels(Vec::<(String, String)>::new()),
        "global",
        &[],
    );
    let mapped = map_entry(&entry).expect("a mapped entry");
    assert!(mapped.labels.is_empty());
    assert!(mapped.resource_labels.is_empty());
    assert_eq!(
        mapped.resource_type, "global",
        "a resource with no labels is still a resource with a type"
    );
}

#[test]
fn the_shared_payload_derivation_answers_the_same_for_both_backends() {
    assert_eq!(
        payload_and_message(Some("plain"), None, "projects/p/logs/syslog"),
        (LogPayload::Text("plain".into()), "plain".into()),
        "text wins over the log name"
    );
    let json = serde_json::json!({"message": "structured"});
    assert_eq!(
        payload_and_message(Some("plain"), Some(&json), "projects/p/logs/syslog"),
        (LogPayload::Text("plain".into()), "plain".into()),
        "text also wins over a structured payload — precedence, not a merge"
    );
    assert_eq!(
        payload_and_message(None, Some(&json), "projects/p/logs/syslog"),
        (
            LogPayload::Json("{\n  \"message\": \"structured\"\n}".into()),
            "structured".into()
        ),
        "structured wins over the log name"
    );
    assert_eq!(
        payload_and_message(None, None, "projects/p/logs/syslog"),
        (LogPayload::Empty, "projects/p/logs/syslog".into()),
        "the log name is the last resort, and only then"
    );
}

#[test]
fn an_empty_base_filter_is_absent_and_any_other_is_kept() {
    assert_eq!(
        non_empty(String::new()),
        None,
        "a project-wide read contributes no clause rather than an empty conjunct"
    );
    assert_eq!(
        non_empty("resource.type=\"gce_instance\"".to_string()),
        Some("resource.type=\"gce_instance\"".to_string())
    );
    assert_eq!(
        non_empty(" ".to_string()),
        Some(" ".to_string()),
        "only the empty string is the absent sentinel"
    );
}

#[test]
fn the_filter_the_read_path_composes_joins_only_the_clauses_that_apply() {
    let query = LogQuery::Resource(LogResourceKind::ComputeInstance {
        instance_id: "8123".into(),
        instance_name: "web-1".into(),
        zone: "europe-west1-b".into(),
    });
    let composed = compose_filter([
        non_empty(query.render_filter()),
        LogSeverity::Warning.filter_clause(),
        Some(format!("timestamp>={}", quote("2024-01-02T03:04:05Z"))),
    ]);
    assert_eq!(
        composed,
        "resource.type=\"gce_instance\" AND resource.labels.instance_id=\"8123\" \
         AND severity>=WARNING AND timestamp>=\"2024-01-02T03:04:05Z\"",
        "the three clauses the native read builds are joined with ` AND `, in \
         the order the read composes them"
    );
}

#[test]
fn a_project_wide_read_at_the_default_floor_composes_only_the_window() {
    let composed = compose_filter([
        non_empty(LogQuery::ProjectLogs.render_filter()),
        LogSeverity::Default.filter_clause(),
        Some(format!("timestamp>={}", quote("2024-01-02T03:04:05Z"))),
    ]);
    assert_eq!(
        composed, "timestamp>=\"2024-01-02T03:04:05Z\"",
        "an absent resource clause and an unassigned floor are skipped, not \
         emitted as empty conjuncts that would make the filter unparseable"
    );
}

#[test]
fn an_all_absent_clause_set_composes_to_an_empty_filter() {
    assert_eq!(
        compose_filter([None, None, None]),
        "",
        "nothing to narrow by is an empty filter the caller can omit entirely"
    );
    assert_eq!(
        compose_filter([Some("severity>=ERROR".to_string())]),
        "severity>=ERROR",
        "and a single clause carries no separator"
    );
}

// ------------------------------------------- the three decisions a read makes
//
// Each of these was written inline in the request body, where a test cannot
// reach it: a read hits the API and the harnesses hold no clients. A mutation
// sweep turned the window's `-` into `+`, deleted the `!` that decides whether
// a filter is sent at all, and turned the dropped counter's `+=` into `-=` and
// `*=`, and every one of those survived. They are functions now.

/// The window is counted backwards from now, and by the length its own bucket
/// names.
///
/// A `+` here asks the API for entries newer than an hour in the *future*,
/// which returns nothing and reads on screen as a project with no logs.
#[test]
fn the_recent_window_starts_that_many_minutes_before_now_not_after() {
    let now = chrono::DateTime::parse_from_rfc3339("2026-09-11T12:00:00Z")
        .expect("a valid timestamp")
        .with_timezone(&chrono::Utc);

    for (range, expected) in [
        (LogTimeRange::OneMinute, "2026-09-11T11:59:00+00:00"),
        (LogTimeRange::ThirtyMinutes, "2026-09-11T11:30:00+00:00"),
        (LogTimeRange::OneHour, "2026-09-11T11:00:00+00:00"),
        (LogTimeRange::SixHours, "2026-09-11T06:00:00+00:00"),
        (LogTimeRange::TwentyFourHours, "2026-09-10T12:00:00+00:00"),
    ] {
        let start = window_start(now, range);
        assert_eq!(start.to_rfc3339(), expected, "window {}", range.label());
        assert!(
            start < now,
            "a window of history ends at now and starts before it"
        );
    }
}

/// An empty filter is sent as no filter, not as an empty one.
#[test]
fn a_composed_filter_is_sent_only_when_it_says_something() {
    assert_eq!(filter_to_send(""), None, "nothing to narrow by");
    assert_eq!(
        filter_to_send("severity>=ERROR"),
        Some("severity>=ERROR"),
        "a real filter is sent verbatim"
    );
    assert_eq!(
        filter_to_send(" "),
        Some(" "),
        "only an empty string is nothing; whitespace is the caller's business"
    );
}

/// A mapped entry lands in the page; an unmappable one is counted.
///
/// The count is what the viewer reports, so it is the difference between a
/// page that is short and a page that is short and says so.
#[test]
fn an_unmappable_entry_is_counted_rather_than_passed_over_in_silence() {
    let entry = |insert: &str| {
        map_entry(
            &ApiLogEntry::new()
                .set_insert_id(insert)
                .set_log_name("projects/p/logs/run.googleapis.com%2Fstdout"),
        )
    };

    let mut outcome = LogReadOutcome::default();
    take_entry(&mut outcome, entry("first"));
    assert_eq!(outcome.entries.len(), 1);
    assert_eq!(outcome.dropped, 0, "a readable entry is not a drop");

    take_entry(&mut outcome, None);
    assert_eq!(outcome.entries.len(), 1, "a dropped entry adds no row");
    assert_eq!(outcome.dropped, 1);

    take_entry(&mut outcome, None);
    take_entry(&mut outcome, entry("second"));
    assert_eq!(
        (outcome.entries.len(), outcome.dropped),
        (2, 2),
        "each kind accumulates on its own side, one at a time"
    );
}
