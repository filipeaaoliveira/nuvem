//! Pub/Sub topics, and the subscription count derived alongside them.
//!
//! # Two calls, only one of them essential
//!
//! A topic resource does not carry the number of subscriptions attached to it.
//! That number comes from listing the project's *subscriptions* and grouping
//! them by the `topic` each one names — `gcloud pubsub subscriptions list`
//! returns every subscription in the project with its parent topic on each row,
//! so the grouping is client-side off **one** extra call rather than one call
//! per topic.
//!
//! The two calls are not equals. The topics call is the load; the subscriptions
//! call is an enrichment, and it is the one an operator may not be allowed to
//! make, since `pubsub.topics.list` granted with `pubsub.subscriptions.list`
//! denied is an ordinary read-only IAM shape. So a failure of the second call
//! is swallowed and every count becomes [`SubscriptionCount::Unavailable`],
//! leaving the table's other four columns intact. A topics list that fails
//! wholesale because a secondary enrichment call failed is worse than one with
//! no count. Only a failure of the *topics* call fails the load.
//!
//! # Why not `BatchableService`
//!
//! `BatchableService` (`crate::app::state::resources::batch`) was considered
//! for the second call and does not fit. It is the trait for marked-row
//! *lifecycle batch actions*: it carries an `Action` type, an eligibility test,
//! a skip reason and a per-row gcloud command preview, so that a multi-row
//! start or stop can report which rows it declined and why. It says nothing
//! about fanning a read across rows. This service is read-only and has no
//! lifecycle action to give it, and the enrichment here is one project-wide
//! call rather than one call per row — the opposite of a fan-out.

use serde::Deserialize;

use nuvem_domain::gcp::configuration::GcloudContext;
use nuvem_domain::gcp::filter::label_filter_field;
use nuvem_domain::gcp::resource::{
    ComputeKeyValue, PubSubDeadLetterPolicy, PubSubDelivery, PubSubSubscription,
    PubSubSubscriptionState, PubSubTopic, ResourceServiceKind, SubscriptionCount,
};

use crate::services::gcloud::cli::{CommandError, CommandRunner, GcloudCommand};
use crate::services::resources::list_filter::{RESOURCE_LIST_LIMIT, pushdown_expression};

/// Verbatim-superset raw fields eligible for the regex pushdown — deliberately
/// none, and staying that way.
///
/// The never-drop-a-row invariant is a claim about *display* values: each of a
/// service's searchable fields must be a verbatim substring of a pushed raw
/// field, or be enumerated in [`OPAQUE_LABELS`]. The topics table's searchable
/// set includes the subscription count's rendered label, which appears in no
/// raw field of the topic resource at all — it is derived from a different
/// call. Pushing any clause would therefore have to enumerate every count an
/// operator might type. Empty degrades to full client-side narrowing, which is
/// the safe direction.
const FILTER_FIELDS: &[&str] = &[];

/// Empty for the subscriptions table, and for a reason of its own rather than
/// the topics table's.
///
/// The never-drop-a-row invariant is a claim about *display* values: each
/// searchable field must be a verbatim substring of a pushed raw field. Two of
/// this table's columns fail that outright. `DELIVERY` renders `Pull`/`Push`,
/// which is derived from whether `pushConfig` carries an endpoint and appears
/// verbatim in no raw field at all. `STATE` renders `Resource error` against a
/// raw `RESOURCE_ERROR`. Pushing any clause would drop rows an operator can
/// see, so nothing is pushed and narrowing stays wholly client-side — the safe
/// direction.
const SUBSCRIPTION_FILTER_FIELDS: &[&str] = &[];

/// Empty for the same reason [`SUBSCRIPTION_FILTER_FIELDS`] is: with nothing
/// pushed there is nothing for the gate to protect.
const SUBSCRIPTION_OPAQUE_LABELS: &[&str] = &[];

/// Empty for the same reason [`FILTER_FIELDS`] is: with nothing pushed there is
/// nothing for the gate to protect.
const OPAQUE_LABELS: &[&str] = &[];

/// The value a subscription's `topic` carries once its topic has been deleted.
/// A real value the API returns rather than an error, and it matches no topic.
const DELETED_TOPIC: &str = "_deleted-topic_";

#[derive(Debug, thiserror::Error)]
pub enum PubSubParseError {
    #[error("invalid Pub/Sub JSON: {0}")]
    Json(#[from] serde_json::Error),
}

#[derive(Debug, thiserror::Error)]
pub enum PubSubLoadError {
    #[error("{0}")]
    Command(#[from] CommandError),
    #[error("{0}")]
    Parse(#[from] PubSubParseError),
}

pub struct GcloudPubSubService<R> {
    runner: R,
}

impl<R> GcloudPubSubService<R> {
    pub fn new(runner: R) -> Self {
        Self { runner }
    }

    pub fn runner(&self) -> &R {
        &self.runner
    }
}

impl<R: CommandRunner> GcloudPubSubService<R> {
    /// Lists a project's topics, each carrying the number of subscriptions
    /// attached to it where that could be determined.
    ///
    /// The subscriptions call is best-effort by construction: its result is
    /// only ever used to *upgrade* counts from
    /// [`SubscriptionCount::Unavailable`], so any way it can fail — a refused
    /// permission, a timeout, an unreadable document — leaves the topics
    /// exactly as the first call parsed them.
    pub fn load_topics(
        &self,
        context: &GcloudContext,
        filter: Option<&str>,
    ) -> Result<Vec<PubSubTopic>, PubSubLoadError> {
        let mut args = base_args(context);
        args.extend([
            "pubsub".to_string(),
            "topics".to_string(),
            "list".to_string(),
            "--format=json".to_string(),
            format!("--limit={RESOURCE_LIST_LIMIT}"),
        ]);
        if let Some(expression) = filter.and_then(|applied| {
            pushdown_expression(
                applied,
                FILTER_FIELDS,
                OPAQUE_LABELS,
                label_filter_field(ResourceServiceKind::PubSubTopics),
            )
        }) {
            args.push(format!("--filter={expression}"));
        }
        let output = self.runner.run(GcloudCommand::new(args))?;
        let mut topics = parse_topics_json(&output.stdout)?;

        // Best-effort by construction, and the one place that decides so. The
        // result is only ever used to *upgrade* counts away from
        // `Unavailable`, so any way the call can fail — a refused permission,
        // a timeout, an unreadable document — leaves the topics exactly as the
        // first call parsed them. The flat subscriptions table makes the
        // opposite choice from the same call.
        if let Ok(subscriptions) = self.load_subscriptions(context, None) {
            apply_subscription_counts(&mut topics, &subscriptions);
        }

        Ok(topics)
    }

    /// Every subscription in the project, each naming its parent topic.
    ///
    /// One call returns the whole set, which is what lets both callers exist:
    /// the flat subscriptions table renders these rows directly, and
    /// [`load_topics`](Self::load_topics) groups the same rows by topic to
    /// derive its count column.
    ///
    /// # The failure policy belongs to the caller, not to this call
    ///
    /// This hands out a `Result` and takes no view on what a failure means,
    /// because the two callers need opposite things from the same refusal.
    /// For the flat table a refusal is the load failing: there is nothing left
    /// to render, and an empty table would be the claim "this project has no
    /// subscriptions", which a denied permission is in no position to make.
    /// For the topics table the same refusal is a missing count on a table
    /// whose other four columns are intact, so it is swallowed at that call
    /// site — see the note there.
    ///
    /// Collapsing the two into one policy here is the regression this shape
    /// exists to prevent, and `pubsub_service_test.rs` pins both halves.
    pub fn load_subscriptions(
        &self,
        context: &GcloudContext,
        filter: Option<&str>,
    ) -> Result<Vec<PubSubSubscription>, PubSubLoadError> {
        let mut args = base_args(context);
        args.extend([
            "pubsub".to_string(),
            "subscriptions".to_string(),
            "list".to_string(),
            "--format=json".to_string(),
            format!("--limit={RESOURCE_LIST_LIMIT}"),
        ]);
        if let Some(expression) = filter.and_then(|applied| {
            pushdown_expression(
                applied,
                SUBSCRIPTION_FILTER_FIELDS,
                SUBSCRIPTION_OPAQUE_LABELS,
                label_filter_field(ResourceServiceKind::PubSubSubscriptions),
            )
        }) {
            args.push(format!("--filter={expression}"));
        }
        let output = self.runner.run(GcloudCommand::new(args))?;
        Ok(parse_subscriptions_json(&output.stdout)?)
    }
}

/// Shared leading args: `--configuration <cfg> [--project <p>]`.
fn base_args(context: &GcloudContext) -> Vec<String> {
    let mut args = vec![
        "--configuration".to_string(),
        context.configuration.as_str().to_string(),
    ];
    if let Some(project) = context.project.as_deref() {
        args.extend(["--project".to_string(), project.to_string()]);
    }
    args
}

/// The wire shape of a topic.
///
/// Every field defaults, so a row missing any of them still parses; only a
/// document that is not the expected JSON fails the load, which is the same
/// bargain the sibling services strike.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawTopic {
    #[serde(default)]
    name: String,
    #[serde(default)]
    message_retention_duration: Option<String>,
    #[serde(default)]
    kms_key_name: Option<String>,
    #[serde(default)]
    labels: std::collections::BTreeMap<String, String>,
}

/// The wire shape of a subscription.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawSubscription {
    #[serde(default)]
    name: String,
    #[serde(default)]
    topic: String,
    #[serde(default)]
    push_config: Option<RawPushConfig>,
    #[serde(default)]
    ack_deadline_seconds: Option<u32>,
    #[serde(default)]
    message_retention_duration: Option<String>,
    #[serde(default)]
    dead_letter_policy: Option<RawDeadLetterPolicy>,
    #[serde(default)]
    state: Option<String>,
    #[serde(default)]
    labels: std::collections::BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawPushConfig {
    #[serde(default)]
    push_endpoint: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawDeadLetterPolicy {
    #[serde(default)]
    dead_letter_topic: String,
    #[serde(default)]
    max_delivery_attempts: Option<u32>,
}

pub fn parse_topics_json(stdout: &str) -> Result<Vec<PubSubTopic>, PubSubParseError> {
    let raw: Vec<RawTopic> = serde_json::from_str(stdout)?;
    Ok(raw.into_iter().map(topic_from_raw).collect())
}

/// A topic as the list call knows it — which is to say, with no count.
///
/// The count is not defaulted to `Known(0)` here: this call cannot see a single
/// subscription, so zero would be a claim it is in no position to make. It
/// starts unknown and is upgraded by [`apply_subscription_counts`] only once
/// the other call has actually answered.
fn topic_from_raw(raw: RawTopic) -> PubSubTopic {
    PubSubTopic {
        name: short_id(&raw.name),
        full_name: raw.name,
        subscription_count: SubscriptionCount::Unavailable,
        message_retention: raw.message_retention_duration,
        kms_key: raw.kms_key_name,
        labels: key_values(raw.labels),
    }
}

pub fn parse_subscriptions_json(stdout: &str) -> Result<Vec<PubSubSubscription>, PubSubParseError> {
    let raw: Vec<RawSubscription> = serde_json::from_str(stdout)?;
    Ok(raw.into_iter().map(subscription_from_raw).collect())
}

fn subscription_from_raw(raw: RawSubscription) -> PubSubSubscription {
    PubSubSubscription {
        name: short_id(&raw.name),
        full_name: raw.name,
        topic: raw.topic,
        delivery: delivery_from_raw(raw.push_config),
        ack_deadline_seconds: raw.ack_deadline_seconds,
        message_retention: raw.message_retention_duration,
        dead_letter: raw.dead_letter_policy.map(|policy| PubSubDeadLetterPolicy {
            dead_letter_topic: policy.dead_letter_topic,
            max_delivery_attempts: policy.max_delivery_attempts,
        }),
        state: PubSubSubscriptionState::from_api(raw.state.as_deref()),
        labels: key_values(raw.labels),
    }
}

/// Push only when there is an endpoint to push to.
///
/// gcloud renders a pull subscription's `pushConfig` as `{}` rather than
/// omitting it, so the field's *presence* says nothing. Reading presence as
/// push would label every pull subscription a push one with a blank endpoint.
fn delivery_from_raw(push_config: Option<RawPushConfig>) -> PubSubDelivery {
    delivery_from_endpoint(push_config.and_then(|config| config.push_endpoint))
}

/// The rule itself, over the endpoint alone, so both backends agree.
///
/// An empty endpoint is not a push endpoint: gcloud renders a pull
/// subscription's `pushConfig` as `{}` and the client answers a `PushConfig`
/// whose endpoint is `""`, and reading either as push would label every pull
/// subscription a push one with a blank target.
pub fn delivery_from_endpoint(endpoint: Option<String>) -> PubSubDelivery {
    match endpoint.filter(|endpoint| !endpoint.is_empty()) {
        Some(endpoint) => PubSubDelivery::Push { endpoint },
        None => PubSubDelivery::Pull,
    }
}

/// Attributes each subscription to the topic it names, and marks every topic's
/// count as known.
///
/// Called only when the subscriptions call succeeded, which is what makes
/// `Known(0)` correct for a topic no subscription named: the answer is in hand
/// and it is zero, as distinct from [`SubscriptionCount::Unavailable`].
///
/// The join is on the *full* resource name, exactly. A subscription names its
/// topic as `projects/p/topics/orders`, and matching on the short id — or on
/// any suffix or `contains` test — would give `orders` the subscriptions of
/// `back-orders` too.
pub fn apply_subscription_counts(topics: &mut [PubSubTopic], subscriptions: &[PubSubSubscription]) {
    let mut counts: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
    for subscription in subscriptions {
        if subscription.topic == DELETED_TOPIC {
            continue;
        }
        *counts.entry(subscription.topic.as_str()).or_insert(0) += 1;
    }

    for topic in topics {
        let count = counts.get(topic.full_name.as_str()).copied().unwrap_or(0);
        topic.subscription_count = SubscriptionCount::Known(count);
    }
}

/// The trailing segment of a fully-qualified resource path.
///
/// The API returns `projects/p/topics/orders`; a table cell wants `orders`. A
/// name with no slash is already short and passes through unchanged.
pub fn short_id(resource_path: &str) -> String {
    resource_path
        .rsplit('/')
        .next()
        .unwrap_or(resource_path)
        .to_string()
}

/// Sorted key/value pairs. Sorted rather than in map order so a reload does not
/// reshuffle them under the operator's cursor.
pub fn key_values(map: std::collections::BTreeMap<String, String>) -> Vec<ComputeKeyValue> {
    map.into_iter()
        .map(|(key, value)| ComputeKeyValue { key, value })
        .collect()
}
