use std::collections::BTreeMap;

use serde_json::Value;

use nuvem_domain::gcp::configuration::GcloudContext;
use nuvem_domain::gcp::logging::{
    LogCursor, LogEntry, LogPayload, LogQuery, LogSeverity, LogTimeRange, quote, sanitize_log_text,
};

use crate::services::gcloud::cli::{CommandError, CommandRunner, GcloudCommand};

#[derive(Debug, thiserror::Error)]
pub enum LogParseError {
    #[error("invalid log entries JSON: {0}")]
    Json(#[from] serde_json::Error),
}

#[derive(Debug, thiserror::Error)]
pub enum LogLoadError {
    #[error("{0}")]
    Command(#[from] CommandError),
    #[error("{0}")]
    Parse(#[from] LogParseError),
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LogReadOutcome {
    pub entries: Vec<LogEntry>,
    pub dropped: usize,
}

pub struct GcloudLoggingService<R> {
    runner: R,
}

impl<R> GcloudLoggingService<R> {
    pub fn new(runner: R) -> Self {
        Self { runner }
    }
    pub fn runner(&self) -> &R {
        &self.runner
    }
}

impl<R: CommandRunner> GcloudLoggingService<R> {
    /// Reads the newest page for `query`, narrowed server-side by
    /// `severity_floor`: the floor is ANDed into the gcloud filter rather than
    /// applied client-side, so fewer entries are fetched. A
    /// `LogSeverity::Default` floor contributes nothing to the argv.
    pub fn read_recent(
        &self,
        context: &GcloudContext,
        query: &LogQuery,
        severity_floor: LogSeverity,
        range: LogTimeRange,
        limit: usize,
    ) -> Result<LogReadOutcome, LogLoadError> {
        let mut args: Vec<String> = vec![
            "--configuration".into(),
            context.configuration.as_str().into(),
            "logging".into(),
            "read".into(),
        ];
        let filter = compose_filter([
            non_empty(query.render_filter()),
            severity_floor.filter_clause(),
        ]);
        // A project-wide read with no floor composes to an empty filter; the
        // positional argument is omitted entirely rather than passed as an
        // empty string, keeping the argv identical to an unfiltered read.
        if !filter.is_empty() {
            args.push(filter);
        }
        args.push("--format=json".into());
        args.push("--order=desc".into());
        args.push("--limit".into());
        args.push(limit.to_string());
        args.push(format!("--freshness={}", range.freshness_arg()));
        push_project(&mut args, context);
        let output = self.runner.run(GcloudCommand::new(args))?;
        let mut outcome = parse_log_entries_json(&output.stdout)?;
        // gcloud returns `--order=desc` (newest-first). Normalize to ascending
        // (oldest -> newest) so the buffer convention `entries.back() == newest`
        // holds everywhere, matching `read_since` (`--order=asc`).
        outcome.entries.reverse();
        Ok(outcome)
    }

    /// Incremental tail from `cursor`, narrowed server-side by
    /// `severity_floor` the same way `read_recent` is. The timestamp clause is
    /// always present, so the composed filter is never empty here.
    pub fn read_since(
        &self,
        context: &GcloudContext,
        query: &LogQuery,
        severity_floor: LogSeverity,
        cursor: &LogCursor,
        limit: usize,
    ) -> Result<LogReadOutcome, LogLoadError> {
        let filter = compose_filter([
            non_empty(query.render_filter()),
            severity_floor.filter_clause(),
            // Through `quote` like every other interpolated clause: the
            // timestamp comes from parsed gcloud JSON, and this line must not
            // be the one place a quote in that field escapes the expression.
            Some(format!("timestamp>={}", quote(&cursor.timestamp))),
        ]);
        let mut args: Vec<String> = vec![
            "--configuration".into(),
            context.configuration.as_str().into(),
            "logging".into(),
            "read".into(),
            filter,
            "--format=json".into(),
            "--order=asc".into(),
            "--limit".into(),
            limit.to_string(),
        ];
        push_project(&mut args, context);
        let output = self.runner.run(GcloudCommand::new(args))?;
        Ok(parse_log_entries_json(&output.stdout)?)
    }
}

/// Joins the clauses that apply with ` AND `, the composition gcloud's
/// `logging read` filter language expects. Absent clauses (an empty base
/// filter for project-wide reads, a `LogSeverity::Default` floor) are skipped
/// rather than emitted as empty conjuncts, so an all-absent set yields an
/// empty filter the caller can omit.
pub fn compose_filter(clauses: impl IntoIterator<Item = Option<String>>) -> String {
    clauses
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" AND ")
}

pub fn non_empty(clause: String) -> Option<String> {
    (!clause.is_empty()).then_some(clause)
}

fn push_project(args: &mut Vec<String>, context: &GcloudContext) {
    if let Some(project) = &context.project {
        args.push("--project".into());
        args.push(project.clone());
    }
}

pub fn parse_log_entries_json(json: &str) -> Result<LogReadOutcome, LogParseError> {
    let raw: Vec<Value> = serde_json::from_str(json)?;
    let mut entries = Vec::new();
    let mut dropped = 0usize;
    for value in raw {
        match entry_from_value(&value) {
            Some(entry) => entries.push(entry),
            None => dropped += 1,
        }
    }
    Ok(LogReadOutcome { entries, dropped })
}

fn entry_from_value(value: &Value) -> Option<LogEntry> {
    let insert_id = value.get("insertId")?.as_str()?.to_string();
    let timestamp = value
        .get("timestamp")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let severity =
        LogSeverity::from_api(value.get("severity").and_then(Value::as_str).unwrap_or(""));
    let log_name = value
        .get("logName")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let resource = value.get("resource");
    let resource_type = resource
        .and_then(|r| r.get("type"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let resource_labels = string_map(resource.and_then(|r| r.get("labels")));
    let labels = string_map(value.get("labels"));

    let (payload, message) = payload_and_message(
        value.get("textPayload").and_then(Value::as_str),
        value.get("jsonPayload"),
        &log_name,
    );

    Some(LogEntry {
        insert_id,
        timestamp,
        severity,
        resource_type,
        resource_labels,
        log_name,
        message,
        payload,
        labels,
    })
}

fn string_map(value: Option<&Value>) -> BTreeMap<String, String> {
    let mut map = BTreeMap::new();
    if let Some(Value::Object(obj)) = value {
        for (k, v) in obj {
            if let Some(s) = v.as_str() {
                map.insert(k.clone(), s.to_string());
            }
        }
    }
    map
}

/// The payload a viewer paints and the one-line message beside it.
///
/// Shared with the native path. The precedence — text, then structured, then
/// the log name as a last resort — and the choice of a structured payload's
/// `message` field over its whole body are what a reader sees in the list, so
/// the two backends must not answer differently.
pub fn payload_and_message(
    text: Option<&str>,
    json: Option<&Value>,
    log_name: &str,
) -> (LogPayload, String) {
    if let Some(text) = text {
        return (LogPayload::Text(text.to_string()), sanitize_log_text(text));
    }
    if let Some(json) = json {
        let message = json
            .get("message")
            .and_then(Value::as_str)
            .map(sanitize_log_text)
            .unwrap_or_else(|| sanitize_log_text(&json.to_string()));
        let pretty = serde_json::to_string_pretty(json).unwrap_or_else(|_| json.to_string());
        return (LogPayload::Json(pretty), message);
    }
    (LogPayload::Empty, sanitize_log_text(log_name))
}
