//! Persistent disks' demo backend: the aggregated listing, from the capture.
//!
//! `disks.aggregatedList` answers a scope-keyed map rather than an array, and
//! disks are the service where both kinds of scope appear: a zonal disk is
//! filed under `zones/<zone>` and a regional one under `regions/<region>`.
//! [`by_scope`] rebuilds the grouping from each row's own
//! `zone`/`region` self-link, shortened with `short_id` -- the function
//! `map_disk` fills the LOCATION column with, in the same `zone`-then-`region`
//! precedence -- so the key and the cell read the same field.
//!
//! `persistent_disks.json` also carries a row with neither field. It is filed
//! under `global`, the API's own name for that scope, rather than dropped:
//! dropping it would make the native backend paint one row fewer than the CLI
//! backend from the same bytes, which is the drift this module exists to
//! prevent.
//!
//! # What the one wired method reaches
//!
//! Only `aggregated_list`. The generated `disks::AggregatedList` builder has
//! `send`, `by_page` and `by_item` and no poller, and `by_page` re-`send`s with
//! a page token, so nothing else on the `Disks` stub is reachable from
//! `SdkDisksService`'s single call. Every other method stays defaulted for
//! that reason; `next_page_token` is left unset so the paginator stops after
//! this page instead of asking for a second.

use google_cloud_compute_v1::client::Disks;
use google_cloud_compute_v1::model;
use google_cloud_gax::error::Error;

use crate::services::resources::disks::short_id;

use super::{aggregated_project, by_scope, fans_out_over_every_scope, scope};
use crate::services::demo::Corpus;

/// The disks capture: `gcloud compute disks list --format=json`.
const DISKS: &str = include_str!("../../../../tests/fixtures/persistent_disks.json");

/// What `nuvem --demo` lists: see [`Corpus::Showcase`].
const SHOWCASE_DISKS: &str = include_str!("../../../../tests/fixtures/demo/persistent_disks.json");

/// The capture regrouped into the scope-keyed map the RPC answers.
fn aggregated(disks: &str) -> Result<model::DiskAggregatedList, Error> {
    let scopes = by_scope(disks, |disk: &model::Disk| {
        scope(
            disk.zone.as_deref().map(short_id),
            disk.region.as_deref().map(short_id),
        )
    })?;
    Ok(model::DiskAggregatedList::new().set_items(
        scopes
            .into_iter()
            .map(|(scope, disks)| (scope, model::DisksScopedList::new().set_disks(disks))),
    ))
}

/// Answers `aggregated_list` from the disks capture.
#[derive(Debug, Default)]
pub struct DisksStub {
    pub corpus: Corpus,
}

impl google_cloud_compute_v1::stub::Disks for DisksStub {
    /// The project and both scope flags are read before the capture is
    /// answered, as they are in `compute.rs`: `gcloud compute disks list`
    /// sends the same two, and either one flipped changes which zones report.
    async fn aggregated_list(
        &self,
        request: model::disks::AggregatedListRequest,
        _options: google_cloud_gax::options::RequestOptions,
    ) -> google_cloud_gax::Result<google_cloud_gax::response::Response<model::DiskAggregatedList>>
    {
        aggregated_project(&request.project)?;
        fans_out_over_every_scope(request.include_all_scopes, request.return_partial_success)?;
        Ok(google_cloud_gax::response::Response::from(aggregated(
            self.corpus.pick(DISKS, SHOWCASE_DISKS),
        )?))
    }
}

/// The client `SdkDisksService::from_fixtures` is built from.
pub fn client() -> Disks {
    client_of(Corpus::Captures)
}

/// The client over either corpus.
pub fn client_of(corpus: Corpus) -> Disks {
    Disks::from_stub(DisksStub { corpus })
}
