//! Secret Manager secrets through the native client.
//!
//! Only names, labels and replication are read here, as on the CLI path: a
//! secret's *value* is never fetched by a listing.

use google_cloud_auth::credentials::Credentials;
use google_cloud_secretmanager_v1::client::SecretManagerService;
use nuvem_domain::gcp::resource::{Secret, SecretDetail, SecretVersion};

use crate::services::resources::list_filter::{RESOURCE_LIST_LIMIT, rows_are_capped};
use crate::services::resources::secrets::{
    key_values, replication_label_from, rotation_label_from, short_id, version_state,
};
use crate::services::sdk::{ApiError, detail_json, timestamp_to_string};

pub struct SdkSecretsService {
    secrets: SecretManagerService,
}

impl SdkSecretsService {
    pub async fn connect(credentials: Credentials) -> Result<Self, ApiError> {
        let secrets = SecretManagerService::builder()
            .with_credentials(credentials)
            .build()
            .await
            .map_err(|error| ApiError::new(error.to_string()))?;
        Ok(Self { secrets })
    }

    /// The same service, built from an already-constructed client. See
    /// `SdkGkeService::from_clusters_for_test`.
    pub fn from_secrets_for_test(secrets: SecretManagerService) -> Self {
        Self { secrets }
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
            secrets: crate::services::demo::sdk::secrets::client(),
        }
    }
    pub async fn load_secrets(&self, project: &str) -> Result<Vec<Secret>, ApiError> {
        use google_cloud_gax::paginator::ItemPaginator;
        let mut rows = Vec::new();
        let mut items = self
            .secrets
            .list_secrets()
            .set_parent(format!("projects/{project}"))
            .set_page_size(RESOURCE_LIST_LIMIT as i32)
            .by_item();
        while let Some(secret) = items.next().await {
            rows.push(map_secret(&secret?));
            if rows_are_capped(rows.len(), RESOURCE_LIST_LIMIT) {
                break;
            }
        }
        Ok(rows)
    }

    /// The versions of one secret: their identity and state only. This is the
    /// `versions.list` call, which returns metadata; the call that would return
    /// a payload is `access`, and nothing here makes it.
    pub async fn load_secret_versions(
        &self,
        project: &str,
        secret: &str,
    ) -> Result<Vec<SecretVersion>, ApiError> {
        use google_cloud_gax::paginator::ItemPaginator;
        let mut rows = Vec::new();
        let mut items = self
            .secrets
            .list_secret_versions()
            .set_parent(format!("projects/{project}/secrets/{secret}"))
            .by_item();
        while let Some(version) = items.next().await {
            rows.push(map_version(&version?));
        }
        Ok(rows)
    }

    /// One secret described in full, for the detail pane.
    ///
    /// `versions` stays empty: the pane fetches them separately, and this is
    /// still metadata only -- nothing here calls `access`.
    pub async fn load_secret_detail(
        &self,
        project: &str,
        secret: &str,
    ) -> Result<SecretDetail, ApiError> {
        let secret = self
            .secrets
            .get_secret()
            .set_name(format!("projects/{project}/secrets/{secret}"))
            .send()
            .await?;
        Ok(map_secret_detail(&secret))
    }
}

pub fn map_version(version: &google_cloud_secretmanager_v1::model::SecretVersion) -> SecretVersion {
    SecretVersion {
        name: short_id(&version.name),
        state: version_state(version.state.name()),
        create_time: timestamp_to_string(version.create_time.as_ref()),
    }
}

pub fn map_secret(secret: &google_cloud_secretmanager_v1::model::Secret) -> Secret {
    use google_cloud_secretmanager_v1::model::replication::Replication as Kind;

    let (automatic, user_managed) = match secret
        .replication
        .as_ref()
        .and_then(|replication| replication.replication.as_ref())
    {
        Some(Kind::Automatic(_)) => (true, None),
        Some(Kind::UserManaged(managed)) => (
            false,
            Some(
                managed
                    .replicas
                    .iter()
                    .map(|replica| replica.location.clone())
                    .filter(|location| !location.is_empty())
                    .collect::<Vec<_>>(),
            ),
        ),
        _ => (false, None),
    };

    Secret {
        name: short_id(&secret.name),
        create_time: timestamp_to_string(secret.create_time.as_ref()),
        labels: key_values(
            secret
                .labels
                .iter()
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect(),
        ),
        replication: replication_label_from(automatic, user_managed),
    }
}

pub fn map_secret_detail(secret: &google_cloud_secretmanager_v1::model::Secret) -> SecretDetail {
    use google_cloud_secretmanager_v1::model::replication::Replication as Kind;
    let (automatic, user_managed) = match secret
        .replication
        .as_ref()
        .and_then(|replication| replication.replication.as_ref())
    {
        Some(Kind::Automatic(_)) => (true, None),
        Some(Kind::UserManaged(managed)) => (
            false,
            Some(
                managed
                    .replicas
                    .iter()
                    .map(|replica| replica.location.clone())
                    .filter(|location| !location.is_empty())
                    .collect::<Vec<_>>(),
            ),
        ),
        _ => (false, None),
    };

    SecretDetail {
        name: short_id(&secret.name),
        create_time: timestamp_to_string(secret.create_time.as_ref()),
        labels: key_values(
            secret
                .labels
                .iter()
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect(),
        ),
        annotations: key_values(
            secret
                .annotations
                .iter()
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect(),
        ),
        replication: replication_label_from(automatic, user_managed),
        rotation: secret.rotation.as_ref().and_then(|rotation| {
            rotation_label_from(
                rotation
                    .rotation_period
                    .as_ref()
                    .map(|period| format!("{}s", period.seconds())),
                timestamp_to_string(rotation.next_rotation_time.as_ref()),
            )
        }),
        // `expiration` is a oneof: an absolute time or a TTL. Only the
        // absolute form is a timestamp the pane can show, and a TTL secret
        // reports no expire time rather than a computed one.
        expire_time: secret
            .expire_time()
            .map(|at| at.as_ref())
            .and_then(|at| timestamp_to_string(Some(at))),
        // Filled by the separate versions fetch, as on the CLI path.
        versions: Vec::new(),
        raw_json: detail_json(secret),
    }
}
