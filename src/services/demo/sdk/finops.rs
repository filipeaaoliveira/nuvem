//! FinOps recommendations' demo backend, keyed on the pair the request names.
//!
//! Each fixture is a plain envelope -- the captured array goes into
//! `ListRecommendationsResponse.recommendations` -- but *which* fixture is not
//! a property of the recommender alone, and that is the whole difficulty here.
//!
//! # Why the location is half the key
//!
//! `recommendations.list` takes one `(recommender, location)` pair per call;
//! there is no list-all. `FinOpsState::recommendations` then flattens a
//! recommender's cells across every location it asked about, so a stub that
//! served the same rows for every location would list one recommendation once
//! per zone and the savings total would count it that many times -- a cost
//! screen reporting money that does not exist, which is the one direction it
//! must never be wrong in. `recommendations_fixture` in `demo/mod.rs` keys the
//! CLI path on the same pair, for the same reason, and this stub reads the
//! pair back out of the request's `parent` so the two backends answer alike.
//!
//! Every pair that is not captured answers `[]`, which is what the real API
//! does and a state the cards distinguish: `CellState::Empty` is "asked,
//! nothing to save" rather than "not asked".
//!
//! The captured locations are the ones `query_locations` derives from the demo
//! compute and disk fixtures: `europe-west1-b` for the three zonal
//! recommenders, and its region `europe-west1` for the address recommender,
//! which is regional.

use google_cloud_recommender_v1::client::Recommender;
use google_cloud_recommender_v1::model;

use super::envelope;

/// The answer for a pair nothing was captured for.
const EMPTY: &str = "[]";

const IDLE_VMS: &str = include_str!("../../../../tests/fixtures/finops_idle_vms.json");
const MACHINE_TYPE: &str = include_str!("../../../../tests/fixtures/finops_machine_type.json");
const IDLE_DISKS: &str = include_str!("../../../../tests/fixtures/finops_idle_disks.json");
const UNUSED_ADDRESSES: &str =
    include_str!("../../../../tests/fixtures/finops_unused_addresses.json");

/// The `(recommender, location)` pair a request's parent encodes.
///
/// `projects/<project>/locations/<location>/recommenders/<recommender>` is the
/// only shape `SdkFinOpsService` builds, and anything else yields `None` --
/// which answers `[]` rather than picking a fixture by accident. Matched as a
/// whole path rather than by counting segments from either end: a `parent`
/// that named a billing account or an organisation has the same length and a
/// different meaning.
fn pair(parent: &str) -> Option<(&str, &str)> {
    match parent.split('/').collect::<Vec<_>>().as_slice() {
        [
            "projects",
            _,
            "locations",
            location,
            "recommenders",
            recommender,
        ] => Some((recommender, location)),
        _ => None,
    }
}

/// The capture for one pair, or the empty listing.
fn recommendations_for(parent: &str) -> &'static str {
    let Some((recommender, location)) = pair(parent) else {
        return EMPTY;
    };
    match (recommender, location) {
        ("google.compute.instance.IdleResourceRecommender", "europe-west1-b") => IDLE_VMS,
        ("google.compute.instance.MachineTypeRecommender", "europe-west1-b") => MACHINE_TYPE,
        ("google.compute.disk.IdleResourceRecommender", "europe-west1-b") => IDLE_DISKS,
        ("google.compute.address.IdleResourceRecommender", "europe-west1") => UNUSED_ADDRESSES,
        _ => EMPTY,
    }
}

/// Answers `list_recommendations` from the capture the request's parent names.
///
/// One page per pair, so the item paginator stops after it.
/// `ListRecommendations` has no poller in the generated builder, and the
/// service makes no other call, so this is the only method reached.
#[derive(Debug, Default)]
pub struct RecommenderStub;

impl google_cloud_recommender_v1::stub::Recommender for RecommenderStub {
    async fn list_recommendations(
        &self,
        request: model::ListRecommendationsRequest,
        _options: google_cloud_gax::options::RequestOptions,
    ) -> google_cloud_gax::Result<
        google_cloud_gax::response::Response<model::ListRecommendationsResponse>,
    > {
        Ok(google_cloud_gax::response::Response::from(envelope(
            "recommendations",
            recommendations_for(&request.parent),
        )?))
    }
}

/// The client `SdkFinOpsService::from_fixtures` is built from.
pub fn client() -> Recommender {
    Recommender::from_stub(RecommenderStub)
}
