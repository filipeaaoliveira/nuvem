//! Completeness of the user documentation for the vocabularies this crate
//! owns: error categories, FinOps recommenders, log severities and since
//! windows, and tunnel kinds.
//!
//! Each test walks the type's own values and fails naming every one the page
//! that documents it leaves out. Where a type has no `ALL` constant, an
//! exhaustive `match` beside the list is what keeps the list whole: adding a
//! variant stops the test compiling until the variant is listed.

use std::collections::BTreeSet;
use std::path::PathBuf;

use nuvem_domain::gcp::diagnostics::{GcloudErrorKind, Retryability};
use nuvem_domain::gcp::finops::RecommenderId;
use nuvem_domain::gcp::logging::{LogSeverity, LogTimeRange};
use nuvem_domain::gcp::tunnel::TunnelKind;

/// A page under the repository's `docs/`, read from this crate's directory.
fn page(relative: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../docs")
        .join(relative);
    std::fs::read_to_string(&path).unwrap_or_else(|error| panic!("{path:?}: {error}"))
}

/// The rows of the first table after the `heading` line, as trimmed cells.
fn table_after(text: &str, heading: &str) -> Vec<Vec<String>> {
    let start = if heading.is_empty() {
        0
    } else {
        text.find(&format!("\n{heading}\n"))
            .unwrap_or_else(|| panic!("the page has a {heading:?} heading"))
    };
    let mut lines = text[start..]
        .lines()
        .skip_while(|line| !line.starts_with('|'));
    assert!(lines.next().is_some(), "a table follows {heading:?}");
    lines
        .skip(1)
        .take_while(|line| line.starts_with('|'))
        .map(|line| {
            line.trim()
                .trim_matches('|')
                .split('|')
                .map(|cell| cell.trim().to_string())
                .collect()
        })
        .collect()
}

fn ticked(cell: &str) -> Vec<String> {
    cell.split('`')
        .skip(1)
        .step_by(2)
        .map(str::to_string)
        .collect()
}

/// Every error category is a row of the troubleshooting table, with the hint
/// nuvem shows and whether it is retried, and every row is a real category.
#[test]
fn troubleshooting_documents_every_error_category() {
    let every = [
        GcloudErrorKind::PermissionDenied,
        GcloudErrorKind::ApiDisabled,
        GcloudErrorKind::InvalidQuery,
        GcloudErrorKind::Quota,
        GcloudErrorKind::Network,
        GcloudErrorKind::Parse,
        GcloudErrorKind::MissingProject,
        GcloudErrorKind::Unknown,
        GcloudErrorKind::MissingExecutable,
        GcloudErrorKind::Unauthenticated,
        GcloudErrorKind::InvalidInput,
        GcloudErrorKind::Cancelled,
    ];
    for kind in every {
        match kind {
            GcloudErrorKind::PermissionDenied
            | GcloudErrorKind::ApiDisabled
            | GcloudErrorKind::InvalidQuery
            | GcloudErrorKind::Quota
            | GcloudErrorKind::Network
            | GcloudErrorKind::Parse
            | GcloudErrorKind::MissingProject
            | GcloudErrorKind::Unknown
            | GcloudErrorKind::MissingExecutable
            | GcloudErrorKind::Unauthenticated
            | GcloudErrorKind::InvalidInput
            | GcloudErrorKind::Cancelled => {}
        }
    }

    let rows = table_after(
        &page("troubleshooting/common-problems.md"),
        "## Error categories",
    );
    let documented: BTreeSet<(String, String, String)> = rows
        .into_iter()
        .map(|row| (row[0].clone(), row[1].clone(), row[2].clone()))
        .collect();
    let real: BTreeSet<(String, String, String)> = every
        .iter()
        .map(|kind| {
            let retried = match kind.retryability() {
                Retryability::Transient => "transient",
                Retryability::Persistent => "persistent",
            };
            (
                kind.label().to_string(),
                kind.user_action().to_string(),
                retried.to_string(),
            )
        })
        .collect();
    assert_eq!(
        documented, real,
        "the troubleshooting table lists every error category with its hint and retry class"
    );
}

/// Every FinOps recommender is a row of the FinOps guide's card table, with
/// the card heading nuvem shows and the recommender id it queries.
#[test]
fn finops_guide_documents_every_recommender() {
    let rows = table_after(&page("guides/finops.md"), "## What it shows");
    let documented: BTreeSet<(String, String)> = rows
        .into_iter()
        .map(|row| {
            let id = ticked(&row[1]).into_iter().next().unwrap_or_default();
            (row[0].clone(), id)
        })
        .collect();
    let real: BTreeSet<(String, String)> = RecommenderId::ALL
        .iter()
        .map(|recommender| {
            (
                recommender.label().to_string(),
                recommender.id().to_string(),
            )
        })
        .collect();
    assert_eq!(
        documented, real,
        "the FinOps guide lists every recommender card"
    );
}

/// The rows of the first table after a `<!-- marker -->` line.
fn table_after_marker(text: &str, marker: &str) -> Vec<Vec<String>> {
    let at = text
        .find(marker)
        .unwrap_or_else(|| panic!("the page has a {marker} marker"));
    table_after(&text[at..], "")
}

/// The severity-floor table on the Cloud Logging page lists every floor, in
/// the order `s` steps through them, and nothing else.
///
/// `LogSeverity::Default` is the floor's off position. It sends no severity
/// clause, and the Logging header shows it as `all`, so that is the name the
/// table gives it.
#[test]
fn logging_guide_documents_every_severity_floor() {
    let mut severities = [
        LogSeverity::Default,
        LogSeverity::Debug,
        LogSeverity::Info,
        LogSeverity::Notice,
        LogSeverity::Warning,
        LogSeverity::Error,
        LogSeverity::Critical,
        LogSeverity::Alert,
        LogSeverity::Emergency,
    ];
    for severity in severities {
        match severity {
            LogSeverity::Default
            | LogSeverity::Debug
            | LogSeverity::Info
            | LogSeverity::Notice
            | LogSeverity::Warning
            | LogSeverity::Error
            | LogSeverity::Critical
            | LogSeverity::Alert
            | LogSeverity::Emergency => {}
        }
    }
    severities.sort_by_key(|severity| severity.rank());
    let real: Vec<String> = severities
        .iter()
        .map(|severity| match severity {
            LogSeverity::Default => "all".to_string(),
            other => other.label().to_string(),
        })
        .collect();

    let rows = table_after_marker(&page("guides/cloud-logging.md"), "<!-- log-severities -->");
    let documented: Vec<String> = rows
        .iter()
        .map(|row| ticked(&row[0]).into_iter().next().unwrap_or_default())
        .collect();
    assert_eq!(
        documented, real,
        "the severity table lists every floor, lowest to highest"
    );
}

/// The window table on the Cloud Logging page pairs every digit key with the
/// window `LogTimeRange::from_bucket_digit` gives it, marks the default, and
/// lists nothing else.
#[test]
fn logging_guide_documents_every_since_window() {
    let real: Vec<(String, String, bool)> = ('0'..='9')
        .filter_map(|digit| {
            LogTimeRange::from_bucket_digit(digit).map(|window| {
                (
                    digit.to_string(),
                    window.label().to_string(),
                    window == LogTimeRange::DEFAULT,
                )
            })
        })
        .collect();

    let rows = table_after_marker(&page("guides/cloud-logging.md"), "<!-- log-windows -->");
    let documented: Vec<(String, String, bool)> = rows
        .iter()
        .map(|row| {
            let key = ticked(&row[0]).into_iter().next().unwrap_or_default();
            let window = ticked(&row[1]).into_iter().next().unwrap_or_default();
            (key, window, row[1].contains("(the default)"))
        })
        .collect();
    assert_eq!(
        documented, real,
        "the window table lists every digit, its window, and the default"
    );
}

/// Every tunnel kind's badge is a row of the Tunnels guide's kind table.
#[test]
fn tunnels_guide_documents_every_tunnel_kind() {
    let every = [
        TunnelKind::IapTcp,
        TunnelKind::SshLocalForward,
        TunnelKind::CloudSqlProxy,
    ];
    for kind in every {
        match kind {
            TunnelKind::IapTcp | TunnelKind::SshLocalForward | TunnelKind::CloudSqlProxy => {}
        }
    }

    let text = page("guides/tunnels.md");
    let start = text
        .find("| Kind |")
        .expect("the Tunnels guide has a table of tunnel kinds");
    let documented: BTreeSet<String> = text[start..]
        .lines()
        .skip(2)
        .take_while(|line| line.starts_with('|'))
        .filter_map(|line| ticked(line).into_iter().next())
        .collect();
    let real: BTreeSet<String> = every.iter().map(|kind| kind.label().to_string()).collect();
    assert_eq!(
        documented, real,
        "the Tunnels guide lists every tunnel kind"
    );
}
