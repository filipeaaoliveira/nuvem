//! The discovery mappers -- the client's `Project`, `Region` and `Zone` turned
//! into the three rows that fill the context pickers.
//!
//! A mutation sweep reported `map_project`, `map_region` and `map_zone` as
//! untested: a body replaced wholesale with `Default::default()`, an
//! `unwrap_or_default` fallback deleted, or a `map`/`and_then` closure answering
//! a constant, and nothing went red. Every field asserted here is therefore
//! planted with a distinct, non-default value -- the v3 resource path in `name`
//! differs from the display name, the zone's region URL differs from its tail --
//! and both sides of every conditional are exercised with outputs that differ:
//! a name present against absent, a status the client can number against one it
//! cannot, a region URL against a bare segment against nothing at all.

use google_cloud_compute_v1::model::region::Status as RegionStatus;
use google_cloud_compute_v1::model::zone::Status as ZoneStatus;
use google_cloud_compute_v1::model::{Region, Zone};
use google_cloud_resourcemanager_v3::model::Project;
use google_cloud_resourcemanager_v3::model::project::State as ProjectState;
use nuvem::services::sdk::discovery::{map_project, map_region, map_zone};
use nuvem_domain::gcp::location::{ComputeRegion, ComputeZone};
use nuvem_domain::gcp::project::GcloudProject;

/// What `map_region`/`map_zone` write into `status`: the API's own word.
///
/// These are spelled out rather than recomputed from `Status::name()`, so a
/// mapper that started answering the enum's *number* -- `"1"` for `UP`, which
/// is what both mappers did until the read-path measurement caught it -- fails
/// here instead of agreeing with itself. The gcloud path for the same listing
/// (`parse_regions_json`) passes the API's string through untouched, the picker
/// paints one cell for both backends, and its incremental filter matches on
/// what is painted: typing `up` must narrow to the regions that are up.
const STATUS_UP_AS_MAPPED: &str = "UP";
const STATUS_DOWN_AS_MAPPED: &str = "DOWN";

/// The v3 `name` is the resource path, never the human name -- planted
/// differently from `display_name` so a mapper reading the wrong one fails.
const PROJECT_RESOURCE_PATH: &str = "projects/415104041262";
const PROJECT_ID: &str = "acme-payments-prod";
const PROJECT_DISPLAY_NAME: &str = "Payments Production";

const REGION_URL: &str =
    "https://www.googleapis.com/compute/v1/projects/acme-payments-prod/regions/europe-west4";

fn active_project() -> Project {
    Project::new()
        .set_name(PROJECT_RESOURCE_PATH)
        .set_project_id(PROJECT_ID)
        .set_display_name(PROJECT_DISPLAY_NAME)
        .set_state(ProjectState::Active)
}

#[test]
fn map_project_takes_the_row_name_from_display_name_and_the_state_from_state() {
    let project = active_project()
        .set_parent("organizations/778899")
        .set_etag("W/\"planted-etag\"");

    assert_eq!(
        map_project(&project),
        GcloudProject {
            project_id: PROJECT_ID.to_string(),
            name: PROJECT_DISPLAY_NAME.to_string(),
            lifecycle_state: Some("ACTIVE".to_string()),
        }
    );
}

#[test]
fn map_project_never_reports_the_v3_resource_path_as_the_row_name() {
    let row = map_project(&active_project());

    assert_ne!(row.name, PROJECT_RESOURCE_PATH);
    assert_eq!(row.name, PROJECT_DISPLAY_NAME);
}

#[test]
fn map_project_reports_a_project_inside_its_deletion_window() {
    let project = active_project().set_state(ProjectState::DeleteRequested);

    assert_eq!(
        map_project(&project),
        GcloudProject {
            project_id: PROJECT_ID.to_string(),
            name: PROJECT_DISPLAY_NAME.to_string(),
            lifecycle_state: Some("DELETE_REQUESTED".to_string()),
        }
    );
}

#[test]
fn map_project_keeps_the_unspecified_state_under_its_api_name_rather_than_dropping_it() {
    let project = active_project().set_state(ProjectState::Unspecified);

    assert_eq!(
        map_project(&project).lifecycle_state,
        Some("STATE_UNSPECIFIED".to_string())
    );
}

#[test]
fn map_project_keeps_a_state_name_the_client_does_not_know() {
    let project = active_project().set_state(ProjectState::from("SUSPENDED"));

    assert_eq!(
        map_project(&project).lifecycle_state,
        Some("SUSPENDED".to_string())
    );
}

#[test]
fn map_project_has_no_lifecycle_state_when_the_client_cannot_name_the_state() {
    // A state that arrived as a bare number the generated enum has no name for:
    // `State::name()` answers `None`, and the row must carry that through
    // rather than inventing a label.
    let project = active_project().set_state(ProjectState::from(97_i32));

    assert_eq!(map_project(&project).lifecycle_state, None);
}

#[test]
fn map_project_leaves_an_empty_display_name_empty_rather_than_falling_back_to_the_id() {
    let project = Project::new()
        .set_name(PROJECT_RESOURCE_PATH)
        .set_project_id(PROJECT_ID)
        .set_state(ProjectState::Active);

    assert_eq!(
        map_project(&project),
        GcloudProject {
            project_id: PROJECT_ID.to_string(),
            name: String::new(),
            lifecycle_state: Some("ACTIVE".to_string()),
        }
    );
}

#[test]
fn map_region_carries_the_name_and_status_and_ignores_the_neighbouring_fields() {
    let mut region = Region::new();
    region.name = Some("europe-west4".into());
    region.status = Some(RegionStatus::Up);
    region.description = Some("planted description".into());
    region.self_link = Some(REGION_URL.into());
    region.id = Some(7_712_233_445_566);
    region.zones = vec![format!("{REGION_URL}-a"), format!("{REGION_URL}-b")];

    assert_eq!(
        map_region(&region),
        ComputeRegion {
            name: "europe-west4".to_string(),
            status: Some(STATUS_UP_AS_MAPPED.to_string()),
        }
    );
}

#[test]
fn map_region_tells_a_down_region_from_an_up_one() {
    let mut up = Region::new();
    up.name = Some("europe-west4".into());
    up.status = Some(RegionStatus::Up);

    let mut down = Region::new();
    down.name = Some("us-east4".into());
    down.status = Some(RegionStatus::Down);

    assert_eq!(
        map_region(&up).status,
        Some(STATUS_UP_AS_MAPPED.to_string())
    );
    assert_eq!(
        map_region(&down).status,
        Some(STATUS_DOWN_AS_MAPPED.to_string())
    );
    assert_ne!(map_region(&up).status, map_region(&down).status);
}

#[test]
fn map_region_has_no_status_when_the_listing_carried_none() {
    let mut region = Region::new();
    region.name = Some("southamerica-east1".into());
    region.status = None;

    assert_eq!(
        map_region(&region),
        ComputeRegion {
            name: "southamerica-east1".to_string(),
            status: None,
        }
    );
}

#[test]
fn map_region_carries_a_status_word_the_client_does_not_recognise() {
    // A status the generated enum has no variant for keeps its name, so a word
    // this build predates still reaches the cell rather than blanking it.
    let mut region = Region::new();
    region.name = Some("europe-west4".into());
    region.status = Some(RegionStatus::from("STANDBY"));

    assert_eq!(map_region(&region).status, Some("STANDBY".to_string()));
}

#[test]
fn map_region_drops_a_status_number_that_names_nothing() {
    // A bare number has no name to print, and `"42"` in the status column would
    // be worse than an empty one: it reads as a value rather than as an unknown.
    let mut region = Region::new();
    region.name = Some("europe-west4".into());
    region.status = Some(RegionStatus::from(42_i32));

    assert_eq!(map_region(&region).status, None);
}

#[test]
fn map_region_falls_back_to_an_empty_name_when_the_listing_omitted_one() {
    let mut region = Region::new();
    region.name = None;
    region.status = Some(RegionStatus::Down);
    region.self_link = Some(REGION_URL.into());

    assert_eq!(
        map_region(&region),
        ComputeRegion {
            name: String::new(),
            status: Some(STATUS_DOWN_AS_MAPPED.to_string()),
        }
    );
}

#[test]
fn map_zone_reads_the_region_from_the_tail_of_the_url_the_api_sent() {
    let mut zone = Zone::new();
    zone.name = Some("europe-west4-b".into());
    zone.region = Some(REGION_URL.into());
    zone.status = Some(ZoneStatus::Up);
    zone.description = Some("planted description".into());
    zone.self_link = Some(format!("{REGION_URL}-b/zones/europe-west4-b"));
    zone.available_cpu_platforms = vec!["Intel Cascade Lake".into(), "AMD Rome".into()];

    assert_eq!(
        map_zone(&zone),
        ComputeZone {
            name: "europe-west4-b".to_string(),
            region: Some("europe-west4".to_string()),
            status: Some(STATUS_UP_AS_MAPPED.to_string()),
        }
    );
}

#[test]
fn map_zone_reads_the_region_the_api_stated_rather_than_parsing_the_zone_name() {
    // The zone name's own prefix (`us-central1`) is planted to disagree with
    // the region the listing stated (`europe-west4`), so a mapper that split
    // the name on its last dash -- the shortcut `ComputeZone`'s own doc warns
    // against -- answers differently from one that reads `zone.region`.
    let mut zone = Zone::new();
    zone.name = Some("us-central1-a".into());
    zone.region = Some(REGION_URL.into());
    zone.status = Some(ZoneStatus::Up);

    assert_eq!(
        map_zone(&zone),
        ComputeZone {
            name: "us-central1-a".to_string(),
            region: Some("europe-west4".to_string()),
            status: Some(STATUS_UP_AS_MAPPED.to_string()),
        }
    );
}

#[test]
fn map_zone_keeps_a_region_that_arrived_without_any_url_prefix() {
    let mut zone = Zone::new();
    zone.name = Some("us-central1-a".into());
    zone.region = Some("us-central1".into());
    zone.status = Some(ZoneStatus::Up);

    assert_eq!(
        map_zone(&zone),
        ComputeZone {
            name: "us-central1-a".to_string(),
            region: Some("us-central1".to_string()),
            status: Some(STATUS_UP_AS_MAPPED.to_string()),
        }
    );
}

#[test]
fn map_zone_has_no_region_when_the_listing_carried_none() {
    let mut zone = Zone::new();
    zone.name = Some("us-central1-a".into());
    zone.region = None;
    zone.status = Some(ZoneStatus::Up);

    assert_eq!(
        map_zone(&zone),
        ComputeZone {
            name: "us-central1-a".to_string(),
            region: None,
            status: Some(STATUS_UP_AS_MAPPED.to_string()),
        }
    );
}

#[test]
fn map_zone_tells_a_present_but_empty_region_from_an_absent_one() {
    // An empty string is still a region the API stated: the tail of "" is "",
    // and the row says `Some("")` rather than collapsing to `None`.
    let mut zone = Zone::new();
    zone.name = Some("us-central1-a".into());
    zone.region = Some(String::new());
    zone.status = Some(ZoneStatus::Down);

    assert_eq!(map_zone(&zone).region, Some(String::new()));

    let mut absent = zone.clone();
    absent.region = None;

    assert_ne!(map_zone(&zone).region, map_zone(&absent).region);
}

#[test]
fn map_zone_tells_a_down_zone_from_an_up_one() {
    let mut up = Zone::new();
    up.name = Some("europe-west4-b".into());
    up.region = Some(REGION_URL.into());
    up.status = Some(ZoneStatus::Up);

    let mut down = Zone::new();
    down.name = Some("europe-west4-c".into());
    down.region = Some(REGION_URL.into());
    down.status = Some(ZoneStatus::Down);

    assert_eq!(map_zone(&up).status, Some(STATUS_UP_AS_MAPPED.to_string()));
    assert_eq!(
        map_zone(&down).status,
        Some(STATUS_DOWN_AS_MAPPED.to_string())
    );
    assert_ne!(map_zone(&up).status, map_zone(&down).status);
}

#[test]
fn map_zone_has_no_status_when_the_listing_carried_none() {
    let mut zone = Zone::new();
    zone.name = Some("europe-west4-b".into());
    zone.region = Some(REGION_URL.into());
    zone.status = None;

    assert_eq!(
        map_zone(&zone),
        ComputeZone {
            name: "europe-west4-b".to_string(),
            region: Some("europe-west4".to_string()),
            status: None,
        }
    );
}

#[test]
fn map_zone_carries_a_status_word_the_client_does_not_recognise() {
    let mut zone = Zone::new();
    zone.name = Some("europe-west4-b".into());
    zone.region = Some(REGION_URL.into());
    zone.status = Some(ZoneStatus::from("STANDBY"));

    assert_eq!(map_zone(&zone).status, Some("STANDBY".to_string()));
}

#[test]
fn map_zone_drops_a_status_number_that_names_nothing() {
    // The integer arm of the unknown-value split, mirrored from the region
    // case: a number the enum cannot name is dropped rather than printed, which
    // is the side a body answering the value would not produce.
    let mut zone = Zone::new();
    zone.name = Some("europe-west4-b".into());
    zone.region = Some(REGION_URL.into());
    zone.status = Some(ZoneStatus::from(42_i32));

    assert_eq!(map_zone(&zone).status, None);
}

#[test]
fn map_zone_falls_back_to_an_empty_name_when_the_listing_omitted_one() {
    let mut zone = Zone::new();
    zone.name = None;
    zone.region = Some(REGION_URL.into());
    zone.status = Some(ZoneStatus::Down);

    assert_eq!(
        map_zone(&zone),
        ComputeZone {
            name: String::new(),
            region: Some("europe-west4".to_string()),
            status: Some(STATUS_DOWN_AS_MAPPED.to_string()),
        }
    );
}
