//! VPC's demo backend: networks, subnets and firewall rules, all three from
//! the captures.
//!
//! Networks and firewall rules are envelope-only. `vpc_networks.json` and
//! `vpc_firewall_rules.json` are the same captures the CLI parser reads, and
//! the elements in them are already the proto3 JSON the generated `Network`
//! and `Firewall` models deserialize -- the only difference between the two
//! wire shapes is the `NetworkList`/`FirewallList` message that carries the
//! array in its `items` field, which [`envelope`] supplies at
//! runtime. No second copy of either capture exists.
//!
//! # Subnets are the shape the envelope does not cover
//!
//! `subnetworks.aggregatedList` does not answer an array. It answers a map
//! keyed by `regions/<region>` whose values are per-scope objects, and
//! `vpc_subnets.json` is the flattened listing gcloud printed from it, so
//! there is no field name that turns one into the other.
//! [`by_scope`] rebuilds the grouping at call time instead,
//! reading each row's own `region` self-link with `resource_id` -- the
//! function `map_subnet` fills the REGION column with. Still one copy of the
//! capture, and no file records a grouping that could drift from the rows it
//! groups. The capture names four regions, so this answers four scopes.
//!
//! # What the one new method reaches
//!
//! Only `aggregated_list`. The generated `subnetworks::AggregatedList` builder
//! has `send`, `by_page` and `by_item` and no poller -- unlike its sibling
//! `Delete`, whose `poller()` reads operations back through the same stub --
//! so `SdkVpcService::load_subnets` reaches this method and nothing else.
//! `next_page_token` is left unset, which stops the paginator after one page.

use google_cloud_compute_v1::client::{Firewalls, Networks, Subnetworks};
use google_cloud_compute_v1::model;
use google_cloud_gax::error::Error;

use crate::services::resources::vpc::resource_id;

use super::{aggregated_project, by_scope, envelope, scope};

/// The networks capture: `gcloud compute networks list --format=json`.
const NETWORKS: &str = include_str!("../../../../tests/fixtures/vpc_networks.json");

/// The firewall rules capture: `gcloud compute firewall-rules list --format=json`.
const FIREWALL_RULES: &str = include_str!("../../../../tests/fixtures/vpc_firewall_rules.json");

/// Answers `list` from the networks capture.
///
/// One page: `next_page_token` is left unset, so the item paginator reads this
/// as the only page and stops rather than asking for a second one. Every other
/// method of the trait stays defaulted -- `SdkVpcService` makes this one call
/// on this client, and the generated `networks::List` builder has no poller,
/// so nothing calls on its behalf either.
#[derive(Debug, Default)]
pub struct NetworksStub;

impl google_cloud_compute_v1::stub::Networks for NetworksStub {
    async fn list(
        &self,
        request: model::networks::ListRequest,
        _options: google_cloud_gax::options::RequestOptions,
    ) -> google_cloud_gax::Result<google_cloud_gax::response::Response<model::NetworkList>> {
        aggregated_project(&request.project)?;
        Ok(google_cloud_gax::response::Response::from(envelope(
            "items", NETWORKS,
        )?))
    }
}

/// The subnets capture: `gcloud compute networks subnets list --format=json`.
const SUBNETS: &str = include_str!("../../../../tests/fixtures/vpc_subnets.json");

/// The capture regrouped into the region-keyed map the RPC answers.
fn aggregated() -> Result<model::SubnetworkAggregatedList, Error> {
    let scopes = by_scope(SUBNETS, |subnet: &model::Subnetwork| {
        scope(None, subnet.region.as_deref().map(resource_id))
    })?;
    Ok(
        model::SubnetworkAggregatedList::new().set_items(scopes.into_iter().map(
            |(scope, subnetworks)| {
                (
                    scope,
                    model::SubnetworksScopedList::new().set_subnetworks(subnetworks),
                )
            },
        )),
    )
}

/// Answers `aggregated_list` from the subnets capture, which is what the CLI's
/// unqualified subnets listing returns.
#[derive(Debug, Default)]
pub struct SubnetworksStub;

impl google_cloud_compute_v1::stub::Subnetworks for SubnetworksStub {
    /// The project is read; the two scope flags are not, because
    /// `SdkVpcService::load_subnets` does not send them. That is a genuine
    /// difference from the compute and disks callers, which do, and checking
    /// for flags this caller never sets would pin the difference here rather
    /// than guard anything.
    async fn aggregated_list(
        &self,
        request: model::subnetworks::AggregatedListRequest,
        _options: google_cloud_gax::options::RequestOptions,
    ) -> google_cloud_gax::Result<
        google_cloud_gax::response::Response<model::SubnetworkAggregatedList>,
    > {
        aggregated_project(&request.project)?;
        Ok(google_cloud_gax::response::Response::from(aggregated()?))
    }
}

/// Answers `list` from the firewall rules capture.
#[derive(Debug, Default)]
pub struct FirewallsStub;

impl google_cloud_compute_v1::stub::Firewalls for FirewallsStub {
    async fn list(
        &self,
        request: model::firewalls::ListRequest,
        _options: google_cloud_gax::options::RequestOptions,
    ) -> google_cloud_gax::Result<google_cloud_gax::response::Response<model::FirewallList>> {
        aggregated_project(&request.project)?;
        Ok(google_cloud_gax::response::Response::from(envelope(
            "items",
            FIREWALL_RULES,
        )?))
    }
}

/// The three clients `SdkVpcService::from_fixtures` is built from.
pub fn clients() -> (Networks, Subnetworks, Firewalls) {
    (
        Networks::from_stub(NetworksStub),
        Subnetworks::from_stub(SubnetworksStub),
        Firewalls::from_stub(FirewallsStub),
    )
}
