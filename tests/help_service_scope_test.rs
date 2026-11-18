//! The help overlay's "Current view" section names only keys the current view
//! answers.
//!
//! The section used to be one flat list for every resource service, so a GCE
//! instances table taught `i Bucket metadata`, `y Copy gs:// URI` and
//! `n Next page` — the Cloud Storage object browser's keys, which do nothing on
//! a VM table — and Cloud Storage taught the four marking keys, which the
//! reducer answers only for Compute Engine and Cloud SQL.
//!
//! The catalogue's own stated principle already said this: the drill-down arm
//! drops `s` and `←`/`→` because "teaching a key that dispatches nothing is
//! what this catalogue is not for". The only difference was that the drill-down
//! case was expressible in `HelpContext` and the service case was not.

use nuvem::app::projection::help_model;
use nuvem::app::state::root::RootState;
use nuvem::app::{AppState, Workspace};
use nuvem_domain::gcp::resource::ResourceServiceKind;

fn root_on(kind: ResourceServiceKind) -> RootState {
    let mut app = AppState::new();
    app.root.workspace = Workspace::Resources;
    app.root.resources.select_service(kind);
    app.root
}

fn current_view_labels(kind: ResourceServiceKind) -> Vec<&'static str> {
    let root = root_on(kind);
    let model = help_model(&root);
    let section = model
        .sections
        .iter()
        .find(|section| section.title == "Current view")
        .expect("the help model always has a current-view section");
    section
        .bindings
        .iter()
        .map(|binding| binding.label)
        .collect()
}

/// The two concrete cases from the ticket.
#[test]
fn a_vm_table_does_not_teach_the_object_browsers_keys() {
    let labels = current_view_labels(ResourceServiceKind::ComputeInstances);
    for storage_only in ["Bucket metadata", "Copy gs:// URI", "Next page"] {
        assert!(
            !labels.contains(&storage_only),
            "GCE instances teaches {storage_only:?}, which it does not answer; \
             taught: {labels:?}"
        );
    }
}

#[test]
fn the_object_browser_does_not_teach_the_marking_keys() {
    let labels = current_view_labels(ResourceServiceKind::CloudStorage);
    for mark_only in ["Mark", "Range mark", "Clear marks", "Unmark visible"] {
        assert!(
            !labels.contains(&mark_only),
            "Cloud Storage teaches {mark_only:?}, which it does not answer; \
             taught: {labels:?}"
        );
    }
}

/// The other direction, so the filter cannot pass by teaching nothing: a
/// service that *does* answer a key must still be told about it.
#[test]
fn a_service_still_learns_the_keys_it_answers() {
    let compute = current_view_labels(ResourceServiceKind::ComputeInstances);
    for expected in ["Mark", "Range mark", "Start tunnel"] {
        assert!(
            compute.contains(&expected),
            "GCE instances answers {expected:?} and must be taught it; \
             taught: {compute:?}"
        );
    }

    let storage = current_view_labels(ResourceServiceKind::CloudStorage);
    for expected in ["Bucket metadata", "Copy gs:// URI", "Next page"] {
        assert!(
            storage.contains(&expected),
            "Cloud Storage answers {expected:?} and must be taught it; \
             taught: {storage:?}"
        );
    }
}

/// Every service is taught the keys every service answers, so the filter can
/// only ever remove a service-scoped binding.
#[test]
fn every_service_keeps_the_bindings_no_service_scopes() {
    for kind in ResourceServiceKind::ALL {
        let labels = current_view_labels(kind);
        for universal in ["Filter", "Sort column", "Wide columns", "Move selection"] {
            assert!(
                labels.contains(&universal),
                "{kind:?} lost {universal:?}, which every resource list answers; \
                 taught: {labels:?}"
            );
        }
    }
}

/// The taught set and the capability agree, in both directions, for every
/// service.
///
/// This pins the *filter*, not the predicates: both sides read the same
/// `service_capabilities` function, so it fails when `answered_by` stops
/// consulting one — and passes happily if a predicate itself is wrong. What the
/// predicates say is pinned against the reducer, independently, in
/// `service_capability_parity_test`.
#[test]
fn every_service_is_taught_a_scoped_key_exactly_when_it_answers_it() {
    use nuvem::app::service_capabilities::{browses_objects, supports_marks, supports_tunnel};

    for kind in ResourceServiceKind::ALL {
        let labels = current_view_labels(kind);
        let taught = |label: &str| labels.contains(&label);

        for label in ["Mark", "Range mark", "Clear marks", "Unmark visible"] {
            assert_eq!(
                taught(label),
                supports_marks(kind),
                "{kind:?}: taught {label:?} = {}, supports_marks = {}",
                taught(label),
                supports_marks(kind)
            );
        }
        assert_eq!(
            taught("Start tunnel"),
            supports_tunnel(kind),
            "{kind:?}: taught \"Start tunnel\" = {}, supports_tunnel = {}",
            taught("Start tunnel"),
            supports_tunnel(kind)
        );
        for label in ["Bucket metadata", "Copy gs:// URI", "Next page"] {
            assert_eq!(
                taught(label),
                browses_objects(kind),
                "{kind:?}: taught {label:?} = {}, browses_objects = {}",
                taught(label),
                browses_objects(kind)
            );
        }
    }
}
