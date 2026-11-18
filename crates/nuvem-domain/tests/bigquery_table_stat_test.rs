//! Pins [`TableStat::known`], the one accessor that decides whether a number
//! BigQuery never answered can be rendered as a zero.
//!
//! This lives in the domain crate rather than in the root crate's suite on
//! purpose: an accessor in `nuvem-domain` is pinned by a test that
//! `nuvem-domain`'s own test binary runs, so the crate can be checked on its
//! own. The root suite's rendering tests cover the same behaviour end to end
//! but only through the binary crate.

use nuvem_domain::gcp::resource::TableStat;

/// A table that reported a number answers with that number, zero included.
///
/// The zero is the case worth spelling out: `Known(0)` is a table that ran its
/// `show` and came back saying it holds nothing. Answering `None` for it would
/// turn a measured emptiness into "we never asked".
#[test]
fn a_known_stat_answers_with_the_number_it_holds() {
    assert_eq!(
        TableStat::Known(0).known(),
        Some(0),
        "a table that reported zero rows reported a number, and the number is \
         zero: this is a measurement, not a missing answer"
    );
    assert_eq!(
        TableStat::Known(1).known(),
        Some(1),
        "the payload is passed through, not replaced by a constant"
    );
    assert_eq!(
        TableStat::Known(-1).known(),
        Some(-1),
        "and passed through with its sign: BigQuery's int64 fields are \
         reparsed from wire strings, so a negative is what a malformed answer \
         looks like once it has parsed, and it must not read as some other \
         number"
    );
    assert_eq!(
        TableStat::Known(i64::MAX).known(),
        Some(i64::MAX),
        "including at the far end of the int64 range these fields are"
    );
}

/// A call that did not answer stays unanswered.
///
/// `Unavailable` is the shape of `bigquery.tables.list` granted with
/// `bigquery.tables.get` denied — an ordinary read-only IAM shape. The caller
/// formats the number itself, so anything other than `None` here becomes a
/// byte size or a row count printed for a table nobody could read.
#[test]
fn an_unavailable_stat_answers_with_nothing_at_all() {
    assert_eq!(
        TableStat::Unavailable.known(),
        None,
        "a `show` that never answered is not a table holding zero: the absence \
         has to survive the accessor, or the renderer prints a claim about the \
         data where the truth is a claim about the call"
    );
}

/// The zero and the absence are different values, stated as the distinction
/// itself rather than as two separate mappings.
#[test]
fn a_measured_zero_is_not_an_unanswered_call() {
    assert_ne!(
        TableStat::Known(0).known(),
        TableStat::Unavailable.known(),
        "collapsing these two is the defect this type exists to prevent"
    );
}
