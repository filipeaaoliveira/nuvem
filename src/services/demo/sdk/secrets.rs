//! Secret Manager's demo backend: the listing, the versions and the detail
//! pane, all from the captures.
//!
//! Metadata only, as on every other path: the call that would return a payload
//! is `access`, and nothing here makes it.
//!
//! Two of the three are plain envelopes -- `secrets.json` into
//! `ListSecretsResponse.secrets` and `secret_versions.json` into
//! `ListSecretVersionsResponse.versions`. `secret_detail.json` is a single
//! `Secret` document, because `secrets.get` answers the resource itself, so it
//! is deserialized directly. All three are the captures the CLI parser reads.

use google_cloud_gax::error::Error;
use google_cloud_secretmanager_v1::client::SecretManagerService;
use google_cloud_secretmanager_v1::model;

use super::envelope;

/// The secrets capture: `gcloud secrets list --format=json`.
const SECRETS: &str = include_str!("../../../../tests/fixtures/secrets.json");

/// The versions capture: `gcloud secrets versions list --format=json`.
const VERSIONS: &str = include_str!("../../../../tests/fixtures/secret_versions.json");

/// The detail capture: `gcloud secrets describe --format=json`, one secret.
const SECRET_DETAIL: &str = include_str!("../../../../tests/fixtures/secret_detail.json");

/// Answers the three metadata reads the service makes.
///
/// Both listings leave `next_page_token` unset, so each item paginator reads
/// one page and stops. Neither `ListSecrets` nor `ListSecretVersions` nor
/// `GetSecret` has a poller in the generated builder, so no fourth method is
/// reached on this stub's behalf.
#[derive(Debug, Default)]
pub struct SecretsStub;

impl google_cloud_secretmanager_v1::stub::SecretManagerService for SecretsStub {
    async fn list_secrets(
        &self,
        _request: model::ListSecretsRequest,
        _options: google_cloud_gax::options::RequestOptions,
    ) -> google_cloud_gax::Result<google_cloud_gax::response::Response<model::ListSecretsResponse>>
    {
        Ok(google_cloud_gax::response::Response::from(envelope(
            "secrets", SECRETS,
        )?))
    }

    async fn list_secret_versions(
        &self,
        _request: model::ListSecretVersionsRequest,
        _options: google_cloud_gax::options::RequestOptions,
    ) -> google_cloud_gax::Result<
        google_cloud_gax::response::Response<model::ListSecretVersionsResponse>,
    > {
        Ok(google_cloud_gax::response::Response::from(envelope(
            "versions", VERSIONS,
        )?))
    }

    /// One secret, so there is no field to wrap it in. The requested name is
    /// ignored for the same reason `DemoCommandRunner` answers one
    /// `secrets describe` document whichever secret the argv named.
    async fn get_secret(
        &self,
        _request: model::GetSecretRequest,
        _options: google_cloud_gax::options::RequestOptions,
    ) -> google_cloud_gax::Result<google_cloud_gax::response::Response<model::Secret>> {
        let secret: model::Secret = serde_json::from_str(SECRET_DETAIL).map_err(Error::deser)?;
        Ok(google_cloud_gax::response::Response::from(secret))
    }
}

/// The client `SdkSecretsService::from_fixtures` is built from.
pub fn client() -> SecretManagerService {
    SecretManagerService::from_stub(SecretsStub)
}
