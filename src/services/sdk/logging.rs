//! Cloud Logging reads through the native client.
//!
//! Unlike the Compute filter, the expression this application builds *is* the
//! API's own language: `gcloud logging read` forwards its filter to the same
//! `filter` field this client sets, so the query is passed through rather than
//! dropped. That difference is the whole reason the Compute path refuses to
//! push its filter down and this one does not.

use std::collections::BTreeMap;

use google_cloud_auth::credentials::Credentials;
use google_cloud_logging_v2::client::LoggingServiceV2;
use nuvem_domain::gcp::logging::{LogCursor, LogEntry, LogQuery, LogSeverity, LogTimeRange, quote};

use crate::services::resources::list_filter::rows_are_capped;
use crate::services::resources::logging::{
    LogReadOutcome, compose_filter, non_empty, payload_and_message,
};
use crate::services::sdk::ApiError;

/// The order `read_recent` asks for: newest first, so a capped window is the
/// most recent `limit` entries, which is what a freshness window means. The
/// rows are reversed afterwards to restore the buffer's oldest-first
/// convention.
///
/// The CLI path spells this `--order=desc`, and
/// `logging_read_order_test.rs` holds the two spellings together.
pub const RECENT_ORDER: &str = "timestamp desc";

/// The order `read_since` asks for: oldest first, so a capped window starts at
/// the cursor and leaves the remainder for the next poll rather than skipping
/// it. The CLI path spells this `--order=asc`.
pub const TAIL_ORDER: &str = "timestamp asc";

pub struct SdkLoggingService {
    logging: LoggingServiceV2,
}

impl SdkLoggingService {
    pub async fn connect(credentials: Credentials) -> Result<Self, ApiError> {
        let logging = LoggingServiceV2::builder()
            .with_credentials(credentials)
            .build()
            .await
            .map_err(|error| ApiError::new(error.to_string()))?;
        Ok(Self { logging })
    }

    /// The same service, built from an already-constructed client. See
    /// `SdkGkeService::from_clusters_for_test`.
    pub fn from_logging_for_test(logging: LoggingServiceV2) -> Self {
        Self { logging }
    }

    /// The same service, answering from the demo fixtures.
    ///
    /// Required by `sdk_client_cache!`: the macro's `Fixtures` arm calls this
    /// on every service in its list, so a fifteenth service cannot join that
    /// list without one. It takes no credentials, cannot fail and cannot
    /// block -- a client built by `from_stub` holds a stub where a
    /// transport would be.
    pub fn from_fixtures() -> Self {
        Self {
            logging: crate::services::demo::sdk::logging::client(),
        }
    }
    /// The recent window.
    ///
    /// `--freshness` has no API equivalent, so the window becomes a
    /// `timestamp>=` clause built from the same bucket the flag named. Entries
    /// come back newest-first and are reversed, so the buffer convention
    /// `entries.back() == newest` holds exactly as it does on the CLI path.
    pub async fn read_recent(
        &self,
        project: &str,
        query: &LogQuery,
        severity_floor: LogSeverity,
        range: LogTimeRange,
        limit: usize,
    ) -> Result<LogReadOutcome, ApiError> {
        let since = window_start(chrono::Utc::now(), range);
        let filter = compose_filter([
            non_empty(query.render_filter()),
            severity_floor.filter_clause(),
            Some(format!(
                "timestamp>={}",
                quote(&since.to_rfc3339_opts(chrono::SecondsFormat::Nanos, true))
            )),
        ]);
        let mut outcome = self.read(project, &filter, limit, RECENT_ORDER).await?;
        outcome.entries.reverse();
        Ok(outcome)
    }

    /// The incremental tail, narrowed by the same clauses plus the cursor.
    pub async fn read_since(
        &self,
        project: &str,
        query: &LogQuery,
        severity_floor: LogSeverity,
        cursor: &LogCursor,
        limit: usize,
    ) -> Result<LogReadOutcome, ApiError> {
        let filter = compose_filter([
            non_empty(query.render_filter()),
            severity_floor.filter_clause(),
            Some(format!("timestamp>={}", quote(&cursor.timestamp))),
        ]);
        // Ascending, as `--order=asc` asks for on the CLI path, and the
        // difference is not cosmetic. The cursor clause admits everything newer
        // than the last entry seen, and the cap then keeps `limit` of them. Ask
        // for them newest-first and the cap keeps the NEWEST `limit`, the
        // cursor advances past the rest, and every entry between the cursor and
        // that window is never fetched by this poll or any later one -- a hole
        // in the tail with nothing to report it, because the entries that fell
        // out were mapped fine and `dropped` counts only unmappable ones.
        // Ascending keeps the window contiguous with the cursor, so what the
        // cap defers is fetched by the next poll.
        self.read(project, &filter, limit, TAIL_ORDER).await
    }

    /// One page of at most `limit` entries, ordered as the caller asks.
    async fn read(
        &self,
        project: &str,
        filter: &str,
        limit: usize,
        order_by: &str,
    ) -> Result<LogReadOutcome, ApiError> {
        use google_cloud_gax::paginator::ItemPaginator;

        let mut request = self
            .logging
            .list_log_entries()
            .set_resource_names([format!("projects/{project}")])
            .set_order_by(order_by)
            .set_page_size(limit as i32);
        if let Some(filter) = filter_to_send(filter) {
            request = request.set_filter(filter);
        }

        let mut outcome = LogReadOutcome::default();
        let mut items = request.by_item();
        while let Some(entry) = items.next().await {
            take_entry(&mut outcome, map_entry(&entry?));
            if rows_are_capped(outcome.entries.len(), limit) {
                break;
            }
        }
        Ok(outcome)
    }
}

/// The start of the window `range` names, counted back from `now`.
///
/// `--freshness` has no API equivalent, so the window becomes a `timestamp>=`
/// clause and this is the timestamp it carries. Taken as a function of `now`
/// rather than reading the clock itself, so the subtraction can be tested: the
/// same arithmetic written inline in the request body was a place where a `-`
/// could become a `+` -- asking for logs from an hour in the *future*, which
/// returns nothing and reads as a quiet project.
pub fn window_start(
    now: chrono::DateTime<chrono::Utc>,
    range: LogTimeRange,
) -> chrono::DateTime<chrono::Utc> {
    now - chrono::Duration::minutes(range.minutes())
}

/// The filter to put on the request, or nothing when there is none to send.
///
/// A project-wide read at the default severity composes to an empty string,
/// and an empty `filter` on the wire is not the same as no filter at all. The
/// question is asked here rather than as a negation inside the request body,
/// where dropping the `!` would have sent the empty one and suppressed every
/// real filter -- every log read returning the whole project.
pub fn filter_to_send(filter: &str) -> Option<&str> {
    (!filter.is_empty()).then_some(filter)
}

/// Files one mapped entry into the page being built.
///
/// An entry the mapper could not read is counted rather than dropped in
/// silence: the viewer reports the count, so a page that quietly lost half its
/// rows is visible. Counting is a function so that the increment is reachable
/// by a test; inside the read loop it was a `+= 1` no test could see, which a
/// sweep happily turned into `-= 1` and `*= 1`.
pub fn take_entry(outcome: &mut LogReadOutcome, mapped: Option<LogEntry>) {
    match mapped {
        Some(entry) => outcome.entries.push(entry),
        None => outcome.dropped += 1,
    }
}

/// RFC 3339 with nanoseconds, which is what gcloud printed.
///
/// This is not cosmetic: the tail feeds a previous entry's timestamp back into
/// a `timestamp>="..."` filter clause to resume, so a format the API cannot
/// parse would break resumption rather than merely look different.
fn timestamp_rfc3339(value: Option<&google_cloud_wkt::Timestamp>) -> String {
    value
        .and_then(|stamp| chrono::DateTime::from_timestamp(stamp.seconds(), stamp.nanos() as u32))
        .map(|at| at.to_rfc3339_opts(chrono::SecondsFormat::Nanos, true))
        .unwrap_or_default()
}

fn string_map(labels: &std::collections::HashMap<String, String>) -> BTreeMap<String, String> {
    labels
        .iter()
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect()
}

/// `None` for an entry with no `insertId`, which the outcome counts as dropped
/// exactly as the CLI path does: without one, a tail cannot deduplicate it.
pub fn map_entry(entry: &google_cloud_logging_v2::model::LogEntry) -> Option<LogEntry> {
    use google_cloud_logging_v2::model::log_entry::Payload;

    if entry.insert_id.is_empty() {
        return None;
    }

    let (text, json) = match entry.payload.as_ref() {
        Some(Payload::TextPayload(text)) => (Some(text.as_str()), None),
        Some(Payload::JsonPayload(fields)) => (
            None,
            // `wkt::Struct` is a `serde_json::Map`, so the structured payload
            // reaches the shared derivation as the same `Value` the CLI path
            // hands it.
            Some(serde_json::Value::Object(fields.as_ref().clone())),
        ),
        // A proto payload is not rendered: the CLI path never saw one either,
        // because gcloud renders it under a key this reader does not read.
        _ => (None, None),
    };
    let log_name = entry.log_name.clone();
    let (payload, message) = payload_and_message(text, json.as_ref(), &log_name);

    Some(LogEntry {
        insert_id: entry.insert_id.clone(),
        timestamp: timestamp_rfc3339(entry.timestamp.as_ref()),
        severity: LogSeverity::from_api(entry.severity.name().unwrap_or_default()),
        resource_type: entry
            .resource
            .as_ref()
            .map(|resource| resource.r#type.clone())
            .unwrap_or_default(),
        resource_labels: entry
            .resource
            .as_ref()
            .map(|resource| string_map(&resource.labels))
            .unwrap_or_default(),
        log_name,
        message,
        payload,
        labels: string_map(&entry.labels),
    })
}
