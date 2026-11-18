//! The scrolling window both picker overlays paint through.
//!
//! `viewport_window` is the arithmetic the related-resource jump picker and the
//! configuration context picker each carried a private copy of until this
//! change. What is pinned here is the window's behaviour, stated as properties
//! rather than as the formula again: a test that restates
//! `selected.saturating_sub(capacity / 2).min(...)` passes for any expression
//! equal to itself, including a wrong one copied faithfully from the caller it
//! was extracted from.
//!
//! The companion guard — that both callers still route through it — lives in
//! `tests/picker_viewport_drift_test.rs`.

use nuvem::ui::shared::picker::viewport_window;

/// The grid every property below is checked over.
///
/// Small and exhaustive rather than large and sampled: the window's behaviour
/// changes at the ends of the list and at the point where the list outgrows the
/// viewport, and those boundaries all live within a few rows of zero. A sweep
/// to eight crosses every one of them in both directions, including the
/// degenerate `capacity == 0` and `total == 0` cases a random sample would
/// mostly miss.
fn cases() -> impl Iterator<Item = (usize, usize, usize)> {
    (0..=8).flat_map(|total| {
        (0..=8).flat_map(move |capacity| {
            (0..total.max(1)).map(move |selected| (total, selected, capacity))
        })
    })
}

/// A window short while rows remain is the bug that makes a picker look empty.
#[test]
fn the_window_is_as_full_as_the_rows_allow() {
    for (total, selected, capacity) in cases() {
        let window = viewport_window(total, selected, capacity);

        assert_eq!(
            window.end - window.start,
            capacity.min(total),
            "total={total} selected={selected} capacity={capacity}"
        );
    }
}

/// A modal whose selection has scrolled off the painted rows is unusable: the
/// cursor is the only feedback either picker gives.
#[test]
fn the_selection_is_always_inside_the_window() {
    for (total, selected, capacity) in cases() {
        if total == 0 || capacity == 0 {
            continue;
        }
        let window = viewport_window(total, selected, capacity);

        assert!(
            window.contains(&selected),
            "selected {selected} outside {window:?} for total={total} capacity={capacity}"
        );
    }
}

/// Both callers slice their row vector with this range, so a window past the
/// end is a panic in the renderer rather than a wrong picture.
#[test]
fn the_window_never_runs_past_the_rows() {
    for (total, selected, capacity) in cases() {
        let window = viewport_window(total, selected, capacity);

        assert!(
            window.start <= window.end && window.end <= total,
            "{window:?} escapes total={total} for selected={selected} capacity={capacity}"
        );
    }
}

/// How the window scrolls: it keeps the selection centred, and gives that up
/// only at the ends of the list, where centring would mean painting rows that
/// do not exist.
///
/// This is the property that pins *which* window of the several that would
/// satisfy the three above. Without it, a picker that jumped a page at a time
/// or one that never scrolled until the selection hit the edge would both pass.
#[test]
fn the_window_is_centred_on_the_selection_except_at_the_ends() {
    for (total, selected, capacity) in cases() {
        if total == 0 || capacity == 0 || capacity >= total {
            continue;
        }
        let window = viewport_window(total, selected, capacity);

        assert!(
            window.start == 0 || window.end == total || selected - window.start == capacity / 2,
            "{window:?} is neither flush nor centred for total={total} selected={selected} capacity={capacity}"
        );
    }
}

/// Scrolling one row at a time moves the window by at most one row. A window
/// that lurches is the page-at-a-time behaviour the centring property rules out
/// in the middle of the list; this rules it out at the ends too, where the
/// centring property deliberately stops applying.
#[test]
fn moving_the_selection_by_one_moves_the_window_by_at_most_one() {
    for (total, selected, capacity) in cases() {
        if total == 0 || selected + 1 >= total {
            continue;
        }
        let here = viewport_window(total, selected, capacity);
        let next = viewport_window(total, selected + 1, capacity);

        assert!(
            next.start >= here.start && next.start - here.start <= 1,
            "window jumped from {here:?} to {next:?} for total={total} capacity={capacity}"
        );
    }
}
