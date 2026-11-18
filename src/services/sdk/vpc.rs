//! VPC networks, subnets and firewall rules through the native client.
//!
//! The domain enums' own `from_api` constructors do the interpreting, exactly
//! as they do on the CLI path, so a rule reads the same whichever backend
//! fetched it.

use google_cloud_auth::credentials::Credentials;
use google_cloud_compute_v1::client::{Firewalls, Networks, Subnetworks};
use nuvem_domain::gcp::resource::{
    FirewallAction, FirewallDirection, FirewallRule, FirewallTargets, VpcNetwork, VpcRoutingMode,
    VpcSubnet, VpcSubnetMode,
};

use crate::services::resources::list_filter::{RESOURCE_LIST_LIMIT, rows_are_capped};
use crate::services::resources::vpc::resource_id;
use crate::services::sdk::ApiError;

pub struct SdkVpcService {
    networks: Networks,
    subnetworks: Subnetworks,
    firewalls: Firewalls,
}

impl SdkVpcService {
    pub async fn connect(credentials: Credentials) -> Result<Self, ApiError> {
        let networks = Networks::builder()
            .with_credentials(credentials.clone())
            .build()
            .await
            .map_err(|error| ApiError::new(error.to_string()))?;
        let subnetworks = Subnetworks::builder()
            .with_credentials(credentials.clone())
            .build()
            .await
            .map_err(|error| ApiError::new(error.to_string()))?;
        let firewalls = Firewalls::builder()
            .with_credentials(credentials)
            .build()
            .await
            .map_err(|error| ApiError::new(error.to_string()))?;
        Ok(Self {
            networks,
            subnetworks,
            firewalls,
        })
    }

    /// The same service, built from already-constructed clients. See
    /// `SdkGkeService::from_clusters_for_test`.
    pub fn from_clients_for_test(
        networks: Networks,
        subnetworks: Subnetworks,
        firewalls: Firewalls,
    ) -> Self {
        Self {
            networks,
            subnetworks,
            firewalls,
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
        let (networks, subnetworks, firewalls) = crate::services::demo::sdk::vpc::clients();
        Self {
            networks,
            subnetworks,
            firewalls,
        }
    }
    pub async fn load_networks(&self, project: &str) -> Result<Vec<VpcNetwork>, ApiError> {
        use google_cloud_gax::paginator::ItemPaginator;
        let mut rows = Vec::new();
        let mut items = self
            .networks
            .list()
            .set_project(project)
            .set_max_results(RESOURCE_LIST_LIMIT as u32)
            .by_item();
        while let Some(network) = items.next().await {
            rows.push(map_network(&network?));
            if rows_are_capped(rows.len(), RESOURCE_LIST_LIMIT) {
                break;
            }
        }
        Ok(rows)
    }

    /// Subnets across every region, which is what the CLI's unqualified
    /// `compute networks subnets list` returns.
    pub async fn load_subnets(&self, project: &str) -> Result<Vec<VpcSubnet>, ApiError> {
        use google_cloud_gax::paginator::ItemPaginator;
        let mut rows = Vec::new();
        let mut items = self
            .subnetworks
            .aggregated_list()
            .set_project(project)
            .set_max_results(RESOURCE_LIST_LIMIT as u32)
            .by_item();
        while let Some(entry) = items.next().await {
            let (_scope, scoped) = entry?;
            for subnet in &scoped.subnetworks {
                rows.push(map_subnet(subnet));
            }
            if rows_are_capped(rows.len(), RESOURCE_LIST_LIMIT) {
                rows.truncate(RESOURCE_LIST_LIMIT);
                break;
            }
        }
        Ok(rows)
    }

    pub async fn load_firewall_rules(&self, project: &str) -> Result<Vec<FirewallRule>, ApiError> {
        use google_cloud_gax::paginator::ItemPaginator;
        let mut rows = Vec::new();
        let mut items = self
            .firewalls
            .list()
            .set_project(project)
            .set_max_results(RESOURCE_LIST_LIMIT as u32)
            .by_item();
        while let Some(rule) = items.next().await {
            rows.push(map_firewall_rule(&rule?));
            if rows_are_capped(rows.len(), RESOURCE_LIST_LIMIT) {
                break;
            }
        }
        Ok(rows)
    }
}

pub fn map_network(network: &google_cloud_compute_v1::model::Network) -> VpcNetwork {
    VpcNetwork {
        name: network.name.clone().unwrap_or_default(),
        subnet_mode: VpcSubnetMode::from_api(network.auto_create_subnetworks),
        routing_mode: VpcRoutingMode::from_api(
            network
                .routing_config
                .as_ref()
                .and_then(|config| config.routing_mode.as_ref())
                .and_then(|mode| mode.name()),
        ),
        // The count, not the array, as on the CLI path.
        subnet_count: network.subnetworks.len(),
        description: network.description.clone().filter(|text| !text.is_empty()),
    }
}

pub fn map_subnet(subnet: &google_cloud_compute_v1::model::Subnetwork) -> VpcSubnet {
    VpcSubnet {
        name: subnet.name.clone().unwrap_or_default(),
        region: resource_id(subnet.region.as_deref().unwrap_or_default()),
        network: resource_id(subnet.network.as_deref().unwrap_or_default()),
        ip_cidr_range: subnet.ip_cidr_range.clone().unwrap_or_default(),
        gateway_address: subnet
            .gateway_address
            .clone()
            .filter(|address| !address.is_empty()),
        private_google_access: subnet.private_ip_google_access.unwrap_or(false),
    }
}

pub fn map_firewall_rule(rule: &google_cloud_compute_v1::model::Firewall) -> FirewallRule {
    let direction =
        FirewallDirection::from_api(rule.direction.as_ref().and_then(|value| value.name()));
    // The rule carries the list matching its direction and not the other; an
    // unrecognised direction falls back to whichever list is populated.
    let source_or_dest = match direction {
        FirewallDirection::Egress => rule.destination_ranges.clone(),
        FirewallDirection::Ingress => rule.source_ranges.clone(),
        _ if !rule.source_ranges.is_empty() => rule.source_ranges.clone(),
        _ => rule.destination_ranges.clone(),
    };

    FirewallRule {
        name: rule.name.clone().unwrap_or_default(),
        network: resource_id(rule.network.as_deref().unwrap_or_default()),
        direction,
        priority: rule.priority.and_then(|value| u32::try_from(value).ok()),
        // Presence, not content — and here presence is a non-empty list rather
        // than a present key, because the API models both as repeated fields.
        action: FirewallAction::from_api(!rule.allowed.is_empty(), !rule.denied.is_empty()),
        targets: FirewallTargets::from_api(
            rule.target_tags.clone(),
            rule.target_service_accounts.clone(),
        ),
        source_or_dest,
        disabled: rule.disabled.unwrap_or(false),
    }
}
