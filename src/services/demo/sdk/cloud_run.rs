//! Cloud Run's demo backend: the service listing, the revision listing and
//! both detail panes, all from re-shaped captures.
//!
//! This is the one service in the corpus where the envelope is not the
//! difference. `gcloud run services list` speaks Knative serving v1 --
//! `apiVersion`/`kind`/`metadata`/`spec`/`status` -- and the generated client
//! reads the Cloud Run Admin v2 resource, which carries the same facts under
//! different names and moves readiness out of `status.conditions[type=Ready]`
//! into its own `terminalCondition`. No field name turns one document into the
//! other, so each of the four Knative captures has a v2 counterpart beside it
//! and the CLI parser tests keep reading the originals unchanged.
//!
//! # What the envelope alone would have done
//!
//! [`envelope`] still supplies the `services` and `revisions`
//! fields the RPCs wrap their arrays in, but only over the re-shaped rows.
//! Pointed at the untouched Knative capture it succeeds and answers *five*
//! rows -- `ListServicesResponse` absorbs `metadata`, `spec` and `status` into
//! `_unknown_fields` and leaves every modelled field at its default -- so the
//! table paints five services with no name, no region, no URL and a `-` traffic
//! cell. That is the half-populated shape this whole backend exists to
//! prevent, and a row *count* cannot tell it from a correct listing;
//! `an_unshaped_knative_capture_fails_on_the_cells_rather_than_the_row_count`
//! in `demo_sdk_fixture_test` is the assertion that can.
//!
//! # Why a traffic status names a bare revision
//!
//! `map_service` hands `status.revision` to `traffic_summary_from` as it
//! stands, while `map_service_detail` runs the same field through
//! `name_and_region()` first. So a full `projects/.../revisions/x` path in
//! `trafficStatuses[].revision` -- which is what the sibling
//! `latestReadyRevision` carries, and what a careful author would write for
//! consistency -- paints the entire resource path into the TRAFFIC column
//! while the detail pane still looks right. The real API answers a bare
//! revision name there, so that is what these captures carry, and
//! `the_fixture_cloud_run_rows_match_the_cli_rows_parsed_from_the_same_facts`
//! compares the cell against the CLI's.
//!
//! # Why the timestamps are whole seconds in UTC
//!
//! `timestamp_to_string` renders a protobuf `Timestamp` as `%Y-%m-%dT%H:%M:%SZ`
//! and the CLI path carries the capture's own string through untouched, so the
//! two agree only for an instant that has no sub-second part and no offset.
//! Every timestamp here is whole-second `Z`, exactly as in the Knative
//! originals.
//!
//! # What these two stubs reach
//!
//! Four methods, and no more. `list_services` and `list_revisions` are read
//! through the item paginator, whose `by_page` re-`send`s with a page token;
//! leaving `next_page_token` unset is what stops it after one page.
//! `get_service` and `get_revision` are plain unary calls. The generated
//! `ListServices`, `GetService`, `ListRevisions` and `GetRevision` builders
//! carry `send`/`by_page`/`by_item` and no `poller()`, unlike the sibling
//! `DeleteService`, so nothing calls on the service's behalf and no
//! `get_operation` belongs in either list.

use google_cloud_gax::error::Error;
use google_cloud_run_v2::client::{Revisions, Services};
use google_cloud_run_v2::model;

use super::{envelope, parent_matches};

/// The service listing, re-shaped: Cloud Run Admin v2 `Service` rows mirroring
/// `tests/fixtures/cloud_run_services.json`, which is
/// `gcloud run services list --format=json` in Knative serving v1.
const SERVICES: &str = include_str!("../../../../tests/fixtures/cloud_run_services_v2.json");

/// The service detail, re-shaped: one v2 `Service` mirroring
/// `tests/fixtures/cloud_run_service_detail.json`
/// (`gcloud run services describe orders-api --format=json`).
const SERVICE_DETAIL: &str =
    include_str!("../../../../tests/fixtures/cloud_run_service_detail_v2.json");

/// The revision listing, re-shaped: v2 `Revision` rows mirroring
/// `tests/fixtures/cloud_run_revisions.json`
/// (`gcloud run revisions list --service orders-api --format=json`).
const REVISIONS: &str = include_str!("../../../../tests/fixtures/cloud_run_revisions_v2.json");

/// The revision detail, re-shaped: one v2 `Revision` mirroring
/// `tests/fixtures/cloud_run_revision_detail.json`
/// (`gcloud run revisions describe orders-api-00007-tabc --format=json`).
const REVISION_DETAIL: &str =
    include_str!("../../../../tests/fixtures/cloud_run_revision_detail_v2.json");

/// Answers the service listing and the service detail pane from the captures.
#[derive(Debug, Default)]
pub struct ServicesStub;

impl google_cloud_run_v2::stub::Services for ServicesStub {
    /// The parent is read before the capture is answered. `-` in the location
    /// segment is the wildcard `SdkCloudRunService::load_services` asks for and
    /// matches the template like any other value; what does not match is a
    /// parent assembled from the wrong pieces, which against Cloud Run is a
    /// `404` and here used to be a full table.
    async fn list_services(
        &self,
        request: model::ListServicesRequest,
        _options: google_cloud_gax::options::RequestOptions,
    ) -> google_cloud_gax::Result<google_cloud_gax::response::Response<model::ListServicesResponse>>
    {
        parent_matches(&request.parent, "projects/*/locations/*")?;
        Ok(google_cloud_gax::response::Response::from(envelope(
            "services", SERVICES,
        )?))
    }

    /// The capture is one service, so there is no field to wrap it in.
    ///
    /// The name on the request is ignored: the demo project has one captured
    /// service detail, exactly as `DemoCommandRunner` answers one
    /// `run services describe` document whichever service the argv named.
    async fn get_service(
        &self,
        _request: model::GetServiceRequest,
        _options: google_cloud_gax::options::RequestOptions,
    ) -> google_cloud_gax::Result<google_cloud_gax::response::Response<model::Service>> {
        let service: model::Service = serde_json::from_str(SERVICE_DETAIL).map_err(Error::deser)?;
        Ok(google_cloud_gax::response::Response::from(service))
    }
}

/// Answers the revision listing and the revision detail pane from the captures.
#[derive(Debug, Default)]
pub struct RevisionsStub;

impl google_cloud_run_v2::stub::Revisions for RevisionsStub {
    /// Revisions hang off one service, so the parent carries two more segments
    /// than the service listing's and the template is checked for all four.
    async fn list_revisions(
        &self,
        request: model::ListRevisionsRequest,
        _options: google_cloud_gax::options::RequestOptions,
    ) -> google_cloud_gax::Result<google_cloud_gax::response::Response<model::ListRevisionsResponse>>
    {
        parent_matches(&request.parent, "projects/*/locations/*/services/*")?;
        Ok(google_cloud_gax::response::Response::from(envelope(
            "revisions",
            REVISIONS,
        )?))
    }

    async fn get_revision(
        &self,
        _request: model::GetRevisionRequest,
        _options: google_cloud_gax::options::RequestOptions,
    ) -> google_cloud_gax::Result<google_cloud_gax::response::Response<model::Revision>> {
        let revision: model::Revision =
            serde_json::from_str(REVISION_DETAIL).map_err(Error::deser)?;
        Ok(google_cloud_gax::response::Response::from(revision))
    }
}

/// The two clients `SdkCloudRunService::from_fixtures` is built from.
pub fn clients() -> (Services, Revisions) {
    (
        Services::from_stub(ServicesStub),
        Revisions::from_stub(RevisionsStub),
    )
}
