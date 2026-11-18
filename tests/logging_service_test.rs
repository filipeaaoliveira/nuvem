use std::cell::RefCell;

use nuvem::services::gcloud::cli::{CommandError, CommandOutput, CommandRunner, GcloudCommand};
use nuvem::services::resources::logging::{GcloudLoggingService, parse_log_entries_json};
use nuvem_domain::gcp::configuration::{GcloudConfigurationName, GcloudContext};
use nuvem_domain::gcp::logging::{
    LogCursor, LogPayload, LogQuery, LogResourceKind, LogSeverity, LogTimeRange,
};

#[derive(Default)]
struct FakeRunner {
    outputs: RefCell<Vec<Result<CommandOutput, CommandError>>>,
    commands: RefCell<Vec<GcloudCommand>>,
}

impl CommandRunner for FakeRunner {
    fn run(&self, command: GcloudCommand) -> Result<CommandOutput, CommandError> {
        self.commands.borrow_mut().push(command);
        self.outputs.borrow_mut().remove(0)
    }
}

fn context() -> GcloudContext {
    GcloudContext {
        configuration: GcloudConfigurationName::new("dev"),
        account: None,
        project: Some("dev-project".to_string()),
        region: None,
        zone: None,
    }
}

fn args(values: &[&str]) -> Vec<String> {
    values.iter().map(|v| v.to_string()).collect()
}

#[test]
fn read_recent_runs_native_command_and_parses_entries() {
    let runner = FakeRunner::default();
    runner.outputs.borrow_mut().push(Ok(CommandOutput {
        stdout: include_str!("fixtures/log_entries.json").to_string(),
        stderr: String::new(),
    }));
    let service = GcloudLoggingService::new(runner);

    let outcome = service
        .read_recent(
            &context(),
            &LogQuery::ProjectLogs,
            LogSeverity::Default,
            LogTimeRange::OneHour,
            200,
        )
        .expect("logs should load");

    assert_eq!(outcome.entries.len(), 2);
    assert_eq!(outcome.dropped, 0);
    // gcloud returns `--order=desc` (newest-first); the service normalizes to
    // ascending, so `first()` is the OLDEST entry and `last()` is the newest.
    let first = &outcome.entries[0];
    let last = &outcome.entries[1];
    assert_eq!(first.insert_id, "aaa-1");
    assert_eq!(first.severity, LogSeverity::Error);
    assert_eq!(first.timestamp, "2026-07-19T10:00:00.000000000Z");
    assert_eq!(last.insert_id, "bbb-2");
    assert_eq!(last.timestamp, "2026-07-19T10:00:01.000000000Z");
    // Control chars sanitized out of the compact message.
    assert!(!first.message.contains('\u{1b}'));
    assert!(matches!(last.payload, LogPayload::Json(_)));

    let commands = service.runner().commands.borrow();
    assert_eq!(
        commands[0].args,
        args(&[
            "--configuration",
            "dev",
            "logging",
            "read",
            "--format=json",
            "--order=desc",
            "--limit",
            "200",
            "--freshness=1h",
            "--project",
            "dev-project",
        ])
    );
}

/// The cursor's timestamp is interpolated into the filter expression, and it
/// arrives from parsed gcloud JSON, not from anything this process wrote. A
/// quote inside it must be escaped like every other interpolated operand —
/// this was the one clause composed without `quote()`, so the hostile shape
/// is pinned here.
#[test]
fn a_quote_in_the_cursor_timestamp_cannot_escape_the_filter_expression() {
    let runner = FakeRunner::default();
    runner.outputs.borrow_mut().push(Ok(CommandOutput {
        stdout: "[]".to_string(),
        stderr: String::new(),
    }));
    let service = GcloudLoggingService::new(runner);
    let cursor = LogCursor {
        timestamp: "evil\" OR severity>=EMERGENCY OR timestamp>=\"".to_string(),
        insert_ids: Vec::new(),
    };

    service
        .read_since(
            &context(),
            &LogQuery::ProjectLogs,
            LogSeverity::Default,
            &cursor,
            200,
        )
        .expect("tail should load");

    let commands = service.runner().commands.borrow();
    let clause = commands[0]
        .args
        .iter()
        .find(|arg| arg.starts_with("timestamp>="))
        .expect("the cursor clause is present");
    assert_eq!(
        clause, "timestamp>=\"evil\\\" OR severity>=EMERGENCY OR timestamp>=\\\"\"",
        "every quote in the operand must arrive escaped"
    );
}

#[test]
fn read_since_includes_timestamp_cursor_clause_ascending() {
    let runner = FakeRunner::default();
    runner.outputs.borrow_mut().push(Ok(CommandOutput {
        stdout: "[]".to_string(),
        stderr: String::new(),
    }));
    let service = GcloudLoggingService::new(runner);
    let cursor = LogCursor {
        timestamp: "2026-07-19T10:00:01.000000000Z".to_string(),
        insert_ids: vec!["bbb-2".to_string()],
    };

    let outcome = service
        .read_since(
            &context(),
            &LogQuery::ProjectLogs,
            LogSeverity::Default,
            &cursor,
            200,
        )
        .expect("tail should load");
    assert_eq!(outcome.entries.len(), 0);

    let commands = service.runner().commands.borrow();
    assert_eq!(
        commands[0].args,
        args(&[
            "--configuration",
            "dev",
            "logging",
            "read",
            "timestamp>=\"2026-07-19T10:00:01.000000000Z\"",
            "--format=json",
            "--order=asc",
            "--limit",
            "200",
            "--project",
            "dev-project",
        ])
    );
}

fn empty_json_runner() -> FakeRunner {
    let runner = FakeRunner::default();
    runner.outputs.borrow_mut().push(Ok(CommandOutput {
        stdout: "[]".to_string(),
        stderr: String::new(),
    }));
    runner
}

fn instance_query() -> LogQuery {
    LogQuery::Resource(LogResourceKind::ComputeInstance {
        instance_id: "1001".to_string(),
        instance_name: "api-dev-1".to_string(),
        zone: "europe-west1-b".to_string(),
    })
}

fn severity_clauses(args: &[String]) -> usize {
    args.iter().filter(|arg| arg.contains("severity>=")).count()
}

#[test]
fn read_recent_default_floor_leaves_argv_untouched() {
    // `LogSeverity::Default` is the "no floor" sentinel: the argv must stay
    // byte-identical to a read that predates severity filtering.
    let service = GcloudLoggingService::new(empty_json_runner());
    service
        .read_recent(
            &context(),
            &instance_query(),
            LogSeverity::Default,
            LogTimeRange::OneHour,
            200,
        )
        .expect("logs should load");

    let commands = service.runner().commands.borrow();
    assert_eq!(
        commands[0].args,
        args(&[
            "--configuration",
            "dev",
            "logging",
            "read",
            "resource.type=\"gce_instance\" AND resource.labels.instance_id=\"1001\"",
            "--format=json",
            "--order=desc",
            "--limit",
            "200",
            "--freshness=1h",
            "--project",
            "dev-project",
        ])
    );
}

#[test]
fn read_recent_project_wide_floor_becomes_the_whole_filter() {
    // Project-wide reads render an empty base filter, so the floor alone has
    // to produce a well-formed single positional filter argument.
    let service = GcloudLoggingService::new(empty_json_runner());
    service
        .read_recent(
            &context(),
            &LogQuery::ProjectLogs,
            LogSeverity::Error,
            LogTimeRange::OneHour,
            200,
        )
        .expect("logs should load");

    let commands = service.runner().commands.borrow();
    assert_eq!(
        commands[0].args,
        args(&[
            "--configuration",
            "dev",
            "logging",
            "read",
            "severity>=ERROR",
            "--format=json",
            "--order=desc",
            "--limit",
            "200",
            "--freshness=1h",
            "--project",
            "dev-project",
        ])
    );
    assert_eq!(severity_clauses(&commands[0].args), 1);
}

#[test]
fn read_recent_resource_scoped_floor_is_anded_onto_the_base_filter() {
    let service = GcloudLoggingService::new(empty_json_runner());
    service
        .read_recent(
            &context(),
            &instance_query(),
            LogSeverity::Error,
            LogTimeRange::OneHour,
            200,
        )
        .expect("logs should load");

    let commands = service.runner().commands.borrow();
    assert_eq!(
        commands[0].args,
        args(&[
            "--configuration",
            "dev",
            "logging",
            "read",
            "resource.type=\"gce_instance\" AND resource.labels.instance_id=\"1001\" AND severity>=ERROR",
            "--format=json",
            "--order=desc",
            "--limit",
            "200",
            "--freshness=1h",
            "--project",
            "dev-project",
        ])
    );
    assert_eq!(severity_clauses(&commands[0].args), 1);
}

#[test]
fn read_since_composes_base_floor_and_timestamp_clauses() {
    let service = GcloudLoggingService::new(empty_json_runner());
    let cursor = LogCursor {
        timestamp: "2026-07-19T10:00:01.000000000Z".to_string(),
        insert_ids: vec!["bbb-2".to_string()],
    };
    service
        .read_since(
            &context(),
            &instance_query(),
            LogSeverity::Warning,
            &cursor,
            200,
        )
        .expect("tail should load");

    let commands = service.runner().commands.borrow();
    assert_eq!(
        commands[0].args,
        args(&[
            "--configuration",
            "dev",
            "logging",
            "read",
            "resource.type=\"gce_instance\" AND resource.labels.instance_id=\"1001\" AND severity>=WARNING AND timestamp>=\"2026-07-19T10:00:01.000000000Z\"",
            "--format=json",
            "--order=asc",
            "--limit",
            "200",
            "--project",
            "dev-project",
        ])
    );
    assert_eq!(severity_clauses(&commands[0].args), 1);
}

/// The Cloud Functions filter is the one base filter with a top-level `OR`.
/// Composition appends the floor and the cursor with ` AND `, so the base has
/// to arrive already parenthesized as a single term — otherwise the gen1 half
/// escapes both narrowing clauses and the tail re-delivers entries the buffer
/// has already seen.
#[test]
fn read_since_narrows_both_cloud_function_generations() {
    let service = GcloudLoggingService::new(empty_json_runner());
    let cursor = LogCursor {
        timestamp: "2026-07-19T10:00:01.000000000Z".to_string(),
        insert_ids: vec!["ccc-3".to_string()],
    };
    service
        .read_since(
            &context(),
            &LogQuery::Resource(LogResourceKind::CloudFunction {
                function_name: "thumbnailer".to_string(),
                region: "europe-west1".to_string(),
            }),
            LogSeverity::Error,
            &cursor,
            200,
        )
        .expect("tail should load");

    let commands = service.runner().commands.borrow();
    assert_eq!(
        commands[0].args[4],
        "((resource.type=\"cloud_function\" AND resource.labels.function_name=\"thumbnailer\" AND resource.labels.region=\"europe-west1\") OR (resource.type=\"cloud_run_revision\" AND resource.labels.service_name=\"thumbnailer\" AND resource.labels.location=\"europe-west1\")) AND severity>=ERROR AND timestamp>=\"2026-07-19T10:00:01.000000000Z\""
    );
}

#[test]
fn read_since_project_wide_floor_precedes_the_timestamp_clause() {
    let service = GcloudLoggingService::new(empty_json_runner());
    let cursor = LogCursor {
        timestamp: "2026-07-19T10:00:01.000000000Z".to_string(),
        insert_ids: vec!["bbb-2".to_string()],
    };
    service
        .read_since(
            &context(),
            &LogQuery::ProjectLogs,
            LogSeverity::Error,
            &cursor,
            200,
        )
        .expect("tail should load");

    let commands = service.runner().commands.borrow();
    assert_eq!(
        commands[0].args[4],
        "severity>=ERROR AND timestamp>=\"2026-07-19T10:00:01.000000000Z\""
    );
}

#[test]
fn read_recent_surfaces_command_failure() {
    let runner = FakeRunner::default();
    runner.outputs.borrow_mut().push(Err(CommandError::Failed {
        stdout: String::new(),
        stderr: "PERMISSION_DENIED".to_string(),
        exit_code: None,
    }));
    let service = GcloudLoggingService::new(runner);
    let error = service
        .read_recent(
            &context(),
            &LogQuery::ProjectLogs,
            LogSeverity::Default,
            LogTimeRange::OneHour,
            200,
        )
        .expect_err("failure should surface");
    assert!(error.to_string().contains("PERMISSION_DENIED"));
}

#[test]
fn parser_tolerates_partial_entries() {
    // First entry valid, second missing insertId -> dropped, not fatal.
    let json = r#"[
      {"insertId":"ok-1","timestamp":"2026-07-19T10:00:00Z","severity":"INFO","logName":"l","resource":{"type":"gce_instance","labels":{}}},
      {"timestamp":"2026-07-19T10:00:01Z","severity":"INFO"}
    ]"#;
    let outcome = parse_log_entries_json(json).expect("array parses");
    assert_eq!(outcome.entries.len(), 1);
    assert_eq!(outcome.dropped, 1);
}

#[test]
fn parser_rejects_non_array_json() {
    assert!(parse_log_entries_json("{not-json").is_err());
}
