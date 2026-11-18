//! Pub/Sub's demo backend, and the template every other service follows once
//! it is wired.
//!
//! The two stubs answer from the same captures the CLI path serves and the
//! CLI parser tests read -- `tests/fixtures/pubsub_topics.json` and
//! `pubsub_subscriptions.json`, embedded here by `include_str!` exactly as
//! `DemoCommandRunner` embeds them. Neither file is copied or reshaped for
//! this backend: [`envelope`] supplies the message wrapper
//! the RPC puts around the array at runtime, so there is one capture per
//! listing and both backends read it.
//!
//! Both calls matter, and the second is easy to miss. The topics table's
//! subscription count is not carried by a topic at all -- it is derived by
//! listing the project's *subscriptions* and grouping them by parent, which is
//! a second request that the topics fixture knows nothing about. A backend
//! that stubbed only `list_topics` would paint four intact columns and one
//! dash, on a table of five.

use google_cloud_pubsub::client::{SubscriptionAdmin, TopicAdmin};
use google_cloud_pubsub::model;

use super::envelope;

/// The topics capture: `gcloud pubsub topics list --format=json`.
const TOPICS: &str = include_str!("../../../../tests/fixtures/pubsub_topics.json");

/// The subscriptions capture, which answers both the flat subscriptions table
/// and the topics table's count column.
const SUBSCRIPTIONS: &str = include_str!("../../../../tests/fixtures/pubsub_subscriptions.json");

/// Answers `list_topics` from the capture, and nothing else.
///
/// Every other method of the trait stays defaulted deliberately: this service
/// makes exactly one call on this client, and a default that panics is a
/// louder answer to a call nobody expected than a fabricated success would be.
#[derive(Debug, Default)]
pub struct TopicStub;

impl google_cloud_pubsub::stub::TopicAdmin for TopicStub {
    async fn list_topics(
        &self,
        _request: model::ListTopicsRequest,
        _options: google_cloud_gax::options::RequestOptions,
    ) -> google_cloud_gax::Result<google_cloud_gax::response::Response<model::ListTopicsResponse>>
    {
        // No `next_page_token` is set, so the paginator reads this as the one
        // and only page and stops -- the same shape a small project answers.
        Ok(google_cloud_gax::response::Response::from(envelope(
            "topics", TOPICS,
        )?))
    }
}

/// Answers `list_subscriptions` from the capture, and nothing else.
#[derive(Debug, Default)]
pub struct SubscriptionStub;

impl google_cloud_pubsub::stub::SubscriptionAdmin for SubscriptionStub {
    async fn list_subscriptions(
        &self,
        _request: model::ListSubscriptionsRequest,
        _options: google_cloud_gax::options::RequestOptions,
    ) -> google_cloud_gax::Result<
        google_cloud_gax::response::Response<model::ListSubscriptionsResponse>,
    > {
        Ok(google_cloud_gax::response::Response::from(envelope(
            "subscriptions",
            SUBSCRIPTIONS,
        )?))
    }
}

/// The two clients `SdkPubSubService::from_fixtures` is built from.
pub fn clients() -> (TopicAdmin, SubscriptionAdmin) {
    (
        TopicAdmin::from_stub(TopicStub),
        SubscriptionAdmin::from_stub(SubscriptionStub),
    )
}
