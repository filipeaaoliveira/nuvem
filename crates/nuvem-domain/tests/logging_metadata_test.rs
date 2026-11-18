//! Pins `LogSeverity`'s API parsing and ranking, and the log-text sanitizer.

use nuvem_domain::gcp::logging::{LogSeverity, LogTimeRange, sanitize_log_text};

/// Every documented severity string parses to its own variant — one string
/// per assertion, so a deleted match arm fails its own line rather than
/// hiding behind a neighbour's.
#[test]
fn every_api_severity_parses_to_its_own_variant() {
    for (raw, expected) in [
        ("DEBUG", LogSeverity::Debug),
        ("INFO", LogSeverity::Info),
        ("NOTICE", LogSeverity::Notice),
        ("WARNING", LogSeverity::Warning),
        ("ERROR", LogSeverity::Error),
        ("CRITICAL", LogSeverity::Critical),
        ("ALERT", LogSeverity::Alert),
        ("EMERGENCY", LogSeverity::Emergency),
    ] {
        assert_eq!(LogSeverity::from_api(raw), expected, "for {raw:?}");
    }
    assert_eq!(LogSeverity::from_api("error"), LogSeverity::Error);
    assert_eq!(LogSeverity::from_api("UNHEARD_OF"), LogSeverity::Default);
}

#[test]
fn rank_walks_the_ladder_from_default_to_emergency() {
    let ladder = [
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
    for (position, severity) in ladder.into_iter().enumerate() {
        assert_eq!(severity.rank(), position as u8);
    }
}

#[test]
fn sanitize_log_text_keeps_text_and_drops_controls() {
    assert_eq!(sanitize_log_text("a\tb"), "a b");
    assert_eq!(sanitize_log_text("a\nb\rc"), "abc");
    assert_eq!(sanitize_log_text("a\u{1b}b\u{07}c"), "abc");
    assert_eq!(sanitize_log_text("café 🚀 fine"), "café 🚀 fine");
}

/// Every window's length in minutes, one assertion per bucket.
///
/// `minutes` is what the native backend builds its `timestamp>=` clause from,
/// and `freshness_arg` is what the CLI backend passes to `--freshness`. The
/// two describe the same window through different backends, so they are
/// asserted together: a bucket whose minutes drift from its argument would
/// show the same table two different amounts of history depending on which
/// backend filled it, with nothing on screen to say so.
///
/// The expected minutes are written out rather than computed from the hours,
/// so an arithmetic slip in the function -- a `*` that became a `+`, a body
/// that answers a constant -- fails here instead of being reproduced by the
/// expectation.
#[test]
fn every_window_reports_its_own_length_and_matches_its_freshness_argument() {
    for (range, minutes, argument) in [
        (LogTimeRange::OneMinute, 1_i64, "1m"),
        (LogTimeRange::FiveMinutes, 5, "5m"),
        (LogTimeRange::FifteenMinutes, 15, "15m"),
        (LogTimeRange::ThirtyMinutes, 30, "30m"),
        (LogTimeRange::OneHour, 60, "1h"),
        (LogTimeRange::SixHours, 360, "6h"),
        (LogTimeRange::TwentyFourHours, 1440, "24h"),
    ] {
        assert_eq!(
            range.minutes(),
            minutes,
            "{argument} is {minutes} minutes of history"
        );
        assert_eq!(range.freshness_arg(), argument);
        assert_eq!(
            range.label(),
            argument,
            "the label the operator reads is the window that is asked for"
        );
    }
}

/// The ladder is strictly increasing, so no two buckets claim the same window
/// and `next` never moves the operator to a window the same size.
#[test]
fn each_window_is_longer_than_the_one_below_it() {
    let ladder = [
        LogTimeRange::OneMinute,
        LogTimeRange::FiveMinutes,
        LogTimeRange::FifteenMinutes,
        LogTimeRange::ThirtyMinutes,
        LogTimeRange::OneHour,
        LogTimeRange::SixHours,
        LogTimeRange::TwentyFourHours,
    ];

    for pair in ladder.windows(2) {
        assert!(
            pair[0].minutes() < pair[1].minutes(),
            "{} must be a shorter window than {}",
            pair[0].label(),
            pair[1].label()
        );
        assert_eq!(
            pair[0].next(),
            pair[1],
            "the key that cycles the ladder moves one rung up"
        );
    }
    assert_eq!(
        ladder[ladder.len() - 1].next(),
        ladder[0],
        "the longest window wraps back to the shortest"
    );
}
