//! Pub/Sub topics and subscriptions through the native client.

use google_cloud_auth::credentials::Credentials;
use google_cloud_pubsub::client::{SubscriptionAdmin, TopicAdmin};
use nuvem_domain::gcp::resource::{
    PubSubDeadLetterPolicy, PubSubSubscription, PubSubSubscriptionState, PubSubTopic,
    SubscriptionCount,
};

use crate::services::resources::list_filter::{RESOURCE_LIST_LIMIT, rows_are_capped};
use crate::services::resources::pubsub::{
    apply_subscription_counts, delivery_from_endpoint, key_values, short_id,
};
use crate::services::sdk::{ApiError, non_empty};

pub struct SdkPubSubService {
    topics: TopicAdmin,
    subscriptions: SubscriptionAdmin,
}

impl SdkPubSubService {
    pub async fn connect(credentials: Credentials) -> Result<Self, ApiError> {
        let topics = TopicAdmin::builder()
            .with_credentials(credentials.clone())
            .build()
            .await
            .map_err(|error| ApiError::new(error.to_string()))?;
        let subscriptions = SubscriptionAdmin::builder()
            .with_credentials(credentials)
            .build()
            .await
            .map_err(|error| ApiError::new(error.to_string()))?;
        Ok(Self {
            topics,
            subscriptions,
        })
    }

    /// The same service, built from already-constructed clients. See
    /// `SdkGkeService::from_clusters_for_test`.
    pub fn from_clients_for_test(topics: TopicAdmin, subscriptions: SubscriptionAdmin) -> Self {
        Self {
            topics,
            subscriptions,
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
        let (topics, subscriptions) = crate::services::demo::sdk::pubsub::clients();
        Self {
            topics,
            subscriptions,
        }
    }
    /// Every topic in the project, each carrying its subscription count.
    ///
    /// # Why this makes a second request
    ///
    /// A topic listing cannot answer how many subscriptions point at a topic:
    /// the count lives on the subscriptions, so it is derived by listing them
    /// and grouping by parent. `GcloudPubSubService::load_topics` has always
    /// done exactly this, and the count column is not optional decoration --
    /// it is one of the five columns the topics table renders.
    ///
    /// The join is best-effort and swallowed here, for the same reason it is
    /// swallowed on the CLI path: a refused permission or a failed second
    /// request leaves four intact columns and one that reads as unknown, which
    /// is a better answer than failing a load that already succeeded. The flat
    /// subscriptions table makes the opposite choice from the same call, which
    /// is why the failure policy lives at the call site rather than inside
    /// [`load_subscriptions`](Self::load_subscriptions).
    pub async fn load_topics(&self, project: &str) -> Result<Vec<PubSubTopic>, ApiError> {
        use google_cloud_gax::paginator::ItemPaginator;
        let mut rows = Vec::new();
        let mut items = self
            .topics
            .list_topics()
            .set_project(format!("projects/{project}"))
            .set_page_size(RESOURCE_LIST_LIMIT as i32)
            .by_item();
        while let Some(topic) = items.next().await {
            rows.push(map_topic(&topic?));
            if rows_are_capped(rows.len(), RESOURCE_LIST_LIMIT) {
                break;
            }
        }
        if let Ok(subscriptions) = self.load_subscriptions(project).await {
            apply_subscription_counts(&mut rows, &subscriptions);
        }
        Ok(rows)
    }

    pub async fn load_subscriptions(
        &self,
        project: &str,
    ) -> Result<Vec<PubSubSubscription>, ApiError> {
        use google_cloud_gax::paginator::ItemPaginator;
        let mut rows = Vec::new();
        let mut items = self
            .subscriptions
            .list_subscriptions()
            .set_project(format!("projects/{project}"))
            .set_page_size(RESOURCE_LIST_LIMIT as i32)
            .by_item();
        while let Some(subscription) = items.next().await {
            rows.push(map_subscription(&subscription?));
            if rows_are_capped(rows.len(), RESOURCE_LIST_LIMIT) {
                break;
            }
        }
        Ok(rows)
    }
}

/// A retention window, rendered the way gcloud rendered it.
///
/// The CLI answered `"604800s"`; the client answers a protobuf `Duration`.
/// The cell shows this verbatim, so the two have to agree.
fn duration_to_string(value: Option<&google_cloud_wkt::Duration>) -> Option<String> {
    value.map(|duration| format!("{}s", duration.seconds()))
}

pub fn map_topic(topic: &google_cloud_pubsub::model::Topic) -> PubSubTopic {
    PubSubTopic {
        name: short_id(&topic.name),
        full_name: topic.name.clone(),
        // Left unknown here and filled in by the join in
        // `SdkPubSubService::load_topics`, exactly as on the CLI path: a topic
        // listing alone cannot answer it.
        subscription_count: SubscriptionCount::Unavailable,
        message_retention: duration_to_string(topic.message_retention_duration.as_ref()),
        kms_key: non_empty(&topic.kms_key_name),
        labels: key_values(
            topic
                .labels
                .iter()
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect(),
        ),
    }
}

pub fn map_subscription(
    subscription: &google_cloud_pubsub::model::Subscription,
) -> PubSubSubscription {
    PubSubSubscription {
        name: short_id(&subscription.name),
        full_name: subscription.name.clone(),
        topic: subscription.topic.clone(),
        delivery: delivery_from_endpoint(
            subscription
                .push_config
                .as_ref()
                .map(|config| config.push_endpoint.clone()),
        ),
        // `0` is the model's unset, and a zero-second ack deadline is not a
        // deadline, so it reads as absent rather than as an instant timeout.
        ack_deadline_seconds: (subscription.ack_deadline_seconds != 0)
            .then_some(subscription.ack_deadline_seconds)
            .and_then(|seconds| u32::try_from(seconds).ok()),
        message_retention: duration_to_string(subscription.message_retention_duration.as_ref()),
        dead_letter: subscription.dead_letter_policy.as_ref().map(|policy| {
            PubSubDeadLetterPolicy {
                dead_letter_topic: policy.dead_letter_topic.clone(),
                max_delivery_attempts: u32::try_from(policy.max_delivery_attempts).ok(),
            }
        }),
        state: PubSubSubscriptionState::from_api(subscription.state.name()),
        labels: key_values(
            subscription
                .labels
                .iter()
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect(),
        ),
    }
}
