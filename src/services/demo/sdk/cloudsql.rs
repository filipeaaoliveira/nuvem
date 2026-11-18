//! Cloud SQL's demo backend: the listing, the detail pane, the users tab and
//! the four lifecycle actions, all from the captures.
//!
//! Three shapes, and each one is here for a reason the others do not cover.
//! `cloudsql_instances.json` and `cloudsql_users.json` are arrays that the
//! RPCs wrap in `InstancesListResponse.items` and `UsersListResponse.items`,
//! which is the plain [`envelope`].
//! `cloudsql_instance_detail.json` is a single `DatabaseInstance` document,
//! because `instances.get` answers the resource itself, so it is deserialized
//! directly. All three are the same bytes the CLI parser reads; no second copy
//! of any capture exists.
//!
//! # What a lifecycle action reaches
//!
//! `sql-v1` generates no poller, unlike Compute, so `SdkCloudSqlService`
//! drives the wait itself: `run_action` sends `patch`, `restart` or
//! `failover`, then `await_operation` loops until the operation reports
//! `Done`, sleeping `OPERATION_POLL_INTERVAL_MILLIS` between reads through
//! `operations.get`. Every one of those four methods is reachable, so every
//! one is spelled out.
//!
//! # Why the operation is answered already finished
//!
//! `await_operation` checks the status *before* it sleeps, so an operation
//! that is already `Done` returns on the loop's first pass and a demo action
//! pays none of the 500 ms interval. Answering a `Running` operation instead
//! would cost that sleep on every action -- and, because `operations.get` here
//! answers the same constant, would never finish at all: the loop would sleep
//! and re-read the same unfinished operation forever, with nothing to time it
//! out. A demo mutation that hangs is worse than one that refuses, so the
//! status is `Done` and `operations.get` answers the same finished operation
//! the verbs do.
//!
//! The operation carries no `error` either. `await_operation` reports one as a
//! failed action, exactly as `finished_or_error` does on the Compute side.
//!
//! # Why a demo mutation changes nothing
//!
//! Restarting an instance here does not restart it. The captures are
//! `include_str!` constants compiled into the binary, so there is no state for
//! an action to write to: the table paints the same rows after a Stop as
//! before it, and `pg-standalone` still reads `RUNNABLE`. That is deliberate,
//! and it is what the CLI demo backend already does -- `DemoCommandRunner`
//! answers a lifecycle argv with empty stdout and a zero exit, changing
//! nothing -- so the flip at `services/runtime/session.rs` keeps demo mode's
//! behaviour rather than changing it. What an action does prove is the path:
//! the confirmation, the operations-log entry and its settlement all run for
//! real.

use google_cloud_sql_v1::client::{SqlInstancesService, SqlOperationsService, SqlUsersService};
use google_cloud_sql_v1::model;

use super::{envelope, named};

/// The instances capture: `gcloud sql instances list --format=json`.
const INSTANCES: &str = include_str!("../../../../tests/fixtures/cloudsql_instances.json");

/// The detail capture: `gcloud sql instances describe --format=json`, a single
/// instance rather than a list.
const INSTANCE_DETAIL: &str =
    include_str!("../../../../tests/fixtures/cloudsql_instance_detail.json");

/// The users capture: `gcloud sql users list --format=json`.
const USERS: &str = include_str!("../../../../tests/fixtures/cloudsql_users.json");

/// The operation every lifecycle action answers: already `Done`, and carrying
/// no error. See the module docs for why it is both.
fn finished() -> model::Operation {
    model::Operation::new()
        .set_name("operation-demo-fixture")
        .set_status(model::operation::SqlOperationStatus::Done)
}

/// Answers the listing, the detail pane and the three mutating methods from
/// the captures.
#[derive(Debug, Default)]
pub struct InstancesStub;

impl google_cloud_sql_v1::stub::SqlInstancesService for InstancesStub {
    /// One page: `next_page_token` is left at its empty default, so the item
    /// paginator `load_instances` drains reads this as the only page and stops
    /// rather than asking for a second one.
    async fn list(
        &self,
        _request: model::SqlInstancesListRequest,
        _options: google_cloud_gax::options::RequestOptions,
    ) -> google_cloud_gax::Result<google_cloud_gax::response::Response<model::InstancesListResponse>>
    {
        Ok(google_cloud_gax::response::Response::from(envelope(
            "items", INSTANCES,
        )?))
    }

    /// The instance the request names: the detail capture for the instance it
    /// describes, and the listing's own row for every other one. See
    /// [`named`].
    async fn get(
        &self,
        request: model::SqlInstancesGetRequest,
        _options: google_cloud_gax::options::RequestOptions,
    ) -> google_cloud_gax::Result<google_cloud_gax::response::Response<model::DatabaseInstance>>
    {
        let instance: model::DatabaseInstance = named(
            &[INSTANCE_DETAIL, INSTANCES],
            |name| name == request.instance,
            &format!("Cloud SQL instance `{}`", request.instance),
        )?;
        Ok(google_cloud_gax::response::Response::from(instance))
    }

    /// Start and Stop both land here: Cloud SQL has no such methods, and
    /// `run_action` expresses them as a patch of the activation policy. The
    /// body is discarded -- there is nothing to patch.
    async fn patch(
        &self,
        _request: model::SqlInstancesPatchRequest,
        _options: google_cloud_gax::options::RequestOptions,
    ) -> google_cloud_gax::Result<google_cloud_gax::response::Response<model::Operation>> {
        Ok(google_cloud_gax::response::Response::from(finished()))
    }

    async fn restart(
        &self,
        _request: model::SqlInstancesRestartRequest,
        _options: google_cloud_gax::options::RequestOptions,
    ) -> google_cloud_gax::Result<google_cloud_gax::response::Response<model::Operation>> {
        Ok(google_cloud_gax::response::Response::from(finished()))
    }

    async fn failover(
        &self,
        _request: model::SqlInstancesFailoverRequest,
        _options: google_cloud_gax::options::RequestOptions,
    ) -> google_cloud_gax::Result<google_cloud_gax::response::Response<model::Operation>> {
        Ok(google_cloud_gax::response::Response::from(finished()))
    }
}

/// Answers the users tab from the capture.
#[derive(Debug, Default)]
pub struct UsersStub;

impl google_cloud_sql_v1::stub::SqlUsersService for UsersStub {
    async fn list(
        &self,
        _request: model::SqlUsersListRequest,
        _options: google_cloud_gax::options::RequestOptions,
    ) -> google_cloud_gax::Result<google_cloud_gax::response::Response<model::UsersListResponse>>
    {
        Ok(google_cloud_gax::response::Response::from(envelope(
            "items", USERS,
        )?))
    }
}

/// Answers the read-back `await_operation` makes on the service's behalf.
#[derive(Debug, Default)]
pub struct OperationsStub;

impl google_cloud_sql_v1::stub::SqlOperationsService for OperationsStub {
    /// Unreachable while the verbs answer a finished operation, because
    /// `await_operation` returns before its first sleep. Spelled out anyway:
    /// the generated default is `unimplemented!()`, which on a blocking effect
    /// thread aborts nuvem, and this method is what a verb answering anything
    /// other than `Done` would immediately land on.
    async fn get(
        &self,
        _request: model::SqlOperationsGetRequest,
        _options: google_cloud_gax::options::RequestOptions,
    ) -> google_cloud_gax::Result<google_cloud_gax::response::Response<model::Operation>> {
        Ok(google_cloud_gax::response::Response::from(finished()))
    }
}

/// The three clients `SdkCloudSqlService::from_fixtures` is built from.
pub fn clients() -> (SqlInstancesService, SqlUsersService, SqlOperationsService) {
    (
        SqlInstancesService::from_stub(InstancesStub),
        SqlUsersService::from_stub(UsersStub),
        SqlOperationsService::from_stub(OperationsStub),
    )
}
