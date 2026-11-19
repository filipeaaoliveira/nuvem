//! Cloud Storage through the native client.
//!
//! The CLI path this replaces could not express what the browser needs. `gcloud
//! storage objects list` returns a flat array with no folders and no page
//! token, so folders had to be grouped from object keys and a listing was
//! always its own last page. The API answers `objects`, `prefixes` and
//! `next_page_token` directly, which is the shape the browser was designed
//! against in the first place: `set_delimiter` asks the service to do the
//! grouping, and `set_page_token` fetches the next page for real.

use google_cloud_auth::credentials::Credentials;
use google_cloud_storage::client::StorageControl;
use nuvem_domain::gcp::resource::{
    ComputeKeyValue, StorageBucket, StorageBucketDetail, StorageObject, StorageObjectDetail,
    StorageObjectPage, StoragePrefix,
};

use crate::services::resources::storage::is_folder_placeholder;
use crate::services::sdk::{ApiError, detail_json, non_empty, timestamp_to_string};

/// The delimiter that makes a listing describe one level rather than a subtree.
const DELIMITER: &str = "/";

pub struct SdkStorageService {
    control: StorageControl,
}

impl SdkStorageService {
    pub async fn connect(credentials: Credentials) -> Result<Self, ApiError> {
        let control = StorageControl::builder()
            .with_credentials(credentials)
            .build()
            .await
            .map_err(|error| ApiError::new(error.to_string()))?;
        Ok(Self { control })
    }

    /// The same service, built from an already-constructed client. See
    /// `SdkGkeService::from_clusters_for_test`.
    pub fn from_control_for_test(control: StorageControl) -> Self {
        Self { control }
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
            control: crate::services::demo::sdk::storage::client(),
        }
    }

    /// The same service over the demo showcase, which is what `nuvem --demo`
    /// lists. See [`crate::services::demo::Corpus`].
    pub fn from_showcase() -> Self {
        Self {
            control: crate::services::demo::sdk::storage::client_of(
                crate::services::demo::Corpus::Showcase,
            ),
        }
    }
    pub async fn load_buckets(&self, project: &str) -> Result<Vec<StorageBucket>, ApiError> {
        use google_cloud_gax::paginator::ItemPaginator;

        let mut rows = Vec::new();
        let mut items = self
            .control
            .list_buckets()
            .set_parent(format!("projects/{project}"))
            .by_item();
        while let Some(bucket) = items.next().await {
            rows.push(map_bucket(&bucket?));
        }
        Ok(rows)
    }

    /// One level of a bucket: the folders and objects directly under `prefix`.
    ///
    /// `page_token` is a token the service issued, unlike the CLI path where
    /// no token was ever emitted and the parameter had to be ignored.
    pub async fn load_object_page(
        &self,
        bucket: &str,
        prefix: &str,
        page_token: Option<&str>,
        page_size: u32,
    ) -> Result<StorageObjectPage, ApiError> {
        let mut request = self
            .control
            .list_objects()
            .set_parent(format!("projects/_/buckets/{bucket}"))
            .set_prefix(prefix)
            .set_delimiter(DELIMITER)
            .set_page_size(page_size as i32);
        if let Some(token) = page_token {
            request = request.set_page_token(token);
        }
        let page = request.send().await?;

        Ok(StorageObjectPage {
            prefixes: page
                .prefixes
                .iter()
                .map(|name| StoragePrefix { name: name.clone() })
                .collect(),
            // The zero-byte placeholder some tools write for a folder has the
            // prefix as its whole name. The delimiter already returns the
            // folder itself under `prefixes`, so passing the marker through
            // would add a second row for the same folder -- and one whose name
            // is empty once the browser strips the prefix it is rendering,
            // giving a selectable 0 B row with no name. The CLI path drops it
            // in `parse_object_page_json` for the same reason.
            objects: page
                .objects
                .iter()
                .filter(|object| !is_folder_placeholder(&object.name, prefix))
                .map(map_object)
                .collect(),
            next_page_token: non_empty(&page.next_page_token),
        })
    }
    /// One bucket described in full, for the detail pane.
    pub async fn load_bucket_detail(&self, bucket: &str) -> Result<StorageBucketDetail, ApiError> {
        let bucket = self
            .control
            .get_bucket()
            .set_name(format!("projects/_/buckets/{bucket}"))
            .send()
            .await?;
        Ok(map_bucket_detail(&bucket))
    }

    /// One object described in full, for the detail pane.
    pub async fn load_object_detail(
        &self,
        bucket: &str,
        object: &str,
    ) -> Result<StorageObjectDetail, ApiError> {
        let object = self
            .control
            .get_object()
            .set_bucket(format!("projects/_/buckets/{bucket}"))
            .set_object(object)
            .send()
            .await?;
        Ok(map_object_detail(&object))
    }
}

/// `projects/_/buckets/<id>` is how the API names a bucket; the rows show the
/// id alone, as the CLI did.
fn bucket_id(bucket: &google_cloud_storage::model::Bucket) -> String {
    if bucket.bucket_id.is_empty() {
        bucket
            .name
            .rsplit_once('/')
            .map(|(_, id)| id.to_string())
            .unwrap_or_else(|| bucket.name.clone())
    } else {
        bucket.bucket_id.clone()
    }
}

fn uniform_access(bucket: &google_cloud_storage::model::Bucket) -> bool {
    bucket
        .iam_config
        .as_ref()
        .and_then(|config| config.uniform_bucket_level_access.as_ref())
        .map(|access| access.enabled)
        .unwrap_or(false)
}

fn public_access_prevention(bucket: &google_cloud_storage::model::Bucket) -> Option<String> {
    bucket
        .iam_config
        .as_ref()
        .and_then(|config| non_empty(&config.public_access_prevention))
}

pub fn map_bucket(bucket: &google_cloud_storage::model::Bucket) -> StorageBucket {
    StorageBucket {
        name: bucket_id(bucket),
        location: bucket.location.clone(),
        location_type: non_empty(&bucket.location_type),
        storage_class: bucket.storage_class.clone(),
        versioning_enabled: bucket
            .versioning
            .as_ref()
            .map(|versioning| versioning.enabled)
            .unwrap_or(false),
        uniform_bucket_level_access: uniform_access(bucket),
        public_access_prevention: public_access_prevention(bucket),
        time_created: timestamp_to_string(bucket.create_time.as_ref()),
        updated: timestamp_to_string(bucket.update_time.as_ref()),
    }
}

pub fn map_object(object: &google_cloud_storage::model::Object) -> StorageObject {
    StorageObject {
        key: object.name.clone(),
        size_bytes: u64::try_from(object.size).ok(),
        storage_class: non_empty(&object.storage_class),
        generation: (object.generation != 0).then(|| object.generation.to_string()),
        updated: timestamp_to_string(object.update_time.as_ref()),
        content_type: non_empty(&object.content_type),
        kms_encrypted: !object.kms_key.is_empty(),
        temporary_hold: object.temporary_hold,
        event_based_hold: object.event_based_hold.unwrap_or(false),
        has_retention: object.retention_expire_time.is_some(),
    }
}

pub fn map_bucket_detail(bucket: &google_cloud_storage::model::Bucket) -> StorageBucketDetail {
    StorageBucketDetail {
        name: bucket_id(bucket),
        location: bucket.location.clone(),
        location_type: non_empty(&bucket.location_type),
        storage_class: bucket.storage_class.clone(),
        versioning_enabled: bucket
            .versioning
            .as_ref()
            .map(|versioning| versioning.enabled)
            .unwrap_or(false),
        uniform_bucket_level_access: uniform_access(bucket),
        public_access_prevention: public_access_prevention(bucket),
        requester_pays: bucket
            .billing
            .as_ref()
            .map(|billing| billing.requester_pays)
            .unwrap_or(false),
        // The CLI reads a seconds count as a string; the client models the
        // retention duration, so the same number is rendered the same way.
        retention_period: bucket
            .retention_policy
            .as_ref()
            .and_then(|policy| policy.retention_duration.as_ref())
            .map(|duration| duration.seconds().to_string()),
        // Only presence is ever surfaced -- the name itself is redacted above
        // and must not reach the domain layer.
        default_kms_key_present: bucket
            .encryption
            .as_ref()
            .map(|encryption| !encryption.default_kms_key.is_empty())
            .unwrap_or(false),
        labels: bucket
            .labels
            .iter()
            .map(|(key, value)| ComputeKeyValue {
                key: key.clone(),
                value: value.clone(),
            })
            .collect(),
        time_created: timestamp_to_string(bucket.create_time.as_ref()),
        updated: timestamp_to_string(bucket.update_time.as_ref()),
        raw_json: detail_json(bucket),
    }
}

pub fn map_object_detail(object: &google_cloud_storage::model::Object) -> StorageObjectDetail {
    let checksums = object.checksums.as_ref();
    StorageObjectDetail {
        key: object.name.clone(),
        size_bytes: u64::try_from(object.size).ok(),
        storage_class: non_empty(&object.storage_class),
        generation: (object.generation != 0).then(|| object.generation.to_string()),
        updated: timestamp_to_string(object.update_time.as_ref()),
        content_type: non_empty(&object.content_type),
        kms_encrypted: !object.kms_key.is_empty(),
        temporary_hold: object.temporary_hold,
        event_based_hold: object.event_based_hold.unwrap_or(false),
        retention_expiration: timestamp_to_string(object.retention_expire_time.as_ref()),
        md5_hash: checksums
            .filter(|sums| !sums.md5_hash.is_empty())
            .map(|sums| encode_md5(&sums.md5_hash)),
        crc32c: checksums.and_then(|sums| sums.crc32c).map(encode_crc32c),
        raw_json: detail_json(object),
    }
}

/// gcloud prints these as base64, which is what the pane expects.
fn encode_md5(digest: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(digest)
}

fn encode_crc32c(value: u32) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(value.to_be_bytes())
}
