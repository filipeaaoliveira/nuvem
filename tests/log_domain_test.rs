use nuvem_domain::gcp::logging::{LogSeverity, LogTimeRange, sanitize_log_text};

#[test]
fn severity_parses_from_api_strings_and_ranks() {
    assert_eq!(LogSeverity::from_api("ERROR"), LogSeverity::Error);
    assert_eq!(LogSeverity::from_api("warning"), LogSeverity::Warning);
    assert_eq!(LogSeverity::from_api(""), LogSeverity::Default);
    assert_eq!(LogSeverity::from_api("BOGUS"), LogSeverity::Default);
    assert!(LogSeverity::Error > LogSeverity::Info);
    assert_eq!(LogSeverity::Error.label(), "ERROR");
}

#[test]
fn time_range_maps_to_freshness_and_cycles() {
    assert_eq!(LogTimeRange::OneHour.freshness_arg(), "1h");
    assert_eq!(LogTimeRange::FiveMinutes.freshness_arg(), "5m");
    assert_eq!(LogTimeRange::DEFAULT, LogTimeRange::OneHour);
    assert_eq!(LogTimeRange::OneHour.next(), LogTimeRange::SixHours);
    assert_eq!(
        LogTimeRange::TwentyFourHours.next(),
        LogTimeRange::OneMinute
    );
}

#[test]
fn sanitize_strips_control_and_ansi_sequences() {
    // ESC[2J clear screen, CR, backspace, bell must not survive.
    let raw = "safe\u{1b}[2Jline\rmore\u{08}\u{07}end\ttab";
    let clean = sanitize_log_text(raw);
    assert!(!clean.contains('\u{1b}'));
    assert!(!clean.contains('\r'));
    assert!(!clean.contains('\u{08}'));
    assert!(!clean.contains('\u{07}'));
    // Tabs are converted to a space; printable text preserved.
    assert!(clean.contains("safe"));
    assert!(clean.contains("end"));
    assert!(!clean.contains('\t'));
    // Newlines are removed for the compact one-line message form.
    assert!(!clean.contains('\n'));
}
