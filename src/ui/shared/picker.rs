//! The scrolling window shared by the picker overlays.

use std::ops::Range;

/// The slice of a picker's rows to paint, keeping the selection on screen.
///
/// Both picker overlays — the related-resource jump picker in `ui::resources`
/// and the configuration context picker in `ui::configurations` — carried their
/// own copy of this arithmetic once, having arrived at it
/// independently. They are one function, and keeping them one is the point: a
/// picker whose window drifts strands rows its list still offers, which is the
/// defect fixed in the jump picker and would otherwise be free to
/// reappear in either.
///
/// The window is centred on `selected` and slides only as far as the ends of
/// the list, so it stays full while rows remain: a window allowed to run short
/// paints blank space with rows left to show.
///
/// Takes the window's three inputs and no `Rect`, deliberately. Turning an area
/// into a `capacity` is each picker's own chrome arithmetic — the jump picker
/// owes a blank line and a footer, the context picker a filter line and a
/// footer whose length varies with its filter state — and folding that in would
/// put both pickers' layouts inside one helper, recreating the coupling this
/// exists to remove.
///
/// A `capacity` of zero yields an empty window rather than panicking. Neither
/// caller can reach it — each tests that its chrome fits before rendering any
/// rows, which leaves room for at least one — but the helper is total on its
/// own terms rather than resting on a precondition kept in two other files.
pub fn viewport_window(total: usize, selected: usize, capacity: usize) -> Range<usize> {
    let start = selected
        .saturating_sub(capacity / 2)
        .min(total.saturating_sub(capacity));
    let end = (start + capacity).min(total);
    start..end
}
