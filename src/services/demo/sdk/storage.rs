//! Cloud Storage's demo backend: buckets, one level of a bucket's objects and
//! both detail panes, all from re-shaped captures.
//!
//! gcloud's `--format=json` for `storage` is not the API's schema and not even
//! a renaming of it: it is the CLI's own invented vocabulary, printed from the
//! Python SDK's resource objects. `default_storage_class` for `storageClass`,
//! a flat `versioning_enabled` for `versioning.enabled`, flat
//! `uniform_bucket_level_access` and `public_access_prevention` for the two
//! fields of `iamConfig`, a `storage_url` the API has no field for at all, and
//! -- the one that is a change of units rather than of spelling --
//! `md5_hash`/`crc32c_hash` as base64 strings beside a `checksums` message
//! that carries the CRC as a decoded unsigned integer. So each of the four
//! captures has a Storage v2 counterpart beside it, and the CLI parser tests
//! keep reading the originals unchanged.
//!
//! # The checksum the re-shaping has to convert rather than rename
//!
//! gcloud prints `crc32c_hash` as base64 of the four big-endian bytes;
//! `ObjectChecksums.crc32c` is a `fixed32`, so the v2 capture carries the
//! *number* those bytes encode and `map_object_detail` encodes it back with
//! `encode_crc32c`. `AAAAAA==` is four zero bytes, hence `"crc32c": 0`, and
//! that zero has to be written out: the field is an `Option<u32>`, so omitting
//! it is `None` and blanks the CRC32C line in the pane rather than printing
//! the `AAAAAA==` the CLI pane prints. `md5Hash` needs no conversion --
//! protobuf `bytes` is base64 in JSON, which is the same string gcloud prints.
//!
//! # Why the object listing is regrouped at call time
//!
//! `objects.list` answers `objects`, `prefixes` and `next_page_token` for ONE
//! level of the bucket, decided by the `prefix` and `delimiter` on the
//! request. The capture is the flat recursive listing gcloud printed, which is
//! what `parse_object_page_json` groups on the CLI path -- so a single canned
//! page here would answer the root listing for every folder the operator opens,
//! and `photos/` would paint `top-level.txt`. `object_page` does the grouping
//! the service does, off the request's own prefix and delimiter, exactly as
//! [`by_scope`](super::by_scope) rebuilds an aggregated listing rather than
//! recording the grouping in a second file.
//!
//! Rows keep the capture's order, which is not the lexicographic order the
//! live API answers in. The order is the *original's* order, because these
//! captures exist to be compared row for row against it: sorting here would
//! sort only one of the two backends.
//!
//! # What this stub reaches
//!
//! Four methods. `list_buckets` is read through the item paginator, so
//! `next_page_token` is left unset to stop it after one page; `list_objects`
//! is a single `send()` because the browser pages by hand. `get_bucket` and
//! `get_object` are unary. None of the four generated builders has a
//! `poller()` -- `StorageControl`'s long-running verbs are folder renames and
//! recursive deletes, which nothing here reaches -- so no `get_operation`
//! belongs in this list.

use std::collections::BTreeSet;

use google_cloud_gax::error::Error;
use google_cloud_storage::client::StorageControl;
use google_cloud_storage::model;

use super::{envelope, parent_matches};

/// The bucket listing, re-shaped: Storage v2 `Bucket` rows mirroring
/// `tests/fixtures/gcs_buckets.json` (`gcloud storage buckets list
/// --format=json`).
const BUCKETS: &str = include_str!("../../../../tests/fixtures/gcs_buckets_v2.json");

/// The bucket detail, re-shaped: one v2 `Bucket` mirroring
/// `tests/fixtures/gcs_bucket_detail.json` (`gcloud storage buckets describe
/// gs://alpha-bucket --format=json`).
const BUCKET_DETAIL: &str = include_str!("../../../../tests/fixtures/gcs_bucket_detail_v2.json");

/// The object listing, re-shaped: v2 `Object` rows mirroring
/// `tests/fixtures/gcs_objects.json` (`gcloud storage objects list
/// 'gs://alpha-bucket/**' --format=json`), row for row and in the same order.
const OBJECTS: &str = include_str!("../../../../tests/fixtures/gcs_objects_v2.json");

/// The object detail, re-shaped: one v2 `Object` mirroring
/// `tests/fixtures/gcs_object_detail.json` (`gcloud storage objects describe
/// --format=json`).
const OBJECT_DETAIL: &str = include_str!("../../../../tests/fixtures/gcs_object_detail_v2.json");

/// The flat capture grouped into the one level a delimited listing answers.
///
/// Each name is read relative to `prefix`; a remainder that still contains the
/// delimiter contributes its first segment as a folder, and anything else is an
/// object sitting at this level. Names stay absolute, as the API answers them.
///
/// An empty delimiter is the recursive listing the API answers when the caller
/// asks for no grouping: every matching object is returned and no prefix is.
/// `SdkStorageService::load_object_page` always sets one, but a stub that
/// silently ignored the field would answer a shape the live service never
/// answers for that request.
fn object_page(prefix: &str, delimiter: &str) -> Result<model::ListObjectsResponse, Error> {
    let rows: Vec<model::Object> = serde_json::from_str(OBJECTS).map_err(Error::deser)?;

    let mut prefixes: BTreeSet<String> = BTreeSet::new();
    let mut objects = Vec::new();
    for object in rows {
        let Some(relative) = object.name.strip_prefix(prefix) else {
            continue;
        };
        match relative
            .split_once(delimiter)
            .filter(|_| !delimiter.is_empty())
        {
            Some((folder, _)) => {
                prefixes.insert(format!("{prefix}{folder}{delimiter}"));
            }
            None => objects.push(object),
        }
    }

    Ok(model::ListObjectsResponse::new()
        .set_objects(objects)
        .set_prefixes(prefixes))
}

/// Answers both listings and both detail panes from the captures.
#[derive(Debug, Default)]
pub struct ControlStub;

impl google_cloud_storage::stub::StorageControl for ControlStub {
    /// The parent is read before the capture is answered. Storage takes the
    /// project as a resource path rather than as the bare id Compute wants, so
    /// the two mistakes are opposites and neither is visible in a row.
    async fn list_buckets(
        &self,
        request: model::ListBucketsRequest,
        _options: google_cloud_gax::options::RequestOptions,
    ) -> google_cloud_gax::Result<google_cloud_gax::response::Response<model::ListBucketsResponse>>
    {
        parent_matches(&request.parent, "projects/*")?;
        Ok(google_cloud_gax::response::Response::from(envelope(
            "buckets", BUCKETS,
        )?))
    }

    /// The capture is one bucket, so there is no field to wrap it in. The name
    /// on the request is ignored, as `DemoCommandRunner` ignores the argv's.
    async fn get_bucket(
        &self,
        _request: model::GetBucketRequest,
        _options: google_cloud_gax::options::RequestOptions,
    ) -> google_cloud_gax::Result<google_cloud_gax::response::Response<model::Bucket>> {
        let bucket: model::Bucket = serde_json::from_str(BUCKET_DETAIL).map_err(Error::deser)?;
        Ok(google_cloud_gax::response::Response::from(bucket))
    }

    /// The prefix and delimiter ARE read here, unlike every other request in
    /// this module: they are what decides which level of the bucket the answer
    /// describes, and the browser opens folders by changing them.
    async fn list_objects(
        &self,
        request: model::ListObjectsRequest,
        _options: google_cloud_gax::options::RequestOptions,
    ) -> google_cloud_gax::Result<google_cloud_gax::response::Response<model::ListObjectsResponse>>
    {
        // `_` rather than a project id: an object listing is addressed by
        // bucket, and the bucket names its own project. It is a literal in the
        // template and is not looked up.
        parent_matches(&request.parent, "projects/_/buckets/*")?;
        Ok(google_cloud_gax::response::Response::from(object_page(
            &request.prefix,
            &request.delimiter,
        )?))
    }

    async fn get_object(
        &self,
        _request: model::GetObjectRequest,
        _options: google_cloud_gax::options::RequestOptions,
    ) -> google_cloud_gax::Result<google_cloud_gax::response::Response<model::Object>> {
        let object: model::Object = serde_json::from_str(OBJECT_DETAIL).map_err(Error::deser)?;
        Ok(google_cloud_gax::response::Response::from(object))
    }
}

/// The client `SdkStorageService::from_fixtures` is built from.
pub fn client() -> StorageControl {
    StorageControl::from_stub(ControlStub)
}
