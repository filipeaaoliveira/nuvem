//! One walk over `ResourceServiceKind::ALL` asserting that every service is
//! wired at every per-service site (slice 1).
//!
//! # Why one test and not nine
//!
//! Adding a service variant touches roughly thirty files, and several of the
//! sites it must touch do not fail to compile when they are missed. The case
//! ("three services have no card, and two that do have never painted") was the
//! symptom of exactly that. Split across nine tests, the author adding a
//! service fixes one, reruns, fixes the next, reruns — nine round trips to
//! learn a list this test prints in one. The list is the point: it *is* the
//! worklist for adding a service, so it accumulates every failure and reports
//! them together rather than aborting on the first.
//!
//! # What is not here, and why
//!
//! Sites whose per-kind dispatch is an exhaustive `match` are already
//! compiler-enforced, and a second copy here would only add a place to forget:
//! `project_detail_body`'s per-kind arm, `Effect` selection, the demo runner
//! routes (`demo_runner_test::every_service_kind_has_a_demo_route`), the
//! context-snapshot round trip, and `label_filter_field`. This test covers the
//! sites the compiler cannot see: registry lookup by kind, catalogs resolved
//! through data, projections built by iteration, and the `&[]`/`None` answers
//! an exhaustive match is still free to return.
//!
//! `identity_layout` is checked by omission: it is derived from
//! `catalog_meta`, so a length assertion against that same catalog cannot
//! fail, and the catalog check below is the one that carries it. The Home
//! inventory card is absent for the same reason: `project_home_overview`
//! builds the inventory by mapping over `ALL` and stamping each card with the
//! iterated kind, so a per-kind membership assertion cannot fail by the
//! omission it would claim to guard, and
//! `tests/home_overview_projection_test.rs:175` already asserts the inventory
//! kinds equal `ALL` exactly. Every check that is here was confirmed to fail
//! by breaking one service's wiring, so a green run means the sites are wired
//! rather than that the checks are inert.
//!
//! `ResourceServiceKind::ALL` itself — the enumeration every check below walks
//! — cannot be guarded from here, because a test that iterates a stale `ALL`
//! is stale with it. That hole is closed by construction instead: `ALL` is
//! generated from the enum's own variant list by `service_kinds!` in
//! `nuvem_domain::gcp::resource`.

use nuvem::app::projection::columns::{IDENTITY_COLUMN, catalog_meta};
use nuvem::app::projection::project_service_navigator;
use nuvem::app::service_registry::{load_effect, service_id_for_resource};
use nuvem::app::state::resources::{ResourceLoadPhase, ResourcesState};
use nuvem::app::state::root::RootState;
use nuvem::config::jumps::fields_for;
use nuvem_domain::gcp::configuration::{GcloudConfigurationName, GcloudContext};
use nuvem_domain::gcp::resource::ResourceServiceKind;
use ratatui::layout::Rect;
use std::time::Instant;

/// A per-service site that did not resolve for one kind, in the terms the
/// author adding that kind needs: which service, which site, and what to do.
struct Unwired {
    kind: ResourceServiceKind,
    site: &'static str,
    detail: String,
}

fn context() -> GcloudContext {
    GcloudContext {
        configuration: GcloudConfigurationName::new("dev"),
        account: None,
        project: Some("dev-project".to_string()),
        region: None,
        zone: None,
    }
}

/// Every service kind resolves at every per-service site.
///
/// Each check below is falsifiable on its own: removing one service's
/// registry descriptor, its column catalog arm, its jump fields, or its
/// collection wiring fails this test naming that service and that site, and
/// nothing else.
#[test]
fn every_service_kind_is_wired_at_every_per_service_site() {
    let mut unwired = Vec::new();

    for kind in ResourceServiceKind::ALL {
        let mut record = |site: &'static str, detail: String| {
            unwired.push(Unwired { kind, site, detail });
        };

        // --- Registry: selecting the service must reach a list load. These
        // two checks deliberately restate
        // `service_registry_test::every_resource_service_kind_resolves_to_a_registry_service_id`:
        // the value here is that an unwired registry entry appears in the same
        // worklist message as every other unwired site, rather than costing the
        // author adding a service one more gate run to discover.
        match service_id_for_resource(kind) {
            None => record(
                "service registry",
                "no ServiceId, so selecting it would never load anything; add a \
                 ServiceDescriptor to SERVICE_REGISTRY"
                    .to_string(),
            ),
            Some(id) => {
                if load_effect(id, context(), None).is_none() {
                    record(
                        "registry load effect",
                        "its ServiceId emits no list load; add an arm to load_effect".to_string(),
                    );
                }
            }
        }

        // --- Columns: a table with no identity column cannot be read, and a
        // configured list without one is rejected outright by config::columns.
        let meta = catalog_meta(kind);
        if meta.is_empty() {
            record(
                "column catalog",
                "catalog_meta is empty, so the table has no columns at all".to_string(),
            );
        } else if !meta.iter().any(|column| column.key == IDENTITY_COLUMN) {
            record(
                "column catalog",
                format!(
                    "catalog_meta has no {IDENTITY_COLUMN:?} column, so rows cannot be \
                     told apart and every configured list is rejected"
                ),
            );
        }

        // --- Jumps: a service with no fields can be named by no jump rule.
        let fields = fields_for(kind);
        if fields.is_empty() {
            record(
                "jump fields",
                "fields_for is empty, so no jump rule can name one of its resources".to_string(),
            );
        } else if !fields.iter().any(|(name, _)| *name == IDENTITY_COLUMN) {
            record(
                "jump fields",
                format!("fields_for has no {IDENTITY_COLUMN:?} field"),
            );
        }

        // --- Collection isolation: the kind must own a collection of its own.
        // Loading it has to move its own phase off Idle and leave every other
        // service's phase alone; a kind wired to another service's collection
        // moves two phases at once, and a kind with no collection moves none.
        let mut resources = ResourcesState::default();
        let _ticket = resources.begin_load(kind, Instant::now());
        if matches!(resources.load_phase(kind), ResourceLoadPhase::Idle) {
            record(
                "collection isolation",
                "begin_load left its phase Idle, so the kind has no collection of its own \
                 wired into ResourcesState"
                    .to_string(),
            );
        }
        for other in ResourceServiceKind::ALL {
            if other == kind {
                continue;
            }
            if !matches!(resources.load_phase(other), ResourceLoadPhase::Idle) {
                record(
                    "collection isolation",
                    format!(
                        "loading {kind:?} also moved {other:?} off Idle; the two share one \
                         collection, so one of them is wired to the other's state"
                    ),
                );
            }
        }
    }

    // --- Navigator: built by iterating, so a missing kind is a missing line
    // rather than a failure. Projected once, outside the loop, because it
    // projects every service in one call.
    let root = RootState::default();
    let navigator = project_service_navigator(&root, Rect::new(0, 0, 32, 64));

    for kind in ResourceServiceKind::ALL {
        let Some(id) = service_id_for_resource(kind) else {
            // Already recorded above as a registry failure; the navigator
            // check would only restate it.
            continue;
        };
        if !navigator
            .entries
            .iter()
            .any(|entry| entry.id == Some(id) && entry.selectable)
        {
            unwired.push(Unwired {
                kind,
                site: "navigator entry",
                detail: "no selectable row in the service navigator, so it cannot be \
                         reached from the sidebar"
                    .to_string(),
            });
        }
    }

    assert!(
        unwired.is_empty(),
        "{} per-service site(s) do not resolve — this list is the worklist:\n{}",
        unwired.len(),
        unwired
            .iter()
            .map(|item| format!("  {:?} — {}: {}", item.kind, item.site, item.detail))
            .collect::<Vec<_>>()
            .join("\n")
    );
}
