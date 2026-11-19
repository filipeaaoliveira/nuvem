//! Cloud SQL instances through the native client.

use google_cloud_auth::credentials::Credentials;
use google_cloud_sql_v1::client::{SqlInstancesService, SqlOperationsService, SqlUsersService};
use nuvem_domain::gcp::resource::{
    CloudSqlBackupConfig, CloudSqlInstance, CloudSqlInstanceDetail, CloudSqlMaintenanceWindow,
    CloudSqlUser, ComputeKeyValue,
};

use crate::app::state::resources::CloudSqlInstanceAction;
use crate::services::resources::cloudsql::{
    connectivity, normalize_engine, parse_availability, parse_status,
};
use crate::services::resources::list_filter::{RESOURCE_LIST_LIMIT, rows_are_capped};
use crate::services::sdk::{ApiError, detail_json, non_empty};

/// How long to wait between reads of a running operation.
const OPERATION_POLL_INTERVAL_MILLIS: u64 = 500;

pub struct SdkCloudSqlService {
    instances: SqlInstancesService,
    users: SqlUsersService,
    /// Read back to find out whether a mutation finished.
    ///
    /// `sql-v1` generates no poller, unlike Compute, so waiting for an
    /// operation here is an explicit read rather than machinery the client
    /// provides.
    operations: SqlOperationsService,
}

impl SdkCloudSqlService {
    pub async fn connect(credentials: Credentials) -> Result<Self, ApiError> {
        let instances = SqlInstancesService::builder()
            .with_credentials(credentials.clone())
            .build()
            .await
            .map_err(|error| ApiError::new(error.to_string()))?;
        let users = SqlUsersService::builder()
            .with_credentials(credentials.clone())
            .build()
            .await
            .map_err(|error| ApiError::new(error.to_string()))?;
        let operations = SqlOperationsService::builder()
            .with_credentials(credentials)
            .build()
            .await
            .map_err(|error| ApiError::new(error.to_string()))?;
        Ok(Self {
            instances,
            users,
            operations,
        })
    }

    /// The same service, built from already-constructed clients. See
    /// `SdkGkeService::from_clusters_for_test`.
    pub fn from_clients_for_test(
        instances: SqlInstancesService,
        users: SqlUsersService,
        operations: SqlOperationsService,
    ) -> Self {
        Self {
            instances,
            users,
            operations,
        }
    }

    /// The same service, answering from the demo fixtures.
    ///
    /// Required by `sdk_client_cache!`: the macro's `Fixtures` arm calls this
    /// on every service in its list, so a fifteenth service cannot join that
    /// list without one. It takes no credentials, cannot fail and cannot
    /// block -- a client built by `from_stub` holds a stub where a
    /// transport would be.
    pub fn from_fixtures() -> Self {
        let (instances, users, operations) = crate::services::demo::sdk::cloudsql::clients();
        Self {
            instances,
            users,
            operations,
        }
    }

    /// The same service over the demo showcase, which is what `nuvem --demo`
    /// lists. See [`crate::services::demo::Corpus`].
    pub fn from_showcase() -> Self {
        let (instances, users, operations) = crate::services::demo::sdk::cloudsql::clients_of(
            crate::services::demo::Corpus::Showcase,
        );
        Self {
            instances,
            users,
            operations,
        }
    }

    /// Every instance in the project.
    ///
    /// # Why this drains a paginator rather than reading one response
    ///
    /// `InstancesListResponse` carries a `next_page_token`, so a single
    /// `send()` returns the first page and nothing says so: no error, no
    /// notice, and a table that is quietly short. The CLI path follows the
    /// token to `--limit=500`, so reading one page here would make the two
    /// backends disagree about which instances exist.
    pub async fn load_instances(&self, project: &str) -> Result<Vec<CloudSqlInstance>, ApiError> {
        use google_cloud_gax::paginator::ItemPaginator;
        let mut rows = Vec::new();
        let mut items = self
            .instances
            .list()
            .set_project(project)
            .set_max_results(RESOURCE_LIST_LIMIT as u32)
            .by_item();
        while let Some(instance) = items.next().await {
            rows.push(map_instance(&instance?));
            if rows_are_capped(rows.len(), RESOURCE_LIST_LIMIT) {
                break;
            }
        }
        Ok(rows)
    }

    /// Runs one lifecycle action against one instance.
    ///
    /// Returning `Ok` means the operation finished.
    ///
    /// Sending the request only gets it accepted. Compute drives a generated
    /// poller; `sql-v1` generates none, so the operation is read back here
    /// until it reports done. The operations log has one word for the end of
    /// a lifecycle action, and it means the same thing on both backends.
    pub async fn run_action(
        &self,
        project: &str,
        instance: &str,
        action: CloudSqlInstanceAction,
    ) -> Result<(), ApiError> {
        use google_cloud_sql_v1::model::settings::SqlActivationPolicy;

        let started = match action {
            // Cloud SQL has no start or stop method. Both are a patch of the
            // activation policy, which is why the confirmation names the field
            // it sets: without it the two read identically.
            CloudSqlInstanceAction::Start | CloudSqlInstanceAction::Stop => {
                let policy = if matches!(action, CloudSqlInstanceAction::Start) {
                    SqlActivationPolicy::Always
                } else {
                    SqlActivationPolicy::Never
                };
                // Only the settings, and within them only the policy. A body
                // built from a whole `DatabaseInstance` would send every other
                // field at its zero value, and for a patch that means clearing
                // settings the operator never touched.
                self.instances
                    .patch()
                    .set_project(project)
                    .set_instance(instance)
                    .set_body(
                        google_cloud_sql_v1::model::DatabaseInstance::new().set_settings(
                            google_cloud_sql_v1::model::Settings::new()
                                .set_activation_policy(policy),
                        ),
                    )
                    .send()
                    .await?
            }
            CloudSqlInstanceAction::Restart => {
                self.instances
                    .restart()
                    .set_project(project)
                    .set_instance(instance)
                    .send()
                    .await?
            }
            CloudSqlInstanceAction::Failover => {
                self.instances
                    .failover()
                    .set_project(project)
                    .set_instance(instance)
                    .send()
                    .await?
            }
        };
        self.await_operation(project, started).await
    }

    /// Reads one operation back until it reports done.
    ///
    /// The interval is fixed and short. A lifecycle operation takes seconds to
    /// minutes, the read is cheap, and a backoff schedule here would be a
    /// second policy to reason about beside the one Compute's generated poller
    /// already applies to the other half of the same feature.
    async fn await_operation(
        &self,
        project: &str,
        started: google_cloud_sql_v1::model::Operation,
    ) -> Result<(), ApiError> {
        use google_cloud_sql_v1::model::operation::SqlOperationStatus;

        let mut operation = started;
        loop {
            if let Some(errors) = &operation.error {
                let detail = errors
                    .errors
                    .iter()
                    .map(|entry| entry.message.clone())
                    .filter(|message| !message.is_empty())
                    .collect::<Vec<_>>()
                    .join("; ");
                return Err(ApiError::new(if detail.is_empty() {
                    "the operation failed".to_string()
                } else {
                    detail
                }));
            }
            if matches!(operation.status, SqlOperationStatus::Done) {
                return Ok(());
            }
            // An operation with no name cannot be read back, and looping on it
            // would spin forever. Treated as accepted-but-unobserved rather
            // than as failure: the mutation was taken.
            if operation.name.is_empty() {
                return Ok(());
            }
            tokio::time::sleep(std::time::Duration::from_millis(
                OPERATION_POLL_INTERVAL_MILLIS,
            ))
            .await;
            operation = self
                .operations
                .get()
                .set_project(project)
                .set_operation(&operation.name)
                .send()
                .await?;
        }
    }

    /// The users configured on one instance. A password is never part of this
    /// response, and nothing here reads one.
    pub async fn load_users(
        &self,
        project: &str,
        instance: &str,
    ) -> Result<Vec<CloudSqlUser>, ApiError> {
        let response = self
            .users
            .list()
            .set_project(project)
            .set_instance(instance)
            .send()
            .await?;
        Ok(response.items.iter().map(map_user).collect())
    }

    /// One instance described in full, for the detail pane.
    ///
    /// `users` stays empty here as on the CLI path: the Users tab fetches them
    /// separately, and filling them eagerly would cost a second call on every
    /// drill-in whether or not the tab is opened.
    pub async fn load_instance_detail(
        &self,
        project: &str,
        instance: &str,
    ) -> Result<CloudSqlInstanceDetail, ApiError> {
        let instance = self
            .instances
            .get()
            .set_project(project)
            .set_instance(instance)
            .send()
            .await?;
        Ok(map_instance_detail(&instance))
    }
}

/// One user row.
///
/// # Why the TYPE column is not the model's `kind`
///
/// `CloudSqlUser::kind` is the TYPE column, and TYPE is the authentication
/// type: `BUILT_IN`, `CLOUD_IAM_USER`, `CLOUD_IAM_SERVICE_ACCOUNT`. The CLI
/// parser reads it off the `type` key -- its own field is spelled
/// `#[serde(rename = "type")] kind` -- and this read the generated model's
/// `kind` instead, which is a different field: the client documents it as
/// "This is always `sql#user`", a constant. So the column painted `sql#user`
/// for every user against the live API, and a blank cell against a capture
/// that states no `kind` key, where the CLI painted the type. The domain field
/// being NAMED `kind` while carrying the type is what made the wrong field
/// look like the right one.
pub fn map_user(user: &google_cloud_sql_v1::model::User) -> CloudSqlUser {
    CloudSqlUser {
        name: user.name.clone(),
        host: non_empty(&user.host),
        // The enum's own spelling, which is the word the CLI path carries
        // through verbatim. `name()` is `None` only for a value that arrived
        // as an integer this build has no word for, and a blank cell says
        // that better than a number would.
        kind: user.r#type.name().map(str::to_string),
    }
}
pub fn map_instance(instance: &google_cloud_sql_v1::model::DatabaseInstance) -> CloudSqlInstance {
    let settings = instance.settings.as_ref();
    let ip_configuration = settings.and_then(|settings| settings.ip_configuration.as_ref());
    // A private network is a non-empty string here where the CLI saw a present
    // key, so "configured" is "not empty" rather than "is Some".
    let private = ip_configuration
        .map(|config| !config.private_network.is_empty())
        .unwrap_or(false);
    let database_version = instance
        .database_version
        .name()
        .unwrap_or_default()
        .to_string();

    CloudSqlInstance {
        engine: normalize_engine(&database_version),
        database_version,
        name: instance.name.clone(),
        region: instance.region.clone(),
        tier: settings
            .map(|settings| settings.tier.clone())
            .unwrap_or_default(),
        availability: parse_availability(
            settings.and_then(|settings| settings.availability_type.name()),
        ),
        status: parse_status(instance.state.name().unwrap_or_default()),
        connectivity: connectivity(
            private,
            ip_configuration
                .and_then(|config| config.ipv4_enabled)
                .unwrap_or(false),
            instance
                .ip_addresses
                .iter()
                .any(|ip| ip.r#type.name() == Some("PRIMARY")),
        ),
    }
}

pub fn map_instance_detail(
    instance: &google_cloud_sql_v1::model::DatabaseInstance,
) -> CloudSqlInstanceDetail {
    let settings = instance.settings.as_ref();
    let ip_configuration = settings.and_then(|settings| settings.ip_configuration.as_ref());
    let address_of = |kind: &str| {
        instance
            .ip_addresses
            .iter()
            .find(|address| address.r#type.name() == Some(kind))
            .and_then(|address| non_empty(&address.ip_address))
    };
    let database_version = instance
        .database_version
        .name()
        .unwrap_or_default()
        .to_string();

    CloudSqlInstanceDetail {
        engine: normalize_engine(&database_version),
        database_version,
        name: instance.name.clone(),
        region: instance.region.clone(),
        tier: settings
            .map(|settings| settings.tier.clone())
            .unwrap_or_default(),
        availability: parse_availability(
            settings.and_then(|settings| settings.availability_type.name()),
        ),
        status: parse_status(instance.state.name().unwrap_or_default()),
        activation_policy: settings
            .and_then(|settings| settings.activation_policy.name())
            .map(str::to_string),
        creation_timestamp: crate::services::sdk::timestamp_to_string(
            instance.create_time.as_ref(),
        ),
        connection_name: non_empty(&instance.connection_name),
        public_ip: address_of("PRIMARY"),
        private_ip: address_of("PRIVATE"),
        require_ssl: ip_configuration
            .and_then(|config| config.require_ssl)
            .unwrap_or(false),
        private_network: ip_configuration.and_then(|config| non_empty(&config.private_network)),
        maintenance_window: settings
            .and_then(|settings| settings.maintenance_window.as_ref())
            .map(|window| CloudSqlMaintenanceWindow {
                day: window.day.and_then(|day| u32::try_from(day).ok()),
                hour: window.hour.and_then(|hour| u32::try_from(hour).ok()),
                update_track: window.update_track.name().map(str::to_string),
            }),
        maintenance_version: non_empty(&instance.maintenance_version),
        backup: settings
            .and_then(|settings| settings.backup_configuration.as_ref())
            .map(|backup| CloudSqlBackupConfig {
                enabled: backup.enabled.unwrap_or(false),
                start_time: non_empty(&backup.start_time),
                location: non_empty(&backup.location),
                point_in_time_recovery: backup.point_in_time_recovery_enabled.unwrap_or(false),
                transaction_log_retention_days: backup
                    .transaction_log_retention_days
                    .and_then(|days| u32::try_from(days).ok()),
            })
            .unwrap_or(CloudSqlBackupConfig {
                enabled: false,
                start_time: None,
                location: None,
                point_in_time_recovery: false,
                transaction_log_retention_days: None,
            }),
        replicas: instance.replica_names.clone(),
        master_instance: non_empty(&instance.master_instance_name),
        // Filled by the Users tab's own fetch, as on the CLI path.
        users: Vec::new(),
        labels: settings
            .map(|settings| {
                settings
                    .user_labels
                    .iter()
                    .map(|(key, value)| ComputeKeyValue {
                        key: key.clone(),
                        value: value.clone(),
                    })
                    .collect()
            })
            .unwrap_or_default(),
        raw_json: detail_json(instance),
    }
}
