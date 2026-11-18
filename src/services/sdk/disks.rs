//! Persistent disks through the native client.

use google_cloud_auth::credentials::Credentials;
use google_cloud_compute_v1::client::Disks;
use nuvem_domain::gcp::resource::{DiskStatus, PersistentDisk};

use crate::services::resources::disks::{key_values, short_id};
use crate::services::resources::list_filter::{RESOURCE_LIST_LIMIT, rows_are_capped};
use crate::services::sdk::ApiError;

pub struct SdkDisksService {
    disks: Disks,
}

impl SdkDisksService {
    pub async fn connect(credentials: Credentials) -> Result<Self, ApiError> {
        let disks = Disks::builder()
            .with_credentials(credentials)
            .build()
            .await
            .map_err(|error| ApiError::new(error.to_string()))?;
        Ok(Self { disks })
    }

    /// The same service, built from an already-constructed client. See
    /// `SdkGkeService::from_clusters_for_test`.
    pub fn from_disks_for_test(disks: Disks) -> Self {
        Self { disks }
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
            disks: crate::services::demo::sdk::disks::client(),
        }
    }
    pub async fn load_disks(&self, project: &str) -> Result<Vec<PersistentDisk>, ApiError> {
        use google_cloud_gax::paginator::ItemPaginator;
        let mut rows = Vec::new();
        let mut items = self
            .disks
            .aggregated_list()
            .set_project(project)
            .set_max_results(RESOURCE_LIST_LIMIT as u32)
            // The two flags `gcloud compute disks list` sends alongside the
            // cap, verified against its own `--log-http` output. Without
            // `return_partial_success` one unreachable zone fails the whole
            // aggregated listing, where the CLI backend returned every zone
            // that answered; without `include_all_scopes` the response is
            // confined to the scopes the API decides to include by default.
            .set_include_all_scopes(true)
            .set_return_partial_success(true)
            .by_item();
        while let Some(entry) = items.next().await {
            let (_scope, scoped) = entry?;
            for disk in &scoped.disks {
                rows.push(map_disk(disk));
            }
            if rows_are_capped(rows.len(), RESOURCE_LIST_LIMIT) {
                rows.truncate(RESOURCE_LIST_LIMIT);
                break;
            }
        }
        Ok(rows)
    }
}

pub fn map_disk(disk: &google_cloud_compute_v1::model::Disk) -> PersistentDisk {
    PersistentDisk {
        name: short_id(disk.name.as_deref().unwrap_or_default()),
        // `zone` first: a disk has exactly one of the two, and zonal is the
        // overwhelmingly common case.
        location: disk
            .zone
            .as_deref()
            .or(disk.region.as_deref())
            .map(short_id)
            .unwrap_or_default(),
        status: DiskStatus::from_api(disk.status.as_ref().and_then(|status| status.name())),
        // The API sends a number where the CLI sent a decimal string; an
        // absent size stays 0, which reads as the "not reported" it is.
        size_gb: disk
            .size_gb
            .and_then(|size| u64::try_from(size).ok())
            .unwrap_or(0),
        disk_type: disk.r#type.as_deref().map(short_id).unwrap_or_default(),
        users: disk.users.iter().map(|user| short_id(user)).collect(),
        labels: key_values(
            disk.labels
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect(),
        ),
    }
}
