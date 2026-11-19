//! IAM's demo backend: service accounts and the project policy, both from the
//! captures.
//!
//! Two APIs and two shapes, as on the CLI path. `iam_service_accounts.json` is
//! the array `serviceAccounts.list` wraps in `ListServiceAccountsResponse.accounts`,
//! the plain envelope. `iam_policy.json` is already a whole `Policy` document
//! -- `getIamPolicy` answers the policy itself, and gcloud printed it
//! unchanged -- so it is deserialized directly rather than wrapped.

use google_cloud_gax::error::Error;
use google_cloud_iam_admin_v1::client::Iam;
use google_cloud_resourcemanager_v3::client::Projects;

use super::envelope;
use crate::services::demo::Corpus;

/// The service accounts capture: `gcloud iam service-accounts list --format=json`.
const SERVICE_ACCOUNTS: &str = include_str!("../../../../tests/fixtures/iam_service_accounts.json");

/// The policy capture: `gcloud projects get-iam-policy --format=json`.
const POLICY: &str = include_str!("../../../../tests/fixtures/iam_policy.json");

/// What `nuvem --demo` lists: see [`Corpus::Showcase`].
const SHOWCASE_SERVICE_ACCOUNTS: &str =
    include_str!("../../../../tests/fixtures/demo/iam_service_accounts.json");

/// Answers `list_service_accounts` from the capture.
///
/// One page, so the item paginator stops after it. The generated
/// `ListServiceAccounts` builder has no poller, so no other method of this
/// trait is reached.
#[derive(Debug, Default)]
pub struct AccountsStub {
    pub corpus: Corpus,
}

impl google_cloud_iam_admin_v1::stub::Iam for AccountsStub {
    async fn list_service_accounts(
        &self,
        _request: google_cloud_iam_admin_v1::model::ListServiceAccountsRequest,
        _options: google_cloud_gax::options::RequestOptions,
    ) -> google_cloud_gax::Result<
        google_cloud_gax::response::Response<
            google_cloud_iam_admin_v1::model::ListServiceAccountsResponse,
        >,
    > {
        Ok(google_cloud_gax::response::Response::from(envelope(
            "accounts",
            self.corpus
                .pick(SERVICE_ACCOUNTS, SHOWCASE_SERVICE_ACCOUNTS),
        )?))
    }
}

/// Answers the project's IAM policy from the capture.
///
/// A different call from the one `demo::sdk::discovery` stubs on the same
/// client type, so each module names the methods its own service makes: this
/// one implements `get_iam_policy` and leaves `search_projects` defaulted,
/// and the discovery module does the reverse.
///
/// The captured policy carries `"version": 1`. Nothing here upgrades it:
/// `SdkIamService::load_policy` asks for version 3 so a real backend does not
/// strip a condition, and the capture was taken from a backend that answered
/// what it answered. The conditional binding in it is expressed regardless --
/// the version field is the API's statement about the response, not a filter
/// this stub applies.
#[derive(Debug, Default)]
pub struct PolicyStub;

impl google_cloud_resourcemanager_v3::stub::Projects for PolicyStub {
    async fn get_iam_policy(
        &self,
        _request: google_cloud_iam_v1::model::GetIamPolicyRequest,
        _options: google_cloud_gax::options::RequestOptions,
    ) -> google_cloud_gax::Result<
        google_cloud_gax::response::Response<google_cloud_iam_v1::model::Policy>,
    > {
        let policy: google_cloud_iam_v1::model::Policy =
            serde_json::from_str(POLICY).map_err(Error::deser)?;
        Ok(google_cloud_gax::response::Response::from(policy))
    }
}

/// The two clients `SdkIamService::from_fixtures` is built from.
pub fn clients() -> (Iam, Projects) {
    clients_of(Corpus::Captures)
}

/// The clients over either corpus.
pub fn clients_of(corpus: Corpus) -> (Iam, Projects) {
    (
        Iam::from_stub(AccountsStub { corpus }),
        Projects::from_stub(PolicyStub),
    )
}
