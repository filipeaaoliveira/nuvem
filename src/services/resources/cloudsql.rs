use std::collections::BTreeMap;

use serde::Deserialize;
use tokio_util::sync::CancellationToken;

use nuvem_domain::gcp::configuration::GcloudContext;
use nuvem_domain::gcp::filter::label_filter_field;
use nuvem_domain::gcp::resource::{
    CloudSqlAvailability, CloudSqlBackupConfig, CloudSqlInstance, CloudSqlInstanceDetail,
    CloudSqlMaintenanceWindow, CloudSqlStatus, CloudSqlUser, ComputeKeyValue, EndpointVisibility,
    ResourceServiceKind,
};

use crate::app::state::resources::{CloudSqlInstanceAction, cloud_sql_lifecycle_argv};
use crate::services::gcloud::cli::{CommandCeiling, CommandError, CommandRunner, GcloudCommand};
use crate::services::resources::list_filter::{RESOURCE_LIST_LIMIT, pushdown_expression};

/// Verbatim-superset raw fields eligible for the server-side filter pushdown
/// in [`GcloudCloudSqlService::load_instances`]. See
/// `crate::services::resources::list_filter` for the never-drop-a-row
/// reasoning.
const FILTER_FIELDS: &[&str] = &[
    "name",
    "region",
    "databaseVersion",
    "state",
    "settings.tier",
];

/// Opaque display labels for [`CloudSqlStatus`], [`CloudSqlAvailability`],
/// the normalized engine name (`normalize_engine`), and [`EndpointVisibility`]
/// (all derived, not verbatim in the raw JSON pushed above) — a term that is
/// a substring of one of these is gated out of the pushdown so it stays
/// client-side-only.
const OPAQUE_LABELS: &[&str] = &[
    "runnable",
    "stopped",
    "pending create",
    "maintenance",
    "failed",
    "suspended",
    "unknown",
    "zonal",
    "ha (regional)",
    "postgresql",
    "mysql",
    "sql server",
    "public",
    "private",
];

#[derive(Debug, thiserror::Error)]
pub enum CloudSqlParseError {
    #[error("invalid Cloud SQL JSON: {0}")]
    Json(#[from] serde_json::Error),
}

#[derive(Debug, thiserror::Error)]
pub enum CloudSqlLoadError {
    #[error("{0}")]
    Command(#[from] CommandError),
    #[error("{0}")]
    Parse(#[from] CloudSqlParseError),
}

pub struct GcloudCloudSqlService<R> {
    runner: R,
}

impl<R> GcloudCloudSqlService<R> {
    pub fn new(runner: R) -> Self {
        Self { runner }
    }

    pub fn runner(&self) -> &R {
        &self.runner
    }
}

impl<R: CommandRunner> GcloudCloudSqlService<R> {
    /// Lists Cloud SQL instances, capped at [`RESOURCE_LIST_LIMIT`] rows and,
    /// when `filter` is server-expressible, narrowed with a pushed-down
    /// `--filter` expression. The caller still re-applies the exact
    /// client-side filter to the (superset) result.
    pub fn load_instances(
        &self,
        context: &GcloudContext,
        filter: Option<&str>,
    ) -> Result<Vec<CloudSqlInstance>, CloudSqlLoadError> {
        let mut args: Vec<String> = vec![
            "--configuration".to_string(),
            context.configuration.as_str().to_string(),
            "sql".to_string(),
            "instances".to_string(),
            "list".to_string(),
            "--format=json".to_string(),
            format!("--limit={RESOURCE_LIST_LIMIT}"),
        ];
        if let Some(expression) = filter.and_then(|applied| {
            pushdown_expression(
                applied,
                FILTER_FIELDS,
                OPAQUE_LABELS,
                label_filter_field(ResourceServiceKind::CloudSql),
            )
        }) {
            args.push(format!("--filter={expression}"));
        }
        let output = self.runner.run(GcloudCommand::new(args))?;
        Ok(parse_instances_json(&output.stdout)?)
    }

    pub fn load_instance_detail(
        &self,
        context: &GcloudContext,
        instance: &CloudSqlInstance,
    ) -> Result<CloudSqlInstanceDetail, CloudSqlLoadError> {
        let output = self.runner.run(GcloudCommand::new([
            "--configuration",
            context.configuration.as_str(),
            "sql",
            "instances",
            "describe",
            instance.name.as_str(),
            "--format=json",
        ]))?;
        Ok(parse_instance_detail_json(&output.stdout)?)
    }

    pub fn load_users(
        &self,
        context: &GcloudContext,
        instance_name: &str,
    ) -> Result<Vec<CloudSqlUser>, CloudSqlLoadError> {
        let output = self.runner.run(GcloudCommand::new([
            "--configuration",
            context.configuration.as_str(),
            "sql",
            "users",
            "list",
            "--instance",
            instance_name,
            "--format=json",
        ]))?;
        Ok(parse_users_json(&output.stdout)?)
    }

    pub fn run_instance_action(
        &self,
        ctx: &GcloudContext,
        instance: &CloudSqlInstance,
        action: CloudSqlInstanceAction,
    ) -> Result<(), CloudSqlLoadError> {
        self.run_action_by_id(ctx, instance.name.as_str(), action)
    }

    /// Runs a lifecycle action against an instance identified by name alone.
    /// Cloud SQL instance IDs are unique per project, so the batch path — which
    /// holds projected targets rather than full rows — needs nothing else.
    /// This carries the argv; [`Self::run_instance_action`] delegates here so
    /// the single-row and batch paths cannot execute different commands.
    ///
    /// Delegates to [`Self::run_action_by_id_with_cancellation`] with a token
    /// nobody holds, rather than carrying a second copy of the argv: a runner
    /// handed a fresh token behaves exactly as `run` does, and one builder
    /// means the cancellable and uncancellable forms cannot drift either.
    pub fn run_action_by_id(
        &self,
        ctx: &GcloudContext,
        name: &str,
        action: CloudSqlInstanceAction,
    ) -> Result<(), CloudSqlLoadError> {
        self.run_action_by_id_with_cancellation(ctx, name, action, &CancellationToken::new())
    }

    /// The same action, abandonable by whoever holds `cancellation`.
    ///
    /// The runtime hands this the session's shutdown token so an action still
    /// in flight when the operator presses `q` does not hold `Runtime::drop`
    /// for the thirty minutes its ceiling allows.
    ///
    /// Cancelling does not cancel the operation. gcloud is killed and nuvem
    /// stops waiting; a failover GCP has already accepted proceeds regardless,
    /// which is what [`CommandError::Cancelled`]'s lifecycle caveat says.
    pub fn run_action_by_id_with_cancellation(
        &self,
        ctx: &GcloudContext,
        name: &str,
        action: CloudSqlInstanceAction,
        cancellation: &CancellationToken,
    ) -> Result<(), CloudSqlLoadError> {
        // The lifecycle ceiling, not the read one: none of these argvs carries
        // `--async`, so gcloud blocks until GCP finishes the operation, and a
        // failover or an activation-policy patch can run well past the two
        // minutes a listing is given. The cancellable call keeps that
        // ceiling — `run_with_cancellation` would have dropped it.
        self.runner.run_bounded_with_cancellation(
            GcloudCommand::new(cloud_sql_lifecycle_argv(ctx, name, action))
                .with_ceiling(CommandCeiling::Lifecycle),
            cancellation,
        )?;
        Ok(())
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawInstance {
    name: String,
    database_version: Option<String>,
    region: Option<String>,
    state: Option<String>,
    #[serde(default)]
    settings: RawSettings,
    #[serde(default)]
    ip_addresses: Vec<RawIpAddress>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawSettings {
    tier: Option<String>,
    availability_type: Option<String>,
    activation_policy: Option<String>,
    ip_configuration: Option<RawIpConfig>,
    #[serde(default)]
    user_labels: BTreeMap<String, String>,
    backup_configuration: Option<RawBackupConfig>,
    maintenance_window: Option<RawMaintenanceWindow>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawIpConfig {
    #[serde(default)]
    ipv4_enabled: bool,
    private_network: Option<String>,
    #[serde(default)]
    require_ssl: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawIpAddress {
    #[serde(rename = "type")]
    kind: Option<String>,
    ip_address: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawInstanceDetail {
    name: String,
    database_version: Option<String>,
    region: Option<String>,
    state: Option<String>,
    connection_name: Option<String>,
    create_time: Option<String>,
    maintenance_version: Option<String>,
    master_instance_name: Option<String>,
    #[serde(default)]
    replica_names: Vec<String>,
    #[serde(default)]
    ip_addresses: Vec<RawIpAddress>,
    #[serde(default)]
    settings: RawSettings,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawBackupConfig {
    #[serde(default)]
    enabled: bool,
    start_time: Option<String>,
    location: Option<String>,
    #[serde(default)]
    point_in_time_recovery_enabled: bool,
    transaction_log_retention_days: Option<u32>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawMaintenanceWindow {
    day: Option<u32>,
    hour: Option<u32>,
    update_track: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawUser {
    name: String,
    host: Option<String>,
    #[serde(rename = "type")]
    kind: Option<String>,
}

pub fn parse_instances_json(json: &str) -> Result<Vec<CloudSqlInstance>, CloudSqlParseError> {
    let raw: Vec<RawInstance> = serde_json::from_str(json)?;
    Ok(raw
        .into_iter()
        .map(|item| {
            let db = item.database_version.unwrap_or_default();
            let private = item
                .settings
                .ip_configuration
                .as_ref()
                .and_then(|c| c.private_network.clone())
                .is_some();
            CloudSqlInstance {
                engine: normalize_engine(&db),
                database_version: db,
                name: item.name,
                region: item.region.unwrap_or_default(),
                tier: item.settings.tier.unwrap_or_default(),
                availability: parse_availability(item.settings.availability_type.as_deref()),
                status: parse_status(item.state.as_deref().unwrap_or("")),
                connectivity: connectivity(
                    private,
                    item.settings
                        .ip_configuration
                        .as_ref()
                        .map(|c| c.ipv4_enabled)
                        .unwrap_or(false),
                    item.ip_addresses
                        .iter()
                        .any(|ip| ip.kind.as_deref() == Some("PRIMARY")),
                ),
            }
        })
        .collect())
}

const SECRET_KEYS: &[&str] = &[
    "cert",
    "certSerialNumber",
    "password",
    "rootPassword",
    "sslCert",
    "caCert",
    "privateKey",
    "clientCertificate",
    "clientKey",
    "caCertificate",
];

/// Recursively walks a JSON value, replacing the values of known secret keys
/// and any string containing PEM markers with `"<redacted>"`.
pub fn redact_secrets(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Object(map) => {
            for (k, v) in map.iter_mut() {
                if SECRET_KEYS.iter().any(|s| s.eq_ignore_ascii_case(k)) {
                    *v = serde_json::Value::String("<redacted>".into());
                } else {
                    redact_secrets(v);
                }
            }
        }
        serde_json::Value::Array(items) => items.iter_mut().for_each(redact_secrets),
        serde_json::Value::String(s)
            if s.contains("BEGIN CERTIFICATE") || s.contains("PRIVATE KEY") =>
        {
            *value = serde_json::Value::String("<redacted>".into());
        }
        _ => {}
    }
}

pub fn parse_instance_detail_json(
    json: &str,
) -> Result<CloudSqlInstanceDetail, CloudSqlParseError> {
    let raw: RawInstanceDetail = serde_json::from_str(json)?;
    let mut formatted: serde_json::Value = serde_json::from_str(json)?;
    redact_secrets(&mut formatted);

    let db = raw.database_version.unwrap_or_default();
    let private = raw
        .settings
        .ip_configuration
        .as_ref()
        .and_then(|c| c.private_network.clone());
    let public_ip = raw
        .ip_addresses
        .iter()
        .find(|ip| ip.kind.as_deref() == Some("PRIMARY"))
        .and_then(|ip| ip.ip_address.clone());
    let private_ip = raw
        .ip_addresses
        .iter()
        .find(|ip| ip.kind.as_deref() == Some("PRIVATE"))
        .and_then(|ip| ip.ip_address.clone());
    let require_ssl = raw
        .settings
        .ip_configuration
        .as_ref()
        .map(|c| c.require_ssl)
        .unwrap_or(false);
    let labels = raw
        .settings
        .user_labels
        .into_iter()
        .map(|(key, value)| ComputeKeyValue { key, value })
        .collect();
    let backup = raw
        .settings
        .backup_configuration
        .map(|b| CloudSqlBackupConfig {
            enabled: b.enabled,
            start_time: b.start_time,
            location: b.location,
            point_in_time_recovery: b.point_in_time_recovery_enabled,
            transaction_log_retention_days: b.transaction_log_retention_days,
        })
        .unwrap_or(CloudSqlBackupConfig {
            enabled: false,
            start_time: None,
            location: None,
            point_in_time_recovery: false,
            transaction_log_retention_days: None,
        });
    let maintenance_window = raw
        .settings
        .maintenance_window
        .map(|w| CloudSqlMaintenanceWindow {
            day: w.day,
            hour: w.hour,
            update_track: w.update_track,
        });

    Ok(CloudSqlInstanceDetail {
        engine: normalize_engine(&db),
        database_version: db,
        name: raw.name,
        region: raw.region.unwrap_or_default(),
        tier: raw.settings.tier.unwrap_or_default(),
        availability: parse_availability(raw.settings.availability_type.as_deref()),
        status: parse_status(raw.state.as_deref().unwrap_or("")),
        activation_policy: raw.settings.activation_policy,
        creation_timestamp: raw.create_time,
        connection_name: raw.connection_name,
        public_ip,
        private_ip,
        require_ssl,
        private_network: private,
        maintenance_window,
        maintenance_version: raw.maintenance_version,
        backup,
        replicas: raw.replica_names,
        master_instance: raw.master_instance_name,
        users: Vec::new(),
        labels,
        raw_json: serde_json::to_string_pretty(&formatted)?,
    })
}

pub fn parse_users_json(json: &str) -> Result<Vec<CloudSqlUser>, CloudSqlParseError> {
    let raw: Vec<RawUser> = serde_json::from_str(json)?;
    Ok(raw
        .into_iter()
        .map(|item| CloudSqlUser {
            name: item.name,
            host: item.host,
            kind: item.kind,
        })
        .collect())
}

pub fn normalize_engine(database_version: &str) -> String {
    let up = database_version.to_uppercase();
    if up.starts_with("POSTGRES") {
        "PostgreSQL"
    } else if up.starts_with("MYSQL") {
        "MySQL"
    } else if up.starts_with("SQLSERVER") {
        "SQL Server"
    } else {
        "Unknown"
    }
    .to_string()
}

pub fn parse_status(value: &str) -> CloudSqlStatus {
    match value {
        "RUNNABLE" => CloudSqlStatus::Runnable,
        "STOPPED" => CloudSqlStatus::Stopped,
        "PENDING_CREATE" => CloudSqlStatus::PendingCreate,
        "MAINTENANCE" => CloudSqlStatus::Maintenance,
        "FAILED" | "UNKNOWN_STATE" => CloudSqlStatus::Failed,
        "SUSPENDED" => CloudSqlStatus::Suspended,
        other => CloudSqlStatus::Unknown(other.to_string()),
    }
}

pub fn parse_availability(value: Option<&str>) -> CloudSqlAvailability {
    match value {
        Some("REGIONAL") => CloudSqlAvailability::Regional,
        Some("ZONAL") => CloudSqlAvailability::Zonal,
        _ => CloudSqlAvailability::Unknown,
    }
}

/// Takes `has_primary_ip` rather than the address list so both backends can
/// share the decision: the CLI reads the addresses out of JSON and the native
/// client reads typed `IpMapping`s, and the rule about what makes an instance
/// publicly reachable must not exist twice.
pub fn connectivity(private: bool, ipv4: bool, has_primary_ip: bool) -> EndpointVisibility {
    if private {
        EndpointVisibility::Private
    } else if ipv4 || has_primary_ip {
        EndpointVisibility::Public
    } else {
        EndpointVisibility::Unknown
    }
}
