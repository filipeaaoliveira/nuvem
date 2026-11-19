//! Cloud Logging's demo backend: the entries capture, for both reads.
//!
//! Envelope-only. `log_entries.json` is the array `entries.list` wraps in
//! `ListLogEntriesResponse.entries`, and the entries themselves are the proto3
//! JSON the generated `LogEntry` model reads, `jsonPayload` and all.
//!
//! # Why one answer serves both the window and the tail
//!
//! `read_recent` and `read_since` differ only in the filter and the order they
//! ask for, and this stub honours neither -- the capture is two entries taken
//! at one moment, and filtering them by a `timestamp>=` clause built from
//! `Utc::now()` would make the demo screen empty the day after the capture was
//! taken. `DemoCommandRunner` routes `logging read` to the same file for the
//! same reason: demo mode shows the capture, not a clock.

use google_cloud_logging_v2::client::LoggingServiceV2;
use google_cloud_logging_v2::model;

use super::envelope;
use crate::services::demo::Corpus;

/// The entries capture: `gcloud logging read --format=json`.
const ENTRIES: &str = include_str!("../../../../tests/fixtures/log_entries.json");

/// What `nuvem --demo` lists: see [`Corpus::Showcase`].
const SHOWCASE_ENTRIES: &str = include_str!("../../../../tests/fixtures/demo/log_entries.json");

/// Answers `list_log_entries` from the capture.
///
/// One page, so the item paginator stops after it -- which also bounds the
/// tail: a `read_since` poll that got a page token would ask again and again
/// for the same two entries. `ListLogEntries` has no poller, and both service
/// methods funnel into this one call.
#[derive(Debug, Default)]
pub struct LoggingStub {
    pub corpus: Corpus,
}

impl google_cloud_logging_v2::stub::LoggingServiceV2 for LoggingStub {
    async fn list_log_entries(
        &self,
        _request: model::ListLogEntriesRequest,
        _options: google_cloud_gax::options::RequestOptions,
    ) -> google_cloud_gax::Result<google_cloud_gax::response::Response<model::ListLogEntriesResponse>>
    {
        Ok(google_cloud_gax::response::Response::from(envelope(
            "entries",
            self.corpus.pick(ENTRIES, SHOWCASE_ENTRIES),
        )?))
    }
}

/// The client `SdkLoggingService::from_fixtures` is built from.
pub fn client() -> LoggingServiceV2 {
    client_of(Corpus::Captures)
}

/// The client over either corpus.
pub fn client_of(corpus: Corpus) -> LoggingServiceV2 {
    LoggingServiceV2::from_stub(LoggingStub { corpus })
}
