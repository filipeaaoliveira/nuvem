//! Service accounts and the project IAM policy through the native clients.
//!
//! Two APIs, as on the CLI path: service accounts come from IAM Admin
//! (`gcloud iam service-accounts list`) and the policy from Resource Manager
//! (`gcloud projects get-iam-policy`).

use google_cloud_auth::credentials::Credentials;
use google_cloud_iam_admin_v1::client::Iam;
use google_cloud_resourcemanager_v3::client::Projects;
use nuvem_domain::gcp::resource::{IamPolicyMember, ServiceAccount};

use crate::services::resources::list_filter::{RESOURCE_LIST_LIMIT, rows_are_capped};
use crate::services::sdk::{ApiError, non_empty};

/// The largest page `serviceAccounts.list` accepts.
///
/// Measured, not assumed: asking for [`RESOURCE_LIST_LIMIT`] here is rejected
/// outright with `INVALID_ARGUMENT`, and the listing fails rather than
/// returning a short page -- the API documents the default as 20 and the
/// maximum as 100. The row cap still applies; it is reached across pages
/// instead of within one, which is what the paginator is for.
const SERVICE_ACCOUNT_PAGE_SIZE: i32 = 100;

/// The policy version that can carry conditional bindings.
///
/// Versions 0, 1 and 3 are the valid values; only 3 expresses a condition. See
/// `load_policy` for why anything lower is unsafe here.
const CONDITIONAL_POLICY_VERSION: i32 = 3;

pub struct SdkIamService {
    accounts: Iam,
    projects: Projects,
}

impl SdkIamService {
    pub async fn connect(credentials: Credentials) -> Result<Self, ApiError> {
        let accounts = Iam::builder()
            .with_credentials(credentials.clone())
            .build()
            .await
            .map_err(|error| ApiError::new(error.to_string()))?;
        let projects = Projects::builder()
            .with_credentials(credentials)
            .build()
            .await
            .map_err(|error| ApiError::new(error.to_string()))?;
        Ok(Self { accounts, projects })
    }

    /// The same service, built from already-constructed clients. See
    /// `SdkGkeService::from_clusters_for_test`.
    pub fn from_clients_for_test(accounts: Iam, projects: Projects) -> Self {
        Self { accounts, projects }
    }

    /// The same service, answering from the demo fixtures.
    ///
    /// Required by `sdk_client_cache!`: the macro's `Fixtures` arm calls this
    /// on every service in its list, so a fifteenth service cannot join that
    /// list without one. It takes no credentials, cannot fail and cannot
    /// block -- a client built by `from_stub` holds a stub where a
    /// transport would be.
    pub fn from_fixtures() -> Self {
        let (accounts, projects) = crate::services::demo::sdk::iam::clients();
        Self { accounts, projects }
    }
    pub async fn load_service_accounts(
        &self,
        project: &str,
    ) -> Result<Vec<ServiceAccount>, ApiError> {
        use google_cloud_gax::paginator::ItemPaginator;
        let mut rows = Vec::new();
        let mut items = self
            .accounts
            .list_service_accounts()
            .set_name(format!("projects/{project}"))
            .set_page_size(SERVICE_ACCOUNT_PAGE_SIZE)
            .by_item();
        while let Some(account) = items.next().await {
            rows.push(map_service_account(&account?));
            if rows_are_capped(rows.len(), RESOURCE_LIST_LIMIT) {
                break;
            }
        }
        Ok(rows)
    }

    /// The project's IAM policy, expanded into one row per member.
    ///
    /// # Why the requested policy version is set
    ///
    /// A policy is returned at version 1 unless the request asks for more, and
    /// a version 1 response cannot express a condition: the API's own rule is
    /// that a request for a policy carrying conditional bindings must specify
    /// version 3. Left unset, a conditional grant would arrive stripped of its
    /// condition, and the row would then claim a *broader* grant than the one
    /// that exists -- which is the one direction an access table must never be
    /// wrong in. `gcloud projects get-iam-policy` asks for the highest version
    /// it supports for the same reason.
    pub async fn load_policy(&self, project: &str) -> Result<Vec<IamPolicyMember>, ApiError> {
        let policy = self
            .projects
            .get_iam_policy()
            .set_resource(format!("projects/{project}"))
            .set_options(
                google_cloud_iam_v1::model::GetPolicyOptions::new()
                    .set_requested_policy_version(CONDITIONAL_POLICY_VERSION),
            )
            .send()
            .await?;
        Ok(map_policy(&policy))
    }
}

pub fn map_service_account(
    account: &google_cloud_iam_admin_v1::model::ServiceAccount,
) -> ServiceAccount {
    ServiceAccount {
        email: account.email.clone(),
        // An empty display name is no display name: `Some("")` would paint a
        // blank cell while every `is_some()` downstream believed there was one.
        display_name: non_empty(&account.display_name),
        disabled: account.disabled,
        unique_id: account.unique_id.clone(),
    }
}

pub fn map_policy(policy: &google_cloud_iam_v1::model::Policy) -> Vec<IamPolicyMember> {
    policy
        .bindings
        .iter()
        .flat_map(|binding| {
            let title = binding
                .condition
                .as_ref()
                .map(|condition| condition.title.clone())
                .unwrap_or_default();
            let role = binding.role.clone();
            binding
                .members
                .iter()
                .map(move |member| IamPolicyMember::from_wire(&role, member, Some(title.as_str())))
        })
        .collect()
}
