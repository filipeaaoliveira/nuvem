//! Pub/Sub service layer: the argv nuvem issues, what it makes of the JSON that
//! comes back, and — the part that carries this slice — what it does when the
//! *second* of its two list calls is refused.
//!
//! A topics load makes two calls: `pubsub topics list`, then
//! `pubsub subscriptions list`. Only the first is essential. The second exists
//! to derive the subscription count, and it is the one an operator may not be
//! allowed to make: `pubsub.topics.list` granted with
//! `pubsub.subscriptions.list` denied is an ordinary read-only IAM shape. The
//! degradation tests below are what stop that shape from emptying the table.

use nuvem::services::gcloud::cli::{CommandError, CommandOutput, CommandRunner, GcloudCommand};
use nuvem::services::resources::pubsub::{
    GcloudPubSubService, apply_subscription_counts, parse_subscriptions_json, parse_topics_json,
};
use nuvem_domain::gcp::configuration::{GcloudConfigurationName, GcloudContext};
use nuvem_domain::gcp::resource::{
    PubSubDelivery, PubSubSubscriptionState, PubSubTopic, SubscriptionCount,
};
use std::cell::RefCell;

const TOPICS_JSON: &str = include_str!("fixtures/pubsub_topics.json");
const SUBSCRIPTIONS_JSON: &str = include_str!("fixtures/pubsub_subscriptions.json");

fn ctx() -> GcloudContext {
    GcloudContext {
        configuration: GcloudConfigurationName::new("dev"),
        account: None,
        project: Some("dev-project".into()),
        region: None,
        zone: None,
    }
}

/// Answers each successive call from a script, so a test can give the topics
/// call one payload and the subscriptions call another — or refuse the second
/// while the first succeeds.
struct ScriptedRunner {
    replies: RefCell<std::collections::VecDeque<Result<String, CommandError>>>,
    seen: RefCell<Vec<Vec<String>>>,
}

impl ScriptedRunner {
    fn new(replies: Vec<Result<String, CommandError>>) -> Self {
        Self {
            replies: RefCell::new(replies.into_iter().collect()),
            seen: RefCell::new(Vec::new()),
        }
    }

    /// The happy path: topics, then subscriptions.
    fn both() -> Self {
        Self::new(vec![
            Ok(TOPICS_JSON.to_string()),
            Ok(SUBSCRIPTIONS_JSON.to_string()),
        ])
    }

    fn calls(&self) -> Vec<Vec<String>> {
        self.seen.borrow().clone()
    }
}

impl CommandRunner for ScriptedRunner {
    fn run(&self, command: GcloudCommand) -> Result<CommandOutput, CommandError> {
        self.seen.borrow_mut().push(command.args.clone());
        match self
            .replies
            .borrow_mut()
            .pop_front()
            .expect("the script has a reply for every call the service makes")
        {
            Ok(stdout) => Ok(CommandOutput {
                stdout,
                stderr: String::new(),
            }),
            Err(error) => Err(error),
        }
    }
}

fn denied() -> CommandError {
    CommandError::Failed {
        stdout: String::new(),
        stderr: "PERMISSION_DENIED: User does not have permission \
                 'pubsub.subscriptions.list'"
            .to_string(),
        exit_code: Some(1),
    }
}

fn topic<'a>(topics: &'a [PubSubTopic], name: &str) -> &'a PubSubTopic {
    topics
        .iter()
        .find(|candidate| candidate.name == name)
        .unwrap_or_else(|| panic!("the fixture holds a topic named {name}"))
}

// --- Topic parsing ---

#[test]
fn the_topic_list_parses_the_short_name_retention_and_kms_key() {
    let topics = parse_topics_json(TOPICS_JSON).expect("the fixture parses");

    assert_eq!(topics.len(), 3, "the fixture holds three topics");

    let orders = topic(&topics, "orders");
    // The short id in `name`, the resource path in `full_name`: the table shows
    // one and the join onto subscriptions uses the other.
    assert_eq!(orders.name, "orders");
    assert_eq!(orders.full_name, "projects/dev-project/topics/orders");
    assert_eq!(orders.message_retention.as_deref(), Some("604800s"));
    assert_eq!(
        orders.kms_key.as_deref(),
        Some("projects/dev-project/locations/europe-west1/keyRings/nuvem/cryptoKeys/pubsub")
    );
    assert_eq!(
        orders
            .labels
            .iter()
            .map(|label| (label.key.as_str(), label.value.as_str()))
            .collect::<Vec<_>>(),
        vec![("env", "prod"), ("team", "platform")],
        "labels are sorted, so a reload does not reshuffle them under the cursor"
    );
}

#[test]
fn a_topic_with_no_optional_fields_parses_rather_than_failing_the_load() {
    let topics = parse_topics_json(TOPICS_JSON).expect("the fixture parses");

    let audit = topic(&topics, "audit-log");
    // Absent, not defaulted to something that reads as a configured value.
    assert_eq!(audit.message_retention, None);
    assert_eq!(audit.kms_key, None);
    assert!(audit.labels.is_empty());
}

#[test]
fn a_parsed_topic_starts_with_no_count_until_the_subscriptions_call_answers() {
    let topics = parse_topics_json(TOPICS_JSON).expect("the fixture parses");

    // Parsing a topic cannot know its count — the count comes from the other
    // call. `Known(0)` here would be a claim the parser is in no position to
    // make.
    for topic in &topics {
        assert_eq!(topic.subscription_count, SubscriptionCount::Unavailable);
    }
}

#[test]
fn malformed_topic_json_is_an_error_rather_than_an_empty_table() {
    // An empty table is a claim that the project has no topics. An unreadable
    // answer must not render as that claim.
    assert!(parse_topics_json("{not json").is_err());
}

// --- Subscription parsing ---

#[test]
fn a_subscription_with_no_push_endpoint_is_a_pull_subscription() {
    let subscriptions = parse_subscriptions_json(SUBSCRIPTIONS_JSON).expect("the fixture parses");

    let worker = &subscriptions[0];
    assert_eq!(worker.name, "orders-worker");
    assert_eq!(worker.delivery, PubSubDelivery::Pull);
    assert_eq!(worker.ack_deadline_seconds, Some(60));
    assert_eq!(worker.message_retention.as_deref(), Some("604800s"));
    assert_eq!(worker.state, PubSubSubscriptionState::Active);
    assert_eq!(worker.dead_letter, None);
}

#[test]
fn a_push_subscription_carries_its_endpoint_and_dead_letter_policy() {
    let subscriptions = parse_subscriptions_json(SUBSCRIPTIONS_JSON).expect("the fixture parses");

    let audit = &subscriptions[1];
    assert_eq!(
        audit.delivery,
        PubSubDelivery::Push {
            endpoint: "https://audit.example.com/pubsub".to_string()
        }
    );
    assert_eq!(audit.state, PubSubSubscriptionState::ResourceError);
    let dead_letter = audit
        .dead_letter
        .as_ref()
        .expect("this subscription has a dead-letter policy");
    assert_eq!(
        dead_letter.dead_letter_topic,
        "projects/dev-project/topics/dead-letters"
    );
    assert_eq!(dead_letter.max_delivery_attempts, Some(5));
}

#[test]
fn an_empty_push_config_is_still_a_pull_subscription() {
    let subscriptions = parse_subscriptions_json(SUBSCRIPTIONS_JSON).expect("the fixture parses");

    // gcloud returns `"pushConfig": {}` for a pull subscription. Reading the
    // *presence* of the field as "push" would label every pull subscription a
    // push one with a blank endpoint; the endpoint is what decides.
    assert_eq!(subscriptions[2].delivery, PubSubDelivery::Pull);
}

#[test]
fn a_subscription_with_no_state_field_reads_as_unspecified() {
    let subscriptions = parse_subscriptions_json(SUBSCRIPTIONS_JSON).expect("the fixture parses");

    // proto3 JSON omits zero-valued enums, so a missing `state` *is*
    // `STATE_UNSPECIFIED` rather than a parse gap.
    assert_eq!(
        subscriptions[2].state,
        PubSubSubscriptionState::Unspecified,
        "back-orders-worker has no state field in the fixture"
    );
}

#[test]
fn malformed_subscription_json_is_an_error() {
    assert!(parse_subscriptions_json("{not json").is_err());
}

// --- The join ---

#[test]
fn counts_group_subscriptions_onto_the_topic_that_named_them() {
    let mut topics = parse_topics_json(TOPICS_JSON).expect("the fixture parses");
    let subscriptions = parse_subscriptions_json(SUBSCRIPTIONS_JSON).expect("the fixture parses");

    apply_subscription_counts(&mut topics, &subscriptions);

    assert_eq!(
        topic(&topics, "orders").subscription_count,
        SubscriptionCount::Known(2)
    );
}

#[test]
fn a_topic_whose_id_is_a_suffix_of_another_keeps_its_own_subscriptions() {
    let mut topics = parse_topics_json(TOPICS_JSON).expect("the fixture parses");
    let subscriptions = parse_subscriptions_json(SUBSCRIPTIONS_JSON).expect("the fixture parses");

    apply_subscription_counts(&mut topics, &subscriptions);

    // `orders` and `back-orders`: joining on the short id with a suffix or
    // `contains` test would give `orders` three subscriptions and `back-orders`
    // one. The join is on the full resource name, and it is exact.
    assert_eq!(
        topic(&topics, "orders").subscription_count,
        SubscriptionCount::Known(2),
        "back-orders-worker belongs to back-orders, not to orders"
    );
    assert_eq!(
        topic(&topics, "back-orders").subscription_count,
        SubscriptionCount::Known(1)
    );
}

#[test]
fn a_topic_with_no_subscriptions_counts_zero_once_the_call_has_answered() {
    let mut topics = parse_topics_json(TOPICS_JSON).expect("the fixture parses");
    let subscriptions = parse_subscriptions_json(SUBSCRIPTIONS_JSON).expect("the fixture parses");

    apply_subscription_counts(&mut topics, &subscriptions);

    // Zero is a real answer here, and distinct from `Unavailable`: the call
    // succeeded and named no subscription for this topic.
    assert_eq!(
        topic(&topics, "audit-log").subscription_count,
        SubscriptionCount::Known(0)
    );
}

#[test]
fn a_subscription_whose_topic_was_deleted_is_counted_against_no_topic() {
    let mut topics = parse_topics_json(TOPICS_JSON).expect("the fixture parses");
    let subscriptions = parse_subscriptions_json(SUBSCRIPTIONS_JSON).expect("the fixture parses");

    apply_subscription_counts(&mut topics, &subscriptions);

    // `_deleted-topic_` is a real value the API returns, not an error. It
    // matches no topic row, and must not inflate one.
    let total: usize = topics
        .iter()
        .filter_map(|topic| topic.subscription_count.known())
        .sum();
    assert_eq!(
        total,
        subscriptions.len() - 1,
        "every subscription but the orphan is attributed to a topic"
    );
}

// --- The argv ---

#[test]
fn the_load_issues_the_topics_call_then_the_subscriptions_call() {
    let runner = ScriptedRunner::both();
    GcloudPubSubService::new(&runner)
        .load_topics(&ctx(), None)
        .expect("the fixture parses");

    let calls = runner.calls();
    assert_eq!(calls.len(), 2, "one topics call and one subscriptions call");
    assert!(
        calls[0].contains(&"topics".to_string()) && calls[0].contains(&"list".to_string()),
        "first call lists topics: {:?}",
        calls[0]
    );
    assert!(
        calls[1].contains(&"subscriptions".to_string()) && calls[1].contains(&"list".to_string()),
        "second call lists subscriptions: {:?}",
        calls[1]
    );
    for call in &calls {
        assert!(
            call.contains(&"--configuration".to_string()),
            "every call carries the configuration: {call:?}"
        );
        assert!(
            call.contains(&"--format=json".to_string()),
            "every call asks for JSON: {call:?}"
        );
    }
}

#[test]
fn the_load_joins_the_two_calls_into_counted_topics() {
    let runner = ScriptedRunner::both();
    let topics = GcloudPubSubService::new(&runner)
        .load_topics(&ctx(), None)
        .expect("the fixture parses");

    assert_eq!(
        topic(&topics, "orders").subscription_count,
        SubscriptionCount::Known(2)
    );
    assert_eq!(
        topic(&topics, "audit-log").subscription_count,
        SubscriptionCount::Known(0)
    );
}

// --- Degradation: the subscriptions call is refused ---
//
// These are the tests that carry the slice. `pubsub.topics.list` granted with
// `pubsub.subscriptions.list` denied is an ordinary read-only IAM shape, and
// the topics table has to stay useful in it.

/// The topics call succeeds, the subscriptions call is denied.
fn denied_subscriptions() -> Vec<PubSubTopic> {
    let runner = ScriptedRunner::new(vec![Ok(TOPICS_JSON.to_string()), Err(denied())]);
    GcloudPubSubService::new(&runner)
        .load_topics(&ctx(), None)
        .expect("a denied subscriptions call must not fail the topics load")
}

#[test]
fn a_denied_subscriptions_call_still_returns_every_topic() {
    let topics = denied_subscriptions();

    assert_eq!(
        topics.len(),
        3,
        "the topics call answered, so every topic it listed is here"
    );
}

#[test]
fn a_denied_subscriptions_call_leaves_the_count_unknown_rather_than_zero() {
    let topics = denied_subscriptions();

    for topic in &topics {
        assert_eq!(
            topic.subscription_count,
            SubscriptionCount::Unavailable,
            "{} must read as unknown, not as a topic with no subscribers",
            topic.name
        );
    }
}

#[test]
fn a_denied_subscriptions_call_leaves_the_other_four_columns_intact() {
    let topics = denied_subscriptions();

    // The assertion that catches a load which bailed out wholesale on the
    // secondary call: the four columns that never needed it are still there.
    let orders = topic(&topics, "orders");
    assert_eq!(orders.name, "orders");
    assert_eq!(orders.message_retention.as_deref(), Some("604800s"));
    assert_eq!(
        orders.kms_key.as_deref(),
        Some("projects/dev-project/locations/europe-west1/keyRings/nuvem/cryptoKeys/pubsub")
    );
    assert_eq!(
        orders
            .labels
            .iter()
            .map(|label| (label.key.as_str(), label.value.as_str()))
            .collect::<Vec<_>>(),
        vec![("env", "prod"), ("team", "platform")]
    );
}

#[test]
fn an_unreadable_subscriptions_answer_degrades_the_same_way_as_a_refusal() {
    let runner = ScriptedRunner::new(vec![
        Ok(TOPICS_JSON.to_string()),
        Ok("{not json".to_string()),
    ]);
    let topics = GcloudPubSubService::new(&runner)
        .load_topics(&ctx(), None)
        .expect("an unreadable subscriptions answer must not fail the topics load");

    assert_eq!(topics.len(), 3);
    assert_eq!(
        topic(&topics, "orders").subscription_count,
        SubscriptionCount::Unavailable
    );
}

#[test]
fn a_denied_topics_call_still_fails_the_load() {
    // The degradation is specific to the enrichment call. The topics call *is*
    // the load, and swallowing its failure would render an empty table as the
    // claim that the project has no topics.
    let runner = ScriptedRunner::new(vec![Err(denied())]);
    let result = GcloudPubSubService::new(&runner).load_topics(&ctx(), None);

    assert!(result.is_err(), "a failed topics call must fail the load");
}

// --- The subscriptions call as a load of its own ---
//
// The same call serves two callers with opposite failure policies. The topics
// table treats a refusal as a missing count and renders anyway; the
// subscriptions table has nothing left to render, so it must fail loudly. The
// tests below pin both halves against one implementation, because the way this
// regresses is a later edit giving the shared call a single policy.

#[test]
fn loading_subscriptions_issues_one_project_wide_list_call() {
    let runner = ScriptedRunner::new(vec![Ok(SUBSCRIPTIONS_JSON.to_string())]);
    let service = GcloudPubSubService::new(runner);

    let subscriptions = service
        .load_subscriptions(&ctx(), None)
        .expect("the fixture parses");

    // One call for every subscription in the project, not one call per topic:
    // this is the fact the flat table rests on.
    let calls = service.runner().calls();
    assert_eq!(calls.len(), 1, "a flat load makes exactly one call");
    assert!(
        calls[0].windows(3).any(|window| window
            == [
                "pubsub".to_string(),
                "subscriptions".to_string(),
                "list".to_string()
            ]),
        "expected a `pubsub subscriptions list`, got {:?}",
        calls[0]
    );
    assert_eq!(
        subscriptions.len(),
        4,
        "the fixture holds four subscriptions"
    );
}

#[test]
fn a_refused_subscriptions_load_is_an_error_rather_than_an_empty_table() {
    // The opposite of what the topics load does with the same refusal. An
    // empty table is the claim "this project has no subscriptions", and a
    // denied permission is not in a position to make it.
    let runner = ScriptedRunner::new(vec![Err(denied())]);
    let service = GcloudPubSubService::new(runner);

    assert!(
        service.load_subscriptions(&ctx(), None).is_err(),
        "a refusal must surface, not render as zero subscriptions"
    );
}

#[test]
fn a_refused_subscriptions_call_still_leaves_the_topics_table_standing() {
    // The same call, the same refusal, the other caller. This is the
    // degradation that shipped, re-pinned here because hoisting the call to
    // a `Result` is exactly the edit that could take it away.
    let runner = ScriptedRunner::new(vec![Ok(TOPICS_JSON.to_string()), Err(denied())]);
    let service = GcloudPubSubService::new(runner);

    let topics = service
        .load_topics(&ctx(), None)
        .expect("a refused enrichment must not fail the topics load");

    assert_eq!(topics.len(), 3, "every topic still renders");
    for topic in &topics {
        assert_eq!(
            topic.subscription_count,
            SubscriptionCount::Unavailable,
            "the count reads unknown rather than zero"
        );
    }
}

#[test]
fn malformed_subscription_json_fails_the_flat_load_but_not_the_topics_load() {
    let flat = GcloudPubSubService::new(ScriptedRunner::new(vec![Ok("{not json".to_string())]));
    assert!(
        flat.load_subscriptions(&ctx(), None).is_err(),
        "an unreadable answer is not an empty project"
    );

    let enriched = GcloudPubSubService::new(ScriptedRunner::new(vec![
        Ok(TOPICS_JSON.to_string()),
        Ok("{not json".to_string()),
    ]));
    let topics = enriched
        .load_topics(&ctx(), None)
        .expect("an unreadable enrichment must not fail the topics load");
    assert_eq!(topics.len(), 3);
}

#[test]
fn a_flat_subscription_load_carries_the_parent_topic_and_the_deleted_sentinel() {
    let runner = ScriptedRunner::new(vec![Ok(SUBSCRIPTIONS_JSON.to_string())]);
    let service = GcloudPubSubService::new(runner);

    let subscriptions = service
        .load_subscriptions(&ctx(), None)
        .expect("the fixture parses");

    let worker = subscriptions
        .iter()
        .find(|subscription| subscription.name == "orders-worker")
        .expect("the fixture holds orders-worker");
    assert_eq!(worker.topic, "projects/dev-project/topics/orders");
    assert_eq!(worker.delivery, PubSubDelivery::Pull);
    assert_eq!(worker.ack_deadline_seconds, Some(60));
    assert_eq!(worker.state, PubSubSubscriptionState::Active);

    // A subscription whose topic was deleted is a real row with a real value,
    // not an error and not a row the flat table drops. The topics table skips
    // it when counting; the flat table must still show it.
    let orphaned = subscriptions
        .iter()
        .find(|subscription| subscription.name == "orphaned")
        .expect("a subscription whose topic is gone is still a subscription");
    assert_eq!(orphaned.topic, "_deleted-topic_");
}

#[test]
fn the_subscriptions_call_pushes_no_filter_clause_down() {
    // The searchable set includes the delivery label (`Pull`/`Push`), which is
    // derived from `pushConfig`'s presence and appears verbatim in no raw
    // field, and the state label (`Resource error` against a raw
    // `RESOURCE_ERROR`). Neither is a verbatim substring of anything pushable,
    // so nothing is pushed and narrowing stays client-side — the safe
    // direction, and the same bargain the topics table strikes.
    let runner = ScriptedRunner::new(vec![Ok(SUBSCRIPTIONS_JSON.to_string())]);
    let service = GcloudPubSubService::new(runner);

    service
        .load_subscriptions(&ctx(), Some("orders"))
        .expect("the fixture parses");

    let calls = service.runner().calls();
    assert!(
        !calls[0].iter().any(|arg| arg.starts_with("--filter=")),
        "no clause is eligible for pushdown, got {:?}",
        calls[0]
    );
}
