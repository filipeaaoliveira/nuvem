//! Projects, regions and zones through the native clients.
//!
//! These three fill the context pickers. They are listings by the test the
//! decision record sets out -- flat tables of fields the app models, with no
//! JSON tab behind them -- and they were the last CLI reads left.
//!
//! `tail_segment` is the discovery module's own, because a zone's region
//! arrives as a URL on both paths and the row must read the same either way.

use google_cloud_auth::credentials::Credentials;
use google_cloud_compute_v1::client::{Regions, Zones};
use google_cloud_resourcemanager_v3::client::Projects;
use nuvem_domain::gcp::location::{ComputeRegion, ComputeZone};
use nuvem_domain::gcp::project::GcloudProject;

use crate::services::gcloud::discovery::tail_segment;
use crate::services::sdk::ApiError;

pub struct SdkDiscoveryService {
    projects: Projects,
    regions: Regions,
    zones: Zones,
}

impl SdkDiscoveryService {
    pub async fn connect(credentials: Credentials) -> Result<Self, ApiError> {
        let projects = Projects::builder()
            .with_credentials(credentials.clone())
            .build()
            .await
            .map_err(|error| ApiError::new(error.to_string()))?;
        let regions = Regions::builder()
            .with_credentials(credentials.clone())
            .build()
            .await
            .map_err(|error| ApiError::new(error.to_string()))?;
        let zones = Zones::builder()
            .with_credentials(credentials)
            .build()
            .await
            .map_err(|error| ApiError::new(error.to_string()))?;
        Ok(Self {
            projects,
            regions,
            zones,
        })
    }

    /// The same service, built from already-constructed clients. See
    /// `SdkGkeService::from_clusters_for_test`.
    pub fn from_clients_for_test(projects: Projects, regions: Regions, zones: Zones) -> Self {
        Self {
            projects,
            regions,
            zones,
        }
    }

    /// The same service, answering from the demo fixtures.
    ///
    /// Required by `sdk_client_cache!`: the macro's `Fixtures` arm calls this
    /// on every service in its list, so a fifteenth service cannot join that
    /// list without one. It takes no credentials, cannot fail and cannot
    /// block -- a client built by `from_stub` holds a stub where a
    /// transport would be.
    pub fn from_fixtures() -> Self {
        let (projects, regions, zones) = crate::services::demo::sdk::discovery::clients();
        Self {
            projects,
            regions,
            zones,
        }
    }

    /// The same service over the demo showcase. It has no showcase documents
    /// of its own, so it answers from the captures, as
    /// [`Self::from_fixtures`] does. See [`crate::services::demo::Corpus`].
    pub fn from_showcase() -> Self {
        Self::from_fixtures()
    }

    /// Every project the caller can see. `search` rather than `list` because
    /// `list` enumerates one parent's children, while `gcloud projects list`
    /// answers "what can I reach" -- which is the question the picker asks.
    pub async fn load_projects(&self) -> Result<Vec<GcloudProject>, ApiError> {
        use google_cloud_gax::paginator::ItemPaginator;
        let mut rows = Vec::new();
        let mut items = self.projects.search_projects().by_item();
        while let Some(project) = items.next().await {
            rows.push(map_project(&project?));
        }
        Ok(rows)
    }

    pub async fn load_regions(&self, project: &str) -> Result<Vec<ComputeRegion>, ApiError> {
        use google_cloud_gax::paginator::ItemPaginator;
        let mut rows = Vec::new();
        let mut items = self.regions.list().set_project(project).by_item();
        while let Some(region) = items.next().await {
            rows.push(map_region(&region?));
        }
        Ok(rows)
    }

    pub async fn load_zones(&self, project: &str) -> Result<Vec<ComputeZone>, ApiError> {
        use google_cloud_gax::paginator::ItemPaginator;
        let mut rows = Vec::new();
        let mut items = self.zones.list().set_project(project).by_item();
        while let Some(zone) = items.next().await {
            rows.push(map_zone(&zone?));
        }
        Ok(rows)
    }
}

/// The CLI reads `name` from the project's *display* name and
/// `lifecycleState` from its state, so both are taken from there rather than
/// from `name`, which in v3 is the resource path `projects/<number>`.
pub fn map_project(project: &google_cloud_resourcemanager_v3::model::Project) -> GcloudProject {
    GcloudProject {
        project_id: project.project_id.clone(),
        name: project.display_name.clone(),
        lifecycle_state: project.state.name().map(|state| state.to_string()),
    }
}

/// The status is taken by name, not by number.
///
/// `value()` answers the enum's wire number -- `1`, not `UP`. gcloud prints the
/// name, the picker paints the same cell for both backends, and its incremental
/// filter matches on what is painted, so a number here both reads wrong and
/// makes typing `up` narrow to nothing. `map_project` above takes its state by
/// name for the same reason.
pub fn map_region(region: &google_cloud_compute_v1::model::Region) -> ComputeRegion {
    ComputeRegion {
        name: region.name.clone().unwrap_or_default(),
        status: region
            .status
            .as_ref()
            .and_then(|status| status.name().map(|name| name.to_string())),
    }
}

pub fn map_zone(zone: &google_cloud_compute_v1::model::Zone) -> ComputeZone {
    ComputeZone {
        name: zone.name.clone().unwrap_or_default(),
        region: zone.region.as_deref().map(tail_segment),
        // By name rather than by number, exactly as `map_region` above.
        status: zone
            .status
            .as_ref()
            .and_then(|status| status.name().map(|name| name.to_string())),
    }
}
