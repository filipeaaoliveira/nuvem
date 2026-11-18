//! Compute Engine through the native client.
//!
//! `parse_status` and `tail_segment` are the CLI module's, on purpose: a row
//! must read the same whichever backend filled it, and a second copy of the
//! status vocabulary is how the two would drift apart.

use google_cloud_auth::credentials::Credentials;
use google_cloud_compute_v1::client::Instances;
use google_cloud_lro::Poller;
use nuvem_domain::gcp::resource::{
    ComputeDisk, ComputeInstance, ComputeInstanceDetail, ComputeKeyValue, ComputeNetworkInterface,
};

use crate::app::state::resources::ComputeInstanceAction;
use crate::services::resources::compute::{parse_status, segment_after, tail_segment};
use crate::services::resources::list_filter::{RESOURCE_LIST_LIMIT, rows_are_capped};
use crate::services::sdk::ApiError;

pub struct SdkComputeService {
    instances: Instances,
}

impl SdkComputeService {
    pub async fn connect(credentials: Credentials) -> Result<Self, ApiError> {
        let instances = Instances::builder()
            .with_credentials(credentials)
            .build()
            .await
            .map_err(|error| ApiError::new(error.to_string()))?;
        Ok(Self { instances })
    }

    /// The same service, built from an already-constructed client. See
    /// `SdkGkeService::from_clusters_for_test`.
    pub fn from_instances_for_test(instances: Instances) -> Self {
        Self { instances }
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
            instances: crate::services::demo::sdk::compute::client(),
        }
    }

    /// Every instance in the project, across zones, capped like every other
    /// resource list.
    ///
    /// `aggregated_list` is the API's own equivalent of what `gcloud compute
    /// instances list` does: the CLI fans out over zones and merges, and this
    /// asks the service to do it instead.
    ///
    /// No filter is pushed down, deliberately. `pushdown_expression` emits
    /// gcloud's filter language — `name ~ "(?i)term" OR status ~ ...` — and the
    /// Compute API does not speak it. Measured against the live API: the `~`
    /// form is rejected outright ("Invalid list filter expression"), and the
    /// nearest API form that does parse, `field eq <regex>`, silently returns
    /// *zero* rows when two fields are joined with `OR` rather than the union.
    /// A silently empty listing is worse than an unoptimised one, so the
    /// narrowing stays where it already ran: on the client.
    ///
    /// The visible consequence is confined to a project holding more than
    /// [`RESOURCE_LIST_LIMIT`] instances, where the cap now applies before the
    /// filter rather than after it, so a match beyond the cap is not fetched.
    /// The CLI path had the same cap and pushed the filter through it.
    pub async fn load_instances(&self, project: &str) -> Result<Vec<ComputeInstance>, ApiError> {
        use google_cloud_gax::paginator::ItemPaginator;

        let mut rows = Vec::new();
        let mut pages = self
            .instances
            .aggregated_list()
            .set_project(project)
            .set_max_results(RESOURCE_LIST_LIMIT as u32)
            // The two flags `gcloud compute instances list` sends alongside the
            // cap, verified against its own `--log-http` output. Without
            // `return_partial_success` one unreachable zone fails the whole
            // aggregated listing, where the CLI backend returned every zone
            // that answered; without `include_all_scopes` the response is
            // confined to the scopes the API decides to include by default.
            .set_include_all_scopes(true)
            .set_return_partial_success(true)
            .by_item();
        while let Some(entry) = pages.next().await {
            let (_scope, scoped) = entry?;
            for instance in &scoped.instances {
                rows.push(map_instance(instance));
            }
            if rows_are_capped(rows.len(), RESOURCE_LIST_LIMIT) {
                rows.truncate(RESOURCE_LIST_LIMIT);
                break;
            }
        }
        Ok(rows)
    }
    /// Runs one lifecycle action against one instance.
    ///
    /// # What returning `Ok` means
    ///
    /// The operation finished. Sending the request only gets it accepted, and
    /// the work continues in GCP afterwards; the operations log has one word
    /// for the end of a lifecycle action and it has always meant finished, so
    /// this waits rather than teaching that word a second meaning that
    /// depends on which backend answered.
    ///
    /// The wait is the generated poller, which carries the client's own
    /// backoff and polling-error policy -- the same machinery every other
    /// long-running call in these crates uses, rather than a loop written
    /// here.
    pub async fn run_action(
        &self,
        project: &str,
        zone: &str,
        instance: &str,
        action: ComputeInstanceAction,
    ) -> Result<(), ApiError> {
        // One arm per verb rather than a shared builder: the generated
        // builders are five distinct types, and a method that tried to unify
        // them would be a match on the action either way.
        let operation = match action {
            ComputeInstanceAction::Start => {
                self.instances
                    .start()
                    .set_project(project)
                    .set_zone(zone)
                    .set_instance(instance)
                    .poller()
                    .until_done()
                    .await?
            }
            ComputeInstanceAction::Stop => {
                self.instances
                    .stop()
                    .set_project(project)
                    .set_zone(zone)
                    .set_instance(instance)
                    .poller()
                    .until_done()
                    .await?
            }
            ComputeInstanceAction::Reset => {
                self.instances
                    .reset()
                    .set_project(project)
                    .set_zone(zone)
                    .set_instance(instance)
                    .poller()
                    .until_done()
                    .await?
            }
            ComputeInstanceAction::Suspend => {
                self.instances
                    .suspend()
                    .set_project(project)
                    .set_zone(zone)
                    .set_instance(instance)
                    .poller()
                    .until_done()
                    .await?
            }
            ComputeInstanceAction::Resume => {
                self.instances
                    .resume()
                    .set_project(project)
                    .set_zone(zone)
                    .set_instance(instance)
                    .poller()
                    .until_done()
                    .await?
            }
        };
        finished_or_error(operation)
    }

    /// One instance described in full, for the detail pane.
    pub async fn load_instance_detail(
        &self,
        project: &str,
        zone: &str,
        instance: &str,
    ) -> Result<ComputeInstanceDetail, ApiError> {
        let instance = self
            .instances
            .get()
            .set_project(project)
            .set_zone(zone)
            .set_instance(instance)
            .send()
            .await?;
        Ok(map_instance_detail(&instance))
    }
}

/// A completed operation, or the error it completed with.
///
/// The poller answers `Ok` for any operation that reaches `DONE`, including
/// one that got there by being refused: Compute reports that in the
/// operation's own fields rather than as a transport error. Left unchecked,
/// an instance that was already stopping would settle the operations log at
/// `Succeeded` for work GCP declined to do.
fn finished_or_error(operation: google_cloud_compute_v1::model::Operation) -> Result<(), ApiError> {
    if let Some(error) = &operation.error {
        let detail = error
            .errors
            .iter()
            .filter_map(|entry| entry.message.clone())
            .collect::<Vec<_>>()
            .join("; ");
        return Err(ApiError::new(if detail.is_empty() {
            "the operation failed".to_string()
        } else {
            detail
        }));
    }
    match (
        operation.http_error_status_code,
        &operation.http_error_message,
    ) {
        (Some(code), Some(message)) if code >= 400 => {
            Err(ApiError::new(format!("{code}: {message}")))
        }
        (Some(code), None) if code >= 400 => Err(ApiError::new(format!(
            "the operation failed with status {code}"
        ))),
        _ => Ok(()),
    }
}

pub fn map_instance(instance: &google_cloud_compute_v1::model::Instance) -> ComputeInstance {
    let first_network = instance.network_interfaces.first();
    ComputeInstance {
        // The row's id is a string everywhere else in this application; the
        // API sends the same number the CLI rendered as a decimal string.
        id: instance.id.map(|id| id.to_string()).unwrap_or_default(),
        name: instance.name.clone().unwrap_or_default(),
        zone: tail_segment(instance.zone.as_deref().unwrap_or_default()),
        status: parse_status(
            instance
                .status
                .as_ref()
                .and_then(|status| status.name())
                .unwrap_or_default(),
        ),
        machine_type: tail_segment(instance.machine_type.as_deref().unwrap_or_default()),
        internal_ip: first_network.and_then(|network| network.network_ip.clone()),
        external_ip: first_network
            .and_then(|network| network.access_configs.first())
            .and_then(|access| access.nat_ip.clone()),
    }
}

pub fn map_instance_detail(
    instance: &google_cloud_compute_v1::model::Instance,
) -> ComputeInstanceDetail {
    ComputeInstanceDetail {
        name: instance.name.clone().unwrap_or_default(),
        zone: instance
            .zone
            .as_deref()
            .map(tail_segment)
            .unwrap_or_default(),
        status: parse_status(
            instance
                .status
                .as_ref()
                .and_then(|status| status.name())
                .unwrap_or_default(),
        ),
        machine_type: instance
            .machine_type
            .as_deref()
            .map(tail_segment)
            .unwrap_or_default(),
        creation_timestamp: instance.creation_timestamp.clone(),
        network_interfaces: instance
            .network_interfaces
            .iter()
            .map(|network| ComputeNetworkInterface {
                name: network.name.clone(),
                network: network.network.as_deref().map(tail_segment),
                subnetwork: network.subnetwork.as_deref().map(tail_segment),
                subnetwork_region: network
                    .subnetwork
                    .as_deref()
                    .and_then(|value| segment_after(value, "regions")),
                internal_ip: network.network_ip.clone(),
                external_ip: network
                    .access_configs
                    .first()
                    .and_then(|access| access.nat_ip.clone()),
            })
            .collect(),
        // Absent and present-but-empty both mean the instance carries none,
        // matching what the CLI path makes them mean.
        tags: instance
            .tags
            .as_ref()
            .map(|tags| tags.items.clone())
            .unwrap_or_default(),
        // Emails only, empties dropped: an entry with no email identifies
        // nothing, and carrying it would let a firewall selector that renders
        // empty appear to match.
        service_accounts: instance
            .service_accounts
            .iter()
            .filter_map(|account| account.email.clone())
            .filter(|email| !email.is_empty())
            .collect(),
        disks: instance
            .disks
            .iter()
            .map(|disk| ComputeDisk {
                name: disk.device_name.clone(),
                source: disk.source.as_deref().map(tail_segment),
                boot: disk.boot.unwrap_or(false),
                auto_delete: disk.auto_delete.unwrap_or(false),
                disk_type: disk
                    .r#type
                    .as_ref()
                    .and_then(|kind| kind.name())
                    .map(str::to_string),
                size_gb: disk.disk_size_gb.map(|size| size.to_string()),
            })
            .collect(),
        labels: instance
            .labels
            .iter()
            .map(|(key, value)| ComputeKeyValue {
                key: key.clone(),
                value: value.clone(),
            })
            .collect(),
        metadata: instance
            .metadata
            .as_ref()
            .map(|metadata| {
                metadata
                    .items
                    .iter()
                    .map(|item| ComputeKeyValue {
                        key: item.key.clone().unwrap_or_default(),
                        value: item.value.clone().unwrap_or_default(),
                    })
                    .collect()
            })
            .unwrap_or_default(),
        raw_json: crate::services::sdk::detail_json(instance),
    }
}
