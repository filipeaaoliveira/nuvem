//! Cloud Functions through the native client.

use google_cloud_auth::credentials::Credentials;
use google_cloud_functions_v2::client::FunctionService;
use nuvem_domain::gcp::resource::{CloudFunction, CloudFunctionEnvironment, CloudFunctionState};

use crate::services::resources::functions::{key_values, region_from_path, short_id};
use crate::services::resources::list_filter::{RESOURCE_LIST_LIMIT, rows_are_capped};
use crate::services::sdk::{ApiError, non_empty, timestamp_to_string};

pub struct SdkFunctionsService {
    functions: FunctionService,
}

impl SdkFunctionsService {
    pub async fn connect(credentials: Credentials) -> Result<Self, ApiError> {
        let functions = FunctionService::builder()
            .with_credentials(credentials)
            .build()
            .await
            .map_err(|error| ApiError::new(error.to_string()))?;
        Ok(Self { functions })
    }

    /// The same service, built from an already-constructed client. See
    /// `SdkGkeService::from_clusters_for_test`.
    pub fn from_functions_for_test(functions: FunctionService) -> Self {
        Self { functions }
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
            functions: crate::services::demo::sdk::functions::client(),
        }
    }
    pub async fn load_functions(&self, project: &str) -> Result<Vec<CloudFunction>, ApiError> {
        use google_cloud_gax::paginator::ItemPaginator;
        let mut rows = Vec::new();
        let mut items = self
            .functions
            .list_functions()
            .set_parent(format!("projects/{project}/locations/-"))
            .set_page_size(RESOURCE_LIST_LIMIT as i32)
            .by_item();
        while let Some(function) = items.next().await {
            rows.push(map_function(&function?));
            if rows_are_capped(rows.len(), RESOURCE_LIST_LIMIT) {
                break;
            }
        }
        Ok(rows)
    }
}

pub fn map_function(function: &google_cloud_functions_v2::model::Function) -> CloudFunction {
    CloudFunction {
        name: short_id(&function.name),
        // Read out of the resource path, as on the CLI path: there is no region
        // field, and a function still deploying has no URL to read one from.
        region: region_from_path(&function.name),
        state: CloudFunctionState::from_api(function.state.name()),
        environment: CloudFunctionEnvironment::from_api(function.environment.name()),
        url: non_empty(&function.url),
        update_time: timestamp_to_string(function.update_time.as_ref()),
        labels: key_values(
            function
                .labels
                .iter()
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect(),
        ),
    }
}
