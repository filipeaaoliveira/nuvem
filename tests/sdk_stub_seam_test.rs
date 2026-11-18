//! The native services, driven by stubs instead of by the network.
//!
//! Every generated stub trait gives each of its methods a default that
//! answers "unimplemented", so a fake here implements only the call under
//! test -- fifteen lines rather than the thirty-seven methods the trait
//! declares.
//!
//! This is the seam the read migration never had. The session harnesses hold
//! no clients, so no test ever took a native branch, and the first thing to
//! exercise one was an operator: a panic in the native detail path shipped to
//! main and crashed every pane. Nothing here reaches the network.

use google_cloud_container_v1::client::ClusterManager;
use google_cloud_container_v1::model::{Cluster, ListClustersResponse};
use nuvem::services::sdk::gke::SdkGkeService;

/// Answers `list_clusters` with a planted response, and nothing else.
#[derive(Debug, Default)]
struct ClusterStub;

impl google_cloud_container_v1::stub::ClusterManager for ClusterStub {
    async fn list_clusters(
        &self,
        _req: google_cloud_container_v1::model::ListClustersRequest,
        _options: google_cloud_gax::options::RequestOptions,
    ) -> google_cloud_gax::Result<google_cloud_gax::response::Response<ListClustersResponse>> {
        Ok(google_cloud_gax::response::Response::from(
            ListClustersResponse::new().set_clusters(vec![
                Cluster::new()
                    .set_name("checkout")
                    .set_location("europe-west4")
                    .set_status(google_cloud_container_v1::model::cluster::Status::Running),
            ]),
        ))
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_stubbed_gke_service_lists_the_clusters_the_stub_planted() {
    let service = SdkGkeService::from_clusters_for_test(ClusterManager::from_stub(ClusterStub));

    let rows = service
        .load_clusters("ledger-prod")
        .await
        .expect("the stub answers");

    assert_eq!(rows.len(), 1, "the planted cluster reaches the row");
    assert_eq!(rows[0].name, "checkout");
    assert_eq!(rows[0].location, "europe-west4");
}

/// Every native service can be built from prebuilt clients.
///
/// The walk is over the source rather than over a trait, because the
/// constructors differ in arity: a service holding two clients takes two
/// arguments. What must hold is that each service module has one, so a
/// service added later arrives testable instead of arriving with the blind
/// spot this seam exists to close.
#[test]
fn every_native_service_module_has_a_test_constructor() {
    use std::fs;
    use std::path::Path;

    let directory = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/services/sdk");
    let mut missing = Vec::new();
    let mut checked = 0usize;

    for entry in fs::read_dir(&directory).expect("the native service directory is readable") {
        let path = entry.expect("a readable entry").path();
        let file = path
            .file_name()
            .and_then(|name| name.to_str())
            .expect("a named file")
            .to_string();
        if file == "mod.rs" || !file.ends_with(".rs") {
            continue;
        }
        let source = fs::read_to_string(&path).expect("a readable service module");
        // Only a module that constructs a client needs one.
        if !source.contains("pub async fn connect(") {
            continue;
        }
        checked += 1;
        if !source.contains("_for_test(") {
            missing.push(file);
        }
    }

    assert!(
        checked >= 14,
        "the walk found only {checked} services, so it stopped seeing them \
         rather than them going away"
    );
    assert!(
        missing.is_empty(),
        "these native services cannot be built from a stub, so nothing can \
         test them offline: {missing:?}"
    );
}

/// Every compute verb reaches its own method on the client.
///
/// The stub records which method was called and answers with an operation. A
/// verb wired to the wrong method is what this catches, and no unit test of
/// the call builder can see it: the builder names the method as a string, and
/// this is what proves the string and the call agree.
///
/// All five are implemented on the stub, and that is not thoroughness for its
/// own sake. A generated stub's default panics rather than returning an
/// error, so a verb this fake had not implemented would be indistinguishable
/// from a crash in the code under test.
#[tokio::test(flavor = "multi_thread")]
async fn every_compute_verb_calls_its_own_method() {
    use nuvem::app::state::resources::ComputeInstanceAction;
    use nuvem::services::sdk::compute::SdkComputeService;
    use std::sync::{Arc, Mutex};

    type Recorded = Arc<Mutex<Vec<String>>>;

    #[derive(Debug, Default)]
    struct InstancesStub {
        called: Recorded,
    }

    macro_rules! verb {
        ($method:ident, $request:ty) => {
            async fn $method(
                &self,
                req: $request,
                _options: google_cloud_gax::options::RequestOptions,
            ) -> google_cloud_gax::Result<
                google_cloud_gax::response::Response<google_cloud_compute_v1::model::Operation>,
            > {
                self.called.lock().expect("not poisoned").push(format!(
                    "{} {} {}",
                    stringify!($method),
                    req.zone,
                    req.instance
                ));
                // Answered DONE: the caller drives a poller, and an
                // operation that never completes is an infinite loop rather
                // than a test.
                Ok(google_cloud_gax::response::Response::from(
                    google_cloud_compute_v1::model::Operation::new()
                        .set_status(google_cloud_compute_v1::model::operation::Status::Done),
                ))
            }
        };
    }

    impl google_cloud_compute_v1::stub::Instances for InstancesStub {
        verb!(
            start,
            google_cloud_compute_v1::model::instances::StartRequest
        );
        verb!(stop, google_cloud_compute_v1::model::instances::StopRequest);
        verb!(
            reset,
            google_cloud_compute_v1::model::instances::ResetRequest
        );
        verb!(
            suspend,
            google_cloud_compute_v1::model::instances::SuspendRequest
        );
        verb!(
            resume,
            google_cloud_compute_v1::model::instances::ResumeRequest
        );
    }

    for action in ComputeInstanceAction::ALL {
        let called: Recorded = Arc::new(Mutex::new(Vec::new()));
        let service = SdkComputeService::from_instances_for_test(
            google_cloud_compute_v1::client::Instances::from_stub(InstancesStub {
                called: Arc::clone(&called),
            }),
        );

        service
            .run_action("ledger-prod", "europe-west4-b", "checkout-api-3", action)
            .await
            .expect("the stub answers");

        assert_eq!(
            called.lock().expect("not poisoned").as_slice(),
            [format!(
                "{} europe-west4-b checkout-api-3",
                action.command_verb()
            )],
            "{action:?} must call its own method, with the zone and instance \
             it was given"
        );
    }
}

/// Every Cloud SQL action reaches its own method, and the two patches differ.
///
/// Start and stop are the same method on the same resource and differ only in
/// the activation policy they set, so a transposition between them is
/// invisible to a test that checks the method alone. The policy is what this
/// asserts, and it is the difference between starting a database and stopping
/// one.
#[tokio::test(flavor = "multi_thread")]
async fn every_cloud_sql_action_calls_its_own_method_with_its_own_body() {
    use nuvem::app::state::resources::CloudSqlInstanceAction;
    use nuvem::services::sdk::cloudsql::SdkCloudSqlService;
    use std::sync::{Arc, Mutex};

    type Recorded = Arc<Mutex<Vec<String>>>;

    #[derive(Debug, Default)]
    struct InstancesStub {
        called: Recorded,
    }

    impl google_cloud_sql_v1::stub::SqlInstancesService for InstancesStub {
        async fn patch(
            &self,
            req: google_cloud_sql_v1::model::SqlInstancesPatchRequest,
            _options: google_cloud_gax::options::RequestOptions,
        ) -> google_cloud_gax::Result<
            google_cloud_gax::response::Response<google_cloud_sql_v1::model::Operation>,
        > {
            let policy = req
                .body
                .as_ref()
                .and_then(|body| body.settings.as_ref())
                .and_then(|settings| settings.activation_policy.name())
                .unwrap_or("none")
                .to_string();
            self.called
                .lock()
                .expect("not poisoned")
                .push(format!("patch {} {policy}", req.instance));
            Ok(google_cloud_gax::response::Response::from(
                google_cloud_sql_v1::model::Operation::new()
                    .set_status(google_cloud_sql_v1::model::operation::SqlOperationStatus::Done),
            ))
        }

        async fn restart(
            &self,
            req: google_cloud_sql_v1::model::SqlInstancesRestartRequest,
            _options: google_cloud_gax::options::RequestOptions,
        ) -> google_cloud_gax::Result<
            google_cloud_gax::response::Response<google_cloud_sql_v1::model::Operation>,
        > {
            self.called
                .lock()
                .expect("not poisoned")
                .push(format!("restart {}", req.instance));
            Ok(google_cloud_gax::response::Response::from(
                google_cloud_sql_v1::model::Operation::new()
                    .set_status(google_cloud_sql_v1::model::operation::SqlOperationStatus::Done),
            ))
        }

        async fn failover(
            &self,
            req: google_cloud_sql_v1::model::SqlInstancesFailoverRequest,
            _options: google_cloud_gax::options::RequestOptions,
        ) -> google_cloud_gax::Result<
            google_cloud_gax::response::Response<google_cloud_sql_v1::model::Operation>,
        > {
            self.called
                .lock()
                .expect("not poisoned")
                .push(format!("failover {}", req.instance));
            Ok(google_cloud_gax::response::Response::from(
                google_cloud_sql_v1::model::Operation::new()
                    .set_status(google_cloud_sql_v1::model::operation::SqlOperationStatus::Done),
            ))
        }
    }

    /// The users client this service also holds, which no lifecycle action
    /// touches. It answers nothing; reaching it would be the defect.
    #[derive(Debug, Default)]
    struct UsersStub;

    impl google_cloud_sql_v1::stub::SqlUsersService for UsersStub {}

    /// The operations client. The instances stub answers every mutation with
    /// an operation already `DONE`, so nothing here is ever read back; a call
    /// reaching it would mean the wait did not recognise a finished operation.
    #[derive(Debug, Default)]
    struct OperationsStub;

    impl google_cloud_sql_v1::stub::SqlOperationsService for OperationsStub {}

    for (action, expected) in [
        (CloudSqlInstanceAction::Start, "patch orders-db ALWAYS"),
        (CloudSqlInstanceAction::Stop, "patch orders-db NEVER"),
        (CloudSqlInstanceAction::Restart, "restart orders-db"),
        (CloudSqlInstanceAction::Failover, "failover orders-db"),
    ] {
        let called: Recorded = Arc::new(Mutex::new(Vec::new()));
        let service = SdkCloudSqlService::from_clients_for_test(
            google_cloud_sql_v1::client::SqlInstancesService::from_stub(InstancesStub {
                called: Arc::clone(&called),
            }),
            google_cloud_sql_v1::client::SqlUsersService::from_stub(UsersStub),
            google_cloud_sql_v1::client::SqlOperationsService::from_stub(OperationsStub),
        );

        service
            .run_action("ledger-prod", "orders-db", action)
            .await
            .expect("the stub answers");

        assert_eq!(
            called.lock().expect("not poisoned").as_slice(),
            [expected.to_string()],
            "{action:?} must reach its own method with its own body"
        );
    }
}

/// A compute action waits for its operation, and reports a failed one.
///
/// Sending the request only gets the mutation accepted; the work continues in
/// GCP afterwards. The operations log has one word for the end of a lifecycle
/// action and it has always meant finished, so the native path waits rather
/// than teaching that word a second meaning that depends on which backend
/// answered.
///
/// Two stubs, because the interesting cases are opposite. One answers an
/// operation that is already done: the call returns and the log is right to
/// say it finished. The other answers one carrying an error: the call must
/// fail, because reporting a rejected operation as a success is the failure
/// this whole phase exists to prevent.
#[tokio::test(flavor = "multi_thread")]
async fn a_compute_action_fails_when_its_operation_carries_an_error() {
    use nuvem::app::state::resources::ComputeInstanceAction;
    use nuvem::services::sdk::compute::SdkComputeService;

    #[derive(Debug, Default)]
    struct FailingStub;

    impl google_cloud_compute_v1::stub::Instances for FailingStub {
        async fn stop(
            &self,
            _req: google_cloud_compute_v1::model::instances::StopRequest,
            _options: google_cloud_gax::options::RequestOptions,
        ) -> google_cloud_gax::Result<
            google_cloud_gax::response::Response<google_cloud_compute_v1::model::Operation>,
        > {
            Ok(google_cloud_gax::response::Response::from(
                google_cloud_compute_v1::model::Operation::new()
                    .set_status(google_cloud_compute_v1::model::operation::Status::Done)
                    .set_http_error_status_code(409)
                    .set_http_error_message("instance is already stopping"),
            ))
        }
    }

    let service = SdkComputeService::from_instances_for_test(
        google_cloud_compute_v1::client::Instances::from_stub(FailingStub),
    );

    let outcome = service
        .run_action(
            "ledger-prod",
            "europe-west4-b",
            "checkout-api-3",
            ComputeInstanceAction::Stop,
        )
        .await;

    assert!(
        outcome.is_err(),
        "an operation that finished with an error is not a successful action: \
         reporting it as one would settle the operations log at Succeeded for \
         work GCP refused"
    );
}

/// A Cloud SQL action reads its operation back until it is done.
///
/// `sql-v1` generates no poller, so the wait is an explicit read and this is
/// what proves it happens: the patch answers an operation still `RUNNING`,
/// and the call may not return until a later read says `DONE`. Without the
/// wait the operations log would say a database had stopped while it was
/// still stopping.
#[tokio::test(flavor = "multi_thread")]
async fn a_cloud_sql_action_waits_for_its_operation_to_finish() {
    use nuvem::app::state::resources::CloudSqlInstanceAction;
    use nuvem::services::sdk::cloudsql::SdkCloudSqlService;
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    #[derive(Debug, Default)]
    struct RunningPatchStub;

    impl google_cloud_sql_v1::stub::SqlInstancesService for RunningPatchStub {
        async fn patch(
            &self,
            _req: google_cloud_sql_v1::model::SqlInstancesPatchRequest,
            _options: google_cloud_gax::options::RequestOptions,
        ) -> google_cloud_gax::Result<
            google_cloud_gax::response::Response<google_cloud_sql_v1::model::Operation>,
        > {
            Ok(google_cloud_gax::response::Response::from(
                google_cloud_sql_v1::model::Operation::new()
                    .set_name("op-1")
                    .set_status(google_cloud_sql_v1::model::operation::SqlOperationStatus::Running),
            ))
        }
    }

    #[derive(Debug, Default)]
    struct UsersStub;
    impl google_cloud_sql_v1::stub::SqlUsersService for UsersStub {}

    #[derive(Debug, Default)]
    struct OperationsStub {
        reads: Arc<AtomicUsize>,
    }

    impl google_cloud_sql_v1::stub::SqlOperationsService for OperationsStub {
        async fn get(
            &self,
            req: google_cloud_sql_v1::model::SqlOperationsGetRequest,
            _options: google_cloud_gax::options::RequestOptions,
        ) -> google_cloud_gax::Result<
            google_cloud_gax::response::Response<google_cloud_sql_v1::model::Operation>,
        > {
            assert_eq!(
                req.operation, "op-1",
                "it reads back the operation it started"
            );
            // Still running on the first read, done on the second: a wait that
            // stopped at the first answer would pass a test where the first
            // answer was already done.
            let status = if self.reads.fetch_add(1, Ordering::SeqCst) == 0 {
                google_cloud_sql_v1::model::operation::SqlOperationStatus::Running
            } else {
                google_cloud_sql_v1::model::operation::SqlOperationStatus::Done
            };
            Ok(google_cloud_gax::response::Response::from(
                google_cloud_sql_v1::model::Operation::new()
                    .set_name("op-1")
                    .set_status(status),
            ))
        }
    }

    let reads = Arc::new(AtomicUsize::new(0));
    let service = SdkCloudSqlService::from_clients_for_test(
        google_cloud_sql_v1::client::SqlInstancesService::from_stub(RunningPatchStub),
        google_cloud_sql_v1::client::SqlUsersService::from_stub(UsersStub),
        google_cloud_sql_v1::client::SqlOperationsService::from_stub(OperationsStub {
            reads: Arc::clone(&reads),
        }),
    );

    service
        .run_action("ledger-prod", "orders-db", CloudSqlInstanceAction::Stop)
        .await
        .expect("the operation finishes on the second read");

    assert_eq!(
        reads.load(Ordering::SeqCst),
        2,
        "the wait read the operation back until it said done, rather than \
         returning on the first answer"
    );
}
