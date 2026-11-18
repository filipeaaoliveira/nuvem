//! Persistent disks state: what the table sorts by, what `/` searches, and
//! what a reload does to the cursor.
//!
//! The orphan badge is the thread through all three. It is derived rather than
//! fetched, so it exists only in this layer — a filter that could not see it,
//! or a sort that scattered the badged rows, would leave the operator no way to
//! act on the one column the service exists for.
//!
//! Everything here drives the public API an operator's keypresses reach, so no
//! assertion can pass against a state the UI could not produce.

use nuvem::app::state::resources::{DiskSortField, ORPHAN_BADGE, ResourcesState};
use nuvem_domain::gcp::configuration::{GcloudConfigurationName, GcloudContext};
use nuvem_domain::gcp::diagnostics::{DiagnosticProblem, GcloudErrorKind};
use nuvem_domain::gcp::resource::{DiskStatus, PersistentDisk, ResourceServiceKind};

const DISKS: ResourceServiceKind = ResourceServiceKind::PersistentDisks;

/// Catalog order: name, location, status, orphan, size, type, attached.
const SIZE_COLUMN: usize = 4;
const DISK_COLUMN_COUNT: usize = 7;

fn ctx() -> GcloudContext {
    GcloudContext {
        configuration: GcloudConfigurationName::new("dev"),
        account: None,
        project: Some("dev-project".into()),
        region: None,
        zone: None,
    }
}

fn disk(name: &str, size_gb: u64, status: DiskStatus, users: &[&str]) -> PersistentDisk {
    PersistentDisk {
        name: name.to_string(),
        location: "europe-west1-b".to_string(),
        status,
        size_gb,
        disk_type: "pd-ssd".to_string(),
        users: users.iter().map(|user| user.to_string()).collect(),
        labels: Vec::new(),
    }
}

fn loaded(rows: Vec<PersistentDisk>) -> ResourcesState {
    let mut state = ResourcesState::default();
    state.select_service(DISKS);
    state.set_persistent_disks(ctx(), rows);
    state
}

/// A mixed row set: one orphan, one attached, one transient, one big.
fn mixed() -> ResourcesState {
    loaded(vec![
        disk("attached", 200, DiskStatus::Ready, &["web-01"]),
        disk("detached-small", 9, DiskStatus::Ready, &[]),
        disk("being-built", 50, DiskStatus::Creating, &[]),
        disk("detached-big", 100, DiskStatus::Ready, &[]),
    ])
}

/// Moves the column cursor onto catalog position `index` and sorts by it,
/// through the same keys an operator would press. Nothing here reaches past the
/// public API to install a sort field the UI could not select.
///
/// Presses until the cursor *lands* on the column rather than pressing a fixed
/// number of times: `select_next_column` skips unsortable columns, and `orphan`
/// is one, so a count would silently land one column past the target and sort
/// by something else while still passing a weaker assertion.
fn sort_by_column(state: &mut ResourcesState, index: usize) {
    for _ in 0..DISK_COLUMN_COUNT {
        if state.selected_column() == index {
            break;
        }
        state.select_next_column();
    }
    assert_eq!(
        state.selected_column(),
        index,
        "the column cursor never reached catalog position {index}"
    );
    state.sort_selected_column();
}

fn apply_filter(state: &mut ResourcesState, pattern: &str) {
    state.start_filter();
    for ch in pattern.chars() {
        state.append_filter_char(ch);
    }
    state.apply_filter();
}

fn visible_names(state: &ResourcesState) -> Vec<String> {
    state
        .visible_persistent_disks()
        .into_iter()
        .map(|disk| disk.name.clone())
        .collect()
}

/// Sorting by size orders on the integer, not on the rendered cell.
///
/// As text `"100 GB"` sorts before `"9 GB"`, which is the exact opposite of
/// what an operator scanning for the biggest waste is asking the column for.
#[test]
fn size_sorts_numerically_rather_than_lexically() {
    let mut state = mixed();
    sort_by_column(&mut state, SIZE_COLUMN);

    let sizes: Vec<u64> = state
        .visible_persistent_disks()
        .into_iter()
        .map(|disk| disk.size_gb)
        .collect();

    assert_eq!(
        sizes,
        vec![9, 50, 100, 200],
        "a lexical sort on the rendered cell would put 100 and 200 ahead of 9 and 50"
    );
}

/// `/ORPHAN` isolates exactly the badged rows.
///
/// The badge exists in no gcloud field — it is derived from `users` and
/// `status` in this layer — so unless it is added to the searchable set an
/// operator can read `ORPHAN` in a column and be unable to filter by it.
#[test]
fn filtering_by_the_orphan_badge_keeps_exactly_the_badged_rows() {
    let mut state = mixed();
    apply_filter(&mut state, ORPHAN_BADGE);

    assert_eq!(
        visible_names(&state),
        vec!["detached-big".to_string(), "detached-small".to_string()],
        "the attached disk and the CREATING disk must not survive an ORPHAN filter"
    );
    assert_eq!(state.visible_persistent_disk_count(), 2);
}

/// The status label is searchable as it is painted (`Ready`, not `READY`).
///
/// An operator filters by what they can read in the cell; matching the raw API
/// value instead would make the column and the filter disagree.
#[test]
fn the_status_label_is_searchable_as_painted() {
    let mut state = mixed();
    apply_filter(&mut state, "Creating");

    assert_eq!(visible_names(&state), vec!["being-built".to_string()]);
}

/// The attached-instance names are searchable, so an operator can find the
/// disks belonging to one instance without reading every row.
#[test]
fn the_attached_instance_names_are_searchable() {
    let mut state = mixed();
    apply_filter(&mut state, "web-01");

    assert_eq!(visible_names(&state), vec!["attached".to_string()]);
}

/// A reload keeps the cursor on the disk it was already on.
///
/// Identity is (name, location), not the row index: a new row sorting ahead of
/// the selection would otherwise move the cursor to a different disk.
#[test]
fn a_reload_keeps_the_cursor_on_the_same_disk() {
    let mut state = mixed();
    // Default sort is by name: attached, being-built, detached-big, detached-small.
    // Deliberately none of these names contains "ORPHAN": the badge must be the
    // only thing an ORPHAN filter can match, or the filter test passes on the
    // row names and says nothing about the badge at all.
    state.select_next();
    state.select_next();
    assert_eq!(
        state
            .selected_persistent_disk()
            .expect("a row is selected")
            .name,
        "detached-big"
    );

    // The same disk, reported with a different size, plus a new row ahead of it
    // in the ordering — so an index-preserving implementation lands elsewhere.
    state.set_persistent_disks(
        ctx(),
        vec![
            disk("aaa-new", 10, DiskStatus::Ready, &[]),
            disk("attached", 200, DiskStatus::Ready, &["web-01"]),
            disk("being-built", 50, DiskStatus::Creating, &[]),
            disk("detached-big", 120, DiskStatus::Ready, &[]),
        ],
    );

    assert_eq!(
        state
            .selected_persistent_disk()
            .expect("a row is still selected")
            .name,
        "detached-big",
        "the cursor followed the index instead of the disk"
    );
}

/// The sort field is the table's own vocabulary: its label is the word the
/// status line shows after `Sort:`, and `from_column`/`column` are how a
/// column position and a sort field find each other. A wrong answer here sorts
/// by one column while the header arrow sits on another.
///
/// `orphan` is the one catalogue column with no sort field, so it is pinned as
/// a hole rather than left to whatever the wildcard arm happens to return.
#[test]
fn every_sortable_column_maps_to_its_field_and_back() {
    for (index, field, label) in [
        (0, DiskSortField::Name, "Name"),
        (1, DiskSortField::Location, "Location"),
        (2, DiskSortField::Status, "Status"),
        (4, DiskSortField::Size, "Size"),
        (5, DiskSortField::Type, "Type"),
        (6, DiskSortField::Attached, "Attached"),
    ] {
        assert_eq!(DiskSortField::from_column(index), Some(field));
        assert_eq!(field.column(), index);
        assert_eq!(field.label(), label);
    }
    assert_eq!(
        DiskSortField::from_column(3),
        None,
        "orphan is derived, not a column gcloud can order by"
    );
    assert_eq!(
        DiskSortField::from_column(DiskSortField::COLUMN_COUNT),
        None,
        "a column past the catalogue is not sortable"
    );
    assert_eq!(DiskSortField::COLUMN_COUNT, DISK_COLUMN_COUNT);

    // And the label is what the list actually reports, not a string only this
    // test reads.
    let mut state = mixed();
    assert_eq!(state.sort_label(), "Name");
    sort_by_column(&mut state, 1);
    assert_eq!(state.sort_label(), "Location");
    sort_by_column(&mut state, 6);
    assert_eq!(state.sort_label(), "Attached");
}

/// A recorded diagnostic problem is readable until the next successful load.
///
/// The only way to observe `set_service_problem`'s persistent-disks arm, and
/// the reason the arm exists: an empty list with a permission failure behind
/// it must be able to say which failure, rather than reading as a project with
/// no disks.
#[test]
fn a_recorded_problem_is_readable_until_the_next_successful_load() {
    let mut state = loaded(Vec::new());
    state.set_service_problem(
        DISKS,
        DiagnosticProblem::new(GcloudErrorKind::PermissionDenied, "denied"),
    );

    let problem = state
        .persistent_disks_problem()
        .expect("the recorded problem must be readable");
    assert_eq!(problem.kind, GcloudErrorKind::PermissionDenied);
    assert_eq!(problem.message, "denied");

    state.set_persistent_disks(ctx(), vec![disk("d", 10, DiskStatus::Ready, &[])]);

    assert!(
        state.persistent_disks_problem().is_none(),
        "a successful load must clear the previous problem"
    );
}
