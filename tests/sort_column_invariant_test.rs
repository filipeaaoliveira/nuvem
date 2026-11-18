//! One walk over `ResourceServiceKind::ALL` pinning the sort-column
//! vocabulary of *every* service: sorting by the column under the cursor
//! sorts by that column, for every sortable column of every table.
//!
//! # Why a walk and not a pin per service
//!
//! Each service spells its sort vocabulary as a pair of inherent methods on
//! its own `*SortField` enum — `from_column(index) -> Option<Self>` and
//! `column(self) -> usize` — and the pair is a round trip nobody was
//! checking. The networks slice found three of its arms unexercised in
//! one sweep (`from_column`'s 0 and 2, and `column` itself, which could have
//! answered a constant `1` for every field unnoticed), and the same shape had
//! already been found once before. Three instances of one defect class is
//! past the point where pinning instances beats pinning the class: a copy per
//! service leaves the ten services that shipped before this ticket untested,
//! and leaves the *next* service untested until someone remembers to copy the
//! pin again.
//!
//! This walk covers all of them, including the ones nobody has written yet.
//!
//! # Why it needs no rows
//!
//! Both halves are index arithmetic over the column catalogue, so the
//! assertion holds on an empty collection: `sort_selected_column` resolves the
//! cursor's display position through `from_column`, and `sort_column_index`
//! maps the stored field back through `column`. That is what lets one walk
//! cover every service without a sample row per service — see the note in
//! `vpc_networks_state_test` for the sibling property that is *not* expressible
//! this way.
//!
//! # What it catches
//!
//! A deleted `from_column` arm makes that index resolve to `None`, so
//! `sort_selected_column` returns early and the stored field stays where it
//! was — the cursor and the sort indicator disagree, which is what an operator
//! sees as "pressing sort on this column sorted by a different one". A
//! constant `column` collapses every field onto one display position, so the
//! indicator lands on the wrong header for every column but one.

use nuvem::app::projection::columns::catalog_meta;
use nuvem::app::state::resources::ResourcesState;
use nuvem_domain::gcp::resource::ResourceServiceKind;

/// The display positions a cursor can stop on: the sortable columns of the
/// service's catalogue, in catalogue order.
///
/// Unsortable columns (a flattened `labels` cell, whose ordering would mean
/// nothing) are skipped by `select_next_column` itself, so they are skipped
/// here too rather than asserted about.
fn sortable_positions(kind: ResourceServiceKind) -> Vec<usize> {
    catalog_meta(kind)
        .iter()
        .enumerate()
        .filter(|(_, column)| column.sortable)
        .map(|(index, _)| index)
        .collect()
}

/// Sorting by the column the cursor is on sorts by *that* column, for every
/// sortable column of every service.
///
/// The two directions are pinned together on purpose. `sort_selected_column`
/// reads the cursor through `from_column`; `sort_column_index` reports the
/// stored field through `column`. Asserting they agree at every position
/// makes the pair a round trip, so neither half can drift alone.
#[test]
fn sorting_by_the_selected_column_sorts_by_that_column_for_every_service() {
    let mut disagreements: Vec<String> = Vec::new();

    for kind in ResourceServiceKind::ALL {
        let positions = sortable_positions(kind);
        assert!(
            !positions.is_empty(),
            "{kind:?} has no sortable column at all, so its table cannot be \
             ordered by anything"
        );

        // A sortable position to park the sort on before each measurement.
        // Every service defaults to sorting by its first column, so a
        // `from_column` arm deleted at *that* position is invisible if the
        // field is already there: the failed lookup returns early, the stored
        // field stays put, and the indicator reads correct by accident. The
        // walk therefore moves the sort somewhere else first, so every
        // position under test has to be actually reached rather than merely
        // already occupied. This test was measured missing exactly that mutant
        // before the park was added.
        let park = *positions
            .last()
            .expect("checked non-empty, and a one-column table parks on itself");

        for position in positions.iter().copied() {
            let mut state = ResourcesState::default();
            state.select_service(kind);

            // The cursor clamps at both ends rather than wrapping
            // (`next_sortable_column` stays put when nothing sortable is to
            // its right), so reaching a position means stepping *towards* it.
            //
            // A position the cursor cannot reach is a finding, not a reason to
            // abort: it is what a deleted `from_column` arm looks like from
            // the outside, since the same lookup decides which columns the
            // cursor may stop on. Returning `false` keeps the walk going so
            // one run names every offending service rather than the first.
            let drive_to = |state: &mut ResourcesState, target: usize| -> bool {
                let mut guard = 0;
                while state.selected_column() != target {
                    let before = state.selected_column();
                    if before < target {
                        state.select_next_column();
                    } else {
                        state.select_previous_column();
                    }
                    guard += 1;
                    if state.selected_column() == before || guard > catalog_meta(kind).len() + 1 {
                        return false;
                    }
                }
                true
            };

            if park != position && drive_to(&mut state, park) {
                state.sort_selected_column();
            }

            if !drive_to(&mut state, position) {
                disagreements.push(format!(
                    "  {kind:?}: the column cursor cannot stop on display \
                     position {position}, so that column cannot be sorted by \
                     at all"
                ));
                continue;
            }
            state.sort_selected_column();

            if state.sort_column_index() != position {
                disagreements.push(format!(
                    "  {kind:?}: sorting the column at display position \
                     {position} left the sort indicator on {}",
                    state.sort_column_index()
                ));
            }
        }
    }

    assert!(
        disagreements.is_empty(),
        "{} column(s) sort by something other than themselves — the cursor and \
         the sort indicator disagree:\n{}",
        disagreements.len(),
        disagreements.join("\n")
    );
}

/// A second press of the same column reverses rather than re-sorting, for
/// every service.
///
/// The toggle lives beside the `from_column` lookup in every one of these
/// `sort_selected_column` arms, so it is the same code this walk is already
/// standing in front of — and a service whose arm sets `Ascending`
/// unconditionally would leave an operator unable to reverse that table at
/// all.
#[test]
fn pressing_the_same_column_twice_reverses_the_direction_for_every_service() {
    let mut stuck: Vec<String> = Vec::new();

    for kind in ResourceServiceKind::ALL {
        let mut state = ResourcesState::default();
        state.select_service(kind);

        state.sort_selected_column();
        let first = state.sort_direction();
        state.sort_selected_column();
        let second = state.sort_direction();

        if first == second {
            stuck.push(format!(
                "  {kind:?}: two presses of one column both left the direction \
                 {first:?}"
            ));
        }
    }

    assert!(
        stuck.is_empty(),
        "{} service(s) cannot reverse their sort:\n{}",
        stuck.len(),
        stuck.join("\n")
    );
}
