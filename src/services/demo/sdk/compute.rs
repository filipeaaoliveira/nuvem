//! Compute Engine's demo backend: the aggregated listing, the detail pane and
//! the five lifecycle verbs, all from the captures.
//!
//! `instances.aggregatedList` answers a map keyed by `zones/<zone>`, each value
//! an object holding that zone's instances. `gce_instances.json` is the flat
//! array gcloud printed from such a map, so [`by_scope`]
//! rebuilds the grouping at call time out of each row's own `zone` self-link --
//! read with `tail_segment`, which is the function `map_instance` paints the
//! ZONE column with. Reading the key off the same field with the same function
//! is what makes it impossible for the key and the cell to disagree.
//!
//! The capture names two zones, so this answers two scopes rather than one.
//!
//! `instances.get` answers the resource itself rather than a message wrapping
//! it, so `gce_instance_detail.json` is deserialized directly -- the shape
//! [`envelope`](super::envelope) does not cover, and the same one
//! `gke_cluster_detail.json` has.
//!
//! # What the listing's one method reaches
//!
//! Only `aggregated_list`. The generated `instances::AggregatedList` builder
//! has `send`, `by_page` and `by_item`; `by_page` re-`send`s with a page token
//! and `by_item` is `by_page().items()`, so there is no poller on it and no
//! second stub method behind it. The answer leaves `next_page_token` unset,
//! which is what stops the paginator after one page instead of asking for a
//! second.
//!
//! # What a lifecycle verb reaches, and why it is more than the verb
//!
//! `SdkComputeService::run_action` ends every verb in `.poller().until_done()`,
//! and the generated poller is built out of three things read off *this* stub:
//! the polling error policy, the polling backoff policy and the poller options.
//! Those three are trait accessors with real defaults -- `Aip194Strict`,
//! `ExponentialBackoff::default()` and `PollerOptions::default()` -- rather
//! than the `unimplemented_stub()` every RPC method defaults to, so leaving
//! them alone is safe where leaving an RPC method alone is a panic.
//!
//! The poller then calls the verb's own stub method, and, for an operation
//! that is not yet done, reads it back through `get_operation`. That last one
//! is the method `SdkComputeService` never writes and the reason a reading of
//! the service's call sites is not enough to enumerate this stub.
//!
//! # Why a demo mutation changes nothing
//!
//! Stopping an instance here does not stop it. The captures are `include_str!`
//! constants compiled into the binary, so there is no state for a verb to
//! write to: the table paints the same rows after a Stop as before it, and the
//! instance the detail pane describes is still `RUNNING`. That is deliberate
//! and it is what the CLI demo backend already does -- `DemoCommandRunner`
//! answers a lifecycle argv with empty stdout and a zero exit, changing
//! nothing -- so the flip at `services/runtime/session.rs` keeps demo mode's
//! behaviour rather than changing it. What a verb does prove is the path: the
//! confirmation, the operations-log entry and its settlement all run for real.

use google_cloud_compute_v1::client::Instances;
use google_cloud_compute_v1::model;
use google_cloud_gax::error::Error;

use crate::services::resources::compute::tail_segment;

use super::{aggregated_project, by_scope, fans_out_over_every_scope, named, scope};
use crate::services::demo::Corpus;

/// The instances capture: `gcloud compute instances list --format=json`.
const INSTANCES: &str = include_str!("../../../../tests/fixtures/gce_instances.json");

/// The detail capture: `gcloud compute instances describe --format=json`, a
/// single instance rather than a list.
const INSTANCE_DETAIL: &str = include_str!("../../../../tests/fixtures/gce_instance_detail.json");

/// What `nuvem --demo` lists: see [`Corpus::Showcase`].
const SHOWCASE_INSTANCES: &str = include_str!("../../../../tests/fixtures/demo/gce_instances.json");

/// The capture regrouped into the zone-keyed map the RPC answers.
fn aggregated(instances: &str) -> Result<model::InstanceAggregatedList, Error> {
    let scopes = by_scope(instances, |instance: &model::Instance| {
        scope(instance.zone.as_deref().map(tail_segment), None)
    })?;
    Ok(
        model::InstanceAggregatedList::new().set_items(scopes.into_iter().map(
            |(scope, instances)| {
                (
                    scope,
                    model::InstancesScopedList::new().set_instances(instances),
                )
            },
        )),
    )
}

/// The operation every lifecycle verb answers: one that has already finished,
/// and finished cleanly.
///
/// # Why `DONE` is not enough on its own
///
/// `DiscoveryOperation::done` for this model is `status == Some(DONE)`, and
/// the poller's `handle_start` completes with `Ok` the moment that holds --
/// *including* for an operation that reached `DONE` by being refused, because
/// Compute reports a refusal in the operation's own `error` and
/// `httpError*` fields rather than as a transport failure. `until_done`
/// passes that `Ok` straight through. `finished_or_error` in
/// `services/sdk/compute.rs` is what reads those fields and turns a refused
/// operation into the `Err` the operations log needs, so an operation
/// answered here with `DONE` *and* an error would settle every demo
/// lifecycle action at `Failed`. Leaving all three fields unset is what makes
/// a demo action succeed.
///
/// # Why `DONE` at all, rather than `RUNNING`
///
/// A `RUNNING` operation sends the poller into its polling loop: it would
/// back off and re-read through `get_operation`, so a demo action would pay
/// `ExponentialBackoff`'s first wait before settling. Answering an operation
/// that is already finished lets `handle_start` complete on the first call,
/// which is why a demo lifecycle action settles instantly rather than after a
/// sleep nobody asked for.
///
/// Nothing else is filled in. The name is here so that the operation this
/// stub hands back and the one `get_operation` answers are the same operation;
/// every other field is output-only detail no nuvem code path reads.
fn finished() -> model::Operation {
    model::Operation::new()
        .set_name("operation-demo-fixture")
        .set_status(model::operation::Status::Done)
}

/// Answers the aggregated listing, the detail pane and the five lifecycle
/// verbs from the captures.
#[derive(Debug, Default)]
pub struct InstancesStub {
    pub corpus: Corpus,
}

impl google_cloud_compute_v1::stub::Instances for InstancesStub {
    /// The request is read before it is answered. See
    /// [`aggregated_project`] for why a stub judges
    /// one at all: the builder runs on the demo path exactly as it does on the
    /// live one, and a project or a scope flag nobody reads is a live-only
    /// failure with a green suite behind it.
    async fn aggregated_list(
        &self,
        request: model::instances::AggregatedListRequest,
        _options: google_cloud_gax::options::RequestOptions,
    ) -> google_cloud_gax::Result<google_cloud_gax::response::Response<model::InstanceAggregatedList>>
    {
        aggregated_project(&request.project)?;
        fans_out_over_every_scope(request.include_all_scopes, request.return_partial_success)?;
        Ok(google_cloud_gax::response::Response::from(aggregated(
            self.corpus.pick(INSTANCES, SHOWCASE_INSTANCES),
        )?))
    }

    /// The instance the request names: the detail capture for the instance it
    /// describes, and the listing's own row for every other one. See
    /// [`named`].
    async fn get(
        &self,
        request: model::instances::GetRequest,
        _options: google_cloud_gax::options::RequestOptions,
    ) -> google_cloud_gax::Result<google_cloud_gax::response::Response<model::Instance>> {
        let instance: model::Instance = named(
            &self
                .corpus
                .documents(&[INSTANCE_DETAIL, INSTANCES], &[SHOWCASE_INSTANCES]),
            |name| name == request.instance,
            &format!("instance `{}`", request.instance),
        )?;
        Ok(google_cloud_gax::response::Response::from(instance))
    }

    async fn start(
        &self,
        _request: model::instances::StartRequest,
        _options: google_cloud_gax::options::RequestOptions,
    ) -> google_cloud_gax::Result<google_cloud_gax::response::Response<model::Operation>> {
        Ok(google_cloud_gax::response::Response::from(finished()))
    }

    async fn stop(
        &self,
        _request: model::instances::StopRequest,
        _options: google_cloud_gax::options::RequestOptions,
    ) -> google_cloud_gax::Result<google_cloud_gax::response::Response<model::Operation>> {
        Ok(google_cloud_gax::response::Response::from(finished()))
    }

    async fn reset(
        &self,
        _request: model::instances::ResetRequest,
        _options: google_cloud_gax::options::RequestOptions,
    ) -> google_cloud_gax::Result<google_cloud_gax::response::Response<model::Operation>> {
        Ok(google_cloud_gax::response::Response::from(finished()))
    }

    async fn suspend(
        &self,
        _request: model::instances::SuspendRequest,
        _options: google_cloud_gax::options::RequestOptions,
    ) -> google_cloud_gax::Result<google_cloud_gax::response::Response<model::Operation>> {
        Ok(google_cloud_gax::response::Response::from(finished()))
    }

    async fn resume(
        &self,
        _request: model::instances::ResumeRequest,
        _options: google_cloud_gax::options::RequestOptions,
    ) -> google_cloud_gax::Result<google_cloud_gax::response::Response<model::Operation>> {
        Ok(google_cloud_gax::response::Response::from(finished()))
    }

    /// Reached by the poller the five verbs above end in, never by
    /// `SdkComputeService`.
    ///
    /// Unreachable while those verbs answer a finished operation, because the
    /// poller completes on the first call and never asks a second time. It is
    /// spelled out anyway for the reason the whole stub is spelled out: the
    /// generated default is `unimplemented!()`, which on a blocking effect
    /// thread aborts nuvem rather than reporting anything, and the day a verb
    /// here answers something other than `DONE` is the day this method decides
    /// whether the action settles or the process dies.
    async fn get_operation(
        &self,
        _request: model::zone_operations::GetRequest,
        _options: google_cloud_gax::options::RequestOptions,
    ) -> google_cloud_gax::Result<google_cloud_gax::response::Response<model::Operation>> {
        Ok(google_cloud_gax::response::Response::from(finished()))
    }
}

/// The client `SdkComputeService::from_fixtures` is built from.
pub fn client() -> Instances {
    client_of(Corpus::Captures)
}

/// The client over either corpus.
pub fn client_of(corpus: Corpus) -> Instances {
    Instances::from_stub(InstancesStub { corpus })
}
