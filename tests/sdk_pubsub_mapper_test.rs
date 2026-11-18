//! The native Pub/Sub mappers, field by field and branch by branch.
//!
//! A mutation sweep reported `map_topic` and `map_subscription` as untested:
//! nothing pinned what a generated `Topic`/`Subscription` becomes, so a body
//! replaced wholesale, a flipped `!=`, or a dropped `filter` all survived.
//! Every test below plants distinct, non-default values and asserts the exact
//! row, and every conditional — push vs pull, empty vs set KMS key, a zero ack
//! deadline, an absent dead-letter policy, each `state` arm — is exercised from
//! both sides with outputs that differ.

use google_cloud_pubsub::model::{DeadLetterPolicy, PushConfig, Subscription, Topic, subscription};
use google_cloud_wkt::Duration;

use nuvem::services::resources::pubsub::apply_subscription_counts;
use nuvem::services::sdk::pubsub::{map_subscription, map_topic};
use nuvem_domain::gcp::resource::{
    ComputeKeyValue, PubSubDeadLetterPolicy, PubSubDelivery, PubSubSubscription,
    PubSubSubscriptionState, PubSubTopic, SubscriptionCount,
};

fn duration(seconds: i64) -> Duration {
    Duration::new(seconds, 0).expect("a valid duration")
}

fn pair(key: &str, value: &str) -> ComputeKeyValue {
    ComputeKeyValue {
        key: key.to_string(),
        value: value.to_string(),
    }
}

// ---------------------------------------------------------------- map_topic

/// Kills "replace body with Default::default()" and any per-field drop: every
/// field carries a value distinct from its type's default and from every other
/// field's.
#[test]
fn a_topic_maps_each_field_to_its_own_planted_value() {
    let topic = Topic::new()
        .set_name("projects/ledger-prod/topics/orders")
        .set_kms_key_name("projects/ledger-prod/locations/eu/keyRings/ring/cryptoKeys/topics")
        .set_message_retention_duration(duration(604_800))
        .set_labels([("team", "payments"), ("env", "prod")]);

    assert_eq!(
        map_topic(&topic),
        PubSubTopic {
            name: "orders".to_string(),
            full_name: "projects/ledger-prod/topics/orders".to_string(),
            subscription_count: SubscriptionCount::Unavailable,
            message_retention: Some("604800s".to_string()),
            kms_key: Some(
                "projects/ledger-prod/locations/eu/keyRings/ring/cryptoKeys/topics".to_string()
            ),
            labels: vec![pair("env", "prod"), pair("team", "payments")],
        }
    );
}

/// The other side of every optional field on the topic row: a model that sets
/// none of them must map to absences rather than to empty strings or to a
/// `Some` of the default.
#[test]
fn a_topic_without_retention_kms_or_labels_maps_those_as_absent() {
    let topic = Topic::new().set_name("projects/sandbox/topics/audit-trail");

    assert_eq!(
        map_topic(&topic),
        PubSubTopic {
            name: "audit-trail".to_string(),
            full_name: "projects/sandbox/topics/audit-trail".to_string(),
            subscription_count: SubscriptionCount::Unavailable,
            message_retention: None,
            kms_key: None,
            labels: Vec::new(),
        }
    );
}

/// `non_empty` is the only thing standing between the proto's unset string and
/// a `Some("")` in the detail pane. Both sides, one assertion apart.
#[test]
fn an_empty_kms_key_is_absent_while_a_set_one_survives_verbatim() {
    let unencrypted = Topic::new()
        .set_name("projects/p/topics/plain")
        .set_kms_key_name("");
    assert_eq!(map_topic(&unencrypted).kms_key, None);

    let encrypted = Topic::new()
        .set_name("projects/p/topics/sealed")
        .set_kms_key_name("projects/p/locations/us/keyRings/r/cryptoKeys/k");
    assert_eq!(
        map_topic(&encrypted).kms_key.as_deref(),
        Some("projects/p/locations/us/keyRings/r/cryptoKeys/k"),
        "the key is shown as the API spells it, not summarised"
    );
}

/// The retention cell has to read the way the CLI's `604800s` read. A body
/// replaced with `Some("xyzzy")` or with the seconds alone fails here, and the
/// second case pins that sub-second precision is dropped rather than rounded
/// into the number.
#[test]
fn a_topics_retention_renders_as_whole_seconds_with_the_clis_suffix() {
    let week = Topic::new()
        .set_name("projects/p/topics/week")
        .set_message_retention_duration(duration(604_800));
    assert_eq!(
        map_topic(&week).message_retention.as_deref(),
        Some("604800s")
    );

    let ten_minutes = Topic::new()
        .set_name("projects/p/topics/short")
        .set_message_retention_duration(Duration::new(600, 999_000_000).expect("a valid duration"));
    assert_eq!(
        map_topic(&ten_minutes).message_retention.as_deref(),
        Some("600s"),
        "nanos are not part of the cell the CLI printed"
    );
}

/// A topic listing alone cannot answer the count, and `Unavailable` is not
/// `Known(0)`: the dash the table paints must not become a zero because a
/// mapper defaulted the field.
#[test]
fn a_topics_subscription_count_is_unavailable_rather_than_a_zero() {
    let topic = Topic::new()
        .set_name("projects/p/topics/orders")
        .set_labels([("env", "prod")]);

    let row = map_topic(&topic);
    assert_eq!(row.subscription_count, SubscriptionCount::Unavailable);
    assert_eq!(
        row.subscription_count.known(),
        None,
        "no count is in hand until the subscriptions join runs"
    );
}

/// Labels arrive from a `HashMap`, whose order is not a fact. Three keys
/// planted out of order pin that the mapped row is sorted, so a reload cannot
/// reshuffle the pairs under the operator's cursor.
#[test]
fn topic_labels_are_sorted_by_key_rather_than_left_in_map_order() {
    let topic = Topic::new()
        .set_name("projects/p/topics/orders")
        .set_labels([
            ("zone", "eu"),
            ("app", "ledger"),
            ("managed-by", "terraform"),
        ]);

    assert_eq!(
        map_topic(&topic).labels,
        vec![
            pair("app", "ledger"),
            pair("managed-by", "terraform"),
            pair("zone", "eu"),
        ]
    );
}

/// `short_id` takes the last path segment; a name with no separator is already
/// its own short id, and the two spellings must not collapse into one field.
#[test]
fn a_topic_name_without_a_path_is_its_own_short_id() {
    let topic = Topic::new().set_name("orders");

    let row = map_topic(&topic);
    assert_eq!(row.name, "orders");
    assert_eq!(row.full_name, "orders");
}

// --------------------------------------------------------- map_subscription

/// The whole subscription row in one equality, every field distinct: a body
/// replaced wholesale, a field left at its default, or two fields swapped all
/// die here.
#[test]
fn a_push_subscription_maps_each_field_to_its_own_planted_value() {
    let sub = Subscription::new()
        .set_name("projects/ledger-prod/subscriptions/orders-shipping")
        .set_topic("projects/ledger-prod/topics/orders")
        .set_push_config(PushConfig::new().set_push_endpoint("https://shipping.example.com/push"))
        .set_ack_deadline_seconds(45)
        .set_message_retention_duration(duration(86_400))
        .set_dead_letter_policy(
            DeadLetterPolicy::new()
                .set_dead_letter_topic("projects/ledger-prod/topics/orders-dead")
                .set_max_delivery_attempts(12),
        )
        .set_state(subscription::State::Active)
        .set_labels([("team", "shipping"), ("env", "prod")]);

    assert_eq!(
        map_subscription(&sub),
        PubSubSubscription {
            name: "orders-shipping".to_string(),
            full_name: "projects/ledger-prod/subscriptions/orders-shipping".to_string(),
            topic: "projects/ledger-prod/topics/orders".to_string(),
            delivery: PubSubDelivery::Push {
                endpoint: "https://shipping.example.com/push".to_string()
            },
            ack_deadline_seconds: Some(45),
            message_retention: Some("86400s".to_string()),
            dead_letter: Some(PubSubDeadLetterPolicy {
                dead_letter_topic: "projects/ledger-prod/topics/orders-dead".to_string(),
                max_delivery_attempts: Some(12),
            }),
            state: PubSubSubscriptionState::Active,
            labels: vec![pair("env", "prod"), pair("team", "shipping")],
        }
    );
}

/// The other side of every optional field, and of the delivery branch: no
/// `pushConfig` is a pull subscription rather than a push to nowhere.
#[test]
fn a_subscription_without_push_config_or_options_maps_to_a_bare_pull_row() {
    let sub = Subscription::new()
        .set_name("projects/sandbox/subscriptions/audit-tail")
        .set_topic("projects/sandbox/topics/audit-trail");

    assert_eq!(
        map_subscription(&sub),
        PubSubSubscription {
            name: "audit-tail".to_string(),
            full_name: "projects/sandbox/subscriptions/audit-tail".to_string(),
            topic: "projects/sandbox/topics/audit-trail".to_string(),
            delivery: PubSubDelivery::Pull,
            ack_deadline_seconds: None,
            message_retention: None,
            dead_letter: None,
            state: PubSubSubscriptionState::Unspecified,
            labels: Vec::new(),
        }
    );
}

/// `delivery_from_endpoint` filters on emptiness, not on presence. A
/// `pushConfig` carrying an empty endpoint is still a pull subscription, and
/// dropping the filter would render `Push` with a blank endpoint.
#[test]
fn a_push_config_with_an_empty_endpoint_is_still_pull() {
    let empty = Subscription::new()
        .set_name("projects/p/subscriptions/s")
        .set_topic("projects/p/topics/t")
        .set_push_config(PushConfig::new().set_push_endpoint(""));
    assert_eq!(map_subscription(&empty).delivery, PubSubDelivery::Pull);

    let endpoint = Subscription::new()
        .set_name("projects/p/subscriptions/s")
        .set_topic("projects/p/topics/t")
        .set_push_config(PushConfig::new().set_push_endpoint("https://example.com/push"));
    assert_eq!(
        map_subscription(&endpoint).delivery,
        PubSubDelivery::Push {
            endpoint: "https://example.com/push".to_string()
        },
        "an endpoint travels with the case that has one"
    );
}

/// `0` is the model's unset, so it reads as no deadline rather than as an
/// instant timeout. Flipping the `!=` to `==` turns both assertions inside out.
#[test]
fn a_zero_ack_deadline_is_absent_while_any_other_is_kept() {
    let unset = Subscription::new()
        .set_name("projects/p/subscriptions/s")
        .set_topic("projects/p/topics/t")
        .set_ack_deadline_seconds(0);
    assert_eq!(map_subscription(&unset).ack_deadline_seconds, None);

    let ten = Subscription::new()
        .set_name("projects/p/subscriptions/s")
        .set_topic("projects/p/topics/t")
        .set_ack_deadline_seconds(10);
    assert_eq!(map_subscription(&ten).ack_deadline_seconds, Some(10));

    let maximum = Subscription::new()
        .set_name("projects/p/subscriptions/s")
        .set_topic("projects/p/topics/t")
        .set_ack_deadline_seconds(600);
    assert_eq!(map_subscription(&maximum).ack_deadline_seconds, Some(600));
}

/// The field is a signed `i32` on the wire, and the row's is a `u32`. A
/// negative value has no reading, so it is dropped rather than wrapped into a
/// four-billion-second deadline.
#[test]
fn a_negative_ack_deadline_is_dropped_rather_than_wrapped() {
    let sub = Subscription::new()
        .set_name("projects/p/subscriptions/s")
        .set_topic("projects/p/topics/t")
        .set_ack_deadline_seconds(-30);

    assert_eq!(map_subscription(&sub).ack_deadline_seconds, None);
}

/// The subscription's retention cell shares `duration_to_string` with the
/// topic's, and has to agree with it on both sides.
#[test]
fn a_subscriptions_retention_renders_in_seconds_and_an_absent_one_stays_absent() {
    let retained = Subscription::new()
        .set_name("projects/p/subscriptions/s")
        .set_topic("projects/p/topics/t")
        .set_message_retention_duration(duration(259_200));
    assert_eq!(
        map_subscription(&retained).message_retention.as_deref(),
        Some("259200s")
    );

    let unset = Subscription::new()
        .set_name("projects/p/subscriptions/s")
        .set_topic("projects/p/topics/t");
    assert_eq!(map_subscription(&unset).message_retention, None);
}

/// Both sides of the dead-letter `map`: no policy is `None`, and a policy maps
/// its two fields with values distinct from each other and from the defaults.
#[test]
fn a_dead_letter_policy_maps_its_topic_and_attempts_and_its_absence_maps_to_none() {
    let without = Subscription::new()
        .set_name("projects/p/subscriptions/s")
        .set_topic("projects/p/topics/t");
    assert_eq!(map_subscription(&without).dead_letter, None);

    let with = Subscription::new()
        .set_name("projects/p/subscriptions/s")
        .set_topic("projects/p/topics/t")
        .set_dead_letter_policy(
            DeadLetterPolicy::new()
                .set_dead_letter_topic("projects/p/topics/parking-lot")
                .set_max_delivery_attempts(7),
        );
    assert_eq!(
        map_subscription(&with).dead_letter,
        Some(PubSubDeadLetterPolicy {
            dead_letter_topic: "projects/p/topics/parking-lot".to_string(),
            max_delivery_attempts: Some(7),
        })
    );
}

/// `max_delivery_attempts` is converted, not filtered: the mapper runs
/// `u32::try_from(..).ok()`, so a zero maps to `Some(0)` and only a negative —
/// which `u32` cannot hold — maps to `None`. Pinned because the opposite
/// ("0 means absent", as the ack deadline does) is the plausible misreading.
#[test]
fn a_zero_max_delivery_attempts_is_kept_and_only_a_negative_one_is_dropped() {
    let zero = Subscription::new()
        .set_name("projects/p/subscriptions/s")
        .set_topic("projects/p/topics/t")
        .set_dead_letter_policy(
            DeadLetterPolicy::new()
                .set_dead_letter_topic("projects/p/topics/parking-lot")
                .set_max_delivery_attempts(0),
        );
    assert_eq!(
        map_subscription(&zero)
            .dead_letter
            .expect("a policy was set")
            .max_delivery_attempts,
        Some(0)
    );

    let negative = Subscription::new()
        .set_name("projects/p/subscriptions/s")
        .set_topic("projects/p/topics/t")
        .set_dead_letter_policy(
            DeadLetterPolicy::new()
                .set_dead_letter_topic("projects/p/topics/parking-lot")
                .set_max_delivery_attempts(-1),
        );
    assert_eq!(
        map_subscription(&negative)
            .dead_letter
            .expect("a policy was set")
            .max_delivery_attempts,
        None,
        "a count a u32 cannot hold is no count"
    );
}

/// Every arm of the state translation, each producing a different row value:
/// the two named states, the proto zero, and the fallback.
#[test]
fn every_subscription_state_arm_maps_to_its_own_domain_state() {
    let state_of = |state: subscription::State| {
        let sub = Subscription::new()
            .set_name("projects/p/subscriptions/s")
            .set_topic("projects/p/topics/t")
            .set_state(state);
        map_subscription(&sub).state
    };

    assert_eq!(
        state_of(subscription::State::Active),
        PubSubSubscriptionState::Active
    );
    assert_eq!(
        state_of(subscription::State::ResourceError),
        PubSubSubscriptionState::ResourceError
    );
    assert_eq!(
        state_of(subscription::State::Unspecified),
        PubSubSubscriptionState::Unspecified
    );
}

/// The fallback arm, with a name the client library has never heard of: it is
/// carried through verbatim rather than flattened into `Active` or dropped, so
/// an operator sees the API's own word for a state this build predates.
#[test]
fn an_unrecognised_state_name_is_carried_through_verbatim() {
    let sub = Subscription::new()
        .set_name("projects/p/subscriptions/s")
        .set_topic("projects/p/topics/t")
        .set_state(subscription::State::from("PAUSED_BY_QUOTA"));

    let state = map_subscription(&sub).state;
    assert_eq!(
        state,
        PubSubSubscriptionState::Unrecognized("PAUSED_BY_QUOTA".to_string())
    );
    assert_eq!(
        state.api_value(),
        "PAUSED_BY_QUOTA",
        "the raw name round-trips rather than being relabelled"
    );
}

/// An enum deserialized from an integer this build does not know has no name
/// at all, and `from_api(None)` is the proto zero: `Unspecified`, not an
/// `Unrecognized("")` the state column would paint as a blank.
#[test]
fn a_state_with_no_name_reads_as_unspecified_rather_than_as_an_empty_label() {
    let sub = Subscription::new()
        .set_name("projects/p/subscriptions/s")
        .set_topic("projects/p/topics/t")
        .set_state(subscription::State::from(97));

    assert_eq!(
        map_subscription(&sub).state,
        PubSubSubscriptionState::Unspecified
    );
}

/// The subscription row's labels go through the same sorted collection as the
/// topic's, and both sides — some labels, none — are pinned here.
#[test]
fn subscription_labels_are_sorted_and_an_unlabelled_subscription_maps_to_an_empty_list() {
    let labelled = Subscription::new()
        .set_name("projects/p/subscriptions/s")
        .set_topic("projects/p/topics/t")
        .set_labels([("zone", "eu"), ("app", "ledger"), ("tier", "gold")]);
    assert_eq!(
        map_subscription(&labelled).labels,
        vec![
            pair("app", "ledger"),
            pair("tier", "gold"),
            pair("zone", "eu"),
        ]
    );

    let bare = Subscription::new()
        .set_name("projects/p/subscriptions/s")
        .set_topic("projects/p/topics/t");
    assert_eq!(
        map_subscription(&bare).labels,
        Vec::<ComputeKeyValue>::new()
    );
}

/// The parent topic is copied as the API spells it, because the topics join
/// matches on the full resource name. `_deleted-topic_` is a real value rather
/// than an error, and it has to survive the mapper to match no topic row.
#[test]
fn the_parent_topic_is_copied_verbatim_including_the_deleted_topic_sentinel() {
    let live = Subscription::new()
        .set_name("projects/ledger-prod/subscriptions/orders-shipping")
        .set_topic("projects/ledger-prod/topics/orders");
    let row = map_subscription(&live);
    assert_eq!(row.topic, "projects/ledger-prod/topics/orders");
    assert_eq!(
        row.name, "orders-shipping",
        "the row's own name is shortened even though the topic is not"
    );

    let orphaned = Subscription::new()
        .set_name("projects/ledger-prod/subscriptions/orphan")
        .set_topic("_deleted-topic_");
    assert_eq!(map_subscription(&orphaned).topic, "_deleted-topic_");
}

/// The two native mappers produce rows the subscriptions join can match.
///
/// `SdkPubSubService::load_topics` lists the topics, lists the subscriptions,
/// and hands both to `apply_subscription_counts` — the same join the CLI path
/// has always run. That join matches on the *full* resource name, so it holds
/// only if `map_topic` keeps the topic's name unshortened and `map_subscription`
/// copies its parent verbatim. Each mapper is pinned for that above; this pins
/// the pair against the function that consumes them, because the count column
/// was `Unavailable` on every native read until the join was wired in, and a
/// spelling drift on either side would silently return it to that state — a
/// dash where the CLI backend shows a number.
#[test]
fn native_topic_and_subscription_rows_join_on_the_full_resource_name() {
    let topics = [
        Topic::new().set_name("projects/ledger-prod/topics/orders"),
        Topic::new().set_name("projects/ledger-prod/topics/back-orders"),
        Topic::new().set_name("projects/ledger-prod/topics/quiet"),
    ];
    let subscriptions = [
        Subscription::new()
            .set_name("projects/ledger-prod/subscriptions/shipping")
            .set_topic("projects/ledger-prod/topics/orders"),
        Subscription::new()
            .set_name("projects/ledger-prod/subscriptions/billing")
            .set_topic("projects/ledger-prod/topics/orders"),
        Subscription::new()
            .set_name("projects/ledger-prod/subscriptions/returns")
            .set_topic("projects/ledger-prod/topics/back-orders"),
        Subscription::new()
            .set_name("projects/ledger-prod/subscriptions/orphan")
            .set_topic("_deleted-topic_"),
    ];

    let mut rows: Vec<PubSubTopic> = topics.iter().map(map_topic).collect();
    let subscription_rows: Vec<PubSubSubscription> =
        subscriptions.iter().map(map_subscription).collect();
    apply_subscription_counts(&mut rows, &subscription_rows);

    let counts: Vec<(String, Option<usize>)> = rows
        .iter()
        .map(|row| (row.name.clone(), row.subscription_count.known()))
        .collect();
    assert_eq!(
        counts,
        vec![
            ("orders".to_string(), Some(2)),
            // Not 3: a suffix match would have given `back-orders` the two
            // subscriptions of `orders` as well.
            ("back-orders".to_string(), Some(1)),
            // A topic nothing subscribes to is a known zero, not a dash, and
            // the orphaned subscription's sentinel parent matches no row.
            ("quiet".to_string(), Some(0)),
        ]
    );
}
