//! Cloud Functions' demo backend: the listing, from the capture.
//!
//! Envelope-only. `cloud_functions.json` is the array `functions.list` wraps
//! in `ListFunctionsResponse.functions`, and the elements are the same proto3
//! JSON the generated `Function` model reads -- the same file the CLI parser
//! tests read, wrapped at runtime rather than copied.

use google_cloud_functions_v2::client::FunctionService;
use google_cloud_functions_v2::model;

use super::envelope;

/// The functions capture: `gcloud functions list --format=json`.
const FUNCTIONS: &str = include_str!("../../../../tests/fixtures/cloud_functions.json");

/// Answers `list_functions` from the capture.
///
/// One page, so the item paginator stops after it. `ListFunctions` has no
/// poller in the generated builder, and the service makes no other call, so
/// this is the only method of the trait that is reached.
#[derive(Debug, Default)]
pub struct FunctionsStub;

impl google_cloud_functions_v2::stub::FunctionService for FunctionsStub {
    async fn list_functions(
        &self,
        _request: model::ListFunctionsRequest,
        _options: google_cloud_gax::options::RequestOptions,
    ) -> google_cloud_gax::Result<google_cloud_gax::response::Response<model::ListFunctionsResponse>>
    {
        Ok(google_cloud_gax::response::Response::from(envelope(
            "functions",
            FUNCTIONS,
        )?))
    }
}

/// The client `SdkFunctionsService::from_fixtures` is built from.
pub fn client() -> FunctionService {
    FunctionService::from_stub(FunctionsStub)
}
