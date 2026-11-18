//! The BigQuery service layer: the argv nuvem issues to `bq`, and what it
//! makes of the JSON that comes back.
//!
//! Two things here have no analogue in any other service test, and they are
//! the reason this file exists rather than another row in a shared walk. The
//! argv reaches the `bq` binary rather than `gcloud`, and the gcloud
//! configuration travels in the child's *environment* because `bq` has no
//! `--configuration` flag to carry it.

use nuvem::services::gcloud::cli::{
    CommandError, CommandOutput, CommandProgram, CommandRunner, GcloudCommand,
};
use nuvem::services::resources::bigquery::{
    GcloudBigQueryService, parse_datasets_json, parse_table_schema_json, parse_table_show_json,
    parse_tables_json,
};
use nuvem_domain::gcp::configuration::{GcloudConfigurationName, GcloudContext};
use nuvem_domain::gcp::resource::{BigQueryTableKind, TableStat};
use std::cell::RefCell;
use std::rc::Rc;

const DATASETS_JSON: &str = include_str!("fixtures/bigquery_datasets.json");

fn context_for(configuration: &str, project: Option<&str>) -> GcloudContext {
    GcloudContext {
        configuration: GcloudConfigurationName::new(configuration),
        account: None,
        project: project.map(str::to_string),
        region: None,
        zone: None,
    }
}

/// Records every command it is handed and answers each with the same stdout.
///
/// Modelled on `pubsub_service_test.rs`'s `ScriptedRunner`, narrowed to the
/// one call this service makes, and recording the whole `GcloudCommand`
/// rather than only its args — the program and the environment are half of
/// what this file pins.
#[derive(Clone)]
struct RecordingRunner {
    stdout: String,
    seen: Rc<RefCell<Vec<GcloudCommand>>>,
}

impl RecordingRunner {
    fn returning(stdout: &str) -> Self {
        Self {
            stdout: stdout.to_string(),
            seen: Rc::new(RefCell::new(Vec::new())),
        }
    }

    fn last_command(&self) -> GcloudCommand {
        self.seen
            .borrow()
            .last()
            .cloned()
            .expect("the service issued a command")
    }
}

impl CommandRunner for RecordingRunner {
    fn run(&self, command: GcloudCommand) -> Result<CommandOutput, CommandError> {
        self.seen.borrow_mut().push(command);
        Ok(CommandOutput {
            stdout: self.stdout.clone(),
            stderr: String::new(),
        })
    }
}

// --- Parsing ---

#[test]
fn parses_the_captured_public_dataset_listing() {
    let raw = DATASETS_JSON;

    let datasets = parse_datasets_json(raw).expect("fixture parses");

    assert_eq!(datasets.len(), 4);
    let first = &datasets[0];
    assert_eq!(first.dataset_id, "agent_analytic_sample_dataset");
    assert_eq!(first.project_id, "bigquery-public-data");
    assert_eq!(first.location, "US");
}

/// The captured fixture really has this shape: one dataset carries
/// `friendlyName` and the next omits the key entirely. A row missing an
/// optional field must parse, not fail the load.
#[test]
fn a_dataset_missing_every_optional_field_still_parses() {
    let raw =
        r#"[{"datasetReference":{"datasetId":"d","projectId":"p"},"kind":"bigquery#dataset"}]"#;

    let datasets = parse_datasets_json(raw).expect("minimal row parses");

    assert_eq!(datasets.len(), 1);
    assert_eq!(datasets[0].dataset_id, "d");
    assert_eq!(datasets[0].location, "");
    assert!(datasets[0].friendly_name.is_none());
    assert!(datasets[0].labels.is_empty());
}

/// The fixture's first dataset carries `"friendlyName": ""` — the key present
/// and empty, which is not the same document shape as the key being absent and
/// must not become a friendly name of `Some("")` that the preview then paints
/// as a blank line.
#[test]
fn an_empty_friendly_name_reads_as_no_friendly_name() {
    let datasets = parse_datasets_json(DATASETS_JSON).expect("fixture parses");

    assert!(
        datasets[0].friendly_name.is_none(),
        "an empty friendlyName is an absent one, got {:?}",
        datasets[0].friendly_name
    );
}

/// Labels arrive as a JSON object and the row holds them as an ordered list,
/// so the mapping has to pick an order. Sorted by key, because a listing's
/// object order is not something the API promises and a filter matching
/// `k=v` should not depend on it.
#[test]
fn labels_are_carried_across_in_key_order() {
    let datasets = parse_datasets_json(DATASETS_JSON).expect("fixture parses");

    let keys: Vec<&str> = datasets[1]
        .labels
        .iter()
        .map(|label| label.key.as_str())
        .collect();

    assert_eq!(
        keys,
        vec![
            "dg_data_confidentiality",
            "dg_data_context",
            "dg_data_source"
        ]
    );
}

/// An empty project is the ordinary case, not an error: `bq ls -d` against a
/// project with no datasets exits 0 with zero bytes of stdout. Measured on
/// this machine against a real, dataset-free project.
#[test]
fn empty_stdout_is_an_empty_table_not_a_failure() {
    let datasets = parse_datasets_json("").expect("empty stdout is not a parse error");

    assert!(datasets.is_empty());
}

/// Whitespace-only stdout is the same case: the runner trims nothing, and a
/// trailing newline on an otherwise empty document must not reach serde_json.
#[test]
fn whitespace_only_stdout_is_an_empty_table_too() {
    let datasets = parse_datasets_json("\n  \n").expect("blank stdout is not a parse error");

    assert!(datasets.is_empty());
}

#[test]
fn a_document_that_is_not_the_expected_json_fails_the_load() {
    assert!(
        parse_datasets_json("{\"unexpected\": true}").is_err(),
        "an object where a list belongs must fail the load"
    );
}

// --- The command ---

/// Two things this pins that nothing else can: the argv reaches `bq` rather
/// than gcloud, and the configuration travels in the environment because `bq`
/// has no --configuration flag.
#[test]
fn the_load_issues_a_bq_command_carrying_the_configuration_in_its_environment() {
    let runner = RecordingRunner::returning(DATASETS_JSON);
    let service = GcloudBigQueryService::new(runner.clone());

    service
        .load_datasets(&context_for("prod", Some("example-project")))
        .expect("load succeeds");

    let command = runner.last_command();
    assert_eq!(command.program, CommandProgram::Bq);
    assert!(
        command.args.contains(&"ls".to_string()) && command.args.contains(&"-d".to_string()),
        "{:?}",
        command.args
    );
    assert!(
        command
            .args
            .contains(&"--project_id=example-project".to_string()),
        "{:?}",
        command.args
    );
    assert_eq!(
        command.env,
        vec![(
            "CLOUDSDK_ACTIVE_CONFIG_NAME".to_string(),
            "prod".to_string()
        )]
    );
    assert!(
        !command.args.iter().any(|a| a == "--configuration"),
        "bq has no --configuration flag; the configuration goes in the environment"
    );
}

/// `bq` takes its global flags *before* the subcommand, unlike gcloud. An
/// argv that puts `--format=prettyjson` after `ls` is rejected by the real
/// binary, and `contains` alone would not notice.
#[test]
fn the_global_flags_precede_the_subcommand() {
    let runner = RecordingRunner::returning(DATASETS_JSON);
    let service = GcloudBigQueryService::new(runner.clone());

    service
        .load_datasets(&context_for("dev", Some("p")))
        .expect("load succeeds");

    let args = runner.last_command().args;
    let subcommand = args
        .iter()
        .position(|arg| arg == "ls")
        .expect("the argv names the ls subcommand");
    for flag in ["--format=prettyjson", "--project_id=p"] {
        let at = args
            .iter()
            .position(|arg| arg == flag)
            .unwrap_or_else(|| panic!("the argv carries {flag}: {args:?}"));
        assert!(
            at < subcommand,
            "{flag} is a bq global flag and must precede `ls`: {args:?}"
        );
    }
}

/// A context with no project still issues a call: `bq` falls back to the
/// configuration's own project, which is exactly what the environment
/// variable selects. Emitting `--project_id=` with nothing after it would
/// name a project called the empty string instead.
#[test]
fn a_context_without_a_project_omits_the_project_flag_entirely() {
    let runner = RecordingRunner::returning(DATASETS_JSON);
    let service = GcloudBigQueryService::new(runner.clone());

    service
        .load_datasets(&context_for("dev", None))
        .expect("load succeeds");

    let args = runner.last_command().args;
    assert!(
        !args.iter().any(|arg| arg.starts_with("--project_id")),
        "{args:?}"
    );
}

/// The listing is capped like every other list this app issues, so one
/// pathological project cannot stall the table.
#[test]
fn the_listing_is_capped_at_the_shared_resource_limit() {
    let runner = RecordingRunner::returning(DATASETS_JSON);
    let service = GcloudBigQueryService::new(runner.clone());

    service
        .load_datasets(&context_for("dev", Some("p")))
        .expect("load succeeds");

    let args = runner.last_command().args;
    let at = args
        .iter()
        .position(|arg| arg == "-n")
        .expect("the argv carries the -n cap");
    assert_eq!(
        args.get(at + 1).map(String::as_str),
        Some(
            nuvem::services::resources::list_filter::RESOURCE_LIST_LIMIT
                .to_string()
                .as_str()
        ),
        "{args:?}"
    );
}

// --- Tables inside a dataset ---

const TABLES_JSON: &str = include_str!("fixtures/bigquery_tables.json");

#[test]
fn parses_the_recorded_table_listing() {
    let tables = parse_tables_json(TABLES_JSON).expect("fixture parses");

    assert_eq!(tables.len(), 2);
    assert_eq!(tables[0].table_id, "bikeshare_stations");
    assert_eq!(tables[0].dataset_id, "austin_bikeshare");
    assert_eq!(tables[0].kind, BigQueryTableKind::Table);
    assert_eq!(tables[0].created, Some(1_495_683_814_344));
    assert!(tables[0].labels.is_empty());
    // The listing answers none of the three `bq show` stats, so every row
    // leaves this parse unavailable rather than zeroed — a listed table that
    // has not been enriched yet must not read as an empty one.
    for table in &tables {
        assert_eq!(table.rows, TableStat::Unavailable, "{table:?}");
        assert_eq!(table.bytes, TableStat::Unavailable, "{table:?}");
        assert_eq!(table.last_modified, TableStat::Unavailable, "{table:?}");
    }
}

/// The fixture's two rows really differ this way: the second carries a
/// `labels` object of six entries and the first omits the key entirely.
#[test]
fn a_table_carrying_labels_keeps_them_in_key_order() {
    let tables = parse_tables_json(TABLES_JSON).expect("fixture parses");

    let keys: Vec<&str> = tables[1]
        .labels
        .iter()
        .map(|label| label.key.as_str())
        .collect();

    assert_eq!(
        keys,
        vec![
            "dataplex-data-documentation-published-location",
            "dataplex-data-documentation-published-project",
            "dataplex-data-documentation-published-scan",
            "dataplex-dp-published-location",
            "dataplex-dp-published-project",
            "dataplex-dp-published-scan",
        ]
    );
}

/// `creationTime` arrives as a string, so parsing it can fail on a value that
/// is still valid JSON. A table whose timestamp is unusable is still a table,
/// so the failure resolves to an absent time rather than failing the load.
#[test]
fn a_creation_time_that_is_not_a_number_reads_as_absent() {
    let raw = r#"[{"tableReference":{"datasetId":"d","projectId":"p","tableId":"t"},
                   "type":"TABLE","creationTime":"not-a-number"}]"#;

    let tables = parse_tables_json(raw).expect("a malformed timestamp must not fail the load");

    assert_eq!(tables[0].created, None);
    assert_eq!(tables[0].table_id, "t");
}

/// The other two ways the reparse can fail. An empty string and a value past
/// `i64::MAX` both reach `parse` and both are rejected by it, so they land on
/// the same placeholder the absent and the non-numeric cases do.
#[test]
fn an_empty_or_oversized_creation_time_reads_as_absent_too() {
    for millis in ["", "99999999999999999999999999"] {
        let raw = format!(
            r#"[{{"tableReference":{{"datasetId":"d","projectId":"p","tableId":"t"}},"type":"TABLE","creationTime":"{millis}"}}]"#
        );

        let tables = parse_tables_json(&raw).expect("the row still parses");

        assert_eq!(tables[0].created, None, "for creationTime {millis:?}");
        assert_eq!(tables[0].table_id, "t");
    }
}

#[test]
fn a_table_omitting_its_creation_time_reads_as_absent_too() {
    let raw = r#"[{"tableReference":{"datasetId":"d","projectId":"p","tableId":"t"},
                   "type":"TABLE"}]"#;

    let tables = parse_tables_json(raw).expect("minimal row parses");

    assert_eq!(tables[0].created, None);
}

/// BigQuery's `type` is an open set. An unrecognised one is carried verbatim
/// rather than collapsed into `Table`, so the reader is never told the row is
/// an ordinary table when the API said it is something else.
#[test]
fn an_unknown_table_type_is_preserved_rather_than_collapsed() {
    let raw = r#"[{"tableReference":{"datasetId":"d","projectId":"p","tableId":"t"},
                   "type":"SNAPSHOT"}]"#;

    let tables = parse_tables_json(raw).expect("fixture parses");

    assert_eq!(
        tables[0].kind,
        BigQueryTableKind::Other("SNAPSHOT".to_string())
    );
}

#[test]
fn the_known_table_types_each_map_to_their_own_kind() {
    for (raw_type, expected) in [
        ("TABLE", BigQueryTableKind::Table),
        ("VIEW", BigQueryTableKind::View),
        ("MATERIALIZED_VIEW", BigQueryTableKind::MaterializedView),
        ("EXTERNAL", BigQueryTableKind::External),
    ] {
        let raw = format!(
            r#"[{{"tableReference":{{"datasetId":"d","projectId":"p","tableId":"t"}},"type":"{raw_type}"}}]"#
        );

        let tables = parse_tables_json(&raw).expect("row parses");

        assert_eq!(tables[0].kind, expected, "for type {raw_type}");
    }
}

/// An empty dataset exits 0 with zero bytes of stdout, the same shape an
/// empty project gives the datasets listing.
#[test]
fn empty_stdout_is_an_empty_dataset_not_a_failure() {
    assert!(
        parse_tables_json("")
            .expect("empty stdout parses")
            .is_empty()
    );
}

#[test]
fn a_tables_document_that_is_not_the_expected_json_fails_the_load() {
    assert!(
        parse_tables_json("{\"unexpected\": true}").is_err(),
        "an object where a list belongs must fail the load"
    );
}

/// The datasets call and the tables call differ by exactly one flag, and that
/// flag is what decides which of the two the API answers. `bq help ls` gives
/// `-d` as "show datasets described by this identifier", so the flag does not
/// ignore the positional — it rereads it. Under `-d` this call's
/// `austin_bikeshare` would name the identifier whose *datasets* to list, not
/// the dataset whose tables to list.
#[test]
fn the_tables_load_omits_the_datasets_flag_and_names_its_dataset() {
    let runner = RecordingRunner::returning(TABLES_JSON);
    let service = GcloudBigQueryService::new(runner.clone());

    let tables = service
        .load_tables(
            &context_for("prod", Some("example-project")),
            "austin_bikeshare",
        )
        .expect("load succeeds");

    // The rows the runner answered with, not just the argv that asked for
    // them: an argv assertion alone passes for a body that returns an empty
    // list, and the drill-down that consumes these rows is tested against its
    // own fixtures rather than against this parse.
    assert_eq!(tables.len(), 2, "{tables:?}");
    assert_eq!(tables[0].table_id, "bikeshare_stations");

    let command = runner.last_command();
    assert_eq!(command.program, CommandProgram::Bq);
    assert!(
        !command.args.iter().any(|arg| arg == "-d"),
        "{:?}",
        command.args
    );
    assert!(
        command.args.iter().any(|arg| arg == "austin_bikeshare"),
        "{:?}",
        command.args
    );
    assert_eq!(
        command.env,
        vec![(
            "CLOUDSDK_ACTIVE_CONFIG_NAME".to_string(),
            "prod".to_string()
        )]
    );
}

/// The dataset is a positional argument, so its *position* is the whole of
/// what names it: after the subcommand, and after the `-n` cap's value rather
/// than between the flag and the number it takes.
#[test]
fn the_dataset_is_positional_and_follows_the_subcommand_and_its_flags() {
    let runner = RecordingRunner::returning(TABLES_JSON);
    let service = GcloudBigQueryService::new(runner.clone());

    service
        .load_tables(&context_for("dev", Some("p")), "austin_bikeshare")
        .expect("load succeeds");

    let args = runner.last_command().args;
    let subcommand = args
        .iter()
        .position(|arg| arg == "ls")
        .expect("the argv names the ls subcommand");
    let cap = args
        .iter()
        .position(|arg| arg == "-n")
        .expect("the argv carries the -n cap");
    let dataset = args
        .iter()
        .position(|arg| arg == "austin_bikeshare")
        .expect("the argv names the dataset");

    assert!(subcommand < dataset, "{args:?}");
    assert!(cap + 1 < dataset, "{args:?}");
    assert_eq!(
        args.get(cap + 1).map(String::as_str),
        Some(
            nuvem::services::resources::list_filter::RESOURCE_LIST_LIMIT
                .to_string()
                .as_str()
        ),
        "{args:?}"
    );
    for flag in ["--format=prettyjson", "--project_id=p"] {
        let at = args
            .iter()
            .position(|arg| arg == flag)
            .unwrap_or_else(|| panic!("the argv carries {flag}: {args:?}"));
        assert!(at < subcommand, "{flag} must precede `ls`: {args:?}");
    }
}

/// A context without a project omits the flag entirely here for the same
/// reason the datasets call does: `--project_id=` with nothing after it names
/// a project called the empty string.
#[test]
fn the_tables_load_without_a_project_omits_the_project_flag_entirely() {
    let runner = RecordingRunner::returning(TABLES_JSON);
    let service = GcloudBigQueryService::new(runner.clone());

    service
        .load_tables(&context_for("dev", None), "austin_bikeshare")
        .expect("load succeeds");

    let args = runner.last_command().args;
    assert!(
        !args.iter().any(|arg| arg.starts_with("--project_id")),
        "{args:?}"
    );
}

// --- One table's stats, from `bq show` ---
//
// `bq ls <dataset>` does not report row counts, byte sizes or the
// last-modified time, so each of the three comes from a separate `bq show`
// against one table. The fixture below is a transcription of a live `bq show`
// response, and it carries the two facts these tests exist for: BigQuery
// serialises int64 fields as JSON *strings*, and the response includes a
// `schema` this parse does not read — `parse_table_schema_json` reads it, out
// of its own call, and the tests for that are further down this file.

const TABLE_SHOW_JSON: &str = include_str!("fixtures/bigquery_table_show.json");

/// The three values as the wire really spells them: `"101"`, not `101`.
#[test]
fn parses_the_captured_table_show_stats() {
    let stats = parse_table_show_json(TABLE_SHOW_JSON).expect("fixture parses");

    assert_eq!(stats.rows, TableStat::Known(101));
    assert_eq!(stats.bytes, TableStat::Known(15_283));
    assert_eq!(stats.last_modified, TableStat::Known(1_787_306_613_694));
}

/// The distinction the type exists for: a stat the response does not carry is
/// *unavailable*, and unavailable is not zero. A table genuinely holding no
/// rows and a table whose `show` answered without `numRows` must not render
/// the same cell.
#[test]
fn a_show_missing_every_stat_reads_as_unavailable_rather_than_zero() {
    let stats = parse_table_show_json("{}").expect("an object with no stats still parses");

    for stat in [stats.rows, stats.bytes, stats.last_modified] {
        assert_eq!(stat, TableStat::Unavailable);
        assert_ne!(stat, TableStat::Known(0));
        assert_eq!(stat.known(), None);
    }
}

/// A zero really is on the wire for an empty table, and it must survive as a
/// zero rather than being folded into the absent case.
#[test]
fn a_zero_on_the_wire_stays_a_zero() {
    let stats = parse_table_show_json(r#"{"numRows": "0", "numBytes": "0"}"#)
        .expect("a zeroed table parses");

    assert_eq!(stats.rows, TableStat::Known(0));
    assert_eq!(stats.bytes, TableStat::Known(0));
    assert_eq!(stats.rows.known(), Some(0));
    // Only the two the document carries; the third is still absent.
    assert_eq!(stats.last_modified, TableStat::Unavailable);
}

/// Every way a value can be unreadable ends at the same place: a word, an
/// empty string, and a number too large for an `i64` are all unavailable, and
/// none of them is a reason to fail the whole call.
#[test]
fn a_stat_that_is_not_a_number_reads_as_unavailable() {
    let stats = parse_table_show_json(
        r#"{"numRows": "many", "numBytes": "", "lastModifiedTime": "99999999999999999999"}"#,
    )
    .expect("unreadable values do not fail the parse");

    assert_eq!(stats.rows, TableStat::Unavailable);
    assert_eq!(stats.bytes, TableStat::Unavailable);
    assert_eq!(stats.last_modified, TableStat::Unavailable);
}

/// The response carries `schema.fields`, and *this* parse reads none of it:
/// the schema drill-down re-issues its own `show` for the one table it is
/// drilling into, and reads the field list with `parse_table_schema_json`,
/// rather than the stats fan-out retaining every listed row's schema. A schema
/// of a shape no field reader could accept still parses here, which is what
/// "ignored" means.
#[test]
fn the_schema_is_ignored_rather_than_retained() {
    assert!(
        TABLE_SHOW_JSON.contains("\"schema\""),
        "the fixture is the one that carries a schema"
    );

    let stats = parse_table_show_json(r#"{"numRows": "7", "schema": "not an object at all"}"#)
        .expect("a schema this slice never reads cannot fail the parse");

    assert_eq!(stats.rows, TableStat::Known(7));
}

/// A document that is not JSON at all still fails, as it does for both
/// listings: an unreadable *value* is unavailable, an unreadable *response* is
/// an error.
#[test]
fn a_show_document_that_is_not_the_expected_json_fails() {
    assert!(parse_table_show_json("not json").is_err());
}

/// `show`, not `ls`: a different subcommand with a different positional, and
/// `-d` has no business here at all. The table is named fully qualified,
/// `project:dataset.table`, because that is the only spelling `bq show`
/// resolves independently of the configuration's own dataset.
#[test]
fn the_stats_load_shows_one_fully_qualified_table() {
    let runner = RecordingRunner::returning(TABLE_SHOW_JSON);
    let service = GcloudBigQueryService::new(runner.clone());

    let stats = service
        .load_table_stats(
            &context_for("prod", Some("example-project")),
            "austin_bikeshare",
            "bikeshare_stations",
        )
        .expect("load succeeds");

    // The parsed answer, not only the argv that asked for it. Other tests do
    // reach this now -- the demo runner calls it directly and the enrichment
    // tests drive it through the session -- but they assert the numbers a
    // fixture carries, so this is where the parse itself is pinned.
    assert_eq!(stats.rows, TableStat::Known(101));

    let command = runner.last_command();
    assert_eq!(command.program, CommandProgram::Bq);
    assert!(
        command.args.iter().any(|arg| arg == "show"),
        "{:?}",
        command.args
    );
    assert!(
        !command.args.iter().any(|arg| arg == "ls"),
        "{:?}",
        command.args
    );
    assert!(
        !command.args.iter().any(|arg| arg == "-d"),
        "{:?}",
        command.args
    );
    assert!(
        command
            .args
            .iter()
            .any(|arg| arg == "example-project:austin_bikeshare.bikeshare_stations"),
        "{:?}",
        command.args
    );
    assert_eq!(
        command.env,
        vec![(
            "CLOUDSDK_ACTIVE_CONFIG_NAME".to_string(),
            "prod".to_string()
        )]
    );
    assert!(
        !command.args.iter().any(|arg| arg == "--configuration"),
        "bq has no --configuration flag; the configuration goes in the environment"
    );
}

/// `bq` parses its global flags before the subcommand, and rejects the
/// reverse. Pinned positionally here for the same reason both listings pin it.
#[test]
fn the_stats_globals_precede_the_show_subcommand() {
    let runner = RecordingRunner::returning(TABLE_SHOW_JSON);
    let service = GcloudBigQueryService::new(runner.clone());

    service
        .load_table_stats(&context_for("dev", Some("p")), "d", "t")
        .expect("load succeeds");

    let args = runner.last_command().args;
    let subcommand = args
        .iter()
        .position(|arg| arg == "show")
        .expect("the argv names the show subcommand");
    for flag in ["--format=prettyjson", "--project_id=p"] {
        let at = args
            .iter()
            .position(|arg| arg == flag)
            .unwrap_or_else(|| panic!("the argv carries {flag}: {args:?}"));
        assert!(
            at < subcommand,
            "{flag} is a bq global flag and must precede `show`: {args:?}"
        );
    }
    assert_eq!(
        args.iter().position(|arg| arg == "p:d.t"),
        Some(subcommand + 1),
        "the table is the subcommand's positional: {args:?}"
    );
}

/// Without a project there is nothing to qualify with, so the identifier is
/// `dataset.table` and the flag is omitted entirely — `bq` then resolves both
/// against the configuration the environment selects.
#[test]
fn a_stats_load_without_a_project_names_the_table_unqualified() {
    let runner = RecordingRunner::returning(TABLE_SHOW_JSON);
    let service = GcloudBigQueryService::new(runner.clone());

    service
        .load_table_stats(&context_for("dev", None), "austin_bikeshare", "stations")
        .expect("load succeeds");

    let args = runner.last_command().args;
    assert!(
        !args.iter().any(|arg| arg.starts_with("--project_id")),
        "{args:?}"
    );
    assert!(
        args.iter().any(|arg| arg == "austin_bikeshare.stations"),
        "{args:?}"
    );
}

// --- One table's schema, from the same `bq show` ---
//
// The drill-down below `bq ls <dataset>` is a table's column schema, and it
// comes from the very same `bq show` the stats do — a second parse of the same
// document rather than a second call, and a separate parse rather than a wider
// one, because the stats reader deliberately drops `schema` (see
// `the_schema_is_ignored_rather_than_retained`, which still passes untouched).
//
// The fixture is a transcription of a real `bq --format=prettyjson show`
// response shape, nested three levels deep, so the assertions below are about
// flattening rather than about a shape hand-built to be easy to flatten.

const TABLE_SHOW_NESTED_JSON: &str = include_str!("fixtures/bigquery_table_show_nested.json");

/// Every field in order, parents included and deepest level kept.
///
/// The whole sequence, not a count: a flattener that stops one level short, or
/// one that emits a record's children before the record itself, produces a
/// list of a plausible length and the wrong contents. The `event_params`
/// subtree is the case that decides it — `event_params` is a `REPEATED
/// RECORD`, its `value` child is a `RECORD` of its own, and
/// `event_params.value.string_value` sits three levels down.
#[test]
fn the_nested_schema_flattens_to_every_field_in_document_order() {
    let fields = parse_table_schema_json(TABLE_SHOW_NESTED_JSON).expect("fixture parses");

    let rows: Vec<(&str, &str, &str)> = fields
        .iter()
        .map(|field| {
            (
                field.path.as_str(),
                field.field_type.as_str(),
                field.mode.as_str(),
            )
        })
        .collect();

    assert_eq!(
        rows,
        vec![
            ("event_date", "STRING", "NULLABLE"),
            ("event_timestamp", "INTEGER", "NULLABLE"),
            ("event_name", "STRING", "NULLABLE"),
            ("event_params", "RECORD", "REPEATED"),
            ("event_params.key", "STRING", "NULLABLE"),
            ("event_params.value", "RECORD", "NULLABLE"),
            ("event_params.value.string_value", "STRING", "NULLABLE"),
            ("event_params.value.int_value", "INTEGER", "NULLABLE"),
            ("user_pseudo_id", "STRING", "NULLABLE"),
            ("device", "RECORD", "NULLABLE"),
            ("device.category", "STRING", "NULLABLE"),
            ("device.mobile_brand_name", "STRING", "NULLABLE"),
        ],
        "the flattened schema, parents before children and prefixes reset when \
         a subtree ends"
    );
}

/// A record is a row beside its children, not a container replaced by them.
///
/// `event_params` carries the `REPEATED` that says the table holds many of
/// them per row, and no child of it records that. Dropping the parent row
/// would drop the fact.
#[test]
fn a_record_parent_survives_the_flattening_rather_than_being_replaced() {
    let fields = parse_table_schema_json(TABLE_SHOW_NESTED_JSON).expect("fixture parses");

    let parent = fields
        .iter()
        .find(|field| field.path == "event_params")
        .expect("the record parent is a row of its own");
    assert_eq!(parent.field_type, "RECORD");
    assert_eq!(parent.mode, "REPEATED");

    let parent_at = fields
        .iter()
        .position(|field| field.path == "event_params")
        .expect("the parent is in the list");
    let child_at = fields
        .iter()
        .position(|field| field.path == "event_params.key")
        .expect("the child is in the list");
    assert!(
        parent_at < child_at,
        "a parent precedes its children: {parent_at} vs {child_at}"
    );
}

/// The `Option` earns its keep on a real document: `event_name` and
/// `event_params.value.int_value` carry no `description` key at all, and the
/// fields around them do.
#[test]
fn a_field_without_a_description_reads_as_none_rather_than_an_empty_string() {
    let fields = parse_table_schema_json(TABLE_SHOW_NESTED_JSON).expect("fixture parses");
    let description_of = |path: &str| {
        fields
            .iter()
            .find(|field| field.path == path)
            .unwrap_or_else(|| panic!("{path} is in the flattened schema"))
            .description
            .clone()
    };

    assert_eq!(description_of("event_name"), None);
    assert_eq!(description_of("event_params.value.int_value"), None);
    assert_eq!(
        description_of("event_date"),
        Some("The date on which the event was logged.".to_string())
    );
    assert_eq!(
        description_of("event_params.key"),
        Some("The event parameter's key.".to_string()),
        "a nested field keeps its own description, not its parent's"
    );
}

/// An empty description is on the wire as a distinct document from an absent
/// one, and the parse keeps them apart.
#[test]
fn an_empty_description_is_not_folded_into_an_absent_one() {
    let fields = parse_table_schema_json(
        r#"{"schema": {"fields": [
            {"name": "a", "type": "STRING", "mode": "NULLABLE", "description": ""},
            {"name": "b", "type": "STRING", "mode": "NULLABLE"}
        ]}}"#,
    )
    .expect("the document parses");

    assert_eq!(fields[0].description.as_deref(), Some(""));
    assert_eq!(fields[1].description, None);
}

/// `mode` is optional on the wire and its documented default is `NULLABLE`, so
/// a field omitting it is nullable rather than blank. A blank would paint an
/// empty cell that reads as a rendering fault.
#[test]
fn a_field_without_a_mode_reads_as_nullable() {
    let fields =
        parse_table_schema_json(r#"{"schema": {"fields": [{"name": "a", "type": "STRING"}]}}"#)
            .expect("the document parses");

    assert_eq!(fields.len(), 1);
    assert_eq!(fields[0].mode, "NULLABLE");
}

/// The same parse against the flat fixture: three fields, no nesting, no
/// prefixes. One parse serves both shapes rather than a nested document
/// needing a different reader.
#[test]
fn the_flat_fixture_still_parses_to_its_three_fields() {
    let fields = parse_table_schema_json(TABLE_SHOW_JSON).expect("fixture parses");

    let rows: Vec<(&str, &str)> = fields
        .iter()
        .map(|field| (field.path.as_str(), field.field_type.as_str()))
        .collect();
    assert_eq!(
        rows,
        vec![
            ("station_id", "INTEGER"),
            ("name", "STRING"),
            ("status", "STRING"),
        ]
    );
}

/// A response carrying no `schema` at all is an empty schema, not a failure:
/// the document is a table resource and it was read, it simply named no
/// fields. A document that is not JSON is still an error, as it is for the
/// stats parse.
#[test]
fn a_show_without_a_schema_is_empty_and_a_non_document_still_fails() {
    assert!(
        parse_table_schema_json("{}")
            .expect("an object with no schema parses")
            .is_empty()
    );
    assert!(parse_table_schema_json("not json").is_err());
}

/// The schema load issues the same `bq show` the stats load does — globals
/// first, `show` next, and the fully qualified table as its positional.
#[test]
fn the_schema_load_shows_one_fully_qualified_table() {
    let runner = RecordingRunner::returning(TABLE_SHOW_NESTED_JSON);
    let service = GcloudBigQueryService::new(runner.clone());

    let fields = service
        .load_table_schema(
            &context_for("prod", Some("example-project")),
            "ga4_obfuscated_sample_ecommerce",
            "events_20210131",
        )
        .expect("load succeeds");

    assert_eq!(fields.len(), 12, "the flattened fixture reaches the caller");

    let command = runner.last_command();
    assert_eq!(command.program, CommandProgram::Bq);
    let args = command.args;
    let subcommand = args
        .iter()
        .position(|arg| arg == "show")
        .expect("the argv names the show subcommand");
    for flag in ["--format=prettyjson", "--project_id=example-project"] {
        let at = args
            .iter()
            .position(|arg| arg == flag)
            .unwrap_or_else(|| panic!("the argv carries {flag}: {args:?}"));
        assert!(
            at < subcommand,
            "{flag} is a bq global flag and must precede `show`: {args:?}"
        );
    }
    assert_eq!(
        args.iter().position(
            |arg| arg == "example-project:ga4_obfuscated_sample_ecommerce.events_20210131"
        ),
        Some(subcommand + 1),
        "the table is the subcommand's positional: {args:?}"
    );
    assert!(!args.iter().any(|arg| arg == "ls"), "{args:?}");
    assert!(!args.iter().any(|arg| arg == "-d"), "{args:?}");
    assert_eq!(
        command.env,
        vec![(
            "CLOUDSDK_ACTIVE_CONFIG_NAME".to_string(),
            "prod".to_string()
        )],
        "bq has no --configuration flag; the configuration goes in the environment"
    );
}

/// Without a project the identifier stays `dataset.table`, exactly as the
/// stats load leaves it — a leading `:` would name a project called the empty
/// string.
#[test]
fn a_schema_load_without_a_project_names_the_table_unqualified() {
    let runner = RecordingRunner::returning(TABLE_SHOW_NESTED_JSON);
    let service = GcloudBigQueryService::new(runner.clone());

    service
        .load_table_schema(&context_for("dev", None), "austin_bikeshare", "stations")
        .expect("load succeeds");

    let args = runner.last_command().args;
    assert!(
        !args.iter().any(|arg| arg.starts_with("--project_id")),
        "{args:?}"
    );
    assert!(
        args.iter().any(|arg| arg == "austin_bikeshare.stations"),
        "{args:?}"
    );
}

/// The stats parse still drops the schema, and it does so on the nested
/// fixture too: the two parses read one document for different things, and
/// neither has grown into the other.
#[test]
fn the_nested_fixture_still_parses_as_stats_without_its_schema() {
    let stats = parse_table_show_json(TABLE_SHOW_NESTED_JSON).expect("fixture parses");

    assert_eq!(stats.rows, TableStat::Known(4_238));
    assert_eq!(stats.bytes, TableStat::Known(13_208_066));
    assert_eq!(stats.last_modified, TableStat::Known(1_787_306_613_694));
}

/// A runner that refuses the way the real `bq` refuses.
///
/// Every other stub in this file answers `Ok`. This one reproduces the shape
/// captured from the live binary on 2026-08-25: the diagnostic on **stdout**,
/// stderr empty, and a non-zero exit. `gcloud` does the opposite, which is why
/// the three BigQuery surfaces were the ones that went blank and no other
/// service noticed.
struct RefusingRunner {
    stdout: String,
    exit_code: i32,
}

impl CommandRunner for RefusingRunner {
    fn run(&self, _command: GcloudCommand) -> Result<CommandOutput, CommandError> {
        Err(CommandError::Failed {
            stdout: self.stdout.clone(),
            stderr: String::new(),
            exit_code: Some(self.exit_code),
        })
    }
}

/// The refusal each of the three `bq` calls surfaces says which one it was.
///
/// One walk over all three rather than three tests, because the property is
/// that no call is exempt: the argvs differ, the failure path is shared, and a
/// fix applied at one call site would leave the other two blank. Each wording
/// is verbatim from the live binary.
///
/// The assertion is on the whole message, not on a substring like "Not found":
/// the point of the ticket is that the operator gets the *reason*, and a
/// `contains("Not found")` would pass on a detail that had been truncated to
/// those two words.
#[test]
fn every_bq_refusal_carries_the_reason_bq_printed_on_stdout() {
    type Call = fn(&GcloudBigQueryService<RefusingRunner>) -> String;

    let datasets: Call = |service| {
        service
            .load_datasets(&context_for("dev", Some("p")))
            .expect_err("the runner refuses")
            .to_string()
    };
    let tables: Call = |service| {
        service
            .load_tables(&context_for("dev", Some("p")), "no_such_dataset")
            .expect_err("the runner refuses")
            .to_string()
    };
    let schema: Call = |service| {
        service
            .load_table_schema(
                &context_for("dev", Some("p")),
                "austin_bikeshare",
                "no_such_table",
            )
            .expect_err("the runner refuses")
            .to_string()
    };

    let cases: [(&str, Call, i32); 3] = [
        (
            "BigQuery error in ls operation: Project 'projects/p' not found or deleted.",
            datasets,
            1,
        ),
        (
            "BigQuery error in ls operation: Not found: Dataset p:no_such_dataset",
            tables,
            2,
        ),
        (
            "BigQuery error in show operation: Not found: Table p:austin_bikeshare.no_such_table",
            schema,
            2,
        ),
    ];

    for (wording, call, exit_code) in cases {
        let service = GcloudBigQueryService::new(RefusingRunner {
            stdout: format!("{wording}\n"),
            exit_code,
        });

        let rendered = call(&service);

        assert!(
            rendered.contains(wording),
            "the operator must be told why: {rendered:?}"
        );
    }
}
