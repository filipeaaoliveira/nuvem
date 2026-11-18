use std::collections::{BTreeMap, BTreeSet};

use serde::Deserialize;

use nuvem_domain::gcp::configuration::GcloudContext;
use nuvem_domain::gcp::resource::{
    ComputeKeyValue, StorageBucket, StorageBucketDetail, StorageObject, StorageObjectDetail,
    StorageObjectPage, StoragePrefix,
};

use crate::services::gcloud::cli::{CommandError, CommandRunner, GcloudCommand};

#[derive(Debug, thiserror::Error)]
pub enum StorageParseError {
    #[error("invalid Cloud Storage JSON: {0}")]
    Json(#[from] serde_json::Error),
}

#[derive(Debug, thiserror::Error)]
pub enum StorageLoadError {
    #[error("{0}")]
    Command(#[from] CommandError),
    #[error("{0}")]
    Parse(#[from] StorageParseError),
}

pub struct GcloudStorageService<R> {
    runner: R,
}

impl<R> GcloudStorageService<R> {
    pub fn new(runner: R) -> Self {
        Self { runner }
    }

    pub fn runner(&self) -> &R {
        &self.runner
    }
}

impl<R: CommandRunner> GcloudStorageService<R> {
    pub fn load_buckets(
        &self,
        context: &GcloudContext,
    ) -> Result<Vec<StorageBucket>, StorageLoadError> {
        let output = self.runner.run(GcloudCommand::new([
            "--configuration",
            context.configuration.as_str(),
            "storage",
            "buckets",
            "list",
            "--format=json",
        ]))?;
        Ok(parse_buckets_json(&output.stdout)?)
    }

    pub fn load_bucket_detail(
        &self,
        context: &GcloudContext,
        bucket_name: &str,
    ) -> Result<StorageBucketDetail, StorageLoadError> {
        let output = self.runner.run(GcloudCommand::new([
            "--configuration".to_string(),
            context.configuration.as_str().to_string(),
            "storage".to_string(),
            "buckets".to_string(),
            "describe".to_string(),
            format!("gs://{bucket_name}"),
            "--format=json".to_string(),
        ]))?;
        Ok(parse_bucket_detail_json(&output.stdout)?)
    }

    /// Lists one level of a bucket: the folders and objects directly under
    /// `prefix`.
    ///
    /// The URL carries the `**` recursive wildcard, because `objects list`
    /// matches a *pattern* rather than a prefix: `gs://b/p/` returns only the
    /// objects sitting directly at that level and never reports a folder, so a
    /// bucket whose contents are all nested one level down lists as empty.
    /// The recursive form returns every object beneath `prefix`, and
    /// [`parse_object_page_json`] groups them back into one level.
    ///
    /// `page_token` is accepted and ignored: `gcloud storage objects list`
    /// takes a page token as *input* but never emits one, so there is no token
    /// to hand back and the returned page is always the last one. The bound is
    /// `--limit`, the same flag every other resource list uses.
    pub fn load_object_page(
        &self,
        context: &GcloudContext,
        bucket: &str,
        prefix: &str,
        _page_token: Option<&str>,
        page_size: u32,
    ) -> Result<StorageObjectPage, StorageLoadError> {
        let output = self.runner.run(GcloudCommand::new(vec![
            "--configuration".to_string(),
            context.configuration.as_str().to_string(),
            "storage".to_string(),
            "objects".to_string(),
            "list".to_string(),
            format!("gs://{bucket}/{prefix}**"),
            "--format=json".to_string(),
            format!("--limit={page_size}"),
        ]))?;
        Ok(parse_object_page_json(&output.stdout, prefix)?)
    }

    pub fn load_object_detail(
        &self,
        context: &GcloudContext,
        bucket: &str,
        object_key: &str,
    ) -> Result<StorageObjectDetail, StorageLoadError> {
        let output = self.runner.run(GcloudCommand::new([
            "--configuration".to_string(),
            context.configuration.as_str().to_string(),
            "storage".to_string(),
            "objects".to_string(),
            "describe".to_string(),
            format!("gs://{bucket}/{object_key}"),
            "--format=json".to_string(),
        ]))?;
        Ok(parse_object_detail_json(&output.stdout)?)
    }
}

// gcloud's `--format=json` is NOT the Cloud Storage JSON API schema. It emits
// snake_case keys, `size` as a number rather than a string, and no
// `items`/`prefixes`/`nextPageToken` envelope. The field names below are
// gcloud's own, read off live `gcloud storage` output rather than the API
// reference — modelling the API here is what left this whole workspace unable
// to open a bucket while every fixture-backed test stayed green.
#[derive(Debug, Deserialize)]
struct RawBucket {
    name: String,
    location: Option<String>,
    location_type: Option<String>,
    #[serde(default)]
    default_storage_class: Option<String>,
    #[serde(default)]
    versioning_enabled: bool,
    #[serde(default)]
    uniform_bucket_level_access: bool,
    public_access_prevention: Option<String>,
    creation_time: Option<String>,
    update_time: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RawBucketDetail {
    name: String,
    location: Option<String>,
    location_type: Option<String>,
    #[serde(default)]
    default_storage_class: Option<String>,
    #[serde(default)]
    versioning_enabled: bool,
    #[serde(default)]
    uniform_bucket_level_access: bool,
    public_access_prevention: Option<String>,
    /// Set only on buckets that enable it, so no sampled bucket carried it;
    /// the spelling is gcloud's. Defaulted rather than required so that a
    /// bucket without it still parses.
    #[serde(default)]
    requester_pays: bool,
    #[serde(default)]
    retention_policy: Option<RawRetentionPolicy>,
    /// Only presence is retained (see `default_kms_key_present`); the key name
    /// itself is never copied into any domain struct.
    default_kms_key: Option<String>,
    #[serde(default)]
    labels: BTreeMap<String, String>,
    creation_time: Option<String>,
    update_time: Option<String>,
}

/// Nested policy dictionaries keep the API's own camelCase, even though the
/// resource around them is snake_case: gcloud passes the message straight
/// through (`self.retention_policy['retentionPeriod']` in the SDK's
/// `gcs_resource_reference.py`).
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawRetentionPolicy {
    retention_period: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RawObject {
    name: String,
    /// A number here, where the JSON API sends a decimal string.
    size: Option<u64>,
    storage_class: Option<String>,
    generation: Option<String>,
    update_time: Option<String>,
    content_type: Option<String>,
    /// Only presence is retained (see `kms_encrypted`); the key name itself
    /// is never copied into any domain struct.
    kms_key: Option<String>,
    #[serde(default)]
    temporary_hold: bool,
    #[serde(default)]
    event_based_hold: bool,
    retention_expiration: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RawObjectDetail {
    name: String,
    size: Option<u64>,
    storage_class: Option<String>,
    generation: Option<String>,
    update_time: Option<String>,
    content_type: Option<String>,
    kms_key: Option<String>,
    #[serde(default)]
    temporary_hold: bool,
    #[serde(default)]
    event_based_hold: bool,
    retention_expiration: Option<String>,
    md5_hash: Option<String>,
    crc32c_hash: Option<String>,
}

/// Removes KMS key name material from a parsed JSON value in place so it
/// never ends up in a `raw_json` field. Only presence is ever surfaced to the
/// domain layer (`kms_encrypted`, `default_kms_key_present`); the raw
/// resource name of the key must not be retained anywhere, including in the
/// "raw JSON" passthrough shown for debugging.
pub fn redact_kms_key_material(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Object(map) => {
            // Matched on the normalised key rather than a list of spellings.
            // gcloud emits `default_kms_key`, the JSON API documents
            // `defaultKmsKeyName`, and the generated client emits
            // `defaultKmsKey` -- three spellings of one field, and each time a
            // new one appeared a name-list quietly stopped redacting. Folding
            // case and dropping underscores means the next one is covered
            // before anyone meets it.
            let hits: Vec<String> = map
                .keys()
                .filter(|key| is_kms_key_name_field(key))
                .cloned()
                .collect();
            for key in hits {
                map.insert(key, serde_json::Value::String("<redacted>".to_string()));
            }
            for nested in map.values_mut() {
                redact_kms_key_material(nested);
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                redact_kms_key_material(item);
            }
        }
        _ => {}
    }
}

/// Whether a JSON key names KMS key *material* in any spelling a backend uses.
///
/// `kmsKeyServiceAccount` and similar are deliberately not matched: they name
/// an identity, not the key, and the policy here is about the key name.
fn is_kms_key_name_field(key: &str) -> bool {
    let normalised: String = key
        .chars()
        .filter(|c| *c != '_')
        .flat_map(|c| c.to_lowercase())
        .collect();
    matches!(
        normalised.as_str(),
        "kmskey" | "kmskeyname" | "defaultkmskey" | "defaultkmskeyname" | "destinationkmskey"
    )
}

pub fn parse_buckets_json(json: &str) -> Result<Vec<StorageBucket>, StorageParseError> {
    let raw: Vec<RawBucket> = serde_json::from_str(json)?;
    Ok(raw
        .into_iter()
        .map(|item| StorageBucket {
            name: item.name,
            location: item.location.unwrap_or_default(),
            location_type: item.location_type,
            storage_class: item.default_storage_class.unwrap_or_default(),
            versioning_enabled: item.versioning_enabled,
            uniform_bucket_level_access: item.uniform_bucket_level_access,
            public_access_prevention: item.public_access_prevention,
            time_created: item.creation_time,
            updated: item.update_time,
        })
        .collect())
}

pub fn parse_bucket_detail_json(json: &str) -> Result<StorageBucketDetail, StorageParseError> {
    let raw: RawBucketDetail = serde_json::from_str(json)?;
    let mut formatted: serde_json::Value = serde_json::from_str(json)?;
    redact_kms_key_material(&mut formatted);

    let retention_period = raw
        .retention_policy
        .and_then(|policy| policy.retention_period);
    let labels = raw
        .labels
        .into_iter()
        .map(|(key, value)| ComputeKeyValue { key, value })
        .collect();

    Ok(StorageBucketDetail {
        name: raw.name,
        location: raw.location.unwrap_or_default(),
        location_type: raw.location_type,
        storage_class: raw.default_storage_class.unwrap_or_default(),
        versioning_enabled: raw.versioning_enabled,
        uniform_bucket_level_access: raw.uniform_bucket_level_access,
        public_access_prevention: raw.public_access_prevention,
        requester_pays: raw.requester_pays,
        retention_period,
        default_kms_key_present: raw.default_kms_key.is_some(),
        labels,
        time_created: raw.creation_time,
        updated: raw.update_time,
        raw_json: serde_json::to_string_pretty(&formatted)?,
    })
}

/// Groups a recursive object listing back into the one level the browser shows.
///
/// gcloud returns every object beneath `prefix` as a flat array. Each name is
/// read relative to `prefix`: a remainder that still contains `/` contributes
/// its first segment as a folder, and anything else is an object sitting at
/// this level. Names stay absolute, because the browser strips the current
/// prefix itself when it renders a row.
///
/// The zero-byte placeholder some tools write for a folder has the prefix as
/// its whole name; it is dropped rather than rendered as a nameless row.
/// Whether an object is the zero-byte marker some tools write for a folder.
///
/// Its name is the prefix itself, so once the browser strips the prefix it is
/// rendering, the row has no name left: an empty, selectable 0 B line beside
/// the folder it stands for, which the delimiter already returned as a prefix.
/// Both backends drop it, and both ask here rather than spelling the
/// comparison out, so the rule is pinned in one place instead of twice in
/// bodies that reach the network.
pub fn is_folder_placeholder(object_name: &str, prefix: &str) -> bool {
    object_name == prefix
}

pub fn parse_object_page_json(
    json: &str,
    prefix: &str,
) -> Result<StorageObjectPage, StorageParseError> {
    let raw: Vec<RawObject> = serde_json::from_str(json)?;

    let mut prefixes: BTreeSet<String> = BTreeSet::new();
    let mut objects = Vec::new();
    for item in raw {
        let Some(relative) = item.name.strip_prefix(prefix) else {
            continue;
        };
        if is_folder_placeholder(&item.name, prefix) {
            continue;
        }
        match relative.split_once('/') {
            Some((folder, _)) => {
                prefixes.insert(format!("{prefix}{folder}/"));
            }
            None => objects.push(map_object(item)),
        }
    }

    Ok(StorageObjectPage {
        prefixes: prefixes
            .into_iter()
            .map(|name| StoragePrefix { name })
            .collect(),
        objects,
        // `gcloud storage objects list` accepts a page token but never emits
        // one, so there is no next page to ask for. The listing is bounded by
        // `--limit` instead, and this is always the last page.
        next_page_token: None,
    })
}

pub fn parse_object_detail_json(json: &str) -> Result<StorageObjectDetail, StorageParseError> {
    let raw: RawObjectDetail = serde_json::from_str(json)?;
    let mut formatted: serde_json::Value = serde_json::from_str(json)?;
    redact_kms_key_material(&mut formatted);

    Ok(StorageObjectDetail {
        key: raw.name,
        size_bytes: raw.size,
        storage_class: raw.storage_class,
        generation: raw.generation,
        updated: raw.update_time,
        content_type: raw.content_type,
        kms_encrypted: raw.kms_key.is_some(),
        temporary_hold: raw.temporary_hold,
        event_based_hold: raw.event_based_hold,
        retention_expiration: raw.retention_expiration,
        md5_hash: raw.md5_hash,
        crc32c: raw.crc32c_hash,
        raw_json: serde_json::to_string_pretty(&formatted)?,
    })
}

fn map_object(raw: RawObject) -> StorageObject {
    StorageObject {
        key: raw.name,
        size_bytes: raw.size,
        storage_class: raw.storage_class,
        generation: raw.generation,
        updated: raw.update_time,
        content_type: raw.content_type,
        kms_encrypted: raw.kms_key.is_some(),
        temporary_hold: raw.temporary_hold,
        event_based_hold: raw.event_based_hold,
        has_retention: raw.retention_expiration.is_some(),
    }
}
