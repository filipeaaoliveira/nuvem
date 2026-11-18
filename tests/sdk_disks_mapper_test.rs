//! The persistent-disk mapper: the Compute client's `Disk` turned into the
//! `PersistentDisk` row the disks table renders.
//!
//! A mutation sweep reported `map_disk` as untested. Every field is planted
//! with a distinct non-default value and asserted exactly, so a body replaced
//! wholesale — or a closure answering `None`, `Some(0)`, `"xyzzy"` or an empty
//! collection — cannot pass. Both sides of every conditional are covered too:
//! zone against region against neither, a size that converts against one that
//! cannot against none at all, each documented status against an unrecognised
//! one against an absent one, and empty collections against populated ones
//! whose order is pinned.

use google_cloud_compute_v1::model::Disk;
use google_cloud_compute_v1::model::disk::Status as ApiDiskStatus;
use nuvem::services::sdk::disks::map_disk;
use nuvem_domain::gcp::resource::{ComputeKeyValue, DiskStatus, PersistentDisk};

const ZONE_LINK: &str =
    "https://www.googleapis.com/compute/v1/projects/acme-prod/zones/europe-west1-b";
const REGION_LINK: &str =
    "https://www.googleapis.com/compute/v1/projects/acme-prod/regions/europe-west4";
const BALANCED_TYPE_LINK: &str = "https://www.googleapis.com/compute/v1/projects/acme-prod/zones/europe-west1-b/diskTypes/pd-balanced";
const SSD_TYPE_LINK: &str = "https://www.googleapis.com/compute/v1/projects/acme-prod/zones/europe-west1-b/diskTypes/pd-ssd";
const NAME_LINK: &str = "https://www.googleapis.com/compute/v1/projects/acme-prod/zones/europe-west1-b/disks/archive-vol";
const LEDGER_USER_LINK: &str = "https://www.googleapis.com/compute/v1/projects/acme-prod/zones/europe-west1-b/instances/ledger-01";
const REPORTS_USER_LINK: &str = "https://www.googleapis.com/compute/v1/projects/acme-prod/zones/europe-west1-b/instances/reports-02";

/// A disk with every mapped field set to a distinct, non-default value.
///
/// Distinct rather than shared so a mapper that crosses two fields — reading
/// `location` out of `type`, say — shows up as a mismatch instead of passing.
fn fully_populated_disk() -> Disk {
    let mut disk = Disk::new();
    disk.name = Some("ledger-data".to_string());
    disk.zone = Some(ZONE_LINK.to_string());
    disk.status = Some(ApiDiskStatus::Ready);
    disk.size_gb = Some(512);
    disk.r#type = Some(BALANCED_TYPE_LINK.to_string());
    disk.users = vec![LEDGER_USER_LINK.to_string(), REPORTS_USER_LINK.to_string()];
    disk.labels.insert("role".to_string(), "ledger".to_string());
    disk.labels.insert("env".to_string(), "prod".to_string());
    disk.labels
        .insert("cost-center".to_string(), "finance-42".to_string());
    disk
}

fn label(key: &str, value: &str) -> ComputeKeyValue {
    ComputeKeyValue {
        key: key.to_string(),
        value: value.to_string(),
    }
}

#[test]
fn map_disk_carries_every_field_of_a_fully_populated_zonal_disk() {
    let mapped = map_disk(&fully_populated_disk());

    assert_eq!(
        mapped,
        PersistentDisk {
            name: "ledger-data".to_string(),
            location: "europe-west1-b".to_string(),
            status: DiskStatus::Ready,
            size_gb: 512,
            disk_type: "pd-balanced".to_string(),
            users: vec!["ledger-01".to_string(), "reports-02".to_string()],
            labels: vec![
                label("cost-center", "finance-42"),
                label("env", "prod"),
                label("role", "ledger"),
            ],
        }
    );
}

#[test]
fn map_disk_shortens_a_url_valued_name_to_its_trailing_segment() {
    let mut disk = fully_populated_disk();
    disk.name = Some(NAME_LINK.to_string());

    assert_eq!(map_disk(&disk).name, "archive-vol");
}

#[test]
fn map_disk_leaves_an_already_short_name_unchanged() {
    let mut disk = fully_populated_disk();
    disk.name = Some("archive-vol".to_string());

    assert_eq!(map_disk(&disk).name, "archive-vol");
}

#[test]
fn map_disk_reports_an_omitted_name_as_empty() {
    let mut disk = fully_populated_disk();
    disk.name = None;

    assert_eq!(map_disk(&disk).name, "");
}

#[test]
fn map_disk_reads_the_location_of_a_zonal_disk_from_its_zone() {
    let mut disk = fully_populated_disk();
    disk.zone = Some(ZONE_LINK.to_string());
    disk.region = None;

    assert_eq!(map_disk(&disk).location, "europe-west1-b");
}

#[test]
fn map_disk_falls_back_to_the_region_of_a_regional_disk() {
    let mut disk = fully_populated_disk();
    disk.zone = None;
    disk.region = Some(REGION_LINK.to_string());

    assert_eq!(map_disk(&disk).location, "europe-west4");
}

#[test]
fn map_disk_prefers_the_zone_when_both_zone_and_region_are_reported() {
    let mut disk = fully_populated_disk();
    disk.zone = Some(ZONE_LINK.to_string());
    disk.region = Some(REGION_LINK.to_string());

    assert_eq!(map_disk(&disk).location, "europe-west1-b");
}

#[test]
fn map_disk_reports_an_empty_location_when_neither_zone_nor_region_is_reported() {
    let mut disk = fully_populated_disk();
    disk.zone = None;
    disk.region = None;

    assert_eq!(map_disk(&disk).location, "");
}

#[test]
fn map_disk_translates_every_documented_status() {
    for (api, expected) in [
        (ApiDiskStatus::Creating, DiskStatus::Creating),
        (ApiDiskStatus::Restoring, DiskStatus::Restoring),
        (ApiDiskStatus::Failed, DiskStatus::Failed),
        (ApiDiskStatus::Ready, DiskStatus::Ready),
        (ApiDiskStatus::Deleting, DiskStatus::Deleting),
        (ApiDiskStatus::Unavailable, DiskStatus::Unavailable),
    ] {
        let mut disk = fully_populated_disk();
        disk.status = Some(api.clone());

        assert_eq!(
            map_disk(&disk).status,
            expected,
            "status {api:?} mapped to the wrong domain value"
        );
    }
}

#[test]
fn map_disk_keeps_an_unrecognised_status_name_verbatim() {
    let mut disk = fully_populated_disk();
    disk.status = Some(ApiDiskStatus::from("PROVISIONING"));

    let unrecognised = DiskStatus::Unrecognized("PROVISIONING".to_string());
    assert_eq!(map_disk(&disk).status, unrecognised);
}

#[test]
fn map_disk_reports_an_omitted_status_as_an_empty_unrecognised_value() {
    let mut disk = fully_populated_disk();
    disk.status = None;

    assert_eq!(
        map_disk(&disk).status,
        DiskStatus::Unrecognized(String::new())
    );
}

#[test]
fn map_disk_reports_a_nameless_numeric_status_as_an_empty_unrecognised_value() {
    // An enum deserialized from an integer the client does not know has no
    // name at all — a different absence from a missing field, and one that has
    // to land in the same conservative bucket rather than guess at `Ready`.
    let mut disk = fully_populated_disk();
    disk.status = Some(ApiDiskStatus::from(9001));

    assert_eq!(
        map_disk(&disk).status,
        DiskStatus::Unrecognized(String::new())
    );
}

#[test]
fn map_disk_carries_the_reported_size_in_gigabytes() {
    let mut disk = fully_populated_disk();
    disk.size_gb = Some(375);

    assert_eq!(map_disk(&disk).size_gb, 375);
}

#[test]
fn map_disk_reports_an_omitted_size_as_zero() {
    let mut disk = fully_populated_disk();
    disk.size_gb = None;

    assert_eq!(map_disk(&disk).size_gb, 0);
}

#[test]
fn map_disk_reports_a_negative_size_as_zero_rather_than_wrapping_it() {
    // `size_gb` is an i64 on the wire and a u64 on the row; a nonsense
    // negative must not render as 18 exabytes on a cost-cleanup screen.
    let mut disk = fully_populated_disk();
    disk.size_gb = Some(-1);

    assert_eq!(map_disk(&disk).size_gb, 0);
}

#[test]
fn map_disk_shortens_the_disk_type_url_to_its_trailing_segment() {
    let mut disk = fully_populated_disk();
    disk.r#type = Some(SSD_TYPE_LINK.to_string());

    assert_eq!(map_disk(&disk).disk_type, "pd-ssd");
}

#[test]
fn map_disk_reports_an_omitted_disk_type_as_empty() {
    let mut disk = fully_populated_disk();
    disk.r#type = None;

    assert_eq!(map_disk(&disk).disk_type, "");
}

#[test]
fn map_disk_shortens_every_user_link_and_keeps_the_reported_order() {
    let mut disk = fully_populated_disk();
    disk.users = vec![REPORTS_USER_LINK.to_string(), LEDGER_USER_LINK.to_string()];

    assert_eq!(
        map_disk(&disk).users,
        vec!["reports-02".to_string(), "ledger-01".to_string()]
    );
}

#[test]
fn map_disk_reports_a_disk_with_no_users_as_attached_to_nothing() {
    let mut disk = fully_populated_disk();
    disk.users = Vec::new();

    assert!(map_disk(&disk).users.is_empty());
}

#[test]
fn map_disk_sorts_labels_by_key_whatever_order_the_api_reported_them_in() {
    let mut disk = fully_populated_disk();
    disk.labels.clear();
    disk.labels
        .insert("zone-tier".to_string(), "hot".to_string());
    disk.labels.insert("app".to_string(), "ledger".to_string());
    disk.labels
        .insert("owner".to_string(), "payments".to_string());

    assert_eq!(
        map_disk(&disk).labels,
        vec![
            label("app", "ledger"),
            label("owner", "payments"),
            label("zone-tier", "hot")
        ]
    );
}

#[test]
fn map_disk_reports_an_unlabelled_disk_as_having_no_labels() {
    let mut disk = fully_populated_disk();
    disk.labels.clear();

    assert!(map_disk(&disk).labels.is_empty());
}

#[test]
fn a_mapped_ready_disk_with_no_users_is_an_orphan() {
    let mut disk = fully_populated_disk();
    disk.status = Some(ApiDiskStatus::Ready);
    disk.users = Vec::new();

    assert!(map_disk(&disk).is_orphan());
}

#[test]
fn a_mapped_ready_disk_with_users_is_not_an_orphan() {
    let mut disk = fully_populated_disk();
    disk.status = Some(ApiDiskStatus::Ready);
    disk.users = vec![LEDGER_USER_LINK.to_string()];

    assert!(!map_disk(&disk).is_orphan());
}

#[test]
fn a_mapped_unattached_disk_that_is_not_ready_is_not_an_orphan() {
    // Mid-operation is not waste: badging a `CREATING` disk tells the operator
    // to delete something that is still being built.
    for status in [
        ApiDiskStatus::Creating,
        ApiDiskStatus::Restoring,
        ApiDiskStatus::Failed,
        ApiDiskStatus::Deleting,
        ApiDiskStatus::Unavailable,
    ] {
        let mut disk = fully_populated_disk();
        disk.status = Some(status.clone());
        disk.users = Vec::new();

        assert!(
            !map_disk(&disk).is_orphan(),
            "an unattached {status:?} disk was badged as an orphan"
        );
    }
}
