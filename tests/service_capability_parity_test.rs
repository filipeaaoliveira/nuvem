//! The capability predicates agree with the code that does the work.
//!
//! `app::service_capabilities` states, once, which services answer batch
//! marking, the managed tunnel and the object browser's keys. Exhaustive
//! matches make a *new* service a compile error there, but they cannot make a
//! *wrong* answer one: nothing stops the predicate saying `true` where the
//! reducer returns early.
//!
//! So each predicate is checked against a second, independently written
//! statement of the same fact — the reducer's own behaviour, or a projection
//! that was hand-set per service — walked over every `ResourceServiceKind`.
//! Either one drifting fails this.

use nuvem::app::projection::project_resources;
use nuvem::app::reducer::root::reduce;
use nuvem::app::service_capabilities::{browses_objects, supports_marks, supports_tunnel};
use nuvem::app::{Action, AppState, ResourcesAction};
use nuvem_domain::gcp::resource::ResourceServiceKind;

fn app_on(kind: ResourceServiceKind) -> AppState {
    let mut app = AppState::new();
    app.root.resources.select_service(kind);
    app
}

/// `supports_marks` against the table projection's `show_mark_column`, which is
/// set by hand in each service's model builder.
///
/// Two hand-written statements of one fact, in different files, compared for
/// every service. The projection is the right counterpart because it is what
/// the operator sees: a service whose predicate says it marks but whose table
/// draws no mark column would teach a key whose effect is invisible.
#[test]
fn the_mark_predicate_agrees_with_the_table_that_draws_the_marks() {
    let mut compared = 0usize;
    for kind in ResourceServiceKind::ALL {
        let app = app_on(kind);
        let projection = project_resources(&app.root.resources);
        let Some(model) = projection.table.as_ref() else {
            // No table for this service in the empty state; the projection has
            // nothing to disagree with.
            continue;
        };
        assert_eq!(
            supports_marks(kind),
            model.show_mark_column,
            "{kind:?}: supports_marks says {} but its table draws \
             show_mark_column = {}",
            supports_marks(kind),
            model.show_mark_column
        );
        compared += 1;
    }

    // The `continue` above skips a service whose empty state builds no table,
    // and a version of this that skipped them all would pass having compared
    // nothing.
    assert_eq!(
        compared,
        ResourceServiceKind::ALL.len(),
        "only {compared} of {} services had a table to compare against",
        ResourceServiceKind::ALL.len()
    );
}

/// `supports_tunnel` against the reducer: `t` on a service without a plan must
/// leave no tunnel preview behind.
///
/// The negative direction only. A service that *does* have a plan needs a
/// selected row with the right shape to produce one, which is what the
/// per-service tunnel tests already cover; what this adds is that no service the
/// predicate excludes can quietly open a preview.
///
/// That leaves a predicate answering `true` for everything unexamined here —
/// the loop body would never run. `help_service_scope_test`'s
/// `every_service_is_taught_a_scoped_key_exactly_when_it_answers_it` is what
/// closes that direction; the mutation gate found it open.
#[test]
fn no_service_the_tunnel_predicate_excludes_opens_a_preview() {
    for kind in ResourceServiceKind::ALL {
        if supports_tunnel(kind) {
            continue;
        }
        let mut app = app_on(kind);
        reduce(&mut app, &Action::Resources(ResourcesAction::RequestTunnel));
        assert!(
            app.root.resources.tunnel_launch_preview().is_none(),
            "{kind:?} is excluded by supports_tunnel but `t` opened a preview"
        );
    }
}

/// `browses_objects` against the reducer's own early return: the two keys that
/// guard on `active_service() != CloudStorage` must be no-ops everywhere else.
#[test]
fn no_service_the_object_browser_predicate_excludes_answers_its_keys() {
    for kind in ResourceServiceKind::ALL {
        if browses_objects(kind) {
            continue;
        }
        for action in [
            ResourcesAction::OpenBucketMetadata,
            ResourcesAction::NextPage,
        ] {
            let mut app = app_on(kind);
            let effects = reduce(&mut app, &Action::Resources(action.clone()));
            assert!(
                effects.is_empty(),
                "{kind:?} is excluded by browses_objects but {action:?} \
                 produced {effects:?}"
            );
        }
    }
}

/// Exactly one service browses objects, and it is the one whose keys the
/// catalogue offers.
///
/// Stated because the two tests above are both negative: a predicate that
/// answered `false` for everything would satisfy them and teach nobody.
#[test]
fn each_predicate_is_true_of_at_least_one_service() {
    for (name, predicate) in [
        (
            "supports_marks",
            supports_marks as fn(ResourceServiceKind) -> bool,
        ),
        ("supports_tunnel", supports_tunnel),
        ("browses_objects", browses_objects),
    ] {
        assert!(
            ResourceServiceKind::ALL.into_iter().any(predicate),
            "{name} excludes every service, so it teaches nobody"
        );
    }
}

/// The exact services each predicate includes, read off the reducer.
///
/// The tests above check that an *excluded* service stays inert, which leaves a
/// predicate answering `true` for everything unexamined — their loop bodies
/// never run. The mutation gate found that gap: `replace supports_tunnel -> bool
/// with true` survived all of them, and it survives an `iff` against the taught
/// help set too, because that reads the same function on both sides.
///
/// So the set is stated here, from the reducer's own branches:
/// `request_tunnel` builds a plan for Compute Engine and Cloud SQL and `return`s
/// for the rest; `toggle_mark` dispatches for the same two and has empty blocks
/// for the rest; the object-browser keys guard on `active_service() !=
/// CloudStorage`.
///
/// A hand-written list in a test is usually the thing to avoid, and this one is
/// safe for a specific reason: it cannot *hide* a service, because the predicate
/// it pins is an exhaustive match that a new variant breaks at compile time, and
/// because the negative tests above independently refuse any service the
/// reducer declines. Widening a capability means changing the reducer, the
/// predicate and this line together, which is the deliberate edit it should be.
#[test]
fn each_predicate_includes_exactly_the_services_the_reducer_branches_to() {
    let included = |predicate: fn(ResourceServiceKind) -> bool| -> Vec<ResourceServiceKind> {
        ResourceServiceKind::ALL
            .into_iter()
            .filter(|kind| predicate(*kind))
            .collect()
    };

    assert_eq!(
        included(supports_marks),
        vec![
            ResourceServiceKind::ComputeInstances,
            ResourceServiceKind::CloudSql
        ],
        "supports_marks no longer matches `ResourcesState::toggle_mark`"
    );
    assert_eq!(
        included(supports_tunnel),
        vec![
            ResourceServiceKind::ComputeInstances,
            ResourceServiceKind::CloudSql
        ],
        "supports_tunnel no longer matches `reducer::resources::request_tunnel`"
    );
    assert_eq!(
        included(browses_objects),
        vec![ResourceServiceKind::CloudStorage],
        "browses_objects no longer matches the object browser's guards"
    );
}
