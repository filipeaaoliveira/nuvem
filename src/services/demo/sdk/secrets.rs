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

use google_cloud_secretmanager_v1::client::SecretManagerService;
use google_cloud_secretmanager_v1::model;

use super::{envelope, named, short_name};
use crate::services::demo::Corpus;

/// The secrets capture: `gcloud secrets list --format=json`.
const SECRETS: &str = include_str!("../../../../tests/fixtures/secrets.json");

/// The versions capture: `gcloud secrets versions list --format=json`.
const VERSIONS: &str = include_str!("../../../../tests/fixtures/secret_versions.json");

/// The detail capture: `gcloud secrets describe --format=json`, one secret.
const SECRET_DETAIL: &str = include_str!("../../../../tests/fixtures/secret_detail.json");

/// What `nuvem --demo` lists: see [`Corpus::Showcase`].
const SHOWCASE_SECRETS: &str = include_str!("../../../../tests/fixtures/demo/secrets.json");

/// Answers the three metadata reads the service makes.
///
/// Both listings leave `next_page_token` unset, so each item paginator reads
/// one page and stops. Neither `ListSecrets` nor `ListSecretVersions` nor
/// `GetSecret` has a poller in the generated builder, so no fourth method is
/// reached on this stub's behalf.
#[derive(Debug, Default)]
pub struct SecretsStub {
    pub corpus: Corpus,
}

impl google_cloud_secretmanager_v1::stub::SecretManagerService for SecretsStub {
    async fn list_secrets(
        &self,
        _request: model::ListSecretsRequest,
        _options: google_cloud_gax::options::RequestOptions,
    ) -> google_cloud_gax::Result<google_cloud_gax::response::Response<model::ListSecretsResponse>>
    {
        Ok(google_cloud_gax::response::Response::from(envelope(
            "secrets",
            self.corpus.pick(SECRETS, SHOWCASE_SECRETS),
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

    /// The secret the request names: the detail capture for the secret it
    /// describes, and the listing's own row for every other one. See
    /// [`named`].
    async fn get_secret(
        &self,
        request: model::GetSecretRequest,
        _options: google_cloud_gax::options::RequestOptions,
    ) -> google_cloud_gax::Result<google_cloud_gax::response::Response<model::Secret>> {
        let wanted = short_name(&request.name);
        let secret: model::Secret = named(
            &self
                .corpus
                .documents(&[SECRET_DETAIL, SECRETS], &[SHOWCASE_SECRETS]),
            |name| short_name(name) == wanted,
            &format!("secret `{wanted}`"),
        )?;
        Ok(google_cloud_gax::response::Response::from(secret))
    }
}

/// The client `SdkSecretsService::from_fixtures` is built from.
pub fn client() -> SecretManagerService {
    client_of(Corpus::Captures)
}

/// The client over either corpus.
pub fn client_of(corpus: Corpus) -> SecretManagerService {
    SecretManagerService::from_stub(SecretsStub { corpus })
}
