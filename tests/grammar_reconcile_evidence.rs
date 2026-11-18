//! Evidence generator — the two filter grammars over one set of
//! shared matching primitives.
//!
//! This test exists to stop someone "simplifying" the two grammars into one.
//! The table filter (`gcp::filter`) and the log search
//! (`gcp::text_filter`) are both thin parsers over `gcp::matcher`, and reading
//! that code it looks like the last step left undone is to give them a single
//! default mode too. Doing that would silently break the most common log
//! search there is: a bracketed level such as `[ERROR]`. As a literal
//! substring it finds the brackets the operator typed; as a regex it is a
//! character class that matches every line containing an `E`, an `R` or an
//! `O`, and never matches the brackets at all. The result set would still look
//! plausible, which is what makes it worse than an empty one — nothing would
//! fail, no error would be shown, and the operator would read the wrong lines.
//!
//! So the two surfaces are asserted side by side here, on the same inputs and
//! the same haystacks: what they deliberately do *differently* (the default
//! mode), and what they must keep doing *identically* now that they share the
//! code underneath (inverse, case-insensitivity, fuzzy subsequence). If a
//! future change unifies the defaults, the first block below fails.
//!
//! Every assertion runs unconditionally. When `NUVEM_EVIDENCE_DIR` is set, the
//! same comparisons are also written out as a side-by-side transcript; the
//! test writes nothing at all when it is unset, and never outside that
//! directory.

use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;

use nuvem_domain::gcp::filter::{FilterMode, compile_filter, filter_mode};
use nuvem_domain::gcp::text_filter::{TextFilter, TextFilterKind};

// ---------------------------------------------------------------------------
// The corpus both surfaces are asked about
// ---------------------------------------------------------------------------

/// A log line carrying the bracketed level an operator actually types `[ERROR]`
/// to find.
const BRACKETED_LINE: &str = "2026-07-27T10:00:00Z [ERROR] disk full";

/// The same event without the brackets. A literal `[ERROR]` must NOT find this
/// line — that miss is the whole point of the log surface's default.
const BARE_LINE: &str = "2026-07-27T10:00:00Z ERROR disk full";

/// A table field with a bare `E` and no brackets anywhere: invisible to a
/// literal `[ERROR]`, matched by the character class `[ERROR]` compiles to.
const FIELD_WITH_E: &str = "e2-medium";

/// A table field containing none of `E`, `R`, `O` — so even read as a
/// character class, `[ERROR]` misses it. Without this the "it matches a bare
/// E" claim could be satisfied by a filter that matched everything.
const FIELD_WITHOUT_ERO: &str = "zz-1";

/// A needle in both cases that shares no character with either
/// case-insensitivity haystack below, so every mode on both surfaces must miss
/// it — as a substring, as a subsequence and as a regex. Without these the
/// case-insensitivity block would be all-positive and a matcher that said
/// MATCH to everything would satisfy it.
const ABSENT_NEEDLES: [&str; 2] = ["zqj", "ZQJ"];

// ---------------------------------------------------------------------------
// Asking each surface the same question
// ---------------------------------------------------------------------------

/// Whether the table surface's filter `input` matches any of `haystacks`.
///
/// The table takes a row's searchable display values, so the haystack list is
/// passed as fields; `matches` applies the same any-field-then-invert rule the
/// log surface's `matches_any` does.
fn table_matches(input: &str, haystacks: &[&str]) -> bool {
    let fields: Vec<String> = haystacks.iter().map(|h| (*h).to_string()).collect();
    compile_filter(input)
        .unwrap_or_else(|error| panic!("table filter {input:?} must parse: {error}"))
        .matches(&fields)
}

/// Whether the log surface's search `input` matches any of `haystacks`.
fn log_matches(input: &str, haystacks: &[&str]) -> bool {
    TextFilter::parse(input)
        .unwrap_or_else(|error| panic!("log filter {input:?} must parse: {error}"))
        .matches_any(haystacks.iter().copied())
}

// ---------------------------------------------------------------------------
// Evidence writing
// ---------------------------------------------------------------------------

/// The evidence directory, or `None` when `NUVEM_EVIDENCE_DIR` is unset.
///
/// There is deliberately no fallback location: with the variable unset this
/// test writes nothing, so it cannot collide with a concurrent run or leave
/// files behind on a developer's machine.
fn evidence_dir() -> Option<PathBuf> {
    let dir = std::env::var("NUVEM_EVIDENCE_DIR").ok()?;
    let path = PathBuf::from(dir);
    fs::create_dir_all(&path).ok()?;
    Some(path)
}

/// The transcript accumulated during the run, written once at the end.
static TRANSCRIPT: Mutex<String> = Mutex::new(String::new());

fn record(line: &str) {
    TRANSCRIPT.lock().expect("transcript lock").push_str(line);
    TRANSCRIPT.lock().expect("transcript lock").push('\n');
}

fn section(title: &str) {
    record(&format!("\n{title}\n{}", "-".repeat(title.len())));
}

/// One `input × haystack` row of the transcript, with each surface's verdict.
fn record_pair(table_input: &str, log_input: &str, haystack: &str, table: bool, log: bool) {
    record(&format!(
        "  table {table_input:<12} | log {log_input:<12} | {haystack:<40} | table={} log={}",
        yes_no(table),
        yes_no(log)
    ));
}

fn yes_no(value: bool) -> &'static str {
    if value { "MATCH" } else { "miss " }
}

fn write_transcript() {
    let Some(dir) = evidence_dir() else {
        return;
    };
    let body = TRANSCRIPT.lock().expect("transcript lock").clone();
    fs::write(dir.join("grammar-reconcile-transcript.txt"), body)
        .expect("write grammar transcript");
}

// ---------------------------------------------------------------------------
// The comparison
// ---------------------------------------------------------------------------

#[test]
fn the_two_grammars_share_primitives_but_not_their_defaults() {
    record(
        "The resource-table filter grammar and the log-viewer search grammar,\n\
         asked the same questions side by side. Both compile to the primitives in\n\
         gcp/matcher.rs. The first block is what they deliberately do DIFFERENTLY (the\n\
         default mode); everything after it is what they must do IDENTICALLY.",
    );

    // ---- 1. The same input, two defaults ----------------------------------
    section("1. `[ERROR]` — literal substring on logs, character class on the table");

    // The log surface: an unprefixed input is a literal substring, so the
    // brackets are text. Matching the bracketed line and missing the bare one
    // is the behaviour a unified regex default would destroy.
    let filter = TextFilter::parse("[ERROR]").expect("a bracketed level always parses");
    assert_eq!(
        filter.kind(),
        TextFilterKind::Substring,
        "an unprefixed log search must stay a literal substring"
    );
    assert!(
        log_matches("[ERROR]", &[BRACKETED_LINE]),
        "the log search must find the brackets the operator typed"
    );
    assert!(
        !log_matches("[ERROR]", &[BARE_LINE]),
        "read as a character class, `[ERROR]` would match this unbracketed line — that is \
         exactly the silent breakage this test defends against"
    );
    record_pair(
        "[ERROR]",
        "[ERROR]",
        BRACKETED_LINE,
        table_matches("[ERROR]", &[BRACKETED_LINE]),
        log_matches("[ERROR]", &[BRACKETED_LINE]),
    );
    record_pair(
        "[ERROR]",
        "[ERROR]",
        BARE_LINE,
        table_matches("[ERROR]", &[BARE_LINE]),
        log_matches("[ERROR]", &[BARE_LINE]),
    );

    // The table surface: the same input is a regex, so `[ERROR]` is a
    // character class matching any field with an `E`, `R` or `O` in it.
    assert_eq!(
        filter_mode("[ERROR]"),
        FilterMode::Regex,
        "an unprefixed table filter must stay a regex"
    );
    assert!(
        table_matches("[ERROR]", &[FIELD_WITH_E]),
        "on the table `[ERROR]` is a character class and a bare `e` is a member"
    );
    assert!(
        !table_matches("[ERROR]", &[FIELD_WITHOUT_ERO]),
        "a field with no E, R or O must still miss — otherwise the match above would \
         prove nothing"
    );
    record_pair(
        "[ERROR]",
        "[ERROR]",
        FIELD_WITH_E,
        table_matches("[ERROR]", &[FIELD_WITH_E]),
        log_matches("[ERROR]", &[FIELD_WITH_E]),
    );
    record_pair(
        "[ERROR]",
        "[ERROR]",
        FIELD_WITHOUT_ERO,
        table_matches("[ERROR]", &[FIELD_WITHOUT_ERO]),
        log_matches("[ERROR]", &[FIELD_WITHOUT_ERO]),
    );

    // Stated as the one crossing claim: on the *same* haystack the two
    // surfaces reach opposite verdicts about the same input, and that is
    // correct on both.
    assert!(
        table_matches("[ERROR]", &[BARE_LINE]) && !log_matches("[ERROR]", &[BARE_LINE]),
        "the defaults are distinct: the table reads `[ERROR]` as a class and matches this \
         unbracketed line, the log surface reads it literally and does not"
    );
    record(
        "  => same input, same haystack, opposite verdicts. Both are correct. Unify the\n     \
         defaults and one of these two lines becomes a lie.",
    );

    // ---- 2. `!` inverts identically ---------------------------------------
    section("2. `!` — one inverse, shared by both surfaces");

    // Patterns whose meaning is the same on both surfaces once the log side
    // opts into regex with `/.../`, so the two can be compared row for row.
    let haystack_sets: &[&[&str]] = &[
        &[BRACKETED_LINE],
        &[BARE_LINE],
        &[FIELD_WITH_E, FIELD_WITHOUT_ERO],
        &[FIELD_WITHOUT_ERO],
        &[],
    ];
    for pattern in ["disk", "e2", "d.sk", "nothing-here"] {
        for haystacks in haystack_sets {
            let table_plain = table_matches(pattern, haystacks);
            let table_inverted = table_matches(&format!("!{pattern}"), haystacks);
            let log_plain = log_matches(&format!("/{pattern}/"), haystacks);
            let log_inverted = log_matches(&format!("!/{pattern}/"), haystacks);

            assert_eq!(
                table_inverted, !table_plain,
                "table `!{pattern}` must be exactly the negation of `{pattern}` over {haystacks:?}"
            );
            assert_eq!(
                log_inverted, !log_plain,
                "log `!/{pattern}/` must be exactly the negation of `/{pattern}/` over \
                 {haystacks:?}"
            );
            assert_eq!(
                table_inverted, log_inverted,
                "the two surfaces must agree on the inverted verdict for {pattern:?} over \
                 {haystacks:?} — `!` is implemented once, in gcp::matcher::Predicate"
            );
            record_pair(
                &format!("!{pattern}"),
                &format!("!/{pattern}/"),
                &format!("{haystacks:?}"),
                table_inverted,
                log_inverted,
            );
        }
    }

    // ---- 3. Every mode is case-insensitive on both surfaces ----------------
    section("3. Case-insensitivity, in both directions, on every mode");

    for (needle, haystack) in [("api", "API-PROD"), ("API", "api-dev-1")] {
        // Table: regex (default) and fuzzy.
        assert!(
            table_matches(needle, &[haystack]),
            "table regex {needle:?} must match {haystack:?} case-insensitively"
        );
        assert!(
            table_matches(&format!("-f {needle}"), &[haystack]),
            "table fuzzy {needle:?} must match {haystack:?} case-insensitively"
        );
        // Logs: substring (default), fuzzy and regex.
        assert!(
            log_matches(needle, &[haystack]),
            "log substring {needle:?} must match {haystack:?} case-insensitively"
        );
        assert!(
            log_matches(&format!("~{needle}"), &[haystack]),
            "log fuzzy {needle:?} must match {haystack:?} case-insensitively"
        );
        assert!(
            log_matches(&format!("/{needle}/"), &[haystack]),
            "log regex {needle:?} must match {haystack:?} case-insensitively"
        );
        // The controls for the five assertions above: the same five modes on
        // the same haystack, asked about a needle that is absent from it in
        // either case. Every one must miss, so a matcher that accepted every
        // haystack fails this block instead of passing it.
        for absent in ABSENT_NEEDLES {
            assert!(
                !table_matches(absent, &[haystack]),
                "table regex {absent:?} must miss {haystack:?} — otherwise the case-insensitive \
                 match above proves nothing"
            );
            assert!(
                !table_matches(&format!("-f {absent}"), &[haystack]),
                "table fuzzy {absent:?} must miss {haystack:?} — otherwise the case-insensitive \
                 match above proves nothing"
            );
            assert!(
                !log_matches(absent, &[haystack]),
                "log substring {absent:?} must miss {haystack:?} — otherwise the \
                 case-insensitive match above proves nothing"
            );
            assert!(
                !log_matches(&format!("~{absent}"), &[haystack]),
                "log fuzzy {absent:?} must miss {haystack:?} — otherwise the case-insensitive \
                 match above proves nothing"
            );
            assert!(
                !log_matches(&format!("/{absent}/"), &[haystack]),
                "log regex {absent:?} must miss {haystack:?} — otherwise the case-insensitive \
                 match above proves nothing"
            );
            record(&format!(
                "  {absent:<6} vs {haystack:<10} | table: regex+fuzzy miss  | log: substring+fuzzy+regex miss  (control)"
            ));
        }
        record(&format!(
            "  {needle:<6} vs {haystack:<10} | table: regex+fuzzy MATCH | log: substring+fuzzy+regex MATCH"
        ));
    }

    // ---- 4. One fuzzy subsequence semantic ---------------------------------
    section("4. Fuzzy — `-f needle` on the table and `~needle` on the logs agree");

    // Positive, missing char, out of order, and a repeated needle char that
    // the haystack supplies only once: the four ways a subsequence test can
    // differ between two hand-written copies of it.
    let fuzzy_cases: &[(&str, &[&str])] = &[
        ("dbp", &["db-primary-01"]),
        ("dbz", &["db-primary-01"]),
        ("pbd", &["db-primary-01"]),
        ("aa", &["api"]),
        ("aa", &["aabb"]),
        ("ERR", &["an error occurred"]),
        ("disk", &[BRACKETED_LINE]),
    ];
    for (needle, haystacks) in fuzzy_cases {
        let table = table_matches(&format!("-f {needle}"), haystacks);
        let log = log_matches(&format!("~{needle}"), haystacks);
        assert_eq!(
            table, log,
            "fuzzy {needle:?} over {haystacks:?} must mean the same on both surfaces — one \
             subsequence implementation lives in gcp::matcher"
        );
        record_pair(
            &format!("-f {needle}"),
            &format!("~{needle}"),
            &format!("{haystacks:?}"),
            table,
            log,
        );
    }
    // The pairs above are only worth asserting because they are not all the
    // same answer: at least one match and at least one miss.
    assert!(
        fuzzy_cases
            .iter()
            .any(|(needle, hay)| table_matches(&format!("-f {needle}"), hay))
            && fuzzy_cases
                .iter()
                .any(|(needle, hay)| !table_matches(&format!("-f {needle}"), hay)),
        "the fuzzy corpus must contain both matches and misses, or agreement is vacuous"
    );

    // ---- 5. One regex-error reason, one line, on both surfaces -------------
    section("5. Invalid regex — the same one-line reason on both surfaces");

    // Both grammars render this reason inline in a header truncated to the
    // terminal width, so the shared helper must return the cause and only the
    // cause. The second pattern is the adversarial one: it contains the very
    // `error: ` prefix the helper scans for, and the regex crate echoes the
    // user's pattern in the line above the caret — so a forward scan would
    // strip the prefix off that echo and report the operator's own text back
    // at them instead of the cause.
    for (pattern, cause) in [
        ("a(b", "unclosed group"),
        ("error: [a", "unclosed character class"),
    ] {
        let table_reason = compile_filter(pattern)
            .expect_err("an invalid pattern must not compile")
            .message;
        let log_reason = TextFilter::parse(&format!("/{pattern}/"))
            .expect_err("an invalid pattern must not compile")
            .to_string();

        for (surface, reason) in [("table", &table_reason), ("log", &log_reason)] {
            assert!(
                !reason.contains('\n'),
                "the {surface} reason for {pattern:?} must fit one header line: {reason:?}"
            );
            assert!(
                reason.contains(cause),
                "the {surface} reason for {pattern:?} must name the cause {cause:?}: {reason:?}"
            );
        }
        assert_eq!(
            table_reason, log_reason,
            "both surfaces route their regex errors through gcp::matcher::regex_reason, so \
             {pattern:?} must read identically on each"
        );
        record(&format!(
            "  {pattern:<12} | table {table_reason:<40} | log {log_reason}"
        ));
    }

    write_transcript();
}
