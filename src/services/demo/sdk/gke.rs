//! GKE's demo backend: the listing and the detail pane, both from the captures.
//!
//! Two calls and two shapes. `gke_clusters.json` is the array
//! `clusters.list` wraps in `ListClustersResponse.clusters`, which is the
//! plain envelope; `gke_cluster_detail.json` is a single `Cluster` document,
//! because `clusters.get` answers the resource itself and there is no message
//! around it. The second is the structural difference the envelope helper does
//! not cover, so it is deserialized directly -- still the same bytes the CLI
//! parser reads, still one copy.
//!
//! `clusters.list` is not paginated in this API: the generated `ListClusters`
//! builder has neither `by_item` nor a poller, so the whole listing arrives in
//! this one response, exactly as `SdkGkeService::load_clusters` expects.

use google_cloud_container_v1::client::ClusterManager;
use google_cloud_container_v1::model;
use google_cloud_gax::error::Error;

use super::envelope;

/// The clusters capture: `gcloud container clusters list --format=json`.
const CLUSTERS: &str = include_str!("../../../../tests/fixtures/gke_clusters.json");

/// The detail capture: `gcloud container clusters describe --format=json`, a
/// single cluster rather than a list.
const CLUSTER_DETAIL: &str = include_str!("../../../../tests/fixtures/gke_cluster_detail.json");

/// Answers the listing and the detail pane from the two captures.
#[derive(Debug, Default)]
pub struct ClusterStub;

impl google_cloud_container_v1::stub::ClusterManager for ClusterStub {
    async fn list_clusters(
        &self,
        _request: model::ListClustersRequest,
        _options: google_cloud_gax::options::RequestOptions,
    ) -> google_cloud_gax::Result<google_cloud_gax::response::Response<model::ListClustersResponse>>
    {
        Ok(google_cloud_gax::response::Response::from(envelope(
            "clusters", CLUSTERS,
        )?))
    }

    /// The capture is one cluster, so there is no field to wrap it in.
    ///
    /// The name on the request is ignored: the demo project has one captured
    /// cluster detail, exactly as `DemoCommandRunner` answers one
    /// `clusters describe` document whichever cluster the argv named.
    async fn get_cluster(
        &self,
        _request: model::GetClusterRequest,
        _options: google_cloud_gax::options::RequestOptions,
    ) -> google_cloud_gax::Result<google_cloud_gax::response::Response<model::Cluster>> {
        let cluster: model::Cluster = serde_json::from_str(CLUSTER_DETAIL).map_err(Error::deser)?;
        Ok(google_cloud_gax::response::Response::from(cluster))
    }
}

/// The client `SdkGkeService::from_fixtures` is built from.
pub fn client() -> ClusterManager {
    ClusterManager::from_stub(ClusterStub)
}
