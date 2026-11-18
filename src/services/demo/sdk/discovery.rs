//! Project, region and zone discovery's demo backend: all three listings from
//! the captures.
//!
//! Regions and zones are envelope-only. `gcloud_compute_regions.json` and
//! `gcloud_compute_zones.json` carry exactly the fields `Region` and `Zone`
//! model -- `name`, `status`, and a zone's `region` URL -- and
//! `RegionList`/`ZoneList` put the array in `items`.
//!
//! # Why the project picker needed a second capture
//!
//! `gcloud_projects.json` is Resource Manager v1's vocabulary: `projectId`,
//! `name` for the display name, and `lifecycleState`. v3 renames all three --
//! the display name is `displayName`, `name` is the resource path
//! `projects/<number>`, and the lifecycle is `state` -- so deserializing the v1
//! capture through `SearchProjectsResponse` does not fail. It succeeds, with
//! every renamed key absorbed into `_unknown_fields`, and yields two rows with
//! a blank id, a blank name and no state: a picker of two nameless projects,
//! which is the half-populated shape this backend exists to prevent. A rename
//! is not an envelope, so `gcloud_projects_v3.json` sits beside the v1 file and
//! the CLI parser tests keep reading the original.
//!
//! # What these three stubs reach
//!
//! One method each, all three read through the item paginator, so
//! `next_page_token` is left unset to stop it after one page. `SearchProjects`,
//! `regions::List` and `zones::List` all carry `send`/`by_page`/`by_item` and
//! no `poller()`; the `Projects` client does drive long-running operations --
//! `create_project`, `delete_project`, `undelete_project` all end in a poller
//! that reads back through `get_operation` -- but nothing in
//! `SdkDiscoveryService` reaches a verb that starts one, so `get_operation`
//! stays out of this stub as unreachable rather than as unnoticed.

use google_cloud_compute_v1::client::{Regions, Zones};
use google_cloud_resourcemanager_v3::client::Projects;

use super::envelope;

/// The regions capture: `gcloud compute regions list --format=json`.
const REGIONS: &str = include_str!("../../../../tests/fixtures/gcloud_compute_regions.json");

/// The zones capture: `gcloud compute zones list --format=json`.
const ZONES: &str = include_str!("../../../../tests/fixtures/gcloud_compute_zones.json");

/// The project listing, re-shaped: Resource Manager v3 `Project` rows
/// mirroring `tests/fixtures/gcloud_projects.json`, which is
/// `gcloud projects list --format=json` in Resource Manager v1.
/// Also read by [`super::demo_projects`], which is what every other stub
/// here checks a request's project against.
pub(super) const PROJECTS: &str =
    include_str!("../../../../tests/fixtures/gcloud_projects_v3.json");

/// Answers `search_projects` from the re-shaped project capture.
///
/// `search` rather than `list` because that is the call the picker makes:
/// `list` enumerates one parent's children, and the picker asks what the
/// caller can reach.
#[derive(Debug, Default)]
pub struct ProjectsStub;

impl google_cloud_resourcemanager_v3::stub::Projects for ProjectsStub {
    async fn search_projects(
        &self,
        _request: google_cloud_resourcemanager_v3::model::SearchProjectsRequest,
        _options: google_cloud_gax::options::RequestOptions,
    ) -> google_cloud_gax::Result<
        google_cloud_gax::response::Response<
            google_cloud_resourcemanager_v3::model::SearchProjectsResponse,
        >,
    > {
        Ok(google_cloud_gax::response::Response::from(envelope(
            "projects", PROJECTS,
        )?))
    }
}

/// Answers `list` from the regions capture.
#[derive(Debug, Default)]
pub struct RegionsStub;

impl google_cloud_compute_v1::stub::Regions for RegionsStub {
    async fn list(
        &self,
        _request: google_cloud_compute_v1::model::regions::ListRequest,
        _options: google_cloud_gax::options::RequestOptions,
    ) -> google_cloud_gax::Result<
        google_cloud_gax::response::Response<google_cloud_compute_v1::model::RegionList>,
    > {
        Ok(google_cloud_gax::response::Response::from(envelope(
            "items", REGIONS,
        )?))
    }
}

/// Answers `list` from the zones capture.
#[derive(Debug, Default)]
pub struct ZonesStub;

impl google_cloud_compute_v1::stub::Zones for ZonesStub {
    async fn list(
        &self,
        _request: google_cloud_compute_v1::model::zones::ListRequest,
        _options: google_cloud_gax::options::RequestOptions,
    ) -> google_cloud_gax::Result<
        google_cloud_gax::response::Response<google_cloud_compute_v1::model::ZoneList>,
    > {
        Ok(google_cloud_gax::response::Response::from(envelope(
            "items", ZONES,
        )?))
    }
}

/// The three clients `SdkDiscoveryService::from_fixtures` is built from.
pub fn clients() -> (Projects, Regions, Zones) {
    (
        Projects::from_stub(ProjectsStub),
        Regions::from_stub(RegionsStub),
        Zones::from_stub(ZonesStub),
    )
}
