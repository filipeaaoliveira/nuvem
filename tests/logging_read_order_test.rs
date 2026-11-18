//! The two backends ask for log entries in the same order.
//!
//! The recent window and the incremental tail want opposite orders, and the
//! reason is not cosmetic. The recent window is capped at `limit`, so it must
//! ask newest-first: a freshness window means the most recent entries. The tail
//! is capped at `limit` too, but its filter admits everything newer than the
//! cursor -- so it must ask oldest-first, or the cap keeps the NEWEST `limit`,
//! the cursor advances past everything older than them, and the entries in
//! between are never fetched by this poll or by any later one. That is a silent
//! hole: the skipped entries mapped fine, so nothing counts them as dropped.
//!
//! The native backend spelled both orders `timestamp desc` until a read-path
//! measurement compared the two backends. This test holds the pair together:
//! the CLI's `--order=` argument is read out of the argv a fake runner records,
//! and the native constants are asserted against the same words, so a future
//! change to one backend that is not made to the other fails here.

use std::cell::RefCell;

use nuvem::services::gcloud::cli::{CommandError, CommandOutput, CommandRunner, GcloudCommand};
use nuvem::services::resources::logging::GcloudLoggingService;
use nuvem::services::sdk::logging::{RECENT_ORDER, TAIL_ORDER};
use nuvem_domain::gcp::configuration::{GcloudConfigurationName, GcloudContext};
use nuvem_domain::gcp::logging::{LogCursor, LogQuery, LogSeverity, LogTimeRange};

#[derive(Default)]
struct RecordingRunner {
    commands: RefCell<Vec<GcloudCommand>>,
}

impl CommandRunner for RecordingRunner {
    fn run(&self, command: GcloudCommand) -> Result<CommandOutput, CommandError> {
        self.commands.borrow_mut().push(command);
        Ok(CommandOutput {
            stdout: "[]".to_string(),
            stderr: String::new(),
        })
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

/// The `--order=<word>` the recorded argv carries, whichever position it is in.
fn recorded_order(runner: &RecordingRunner) -> String {
    let commands = runner.commands.borrow();
    let command = commands.last().expect("a command was run");
    command
        .args
        .iter()
        .find_map(|arg| arg.strip_prefix("--order=").map(str::to_string))
        .expect("the logging read states an order")
}

#[test]
fn the_recent_window_asks_both_backends_for_the_newest_entries_first() {
    let runner = RecordingRunner::default();
    let service = GcloudLoggingService::new(runner);
    service
        .read_recent(
            &context(),
            &LogQuery::ProjectLogs,
            LogSeverity::Default,
            LogTimeRange::OneHour,
            200,
        )
        .expect("the read succeeds against an empty answer");

    assert_eq!(recorded_order(service.runner()), "desc");
    assert_eq!(
        RECENT_ORDER, "timestamp desc",
        "the native client asks for the same order in its own spelling"
    );
}

#[test]
fn the_tail_asks_both_backends_for_the_oldest_entries_after_the_cursor() {
    let runner = RecordingRunner::default();
    let service = GcloudLoggingService::new(runner);
    service
        .read_since(
            &context(),
            &LogQuery::ProjectLogs,
            LogSeverity::Default,
            &LogCursor {
                timestamp: "2026-09-11T10:00:00.000000000Z".to_string(),
                insert_ids: vec!["planted-insert-id".to_string()],
            },
            200,
        )
        .expect("the read succeeds against an empty answer");

    assert_eq!(
        recorded_order(service.runner()),
        "asc",
        "oldest-first, so a capped page starts at the cursor and leaves the \
         remainder for the next poll instead of skipping it"
    );
    assert_eq!(
        TAIL_ORDER, "timestamp asc",
        "the native client asks for the same order in its own spelling"
    );
    assert_ne!(
        RECENT_ORDER, TAIL_ORDER,
        "the two reads want opposite orders; collapsing them reintroduces the hole"
    );
}
