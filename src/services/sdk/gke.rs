//! GKE clusters through the native client.

use google_cloud_auth::credentials::Credentials;
use google_cloud_container_v1::client::ClusterManager;
use nuvem_domain::gcp::resource::{ComputeKeyValue, GkeCluster, GkeClusterDetail, GkeNodePool};

use crate::services::resources::gke::{
    GCE_PD_CSI_DRIVER, HORIZONTAL_POD_AUTOSCALING, HTTP_LOAD_BALANCING, addons_from,
    autoscaling_label, endpoint_visibility, parse_cluster_status, tail_segment,
};
use crate::services::resources::list_filter::RESOURCE_LIST_LIMIT;
use crate::services::sdk::{ApiError, detail_json};

pub struct SdkGkeService {
    clusters: ClusterManager,
}

impl SdkGkeService {
    pub async fn connect(credentials: Credentials) -> Result<Self, ApiError> {
        let clusters = ClusterManager::builder()
            .with_credentials(credentials)
            .build()
            .await
            .map_err(|error| ApiError::new(error.to_string()))?;
        Ok(Self { clusters })
    }

    /// The same service, built from an already-constructed client.
    ///
    /// `connect` needs credentials and reaches the network on first use. This
    /// takes a client a test built with `ClusterManager::from_stub`, so the
    /// call shape and the mapping can be exercised with no network, no
    /// credentials and no `gcloud` binary. Named for what it is: nothing in
    /// the application calls it.
    pub fn from_clusters_for_test(clusters: ClusterManager) -> Self {
        Self { clusters }
    }

    /// The same service, answering from the demo fixtures.
    ///
    /// Required by `sdk_client_cache!`: the macro's `Fixtures` arm calls this
    /// on every service in its list, so a fifteenth service cannot join that
    /// list without one. It takes no credentials, cannot fail and cannot
    /// block -- a client built by `from_stub` holds a stub where a
    /// transport would be.
    pub fn from_fixtures() -> Self {
        Self {
            clusters: crate::services::demo::sdk::gke::client(),
        }
    }

    /// The same service over the demo showcase, which is what `nuvem --demo`
    /// lists. See [`crate::services::demo::Corpus`].
    pub fn from_showcase() -> Self {
        Self {
            clusters: crate::services::demo::sdk::gke::client_of(
                crate::services::demo::Corpus::Showcase,
            ),
        }
    }

    /// Clusters in every location.
    ///
    /// `-` as the location is the API's own wildcard, and it is what
    /// `gcloud container clusters list` asks for when no `--region` or
    /// `--zone` narrows it.
    pub async fn load_clusters(&self, project: &str) -> Result<Vec<GkeCluster>, ApiError> {
        let response = self
            .clusters
            .list_clusters()
            .set_parent(format!("projects/{project}/locations/-"))
            .send()
            .await?;
        // `clusters.list` takes no page size -- it answers with every cluster
        // in one response -- so the cap applies to what came back, which is
        // what `--limit` does for this call on the CLI path too.
        Ok(response
            .clusters
            .iter()
            .take(RESOURCE_LIST_LIMIT)
            .map(map_cluster)
            .collect())
    }
    /// One cluster described in full, for the detail pane.
    pub async fn load_cluster_detail(
        &self,
        project: &str,
        location: &str,
        cluster: &str,
    ) -> Result<GkeClusterDetail, ApiError> {
        let cluster = self
            .clusters
            .get_cluster()
            .set_name(format!(
                "projects/{project}/locations/{location}/clusters/{cluster}"
            ))
            .send()
            .await?;
        Ok(map_cluster_detail(&cluster))
    }
}

/// Where the cluster says it lives, from whichever of the two fields carries
/// it.
///
/// `location` supersedes `zone`, but a zonal cluster created before the change
/// still answers `zone` and leaves `location` empty, and the table's REGION
/// cell has to show it either way. `parse_clusters_json` has always done this
/// fallback (`item.location.or(item.zone)`); reading `location` alone here
/// painted a blank cell for `private-prod` where the CLI painted
/// `europe-west1-b`, which is the divergence
/// `the_fixture_gke_rows_match_the_cli_rows_parsed_from_the_same_bytes`
/// recorded before `--demo` ran on this backend.
fn cluster_location(cluster: &google_cloud_container_v1::model::Cluster) -> String {
    if !cluster.location.is_empty() {
        return cluster.location.clone();
    }
    #[allow(deprecated)]
    cluster.zone.clone()
}

/// The reported node count, or `None` when the cluster reports none.
///
/// proto3 has no absent-vs-zero for a scalar: the wire form omits the key
/// exactly when the value is zero, so `0` here is what an omitted
/// `currentNodeCount` deserializes to and is indistinguishable from a real
/// zero. The CLI reads the same bytes into `Option<u32>` and gets `None`, so
/// folding zero back to `None` is what keeps one wire document painting one
/// cell on both backends -- and it is what the two other unset-folds beside it
/// already do for `""`. Passing the zero through claimed a cluster had zero
/// nodes where the CLI painted a dash, which is the silently-painted default
/// this migration exists to avoid.
///
/// A cluster that genuinely runs zero nodes therefore paints a dash. That is
/// the CLI's behaviour today and not a loss introduced here; the number is
/// deprecated at the source, which tells callers to ask the Kubernetes API
/// rather than to trust this field.
fn cluster_node_count(cluster: &google_cloud_container_v1::model::Cluster) -> Option<u32> {
    // Deprecated with no replacement in this API — the note says to ask the
    // Kubernetes API instead, which this application does not talk to. The CLI
    // path reads the same field through gcloud, so nothing is lost by
    // continuing to read it here; it is spelled out rather than silenced
    // blindly.
    #[allow(deprecated)]
    let count = cluster.current_node_count;
    u32::try_from(count).ok().filter(|count| *count != 0)
}

pub fn map_cluster(cluster: &google_cloud_container_v1::model::Cluster) -> GkeCluster {
    // `PrivateClusterConfig.enable_private_endpoint` is deprecated in favour of
    // `ControlPlaneEndpointsConfig.IPEndpointsConfig.enable_public_endpoint`,
    // whose sense is *reversed*. The new field is preferred where the cluster
    // reports it; the old one is the fallback, because a cluster that predates
    // the new config still answers only the old question.
    let private = cluster
        .control_plane_endpoints_config
        .as_ref()
        .and_then(|config| config.ip_endpoints_config.as_ref())
        .and_then(|endpoints| endpoints.enable_public_endpoint)
        .map(|public| !public)
        .unwrap_or_else(|| {
            #[allow(deprecated)]
            cluster
                .private_cluster_config
                .as_ref()
                .map(|config| config.enable_private_endpoint)
                .unwrap_or(false)
        });
    // The generated model uses `""` and `0` for unset where the CLI's JSON
    // omitted the key, so the empties are folded back to `None`: a blank
    // version is not a version, and a cluster with no reported node count is
    // not a cluster with zero nodes. See `cluster_node_count` for the second.
    let endpoint = (!cluster.endpoint.is_empty()).then(|| cluster.endpoint.clone());

    GkeCluster {
        name: cluster.name.clone(),
        location: cluster_location(cluster),
        status: parse_cluster_status(cluster.status.name().unwrap_or_default()),
        current_master_version: (!cluster.current_master_version.is_empty())
            .then(|| cluster.current_master_version.clone()),
        current_node_count: cluster_node_count(cluster),
        endpoint_visibility: endpoint_visibility(endpoint.as_ref(), private),
    }
}

pub fn map_cluster_detail(cluster: &google_cloud_container_v1::model::Cluster) -> GkeClusterDetail {
    let addons = cluster.addons_config.as_ref();
    GkeClusterDetail {
        name: cluster.name.clone(),
        // The same `location`-or-`zone` fallback the row uses: a cluster that
        // reports only `zone` must not read blank on one screen and placed on
        // the other.
        location: cluster_location(cluster),
        status: parse_cluster_status(cluster.status.name().unwrap_or_default()),
        current_master_version: (!cluster.current_master_version.is_empty())
            .then(|| cluster.current_master_version.clone()),
        current_node_version: {
            // Deprecated with no replacement, and read on the CLI path too --
            // spelled out rather than silenced blindly.
            #[allow(deprecated)]
            let version = cluster.current_node_version.clone();
            (!version.is_empty()).then_some(version)
        },
        endpoint: (!cluster.endpoint.is_empty()).then(|| cluster.endpoint.clone()),
        // The same preferred/fallback pair the row uses, rather than the
        // deprecated field alone: a cluster reporting the new config would
        // otherwise read private on one screen and public on the other.
        private_cluster: !endpoint_visibility_public(cluster),
        network: (!cluster.network.is_empty()).then(|| tail_segment(&cluster.network)),
        subnetwork: (!cluster.subnetwork.is_empty()).then(|| tail_segment(&cluster.subnetwork)),
        node_pools: cluster
            .node_pools
            .iter()
            .map(|pool| GkeNodePool {
                name: pool.name.clone(),
                status: parse_cluster_status(pool.status.name().unwrap_or_default()),
                version: (!pool.version.is_empty()).then(|| pool.version.clone()),
                machine_type: pool
                    .config
                    .as_ref()
                    .map(|config| config.machine_type.clone())
                    .filter(|machine| !machine.is_empty()),
                autoscaling: pool.autoscaling.as_ref().map(|scaling| {
                    autoscaling_label(
                        scaling.enabled,
                        u32::try_from(scaling.min_node_count).ok(),
                        u32::try_from(scaling.max_node_count).ok(),
                    )
                }),
                node_count: u32::try_from(pool.initial_node_count).ok(),
            })
            .collect(),
        addons: addons_from(&[
            (
                HTTP_LOAD_BALANCING,
                addons
                    .and_then(|config| config.http_load_balancing.as_ref())
                    .map(|addon| !addon.disabled),
            ),
            (
                HORIZONTAL_POD_AUTOSCALING,
                addons
                    .and_then(|config| config.horizontal_pod_autoscaling.as_ref())
                    .map(|addon| !addon.disabled),
            ),
            (
                GCE_PD_CSI_DRIVER,
                addons
                    .and_then(|config| config.gce_persistent_disk_csi_driver_config.as_ref())
                    .map(|addon| addon.enabled),
            ),
        ]),
        labels: cluster
            .resource_labels
            .iter()
            .map(|(key, value)| ComputeKeyValue {
                key: key.clone(),
                value: value.clone(),
            })
            .collect(),
        raw_json: detail_json(cluster),
    }
}

/// Whether the control plane exposes a public endpoint, preferring the current
/// config over the deprecated one whose sense is reversed.
fn endpoint_visibility_public(cluster: &google_cloud_container_v1::model::Cluster) -> bool {
    cluster
        .control_plane_endpoints_config
        .as_ref()
        .and_then(|config| config.ip_endpoints_config.as_ref())
        .and_then(|endpoints| endpoints.enable_public_endpoint)
        .unwrap_or_else(|| {
            #[allow(deprecated)]
            let private = cluster
                .private_cluster_config
                .as_ref()
                .map(|config| config.enable_private_endpoint)
                .unwrap_or(false);
            !private
        })
}
