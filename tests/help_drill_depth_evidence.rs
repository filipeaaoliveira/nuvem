//! Evidence — the help overlay tells the two drilled levels apart.
//!
//! The overlay taught `Open / enter` in BigQuery's schema view, where `Enter`
//! does nothing. `HelpContext::ResourceDrilldown` is built from
//! `ResourceList`'s bindings with `RES_SORT` and `RES_COLUMN` filtered out and
//! `RES_ESC_BACK` swapped for `RES_LEAVE_DRILLDOWN`; `RES_OPEN` survived,
//! because one catalogue arm served two levels that differ in what `Enter`
//! does — on a table row it opens the schema, on a schema field it opens
//! nothing.
//!
//! The frame footer already told them apart, through
//! `drilldown_opens_another_level`, which reads live state. The overlay could
//! not, because `HelpContext` is derived from the keymap `Context`, which
//! carried one drill-down flag and no depth.
//!
//! Dropping `RES_OPEN` from the arm was not enough on its own, and that is why
//! three slices deferred this rather than doing it badly:
//! `keymap_help_parity_test::every_dispatched_key_has_catalog_metadata`
//! requires every key the keymap dispatches in a context to carry catalogue
//! metadata for it. `Enter` dispatched at both levels, so a catalogue that
//! stopped teaching it at the leaf would have been refused by the gate that
//! exists to keep the two in step. The keymap has to stop dispatching it, and
//! the observable behaviour is unchanged either way: the reducer already
//! refused to open anything from a column row.
//!
//! Three properties are pinned here — the catalogue, the dispatch, and the
//! level *above* still teaching and dispatching it, which is what says this
//! narrowed the claim rather than deleted it.

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
use nuvem::app::projection::binding::{Binding, HelpContext, RES_OPEN, help_model};
use nuvem::app::state::resources::BigQueryView;
use nuvem::app::{Action, ResourcesAction};
use nuvem::input::keymap::action_for_key;

use crate::support::keymap_context::context_for;

use crate::bigquery_drilldown_test::{drilled_in, drilled_into_table, schema_field, table};
use nuvem::app::Workspace;

/// The drill-down fixtures reduce actions straight into `ResourcesState` and
/// never navigate, so `root.workspace` is still `Home` when they return — and
/// `normal_view_context` resolves the *workspace* first. Without this the
/// overlay under test is Home's, which teaches no resource bindings at all and
/// would satisfy a "does not teach Enter" assertion for entirely the wrong
/// reason.
fn in_the_resources_workspace(mut app: nuvem::app::AppState) -> nuvem::app::AppState {
    app.root.workspace = Workspace::Resources;
    app
}

fn press(code: KeyCode) -> KeyEvent {
    KeyEvent {
        code,
        modifiers: KeyModifiers::empty(),
        kind: KeyEventKind::Press,
        state: KeyEventState::empty(),
    }
}

/// Whether the overlay the operator would see teaches `Enter`.
///
/// Read out of `help_model` rather than out of `context_bindings` directly:
/// the defect was not that the catalogue arm was wrong in isolation, it was
/// that the *resolution* from live state landed on an arm that was right for
/// the other level. Asking the model is what exercises that resolution.
fn overlay_teaches_open(app: &nuvem::app::AppState) -> bool {
    help_model(&app.root)
        .sections
        .iter()
        .flat_map(|section| section.bindings.iter())
        .any(|binding| binding.key == RES_OPEN.key && binding.label == RES_OPEN.label)
}

#[test]
fn the_overlay_does_not_teach_open_inside_the_schema_view() {
    let app = in_the_resources_workspace(drilled_into_table(
        "sales",
        vec![table("sales", "orders")],
        vec![schema_field("order_id")],
    ));
    assert_eq!(
        app.root.resources.bigquery_view(),
        BigQueryView::Schema,
        "precondition: the fixture is inside the schema view"
    );

    assert!(
        !overlay_teaches_open(&app),
        "the overlay taught `{}` on a column row, where Enter opens nothing — \
         the footer has never hinted it here",
        RES_OPEN.label
    );
}

/// The level above is untouched: `Enter` on a table row opens that table's
/// column schema, and the overlay says so.
///
/// Without this the fix above is indistinguishable from deleting `RES_OPEN`
/// from the drill-down arm outright, which would make the overlay wrong on the
/// level where it was right.
#[test]
fn the_overlay_still_teaches_open_inside_the_tables_view() {
    let app = in_the_resources_workspace(drilled_in("sales", vec![table("sales", "orders")]));
    assert_eq!(
        app.root.resources.bigquery_view(),
        BigQueryView::Tables,
        "precondition: the fixture is inside the tables view"
    );

    assert!(
        overlay_teaches_open(&app),
        "Enter on a table row opens its column schema, so the overlay must \
         still teach `{}` here",
        RES_OPEN.label
    );
}

/// The keymap stops dispatching `Enter` at the bottom level.
///
/// This is the half the catalogue edit could not be made without. It is also
/// the half that is *not* an observable change: `open_selected_bigquery_table`
/// already refused to open anything from a column row, so the operator saw
/// nothing happen before and sees nothing happen now. What changes is that the
/// keymap and the catalogue now agree about it, which is the agreement
/// `keymap_help_parity_test` enforces.
#[test]
fn enter_dispatches_nothing_at_the_bottom_of_the_drill() {
    let leaf = context_for(HelpContext::ResourceDrilldownLeaf);
    assert_eq!(
        action_for_key(press(KeyCode::Enter), &leaf),
        None,
        "a column is the bottom of the hierarchy: Enter has nothing to open"
    );

    let branch = context_for(HelpContext::ResourceDrilldown);
    assert_eq!(
        action_for_key(press(KeyCode::Enter), &branch),
        Some(Action::Resources(ResourcesAction::OpenSelected)),
        "and one level up it still opens the selected table's schema"
    );
}

/// The leaf catalogue is the branch catalogue minus exactly `RES_OPEN`.
///
/// Written as a difference rather than as a literal list so the two arms
/// cannot drift apart: a binding added to the drill-down arm reaches both
/// levels, which is what "the same view one level down" should mean, and this
/// fails the day someone adds one to only one of them.
#[test]
fn the_leaf_catalogue_differs_from_the_branch_by_one_binding() {
    use nuvem::app::projection::binding::context_bindings;

    let branch = context_bindings(HelpContext::ResourceDrilldown);
    let leaf = context_bindings(HelpContext::ResourceDrilldownLeaf);

    let dropped: Vec<&Binding> = branch.iter().filter(|b| !leaf.contains(b)).collect();
    assert_eq!(
        dropped,
        vec![&RES_OPEN],
        "the leaf level teaches everything the level above does except Enter"
    );
    let added: Vec<&Binding> = leaf.iter().filter(|b| !branch.contains(b)).collect();
    assert!(
        added.is_empty(),
        "the leaf level teaches nothing the level above does not: {added:?}"
    );
}
